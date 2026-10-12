//! Vehicle collision feedback and contact effects (board task 5403e32b, FX-VEH-4; research
//! `research/formats/vehicle-damage-effects.md` sections 2.1 to 2.4).
//!
//! Native contact callback per class (vtable slot 12): land 0x00700400, boat 0x00682fc0, air and
//! sea 0x006ee8a0, base 0x0073efe0. After the damage call (`vehicle_damage::collision_damage`):
//!
//! * a land vehicle whose contact normal points up (normal . up > 0.9) plays the persistent chassis
//!   scrape effect (key `#e89622fb`, default 293) with input `impulse`, SetMatrix and Play each
//!   time (not restarted while running); it does **not** run the base feedback;
//! * every other contact runs 0x0073efe0: gate `0.01 * impulse >= 5` (impulse >= 500), then the
//!   contact impact effect (entry 0, key `#a132f7da`: 67, boats 254) and, when the player occupies
//!   the vehicle and `impulse > 2000`, the shake and haptics effect 425 / 449 / 450 by speed
//!   (< 15, < 25, else; speed = the vehicle velocity length);
//! * helicopter rotor strike (0x007e1400): two rays (sweep angle and sweep angle + pi, 8 m long,
//!   static geometry only) from the main rotor; the sweep advances 2 pi per second. The first ray
//!   that hits applies `200 * (1 - t)^2` damage of type 3, a push and the effect 195 at the hit
//!   point; the effect stops after 1.0 s without a hit (DAT 0x010726f8) or when the player leaves.
//!
//! Not native (labelled in place): avian's per-pair impulse stands for Havok's per-callback impulse;
//! a resting-contact filter and replay holds keep the per-step contacts from replaying effects
//! every 1/60 s (the native entry updater 0x007244c0 state machine is not traced, Gap 5); the
//! surface variant comes from the terrain material under the contact point. Slide (`#eb001f37`,
//! caller not traced) and roll (`#396c6abd`, empty effect 69 in retail) are not played.

use crate::combat::Health;
use crate::effects::system::{EffectController, STATE_PENDING, STATE_RUNNING};
use crate::effects::{EffectSystem, Interaction, PlayRequest};
use crate::vehicle::Occupied;
use crate::vehicle_damage::{DeathClass, VehicleDamage, VehicleFx};
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::vehicle::ContactFxIds;

/// `0.01 * |impulse| >= 5.0` (0x0073efe0): the base feedback gate, as an impulse.
pub const IMPULSE_GATE: f32 = 500.0;
/// `impulse > 2000.0` (DAT 0x01072644): the shake gate.
pub const SHAKE_IMPULSE: f32 = 2000.0;
/// Speed bands of the shake effect (DAT 0x01072638 / 0x01072640).
pub const SHAKE_SPEED_LOW: f32 = 15.0;
pub const SHAKE_SPEED_HIGH: f32 = 25.0;
/// The shake and haptics effects (all hold only camera shake and haptics).
pub const SHAKE_LOW: u32 = 425;
pub const SHAKE_MID: u32 = 449;
pub const SHAKE_HIGH: u32 = 450;
/// A land contact is ground-aligned above this normal . up (0x00700400).
pub const GROUND_ALIGNED: f32 = 0.9;
/// The contact entry updater plays only above this world y (0x007244c0; inferred: the water
/// surface, `effects-pipeline.md` uses 200 m for the water input).
pub const CONTACT_MIN_Y: f32 = 200.1;
/// Rotor strike constants: ray length (0x41000000), damage scale (DAT 0x010726f4), physics kick
/// factors (DAT 0x010726f0, 0x010726ec), the strike effect and its stop delay (DAT 0x010726f8).
pub const ROTOR_RAY_LENGTH: f32 = 8.0;
pub const ROTOR_DAMAGE: f32 = 200.0;
pub const ROTOR_SPIN_KICK: f32 = 10.0;
pub const ROTOR_PUSH: f32 = -0.2;
pub const ROTOR_EFFECT: u32 = 195;
pub const ROTOR_STOP_SECONDS: f32 = 1.0;
/// Rotor strikes need the main rotor at speed. Native: the physics object value +0xb8 >= 5.0
/// (0x009a6f60, not identified); the port uses the blade-blur spool threshold of `heli.rs`.
const ROTOR_SPOOL_MIN: f32 = 0.55;

/// NON-NATIVE: a contact whose velocity change (impulse / mass) is below this is a resting contact
/// (one step of gravity is 0.16 m/s at 60 Hz), not an impact. Havok's callback does not report
/// persistent resting contacts; avian's per-pair impulse does.
const REST_DELTA_V: f32 = 0.4;
/// NON-NATIVE replay holds (seconds): the native entry updater's hold is `1.5 x impulse` in
/// undecoded units (0x0058b9c0 / 0x00669140).
const IMPACT_HOLD: f32 = 0.3;
const SHAKE_HOLD: f32 = 0.15;

/// What a contact does, decided from the numbers alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContactPlan {
    /// Ground-aligned land contact: the chassis scrape effect.
    pub scrape: bool,
    /// The surface-aware contact impact effect (entry 0).
    pub impact: bool,
    /// The shake and haptics effect id.
    pub shake: Option<u32>,
}

/// The effect id of the shake band for a speed (m/s).
pub fn shake_effect(speed: f32) -> u32 {
    if speed < SHAKE_SPEED_LOW {
        SHAKE_LOW
    } else if speed < SHAKE_SPEED_HIGH {
        SHAKE_MID
    } else {
        SHAKE_HIGH
    }
}

/// The contact callback of the vehicle's class (see the module comment). `normal_up` is the contact
/// normal . world up with the normal pointing from the other object into the vehicle (inferred
/// convention: the native normal's sign is not traced).
pub fn plan(
    class: DeathClass,
    impulse: f32,
    mass: f32,
    normal_up: f32,
    speed: f32,
    occupied: bool,
) -> ContactPlan {
    let mut p = ContactPlan::default();
    if impulse / mass.max(1e-3) < REST_DELTA_V {
        return p;
    }
    if class == DeathClass::Land && normal_up > GROUND_ALIGNED {
        p.scrape = true;
        return p;
    }
    if impulse < IMPULSE_GATE {
        return p;
    }
    p.impact = true;
    if occupied && impulse > SHAKE_IMPULSE {
        p.shake = Some(shake_effect(speed));
    }
    p
}

/// The rotor strike damage at hit fraction `t` of the ray: `200 * (1 - t)^2`.
pub fn rotor_damage(t: f32) -> f32 {
    ROTOR_DAMAGE * (1.0 - t) * (1.0 - t)
}

/// Per-vehicle contact effect state: the ids of the `.epe`, the persistent controllers (stopped on
/// cleanup) and the replay holds.
#[derive(Component, Debug)]
pub struct VehicleContactFx {
    pub ids: ContactFxIds,
    scrape: Option<EffectController>,
    scrape_missing: bool,
    rotor: Option<EffectController>,
    rotor_missing: bool,
    /// Velocity before the physics step (Havok's callback sees the pre-solve velocity; inferred).
    pre_speed: f32,
    next_impact: f32,
    next_shake: f32,
    /// Rotor ray sweep angle (+0x32f) and seconds since the last hit (+0x330).
    sweep: f32,
    since_hit: f32,
}

impl VehicleContactFx {
    pub fn new(ids: ContactFxIds) -> Self {
        Self {
            ids,
            scrape: None,
            scrape_missing: false,
            rotor: None,
            rotor_missing: false,
            pre_speed: 0.0,
            next_impact: 0.0,
            next_shake: 0.0,
            sweep: 0.0,
            since_hit: 0.0,
        }
    }

    /// Stop everything that runs (the vehicle goes away).
    pub fn stop_all(&mut self, sys: &mut EffectSystem, immediate: bool) {
        for c in [&self.scrape, &self.rotor].into_iter().flatten() {
            if matches!(sys.state(c), Some(STATE_RUNNING | STATE_PENDING)) {
                sys.stop(c, immediate);
            }
        }
    }

    /// `(scrape, rotor)` controller states (tests).
    #[cfg(test)]
    pub fn report(&self, sys: &EffectSystem) -> (Option<u8>, Option<u8>) {
        (
            self.scrape.as_ref().and_then(|c| sys.state(c)),
            self.rotor.as_ref().and_then(|c| sys.state(c)),
        )
    }
}

pub fn plugin(app: &mut App) {
    app.add_observer(stop_on_remove)
        .add_systems(FixedPreUpdate, record_speed)
        .add_systems(
            FixedPostUpdate,
            contact_feedback.in_set(PhysicsSystems::Last),
        )
        .add_systems(Update, rotor_strike);
}

fn record_speed(mut vehicles: Query<(&mut VehicleContactFx, &LinearVelocity)>) {
    for (mut c, v) in &mut vehicles {
        c.pre_speed = v.0.length();
    }
}

fn stop_on_remove(
    remove: On<Remove, VehicleContactFx>,
    mut vehicles: Query<&mut VehicleContactFx>,
    sys: Option<ResMut<EffectSystem>>,
) {
    let (Some(mut sys), Ok(mut c)) = (sys, vehicles.get_mut(remove.entity)) else {
        return;
    };
    c.stop_all(&mut sys, true);
}

/// The contact of a pair on `body`'s side: the point of the strongest manifold point and the
/// normal pointing from the other object into the body.
fn pair_contact(pair: &ContactPair, body: Entity) -> Option<(Vec3, Vec3)> {
    let mut best: Option<(f32, Vec3, Vec3)> = None;
    for m in &pair.manifolds {
        let n = if pair.body1 == Some(body) { -m.normal } else { m.normal };
        for p in &m.points {
            if best.is_none_or(|b| p.normal_impulse > b.0) {
                best = Some((p.normal_impulse, p.point, n));
            }
        }
    }
    best.map(|b| (b.1, b.2))
}

/// The contact callback (physics step): scrape, impact and shake.
#[allow(clippy::type_complexity)]
fn contact_feedback(
    graph: Res<ContactGraph>,
    time: Res<Time>,
    sys: Option<ResMut<EffectSystem>>,
    mut ground: Option<ResMut<crate::audio::Ground>>,
    game: Option<Res<crate::Game>>,
    mut vehicles: Query<(
        &mut VehicleContactFx,
        &VehicleFx,
        &ComputedMass,
        Has<Occupied>,
    )>,
) {
    let Some(mut sys) = sys else { return };
    let now = time.elapsed_secs();
    for pair in graph.iter_active_touching() {
        let impulse = pair.total_normal_impulse_magnitude();
        if impulse <= 0.0 {
            continue;
        }
        let (a, b) = (pair.body1, pair.body2);
        if a.is_some() && a == b {
            continue;
        }
        for (body, other) in [(a, b), (b, a)] {
            let Some(body) = body else { continue };
            let Ok((mut c, fx, mass, occupied)) = vehicles.get_mut(body) else {
                continue;
            };
            let Some((point, normal)) = pair_contact(pair, body) else {
                continue;
            };
            let p = plan(fx.class(), impulse, mass.value(), normal.y, c.pre_speed, occupied);
            let matrix = Mat4::from_rotation_translation(
                Quat::from_rotation_arc(Vec3::Y, normal.normalize_or(Vec3::Y)),
                point,
            );
            if p.scrape {
                if c.scrape.is_none() && !c.scrape_missing {
                    c.scrape = sys.create(c.ids.scrape);
                    c.scrape_missing = c.scrape.is_none();
                }
                if let Some(ctl) = c.scrape.as_ref() {
                    sys.set_input(ctl, jc2_formats::hash::hash_str("impulse"), impulse);
                    sys.set_transform(ctl, matrix);
                    sys.play_controller(ctl, &[], false);
                }
            }
            // the entry updater plays only above the water surface (world y 200.1, inferred)
            if p.impact
                && point.y > CONTACT_MIN_Y
                && now >= c.next_impact
                && let Some(id) = c.ids.impact
            {
                c.next_impact = now + IMPACT_HOLD;
                let mut req = PlayRequest::new(id).at(matrix);
                // non-native: the surface variant is the terrain material under the point when the
                // other object is static (the native surface source is not traced)
                if other.is_none()
                    && let (Some(g), Some(game)) = (ground.as_mut(), game.as_ref())
                    && let Some(s) = g.surface_at(&game.fs, point)
                {
                    req = req.interaction(Interaction::surface(s.material));
                }
                sys.play(req);
            }
            if let Some(id) = p.shake
                && now >= c.next_shake
            {
                c.next_shake = now + SHAKE_HOLD;
                sys.play(PlayRequest::new(id).at(matrix));
            }
        }
    }
}

/// The helicopter rotor strike (0x007e1400), every frame.
#[allow(clippy::type_complexity)]
fn rotor_strike(
    time: Res<Time>,
    sys: Option<ResMut<EffectSystem>>,
    spatial: SpatialQuery,
    colliders: Query<&ColliderOf>,
    rigid: Query<&RigidBody>,
    rotors: Query<&crate::heli::Rotor>,
    mut helis: Query<(
        Entity,
        &crate::heli::Heli,
        &GlobalTransform,
        &Children,
        &mut VehicleContactFx,
        &mut VehicleDamage,
        &mut Health,
        &ComputedMass,
        Has<Occupied>,
        Forces,
    )>,
) {
    let dt = time.delta_secs();
    let Some(mut sys) = sys else { return };
    if dt <= 0.0 {
        return;
    }
    for (e, heli, gtf, children, mut c, vd, mut health, mass, occupied, mut forces) in
        &mut helis
    {
        let spinning = heli.spool >= ROTOR_SPOOL_MIN;
        if !occupied || !spinning {
            // native: the effect is stopped when the strike is off
            if let Some(ctl) = c.rotor.as_ref()
                && matches!(sys.state(ctl), Some(STATE_RUNNING | STATE_PENDING))
            {
                sys.stop(ctl, false);
            }
            continue;
        }
        // the main rotor hub (inferred: the first rotor part that is not the tail rotor;
        // native: the CRotorInfo list at +0x2f1)
        let Some(hub) = children
            .iter()
            .filter_map(|ch| rotors.get(ch).ok())
            .find(|r| !r.tail)
            .map(|r| r.base.translation)
        else {
            continue;
        };
        let origin = gtf.transform_point(hub);
        let mut hit = None;
        for k in 0..2 {
            let angle = c.sweep + k as f32 * std::f32::consts::PI;
            let local = Quat::from_rotation_y(angle) * Vec3::Z;
            let dir = gtf.rotation() * local;
            let Ok(dir) = Dir3::new(dir) else { continue };
            let filter = SpatialQueryFilter::from_excluded_entities([e]);
            // static geometry only (CStaticOnlyRayHitCollector): a collider with no body, or
            // whose body is static
            let is_static = |ent: Entity| {
                colliders
                    .get(ent)
                    .ok()
                    .and_then(|of| rigid.get(of.body).ok())
                    .is_none_or(|rb| rb.is_static())
            };
            if let Some(h) = spatial.cast_ray_predicate(
                origin,
                dir,
                ROTOR_RAY_LENGTH,
                true,
                &filter,
                &is_static,
            ) {
                hit = Some((h.distance / ROTOR_RAY_LENGTH, origin + dir.as_vec3() * h.distance, dir.as_vec3()));
                break;
            }
        }
        if let Some((t, point, dir)) = hit {
            if health.hp > 0.0 && !vd.invulnerable {
                let lost = vd.type_damage(crate::vehicle_damage::DamageType::Collision, rotor_damage(t));
                health.hp = (health.hp - lost).max(0.0);
            }
            let m = mass.value();
            // kicks (0x009b6930 / 0x009c00e0, not traced: read as an angular impulse about the
            // rotor axis and a linear impulse at the hit point; inferred)
            forces.apply_angular_impulse(gtf.up().as_vec3() * (m * ROTOR_SPIN_KICK));
            forces.apply_linear_impulse_at_point(dir * (m * ROTOR_PUSH), point);
            if c.rotor.is_none() && !c.rotor_missing {
                c.rotor = sys.create(ROTOR_EFFECT);
                c.rotor_missing = c.rotor.is_none();
            }
            if let Some(ctl) = c.rotor.as_ref() {
                sys.set_transform(ctl, Mat4::from_translation(point));
                sys.play_controller(ctl, &[], false);
            }
            c.since_hit = 0.0;
        } else {
            c.since_hit += dt;
            if c.since_hit >= ROTOR_STOP_SECONDS
                && let Some(ctl) = c.rotor.as_ref()
                && matches!(sys.state(ctl), Some(STATE_RUNNING | STATE_PENDING))
            {
                sys.stop(ctl, false);
            }
            c.sweep += std::f32::consts::TAU * dt;
            if c.sweep > std::f32::consts::TAU {
                c.sweep = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shake_bands() {
        assert_eq!(shake_effect(10.0), 425);
        assert_eq!(shake_effect(14.99), 425);
        assert_eq!(shake_effect(15.0), 449);
        assert_eq!(shake_effect(24.9), 449);
        assert_eq!(shake_effect(25.0), 450);
        assert_eq!(shake_effect(40.0), 450);
    }

    #[test]
    fn base_feedback_gate_and_shake_gate() {
        let m = 1500.0;
        // a wall hit at 10 / 20 / 30 m/s of a 1500 kg car stops it: impulse = m * v
        let wall = |v: f32, occupied| plan(DeathClass::Land, m * v, m, 0.0, v, occupied);
        assert_eq!(wall(10.0, true).shake, Some(425));
        assert_eq!(wall(20.0, true).shake, Some(449));
        assert_eq!(wall(30.0, true).shake, Some(450));
        assert!(wall(10.0, true).impact);
        // empty vehicles show the impact but do not shake
        assert!(wall(20.0, false).impact && wall(20.0, false).shake.is_none());
        // 0.01 * impulse >= 5 gate: 600 passes (velocity change 0.4), 450 does not
        assert!(plan(DeathClass::Air, 600.0, 1000.0, 0.0, 5.0, false).impact);
        assert!(!plan(DeathClass::Air, 450.0, 300.0, 0.0, 5.0, false).impact);
        // shake needs impulse > 2000
        assert!(plan(DeathClass::Air, 2000.0, 1000.0, 0.0, 5.0, true).shake.is_none());
        assert_eq!(plan(DeathClass::Air, 2001.0, 1000.0, 0.0, 5.0, true).shake, Some(425));
    }

    #[test]
    fn land_ground_contacts_scrape_instead() {
        let p = plan(DeathClass::Land, 3000.0, 1500.0, 0.95, 20.0, true);
        assert_eq!(p, ContactPlan { scrape: true, impact: false, shake: None });
        // exactly 0.9 is not ground-aligned; other classes never scrape
        assert!(plan(DeathClass::Land, 3000.0, 1500.0, 0.9, 20.0, true).impact);
        assert!(!plan(DeathClass::Sea, 3000.0, 1500.0, 0.95, 20.0, true).scrape);
    }

    #[test]
    fn resting_contacts_are_ignored() {
        // a 40 t tank at rest: one step of weight is 40000 * 9.81 / 60 = 6540 N s
        assert_eq!(plan(DeathClass::Land, 6540.0, 40_000.0, 1.0, 0.0, true), ContactPlan::default());
    }

    #[test]
    fn rotor_damage_falls_off_squared() {
        assert_eq!(rotor_damage(0.0), 200.0);
        assert_eq!(rotor_damage(0.5), 50.0);
        assert_eq!(rotor_damage(1.0), 0.0);
    }

    /// The retail effects exist and the scrape effect takes the `impulse` input; the contact
    /// controllers start and are stopped again. Needs JC2_TEST_DIR.
    #[test]
    fn retail_effects_fixture() {
        use crate::effects::{EffectHost, default_adapters, particle::ParticleManager};
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = std::sync::Arc::new(jc2_formats::GameFs::open(dir).unwrap());
        let host = EffectHost::default();
        *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
        let mut sys = EffectSystem::from_retail(fs.clone(), default_adapters(&host)).unwrap();
        for id in [425, 449, 450, 67, 254, 293, 195] {
            assert!(sys.description(id).is_some(), "effect {id} missing");
        }
        let scrape = sys.description(293).unwrap();
        eprintln!("effect 293 inputs {:?}", scrape.inputs);
        assert!(
            scrape.inputs.contains_key(&jc2_formats::hash::hash_str("impulse")),
            "293 has no impulse input: {:?}",
            scrape.inputs
        );
        let mut c = VehicleContactFx::new(ContactFxIds::default());
        c.scrape = sys.create(293);
        c.rotor = sys.create(ROTOR_EFFECT);
        for ctl in [c.scrape.as_ref(), c.rotor.as_ref()].into_iter().flatten() {
            sys.set_transform(ctl, Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0)));
            sys.play_controller(ctl, &[], false);
        }
        for _ in 0..10 {
            sys.tick(1.0 / 30.0);
        }
        assert!(c.report(&sys).0.is_some());
        c.stop_all(&mut sys, true);
        for _ in 0..30 {
            sys.tick(1.0 / 30.0);
        }
        eprintln!("after stop: {:?}", c.report(&sys));
    }
}
