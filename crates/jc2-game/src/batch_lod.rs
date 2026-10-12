//! Model batches of the far metropolis containers (`.batchlod`, JC2-150 on JC2-146).
//!
//! Natively `CModelBatch::update` (`0xa4c6b0`) walks every loaded batch each frame and
//! shows one of three tiers by the camera's distance to the batch box and to each entry's
//! box (`research/formats/cgd-model-batch-next.md` sections 4 and 5): the merged model from
//! 350 m, the entry's part model between 120 m and 350 m, the entry's individual records
//! closer than 120 m. Models of a batch are spawned hidden with a [`BatchNode`] (`scenery.rs`,
//! `world::spawn_models`) and this system shows the tier that applies. The records are the
//! near models, so inside 120 m they replace the part model instead of drawing beside it.
//!
//! Inferred, not proven: the 120 m test measures to the entry's own model box (the decompiler
//! hides the call's `this`), the boxes are the eight transformed corners of the `.rbm` bounds.

use crate::world::FlyCamera;
use bevy::prelude::*;
use jc2_formats::batchlod::{Aabb, BatchTier};

/// Which model of a batch an entity is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    /// The batch's merged model.
    Merged,
    /// An entry's part model.
    Entry,
    /// One of an entry's individual records.
    Record,
}

/// A batch model's tier data: its kind and the boxes the native update measures.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct BatchNode {
    pub(crate) kind: NodeKind,
    /// The batch's box (the merged model's bounds).
    pub(crate) batch: Aabb,
    /// The owning entry's box (unused by the merged model).
    pub(crate) entry: Aabb,
}

impl BatchNode {
    /// Whether the node draws for a camera at `camera` (engine axes).
    pub(crate) fn shown(&self, camera: [f32; 3]) -> bool {
        let tier = BatchTier::select(
            self.batch.distance_sq(camera),
            self.entry.distance_sq(camera),
        );
        matches!(
            (self.kind, tier),
            (NodeKind::Merged, BatchTier::Merged)
                | (NodeKind::Entry, BatchTier::Entry)
                | (NodeKind::Record, BatchTier::Records)
        )
    }
}

pub(crate) struct BatchLodPlugin;
impl Plugin for BatchLodPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            batch_tiers.after(crate::locations::LocationReconciled),
        );
    }
}

fn batch_tiers(
    cams: Query<&Transform, With<FlyCamera>>,
    mut nodes: Query<(&BatchNode, &mut Visibility)>,
) {
    let Ok(cam) = cams.single() else { return };
    let camera = crate::coords::to_engine(cam.translation);
    for (node, mut vis) in &mut nodes {
        let want = if node.shown(camera) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(lo: [f32; 3], hi: [f32; 3]) -> Aabb {
        Aabb { min: lo, max: hi }
    }

    #[test]
    fn one_tier_shows_at_a_time() {
        let batch = bx([0.0, 0.0, 0.0], [1000.0, 50.0, 1000.0]);
        let entry = bx([500.0, 0.0, 500.0], [520.0, 30.0, 520.0]);
        let node = |kind| BatchNode { kind, batch, entry };
        // far from the batch: the merged model only
        let far = [-400.0, 10.0, 500.0];
        assert!(node(NodeKind::Merged).shown(far));
        assert!(!node(NodeKind::Entry).shown(far));
        assert!(!node(NodeKind::Record).shown(far));
        // inside the batch, 200 m from the entry: its part model only
        let mid = [300.0, 10.0, 520.0];
        assert!(!node(NodeKind::Merged).shown(mid));
        assert!(node(NodeKind::Entry).shown(mid));
        assert!(!node(NodeKind::Record).shown(mid));
        // inside 120 m of the entry: its records only
        let close = [450.0, 10.0, 510.0];
        assert!(!node(NodeKind::Merged).shown(close));
        assert!(!node(NodeKind::Entry).shown(close));
        assert!(node(NodeKind::Record).shown(close));
    }
}
