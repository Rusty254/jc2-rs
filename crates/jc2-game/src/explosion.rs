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

/// A vehicle burning out.
const WRECK_BLAST: Blast = Blast {
    at: Vec3::ZERO,
    radius: 9.0,
    damage: 450.0,
    impulse: 1500.0,
};
/// The game's impulses are in its own units; this turns them into a push that throws a
/// car (tuned, not from the data).
const IMPULSE_SCALE: f32 = 8.0;
const FIREBALL_SECONDS: f32 = 0.7;
const SMOKE_SECONDS: f32 = 25.0;
const PUFF_EVERY: f32 = 0.2;
const PUFF_LIFE: f32 = 3.0;

#[derive(Clone, Copy, Debug)]
pub struct Blast {
    pub at: Vec3,
    pub radius: f32,
    pub damage: f32,
    pub impulse: f32,
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

/// Burnt out: seconds since.
#[derive(Component)]
pub struct Wreck(pub f32, f32);

#[derive(Component)]
struct Fireball {
    age: f32,
    radius: f32,
    material: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Puff {
    age: f32,
    rise: f32,
}

#[derive(Resource)]
pub struct Fx {
    pub sphere: Handle<Mesh>,
    pub rocket: Handle<Mesh>,
    pub rocket_material: Handle<StandardMaterial>,
    smoke: Handle<StandardMaterial>,
    charred: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
}

pub struct ExplosionPlugin;

impl Plugin for ExplosionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Blasts>()
            .add_systems(Startup, setup_fx)
            .add_systems(
                Update,
                (fly, fuses, detonate, wreck_vehicles, burn, fireballs, puffs).chain(),
            );
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
        smoke: materials.add(StandardMaterial {
            base_color: crate::native_units::native_colour(Vec3::new(0.08, 0.07, 0.07), 0.45),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        }),
        charred: HashMap::new(),
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

#[allow(clippy::type_complexity)]
pub(crate) fn detonate(
    mut commands: Commands,
    mut blasts: ResMut<Blasts>,
    fx: Option<Res<Fx>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut healths: Query<(&GlobalTransform, &mut Health, Has<Container>), Without<Dead>>,
    mut bodies: Query<(&RigidBody, &GlobalTransform, Forces)>,
) {
    blasts.fired.clear();
    let Some(fx) = fx else {
        blasts.queue.clear();
        return;
    };
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
        // the fireball and its flash
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
            PointLight {
                color: Color::srgb(1.0, 0.6, 0.25),
                intensity: explosion_lumens(),
                range: b.radius * 8.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Name::new("explosion"),
        ));
    }
}

/// Explosion light strength in native colour units at 1 m (`native_units::point_lumens`).
/// **Policy value** (the earlier 4.0e7 lm tuning is 1,015 units; the exe caps a point-light
/// colour channel at 8 with its own radius fall-off, which is not decoded, so this
/// inverse-square stand-in is not a native intensity).
const EXPLOSION_NATIVE: f32 = 1_000.0;

fn explosion_lumens() -> f32 {
    crate::native_units::point_lumens(EXPLOSION_NATIVE)
}

fn fireballs(
    mut commands: Commands,
    time: Res<Time>,
    atmosphere: Option<Res<crate::atmosphere::AtmosphereState>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<(Entity, &mut Fireball, &mut Transform, &mut PointLight)>,
) {
    for (e, mut f, mut tf, mut light) in &mut q {
        f.age += time.delta_secs();
        let t = (f.age / FIREBALL_SECONDS).min(1.0);
        tf.scale = Vec3::splat(f.radius * (0.3 + 0.7 * (1.0 - (1.0 - t).powi(3))));
        // native point-light colour scale S (1.0 at night, 0.25 by day); the unknown
        // CRenderEngine+0x830 factor is taken as 1 (atmosphere.rs)
        let scale = atmosphere.as_ref().map_or(1.0, |a| a.point_light_factor());
        light.intensity = explosion_lumens() * (1.0 - t).powi(2) * scale;
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
fn wreck_vehicles(
    mut commands: Commands,
    mut blasts: ResMut<Blasts>,
    fx: Option<ResMut<Fx>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut mode: ResMut<CameraMode>,
    vehicles: Query<
        (Entity, &Health, &GlobalTransform, &Transform),
        (With<Container>, Without<Wreck>),
    >,
    children: Query<&Children>,
    mut meshes: Query<&mut MeshMaterial3d<StandardMaterial>>,
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
        blasts.queue.push(Blast {
            at: gtf.translation(),
            ..WRECK_BLAST
        });
        commands
            .entity(e)
            .insert(Wreck(0.0, 0.0))
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
        // char the paint: every part's material, darkened, shared between wrecks
        for c in children.iter_descendants(e) {
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
                    dark.base_color = Color::srgb(0.09, 0.08, 0.075);
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

/// Wrecks smoke for a while.
fn burn(
    mut commands: Commands,
    time: Res<Time>,
    fx: Option<Res<Fx>>,
    mut q: Query<(&mut Wreck, &GlobalTransform)>,
) {
    let Some(fx) = fx else { return };
    let dt = time.delta_secs();
    for (mut w, tf) in &mut q {
        w.0 += dt;
        if w.0 > SMOKE_SECONDS {
            continue;
        }
        w.1 -= dt;
        if w.1 > 0.0 {
            continue;
        }
        w.1 = PUFF_EVERY;
        let jitter = Vec3::new((w.0 * 7.3).sin(), 0.0, (w.0 * 5.1).cos()) * 0.6;
        commands.spawn((
            Mesh3d(fx.sphere.clone()),
            MeshMaterial3d(fx.smoke.clone()),
            Transform::from_translation(tf.translation() + Vec3::Y * 1.2 + jitter)
                .with_scale(Vec3::splat(0.6)),
            Puff {
                age: 0.0,
                rise: 2.0 + (w.0 * 3.7).sin().abs(),
            },
        ));
    }
}

fn puffs(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Puff, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut p, mut tf) in &mut q {
        p.age += dt;
        tf.translation.y += p.rise * dt;
        tf.scale = Vec3::splat(0.6 + p.age * 0.9);
        if p.age > PUFF_LIFE {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    /// The explosion light keeps its tuned strength (4.0e7 lm) in native colour units.
    #[test]
    fn the_explosion_light_keeps_its_tuned_strength_in_native_units() {
        let lm = explosion_lumens();
        assert!((lm / 4.0e7 - 1.0).abs() < 0.03, "{lm}");
    }

    use super::*;

    #[test]
    fn blast_falls_off() {
        assert_eq!(falloff(0.0, 5.0), 1.0);
        assert_eq!(falloff(2.5, 5.0), 0.5);
        assert_eq!(falloff(6.0, 5.0), 0.0);
    }
}
