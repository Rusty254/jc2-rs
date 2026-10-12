//! The native sea (JC2-164; research `research/formats/water-inputs-next.md`, JC2-102, and
//! `water-consumers-next.md`, JC2-095).
//!
//! * **Geometry**: camera-centred clip-map patches, [`crate::ocean_inputs::CLIPMAP_LEVELS`]
//!   levels of side 16 m .. 16384 m, 7 x 7 windows `[X-3, X+3]` per level, each patch a 5 x 5
//!   vertex grid at `y = 200` ([`SEA_LEVEL`]). A coarse patch draws only the quadrants that no
//!   finer window covers ([`crate::ocean_inputs::clipmap_level_at`]), so patches never overlap
//!   (the water is alpha blended; overlap would blend twice). The pool of 11 x 49 patch entities
//!   is created once and re-pointed when the camera crosses a 16 m cell.
//! * **Material** ([`SeaMaterial`], `sea.wgsl`): the water colour `cb0[14]` (tables E/F by camera
//!   height, `AtmosphereState::water_colour`), the optical density, the sun specular `cb1[0]`
//!   (exponent 256), the shore alpha `o1.w` and the native haze fog. The pixel-shader tier per
//!   patch follows [`crate::ocean_inputs::ocean_shader_variant`].
//! * **Blend**: the native pass blends with `SRC1_ALPHA`; here `AlphaMode::Premultiplied`, exact
//!   for the implemented terms (see `sea.wgsl`). `JC2_SEA_DEBUG=lod` tints each tier.
//! * **Scene depth**: the camera's depth prepass (Bevy), the simplest robust stand-in for the
//!   native `min`-over-samples depth copy. **Non-native**; the prepass costs an extra depth pass.
//!
//! **Not done / non-native** (the hand-in lists them as Gaps): the composite normal map and foam
//! (the highlight uses a flat normal), the planar reflection (no reflection pass exists in this
//! crate), the refraction tint `(0.8, 0.97, 0.98)`, distortion and caustics, the lod0 vertex
//! displacement (the noise is not decoded in the brief), skipping patches whose terrain is
//! entirely above the sea (invisible: the terrain occludes them), `WaterBelow` (JC2-094: the
//! camera under the sea keeps the same material with table F and density 0.15, double sided),
//! cells outside the native 32 km grid (drawn here so the sea reaches the horizon; the native
//! far "global" patches are Gap 5 of the brief).

use crate::atmosphere::AtmosphereState;
use crate::ocean_inputs::{
    CLIPMAP_HALF, CLIPMAP_LEVELS, CLIPMAP_ORIGIN, OceanPs, clipmap_level_at, clipmap_side,
    ocean_shader_variant, sun_specular,
};
use crate::post_effects::PostPass;
use crate::sky::Sun;
use crate::sun_horizon::{HorizonShadow, SunHorizon};
use crate::world::SEA_LEVEL;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

/// Patches per level and axis (the 7 x 7 window).
const WINDOW: usize = (2 * CLIPMAP_HALF + 1) as usize;

/// The project's old sea colour (display value), used only when `gradients.bmp` is missing.
const FALLBACK_DISPLAY: Vec3 = Vec3::new(0.05, 0.25, 0.45);

/// The pixel-shader tiers as the shader indexes them.
const TIERS: usize = 4;

/// What `sea.wgsl` reads (`SeaParams`).
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct SeaParams {
    /// `cb0[14]`: rgb water colour, w optical density.
    pub water: Vec4,
    /// `cb1[0]`: rgb sun-specular colour, w exponent.
    pub spec: Vec4,
    /// xyz: unit vector towards the (flipped) light.
    pub to_light: Vec4,
    /// x: tier, y: debug tint.
    pub flags: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SeaMaterial {
    #[uniform(0)]
    pub params: SeaParams,
    /// The shared per-frame constants; the sea reads only the haze fog fields.
    #[storage(1, read_only)]
    pub sun_horizon: Handle<ShaderBuffer>,
}

impl Material for SeaMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("sea.wgsl")).with_source("embedded"),
        )
    }
    fn alpha_mode(&self) -> AlphaMode {
        // the dual-source `SRC1_ALPHA` blend collapses to One / OneMinusSrcAlpha (sea.wgsl)
        AlphaMode::Premultiplied
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // the camera can be under the sea (WaterBelow is JC2-094): draw both faces
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub struct SeaPlugin;

#[derive(Resource)]
struct SeaShader(#[allow(dead_code)] Handle<Shader>);

impl Plugin for SeaPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "sea.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "sea.wgsl");
        crate::haze::ensure_shader(app);
        app.insert_resource(SeaShader(shader))
            .add_plugins(MaterialPlugin::<SeaMaterial>::default())
            .add_systems(Startup, spawn_sea)
            .add_systems(Update, (update_sea_params, update_sea_patches).chain());
    }
}

/// The quadrant mask of a patch: bit `qz * 2 + qx` set = that quadrant is drawn by this level.
type Mask = u8;

#[derive(Component)]
struct SeaPatch {
    level: usize,
    /// Window slot `0..7` along x / z.
    ix: usize,
    iz: usize,
    mask: Mask,
    tier: usize,
}

#[derive(Resource)]
struct SeaAssets {
    /// One mesh per quadrant mask (index 0 unused): 5 x 5 vertices on the unit square.
    meshes: [Handle<Mesh>; 16],
    materials: [Handle<SeaMaterial>; TIERS],
    fallback_horizon: Handle<ShaderBuffer>,
}

/// The unit patch: 5 x 5 vertices over `[0, 1]^2` at `y = 0`, with the triangles of the
/// quadrants in `mask` only (2 x 2 cells each).
pub fn patch_mesh(mask: Mask) -> Mesh {
    let mut pos = Vec::with_capacity(25);
    let mut uv = Vec::with_capacity(25);
    for j in 0..5 {
        for i in 0..5 {
            pos.push([i as f32 / 4.0, 0.0, j as f32 / 4.0]);
            uv.push([i as f32 / 4.0, j as f32 / 4.0]);
        }
    }
    let mut idx: Vec<u32> = Vec::new();
    for q in 0..4usize {
        if mask & (1 << q) == 0 {
            continue;
        }
        let (qx, qz) = (q & 1, q >> 1);
        for cj in 0..2 {
            for ci in 0..2 {
                let (i, j) = (qx * 2 + ci, qz * 2 + cj);
                let a = (j * 5 + i) as u32;
                let (b, c, d) = (a + 1, a + 5, a + 6);
                // counter-clockwise seen from +y (the material is double sided anyway)
                idx.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 25])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U32(idx))
}

fn spawn_sea(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SeaMaterial>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let debug = std::env::var("JC2_SEA_DEBUG").is_ok_and(|v| v.eq_ignore_ascii_case("lod"));
    let fallback = buffers.add(ShaderBuffer::from(SunHorizon::default()));
    let mats: [Handle<SeaMaterial>; TIERS] = std::array::from_fn(|t| {
        materials.add(SeaMaterial {
            params: SeaParams {
                flags: Vec4::new(t as f32, f32::from(debug), 0.0, 0.0),
                ..default()
            },
            sun_horizon: fallback.clone(),
        })
    });
    let mesh_handles: [Handle<Mesh>; 16] = std::array::from_fn(|m| meshes.add(patch_mesh(m as u8)));
    for level in 0..CLIPMAP_LEVELS {
        for iz in 0..WINDOW {
            for ix in 0..WINDOW {
                commands.spawn((
                    Mesh3d(mesh_handles[15].clone()),
                    MeshMaterial3d(mats[0].clone()),
                    Transform::default(),
                    Visibility::Hidden,
                    NotShadowCaster,
                    NotShadowReceiver,
                    SeaPatch {
                        level,
                        ix,
                        iz,
                        mask: 0,
                        tier: 0,
                    },
                    Name::new(format!("sea L{level} {ix},{iz}")),
                ));
            }
        }
    }
    commands.insert_resource(SeaAssets {
        meshes: mesh_handles,
        materials: mats,
        fallback_horizon: fallback,
    });
}

/// The shader tier index of a pixel-shader variant (`WaterBelow` = 3).
fn tier_index(ps: OceanPs) -> usize {
    match ps {
        OceanPs::Lod0 | OceanPs::Lod0Stereo => 0,
        OceanPs::Lod1 => 1,
        OceanPs::Lod2 => 2,
        OceanPs::Below => 3,
    }
}

/// The per-frame material constants for a camera at `camera_y`.
pub fn sea_params(atmosphere: &AtmosphereState, to_light: Vec3, camera_y: f32) -> SeaParams {
    let colour = if atmosphere.loaded {
        atmosphere.water_colour(camera_y)
    } else {
        crate::native_units::display_native(FALLBACK_DISPLAY)
    };
    let spec = sun_specular(atmosphere.sun_dir.y, camera_y, atmosphere.precip);
    SeaParams {
        water: colour.extend(AtmosphereState::water_density(camera_y)),
        spec,
        to_light: to_light.normalize_or_zero().extend(0.0),
        flags: Vec4::ZERO, // tier / debug are per material, kept by `update_sea_params`
    }
}

/// Camera of the world view (the one with the final colour pass).
type WorldCamera<'w, 's> =
    Query<'w, 's, &'static GlobalTransform, (With<Camera3d>, With<PostPass>)>;

fn update_sea_params(
    atmosphere: Res<AtmosphereState>,
    cameras: WorldCamera,
    suns: Query<&Transform, With<Sun>>,
    horizon: Option<Res<HorizonShadow>>,
    assets: Option<Res<SeaAssets>>,
    mut materials: ResMut<Assets<SeaMaterial>>,
) {
    let (Some(assets), Ok(cam)) = (assets, cameras.single()) else {
        return;
    };
    let to_light = suns
        .single()
        .map_or(atmosphere.sun_dir, |t| t.back().as_vec3());
    let want = sea_params(&atmosphere, to_light, cam.translation().y);
    let buffer = match &horizon {
        Some(h) if h.enabled => h.params.clone(),
        _ => assets.fallback_horizon.clone(),
    };
    for h in &assets.materials {
        let Some(m) = materials.get(h) else { continue };
        let params = SeaParams {
            flags: m.params.flags,
            ..want
        };
        if (m.params != params || m.sun_horizon.id() != buffer.id())
            && let Some(mut m) = materials.get_mut(h)
        {
            m.params = params;
            m.sun_horizon = buffer.clone();
        }
    }
}

/// The quadrant mask of the patch of `level` whose south-west... (min x / min z) corner is
/// `origin`: the quadrants whose centre this level is the finest window for.
fn patch_mask(camera: Vec2, level: usize, origin: Vec2) -> Mask {
    let half = clipmap_side(level) * 0.5;
    let mut mask = 0;
    for q in 0..4usize {
        let c = origin
            + Vec2::new(
                ((q & 1) as f32 + 0.5) * half,
                ((q >> 1) as f32 + 0.5) * half,
            );
        if clipmap_level_at(camera, c) == Some(level) {
            mask |= 1 << q;
        }
    }
    mask
}

/// The grid cell index of the camera at a level, and the origin (min corner) of window slot
/// `i`: `CLIPMAP_ORIGIN + (X - 3 + i) * side`.
fn slot_origin(cam: f32, side: f32, i: usize) -> f32 {
    let x = ((cam - CLIPMAP_ORIGIN) / side).floor();
    CLIPMAP_ORIGIN + (x - CLIPMAP_HALF as f32 + i as f32) * side
}

fn update_sea_patches(
    cameras: WorldCamera,
    assets: Option<Res<SeaAssets>>,
    mut last: Local<Option<(i64, i64, u8)>>,
    mut patches: Query<(
        &mut SeaPatch,
        &mut Transform,
        &mut Visibility,
        &mut Mesh3d,
        &mut MeshMaterial3d<SeaMaterial>,
    )>,
) {
    let (Some(assets), Ok(cam)) = (assets, cameras.single()) else {
        return;
    };
    let p = cam.translation();
    // the finest cell (16 m) and the camera height class decide everything
    let key = (
        ((p.x - CLIPMAP_ORIGIN) / clipmap_side(0)).floor() as i64,
        ((p.z - CLIPMAP_ORIGIN) / clipmap_side(0)).floor() as i64,
        u8::from(p.y < SEA_LEVEL) | (u8::from(p.y - SEA_LEVEL >= 128.0) << 1),
    );
    if *last == Some(key) {
        return;
    }
    *last = Some(key);
    let camera = Vec2::new(p.x, p.z);
    for (mut patch, mut tf, mut vis, mut mesh, mut mat) in &mut patches {
        let side = clipmap_side(patch.level);
        let origin = Vec2::new(
            slot_origin(camera.x, side, patch.ix),
            slot_origin(camera.y, side, patch.iz),
        );
        let mask = patch_mask(camera, patch.level, origin);
        let tier = tier_index(ocean_shader_variant(side, p.y, false).1);
        let want_tf = Transform {
            translation: Vec3::new(origin.x, SEA_LEVEL, origin.y),
            scale: Vec3::new(side, 1.0, side),
            ..default()
        };
        if *tf != want_tf {
            *tf = want_tf;
        }
        let want_vis = if mask == 0 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *vis != want_vis {
            *vis = want_vis;
        }
        if mask != 0 && patch.mask != mask {
            mesh.0 = assets.meshes[usize::from(mask)].clone();
        }
        if mask != 0 && patch.tier != tier {
            mat.0 = assets.materials[tier].clone();
        }
        patch.mask = mask;
        patch.tier = tier;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every point of the plane is drawn by exactly one patch quadrant (no overlap, no hole).
    #[test]
    fn the_quadrant_masks_tile_the_plane_exactly_once() {
        for camera in [
            Vec2::ZERO,
            Vec2::new(37.3, -801.9),
            Vec2::new(-5000.0, 4100.5),
        ] {
            // every drawn quadrant as a rectangle (min x, min z, side)
            let mut rects: Vec<(f32, f32, f32)> = Vec::new();
            for level in 0..CLIPMAP_LEVELS {
                let side = clipmap_side(level);
                for iz in 0..WINDOW {
                    for ix in 0..WINDOW {
                        let o = Vec2::new(
                            slot_origin(camera.x, side, ix),
                            slot_origin(camera.y, side, iz),
                        );
                        let m = patch_mask(camera, level, o);
                        for q in 0..4usize {
                            if m & (1 << q) != 0 {
                                let q0 =
                                    o + Vec2::new((q & 1) as f32, (q >> 1) as f32) * side * 0.5;
                                rects.push((q0.x, q0.y, side * 0.5));
                            }
                        }
                    }
                }
            }
            // the plane around the camera on an 8 m lattice (cell centres)
            let mut tested = 0;
            for dz in -40..40 {
                for dx in -40..40 {
                    let pt = camera + Vec2::new(dx as f32 * 8.0 + 4.0, dz as f32 * 8.0 + 4.0);
                    let hits = rects
                        .iter()
                        .filter(|r| {
                            pt.x >= r.0 && pt.x < r.0 + r.2 && pt.y >= r.1 && pt.y < r.1 + r.2
                        })
                        .count();
                    assert_eq!(hits, 1, "camera {camera:?} point {pt:?}");
                    tested += 1;
                }
            }
            assert_eq!(tested, 6400);
        }
    }

    #[test]
    fn the_finest_level_draws_whole_patches_and_the_mask_meshes_have_their_quadrants() {
        let camera = Vec2::new(8.0, 8.0);
        let o = Vec2::new(
            slot_origin(camera.x, 16.0, 3),
            slot_origin(camera.y, 16.0, 3),
        );
        assert_eq!(patch_mask(camera, 0, o), 15);
        // a 5 x 5 patch has 16 cells = 32 triangles; one quadrant 4 cells = 8 triangles
        let tris = |m: Mask| match patch_mesh(m).indices() {
            Some(Indices::U32(i)) => i.len() / 3,
            _ => panic!(),
        };
        assert_eq!((tris(15), tris(1), tris(6), tris(0)), (32, 8, 16, 0));
    }

    /// The premultiplied output equals the native dual-source expression for a scene colour
    /// that the implemented terms leave untinted (documented in sea.wgsl).
    #[test]
    fn premultiplied_output_equals_the_native_blend_for_an_untinted_scene() {
        let (water, spec, fog) = (
            Vec3::new(0.01, 0.13, 0.13),
            Vec3::new(0.3, 0.2, 0.1),
            Vec3::new(0.5, 0.6, 0.7),
        );
        let dst = Vec3::new(0.2, 0.4, 0.1);
        for (a, o1w, t) in [
            (0.0, 0.05, 1.0),
            (0.46, 0.55, 0.8),
            (1.0, 1.0, 0.3),
            (0.7, 1.0, 1.0),
        ] {
            // native: out = o1w * (T * (lerp(dst, water, a) + spec) + (1-T) fog) + dst (1 - o1w)
            let colour = dst.lerp(water, a) + spec;
            let native = (colour * t + fog * (1.0 - t)) * o1w + dst * (1.0 - o1w);
            // here
            let rgb = (water * a + spec) * t + fog * (1.0 - t);
            let rgb = rgb * o1w;
            let alpha = o1w * (1.0 - t * (1.0 - a));
            let ours = rgb + dst * (1.0 - alpha);
            assert!((native - ours).length() < 1e-6, "{a} {o1w} {t}");
        }
    }

    #[test]
    fn the_params_follow_camera_height_and_the_sun() {
        let mut a = AtmosphereState::evaluate(None, 12.0, 0.0, 0.0);
        a.loaded = true;
        a.haze_e = Vec3::new(0.1, 0.2, 0.3);
        a.haze_f = Vec3::new(0.4, 0.5, 0.6);
        let above = sea_params(&a, Vec3::Y, 250.0);
        assert_eq!(above.water, Vec4::new(0.1, 0.2, 0.3, 0.3));
        let below = sea_params(&a, Vec3::Y, 150.0);
        assert_eq!(below.water, Vec4::new(0.4, 0.5, 0.6, 0.15));
        assert_eq!(above.spec.w, 256.0);
        assert!(above.spec.x > 0.0, "noon sun: highlight on");
        // without tables the project's old colour (as a native unit) stands
        let none = sea_params(
            &AtmosphereState::evaluate(None, 12.0, 0.0, 0.0),
            Vec3::Y,
            250.0,
        );
        assert!((none.water.y - 0.0625).abs() < 1e-6);
    }

    #[test]
    fn tiers_follow_the_variant_and_height() {
        let t = |side: f32, y: f32| tier_index(ocean_shader_variant(side, y, false).1);
        assert_eq!(
            (
                t(16.0, 230.0),
                t(64.0, 230.0),
                t(256.0, 230.0),
                t(16.0, 100.0)
            ),
            (0, 1, 2, 3)
        );
    }
}
