//! The native sky passes (JC2-214): the gradient dome, the fog band, the stars and the sun disc
//! and glow, built from the research of JC2-151 / JC2-173 / JC2-174
//! (`research/formats/sky-gradient-next.md`, `sun-glow-next.md`).
//!
//! | pass | native object | here | blend (native, JC2-173) |
//! |---|---|---|---|
//! | `SKY_GRADIENT` 0x28 | the 32-triangle cone, columns by `hour / 1.5`, sun-facing azimuth | [`SkyDomeMaterial`] + `sky_dome.wgsl` | colour ONE / ONE (additive), alpha ONE / ZERO |
//! | `FOG_GRADIENT` 0x29 | ring of 16 segments, fog colour, texture alpha | [`SkyFogBandMaterial`] + `sky_fog_band.wgsl` | SRC_ALPHA / INV_SRC_ALPHA |
//! | `STARS` 0x25 | 2000 screen-space quads, `srand(1)` | [`SkyStarsMaterial`] + `sky_stars.wgsl` | SRC_ALPHA / ONE |
//! | `SUN` 0x26 | the glow (500 m, half size 800) and disc (1500 m, half size 80) quads | [`SkySunMaterial`] + `sky_sun.wgsl` | SRC_ALPHA / ONE |
//!
//! All four draw with depth test GREATER_EQUAL, no depth write, no culling, the clip depth forced
//! to the far plane (so they show only where nothing else drew), and write RGB only. The Bevy
//! transparent phase stands in for the native pass order: a large negative `depth_bias` sorts them
//! before every other transparent item, in the native id order of the additive ones (ascending
//! pass ids, which is *inferred*, JC2-173).
//!
//! **Not native / assumed (each is a Gap in the JC2-214 hand-in):**
//! * the scene clear is the native linear (0.498, 0.498, 1.0, 1.0), depth 0 (`sky::SCENE_CLEAR`,
//!   JC2-203). The dome is additive, so the displayed sky is `saturate(dome + clear)`. The game camera is
//!   non-HDR (8-bit sRGB target, `post_effects.rs`), so blending saturates after each pass as the
//!   native sRGB-view RGBA8 target does (JC2-203 section 12.3);
//! * the sign of the light vector `V`: the dome's sun-facing side and the stars' gate use
//!   `V = -toSun` (JC2-204, inferred from the orange texels agreeing with the sun side);
//! * the in-plane orientation of the sun quads and the rest frame of the stars' rotation
//!   (rows 0 / 2 of `FUN_00a53460` are not decoded; the textures are radial, the stars random);
//! * the fog band mesh (16 segments, radius 2, y -10 .. +1) is a hand decode (`haze-fog-next.md`
//!   section 6, "inferred"); its colour is the native `FUN_00a45110` rule;
//! * the halo (`sunbeam_dif`) and the lens flare are post effects of other tasks (JC2-212 /
//!   JC2-213) and are not drawn here.
//!
//! `JC2_NO_NATIVE_SKY=1` leaves the whole module out (a before / after aid).

use crate::Game;
use crate::atmosphere::AtmosphereState;
use crate::clouds::CloudConfig;
use crate::haze::HazeFog;
use crate::sky::{SkyData, SkyState, TimeOfDay};
use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path, load_embedded_asset};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::NotShadowCaster;
use bevy::mesh::{MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, ColorWrites, Extent3d,
    RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension,
    TextureFormat,
};
use bevy::shader::ShaderRef;
use jc2_formats::sky_dome;
use jc2_formats::weather::Gradient;
use std::f32::consts::PI;

/// The celestial axis the moon's orbit (and so the stars) turns about (`weather.rs` `SUN_AXIS`).
const SKY_AXIS: Vec3 = Vec3::new(0.0, -1.0, 0.8);
/// Glow quad: half size and distance (`FUN_00a6ea10`).
const GLOW_QUAD: (f32, f32) = (800.0, 500.0);
/// Disc quad: half size and distance.
const DISC_QUAD: (f32, f32) = (80.0, 1500.0);
/// Tint scales of `FUN_00a594e0` (`DAT_0108b520`, `DAT_0108b51c`).
const DISC_SCALE: f32 = 2.0;
const GLOW_SCALE: f32 = 0.4;
/// FogGradient ring: radius 2 times the scale (30), 16 segments, vertex y -10 .. +1.
const BAND_SEGMENTS: usize = 16;
/// The sea level of the engine's y axis (the camera switch of `FUN_00a45110`).
const BAND_WATER_SWITCH_Y: f32 = crate::world::SEA_LEVEL;

/// Sort offsets that put the sky ahead of every other transparent item, in pass id order
/// (stars 0x25, sun 0x26, dome 0x28, fog band 0x29).
const BIAS_STARS: f32 = -5.0e7;
const BIAS_SUN_GLOW: f32 = -4.9e7;
const BIAS_SUN_DISC: f32 = -4.8e7;
const BIAS_DOME: f32 = -3.0e7;
const BIAS_BAND: f32 = -2.9e7;

const fn blend(src: BlendFactor, dst: BlendFactor) -> BlendComponent {
    BlendComponent {
        src_factor: src,
        dst_factor: dst,
        operation: BlendOperation::Add,
    }
}

/// The state every native sky block shares: the given blend, RGB write mask, no culling. The
/// depth test (GREATER_EQUAL) and the missing depth write come from the transparent phase.
fn sky_state(
    descriptor: &mut RenderPipelineDescriptor,
    colour: BlendComponent,
    alpha: BlendComponent,
) {
    if let Some(fragment) = descriptor.fragment.as_mut() {
        for target in fragment.targets.iter_mut().flatten() {
            target.blend = Some(BlendState {
                color: colour,
                alpha,
            });
            target.write_mask = ColorWrites::COLOR;
        }
    }
    descriptor.primitive.cull_mode = None;
}

fn shader_path(path: std::path::PathBuf) -> ShaderRef {
    ShaderRef::Path(AssetPath::from_path_buf(path).with_source("embedded"))
}

// ------------------------------------------------------------------------------------ materials

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct DomeParams {
    pub frame_x: Vec4,
    pub frame_y: Vec4,
    pub frame_z: Vec4,
    /// `A`, `B`, `f` of `FUN_00a44d40`.
    pub hour: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SkyDomeMaterial {
    #[uniform(0)]
    pub params: DomeParams,
    #[texture(1)]
    #[sampler(2)]
    pub sky: Handle<Image>,
}

impl Material for SkyDomeMaterial {
    fn vertex_shader() -> ShaderRef {
        shader_path(embedded_path!("sky_dome.wgsl"))
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        BIAS_DOME
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
        // native: colour ONE / ONE (additive), alpha ONE / ZERO (JC2-173 section 10.2)
        sky_state(
            descriptor,
            blend(BlendFactor::One, BlendFactor::One),
            blend(BlendFactor::One, BlendFactor::Zero),
        );
        Ok(())
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct BandParams {
    /// `cb1[4]` of the FogGradient data: `(30, (0.1 + 10 P) 20, 30, 1)`.
    pub scale: Vec4,
    /// `cb1[5]`: `(fog colour, 1)`.
    pub colour: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SkyFogBandMaterial {
    #[uniform(0)]
    pub params: BandParams,
    #[texture(1)]
    #[sampler(2)]
    pub ramp: Handle<Image>,
}

impl Material for SkyFogBandMaterial {
    fn vertex_shader() -> ShaderRef {
        shader_path(embedded_path!("sky_fog_band.wgsl"))
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        BIAS_BAND
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
        // native: colour SRC_ALPHA / INV_SRC_ALPHA, alpha ZERO / ONE
        sky_state(
            descriptor,
            blend(BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
            blend(BlendFactor::Zero, BlendFactor::One),
        );
        Ok(())
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct StarParams {
    pub frame_x: Vec4,
    pub frame_y: Vec4,
    pub frame_z: Vec4,
    /// `cb1[0]` = `(1, 1, 1, min(byte / 128, 1))`.
    pub colour: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SkyStarsMaterial {
    #[uniform(0)]
    pub params: StarParams,
    #[texture(1)]
    #[sampler(2)]
    pub star: Handle<Image>,
}

impl Material for SkyStarsMaterial {
    fn vertex_shader() -> ShaderRef {
        shader_path(embedded_path!("sky_stars.wgsl"))
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        BIAS_STARS
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
        // native: SRC_ALPHA / ONE for colour and alpha
        let add = blend(BlendFactor::SrcAlpha, BlendFactor::One);
        sky_state(descriptor, add, add);
        Ok(())
    }
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct SunParams {
    pub frame_x: Vec4,
    pub frame_y: Vec4,
    pub frame_z: Vec4,
    /// x = half size, y = distance.
    pub geom: Vec4,
    pub tint: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SkySunMaterial {
    #[uniform(0)]
    pub params: SunParams,
    #[texture(1)]
    #[sampler(2)]
    pub image: Handle<Image>,
    /// Sort offset: the glow draws before the disc.
    pub bias: f32,
}

impl Material for SkySunMaterial {
    fn vertex_shader() -> ShaderRef {
        shader_path(embedded_path!("sky_sun.wgsl"))
    }
    fn fragment_shader() -> ShaderRef {
        Self::vertex_shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        self.bias
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
        // native: SRC_ALPHA / ONE colour, ONE / ONE alpha (JC2-174 section 2.3)
        sky_state(
            descriptor,
            blend(BlendFactor::SrcAlpha, BlendFactor::One),
            blend(BlendFactor::One, BlendFactor::One),
        );
        Ok(())
    }
}

// ------------------------------------------------------------------------------------- the maths

/// The dome rotation `FUN_004dc8a0(eye 0, target (Vx, 0, Vz) normalised, up +Y)`: rows
/// `(x, y, z)` with local +z along `-V_xz`. With `V = -toSun` (inferred, JC2-204) local +z points at
/// the sun's azimuth. Directly overhead (`|V_xz|^2 < 1e-6`) the vector stays unnormalised, as natively.
pub fn dome_frame(to_sun: Vec3) -> (Vec3, Vec3, Vec3) {
    let v = Vec3::new(-to_sun.x, 0.0, -to_sun.z);
    let d = if v.length_squared() - 1e-6 >= 0.0 {
        v.normalize()
    } else {
        v
    };
    // row 2 = normalize(eye - target); row 0 = normalize(up x row 2); row 1 = row 2 x row 0
    let z = (-d).normalize_or_zero();
    let x = Vec3::Y.cross(z).normalize_or_zero();
    let y = z.cross(x);
    (x, y, z)
}

/// A frame whose row 1 is `dir`: the plane of the sun quads. **Assumed** orientation of rows 0 and 2
/// (the textures are radial), see the module comment.
pub fn plane_frame(dir: Vec3) -> (Vec3, Vec3, Vec3) {
    let y = dir.normalize_or_zero();
    let reference = if y.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let x = reference.cross(y).normalize_or_zero();
    let z = x.cross(y);
    (x, y, z)
}

/// The star sphere's rotation (the moon's orbit, `FUN_00a45260`: `FUN_00a53460(hour + 12)`): the
/// rotation about the orbit axis by `hour pi / 12`, applied to the rest frame. **Assumed** rest
/// frame: the world axes (the stars are random directions, so only the turning is observable).
pub fn star_frame(hour: f32) -> (Vec3, Vec3, Vec3) {
    let q = Quat::from_axis_angle(SKY_AXIS.normalize(), hour * PI / 12.0);
    (q * Vec3::X, q * Vec3::Y, q * Vec3::Z)
}

/// `FUN_00a594e0`: the disc and glow tints. `f = max(0.9 - 1.5 |L.y|, 0)`; disc
/// `2 (1, 1 - 0.75 f, 1 - f, ...)`, glow `0.4 (1, 1 - 0.4 f, 1 - f, ...)`, alpha 1.
pub fn sun_tints(abs_light_y: f32) -> (Vec4, Vec4) {
    let f = (0.9 - 1.5 * abs_light_y).max(0.0);
    (
        Vec4::new(
            DISC_SCALE,
            DISC_SCALE * (1.0 - 0.75 * f),
            DISC_SCALE * (1.0 - f),
            1.0,
        ),
        Vec4::new(
            GLOW_SCALE,
            GLOW_SCALE * (1.0 - 0.4 * f),
            GLOW_SCALE * (1.0 - f),
            1.0,
        ),
    )
}

/// `FUN_00a45110` colour rule: the water-layer colour when the camera is at or below the sea
/// switch (y 200), the atmospheric fog colour above it.
pub fn band_colour(cam_y: f32, atmosphere: Vec3, water: Vec3) -> Vec3 {
    if cam_y <= BAND_WATER_SWITCH_Y {
        water
    } else {
        atmosphere
    }
}

/// `FUN_00a45110` ring scale: `(30, (0.1 + 10 P) 20, 30, 1)`, `P` the precipitation amount.
pub fn band_scale(precipitation: f32) -> Vec4 {
    Vec4::new(30.0, (0.1 + 10.0 * precipitation) * 20.0, 30.0, 1.0)
}

fn rows(frame: (Vec3, Vec3, Vec3)) -> [Vec4; 3] {
    [
        frame.0.extend(0.0),
        frame.1.extend(0.0),
        frame.2.extend(0.0),
    ]
}

// --------------------------------------------------------------------------------------- meshes

fn dome_mesh() -> Mesh {
    let v = sky_dome::dome_vertices();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        v.iter().map(|v| v.position).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        v.iter().map(|v| v.uv).collect::<Vec<_>>(),
    )
}

/// 17 vertex pairs (a triangle strip): the ring at radius 2, bottom y -10 then top y +1, `v = y`.
/// **Inferred** geometry (`haze-fog-next.md` section 6).
fn band_mesh() -> Mesh {
    let mut pos = Vec::new();
    let mut uv = Vec::new();
    for i in 0..=BAND_SEGMENTS {
        let a = 2.0 * PI * i as f32 / BAND_SEGMENTS as f32;
        let u = i as f32 / BAND_SEGMENTS as f32;
        for y in [-10.0f32, 1.0] {
            pos.push([2.0 * a.sin(), y, 2.0 * a.cos()]);
            uv.push([u, y]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleStrip,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
}

/// Six vertices per star (the exe expands each star six times and picks the corner from the
/// vertex id), POSITION = direction x distance, UV_0.x = size.
fn stars_mesh() -> Mesh {
    let stars = sky_dome::make_stars(sky_dome::STAR_COUNT);
    let mut pos = Vec::with_capacity(stars.len() * 6);
    let mut size = Vec::with_capacity(stars.len() * 6);
    for s in &stars {
        for _ in 0..6 {
            pos.push(s.position);
            size.push([s.size, 0.0]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, size)
}

/// The 6-vertex `Open` quad in the (x, z) plane, local extent -1 .. 1 (scaled by the half size in
/// the shader), the uv order of `FUN_00a6ea10`.
fn sun_quad_mesh() -> Mesh {
    let p = |x: f32, z: f32| [x, 0.0, z];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            p(-1.0, -1.0),
            p(1.0, 1.0),
            p(-1.0, 1.0),
            p(-1.0, -1.0),
            p(1.0, -1.0),
            p(1.0, 1.0),
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![
            [0.0f32, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
        ],
    )
}

// ------------------------------------------------------------------------------------- textures

fn sampler(
    u: ImageAddressMode,
    v: ImageAddressMode,
    mip: ImageFilterMode,
    max_lod: f32,
) -> ImageSampler {
    ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: u,
        address_mode_v: v,
        address_mode_w: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: mip,
        lod_min_clamp: 0.0,
        lod_max_clamp: max_lod,
        ..default()
    })
}

/// `skygradient_dif.dds` as the game reads it: 32 x 128 `R8G8B8A8_UNORM_SRGB`, wrap in u, clamp
/// in v, bilinear, no mips (sampler 0x14 with MaxLOD 0, JC2-173).
fn dome_image(sky: &Gradient) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: sky.width as u32,
            height: sky.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        sky.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = sampler(
        ImageAddressMode::Repeat,
        ImageAddressMode::ClampToEdge,
        ImageFilterMode::Nearest,
        0.0,
    );
    image
}

fn texture(
    game: &Game,
    images: &mut Assets<Image>,
    name: &str,
    srgb: bool,
    sampler_state: ImageSampler,
) -> Option<Handle<Image>> {
    let mut image = crate::impostors::atlas_image(game, name, srgb)?;
    image.sampler = sampler_state;
    Some(images.add(image))
}

// -------------------------------------------------------------------------------------- systems

#[derive(Resource)]
struct SkyShaders(#[allow(dead_code)] Vec<Handle<Shader>>);

#[derive(Component)]
struct StarsEntity;

/// The handles `update_native_sky` rewrites every frame.
#[derive(Resource)]
struct NativeSky {
    dome: Handle<SkyDomeMaterial>,
    band: Option<Handle<SkyFogBandMaterial>>,
    stars: Option<Handle<SkyStarsMaterial>>,
    glow: Option<Handle<SkySunMaterial>>,
    disc: Option<Handle<SkySunMaterial>>,
}

pub struct NativeSkyPlugin;

impl Plugin for NativeSkyPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("JC2_NO_NATIVE_SKY").is_ok() {
            return;
        }
        bevy::asset::embedded_asset!(app, "sky_dome.wgsl");
        bevy::asset::embedded_asset!(app, "sky_fog_band.wgsl");
        bevy::asset::embedded_asset!(app, "sky_stars.wgsl");
        bevy::asset::embedded_asset!(app, "sky_sun.wgsl");
        let shaders = vec![
            load_embedded_asset!(app, "sky_dome.wgsl"),
            load_embedded_asset!(app, "sky_fog_band.wgsl"),
            load_embedded_asset!(app, "sky_stars.wgsl"),
            load_embedded_asset!(app, "sky_sun.wgsl"),
        ];
        app.insert_resource(SkyShaders(shaders))
            .add_plugins((
                MaterialPlugin::<SkyDomeMaterial>::default(),
                MaterialPlugin::<SkyFogBandMaterial>::default(),
                MaterialPlugin::<SkyStarsMaterial>::default(),
                MaterialPlugin::<SkySunMaterial>::default(),
            ))
            .add_systems(Update, (spawn_native_sky, update_native_sky).chain());
    }
}

/// Builds the entities once the game files and the sky table are in.
#[allow(clippy::too_many_arguments)]
fn spawn_native_sky(
    mut commands: Commands,
    game: Option<Res<Game>>,
    data: Res<SkyData>,
    cfg: Option<Res<CloudConfig>>,
    existing: Option<Res<NativeSky>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut domes: ResMut<Assets<SkyDomeMaterial>>,
    mut bands: ResMut<Assets<SkyFogBandMaterial>>,
    mut stars: ResMut<Assets<SkyStarsMaterial>>,
    mut suns: ResMut<Assets<SkySunMaterial>>,
) {
    if existing.is_some() {
        return;
    }
    let (Some(game), Some(atm)) = (game, data.0.as_ref()) else {
        return;
    };
    let with_stars = cfg.is_none_or(|c| c.stars);
    let common = || {
        (
            Transform::default(),
            NotShadowCaster,
            bevy::camera::visibility::NoFrustumCulling,
        )
    };

    let dome = domes.add(SkyDomeMaterial {
        params: DomeParams::default(),
        sky: images.add(dome_image(&atm.sky)),
    });
    commands.spawn((
        Name::new("native sky dome"),
        Mesh3d(meshes.add(dome_mesh())),
        MeshMaterial3d(dome.clone()),
        common(),
    ));

    // the fog band: alpha ramp, CLAMP / CLAMP, linear with mip point (sampler 0x14)
    let band = texture(
        &game,
        &mut images,
        "textures\\atmosphere\\fog_gradient.dds",
        false,
        sampler(
            ImageAddressMode::ClampToEdge,
            ImageAddressMode::ClampToEdge,
            ImageFilterMode::Nearest,
            0.0,
        ),
    )
    .map(|ramp| {
        let m = bands.add(SkyFogBandMaterial {
            params: BandParams::default(),
            ramp,
        });
        commands.spawn((
            Name::new("native fog band"),
            Mesh3d(meshes.add(band_mesh())),
            MeshMaterial3d(m.clone()),
            common(),
        ));
        m
    });

    // the stars: CLAMP, trilinear (sampler 0x15)
    let star = if with_stars {
        texture(
            &game,
            &mut images,
            "textures\\atmosphere\\star_dif.dds",
            true,
            sampler(
                ImageAddressMode::ClampToEdge,
                ImageAddressMode::ClampToEdge,
                ImageFilterMode::Linear,
                f32::MAX,
            ),
        )
        .map(|star| {
            let m = stars.add(SkyStarsMaterial {
                params: StarParams::default(),
                star,
            });
            commands.spawn((
                StarsEntity,
                Name::new("native stars"),
                Mesh3d(meshes.add(stars_mesh())),
                MeshMaterial3d(m.clone()),
                common(),
            ));
            m
        })
    } else {
        None
    };

    // the sun quads: CLAMP, bilinear, no mips (sampler 0x14)
    let quad = meshes.add(sun_quad_mesh());
    let mut sun_quad = |name: &'static str, file: &str, (h, dist): (f32, f32), bias: f32| {
        texture(
            &game,
            &mut images,
            file,
            true,
            sampler(
                ImageAddressMode::ClampToEdge,
                ImageAddressMode::ClampToEdge,
                ImageFilterMode::Nearest,
                0.0,
            ),
        )
        .map(|image| {
            let m = suns.add(SkySunMaterial {
                params: SunParams {
                    geom: Vec4::new(h, dist, 0.0, 0.0),
                    ..default()
                },
                image,
                bias,
            });
            commands.spawn((
                Name::new(name),
                Mesh3d(quad.clone()),
                MeshMaterial3d(m.clone()),
                common(),
            ));
            m
        })
    };
    let glow = sun_quad(
        "native sun glow",
        "textures\\atmosphere\\sun_gradient_dif.dds",
        GLOW_QUAD,
        BIAS_SUN_GLOW,
    );
    let disc = sun_quad(
        "native sun disc",
        "textures\\atmosphere\\sun_dif.dds",
        DISC_QUAD,
        BIAS_SUN_DISC,
    );
    commands.insert_resource(NativeSky {
        dome,
        band,
        stars: star,
        glow,
        disc,
    });
}

/// Every frame: the hour inputs and the sun side of the dome, the band colour, the star turn and
/// alpha, and the sun tints and frame.
#[allow(clippy::too_many_arguments)]
fn update_native_sky(
    time: Res<TimeOfDay>,
    sky: Option<Res<NativeSky>>,
    haze: Res<HazeFog>,
    atmosphere: Res<AtmosphereState>,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    mut domes: ResMut<Assets<SkyDomeMaterial>>,
    mut bands: ResMut<Assets<SkyFogBandMaterial>>,
    mut stars: ResMut<Assets<SkyStarsMaterial>>,
    mut suns: ResMut<Assets<SkySunMaterial>>,
    mut star_vis: Query<&mut Visibility, With<StarsEntity>>,
) {
    let Some(sky) = sky else { return };
    let st = SkyState::at(time.hours);
    let to_sun = st.sun_dir;

    if let Some(mut m) = domes.get_mut(&sky.dome) {
        let [x, y, z] = rows(dome_frame(to_sun));
        let (a, b, f) = sky_dome::hour_inputs(time.hours);
        m.params = DomeParams {
            frame_x: x,
            frame_y: y,
            frame_z: z,
            hour: Vec4::new(a, b, f, 0.0),
        };
    }

    if let (Some(h), Some(cam)) = (&sky.band, cams.iter().next())
        && let Some(mut m) = bands.get_mut(h)
    {
        let y = cam.translation().y;
        let colour = band_colour(
            y,
            haze.colour,
            atmosphere.water_colour(y.min(BAND_WATER_SWITCH_Y - 1e-3)),
        );
        m.params = BandParams {
            scale: band_scale(atmosphere.precip),
            colour: colour.extend(1.0),
        };
    }

    if let Some(h) = &sky.stars
        && let Some(mut m) = stars.get_mut(h)
    {
        // V = -toSun (inferred sign): V.y = -sun_dir.y
        let byte = sky_dome::star_alpha_byte(-to_sun.y);
        let [x, y, z] = rows(star_frame(time.hours));
        m.params = StarParams {
            frame_x: x,
            frame_y: y,
            frame_z: z,
            colour: Vec4::new(1.0, 1.0, 1.0, sky_dome::star_alpha(byte)),
        };
        // the draw is gated on a non-zero alpha byte (`FUN_00a454a0`)
        for mut v in &mut star_vis {
            *v = if byte == 0 {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            };
        }
    }

    // |L.y| of the flipped light equals |sun y|
    let (disc_tint, glow_tint) = sun_tints(to_sun.y.abs());
    let [x, y, z] = rows(plane_frame(to_sun));
    for (handle, tint) in [(&sky.disc, disc_tint), (&sky.glow, glow_tint)] {
        if let Some(h) = handle
            && let Some(mut m) = suns.get_mut(h)
        {
            m.params.frame_x = x;
            m.params.frame_y = y;
            m.params.frame_z = z;
            m.params.tint = tint;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::weather::{elevation_degrees, moon_direction, sun_direction};

    #[test]
    fn the_dome_faces_the_sun_azimuth() {
        for hour in [6.5, 12.0, 17.5, 21.0] {
            let sun = Vec3::from(sun_direction(hour));
            let (x, y, z) = dome_frame(sun);
            let flat = Vec3::new(sun.x, 0.0, sun.z).normalize();
            assert!((z - flat).length() < 1e-5, "{hour}");
            assert!((y - Vec3::Y).length() < 1e-5);
            // a proper rotation: x cross y = z
            assert!((x.cross(y) - z).length() < 1e-5);
            assert!(x.dot(z).abs() < 1e-6);
        }
    }

    #[test]
    fn the_sun_quads_sit_in_the_plane_perpendicular_to_the_sun() {
        let sun = Vec3::from(sun_direction(15.0));
        let (x, y, z) = plane_frame(sun);
        assert!((y - sun).length() < 1e-5);
        assert!(x.dot(y).abs() < 1e-5 && z.dot(y).abs() < 1e-5);
        assert!(x.dot(z).abs() < 1e-5);
    }

    /// Probe table of `sun-glow-next.md` section 2.4 (tolerance 2e-3 for its rounding).
    #[test]
    fn the_sun_tints_follow_the_probe_table() {
        let near = |a: Vec4, b: [f32; 3]| {
            (a.x - b[0]).abs() < 2e-3 && (a.y - b[1]).abs() < 2e-3 && (a.z - b[2]).abs() < 2e-3
        };
        let (d, g) = sun_tints(0.0862);
        assert!(near(d, [2.0, 0.844, 0.459]), "{d:?}");
        assert!(near(g, [0.4, 0.277, 0.092]), "{g:?}");
        let (d, g) = sun_tints(0.0052);
        assert!(near(d, [2.0, 0.662, 0.216]), "{d:?}");
        assert!(near(g, [0.4, 0.257, 0.043]), "{g:?}");
        let (d, g) = sun_tints(0.5346);
        assert!(near(d, [2.0, 1.853, 1.804]), "{d:?}");
        assert!(near(g, [0.4, 0.384, 0.361]), "{g:?}");
        // noon: plain 2.0 and 0.4
        let (d, g) = sun_tints(0.7);
        assert_eq!((d.x, d.y, d.z), (2.0, 2.0, 2.0));
        assert!(near(g, [0.4, 0.4, 0.4]));
    }

    #[test]
    fn the_band_takes_the_fog_colour_above_the_sea_and_the_water_colour_at_or_below() {
        let (a, w) = (Vec3::new(0.8, 0.9, 1.0), Vec3::new(0.1, 0.2, 0.3));
        assert_eq!(band_colour(250.0, a, w), a);
        assert_eq!(band_colour(200.0, a, w), w);
        assert_eq!(band_colour(100.0, a, w), w);
        assert_eq!(band_scale(0.0), Vec4::new(30.0, 2.0, 30.0, 1.0));
        assert!((band_scale(1.0).y - 202.0).abs() < 1e-4);
    }

    /// The star sphere turns with the moon's orbit: the rotation of the moon's position at hour 0
    /// is the moon at the hour (the same Rodrigues rotation about the same axis).
    #[test]
    fn the_star_frame_is_the_moon_orbit() {
        let rest = Vec3::from(moon_direction(0.0));
        for hour in [3.0, 9.5, 14.0, 22.0] {
            let (x, y, z) = star_frame(hour);
            let turned = rest.x * x + rest.y * y + rest.z * z;
            let moon = Vec3::from(moon_direction(hour));
            assert!(
                (turned - moon).length() < 1e-4,
                "{hour}: {turned:?} {moon:?}"
            );
        }
        // the moon is below the horizon when the sun is up, the stars follow the moon
        assert!(elevation_degrees(moon_direction(12.0)) < 0.0);
    }

    #[test]
    fn the_meshes_have_the_native_vertex_counts() {
        assert_eq!(dome_mesh().count_vertices(), 96);
        assert_eq!(band_mesh().count_vertices(), 34);
        assert_eq!(stars_mesh().count_vertices(), 12_000);
        assert_eq!(sun_quad_mesh().count_vertices(), 6);
    }
}
