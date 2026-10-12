//! The native hemisphere ambient for plain `StandardMaterial` models (JC2-177).
//!
//! The pixel shaders of Lambert, SkinnedGeneral (characters), CarPaint (vehicles), Window,
//! VegetationBark, Facade, Merged, Decal, SplineRoad and Box blocks compute their ambient as
//! `lerp(cb0[11], cb0[10], sat(N.y / 2 + 1/2))` (research/formats/ambient-hemisphere-next.md,
//! scan of `DX10_Shaders_F.shader_bundle`). Bevy's `AmbientLight` is the uniform mean; the
//! [`HemisphereExtension`] fragment shader (`hemisphere_main.wgsl`) adds `tilt * N.y` from the
//! shared `SunHorizon` buffer (the `tilt` field, `sky_light.rs`), exactly like the General,
//! terrain and grass routes. Lighting, fog and texturing stay Bevy's `StandardMaterial`.
//!
//! JC2-186: the pixel shaders of these families also multiply the ambient by the terrain
//! AmbientOcclusionMap `t11.x` (`research/tools/ambient_ao_probe.py`); the fragment shader
//! samples the shared map ([`HorizonShadow::ao`], bound by [`bind_ao`]) and multiplies
//! `diffuse_occlusion` by it, as the General route does. DecalSimple, DecalDeformable (no `t11`
//! sampled at all) and DeformableWindow (`t11` only feeds the sun) do not read it
//! ([`reads_terrain_ao`]).
//!
//! Not done (labelled, see the research doc): the sky / ground ambients are the engine's
//! (`sky_light.rs`), not a traced native per-hour table.

use crate::sun_horizon::HorizonShadow;
use crate::terrain_material::SunHorizonShader;
use bevy::asset::{AssetPath, embedded_path, load_embedded_asset};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

pub type HemisphereMaterial = ExtendedMaterial<StandardMaterial, HemisphereExtension>;

/// True for the RBM block families whose native pixel shader uses the hemisphere ambient
/// (the "Hemisphere" row of the scan table). Foliage families are uniform and keep their own
/// routes; `General` is included for the blocks that stay on the legacy fallback (the native
/// General route has its own shader with the same tilt).
pub fn is_hemisphere_family(type_name: &str) -> bool {
    matches!(
        type_name,
        "General"
            | "Lambert"
            | "LambertFlatNormal"
            | "Window"
            | "WindowSimple"
            | "DeformableWindow"
            | "Facade"
            | "DecalSimple"
            | "DecalDeformable"
            | "SplineRoad"
            | "Box"
            | "VegetationBark"
    ) || type_name.starts_with("SkinnedGeneral")
        || type_name.starts_with("CarPaint")
        || type_name.starts_with("Merged")
}

/// False for the families whose native pixel shader does not multiply the ambient by `t11.x`
/// (JC2-186 probe: `DecalSimple` and `DecalDeformable` sample no `t11`; `DeformableWindow` uses
/// it for the sun only). Every other hemisphere family does.
pub fn reads_terrain_ao(type_name: &str) -> bool {
    !matches!(type_name, "DecalSimple" | "DecalDeformable" | "DeformableWindow")
}

/// True where the native pixel shader scales the Sun's direct term by the terrain horizon shadow
/// `max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w)`: every family that samples `t11` and
/// has the term (`research/tools/sun_terms_probe.py`: lambert, lambertflatnormal, carpaint,
/// skinnedgeneral, facade, merged, window, windowsimple, deformablewindow, splineroad, box,
/// vegetationbark, general). The two decals sample no `t11`.
pub fn reads_sun_horizon(type_name: &str) -> bool {
    !matches!(type_name, "DecalSimple" | "DecalDeformable")
}

/// True where the Sun's term also carries `t11.x / 2 + 1/2` (probe census: the bark and foliage
/// families, grass, terrain and impostors; **not** lambert, carpaint, skinnedgeneral, facade, merged,
/// window or box). SplineRoad has two pixel shaders: the lit `splineroad` (bundle PS index 2, 460
/// lines) has the factor (`t11.x * 0.5 + 0.5` multiplies `N.L * horizon * cascade`), while
/// `splineroadsimple` is a flat 0.8 grey with no lighting and no `t11` at all, so the lit one is
/// the variant the road's colour comes from (lighting-gaps-next.md G10).
pub fn sun_uses_ao_half(type_name: &str) -> bool {
    matches!(type_name, "VegetationBark" | "SplineRoad")
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct HemisphereExtension {
    /// `t11`: `ambientocclusionmap.dds` (clamp, linear), filled by [`bind_ao`]. Until then the
    /// default (white) image, i.e. AO 1.
    #[texture(102)]
    #[sampler(103)]
    pub ao_map: Handle<Image>,
    /// `x` = 1 when the family reads the terrain AO ([`reads_terrain_ao`]).
    #[uniform(104)]
    pub ao_flags: Vec4,
    /// The shared per-frame constants (`sun_horizon::SunHorizon`); only `tilt` is read.
    /// Filled by `haze::add_haze_binding` (the all-zero fallback without a terrain).
    #[storage(101, read_only)]
    pub sun_horizon: Handle<ShaderBuffer>,
    /// `x` = 1 when the block is a 40-byte VegetationBark with the `_Dirt` blend ([`HemisphereExtension::set_dirt`]).
    #[uniform(109)]
    pub dirt_flags: Vec4,
    /// `t2`: the dirt diffuse (alpha = blend mask) of the native `barkpl0_dirt` pixel shader, read at UV1.
    #[texture(105)]
    #[sampler(106)]
    pub dirt_diffuse: Option<Handle<Image>>,
    /// `t3`: the dirt normal map, read at UV1.
    #[texture(107)]
    #[sampler(108)]
    pub dirt_normal: Option<Handle<Image>>,
    /// The frame's selected point lights and the 4 m cell grid (`point_lights.rs`), shared.
    #[storage(120, read_only)]
    pub point_lights: Handle<ShaderBuffer>,
    #[storage(121, read_only)]
    pub point_cells: Handle<ShaderBuffer>,
}

impl Default for HemisphereExtension {
    fn default() -> Self {
        Self {
            ao_map: Handle::default(),
            ao_flags: Vec4::ZERO,
            dirt_flags: Vec4::ZERO,
            dirt_diffuse: None,
            dirt_normal: None,
            sun_horizon: Handle::default(),
            point_lights: crate::point_lights::LIGHT_BUFFER,
            point_cells: crate::point_lights::CELL_BUFFER,
        }
    }
}

impl HemisphereExtension {
    /// Enables the native `_Dirt` blend of the 40-byte VegetationBark blocks (`barkpl0_dirt` PS,
    /// lighting-gaps-next.md G9): `t2` / `t3` are the material's `properties` / `deformed_diffuse`
    /// slots. `JC2_NO_BARK_DIRT=1` keeps the plain bark (before / after captures).
    pub fn set_dirt(&mut self, diffuse: Handle<Image>, normal: Option<Handle<Image>>) {
        if std::env::var_os("JC2_NO_BARK_DIRT").is_some() {
            return;
        }
        self.dirt_flags.x = 1.0;
        // y = 1 when the dirt normal map is present too
        self.dirt_flags.y = f32::from(normal.is_some());
        self.dirt_diffuse = Some(diffuse);
        self.dirt_normal = normal;
    }
}

impl MaterialExtension for HemisphereExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("hemisphere_main.wgsl"))
                .with_source("embedded"),
        )
    }
}

impl crate::haze::HazeBound for HemisphereMaterial {
    fn haze_buffer(&self) -> &Handle<ShaderBuffer> {
        &self.extension.sun_horizon
    }
    fn set_haze_buffer(&mut self, buffer: Handle<ShaderBuffer>) {
        self.extension.sun_horizon = buffer;
    }
}

/// The StandardMaterial under the extension: Bevy's own PBR and fog stay, but the custom
/// fragment shader needs forward rendering (as the General route).
pub fn source_material(mut base: StandardMaterial, type_name: &str) -> HemisphereMaterial {
    base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
    HemisphereMaterial {
        base,
        extension: HemisphereExtension {
            // `JC2_NO_HEMISPHERE_AO=1`: the ambient without the terrain AO (before / after captures)
            // `JC2_NO_HEMISPHERE_SUN=1`: the Sun without the horizon / AO factors and the cascade floor
            ao_flags: Vec4::new(
                f32::from(
                    reads_terrain_ao(type_name)
                        && std::env::var_os("JC2_NO_HEMISPHERE_AO").is_none(),
                ),
                f32::from(
                    reads_sun_horizon(type_name)
                        && std::env::var_os("JC2_NO_HEMISPHERE_SUN").is_none(),
                ),
                f32::from(
                    sun_uses_ao_half(type_name)
                        && std::env::var_os("JC2_NO_HEMISPHERE_SUN").is_none(),
                ),
                0.0,
            ),
            ..default()
        },
    }
}

/// Sets the explicit alpha-test reference of a blended pass (`ao_flags.w`; none = 0).
pub fn with_alpha_reference(mut m: HemisphereMaterial, reference: Option<f32>) -> HemisphereMaterial {
    m.extension.ao_flags.w = reference.unwrap_or(0.0);
    m
}

/// Points every hemisphere material at the terrain AO map (`HorizonShadow::ao`) once there is
/// one and it is enabled; without it (viewer, fixtures, `JC2_NO_HORIZON`) the white default
/// image stays bound, i.e. AO 1. Writes only the materials that differ.
fn bind_ao(horizon: Option<Res<HorizonShadow>>, mut materials: ResMut<Assets<HemisphereMaterial>>) {
    let Some(h) = horizon.filter(|h| h.enabled) else {
        return;
    };
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.extension.ao_map.id() != h.ao.id())
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.ao_map = h.ao.clone();
        }
    }
}

#[derive(Resource)]
struct HemisphereShader(#[allow(dead_code)] Handle<Shader>);

pub struct HemisphereMaterialPlugin;

impl Plugin for HemisphereMaterialPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "hemisphere_main.wgsl");
        let shader: Handle<Shader> = load_embedded_asset!(app, "hemisphere_main.wgsl");
        if !app.world().contains_resource::<SunHorizonShader>() {
            bevy::asset::embedded_asset!(app, "sun_horizon.wgsl");
            let horizon: Handle<Shader> = load_embedded_asset!(app, "sun_horizon.wgsl");
            app.insert_resource(SunHorizonShader(horizon));
        }
        crate::haze::ensure_shader(app);
        crate::point_lights::ensure_shader(app);
        app.insert_resource(HemisphereShader(shader))
            .add_plugins(MaterialPlugin::<HemisphereMaterial>::default());
        crate::haze::add_haze_binding::<HemisphereMaterial>(app);
        app.add_systems(
            PostUpdate,
            bind_ao.before(bevy::asset::AssetEventSystems),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shader replaces `pbr.wgsl`, where Bevy calls `alpha_discard`: without the call
    /// `AlphaMode::Mask` is inert (rico-eyes.md section 1; the lash cards drew opaque).
    #[test]
    fn shader_applies_the_alpha_test_and_the_blended_reference() {
        let src = include_str!("hemisphere_main.wgsl");
        assert!(src.contains("input.material.base_color = alpha_discard(input.material, input.material.base_color);"));
        assert!(src.contains("ao_flags.w > 0.0 && input.material.base_color.a < ao_flags.w"));
        let m = with_alpha_reference(source_material(StandardMaterial::default(), "SkinnedGeneral"), Some(3.0 / 510.0));
        assert_eq!(m.extension.ao_flags.w, 3.0 / 510.0);
        assert_eq!(source_material(StandardMaterial::default(), "Lambert").extension.ao_flags.w, 0.0);
    }

    #[test]
    fn terrain_ao_families_follow_the_probe() {
        for t in ["Lambert", "SkinnedGeneral", "CarPaint", "Window", "Facade", "Merged", "Box"] {
            assert!(reads_terrain_ao(t), "{t}");
        }
        for t in ["DecalSimple", "DecalDeformable", "DeformableWindow"] {
            assert!(!reads_terrain_ao(t), "{t}");
        }
        let m = source_material(StandardMaterial::default(), "Lambert");
        assert_eq!(m.extension.ao_flags.x, 1.0);
        let m = source_material(StandardMaterial::default(), "DecalSimple");
        assert_eq!(m.extension.ao_flags.x, 0.0);
    }

    #[test]
    fn families_follow_the_scan_table() {
        for t in [
            "Lambert",
            "SkinnedGeneral",
            "SkinnedGeneral1Spec",
            "CarPaint",
            "CarPaintSimple",
            "Window",
            "DeformableWindow",
            "Facade",
            "Merged",
            "MergedSH3WaterFog",
            "DecalSimple",
            "DecalDeformable",
            "SplineRoad",
            "Box",
            "VegetationBark",
            "General",
        ] {
            assert!(is_hemisphere_family(t), "{t}");
        }
        // uniform (cb0[12] * 0.5) families and the impostors keep their own routes
        for t in [
            "VegetationFoliage",
            "BillboardFoliage",
            "DecalSkinned",
            "TreeImpostorTop",
            "TreeImpostorTrunk",
            "Leaves",
            "Flag",
        ] {
            assert!(!is_hemisphere_family(t), "{t}");
        }
    }
}
