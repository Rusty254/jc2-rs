//! Parachute capture harness (JC2-950d; `research/formats/parachute-parity.md` section 5).
//!
//! `jc2 <game> --chute-scenario <name> [--chute-out DIR]` puts Rico at a scenario's place and
//! altitude, fixes the time of day and clouds, drives a scripted input timeline (deploy, steer,
//! cut) and saves one screenshot per named instant to `DIR` (default
//! `<game>/jc2-rs/_extract/parachute`) as `<scenario>_<instant>.png`, next to a
//! `<scenario>.md` table (what to compare with which reference shots) and a
//! `<scenario>.json` receipt (the game time, Rico's state and the camera at each frame).
//! The game then quits. `scripts/chute-capture.ps1` runs all scenarios in sequence.
//!
//! How it stays repeatable:
//! * the timeline runs on fixed-step ticks (not wall-clock): the inputs are written into
//!   [`HeroInput`] on the fixed step and, for the animation, again after `hero_input` in `Update`;
//! * a frame is requested with the virtual clock paused until the file is saved, so the
//!   simulation does not move while the screenshot is taken (the frame is the one rendered at
//!   the tick that was due; the pause takes effect from the next frame, so a frame can drift by
//!   at most one rendered frame);
//! * Rico is held (position and velocity reset every fixed step) at the start altitude for
//!   [`HOLD_SECONDS`] of real time so the terrain and its models around that point have
//!   streamed in before the fall;
//! * time of day is fixed (clock stopped), NPCs, civilians and traffic are off, health is kept full.
//!
//! Nothing here changes how the parachute works: it writes the same [`HeroInput`] fields the
//! keyboard does (`chute_toggle`, `chute_pull`, `chute_turn`), moves Rico once at the start,
//! sets the orbit camera and, for the above-the-canopy view, places the camera.
//!
//! **Non-native**: the reference shots' positions, headings and camera poses are not recorded
//! (gap 1 of the audit). Every place below is a landmark-level guess made for repeatability and
//! the camera angles are chosen from the shots' look; none of it is original data. The weather
//! (snow, gusts) is whatever the zone gives; the original's RNG seeds are unknown, and the
//! runtime has no global RNG to fix.

use crate::character::{Hero, HeroInput, OrbitCamera, Settling};
use crate::combat::Health;
use crate::coords::facing_dir;
use crate::world::FlyCamera;
use avian3d::prelude::*;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use std::path::{Path, PathBuf};

/// Real seconds to wait after the hero is placed before the scenario starts (start-up loading
/// and shader compiles).
const WARM_UP_SECONDS: f32 = 12.0;
/// Real seconds Rico is held at the start altitude so the surroundings stream in.
pub const HOLD_SECONDS: f32 = 6.0;
/// Real seconds the whole scenario may take after the hold before it is given up.
const RUN_TIMEOUT_SECONDS: f32 = 240.0;
/// Real seconds to wait for one screenshot to be written.
const SHOT_TIMEOUT_SECONDS: f32 = 20.0;

/// The scenarios (audit table S1 to S6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// S1: jungle plain by day, jump, free fall, deploy.
    JungleDeployDay,
    /// S2: over the snow base at night, camera behind, steady glide at 60 m.
    BaseNightBehind,
    /// S3: hard bank left then right.
    BankHard,
    /// S4: glide onto flat ground at night and the collapse on touchdown.
    LandingCollapse,
    /// S5: camera above the canopy looking down through the lines.
    CanopyFromAbove,
    /// S6: free-fall poses and the cut-away.
    FreefallPoses,
}

pub const ALL: [Scenario; 6] = [
    Scenario::JungleDeployDay,
    Scenario::BaseNightBehind,
    Scenario::BankHard,
    Scenario::LandingCollapse,
    Scenario::CanopyFromAbove,
    Scenario::FreefallPoses,
];

/// Where and when a scenario happens.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub tile: (u32, u32),
    /// Engine `(x, z)` of the start point (inside `tile`).
    pub xz: [f32; 2],
    /// The clock, `HH:MM`.
    pub clock: &'static str,
    /// Cloud cover (0 clear) when the scenario fixes it.
    pub cloud_cover: Option<f32>,
    /// Where the place came from (all are non-native choices).
    pub note: &'static str,
}

/// An input event on the timeline (seconds since release).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum What {
    /// One press of the chute key (deploys in the air, cuts under the canopy).
    Toggle,
    /// Hold the turn input (`+1` is `A`, left) until changed.
    Turn(f32),
    /// Hold the pull input (`+1` is `W`, forward) until changed.
    Pull(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub at: f32,
    pub what: What,
}

/// When a frame is taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum At {
    /// Seconds since release.
    T(f32),
    /// Canopy open and at most this many metres above the ground.
    Hag(f32),
    /// This many seconds (estimated by the feet's height over the fall speed) from touchdown.
    BeforeTouchdown(f32),
    /// This many seconds after the first grounded step.
    AfterTouchdown(f32),
}

#[derive(Clone, Copy, Debug)]
pub struct Shot {
    /// The instant's name (file name part).
    pub name: &'static str,
    pub at: At,
}

/// Where the camera is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cam {
    /// The orbit camera held at `yaw` degrees round Rico (0 behind) and `pitch` degrees.
    Behind { yaw: f32, pitch: f32 },
    /// Explicit pose: `up` metres above Rico and `back` metres behind him, looking at him.
    Above { up: f32, back: f32 },
}

impl Scenario {
    pub fn parse(name: &str) -> Option<Self> {
        ALL.into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(name))
    }

    /// The command-line name and the file prefix.
    pub fn name(self) -> &'static str {
        match self {
            Scenario::JungleDeployDay => "jungle-deploy-day",
            Scenario::BaseNightBehind => "base-night-behind",
            Scenario::BankHard => "bank-hard",
            Scenario::LandingCollapse => "landing-collapse",
            Scenario::CanopyFromAbove => "canopy-from-above",
            Scenario::FreefallPoses => "freefall-poses",
        }
    }

    /// The audit's scenario id.
    pub fn id(self) -> &'static str {
        match self {
            Scenario::JungleDeployDay => "S1",
            Scenario::BaseNightBehind => "S2",
            Scenario::BankHard => "S3",
            Scenario::LandingCollapse => "S4",
            Scenario::CanopyFromAbove => "S5",
            Scenario::FreefallPoses => "S6",
        }
    }

    pub fn site(self) -> Site {
        // The jungle: README's `--world 40,40` (tile origin x 4096, z 4096, 512 m wide); the
        // road beside the shots is not located (gap), so the tile's middle is used.
        const JUNGLE: Site = Site {
            tile: (40, 40),
            xz: [4352.0, 4352.0],
            clock: "12:00",
            cloud_cover: Some(0.0),
            note: "jungle tile 40,40, tile middle (the road of the reference shots is not located)",
        };
        // The start base Kem Gunung Dataran Tinggi (`keymissions/km01/km01.base`, tile 29,18);
        // the point is the one the location tests use inside the near box.
        const BASE_NIGHT: Site = Site {
            tile: (29, 18),
            xz: [-1245.0, -7050.0],
            clock: "23:00",
            cloud_cover: None,
            note: "start base tile 29,18 at (-1245, -7050), a point inside the base's near box",
        };
        match self {
            Scenario::JungleDeployDay | Scenario::CanopyFromAbove => JUNGLE,
            Scenario::BaseNightBehind | Scenario::LandingCollapse => BASE_NIGHT,
            Scenario::BankHard => Site {
                clock: "12:00",
                note: "start base tile 29,18 at (-1245, -7050) by day (snow zone)",
                ..BASE_NIGHT
            },
            Scenario::FreefallPoses => Site {
                note: "jungle tile 40,40, tile middle; high start for a long fall",
                ..JUNGLE
            },
        }
    }

    /// Metres above the ground Rico is released from.
    pub fn altitude(self) -> f32 {
        match self {
            Scenario::JungleDeployDay | Scenario::CanopyFromAbove => 200.0,
            Scenario::BaseNightBehind => 160.0,
            Scenario::BankHard => 300.0,
            Scenario::LandingCollapse => 70.0,
            Scenario::FreefallPoses => 1000.0,
        }
    }

    /// Heading of the model at release (radians; 0 faces +z). Non-native.
    pub fn facing(self) -> f32 {
        0.0
    }

    pub fn events(self) -> Vec<Event> {
        use What::*;
        let e = |at, what| Event { at, what };
        match self {
            Scenario::JungleDeployDay => vec![e(4.0, Toggle)],
            Scenario::BaseNightBehind => vec![e(1.0, Toggle)],
            Scenario::BankHard => vec![
                e(1.0, Toggle),
                e(6.0, Turn(1.0)),
                e(8.0, Turn(-1.0)),
                e(10.0, Turn(0.0)),
            ],
            Scenario::LandingCollapse => vec![e(0.5, Toggle)],
            Scenario::CanopyFromAbove => vec![e(1.0, Toggle)],
            Scenario::FreefallPoses => vec![
                e(3.5, Turn(1.0)),
                e(5.5, Turn(0.0)),
                e(6.0, Pull(-1.0)),
                e(8.0, Pull(0.0)),
                e(9.0, Toggle),
                e(12.0, Toggle),
            ],
        }
    }

    pub fn shots(self) -> Vec<Shot> {
        let s = |name, at| Shot { name, at };
        match self {
            Scenario::JungleDeployDay => vec![
                s("fall", At::T(3.9)),
                s("deploy-p0.0", At::T(4.0)),
                s("deploy-p0.3", At::T(4.3)),
                s("deploy-p1.0", At::T(5.0)),
                s("steady", At::T(10.0)),
            ],
            Scenario::BaseNightBehind => vec![s("steady-60m", At::Hag(60.0))],
            Scenario::BankHard => vec![
                s("steady", At::T(5.9)),
                s("left-peak", At::T(7.9)),
                s("right-peak", At::T(9.9)),
                s("recover", At::T(12.5)),
            ],
            Scenario::LandingCollapse => vec![
                s("before-0.5", At::BeforeTouchdown(0.5)),
                s("touchdown", At::AfterTouchdown(0.0)),
                s("after-0.5", At::AfterTouchdown(0.5)),
                s("after-2.0", At::AfterTouchdown(2.0)),
            ],
            Scenario::CanopyFromAbove => vec![s("steady", At::T(8.0))],
            Scenario::FreefallPoses => vec![
                s("belly", At::T(3.4)),
                s("bank-left", At::T(5.0)),
                s("pull-back", At::T(7.5)),
                s("deploy-p0.5", At::T(9.5)),
                s("canopy", At::T(11.9)),
                s("cut-p0.0", At::T(12.0)),
                s("cut-p1.0", At::T(13.0)),
            ],
        }
    }

    pub fn camera(self) -> Cam {
        match self {
            // the default chase camera
            Scenario::JungleDeployDay
            | Scenario::BaseNightBehind
            | Scenario::LandingCollapse
            | Scenario::FreefallPoses => Cam::Behind {
                yaw: 0.0,
                pitch: -14.0,
            },
            // from his side, a little above, so the roll shows
            Scenario::BankHard => Cam::Behind {
                yaw: 70.0,
                pitch: -10.0,
            },
            Scenario::CanopyFromAbove => Cam::Above { up: 9.0, back: 3.0 },
        }
    }

    /// The reference shots (file-name stems under `reference/chute`) and what to compare.
    pub fn compare(self) -> (&'static str, &'static str) {
        match self {
            Scenario::JungleDeployDay => (
                "canopy-deploying-over-jungle-plain-daylight_ck47, canopy-open-jungle-plain-road-daylight_ck49, deploy-unfolding_a06, pilot-chute-*",
                "bag/crumple phase, inflation time, canopy size and colour, lines dark, prompt text",
            ),
            Scenario::BaseNightBehind => (
                "canopy-behind-lines-base-rooftops-red-antenna-night_z29, canopy-behind-over-base-courtyard-building-17_z50, canopy-behind-rico-approaching-base-star-emblem-tanks_z09",
                "silhouette against sky, cell shading, line visibility, ambient only lighting",
            ),
            Scenario::BankHard => (
                "canopy-hard-bank-sideways-stripes-snow-gusts_v28, canopy-bank-over-*, canopy-side-*",
                "canopy roll angle (about 0.65 rad at full input), Rico hangs, yaw rate 0.79 rad/s",
            ),
            Scenario::LandingCollapse => (
                "canopy-collapsing-low-snow-landing-zone-night_f04, canopy-closing_b37, canopy-landing-*, canopy-draped-over-cruiser-landing_bx11",
                "landing clip by speed class, collapse look, canopy removal, no pop (the vehicle-roof variant is not scripted)",
            ),
            Scenario::CanopyFromAbove => (
                "canopy-above-rico-below_a10, canopy-above-lines-*, top-down-through-lines-*, lines-fan-from-above-*",
                "line fan geometry, canopy edge, camera elevation and FOV",
            ),
            Scenario::FreefallPoses => (
                "freefall-*",
                "body pose, camera 2, wind effects, the cut-away",
            ),
        }
    }
}

/// Everything the harness needs, from the command line.
#[derive(Resource, Clone, Debug)]
pub struct ChuteCapture {
    pub scenario: Scenario,
    pub out: PathBuf,
}

/// The default output folder: `<game dir>/jc2-rs/_extract/parachute`.
pub fn default_out(game_dir: &Path) -> PathBuf {
    game_dir.join("jc2-rs").join("_extract").join("parachute")
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Waiting for Rico to be placed and the warm-up to pass.
    Wait,
    /// At altitude, held, until this real time.
    Hold {
        until: f32,
    },
    /// Falling: the timeline runs.
    Run {
        started: f32,
    },
    Done,
}

#[derive(Resource)]
struct Run {
    cfg: ChuteCapture,
    phase: Phase,
    events: Vec<Event>,
    shots: Vec<Shot>,
    /// The start position (kept while held).
    hold_at: Vec3,
    /// Fixed steps since release.
    ticks: u32,
    next_event: usize,
    turn: f32,
    pull: f32,
    /// The chute key goes down on the next fixed step.
    toggle: bool,
    landed_at: Option<u32>,
    /// The tick of the last progress line.
    logged: u32,
    next_shot: usize,
    /// A screenshot in flight: its name, and when it was requested (real time).
    pending: Option<(&'static str, f32)>,
    saved: bool,
    receipts: Vec<serde_json::Value>,
}

impl Run {
    fn seconds(&self, step: f32) -> f32 {
        self.ticks as f32 * step
    }
}

pub struct ChuteCapturePlugin(pub ChuteCapture);

impl Plugin for ChuteCapturePlugin {
    fn build(&self, app: &mut App) {
        let s = self.0.scenario;
        app.insert_resource(Run {
            cfg: self.0.clone(),
            phase: Phase::Wait,
            events: s.events(),
            shots: s.shots(),
            hold_at: Vec3::ZERO,
            ticks: 0,
            next_event: 0,
            turn: 0.0,
            pull: 0.0,
            toggle: false,
            landed_at: None,
            logged: 0,
            next_shot: 0,
            pending: None,
            saved: false,
            receipts: Vec::new(),
        })
        .add_systems(
            FixedUpdate,
            (
                advance.before(crate::character::hero_physics),
                hold.after(crate::character::hero_physics),
            ),
        )
        .add_systems(
            Update,
            (
                sequence
                    .after(crate::character::hero_input)
                    .before(crate::character::hero_animation),
                above_camera.after(crate::character::hero_camera),
            ),
        );
    }
}

/// Fixed step, before the hero's: count the timeline and write the inputs.
fn advance(
    mut run: ResMut<Run>,
    fixed: Res<Time<Fixed>>,
    mut input: ResMut<HeroInput>,
    heroes: Query<&Hero>,
) {
    if !matches!(run.phase, Phase::Run { .. }) {
        return;
    }
    run.ticks += 1;
    let t = run.seconds(fixed.timestep().as_secs_f32());
    while let Some(ev) = run.events.get(run.next_event).copied() {
        if ev.at > t {
            break;
        }
        run.next_event += 1;
        match ev.what {
            What::Toggle => run.toggle = true,
            What::Turn(v) => run.turn = v,
            What::Pull(v) => run.pull = v,
        }
    }
    if run.toggle {
        run.toggle = false;
        input.chute_toggle = true;
    }
    input.chute_pull = run.pull;
    input.chute_turn = run.turn;
    if run.landed_at.is_none() && run.ticks > 30 && heroes.single().is_ok_and(|h| h.grounded) {
        run.landed_at = Some(run.ticks);
    }
}

/// Fixed step, after the hero's: hold Rico still at the start altitude while the scenery loads.
fn hold(
    run: Res<Run>,
    mut heroes: Query<(&mut Transform, &mut Position, &mut LinearVelocity), With<Hero>>,
) {
    if !matches!(run.phase, Phase::Hold { .. }) {
        return;
    }
    if let Ok((mut tf, mut pos, mut lin)) = heroes.single_mut() {
        tf.translation = run.hold_at;
        pos.0 = run.hold_at;
        lin.0 = Vec3::ZERO;
    }
}

type HeroParts<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Hero,
        &'static mut Transform,
        &'static mut LinearVelocity,
        Option<&'static mut Health>,
    ),
    Without<Settling>,
>;

/// Update: the phases, the input for the animation, the camera, the screenshots.
#[allow(clippy::too_many_arguments)]
fn sequence(
    mut run: ResMut<Run>,
    real: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    mut virt: ResMut<Time<Virtual>>,
    mut input: ResMut<HeroInput>,
    mut orbit: ResMut<OrbitCamera>,
    mut heroes: HeroParts,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    spatial: SpatialQuery,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let now = real.elapsed_secs();
    let Ok((entity, mut hero, mut tf, mut lin, health)) = heroes.single_mut() else {
        return; // not placed yet (still settling) or no hero
    };
    if let Some(mut h) = health {
        h.hp = h.max;
    }
    let scenario = run.cfg.scenario;
    match run.phase {
        Phase::Wait => {
            if now < WARM_UP_SECONDS {
                return;
            }
            let at = tf.translation + Vec3::Y * scenario.altitude();
            info!(
                "chute-capture {}: released from {:.0} m above {:?} ({})",
                scenario.name(),
                scenario.altitude(),
                tf.translation,
                scenario.site().note
            );
            // (the spatial query reads Position, so the move goes through commands)
            commands
                .entity(entity)
                .insert((Transform::from_translation(at), Position(at)));
            tf.translation = at;
            lin.0 = Vec3::ZERO;
            hero.facing = scenario.facing();
            hero.grounded = false;
            run.hold_at = at;
            run.phase = Phase::Hold {
                until: now + HOLD_SECONDS,
            };
        }
        Phase::Hold { until } => {
            hero.facing = scenario.facing();
            if now >= until {
                run.phase = Phase::Run { started: now };
                run.ticks = 0;
            }
        }
        Phase::Run { started } => {
            // the same inputs the fixed step wrote, for the animation's reading of them
            input.chute_pull = run.pull;
            input.chute_turn = run.turn;
            match scenario.camera() {
                Cam::Behind { yaw, pitch } => {
                    orbit.yaw = hero.facing + yaw.to_radians();
                    orbit.pitch = pitch.to_radians();
                }
                Cam::Above { .. } => {
                    orbit.yaw = hero.facing;
                    orbit.pitch = (-14.0f32).to_radians();
                }
            }
            if now - started > RUN_TIMEOUT_SECONDS {
                error!("chute-capture: timed out before every frame was taken");
                write_report(&run, false);
                exit.write(AppExit::error());
                run.phase = Phase::Done;
                return;
            }
            // a frame in flight
            if let Some((name, since)) = run.pending {
                if run.saved {
                    info!("chute-capture: saved {name}");
                    run.pending = None;
                    run.saved = false;
                    run.next_shot += 1;
                    virt.unpause();
                } else if now - since > SHOT_TIMEOUT_SECONDS {
                    error!("chute-capture: the frame {name} never arrived");
                    write_report(&run, false);
                    exit.write(AppExit::error());
                    run.phase = Phase::Done;
                }
                return;
            }
            let Some(shot) = run.shots.get(run.next_shot).copied() else {
                write_report(&run, true);
                exit.write(AppExit::Success);
                run.phase = Phase::Done;
                return;
            };
            let step = fixed.timestep().as_secs_f32();
            let t = run.seconds(step);
            let origin = tf.translation;
            let hag = spatial
                .cast_ray(
                    origin,
                    Dir3::NEG_Y,
                    5000.0,
                    true,
                    &SpatialQueryFilter::from_excluded_entities([entity]),
                )
                .map(|h| h.distance);
            let fall = -lin.0.y;
            if run.ticks.is_multiple_of(320) && run.ticks != run.logged {
                run.logged = run.ticks;
                info!(
                    "chute-capture: {t:.1} s at {origin:?}, {hag:?} m up, fall {fall:.1} m/s, chute {}, grounded {}",
                    hero.parachute, hero.grounded
                );
            }
            let due = match shot.at {
                At::T(at) => t >= at,
                At::Hag(h) => hero.parachute && hag.is_some_and(|d| d <= h),
                At::BeforeTouchdown(s) => {
                    // the feet's height over the fall speed (the ray starts at his middle); a
                    // landing that slipped past the estimate still takes the frame
                    run.landed_at.is_some()
                        || (!hero.grounded
                            && fall > 0.1
                            && hag
                                .is_some_and(|d| (d - crate::character::HEIGHT / 2.0) / fall <= s))
                }
                At::AfterTouchdown(s) => run
                    .landed_at
                    .is_some_and(|l| (run.ticks - l) as f32 * step >= s),
            };
            if !due {
                return;
            }
            let path = run
                .cfg
                .out
                .join(format!("{}_{}.png", scenario.name(), shot.name));
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let ticks = run.ticks;
            let cam = cams.single().map(|c| c.translation).unwrap_or(Vec3::ZERO);
            run.receipts.push(serde_json::json!({
                "instant": shot.name,
                "file": path.file_name().map(|n| n.to_string_lossy().into_owned()),
                "fixed_ticks": ticks,
                "seconds": t,
                "hero": [origin.x, origin.y, origin.z],
                "velocity": [lin.0.x, lin.0.y, lin.0.z],
                "height_above_ground": hag,
                "facing": hero.facing,
                "parachute": hero.parachute,
                "grounded": hero.grounded,
                "camera": [cam.x, cam.y, cam.z],
            }));
            info!("chute-capture: {} at {t:.2} s", path.display());
            virt.pause();
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path))
                .observe(|_: On<ScreenshotCaptured>, mut run: ResMut<Run>| {
                    run.saved = true;
                });
            run.pending = Some((shot.name, now));
        }
        Phase::Done => {}
    }
}

/// S5: the camera above the canopy, after the hero camera has placed its own.
fn above_camera(
    run: Res<Run>,
    heroes: Query<(&Hero, &Transform)>,
    mut cams: Query<&mut Transform, (With<FlyCamera>, Without<Hero>)>,
) {
    let Cam::Above { up, back } = run.cfg.scenario.camera() else {
        return;
    };
    if !matches!(run.phase, Phase::Run { .. }) {
        return;
    }
    let (Ok((hero, hero_tf)), Ok(mut cam)) = (heroes.single(), cams.single_mut()) else {
        return;
    };
    let target = hero_tf.translation + Vec3::Y * 0.6;
    cam.translation = target + Vec3::Y * up - facing_dir(hero.facing) * back;
    cam.look_at(target, Vec3::Y);
}

/// The comparison table and the receipt, next to the frames.
fn write_report(run: &Run, complete: bool) {
    let s = run.cfg.scenario;
    let (refs, what) = s.compare();
    let site = s.site();
    let mut md = format!(
        "# {} {}\n\nPlace: {} (non-native choice). Clock {}, cloud cover {:?}, released {:.0} m above the ground, camera {:?}.\n\
         Complete: {complete}. Reference shots: `reference/chute/` {refs}.\nCompare: {what}.\n\n\
         | frame | game time (s) | compare with |\n|---|---|---|\n",
        s.id(),
        s.name(),
        site.note,
        site.clock,
        site.cloud_cover,
        s.altitude(),
        s.camera(),
    );
    for r in &run.receipts {
        md.push_str(&format!(
            "| {} | {:.2} | {} |\n",
            r["file"].as_str().unwrap_or("?"),
            r["seconds"].as_f64().unwrap_or(0.0),
            refs
        ));
    }
    let _ = std::fs::create_dir_all(&run.cfg.out);
    let base = run.cfg.out.join(s.name());
    if let Err(e) = std::fs::write(base.with_extension("md"), md) {
        error!("chute-capture: cannot write the table: {e}");
    }
    let receipt = serde_json::json!({
        "scenario": s.name(),
        "id": s.id(),
        "complete": complete,
        "tile": [site.tile.0, site.tile.1],
        "start_xz": site.xz,
        "clock": site.clock,
        "altitude": s.altitude(),
        "events": run.events.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>(),
        "frames": run.receipts,
    });
    if let Err(e) = std::fs::write(
        base.with_extension("json"),
        serde_json::to_string_pretty(&receipt).unwrap_or_default(),
    ) {
        error!("chute-capture: cannot write the receipt: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for s in ALL {
            assert_eq!(Scenario::parse(s.name()), Some(s));
        }
        assert_eq!(Scenario::parse("nope"), None);
    }

    #[test]
    fn sites_are_inside_their_tiles() {
        for s in ALL {
            let site = s.site();
            let (x0, z0) = jc2_formats::terrain::tile_origin(site.tile.0, site.tile.1);
            let size = jc2_formats::terrain::TILE_SIZE;
            assert!(site.xz[0] >= x0 && site.xz[0] < x0 + size, "{s:?} x");
            assert!(site.xz[1] >= z0 && site.xz[1] < z0 + size, "{s:?} z");
            assert!(jc2_formats::weather::parse_clock(site.clock).is_some());
        }
    }

    #[test]
    fn timelines_are_ordered_and_frames_unique() {
        for s in ALL {
            let ev = s.events();
            assert!(ev.windows(2).all(|w| w[0].at <= w[1].at), "{s:?} events");
            let shots = s.shots();
            let mut names: Vec<_> = shots.iter().map(|x| x.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), shots.len(), "{s:?} frame names");
            // timed frames come in time order (the sequence takes them one by one)
            let times: Vec<f32> = shots
                .iter()
                .filter_map(|x| match x.at {
                    At::T(t) => Some(t),
                    _ => None,
                })
                .collect();
            assert!(times.windows(2).all(|w| w[0] <= w[1]), "{s:?} frames");
        }
    }

    #[test]
    fn output_folder_is_under_the_extract_folder() {
        let p = default_out(Path::new("game"));
        assert!(
            p.ends_with("jc2-rs/_extract/parachute") || p.ends_with("jc2-rs\\_extract\\parachute")
        );
    }
}
