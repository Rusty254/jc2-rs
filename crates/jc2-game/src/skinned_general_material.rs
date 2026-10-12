//! The native `SkinnedGeneral` pixel shader for the parachute canopy (board 94315ae0; research
//! `research/formats/parachute-canopy-shading.md`, PS offset 6753928, constructor `0x00932250`).
//!
//! The canopy block (`gae05_lod1-gae05.rbm`, header flags `0x00010000`) is an ordinary opaque,
//! double-sided `SkinnedGeneral` block. This material replaces Bevy's lighting for it with the
//! decoded formula (`skinned_general_main.wgsl`):
//!
//! * `mpm` roles (native, from the data flow of the pixel shader): R scales the sun, the
//!   hemisphere ambient and the point lights and gates the rim, G scales the direct specular,
//!   B sets the specular exponent `32 B + 4`.
//! * instance constants are the native defaults (`cb1[0] = (0,0,0,1)`: no tint, `cb1[1]` white,
//!   `cb1[2] = (c, 6, 1, 1)`: full horizon receive). The height ramp `s = sat(6 - 3.3333 y)` is 0
//!   for the whole cloth and the lines (`y >= 1.8`), so `Dm = D`, exponent `32 B + 4`, specular
//!   scale 1 for them; the 124 harness / strap vertices (and Rico's body below 1.8 m) blend toward
//!   `D*D` through the ramp.
//! * the height ramp `s` is applied where the mesh carries the raw model-space height in UV1
//!   (`rbm_loader::block_to_mesh` stores it for every `SkinnedGeneral` block).
//! * rim term with the constructor's constants (`e = 1.5 / lerp(1, 3, t)`, `k = lerp(3.1, 10, t)`
//!   times the second header float, `t` = ramp of the camera distance 4.2 .. 40).
//!
//! Inferred / not done (labelled): the render-state LOD scale `+0xcac` is taken as 1; the sun's
//! cascade floor `0.1 + 0.9 c` is the General route's inferred floor; the terrain AO option
//! `cb4[3]` default; the underwater term `cb0[14]`; the red look of some night shots is a local
//! point light (point-light grid, shared with every lit material), not a material property.

use crate::general_material::{GeneralFallback, shared_inputs};
use crate::sun_horizon::HorizonShadow;
use crate::terrain_material::SunHorizonShader;
use bevy::asset::{AssetPath, embedded_path, load_embedded_asset};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

pub type SkinnedGeneralMaterial = ExtendedMaterial<StandardMaterial, SkinnedGeneralExtension>;

/// Rim distance ramp (constructor `0x00932250`, probe-verified `movss` stores).
#[allow(dead_code)] // CPU mirror of the shader constants (tests, documentation)
pub const RIM_NEAR: f32 = 4.2;
#[allow(dead_code)]
pub const RIM_FAR: f32 = 40.0;

/// The blocks that take this route: `SkinnedGeneral` with the opaque low byte (0) and the
/// double-sided bit 16 of the block flags (`0x00931080`: cull code 1 when low byte == 1 or
/// bit 16). Rico's blocks (`0x100000`, `0x80000`, `4`, low byte 1) keep the hemisphere route.
pub fn is_double_sided_opaque(type_name: &str, flags: u32) -> bool {
    type_name == "SkinnedGeneral" && flags & 0xff == 0 && flags & 0x10000 != 0
}

/// `(e, k)` of the rim term for a camera distance. `lod_scale` is render state `+0xcac`
/// (1.0 here, inferred) and `block_scale` the block's second header float (`+0x30`).
#[allow(dead_code)]
pub fn rim_constants(distance: f32, lod_scale: f32, block_scale: f32) -> (f32, f32) {
    let t = ((distance / lod_scale - RIM_NEAR) / (RIM_FAR - RIM_NEAR)).clamp(0.0, 1.0);
    let e = 1.5 / (1.0 + 2.0 * t);
    let k = (3.1 + (10.0 - 3.1) * t) * block_scale;
    (e, k)
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SkinnedGeneralExtension {
    /// x = rim scale (block `+0x30`), y = LOD scale (`+0xcac`, 1), z = 1 when `mpm` is bound.
    #[uniform(100)]
    pub params: Vec4,
    /// `t2`: the properties map (R light scale, G specular, B exponent), linear data.
    #[texture(101)]
    #[sampler(102)]
    pub mpm: Option<Handle<Image>>,
    /// `t11`: the terrain AO / horizon map, filled by [`bind_inputs`].
    #[texture(105)]
    #[sampler(106)]
    pub horizon_map: Handle<Image>,
    /// The per-frame `SunHorizon` constants, filled by [`bind_inputs`].
    #[storage(107, read_only)]
    pub sun_horizon: Handle<ShaderBuffer>,
    /// The per-frame `GeneralLight` (`cb0[3]`, `cb0[10]`, `cb0[1]`), filled by [`bind_inputs`].
    #[storage(110, read_only)]
    pub light: Handle<ShaderBuffer>,
    #[storage(120, read_only)]
    pub point_lights: Handle<ShaderBuffer>,
    #[storage(121, read_only)]
    pub point_cells: Handle<ShaderBuffer>,
}

impl SkinnedGeneralExtension {
    pub fn new(mpm: Option<Handle<Image>>, block_scale: f32) -> Self {
        Self {
            params: Vec4::new(block_scale, 1.0, f32::from(u8::from(mpm.is_some())), 0.0),
            mpm,
            horizon_map: Handle::default(),
            sun_horizon: Handle::default(),
            light: Handle::default(),
            point_lights: crate::point_lights::LIGHT_BUFFER,
            point_cells: crate::point_lights::CELL_BUFFER,
        }
    }
}

impl MaterialExtension for SkinnedGeneralExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("skinned_general_main.wgsl"))
                .with_source("embedded"),
        )
    }
}

/// The material for one canopy-like block: opaque, double sided (cull none), no Bevy fog (the
/// native haze is applied in the shader), forward rendering for the custom fragment shader.
pub fn source_material(
    mut base: StandardMaterial,
    mpm: Option<Handle<Image>>,
    block_scale: f32,
) -> SkinnedGeneralMaterial {
    base.alpha_mode = AlphaMode::Opaque;
    base.cull_mode = None;
    base.double_sided = true;
    base.fog_enabled = false;
    base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
    SkinnedGeneralMaterial {
        base,
        extension: SkinnedGeneralExtension::new(mpm, block_scale),
    }
}

/// The material for one of Rico's character blocks (clothes, skin, hair, eyeball tile; board
/// edb9fdda): the same shader, but the alpha mode and the cull mode the loader derived from the
/// block flags (`rbm_loader::character_state`) are kept. `blend_ref` is the explicit alpha
/// reference of the blended bit-18 hair pass (Bevy's `Blend` has no cutoff). INFERRED: that these
/// blocks run the canopy's pixel shader (rico-model-detail.md section 2).
pub fn character_material(
    mut base: StandardMaterial,
    mpm: Option<Handle<Image>>,
    block_scale: f32,
    blend_ref: Option<f32>,
) -> SkinnedGeneralMaterial {
    base.fog_enabled = false;
    base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
    let mut extension = SkinnedGeneralExtension::new(mpm, block_scale);
    extension.params.w = blend_ref.unwrap_or(0.0);
    SkinnedGeneralMaterial { base, extension }
}

/// Points every skinned-general material at the shared horizon map, constants and light buffer.
fn bind_inputs(
    horizon: Option<Res<HorizonShadow>>,
    fallback: Option<Res<GeneralFallback>>,
    mut materials: ResMut<Assets<SkinnedGeneralMaterial>>,
) {
    let Some(shared) = shared_inputs(horizon.as_deref(), fallback.as_deref()) else {
        return;
    };
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| {
            m.extension.horizon_map.id() != shared.map.id()
                || m.extension.sun_horizon.id() != shared.buffer.id()
                || m.extension.light.id() != shared.light.id()
        })
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.horizon_map = shared.map.clone();
            m.extension.sun_horizon = shared.buffer.clone();
            m.extension.light = shared.light.clone();
        }
    }
}

#[derive(Resource)]
struct SkinnedGeneralShader(#[allow(dead_code)] Handle<Shader>);

pub struct SkinnedGeneralMaterialPlugin;

impl Plugin for SkinnedGeneralMaterialPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "skinned_general_main.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "skinned_general_main.wgsl");
        if !app.world().contains_resource::<SunHorizonShader>() {
            bevy::asset::embedded_asset!(app, "sun_horizon.wgsl");
            let horizon: Handle<Shader> = load_embedded_asset!(app, "sun_horizon.wgsl");
            app.insert_resource(SunHorizonShader(horizon));
        }
        crate::haze::ensure_shader(app);
        crate::point_lights::ensure_shader(app);
        app.insert_resource(SkinnedGeneralShader(shader))
            .add_plugins(MaterialPlugin::<SkinnedGeneralMaterial>::default())
            .add_systems(Update, bind_inputs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canopy_flags_select_the_route() {
        assert!(is_double_sided_opaque("SkinnedGeneral", 0x0001_0000));
        // Rico's clothes, hair and eyes keep the hemisphere route
        for f in [0x10_0000, 0x8_0000, 4, 1] {
            assert!(!is_double_sided_opaque("SkinnedGeneral", f), "{f:#x}");
        }
        assert!(!is_double_sided_opaque("General", 0x0001_0000));
    }

    #[test]
    fn rim_constants_follow_the_constructor() {
        // near: e = 1.5, k = 3.1; far: e = 0.5, k = 10 (block scale 1)
        assert_eq!(rim_constants(2.0, 1.0, 1.0), (1.5, 3.1));
        let (e, k) = rim_constants(60.0, 1.0, 1.0);
        assert!((e - 0.5).abs() < 1e-6 && (k - 10.0).abs() < 1e-5);
        let (e, k) = rim_constants(22.1, 1.0, 1.0);
        assert!((e - 1.5 / 2.0).abs() < 1e-5 && (k - 6.55).abs() < 1e-4);
    }

    #[test]
    fn shader_follows_the_recipe() {
        let src = include_str!("skinned_general_main.wgsl");
        assert!(src.contains("32.0 * mpm.b + 4.0"));
        assert!(src.contains("saturate(2.0 * mpm.r - 1.0)"));
        assert!(src.contains("4.2") && src.contains("35.8"));
    }
}
