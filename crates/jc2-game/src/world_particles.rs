//! Placed world particle objects (`CParticleObject`, FX-SMOKE-3; research
//! `research/formats/smoke-effects.md` section 7): chimneys, damaged-building smoke, steam,
//! camp fires. Each object names a particle leaf directly, with the object's matrix as the
//! emitter's live matrix and no effect id.
//!
//! Native: loader 0x00649f10 (leaf name, autostart byte +0xa8 default 1, three event strings),
//! constructor 0x00796040, the destructor stops every emitter with this data and matrix
//! (0x00a6ca40).
//!
//! Inferred / not native-proven (smoke-effects.md Gap 1, board tasks c90b9918 and d893931d):
//! * the spawn call and its flags: flags 1 (the emitter may release itself) and duration 0 (the
//!   leaf's own window decides), as the other world emitter callers 0x00724670 / 0x006fa510 pass;
//! * autostart objects start when the container that holds them loads, and stop with the
//!   container (the tile unload despawns the entity, dropping the handle stops the emitters);
//! * start/stop events: only the events a destroyed unit fires (`UnitDestroyed::fires`, the
//!   damage controls' `on_destroy` strings) are wired. Mission script events (`bridgeSmoke`,
//!   `::drop`, `playSteam01`), barrel stage events (`::fire`) and the third event slot (role
//!   unknown) have no producer yet, so those objects stay idle;
//! * a start delay (leaf fields unknown, Gap 3): none.
//!
//! The 1024-emitter pool, the 16384 particle gate and the 800 m birth gate (camera distance) are
//! the particle manager's own; this module adds no distance rule of its own.

use crate::destruction::UnitDestroyed;
use crate::effects::EffectHost;
use crate::effects::particle::SharedParticles;
use bevy::prelude::*;

/// One placed particle object (an entity of its tile, despawned with the tile).
#[derive(Component)]
pub struct WorldParticle {
    /// Leaf name (directory and `.psm` stripped).
    pub leaf: String,
    /// World matrix in Bevy axes.
    pub matrix: Mat4,
    /// The tile whose container placed it.
    pub tile: (u32, u32),
    pub autostart: bool,
    /// Event tokens (`e{name({GUID})}::destroyed`) that start / stop it.
    pub start: Vec<String>,
    pub stop: Vec<String>,
    /// Role open (Gap 1): parsed, not wired.
    #[allow(dead_code)]
    pub third: Vec<String>,
    /// Emitters of the running start (head first).
    ids: Vec<u64>,
    /// Stopped emitters whose particles are still flying.
    retired: Vec<u64>,
    /// Set the first time the object is seen.
    manager: Option<SharedParticles>,
    /// Starts so far (tests and diagnostics).
    pub starts: u32,
}

impl WorldParticle {
    pub fn new(
        p: &jc2_formats::placed_particle::PlacedParticle,
        base: Option<&jc2_formats::placement::Transform3>,
        tile: (u32, u32),
    ) -> Self {
        let mut world = p.matrix();
        if let Some(b) = base {
            // the object is relative to the exported entity put down at `base`
            world = world.then(b);
        }
        let tokens = |s: &Option<String>| -> Vec<String> {
            s.as_deref()
                .map(|s| s.split_whitespace().map(str::to_string).collect())
                .unwrap_or_default()
        };
        Self {
            leaf: p.leaf.clone(),
            matrix: crate::coords::mat_to_bevy(&world).to_matrix(),
            tile,
            autostart: p.autostart,
            start: tokens(&p.start_event),
            stop: tokens(&p.stop_event),
            third: tokens(&p.third_event),
            ids: Vec::new(),
            retired: Vec::new(),
            manager: None,
            starts: 0,
        }
    }

    /// Whether an emitter of this object is currently emitting.
    #[cfg(test)]
    pub fn emitting(&self) -> bool {
        self.manager.as_ref().is_some_and(|m| {
            m.lock()
                .is_ok_and(|m| self.ids.iter().any(|id| m.emitter_emitting(*id)))
        })
    }

    #[cfg(test)]
    pub fn running(&self) -> bool {
        !self.ids.is_empty()
    }

    fn start_now(&mut self) {
        let Some(manager) = &self.manager else { return };
        let Ok(mut m) = manager.lock() else { return };
        if self.ids.iter().any(|id| m.emitter_emitting(*id)) {
            return;
        }
        for id in std::mem::take(&mut self.ids) {
            if m.emitter_alive(id) {
                self.retired.push(id);
            } else {
                m.kill_emitter(id);
            }
        }
        // flags 1, duration 0 (inferred, see the module doc)
        self.ids = m.start_emitter(&self.leaf, 0.0, true, self.matrix, Vec3::ZERO);
        self.starts += 1;
    }

    fn stop_now(&mut self) {
        let Some(manager) = &self.manager else { return };
        let Ok(mut m) = manager.lock() else { return };
        for id in &self.ids {
            m.stop_emitter(*id);
        }
        self.retired.append(&mut self.ids);
    }
}

impl Drop for WorldParticle {
    /// Native destructor 0x00a6ca40: stop every emitter with this data and matrix. The manager
    /// keeps stepping them until the particles are gone.
    fn drop(&mut self) {
        let Some(manager) = &self.manager else { return };
        let ids: Vec<u64> = self.ids.drain(..).chain(self.retired.drain(..)).collect();
        if let Ok(mut m) = manager.lock() {
            m.detach_emitters(ids);
        }
    }
}

/// Start autostart objects, route unit events, step the emitters.
fn drive(
    time: Res<Time>,
    host: Option<Res<EffectHost>>,
    mut events: MessageReader<UnitDestroyed>,
    mut objects: Query<&mut WorldParticle>,
) {
    let Some(host) = host else {
        events.clear();
        return;
    };
    for mut o in &mut objects {
        if o.manager.is_none() {
            o.manager = Some(host.particles.clone());
            if o.autostart {
                o.start_now();
            }
        }
    }
    for ev in events.read() {
        let tokens: Vec<&str> = ev
            .fires
            .iter()
            .flat_map(|s| s.split_whitespace())
            .collect();
        if tokens.is_empty() {
            continue;
        }
        for mut o in &mut objects {
            if o.tile != ev.piece.tile {
                continue;
            }
            if o.stop.iter().any(|t| tokens.contains(&t.as_str())) {
                o.stop_now();
            }
            if o.start.iter().any(|t| tokens.contains(&t.as_str())) {
                o.start_now();
            }
        }
    }
    // emitters the objects own are stepped by their owner (the manager steps detached ones)
    let dt = time.delta_secs();
    let Ok(mut m) = host.particles.lock() else {
        return;
    };
    for mut o in &mut objects {
        let o = &mut *o;
        for id in o.ids.iter().chain(&o.retired) {
            m.update_emitter(*id, dt);
        }
        let mut gone = Vec::new();
        o.retired.retain(|id| {
            let alive = m.emitter_alive(*id);
            if !alive {
                gone.push(*id);
            }
            alive
        });
        for id in gone {
            m.kill_emitter(id);
        }
    }
}

pub struct WorldParticlesPlugin;

impl Plugin for WorldParticlesPlugin {
    fn build(&self, app: &mut App) {
        // `JC2_NO_WORLD_PARTICLES=1`: the objects stay inert (before/after captures)
        if std::env::var_os("JC2_NO_WORLD_PARTICLES").is_some() {
            return;
        }
        app.add_message::<UnitDestroyed>().add_systems(
            Update,
            drive
                .after(crate::effects::tick_effects)
                .before(crate::effects::particle_render::draw_particles),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::particle::ParticleManager;
    use crate::world::Piece;
    use jc2_formats::placed_particle::PlacedParticle;
    use jc2_formats::placement::Transform3;

    fn placed(
        leaf: &str,
        autostart: bool,
        start: Option<&str>,
        stop: Option<&str>,
    ) -> PlacedParticle {
        PlacedParticle {
            leaf: leaf.into(),
            autostart,
            translation_only: false,
            start_event: start.map(str::to_string),
            stop_event: stop.map(str::to_string),
            third_event: None,
            world: Transform3::from_mat4x3([
                1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 10.0, 20.0, 30.0,
            ]),
        }
    }

    fn app(manager: ParticleManager) -> App {
        let mut app = App::new();
        let host = EffectHost::default();
        *host.particles.lock().unwrap() = manager;
        app.insert_resource(host)
            .init_resource::<Time>()
            .add_message::<UnitDestroyed>()
            .add_systems(Update, drive);
        app
    }

    fn destroyed(tile: (u32, u32), fires: &str) -> UnitDestroyed {
        UnitDestroyed {
            source_container: String::new(),
            piece: Piece {
                source: None,
                tile,
                unit: 0,
            },
            name: "dac".into(),
            at: Vec3::ZERO,
            activity: None,
            counter: None,
            fires: vec![fires.into()],
            mission_owner: None,
        }
    }

    fn step(app: &mut App) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(50));
        app.update();
    }

    fn manager() -> ParticleManager {
        std::env::var("JC2_TEST_DIR")
            .ok()
            .and_then(|d| jc2_formats::GameFs::open(d).ok())
            .map(|fs| ParticleManager::from_game(&fs))
            .unwrap_or_else(|| ParticleManager::new(None))
    }

    #[test]
    fn autostart_runs_until_the_stop_event_and_the_tile_unload() {
        let mut app = app(manager());
        let ev = "t{bui.mill({G})}::destroyed";
        let o = WorldParticle::new(
            &placed("fx_env_chimney_idyllic_medium_01", true, None, Some(ev)),
            None,
            (3, 4),
        );
        assert_eq!(o.matrix.w_axis.truncate(), Vec3::new(10.0, 20.0, 30.0));
        let e = app.world_mut().spawn(o).id();
        step(&mut app);
        assert!(app.world().get::<WorldParticle>(e).unwrap().emitting());
        // an unrelated event, and the right event on another tile: nothing
        let mut msgs = app.world_mut().resource_mut::<Messages<UnitDestroyed>>();
        msgs.write(destroyed((3, 4), "t{other({G})}::destroyed"));
        msgs.write(destroyed((9, 9), ev));
        step(&mut app);
        assert!(app.world().get::<WorldParticle>(e).unwrap().emitting());
        app.world_mut()
            .resource_mut::<Messages<UnitDestroyed>>()
            .write(destroyed((3, 4), ev));
        step(&mut app);
        assert!(!app.world().get::<WorldParticle>(e).unwrap().emitting());
        // unloading the tile drops the object: the manager runs the rest out
        app.world_mut().despawn(e);
        let host = app.world().resource::<EffectHost>().clone();
        for _ in 0..600 {
            step(&mut app);
            host.particles.lock().unwrap().update_detached(0.05);
        }
        assert_eq!(host.particles.lock().unwrap().emitter_count(), 0);
    }

    #[test]
    fn destroyed_event_starts_the_damage_smoke() {
        let mut app = app(manager());
        let ev = "e{go043h.10x10({961A})}::destroyed";
        let e = app
            .world_mut()
            .spawn(WorldParticle::new(
                &placed("fx_env_smoke_damage_large_01", false, Some(ev), None),
                None,
                (1, 1),
            ))
            .id();
        step(&mut app);
        step(&mut app);
        let o = app.world().get::<WorldParticle>(e).unwrap();
        assert!(
            !o.running() && o.starts == 0,
            "event objects wait for their event"
        );
        // the unit's on_destroy string carries more events after the one we listen to
        app.world_mut()
            .resource_mut::<Messages<UnitDestroyed>>()
            .write(destroyed(
                (1, 1),
                &format!("{ev} l{{wc\\22\\22x19({{50BD}})}}::poweroff gsy.activity.type17"),
            ));
        step(&mut app);
        let o = app.world().get::<WorldParticle>(e).unwrap();
        assert_eq!(o.starts, 1);
        assert!(o.running());
        // a retail leaf really emits (a names-only manager starts headless emitters)
        if std::env::var_os("JC2_TEST_DIR").is_some() {
            for _ in 0..20 {
                step(&mut app);
            }
            let host = app.world().resource::<EffectHost>().clone();
            assert!(host.particles.lock().unwrap().total_particles() > 0);
        }
    }
}
