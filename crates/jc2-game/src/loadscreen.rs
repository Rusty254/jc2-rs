//! Loading screen and stability gate (JC2-loadscreen task dec4b1ed).
//!
//! A black screen with "LOADING" is shown, and the clock held (`Time<Virtual>` paused), at
//! start-up and after any jump of the player far from where he was (fast travel, respawn at
//! the start, mission teleport), until the world around him has streamed in: the tiles in a
//! ring around the focus are loaded, none of their models is still waiting for the asset
//! server, and that has held for a few frames. Then the clock runs and the screen goes.
//!
//! Nothing here is native-proven. The retail executable has no decoded loading-screen
//! state (`research/formats/location-activation-next.md` gap 2: the CLocation load stages
//! and per-frame budget are not decoded), so the screen's look and every threshold below are
//! labelled non-native. The readiness query only reads the streamer ([`Streamer::loaded`],
//! each tile's `pending` list); it does not touch the streaming internals.

use crate::character::{Hero, HeroLoading};
use crate::coords::engine_xz;
use crate::vehicle::Occupied;
use crate::world::{FlyCamera, Streamer, WorldConfig};
use bevy::prelude::*;
use jc2_formats::terrain::{TILES_PER_SIDE, tile_index};

/// Tiles in every direction around the focus that must be in (1 → 3 x 3 tiles, 1.5 km
/// across). Non-native; clamped to the streamer's own radius.
const READY_RADIUS: u32 = 1;
/// Consecutive ready frames before the screen goes (non-native).
const STABLE_FRAMES: u32 = 8;
/// The screen stays at least this many frames, so it never flashes (non-native).
const MIN_FRAMES: u32 = 12;
/// Give up waiting after this much real time and go on anyway (non-native).
const TIMEOUT_SECS: f32 = 45.0;
/// A move of the focus by more than this in one frame is a teleport (metres; a tile is 512 m
/// and nothing in the game covers that in a frame).
const JUMP_DISTANCE: f32 = 600.0;

/// Why the screen is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadReason {
    Startup,
    FastTravel,
    Teleport,
}

/// The loading state. `active` holds the clock and shows the screen.
#[derive(Resource, Debug)]
pub struct LoadScreen {
    pub active: bool,
    pub reason: LoadReason,
    gate: Gate,
    /// Real time (s) the current load started.
    started: Option<f32>,
    /// The clock was paused by this screen (so it unpauses it).
    paused: bool,
    /// The focus last frame, for the jump detector.
    last_focus: Option<Vec3>,
    progress: Progress,
}

impl Default for LoadScreen {
    fn default() -> Self {
        Self {
            active: true,
            reason: LoadReason::Startup,
            gate: Gate::default(),
            started: None,
            paused: false,
            last_focus: None,
            progress: Progress::default(),
        }
    }
}

impl LoadScreen {
    /// Ask for the screen (a fast travel, a teleport): the next frame starts holding.
    pub fn request(&mut self, reason: LoadReason) {
        if !self.active {
            self.active = true;
            self.reason = reason;
            self.gate = Gate::default();
            self.started = None; // set from the clock by `drive`
            self.last_focus = None;
        }
    }
}

/// What is wanted and what is in, for the screen's text and the ready test.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Progress {
    needed: usize,
    loaded: usize,
    /// Models of the needed tiles still waiting for the asset server.
    pending: usize,
}

impl Progress {
    fn ready(&self) -> bool {
        self.needed > 0 && self.loaded == self.needed && self.pending == 0
    }
}

/// The frame counting: ready for [`STABLE_FRAMES`] in a row, shown for [`MIN_FRAMES`].
#[derive(Clone, Copy, Debug, Default)]
struct Gate {
    frames: u32,
    stable: u32,
}

impl Gate {
    /// One frame; true once the world may be shown.
    fn step(&mut self, ready: bool) -> bool {
        self.frames += 1;
        self.stable = if ready { self.stable + 1 } else { 0 };
        self.stable >= STABLE_FRAMES && self.frames >= MIN_FRAMES
    }
}

/// The tiles within `radius` of `centre`, inside the world.
fn ring(centre: (u32, u32), radius: u32) -> Vec<(u32, u32)> {
    let r = radius as i64;
    let mut out = Vec::new();
    for dz in -r..=r {
        for dx in -r..=r {
            let (x, z) = (centre.0 as i64 + dx, centre.1 as i64 + dz);
            if (0..TILES_PER_SIDE as i64).contains(&x) && (0..TILES_PER_SIDE as i64).contains(&z) {
                out.push((x as u32, z as u32));
            }
        }
    }
    out
}

impl Streamer {
    /// Read-only readiness of the area around `centre`: how many of the tiles within
    /// `radius` are loaded and how many of their models are still pending.
    fn area_progress(&self, centre: (u32, u32), radius: u32) -> Progress {
        let tiles = ring(centre, radius);
        let mut p = Progress {
            needed: tiles.len(),
            ..default()
        };
        for k in tiles {
            if let Some(t) = self.loaded.get(&k) {
                p.loaded += 1;
                p.pending += t.pending.len();
            }
        }
        p
    }
}

#[derive(Component)]
struct LoadRoot;
#[derive(Component)]
struct LoadDetail;

pub struct LoadScreenPlugin;

impl Plugin for LoadScreenPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LoadScreen>()
            .add_systems(Startup, spawn_screen)
            .add_systems(Update, (drive, show).chain());
    }
}

fn spawn_screen(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(Color::BLACK),
            // above the HUD, the map and the browser
            GlobalZIndex(1000),
            LoadRoot,
            Name::new("loading screen"),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("LOADING"),
                TextFont::from_font_size(34.0),
                TextColor(Color::srgb(0.92, 0.92, 0.92)),
            ));
            p.spawn((
                Text::new(""),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.55, 0.55, 0.55)),
                LoadDetail,
            ));
        });
}

type FocusQuery<'w, 's> = (
    Query<'w, 's, &'static Transform, With<Hero>>,
    Query<'w, 's, &'static Transform, With<Occupied>>,
    Query<'w, 's, &'static Transform, With<FlyCamera>>,
);

#[allow(clippy::too_many_arguments)]
fn drive(
    mut screen: ResMut<LoadScreen>,
    streamer: Option<Res<Streamer>>,
    cfg: Option<Res<WorldConfig>>,
    hero_loading: Option<Res<HeroLoading>>,
    real: Res<Time<Real>>,
    mut virt: ResMut<Time<Virtual>>,
    map: Option<Res<crate::map::MapState>>,
    focus: FocusQuery,
) {
    let now = real.elapsed_secs();
    let focus_pos = focus
        .0
        .single()
        .ok()
        .or_else(|| focus.1.single().ok())
        .or_else(|| focus.2.single().ok())
        .map(|t| t.translation);
    if !screen.active {
        // a far jump of the focus (respawn, mission teleport) brings the screen back
        if let (Some(p), Some(last)) = (focus_pos, screen.last_focus)
            && p.distance(last) > JUMP_DISTANCE
        {
            info!("loadscreen: focus jumped {:.0} m, holding for the area", p.distance(last));
            screen.request(LoadReason::Teleport);
        } else {
            screen.last_focus = focus_pos;
            return;
        }
    }
    let started = *screen.started.get_or_insert(now);
    if !virt.is_paused() {
        virt.pause();
        screen.paused = true;
    }
    let radius = cfg.map_or(READY_RADIUS, |c| READY_RADIUS.min(c.radius));
    let progress = match (&streamer, focus_pos) {
        (Some(st), Some(p)) => {
            let (x, z) = engine_xz(p);
            tile_index(x, z).map_or(Progress::default(), |t| st.area_progress(t, radius))
        }
        _ => Progress::default(),
    };
    screen.progress = progress;
    // Rico himself must exist when the game has one (he spawns once the start tile is in)
    let hero_pending = hero_loading.is_some_and(|h| !h.spawned);
    let ready = progress.ready() && !hero_pending;
    let timed_out = now - started > TIMEOUT_SECS;
    let done = screen.gate.step(ready);
    if done || timed_out {
        if timed_out && !done {
            warn!(
                "loadscreen: area still not ready after {TIMEOUT_SECS} s ({progress:?}), going on anyway"
            );
        }
        info!(
            "loadscreen: {:?} done after {} frames ({progress:?})",
            screen.reason, screen.gate.frames
        );
        screen.active = false;
        screen.last_focus = focus_pos;
        // the map's own arrival wait (fast travel) and an open map keep the clock themselves
        let map_holds = map.is_some_and(|m| m.open || m.arriving.is_some());
        if screen.paused && !map_holds {
            virt.unpause();
        }
        screen.paused = false;
    }
}

fn show(
    screen: Res<LoadScreen>,
    mut roots: Query<&mut Visibility, With<LoadRoot>>,
    mut detail: Query<&mut Text, With<LoadDetail>>,
) {
    for mut v in &mut roots {
        v.set_if_neq(if screen.active {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }
    if screen.active {
        let p = screen.progress;
        for mut t in &mut detail {
            **t = format!(
                "streaming the area: {}/{} tiles, {} objects waiting",
                p.loaded, p.needed, p.pending
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_is_clipped_to_the_world() {
        assert_eq!(ring((10, 10), 1).len(), 9);
        assert_eq!(ring((0, 0), 1).len(), 4);
        let last = TILES_PER_SIDE - 1;
        assert_eq!(ring((last, 5), 1).len(), 6);
        assert_eq!(ring((3, 3), 0), vec![(3, 3)]);
    }

    #[test]
    fn gate_needs_a_stable_run_and_a_minimum_time() {
        let mut g = Gate::default();
        // ready from the first frame: still waits for the minimum
        let mut at = None;
        for i in 1..=40 {
            if g.step(true) {
                at = Some(i);
                break;
            }
        }
        assert_eq!(at, Some(MIN_FRAMES.max(STABLE_FRAMES)));
        // a dip resets the stable run
        let mut g = Gate::default();
        for _ in 0..30 {
            g.step(true);
        }
        let mut g2 = Gate { frames: 30, stable: 0 };
        assert!(!g2.step(true));
        assert!(!g.step(false));
        for _ in 0..STABLE_FRAMES - 1 {
            assert!(!g.step(true));
        }
        assert!(g.step(true));
    }

    #[test]
    fn progress_ready_needs_every_tile_and_no_pending() {
        let p = Progress { needed: 9, loaded: 9, pending: 0 };
        assert!(p.ready());
        assert!(!Progress { loaded: 8, ..p }.ready());
        assert!(!Progress { pending: 1, ..p }.ready());
        assert!(!Progress::default().ready());
    }

    #[test]
    fn a_request_restarts_the_gate_only_when_idle() {
        let mut s = LoadScreen {
            active: false,
            ..default()
        };
        s.gate.frames = 50;
        s.request(LoadReason::FastTravel);
        assert!(s.active);
        assert_eq!(s.reason, LoadReason::FastTravel);
        assert_eq!(s.gate.frames, 0);
        s.gate.frames = 3;
        s.request(LoadReason::Teleport);
        assert_eq!(s.reason, LoadReason::FastTravel);
        assert_eq!(s.gate.frames, 3);
    }
}
