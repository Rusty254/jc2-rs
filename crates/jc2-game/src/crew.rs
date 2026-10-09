//! Crews and hijacking: the drivers and pilots sitting in vehicles, and Rico throwing
//! them out.
//!
//! **Who drives.** Nothing in the data seats a person in a placed vehicle: the spawn
//! sets' `CVehicleComponent`s name only the container (the village's attack helicopter,
//! trucks and APC stand parked and empty), and `worldsim.bin` spawns its cars, boats
//! and planes with `worldsim.aiteam` — driven, by civilians of `character_types` (the
//! cars' `#6ffccc84`, 1 on cars, bikes, small boats and planes and 0 on buses, trucks,
//! the barge and the junk, is unexplained). `heatspawn.bin`'s units carry `num_seats`.
//! So jc2-rs puts a driver where something drives: every traffic car gets a civilian
//! (`CrewRequest(CrewKind::Civilian)`), the heat's vehicles soldiers (`heat.rs`), and
//! `--crewed` the vehicles of `--heli`/`--boat`/`--plane`/`--drive` (a soldier flies a
//! helicopter in circles over its start, a civilian cruises the boat in circles).
//!
//! **Where he sits.** The doll's `SO_Driver` smart object (`jc2_formats::vehicle::Seat`):
//! the driver is a child entity of the vehicle with his model's feet frame at that point,
//! dressed like the soldiers and civilians (`npc::request_outfit`/`npc::dress`: a
//! government grunt or elite rolled from `enemytypespawnsettings.bin`, or a generic
//! civilian), playing `mv_driving_fwd` (soldiers) or `civ_driving_fwd` (civilians).
//!
//! **Hijacking.** `E` on a crewed car or boat from the ground, or on any crewed vehicle
//! from one of its stunt positions (a helicopter or a plane only from there, as in the
//! original: "When all guards are dead, press E to pull out the pilot" — the guards
//! condition is skipped, nobody else rides along), throws the driver out — to the left
//! out of a car or a helicopter, off the stern of a boat — and seats Rico, in control.
//! The thrown driver tumbles with gravity (no physics body: a ballistic fall, turning
//! over, until a ray finds the ground or the sea), lies there and goes after
//! `LIE_SECONDS`. Every hijack counts in [`Hijackings`] (for the HUD's "Hijackings: N").

use crate::Game;
use crate::boat::Watercraft;
use crate::character::{ENTER_RANGE, HEIGHT, Hero};
use crate::coords::{MODEL_RIGHT, facing_dir, forward_of, to_bevy, yaw_of, yaw_toward};
use crate::fleet::Container;
use crate::heli::Heli;
use crate::npc::{DollCache, EnemyRoster, dress, outfit_ready, request_outfit, unit_rand};
use crate::physics::stow_passenger;
use crate::plane::Plane;
use crate::rbm_loader::RbmAsset;
use crate::rig::{Animator, Clip, Rig, RigData};
use crate::stunts::{OnStunt, StuntInput};
use crate::vehicle::{AiDriven, CameraMode, Enterable, Occupied};
use crate::world::SEA_LEVEL;
use avian3d::prelude::*;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use jc2_formats::sarc::SmallArchive;
use jc2_formats::terrain::GlobalTerrain;
use jc2_formats::vehicle::VehicleDoll;
use std::collections::HashMap;
use std::sync::Arc;

/// A thrown driver lies this long before he goes (s).
pub const LIE_SECONDS: f32 = 30.0;
/// How hard a driver is thrown out: sideways (or back off a boat) and up (m/s).
const THROW_OUT: f32 = 4.5;
const THROW_UP: f32 = 3.0;
/// The soldiers' doll when the roster can't be read, and the civilians'.
const SOLDIER_DOLL: &str = "pd_gov_base01.ee";
const CIVILIAN_DOLLS: [&str; 3] = [
    "pd_generic_male_1.ee",
    "pd_generic_female_1.ee",
    "pd_generic_male_2.ee",
];
const CIVILIAN_SKELETON: &str = "civilian_biped.bsk";
/// Where the driver sits when the doll has no `SO_Driver` (vehicle frame).
const FALLBACK_SEAT: Vec3 = Vec3::new(-0.4, 0.6, -0.4);

/// Who drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrewKind {
    /// A government soldier (the heat's vehicles, military helicopters).
    Soldier,
    /// A villager (traffic, boats).
    Civilian,
}

/// Put a driver in this vehicle (the fleet inserts it for a `Request` with a crew).
#[derive(Component, Clone, Copy, Debug)]
pub struct CrewRequest(pub CrewKind);

/// Someone drives this vehicle: `driver` is the person in the seat once dressed (a
/// child entity), `None` while his outfit loads.
#[derive(Component, Clone, Copy, Debug)]
pub struct Crew {
    pub kind: CrewKind,
    pub driver: Option<Entity>,
}

/// A person sitting in a vehicle, at `local` in its frame.
#[derive(Component, Clone, Copy, Debug)]
pub struct Seated {
    pub vehicle: Entity,
    pub local: Vec3,
}

/// A person thrown out of a vehicle: falling, then lying (`landed` counts the seconds).
#[derive(Component, Clone, Copy, Debug)]
pub struct Thrown {
    pub velocity: Vec3,
    /// The axis he turns over about, and how fast (rad/s).
    pub spin: Vec3,
    pub age: f32,
    pub landed: Option<f32>,
}

/// Vehicles Rico took from their drivers. The HUD reads it ("Hijackings: N"): `count`
/// goes up by one per hijack, `last` is the vehicle.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct Hijackings {
    pub count: u32,
    pub last: Option<Entity>,
}

/// A helicopter's pilot: it circles `center` at `radius`, `altitude` over it, turning
/// at `cruise` m/s. Hunting, the centre is Rico, and the door gun fires at him.
#[derive(Component, Clone, Debug)]
pub struct HeliPilot {
    pub center: Vec3,
    pub radius: f32,
    pub altitude: f32,
    pub cruise: f32,
    pub hunt: bool,
    /// Seconds to the next shot, and shots left in the burst.
    pub gun: f32,
    pub burst: u32,
    /// Shots fired, for the tests.
    pub shots: u32,
}

impl HeliPilot {
    pub fn circling(center: Vec3, radius: f32, altitude: f32) -> Self {
        Self {
            center,
            radius,
            altitude,
            cruise: 10.0,
            hunt: false,
            gun: 2.0,
            burst: 0,
            shots: 0,
        }
    }

    /// Circling Rico (`radius`, `altitude` over the ground under him) with the gun.
    pub fn hunting(radius: f32, altitude: f32) -> Self {
        Self {
            hunt: true,
            // (round 50 m the tilt the helicopters have holds about 10 m/s)
            cruise: 10.0,
            gun: 3.0,
            ..Self::circling(Vec3::ZERO, radius, altitude)
        }
    }
}

/// A boat's skipper: cruises in a slow circle, steering away from shallow water.
#[derive(Component, Clone, Copy, Debug)]
pub struct BoatPilot {
    pub throttle: f32,
    pub rudder: f32,
    /// Keep the same escape side until the forward corridor clears.
    avoidance: f32,
    probe_in: f32,
}

#[derive(Default)]
struct BoatGround {
    map: Option<GlobalTerrain>,
    loaded: bool,
}

/// Outfits on their way, the dolls and rigs read so far, the seats by container.
#[derive(Resource, Default)]
struct CrewLoader {
    dolls: DollCache,
    rigs: HashMap<String, Option<Arc<RigData>>>,
    seats: HashMap<String, Option<Vec3>>,
    pending: Vec<PendingCrew>,
    seed: u32,
}

struct PendingCrew {
    vehicle: Entity,
    kind: CrewKind,
    seat: Vec3,
    skeleton: String,
    models: Vec<Handle<RbmAsset>>,
}

pub struct CrewPlugin;

impl Plugin for CrewPlugin {
    fn build(&self, app: &mut App) {
        use crate::character::{hero_input, vehicle_entry};
        app.init_resource::<CrewLoader>()
            .init_resource::<Hijackings>()
            .init_resource::<CameraMode>()
            .add_systems(
                Update,
                (
                    (request_crews, seat_crews).chain(),
                    hijack_input
                        .after(hero_input)
                        .before(StuntInput)
                        .before(vehicle_entry),
                    (heli_pilots, boat_pilots, heli_guns),
                    crew_animation,
                ),
            )
            .add_systems(FixedUpdate, thrown_fall);
        if !app.is_plugin_added::<crate::rig::RigPlugin>() {
            app.add_plugins(crate::rig::RigPlugin);
        }
    }
}

/// The driver's seat of a vehicle container's doll (Bevy axes).
pub fn driver_seat(game: &Game, container: &str) -> Option<Vec3> {
    let data = game.fs.read_inflated(container).ok()?;
    let sarc = SmallArchive::parse(&data).ok()?;
    let e = sarc
        .entries
        .iter()
        .find(|e| e.name.to_ascii_lowercase().ends_with("doll"))?;
    let doll = VehicleDoll::parse(sarc.data(&data, e).ok()?).ok()?;
    doll.driver_seat().map(|s| to_bevy(s.world.translation()))
}

/// New crew requests: the vehicle is crewed from now on (hijackable at once); the
/// driver's outfit is asked for. A helicopter or boat with nobody steering it gets a
/// pilot that circles.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn request_crews(
    mut commands: Commands,
    mut loader: ResMut<CrewLoader>,
    game: Option<Res<Game>>,
    assets: Res<AssetServer>,
    roster: Option<Res<EnemyRoster>>,
    fresh: Query<
        (
            Entity,
            &CrewRequest,
            Option<&Container>,
            &Transform,
            Has<Heli>,
            Has<Watercraft>,
            Has<AiDriven>,
        ),
        Without<Occupied>,
    >,
) {
    for (vehicle, req, container, tf, heli, boat, ai) in &fresh {
        let kind = req.0;
        commands
            .entity(vehicle)
            .remove::<CrewRequest>()
            .insert(Crew { kind, driver: None });
        info!(
            "crew: a {kind:?} drives {vehicle:?} ({})",
            container.map_or("?", |c| c.0.as_str())
        );
        if !ai && heli {
            commands
                .entity(vehicle)
                .insert((HeliPilot::circling(tf.translation, 60.0, 35.0), AiDriven));
        } else if !ai && boat {
            commands.entity(vehicle).insert((
                BoatPilot {
                    throttle: 0.4,
                    rudder: 0.7,
                    avoidance: 0.0,
                    probe_in: 0.0,
                },
                AiDriven,
            ));
        }
        let Some(game) = game.as_deref() else {
            continue;
        };
        loader.seed = loader.seed.wrapping_add(0x9e37_79b9);
        let seed = loader.seed;
        let doll = match kind {
            CrewKind::Soldier => roster
                .as_deref()
                .map(|r| r.roll(unit_rand(seed), unit_rand(seed ^ 0x5bd1)).1)
                .unwrap_or_else(|| SOLDIER_DOLL.to_string()),
            CrewKind::Civilian => {
                let pick = CIVILIAN_DOLLS[(unit_rand(seed) * 3.0) as usize % 3];
                if game.fs.contains(pick) {
                    pick.to_string()
                } else {
                    CIVILIAN_DOLLS[0].to_string()
                }
            }
        };
        let seat = container
            .map(|c| {
                *loader
                    .seats
                    .entry(c.0.clone())
                    .or_insert_with(|| driver_seat(game, &c.0))
            })
            .unwrap_or(None)
            .unwrap_or(FALLBACK_SEAT);
        let Some((skeleton, models)) =
            request_outfit(game, &assets, &mut loader.dolls, &doll, seed as usize)
        else {
            continue;
        };
        loader.pending.push(PendingCrew {
            vehicle,
            kind,
            seat,
            skeleton,
            models,
        });
    }
}

/// The skeleton's rig with the clips a crew member needs (Rico's when it is his).
fn crew_rig(
    loader: &mut CrewLoader,
    game: &Game,
    hero_rig: Option<&Rig>,
    skeleton: &str,
) -> Option<Arc<RigData>> {
    match hero_rig {
        Some(r) if r.file == skeleton => Some(r.0.clone()),
        _ => loader
            .rigs
            .entry(skeleton.to_string())
            .or_insert_with(|| {
                if skeleton == CIVILIAN_SKELETON {
                    Rig::load_with(
                        &game.fs,
                        skeleton,
                        &[Clip::CivIdle, Clip::CivDrive, Clip::CivRun],
                    )
                } else {
                    Rig::load(&game.fs, skeleton)
                }
                .map(|r| r.0)
            })
            .clone(),
    }
}

/// Seat the drivers whose outfits have loaded.
#[allow(clippy::too_many_arguments)]
fn seat_crews(
    mut commands: Commands,
    mut loader: ResMut<CrewLoader>,
    game: Option<Res<Game>>,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    hero_rig: Option<Res<Rig>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    mut crews: Query<&mut Crew, Without<Occupied>>,
) {
    let Some(game) = game else { return };
    let pending = std::mem::take(&mut loader.pending);
    let mut keep = Vec::new();
    for p in pending {
        // hijacked, taken or gone meanwhile: nobody to seat
        let Ok(mut crew) = crews.get_mut(p.vehicle) else {
            continue;
        };
        if crew.driver.is_some() {
            continue;
        }
        if !outfit_ready(&assets, &rbms, &p.models) {
            keep.push(p);
            continue;
        }
        let rig = crew_rig(&mut loader, &game, hero_rig.as_deref(), &p.skeleton);
        // the model's feet frame on the seat point
        let local = p.seat + Vec3::Y * (HEIGHT / 2.0);
        let person = commands
            .spawn((
                Transform::from_translation(local),
                Visibility::default(),
                Seated {
                    vehicle: p.vehicle,
                    local,
                },
                Name::new(match p.kind {
                    CrewKind::Soldier => "pilot",
                    CrewKind::Civilian => "driver",
                }),
            ))
            .id();
        commands.entity(p.vehicle).add_child(person);
        if let Some(mut an) = dress(
            &mut commands,
            person,
            0.0,
            rig,
            &p.models,
            &rbms,
            &mut bindposes,
        ) {
            an.play(drive_clip(&an));
            commands.entity(person).insert(an);
        }
        crew.driver = Some(person);
        debug!(
            "crew: the {:?} sits in {:?} at {local:?}",
            p.kind, p.vehicle
        );
    }
    loader.pending.extend(keep);
}

/// The sitting clip the rig has.
fn drive_clip(an: &Animator) -> Clip {
    [Clip::Drive, Clip::CivDrive, Clip::Idle, Clip::CivIdle]
        .into_iter()
        .find(|c| an.rig.clips.contains_key(c))
        .unwrap_or(Clip::Idle)
}

/// The falling clip the rig has.
fn fall_clip(an: &Animator) -> Clip {
    [Clip::Fall, Clip::CivRun, Clip::Idle, Clip::CivIdle]
        .into_iter()
        .find(|c| an.rig.clips.contains_key(c))
        .unwrap_or(Clip::Idle)
}

fn crew_animation(mut q: Query<(&mut Animator, Has<Seated>, Has<Thrown>)>) {
    for (mut an, seated, thrown) in &mut q {
        if seated {
            let c = drive_clip(&an);
            an.play(c);
        } else if thrown {
            let c = fall_clip(&an);
            an.play(c);
        }
    }
}

type Vehicles<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Transform,
        &'static LinearVelocity,
        &'static Enterable,
        Option<&'static Crew>,
        Has<Heli>,
        Has<Plane>,
        Has<Watercraft>,
    ),
    Without<Hero>,
>;

/// `E` on a crewed vehicle: from the ground (cars and boats) or from a stunt position
/// (anything) the driver is thrown out and Rico takes the seat. On a stunt position the
/// seat is taken here even when nobody drives. A crewed helicopter or plane can't be
/// entered from the ground: the key does nothing.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn hijack_input(
    mut commands: Commands,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mode: ResMut<CameraMode>,
    mut hijacks: ResMut<Hijackings>,
    mut heroes: Query<
        (
            Entity,
            &mut Hero,
            &Transform,
            &mut LinearVelocity,
            &mut Visibility,
            Option<&OnStunt>,
        ),
        Without<crate::combat::Dead>,
    >,
    vehicles: Vehicles,
    seated: Query<&Seated>,
) {
    if !keys.just_pressed(KeyCode::KeyE) || *mode != CameraMode::Hero {
        return;
    }
    let Ok((hero_e, mut hero, tf, mut lin, mut vis, on)) = heroes.single_mut() else {
        return;
    };
    if hero.in_vehicle.is_some() {
        return;
    }
    let target = match on {
        // on the wire `E` climbs up (`stunts.rs`)
        Some(on) if on.wire.is_some() => return,
        Some(on) => on.vehicle,
        None => {
            if hero.parachute || hero.grapple.is_some() {
                return;
            }
            // the vehicle the ordinary `E` would pick
            let nearest = vehicles
                .iter()
                .map(|(e, t, _, en, ..)| (e, t.translation.distance(tf.translation) - en.reach))
                .filter(|(_, d)| *d < ENTER_RANGE)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((e, _)) = nearest else { return };
            let Ok((_, _, _, _, crew, heli, plane, _)) = vehicles.get(e) else {
                return;
            };
            if crew.is_none() {
                // nobody in it: the ordinary `E`
                return;
            }
            if heli || plane {
                // a pilot is pulled out only from the outside of the aircraft
                keys.clear_just_pressed(KeyCode::KeyE);
                info!("rico: a crewed aircraft is hijacked from its stunt position");
                return;
            }
            e
        }
    };
    let Ok((vehicle, vtf, vlin, _, crew, _, _, boat)) = vehicles.get(target) else {
        return;
    };
    keys.clear_just_pressed(KeyCode::KeyE);
    if let Some(crew) = crew {
        if let Some(driver) = crew.driver {
            let local = seated
                .get(driver)
                .ok()
                .filter(|s| s.vehicle == vehicle)
                .map_or(FALLBACK_SEAT, |s| s.local);
            throw_out(&mut commands, driver, vtf, vlin.0, local, boat);
        }
        commands
            .entity(vehicle)
            .remove::<(Crew, CrewRequest, HeliPilot, BoatPilot, AiDriven)>();
        hijacks.count += 1;
        hijacks.last = Some(vehicle);
        info!(
            "rico: hijacked {vehicle:?} from its {:?} driver ({} so far)",
            crew.kind, hijacks.count
        );
    }
    // into the seat (as `character::vehicle_entry` does)
    if on.is_some() {
        commands.entity(hero_e).remove::<OnStunt>();
        hero.stunt = None;
        hero.stunt_clip = None;
    }
    hero.in_vehicle = Some(vehicle);
    hero.grapple = None;
    commands.entity(vehicle).insert(Occupied);
    stow_passenger(&mut commands, hero_e, vehicle);
    *vis = Visibility::Hidden;
    lin.0 = Vec3::ZERO;
    *mode = CameraMode::Drive;
    info!("rico: in the vehicle");
}

/// Out of the seat at `local` of the vehicle at `vtf`: to its left (off the stern of a
/// boat), up, with the vehicle's velocity, turning over.
fn throw_out(
    commands: &mut Commands,
    driver: Entity,
    vtf: &Transform,
    velocity: Vec3,
    local: Vec3,
    boat: bool,
) {
    let at = vtf.translation + vtf.rotation * local;
    let out = if boat {
        -forward_of(vtf.rotation)
    } else {
        vtf.rotation * -MODEL_RIGHT
    };
    let flat = Vec3::new(out.x, 0.0, out.z).normalize_or(Vec3::X);
    // from the seat to beside the vehicle at once (no collider to get stuck in)
    let start = at + flat * 1.2 + Vec3::Y * 0.3;
    let spin = Vec3::Y.cross(flat).normalize_or(Vec3::X) * 3.0;
    commands
        .entity(driver)
        .remove::<(ChildOf, Seated)>()
        .insert((
            Transform::from_translation(start)
                .with_rotation(Quat::from_rotation_y(yaw_toward(flat))),
            Thrown {
                velocity: velocity + flat * THROW_OUT + Vec3::Y * THROW_UP,
                spin,
                age: 0.0,
                landed: None,
            },
        ));
    info!("crew: thrown out at {start:?}");
}

/// A thrown driver falls (gravity, turning over) until the ground or the sea, lies, and
/// goes.
fn thrown_fall(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut q: Query<(Entity, &mut Thrown, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut t, mut tf) in &mut q {
        t.age += dt;
        if let Some(lying) = t.landed.as_mut() {
            *lying += dt;
            if *lying > LIE_SECONDS {
                commands.entity(e).despawn();
            }
            continue;
        }
        if t.age > 120.0 {
            commands.entity(e).despawn();
            continue;
        }
        t.velocity.y -= 9.81 * dt;
        let step = t.velocity * dt;
        let ground = if t.velocity.y < 0.0 && t.age > 0.15 {
            spatial
                .cast_ray(
                    tf.translation,
                    Dir3::NEG_Y,
                    HEIGHT / 2.0 + step.y.abs() + 0.05,
                    true,
                    &SpatialQueryFilter::default(),
                )
                .map(|h| tf.translation.y - h.distance)
        } else {
            None
        };
        let water = (tf.translation.y + step.y < SEA_LEVEL).then_some(SEA_LEVEL - 0.3);
        if let Some(floor) = ground.or(water) {
            // on his back where he came down
            let yaw = yaw_of(tf.rotation);
            tf.translation = Vec3::new(tf.translation.x, floor + 0.25, tf.translation.z);
            tf.rotation =
                Quat::from_rotation_y(yaw) * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
            t.landed = Some(0.0);
            t.velocity = Vec3::ZERO;
            continue;
        }
        tf.translation += step;
        let turn = t.spin * dt;
        if let Ok(axis) = Dir3::new(turn) {
            tf.rotation = Quat::from_axis_angle(*axis, turn.length()) * tf.rotation;
        }
    }
}

/// The ground below `from` (a ray straight down), else the sea.
fn ground_below(spatial: &SpatialQuery, from: Vec3) -> f32 {
    spatial
        .cast_ray(
            from,
            Dir3::NEG_Y,
            1000.0,
            true,
            &SpatialQueryFilter::default(),
        )
        .map(|h| from.y - h.distance)
        .unwrap_or(SEA_LEVEL)
        .max(SEA_LEVEL)
}

/// The pilots fly: round the centre (Rico when hunting) at the radius, the altitude
/// above the ground there, nose along the way.
#[allow(clippy::type_complexity)]
fn heli_pilots(
    spatial: SpatialQuery,
    heroes: Query<&Transform, With<Hero>>,
    mut helis: Query<
        (&mut Heli, &mut HeliPilot, &Transform, &LinearVelocity),
        (Without<Occupied>, Without<Hero>),
    >,
) {
    let rico = heroes.single().ok().map(|t| t.translation);
    for (mut h, mut pilot, tf, lin) in &mut helis {
        if pilot.hunt
            && let Some(r) = rico
        {
            pilot.center = r;
        }
        let pos = tf.translation;
        let c = pilot.center;
        let off = Vec3::new(pos.x - c.x, 0.0, pos.z - c.z);
        // the velocity it wants: round the circle (anticlockwise seen from above), plus
        // in or out towards the radius
        let r = off.length().max(0.1);
        let radial = off / r;
        let tangent = Vec3::new(radial.z, 0.0, -radial.x);
        let to = tangent * pilot.cruise
            + radial * ((pilot.radius - r) * 0.6).clamp(-pilot.cruise, pilot.cruise);
        let want = yaw_toward(to);
        let err = (want - h.heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        h.yaw = (err * 2.0).clamp(-1.0, 1.0);
        // the body slides (the rotor only pushes where it tilts): stick and bank against
        // the velocity's error, not just along the nose
        let want_v = to.clamp_length_max(pilot.cruise * 1.2);
        let miss = want_v - Vec3::new(lin.0.x, 0.0, lin.0.z);
        let fwd = facing_dir(h.heading);
        let right = Quat::from_rotation_y(h.heading) * MODEL_RIGHT;
        h.pitch = (miss.dot(fwd) * 0.25).clamp(-0.7, 0.9);
        h.roll = (miss.dot(right) * 0.25).clamp(-0.7, 0.7);
        // (from under its own hull, and from just over the centre)
        let floor = ground_below(&spatial, pos - Vec3::Y * 4.0)
            .max(ground_below(&spatial, c + Vec3::Y * 3.0));
        let want_y = floor + pilot.altitude;
        h.collective = ((want_y - pos.y) * 0.25 - lin.0.y * 0.2).clamp(-1.0, 1.0);
        debug!(
            "pilot: at {pos:?} spool {:.2}, floor {floor:.1}, wants y {want_y:.1}, collective {:.2}, pitch {:.2}, yaw {:.2}",
            h.spool, h.collective, h.pitch, h.yaw
        );
    }
}

/// Distance of a clear corridor, including both sides of the hull. Numbers are
/// assumed navigation margins, not recovered AI settings.
fn boat_clearance(
    at: Vec3,
    direction: Vec3,
    length: f32,
    lookahead: f32,
    deep: &impl Fn(Vec3) -> bool,
) -> f32 {
    let right = direction.cross(Vec3::Y);
    let half = length * 0.5 + 3.0;
    let width = (length * 0.25).max(3.0);
    for i in 0..=8 {
        let distance = half + lookahead * i as f32 / 8.0;
        let centre = at + direction * distance;
        if [-width, 0.0, width]
            .into_iter()
            .any(|offset| !deep(centre + right * offset))
        {
            return lookahead * i as f32 / 8.0;
        }
    }
    lookahead
}

fn navigate_boat(
    pilot: &mut BoatPilot,
    at: Vec3,
    forward: Vec3,
    length: f32,
    speed: f32,
    deep: impl Fn(Vec3) -> bool,
) -> (f32, f32) {
    // A rudder-only boat needs room to turn and coast: probe eight seconds ahead.
    let lookahead = (speed * 8.0 + length * 4.0).clamp(60.0, 200.0);
    let clearance = |direction| boat_clearance(at, direction, length, lookahead, &deep);
    let ahead = clearance(forward);
    if ahead >= lookahead {
        pilot.avoidance = 0.0;
        return (pilot.throttle, pilot.rudder);
    }
    let side_clear = |side: f32| {
        [0.65_f32, 1.2, 1.8]
            .into_iter()
            .map(|angle| clearance(Quat::from_rotation_y(side * angle) * forward))
            .fold(0.0_f32, f32::max)
    };
    let left = side_clear(1.0);
    let right = side_clear(-1.0);
    if pilot.avoidance == 0.0
        || side_clear(pilot.avoidance) < lookahead * 0.2
            && side_clear(-pilot.avoidance) > lookahead * 0.6
    {
        pilot.avoidance = if left >= right { 1.0 } else { -1.0 };
    }
    if ahead < (speed * 4.0).max(15.0) {
        if clearance(-forward) >= lookahead * 0.6 {
            // Reverse rudder too: stern thrust reverses the yaw torque.
            return (-0.5, -pilot.avoidance);
        }
        return (0.0, pilot.avoidance);
    }
    (pilot.throttle.min(0.18), pilot.avoidance)
}

#[allow(clippy::type_complexity)]
fn boat_pilots(
    time: Res<Time>,
    game: Option<Res<Game>>,
    spatial: SpatialQuery,
    children: Query<&Children>,
    mut ground: Local<BoatGround>,
    mut boats: Query<
        (
            Entity,
            &mut Watercraft,
            &mut BoatPilot,
            &Position,
            &Rotation,
            &LinearVelocity,
        ),
        Without<Occupied>,
    >,
) {
    if boats.is_empty() {
        return;
    }
    if !ground.loaded
        && let Some(game) = game
    {
        ground.map = game
            .fs
            .read_inflated("terrain_win32.dat")
            .ok()
            .and_then(|data| GlobalTerrain::parse(&data).ok());
        ground.loaded = true;
    }
    for (entity, mut boat, mut pilot, position, rotation, velocity) in &mut boats {
        pilot.probe_in -= time.delta_secs();
        if pilot.probe_in > 0.0 {
            continue;
        }
        pilot.probe_in = 0.25;
        let filter = boat_probe_filter(entity, &children);
        let deep = |point: Vec3| {
            let coarse = ground
                .map
                .as_ref()
                .map(|map| map.height_at(point.x, point.z));
            // Loaded collision catches shore detail missed by the 64 m overview.
            // Start above sea level to catch rocks and raised shoreline too.
            let from = Vec3::new(point.x, SEA_LEVEL + 100.0, point.z);
            let local = spatial
                .cast_ray(from, Dir3::NEG_Y, 1000.0, true, &filter)
                .map(|hit| from.y - hit.distance);
            match (coarse, local) {
                (Some(a), Some(b)) => a.max(b) < SEA_LEVEL - 3.0,
                (Some(height), None) | (None, Some(height)) => height < SEA_LEVEL - 3.0,
                (None, None) => false,
            }
        };
        let direction = forward_of(rotation.0);
        let forward = Vec3::new(direction.x, 0.0, direction.z).normalize_or(Vec3::NEG_Z);
        let speed = Vec2::new(velocity.x, velocity.z).length();
        (boat.throttle, boat.rudder) =
            navigate_boat(&mut pilot, position.0, forward, boat.length, speed, deep);
    }
}

fn boat_probe_filter(entity: Entity, children: &Query<&Children>) -> SpatialQueryFilter {
    SpatialQueryFilter::from_excluded_entities(
        std::iter::once(entity).chain(children.iter_descendants(entity)),
    )
}

/// A hunting helicopter's door gun: bursts at Rico when he is within 80 m and in sight
/// (assumed numbers: 10 damage a hit, a quarter of the shots hit, 6-shot bursts every 3 s).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn heli_guns(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut tracer: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut heroes: Query<(Entity, &Transform, Option<&mut crate::combat::Health>), With<Hero>>,
    mut helis: Query<(Entity, &mut HeliPilot, &Transform, Option<&Children>), Without<Hero>>,
    mut flashes: Query<(Entity, &mut GunTracer)>,
) {
    let dt = time.delta_secs();
    for (e, mut t) in &mut flashes {
        t.0 -= dt;
        if t.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
    let Ok((hero_e, htf, mut health)) = heroes.single_mut() else {
        return;
    };
    if let (None, Some(mut m), Some(mut s)) = (tracer.as_ref(), meshes, materials) {
        *tracer = Some((
            m.add(Cylinder::new(1.0, 1.0)),
            s.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.7, 0.2),
                emissive: LinearRgba::rgb(4.0, 2.0, 0.4),
                unlit: true,
                ..default()
            }),
        ));
    }
    for (e, mut pilot, tf, children) in &mut helis {
        if !pilot.hunt {
            continue;
        }
        pilot.gun -= dt;
        if pilot.gun > 0.0 {
            continue;
        }
        // under the hull, out of its own colliders
        let muzzle = tf.translation - Vec3::Y * 2.5;
        let to = htf.translation - muzzle;
        let d = to.length();
        let Ok(dir) = Dir3::new(to) else { continue };
        let filter = SpatialQueryFilter::from_excluded_entities(
            std::iter::once(e).chain(children.into_iter().flat_map(|c| c.iter())),
        );
        let seen = d < 80.0
            && spatial
                .cast_ray(muzzle, dir, d + 1.0, true, &filter)
                .is_none_or(|h| h.entity == hero_e || h.distance > d - 1.0);
        if !seen {
            pilot.gun = 0.5;
            pilot.burst = 0;
            continue;
        }
        if pilot.burst == 0 {
            pilot.burst = 6;
        }
        pilot.burst -= 1;
        pilot.gun = if pilot.burst > 0 { 0.12 } else { 3.0 };
        pilot.shots += 1;
        let hit = unit_rand(pilot.shots.wrapping_mul(2654435761)) < 0.25;
        let aim = if hit {
            htf.translation
        } else {
            htf.translation + Vec3::new(1.5, 0.5, -1.0) * (pilot.shots % 3) as f32
        };
        if hit && let Some(h) = health.as_mut() {
            h.hp -= 10.0;
        }
        if let Some((m, s)) = tracer.as_ref() {
            commands.spawn((
                Mesh3d(m.clone()),
                MeshMaterial3d(s.clone()),
                crate::character::segment_transform(muzzle, aim, 0.012),
                GunTracer(0.06),
            ));
        }
    }
}

#[derive(Component)]
struct GunTracer(f32);

#[cfg(test)]
mod tests;
