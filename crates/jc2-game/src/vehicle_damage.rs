//! The vehicle damage model (FX-VEH-3): the PD variant damage block (max health, instant-wreck
//! threshold, per-type thresholds, collision multiplier), collision damage from the physics
//! contacts, the instant wreck and the 5 % pin on shots, plus the burn timer that finishes a
//! vehicle the pin keeps alive. Research: `research/formats/vehicle-damage-effects.md`
//! sections 1, 2.1 and 3, `vehicle-fire-explosion.md` section 4.2.
//!
//! Native (read from the executable, addresses in the research docs): the damage block keys and
//! defaults (0x008cb720), the type table (0x00681480 / 0x00843800, slot 69 0x0056dcd0), the collision
//! formula and the instant-wreck rule (0x00805700), the shot pin (0x00626440), the burn timer
//! (0x00572770). Everything labelled "inferred" or "non-native" is not proven.
//!
//! FX-VEH-1 adds the damage state machine of 0x00572770 ([`VehicleDamage::step`]: smoking, burning
//! with the burn timer, the post-explosion timer, water hysteresis, the flip burn with its
//! unknown limit) and the slot 8 / 12 / 13 / 14 / 15 effect controllers ([`VehicleFx`]) that follow
//! those flags. The controllers are unbounded leaves that never end by themselves (484554ae), so
//! [`VehicleFx`] keeps the handles and `Stop`s them when a flag clears (repair, burn end, water),
//! when the vehicle explodes (slot 13 and 8) and when the vehicle goes away.

use crate::combat::Health;
use crate::effects::EffectSystem;
use crate::effects::system::{EffectController, STATE_PENDING, STATE_RUNNING};
use crate::explosion::Wreck;
use crate::vehicle::Occupied;
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::vehicle::{DamageModule, EffectProp, effect_slot};

/// `DAT 0x01093c04`: the factor of the collision damage `3 * impulse / mass` (0x00805700).
pub const COLLISION_FACTOR: f32 = 3.0;
/// Instant-wreck threshold factor while the player occupies the vehicle and +0x36a is clear.
pub const OCCUPANT_FACTOR: f32 = 2.0;
/// The same factor when +0x36a is set.
pub const FLAG_36A_FACTOR: f32 = 0.05;
/// Burn seconds (+0x340) written by the instant wreck.
pub const INSTANT_WRECK_BURN_SECONDS: f32 = 0.1;
/// A shot that leaves the vehicle below this health ratio pins it (0x00626440).
pub const PIN_BELOW: f32 = 0.1;
/// The pinned health as a fraction of the maximum.
pub const PIN_TO: f32 = 0.05;
/// Constructor defaults of 0x005cfe40 for the `.epe` burn keys (`#189e862f`, `#4476a911`).
pub const DEFAULT_BURN_THRESHOLD: f32 = 0.1;
pub const DEFAULT_BURN_SECONDS: f32 = 8.0;
/// Constructor default of `#01429682` (0x005cfe40).
pub const DEFAULT_SMOKE_THRESHOLD: f32 = 0.4;
/// Non-native: `#9011eda3` has no constructor default; all 90 retail vehicles carry 8.0.
pub const FALLBACK_POST_EXPLOSION_SECONDS: f32 = 8.0;
/// Water: above this submersion ratio smoke and fire are forced off (0x00572770 step 5).
pub const SUBMERGE_ABOVE: f32 = 0.8;
/// Water: the submerged flag clears when the ratio is at or below 0.8 **and** below this.
pub const SUBMERGE_CLEAR_BELOW: f32 = 0.6;
/// The flip burn (0x0079e310) needs the vehicle nearly stopped: speed squared below this.
pub const FLIP_SPEED_SQ: f32 = 20.0;
/// ... and upside down: the matrix element at +0x2fc below this (inferred: the up axis' y).
pub const FLIP_UP_BELOW: f32 = -0.5;
/// The flip burn accumulator limit (+0x488). Native data source not traced (vehicle-fire-explosion
/// Gap 6): `None` = unknown, so a flipped vehicle never starts to burn by itself here.
pub const FLIP_BURN_LIMIT: Option<f32> = None;
/// The smoke burst (slot 14) fires when `rand() % 500 > 495` (0x00516640): 4 in 500 calls.
pub const BURST_MODULUS: u32 = 500;
pub const BURST_ABOVE: u32 = 495;
/// `max_health` in the vehicle `.epe` (all 90 retail vehicles; the `CDamageable` loader 0x00741890).
pub const EPE_MAX_HEALTH: f32 = 100.0;

/// Which value the vehicle's effective maximum health takes (research gap ada341f5 / Gap 2:
/// whether `CVehicle` slot 69 (0x0056dcd0, the damage module value) runs after the `CDamageable`
/// loader (the `.epe` value 100) is not decided). A labelled switch, not a native fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaxHealthSource {
    /// The doll `CVehicleDamageModule` value (500 to 5000). The default: the module is the only source
    /// of a per-vehicle value and the type thresholds are tuned for it.
    DollModule,
    /// The `.epe` `max_health` = 100 on every vehicle.
    EpeHundred,
    /// The earlier placeholder: `vehicletypes.bin` `health` (assumed, not the damage module).
    VehicleTypes,
}

impl MaxHealthSource {
    /// `JC2_VEHICLE_MAX_HEALTH=module|epe|types` selects it for a run; the default is
    /// [`MaxHealthSource::DollModule`].
    pub fn current() -> Self {
        static CHOICE: std::sync::OnceLock<MaxHealthSource> = std::sync::OnceLock::new();
        *CHOICE.get_or_init(|| match std::env::var("JC2_VEHICLE_MAX_HEALTH").as_deref() {
            Ok("epe") => MaxHealthSource::EpeHundred,
            Ok("types") => MaxHealthSource::VehicleTypes,
            _ => MaxHealthSource::DollModule,
        })
    }
}

/// The damage types of the per-type table (0x00843800). Types 4 and 5 have callers but no table
/// entry (research Gap 2), so they are not modelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageType {
    /// Threshold from the module key `#2b8a8075` (no caller known yet; the table is complete).
    #[allow(dead_code)]
    Type1,
    /// A direct hit (slot 9, 0x00626440): threshold 0.
    Shot,
    /// Collision (0x00805700) and the helicopter rotor strike: threshold from `#05de7e98`.
    Collision,
}

/// `#4476a911` / `#189e862f` as the burn timer reads them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BurnTuning {
    /// `#01429682` (+0x330), constructor default 0.4.
    pub smoke_threshold: f32,
    pub threshold: f32,
    pub seconds: f32,
    /// `#9011eda3` (+0x348): no constructor default. `None` (key missing) uses
    /// [`FALLBACK_POST_EXPLOSION_SECONDS`].
    pub post_explosion_seconds: Option<f32>,
}

impl Default for BurnTuning {
    fn default() -> Self {
        Self {
            smoke_threshold: DEFAULT_SMOKE_THRESHOLD,
            threshold: DEFAULT_BURN_THRESHOLD,
            seconds: DEFAULT_BURN_SECONDS,
            post_explosion_seconds: None,
        }
    }
}

impl From<jc2_formats::vehicle::BurnTuning> for BurnTuning {
    fn from(b: jc2_formats::vehicle::BurnTuning) -> Self {
        Self {
            smoke_threshold: b.smoke_threshold,
            threshold: b.threshold,
            seconds: b.seconds,
            post_explosion_seconds: b.post_explosion_seconds,
        }
    }
}

/// What a collision does to the vehicle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CollisionOutcome {
    /// Nothing (below the type threshold).
    None,
    /// This much health is lost.
    Damage(f32),
    /// Health goes to 0 at once, burn seconds 0.1.
    InstantWreck,
}

/// The damage state of one vehicle (vehicle +0x338, +0x33c, +0x340, +0x369, +0x36a, +0x388).
#[derive(Component, Clone, Debug)]
pub struct VehicleDamage {
    pub module: DamageModule,
    pub burn: BurnTuning,
    /// +0x33c: the burn timer (never decreases, 4.2).
    pub burn_timer: f32,
    /// +0x340: burn seconds (the `.epe` value; the instant wreck overwrites it with 0.1).
    pub burn_seconds: f32,
    /// +0x35b: burning. The fire effects follow this flag ([`VehicleFx`]).
    pub burning: bool,
    /// +0x35a: smoking.
    pub smoking: bool,
    /// +0x361: submerged (water hysteresis 0.8 / 0.6); the effect owners force smoke and fire off.
    pub submerged: bool,
    /// +0x344: the post-explosion timer.
    pub post_timer: f32,
    /// +0x484: the flip burn accumulator (limit unknown, [`FLIP_BURN_LIMIT`]).
    pub flip_timer: f32,
    /// +0x369: set by the air shot-down starter 0x0073d740 (not wired).
    pub shot_down: bool,
    /// +0x36a: set by the seat scan 0x0067b030 (condition not decoded, always false here).
    pub seat_flag_36a: bool,
    /// +0x41: invulnerable.
    pub invulnerable: bool,
}

impl VehicleDamage {
    pub fn new(module: DamageModule, burn: BurnTuning) -> Self {
        Self {
            module,
            burn,
            burn_timer: 0.0,
            burn_seconds: burn.seconds,
            burning: false,
            smoking: false,
            submerged: false,
            post_timer: 0.0,
            flip_timer: 0.0,
            shot_down: false,
            seat_flag_36a: false,
            invulnerable: false,
        }
    }

    /// The per-type table (slot 69 installs types 1, 2 and 3 as mode 1, multiplier 1.0): the amount
    /// that is left after the type's threshold, `max(0, amount - threshold)`.
    pub fn type_damage(&self, ty: DamageType, amount: f32) -> f32 {
        let threshold = match ty {
            DamageType::Type1 => self.module.type1_threshold,
            DamageType::Shot => 0.0,
            DamageType::Collision => self.module.type3_threshold,
        };
        (amount - threshold).max(0.0)
    }

    /// `3 * |impulse| / mass * multiplier` (0x00805700; mass = the physics object's, inferred).
    pub fn collision_local(&self, impulse: f32, mass: f32) -> f32 {
        COLLISION_FACTOR / mass.max(1e-3) * impulse.abs() * self.module.collision_multiplier
    }

    /// The effect of a contact with `impulse` on a vehicle of `mass` (0x00805700). `occupied` is the
    /// player-occupant byte (+0xba6). The caller skips dead vehicles.
    pub fn collision(&self, impulse: f32, mass: f32, occupied: bool) -> CollisionOutcome {
        let local = self.collision_local(impulse, mass);
        let factor = if self.seat_flag_36a {
            FLAG_36A_FACTOR
        } else if occupied {
            OCCUPANT_FACTOR
        } else {
            1.0
        };
        if (local - self.module.instant_wreck * factor > 0.0 || self.shot_down) && !self.invulnerable
        {
            return CollisionOutcome::InstantWreck;
        }
        let lost = self.type_damage(DamageType::Collision, local);
        if lost > 0.0 {
            CollisionOutcome::Damage(lost)
        } else {
            CollisionOutcome::None
        }
    }

    /// Apply a collision outcome to the health (`SetHealth`).
    pub fn apply_collision(&mut self, h: &mut Health, outcome: CollisionOutcome) {
        match outcome {
            CollisionOutcome::None => {}
            CollisionOutcome::Damage(d) => h.hp = (h.hp - d).max(0.0),
            CollisionOutcome::InstantWreck => {
                self.burn_seconds = INSTANT_WRECK_BURN_SECONDS;
                h.hp = 0.0;
            }
        }
    }

    /// A shot (slot 9, 0x00626440): the amount goes through the type 2 table, then, when the health
    /// ratio is below 10 %, the vehicle has not exploded and the burn timer has not passed the burn
    /// seconds, health is pinned at 5 % of the maximum. Returns the health that was lost (0 when the
    /// vehicle is dead, exploded or invulnerable: `ApplyDamage` returns 0).
    ///
    /// `source_ok` stands for the pin's second condition: a player occupant (+0xba6) **or** the check
    /// on the damage source 0x005b9880 (not decoded). The callers pass `true` for the player's own
    /// shots (inferred: the source check is most likely "the shooter is the player"; unproven).
    pub fn shot(&mut self, h: &mut Health, damage: f32, exploded: bool, source_ok: bool) -> f32 {
        if h.hp <= 0.0 || exploded || self.invulnerable {
            return 0.0;
        }
        let lost = self.type_damage(DamageType::Shot, damage);
        let before = h.hp;
        h.hp -= lost;
        if lost > 0.0
            && h.max > 0.0
            && h.hp / h.max < PIN_BELOW
            && self.burn_timer <= self.burn_seconds
            && source_ok
        {
            h.hp = h.max * PIN_TO;
        }
        before - h.hp
    }

    /// The per-frame damage state of `CVehicle` slot 27's helper 0x00572770 (`vehicle-fire-explosion.md`
    /// section 4.2, steps 1, 2, 4 and 5; step 3, the death, is `explosion::wreck_vehicles`).
    ///
    /// Reconciliation with the native routine (differences are the port's, not native):
    /// * the burn timer advances while `hp / max < burn threshold` whether or not the vehicle is
    ///   burning, never decreases and kills at `> burn seconds`, as native; unlike the first FX-VEH-3
    ///   version it also runs on the frame the health reaches 0 and on wrecks (native: the timer is
    ///   clamped and `SetHealth(0)` is a repeated no-op), which is what sets `burning` on a
    ///   vehicle that went straight to 0 (a collision wreck): the slot 15 fire needs it;
    /// * `burning` is set only while the vehicle has not exploded (+0x35c), as native; `smoking` is
    ///   cleared by the burn step and is only set while not burning and not dead;
    /// * the post-explosion timer (+0x344) runs only after the explosion (inferred from the
    ///   constructor value 0 and the flag clear; native gating not traced) and clears smoking and
    ///   burning at the post-explosion fire seconds;
    /// * repair (health back above a threshold) is **non-native** (the executable has no repair path):
    ///   `burning` and the timer reset above the burn threshold, `smoking` above the smoke threshold.
    pub fn step(&mut self, h: &mut Health, i: &StepInput) {
        if h.max <= 0.0 {
            return;
        }
        let dead = h.hp <= 0.0;
        let ratio = h.hp / h.max;
        // non-native repair
        if !i.exploded && !dead {
            if ratio >= self.burn.threshold && (self.burning || self.burn_timer > 0.0) {
                self.burning = false;
                self.burn_timer = 0.0;
            }
            if ratio >= self.burn.smoke_threshold {
                self.smoking = false;
            }
        }
        // 1. smoking, and the flip burn
        if !self.burning && !dead {
            if !self.smoking && ratio < self.burn.smoke_threshold {
                self.smoking = true;
            }
            if i.upside_down && i.speed_sq < FLIP_SPEED_SQ && !self.submerged {
                self.flip_timer += i.dt;
                if let Some(limit) = FLIP_BURN_LIMIT
                    && self.flip_timer > limit
                {
                    h.hp = (self.burn.threshold * h.max - 1.0).max(0.0);
                }
            } else {
                self.flip_timer = 0.0;
            }
        }
        // 2. burning
        if h.hp / h.max < self.burn.threshold {
            self.smoking = false;
            if !self.burning && !i.exploded {
                self.burning = true;
            }
            self.burn_timer += i.dt;
            if self.burn_timer > self.burn_seconds {
                self.burn_timer = self.burn_seconds;
                h.hp = 0.0;
            }
        }
        // 4. after the explosion
        if i.exploded {
            self.post_timer += i.dt;
            let seconds = self
                .burn
                .post_explosion_seconds
                .unwrap_or(FALLBACK_POST_EXPLOSION_SECONDS);
            if self.post_timer >= seconds {
                self.smoking = false;
                self.burning = false;
            }
        }
        // 5. water
        if i.water > SUBMERGE_ABOVE {
            if !self.submerged {
                self.smoking = false;
                self.burning = false;
                self.submerged = true;
            }
        } else if self.submerged && i.water < SUBMERGE_CLEAR_BELOW {
            self.submerged = false;
        }
    }
}

/// What [`VehicleDamage::step`] needs besides the health.
#[derive(Clone, Copy, Debug, Default)]
pub struct StepInput {
    pub dt: f32,
    /// +0x35c: the death routine ran (the [`Wreck`] marker).
    pub exploded: bool,
    pub speed_sq: f32,
    pub upside_down: bool,
    /// The submerged fraction (physics object, vtable +0xa8).
    pub water: f32,
}

/// The vehicle's maximum health under the chosen [`MaxHealthSource`]. `types` is the placeholder
/// `vehicletypes.bin` value.
pub fn max_health(source: MaxHealthSource, module: &DamageModule, types: Option<f32>) -> f32 {
    match source {
        MaxHealthSource::DollModule => module.max_health,
        MaxHealthSource::EpeHundred => EPE_MAX_HEALTH,
        MaxHealthSource::VehicleTypes => types.unwrap_or(module.max_health),
    }
}

/// The class prologue of the death routine (`vehicle-fire-explosion.md` section 5 item 1) posts one of
/// three events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeathClass {
    /// `CLandVehicle`, `CCar`, `CMotorBike` (0x00647bd0).
    #[default]
    Land,
    /// `CAirVehicle`, `CHelicopter`, `CAirPlane` (0x0079ed50).
    Air,
    /// `CSeaVehicle`, `CBoat` (0x007ff860).
    Sea,
}

impl DeathClass {
    /// The game event the class prologue posts. Native: the land condition 0x007990c0 is not traced
    /// (treated as always true here, labelled).
    pub fn event_name(self) -> &'static str {
        match self {
            DeathClass::Land => EVENT_LAND_DESTROYED,
            DeathClass::Air => EVENT_AIR_DESTROYED,
            DeathClass::Sea => EVENT_SEA_DESTROYED,
        }
    }
}

/// Posted by `SetHealth` (0x007e87a0 via vtable slot 24, 0x006b61e0) when health first reaches 0.
pub const EVENT_OBJECT_DESTROYED: &str = "gsy.object_destroyed";
pub const EVENT_LAND_DESTROYED: &str = "government.landvehicle.destroyed";
pub const EVENT_AIR_DESTROYED: &str = "government.airvehicle.destroyed";
pub const EVENT_SEA_DESTROYED: &str = "government.seavehicle.destroyed";

pub struct VehicleDamagePlugin;

impl Plugin for VehicleDamagePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            damage_state.before(crate::vehicle_death::vehicle_death),
        )
        // `JC2_VEHICLE_FX_OFF=1` leaves the controllers out (the "before" captures: master drew
        // nothing for a smoking or burning vehicle until it exploded)
        .add_systems(
            Update,
            drive_effects
                .after(damage_state)
                .run_if(|| std::env::var_os("JC2_VEHICLE_FX_OFF").is_none()),
        )
        .add_observer(stop_on_remove)
        .add_systems(
            FixedPostUpdate,
            collision_damage.in_set(PhysicsSystems::Last),
        );
        // collision feedback and contact effects (FX-VEH-4)
        crate::vehicle_contact_fx::plugin(app);
        // Capture fixture: `JC2_VEHICLE_FIRE_FIXTURE=<health fraction>` sets every vehicle that
        // spawns at full health to that fraction once (0.3 smokes, 0.15 smokes under the 0.4
        // threshold, 0.05 burns and dies after the burn seconds). Debug aid, not a game feature.
        if let Some(fraction) = std::env::var("JC2_VEHICLE_FIRE_FIXTURE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            app.add_systems(
                Update,
                // every vehicle (run with `--no-traffic --no-npcs` so only the `--drive` car exists)
                // (`--no-hero --drive` has no combat plugin and so no health: the occupied car gets one)
                move |mut commands: Commands,
                      mut done: Local<std::collections::HashSet<Entity>>,
                      mut vehicles: Query<
                    (Entity, &VehicleDamage, Option<&mut Health>, Has<Occupied>),
                >| {
                    for (e, vd, health, occupied) in &mut vehicles {
                        match health {
                            Some(mut h) if done.insert(e) => h.hp = h.max * fraction,
                            None if occupied && done.insert(e) => {
                                let max = vd.module.max_health;
                                commands.entity(e).insert(Health {
                                    hp: max * fraction,
                                    max,
                                });
                            }
                            _ => {}
                        }
                    }
                },
            );
            // `JC2_VEHICLE_FIRE_VIEW=right,up,forward` (metres in the occupied car's frame): put the
            // camera there looking at the car, for captures (the chase camera sees the tail only)
            if let Some(offset) = std::env::var("JC2_VEHICLE_FIRE_VIEW").ok().and_then(|v| {
                let n: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
                (n.len() == 3).then(|| Vec3::new(n[0], n[1], n[2]))
            }) {
                app.add_systems(
                    PostUpdate,
                    (move |cars: Query<&Transform, (With<Occupied>, Without<crate::world::FlyCamera>)>,
                           mut cams: Query<&mut Transform, With<crate::world::FlyCamera>>| {
                        let (Ok(car), Ok(mut cam)) = (cars.single(), cams.single_mut()) else {
                            return;
                        };
                        let at = car.translation
                            + car.right() * offset.x
                            + car.up() * offset.y
                            + car.forward() * offset.z;
                        cam.translation = at;
                        cam.look_at(car.translation + Vec3::Y * 1.2, Vec3::Y);
                    })
                    .before(TransformSystems::Propagate),
                );
            }
            // and print the controllers of the vehicles every 2 s
            app.add_systems(
                Update,
                |time: Res<Time>,
                 mut last: Local<f32>,
                 sys: Option<Res<EffectSystem>>,
                 host: Res<crate::effects::EffectHost>,
                 vehicles: Query<(Entity, &VehicleFx, &VehicleDamage)>| {
                    let Some(sys) = sys else { return };
                    if time.elapsed_secs() - *last < 2.0 {
                        return;
                    }
                    *last = time.elapsed_secs();
                    for (e, fx, vd) in &vehicles {
                        if vd.smoking || vd.burning {
                            let live = host.particles.lock().map_or(0, |p| p.total_particles());
                            info!(
                                "vehicle fixture {e}: smoking {} burning {} burn {:.1}s controllers {:?} live particles {live}",
                                vd.smoking,
                                vd.burning,
                                vd.burn_timer,
                                fx.report(&sys)
                            );
                        }
                    }
                },
            );
        }
    }
}

/// The submerged fraction of the vehicle's collision box (inferred: the native ratio comes from the
/// physics object, vtable +0xa8, whose definition is not traced).
pub fn water_ratio(aabb: Option<&ColliderAabb>) -> f32 {
    let Some(a) = aabb else { return 0.0 };
    let height = (a.max.y - a.min.y).max(1e-3);
    ((crate::world::SEA_LEVEL - a.min.y) / height).clamp(0.0, 1.0)
}

/// The damage state of every vehicle each frame (0x00572770). Runs before the death routine so the
/// frame that takes the health to 0 still sets `burning` (the slot 15 fire needs it).
#[allow(clippy::type_complexity)]
fn damage_state(
    time: Res<Time>,
    mut vehicles: Query<(
        Entity,
        &mut VehicleDamage,
        &mut Health,
        &GlobalTransform,
        Option<&LinearVelocity>,
        Option<&ColliderAabb>,
        Has<Wreck>,
    )>,
) {
    for (e, mut vd, mut h, gtf, vel, aabb, exploded) in &mut vehicles {
        let input = StepInput {
            dt: time.delta_secs(),
            exploded,
            speed_sq: vel.map_or(0.0, |v| v.0.length_squared()),
            upside_down: gtf.up().y < FLIP_UP_BELOW,
            water: water_ratio(aabb),
        };
        let (was_smoking, was_burning) = (vd.smoking, vd.burning);
        vd.step(&mut h, &input);
        if vd.smoking != was_smoking || vd.burning != was_burning {
            info!(
                "vehicle {e}: smoking {} burning {} (health {:.0} / {:.0}, burn timer {:.1} s)",
                vd.smoking, vd.burning, h.hp, h.max, vd.burn_timer
            );
        }
    }
}

/// The effect mount points of the doll and their controllers (`vehicle-fire-explosion.md` section 2).
/// Controllers are created on first use and kept; every one that starts is stopped again when its
/// flag clears and when this component goes away.
#[derive(Component, Debug)]
pub struct VehicleFx {
    props: Vec<FxProp>,
    /// Which `government.*.destroyed` event the death routine posts (the class prologue).
    class: DeathClass,
    /// An air vehicle: it has a slot 8 (shot-down) mount (the air builder 0x0079d220 reads it).
    air: bool,
    prev_burning: bool,
    rng: u32,
}

#[derive(Debug)]
struct FxProp {
    slot: i32,
    id: u32,
    local: Mat4,
    ctl: Option<EffectController>,
    /// `create` found no effect for the id: do not ask again.
    missing: bool,
}

/// The vehicle state the owners 0x00516640 / 0x0063fda0 read.
#[derive(Clone, Copy, Debug)]
pub struct FxState {
    pub smoking: bool,
    pub burning: bool,
    pub exploded: bool,
    pub submerged: bool,
    /// +0x35e, "engine started" (the port: someone is driving it, inferred).
    pub engine_on: bool,
    pub matrix: Mat4,
    pub velocity: Vec3,
}

impl VehicleFx {
    /// The slots the per-frame owners drive (the engine and water effect owners were not traced).
    pub const SLOTS: [i32; 5] = [
        effect_slot::SHOTDOWN,
        effect_slot::ENGINE_SMOKING,
        effect_slot::ENGINE_BURNING,
        effect_slot::ENGINE_SMOKE_BURST,
        effect_slot::POST_EXPLOSION_FIRE,
    ];

    /// The death routine's slots (FX-VEH-2): 1 explosion, 5 water explosion. Played once by
    /// [`VehicleFx::explode`]; kept here so the handles are stopped on cleanup.
    pub const DEATH_SLOTS: [i32; 2] = [effect_slot::EXPLOSION, effect_slot::WATER_EXPLOSION];

    pub fn new(effects: &[EffectProp]) -> Self {
        let props: Vec<FxProp> = effects
            .iter()
            .filter(|e| Self::SLOTS.contains(&e.slot) || Self::DEATH_SLOTS.contains(&e.slot))
            .map(|e| FxProp {
                slot: e.slot,
                id: e.id,
                local: crate::coords::mat_to_bevy(&e.local).to_matrix(),
                ctl: None,
                missing: false,
            })
            .collect();
        Self {
            air: props.iter().any(|p| p.slot == effect_slot::SHOTDOWN),
            class: DeathClass::Land,
            props,
            prev_burning: false,
            rng: 0,
        }
    }

    /// Set the vehicle class (the class prologue of the death routine picks the event name).
    pub fn with_class(mut self, class: DeathClass) -> Self {
        self.class = class;
        self
    }

    pub fn class(&self) -> DeathClass {
        self.class
    }

    /// The slot the death routine plays (0x00559150 step 3): 5 when submerged (+0x361), else 1.
    pub fn death_slot(submerged: bool) -> i32 {
        if submerged {
            effect_slot::WATER_EXPLOSION
        } else {
            effect_slot::EXPLOSION
        }
    }

    /// The death routine's effect (0x00559150 step 3): every instance of the slot 1 effects (slot 5
    /// when `submerged`) is placed with the vehicle matrix and played once. A vehicle without the
    /// slot plays nothing (cars and boats have no slot 5). Returns how many were started. The
    /// controllers stay in the mount list so [`VehicleFx::stop_all`] reaches them.
    pub fn explode(&mut self, sys: &mut EffectSystem, matrix: Mat4, submerged: bool) -> usize {
        let slot = Self::death_slot(submerged);
        let mut started = 0;
        for p in self.props.iter_mut().filter(|p| p.slot == slot) {
            if p.ctl.is_none() && !p.missing {
                p.ctl = sys.create(p.id);
                p.missing = p.ctl.is_none();
            }
            let Some(c) = p.ctl.as_ref() else { continue };
            sys.set_transform(c, matrix * p.local);
            sys.play_controller(c, &[], false);
            started += 1;
        }
        started
    }

    /// One draw of the vehicle's xorshift stream (the wheel start index of the death routine).
    pub fn next_random(&mut self) -> u32 {
        if self.rng == 0 {
            self.rng = 0x9e37_79b9;
        }
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    /// A vehicle with a doll that has no driven slot.
    pub fn is_empty(&self) -> bool {
        self.props.is_empty()
    }

    /// `(slot, effect id, controller state)` per mount point; `None` = no controller yet.
    pub fn report(&self, sys: &EffectSystem) -> Vec<(i32, u32, Option<u8>)> {
        self.props
            .iter()
            .map(|p| (p.slot, p.id, p.ctl.as_ref().and_then(|c| sys.state(c))))
            .collect()
    }

    /// One `rand() % 500 > 495` draw (xorshift32 here; the native generator is the C runtime's).
    fn burst_roll(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x % BURST_MODULUS > BURST_ABOVE
    }

    /// Seed the generator (never 0).
    pub fn seed(&mut self, seed: u32) {
        self.rng = seed | 1;
    }

    /// SetMatrix (vehicle matrix composed with the local one, 0x00631d00), `Play(0)` when it is
    /// not running, velocity (0x00780690) and input 0 = squared speed (0x00638f00, values below
    /// 0.001 become 0; the smoke and fire effects bind no input, smoke-effects.md).
    fn run(&mut self, sys: &mut EffectSystem, slot: i32, s: &FxState) {
        let speed_sq = s.velocity.length_squared();
        let speed_sq = if speed_sq < 0.001 { 0.0 } else { speed_sq };
        for p in self.props.iter_mut().filter(|p| p.slot == slot) {
            if p.ctl.is_none() && !p.missing {
                p.ctl = sys.create(p.id);
                p.missing = p.ctl.is_none();
            }
            let Some(c) = p.ctl.as_ref() else { continue };
            sys.set_transform(c, s.matrix * p.local);
            sys.play_controller(c, &[], false);
            sys.set_velocity(c, s.velocity);
            sys.set_input(c, jc2_formats::hash::hash_str("speed"), speed_sq);
        }
    }

    /// `Stop(0)` on the running controllers of a slot.
    fn stop(&mut self, sys: &mut EffectSystem, slot: i32, immediate: bool) {
        for p in self.props.iter().filter(|p| p.slot == slot) {
            if let Some(c) = &p.ctl
                && matches!(sys.state(c), Some(STATE_RUNNING | STATE_PENDING))
            {
                sys.stop(c, immediate);
            }
        }
    }

    /// Stop everything (the vehicle goes away).
    pub fn stop_all(&mut self, sys: &mut EffectSystem, immediate: bool) {
        for slot in Self::SLOTS.into_iter().chain(Self::DEATH_SLOTS) {
            self.stop(sys, slot, immediate);
        }
    }

    /// The per-frame owners: smoke 0x00516640 (slots 12 and 14), fire 0x0063fda0 (13 and 15) and the
    /// air shot-down starter (slot 8, 0x0073d740: started when burning begins; stopped by the death
    /// routine and, non-native, when the flag clears).
    pub fn drive(&mut self, sys: &mut EffectSystem, s: &FxState) {
        let smoking = s.smoking && !s.submerged;
        let burning = s.burning && !s.submerged;
        if smoking {
            self.run(sys, effect_slot::ENGINE_SMOKING, s);
            if s.engine_on && self.burst_roll() {
                self.run(sys, effect_slot::ENGINE_SMOKE_BURST, s);
            }
        } else {
            self.stop(sys, effect_slot::ENGINE_SMOKING, false);
            self.stop(sys, effect_slot::ENGINE_SMOKE_BURST, false);
        }
        if s.exploded {
            self.stop(sys, effect_slot::ENGINE_BURNING, false);
            if burning {
                self.run(sys, effect_slot::POST_EXPLOSION_FIRE, s);
            } else {
                self.stop(sys, effect_slot::POST_EXPLOSION_FIRE, false);
            }
        } else if burning {
            self.run(sys, effect_slot::ENGINE_BURNING, s);
        } else {
            self.stop(sys, effect_slot::ENGINE_BURNING, false);
        }
        if self.air {
            if s.exploded || !s.burning {
                self.stop(sys, effect_slot::SHOTDOWN, false);
            } else if !self.prev_burning || self.slot_running(sys, effect_slot::SHOTDOWN) {
                self.run(sys, effect_slot::SHOTDOWN, s);
            }
        }
        self.prev_burning = s.burning;
    }

    fn slot_running(&self, sys: &EffectSystem, slot: i32) -> bool {
        self.props.iter().filter(|p| p.slot == slot).any(|p| {
            p.ctl
                .as_ref()
                .is_some_and(|c| sys.state(c) == Some(STATE_RUNNING))
        })
    }
}

/// Run the owners for every vehicle with mount points.
#[allow(clippy::type_complexity)]
fn drive_effects(
    sys: Option<ResMut<EffectSystem>>,
    mut vehicles: Query<(
        Entity,
        &mut VehicleFx,
        &VehicleDamage,
        &GlobalTransform,
        Option<&LinearVelocity>,
        Has<Occupied>,
        Has<Wreck>,
    )>,
) {
    let Some(mut sys) = sys else { return };
    for (e, mut fx, vd, gtf, vel, occupied, exploded) in &mut vehicles {
        if fx.is_empty() {
            continue;
        }
        if fx.rng == 0 {
            fx.seed(e.index().index().wrapping_mul(2_654_435_761));
        }
        fx.drive(
            &mut sys,
            &FxState {
                smoking: vd.smoking,
                burning: vd.burning,
                exploded,
                submerged: vd.submerged,
                engine_on: occupied,
                matrix: gtf.to_matrix(),
                velocity: vel.map_or(Vec3::ZERO, |v| v.0),
            },
        );
    }
}

/// A vehicle despawns (tile unload, cleanup): its running controllers are stopped at once, the
/// handles drop with the component.
fn stop_on_remove(
    remove: On<Remove, VehicleFx>,
    mut vehicles: Query<&mut VehicleFx>,
    sys: Option<ResMut<EffectSystem>>,
) {
    let (Some(mut sys), Ok(mut fx)) = (sys, vehicles.get_mut(remove.entity)) else {
        return;
    };
    fx.stop_all(&mut sys, true);
}

/// Contact damage (0x00805700). The impulse is the contact pair's summed normal impulse of the last
/// physics step (kg m/s); the native value is Havok's per-callback impulse (inferred equivalent).
/// Contacts between two parts of the same vehicle do not count. Not native: which other objects are
/// excluded (the class tested by 0x0056aec0 is not identified).
#[allow(clippy::type_complexity)]
fn collision_damage(
    graph: Res<ContactGraph>,
    mut vehicles: Query<
        (&mut VehicleDamage, &mut Health, &ComputedMass, Has<Occupied>),
        Without<Wreck>,
    >,
) {
    for pair in graph.iter_active_touching() {
        let impulse = pair.total_normal_impulse_magnitude();
        if impulse <= 0.0 {
            continue;
        }
        let (a, b) = (pair.body1, pair.body2);
        if a.is_some() && a == b {
            continue;
        }
        for body in [a, b].into_iter().flatten() {
            let Ok((mut vd, mut h, mass, occupied)) = vehicles.get_mut(body) else {
                continue;
            };
            if h.hp <= 0.0 {
                continue;
            }
            let outcome = vd.collision(impulse, mass.value(), occupied);
            vd.apply_collision(&mut h, outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The jeep's block (`CarDamage_Medium`): 750 hp, wreck 100, type 1 / 3 thresholds 5 / 7, x0.75.
    fn jeep() -> VehicleDamage {
        VehicleDamage::new(
            DamageModule {
                max_health: 750.0,
                instant_wreck: 100.0,
                type1_threshold: 5.0,
                type3_threshold: 7.0,
                collision_multiplier: 0.75,
                ..DamageModule::default()
            },
            BurnTuning::default(),
        )
    }

    #[test]
    fn type_table_subtracts_the_threshold() {
        let v = jeep();
        assert_eq!(v.type_damage(DamageType::Shot, 30.0), 30.0);
        assert_eq!(v.type_damage(DamageType::Type1, 12.0), 7.0);
        assert_eq!(v.type_damage(DamageType::Collision, 7.0), 0.0);
        assert_eq!(v.type_damage(DamageType::Collision, 10.0), 3.0);
    }

    #[test]
    fn collision_formula_and_threshold() {
        let v = jeep();
        // 3 * 15000 / 1500 * 0.75 = 22.5, minus 7
        assert!((v.collision_local(15000.0, 1500.0) - 22.5).abs() < 1e-4);
        match v.collision(15000.0, 1500.0, false) {
            CollisionOutcome::Damage(d) => assert!((d - 15.5).abs() < 1e-3, "{d}"),
            o => panic!("{o:?}"),
        }
        // a resting contact (one frame of weight) loses nothing
        assert_eq!(v.collision(245.0, 1500.0, false), CollisionOutcome::None);
    }

    #[test]
    fn instant_wreck_threshold_and_occupant_factor() {
        let mut v = jeep();
        // local = 3 * 100000 / 1500 * 0.75 = 150 > 100: wreck for an empty car ...
        assert_eq!(
            v.collision(100_000.0, 1500.0, false),
            CollisionOutcome::InstantWreck
        );
        // ... but not with the player in it (threshold x2 = 200): damage instead
        assert!(matches!(
            v.collision(100_000.0, 1500.0, true),
            CollisionOutcome::Damage(_)
        ));
        // +0x36a: threshold x0.05 = 5
        v.seat_flag_36a = true;
        assert_eq!(
            v.collision(15000.0, 1500.0, true),
            CollisionOutcome::InstantWreck
        );
        // invulnerable: never wrecked
        v.invulnerable = true;
        assert!(!matches!(
            v.collision(15000.0, 1500.0, true),
            CollisionOutcome::InstantWreck
        ));
        let mut v = jeep();
        let mut h = Health::new(750.0);
        v.apply_collision(&mut h, CollisionOutcome::InstantWreck);
        assert_eq!((h.hp, v.burn_seconds), (0.0, INSTANT_WRECK_BURN_SECONDS));
    }

    #[test]
    fn shots_pin_at_five_percent() {
        let mut v = jeep();
        let mut h = Health::new(750.0);
        assert_eq!(v.shot(&mut h, 100.0, false, true), 100.0);
        assert_eq!(h.hp, 650.0);
        // a shot that would leave 4 % (30 hp) pins at 5 % (37.5) instead of killing
        h.hp = 100.0;
        v.shot(&mut h, 70.0, false, true);
        assert_eq!(h.hp, 37.5);
        // and a shot that would kill it
        v.shot(&mut h, 5000.0, false, true);
        assert_eq!(h.hp, 37.5);
        // exploded vehicles take nothing; a failed source check does not pin
        assert_eq!(v.shot(&mut h, 10.0, true, true), 0.0);
        let mut h = Health::new(750.0);
        v.shot(&mut h, 800.0, false, false);
        assert!(h.hp <= 0.0);
        // not pinned once the burn timer has passed the burn seconds
        let mut h = Health::new(750.0);
        v.burn_timer = v.burn_seconds + 0.1;
        v.shot(&mut h, 800.0, false, true);
        assert!(h.hp <= 0.0);
    }

    fn tick(v: &mut VehicleDamage, h: &mut Health, dt: f32) {
        v.step(
            h,
            &StepInput {
                dt,
                ..default()
            },
        );
    }

    #[test]
    fn a_pinned_vehicle_burns_out() {
        let mut v = jeep();
        let mut h = Health::new(750.0);
        h.hp = 100.0;
        v.shot(&mut h, 70.0, false, true);
        assert_eq!(h.hp, 37.5);
        // 5 % < the 0.1 default burn threshold: burning, dead after 8 s
        let mut t = 0.0_f32;
        while h.hp > 0.0 && t < 20.0 {
            tick(&mut v, &mut h, 0.25);
            t += 0.25;
        }
        assert!(v.burning);
        assert!((t - 8.25).abs() < 1e-3, "died at {t}");
        // healthy vehicles do not burn
        let mut v = jeep();
        let mut h = Health::new(750.0);
        tick(&mut v, &mut h, 100.0);
        assert!(!v.burning && !v.smoking && h.hp == 750.0);
    }

    /// Thresholds: smoke below the smoke threshold, burning (smoke off) below the burn threshold.
    #[test]
    fn smoke_then_fire_at_the_thresholds() {
        let mut v = jeep();
        v.burn.smoke_threshold = 0.4;
        v.burn.threshold = 0.2;
        let mut h = Health::new(100.0);
        h.hp = 40.0; // exactly the threshold: not below
        tick(&mut v, &mut h, 0.016);
        assert!(!v.smoking);
        h.hp = 39.0;
        tick(&mut v, &mut h, 0.016);
        assert!(v.smoking && !v.burning);
        h.hp = 19.0;
        tick(&mut v, &mut h, 0.016);
        assert!(v.burning && !v.smoking);
        // the smoke does not come back while burning
        h.hp = 30.0;
        tick(&mut v, &mut h, 0.016);
        // (non-native repair: above the burn threshold the burn stops, the smoke returns)
        assert!(!v.burning && v.smoking);
    }

    /// The burn timer: runs below the threshold, kills past the burn seconds, 4 s variants.
    #[test]
    fn burn_timer_kills_after_burn_seconds() {
        for seconds in [8.0_f32, 4.0] {
            let mut v = jeep();
            v.burn_seconds = seconds;
            let mut h = Health::new(100.0);
            h.hp = 5.0;
            let mut t = 0.0;
            while h.hp > 0.0 && t < 30.0 {
                tick(&mut v, &mut h, 0.5);
                t += 0.5;
            }
            assert!((t - seconds - 0.5).abs() < 1e-3, "{seconds}: died at {t}");
            assert_eq!(v.burn_timer, seconds);
        }
    }

    /// A collision wreck (health straight to 0) still burns on that frame, so the post-explosion
    /// fire runs; after the explosion the flags clear at the post-explosion seconds and stay clear.
    #[test]
    fn explosion_then_post_explosion_timer() {
        let mut v = jeep();
        v.burn.post_explosion_seconds = Some(8.0);
        let mut h = Health::new(100.0);
        h.hp = 0.0;
        tick(&mut v, &mut h, 0.016);
        assert!(v.burning, "burning set on the frame of death");
        let dead = StepInput {
            dt: 1.0,
            exploded: true,
            ..default()
        };
        for _ in 0..7 {
            v.step(&mut h, &dead);
        }
        assert!(v.burning, "still burning at 7 s");
        v.step(&mut h, &dead);
        assert!(!v.burning && !v.smoking, "cleared at 8 s");
        v.step(&mut h, &dead);
        assert!(!v.burning, "an exploded vehicle never re-ignites");
    }

    /// Water hysteresis: above 0.8 the flags go and the vehicle is submerged until the ratio is
    /// below 0.6 (between 0.6 and 0.8 it stays submerged).
    #[test]
    fn water_hysteresis() {
        let mut v = jeep();
        let mut h = Health::new(100.0);
        h.hp = 30.0;
        tick(&mut v, &mut h, 0.016);
        assert!(v.smoking);
        let wet = |water| StepInput {
            dt: 0.016,
            water,
            ..default()
        };
        v.step(&mut h, &wet(0.85));
        assert!(v.submerged && !v.smoking);
        v.step(&mut h, &wet(0.7));
        assert!(v.submerged, "between 0.6 and 0.8 it stays submerged");
        v.step(&mut h, &wet(0.5));
        assert!(!v.submerged);
        v.step(&mut h, &wet(0.0));
        assert!(v.smoking, "the smoke comes back once the vehicle is dry");
    }

    /// The flip burn accumulates while upside down and nearly stopped; its limit is unknown, so it
    /// never fires, and the accumulator resets when the vehicle moves.
    #[test]
    fn flip_burn_limit_is_unknown() {
        let mut v = jeep();
        let mut h = Health::new(100.0);
        let flipped = StepInput {
            dt: 1.0,
            upside_down: true,
            speed_sq: 1.0,
            ..default()
        };
        for _ in 0..100 {
            v.step(&mut h, &flipped);
        }
        assert_eq!((v.flip_timer, h.hp), (100.0, 100.0));
        v.step(
            &mut h,
            &StepInput {
                speed_sq: 100.0,
                ..flipped
            },
        );
        assert_eq!(v.flip_timer, 0.0);
    }

    /// The smoke burst is 4 draws in 500 (rand % 500 > 495): the xorshift stream stays near 0.8 %.
    #[test]
    fn burst_rate_is_four_in_five_hundred() {
        let mut fx = VehicleFx::new(&[]);
        fx.seed(12345);
        let n = 500_000;
        let hits = (0..n).filter(|_| fx.burst_roll()).count();
        let rate = hits as f32 / n as f32;
        assert!((rate - 0.008).abs() < 0.001, "{rate}");
    }

    /// A 1500 kg block dropped on flat ground at `speed` m/s: resting contact costs nothing, a crash
    /// costs health through the thresholds, a hard enough one wrecks it.
    fn crash(speed: f32, rest_only: bool) -> (f32, f32) {
        let mut app = crate::physics::tests::headless_app();
        app.insert_resource(Gravity(Vec3::NEG_Y * 9.81))
            .add_systems(Update, damage_state)
            .add_systems(
                FixedPostUpdate,
                collision_damage.in_set(PhysicsSystems::Last),
            );
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(400.0, 2.0, 400.0),
            Transform::from_xyz(0.0, -1.0, 0.0),
        ));
        let car = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::cuboid(2.0, 1.0, 4.4),
                Mass(1500.0),
                LinearVelocity(Vec3::NEG_Y * if rest_only { 0.0 } else { speed }),
                Transform::from_xyz(0.0, if rest_only { 0.55 } else { 1.5 }, 0.0),
                Health::new(750.0),
                jeep(),
            ))
            .id();
        for _ in 0..192 {
            app.update();
        }
        let hp = app.world().get::<Health>(car).unwrap().hp;
        let burn = app.world().get::<VehicleDamage>(car).unwrap().burn_seconds;
        eprintln!("crash at {speed} m/s (rest {rest_only}): hp {hp}");
        (hp, burn)
    }

    #[test]
    fn contacts_hurt_the_vehicle() {
        // standing on the ground: nothing is lost
        assert_eq!(crash(0.0, true).0, 750.0);
        // a 25 m/s landing: 3 * dv * 0.75 - 7 = about 49 off, no wreck
        let (hp, burn) = crash(25.0, false);
        assert!(hp < 750.0 && hp > 600.0, "hp {hp}");
        assert_eq!(burn, DEFAULT_BURN_SECONDS);
        // 120 m/s: local > 100 (empty car, instant wreck)
        let (hp, burn) = crash(120.0, false);
        assert_eq!((hp, burn), (0.0, INSTANT_WRECK_BURN_SECONDS));
    }

    #[test]
    fn max_health_switch() {
        let m = jeep().module;
        assert_eq!(max_health(MaxHealthSource::DollModule, &m, Some(1980.0)), 750.0);
        assert_eq!(max_health(MaxHealthSource::EpeHundred, &m, Some(1980.0)), 100.0);
        assert_eq!(max_health(MaxHealthSource::VehicleTypes, &m, Some(1980.0)), 1980.0);
        assert_eq!(max_health(MaxHealthSource::VehicleTypes, &m, None), 750.0);
    }

    /// The dev fixture (FX-VEH-1): burns a car, a bike, a boat and a helicopter through the real
    /// retail doll mounts, effect files and the state machine, and prints the controller state of
    /// every mount per phase (`cargo test ... burning_fixture -- --nocapture`). Needs JC2_TEST_DIR.
    #[test]
    fn burning_fixture() {
        use crate::effects::{EffectHost, EffectSystem, default_adapters, particle::ParticleManager};
        use crate::effects::system::{STATE_DEAD, STATE_RUNNING};
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = std::sync::Arc::new(jc2_formats::GameFs::open(dir).unwrap());
        let host = EffectHost::default();
        *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
        let mut sys = EffectSystem::from_retail(fs.clone(), default_adapters(&host)).unwrap();
        for (container, label) in [
            ("lave.v014_jeep", "car"),
            ("lave.v006_moped", "bike"),
            ("seve.v082_motorboat_standard_small", "boat"),
            ("arve.v059_civilian_helicopter", "helicopter"),
        ] {
            let Ok(data) = fs.read_inflated(&format!("{container}.ee")) else {
                continue;
            };
            let sarc = jc2_formats::sarc::SmallArchive::parse(&data).unwrap();
            let doll = sarc
                .entries
                .iter()
                .find(|e| e.name.ends_with("vdoll"))
                .and_then(|e| jc2_formats::vehicle::VehicleDoll::parse(sarc.data(&data, e).ok()?).ok())
                .expect("doll");
            let burn: BurnTuning = sarc
                .get(&format!("{container}.epe"))
                .and_then(|e| sarc.data(&data, e).ok())
                .and_then(|d| jc2_formats::compress::maybe_inflate(d).ok())
                .map(|d| jc2_formats::vehicle::BurnTuning::from_epe(&d))
                .unwrap_or_default()
                .into();
            let mut fx = VehicleFx::new(&doll.effects);
            fx.seed(7);
            let mut v = VehicleDamage::new(DamageModule::default(), burn);
            let mut h = Health::new(100.0);
            let state = |v: &VehicleDamage, exploded, vel: Vec3| FxState {
                smoking: v.smoking,
                burning: v.burning,
                exploded,
                submerged: v.submerged,
                engine_on: true,
                matrix: Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0)),
                velocity: vel,
            };
            let dt = 1.0 / 30.0;
            let mut exploded = false;
            let run = |phase: &str,
                           secs: f32,
                           v: &mut VehicleDamage,
                           h: &mut Health,
                           exploded: &mut bool,
                           sys: &mut EffectSystem,
                           fx: &mut VehicleFx| {
                for _ in 0..(secs / dt) as usize {
                    v.step(
                        h,
                        &StepInput {
                            dt,
                            exploded: *exploded,
                            ..default()
                        },
                    );
                    if h.hp <= 0.0 {
                        *exploded = true;
                    }
                    fx.drive(sys, &state(v, *exploded, Vec3::new(3.0, 0.0, 0.0)));
                    sys.tick(dt);
                }
                let report: Vec<String> = fx
                    .report(sys)
                    .iter()
                    .map(|(slot, id, st)| format!("{slot}:{id}:{}", st.map_or("-".into(), |s| s.to_string())))
                    .collect();
                eprintln!(
                    "{label} {container} [{phase}] hp {:.0} smoking {} burning {} burn {:.1}s post {:.1}s | slot:id:state {}",
                    h.hp, v.smoking, v.burning, v.burn_timer, v.post_timer, report.join(" ")
                );
            };
            let states = |fx: &VehicleFx, sys: &EffectSystem, slot: i32| -> Vec<Option<u8>> {
                fx.report(sys).iter().filter(|r| r.0 == slot).map(|r| r.2).collect()
            };
            run("healthy", 1.0, &mut v, &mut h, &mut exploded, &mut sys, &mut fx);
            assert!(fx.report(&sys).iter().all(|r| r.2.is_none()), "nothing before damage");
            h.hp = 30.0;
            run("30 percent", 2.0, &mut v, &mut h, &mut exploded, &mut sys, &mut fx);
            assert!(v.smoking && !v.burning);
            for st in states(&fx, &sys, 12) {
                assert_eq!(st, Some(STATE_RUNNING), "smoke running");
            }
            for st in states(&fx, &sys, 13) {
                assert_eq!(st, None, "no fire yet");
            }
            h.hp = 8.0;
            run("8 percent", 2.0, &mut v, &mut h, &mut exploded, &mut sys, &mut fx);
            assert!(v.burning && !v.smoking);
            for st in states(&fx, &sys, 13) {
                assert_eq!(st, Some(STATE_RUNNING), "fire running");
            }
            for st in states(&fx, &sys, 12) {
                assert_ne!(st, Some(STATE_RUNNING), "smoke stopped when the fire starts");
            }
            // non-native repair: the fire stops and the smoke returns
            h.hp = 30.0;
            run("repaired to 30", 2.0, &mut v, &mut h, &mut exploded, &mut sys, &mut fx);
            assert!(!v.burning && v.smoking);
            for st in states(&fx, &sys, 13) {
                assert_ne!(st, Some(STATE_RUNNING), "fire stopped on repair");
            }
            h.hp = 8.0;
            run("burning again", 1.0, &mut v, &mut h, &mut exploded, &mut sys, &mut fx);
            // the burn timer ends the vehicle
            run("burn out", burn.seconds + 1.0, &mut v, &mut h, &mut exploded, &mut sys, &mut fx);
            assert!(exploded && h.hp <= 0.0, "the burn timer killed it");
            for st in states(&fx, &sys, 13) {
                assert_ne!(st, Some(STATE_RUNNING), "slot 13 stops at the explosion");
            }
            for st in states(&fx, &sys, 15) {
                assert_eq!(st, Some(STATE_RUNNING), "post-explosion fire runs");
            }
            for st in states(&fx, &sys, 8) {
                assert_ne!(st, Some(STATE_RUNNING), "the shot-down effect stops at the explosion");
            }
            run("wreck", 9.0, &mut v, &mut h, &mut exploded, &mut sys, &mut fx);
            assert!(!v.burning);
            for st in states(&fx, &sys, 15) {
                assert_ne!(st, Some(STATE_RUNNING), "post-explosion fire ended");
            }
            // the controllers' components end by themselves only because Stop closes the
            // unbounded emitters: run the effect system on and see nothing is left running
            for _ in 0..600 {
                sys.tick(dt);
            }
            assert!(
                fx.report(&sys)
                    .iter()
                    .all(|r| matches!(r.2, None | Some(STATE_DEAD) | Some(crate::effects::system::STATE_TAIL) | Some(crate::effects::system::STATE_STOPPING)
                        )),
                "{:?}",
                fx.report(&sys)
            );
            fx.stop_all(&mut sys, true);
        }
    }
}
