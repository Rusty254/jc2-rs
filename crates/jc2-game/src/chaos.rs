//! Chaos and money. Destroying government property fires an activity
//! (`gsy.activity.typeNN`, read with the destructible units: `jc2_formats::destruction`)
//! whose chaos and money `achievements.bin` gives (`jc2_formats::chaos`): a gas pump +100
//! and $500, a generator +250 and $1250, a radio mast +1000 and $5000. The total picks
//! the game's stage (`aidifficulty.bin`'s `chaos_levels`: a new one every 100 000), which
//! picks the soldiers' classes (`EnemyRoster`) and how high the heat may climb
//! (`heatsystem.bin`'s `heat_level_chaos_N` → [`Heat::cap`]), and unlocks the Black
//! Market's items (`blackmarket.rs`).
//!
//! Each settlement counts its sabotage targets and resource crates (`<id>.count.up` in
//! its location file): its completion is the share of those done ([`Progress`]); the HUD
//! shows it as "N % complete". The CHAOS popup ([`ChaosPopup`]) is what the HUD draws
//! when chaos comes in. See `research/formats/chaos.md`.

use crate::Game;
use crate::destruction::UnitDestroyed;
use crate::explosion::Wreck;
use crate::fleet::Container;
use crate::heat::Heat;
use crate::npc::EnemyRoster;
use crate::world::Streamer;
use bevy::prelude::*;
use jc2_formats::chaos::{Activities, Activity, ChaosLevels};
use jc2_formats::heat::{HeatLevels, HeatSpawn};
use jc2_formats::pickups::ResourceKind;
use jc2_formats::placement::bare_name;
use std::collections::{BTreeMap, HashMap, HashSet};

/// The activity a burnt-out military vehicle counts as ("Military Vehicle Destroyed":
/// +50, $300).
pub const MILITARY_VEHICLE: u32 = 23;
/// How long the CHAOS popup stays (s), and within how long of the last award a new one
/// adds to its numbers instead of replacing them (assumed, from the screenshots: five
/// small fuel containers in a row read "+50").
pub const POPUP_LIFE: f32 = 4.0;
const POPUP_MERGE: f32 = 2.5;

/// The chaos total and the game's stage.
#[derive(Resource, Debug, Default)]
pub struct Chaos {
    pub total: u32,
    /// 0 … 4 (`chaos_00` … `chaos_04`).
    pub stage: usize,
    pub levels: ChaosLevels,
    /// In-session units already paid for (canonical source, local index).
    awarded: HashSet<(String, u32)>,
    /// Sabotage targets destroyed (activities with `sabotage`).
    pub sabotaged: u32,
}

impl Chaos {
    pub fn add(&mut self, n: u32) {
        self.total = self.total.saturating_add(n);
        self.stage = self.levels.stage(self.total);
    }
}

/// Money and the resource parts (the PDA's "Resources").
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct Wallet {
    pub money: u32,
    pub weapon_parts: u32,
    pub vehicle_parts: u32,
    pub armor_parts: u32,
}

impl Wallet {
    /// A resource crate taken: what it held.
    pub fn take(&mut self, kind: ResourceKind) -> u32 {
        let n = kind.amount();
        match kind {
            ResourceKind::Weapon => self.weapon_parts += n,
            ResourceKind::Vehicle => self.vehicle_parts += n,
            ResourceKind::Armor => self.armor_parts += n,
            ResourceKind::Cash => self.money = self.money.saturating_add(n),
        }
        n
    }
}

/// Settlement completion: what counts (`<id>.count.up` in the location files) and what
/// is done.
#[derive(Resource, Debug, Default)]
pub struct Progress {
    pub totals: HashMap<String, u32>,
    done: HashMap<String, HashSet<String>>,
    /// Counters that are another settlement's: a stronghold's targets fire
    /// `f1s07.CountUp`, which counts for settlement `03x26` (`missions.rs` fills this
    /// from `icons_global.bin`).
    pub alias: HashMap<String, String>,
}

impl Progress {
    /// The settlement a counter counts for.
    pub fn settlement<'a>(&'a self, counter: &'a str) -> &'a str {
        self.alias.get(counter).map_or(counter, String::as_str)
    }

    /// Count `what` (a unit, a crate) towards settlement `id`; false if it already was.
    pub fn count(&mut self, id: &str, what: String) -> bool {
        let id = self.settlement(id).to_string();
        self.done.entry(id).or_default().insert(what)
    }

    pub fn done(&self, id: &str) -> u32 {
        self.done
            .get(self.settlement(id))
            .map_or(0, |d| d.len() as u32)
    }

    /// "N % complete": `None` for a place with nothing to count (or not loaded yet).
    pub fn percent(&self, id: &str) -> Option<u32> {
        let total = *self.totals.get(self.settlement(id))?;
        (total > 0).then(|| (self.done(id).min(total) * 100) / total)
    }
}

/// The CHAOS popup: the latest award (summed with the ones just before it).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct ChaosPopup {
    pub chaos: u32,
    pub money: u32,
    /// The text key (`act_type_txt_11`) or English of what was destroyed.
    pub title: String,
    pub age: f32,
    pub active: bool,
    /// Awards shown so far, for the tests.
    pub count: u32,
}

impl ChaosPopup {
    pub fn push(&mut self, a: &Activity) {
        if self.active && self.age < POPUP_MERGE {
            self.chaos += a.chaos;
            self.money += a.money;
        } else {
            self.chaos = a.chaos;
            self.money = a.money;
        }
        self.title = a.title.clone();
        self.age = 0.0;
        self.active = true;
        self.count += 1;
    }

    /// 0..1: slides up and fades in, holds, fades out.
    pub fn alpha(&self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let fade_in = (self.age / 0.25).clamp(0.0, 1.0);
        let fade_out = ((POPUP_LIFE - self.age) / 0.8).clamp(0.0, 1.0);
        fade_in.min(fade_out)
    }
}

/// What the data says chaos is worth.
#[derive(Resource, Debug, Default)]
pub struct ChaosTables {
    pub activities: Activities,
    pub heat: HeatLevels,
    /// Containers of the military's vehicles (`heatspawn.bin`'s `entity_file`s), whose
    /// wrecks count as "Military Vehicle Destroyed".
    pub military: HashSet<String>,
}

/// What the game starts with (`--chaos`, `--money`; debug and screenshots).
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ChaosStart {
    pub chaos: u32,
    pub money: u32,
}

pub struct ChaosPlugin;

impl Plugin for ChaosPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Chaos>()
            .init_resource::<Wallet>()
            .init_resource::<Progress>()
            .init_resource::<ChaosPopup>()
            .init_resource::<ChaosTables>()
            .add_message::<UnitDestroyed>()
            .add_systems(Startup, read_tables)
            .add_systems(
                Update,
                (
                    award_units,
                    award_wrecks,
                    settlement_totals,
                    apply_stage,
                    tick_popup,
                )
                    .chain(),
            );
    }
}

fn read_tables(
    game: Option<Res<Game>>,
    start: Option<Res<ChaosStart>>,
    mut chaos: ResMut<Chaos>,
    mut wallet: ResMut<Wallet>,
    mut tables: ResMut<ChaosTables>,
) {
    if let Some(game) = game {
        let read = |n: &str| game.fs.read_inflated(n).ok();
        if let Some(a) = read("achievements.bin").and_then(|d| Activities::parse(&d).ok()) {
            tables.activities = a;
        }
        if let Some(l) = read("aidifficulty.bin").and_then(|d| ChaosLevels::parse(&d).ok()) {
            chaos.levels = l;
        }
        if let Some(h) = read("heatsystem.bin").and_then(|d| HeatLevels::parse(&d).ok()) {
            tables.heat = h;
        }
        if let Some(h) = read("heatspawn.bin").and_then(|d| HeatSpawn::parse(&d).ok()) {
            tables.military = h
                .unit_types
                .iter()
                .map(|u| bare_name(&u.entity_file).to_ascii_lowercase())
                .filter(|c| !c.is_empty())
                .collect();
        }
    }
    if let Some(s) = start {
        chaos.total = s.chaos;
        wallet.money = s.money;
    }
    chaos.stage = chaos.levels.stage(chaos.total);
    info!(
        "chaos: {} activity types, {} stages; starting at {} chaos (stage {}), ${}",
        tables.activities.0.len(),
        chaos.levels.levels.len(),
        chaos.total,
        chaos.stage,
        wallet.money
    );
}

/// Pay out an activity: chaos, money, the popup.
pub fn award(
    kind: u32,
    tables: &ChaosTables,
    chaos: &mut Chaos,
    wallet: &mut Wallet,
    popup: &mut ChaosPopup,
) -> Option<Activity> {
    let a = tables.activities.get(kind)?.clone();
    chaos.add(a.chaos);
    wallet.money = wallet.money.saturating_add(a.money);
    if a.sabotage {
        chaos.sabotaged += 1;
    }
    if a.chaos > 0 || a.money > 0 {
        popup.push(&a);
    }
    info!(
        "chaos: {} (+{} chaos, ${}) → {} chaos, stage {}, ${}",
        a.title, a.chaos, a.money, chaos.total, chaos.stage, wallet.money
    );
    Some(a)
}

/// Destroyed units pay their activity and count towards their settlement.
fn award_units(
    mut destroyed: MessageReader<UnitDestroyed>,
    tables: Res<ChaosTables>,
    mut chaos: ResMut<Chaos>,
    mut wallet: ResMut<Wallet>,
    mut popup: ResMut<ChaosPopup>,
    mut progress: ResMut<Progress>,
) {
    for d in destroyed.read() {
        if !chaos
            .awarded
            .insert((d.source_container.clone(), d.piece.unit))
        {
            continue;
        }
        if let Some(kind) = d.activity {
            award(kind, &tables, &mut chaos, &mut wallet, &mut popup);
        }
        if let Some(id) = &d.counter {
            progress.count(id, format!("unit {} {}", d.source_container, d.piece.unit));
        }
    }
}

/// A military vehicle burning out counts too.
fn award_wrecks(
    wrecks: Query<&Container, Added<Wreck>>,
    tables: Res<ChaosTables>,
    mut chaos: ResMut<Chaos>,
    mut wallet: ResMut<Wallet>,
    mut popup: ResMut<ChaosPopup>,
) {
    for c in &wrecks {
        let name = c.0.to_ascii_lowercase();
        if tables.military.contains(&name) || name.contains("military") {
            award(
                MILITARY_VEHICLE,
                &tables,
                &mut chaos,
                &mut wallet,
                &mut popup,
            );
        }
    }
}

/// Learn what counts in each settlement as its tile loads.
fn settlement_totals(streamer: Option<Res<Streamer>>, mut progress: ResMut<Progress>) {
    let Some(st) = streamer else { return };
    if !st.is_changed() {
        return;
    }
    let mut found: BTreeMap<&str, u32> = BTreeMap::new();
    for (_, _, t) in st.gameplay_sources() {
        for (id, n) in &t.counters {
            *found.entry(id.as_str()).or_insert(0) += n;
        }
    }
    for (id, n) in found {
        let id = progress.settlement(id).to_string();
        let e = progress.totals.entry(id).or_insert(0);
        *e = (*e).max(n);
    }
}

/// The stage drives the soldiers' classes and the heat's ceiling.
fn apply_stage(
    chaos: Res<Chaos>,
    tables: Res<ChaosTables>,
    roster: Option<ResMut<EnemyRoster>>,
    heat: Option<ResMut<Heat>>,
) {
    let total = chaos.total as f32;
    if let Some(mut r) = roster
        && r.chaos() != total
    {
        if r.0.as_ref().and_then(|e| e.stage(r.chaos()))
            != r.0.as_ref().and_then(|e| e.stage(total))
        {
            info!("chaos {total}: the soldiers of stage {} now", chaos.stage);
        }
        r.set_chaos(total);
    }
    if let Some(mut h) = heat {
        let cap = tables.heat.cap(total);
        if h.cap != cap {
            h.cap = cap;
        }
    }
}

fn tick_popup(time: Res<Time>, mut popup: ResMut<ChaosPopup>) {
    if popup.active {
        popup.age += time.delta_secs();
        if popup.age >= POPUP_LIFE {
            popup.active = false;
        }
    }
}

/// `--face-elite` (debug, for screenshots): once an elite soldier stands in the world,
/// put Rico a few metres in front of him, looking at him.
#[derive(Resource, Default)]
pub struct FaceElite(pub bool);

pub fn face_elite(
    mut commands: Commands,
    mut face: ResMut<FaceElite>,
    time: Res<Time>,
    orbit: Option<ResMut<crate::character::OrbitCamera>>,
    heroes: Query<(Entity, &Transform), With<crate::character::Hero>>,
    elites: Query<
        (&crate::npc::Npc, &crate::npc::Soldier, &Transform),
        Without<crate::character::Hero>,
    >,
) {
    if face.0 || time.elapsed_secs() < 8.0 {
        return;
    }
    let (Some(mut orbit), Ok((hero, rico))) = (orbit, heroes.single()) else {
        return;
    };
    // patrols walk the open ground; guards may stand on a roof
    let key = |x: &(&crate::npc::Npc, &crate::npc::Soldier, &Transform)| {
        let guard = !matches!(x.0.behaviour, crate::npc::Behaviour::Patrol { .. });
        (guard, x.2.translation.distance(rico.translation))
    };
    let Some((npc, _, tf)) = elites.iter().filter(|(_, s, _)| s.elite).min_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
    }) else {
        return;
    };
    face.0 = true;
    let ahead = Vec3::new(-npc.facing.sin(), 0.0, -npc.facing.cos());
    // ahead of him and a little to the side, so Rico does not hide him from the camera
    let side = Vec3::new(-ahead.z, 0.0, ahead.x);
    let at = tf.translation + ahead * 5.0 + side * 2.0 + Vec3::Y * 0.3;
    commands.entity(hero).insert((
        Transform::from_translation(at),
        avian3d::prelude::Position(at),
    ));
    let d = tf.translation - side * 2.5 - at;
    orbit.yaw = (-d.x).atan2(-d.z);
    orbit.pitch = -0.1;
    info!(
        "face-elite: rico at {at:?}, facing the elite at {:?}",
        tf.translation
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::destruction::{DestructionPlugin, StructureHits};
    use crate::explosion::ExplosionPlugin;
    use crate::vehicle::CameraMode;
    use crate::world::Piece;

    #[test]
    fn popup_sums_quick_awards() {
        let pump = Activity {
            kind: 11,
            money: 500,
            chaos: 100,
            title: "act_type_txt_11".into(),
            ..default()
        };
        let mut p = ChaosPopup::default();
        p.push(&pump);
        assert_eq!((p.chaos, p.money, p.alpha()), (100, 500, 0.0));
        p.age = 1.0;
        p.push(&pump);
        assert_eq!((p.chaos, p.money), (200, 1000));
        p.age = 3.0;
        p.push(&pump);
        assert_eq!((p.chaos, p.money), (100, 500));
        let mut pr = Progress::default();
        pr.totals.insert("31x14".into(), 15);
        assert_eq!(pr.percent("31x14"), Some(0));
        assert!(pr.count("31x14", "a".into()));
        assert!(!pr.count("31x14", "a".into()));
        assert_eq!(pr.percent("31x14"), Some(6));
        assert_eq!(pr.percent("nowhere"), None);
    }

    /// Headless, on tile 51,10 (a village with two gas pumps): shooting a gas pump down
    /// pays the data's chaos and money (+100, $500), shows the CHAOS popup with its name
    /// and counts it once. Needs the tile's test data and `achievements.bin`.
    #[test]
    fn destroying_a_gas_pump_pays_chaos() {
        let Some(mut app) = crate::world::tests::tile_app_at((51, 10), |app| {
            app.init_resource::<CameraMode>().add_plugins((
                ExplosionPlugin,
                DestructionPlugin,
                ChaosPlugin,
            ));
        }) else {
            return;
        };
        let st = app.world().resource::<Streamer>();
        let unit = st.loaded[&(51, 10)]
            .units
            .iter()
            .position(|u| u.activity == Some(11))
            .expect("a gas pump") as u32;
        let piece = Piece {
            source: st.loaded[&(51, 10)].source_key,
            tile: (51, 10),
            unit,
        };
        assert_eq!(app.world().resource::<Chaos>().total, 0);
        app.world_mut()
            .resource_mut::<StructureHits>()
            .queue
            .push((piece, 1000.0));
        for _ in 0..3 {
            app.update();
        }
        let c = app.world().resource::<Chaos>();
        assert_eq!((c.total, c.stage, c.sabotaged), (100, 0, 1));
        assert_eq!(app.world().resource::<Wallet>().money, 500);
        let p = app.world().resource::<ChaosPopup>();
        assert!(p.active);
        assert_eq!((p.chaos, p.money), (100, 500));
        assert_eq!(p.title, "act_type_txt_11");
        // destroyed again (the unit is gone): nothing more
        app.world_mut()
            .resource_mut::<StructureHits>()
            .queue
            .push((piece, 1000.0));
        app.update();
        assert_eq!(app.world().resource::<Chaos>().total, 100);
        // the village counts its targets and crates
        let pr = app.world().resource::<Progress>();
        assert_eq!(pr.totals.get("51x10"), Some(&11));
        let before = pr.done("51x10");
        let event = UnitDestroyed {
            source_container: "locations/other/gas-pump-source.nlz".into(),
            piece,
            name: "other source pump".into(),
            at: Vec3::ZERO,
            activity: Some(11),
            counter: Some("51x10".into()),
            fires: Vec::new(),
            mission_owner: None,
        };
        let mut messages = app.world_mut().resource_mut::<Messages<UnitDestroyed>>();
        messages.write(event.clone());
        messages.write(event);
        app.update();
        assert_eq!(
            app.world().resource::<Chaos>().total,
            200,
            "same local unit in different archive pays once independently"
        );
        assert_eq!(app.world().resource::<Wallet>().money, 1000);
        assert_eq!(
            app.world().resource::<Progress>().done("51x10"),
            before + 1,
            "new source counts once, duplicate events do not count twice"
        );
    }

    /// The chaos total crosses into the next stage (100 000): the roster rolls that
    /// stage's soldiers, elites at its `elite_probability_normal`, and the heat may climb
    /// to level 3. Needs `enemytypespawnsettings.bin`, `aidifficulty.bin`,
    /// `heatsystem.bin`, `achievements.bin`.
    #[test]
    fn crossing_a_stage_brings_elites() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = std::sync::Arc::new(jc2_formats::GameFs::open(dir).unwrap());
        for f in [
            "enemytypespawnsettings.bin",
            "aidifficulty.bin",
            "heatsystem.bin",
            "achievements.bin",
        ] {
            if !fs.contains(f) {
                return;
            }
        }
        let types = jc2_formats::enemies::EnemyTypes::parse(
            &fs.read_inflated("enemytypespawnsettings.bin").unwrap(),
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Game { fs })
            .insert_resource(EnemyRoster(Some(types.clone()), 0.0))
            .init_resource::<Heat>()
            .add_plugins(ChaosPlugin);
        app.update();
        let share = |app: &App| {
            let r = app.world().resource::<EnemyRoster>();
            let n = 4000u32;
            let elites = (0..n)
                .filter(|i| {
                    r.roll(
                        crate::npc::unit_rand(*i),
                        crate::npc::unit_rand(i ^ 0x5bd1_e995),
                    )
                    .0
                    .elite
                })
                .count();
            elites as f32 / n as f32
        };
        let s0 = types
            .stage(0.0)
            .unwrap()
            .elite_chance(crate::npc::DIFFICULTY);
        let s1 = types
            .stage(150_000.0)
            .unwrap()
            .elite_chance(crate::npc::DIFFICULTY);
        assert!(s1 > s0, "{s0} {s1}");
        assert!((share(&app) - s0).abs() < 0.03, "{} vs {s0}", share(&app));
        assert_eq!(app.world().resource::<Heat>().cap, 40.0);
        // a radio mast's worth at a time until the stage turns over
        let mut n = 0;
        while app.world().resource::<Chaos>().total < 100_000 {
            let w = app.world_mut();
            w.resource_scope(|w, tables: Mut<ChaosTables>| {
                w.resource_scope(|w, mut chaos: Mut<Chaos>| {
                    w.resource_scope(|w, mut wallet: Mut<Wallet>| {
                        let mut popup = w.resource_mut::<ChaosPopup>();
                        award(20, &tables, &mut chaos, &mut wallet, &mut popup);
                    })
                })
            });
            n += 1;
        }
        assert_eq!(n, 100, "a hundred radio masts");
        app.update();
        assert_eq!(app.world().resource::<Chaos>().stage, 1);
        assert_eq!(app.world().resource::<EnemyRoster>().chaos(), 100_000.0);
        assert!((share(&app) - s1).abs() < 0.03, "{} vs {s1}", share(&app));
        assert_eq!(app.world().resource::<Heat>().cap, 60.0);
        assert_eq!(app.world().resource::<Wallet>().money, 500_000);
    }
}
