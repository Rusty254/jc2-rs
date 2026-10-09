//! Missions, strongholds and colonels, run from the game's data
//! (`research/formats/missions.md`, `jc2_formats::missions`).
//!
//! **Missions.** A mission is its script — the world objects of `<id>.bl.bl` — talking to
//! the mission system through events. [`MissionRun`] holds the objects and plays them:
//! event triggers pass events on, time triggers fire after their time, count triggers
//! count, shape triggers fire when Rico enters or leaves them, message objects set the
//! objective line, target indicators put up world markers, map icons put objective icons
//! on the minimap and the map, dialogues show their line as a subtitle for as long as its
//! cues last and then fire their "done" event, spawn sets put exported entities down (the
//! silos of `f2t10`, destructible through `destruction.rs`) and run the entities' own
//! objects. The mission system part is `missionrules.brf`: `msy.<objective>.complete`
//! from the script completes an objective, the rules then activate the next one
//! (`msy.<objective>.activated`), and the last completes the mission: the banner
//! ("Faction Mission Completed"), `missiondata.bin`'s reward (money, chaos, parts).
//! What the script asks of classes not run here (the AI teams of the bigger missions,
//! cutscenes, music, heat control, restart points beyond the start) is skipped.
//! `--mission <id>` starts one at its restart point.
//!
//! **Strongholds.** The nine settlements of kind 32: their base container loads with
//! their tile (`ExtraContainers`), their targets and crates count towards them through
//! their own counter (`f1s07.CountUp`, `chaos::Progress::alias`) out of the data's total
//! ("N % complete"). With every target that is loaded destroyed (or the data's total
//! reached), and the chaos at the takeover mission's unlock, Rico holds the stronghold
//! for [`DEFEND_SECONDS`] against the heat it raises; then it is the faction's: the
//! banner, the reward, the faction's icon on the maps, the government's spawn sets
//! switched off (`f1s07.before.disable`) and the faction's switched on (the takeover
//! mission's last objective's `msy.<obj>.completed`). The original runs a whole mission
//! for this (escort the technician, defend him); see `missions.md`.
//!
//! **Colonels.** `npc.rs` spawns them from the location data (`ColonelTag`); a kill pays
//! the data's activity ("Colonel Assassinated": +1000 chaos, $5000), counts towards the
//! settlement and shows "Colonel … Assassinated".

use crate::Game;
use crate::bitmap_text::BitmapText;
use crate::chaos::{Chaos, ChaosPopup, ChaosTables, Progress, Wallet};
use crate::character::Hero;
use crate::combat::Dead;
use crate::coords::{to_bevy, to_engine};
use crate::destruction::UnitDestroyed;
use crate::hud::{ExtraBlips, ExtraMarkers, MarkerView, Notices, Objective};
use crate::map::{MapData, MapPins};
use crate::npc::ColonelTag;
use crate::world::{ExtraContainers, Streamer, WorldConfig, place_entity};
use bevy::prelude::*;
use jc2_formats::missions::{
    MissionData, MissionKind, MissionRecord, Region, Rule, ScriptKind, ScriptObject, Stronghold,
    events, first_property_member, objective_chain, parse_bmf, parse_regions, parse_rules,
    script_objects, strongholds,
};
use jc2_formats::placement::{Transform3, bare_name};
use jc2_formats::text::{TextDb, strip_cues};
use std::collections::HashSet;

#[path = "mission_access.rs"]
pub mod access;
pub use access::MissionOwner;

/// How long Rico holds a stronghold before it is taken (s; assumed: the original's
/// defence is a scripted mission).
pub const DEFEND_SECONDS: f32 = 60.0;
/// The banner's life (s).
pub const BANNER_LIFE: f32 = 6.0;
/// A colonel's marker shows this close (m: his `CShapeTrigger`'s 40 m sphere in the data).
const COLONEL_MARKER: f32 = 40.0;
/// How far a stronghold reaches (m; `hud.rs`'s area radius for kind 32).
const STRONGHOLD_RADIUS: f32 = 300.0;
/// The objective icon (`icon_types.bin` kind 15).
const OBJECTIVE_ICON: &str = "hud_icon_objective_dif.dds";
/// What a colonel is drawn with on the maps (no icon kind of his own in the data).
const COLONEL_ICON: &str = "hud_icon_foot_red_dif.dds";

// ---------------------------------------------------------------------------------
// the data and the events
// ---------------------------------------------------------------------------------

/// Everything the mission system reads.
#[derive(Resource, Default)]
pub struct MissionDb {
    pub table: Vec<MissionRecord>,
    pub rules: Vec<Rule>,
    pub data: MissionData,
    pub regions: Vec<Region>,
    pub strongholds: Vec<Stronghold>,
    pub text: Option<TextDb>,
    pub catalog: access::MissionCatalog,
}

impl MissionDb {
    pub fn load(fs: &jc2_formats::GameFs) -> Self {
        let read = |n: &str| fs.read_inflated(n).ok();
        let mut db = Self {
            table: read("missions.bmf")
                .and_then(|d| parse_bmf(&d).ok())
                .unwrap_or_default(),
            rules: read("missionrules.brf")
                .and_then(|d| parse_rules(&d).ok())
                .unwrap_or_default(),
            data: read("missiondata.bin")
                .and_then(|d| MissionData::parse(&d).ok())
                .unwrap_or_default(),
            regions: read("regions.csv")
                .map(|d| parse_regions(&d))
                .unwrap_or_default(),
            strongholds: read("icons_global.bin")
                .and_then(|d| strongholds(&d).ok())
                .unwrap_or_default(),
            text: TextDb::load(fs, "en"),
            catalog: access::MissionCatalog::default(),
        };
        db.catalog = access::MissionCatalog::load(fs, &db);
        info!(
            "missions: {} in missions.bmf, {} rules, {} strongholds with {} regions, {} rewards",
            db.table.len(),
            db.rules.len(),
            db.strongholds.len(),
            db.regions.len(),
            db.data.rewards.len()
        );
        db
    }

    /// The text of `key` (cues taken out), else `fallback`.
    pub fn text(&self, key: &str, fallback: &str) -> String {
        self.text
            .as_ref()
            .and_then(|t| t.get(key))
            .map(strip_cues)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.to_string())
    }

    pub fn record(&self, id: &str) -> Option<&MissionRecord> {
        self.table.iter().find(|m| m.id.eq_ignore_ascii_case(id))
    }
}

/// Events waiting to be delivered (lower case, one each).
#[derive(Resource, Default, Debug)]
pub struct EventBus {
    pub queue: Vec<String>,
}

impl EventBus {
    /// Fire an event string (several events separated by spaces).
    pub fn fire(&mut self, s: &str) {
        self.queue.extend(events(s));
    }
}

/// Every event fired so far: spawn sets switch on and off by them (`npc.rs`), as the
/// stronghold's status router does in the original (`f1s07.before.enabled`).
#[derive(Resource, Default, Debug)]
pub struct WorldFlags {
    pub fired: HashSet<String>,
}

/// `--mission <id>`: start it once the world is there.
#[derive(Resource, Debug, Clone)]
pub struct MissionStart(pub String);

/// The banner in the middle of the screen: "Faction Mission Started / Offensive Action".
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct MissionBanner {
    pub title: String,
    pub subtitle: String,
    /// The reward lines under it ("+1 Weapon part").
    pub rewards: Vec<String>,
    pub completed: bool,
    pub age: f32,
    pub active: bool,
    /// Banners shown so far (tests).
    pub count: u32,
}

impl MissionBanner {
    pub fn show(&mut self, title: String, subtitle: String, completed: bool, rewards: Vec<String>) {
        *self = Self {
            title,
            subtitle,
            rewards,
            completed,
            age: 0.0,
            active: true,
            count: self.count + 1,
        };
    }

    pub fn alpha(&self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let fade_in = (self.age / 0.3).clamp(0.0, 1.0);
        let fade_out = ((BANNER_LIFE - self.age) / 0.8).clamp(0.0, 1.0);
        fade_in.min(fade_out)
    }
}

/// A dialogue's line at the bottom of the screen.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct Subtitle(pub Option<String>);

// ---------------------------------------------------------------------------------
// a running mission
// ---------------------------------------------------------------------------------

/// The events of an event string, scoped to an entity instance: a spawned entity's own
/// events (`e{gb017b.10x10({…})}::destroyed`) are its own, five silos fire five
/// different ones.
pub fn scoped(s: &str, scope: Option<u32>) -> Vec<String> {
    events(s)
        .map(|e| match scope {
            Some(n) if e.contains('{') => format!("{e}#{n}"),
            _ => e,
        })
        .collect()
}

/// One script object while the mission runs.
#[derive(Debug, Clone)]
pub struct RunObject {
    pub obj: ScriptObject,
    /// Absolute (engine axes): the object's world, under its entity's when it has one.
    pub world: Transform3,
    pub enabled: bool,
    pub scope: Option<u32>,
    enable_on: Vec<String>,
    disable_on: Vec<String>,
    /// What it reacts to (a trigger's input, a message's show event …).
    on: Vec<String>,
    /// A second input: a time trigger's stop, a count trigger's count-down.
    off: Vec<String>,
    /// What it fires.
    fire: Vec<String>,
    /// A shape's exit event.
    fire2: Vec<String>,
    pub timer: Option<f32>,
    pub count: i32,
    pub fired: bool,
    pub inside: bool,
    pub spawned: bool,
    /// The destructible units its entity put on a tile.
    pub units: Vec<crate::world::Piece>,
}

impl RunObject {
    pub fn new(obj: ScriptObject, base: Option<&Transform3>, scope: Option<u32>) -> Self {
        let sc = |s: &str| scoped(s, scope);
        let (on, off, fire, fire2) = match &obj.kind {
            ScriptKind::EventTrigger { on, fire } => (sc(on), vec![], sc(fire), vec![]),
            ScriptKind::TimeTrigger {
                start_on,
                stop_on,
                fire,
                ..
            } => (sc(start_on), sc(stop_on), sc(fire), vec![]),
            ScriptKind::CountTrigger {
                up_on,
                down_on,
                fire,
                ..
            } => (sc(up_on), sc(down_on), sc(fire), vec![]),
            ScriptKind::ShapeTrigger { enter, exit, .. } => (vec![], vec![], sc(enter), sc(exit)),
            ScriptKind::Message { show_on, .. } => (sc(show_on), vec![], vec![], vec![]),
            ScriptKind::Dialogue { play_on, done, .. } => (sc(play_on), vec![], sc(done), vec![]),
            ScriptKind::Spawn { spawned, .. } => (vec![], vec![], sc(spawned), vec![]),
            _ => (vec![], vec![], vec![], vec![]),
        };
        Self {
            world: base.map_or(obj.world, |b| obj.world.then(b)),
            enabled: obj.enabled,
            enable_on: sc(&obj.enable_on),
            disable_on: sc(&obj.disable_on),
            obj,
            scope,
            on,
            off,
            fire,
            fire2,
            timer: None,
            count: 0,
            fired: false,
            inside: false,
            spawned: false,
            units: Vec::new(),
        }
    }

    pub fn position(&self) -> Vec3 {
        to_bevy(self.world.translation())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Running,
    Completed,
    Failed,
}

/// The mission being played.
#[derive(Debug, Clone)]
pub struct MissionRun {
    pub id: String,
    pub owner: MissionOwner,
    /// Newly introduced script events: cancellation restores pre-existing world flags.
    pub event_flags: HashSet<String>,
    pub kind: MissionKind,
    /// The objectives, in order (`missionrules.brf`).
    pub chain: Vec<String>,
    /// The rule variables set (lower case: `f2t10obj01.completed`).
    pub vars: HashSet<String>,
    pub current: Option<String>,
    pub objects: Vec<RunObject>,
    pub state: RunState,
    /// The objective line the script shows.
    pub objective: Option<String>,
    /// Objectives completed so far, in order.
    pub completed: Vec<String>,
    next_scope: u32,
}

impl MissionRun {
    pub fn new(id: &str, kind: MissionKind, rules: &[Rule], objects: Vec<ScriptObject>) -> Self {
        Self {
            id: id.to_string(),
            owner: MissionOwner(0),
            event_flags: HashSet::new(),
            kind,
            chain: objective_chain(rules, id),
            vars: HashSet::new(),
            current: None,
            objects: objects
                .into_iter()
                .map(|o| RunObject::new(o, None, None))
                .collect(),
            state: RunState::Running,
            objective: None,
            completed: Vec::new(),
            next_scope: 1,
        }
    }

    /// Start does not assert resource readiness. The launch service acknowledges the
    /// supported resource contract separately before objectives can activate.
    pub fn start(&mut self, rules: &[Rule]) -> Vec<String> {
        let id = self.id.to_ascii_lowercase();
        self.vars.insert(format!("{id}.active"));
        self.run_rules(rules)
    }

    pub fn resources_loaded(&mut self, rules: &[Rule]) -> Vec<String> {
        let id = self.id.to_ascii_lowercase();
        self.vars.insert(format!("{id}.resource_loaded"));
        self.run_rules(rules)
    }

    fn owns_event(&self, e: &str) -> bool {
        matches!(e, "msy.mission.fail" | "mission.fail")
            || e.starts_with(&format!("msy.{}.", self.id.to_ascii_lowercase()))
            || self
                .chain
                .iter()
                .any(|o| e.starts_with(&format!("msy.{}.", o.to_ascii_lowercase())))
            || self.objects.iter().any(|o| {
                [
                    &o.enable_on,
                    &o.disable_on,
                    &o.on,
                    &o.off,
                    &o.fire,
                    &o.fire2,
                ]
                .iter()
                .any(|list| list.iter().any(|event| event == e))
            })
    }

    /// Evaluate the mission's rules until nothing changes: an objective that becomes
    /// active is announced (`msy.<obj>.activated`, `msy.<obj>.checkpoint`), the mission's
    /// own `COMPLETED` ends it.
    fn run_rules(&mut self, rules: &[Rule]) -> Vec<String> {
        let id = self.id.to_ascii_lowercase();
        let mut out = Vec::new();
        for _ in 0..64 {
            let mut changed = false;
            for r in rules {
                if !r.name.to_ascii_lowercase().starts_with(&id) {
                    continue;
                }
                if !r
                    .condition
                    .eval(&|v: &str| self.vars.contains(&v.to_ascii_lowercase()))
                {
                    continue;
                }
                for o in &r.outputs {
                    let v = o.to_ascii_lowercase();
                    if !self.vars.insert(v.clone()) {
                        continue;
                    }
                    changed = true;
                    if let Some(obj) = v.strip_suffix(".active") {
                        self.current = self
                            .chain
                            .iter()
                            .find(|c| c.eq_ignore_ascii_case(obj))
                            .cloned()
                            .or_else(|| Some(obj.to_string()));
                        out.push(format!("msy.{obj}.activated"));
                        out.push(format!("msy.{obj}.checkpoint"));
                    } else if v == format!("{id}.completed") {
                        self.state = RunState::Completed;
                        out.push(format!("msy.{id}.completed"));
                    }
                }
            }
            if !changed {
                break;
            }
        }
        out
    }

    /// Deliver one event: the mission system's own, then every object. Returns what that
    /// fires in turn.
    pub fn deliver(
        &mut self,
        e: &str,
        rules: &[Rule],
        db: &MissionDb,
        sub: &mut Subtitle,
    ) -> Vec<String> {
        let mut out = Vec::new();
        if self.state != RunState::Running {
            return out;
        }
        // the mission system: an objective completed
        if let Some(obj) = e
            .strip_prefix("msy.")
            .and_then(|r| r.strip_suffix(".complete"))
        {
            let key = format!("{obj}.completed");
            if self.chain.iter().any(|c| c.eq_ignore_ascii_case(obj)) && !self.vars.contains(&key) {
                self.vars.insert(key);
                self.completed.push(obj.to_string());
                out.push(format!("msy.{obj}.completed"));
                out.extend(self.run_rules(rules));
            }
        }
        if matches!(e, "msy.mission.fail" | "mission.fail") {
            self.state = RunState::Failed;
            return out;
        }
        for o in &mut self.objects {
            if o.enable_on.iter().any(|x| x == e) {
                o.enabled = true;
            }
            if o.disable_on.iter().any(|x| x == e) {
                o.enabled = false;
                o.timer = None;
            }
            let hit = o.on.iter().any(|x| x == e);
            let hit_off = o.off.iter().any(|x| x == e);
            if !hit && !hit_off {
                continue;
            }
            match &o.obj.kind {
                ScriptKind::EventTrigger { .. } => {
                    if hit && o.enabled {
                        out.extend(o.fire.iter().cloned());
                    }
                }
                ScriptKind::TimeTrigger { time, .. } => {
                    if hit_off {
                        o.timer = None;
                    } else if o.enabled {
                        o.timer = Some(*time);
                    }
                }
                ScriptKind::CountTrigger { max, .. } => {
                    if !o.enabled {
                        continue;
                    }
                    if hit {
                        o.count += 1;
                    }
                    if hit_off {
                        o.count -= 1;
                    }
                    if o.count >= *max && !o.fired {
                        o.fired = true;
                        out.extend(o.fire.iter().cloned());
                    }
                }
                ScriptKind::Message { text, .. } => {
                    self.objective = Some(db.text(text, text));
                }
                ScriptKind::Dialogue { cue, .. } if o.enabled => {
                    let line = db.text(cue, "");
                    o.timer = Some(dialogue_seconds(
                        db.text.as_ref().and_then(|t| t.get(cue)).unwrap_or(""),
                    ));
                    sub.0 = (!line.is_empty()).then_some(line);
                }
                _ => {}
            }
        }
        out
    }

    /// Time passes: timers and dialogues run out and fire.
    pub fn tick(&mut self, dt: f32, sub: &mut Subtitle) -> Vec<String> {
        let mut out = Vec::new();
        for o in &mut self.objects {
            let Some(t) = o.timer.as_mut() else { continue };
            *t -= dt;
            if *t > 0.0 {
                continue;
            }
            out.extend(o.fire.iter().cloned());
            match &o.obj.kind {
                ScriptKind::TimeTrigger {
                    looped: true, time, ..
                } if *time > 0.0 => o.timer = Some(*time),
                ScriptKind::Dialogue { .. } => {
                    o.timer = None;
                    sub.0 = None;
                }
                _ => o.timer = None,
            }
        }
        out
    }

    /// Rico at engine position `p`: the shape triggers he enters or leaves fire.
    pub fn shapes(&mut self, p: [f32; 3]) -> Vec<String> {
        let mut out = Vec::new();
        for o in &mut self.objects {
            let ScriptKind::ShapeTrigger { shape, player, .. } = &o.obj.kind else {
                continue;
            };
            if !*player || !o.enabled {
                if o.inside && !o.enabled {
                    o.inside = false;
                }
                continue;
            }
            let t = o.world.translation();
            let d = [p[0] - t[0], p[1] - t[1], p[2] - t[2]];
            // the shape's frame: the rows are its axes (orthonormal)
            let local = [0, 1, 2].map(|i| {
                let r = o.world.row(i);
                let len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt().max(1e-6);
                (d[0] * r[0] + d[1] * r[1] + d[2] * r[2]) / len
            });
            let inside = shape.contains(local);
            if inside && !o.inside {
                out.extend(o.fire.iter().cloned());
            } else if !inside && o.inside {
                out.extend(o.fire2.iter().cloned());
            }
            o.inside = inside;
        }
        out
    }

    /// An exported entity's own objects join the script, scoped to this instance.
    pub fn add_entity(&mut self, objects: Vec<ScriptObject>, base: &Transform3) -> u32 {
        let scope = self.next_scope;
        self.next_scope += 1;
        self.objects.extend(
            objects
                .into_iter()
                .map(|o| RunObject::new(o, Some(base), Some(scope))),
        );
        scope
    }

    /// The markers of the enabled target indicators: where, what (text key).
    pub fn markers(&self) -> Vec<(Vec3, String)> {
        self.objects
            .iter()
            .filter(|o| o.enabled)
            .filter_map(|o| match &o.obj.kind {
                ScriptKind::TargetIndicator { text } => Some((o.position(), text.clone())),
                _ => None,
            })
            .collect()
    }

    /// The enabled objective icons.
    pub fn icons(&self) -> Vec<Vec3> {
        self.objects
            .iter()
            .filter(|o| o.enabled && matches!(o.obj.kind, ScriptKind::MapIcon { .. }))
            .map(RunObject::position)
            .collect()
    }

    /// A counter that has started: (label key, count, max).
    pub fn counter(&self) -> Option<(String, i32, i32)> {
        self.objects.iter().find_map(|o| match &o.obj.kind {
            ScriptKind::CountTrigger {
                max, text: Some(t), ..
            } if o.enabled && o.count > 0 && !o.fired => Some((t.clone(), o.count, *max)),
            _ => None,
        })
    }
}

/// How long a line plays: its last cue's end (`{0 - 2.4}… {2.6 - 6}`), else by its length
/// (15 characters a second), at least 2 s.
pub fn dialogue_seconds(text: &str) -> f32 {
    let mut end: f32 = 0.0;
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        let Some(j) = rest[i..].find('}') else { break };
        let cue = &rest[i + 1..i + j];
        if let Some((_, b)) = cue.split_once('-')
            && let Ok(v) = b.trim().parse::<f32>()
        {
            end = end.max(v);
        }
        rest = &rest[i + j + 1..];
    }
    if end > 0.0 {
        end
    } else {
        (strip_cues(text).chars().count() as f32 / 15.0).max(2.0)
    }
}

/// The mission being played, if any.
#[derive(Resource, Default, Debug)]
pub struct ActiveMission(pub Option<MissionRun>);

/// The missions completed (ids).
#[derive(Resource, Default, Debug)]
pub struct MissionsDone(pub Vec<String>);

/// What a mission pays when it is done (`missiondata.bin`); the reward lines for the
/// banner.
pub fn pay(
    kind: MissionKind,
    db: &MissionDb,
    chaos: &mut Chaos,
    wallet: &mut Wallet,
) -> Vec<String> {
    let Some(r) = db.data.reward(kind) else {
        return Vec::new();
    };
    chaos.add(r.chaos);
    wallet.money = wallet.money.saturating_add(r.money);
    wallet.weapon_parts += r.weapon_parts;
    wallet.vehicle_parts += r.vehicle_parts;
    wallet.armor_parts += r.armor_parts;
    let mut lines = Vec::new();
    if r.money > 0 {
        lines.push(format!("${}", r.money));
    }
    if r.chaos > 0 {
        lines.push(format!("+{} chaos", r.chaos));
    }
    if r.vehicle_parts > 0 {
        lines.push(format!("+{} Vehicle part", r.vehicle_parts));
    }
    if r.weapon_parts > 0 {
        lines.push(format!("+{} Weapon part", r.weapon_parts));
    }
    if r.armor_parts > 0 {
        lines.push(format!("+{} Armor part", r.armor_parts));
    }
    lines
}

// ---------------------------------------------------------------------------------
// strongholds
// ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The government's.
    Government,
    /// Its targets are down: Rico holds it.
    Defending,
    /// The faction's.
    Taken,
}

#[derive(Debug, Clone)]
pub struct StrongholdState {
    pub data: Stronghold,
    pub phase: Phase,
    /// Seconds of the defence left.
    pub defend_left: f32,
}

#[derive(Resource, Default, Debug)]
pub struct Strongholds(pub Vec<StrongholdState>);

// ---------------------------------------------------------------------------------
// the plugin
// ---------------------------------------------------------------------------------

pub struct MissionsPlugin;

impl Plugin for MissionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EventBus>()
            .init_resource::<WorldFlags>()
            .init_resource::<ActiveMission>()
            .init_resource::<MissionsDone>()
            .init_resource::<MissionBanner>()
            .init_resource::<Subtitle>()
            .init_resource::<Strongholds>()
            .init_resource::<ExtraContainers>()
            .init_resource::<access::MissionLaunches>()
            .add_message::<UnitDestroyed>()
            .add_systems(Startup, (load_missions, build_banner).chain())
            .add_systems(
                Update,
                (
                    access::service_launches,
                    access::prepare_location_resources,
                    spawn_entities,
                    run_mission,
                    pump_events,
                    finish_mission,
                    strongholds_progress,
                    colonel_kills,
                    feed_hud,
                    show_banner,
                )
                    .chain(),
            );
    }
}

/// Read the data; strongholds: their bases load with their tiles, their counters count
/// for them, the government holds them.
fn load_missions(
    mut commands: Commands,
    game: Option<Res<Game>>,
    mut extra: ResMut<ExtraContainers>,
    mut strongholds_res: ResMut<Strongholds>,
    progress: Option<ResMut<Progress>>,
    mut bus: ResMut<EventBus>,
) {
    let Some(game) = game else { return };
    let db = MissionDb::load(&game.fs);
    let mut progress = progress;
    for s in &db.strongholds {
        if let Some(t) = jc2_formats::terrain::tile_index(s.position[0], s.position[2]) {
            let base = s.base_container();
            if game.fs.contains(&base) {
                extra.0.entry(t).or_default().push(base);
            }
        }
        if let (Some(p), Some(c)) = (progress.as_mut(), s.counter()) {
            p.alias.insert(c.to_string(), s.id.clone());
            let e = p.totals.entry(s.id.clone()).or_insert(0);
            *e = (*e).max(s.total);
        }
        // the status router (`CMissionStatusRouter`): not taken yet
        bus.fire(&format!("{}.before.enabled", s.mission));
        strongholds_res.0.push(StrongholdState {
            data: s.clone(),
            phase: Phase::Government,
            defend_left: DEFEND_SECONDS,
        });
    }
    commands.insert_resource(db);
}

/// The script's spawn sets put their entities down when Rico comes within their
/// distance of a loaded tile.
#[allow(clippy::too_many_arguments)]
fn spawn_entities(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    game: Option<Res<Game>>,
    cfg: Option<Res<WorldConfig>>,
    st: Option<ResMut<Streamer>>,
    heroes: Query<&Transform, With<Hero>>,
    mut active: ResMut<ActiveMission>,
    mut bus: ResMut<EventBus>,
) {
    let (Some(assets), Some(game), Some(cfg), Some(mut st)) = (assets, game, cfg, st) else {
        return;
    };
    let Some(run) = active.0.as_mut() else { return };
    let Ok(hero) = heroes.single() else { return };
    let mut entities: Vec<(Vec<ScriptObject>, Transform3)> = Vec::new();
    for o in &mut run.objects {
        let ScriptKind::Spawn {
            entity: Some(entity),
            distance,
            ..
        } = &o.obj.kind
        else {
            continue;
        };
        if o.spawned || !o.enabled {
            continue;
        }
        let at = o.position();
        if at.distance(hero.translation) > *distance {
            continue;
        }
        let p = o.world.translation();
        let Some(key) = jc2_formats::terrain::tile_index(p[0], p[2]) else {
            continue;
        };
        let Some(units) = place_entity(
            &mut st,
            key,
            entity,
            &o.world,
            &mut commands,
            &assets,
            &game,
            &cfg,
            run.owner,
        ) else {
            continue;
        };
        o.spawned = true;
        o.units = units
            .into_iter()
            .map(|unit| crate::world::Piece {
                tile: key,
                unit,
                source: st.loaded[&key].source_key,
            })
            .collect();
        bus.queue.extend(o.fire.iter().cloned());
        // the entity's own objects (its markers, its relays)
        if let Ok(data) = game.fs.read_inflated(bare_name(entity))
            && let Some(file) = first_property_member(&data, &[".epe"])
        {
            entities.push((script_objects(&file), o.world));
        }
    }
    for (objs, base) in entities {
        let scope = run.add_entity(objs, &base);
        // the spawn set's units belong to the scope just made
        if let Some(o) = run
            .objects
            .iter_mut()
            .filter(|o| o.spawned && o.scope.is_none() && !o.units.is_empty())
            .find(|o| o.world == base)
        {
            o.scope = Some(scope);
        }
    }
}

/// Timers, Rico in the shapes, destroyed units.
fn run_mission(
    time: Res<Time>,
    heroes: Query<&Transform, With<Hero>>,
    mut destroyed: MessageReader<UnitDestroyed>,
    mut active: ResMut<ActiveMission>,
    mut bus: ResMut<EventBus>,
    mut sub: ResMut<Subtitle>,
) {
    let Some(run) = active.0.as_mut() else {
        destroyed.read().count();
        return;
    };
    if run.state != RunState::Running {
        destroyed.read().count();
        return;
    }
    let mut out = run.tick(time.delta_secs(), &mut sub);
    if let Ok(h) = heroes.single() {
        out.extend(run.shapes(to_engine(h.translation)));
    }
    for d in destroyed.read() {
        if d.mission_owner.is_some_and(|owner| owner != run.owner) {
            continue;
        }
        let key = d.piece;
        if let Some(scope) = run
            .objects
            .iter()
            .find(|o| o.units.contains(&key))
            .map(|o| o.scope)
        {
            for f in &d.fires {
                out.extend(scoped(f, scope));
            }
        }
    }
    bus.queue.extend(out);
}

/// Deliver the events: to the mission, and into the flags the spawn sets read.
fn pump_events(
    db: Option<Res<MissionDb>>,
    mut bus: ResMut<EventBus>,
    mut flags: ResMut<WorldFlags>,
    mut active: ResMut<ActiveMission>,
    mut sub: ResMut<Subtitle>,
) {
    let empty = MissionDb::default();
    let db = db.as_deref().unwrap_or(&empty);
    // a bound on the chain reactions of one frame
    for _ in 0..64 {
        if bus.queue.is_empty() {
            break;
        }
        let batch = std::mem::take(&mut bus.queue);
        for e in batch {
            if flags.fired.insert(e.clone())
                && let Some(run) = active.0.as_mut()
                && run.owns_event(&e)
            {
                run.event_flags.insert(e.clone());
            }
            if let Some(run) = active.0.as_mut() {
                let more = run.deliver(&e, &db.rules, db, &mut sub);
                bus.queue.extend(more);
            }
        }
    }
}

/// A mission that is done: the banner, the reward.
fn finish_mission(world: &mut World) {
    if !world.contains_resource::<MissionDb>() {
        return;
    }
    let Some(state) = world
        .resource::<ActiveMission>()
        .0
        .as_ref()
        .map(|r| r.state)
    else {
        return;
    };
    if state == RunState::Running {
        return;
    }
    let Some(run) = world.resource_mut::<ActiveMission>().0.take() else {
        return;
    };
    access::cleanup_owned(world, run.owner);
    access::clear_presentation(world);
    if state == RunState::Failed {
        access::clear_run_events(world, &run);
    }
    world.resource_scope(|world, db: Mut<MissionDb>| {
        let name = db.text(&format!("{}_name_txt", run.id), &run.id);
        match state {
            RunState::Completed => {
                // A debug replay and a stronghold's simplified takeover share this ledger.
                let first = !access::mission_completed(
                    &run.id,
                    world.resource::<MissionsDone>(),
                    world.resource::<Strongholds>(),
                );
                let rewards = if first
                    && world.contains_resource::<Chaos>()
                    && world.contains_resource::<Wallet>()
                {
                    world.resource_scope(|world, mut chaos: Mut<Chaos>| {
                        pay(
                            run.kind,
                            &db,
                            &mut chaos,
                            &mut world.resource_mut::<Wallet>(),
                        )
                    })
                } else {
                    Vec::new()
                };
                if first {
                    world.resource_mut::<MissionsDone>().0.push(run.id.clone());
                }
                let stronghold = if run.kind == MissionKind::Stronghold {
                    let mut holds = world.resource_mut::<Strongholds>();
                    holds
                        .0
                        .iter_mut()
                        .find(|s| s.data.mission.eq_ignore_ascii_case(&run.id))
                        .map(|s| {
                            s.phase = Phase::Taken;
                            s.data.clone()
                        })
                } else {
                    None
                };
                if let Some(s) = stronghold {
                    let mut bus = world.resource_mut::<EventBus>();
                    bus.fire(&format!("{}.before.disable", run.id));
                    bus.fire(&s.complete_event);
                    if let Some(last) = run.chain.last() {
                        bus.fire(&format!("msy.{last}.completed"));
                    }
                    if let Some(mut map) = world.get_resource_mut::<MapData>()
                        && let Some(faction) = s.faction()
                        && let Some(icon) = faction.stronghold_icon(true)
                        && let Some(settlement) = map.settlements.iter_mut().find(|x| x.id == s.id)
                    {
                        settlement.texture = Some(icon);
                    }
                };
                let (key, fallback) = run.kind.completed_text();
                world.resource_mut::<MissionBanner>().show(
                    db.text(key, fallback),
                    name,
                    true,
                    rewards,
                );
                info!("mission {} completed", run.id);
            }
            _ => {
                world.resource_mut::<MissionBanner>().show(
                    db.text("mission_msg_02", "Mission Failed"),
                    name,
                    false,
                    Vec::new(),
                );
                info!("mission {} failed", run.id);
            }
        }
    });
}

/// The strongholds' targets, the takeover and the defence.
#[allow(clippy::too_many_arguments)]
fn strongholds_progress(
    time: Res<Time>,
    db: Option<Res<MissionDb>>,
    st: Option<Res<Streamer>>,
    progress: Option<Res<Progress>>,
    mut list: ResMut<Strongholds>,
    mut bus: ResMut<EventBus>,
    mut banner: ResMut<MissionBanner>,
    mut objective: Option<ResMut<Objective>>,
    chaos: Option<ResMut<Chaos>>,
    wallet: Option<ResMut<Wallet>>,
    heat: Option<ResMut<crate::heat::Heat>>,
    map: Option<ResMut<MapData>>,
    heroes: Query<&Transform, With<Hero>>,
    mut missions_done: ResMut<MissionsDone>,
) {
    let (Some(db), Some(progress)) = (db, progress) else {
        return;
    };
    let dt = time.delta_secs();
    let hero = heroes.single().ok().map(|t| t.translation);
    let (mut chaos, mut wallet, mut heat, mut map) = (chaos, wallet, heat, map);
    for s in &mut list.0 {
        let id = s.data.id.clone();
        let mission = s.data.mission.to_ascii_lowercase();
        let near = hero.is_none_or(|h| {
            Vec2::new(h.x - s.data.position[0], h.z - s.data.position[2]).length()
                < STRONGHOLD_RADIUS
        });
        match s.phase {
            Phase::Government => {
                let done = progress.done(&id);
                // what of its targets is loaded now (the bases' and tiles' counters)
                let available: u32 = match (&st, s.data.counter()) {
                    (Some(st), Some(c)) => st
                        .gameplay_sources()
                        .map(|(_, _, t)| t.counters.get(c).copied().unwrap_or(0))
                        .sum(),
                    _ => 0,
                };
                let unlocked = db
                    .record(&mission)
                    .is_none_or(|r| chaos.as_ref().is_none_or(|c| c.total >= r.chaos));
                let all_down = done >= s.data.total || (available > 0 && done >= available);
                if done > 0 && all_down && unlocked && near {
                    s.phase = Phase::Defending;
                    s.defend_left = DEFEND_SECONDS;
                    banner.show(
                        db.text("mission_msg_16", "Stronghold Takeover Started"),
                        db.text(&format!("{mission}_title_txt"), &id),
                        false,
                        Vec::new(),
                    );
                    // the government answers
                    if let Some(h) = heat.as_mut() {
                        h.add(40.0);
                    }
                    info!("stronghold {id} ({mission}): its targets are down, hold it");
                }
            }
            Phase::Defending => {
                if !near {
                    // Rico left: the government holds it again
                    s.phase = Phase::Government;
                    if let Some(o) = objective.as_mut() {
                        o.0 = None;
                    }
                    continue;
                }
                s.defend_left -= dt;
                if let Some(o) = objective.as_mut() {
                    o.0 = Some(format!(
                        "{} {:.0} s",
                        db.text(&format!("{mission}_bar_txt"), "Securing Stronghold"),
                        s.defend_left.max(0.0).ceil()
                    ));
                }
                if s.defend_left <= 0.0 {
                    s.phase = Phase::Taken;
                    if let Some(o) = objective.as_mut() {
                        o.0 = None;
                    }
                    // what the takeover mission's end fires: the government's spawn
                    // sets go, the faction's come
                    bus.fire(&format!("{mission}.before.disable"));
                    if let Some(last) = objective_chain(&db.rules, &mission).last() {
                        bus.fire(&format!("msy.{last}.completed"));
                    }
                    bus.fire(&s.data.complete_event);
                    let first = !missions_done
                        .0
                        .iter()
                        .any(|id| id.eq_ignore_ascii_case(&mission));
                    let rewards = match (first, chaos.as_mut(), wallet.as_mut()) {
                        (true, Some(c), Some(w)) => pay(MissionKind::Stronghold, &db, c, w),
                        _ => Vec::new(),
                    };
                    if first {
                        missions_done.0.push(mission.clone());
                    }
                    banner.show(
                        db.text("mission_msg_11", "Stronghold Takeover Completed"),
                        db.text(&format!("{mission}_title_txt"), &id),
                        true,
                        rewards,
                    );
                    if let (Some(m), Some(f)) = (map.as_mut(), s.data.faction())
                        && let Some(icon) = f.stronghold_icon(true)
                        && let Some(set) = m.settlements.iter_mut().find(|x| x.id == id)
                    {
                        set.texture = Some(icon);
                    }
                    info!("stronghold {id} taken for the {:?}", s.data.faction());
                }
            }
            Phase::Taken => {}
        }
    }
}

/// A colonel down: the data's activity, the settlement's count, the message.
#[allow(clippy::too_many_arguments)]
fn colonel_kills(
    killed: Query<&ColonelTag, Added<Dead>>,
    db: Option<Res<MissionDb>>,
    tables: Option<Res<ChaosTables>>,
    chaos: Option<ResMut<Chaos>>,
    wallet: Option<ResMut<Wallet>>,
    popup: Option<ResMut<ChaosPopup>>,
    progress: Option<ResMut<Progress>>,
    notices: Option<ResMut<Notices>>,
    mut bus: ResMut<EventBus>,
) {
    let (mut chaos, mut wallet, mut popup, mut progress, mut notices) =
        (chaos, wallet, popup, progress, notices);
    for ColonelTag(c) in &killed {
        if let (Some(kind), Some(t), Some(ch), Some(w), Some(p)) = (
            c.activity,
            tables.as_deref(),
            chaos.as_mut(),
            wallet.as_mut(),
            popup.as_mut(),
        ) {
            crate::chaos::award(kind, t, ch, w, p);
        }
        if let (Some(counter), Some(p)) = (&c.counter, progress.as_mut()) {
            p.count(counter, format!("colonel {}", c.id));
        }
        let msg = db
            .as_ref()
            .map(|d| d.text(&c.message_key(), "Colonel Assassinated"))
            .unwrap_or_else(|| "Colonel Assassinated".into());
        if let Some(n) = notices.as_mut() {
            n.push(msg.clone());
        }
        bus.fire(&c.killed_event);
        if let Some(e) = &c.complete_event {
            bus.fire(e);
        }
        info!("colonel {} killed: {msg}", c.id);
    }
}

/// The objective line, the markers, the minimap and map icons.
#[allow(clippy::too_many_arguments)]
fn feed_hud(
    time: Res<Time>,
    db: Option<Res<MissionDb>>,
    active: Res<ActiveMission>,
    st: Option<Res<Streamer>>,
    flags: Res<WorldFlags>,
    heroes: Query<&Transform, With<Hero>>,
    colonels: Query<(&ColonelTag, &Transform), Without<Dead>>,
    objective: Option<ResMut<Objective>>,
    markers: Option<ResMut<ExtraMarkers>>,
    blips: Option<ResMut<ExtraBlips>>,
    pins: Option<ResMut<MapPins>>,
    mut banner: ResMut<MissionBanner>,
) {
    if banner.active {
        banner.age += time.delta_secs();
        if banner.age >= BANNER_LIFE {
            banner.active = false;
        }
    }
    let empty = MissionDb::default();
    let db = db.as_deref().unwrap_or(&empty);
    let here = heroes.single().ok().map(|t| t.translation);
    let mut m: Vec<MarkerView> = Vec::new();
    let mut b: Vec<(Vec3, String)> = Vec::new();
    let mut map_pins: Vec<(Vec3, String)> = Vec::new();
    if let Some(run) = &active.0 {
        for (p, text) in run.markers() {
            let d = here.map_or(0.0, |h| h.distance(p));
            m.push(MarkerView {
                world: p,
                label: format!("{} {d:.0} m", db.text(&text, ""))
                    .trim_start()
                    .to_string(),
                foe: true,
            });
        }
        for p in run.icons() {
            b.push((p, OBJECTIVE_ICON.to_string()));
            map_pins.push((p, OBJECTIVE_ICON.to_string()));
        }
        if let Some(mut o) = objective {
            let line = run.objective.clone().map(|t| match run.counter() {
                Some((key, n, max)) => format!("{t}  {} {n}/{max}", db.text(&key, "")),
                None => t,
            });
            if o.0 != line {
                o.0 = line;
            }
        }
    }
    // colonels: a marker close by, an icon on the maps for the ones the loaded tiles place
    for (ColonelTag(c), tf) in &colonels {
        if let Some(h) = here {
            let d = h.distance(tf.translation);
            if d < COLONEL_MARKER {
                m.push(MarkerView {
                    world: tf.translation + Vec3::Y * 1.2,
                    label: format!("{} {d:.0} m", db.text(&c.name_key(), "Colonel")),
                    foe: true,
                });
            }
        }
    }
    if let Some(st) = &st {
        for t in st.loaded.values() {
            for c in &t.colonels {
                if flags.fired.contains(&c.killed_event.to_ascii_lowercase()) {
                    continue;
                }
                let p = to_bevy(c.position);
                b.push((p, COLONEL_ICON.to_string()));
                map_pins.push((p, COLONEL_ICON.to_string()));
            }
        }
    }
    if let Some(mut x) = markers {
        x.0 = m;
    }
    if let Some(mut x) = blips {
        x.0 = b;
    }
    if let Some(mut x) = pins
        && x.0 != map_pins
    {
        x.0 = map_pins;
    }
}

// ---------------------------------------------------------------------------------
// the banner and the subtitle (Bevy UI, laid out like `hud.rs`)
// ---------------------------------------------------------------------------------

#[derive(Component)]
struct BannerRoot;
#[derive(Component)]
struct BannerTitle;
#[derive(Component)]
struct BannerSubtitle;
#[derive(Component)]
struct BannerRewards;
#[derive(Component)]
struct RewardsText;
#[derive(Component)]
struct SubtitleBox;
#[derive(Component)]
struct SubtitleText;

/// The banner: centred near the top (reference frame 2000 × 1116: 940 wide from y 265,
/// the title on the glass, the mission's name on a dark inner panel), the reward lines
/// low in the middle; the subtitle line at the bottom.
fn build_banner(mut commands: Commands) {
    use crate::bitmap_text::{HudFont, TextAlign};
    use crate::hud::{bt, bts, child, pill, r};
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: r(265.0),
                width: r(940.0),
                margin: UiRect::left(r(-470.0)),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(r(14.0)),
                row_gap: r(10.0),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(r(12.0)),
                display: Display::None,
                ..default()
            },
            pill(0.82),
            GlobalZIndex(40),
            BannerRoot,
            Name::new("mission banner"),
        ))
        .id();
    child(
        &mut commands,
        root,
        (
            bts(HudFont::Bold, 2.2),
            Node {
                margin: UiRect::left(r(170.0)),
                ..default()
            },
            BannerTitle,
        ),
    );
    let inner = child(
        &mut commands,
        root,
        (
            Node {
                padding: UiRect::new(r(170.0), r(14.0), r(8.0), r(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(r(6.0)),
                ..default()
            },
            pill(0.95),
        ),
    );
    child(
        &mut commands,
        inner,
        (bts(HudFont::Bold, 1.6), BannerSubtitle),
    );
    let rewards = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(20.0),
                width: Val::Percent(60.0),
                top: r(880.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(40),
            BannerRewards,
            Name::new("mission rewards"),
        ))
        .id();
    child(
        &mut commands,
        rewards,
        (bts(HudFont::Bold, 1.7), RewardsText),
    );
    let subtitle = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(15.0),
                width: Val::Percent(70.0),
                bottom: r(150.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(39),
            SubtitleBox,
            Name::new("subtitle"),
        ))
        .id();
    child(
        &mut commands,
        subtitle,
        (
            bt(HudFont::Subtitle, 1.3)
                .with_align(TextAlign::Center)
                .with_wrap(1400.0),
            SubtitleText,
        ),
    );
}

#[allow(clippy::type_complexity)]
fn show_banner(
    banner: Res<MissionBanner>,
    sub: Res<Subtitle>,
    mut nodes: ParamSet<(
        Query<&mut Node, With<BannerRoot>>,
        Query<&mut Node, With<BannerRewards>>,
        Query<&mut Node, With<SubtitleBox>>,
    )>,
    mut texts: ParamSet<(
        Query<&mut BitmapText, With<BannerTitle>>,
        Query<&mut BitmapText, With<BannerSubtitle>>,
        Query<&mut BitmapText, With<RewardsText>>,
        Query<&mut BitmapText, With<SubtitleText>>,
    )>,
) {
    let a = banner.alpha();
    let display = |on: bool| if on { Display::Flex } else { Display::None };
    for mut n in &mut nodes.p0() {
        n.display = display(a > 0.0);
    }
    for mut n in &mut nodes.p1() {
        n.display = display(a > 0.0 && !banner.rewards.is_empty());
    }
    for mut n in &mut nodes.p2() {
        n.display = display(sub.0.is_some());
    }
    if banner.is_changed() {
        for mut t in &mut texts.p0() {
            t.text = banner.title.clone();
        }
        for mut t in &mut texts.p1() {
            t.text = banner.subtitle.clone();
        }
        for mut t in &mut texts.p2() {
            t.text = banner.rewards.join("     ");
        }
    }
    if sub.is_changed() {
        for mut t in &mut texts.p3() {
            t.text = sub.0.clone().unwrap_or_default();
        }
    }
}

/// Where `--mission <id>` starts: the tile and engine `(x, z)` of the mission's restart
/// point (`jc2_formats::missions::start_point`).
pub fn mission_start_point(fs: &jc2_formats::GameFs, id: &str) -> Option<((u32, u32), [f32; 2])> {
    let objs = jc2_formats::missions::mission_script(fs, id).ok()?;
    let p = jc2_formats::missions::start_point(&objs)?;
    let tile = jc2_formats::terrain::tile_index(p[0], p[2])?;
    Some((tile, [p[0], p[2]]))
}

/// A colonel's world transform (Bevy), for the tests.
#[cfg(test)]
pub fn colonel_transform(c: &jc2_formats::missions::Colonel) -> Transform {
    let mut t = Transform3::IDENTITY;
    t.m[9..12].copy_from_slice(&c.position);
    crate::coords::mat_to_bevy(&t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chaos::ChaosPlugin;

    #[test]
    fn cues_and_scopes() {
        assert_eq!(dialogue_seconds("{0 - 2.4}Aha. {2.6 - 6}Little Guy."), 6.0);
        assert_eq!(dialogue_seconds("Get to it"), 2.0);
        assert_eq!(
            scoped(
                "e{gb017b.10x10({D8})}::destroyed f2t10.destructable.complete",
                Some(3)
            ),
            [
                "e{gb017b.10x10({d8})}::destroyed#3",
                "f2t10.destructable.complete"
            ]
        );
        let mut b = MissionBanner::default();
        b.show("A".into(), "B".into(), false, vec![]);
        assert_eq!((b.count, b.alpha()), (1, 0.0));
        b.age = 1.0;
        assert_eq!(b.alpha(), 1.0);
    }

    /// The headless app the mission tests share: the data, chaos and money, the mission
    /// system; no world, no Rico. `None` without the test data.
    fn mission_app() -> Option<App> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = std::sync::Arc::new(jc2_formats::GameFs::open(dir).ok()?);
        for f in [
            "missions.bmf",
            "missionrules.brf",
            "missiondata.bin",
            "f2t10.blz",
        ] {
            if !fs.contains(f) {
                return None;
            }
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(0.1),
            ))
            .insert_resource(Game { fs })
            .init_resource::<Objective>()
            .init_resource::<Notices>()
            .add_plugins((ChaosPlugin, MissionsPlugin));
        app.update();
        Some(app)
    }

    fn fire(app: &mut App, e: &str) {
        app.world_mut().resource_mut::<EventBus>().fire(e);
    }

    /// Run frames (0.1 s each; nothing loads on other threads here) until `done`.
    fn run_until(app: &mut App, what: &str, mut done: impl FnMut(&App) -> bool) {
        for _ in 0..2000 {
            app.update();
            if done(app) {
                return;
            }
        }
        panic!("timed out waiting for {what}");
    }

    fn objective(app: &App) -> Option<String> {
        app.world().resource::<Objective>().0.clone()
    }

    /// The proof mission, driven by the events its script and the world would fire:
    /// "Get to the military base" (the silos' spawn sets and the base's shape trigger),
    /// then "Destroy the silos" (five destroyed), then the dialogue, the completion, the
    /// banner and `missiondata.bin`'s faction-task reward. Needs JC2_TEST_DIR.
    #[test]
    fn the_proof_mission_runs_objective_by_objective() {
        let Some(mut app) = mission_app() else { return };
        app.insert_resource(MissionStart("f2t10".into()));
        app.update();
        {
            let run = app.world().resource::<ActiveMission>();
            let run = run.0.as_ref().expect("the mission runs");
            assert_eq!(run.chain, ["f2t10Obj01", "f2t10Obj02"]);
            assert_eq!(run.current.as_deref(), Some("f2t10Obj01"));
        }
        let b = app.world().resource::<MissionBanner>().clone();
        assert!(b.active && !b.completed);
        assert_eq!(b.title, "Faction Mission Started");
        assert_eq!(b.subtitle, "Offensive Action");
        // 4.5 s after the objective's activation its line, marker and icon show
        run_until(&mut app, "the first objective", |a| {
            objective(a).as_deref() == Some("Get to the military base")
        });
        {
            let run = app.world().resource::<ActiveMission>();
            let run = run.0.as_ref().unwrap();
            assert!(!run.markers().is_empty(), "a marker on the base");
            assert!(!run.icons().is_empty(), "an objective icon");
        }
        // the five silos stand and Rico enters the base
        for _ in 0..5 {
            fire(&mut app, "f2t10.destructable.spawned");
        }
        fire(&mut app, "f2t10.base.entered");
        run_until(&mut app, "the second objective", |a| {
            a.world()
                .resource::<ActiveMission>()
                .0
                .as_ref()
                .is_some_and(|r| r.current.as_deref() == Some("f2t10Obj02"))
        });
        // the employer's line plays, then "Destroy the silos"
        run_until(&mut app, "the second objective's line", |a| {
            objective(a).is_some_and(|o| o.starts_with("Destroy the silos"))
        });
        let money = app.world().resource::<Wallet>().money;
        let chaos = app.world().resource::<Chaos>().total;
        for i in 0..5 {
            fire(&mut app, "f2t10.destructable.complete");
            app.update();
            let done = app.world().resource::<MissionsDone>().0.len();
            assert_eq!(done, 0, "done after {} silos", i + 1);
        }
        run_until(&mut app, "the mission's end", |a| {
            !a.world().resource::<MissionsDone>().0.is_empty()
        });
        assert_eq!(app.world().resource::<MissionsDone>().0, ["f2t10"]);
        let b = app.world().resource::<MissionBanner>().clone();
        assert!(b.active && b.completed);
        assert_eq!(b.title, "Faction Mission Completed");
        let w = app.world().resource::<Wallet>();
        assert_eq!(w.money - money, 10_000);
        assert_eq!((w.weapon_parts, w.vehicle_parts), (1, 1));
        assert_eq!(app.world().resource::<Chaos>().total - chaos, 5_000);
        assert!(app.world().resource::<ActiveMission>().0.is_none());
    }

    /// A stronghold's targets all down: Rico holds it, then it is the faction's — its
    /// icon flips, the government's spawn sets are switched off and the faction's on, and
    /// the takeover pays. Needs JC2_TEST_DIR.
    #[test]
    fn taking_a_stronghold_flips_it() {
        let Some(mut app) = mission_app() else { return };
        app.insert_resource(MapData {
            settlements: vec![jc2_formats::icons::SettlementIcon {
                id: "03x26".into(),
                kind: 32,
                position: [-14871.2, 196.3, -2944.4],
                text: "f1s07_title_txt".into(),
                texture: Some("hud_icon_sh_dif.dds".into()),
            }],
            ..default()
        });
        app.update();
        {
            let s = app.world().resource::<Strongholds>();
            assert_eq!(s.0.len(), 9);
            let flags = app.world().resource::<WorldFlags>();
            assert!(flags.fired.contains("f1s07.before.enabled"));
            let p = app.world().resource::<Progress>();
            assert_eq!(p.settlement("f1s07.CountUp"), "03x26");
            assert_eq!(p.percent("03x26"), Some(0));
            let extra = app.world().resource::<ExtraContainers>();
            assert!(
                extra
                    .0
                    .get(&(2, 26))
                    .is_some_and(|v| v.contains(&"f1s07.base.nlz".to_string()))
            );
        }
        // its 29 targets go (no Rico here: he counts as being there)
        for i in 0..29 {
            app.world_mut()
                .resource_mut::<Progress>()
                .count("f1s07.CountUp", format!("target {i}"));
        }
        assert_eq!(
            app.world().resource::<Progress>().percent("03x26"),
            Some(100)
        );
        app.update();
        let phase = |a: &App| {
            a.world()
                .resource::<Strongholds>()
                .0
                .iter()
                .find(|s| s.data.mission == "f1s07")
                .unwrap()
                .phase
        };
        assert_eq!(phase(&app), Phase::Defending);
        assert_eq!(
            app.world().resource::<MissionBanner>().title,
            "Stronghold Takeover Started"
        );
        let money = app.world().resource::<Wallet>().money;
        run_until(&mut app, "the takeover", |a| phase(a) == Phase::Taken);
        app.update();
        let flags = app.world().resource::<WorldFlags>();
        assert!(flags.fired.contains("f1s07.before.disable"));
        assert!(flags.fired.contains("msy.f1s07obj04.completed"));
        assert!(flags.fired.contains("f1s07.complete"));
        let map = app.world().resource::<MapData>();
        assert_eq!(
            map.settlements[0].texture.as_deref(),
            Some("hud_icon_sh_roaches_c_dif.dds")
        );
        assert_eq!(app.world().resource::<Wallet>().money - money, 17_500);
        let b = app.world().resource::<MissionBanner>();
        assert_eq!(b.title, "Stronghold Takeover Completed");
        // the spawn sets of its base: the government's off, the Roaches' on
        let team = |enabled: bool, enable: &str, disable: &str| jc2_formats::character::TeamSpawn {
            name: "sps".into(),
            world: Transform3::IDENTITY,
            aiteam: "bofcx1.aiteam".into(),
            agents: vec!["pelle".into()],
            route: None,
            area: None,
            faction: 0,
            enabled,
            enable_event: enable.into(),
            disable_event: disable.into(),
        };
        let gov = team(false, "f1s07.before.enabled", "f1s07.before.disable");
        let roaches = team(false, "msy.f1s07obj04.completed", "");
        assert_eq!(crate::npc::team_wanted(&gov, &flags.fired), (false, true));
        assert_eq!(
            crate::npc::team_wanted(&roaches, &flags.fired),
            (true, true)
        );
        let before: HashSet<String> = ["f1s07.before.enabled".to_string()].into();
        assert!(crate::npc::team_wanted(&gov, &before).0);
        assert!(!crate::npc::team_wanted(&roaches, &before).0);
    }

    /// A stronghold's base loads with its tile (`ExtraContainers`): its targets fire the
    /// stronghold's counter, its spawn sets wait for the status router's events. Tile
    /// 2,26 has no container of its own. Needs JC2_TEST_DIR.
    #[test]
    fn a_strongholds_base_loads_with_its_tile() {
        let Some(app) =
            crate::world::tests::tile_app_with((2, 26), vec!["f1s07.base.nlz".into()], |_| {})
        else {
            return;
        };
        let st = app.world().resource::<Streamer>();
        let t = st
            .gameplay_sources()
            .find(|(_, _, t)| t.source_container.ends_with("f1s07.base.nlz"))
            .unwrap()
            .2;
        // 3 SAM relays, 6 crates, 3 fuel tanks, 2 cranes
        assert_eq!(t.counters.get("f1s07.CountUp"), Some(&14));
        let units = t
            .units
            .iter()
            .filter(|u| u.counter.as_deref() == Some("f1s07.CountUp"))
            .count();
        assert!(units >= 5, "{units} targets");
        eprintln!(
            "f1s07's base on tile 2,26: {} objects, {units} targets",
            t.objects_total
        );
        assert!(t.objects_total > 100, "{}", t.objects_total);
        assert!(
            t.ai.teams
                .iter()
                .any(|tm| tm.enable_event == "f1s07.before.enabled" && !tm.enabled)
        );
        assert!(
            t.ai.teams
                .iter()
                .any(|tm| tm.faction == 2 && tm.enable_event.ends_with("obj04.completed"))
        );
    }

    /// A colonel killed pays the data's activity (type 8: +1000 chaos, $5000), counts for
    /// his settlement and says so. Needs JC2_TEST_DIR (`51x10.nlz`, `achievements.bin`).
    #[test]
    fn killing_a_colonel_pays() {
        let Some(mut app) = mission_app() else { return };
        let fs = app.world().resource::<Game>().fs.clone();
        let Ok(data) = fs.read_inflated("51x10.nlz") else {
            return;
        };
        let file = first_property_member(&data, &[".blo"]).unwrap();
        let colonel = jc2_formats::missions::colonels(&file).remove(0);
        app.world_mut()
            .resource_mut::<Progress>()
            .totals
            .insert("51x10".into(), 11);
        let e = app
            .world_mut()
            .spawn((ColonelTag(colonel.clone()), colonel_transform(&colonel)))
            .id();
        app.update();
        let (chaos, money) = (
            app.world().resource::<Chaos>().total,
            app.world().resource::<Wallet>().money,
        );
        app.world_mut().entity_mut(e).insert(Dead(0.0));
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Chaos>().total - chaos, 1_000);
        assert_eq!(app.world().resource::<Wallet>().money - money, 5_000);
        assert_eq!(app.world().resource::<Progress>().done("51x10"), 1);
        let n = app.world().resource::<Notices>();
        assert!(
            n.0.iter().any(|x| x.text.ends_with("Assassinated")),
            "{:?}",
            n.0
        );
        let flags = app.world().resource::<WorldFlags>();
        assert!(flags.fired.contains("msy.age40.complete"));
        assert!(
            flags
                .fired
                .contains(&colonel.killed_event.to_ascii_lowercase())
        );
    }
}
