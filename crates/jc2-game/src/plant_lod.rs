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
//! and the far presence range (`scenery::TREE_RANGE` / `BUSH_RANGE`) still decides when the
//! model hands over to the impostor cards or disappears.

use crate::rbm_loader::RbmAsset;
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
}

pub struct PlantLodPlugin;

impl Plugin for PlantLodPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, update_plant_lods);
    }
}

fn update_plant_lods(
    mut commands: Commands,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    rbms: Res<Assets<RbmAsset>>,
    mut plants: Query<(Entity, &GlobalTransform, &mut PlantLod)>,
) {
    let Some(cam) = cams.iter().next() else {
        return;
    };
    let eye = cam.translation();
    let mut swaps = 0;
    for (entity, tf, mut plant) in &mut plants {
        let d2 = eye.distance_squared(tf.translation());
        // `None`: the original culls it or has nothing left to draw; presence at range is
        // the project's `VisibilityRange`, so the current model stays.
        let Some(want) = plant.set.plan.model_at(d2) else {
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
                    if let Some((from, to)) = vis_range {
                        child.insert(bevy::camera::visibility::VisibilityRange {
                            start_margin: 0.0..0.0,
                            end_margin: from..to,
                            use_aabb: false,
                        });
                    }
                }
            });
    }
}
