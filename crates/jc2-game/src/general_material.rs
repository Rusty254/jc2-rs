//! Original `General` (RBM) material: PS 6062008 / VS 188280 (JC2-085).
//!
//! An `ExtendedMaterial` over `StandardMaterial` that adds, in the main pass, the parts of
//! the original General pixel shader that research proved (see
//! `research/formats/material-parameters-next.md`, "Major decoded result" and the JC2-085
//! correction, and the disassembly walk-through in `general_main.wgsl`):
//!
//! * **colour**: `C = diffuse(t0) * vertex_colour * AO(t3 at UV1)`. The vertex colour is the
//!   packed float of VS 188280 (`jc2_formats::rbm::general_vertex_colour`). `t3` is optionally
//!   reduced to one value by `dot(t3.rgb, ChannelAOMask)` (`cb2[1].w > 0.5`, UseChannelAO).
//!   UseChannelTextures (`cb2[0].y`) replaces the diffuse with `dot(t0, ChannelTextureMask)`
//!   and alpha 1.
//! * **MPM (`t2`)**: B remaps the ambient + Sun light, `L' = L + B (2 - L)`; shown in the
//!   shader as `C L' = rgb + B (2 C - rgb)` over Bevy's lit diffuse. G scales the specular
//!   (`pow(max(N.H, .001), SpecularPower)`, `SpecularPower` is `cb2[0].x`) and R scales the
//!   environment reflection.
//! * **Sun horizon / AO** (`t11`): ambient `* t11.x`; the Sun's direct term `* shadow`, with
//!   `shadow = max(saturate(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w)` through the shared
//!   `jc2::sun_horizon` functions. Unlike the terrain, General has **no** `(AO / 2 + 1/2)`
//!   factor on the Sun (PS 6062008 `r0.y`), so `horizon_shadow`, not `sun_factor`, is used.
//! * **alpha test (render flag 0x100)**: discard strictly below the native reference
//!   `(2 * 127 - 1) / 510` of `diffuse.a * instance alpha` (instance alpha 1 here).
//!
//! Explicitly **unproven / not done** (all labelled here, none guessed into a constant):
//!
//! * R, the environment reflection (JC2-166, `general-terms-next.md` section 1): the cube
//!   `genericenvcube_dif.dds` as `t4`, `cb0[3] + cb0[10]` from [`GeneralLight`]. Native; the
//!   UNORM sampling is **inferred** (Gap 5).
//! * G's specular colour `cb0[1].xyz = 4 x` gradient row 1 (`FUN_00a58a00`), carried in
//!   [`GeneralLight::specular`]. Native.
//! * The per-instance record (`FUN_009188e0`: receive +0xf, tint +0x14.., fade +0x1c/0x1e, alpha
//!   +0xb, snow +0xc) has **no producer found** (Gap 1): [`GeneralInstance`] carries the identity
//!   defaults per material, the shader applies them natively.
//! * point lights (`cb0[15+i]`, `cb0[80+i]`), cloud providers.
//! * The unit scale: the engine's lights are not the native `cb0[3]`, `cb0[10]`, `cb0[11]`
//!   (their per-hour tables are untraced), so the constant 2 of the B remap is applied in
//!   Bevy's lit-output units (an assumption: engine output == native `C L`).
//! * Render flags 0x2 (WithAlpha), 0x4 (additive) and 0x80 (AnimateTexture), unknown bits and
//!   0x20 + 0x100 together are not supported: those blocks keep the legacy StandardMaterial.
//!   0x40 (UseSnow) is supported as "no snow": no snow provider exists.
//!
//! The functions below are the CPU mirror of the shader (tests and documentation of the
//! formulas); the Rust side also builds the uniform and binds the shared horizon map.

use crate::sun_horizon::{HorizonShadow, SunHorizon};
use crate::terrain_material::SunHorizonShader;
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;
use jc2_formats::dds::{DdsFormat, DdsImage};
use jc2_formats::rbm::GeneralHeader;

/// `textures/genericenvcube_dif.dds`: the MPM-R environment cube `t4`, one static file for
/// every zone, hour and weather (research `general-terms-next.md` section 1, bound by
/// `FUN_0096eed0` from `GetCubeTexture("GenericEnvCube_dif.dds")`).
pub const ENV_CUBE: &str = "textures\\genericenvcube_dif.dds";

pub type GeneralMaterial = ExtendedMaterial<StandardMaterial, GeneralExtension>;

/// Render flags (`CRenderBlockGeneral` +0x7c), from `FUN_00935840`.
pub const FLAG_NO_CULLING: u32 = 0x1;
pub const FLAG_WITH_ALPHA: u32 = 0x2;
pub const FLAG_ADDITIVE: u32 = 0x4;
pub const FLAG_CHANNEL_TEXTURES: u32 = 0x20;
pub const FLAG_SNOW: u32 = 0x40;
pub const FLAG_ANIMATE_TEXTURE: u32 = 0x80;
pub const FLAG_ALPHA_TEST: u32 = 0x100;
pub const FLAG_CHANNEL_AO: u32 = 0x200;
/// The flags with a bounded implementation here (see the module docs for 0x40).
const SUPPORTED_FLAGS: u32 =
    FLAG_NO_CULLING | FLAG_CHANNEL_TEXTURES | FLAG_SNOW | FLAG_ALPHA_TEST | FLAG_CHANNEL_AO;

/// `(2 * 127 - 1) / 510`: the native alpha reference (`FUN_00949aa0`, byte 127 from the type
/// constructor `0x0091a390`). Strictly-below discards.
pub const ALPHA_REFERENCE: f32 = 253.0 / 510.0;

/// Shader-side flags of [`GeneralParams::flags`] (mirrored in `general_main.wgsl`).
pub const SHADER_CHANNEL_TEXTURES: u32 = 1;
pub const SHADER_HAS_MPM: u32 = 2;
pub const SHADER_HAS_AO: u32 = 4;
/// The environment cube is bound (set by [`bind_horizon`]; the cube view of an unbound material
/// is Bevy's white fallback, which must not be reflected).
pub const SHADER_HAS_ENV: u32 = 16;

/// Why a General block is not on the native route (it keeps the legacy StandardMaterial).
pub fn unsupported_reason(h: &GeneralHeader) -> Option<String> {
    let f = h.render_flags;
    if f & FLAG_WITH_ALPHA != 0 {
        return Some("WithAlpha (0x2): native dual-source blend not supported".into());
    }
    if f & FLAG_ADDITIVE != 0 {
        return Some("additive (0x4): native dual-source blend not supported".into());
    }
    if f & FLAG_ANIMATE_TEXTURE != 0 {
        return Some("AnimateTexture (0x80): animation state not supported".into());
    }
    if f & !(SUPPORTED_FLAGS | FLAG_WITH_ALPHA | FLAG_ADDITIVE | FLAG_ANIMATE_TEXTURE) != 0 {
        return Some(format!("unknown render flag bits {f:#x}"));
    }
    if f & FLAG_CHANNEL_TEXTURES != 0 && f & FLAG_ALPHA_TEST != 0 {
        return Some(
            "channel textures + alpha test: depth/shadow pass tests the raw diffuse alpha".into(),
        );
    }
    None
}

/// The uniform (`GeneralParams` in `general_main.wgsl`).
#[derive(Clone, Copy, Debug, PartialEq, ShaderType)]
pub struct GeneralParams {
    /// `cb2[2]`: ChannelTextureMask.
    pub channel_texture_mask: Vec4,
    /// xyz `cb2[1].xyz`: ChannelAOMask; w `cb2[1].w`: channel AO factor (> 0.5 selects).
    pub channel_ao: Vec4,
    /// `cb2[0].x`.
    pub specular_power: f32,
    pub flags: u32,
    pub alpha_reference: f32,
    pub reserved: f32,
    /// The instance record (see [`GeneralInstance`]): x receive (`cb1[2].w`), y LOD fade (`v6.w`),
    /// z alpha (`cb1[11].w`), w snow byte as `f` in 0..1 (`+0xc / 255`).
    pub instance: Vec4,
    /// Instance tint `(rgb, a)` (`+0x14..0x17 / 255`), blended after the fog.
    pub tint: Vec4,
}

/// The per-instance constants of `FUN_009188e0`. Producers unknown (Gap 1): the defaults are the
/// identity (full receive, no fade, alpha 1, no snow, no tint), as JC2-085 assumed.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)] // the drop-in point for the instance-record provider (Gap 1)
pub struct GeneralInstance {
    pub receive: f32,
    pub lod_fade: f32,
    pub alpha: f32,
    pub snow: f32,
    pub tint: Vec4,
}

impl Default for GeneralInstance {
    fn default() -> Self {
        Self {
            receive: 1.0,
            lod_fade: 0.0,
            alpha: 1.0,
            snow: 0.0,
            tint: Vec4::ZERO,
        }
    }
}

#[allow(dead_code)] // see GeneralInstance
impl GeneralParams {
    pub fn with_instance(mut self, i: GeneralInstance) -> Self {
        self.instance = Vec4::new(i.receive, i.lod_fade, i.alpha, i.snow);
        self.tint = i.tint;
        self
    }
}

impl GeneralParams {
    pub fn new(h: &GeneralHeader, has_mpm: bool, has_ao: bool) -> Self {
        let mut flags = 0;
        if h.channel_textures() {
            flags |= SHADER_CHANNEL_TEXTURES;
        }
        if has_mpm {
            flags |= SHADER_HAS_MPM;
        }
        if has_ao {
            flags |= SHADER_HAS_AO;
        }
        let m = h.channel_ao_mask;
        Self {
            channel_texture_mask: Vec4::from_array(h.channel_texture_mask),
            channel_ao: Vec4::new(m[0], m[1], m[2], h.channel_ao_factor),
            specular_power: h.specular_power,
            flags,
            alpha_reference: ALPHA_REFERENCE,
            reserved: 0.0,
            instance: Vec4::new(1.0, 0.0, 1.0, 0.0),
            tint: Vec4::ZERO,
        }
    }
}

impl Default for GeneralParams {
    fn default() -> Self {
        Self {
            channel_texture_mask: Vec4::ZERO,
            channel_ao: Vec4::ZERO,
            specular_power: 1.0,
            flags: 0,
            alpha_reference: ALPHA_REFERENCE,
            reserved: 0.0,
            instance: Vec4::new(1.0, 0.0, 1.0, 0.0),
            tint: Vec4::ZERO,
        }
    }
}

/// The per-frame light constants of the General pixel shader in native colour units
/// (`GeneralLight` in `general_main.wgsl`), shared by every material.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct GeneralLight {
    /// `cb0[3].rgb`: the sun colour by day, the moon colour at night.
    pub sun: Vec4,
    /// `cb0[10].rgb`: the sky ambient (up-facing).
    pub sky_ambient: Vec4,
    /// `cb0[1].xyz = 4 x` gradient row 1 (the factor 4.0 is the float at 0x00e251b4).
    pub specular: Vec4,
    /// `cb0[12].xyz = cb0[10] + cb0[11]`: the flat ambient of the VegetationFoliage pixel shader
    /// (foliage_main.wgsl; the other shaders' structs stop at `specular`, a larger buffer is fine).
    pub ambient_sum: Vec4,
}

impl GeneralLight {
    pub fn from_state(l: &crate::sky_light::LightState) -> Self {
        Self {
            sun: l.sun.extend(0.0),
            sky_ambient: l.sky_ambient.extend(0.0),
            specular: (l.specular * 4.0).extend(0.0),
            ambient_sum: (l.sky_ambient + l.ground_ambient).extend(0.0),
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct GeneralExtension {
    #[uniform(100)]
    pub params: GeneralParams,
    /// `t2`: the properties map (MPM), linear data. Bound to a stand-in when absent.
    #[texture(101)]
    #[sampler(102)]
    pub mpm: Option<Handle<Image>>,
    /// `t3`: the UV1 AO / colour multiplier, linear data.
    #[texture(103)]
    #[sampler(104)]
    pub ao: Option<Handle<Image>>,
    /// `t11`: `ambientocclusionmap.dds` (clamp, linear), filled by [`bind_horizon`].
    #[texture(105)]
    #[sampler(106)]
    pub horizon_map: Handle<Image>,
    /// The per-frame horizon constants (one [`SunHorizon`]), filled by [`bind_horizon`].
    #[storage(107, read_only)]
    pub sun_horizon: Handle<ShaderBuffer>,
    /// `t4`: the environment cube (`genericenvcube_dif.dds`, clamp, linear, mip point, UNORM),
    /// filled by [`bind_horizon`]. `None` binds Bevy's fallback and clears [`SHADER_HAS_ENV`].
    #[texture(108, dimension = "cube")]
    #[sampler(109)]
    pub env_cube: Option<Handle<Image>>,
    /// The per-frame [`GeneralLight`], filled by [`bind_horizon`].
    #[storage(110, read_only)]
    pub light: Handle<ShaderBuffer>,
    /// The frame's 64 selected point lights and the 4 m cell grid (`point_lights.rs`), shared.
    #[storage(120, read_only)]
    pub point_lights: Handle<ShaderBuffer>,
    #[storage(121, read_only)]
    pub point_cells: Handle<ShaderBuffer>,
}

impl GeneralExtension {
    pub fn new(
        params: GeneralParams,
        mpm: Option<Handle<Image>>,
        ao: Option<Handle<Image>>,
    ) -> Self {
        Self {
            params,
            mpm,
            ao,
            horizon_map: Handle::default(),
            sun_horizon: Handle::default(),
            env_cube: None,
            light: Handle::default(),
            point_lights: crate::point_lights::LIGHT_BUFFER,
            point_cells: crate::point_lights::CELL_BUFFER,
        }
    }
}

impl MaterialExtension for GeneralExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("general_main.wgsl")).with_source("embedded"),
        )
    }
}

/// The StandardMaterial under a General extension. Roughness 1 and reflectance 0 take Bevy's
/// own specular and environment lobes out: the original's are the explicit terms of the shader.
pub fn source_material(mut base: StandardMaterial, alpha_test: bool) -> StandardMaterial {
    base.perceptual_roughness = 1.0;
    base.metallic = 0.0;
    base.reflectance = 0.0;
    // the native fog is applied in `general_main.wgsl` (`haze_fog.wgsl`), not Bevy's
    base.fog_enabled = false;
    base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
    // Masked routing runs the fragment shader in the depth / shadow passes too, with the
    // same strictly-below reference as the main pass (render flag 0x100).
    base.alpha_mode = if alpha_test {
        AlphaMode::Mask(ALPHA_REFERENCE)
    } else {
        AlphaMode::Opaque
    };
    base
}

/// Registers the material and its shader. Also keeps `jc2::sun_horizon` loaded when no
/// terrain plugin did.
pub struct GeneralMaterialPlugin;

#[derive(Resource)]
struct GeneralShader(#[allow(dead_code)] Handle<Shader>);

/// The horizon inputs used when there is no terrain (viewer, fixtures): open ground and a
/// shadow of 1 (`cb0[0].z = 1`, floor 0, no Sun match).
#[derive(Resource)]
pub(crate) struct GeneralFallback {
    map: Handle<Image>,
    buffer: Handle<ShaderBuffer>,
    /// The shared per-frame [`GeneralLight`] buffer.
    light: Handle<ShaderBuffer>,
    /// The environment cube, `None` when the file is unreadable (the General route then
    /// reflects nothing; the original's own failure fallback is unresolved, Gap 5).
    env: Option<Handle<Image>>,
}

impl Plugin for GeneralMaterialPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "general_main.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "general_main.wgsl");
        if !app.world().contains_resource::<SunHorizonShader>() {
            bevy::asset::embedded_asset!(app, "sun_horizon.wgsl");
            let horizon: Handle<Shader> = load_embedded_asset!(app, "sun_horizon.wgsl");
            app.insert_resource(SunHorizonShader(horizon));
        }
        crate::haze::ensure_shader(app);
        crate::point_lights::ensure_shader(app);
        app.insert_resource(GeneralShader(shader))
            .add_plugins(MaterialPlugin::<GeneralMaterial>::default())
            .add_systems(Startup, make_fallback)
            .add_systems(
                Update,
                (
                    update_general_light.after(crate::sky_light::update_sky_lighting),
                    bind_horizon,
                )
                    .chain(),
            );
    }
}

fn make_fallback(
    mut commands: Commands,
    game: Option<Res<crate::Game>>,
    mut images: ResMut<Assets<Image>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let env = game.and_then(|game| {
        let image = game
            .fs
            .read_inflated(ENV_CUBE)
            .map_err(|e| e.to_string())
            .and_then(|bytes| env_cube_image(&bytes));
        match image {
            Ok(i) => Some(images.add(i)),
            Err(e) => {
                warn!("{ENV_CUBE}: {e}; the General material reflects nothing");
                None
            }
        }
    });
    commands.insert_resource(GeneralFallback {
        light: buffers.add(ShaderBuffer::from(GeneralLight::default())),
        env,
        map: images.add(crate::sun_horizon::open_ground()),
        buffer: buffers.add(ShaderBuffer::from(SunHorizon {
            cb0: Vec4::new(0.0, 0.0, 1.0, 1.0),
            sun: Vec4::ZERO,
            tilt: Vec4::ZERO,
            ..default()
        })),
    });
}

/// The shared per-frame inputs every native-lit RBM material binds: the horizon map, the
/// `SunHorizon` buffer and the [`GeneralLight`] buffer (the terrain's when there is one and it
/// is enabled, the open-ground fallback otherwise). `None` before the fallback exists.
pub(crate) struct SharedInputs {
    pub map: Handle<Image>,
    pub buffer: Handle<ShaderBuffer>,
    pub light: Handle<ShaderBuffer>,
}

pub(crate) fn shared_inputs(
    horizon: Option<&HorizonShadow>,
    fallback: Option<&GeneralFallback>,
) -> Option<SharedInputs> {
    let fallback = fallback?;
    let (map, buffer) = match horizon {
        Some(h) if h.enabled => (h.ao.clone(), h.params.clone()),
        _ => (fallback.map.clone(), fallback.buffer.clone()),
    };
    Some(SharedInputs {
        map,
        buffer,
        light: fallback.light.clone(),
    })
}

/// Points every General material at the shared horizon map and constants: the terrain's
/// ([`HorizonShadow`]) when there is one and it is enabled, the fallback otherwise. Sweeps
/// the (immutable) asset list and writes only the materials that differ, so a material that
/// is already right is never marked changed.
fn bind_horizon(
    horizon: Option<Res<HorizonShadow>>,
    fallback: Option<Res<GeneralFallback>>,
    mut materials: ResMut<Assets<GeneralMaterial>>,
) {
    let Some(fallback) = fallback else {
        return;
    };
    let (map, buffer) = match &horizon {
        Some(h) if h.enabled => (h.ao.clone(), h.params.clone()),
        _ => (fallback.map.clone(), fallback.buffer.clone()),
    };
    let env_id = fallback.env.as_ref().map(|h| h.id());
    let stale: Vec<AssetId<GeneralMaterial>> = materials
        .iter()
        .filter(|(_, m)| {
            m.extension.horizon_map.id() != map.id()
                || m.extension.sun_horizon.id() != buffer.id()
                || m.extension.light.id() != fallback.light.id()
                || m.extension.env_cube.as_ref().map(|h| h.id()) != env_id
        })
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.horizon_map = map.clone();
            m.extension.sun_horizon = buffer.clone();
            m.extension.light = fallback.light.clone();
            m.extension.env_cube = fallback.env.clone();
            if fallback.env.is_some() {
                m.extension.params.flags |= SHADER_HAS_ENV;
            } else {
                m.extension.params.flags &= !SHADER_HAS_ENV;
            }
        }
    }
}

/// Writes the native `cb0[3]` / `cb0[10]` of the smoothed zone light state into the shared
/// buffer, only when they changed. Without a [`crate::sky_light::SkyLighting`] (viewers,
/// fixtures) the buffer stays zero: nothing is reflected.
fn update_general_light(
    lighting: Option<Res<crate::sky_light::SkyLighting>>,
    fallback: Option<Res<GeneralFallback>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    mut last: Local<Option<GeneralLight>>,
) {
    let (Some(lighting), Some(fallback)) = (lighting, fallback) else {
        return;
    };
    let now = GeneralLight::from_state(&lighting.state);
    if *last == Some(now) {
        return;
    }
    if let Some(mut b) = buffers.get_mut(&fallback.light) {
        b.set_data(now);
        *last = Some(now);
    }
}

/// `genericenvcube_dif.dds` as a GPU cube: BC1 **UNORM** (not sRGB: the cube loader
/// `FUN_00966560` never asks the `_dif` name callback and builds the view with the resource
/// format, so the original samples it as linear UNORM; **inferred** from the absence of the
/// callback, Gap 5), six 128 x 128 faces in DDS order (+X -X +Y -Y +Z -Z, the order of wgpu
/// cube layers), one mip, clamped, linear filtering, mip filter point (sampler `s4` of
/// `FUN_0096eed0`: address (3,3,3), D3D10 filter 0x14).
pub fn env_cube_image(file: &[u8]) -> Result<Image, String> {
    let dds = DdsImage::parse(file).map_err(|e| e.to_string())?;
    // DDSCAPS2_CUBEMAP | all six face flags (caps2 at byte 112)
    let caps2 = file
        .get(112..116)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or("DDS header is short")?;
    if caps2 & 0xfe00 != 0xfe00 {
        return Err(format!("not a six-face cube (caps2 {caps2:#x})"));
    }
    if dds.format != DdsFormat::Dxt1 || dds.width != dds.height || dds.mips != 1 {
        return Err(format!(
            "expected a one-mip square DXT1 cube, got {}x{} {:?} with {} mips",
            dds.width, dds.height, dds.format, dds.mips
        ));
    }
    let face = dds.level_bytes(0).ok_or("cannot size the DXT1 face")?;
    if dds.data.len() < face * 6 {
        return Err("cube data is short".into());
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: dds.width,
            height: dds.height,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        TextureFormat::Bc1RgbaUnorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(dds.data[..face * 6].to_vec());
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    // CLAMP, fields (4, 2): linear min/mag, POINT mip, filter 0x14 (`FUN_0096eed0` slot 4)
    image.sampler = crate::native_sampler::CLAMP_LINEAR_MIP_POINT.sampler();
    Ok(image)
}

// ---- CPU mirror of the shader's channel arithmetic (tests / documentation) ----

/// `dp4 r2.x, t0, cb2[2]`; alpha becomes 1 (`movc r0, cb2[0].y > .5, (r2.x, r2.x, r2.x, 1), t0`).
#[cfg(test)]
pub fn channel_texture_colour(t0: [f32; 4], mask: [f32; 4]) -> [f32; 4] {
    let d: f32 = t0.iter().zip(mask).map(|(a, b)| a * b).sum();
    [d, d, d, 1.0]
}

/// The AO factor of `t3`: `dp3(t3.rgb, ChannelAOMask)` when the factor `cb2[1].w` is above
/// .5 (UseChannelAO), the sampled RGB otherwise.
#[cfg(test)]
pub fn ao_factor(t3: [f32; 3], mask: [f32; 3], factor: f32) -> [f32; 3] {
    if factor > 0.5 {
        let d = t3[0] * mask[0] + t3[1] * mask[1] + t3[2] * mask[2];
        [d, d, d]
    } else {
        t3
    }
}

/// `L + B (2 - L)` (`add r8 = 2 - L; mad r4 = B r8 + L`): a lerp toward the constant 2.
#[cfg(test)]
pub fn lighting_remap(l: f32, b: f32) -> f32 {
    l + b * (2.0 - l)
}

/// What the shader adds to Bevy's lit diffuse `rgb = C L` to get `C L'`.
#[cfg(test)]
pub fn lit_with_remap(c: f32, lit: f32, b: f32) -> f32 {
    lit + b * (2.0 * c - lit)
}

/// `pow(max(N.H, .001), SpecularPower) * G * shadow * (1 - v6.w)` before the colour `cb0[1]`.
#[cfg(test)]
pub fn specular_term(n_dot_h: f32, power: f32, g: f32, shadow: f32, v6w: f32) -> f32 {
    n_dot_h.max(0.001).powf(power) * shadow * g * (1.0 - v6w)
}

/// `1 - saturate(N.V)^8`, the view-angle weight of the reflection term.
#[cfg(test)]
pub fn reflection_view_weight(n_dot_v: f32) -> f32 {
    1.0 - n_dot_v.clamp(0.0, 1.0).powi(8)
}

/// `sat(MPM.r * (1 - sat(N.V)^8))`: the reflection strength before `cube * (cb0[3] + cb0[10])`.
#[cfg(test)]
pub fn reflection_weight(mpm_r: f32, n_dot_v: f32) -> f32 {
    (mpm_r * reflection_view_weight(n_dot_v)).clamp(0.0, 1.0)
}

/// LOD cross-fade (`FUN_009188e0`, instance `+0x1c` mode, `+0x1e` value byte): mode 0 -> 0,
/// mode 1 -> `max(v - 0.75, 0) * 4`, else 1 (0.75 at 0xe30d18).
#[cfg(test)]
pub fn lod_fade(mode: u8, value: u8) -> f32 {
    match mode {
        0 => 0.0,
        1 => (value as f32 / 255.0 - 0.75).max(0.0) * 4.0,
        _ => 1.0,
    }
}

/// The snow constants of an instance byte `+0xc` (`f = byte / 255`), gate render flag 0x40:
/// `(cb1[1].rgb, cb1[1].w, cb1[11].x)`. 0 -> no snow `((0,0,0), 1, 0)`.
#[cfg(test)]
pub fn snow_constants(f: f32) -> ([f32; 3], f32, f32) {
    if f <= 0.0 {
        ([0.0; 3], 1.0, 0.0)
    } else if f < 0.5 {
        ([0.508, 0.54, 0.645], 1.0 - 2.0 * f, 1.0)
    } else {
        let g = f - 0.5;
        ([0.2932, 0.2011, 0.1247], 1.0 - 2.0 * g, -1.0)
    }
}

/// `s = sat((N.y + sat(sign * fade^2 + AO.r) - 1.7 - w) * 5)`, the lerp factor to `cb1[1].rgb * L`.
#[cfg(test)]
pub fn snow_mask(n_y: f32, ao_r: f32, sign: f32, fade: f32, w: f32) -> f32 {
    let x = (sign * fade * fade + ao_r).clamp(0.0, 1.0);
    ((n_y + x - 1.7 - w) * 5.0).clamp(0.0, 1.0)
}

/// The shadow with the receive strength: `h' = 1 + w (h - 1)`, times the cascade term with its
/// floor `0.1 + 0.9 c` (inferred from `cb0[9] = (1, 0, 1 - f, f)`, f = 0.1).
#[cfg(test)]
pub fn receive_shadow(h: f32, w: f32, c: f32) -> f32 {
    (1.0 + w * (h - 1.0)) * (0.1 + 0.9 * c)
}

/// Instance tint after fog: `fogged * (1 - a) + tint * a`.
#[cfg(test)]
pub fn instance_tint(fogged: f32, tint: f32, a: f32) -> f32 {
    fogged * (1.0 - a) + tint * a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(flags: u32) -> GeneralHeader {
        GeneralHeader {
            channel_texture_mask: [0.0, 0.0, 1.0, 0.0],
            channel_ao_mask: [0.0, 1.0, 0.0],
            channel_ao_factor: 1.0,
            depth_offset: 0.0,
            specular_power: 16.0,
            vertex_format: 1,
            position_extent: 1.0,
            uv0_extent: [1.0, 1.0],
            uv1_extent: [1.0, 1.0],
            colour_extent: 1.0,
            packed_colour: 0,
            render_flags: flags,
        }
    }

    #[test]
    fn b_channel_matches_the_disassembled_remap() {
        // PS 6062008: add r8 = 2 - L; mad r4 = B * r8 + L. L=.5/B=.5 -> 1.25; B=1 -> 2.
        assert_eq!(lighting_remap(0.5, 0.5), 1.25);
        assert_eq!(lighting_remap(0.5, 1.0), 2.0);
        assert_eq!(lighting_remap(0.3, 0.0), 0.3);
        assert_eq!(lighting_remap(1.5, 1.0), 2.0);
        // the shader's form on the lit diffuse C L is C L' exactly
        for (c, l, b) in [(0.8, 0.5, 0.5), (0.2, 1.1, 0.25), (1.0, 0.0, 1.0)] {
            let want = c * lighting_remap(l, b);
            assert!(
                (lit_with_remap(c, c * l, b) - want).abs() < 1e-6,
                "{c} {l} {b}"
            );
        }
    }

    #[test]
    fn channel_selection_isolates_each_channel() {
        let t0 = [0.1, 0.2, 0.3, 0.4];
        for (i, v) in t0.iter().enumerate() {
            let mut mask = [0.0; 4];
            mask[i] = 1.0;
            assert_eq!(channel_texture_colour(t0, mask), [*v, *v, *v, 1.0]);
        }
        // the crate's mask [0,0,1,0] selects the third channel
        assert_eq!(channel_texture_colour(t0, [0.0, 0.0, 1.0, 0.0])[0], 0.3);
        let t3 = [0.25, 0.5, 0.75];
        for (i, v) in t3.iter().enumerate() {
            let mut mask = [0.0; 3];
            mask[i] = 1.0;
            assert_eq!(ao_factor(t3, mask, 1.0), [*v; 3]);
        }
        // without UseChannelAO (factor <= .5) the RGB passes through unchanged
        assert_eq!(ao_factor(t3, [0.0, 1.0, 0.0], 0.0), t3);
        assert_eq!(ao_factor(t3, [0.0, 1.0, 0.0], 0.5), t3);
    }

    #[test]
    fn specular_and_reflection_terms() {
        assert_eq!(specular_term(1.0, 120.0, 1.0, 1.0, 0.0), 1.0);
        assert_eq!(specular_term(1.0, 120.0, 0.5, 1.0, 0.0), 0.5);
        assert_eq!(specular_term(1.0, 16.0, 1.0, 0.25, 0.0), 0.25);
        assert_eq!(specular_term(1.0, 16.0, 1.0, 1.0, 1.0), 0.0);
        // negative N.H is clamped to .001 before the power
        assert!((specular_term(-1.0, 2.0, 1.0, 1.0, 0.0) - 1e-6).abs() < 1e-9);
        assert_eq!(reflection_view_weight(1.0), 0.0);
        assert_eq!(reflection_view_weight(0.0), 1.0);
    }

    /// general-terms-next.md sections 1 to 4 (probe fixtures).
    #[test]
    fn instance_and_reflection_terms() {
        assert_eq!(reflection_weight(1.0, 1.0), 0.0);
        assert_eq!(reflection_weight(1.0, 0.0), 1.0);
        assert_eq!(reflection_weight(2.0, 0.5), 1.0);
        assert_eq!(lod_fade(0, 200), 0.0);
        assert_eq!(lod_fade(2, 0), 1.0);
        assert_eq!(lod_fade(1, 191), 0.0);
        assert!((lod_fade(1, 255) - 1.0).abs() < 1e-6);
        // snow fixtures: N.y 1, AO 1, fade 0: w = 0.3 -> 0, 0.4 -> 0.5, 0.45 -> 1; N.y .5 -> 0
        let (_, w, sign) = snow_constants(0.25);
        assert_eq!((w, sign), (0.5, 1.0));
        for (f, want) in [(0.3, 0.0), (0.4, 0.5), (0.45, 1.0)] {
            let (_, w, sign) = snow_constants(f);
            assert!(
                (snow_mask(1.0, 1.0, sign, 0.0, w) - want).abs() < 1e-5,
                "{f}"
            );
        }
        let (_, w, sign) = snow_constants(0.45);
        assert_eq!(snow_mask(0.5, 1.0, sign, 0.0, w), 0.0);
        let (rgb, w, sign) = snow_constants(0.75);
        assert_eq!((rgb[0], w, sign), (0.2932, 0.5, -1.0));
        assert_eq!(snow_constants(0.0), ([0.0; 3], 1.0, 0.0));
        // receive 0 ignores the horizon shadow, never the cascade
        assert_eq!(receive_shadow(0.0, 0.0, 1.0), 1.0);
        assert!((receive_shadow(0.0, 0.0, 0.0) - 0.1).abs() < 1e-6);
        assert_eq!(receive_shadow(0.5, 1.0, 1.0), 0.5);
        assert_eq!(instance_tint(0.8, 0.2, 0.0), 0.8);
        assert_eq!(instance_tint(0.8, 0.2, 1.0), 0.2);
    }

    #[test]
    fn support_matrix_and_params() {
        // the two retail fixtures: hook 0, crate 0x60 (channel textures + snow)
        assert!(unsupported_reason(&header(0)).is_none());
        assert!(unsupported_reason(&header(0x60)).is_none());
        assert!(unsupported_reason(&header(FLAG_ALPHA_TEST)).is_none());
        assert!(unsupported_reason(&header(FLAG_NO_CULLING | FLAG_CHANNEL_AO)).is_none());
        for f in [
            FLAG_WITH_ALPHA,
            FLAG_ADDITIVE,
            FLAG_ANIMATE_TEXTURE,
            0x1000,
            0x80000000,
        ] {
            assert!(unsupported_reason(&header(f)).is_some(), "{f:#x}");
        }
        assert!(unsupported_reason(&header(FLAG_CHANNEL_TEXTURES | FLAG_ALPHA_TEST)).is_some());
        let p = GeneralParams::new(&header(0x60), true, false);
        assert_eq!(p.flags, SHADER_CHANNEL_TEXTURES | SHADER_HAS_MPM);
        assert_eq!(p.channel_texture_mask, Vec4::new(0.0, 0.0, 1.0, 0.0));
        assert_eq!(p.channel_ao, Vec4::new(0.0, 1.0, 0.0, 1.0));
        assert_eq!(p.specular_power, 16.0);
        let p = GeneralParams::new(&header(0), false, true);
        assert_eq!(p.flags, SHADER_HAS_AO);
        assert_eq!(p.instance, Vec4::new(1.0, 0.0, 1.0, 0.0));
        // (2 * 127 - 1) / 510 in f32
        assert_eq!(ALPHA_REFERENCE, 253.0f32 / 510.0f32);
        assert!((ALPHA_REFERENCE - 0.496_078_43).abs() < 1e-7);
    }

    /// Probe fixtures (`general-terms-next.md` section 1): 128 x 128 DXT1, six faces, one mip,
    /// face means +Y (168, 194, 224) and -Y (58, 48, 36); the Bevy image is a BC1 UNORM cube.
    #[test]
    fn retail_env_cube_is_a_bc1_unorm_cube() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP env cube: JC2_TEST_DIR absent");
            return;
        };
        let fs = jc2_formats::GameFs::open(dir).unwrap();
        let bytes = fs.read_inflated(ENV_CUBE).unwrap();
        assert_eq!(bytes.len(), 49_280);
        let image = env_cube_image(&bytes).unwrap();
        let size = image.texture_descriptor.size;
        assert_eq!(
            (size.width, size.height, size.depth_or_array_layers),
            (128, 128, 6)
        );
        assert_eq!(image.texture_descriptor.format, TextureFormat::Bc1RgbaUnorm);
        assert_eq!(image.texture_descriptor.mip_level_count, 1);
        assert_eq!(image.data.as_ref().unwrap().len(), 6 * 8192);
        assert_eq!(
            image.texture_view_descriptor.as_ref().unwrap().dimension,
            Some(TextureViewDimension::Cube)
        );
        let face = |i: usize| {
            let f = &image.data.as_ref().unwrap()[i * 8192..(i + 1) * 8192];
            let dds = DdsImage::parse(&jc2_formats::dds::write_dds(
                128,
                128,
                1,
                DdsFormat::Dxt1,
                f,
            ))
            .unwrap();
            dds.mean_rgb(0, 0, 0, 128, 128).unwrap()
        };
        for (i, want) in [(2, [168.0, 194.0, 224.0]), (3, [58.0, 48.0, 36.0])] {
            let m = face(i);
            assert!(
                m.iter().zip(want).all(|(a, b)| (a - b).abs() < 1.5),
                "face {i}: {m:?}"
            );
        }
        // anything else is refused: a 2D texture is not a cube
        let mut flat = bytes.clone();
        flat[112..116].copy_from_slice(&0u32.to_le_bytes());
        assert!(env_cube_image(&flat).is_err());
    }

    /// The shader parses and validates with stand-ins for Bevy's modules (offline; the real
    /// pipeline is checked at runtime by launching the game).
    #[test]
    fn general_shader_validates() {
        let src = include_str!("general_main.wgsl");
        let mut body = String::new();
        let mut in_import = false;
        for line in src.lines() {
            let t = line.trim();
            if in_import {
                in_import = t != "}";
                continue;
            }
            if t.starts_with("#import") {
                in_import = t.contains('{') && !t.contains('}');
                continue;
            }
            if t.starts_with("#ifdef") || t.starts_with("#ifndef") || t == "#endif" || t == "#else"
            {
                continue;
            }
            body.push_str(
                &line
                    .replace("#{MATERIAL_BIND_GROUP}", "2")
                    .replace("view_bindings::", "")
                    .replace("mesh_view_types::", "")
                    .replace("lighting::", "")
                    .replace("shadows::", "")
                    .replace("pbr_bindings::", ""),
            );
            body.push('\n');
        }
        let stubs = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
    @location(4) color: vec4<f32>,
}
struct FragmentOutput { @location(0) color: vec4<f32> }
struct View {
    world_position: vec3<f32>,
    clip_from_view: mat4x4<f32>,
    view_from_world: mat4x4<f32>,
    exposure: f32,
    mip_bias: f32,
}
@group(0) @binding(0) var<uniform> view: View;
struct DirectionalLight { direction_to_light: vec3<f32>, flags: u32, color: vec4<f32> }
struct Lights { n_directional_lights: u32, directional_lights: array<DirectionalLight, 4> }
@group(0) @binding(1) var<uniform> lights: Lights;
@group(2) @binding(1) var base_color_texture: texture_2d<f32>;
@group(2) @binding(2) var base_color_sampler: sampler;
const DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT: u32 = 1u;
const MESH_FLAGS_SHADOW_RECEIVER_BIT: u32 = 536870912u;
const LAYER_BASE: u32 = 0u;
const STANDARD_MATERIAL_FLAGS_UNLIT_BIT: u32 = 32u;
struct Layer {
    NdotV: f32, N: vec3<f32>, R: vec3<f32>, perceptual_roughness: f32, roughness: f32,
}
struct LightingInput {
    layers: array<Layer, 1>,
    P: vec3<f32>, V: vec3<f32>, diffuse_color: vec3<f32>, metallic: f32,
    F0_dielectric: vec3<f32>, F0_metallic: vec3<f32>, F_ab: vec2<f32>,
}
fn perceptualRoughnessToRoughness(r: f32) -> f32 { return r * r; }
fn F_AB(r: f32, n: f32) -> vec2<f32> { return vec2<f32>(r, n); }
fn directional_light(i: u32, input: ptr<function, LightingInput>, d: bool) -> vec3<f32> {
    return (*input).diffuse_color;
}
fn fetch_directional_shadow(i: u32, p: vec4<f32>, n: vec3<f32>, z: f32, xy: vec2<f32>) -> f32 {
    return 1.0;
}
fn calculate_diffuse_color(c: vec3<f32>, m: f32, st: f32, dt: f32) -> vec3<f32> { return c; }
fn calculate_F0_dielectric(r: vec3<f32>) -> vec3<f32> { return r; }
struct Mat {
    base_color: vec4<f32>, perceptual_roughness: f32, metallic: f32,
    specular_transmission: f32, diffuse_transmission: f32, reflectance: vec3<f32>,
    flags: u32,
}
struct PbrInput {
    material: Mat, N: vec3<f32>, V: vec3<f32>, diffuse_occlusion: vec3<f32>, flags: u32,
    frag_coord: vec4<f32>, world_normal: vec3<f32>, world_position: vec4<f32>,
}
fn pbr_input_from_standard_material(in: VertexOutput, f: bool) -> PbrInput {
    var p: PbrInput; p.material.base_color = vec4<f32>(1.0); p.N = in.world_normal; return p;
}
fn alpha_discard(m: Mat, c: vec4<f32>) -> vec4<f32> { return c; }
fn apply_pbr_lighting(p: PbrInput) -> vec4<f32> { return p.material.base_color; }
fn main_pass_post_lighting_processing(p: PbrInput, c: vec4<f32>) -> vec4<f32> { return c; }
"#;
        let shared: String = [
            include_str!("sun_horizon.wgsl"),
            include_str!("haze_fog.wgsl"),
            include_str!("point_lights.wgsl"),
        ]
        .iter()
        .flat_map(|m| m.lines())
        .filter(|l| !l.starts_with("#define_import_path"))
        .map(|l| l.replace("#{MATERIAL_BIND_GROUP}", "2"))
        .collect::<Vec<_>>()
        .join("\n");
        let full = format!("{stubs}\n{shared}\n{body}");
        let module = naga::front::wgsl::parse_str(&full).unwrap_or_else(|e| {
            panic!("{}", e.emit_to_string(&full));
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        if let Err(e) = validator.validate(&module) {
            panic!("{}", e.emit_to_string(&full));
        }
    }

    /// The shader uses the native decoded constants and the horizon shadow without the
    /// terrain's `(AO / 2 + 1/2)` factor.
    #[test]
    fn shader_follows_the_disassembly() {
        let src = include_str!("general_main.wgsl");
        assert!(src.contains("horizon_shadow(t11"));
        assert!(!src.contains("sun_factor"));
        assert!(src.contains("mpm.b * (2.0 * base.rgb - rgb)"));
    }
}
