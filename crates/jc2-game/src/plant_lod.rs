//! Native per-asset model LOD switching for scenery plants (JC2-086).
//!
//! The original picks, every frame and per instance, one of the five slots of the plant's
//! `.lod` from the camera distance and the descriptor's LOD scale (`jc2_formats::veglod`,
//! `research/formats/vegetation-lod-native.md`). Slots differ only by a hard switch (no
//! cross-fade), so a plant is one entity whose child meshes are replaced when the selected
//! model changes: no second actor, no second collider (colliders are placed once per
//! instance by `scenery.rs` and never touched here).
//!
//! Project policy, not native: [`MAX_SWAPS_PER_FRAME`] bounds the work after a camera jump,
//! and for the meshes that do not use the foliage material the far presence range
//! (`scenery::TREE_RANGE` / `BUSH_RANGE`) still decides when they disappear. Foliage-material
//! meshes have no such range: they fade out over the native 55..110 m (`veg_fade.rs`, JC2-140).

use crate::rbm_loader::RbmAsset;
use bevy::mesh::MeshTag;
use bevy::prelude::*;
use jc2_formats::veglod::SlotPlan;
use std::sync::Arc;

/// Model swaps applied per frame (policy: a camera jump re-selects every plant at once).
pub const MAX_SWAPS_PER_FRAME: usize = 128;

/// The distinct models of one plant type, held (strongly) while any instance lives, and the
/// native slot plan that chooses among them.
pub(crate) struct PlantLodSet {
    pub(crate) plan: SlotPlan,
    pub(crate) handles: Vec<Handle<RbmAsset>>,
}

/// Marks a plant root whose model follows the camera distance.
#[derive(Component)]
pub(crate) struct PlantLod {
    pub(crate) set: Arc<PlantLodSet>,
    /// Index into the set's models currently spawned as children.
    pub(crate) shown: usize,
    pub(crate) vis_range: Option<(f32, f32)>,
    /// The instance fade byte last written to the foliage meshes (native owner `+0x1f`,
    /// `veglod::instance_byte`; 255 = untouched, JC2-153).
    pub(crate) byte: u8,
    /// Whether the selection follows the camera zoom factor `camera+0x620`
    /// (`veglod::camera_zoom`, JC2-259). Native: a model owner stores `1 / (s F)^2` and the
    /// per-frame submission of dynamic model instances (`00a85a60`, which vehicle parts go
    /// through) refreshes it from the camera (`00a42970`). Plants and placed static objects
    /// stay at F = 1 (their owners take F once when created, `00a4f4b0`; Gap).
    pub(crate) camera_zoom: bool,
    /// A vehicle part: the native vehicle code picks one level for the whole vehicle from its
    /// body model's LOD scale (`006a0e80` -> `0061bb00`, `veglod::vehicle_level`) instead of the
    /// owner multiplier, and no cull applies (board 27da3660). `None` for plants and placed
    /// objects.
    pub(crate) vehicle: Option<VehicleLod>,
}

/// The vehicle-wide inputs of the vehicle LOD level.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VehicleLod {
    /// LOD scale of the vehicle's first part (its body model, `006a0e80`).
    pub(crate) body_scale: f32,
    /// The model-LOD distance unit (`veglod::distance_unit`).
    pub(crate) unit: f32,
}

pub struct PlantLodPlugin;

impl Plugin for PlantLodPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_plant_lods);
    }
}

fn update_plant_lods(
    mut commands: Commands,
    cams: Query<(&GlobalTransform, Option<&Projection>), With<Camera3d>>,
    camera_fov: Option<Res<crate::gameplay_camera::CameraFov>>,
    rbms: Res<Assets<RbmAsset>>,
    lodfactor: Option<Res<crate::veg_fade::LodFactor>>,
    mut plants: Query<(Entity, &GlobalTransform, &mut PlantLod, Option<&Children>)>,
    foliage: Query<(), With<MeshMaterial3d<crate::foliage_alpha::FoliageMaterial>>>,
) {
    let Some((cam, projection)) = cams.iter().next() else {
        return;
    };
    let eye = cam.translation();
    // `camera+0x620` of the realised FOV (the same FOV the terrain and foliage fades use)
    let zoom = crate::gameplay_camera::realised_fov(camera_fov.as_deref(), projection)
        .map_or(1.0, jc2_formats::veglod::camera_zoom);
    let lodfactor = lodfactor.map(|l| *l).unwrap_or_default();
    let mut swaps = 0;
    for (entity, tf, mut plant, children) in &mut plants {
        let d2 = eye.distance_squared(tf.translation());
        // The native last-slot fade byte (`00a89b90`): per instance, to the foliage meshes.
        // `None` is "no submission" natively; presence stays the project's `VisibilityRange`
        // (Gap), so the byte keeps its last value.
        let byte = plant.set.plan.fade_byte_at(d2).unwrap_or(plant.byte);
        if byte != plant.byte
            && let Some(children) = children
        {
            for child in children.iter().filter(|c| foliage.contains(*c)) {
                commands
                    .entity(child)
                    .insert(crate::foliage_alpha::instance_tag(byte));
            }
            plant.byte = byte;
        }
        // `None`: the original culls it or has nothing left to draw; presence at range is
        // the project's `VisibilityRange`, so the current model stays.
        let want = if let Some(v) = plant.vehicle {
            let scaled = jc2_formats::veglod::vehicle_scaled_d2(v.body_scale, d2, zoom, v.unit);
            plant
                .set
                .plan
                .model_at_vehicle_level(jc2_formats::veglod::vehicle_level(scaled))
        } else if plant.camera_zoom {
            plant.set.plan.model_at_zoom(d2, zoom)
        } else {
            plant.set.plan.model_at(d2)
        };
        let Some(want) = want else {
            continue;
        };
        if want == plant.shown {
            continue;
        }
        // not loaded yet: keep what is shown and try again next frame (the handle is held)
        let Some(rbm) = rbms.get(&plant.set.handles[want]) else {
            continue;
        };
        if swaps >= MAX_SWAPS_PER_FRAME {
            break;
        }
        swaps += 1;
        plant.shown = want;
        let vis_range = plant.vis_range;
        let tag = crate::foliage_alpha::instance_tag(plant.byte);
        commands
            .entity(entity)
            .despawn_children()
            .with_children(|parent| {
                for b in &rbm.blocks {
                    let mut child = parent.spawn((
                        Mesh3d(b.mesh.clone()),
                        b.material_component(),
                        Transform::IDENTITY,
                    ));
                    if b.foliage.is_some() {
                        child.insert(MeshTag(tag.0));
                    }
                    // foliage-material meshes fade out natively (`veg_fade.rs`, JC2-140): no
                    // hand-over range, only a lossless cull; the others keep the project range
                    if let Some(range) = crate::veg_fade::plant_visibility_range(
                        b.foliage.is_some(),
                        vis_range,
                        lodfactor.0,
                    ) {
                        child.insert(range);
                    }
                }
            });
    }
}
