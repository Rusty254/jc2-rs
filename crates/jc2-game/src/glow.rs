//! The distant light-source glow sprites (JC2-197; research `research/formats/glow-sprites.md`,
//! JC2-136): the native `Lightsource` pass of `CRenderBlockTypeLights`.
//!
//! * **Data (native):** `global/all_lights.light_info`, 32012 sprites (`jc2_formats::light_info`),
//!   one mesh per 4096 m tile as in the loader `0x00930000` / `0x00934860`, six vertices per
//!   sprite (non-indexed triangle list). Bevy's per-mesh frustum culling stands in for the
//!   per-tile box test of `0x00970120`; there is no per-sprite CPU culling natively either.
//! * **Shader (native arithmetic):** `glow.wgsl`, the probe-pinned `lightsource` vertex and
//!   pixel stages: distance fade `0.25 * saturate((d - 200) / 800)` (invisible inside 206.4 m),
//!   quad size `1 + type / 2` metres, `lens5.dds` red channel, gain, alpha `* 1.5`.
//! * **Gain (native):** the day / night key `#178dbdf8` written by `FUN_00954f00` from the
//!   clock: 6.0 from 22:00 to 04:00, ramping to 3.5 over 04:00..06:00 and back over 18:00..22:00
//!   ([`glow_gain`], `night-exposure-next.md` section 3).
//!
//! **Not native / assumed** (Gaps G1, G2 of the research doc, JC2-195 and JC2-196):
//! * the blend: the native pass sets engine blend modes 3 (colour) / 2 (alpha), whose driver
//!   state is not decoded. Here additive One / One, with the depth test on and no depth write;
//! * the sprite size constant `A` (inferred as `cot(fovY / 2) * height / 2`), the fixed 720
//!   reference height, and the placement of the pass in the frame (here: the transparent phase,
//!   before the post chain);
//! * the sampler of `lens5.dds` (linear, clamped) and its colour space (the file name has no
//!   `_dif` suffix, so the project's gamma policy reads it as linear).
//!
//! `JC2_NO_GLOW=1` skips the pass (a before / after aid).

use crate::Game;
use crate::sky::TimeOfDay;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use jc2_formats::light_info::{self, GlowSprite, LIGHT_INFO_FILE, TILES_PER_AXIS};

const LENS_TEXTURE: &str = "textures\\atmosphere\\lens5.dds";

/// The gain of the day / night key `#178dbdf8` at `hour` (0..24), `FUN_00a66c30` end /
/// `FUN_00954f00`: `3.5 + (6.0 - 3.5) * f`, `f` = 1 from 22:00 and before 04:00, `(h - 18) / 4`
/// over 18:00..22:00, 0 over 06:00..18:00 and `1 - clamp((h - 4) / 2)` over 04:00..06:00.
pub fn glow_gain(hour: f32) -> f32 {
    let h = hour.rem_euclid(24.0);
    let f = if h >= 22.0 {
        1.0
    } else if h >= 18.0 {
        (h - 18.0) / 4.0
    } else if h > 6.0 {
        0.0
    } else {
        1.0 - ((h - 4.0) / 2.0).clamp(0.0, 1.0)
    };
    3.5 + (6.0 - 3.5) * f
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct GlowParams {
    /// x: the gain (`cb1[0].x` of the pixel stage).
    pub gain: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct GlowMaterial {
    #[uniform(0)]
    pub params: GlowParams,
    #[texture(1)]
    #[sampler(2)]
    pub lens: Handle<Image>,
}

impl Material for GlowMaterial {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("glow.wgsl")).with_source("embedded"),
        )
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        // transparent phase, depth test on, no depth write; the blend is replaced in `specialize`
        AlphaMode::Add
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // ASSUMED (JC2-195): additive One / One. The shader already multiplies the colour by I.
        let add = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Add,
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: add,
                    alpha: add,
                });
            }
        }
        // the quads face the camera by construction; never cull
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Resource)]
struct GlowShader(#[allow(dead_code)] Handle<Shader>);

#[derive(Resource)]
struct GlowAssets(Handle<GlowMaterial>);

pub struct GlowPlugin;

impl Plugin for GlowPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("JC2_NO_GLOW").is_ok() {
            return;
        }
        bevy::asset::embedded_asset!(app, "glow.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "glow.wgsl");
        app.insert_resource(GlowShader(shader))
            .add_plugins(MaterialPlugin::<GlowMaterial>::default())
            .add_systems(Startup, spawn_glow)
            .add_systems(Update, update_glow_gain);
    }
}

/// The six vertices of every sprite of one tile run (non-indexed triangle list, as natively):
/// POSITION = the sprite's world position, UV_0.x = its size, COLOR = rgb.
pub fn tile_mesh(sprites: &[GlowSprite]) -> Mesh {
    let n = sprites.len() * 6;
    let mut pos = Vec::with_capacity(n);
    let mut size = Vec::with_capacity(n);
    let mut colour = Vec::with_capacity(n);
    for s in sprites {
        let rgb = [
            f32::from(s.rgb[0]) / 255.0,
            f32::from(s.rgb[1]) / 255.0,
            f32::from(s.rgb[2]) / 255.0,
            0.0,
        ];
        for _ in 0..6 {
            pos.push(s.pos);
            size.push([s.size(), 0.0]);
            colour.push(rgb);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, size)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colour)
}

/// Split the (tile-sorted) sprites into one run per tile slot; runs outside the grid are
/// dropped.
pub fn tile_runs(sprites: &[GlowSprite]) -> Vec<(usize, Vec<GlowSprite>)> {
    let mut runs: Vec<(usize, Vec<GlowSprite>)> = Vec::new();
    for s in sprites {
        let Some(tile) = s.tile() else { continue };
        match runs.iter_mut().find(|(t, _)| *t == tile) {
            Some((_, run)) => run.push(*s),
            None => runs.push((tile, vec![*s])),
        }
    }
    runs
}

fn spawn_glow(
    mut commands: Commands,
    game: Option<Res<Game>>,
    time: Option<Res<TimeOfDay>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<GlowMaterial>>,
) {
    let Some(game) = game else { return };
    let sprites = match game
        .fs
        .read_any(LIGHT_INFO_FILE)
        .map_err(|e| e.to_string())
        .and_then(|d| light_info::parse(&d).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(e) => {
            warn!("{LIGHT_INFO_FILE}: {e}; no glow sprites");
            return;
        }
    };
    let Some(mut lens) = crate::impostors::atlas_image(&game, LENS_TEXTURE, false) else {
        warn!("{LENS_TEXTURE} not read; no glow sprites");
        return;
    };
    // ASSUMED sampler (Gap G4): linear, clamped
    lens.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    let hour = time.map_or(12.0, |t| t.hours);
    let material = materials.add(GlowMaterial {
        params: GlowParams {
            gain: Vec4::new(glow_gain(hour), 0.0, 0.0, 0.0),
        },
        lens: images.add(lens),
    });
    let runs = tile_runs(&sprites);
    let total: usize = runs.iter().map(|(_, r)| r.len()).sum();
    info!(
        "glow sprites: {total} in {} of {} tiles",
        runs.len(),
        TILES_PER_AXIS * TILES_PER_AXIS
    );
    for (tile, run) in runs {
        commands.spawn((
            Name::new(format!("glow tile {tile}")),
            Mesh3d(meshes.add(tile_mesh(&run))),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }
    commands.insert_resource(GlowAssets(material));
}

/// The gain follows the clock every frame (`FUN_00954f00` runs from the sky update).
fn update_glow_gain(
    time: Res<TimeOfDay>,
    assets: Option<Res<GlowAssets>>,
    mut materials: ResMut<Assets<GlowMaterial>>,
) {
    let Some(assets) = assets else { return };
    let gain = Vec4::new(glow_gain(time.hours), 0.0, 0.0, 0.0);
    if materials
        .get(&assets.0)
        .is_some_and(|m| m.params.gain != gain)
        && let Some(mut m) = materials.get_mut(&assets.0)
    {
        m.params.gain = gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gain_follows_the_native_key() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // day
        assert!(near(glow_gain(12.0), 3.5));
        assert!(near(glow_gain(6.5), 3.5));
        assert!(near(glow_gain(18.0), 3.5));
        // night
        assert!(near(glow_gain(22.0), 6.0));
        assert!(near(glow_gain(23.5), 6.0));
        assert!(near(glow_gain(0.0), 6.0));
        assert!(near(glow_gain(4.0), 6.0));
        // ramps: 20:00 is half way, 05:00 is half way, 06:00 is day again
        assert!(near(glow_gain(20.0), 4.75));
        assert!(near(glow_gain(5.0), 4.75));
        assert!(near(glow_gain(6.0), 3.5));
    }

    fn sprite(x: f32, z: f32, kind: u8) -> GlowSprite {
        GlowSprite {
            pos: [x, 300.0, z],
            rgb: [255, 128, 0],
            kind,
        }
    }

    #[test]
    fn runs_group_by_tile_and_the_mesh_has_six_vertices_per_sprite() {
        let s = [
            sprite(10.0, 10.0, 2),
            sprite(20.0, 30.0, 14),
            sprite(-16000.0, -16000.0, 1),
            sprite(99999.0, 0.0, 1), // outside the grid: dropped
        ];
        let runs = tile_runs(&s);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].1.len(), 2);
        let mesh = tile_mesh(&runs[0].1);
        assert_eq!(mesh.count_vertices(), 12);
    }
}
