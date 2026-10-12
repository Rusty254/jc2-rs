//! `EffectSystem`: the port of the native `CEffectSystem` / `CEffect` lifecycle (research doc
//! sections 2 to 4, 5.4). Everything here is native-proven unless a comment says "inferred".
//!
//! Frame order (doc 2.2, 0x00781d60): [`EffectSystem::flush`] then [`EffectSystem::update`], both
//! done by [`EffectSystem::tick`].
//!
//! * [`EffectSystem::play`]: fire and forget (native `Play` 0x00579f50 / 0x006cd100). A new
//!   `CEffect` is built from the shared description on every call; it fails (None) for an unknown
//!   id, an id without a file (the 8 `.xml` names) or when the live counter is above the cap.
//! * [`EffectSystem::create`]: a controller (native handle getter 0x0062a7a0): the effect sits in
//!   the weak list in state 0 and lives as long as the [`EffectController`] is held; driven with
//!   [`EffectSystem::play_controller`] (0x005c1cc0), [`EffectSystem::stop`],
//!   [`EffectSystem::set_transform`], [`EffectSystem::set_velocity`], [`EffectSystem::set_input`].
//! * States (doc 4): 4 start pending, 1 running, 2 tail, 3 stopping, 0 dead; 5 only exists inside
//!   `SetState` (it runs `Begin`).

use super::component::{AdapterSet, ComponentInit, Counters, EffectComponent, KINDS};
use bevy::prelude::*;
use jc2_formats::GameFs;
use jc2_formats::effect::{EffectDescription, EffectRegistry, Interaction};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Weak};

pub const STATE_DEAD: u8 = 0;
pub const STATE_RUNNING: u8 = 1;
pub const STATE_TAIL: u8 = 2;
pub const STATE_STOPPING: u8 = 3;
pub const STATE_PENDING: u8 = 4;
const STATE_BEGIN: u8 = 5;

/// Where descriptions come from. Parsed lazily per id; same results as parsing everything at start.
pub trait DescriptionSource: Send + Sync {
    /// Native `CEffectSystem+0x14`: the number of slots.
    fn slot_count(&self) -> u32;
    /// `None` for an unused slot (the id has no file).
    fn description(&mut self, id: u32) -> Option<Arc<EffectDescription>>;
    /// The id of the registry entry called `name` (file stem, case-insensitive), if the source
    /// has names.
    fn id_by_name(&self, _name: &str) -> Option<u32> {
        None
    }
}

/// The retail files through the game file system.
pub struct RetailSource {
    fs: Arc<GameFs>,
    registry: EffectRegistry,
}

impl RetailSource {
    pub fn new(fs: Arc<GameFs>, registry: EffectRegistry) -> Self {
        Self { fs, registry }
    }
}

impl DescriptionSource for RetailSource {
    fn slot_count(&self) -> u32 {
        self.registry.slot_count()
    }
    fn description(&mut self, id: u32) -> Option<Arc<EffectDescription>> {
        self.registry
            .load_description(&self.fs, id)
            .map(Arc::new)
    }
    fn id_by_name(&self, name: &str) -> Option<u32> {
        self.registry
            .entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .map(|e| e.id)
    }
}

/// Descriptions held in memory (tests, tools). Index = id.
pub struct MemorySource(pub Vec<Option<Arc<EffectDescription>>>);

impl DescriptionSource for MemorySource {
    fn slot_count(&self) -> u32 {
        self.0.len() as u32
    }
    fn description(&mut self, id: u32) -> Option<Arc<EffectDescription>> {
        self.0.get(id as usize).cloned().flatten()
    }
}

/// Identity of a live effect inside the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EffectKey(u64);

/// A non-owning reference to a fire-and-forget effect returned by [`EffectSystem::play`]. Calls with
/// it are no-ops once the effect has been freed (the frame after its state reached 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectHandle(EffectKey);

/// An owning reference to a controller effect (native `shared_ptr` handle). Dropping the last clone
/// lets the system free the effect on its next update.
#[derive(Clone, Debug)]
pub struct EffectController {
    key: EffectKey,
    token: Arc<()>,
}

/// Anything the system's per-effect calls accept.
pub trait AsEffect {
    fn key(&self) -> EffectKey;
}
impl AsEffect for EffectHandle {
    fn key(&self) -> EffectKey {
        self.0
    }
}
impl AsEffect for EffectController {
    fn key(&self) -> EffectKey {
        self.key
    }
}

/// Parameters of a Play (native `0x006cd100`: id, interaction list, matrix, velocity, inputs, object
/// array, direction, component mask). The plain `0x00579f50` Play is `PlayRequest::new(id)` with a
/// transform.
#[derive(Clone, Debug)]
pub struct PlayRequest {
    pub id: u32,
    /// Bevy world space; passed unchanged to the components.
    pub transform: Mat4,
    pub velocity: Vec3,
    /// `(input name hash, value)`.
    pub inputs: Vec<(u32, f32)>,
    /// Surface / water entries that pick the component variations (doc 3).
    pub interactions: Vec<Interaction>,
    /// The callers pass the matrix's second row (inferred: the surface normal). Stored only.
    pub direction: Vec3,
    /// A bit per component kind: a component runs only if `(mask & (1 << kind)) == 0`.
    pub component_mask: u32,
    /// The hit objects. Stored only (meaning open, doc gap 6).
    pub objects: Vec<Entity>,
}

impl PlayRequest {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            transform: Mat4::IDENTITY,
            velocity: Vec3::ZERO,
            inputs: Vec::new(),
            interactions: Vec::new(),
            direction: Vec3::ZERO,
            component_mask: 0,
            objects: Vec::new(),
        }
    }
    pub fn at(mut self, transform: Mat4) -> Self {
        self.transform = transform;
        self
    }
    pub fn at_point(self, p: Vec3) -> Self {
        self.at(Mat4::from_translation(p))
    }
    pub fn velocity(mut self, v: Vec3) -> Self {
        self.velocity = v;
        self
    }
    pub fn input(mut self, key: u32, value: f32) -> Self {
        self.inputs.push((key, value));
        self
    }
    pub fn interaction(mut self, i: Interaction) -> Self {
        self.interactions.push(i);
        self
    }
    pub fn mask(mut self, mask: u32) -> Self {
        self.component_mask = mask;
        self
    }
}

struct Slot {
    comp: Box<dyn EffectComponent>,
    kind: u32,
    uid: u32,
    variation: u32,
}

/// State shared by every effect: live counter, clock, description timers, adapters.
struct Core {
    live: u32,
    cap: u32,
    clock: f64,
    /// Clock value of the last Begin per description (`desc+0x44` is `clock - this`).
    last_begin: Vec<Option<f64>>,
    adapters: AdapterSet,
}

struct CEffect {
    desc: Arc<EffectDescription>,
    state: u8,
    elapsed: f32,
    matrix: Mat4,
    velocity: Vec3,
    inputs: BTreeMap<u32, f32>,
    comps: Vec<Slot>,
    objects: Vec<Entity>,
    direction: Vec3,
    mask: u32,
    interactions: Vec<Interaction>,
}

impl CEffect {
    fn new(desc: Arc<EffectDescription>) -> Self {
        Self {
            desc,
            state: STATE_DEAD,
            elapsed: 0.0,
            matrix: Mat4::IDENTITY,
            velocity: Vec3::ZERO,
            inputs: BTreeMap::new(),
            comps: Vec::new(),
            objects: Vec::new(),
            direction: Vec3::ZERO,
            mask: 0,
            interactions: Vec::new(),
        }
    }

    /// 0x00583100. Live counter +1 when leaving 0, -1 on entering 0.
    fn set_state(&mut self, core: &mut Core, n: u8) {
        if n == self.state {
            return;
        }
        if self.state == STATE_DEAD {
            core.live += 1;
        }
        self.state = n;
        match n {
            STATE_DEAD => core.live = core.live.saturating_sub(1),
            STATE_RUNNING => self.elapsed = 0.0,
            STATE_STOPPING => {
                for s in &mut self.comps {
                    s.comp.stop(false);
                }
            }
            STATE_BEGIN => self.begin(core),
            _ => {}
        }
    }

    /// Instantiate the components from the resolved interaction matrix (0x0078bd40 / 0x006048b0).
    /// The new component is handed the effect's matrix, velocity and inputs before `start`
    /// (inferred: the native builder forwards what Play stored).
    fn build(&mut self, core: &mut Core) {
        let desc = self.desc.clone();
        for (comp, var) in desc.instances(&self.interactions) {
            let init = ComponentInit {
                effect: &desc,
                component: comp,
                variation: var,
            };
            let Some(mut c) = core.adapters.create(&init) else {
                continue;
            };
            c.set_transform(&self.matrix);
            c.set_velocity(self.velocity);
            for (k, v) in &self.inputs {
                c.set_input(*k, *v);
            }
            self.comps.push(Slot {
                comp: c,
                kind: comp.kind,
                uid: comp.uid,
                variation: var.uid,
            });
        }
    }

    /// 0x0058f2b0. Skipped while in the tail state (a tail effect can be restarted); otherwise
    /// builds the components if there are none, enables each by the mask and starts it. No
    /// component started: the effect dies at once.
    fn begin(&mut self, core: &mut Core) {
        if self.state != STATE_TAIL {
            if self.comps.is_empty() {
                self.build(core);
            }
            let mut started = 0;
            for s in &mut self.comps {
                s.comp.set_enabled((self.mask & (1u32 << s.kind)) == 0);
                if s.comp.start() {
                    started += 1;
                }
            }
            if started == 0 {
                self.set_state(core, STATE_DEAD);
                return;
            }
        }
        if let Some(t) = core.last_begin.get_mut(self.desc.id as usize) {
            *t = Some(core.clock);
        }
        self.set_state(core, STATE_RUNNING);
    }

    /// 0x006e6bc0.
    fn update(&mut self, core: &mut Core, dt: f32) {
        if self.state == STATE_PENDING {
            self.begin(core);
            return;
        }
        let mut alive = 0;
        if self.state != STATE_DEAD {
            for s in &mut self.comps {
                if s.comp.is_alive() {
                    s.comp.update(dt);
                    alive += 1;
                }
            }
        }
        match self.state {
            STATE_RUNNING => {
                self.elapsed += dt;
                if alive == 0 && !self.desc.settings.looping {
                    self.state = STATE_TAIL;
                }
            }
            STATE_TAIL => {
                self.elapsed += dt;
                if self.elapsed >= self.desc.settings.tail_time {
                    self.set_state(core, STATE_STOPPING);
                }
            }
            STATE_STOPPING if alive == 0 => self.set_state(core, STATE_DEAD),
            _ => {}
        }
    }

    /// 0x00569d00. Graceful in state 1: stopping when the tail time has already elapsed, else the
    /// tail state; other states unchanged. Immediate: every component killed, state 0.
    fn stop(&mut self, core: &mut Core, immediate: bool) {
        if !immediate {
            if self.state == STATE_RUNNING {
                if self.elapsed >= self.desc.settings.tail_time {
                    self.set_state(core, STATE_STOPPING);
                } else {
                    self.state = STATE_TAIL;
                }
            }
        } else {
            for s in &mut self.comps {
                s.comp.stop(true);
            }
            if self.state != STATE_DEAD {
                self.state = STATE_DEAD;
                core.live = core.live.saturating_sub(1);
            }
        }
    }
}

type Entry = (Weak<()>, EffectKey);

/// Plain-data view of the system for logs and tests.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    pub live: u32,
    pub cap: u32,
    pub strong: usize,
    pub weak: usize,
    pub queued: usize,
    pub plays: u64,
    pub rejected_unknown: u64,
    pub rejected_cap: u64,
    pub created: [u64; KINDS],
    /// Starts of a stub standing in for an unimplemented adapter, by kind.
    pub stub_started: [u64; KINDS],
    pub no_instance_class: [u64; KINDS],
    pub inputs_unread: u64,
}

/// One component of a live effect, for diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentState {
    pub kind: u32,
    pub uid: u32,
    pub variation: u32,
    pub alive: bool,
    pub adapter: bool,
}

/// The effect system resource.
#[derive(Resource)]
pub struct EffectSystem {
    core: Core,
    source: Box<dyn DescriptionSource>,
    descs: Vec<Option<Arc<EffectDescription>>>,
    loaded: Vec<bool>,
    effects: HashMap<EffectKey, CEffect>,
    strong: Vec<EffectKey>,
    weak: Vec<Entry>,
    incoming: Vec<Entry>,
    queue: Vec<(EffectKey, Vec<Interaction>)>,
    next_key: u64,
    plays: u64,
    rejected_unknown: u64,
    rejected_cap: u64,
}

impl EffectSystem {
    /// `cap`: the live-effect cap (the registry's `#6a32433a`, 255).
    pub fn new(cap: u32, source: Box<dyn DescriptionSource>, adapters: AdapterSet) -> Self {
        let n = source.slot_count() as usize;
        Self {
            core: Core {
                live: 0,
                cap,
                clock: 0.0,
                last_begin: vec![None; n],
                adapters,
            },
            source,
            descs: vec![None; n],
            loaded: vec![false; n],
            effects: HashMap::new(),
            strong: Vec::new(),
            weak: Vec::new(),
            incoming: Vec::new(),
            queue: Vec::new(),
            next_key: 1,
            plays: 0,
            rejected_unknown: 0,
            rejected_cap: 0,
        }
    }

    /// The retail registry and files.
    pub fn from_retail(fs: Arc<GameFs>, adapters: AdapterSet) -> jc2_formats::Result<Self> {
        let registry = EffectRegistry::load(&fs)?;
        let cap = registry.live_cap;
        Ok(Self::new(
            cap,
            Box::new(RetailSource::new(fs, registry)),
            adapters,
        ))
    }

    /// The registry id called `name` (for tools and fixtures; gameplay callers pass ids).
    pub fn id_of(&self, name: &str) -> Option<u32> {
        self.source.id_by_name(name)
    }

    pub fn slot_count(&self) -> u32 {
        self.descs.len() as u32
    }

    pub fn adapters(&self) -> &AdapterSet {
        &self.core.adapters
    }

    /// The parsed description of `id` (parsed on first use).
    pub fn description(&mut self, id: u32) -> Option<Arc<EffectDescription>> {
        let i = id as usize;
        if i >= self.descs.len() {
            return None;
        }
        if !self.loaded[i] {
            self.loaded[i] = true;
            self.descs[i] = self.source.description(id);
        }
        self.descs[i].clone()
    }

    /// 0x00694fc0 `CreateEffect`: fails unless the id has a description and the live counter is not
    /// above the cap.
    fn create_effect(&mut self, id: u32) -> Option<EffectKey> {
        let Some(desc) = self.description(id) else {
            self.rejected_unknown += 1;
            return None;
        };
        if self.core.live > self.core.cap {
            self.rejected_cap += 1;
            return None;
        }
        let key = EffectKey(self.next_key);
        self.next_key += 1;
        self.effects.insert(key, CEffect::new(desc));
        Some(key)
    }

    /// Seconds since the last Begin of the description `id` (native `desc+0x44`); the time since the
    /// system started when it never began.
    pub fn since_start(&self, id: u32) -> f32 {
        let begun = self.core.last_begin.get(id as usize).copied().flatten();
        (self.core.clock - begun.unwrap_or(0.0)) as f32
    }

    /// Fire and forget. `None` when the native Play returns 0 (doc 2.1).
    pub fn play(&mut self, req: PlayRequest) -> Option<EffectHandle> {
        let key = self.create_effect(req.id)?;
        self.plays += 1;
        let immediate = !req.objects.is_empty() || !req.interactions.is_empty();
        let queued = req.interactions.clone();
        {
            let core = &mut self.core;
            let e = self.effects.get_mut(&key)?;
            e.matrix = req.transform;
            e.velocity = req.velocity;
            e.inputs = req.inputs.iter().copied().collect();
            e.mask = req.component_mask;
            e.direction = req.direction;
            e.objects = req.objects;
            e.interactions = req.interactions;
            if immediate {
                // 0x006cd100: with objects or interactions Begin runs now instead of waiting at 4
                e.begin(core);
            }
        }
        self.queue.push((key, queued));
        Some(EffectHandle(key))
    }

    /// The handle getter (0x0062a7a0): a controller effect, state 0 until [`play_controller`](Self::play_controller).
    pub fn create(&mut self, id: u32) -> Option<EffectController> {
        let key = self.create_effect(id)?;
        let token = Arc::new(());
        self.incoming.push((Arc::downgrade(&token), key));
        Some(EffectController { key, token })
    }

    /// Controller `Play(interactions, restart)` (0x005c1cc0, doc 5.4). `false` when the effect is
    /// running and `restart` is not set (nothing happens).
    pub fn play_controller(
        &mut self,
        c: &EffectController,
        interactions: &[Interaction],
        restart: bool,
    ) -> bool {
        let core = &mut self.core;
        let Some(e) = self.effects.get_mut(&c.key) else {
            return false;
        };
        if e.state == STATE_RUNNING {
            if !restart {
                return false;
            }
            e.interactions = interactions.to_vec();
            e.set_state(core, STATE_BEGIN);
            return true;
        }
        e.interactions = interactions.to_vec();
        if e.objects.is_empty() {
            e.set_state(core, STATE_PENDING);
        } else {
            e.begin(core);
        }
        true
    }

    /// `Stop(immediate)` (0x00569d00, doc 2.3).
    pub fn stop(&mut self, e: &impl AsEffect, immediate: bool) {
        if let Some(fx) = self.effects.get_mut(&e.key()) {
            fx.stop(&mut self.core, immediate);
        }
    }

    /// 0x00671310: store and forward to every component.
    pub fn set_transform(&mut self, e: &impl AsEffect, m: Mat4) {
        if let Some(fx) = self.effects.get_mut(&e.key()) {
            fx.matrix = m;
            for s in &mut fx.comps {
                s.comp.set_transform(&m);
            }
        }
    }

    /// 0x00735760.
    pub fn set_velocity(&mut self, e: &impl AsEffect, v: Vec3) {
        if let Some(fx) = self.effects.get_mut(&e.key()) {
            fx.velocity = v;
            for s in &mut fx.comps {
                s.comp.set_velocity(v);
            }
        }
    }

    /// 0x006f67d0.
    pub fn set_input(&mut self, e: &impl AsEffect, key: u32, value: f32) {
        if let Some(fx) = self.effects.get_mut(&e.key()) {
            fx.inputs.insert(key, value);
            for s in &mut fx.comps {
                s.comp.set_input(key, value);
            }
        }
    }

    /// The state (0..4) of a live effect; `None` once it was freed.
    pub fn state(&self, e: &impl AsEffect) -> Option<u8> {
        self.effects.get(&e.key()).map(|f| f.state)
    }

    pub fn elapsed(&self, e: &impl AsEffect) -> Option<f32> {
        self.effects.get(&e.key()).map(|f| f.elapsed)
    }

    pub fn components(&self, e: &impl AsEffect) -> Vec<ComponentState> {
        self.effects.get(&e.key()).map_or_else(Vec::new, |f| {
            f.comps
                .iter()
                .map(|s| ComponentState {
                    kind: s.kind,
                    uid: s.uid,
                    variation: s.variation,
                    alive: s.comp.is_alive(),
                    adapter: self.core.adapters.has_adapter(s.kind),
                })
                .collect()
        })
    }

    /// `(registry id, state, components)` of every effect still in the system (diagnostics).
    pub fn active(&self) -> Vec<(u32, u8, Vec<ComponentState>)> {
        let mut v: Vec<_> = self
            .effects
            .iter()
            .map(|(k, f)| (f.desc.id, f.state, self.components(&EffectHandle(*k))))
            .collect();
        v.sort_by_key(|e| e.0);
        v
    }

    /// The live-effect counter (`DAT_011f9c2c`).
    pub fn live(&self) -> u32 {
        self.core.live
    }

    /// Flush the start queue (0x00836840): effects not already running get their interaction list;
    /// with objects or interactions `Begin` runs now, otherwise the state becomes 4; every queued
    /// effect joins the strong list.
    pub fn flush(&mut self) {
        let queue = std::mem::take(&mut self.queue);
        for (key, interactions) in queue {
            let Some(e) = self.effects.get_mut(&key) else {
                continue;
            };
            if e.state != STATE_RUNNING {
                e.interactions = interactions;
                if !e.objects.is_empty() || !e.interactions.is_empty() {
                    e.begin(&mut self.core);
                } else {
                    e.set_state(&mut self.core, STATE_PENDING);
                }
            }
            self.strong.push(key);
        }
    }

    /// 0x005a34f0: advance the clock (the per-description timers), merge the incoming weak
    /// entries, update the weak list (entries whose handle is gone are removed and the effect
    /// freed), then the strong list (a state-0 effect is removed, which frees it).
    pub fn update(&mut self, dt: f32) {
        self.core.clock += f64::from(dt);
        self.weak.append(&mut self.incoming);
        let mut i = 0;
        while i < self.weak.len() {
            let key = self.weak[i].1;
            if self.weak[i].0.upgrade().is_some() {
                if let Some(e) = self.effects.get_mut(&key) {
                    e.update(&mut self.core, dt);
                }
                i += 1;
            } else {
                // the last handle is gone: the effect is freed (live counter released: inferred
                // destructor behaviour)
                if let Some(e) = self.effects.remove(&key)
                    && e.state != STATE_DEAD
                {
                    self.core.live = self.core.live.saturating_sub(1);
                }
                self.weak.swap_remove(i);
            }
        }
        let mut i = 0;
        while i < self.strong.len() {
            let key = self.strong[i];
            match self.effects.get_mut(&key) {
                Some(e) if e.state != STATE_DEAD => {
                    e.update(&mut self.core, dt);
                    i += 1;
                }
                _ => {
                    self.effects.remove(&key);
                    self.strong.swap_remove(i);
                }
            }
        }
    }

    /// One frame: flush, then update (doc 2.2).
    pub fn tick(&mut self, dt: f32) {
        self.flush();
        self.update(dt);
    }

    pub fn diagnostics(&self) -> Diagnostics {
        let c: &Counters = &self.core.adapters.counters;
        let snap = |a: &[std::sync::atomic::AtomicU64; KINDS]| {
            std::array::from_fn(|i| a[i].load(std::sync::atomic::Ordering::Relaxed))
        };
        Diagnostics {
            live: self.core.live,
            cap: self.core.cap,
            strong: self.strong.len(),
            weak: self.weak.len() + self.incoming.len(),
            queued: self.queue.len(),
            plays: self.plays,
            rejected_unknown: self.rejected_unknown,
            rejected_cap: self.rejected_cap,
            created: snap(&c.created),
            stub_started: snap(&c.stub_started),
            no_instance_class: snap(&c.no_instance_class),
            inputs_unread: c.inputs_unread.load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}
