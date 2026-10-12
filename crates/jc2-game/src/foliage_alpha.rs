//! Ordinary foliage discard shared by main, depth and shadow. Native flags and
//! instance-byte arithmetic are supported; per-asset distance scheduling is086.
use bevy::asset::{
    AssetApp, AssetEventSystems, AssetPath, embedded_path, load_embedded_asset, uuid_handle,
};
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::system::EntityCommands;
use bevy::ecs::world::DeferredWorld;
use bevy::mesh::MeshTag;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::{Shader, ShaderRef};
use crate::sun_horizon::HorizonShadow;
use crate::terrain_material::SunHorizonShader;
use jc2_formats::foliage::FoliageAlpha;
use std::collections::{HashMap, HashSet};

pub type FoliageMaterial = ExtendedMaterial<StandardMaterial, FoliageExtension>;

#[derive(Clone, Debug, ShaderType)]
pub struct FoliageParams {
    enabled: u32,
    /// Native lighting flags ([`LIGHT_NORMAL_MAP`], [`LIGHT_AOC`], [`LIGHT_NATIVE`]); 0 = the
    /// Bevy PBR route. (The per-instance byte is the `MeshTag` of the entity
    /// ([`instance_tag`]), not a material value, JC2-153.)
    lighting: u32,
    /// PS `cb1[1].x`: the specular amplitude = model header float 0 (`0x968416`).
    spec_amplitude: f32,
    /// PS `cb1[1].y`: the specular power = model header float 1 (`0x968420`).
    spec_power: f32,
    /// Native distance fade (`veg_fade::VegetationFade::foliage`): `x`, `y` of
    /// `1 - sat(dist * y - x)`, `z` = 1 when active. Shared by every foliage material and
    /// updated by [`push_fade`] when the camera FOV changes; not part of the instance state.
    fade: Vec4,
    /// BillboardFoliage blocks (f012a36f): `xyz` = the mean of the card centres in model space
    /// (block field +0x28, VS `cb2[5].xyz`), `w` = 1; every other foliage block has all zero.
    billboard: Vec4,
}
impl From<FoliageAlpha> for FoliageParams {
    fn from(alpha: FoliageAlpha) -> Self {
        Self {
            enabled: u32::from(alpha.enabled()),
            lighting: 0,
            spec_amplitude: 0.0,
            spec_power: 0.0,
            fade: Vec4::ZERO,
            billboard: Vec4::ZERO,
        }
    }
}

/// The pipeline key of [`FoliageExtension`]: BillboardFoliage blocks swap the vertex stage
/// (`foliage_billboard.wgsl`) in the main, depth and shadow pipelines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FoliageKey {
    pub billboard: bool,
}
impl From<&FoliageExtension> for FoliageKey {
    fn from(e: &FoliageExtension) -> Self {
        Self {
            billboard: e.params.billboard.w != 0.0,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(FoliageKey)]
pub struct FoliageExtension {
    #[uniform(100)]
    params: FoliageParams,
    /// The shared per-frame constants (`sun_horizon::SunHorizon`); foliage reads only the haze
    /// fog fields (JC2-155). Filled by [`bind_haze`], like the General material's.
    #[storage(101, read_only)]
    sun_horizon: Handle<ShaderBuffer>,
    /// The main camera position for the distance fade in orthographic (shadow cascade) views,
    /// `veg_fade::VegetationCamera`; bound by [`push_fade`] (JC2-153).
    #[storage(102, read_only)]
    camera: Handle<ShaderBuffer>,
    pub state: FoliageAlpha,
    /// `t1`: the `*_nrm.dds` (RGB tangent-space normal minus 0.5, A = the translucency weight),
    /// linear data. `None` = geometric normal and alpha 1.
    #[texture(103)]
    #[sampler(104)]
    normal: Option<Handle<Image>>,
    /// `t2`: the `*_aoc.dds` at UV1, linear data; R scales the ambient. `None` = 1.
    #[texture(105)]
    #[sampler(106)]
    aoc: Option<Handle<Image>>,
    /// `t11`: `ambientocclusionmap.dds`, filled by [`bind_native`].
    #[texture(107)]
    #[sampler(108)]
    horizon_map: Handle<Image>,
    /// The per-frame native light colours (`general_material::GeneralLight`), filled by
    /// [`bind_native`]; all zero until a sky lighting exists (the Bevy route then draws).
    #[storage(109, read_only)]
    light: Handle<ShaderBuffer>,
    /// The frame's selected point lights and the 4 m cell grid (`point_lights.rs`), shared.
    #[storage(120, read_only)]
    point_lights: Handle<ShaderBuffer>,
    #[storage(121, read_only)]
    point_cells: Handle<ShaderBuffer>,
}

/// `params.lighting` bits (mirrored in `foliage_main.wgsl`).
pub const LIGHT_NORMAL_MAP: u32 = 1;
pub const LIGHT_AOC: u32 = 2;
pub const LIGHT_NATIVE: u32 = 4;

/// Fixed handle of the BillboardFoliage vertex shader (`foliage_billboard.wgsl`), inserted by
/// [`FoliageMaterialPlugin`]; [`FoliageExtension::specialize`] points the billboard pipelines at it.
const BILLBOARD_VERTEX: Handle<Shader> = uuid_handle!("6a2f0b6e-7a46-4c0e-9a1d-f012a36f0001");

/// Fixed handle of the zero light buffer a foliage material starts with.
const ZERO_LIGHT: Handle<ShaderBuffer> = uuid_handle!("6a2f0b6e-7a46-4c0e-9a1d-7625eb04a001");

/// The per-block inputs of the native `vegetationfoliage` pixel shader (7625eb04): the textures
/// `t1`, `t2` and the specular pair `cb1[1].xy` = model header floats 0 and 1.
#[derive(Clone, Debug, Default)]
pub struct NativeFoliage {
    pub normal: Option<Handle<Image>>,
    pub aoc: Option<Handle<Image>>,
    pub spec_amplitude: f32,
    pub spec_power: f32,
}

impl FoliageExtension {
    pub fn new(state: FoliageAlpha) -> Self {
        Self {
            params: state.into(),
            sun_horizon: Handle::default(),
            camera: Handle::default(),
            state,
            normal: None,
            aoc: None,
            horizon_map: Handle::default(),
            light: ZERO_LIGHT,
            point_lights: crate::point_lights::LIGHT_BUFFER,
            point_cells: crate::point_lights::CELL_BUFFER,
        }
    }

    /// Switches the material to the native lighting route (unless `JC2_NO_NATIVE_FOLIAGE=1`,
    /// the before / after switch for captures).
    pub fn set_native(&mut self, native: NativeFoliage) {
        let mut flags = 0;
        if std::env::var_os("JC2_NO_NATIVE_FOLIAGE").is_none() {
            flags |= LIGHT_NATIVE;
        }
        if native.normal.is_some() {
            flags |= LIGHT_NORMAL_MAP;
        }
        if native.aoc.is_some() {
            flags |= LIGHT_AOC;
        }
        self.params.lighting = flags;
        self.params.spec_amplitude = native.spec_amplitude;
        self.params.spec_power = native.spec_power;
        self.normal = native.normal;
        self.aoc = native.aoc;
    }

    /// Makes this a BillboardFoliage material (f012a36f): the camera-facing vertex stage, the
    /// native `billboardfoliage` pixel shader and the 4x alpha test. `centroid` is the mean of
    /// the block's card centres in model space (what the native loader `0x960490` stores at
    /// block +0x28). The pixel shader needs the native light colours, so `LIGHT_NATIVE` is set.
    pub fn set_billboard(&mut self, centroid: Vec3) {
        self.params.billboard = centroid.extend(1.0);
        self.params.lighting = LIGHT_NATIVE;
    }

    /// The same material for another instance state: every shared input is kept.
    pub fn with_state(&self, state: FoliageAlpha) -> Self {
        let mut e = self.clone();
        e.state = state;
        e.params.enabled = u32::from(state.enabled());
        e
    }
}
impl MaterialExtension for FoliageExtension {
    fn fragment_shader() -> ShaderRef {
        AssetPath::from_path_buf(embedded_path!("foliage_main.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        AssetPath::from_path_buf(embedded_path!("foliage_prepass.wgsl"))
            .with_source("embedded")
            .into()
    }
    fn specialize(
        _pipeline: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        if key.bind_group_data.billboard {
            descriptor.vertex.shader = BILLBOARD_VERTEX;
        }
        Ok(())
    }
}

pub fn source_material(mut base: StandardMaterial, state: FoliageAlpha) -> FoliageMaterial {
    // Masked routing invokes a fragment shader in main/depth/shadow. Our shared
    // discard replaces StandardMaterial cutoff; no alpha-to-coverage is used.
    base.alpha_mode = AlphaMode::Mask(0.0);
    // the native fog is applied in `foliage_main.wgsl` (`haze_fog.wgsl`), not Bevy's
    base.fog_enabled = false;
    base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
    FoliageMaterial {
        base,
        extension: FoliageExtension::new(state),
    }
}

/// The per-instance fade byte (model-instance header +0xb, JC2-140 section 5) rides in the
/// entity's `MeshTag`, so it is per instance in the main, prepass and shadow passes without a
/// private material. Project encoding: `tag = 255 - byte`, because an entity without a
/// `MeshTag` reads 0 and the native initial byte is 255 (`009236d0`).
pub fn instance_tag(byte: u8) -> MeshTag {
    MeshTag(255 - u32::from(byte))
}

/// Only explicit proven instance-byte updates/fixtures change this state. The
/// world does not synthesize a byte from Bevy VisibilityRange distance. `state.instance_byte`
/// is written to the entity's [`MeshTag`] by [`sync_instances`]; a private material copy is
/// made only when `state.raw_flags` differ from the shared source's.
#[derive(Component, Clone)]
pub struct FoliageInstance {
    pub source: Handle<FoliageMaterial>,
    pub state: FoliageAlpha,
}

pub fn attach(entity: &mut EntityCommands, source: &Handle<FoliageMaterial>, state: FoliageAlpha) {
    entity.remove::<MeshMaterial3d<StandardMaterial>>().insert((
        MeshMaterial3d(source.clone()),
        FoliageInstance {
            source: source.clone(),
            state,
        },
    ));
}

/// One narrow attachment component lets generic RBM spawners choose the typed
/// material on insertion without giving every gameplay system custom assets.
#[derive(Component, Clone)]
#[component(on_insert = insert_rbm_material)]
pub struct RbmMaterial {
    pub standard: Handle<StandardMaterial>,
    pub foliage: Option<(Handle<FoliageMaterial>, FoliageAlpha)>,
    /// The native General route (JC2-085); `standard` is its legacy fallback.
    pub general: Option<Handle<crate::general_material::GeneralMaterial>>,
    /// The native hemisphere ambient route (JC2-177) for plain families; `standard` is the
    /// fallback.
    pub hemisphere: Option<Handle<crate::hemisphere_material::HemisphereMaterial>>,
    /// The native opaque double-sided `SkinnedGeneral` route (parachute canopy).
    pub skinned_general:
        Option<Handle<crate::skinned_general_material::SkinnedGeneralMaterial>>,
}
fn insert_rbm_material(mut world: DeferredWorld, context: HookContext) {
    let material = world.get::<RbmMaterial>(context.entity).unwrap().clone();
    let mut commands = world.commands();
    let mut entity = commands.entity(context.entity);
    if let Some((source, state)) = material.foliage {
        attach(&mut entity, &source, state);
    } else if let Some(general) = material.general {
        entity
            .remove::<(
                MeshMaterial3d<StandardMaterial>,
                MeshMaterial3d<FoliageMaterial>,
                MeshMaterial3d<crate::hemisphere_material::HemisphereMaterial>,
                MeshMaterial3d<crate::skinned_general_material::SkinnedGeneralMaterial>,
                FoliageInstance,
            )>()
            .insert(MeshMaterial3d(general));
    } else if let Some(skinned) = material.skinned_general {
        entity
            .remove::<(
                MeshMaterial3d<StandardMaterial>,
                MeshMaterial3d<FoliageMaterial>,
                MeshMaterial3d<crate::general_material::GeneralMaterial>,
                MeshMaterial3d<crate::hemisphere_material::HemisphereMaterial>,
                FoliageInstance,
            )>()
            .insert(MeshMaterial3d(skinned));
    } else if let Some(hemisphere) = material.hemisphere {
        entity
            .remove::<(
                MeshMaterial3d<StandardMaterial>,
                MeshMaterial3d<FoliageMaterial>,
                MeshMaterial3d<crate::general_material::GeneralMaterial>,
                MeshMaterial3d<crate::skinned_general_material::SkinnedGeneralMaterial>,
                FoliageInstance,
            )>()
            .insert(MeshMaterial3d(hemisphere));
    } else {
        entity
            .remove::<(
                MeshMaterial3d<FoliageMaterial>,
                MeshMaterial3d<crate::general_material::GeneralMaterial>,
                MeshMaterial3d<crate::hemisphere_material::HemisphereMaterial>,
                MeshMaterial3d<crate::skinned_general_material::SkinnedGeneralMaterial>,
                FoliageInstance,
            )>()
            .insert(MeshMaterial3d(material.standard));
    }
}

#[derive(Resource, Default)]
struct OwnedFoliageMaterials {
    materials: HashMap<Entity, OwnedMaterial>,
    pending: HashSet<Entity>,
}
struct OwnedMaterial {
    handle: Handle<FoliageMaterial>,
    source: Handle<FoliageMaterial>,
}

/// Headless registration and instance ownership, independent of a GPU plugin.
pub struct FoliageInstancePlugin;
impl Plugin for FoliageInstancePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<FoliageMaterial>()
            .init_asset::<crate::general_material::GeneralMaterial>()
            .init_asset::<crate::hemisphere_material::HemisphereMaterial>()
            .init_asset::<crate::skinned_general_material::SkinnedGeneralMaterial>()
            .init_resource::<OwnedFoliageMaterials>()
            .add_systems(PostUpdate, sync_instances.before(AssetEventSystems));
    }
}

#[derive(Resource)]
struct FoliageShaders(#[allow(dead_code)] [Handle<Shader>; 3]);
pub struct FoliageMaterialPlugin;
impl Plugin for FoliageMaterialPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "foliage_alpha.wgsl");
        bevy::asset::embedded_asset!(app, "foliage_main.wgsl");
        bevy::asset::embedded_asset!(app, "foliage_prepass.wgsl");
        let shaders = [
            load_embedded_asset!(app, "foliage_alpha.wgsl"),
            load_embedded_asset!(app, "foliage_main.wgsl"),
            load_embedded_asset!(app, "foliage_prepass.wgsl"),
        ];
        crate::haze::ensure_shader(app);
        crate::point_lights::ensure_shader(app);
        if !app.world().contains_resource::<SunHorizonShader>() {
            bevy::asset::embedded_asset!(app, "sun_horizon.wgsl");
            let horizon: Handle<Shader> = load_embedded_asset!(app, "sun_horizon.wgsl");
            app.insert_resource(SunHorizonShader(horizon));
        }
        if let Some(mut assets) = app.world_mut().get_resource_mut::<Assets<Shader>>() {
            let _ = assets.insert(
                BILLBOARD_VERTEX.id(),
                Shader::from_wgsl(
                    include_str!("foliage_billboard.wgsl"),
                    "foliage_billboard.wgsl",
                ),
            );
        }
        app.insert_resource(FoliageShaders(shaders)).add_plugins((
            MaterialPlugin::<FoliageMaterial>::default(),
            FoliageInstancePlugin,
        ));
        crate::haze::add_haze_binding::<FoliageMaterial>(app);
        app.add_systems(Startup, make_zero_light).add_systems(
            PostUpdate,
            (push_fade, bind_native).before(AssetEventSystems),
        );
    }
}

impl crate::haze::HazeBound for FoliageMaterial {
    fn haze_buffer(&self) -> &Handle<ShaderBuffer> {
        &self.extension.sun_horizon
    }
    fn set_haze_buffer(&mut self, buffer: Handle<ShaderBuffer>) {
        self.extension.sun_horizon = buffer;
    }
}

fn make_zero_light(buffers: Option<ResMut<Assets<ShaderBuffer>>>) {
    if let Some(mut buffers) = buffers {
        let _ = buffers.insert(
            ZERO_LIGHT.id(),
            ShaderBuffer::from(crate::general_material::GeneralLight::default()),
        );
    }
}

/// Points every foliage material at the shared terrain AO map (`t11`) and the native light
/// buffer once they exist (the General route's shared inputs); writes only what differs.
fn bind_native(
    horizon: Option<Res<HorizonShadow>>,
    fallback: Option<Res<crate::general_material::GeneralFallback>>,
    mut materials: ResMut<Assets<FoliageMaterial>>,
) {
    let Some(inputs) =
        crate::general_material::shared_inputs(horizon.as_deref(), fallback.as_deref())
    else {
        return;
    };
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| {
            m.extension.horizon_map.id() != inputs.map.id()
                || m.extension.light.id() != inputs.light.id()
        })
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.horizon_map = inputs.map.clone();
            m.extension.light = inputs.light.clone();
        }
    }
}

/// Copy the shared native distance fade (`veg_fade::VegetationFade`) into every foliage
/// material that does not have it yet. The constants change only with the camera FOV, so this
/// is a compare per material per frame and a write only after a change or for a new material;
/// no per-plant material is created. Without the resource (model viewer, fixtures) the fade
/// stays inactive.
fn push_fade(
    fade: Option<Res<crate::veg_fade::VegetationFade>>,
    camera: Option<Res<crate::veg_fade::VegetationCamera>>,
    mut materials: ResMut<Assets<FoliageMaterial>>,
) {
    let want = fade.map_or(Vec4::ZERO, |f| f.foliage);
    // the shadow cascades' camera position (`VegetationCamera`)
    let buffer = camera.map(|c| c.0.clone());
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| {
            m.extension.params.fade != want
                || buffer
                    .as_ref()
                    .is_some_and(|b| m.extension.camera.id() != b.id())
        })
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.params.fade = want;
            if let Some(b) = &buffer {
                m.extension.camera = b.clone();
            }
        }
    }
}

fn sync_instances(
    mut commands: Commands,
    mut materials: ResMut<Assets<FoliageMaterial>>,
    mut owned: ResMut<OwnedFoliageMaterials>,
    mut removed: RemovedComponents<FoliageInstance>,
    mut instances: Query<(
        Entity,
        Ref<FoliageInstance>,
        &mut MeshMaterial3d<FoliageMaterial>,
    )>,
    mut reset: Query<&mut MeshMaterial3d<FoliageMaterial>, Without<FoliageInstance>>,
) {
    for entity in removed.read() {
        owned.pending.remove(&entity);
        if let Ok(mut e) = commands.get_entity(entity) {
            e.remove::<MeshTag>();
        }
        if let Some(private) = owned.materials.remove(&entity) {
            if let Ok(mut target) = reset.get_mut(entity) {
                target.0 = private.source;
            }
            materials.remove(private.handle.id());
        }
    }
    for (entity, instance, mut target) in &mut instances {
        if !instance.is_changed() && !owned.pending.contains(&entity) {
            continue;
        }
        commands
            .entity(entity)
            .insert(instance_tag(instance.state.instance_byte));
        let Some(source) = materials.get(&instance.source).cloned() else {
            owned.pending.insert(entity);
            continue;
        };
        owned.pending.remove(&entity);
        if instance.state.raw_flags == source.extension.state.raw_flags {
            target.0 = instance.source.clone();
            if let Some(private) = owned.materials.remove(&entity) {
                materials.remove(private.handle.id());
            }
            continue;
        }
        let extension = source.extension.with_state(instance.state);
        let mut private = source;
        private.extension = extension;
        if let Some(private_handle) = owned.materials.get_mut(&entity)
            && let Some(mut existing) = materials.get_mut(&private_handle.handle)
        {
            *existing = private;
            private_handle.source = instance.source.clone();
            target.0 = private_handle.handle.clone();
            continue;
        }
        let handle = materials.add(private);
        target.0 = handle.clone();
        owned.materials.insert(
            entity,
            OwnedMaterial {
                handle,
                source: instance.source.clone(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;

    /// 7625eb04: the native route's inputs (t1, t2, cb1[1].xy) survive a per-instance copy, and
    /// the lighting flags follow the block's textures.
    #[test]
    fn native_inputs_follow_the_block_and_survive_instance_copies() {
        let state = FoliageAlpha::initialized(64);
        let mut e = FoliageExtension::new(state);
        assert_eq!(e.params.lighting, 0, "a plain material keeps the Bevy route");
        let image: Handle<Image> = Handle::default();
        e.set_native(NativeFoliage {
            normal: Some(image.clone()),
            aoc: None,
            spec_amplitude: 0.15,
            spec_power: 15.0,
        });
        // JC2_NO_NATIVE_FOLIAGE switches LIGHT_NATIVE off for the before captures
        let native = if std::env::var_os("JC2_NO_NATIVE_FOLIAGE").is_some() { 0 } else { LIGHT_NATIVE };
        assert_eq!(e.params.lighting, native | LIGHT_NORMAL_MAP);
        assert_eq!((e.params.spec_amplitude, e.params.spec_power), (0.15, 15.0));
        e.params.fade = Vec4::new(1.0, 2.0, 1.0, 0.0);
        let copy = e.with_state(FoliageAlpha::initialized(0x20));
        assert_eq!(copy.params.lighting, e.params.lighting);
        assert_eq!(copy.params.fade, e.params.fade);
        assert_eq!(copy.params.spec_power, 15.0);
        assert!(copy.normal.is_some() && copy.aoc.is_none());
        assert_eq!(copy.state.raw_flags, 0x20);
    }

    /// The shader carries the native terms of PS 7510628 (constants from its disassembly).
    #[test]
    fn shader_has_the_native_terms() {
        let src = include_str!("foliage_main.wgsl");
        for needle in [
            "vec3<f32>(0.5, 0.6, 0.15)",
            "vec3<f32>(0.508, 0.54, 0.645)",
            "clamp(10.0 * (n_l * dot(n, -v)) - 2.0, -1.0, 1.0) * 0.5 + 0.5",
            "pow(t2r * 3.0, 1.5)",
            "light.ambient_sum.rgb * (t2r * t11.x)",
            "max(shadow, 0.3)",
            "t11.x * 0.5 + 0.5",
        ] {
            assert!(src.contains(needle), "{needle}");
        }
    }

    /// f012a36f: a BillboardFoliage material selects the vertex-stage pipeline key and the native
    /// light route, and every instance copy keeps both.
    #[test]
    fn billboard_materials_select_the_card_vertex_stage_and_survive_instance_copies() {
        let plain = FoliageExtension::new(FoliageAlpha::initialized(64));
        assert!(!FoliageKey::from(&plain).billboard);
        let mut e = FoliageExtension::new(FoliageAlpha::initialized(0));
        e.set_billboard(Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(e.params.billboard, Vec4::new(1.0, 2.0, 3.0, 1.0));
        assert_eq!(e.params.lighting, LIGHT_NATIVE);
        assert!(FoliageKey::from(&e).billboard);
        let copy = e.with_state(FoliageAlpha::initialized(0));
        assert_eq!(copy.params.billboard, e.params.billboard);
        assert!(FoliageKey::from(&copy).billboard);
    }

    /// The billboard shaders carry the native terms of VS / PS `billboardfoliage` (constants from
    /// the disassembly, research/formats/billboard-foliage-next.md).
    #[test]
    fn billboard_shaders_have_the_native_terms() {
        let vs = include_str!("foliage_billboard.wgsl");
        for needle in [
            "fract(t) * 6.28319 - 3.14159",
            "x7 * 0.000198413",
            "x6 * 0.00138889",
            "SWAY_AMPLITUDE: f32 = 0.03",
            "PIVOT_OFFSET: vec3<f32> = vec3<f32>(0.0, 5.0, 0.0)",
            "CROWN_SCALE: vec2<f32> = vec2<f32>(10.0, 10.0)",
            "let right = cross(up, v);",
            "let top = cross(v, right);",
            "0.25 * vc",
            "saturate(length(centroid - world) / CROWN_SCALE.x)",
        ] {
            assert!(vs.contains(needle), "{needle}");
        }
        let ps = include_str!("foliage_main.wgsl");
        for needle in [
            "light.sun.rgb * s + light.ambient_sum.rgb * (0.5 * t11.x)",
            "diffuse * v4.y * lit",
        ] {
            assert!(ps.contains(needle), "{needle}");
        }
        assert!(include_str!("foliage_alpha.wgsl").contains("4.0 * texture_alpha * fraction * fade"));
    }

    #[test]
    fn instance_updates_reset_reuse_and_removal_preserve_shared_source_and_siblings() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            FoliageInstancePlugin,
        ));
        let state = FoliageAlpha::initialized(64);
        let source = app
            .world_mut()
            .resource_mut::<Assets<FoliageMaterial>>()
            .add(source_material(StandardMaterial::default(), state));
        let a = app
            .world_mut()
            .spawn((
                FoliageInstance {
                    source: source.clone(),
                    state,
                },
                MeshMaterial3d(source.clone()),
            ))
            .id();
        let b = app
            .world_mut()
            .spawn((
                FoliageInstance {
                    source: source.clone(),
                    state,
                },
                MeshMaterial3d(source.clone()),
            ))
            .id();
        app.update();
        for byte in [96, 0, 32, 97, 96] {
            app.world_mut()
                .get_mut::<FoliageInstance>(a)
                .unwrap()
                .state
                .raw_flags = byte;
            app.update();
            let handle = &app
                .world()
                .get::<MeshMaterial3d<FoliageMaterial>>(a)
                .unwrap()
                .0;
            assert_ne!(handle.id(), source.id());
            let materials = app.world().resource::<Assets<FoliageMaterial>>();
            assert_eq!(
                materials.get(handle).unwrap().extension.state.raw_flags,
                byte
            );
            assert_eq!(materials.get(&source).unwrap().extension.state, state);
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<FoliageMaterial>>(b)
                    .unwrap()
                    .0
                    .id(),
                source.id()
            );
            assert_eq!(materials.len(), 2);
        }
        app.world_mut().get_mut::<FoliageInstance>(a).unwrap().state = state;
        app.update();
        assert_eq!(app.world().resource::<Assets<FoliageMaterial>>().len(), 1);
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<FoliageMaterial>>(a)
                .unwrap()
                .0
                .id(),
            source.id()
        );
        app.world_mut()
            .get_mut::<FoliageInstance>(b)
            .unwrap()
            .state
            .raw_flags = 96;
        app.update();
        assert_eq!(app.world().resource::<Assets<FoliageMaterial>>().len(), 2);
        app.world_mut().despawn(b);
        app.update();
        assert_eq!(app.world().resource::<Assets<FoliageMaterial>>().len(), 1);
        assert_eq!(
            app.world()
                .resource::<Assets<FoliageMaterial>>()
                .get(&source)
                .unwrap()
                .extension
                .state,
            state
        );
    }
    #[test]
    fn attachment_replacement_and_override_removal_restore_one_material_route_before_last() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            FoliageInstancePlugin,
        ));
        app.init_asset::<StandardMaterial>();
        let state = FoliageAlpha::initialized(64);
        let source = app
            .world_mut()
            .resource_mut::<Assets<FoliageMaterial>>()
            .add(source_material(StandardMaterial::default(), state));
        let standard = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let normal = RbmMaterial {
            standard: standard.clone(),
            foliage: None,
            general: None,
            hemisphere: None,
            skinned_general: None,
        };
        let leaf = RbmMaterial {
            standard: standard.clone(),
            foliage: Some((source.clone(), state)),
            general: None,
            hemisphere: None,
            skinned_general: None,
        };
        let entity = app.world_mut().spawn(normal.clone()).id();
        app.update();
        assert!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(entity)
                .is_some()
        );
        for _ in 0..2 {
            app.world_mut().entity_mut(entity).insert(leaf.clone());
            app.update();
            assert!(
                app.world()
                    .get::<MeshMaterial3d<StandardMaterial>>(entity)
                    .is_none()
            );
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<FoliageMaterial>>(entity)
                    .unwrap()
                    .0
                    .id(),
                source.id()
            );
            app.world_mut()
                .get_mut::<FoliageInstance>(entity)
                .unwrap()
                .state
                .raw_flags = 96;
            app.update();
            assert_eq!(app.world().resource::<Assets<FoliageMaterial>>().len(), 2);
            app.world_mut()
                .entity_mut(entity)
                .remove::<FoliageInstance>();
            app.update();
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<FoliageMaterial>>(entity)
                    .unwrap()
                    .0
                    .id(),
                source.id()
            );
            assert_eq!(app.world().resource::<Assets<FoliageMaterial>>().len(), 1);
            app.world_mut().entity_mut(entity).insert(leaf.clone());
            app.update();
            app.world_mut()
                .get_mut::<FoliageInstance>(entity)
                .unwrap()
                .state
                .raw_flags = 32;
            app.update();
            app.world_mut().entity_mut(entity).insert(normal.clone());
            app.update();
            assert!(
                app.world()
                    .get::<MeshMaterial3d<FoliageMaterial>>(entity)
                    .is_none()
            );
            assert!(app.world().get::<FoliageInstance>(entity).is_none());
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<StandardMaterial>>(entity)
                    .unwrap()
                    .0
                    .id(),
                standard.id()
            );
            assert_eq!(app.world().resource::<Assets<FoliageMaterial>>().len(), 1);
            assert_eq!(
                app.world()
                    .resource::<Assets<FoliageMaterial>>()
                    .get(&source)
                    .unwrap()
                    .extension
                    .state,
                state
            );
        }
    }
    #[test]
    fn delayed_source_asset_retries_an_unchanged_instance_without_mutating_the_source() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            FoliageInstancePlugin,
        ));
        let state = FoliageAlpha::initialized(64);
        let source = app
            .world()
            .resource::<Assets<FoliageMaterial>>()
            .reserve_handle();
        let mut changed = state;
        changed.raw_flags = 96;
        let entity = app
            .world_mut()
            .spawn((
                FoliageInstance {
                    source: source.clone(),
                    state: changed,
                },
                MeshMaterial3d(source.clone()),
            ))
            .id();
        app.update();
        app.update();
        assert_eq!(
            app.world()
                .resource::<OwnedFoliageMaterials>()
                .pending
                .len(),
            1
        );
        app.world_mut()
            .resource_mut::<Assets<FoliageMaterial>>()
            .insert(
                source.id(),
                source_material(StandardMaterial::default(), state),
            )
            .unwrap();
        app.update();
        assert!(
            app.world()
                .resource::<OwnedFoliageMaterials>()
                .pending
                .is_empty()
        );
        let handle = &app
            .world()
            .get::<MeshMaterial3d<FoliageMaterial>>(entity)
            .unwrap()
            .0;
        assert_ne!(handle.id(), source.id());
        assert_eq!(
            app.world()
                .resource::<Assets<FoliageMaterial>>()
                .get(handle)
                .unwrap()
                .extension
                .state,
            changed
        );
        assert_eq!(
            app.world()
                .resource::<Assets<FoliageMaterial>>()
                .get(&source)
                .unwrap()
                .extension
                .state,
            state
        );
        app.world_mut()
            .entity_mut(entity)
            .remove::<FoliageInstance>();
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<FoliageMaterial>>(entity)
                .unwrap()
                .0
                .id(),
            source.id()
        );
    }
    #[test]
    fn shared_discard_wgsl_validates_and_both_passes_import_the_same_contract() {
        let source = include_str!("foliage_alpha.wgsl")
            .lines()
            .filter(|line| !line.starts_with("#define_import_path") && !line.starts_with("#import"))
            .collect::<Vec<_>>()
            .join("\n")
            .replace("mesh_functions::", "")
            .replace("#{MATERIAL_BIND_GROUP}", "2");
        // stand-ins for the Bevy view and mesh bindings the real module imports
        let source = format!(
            "struct View {{ clip_from_view: mat4x4<f32>, world_position: vec3<f32> }}\n\
             @group(0) @binding(0) var<uniform> view: View;\n\
             fn get_world_from_local(i: u32) -> mat4x4<f32> {{ return mat4x4<f32>(); }}\n\
             fn get_tag(i: u32) -> u32 {{ return 0u; }}\n{source}"
        );
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        for shader in [
            include_str!("foliage_main.wgsl"),
            include_str!("foliage_prepass.wgsl"),
        ] {
            // (the main pass also imports the native-lighting accessors)
            assert!(shader.contains(
                "#import jc2::foliage_alpha::{foliage_should_discard, foliage_distance_fade"
            ));
            assert!(
                shader.contains("foliage_distance_fade(in.world_position.xyz, in.instance_index)")
            );
            assert!(!shader.contains("prepass_alpha_discard("));
            assert!(!shader.contains("= alpha_discard("));
        }
    }

    /// JC2-153: the instance byte is the entity's `MeshTag` (`255 - byte`), no private
    /// material; removing the component restores the untagged (byte 255) default.
    #[test]
    fn the_instance_byte_rides_in_the_mesh_tag_without_a_private_material() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            FoliageInstancePlugin,
        ));
        let state = FoliageAlpha::initialized(64);
        let source = app
            .world_mut()
            .resource_mut::<Assets<FoliageMaterial>>()
            .add(source_material(StandardMaterial::default(), state));
        let e = app
            .world_mut()
            .spawn((
                FoliageInstance {
                    source: source.clone(),
                    state,
                },
                MeshMaterial3d(source.clone()),
            ))
            .id();
        app.update();
        assert_eq!(app.world().get::<MeshTag>(e).unwrap().0, 0);
        for byte in [0u8, 125, 1, 253, 255] {
            app.world_mut()
                .get_mut::<FoliageInstance>(e)
                .unwrap()
                .state
                .instance_byte = byte;
            app.update();
            assert_eq!(
                app.world().get::<MeshTag>(e).unwrap().0,
                255 - u32::from(byte)
            );
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<FoliageMaterial>>(e)
                    .unwrap()
                    .0
                    .id(),
                source.id()
            );
            assert_eq!(app.world().resource::<Assets<FoliageMaterial>>().len(), 1);
        }
        app.world_mut().entity_mut(e).remove::<FoliageInstance>();
        app.update();
        assert!(app.world().get::<MeshTag>(e).is_none());
    }
}
