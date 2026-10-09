//! The grappling hook, beyond a ray and a pull: the hook *flies* out of the gauntlet
//! (at `#59e07a13` m/s, assumed to be its speed) and attaches on arrival or, past the
//! range, comes back; the crosshair *locks on* to what it can take hold of; reeling
//! speeds up and brakes into the anchor; on arrival Rico lands, clings to a wall, hangs
//! from a ledge or an overhang (`Space` jumps or climbs, `Ctrl` drops), or is let go at a
//! vehicle; light things and soldiers are pulled to him instead; and one cable renderer
//! draws every wire (the hook's, the helicopter's, the tethers').
//!
//! The state lives in [`GrappleRig`] on Rico (`Hero::grapple` is still "the wire holds
//! him"); `character.rs` calls [`try_fire`] for `F`, [`hang_inputs`] and [`drive_attached`]
//! from its fixed step.

use crate::character::{Grapple, HEIGHT, Hero, HeroInput, HeroParams, RADIUS, heading};
use crate::coords::MODEL_FORWARD;
use crate::npc::{Npc, follow_ground};
use crate::stunts::StuntSpots;
use crate::vehicle::{CameraMode, Enterable};
use crate::world::FlyCamera;
use avian3d::prelude::*;
use bevy::prelude::*;

mod class;
/// The classification API JC2-038 / JC2-039 consume.
#[allow(unused_imports)]
pub use class::{
    Basis, ClassInput, FALLBACK_LIGHT_MASS, GrappleClass, GrappleClassification, GrappleWeight,
    WeightCategory, classify,
};

// ---------------------------------------------------------------- tuning

/// Retracting after a miss or a release: the hook comes home this many times faster
/// than it flew out.
const RETRACT_FACTOR: f32 = 2.0;
/// Reeling: the speed-up (m/s²) and the braking (m/s²) the controller may use. The first
/// is the original's yank softened; the second sets the speed it carries into the anchor:
/// `sqrt(2·BRAKE·distance)`, so he comes to rest on the spot.
pub const REEL_ACCEL: f32 = 45.0;
pub const REEL_BRAKE: f32 = 50.0;
/// Within this (m) of the goal he has arrived.
const ARRIVE_EPS: f32 = 0.12;
/// The longest a reel takes (s) before the wire gives up.
const REEL_TIMEOUT: f32 = 8.0;
/// A hooked vehicle lets him go this near (m); `stunts.rs` lands him on its stunt point.
pub const VEHICLE_RELEASE: f32 = 2.0;
/// How high above the hook (m) a wall's top edge may be for it to count as a ledge.
pub const LEDGE_REACH: f32 = 2.5;
/// Hanging from the hook: his middle this far below it (m).
const HANG_BELOW: f32 = 0.55;
/// Hanging at an overhang: his middle this far below the ceiling (m).
const OVERHANG_BELOW: f32 = 0.75;
/// Jumping off a wall: away from it, and up (m/s).
const WALL_JUMP_OUT: f32 = 4.5;
const WALL_JUMP_UP: f32 = 5.5;
/// Climbing a ledge: the speed into the wall while he rises (m/s).
const CLIMB_IN: f32 = 2.2;
/// ... for at most this long (s).
const CLIMB_TIME: f32 = 1.2;
/// A pulled prop stops (m from Rico) and lets go; its speed is capped (m/s).
const PROP_STOP: f32 = 2.5;
const PROP_MAX_SPEED: f32 = 20.0;
const PROP_TIMEOUT: f32 = 6.0;
/// A pulled soldier: where he is let go (m), his speed cap (m/s), his mass (kg) and how
/// long he lies there (s). Assumed numbers.
const SOLDIER_STOP: f32 = 1.8;
const SOLDIER_MAX_SPEED: f32 = 14.0;
const SOLDIER_MASS: f32 = 80.0;
const SOLDIER_LYING: f32 = 1.4;
/// Lock-on magnetism: the cone (degrees) around the crosshair in which a vehicle, a
/// soldier, a light prop or a ledge pulls the aim to it.
pub const MAGNET_DEG: f32 = 4.0;
/// The cable: segments, the sag a slack wire has (fraction of its length) and the
/// thinnest radius (m); farther away it is drawn a little thicker so it keeps a pixel.
const CABLE_SEGMENTS: usize = 14;
const CABLE_SAG: f32 = 0.06;
const CABLE_MIN_RADIUS: f32 = 0.006;
const CABLE_PIXEL: f32 = 0.001;

/// The numbers from `grapplingparams.bin` (with the fallbacks when it is not there).
#[derive(Clone, Copy, Debug)]
pub struct Tuning {
    pub range: f32,
    pub reel_speed: f32,
    pub hook_speed: f32,
    pub pull_object: f32,
    pub pull_character: f32,
    pub impulse_object: f32,
    pub impulse_character: f32,
}

impl Tuning {
    pub fn new(params: Option<&HeroParams>) -> Self {
        match params {
            Some(p) => Self {
                range: p.grapple.range,
                reel_speed: p.grapple.reel_in_speed,
                hook_speed: p.grapple.hook_speed,
                pull_object: p.grapple.pull_force,
                pull_character: p.grapple.pull_force_character,
                impulse_object: p.grapple.object_impulse,
                impulse_character: p.grapple.character_impulse,
            },
            None => Self {
                range: 90.0,
                reel_speed: 40.0,
                hook_speed: 120.0,
                pull_object: 500.0,
                pull_character: 850.0,
                impulse_object: 200.0,
                impulse_character: 200.0,
            },
        }
    }
}

// ---------------------------------------------------------------- state

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// The hook is flying out.
    Flight,
    /// The hook is coming home (a miss, a release).
    Retract,
    /// The wire holds and winds him in.
    Reel,
    /// Reeled in to a wall, ledge or overhang: he hangs there.
    Hang,
    /// A soldier or a light prop is being pulled to him.
    Pull,
}

/// What happens at the end of the reel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Arrival {
    /// A floor: he lands and the wire lets go.
    #[default]
    Land,
    /// A wall (or a ledge when `ledge_top` is set): he clings.
    Cling,
    /// A ceiling: he hangs below it.
    Overhang,
    /// A vehicle or another moving thing: the wire lets go near it (`stunts.rs` takes over).
    Follow,
}

/// What the crosshair is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    /// Plain ground or wall: the hook holds, but nothing is highlighted.
    Surface,
    /// A wall with an edge to climb on within reach.
    Ledge,
    Vehicle,
    Character,
    Object,
}

#[derive(Clone, Copy, Debug)]
pub struct HookHit {
    pub normal: Vec3,
    /// The body that was hit (a vehicle, a soldier, a prop), if it moves.
    pub body: Option<Entity>,
    pub kind: TargetKind,
    /// A soldier or a light prop: pulled to Rico instead of reeling him in.
    pub pull: bool,
}

/// The hook on its way out.
#[derive(Clone, Copy, Debug)]
pub struct Flight {
    pub origin: Vec3,
    pub target: Vec3,
    pub total: f32,
    pub travelled: f32,
    pub speed: f32,
    pub hit: Option<HookHit>,
    /// The body the target rides on and the point in its frame.
    pub track: Option<(Entity, Vec3)>,
}

#[derive(Clone, Copy, Debug)]
pub struct PullInfo {
    pub body: Entity,
    pub local: Vec3,
    pub soldier: bool,
    pub age: f32,
    pub fresh: bool,
}

/// Rico's end of the hook's state.
#[derive(Component, Clone, Debug, Default)]
pub struct GrappleRig {
    pub phase: Phase,
    /// The gauntlet, world space (`None` until the launcher's joint is known).
    pub hand: Option<Vec3>,
    pub flight: Option<Flight>,
    /// Where the hook is drawn, and which way it points.
    pub hook: Vec3,
    pub hook_rot: Quat,
    pub arrival: Arrival,
    /// Where the reel takes him (world; the anchor itself for [`Arrival::Follow`]).
    pub goal: Vec3,
    pub ledge_top: Option<f32>,
    /// The body the anchor rides on and the point in its frame.
    pub track: Option<(Entity, Vec3)>,
    pub pull: Option<PullInfo>,
    /// His speed along the wire (m/s).
    pub reel: f32,
    stall: f32,
    /// How slack the wire is drawn: 0 taut, 1 hanging.
    pub slack: f32,
    /// Seconds he still presses into the wall while he climbs over a ledge, and which way.
    pub climb: f32,
    pub climb_dir: Vec3,
}

impl GrappleRig {
    /// The gauntlet's position, or where it would be on his body.
    pub fn hand_or(&self, hero_pos: Vec3, facing: f32) -> Vec3 {
        self.hand.unwrap_or_else(|| default_hand(hero_pos, facing))
    }

    fn retract(&mut self) {
        self.phase = Phase::Retract;
        self.flight = None;
        self.pull = None;
        self.track = None;
        self.reel = 0.0;
        self.stall = 0.0;
        self.slack = 0.8;
    }
}

/// The left hand, where the launcher sits (−x is his left, −z ahead of him).
pub fn default_hand(hero_pos: Vec3, facing: f32) -> Vec3 {
    hero_pos + Quat::from_rotation_y(facing) * Vec3::new(-0.3, 0.45, -0.2)
}

/// A mesh on the arm that marks the launcher; its joint's position is the hand.
#[derive(Component)]
pub struct GrappleLauncher;

/// A soldier on the hook: pulled off his feet to Rico, lying a moment, then up again.
#[derive(Component, Debug, Clone, Copy)]
pub struct Hooked {
    pub speed: f32,
    /// 0 upright .. 1 on his back.
    pub lean: f32,
    /// Seconds he still lies after the wire let go.
    pub down: f32,
    pub toward: Vec3,
    pub pulled: bool,
}

// ---------------------------------------------------------------- the lock-on

#[derive(Clone, Copy, Debug)]
pub struct LockTarget {
    pub kind: TargetKind,
    pub point: Vec3,
    pub normal: Vec3,
    /// The body (vehicle, soldier, prop) for a moving target.
    pub body: Option<Entity>,
    pub distance: f32,
    /// How far off the crosshair the target's middle is (radians).
    pub angle: f32,
    /// Pulled in by the magnetism rather than under the crosshair.
    pub magnet: bool,
}

/// What the crosshair is over, for the HUD (`hud.rs` tints the reticle green when
/// [`LockOn::locked`]).
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct LockOn {
    pub target: Option<LockTarget>,
}

impl LockOn {
    /// A vehicle, soldier, prop or ledge is in the sights: the green lock-on.
    pub fn locked(&self) -> bool {
        self.target.is_some_and(|t| t.kind != TargetKind::Surface)
    }

    /// The reticle's colour: green on a lock, white otherwise.
    pub fn tint(&self) -> Color {
        if self.locked() {
            Color::srgb(0.25, 1.0, 0.35)
        } else {
            Color::WHITE
        }
    }
}

pub type Bodies<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ColliderAabb,
        &'static RigidBody,
        Has<Npc>,
        Has<StuntSpots>,
        Has<Enterable>,
        Option<&'static ComputedMass>,
        Option<&'static GrappleWeight>,
    ),
    Without<Hero>,
>;

/// Classify the body (see [`class`]); `None` when the hook treats it as plain surface.
pub fn body_class(
    rb: &RigidBody,
    npc: bool,
    stunt: bool,
    enterable: bool,
    mass: Option<&ComputedMass>,
    weight: Option<&GrappleWeight>,
) -> Option<GrappleClassification> {
    let c = classify(ClassInput {
        character: npc,
        vehicle: !npc && (stunt || enterable),
        dynamic: *rb == RigidBody::Dynamic,
        category: weight.map(|w| w.0),
        mass: mass.map(|m| m.value()),
    });
    (c.class != GrappleClass::Static).then_some(c)
}

fn body_kind(
    rb: &RigidBody,
    npc: bool,
    stunt: bool,
    enterable: bool,
    mass: Option<&ComputedMass>,
    weight: Option<&GrappleWeight>,
) -> Option<(TargetKind, bool)> {
    let c = body_class(rb, npc, stunt, enterable, mass, weight)?;
    let kind = match c.class {
        GrappleClass::Character => TargetKind::Character,
        GrappleClass::Vehicle => TargetKind::Vehicle,
        _ => TargetKind::Object,
    };
    Some((kind, c.pull))
}

/// The top of the wall's edge above `point` (world y) when `normal` is a wall and its top
/// is within [`LEDGE_REACH`].
pub fn find_ledge(
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
    point: Vec3,
    normal: Vec3,
) -> Option<f32> {
    if normal.y.abs() > 0.5 {
        return None;
    }
    let into = -Vec3::new(normal.x, 0.0, normal.z).normalize_or_zero();
    if into == Vec3::ZERO {
        return None;
    }
    let probe = point + into * 0.25 + Vec3::Y * LEDGE_REACH;
    let hit = spatial.cast_ray(probe, Dir3::NEG_Y, LEDGE_REACH + 0.3, true, filter)?;
    let top = probe.y - hit.distance;
    (hit.distance > 0.01 && hit.normal.y > 0.7 && top > point.y + 0.15).then_some(top)
}

/// One ray from the camera: the hit (point, normal, body, distance from the camera),
/// no nearer than Rico himself.
fn aim_ray(
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
    origin: Vec3,
    dir: Dir3,
    range: f32,
    to_hero: f32,
) -> Option<(Vec3, Vec3, Entity, f32)> {
    let hit = spatial.cast_ray(origin, dir, range + to_hero, true, filter)?;
    (hit.distance > to_hero + 0.25).then(|| {
        (
            origin + dir * hit.distance,
            hit.normal,
            hit.entity,
            hit.distance,
        )
    })
}

/// The lock-on: what the crosshair is over, within range, with magnetism towards
/// vehicles, soldiers, light props (their middle within [`MAGNET_DEG`] of the
/// crosshair's ray, widened by their size) and ledges (a ring of rays at the same angle).
#[allow(clippy::too_many_arguments)]
pub fn update_lock_on(
    mode: Res<CameraMode>,
    params: Option<Res<HeroParams>>,
    spatial: SpatialQuery,
    mut lock: ResMut<LockOn>,
    heroes: Query<(Entity, &Hero, &Transform)>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    colliders: Query<&ColliderOf>,
    bodies: Bodies,
) {
    lock.target = None;
    if *mode != CameraMode::Hero {
        return;
    }
    let (Ok((hero_entity, hero, tf)), Ok(cam)) = (heroes.single(), cams.single()) else {
        return;
    };
    if hero.in_vehicle.is_some() {
        return;
    }
    let range = Tuning::new(params.as_deref()).range;
    let dir = cam.forward();
    let to_hero = (tf.translation - cam.translation)
        .dot(dir.as_vec3())
        .max(0.0);
    let filter = SpatialQueryFilter::from_excluded_entities([hero_entity]);
    let body_of = |e: Entity| colliders.get(e).map(|c| c.body).unwrap_or(e);
    let special = |e: Entity| {
        bodies
            .get(e)
            .ok()
            .and_then(|(_, _, rb, npc, stunt, enter, mass, w)| {
                body_kind(rb, npc, stunt, enter, mass, w)
            })
    };
    let hit = aim_ray(&spatial, &filter, cam.translation, dir, range, to_hero);
    // under the crosshair: something that moves
    if let Some((point, normal, entity, dist)) = hit {
        let body = body_of(entity);
        if let Some((kind, _)) = special(body) {
            lock.target = Some(LockTarget {
                kind,
                point,
                normal,
                body: Some(body),
                distance: dist - to_hero,
                angle: 0.0,
                magnet: false,
            });
            return;
        }
    }
    // magnetism: the nearest middle inside the cone
    let cone = MAGNET_DEG.to_radians();
    let mut best: Option<(f32, Entity, Vec3, TargetKind)> = None;
    for (e, aabb, rb, npc, stunt, enter, mass, w) in &bodies {
        let Some((kind, _)) = body_kind(rb, npc, stunt, enter, mass, w) else {
            continue;
        };
        let centre = (aabb.min + aabb.max) * 0.5;
        let radius = ((aabb.max - aabb.min) * 0.5).length().min(4.0);
        let v = centre - cam.translation;
        let d = v.length();
        if d < to_hero + 1.0 || (centre - tf.translation).length() > range {
            continue;
        }
        let angle = v.angle_between(dir.as_vec3());
        let off = (angle - (radius / d).atan()).max(0.0);
        if off <= cone && best.is_none_or(|b| off < b.0) {
            best = Some((off, e, centre, kind));
        }
    }
    if let Some((off, body, centre, kind)) = best {
        // onto its surface when a ray from the camera finds it, else its middle
        let toward = Dir3::new(centre - cam.translation).unwrap_or(dir);
        let surface = spatial
            .cast_ray(cam.translation, toward, range + to_hero, true, &filter)
            .filter(|h| body_of(h.entity) == body)
            .map(|h| (cam.translation + toward * h.distance, h.normal));
        let (point, normal) = surface.unwrap_or((centre, Vec3::Y));
        lock.target = Some(LockTarget {
            kind,
            point,
            normal,
            body: Some(body),
            distance: point.distance(tf.translation),
            angle: off,
            magnet: true,
        });
        return;
    }
    // a wall with an edge: under the crosshair, or just off it
    let ledge_at = |h: &(Vec3, Vec3, Entity, f32)| find_ledge(&spatial, &filter, h.0, h.1);
    let right = dir.cross(Vec3::Y).normalize_or_zero();
    let mut samples = vec![hit];
    for (dy, dx) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let tilt = Quat::from_axis_angle(right, dy * cone * 0.75)
            * Quat::from_axis_angle(Vec3::Y, -dx * cone * 0.75);
        if let Ok(d) = Dir3::new(tilt * dir.as_vec3()) {
            samples.push(aim_ray(
                &spatial,
                &filter,
                cam.translation,
                d,
                range,
                to_hero,
            ));
        }
    }
    for (i, s) in samples.iter().enumerate() {
        if let Some(h) = s
            && ledge_at(h).is_some()
        {
            lock.target = Some(LockTarget {
                kind: TargetKind::Ledge,
                point: h.0,
                normal: h.1,
                body: None,
                distance: h.3 - to_hero,
                angle: if i == 0 { 0.0 } else { cone * 0.75 },
                magnet: i != 0,
            });
            return;
        }
    }
    if let Some((point, normal, _, dist)) = hit {
        lock.target = Some(LockTarget {
            kind: TargetKind::Surface,
            point,
            normal,
            body: None,
            distance: dist - to_hero,
            angle: 0.0,
            magnet: false,
        });
    }
}

// ---------------------------------------------------------------- firing

/// `F` (without `Ctrl`): fire the hook at the lock-on or what the camera's ray hits, cut
/// the flight short, or let go. A miss flies out to the range and comes back.
#[allow(clippy::too_many_arguments)]
pub fn try_fire(
    hero_entity: Entity,
    hero: &mut Hero,
    rig: &mut GrappleRig,
    hero_pos: Vec3,
    cam: &Transform,
    lock: &LockOn,
    spatial: &SpatialQuery,
    tuning: &Tuning,
    poses: &Query<(&Position, &Rotation)>,
    colliders: &Query<&ColliderOf>,
    bodies: &Bodies,
) {
    if hero.grapple.is_some() || rig.phase == Phase::Pull {
        release(hero, rig);
        return;
    }
    if rig.phase == Phase::Flight {
        rig.retract();
        return;
    }
    let origin = rig.hand_or(hero_pos, hero.facing);
    let dir = cam.forward();
    let to_hero = (hero_pos - cam.translation).dot(dir.as_vec3()).max(0.0);
    let filter = SpatialQueryFilter::from_excluded_entities([hero_entity]);
    // what we hit: the lock-on, else the ray
    let found = match lock.target {
        Some(t) => Some((t.point, t.normal, t.body, t.kind)),
        None => aim_ray(
            spatial,
            &filter,
            cam.translation,
            dir,
            tuning.range,
            to_hero,
        )
        .map(|(p, n, e, _)| {
            let body = colliders.get(e).map(|c| c.body).unwrap_or(e);
            let kind = bodies
                .get(body)
                .ok()
                .and_then(|(_, _, rb, npc, stunt, enter, mass, w)| {
                    body_kind(rb, npc, stunt, enter, mass, w)
                });
            match kind {
                Some((k, _)) => (p, n, Some(body), k),
                None => (p, n, None, TargetKind::Surface),
            }
        }),
    };
    let (target, hit) = match found {
        Some((point, normal, body, kind)) if point.distance(origin) <= tuning.range + 2.0 => {
            let pull = body
                .and_then(|b| bodies.get(b).ok())
                .and_then(|(_, _, rb, npc, stunt, enter, mass, w)| {
                    body_kind(rb, npc, stunt, enter, mass, w)
                })
                .is_some_and(|(_, p)| p);
            (
                point,
                Some(HookHit {
                    normal,
                    body,
                    kind,
                    pull,
                }),
            )
        }
        _ => {
            // nothing in range: the hook flies to the end of its line and comes back
            let aim = cam.translation + dir * (tuning.range + to_hero);
            let d = (aim - origin).normalize_or(dir.as_vec3());
            (origin + d * tuning.range, None)
        }
    };
    let track = hit.and_then(|h| h.body).and_then(|b| {
        poses
            .get(b)
            .ok()
            .map(|(p, r)| (b, r.0.inverse() * (target - p.0)))
    });
    let to = target - origin;
    hero.facing = heading(to, hero.facing);
    rig.flight = Some(Flight {
        origin,
        target,
        total: to.length().max(0.1),
        travelled: 0.0,
        speed: tuning.hook_speed.max(1.0),
        hit,
        track,
    });
    rig.phase = Phase::Flight;
    rig.hook = origin;
    rig.hook_rot = Quat::from_rotation_arc(MODEL_FORWARD, to.normalize_or(Vec3::NEG_Z));
    rig.slack = 0.6;
    rig.pull = None;
    rig.track = None;
}

/// Let go of everything: the wire, a hooked soldier or prop. Rico keeps his velocity.
pub fn release(hero: &mut Hero, rig: &mut GrappleRig) {
    hero.grapple = None;
    hero.hanging = false;
    rig.retract();
}

// ---------------------------------------------------------------- the flight

/// The hook in the air: out at `hook_speed`, attached on arrival (reel him in, or pull
/// the soldier or prop) or coming home after a miss.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn hook_flight(
    time: Res<Time>,
    params: Option<Res<HeroParams>>,
    spatial: SpatialQuery,
    poses: Query<(&Position, &Rotation), Without<Hero>>,
    mut heroes: Query<(Entity, &mut Hero, &Transform, &mut GrappleRig)>,
) {
    let dt = time.delta_secs();
    let tuning = Tuning::new(params.as_deref());
    let Ok((entity, mut hero, tf, mut rig)) = heroes.single_mut() else {
        return;
    };
    let hand = rig.hand_or(tf.translation, hero.facing);
    match rig.phase {
        Phase::Flight => {
            let Some(mut f) = rig.flight else {
                rig.phase = Phase::Idle;
                return;
            };
            if let Some((b, local)) = f.track
                && let Ok((p, r)) = poses.get(b)
            {
                f.target = p.0 + r.0 * local;
                f.total = f.origin.distance(f.target).max(0.1);
            }
            f.travelled += f.speed * dt;
            let dir = (f.target - f.origin).normalize_or(Vec3::NEG_Z);
            rig.hook_rot = Quat::from_rotation_arc(MODEL_FORWARD, dir);
            if f.travelled < f.total {
                rig.hook = f.origin + dir * f.travelled;
                rig.slack = 0.6 * (1.0 - f.travelled / f.total);
                rig.flight = Some(f);
                return;
            }
            rig.hook = f.target;
            rig.flight = None;
            let Some(hit) = f.hit else {
                // a miss: home again
                rig.retract();
                return;
            };
            if hit.pull
                && let Some(body) = hit.body
            {
                rig.phase = Phase::Pull;
                rig.pull = Some(PullInfo {
                    body,
                    local: f.track.map(|t| t.1).unwrap_or(Vec3::ZERO),
                    soldier: hit.kind == TargetKind::Character,
                    age: 0.0,
                    fresh: true,
                });
                rig.slack = 0.0;
                return;
            }
            // heavy or fixed: the wire holds and he is reeled in
            hero.grapple = Some(Grapple {
                anchor: f.target,
                normal: hit.normal,
                age: 0.0,
            });
            hero.grounded = false;
            let filter = SpatialQueryFilter::from_excluded_entities([entity]);
            begin_reel(&mut rig, f.target, hit.normal, f.track, &spatial, &filter);
        }
        Phase::Retract => {
            let to = hand - rig.hook;
            let d = to.length();
            let step = tuning.hook_speed * RETRACT_FACTOR * dt;
            rig.slack = 0.8;
            if d <= step.max(0.4) {
                rig.phase = Phase::Idle;
            } else {
                rig.hook += to / d * step;
            }
        }
        _ => {}
    }
}

/// Work out where the reel ends for a wire that has just taken hold at `anchor`.
pub fn begin_reel(
    rig: &mut GrappleRig,
    anchor: Vec3,
    normal: Vec3,
    track: Option<(Entity, Vec3)>,
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
) {
    let n = normal.normalize_or(Vec3::Y);
    rig.phase = Phase::Reel;
    rig.flight = None;
    rig.pull = None;
    rig.track = track;
    rig.reel = 0.0;
    rig.stall = 0.0;
    rig.ledge_top = None;
    rig.hook = anchor;
    rig.hook_rot = Quat::from_rotation_arc(MODEL_FORWARD, -n);
    rig.slack = 0.0;
    if track.is_some() {
        rig.arrival = Arrival::Follow;
        rig.goal = anchor;
        return;
    }
    if n.y > 0.7 {
        rig.arrival = Arrival::Land;
        rig.goal = anchor + n * (HEIGHT / 2.0 + 0.12);
    } else if n.y < -0.7 {
        rig.arrival = Arrival::Overhang;
        rig.goal = anchor + n * OVERHANG_BELOW;
    } else {
        let flat = Vec3::new(n.x, 0.0, n.z).normalize_or_zero();
        let mut goal = anchor + flat * (RADIUS + 0.1) - Vec3::Y * HANG_BELOW;
        rig.arrival = Arrival::Cling;
        rig.ledge_top = find_ledge(spatial, filter, anchor, n);
        // too near the ground to hang: he walks up to it and stands
        if let Some(h) = spatial.cast_ray(goal, Dir3::NEG_Y, HEIGHT / 2.0 + 0.2, true, filter)
            && h.distance < HEIGHT / 2.0 + 0.1
        {
            goal.y = goal.y - h.distance + HEIGHT / 2.0 + 0.05;
            rig.arrival = Arrival::Land;
            rig.ledge_top = None;
        }
        rig.goal = goal;
    }
}

/// While the wire holds a moving thing, its end and the goal ride on it.
fn track_anchor(
    poses: Query<(&Position, &Rotation), Without<Hero>>,
    mut heroes: Query<(&mut Hero, &mut GrappleRig)>,
) {
    let Ok((mut hero, mut rig)) = heroes.single_mut() else {
        return;
    };
    let Some((b, local)) = rig.track else { return };
    let Some(g) = hero.grapple.as_mut() else {
        return;
    };
    match poses.get(b) {
        Ok((p, r)) => {
            g.anchor = p.0 + r.0 * local;
            rig.goal = g.anchor;
            rig.hook = g.anchor;
        }
        Err(_) => {
            hero.grapple = None;
        }
    }
}

/// Keep the rig and Rico agreeing: a wire somebody else cut, or a vehicle seat, ends it.
fn sync_rig(mut heroes: Query<(&mut Hero, &mut GrappleRig)>) {
    let Ok((mut hero, mut rig)) = heroes.single_mut() else {
        return;
    };
    if matches!(rig.phase, Phase::Reel | Phase::Hang) && hero.grapple.is_none() {
        hero.hanging = false;
        rig.retract();
    }
    if hero.grapple.is_none() {
        hero.hanging = false;
    }
    if (hero.in_vehicle.is_some() || hero.stunt.is_some())
        && matches!(rig.phase, Phase::Flight | Phase::Pull)
    {
        rig.retract();
    }
}

// ---------------------------------------------------------------- reeling and hanging

/// Climbing over a ledge: he keeps pushing into the wall while he rises (the wall would
/// otherwise kill the sideways speed and he would slide back down), until he stands.
pub fn climb_assist(hero: &Hero, rig: &mut GrappleRig, v: &mut Vec3, dt: f32) {
    if rig.climb <= 0.0 {
        return;
    }
    rig.climb -= dt;
    if hero.grounded || hero.grapple.is_some() {
        rig.climb = 0.0;
        return;
    }
    v.x = rig.climb_dir.x * CLIMB_IN;
    v.z = rig.climb_dir.z * CLIMB_IN;
}

/// The `Space` and `Ctrl` of a hanging Rico, before `character.rs` reads `Space` as the
/// chute: `Space` climbs a ledge or leaps off a wall, `Ctrl` drops.
pub fn hang_inputs(
    hero: &mut Hero,
    rig: &mut GrappleRig,
    input: &mut HeroInput,
    v: &mut Vec3,
    pos: Vec3,
) {
    if rig.phase != Phase::Hang || hero.grapple.is_none() {
        return;
    }
    let normal = hero.grapple.map(|g| g.normal).unwrap_or(Vec3::Y);
    let flat = Vec3::new(normal.x, 0.0, normal.z).normalize_or_zero();
    if input.chute_toggle {
        input.chute_toggle = false;
        match (rig.arrival, rig.ledge_top) {
            (Arrival::Cling, Some(top)) => {
                // up and over the edge
                let rise = (top + HEIGHT / 2.0 + 0.25 - pos.y).max(0.5);
                *v = -flat * CLIMB_IN + Vec3::Y * ((2.0 * 9.81 * rise).sqrt() + 0.3);
                rig.climb = CLIMB_TIME;
                rig.climb_dir = -flat;
            }
            (Arrival::Cling, None) => {
                *v = flat * WALL_JUMP_OUT + Vec3::Y * WALL_JUMP_UP;
            }
            _ => *v = Vec3::ZERO,
        }
        release(hero, rig);
        return;
    }
    if input.drop {
        *v = Vec3::ZERO;
        release(hero, rig);
    }
}

/// One fixed step of the wire: wind him in with a speed that rises and brakes into the
/// goal, then land, cling, hang or let go. `v` is his velocity going in and out.
#[allow(clippy::too_many_arguments)]
pub fn drive_attached(
    hero: &mut Hero,
    rig: &mut GrappleRig,
    pos: Vec3,
    v: &mut Vec3,
    tuning: &Tuning,
    dt: f32,
    spatial: &SpatialQuery,
    filter: &SpatialQueryFilter,
) {
    let Some(mut g) = hero.grapple else {
        hero.hanging = false;
        return;
    };
    if !matches!(rig.phase, Phase::Reel | Phase::Hang) {
        // somebody set the wire without a flight (a test, a mission): take it as it is
        let track = rig.track;
        begin_reel(rig, g.anchor, g.normal, track, spatial, filter);
    }
    g.age += dt;
    rig.hook = g.anchor;
    hero.grapple = Some(g);
    if rig.phase == Phase::Hang {
        hero.hanging = true;
        rig.slack = 0.02;
        let flat = Vec3::new(g.normal.x, 0.0, g.normal.z);
        hero.facing = heading(-flat, hero.facing);
        // hold the spot: a spring, with the step's gravity cancelled
        *v = ((rig.goal - pos) * 10.0).clamp_length_max(6.0);
        v.y += 9.81 * dt;
        return;
    }
    hero.hanging = false;
    rig.slack = 0.0;
    let to = rig.goal - pos;
    let dist = to.length();
    if g.age > REEL_TIMEOUT {
        release(hero, rig);
        return;
    }
    if rig.arrival == Arrival::Follow {
        if g.anchor.distance(pos) < VEHICLE_RELEASE {
            release(hero, rig);
            return;
        }
    } else {
        // no progress for a while close to it (a corner in the way): arrived
        let closing = v.dot(to.normalize_or_zero());
        if g.age > 0.5 && closing < 0.5 && dist < 4.0 {
            rig.stall += dt;
        } else {
            rig.stall = 0.0;
        }
        if dist < ARRIVE_EPS || rig.stall > 0.3 {
            arrive(hero, rig, pos, v);
            return;
        }
    }
    let dir = to / dist.max(1e-4);
    let closing = v.dot(dir);
    // a taut wire: the sideways motion dies off quickly so he flies straight at the hook
    let sideways = *v - dir * closing;
    *v -= sideways * (1.0 - (-dt / 0.15).exp());
    // the speed he may have here: up to the reel speed, braking into the goal
    let stop = if rig.arrival == Arrival::Follow {
        VEHICLE_RELEASE
    } else {
        0.0
    };
    let cmd = tuning
        .reel_speed
        .min((2.0 * REEL_BRAKE * (dist - stop).max(0.0)).sqrt());
    let step = ((cmd - closing).clamp(-REEL_BRAKE * dt, REEL_ACCEL * dt)).max(-closing.max(0.0));
    *v += dir * step;
    // never faster than the last step can use: no overshoot
    let now = v.dot(dir);
    if now > dist / dt * 0.6 && rig.arrival != Arrival::Follow {
        *v -= dir * (now - dist / dt * 0.6);
    }
    rig.reel = v.dot(dir).max(0.0);
    // the step's gravity would bend the line: cancel it
    v.y += 9.81 * dt;
    hero.facing = heading(dir, hero.facing);
}

fn arrive(hero: &mut Hero, rig: &mut GrappleRig, pos: Vec3, v: &mut Vec3) {
    *v = Vec3::ZERO;
    match rig.arrival {
        Arrival::Cling | Arrival::Overhang => {
            rig.phase = Phase::Hang;
            rig.reel = 0.0;
            // held where he is when a corner stopped him short
            if pos.distance(rig.goal) > 0.4 {
                rig.goal = pos;
            }
            hero.hanging = true;
        }
        _ => release(hero, rig),
    }
}

// ---------------------------------------------------------------- pulling things

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn pull_things(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    params: Option<Res<HeroParams>>,
    mut heroes: Query<(&Hero, &Transform, &mut GrappleRig)>,
    mut props: Query<
        (
            &mut LinearVelocity,
            &Position,
            &Rotation,
            Option<&ComputedMass>,
        ),
        (Without<Hero>, Without<Hooked>),
    >,
    mut soldiers: Query<(Entity, &mut Hooked, &mut Transform), Without<Hero>>,
) {
    let dt = time.delta_secs();
    let t = Tuning::new(params.as_deref());
    let Ok((hero, tf, mut rig)) = heroes.single_mut() else {
        return;
    };
    let me = tf.translation;
    let mut ended = false;
    if let (Phase::Pull, Some(mut info)) = (rig.phase, rig.pull) {
        info.age += dt;
        if info.soldier {
            match soldiers.get_mut(info.body) {
                Ok((_, mut h, _)) => h.pulled = true,
                Err(_) => {
                    if info.fresh {
                        commands.entity(info.body).try_insert(Hooked {
                            speed: t.impulse_character / SOLDIER_MASS,
                            lean: 0.0,
                            down: 0.0,
                            toward: Vec3::NEG_Z,
                            pulled: true,
                        });
                    } else {
                        ended = true;
                    }
                }
            }
        } else {
            match props.get_mut(info.body) {
                Ok((mut v, p, r, mass)) => {
                    let m = mass.map(|m| m.value()).unwrap_or(0.0);
                    let at = p.0 + r.0 * info.local;
                    let to = me - at;
                    let d = to.length();
                    if m <= 0.0 || d < PROP_STOP || info.age > PROP_TIMEOUT {
                        ended = true;
                    } else {
                        let dir = to / d;
                        commands.entity(info.body).try_remove::<Sleeping>();
                        if info.fresh {
                            v.0 += dir * (t.impulse_object / m);
                        }
                        let along = v.0.dot(dir);
                        let side = v.0 - dir * along;
                        v.0 -= side * (1.0 - (-3.0 * dt).exp());
                        if along < PROP_MAX_SPEED {
                            v.0 += dir * ((t.pull_object / m) * dt).min(PROP_MAX_SPEED - along);
                        }
                    }
                }
                Err(_) => ended = true,
            }
        }
        info.fresh = false;
        rig.pull = Some(info);
        rig.slack = 0.0;
        let _ = hero;
    }
    // the soldiers: dragged to him, lying a moment, up again
    for (e, mut h, mut stf) in &mut soldiers {
        let at = stf.translation;
        if h.pulled {
            let flat = Vec3::new(me.x - at.x, 0.0, me.z - at.z);
            let d = flat.length();
            if d < SOLDIER_STOP {
                h.pulled = false;
                h.down = SOLDIER_LYING;
                h.speed = 0.0;
                if rig.pull.is_some_and(|p| p.body == e) {
                    ended = true;
                }
            } else {
                let dir = flat / d;
                h.toward = dir;
                h.speed = (h.speed + t.pull_character / SOLDIER_MASS * dt).min(SOLDIER_MAX_SPEED);
                let step = (h.speed * dt).min(d - SOLDIER_STOP);
                stf.translation += dir * step;
                h.lean = (h.lean + dt / 0.25).min(1.0);
            }
        } else if h.down > 0.0 {
            h.down -= dt;
        } else {
            h.lean = (h.lean - dt / 0.4).max(0.0);
        }
        follow_ground(&spatial, e, &mut stf);
        let lean = h.lean;
        stf.rotation = if lean > 0.0 {
            let axis = Vec3::Y.cross(h.toward).normalize_or(Vec3::X);
            Quat::from_axis_angle(axis, -lean * 1.4)
        } else {
            Quat::IDENTITY
        };
        stf.translation.y -= lean * (HEIGHT / 2.0 - 0.3);
        if !h.pulled && h.down <= 0.0 && h.lean <= 0.0 {
            commands.entity(e).try_remove::<Hooked>();
        }
    }
    if ended && rig.phase == Phase::Pull {
        rig.retract();
    }
}

// ---------------------------------------------------------------- the cable

/// A thin cable between two points, drawn as a chain of short cylinders: it sags while
/// `slack` is above zero and is straight when it is taut. Every wire in the game (the
/// hook's, the helicopter's, the tethers') is one of these.
#[derive(Component, Clone, Copy, Debug, Default)]
#[require(Transform, Visibility)]
pub struct Cable {
    pub a: Vec3,
    pub b: Vec3,
    pub slack: f32,
}

#[derive(Component)]
pub(crate) struct CableSegments(Vec<Entity>);

/// The points of a cable: a parabola sagging `slack · 6 %` of the length.
pub fn cable_points(c: &Cable) -> [Vec3; CABLE_SEGMENTS + 1] {
    let len = c.a.distance(c.b);
    let sag = c.slack.clamp(0.0, 1.0) * CABLE_SAG * len;
    std::array::from_fn(|i| {
        let t = i as f32 / CABLE_SEGMENTS as f32;
        c.a.lerp(c.b, t) - Vec3::Y * sag * 4.0 * t * (1.0 - t)
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn draw_cables(
    mut commands: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    params: Option<Res<HeroParams>>,
    cams: Query<&GlobalTransform, With<FlyCamera>>,
    cables: Query<(Entity, &Cable, &GlobalTransform, Option<&CableSegments>)>,
    mut segs: Query<
        (&mut Transform, &mut GlobalTransform),
        (Without<Cable>, Without<Hero>, Without<FlyCamera>),
    >,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let (mesh, material) = assets
        .get_or_insert_with(|| {
            (
                meshes.add(Cylinder::new(1.0, 1.0)),
                materials.add(StandardMaterial {
                    base_color: Color::srgb(0.95, 0.95, 0.93),
                    unlit: true,
                    ..default()
                }),
            )
        })
        .clone();
    let base = params
        .map(|p| p.grapple.wire_width)
        .unwrap_or(CABLE_MIN_RADIUS)
        .max(CABLE_MIN_RADIUS);
    let cam = cams.single().map(|c| c.translation()).ok();
    for (e, cable, parent, have) in &cables {
        let pts = cable_points(cable);
        // thicker with distance, to stay about a pixel across
        let mid = (cable.a + cable.b) * 0.5;
        let radius = cam
            .map(|c| base.max(c.distance(mid) * CABLE_PIXEL))
            .unwrap_or(base.max(0.01));
        match have {
            Some(s) => {
                for (i, id) in s.0.iter().enumerate() {
                    if let Ok((mut t, mut global)) = segs.get_mut(*id) {
                        let world = crate::character::segment_transform(pts[i], pts[i + 1], radius);
                        *t = Transform::from_matrix(
                            Mat4::from(parent.affine().inverse()) * world.to_matrix(),
                        );
                        // We run after propagation: publish the segment's world frame
                        // now, so the rendered wire and final launcher share this frame.
                        *global = GlobalTransform::from(world);
                    }
                }
            }
            None => {
                let ids = (0..CABLE_SEGMENTS)
                    .map(|i| {
                        let world = crate::character::segment_transform(pts[i], pts[i + 1], radius);
                        let local = Transform::from_matrix(
                            Mat4::from(parent.affine().inverse()) * world.to_matrix(),
                        );
                        commands
                            .spawn((
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(material.clone()),
                                local,
                                GlobalTransform::from(world),
                                ChildOf(e),
                            ))
                            .id()
                    })
                    .collect();
                commands.entity(e).insert(CableSegments(ids));
            }
        }
    }
}

// ---------------------------------------------------------------- plumbing

/// The launcher's joint is the hand: the wire starts there.
fn mark_launcher(add: On<Add, Name>, names: Query<&Name>, mut commands: Commands) {
    if names
        .get(add.entity)
        .is_ok_and(|n| n.as_str() == "grapple launcher")
    {
        commands.entity(add.entity).insert(GrappleLauncher);
    }
}

fn give_rig(add: On<Add, Hero>, rigs: Query<&GrappleRig>, mut commands: Commands) {
    if rigs.get(add.entity).is_err() {
        commands.entity(add.entity).insert(GrappleRig::default());
    }
}

fn track_hand(
    launchers: Query<&GlobalTransform, With<GrappleLauncher>>,
    mut heroes: Query<&mut GrappleRig>,
) {
    let (Some(l), Ok(mut rig)) = (launchers.iter().next(), heroes.single_mut()) else {
        return;
    };
    rig.hand = Some(l.translation());
}

/// `--grapple-demo`: for screenshots, find a wall near Rico, look at it and fire the hook
/// at `fire_at` seconds (real time) so that the shot at the end of the run shows the hook
/// in flight or Rico hanging.
#[derive(Resource, Clone, Copy, Debug)]
pub struct GrappleDemo {
    pub hang: bool,
    pub fire_at: f32,
    pub aimed: bool,
    pub fired: bool,
}

#[allow(clippy::too_many_arguments)]
fn grapple_demo(
    time: Res<Time<Real>>,
    demo: Option<ResMut<GrappleDemo>>,
    spatial: SpatialQuery,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut orbit: ResMut<crate::character::OrbitCamera>,
    params: Option<ResMut<HeroParams>>,
    names: Query<&Name>,
    heroes: Query<(Entity, &Hero, &Transform, &GrappleRig), Without<crate::character::Settling>>,
    mut last_log: Local<f32>,
    mut healths: Query<&mut crate::combat::Health, With<Hero>>,
    mut turned: Local<bool>,
) {
    let Some(mut demo) = demo else { return };
    // the soldiers of the base must not end the run: he is not hurt
    for mut h in &mut healths {
        h.hp = h.max;
    }
    let Ok((entity, hero, tf, rig)) = heroes.single() else {
        return;
    };
    if demo.fired {
        keys.release(KeyCode::KeyF);
        let now = time.elapsed_secs();
        if now - *last_log > 0.4 {
            *last_log = now;
            if !*turned {
                // seen from the side: from behind, the wire runs away along the view
                *turned = true;
                orbit.yaw += 1.1;
                orbit.pitch = 0.0;
            }
            info!(
                "grapple demo: {:?} hook {:.1} m from the hand at {:?}",
                rig.phase,
                rig.hook.distance(rig.hand_or(tf.translation, hero.facing)),
                rig.hook
            );
        }
        return;
    }
    if !hero.grounded {
        return;
    }
    let now = time.elapsed_secs();
    if !demo.aimed && now > demo.fire_at - 1.5 {
        // the nearest wall in the half circle ahead, 12 to 70 m away, a little above him
        demo.aimed = true;
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let mut best: Option<(f32, f32, f32)> = None;
        let mut best_building: Option<(f32, f32, f32)> = None;
        for i in 0..72 {
            let yaw = i as f32 / 72.0 * std::f32::consts::TAU;
            for pitch in [0.1f32, 0.3] {
                let dir = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::NEG_Z;
                let Ok(d) = Dir3::new(dir) else { continue };
                if let Some(h) = spatial.cast_ray(tf.translation, d, 70.0, true, &filter)
                    && h.distance > 12.0
                    && h.normal.y.abs() < 0.4
                {
                    if best.is_none_or(|b| h.distance < b.0) {
                        best = Some((h.distance, yaw, pitch));
                    }
                    // a building's collision rather than the mountain
                    let building = names
                        .get(h.entity)
                        .is_ok_and(|n| n.as_str().starts_with("collision"));
                    if building && best_building.is_none_or(|b| h.distance < b.0) {
                        best_building = Some((h.distance, yaw, pitch));
                    }
                }
            }
        }
        match best_building.or(best) {
            Some((dist, yaw, pitch)) => {
                info!("grapple demo: a wall {dist:.0} m away at yaw {yaw:.2}");
                orbit.yaw = yaw;
                orbit.pitch = pitch;
            }
            None => info!("grapple demo: no wall in reach, firing ahead"),
        }
    }
    if demo.aimed && now >= demo.fire_at {
        demo.fired = true;
        if !demo.hang
            && let Some(mut p) = params
        {
            // slow enough for a frame to catch it (the smoke runs draw a few frames a second)
            p.grapple.hook_speed = 3.0;
        }
        keys.press(KeyCode::KeyF);
        info!("grapple demo: fired at {now:.1} s");
    }
}

pub struct GrapplePlugin;

impl Plugin for GrapplePlugin {
    fn build(&self, app: &mut App) {
        use crate::character::{grapple_fire, hero_physics, hero_visuals};
        app.init_resource::<LockOn>()
            .add_observer(mark_launcher)
            .add_observer(give_rig)
            .add_systems(
                Update,
                (
                    track_hand.before(hero_visuals),
                    update_lock_on.before(grapple_fire),
                    grapple_demo.before(grapple_fire),
                ),
            )
            .add_systems(
                PostUpdate,
                draw_cables.after(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                FixedUpdate,
                (
                    (sync_rig, track_anchor, hook_flight)
                        .chain()
                        .before(hero_physics),
                    pull_things.after(hero_physics),
                ),
            );
    }
}

#[cfg(test)]
mod tests;
