//! Explosions: rockets in flight, thrown grenades on their fuse, the blast itself
//! (damage falling off over its `range`, a push on everything that moves, a fireball and
//! a flash), and vehicles that burn out when their health runs out — they blow up, lose
//! their driver, char black and smoke.
//!
//! The grenade's numbers are the game's (`weapons.bin`: `damage` 500, `impulse` 1000,
//! `range` 5 m, `detonation_time` 3.5 s); a rocket hits with its weapon's damage over the
//! grenade's radius, a burning-out vehicle with the values below (its explosion effect
//! is not decoded).

use crate::character::Hero;
use crate::combat::{Dead, Health};
use crate::fleet::Container;
use crate::heli::Heli;
use crate::vehicle::{CameraMode, Enterable, Occupied};
use avian3d::prelude::*;
use bevy::prelude::*;
use std::collections::HashMap;

/// A vehicle burning out. NON-NATIVE placeholder for the damage and the push (the original posts
/// no area damage, only the code pulse; who is hurt is Gap 3 of `vehicle-fire-explosion.md`). It
/// plays no effect: the explosion is the vehicle's own doll effect (slot 1 / 5, `vehicle_death`).
const WRECK_BLAST: Blast = Blast {
    at: Vec3::ZERO,
    radius: 9.0,
    damage: 450.0,
    impulse: 1500.0,
    effect: None,
};
/// The game's impulses are in its own units; this turns them into a push that throws a
/// car (tuned, not from the data).
const IMPULSE_SCALE: f32 = 8.0;
/// NON-NATIVE fallback (coordinator decision, 12 October 2026): the old placeholder fireball
/// (a growing translucent sphere, assumed 0.7 s) is drawn at every blast in addition to the
/// native effect. Re-evaluated in aee5ba94 (size, birth direction, 12 linked systems and the pixel
/// stage are native now): the smoke column, sparks and debris are right, but the hot flash/fire
/// leaves (`fx_exp_grenade_01/03/04`, screen blend) are still much fainter than the original frames
/// in `reference/effects/grenade-explosion-*` (lighting, emissive and exact blend unresolved), so
/// it stays. Re-checked (5d34822d): with particle textures read as raw bytes (native colour-view
/// policy) the 0.1 s flash and the smoke match the reference, but from about 0.2 s the native leaves
/// show only grey smoke while the original keeps an orange fire body lit by the effect light: the
/// particle lighting of bit6-clear leaves (`cb3[5]`/`cb3[6]`, doc Gap 5) is still unresolved.
/// `JC2_NO_FALLBACK_FIREBALL=1` hides it for captures. Set to `false` (or delete
/// `Fireball`, `fireballs` and the spawn in `detonate`) once the native effect is as visible as the
/// original.
const FALLBACK_FIREBALL: bool = true;
const FIREBALL_SECONDS: f32 = 0.7;

#[derive(Clone, Copy, Debug)]
pub struct Blast {
    pub at: Vec3,
    pub radius: f32,
    pub damage: f32,
    pub impulse: f32,
    /// The native effect id this blast plays (`research/formats/explosion-effects.md` section 4);
    /// `None` plays no effect (the damage and push still happen).
    pub effect: Option<u32>,
}

/// Native effect ids per blast source (explosion-effects.md section 4, proven from the weapon
/// records / impact ids): hand grenade and grenade launcher 35 `grenade_explosion`, cluster bomb
/// 411, rocket launcher 5 `missile_explosion`, small missile 168, cannon impact 377.
#[allow(dead_code)] // ids for the cluster bomb, small missile and cannon sources (not yet in the game)
pub mod effect_id {
    pub const GRENADE: u32 = 35;
    pub const CLUSTER_GRENADE: u32 = 411;
    pub const MISSILE: u32 = 5;
    pub const MISSILE_SMALL: u32 = 168;
    pub const CANNON_IMPACT: u32 = 377;
    /// `explosion_lave`, the class effect of the 37 `CCar` vehicles (`vehicle-fire-explosion.md`
    /// section 3.1). Vehicles play the effect of their own doll slot 1 (`vehicle_death`); this id is
    /// only used by the effect tests.
    #[allow(dead_code)]
    pub const VEHICLE_CAR: u32 = 128;
    /// `explosion_barrel` (placed `CEffectPointEmitter` id of the red barrel, census in the doc).
    pub const BARREL: u32 = 27;
}

/// Blasts to set off this frame.
#[derive(Resource, Default)]
pub struct Blasts {
    pub queue: Vec<Blast>,
    /// How many have gone off, for the tests.
    pub count: u32,
    /// The ones that went off this frame (for the systems after `detonate`).
    pub fired: Vec<Blast>,
}

/// A rocket or grenade-launcher round: flies straight (or falls), goes off on contact.
#[derive(Component)]
pub struct Projectile {
    pub velocity: Vec3,
    pub gravity: f32,
    pub life: f32,
    pub blast: Blast,
    pub owner: Entity,
}

/// A thrown grenade: a small dynamic body that bounces until its fuse runs out.
#[derive(Component)]
pub struct Grenade {
    pub fuse: f32,
    pub blast: Blast,
}

/// Burnt out: the death routine ran (native +0x35c). The wreck's smoke and fire are the slot 13 /
/// 15 effect controllers of `vehicle_damage::VehicleFx` (FX-VEH-1), no longer sphere puffs.
#[derive(Component)]
pub struct Wreck;

/// The non-native fallback fireball (see [`FALLBACK_FIREBALL`]).
#[derive(Component)]
struct Fireball {
    age: f32,
    radius: f32,
    material: Handle<StandardMaterial>,
}

#[derive(Resource)]
pub struct Fx {
    pub sphere: Handle<Mesh>,
    pub rocket: Handle<Mesh>,
    pub rocket_material: Handle<StandardMaterial>,
    charred: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
    /// The same for the hemisphere-ambient route of the vehicle parts (JC2-177).
    charred_hemisphere: HashMap<
        AssetId<crate::hemisphere_material::HemisphereMaterial>,
        Handle<crate::hemisphere_material::HemisphereMaterial>,
    >,
}

pub struct ExplosionPlugin;

impl Plugin for ExplosionPlugin {
    fn build(&self, app: &mut App) {
        crate::vehicle_death::plugin(app);
        app.init_resource::<Blasts>()
            .add_systems(Startup, setup_fx)
            .add_systems(
                Update,
                (fly, fuses, detonate, crate::vehicle_death::vehicle_death, wreck_vehicles, fireballs).chain(),
            );
        // opt-in capture fixture: `JC2_BLAST_FIXTURE=<delay seconds>` sets off one hand-grenade
        // blast `JC2_BLAST_FIXTURE_DIST` (default 18) metres in front of Rico (else the first camera)
        if let Some(delay) = std::env::var("JC2_BLAST_FIXTURE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            app.add_systems(Update, move |time: Res<Time>,
                                          heroes: Query<&GlobalTransform, With<Hero>>,
                                          cams: Query<&GlobalTransform, With<Camera3d>>,
                                          mut blasts: ResMut<Blasts>,
                                          mut done: Local<bool>| {
                if *done || time.elapsed_secs() < delay {
                    return;
                }
                // in front of Rico (the gameplay camera follows him), else of the first camera
                // (`JC2_BLAST_FIXTURE_CAM=1`: in front of the first camera instead, so a capture
                // is independent of the way Rico happens to face)
                let by_camera = std::env::var_os("JC2_BLAST_FIXTURE_CAM").is_some();
                let Some(c) = (if by_camera {
                    cams.iter().next()
                } else {
                    heroes.iter().next().or_else(|| cams.iter().next())
                }) else {
                    return;
                };
                *done = true;
                let dist = std::env::var("JC2_BLAST_FIXTURE_DIST")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(18.0);
                info!("blast fixture: grenade effect {} {dist} m in front of the hero", effect_id::GRENADE);
                // `JC2_BLAST_FIXTURE_CAM=ground`: at Rico's height (+1 m), along the camera's heading
                let at = match (std::env::var("JC2_BLAST_FIXTURE_CAM").as_deref(), heroes.iter().next()) {
                    (Ok("ground"), Some(h)) => {
                        let f = c.forward().as_vec3();
                        let flat = Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z);
                        h.translation() + flat * dist + Vec3::Y
                    }
                    _ => c.translation() + c.forward().as_vec3() * dist + Vec3::Y * 3.0,
                };
                info!("blast fixture: blast at {at:?}, source {:?}", c.translation());
                blasts.queue.push(Blast {
                    at,
                    radius: 5.0,
                    damage: 0.0,
                    impulse: 0.0,
                    effect: Some(effect_id::GRENADE),
                });
            });
            // Capture aid (aee5ba94): `JC2_BLAST_FIXTURE_SHOT=<age>` saves a screenshot to
            // `JC2_BLAST_FIXTURE_SHOT_PATH` when the blast is `<age>` game seconds old, then quits.
            // `JC2_BLAST_FIXTURE_SLOW=<k>` slows the game clock to `k` of real time from the blast on,
            // so a slow machine (250 ms frames) still shows exact ages.
            if let (Some(age), Ok(path)) = (
                std::env::var("JC2_BLAST_FIXTURE_SHOT")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok()),
                std::env::var("JC2_BLAST_FIXTURE_SHOT_PATH"),
            ) {
                let slow = std::env::var("JC2_BLAST_FIXTURE_SLOW")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(1.0);
                app.add_systems(
                    Update,
                    move |time: Res<Time>,
                          real: Res<Time<Real>>,
                          mut virt: ResMut<Time<Virtual>>,
                          blasts: Res<Blasts>,
                          mut commands: Commands,
                          mut exit: MessageWriter<AppExit>,
                          mut blast_at: Local<Option<f32>>,
                          mut requested: Local<Option<f32>>| {
                        // blasts of the world itself (burning wrecks) also count: wait for the fixture
                        if blasts.count == 0 || time.elapsed_secs() < delay + 0.5 {
                            return;
                        }
                        let at = *blast_at.get_or_insert_with(|| {
                            virt.set_relative_speed(slow);
                            time.elapsed_secs()
                        });
                        if requested.is_none() && time.elapsed_secs() - at >= age {
                            info!("blast fixture: screenshot at age {:.3} s", time.elapsed_secs() - at);
                            commands
                                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                                .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
                            *requested = Some(real.elapsed_secs());
                        }
                        if requested.is_some_and(|r| real.elapsed_secs() - r > 4.0) {
                            exit.write(AppExit::Success);
                        }
                    },
                );
            }
        }
    }
}

fn setup_fx(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(Fx {
        sphere: meshes.add(Sphere::new(1.0).mesh().ico(3).unwrap()),
        rocket: meshes.add(Capsule3d::new(0.06, 0.5)),
        rocket_material: materials.add(StandardMaterial {
            // unlit: `base_color` is what shows (native scene unit from a display value, policy)
            base_color: crate::native_units::native_colour(Vec3::new(1.0, 0.7, 0.35), 1.0),
            unlit: true,
            ..default()
        }),
        charred: HashMap::new(),
        charred_hemisphere: HashMap::new(),
    });
}

/// Rockets: step along the velocity, casting the segment; go off on whatever they meet.
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut blasts: ResMut<Blasts>,
    mut q: Query<(Entity, &mut Projectile, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut p, mut tf) in &mut q {
        p.life -= dt;
        p.velocity.y -= p.gravity * dt;
        let step = p.velocity * dt;
        let len = step.length();
        let filter = SpatialQueryFilter::from_excluded_entities([p.owner]);
        let hit = Dir3::new(step)
            .ok()
            .and_then(|d| spatial.cast_ray(tf.translation, d, len, true, &filter));
        if let Some(h) = hit {
            let at = tf.translation + step.normalize() * h.distance;
            blasts.queue.push(Blast { at, ..p.blast });
            commands.entity(e).despawn();
            continue;
        }
        if p.life <= 0.0 {
            blasts.queue.push(Blast {
                at: tf.translation,
                ..p.blast
            });
            commands.entity(e).despawn();
            continue;
        }
        tf.translation += step;
        if let Ok(d) = Dir3::new(p.velocity) {
            tf.rotation = Quat::from_rotation_arc(Vec3::Y, d.as_vec3());
        }
    }
}

fn fuses(
    mut commands: Commands,
    time: Res<Time>,
    mut blasts: ResMut<Blasts>,
    mut q: Query<(Entity, &mut Grenade, &Transform)>,
) {
    for (e, mut g, tf) in &mut q {
        g.fuse -= time.delta_secs();
        if g.fuse <= 0.0 {
            blasts.queue.push(Blast {
                at: tf.translation,
                ..g.blast
            });
            commands.entity(e).despawn();
        }
    }
}

/// How much of a blast reaches `d` metres from its centre.
pub fn falloff(d: f32, radius: f32) -> f32 {
    (1.0 - d / radius).clamp(0.0, 1.0)
}

/// The resolved rigid body of whatever a ray or a blast touched: a vehicle's extra
/// chassis colliders are children of its body.
pub fn body_of(e: Entity, colliders: &Query<&ColliderOf>) -> Entity {
    colliders.get(e).map_or(e, |c| c.body)
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn detonate(
    mut commands: Commands,
    mut blasts: ResMut<Blasts>,
    fx: Option<Res<Fx>>,
    lights: Option<Res<crate::point_lights::NativeLightEffects>>,
    effects: Option<ResMut<crate::effects::EffectSystem>>,
    host: Option<Res<crate::effects::EffectHost>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    ground: Option<ResMut<crate::audio::Ground>>,
    game: Option<Res<crate::Game>>,
    mut healths: Query<(&GlobalTransform, &mut Health, Has<Container>), Without<Dead>>,
    mut bodies: Query<(&RigidBody, &GlobalTransform, Forces)>,
) {
    blasts.fired.clear();
    let Some(fx) = fx else {
        blasts.queue.clear();
        return;
    };
    let mut effects = effects;
    let mut ground = ground;
    for b in std::mem::take(&mut blasts.queue) {
        blasts.count += 1;
        blasts.fired.push(b);
        for (tf, mut h, vehicle) in &mut healths {
            // vehicles are big: measure to roughly their skin
            let d = (tf.translation().distance(b.at) - if vehicle { 1.5 } else { 0.0 }).max(0.0);
            let k = falloff(d, b.radius);
            if k > 0.0 {
                h.hp -= b.damage * k;
            }
        }
        for (body, tf, mut f) in &mut bodies {
            if !body.is_dynamic() {
                continue;
            }
            let to = tf.translation() - b.at;
            let d = to.length();
            let k = falloff(d, b.radius * 1.5);
            if k <= 0.0 {
                continue;
            }
            let dir = (to.normalize_or(Vec3::Y) + Vec3::Y * 0.6).normalize();
            f.apply_linear_impulse(dir * b.impulse * IMPULSE_SCALE * k);
            f.apply_angular_impulse(dir.cross(Vec3::Y) * b.impulse * k * 0.5);
        }
        // Explosive detonation pulse (0x005569a0 / 0x007579f0: a CForcePulse with extents (R,R,R)
        // applied at the blast, so it shakes the camera by the 3 x R rule). Only for the explosive
        // sources whose effect graph has no pulse component of its own. The A/B/C mapping from the
        // explosive record is an open gap (explosion-effects.md Gap 2): A uses the record impulse,
        // NON-NATIVE; A only feeds the rumble term, the push stays with the tuned code above.
        let explosive = matches!(
            b.effect,
            Some(
                effect_id::GRENADE
                    | effect_id::CLUSTER_GRENADE
                    | effect_id::MISSILE
                    | effect_id::MISSILE_SMALL
            )
        );
        if let (Some(host), true) = (host.as_ref(), explosive) {
            host.pulses.queue_pulse(
                b.at,
                crate::effects::pulse::PulseParams { a: b.impulse, c: 0.0, extents: Vec3::splat(b.radius) },
            );
        }
        if FALLBACK_FIREBALL && std::env::var_os("JC2_NO_FALLBACK_FIREBALL").is_none() {
            let material = materials.add(StandardMaterial {
                base_color: crate::native_units::native_colour(Vec3::new(1.0, 0.55, 0.15), 0.9),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            });
            commands.spawn((
                Mesh3d(fx.sphere.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(b.at).with_scale(Vec3::splat(0.3)),
                Fireball {
                    age: 0.0,
                    radius: b.radius * 0.8,
                    material,
                },
                Name::new("explosion (non-native fallback fireball)"),
            ));
        }
        // The native effect (particles, light; decal, sound, pulse and shake are the effect
        // system's stubs for now): surface selection as the bullet impacts, water flag below sea
        // level (explosion-effects.md section 3 note a; the (flag, material) pair is
        // weapon-fire-effects.md section 3).
        if let Some(id) = b.effect {
            if let Some(effects) = effects.as_mut() {
                let interaction = if b.at.y < crate::world::SEA_LEVEL {
                    Some(crate::effects::Interaction::WATER)
                } else {
                    match (ground.as_mut(), game.as_ref()) {
                        (Some(g), Some(game)) => g
                            .surface_at(&game.fs, b.at)
                            .map(|s| crate::effects::Interaction::surface(s.material)),
                        _ => None,
                    }
                };
                let mut req = crate::effects::PlayRequest::new(id).at_point(b.at);
                if let Some(i) = interaction {
                    req = req.interaction(i);
                }
                let h = effects.play(req);
                debug!("blast effect {id} at {:?} -> {h:?}", b.at);
            } else if let Some(lights) = &lights {
                // no effect system (registry unreadable): the keyed light alone, as before
                lights.spawn(&mut commands, crate::point_lights::BLAST_EFFECT, b.at);
            }
        }
    }
}

fn fireballs(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &mut Fireball, &mut Transform)>,
) {
    for (e, mut f, mut tf) in &mut q {
        f.age += time.delta_secs();
        let t = (f.age / FIREBALL_SECONDS).min(1.0);
        tf.scale = Vec3::splat(f.radius * (0.3 + 0.7 * (1.0 - (1.0 - t).powi(3))));
        if let Some(mut m) = materials.get_mut(&f.material) {
            m.base_color.set_alpha(0.9 * (1.0 - t));
        }
        if t >= 1.0 {
            commands.entity(e).despawn();
        }
    }
}

/// Vehicles at zero health blow up: the blast, out goes the driver, the paint chars.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn wreck_vehicles(
    mut commands: Commands,
    mut blasts: ResMut<Blasts>,
    fx: Option<ResMut<Fx>>,
    host: Option<Res<crate::effects::EffectHost>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut hemisphere_materials: Option<
        ResMut<Assets<crate::hemisphere_material::HemisphereMaterial>>,
    >,
    mut mode: ResMut<CameraMode>,
    vehicles: Query<
        (Entity, &Health, &GlobalTransform, &Transform),
        (With<Container>, Without<Wreck>),
    >,
    children: Query<&Children>,
    mut meshes: Query<&mut MeshMaterial3d<StandardMaterial>>,
    mut hemisphere_meshes: Query<
        &mut MeshMaterial3d<crate::hemisphere_material::HemisphereMaterial>,
    >,
    mut helis: Query<&mut Heli>,
    mut heroes: Query<
        (
            Entity,
            &mut Hero,
            &mut Transform,
            &mut LinearVelocity,
            &mut Visibility,
        ),
        Without<Container>,
    >,
) {
    let Some(mut fx) = fx else { return };
    for (e, h, gtf, vtf) in &vehicles {
        if h.hp > 0.0 {
            continue;
        }
        info!("vehicle {e} burns out");
        // Vehicle death (CVehicle slot 78, 0x00559150): the code-built pulse (7 / 300 / 500) and
        // the three-key flash. The flash radius is a labelled non-native constant (Gap 4).
        if let Some(host) = host.as_ref() {
            host.pulses
                .queue_pulse(gtf.translation(), crate::effects::pulse::VEHICLE_DEATH_PULSE);
        }
        commands.spawn((
            Transform::from_translation(gtf.translation()),
            GlobalTransform::from_translation(gtf.translation()),
            crate::point_lights::EffectLight {
                track: std::sync::Arc::new(crate::effects::pulse::vehicle_death_flash()),
                clock: 0.0,
            },
            Name::new("vehicle death flash"),
        ));
        blasts.queue.push(Blast {
            at: gtf.translation(),
            ..WRECK_BLAST
        });
        commands
            .entity(e)
            .insert(Wreck)
            .remove::<(Enterable, Occupied)>();
        if let Ok(mut heli) = helis.get_mut(e) {
            heli.spool = 0.0;
        }
        // whoever was in it is thrown out
        for (he, mut hero, mut tf, mut lin, mut vis) in &mut heroes {
            if hero.in_vehicle != Some(e) {
                continue;
            }
            let p = vtf.translation + vtf.rotation * Vec3::new(-2.5, 1.5, 0.0);
            tf.translation = p;
            lin.0 = Vec3::new(0.0, 6.0, 0.0);
            crate::physics::release_passenger(&mut commands, he, p, lin.0);
            *vis = Visibility::Inherited;
            hero.in_vehicle = None;
            hero.grounded = false;
            *mode = CameraMode::Hero;
        }
        // char the paint: every part's material, set to the native wreck paint grey (non-native
        // single-colour mapping, see `vehicle_death::charred_colour`), shared between wrecks
        for c in children.iter_descendants(e) {
            if let (Ok(mut m), Some(hemisphere_materials)) =
                (hemisphere_meshes.get_mut(c), hemisphere_materials.as_mut())
            {
                let id = m.0.id();
                let charred = match fx.charred_hemisphere.get(&id) {
                    Some(h) => h.clone(),
                    None => {
                        let Some(src) = hemisphere_materials.get(id) else {
                            continue;
                        };
                        let mut dark = src.clone();
                        dark.base.base_color = crate::vehicle_death::charred_colour();
                        dark.base.perceptual_roughness = 0.95;
                        dark.base.metallic = 0.0;
                        let h = hemisphere_materials.add(dark);
                        fx.charred_hemisphere.insert(id, h.clone());
                        h
                    }
                };
                m.0 = charred;
                continue;
            }
            let Ok(mut m) = meshes.get_mut(c) else {
                continue;
            };
            let id = m.0.id();
            let charred = match fx.charred.get(&id) {
                Some(h) => h.clone(),
                None => {
                    let Some(src) = materials.get(id) else {
                        continue;
                    };
                    let mut dark = src.clone();
                    dark.base_color = crate::vehicle_death::charred_colour();
                    dark.perceptual_roughness = 0.95;
                    dark.metallic = 0.0;
                    let h = materials.add(dark);
                    fx.charred.insert(id, h.clone());
                    h
                }
            };
            m.0 = charred;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blast_falls_off() {
        assert_eq!(falloff(0.0, 5.0), 1.0);
        assert_eq!(falloff(2.5, 5.0), 0.5);
        assert_eq!(falloff(6.0, 5.0), 0.0);
    }

    /// The retail explosion effects start particles through the real adapters for every source
    /// id, with the soil and the water selection (explosion-effects.md section 3).
    #[test]
    fn retail_blast_effects_spawn_particles_and_lights() {
        use crate::effects::{
            EffectHost, EffectSystem, Interaction, PlayRequest, default_adapters,
            particle::ParticleManager,
        };
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = std::sync::Arc::new(jc2_formats::GameFs::open(dir).unwrap());
        let host = EffectHost::default();
        *host.particles.lock().unwrap() = ParticleManager::from_game(&fs);
        let mut fx = EffectSystem::from_retail(fs, default_adapters(&host)).unwrap();
        for id in [
            effect_id::GRENADE,
            effect_id::MISSILE,
            effect_id::VEHICLE_CAR,
            effect_id::BARREL,
        ] {
            for i in [Interaction::surface(1), Interaction::WATER] {
                let h = fx
                    .play(
                        PlayRequest::new(id)
                            .at_point(Vec3::new(1.0, 2.0, 3.0))
                            .interaction(i),
                    )
                    .unwrap_or_else(|| panic!("effect {id} refused"));
                fx.tick(1.0 / 60.0);
                let comps = fx.components(&h);
                assert!(
                    comps.iter().any(|c| c.kind == 1 && c.adapter),
                    "{id}: {comps:?}"
                );
            }
        }
    }
}
