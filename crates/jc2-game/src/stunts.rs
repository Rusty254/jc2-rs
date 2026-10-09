//! Stunts (STATUS step 7): standing on a vehicle, hanging under a helicopter, and
//! tethering two things together with the grappling hook.
//!
//! **Stunt positions.** Every vehicle doll names its stunt positions: `CSmartObject`s with
//! a stunt controller (`jc2_formats::vehicle::StuntPosition`). Cars, boats and planes carry
//! a `CRoofTopStuntController` (the jeep on its hardtop and hood, the motorboat on the bow
//! deck, the fighter on the fuselage): Rico stands there. Helicopters carry a
//! `CBellyStuntController` under the hull, and the chopper and the military helicopter a
//! `CThreeNodeClingController` at the nose as well: there he hangs on by his hands.
//! Rico gets onto one by hooking the vehicle (`F` at its body: the hook sticks to it and
//! he is reeled up) or with `Q` close to it. Then he is *pinned*: each physics step his
//! `Position` is set to the spot in the vehicle's frame (after the solver, so there is no
//! lag), his body is made inert with `physics::stow_passenger` (no collision, no gravity)
//! and his own controller is skipped (`Hero::stunt`). On a roof he faces the nose and
//! crouches once the vehicle is quick; under a helicopter he hangs by a hand (`Clip::Hang`, the grapple hang, as the original shows him).
//! He can fire his guns, and gets off with `Space` (a jump with the vehicle's velocity;
//! `Space` again opens the chute), `Q` (steps off), `E` (into the seat; on a crewed
//! vehicle `crew.rs` throws the driver out first) or `F` (fires the hook). Every vehicle
//! the fleet builds gets a `StuntSpots` component from its doll, driven or not.
//!
//! **On the wire.** A flying helicopter hooked from below does not reel Rico all the way
//! up: once he is within `WIRE_LENGTH` of the hook point, below it and with air under his
//! feet, he hangs on the wire, pinned straight below the hook at the wire's length (the
//! same inert passenger). He can shoot; `Space`, `Q` or `F` let go (he falls, and `Space`
//! once more opens the chute) and `E` climbs up to the helicopter's stunt position.
//!
//! **Tethers.** `Ctrl`+`F` shoots the tether gun where the camera looks: the first shot
//! sets one end (a vehicle, a dynamic prop, a soldier, or static ground and buildings),
//! the second sets the other and a cable (an Avian `DistanceJoint` between the two anchor
//! points, in the bodies' own frames) pulls them together: its length winds in towards
//! `TETHER_MIN` at `TETHER_RATE` m/s. At most `MAX_TETHERS` at once (the oldest goes);
//! `X` clears them all; they snap when stretched past `TETHER_MAX_STRETCH`, and vanish
//! with the bodies they hold (tiles unloading, vehicles bursting).

use crate::Game;
use crate::character::{HEIGHT, Hero, HeroParams, Settling};
use crate::coords::{MODEL_FORWARD, forward_of, to_bevy};
use crate::fleet::Container;
use crate::grapple::Cable;
use crate::heli::Heli;
use crate::physics::{release_passenger, stow_passenger};
use crate::rig::Clip;
use crate::vehicle::{CameraMode, Occupied};
use crate::world::FlyCamera;
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::sarc::SmallArchive;
use jc2_formats::vehicle::{StuntKind, StuntPosition, VehicleDoll};
use std::collections::HashMap;

/// How near (m) Rico must be to a stunt point to get up with `Q`.
pub const MOUNT_REACH: f32 = 6.0;
/// How near (m) to a stunt point the hook leaves him for him to land on it.
pub const HOOK_MOUNT_REACH: f32 = 6.0;
/// Jumping off a roof: the up speed added to the car's velocity.
const JUMP_OFF_SPEED: f32 = 5.5;
/// Hanging under a helicopter's belly: his middle this far below the stunt point, on a
/// short wire from the launcher on his forearm (the grapple hang holds it at chest
/// height), as the original hangs him under the skids.
pub const HANG_DROP: f32 = 2.0;
/// Clinging to a helicopter's nose: his middle this far ahead of the point and below it.
const CLING_OUT: f32 = 0.45;
const CLING_DROP: f32 = 0.5;
/// The wire under a flying helicopter: he is caught on it this near the hook (m) and
/// hangs at least `WIRE_MIN` below it, with at least `WIRE_CLEARANCE` of air under his
/// feet (over the ground the hook reels him up as for any vehicle).
pub const WIRE_LENGTH: f32 = 6.0;
const WIRE_MIN: f32 = 2.5;
const WIRE_CLEARANCE: f32 = 2.0;
/// The swing on the wire: how fast his motion relative to the hook point dies away (1/s;
/// air drag and the wire's own give). Assumed: the data has no number for it.
pub const WIRE_DAMPING: f32 = 0.5;

/// Winding in: the cable's shortest length (m between the two anchor points) and speed.
pub const TETHER_MIN: f32 = 2.5;
pub const TETHER_RATE: f32 = 8.0;
/// A cable stretched this much (m) past its length snaps.
pub const TETHER_MAX_STRETCH: f32 = 4.0;
/// The longest cable that can be set up (m between the anchors).
pub const TETHER_MAX_LENGTH: f32 = 200.0;
pub const MAX_TETHERS: usize = 3;

// ---------------------------------------------------------------- the roof

/// One stunt position of a vehicle, in its frame (Bevy axes).
#[derive(Clone, Debug, PartialEq)]
pub struct StuntSpot {
    /// Authored record retained for exact-ID transfer resolution.
    pub authored: StuntPosition,
    pub name: String,
    /// On top, under the belly or at the nose.
    pub kind: StuntKind,
    /// The doll's point: where his feet go on a roof, his hands under a helicopter.
    pub local: Vec3,
    /// The grappling-jump trigger box (centre, half extents); zero half extents when the
    /// doll has none.
    pub box_center: Vec3,
    pub box_half: Vec3,
}

impl StuntSpot {
    pub fn from_data(p: &StuntPosition) -> Self {
        let (box_center, box_half) = match &p.grapple_trigger {
            Some(t) => (to_bevy(t.center), Vec3::from(t.half_extents).abs()),
            None => (Vec3::ZERO, Vec3::ZERO),
        };
        Self {
            authored: p.clone(),
            name: p.name.clone(),
            kind: p.kind,
            local: to_bevy(p.position()),
            box_center,
            box_half,
        }
    }

    fn in_box(&self, local_point: Vec3) -> bool {
        self.box_half == Vec3::ZERO
            || !(local_point - self.box_center)
                .abs()
                .cmpgt(self.box_half)
                .any()
    }

    /// Where his middle goes, in the vehicle's frame.
    pub fn centre(&self) -> Vec3 {
        match self.kind {
            StuntKind::Roof => self.local + Vec3::Y * (HEIGHT / 2.0 + 0.02),
            StuntKind::Belly => self.local - Vec3::Y * HANG_DROP,
            StuntKind::Cling => self.local + MODEL_FORWARD * CLING_OUT - Vec3::Y * CLING_DROP,
        }
    }

    /// The clip he holds on with (none: he stands or crouches).
    pub fn clip(&self) -> Option<Clip> {
        match self.kind {
            StuntKind::Roof => None,
            StuntKind::Belly | StuntKind::Cling => Some(Clip::Hang),
        }
    }
}

/// The stunt positions of a vehicle (on the vehicle entity).
#[derive(Component, Clone, Debug, Default)]
pub struct StuntSpots(pub Vec<StuntSpot>);

/// The doll's stunt positions by container name, read once per kind of vehicle.
#[derive(Resource, Default)]
pub struct StuntCatalog(HashMap<String, Vec<StuntSpot>>);

/// Rico is on this vehicle: at a stunt position (`local` is where his middle is, in its
/// frame), or on the wire below it (`wire` is the length, `local` the hook point).
#[derive(Component, Clone, Copy, Debug)]
pub struct OnStunt {
    pub vehicle: Entity,
    pub local: Vec3,
    pub kind: StuntKind,
    pub wire: Option<f32>,
}

/// Current authored smart-object identity, independent of vector order.
#[derive(Component, Clone, Copy, Debug)]
pub struct CurrentStuntSpot(pub Option<i32>);

/// Runtime bridge between observed original action endpoints. Timing/path are an
/// explicit approximation, not decoded retail clip playback or collision checking.
#[derive(Component, Clone, Debug)]
pub struct StuntTransfer {
    pub action: jc2_formats::vehicle::StuntAction,
    pub destination: i32,
    pub kind: StuntKind,
    from: Vec3,
    to: Vec3,
    elapsed: f32,
}

impl StuntTransfer {
    fn advance(&mut self, dt: f32) -> (Vec3, Vec3, bool) {
        const DURATION: f32 = 0.6;
        self.elapsed = (self.elapsed + dt.max(0.0)).min(DURATION);
        let t = self.elapsed / DURATION;
        let blend = t * t * (3.0 - 2.0 * t);
        let rate = (self.to - self.from) * (6.0 * t * (1.0 - t) / DURATION);
        (self.from.lerp(self.to, blend), rate, t >= 1.0)
    }
}

/// Unique exact-ID association and compatible observed action/controller kinds.
/// Disabled/conditional actions stay diagnostic until their enabling state is known.
fn transfer_options<'a>(
    source: &'a StuntSpot,
    spots: &'a StuntSpots,
) -> Vec<(&'a jc2_formats::vehicle::StuntAction, &'a StuntSpot)> {
    let mut options = Vec::new();
    for action in &source.authored.actions {
        let Some(id) = action.destination_id() else {
            continue;
        };
        let Some(kind) = action.transfer_kind() else {
            continue;
        };
        if action.raw.int("enabled") == Some(0)
            || ["enable_event", "disable_event"]
                .iter()
                .any(|key| action.raw.str(key).is_some_and(|s| !s.is_empty()))
        {
            continue;
        }
        let mut targets = spots.0.iter().filter(|s| s.authored.object_id == Some(id));
        if let Some(target) = targets.next()
            && targets.next().is_none()
            && target.kind == kind
            && source.authored.object_id != Some(id)
        {
            options.push((action, target));
        }
    }
    options.sort_by(|a, b| {
        a.0.name()
            .cmp(b.0.name())
            .then_with(|| a.0.key.hash().cmp(&b.0.key.hash()))
    });
    options
}

pub(crate) fn transfer_targets(
    on: &OnStunt,
    current: &CurrentStuntSpot,
    spots: &StuntSpots,
) -> (Option<&'static str>, Option<&'static str>) {
    if on.wire.is_some() {
        return (None, None);
    }
    let Some(id) = current.0 else {
        return (None, None);
    };
    let mut sources = spots.0.iter().filter(|s| s.authored.object_id == Some(id));
    let Some(source) = sources.next() else {
        return (None, None);
    };
    if sources.next().is_some() {
        return (None, None);
    }
    let options = transfer_options(source, spots);
    let label = |s: &StuntSpot| match s.kind {
        StuntKind::Cling => "front cling",
        StuntKind::Belly => "belly",
        StuntKind::Roof if s.name == "SO_Hood_RD" => "hood",
        StuntKind::Roof => "roof",
    };
    (
        options.first().map(|o| label(o.1)),
        options
            .last()
            .filter(|_| options.len() > 1)
            .map(|o| label(o.1)),
    )
}

pub(crate) fn transfer_hint(
    on: &OnStunt,
    current: &CurrentStuntSpot,
    spots: &StuntSpots,
) -> String {
    match transfer_targets(on, current, spots) {
        (Some(first), Some(last)) => format!("   Z {first} / Shift+Z {last}"),
        (Some(first), _) => format!("   Z {first}"),
        _ => String::new(),
    }
}

fn begin_transfer(
    commands: &mut Commands,
    entity: Entity,
    on: &OnStunt,
    current: Option<&CurrentStuntSpot>,
    spots: &StuntSpots,
    reverse: bool,
) {
    let Some(id) = current.and_then(|s| s.0) else {
        return;
    };
    let mut sources = spots.0.iter().filter(|s| s.authored.object_id == Some(id));
    let Some(source) = sources.next() else { return };
    if sources.next().is_some() {
        return;
    }
    let options = transfer_options(source, spots);
    let choice = if reverse {
        options.last()
    } else {
        options.first()
    };
    let Some((action, target)) = choice else {
        info!(
            "rico: no supported authored transfer from {} (unknown links retained)",
            source.name
        );
        return;
    };
    info!(
        "rico: transfer {} -> {} via {}",
        source.name,
        target.name,
        action.name()
    );
    commands.entity(entity).insert(StuntTransfer {
        action: (*action).clone(),
        destination: target.authored.object_id.unwrap(),
        kind: target.kind,
        from: on.local,
        to: target.centre(),
        elapsed: 0.0,
    });
}

/// World-space model correction only; the physical pendulum remains authoritative.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct WireTilt(pub Quat);

pub fn smooth_wire_tilt(previous: Quat, up: Vec3, dt: f32) -> Quat {
    let Some(up) = up.try_normalize() else {
        return previous;
    };
    let target = Quat::from_rotation_arc(Vec3::Y, up);
    previous
        .slerp(target, 1.0 - (-10.0 * dt.max(0.0)).exp())
        .normalize()
}

/// The hook is stuck in a vehicle, at `local` in its frame.
#[derive(Component, Clone, Copy, Debug)]
pub struct HookedVehicle {
    pub vehicle: Entity,
    pub local: Vec3,
}

/// Rico on the wire swings like a pendulum under the hook point: his position and
/// velocity (world), stepped by [`swing_step`] while the hook point moves with the
/// helicopter.
#[derive(Component, Clone, Copy, Debug)]
pub struct WireSwing {
    pub pos: Vec3,
    pub vel: Vec3,
}

/// One step of the pendulum: gravity, damping of the motion relative to the moving
/// pivot, then the wire's length holds him at `length` from `hook`. The velocity comes
/// from the constrained move, so the pivot's own motion drives the swing.
pub fn swing_step(state: &mut WireSwing, hook: Vec3, hook_vel: Vec3, length: f32, dt: f32) {
    if dt <= 0.0 {
        return;
    }
    let mut v = state.vel + Vec3::NEG_Y * 9.81 * dt;
    v = hook_vel + (v - hook_vel) * (-WIRE_DAMPING * dt).exp();
    let mut p = state.pos + v * dt;
    let d = p - hook;
    let len = d.length();
    p = if len > 1e-3 {
        hook + d / len * length
    } else {
        hook - Vec3::Y * length
    };
    state.vel = (p - state.pos) / dt;
    state.pos = p;
}

/// The spot of a vehicle (at `pos`/`rot`) nearest to `at`, within `reach`. With
/// `use_box` the point must also lie in the spot's trigger box.
pub fn nearest_spot(
    at: Vec3,
    pos: Vec3,
    rot: Quat,
    spots: &StuntSpots,
    reach: f32,
    use_box: bool,
) -> Option<(usize, f32)> {
    let local_at = rot.inverse() * (at - pos);
    spots
        .0
        .iter()
        .enumerate()
        .filter(|(_, s)| !use_box || s.in_box(local_at))
        .map(|(i, s)| (i, (pos + rot * s.local).distance(at)))
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The stunt positions in a vehicle container's doll.
pub fn read_spots(game: Option<&Game>, container: &str) -> Vec<StuntSpot> {
    game.and_then(|g| g.fs.read_inflated(container).ok())
        .and_then(|data| {
            let sarc = SmallArchive::parse(&data).ok()?;
            let e = sarc
                .entries
                .iter()
                .find(|e| e.name.to_ascii_lowercase().ends_with("doll"))?;
            VehicleDoll::parse(sarc.data(&data, e).ok()?).ok()
        })
        .map(|d| d.stunts.iter().map(StuntSpot::from_data).collect())
        .unwrap_or_default()
}

/// Read the stunt positions of a new vehicle's doll (cars, helicopters, boats, planes:
/// every vehicle the fleet builds is tagged with its container).
fn spots_for_vehicle(
    add: On<Add, Container>,
    vehicles: Query<&Container>,
    game: Option<Res<Game>>,
    mut catalog: ResMut<StuntCatalog>,
    mut commands: Commands,
) {
    let Ok(c) = vehicles.get(add.entity) else {
        return;
    };
    let name = c.0.clone();
    let spots = catalog
        .0
        .entry(name.clone())
        .or_insert_with(|| read_spots(game.as_deref(), &name))
        .clone();
    commands.entity(add.entity).insert(StuntSpots(spots));
}

/// The way he faces on a vehicle turned by `rot`: the nose, or back at it when clinging
/// to the front.
fn facing_on(rot: Quat, kind: StuntKind, keep: f32) -> f32 {
    let nose = forward_of(rot);
    let dir = if kind == StuntKind::Cling {
        -nose
    } else {
        nose
    };
    crate::character::heading(dir, keep)
}

/// Stow him on `vehicle` at `world` moving at `velocity`, held there by `on`.
fn pin(
    commands: &mut Commands,
    entity: Entity,
    hero: &mut Hero,
    world: Vec3,
    velocity: Vec3,
    on: OnStunt,
) {
    stow_passenger(commands, entity, on.vehicle);
    commands
        .entity(entity)
        .insert(on)
        .queue(move |mut e: EntityWorldMut| {
            // stow puts him at the vehicle's origin: this is the spot
            e.insert((Position(world), LinearVelocity(velocity)));
            if let Some(mut t) = e.get_mut::<Transform>() {
                t.translation = world;
            }
        });
    hero.stunt = Some(on.vehicle);
    hero.grapple = None;
    hero.parachute = false;
    hero.grounded = true;
    hero.speed = 0.0;
}

/// Put Rico on a stunt position.
#[allow(clippy::too_many_arguments)]
fn attach(
    commands: &mut Commands,
    entity: Entity,
    hero: &mut Hero,
    vehicle: Entity,
    pos: Vec3,
    rot: Quat,
    velocity: Vec3,
    spot: &StuntSpot,
) {
    let local = spot.centre();
    let on = OnStunt {
        vehicle,
        local,
        kind: spot.kind,
        wire: None,
    };
    pin(commands, entity, hero, pos + rot * local, velocity, on);
    commands
        .entity(entity)
        .insert(CurrentStuntSpot(spot.authored.object_id));
    commands
        .entity(entity)
        .remove::<(StuntTransfer, WireSwing, WireTilt)>();
    hero.stunt_clip = spot.clip();
    hero.facing = facing_on(rot, spot.kind, hero.facing);
    info!("rico: on {vehicle:?} ({}, {:?})", spot.name, spot.kind);
}

/// Hang Rico on the wire `length` below the hook point `hook` (in the vehicle's frame).
#[allow(clippy::too_many_arguments)]
fn attach_wire(
    commands: &mut Commands,
    entity: Entity,
    hero: &mut Hero,
    vehicle: Entity,
    hook: Vec3,
    length: f32,
    pos: Vec3,
    rot: Quat,
    velocity: Vec3,
) {
    let on = OnStunt {
        vehicle,
        local: hook,
        kind: StuntKind::Roof,
        wire: Some(length),
    };
    let world = pos + rot * hook - Vec3::Y * length;
    pin(commands, entity, hero, world, velocity, on);
    commands
        .entity(entity)
        .remove::<(CurrentStuntSpot, StuntTransfer)>();
    commands.entity(entity).insert(WireTilt::default());
    commands.entity(entity).insert(WireSwing {
        pos: world,
        vel: velocity,
    });
    hero.stunt_clip = Some(Clip::Hang);
    hero.facing = facing_on(rot, StuntKind::Roof, hero.facing);
    info!("rico: on the wire {length:.1} m under {vehicle:?}");
}

/// Take Rico off the vehicle, at `position` moving at `velocity`.
pub fn detach(
    commands: &mut Commands,
    entity: Entity,
    hero: &mut Hero,
    position: Vec3,
    velocity: Vec3,
) {
    release_passenger(commands, entity, position, velocity);
    commands.entity(entity).remove::<(
        OnStunt,
        WireSwing,
        CurrentStuntSpot,
        StuntTransfer,
        WireTilt,
    )>();
    hero.stunt = None;
    hero.stunt_clip = None;
    hero.grounded = false;
    info!("rico: off the vehicle");
}

type Rooftops<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Position,
        &'static Rotation,
        &'static LinearVelocity,
        &'static AngularVelocity,
        Option<&'static ComputedCenterOfMass>,
        &'static StuntSpots,
    ),
    (Without<Hero>, Without<Occupied>),
>;

/// `Q` gets up on the nearest stunt position; on it `Space` jumps off, `Q` steps off, `E`
/// climbs in (the normal `E` handling then seats him, or `crew.rs` hijacks first) and
/// `F` fires the hook. On the wire `Space`, `Q` and `F` let go and `E` climbs up.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn mount_input(
    mut commands: Commands,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mode: Res<CameraMode>,
    mut heroes: Query<
        (
            Entity,
            &mut Hero,
            &Transform,
            &LinearVelocity,
            Option<&OnStunt>,
            Option<&CurrentStuntSpot>,
            Has<StuntTransfer>,
        ),
        (
            Without<crate::combat::Dead>,
            Without<crate::ragdoll::PendingRagdoll>,
            Without<crate::ragdoll::PhysicsPose>,
        ),
    >,
    vehicles: Rooftops,
) {
    if *mode != CameraMode::Hero {
        return;
    }
    let Ok((entity, mut hero, tf, lin, on, current, transferring)) = heroes.single_mut() else {
        return;
    };
    if let Some(on) = on {
        let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
        let jump = keys.just_pressed(KeyCode::Space);
        let quit = keys.just_pressed(KeyCode::KeyQ);
        let enter = keys.just_pressed(KeyCode::KeyE);
        let hook = keys.just_pressed(KeyCode::KeyF) && !ctrl;
        // (`Drop` is `Alt` under a helicopter in the original, `Ctrl` on a ledge)
        let drop = keys.just_pressed(KeyCode::AltLeft) || keys.just_pressed(KeyCode::ControlLeft);
        if on.wire.is_some() {
            if enter {
                // up the wire onto the helicopter; the key is spent (not a seat yet)
                keys.clear_just_pressed(KeyCode::KeyE);
                if let Ok((car, p, r, l, a, center, spots)) = vehicles.get(on.vehicle)
                    && let Some((i, _)) =
                        nearest_spot(tf.translation, p.0, r.0, spots, f32::MAX, false)
                {
                    let at = p.0 + r.0 * spots.0[i].centre();
                    attach(
                        &mut commands,
                        entity,
                        &mut hero,
                        car,
                        p.0,
                        r.0,
                        l.0 + a
                            .0
                            .cross(at - p.0 - r.0 * center.map_or(Vec3::ZERO, |c| c.0)),
                        &spots.0[i],
                    );
                }
            } else if jump || quit || hook || drop {
                // let go: he falls with the helicopter's speed
                detach(&mut commands, entity, &mut hero, tf.translation, lin.0);
            }
            return;
        }
        if !(jump || quit || enter || hook) && !transferring && keys.just_pressed(KeyCode::KeyZ) {
            if let Ok((_, _, _, _, _, _, spots)) = vehicles.get(on.vehicle) {
                begin_transfer(
                    &mut commands,
                    entity,
                    on,
                    current,
                    spots,
                    keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
                );
            }
            return;
        }
        if !(jump || quit || enter || hook) {
            return;
        }
        let up = if jump {
            Vec3::Y * JUMP_OFF_SPEED
        } else {
            Vec3::ZERO
        };
        detach(&mut commands, entity, &mut hero, tf.translation, lin.0 + up);
        return;
    }
    if !keys.just_pressed(KeyCode::KeyQ) || hero.in_vehicle.is_some() || hero.grapple.is_some() {
        return;
    }
    let best = vehicles
        .iter()
        .filter_map(|(e, p, r, l, a, center, spots)| {
            nearest_spot(tf.translation, p.0, r.0, spots, MOUNT_REACH, true).map(|(i, d)| {
                (
                    e,
                    p.0,
                    r.0,
                    l.0,
                    a.0,
                    center.map_or(Vec3::ZERO, |c| c.0),
                    spots,
                    i,
                    d,
                )
            })
        })
        .min_by(|a, b| a.8.total_cmp(&b.8));
    if let Some((car, p, r, l, a, center, spots, i, _)) = best {
        let at = p + r * spots.0[i].centre();
        attach(
            &mut commands,
            entity,
            &mut hero,
            car,
            p,
            r,
            l + a.cross(at - p - r * center),
            &spots.0[i],
        );
    }
}

/// After the physics step: pin Rico to his spot (or the wire), in the vehicle's frame,
/// and give him its velocity there. A vanished vehicle, a respawn or a death lets him go.
#[allow(clippy::type_complexity)]
fn follow_vehicle(
    mut commands: Commands,
    mut heroes: Query<
        (
            Entity,
            &mut Hero,
            &mut OnStunt,
            &mut Position,
            &mut Transform,
            &mut LinearVelocity,
            Has<Settling>,
            Option<&mut WireSwing>,
            Option<&mut StuntTransfer>,
            Option<&mut CurrentStuntSpot>,
            Option<&mut WireTilt>,
        ),
        (
            Without<crate::combat::Dead>,
            Without<crate::ragdoll::PendingRagdoll>,
            Without<crate::ragdoll::PhysicsPose>,
        ),
    >,
    vehicles: Query<
        (
            &Position,
            &Rotation,
            &LinearVelocity,
            &AngularVelocity,
            Option<&ComputedCenterOfMass>,
        ),
        Without<Hero>,
    >,
    time: Res<Time>,
) {
    let Ok((
        entity,
        mut hero,
        mut on,
        mut pos,
        mut tf,
        mut lin,
        settling,
        swing,
        transfer,
        current,
        tilt,
    )) = heroes.single_mut()
    else {
        return;
    };
    let Ok((vpos, vrot, vlin, vang, center)) = vehicles.get(on.vehicle) else {
        let (p, v) = (pos.0, lin.0);
        detach(&mut commands, entity, &mut hero, p, v);
        return;
    };
    if settling {
        // he died and came back at the start: the respawn placed him already
        commands.entity(entity).remove::<(
            OnStunt,
            WireSwing,
            CurrentStuntSpot,
            StuntTransfer,
            WireTilt,
        )>();
        let (p, v) = (pos.0, Vec3::ZERO);
        release_passenger(&mut commands, entity, p, v);
        hero.stunt = None;
        hero.stunt_clip = None;
        return;
    }
    let support_center = vpos.0 + vrot.0 * center.map_or(Vec3::ZERO, |c| c.0);
    let relative_velocity = if let Some(mut transfer) = transfer {
        let (local, rate, done) = transfer.advance(time.delta_secs());
        on.local = local;
        if done {
            info!(
                "rico: completed authored transfer {}",
                transfer.action.name()
            );
            on.kind = transfer.kind;
            hero.stunt_clip = match on.kind {
                StuntKind::Roof => None,
                _ => Some(Clip::Hang),
            };
            if let Some(mut current) = current {
                current.0 = Some(transfer.destination);
            }
            commands.entity(entity).remove::<StuntTransfer>();
        }
        vrot.0 * rate
    } else {
        Vec3::ZERO
    };
    let (target, velocity) = match (on.wire, swing) {
        (Some(length), swing) => {
            // on the wire: a pendulum under the hook point, driven by the helicopter
            let hook = vpos.0 + vrot.0 * on.local;
            let hook_vel = vlin.0 + vang.0.cross(hook - support_center);
            let mut state = swing.as_deref().copied().unwrap_or(WireSwing {
                pos: hook - Vec3::Y * length,
                vel: hook_vel,
            });
            swing_step(&mut state, hook, hook_vel, length, time.delta_secs());
            if let Some(mut s) = swing {
                *s = state;
            } else {
                commands.entity(entity).insert(state);
            }
            if let Some(mut tilt) = tilt {
                tilt.0 = smooth_wire_tilt(tilt.0, hook - state.pos, time.delta_secs());
            }
            (state.pos, state.vel)
        }
        (None, _) => {
            let t = vpos.0 + vrot.0 * on.local;
            (
                t,
                vlin.0 + vang.0.cross(t - support_center) + relative_velocity,
            )
        }
    };
    pos.0 = target;
    tf.translation = target;
    lin.0 = velocity;
    hero.facing = facing_on(vrot.0, on.kind, hero.facing);
    hero.grounded = true;
    hero.grapple = None;
    hero.parachute = false;
}

/// A hook that has just been fired into a vehicle sticks to it.
#[allow(clippy::type_complexity)]
fn hook_target(
    mut commands: Commands,
    heroes: Query<(Entity, &Hero, Option<&HookedVehicle>)>,
    vehicles: Query<(Entity, &Position, &Rotation, &ColliderAabb), With<StuntSpots>>,
) {
    let Ok((entity, hero, hooked)) = heroes.single() else {
        return;
    };
    let Some(g) = hero.grapple else { return };
    if hooked.is_some() || g.age > 0.2 {
        return;
    }
    for (car, pos, rot, aabb) in &vehicles {
        let slack = Vec3::splat(0.75);
        if g.anchor.cmpge(aabb.min - slack).all() && g.anchor.cmple(aabb.max + slack).all() {
            commands.entity(entity).insert(HookedVehicle {
                vehicle: car,
                local: rot.0.inverse() * (g.anchor - pos.0),
            });
            return;
        }
    }
}

/// While the wire holds, its end rides on the vehicle.
fn stick_hook(
    mut commands: Commands,
    mut heroes: Query<(Entity, &mut Hero, &HookedVehicle)>,
    vehicles: Query<(&Position, &Rotation), Without<Hero>>,
) {
    for (entity, mut hero, hooked) in &mut heroes {
        match vehicles.get(hooked.vehicle) {
            Ok((pos, rot)) => {
                if let Some(g) = hero.grapple.as_mut() {
                    g.anchor = pos.0 + rot.0 * hooked.local;
                }
            }
            Err(_) => {
                commands.entity(entity).remove::<HookedVehicle>();
            }
        }
    }
}

/// Reeled up under a flying helicopter: he stays on the wire below it.
#[allow(clippy::type_complexity)]
fn wire_catch(
    mut commands: Commands,
    spatial: SpatialQuery,
    mut heroes: Query<(Entity, &mut Hero, &Transform, &HookedVehicle)>,
    helis: Query<
        (
            &Position,
            &Rotation,
            &LinearVelocity,
            &AngularVelocity,
            Option<&ComputedCenterOfMass>,
        ),
        (With<Heli>, Without<Hero>),
    >,
) {
    for (entity, mut hero, tf, hooked) in &mut heroes {
        if hero.grapple.is_none() || hero.stunt.is_some() || hero.in_vehicle.is_some() {
            continue;
        }
        let Ok((p, r, l, a, center)) = helis.get(hooked.vehicle) else {
            continue;
        };
        let hook = p.0 + r.0 * hooked.local;
        let below = hook.y - tf.translation.y;
        if below < 1.0 || hook.distance(tf.translation) > WIRE_LENGTH {
            continue;
        }
        // over the ground he is reeled up to the helicopter as onto any vehicle
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        if spatial
            .cast_ray(
                tf.translation,
                Dir3::NEG_Y,
                HEIGHT / 2.0 + WIRE_CLEARANCE,
                true,
                &filter,
            )
            .is_some()
        {
            continue;
        }
        commands.entity(entity).remove::<HookedVehicle>();
        attach_wire(
            &mut commands,
            entity,
            &mut hero,
            hooked.vehicle,
            hooked.local,
            below.clamp(WIRE_MIN, WIRE_LENGTH),
            p.0,
            r.0,
            l.0 + a
                .0
                .cross(hook - p.0 - r.0 * center.map_or(Vec3::ZERO, |c| c.0)),
        );
    }
}

/// The wire has reeled him in: if he is at a stunt point of the vehicle he hooked, he
/// lands on it.
#[allow(clippy::type_complexity)]
fn hook_arrival(
    mut commands: Commands,
    mut heroes: Query<(Entity, &mut Hero, &Transform, &HookedVehicle)>,
    vehicles: Rooftops,
) {
    for (entity, mut hero, tf, hooked) in &mut heroes {
        if hero.grapple.is_some() {
            continue;
        }
        commands.entity(entity).remove::<HookedVehicle>();
        if hero.stunt.is_some() || hero.in_vehicle.is_some() {
            continue;
        }
        let Ok((car, p, r, l, a, center, spots)) = vehicles.get(hooked.vehicle) else {
            continue;
        };
        let Some((i, _)) = nearest_spot(tf.translation, p.0, r.0, spots, HOOK_MOUNT_REACH, false)
        else {
            continue;
        };
        let at = p.0 + r.0 * spots.0[i].centre();
        attach(
            &mut commands,
            entity,
            &mut hero,
            car,
            p.0,
            r.0,
            l.0 + a
                .0
                .cross(at - p.0 - r.0 * center.map_or(Vec3::ZERO, |c| c.0)),
            &spots.0[i],
        );
    }
}

/// On a vehicle the orbit camera looks past it: the vehicle he rides does not pull the
/// camera in (its hull would put the camera inside his head), and under a helicopter
/// it stands further back. Runs after `character::hero_camera` and redoes its placement.
#[allow(clippy::type_complexity)]
fn stunt_camera(
    mode: Res<CameraMode>,
    spatial: SpatialQuery,
    orbit: Option<Res<crate::character::OrbitCamera>>,
    heroes: Query<(Entity, &Transform, &OnStunt), With<Hero>>,
    vehicles: Query<Option<&Children>, Without<Hero>>,
    mut cams: Query<&mut Transform, (With<FlyCamera>, Without<Hero>)>,
) {
    let Some(orbit) = orbit else { return };
    if *mode != CameraMode::Hero {
        return;
    }
    let Ok((entity, tf, on)) = heroes.single() else {
        return;
    };
    let Ok(mut cam) = cams.single_mut() else {
        return;
    };
    let mut skip = vec![entity, on.vehicle];
    if let Ok(Some(children)) = vehicles.get(on.vehicle) {
        skip.extend(children.iter());
    }
    let hanging = on.wire.is_some() || on.kind != StuntKind::Roof;
    let distance = if hanging {
        orbit.distance * 1.7
    } else {
        orbit.distance * 1.2
    };
    let target = tf.translation + Vec3::Y * 0.6;
    let rot = Quat::from_euler(EulerRot::YXZ, orbit.yaw, orbit.pitch, 0.0);
    let back = rot * Vec3::Z;
    let dist = match Dir3::new(back) {
        Ok(d) => spatial
            .cast_ray(
                target,
                d,
                distance,
                true,
                &SpatialQueryFilter::from_excluded_entities(skip),
            )
            .map(|h| (h.distance - 0.3).max(0.6))
            .unwrap_or(distance),
        Err(_) => distance,
    };
    cam.translation = target + back * dist;
    cam.look_at(target, Vec3::Y);
}

/// `--ride`: put Rico on the first vehicle the command line asked for, once he and it
/// are there (at its first stunt position, or on the wire under it).
#[derive(Resource, Debug, Clone, Copy)]
pub struct RideAtStart {
    pub wire: bool,
    pub done: bool,
}

fn ride_at_start(
    mut commands: Commands,
    ride: Option<ResMut<RideAtStart>>,
    fleet: Option<Res<crate::fleet::Fleet>>,
    mut heroes: Query<(Entity, &mut Hero), Without<Settling>>,
    vehicles: Rooftops,
) {
    let (Some(mut ride), Some(fleet)) = (ride, fleet) else {
        return;
    };
    if ride.done {
        return;
    }
    let Ok((entity, mut hero)) = heroes.single_mut() else {
        return;
    };
    if !hero.grounded || hero.in_vehicle.is_some() {
        return;
    }
    let Some((car, p, r, l, a, center, spots)) = fleet
        .spawned
        .iter()
        .filter(|(_, tile)| tile.is_none())
        .find_map(|(e, _)| vehicles.get(*e).ok())
    else {
        return;
    };
    let Some(spot) = spots.0.first() else {
        return;
    };
    ride.done = true;
    if ride.wire {
        let hook = spot.local;
        attach_wire(
            &mut commands,
            entity,
            &mut hero,
            car,
            hook,
            5.0,
            p.0,
            r.0,
            l.0 + a.0.cross(r.0 * (hook - center.map_or(Vec3::ZERO, |c| c.0))),
        );
    } else {
        let at = r.0 * spot.centre();
        attach(
            &mut commands,
            entity,
            &mut hero,
            car,
            p.0,
            r.0,
            l.0 + a.0.cross(at - r.0 * center.map_or(Vec3::ZERO, |c| c.0)),
            spot,
        );
    }
}

/// The wire he hangs on, from his raised hands up to the hook point.
#[derive(Component)]
struct HangWire;

#[allow(clippy::type_complexity)]
fn hang_wire_visual(
    mut commands: Commands,
    heroes: Query<
        (&Transform, &Hero, &OnStunt),
        (
            Without<crate::combat::Dead>,
            Without<crate::ragdoll::PendingRagdoll>,
            Without<crate::ragdoll::PhysicsPose>,
        ),
    >,
    launchers: Query<&GlobalTransform, With<crate::grapple::GrappleLauncher>>,
    vehicles: Query<&Transform, (Without<Hero>, Without<HangWire>)>,
    mut wires: Query<(Entity, &mut Cable), With<HangWire>>,
) {
    let hang = heroes.single().ok().and_then(|(tf, h, on)| {
        // on the wire, or under the belly on a short one (the original hangs him from the
        // skid by the hook's wire)
        let held = match (on.wire, on.kind) {
            (Some(_), _) => on.local,
            (None, StuntKind::Belly) => on.local + Vec3::Y * HANG_DROP,
            _ => return None,
        };
        let v = vehicles.get(on.vehicle).ok()?;
        let hook = v.translation + v.rotation * held;
        // from the launcher on his left forearm (as `character::hero_visuals` draws it)
        let hand = launchers
            .single()
            .map(|t| t.translation())
            .unwrap_or_else(|_| crate::grapple::default_hand(tf.translation, h.facing));
        Some((hand, hook))
    });
    let Some((a, b)) = hang else {
        for (e, _) in &wires {
            commands.entity(e).despawn();
        }
        return;
    };
    // taut: his weight hangs on it
    let cable = Cable { a, b, slack: 0.02 };
    if let Ok((_, mut c)) = wires.single_mut() {
        *c = cable;
        return;
    }
    commands.spawn((cable, HangWire, Name::new("hang wire")));
}

// ---------------------------------------------------------------- tethers

/// One end of a cable.
#[derive(Clone, Copy, Debug)]
pub struct TetherEnd {
    /// The body the joint holds (a dynamic body, or a static [`TetherAnchor`]).
    pub body: Entity,
    /// The anchor point in that body's frame.
    pub local: Vec3,
    /// What was shot: it must stay alive for the cable to.
    pub owner: Entity,
    /// The point entity made for a static target; goes with the cable.
    pub anchor: Option<Entity>,
    /// Where the shot hit (world), for when the end's entity is not spawned yet.
    pub point: Vec3,
}

/// A cable: on the `DistanceJoint` entity that pulls its two ends together.
#[derive(Component, Debug)]
pub struct Tether {
    pub ends: [TetherEnd; 2],
    /// The current length: the joint's upper limit.
    pub rest: f32,
    /// Order of creation, the lowest goes first.
    pub serial: u64,
}

/// A fixed point in the world standing in for a static thing a cable is tied to.
#[derive(Component)]
pub struct TetherAnchor;

/// A tether-gun shot that hit `entity` at `point` (world).
#[derive(Clone, Copy, Debug)]
pub struct TetherShot {
    pub entity: Entity,
    pub point: Vec3,
}

#[derive(Resource, Default)]
pub struct TetherShots(pub Vec<TetherShot>);

#[derive(Resource, Default)]
pub struct TetherState {
    /// The first end, waiting for the second shot.
    pub pending: Option<TetherEnd>,
    /// Remove everything (`X`).
    pub clear: bool,
    serial: u64,
}

#[derive(Component)]
struct TetherWire(Option<Entity>);

/// `Ctrl`+`F`: shoot where the camera looks. `X`: clear them.
#[allow(clippy::too_many_arguments)]
fn tether_input(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<CameraMode>,
    params: Option<Res<HeroParams>>,
    spatial: SpatialQuery,
    heroes: Query<(Entity, &Hero, &Transform), Without<crate::combat::Dead>>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    mut shots: ResMut<TetherShots>,
    mut state: ResMut<TetherState>,
) {
    if *mode != CameraMode::Hero {
        return;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        state.clear = true;
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    if !(ctrl && keys.just_pressed(KeyCode::KeyF)) {
        return;
    }
    let Ok((hero_entity, hero, tf)) = heroes.single() else {
        return;
    };
    if hero.in_vehicle.is_some() {
        return;
    }
    let Ok(cam) = cams.single() else { return };
    let range = params.map(|p| p.grapple.range).unwrap_or(90.0);
    let dir = cam.forward();
    let to_hero = (tf.translation - cam.translation)
        .dot(dir.as_vec3())
        .max(0.0);
    let filter = SpatialQueryFilter::from_excluded_entities([hero_entity]);
    if let Some(hit) = spatial.cast_ray(cam.translation, dir, range + to_hero, true, &filter)
        && hit.distance > to_hero + 1.0
    {
        shots.0.push(TetherShot {
            entity: hit.entity,
            point: cam.translation + dir * hit.distance,
        });
    }
}

type BodyQuery<'w, 's> = Query<'w, 's, (&'static RigidBody, &'static Position, &'static Rotation)>;

/// The end a shot makes: on a dynamic body at the hit point in its frame, otherwise on a
/// fixed anchor point entity.
fn make_end(
    commands: &mut Commands,
    shot: TetherShot,
    colliders: &Query<&ColliderOf>,
    bodies: &BodyQuery,
) -> TetherEnd {
    let body = colliders
        .get(shot.entity)
        .map(|c| c.body)
        .unwrap_or(shot.entity);
    if let Ok((rb, pos, rot)) = bodies.get(body)
        && *rb == RigidBody::Dynamic
    {
        return TetherEnd {
            body,
            local: rot.0.inverse() * (shot.point - pos.0),
            owner: shot.entity,
            anchor: None,
            point: shot.point,
        };
    }
    let anchor = commands
        .spawn((
            RigidBody::Static,
            Position(shot.point),
            Transform::from_translation(shot.point),
            TetherAnchor,
            Name::new("tether anchor"),
        ))
        .id();
    TetherEnd {
        body: anchor,
        local: Vec3::ZERO,
        owner: body,
        anchor: Some(anchor),
        point: shot.point,
    }
}

fn end_world(end: &TetherEnd, bodies: &Query<(&Position, &Rotation)>) -> Option<Vec3> {
    let (p, r) = bodies.get(end.body).ok()?;
    Some(p.0 + r.0 * end.local)
}

fn drop_end(commands: &mut Commands, end: &TetherEnd) {
    if let Some(a) = end.anchor {
        commands.entity(a).try_despawn();
    }
}

/// Turn shots into a pending end, then a cable; `X` clears.
#[allow(clippy::too_many_arguments)]
fn tether_shots(
    mut commands: Commands,
    mut shots: ResMut<TetherShots>,
    mut state: ResMut<TetherState>,
    colliders: Query<&ColliderOf>,
    bodies: BodyQuery,
    poses: Query<(&Position, &Rotation)>,
    tethers: Query<(Entity, &Tether)>,
) {
    if state.clear {
        state.clear = false;
        if let Some(p) = state.pending.take() {
            drop_end(&mut commands, &p);
        }
        for (e, _) in &tethers {
            commands.entity(e).try_despawn();
        }
    }
    for shot in std::mem::take(&mut shots.0) {
        let end = make_end(&mut commands, shot, &colliders, &bodies);
        let Some(first) = state.pending.take() else {
            state.pending = Some(end);
            info!("tether: first end set");
            continue;
        };
        // two fixed things, or the same thing twice, are no cable
        let both_fixed = first.anchor.is_some() && end.anchor.is_some();
        // (a fixed point made by this very shot is not spawned yet: use where it hit)
        let a = end_world(&first, &poses).unwrap_or(first.point);
        let b = end_world(&end, &poses).unwrap_or(end.point);
        let dist = a.distance(b);
        if first.body == end.body || both_fixed || dist > TETHER_MAX_LENGTH {
            info!("tether: refused ({dist:.1} m)");
            drop_end(&mut commands, &first);
            drop_end(&mut commands, &end);
            continue;
        }
        // room for it: the oldest of the others goes
        let mut live: Vec<(u64, Entity)> = tethers.iter().map(|(e, t)| (t.serial, e)).collect();
        live.sort();
        while live.len() >= MAX_TETHERS {
            let (_, old) = live.remove(0);
            commands.entity(old).try_despawn();
        }
        state.serial += 1;
        let rest = dist.max(TETHER_MIN);
        commands.spawn((
            DistanceJoint::new(first.body, end.body)
                .with_local_anchor1(first.local)
                .with_local_anchor2(end.local)
                .with_limits(0.0, rest),
            Tether {
                ends: [first, end],
                rest,
                serial: state.serial,
            },
            Name::new("tether"),
        ));
        info!("tether: {dist:.1} m cable");
    }
}

/// Wind the cable in, snap it when stretched too far, drop it when a body is gone.
#[allow(clippy::too_many_arguments)]
fn tether_step(
    mut commands: Commands,
    time: Res<Time>,
    mut state: ResMut<TetherState>,
    poses: Query<(&Position, &Rotation)>,
    alive: Query<Entity>,
    mut tethers: Query<(Entity, &mut Tether, &mut DistanceJoint)>,
) {
    let dt = time.delta_secs();
    if let Some(p) = state.pending
        && (alive.get(p.body).is_err() || alive.get(p.owner).is_err())
    {
        drop_end(&mut commands, &p);
        state.pending = None;
    }
    for (e, mut t, mut joint) in &mut tethers {
        let gone = t
            .ends
            .iter()
            .any(|end| alive.get(end.body).is_err() || alive.get(end.owner).is_err());
        let (Some(a), Some(b)) = (end_world(&t.ends[0], &poses), end_world(&t.ends[1], &poses))
        else {
            commands.entity(e).try_despawn();
            continue;
        };
        if gone {
            commands.entity(e).try_despawn();
            continue;
        }
        let dist = a.distance(b);
        if dist > t.rest + TETHER_MAX_STRETCH {
            info!("tether: snapped at {dist:.1} m");
            commands.entity(e).try_despawn();
            continue;
        }
        let rest = (t.rest.min(dist) - TETHER_RATE * dt).max(TETHER_MIN);
        if rest != t.rest {
            t.rest = rest;
            joint.limits = DistanceLimit::new(0.0, rest);
        }
    }
}

/// A cable that goes takes the fixed anchor points it made with it.
fn tether_removed(remove: On<Remove, Tether>, tethers: Query<&Tether>, mut commands: Commands) {
    if let Ok(t) = tethers.get(remove.entity) {
        for end in &t.ends {
            drop_end(&mut commands, end);
        }
    }
}

/// A body that is despawned takes its cables with it at once (not a step later, with the
/// solver holding a joint to nothing).
fn body_removed(
    remove: On<Remove, RigidBody>,
    tethers: Query<(Entity, &Tether)>,
    mut commands: Commands,
) {
    for (e, t) in &tethers {
        if t.ends
            .iter()
            .any(|end| end.body == remove.entity || end.owner == remove.entity)
        {
            commands.entity(e).try_despawn();
        }
    }
}

/// The cables as thin white lines (`grapple::Cable`, as the hook's wire), and the first
/// end's line from Rico's hand while it waits. A cable with length to spare sags.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn tether_visuals(
    mut commands: Commands,
    state: Res<TetherState>,
    tethers: Query<(Entity, &Tether)>,
    poses: Query<(&Position, &Rotation)>,
    heroes: Query<(&Hero, &Transform)>,
    mut wires: Query<(Entity, &TetherWire, &mut Cable), Without<Hero>>,
) {
    let mut seen_pending = false;
    let mut have: Vec<Entity> = Vec::new();
    for (wire, w, mut cable) in &mut wires {
        match w.0 {
            Some(t) => {
                let Ok((_, tether)) = tethers.get(t) else {
                    commands.entity(wire).try_despawn();
                    continue;
                };
                have.push(t);
                if let (Some(a), Some(b)) = (
                    end_world(&tether.ends[0], &poses),
                    end_world(&tether.ends[1], &poses),
                ) {
                    // wound in tight, it is straight; with length to spare it sags
                    let spare = (1.0 - a.distance(b) / tether.rest.max(0.1)).clamp(0.0, 1.0);
                    *cable = Cable {
                        a,
                        b,
                        slack: spare * 2.0,
                    };
                }
            }
            None => {
                seen_pending = true;
                match (
                    state.pending.as_ref().and_then(|p| end_world(p, &poses)),
                    heroes.single(),
                ) {
                    (Some(end), Ok((hero, h))) => {
                        *cable = Cable {
                            a: crate::grapple::default_hand(h.translation, hero.facing),
                            b: end,
                            slack: 0.15,
                        };
                    }
                    _ => commands.entity(wire).try_despawn(),
                }
            }
        }
    }
    let spawn_wire = |commands: &mut Commands, owner: Option<Entity>| {
        commands.spawn((
            Cable::default(),
            TetherWire(owner),
            Name::new("tether wire"),
        ));
    };
    for (t, _) in &tethers {
        if !have.contains(&t) {
            spawn_wire(&mut commands, Some(t));
        }
    }
    if state.pending.is_some() && !seen_pending {
        spawn_wire(&mut commands, None);
    }
}

/// The system that reads the stunt keys (`crew.rs` hijacks before it).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct StuntInput;

/// Any route that removes support (including crew seating) clears transient state.
fn stunt_removed(remove: On<Remove, OnStunt>, mut commands: Commands) {
    commands
        .entity(remove.entity)
        .try_remove::<(CurrentStuntSpot, StuntTransfer, WireSwing, WireTilt)>();
}

pub struct RoofPlugin;

impl Plugin for RoofPlugin {
    fn build(&self, app: &mut App) {
        use crate::character::{grapple_fire, hero_input, hero_physics, vehicle_entry};
        app.init_resource::<StuntCatalog>()
            .add_observer(spots_for_vehicle)
            .add_observer(stunt_removed)
            .add_systems(
                Update,
                (
                    mount_input
                        .in_set(StuntInput)
                        .after(hero_input)
                        .before(vehicle_entry),
                    hook_target.after(grapple_fire),
                    ride_at_start,
                    stunt_camera.after(crate::character::hero_camera),
                ),
            )
            .add_systems(
                FixedUpdate,
                (
                    stick_hook.before(hero_physics),
                    (wire_catch, hook_arrival).chain().after(hero_physics),
                ),
            )
            .add_systems(
                PostUpdate,
                hang_wire_visual
                    .after(bevy::transform::TransformSystems::Propagate)
                    .before(crate::grapple::draw_cables),
            )
            .add_systems(FixedPostUpdate, follow_vehicle.in_set(PhysicsSystems::Last));
    }
}

pub struct TetherPlugin;

impl Plugin for TetherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<TetherShots>()
            .init_resource::<TetherState>()
            .add_observer(tether_removed)
            .add_observer(body_removed)
            .add_systems(Update, (tether_input, tether_shots, tether_visuals).chain())
            .add_systems(FixedPreUpdate, tether_step);
    }
}

/// Both halves.
pub struct StuntPlugin;

impl Plugin for StuntPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((RoofPlugin, TetherPlugin));
    }
}

#[cfg(test)]
pub(crate) mod tests;
