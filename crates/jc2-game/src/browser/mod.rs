//! `jc2 <game> --browse`: the asset browser.
//!
//! Left: a lazily expanded tree of every file the game's virtual file system can see
//! (folders from the resolved names, unresolved hashes under `unknown`; SARC containers
//! open into their members), or a flat list of matches while the text / kind filter is
//! set. Right: a preview by kind — a decoded texture with a mip selector, an orbit
//! render of a model, a property file's tree, text, a Havok / animation / FMOD summary,
//! a sound bank with play buttons, a hex dump — and actions (copy the name or hash,
//! export, open in the model viewer or the world).
//!
//! The UI is egui through `bevy_egui`; the model preview is a second camera rendering
//! into an image that egui draws (`model3d.rs`). Everything that is not drawing lives
//! in `jc2_formats::browse` (the tree, the search, the previews), so it is tested
//! without a window. File reads, previews and the background scan run on threads; the
//! frame loop only polls their channels.

mod model3d;
mod ui;

use crate::Game;
use bevy::prelude::*;
use bevy_egui::{EguiGlobalSettings, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext, egui};
use jc2_formats::GameFs;
use jc2_formats::browse::{
    self, Filter, Group, Hit, Index, Preview, Row, RowKind, ScanInfo, ScanTable, SearchResults,
    Target, Tree, TreeState,
};
use jc2_formats::detect::FileKind;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

/// `--browse-select`: what to select at start (debug / screenshots).
pub struct BrowserPlugin {
    pub select: Option<String>,
    /// `--browse-filter`: the text to start with in the filter box.
    pub filter: Option<String>,
}

#[derive(Resource)]
struct InitialSelect(Option<String>, Option<String>);

impl Plugin for BrowserPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            // the context is attached to the 2D camera below, not to the first camera
            // that happens to spawn
            .insert_resource(EguiGlobalSettings {
                auto_create_primary_context: false,
                ..default()
            })
            .add_plugins(model3d::ModelPreviewPlugin)
            .insert_resource(InitialSelect(self.select.clone(), self.filter.clone()))
            .add_systems(Startup, setup)
            .add_systems(Update, poll)
            .add_systems(EguiPrimaryContextPass, ui::draw);
    }
}

fn setup(mut commands: Commands, game: Res<Game>, initial: Res<InitialSelect>) {
    commands.spawn((Camera2d, PrimaryEguiContext));
    let mut browser = Browser::new(game.fs.clone());
    if let Some(f) = &initial.1 {
        browser.filter.text = f.clone();
    }
    if let Some(sel) = &initial.0
        && !browser.select_path(sel)
    {
        warn!("--browse-select: {sel} is not a file in the archives");
        browser.status = format!("no such file: {sel}");
    }
    commands.insert_resource(browser);
}

fn poll(mut browser: ResMut<Browser>, time: Res<Time<Real>>) {
    browser.poll(time.elapsed_secs());
}

/// A batch of scan results from the background thread: (item index, what it holds).
type ScanBatch = Vec<(usize, ScanInfo)>;

/// A decoded texture on its way to the screen.
struct TextureView {
    key: (Target, u32, bool, bool),
    handle: egui::TextureHandle,
}

/// Which picture of a texture to show.
struct TextureOptions {
    mip: u32,
    /// The mip chosen automatically for this target (so a new target restarts it).
    for_target: Option<Target>,
    alpha: bool,
    actual: bool,
}

#[derive(Resource)]
struct Browser {
    fs: Arc<GameFs>,
    index: Arc<Index>,
    tree: Tree,
    group: Group,
    scan: ScanTable,
    scan_rx: Mutex<Option<Receiver<ScanBatch>>>,
    state: TreeState,
    rows: Vec<Row>,
    rows_dirty: bool,
    filter: Filter,
    /// The filter the current `results` answer, and the scan progress they saw.
    searched: Option<(Filter, usize)>,
    results: Option<SearchResults>,
    last_search_at: f32,
    selected: Option<Target>,
    preview: Option<Arc<Preview>>,
    /// A preview is being built for this target.
    loading: Option<Target>,
    job_rx: Mutex<Option<Receiver<Preview>>>,
    /// Scroll the tree to this row once (a selection from the command line).
    scroll_to_row: Option<usize>,
    export_dir: String,
    status: String,
    tex: Option<TextureView>,
    tex_opts: TextureOptions,
    bank: Option<(Target, Arc<Vec<u8>>)>,
    focus_filter: bool,
}

impl Browser {
    fn new(fs: Arc<GameFs>) -> Self {
        let index = Arc::new(Index::from_fs(&fs));
        let group = Group::Path;
        let tree = Tree::build(&index, group);
        let scan = ScanTable::new(index.items.len());
        let rx = spawn_scan(fs.clone(), index.clone());
        let mut b = Self {
            fs,
            index,
            tree,
            group,
            scan,
            scan_rx: Mutex::new(Some(rx)),
            state: TreeState::default(),
            rows: Vec::new(),
            rows_dirty: true,
            filter: Filter::default(),
            searched: None,
            results: None,
            last_search_at: 0.0,
            selected: None,
            preview: None,
            loading: None,
            job_rx: Mutex::new(None),
            scroll_to_row: None,
            export_dir: default_export_dir().display().to_string(),
            status: String::new(),
            tex: None,
            tex_opts: TextureOptions {
                mip: 0,
                for_target: None,
                alpha: false,
                actual: false,
            },
            bank: None,
            focus_filter: false,
        };
        // the top level starts open
        b.refresh_rows();
        b
    }

    fn refresh_rows(&mut self) {
        self.rows = self.tree.flatten(&self.index, &self.scan, &self.state);
        self.rows_dirty = false;
    }

    /// Collect what the background threads have finished.
    fn poll(&mut self, now: f32) {
        let mut got = 0;
        if let Some(rx) = self.scan_rx.lock().unwrap().as_ref() {
            while let Ok(batch) = rx.try_recv() {
                got += batch.len();
                for (i, info) in batch {
                    self.scan.set(i, info);
                }
            }
        }
        if got > 0 {
            self.rows_dirty = true;
        }
        if self.scan.complete() {
            *self.scan_rx.lock().unwrap() = None;
        }
        if let Some(rx) = self.job_rx.lock().unwrap().as_ref()
            && let Ok(p) = rx.try_recv()
        {
            // a stale answer (the user clicked on) is dropped
            if self.selected.as_ref() == Some(&p.target) {
                self.preview = Some(Arc::new(p));
                self.loading = None;
            }
        }
        if self.loading.is_none() {
            *self.job_rx.lock().unwrap() = None;
        }
        // the search follows the filter, and the scan while it is still running
        if self.filter.is_active() {
            let key = (self.filter.clone(), self.scan.done);
            let stale = match &self.searched {
                None => true,
                Some((f, _)) if *f != self.filter => true,
                Some((_, done)) => *done != self.scan.done && now - self.last_search_at > 0.6,
            };
            if stale {
                self.results = Some(browse::search(&self.index, &self.scan, &self.filter));
                self.searched = Some(key);
                self.last_search_at = now;
            }
        } else if self.results.is_some() {
            self.results = None;
            self.searched = None;
        }
        if self.rows_dirty {
            self.refresh_rows();
        }
    }

    fn set_group(&mut self, group: Group) {
        if group == self.group {
            return;
        }
        self.group = group;
        self.tree = Tree::build(&self.index, group);
        self.state = TreeState::default();
        self.rows_dirty = true;
        if let Some(t) = self.selected.clone() {
            self.reveal(&t);
        }
    }

    /// Scan one item now (its members), if the background pass has not got to it.
    fn ensure_scanned(&mut self, item: usize) {
        if self.scan.get(item).is_none()
            && let Some(loc) = self.fs.locate_hash(self.index.items[item].hash)
            && let Some(info) = browse::scan_entry(&self.fs, &loc)
        {
            self.scan.set(item, info);
            self.rows_dirty = true;
        }
    }

    fn toggle_folder(&mut self, folder: usize) {
        self.state.toggle_folder(folder);
        self.rows_dirty = true;
    }

    fn toggle_container(&mut self, item: usize) {
        self.ensure_scanned(item);
        self.state.toggle_container(item);
        self.rows_dirty = true;
    }

    /// Open the folders (and the container) above a target and scroll to its row.
    fn reveal(&mut self, t: &Target) {
        let Some(item) = self.index.by_hash(t.file_hash()) else {
            return;
        };
        self.state.reveal(&self.tree, item);
        if matches!(t, Target::Member { .. }) {
            self.ensure_scanned(item);
            self.state.open_containers.insert(item);
        }
        self.refresh_rows();
        self.scroll_to_row = self
            .rows
            .iter()
            .position(|r| self.row_target(r).as_ref() == Some(t));
    }

    /// Select by text (`general.blz/gae06_parachute_dif.dds`, a bare name, a hash).
    fn select_path(&mut self, text: &str) -> bool {
        match browse::parse_target(&self.fs, text) {
            Some(t) => {
                self.reveal(&t);
                self.select(t);
                true
            }
            None => false,
        }
    }

    fn select(&mut self, t: Target) {
        if self.selected.as_ref() == Some(&t) {
            return;
        }
        self.selected = Some(t.clone());
        self.preview = None;
        self.loading = Some(t.clone());
        let (tx, rx) = channel();
        *self.job_rx.lock().unwrap() = Some(rx);
        let fs = self.fs.clone();
        std::thread::Builder::new()
            .name("browser-preview".into())
            .spawn(move || {
                let _ = tx.send(browse::build_preview(&fs, &t));
            })
            .expect("spawn the preview thread");
    }

    fn row_target(&self, r: &Row) -> Option<Target> {
        match r.kind {
            RowKind::File { item } | RowKind::Container { item, .. } => {
                Some(Target::File(self.index.items[item].hash))
            }
            RowKind::Member { item, member } => {
                let m = self.scan.get(item)?.members.get(member)?;
                Some(Target::Member {
                    container: self.index.items[item].hash,
                    name: m.name.clone(),
                })
            }
            RowKind::Folder { .. } => None,
        }
    }

    fn hit_target(&self, h: &Hit) -> Option<Target> {
        let item = &self.index.items[h.item];
        match h.member {
            None => Some(Target::File(item.hash)),
            Some(m) => Some(Target::Member {
                container: item.hash,
                name: self.scan.get(h.item)?.members.get(m)?.name.clone(),
            }),
        }
    }

    /// Up / down the list that is showing (the search results, or the open tree).
    fn step_selection(&mut self, delta: isize) {
        let targets: Vec<Option<Target>> = match &self.results {
            Some(r) => r.hits.iter().map(|h| self.hit_target(h)).collect(),
            None => self.rows.iter().map(|r| self.row_target(r)).collect(),
        };
        let cur = self
            .selected
            .as_ref()
            .and_then(|s| targets.iter().position(|t| t.as_ref() == Some(s)));
        let mut i = match cur {
            Some(c) => c as isize + delta,
            None => 0,
        };
        while i >= 0 && (i as usize) < targets.len() {
            if let Some(t) = &targets[i as usize] {
                let t = t.clone();
                self.select(t);
                // keep it in view
                self.scroll_to_row = Some(i as usize);
                return;
            }
            i += delta;
        }
    }

    /// Write the selected file to the export folder as `jc2tool extract` lays it out.
    fn export(&mut self, inflated: bool) {
        let (Some(t), Some(p)) = (self.selected.clone(), self.preview.clone()) else {
            return;
        };
        let result = (|| -> Result<PathBuf, String> {
            let bytes = browse::export_bytes(&self.fs, &t, inflated).map_err(|e| e.to_string())?;
            let rel = browse::export_relative(&self.fs, &t, p.kind);
            let rel: PathBuf = rel.split(['\\', '/']).collect();
            let path = PathBuf::from(self.export_dir.trim()).join(rel);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            }
            std::fs::write(&path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(path)
        })();
        self.status = match result {
            Ok(path) => {
                info!("exported {} to {}", p.label, path.display());
                format!("exported to {}", path.display())
            }
            Err(e) => {
                error!("export failed: {e}");
                format!("export failed: {e}")
            }
        };
    }

    /// Start another `jc2` on the same game folder (the model viewer, a world tile).
    fn relaunch(&mut self, args: &[&str]) {
        let result = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg(&self.fs.root)
                .args(args)
                .spawn()
        });
        self.status = match result {
            Ok(_) => format!("started: jc2 {}", args.join(" ")),
            Err(e) => {
                error!("could not start jc2 {}: {e}", args.join(" "));
                format!("could not start jc2: {e}")
            }
        };
    }

    /// The bytes of the selected sound bank, kept for the play buttons.
    fn bank_bytes(&mut self, t: &Target) -> Option<Arc<Vec<u8>>> {
        if let Some((bt, b)) = &self.bank
            && bt == t
        {
            return Some(b.clone());
        }
        let bytes = Arc::new(browse::load_target(&self.fs, t).ok()?.bytes);
        self.bank = Some((t.clone(), bytes.clone()));
        Some(bytes)
    }
}

fn default_export_dir() -> PathBuf {
    std::env::temp_dir().join("jc2-export")
}

/// Read every file's first bytes on a thread: what each really is, and the members of
/// the containers, so the kind filter and the search see inside them.
fn spawn_scan(fs: Arc<GameFs>, index: Arc<Index>) -> Receiver<ScanBatch> {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("browser-scan".into())
        .spawn(move || {
            let mut batch: ScanBatch = Vec::new();
            for (i, it) in index.items.iter().enumerate() {
                if let Some(loc) = fs.locate_hash(it.hash)
                    && let Some(info) = browse::scan_entry(&fs, &loc)
                {
                    batch.push((i, info));
                }
                if batch.len() >= 512 && tx.send(std::mem::take(&mut batch)).is_err() {
                    return;
                }
            }
            let _ = tx.send(batch);
        })
        .expect("spawn the scan thread");
    rx
}

/// The row colour for a kind of file.
fn kind_color(k: Option<FileKind>) -> egui::Color32 {
    let c = |r, g, b| egui::Color32::from_rgb(r, g, b);
    match k {
        Some(FileKind::Dds) | Some(FileKind::Bmp) => c(120, 200, 140),
        Some(FileKind::Rbm) => c(120, 170, 235),
        Some(FileKind::Sarc) => c(235, 205, 110),
        Some(FileKind::Property) | Some(FileKind::Blackboard) => c(235, 165, 100),
        Some(FileKind::Havok) => c(200, 140, 220),
        Some(FileKind::Fev) | Some(FileKind::Fsb) => c(235, 130, 150),
        Some(FileKind::Csv) | Some(FileKind::Text) | Some(FileKind::Xml) => c(200, 200, 200),
        _ => c(150, 150, 160),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::browse::PreviewKind;
    use std::time::{Duration, Instant};

    fn test_fs() -> Option<Arc<GameFs>> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        GameFs::open(dir).ok().map(Arc::new)
    }

    /// Poll until `done` or a wall-clock limit: threads decide when, not a frame count.
    fn wait_for(b: &mut Browser, what: &str, limit_s: u64, done: impl Fn(&Browser) -> bool) {
        let t0 = Instant::now();
        while !done(b) {
            assert!(
                t0.elapsed() < Duration::from_secs(limit_s),
                "timed out waiting for {what}"
            );
            b.poll(t0.elapsed().as_secs_f32());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn selects_a_member_and_builds_the_preview() {
        let Some(fs) = test_fs() else { return };
        let mut b = Browser::new(fs);
        // the closed tree: only top-level folders
        assert!(b.rows.len() < 200, "{}", b.rows.len());
        assert!(b.select_path("general.blz/gae06_parachute_dif.dds"));
        // revealing opened the folders above general.blz and the container itself
        assert!(
            b.rows
                .iter()
                .any(|r| matches!(r.kind, RowKind::Member { .. }))
        );
        assert!(b.scroll_to_row.is_some());
        wait_for(&mut b, "the texture preview", 60, |b| b.preview.is_some());
        let p = b.preview.clone().unwrap();
        assert_eq!(p.view, PreviewKind::Texture);
        assert!(p.label.ends_with("gae06_parachute_dif.dds"));
        // the tree row and the selection agree
        let row = b.scroll_to_row.unwrap();
        assert_eq!(b.row_target(&b.rows[row]), b.selected);
    }

    #[test]
    fn the_filter_and_the_scan_find_members() {
        let Some(fs) = test_fs() else { return };
        let mut b = Browser::new(fs);
        b.filter.text = "pda_map_dif".into();
        b.poll(0.0);
        let r = b.results.as_ref().unwrap();
        assert_eq!(r.total, 1);
        // once the background scan has read the containers, members are searchable too
        b.filter.text = "gae06_parachute_dif".into();
        wait_for(&mut b, "the background scan", 600, |b| b.scan.complete());
        b.poll(1000.0);
        let r = b.results.as_ref().unwrap();
        assert!(r.hits.iter().any(|h| h.member.is_some()), "{}", r.total);
        let t = b.hit_target(&r.hits[0]).unwrap();
        assert!(matches!(t, Target::Member { .. } | Target::File(_)));
        // the kind filter, on content
        b.filter = Filter {
            text: String::new(),
            kind: Some(FileKind::Dds),
        };
        b.poll(2000.0);
        assert!(b.results.as_ref().unwrap().total > 1000);
    }

    #[test]
    fn up_and_down_walk_the_open_rows() {
        let Some(fs) = test_fs() else { return };
        let mut b = Browser::new(fs);
        assert!(b.select_path("pda_map_dif.dds"));
        let first = b.selected.clone();
        b.step_selection(1);
        assert_ne!(b.selected, first);
        b.step_selection(-1);
        assert_eq!(b.selected, first);
        wait_for(&mut b, "a preview", 60, |b| b.preview.is_some());
    }
}
