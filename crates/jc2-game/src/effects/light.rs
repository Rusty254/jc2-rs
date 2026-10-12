//! Adapter: `LightSource` components (file type 7, native `CECI_LightSource` 0x00eba988) onto the
//! existing keyed effect lights of [`crate::point_lights`] (`EffectLight`).
//!
//! The component owns a clock and the keyed tracks; it asks the host (a Bevy system, see
//! [`apply_host_ops`]) to spawn / move / remove the `EffectLight` entities, so the effect lifecycle
//! stays free of the ECS. Alive while the managed light exists: a non-looping track expires after
//! its last key (native: the light manager expires it, doc 6.2); a looping one lives until Stop.
//!
//! Inferred (not native-proven): Stop removes the light at once. The native Stop only sets the
//! managed light's fade flag (0x007cced0) and the fade curve is not decoded (point-light-model gap).

use super::component::{ComponentInit, EffectComponent};
use crate::point_lights::EffectLight;
use bevy::prelude::*;
use jc2_formats::point_light::{LightTrack, variation_light_tracks};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A request from a light component to the ECS side.
#[derive(Debug)]
pub enum LightOp {
    Spawn {
        token: u64,
        tracks: Vec<Arc<LightTrack>>,
        at: Vec3,
    },
    Move {
        token: u64,
        at: Vec3,
    },
    Release {
        token: u64,
    },
}

/// The outbox shared by the light components and [`apply_host_ops`].
#[derive(Default)]
pub struct HostQueue {
    ops: Mutex<Vec<LightOp>>,
    next: AtomicU64,
}

impl HostQueue {
    fn push(&self, op: LightOp) {
        if let Ok(mut o) = self.ops.lock() {
            o.push(op);
        }
    }
    fn token(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }
    pub fn drain(&self) -> Vec<LightOp> {
        self.ops.lock().map(|mut o| std::mem::take(&mut *o)).unwrap_or_default()
    }
}

pub struct LightComponent {
    tracks: Vec<Arc<LightTrack>>,
    host: Arc<HostQueue>,
    token: Option<u64>,
    clock: f32,
    at: Vec3,
    enabled: bool,
}

impl LightComponent {
    pub fn new(tracks: Vec<LightTrack>, host: Arc<HostQueue>) -> Self {
        Self {
            tracks: tracks.into_iter().map(Arc::new).collect(),
            host,
            token: None,
            clock: 0.0,
            at: Vec3::ZERO,
            enabled: true,
        }
    }

    /// The constructor registered for kind 7.
    pub fn from_init(init: &ComponentInit, host: Arc<HostQueue>) -> Box<dyn EffectComponent> {
        Box::new(Self::new(variation_light_tracks(&init.variation.node), host))
    }

    fn release(&mut self) {
        if let Some(token) = self.token.take() {
            self.host.push(LightOp::Release { token });
        }
    }

    fn any_track_live(&self) -> bool {
        self.tracks.iter().any(|t| t.evaluate(self.clock).is_some())
    }
}

impl Drop for LightComponent {
    fn drop(&mut self) {
        self.release();
    }
}

impl EffectComponent for LightComponent {
    fn start(&mut self) -> bool {
        if !self.enabled || self.tracks.is_empty() {
            return false;
        }
        // a second Start (controller restart) re-creates the managed light
        self.release();
        let token = self.host.token();
        self.token = Some(token);
        self.clock = 0.0;
        self.host.push(LightOp::Spawn {
            token,
            tracks: self.tracks.clone(),
            at: self.at,
        });
        true
    }
    fn stop(&mut self, _immediate: bool) {
        self.release();
    }
    fn is_alive(&self) -> bool {
        self.token.is_some() && self.any_track_live()
    }
    fn update(&mut self, dt: f32) {
        self.clock += dt;
        if !self.any_track_live() {
            self.release();
        }
    }
    fn set_transform(&mut self, m: &Mat4) {
        self.at = m.w_axis.truncate();
        if let Some(token) = self.token {
            self.host.push(LightOp::Move { token, at: self.at });
        }
    }
    fn set_velocity(&mut self, _v: Vec3) {}
    fn set_input(&mut self, _key: u32, _value: f32) {}
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// Entities spawned for the light tokens.
#[derive(Resource, Default)]
pub struct LightEntities(HashMap<u64, Vec<Entity>>);

impl LightEntities {
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Apply the queued light requests: spawn `EffectLight` entities (the same component the existing
/// light renderer reads), move them with the effect, despawn on release.
pub fn apply_host_ops(
    host: Res<super::EffectHost>,
    mut commands: Commands,
    mut lights: ResMut<LightEntities>,
    mut transforms: Query<&mut Transform>,
) {
    for op in host.lights.drain() {
        match op {
            LightOp::Spawn { token, tracks, at } => {
                let ents = tracks
                    .into_iter()
                    .map(|t| {
                        commands
                            .spawn((
                                Transform::from_translation(at),
                                GlobalTransform::from_translation(at),
                                EffectLight {
                                    track: t,
                                    clock: 0.0,
                                },
                                Name::new("effect light"),
                            ))
                            .id()
                    })
                    .collect();
                lights.0.insert(token, ents);
            }
            LightOp::Move { token, at } => {
                for e in lights.0.get(&token).into_iter().flatten() {
                    if let Ok(mut t) = transforms.get_mut(*e) {
                        t.translation = at;
                    } else {
                        // spawned this frame: the component is not queryable yet
                        commands.entity(*e).insert(Transform::from_translation(at));
                    }
                }
            }
            LightOp::Release { token } => {
                for e in lights.0.remove(&token).into_iter().flatten() {
                    commands.entity(e).try_despawn();
                }
            }
        }
    }
    // entities removed by the light manager's own expiry no longer need tracking
}

#[cfg(test)]
mod tests {
    use super::*;
    use jc2_formats::point_light::LightKey;

    fn track(end: f32, looping: bool) -> LightTrack {
        let k = |t: f32| LightKey {
            time: t,
            colour: [1.0; 3],
            radius: Some(5.0),
            clq: [1.0; 3],
        };
        LightTrack {
            offset: [0.0; 3],
            looping,
            priority: 1,
            keys: vec![k(0.0), k(end)],
        }
    }

    #[test]
    fn lives_for_its_keys_then_releases() {
        let host = Arc::new(HostQueue::default());
        let mut c = LightComponent::new(vec![track(0.5, false)], host.clone());
        c.set_transform(&Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        assert!(c.start());
        assert!(c.is_alive());
        c.update(0.3);
        assert!(c.is_alive());
        c.update(0.3);
        assert!(!c.is_alive());
        let ops = host.drain();
        assert!(matches!(ops[0], LightOp::Spawn { at, .. } if at == Vec3::new(1.0, 2.0, 3.0)));
        assert!(matches!(ops.last(), Some(LightOp::Release { .. })));
    }

    #[test]
    fn looping_light_needs_stop_and_drop_releases() {
        let host = Arc::new(HostQueue::default());
        let mut c = LightComponent::new(vec![track(0.5, true)], host.clone());
        assert!(c.start());
        for _ in 0..100 {
            c.update(0.1);
        }
        assert!(c.is_alive());
        c.stop(false);
        assert!(!c.is_alive());
        assert!(c.start());
        drop(c);
        let releases = host
            .drain()
            .iter()
            .filter(|o| matches!(o, LightOp::Release { .. }))
            .count();
        assert_eq!(releases, 2);
    }

    #[test]
    fn disabled_light_does_not_start() {
        let host = Arc::new(HostQueue::default());
        let mut c = LightComponent::new(vec![track(0.5, false)], host);
        c.set_enabled(false);
        assert!(!c.start());
    }
}
