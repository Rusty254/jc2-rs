//! Durable world-object identity and the reward ledger (JC2-028).
//!
//! Runtime indices (`Piece::unit`, a tile's `resources` position, entity IDs, source
//! generations) say which object *this session's* tile instance means; they are never
//! durable. An [`ObjectId`] keeps the full source tuple instead (design:
//! `research/formats/persistence-design.md`, "Stable object identities"):
//!
//! ```text
//! canonical container + member (table / exported entity)
//! namespace (tile placement, location, mission export)
//! authored identifier, else source-record ordinal
//! placement instance (mission exports: the spawn's base transform) + duplicate number
//! content fingerprint of the source set
//! ```
//!
//! The 64-bit hash of an id only accelerates lookup; the full tuple decides equality, so
//! a collision never merges two objects ([`Ledger::with_hasher`] lets the tests force
//! one). Identity is only valid inside one content revision: a source set whose
//! fingerprint differs from the recorded one is refused ([`IdentityError::RevisionChanged`])
//! until [`Ledger::reconcile`] is called with an explicit mapping, so a changed archive
//! never silently re-maps an old ordinal onto a new object.
//!
//! The [`Ledger`] holds the durable facts (collected, destroyed, rewarded) and runs the
//! atomic transaction [`Ledger::commit`]: ledger flags, wallet, Chaos and settlement
//! counter change together or not at all. Events name the object by an [`ObjectRef`]
//! (live source generation + table + runtime index); a generation that has been released
//! or replaced no longer resolves, so old events are rejected while the durable facts
//! stay. [`Ledger::snapshot`] / [`Ledger::restore`] install state without replaying any
//! award. Nothing here touches disk (JC2-029 owns the document and its migration).
//!
//! Native status: the identity *scheme* is the jc2-rs design above (non-native); the
//! source records it is built from (`CSpawnSet` exported entities, `CResourceItem`,
//! damage-control names) are native placement data. Whether retail rolls these facts back
//! on mission retry is unresolved (persistence-design.md, "unresolved retail behavior").

// `snapshot`/`restore`/`reconcile` and the report types are the interface JC2-029's disk
// document will call; until then only the tests use them.
#![allow(dead_code)]

use crate::chaos::{Chaos, Progress, Wallet};
use crate::world::{GameplaySourceKey, Streamer};
use bevy::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Where a record comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Namespace {
    /// A terrain tile's placement file.
    Tile,
    /// A registry location container (`locations.rs`).
    Location,
    /// An exported entity a mission puts down (`world::place_entity`).
    MissionExport,
}

/// The record table of a source set an object lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Table {
    /// Destructible units (damage controls).
    Unit,
    /// Weapon stashes (`CSpawnSet` + `CExportedEntityComponent`).
    Stash,
    /// Resource crates (`CResourceItem`).
    Resource,
}

impl Table {
    fn member(self) -> &'static str {
        match self {
            Table::Unit => "unit",
            Table::Stash => "stash",
            Table::Resource => "resource",
        }
    }
}

/// How the source names the object.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Locator {
    /// The authored name (not necessarily unique: see `ObjectId::duplicate`).
    Authored(String),
    /// The source-record ordinal, valid only inside the content revision.
    Ordinal(u32),
}

/// Full source identity of one world object.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId {
    /// Canonical container identity (`jc2_formats::location::container_identity`).
    pub container: String,
    pub namespace: Namespace,
    /// Table name, or for mission exports `<entity>/<table>`.
    pub member: String,
    pub locator: Locator,
    /// Logical placement instance of the source set ("" for a tile or location; a mission
    /// export uses its base transform, so two placements of one entity stay distinct and
    /// a retry of the same placement stays the same object).
    pub placement: String,
    /// Nth object with the same locator in its set (authored names repeat: ten barrels).
    pub duplicate: u32,
    /// Fingerprint of the source set's records.
    pub content: u64,
}

impl ObjectId {
    /// Canonical text of the whole tuple, for sets that key on strings (`Progress`).
    pub fn durable_key(&self) -> String {
        let loc = match &self.locator {
            Locator::Authored(a) => format!("a:{a}"),
            Locator::Ordinal(o) => format!("o:{o}"),
        };
        format!(
            "{}|{:?}|{}|{}|{}#{}|{:016x}",
            self.container,
            self.namespace,
            self.member,
            loc,
            self.placement,
            self.duplicate,
            self.content
        )
    }
}

/// A registered source set: the unit of content-revision checking.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceSet {
    pub container: String,
    pub namespace: Namespace,
    /// Mission exports: the entity container; otherwise empty.
    pub entity: String,
    /// Mission exports: the placement instance; otherwise empty.
    pub placement: String,
}

/// One source record of a set, as the loaders know it.
#[derive(Clone, Debug)]
pub struct Record {
    pub table: Table,
    /// Authored name when the source has one.
    pub authored: Option<String>,
    /// Source-record ordinal within its table in the set.
    pub ordinal: u32,
    /// What it is (entity container, resource kind …): part of the content fingerprint.
    pub asset: String,
    /// Authored position, part of the content fingerprint (finite, bit-compared).
    pub anchor: [f32; 3],
    /// Where the runtime keeps it right now (`Piece::unit`, index in the tile list).
    pub runtime_index: u32,
}

/// A runtime reference an event carries: live source generation, table, runtime index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectRef {
    pub source: u64,
    pub table: Table,
    pub index: u32,
}

impl ObjectRef {
    pub fn new(source: GameplaySourceKey, table: Table, index: u32) -> Self {
        Self {
            source: source.0,
            table,
            index,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityError {
    /// The set's content differs from the revision recorded earlier.
    RevisionChanged {
        set: SourceSet,
        recorded: u64,
        found: u64,
    },
    /// Two records of one table share an ordinal.
    DuplicateOrdinal { table: Table, ordinal: u32 },
    /// Two records claim one runtime slot of a source generation.
    DuplicateRuntimeIndex { table: Table, index: u32 },
    /// A snapshot lists one id twice.
    DuplicateEntry(ObjectId),
    /// A non-finite authored position.
    NonFinite,
}

/// What an object has gone through (durable).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fact {
    pub collected: bool,
    pub destroyed: bool,
    /// A reward transaction was applied for it.
    pub rewarded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Collect,
    Destroy,
}

/// What an object pays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reward {
    pub chaos: u32,
    pub money: u32,
    pub weapon_parts: u32,
    pub vehicle_parts: u32,
    pub armor_parts: u32,
    /// A sabotage target (`Chaos::sabotaged`).
    pub sabotage: bool,
    /// The settlement counter it counts towards.
    pub settlement: Option<String>,
}

impl Reward {
    fn needs_chaos(&self) -> bool {
        self.chaos > 0 || self.sabotage
    }
    fn needs_wallet(&self) -> bool {
        self.money > 0 || self.weapon_parts > 0 || self.vehicle_parts > 0 || self.armor_parts > 0
    }
}

/// The books a reward lands in. A missing book rejects a reward that needs it; the
/// settlement counter is derived progress and is skipped without `Progress`.
#[derive(Default)]
pub struct Books<'a> {
    pub chaos: Option<&'a mut Chaos>,
    pub wallet: Option<&'a mut Wallet>,
    pub progress: Option<&'a mut Progress>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reject {
    /// The reward needs a book that is not there.
    MissingBook(&'static str),
    /// The external effect (arsenal, …) refused.
    Gate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Ledger and books changed.
    Applied,
    /// The fact already stands: nothing changed.
    AlreadyDone,
    /// The handle's generation was released or replaced (or never registered).
    Stale,
    /// Nothing changed.
    Rejected(Reject),
}

struct Slot {
    id: ObjectId,
    fact: Fact,
}

/// What [`Ledger::reconcile`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    pub moved: usize,
    /// Facts of old objects the mapping did not carry over (they are forgotten: a
    /// deliberate loss the caller asked for).
    pub dropped: usize,
}

/// Counters for the tests and diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Lookups whose hash bucket held a different id.
    pub collisions: u32,
    pub rejected_revisions: u32,
}

/// Durable facts per full `ObjectId`, plus the live handles of loaded generations.
pub struct Ledger {
    slots: Vec<Slot>,
    /// Hash bucket → slot indices; only an accelerator.
    by_hash: HashMap<u64, Vec<usize>>,
    hasher: fn(&ObjectId) -> u64,
    revisions: HashMap<SourceSet, u64>,
    live: HashMap<ObjectRef, usize>,
    stats: Stats,
}

impl Default for Ledger {
    fn default() -> Self {
        Self::with_hasher(stable_hash)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(FNV_OFFSET)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(FNV_PRIME);
        }
    }
    /// Length-prefixed, so field boundaries cannot shift.
    fn text(&mut self, s: &str) {
        self.bytes(&(s.len() as u64).to_le_bytes());
        self.bytes(s.as_bytes());
    }
    fn num(&mut self, n: u64) {
        self.bytes(&n.to_le_bytes());
    }
}

/// FNV-1a over the whole tuple: stable across runs (unlike `DefaultHasher`).
pub fn stable_hash(id: &ObjectId) -> u64 {
    let mut h = Fnv::new();
    h.text(&id.container);
    h.num(id.namespace as u64);
    h.text(&id.member);
    match &id.locator {
        Locator::Authored(a) => {
            h.num(1);
            h.text(a);
        }
        Locator::Ordinal(o) => {
            h.num(2);
            h.num(u64::from(*o));
        }
    }
    h.text(&id.placement);
    h.num(u64::from(id.duplicate));
    h.num(id.content);
    h.0
}

/// Fingerprint of a source set's records (order, names, assets and positions).
pub fn fingerprint(records: &[Record]) -> u64 {
    let mut h = Fnv::new();
    h.num(records.len() as u64);
    for r in records {
        h.num(r.table as u64);
        match &r.authored {
            Some(a) => {
                h.num(1);
                h.text(a);
            }
            None => h.num(0),
        }
        h.num(u64::from(r.ordinal));
        h.text(&r.asset);
        for v in r.anchor {
            h.num(u64::from(v.to_bits()));
        }
    }
    h.0
}

impl Ledger {
    /// A ledger with another id hash (the tests force collisions with it).
    pub fn with_hasher(hasher: fn(&ObjectId) -> u64) -> Self {
        Self {
            slots: Vec::new(),
            by_hash: HashMap::new(),
            hasher,
            revisions: HashMap::new(),
            live: HashMap::new(),
            stats: Stats::default(),
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Number of distinct objects known.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn find(&mut self, id: &ObjectId) -> Option<usize> {
        let h = (self.hasher)(id);
        let bucket = self.by_hash.get(&h)?;
        for &i in bucket {
            if self.slots[i].id == *id {
                return Some(i);
            }
        }
        self.stats.collisions += 1;
        None
    }

    fn find_ro(&self, id: &ObjectId) -> Option<usize> {
        let h = (self.hasher)(id);
        self.by_hash
            .get(&h)?
            .iter()
            .copied()
            .find(|&i| self.slots[i].id == *id)
    }

    fn intern(&mut self, id: ObjectId, fact: Fact) -> usize {
        if let Some(i) = self.find(&id) {
            return i;
        }
        let h = (self.hasher)(&id);
        let i = self.slots.len();
        self.slots.push(Slot { id, fact });
        self.by_hash.entry(h).or_default().push(i);
        i
    }

    /// The ids of `records` in `set` at `content`; pure.
    fn make_ids(
        set: &SourceSet,
        records: &[Record],
        content: u64,
    ) -> Result<Vec<ObjectId>, IdentityError> {
        let mut seen_ordinals: HashSet<(Table, u32)> = HashSet::new();
        let mut repeats: HashMap<(Table, Locator), u32> = HashMap::new();
        let mut out = Vec::with_capacity(records.len());
        for r in records {
            if r.anchor.iter().any(|v| !v.is_finite()) {
                return Err(IdentityError::NonFinite);
            }
            if !seen_ordinals.insert((r.table, r.ordinal)) {
                return Err(IdentityError::DuplicateOrdinal {
                    table: r.table,
                    ordinal: r.ordinal,
                });
            }
            let locator = match r.authored.as_deref() {
                Some(a) if !a.is_empty() => Locator::Authored(a.to_string()),
                _ => Locator::Ordinal(r.ordinal),
            };
            let n = repeats.entry((r.table, locator.clone())).or_insert(0);
            let duplicate = *n;
            *n += 1;
            let member = if set.entity.is_empty() {
                r.table.member().to_string()
            } else {
                format!("{}/{}", set.entity, r.table.member())
            };
            out.push(ObjectId {
                container: set.container.clone(),
                namespace: set.namespace,
                member,
                locator,
                placement: set.placement.clone(),
                duplicate,
                content,
            });
        }
        Ok(out)
    }

    /// Register a source set of the live generation `source` and bind each record's
    /// runtime slot to its object. Registering the same set again (unload/reload, a
    /// replaced generation, a second owner) yields the same ids and keeps the facts. A
    /// set whose fingerprint differs from the recorded one is refused with nothing
    /// changed. Binding `source` replaces nothing of other generations.
    pub fn register(
        &mut self,
        set: &SourceSet,
        source: u64,
        records: &[Record],
    ) -> Result<Vec<ObjectId>, IdentityError> {
        let content = fingerprint(records);
        if let Some(&recorded) = self.revisions.get(set)
            && recorded != content
        {
            self.stats.rejected_revisions += 1;
            return Err(IdentityError::RevisionChanged {
                set: set.clone(),
                recorded,
                found: content,
            });
        }
        let ids = Self::make_ids(set, records, content)?;
        let mut taken = HashSet::new();
        for r in records {
            let h = ObjectRef {
                source,
                table: r.table,
                index: r.runtime_index,
            };
            if !taken.insert(h) || self.live.contains_key(&h) {
                return Err(IdentityError::DuplicateRuntimeIndex {
                    table: r.table,
                    index: r.runtime_index,
                });
            }
        }
        // validated: now mutate
        self.revisions.insert(set.clone(), content);
        for (r, id) in records.iter().zip(&ids) {
            let slot = self.intern(id.clone(), Fact::default());
            self.live.insert(
                ObjectRef {
                    source,
                    table: r.table,
                    index: r.runtime_index,
                },
                slot,
            );
        }
        Ok(ids)
    }

    /// The owner of generation `source` is gone: its handles stop resolving. Durable
    /// facts stay.
    pub fn release_source(&mut self, source: u64) {
        self.live.retain(|h, _| h.source != source);
    }

    /// Does generation `source` have live handles?
    #[cfg(test)]
    pub fn has_source(&self, source: u64) -> bool {
        self.live.keys().any(|h| h.source == source)
    }

    /// The object a live handle names.
    pub fn resolve(&self, h: ObjectRef) -> Option<&ObjectId> {
        self.live.get(&h).map(|&i| &self.slots[i].id)
    }

    /// The facts of a live handle.
    pub fn fact_of(&self, h: ObjectRef) -> Option<Fact> {
        self.live.get(&h).map(|&i| self.slots[i].fact)
    }

    /// The facts of an object by full id.
    pub fn fact(&self, id: &ObjectId) -> Option<Fact> {
        self.find_ro(id).map(|i| self.slots[i].fact)
    }

    /// The reward transaction. Order: resolve the handle (stale → nothing), check the
    /// fact (already done → nothing), check the books can take the reward, run `gate` (an
    /// external effect such as the arsenal taking the gun; `false` rejects), and only
    /// then change ledger and books together. Rejected and stale calls change neither.
    pub fn commit(
        &mut self,
        h: ObjectRef,
        event: Event,
        reward: Option<(&Reward, &mut Books)>,
        gate: impl FnOnce() -> bool,
    ) -> Outcome {
        let Some(&slot) = self.live.get(&h) else {
            return Outcome::Stale;
        };
        let fact = self.slots[slot].fact;
        let done = match event {
            Event::Collect => fact.collected,
            Event::Destroy => fact.destroyed,
        };
        if done {
            return Outcome::AlreadyDone;
        }
        if let Some((r, books)) = &reward {
            if r.needs_chaos() && books.chaos.is_none() {
                return Outcome::Rejected(Reject::MissingBook("chaos"));
            }
            if r.needs_wallet() && books.wallet.is_none() {
                return Outcome::Rejected(Reject::MissingBook("wallet"));
            }
        }
        if !gate() {
            return Outcome::Rejected(Reject::Gate);
        }
        // from here nothing can fail
        let key = self.slots[slot].id.durable_key();
        let f = &mut self.slots[slot].fact;
        match event {
            Event::Collect => f.collected = true,
            Event::Destroy => f.destroyed = true,
        }
        if let Some((r, books)) = reward {
            f.rewarded = true;
            if let Some(c) = books.chaos.as_mut() {
                c.add(r.chaos);
                if r.sabotage {
                    c.sabotaged += 1;
                }
            }
            if let Some(w) = books.wallet.as_mut() {
                w.money = w.money.saturating_add(r.money);
                w.weapon_parts = w.weapon_parts.saturating_add(r.weapon_parts);
                w.vehicle_parts = w.vehicle_parts.saturating_add(r.vehicle_parts);
                w.armor_parts = w.armor_parts.saturating_add(r.armor_parts);
            }
            if let (Some(id), Some(p)) = (&r.settlement, books.progress.as_deref_mut()) {
                p.count(id, key);
            }
        }
        Outcome::Applied
    }

    /// Explicit reconciliation of a changed source set: `records` is the new content;
    /// `map` says which new record (index into `records`) each old object becomes. Old
    /// objects of the set that the map does not carry are forgotten and counted. Nothing
    /// happens automatically: `register` refuses a changed set until this is called.
    pub fn reconcile(
        &mut self,
        set: &SourceSet,
        records: &[Record],
        map: &HashMap<ObjectId, usize>,
    ) -> Result<ReconcileReport, IdentityError> {
        let content = fingerprint(records);
        let new_ids = Self::make_ids(set, records, content)?;
        if map.values().any(|&i| i >= records.len()) {
            return Err(IdentityError::DuplicateOrdinal {
                table: Table::Unit,
                ordinal: u32::MAX,
            });
        }
        let mut targets = HashSet::new();
        for &i in map.values() {
            if !targets.insert(i) {
                return Err(IdentityError::DuplicateOrdinal {
                    table: records[i].table,
                    ordinal: records[i].ordinal,
                });
            }
        }
        let in_set = |id: &ObjectId| {
            id.container == set.container
                && id.namespace == set.namespace
                && id.placement == set.placement
                && match id.member.split_once('/') {
                    Some((e, _)) => e == set.entity,
                    None => set.entity.is_empty(),
                }
        };
        let old: Vec<usize> = (0..self.slots.len())
            .filter(|&i| in_set(&self.slots[i].id))
            .collect();
        let mut carried: Vec<(usize, Fact)> = Vec::new();
        let mut report = ReconcileReport::default();
        for &i in &old {
            match map.get(&self.slots[i].id) {
                Some(&to) => {
                    carried.push((to, self.slots[i].fact));
                    report.moved += 1;
                }
                None => report.dropped += 1,
            }
        }
        // rebuild without the old objects of the set, then add the new ones
        let keep: Vec<Slot> = std::mem::take(&mut self.slots)
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !old.contains(i))
            .map(|(_, s)| s)
            .collect();
        self.slots = keep;
        self.by_hash.clear();
        let reindex: Vec<(usize, u64)> = self
            .slots
            .iter()
            .enumerate()
            .map(|(i, s)| (i, (self.hasher)(&s.id)))
            .collect();
        for (i, h) in reindex {
            self.by_hash.entry(h).or_default().push(i);
        }
        // old live handles into the set are dead: the runtime re-registers
        self.live.clear();
        self.revisions.insert(set.clone(), content);
        for (i, id) in new_ids.into_iter().enumerate() {
            let fact = carried
                .iter()
                .find(|(to, _)| *to == i)
                .map(|(_, f)| *f)
                .unwrap_or_default();
            self.intern(id, fact);
        }
        Ok(report)
    }

    /// Every durable fact (sorted, deterministic) and the recorded revisions.
    pub fn snapshot(&self) -> Snapshot {
        let mut entries: Vec<(ObjectId, Fact)> = self
            .slots
            .iter()
            .filter(|s| s.fact != Fact::default())
            .map(|s| (s.id.clone(), s.fact))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let revisions: BTreeMap<SourceSet, u64> = self
            .revisions
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        Snapshot {
            entries,
            revisions: revisions.into_iter().collect(),
        }
    }

    /// Install a snapshot: facts and revisions, no award events, no wallet changes.
    /// Validated first (duplicate ids refuse); on error nothing changed. Facts of
    /// objects not registered yet wait for their source set; an id whose recorded
    /// revision disagrees with the snapshot's is refused like a changed set.
    pub fn restore(&mut self, snap: &Snapshot) -> Result<(), IdentityError> {
        let mut seen = HashSet::new();
        for (id, _) in &snap.entries {
            if !seen.insert(id) {
                return Err(IdentityError::DuplicateEntry(id.clone()));
            }
        }
        for (set, fp) in &snap.revisions {
            if let Some(&have) = self.revisions.get(set)
                && have != *fp
            {
                return Err(IdentityError::RevisionChanged {
                    set: set.clone(),
                    recorded: have,
                    found: *fp,
                });
            }
        }
        for (set, fp) in &snap.revisions {
            self.revisions.insert(set.clone(), *fp);
        }
        for (id, fact) in &snap.entries {
            let i = self.intern(id.clone(), Fact::default());
            let f = &mut self.slots[i].fact;
            f.collected |= fact.collected;
            f.destroyed |= fact.destroyed;
            f.rewarded |= fact.rewarded;
        }
        Ok(())
    }
}

/// The durable part of a [`Ledger`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub entries: Vec<(ObjectId, Fact)>,
    pub revisions: Vec<(SourceSet, u64)>,
}

/// The ledger as a resource.
#[derive(Resource, Default)]
pub struct ObjectLedger(pub Ledger);

/// Systems that bind loaded sources to the ledger run in this set.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentitySynced;

/// Registers every loaded gameplay source's objects and releases the dead ones.
pub struct IdentityPlugin;

impl Plugin for IdentityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ObjectLedger>()
            .init_resource::<SourceSync>()
            .add_systems(
                Update,
                (sync_sources, retire_destroyed_pieces)
                    .chain()
                    .in_set(IdentitySynced)
                    .after(crate::locations::LocationReconciled),
            );
    }
}

/// What the sync has done per live generation: how many of its units are registered
/// (mission exports append units late, in complete sets).
#[derive(Resource, Default)]
struct SourceSync {
    units_done: HashMap<u64, usize>,
}

/// Records of a loaded tile's own (non-mission) tables, with runtime indices.
fn tile_records(tile: &crate::world::LoadedTile) -> Vec<Record> {
    let mut out = Vec::new();
    for (i, u) in tile.units.iter().enumerate() {
        if u.export.is_some() {
            continue;
        }
        out.push(Record {
            table: Table::Unit,
            authored: Some(u.name.clone()),
            ordinal: u.source_ordinal,
            asset: String::new(),
            anchor: [u.center.x, u.center.y, u.center.z],
            runtime_index: i as u32,
        });
    }
    for (i, s) in tile.exported_spawns.iter().enumerate() {
        let t = s.world.translation();
        out.push(Record {
            table: Table::Stash,
            authored: Some(s.name.clone()),
            ordinal: i as u32,
            asset: s.entity.to_ascii_lowercase(),
            anchor: t,
            runtime_index: i as u32,
        });
    }
    for (i, r) in tile.resources.iter().enumerate() {
        out.push(Record {
            table: Table::Resource,
            authored: Some(r.name.clone()),
            ordinal: i as u32,
            asset: format!("{}:{:?}", r.entity.to_ascii_lowercase(), r.kind),
            anchor: r.world.translation(),
            runtime_index: i as u32,
        });
    }
    out
}

/// The mission-export sets among a tile's units from index `from` on, each complete
/// (`place_entity` appends one entity's units in one call).
fn export_sets(tile: &crate::world::LoadedTile, from: usize) -> Vec<(SourceSet, Vec<Record>)> {
    let mut groups: Vec<(SourceSet, Vec<Record>)> = Vec::new();
    for (i, u) in tile.units.iter().enumerate().skip(from) {
        let Some(e) = &u.export else { continue };
        let set = SourceSet {
            container: tile.source_container.clone(),
            namespace: Namespace::MissionExport,
            entity: e.entity.clone(),
            placement: e.placement.clone(),
        };
        let rec = Record {
            table: Table::Unit,
            authored: Some(u.name.clone()),
            ordinal: u.source_ordinal,
            asset: String::new(),
            anchor: [u.center.x, u.center.y, u.center.z],
            runtime_index: i as u32,
        };
        match groups.iter_mut().find(|(s, _)| *s == set) {
            Some((_, v)) => v.push(rec),
            None => groups.push((set, vec![rec])),
        }
    }
    groups
}

fn sync_sources(
    st: Option<ResMut<Streamer>>,
    mut ledger: ResMut<ObjectLedger>,
    mut sync: ResMut<SourceSync>,
) {
    let Some(mut st) = st else { return };
    let mut restore: Vec<(GameplaySourceKey, u32)> = Vec::new();
    let mut alive: HashSet<u64> = HashSet::new();
    {
        let st: &Streamer = &st;
        let loaded = st.loaded.values().map(|t| (Namespace::Tile, t)).chain(
            st.location_tiles
                .values()
                .map(|(_, t)| (Namespace::Location, t)),
        );
        for (ns, tile) in loaded {
            let Some(key) = tile.source_key else { continue };
            if !tile.gameplay_enabled {
                continue;
            }
            alive.insert(key.0);
            let done = sync.units_done.get(&key.0).copied();
            let mut sets: Vec<(SourceSet, Vec<Record>)> = Vec::new();
            if done.is_none() {
                sets.push((
                    SourceSet {
                        container: tile.source_container.clone(),
                        namespace: ns,
                        entity: String::new(),
                        placement: String::new(),
                    },
                    tile_records(tile),
                ));
            }
            sets.extend(export_sets(tile, done.unwrap_or(0)));
            for (set, records) in sets {
                match ledger.0.register(&set, key.0, &records) {
                    Ok(ids) => {
                        // objects already destroyed: not a new event, just state
                        for (r, id) in records.iter().zip(&ids) {
                            if r.table == Table::Unit
                                && set.namespace != Namespace::MissionExport
                                && ledger.0.fact(id).is_some_and(|f| f.destroyed)
                                && tile
                                    .units
                                    .get(r.runtime_index as usize)
                                    .is_some_and(|u| !u.destroyed && u.mission_owner.is_none())
                            {
                                restore.push((key, r.runtime_index));
                            }
                        }
                    }
                    Err(e) => {
                        warn!("identity: {} refused: {e:?}", set.container);
                    }
                }
            }
            sync.units_done.insert(key.0, tile.units.len());
        }
    }
    let st = &mut *st;
    for (key, index) in restore {
        let tile = st
            .loaded
            .values_mut()
            .chain(st.location_tiles.values_mut().map(|(_, t)| t))
            .find(|t| t.source_key == Some(key));
        if let Some(u) = tile.and_then(|t| t.units.get_mut(index as usize)) {
            u.destroyed = true;
            u.debris.clear();
            u.fires.clear();
        }
    }
    let dead: Vec<u64> = sync
        .units_done
        .keys()
        .copied()
        .filter(|k| !alive.contains(k))
        .collect();
    for k in dead {
        sync.units_done.remove(&k);
        ledger.0.release_source(k);
    }
}

/// Parts that appear for a unit the ledger says is already destroyed (a tile that reloaded
/// or a late asset) are removed again: the intact model must not come back.
fn retire_destroyed_pieces(
    mut commands: Commands,
    st: Option<Res<Streamer>>,
    pieces: Query<(Entity, &crate::world::Piece), Added<crate::world::Piece>>,
) {
    let Some(st) = st else { return };
    for (e, p) in &pieces {
        let tile = match p.source {
            Some(k) => st
                .gameplay_sources()
                .find(|(s, _, _)| *s == k)
                .map(|(_, _, t)| t),
            None => st.loaded.get(&p.tile),
        };
        if tile
            .and_then(|t| t.units.get(p.unit as usize))
            .is_some_and(|u| u.destroyed)
        {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(container: &str) -> SourceSet {
        SourceSet {
            container: container.into(),
            namespace: Namespace::Tile,
            entity: String::new(),
            placement: String::new(),
        }
    }

    fn rec(table: Table, name: &str, ordinal: u32, at: f32, index: u32) -> Record {
        Record {
            table,
            authored: Some(name.into()),
            ordinal,
            asset: "asset".into(),
            anchor: [at, 0.0, 0.0],
            runtime_index: index,
        }
    }

    fn barrels(indices: [u32; 2]) -> Vec<Record> {
        vec![
            rec(Table::Unit, "dac.barrel.1", 0, 1.0, indices[0]),
            rec(Table::Unit, "dac.barrel.1", 1, 2.0, indices[1]),
        ]
    }

    fn pump_reward() -> Reward {
        Reward {
            chaos: 100,
            money: 500,
            sabotage: true,
            settlement: Some("31x14".into()),
            ..Default::default()
        }
    }

    fn books<'a>(c: &'a mut Chaos, w: &'a mut Wallet, p: &'a mut Progress) -> Books<'a> {
        Books {
            chaos: Some(c),
            wallet: Some(w),
            progress: Some(p),
        }
    }

    fn h(source: u64, index: u32) -> ObjectRef {
        ObjectRef {
            source,
            table: Table::Unit,
            index,
        }
    }

    #[test]
    fn same_asset_two_placements_stay_distinct() {
        let mut l = Ledger::default();
        let ids = l.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        assert_ne!(ids[0], ids[1]);
        assert_eq!(ids[0].duplicate, 0);
        assert_eq!(ids[1].duplicate, 1);
        assert_eq!(l.len(), 2);
        // a mission export placed twice at different bases is two sets
        let mut a = set("t");
        a.namespace = Namespace::MissionExport;
        a.entity = "silo.ee".into();
        a.placement = "base-a".into();
        let mut b = a.clone();
        b.placement = "base-b".into();
        let one = [rec(Table::Unit, "dac.silo", 0, 5.0, 10)];
        let ia = l.register(&a, 1, &one).unwrap();
        let one_b = [rec(Table::Unit, "dac.silo", 0, 5.0, 11)];
        let ib = l.register(&b, 1, &one_b).unwrap();
        assert_ne!(ia[0], ib[0]);
        assert_eq!(l.len(), 4);
    }

    #[test]
    fn runtime_order_does_not_change_identity_and_shared_owners_dedupe() {
        let mut l = Ledger::default();
        let first = l.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        // a second owner / a replaced generation lists the same source set at other
        // runtime indices (a mission export pushed units in front of it)
        let second = l.register(&set("t"), 2, &barrels([7, 8])).unwrap();
        assert_eq!(first, second);
        assert_eq!(l.len(), 2, "one original object, one slot");
        assert_eq!(l.resolve(h(2, 8)), Some(&first[1]));
        assert_eq!(l.resolve(h(1, 1)), Some(&first[1]));
    }

    #[test]
    fn changed_content_is_refused_until_reconciled() {
        let mut l = Ledger::default();
        let ids = l.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        let mut b = Books::default();
        let (mut c, mut w, mut p) = (Chaos::default(), Wallet::default(), Progress::default());
        b.chaos = Some(&mut c);
        b.wallet = Some(&mut w);
        b.progress = Some(&mut p);
        assert_eq!(
            l.commit(
                h(1, 0),
                Event::Destroy,
                Some((&pump_reward(), &mut b)),
                || true
            ),
            Outcome::Applied
        );
        l.release_source(1);
        // the archive now lists the barrels in the other order
        let swapped = vec![
            rec(Table::Unit, "dac.barrel.1", 0, 2.0, 0),
            rec(Table::Unit, "dac.barrel.1", 1, 1.0, 1),
        ];
        let err = l.register(&set("t"), 2, &swapped).unwrap_err();
        assert!(matches!(err, IdentityError::RevisionChanged { .. }));
        assert_eq!(l.stats().rejected_revisions, 1);
        assert!(
            l.resolve(h(2, 0)).is_none(),
            "nothing bound by a refused set"
        );
        // the old facts survive untouched
        assert!(l.fact(&ids[0]).unwrap().destroyed);
        // deliberate reconciliation: old barrel 0 (x=1) is the new record 1
        let mut map = HashMap::new();
        map.insert(ids[0].clone(), 1usize);
        let report = l.reconcile(&set("t"), &swapped, &map).unwrap();
        assert_eq!(
            report,
            ReconcileReport {
                moved: 1,
                dropped: 1
            }
        );
        let new_ids = l.register(&set("t"), 3, &swapped).unwrap();
        assert!(l.fact(&new_ids[1]).unwrap().destroyed, "carried by the map");
        assert!(!l.fact(&new_ids[0]).unwrap().destroyed);
    }

    #[test]
    fn duplicate_and_invalid_records_are_explicit() {
        let mut l = Ledger::default();
        let dup = vec![
            rec(Table::Unit, "a", 3, 0.0, 0),
            rec(Table::Unit, "b", 3, 0.0, 1),
        ];
        assert_eq!(
            l.register(&set("t"), 1, &dup),
            Err(IdentityError::DuplicateOrdinal {
                table: Table::Unit,
                ordinal: 3
            })
        );
        let same_slot = vec![
            rec(Table::Unit, "a", 0, 0.0, 4),
            rec(Table::Unit, "b", 1, 0.0, 4),
        ];
        assert!(matches!(
            l.register(&set("t"), 1, &same_slot),
            Err(IdentityError::DuplicateRuntimeIndex { .. })
        ));
        let nan = vec![rec(Table::Unit, "a", 0, f32::NAN, 0)];
        assert_eq!(
            l.register(&set("t"), 1, &nan),
            Err(IdentityError::NonFinite)
        );
        assert!(l.is_empty(), "refused sets register nothing");
        // unnamed records fall back to the ordinal
        let unnamed = vec![Record {
            authored: None,
            ..rec(Table::Stash, "", 5, 0.0, 0)
        }];
        let ids = l.register(&set("t"), 1, &unnamed).unwrap();
        assert_eq!(ids[0].locator, Locator::Ordinal(5));
    }

    fn colliding(_: &ObjectId) -> u64 {
        7
    }

    #[test]
    fn hash_collisions_are_resolved_by_the_full_tuple() {
        let mut l = Ledger::with_hasher(colliding);
        let ids = l.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        assert_eq!(l.len(), 2);
        let mut b = Books::default();
        let mut c = Chaos::default();
        let mut w = Wallet::default();
        b.chaos = Some(&mut c);
        b.wallet = Some(&mut w);
        let r = Reward {
            chaos: 5,
            ..Default::default()
        };
        assert_eq!(
            l.commit(h(1, 1), Event::Destroy, Some((&r, &mut b)), || true),
            Outcome::Applied
        );
        assert!(l.fact(&ids[1]).unwrap().destroyed);
        assert!(
            !l.fact(&ids[0]).unwrap().destroyed,
            "neighbour in the same bucket"
        );
        assert!(l.stats().collisions > 0);
        // registering again finds the same slots in the shared bucket
        l.register(&set("t"), 2, &barrels([3, 4])).unwrap();
        assert_eq!(l.len(), 2);
    }

    #[test]
    fn repeated_overlapping_and_stale_events_award_once() {
        let mut l = Ledger::default();
        l.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        let (mut c, mut w, mut p) = (Chaos::default(), Wallet::default(), Progress::default());
        let r = pump_reward();
        {
            let mut b = books(&mut c, &mut w, &mut p);
            assert_eq!(
                l.commit(h(1, 0), Event::Destroy, Some((&r, &mut b)), || true),
                Outcome::Applied
            );
            for _ in 0..3 {
                assert_eq!(
                    l.commit(h(1, 0), Event::Destroy, Some((&r, &mut b)), || true),
                    Outcome::AlreadyDone
                );
            }
        }
        assert_eq!((c.total, c.sabotaged, w.money), (100, 1, 500));
        assert_eq!(p.done("31x14"), 1);
        // generation 1 replaced by generation 2: the old token is dead, the fact stays
        l.release_source(1);
        l.register(&set("t"), 2, &barrels([0, 1])).unwrap();
        {
            let mut b = books(&mut c, &mut w, &mut p);
            assert_eq!(
                l.commit(h(1, 1), Event::Destroy, Some((&r, &mut b)), || true),
                Outcome::Stale
            );
            assert_eq!(
                l.commit(h(2, 0), Event::Destroy, Some((&r, &mut b)), || true),
                Outcome::AlreadyDone,
                "the same object under the new generation"
            );
            assert_eq!(
                l.commit(h(2, 1), Event::Destroy, Some((&r, &mut b)), || true),
                Outcome::Applied,
                "the other barrel is a different object"
            );
        }
        assert_eq!((c.total, w.money), (200, 1000));
        assert_eq!(p.done("31x14"), 2);
    }

    #[test]
    fn rejected_transactions_change_nothing() {
        let mut l = Ledger::default();
        l.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        let (mut c, mut w, mut p) = (Chaos::default(), Wallet::default(), Progress::default());
        let r = pump_reward();
        // an external effect that refuses
        {
            let mut b = books(&mut c, &mut w, &mut p);
            assert_eq!(
                l.commit(h(1, 0), Event::Destroy, Some((&r, &mut b)), || false),
                Outcome::Rejected(Reject::Gate)
            );
        }
        // a missing wallet
        {
            let mut b = Books {
                chaos: Some(&mut c),
                wallet: None,
                progress: Some(&mut p),
            };
            assert_eq!(
                l.commit(h(1, 0), Event::Destroy, Some((&r, &mut b)), || true),
                Outcome::Rejected(Reject::MissingBook("wallet"))
            );
        }
        // an unregistered handle
        {
            let mut b = books(&mut c, &mut w, &mut p);
            assert_eq!(
                l.commit(h(9, 9), Event::Destroy, Some((&r, &mut b)), || true),
                Outcome::Stale
            );
        }
        assert_eq!(
            (c.total, c.sabotaged, w.money, p.done("31x14")),
            (0, 0, 0, 0)
        );
        assert_eq!(l.fact_of(h(1, 0)), Some(Fact::default()));
        // and a later good one still works
        let mut b = books(&mut c, &mut w, &mut p);
        assert_eq!(
            l.commit(h(1, 0), Event::Destroy, Some((&r, &mut b)), || true),
            Outcome::Applied
        );
    }

    #[test]
    fn restore_installs_facts_without_awards() {
        let mut l = Ledger::default();
        let ids = l.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        let (mut c, mut w, mut p) = (Chaos::default(), Wallet::default(), Progress::default());
        {
            let mut b = books(&mut c, &mut w, &mut p);
            l.commit(
                h(1, 1),
                Event::Destroy,
                Some((&pump_reward(), &mut b)),
                || true,
            );
        }
        let snap = l.snapshot();
        assert_eq!(snap.entries.len(), 1);
        // a fresh session installs it; later registration sees the fact, books untouched
        let mut fresh = Ledger::default();
        fresh.restore(&snap).unwrap();
        let (mut c2, mut w2, mut p2) = (Chaos::default(), Wallet::default(), Progress::default());
        let again = fresh.register(&set("t"), 5, &barrels([2, 3])).unwrap();
        assert_eq!(again, ids);
        assert!(fresh.fact(&again[1]).unwrap().destroyed);
        {
            let mut b = books(&mut c2, &mut w2, &mut p2);
            assert_eq!(
                fresh.commit(
                    h(5, 3),
                    Event::Destroy,
                    Some((&pump_reward(), &mut b)),
                    || true
                ),
                Outcome::AlreadyDone
            );
        }
        assert_eq!((c2.total, w2.money), (0, 0));
        // duplicates refuse atomically
        let mut bad = snap.clone();
        bad.entries.push(bad.entries[0].clone());
        let mut other = Ledger::default();
        assert!(matches!(
            other.restore(&bad),
            Err(IdentityError::DuplicateEntry(_))
        ));
        assert!(other.is_empty());
        // a snapshot from another content revision is refused against a registered set
        let mut changed = Ledger::default();
        changed.register(&set("t"), 1, &barrels([0, 1])).unwrap();
        let mut snap2 = snap.clone();
        snap2.revisions[0].1 ^= 1;
        assert!(matches!(
            changed.restore(&snap2),
            Err(IdentityError::RevisionChanged { .. })
        ));
    }

    #[test]
    fn durable_key_is_the_full_tuple() {
        let mut l = Ledger::default();
        let a = l.register(&set("x"), 1, &barrels([0, 1])).unwrap();
        let b = l
            .register(&set("y"), 1, &[rec(Table::Unit, "dac.barrel.1", 0, 1.0, 5)])
            .unwrap();
        assert_ne!(a[0].durable_key(), a[1].durable_key());
        assert_ne!(a[0].durable_key(), b[0].durable_key());
        assert_eq!(stable_hash(&a[0]), stable_hash(&a[0].clone()));
    }
}

#[cfg(test)]
mod ecs_tests {
    use super::*;
    use crate::Game;
    use crate::missions::MissionOwner;
    use crate::world::WorldConfig;
    use bevy::ecs::system::RunSystemOnce;
    use jc2_formats::missions::{ScriptKind, script_objects};
    use jc2_formats::placement::Transform3;

    /// Headless, tile 31,14: the proof mission's silo is put down twice at different bases
    /// (two placements of one asset) and once more at the first base by a retry (a new
    /// owner generation, later runtime slots). The two placements are different objects, the
    /// retry is the same objects, and every unit resolves to its full source tuple
    /// through the runtime slot the units landed in. Needs the tile's test data.
    #[test]
    fn mission_export_units_get_stable_placement_identities() {
        let Some(mut app) = crate::world::tests::tile_app(|app| {
            app.add_plugins(IdentityPlugin);
        }) else {
            return;
        };
        let fs = app.world().resource::<Game>().fs.clone();
        let file = jc2_formats::missions::mission_file(&fs, "f2t10").unwrap();
        let Some(entity) = script_objects(&file)
            .into_iter()
            .find_map(|o| match o.kind {
                ScriptKind::Spawn {
                    entity: Some(entity),
                    ..
                } => Some(entity),
                _ => None,
            })
        else {
            return;
        };
        let place = |app: &mut App, base: Transform3, owner: u64| -> Vec<u32> {
            let entity = entity.clone();
            app.world_mut()
                .run_system_once(
                    move |mut commands: Commands,
                          mut st: ResMut<Streamer>,
                          assets: Res<AssetServer>,
                          game: Res<Game>,
                          cfg: Res<WorldConfig>| {
                        crate::world::place_entity(
                            &mut st,
                            (31, 14),
                            &entity,
                            &base,
                            &mut commands,
                            &assets,
                            &game,
                            &cfg,
                            MissionOwner(owner),
                        )
                    },
                )
                .unwrap()
                .expect("the silo places")
        };
        let ids = |app: &App, units: &[u32]| -> Vec<ObjectId> {
            let st = app.world().resource::<Streamer>();
            let source = st.loaded[&(31, 14)].source_key.unwrap();
            let ledger = &app.world().resource::<ObjectLedger>().0;
            units
                .iter()
                .map(|&u| {
                    ledger
                        .resolve(ObjectRef::new(source, Table::Unit, u))
                        .expect("registered")
                        .clone()
                })
                .collect()
        };
        let base_a = Transform3::IDENTITY;
        let mut base_b = Transform3::IDENTITY;
        base_b.m[9] = 40.0;
        let a = place(&mut app, base_a, 1);
        let b = place(&mut app, base_b, 2);
        assert!(!a.is_empty());
        app.update();
        let (ida, idb) = (ids(&app, &a), ids(&app, &b));
        assert!(
            ida.iter()
                .all(|i| i.namespace == Namespace::MissionExport && !i.placement.is_empty())
        );
        assert!(ida.iter().zip(&idb).all(|(x, y)| x != y), "two placements");
        assert_eq!(
            ida.iter().map(|i| &i.locator).collect::<Vec<_>>(),
            idb.iter().map(|i| &i.locator).collect::<Vec<_>>(),
            "same asset, same source records"
        );
        // a retry: a new generation puts the entity down again, in later runtime slots
        let retry = place(&mut app, base_a, 3);
        assert!(retry.iter().all(|r| !a.contains(r)));
        app.update();
        assert_eq!(ids(&app, &retry), ida, "same logical objects, new slots");
        let own = app.world().resource::<ObjectLedger>().0.len();
        app.update();
        assert_eq!(
            app.world().resource::<ObjectLedger>().0.len(),
            own,
            "re-syncing registers nothing new"
        );
    }
}
