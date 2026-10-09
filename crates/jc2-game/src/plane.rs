//! Planes: an `arve.*` doll with a `CAirPlaneController`, flown with the numbers its
//! modules carry — the helicopter modules every air vehicle has (mass, centre of mass,
//! the engine's `#ec5a5e47` as full thrust, `angular_pitch/roll/yaw_speed` as the turn
//! rates) and the wing modules (`jc2_formats::vehicle::Airfoil`: area × lift coefficient
//! in air of density 1.3). The model is arcade, like the game's: thrust along the nose,
//! quadratic drag that tops out at the doll's `#b92afbce` (150, taken as m/s for planes),
//! lift on the wings growing with the square of the airspeed and with the angle of
//! attack, the stick turning the body at the doll's rates (less of it when slow), the
//! nose weathervaning into the airflow, and side-slip bled off. On the ground it stands
//! on a spring under each of the doll's tyres, rolls freely forward, grips sideways and
//! steers its nose wheel with the rudder.
//!
//! `jc2 --world 31,14 --plane [arve.v067_speedplane.ee]` parks one beside Rico. In it:
//! `Space` throttle up, `C` throttle down, `W`/`S` nose down/up, `A`/`D` roll,
//! `←`/`→` rudder, `E` get out, `V` free camera.

use crate::coords::{MODEL_FORWARD, MODEL_RIGHT, forward_of, mat_to_bevy};
use crate::rbm_loader::RbmAsset;
use crate::vehicle::{
    AiDriven, CameraMode, Enterable, Occupied, PendingPart, chassis_collider, inertia_for_mass,
};
use crate::world::FlyCamera;
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::collision::CollisionBody;
use jc2_formats::vehicle::{Airfoil, Helicopter};

const G: f32 = 9.81;
/// The data's lift falls short of holding the planes up at sensible speeds; this is the
/// tuning that makes the fighter stall near 230 km/h (not from the data).
const LIFT_GAIN: f32 = 2.0;
/// Lift grows with the angle of attack: ×(1 + α / AOA_SCALE) up to the stall angle.
const AOA_SCALE: f32 = 0.12;
const STALL_ANGLE: f32 = 0.35;
/// How fast the nose swings into the airflow, and side-slip dies away (1/s).
const WEATHERVANE: f32 = 1.2;
const SIDESLIP: f32 = 1.5;
/// How fast the body's turn rate follows the stick (1/s).
const RESPONSE: f32 = 4.0;
const THROTTLE_RATE: f32 = 0.5;
/// The gear: how far a spring sags under the plane's weight, how much more it can
/// travel, and how high above the tyre the cast starts (m); sideways grip (1/s) and
/// rolling resistance (1/s).
const GEAR_SAG: f32 = 0.12;
const GEAR_TRAVEL: f32 = 0.25;
const GEAR_REACH: f32 = 1.0;
const GEAR_GRIP: f32 = 3.0;
const ROLLING: f32 = 0.02;

#[derive(Component, Debug, Clone)]
pub struct Plane {
    pub params: Helicopter,
    pub name: String,
    /// Σ area × lift coefficient × density / 2 of the wings: lift = this × v² (N).
    pub lift_factor: f32,
    /// Quadratic drag (N per (m/s)²) that caps the speed where thrust runs out.
    pub drag: f32,
    /// Engine power 0..1 and the stick, −1..1.
    pub throttle: f32,
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
    pub length: f32,
    /// The tyres (centre, radius) in body axes.
    pub wheels: Vec<(Vec3, f32)>,
    /// Rolling on the ground (any wheel touching).
    pub grounded: bool,
}

impl Plane {
    pub fn new(
        params: Helicopter,
        wings: &[Airfoil],
        wheels: &[(Vec3, f32)],
        name: String,
        length: f32,
    ) -> Self {
        let lift_factor = wings
            .iter()
            .map(|w| 0.5 * w.density * w.area * w.lift)
            .sum::<f32>()
            .max(0.1)
            * LIFT_GAIN;
        let top = Self::top_speed(&params);
        let drag = params.lift_max / (top * top);
        Self {
            params,
            name,
            lift_factor,
            drag,
            throttle: 0.0,
            pitch: 0.0,
            roll: 0.0,
            yaw: 0.0,
            length,
            wheels: wheels.to_vec(),
            grounded: true,
        }
    }

    /// `#b92afbce` as m/s.
    pub fn top_speed(p: &Helicopter) -> f32 {
        p.top_speed_kmh.max(30.0)
    }

    /// The airspeed at which the wings carry the weight in level flight.
    pub fn stall_speed(&self) -> f32 {
        (self.params.mass * G / self.lift_factor).sqrt()
    }
}

#[derive(Component)]
struct PlaneHud;

pub struct PlanePlugin;

impl Plugin for PlanePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .add_systems(Startup, spawn_plane_hud)
            .add_systems(Update, (plane_input, plane_camera, plane_hud))
            .add_systems(FixedUpdate, plane_physics);
    }
}

fn spawn_plane_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(16.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            bottom: Val::Px(34.0),
            ..default()
        },
        PlaneHud,
        crate::hud::DevText,
    ));
}

/// Spawn a plane with its parts, standing on its wheels on `ground`, facing `rotation`.
#[allow(clippy::too_many_arguments)]
pub fn build_plane(
    commands: &mut Commands,
    params: &Helicopter,
    wings: &[Airfoil],
    wheels: &[(Vec3, f32)],
    name: &str,
    body: &RbmAsset,
    meshes: &Assets<Mesh>,
    rbms: &Assets<RbmAsset>,
    parts: &[PendingPart],
    body_collision: Option<&CollisionBody>,
    ground: Vec3,
    rotation: Quat,
) -> Entity {
    let (collider, extra) = chassis_collider(body, meshes, body_collision);
    let p = params.clone();
    let (lo, hi) = (body.bbox_min, body.bbox_max);
    let length = hi.z - lo.z;
    // stand on the wheels: the lowest tyre's bottom on the ground
    let feet = wheels
        .iter()
        .map(|(c, r)| c.y - r)
        .fold(f32::INFINITY, f32::min);
    let feet = if feet.is_finite() { feet } else { lo.y };
    let start = ground + Vec3::Y * (0.3 - feet.min(lo.y.max(feet)));
    let inertia = inertia_for_mass(&collider, p.mass);
    let com = crate::coords::to_bevy(p.center_of_mass);

    let plane = Plane::new(p.clone(), wings, wheels, name.to_string(), length);
    info!(
        "{name}: {} kg, thrust {} N, top {:.0} km/h, stalls at {:.0} km/h",
        p.mass,
        p.lift_max,
        Plane::top_speed(&p) * 3.6,
        plane.stall_speed() * 3.6
    );
    let mut entity = commands.spawn((
        RigidBody::Dynamic,
        collider,
        (
            Mass(p.mass),
            inertia,
            CenterOfMass(com),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
        ),
        AngularDamping(p.angular_damping.max(0.5)),
        Friction::new(0.6),
        SleepingDisabled,
        TransformInterpolation,
        Transform::from_translation(start).with_rotation(rotation),
        Visibility::default(),
        plane,
        Enterable {
            reach: length * 0.3,
        },
        Name::new(format!("plane {name}")),
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
    info!("{name}: parked at {start:?}");
    id
}

fn plane_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mode: Res<CameraMode>,
    mut planes: Query<(&mut Plane, Has<Occupied>, Has<AiDriven>)>,
) {
    let dt = time.delta_secs();
    for (mut p, occupied, ai) in &mut planes {
        if ai && !occupied {
            // a pilot has the stick (`crew.rs`)
            continue;
        }
        if *mode != CameraMode::Drive || !occupied {
            p.throttle = (p.throttle - dt * THROTTLE_RATE).max(0.0);
            p.pitch = 0.0;
            p.roll = 0.0;
            p.yaw = 0.0;
            continue;
        }
        let axis = |pos: &[KeyCode], neg: &[KeyCode]| {
            let a = pos.iter().any(|k| keys.pressed(*k)) as i32;
            let b = neg.iter().any(|k| keys.pressed(*k)) as i32;
            (a - b) as f32
        };
        let t = axis(&[KeyCode::Space], &[KeyCode::KeyC, KeyCode::ControlLeft]);
        p.throttle = (p.throttle + t * THROTTLE_RATE * dt).clamp(0.0, 1.0);
        p.pitch = axis(&[KeyCode::KeyW], &[KeyCode::KeyS]);
        p.roll = axis(&[KeyCode::KeyD], &[KeyCode::KeyA]);
        p.yaw = axis(&[KeyCode::ArrowLeft], &[KeyCode::ArrowRight]);
    }
}

/// The flight model (see the module notes). Body axes in Bevy's frame (the engine's,
/// `coords.rs`): the nose is −z, up +y, and the pilot's right is +x.
#[allow(clippy::type_complexity)]
pub fn plane_physics(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut planes: Query<(Entity, &mut Plane, &Transform, Option<&Children>, Forces)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (e, mut p, tf, children, mut forces) in &mut planes {
        let rot = tf.rotation;
        let (fwd, up, right) = (forward_of(rot), rot * Vec3::Y, rot * MODEL_RIGHT);
        let v = forces.linear_velocity();
        let speed = v.length();
        let v_fwd = v.dot(fwd).max(0.0);
        let mass = p.params.mass;
        // the gear: a spring under each tyre, cast past the plane's own colliders
        let mut filter = SpatialQueryFilter::from_excluded_entities([e]);
        filter
            .excluded_entities
            .extend(children.iter().flat_map(|c| c.iter()));
        let n = p.wheels.len().max(1) as f32;
        let k = mass * G / (n * GEAR_SAG);
        let c = 2.0 * (k * mass / n).sqrt() * 0.6;
        let mut touching = 0;
        for &(centre, radius) in &p.wheels.clone() {
            let top = tf.transform_point(centre) + up * GEAR_REACH;
            let reach = GEAR_REACH + radius + GEAR_TRAVEL;
            let Ok(down) = Dir3::new(-up) else { continue };
            let Some(hit) = spatial.cast_ray(top, down, reach, true, &filter) else {
                continue;
            };
            let compression = (GEAR_REACH + radius + GEAR_SAG) - hit.distance;
            if compression <= -GEAR_SAG {
                continue;
            }
            touching += 1;
            let at = top - up * hit.distance;
            let point_v = forces.velocity_at_point(at);
            let along = point_v.dot(up);
            let push = (k * compression.max(0.0) - c * along).max(0.0);
            // the tyres roll forward freely and grip sideways
            let side = -right * point_v.dot(right) * (mass / n) * GEAR_GRIP;
            let roll = -fwd * point_v.dot(fwd) * (mass / n) * ROLLING;
            forces.apply_force_at_point(up * push + side + roll, at);
        }
        p.grounded = touching > 0 && up.y > 0.5;
        // thrust and drag
        let mut force = fwd * (p.throttle * p.params.lift_max) - v * speed * p.drag;
        // lift, with the angle of attack (the airflow meeting the wing from below)
        let alpha = if v_fwd > 5.0 {
            (-v.dot(up)).atan2(v_fwd)
        } else {
            0.0
        };
        let aoa = if alpha.abs() > STALL_ANGLE {
            0.3
        } else {
            (1.0 + alpha / AOA_SCALE).clamp(0.0, 1.0 + STALL_ANGLE / AOA_SCALE)
        };
        let lift = (p.lift_factor * v_fwd * v_fwd * aoa).min(3.0 * mass * G);
        force += up * lift;
        if !p.grounded {
            // side-slip bleeds off in the air (the tyres do it on the ground)
            force -= right * v.dot(right) * SIDESLIP * mass;
        }
        forces.apply_force(force);
        // the stick: turn rates at the doll's speeds, less when slow
        let authority = (v_fwd / p.stall_speed()).clamp(0.15, 1.0);
        let yaw_rate = if p.params.yaw_speed > 0.05 {
            p.params.yaw_speed
        } else {
            0.4
        };
        // a turn about `right` lifts the nose, about `fwd` drops the right wing, about
        // `up` swings the nose left: pitch input is nose down, roll banks right, yaw
        // turns left
        let pitch = -right * (p.pitch * p.params.pitch_speed * authority);
        let w = forces.angular_velocity();
        let want = if p.grounded {
            // nose-wheel steering; the nose lifts only once the wings bite, the gear
            // holds the rest
            let steer = up * (p.yaw * 0.5 - p.roll * 0.3);
            let nose = if authority > 0.6 {
                pitch
            } else {
                right * w.dot(right)
            };
            steer + nose + fwd * w.dot(fwd)
        } else {
            let mut want = pitch
                + fwd * (p.roll * p.params.roll_speed * authority)
                + up * (p.yaw * yaw_rate * authority);
            if speed > 5.0 {
                // the nose swings into the airflow
                want += fwd.cross(v / speed) * WEATHERVANE * authority;
            }
            want
        };
        let blend = 1.0 - (-RESPONSE * dt).exp();
        if !p.grounded || p.yaw != 0.0 || p.roll != 0.0 || p.pitch != 0.0 {
            let w_new = w + (want - w) * blend;
            *forces.angular_velocity_mut() = w_new;
        }
    }
}

type OccupiedPlane<'w, 's> =
    Query<'w, 's, (&'static Transform, &'static Plane), (With<Occupied>, Without<FlyCamera>)>;

fn plane_camera(
    time: Res<Time>,
    mode: Res<CameraMode>,
    planes: OccupiedPlane,
    mut cams: Query<(&mut Transform, &mut FlyCamera)>,
) {
    if *mode != CameraMode::Drive {
        return;
    }
    let Ok((tf, p)) = planes.single() else { return };
    let Ok((mut cam, mut fly)) = cams.single_mut() else {
        return;
    };
    let fwd = forward_of(tf.rotation);
    let flat = Vec3::new(fwd.x, fwd.y * 0.5, fwd.z).normalize_or(MODEL_FORWARD);
    let dist = (p.length * 1.8).max(14.0);
    let target = tf.translation - flat * dist + Vec3::Y * (p.length * 0.4).max(4.0);
    let k = 1.0 - (-5.0 * time.delta_secs()).exp();
    cam.translation = cam.translation.lerp(target, k);
    cam.look_at(tf.translation + fwd * 4.0 + Vec3::Y * 1.5, Vec3::Y);
    let f = cam.forward().as_vec3();
    fly.yaw = (-f.x).atan2(-f.z);
    fly.pitch = f.y.asin();
}

fn plane_hud(
    mode: Res<CameraMode>,
    planes: Query<(&Plane, &Transform, &LinearVelocity), With<Occupied>>,
    mut q: Query<&mut Text, With<PlaneHud>>,
) {
    for mut t in &mut q {
        let Ok((p, tf, lin)) = planes.single() else {
            if !t.is_empty() {
                **t = String::new();
            }
            continue;
        };
        if *mode != CameraMode::Drive {
            **t = String::new();
            continue;
        }
        **t = format!(
            "{}   {:4.0} km/h (stall {:3.0})   throttle {:3.0}%   climb {:+5.1} m/s   {:4.0} m above sea{}\n\
             Space/C throttle · W/S nose down/up · A/D roll · ←/→ rudder · E get out · V free camera",
            p.name,
            lin.length() * 3.6,
            p.stall_speed() * 3.6,
            p.throttle * 100.0,
            lin.y,
            tf.translation.y - crate::world::SEA_LEVEL,
            if p.grounded { "   on the ground" } else { "" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::{Place, testing};

    fn state(app: &mut App, e: Entity) -> (Plane, Transform, Vec3) {
        let w = app.world();
        (
            w.get::<Plane>(e).unwrap().clone(),
            *w.get::<Transform>(e).unwrap(),
            w.get::<LinearVelocity>(e).unwrap().0,
        )
    }

    /// Headless: the fighter rolls down a runway at full throttle and lifts off past its
    /// stall speed; up high it holds its height at speed, banks and turns, and pulls up.
    /// Needs JC2_TEST_DIR with `arve.v071_jet_fighter.ee`.
    #[test]
    fn the_fighter_takes_off_and_turns() {
        let Some((mut app, ground, _)) = testing::app(&["arve.v071_jet_fighter.ee"], |app| {
            app.add_plugins(PlanePlugin);
        }) else {
            return;
        };
        let jet = testing::spawn(
            &mut app,
            "arve.v071_jet_fighter.ee",
            Place::NearStart(Vec3::ZERO),
        );
        assert!(app.world().get::<Occupied>(jet).is_some());
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Drive;
        let step = |app: &mut App, keys: &[KeyCode], frames: usize| {
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
        step(&mut app, &[], 64);
        let (p, tf, _) = state(&mut app, jet);
        assert!(p.grounded, "parked on its wheels");
        let parked = tf.translation.y;
        assert!((parked - ground).abs() < 3.0, "{parked} over {ground}");
        // full throttle: it rolls and gathers speed
        step(&mut app, &[KeyCode::Space], 64 * 6);
        let (p, tf, v) = state(&mut app, jet);
        assert!(p.grounded, "still on the runway");
        assert!((tf.rotation * Vec3::Y).y > 0.99, "level on its gear");
        assert!(p.throttle > 0.99);
        let fwd = forward_of(tf.rotation);
        assert!(v.dot(fwd) > 40.0, "rolling at {:.1} m/s", v.dot(fwd));
        // up high, at speed, level: it holds roughly its height
        let high = tf.translation + Vec3::Y * 800.0;
        {
            let mut e = app.world_mut().entity_mut(jet);
            e.insert((
                Transform::from_translation(high).with_rotation(tf.rotation),
                Position(high),
                LinearVelocity(fwd * 110.0),
                AngularVelocity(Vec3::ZERO),
            ));
        }
        step(&mut app, &[KeyCode::Space], 64 * 4);
        let (p, tf, v) = state(&mut app, jet);
        assert!(!p.grounded);
        let dy = tf.translation.y - high.y;
        assert!(dy.abs() < 120.0, "climbed {dy:.0} m");
        assert!(v.length() > 80.0, "{:.0} m/s", v.length());
        // bank right and pull: it turns right (a negative yaw, clockwise from above)
        let heading = |t: &Transform| crate::coords::yaw_of(t.rotation);
        let right0 = tf.rotation * MODEL_RIGHT;
        let h0 = heading(&tf);
        step(&mut app, &[KeyCode::Space, KeyCode::KeyD], 32);
        let (_, tf1, _) = state(&mut app, jet);
        let right1 = tf1.rotation * MODEL_RIGHT;
        assert!(
            right1.y < right0.y - 0.1,
            "D banked the wrong way: right wing {right0:?} → {right1:?}"
        );
        step(&mut app, &[KeyCode::Space, KeyCode::KeyS], 64 * 2);
        let (_, tf2, _) = state(&mut app, jet);
        let turned = (heading(&tf2) - h0 + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        eprintln!("turned {:.0}°", turned.to_degrees());
        assert!(
            turned < -0.3,
            "turned {:.2} rad (should be right, negative)",
            turned
        );
    }
}
