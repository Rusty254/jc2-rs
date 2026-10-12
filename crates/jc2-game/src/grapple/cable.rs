//! The grappling wire as the original draws it (`research/formats/grapple-tether-visuals.md`).
//!
//! Native (read from `JustCause2.exe` and `settings/grapplingparams.bin`, doc sections 2 to 4):
//! * the wire is an eight-sided tube, circumradius `wire_width * 1.4142`, rebuilt every frame
//!   (`0x007DE2E0`, `0x00939130`);
//! * a completed two-ended wire is an 11-point parabola dipping 1 percent of its length
//!   (`y += ((2t-1)^2 - 1) * (0.02 * length) * 0.5`), straight while a pull is active; every
//!   other polyline (in flight, cut pieces) is drawn as it is;
//! * a cut leaves two retract pieces anchored at the two ends and aimed at
//!   `T = A + normalize(B - A) * 0.5`, shrinking at 100 m/s (full length) up to 300 m/s, removed
//!   at a length of 0.001 m or less (`0x0083FE20`, `0x0061B770`);
//! * two hook model instances (`hook_model`) sit at the first and the last point.
//!
//! Not native (doc gap 1, labelled again where it is used): the wire shader (a lit
//! `StandardMaterial` with `Wire_DIF` only; `Wire_nrm` / `Wire_mpm` are not used), the texture
//! coordinates (the floats 10.0 / 3.0 at `+0xf8/+0xfc` are not decoded), smooth normals, open
//! tube ends, and the hook orientation and offsets (the hand-end blend `0x0069C350` /
//! `0x0052C310` and the 0.06 m push are unresolved).

use crate::rbm_loader::RbmAsset;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Points of a sagging wire and its segments (native: `0x0106D6B8`).
pub const SAG_SEGMENTS: usize = 10;
/// Sag amplitude as a fraction of the wire length (native: `.rdata 0x00F09268`).
pub const SAG_FRACTION: f32 = 0.02;
/// The tube has eight sides (native: the profile vector of `0x00802EC0`).
pub const TUBE_SIDES: usize = 8;
/// `wire_width * 1.4142` is the circumradius (native: constant at `0x00E49C44`).
#[allow(clippy::approx_constant)] // the native constant is the literal 1.4142, not sqrt(2)
pub const TUBE_RADIUS_FACTOR: f32 = 1.4142;
/// Retract speeds of a cut piece in m/s (native `.rdata 0x00E38304` / `0x00E22EE8`): the slow
/// one at the full length, the fast one as the piece vanishes.
pub const RETRACT_FAST: f32 = 300.0;
pub const RETRACT_SLOW: f32 = 100.0;
/// A piece shorter than this is removed (native).
pub const RETRACT_END: f32 = 0.001;
/// The cut point lies this far from the first end (native: `wire[+0x14]`, ctor `0x00686690`).
pub const CUT_DISTANCE: f32 = 0.5;
/// The loader default of `wire_width` when `grapplingparams.bin` is missing (`0x00802EC0`).
pub const WIRE_WIDTH_DEFAULT: f32 = 0.01;

/// A wire between two points. `sag` is the completed-wire path of the draw (11-point parabola,
/// off while a pull is active); `hooks` puts a hook model on each end.
#[derive(Component, Clone, Copy, Debug, Default)]
#[require(Transform, Visibility)]
pub struct Cable {
    pub a: Vec3,
    pub b: Vec3,
    pub sag: bool,
    pub hooks: bool,
}

/// The hook model (`hook_model`, `general.blz` wea04 lod1), set by the character set-up.
#[derive(Resource, Clone, Default)]
pub struct HookModel(pub Option<Handle<RbmAsset>>);

/// The tube child of a [`Cable`].
#[derive(Component)]
pub(crate) struct CableTube(Entity, Handle<Mesh>);

/// The two hook-model roots of a [`Cable`] (each a parent of the hook's blocks).
#[derive(Component)]
pub(crate) struct CableHooks([Entity; 2]);

/// One retracting piece of a cut wire (native `0x0061B770` record: anchor, unit direction,
/// `total`, `len`).
#[derive(Component, Clone, Copy, Debug)]
pub struct RetractPiece {
    pub anchor: Vec3,
    pub dir: Vec3,
    pub total: f32,
    pub len: f32,
}

impl RetractPiece {
    /// A piece anchored at `anchor`, aimed at `target`.
    pub fn aimed(anchor: Vec3, target: Vec3) -> Self {
        let d = target - anchor;
        let len = d.length();
        Self {
            anchor,
            dir: if len > 1e-6 { d / len } else { Vec3::Y },
            total: len,
            len,
        }
    }

    /// One frame of the native shrink: `len -= ((len / total) * (100 - 300) + 300) * dt`.
    /// Returns false once the piece is to be removed.
    pub fn step(&mut self, dt: f32) -> bool {
        let share = if self.total > 0.0 { self.len / self.total } else { 0.0 };
        self.len -= (share * (RETRACT_SLOW - RETRACT_FAST) + RETRACT_FAST) * dt;
        self.len > RETRACT_END
    }

    pub fn end(&self) -> Vec3 {
        self.anchor + self.dir * self.len
    }
}

/// The two pieces of a cut wire (native cut, doc section 3.2): both aimed at
/// `T = A + normalize(B - A) * 0.5`.
pub fn cut_pieces(a: Vec3, b: Vec3) -> [RetractPiece; 2] {
    let d = (b - a).normalize_or(Vec3::Y);
    let t = a + d * CUT_DISTANCE;
    [RetractPiece::aimed(a, t), RetractPiece::aimed(b, t)]
}

/// Spawn the pieces of a cut wire (drawn by [`draw_cables`], shrunk by [`step_retract_pieces`]).
pub fn spawn_cut(commands: &mut Commands, a: Vec3, b: Vec3) {
    for piece in cut_pieces(a, b) {
        commands.spawn((
            piece,
            Cable {
                a: piece.anchor,
                b: piece.end(),
                sag: false,
                hooks: false,
            },
            Name::new("cut wire piece"),
        ));
    }
}

pub(crate) fn step_retract_pieces(
    mut commands: Commands,
    time: Res<Time>,
    mut pieces: Query<(Entity, &mut RetractPiece, &mut Cable)>,
) {
    let dt = time.delta_secs();
    for (e, mut piece, mut cable) in &mut pieces {
        if piece.step(dt) {
            cable.a = piece.anchor;
            cable.b = piece.end();
        } else {
            commands.entity(e).try_despawn();
        }
    }
}

/// The 11-point sag polyline of a completed two-ended wire (native, doc section 2.2).
/// `tension` is the native factor `1 - elapsed / max` of the pull timer (0 from the first
/// frame as shipped, so a pull draws a straight line).
pub fn sag_polyline(a: Vec3, b: Vec3, tension: f32) -> Vec<Vec3> {
    let d = b - a;
    let amp = d.length() * SAG_FRACTION * tension;
    (0..=SAG_SEGMENTS)
        .map(|i| {
            let t = i as f32 / SAG_SEGMENTS as f32;
            let mut p = a + d * t;
            p.y += (((2.0 * t - 1.0).powi(2)) - 1.0) * amp * 0.5;
            p
        })
        .collect()
}

/// The polyline the tube is swept along: 11 points when the sag path applies, else the two
/// ends as they are.
pub fn cable_points(c: &Cable) -> Vec<Vec3> {
    if c.sag {
        sag_polyline(c.a, c.b, 1.0)
    } else {
        vec![c.a, c.b]
    }
}

/// The octagon of the native loader: `(r,0) (w,w) (0,r) ...` with `w = wire_width`, all on a
/// circle of radius `r = wire_width * 1.4142`.
pub fn octagon(wire_width: f32) -> [Vec2; TUBE_SIDES] {
    let r = wire_width * TUBE_RADIUS_FACTOR;
    std::array::from_fn(|k| {
        let ang = k as f32 / TUBE_SIDES as f32 * std::f32::consts::TAU;
        Vec2::new(ang.cos(), ang.sin()) * r
    })
}

/// The tube swept along `points` (native: eight cross-section vertices per point; the frame
/// is parallel transported, the sweep itself is not decoded). Non-native: smooth outward
/// normals, one extra seam vertex per ring, open ends, texture coordinates with `u` around the
/// tube and `v` along it at one repeat per side length, so texels stay square (the native
/// floats 10.0 / 3.0 at renderable `+0xf8/+0xfc` are unread).
pub fn tube_mesh(points: &[Vec3], wire_width: f32) -> Mesh {
    let profile = octagon(wire_width);
    let ring = TUBE_SIDES + 1;
    let r = wire_width * TUBE_RADIUS_FACTOR;
    let side = 2.0 * r * (std::f32::consts::PI / TUBE_SIDES as f32).sin();
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(points.len() * ring);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(points.len() * ring);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(points.len() * ring);
    let mut indices: Vec<u32> = Vec::new();
    if points.len() >= 2 {
        let mut along = 0.0;
        let mut normal = Vec3::ZERO;
        let mut last_tangent = (points[1] - points[0]).normalize_or(Vec3::Y);
        for (i, &p) in points.iter().enumerate() {
            let before = points[i.saturating_sub(1)];
            let after = points[(i + 1).min(points.len() - 1)];
            let tangent = (after - before).normalize_or(last_tangent);
            last_tangent = tangent;
            if i == 0 {
                normal = tangent.any_orthonormal_vector();
            } else {
                along += p.distance(before);
                // parallel transport: drop the part of the old normal along the new tangent
                normal = (normal - tangent * normal.dot(tangent))
                    .try_normalize()
                    .unwrap_or_else(|| tangent.any_orthonormal_vector());
            }
            let binormal = tangent.cross(normal);
            for k in 0..ring {
                let c = profile[k % TUBE_SIDES];
                let radial = (normal * c.x + binormal * c.y) / r;
                positions.push((p + radial * r).to_array());
                normals.push(radial.to_array());
                uvs.push([k as f32 / TUBE_SIDES as f32, along / (side * TUBE_SIDES as f32)]);
            }
        }
        for i in 0..points.len() - 1 {
            for k in 0..TUBE_SIDES {
                let a = (i * ring + k) as u32;
                let b = a + 1;
                let c = a + ring as u32;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices))
}

/// The hook at the first (`last == false`) or last point, oriented along the wire (native:
/// each hook has a basis from the wire direction, the second reversed, doc section 4).
/// Non-native: which model axis is "forward" (taken as -Z) and no offset (the native 0.06 m
/// push and the 0.7 / 0.3 blend of the first hook are unresolved, doc gap 1).
pub fn hook_transform(points: &[Vec3], last: bool) -> Transform {
    let n = points.len();
    if n < 2 {
        return Transform::from_translation(points.first().copied().unwrap_or(Vec3::ZERO));
    }
    let (p, dir) = if last {
        (points[n - 1], points[n - 2] - points[n - 1])
    } else {
        (points[0], points[1] - points[0])
    };
    let dir = dir.normalize_or(Vec3::NEG_Z);
    // the first hook faces along the wire, the last one the reverse (native reversal)
    let facing = if last { -dir } else { dir };
    let up = if facing.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    Transform::from_translation(p).looking_to(facing, up)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn draw_cables(
    mut commands: Commands,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    server: Option<Res<AssetServer>>,
    mut material: Local<Option<Handle<StandardMaterial>>>,
    params: Option<Res<crate::character::HeroParams>>,
    hook: Option<Res<HookModel>>,
    rbms: Option<Res<Assets<RbmAsset>>>,
    cables: Query<(
        Entity,
        &Cable,
        &GlobalTransform,
        Option<&CableTube>,
        Option<&CableHooks>,
    )>,
    mut children: Query<(&mut Transform, &mut GlobalTransform), Without<Cable>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let width = params
        .map(|p| p.grapple.wire_width)
        .unwrap_or(WIRE_WIDTH_DEFAULT);
    let wire_material = material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                // NON-NATIVE: the wire shader and the use of Wire_nrm / Wire_mpm are unknown
                // (doc gap 1); a lit standard material with Wire_DIF stands in.
                base_color_texture: server.as_ref().map(|s| {
                    s.load_builder()
                        .with_settings(|settings: &mut bevy::image::ImageLoaderSettings| {
                            settings.sampler = crate::rbm_loader::wrap_sampler();
                        })
                        .load::<Image>("jc2://wire_dif.dds")
                }),
                perceptual_roughness: 0.7,
                metallic: 0.0,
                cull_mode: None,
                double_sided: true,
                ..default()
            })
        })
        .clone();
    let hook_asset = hook
        .as_deref()
        .and_then(|h| h.0.as_ref())
        .zip(rbms.as_deref())
        .and_then(|(h, assets)| assets.get(h));
    for (entity, cable, global, tube, hooks) in &cables {
        let points = cable_points(cable);
        // the parent's frame: the tube is built in world space and carried by an inverse
        // (every cable is a root, so this is the identity in practice)
        let inverse = Transform::from_matrix(Mat4::from(global.affine().inverse()));
        match tube {
            Some(CableTube(child, mesh)) => {
                if let Some(mut m) = meshes.get_mut(mesh) {
                    *m = tube_mesh(&points, width);
                }
                if let Ok((mut t, mut g)) = children.get_mut(*child) {
                    *t = inverse;
                    // We run after propagation: publish the world frame now.
                    *g = GlobalTransform::IDENTITY;
                }
            }
            None => {
                let mesh = meshes.add(tube_mesh(&points, width));
                let child = commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(wire_material.clone()),
                        inverse,
                        GlobalTransform::IDENTITY,
                        // the mesh is rebuilt every frame; its AABB would go stale
                        bevy::camera::visibility::NoFrustumCulling,
                        ChildOf(entity),
                    ))
                    .id();
                commands.entity(entity).insert(CableTube(child, mesh));
            }
        }
        if !cable.hooks {
            continue;
        }
        match hooks {
            Some(CableHooks(ids)) => {
                for (i, id) in ids.iter().enumerate() {
                    if let Ok((mut t, mut g)) = children.get_mut(*id) {
                        let world = hook_transform(&points, i == 1);
                        *t = Transform::from_matrix(
                            Mat4::from(global.affine().inverse()) * world.to_matrix(),
                        );
                        *g = GlobalTransform::from(world);
                    }
                }
            }
            None => {
                let Some(asset) = hook_asset else { continue };
                let ids: [Entity; 2] = std::array::from_fn(|i| {
                    let world = hook_transform(&points, i == 1);
                    let root = commands
                        .spawn((
                            Transform::from_matrix(
                                Mat4::from(global.affine().inverse()) * world.to_matrix(),
                            ),
                            GlobalTransform::from(world),
                            Visibility::Inherited,
                            Name::new("wire hook"),
                            ChildOf(entity),
                        ))
                        .id();
                    for b in &asset.blocks {
                        commands.spawn((
                            Mesh3d(b.mesh.clone()),
                            b.material_component(),
                            Transform::IDENTITY,
                            ChildOf(root),
                        ));
                    }
                    root
                });
                commands.entity(entity).insert(CableHooks(ids));
            }
        }
    }
}
