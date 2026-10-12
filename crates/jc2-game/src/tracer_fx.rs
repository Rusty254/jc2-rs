//! Bullet tracers (board task aa9947ad, FX-GUN-3; research `research/formats/weapon-fire-effects.md`
//! sections 2 and 6).
//!
//! Native: the bullet factory 0x004f9560 creates a `CBulletTrail` (constructor 0x0056f420, vtable
//! 0x00eba7bc) next to the `CBullet` for every shot on which the weapon's period `#04f3f8c6`
//! (type +0x19c) divides the shot counter. The trail is its own moving object: it takes the bullet's
//! aim matrix, the speed `tracer_fire_velocity` (type +0x1a0, written to trail +0xa8) and runs until
//! its range is used up (update 0x005d8c50). The bullet manager preloads `bullet_trail_new_dif.dds`
//! and `bullet_tracer_dif.dds` (0x00657600).
//!
//! What is native here: a trail only on weapons whose period is non-zero, its speed
//! (`tracer_fire_velocity`, so the tracer visibly outruns nothing: 100 m/s for the Beretta against
//! the bullet's 750), the start at the muzzle along the shot and the `bullet_trail_new_dif.dds`
//! sprite (a glowing head with an orange streak behind it, 128 x 512).
//!
//! What is NOT native (inferred, research Gap 4): the draw routine of the trail was not found, so the
//! quad size, the head position inside the texture, the crossed-quad billboard and additive blend are
//! the port's choice. `tracer_scale_factor` and the other loaded tracer fields (`#57d5fa61`,
//! `#6837be43`, `#a5d17f96`) have no decoded consumer and are not used. The trail's speed loss
//! (0x005d8c50 subtracts a global from the speed below altitude 200) is not applied: the global's
//! meaning is unresolved.

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// `textures\bullet_trail_new_dif.dds` (native 0x00657600 preloads it): DXT5, 128 x 512.
const TRAIL_TEXTURE: &str = "jc2://textures/bullet_trail_new_dif.dds";
/// Length and width of the trail quad (m). Non-native (inferred): chosen to keep the texture's
/// 1 : 4 aspect.
const TRAIL_LENGTH: f32 = 2.4;
const TRAIL_WIDTH: f32 = 0.6;
/// Where the glowing head sits along the texture (v of the head, measured on the DDS). Non-native.
const HEAD_V: f32 = 0.12;

/// The shared mesh and material of every tracer.
#[derive(Clone)]
pub struct TracerAssets {
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

impl TracerAssets {
    pub fn new(
        assets: &AssetServer,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Self {
        Self {
            mesh: meshes.add(trail_mesh()),
            material: materials.add(StandardMaterial {
                base_color_texture: Some(assets.load(TRAIL_TEXTURE)),
                base_color: Color::WHITE,
                unlit: true,
                alpha_mode: AlphaMode::Add,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
        }
    }
}

/// Two crossed quads along local -Z with the head at the origin (so the trail needs no camera
/// facing). The head is at uv `HEAD_V`; the streak runs to +Z behind it.
fn trail_mesh() -> Mesh {
    let (z_head, z_tail) = (-HEAD_V * TRAIL_LENGTH, (1.0 - HEAD_V) * TRAIL_LENGTH);
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    let mut idx = Vec::new();
    for k in 0..2 {
        let side = if k == 0 { Vec3::X } else { Vec3::Y } * TRAIL_WIDTH * 0.5;
        let base = pos.len() as u32;
        // z_head is the front (travel direction, -Z), v = 0 at the texture top
        pos.extend([
            (-side + Vec3::Z * z_head).to_array(),
            (side + Vec3::Z * z_head).to_array(),
            (side + Vec3::Z * z_tail).to_array(),
            (-side + Vec3::Z * z_tail).to_array(),
        ]);
        uv.extend([[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let n = pos.len();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; n])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U32(idx))
}

/// A flying tracer: it moves along `direction` at `speed` (m/s).
#[derive(Component)]
pub struct TracerFlight {
    direction: Vec3,
    speed: f32,
}

/// The components of a tracer from `from` to `to` at `speed`, and its flight time (s).
pub fn tracer_bundle(
    a: &TracerAssets,
    from: Vec3,
    to: Vec3,
    speed: f32,
) -> Option<(impl Bundle, f32)> {
    let span = to - from;
    let distance = span.length();
    if speed <= 0.0 || distance < 1e-3 {
        return None;
    }
    let direction = span / distance;
    Some((
        (
            Mesh3d(a.mesh.clone()),
            MeshMaterial3d(a.material.clone()),
            Transform::from_translation(from)
                .with_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, direction)),
            TracerFlight { direction, speed },
            NotShadowCaster,
            Name::new("tracer"),
        ),
        distance / speed,
    ))
}

/// Move every tracer along its shot.
pub fn fly_tracers(time: Res<Time>, mut q: Query<(&mut Transform, &TracerFlight)>) {
    for (mut t, f) in &mut q {
        t.translation += f.direction * f.speed * time.delta_secs();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flight_time_is_distance_over_speed() {
        let (mut meshes, mut materials) = (Assets::<Mesh>::default(), Assets::default());
        let assets = TracerAssets {
            mesh: meshes.add(trail_mesh()),
            material: materials.add(StandardMaterial::default()),
        };
        let (_, t) = tracer_bundle(&assets, Vec3::ZERO, Vec3::new(0.0, 0.0, -50.0), 100.0).unwrap();
        assert!((t - 0.5).abs() < 1e-6);
        assert!(tracer_bundle(&assets, Vec3::ZERO, Vec3::ONE, 0.0).is_none());
        assert!(tracer_bundle(&assets, Vec3::ONE, Vec3::ONE, 100.0).is_none());
    }

    #[test]
    fn mesh_has_two_quads() {
        let m = trail_mesh();
        assert_eq!(m.count_vertices(), 8);
    }
}
