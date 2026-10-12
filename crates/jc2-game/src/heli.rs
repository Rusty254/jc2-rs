//! Helicopters: an `arve.*` vehicle doll flown with the numbers its helicopter modules
//! carry (`jc2_formats::vehicle::Helicopter`). The flight model is the arcade one the
//! game plays like, not rotor aerodynamics: the collective holds, raises or lowers the
//! altitude, the body tilts up to `max_tilt` and the rotor's lift, tilted with it, pulls
//! it along, quadratic drag caps the speed at the doll's top speed, and the pedals turn
//! it at `yaw_speed`.
//!
//! `jc2 --world 31,14 --heli [arve.v062_chopper.ee]` parks one beside Rico. In it:
//! `Space` climb, `C` (or `Ctrl`) descend, `W`/`S` nose down/up, `A`/`D` turn,
//! `←`/`→` bank, `E` get out, `V` free camera.

use crate::coords::{facing_dir, mat_to_bevy, yaw_of};
use crate::rbm_loader::RbmAsset;
use crate::vehicle::{
    AiDriven, CameraMode, Enterable, Occupied, PendingPart, chassis_collider, inertia_for_mass,
};
use crate::world::FlyCamera;
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::collision::CollisionBody;
use jc2_formats::vehicle::Helicopter;

const G: f32 = 9.81;
/// Seconds for the rotor to spin up from rest (and down again).
const SPOOL_SECONDS: f32 = 4.0;
/// How hard the body is pulled to the attitude the stick asks for (1/s).
const ATTITUDE_GAIN: f32 = 3.0;
/// How quickly the vertical speed follows the collective (1/s).
const HOLD_GAIN: f32 = 1.5;
/// Vertical speeds at full collective, up and down (m/s).
const CLIMB_SPEED: f32 = 14.0;
const SINK_SPEED: f32 = 10.0;

/// A helicopter's state.
#[derive(Component, Debug, Clone)]
pub struct Heli {
    pub params: Helicopter,
    pub name: String,
    /// 0..1: how far the rotor has spun up.
    pub spool: f32,
    pub rotor_angle: f32,
    /// Heading the pedals steer (radians about +y; the nose is +z at 0).
    pub heading: f32,
    /// Inputs, −1..1.
    pub collective: f32,
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
    pub length: f32,
}

impl Heli {
    pub fn new(params: Helicopter, name: String, length: f32) -> Self {
        Self {
            params,
            name,
            spool: 0.0,
            rotor_angle: 0.0,
            heading: 0.0,
            collective: 0.0,
            pitch: 0.0,
            roll: 0.0,
            yaw: 0.0,
            length,
        }
    }

    /// The quadratic drag constant that makes full tilt top out at the top speed.
    pub fn drag(&self) -> f32 {
        let top = (self.params.top_speed_kmh / 3.6).max(10.0);
        G * self.params.max_tilt.tan() / (top * top)
    }
}

/// A rotor part: its rest transform and spin axis (main rotor: y, tail: x).
#[derive(Component)]
pub(crate) struct Rotor {
    /// Rest transform in the helicopter's frame.
    pub(crate) base: Transform,
    axis: Vec3,
    /// `Some(true)`: the blurred disc, shown when spinning; `Some(false)`: the still
    /// blades, shown when slow; `None` (the axle): always.
    blurred: Option<bool>,
    /// The tail rotor (the main rotor is the one without it).
    pub(crate) tail: bool,
}

#[derive(Component)]
struct HeliHud;

pub struct HeliPlugin;

impl Plugin for HeliPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .add_systems(Startup, spawn_heli_hud)
            .add_systems(Update, (heli_input, rotors, heli_camera, heli_hud))
            .add_systems(FixedUpdate, heli_physics);
    }
}

fn spawn_heli_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont::from_font_size(16.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            bottom: Val::Px(10.0),
            ..default()
        },
        HeliHud,
        crate::hud::DevText,
    ));
}

/// Spawn a helicopter with its parts, standing on `ground`, facing `rotation`.
#[allow(clippy::too_many_arguments)]
pub fn build_heli(
    commands: &mut Commands,
    params: &Helicopter,
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
    let start = ground + Vec3::Y * (0.6 - lo.y.min(0.0));
    let inertia = inertia_for_mass(&collider, p.mass);
    let com = crate::coords::to_bevy(p.center_of_mass);

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
        LinearDamping(p.linear_damping * 0.2),
        AngularDamping(p.angular_damping),
        Friction::new(0.9),
        SleepingDisabled,
        TransformInterpolation,
        Transform::from_translation(start).with_rotation(rotation),
        Visibility::default(),
        Heli::new(p, name.to_string(), length),
        Enterable {
            reach: length * 0.3,
        },
        Name::new(format!("helicopter {name}")),
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
            let name = pp.part.name.to_ascii_lowercase();
            let rotor = name.starts_with("rotor").then(|| Rotor {
                base: tf,
                tail: name.contains("tail"),
                axis: if name.contains("tail") {
                    Vec3::X
                } else {
                    Vec3::Y
                },
                blurred: if name.contains("axel") || name.contains("axle") {
                    None
                } else {
                    Some(name.contains("blurred"))
                },
            });
            let mut part =
                parent.spawn((tf, Visibility::default(), Name::new(pp.part.name.clone())));
            if let Some(r) = rotor {
                if r.blurred == Some(true) {
                    part.insert(Visibility::Hidden);
                }
                part.insert(r);
            }
            if let Some(l) = pp.plant_lod() {
                part.insert(l);
            }
            part.with_children(|c| {
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

fn heli_input(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mode: Res<CameraMode>,
    mut helis: Query<(&mut Heli, &Transform, Has<Occupied>, Has<AiDriven>)>,
) {
    let dt = time.delta_secs();
    for (mut h, tf, occupied, ai) in &mut helis {
        if ai && !occupied {
            // a pilot has the controls (`crew.rs`)
            continue;
        }
        if *mode != CameraMode::Drive || !occupied {
            h.collective = 0.0;
            h.pitch = 0.0;
            h.roll = 0.0;
            h.yaw = 0.0;
            // keep the heading where the body points for when someone climbs in
            h.heading = yaw_of(tf.rotation);
            continue;
        }
        let axis = |pos: &[KeyCode], neg: &[KeyCode]| {
            let p = pos.iter().any(|k| keys.pressed(*k)) as i32;
            let n = neg.iter().any(|k| keys.pressed(*k)) as i32;
            (p - n) as f32
        };
        let ease =
            |cur: f32, target: f32, rate: f32| cur + (target - cur).clamp(-rate * dt, rate * dt);
        h.collective = ease(
            h.collective,
            axis(
                &[KeyCode::Space],
                &[KeyCode::KeyC, KeyCode::ControlLeft, KeyCode::ControlRight],
            ),
            4.0,
        );
        h.pitch = ease(
            h.pitch,
            axis(
                &[KeyCode::KeyW, KeyCode::ArrowUp],
                &[KeyCode::KeyS, KeyCode::ArrowDown],
            ),
            3.0,
        );
        h.roll = ease(
            h.roll,
            axis(&[KeyCode::ArrowRight], &[KeyCode::ArrowLeft]),
            3.0,
        );
        h.yaw = ease(h.yaw, axis(&[KeyCode::KeyA], &[KeyCode::KeyD]), 4.0);
    }
}

/// The arcade flight model, one fixed step.
#[allow(clippy::type_complexity)]
pub fn heli_physics(
    time: Res<Time>,
    mut helis: Query<(
        &mut Heli,
        &Transform,
        &mut LinearVelocity,
        &mut AngularVelocity,
        Has<Occupied>,
        Has<AiDriven>,
    )>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut h, tf, mut lin, mut ang, occupied, ai) in &mut helis {
        let p = h.params.clone();
        // the rotor spins up while someone flies it (Rico or a pilot)
        let target_spool = if occupied || ai { 1.0 } else { 0.0 };
        h.spool += (target_spool - h.spool).clamp(-dt / SPOOL_SECONDS, dt / SPOOL_SECONDS);
        h.rotor_angle += h.spool * p.rotor_max_rpm / 60.0 * std::f32::consts::TAU * dt * 2.5;
        let rot = tf.rotation;
        let up = rot * Vec3::Y;
        let mut v = lin.0;

        // collective: climb, sink or hold the altitude
        let climb = (p.lift_max / p.mass - G).max(0.5);
        let sink = (p.lift_down / p.mass).max(1.0);
        let target_vy = if h.collective >= 0.0 {
            CLIMB_SPEED * h.collective
        } else {
            SINK_SPEED * h.collective
        };
        // braking a climb may drop the thrust to nothing; pushing down adds `sink`
        let floor = if target_vy < 0.0 {
            -(G + sink).min(2.0 * G)
        } else {
            -G
        };
        let want_vertical_accel = ((target_vy - v.y) * HOLD_GAIN * 1.5).clamp(floor, climb);
        // the rotor pulls along its axis; enough of it to give that vertical acceleration
        let tilt_y = up.y.max(0.4);
        let thrust = ((G + want_vertical_accel) / tilt_y).clamp(0.0, p.lift_max / p.mass) * h.spool;
        v += up * thrust * dt;
        // the extra push down when descending fast
        if want_vertical_accel < -G {
            v.y += (want_vertical_accel + G) * dt;
        }

        // drag: the top speed is where full tilt's push meets it
        let horizontal = Vec3::new(v.x, 0.0, v.z);
        let k = h.drag();
        v -= horizontal * horizontal.length() * k * dt;
        lin.0 = v;

        // attitude: the stick asks for a tilt, the pedals for a turn. The body faces −z
        // with +x on its right: a positive turn about y is to the left, nose down is a
        // negative turn about x, banking right (the right side down) a negative turn
        // about z
        if h.spool > 0.2 {
            h.heading += h.yaw * p.yaw_speed * dt;
            let target = Quat::from_rotation_y(h.heading)
                * Quat::from_rotation_x(-h.pitch * p.max_tilt)
                * Quat::from_rotation_z(-h.roll * p.max_tilt);
            let err = target * rot.inverse();
            let (axis, mut angle) = err.to_axis_angle();
            if angle > std::f32::consts::PI {
                angle -= std::f32::consts::TAU;
            }
            let want = axis * angle * ATTITUDE_GAIN * h.spool;
            let k = 1.0 - (-dt * 8.0).exp();
            ang.0 = ang.0.lerp(want, k);
        }
    }
}

fn rotors(
    helis: Query<(&Heli, &Children)>,
    mut parts: Query<(&Rotor, &mut Transform, &mut Visibility)>,
) {
    for (h, children) in &helis {
        let blurred = h.spool > 0.55;
        for child in children.iter() {
            let Ok((r, mut tf, mut vis)) = parts.get_mut(child) else {
                continue;
            };
            let speed = if r.tail { 2.7 } else { 1.0 };
            *tf = r.base;
            tf.rotation = r.base.rotation * Quat::from_axis_angle(r.axis, h.rotor_angle * speed);
            let show = r.blurred.is_none_or(|b| b == blurred);
            vis.set_if_neq(if show {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }
}

type OccupiedHeli<'w, 's> =
    Query<'w, 's, (&'static Transform, &'static Heli), (With<Occupied>, Without<FlyCamera>)>;

fn heli_camera(
    time: Res<Time>,
    mode: Res<CameraMode>,
    helis: OccupiedHeli,
    mut cams: Query<(&mut Transform, &mut FlyCamera)>,
) {
    if *mode != CameraMode::Drive {
        return;
    }
    let Ok((tf, h)) = helis.single() else { return };
    let Ok((mut cam, mut fly)) = cams.single_mut() else {
        return;
    };
    let heading = facing_dir(h.heading);
    let dist = (h.length * 1.6).max(12.0);
    let target = tf.translation - heading * dist + Vec3::Y * (h.length * 0.45).max(4.0);
    let k = 1.0 - (-4.0 * time.delta_secs()).exp();
    cam.translation = cam.translation.lerp(target, k);
    cam.look_at(tf.translation + Vec3::Y * 1.5, Vec3::Y);
    let f = cam.forward().as_vec3();
    fly.yaw = (-f.x).atan2(-f.z);
    fly.pitch = f.y.asin();
}

fn heli_hud(
    mode: Res<CameraMode>,
    helis: Query<(&Heli, &Transform, &LinearVelocity, Has<Occupied>)>,
    mut q: Query<&mut Text, With<HeliHud>>,
) {
    let Ok((h, tf, lin, occupied)) = helis.single() else {
        return;
    };
    for mut t in &mut q {
        if *mode != CameraMode::Drive || !occupied {
            **t = String::new();
            continue;
        }
        **t = format!(
            "{}   {:4.0} km/h   climb {:+5.1} m/s   {:4.0} m above sea   rotor {:3.0}%\n\
             Space climb · C descend · W/S nose down/up · A/D turn · ←/→ bank · E get out · V free camera",
            h.name,
            Vec3::new(lin.x, 0.0, lin.z).length() * 3.6,
            lin.y,
            tf.translation.y - crate::world::SEA_LEVEL,
            h.spool * 100.0
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::terrain_collider;
    use jc2_formats::terrain::TerrainTile;

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

    /// The flight model alone, on a box with the chopper's numbers: it spins up, lifts
    /// off on the collective, holds its height when let go, flies forward at about the
    /// top speed with the nose down, and turns.
    #[test]
    fn hovers_climbs_cruises_and_turns() {
        let tile = flat_tile(32000);
        let ground = tile.cell_height(0, 0);
        let (collider, tf) = terrain_collider(&tile, -512.0, -9216.0);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugins::default(),
        ))
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .add_systems(FixedUpdate, heli_physics);
        app.world_mut().spawn((RigidBody::Static, collider, tf));
        let params = Helicopter {
            mass: 10000.0,
            lift_max: 200000.0,
            lift_down: 25000.0,
            ..Helicopter::default()
        };
        let body = Collider::cuboid(3.0, 2.5, 12.0);
        let inertia = inertia_for_mass(&body, params.mass);
        let id = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                body,
                (Mass(params.mass), inertia, NoAutoMass, NoAutoAngularInertia),
                LinearDamping(0.03),
                AngularDamping(0.75),
                SleepingDisabled,
                Transform::from_xyz(-256.0, ground + 1.3, -8960.0),
                Heli::new(params, "test".into(), 12.0),
                Occupied,
            ))
            .id();
        app.finish();
        let run = |app: &mut App, secs: f32, f: &dyn Fn(&mut Heli)| {
            for _ in 0..(secs * 60.0) as usize {
                {
                    let mut h = app.world_mut().get_mut::<Heli>(id).unwrap();
                    f(&mut h);
                }
                app.update();
            }
        };
        let state = |app: &App| {
            let t = app.world().get::<Transform>(id).unwrap().translation;
            let v = app.world().get::<LinearVelocity>(id).unwrap().0;
            let h = app.world().get::<Heli>(id).unwrap().clone();
            (t, v, h)
        };
        // spin up on the ground: it stays there
        run(&mut app, 4.5, &|_| {});
        let (p0, _, h) = state(&app);
        assert!(h.spool > 0.99, "spool {}", h.spool);
        assert!(p0.y < ground + 3.0, "lifted off on its own: {}", p0.y);
        // climb for 4 s
        run(&mut app, 4.0, &|h| h.collective = 1.0);
        let (p1, v1, _) = state(&app);
        assert!(p1.y - p0.y > 30.0, "climbed {}", p1.y - p0.y);
        assert!(
            v1.y > 0.8 * CLIMB_SPEED && v1.y < 1.1 * CLIMB_SPEED,
            "climb rate {}",
            v1.y
        );
        // let go: it stops climbing and holds
        run(&mut app, 5.0, &|h| h.collective = 0.0);
        let (p2, v2, _) = state(&app);
        assert!(v2.y.abs() < 0.5, "still moving vertically: {v2:?}");
        run(&mut app, 3.0, &|h| h.collective = 0.0);
        let (p3, _, _) = state(&app);
        assert!((p3.y - p2.y).abs() < 1.5, "drifted {} m", p3.y - p2.y);
        // nose down: it flies forward (−z, the way it faces) towards the top speed
        run(&mut app, 25.0, &|h| h.pitch = 1.0);
        let (p4, v4, h4) = state(&app);
        let speed = Vec3::new(v4.x, 0.0, v4.z).length();
        let top = h4.params.top_speed_kmh / 3.6;
        assert!(v4.z < 0.0 && p4.z < p3.z - 200.0, "went {:?}", p4 - p3);
        assert!(
            speed > 0.7 * top && speed < 1.2 * top,
            "{speed} m/s vs top {top}"
        );
        assert!(
            (p4.y - p3.y).abs() < 15.0,
            "lost/gained {} m in cruise",
            p4.y - p3.y
        );
        // turn left with the pedals
        run(&mut app, 2.0, &|h| {
            h.pitch = 0.0;
            h.yaw = 1.0;
        });
        let (_, _, h5) = state(&app);
        assert!(h5.heading > 2.0, "heading {}", h5.heading);
        let r = app.world().get::<Transform>(id).unwrap().rotation;
        let body = yaw_of(r);
        let diff = (body - h5.heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        assert!(diff.abs() < 0.5, "body heading {body} vs {}", h5.heading);
        // the pedals turned it left: the nose swung from −z towards −x
        let nose = crate::coords::forward_of(r);
        assert!(h5.heading > 0.0 && nose.x < 0.0, "nose {nose:?}");
        // bank right (→): the right side (+x in the body) goes down
        run(&mut app, 1.0, &|h| {
            h.yaw = 0.0;
            h.roll = 1.0;
        });
        let r = app.world().get::<Transform>(id).unwrap().rotation;
        let right = r * crate::coords::MODEL_RIGHT;
        assert!(right.y < -0.1, "banked the wrong way: right side {right:?}");

        eprintln!(
            "heli: climbed {:.0} m in 4 s ({:.1} m/s), held {:.2} m/s, cruised {:.0} km/h (top {:.0})",
            p1.y - p0.y,
            v1.y,
            v2.y,
            speed * 3.6,
            top * 3.6
        );
    }

    /// The real chopper from the test data: it parks on the ground with its rotors
    /// found, then lifts off. Needs JC2_TEST_DIR with `arve.v062_chopper.ee` and
    /// `general.blz` (the shared rotor models) reachable.
    #[test]
    fn chopper_parks_and_lifts_off() {
        use crate::fleet::{Place, testing};
        let Some((mut app, ground, _)) = testing::app(&["arve.v062_chopper.ee"], |app| {
            app.add_plugins(HeliPlugin);
        }) else {
            return;
        };
        let id = testing::spawn(
            &mut app,
            "arve.v062_chopper.ee",
            Place::NearStart(Vec3::ZERO),
        );
        // no Rico here, so the fleet put the player in it: Drive mode, keys fly it
        assert!(app.world().get::<Occupied>(id).is_some());
        // no Rico in this app, so it is occupied from the start; let it settle and spool
        for _ in 0..300 {
            app.update();
        }
        let p0 = app.world().get::<Transform>(id).unwrap().translation;
        assert!(
            p0.y > ground && p0.y < ground + 4.0,
            "parked at {} over ground {ground}",
            p0.y
        );
        let mut rotors = app.world_mut().query::<&Rotor>();
        let n = rotors.iter(app.world()).count();
        assert!(n >= 4, "{n} rotor parts");
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        for _ in 0..240 {
            app.update();
        }
        let p1 = app.world().get::<Transform>(id).unwrap().translation;
        assert!(p1.y - p0.y > 20.0, "only rose {} m", p1.y - p0.y);
        eprintln!(
            "chopper: parked {:.2} m above the ground, {n} rotor parts, rose {:.0} m in 4 s",
            p0.y - ground,
            p1.y - p0.y
        );
    }
}
