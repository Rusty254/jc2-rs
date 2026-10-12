//! Adapter: `VisualParticleSystem` components (file type 1, native `CECI_VisualParticleSystem`
//! 0x00eba4c8) with the particle manager's admission rules (doc 6.1, 0x00a6cb40) and the emitter
//! simulation of [`super::particle_sim`], drawn by [`super::particle_render`].
//!
//! Native-proven here: the 1024 live-emitter limit, the density scale
//! `max(0.25, 1 - (live + 1) / 1024)` applied to the particle cap and the emission spacing, a
//! trailing `.psm` stripped before the `particles.blz` lookup, that a name which does not resolve
//! spawns nothing (16 retail names dangle), a non-positive authored cap spawns nothing, and the
//! Start rule of doc 5.4 (a live emitter keeps running; a finished one is replaced).
//!
//! Native-proven (`particle-units-next.md` section 5, `0xa71720` / `0xa6d290`): every
//! `linked_system_N` (N = 0, 1, ... until the first missing one; up to 33 in retail, 12 for
//! `fx_exp_grenade_01`) starts together with its parent, immediately, with the same source, and
//! starts its own links in turn; each counts as its own emitter. The per-link start delay fields are
//! not located (Gap 1): the links start at 0.
//!
//! Native-proven lifecycle (`smoke-effects.md` sections 2 to 5, task 484554ae): linked systems follow
//! the head's live matrix, get its duration, never release themselves and stop when the head stops
//! or finishes; there is no depth or repeat limit (retail trees reach 8 levels; the pool bounds
//! cycles). Every particle of a leaf with a trail system (`#1de3fca7`) gets its own trail emitter
//! that follows it, receives its alpha as system scale and stops with it. `Stop` (and dropping the
//! component, which is `Stop(immediate)`) never removes live particles: dropped emitters are
//! stopped and the manager runs them out ([`ParticleManager::update_detached`]). The variation
//! flags byte `#b0124c0a` (default 1) decides whether the head may release itself.
//!
//! Inferred (not native-proven): a restarted component retires emitters that stopped emitting
//! instead of waiting for their particles (muzzle task 446ad726; the native "finished" flag
//! `word[emitter+0x2a2] & 6` is not decoded); the cull-distance origin is the camera (Gap 3); a
//! trail's orientation is the source matrix's.
//!
//! A manager created without the game files has no leaf data; its emitters are headless records that
//! live for the variation's `duration` seconds (unit tests of the lifecycle).

use super::component::{ComponentInit, EffectComponent};
use super::particle_sim::{Admission, Emitter, JobContext, Particle};
use bevy::prelude::*;
use jc2_formats::GameFs;
use jc2_formats::particle::{ParticleSystem, RELEASE_FLAG_HASH};
use jc2_formats::property::Variant;
use jc2_formats::sarc::SmallArchive;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// Live emitter limit (native `[mgr+0x28] < 0x400`).
pub const MAX_EMITTERS: usize = 1024;

/// `s = max(0.25, 1 - (live + 1) / 1024)` (0x00a6b120). Same f32 arithmetic as the native code
/// (multiplication by `1/1024`, floor taken when `0.25 - s >= 0`).
pub fn density_scale(live: usize) -> f32 {
    let s = 1.0 - (live as f32 + 1.0) * (1.0 / 1024.0);
    if 0.25 - s >= 0.0 { 0.25 } else { s }
}

/// What `spawn` decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Spawned {
    pub name: String,
    /// The density scale the emitter was started with (cap `ceil(cap * s)`, interval `/ s`).
    pub scale: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParticleStats {
    pub spawned: u64,
    pub refused_unresolved_name: u64,
    pub refused_full: u64,
    /// The leaf exists but does not parse (39 retail leaves) or has a non-positive cap.
    pub refused_unusable: u64,
    /// Emitters that exist only as a lifetime record (no leaf data loaded).
    pub headless: u64,
    /// Follow-on slot emitters started with a head.
    pub chained: u64,
}

/// The leaves of `global/particles.blz`, parsed on first use.
struct Library {
    blob: Vec<u8>,
    archive: SmallArchive,
    cache: HashMap<String, Option<Arc<ParticleSystem>>>,
}

impl Library {
    fn system(&mut self, name: &str) -> Option<Arc<ParticleSystem>> {
        if let Some(c) = self.cache.get(name) {
            return c.clone();
        }
        let parsed = self
            .archive
            .get(&format!("{name}.psmb"))
            .and_then(|e| self.archive.data(&self.blob, e).ok())
            .and_then(|d| ParticleSystem::parse(d).ok())
            .map(Arc::new);
        self.cache.insert(name.to_string(), parsed.clone());
        parsed
    }
}

enum Body {
    Sim(Box<Emitter>),
    /// `(age, life)` without leaf data.
    Headless(f32, f32),
}

struct Slot {
    body: Body,
    /// Still counted in the live total (native `[mgr+0x28]`).
    counted: bool,
    /// The emitter that started this one as a linked system (native child+0x28c).
    parent: Option<u64>,
    /// Linked systems this emitter started (native +0x280).
    linked: Vec<u64>,
    /// The emitter whose particle owns this trail chain: it steps this slot (trail heads and their
    /// linked systems).
    driver: Option<u64>,
    /// Trail chains this emitter steps.
    driven: Vec<u64>,
    /// Owner gone (component dropped): the manager steps it until it finishes.
    detached: bool,
    /// The stop of the linked systems after this emitter finished was already sent.
    linked_stopped: bool,
}

/// The manager: the live emitter count, the leaf library and every emitter.
pub struct ParticleManager {
    live: usize,
    /// Lower-case `particles.blz` member names without `.psmb`; `None` = names not loaded (every
    /// name resolves; unit tests).
    names: Option<HashSet<String>>,
    library: Option<Library>,
    emitters: BTreeMap<u64, Slot>,
    next_id: u64,
    /// Live particles of all simulated emitters (the job context's global count).
    total_particles: usize,
    /// Position the cull distance of every leaf is measured from (inferred: the render camera,
    /// `smoke-effects.md` Gap 3). `None` disables the gate.
    camera: Option<Vec3>,
    /// Emitters whose component is gone; see [`Self::update_detached`].
    detached: Vec<u64>,
    pub stats: ParticleStats,
}

impl ParticleManager {
    pub fn new(names: Option<HashSet<String>>) -> Self {
        Self {
            live: 0,
            names,
            library: None,
            emitters: BTreeMap::new(),
            next_id: 1,
            total_particles: 0,
            camera: None,
            detached: Vec::new(),
            stats: ParticleStats::default(),
        }
    }

    /// Read `global/particles.blz` (names and leaves).
    pub fn from_game(fs: &GameFs) -> Self {
        let archive = fs
            .read_inflated("particles.blz")
            .ok()
            .and_then(|blob| SmallArchive::parse(&blob).ok().map(|a| (blob, a)));
        let Some((blob, archive)) = archive else {
            return Self::new(None);
        };
        let names = archive
            .entries
            .iter()
            .filter_map(|e| {
                e.name
                    .to_ascii_lowercase()
                    .strip_suffix(".psmb")
                    .map(str::to_string)
            })
            .collect();
        let mut m = Self::new(Some(names));
        m.library = Some(Library {
            blob,
            archive,
            cache: HashMap::new(),
        });
        m
    }

    pub fn live(&self) -> usize {
        self.live
    }

    /// Set the position the births' cull distance is measured from.
    pub fn set_camera(&mut self, position: Option<Vec3>) {
        self.camera = position;
    }

    /// Live particles of all simulated emitters (the global gate's count).
    pub fn total_particles(&self) -> usize {
        self.total_particles
    }

    /// Whether `name` resolves (`.psm` stripped, case-insensitive).
    pub fn resolves(&self, name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        let n = n.strip_suffix(".psm").unwrap_or(&n);
        !n.is_empty() && self.names.as_ref().is_none_or(|s| s.contains(n))
    }

    fn normalized(name: &str) -> String {
        let n = name.to_ascii_lowercase();
        n.strip_suffix(".psm").unwrap_or(&n).to_string()
    }

    /// 0x00a6cb40 admission: `None` (nothing spawned) when the name does not resolve or the limit
    /// is reached.
    pub fn spawn(&mut self, name: &str) -> Option<Spawned> {
        if !self.resolves(name) {
            self.stats.refused_unresolved_name += 1;
            return None;
        }
        if self.live >= MAX_EMITTERS {
            self.stats.refused_full += 1;
            return None;
        }
        let scale = density_scale(self.live);
        self.live += 1;
        self.stats.spawned += 1;
        Some(Spawned {
            name: name.to_string(),
            scale,
        })
    }

    fn release(&mut self) {
        self.live = self.live.saturating_sub(1);
    }

    fn add(&mut self, body: Body) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.emitters.insert(
            id,
            Slot {
                body,
                counted: true,
                parent: None,
                linked: Vec::new(),
                driver: None,
                driven: Vec::new(),
                detached: false,
                linked_stopped: false,
            },
        );
        id
    }

    /// Start the head system and every linked system (all of them at once, native `0xa6d290`).
    /// `may_release` is the variation flags byte's bit 0 (`#b0124c0a`, default 1). Returns the ids
    /// of the emitters that started (empty when the head was refused), head first.
    pub fn start_emitter(
        &mut self,
        name: &str,
        duration: f32,
        may_release: bool,
        matrix: Mat4,
        velocity: Vec3,
    ) -> Vec<u64> {
        let mut ids = Vec::new();
        let ctx = Start {
            duration,
            may_release,
            matrix,
            velocity,
        };
        self.start_chain(name, &ctx, None, &mut ids);
        ids
    }

    /// One emitter and, recursively, its linked systems. There is no depth limit or repeat guard
    /// (native has none; the 1024 emitter pool bounds a cycle, retail has none). Linked systems get
    /// the head's duration and never release themselves (flags 0).
    fn start_chain(&mut self, name: &str, ctx: &Start, parent: Option<u64>, ids: &mut Vec<u64>) {
        let key = Self::normalized(name);
        let leaf = self.library.as_mut().map(|l| l.system(&key));
        if matches!(&leaf, Some(None)) && self.resolves(name) {
            // resolvable name whose leaf does not parse
            self.stats.refused_unusable += 1;
            return;
        }
        if let Some(Some(sys)) = &leaf
            && sys.max_particles <= 0
        {
            // non-positive authored capacity: nothing to spawn (doc 6.1)
            self.stats.refused_unusable += 1;
            return;
        }
        let Some(spawned) = self.spawn(name) else {
            return;
        };
        match leaf.flatten() {
            Some(sys) => {
                let children = sys.children.clone();
                let seed = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(self.next_id + 1);
                let mut e = Emitter::new(
                    sys,
                    Admission {
                        scale: spawned.scale,
                        duration: ctx.duration,
                        may_release: parent.is_none() && ctx.may_release,
                        seed,
                    },
                );
                e.matrix = ctx.matrix;
                e.velocity = ctx.velocity;
                let id = self.add(Body::Sim(Box::new(e)));
                if let Some(p) = parent {
                    if let Some(s) = self.emitters.get_mut(&id) {
                        s.parent = Some(p);
                    }
                    if let Some(s) = self.emitters.get_mut(&p) {
                        s.linked.push(id);
                    }
                    self.stats.chained += 1;
                }
                ids.push(id);
                for c in children {
                    self.start_chain(&c, ctx, Some(id), ids);
                }
            }
            None => {
                self.stats.headless += 1;
                ids.push(self.add(Body::Headless(0.0, ctx.duration)));
            }
        }
    }

    /// 0x00a6e2f0: a trail emitter (flags 1, copied matrix) for a newborn particle of `driver`.
    fn start_trail(&mut self, driver: u64, name: &str, matrix: Mat4) -> Option<u64> {
        let mut ids = Vec::new();
        let ctx = Start {
            duration: 0.0,
            may_release: true,
            matrix,
            velocity: Vec3::ZERO,
        };
        self.start_chain(name, &ctx, None, &mut ids);
        let head = *ids.first()?;
        for id in &ids {
            if let Some(s) = self.emitters.get_mut(id) {
                s.driver = Some(driver);
            }
        }
        if let Some(s) = self.emitters.get_mut(&driver) {
            s.driven.extend(ids);
        }
        Some(head)
    }

    /// Whether the emitter still exists and has not finished.
    pub fn emitter_alive(&self, id: u64) -> bool {
        self.emitters.get(&id).is_some_and(|s| match &s.body {
            Body::Sim(e) => !e.finished(),
            Body::Headless(age, life) => s.counted && age < life,
        })
    }

    /// Whether the emitter still births particles (its emission window is not over and it was not
    /// stopped); particles it already made may outlive that.
    pub fn emitter_emitting(&self, id: u64) -> bool {
        self.emitters.get(&id).is_some_and(|s| match &s.body {
            Body::Sim(e) => e.is_emitting() && !e.is_stopping(),
            Body::Headless(age, life) => s.counted && age < life,
        })
    }

    /// One job of the emitter (native `0xa66490` plus the end-test pass `0xa6e6c0`): the
    /// simulation step, the finish and the stop of its linked systems, then its particles' trail
    /// emitters, which this call also steps.
    pub fn update_emitter(&mut self, id: u64, dt: f32) {
        let ctx = JobContext {
            total_particles: self.total_particles,
            camera: self.camera,
        };
        let mut release = false;
        let mut finished = false;
        let mut delta = 0isize;
        if let Some(s) = self.emitters.get_mut(&id) {
            match &mut s.body {
                Body::Sim(e) => {
                    let before = e.particles.len();
                    e.step_with(dt, &ctx);
                    delta = e.particles.len() as isize - before as isize;
                    if e.finished() {
                        finished = true;
                        if s.counted {
                            s.counted = false;
                            release = true;
                        }
                    }
                }
                Body::Headless(age, life) => {
                    *age += dt;
                    if *age >= *life && s.counted {
                        s.counted = false;
                        release = true;
                    }
                }
            }
        }
        self.total_particles = self.total_particles.saturating_add_signed(delta);
        if release {
            self.release();
        }
        if finished {
            // 0xa6af80(0): a finished head stops every linked system, once
            let linked = self
                .emitters
                .get_mut(&id)
                .filter(|s| !s.linked_stopped)
                .map(|s| {
                    s.linked_stopped = true;
                    s.linked.clone()
                });
            for c in linked.into_iter().flatten() {
                self.stop_emitter(c);
            }
        }
        self.service_trails(id);
        self.step_driven(id, dt);
    }

    /// The trail emitters of `id`'s particles (native `0xa6e2f0`, once per job).
    fn service_trails(&mut self, id: u64) {
        enum Act {
            Spawn(usize, Mat4),
            Follow(usize, u64, Mat4, f32),
            Stop(usize, u64),
            Skip(usize),
        }
        let (name, orphans, plan) = {
            let Some(Slot {
                body: Body::Sim(e), ..
            }) = self.emitters.get_mut(&id)
            else {
                return;
            };
            let orphans = std::mem::take(&mut e.orphan_trails);
            let Some(name) = e.system.trail.clone() else {
                return;
            };
            let stopping = e.is_stopping();
            let mut plan = Vec::new();
            for (i, p) in e.particles.iter().enumerate() {
                let mut m = e.matrix;
                m.w_axis = e.world_position(p).extend(1.0);
                let dead = p.age >= 1.0 || p.size <= 0.0;
                match p.trail {
                    None if p.needs_trail => plan.push(if stopping || dead {
                        Act::Skip(i)
                    } else {
                        Act::Spawn(i, m)
                    }),
                    Some(t) if dead || stopping => plan.push(Act::Stop(i, t)),
                    Some(t) => plan.push(Act::Follow(i, t, m, p.alpha)),
                    None => {}
                }
            }
            (name, orphans, plan)
        };
        for t in orphans {
            self.stop_emitter(t);
        }
        for act in plan {
            match act {
                Act::Skip(i) => self.particle_mut(id, i, |p| p.needs_trail = false),
                Act::Spawn(i, m) => {
                    let t = self.start_trail(id, &name, m);
                    self.particle_mut(id, i, |p| {
                        p.needs_trail = false;
                        p.trail = t;
                    });
                }
                Act::Stop(i, t) => {
                    self.stop_emitter(t);
                    self.particle_mut(id, i, |p| p.trail = None);
                }
                Act::Follow(i, t, m, alpha) => {
                    if self.emitters.contains_key(&t) && self.emitter_alive(t) {
                        self.set_source(t, m, None);
                        if alpha < 1.0 {
                            self.set_system_scale(t, alpha);
                        }
                    } else {
                        self.particle_mut(id, i, |p| p.trail = None);
                    }
                }
            }
        }
    }

    fn particle_mut(&mut self, id: u64, index: usize, f: impl FnOnce(&mut Particle)) {
        if let Some(Slot {
            body: Body::Sim(e), ..
        }) = self.emitters.get_mut(&id)
            && let Some(p) = e.particles.get_mut(index)
        {
            f(p);
        }
    }

    /// Step the trail chains this emitter owns and remove the ones that finished.
    fn step_driven(&mut self, id: u64, dt: f32) {
        let list = match self.emitters.get(&id) {
            Some(s) if !s.driven.is_empty() => s.driven.clone(),
            _ => return,
        };
        for t in list {
            if !self.emitters.contains_key(&t) {
                continue;
            }
            self.update_emitter(t, dt);
            if self.sim_finished(t) {
                self.kill_emitter(t);
            }
        }
        if let Some(s) = self.emitters.get(&id) {
            let keep: Vec<u64> = s
                .driven
                .iter()
                .copied()
                .filter(|t| self.emitters.contains_key(t))
                .collect();
            if let Some(s) = self.emitters.get_mut(&id) {
                s.driven = keep;
            }
        }
    }

    fn sim_finished(&self, id: u64) -> bool {
        self.emitters.get(&id).is_some_and(|s| match &s.body {
            Body::Sim(e) => e.finished(),
            Body::Headless(age, life) => !s.counted || age >= life,
        })
    }

    /// The emitter's source frame moves it and every linked system (native: one shared matrix).
    pub fn set_source(&mut self, id: u64, matrix: Mat4, velocity: Option<Vec3>) {
        let mut linked = Vec::new();
        if let Some(s) = self.emitters.get_mut(&id) {
            linked.clone_from(&s.linked);
            if let Body::Sim(e) = &mut s.body {
                e.matrix = matrix;
                if let Some(v) = velocity {
                    e.velocity = v;
                }
            }
        }
        for c in linked {
            self.set_source(c, matrix, velocity);
        }
    }

    /// The system scale (native `+0xac`) of the emitter and its linked systems.
    pub fn set_system_scale(&mut self, id: u64, scale: f32) {
        let mut linked = Vec::new();
        if let Some(s) = self.emitters.get_mut(&id) {
            linked.clone_from(&s.linked);
            if let Body::Sim(e) = &mut s.body {
                e.system_scale = scale;
            }
        }
        for c in linked {
            self.set_system_scale(c, scale);
        }
    }

    /// Native stop (`0xa6af80(1)`): the stop flag only. Linked systems stop with the head, live
    /// particles are never removed, and there is no immediate variant (`Stop(immediate)` of the
    /// effect only drops the handle, see [`Self::detach_emitters`]).
    pub fn stop_emitter(&mut self, id: u64) {
        let mut release = false;
        let mut linked = Vec::new();
        if let Some(s) = self.emitters.get_mut(&id) {
            linked.clone_from(&s.linked);
            match &mut s.body {
                Body::Sim(e) => e.stop(),
                Body::Headless(age, life) => {
                    *age = *life;
                    if s.counted {
                        s.counted = false;
                        release = true;
                    }
                }
            }
        }
        if release {
            self.release();
        }
        for c in linked {
            self.stop_emitter(c);
        }
    }

    /// Remove the emitter record at once (its particles go with it). Trail chains it still steps
    /// are detached, not killed.
    pub fn kill_emitter(&mut self, id: u64) {
        let Some(s) = self.emitters.remove(&id) else {
            return;
        };
        if s.counted {
            self.release();
        }
        if let Body::Sim(e) = &s.body {
            self.total_particles = self.total_particles.saturating_sub(e.particles.len());
        }
        for t in s.driven {
            self.detach_one(t);
        }
        if let Some(d) = s.driver
            && let Some(ds) = self.emitters.get_mut(&d)
        {
            ds.driven.retain(|t| *t != id);
        }
        if let Some(p) = s.parent
            && let Some(ps) = self.emitters.get_mut(&p)
        {
            ps.linked.retain(|c| *c != id);
        }
        for c in s.linked {
            if let Some(cs) = self.emitters.get_mut(&c) {
                cs.parent = None;
            }
        }
    }

    /// The owner of these emitters is gone: they stop (graceful) and the manager keeps stepping
    /// them until they finish (native: the component only drops its handle, particles live on).
    pub fn detach_emitters(&mut self, ids: impl IntoIterator<Item = u64>) {
        for id in ids {
            self.detach_one(id);
        }
    }

    fn detach_one(&mut self, id: u64) {
        let headless = match self.emitters.get(&id) {
            None => return,
            Some(s) => matches!(s.body, Body::Headless(..)),
        };
        if headless || self.sim_finished(id) {
            self.kill_emitter(id);
            return;
        }
        if let Some(s) = self.emitters.get_mut(&id) {
            s.driver = None;
            if !s.detached {
                s.detached = true;
                self.detached.push(id);
            }
        }
        self.stop_emitter(id);
    }

    /// Step the detached emitters (once per frame, from the plugin) and drop the finished ones.
    pub fn update_detached(&mut self, dt: f32) {
        for id in std::mem::take(&mut self.detached) {
            if !self.emitters.contains_key(&id) {
                continue;
            }
            self.update_emitter(id, dt);
            if self.sim_finished(id) {
                self.kill_emitter(id);
            } else {
                self.detached.push(id);
            }
        }
    }

    /// Emitters waiting in the detached list.
    pub fn detached_count(&self) -> usize {
        self.detached.len()
    }

    /// Simulated emitters (for the renderer).
    pub fn sim_emitters(&self) -> impl Iterator<Item = (u64, &Emitter)> {
        self.emitters.iter().filter_map(|(id, s)| match &s.body {
            Body::Sim(e) => Some((*id, &**e)),
            Body::Headless(..) => None,
        })
    }

    pub fn emitter(&self, id: u64) -> Option<&Emitter> {
        match &self.emitters.get(&id)?.body {
            Body::Sim(e) => Some(e),
            Body::Headless(..) => None,
        }
    }

    /// The linked systems an emitter started.
    pub fn linked(&self, id: u64) -> &[u64] {
        self.emitters.get(&id).map_or(&[], |s| &s.linked)
    }

    pub fn emitter_count(&self) -> usize {
        self.emitters.len()
    }

    /// Live particles over all simulated emitters.
    pub fn particle_count(&self) -> usize {
        self.sim_emitters().map(|(_, e)| e.particles.len()).sum()
    }
}

struct Start {
    duration: f32,
    may_release: bool,
    matrix: Mat4,
    velocity: Vec3,
}

/// Shared handle the adapter and the plugin use.
pub type SharedParticles = Arc<Mutex<ParticleManager>>;

pub struct ParticleComponent {
    name: String,
    duration: f32,
    /// Variation flags byte `#b0124c0a` bit 0 (default 1): may the emitter release itself.
    may_release: bool,
    manager: SharedParticles,
    ids: Vec<u64>,
    /// Emitters of an earlier start that stopped emitting but whose particles are still flying
    /// (a restarted muzzle flash keeps its shell casings).
    retired: Vec<u64>,
    matrix: Mat4,
    velocity: Vec3,
    enabled: bool,
}

impl ParticleComponent {
    pub fn new(name: String, duration: f32, manager: SharedParticles) -> Self {
        Self {
            name,
            duration,
            may_release: true,
            manager,
            ids: Vec::new(),
            retired: Vec::new(),
            matrix: Mat4::IDENTITY,
            velocity: Vec3::ZERO,
            enabled: true,
        }
    }

    /// The constructor registered for kind 1. `name`, `duration` and the flags byte are the
    /// variation's fields (doc 1.2, `smoke-effects.md` section 1).
    pub fn from_init(init: &ComponentInit, manager: SharedParticles) -> Box<dyn EffectComponent> {
        let n = &init.variation.node;
        let mut c = Self::new(
            n.str("name").unwrap_or("").to_string(),
            n.float("duration").unwrap_or(0.0).max(0.0),
            manager,
        );
        if let Some(Variant::Int(v)) = n.value_hash(RELEASE_FLAG_HASH) {
            c.may_release = v & 1 != 0;
        }
        Box::new(c)
    }

    /// Native component destruction / `Stop(immediate)`: stop the emitters and let the manager
    /// run them out; live particles are never removed.
    fn release_handle(&mut self) {
        if self.ids.is_empty() && self.retired.is_empty() {
            return;
        }
        let ids: Vec<u64> = self.ids.drain(..).chain(self.retired.drain(..)).collect();
        if let Ok(mut m) = self.manager.lock() {
            m.detach_emitters(ids);
        }
    }

    fn any_alive(&self) -> bool {
        self.manager.lock().is_ok_and(|m| {
            self.ids
                .iter()
                .chain(&self.retired)
                .any(|id| m.emitter_alive(*id))
        })
    }
}

impl Drop for ParticleComponent {
    fn drop(&mut self) {
        self.release_handle();
    }
}

impl EffectComponent for ParticleComponent {
    fn start(&mut self) -> bool {
        if !self.enabled {
            return false;
        }
        // doc 5.4: a live emitter just keeps running; a finished one is replaced.
        // Inferred (not native-proven, muzzle task 446ad726): "live" means still emitting. An
        // emitter whose window is over but whose particles still fly (a shell casing lives 1 s) is
        // retired, not waited for, so a gun firing faster than that restarts its flash every shot.
        let Ok(mut m) = self.manager.lock() else {
            return false;
        };
        if self.ids.iter().any(|id| m.emitter_emitting(*id)) {
            return true;
        }
        for id in std::mem::take(&mut self.ids) {
            if m.emitter_alive(id) {
                m.stop_emitter(id);
                self.retired.push(id);
            } else {
                m.kill_emitter(id);
            }
        }
        self.ids = m.start_emitter(
            &self.name,
            self.duration,
            self.may_release,
            self.matrix,
            self.velocity,
        );
        !self.ids.is_empty()
    }
    /// Graceful and immediate are the same for particles (`smoke-effects.md` section 4): the
    /// emitter stops births, what is alive finishes.
    fn stop(&mut self, _immediate: bool) {
        if let Ok(mut m) = self.manager.lock() {
            for id in &self.ids {
                m.stop_emitter(*id);
            }
        }
    }
    fn is_alive(&self) -> bool {
        self.any_alive()
    }
    fn update(&mut self, dt: f32) {
        if let Ok(mut m) = self.manager.lock() {
            for id in self.ids.iter().chain(&self.retired) {
                m.update_emitter(*id, dt);
            }
            let mut gone = Vec::new();
            self.retired.retain(|id| {
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
    fn set_transform(&mut self, m: &Mat4) {
        self.matrix = *m;
        if let Ok(mut mgr) = self.manager.lock() {
            for id in &self.ids {
                mgr.set_source(*id, *m, None);
            }
        }
    }
    fn set_velocity(&mut self, v: Vec3) {
        self.velocity = v;
        if let Ok(mut mgr) = self.manager.lock() {
            for id in &self.ids {
                mgr.set_source(*id, self.matrix, Some(v));
            }
        }
    }
    fn set_input(&mut self, _key: u32, _value: f32) {}
    fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr(names: &[&str]) -> SharedParticles {
        Arc::new(Mutex::new(ParticleManager::new(Some(
            names.iter().map(|s| s.to_string()).collect(),
        ))))
    }

    fn retail() -> Option<ParticleManager> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        Some(ParticleManager::from_game(&GameFs::open(dir).ok()?))
    }

    #[test]
    fn density_scale_matches_the_native_formula() {
        assert_eq!(density_scale(0), 1.0 - 1.0 / 1024.0);
        assert!((density_scale(511) - 0.5).abs() < 1e-6);
        // floor at 0.25 from 767 live emitters on
        assert!(density_scale(766) > 0.25);
        assert_eq!(density_scale(767), 0.25);
        assert_eq!(density_scale(1023), 0.25);
    }

    #[test]
    fn limit_and_unresolved_names() {
        let mut m = ParticleManager::new(Some(["fx_a".to_string()].into()));
        assert!(m.resolves("FX_A.psm"));
        assert!(!m.resolves(""));
        assert!(m.spawn("bloodsplat").is_none());
        for _ in 0..MAX_EMITTERS {
            assert!(m.spawn("fx_a").is_some());
        }
        assert!(m.spawn("fx_a").is_none());
        assert_eq!(m.stats.refused_full, 1);
        assert_eq!(m.stats.refused_unresolved_name, 1);
        assert_eq!(m.live(), MAX_EMITTERS);
    }

    #[test]
    fn emitter_lifetime_and_restart_rule() {
        // no leaf data: headless emitters that live for the variation duration
        let m = mgr(&["fx_a"]);
        let mut c = ParticleComponent::new("fx_a".into(), 1.0, m.clone());
        assert!(c.start());
        assert_eq!(m.lock().unwrap().live(), 1);
        // a second Start while alive keeps the emitter
        assert!(c.start());
        assert_eq!(m.lock().unwrap().live(), 1);
        c.update(0.6);
        assert!(c.is_alive());
        c.update(0.6);
        assert!(!c.is_alive());
        assert_eq!(m.lock().unwrap().live(), 0);
        // finished: Start replaces it
        assert!(c.start());
        assert!(c.is_alive());
        drop(c);
        assert_eq!(m.lock().unwrap().live(), 0);
    }

    #[test]
    fn dangling_name_does_not_start() {
        let m = mgr(&["fx_a"]);
        let mut c = ParticleComponent::new("fx_bulhit_flesh_small_01".into(), 1.0, m);
        assert!(!c.start());
        assert!(!c.is_alive());
    }

    #[test]
    fn retail_leaf_runs_chains_and_releases() {
        let Some(m) = retail() else { return };
        let m = Arc::new(Mutex::new(m));
        // the barrel explosion head names ten follow-on systems
        let mut c = ParticleComponent::new("fx_exp_barrel_01.psm".into(), 0.0, m.clone());
        c.set_transform(&Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0)));
        assert!(c.start());
        let (live, chained) = {
            let g = m.lock().unwrap();
            (g.live(), g.stats.chained)
        };
        assert!(live > 1 && chained as usize == live - 1, "{live} {chained}");
        for _ in 0..30 {
            c.update(1.0 / 60.0);
        }
        assert!(m.lock().unwrap().particle_count() > 0);
        // everything ends by itself (one-shot window), then releases the count
        for _ in 0..1200 {
            c.update(1.0 / 60.0);
        }
        assert!(!c.is_alive());
        assert_eq!(m.lock().unwrap().live(), 0);
        // graceful stop of a looping-style emitter drains, immediate stop removes
        let mut c2 = ParticleComponent::new("fx_vehicle_mc_smoke_01".into(), 0.0, m.clone());
        assert!(c2.start());
        for _ in 0..60 {
            c2.update(1.0 / 60.0);
        }
        assert!(c2.is_alive());
        c2.stop(false);
        for _ in 0..120 {
            c2.update(1.0 / 60.0);
        }
        assert!(!c2.is_alive());
        assert_eq!(m.lock().unwrap().live(), 0);
        assert_eq!(m.lock().unwrap().emitter_count(), live + c2.ids.len());
        drop(c2);
        drop(c);
        assert_eq!(m.lock().unwrap().emitter_count(), 0);
    }

    #[test]
    fn unparseable_leaf_is_refused_not_counted() {
        let Some(mut m) = retail() else { return };
        // 9 retail leaves do not parse (no texture string), e.g. fx_env_antenna_break_01
        let ids = m.start_emitter("fx_env_antenna_break_01", 0.0, false, Mat4::IDENTITY, Vec3::ZERO);
        assert!(ids.is_empty());
        assert_eq!(m.live(), 0);
        assert_eq!(m.stats.refused_unusable, 1);
        // a leaf with a non-positive base lifetime is legal and starts (dc95ceb0: the per-particle
        // lifetime is clamped to 0.001 s at birth)
        let ids = m.start_emitter("fx_fire_car_trail_05", 0.0, false, Mat4::IDENTITY, Vec3::ZERO);
        assert!(!ids.is_empty());
        for id in &ids {
            for _ in 0..30 {
                m.update_emitter(*id, 1.0 / 60.0);
            }
        }
    }

    #[test]
    fn the_grenade_head_starts_twelve_linked_systems_together() {
        let Some(mut m) = retail() else { return };
        let ids = m.start_emitter("fx_exp_grenade_01", 0.0, false, Mat4::IDENTITY, Vec3::ZERO);
        // the head and its 12 links (including fx_exp_grenade_12 and _13)
        assert_eq!(ids.len(), 13);
        assert_eq!(m.stats.chained, 12);
        // the first job is the first-frame hold, births start in the second
        run(&mut m, &ids, 2);
        assert!(m.particle_count() > 0);
    }

    /// Leaf names of the retail library, sorted.
    fn retail_names(m: &ParticleManager) -> Vec<String> {
        let mut v: Vec<String> = m.names.iter().flatten().cloned().collect();
        v.sort();
        v
    }

    /// Size and depth (levels, head = 1) of a leaf's linked-system tree.
    fn tree(m: &mut ParticleManager, name: &str) -> (usize, usize) {
        let Some(sys) = m
            .library
            .as_mut()
            .unwrap()
            .system(&ParticleManager::normalized(name))
        else {
            return (0, 0);
        };
        if sys.max_particles <= 0 {
            return (0, 0);
        }
        let (mut n, mut d) = (1, 1);
        for c in sys.children.clone() {
            let (cn, cd) = tree(m, &c);
            n += cn;
            d = d.max(cd + 1);
        }
        (n, d)
    }

    fn run(m: &mut ParticleManager, ids: &[u64], frames: usize) {
        for _ in 0..frames {
            for id in ids {
                m.update_emitter(*id, 1.0 / 60.0);
            }
        }
    }

    #[test]
    fn linked_systems_follow_the_heads_live_matrix_and_never_release_themselves() {
        let Some(mut m) = retail() else { return };
        // effect 150's smoke: a head and two linked smoke systems
        let ids = m.start_emitter(
            "fx_vehicle_smoke_medium_01",
            0.0,
            true,
            Mat4::IDENTITY,
            Vec3::ZERO,
        );
        assert_eq!(ids.len(), 3, "head + _02 + _04");
        assert_eq!(m.linked(ids[0]), &ids[1..]);
        let moved = Mat4::from_translation(Vec3::new(10.0, 0.0, 5.0));
        m.set_source(ids[0], moved, Some(Vec3::X));
        for id in &ids {
            assert_eq!(m.emitter(*id).unwrap().matrix, moved);
            assert_eq!(m.emitter(*id).unwrap().velocity, Vec3::X);
        }
        // children never release themselves, the unbounded head neither: all stay alive
        run(&mut m, &ids, 600);
        assert!(ids.iter().all(|id| m.emitter_alive(*id)));
        assert_eq!(m.live(), 3);
        // stopping the head stops the children; everything drains and releases
        m.stop_emitter(ids[0]);
        run(&mut m, &ids, 1200);
        assert!(ids.iter().all(|id| !m.emitter_alive(*id)));
        assert_eq!(m.live(), 0);
    }

    #[test]
    fn deep_linked_trees_start_whole() {
        let Some(mut m) = retail() else { return };
        let mut best = (0, 0, String::new());
        for n in retail_names(&m) {
            let (count, depth) = tree(&mut m, &n);
            if depth > best.1 || (depth == best.1 && count > best.0) {
                best = (count, depth, n);
            }
        }
        // linked-system trees are several levels deep (the doc's 8 levels also count trail edges)
        assert!(best.1 >= 5, "{best:?}");
        if best.0 < MAX_EMITTERS {
            let ids = m.start_emitter(&best.2, 0.0, true, Mat4::IDENTITY, Vec3::ZERO);
            assert_eq!(ids.len(), best.0, "{}", best.2);
        }
    }

    #[test]
    fn stop_never_kills_particles_and_dropping_the_component_detaches_them() {
        let Some(m) = retail() else { return };
        let m = Arc::new(Mutex::new(m));
        let mut c = ParticleComponent::new("fx_vehicle_mc_smoke_01".into(), 0.0, m.clone());
        assert!(c.start());
        for _ in 0..90 {
            c.update(1.0 / 60.0);
        }
        let before = m.lock().unwrap().particle_count();
        assert!(before > 10, "{before}");
        // Stop(immediate): births stop at once (unbounded window closed), nothing is removed
        c.stop(true);
        assert!(!m.lock().unwrap().emitter_emitting(c.ids[0]));
        assert_eq!(m.lock().unwrap().particle_count(), before);
        // dropping the handle keeps the particles; the manager runs them out
        drop(c);
        assert_eq!(m.lock().unwrap().particle_count(), before);
        assert!(m.lock().unwrap().detached_count() > 0);
        for _ in 0..180 {
            m.lock().unwrap().update_detached(1.0 / 60.0);
        }
        let g = m.lock().unwrap();
        assert_eq!((g.particle_count(), g.emitter_count(), g.live()), (0, 0, 0));
        assert_eq!(g.total_particles(), 0);
    }

    #[test]
    fn the_flags_byte_decides_whether_the_head_releases_itself() {
        let Some(m) = retail() else { return };
        let m = Arc::new(Mutex::new(m));
        // a bounded window (duration 0.3 s) over one-second particle lives
        let mut c = ParticleComponent::new("fx_vehicle_mc_smoke_01".into(), 0.3, m.clone());
        c.may_release = false;
        assert!(c.start());
        for _ in 0..300 {
            c.update(1.0 / 60.0);
        }
        // window over, no particle alive, but the flag byte is 0: the slot is held until Stop
        assert_eq!(m.lock().unwrap().particle_count(), 0);
        assert!(c.is_alive());
        assert_eq!(m.lock().unwrap().live(), c.ids.len());
        c.stop(false);
        assert!(!c.is_alive());
        c.update(1.0 / 60.0);
        assert_eq!(m.lock().unwrap().live(), 0);
        // the same with the default flag releases on its own
        let mut c = ParticleComponent::new("fx_vehicle_mc_smoke_01".into(), 0.3, m.clone());
        assert!(c.start());
        for _ in 0..300 {
            c.update(1.0 / 60.0);
        }
        assert!(!c.is_alive());
        assert_eq!(m.lock().unwrap().live(), 0);
    }

    #[test]
    fn a_stopped_bounded_emitter_keeps_its_window() {
        let Some(m) = retail() else { return };
        let m = Arc::new(Mutex::new(m));
        let mut c = ParticleComponent::new("fx_vehicle_mc_smoke_01".into(), 1.0, m.clone());
        assert!(c.start());
        for _ in 0..12 {
            c.update(1.0 / 60.0);
        }
        c.stop(false);
        let head = c.ids[0];
        let births = m.lock().unwrap().emitter(head).unwrap().births;
        for _ in 0..30 {
            c.update(1.0 / 60.0);
        }
        // it still births until its 1 s are over
        assert!(m.lock().unwrap().emitter(head).unwrap().births > births);
        for _ in 0..200 {
            c.update(1.0 / 60.0);
        }
        assert!(!c.is_alive());
    }

    #[test]
    fn every_particle_of_a_trail_leaf_gets_a_trail_that_follows_it() {
        let Some(mut m) = retail() else { return };
        let name = retail_names(&m)
            .into_iter()
            .find(|n| {
                m.library.as_mut().unwrap().system(n).is_some_and(|s| {
                    s.trail.is_some() && s.max_particles > 0 && s.children.is_empty()
                })
            })
            .expect("a retail leaf with a trail system");
        let matrix = Mat4::from_translation(Vec3::new(3.0, 2.0, 1.0));
        let ids = m.start_emitter(&name, 0.0, true, matrix, Vec3::ZERO);
        assert_eq!(ids.len(), 1);
        let head = ids[0];
        let mut followed = 0;
        for _ in 0..90 {
            m.update_emitter(head, 1.0 / 60.0);
            let e = m.emitter(head).unwrap();
            for p in &e.particles {
                if let Some(t) = p.trail {
                    // the trail sits at the particle and carries the particle's alpha
                    let te = m.emitter(t).expect("trail emitter");
                    assert!((te.matrix.w_axis.truncate() - e.world_position(p)).length() < 1e-3);
                    if p.alpha < 1.0 {
                        assert_eq!(te.system_scale, p.alpha);
                    }
                    followed += 1;
                }
            }
        }
        assert!(followed > 0 && m.emitter_count() > 1, "{name}");
        // stop: the head stops births, the trails stop with their particles, everything drains
        m.stop_emitter(head);
        run(&mut m, &[head], 2400);
        assert!(m.sim_finished(head), "{name}");
        assert_eq!(m.emitter_count(), 1, "trail chains end with their particles");
        assert_eq!(m.live(), 0);
    }

    #[test]
    fn retail_trail_leaves_are_parsed() {
        let Some(mut m) = retail() else { return };
        let n = retail_names(&m)
            .into_iter()
            .filter(|n| m.library.as_mut().unwrap().system(n).is_some_and(|s| s.trail.is_some()))
            .count();
        // 273 leaves name a trail system; the parser skips the few leaves that do not parse
        assert!((250..=273).contains(&n), "{n}");
        // leaves with debris slots exist (capacity at most 150)
        let debris: Vec<_> = retail_names(&m)
            .into_iter()
            .filter_map(|n| m.library.as_mut().unwrap().system(&n))
            .filter(|s| s.debris_slots)
            .collect();
        assert!(!debris.is_empty());
        assert!(debris.iter().all(|s| s.max_particles > 0));
    }
}
