//! The model behind the asset browser (`jc2 --browse`), free of any UI so it can be tested
//! headless: an [`Index`] of every file the [`GameFs`] can see, the folder [`Tree`] built
//! from the resolved names, a background [`ScanInfo`] per file (its real kind by content,
//! the members of a SARC container), the [`flatten`](Tree::flatten)ed rows an immediate-mode
//! list draws lazily, the [`search`] over names and kinds, and [`build_preview`], which
//! reads one file or container member and turns it into what the right-hand pane shows
//! (the same text `jc2tool prop / rbm / anim / collision` print, a decoded texture, a
//! container's member list, a hex dump).

use crate::animation::{InterleavedAnimation, Skeleton, SplineAnimation};
use crate::compress;
use crate::dds::DdsImage;
use crate::detect::{FileKind, detect};
use crate::hash::{file_name, name_hash};
use crate::havok::Packfile;
use crate::names::NameTable;
use crate::property::{Key, Node, PropertyFile, Variant};
use crate::rbm::RbmModel;
use crate::sarc::SmallArchive;
use crate::vfs::{GameFs, Located, Source};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::Read;

/// How many bytes of a text file the preview shows.
pub const TEXT_LIMIT: usize = 64 * 1024;
/// How many bytes the hex dump shows.
pub const HEX_BYTES: usize = 512;
/// Lines of a property tree / summary kept in a preview.
pub const LINE_LIMIT: usize = 100_000;
/// Hits a search lists (the rest is counted).
pub const SEARCH_LIMIT: usize = 5_000;
/// Models whose statistics a container preview adds up.
const MODEL_STATS_LIMIT: usize = 48;

// ───────────────────────────── the file index ─────────────────────────────

/// One file the engine can see.
#[derive(Clone, Debug)]
pub struct FileItem {
    pub hash: u32,
    /// Stored size (the zlib stream when compressed).
    pub size: u64,
    /// Index into [`Index::archives`].
    pub archive: usize,
    /// The resolved path (`gui\credits.blz`), when the hash is in the name lists.
    pub name: Option<String>,
    /// The bare file name for named files, the hash in hex otherwise.
    pub leaf: String,
    /// `name` (or the hash in hex) in lower case, for matching.
    pub lname: String,
}

impl FileItem {
    pub fn new(hash: u32, size: u64, archive: usize, name: Option<String>) -> Self {
        let leaf = match &name {
            Some(n) => file_name(n).to_string(),
            None => format!("{hash:08x}"),
        };
        let lname = name
            .as_deref()
            .map(str::to_ascii_lowercase)
            .unwrap_or_else(|| format!("{hash:08x}"));
        Self {
            hash,
            size,
            archive,
            name,
            leaf,
            lname,
        }
    }

    /// `name` or `unknown\<hash>`: what to print for this file.
    pub fn display(&self) -> String {
        match &self.name {
            Some(n) => n.clone(),
            None => format!("unknown\\{:08x}", self.hash),
        }
    }
}

/// Everything [`GameFs::entries`] lists, with names resolved.
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub items: Vec<FileItem>,
    /// `archives_win32/pc0` … `DLC/pc_60`, `dropzone`: the mounts, in precedence order.
    pub archives: Vec<String>,
    by_hash: HashMap<u32, usize>,
}

impl Index {
    pub fn from_fs(fs: &GameFs) -> Self {
        let mut idx = Index::default();
        let mut archive_of: HashMap<String, usize> = HashMap::new();
        for e in fs.entries() {
            let arch = fs.archive_name(e.source);
            let ai = match archive_of.get(arch) {
                Some(&i) => i,
                None => {
                    let i = idx.archives.len();
                    idx.archives.push(arch.to_string());
                    archive_of.insert(arch.to_string(), i);
                    i
                }
            };
            let name = fs.names.file(e.name_hash).map(str::to_string);
            idx.push(FileItem::new(e.name_hash, e.size, ai, name));
        }
        idx
    }

    /// For tests and tools that build an index by hand.
    pub fn push(&mut self, item: FileItem) -> usize {
        let i = self.items.len();
        self.by_hash.insert(item.hash, i);
        self.items.push(item);
        i
    }

    pub fn by_hash(&self, hash: u32) -> Option<usize> {
        self.by_hash.get(&hash).copied()
    }

    pub fn named(&self) -> usize {
        self.items.iter().filter(|i| i.name.is_some()).count()
    }
}

/// SARC containers by file name: `.ee .eez .bl .blz .nl .nlz .fl .flz`.
pub fn is_container_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    match l.rsplit_once('.') {
        Some((_, ext)) => ["ee", "eez", "bl", "blz", "nl", "nlz", "fl", "flz"].contains(&ext),
        None => false,
    }
}

/// A best guess at a file's kind from its extension, for what has not been read yet
/// (and for container members).
pub fn kind_from_name(name: &str) -> Option<FileKind> {
    let l = name.to_ascii_lowercase();
    let ext = l.rsplit_once('.')?.1;
    Some(match ext {
        "dds" => FileKind::Dds,
        "rbm" => FileKind::Rbm,
        "ee" | "eez" | "bl" | "blz" | "nl" | "nlz" | "fl" | "flz" | "sarc" => FileKind::Sarc,
        "bin" | "blo" | "asb" | "pfs" | "epe" | "rg" | "mvdoll" | "lod" | "batchlod" => {
            FileKind::Property
        }
        "pfx" | "hkx" | "ban" | "bsk" => FileKind::Havok,
        "fev" => FileKind::Fev,
        "fsb" => FileKind::Fsb,
        "csv" => FileKind::Csv,
        "txt" | "cfg" | "ini" | "log" => FileKind::Text,
        "xml" => FileKind::Xml,
        "bmp" => FileKind::Bmp,
        "bik" => FileKind::Bink,
        "adf" => FileKind::Adf,
        "agui" | "gui" => FileKind::Anark,
        "psh" | "vsh" | "shader" => FileKind::Shader,
        _ => return None,
    })
}

/// Every kind the kind filter offers, with its label.
pub const KINDS: &[(FileKind, &str)] = &[
    (FileKind::Sarc, "SARC container"),
    (FileKind::Dds, "DDS texture"),
    (FileKind::Rbm, "RBM model"),
    (FileKind::Property, "property file"),
    (FileKind::Blackboard, "blackboard (PCBB)"),
    (FileKind::Havok, "Havok"),
    (FileKind::Fev, "FMOD event (fev)"),
    (FileKind::Fsb, "FMOD bank (fsb)"),
    (FileKind::Csv, "CSV"),
    (FileKind::Text, "text"),
    (FileKind::Xml, "XML"),
    (FileKind::Anark, "Anark UI"),
    (FileKind::Adf, "ADF"),
    (FileKind::Shader, "shader"),
    (FileKind::Bmp, "BMP"),
    (FileKind::Bink, "Bink video"),
    (FileKind::Unknown, "unknown"),
];

pub fn kind_label(k: FileKind) -> &'static str {
    KINDS
        .iter()
        .find(|(kk, _)| *kk == k)
        .map(|(_, l)| *l)
        .unwrap_or("other")
}

// ───────────────────────────── the background scan ─────────────────────────────

/// A member of a SARC container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub size: u32,
}

/// What a peek at the start of a file says.
#[derive(Clone, Debug, PartialEq)]
pub struct ScanInfo {
    /// By content, after any zlib wrapper.
    pub kind: FileKind,
    pub compressed: bool,
    /// The members, for a SARC container.
    pub members: Vec<Member>,
}

/// The decoded start of `raw`: the first `limit` bytes after a zlib wrapper, whatever
/// that wrapper's total size.
pub fn inflated_prefix(raw: &[u8], limit: usize) -> (Vec<u8>, bool) {
    if compress::is_zlib(raw) {
        let mut out = Vec::new();
        // a stream that breaks halfway still gives what it decoded
        let _ = flate2::read::ZlibDecoder::new(raw)
            .take(limit as u64)
            .read_to_end(&mut out);
        (out, true)
    } else {
        (raw[..raw.len().min(limit)].to_vec(), false)
    }
}

fn raw_bytes<'a>(fs: &'a GameFs, loc: &Located) -> crate::Result<Cow<'a, [u8]>> {
    match loc.source {
        Source::Archive { archive, entry } => {
            let a = &fs.archives[archive];
            Ok(Cow::Borrowed(a.slice(&a.table.entries[entry])?))
        }
        Source::Dropzone => Ok(Cow::Owned(fs.read_hash(loc.name_hash)?)),
    }
}

/// Peek at one file: its kind by content and, for a SARC container, its member list —
/// from the start of the (partly inflated) stream only, so a whole game is cheap.
pub fn scan_entry(fs: &GameFs, loc: &Located) -> Option<ScanInfo> {
    let raw = raw_bytes(fs, loc).ok()?;
    let (head, compressed) = inflated_prefix(&raw, 4096);
    let kind = detect(&head);
    let mut members = Vec::new();
    if kind == FileKind::Sarc {
        let le = head[0..4] == [4, 0, 0, 0];
        let index_size = head.get(12..16).map(|b| {
            let a = [b[0], b[1], b[2], b[3]];
            if le {
                u32::from_le_bytes(a)
            } else {
                u32::from_be_bytes(a)
            }
        });
        if let Some(n) = index_size
            && (n as usize) < 16 << 20
        {
            let (prefix, _) = inflated_prefix(&raw, 16 + n as usize);
            if let Ok(s) = SmallArchive::parse(&prefix) {
                members = s
                    .entries
                    .into_iter()
                    .map(|e| Member {
                        name: e.name,
                        size: e.size,
                    })
                    .collect();
            }
        }
    }
    Some(ScanInfo {
        kind,
        compressed,
        members,
    })
}

/// The scan results so far, one slot per [`Index`] item.
#[derive(Clone, Debug, Default)]
pub struct ScanTable {
    slots: Vec<Option<ScanInfo>>,
    pub done: usize,
}

impl ScanTable {
    pub fn new(items: usize) -> Self {
        Self {
            slots: vec![None; items],
            done: 0,
        }
    }

    pub fn set(&mut self, item: usize, info: ScanInfo) {
        if let Some(slot) = self.slots.get_mut(item) {
            if slot.is_none() {
                self.done += 1;
            }
            *slot = Some(info);
        }
    }

    pub fn get(&self, item: usize) -> Option<&ScanInfo> {
        self.slots.get(item).and_then(|s| s.as_ref())
    }

    pub fn total(&self) -> usize {
        self.slots.len()
    }

    pub fn complete(&self) -> bool {
        self.done >= self.slots.len()
    }

    /// Synchronous scan of every item: for tests and small indexes.
    pub fn scan_all(fs: &GameFs, index: &Index) -> Self {
        let mut t = Self::new(index.items.len());
        for (i, it) in index.items.iter().enumerate() {
            if let Some(loc) = fs.locate_hash(it.hash)
                && let Some(info) = scan_entry(fs, &loc)
            {
                t.set(i, info);
            }
        }
        t
    }
}

/// The kind of an item: by content once scanned, by its name before.
pub fn item_kind(index: &Index, scan: &ScanTable, item: usize) -> Option<FileKind> {
    scan.get(item)
        .map(|s| s.kind)
        .or_else(|| index.items[item].name.as_deref().and_then(kind_from_name))
}

/// Whether an item opens into members in the tree.
pub fn is_container(index: &Index, scan: &ScanTable, item: usize) -> bool {
    match scan.get(item) {
        Some(s) => s.kind == FileKind::Sarc,
        None => index.items[item]
            .name
            .as_deref()
            .is_some_and(is_container_name),
    }
}

// ───────────────────────────── the folder tree ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Folders from the resolved names, unresolved hashes under `unknown`.
    Path,
    /// Archives first (`pc0` … `DLC/pc_60`, `dropzone`), then the same folders.
    Archive,
}

pub const TOP_LEVEL: &str = "(top level)";
pub const UNKNOWN: &str = "unknown";

#[derive(Clone, Debug, Default)]
pub struct Folder {
    pub name: String,
    pub parent: usize,
    pub folders: Vec<usize>,
    /// Item indexes, sorted by name.
    pub files: Vec<usize>,
    /// Files in this folder and below.
    pub count: usize,
}

#[derive(Clone, Debug)]
pub struct Tree {
    /// Folder 0 is the root.
    pub folders: Vec<Folder>,
    /// The folder each item sits in.
    pub item_folder: Vec<usize>,
}

impl Tree {
    pub fn build(index: &Index, group: Group) -> Self {
        let mut t = Tree {
            folders: vec![Folder {
                name: String::new(),
                parent: usize::MAX,
                ..Default::default()
            }],
            item_folder: vec![0; index.items.len()],
        };
        let mut lookup: HashMap<(usize, String), usize> = HashMap::new();
        for (i, it) in index.items.iter().enumerate() {
            let mut cur = 0;
            if group == Group::Archive {
                cur = t.child(&mut lookup, cur, &index.archives[it.archive]);
            }
            match &it.name {
                Some(path) => {
                    let comps: Vec<&str> = path.split(['\\', '/']).collect();
                    let dirs = &comps[..comps.len() - 1];
                    if dirs.iter().all(|d| d.is_empty()) && group == Group::Path {
                        cur = t.child(&mut lookup, cur, TOP_LEVEL);
                    }
                    for d in dirs.iter().filter(|d| !d.is_empty()) {
                        cur = t.child(&mut lookup, cur, d);
                    }
                }
                None => cur = t.child(&mut lookup, cur, UNKNOWN),
            }
            t.folders[cur].files.push(i);
            t.item_folder[i] = cur;
        }
        // children always come after their parents, so counts add up in one backwards pass
        for f in (0..t.folders.len()).rev() {
            let n = t.folders[f].files.len()
                + t.folders[f]
                    .folders
                    .iter()
                    .map(|&c| t.folders[c].count)
                    .sum::<usize>();
            t.folders[f].count = n;
        }
        let rank = |name: &str| match name {
            TOP_LEVEL => 1,
            UNKNOWN => 2,
            _ => 0,
        };
        for f in 0..t.folders.len() {
            let mut kids = std::mem::take(&mut t.folders[f].folders);
            kids.sort_by_key(|&c| {
                let n = &t.folders[c].name;
                (rank(n), n.to_ascii_lowercase())
            });
            t.folders[f].folders = kids;
            let mut files = std::mem::take(&mut t.folders[f].files);
            files.sort_by(|&a, &b| {
                let (a, b) = (&index.items[a], &index.items[b]);
                a.leaf
                    .to_ascii_lowercase()
                    .cmp(&b.leaf.to_ascii_lowercase())
                    .then(a.hash.cmp(&b.hash))
            });
            t.folders[f].files = files;
        }
        t
    }

    fn child(
        &mut self,
        lookup: &mut HashMap<(usize, String), usize>,
        parent: usize,
        name: &str,
    ) -> usize {
        let key = (parent, name.to_ascii_lowercase());
        if let Some(&c) = lookup.get(&key) {
            return c;
        }
        let c = self.folders.len();
        self.folders.push(Folder {
            name: name.to_string(),
            parent,
            ..Default::default()
        });
        self.folders[parent].folders.push(c);
        lookup.insert(key, c);
        c
    }

    /// The folder named `path` (`gui`, `gui\icons`), from the root.
    pub fn folder_at(&self, path: &str) -> Option<usize> {
        let mut cur = 0;
        for part in path.split(['\\', '/']).filter(|p| !p.is_empty()) {
            cur = *self.folders[cur]
                .folders
                .iter()
                .find(|&&c| self.folders[c].name.eq_ignore_ascii_case(part))?;
        }
        Some(cur)
    }

    /// The folders from the top down to (and including) the one holding `item`.
    pub fn ancestors_of(&self, item: usize) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut f = self.item_folder[item];
        while f != 0 && f != usize::MAX {
            chain.push(f);
            f = self.folders[f].parent;
        }
        chain.reverse();
        chain
    }
}

/// Which folders and containers are open.
#[derive(Clone, Debug, Default)]
pub struct TreeState {
    pub open_folders: HashSet<usize>,
    pub open_containers: HashSet<usize>,
}

impl TreeState {
    pub fn toggle_folder(&mut self, f: usize) {
        if !self.open_folders.remove(&f) {
            self.open_folders.insert(f);
        }
    }

    pub fn toggle_container(&mut self, item: usize) {
        if !self.open_containers.remove(&item) {
            self.open_containers.insert(item);
        }
    }

    /// Open every folder above `item`, so its row is in the list.
    pub fn reveal(&mut self, tree: &Tree, item: usize) {
        self.open_folders.extend(tree.ancestors_of(item));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Folder {
        folder: usize,
        open: bool,
    },
    File {
        item: usize,
    },
    Container {
        item: usize,
        open: bool,
    },
    /// Member `member` of the container `item`.
    Member {
        item: usize,
        member: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub depth: u16,
    pub kind: RowKind,
}

impl Tree {
    /// The rows of the open part of the tree, top to bottom. Closed folders contribute
    /// one row and none of their contents, which is what keeps 28k files cheap.
    pub fn flatten(&self, index: &Index, scan: &ScanTable, st: &TreeState) -> Vec<Row> {
        let mut out = Vec::new();
        self.walk(0, 0, index, scan, st, &mut out);
        out
    }

    fn walk(
        &self,
        folder: usize,
        depth: u16,
        index: &Index,
        scan: &ScanTable,
        st: &TreeState,
        out: &mut Vec<Row>,
    ) {
        let f = &self.folders[folder];
        for &c in &f.folders {
            let open = st.open_folders.contains(&c);
            out.push(Row {
                depth,
                kind: RowKind::Folder { folder: c, open },
            });
            if open {
                self.walk(c, depth + 1, index, scan, st, out);
            }
        }
        for &item in &f.files {
            if is_container(index, scan, item) {
                let open = st.open_containers.contains(&item);
                out.push(Row {
                    depth,
                    kind: RowKind::Container { item, open },
                });
                if open && let Some(s) = scan.get(item) {
                    for member in 0..s.members.len() {
                        out.push(Row {
                            depth: depth + 1,
                            kind: RowKind::Member { item, member },
                        });
                    }
                }
            } else {
                out.push(Row {
                    depth,
                    kind: RowKind::File { item },
                });
            }
        }
    }
}

// ───────────────────────────── search ─────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub text: String,
    pub kind: Option<FileKind>,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || self.kind.is_some()
    }

    fn terms(&self) -> Vec<String> {
        self.text
            .split_whitespace()
            .map(|t| t.trim_start_matches("0x").to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub item: usize,
    pub member: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub hits: Vec<Hit>,
    /// Matches in all, of which `hits` lists the first [`SEARCH_LIMIT`].
    pub total: usize,
}

impl SearchResults {
    fn add(&mut self, hit: Hit) {
        self.total += 1;
        if self.hits.len() < SEARCH_LIMIT {
            self.hits.push(hit);
        }
    }
}

fn contains_ci(hay: &str, needle_lower: &str) -> bool {
    let (h, n) = (hay.as_bytes(), needle_lower.as_bytes());
    if n.is_empty() {
        return true;
    }
    if n.len() > h.len() {
        return false;
    }
    h.windows(n.len())
        .any(|w| w.iter().zip(n).all(|(a, b)| a.to_ascii_lowercase() == *b))
}

/// Files whose name (or hash, or archive) contains every word of the filter, and — once
/// the scan has listed them — container members likewise, narrowed to a kind.
pub fn search(index: &Index, scan: &ScanTable, filter: &Filter) -> SearchResults {
    let terms = filter.terms();
    let mut res = SearchResults::default();
    for (i, it) in index.items.iter().enumerate() {
        let archive = &index.archives[it.archive];
        let kind_ok = match filter.kind {
            None => true,
            Some(k) => item_kind(index, scan, i) == Some(k),
        };
        let item_terms: Vec<bool> = terms
            .iter()
            .map(|t| it.lname.contains(t.as_str()) || contains_ci(archive, t))
            .collect();
        if kind_ok && item_terms.iter().all(|&b| b) {
            res.add(Hit {
                item: i,
                member: None,
            });
        }
        // members: some word must hit the member's own name, the rest may hit the container's
        if let Some(s) = scan.get(i) {
            if terms.is_empty() && filter.kind.is_none() {
                continue;
            }
            for (mi, m) in s.members.iter().enumerate() {
                if let Some(k) = filter.kind
                    && kind_from_name(&m.name) != Some(k)
                {
                    continue;
                }
                let hits: Vec<bool> = terms.iter().map(|t| contains_ci(&m.name, t)).collect();
                let ok = hits.iter().zip(&item_terms).all(|(m, i)| *m || *i)
                    && (terms.is_empty() || hits.iter().any(|&b| b));
                if ok {
                    res.add(Hit {
                        item: i,
                        member: Some(mi),
                    });
                }
            }
        }
    }
    res
}

// ───────────────────────────── targets ─────────────────────────────

/// Something to preview: a file, or a member of a container file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    File(u32),
    Member { container: u32, name: String },
}

impl Target {
    pub fn file_hash(&self) -> u32 {
        match self {
            Target::File(h) => *h,
            Target::Member { container, .. } => *container,
        }
    }
}

/// `pda_map_dif.dds`, `general.blz/gae06_parachute_dif.dds` (either slash), a path with
/// folders, or a hash in hex (`d0a17bcb`).
pub fn parse_target(fs: &GameFs, text: &str) -> Option<Target> {
    let text = text.trim().replace('\\', "/");
    if fs.contains(&text) {
        return Some(Target::File(name_hash(&text)));
    }
    // the longest prefix that is a known file is the container
    let mut end = text.len();
    while let Some(i) = text[..end].rfind('/') {
        if fs.contains(&text[..i]) {
            return Some(Target::Member {
                container: name_hash(&text[..i]),
                name: text[i + 1..].to_string(),
            });
        }
        end = i;
    }
    let hex = text.trim_start_matches("0x");
    if hex.len() == 8
        && let Ok(h) = u32::from_str_radix(hex, 16)
        && fs.locate_hash(h).is_some()
    {
        return Some(Target::File(h));
    }
    None
}

/// The text to show for a target: `container/member` with the resolved path.
pub fn target_label(fs: &GameFs, t: &Target) -> String {
    let file = |h: u32| {
        fs.names
            .file(h)
            .map(str::to_string)
            .unwrap_or_else(|| format!("unknown\\{h:08x}"))
    };
    match t {
        Target::File(h) => file(*h),
        Target::Member { container, name } => format!("{}/{name}", file(*container)),
    }
}

/// The bytes of a target with any zlib wrapper removed, plus what was stored.
pub struct Loaded {
    pub bytes: Vec<u8>,
    /// Size as stored (before inflating).
    pub stored: usize,
    pub compressed: bool,
}

pub fn load_target(fs: &GameFs, t: &Target) -> crate::Result<Loaded> {
    match t {
        Target::File(h) => {
            let raw = fs.read_hash(*h)?;
            let compressed = compress::is_zlib(&raw);
            let stored = raw.len();
            let bytes = match compress::maybe_inflate(&raw)? {
                Cow::Borrowed(_) => raw,
                Cow::Owned(v) => v,
            };
            Ok(Loaded {
                bytes,
                stored,
                compressed,
            })
        }
        Target::Member { container, name } => {
            let raw = fs.read_hash(*container)?;
            let blob = compress::maybe_inflate(&raw)?.into_owned();
            let sarc = SmallArchive::parse(&blob)?;
            let e = sarc
                .get(name)
                .ok_or_else(|| crate::Error::NotFound(name.clone()))?;
            let stored_bytes = sarc.data(&blob, e)?;
            Ok(Loaded {
                compressed: compress::is_zlib(stored_bytes),
                stored: stored_bytes.len(),
                bytes: compress::maybe_inflate(stored_bytes)?.into_owned(),
            })
        }
    }
}

/// The bytes `jc2tool extract` would write for a target: the file as stored in the
/// archive (`inflated` removes a zlib wrapper), or a member as stored in its container.
pub fn export_bytes(fs: &GameFs, t: &Target, inflated: bool) -> crate::Result<Vec<u8>> {
    match t {
        Target::File(h) if !inflated => fs.read_hash(*h),
        Target::Member { container, name } if !inflated => {
            let raw = fs.read_hash(*container)?;
            let blob = compress::maybe_inflate(&raw)?.into_owned();
            let sarc = SmallArchive::parse(&blob)?;
            let e = sarc
                .get(name)
                .ok_or_else(|| crate::Error::NotFound(name.clone()))?;
            Ok(sarc.data(&blob, e)?.to_vec())
        }
        _ => Ok(load_target(fs, t)?.bytes),
    }
}

/// Where `jc2tool extract` would put a target below an output folder (backslashes).
pub fn export_relative(fs: &GameFs, t: &Target, kind: FileKind) -> String {
    match t {
        Target::File(h) => fs.names.file_or_placeholder(*h, kind.extension()),
        Target::Member { container, name } => {
            let base = fs.names.file_or_placeholder(*container, "sarc");
            format!("{base}.d\\{}", name.replace(['\\', '/'], "_"))
        }
    }
}

// ───────────────────────────── previews ─────────────────────────────

/// Which pane the right side draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Texture,
    Model,
    Container,
    Properties,
    Text,
    Havok,
    Animation,
    Fev,
    Fsb,
    Hex,
}

impl PreviewKind {
    pub fn label(self) -> &'static str {
        match self {
            PreviewKind::Texture => "texture",
            PreviewKind::Model => "model",
            PreviewKind::Container => "container",
            PreviewKind::Properties => "property file",
            PreviewKind::Text => "text",
            PreviewKind::Havok => "Havok packfile",
            PreviewKind::Animation => "animation / skeleton",
            PreviewKind::Fev => "FMOD event project",
            PreviewKind::Fsb => "FMOD sound bank",
            PreviewKind::Hex => "binary (hex dump)",
        }
    }
}

/// The content kind of `data` and the preview that suits it. `name` helps where the
/// bytes alone do not say (Havok animations and skeletons, `.ban` / `.bsk`).
pub fn preview_kind(name: &str, data: &[u8]) -> (FileKind, PreviewKind) {
    let mut kind = detect(data);
    if kind == FileKind::Unknown && Packfile::is_packfile(data) {
        kind = FileKind::Havok;
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    let view = match kind {
        FileKind::Sarc => PreviewKind::Container,
        FileKind::Dds => PreviewKind::Texture,
        FileKind::Rbm => PreviewKind::Model,
        FileKind::Property | FileKind::Blackboard => PreviewKind::Properties,
        FileKind::Csv | FileKind::Text | FileKind::Xml => PreviewKind::Text,
        FileKind::Havok => {
            if ext == "ban" || ext == "bsk" || is_animation_packfile(data) {
                PreviewKind::Animation
            } else {
                PreviewKind::Havok
            }
        }
        FileKind::Fev => PreviewKind::Fev,
        FileKind::Fsb => PreviewKind::Fsb,
        _ => PreviewKind::Hex,
    };
    (kind, view)
}

fn is_animation_packfile(data: &[u8]) -> bool {
    Packfile::parse(data).is_ok_and(|pf| {
        pf.objects.iter().any(|o| {
            matches!(
                o.class.as_str(),
                "hkaSkeleton"
                    | "hkaSplineSkeletalAnimation"
                    | "hkaInterleavedSkeletalAnimation"
                    | "hkaAnimationContainer"
            )
        })
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemberRow {
    pub name: String,
    pub size: u32,
    pub kind: Option<FileKind>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SampleRow {
    pub index: usize,
    pub name: String,
    pub seconds: f32,
    pub hz: u32,
    pub channels: u16,
    pub size: usize,
    /// MPEG, so the browser can play it.
    pub playable: bool,
}

/// Everything the right pane needs to draw one target.
#[derive(Clone, Debug)]
pub struct Preview {
    pub target: Target,
    /// `gui\credits.blz`, `general.blz/gae06_parachute_dif.dds`.
    pub label: String,
    pub kind: FileKind,
    pub view: PreviewKind,
    pub compressed: bool,
    pub stored: usize,
    pub size: usize,
    /// Facts for the header, one per line.
    pub info: Vec<String>,
    /// A decodable texture (`PreviewKind::Texture`).
    pub texture: Option<DdsImage>,
    /// The property tree, a summary, the text, or the hex dump.
    pub lines: Vec<String>,
    /// A line limit or byte limit cut `lines` short.
    pub truncated: bool,
    /// A container's members.
    pub members: Vec<MemberRow>,
    /// A sound bank's samples.
    pub samples: Vec<SampleRow>,
    /// What `jc2 --model` takes to show this in 3D: `container.ee`, `container/part.rbm`
    /// or `part.rbm`. `None` when there is no model or it has no addressable name.
    pub model: Option<String>,
    /// A container with a vehicle doll.
    pub vehicle: bool,
    /// `31x14.nlz`: a location tile.
    pub tile: Option<(u32, u32)>,
    pub error: Option<String>,
}

/// `31x14.nlz` → `(31, 14)`.
pub fn parse_tile_name(leaf: &str) -> Option<(u32, u32)> {
    let l = leaf.to_ascii_lowercase();
    let stem = l.strip_suffix(".nlz").or_else(|| l.strip_suffix(".nl"))?;
    let (a, b) = stem.split_once('x')?;
    let (x, z) = (a.parse::<u32>().ok()?, b.parse::<u32>().ok()?);
    (x < 64 && z < 64).then_some((x, z))
}

/// A hex dump, 16 bytes a line: offset, hex, ASCII.
pub fn hex_dump(data: &[u8], limit: usize) -> Vec<String> {
    data[..data.len().min(limit)]
        .chunks(16)
        .enumerate()
        .map(|(i, c)| {
            let mut hex = String::new();
            for (j, b) in c.iter().enumerate() {
                if j == 8 {
                    hex.push(' ');
                }
                let _ = write!(hex, "{b:02x} ");
            }
            let ascii: String = c
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{:08x}  {:<49} |{ascii}|", i * 16, hex)
        })
        .collect()
}

fn key_name(names: &NameTable, k: &Key) -> String {
    match k {
        Key::Name(n) => n.clone(),
        Key::Hash(h) => names
            .property(*h)
            .map(str::to_string)
            .unwrap_or_else(|| format!("#{h:08x}")),
    }
}

fn variant_text(v: &Variant) -> String {
    match v {
        Variant::Int(i) => format!("{i}"),
        Variant::Float(f) => format!("{f}"),
        Variant::Str(s) => format!("\"{s}\""),
        Variant::Vec2(a) => format!("{a:?}"),
        Variant::Vec3(a) => format!("{a:?}"),
        Variant::Vec4(a) => format!("{a:?}"),
        Variant::Mat3x3(a) => format!("mat3x3{a:?}"),
        Variant::Mat4x3(a) => format!("mat4x3{a:?}"),
        Variant::Mat4x4(a) => format!("mat4x4{a:?}"),
        Variant::Ints(a) => format!("ints[{}]{:?}", a.len(), &a[..a.len().min(8)]),
        Variant::Floats(a) => format!("floats[{}]{:?}", a.len(), &a[..a.len().min(8)]),
    }
}

/// The property tree as indented lines, key hashes resolved to names (`#hash` otherwise):
/// the same text `jc2tool prop` prints. The flag says the line limit cut it short.
pub fn property_lines(names: &NameTable, pf: &PropertyFile, limit: usize) -> (Vec<String>, bool) {
    fn node(
        names: &NameTable,
        n: &Node,
        depth: usize,
        out: &mut Vec<String>,
        limit: usize,
    ) -> bool {
        let pad = "  ".repeat(depth);
        if let Some(t) = &n.tag {
            out.push(format!("{pad}@tag \"{t}\""));
        }
        for (k, v) in &n.values {
            if out.len() >= limit {
                return true;
            }
            out.push(format!("{pad}{} = {}", key_name(names, k), variant_text(v)));
        }
        for (k, c) in &n.children {
            if out.len() >= limit {
                return true;
            }
            out.push(format!("{pad}{} {{", key_name(names, k)));
            if node(names, c, depth + 1, out, limit) {
                return true;
            }
            out.push(format!("{pad}}}"));
        }
        false
    }
    let mut out = Vec::new();
    let mut cut = false;
    for (i, n) in pf.nodes.iter().enumerate() {
        out.push(format!("node[{i}]"));
        if node(names, n, 1, &mut out, limit) {
            cut = true;
            break;
        }
    }
    (out, cut)
}

/// The block summary `jc2tool rbm` prints for one model.
pub fn rbm_lines(label: &str, m: &RbmModel) -> Vec<String> {
    let mut out = vec![format!(
        "{label}: RBMDL {}.{}.{}  bbox {:.2?}..{:.2?}  {} blocks, {} verts, {} tris{}",
        m.version.0,
        m.version.1,
        m.version.2,
        m.bbox_min,
        m.bbox_max,
        m.blocks.len(),
        m.vertex_count(),
        m.triangle_count(),
        m.truncated_at
            .as_ref()
            .map(|(i, t)| format!("  [stopped at block {i}: unsupported type {t}]"))
            .unwrap_or_default()
    )];
    for (i, b) in m.blocks.iter().enumerate() {
        let bounds = b
            .mesh
            .bounds()
            .map(|(lo, hi)| format!("{:.2?}..{:.2?}", lo, hi))
            .unwrap_or_default();
        out.push(format!(
            "  [{i}] {} v{}  {} verts {} tris  normals={} uv0={}  {}",
            b.type_name,
            b.version,
            b.mesh.vertex_count(),
            b.mesh.triangle_count(),
            !b.mesh.normals.is_empty(),
            !b.mesh.uv0.is_empty(),
            bounds
        ));
        let tex: Vec<&str> = b
            .material
            .textures()
            .into_iter()
            .filter(|t| !t.is_empty())
            .collect();
        if !tex.is_empty() {
            out.push(format!("      textures: {}", tex.join(", ")));
        }
        if !b.instances.is_empty() {
            out.push(format!("      {} placed instances", b.instances.len()));
        }
    }
    out
}

fn havok_lines(data: &[u8]) -> Vec<String> {
    use crate::collision::{bodies, bounds};
    let mut out = Vec::new();
    let pf = match Packfile::parse(data) {
        Ok(p) => p,
        Err(e) => return vec![format!("not a readable Havok packfile: {e}")],
    };
    out.push(format!(
        "{} ({} bytes of data, {} objects, {} class names)",
        pf.version,
        pf.data().len(),
        pf.objects.len(),
        pf.classnames.len()
    ));
    for (vname, class, off) in pf.root_variants() {
        out.push(format!("root variant {vname:?}: {class} @{off}"));
    }
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for o in &pf.objects {
        *counts.entry(o.class.as_str()).or_default() += 1;
    }
    out.push(format!(
        "objects: {}",
        counts
            .iter()
            .map(|(c, n)| format!("{c}×{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    ));
    for (i, b) in bodies(&pf).iter().enumerate() {
        let t = b.transform.translation();
        let leaves: Vec<&'static str> = b.shape.flatten().iter().map(|(_, l)| l.kind()).collect();
        out.push(format!(
            "body {i}: at ({:.3} {:.3} {:.3})  shape {}  leaves: {}",
            t[0],
            t[1],
            t[2],
            b.shape.kind(),
            if leaves.is_empty() {
                "unsupported".to_string()
            } else {
                leaves.join(" ")
            }
        ));
        if let Some((lo, hi)) = bounds(b) {
            out.push(format!("  bounds {lo:?} .. {hi:?}"));
        }
    }
    out
}

fn animation_lines(data: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(sk) = Skeleton::parse(data) {
        out.push(format!("skeleton {:?}: {} bones", sk.name, sk.bones.len()));
        for (i, b) in sk.bones.iter().enumerate() {
            let mut depth = 0;
            let mut p = b.parent;
            while let Some(pi) = p {
                depth += 1;
                p = sk.bones[pi].parent;
            }
            let r = &sk.reference_pose[i];
            out.push(format!(
                "{i:3} {:indent$}{:<20} t=({:7.3} {:7.3} {:7.3})  q=({:6.3} {:6.3} {:6.3} {:6.3})",
                "",
                b.name,
                r.translation[0],
                r.translation[1],
                r.translation[2],
                r.rotation[0],
                r.rotation[1],
                r.rotation[2],
                r.rotation[3],
                indent = depth * 2
            ));
        }
        return out;
    }
    if let Ok(clip) = SplineAnimation::parse(data) {
        out.push(format!(
            "spline clip: {} tracks, {} frames, {:.3} s ({:.4} s/frame), {} block(s), root motion {}",
            clip.track_count(),
            clip.num_frames,
            clip.duration,
            clip.frame_duration,
            clip.blocks.len(),
            match clip.root_speed() {
                Some(v) => format!("{v:.2} m/s"),
                None => "none".into(),
            }
        ));
        if let Some(first) = clip.blocks.first() {
            for (ti, t) in first.iter().enumerate() {
                let kind = |knots: usize| if knots == 0 { "static" } else { "spline" };
                out.push(format!(
                    "{ti:3} {:<20} pos {:<6} rot {:<6} scale {:<6}",
                    clip.track_names.get(ti).map_or("?", String::as_str),
                    kind(t.position.knots.len()),
                    kind(t.rotation.knots.len()),
                    kind(t.scale.knots.len()),
                ));
            }
        }
        return out;
    }
    match InterleavedAnimation::parse(data) {
        Ok(clip) => {
            out.push(format!(
                "interleaved clip: {} tracks, {} frames, {:.3} s",
                clip.track_names.len(),
                clip.frames.len(),
                clip.duration
            ));
            for (ti, n) in clip.track_names.iter().enumerate() {
                out.push(format!("{ti:3} {n}"));
            }
        }
        Err(e) => out.push(format!("not a skeleton or animation clip: {e}")),
    }
    out
}

fn fev_lines(data: &[u8]) -> Vec<String> {
    use crate::fev::EventFile;
    let f = match EventFile::parse(data) {
        Ok(f) => f,
        Err(e) => return vec![format!("not a readable FMOD event project: {e}")],
    };
    let events = f.events();
    let mut out = vec![format!(
        "project {:?}: {} events, {} sound definitions, {} wave banks",
        f.project,
        events.len(),
        f.sound_defs.len(),
        f.banks.len()
    )];
    for b in &f.banks {
        out.push(format!("  bank {} → {}", b.name, b.file_name()));
    }
    out.push(String::new());
    for e in events.iter().take(2000) {
        out.push(format!(
            "{:<56} vol {:.2}  {} layers  {}",
            e.path,
            e.volume,
            e.layers.len(),
            if e.is_3d() {
                format!("3D {}..{} m", e.min_distance, e.max_distance)
            } else {
                "2D".to_string()
            }
        ));
    }
    if events.len() > 2000 {
        out.push(format!("… {} more events", events.len() - 2000));
    }
    out
}

fn fsb_rows(data: &[u8]) -> Result<Vec<SampleRow>, String> {
    let bank = crate::fsb::SoundBank::parse(data).map_err(|e| e.to_string())?;
    Ok(bank
        .samples
        .iter()
        .enumerate()
        .map(|(i, s)| SampleRow {
            index: i,
            name: s.name.clone(),
            seconds: s.length_samples as f32 / s.frequency.max(1) as f32,
            hz: s.frequency,
            channels: s.channels,
            size: s.size,
            playable: s.is_mpeg(),
        })
        .collect())
}

/// An FSB sample as a plain MP3 stream, for the play button.
pub fn fsb_sample_mp3(data: &[u8], index: usize) -> Option<Vec<u8>> {
    let bank = crate::fsb::SoundBank::parse(data).ok()?;
    if index >= bank.samples.len() {
        return None;
    }
    bank.mpeg(data, index)
}

fn container_preview(p: &mut Preview, data: &[u8], bare_name: Option<&str>) {
    let sarc = match SmallArchive::parse(data) {
        Ok(s) => s,
        Err(e) => {
            p.error = Some(e.to_string());
            return;
        }
    };
    let mut rbms = 0usize;
    for e in &sarc.entries {
        let lower = e.name.to_ascii_lowercase();
        if lower.ends_with(".rbm") {
            rbms += 1;
        }
        if lower.ends_with(".mvdoll") {
            p.vehicle = true;
        }
        let kind = sarc.data(data, e).ok().and_then(|b| {
            let (head, _) = inflated_prefix(b, 512);
            match detect(&head) {
                FileKind::Unknown | FileKind::Empty => kind_from_name(&e.name),
                k => Some(k),
            }
        });
        p.members.push(MemberRow {
            name: e.name.clone(),
            size: e.size,
            kind,
        });
    }
    p.info.push(format!(
        "SARC v{}, {} members{}",
        sarc.version,
        sarc.entries.len(),
        if p.vehicle {
            ", a vehicle (has a doll)"
        } else {
            ""
        }
    ));
    if rbms > 0 {
        p.model = bare_name.map(str::to_string);
        p.info.push(format!("{rbms} models (.rbm)"));
        // statistics of the first few models
        let (mut blocks, mut verts, mut tris, mut parsed) = (0, 0, 0, 0);
        let mut lines = Vec::new();
        for e in sarc
            .entries
            .iter()
            .filter(|e| e.name.to_ascii_lowercase().ends_with(".rbm"))
            .take(MODEL_STATS_LIMIT)
        {
            let Ok(b) = sarc.data(data, e) else { continue };
            let Ok(b) = compress::maybe_inflate(b) else {
                continue;
            };
            match RbmModel::parse(&b) {
                Ok(m) => {
                    parsed += 1;
                    blocks += m.blocks.len();
                    verts += m.vertex_count();
                    tris += m.triangle_count();
                    lines.extend(rbm_lines(&e.name, &m));
                }
                Err(err) => lines.push(format!("{}: {err}", e.name)),
            }
        }
        p.info.push(format!(
            "{parsed}{} models parsed: {blocks} blocks, {verts} vertices, {tris} triangles",
            if rbms > MODEL_STATS_LIMIT {
                format!(" of the first {MODEL_STATS_LIMIT}")
            } else {
                String::new()
            }
        ));
        p.lines = lines;
    }
    if p.vehicle
        && let Some(doll) = sarc
            .entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase().ends_with(".mvdoll"))
            .and_then(|e| sarc.data(data, e).ok())
            .and_then(|d| crate::vehicle::VehicleDoll::parse(d).ok())
    {
        p.info.push(format!(
            "vehicle doll: {} parts, {} axles",
            doll.parts.len(),
            doll.axles.len()
        ));
    }
}

/// Read a target and prepare everything the right pane shows. Never fails: a problem
/// is reported in `error`.
pub fn build_preview(fs: &GameFs, target: &Target) -> Preview {
    let mut p = Preview {
        target: target.clone(),
        label: target_label(fs, target),
        kind: FileKind::Unknown,
        view: PreviewKind::Hex,
        compressed: false,
        stored: 0,
        size: 0,
        info: Vec::new(),
        texture: None,
        lines: Vec::new(),
        truncated: false,
        members: Vec::new(),
        samples: Vec::new(),
        model: None,
        vehicle: false,
        tile: None,
        error: None,
    };
    let loaded = match load_target(fs, target) {
        Ok(l) => l,
        Err(e) => {
            p.error = Some(e.to_string());
            return p;
        }
    };
    let data = loaded.bytes;
    p.compressed = loaded.compressed;
    p.stored = loaded.stored;
    p.size = data.len();
    let bare = match target {
        Target::File(h) => fs.names.file(*h).map(|n| file_name(n).to_string()),
        Target::Member { .. } => None,
    };
    let leaf_name = match target {
        Target::File(_) => bare.clone().unwrap_or_default(),
        Target::Member { name, .. } => name.clone(),
    };
    let (kind, view) = preview_kind(&leaf_name, &data);
    p.kind = kind;
    p.view = view;
    p.tile = parse_tile_name(&leaf_name);
    match view {
        PreviewKind::Texture => match DdsImage::parse(&data) {
            Ok(img) => {
                p.info.push(format!(
                    "{} × {}   {:?}   {} mip level{}",
                    img.width,
                    img.height,
                    img.format,
                    img.mips,
                    if img.mips == 1 { "" } else { "s" }
                ));
                if img.decode_rgba(0).is_some() {
                    p.texture = Some(img);
                } else {
                    p.info.push("this pixel format has no CPU decoder".into());
                    p.lines = hex_dump(&data, HEX_BYTES);
                }
            }
            Err(e) => {
                p.error = Some(e.to_string());
                p.lines = hex_dump(&data, HEX_BYTES);
            }
        },
        PreviewKind::Model => {
            match RbmModel::parse(&data) {
                Ok(m) => {
                    p.lines = rbm_lines(&leaf_name, &m);
                    p.info.push(format!(
                        "{} blocks, {} vertices, {} triangles",
                        m.blocks.len(),
                        m.vertex_count(),
                        m.triangle_count()
                    ));
                }
                Err(e) => p.error = Some(e.to_string()),
            }
            p.model = match target {
                Target::File(_) => bare.clone(),
                Target::Member { container, name } => fs
                    .names
                    .file(*container)
                    .map(|c| format!("{}/{name}", file_name(c))),
            };
        }
        PreviewKind::Container => container_preview(&mut p, &data, bare.as_deref()),
        PreviewKind::Properties => match PropertyFile::parse_any(&data) {
            Ok(pf) => {
                let (lines, cut) = property_lines(&fs.names, &pf, LINE_LIMIT);
                p.info.push(format!(
                    "{} node{}, {} lines",
                    pf.nodes.len(),
                    if pf.nodes.len() == 1 { "" } else { "s" },
                    lines.len()
                ));
                p.lines = lines;
                p.truncated = cut;
            }
            Err(e) => {
                p.error = Some(format!("property parse: {e}"));
                p.lines = hex_dump(&data, HEX_BYTES);
            }
        },
        PreviewKind::Text => {
            let cut = data.len() > TEXT_LIMIT;
            let text = String::from_utf8_lossy(&data[..data.len().min(TEXT_LIMIT)]);
            p.lines = text.lines().map(str::to_string).collect();
            p.truncated = cut;
        }
        PreviewKind::Havok => p.lines = havok_lines(&data),
        PreviewKind::Animation => p.lines = animation_lines(&data),
        PreviewKind::Fev => p.lines = fev_lines(&data),
        PreviewKind::Fsb => match fsb_rows(&data) {
            Ok(rows) => p.samples = rows,
            Err(e) => {
                p.error = Some(e);
                p.lines = hex_dump(&data, HEX_BYTES);
            }
        },
        PreviewKind::Hex => p.lines = hex_dump(&data, HEX_BYTES),
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dds::{DdsFormat, write_dds};

    fn item(i: &mut Index, name: Option<&str>, hash: u32, archive: usize) -> usize {
        i.push(FileItem::new(hash, 100, archive, name.map(str::to_string)))
    }

    fn sample_index() -> Index {
        let mut i = Index {
            archives: vec!["archives_win32/pc0".into(), "dropzone".into()],
            ..Default::default()
        };
        item(&mut i, Some("gui\\credits.blz"), 1, 0);
        item(&mut i, Some("gui\\icons\\map_icon.dds"), 2, 0);
        item(&mut i, Some("Gui\\icons\\other.dds"), 3, 0);
        item(&mut i, Some("pda_map_dif.dds"), 4, 0);
        item(&mut i, Some("global\\areasets\\general.blz"), 5, 1);
        item(&mut i, None, 0xdead_beef, 0);
        item(&mut i, None, 0x0000_0042, 1);
        i
    }

    fn rbm_bytes() -> Vec<u8> {
        let mut b = 5u32.to_le_bytes().to_vec();
        b.extend_from_slice(b"RBMDL");
        for v in [1i32, 13, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        for _ in 0..6 {
            b.extend_from_slice(&0f32.to_le_bytes());
        }
        b.extend_from_slice(&0i32.to_le_bytes());
        b
    }

    #[test]
    fn tree_groups_by_folder_and_unknown() {
        let idx = sample_index();
        let t = Tree::build(&idx, Group::Path);
        let root = &t.folders[0];
        let names: Vec<&str> = root
            .folders
            .iter()
            .map(|&c| t.folders[c].name.as_str())
            .collect();
        // folders alphabetical (case-insensitive: `Gui` and `gui` are one), then the
        // top level, then the unresolved
        assert_eq!(names, ["global", "gui", TOP_LEVEL, UNKNOWN]);
        let gui = t.folder_at("gui").unwrap();
        assert_eq!(t.folders[gui].count, 3);
        assert_eq!(t.folders[gui].files.len(), 1);
        let icons = t.folder_at("gui\\icons").unwrap();
        assert_eq!(t.folders[icons].files.len(), 2);
        let unknown = t.folder_at(UNKNOWN).unwrap();
        assert_eq!(t.folders[unknown].count, 2);
        let top = t.folder_at(TOP_LEVEL).unwrap();
        assert_eq!(idx.items[t.folders[top].files[0]].leaf, "pda_map_dif.dds");
        assert_eq!(t.folders[0].count, idx.items.len());
        assert_eq!(t.item_folder[1], icons);
        assert_eq!(t.ancestors_of(1), vec![gui, icons]);
    }

    #[test]
    fn tree_groups_by_archive() {
        let idx = sample_index();
        let t = Tree::build(&idx, Group::Archive);
        let top: Vec<&str> = t.folders[0]
            .folders
            .iter()
            .map(|&c| t.folders[c].name.as_str())
            .collect();
        assert_eq!(top, ["archives_win32/pc0", "dropzone"]);
        let dz = t.folder_at("dropzone").unwrap();
        assert_eq!(t.folders[dz].count, 2);
        assert!(t.folder_at("dropzone\\global\\areasets").is_some());
        assert!(t.folder_at("dropzone\\unknown").is_some());
    }

    #[test]
    fn closed_folders_cost_one_row() {
        let idx = sample_index();
        let t = Tree::build(&idx, Group::Path);
        let scan = ScanTable::new(idx.items.len());
        let mut st = TreeState::default();
        let rows = t.flatten(&idx, &scan, &st);
        assert_eq!(rows.len(), 4); // global, gui, (top level), unknown
        assert!(
            rows.iter()
                .all(|r| matches!(r.kind, RowKind::Folder { open: false, .. }))
        );
        // opening what holds credits.blz (a container by its name) shows gui's contents only
        st.reveal(&t, 0);
        let rows = t.flatten(&idx, &scan, &st);
        assert!(rows.iter().any(|r| r.kind
            == RowKind::Container {
                item: 0,
                open: false
            }
            && r.depth == 1));
        assert!(!rows.iter().any(|r| r.kind == RowKind::File { item: 1 }));
        st.open_folders.insert(t.folder_at("gui\\icons").unwrap());
        let rows = t.flatten(&idx, &scan, &st);
        assert!(
            rows.iter()
                .any(|r| r.kind == RowKind::File { item: 1 } && r.depth == 2)
        );
    }

    #[test]
    fn containers_expand_into_members() {
        let idx = sample_index();
        let t = Tree::build(&idx, Group::Path);
        let mut scan = ScanTable::new(idx.items.len());
        scan.set(
            0,
            ScanInfo {
                kind: FileKind::Sarc,
                compressed: true,
                members: vec![
                    Member {
                        name: "a.dds".into(),
                        size: 10,
                    },
                    Member {
                        name: "b.rbm".into(),
                        size: 20,
                    },
                ],
            },
        );
        let mut st = TreeState::default();
        st.reveal(&t, 0);
        st.toggle_container(0);
        let rows = t.flatten(&idx, &scan, &st);
        let at = rows
            .iter()
            .position(|r| {
                matches!(
                    r.kind,
                    RowKind::Container {
                        item: 0,
                        open: true
                    }
                )
            })
            .unwrap();
        assert_eq!(rows[at + 1].kind, RowKind::Member { item: 0, member: 0 });
        assert_eq!(rows[at + 2].kind, RowKind::Member { item: 0, member: 1 });
        assert_eq!(rows[at + 2].depth, rows[at].depth + 1);
    }

    #[test]
    fn filter_finds_pda_map() {
        let idx = sample_index();
        let mut scan = ScanTable::new(idx.items.len());
        scan.set(
            4,
            ScanInfo {
                kind: FileKind::Sarc,
                compressed: true,
                members: vec![Member {
                    name: "gae06_parachute_dif.dds".into(),
                    size: 10,
                }],
            },
        );
        let f = |text: &str, kind: Option<FileKind>| {
            search(
                &idx,
                &scan,
                &Filter {
                    text: text.into(),
                    kind,
                },
            )
        };
        let r = f("PDA_MAP", None);
        assert_eq!(
            r.hits,
            vec![Hit {
                item: 3,
                member: None
            }]
        );
        // by kind: two icons, pda_map and the parachute member
        assert_eq!(f("", Some(FileKind::Dds)).total, 4);
        // a member by name, and a word of its container
        let r = f("parachute general", None);
        assert_eq!(
            r.hits,
            vec![Hit {
                item: 4,
                member: Some(0)
            }]
        );
        // by hash
        let r = f("0xdeadbeef", None);
        assert_eq!(
            r.hits,
            vec![Hit {
                item: 5,
                member: None
            }]
        );
        // by archive
        assert_eq!(f("dropzone", None).total, 2);
        // an unread, unnamed file has no kind to filter on
        assert_eq!(f("dropzone", Some(FileKind::Unknown)).total, 0);
        assert!(!Filter::default().is_active());
    }

    #[test]
    fn kinds_pick_the_preview() {
        let dds = write_dds(8, 8, 1, DdsFormat::Argb8888, &[0u8; 8 * 8 * 4]);
        assert_eq!(
            preview_kind("x.dds", &dds),
            (FileKind::Dds, PreviewKind::Texture)
        );
        assert_eq!(
            preview_kind("x.rbm", &rbm_bytes()),
            (FileKind::Rbm, PreviewKind::Model)
        );
        // the shape of a shipped file: a root with by-hash children and values
        let mut inner = Node::default();
        inner
            .values
            .push((Key::Name("action".into()), Variant::Int(7)));
        let mut root = Node::default();
        root.children.push((Key::Hash(0xdead_beef), inner));
        root.values
            .push((Key::Hash(1), Variant::Vec3([1.0, 2.0, 3.0])));
        let pf = PropertyFile { nodes: vec![root] };
        assert_eq!(
            preview_kind("x.bin", &pf.to_bytes()),
            (FileKind::Property, PreviewKind::Properties)
        );
        assert_eq!(
            preview_kind("x.csv", b"id,name\n1,two\n"),
            (FileKind::Csv, PreviewKind::Text)
        );
        let sarc = SmallArchive::build(&[("a.bin", b"hello")], 2);
        assert_eq!(
            preview_kind("x.ee", &sarc),
            (FileKind::Sarc, PreviewKind::Container)
        );
        assert_eq!(
            preview_kind("x.dat", &[0x00, 0x9f, 0x00, 0xff, 0x01, 0x02, 0x80, 0x90]),
            (FileKind::Unknown, PreviewKind::Hex)
        );
        assert_eq!(parse_tile_name("31x14.nlz"), Some((31, 14)));
        assert_eq!(parse_tile_name("areaset07.blz"), None);
        assert!(is_container_name("lave.v046_buscoach.ee"));
        assert!(!is_container_name("pda_map_dif.dds"));
        assert_eq!(kind_from_name("A.DDS"), Some(FileKind::Dds));
    }

    #[test]
    fn hex_dump_shape() {
        let l = hex_dump(b"JC2 hello, world!", 512);
        assert_eq!(l.len(), 2);
        assert!(l[0].starts_with("00000000  4a 43 32 20"));
        assert!(l[0].ends_with("|JC2 hello, world|"));
        assert_eq!(hex_dump(&[0; 2000], HEX_BYTES).len(), 32);
    }

    #[test]
    fn partial_inflate_keeps_the_start() {
        let big = vec![7u8; 1 << 20];
        let z = compress::deflate(&big, 6);
        let (head, compressed) = inflated_prefix(&z, 100);
        assert!(compressed);
        assert_eq!(head.len(), 100);
        let (head, compressed) = inflated_prefix(b"DDS plain", 4);
        assert!(!compressed);
        assert_eq!(head, b"DDS ");
    }

    #[test]
    fn property_lines_resolve_names() {
        let mut names = NameTable::empty();
        names.add_property("speed");
        let pf = PropertyFile {
            nodes: vec![Node {
                tag: Some("t".into()),
                children: vec![(
                    Key::Name("kid".into()),
                    Node {
                        tag: None,
                        children: vec![],
                        values: vec![(Key::Name("name".into()), Variant::Str("x".into()))],
                    },
                )],
                values: vec![
                    (
                        Key::Hash(crate::hash::hash_str("speed")),
                        Variant::Float(2.0),
                    ),
                    (Key::Hash(0x1234_5678), Variant::Int(3)),
                ],
            }],
        };
        let (lines, cut) = property_lines(&names, &pf, 100);
        assert!(!cut);
        assert_eq!(
            lines,
            [
                "node[0]",
                "  @tag \"t\"",
                "  speed = 2",
                "  #12345678 = 3",
                "  kid {",
                "    name = \"x\"",
                "  }"
            ]
        );
        assert!(property_lines(&names, &pf, 3).1);
    }

    // ── against the real install (JC2_TEST_DIR) ──

    fn test_fs() -> Option<GameFs> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        GameFs::open(dir).ok()
    }

    #[test]
    fn real_index_and_filter() {
        let Some(fs) = test_fs() else { return };
        let idx = Index::from_fs(&fs);
        assert!(idx.items.len() > 20_000);
        assert!(idx.archives.iter().any(|a| a.ends_with("pc0")));
        let tree = Tree::build(&idx, Group::Path);
        assert_eq!(tree.folders[0].count, idx.items.len());
        assert!(tree.folder_at(UNKNOWN).is_some());
        let scan = ScanTable::new(idx.items.len());
        let r = search(
            &idx,
            &scan,
            &Filter {
                text: "pda_map_dif.dds".into(),
                kind: None,
            },
        );
        assert_eq!(r.total, 1);
        assert_eq!(idx.items[r.hits[0].item].leaf, "pda_map_dif.dds");
        // the lazy tree: the closed top level is a handful of rows, not 28k
        let rows = tree.flatten(&idx, &scan, &TreeState::default());
        assert!(rows.len() < 200, "{}", rows.len());
    }

    #[test]
    fn real_container_lists_its_members() {
        let Some(fs) = test_fs() else { return };
        let loc = fs.locate("general.blz").unwrap();
        let info = scan_entry(&fs, &loc).unwrap();
        assert_eq!(info.kind, FileKind::Sarc);
        assert!(info.compressed);
        assert!(
            info.members
                .iter()
                .any(|m| m.name == "gae06_parachute_dif.dds")
        );
        // the prefix listing agrees with a full parse
        let full = fs.read_inflated("general.blz").unwrap();
        let sarc = SmallArchive::parse(&full).unwrap();
        assert_eq!(sarc.entries.len(), info.members.len());
        // and through the preview
        let t = parse_target(&fs, "general.blz").unwrap();
        let p = build_preview(&fs, &t);
        assert_eq!(p.view, PreviewKind::Container);
        assert_eq!(p.members.len(), sarc.entries.len());
        assert!(
            p.members
                .iter()
                .any(|m| m.name == "gae06_parachute_dif.dds" && m.kind == Some(FileKind::Dds))
        );
    }

    #[test]
    fn real_previews() {
        let Some(fs) = test_fs() else { return };
        // a texture inside a container
        let t = parse_target(&fs, "general.blz/gae06_parachute_dif.dds").unwrap();
        assert!(matches!(t, Target::Member { .. }));
        let p = build_preview(&fs, &t);
        assert_eq!(p.view, PreviewKind::Texture, "{:?}", p.error);
        let img = p.texture.as_ref().unwrap();
        assert!(img.width >= 64 && img.decode_rgba(0).is_some());
        // a top-level texture
        let p = build_preview(&fs, &parse_target(&fs, "pda_map_dif.dds").unwrap());
        assert_eq!(p.view, PreviewKind::Texture);
        // a model in a vehicle container
        let t = parse_target(&fs, "lave.v046_buscoach.ee/v046-body_m_lod1.rbm").unwrap();
        let p = build_preview(&fs, &t);
        assert_eq!(p.view, PreviewKind::Model, "{:?}", p.error);
        assert_eq!(
            p.model.as_deref(),
            Some("lave.v046_buscoach.ee/v046-body_m_lod1.rbm")
        );
        assert!(p.lines[0].contains("4 blocks"), "{}", p.lines[0]);
        assert!(p.lines.iter().any(|l| l.contains("textures:")));
        // the vehicle container
        let p = build_preview(&fs, &parse_target(&fs, "lave.v046_buscoach.ee").unwrap());
        assert!(p.vehicle);
        assert_eq!(p.model.as_deref(), Some("lave.v046_buscoach.ee"));
        // a property file, with resolved keys
        let p = build_preview(&fs, &parse_target(&fs, "camera_settings.bin").unwrap());
        assert_eq!(p.view, PreviewKind::Properties, "{:?}", p.error);
        assert!(p.lines.len() > 5);
        // a CSV is text
        let p = build_preview(&fs, &parse_target(&fs, "text\\button_icons.csv").unwrap());
        assert_eq!((p.kind, p.view), (FileKind::Csv, PreviewKind::Text));
        assert!(p.lines.len() > 3 && p.lines[0].contains(','));
        // a sound bank lists its samples, a Havok file its objects
        let p = build_preview(&fs, &parse_target(&fs, "gui.fsb").unwrap());
        assert_eq!(p.view, PreviewKind::Fsb);
        assert!(p.samples.len() > 10 && p.samples[0].playable);
        let t = parse_target(&fs, "31x14.nlz/go098_lod1-a_col.pfx").unwrap();
        let p = build_preview(&fs, &t);
        assert_eq!(p.view, PreviewKind::Havok, "{:?}", p.error);
        let t = parse_target(&fs, "animations.blz/rico_open_chute.ban");
        if let Some(t) = t {
            let p = build_preview(&fs, &t);
            assert_eq!(p.view, PreviewKind::Animation, "{:?}", p.error);
            assert!(p.lines[0].contains("clip"), "{}", p.lines[0]);
        }
        // a location tile
        let p = build_preview(&fs, &parse_target(&fs, "31x14.nlz").unwrap());
        assert_eq!(p.tile, Some((31, 14)));
        // export: the stored bytes of a file are the archive's
        let t = parse_target(&fs, "pda_map_dif.dds").unwrap();
        let raw = export_bytes(&fs, &t, false).unwrap();
        assert_eq!(raw, fs.read("pda_map_dif.dds").unwrap());
        assert!(export_bytes(&fs, &t, true).unwrap().starts_with(b"DDS "));
        assert_eq!(export_relative(&fs, &t, FileKind::Dds), "pda_map_dif.dds");
    }

    #[test]
    fn real_paths_parse() {
        let Some(fs) = test_fs() else { return };
        assert!(parse_target(&fs, "no_such_file.dds").is_none());
        assert_eq!(
            parse_target(&fs, "d0a17bcb"),
            Some(Target::File(0xd0a1_7bcb))
        );
    }
}
