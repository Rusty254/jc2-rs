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
const WALK: f32 = 2.6;
const RUN: f32 = 4.5;
const SPRINT: f32 = 7.0;
const GROUND_ACCEL: f32 = 40.0;
const AIR_ACCEL: f32 = 8.0;
const JUMP_SPEED: f32 = 5.5;
/// Steeper than this is a wall, not ground.
const MAX_SLOPE: f32 = 0.87;
/// How far below the capsule the ground probe reaches.
const GROUND_PROBE: f32 = 0.25;
/// Parachute: descent (m/s) at neutral, pulling back (S) and diving (W); forward glide.
const CHUTE_DESCENT: f32 = 5.5;
const CHUTE_DESCENT_SLOW: f32 = 3.0;
const CHUTE_DESCENT_FAST: f32 = 9.0;
const CHUTE_GLIDE: f32 = 11.0;
const CHUTE_GLIDE_SLOW: f32 = 6.0;
const CHUTE_GLIDE_FAST: f32 = 15.0;
const CHUTE_TURN: f32 = 1.3;
const CANOPY_HEIGHT: f32 = 5.0;
/// Minimum height above the ground to open the chute.
const CHUTE_MIN_HEIGHT: f32 = 3.0;
pub const ENTER_RANGE: f32 = 4.5;
const CAMERA_DISTANCE: f32 = 4.2;
const CAMERA_DISTANCE_CHUTE: f32 = 7.5;
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
    jump_cooldown: f32,
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
            jump_cooldown: 0.0,
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

#[derive(Resource)]
struct HeroLoading {
    model: Handle<RbmAsset>,
    hook: Option<Handle<RbmAsset>>,
    spawned: bool,
}

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::rig::RigPlugin>() {
            app.add_plugins(crate::rig::RigPlugin);
        }
        if !app.is_plugin_added::<crate::grapple::GrapplePlugin>() {
            app.add_plugins(crate::grapple::GrapplePlugin);
        }
        app.add_plugins(crate::ragdoll::RagdollPlugin);
        app.init_resource::<CameraMode>()
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
    *mode = CameraMode::Hero;
    commands.insert_resource(HeroParams { settings, grapple });
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
        base_color: Color::srgb(0.85, 0.85, 0.8),
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
        orbit.pitch = (orbit.pitch - motion.delta.y * 0.0035).clamp(-1.3, 0.9);
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
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        mv += fwd;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
        mv -= fwd;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        mv += right;
    }
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        mv -= right;
    }
    input.movement = mv.normalize_or_zero();
    input.sprint = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
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
        (keys.pressed(KeyCode::KeyW) as i32 - keys.pressed(KeyCode::KeyS) as i32) as f32;
    input.chute_turn =
        (keys.pressed(KeyCode::KeyA) as i32 - keys.pressed(KeyCode::KeyD) as i32) as f32;
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
    let mv = input.movement;
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
        // walk on the ground plane
        let speed = if mv == Vec3::ZERO {
            0.0
        } else if input.sprint {
            SPRINT
        } else if input.movement.length() > 0.5 {
            RUN
        } else {
            WALK
        };
        let along = (mv - normal * mv.dot(normal)).normalize_or_zero();
        let target = along * speed;
        let horizontal = Vec3::new(v.x, 0.0, v.z);
        let delta = target - horizontal;
        let step = GROUND_ACCEL * dt;
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
        if input.jump {
            v.y = JUMP_SPEED;
            hero.grounded = false;
            hero.jump_cooldown = 0.25;
        }
        hero.facing = ground_facing(hero.facing, mv, hero.aim_yaw);
    } else if hero.parachute {
        // the chute: drift forward at a steady glide, sink gently, turn with A/D
        hero.facing += input.chute_turn * CHUTE_TURN * dt;
        let fwd = facing_dir(hero.facing);
        let (descent, glide) = if input.chute_pull > 0.5 {
            (CHUTE_DESCENT_FAST, CHUTE_GLIDE_FAST)
        } else if input.chute_pull < -0.5 {
            (CHUTE_DESCENT_SLOW, CHUTE_GLIDE_SLOW)
        } else {
            (CHUTE_DESCENT, CHUTE_GLIDE)
        };
        // (rising off a reel, the climb fades slowly: the grapple-and-chute boost)
        let kv = 1.0 - (-dt / if v.y > 0.0 { 2.5 } else { 0.6 }).exp();
        let kh = 1.0 - (-dt / 1.2).exp();
        let target_h = fwd * glide;
        v.x += (target_h.x - v.x) * kh;
        v.z += (target_h.z - v.z) * kh;
        v.y += (-descent - v.y) * kv;
        // the step's gravity would undo the sink: cancel it
        v.y += 9.81 * dt;
    } else {
        // airborne: a little steering, facing the way we fly
        let step = AIR_ACCEL * dt;
        v.x += mv.x * step;
        v.z += mv.z * step;
        let h = Vec3::new(v.x, 0.0, v.z);
        if h.length() > 1.0 {
            hero.facing = yaw_toward(h);
        }
    }

    // hanging from the hook: `Space` climbs or leaps, `Ctrl` drops (before the chute sees it)
    crate::grapple::hang_inputs(&mut hero, rig, &mut input, &mut v, tf.translation);
    crate::grapple::climb_assist(&hero, rig, &mut v, dt);
    if input.chute_toggle {
        input.chute_toggle = false;
        if hero.parachute {
            hero.parachute = false;
        } else if !hero.grounded {
            // only with room under the feet
            let room = spatial
                .cast_ray(
                    tf.translation,
                    Dir3::NEG_Y,
                    CHUTE_MIN_HEIGHT + HEIGHT,
                    true,
                    &filter,
                )
                .map(|h| h.distance - HEIGHT / 2.0)
                .unwrap_or(f32::MAX);
            if room > CHUTE_MIN_HEIGHT {
                hero.parachute = true;
                if hero.grapple.is_none() {
                    // opening the chute kills the fall
                    v.y = v.y.max(-CHUTE_DESCENT);
                }
            }
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
        &mut Hero,
        &LinearVelocity,
        &mut Animator,
        Option<&mut Arms>,
        Option<&Children>,
        Option<&GrappleRig>,
        Option<&crate::ragdoll::FallbackPose>,
    )>,
    models: Query<&Transform, With<HeroModel>>,
    pose: Option<Res<crate::poses::PoseOverride>>,
    input: Res<HeroInput>,
    mut seen: Local<HeroAnimMemory>,
) {
    let Some(rig) = rig else { return };
    let dt = time.delta_secs();
    for (mut hero, lin, mut an, mut arms, children, hook, fallback) in &mut heroes {
        if hero.downed {
            seen.fall_speed = 0.0;
            seen.hooked = false;
            hero.aim_yaw = None;
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
        let (clip, gait, rate, blend) = match pose.as_deref() {
            // debug: `--pose`
            Some(p) => (Some(p.clip), p.gait, 1.0, 0.2),
            None => pick_base(&hero, lin, &an, &rig, class, hook, &mut seen),
        };
        if let Some(clip) = clip {
            an.rate = rate;
            if clip == Clip::Chute {
                // the speed axis of the canopy's time blend: W dives, S leans back
                an.hold(clip, 0.5 + 0.4 * input.chute_pull.clamp(-1.0, 1.0), blend);
            } else if clip == Clip::Reel {
                // `ANIM_REEL_FLIGHT` is a held pose (play mode 0): its middle
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
        let phase = hook.map(|g| g.phase);
        let firing = matches!(phase, Some(GrapplePhase::Flight));
        if firing
            && !seen.hooked
            && let Some(a) = arms.as_mut()
        {
            a.hooks += 1;
        }
        seen.hooked = firing;
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
                    // its turn axis: A pulls the left handle down
                    let t = 0.5 - 0.4 * input.chute_turn.clamp(-1.0, 1.0);
                    an.hold_layer(crate::rig::Slot::Wield, Clip::ChuteTurn, t, 0.3);
                }
            }
            None => hero.aim_yaw = None,
        }
        let action = an.layer(crate::rig::Slot::Action).clip;
        if matches!(phase, Some(GrapplePhase::Flight | GrapplePhase::Pull)) {
            if action.is_none() || action == Some(Clip::GrappleAim) {
                an.set_layer(crate::rig::Slot::Action, Some(Clip::GrappleAim), 0.1, false);
            }
        } else if action == Some(Clip::GrappleAim) {
            an.set_layer(crate::rig::Slot::Action, None, 0.4, false);
        }
    }
}

/// What `hero_animation` remembers between frames.
#[derive(Default)]
pub(crate) struct HeroAnimMemory {
    /// The fastest he fell (m/s) since leaving the ground.
    fall_speed: f32,
    hooked: bool,
}

/// Falls faster than these (m/s) land heavily, and roll when he is moving too.
const HEAVY_LANDING: f32 = 11.0;
const ROLL_LANDING: f32 = 17.0;

/// The base clip for his state (`None`: keep what plays, a landing playing out), the
/// gait the gun follows, the playback rate and the crossfade.
fn pick_base(
    hero: &Hero,
    lin: &LinearVelocity,
    an: &Animator,
    rig: &Rig,
    class: Option<crate::poses::WeaponClass>,
    hook: Option<&GrappleRig>,
    seen: &mut HeroAnimMemory,
) -> (Option<Clip>, Gait, f32, f32) {
    let duration = |c: Clip| rig.duration(c);
    let has = |c: Clip| rig.has(c);
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
    if hero.grounded && hero.grapple.is_none() && on_foot {
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
    if !hero.grounded {
        let clip = if hero.air_time < 0.35 && lin.y > 0.5 {
            Clip::Jump
        } else if hero.air_time > 1.5 && lin.y < -12.0 && has(Clip::Dive) {
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
    } else if speed < SPRINT_MIN || side {
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
    )>,
    mut models: Query<
        (&mut Transform, &mut ModelVisualState),
        (With<HeroModel>, Without<Hero>, Without<Canopy>),
    >,
    mut canopies: Query<
        (&mut Transform, &mut Visibility),
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
    let Ok((hero, tf, children, rig, an, dead, physics_pose, pending, wire_tilt)) = heroes.single()
    else {
        return;
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
            let target = correction * Quat::from_rotation_y(hero.facing - body);
            m.rotation = if wire_tilt.is_some() || state.wire_tilt {
                // The correction already uses fixed-step exponential smoothing;
                // do not introduce an extra render-frame-dependent body-up lag.
                target
            } else {
                m.rotation.slerp(target, 0.35)
            };
            state.wire_tilt = wire_tilt.is_some();
            // lean into the glide under the canopy, hang from the wire (forward is −z:
            // a negative turn about x tips the head forward)
            let tilt = if hero.parachute { 0.35 } else { 0.0 };
            m.rotation *= Quat::from_rotation_x(-tilt);
        }
        if let Ok((mut c, mut vis)) = canopies.get_mut(child) {
            *vis = if hero.parachute {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            c.rotation = Quat::from_rotation_y(hero.facing);
        }
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
                w.slack = r.slack;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

type HeroCam<'w, 's> =
    Query<'w, 's, (&'static mut Transform, &'static mut FlyCamera), Without<Hero>>;

pub(crate) fn hero_camera(
    mode: Res<CameraMode>,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut orbit: ResMut<OrbitCamera>,
    heroes: Query<(Entity, &Hero, &Transform)>,
    mut cams: HeroCam,
) {
    if *mode != CameraMode::Hero {
        return;
    }
    let Ok((entity, hero, tf)) = heroes.single() else {
        return;
    };
    let Ok((mut cam_tf, mut fly)) = cams.single_mut() else {
        return;
    };
    let wanted = if hero.parachute {
        CAMERA_DISTANCE_CHUTE
    } else {
        CAMERA_DISTANCE
    };
    orbit.distance += (wanted - orbit.distance) * (1.0 - (-3.0 * time.delta_secs()).exp());
    let target = tf.translation + Vec3::Y * 0.6;
    let rot = Quat::from_euler(EulerRot::YXZ, orbit.yaw, orbit.pitch, 0.0);
    let back = rot * Vec3::Z;
    // pull the camera in when something stands between it and Rico
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);
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
             WASD move  Shift sprint  Space jump / parachute  F grapple  E vehicle  right-drag or Tab look  V {}",
            hero.speed,
            if hero.air_time > 0.3 && !hero.parachute && hero.grapple.is_none() {
                format!("   falling {:.1} s", hero.air_time)
            } else {
                String::new()
            },
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
            assert_eq!(joints.iter(app.world()).count(), 84);
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
        assert!(h.speed > RUN - 0.5, "speed {}", h.speed);
        release(&mut app, KeyCode::KeyW);
        step(&mut app, 30);
        let (h, _, _) = hero(&mut app);
        assert!(h.speed < 0.3, "still moving at {}", h.speed);

        // jump
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        step(&mut app, 10);
        let (h, p3, v) = hero(&mut app);
        assert!(
            !h.grounded && p3.y > p2.y + 0.3 && v.y > 0.0,
            "jump {h:?} {v:?}"
        );
        step(&mut app, 90);
        let (h, _, _) = hero(&mut app);
        assert!(h.grounded, "didn't land after the jump: {h:?}");

        // parachute from 80 m up
        {
            let mut q = app
                .world_mut()
                .query::<(&mut Transform, &mut Position, &mut LinearVelocity, &Hero)>();
            let (mut t, mut pos, mut v, _) = q.single_mut(app.world_mut()).unwrap();
            t.translation.y = ground + 80.0;
            pos.0 = t.translation;
            v.0 = Vec3::ZERO;
        }
        step(&mut app, 20);
        let (h, _, _) = hero(&mut app);
        assert!(!h.grounded, "should be falling");
        press(&mut app, KeyCode::Space);
        step(&mut app, 1);
        release(&mut app, KeyCode::Space);
        step(&mut app, 180);
        let (h, p4, v) = hero(&mut app);
        assert!(h.parachute, "chute not open: {h:?}");
        assert!(
            (v.y + CHUTE_DESCENT).abs() < 1.0,
            "descent {} m/s (want {})",
            v.y,
            -CHUTE_DESCENT
        );
        let glide = Vec3::new(v.x, 0.0, v.z).length();
        assert!((glide - CHUTE_GLIDE).abs() < 1.5, "glide {glide} m/s");
        assert!(p4.y > ground + 30.0, "too low already: {}", p4.y);
        // the chute closes on landing
        for _ in 0..1500 {
            step(&mut app, 1);
            let (h, _, _) = hero(&mut app);
            if h.grounded {
                break;
            }
        }
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
        step(&mut app, 60);
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
            let (clip, _, rate, _) = pick_base(&hero, &lin, &an, &rig, None, None, &mut mem);
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
        let (clip, ..) = pick_base(&hero, &lin, &an, &rig, None, None, &mut mem);
        assert_eq!(clip, Some(Clip::RunStrafeLeft));
    }
}
