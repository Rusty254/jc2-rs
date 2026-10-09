//! The canopy, built from the game's parachute skeleton. `parachute.bsk` has no mesh:
//! its three chains of bones (`lft_1..4`, `mid_1..4`, `rgt_1..4`) run from the harness
//! up to the canopy's left edge, centre line and right edge, and `chute_open_chute.ban`
//! unfolds them. The surface is lofted across the chain tips here (an arch extruded
//! front to back) and the lines are drawn along the chains — the shape the game's
//! cloth sits on, without the cloth. See `research/formats/character.md`.

use crate::coords::qs_to_bevy;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use jc2_formats::GameFs;
use jc2_formats::animation::{QsTransform, Skeleton, SplineAnimation};

/// Front-to-back depth of the canopy (not in the skeleton; a ram-air chute of this
/// 3.9 m span would be about this deep).
const DEPTH: f32 = 2.6;
const ARCH_SEGMENTS: usize = 16;
const DEPTH_SEGMENTS: usize = 6;
const LINE_WIDTH: f32 = 0.012;
/// The fabric: near-black charcoal, matched by eye to screenshots of the original
/// (`reference/chute/`; nothing in the data gives the colour). The real canopy also has
/// faint blue-grey stripes along its 9–10 cells.
const CANOPY_COLOUR: Color = Color::srgb(0.06, 0.065, 0.075);
/// The lines are drawn twice, this far in front of and behind the bones' plane.
const LINE_SPREAD: f32 = 0.9;

/// The parachute skeleton and its opening clip.
#[derive(Resource)]
pub struct ChuteRig {
    pub skeleton: Skeleton,
    pub open: Option<SplineAnimation>,
    /// Track → bone of the opening clip.
    open_bones: Vec<Option<usize>>,
    chains: [[usize; 4]; 3],
    base: usize,
}

impl ChuteRig {
    pub fn load(fs: &GameFs) -> Option<Self> {
        let bytes = fs.read_inflated("parachute.bsk").ok()?;
        let skeleton = Skeleton::parse(&bytes).ok()?;
        let bone = |n: &str| skeleton.bone_index(n);
        let chain = |p: &str| -> Option<[usize; 4]> {
            Some([
                bone(&format!("{p}_1"))?,
                bone(&format!("{p}_2"))?,
                bone(&format!("{p}_3"))?,
                bone(&format!("{p}_4"))?,
            ])
        };
        let chains = [chain("lft")?, chain("mid")?, chain("rgt")?];
        let base = bone("base")?;
        let open = crate::rig::read_animation_file(fs, "chute_open_chute.ban")
            .and_then(|b| SplineAnimation::parse(&b).ok());
        let open_bones = open
            .as_ref()
            .map(|a| a.track_names.iter().map(|n| bone(n)).collect())
            .unwrap_or_default();
        info!(
            "parachute: {} bones, opening clip {}",
            skeleton.bones.len(),
            if open.is_some() { "yes" } else { "no" }
        );
        Some(Self {
            skeleton,
            open,
            open_bones,
            chains,
            base,
        })
    }

    /// Bone positions in the canopy's frame (Bevy axes) for the pose at `time` into the
    /// opening clip (`None` = the reference pose).
    fn bone_positions(&self, time: Option<f32>, scratch: &mut Vec<QsTransform>) -> Vec<Vec3> {
        let n = self.skeleton.bones.len();
        let mut local: Vec<Transform> = self
            .skeleton
            .reference_pose
            .iter()
            .map(qs_to_bevy)
            .collect();
        if let (Some(t), Some(open)) = (time, &self.open) {
            open.sample(t, scratch);
            for (ti, qs) in scratch.iter().enumerate() {
                if let Some(Some(b)) = self.open_bones.get(ti) {
                    local[*b] = qs_to_bevy(qs);
                }
            }
        }
        let mut global: Vec<Transform> = Vec::with_capacity(n);
        for (i, bone) in self.skeleton.bones.iter().enumerate() {
            let g = match bone.parent {
                Some(p) if p < i => global[p] * local[i],
                _ => local[i],
            };
            global.push(g);
        }
        global.iter().map(|g| g.translation).collect()
    }
}

/// The canopy entity's working state.
#[derive(Component)]
pub struct CanopyRig {
    mesh: Handle<Mesh>,
    /// Line entities: (entity, from bone, to bone, z offset).
    lines: Vec<(Entity, usize, usize, f32)>,
    /// Seconds into the opening clip; `None` when closed.
    opening: Option<f32>,
    scratch: Vec<QsTransform>,
}

/// An empty, updatable canopy mesh.
fn canopy_mesh() -> Mesh {
    let (n, m) = (ARCH_SEGMENTS, DEPTH_SEGMENTS);
    let count = (n + 1) * (m + 1);
    let mut uv = Vec::with_capacity(count);
    for j in 0..=m {
        for i in 0..=n {
            uv.push([i as f32 / n as f32, j as f32 / m as f32]);
        }
    }
    let mut idx = Vec::new();
    for j in 0..m {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u32;
            let b = a + 1;
            let c = a + (n + 1) as u32;
            let d = c + 1;
            idx.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; count]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; count]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// Quadratic curve through three points (`t` = 0 at `a`, 1 at `c`, passes `b` at ½).
fn arch(a: Vec3, b: Vec3, c: Vec3, t: f32) -> Vec3 {
    // Lagrange through (0, a), (0.5, b), (1, c)
    let l0 = 2.0 * (t - 0.5) * (t - 1.0);
    let l1 = -4.0 * t * (t - 1.0);
    let l2 = 2.0 * t * (t - 0.5);
    a * l0 + b * l1 + c * l2
}

/// Spawn the canopy (hidden) under `parent`, at the model's origin.
pub fn spawn_canopy(
    commands: &mut Commands,
    parent: Entity,
    rig: &ChuteRig,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    origin: Vec3,
) -> Entity {
    let mesh = meshes.add(canopy_mesh());
    // Plain fabric: the game's canopy model and texture (`mc01_parachute_dif`, listed by
    // the doll) are not in the data, and `gae06_parachute_dif` is the atlas of the
    // 15 cm parachute pack, not a canopy (research/formats/character.md).
    let canopy_material = materials.add(StandardMaterial {
        base_color: CANOPY_COLOUR,
        perceptual_roughness: 0.9,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    let line_material = materials.add(StandardMaterial {
        // unlit: native scene unit from a display value (policy, `native_units.rs`)
        base_color: crate::native_units::native_colour(Vec3::new(0.85, 0.85, 0.8), 1.0),
        unlit: true,
        ..default()
    });
    let cylinder = meshes.add(Cylinder::new(1.0, 1.0));
    let canopy = commands
        .spawn((
            Transform::from_translation(origin),
            Visibility::Hidden,
            crate::character::Canopy,
            Name::new("parachute"),
        ))
        .id();
    commands.entity(parent).add_child(canopy);
    let surface = commands
        .spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(canopy_material),
            Transform::IDENTITY,
            // the mesh is rebuilt every frame from an empty one: its bounding box from
            // spawn time is a point, and the camera would cull the whole canopy
            bevy::camera::visibility::NoFrustumCulling,
            Name::new("canopy"),
        ))
        .id();
    commands.entity(canopy).add_child(surface);
    let mut lines = Vec::new();
    for chain in &rig.chains {
        for z in [-LINE_SPREAD, LINE_SPREAD] {
            let mut from = rig.base;
            for &to in chain {
                let e = commands
                    .spawn((
                        Mesh3d(cylinder.clone()),
                        MeshMaterial3d(line_material.clone()),
                        Transform::IDENTITY,
                        Name::new("line"),
                    ))
                    .id();
                commands.entity(canopy).add_child(e);
                lines.push((e, from, to, z));
                from = to;
            }
        }
    }
    commands.entity(canopy).insert(CanopyRig {
        mesh,
        lines,
        opening: None,
        scratch: Vec::new(),
    });
    canopy
}

/// Unfold when shown, hold the open pose, and keep the surface and lines on the bones.
pub fn animate_canopy(
    time: Res<Time>,
    rig: Option<Res<ChuteRig>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut canopies: Query<(&mut CanopyRig, &Visibility)>,
    mut lines: Query<&mut Transform>,
) {
    let Some(rig) = rig else { return };
    for (mut canopy, vis) in &mut canopies {
        if *vis == Visibility::Hidden {
            canopy.opening = None;
            continue;
        }
        let duration = rig.open.as_ref().map(|a| a.duration).unwrap_or(0.0);
        let t = match canopy.opening {
            Some(t) => (t + time.delta_secs()).min(duration),
            None => 0.0,
        };
        canopy.opening = Some(t);
        let CanopyRig {
            mesh,
            lines: line_list,
            scratch,
            ..
        } = &mut *canopy;
        let pos = rig.bone_positions(rig.open.as_ref().map(|_| t), scratch);
        // the surface: an arch through the chain tips, extruded front to back
        let (l, m, r) = (
            pos[rig.chains[0][3]],
            pos[rig.chains[1][3]],
            pos[rig.chains[2][3]],
        );
        let (n, mm) = (ARCH_SEGMENTS, DEPTH_SEGMENTS);
        let mut positions = Vec::with_capacity((n + 1) * (mm + 1));
        for j in 0..=mm {
            let v = j as f32 / mm as f32 - 0.5;
            // the front and back edges droop a little
            let droop = 0.25 * (1.0 - (2.0 * v).powi(2));
            for i in 0..=n {
                let p = arch(l, m, r, i as f32 / n as f32);
                positions.push([p.x, p.y - 0.25 + droop, p.z + v * DEPTH]);
            }
        }
        let mut normals = vec![[0.0f32, 1.0, 0.0]; positions.len()];
        for j in 0..=mm {
            for i in 0..=n {
                let at = |ii: usize, jj: usize| Vec3::from(positions[jj * (n + 1) + ii]);
                let du = at((i + 1).min(n), j) - at(i.saturating_sub(1), j);
                let dv = at(i, (j + 1).min(mm)) - at(i, j.saturating_sub(1));
                normals[j * (n + 1) + i] = du.cross(dv).normalize_or(Vec3::Y).to_array();
            }
        }
        if let Some(mut m) = meshes.get_mut(&*mesh) {
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        }
        for &(e, from, to, z) in line_list.iter() {
            if let Ok(mut tf) = lines.get_mut(e) {
                let a = pos[from] + Vec3::Z * z * 0.3;
                let b = pos[to] + Vec3::Z * z;
                *tf = crate::character::segment_transform(a, b, LINE_WIDTH);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `parachute.bsk` and `chute_open_chute.ban` in the dropzone.
    #[test]
    fn canopy_unfolds() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        if !fs.contains("parachute.bsk") {
            return;
        }
        let rig = ChuteRig::load(&fs).expect("parachute skeleton");
        assert_eq!(rig.skeleton.bones.len(), 17);
        let mut scratch = Vec::new();
        let reference = rig.bone_positions(None, &mut scratch);
        let (l, m, r) = (
            reference[rig.chains[0][3]],
            reference[rig.chains[1][3]],
            reference[rig.chains[2][3]],
        );
        assert!(
            (l.x + 1.92).abs() < 0.05 && (l.y - 4.47).abs() < 0.05,
            "{l:?}"
        );
        assert!(m.x.abs() < 0.05 && (m.y - 4.9).abs() < 0.05, "{m:?}");
        assert!((r.x - 1.93).abs() < 0.05, "{r:?}");
        if let Some(open) = &rig.open {
            let start = rig.bone_positions(Some(0.0), &mut scratch);
            let end = rig.bone_positions(Some(open.duration), &mut scratch);
            let span = |p: &[Vec3]| (p[rig.chains[2][3]] - p[rig.chains[0][3]]).length();
            eprintln!(
                "canopy span: folded {:.2} m, open {:.2} m over {:.2} s",
                span(&start),
                span(&end),
                open.duration
            );
            assert!(span(&end) > 3.0, "open span {}", span(&end));
            assert!(span(&start) < span(&end), "the clip should unfold it");
        }
    }
}
