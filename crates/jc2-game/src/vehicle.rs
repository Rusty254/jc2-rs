//! Milestone 4: a drivable car.
//!
//! A raycast vehicle on Avian: the chassis is a dynamic body wearing the convex hull of
//! its body model; every wheel is a ray along the suspension, a spring/damper and a tyre
//! model that pushes the chassis along the ground. Everything that can come from the
//! data does (`jc2_formats::vehicle`): wheel positions, radii and friction, suspension
//! travel and relative stiffness, mass and centre of mass, engine torque and rev range,
//! gear ratios, top speed, steering lock and its speed falloff, brake torques, which
//! axle the handbrake locks. The tyre and engine models themselves are ours — the
//! original drives a Havok vehicle whose exact curves we have not reverse engineered.
//!
//! `jc2 --world 31,14 --drive lave.v014_jeep.ee`
//! W/S throttle and brake/reverse, A/D steer, Space handbrake, V chase/free camera,
//! R puts the car back on its wheels.

use crate::Game;
use crate::coords::{MODEL_FORWARD, MODEL_RIGHT, forward_of, mat_to_bevy, to_bevy};
use crate::physics::colliders_from_body;
use crate::rbm_loader::RbmAsset;
use crate::world::FlyCamera;
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::collision::{CollisionBody, parse_pfx};
use jc2_formats::lod::LodDescriptor;
use jc2_formats::sarc::SmallArchive;
use jc2_formats::vehicle::{VehicleDoll, VehiclePart};

/// Who the camera and the keys belong to: the free fly camera, Rico, or the car.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraMode {
    #[default]
    Free,
    Hero,
    Drive,
}

/// Per-wheel data derived from the doll.
#[derive(Clone, Debug)]
pub struct WheelSpec {
    /// Hub position in the chassis frame (Bevy axes) at the modelled suspension pose.
    pub hub: Vec3,
    pub radius: f32,
    pub travel: f32,
    /// Spring constant (N/m) and damping (N·s/m) for compression / rebound.
    pub spring: f32,
    pub damp_compress: f32,
    pub damp_rebound: f32,
    pub friction: f32,
    pub steers: bool,
    pub drives: bool,
    pub handbrake: bool,
    pub brake_force: f32,
    /// Rotation and scale of the tyre model in the chassis frame.
    pub base_rotation: Quat,
    pub scale: Vec3,
    /// The doll's wheel number, to match tyre parts to this entry.
    pub number: u32,
}

#[derive(Clone, Debug)]
pub struct VehicleSpec {
    pub name: String,
    pub mass: f32,
    pub center_of_mass: Vec3,
    pub wheels: Vec<WheelSpec>,
    /// Full steering lock in radians, and the speed (m/s) above which it shrinks.
    pub max_steer: f32,
    pub steer_speed: f32,
    pub torque: f32,
    pub torque_rpm: f32,
    pub rpm_min: f32,
    pub rpm_max: f32,
    pub gears: Vec<f32>,
    pub reverse: f32,
    pub final_drive: f32,
    pub top_speed: f32,
    /// Quadratic drag so that the top gear's force balances at `top_speed`.
    pub drag: f32,
}

/// Fraction of the suspension travel used up when the car sits still, i.e. where the
/// artist put the wheel.
const REST_COMPRESSION: f32 = 0.45;
/// Slip angle (radians) at which a tyre reaches full lateral grip, and the sideways
/// speed that stands in for it when the wheel barely rolls.
const SLIP_ANGLE_SATURATION: f32 = 0.14;
const LATERAL_SATURATION_MIN: f32 = 0.3;
/// Rolling resistance as a fraction of weight.
const ROLLING_RESISTANCE: f32 = 0.015;

impl VehicleSpec {
    pub fn from_doll(name: &str, doll: &VehicleDoll) -> Self {
        let mass = doll.mass();
        let g = 9.81;
        // share of the weight each wheel carries, from the axle strengths
        let tyres = doll.wheels();
        let strengths: Vec<f32> = tyres
            .iter()
            .map(|p| {
                doll.axle_for_wheel(p.wheel.unwrap_or(0))
                    .and_then(|a| a.suspension.as_ref())
                    .map(|s| s.strength)
                    .unwrap_or(1.0)
                    .max(0.1)
            })
            .collect();
        let total_strength: f32 = strengths.iter().sum::<f32>().max(0.1);
        let mut wheels = Vec::new();
        for (p, strength) in tyres.iter().zip(strengths) {
            let axle = doll.axle_for_wheel(p.wheel.unwrap_or(0));
            let susp = axle.and_then(|a| a.suspension.as_ref());
            let share = strength / total_strength;
            let travel = susp.map(|s| s.length).unwrap_or(0.25).max(0.05);
            let spring = share * mass * g / (REST_COMPRESSION * travel);
            let sprung = share * mass;
            let zc = susp
                .map(|s| 0.25 + 0.05 * s.compression)
                .unwrap_or(0.4)
                .clamp(0.2, 0.9);
            let zr = susp
                .map(|s| 0.3 + 0.05 * s.relaxation)
                .unwrap_or(0.5)
                .clamp(0.2, 0.9);
            let tf = mat_to_bevy(&p.world);
            let front = axle
                .map(|a| a.index == 0)
                .unwrap_or(p.world.translation()[2] < 0.0);
            // The doll's origin sits on the ground and the tyre model is scaled so that
            // its bottom touches it, so the rolling radius is the hub height; the module's
            // `radius` (about 2/3 of that) is Havok's smaller physics wheel.
            let radius = axle.map(|a| a.radius).unwrap_or(0.35).max(tf.translation.y);
            wheels.push(WheelSpec {
                hub: tf.translation,
                radius,
                travel,
                spring,
                damp_compress: 2.0 * zc * (spring * sprung).sqrt(),
                damp_rebound: 2.0 * zr * (spring * sprung).sqrt(),
                friction: axle.map(|a| a.friction).unwrap_or(1.0).clamp(0.5, 2.0),
                steers: front,
                drives: true,
                handbrake: axle
                    .and_then(|a| a.brake.as_ref())
                    .map(|b| b.handbrake)
                    .unwrap_or(!front),
                brake_force: axle
                    .and_then(|a| a.brake.as_ref())
                    .map(|b| b.max_torque)
                    .unwrap_or(2000.0)
                    / radius,
                base_rotation: tf.rotation,
                scale: p.scale.map(Vec3::from).unwrap_or(Vec3::ONE),
                number: p.wheel.unwrap_or(0),
            });
        }
        let engine = doll.engine.as_ref();
        let trm = doll.transmission.as_ref();
        let gears = trm
            .map(|t| t.gears.clone())
            .filter(|g| !g.is_empty())
            .unwrap_or(vec![3.0, 2.0, 1.4, 1.0, 0.8]);
        let final_drive = trm.map(|t| t.reverse).unwrap_or(3.0).clamp(2.0, 5.0);
        let torque = engine.map(|e| e.torque).unwrap_or(300.0);
        let top_speed = trm.map(|t| t.top_speed_kmh).unwrap_or(120.0) / 3.6;
        let radius = wheels.first().map(|w| w.radius).unwrap_or(0.35);
        let top_force = torque * gears.last().copied().unwrap_or(1.0) * final_drive / radius;
        let drag = top_force / (top_speed * top_speed).max(1.0);
        let com = doll
            .base
            .as_ref()
            .map(|b| to_bevy(b.center_of_mass))
            .unwrap_or(Vec3::new(0.0, 0.5, 0.0));
        Self {
            name: name.to_string(),
            mass,
            center_of_mass: com,
            wheels,
            max_steer: doll
                .steering
                .as_ref()
                .map(|s| s.max_angle)
                .unwrap_or(35.0)
                .to_radians(),
            steer_speed: doll.steering.as_ref().map(|s| s.speed_kmh).unwrap_or(50.0) / 3.6,
            torque,
            torque_rpm: engine.map(|e| e.torque_rpm).unwrap_or(3000.0),
            rpm_min: engine.map(|e| e.rpm_min).unwrap_or(800.0),
            rpm_max: engine.map(|e| e.rpm_max).unwrap_or(6000.0),
            gears,
            reverse: trm.map(|t| t.reverse).unwrap_or(3.0),
            final_drive,
            top_speed,
            drag,
        }
    }

    /// Engine torque at an rpm: flat around the peak, falling off towards the limits.
    fn torque_at(&self, rpm: f32) -> f32 {
        let span = (self.rpm_max - self.rpm_min).max(1.0);
        let x = ((rpm - self.torque_rpm) / span).abs();
        self.torque * (1.0 - 0.6 * x * x).max(0.3)
    }
}

#[derive(Clone, Debug, Default)]
pub struct WheelState {
    pub compression: f32,
    pub grounded: bool,
    /// Current hub position in the chassis frame.
    pub hub: Vec3,
    pub spin: f32,
    pub steer: f32,
    pub load: f32,
    /// Last longitudinal / lateral tyre force (N), for the HUD and tests.
    pub force: Vec2,
}

#[derive(Component)]
pub struct Vehicle {
    pub spec: VehicleSpec,
    pub wheels: Vec<WheelState>,
    pub throttle: f32,
    pub brake: f32,
    pub steer_input: f32,
    pub handbrake: bool,
    pub gear: usize,
    pub rpm: f32,
    pub speed: f32,
    pub length: f32,
    /// Child entities carrying extra chassis colliders; the wheel rays must skip them.
    pub collider_children: Vec<Entity>,
}

/// Something Rico can climb into; `reach` widens the `E` range for big vehicles.
#[derive(Component, Clone, Copy)]
pub struct Enterable {
    pub reach: f32,
}

/// The vehicle being driven: Rico is in it (or there is no Rico, `--no-hero`).
#[derive(Component)]
pub struct Occupied;

/// A car the traffic AI drives (`traffic.rs` sets its throttle, brake and steering); the
/// keys leave it alone until someone gets in.
#[derive(Component)]
pub struct AiDriven;

/// Present when the character plugin runs; without it vehicles drive themselves.
#[derive(Resource)]
pub struct HeroEnabled;

#[derive(Component)]
struct WheelVisual(usize);

#[derive(Component)]
struct DriveHud;

pub struct PendingPart {
    pub part: VehiclePart,
    pub handle: Handle<RbmAsset>,
}

pub struct VehiclePlugin;

impl Plugin for VehiclePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .add_systems(Startup, spawn_drive_hud)
            .add_systems(
                Update,
                (drive_input, wheel_visuals, chase_camera, drive_hud),
            )
            .add_systems(FixedUpdate, vehicle_physics);
    }
}

/// A vehicle's parts, requested from the asset server.
pub struct DollParts {
    pub parts: Vec<PendingPart>,
    /// The root part's model (the body).
    pub body: Handle<RbmAsset>,
    /// The body part's Havok shape (`v014_lod1-body_col.pfx`), when it decodes.
    pub body_collision: Option<CollisionBody>,
}

/// Ask for every part's model at `lod` and decode the body's collision shape.
pub fn load_doll_parts(
    container: &str,
    sarc: &SmallArchive,
    data: &[u8],
    doll: &VehicleDoll,
    game: &Game,
    assets: &AssetServer,
    lod: usize,
) -> Option<DollParts> {
    let mut parts = Vec::new();
    let mut body: Option<Handle<RbmAsset>> = None;
    let mut body_collision: Option<CollisionBody> = None;
    for part in &doll.parts {
        if part.depth == 0 && body_collision.is_none() && !part.collision.is_empty() {
            let bytes = sarc
                .get(&part.collision)
                .and_then(|e| sarc.data(data, e).ok())
                .map(|b| b.to_vec())
                .or_else(|| game.fs.read_inflated(&part.collision).ok());
            match bytes.map(|b| parse_pfx(&b)) {
                Some(Ok(mut bodies)) if !bodies.is_empty() => {
                    let b = bodies.remove(0);
                    info!(
                        "{container}: chassis collision {} ({} shapes)",
                        part.collision,
                        b.shape.leaf_count()
                    );
                    body_collision = Some(b);
                }
                Some(Err(e)) => warn!("{container}: {}: {e}", part.collision),
                _ => warn!(
                    "{container}: {} not found, using the render mesh",
                    part.collision
                ),
            }
        }
        if part.model.is_empty() {
            continue;
        }
        let lod_name = part.model_file();
        let text = match sarc.get(lod_name) {
            Some(e) => sarc.data(data, e).ok().map(|b| b.to_vec()),
            None => game.fs.read_inflated(lod_name).ok(),
        };
        let Some(text) = text else {
            warn!("{}: {lod_name} not found", part.name);
            continue;
        };
        let d = LodDescriptor::parse(&String::from_utf8_lossy(&text));
        let Some(model) = d.model_for_level(lod) else {
            continue;
        };
        let handle: Handle<RbmAsset> = assets.load(format!("jc2://{container}/{model}"));
        if part.depth == 0 && body.is_none() {
            body = Some(handle.clone());
        }
        parts.push(PendingPart {
            part: part.clone(),
            handle,
        });
    }
    Some(DollParts {
        parts,
        body: body?,
        body_collision,
    })
}

fn spawn_drive_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(16.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            bottom: Val::Px(10.0),
            ..default()
        },
        DriveHud,
        crate::hud::DevText,
    ));
}

/// Spawn a car with its parts, wheels resting on the ground at `ground` (the point under
/// the chassis origin), facing `rotation`. The parts must have loaded.
#[allow(clippy::too_many_arguments)]
pub fn build_car(
    commands: &mut Commands,
    spec: &VehicleSpec,
    body: &RbmAsset,
    meshes: &Assets<Mesh>,
    rbms: &Assets<RbmAsset>,
    parts: &[PendingPart],
    body_collision: Option<&CollisionBody>,
    ground: Vec3,
    rotation: Quat,
) -> Entity {
    let (lo, hi) = (body.bbox_min, body.bbox_max);
    let (collider, extra_colliders) = chassis_collider(body, meshes, body_collision);
    let spec = spec.clone();
    let mass = spec.mass;
    let inertia = inertia_for_mass(&collider, mass);
    let wheels: Vec<WheelState> = spec
        .wheels
        .iter()
        .map(|w| WheelState {
            hub: w.hub,
            compression: REST_COMPRESSION * w.travel,
            ..default()
        })
        .collect();
    let ground_clearance = spec
        .wheels
        .iter()
        .map(|w| w.hub.y - w.radius)
        .fold(f32::MAX, f32::min)
        .max(0.0);
    let start = ground + Vec3::Y * (0.8 - ground_clearance);
    let length = hi.z - lo.z;
    let vehicle = Vehicle {
        spec: spec.clone(),
        wheels,
        throttle: 0.0,
        brake: 0.0,
        steer_input: 0.0,
        handbrake: false,
        gear: 0,
        rpm: spec.rpm_min,
        speed: 0.0,
        length,
        collider_children: Vec::new(),
    };
    let mut entity = commands.spawn((
        RigidBody::Dynamic,
        collider,
        (
            Mass(mass),
            inertia,
            CenterOfMass(spec.center_of_mass),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
        ),
        AngularDamping(0.5),
        LinearDamping(0.02),
        SleepingDisabled,
        TransformInterpolation,
        Transform::from_translation(start).with_rotation(rotation),
        Visibility::default(),
        vehicle,
        Enterable {
            reach: length * 0.3,
        },
        Name::new(format!("vehicle {}", spec.name)),
    ));
    let mut collider_children = Vec::new();
    entity.with_children(|parent| {
        for (p, r, c) in extra_colliders {
            let id = parent
                .spawn((
                    c,
                    Transform {
                        translation: p,
                        rotation: r,
                        scale: Vec3::ONE,
                    },
                    Name::new("chassis mesh collider"),
                ))
                .id();
            collider_children.push(id);
        }
        for p in parts {
            let Some(rbm) = rbms.get(&p.handle) else {
                continue;
            };
            let mut tf = mat_to_bevy(&p.part.world);
            if let Some(sc) = p.part.scale {
                tf.scale *= Vec3::from(sc);
            }
            let wheel_index = p
                .part
                .wheel
                .and_then(|w| spec.wheels.iter().position(|ws| ws.number == w));
            let mut part_entity =
                parent.spawn((tf, Visibility::default(), Name::new(p.part.name.clone())));
            if let Some(i) = wheel_index {
                part_entity.insert(WheelVisual(i));
            }
            part_entity.with_children(|pp| {
                for b in &rbm.blocks {
                    pp.spawn((
                        Mesh3d(b.mesh.clone()),
                        b.material_component(),
                        Transform::IDENTITY,
                    ));
                }
            });
        }
    });
    entity
        .entry::<Vehicle>()
        .and_modify(move |mut v| v.collider_children = collider_children);
    let id = entity.id();
    info!("{}: spawned at {start:?}, {} parts", spec.name, parts.len());
    id
}

/// The chassis collider: the game's own shape when it decoded, else the convex hull
/// of the body's render meshes. Triangle meshes in the Havok body come back separately
/// (parry allows no mesh inside a compound) as child colliders.
pub fn chassis_collider(
    body: &RbmAsset,
    meshes: &Assets<Mesh>,
    body_collision: Option<&CollisionBody>,
) -> (Collider, Vec<(Vec3, Quat, Collider)>) {
    // chassis: convex hull of the body's render meshes
    let mut points: Vec<Vec3> = Vec::new();
    for b in &body.blocks {
        if let Some(m) = meshes.get(&b.mesh)
            && let Some(p) = m
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
        {
            points.extend(p.iter().map(|v| Vec3::from(*v)));
        }
    }
    let (lo, hi) = (body.bbox_min, body.bbox_max);
    // the game's own shape when it decoded, else the hull of the render mesh; a Havok
    // body's triangle meshes become child colliders
    let mut extra_colliders: Vec<(Vec3, Quat, Collider)> = Vec::new();
    let collider = body_collision
        .and_then(|b| {
            let mut parts = colliders_from_body(b);
            if parts.is_empty() {
                return None;
            }
            let (p, r, c) = parts.remove(0);
            extra_colliders = parts;
            Some(
                if p.length() < 1e-5 && r.abs_diff_eq(Quat::IDENTITY, 1e-5) {
                    c
                } else if c.shape().as_composite_shape().is_some() {
                    extra_colliders.insert(0, (p, r, c));
                    Collider::convex_hull(points.clone()).unwrap_or_else(|| Collider::sphere(0.5))
                } else {
                    Collider::compound(vec![(Position(p), Rotation(r), c)])
                },
            )
        })
        .or_else(|| Collider::convex_hull(points.clone()))
        .unwrap_or_else(|| Collider::cuboid(hi.x - lo.x, hi.y - lo.y, hi.z - lo.z));
    (collider, extra_colliders)
}

/// Angular inertia of a collider's shape scaled to a given total mass. (Avian's
/// `AngularInertia::from_shape(collider, mass)` hands the mass to parry as a density, which
/// for a car-sized hull is a hundred times too much.)
pub fn inertia_for_mass(collider: &Collider, mass: f32) -> AngularInertia {
    let props = collider.shape_scaled().mass_properties(1.0);
    let unit_mass = props.mass().max(1e-6);
    let principal = props.principal_inertia() * (mass / unit_mass);
    let frame: Quat = props.principal_inertia_local_frame;
    AngularInertia::new_with_local_frame(principal, frame)
}

#[allow(clippy::type_complexity)]
fn drive_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mode: Res<CameraMode>,
    mut cars: Query<(
        &mut Vehicle,
        &mut Transform,
        &mut LinearVelocity,
        &mut AngularVelocity,
        Has<Occupied>,
        Has<AiDriven>,
    )>,
) {
    for (mut car, mut tf, mut lin, mut ang, occupied, ai) in &mut cars {
        if ai && !occupied {
            // the traffic AI has the pedals (`traffic.rs`)
            continue;
        }
        if *mode != CameraMode::Drive || !occupied {
            // the keys belong to Rico, another vehicle or the free camera; coast
            car.throttle = 0.0;
            car.brake = 0.0;
            car.steer_input = 0.0;
            car.handbrake = false;
            continue;
        }
        drive_one(&keys, &time, &mut car, &mut tf, &mut lin, &mut ang);
    }
}

/// `tf` stays a `Mut`: it is only written when `R` rights the car. Borrowing it mutably
/// every frame marks it changed, and Avian then copies the (interpolated, a frame
/// behind) `Transform` back onto the body's `Position` — the car was dragged back each
/// frame you drove it and only reached its speed once you got out.
fn drive_one(
    keys: &ButtonInput<KeyCode>,
    time: &Time,
    car: &mut Vehicle,
    tf: &mut Mut<Transform>,
    lin: &mut LinearVelocity,
    ang: &mut AngularVelocity,
) {
    let dt = time.delta_secs();
    let fwd = keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp);
    let back = keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown);
    let left = keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft);
    let right = keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight);
    // S brakes while rolling forward and reverses once (nearly) stopped
    let moving_forward = car.speed > 0.5;
    let moving_back = car.speed < -0.5;
    let (throttle, brake) = match (fwd, back) {
        (true, false) if moving_back => (0.0, 1.0),
        (true, false) => (1.0, 0.0),
        (false, true) if moving_forward => (0.0, 1.0),
        (false, true) => (-1.0, 0.0),
        _ => (0.0, 0.0),
    };
    let rate = 3.0 * dt;
    car.throttle += (throttle - car.throttle).clamp(-rate, rate);
    car.brake += (brake - car.brake).clamp(-rate * 2.0, rate * 2.0);
    let steer_target = match (left, right) {
        (true, false) => 1.0,
        (false, true) => -1.0,
        _ => 0.0,
    };
    let steer_rate = (if steer_target == 0.0 { 4.0 } else { 2.5 }) * dt;
    car.steer_input += (steer_target - car.steer_input).clamp(-steer_rate, steer_rate);
    car.handbrake = keys.pressed(KeyCode::Space);
    if keys.just_pressed(KeyCode::KeyR) {
        // back on the wheels, a little above where it is
        let yaw = tf.rotation.to_euler(EulerRot::YXZ).0;
        tf.translation += Vec3::Y * 1.5;
        tf.rotation = Quat::from_rotation_y(yaw);
        lin.0 = Vec3::ZERO;
        ang.0 = Vec3::ZERO;
        for w in &mut car.wheels {
            w.compression = 0.0;
        }
    }
}

pub(crate) fn vehicle_physics(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut cars: Query<(Entity, &mut Vehicle, Forces)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (entity, mut car, mut forces) in &mut cars {
        let pos = forces.position().0;
        let rot = forces.rotation().0;
        let up = rot * Vec3::Y;
        let forward = forward_of(rot);
        let vel = forces.linear_velocity();
        let speed = vel.dot(forward);
        car.speed = speed;
        let spec = car.spec.clone();
        let g = 9.81;

        // steering lock shrinks with speed
        let lock = spec.max_steer * (spec.steer_speed / speed.abs().max(spec.steer_speed));
        let steer = car.steer_input * lock;

        // engine: automatic gearbox picked from the wheel speed
        let radius = spec.wheels.first().map(|w| w.radius).unwrap_or(0.35);
        let wheel_rps = speed.abs() / (std::f32::consts::TAU * radius);
        let rpm_for = |ratio: f32| wheel_rps * 60.0 * ratio * spec.final_drive;
        let mut gear = 0;
        for (i, ratio) in spec.gears.iter().enumerate() {
            gear = i;
            if rpm_for(*ratio) < spec.rpm_max * 0.9 {
                break;
            }
        }
        let reversing = car.throttle < 0.0 && speed < 1.0;
        let ratio = if reversing {
            spec.reverse
        } else {
            spec.gears[gear]
        };
        let rpm = rpm_for(ratio).clamp(spec.rpm_min, spec.rpm_max);
        car.gear = gear;
        car.rpm = rpm;
        let mut drive_total =
            spec.torque_at(rpm) * ratio * spec.final_drive / radius * car.throttle.abs();
        if reversing {
            drive_total = -drive_total;
        } else if car.throttle < 0.0 {
            drive_total = 0.0;
        }
        if speed.abs() > spec.top_speed {
            drive_total = 0.0;
        }
        let driven = spec.wheels.iter().filter(|w| w.drives).count().max(1) as f32;
        let (throttle, brake, handbrake) = (car.throttle, car.brake, car.handbrake);

        let filter = SpatialQueryFilter::from_excluded_entities(
            std::iter::once(entity).chain(car.collider_children.iter().copied()),
        );
        let mut grounded = 0;
        for i in 0..spec.wheels.len() {
            let w = &spec.wheels[i];
            let rest_offset = REST_COMPRESSION * w.travel;
            let origin = pos + rot * (w.hub + Vec3::Y * rest_offset);
            let max = w.travel + w.radius;
            let hit = spatial.cast_ray(
                origin,
                Dir3::new(-up).unwrap_or(Dir3::NEG_Y),
                max,
                true,
                &filter,
            );
            let state = &mut car.wheels[i];
            let Some(hit) = hit else {
                state.grounded = false;
                state.compression = 0.0;
                state.load = 0.0;
                state.hub = w.hub + Vec3::Y * (rest_offset - w.travel);
                state.steer = if w.steers { steer } else { 0.0 };
                continue;
            };
            grounded += 1;
            let d = hit.distance.clamp(w.radius, max);
            let compression = (max - d).clamp(0.0, w.travel);
            let comp_rate = (compression - state.compression) / dt;
            state.compression = compression;
            state.grounded = true;
            state.hub = w.hub + Vec3::Y * (rest_offset - (d - w.radius));
            state.steer = if w.steers { steer } else { 0.0 };
            let contact = origin - up * d;
            let damping = if comp_rate > 0.0 {
                w.damp_compress
            } else {
                w.damp_rebound
            };
            let load = (w.spring * compression + damping * comp_rate).max(0.0);
            state.load = load;
            forces.apply_force_at_point(up * load, contact);

            // tyre forces in the contact plane
            let n = hit.normal.normalize_or(up);
            let steer_rot = Quat::from_axis_angle(up, state.steer);
            let wheel_fwd = (steer_rot * forward).reject_from(n).normalize_or(forward);
            // (the wheel's left: the lateral force's sign does not depend on it)
            let wheel_side = n.cross(wheel_fwd).normalize_or(-(rot * MODEL_RIGHT));
            let v = forces.velocity_at_point(contact);
            let v_f = v.dot(wheel_fwd);
            let v_s = v.dot(wheel_side);
            let mu = w.friction;
            let grip = if handbrake && w.handbrake { 0.55 } else { 1.0 };
            let max_f = mu * load * grip;
            let mut f_long = 0.0;
            if w.drives {
                f_long += drive_total / driven;
            }
            if brake > 0.0 {
                f_long -= (v_f / 0.5).clamp(-1.0, 1.0) * w.brake_force * brake;
            }
            if handbrake && w.handbrake {
                f_long -= (v_f / 0.5).clamp(-1.0, 1.0) * max_f;
            }
            // rolling resistance and engine braking
            if throttle == 0.0 || throttle.signum() != v_f.signum() {
                f_long -= (v_f / 0.3).clamp(-1.0, 1.0) * ROLLING_RESISTANCE * load * 2.0;
            } else {
                f_long -= (v_f / 0.3).clamp(-1.0, 1.0) * ROLLING_RESISTANCE * load;
            }
            let v_sat = (SLIP_ANGLE_SATURATION * v_f.abs()).max(LATERAL_SATURATION_MIN);
            let mut f_lat = -(v_s / v_sat).clamp(-1.0, 1.0) * max_f;
            // friction circle
            let total = (f_long * f_long + f_lat * f_lat).sqrt();
            if total > max_f && total > 0.0 {
                let k = max_f / total;
                f_long *= k;
                f_lat *= k;
            }
            forces.apply_force_at_point(wheel_fwd * f_long + wheel_side * f_lat, contact);
            state.force = Vec2::new(f_long, f_lat);
            state.spin += v_f * dt / w.radius;
        }
        // air drag on the chassis
        let v_len = vel.length();
        if v_len > 0.1 {
            forces.apply_force(-vel * v_len * spec.drag);
        }
        // a little self-righting torque when airborne keeps rolls short
        if grounded == 0 {
            let tilt = up.cross(Vec3::Y);
            forces.apply_torque(tilt * spec.mass * g * 0.05);
        }
    }
}

fn wheel_visuals(
    cars: Query<(&Vehicle, &Children)>,
    mut wheels: Query<(&WheelVisual, &mut Transform)>,
) {
    for (car, children) in &cars {
        for child in children.iter() {
            let Ok((wv, mut tf)) = wheels.get_mut(child) else {
                continue;
            };
            let Some(spec) = car.spec.wheels.get(wv.0) else {
                continue;
            };
            let Some(state) = car.wheels.get(wv.0) else {
                continue;
            };
            tf.translation = state.hub;
            // a positive steer turns left (about +y); rolling forward (towards −z) turns
            // the top of the tyre forward, a negative turn about x
            tf.rotation = Quat::from_rotation_y(state.steer)
                * Quat::from_rotation_x(-state.spin)
                * spec.base_rotation;
            tf.scale = spec.scale;
        }
    }
}

type PlayerCar<'w, 's> =
    Query<'w, 's, (&'static Transform, &'static Vehicle), (With<Occupied>, Without<FlyCamera>)>;

fn chase_camera(
    time: Res<Time>,
    mode: Res<CameraMode>,
    cars: PlayerCar,
    mut cams: Query<(&mut Transform, &mut FlyCamera)>,
) {
    if *mode != CameraMode::Drive {
        return;
    }
    let Ok((car_tf, car)) = cars.single() else {
        return;
    };
    let Ok((mut cam_tf, mut fly)) = cams.single_mut() else {
        return;
    };
    let forward = forward_of(car_tf.rotation);
    let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(MODEL_FORWARD);
    let dist = (car.length * 1.8).max(6.0);
    let target_pos = car_tf.translation - flat * dist + Vec3::Y * (car.length * 0.35).max(2.0);
    let look_at = car_tf.translation + Vec3::Y * 1.2;
    let k = 1.0 - (-6.0 * time.delta_secs()).exp();
    cam_tf.translation = cam_tf.translation.lerp(target_pos, k);
    cam_tf.look_at(look_at, Vec3::Y);
    // keep the fly camera's idea of where it looks in sync for when the user takes over
    let f = cam_tf.forward().as_vec3();
    fly.yaw = (-f.x).atan2(-f.z);
    fly.pitch = f.y.asin();
}

fn drive_hud(
    mode: Res<CameraMode>,
    cars: Query<&Vehicle, With<Occupied>>,
    mut q: Query<&mut Text, With<DriveHud>>,
) {
    let car = cars.single().ok();
    for mut t in &mut q {
        let Some(car) = car.filter(|_| *mode == CameraMode::Drive) else {
            **t = String::new();
            continue;
        };
        let grounded = car.wheels.iter().filter(|w| w.grounded).count();
        **t = format!(
            "{}   {:5.0} km/h   gear {}   {:4.0} rpm   {}/{} wheels down{}{}\n\
             W/S throttle · brake/reverse   A/D steer   Space handbrake   E get out   V free camera   R upright",
            car.spec.name,
            car.speed.abs() * 3.6,
            if car.throttle < 0.0 && car.speed < 1.0 {
                "R".to_string()
            } else {
                (car.gear + 1).to_string()
            },
            car.rpm,
            grounded,
            car.wheels.len(),
            if car.handbrake { "   HANDBRAKE" } else { "" },
            if car.brake > 0.0 { "   braking" } else { "" },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Headless drive: the bus from the test data settles on a flat tile, drives off
    /// under full throttle, and turns left when steered. Needs JC2_TEST_DIR with
    /// `lave.v046_buscoach.ee` reachable.
    #[test]
    fn bus_settles_drives_and_turns() {
        use crate::fleet::{Place, testing};
        // the physics without the keys: the test drives
        let Some((mut app, ground, _)) = testing::app(&["lave.v046_buscoach.ee"], |app| {
            app.add_systems(FixedUpdate, vehicle_physics);
        }) else {
            return;
        };
        let car = testing::spawn(
            &mut app,
            "lave.v046_buscoach.ee",
            Place::NearStart(Vec3::ZERO),
        );
        let wheels = app.world().get::<Vehicle>(car).unwrap().wheels.len();
        assert_eq!(wheels, 6);

        // settle
        for _ in 0..128 {
            app.update();
        }
        let tf = *app.world().get::<Transform>(car).unwrap();
        let v = app.world().get::<Vehicle>(car).unwrap();
        let grounded = v.wheels.iter().filter(|w| w.grounded).count();
        assert_eq!(grounded, 6, "wheels down after settling");
        let up = tf.rotation * Vec3::Y;
        assert!(up.y > 0.98, "upright: {up:?}");
        assert!(
            tf.translation.y > ground && tf.translation.y < ground + 2.0,
            "{}",
            tf.translation.y - ground
        );
        assert!(v.speed.abs() < 0.5, "still: {}", v.speed);
        let start = tf.translation;

        // full throttle for 4 s
        for _ in 0..256 {
            app.world_mut().get_mut::<Vehicle>(car).unwrap().throttle = 1.0;
            app.update();
        }
        let tf = *app.world().get::<Transform>(car).unwrap();
        let v = app.world().get::<Vehicle>(car).unwrap();
        assert!(v.speed > 4.0, "speed {}", v.speed);
        // forward is −z (the nose, `coords::MODEL_FORWARD`)
        assert!(
            start.z - tf.translation.z > 8.0,
            "moved {:?}",
            tf.translation - start
        );
        let up = tf.rotation * Vec3::Y;
        assert!(up.y > 0.95, "upright while driving: {up:?}");

        // steer left (positive) for 3 s: heading swings from −z towards −x
        for i in 0..192 {
            let mut car_mut = app.world_mut().get_mut::<Vehicle>(car).unwrap();
            car_mut.throttle = 0.6;
            car_mut.steer_input = 1.0;
            app.update();
            if i % 64 == 0 {
                let v = app.world().get::<Vehicle>(car).unwrap();
                eprintln!(
                    "t={:.1} speed {:.1} m/s yaw rate {:.3} rad/s",
                    i as f32 / 64.0,
                    v.speed,
                    app.world().get::<AngularVelocity>(car).unwrap().0.y
                );
            }
        }
        let tf = *app.world().get::<Transform>(car).unwrap();
        let nose = forward_of(tf.rotation);
        assert!(nose.x < -0.3, "turned: nose {nose:?}");
        let up = tf.rotation * Vec3::Y;
        assert!(up.y > 0.9, "upright after turning: {up:?}");
    }

    /// The jeep, driven with the keys: `W` and then `W` + `A`. Seen from the chase camera
    /// behind it (a Bevy camera looking along the nose, `forward × y` on its right), the
    /// car turns to the LEFT and ends up left of where it was heading. Needs JC2_TEST_DIR
    /// with `lave.v014_jeep.ee`.
    #[test]
    fn a_car_steered_left_turns_left() {
        use crate::fleet::{Place, testing};
        let Some((mut app, _, _)) = testing::app(&["lave.v014_jeep.ee"], |app| {
            app.add_systems(Update, drive_input)
                .add_systems(FixedUpdate, vehicle_physics);
        }) else {
            return;
        };
        let car = testing::spawn(&mut app, "lave.v014_jeep.ee", Place::NearStart(Vec3::ZERO));
        assert!(app.world().get::<Occupied>(car).is_some());
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Drive;
        let drive = |app: &mut App, keys: &[KeyCode], frames: usize| {
            for _ in 0..frames {
                {
                    let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                    k.reset_all();
                    for key in keys {
                        k.press(*key);
                    }
                }
                app.update();
            }
        };
        drive(&mut app, &[], 96);
        drive(&mut app, &[KeyCode::KeyW], 128);
        let tf0 = *app.world().get::<Transform>(car).unwrap();
        let f0 = forward_of(tf0.rotation);
        let cam = Transform::from_translation(tf0.translation - f0 * 8.0 + Vec3::Y * 3.0)
            .looking_to(Vec3::new(f0.x, 0.0, f0.z), Vec3::Y);
        let left = -cam.right().as_vec3();
        assert!(app.world().get::<Vehicle>(car).unwrap().speed > 3.0);
        drive(&mut app, &[KeyCode::KeyW, KeyCode::KeyA], 128);
        let v = app.world().get::<Vehicle>(car).unwrap();
        assert!(v.steer_input > 0.9, "A steers positive: {}", v.steer_input);
        let tf1 = *app.world().get::<Transform>(car).unwrap();
        let nose = forward_of(tf1.rotation);
        let moved = tf1.translation - tf0.translation;
        eprintln!("jeep: nose {f0:?} → {nose:?}, moved {moved:?}, left of the camera {left:?}");
        assert!(
            nose.dot(left) > 0.3,
            "the nose did not swing left: {nose:?}"
        );
        assert!(moved.dot(left) > 2.0, "it did not go left: {moved:?}");
        // and the wheels turned left with it
        assert!(v.wheels.iter().any(|w| w.steer > 0.1));
    }

    /// Driving must not fight the physics: with frames shorter than the physics step
    /// (Transform interpolated, a fraction of a step behind `Position`) the jeep covers
    /// the same ground in the same time as with one step per frame. It used to lose its
    /// momentum every frame while driven. Needs JC2_TEST_DIR with `lave.v014_jeep.ee`.
    #[test]
    fn driving_keeps_its_momentum_between_frames() {
        use crate::fleet::{Place, testing};
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        fn distance(frame_hz: f64, secs: f64) -> Option<f32> {
            let (mut app, _, _) = testing::app(&["lave.v014_jeep.ee"], |app| {
                app.add_systems(Update, drive_input)
                    .add_systems(FixedUpdate, vehicle_physics);
            })?;
            let car = testing::spawn(&mut app, "lave.v014_jeep.ee", Place::NearStart(Vec3::ZERO));
            *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Drive;
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / frame_hz,
            )));
            let run = |app: &mut App, keys: &[KeyCode], secs: f64| {
                for _ in 0..(secs * frame_hz).round() as usize {
                    {
                        let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
                        k.reset_all();
                        for key in keys {
                            k.press(*key);
                        }
                    }
                    app.update();
                }
            };
            run(&mut app, &[], 1.5);
            let p0 = app.world().get::<Position>(car)?.0;
            run(&mut app, &[KeyCode::KeyW], secs);
            let p1 = app.world().get::<Position>(car)?.0;
            Some((p1 - p0).with_y(0.0).length())
        }
        let Some(steady) = distance(64.0, 4.0) else {
            return;
        };
        let fast = distance(100.0, 4.0).unwrap();
        eprintln!("jeep, 4 s of throttle: {steady:.1} m at 64 fps, {fast:.1} m at 100 fps");
        assert!(steady > 10.0, "the jeep barely moved: {steady}");
        assert!(
            fast > steady * 0.85,
            "driving at 100 fps lost momentum: {fast:.1} m vs {steady:.1} m"
        );
    }
}
