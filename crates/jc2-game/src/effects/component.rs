//! The component contract of an effect (native `CEffectComponentInstance`, vftable 0x00eb99cc,
//! doc 1.3) and the adapter registry.
//!
//! Slot roles (doc 1.3): 0 `Start() -> bool`, 1 `Stop(immediate)`, 3 `IsAlive() -> bool`,
//! 4 `SetInput(key, value)`, 7 `SetMatrix`, 8 `SetVelocity`, 10 `Update(dt)` (only called while
//! alive), 11 `SetEnabled`.

use bevy::prelude::*;
use jc2_formats::effect::{ComponentDesc, EffectDescription, Variation, kind};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Number of component kinds (file `type` 0..=10).
pub const KINDS: usize = 11;

/// One live component of one effect. Implementations must not panic on repeated `start`
/// (a controller restart calls it again on every existing component, doc 5.4).
pub trait EffectComponent: Send + Sync {
    /// Begin work. `false` = the component could not start (it does not count towards the effect
    /// surviving `Begin`).
    fn start(&mut self) -> bool;
    /// `immediate = false`: stop producing and let what exists finish; `true`: kill at once.
    fn stop(&mut self, immediate: bool);
    /// Whether the component still keeps the effect alive. The one-shot classes (decal, force pulse,
    /// haptics, camera shake) are never alive after `start`.
    fn is_alive(&self) -> bool;
    /// Called every frame while `is_alive()`.
    fn update(&mut self, dt: f32);
    /// The effect's transform (Bevy world space; the 4x4 passes through unchanged).
    fn set_transform(&mut self, m: &Mat4);
    fn set_velocity(&mut self, v: Vec3);
    /// A named float input (`key` = hash of the input name).
    fn set_input(&mut self, key: u32, value: f32);
    /// `false` when the component's kind is masked off by the Play's component mask.
    fn set_enabled(&mut self, enabled: bool);
}

/// What an adapter constructor gets to build one component instance.
pub struct ComponentInit<'a> {
    pub effect: &'a EffectDescription,
    pub component: &'a ComponentDesc,
    pub variation: &'a Variation,
}

type Ctor = Box<dyn Fn(&ComponentInit) -> Box<dyn EffectComponent> + Send + Sync>;

/// Per-kind counters shared by the adapters, so an unimplemented adapter is visible (never
/// silently absent).
#[derive(Default)]
pub struct Counters {
    /// Component instances built, by kind.
    pub created: [AtomicU64; KINDS],
    /// `start()` calls on a stub (native-contract placeholder) of that kind.
    pub stub_started: [AtomicU64; KINDS],
    /// Components skipped because the native game has no instance class (kinds 2 and 6).
    pub no_instance_class: [AtomicU64; KINDS],
    /// Inputs delivered to a component that has no reader for them yet.
    pub inputs_unread: AtomicU64,
}

impl Counters {
    pub fn get(&self, which: &[AtomicU64; KINDS], kind: u32) -> u64 {
        which
            .get(kind as usize)
            .map_or(0, |c| c.load(Ordering::Relaxed))
    }
    pub fn bump(which: &[AtomicU64; KINDS], kind: u32) {
        if let Some(c) = which.get(kind as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The adapters, one constructor per component kind. A kind without a registered constructor gets
/// the native-contract stub of [`StubComponent`].
pub struct AdapterSet {
    ctors: BTreeMap<u32, Ctor>,
    pub counters: Arc<Counters>,
}

impl AdapterSet {
    /// Only stubs; register real adapters with [`register`](Self::register).
    pub fn stubs() -> Self {
        Self {
            ctors: BTreeMap::new(),
            counters: Arc::new(Counters::default()),
        }
    }

    pub fn register(
        &mut self,
        kind: u32,
        ctor: impl Fn(&ComponentInit) -> Box<dyn EffectComponent> + Send + Sync + 'static,
    ) {
        self.ctors.insert(kind, Box::new(ctor));
    }

    /// Whether a real (non-stub) adapter is registered for `kind`.
    pub fn has_adapter(&self, kind: u32) -> bool {
        self.ctors.contains_key(&kind)
    }

    /// Build a component. `None` for the kinds the native game has no instance class for.
    pub fn create(&self, init: &ComponentInit) -> Option<Box<dyn EffectComponent>> {
        let k = init.component.kind;
        if k == kind::POST_EFFECT || k == kind::CUTSCENE || k as usize >= KINDS {
            Counters::bump(&self.counters.no_instance_class, k);
            return None;
        }
        Counters::bump(&self.counters.created, k);
        Some(match self.ctors.get(&k) {
            Some(c) => c(init),
            None => Box::new(StubComponent::new(k, self.counters.clone())),
        })
    }
}

/// The placeholder for a component class without an adapter: follows the native one-shot contract
/// (`start()` succeeds so the rest of the effect still runs; never alive afterwards, so it never
/// holds the effect open) and counts each start in [`Counters::stub_started`].
pub struct StubComponent {
    kind: u32,
    counters: Arc<Counters>,
    enabled: bool,
}

impl StubComponent {
    pub fn new(kind: u32, counters: Arc<Counters>) -> Self {
        Self {
            kind,
            counters,
            enabled: true,
        }
    }
}

impl EffectComponent for StubComponent {
    fn start(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        Counters::bump(&self.counters.stub_started, self.kind);
        true
    }
    fn stop(&mut self, _immediate: bool) {}
    fn is_alive(&self) -> bool {
        false
    }
    fn update(&mut self, _dt: f32) {}
    fn set_transform(&mut self, _m: &Mat4) {}
    fn set_velocity(&mut self, _v: Vec3) {}
    fn set_input(&mut self, _key: u32, _value: f32) {
        self.counters.inputs_unread.fetch_add(1, Ordering::Relaxed);
    }
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}
