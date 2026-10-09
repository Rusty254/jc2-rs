//! People in the world: the soldiers the locations' team spawn sets place
//! (`jc2_formats::character::AiPlacements`). Guards stand at their post and now and then
//! walk to one of their area's guard points and back; patrols walk their route. Each is a
//! character doll (one variant of every part group), skinned to its skeleton and animated
//! with the same clips as Rico. They appear with their tile and leave with it.
//!
//! Who each soldier is comes from `enemytypespawnsettings.bin`
//! (`jc2_formats::enemies`): at the game's stage (the stage of the chaos total,
//! `chaos.rs` keeps [`EnemyRoster::chaos`] up to date) a share of them are elites
//! (`elite_probability_normal`: 10 % at the start, 45 % at the last stage), the rest grunts; the class gives the
//! health (100 / 250) and the weapon, rolled by its weights (grunts mostly the Beretta,
//! elites mostly the assault rifle), and the faction the doll (`grunt_cdoll` /
//! `elite_cdoll`). That the village teams (`bofcx1.aiteam`) are the government's is an
//! assumption: the team data names an AI agent, not a faction.

use crate::Game;
use crate::character::{HEIGHT, RADIUS};
use crate::combat::Dead;
use crate::coords::{mat_to_bevy, to_bevy, yaw_of, yaw_toward};
use crate::poses::{Arms, Gait, drive_upper_body};
use crate::rbm_loader::RbmAsset;
use crate::rig::{Animator, Clip, Rig, RigData, skin, spawn_joints};
use crate::world::Streamer;
use crate::world::{GameplaySourceKey, SourceOrigin};
use avian3d::prelude::*;
use bevy::asset::LoadState;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use jc2_formats::character::{AiPlacements, CharacterDoll, TeamSpawn};
use jc2_formats::enemies::{Difficulty, EnemyTypes, GOVERNMENT};
use jc2_formats::lod::LodDescriptor;
use jc2_formats::missions::{Colonel, events};
use jc2_formats::placement::bare_name;
use jc2_formats::sarc::SmallArchive;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const WALK_SPEED: f32 = 1.4;
/// Reinforcements hurry (m/s; the walk clip at its fastest rate and a bit).
const RUN_SPEED: f32 = 3.0;
const ARRIVE: f32 = 0.4;
/// Guards pause this long (s) between strolls, give or take.
const GUARD_PAUSE: f32 = 9.0;
/// The government grunt's doll, when `enemytypespawnsettings.bin` can't be read.
const SOLDIER: &str = "pd_gov_base01.ee";
/// The difficulty for `elite_probability_*`.
pub const DIFFICULTY: Difficulty = Difficulty::Normal;

/// Who a soldier is: elite or grunt, the weapon he carries (a `weapons.bin` name) and
/// his health.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Soldier {
    pub elite: bool,
    pub weapon: String,
    pub health: f32,
}

/// `enemytypespawnsettings.bin`, read once, and the chaos total that picks its stage.
#[derive(Resource, Default)]
pub struct EnemyRoster(pub Option<EnemyTypes>, pub f32);

impl EnemyRoster {
    /// The chaos points the soldiers are rolled at (`chaos.rs` sets it).
    pub fn chaos(&self) -> f32 {
        self.1
    }

    pub fn set_chaos(&mut self, chaos: f32) {
        self.1 = chaos;
    }

    /// Roll a soldier of the government (the class with `r1`, the weapon with `r2`, both
    /// in 0..1) and the doll container he wears.
    pub fn roll(&self, r1: f32, r2: f32) -> (Soldier, String) {
        let Some(stage) = self.0.as_ref().and_then(|e| e.stage(self.1)) else {
            return (
                Soldier {
                    elite: false,
                    weapon: "Assault".into(),
                    health: 100.0,
                },
                SOLDIER.into(),
            );
        };
        let elite = r1 < stage.elite_chance(DIFFICULTY);
        let class = if elite { &stage.elite } else { &stage.grunt };
        let doll = self
            .0
            .as_ref()
            .and_then(|e| e.faction(GOVERNMENT))
            .map(|f| {
                if elite {
                    f.elite_container()
                } else {
                    f.grunt_container()
                }
            })
            .unwrap_or_else(|| SOLDIER.into());
        (
            Soldier {
                elite,
                weapon: class.pick_weapon(r2).unwrap_or("Assault").to_string(),
                health: class.health,
            },
            doll,
        )
    }
}

impl EnemyRoster {
    /// A soldier of `faction` (0 the government; 1 Reapers, 2 Roaches, 3 Ular Boys wear
    /// their faction's dolls from `enemytypespawnsettings.bin`), an elite when `elite`.
    pub fn roll_for(&self, faction: i32, elite: bool, r1: f32, r2: f32) -> (Soldier, String) {
        let (soldier, doll) = self.roll(if elite { 0.0 } else { r1 }, r2);
        let key = jc2_formats::missions::Faction::from_icon_faction(faction)
            .and_then(|f| f.enemies_key());
        let doll = key
            .and_then(|k| {
                self.0
                    .as_ref()?
                    .faction(jc2_formats::hash::hash_str(k))
                    .map(|f| {
                        if soldier.elite {
                            f.elite_container()
                        } else {
                            f.grunt_container()
                        }
                    })
            })
            .unwrap_or(doll);
        (soldier, doll)
    }
}

/// A soldier on Rico's side (a faction's, at a stronghold it has taken): he does not
/// shoot at Rico (`combat.rs`).
#[derive(Component, Debug)]
pub struct Friendly;

/// A colonel: who he is and what his death fires (`missions.rs` pays for it).
#[derive(Component, Clone, Debug)]
pub struct ColonelTag(pub Colonel);

/// The team spawn set a soldier stands for (tile, index into its teams).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TeamOf(pub GameplaySourceKey, pub usize);

/// Whether a team spawn set places its soldiers now, given the events fired so far
/// (`missions::WorldFlags`): `(wanted, take away the ones standing)`. A set that is on
/// stays on until one of its `disable_event`s fires; a set that is off waits for one of
/// its `enable_event`s. Local events (`t{mod.age({…})}::spawn`, a trigger inside the
/// location) are the location's own logic, not run here: such a set is taken as on, as
/// before. Only a global event (`f1s07.before.disable`) takes standing soldiers away.
pub fn team_wanted(team: &TeamSpawn, fired: &HashSet<String>) -> (bool, bool) {
    let off: Vec<String> = events(&team.disable_event)
        .filter(|e| fired.contains(e))
        .collect();
    if !off.is_empty() {
        return (false, off.iter().any(|e| !e.contains('{')));
    }
    if team.enabled || team.enable_event.trim().is_empty() || team.enable_event.contains('{') {
        return (true, false);
    }
    (events(&team.enable_event).any(|e| fired.contains(&e)), true)
}

/// A repeatable 0..1 from a seed (a hash, so neighbouring seeds differ).
pub fn unit_rand(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

#[derive(Clone, Debug)]
pub enum Behaviour {
    /// Stand at `home`; stroll to one of `points` and back.
    Guard { home: Vec3, points: Vec<Vec3> },
    /// Walk `points` in a loop.
    Patrol { points: Vec<Vec3>, next: usize },
}

#[derive(Component, Debug)]
pub struct Npc {
    pub behaviour: Behaviour,
    pub facing: f32,
    pub target: Option<Vec3>,
    pub wait: f32,
    pub speed: f32,
    /// Has seen Rico: stands, faces him and shoots instead of walking its rounds.
    pub alert: bool,
    /// How fast he walks (m/s): a stroll on his rounds, a run when sent somewhere.
    pub pace: f32,
    seed: u32,
}

impl Npc {
    /// A soldier standing guard at `home` (for tests elsewhere in the crate).
    #[cfg(test)]
    pub fn guard(home: Vec3, facing: f32) -> Self {
        Self {
            behaviour: Behaviour::Guard {
                home,
                points: Vec::new(),
            },
            facing,
            target: None,
            wait: 1e9,
            speed: 0.0,
            alert: false,
            pace: WALK_SPEED,
            seed: 1,
        }
    }

    /// Make `p` his post and head there now (reinforcements converging on Rico).
    pub fn send_to(&mut self, p: Vec3) {
        if let Behaviour::Guard { home, .. } = &mut self.behaviour {
            *home = p;
        }
        self.target = Some(p);
    }
}

#[derive(Clone)]
struct Request {
    origin: Option<SourceOrigin>,
    container: String,
    at: Vec3,
    facing: f32,
    behaviour: Behaviour,
    tile: (u32, u32),
    seed: usize,
    soldier: Soldier,
    /// Head here as soon as he stands (reinforcements; `heat.rs`).
    target: Option<Vec3>,
    /// The team spawn set he belongs to.
    team: Option<(GameplaySourceKey, usize)>,
    /// He is this colonel.
    colonel: Option<Colonel>,
    /// A faction's soldier: does not fight Rico.
    friendly: bool,
}

struct Pending {
    req: Request,
    skeleton: String,
    models: Vec<Handle<RbmAsset>>,
}

#[derive(Resource, Default)]
pub struct Npcs {
    requests: Vec<Request>,
    pending: Vec<Pending>,
    pub spawned: Vec<(Entity, (u32, u32))>,
    tiles_done: HashSet<GameplaySourceKey>,
    rigs: HashMap<String, Option<Arc<RigData>>>,
    dolls: HashMap<String, Option<CharacterDoll>>,
    /// Team spawn sets whose soldiers are up (or asked for).
    teams_up: HashSet<(GameplaySourceKey, usize)>,
    /// Teams to take away (their set was switched off).
    teams_down: Vec<(GameplaySourceKey, usize)>,
}

pub struct NpcPlugin;

impl Plugin for NpcPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Npcs>()
            .init_resource::<EnemyRoster>()
            .add_systems(Startup, read_roster)
            .add_systems(
                Update,
                (
                    tile_teams,
                    request_models,
                    spawn_npcs,
                    drop_with_tiles,
                    drop_switched_off,
                )
                    .chain()
                    .after(crate::locations::LocationReconciled)
                    .in_set(crate::world::GameplaySourcesReconciled),
            )
            .add_systems(Update, (npc_behaviour, npc_animation).chain())
            .add_plugins(crate::heat::HeatPlugin);
        if !app.is_plugin_added::<crate::rig::RigPlugin>() {
            app.add_plugins(crate::rig::RigPlugin);
        }
    }
}

fn read_roster(mut roster: ResMut<EnemyRoster>, game: Res<Game>) {
    roster.0 = game
        .fs
        .read_inflated("enemytypespawnsettings.bin")
        .ok()
        .and_then(|d| EnemyTypes::parse(&d).ok());
    match roster.0.as_ref().and_then(|e| e.stage(roster.1)) {
        Some(s) => info!(
            "soldiers: {:.0} % elites ({} hp, {}), grunts {} hp ({})",
            s.elite_chance(DIFFICULTY) * 100.0,
            s.elite.health,
            s.elite
                .weapons
                .iter()
                .map(|w| format!("{} {}", w.0, w.1))
                .collect::<Vec<_>>()
                .join(", "),
            s.grunt.health,
            s.grunt
                .weapons
                .iter()
                .map(|w| format!("{} {}", w.0, w.1))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        None => warn!("enemytypespawnsettings.bin not read: every soldier a rifleman"),
    }
}

impl Npcs {
    /// A soldier rolled from the roster, walking from `at` to `target` (reinforcements):
    /// he stands with `tile` and leaves with it.
    pub fn reinforce(
        &mut self,
        roster: &EnemyRoster,
        at: Vec3,
        target: Vec3,
        tile: (u32, u32),
        seed: u32,
    ) {
        let (soldier, container) = roster.roll(unit_rand(seed), unit_rand(seed ^ 0x5bd1_e995));
        self.requests.push(Request {
            origin: None,
            container,
            at,
            facing: 0.0,
            // where he is sent becomes his post
            behaviour: Behaviour::Guard {
                home: target,
                points: Vec::new(),
            },
            tile,
            seed: seed as usize,
            soldier,
            target: Some(target),
            team: None,
            colonel: None,
            friendly: false,
        });
    }

    /// Reinforcements asked for and not standing yet.
    pub fn reinforcements_waiting(&self) -> usize {
        self.requests.iter().filter(|r| r.target.is_some()).count()
            + self
                .pending
                .iter()
                .filter(|p| p.req.target.is_some())
                .count()
    }
}

/// Queue the soldiers of tiles that just loaded, and of team spawn sets that an event
/// switched on (`team_wanted`); colonels come with their own health and doll
/// (`jc2_formats::missions::colonels`), a faction's soldiers in its dolls.
fn tile_teams(
    mut npcs: ResMut<Npcs>,
    streamer: Option<Res<Streamer>>,
    roster: Res<EnemyRoster>,
    game: Option<Res<Game>>,
    flags: Option<Res<crate::missions::WorldFlags>>,
) {
    let Some(st) = streamer else { return };
    let empty = HashSet::new();
    let fired = flags.as_ref().map_or(&empty, |f| &f.fired);
    let changed = flags.as_ref().is_some_and(|f| f.is_changed());
    for (source, key, tile) in st.gameplay_sources() {
        if !changed && npcs.tiles_done.contains(&source) {
            continue;
        }
        npcs.tiles_done.insert(source);
        let ai: &AiPlacements = &tile.ai;
        for (ti, team) in ai.teams.iter().enumerate() {
            let (wanted, take_away) = team_wanted(team, fired);
            let up = npcs.teams_up.contains(&(source, ti));
            if !wanted {
                if up && take_away {
                    npcs.teams_up.remove(&(source, ti));
                    npcs.teams_down.push((source, ti));
                }
                continue;
            }
            if up {
                continue;
            }
            npcs.teams_up.insert((source, ti));
            let tf = mat_to_bevy(&team.world);
            let colonel = team
                .agents
                .iter()
                .any(|a| a.eq_ignore_ascii_case("general"))
                .then(|| {
                    let p = team.world.translation();
                    tile.colonels
                        .iter()
                        .find(|c| (c.position[0] - p[0]).abs() + (c.position[2] - p[2]).abs() < 1.0)
                })
                .flatten();
            let friendly = team.faction != 0;
            for (ai_i, role) in team.agents.iter().enumerate() {
                // every village team we have seen is `bofc*`, taken as the government's
                let seed = key.0 as usize * 31 + key.1 as usize * 7 + ti * 3 + ai_i;
                let general = colonel.is_some() && role.eq_ignore_ascii_case("general");
                let (mut soldier, mut container) = roster.roll_for(
                    team.faction,
                    colonel.is_some(),
                    unit_rand(seed as u32),
                    unit_rand(seed as u32 ^ 0x5bd1_e995),
                );
                if let Some(c) = colonel {
                    // the data's health; his escort keeps the elite class's if that is more
                    let agent = if general {
                        Some(&c.colonel)
                    } else {
                        c.guards.get(ai_i.saturating_sub(1))
                    };
                    if let Some(a) = agent {
                        soldier.health = if general {
                            a.health
                        } else {
                            a.health.max(soldier.health)
                        };
                        if general
                            && let Some(doll) = a.doll_container()
                            && game.as_ref().is_some_and(|g| g.fs.contains(&doll))
                        {
                            container = doll;
                        }
                    }
                }
                let behaviour = match (&team.route, &team.area) {
                    (Some(r), _) => match ai.routes.iter().find(|x| &x.name == r) {
                        Some(route) if !route.points.is_empty() => Behaviour::Patrol {
                            points: route.points.iter().map(|p| to_bevy(*p)).collect(),
                            next: 0,
                        },
                        _ => Behaviour::Guard {
                            home: tf.translation,
                            points: Vec::new(),
                        },
                    },
                    (None, Some(a)) => Behaviour::Guard {
                        home: tf.translation,
                        points: ai
                            .areas
                            .iter()
                            .find(|x| &x.name == a)
                            .map(|x| x.points.iter().map(|p| to_bevy(*p)).collect())
                            .unwrap_or_default(),
                    },
                    _ => Behaviour::Guard {
                        home: tf.translation,
                        points: Vec::new(),
                    },
                };
                // a second agent of a team stands a little aside
                let at = tf.translation + tf.rotation * Vec3::X * (ai_i as f32 * 1.2);
                debug!(
                    "{}: a {role} at {at:?}, {} with the {}",
                    team.name,
                    if soldier.elite { "elite" } else { "grunt" },
                    soldier.weapon
                );
                npcs.requests.push(Request {
                    origin: Some(SourceOrigin {
                        key: source,
                        container: tile.source_container.clone(),
                        object: team.name.clone(),
                        ordinal: ti,
                    }),
                    container,
                    at,
                    facing: yaw_of(tf.rotation),
                    behaviour,
                    tile: key,
                    seed,
                    soldier,
                    target: None,
                    team: Some((source, ti)),
                    colonel: general.then(|| colonel.cloned()).flatten(),
                    friendly,
                });
            }
        }
    }
    npcs.tiles_done.retain(|k| st.source_alive(*k));
    npcs.teams_up.retain(|(k, _)| st.source_alive(*k));
}

/// Soldiers of team spawn sets an event switched off go.
fn drop_switched_off(
    mut commands: Commands,
    mut npcs: ResMut<Npcs>,
    teams: Query<(Entity, &TeamOf)>,
) {
    if npcs.teams_down.is_empty() {
        return;
    }
    let down = std::mem::take(&mut npcs.teams_down);
    for (e, t) in &teams {
        if down.contains(&(t.0, t.1)) {
            commands.entity(e).try_despawn();
            npcs.spawned.retain(|(x, _)| *x != e);
        }
    }
    npcs.pending
        .retain(|p| !p.req.team.is_some_and(|t| down.contains(&t)));
    npcs.requests
        .retain(|r| !r.team.is_some_and(|t| down.contains(&t)));
}

/// Character dolls by container, read once (`None`: it has none).
pub(crate) type DollCache = HashMap<String, Option<CharacterDoll>>;

/// Read a doll container's `.cdoll` and ask for one outfit of it (picked by `seed`) at
/// LOD 1: the skeleton file the doll names (`biped.bsk`, `civilian_biped.bsk` — the
/// civilians' dolls say `.skeleton`) and the model handles. `None` without a doll or a
/// model. Shared by the soldiers and the civilians.
pub(crate) fn request_outfit(
    game: &Game,
    assets: &AssetServer,
    dolls: &mut DollCache,
    container: &str,
    seed: usize,
) -> Option<(String, Vec<Handle<RbmAsset>>)> {
    let Ok(data) = game.fs.read_inflated(container) else {
        warn!("{container}: not found");
        return None;
    };
    let sarc = SmallArchive::parse(&data).ok()?;
    let doll = dolls
        .entry(container.to_string())
        .or_insert_with(|| {
            sarc.entries
                .iter()
                .find(|e| e.name.to_ascii_lowercase().ends_with(".cdoll"))
                .and_then(|e| sarc.data(&data, e).ok())
                .and_then(|d| CharacterDoll::parse(d).ok())
        })
        .clone();
    let Some(doll) = doll else {
        warn!("{container}: no character doll inside");
        return None;
    };
    let mut models = Vec::new();
    for lod in doll.outfit(seed) {
        let lod_name = bare_name(&lod).to_ascii_lowercase();
        let Some(text) = sarc
            .get(&lod_name)
            .and_then(|e| sarc.data(&data, e).ok())
            .map(|b| b.to_vec())
            .or_else(|| game.fs.read_inflated(&lod_name).ok())
        else {
            continue;
        };
        let d = LodDescriptor::parse(&String::from_utf8_lossy(&text));
        if let Some(rbm) = d.model_for_level(1) {
            models.push(assets.load(format!("jc2://{container}/{rbm}")));
        }
    }
    if models.is_empty() {
        return None;
    }
    let mut skeleton = bare_name(&doll.skeleton).to_ascii_lowercase();
    if let Some(stem) = skeleton.strip_suffix(".skeleton") {
        skeleton = format!("{stem}.bsk");
    }
    Some((skeleton, models))
}

/// Read each request's doll and ask for its outfit.
fn request_models(mut npcs: ResMut<Npcs>, assets: Res<AssetServer>, game: Res<Game>) {
    let requests = std::mem::take(&mut npcs.requests);
    for req in requests {
        let Some((skeleton, models)) =
            request_outfit(&game, &assets, &mut npcs.dolls, &req.container, req.seed)
        else {
            continue;
        };
        npcs.pending.push(Pending {
            req,
            skeleton,
            models,
        });
    }
}

/// Whether every model of an outfit has loaded (or failed to).
pub(crate) fn outfit_ready(
    assets: &AssetServer,
    rbms: &Assets<RbmAsset>,
    models: &[Handle<RbmAsset>],
) -> bool {
    models.iter().all(|h| {
        rbms.get(h).is_some() || matches!(assets.get_load_state(h.id()), Some(LoadState::Failed(_)))
    })
}

/// Put an outfit on `body`: a model entity (the feet `HEIGHT / 2` below the body's
/// centre, turned to `facing`, tagged `NpcModel`) with the meshes skinned to `rig`'s
/// joints. Returns the animator for the body when there is a rig.
pub(crate) fn dress(
    commands: &mut Commands,
    body: Entity,
    facing: f32,
    rig: Option<Arc<RigData>>,
    models: &[Handle<RbmAsset>],
    rbms: &Assets<RbmAsset>,
    bindposes: &mut Assets<SkinnedMeshInverseBindposes>,
) -> Option<Animator> {
    let model = commands
        .spawn((
            Transform::from_xyz(0.0, -HEIGHT / 2.0, 0.0)
                .with_rotation(Quat::from_rotation_y(facing)),
            Visibility::default(),
            NpcModel,
        ))
        .id();
    commands.entity(body).add_child(model);
    let rigged = rig.map(|r| {
        let (joints, handle) = spawn_joints(commands, model, &r.skeleton, bindposes);
        (r, joints, handle)
    });
    for h in models {
        let Some(rbm) = rbms.get(h) else { continue };
        for b in &rbm.blocks {
            let mut mesh = commands.spawn((
                Mesh3d(b.mesh.clone()),
                b.material_component(),
                Transform::IDENTITY,
            ));
            if let Some((_, joints, handle)) = &rigged
                && b.skinned
            {
                mesh.insert(skin(joints, handle));
            }
            let id = mesh.id();
            commands.entity(model).add_child(id);
        }
    }
    rigged.map(|(r, joints, _)| Animator::new(r, joints))
}

/// Put a walking body's feet back on the ground under it (a ray from a metre above).
pub(crate) fn follow_ground(spatial: &SpatialQuery, body: Entity, tf: &mut Transform) {
    if let Some(hit) = spatial.cast_ray(
        tf.translation + Vec3::Y * 1.0,
        Dir3::NEG_Y,
        6.0,
        true,
        &SpatialQueryFilter::from_excluded_entities([body]),
    ) {
        let ground = tf.translation.y + 1.0 - hit.distance;
        tf.translation.y = ground + HEIGHT / 2.0;
    }
}

/// Turn a facing (yaw about +y, 0 = facing −z, `coords::yaw_toward`) smoothly towards the
/// flat direction `dir`.
pub(crate) fn turn_toward(facing: &mut f32, dir: Vec3, dt: f32) {
    let want = yaw_toward(dir);

    let mut diff = want - *facing;
    diff = (diff + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    *facing += diff * (1.0 - (-6.0 * dt).exp());
}

#[allow(clippy::too_many_arguments)]
fn spawn_npcs(
    mut commands: Commands,
    mut npcs: ResMut<Npcs>,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    game: Res<Game>,
    hero_rig: Option<Res<Rig>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    spatial: SpatialQuery,
    streamer: Option<Res<Streamer>>,
) {
    if npcs.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut npcs.pending);
    let mut keep = Vec::new();
    for p in pending {
        if p.req
            .origin
            .as_ref()
            .is_some_and(|o| streamer.as_ref().is_none_or(|st| !st.source_alive(o.key)))
        {
            continue;
        }
        if !outfit_ready(&assets, &rbms, &p.models) {
            keep.push(p);
            continue;
        }
        // stand on the ground (wait for its collider)
        let Some(hit) = spatial.cast_ray(
            p.req.at + Vec3::Y * 50.0,
            Dir3::NEG_Y,
            200.0,
            true,
            &SpatialQueryFilter::default(),
        ) else {
            keep.push(p);
            continue;
        };
        let ground = p.req.at.y + 50.0 - hit.distance;
        // the skeleton: Rico's when it is his, else loaded once per file
        let rig = match &hero_rig {
            Some(r) if r.file == p.skeleton => Some(r.0.clone()),
            _ => npcs
                .rigs
                .entry(p.skeleton.clone())
                .or_insert_with(|| Rig::load(&game.fs, &p.skeleton).map(|r| r.0))
                .clone(),
        };
        let pos = Vec3::new(p.req.at.x, ground + HEIGHT / 2.0, p.req.at.z);
        let npc = commands
            .spawn((
                RigidBody::Kinematic,
                Collider::capsule(RADIUS, HEIGHT - 2.0 * RADIUS),
                Transform::from_translation(pos),
                Visibility::default(),
                Npc {
                    behaviour: p.req.behaviour.clone(),
                    facing: p.req.facing,
                    target: p.req.target,
                    wait: (p.req.seed % 7) as f32,
                    speed: 0.0,
                    alert: false,
                    pace: if p.req.target.is_some() {
                        RUN_SPEED
                    } else {
                        WALK_SPEED
                    },
                    seed: p.req.seed as u32,
                },
                p.req.soldier.clone(),
                Name::new(format!("npc {}", p.req.container)),
            ))
            .id();
        if p.req.target.is_some() {
            commands.entity(npc).insert(crate::heat::Reinforcement);
        }
        if let Some(t) = p.req.team {
            commands.entity(npc).insert(TeamOf(t.0, t.1));
        }
        if let Some(c) = &p.req.colonel {
            commands.entity(npc).insert((
                ColonelTag(c.clone()),
                Name::new(format!("colonel {}", c.id)),
            ));
        }
        if p.req.friendly {
            commands.entity(npc).insert(Friendly);
        }
        if let Some(an) = dress(
            &mut commands,
            npc,
            p.req.facing,
            rig,
            &p.models,
            &rbms,
            &mut bindposes,
        ) {
            commands.entity(npc).insert(an);
        }
        if let Some(origin) = p.req.origin {
            commands.entity(npc).insert(origin);
        }
        npcs.spawned.push((npc, p.req.tile));
    }
    npcs.pending.extend(keep);
}

/// The model under a person's body (soldier or civilian): turned to face where they go.
#[derive(Component)]
pub(crate) struct NpcModel;

fn drop_with_tiles(
    mut commands: Commands,
    mut npcs: ResMut<Npcs>,
    streamer: Option<Res<Streamer>>,
    origins: Query<&SourceOrigin>,
) {
    let Some(st) = streamer else { return };
    let mut keep = Vec::with_capacity(npcs.spawned.len());
    for (e, tile) in std::mem::take(&mut npcs.spawned) {
        if origins.get(e).map_or_else(
            |_| st.loaded.contains_key(&tile),
            |o| st.source_alive(o.key),
        ) {
            keep.push((e, tile));
        } else {
            commands.entity(e).try_despawn();
        }
    }
    npcs.spawned = keep;
    npcs.pending.retain(|p| {
        p.req.origin.as_ref().map_or_else(
            || st.loaded.contains_key(&p.req.tile),
            |o| st.source_alive(o.key),
        )
    });
    npcs.requests
        .retain(|r| r.origin.as_ref().is_none_or(|o| st.source_alive(o.key)));
}

/// Walk to the current target over the ground; pick the next one when there.
#[allow(clippy::type_complexity)]
fn npc_behaviour(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut q: Query<
        (Entity, &mut Npc, &mut Transform, &Children),
        (Without<Dead>, Without<crate::grapple::Hooked>),
    >,
    mut models: Query<&mut Transform, (With<NpcModel>, Without<Npc>)>,
) {
    let dt = time.delta_secs();
    for (e, mut npc, mut tf, children) in &mut q {
        // choose a target
        if npc.alert {
            npc.speed = 0.0;
            for c in children.iter() {
                if let Ok(mut m) = models.get_mut(c) {
                    m.rotation = Quat::from_rotation_y(npc.facing);
                }
            }
            continue;
        }
        if npc.target.is_none() {
            npc.wait -= dt;
            if npc.wait <= 0.0 {
                let seed = npc.seed;
                let pos = tf.translation;
                npc.target = match &mut npc.behaviour {
                    Behaviour::Patrol { points, next } => {
                        let t = points[*next % points.len()];
                        *next = (*next + 1) % points.len();
                        Some(t)
                    }
                    Behaviour::Guard { home, points } => {
                        let at_home = Vec2::new(pos.x - home.x, pos.z - home.z).length() < 1.0;
                        if at_home && !points.is_empty() {
                            Some(
                                points[(seed as usize + (time.elapsed_secs() as usize))
                                    % points.len()],
                            )
                        } else if !at_home {
                            Some(*home)
                        } else {
                            None
                        }
                    }
                };
                npc.wait = match npc.behaviour {
                    Behaviour::Patrol { .. } => 0.0,
                    Behaviour::Guard { .. } => GUARD_PAUSE + (seed % 5) as f32,
                };
            }
        }
        // walk
        let mut speed = 0.0;
        if let Some(t) = npc.target {
            let to = Vec3::new(t.x - tf.translation.x, 0.0, t.z - tf.translation.z);
            let d = to.length();
            if d < ARRIVE {
                npc.target = None;
                if matches!(npc.behaviour, Behaviour::Patrol { .. }) {
                    npc.wait = 1.0;
                }
            } else {
                let dir = to / d;
                let step = (npc.pace * dt).min(d);
                tf.translation += dir * step;
                speed = npc.pace;
                turn_toward(&mut npc.facing, dir, dt);
            }
        }
        npc.speed = speed;
        // keep the feet on the ground
        if speed > 0.0 {
            follow_ground(&spatial, e, &mut tf);
        }
        for c in children.iter() {
            if let Ok(mut m) = models.get_mut(c) {
                m.rotation = Quat::from_rotation_y(npc.facing);
            }
        }
    }
}

/// The soldiers' own clips (`npc_std_idle`, `npc_wk_fwd`, `npc_tactical_fwd`) for the
/// legs, and over them how they hold, aim, fire and reload their gun (`poses.rs`, from
/// the `Arms` that `combat.rs` keeps).
#[allow(clippy::type_complexity)]
fn npc_animation(
    time: Res<Time>,
    mut q: Query<
        (
            &Npc,
            &mut Animator,
            Option<&mut Arms>,
            Option<&Children>,
            Option<&crate::grapple::Hooked>,
        ),
        Without<Dead>,
    >,
    models: Query<&Transform, With<NpcModel>>,
) {
    let dt = time.delta_secs();
    for (npc, mut an, arms, children, hooked) in &mut q {
        let pick = |own: Clip, rico: Clip| if an.rig.has(own) { own } else { rico };
        let (clip, gait) = if npc.speed > (WALK_SPEED + RUN_SPEED) / 2.0 {
            (pick(Clip::NpcRun, Clip::Run), Gait::Run)
        } else if npc.speed > 0.1 {
            (pick(Clip::NpcWalk, Clip::Walk), Gait::Walk)
        } else {
            (pick(Clip::NpcIdle, Clip::Idle), Gait::Idle)
        };
        an.rate = match an.rig.clips.get(&clip).and_then(|c| c.natural_speed) {
            Some(v) if npc.speed > 0.1 => (npc.speed / v).clamp(0.6, 1.6),
            _ => 1.0,
        };
        an.play_blend(clip, 0.25);
        if let Some(mut arms) = arms {
            let model_rot = children
                .and_then(|c| c.iter().find_map(|c| models.get(c).ok()))
                .map_or(Quat::from_rotation_y(npc.facing), |t| t.rotation);
            drive_upper_body(&mut an, &mut arms, gait, model_rot, true, dt);
        }
        // dragged on the hook: arms flailing (`UB_ANIM_GRAPPLED`), no aim
        if hooked.is_some_and(|h| h.pulled || h.lean > 0.0) {
            an.set_layer(crate::rig::Slot::Wield, Some(Clip::NpcStumble), 0.1, false);
            an.set_layer(crate::rig::Slot::Aim, None, 0.1, false);
            an.aim.target = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive_source::JcAssetReader;
    use crate::rbm_loader::RbmLoader;
    use crate::world::{self, WorldConfig};
    use bevy::asset::AssetApp;
    use bevy::asset::io::AssetSourceBuilder;
    use jc2_formats::GameFs;

    /// A thousand soldiers rolled from `enemytypespawnsettings.bin`: about one in ten an
    /// elite (250 hp, in `pd_gov_elite`, mostly with the assault rifle), the rest grunts
    /// (100 hp, `pd_gov_base01`, mostly the Beretta).
    #[test]
    fn roster_rolls_grunts_and_elites() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("enemytypespawnsettings.bin") else {
            return;
        };
        let roster = EnemyRoster(Some(EnemyTypes::parse(&data).unwrap()), 0.0);
        let mut elites = 0;
        let mut berettas = 0;
        let mut rifles = 0;
        for i in 0..1000u32 {
            let (s, doll) = roster.roll(unit_rand(i), unit_rand(i ^ 0x5bd1_e995));
            if s.elite {
                elites += 1;
                assert_eq!((s.health, doll.as_str()), (250.0, "pd_gov_elite.ee"));
                rifles += usize::from(s.weapon == "Assault");
            } else {
                assert_eq!((s.health, doll.as_str()), (100.0, "pd_gov_base01.ee"));
                berettas += usize::from(s.weapon == "Beretta Handgun");
            }
        }
        assert!((70..140).contains(&elites), "{elites} elites");
        assert!(berettas > 450 && rifles > elites / 2, "{berettas} {rifles}");
        assert!(fs.contains("pd_gov_elite.ee"));
    }

    /// Tile 31,14 streams in with its six soldiers: dressed, skinned to the biped and
    /// animated, standing on the ground; the patrol walks off along its route. Needs
    /// JC2_TEST_DIR with the tile, `31x14.nlz`, `pd_gov_base01.ee`, `biped.bsk`,
    /// `std_idle.ban` and `wk_fwd.ban` (the clips are members of `animations.blz`).
    #[test]
    fn village_soldiers_stand_and_patrol() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        for f in [
            "31x14.nlz",
            "terrain_win32_31_14.dat",
            SOLDIER,
            "biped.bsk",
            "wk_fwd.ban",
        ] {
            if !fs.contains_any(f) {
                return;
            }
        }
        let reader_fs = fs.clone();
        let mut app = App::new();
        app.register_asset_source(
            "jc2",
            AssetSourceBuilder::new(move || Box::new(JcAssetReader::new(reader_fs.clone()))),
        )
        .add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            crate::physics::tests::headless_physics(),
        ))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 30.0),
        ))
        .init_asset::<StandardMaterial>()
        .init_asset::<Image>()
        // Headless fixtures still decode required retail BC colour images.
        .register_asset_loader(bevy::image::ImageLoader::new(
            bevy::image::CompressedImageFormats::BC,
        ))
        .init_asset::<RbmAsset>()
        .init_asset::<crate::foliage_alpha::FoliageMaterial>()
        .init_asset::<crate::general_material::GeneralMaterial>()
        .register_asset_loader(RbmLoader::default())
        .insert_resource(Game { fs })
        .insert_resource(WorldConfig {
            tile: (31, 14),
            radius: 0,
            lod: 1,
            models: true,
            physics: true,
            flat_terrain: true,
        })
        .init_asset::<crate::terrain_material::TerrainSplatMaterial>()
        .add_systems(Startup, world::setup)
        .add_systems(Update, (world::stream_tiles, world::spawn_models).chain())
        .add_plugins(NpcPlugin);
        app.finish();
        // the models load on other threads: wait for them by the clock (generously, the
        // machine may be busy), the walking below is counted in frames
        let started = std::time::Instant::now();
        while started.elapsed().as_secs() < 90 {
            app.update();
            if app.world().resource::<Npcs>().spawned.len() >= 5 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // a few frames more for a sixth that should not come
        for _ in 0..30 {
            app.update();
        }
        let n = app.world().resource::<Npcs>().spawned.len();
        // the sixth (the entrance patrol) stands 430 m west, over tile 30,14's ground,
        // which this test does not load: it waits for it
        assert_eq!(n, 5, "soldiers spawned");
        assert_eq!(app.world().resource::<Npcs>().pending.len(), 1);
        let mut q = app
            .world_mut()
            .query::<(Entity, &Npc, &Transform, &Animator, &Soldier)>();
        // by entity: the query's order changes as their components do
        let start: Vec<(Entity, Vec3, bool)> = q
            .iter(app.world())
            .map(|(e, n, t, a, s)| {
                assert_eq!(a.joints.len(), 84);
                assert!(s.health >= 100.0, "{s:?}");
                (
                    e,
                    t.translation,
                    matches!(n.behaviour, Behaviour::Patrol { .. }),
                )
            })
            .collect();
        assert!(start.iter().any(|(_, _, patrol)| *patrol), "no patrol");
        // ten seconds on
        for _ in 0..300 {
            app.update();
        }
        let mut q = app.world_mut().query::<(&Npc, &Transform, &Animator)>();
        let mut walked = 0.0f32;
        for (e, p0, patrol) in &start {
            let (n, t, a) = q.get(app.world(), *e).unwrap();
            if *patrol {
                walked = Vec2::new(t.translation.x - p0.x, t.translation.z - p0.z).length();
                assert!(
                    matches!(a.clip, Clip::Walk | Clip::NpcWalk),
                    "the patrol should be walking: {n:?} {:?}",
                    a.clip
                );
            }
        }
        assert!(walked > 3.0, "the patrol walked {walked} m");
        eprintln!("31,14: {n} soldiers, the patrol walked {walked:.1} m in 10 s");
    }

    /// Tile 51,10's colonel stands with the data's 500 health in his own doll, tagged with
    /// what his death pays, and his guard beside him. Needs JC2_TEST_DIR.
    #[test]
    fn the_colonel_of_51x10_stands_with_his_guard() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        for f in ["pd_ms_gov_shielder.ee", "biped.bsk", "51x10.nlz"] {
            if !fs.contains_any(f) {
                return;
            }
        }
        let Some(mut app) = crate::world::tests::tile_app_at((51, 10), |app| {
            app.add_plugins(NpcPlugin);
        }) else {
            return;
        };
        let started = std::time::Instant::now();
        let colonel = loop {
            app.update();
            let mut q = app.world_mut().query::<(Entity, &ColonelTag, &Soldier)>();
            if let Some((e, c, s)) = q.iter(app.world()).next() {
                break (e, c.0.id.clone(), s.clone());
            }
            assert!(
                started.elapsed().as_secs() < 90,
                "no colonel spawned on 51,10"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(colonel.1, "age40");
        assert_eq!(colonel.2.health, 500.0);
        assert!(colonel.2.elite);
        let name = app
            .world()
            .get::<Name>(colonel.0)
            .unwrap()
            .as_str()
            .to_string();
        assert_eq!(name, "colonel age40");
        // his team's other agent is up (or on its way) too
        let npcs = app.world().resource::<Npcs>();
        let team = app.world().get::<TeamOf>(colonel.0).copied().unwrap();
        assert!(npcs.teams_up.contains(&(team.0, team.1)));
        eprintln!("51,10: colonel {} with {} hp", colonel.1, colonel.2.health);
    }
}
