//! The vehicle death sequence (FX-VEH-2): `CVehicle` slot 78 (0x00559150 and the class prologues
//! 0x00647bd0 car, 0x0080a7a0 bike, 0x0079ed50 air, 0x0061fad0 plane, 0x007ff860 boat). Research:
//! `research/formats/vehicle-fire-explosion.md` sections 4 to 6.
//!
//! What this module does when a vehicle's health reaches 0 (the frame `explosion::wreck_vehicles`
//! turns it into a [`crate::explosion::Wreck`]):
//! * posts `gsy.object_destroyed` (SetHealth, 0x007e87a0) and the class event
//!   (`government.landvehicle|airvehicle|seavehicle.destroyed`) as [`GameEvent`] messages;
//! * plays the explosion effect of the vehicle's own doll: every slot 1 mount, slot 5 when
//!   submerged (a vehicle without the slot plays nothing: cars and boats have no slot 5) with the
//!   vehicle matrix ([`crate::vehicle_damage::VehicleFx::explode`]);
//! * throws three wheels as rigid bodies (cars and bikes).
//!
//! The post-explosion fire (slot 15, `post_explosion_fire_seconds`) is `VehicleFx::drive`; the code
//! pulse, the flash and the charred paint are in `explosion::wreck_vehicles`.

use crate::combat::Health;
use crate::effects::EffectSystem;
use crate::fleet::Container;
use crate::vehicle::{Vehicle, WheelVisual};
use crate::vehicle_damage::{EVENT_OBJECT_DESTROYED, VehicleDamage, VehicleFx};
use avian3d::prelude::*;
use bevy::prelude::*;

/// A game event the mission and activity scripts listen to (`gsy.*`, `government.*`). The port has
/// no script bus yet: the events are posted as messages so the first consumer finds them.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct GameEvent {
    pub name: &'static str,
    /// The object the event is about.
    pub source: Entity,
}

/// How many wheels the car prologue throws: three passes (0x00647bd0).
pub const WHEEL_PASSES: usize = 3;
/// The mass handed to a thrown wheel's rigid body: the value 20.0 (`0x41a00000`) passed to
/// 0x00612ac0. Its meaning as a mass is *inferred*.
pub const THROWN_WHEEL_MASS: f32 = 20.0;
/// Below this squared speed the native code adds a random 0..1 share of a fixed vector (0x00612ac0).
pub const SLOW_SPEED_SQ: f32 = 16.0;
/// NON-NATIVE: the fixed vector's value is not read (Gap 1 of the FX-VEH-2 hand-in): an upward kick
/// plus a push away from the chassis, scaled by one random 0..1 draw.
pub const SLOW_KICK: Vec3 = Vec3::new(2.0, 4.0, 2.0);
/// NON-NATIVE: the original keeps the wheel bodies; the port frees the physics bodies after this.
pub const THROWN_WHEEL_SECONDS: f32 = 30.0;

/// The charred look (`vehicle-damage-effects.md` section 7, `vehicle-fire-explosion.md` 5.1).
/// Native constants: slot 31 writes these into the paint metadata of every part model.
pub const CHAR_PAINT_1: f32 = 0.094;
#[allow(dead_code)] // the second paint colour: the port's materials have one base colour
pub const CHAR_PAINT_2: f32 = 0.078;
/// Native: the instance tint bytes slot 31 / slot 85 write (RGBA at +0x24..+0x27, and +0x20, +0x21).
/// Their consumer is not traced (Gap 2 in the research doc), so they do not feed the port's colour.
#[allow(dead_code)]
pub const CHAR_TINT_BYTES: [u8; 4] = [28, 28, 25, 252];
#[allow(dead_code)]
pub const CHAR_FLAG_BYTES: [u8; 2] = [25, 255];

/// The base colour of a charred material. The two constants are the first and second paint colour
/// the shader reads (decoded like the palette entries, `pow(c, 2.2)`, so they are linear). Mapping
/// the first one onto the single `base_color` of the port's materials is **non-native** (the real
/// materials carry two paint colours and a damage layer; the CarPaint milestone, task f0cd863b).
pub fn charred_colour() -> Color {
    Color::linear_rgb(CHAR_PAINT_1, CHAR_PAINT_1, CHAR_PAINT_1)
}

/// The wheels the car prologue launches: three passes over consecutive wheels from a random start
/// (modulus the wheel count, +0x370); a wheel already detached (+0x181) is skipped, so a vehicle with
/// fewer than three wheels throws each once. The "consecutive" reading of the three passes is
/// *inferred* from the decompilation.
pub fn wheel_throw_indices(start: usize, count: usize) -> Vec<usize> {
    let mut out = Vec::new();
    if count == 0 {
        return out;
    }
    for pass in 0..WHEEL_PASSES {
        let i = (start + pass) % count;
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

/// A wheel flying off a dead vehicle.
#[derive(Component)]
pub struct ThrownWheel {
    pub age: f32,
}

pub(crate) fn plugin(app: &mut App) {
    app.add_message::<GameEvent>()
        .add_systems(Update, thrown_wheels);
}

/// The death routine of every vehicle whose health reached 0 and has not exploded yet. Runs just
/// before `explosion::wreck_vehicles`, which sets the [`crate::explosion::Wreck`] marker.
#[allow(clippy::type_complexity)]
pub(crate) fn vehicle_death(
    mut commands: Commands,
    sys: Option<ResMut<EffectSystem>>,
    mut events: MessageWriter<GameEvent>,
    mut vehicles: Query<
        (
            Entity,
            &Health,
            &GlobalTransform,
            &mut VehicleFx,
            &VehicleDamage,
            Option<&Vehicle>,
            Option<&LinearVelocity>,
            Option<&Children>,
        ),
        (With<Container>, Without<crate::explosion::Wreck>),
    >,
    parts: Query<(&WheelVisual, &GlobalTransform)>,
) {
    let mut sys = sys;
    for (e, h, gtf, mut fx, vd, car, vel, children) in &mut vehicles {
        if h.hp > 0.0 {
            continue;
        }
        // events: SetHealth posts the generic one, the class prologue the specific one
        events.write(GameEvent {
            name: EVENT_OBJECT_DESTROYED,
            source: e,
        });
        events.write(GameEvent {
            name: fx.class().event_name(),
            source: e,
        });
        let matrix = gtf.to_matrix();
        if let Some(sys) = sys.as_mut() {
            let n = fx.explode(sys, matrix, vd.submerged);
            debug!(
                "vehicle {e}: death effect slot {} x{n}",
                VehicleFx::death_slot(vd.submerged)
            );
        }
        // three wheels thrown (cars and bikes: the car prologue; the other classes have none here)
        let (Some(car), Some(children)) = (car, children) else {
            continue;
        };
        let velocity = vel.map_or(Vec3::ZERO, |v| v.0);
        let start = fx.next_random() as usize;
        for index in wheel_throw_indices(start, car.spec.wheels.len()) {
            let mine: Vec<(Entity, GlobalTransform)> = children
                .iter()
                .filter_map(|c| {
                    let (wv, g) = parts.get(c).ok()?;
                    (wv.0 == index).then_some((c, *g))
                })
                .collect();
            let Some((_, first)) = mine.first() else {
                continue;
            };
            let spec = &car.spec.wheels[index];
            let body_at = Transform::from_translation(first.translation())
                .with_rotation(first.rotation());
            let body_world = body_at.to_matrix();
            let mut v = velocity;
            if velocity.length_squared() < SLOW_KICK_BELOW {
                let k = fx.next_random() as f32 / u32::MAX as f32;
                let outward = (first.translation() - gtf.translation()).with_y(0.0).normalize_or_zero();
                v += (Vec3::Y * SLOW_KICK.y + outward * SLOW_KICK.x) * k;
            }
            let body = commands
                .spawn((
                    body_at,
                    Visibility::default(),
                    RigidBody::Dynamic,
                    Collider::sphere(spec.radius),
                    Mass(THROWN_WHEEL_MASS),
                    LinearVelocity(v),
                    ThrownWheel { age: 0.0 },
                    Name::new("thrown wheel"),
                ))
                .id();
            for (part, g) in mine {
                let local = Transform::from_matrix(body_world.inverse() * g.to_matrix());
                commands
                    .entity(part)
                    .remove::<WheelVisual>()
                    .insert((ChildOf(body), local));
            }
        }
    }
}

const SLOW_KICK_BELOW: f32 = SLOW_SPEED_SQ;

fn thrown_wheels(
    mut commands: Commands,
    time: Res<Time>,
    mut wheels: Query<(Entity, &mut ThrownWheel)>,
) {
    for (e, mut w) in &mut wheels {
        w.age += time.delta_secs();
        if w.age > THROWN_WHEEL_SECONDS {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vehicle_damage::{
        DeathClass, EVENT_AIR_DESTROYED, EVENT_LAND_DESTROYED, EVENT_SEA_DESTROYED,
    };

    #[test]
    fn event_names_by_class() {
        assert_eq!(EVENT_OBJECT_DESTROYED, "gsy.object_destroyed");
        assert_eq!(DeathClass::Land.event_name(), "government.landvehicle.destroyed");
        assert_eq!(DeathClass::Air.event_name(), "government.airvehicle.destroyed");
        assert_eq!(DeathClass::Sea.event_name(), "government.seavehicle.destroyed");
        assert_eq!(DeathClass::Land.event_name(), EVENT_LAND_DESTROYED);
        assert_eq!(DeathClass::Air.event_name(), EVENT_AIR_DESTROYED);
        assert_eq!(DeathClass::Sea.event_name(), EVENT_SEA_DESTROYED);
    }

    #[test]
    fn three_consecutive_wheels_from_a_random_start() {
        assert_eq!(wheel_throw_indices(3, 4), vec![3, 0, 1]);
        assert_eq!(wheel_throw_indices(7, 4), vec![3, 0, 1]);
        assert_eq!(wheel_throw_indices(0, 6), vec![0, 1, 2]);
        // a bike has two wheels: each goes once
        assert_eq!(wheel_throw_indices(5, 2), vec![1, 0]);
        assert!(wheel_throw_indices(5, 0).is_empty());
    }

    #[test]
    fn explosion_slot_follows_the_water() {
        assert_eq!(VehicleFx::death_slot(false), 1);
        assert_eq!(VehicleFx::death_slot(true), 5);
    }

    #[test]
    fn the_wreck_colour_is_the_native_constant() {
        let c = charred_colour().to_linear();
        assert!((c.red - 0.094).abs() < 1e-6 && (c.blue - 0.094).abs() < 1e-6);
        assert_eq!(CHAR_TINT_BYTES, [28, 28, 25, 252]);
    }

    fn spec(wheels: usize) -> crate::vehicle::VehicleSpec {
        let wheel = |i: usize| crate::vehicle::WheelSpec {
            hub: Vec3::new(if i.is_multiple_of(2) { -1.0 } else { 1.0 }, 0.4, i as f32),
            radius: 0.35,
            travel: 0.2,
            spring: 1.0,
            damp_compress: 1.0,
            damp_rebound: 1.0,
            friction: 1.0,
            steers: false,
            drives: false,
            handbrake: false,
            brake_force: 1.0,
            base_rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            number: i as u32,
        };
        crate::vehicle::VehicleSpec {
            name: "test".into(),
            mass: 1000.0,
            center_of_mass: Vec3::ZERO,
            wheels: (0..wheels).map(wheel).collect(),
            max_steer: 0.5,
            steer_speed: 10.0,
            torque: 1.0,
            torque_rpm: 1.0,
            rpm_min: 1.0,
            rpm_max: 2.0,
            gears: vec![1.0],
            reverse: 1.0,
            final_drive: 1.0,
            top_speed: 10.0,
            drag: 0.0,
        }
    }

    /// A dead car posts both events once and throws three of its four wheels as rigid bodies.
    #[test]
    fn a_dead_car_posts_events_and_throws_three_wheels() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<GameEvent>()
            .add_systems(Update, vehicle_death);
        let car = app
            .world_mut()
            .spawn((
                Container("lave.v014_jeep".into()),
                Health { hp: 0.0, max: 100.0 },
                GlobalTransform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                VehicleFx::new(&[]).with_class(DeathClass::Land),
                VehicleDamage::new(Default::default(), Default::default()),
                crate::vehicle::Vehicle {
                    spec: spec(4),
                    wheels: Vec::new(),
                    throttle: 0.0,
                    brake: 0.0,
                    steer_input: 0.0,
                    handbrake: false,
                    gear: 0,
                    rpm: 0.0,
                    speed: 0.0,
                    length: 4.0,
                    collider_children: Vec::new(),
                },
                LinearVelocity(Vec3::new(0.0, 0.0, 20.0)),
            ))
            .id();
        for i in 0..4usize {
            let hub = Vec3::new(if i.is_multiple_of(2) { -1.0 } else { 1.0 }, 1.4, i as f32);
            app.world_mut().spawn((
                WheelVisual(i),
                GlobalTransform::from_translation(hub),
                Transform::from_translation(hub),
                ChildOf(car),
            ));
        }
        app.update();
        let events: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<GameEvent>>()
            .drain()
            .map(|e| e.name)
            .collect();
        assert_eq!(
            events,
            vec!["gsy.object_destroyed", "government.landvehicle.destroyed"]
        );
        let mut q = app.world_mut().query::<&ThrownWheel>();
        assert_eq!(q.iter(app.world()).count(), 3);
        let mut bodies = app.world_mut().query::<(&ThrownWheel, &LinearVelocity, &Mass)>();
        for (_, v, m) in bodies.iter(app.world()) {
            assert_eq!(v.0.z, 20.0, "the vehicle velocity is inherited");
            assert_eq!(m.0, THROWN_WHEEL_MASS);
        }
        // three wheel parts now hang off wheel bodies, one stayed on the car
        let mut on_car = app.world_mut().query::<(&WheelVisual, &ChildOf)>();
        assert_eq!(on_car.iter(app.world()).filter(|(_, c)| c.parent() == car).count(), 1);
    }

    /// The vehicle death through the real doll mounts and effect files: a car, a bike, a boat, a
    /// helicopter and a plane play their own explosion (slot 5 when submerged, nothing for a
    /// vehicle without the slot), and the controllers are stopped again on cleanup. Needs JC2_TEST_DIR.
    #[test]
    fn death_fixture() {
        use crate::effects::{EffectHost, default_adapters, particle::ParticleManager};
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = std::sync::Arc::new(jc2_formats::GameFs::open(dir).unwrap());
        let host = EffectHost::default();
        *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
        let mut sys = EffectSystem::from_retail(fs.clone(), default_adapters(&host)).unwrap();
        // (container, class, slot 1 id, slot 5 id)
        for (container, class, dry, wet) in [
            ("lave.v014_jeep", DeathClass::Land, Some(128), None),
            ("lave.v006_moped", DeathClass::Land, Some(164), None),
            ("seve.v082_motorboat_standard_small", DeathClass::Sea, Some(156), None),
            ("arve.v059_civilian_helicopter", DeathClass::Air, Some(157), Some(44)),
            ("arve.v010_personal_propellerplane", DeathClass::Air, Some(128), Some(44)),
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
            let ids = |slot: i32| -> Option<u32> {
                doll.effects.iter().find(|e| e.slot == slot).map(|e| e.id)
            };
            assert_eq!(ids(1), dry, "{container} slot 1");
            assert_eq!(ids(5), wet, "{container} slot 5");
            for submerged in [false, true] {
                let mut fx = VehicleFx::new(&doll.effects).with_class(class);
                let m = Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0));
                let started = fx.explode(&mut sys, m, submerged);
                let want = if submerged { wet } else { dry };
                assert_eq!(started, usize::from(want.is_some()), "{container} submerged {submerged}");
                for _ in 0..30 {
                    sys.tick(1.0 / 30.0);
                }
                let running = fx
                    .report(&sys)
                    .iter()
                    .filter(|r| r.0 == VehicleFx::death_slot(submerged))
                    .filter_map(|r| r.2)
                    .count();
                eprintln!(
                    "{container} submerged {submerged}: slot {} effect {:?}, controllers with a state {running}",
                    VehicleFx::death_slot(submerged),
                    want
                );
                fx.stop_all(&mut sys, true);
            }
        }
    }
}
