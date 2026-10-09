//! Boats: a `seve.*` vehicle doll floated on the doll's own buoyancy points and pushed
//! by its propeller (`jc2_formats::vehicle::Boat`). Each `CBoyancyModule` point lifts in
//! proportion to how deep it sits below the surface (y = 200), so the hull settles,
//! pitches and rolls on its own; the hull resists sideways motion like a keel, the
//! thrust acts at the rudder and turns with it.
//!
//! `jc2 --world 31,14 --boat [seve.v083_motorboat_small.ee]` moors one on the nearest
//! open water (from the global height map); in it `W`/`S` throttle and reverse,
//! `A`/`D` rudder, `E` get out.

use crate::coords::{MODEL_FORWARD, forward_of, mat_to_bevy, to_bevy};
use crate::rbm_loader::RbmAsset;
use crate::vehicle::{
    AiDriven, CameraMode, Enterable, Occupied, PendingPart, chassis_collider, inertia_for_mass,
};
use crate::world::{FlyCamera, SEA_LEVEL};
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::collision::CollisionBody;
use jc2_formats::terrain::GlobalTerrain;
use jc2_formats::vehicle::Boat;

const G: f32 = 9.81;
/// Depth (m) at which a buoyancy point gives its full share.
const FULL_DEPTH: f32 = 0.25;
/// Vertical damping of a submerged point, per kg of the boat's share (1/s).
const HEAVE_DAMPING: f32 = 2.5;
/// Sideways water resistance (1/s): how quickly the keel kills a skid.
const KEEL: f32 = 2.0;
/// How hard the water resists the hull turning (1/s, times the hull's yaw inertia).
const YAW_DRAG: f32 = 10.0;
/// Hull drag such that a boat pushing its thrust tops out near this speed (m/s).
const TOP_SPEED: f32 = 22.0;

#[derive(Component, Debug, Clone)]
pub struct Watercraft {
    pub params: Boat,
    pub name: String,
    /// Inputs −1..1.
    pub throttle: f32,
    pub rudder: f32,
    /// Buoyancy points in the body frame (Bevy axes).
    pub floats: Vec<Vec3>,
    /// Where the propeller pushes, body frame.
    pub prop: Vec3,
    pub length: f32,
    /// How many points are in the water (for the HUD and the tests).
    pub wet: usize,
}

#[derive(Component)]
struct BoatHud;

pub struct BoatPlugin;

impl Plugin for BoatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .add_systems(Startup, spawn_boat_hud)
            .add_systems(Update, (boat_input, boat_camera, boat_hud))
            .add_systems(FixedUpdate, boat_physics);
    }
}

/// The nearest open water to `(x, z)` (engine axes), searched on rings 32 m apart out to
/// `radius`: a point with the sea floor at least `depth` down, and the same within 12 m
/// around it.
pub fn find_open_water(
    map: &GlobalTerrain,
    x: f32,
    z: f32,
    radius: f32,
    depth: f32,
) -> Option<(f32, f32)> {
    let deep = |x: f32, z: f32| map.height_at(x, z) < SEA_LEVEL - depth;
    let clear = |x: f32, z: f32| {
        deep(x, z)
            && (0..8).all(|i| {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                deep(x + 12.0 * a.cos(), z + 12.0 * a.sin())
            })
    };
    if clear(x, z) {
        return Some((x, z));
    }
    let mut r = 32.0;
    while r <= radius {
        let n = ((std::f32::consts::TAU * r) / 24.0).ceil() as usize;
        for i in 0..n {
            let a = i as f32 * std::f32::consts::TAU / n as f32;
            let (px, pz) = (x + r * a.cos(), z + r * a.sin());
            if clear(px, pz) {
                return Some((px, pz));
            }
        }
        r += 32.0;
    }
    None
}

fn spawn_boat_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(16.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            bottom: Val::Px(10.0),
            ..default()
        },
        BoatHud,
        crate::hud::DevText,
    ));
}

/// Spawn a boat with its parts afloat at `water` (only x and z are used), facing
/// `rotation`.
#[allow(clippy::too_many_arguments)]
pub fn build_boat(
    commands: &mut Commands,
    params: &Boat,
    name: &str,
    body: &RbmAsset,
    meshes: &Assets<Mesh>,
    rbms: &Assets<RbmAsset>,
    parts: &[PendingPart],
    body_collision: Option<&CollisionBody>,
    water: Vec3,
    rotation: Quat,
) -> Entity {
    let (collider, extra) = chassis_collider(body, meshes, body_collision);
    let p = params.clone();
    let (lo, hi) = (body.bbox_min, body.bbox_max);
    let length = hi.z - lo.z;
    let floats: Vec<Vec3> = p.floats.iter().map(|(f, _)| to_bevy(*f)).collect();
    let prop = to_bevy(p.rudder);
    let start = Vec3::new(water.x, SEA_LEVEL + 0.3, water.z);
    let inertia = inertia_for_mass(&collider, p.mass);
    let mut entity = commands.spawn((
        RigidBody::Dynamic,
        collider,
        (
            Mass(p.mass),
            inertia,
            CenterOfMass(to_bevy(p.center_of_mass)),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
        ),
        LinearDamping(0.05),
        AngularDamping(p.angular_damping.max(0.5)),
        SleepingDisabled,
        TransformInterpolation,
        Transform::from_translation(start).with_rotation(rotation),
        Visibility::default(),
        Watercraft {
            params: p,
            name: name.to_string(),
            throttle: 0.0,
            rudder: 0.0,
            floats,
            prop,
            length,
            wet: 0,
        },
        Enterable {
            reach: length * 0.35,
        },
        Name::new(format!("boat {name}")),
    ));
    entity.with_children(|parent| {
        for (p, r, c) in extra {
            parent.spawn((
                c,
                Transform {
                    translation: p,
                    rotation: r,
                    scale: Vec3::ONE,
                },
            ));
        }
        for pp in parts {
            let Some(rbm) = rbms.get(&pp.handle) else {
                continue;
            };
            let mut tf = mat_to_bevy(&pp.part.world);
            if let Some(sc) = pp.part.scale {
                tf.scale *= Vec3::from(sc);
            }
            parent
                .spawn((tf, Visibility::default(), Name::new(pp.part.name.clone())))
                .with_children(|c| {
                    for b in &rbm.blocks {
                        c.spawn((
                            Mesh3d(b.mesh.clone()),
                            b.material_component(),
                            Transform::IDENTITY,
                        ));
                    }
                });
        }
    });
    let id = entity.id();
    info!("{name}: afloat at {start:?}");
    id
}

fn boat_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mode: Res<CameraMode>,
    mut boats: Query<(&mut Watercraft, Has<Occupied>, Has<AiDriven>)>,
) {
    let dt = time.delta_secs();
    for (mut b, occupied, ai) in &mut boats {
        if ai && !occupied {
            // a skipper has the helm (`crew.rs`)
            continue;
        }
        let (t, r) = if *mode == CameraMode::Drive && occupied {
            let axis = |p: KeyCode, p2: KeyCode, n: KeyCode, n2: KeyCode| {
                (keys.pressed(p) || keys.pressed(p2)) as i32 as f32
                    - (keys.pressed(n) || keys.pressed(n2)) as i32 as f32
            };
            (
                axis(
                    KeyCode::KeyW,
                    KeyCode::ArrowUp,
                    KeyCode::KeyS,
                    KeyCode::ArrowDown,
                ),
                axis(
                    KeyCode::KeyA,
                    KeyCode::ArrowLeft,
                    KeyCode::KeyD,
                    KeyCode::ArrowRight,
                ),
            )
        } else {
            (0.0, 0.0)
        };
        b.throttle += (t - b.throttle).clamp(-2.0 * dt, 2.0 * dt);
        b.rudder += (r - b.rudder).clamp(-3.0 * dt, 3.0 * dt);
    }
}

/// Buoyancy, hull drag and the propeller, one fixed step.
pub fn boat_physics(time: Res<Time>, mut boats: Query<(&mut Watercraft, Forces)>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut boat, mut forces) in &mut boats {
        let pos = forces.position().0;
        let rot = forces.rotation().0;
        let p = boat.params.clone();
        let n = boat.floats.len().max(1) as f32;
        // each point carries buoyancy × its share of the weight at full depth
        let share = p.buoyancy * p.mass * G / n;
        let mut wet = 0;
        let floats = boat.floats.clone();
        for f in &floats {
            let world = pos + rot * *f;
            let depth = SEA_LEVEL - world.y;
            if depth <= 0.0 {
                continue;
            }
            wet += 1;
            let k = (depth / FULL_DEPTH).min(1.0);
            let v = forces.velocity_at_point(world);
            let lift = share * k - v.y * HEAVE_DAMPING * p.mass / n * k;
            forces.apply_force_at_point(Vec3::Y * lift, world);
        }
        boat.wet = wet;
        if wet == 0 {
            continue;
        }
        let wet_share = wet as f32 / n;
        let fwd = forward_of(rot);
        let flat_fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(MODEL_FORWARD);
        let side = Vec3::Y.cross(flat_fwd);
        let v = forces.linear_velocity();
        let along = v.dot(flat_fwd);
        let across = v.dot(side);
        // the keel: sideways motion dies, forward motion meets quadratic drag
        let drag_k = p.thrust / (TOP_SPEED * TOP_SPEED);
        let hull = -side * across * KEEL * p.mass * wet_share
            - flat_fwd * along * along.abs() * drag_k * wet_share;
        forces.apply_force(hull);
        // the hull resists turning: yaw damping on a slender-hull inertia (m·L²/12)
        let yaw_inertia = p.mass * boat.length * boat.length / 12.0;
        let w = forces.angular_velocity();
        forces.apply_torque(Vec3::Y * (-w.y * YAW_DRAG * yaw_inertia * wet_share));
        // the propeller, at the rudder, turned with it (only while it is in the water).
        // It pushes at the height of the centre of mass: below it, the push would
        // stand the hull on its tail long before planing lifted the bow
        let prop_world = pos + rot * boat.prop;
        let com_y = to_bevy(p.center_of_mass).y;
        let push_at = pos + rot * Vec3::new(boat.prop.x, com_y, boat.prop.z);
        if SEA_LEVEL - prop_world.y > -0.3 {
            let thrust = if boat.throttle >= 0.0 {
                p.thrust * boat.throttle
            } else {
                p.reverse_thrust * boat.throttle
            };
            let angle = -boat.rudder * p.max_steer_deg.to_radians();
            let dir = Quat::from_axis_angle(Vec3::Y, angle) * flat_fwd;
            forces.apply_force_at_point(dir * thrust, push_at);
        }
    }
}

type OccupiedBoat<'w, 's> =
    Query<'w, 's, (&'static Transform, &'static Watercraft), (With<Occupied>, Without<FlyCamera>)>;

fn boat_camera(
    time: Res<Time>,
    mode: Res<CameraMode>,
    boats: OccupiedBoat,
    mut cams: Query<(&mut Transform, &mut FlyCamera)>,
) {
    if *mode != CameraMode::Drive {
        return;
    }
    let Ok((tf, b)) = boats.single() else { return };
    let Ok((mut cam, mut fly)) = cams.single_mut() else {
        return;
    };
    let fwd = forward_of(tf.rotation);
    let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(MODEL_FORWARD);
    let dist = (b.length * 1.8).max(8.0);
    let target = tf.translation - flat * dist + Vec3::Y * (b.length * 0.4).max(3.0);
    let k = 1.0 - (-5.0 * time.delta_secs()).exp();
    cam.translation = cam.translation.lerp(target, k);
    cam.look_at(tf.translation + Vec3::Y * 1.0, Vec3::Y);
    let f = cam.forward().as_vec3();
    fly.yaw = (-f.x).atan2(-f.z);
    fly.pitch = f.y.asin();
}

fn boat_hud(
    mode: Res<CameraMode>,
    boats: Query<(&Watercraft, &LinearVelocity, Has<Occupied>)>,
    mut q: Query<&mut Text, With<BoatHud>>,
) {
    let Ok((b, lin, occupied)) = boats.single() else {
        return;
    };
    for mut t in &mut q {
        if *mode != CameraMode::Drive || !occupied {
            **t = String::new();
            continue;
        }
        **t = format!(
            "{}   {:4.0} km/h   throttle {:+4.0}%   {}/{} floats wet\n\
             W/S throttle · reverse   A/D rudder   E get out   V free camera",
            b.name,
            Vec3::new(lin.x, 0.0, lin.z).length() * 3.6,
            b.throttle * 100.0,
            b.wet,
            b.floats.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Game;

    /// The motorboat from the test data floats level at the surface, runs ahead on the
    /// throttle and turns on the rudder. Needs JC2_TEST_DIR with
    /// `seve.v083_motorboat_small.ee` and `terrain_win32.dat` reachable.
    #[test]
    fn motorboat_floats_runs_and_turns() {
        use crate::fleet::{Place, testing};
        let Some((mut app, _, _)) = testing::app(
            &["seve.v083_motorboat_small.ee", "terrain_win32.dat"],
            |app| {
                app.add_plugins(BoatPlugin);
            },
        ) else {
            return;
        };
        let fs = app.world().resource::<Game>().fs.clone();
        let map = GlobalTerrain::parse(&fs.read_inflated("terrain_win32.dat").unwrap()).unwrap();
        // the corner of the map is open sea
        let (wx, wz) = find_open_water(&map, -15000.0, -15000.0, 500.0, 3.0).unwrap();
        let id = testing::spawn(
            &mut app,
            "seve.v083_motorboat_small.ee",
            Place::At(Transform::from_translation(crate::coords::bevy_xyz(
                wx, SEA_LEVEL, wz,
            ))),
        );
        for _ in 0..600 {
            app.update();
        }
        let tf = *app.world().get::<Transform>(id).unwrap();
        let v = app.world().get::<LinearVelocity>(id).unwrap().0;
        let up = tf.rotation * Vec3::Y;
        assert!(
            (tf.translation.y - SEA_LEVEL).abs() < 1.0 && v.length() < 0.5,
            "floating at {} moving {v:?}",
            tf.translation.y
        );
        assert!(up.y > 0.97, "listing: up {up:?}");
        let p0 = tf.translation;
        let fwd0 = forward_of(tf.rotation);
        // full throttle for 10 s
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        for _ in 0..600 {
            app.update();
        }
        let tf1 = *app.world().get::<Transform>(id).unwrap();
        let v1 = app.world().get::<LinearVelocity>(id).unwrap().0;
        let ran = (tf1.translation - p0).dot(fwd0);
        assert!(ran > 80.0, "only ran {ran} m");
        assert!(v1.length() > 12.0, "speed {}", v1.length());
        assert!(
            (tf1.translation.y - SEA_LEVEL).abs() < 2.0,
            "flew or sank: {}",
            tf1.translation.y
        );
        // rudder left for 4 s
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyA);
        let mut turned = 0.0f32;
        for _ in 0..240 {
            app.update();
            turned += app.world().get::<AngularVelocity>(id).unwrap().0.y / 64.0;
        }
        // left (A) turns the bow to its left (anticlockwise from above): a positive yaw

        assert!(
            turned > 0.8 && turned < 5.0,
            "turned {turned} rad in 3.75 s"
        );
        let wet = app.world().get::<Watercraft>(id).unwrap().wet;
        eprintln!(
            "boat: settled at {:+.2} m, {ran:.0} m in 10 s ({:.0} km/h), turned {:.0}° in 4 s, {wet} floats wet",
            tf.translation.y - SEA_LEVEL,
            v1.length() * 3.6,
            turned.abs().to_degrees()
        );
    }
}
