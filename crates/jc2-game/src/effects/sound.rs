//! Adapters: `Sound` (file type 0, native `CECI_Sound` vftable 0x00eba61c) and `SoundPostEffect`
//! (type 9, `CECI_SoundPostEffect` vftable 0x00eba9e4) onto the project audio (`crate::audio`).
//!
//! Like the light adapter the components only talk to an outbox ([`SoundQueue`]); the audio plugin
//! drains it every frame (`audio::effect_sounds`) and turns the ops into FMOD-style event
//! instances. Which event a `Sound` plays is the variation's `(bank_id, sound_id)` pair, resolved by
//! the same `soundmap` table as every other sound. The **per-surface variation is already made by
//! the spawner**: the interaction matrix picks the variation of the component for the surface or
//! the water flag (doc 3), so an impact's `Sound` arrives here as the right pair already (bullet
//! impacts: bank 17, a sound per material class; `hook_impact`: bank 26, sounds 3..7).
//!
//! Native (research `effects-pipeline.md` 6.2, `JustCause2.exe`): `Start` 0x007bed40 checks the
//! enable flag first; `IsAlive` is a state field (0x00822690); `Stop` 0x00852220; the keys `bank_id`
//! and `sound_id` are the pair of `soundmap`.
//!
//! Inferred, not native-proven (the decode is the open gap task c82f3b20, doc gaps 2 and 3):
//! * the component is alive while the audio instance it started is (the native state field is
//!   non-zero until the sequence ends); a component whose pair does not resolve, or an audio
//!   plugin that is not running, is not alive;
//! * an effect that is `looping` holds its event (loops play until `Stop`); any other effect plays
//!   the event once (its loops once, the audio's one-shot rule);
//! * `Stop` fades the instance out by the event's own fade-out (graceful and immediate alike);
//! * volume 1.0: the unnamed volume/delay/randomisation fields (`#9967eecc`, `#525ff155`, ...),
//!   `var_id_focus` / `var_id_cursor` and the named inputs are not read (counted in
//!   [`Counters::inputs_unread`](super::component::Counters)).
//!
//! `SoundPostEffect` (`attack`, `release`, `gain`, `type`, `target`, `looping`, `delay`,
//! `dsp_variable_*`) is parsed into [`PostEffectParams`] and each start is queued as
//! [`SoundOp::Post`], but **no DSP is applied**: what `target` selects and what the gain does to it
//! are undecoded (0x0078c020, 0x00619ca0), and the project does not guess them. The component is
//! never alive (it cannot hold its effect open until the native lifetime is known) and each start
//! is counted in [`SoundQueue::post_started`].

use super::component::{ComponentInit, EffectComponent};
use bevy::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The fields of a `SoundPostEffect` variation, by their file names (all optional in the file).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct PostEffectParams {
    pub attack: f32,
    pub release: f32,
    pub gain: f32,
    pub kind: i32,
    pub target: i32,
    pub looping: bool,
    pub delay: f32,
    pub dsp_variable_id: i32,
    pub dsp_variable_value0: f32,
    pub dsp_variable_value1: f32,
}

impl PostEffectParams {
    pub fn from_node(n: &jc2_formats::property::Node) -> Self {
        let f = |k: &str| n.float(k).unwrap_or(0.0);
        let i = |k: &str| n.int(k).unwrap_or(0);
        Self {
            attack: f("attack"),
            release: f("release"),
            gain: f("gain"),
            kind: i("type"),
            target: i("target"),
            looping: i("looping") != 0,
            delay: f("delay"),
            dsp_variable_id: i("dsp_variable_id"),
            dsp_variable_value0: f("dsp_variable_value0"),
            dsp_variable_value1: f("dsp_variable_value1"),
        }
    }
}

/// A request from a sound component to the audio side.
#[derive(Debug)]
pub enum SoundOp {
    /// Start the event of `bank:sound` at `at`. `alive` is set false by the audio when the
    /// instance ends (or never starts). `hold`: loops play until a [`SoundOp::Stop`].
    Start {
        token: u64,
        bank: u32,
        sound: u32,
        at: Vec3,
        hold: bool,
        alive: Arc<AtomicBool>,
    },
    Move {
        token: u64,
        at: Vec3,
    },
    Stop {
        token: u64,
        immediate: bool,
    },
    /// A post effect started. Recorded by the audio side, not applied (see the module docs).
    Post {
        token: u64,
        params: PostEffectParams,
    },
}

/// The outbox shared by the sound components and `audio::effect_sounds`.
#[derive(Default)]
pub struct SoundQueue {
    ops: Mutex<Vec<SoundOp>>,
    next: AtomicU64,
    connected: AtomicBool,
    /// Post effect starts (their DSP is not applied, so they are counted, never silent).
    pub post_started: AtomicU64,
}

impl SoundQueue {
    fn push(&self, op: SoundOp) {
        if let Ok(mut o) = self.ops.lock() {
            o.push(op);
        }
    }
    fn token(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed) + 1
    }
    pub fn drain(&self) -> Vec<SoundOp> {
        self.ops.lock().map(|mut o| std::mem::take(&mut *o)).unwrap_or_default()
    }
    /// The audio plugin calls this every frame it drains; until then no sound component is alive
    /// (nothing would ever end it).
    pub fn connect(&self) {
        self.connected.store(true, Ordering::Relaxed);
    }
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

pub struct SoundComponent {
    bank: Option<u32>,
    sound: Option<u32>,
    hold: bool,
    queue: Arc<SoundQueue>,
    token: Option<u64>,
    alive: Arc<AtomicBool>,
    at: Vec3,
    enabled: bool,
}

impl SoundComponent {
    pub fn new(
        bank: Option<u32>,
        sound: Option<u32>,
        hold: bool,
        queue: Arc<SoundQueue>,
    ) -> Self {
        Self {
            bank,
            sound,
            hold,
            queue,
            token: None,
            alive: Arc::new(AtomicBool::new(false)),
            at: Vec3::ZERO,
            enabled: true,
        }
    }

    /// The constructor registered for kind 0.
    pub fn from_init(init: &ComponentInit, queue: Arc<SoundQueue>) -> Box<dyn EffectComponent> {
        let n = &init.variation.node;
        let id = |k: &str| n.int(k).and_then(|v| u32::try_from(v).ok());
        Box::new(Self::new(
            id("bank_id"),
            id("sound_id"),
            init.effect.settings.looping,
            queue,
        ))
    }

    fn release(&mut self, immediate: bool) {
        if let Some(token) = self.token.take() {
            self.alive.store(false, Ordering::Relaxed);
            self.queue.push(SoundOp::Stop { token, immediate });
        }
    }
}

impl Drop for SoundComponent {
    fn drop(&mut self) {
        self.release(true);
    }
}

impl EffectComponent for SoundComponent {
    fn start(&mut self) -> bool {
        // 0x007bed40 checks the enable flag first; a pair that names nothing cannot start
        let (Some(bank), Some(sound)) = (self.bank, self.sound) else {
            return false;
        };
        if !self.enabled {
            return false;
        }
        // a second Start (controller restart) replaces the sequence
        self.release(true);
        let token = self.queue.token();
        self.token = Some(token);
        self.alive = Arc::new(AtomicBool::new(self.queue.is_connected()));
        self.queue.push(SoundOp::Start {
            token,
            bank,
            sound,
            at: self.at,
            hold: self.hold,
            alive: self.alive.clone(),
        });
        true
    }
    fn stop(&mut self, immediate: bool) {
        self.release(immediate);
    }
    fn is_alive(&self) -> bool {
        self.token.is_some() && self.alive.load(Ordering::Relaxed)
    }
    fn update(&mut self, _dt: f32) {}
    fn set_transform(&mut self, m: &Mat4) {
        self.at = m.w_axis.truncate();
        if let Some(token) = self.token {
            self.queue.push(SoundOp::Move { token, at: self.at });
        }
    }
    fn set_velocity(&mut self, _v: Vec3) {}
    fn set_input(&mut self, _key: u32, _value: f32) {}
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// `SoundPostEffect`: parsed and queued, not applied (module docs).
pub struct PostEffectComponent {
    params: PostEffectParams,
    queue: Arc<SoundQueue>,
    enabled: bool,
}

impl PostEffectComponent {
    pub fn new(params: PostEffectParams, queue: Arc<SoundQueue>) -> Self {
        Self {
            params,
            queue,
            enabled: true,
        }
    }

    /// The constructor registered for kind 9.
    pub fn from_init(init: &ComponentInit, queue: Arc<SoundQueue>) -> Box<dyn EffectComponent> {
        Box::new(Self::new(
            PostEffectParams::from_node(&init.variation.node),
            queue,
        ))
    }
}

impl EffectComponent for PostEffectComponent {
    fn start(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        let token = self.queue.token();
        self.queue.post_started.fetch_add(1, Ordering::Relaxed);
        self.queue.push(SoundOp::Post {
            token,
            params: self.params,
        });
        true
    }
    fn stop(&mut self, _immediate: bool) {}
    fn is_alive(&self) -> bool {
        false
    }
    fn update(&mut self, _dt: f32) {}
    fn set_transform(&mut self, _m: &Mat4) {}
    fn set_velocity(&mut self, _v: Vec3) {}
    fn set_input(&mut self, _key: u32, _value: f32) {}
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(queue: &Arc<SoundQueue>) -> SoundComponent {
        SoundComponent::new(Some(26), Some(0), false, queue.clone())
    }

    #[test]
    fn start_queues_the_pair_and_follows_the_audio() {
        let q = Arc::new(SoundQueue::default());
        q.connect();
        let mut c = comp(&q);
        c.set_transform(&Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)));
        assert!(!c.is_alive());
        assert!(c.start());
        let ops = q.drain();
        let SoundOp::Start { bank, sound, at, hold, alive, .. } = &ops[0] else {
            panic!()
        };
        assert_eq!((*bank, *sound, *at, *hold), (26, 0, Vec3::new(1.0, 2.0, 3.0), false));
        assert!(c.is_alive());
        // the audio ends the instance: the component dies with it
        alive.store(false, Ordering::Relaxed);
        assert!(!c.is_alive());
    }

    #[test]
    fn not_alive_without_audio() {
        let q = Arc::new(SoundQueue::default());
        let mut c = comp(&q);
        assert!(c.start());
        assert!(!c.is_alive(), "nothing would ever end it");
    }

    #[test]
    fn stop_move_restart_and_enable() {
        let q = Arc::new(SoundQueue::default());
        q.connect();
        let mut c = comp(&q);
        c.set_enabled(false);
        assert!(!c.start(), "0x007bed40 checks the enable flag first");
        assert!(q.drain().is_empty());
        c.set_enabled(true);
        assert!(c.start());
        c.set_transform(&Mat4::from_translation(Vec3::X));
        c.stop(false);
        assert!(!c.is_alive());
        let ops = q.drain();
        assert!(matches!(ops[1], SoundOp::Move { .. }));
        assert!(matches!(ops[2], SoundOp::Stop { immediate: false, .. }));
        // a restart replaces the sequence
        assert!(c.start());
        assert!(c.start());
        let ops = q.drain();
        assert!(matches!(ops[1], SoundOp::Stop { .. }));
        assert!(matches!(ops[2], SoundOp::Start { .. }));
        drop(c);
        assert!(matches!(q.drain()[0], SoundOp::Stop { immediate: true, .. }));
    }

    #[test]
    fn unresolved_pair_does_not_start() {
        let q = Arc::new(SoundQueue::default());
        assert!(!SoundComponent::new(None, Some(0), false, q.clone()).start());
        assert!(!SoundComponent::new(Some(1), None, false, q).start());
    }

    #[test]
    fn post_effect_is_counted_and_never_holds_the_effect() {
        let q = Arc::new(SoundQueue::default());
        let mut c = PostEffectComponent::new(PostEffectParams::default(), q.clone());
        assert!(c.start());
        assert!(!c.is_alive());
        assert_eq!(q.post_started.load(Ordering::Relaxed), 1);
        assert!(matches!(q.drain()[0], SoundOp::Post { .. }));
    }
}
