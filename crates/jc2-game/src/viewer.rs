//! Milestone 2: the model viewer. Loads every `.rbm` of one LOD level from a
//! container, spawns the blocks, and gives you an orbit camera. A vehicle container
//! (one with an `.mvdoll`) is assembled from its doll instead: every part — body,
//! doors, the shared tyres, seats and steering wheel — at its own place.
//!
//! Left-drag orbits, right-drag pans, wheel zooms, `F` re-frames. `--camera-yaw` and
//! `--camera-pitch` (degrees) start the camera at a fixed angle: yaw 0 sits on +z looking
//! along −z, i.e. behind a vehicle (whose nose is −z, `coords.rs`).

use crate::Game;
use crate::coords::mat_to_bevy;
use crate::rbm_loader::RbmAsset;
use bevy::asset::LoadState;
use bevy::ecs::system::SystemParam;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use jc2_formats::GameFs;
use jc2_formats::lod::LodDescriptor;
use jc2_formats::sarc::SmallArchive;
use jc2_formats::vehicle::VehicleDoll;
use std::collections::HashSet;

#[derive(Resource, Clone)]
pub struct ViewerConfig {
    /// Scripted material capture: preserve the framed camera and record its state.
    pub lock_capture: bool,
    /// Diagnostic receiver/shadow setup; ordinary viewing keeps its existing lighting.
    pub material_check: bool,
    pub depth_prepass: bool,
    pub normal_maps: bool,
    pub light_yaw: Option<f32>,
    /// `container.ee` (all .rbm members matching `lod`) or `container.ee/model.rbm`
    pub model: String,
    /// "1", "3", ... or "all"
    pub lod: String,
    /// The orbit camera's starting yaw and pitch (radians), when given.
    pub camera_yaw: Option<f32>,
    pub camera_pitch: Option<f32>,
    /// Doll parts left out (lower-case name fragments).
    pub hide_parts: Vec<String>,
}

struct Entry {
    name: String,
    handle: Handle<RbmAsset>,
    transform: Transform,
    spawned: bool,
}

#[derive(Resource, Default)]
struct Loading {
    entries: Vec<Entry>,
    failed: HashSet<AssetId<RbmAsset>>,
    bbox: Option<(Vec3, Vec3)>,
    framed: bool,
    /// Parts placed from a vehicle doll (otherwise the container's models at the origin).
    from_doll: bool,
}

#[derive(Component)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

#[derive(Component)]
struct ModelPart;

#[derive(Component)]
struct Hud;

pub struct ViewerPlugin;

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Loading>()
            .add_systems(Startup, setup)
            .add_systems(Update, (spawn_loaded, orbit_camera, hud));
    }
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    game: Res<Game>,
    cfg: Res<ViewerConfig>,
    mut loading: ResMut<Loading>,
) {
    let camera = commands
        .spawn((
            Camera3d::default(),
            Transform::from_xyz(0.0, 3.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
            OrbitCamera {
                target: Vec3::ZERO,
                yaw: cfg.camera_yaw.unwrap_or(0.6),
                pitch: cfg.camera_pitch.unwrap_or(0.35),
                distance: 15.0,
            },
            AmbientLight {
                color: Color::WHITE,
                brightness: if cfg.material_check { 80.0 } else { 300.0 },
                ..default()
            },
        ))
        .id();
    if cfg.depth_prepass {
        commands
            .entity(camera)
            .insert(bevy::core_pipeline::prepass::DepthPrepass);
    }
    commands.spawn((
        DirectionalLight {
            illuminance: 8_000.0,
            shadow_maps_enabled: cfg.material_check,
            ..default()
        },
        Transform::from_rotation(cfg.light_yaw.map_or_else(
            || Quat::from_euler(EulerRot::XYZ, -0.9, 0.5, 0.0),
            // Rotate the tilted light around world up so a yaw comparison retains
            // its elevation instead of sending the light below the receiver.
            |yaw| Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-0.9),
        )),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: if cfg.material_check { 400.0 } else { 2_500.0 },
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.6, -2.4, 0.0)),
    ));
    commands.spawn((
        Text::new("loading…"),
        TextFont::from_font_size(14.0),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(10.0),
            top: Val::Px(8.0),
            ..default()
        },
        Hud,
    ));

    let set = model_members(&game.fs, &cfg.model, &cfg.lod, &cfg.hide_parts);
    let container = set.container;
    loading.from_doll = set.from_doll;
    let members = set.members;
    if members.is_empty() {
        error!("no .rbm models found in {}", cfg.model);
    }
    for (m, transform) in members {
        let path = asset_path(&container, &m);
        info!("loading {path}");
        let handle: Handle<RbmAsset> = assets.load(path);
        loading.entries.push(Entry {
            name: m,
            handle,
            transform,
            spawned: false,
        });
    }
}

/// The models `--model <model>` puts on screen.
pub struct ModelSet {
    /// The container (or bare model) the names below live in.
    pub container: String,
    /// (member name, placement) pairs to load from `container`.
    pub members: Vec<(String, Transform)>,
    /// The parts were placed from a vehicle doll.
    pub from_doll: bool,
}

/// What `--model <model>` shows: one member (`container.ee/model.rbm`), or a whole
/// container — a vehicle assembled from its doll, else the models its `.lod` descriptors
/// pick for `lod`, else every `.rbm` of that level. The browser's 3D preview uses this too.
pub fn model_members(fs: &GameFs, model: &str, lod: &str, hide_parts: &[String]) -> ModelSet {
    let mut from_doll = false;
    let model = model.replace('\\', "/");
    let (container, member) =
        crate::archive_source::JcAssetReader::split(std::path::Path::new(&model));
    // (member name, transform) pairs to load from `container`
    let mut members: Vec<(String, Transform)> = Vec::new();
    match member {
        Some(m) => members.push((m, Transform::IDENTITY)),
        None => match fs.read_inflated(&container) {
            Ok(data) if jc2_formats::sarc::is_sarc(&data) => {
                if let Ok(s) = SmallArchive::parse(&data) {
                    let tag = format!("lod{}", lod);
                    let level: Option<usize> = lod.parse().ok();
                    // A vehicle: assemble it from its doll, parts at their places.
                    let doll = s
                        .entries
                        .iter()
                        .find(|e| e.name.to_ascii_lowercase().ends_with(".mvdoll"))
                        .and_then(|e| s.data(&data, e).ok())
                        .and_then(|d| VehicleDoll::parse(d).ok());
                    if let (Some(doll), Some(level)) = (&doll, level) {
                        for part in &doll.parts {
                            let name = part.name.to_ascii_lowercase();
                            if part.model.is_empty()
                                || hide_parts.iter().any(|h| name.contains(h.as_str()))
                            {
                                continue;
                            }

                            let lod_name = part.model_file();
                            let lod_text = match s.get(lod_name) {
                                Some(e) => s.data(&data, e).ok().map(|b| b.to_vec()),
                                None => fs.read_inflated(lod_name).ok(),
                            };
                            let Some(text) = lod_text else {
                                warn!("{}: {lod_name} not found", part.name);
                                continue;
                            };
                            let d = LodDescriptor::parse(&String::from_utf8_lossy(&text));
                            let Some(model) = d.model_for_level(level) else {
                                continue;
                            };
                            let mut tf = mat_to_bevy(&part.world);
                            if let Some(sc) = part.scale {
                                tf.scale *= Vec3::from(sc);
                            }
                            members.push((model.to_string(), tf));
                        }
                        from_doll = true;
                        info!(
                            "{container}: {} parts placed from the vehicle doll",
                            members.len()
                        );
                    }
                    // Otherwise the container's .lod descriptors say which model draws at each level.
                    if members.is_empty()
                        && let Some(level) = level
                    {
                        for e in &s.entries {
                            if !e.name.to_ascii_lowercase().ends_with(".lod") {
                                continue;
                            }
                            let Ok(bytes) = s.data(&data, e) else {
                                continue;
                            };
                            let d = LodDescriptor::parse(&String::from_utf8_lossy(bytes));
                            let Some(model) = d.model_for_level(level) else {
                                continue;
                            };
                            if s.get(model).is_some() || fs.contains(model) {
                                if !members.iter().any(|(m, _)| m.eq_ignore_ascii_case(model)) {
                                    members.push((model.to_string(), Transform::IDENTITY));
                                }
                            } else {
                                warn!("{}: {model} not found", e.name);
                            }
                        }
                    }
                    if members.is_empty() {
                        for e in &s.entries {
                            let lower = e.name.to_ascii_lowercase();
                            if lower.ends_with(".rbm") && (lod == "all" || lower.contains(&tag)) {
                                members.push((e.name.clone(), Transform::IDENTITY));
                            }
                        }
                    }
                    if members.is_empty() {
                        // no LOD tag in the names: take everything
                        members.extend(
                            s.entries
                                .iter()
                                .filter(|e| e.name.to_ascii_lowercase().ends_with(".rbm"))
                                .map(|e| (e.name.clone(), Transform::IDENTITY)),
                        );
                    }
                }
            }
            Ok(_) => {
                // a bare top-level .rbm
                members.push((container.clone(), Transform::IDENTITY));
            }
            Err(e) => error!("cannot read {container}: {e}"),
        },
    }
    ModelSet {
        container,
        members,
        from_doll,
    }
}

/// The `jc2://` path of a member of `container` (or of the bare model).
pub fn asset_path(container: &str, member: &str) -> String {
    if member == container {
        format!("jc2://{member}")
    } else {
        format!("jc2://{container}/{member}")
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_loaded(
    mut commands: Commands,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    cfg: Res<ViewerConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut loading: ResMut<Loading>,
    mut cams: Query<(&mut OrbitCamera, &mut Transform)>,
) {
    let mut bbox = loading.bbox;
    let mut failed = std::mem::take(&mut loading.failed);
    for e in loading.entries.iter_mut() {
        let id = e.handle.id();
        if e.spawned || failed.contains(&id) {
            continue;
        }
        if let Some(LoadState::Failed(err)) = assets.get_load_state(id) {
            error!("{}: {err}", e.name);
            failed.insert(id);
            continue;
        }
        let Some(rbm) = rbms.get(&e.handle) else {
            continue;
        };
        // A comparison must wait for labeled material assets; otherwise it could
        // silently spawn the original normal-mapped material for one frame's race.
        if (cfg.material_check || !cfg.normal_maps)
            && rbm
                .blocks
                .iter()
                .any(|b| materials.get(&b.material).is_none())
        {
            continue;
        }
        if cfg.material_check {
            let normal_count = rbm
                .blocks
                .iter()
                .filter(|b| {
                    materials
                        .get(&b.material)
                        .is_some_and(|m| m.normal_map_texture.is_some())
                })
                .count();
            println!(
                "material-check model={} blocks={} source_normal_maps={} comparison_normals={}",
                e.name,
                rbm.blocks.len(),
                normal_count,
                cfg.normal_maps
            );
        }
        if let Some(t) = &rbm.truncated_at {
            warn!("{}: stopped at unsupported block type {t}", e.name);
        }
        commands
            .spawn((
                e.transform,
                Visibility::default(),
                ModelPart,
                Name::new(e.name.clone()),
            ))
            .with_children(|parent| {
                for b in &rbm.blocks {
                    let material = if cfg.normal_maps {
                        b.material.clone()
                    } else if let Some(source) = materials.get(&b.material) {
                        let mut comparison = source.clone();
                        comparison.normal_map_texture = None;
                        materials.add(comparison)
                    } else {
                        b.material.clone()
                    };
                    parent.spawn((
                        Mesh3d(b.mesh.clone()),
                        b.material_component_with(material),
                        Transform::IDENTITY,
                        Name::new(b.type_name.clone()),
                    ));
                }
            });
        let textures: Vec<&str> = rbm
            .blocks
            .iter()
            .map(|b| b.diffuse.as_str())
            .filter(|t| !t.is_empty())
            .collect();
        info!(
            "{}: {} blocks spawned, textures {}",
            e.name,
            rbm.blocks.len(),
            textures.join(" ")
        );
        // bounding box of the placed model
        let corners = [rbm.bbox_min, rbm.bbox_max];
        for k in 0..8 {
            let c = Vec3::new(
                corners[k & 1].x,
                corners[(k >> 1) & 1].y,
                corners[(k >> 2) & 1].z,
            );
            let w = e.transform.transform_point(c);
            bbox = Some(match bbox {
                None => (w, w),
                Some((lo, hi)) => (lo.min(w), hi.max(w)),
            });
        }
        e.spawned = true;
    }
    loading.failed = failed;
    loading.bbox = bbox;
    let done = loading
        .entries
        .iter()
        .all(|e| e.spawned || loading.failed.contains(&e.handle.id()));
    if done && !loading.framed {
        if let Some((lo, hi)) = loading.bbox {
            if cfg.material_check {
                let extent = (hi - lo).max_element().max(1.0);
                commands.spawn((
                    Mesh3d(meshes.add(Plane3d::default().mesh().size(extent * 5.0, extent * 5.0))),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: Color::srgb(0.55, 0.55, 0.55),
                        perceptual_roughness: 1.0,
                        ..default()
                    })),
                    Transform::from_xyz((lo.x + hi.x) * 0.5, lo.y - 0.02, (lo.z + hi.z) * 0.5),
                    bevy::light::NotShadowCaster,
                    Name::new("material check shadow receiver"),
                ));
                println!(
                    "material check: depth_prepass={} normal_maps={} model bounds={lo:?}..{hi:?}",
                    cfg.depth_prepass, cfg.normal_maps
                );
            }
            for (mut cam, mut tf) in &mut cams {
                frame(&mut cam, lo, hi);
                // capture aid: `JC2_VIEWER_ZOOM=0.4` moves the framed camera closer (before/after
                // captures of texture magnification); it changes nothing without the variable
                if let Some(zoom) = std::env::var("JC2_VIEWER_ZOOM")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .filter(|z| *z > 0.0)
                {
                    cam.distance *= zoom;
                }
                apply(&cam, &mut tf);
            }
        }
        loading.framed = true;
    }
}

pub(crate) fn frame(cam: &mut OrbitCamera, lo: Vec3, hi: Vec3) {
    cam.target = (lo + hi) * 0.5;
    cam.distance = ((hi - lo).length() * 0.9).max(0.5);
}

pub(crate) fn apply(cam: &OrbitCamera, tf: &mut Transform) {
    let rot = Quat::from_euler(EulerRot::YXZ, cam.yaw, -cam.pitch, 0.0);
    let offset = rot * Vec3::new(0.0, 0.0, cam.distance);
    tf.translation = cam.target + offset;
    tf.look_at(cam.target, Vec3::Y);
}

fn orbit_camera(
    cfg: Res<ViewerConfig>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    loading: Res<Loading>,
    mut cams: Query<(&mut OrbitCamera, &mut Transform)>,
) {
    if cfg.lock_capture {
        return;
    }
    for (mut cam, mut tf) in &mut cams {
        let d = motion.delta;
        if buttons.pressed(MouseButton::Left) && d != Vec2::ZERO {
            cam.yaw -= d.x * 0.006;
            cam.pitch = (cam.pitch + d.y * 0.006).clamp(-1.5, 1.5);
        }
        if buttons.pressed(MouseButton::Right) && d != Vec2::ZERO {
            let right = tf.right();
            let up = tf.up();
            let k = cam.distance * 0.0015;
            cam.target += (-right * d.x + up * d.y) * k;
        }
        if scroll.delta.y != 0.0 {
            let step = match scroll.unit {
                MouseScrollUnit::Line => scroll.delta.y * 0.12,
                MouseScrollUnit::Pixel => scroll.delta.y * 0.004,
            };
            cam.distance = (cam.distance * (1.0 - step)).clamp(0.2, 2000.0);
        }
        if keys.just_pressed(KeyCode::KeyF)
            && let Some((lo, hi)) = loading.bbox
        {
            frame(&mut cam, lo, hi);
        }
        apply(&cam, &mut tf);
    }
}

/// Snapshot at screenshot request time. In locked mode the camera remains unchanged
/// while the GPU capture completes; other viewer modes produce no extra receipt.
#[derive(SystemParam)]
pub(crate) struct MaterialCapture<'w, 's> {
    cfg: Option<Res<'w, ViewerConfig>>,
    loading: Option<Res<'w, Loading>>,
    assets: Option<Res<'w, AssetServer>>,
    cameras: Query<
        'w,
        's,
        (
            &'static OrbitCamera,
            &'static Transform,
            &'static Projection,
        ),
    >,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

impl MaterialCapture<'_, '_> {
    pub(crate) fn snapshot(&self) -> Option<serde_json::Value> {
        let cfg = self.cfg.as_ref().filter(|cfg| cfg.lock_capture)?;
        let loading = self.loading.as_ref()?;
        let Ok((camera, transform, projection)) = self.cameras.single() else {
            return Some(serde_json::json!({"ready": false, "error": "expected one orbit camera"}));
        };
        let dependencies_ready = self.assets.as_ref().is_some_and(|assets| {
            loading
                .entries
                .iter()
                .all(|entry| entry.spawned && assets.is_loaded_with_dependencies(entry.handle.id()))
        });
        let viewport = self.windows.single().ok().map(|window| {
            serde_json::json!({
                "width": window.physical_width(), "height": window.physical_height(),
                "scale_factor": window.scale_factor(),
            })
        });
        Some(serde_json::json!({
            "version": 1,
            "locked": true,
            "ready": loading.framed && !loading.entries.is_empty()
                && loading.failed.is_empty() && dependencies_ready && viewport.is_some(),
            "models": loading.entries.len(),
            "failed_models": loading.failed.len(),
            "dependencies_ready": dependencies_ready,
            "model": cfg.model, "lod": cfg.lod, "hide_parts": cfg.hide_parts,
            "camera": {
                "target": camera.target.to_array(), "yaw": camera.yaw, "pitch": camera.pitch,
                "distance": camera.distance, "translation": transform.translation.to_array(),
                "rotation": transform.rotation.to_array(), "scale": transform.scale.to_array(),
                "projection": format!("{projection:?}"), "viewport": viewport,
            },
            "settings": {"depth_prepass": cfg.depth_prepass,
                "normal_maps": cfg.normal_maps, "light_yaw": cfg.light_yaw},
        }))
    }
}

fn hud(
    loading: Res<Loading>,
    cfg: Res<ViewerConfig>,
    rbms: Res<Assets<RbmAsset>>,
    mut q: Query<&mut Text, With<Hud>>,
) {
    if !loading.is_changed() {
        return;
    }
    let mut tris = 0usize;
    let mut blocks = 0usize;
    let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
    for e in &loading.entries {
        if let Some(r) = rbms.get(&e.handle) {
            for b in &r.blocks {
                tris += b.triangles;
                blocks += 1;
                *kinds.entry(b.type_name.clone()).or_default() += 1;
            }
        }
    }
    let kinds: Vec<String> = kinds.into_iter().map(|(k, v)| format!("{k}×{v}")).collect();
    for mut t in &mut q {
        **t = format!(
            "jc2-rs model viewer   {}  (lod {}){}\n{} / {} models loaded{}   {} blocks, {} triangles   [{}]\nleft-drag orbit · right-drag pan · wheel zoom · F frame",
            cfg.model,
            cfg.lod,
            if loading.from_doll {
                "   assembled from the vehicle doll"
            } else {
                ""
            },
            loading.entries.iter().filter(|e| e.spawned).count(),
            loading.entries.len(),
            if loading.failed.is_empty() {
                String::new()
            } else {
                format!(", {} failed", loading.failed.len())
            },
            blocks,
            tris,
            kinds.join(" ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Needs a game folder: set JC2_TEST_DIR.
    #[test]
    fn model_members_assembles_a_vehicle_and_finds_one_model() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        // a vehicle: every doll part at its own place
        let set = model_members(&fs, "lave.v046_buscoach.ee", "1", &[]);
        assert_eq!(set.container, "lave.v046_buscoach.ee");
        assert!(set.from_doll);
        assert!(set.members.len() >= 8, "{}", set.members.len());
        // parts left out by name
        let fewer = model_members(&fs, "lave.v046_buscoach.ee", "1", &["door".to_string()]);
        assert!(fewer.members.len() < set.members.len());
        // one model of a container
        let one = model_members(&fs, "lave.v046_buscoach.ee/v046-body_m_lod1.rbm", "1", &[]);
        assert_eq!(one.members.len(), 1);
        assert!(!one.from_doll);
        assert_eq!(
            asset_path(&one.container, &one.members[0].0),
            "jc2://lave.v046_buscoach.ee/v046-body_m_lod1.rbm"
        );
        assert_eq!(asset_path("a.rbm", "a.rbm"), "jc2://a.rbm");
        let nested = model_members(
            &fs,
            "climate/zone_1/vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm",
            "1",
            &[],
        );
        assert_eq!(nested.container, "climate/zone_1/vegetation_1.blz");
        assert_eq!(nested.members[0].0, "Arctic_T22_PineL_LOD1-Whole.rbm");
    }
}
