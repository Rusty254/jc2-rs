//! Boats: a `seve.*` vehicle doll floated on the doll's own buoyancy spheres and pushed
//! by its propeller (`jc2_formats::vehicle::Boat`).
//!
//! The float model is the native one (JC2-216, `research/formats/water-buoyancy.md`, driver
//! `FUN_009893c0`, per sphere `FUN_009a5c60`, set-up `pfxSeaVehicle`): every `CBoyancyModule`
//! position carries a sphere of the **one** radius of module 0; its submerged cap volume
//! against the CPU **wave height** at the sphere ([`crate::sea_wave`]) lifts with
//! `boyancy * V * (bias + 1) * 9820` and meets a per-axis quadratic water drag scaled by the
//! wetted segment; the hull's mass is `1100 * submerged volume + dry mass` every step with the
//! inertia scaled by the same ratio and the vertical speed reduced by the mass ratio; a wrecked
//! hull (health ratio 0 or less) loses `0.2` of the bias per second down to `-0.9`.
//!
//! **Not native (stand-ins, still open as research Gaps):** the propeller thrust, rudder, a
//! top-speed hull drag and the engine/steering modules (`pfxBoatEngine` is not decoded), the horizontal-velocity kill
//! byte of `pfxSeaVehicle` (+0x2c), the Havok body damping (taken from the doll's
//! `linear_damping`/`angular_damping`), the ray-cast water plane of body key 4, and the
//! restore paths of the sinking bias.
//!
//! `jc2 --world 31,14 --boat [seve.v083_motorboat_small.ee]` moors one on the nearest
//! open water (from the global height map); in it `W`/`S` throttle and reverse,
//! `A`/`D` rudder, `E` get out.

use crate::combat::Health;
use crate::coords::{MODEL_FORWARD, forward_of, mat_to_bevy, to_bevy};
use crate::rbm_loader::RbmAsset;
use crate::sea_wave::{SeaWaves, sea_waves_step};
use crate::vehicle::{AiDriven, CameraMode, Enterable, Occupied, PendingPart, chassis_collider};
use crate::world::{FlyCamera, SEA_LEVEL};
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::collision::CollisionBody;
use jc2_formats::terrain::GlobalTerrain;
use jc2_formats::vehicle::Boat;
use std::f32::consts::PI;

/// `FUN_009a5c60`: the lift scale, read as 1000 kg/m^3 x 9.82 (the split is inferred).
pub const LIFT_SCALE: f32 = 9820.0;
/// `FUN_009893c0`: the mass of water carried along per m^3 submerged.
pub const CARRIED_WATER: f32 = 1100.0;
/// `CSeaVehicle` (0x627610): the bias falls by this per second of frame time while the health
/// ratio is 0 or less, down to [`SINK_FLOOR`].
pub const SINK_RATE: f32 = 0.2;
pub const SINK_FLOOR: f32 = -0.9;
/// `FUN_009a5c60`: the drag is skipped when the point velocity's square is below this.
const DRAG_MIN_SPEED_SQ: f32 = 0.0001;
/// Non-native stand-in (the engine action is undecoded): a boat pushing its thrust tops out
/// near this speed (m/s), as the earlier runtime tuned it.
const TOP_SPEED: f32 = 22.0;

#[derive(Component, Debug, Clone)]
pub struct Watercraft {
    pub params: Boat,
    pub name: String,
    /// Inputs −1..1.
    pub throttle: f32,
    pub rudder: f32,
    /// Buoyancy sphere centres in the body frame (Bevy axes).
    pub floats: Vec<Vec3>,
    /// Where the propeller pushes, body frame.
    pub prop: Vec3,
    pub length: f32,
    /// How many spheres touch the water (for the HUD and the tests).
    pub wet: usize,
    /// The hkBoyancy block's buoyancy bias B (+0x74): the lift scale is `B + 1`.
    pub bias: f32,
    /// The summed submerged volume of the last step (+0x34), m^3.
    pub volume: f32,
    /// The mass used on the previous step (+0x3c); the dry mass at the start (inferred).
    pub last_mass: f32,
}

#[derive(Component)]
struct BoatHud;

pub struct BoatPlugin;

impl Plugin for BoatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .init_resource::<SeaWaves>()
            .add_systems(Startup, spawn_boat_hud)
            .add_systems(Update, (boat_input, boat_camera, boat_hud))
            .add_systems(FixedUpdate, (sea_waves_step, boat_physics).chain());
    }
}

/// What one buoyancy sphere of radius `r` has under the water when its centre is `dist` above the
/// surface (`FUN_009a5c60`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SphereWet {
    /// Submerged volume, m^3.
    pub volume: f32,
    /// The lift application offset above the centre (`c`; the sign is Gap 2 of the research).
    pub offset: f32,
    /// The wetted area for the drag, m^2 (as decompiled: the segment of the cap of height `h`,
    /// also for a centre below the water, Gap 4).
    pub area: f32,
}

pub fn sphere_wet(r: f32, dist: f32) -> Option<SphereWet> {
    if dist >= r {
        return None;
    }
    let sphere = 4.0 * PI / 3.0 * r * r * r;
    if dist < -r {
        return Some(SphereWet {
            volume: sphere,
            offset: 0.0,
            area: PI * r * r,
        });
    }
    let h = if dist >= 0.0 { r - dist } else { r + dist };
    let cap = PI / 3.0 * h * h * (3.0 * r - h);
    let volume = if dist >= 0.0 { cap } else { sphere - cap };
    let offset = 3.0 * (2.0 * r - h) * (2.0 * r - h) / (4.0 * (3.0 * r - h));
    let area =
        r * r * ((r - h) / r).clamp(-1.0, 1.0).acos() - (r - h) * (2.0 * r * h - h * h).max(0.0).sqrt();
    Some(SphereWet {
        volume,
        offset,
        area,
    })
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
    let length = body.bbox_max.z - body.bbox_min.z;
    let id = spawn_hull(commands, params, name, collider, length, water, rotation);
    let start = Vec3::new(water.x, SEA_LEVEL + 0.3, water.z);
    let mut entity = commands.entity(id);
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
            let mut part =
                parent.spawn((tf, Visibility::default(), Name::new(pp.part.name.clone())));
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
    info!("{name}: afloat at {start:?}");
    id
}

/// The hull body of a boat: rigid body, native mass and inertia, the float state. Parts and
/// extra colliders are added by [`build_boat`]; the tests spawn it alone for hulls whose
/// models the game files do not hold.
pub fn spawn_hull(
    commands: &mut Commands,
    params: &Boat,
    name: &str,
    collider: Collider,
    length: f32,
    water: Vec3,
    rotation: Quat,
) -> Entity {
    let p = params.clone();
    let floats: Vec<Vec3> = p.floats.iter().map(|(f, _)| to_bevy(*f)).collect();
    let prop = to_bevy(p.rudder);
    let start = Vec3::new(water.x, SEA_LEVEL + 0.3, water.z);
    // `pfxSeaVehicle` set-up (FUN_009ad7c0): the inertia is diag(factors) * mass
    let inertia = AngularInertia::new(Vec3::from(p.inertia_factors) * p.mass);
    commands
        .spawn((
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
            // Havok body damping from the doll (inferred as the source of `linear_damping`)
            LinearDamping(p.linear_damping),
            AngularDamping(p.angular_damping),
            SleepingDisabled,
            TransformInterpolation,
            Transform::from_translation(start).with_rotation(rotation),
            Visibility::default(),
            Watercraft {
                last_mass: p.mass,
                params: p,
                name: name.to_string(),
                throttle: 0.0,
                rudder: 0.0,
                floats,
                prop,
                length,
                wet: 0,
                bias: 0.0,
                volume: 0.0,
            },
            Enterable {
                reach: length * 0.35,
            },
            Name::new(format!("boat {name}")),
        ))
        .id()
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

/// Buoyancy, water drag, the added-mass update, the sinking bias and the propeller, one fixed
/// step (`FUN_009893c0` over every sphere, then `pfxSeaVehicle`'s own drag).
#[allow(clippy::type_complexity)]
pub fn boat_physics(
    time: Res<Time>,
    waves: Res<SeaWaves>,
    mut boats: Query<(
        &mut Watercraft,
        Forces,
        &mut Mass,
        &mut AngularInertia,
        Option<&Health>,
    )>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (mut boat, mut forces, mut mass, mut inertia, health) in &mut boats {
        let pos = forces.position().0;
        let rot = forces.rotation().0;
        let p = boat.params.clone();
        let hull = &p.hull;
        let r = hull.radius;
        let axes = [rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z];
        // CSeaVehicle: a wrecked hull loses lift (health ratio 0 or less; inferred reading)
        if health.is_some_and(|h| h.hp <= 0.0) {
            boat.bias = (boat.bias - SINK_RATE * dt).max(SINK_FLOOR);
        }
        let scale = p.buoyancy * (boat.bias + 1.0) * LIFT_SCALE;
        let mut volume = 0.0;
        let mut wet = 0;
        let floats = boat.floats.clone();
        for f in &floats {
            let centre = pos + rot * *f;
            let surface = waves.height(centre.x, centre.z, pos);
            let Some(s) = sphere_wet(r, centre.y - surface) else {
                continue;
            };
            wet += 1;
            volume += s.volume;
            // the lift, along +y (the ray-cast plane normal of body key 4 is Gap 1)
            forces.apply_force_at_point(
                Vec3::Y * (scale * s.volume),
                centre + Vec3::Y * s.offset,
            );
            // per-axis quadratic drag of the point's velocity, scaled by the wetted segment
            let v = forces.velocity_at_point(centre);
            if v.length_squared() >= DRAG_MIN_SPEED_SQ {
                let local = Vec3::new(
                    v.dot(axes[0]),
                    v.dot(axes[1]).clamp(-hull.up_velocity_cap, hull.up_velocity_cap),
                    v.dot(axes[2]),
                );
                let k = -0.5 * hull.drag * s.area;
                let coefficient = [hull.side_area, hull.bottom_area, 1.0];
                let mut force = Vec3::ZERO;
                for i in 0..3 {
                    force += axes[i] * (k * coefficient[i] * local[i].abs() * local[i]);
                }
                forces.apply_force_at_point(force, centre);
            }
        }
        boat.wet = wet;
        boat.volume = volume;
        // the added-mass rule: water carried along, the inertia follows, a plunging hull slows
        let dry = p.mass;
        let m = CARRIED_WATER * volume + dry;
        if (mass.0 - m).abs() > 1e-3 {
            mass.0 = m;
            *inertia = AngularInertia::new(Vec3::from(p.inertia_factors) * m);
        }
        let ratio = (boat.last_mass / m).min(1.0);
        if ratio < 1.0 {
            forces.linear_velocity_mut().y *= ratio;
        }
        boat.last_mass = m;
        // pfxSeaVehicle: the whole hull's quadratic drag at the centre of mass
        let v = forces.linear_velocity();
        forces.apply_force(-(p.drag * p.drag_area * v.length()) * v);

        // --- stand-in (not native, the engine action is undecoded): the propeller at the
        // rudder, turned with it, only while it is in the water. It pushes at the height of
        // the centre of mass: below it, the push would stand the hull on its tail long before
        // planing lifted the bow
        let fwd = forward_of(rot);
        let flat_fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(MODEL_FORWARD);
        // stand-in top speed: with the thrust below the sphere drag alone lets a hull reach
        // ~40 m/s, so a quadratic hull drag keeps the thrust topping out near TOP_SPEED
        if wet > 0 {
            let along = forces.linear_velocity().dot(flat_fwd);
            let wet_share = wet as f32 / floats.len().max(1) as f32;
            forces.apply_force(
                -flat_fwd * along * along.abs() * (p.thrust / (TOP_SPEED * TOP_SPEED)) * wet_share,
            );
        }
        let prop_world = pos + rot * boat.prop;
        let com_y = to_bevy(p.center_of_mass).y;
        let push_at = pos + rot * Vec3::new(boat.prop.x, com_y, boat.prop.z);
        if waves.height(prop_world.x, prop_world.z, pos) - prop_world.y > -0.3 {
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
    use crate::fleet::{Place, testing};

    /// The 14 retail boats (`seve.*`): the four `v011` hulls, then one per class.
    const BOATS: [&str; 14] = [
        "seve.v011_charter.ee",
        "seve.v011_houseboat.ee",
        "seve.v011_sailboat.ee",
        "seve.v011_yacht.ee",
        "seve.v082_motorboat_standard_small.ee",
        "seve.v083_motorboat_small.ee",
        "seve.v086_barge.ee",
        "seve.v087_catamaran_speedboat.ee",
        "seve.v088_djonk.ee",
        "seve.v089_raceboat.ee",
        "seve.v095_swamp_boat.ee",
        "seve.v098_indo.ee",
        "seve.v104_attack_boat.ee",
        "seve.v105_military_boat.ee",
    ];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4 * b.abs().max(1.0)
    }

    /// The cap formulas of `FUN_009a5c60`: nothing when the centre is a radius above the water,
    /// the whole sphere a radius below, a hemisphere (centroid `3r/8` from the centre, wetted
    /// area `pi r^2 / 2`) with the centre in the surface, and no jump anywhere between.
    #[test]
    fn sphere_cap_formulas() {
        let r = 0.7;
        let sphere = 4.0 * PI / 3.0 * r * r * r;
        assert_eq!(sphere_wet(r, r), None);
        assert_eq!(sphere_wet(r, r + 3.0), None);
        let below = sphere_wet(r, -r - 0.01).unwrap();
        assert!(close(below.volume, sphere) && below.offset == 0.0 && close(below.area, PI * r * r));
        let half = sphere_wet(r, 0.0).unwrap();
        assert!(close(half.volume, sphere / 2.0), "{half:?}");
        assert!(close(half.offset, 3.0 * r / 8.0), "{half:?}");
        assert!(close(half.area, PI * r * r / 2.0), "{half:?}");
        // volume grows steadily as the centre sinks; no jump at the two ends
        let mut last = 0.0;
        for i in 0..=2000 {
            let dist = r - i as f32 * (2.0 * r) / 2000.0;
            let v = sphere_wet(r, dist).map_or(0.0, |s| s.volume);
            assert!(v >= last - 1e-6 && v - last < 0.003, "volume {v} after {last} at {dist}");
            last = v;
        }
        assert!(close(last, sphere));
    }

    /// Open sea at the corner of the map, as far as the global height map goes.
    fn open_sea(app: &App) -> (f32, f32) {
        let fs = app.world().resource::<Game>().fs.clone();
        let map = GlobalTerrain::parse(&fs.read_inflated("terrain_win32.dat").unwrap()).unwrap();
        find_open_water(&map, -15000.0, -15000.0, 500.0, 3.0).unwrap()
    }

    fn fixed_steps(app: &mut App, n: usize) {
        for _ in 0..n {
            app.update();
        }
    }

    /// Every retail boat settles on a flat sea at the draft the formulas give (the immersed
    /// volume `m g / (boyancy 9820 - 1100 g)`, mean of 3 s), level and at rest, then a wrecked
    /// hull (health 0) loses its bias at 0.2 per second to -0.9 and goes down. Needs
    /// JC2_TEST_DIR with the 14 `seve.*` containers and `terrain_win32.dat`.
    #[test]
    fn retail_boats_float_and_a_wreck_sinks() {
        let mut needs: Vec<&str> = BOATS.to_vec();
        needs.push("terrain_win32.dat");
        let Some((mut app, _, _)) = testing::app(&needs, |app| {
            app.add_plugins(BoatPlugin);
        }) else {
            return;
        };
        let (wx, wz) = open_sea(&app);
        // a flat sea for the draft check: a camera far above keeps the wave gate shut
        app.world_mut().resource_mut::<SeaWaves>().camera = Some(Vec3::new(0.0, 400.0, 0.0));
        let mut ids = Vec::new();
        for (i, name) in BOATS.iter().enumerate() {
            let at = Vec3::new(wx + 90.0 * (i % 5) as f32, SEA_LEVEL, wz + 90.0 * (i / 5) as f32);
            if *name == "seve.v095_swamp_boat.ee" {
                // the retail container holds the doll only (no models): the hull alone
                let fs = app.world().resource::<Game>().fs.clone();
                let data = fs.read_inflated(name).unwrap();
                let sarc = jc2_formats::sarc::SmallArchive::parse(&data).unwrap();
                let e = sarc
                    .entries
                    .iter()
                    .find(|e| e.name.to_ascii_lowercase().ends_with("vdoll"))
                    .unwrap();
                let doll =
                    jc2_formats::vehicle::VehicleDoll::parse(sarc.data(&data, e).unwrap()).unwrap();
                let id = {
                    let mut commands = app.world_mut().commands();
                    spawn_hull(
                        &mut commands,
                        &doll.boat.unwrap(),
                        name,
                        Collider::cuboid(1.8, 0.8, 4.0),
                        4.0,
                        at,
                        Quat::IDENTITY,
                    )
                };
                app.world_mut().flush();
                ids.push(id);
                continue;
            }
            ids.push(testing::spawn(
                &mut app,
                name,
                Place::At(Transform::from_translation(at)),
            ));
        }
        // the spawn waits dozens of frames per boat: the early ones have already settled
        fixed_steps(&mut app, 64 * 25);
        let mut volumes = vec![0.0f32; ids.len()];
        for _ in 0..192 {
            app.update();
            for (v, id) in volumes.iter_mut().zip(&ids) {
                *v += app.world().get::<Watercraft>(*id).unwrap().volume / 192.0;
            }
        }
        for ((name, id), mean) in BOATS.iter().zip(&ids).zip(&volumes) {
            let b = app.world().get::<Watercraft>(*id).unwrap();
            let tf = app.world().get::<Transform>(*id).unwrap();
            let v = app.world().get::<LinearVelocity>(*id).unwrap().0;
            let (boyancy, dry) = (b.params.buoyancy, b.params.mass);
            let expect = dry * 9.81 / (boyancy * LIFT_SCALE - CARRIED_WATER * 9.81);
            assert!(
                (mean - expect).abs() < 0.12 * expect,
                "{name}: immersed {mean:.3} m3, formulas say {expect:.3}"
            );
            assert!(
                (tf.translation.y - SEA_LEVEL).abs() < 2.5,
                "{name}: at {:.2}",
                tf.translation.y - SEA_LEVEL
            );
            assert!(v.length() < 0.6, "{name}: still moving {v:?}");
            assert!((tf.rotation * Vec3::Y).y > 0.9, "{name}: listing");
            assert!(b.wet > 0 && b.bias == 0.0);
            eprintln!(
                "{name:42} spheres {:2} wet {} immersed {mean:5.2} m3 (formulas {expect:5.2}) y {:+.2}",
                b.floats.len(),
                b.wet,
                tf.translation.y - SEA_LEVEL
            );
        }
        // wrecks
        for id in &ids {
            app.world_mut()
                .entity_mut(*id)
                .insert(Health { hp: 0.0, max: 100.0 });
        }
        fixed_steps(&mut app, 64 * 40);
        for (name, id) in BOATS.iter().zip(&ids) {
            let b = app.world().get::<Watercraft>(*id).unwrap();
            let y = app.world().get::<Transform>(*id).unwrap().translation.y;
            assert!((b.bias + 0.9).abs() < 1e-3, "{name}: bias {}", b.bias);
            assert!(y < SEA_LEVEL - 3.0, "{name}: wreck still at {:+.2}", y - SEA_LEVEL);
        }
    }

    /// The motorboat from the test data floats level at the surface, runs ahead on the
    /// throttle and turns on the rudder. Needs JC2_TEST_DIR with
    /// `seve.v083_motorboat_small.ee` and `terrain_win32.dat` reachable.
    #[test]
    fn motorboat_floats_runs_and_turns() {
        let Some((mut app, _, _)) = testing::app(
            &["seve.v083_motorboat_small.ee", "terrain_win32.dat"],
            |app| {
                app.add_plugins(BoatPlugin);
            },
        ) else {
            return;
        };
        let (wx, wz) = open_sea(&app);
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
            (tf.translation.y - SEA_LEVEL).abs() < 1.0 && v.length() < 1.0,
            "floating at {} moving {v:?}",
            tf.translation.y
        );
        assert!(up.y > 0.95, "listing: up {up:?}");
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
        assert!(ran > 60.0, "only ran {ran} m");
        assert!(v1.length() > 8.0, "speed {}", v1.length());
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
            turned > 0.5 && turned < 6.0,
            "turned {turned} rad in 3.75 s"
        );
        let wet = app.world().get::<Watercraft>(id).unwrap().wet;
        eprintln!(
            "boat: settled at {:+.2} m, {ran:.0} m in 10 s ({:.0} km/h), turned {:.0}Â° in 4 s, {wet} spheres wet",
            tf.translation.y - SEA_LEVEL,
            v1.length() * 3.6,
            turned.abs().to_degrees()
        );
    }

    /// On the CPU waves (camera at the sea level, so the gate is open) a boat rides them: its
    /// height follows the wave under its spheres within the draft, over 20 s. Prints the wave
    /// amplitude and the boat's heave.
    #[test]
    fn boat_rides_the_waves() {
        let Some((mut app, _, _)) = testing::app(
            &["seve.v083_motorboat_small.ee", "terrain_win32.dat"],
            |app| {
                app.add_plugins(BoatPlugin);
            },
        ) else {
            return;
        };
        let (wx, wz) = open_sea(&app);
        app.world_mut().resource_mut::<SeaWaves>().camera = Some(Vec3::new(wx, SEA_LEVEL + 3.0, wz));
        let id = testing::spawn(
            &mut app,
            "seve.v083_motorboat_small.ee",
            Place::At(Transform::from_translation(crate::coords::bevy_xyz(
                wx, SEA_LEVEL, wz,
            ))),
        );
        fixed_steps(&mut app, 64 * 6);
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let (mut wave_lo, mut wave_hi) = (f32::MAX, f32::MIN);
        for _ in 0..64 * 20 {
            app.update();
            let tf = *app.world().get::<Transform>(id).unwrap();
            let waves = app.world().resource::<SeaWaves>().clone();
            let w = waves.height(tf.translation.x, tf.translation.z, tf.translation);
            lo = lo.min(tf.translation.y);
            hi = hi.max(tf.translation.y);
            wave_lo = wave_lo.min(w);
            wave_hi = wave_hi.max(w);
            assert!(
                (tf.translation.y - w).abs() < 1.5,
                "boat {:+.2} against wave {:+.2}",
                tf.translation.y - SEA_LEVEL,
                w - SEA_LEVEL
            );
        }
        eprintln!(
            "waves under the boat {:+.2}..{:+.2} m, boat heave {:+.2}..{:+.2} m",
            wave_lo - SEA_LEVEL,
            wave_hi - SEA_LEVEL,
            lo - SEA_LEVEL,
            hi - SEA_LEVEL
        );
        assert!(hi - lo > 0.01, "the boat does not move with the waves");
    }
}
