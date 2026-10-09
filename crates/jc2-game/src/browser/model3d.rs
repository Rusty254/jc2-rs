//! The browser's 3D preview: a camera that renders into an image (which egui draws in the
//! right-hand pane), on its own render layer so nothing else in the app shows up in it.
//! Which models appear is decided by `viewer::model_members` — the same rules as
//! `--model`: a vehicle assembled from its doll, a container's models of one LOD level, or a
//! single `.rbm`.

use crate::Game;
use crate::rbm_loader::RbmAsset;
use crate::viewer::{self, OrbitCamera};
use bevy::asset::LoadState;
use bevy::camera::{RenderTarget, visibility::RenderLayers};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDescriptor, TextureDimension, TextureFormat, TextureUsages,
};
use bevy_egui::{EguiTextureHandle, EguiUserTextures, egui};

/// The size of the image the preview camera renders into (shown scaled to fit).
pub const VIEW_SIZE: (u32, u32) = (1280, 800);
/// Models spawned for one container at most (an area set holds thousands).
pub const MAX_MODELS: usize = 150;
const LAYER: usize = 1;

#[derive(Component)]
pub struct PreviewCamera;

#[derive(Component)]
struct PreviewRoot;

struct Part {
    name: String,
    handle: Handle<RbmAsset>,
    transform: Transform,
    spawned: bool,
    failed: bool,
}

#[derive(Resource)]
pub struct ModelPreview {
    pub texture: egui::TextureId,
    /// What the right pane wants shown: a `--model` argument. The system below makes the
    /// scene match.
    pub wanted: Option<String>,
    /// LOD level for containers: "1", "2", "3", "4" or "all".
    pub lod: String,
    shown: Option<(String, String)>,
    root: Option<Entity>,
    parts: Vec<Part>,
    bbox: Option<(Vec3, Vec3)>,
    framed: bool,
    pub from_doll: bool,
    /// Asked by the UI: put the camera back on the whole model.
    pub reframe: bool,
    /// Parts selected / loaded / failed, for the toolbar.
    pub counts: (usize, usize, usize),
    pub truncated: bool,
}

pub struct ModelPreviewPlugin;

impl Plugin for ModelPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (sync_scene, spawn_loaded, activate).chain());
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut user_textures: ResMut<EguiUserTextures>,
) {
    let size = Extent3d {
        width: VIEW_SIZE.0,
        height: VIEW_SIZE.1,
        ..default()
    };
    let mut image = Image {
        texture_descriptor: TextureDescriptor {
            label: Some("browser model preview"),
            size,
            dimension: TextureDimension::D2,
            format: TextureFormat::Bgra8UnormSrgb,
            mip_level_count: 1,
            sample_count: 1,
            usage: TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST
                | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        },
        ..default()
    };
    image.resize(size);
    let handle = images.add(image);
    let texture = user_textures.add_image(EguiTextureHandle::Strong(handle.clone()));
    commands.insert_resource(ModelPreview {
        texture,
        wanted: None,
        lod: "1".into(),
        shown: None,
        root: None,
        parts: Vec::new(),
        bbox: None,
        framed: false,
        from_doll: false,
        reframe: false,
        counts: (0, 0, 0),
        truncated: false,
    });

    let layer = RenderLayers::layer(LAYER);
    commands.spawn((
        PreviewCamera,
        Camera3d::default(),
        Camera {
            // before the camera that draws egui
            order: -1,
            // off until a model is shown
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.085, 0.09, 0.105)),
            ..default()
        },
        RenderTarget::Image(handle.into()),
        Transform::from_xyz(0.0, 3.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
        OrbitCamera {
            target: Vec3::ZERO,
            yaw: 0.6,
            pitch: 0.35,
            distance: 15.0,
        },
        AmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        },
        layer.clone(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0)),
        layer.clone(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 2_500.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.6, -2.4, 0.0)),
        layer,
    ));
}

/// Make the scene match `wanted` + `lod`: drop the old models, load the new ones.
fn sync_scene(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    mut preview: ResMut<ModelPreview>,
) {
    let want = preview.wanted.clone().map(|w| (w, preview.lod.clone()));
    if want == preview.shown {
        return;
    }
    if let Some(root) = preview.root.take() {
        commands.entity(root).despawn();
    }
    preview.parts.clear();
    preview.bbox = None;
    preview.framed = false;
    preview.from_doll = false;
    preview.truncated = false;
    preview.counts = (0, 0, 0);
    preview.shown = want.clone();
    let Some((model, lod)) = want else { return };
    let set = viewer::model_members(&game.fs, &model, &lod, &[]);
    preview.from_doll = set.from_doll;
    let total = set.members.len();
    preview.truncated = total > MAX_MODELS;
    for (name, transform) in set.members.into_iter().take(MAX_MODELS) {
        let path = viewer::asset_path(&set.container, &name);
        let handle: Handle<RbmAsset> = assets.load(path);
        preview.parts.push(Part {
            name,
            handle,
            transform,
            spawned: false,
            failed: false,
        });
    }
    preview.counts = (preview.parts.len(), 0, 0);
    preview.root = Some(
        commands
            .spawn((
                PreviewRoot,
                Transform::IDENTITY,
                Visibility::default(),
                RenderLayers::layer(LAYER),
                Name::new("browser preview"),
            ))
            .id(),
    );
}

fn spawn_loaded(
    mut commands: Commands,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    mut preview: ResMut<ModelPreview>,
    mut cams: Query<(&mut OrbitCamera, &mut Transform), With<PreviewCamera>>,
) {
    let Some(root) = preview.root else { return };
    let mut bbox = preview.bbox;
    let (mut loaded, mut failed) = (0, 0);
    for p in preview.parts.iter_mut() {
        if p.failed {
            failed += 1;
            continue;
        }
        if p.spawned {
            loaded += 1;
            continue;
        }
        if let Some(LoadState::Failed(err)) = assets.get_load_state(p.handle.id()) {
            warn!("browser preview: {}: {err}", p.name);
            p.failed = true;
            failed += 1;
            continue;
        }
        let Some(rbm) = rbms.get(&p.handle) else {
            continue;
        };
        commands.entity(root).with_children(|parent| {
            parent
                .spawn((
                    p.transform,
                    Visibility::default(),
                    RenderLayers::layer(LAYER),
                    Name::new(p.name.clone()),
                ))
                .with_children(|part| {
                    for b in &rbm.blocks {
                        part.spawn((
                            Mesh3d(b.mesh.clone()),
                            b.material_component(),
                            Transform::IDENTITY,
                            RenderLayers::layer(LAYER),
                        ));
                    }
                });
        });
        let corners = [rbm.bbox_min, rbm.bbox_max];
        for k in 0..8 {
            let c = Vec3::new(
                corners[k & 1].x,
                corners[(k >> 1) & 1].y,
                corners[(k >> 2) & 1].z,
            );
            let w = p.transform.transform_point(c);
            bbox = Some(match bbox {
                None => (w, w),
                Some((lo, hi)) => (lo.min(w), hi.max(w)),
            });
        }
        p.spawned = true;
        loaded += 1;
    }
    preview.bbox = bbox;
    let total = preview.parts.len();
    preview.counts = (total, loaded, failed);
    let done = loaded + failed == total;
    if (done && !preview.framed || preview.reframe)
        && let Some((lo, hi)) = preview.bbox
    {
        for (mut cam, mut tf) in &mut cams {
            viewer::frame(&mut cam, lo, hi);
            viewer::apply(&cam, &mut tf);
        }
        preview.framed = done;
        preview.reframe = false;
    }
}

/// The preview camera only renders while a model is shown.
fn activate(preview: Res<ModelPreview>, mut cams: Query<&mut Camera, With<PreviewCamera>>) {
    let on = preview.wanted.is_some();
    for mut c in &mut cams {
        if c.is_active != on {
            c.is_active = on;
        }
    }
}
