//! Milestone 5: Rico. A third-person character on the streamed world — walking,
//! sprinting and jumping on a capsule body, the grappling hook (`settings\grapplingparams.bin`
//! drives the pull), the parachute, and getting in and out of the car.
//!
//! Hero controls: right-drag (or `Tab` to grab the cursor) to look, `WASD` move, `Shift`
//! sprint, `Space` jump — in the air it opens and closes the parachute — `F` fires the
//! grappling hook at whatever the camera looks at (and releases it), `E` gets in or out
//! of a nearby vehicle, `V` switches to the free camera and back.
//!
//! The model is the game's `mc01_lod1-rico.rbm`, skinned to its `biped.bsk` skeleton and
//! driven by the game's `.ban` clips (`rig.rs`). The canopy is a stand-in mesh: the
//! game's parachute is Havok cloth, not an RBM.

use avian3d::prelude::*;
use bevy::asset::LoadState;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::Game;
use crate::controls::{
    Controls, CROUCH as CROUCH_ACTION, EVADE as EVADE_ACTION, MOVE_BACK, MOVE_FORWARD, MOVE_LEFT, MOVE_RIGHT, SPRINT as SPRINT_ACTION,
};
use crate::coords::{engine_xz, facing_dir, yaw_of, yaw_toward};
use crate::grapple::{GrappleRig, LockOn, Phase as GrapplePhase, Tuning};
use crate::parachute::{ChuteRig, spawn_canopy};
use crate::poses::{Arms, Gait, drive_upper_body};
use crate::rbm_loader::RbmAsset;
use crate::rig::{Animator, Clip, Rig, skin, spawn_joints};
use crate::vehicle::{CameraMode, Enterable, HeroEnabled, Occupied};
use crate::world::{FlyCamera, SpawnPoint};
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use jc2_formats::lod::LodDescriptor;
use jc2_formats::player::{GrapplingParams, PlayerSettings};
use jc2_formats::sarc::SmallArchive;

/// Capsule body: 1.8 m tall, 0.3 m radius; the model's feet are at the capsule's bottom.
pub const HEIGHT: f32 = 1.8;
pub const RADIUS: f32 = 0.3;
const MASS: f32 = 80.0;
const GROUND_ACCEL: f32 = 40.0;
// Native locomotion values (JC2-241, `research/formats/rico-locomotion-motion.md`). The
// ground speed is the root motion of the playing clips times the state's time scale:
/// Root-motion speeds (m/s) of `wk_fwd`, `run_fwd`, `dash_fwd` (`jc2tool anim`).
const WALK_CLIP: f32 = 1.3325;
const RUN_CLIP: f32 = 3.3333;
const DASH_CLIP: f32 = 4.1667;
/// The stick's first-order filter gain per native update (`0.007c6810`), and the update
/// rate it is applied at (the clips' 30 Hz; the native tick is Gap 3 of that document).
const STICK_GAIN: f32 = 0.1;
const STICK_HZ: f32 = 30.0;
/// `S_DASH`: the counter runs 1 -> 0 over 1.5 s, the time scale 1.5625 -> 2.0.
const DASH_SECONDS: f32 = 1.5;
/// Turn: the facing turns at `2 x rate` rad/s (`006831f0`); rate 1.5 pi on the ground, pi/2
/// sprinting and in the air (aiming turns instantly here: Gap 4).
const TURN_GROUND: f32 = 3.0 * std::f32::consts::PI;
const TURN_SLOW: f32 = std::f32::consts::PI;
/// Gravity of the character instance (`settings/player.bin` `0xc481b248`); the engine's own
/// 9.81 acts too, the difference is added while he flies freely.
const GRAVITY: f32 = 21.0;
/// The running jump's climb: the start clip's vertical root motion, 4.27 m/s from 0.1 s for
/// 0.367 s (`run_jump_start`). The player's jump is always that one (`ACT_JUMP_AT`). That
/// the character keeps the vertical speed afterwards is inferred (Gap 1 of that document).
const JUMP_DELAY: f32 = 0.1;
const JUMP_CLIMB_SPEED: f32 = 4.2773;
const JUMP_CLIMB_SECONDS: f32 = 0.367;
/// Air steering: a lift velocity of 5.0 m/s times the stick (`007f5810`, player.bin
/// `0x77fd88be`), approached with the in-air context's gain 0.05 per update (inferred
/// hand-over, Gap 1; the sprint-jump 7.0 m/s table is not modelled).
const AIR_STEER: f32 = 5.0;
const AIR_GAIN: f32 = 0.05;
/// How long a jump press waits for the animation state to take it (native, `0x01077710`).
const JUMP_BUFFER: f32 = 0.3;
/// Steeper than this is a wall, not ground.
const MAX_SLOPE: f32 = 0.87;
/// How far below the capsule the ground probe reaches.
const GROUND_PROBE: f32 = 0.25;
const CANOPY_HEIGHT: f32 = 5.0;
pub const ENTER_RANGE: f32 = 4.5;
const CAMERA_DISTANCE: f32 = 4.2;
/// Swimming: how deep the capsule's centre sits below the surface, and the speed.
const SWIM_DEPTH: f32 = 0.35;
const SWIM_SPEED: f32 = 2.2;
const SWIM_ACCEL: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grapple {
    pub anchor: Vec3,
    pub normal: Vec3,
    pub age: f32,
}

/// The player character.
#[derive(Component, Debug, Clone)]
pub struct Hero {
    /// Down at zero health, until combat restores him at the spawn point.
    pub downed: bool,
    pub grounded: bool,
    pub ground_normal: Vec3,
    /// Yaw the model faces (radians about +y; the model looks down +z at 0).
    pub facing: f32,
    /// Horizontal speed, for the HUD.
    pub speed: f32,
    pub parachute: bool,
    pub grapple: Option<Grapple>,
    /// Reeled in to a wall, ledge or overhang and hanging there from the hook
    /// (`grapple.rs`; `Space` jumps or climbs, `Ctrl` drops). `grapple` is still set.
    pub hanging: bool,
    pub in_vehicle: Option<Entity>,
    /// Standing on this vehicle's roof (`stunts.rs` pins him there; his own controller
    /// is skipped meanwhile).
    pub stunt: Option<Entity>,
    /// While on a vehicle: the clip `stunts.rs` wants instead of standing (hanging under
    /// a helicopter or on the wire below it).
    pub stunt_clip: Option<crate::rig::Clip>,
    pub air_time: f32,
    /// In the sea, treading water (the ground is too deep to wade).
    pub swimming: bool,
    /// Aiming along this yaw (`coords::yaw_toward`): standing, he turns to it; moving,
    /// his legs run or strafe and the spine turns the rest (`hero_animation`).
    pub aim_yaw: Option<f32>,
    /// The foot machine is in a landing whose clip has no extracted motion and no
    /// navigate/jump edge (`S_JUMP_LAND`, `S_JUMP_LAND_HEAVY`, `S_LANDING_HARD`): walking and
    /// jumping wait for it (inferred from the graph; `foot_machine.rs`).
    pub move_lock: bool,
    /// The foot machine's current state takes `ACT_JUMP_AT` / `ACT_PULLCHORD` (the native
    /// jump handler and the canopy's open driver both ask that before they act).
    pub jump_ok: bool,
    pub chute_ok: bool,
    /// Seconds a jump press stays valid (the native 0.3 s buffer, `0x01077710`).
    jump_buffer: f32,
    jump_cooldown: f32,
    /// The low-passed stick (planar), the sprint counter, the take-off delay and the climb
    /// left (see the constants), and the clip root-motion speed of the air state he is in.
    stick: Vec3,
    dash_counter: f32,
    was_sprinting: bool,
    jump_delay: f32,
    climb_left: f32,
    /// In free air since the ground (a jump, a ledge): the native air model applies. A flight
    /// that the hook has touched keeps the older rules (its own jumps and releases).
    free_air: bool,
    /// Forward root motion (m/s) of the foot machine's air state (`S_JUMP_TO_FALL_TRANSITION`).
    pub air_root: f32,
    /// The canopy's flight data block (`D+0x30..`, `parachute_flight.rs`), live while the
    /// chute is open.
    pub canopy: crate::parachute_flight::CanopyFlight,
    /// The foot machine is in `S_SKYDIVE`: the freefall model (`0x006b9290`) flies him.
    pub skydiving: bool,
    skydive: Option<crate::parachute_flight::Skydive>,
    /// The foot machine drives the states (set by its step); without it the chord press alone
    /// starts the canopy model (fallback, non-native).
    foot_driven: bool,
    /// The foot machine is in a `CParachuteController` state (`S_UNFOLD_PARACHUTE_*`,
    /// `S_PARACHUTE`): the canopy flight model flies him
    /// (`parachute-state-velocities.md` section 2).
    chute_state: bool,
    /// The canopy flight model has run its opening frame (`CanopyFlight::open` and the opening
    /// velocity transform happen on the first frame of the unfold state, not at the press).
    pub canopy_live: bool,
    /// The canopy's matrix at a cut-away, handed to the freefall model that starts next
    /// (the freefall start keeps the character's current matrix).
    cut_matrix: Option<crate::parachute_flight::Rows>,
    /// `S_JUMP_TO_SKYDIVE`: the progress of its clip (`CBaseJumpController` update `0x00756ff0`)
    /// and the speed at the controller start (`S+0x34`).
    base_jump: Option<f32>,
    base_v0: Option<f32>,
    /// The foot machine's ground state moves him by its clip's root motion (the evasive roll,
    /// the rise from a crouch) or holds him still (the crouch): `foot_actions.rs`.
    pub root_drive: Option<crate::foot_actions::RootDrive>,
}

impl Default for Hero {
    fn default() -> Self {
        Self {
            downed: false,
            grounded: false,
            ground_normal: Vec3::Y,
            facing: 0.0,
            speed: 0.0,
            parachute: false,
            grapple: None,
            hanging: false,
            in_vehicle: None,
            stunt: None,
            stunt_clip: None,
            air_time: 0.0,
            swimming: false,
            aim_yaw: None,
            move_lock: false,
            jump_ok: true,
            chute_ok: true,
            jump_buffer: 0.0,
            jump_cooldown: 0.0,
            stick: Vec3::ZERO,
            dash_counter: 1.0,
            was_sprinting: false,
            jump_delay: 0.0,
            climb_left: 0.0,
            free_air: true,
            air_root: 0.0,
            canopy: Default::default(),
            skydiving: false,
            skydive: None,
            foot_driven: false,
            chute_state: false,
            canopy_live: false,
            cut_matrix: None,
            base_jump: None,
            base_v0: None,
            root_drive: None,
        }
    }
}

/// The visual child that turns to face the way Rico walks.
#[derive(Component)]
#[require(ModelVisualState)]
pub(crate) struct HeroModel;
#[derive(Component, Default)]
pub(crate) struct ModelVisualState {
    wire_tilt: bool,
}

#[derive(Component)]
pub struct Canopy;
#[derive(Component)]
pub(crate) struct HookVisual;
#[derive(Component)]
pub(crate) struct WireVisual;
#[derive(Component)]
struct HeroHud;

/// What the keys asked for this frame; consumed by the fixed-step physics.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct HeroInput {
    /// World-space direction on the ground plane (camera-relative), unit or zero.
    pub movement: Vec3,
    pub sprint: bool,
    pub jump: bool,
    pub chute_toggle: bool,
    /// `Ctrl`: drop from a hang.
    pub drop: bool,
    pub chute_pull: f32,
    pub chute_turn: f32,
    /// The crouch key is down (hold mode, the native default; JC2-051).
    pub crouch: bool,
    /// The evade key is down (the foot driver reads the press and the tap-release from it:
    /// `foot_actions::EvadeKey`).
    pub evade: bool,
    /// FIRE_GRAPPLE (id 0x51) is pressed or held: the canopy's hook assist is off meanwhile.
    pub fire_grapple: bool,
}

/// The third-person camera around Rico.
#[derive(Resource, Debug)]
pub struct OrbitCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: -0.25,
            distance: CAMERA_DISTANCE,
        }
    }
}

#[derive(Resource)]
pub struct HeroParams {
    /// Model and skeleton names (the skeleton waits for the animation milestone).
    #[allow(dead_code)]
    pub settings: PlayerSettings,
    pub grapple: GrapplingParams,
}

/// Rico's main-layer animation graph and its animation set (`basestatemachine.afsm`,
/// `kc001_base.asb`), read from the game's data (`foot_machine.rs`).
#[derive(Resource)]
pub struct FootMachineData {
    pub machine: crate::foot_machine::Machine,
    pub set: crate::foot_machine::AnimSet,
}

#[derive(Resource)]
pub(crate) struct HeroLoading {
    model: Handle<RbmAsset>,
    hook: Option<Handle<RbmAsset>>,
    pub(crate) spawned: bool,
}

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::rig::RigPlugin>() {
            app.add_plugins(crate::rig::RigPlugin);
        }
        if !app.is_plugin_added::<crate::ground_ik::GroundIkPlugin>() {
            app.add_plugins(crate::ground_ik::GroundIkPlugin);
        }
        if !app.is_plugin_added::<crate::grapple::GrapplePlugin>() {
            app.add_plugins(crate::grapple::GrapplePlugin);
        }
        if !app.is_plugin_added::<crate::controls::ControlsPlugin>() {
            app.add_plugins(crate::controls::ControlsPlugin::default());
        }
        app.add_plugins(crate::ragdoll::RagdollPlugin);
        if !app.is_plugin_added::<crate::gameplay_camera::GameplayCameraPlugin>() {
            app.add_plugins(crate::gameplay_camera::GameplayCameraPlugin);
        }
        app.init_resource::<CameraMode>()
            .init_resource::<crate::parachute::CanopyPoseFeed>()
            .insert_resource(HeroEnabled)
            .init_resource::<HeroInput>()
            .init_resource::<OrbitCamera>()
            .add_systems(Startup, request_hero)
            .add_systems(
                Update,
                (
                    spawn_hero,
                    hero_input,
                    vehicle_entry,
                    grapple_fire,
                    hero_animation.after(crate::combat::hero_arms),
                    hero_visuals,
                    crate::parachute::attach_canopy_model,
                    crate::parachute::animate_canopy,
                    hero_camera,
                    hero_hud,
                )
                    .chain(),
            )
            .add_systems(FixedUpdate, (hero_physics, settle_spawn).chain());
    }
}

/// Read the player settings and ask for the models.
fn request_hero(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    mut mode: ResMut<CameraMode>,
) {
    let settings = game
        .fs
        .read_inflated("player.bin")
        .ok()
        .and_then(|d| PlayerSettings::parse(&d).ok())
        .unwrap_or_else(|| {
            warn!("settings\\player.bin not found; using the default model");
            PlayerSettings {
                model: "Characters\\MainCharacters\\Rico\\mc01_lod1-rico".into(),
                skeleton: String::new(),
            }
        });
    let grapple = game
        .fs
        .read_inflated("grapplingparams.bin")
        .ok()
        .and_then(|d| GrapplingParams::parse(&d).ok())
        .unwrap_or_default();
    let model_file = settings.model_file();
    info!(
        "hero: {model_file} on {}, grapple pull {} N, reel {} m/s, range {} m",
        settings.skeleton, grapple.pull_force_character, grapple.reel_in_speed, grapple.range
    );
    let model: Handle<RbmAsset> = assets.load(format!("jc2://{model_file}"));
    // the hook: its .lod lives in general.blz (the launcher is part of his model: the
    // gauntlet on the left forearm; `wea04-a.lod` is the revolver)
    let general = game
        .fs
        .read_inflated("general.blz")
        .ok()
        .and_then(|data| SmallArchive::parse(&data).ok().map(|s| (data, s)));
    let from_lod = |lod: &str| {
        let (data, sarc) = general.as_ref()?;
        let e = sarc.get(lod)?;
        let text = sarc.data(data, e).ok()?;
        let d = LodDescriptor::parse(&String::from_utf8_lossy(text));
        let rbm = d.model_for_level(1).or_else(|| d.model_for_level(0))?;
        Some(assets.load::<RbmAsset>(format!("jc2://general.blz/{rbm}")))
    };
    let hook = from_lod(&grapple.hook_lod());
    if hook.is_none() {
        warn!("grappling hook model not found; a sphere stands in");
    }
    // the skeleton and the clips
    let skeleton_file = settings
        .skeleton
        .rsplit(['\\', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("biped.bsk")
        .to_ascii_lowercase();
    match Rig::load(&game.fs, &skeleton_file) {
        Some(rig) => commands.insert_resource(rig),
        None => warn!("{skeleton_file}: no skeleton, Rico stays in his bind pose"),
    }
    match ChuteRig::load(&game.fs) {
        Some(rig) => commands.insert_resource(rig),
        None => warn!("parachute.bsk: no parachute skeleton, a dome stands in"),
    }
    match (
        crate::foot_machine::Machine::load(&game.fs, "basestatemachine.afsm"),
        crate::foot_machine::AnimSet::load(&game.fs, "kc001_base.asb"),
    ) {
        (Some(machine), Some(set)) => {
            info!(
                "foot machine: {} states, {} transitions",
                machine.states.len(),
                machine.transitions.len()
            );
            // the clip table against the animation set: a mismatch means a changed game data set
            for (state, clip) in NATIVE_STATE_CLIPS {
                let bound = set
                    .states
                    .get(*state)
                    .is_some_and(|a| a.bans.iter().any(|b| b == clip.file()));
                if !bound {
                    warn!("foot machine: {state} is not bound to {} in kc001_base.asb", clip.file());
                }
            }
            commands.insert_resource(FootMachineData { machine, set });
        }
        _ => warn!("basestatemachine.afsm / kc001_base.asb not found: Rico uses the fallback animation rules"),
    }
    *mode = CameraMode::Hero;
    commands.insert_resource(HeroParams { settings, grapple });
    commands.insert_resource(crate::grapple::HookModel(hook.clone()));
    commands.insert_resource(HeroLoading {
        model,
        hook,
        spawned: false,
    });
}

/// A dome 6.5 m across for the stand-in canopy, with its apex `CANOPY_HEIGHT` above
/// the shoulders.
fn canopy_mesh() -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, PrimitiveTopology};
    let (n, m) = (16usize, 8usize);
    let (w, d) = (6.5f32, 3.4f32);
    let mut pos = Vec::new();
    let mut nor = Vec::new();
    let mut uv = Vec::new();
    for j in 0..=m {
        for i in 0..=n {
            let u = i as f32 / n as f32 * 2.0 - 1.0;
            let v = j as f32 / m as f32 * 2.0 - 1.0;
            let y = CANOPY_HEIGHT - 1.1 * (u * u + 0.6 * v * v);
            pos.push([u * w / 2.0, y, v * d / 2.0]);
            nor.push(
                Vec3::new(2.2 * u / (w / 2.0), 1.0, 1.32 * v / (d / 2.0))
                    .normalize()
                    .to_array(),
            );
            uv.push([i as f32 / n as f32, j as f32 / m as f32]);
        }
    }
    let mut idx = Vec::new();
    for j in 0..m {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let b = a + 1;
            let c = a + (n + 1) as u32;
            let dd = c + 1;
            idx.extend_from_slice(&[a, c, b, b, c, dd]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// A unit cylinder along +y stretched between two points.
pub fn segment_transform(a: Vec3, b: Vec3, width: f32) -> Transform {
    let d = b - a;
    let len = d.length().max(1e-4);
    Transform {
        translation: (a + b) / 2.0,
        rotation: Quat::from_rotation_arc(Vec3::Y, d / len),
        scale: Vec3::new(width, len, width),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_hero(
    mut commands: Commands,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    rig: Option<Res<Rig>>,
    chute: Option<Res<ChuteRig>>,
    spawn: Option<Res<SpawnPoint>>,
    loading: Option<ResMut<HeroLoading>>,
    mut orbit: ResMut<OrbitCamera>,
) {
    let Some(mut loading) = loading else { return };
    if loading.spawned {
        return;
    }
    let Some(spawn) = spawn else { return };
    let ready = |h: &Handle<RbmAsset>| {
        rbms.get(h).is_some() || matches!(assets.get_load_state(h.id()), Some(LoadState::Failed(_)))
    };
    if !ready(&loading.model) || loading.hook.as_ref().is_some_and(|h| !ready(h)) {
        return;
    }
    // beside the spawn point (a car may be parked on it)
    let start = spawn.0 + Vec3::new(3.0, HEIGHT / 2.0 + 0.3, 0.0);
    let hero = Hero::default();
    // the camera behind him looks the way he faces (the same yaw convention)
    orbit.yaw = hero.facing;
    let canopy_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.16, 0.18),
        perceptual_roughness: 0.9,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    let line_material = materials.add(StandardMaterial {
        // unlit: native scene unit from a display value (policy, `native_units.rs`)
        base_color: crate::native_units::native_colour(Vec3::new(0.85, 0.85, 0.8), 1.0),
        unlit: true,
        ..default()
    });
    let cylinder = meshes.add(Cylinder::new(1.0, 1.0));
    let canopy = meshes.add(canopy_mesh());
    let entity = commands.spawn((
        RigidBody::Dynamic,
        Collider::capsule(RADIUS, HEIGHT - 2.0 * RADIUS),
        LockedAxes::ROTATION_LOCKED,
        (Mass(MASS), NoAutoMass),
        Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
        Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
        SleepingDisabled,
        TransformInterpolation,
        Transform::from_translation(start),
        Visibility::default(),
        hero,
        Name::new("rico"),
    ));
    let hero_entity = entity.id();
    commands.entity(hero_entity).insert(Settling::default());
    // the model, with its skeleton when there is one
    let model_entity = commands
        .spawn((
            Transform::from_xyz(0.0, -HEIGHT / 2.0, 0.0),
            Visibility::default(),
            HeroModel,
            Name::new("rico model"),
        ))
        .id();
    commands.entity(hero_entity).add_child(model_entity);
    let rigged = rig.as_ref().map(|r| {
        let (joints, handle) =
            spawn_joints(&mut commands, model_entity, &r.skeleton, &mut bindposes);
        (joints, handle, r.skeleton.bones.len())
    });
    match rbms.get(&loading.model) {
        Some(rbm) => {
            for b in &rbm.blocks {
                let mut mesh = commands.spawn((
                    Mesh3d(b.mesh.clone()),
                    b.material_component(),
                    Transform::IDENTITY,
                ));
                if let Some((joints, handle, _)) = &rigged
                    && b.skinned
                {
                    mesh.insert(skin(joints, handle));
                }
                let id = mesh.id();
                commands.entity(model_entity).add_child(id);
            }
        }
        None => {
            let id = commands
                .spawn((
                    Mesh3d(meshes.add(Capsule3d::new(RADIUS, HEIGHT - 2.0 * RADIUS))),
                    MeshMaterial3d(materials.add(Color::srgb(0.8, 0.3, 0.2))),
                    Transform::from_xyz(0.0, HEIGHT / 2.0, 0.0),
                ))
                .id();
            commands.entity(model_entity).add_child(id);
        }
    }
    if let Some((joints, _, _)) = rigged {
        // the launcher is the gauntlet on his left forearm, part of his own mesh (the
        // `wea04-a.lod` once hung here is the revolver): an empty marker on the left
        // hand's attach bone says where the wire leaves it (`grapple::GrappleLauncher`)
        if let Some(wrist) = rig.as_ref().and_then(|r| {
            r.skeleton
                .bone_index("AttachHandLeft")
                .or_else(|| r.skeleton.bone_index("LeftHand"))
        }) {
            commands.spawn((
                Transform::IDENTITY,
                Visibility::default(),
                Name::new("grapple launcher"),
                ChildOf(joints[wrist]),
            ));
        }
        commands.entity(hero_entity).insert((
            Animator::new(rig.as_ref().unwrap().0.clone(), joints),
            Arms::default(),
            crate::foot_machine::FootDriver::default(),
        ));
    }
    match chute.as_ref() {
        Some(chute) => {
            spawn_canopy(
                &mut commands,
                hero_entity,
                chute,
                &mut meshes,
                &mut materials,
                Vec3::new(0.0, -HEIGHT / 2.0, 0.0),
            );
        }
        None => {
            let mut entity = commands.entity(hero_entity);
            entity.with_children(|parent| {
                // the parachute: canopy plus six lines from the shoulders
                parent
                    .spawn((
                        Transform::from_xyz(0.0, 0.5, 0.0),
                        Visibility::Hidden,
                        Canopy,
                        Name::new("parachute"),
                    ))
                    .with_children(|cp| {
                        cp.spawn((
                            Mesh3d(canopy),
                            MeshMaterial3d(canopy_material),
                            Transform::IDENTITY,
                        ));
                        for (sx, sz) in [
                            (-1.0, -1.0),
                            (-1.0, 0.0),
                            (-1.0, 1.0),
                            (1.0, -1.0),
                            (1.0, 0.0),
                            (1.0, 1.0),
                        ] {
                            let shoulder = Vec3::new(0.25 * sx, 0.0, 0.0);
                            let corner = Vec3::new(
                                3.0 * sx,
                                CANOPY_HEIGHT - 1.1 * (1.0 + 0.6 * sz * sz),
                                1.6 * sz,
                            );
                            cp.spawn((
                                Mesh3d(cylinder.clone()),
                                MeshMaterial3d(line_material.clone()),
                                segment_transform(shoulder, corner, 0.012),
                            ));
                        }
                    });
            });
        }
    }
    // the hook and its wire live in world space
    let mut hook = commands.spawn((
        Transform::IDENTITY,
        Visibility::Hidden,
        HookVisual,
        Name::new("grappling hook"),
    ));
    hook.with_children(|hp| match loading.hook.as_ref().and_then(|h| rbms.get(h)) {
        Some(rbm) => {
            for b in &rbm.blocks {
                hp.spawn((
                    Mesh3d(b.mesh.clone()),
                    b.material_component(),
                    Transform::IDENTITY,
                ));
            }
        }
        None => {
            hp.spawn((
                Mesh3d(meshes.add(Sphere::new(0.08))),
                MeshMaterial3d(materials.add(Color::srgb(0.3, 0.3, 0.3))),
                Transform::IDENTITY,
            ));
        }
    });
    commands.spawn((
        crate::grapple::Cable::default(),
        Visibility::Hidden,
        WireVisual,
        Name::new("grapple wire"),
    ));
    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(16.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            bottom: Val::Px(10.0),
            ..default()
        },
        HeroHud,
        crate::hud::DevText,
    ));
    info!("rico: spawned at {start:?}");
    loading.spawned = true;
}

type WindowCursor<'w, 's> = Query<'w, 's, &'static mut CursorOptions, With<PrimaryWindow>>;

/// Keys and mouse → `HeroInput`; also the camera-mode switch and the cursor grab.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hero_input(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut mode: ResMut<CameraMode>,
    mut input: ResMut<HeroInput>,
    mut orbit: ResMut<OrbitCamera>,
    controls: Res<Controls>,
    heroes: Query<&Hero>,
    mut cursors: WindowCursor,
) {
    let Ok(hero) = heroes.single() else { return };
    if keys.just_pressed(KeyCode::KeyV) {
        *mode = match *mode {
            CameraMode::Free if hero.in_vehicle.is_some() => CameraMode::Drive,
            CameraMode::Free => CameraMode::Hero,
            _ => CameraMode::Free,
        };
    }
    let mut grabbed = false;
    if let Ok(mut cursor) = cursors.single_mut() {
        if keys.just_pressed(KeyCode::Tab) || keys.just_pressed(KeyCode::Escape) {
            let grab =
                cursor.grab_mode == CursorGrabMode::None && !keys.just_pressed(KeyCode::Escape);
            cursor.grab_mode = if grab {
                CursorGrabMode::Locked
            } else {
                CursorGrabMode::None
            };
            cursor.visible = !grab;
        }
        grabbed = cursor.grab_mode != CursorGrabMode::None;
    }
    if *mode == CameraMode::Hero
        && (grabbed || buttons.pressed(MouseButton::Right))
        && motion.delta != Vec2::ZERO
    {
        orbit.yaw -= motion.delta.x * 0.0035;
        orbit.pitch = (orbit.pitch - motion.delta.y * 0.0035).clamp(
            crate::gameplay_camera::FOOT_PITCH_LIMITS[0],
            crate::gameplay_camera::FOOT_PITCH_LIMITS[1],
        );
    }
    if hero.downed {
        *input = HeroInput::default();
        return;
    }
    if *mode != CameraMode::Hero || hero.in_vehicle.is_some() || hero.stunt.is_some() {
        // the keys belong to the car or the free camera
        let keep_chute = input.chute_toggle;
        *input = HeroInput::default();
        input.chute_toggle = keep_chute;
        return;
    }
    let fwd = Vec3::new(-orbit.yaw.sin(), 0.0, -orbit.yaw.cos());
    let right = Vec3::new(orbit.yaw.cos(), 0.0, -orbit.yaw.sin());
    let mut mv = Vec3::ZERO;
    // movement and sprint come from the shared action service (`controls.rs`), whose
    // default keys are the project's WASD / arrows / Shift; nothing here reads them raw
    if controls.down(MOVE_FORWARD) {
        mv += fwd;
    }
    if controls.down(MOVE_BACK) {
        mv -= fwd;
    }
    if controls.down(MOVE_RIGHT) {
        mv += right;
    }
    if controls.down(MOVE_LEFT) {
        mv -= right;
    }
    input.movement = mv.normalize_or_zero();
    input.sprint = controls.down(SPRINT_ACTION);
    input.crouch = controls.down(CROUCH_ACTION);
    input.evade = controls.down(EVADE_ACTION);
    // (`F` fires the hook, `grapple_fire`; Ctrl+F is the tether gun, not FIRE_GRAPPLE)
    input.fire_grapple = keys.pressed(KeyCode::KeyF)
        && !(keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight));
    // edge-triggered keys accumulate until the fixed step consumes them
    if keys.just_pressed(KeyCode::Space) {
        if hero.grounded && !hero.parachute {
            input.jump = true;
        } else {
            input.chute_toggle = true;
        }
    }
    if keys.just_pressed(KeyCode::ControlLeft) {
        input.drop = true;
    }
    input.chute_pull =
        (controls.down(MOVE_FORWARD) as i32 - controls.down(MOVE_BACK) as i32) as f32;
    input.chute_turn = (controls.down(MOVE_LEFT) as i32 - controls.down(MOVE_RIGHT) as i32) as f32;
}

/// `E`: into the nearest vehicle, or out of the one Rico drives.
#[allow(clippy::type_complexity)]
pub(crate) fn vehicle_entry(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<CameraMode>,
    mut heroes: Query<(
        Entity,
        &mut Hero,
        &mut Transform,
        &mut LinearVelocity,
        &mut Visibility,
    )>,
    cars: Query<(Entity, &Transform, &LinearVelocity, &Enterable), Without<Hero>>,
) {
    if !keys.just_pressed(KeyCode::KeyE) || *mode == CameraMode::Free {
        return;
    }
    let Ok((hero_entity, mut hero, mut tf, mut lin, mut vis)) = heroes.single_mut() else {
        return;
    };
    if hero.downed {
        return;
    }
    if let Some(car) = hero.in_vehicle {
        // out: beside the car, with its velocity
        if let Ok((_, car_tf, car_lin, v)) = cars.get(car) {
            let side = car_tf.rotation * Vec3::X * (v.reach + 1.2);
            tf.translation = car_tf.translation - side + Vec3::Y * 1.0;
            lin.0 = car_lin.0;
        }
        commands.entity(car).remove::<Occupied>();
        crate::physics::release_passenger(&mut commands, hero_entity, tf.translation, lin.0);
        *vis = Visibility::Inherited;
        hero.in_vehicle = None;
        hero.grounded = false;
        hero.facing = car_tf_yaw(cars.get(car).map(|c| c.1).ok());
        *mode = CameraMode::Hero;
        info!("rico: out of the vehicle");
        return;
    }
    if hero.parachute || hero.grapple.is_some() {
        return;
    }
    let nearest = cars
        .iter()
        .map(|(e, t, _, v)| (e, t.translation.distance(tf.translation) - v.reach))
        .filter(|(_, d)| *d < ENTER_RANGE)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((car, _)) = nearest {
        hero.in_vehicle = Some(car);
        commands.entity(car).insert(Occupied);
        crate::physics::stow_passenger(&mut commands, hero_entity, car);
        *vis = Visibility::Hidden;
        lin.0 = Vec3::ZERO;
        *mode = CameraMode::Drive;
        info!("rico: in the vehicle");
    }
}

/// How high above the known ground the placement ray starts, and how far down it looks.
const PLACE_RAY_RISE: f32 = 300.0;
const PLACE_RAY_REACH: f32 = 1500.0;
/// Fixed steps the ground must read the same before he is let go, and the longest wait.
const SETTLE_STABLE_TICKS: u32 = 10;
const SETTLE_MAX_TICKS: u32 = 64 * 10;

/// Where Rico's centre goes to stand at `(x, z)`: on the first static collider a ray
/// from `PLACE_RAY_RISE` above `top` (or the ground) hits (terrain, a base's slab, a
/// roof), else on `fallback_ground`. `filter` should leave out him and the movable
/// things. The flag says whether the ray found anything.
pub fn stand_on_ground(
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
    x: f32,
    z: f32,
    top: f32,
    fallback_ground: f32,
) -> (Vec3, bool) {
    let origin = Vec3::new(x, top.max(fallback_ground) + PLACE_RAY_RISE, z);
    match spatial.cast_ray(origin, Dir3::NEG_Y, PLACE_RAY_REACH, true, filter) {
        Some(hit) => (
            Vec3::new(x, origin.y - hit.distance + HEIGHT / 2.0 + 0.3, z),
            true,
        ),
        None => (Vec3::new(x, fallback_ground + HEIGHT / 2.0 + 0.3, z), false),
    }
}

/// Rico is held where he stands until the ground under him has colliders and has stopped
/// changing (the start tile's static collision comes in over a few frames, and a base
/// like tile 31,14's has slabs and floors above the terrain's height), re-placed on top
/// of whatever a ray from above finds each step. Added at the start and on respawn.
#[derive(Component, Default)]
pub struct Settling {
    ticks: u32,
    stable: u32,
    last_y: f32,
    /// The terrain's height at his x/z, once looked up (the ray's fallback), and the
    /// higher of that and where he started (the ray starts above it).
    ground: Option<(f32, f32)>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn settle_spawn(
    mut commands: Commands,
    spatial: SpatialQuery,
    game: Option<Res<Game>>,
    map: Option<Res<crate::map::MapData>>,
    streamer: Option<Res<crate::world::Streamer>>,
    movers: Query<(Entity, &RigidBody), Without<Hero>>,
    mut heroes: Query<(
        Entity,
        &Hero,
        &mut Settling,
        &Transform,
        &mut LinearVelocity,
    )>,
) {
    let Ok((entity, hero, mut s, tf, mut lin)) = heroes.single_mut() else {
        return;
    };
    if hero.in_vehicle.is_some() {
        commands.entity(entity).remove::<Settling>();
        return;
    }
    s.ticks += 1;
    let (x, z) = (tf.translation.x, tf.translation.z);
    let (ex, ez) = engine_xz(tf.translation);
    let (fallback, top) = *s.ground.get_or_insert_with(|| {
        let ground = match &game {
            Some(g) => {
                crate::map::ground_at(&g.fs, map.as_ref().and_then(|m| m.global.as_ref()), ex, ez)
            }
            None => tf.translation.y,
        };
        (ground, ground.max(tf.translation.y))
    });
    let filter = SpatialQueryFilter::from_excluded_entities(
        movers
            .iter()
            .filter(|(_, b)| **b != RigidBody::Static)
            .map(|(e, _)| e)
            .chain([entity]),
    );
    let (at, hit) = stand_on_ground(&spatial, &filter, x, z, top, fallback);
    // (the spatial query reads Position, so the move goes through commands)
    commands
        .entity(entity)
        .insert((Transform::from_translation(at), Position(at)));
    lin.0 = Vec3::ZERO;
    if hit && (at.y - s.last_y).abs() < 0.02 {
        s.stable += 1;
    } else {
        s.stable = 0;
    }
    s.last_y = at.y;
    // a tile whose models still stream in may be about to grow trimesh colliders
    let tile = jc2_formats::terrain::tile_index(ex, ez);
    let streaming = streamer
        .zip(tile)
        .is_some_and(|(st, t)| st.loaded.get(&t).is_some_and(|l| !l.pending.is_empty()));
    if (s.stable >= SETTLE_STABLE_TICKS && !streaming) || s.ticks > SETTLE_MAX_TICKS {
        info!(
            "rico: placed at {at:?} ({}; terrain {fallback:.1} m){}",
            if hit {
                "on a collider"
            } else {
                "no collider found"
            },
            if s.ticks > SETTLE_MAX_TICKS {
                ", gave up waiting"
            } else {
                ""
            }
        );
        commands.entity(entity).remove::<Settling>();
    }
}

/// The yaw (`Quat::from_rotation_y`, the model facing [`MODEL_FORWARD`] = −z) that looks
/// along `dir` in the horizontal plane; `keep` when `dir` is nearly vertical (a hook
/// straight above him must not spin him round).
pub fn heading(dir: Vec3, keep: f32) -> f32 {
    if dir.x * dir.x + dir.z * dir.z < 1e-4 * dir.length_squared() {
        keep
    } else {
        yaw_toward(dir)
    }
}

fn car_tf_yaw(tf: Option<&Transform>) -> f32 {
    tf.map(|t| yaw_of(t.rotation)).unwrap_or(0.0)
}

/// `F`: fire the hook (`grapple.rs`: it flies out, locks on, attaches on arrival), cut a
/// flight short, or let go.
#[allow(clippy::too_many_arguments)]
pub(crate) fn grapple_fire(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<CameraMode>,
    params: Option<Res<HeroParams>>,
    lock: Res<LockOn>,
    spatial: SpatialQuery,
    mut heroes: Query<(Entity, &mut Hero, &Transform, &mut GrappleRig)>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    poses: Query<(&Position, &Rotation)>,
    colliders: Query<&ColliderOf>,
    bodies: crate::grapple::Bodies,
) {
    // Ctrl+F is the tether gun (stunts.rs)
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if !keys.just_pressed(KeyCode::KeyF) || *mode != CameraMode::Hero || ctrl {
        return;
    }
    let Ok((entity, mut hero, tf, mut rig)) = heroes.single_mut() else {
        return;
    };
    if hero.downed {
        return;
    }
    if hero.in_vehicle.is_some() {
        return;
    }
    let Ok(cam) = cams.single() else { return };
    crate::grapple::try_fire(
        entity,
        &mut hero,
        &mut rig,
        tf.translation,
        cam,
        &lock,
        &spatial,
        &Tuning::new(params.as_deref()),
        &poses,
        &colliders,
        &bodies,
    );
}

/// Ground probe, walking, jumping, the chute and the hook — one fixed step.
#[allow(clippy::type_complexity)]
pub(crate) fn hero_physics(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut input: ResMut<HeroInput>,
    params: Option<Res<HeroParams>>,
    mut heroes: Query<(
        Entity,
        &mut Hero,
        &mut Transform,
        &mut LinearVelocity,
        Option<&mut GrappleRig>,
    )>,
    cars: Query<(&Transform, &LinearVelocity), (With<Enterable>, Without<Hero>)>,
    cameras: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let Ok((entity, mut hero, mut tf, mut lin, rig)) = heroes.single_mut() else {
        return;
    };
    if hero.downed {
        *input = HeroInput::default();
        hero.speed = 0.0;
        // Retain gravity/contacts; never run the movement, jump or grapple controller.
        lin.x = 0.0;
        lin.z = 0.0;
        return;
    }
    let mut no_rig = GrappleRig::default();
    let rig = rig.map(|r| r.into_inner()).unwrap_or(&mut no_rig);
    if let Some(car) = hero.in_vehicle {
        if let Ok((car_tf, car_lin)) = cars.get(car) {
            tf.translation = car_tf.translation;
            lin.0 = car_lin.0;
        }
        *input = HeroInput::default();
        return;
    }
    if hero.stunt.is_some() {
        // pinned to a roof by stunts.rs
        *input = HeroInput::default();
        hero.speed = 0.0;
        hero.grounded = true;
        return;
    }
    let tuning = Tuning::new(params.as_deref());

    // ground probe: a sphere cast down from the capsule's centre
    let half = HEIGHT / 2.0 - RADIUS;
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    let probe = Collider::sphere(RADIUS * 0.9);
    let hit = spatial.cast_shape(
        &probe,
        tf.translation,
        Quat::IDENTITY,
        Dir3::NEG_Y,
        &ShapeCastConfig::from_max_distance(half + GROUND_PROBE),
        &filter,
    );
    let (on_ground, normal, gap) = match hit {
        Some(h) if h.normal1.y > MAX_SLOPE.cos() => (true, h.normal1, h.distance - half),
        _ => (false, Vec3::Y, f32::MAX),
    };
    hero.jump_cooldown = (hero.jump_cooldown - dt).max(0.0);
    hero.jump_buffer = if input.jump {
        JUMP_BUFFER
    } else {
        (hero.jump_buffer - dt).max(0.0)
    };
    let grappling = hero.grapple.is_some();
    // the sea: once the water reaches his chest he swims instead of wading
    let surface = crate::world::SEA_LEVEL;
    hero.swimming = tf.translation.y < surface - SWIM_DEPTH + 0.3
        && !grappling
        && !(on_ground && tf.translation.y > surface - SWIM_DEPTH);
    hero.grounded =
        on_ground && hero.jump_cooldown == 0.0 && !grappling && lin.y <= 1.0 && !hero.swimming;
    hero.ground_normal = normal;
    if hero.grounded {
        hero.air_time = 0.0;
        if hero.parachute {
            hero.parachute = false;
        }
    } else {
        hero.air_time += dt;
    }

    let mut v = lin.0;
    if !hero.parachute {
        hero.canopy_live = false;
    }
    if hero.grounded {
        hero.cut_matrix = None;
    }
    if hero.base_jump.is_none() {
        hero.base_v0 = None;
    }
    // the canopy model flies from the first frame of a `CParachuteController` state, not from
    // the press (the pull-open state belongs to the freefall controller)
    let canopy_flight =
        hero.parachute && (hero.chute_state || hero.grapple.is_some() || !hero.foot_driven);
    // (the dive matrix survives until the canopy's opening frame has read it)
    if !(hero.skydiving && hero.free_air) && !(canopy_flight && !hero.canopy_live) {
        hero.skydive = None;
    }
    // a landing that the animation graph does not let him walk out of (`Hero::move_lock`)
    let locked = hero.move_lock && hero.grounded;
    let mv = if locked { Vec3::ZERO } else { input.movement };
    if hero.swimming {
        // tread water at the surface, paddle with the keys, no chute, no jump
        hero.parachute = false;
        hero.air_time = 0.0;
        let target = mv * SWIM_SPEED;
        let step = SWIM_ACCEL * dt;
        let delta = Vec3::new(target.x - v.x, 0.0, target.z - v.z);
        let change = if delta.length() > step {
            delta.normalize() * step
        } else {
            delta
        };
        v.x += change.x;
        v.z += change.z;
        let want_y = surface - SWIM_DEPTH;
        let vy_target = ((want_y - tf.translation.y) * 2.0).clamp(-3.0, 3.0);
        v.y += (vy_target - v.y) * (1.0 - (-dt / 0.3).exp());
        v.y += 9.81 * dt; // cancel the step's gravity
        if mv != Vec3::ZERO {
            hero.facing = yaw_toward(mv);
        }
    } else if hero.grounded {
        // walk on the ground plane: the speed is the clips' root motion times the state's
        // time scale, from the low-passed stick (native; `rico-locomotion-motion.md`)
        let gain = 1.0 - (1.0 - STICK_GAIN).powf(dt * STICK_HZ);
        let stick = hero.stick;
        hero.stick = stick + (mv - stick) * gain;
        if hero.stick.length() < 1e-4 {
            hero.stick = Vec3::ZERO;
        }
        let sp = hero.stick.length();
        hero.free_air = true;
        let aiming = hero.aim_yaw.is_some();
        // (the foot machine's roll and crouch states move him by their clip: `foot_actions.rs`)
        let driven = hero.root_drive;
        let sprinting = input.sprint && mv.length() > 0.9 && !aiming && driven.is_none();
        if sprinting {
            if !hero.was_sprinting {
                hero.dash_counter = 1.0;
            }
            hero.dash_counter = (hero.dash_counter - dt / DASH_SECONDS).max(0.0);
        }
        hero.was_sprinting = sprinting;
        let speed = if let Some(rd) = driven {
            rd.speed
        } else if mv == Vec3::ZERO && sp <= 0.1 {
            0.0 // the stop deadzone (controller +0x34)
        } else if sprinting {
            dash_speed(hero.dash_counter)
        } else if aiming {
            strafe_speed(sp)
        } else {
            navigation_speed(sp)
        };
        let dir = if let Some(rd) = driven {
            // face the desired direction (the raw stick, S+0x7c, as the sprint does) at the
            // state's turn rate (not the aim: the roll and the crouch are not strafe
            // states), travel along the facing
            if mv.length_squared() > 1e-6 {
                hero.facing = turn_toward(hero.facing, yaw_toward(mv), rd.turn * dt);
            }
            facing_dir(hero.facing)
        } else if aiming {
            hero.facing = ground_facing(hero.facing, mv, hero.aim_yaw);
            hero.stick.normalize_or_zero()
        } else {
            // he turns towards the (filtered; raw when sprinting) stick at 2 x rate
            let want = if sprinting { mv } else { hero.stick };
            if want.length_squared() > 1e-6 {
                let rate = if sprinting { TURN_SLOW } else { TURN_GROUND };
                hero.facing = turn_toward(hero.facing, yaw_toward(want), rate * dt);
            }
            facing_dir(hero.facing)
        };
        let along = (dir - normal * dir.dot(normal)).normalize_or_zero();
        let target = along * speed;
        let horizontal = Vec3::new(v.x, 0.0, v.z);
        let delta = target - horizontal;
        // (a root-motion state sets the velocity outright, as the clip dictates it)
        let step = if driven.is_some() { f32::MAX } else { GROUND_ACCEL * dt };
        let change = if delta.length() > step {
            delta.normalize() * step
        } else {
            delta
        };
        v.x += change.x;
        v.z += change.z;
        // Follow the ground plane exactly and cancel the step's gravity: a frictionless
        // capsule pressed into a slope, or gravity left in, pushes him sideways and he
        // slides downhill. Closing a small gap is done along the normal, never straight
        // down, for the same reason.
        let tangent_y = if normal.y > 1e-3 {
            -(normal.x * v.x + normal.z * v.z) / normal.y
        } else {
            0.0
        };
        v.y = tangent_y + 9.81 * dt;
        if gap > 0.02 {
            v -= normal * 2.0;
        }
        // native (`0x007b6520`, JC2-170): the jump handler needs the animation state to take
        // `ACT_JUMP_AT`; a press is retried for 0.3 s (the landings do not take it)
        if hero.jump_buffer > 0.0 && hero.jump_ok && hero.jump_delay <= 0.0 {
            hero.jump_delay = JUMP_DELAY;
            hero.jump_buffer = 0.0;
        }
        if hero.jump_delay > 0.0 {
            // the `jump_start` event at 0.1 s into the start clip: the climb begins
            hero.jump_delay -= dt;
            if hero.jump_delay <= 0.0 {
                v.y = JUMP_CLIMB_SPEED;
                hero.grounded = false;
                hero.jump_cooldown = 0.25;
                hero.climb_left = JUMP_CLIMB_SECONDS;
            }
        }
    } else if canopy_flight {
        // the canopy flight model (`0x005a9950`, JC2-263): the force law, the spring on the
        // three Euler angles and, with a wire attached, the hook assist and the 40 m/s cap.
        // The hook's world position is the anchor here (the wire end record: inferred).
        use crate::parachute_flight::{CanopyInput, HookAssist};
        if !hero.canopy_live {
            // the first frame of the unfold state (board 4d7f7642): the opening frame converts
            // the character matrix (`0x005a9a72`, JC2-270): the dive matrix carries its pitch
            // and bank into the canopy; from any other state the matrix is taken as yaw only
            // (inferred: the on-foot matrix)
            hero.canopy_live = true;
            hero.canopy = match hero.skydive {
                Some(sd) => crate::parachute_flight::CanopyFlight::open_from_matrix(&sd.matrix),
                None => crate::parachute_flight::CanopyFlight::open(hero.facing),
            };
            if hero.grapple.is_none() {
                // native opening transform (`0x005a9950`, JC2-170): part of the fall
                // becomes forward speed; the canopy model takes the velocity from here
                v = chute_open_velocity(v);
            }
        }
        let hook = hero.grapple.map(|g| HookAssist { target: g.anchor });
        // `S+4`: the character's right axis against the active camera's (`0x0077fc80`)
        let camera_angle = cameras.single().map_or(0.0, |c| {
            let right = Quat::from_rotation_y(hero.facing) * Vec3::X;
            crate::parachute_flight::camera_angle(right, crate::gameplay_camera::camera_right(c))
        });
        let inp = CanopyInput {
            pitch: input.chute_pull.clamp(-1.0, 1.0),
            roll: -input.chute_turn.clamp(-1.0, 1.0),
            camera_angle,
            // setting 0x26, code default 0 (the shipped option default is unknown)
            pitch_invert: false,
            fire_grapple: input.fire_grapple,
        };
        v = hero.canopy.step(v, inp, hook, tf.translation, dt);
        // the canopy owns the matrix now; a later cut-away restarts the freefall model
        hero.skydive = None;
        // the canopy yaw is the character's (one frame: facing -Z at 0, positive to the left)
        hero.facing = hero.canopy.a34;
        // the model carries its own gravity: cancel the step's
        v.y += 9.81 * dt;
    } else if let (Some(w), true, true) = (hero.base_jump, hero.free_air, hero.grapple.is_none()) {
        // `CBaseJumpController` (`0x00756ff0`): speed from the entry speed to 12 m/s along the
        // clip's progress, along the facing, no gravity, the clip's root motion ignored
        let v0 = *hero.base_v0.get_or_insert(v.length());
        v = facing_dir(hero.facing) * crate::parachute_flight::base_jump_speed(v0, w);
        v.y += 9.81 * dt;
    } else if hero.skydiving && hero.free_air && hero.grapple.is_none() {
        // the freefall model (`0x006b9290`): drag and the lift-like term from the two pose
        // curves, the vertical cap, and the three-matrix turn of the body. The controller
        // start (`0x0065c9a0`) runs when the controller changes to the SkyDive one: after a
        // cut-away the body matrix is the canopy's (pitch and bank carried over)
        let mut sd = hero.skydive.unwrap_or_else(|| match hero.cut_matrix.take() {
            Some(m) => crate::parachute_flight::Skydive::start_from_matrix(m),
            None => crate::parachute_flight::Skydive::start(hero.facing),
        });
        v = sd.step(v, input.chute_pull, input.chute_turn, 0.0, dt);
        hero.facing = sd.yaw();
        hero.skydive = Some(sd);
        v.y += 9.81 * dt;
    } else {
        hero.jump_delay = 0.0;
        let h = Vec3::new(v.x, 0.0, v.z);
        if hero.grapple.is_some() {
            hero.free_air = false;
        }
        if !hero.free_air {
            // on the wire: the old (non-native) little steering, the hook owns the rest
            v.x += mv.x * GRAPPLE_AIR_ACCEL * dt;
            v.z += mv.z * GRAPPLE_AIR_ACCEL * dt;
            if h.length() > 1.0 {
                hero.facing = yaw_toward(h);
            }
        } else {
            // the Havok character context (`research/formats/rico-jump-air-context.md`,
            // JC2-254): Jumping while the start clip plays, InAir after it
            let target = mv * AIR_STEER + facing_dir(hero.facing) * hero.air_root;
            let jumping = hero.climb_left > 0.0;
            if jumping {
                hero.climb_left -= dt;
            }
            v = air_velocity_step(v, target, jumping, dt);
            // the engine's own gravity acts on the step too: the context's -21 is the net
            v.y += if jumping { 9.81 * dt } else { 0.0 };
            if h.length() > 1.0 {
                hero.facing = turn_toward(hero.facing, yaw_toward(h), TURN_SLOW * dt);
            }
        }
    }

    // hanging from the hook: `Space` climbs or leaps, `Ctrl` drops (before the chute sees it)
    crate::grapple::hang_inputs(&mut hero, rig, &mut input, &mut v, tf.translation);
    crate::grapple::climb_assist(&hero, rig, &mut v, dt);
    if input.chute_toggle {
        input.chute_toggle = false;
        if hero.parachute {
            hero.parachute = false;
            if hero.canopy_live {
                hero.cut_matrix = Some(hero.canopy.matrix());
                hero.canopy_live = false;
            }
        } else if !hero.grounded && hero.chute_ok {
            // native (`0x0074edd0`, JC2-170): the canopy opens only if the animation state
            // takes `ACT_PULLCHORD` (the skydive does; the fall and its lead-in do not), on
            // the frame of the press. No room-under-the-feet rule (JC2-263): the skydive
            // state is only entered with 28.5 m of air under it, and the landing edges take
            // the canopy down
            // (the chord pull only opens the canopy object; the freefall model keeps flying
            // through `S_PULL_OPEN_PARACHUTE_VERTICAL` and the canopy model with its opening
            // transform starts at the first frame of the unfold state, above)
            hero.parachute = true;
            hero.canopy_live = false;
        }
    }

    // the hook: wind in with a speed that rises and brakes into the goal, then land,
    // cling, hang or let go (`grapple.rs`)
    crate::grapple::drive_attached(
        &mut hero,
        rig,
        tf.translation,
        &mut v,
        &tuning,
        dt,
        &spatial,
        &filter,
    );

    hero.speed = Vec3::new(v.x, 0.0, v.z).length();
    lin.0 = v;
    input.jump = false;
    input.drop = false;
}

/// One update of the Havok character context in the air (`rico-jump-air-context.md`, JC2-254).
/// Jumping (the start clip plays): the velocity is the desired velocity (gain 1.0, no gravity),
/// the vertical part being the clip's climb. InAir: the vertical speed is kept and `-21 dt`
/// added (no terminal velocity); horizontally `v += 0.05 (desired - v)` per update, in both
/// directions (it brakes too). `desired` is the lift plus root motion; the update rate (here
/// `STICK_HZ`) is JC2-255 and the sprint-jump lift ramp (`0x006dfcc0`) is not modelled.
fn air_velocity_step(v: Vec3, desired: Vec3, jumping: bool, dt: f32) -> Vec3 {
    if jumping {
        return Vec3::new(desired.x, JUMP_CLIMB_SPEED, desired.z);
    }
    let gain = 1.0 - (1.0 - AIR_GAIN).powf(dt * STICK_HZ);
    Vec3::new(
        v.x + (desired.x - v.x) * gain,
        v.y - (GRAVITY - 9.81) * dt,
        v.z + (desired.z - v.z) * gain,
    )
}

/// The velocity the canopy gives him the frame it opens (native, `0x005a9950`: the opening
/// frame of the flight model; fixtures in `foot_controller_probe.py`). Falling: `vc` is the
/// fall speed capped at 30 m/s; 40 % of it is added along the horizontal heading of the
/// velocity and the vertical speed becomes `0.6 vc + (vy - vc)`. Rising or level: unchanged.
pub(crate) fn chute_open_velocity(v: Vec3) -> Vec3 {
    if v.y >= 0.0 {
        return v;
    }
    let vc = v.y.max(-30.0);
    let h = Vec3::new(v.x, 0.0, v.z).normalize_or_zero();
    Vec3::new(
        v.x + 0.4 * vc.abs() * h.x,
        0.6 * vc + (v.y - vc),
        v.z + 0.4 * vc.abs() * h.z,
    )
}

/// The finish margin of the deployment states' `C_WAIT_FOR_ANIM_TO_FINISH`: the predicate
/// `0x004ae580` is true at clip time >= duration - margin, margin = asb key `0x96270a00` of the
/// state's descriptor (`parachute-deploy-duration.md`, board caf7b21c, `kc001_base.asb`). The
/// pull-open lasts 0.463 s, the unfold continues the clip to 1.633 s (the glide starts 1.633 s
/// after the press), the cut-away lead-in ends at 1.0 s.
fn deploy_finish_margin(state: &str) -> f32 {
    match state {
        "S_PULL_OPEN_PARACHUTE_VERTICAL" => 1.97,
        "S_UNFOLD_PARACHUTE_VERTICAL" => 0.8,
        "S_UNFOLD_PARACHUTE_HORIZONTAL" => 1.2,
        "S_PARACHUTE_DETACH_TO_FREEFALL" => 0.4,
        _ => 0.0,
    }
}

/// The old air steering on the wire (non-native; the hook code owns that flight).
const GRAPPLE_AIR_ACCEL: f32 = 8.0;

/// Turn `cur` towards `target` by at most `max` radians.
fn turn_toward(cur: f32, target: f32, max: f32) -> f32 {
    cur + wrap_angle(target - cur).clamp(-max, max)
}

/// `S_UPRIGHT_BASIC_NAVIGATION` straight ahead (`007d6790`, probe `locomotion_motion_probe.py`):
/// time scale `min(1.5825 sp + 0.66, 1.5825)`, run weight from 1.042 to 1.5625 times `sp`,
/// `+0.07` (0.3 * 0.5 - 0.22), times the blend of the walk and run clip speeds.
pub(crate) fn navigation_speed(sp: f32) -> f32 {
    let ts = (1.5825 * sp + 0.66).min(1.5825);
    let w = ((ts - 1.042) / (1.5625 - 1.042)).clamp(0.0, 1.0) * sp;
    let w = w.min(1.0);
    (ts + 0.07) * ((1.0 - w) * WALK_CLIP + w * RUN_CLIP)
}

/// `S_DASH` (`00606cb0`): counter `c` 1 -> 0 over 1.5 s, time scale `(1.5625 - 2) c + 2`,
/// the run clip at weight `c` and the dash clip at `1 - c`.
pub(crate) fn dash_speed(c: f32) -> f32 {
    let ts = (1.5625 - 2.0) * c + 2.0;
    ts * (c * RUN_CLIP + (1.0 - c) * DASH_CLIP)
}

/// `S_UPRIGHT_STRAFE` straight ahead (`007ea9a0`): 5.00 m/s at full stick, 2.06 with the
/// WALK key.
pub(crate) fn strafe_speed(sp: f32) -> f32 {
    let m = (1.5625 * sp + 0.6).min(1.5625);
    let ts = 0.96 * m;
    let w = ((m - 1.0 / 0.96) / (1.0 / 0.64 - 1.0 / 0.96)).clamp(0.0, 1.0) * sp;
    let w = w.min(1.0);
    ts * ((1.0 - w) * WALK_CLIP + w * RUN_CLIP)
}

/// Beyond this angle between the aim and the way he runs, his legs strafe.
const STRAFE_FROM: f32 = 0.9;

/// The way his legs face on the ground: where he walks; aiming, standing still, the aim;
/// aiming while moving, still where he walks unless the aim is more than
/// [`STRAFE_FROM`] off it, then side-on to it (he strafes, the spine turns the rest).
pub fn ground_facing(facing: f32, mv: Vec3, aim: Option<f32>) -> f32 {
    let moving = mv != Vec3::ZERO;
    match aim {
        Some(a) if !moving => a,
        Some(a) => {
            let walk = yaw_toward(mv);
            let off = wrap_angle(a - walk);
            if off.abs() <= STRAFE_FROM {
                walk
            } else {
                walk + off.signum() * std::f32::consts::FRAC_PI_2
            }
        }
        None if moving => yaw_toward(mv),
        None => facing,
    }
}

fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Pick the clip for Rico's state; landings play out before anything else.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn hero_animation(
    time: Res<Time>,
    rig: Option<Res<Rig>>,
    ragdoll: Option<Res<crate::ragdoll::RagdollAsset>>,
    mut heroes: Query<(
        Entity,
        &mut Hero,
        &Transform,
        &LinearVelocity,
        &mut Animator,
        Option<&mut Arms>,
        Option<&Children>,
        Option<&GrappleRig>,
        Option<&crate::ragdoll::FallbackPose>,
        Option<&mut crate::foot_machine::FootDriver>,
    )>,
    models: Query<&Transform, (With<HeroModel>, Without<Hero>)>,
    pose: Option<Res<crate::poses::PoseOverride>>,
    input: Res<HeroInput>,
    foot: Option<Res<FootMachineData>>,
    spatial: SpatialQuery,
    mut seen: Local<HeroAnimMemory>,
    mut canopy_feed: Option<ResMut<crate::parachute::CanopyPoseFeed>>,
) {
    let Some(rig) = rig else { return };
    let dt = time.delta_secs();
    for (entity, mut hero, tf, lin, mut an, mut arms, children, hook, fallback, mut driver) in
        &mut heroes
    {
        // the canopy blend inputs for `parachute::animate_canopy`
        if let Some(feed) = canopy_feed.as_mut() {
            feed.0 = hero.canopy.pose;
        }
        if hero.downed {
            seen.fall_speed = 0.0;
            hero.aim_yaw = None;
            hero.move_lock = false;
            if let Some(d) = driver.as_mut() {
                d.set_state("S_UPRIGHT_IDLE");
            }
            // Physics activation captures the last propagated pose; do not replace it.
            if ragdoll.is_none() || fallback.is_some() {
                an.hold(Clip::Idle, 0.0, 0.1);
            }
            an.layers = Default::default();
            an.aim.weight = 0.0;
            an.aim.target = 0.0;
            if let Some(arms) = arms.as_mut() {
                arms.aim_for = 0.0;
                arms.alert_for = 0.0;
                arms.gait = Gait::Seated; // stow the gun; no wield/aim layer
            }
            continue;
        }
        let model_rot = children
            .and_then(|c| c.iter().find_map(|c| models.get(c).ok()))
            .map(|t| t.rotation)
            .unwrap_or(Quat::from_rotation_y(hero.facing));
        let class = arms.as_ref().and_then(|a| a.class);
        // the hardest fall since he left the ground decides the landing
        if !hero.grounded {
            seen.fall_speed = seen.fall_speed.max(-lin.y);
        }
        // the native foot machine: physical events in, actions out, a state back
        if let (Some(data), Some(d)) = (foot.as_deref(), driver.as_deref_mut()) {
            drive_foot(data, d, &mut hero, entity, tf, lin, &an, &rig, &input, &spatial, dt);
        }
        let (clip, gait, rate, blend) = match pose.as_deref() {
            // debug: `--pose`
            Some(p) => (Some(p.clip), p.gait, 1.0, 0.2),
            None => pick_base(
                &hero,
                lin,
                &an,
                &rig,
                class,
                hook,
                &mut seen,
                foot.as_deref().zip(driver.as_deref()),
            ),
        };
        if let Some(clip) = clip {
            an.rate = rate;
            if clip == Clip::Chute {
                // the speed axis of the canopy's time blend: the clip's time is its length
                // times (axis + 1) / 2 (`CParachuteState`, `00713d60`). The axis is the
                // native `D+4` (physical pitch, stick feed-forward, flare offset; landing
                // doc section 10.3), not the stick itself
                let axis = hero.canopy.pose.d4.clamp(-1.0, 1.0);
                an.hold(clip, rig.duration(clip) * (axis + 1.0) * 0.5, blend);
            } else if hook.is_some_and(|g| g.anim.body.held_pose()) {
                // `ANIM_REEL_FLIGHT` is a held pose (play mode 0): its middle (a `Gap`
                // shared by the other `CVelocityTimeVariableState` clips of the hook)
                an.hold(clip, rig.duration(clip) * 0.5, blend);
            } else {
                an.play_blend(clip, blend);
            }
        }
        if hero.grounded {
            seen.fall_speed = 0.0;
        }
        // the hook just went out: the left arm fires it (`LA_ANIM_RAISE_GRAPPLE`), then
        // holds out at it while it flies or pulls (`LA_ANIM_GRAPPLE_AIM`)
        // the arm entered `LA_S_RAISE_GRAPPLE` (`grapple/anim.rs`): the shot
        let shots = hook.map_or(0, |g| g.anim.shots);
        if shots != seen.shots
            && shots > 0
            && let Some(a) = arms.as_mut()
        {
            a.hooks += 1;
        }
        seen.shots = shots;
        let arm = hook.map(|g| g.anim.arm);
        match arms.as_mut() {
            Some(a) => {
                // standing with a two-handed gun he aims side-on, the legs a little to
                // the right of the aim (the alerted idle's stance)
                let class = a.class.unwrap_or_default();
                let stance = if !class.one_handed() && hero.speed < 0.4 {
                    -0.5
                } else {
                    0.0
                };
                hero.aim_yaw =
                    (a.aiming() && gait.holds(class)).then(|| yaw_toward(a.aim_dir) + stance);
                drive_upper_body(&mut an, a, gait, model_rot, false, dt);
                if gait == Gait::Chute {
                    // its turn axis: the native bank axis `D+8` (a right bank positive;
                    // same time law as the speed axis; which Rico clip pairs with it is
                    // non-native, task 57fed721)
                    let axis = hero.canopy.pose.d8.clamp(-1.0, 1.0);
                    let t = rig.duration(Clip::ChuteTurn) * (axis + 1.0) * 0.5;
                    an.hold_layer(crate::rig::Slot::Wield, Clip::ChuteTurn, t, 0.3);
                }
            }
            None => hero.aim_yaw = None,
        }
        // the left arm's graph (`leftarmstatemachine`): holds out at the hook while it flies
        // and the wire holds (`LA_S_WAITING_FOR_HOOK` / `_HOOK_HIT` / `_GRAPPLE`), pulls
        // (`LA_S_GRAPPLE_PULL`), drops back (`LA_S_GRAPPLE_DETACH` / `_HOOK_MISSED` /
        // `_TO_IDLE`), fires the second hook (`LA_S_DUAL_GRAPPLE`)
        let action = an.layer(crate::rig::Slot::Action).clip;
        let entered = arm != seen.arm;
        seen.arm = arm;
        match arm {
            Some(
                crate::grapple::ArmState::WaitingForHook
                | crate::grapple::ArmState::HookHit
                | crate::grapple::ArmState::Grapple,
            ) => {
                if action.is_none() || action == Some(Clip::GrappleAim) {
                    an.set_layer(crate::rig::Slot::Action, Some(Clip::GrappleAim), 0.1, false);
                }
            }
            Some(
                s @ (crate::grapple::ArmState::GrapplePull
                | crate::grapple::ArmState::GrappleDetach
                | crate::grapple::ArmState::HookMissed
                | crate::grapple::ArmState::GrappleToIdle
                | crate::grapple::ArmState::DualGrapple),
            ) => {
                if entered && let Some(c) = s.clip() {
                    // layer 2 is the left arm: `grpl_idle_to_pull` is a whole-body take
                    an.play_once_masked(
                        crate::rig::Slot::Action,
                        c,
                        1.0,
                        s.blend().max(0.05),
                        "LeftShoulder",
                    );
                }
            }
            _ => {
                if action == Some(Clip::GrappleAim) {
                    an.set_layer(crate::rig::Slot::Action, None, 0.4, false);
                }
            }
        }
    }
}

/// What `hero_animation` remembers between frames.
#[derive(Default)]
pub(crate) struct HeroAnimMemory {
    /// The fastest he fell (m/s) since leaving the ground.
    fall_speed: f32,
    /// How many shots the hook's left arm had raised (`GrappleAnim::shots`).
    shots: u32,
    /// The left-arm grapple state of the last frame.
    arm: Option<crate::grapple::ArmState>,
}

/// The main-layer states whose clip `kc001_base.asb` binds (a test reads the set and checks
/// each file is among the state's `.ban` list). States that blend several clips by a
/// parameter name the one the runtime plays: `S_SKYDIVE` (four clips by turn), `S_PARACHUTE`
/// (the speed/turn time blend), `S_DASH` (lean and slope clips). Gaps in
/// `research/formats/rico-controller-next.md`.
pub(crate) const NATIVE_STATE_CLIPS: &[(&str, Clip)] = &[
    ("S_JUMP_START_STANDING", Clip::JumpStartStanding),
    ("S_JUMP_FLIGHT_STANDING", Clip::JumpFlightStanding),
    ("S_JUMP_START_SPRINT_FORWARD", Clip::JumpStartRun),
    ("S_JUMP_TO_FALL_TRANSITION", Clip::JumpTimeBlend),
    ("S_JUMP_TO_SKYDIVE", Clip::JumpToSkydive),
    ("S_FALL_TO_SKYDIVE", Clip::FallToSkydive),
    ("S_SKYDIVE", Clip::Dive),
    ("S_PULL_OPEN_PARACHUTE_VERTICAL", Clip::ChuteOpen),
    ("S_UNFOLD_PARACHUTE_VERTICAL", Clip::ChuteOpen),
    ("S_UNFOLD_PARACHUTE_HORIZONTAL", Clip::ChuteOpenHorizontal),
    ("S_PARACHUTE", Clip::Chute),
    ("S_PARACHUTE_DETACH_TO_FREEFALL", Clip::ChuteDetach),
    ("S_JUMP_LAND", Clip::LandLight),
    ("S_JUMP_LAND_INTERRUPT", Clip::LandLight),
    ("S_JUMP_LAND_HEAVY", Clip::LandHeavy),
    ("S_JUMP_RUNNING_LAND", Clip::Land),
    ("S_JUMP_RUNNING_LAND_INTERRUPT", Clip::Land),
    ("S_PARACHUTE_LANDING", Clip::ChuteLand),
    ("S_LANDING_ROLL", Clip::LandRoll),
    ("S_LANDING_HARD", Clip::LandHard),
    ("S_DASH", Clip::Sprint),
    ("S_DASH_STOP", Clip::DashToIdle),
    // JC2-051: the crouch (`crouch_DN` 0.4 s, `cr_idle`, `crouch_UP` 0.5 s) and the evasive roll
    ("S_DOWN_TO_CROUCH", Clip::CrouchDown),
    ("S_CROUCH", Clip::Crouch),
    ("S_UP_FROM_CROUCH", Clip::CrouchUp),
    ("S_EVADE", Clip::EvadeFwd),
    ("S_EVADE_INTERRUPT", Clip::EvadeFwd),
    ("S_SWIM_IDLE", Clip::SwimIdle),
    ("S_BLEND_TO_SWIM_IDLE", Clip::SwimIdle),
    ("S_SWIM_NAVIGATION", Clip::Swim),
    ("S_SWIM_RUN_TO_SWIM", Clip::RunToSwim),
    ("S_SWIM_DIVE_IDLE", Clip::SwimDiveIdle),
    // approximations, not in the animation set's list for the state: `S_FALL` is
    // `fall_base_pose` plus additive layers (the runtime has no additive blend), `fall.ban`
    // is the sprint-forward flight's clip; `S_UPRIGHT_IDLE` is a static `std_idle_var*`
    // stance plus the additive breathing layer, `std_idle.ban` is `S_UPRIGHT_IDLE_VARIED`'s.
];

/// Approximations the table above does not check (see the comment there).
pub(crate) const APPROXIMATE_STATE_CLIPS: &[(&str, Clip)] =
    &[("S_FALL", Clip::Fall), ("S_UPRIGHT_IDLE", Clip::Idle)];

fn native_clip(state: &str) -> Option<Clip> {
    NATIVE_STATE_CLIPS
        .iter()
        .chain(APPROXIMATE_STATE_CLIPS)
        .find(|(s, _)| *s == state)
        .map(|(_, c)| *c)
}

/// The states whose clip depends on the speed and on the legs' strafe, presented by the
/// ground rules at the end of `pick_base`.
fn is_locomotion_state(state: &str) -> bool {
    matches!(
        state,
        "S_UPRIGHT_IDLE" | "S_UPRIGHT_BASIC_NAVIGATION" | "S_UPRIGHT_STRAFE" | "S_UPRIGHT_IDLE_VARIED"
    )
}

/// The clip a state is waiting on for `C_WAIT_FOR_ANIM_TO_FINISH`.
fn expected_clip(state: &str, playing: Clip) -> Option<Clip> {
    if state == "S_UPRIGHT_STOP" {
        return matches!(playing, Clip::WalkToIdle | Clip::RunToIdle).then_some(playing);
    }
    native_clip(state)
}

/// Feed the foot machine this frame's physical events and let it move (`foot_machine.rs`).
#[allow(clippy::too_many_arguments)]
fn drive_foot(
    data: &FootMachineData,
    d: &mut crate::foot_machine::FootDriver,
    hero: &mut Hero,
    entity: Entity,
    tf: &Transform,
    lin: &LinearVelocity,
    an: &Animator,
    rig: &Rig,
    input: &HeroInput,
    spatial: &SpatialQuery,
    dt: f32,
) {
    use crate::foot_machine::{FootFrame, Sense, is_chute_state};
    let g = hero.grounded;
    hero.foot_driven = true;
    if !d.stepped() && d.state != data.machine.initial && !data.machine.initial.is_empty() {
        // the graph's own initial state (`S_UPRIGHT_IDLE`) before the first step
        d.set_state(&data.machine.initial);
    }
    let free = hero.in_vehicle.is_none() && hero.stunt.is_none() && hero.grapple.is_none();
    if !free {
        // the wire, a car or a roof own the pose: keep the machine where it would be after
        if hero.grapple.is_some() && hero.parachute {
            d.set_state("S_PARACHUTE");
        } else {
            d.set_state(if g { "S_UPRIGHT_IDLE" } else { "S_FALL" });
        }
        d.was_grounded = g;
        hero.move_lock = false;
        hero.jump_ok = true;
        hero.chute_ok = true;
        hero.skydiving = false;
        hero.chute_state = false;
        hero.base_jump = None;
        hero.air_root = 0.0;
        hero.root_drive = None;
        return;
    }
    let horizontal = lin.x.hypot(lin.z);
    if !g {
        d.note_air(horizontal, lin.y);
    }
    let frame = FootFrame {
        dt,
        touched_down: !d.was_grounded && g,
        // the physics' own jump (it arms `jump_cooldown`): not a ledge, not a hop over a
        // seam in the ground collider
        jumped: d.was_grounded && !g && hero.jump_cooldown > 0.0 && lin.y > 1.0 && !hero.swimming,
        moving: input.movement.length_squared() > 1e-4,
        aiming: hero.aim_yaw.is_some(),
        // native (`0x005ff2f0`, JC2-170): the dash needs the key held and the input
        // magnitude or the speed above 0.9 (the 0.25 s evade lockout has no evade here)
        sprint: input.sprint && (input.movement.length() > 0.9 || horizontal > 0.9),
        chute_wanted: hero.parachute,
        // hold mode; not on the wire, in a car or on a roof (`free`), not in the sea. The
        // controller's other blocks (`this+0x20`, `this+0x1c`, `005ebc90`: a grenade throw in
        // the left-arm layer, the modal flag `005cf980+0xd0/+0x2ad`) are Gap 2
        crouch: input.crouch && !hero.swimming && !hero.parachute,
        evade_down: input.evade,
        keyboard: true,
    };
    // `C_WAIT_FOR_ANIM_TO_FINISH` for a state: its clip is the one playing and has run out
    // (a state entered a moment ago, with another clip, has not)
    let finished = |state: &str| match expected_clip(state, an.clip) {
        Some(c) if c == an.clip => {
            !c.looping() && an.time + 1e-3 >= rig.duration(c) - deploy_finish_margin(state)
        }
        // a clip the runtime does not have cannot be waited for
        Some(c) => !rig.has(c),
        None => false,
    };
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    let clearance = if g {
        0.0
    } else {
        spatial
            .cast_ray(tf.translation, Dir3::NEG_Y, 40.0, true, &filter)
            .map_or(f32::INFINITY, |h| (h.distance - HEIGHT / 2.0).max(0.0))
    };
    // the cut-chute edges' `CForwardRayCastFanCondition` (`0x00799600`): 10 rays from the
    // planar forward to straight down, 60 m (the 30 m edge reads the same nearest hit),
    // from the body position plus 0.25 m. The body position is taken at the feet (inferred:
    // the capsule centre is HEIGHT / 2 above them).
    let forward = if !g && is_chute_state(&d.state) {
        let planar = Vec3::new(-hero.facing.sin(), 0.0, -hero.facing.cos());
        let origin = tf.translation - Vec3::Y * (HEIGHT / 2.0) + Vec3::Y * crate::parachute_flight::FAN_ORIGIN_UP;
        crate::parachute_flight::forward_fan(planar, crate::parachute_flight::FAN_RAYS)
            .into_iter()
            .filter_map(|r| Dir3::new(r).ok())
            .filter_map(|dir| spatial.cast_ray(origin, dir, 60.0, true, &filter))
            .map(|h| h.distance)
            .fold(f32::INFINITY, f32::min)
    } else {
        f32::INFINITY
    };
    let sea = crate::world::SEA_LEVEL;
    let mut sense = Sense {
        ground_contact: g,
        fall_time: d.fall_time,
        input_magnitude: input.movement.length().min(1.0),
        horizontal_speed: horizontal,
        vertical_speed: lin.y,
        anim_finished: false,
        main_blend_finished: an.fading.is_none(),
        feet_above_sea: tf.translation.y - HEIGHT / 2.0 - sea,
        hips_above_sea: tf.translation.y - sea,
        swimming: hero.swimming,
        ground_clearance: clearance,
        forward_clearance: forward,
        dash_enabled: input.sprint,
    };
    let ctx = crate::foot_machine::Ctx {
        machine: &data.machine,
        finished: &finished,
    };
    d.step(&ctx, &frame, &mut sense);
    d.was_grounded = g;
    // the running jump's flight clip carries forward root motion, 4.63 -> 4.34 m/s of clip
    // time at the state's playback speed 1.2 (`run_jump_timeblend`; JC2-241 section 6)
    hero.air_root = if d.state == "S_JUMP_TO_FALL_TRANSITION" {
        1.2 * (4.63 + (4.34 - 4.63) * (d.state_time * 1.2 / 1.467).min(1.0))
    } else {
        0.0
    };
    hero.root_drive = if g {
        crate::foot_actions::root_drive(&d.state, d.state_time, dt, rig)
    } else {
        None
    };
    hero.jump_ok = d.accepts(&data.machine, "ACT_JUMP_AT", &sense);
    hero.chute_ok = d.accepts(&data.machine, "ACT_PULLCHORD", &sense);
    // the freefall model (`0x006b9290`) runs in every `CSkyDiveController` state, the chord
    // pull and the cut-away included (`parachute-body-orientation.md` section 5)
    hero.skydiving = crate::hud::is_skydive_controller_state(&d.state);
    hero.chute_state = crate::gameplay_camera::is_parachute_controller_state(&d.state);
    // `CBaseJumpController`: the progress of the main layer's clip instance (`0x008c6e70`);
    // the state clock stands for the clip time (the clip starts with the state)
    hero.base_jump = (d.state == "S_JUMP_TO_SKYDIVE")
        .then(|| (d.state_time / rig.duration(Clip::JumpToSkydive).max(1e-3)).clamp(0.0, 1.0));
    hero.move_lock = matches!(
        d.state.as_str(),
        "S_JUMP_LAND" | "S_JUMP_LAND_HEAVY" | "S_LANDING_HARD"
    );
    // non-native resyncs (the dispatcher that keeps the real graph in step is undecoded):
    // an air state that never saw the ground, a state this runtime has no clip for
    let air_state = matches!(
        d.state.as_str(),
        "S_FALL" | "S_FALL_TO_SKYDIVE" | "S_SKYDIVE" | "S_JUMP_TO_FALL_TRANSITION"
    );
    let unknown = native_clip(&d.state).is_none()
        && !is_locomotion_state(&d.state)
        && d.state != "S_UPRIGHT_STOP";
    if (g && !hero.swimming && air_state && d.state_time > 0.5) || (unknown && d.state_time > 3.0) {
        d.set_state(if g { "S_UPRIGHT_IDLE" } else { "S_FALL" });
    }
}

/// The foot machine's presentation of its state, `None` for the locomotion states (which
/// follow the speed and the strafe) and states this runtime has no clip for.
fn native_base(
    data: &FootMachineData,
    d: &crate::foot_machine::FootDriver,
    hero: &Hero,
    an: &Animator,
    rig: &Rig,
) -> Option<(Option<Clip>, Gait, f32, f32)> {
    let state = d.state.as_str();
    let anim = data.set.states.get(state);
    let blend = anim.map_or(FADE, |a| a.blend);
    let rate = anim.map_or(1.0, |a| if a.speed > 0.0 { a.speed } else { 1.0 });
    let clip = if state == "S_UPRIGHT_STOP" {
        // `CStopBlendState` picks between the walk and run stops; by the pace he stops from
        // (inferred)
        match an.clip {
            Clip::WalkToIdle | Clip::RunToIdle => an.clip,
            Clip::Walk | Clip::Walk1H | Clip::WalkStrafeLeft | Clip::WalkStrafeRight => {
                Clip::WalkToIdle
            }
            _ => Clip::RunToIdle,
        }
    } else if is_locomotion_state(state) {
        return None;
    } else {
        native_clip(state)?
    };
    if !rig.has(clip) {
        return None;
    }
    let gait = if crate::foot_machine::is_chute_state(state) {
        Gait::Chute
    } else if state == "S_DASH" {
        Gait::Sprint
    } else if matches!(state, "S_CROUCH" | "S_DOWN_TO_CROUCH" | "S_UP_FROM_CROUCH") {
        // (inferred: the held gun takes its crouched pose, the roof crouch's `cr_idle_*` set;
        // the upper-body graph has no crouch states of its own)
        Gait::Crouch
    } else if state.starts_with("S_SWIM") || state == "S_BLEND_TO_SWIM_IDLE" {
        Gait::Swim
    } else if state.contains("JUMP_START")
        || state.contains("JUMP_FLIGHT")
        || state.contains("TO_FALL")
        || state.contains("TO_SKYDIVE")
        || matches!(state, "S_FALL" | "S_SKYDIVE" | "S_PARACHUTE_DETACH_TO_FREEFALL")
    {
        Gait::Air
    } else {
        Gait::Idle
    };
    let rate = if state == "S_DASH" {
        // the sprint's pace in the clip's own root-motion speed
        match rig.clips.get(&clip).and_then(|l| l.natural_speed) {
            Some(s) => (hero.speed / s).clamp(0.5, 2.0),
            None => 1.0,
        }
    } else if state == "S_PARACHUTE" {
        0.0
    } else {
        rate
    };
    Some((Some(clip), gait, rate, blend))
}

/// Falls faster than these (m/s) land heavily, and roll when he is moving too. Fallback
/// rules only (without the foot machine's data): the graph's own rule for the heavy
/// landing is a fall timer past 1.5 s (`foot_machine::HEAVY_LANDING_FALL_SECONDS`).
const HEAVY_LANDING: f32 = 11.0;
const ROLL_LANDING: f32 = 17.0;

/// The base clip for his state (`None`: keep what plays, a landing playing out), the
/// gait the gun follows, the playback rate and the crossfade.
#[allow(clippy::too_many_arguments)]
fn pick_base(
    hero: &Hero,
    lin: &LinearVelocity,
    an: &Animator,
    rig: &Rig,
    class: Option<crate::poses::WeaponClass>,
    hook: Option<&GrappleRig>,
    seen: &mut HeroAnimMemory,
    native: Option<(&FootMachineData, &crate::foot_machine::FootDriver)>,
) -> (Option<Clip>, Gait, f32, f32) {
    let duration = |c: Clip| rig.duration(c);
    let has = |c: Clip| rig.has(c);
    // with the foot machine driving (clock running), the fallback landing rules below stay
    // out of it: the graph decides the landing clips
    let native = native.filter(|(_, d)| d.stepped());
    let landing = matches!(
        an.clip,
        Clip::Land
            | Clip::LandLight
            | Clip::LandHeavy
            | Clip::LandRoll
            | Clip::LandHard
            | Clip::ChuteLand
    );
    let on_foot = hero.in_vehicle.is_none() && hero.stunt.is_none();
    // the hook's own body states (`basestatemachine`: the reel, its impacts, the hang):
    // the clip bound to the state the graph is in (`grapple/anim.rs`)
    if on_foot
        && !hero.parachute
        && let Some(g) = hook
        && let Some(clip) = g.anim.body.clip()
        && has(clip)
    {
        let gait = if g.anim.body == crate::grapple::BodyState::ReelEndLand {
            Gait::Idle
        } else {
            Gait::Hang
        };
        return (Some(clip), gait, g.anim.body.speed(), g.anim.body.blend().max(0.02));
    }
    if native.is_none() && hero.grounded && hero.grapple.is_none() && on_foot {
        let left = duration(an.clip) - an.time;
        // a roll or a hard landing plays out; a light one gives way to running
        if landing
            && left > 0.05
            && (hero.speed < 1.0
                || matches!(an.clip, Clip::LandRoll | Clip::LandHeavy | Clip::LandHard))
        {
            return (None, Gait::Idle, an.rate, 0.1);
        }
        if !landing && matches!(an.clip, Clip::Fall | Clip::Jump | Clip::Dive | Clip::Reel) {
            let fall = seen.fall_speed;
            let clip = if an.clip == Clip::Dive || fall > ROLL_LANDING {
                if hero.speed > 3.0 && has(Clip::LandRoll) {
                    Clip::LandRoll
                } else {
                    Clip::LandHeavy
                }
            } else if fall > HEAVY_LANDING {
                Clip::LandHeavy
            } else if hero.speed > 2.0 {
                Clip::Land
            } else {
                Clip::LandLight
            };
            let clip = if has(clip) { clip } else { Clip::Land };
            return (Some(clip), Gait::Idle, 1.0, 0.08);
        }
        if an.clip == Clip::Chute {
            // `S_PARACHUTE`'s landings split on 8 m/s down and 8 m/s along
            let clip = if seen.fall_speed > 8.0 && has(Clip::LandHard) {
                Clip::LandHard
            } else if hero.speed > 8.0 && has(Clip::LandRoll) {
                Clip::LandRoll
            } else {
                Clip::ChuteLand
            };
            return (Some(clip), Gait::Idle, 1.0, 0.1);
        }
    }
    let speed_rate = |c: Clip, speed: f32| match rig.clips.get(&c).and_then(|l| l.natural_speed) {
        Some(s) => (speed / s).clamp(0.5, 2.0),
        None => 1.0,
    };
    if hero.in_vehicle.is_some() {
        return (Some(Clip::Idle), Gait::Seated, 1.0, FADE);
    }
    if hero.stunt.is_some() {
        // on a roof: crouch once the vehicle is going fast (`lin` is the roof's velocity)
        if let Some(c) = hero.stunt_clip.filter(|c| has(*c)) {
            let gait = if c == Clip::Hang {
                Gait::Hang
            } else {
                Gait::Idle
            };
            return (Some(c), gait, 1.0, FADE);
        }
        if lin.length() > 3.0 && has(Clip::Crouch) {
            return (Some(Clip::Crouch), Gait::Crouch, 1.0, 0.25);
        }
        return (Some(Clip::Idle), Gait::Idle, 1.0, 0.25);
    }
    // Free movement: the foot machine's state names the clip, blend and speed.
    if hero.grapple.is_none()
        && let Some((data, driver)) = native
        && let Some(p) = native_base(data, driver, hero, an, rig)
    {
        return p;
    }
    // The canopy keeps its pose while the grapple supplies a towing force.
    // Grapple state alone must not replace it with the wire-hanging pose.
    if hero.parachute {
        return (Some(Clip::Chute), Gait::Chute, 0.0, 0.5);
    }
    if hero.grapple.is_some() {
        // over a ledge: the bounce up out of the hang (`ANIM_REELED_IN_JUMP`)
        if hook.is_some_and(|g| g.climb > 0.0) && has(Clip::GrappleClimb) {
            return (Some(Clip::GrappleClimb), Gait::Hang, 1.0, 0.1);
        }
        // reeled in, flying at the hook (`ANIM_REEL_FLIGHT`); there, hanging from the
        // wire at a wall, a ledge or under an overhang (`ANIM_HANGED`: `grpl_hang`)
        let reeling = !hero.hanging
            && hook.is_none_or(|g| g.phase == GrapplePhase::Reel)
            && lin.length() > 4.0;
        let clip = if reeling && has(Clip::Reel) {
            Clip::Reel
        } else {
            Clip::Hang
        };
        return (Some(clip), Gait::Hang, 1.0, 0.2);
    }
    if hero.swimming {
        return if hero.speed > 0.3 {
            (
                Some(Clip::Swim),
                Gait::Swim,
                speed_rate(Clip::Swim, hero.speed).max(0.8),
                0.3,
            )
        } else {
            (Some(Clip::SwimIdle), Gait::Swim, 1.0, 0.3)
        };
    }
    // (a locomotion state keeps its ground pose over a one-frame loss of ground contact: the
    // graph's `ACT_UPDATE` fall edge needs open air under him to leave it)
    let grounded_like =
        hero.grounded || native.is_some_and(|(_, d)| is_locomotion_state(&d.state));
    if !grounded_like {
        let clip = if hero.air_time < 0.35 && lin.y > 0.5 {
            Clip::Jump
        } else if native.map_or_else(
            || crate::gameplay_camera::is_skydiving(hero, lin.y),
            |(_, d)| crate::hud::is_skydive_controller_state(&d.state),
        ) && has(Clip::Dive)
        {
            Clip::Dive
        } else {
            Clip::Fall
        };
        // the `.asb`'s blend-ins: fall 0.3 s, skydive 0.4
        let blend = match clip {
            Clip::Dive => 0.4,
            Clip::Fall => 0.3,
            _ => 0.15,
        };
        return (Some(clip), Gait::Air, 1.0, blend);
    }
    // on the ground: the clip by speed, and sideways when the legs strafe
    let speed = hero.speed;
    if speed <= 0.4 {
        return (Some(Clip::Idle), Gait::Idle, 1.0, NAV_BLEND);
    }
    let v = Vec3::new(lin.x, 0.0, lin.z);
    let rel = wrap_angle(yaw_toward(v) - hero.facing);
    let side = rel.abs() > 0.8 && rel.abs() < 2.35;
    let one_handed = class.is_some_and(|c| c.one_handed());
    let (clip, gait) = if speed < WALK_MAX {
        let c = match (side, rel > 0.0) {
            (true, true) => Clip::WalkStrafeLeft,
            (true, false) => Clip::WalkStrafeRight,
            _ if one_handed && has(Clip::Walk1H) => Clip::Walk1H,
            _ => Clip::Walk,
        };
        (c, Gait::Walk)
    } else if speed < SPRINT_MIN || side || native.is_some() {
        // (with the foot machine driving, the sprint is the `S_DASH` state, not a speed)
        let c = match (side, rel > 0.0) {
            (true, true) => Clip::RunStrafeLeft,
            (true, false) => Clip::RunStrafeRight,
            _ => Clip::Run,
        };
        (c, Gait::Run)
    } else {
        (Clip::Sprint, Gait::Sprint)
    };
    let clip = if has(clip) { clip } else { Clip::Run };
    // navigation 0.24 s; into a strafe 0.13
    let blend = if side { 0.13 } else { NAV_BLEND };
    (Some(clip), gait, speed_rate(clip, speed), blend)
}

/// Below this he walks (m/s); from this he sprints.
const WALK_MAX: f32 = 2.2;
const SPRINT_MIN: f32 = 5.6;
const FADE: f32 = 0.15;
/// `ANIM_UPRIGHT_BASIC_NAVIGATION`'s blend-in (s).
const NAV_BLEND: f32 = 0.24;

/// Turn the model, show the chute, place the hook and the wire.
#[allow(clippy::type_complexity)]
pub(crate) fn hero_visuals(
    arsenal: Option<Res<crate::combat::Arsenal>>,
    heroes: Query<(
        &Hero,
        &Transform,
        &Children,
        Option<&GrappleRig>,
        Option<&Animator>,
        Option<&crate::combat::Dead>,
        Option<&crate::ragdoll::PhysicsPose>,
        Option<&crate::ragdoll::PendingRagdoll>,
        Option<&crate::stunts::WireTilt>,
        Option<&crate::foot_machine::FootDriver>,
    )>,
    mut models: Query<
        (&mut Transform, &mut ModelVisualState),
        (With<HeroModel>, Without<Hero>, Without<Canopy>),
    >,
    mut canopies: Query<
        (&mut Transform, &mut crate::parachute::CanopyRig),
        (With<Canopy>, Without<Hero>, Without<HeroModel>),
    >,
    mut hooks: Query<
        (&mut Transform, &mut Visibility),
        (
            With<HookVisual>,
            Without<Hero>,
            Without<HeroModel>,
            Without<Canopy>,
            Without<WireVisual>,
        ),
    >,
    mut wires: Query<
        (&mut crate::grapple::Cable, &mut Visibility),
        (
            With<WireVisual>,
            Without<Hero>,
            Without<HeroModel>,
            Without<Canopy>,
            Without<HookVisual>,
        ),
    >,
) {
    let Ok((hero, tf, children, rig, an, dead, physics_pose, pending, wire_tilt, driver)) =
        heroes.single()
    else {
        return;
    };
    // the canopy is up while the foot machine is in one of the canopy states (the pulled
    // chord, the unfolding, the flight); without the machine, while the controller's flag is
    let canopy_up = match driver {
        Some(d) if d.stepped() => crate::foot_machine::is_chute_state(&d.state),
        _ => hero.parachute,
    };
    // a strafe is a side-on take: turn the model against the body's turn in it
    let body = an.map_or(0.0, |a| a.clip.body_yaw());
    for child in children.iter() {
        if let Ok((mut m, mut state)) = models.get_mut(child) {
            if physics_pose.is_some() || pending.is_some() {
                continue;
            }
            if hero.downed {
                let fallen = (dead.map_or(0.0, |d| d.0) / 0.5).clamp(0.0, 1.0);
                m.rotation = Quat::from_rotation_y(hero.facing)
                    * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2 * fallen);
                m.translation.y = -HEIGHT / 2.0 + 0.15 * fallen;
                continue;
            }
            let correction = wire_tilt.map_or(Quat::IDENTITY, |t| t.0);
            m.translation = correction * (Vec3::NEG_Y * (HEIGHT / 2.0));
            // the native body orientation is one full matrix about the character origin
            // (`parachute-body-orientation.md` section 1; "the mesh uses `character + 0x15c`" is
            // inferred, board b67acd7f): the canopy's `M` under the canopy, the freefall
            // model's matrix while diving, otherwise yaw only
            let matrix = if hero.canopy_live {
                Some(hero.canopy.matrix())
            } else if hero.skydiving {
                hero.skydive.map(|sd| sd.matrix)
            } else {
                None
            };
            if let Some(rows) = matrix {
                let q = Quat::from_mat3(&Mat3::from_cols(rows[0], rows[1], rows[2]));
                m.rotation = correction * q;
                state.wire_tilt = wire_tilt.is_some();
                continue;
            }
            let target = correction * Quat::from_rotation_y(hero.facing - body);
            m.rotation = if wire_tilt.is_some() || state.wire_tilt {
                // The correction already uses fixed-step exponential smoothing;
                // do not introduce an extra render-frame-dependent body-up lag.
                target
            } else {
                m.rotation.slerp(target, 0.35)
            };
            state.wire_tilt = wire_tilt.is_some();
        }
        if let Ok((mut c, mut rig)) = canopies.get_mut(child) {
            // visibility and clips belong to the canopy's own state machine
            // (`parachute::animate_canopy`, `parachute-deploy-collapse.md`)
            let state = driver.map_or("", |d| d.state.as_str());
            rig.command = crate::parachute::CanopyCommand {
                up: canopy_up,
                horizontal: state == "S_UNFOLD_PARACHUTE_HORIZONTAL",
                cut_away: state == "S_PARACHUTE_DETACH_TO_FREEFALL",
            };
            // the flight basis `Rz(a38)*Rx(a30)*Ry(a34)` (`0x005ab3f2`, JC2-8d8d): bank in
            // turns, nose up braking, nose down diving; before the flight model runs the
            // canopy is only yawed
            if hero.canopy_live {
                let o = crate::parachute::canopy_orientation(
                    &hero.canopy.matrix(),
                    Vec3::new(0.0, -HEIGHT / 2.0, 0.0),
                );
                c.rotation = o.rotation;
                c.translation = o.translation;
            } else {
                c.rotation = Quat::from_rotation_y(hero.facing);
            }
        }
    }
    // the canopy's hand handles and harness follow Rico's `AttachHandRight`, `AttachHandLeft`
    // and `Spine1` (native `0x00657010`, JC2-0fa7); `parachute::animate_canopy` applies them
    let body = models.iter().next().map(|(m, _)| *m);
    // which hands keep their handle: the weapon holder state (`Arsenal`), native section 6
    let holding = crate::parachute::hold_flags(arsenal.as_deref().map_or(
        crate::parachute::HoldInputs::default(),
        |a| {
            let g = a.guns.get(a.current);
            crate::parachute::HoldInputs {
                drawn: a.drawn && g.is_some(),
                pending: false,
                two_handed: g.is_some_and(|g| !g.class.one_handed()),
                reloading: g.is_some_and(|g| g.ammo.reloading.is_some()),
            }
        },
    ));
    for (c, mut rig) in &mut canopies {
        rig.attach = match (an, body) {
            (Some(an), Some(body)) if canopy_up => crate::parachute::attach_from_rico(
                &an.rig.skeleton,
                &an.local,
                &body,
                &c,
                holding,
            ),
            _ => None,
        };
    }
    // the hook (in flight, holding, coming home) and its wire from the gauntlet
    let shown = rig.filter(|r| r.phase != crate::grapple::Phase::Idle);
    for (mut h, mut vis) in &mut hooks {
        match shown {
            Some(r) => {
                *vis = Visibility::Inherited;
                h.translation = r.hook;
                h.rotation = r.hook_rot;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    for (mut w, mut vis) in &mut wires {
        match shown {
            Some(r) => {
                *vis = Visibility::Inherited;
                w.a = r.hand_or(tf.translation, hero.facing);
                w.b = r.hook;
                // the completed wire sags unless a pull is active (native, cable.rs); in
                // flight, coming home, reeling and pulling it is drawn as it is
                w.sag = r.phase == crate::grapple::Phase::Hang;
                w.hooks = false; // this wire's hook is `HookVisual` (a separate model)
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

type HeroCam<'w, 's> =
    Query<'w, 's, (&'static mut Transform, &'static mut FlyCamera), Without<Hero>>;

type HeroCamParts<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Hero,
        &'static Transform,
        Option<&'static crate::foot_machine::FootDriver>,
        Option<&'static LinearVelocity>,
    ),
>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn hero_camera(
    mode: Res<CameraMode>,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut orbit: ResMut<OrbitCamera>,
    heroes: HeroCamParts,
    mut cams: HeroCam,
    gameplay: Option<Res<crate::gameplay_camera::GameplayCamera>>,
    mut rig: ResMut<crate::gameplay_camera::ChuteRig>,
    mut foot: ResMut<crate::gameplay_camera::FootRig>,
) {
    if *mode != CameraMode::Hero {
        return;
    }
    let Ok((entity, hero, tf, driver, velocity)) = heroes.single() else {
        return;
    };
    let Ok((mut cam_tf, mut fly)) = cams.single_mut() else {
        return;
    };
    // the chute and skydive cameras (ids 1 and 2) place themselves by the native model
    // (`research/formats/parachute-camera-placement.md`); every other state keeps the orbit
    let chute = gameplay.as_deref().and_then(|g| g.chute_placement());
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
    if let Some((id, kind, placement)) = chute.filter(|_| !hero.downed) {
        if rig.built_for != Some(id) {
            rig.reset();
            rig.built_for = Some(id);
        }
        // camera node id 2 is the character's whole world matrix at the feet (research
        // addendum A, `character + 0x15c`): the canopy basis while the canopy flies, the
        // skydive pose while falling, otherwise the plain heading
        let node = tf.translation - Vec3::Y * (HEIGHT / 2.0);
        let rows_rot = |m: &crate::parachute_flight::Rows| {
            Quat::from_mat3(&Mat3::from_cols(m[0], m[1], m[2]))
        };
        let node_rot = match (hero.canopy_live, hero.skydive) {
            (true, _) => rows_rot(&hero.canopy.matrix()),
            (false, Some(sd)) if kind == crate::gameplay_camera::ChuteKind::Freefall => {
                rows_rot(&sd.matrix)
            }
            _ => Quat::from_rotation_y(hero.facing),
        };
        // `|S+4| > 0.9`: Rico's right axis against the camera's (`0077fc80`)
        let s4 = crate::parachute_flight::camera_angle(
            Quat::from_rotation_y(hero.facing) * Vec3::X,
            crate::gameplay_camera::camera_right(&cam_tf),
        );
        let dt = time.delta_secs();
        let pose = rig.step(
            kind,
            placement,
            &crate::gameplay_camera::ChuteFrame {
                node_pos: node,
                node_rot,
                orbit: (orbit.yaw, orbit.pitch),
                turn_gate: s4.abs() > crate::parachute_flight::CAMERA_TURN_THRESHOLD,
                tilt_f: hero.skydive.map_or(0.5, |sd| 1.0 - sd.p),
                // w34 only exists while the canopy flies (`D+0x40`)
                yaw_rate: if hero.canopy_live { hero.canopy.yaw_rate() } else { 0.0 },
                dt,
            },
        );
        let (yaw, pitch) = rig.orbit();
        orbit.yaw = yaw;
        orbit.pitch = pitch;
        // the free fraction of the target-to-camera line (the project's ray cast stands in
        // for the native shape cast `007b9fe0`, Gap 2), then the record's collision filter
        let to_cam = pose.camera - pose.target;
        let len = to_cam.length();
        let hit = match Dir3::new(to_cam) {
            Ok(d) => spatial
                .cast_ray(pose.target, d, len, true, &filter)
                .map_or(1.0, |h| ((h.distance - 0.3).max(0.6).min(len) / len).clamp(0.0, 1.0)),
            Err(_) => 1.0,
        };
        let (cam, target) = rig.collide(
            crate::gameplay_camera::ChuteCollision::from_class(placement.collision),
            &pose,
            hit,
            dt,
        );
        orbit.distance = (cam - target).length();
        cam_tf.translation = cam;
        cam_tf.look_at(target, Vec3::Y);
        fly.yaw = pose.yaw;
        fly.pitch = pose.pitch;
        return;
    }
    rig.reset();
    // the on-foot camera (id `OnFoot`) places itself by its native record
    // (`research/formats/onfoot-camera-placement.md`); other states keep the orbit below
    if let Some(placement) = gameplay
        .as_deref()
        .and_then(|g| g.foot_placement())
        .filter(|_| !hero.downed)
    {
        let dt = time.delta_secs();
        let [pmin, pmax] = placement.pitch_limits.unwrap_or(crate::gameplay_camera::FOOT_PITCH_LIMITS);
        orbit.pitch = orbit.pitch.clamp(pmin, pmax);
        let dashing = driver.is_some_and(|d| crate::gameplay_camera::is_dash_state(&d.state));
        let speed_sq = velocity.map_or(0.0, |v| v.0.length_squared());
        let run = foot.step(crate::gameplay_camera::run_target(dashing, speed_sq), dt);
        // camera node id 2 is the character matrix at the feet (research addendum A)
        let node = tf.translation - Vec3::Y * (HEIGHT / 2.0);
        let (cam, target) = crate::gameplay_camera::foot_pose(
            placement,
            node,
            Quat::from_rotation_y(hero.facing),
            (orbit.yaw, orbit.pitch),
            run,
        );
        // pull the camera in when something stands between it and the target. The native
        // class (`CCameraCollision`, record `+0xbc` = 4) is not decoded: the project's ray cast.
        let to_cam = cam - target;
        let len = to_cam.length();
        let dist = match Dir3::new(to_cam) {
            Ok(d) => spatial
                .cast_ray(target, d, len, true, &filter)
                .map_or(len, |h| (h.distance - 0.3).max(0.6).min(len)),
            Err(_) => len,
        };
        orbit.distance = dist;
        cam_tf.translation = target + to_cam.normalize_or_zero() * dist;
        cam_tf.look_at(target, Vec3::Y);
        fly.yaw = orbit.yaw;
        fly.pitch = orbit.pitch;
        return;
    }
    foot.run = 0.0;
    let wanted = CAMERA_DISTANCE;
    orbit.distance += (wanted - orbit.distance) * (1.0 - (-3.0 * time.delta_secs()).exp());
    let target = tf.translation + Vec3::Y * 0.6;
    let rot = Quat::from_euler(EulerRot::YXZ, orbit.yaw, orbit.pitch, 0.0);
    let back = rot * Vec3::Z;
    // pull the camera in when something stands between it and Rico
    let dist = match Dir3::new(back) {
        Ok(d) => spatial
            .cast_ray(target, d, orbit.distance, true, &filter)
            .map(|h| (h.distance - 0.3).max(0.6))
            .unwrap_or(orbit.distance),
        Err(_) => orbit.distance,
    };
    cam_tf.translation = target + back * dist;
    cam_tf.look_at(target, Vec3::Y);
    fly.yaw = orbit.yaw;
    fly.pitch = orbit.pitch;
}

#[allow(clippy::type_complexity)]
fn hero_hud(
    mode: Res<CameraMode>,
    lock: Res<LockOn>,
    heroes: Query<(
        &Hero,
        Option<&crate::stunts::OnStunt>,
        Option<&crate::stunts::CurrentStuntSpot>,
        Has<crate::stunts::StuntTransfer>,
    )>,
    vehicles: Query<&crate::stunts::StuntSpots>,
    controls: Res<Controls>,
    mut q: Query<&mut Text, With<HeroHud>>,
) {
    let Ok((hero, on, current, transferring)) = heroes.single() else {
        return;
    };
    let transfer = match (on, current) {
        _ if transferring => "   transferring".to_string(),
        (Some(on), Some(current)) if !hero.downed => vehicles
            .get(on.vehicle)
            .map(|spots| crate::stunts::transfer_hint(on, current, spots))
            .unwrap_or_default(),
        _ => String::new(),
    };
    // the prompt shows the keys the actions are really on
    let move_keys = [MOVE_FORWARD, MOVE_LEFT, MOVE_BACK, MOVE_RIGHT]
        .map(|a| controls.label(a))
        .join("/");
    for mut t in &mut q {
        if *mode == CameraMode::Drive || hero.in_vehicle.is_some() {
            **t = String::new();
            continue;
        }
        let state = if hero.hanging {
            "hanging"
        } else if hero.grapple.is_some() {
            "on the wire"
        } else if hero.swimming {
            "swimming"
        } else if hero.parachute {
            "parachute"
        } else if hero.grounded {
            "on foot"
        } else {
            "airborne"
        };
        let aim = match lock.target {
            Some(l) if l.kind != crate::grapple::TargetKind::Surface => format!(
                "   lock {:?} {:.0} m{}{:.1} deg",
                l.kind,
                l.distance,
                if l.magnet { " magnet " } else { " " },
                l.angle.to_degrees()
            ),
            _ => String::new(),
        };
        **t = format!(
            "Rico   {state}   {:4.1} m/s{aim}{transfer}{}\n\
             {} move  {} sprint  Space jump / parachute  F grapple  E vehicle  right-drag or Tab look  V {}",
            hero.speed,
            if hero.air_time > 0.3 && !hero.parachute && hero.grapple.is_none() {
                format!("   falling {:.1} s", hero.air_time)
            } else {
                String::new()
            },
            move_keys,
            controls.label(SPRINT_ACTION),
            if *mode == CameraMode::Hero {
                "free camera"
            } else {
                "back to Rico"
            },
        );
    }
}

#[cfg(test)]
mod tests {
    /// JC2-270: the context's jump law with the engine's 9.81 modelled: apex 2.00 m, no
    /// terminal velocity, horizontal 5 % per update both ways, Jumping = desired velocity.
    #[test]
    fn air_context_laws() {
        let dt = 1.0 / 60.0;
        let engine = |mut v: Vec3, jumping: bool| {
            v.y += if jumping { 9.81 * dt } else { 0.0 }; // the caller's cancel
            v.y -= 9.81 * dt; // the physics step
            v
        };
        let (mut v, mut y, mut apex, mut left) = (Vec3::ZERO, 0.0_f32, 0.0_f32, JUMP_CLIMB_SECONDS);
        for _ in 0..240 {
            let jumping = left > 0.0;
            left -= dt;
            v = engine(air_velocity_step(v, Vec3::ZERO, jumping, dt), jumping);
            y += v.y * dt;
            apex = apex.max(y);
        }
        assert!((apex - 2.0).abs() < 0.08, "apex {apex}");
        // free fall: -21 t, no cap (3 s = 63 m/s)
        let mut f = Vec3::ZERO;
        for _ in 0..180 {
            f = engine(air_velocity_step(f, Vec3::ZERO, false, dt), false);
        }
        assert!((f.y + 63.0).abs() < 0.5, "{}", f.y);
        // horizontal: 5 % per update (here 30 Hz), braking as well as speeding up
        let mut h = Vec3::new(10.0, 0.0, 0.0);
        for _ in 0..40 {
            h = air_velocity_step(h, Vec3::ZERO, false, 1.0 / STICK_HZ);
        }
        assert!((h.x - 10.0 * 0.95_f32.powi(40)).abs() < 0.05, "{}", h.x);
        // Jumping: the desired velocity outright
        let j = air_velocity_step(Vec3::new(9.0, 3.0, 0.0), Vec3::new(5.0, 0.0, 1.0), true, dt);
        assert_eq!((j.x, j.z), (5.0, 1.0));
    }

    use super::*;
    use crate::archive_source::JcAssetReader;
    use crate::coords::MODEL_FORWARD;
    use crate::physics::terrain_collider;
    use crate::rbm_loader::RbmLoader;
    use bevy::asset::AssetApp;
    use bevy::asset::io::AssetSourceBuilder;
    use jc2_formats::GameFs;
    use jc2_formats::terrain::TerrainTile;
    use std::sync::Arc;

    fn flat_tile(raw: u16) -> TerrainTile {
        let mut d = Vec::new();
        d.extend_from_slice(&12u32.to_le_bytes());
        d.extend_from_slice(&123u32.to_le_bytes());
        for _ in 0..132 * 132 {
            d.extend_from_slice(&raw.to_le_bytes());
        }
        d.extend(std::iter::repeat_n(0u8, 4 * 128 * 128 * 2));
        TerrainTile::parse(&d).unwrap()
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }
    fn release(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(key);
    }
    fn step(app: &mut App, frames: usize) {
        for _ in 0..frames {
            app.update();
            // what the input plugin would do between frames
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.bypass_change_detection().clear();
        }
    }
    fn hero(app: &mut App) -> (Hero, Vec3, Vec3) {
        let mut q = app
            .world_mut()
            .query::<(&Hero, &Transform, &LinearVelocity)>();
        let (h, t, v) = q.single(app.world()).unwrap();
        (h.clone(), t.translation, v.0)
    }

    /// Headless: standing still on a 20° slope he stays put (he slid down it: gravity
    /// pressed the frictionless capsule into the slope). Needs Rico's model.
    #[test]
    fn rico_stands_on_a_slope() {
        let Some((mut app, _, spawn)) =
            crate::fleet::testing::app(&["mc01_lod1-rico.rbm"], |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins(CharacterPlugin);
            })
        else {
            return;
        };
        // a slab tilted 20° about x through the start, its top a few metres up
        let tilt = Quat::from_rotation_x(0.35);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(60.0, 2.0, 60.0),
            Transform::from_translation(spawn + Vec3::Y * 3.0 - tilt * Vec3::Y).with_rotation(tilt),
        ));
        app.world_mut().spawn((
            Transform::from_translation(spawn + Vec3::new(0.0, 4.0, 8.0)),
            FlyCamera {
                yaw: 0.0,
                pitch: 0.0,
                speed: 1.0,
            },
        ));
        // (his model loads on other threads: wait for him by the wall clock)
        crate::fleet::testing::wait_for_hero(&mut app);
        // drop him onto the slab
        let mut q = app.world_mut().query_filtered::<Entity, With<Hero>>();
        let e = q.single(app.world()).unwrap();
        let above = spawn + Vec3::Y * 5.0;
        app.world_mut().entity_mut(e).insert((
            Transform::from_translation(above),
            Position(above),
            LinearVelocity::ZERO,
        ));
        step(&mut app, 120);
        let (h, start, _) = hero(&mut app);
        assert!(h.grounded, "on the slope");
        assert!(
            h.ground_normal.y < 0.97,
            "the slab is tilted: {:?}",
            h.ground_normal
        );
        step(&mut app, 64 * 3);
        let (_, end, _) = hero(&mut app);
        let moved = (end - start).length();
        assert!(moved < 0.1, "slid {moved:.2} m");
    }

    /// Headless: Rico lands on a flat tile, walks when W is held, jumps, flies the
    /// parachute at its glide speed from 80 m up, and gets pulled along the wire.
    /// Needs JC2_TEST_DIR with `mc01_lod1-rico.rbm` and `general.blz` reachable.
    #[test]
    fn rico_walks_jumps_chutes_and_grapples() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        if !fs.contains("mc01_lod1-rico.rbm") {
            return;
        }
        let tile = flat_tile(32000);
        let ground = tile.cell_height(0, 0);
        let (collider, tf) = terrain_collider(&tile, -512.0, -9216.0);
        let reader_fs = fs.clone();
        let mut app = App::new();
        app.register_asset_source(
            "jc2",
            AssetSourceBuilder::new(move || Box::new(JcAssetReader::new(reader_fs.clone()))),
        );
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugins::default(),
        ))
        .init_asset::<Image>()
        // Headless fixtures still decode required retail BC colour images.
        .register_asset_loader(bevy::image::ImageLoader::new(
            bevy::image::CompressedImageFormats::BC,
        ))
        .init_asset::<StandardMaterial>()
        .init_asset::<RbmAsset>()
        .init_asset::<crate::foliage_alpha::FoliageMaterial>()
        .init_asset::<crate::general_material::GeneralMaterial>()
        .init_asset::<crate::hemisphere_material::HemisphereMaterial>()
        .init_asset::<crate::skinned_general_material::SkinnedGeneralMaterial>()
        .init_asset_loader::<RbmLoader>()
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<CameraMode>()
        .insert_resource(Game { fs })
        .insert_resource(SpawnPoint(Vec3::new(-256.0, ground, -8960.0)))
        .add_plugins(CharacterPlugin);
        app.world_mut()
            .spawn((RigidBody::Static, collider, tf, Name::new("ground")));
        // two tiles east: a sea floor 66 m under the surface
        let (floor, floor_tf) = terrain_collider(&flat_tile(4000), 512.0, -9216.0);
        app.world_mut()
            .spawn((RigidBody::Static, floor, floor_tf, Name::new("sea floor")));
        app.world_mut().spawn((
            Transform::from_xyz(-250.0, ground + 2.0, -8955.0),
            FlyCamera {
                yaw: 0.0,
                pitch: 0.0,
                speed: 1.0,
            },
        ));
        app.finish();

        // spawn and settle
        // (his model loads on other threads: wait for him by the wall clock)
        crate::fleet::testing::wait_for_hero(&mut app);
        let (h, p, _) = hero(&mut app);
        assert!(h.grounded, "rico never landed: {h:?} at {p:?}");
        step(&mut app, 30);
        let (h, p, _) = hero(&mut app);
        assert!(h.grounded, "rico didn't stay down: {h:?} at {p:?}");
        assert!(
            (p.y - (ground + HEIGHT / 2.0)).abs() < 0.08,
            "standing at {} on ground {ground}",
            p.y
        );

        // the rig: 84 joints under the model, idling
        let rigged = app.world().get_resource::<Rig>().is_some();
        let fs = &app.world().resource::<Game>().fs;
        assert_eq!(
            rigged,
            fs.contains_any("biped.bsk") && fs.contains_any("std_idle.ban")
        );
        if rigged {
            let mut q = app.world_mut().query::<&Animator>();
            let an = q.single(app.world()).expect("animator on the hero");
            assert_eq!(an.joints.len(), 84);
            assert_eq!(an.clip, Clip::Idle);
            let mut joints = app.world_mut().query::<(&crate::rig::Joint, &Transform)>();
            // plus the 17 joints of the canopy's `parachute.bsk` when the canopy is rigged
            let chute_joints = app
                .world()
                .get_resource::<ChuteRig>()
                .map_or(0, |c| c.skeleton.bones.len());
            assert_eq!(joints.iter(app.world()).count(), 84 + chute_joints);
        }

        // walk: W for two seconds moves him along the camera's forward (−z: it starts behind
        // him, looking the way he faces)
        press(&mut app, KeyCode::KeyW);
        step(&mut app, 120);
        let (h, p2, _) = hero(&mut app);
        let moved = p2 - p;
        assert!(moved.z < -5.0, "walked {moved:?}");
        if rigged {
            let mut q = app.world_mut().query::<&Animator>();
            let an = q.single(app.world()).unwrap();
            assert_eq!(an.clip, Clip::Run);
            // the legs swing: the left thigh has left its reference pose
            let mut joints = app.world_mut().query::<(&crate::rig::Joint, &Transform)>();
            let thigh = joints
                .iter(app.world())
                .find(|(j, _)| j.0 == 4)
                .map(|(_, t)| *t)
                .unwrap();
            assert!(
                thigh.rotation.angle_between(Quat::IDENTITY) > 0.05,
                "thigh never moved: {:?}",
                thigh.rotation
            );
        }
        assert!((h.speed - 5.51).abs() < 0.3, "speed {}", h.speed);
        release(&mut app, KeyCode::KeyW);
        step(&mut app, 70);
        let (h, _, _) = hero(&mut app);
        assert!(h.speed < 0.3, "still moving at {}", h.speed);

        // jump
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        step(&mut app, 24);
        let (h, p3, v) = hero(&mut app);
        assert!(
            !h.grounded && p3.y > p2.y + 0.3 && v.y > 0.0,
            "jump {h:?} {v:?}"
        );
        step(&mut app, 100);
        let (h, _, _) = hero(&mut app);
        assert!(h.grounded, "didn't land after the jump: {h:?}");

        // parachute from 80 m up
        {
            let mut q = app
                .world_mut()
                .query::<(&mut Transform, &mut Position, &mut LinearVelocity, &Hero)>();
            let (mut t, mut pos, mut v, _) = q.single_mut(app.world_mut()).unwrap();
            t.translation.y = ground + 200.0;
            pos.0 = t.translation;
            v.0 = Vec3::ZERO;
        }
        // (the chord is taken only from the skydive: a press during the fall's 0.7 s lead-in
        // is lost, as natively, so let him settle into it first)
        step(&mut app, 60);
        let (h, _, _) = hero(&mut app);
        assert!(!h.grounded, "should be falling");
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        step(&mut app, 900);
        let (h, p4, v) = hero(&mut app);
        assert!(h.parachute, "chute not open: {h:?}");
        // native neutral glide (brief section 5): 9.60 m/s forward, 1.69 m/s sink
        assert!((v.y + 1.69).abs() < 0.3, "descent {} m/s (want -1.69)", v.y);
        let glide = Vec3::new(v.x, 0.0, v.z).length();
        assert!((glide - 9.60).abs() < 0.5, "glide {glide} m/s");
        assert!(p4.y > ground + 30.0, "too low already: {}", p4.y);
        // the chute closes on landing (circling with A: the tile under him is finite)
        press(&mut app, KeyCode::KeyA);
        let orbit_yaw = app.world().resource::<OrbitCamera>().yaw;
        for _ in 0..8000 {
            step(&mut app, 1);
            let (h, _, _) = hero(&mut app);
            // the camera follows the canopy (else the camera-steered turn, `S+4`, counters A)
            app.world_mut().resource_mut::<OrbitCamera>().yaw = h.facing;
            if h.grounded {
                break;
            }
        }
        release(&mut app, KeyCode::KeyA);
        app.world_mut().resource_mut::<OrbitCamera>().yaw = orbit_yaw;
        let (h, _, _) = hero(&mut app);
        assert!(h.grounded && !h.parachute, "landing: {h:?}");
        step(&mut app, 60);
        let (h, _, _) = hero(&mut app);
        assert!(h.speed < 0.3, "still sliding at {}", h.speed);

        // the wire: an anchor 30 m away and 12 m up pulls him off the ground
        let anchor;
        {
            let mut q = app.world_mut().query::<(&mut Hero, &Transform)>();
            let (mut h, t) = q.single_mut(app.world_mut()).unwrap();
            anchor = t.translation + Vec3::new(0.0, 12.0, -30.0);
            h.grapple = Some(Grapple {
                anchor,
                normal: Vec3::Y,
                age: 0.0,
            });
        }
        let (_, p5, _) = hero(&mut app);
        // (the native ramp 11 -> 40 m/s covers the 32 m in about 1 s, so look half way)
        step(&mut app, 30);
        let (h, p6, v) = hero(&mut app);
        let before = p5.distance(anchor);
        let after = p6.distance(anchor);
        assert!(before - after > 6.0, "pulled {before} -> {after}");
        assert!(!h.grounded && v.length() > 8.0, "on the wire: {h:?} {v:?}");
        for _ in 0..240 {
            step(&mut app, 1);
            let (h, _, _) = hero(&mut app);
            if h.grapple.is_none() {
                break;
            }
        }
        let (h, p7, _) = hero(&mut app);
        assert!(h.grapple.is_none(), "wire never released");
        assert!(
            p7.distance(anchor) < 6.0,
            "let go {} m from the anchor",
            p7.distance(anchor)
        );
        step(&mut app, 120);

        // F fires at what the camera looks at: a box 20 m ahead of him (the camera sits
        // behind him, so the ray passes his capsule first)
        let (_, p8, _) = hero(&mut app);
        let box_pos = p8 + Vec3::new(0.0, 0.5, 20.0);
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(2.0, 3.0, 2.0),
            Transform::from_translation(box_pos),
            Name::new("crate"),
        ));
        step(&mut app, 2);
        {
            let mut cams = app
                .world_mut()
                .query_filtered::<&mut Transform, With<FlyCamera>>();
            let mut cam = cams.single_mut(app.world_mut()).unwrap();
            cam.translation = p8 + Vec3::new(0.0, 1.0, -4.0);
            cam.look_at(box_pos, Vec3::Y);
        }
        app.world_mut()
            .resource_mut::<CameraMode>()
            .set_if_neq(CameraMode::Hero);
        press(&mut app, KeyCode::KeyF);
        step(&mut app, 1);
        release(&mut app, KeyCode::KeyF);
        // the hook flies out first: 20 m at 120 m/s
        step(&mut app, 24);
        let (h, _, _) = hero(&mut app);
        let g = h.grapple.expect("the hook should have hit the crate");
        assert!(
            (g.anchor - box_pos).abs().max_element() < 2.5,
            "hooked {:?}, crate at {box_pos:?}",
            g.anchor
        );
        // dropped into the sea he floats and paddles
        release(&mut app, KeyCode::KeyW);
        {
            let mut q = app.world_mut().query::<(
                &mut Transform,
                &mut Position,
                &mut LinearVelocity,
                &mut Hero,
            )>();
            let (mut t, mut pos, mut v, mut h) = q.single_mut(app.world_mut()).unwrap();
            t.translation = Vec3::new(768.0, crate::world::SEA_LEVEL + 3.0, -8960.0);
            pos.0 = t.translation;
            v.0 = Vec3::ZERO;
            h.grapple = None;
        }
        step(&mut app, 240);
        let (h, p9, v) = hero(&mut app);
        assert!(h.swimming, "not swimming: {h:?} at {p9:?}");
        assert!(
            (p9.y - (crate::world::SEA_LEVEL - SWIM_DEPTH)).abs() < 0.3 && v.y.abs() < 0.5,
            "floating at {} ({:?})",
            p9.y,
            v
        );
        press(&mut app, KeyCode::KeyW);
        step(&mut app, 120);
        let (h, p10, _) = hero(&mut app);
        assert!(
            h.swimming && p9.z - p10.z > 2.0,
            "paddled {} m: {h:?}",
            p9.z - p10.z
        );
        if rigged {
            let mut q = app.world_mut().query::<&Animator>();
            assert_eq!(q.single(app.world()).unwrap().clip, Clip::Swim);
        }
        eprintln!(
            "rico: walked, jumped, flew the chute, rode the wire to {anchor:?}, hooked the crate and swam"
        );
    }

    #[test]
    fn he_faces_the_hook() {
        // the model looks down −z at yaw 0: a hook ahead (−z) means no turn, to the side
        // (+x) a quarter turn, behind him a half turn
        let face = |dir: Vec3| Quat::from_rotation_y(heading(dir, 0.0)) * MODEL_FORWARD;
        assert!(face(Vec3::new(0.0, 0.3, -1.0)).distance(Vec3::NEG_Z) < 1e-5);
        assert!(face(Vec3::new(5.0, 2.0, 0.0)).distance(Vec3::X) < 1e-5);
        assert!(face(Vec3::new(0.0, -1.0, 3.0)).distance(Vec3::Z) < 1e-5);
        // straight up: keep the way he was facing
        assert_eq!(heading(Vec3::Y * 40.0, 1.25), 1.25);
        // and the wire pulls him the way the heading says
        let dir = Vec3::new(-0.6, 0.2, 0.7).normalize();
        let fwd = Quat::from_rotation_y(heading(dir, 0.0)) * MODEL_FORWARD;

        assert!(fwd.dot(Vec3::new(dir.x, 0.0, dir.z).normalize()) > 0.9999);
    }

    /// Headless: a slab 7 m over the spawn point (a base's floor on top of the terrain
    /// height he used to be dropped at): he ends up standing on top of it, not under it.
    #[test]
    fn rico_is_placed_on_top_of_a_slab_over_the_spawn() {
        let Some((mut app, _, spawn)) =
            crate::fleet::testing::app(&["mc01_lod1-rico.rbm"], |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins(CharacterPlugin);
            })
        else {
            return;
        };
        let top = spawn.y + 7.0;
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(40.0, 1.0, 40.0),
            Transform::from_translation(Vec3::new(spawn.x, top - 0.5, spawn.z)),
        ));
        app.world_mut().spawn((
            Transform::from_translation(spawn + Vec3::new(0.0, 4.0, 8.0)),
            FlyCamera {
                yaw: 0.0,
                pitch: 0.0,
                speed: 1.0,
            },
        ));
        crate::fleet::testing::wait_for_hero(&mut app);
        step(&mut app, 30);
        let (h, p, _) = hero(&mut app);
        assert!(h.grounded, "not standing: {h:?} at {p:?}");
        assert!(
            (p.y - (top + HEIGHT / 2.0)).abs() < 0.1,
            "standing at {} but the slab's top is {top}",
            p.y
        );
        let mut q = app.world_mut().query::<&Settling>();
        assert!(q.iter(app.world()).next().is_none(), "still settling");
    }

    #[test]
    fn the_legs_face_the_walk_or_strafe_side_on_to_the_aim() {
        let ahead = Vec3::NEG_Z;
        assert_eq!(ground_facing(0.3, Vec3::ZERO, None), 0.3);
        assert!((ground_facing(0.0, Vec3::X, None) - yaw_toward(Vec3::X)).abs() < 1e-6);
        // standing, aiming: he turns to the aim
        assert_eq!(ground_facing(0.0, Vec3::ZERO, Some(1.0)), 1.0);
        // running ahead, aiming 30° off: the legs keep running ahead, the spine turns
        assert!(ground_facing(0.0, ahead, Some(0.5)).abs() < 1e-6);
        // aiming 120° to his left: side-on, strafing
        let f = ground_facing(0.0, ahead, Some(2.1));
        assert!((f - std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{f}");
    }

    /// The locomotion clip follows the speed and plays at the speed over the clip's
    /// root-motion speed, so the feet do not slide. Needs JC2_TEST_DIR.
    #[test]
    fn grapple_towing_keeps_the_parachute_pose() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = jc2_formats::GameFs::open(dir).unwrap();
        let rig = Rig::load(&fs, "biped.bsk").unwrap();
        let an = Animator::new(
            rig.0.clone(),
            vec![Entity::PLACEHOLDER; rig.skeleton.bones.len()],
        );
        let hero = Hero {
            parachute: true,
            grapple: Some(Grapple {
                anchor: Vec3::new(0.0, 40.0, -60.0),
                normal: Vec3::Y,
                age: 1.0,
            }),
            ..default()
        };
        let mut hook = GrappleRig::default();
        hook.phase = GrapplePhase::Reel;
        let mut mem = HeroAnimMemory::default();
        let (clip, gait, rate, _) = pick_base(
            &hero,
            &LinearVelocity(Vec3::NEG_Z * 30.0),
            &an,
            &rig,
            None,
            Some(&hook),
            &mut mem,
            None,
        );
        assert_eq!(clip, Some(Clip::Chute));
        assert_eq!(gait, Gait::Chute);
        assert_eq!(rate, 0.0);
    }

    #[test]
    fn locomotion_plays_at_the_speed_of_its_root_motion() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = jc2_formats::GameFs::open(dir).unwrap();
        let Some(rig) = Rig::load(&fs, "biped.bsk") else {
            return;
        };
        let an = Animator::new(
            rig.0.clone(),
            vec![Entity::PLACEHOLDER; rig.skeleton.bones.len()],
        );
        let mut mem = HeroAnimMemory::default();
        for (speed, want) in [(1.5, Clip::Walk), (4.5, Clip::Run), (7.0, Clip::Sprint)] {
            let hero = Hero {
                grounded: true,
                speed,
                ..default()
            };
            let lin = LinearVelocity(Vec3::new(0.0, 0.0, -speed));
            let (clip, _, rate, _) = pick_base(&hero, &lin, &an, &rig, None, None, &mut mem, None);
            assert_eq!(clip, Some(want));
            let natural = rig.clips[&want].anim.root_speed().unwrap();
            assert!(
                (rate - speed / natural).abs() < 1e-4,
                "{want:?} at {speed} m/s plays at {rate}, its root motion is {natural} m/s"
            );
        }
        // sideways to his left while facing ahead: the strafe
        let hero = Hero {
            grounded: true,
            speed: 4.5,
            ..default()
        };
        let lin = LinearVelocity(Vec3::new(-4.5, 0.0, 0.0));
        let (clip, ..) = pick_base(&hero, &lin, &an, &rig, None, None, &mut mem, None);
        assert_eq!(clip, Some(Clip::RunStrafeLeft));
    }

    // ---- headless animation-state traces (JC2-238): one scenario per foot-machine path ----

    use crate::foot_machine::FootDriver;

    /// Rico on the flat test tile, standing, with the foot machine's transition log on.
    fn foot_app() -> Option<(App, f32)> {
        let (mut app, ground, spawn) =
            crate::fleet::testing::app(&["mc01_lod1-rico.rbm", "basestatemachine.afsm"], |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins(CharacterPlugin);
            })?;
        app.world_mut().spawn((
            Transform::from_translation(spawn + Vec3::new(0.0, 4.0, 8.0)),
            FlyCamera {
                yaw: 0.0,
                pitch: 0.0,
                speed: 1.0,
            },
        ));
        // a sea floor east of the tile for the swimming scenario
        let (floor, floor_tf) = terrain_collider(&flat_tile(4000), 512.0, -9216.0);
        app.world_mut()
            .spawn((RigidBody::Static, floor, floor_tf, Name::new("sea floor")));
        crate::fleet::testing::wait_for_hero(&mut app);
        let mut q = app.world_mut().query::<&mut FootDriver>();
        q.single_mut(app.world_mut()).unwrap().trace = Some(Vec::new());
        Some((app, ground))
    }

    fn snapshot(app: &mut App) -> (Hero, String, Clip, Vec3, Vec3) {
        let mut q = app.world_mut().query::<(
            &Hero,
            &FootDriver,
            &Animator,
            &Transform,
            &LinearVelocity,
        )>();
        let (h, d, a, t, v) = q.single(app.world()).unwrap();
        (h.clone(), d.state.clone(), a.clip, t.translation, v.0)
    }

    /// Run `frames`, logging every change of the machine's state or of the clip.
    fn follow(app: &mut App, label: &str, frames: usize, t0: &mut usize) -> Vec<(String, Clip)> {
        let mut rows: Vec<(String, Clip)> = Vec::new();
        let mut last = None;
        for _ in 0..frames {
            step(app, 1);
            *t0 += 1;
            let (h, state, clip, p, v) = snapshot(app);
            let key = (state.clone(), clip);
            if last.as_ref() != Some(&key) {
                let clip_name = format!("{clip:?}");
                eprintln!(
                    "[{label}] {:6.2}s {state:<34} {clip_name:<20} grounded={} y={:+7.2} v=({:+5.1},{:+6.1},{:+5.1}){}",
                    *t0 as f32 / 64.0,
                    h.grounded,
                    p.y,
                    v.x,
                    v.y,
                    v.z,
                    if h.move_lock { " [move lock]" } else { "" },
                );
                rows.push(key.clone());
                last = Some(key);
            }
        }
        rows
    }

    /// The graph conditions the scenario met that the evaluator cannot decide (each is a
    /// Gap in `research/formats/rico-controller-next.md` when it sits on a path he takes).
    fn unsupported(app: &mut App, label: &str) -> Vec<String> {
        let mut q = app.world_mut().query::<&FootDriver>();
        let u = q.single(app.world()).unwrap().unsupported.clone();
        eprintln!("[{label}] conditions the evaluator could not decide: {u:?}");
        u
    }

    fn states(rows: &[(String, Clip)]) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for (s, _) in rows {
            if out.last() != Some(&s.as_str()) {
                out.push(s);
            }
        }
        out
    }

    /// `want` appears in `got` in this order (other states may sit between).
    fn in_order(got: &[&str], want: &[&str]) -> bool {
        let mut it = got.iter();
        want.iter().all(|w| it.any(|g| g == w))
    }

    fn clip_in(rows: &[(String, Clip)], state: &str) -> Option<Clip> {
        rows.iter().find(|(s, _)| s == state).map(|(_, c)| *c)
    }

    fn teleport(app: &mut App, to: Vec3, v: Vec3) {
        let mut q = app
            .world_mut()
            .query::<(&mut Transform, &mut Position, &mut LinearVelocity, &Hero)>();
        let (mut t, mut pos, mut lv, _) = q.single_mut(app.world_mut()).unwrap();
        t.translation = to;
        pos.0 = to;
        lv.0 = v;
    }

    /// Walk, sprint, let go of the sprint, stop: idle -> navigation -> dash -> navigation ->
    /// stop -> idle, each on its asb clip.
    #[test]
    fn trace_walk_sprint_stop() {
        let Some((mut app, _)) = foot_app() else { return };
        let mut t = 0;
        press(&mut app, KeyCode::KeyW);
        let a = follow(&mut app, "walk", 70, &mut t);
        press(&mut app, KeyCode::ShiftLeft);
        let b = follow(&mut app, "sprint", 70, &mut t);
        release(&mut app, KeyCode::ShiftLeft);
        let c = follow(&mut app, "unsprint", 40, &mut t);
        release(&mut app, KeyCode::KeyW);
        let d = follow(&mut app, "stop", 60, &mut t);
        let clips = |rows: &[(String, Clip)], state: &str| -> Vec<Clip> {
            rows.iter().filter(|(s, _)| s == state).map(|(_, c)| *c).collect()
        };
        assert!(clips(&a, "S_UPRIGHT_BASIC_NAVIGATION").contains(&Clip::Run), "{a:?}");
        assert_eq!(clips(&b, "S_DASH"), vec![Clip::Sprint], "{b:?}");
        assert!(states(&c).contains(&"S_UPRIGHT_BASIC_NAVIGATION"), "{c:?}");
        assert_eq!(clip_in(&d, "S_UPRIGHT_STOP"), Some(Clip::RunToIdle), "{d:?}");
        let tail: Vec<&str> = states(&d);
        assert!(in_order(&tail, &["S_UPRIGHT_STOP", "S_UPRIGHT_IDLE"]), "{d:?}");
        let (h, state, clip, ..) = snapshot(&mut app);
        assert_eq!((state.as_str(), clip), ("S_UPRIGHT_IDLE", Clip::Idle));
        assert!(h.speed < 0.3);
        // walking on level ground never plays the jump clip (a one-frame loss of ground
        // contact over a collider seam used to flash it)
        for rows in [&a, &b, &c, &d] {
            assert!(!rows.iter().any(|(_, c)| *c == Clip::Jump), "{rows:?}");
        }
        // every condition on these paths is decided
        let u = unsupported(&mut app, "walk");
        assert!(u.is_empty(), "{u:?}");
    }

    /// The opening transform of the flight model (the fixtures of `foot_controller_probe.py`).
    /// The native ground speeds of `rico-locomotion-motion.md` (tables 4.1 to 4.3).
    #[test]
    fn native_ground_speeds() {
        let near = |a: f32, b: f32| (a - b).abs() < 0.02;
        assert!(near(navigation_speed(1.0), 5.51), "{}", navigation_speed(1.0));
        assert!(near(navigation_speed(0.425), 2.53), "{}", navigation_speed(0.425));
        assert!(near(navigation_speed(0.5), 3.22));
        assert!(near(dash_speed(1.0), 5.21) && near(dash_speed(0.0), 8.33));
        assert!(near(dash_speed(0.5), 6.68));
        assert!(near(strafe_speed(1.0), 5.00) && near(strafe_speed(0.425), 2.06));
    }

    #[test]
    fn chute_opening_velocity_transform() {
        let v = chute_open_velocity(Vec3::new(3.0, -20.0, 4.0));
        assert!((v - Vec3::new(7.8, -12.0, 10.4)).length() < 1e-4, "{v}");
        // a capped fall speed, no heading
        assert_eq!(chute_open_velocity(Vec3::new(0.0, -40.0, 0.0)), Vec3::new(0.0, -28.0, 0.0));
        // rising or level: untouched
        assert_eq!(chute_open_velocity(Vec3::new(1.0, 2.0, 3.0)), Vec3::new(1.0, 2.0, 3.0));
    }

    /// A jump from rest is `ACT_JUMP_AT` like every other (the only dispatcher), a press
    /// during a landing waits up to 0.3 s for the landing to end, an earlier one is lost.
    #[test]
    fn trace_jumps() {
        let Some((mut app, _)) = foot_app() else { return };
        let mut t = 0;
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        let a = follow(&mut app, "standing jump", 110, &mut t);
        assert!(
            in_order(
                &states(&a),
                &["S_JUMP_START_SPRINT_FORWARD", "S_JUMP_TO_FALL_TRANSITION", "S_JUMP_LAND", "S_UPRIGHT_IDLE"]
            ),
            "{a:?}"
        );
        assert_eq!(clip_in(&a, "S_JUMP_START_SPRINT_FORWARD"), Some(Clip::JumpStartRun));
        assert_eq!(clip_in(&a, "S_JUMP_LAND"), Some(Clip::LandLight));
        // jump, and press again at the start of the landing (the landing lasts 0.55 s, the
        // press waits 0.3 s): the press is lost
        let jump = |app: &mut App| {
            press(app, KeyCode::Space);
            step(app, 1);
            release(app, KeyCode::Space);
        };
        let land = |app: &mut App| {
            for _ in 0..400 {
                step(app, 1);
                if snapshot(app).1 == "S_JUMP_LAND" {
                    return;
                }
            }
            panic!("never landed");
        };
        jump(&mut app);
        land(&mut app);
        jump(&mut app);
        let lost = follow(&mut app, "press at the start of the landing", 90, &mut t);
        assert!(!states(&lost).contains(&"S_JUMP_START_SPRINT_FORWARD"), "{lost:?}");
        assert!(states(&lost).contains(&"S_UPRIGHT_IDLE"), "{lost:?}");
        // jump again, press 0.25 s before the landing lets go: it fires then
        jump(&mut app);
        land(&mut app);
        step(&mut app, 20);
        jump(&mut app);
        let buffered = follow(&mut app, "press near the end of the landing", 60, &mut t);
        assert!(
            in_order(&states(&buffered), &["S_JUMP_LAND_INTERRUPT", "S_JUMP_START_SPRINT_FORWARD"]),
            "{buffered:?}"
        );
        // let that jump finish, and stand
        follow(&mut app, "settle", 200, &mut t);
        // running jump
        press(&mut app, KeyCode::KeyW);
        follow(&mut app, "run up", 50, &mut t);
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        let b = follow(&mut app, "running jump", 130, &mut t);
        assert!(
            in_order(
                &states(&b),
                &[
                    "S_JUMP_START_SPRINT_FORWARD",
                    "S_JUMP_TO_FALL_TRANSITION",
                    "S_JUMP_RUNNING_LAND",
                    "S_UPRIGHT_BASIC_NAVIGATION"
                ]
            ),
            "{b:?}"
        );
        assert_eq!(clip_in(&b, "S_JUMP_START_SPRINT_FORWARD"), Some(Clip::JumpStartRun));
        assert_eq!(clip_in(&b, "S_JUMP_TO_FALL_TRANSITION"), Some(Clip::JumpTimeBlend));
        assert_eq!(clip_in(&b, "S_JUMP_RUNNING_LAND"), Some(Clip::Land));
    }

    fn hero_pos(app: &mut App) -> Vec3 {
        snapshot(app).3
    }

    /// Crouch (hold mode): idle -> `S_DOWN_TO_CROUCH` (`crouch_DN`) -> `S_CROUCH` (`cr_idle`),
    /// held still; released -> `S_UP_FROM_CROUCH` (`crouch_UP`) -> idle. Moving input above 0.7
    /// stands him into the run; a crouch pressed with it never starts; the jump is open.
    #[test]
    fn trace_crouch() {
        let Some((mut app, _)) = foot_app() else { return };
        let mut t = 0;
        follow(&mut app, "settle", 20, &mut t);
        let p0 = hero_pos(&mut app);
        press(&mut app, KeyCode::ControlLeft);
        let a = follow(&mut app, "crouch held", 100, &mut t);
        assert!(in_order(&states(&a), &["S_DOWN_TO_CROUCH", "S_CROUCH"]), "{a:?}");
        assert_eq!(clip_in(&a, "S_DOWN_TO_CROUCH"), Some(Clip::CrouchDown));
        assert_eq!(clip_in(&a, "S_CROUCH"), Some(Clip::Crouch));
        assert!(!states(&a).contains(&"S_UP_FROM_CROUCH"), "held: it stays down, {a:?}");
        assert!((hero_pos(&mut app) - p0).length() < 0.05, "crouching does not move him");
        // a stick direction alone does not walk him (S_CROUCH takes no navigate): input 1.0
        // is above 0.7 and the graph stands him up into the run
        release(&mut app, KeyCode::ControlLeft);
        let b = follow(&mut app, "released", 80, &mut t);
        assert!(in_order(&states(&b), &["S_UP_FROM_CROUCH", "S_UPRIGHT_IDLE"]), "{b:?}");
        assert_eq!(clip_in(&b, "S_UP_FROM_CROUCH"), Some(Clip::CrouchUp));
        let moved = (hero_pos(&mut app) - p0).length();
        assert!((0.1..0.4).contains(&moved), "the rise carries him 0.23 m: {moved}");
        // crouched, then W: out of the crouch into the run
        press(&mut app, KeyCode::ControlLeft);
        follow(&mut app, "crouch again", 50, &mut t);
        // `follow` logs only changes, so the state he is leaving is read before the press
        assert_eq!(snapshot(&mut app).1, "S_CROUCH");
        press(&mut app, KeyCode::KeyW);
        let c = follow(&mut app, "W while crouched", 60, &mut t);
        assert_eq!(states(&c).first(), Some(&"S_UPRIGHT_BASIC_NAVIGATION"), "straight out of S_CROUCH: {c:?}");
        assert!(!states(&c).contains(&"S_UP_FROM_CROUCH"), "{c:?}");
        // running with Ctrl held never crouches
        let d = follow(&mut app, "running, crouch held", 60, &mut t);
        assert!(states(&d).iter().all(|s| !s.contains("CROUCH")), "{d:?}");
        release(&mut app, KeyCode::ControlLeft);
        release(&mut app, KeyCode::KeyW);
        follow(&mut app, "stop", 120, &mut t);
        // crouched, then a jump
        press(&mut app, KeyCode::ControlLeft);
        follow(&mut app, "crouch for the jump", 60, &mut t);
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        let e = follow(&mut app, "jump from the crouch", 60, &mut t);
        assert!(states(&e).contains(&"S_JUMP_START_SPRINT_FORWARD"), "{e:?}");
        let u = unsupported(&mut app, "crouch");
        assert!(u.is_empty(), "{u:?}");
    }

    /// The evasive roll: `S_EVADE` (`evade_fwd`, 5.69 m of root motion), `S_EVADE_INTERRUPT`,
    /// idle; no second roll or jump during it; a roll after it; the sprint after a roll waits.
    #[test]
    fn trace_evade() {
        let Some((mut app, _)) = foot_app() else { return };
        let mut t = 0;
        follow(&mut app, "settle", 20, &mut t);
        let p0 = hero_pos(&mut app);
        press(&mut app, KeyCode::KeyC);
        step(&mut app, 1);
        release(&mut app, KeyCode::KeyC);
        let a = follow(&mut app, "evade from rest", 100, &mut t);
        assert!(in_order(&states(&a), &["S_EVADE", "S_EVADE_INTERRUPT", "S_UPRIGHT_IDLE"]), "{a:?}");
        assert_eq!(clip_in(&a, "S_EVADE"), Some(Clip::EvadeFwd));
        let d = hero_pos(&mut app) - p0;
        let travelled = Vec3::new(d.x, 0.0, d.z).length();
        eprintln!("[evade from rest] travelled {travelled:.2} m");
        assert!((travelled - 5.69).abs() < 0.7, "the roll covers its root motion: {travelled}");
        // a press during the roll, and a jump press, do nothing
        press(&mut app, KeyCode::KeyC);
        step(&mut app, 1);
        release(&mut app, KeyCode::KeyC);
        let mut b = follow(&mut app, "roll, then presses during it", 10, &mut t);
        press(&mut app, KeyCode::KeyC);
        step(&mut app, 1);
        release(&mut app, KeyCode::KeyC);
        b.extend(follow(&mut app, "after the second press", 8, &mut t));
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        b.extend(follow(&mut app, "after the jump press", 100, &mut t));
        assert!(!states(&b).contains(&"S_JUMP_START_SPRINT_FORWARD"), "{b:?}");
        assert_eq!(states(&b).iter().filter(|s| **s == "S_EVADE").count(), 1, "{b:?}");
        // the roll while running, then on into the run
        follow(&mut app, "settle", 60, &mut t);
        press(&mut app, KeyCode::KeyW);
        follow(&mut app, "run up", 40, &mut t);
        press(&mut app, KeyCode::KeyC);
        step(&mut app, 1);
        release(&mut app, KeyCode::KeyC);
        let c = follow(&mut app, "evade running", 100, &mut t);
        assert!(in_order(&states(&c), &["S_EVADE", "S_EVADE_INTERRUPT", "S_UPRIGHT_IDLE", "S_UPRIGHT_BASIC_NAVIGATION"]), "{c:?}");
        // with the sprint key down the roll ends in the dash, and a sprint needs the 0.25 s
        press(&mut app, KeyCode::ShiftLeft);
        follow(&mut app, "sprint", 60, &mut t);
        press(&mut app, KeyCode::KeyC);
        step(&mut app, 1);
        release(&mut app, KeyCode::KeyC);
        let e = follow(&mut app, "evade sprinting", 100, &mut t);
        assert!(in_order(&states(&e), &["S_EVADE", "S_EVADE_INTERRUPT", "S_DASH"]), "{e:?}");
        let u = unsupported(&mut app, "evade");
        assert!(u.is_empty(), "{u:?}");
    }

    /// The chord is pulled only where the graph takes `ACT_PULLCHORD`: not in the 0.7 s lead-in
    /// to the skydive, not in an ordinary fall (a drop under 28.5 m), yes in the skydive.
    #[test]
    fn trace_the_chord_needs_the_skydive() {
        let Some((mut app, ground)) = foot_app() else { return };
        let mut t = 0;
        let here = snapshot(&mut app).3;
        let tap = |app: &mut App| {
            press(app, KeyCode::Space);
            step(app, 1);
            release(app, KeyCode::Space);
        };
        // a fall from 22 m stays `S_FALL`: the press does nothing
        teleport(&mut app, Vec3::new(here.x, ground + 28.0, here.z), Vec3::ZERO);
        follow(&mut app, "22 m", 20, &mut t);
        tap(&mut app);
        step(&mut app, 5);
        let (h, state, ..) = snapshot(&mut app);
        assert!(!h.parachute && state == "S_FALL", "{h:?} {state}");
        for _ in 0..400 {
            step(&mut app, 1);
        }
        // from higher: the lead-in refuses, the skydive accepts
        teleport(&mut app, Vec3::new(here.x, ground + 200.0, here.z), Vec3::ZERO);
        follow(&mut app, "200 m", 10, &mut t);
        tap(&mut app);
        let (h, state, ..) = snapshot(&mut app);
        assert!(!h.parachute && state == "S_FALL_TO_SKYDIVE", "{h:?} {state}");
        let rows = follow(&mut app, "skydive", 60, &mut t);
        assert!(states(&rows).contains(&"S_SKYDIVE"), "{rows:?}");
        tap(&mut app);
        step(&mut app, 3);
        let (h, state, ..) = snapshot(&mut app);
        assert!(h.parachute && state == "S_PULL_OPEN_PARACHUTE_VERTICAL", "{h:?} {state}");
        // and a second press cuts the canopy: the graph lets go (S_PARACHUTE is reached
        // first, so wait out the unfold)
        follow(&mut app, "unfold", 400, &mut t);
        tap(&mut app);
        let rows = follow(&mut app, "cut", 120, &mut t);
        assert!(!snapshot(&mut app).0.parachute);
        let s = states(&rows);
        assert!(
            s.iter().any(|x| matches!(*x, "S_FALL" | "S_PARACHUTE_DETACH_TO_FREEFALL" | "S_FALL_TO_SKYDIVE")),
            "{rows:?}"
        );
    }

    /// A short drop lands lightly; a long one (fall timer past 1.5 s, still in `S_FALL`,
    /// which is what a drop under 28.5 m is) lands heavily and holds him a moment.
    #[test]
    fn trace_falls() {
        let Some((mut app, ground)) = foot_app() else { return };
        let mut t = 0;
        let here = snapshot(&mut app).3;
        teleport(&mut app, here + Vec3::Y * 3.0, Vec3::ZERO);
        let a = follow(&mut app, "3 m drop", 100, &mut t);
        assert!(in_order(&states(&a), &["S_FALL", "S_JUMP_LAND", "S_UPRIGHT_IDLE"]), "{a:?}");
        assert_eq!(clip_in(&a, "S_FALL"), Some(Clip::Fall));
        teleport(&mut app, Vec3::new(here.x, ground + 28.0, here.z), Vec3::ZERO);
        let mut locked = false;
        let mut rows = Vec::new();
        let mut last = None;
        for _ in 0..260 {
            step(&mut app, 1);
            t += 1;
            let (h, state, clip, ..) = snapshot(&mut app);
            locked |= h.move_lock;
            let key = (state, clip);
            if last.as_ref() != Some(&key) {
                eprintln!("[28 m drop] {:6.2}s {:<30} {:?} lock={}", t as f32 / 64.0, key.0, key.1, h.move_lock);
                rows.push(key.clone());
                last = Some(key);
            }
        }
        assert!(
            in_order(&states(&rows), &["S_FALL", "S_JUMP_LAND_HEAVY", "S_UPRIGHT_IDLE"]),
            "{rows:?}"
        );
        assert_eq!(clip_in(&rows, "S_JUMP_LAND_HEAVY"), Some(Clip::LandHeavy));
        assert!(locked, "the heavy landing never held him");
    }

    /// Skydive, open the chute, fly it, land it: the pulled chord and the unfolding play
    /// before the flight; the landing is picked by the speeds at touchdown.
    #[test]
    fn trace_skydive_and_parachute() {
        let Some((mut app, ground)) = foot_app() else { return };
        let mut t = 0;
        let here = snapshot(&mut app).3;
        teleport(&mut app, Vec3::new(here.x, ground + 130.0, here.z), Vec3::ZERO);
        let a = follow(&mut app, "free fall", 160, &mut t);
        assert!(
            in_order(&states(&a), &["S_FALL", "S_FALL_TO_SKYDIVE", "S_SKYDIVE"]),
            "{a:?}"
        );
        assert_eq!(clip_in(&a, "S_FALL_TO_SKYDIVE"), Some(Clip::FallToSkydive));
        assert_eq!(clip_in(&a, "S_SKYDIVE"), Some(Clip::Dive));
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        let b = follow(&mut app, "open", 400, &mut t);
        assert!(
            in_order(
                &states(&b),
                &[
                    "S_PULL_OPEN_PARACHUTE_VERTICAL",
                    "S_UNFOLD_PARACHUTE_VERTICAL",
                    "S_PARACHUTE"
                ]
            ),
            "{b:?}"
        );
        assert_eq!(clip_in(&b, "S_PULL_OPEN_PARACHUTE_VERTICAL"), Some(Clip::ChuteOpen));
        // the canopy shows from the pulled chord on
        let canopy_shown = {
            let mut q = app
                .world_mut()
                .query_filtered::<&Visibility, With<Canopy>>();
            q.iter(app.world()).any(|v| *v != Visibility::Hidden)
        };
        assert!(canopy_shown, "no canopy in the chute states");
        // down it comes (circling with A, the tile is finite); the neutral glide is faster
        // than the native nice-landing band
        press(&mut app, KeyCode::KeyA);
        let mut rows = Vec::new();
        let mut last = None;
        for _ in 0..12000 {
            step(&mut app, 1);
            t += 1;
            let (h, state, clip, _, v) = snapshot(&mut app);
            // the camera follows the canopy's heading (a player circling with the mouse as well):
            // the native chute turns towards a camera more than 0.9 rad off (`S+4`, JC2-d5c3)
            app.world_mut().resource_mut::<OrbitCamera>().yaw = h.facing;
            let key = (state, clip);
            if last.as_ref() != Some(&key) {
                eprintln!(
                    "[landing] {:6.2}s {:<30} {:?} v=({:+5.1},{:+5.1},{:+5.1})",
                    t as f32 / 64.0,
                    key.0,
                    key.1,
                    v.x,
                    v.y,
                    v.z
                );
                rows.push(key.clone());
                last = Some(key);
            }
            if h.grounded {
                release(&mut app, KeyCode::KeyA);
            }
            if h.grounded && rows.len() > 2 && key_is_idle(&rows) {
                break;
            }
        }
        let land: Vec<&str> = states(&rows);
        assert!(land.contains(&"S_PARACHUTE"), "{rows:?}");
        assert!(
            land.iter().any(|s| matches!(*s, "S_PARACHUTE_LANDING" | "S_LANDING_ROLL" | "S_LANDING_HARD")),
            "no chute landing: {rows:?}"
        );
        assert_eq!(clip_in(&rows, "S_PARACHUTE_LANDING").unwrap_or(Clip::ChuteLand), Clip::ChuteLand);
        assert_eq!(clip_in(&rows, "S_LANDING_ROLL").unwrap_or(Clip::LandRoll), Clip::LandRoll);
        unsupported(&mut app, "skydive and chute");
    }

    /// Board 4d7f7642 (`parachute-state-velocities.md`): the lead-in `S_FALL_TO_SKYDIVE` flies
    /// the freefall model (no ballistic -21), the pull-open keeps it, and the canopy model with
    /// its opening transform starts on the first frame of the unfold state, not at the press.
    #[test]
    fn trace_state_velocities() {
        let Some((mut app, ground)) = foot_app() else { return };
        let mut t = 0;
        let here = snapshot(&mut app).3;
        teleport(&mut app, Vec3::new(here.x, ground + 400.0, here.z), Vec3::ZERO);
        // per tick: (state, velocity, canopy_live)
        let mut ticks: Vec<(String, Vec3, bool)> = Vec::new();
        let mut pressed = false;
        for _ in 0..1200 {
            step(&mut app, 1);
            t += 1;
            let (h, state, _, _, v) = snapshot(&mut app);
            ticks.push((state.clone(), v, h.canopy_live));
            if state == "S_SKYDIVE" && !pressed && t > 200 {
                pressed = true;
                press(&mut app, KeyCode::Space);
                step(&mut app, 1);
                release(&mut app, KeyCode::Space);
            }
            if state == "S_PARACHUTE" {
                break;
            }
        }
        let of = |s: &str| -> Vec<usize> {
            ticks.iter().enumerate().filter(|(_, x)| x.0 == s).map(|(i, _)| i).collect()
        };
        // the lead-in: the vertical speed changes by far less than the ballistic 21 m/s^2
        let lead = of("S_FALL_TO_SKYDIVE");
        assert!(lead.len() > 10, "no lead-in: {:?}", ticks.iter().map(|x| &x.0).collect::<Vec<_>>());
        let (a, b) = (lead[2], *lead.last().unwrap());
        let accel = (ticks[b].1.y - ticks[a].1.y) / ((b - a) as f32 / 64.0);
        assert!(accel > -15.0, "the lead-in fell ballistically: {accel} m/s^2");
        // the pull-open: still the freefall model, no canopy yet, no opening transform
        let pull = of("S_PULL_OPEN_PARACHUTE_VERTICAL");
        assert!(!pull.is_empty());
        assert!(pull.iter().all(|&i| !ticks[i].2), "the canopy model ran in the pull-open");
        // board caf7b21c: the pull-open lasts 0.463 s, the unfold to clip time 1.633 s (1.17 s)
        let secs = |n: usize| n as f32 / 64.0;
        assert!((secs(pull.len()) - 0.463).abs() < 0.08, "pull-open {} s", secs(pull.len()));
        let unfold_len = of("S_UNFOLD_PARACHUTE_VERTICAL").len();
        assert!((secs(unfold_len) - 1.17).abs() < 0.12, "unfold {} s", secs(unfold_len));
        let jump = |i: usize| (ticks[i].1.y - ticks[i - 1].1.y).abs();
        assert!(pull.iter().skip(1).all(|&i| jump(i) < 1.0), "velocity jumped in the pull-open");
        // the unfold: the model and the transform start on its first frame
        let unfold = of("S_UNFOLD_PARACHUTE_VERTICAL");
        assert!(!unfold.is_empty());
        // (the velocity step reads the state of the previous update: at most one tick late)
        let first = ticks.iter().position(|x| x.2).expect("the canopy model never started");
        assert!(first >= unfold[0] && first <= unfold[0] + 1, "{first} vs {}", unfold[0]);
        assert!(ticks[first].0.starts_with("S_UNFOLD"), "{}", ticks[first].0);
        assert!(
            ticks[first].1.y > ticks[first - 1].1.y + 3.0,
            "no opening transform at the unfold: {:?} -> {:?}",
            ticks[first - 1].1,
            ticks[first].1
        );
        assert!(unfold.iter().filter(|&&i| i > first).all(|&i| jump(i) < 3.0), "the transform ran twice");
    }

    /// Cutting the chute (`ACT_PULLCHORD` in `S_PARACHUTE`) is routed by the forward fan
    /// (brief section 4): ground within 30 m -> `S_FALL`; something within 60 m only ->
    /// `S_FALL_TO_SKYDIVE`; nothing within 60 m -> `S_PARACHUTE_DETACH_TO_FREEFALL`.
    #[test]
    fn trace_cut_chute_routes_by_the_fan() {
        for (height, want) in [
            (20.0, "S_FALL"),
            (45.0, "S_FALL_TO_SKYDIVE"),
            (300.0, "S_PARACHUTE_DETACH_TO_FREEFALL"),
        ] {
            let Some((mut app, ground)) = foot_app() else { return };
            let mut t = 0;
            let here = snapshot(&mut app).3;
            teleport(&mut app, Vec3::new(here.x, ground + 400.0, here.z), Vec3::ZERO);
            follow(&mut app, "fall", 160, &mut t);
            press(&mut app, KeyCode::Space);
            step(&mut app, 1);
            release(&mut app, KeyCode::Space);
            let b = follow(&mut app, "open", 400, &mut t);
            assert!(states(&b).contains(&"S_PARACHUTE"), "{b:?}");
            // the canopy at a chosen height over the flat ground
            let p = snapshot(&mut app).3;
            teleport(&mut app, Vec3::new(p.x, ground + height, p.z), Vec3::new(0.0, -1.7, -9.6));
            step(&mut app, 2);
            press(&mut app, KeyCode::Space);
            step(&mut app, 1);
            release(&mut app, KeyCode::Space);
            let c = follow(&mut app, "cut", 20, &mut t);
            assert!(states(&c).contains(&want), "{height} m: {c:?}");
        }
    }

    fn key_is_idle(rows: &[(String, Clip)]) -> bool {
        rows.last().is_some_and(|(s, _)| s == "S_UPRIGHT_IDLE")
    }

    /// The native canopy model's fixed points against the landing edges (brief section 3 and
    /// 5): full back (7.24 m/s along, 9.52 down) is past the hard edge (vertical < -8.1) and
    /// not past the fast one; the dive (W held, 45 m/s along) is past the fast edge and rolls.
    #[test]
    fn trace_chute_landing_bands() {
        for (label, key, want) in [
            ("full back", KeyCode::KeyS, "S_LANDING_HARD"),
            ("dive glide", KeyCode::KeyW, "S_LANDING_ROLL"),
        ] {
            let Some((mut app, ground)) = foot_app() else { return };
            let mut t = 0;
            let here = snapshot(&mut app).3;
            teleport(&mut app, Vec3::new(here.x, ground + 90.0, here.z), Vec3::ZERO);
            follow(&mut app, "fall", 60, &mut t);
            press(&mut app, KeyCode::Space);
            step(&mut app, 1);
            release(&mut app, KeyCode::Space);
            press(&mut app, key);
            let rows = follow(&mut app, label, 3000, &mut t);
            assert!(states(&rows).contains(&want), "{label}: {rows:?}");
        }
    }

    /// Dropped into the sea he floats, paddles, and wades out.
    #[test]
    fn trace_swim() {
        let Some((mut app, _)) = foot_app() else { return };
        let mut t = 0;
        teleport(
            &mut app,
            Vec3::new(768.0, crate::world::SEA_LEVEL + 3.0, -8960.0),
            Vec3::ZERO,
        );
        let a = follow(&mut app, "splash", 200, &mut t);
        assert!(
            in_order(&states(&a), &["S_FALL", "S_BLEND_TO_SWIM_IDLE", "S_SWIM_IDLE"]),
            "{a:?}"
        );
        assert_eq!(clip_in(&a, "S_SWIM_IDLE"), Some(Clip::SwimIdle));
        press(&mut app, KeyCode::KeyW);
        let b = follow(&mut app, "paddle", 100, &mut t);
        assert!(states(&b).contains(&"S_SWIM_NAVIGATION"), "{b:?}");
        assert_eq!(clip_in(&b, "S_SWIM_NAVIGATION"), Some(Clip::Swim));
        unsupported(&mut app, "swim");
    }
}
