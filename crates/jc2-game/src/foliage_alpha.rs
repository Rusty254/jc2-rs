//! Ordinary foliage discard shared by main, depth and shadow. Native flags and
//! instance-byte arithmetic are supported; per-asset distance scheduling is086.
use bevy::asset::{AssetApp, AssetEventSystems, AssetPath, embedded_path, load_embedded_asset};
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::system::EntityCommands;
use bevy::ecs::world::DeferredWorld;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use jc2_formats::foliage::FoliageAlpha;
use std::collections::{HashMap, HashSet};

pub type FoliageMaterial = ExtendedMaterial<StandardMaterial, FoliageExtension>;

#[derive(Clone, Debug, ShaderType)]
pub struct FoliageParams {
    enabled: u32,
    instance_fraction: f32,
    reference: f32,
    reserved: f32,
}
impl From<FoliageAlpha> for FoliageParams {
    fn from(alpha: FoliageAlpha) -> Self {
        Self {
            enabled: u32::from(alpha.enabled()),
            instance_fraction: alpha.fraction(),
            reference: alpha.reference(),
            reserved: 0.0,
        }
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FoliageExtension {
    #[uniform(100)]
    params: FoliageParams,
    pub state: FoliageAlpha,
}
impl FoliageExtension {
    pub fn new(state: FoliageAlpha) -> Self {
        Self {
            params: state.into(),
            state,
        }
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
}

pub fn source_material(mut base: StandardMaterial, state: FoliageAlpha) -> FoliageMaterial {
    // Masked routing invokes a fragment shader in main/depth/shadow. Our shared
    // discard replaces StandardMaterial cutoff; no alpha-to-coverage is used.
    base.alpha_mode = AlphaMode::Mask(0.0);
    base.opaque_render_method = bevy::material::OpaqueRendererMethod::Forward;
    FoliageMaterial {
        base,
        extension: FoliageExtension::new(state),
    }
}

/// Only explicit proven instance-byte updates/fixtures change this state. The
/// world does not synthesize a byte from Bevy VisibilityRange distance.
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
                FoliageInstance,
            )>()
            .insert(MeshMaterial3d(general));
    } else {
        entity
            .remove::<(
                MeshMaterial3d<FoliageMaterial>,
                MeshMaterial3d<crate::general_material::GeneralMaterial>,
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
        app.insert_resource(FoliageShaders(shaders)).add_plugins((
            MaterialPlugin::<FoliageMaterial>::default(),
            FoliageInstancePlugin,
        ));
    }
}

fn sync_instances(
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
        let Some(source) = materials.get(&instance.source).cloned() else {
            owned.pending.insert(entity);
            continue;
        };
        owned.pending.remove(&entity);
        if instance.state == source.extension.state {
            target.0 = instance.source.clone();
            if let Some(private) = owned.materials.remove(&entity) {
                materials.remove(private.handle.id());
            }
            continue;
        }
        let mut private = source;
        private.extension = FoliageExtension::new(instance.state);
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
        for byte in [0, 125, 1, 253, 0] {
            app.world_mut()
                .get_mut::<FoliageInstance>(a)
                .unwrap()
                .state
                .instance_byte = byte;
            app.update();
            let handle = &app
                .world()
                .get::<MeshMaterial3d<FoliageMaterial>>(a)
                .unwrap()
                .0;
            assert_ne!(handle.id(), source.id());
            let materials = app.world().resource::<Assets<FoliageMaterial>>();
            assert_eq!(
                materials.get(handle).unwrap().extension.state.instance_byte,
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
            .instance_byte = 0;
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
        };
        let leaf = RbmMaterial {
            standard: standard.clone(),
            foliage: Some((source.clone(), state)),
            general: None,
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
                .instance_byte = 0;
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
                .instance_byte = 125;
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
        changed.instance_byte = 0;
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
            .filter(|line| !line.starts_with("#define_import_path"))
            .collect::<Vec<_>>()
            .join("\n")
            .replace("#{MATERIAL_BIND_GROUP}", "2");
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
            assert!(shader.contains("#import jc2::foliage_alpha::foliage_should_discard"));
            assert!(!shader.contains("prepass_alpha_discard("));
            assert!(!shader.contains("= alpha_discard("));
        }
    }
}
