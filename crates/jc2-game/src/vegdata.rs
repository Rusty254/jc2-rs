//! The vegetation data read once and shared (scenery, grass and the tree impostors all
//! need it): the climate zones' types and sets, the noise tables, the impostor atlas
//! rectangles. Inserted while the plugins are built, so every startup system can use it.

use crate::Game;
use bevy::prelude::*;
use jc2_formats::impostors::TreeImpostors;
use jc2_formats::vegetation::{Fbm, Vegetation};
use std::sync::Arc;

#[derive(Resource, Clone)]
pub struct VegData {
    pub veg: Option<Arc<Vegetation>>,
    pub fbm: Arc<Fbm>,
    pub impostors: Option<Arc<TreeImpostors>>,
}

pub struct VegDataPlugin;

impl Plugin for VegDataPlugin {
    fn build(&self, app: &mut App) {
        if app.world().contains_resource::<VegData>() {
            return;
        }
        let Some(game) = app.world().get_resource::<Game>() else {
            return;
        };
        let started = std::time::Instant::now();
        let veg = match Vegetation::load(&game.fs) {
            Ok(v) => Some(Arc::new(v)),
            Err(e) => {
                warn!("vegetation: climate data not readable ({e}); no plants");
                None
            }
        };
        let impostors = TreeImpostors::load(&game.fs).ok().map(Arc::new);
        info!(
            "vegetation data loaded in {:.0} ms ({} tree impostors)",
            started.elapsed().as_secs_f32() * 1000.0,
            impostors.as_ref().map_or(0, |i| i.trees.len())
        );
        let data = VegData {
            veg,
            fbm: Arc::new(Fbm::vegetation()),
            impostors,
        };
        app.insert_resource(data);
    }
}

/// Add the plugin unless it is there already.
pub fn ensure(app: &mut App) {
    if !app.world().contains_resource::<VegData>() {
        app.add_plugins(VegDataPlugin);
    }
}
