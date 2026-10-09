//! People on foot: the civilians of the world simulation. Each loaded tile's
//! `CCivilianRoamArea`s (`jc2_formats::civilians::roam_areas`) get up to `--civilians N`
//! people (12 by default, never more than the areas' `capacity`), dressed in a doll that
//! `worldsim.bin`'s `character_types` weigh for the ground's game zone (jungle, city,
//! arctic…), skinned to `civilian_biped.bsk` and animated with the civilians' own clips
//! (`pose_shared_idle/walk/panicrun.ban`). They stroll between random points of their
//! box, pausing now and then; shots or blasts within 30 m send them running away for a
//! few seconds; they die to Rico's guns (a little heat, less than a soldier) and leave
//! with their tile. See `research/formats/civilians.md`.
//!
//! Also here: `TileSim`, the tile data the civilians and the traffic share (roam areas,
//! road splines), read once from each tile's placement file as it streams in.

use crate::Game;
use crate::character::{HEIGHT, RADIUS};
use crate::combat::{Dead, Health};
use crate::coords::to_bevy;
use crate::npc::{DollCache, dress, follow_ground, outfit_ready, request_outfit, turn_toward};
use crate::rbm_loader::RbmAsset;
use crate::rig::{Animator, Clip, Rig, RigData};
use crate::world::Streamer;
use avian3d::prelude::*;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use jc2_formats::civilians::{GameZones, RoamArea, WorldSim, pick, roam_areas};
use jc2_formats::roads::{RoadSpline, container_placements, road_splines};
use jc2_formats::terrain::{TerrainTile, tile_container_name, tile_file_name, tile_origin};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// People per tile unless `--civilians` says otherwise.
pub const DEFAULT_PER_TILE: usize = 12;
/// Strolling pace (m/s; `pose_shared_walk.ban` is made for 0.93).
const WALK_SPEED: f32 = 1.1;
/// Running away (m/s; `pose_shared_panicrun.ban` is made for 4.9).
const FLEE_SPEED: f32 = 4.5;
/// Shots and blasts closer than this scare them (m). Assumed: `aiheatparams.bin`'s
/// civilian events carry ranges of 40–100 whose meaning is not known.
pub const FEAR_RADIUS: f32 = 30.0;
/// How long a fright lasts (s), give or take a second.
const FLEE_SECONDS: f32 = 7.0;
const ARRIVE: f32 = 0.4;
/// Health of a civilian (assumed; a grunt has 100).
const CIVILIAN_HEALTH: f32 = 50.0;
/// Heat a civilian's death adds: `heatsystem.bin` does not say (its event table in
/// `aiheatparams.bin` is unnamed), so the task's +4 — a third of a soldier's.
pub const KILL_HEAT: f32 = 4.0;
/// A body lies this long (s).
const CORPSE_SECONDS: f32 = 30.0;
/// The doll for anyone when `worldsim.bin` can't be read.
const FALLBACK_DOLL: &str = "pd_generic_male_1.ee";
/// The civilians' skeleton and the clips they use on it.
const CIVILIAN_SKELETON: &str = "civilian_biped.bsk";
const CIVILIAN_CLIPS: [Clip; 3] = [Clip::CivIdle, Clip::CivWalk, Clip::CivRun];

/// How many per tile (0: none).
#[derive(Resource, Clone, Copy, Debug)]
pub struct CivilianConfig {
    pub per_tile: usize,
}

impl Default for CivilianConfig {
    fn default() -> Self {
        Self {
            per_tile: DEFAULT_PER_TILE,
        }
    }
}

/// What a streamed tile adds to the world simulation.
#[derive(Clone, Debug, Default)]
pub struct TileExtras {
    pub areas: Vec<RoamArea>,
    pub roads: Vec<RoadSpline>,
    /// The game zone (`jungle`, `city`…) at each area's centre.
    pub area_zones: Vec<String>,
}

/// Roam areas and road splines of the loaded tiles, read once per tile.
#[derive(Resource, Default)]
pub struct TileSim {
    pub tiles: HashMap<(u32, u32), TileExtras>,
    zones: Option<Option<GameZones>>,
}

/// Reads `TileSim`; added by whichever of the civilian and traffic plugins comes first.
pub struct TileSimPlugin;

impl Plugin for TileSimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TileSim>()
            .add_systems(Update, read_tile_extras);
    }
}

fn read_tile_extras(mut sim: ResMut<TileSim>, streamer: Option<Res<Streamer>>, game: Res<Game>) {
    let Some(st) = streamer else { return };
    if sim.zones.is_none() {
        sim.zones = Some(
            game.fs
                .read_inflated("gamezone.bin")
                .ok()
                .and_then(|d| GameZones::parse(&d).ok()),
        );
    }
    let fresh: Vec<(u32, u32)> = st
        .loaded
        .keys()
        .filter(|k| !sim.tiles.contains_key(*k))
        .copied()
        .collect();
    for key in fresh {
        let mut extras = TileExtras::default();
        if let Ok(file) = container_placements(&game.fs, &tile_container_name(key.0, key.1)) {
            extras.areas = roam_areas(&file);
            extras.roads = road_splines(&file);
        }
        if !extras.areas.is_empty() {
            let terrain = game
                .fs
                .read_inflated(&tile_file_name(key.0, key.1))
                .ok()
                .and_then(|d| TerrainTile::parse(&d).ok());
            let (x0, z0) = tile_origin(key.0, key.1);
            let zones = sim.zones.as_ref().and_then(|z| z.as_ref());
            extras.area_zones = extras
                .areas
                .iter()
                .map(|a| {
                    let c = a.world.translation();
                    match (&terrain, zones) {
                        (Some(t), Some(z)) => z.of_material(t.material_local(c[0] - x0, c[2] - z0)),
                        _ => "default",
                    }
                    .to_string()
                })
                .collect();
        }
        sim.tiles.insert(key, extras);
    }
    sim.tiles.retain(|k, _| st.loaded.contains_key(k));
}

/// A person on foot.
#[derive(Component, Debug)]
pub struct Civilian {
    pub area: RoamArea,
    pub facing: f32,
    pub target: Option<Vec3>,
    pub wait: f32,
    pub speed: f32,
    /// Running from here, for this much longer (s).
    pub flee: Option<(Vec3, f32)>,
    seed: u32,
}

impl Civilian {
    pub fn new(area: RoamArea, facing: f32, seed: u32) -> Self {
        Self {
            area,
            facing,
            target: None,
            wait: (seed % 5) as f32 * 0.7,
            speed: 0.0,
            flee: None,
            seed,
        }
    }

    fn rand(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        crate::npc::unit_rand(self.seed)
    }

    /// Run from `from` for a while (a fresh scare restarts the clock).
    pub fn scare(&mut self, from: Vec3) {
        let t = FLEE_SECONDS + self.rand() * 2.0 - 1.0;
        self.flee = Some((from, t));
        self.target = None;
    }
}

#[derive(Clone)]
struct Request {
    container: String,
    area: RoamArea,
    at: Vec3,
    facing: f32,
    tile: (u32, u32),
    seed: u32,
}

struct Pending {
    req: Request,
    skeleton: String,
    models: Vec<Handle<RbmAsset>>,
}

#[derive(Resource, Default)]
pub struct Civilians {
    requests: Vec<Request>,
    pending: Vec<Pending>,
    pub spawned: Vec<(Entity, (u32, u32))>,
    tiles_done: HashSet<(u32, u32)>,
    dolls: DollCache,
    rigs: HashMap<String, Option<Arc<RigData>>>,
    sim: Option<WorldSim>,
    /// Shots Rico had fired when last looked at.
    shots_seen: u32,
}

impl Civilians {
    /// Ask for someone to roam `area`, standing at `at` (Bevy axes) to begin with.
    pub fn request(
        &mut self,
        container: &str,
        area: RoamArea,
        at: Vec3,
        tile: (u32, u32),
        seed: u32,
    ) {
        self.requests.push(Request {
            container: container.to_string(),
            area,
            at,
            facing: crate::npc::unit_rand(seed ^ 0x1234) * std::f32::consts::TAU,
            tile,
            seed,
        });
    }
}

pub struct CivilianPlugin;

impl Plugin for CivilianPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<TileSimPlugin>() {
            app.add_plugins(TileSimPlugin);
        }
        if !app.is_plugin_added::<crate::rig::RigPlugin>() {
            app.add_plugins(crate::rig::RigPlugin);
        }
        app.init_resource::<Civilians>()
            .init_resource::<CivilianConfig>()
            .add_systems(Startup, read_worldsim)
            .add_systems(
                Update,
                (
                    tile_people.after(read_tile_extras),
                    request_models,
                    spawn_people,
                    drop_with_tiles,
                )
                    .chain(),
            )
            .add_systems(Update, (scare, wander, deaths, animation).chain());
    }
}

fn read_worldsim(mut civ: ResMut<Civilians>, game: Res<Game>) {
    civ.sim = game
        .fs
        .read_inflated("worldsim.bin")
        .ok()
        .and_then(|d| WorldSim::parse(&d).ok());
    match &civ.sim {
        Some(s) => info!("civilians: {} kinds of people", s.characters.len()),
        None => warn!("worldsim.bin not read: every civilian in {FALLBACK_DOLL}"),
    }
}

/// Fill the roam areas of tiles that just loaded.
fn tile_people(
    mut civ: ResMut<Civilians>,
    config: Res<CivilianConfig>,
    sim: Res<TileSim>,
    game: Res<Game>,
) {
    for (&key, extras) in &sim.tiles {
        if civ.tiles_done.contains(&key) {
            continue;
        }
        civ.tiles_done.insert(key);
        let areas = &extras.areas;
        let capacity: usize = areas.iter().map(|a| a.capacity as usize).sum();
        let mut left = config.per_tile.min(capacity);
        let mut placed = vec![0usize; areas.len()];
        let mut n = 0u32;
        while left > 0 {
            let mut any = false;
            for (i, a) in areas.iter().enumerate() {
                if left == 0 {
                    break;
                }
                if placed[i] >= a.capacity as usize {
                    continue;
                }
                any = true;
                let seed = key.0 * 7919 + key.1 * 104_729 + n * 31 + i as u32;
                n += 1;
                let zone = extras.area_zones.get(i).map_or("default", String::as_str);
                let container = civ
                    .sim
                    .as_ref()
                    .and_then(|s| pick(&s.characters, zone, crate::npc::unit_rand(seed)))
                    .map(|t| t.doll_container())
                    .filter(|c| game.fs.contains(c))
                    .unwrap_or_else(|| FALLBACK_DOLL.to_string());
                // at a spawn point when there are any left, else anywhere in the box
                let at = match a.spawn_points.get(placed[i]) {
                    Some(p) => *p,
                    None => a.point(
                        crate::npc::unit_rand(seed ^ 0xa1) * 2.0 - 1.0,
                        crate::npc::unit_rand(seed ^ 0xb2) * 2.0 - 1.0,
                    ),
                };
                civ.request(&container, a.clone(), to_bevy(at), key, seed);
                placed[i] += 1;
                left -= 1;
            }
            if !any {
                break;
            }
        }
        if n > 0 {
            info!("tile {key:?}: {n} civilians in {} roam areas", areas.len());
        }
    }
    let loaded: HashSet<(u32, u32)> = sim.tiles.keys().copied().collect();
    civ.tiles_done.retain(|k| loaded.contains(k));
}

fn request_models(mut civ: ResMut<Civilians>, assets: Res<AssetServer>, game: Res<Game>) {
    let requests = std::mem::take(&mut civ.requests);
    for req in requests {
        let Some((skeleton, models)) = request_outfit(
            &game,
            &assets,
            &mut civ.dolls,
            &req.container,
            req.seed as usize,
        ) else {
            continue;
        };
        civ.pending.push(Pending {
            req,
            skeleton,
            models,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_people(
    mut commands: Commands,
    mut civ: ResMut<Civilians>,
    assets: Res<AssetServer>,
    rbms: Res<Assets<RbmAsset>>,
    game: Res<Game>,
    hero_rig: Option<Res<Rig>>,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
    spatial: SpatialQuery,
) {
    if civ.pending.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut civ.pending);
    let mut keep = Vec::new();
    for p in pending {
        if !outfit_ready(&assets, &rbms, &p.models) {
            keep.push(p);
            continue;
        }
        // on the ground (wait for its collider)
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
        let rig = match &hero_rig {
            Some(r) if r.file == p.skeleton => Some(r.0.clone()),
            _ => civ
                .rigs
                .entry(p.skeleton.clone())
                .or_insert_with(|| {
                    if p.skeleton == CIVILIAN_SKELETON {
                        Rig::load_with(&game.fs, &p.skeleton, &CIVILIAN_CLIPS)
                    } else {
                        Rig::load(&game.fs, &p.skeleton)
                    }
                    .map(|r| r.0)
                })
                .clone(),
        };
        let pos = Vec3::new(p.req.at.x, ground + HEIGHT / 2.0, p.req.at.z);
        let body = commands
            .spawn((
                RigidBody::Kinematic,
                Collider::capsule(RADIUS, HEIGHT - 2.0 * RADIUS),
                Transform::from_translation(pos),
                Visibility::default(),
                Civilian::new(p.req.area.clone(), p.req.facing, p.req.seed),
                Health::new(CIVILIAN_HEALTH),
                Name::new(format!("civilian {}", p.req.container)),
            ))
            .id();
        if let Some(an) = dress(
            &mut commands,
            body,
            p.req.facing,
            rig,
            &p.models,
            &rbms,
            &mut bindposes,
        ) {
            commands.entity(body).insert(an);
        }
        civ.spawned.push((body, p.req.tile));
        debug!("civilian {} at {pos:?}", p.req.container);
    }
    if keep.is_empty() && civ.requests.is_empty() {
        info!("civilians: {} standing", civ.spawned.len());
    }
    civ.pending.extend(keep);
}

fn drop_with_tiles(
    mut commands: Commands,
    mut civ: ResMut<Civilians>,
    streamer: Option<Res<Streamer>>,
) {
    let Some(st) = streamer else { return };
    let mut keep = Vec::with_capacity(civ.spawned.len());
    for (e, tile) in std::mem::take(&mut civ.spawned) {
        if st.loaded.contains_key(&tile) {
            keep.push((e, tile));
        } else if let Ok(mut ec) = commands.get_entity(e) {
            ec.despawn();
        }
    }
    civ.spawned = keep;
    civ.pending.retain(|p| st.loaded.contains_key(&p.req.tile));
    civ.requests.retain(|r| st.loaded.contains_key(&r.tile));
}

/// Shots near Rico and blasts send the people around them running; so does being hurt.
#[allow(clippy::type_complexity)]
fn scare(
    mut civ: ResMut<Civilians>,
    arsenal: Option<Res<crate::combat::Arsenal>>,
    blasts: Option<Res<crate::explosion::Blasts>>,
    heroes: Query<&Transform, With<crate::character::Hero>>,
    mut people: Query<(&mut Civilian, &Transform, &Health), Without<Dead>>,
) {
    let mut sources: Vec<Vec3> = Vec::new();
    if let Some(a) = arsenal {
        if a.shots != civ.shots_seen
            && let Ok(rico) = heroes.single()
        {
            sources.push(rico.translation);
        }
        civ.shots_seen = a.shots;
    }
    if let Some(b) = blasts {
        sources.extend(b.fired.iter().map(|b| b.at));
    }
    let rico = heroes.single().ok().map(|t| t.translation);
    for (mut c, tf, h) in &mut people {
        let near = sources
            .iter()
            .filter(|s| s.distance(tf.translation) < FEAR_RADIUS)
            .min_by(|a, b| {
                a.distance(tf.translation)
                    .total_cmp(&b.distance(tf.translation))
            });
        if let Some(s) = near {
            c.scare(*s);
        } else if h.hp < h.max && c.flee.is_none() {
            c.scare(rico.unwrap_or(tf.translation - Vec3::Z));
        }
    }
}

/// Stroll inside the area, or run from a fright.
fn wander(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut q: Query<(Entity, &mut Civilian, &mut Transform, &Children), Without<Dead>>,
    mut models: Query<&mut Transform, (With<crate::npc::NpcModel>, Without<Civilian>)>,
) {
    let dt = time.delta_secs();
    for (e, mut c, mut tf, children) in &mut q {
        let mut velocity = Vec3::ZERO;
        if let Some((from, left)) = c.flee {
            let away = Vec3::new(tf.translation.x - from.x, 0.0, tf.translation.z - from.z);
            let dir = away
                .try_normalize()
                .unwrap_or_else(|| crate::coords::facing_dir(c.facing));
            velocity = dir * FLEE_SPEED;
            let left = left - dt;
            c.flee = (left > 0.0).then_some((from, left));
            if c.flee.is_none() {
                c.wait = 2.0;
            }
        } else {
            if c.target.is_none() {
                c.wait -= dt;
                if c.wait <= 0.0 {
                    let (u, v) = (c.rand() * 2.0 - 1.0, c.rand() * 2.0 - 1.0);
                    c.target = Some(to_bevy(c.area.point(u * 0.9, v * 0.9)));
                    c.wait = 2.0 + c.rand() * 6.0;
                }
            }
            if let Some(t) = c.target {
                let to = Vec3::new(t.x - tf.translation.x, 0.0, t.z - tf.translation.z);
                let d = to.length();
                if d < ARRIVE {
                    c.target = None;
                } else {
                    velocity = to / d * WALK_SPEED.min(d / dt.max(1e-3));
                }
            }
        }
        let speed = velocity.length();
        c.speed = speed;
        if speed > 0.0 {
            tf.translation += velocity * dt;
            let mut facing = c.facing;
            turn_toward(&mut facing, velocity / speed, dt);
            c.facing = facing;
            follow_ground(&spatial, e, &mut tf);
        }
        for ch in children.iter() {
            if let Ok(mut m) = models.get_mut(ch) {
                m.rotation = Quat::from_rotation_y(c.facing);
            }
        }
    }
}

/// People at zero health fall, add a little heat and lie a while.
#[allow(clippy::type_complexity)]
fn deaths(
    mut commands: Commands,
    time: Res<Time>,
    heat: Option<ResMut<crate::heat::Heat>>,
    fresh: Query<(Entity, &Health), (With<Civilian>, Without<Dead>)>,
    mut dead: Query<(Entity, &mut Dead, &Children), With<Civilian>>,
    mut models: Query<&mut Transform, (With<crate::npc::NpcModel>, Without<Civilian>)>,
) {
    let mut heat = heat;
    for (e, h) in &fresh {
        if h.hp <= 0.0 {
            commands
                .entity(e)
                .insert(Dead(0.0))
                .remove::<(Collider, RigidBody)>();
            if let Some(heat) = heat.as_mut() {
                heat.add(KILL_HEAT);
            }
        }
    }
    let dt = time.delta_secs();
    for (e, mut d, children) in &mut dead {
        d.0 += dt;
        let k = (d.0 / 0.5).min(1.0);
        for ch in children.iter() {
            if let Ok(mut m) = models.get_mut(ch) {
                let yaw = m.rotation.to_euler(EulerRot::YXZ).0;
                m.rotation = Quat::from_rotation_y(yaw)
                    * Quat::from_rotation_x(std::f32::consts::FRAC_PI_2 * k);
                m.translation.y = -HEIGHT / 2.0 + 0.15 * k;
            }
        }
        if d.0 > CORPSE_SECONDS {
            commands.entity(e).despawn();
        }
    }
}

/// The clip for the pace: the civilian clips when the rig has them, else Rico's.
fn animation(mut q: Query<(&Civilian, &mut Animator, Has<Dead>)>) {
    for (c, mut an, dead) in &mut q {
        let has = |clip: Clip| an.rig.clips.contains_key(&clip);
        let (walk, run, idle) = if has(Clip::CivWalk) {
            (Clip::CivWalk, Clip::CivRun, Clip::CivIdle)
        } else {
            (Clip::Walk, Clip::Run, Clip::Idle)
        };
        if dead {
            an.rate = 0.0;
            continue;
        }
        let clip = if c.speed > 3.0 {
            run
        } else if c.speed > 0.1 {
            walk
        } else {
            idle
        };
        an.rate = match an.rig.clips.get(&clip).and_then(|l| l.natural_speed) {
            Some(v) if c.speed > 0.1 => (c.speed / v).clamp(0.6, 1.6),
            _ => 1.0,
        };
        an.play(clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::to_engine;
    use crate::fleet::testing;
    use jc2_formats::placement::Transform3;

    /// A position in Bevy axes inside a roam area.
    fn inside(area: &RoamArea, p: Vec3, margin: f32) -> bool {
        area.contains(to_engine(p), margin)
    }

    /// A 12 × 20 m roam area on the flat test tile, around `centre` (Bevy axes).
    fn area(centre: Vec3) -> RoamArea {
        let c = to_engine(centre);
        let yaw: f32 = 0.5;
        RoamArea {
            name: "test".into(),
            world: Transform3::from_mat4x3([
                yaw.cos(),
                0.0,
                -yaw.sin(),
                0.0,
                1.0,
                0.0,
                yaw.sin(),
                0.0,
                yaw.cos(),
                c[0],
                c[1],
                c[2],
            ]),
            half_extents: [6.0, 5.0, 10.0],
            capacity: 4,
            spawn_points: Vec::new(),
        }
    }

    /// Four people in a box until they stand: the app, their entities and the box.
    fn people(n: u32) -> Option<(App, Vec<Entity>, RoamArea)> {
        let needs = [FALLBACK_DOLL, CIVILIAN_SKELETON, "pose_shared_walk.ban"];
        let (mut app, ground, spawn) = testing::app(&[FALLBACK_DOLL], |app| {
            app.init_resource::<crate::explosion::Blasts>()
                .init_resource::<crate::heat::Heat>()
                .add_plugins(CivilianPlugin);
        })?;
        {
            let fs = &app.world().resource::<Game>().fs;
            if needs.iter().any(|f| !fs.contains_any(f)) {
                return None;
            }
        }
        let a = area(Vec3::new(spawn.x, ground, spawn.z));
        for i in 0..n {
            let at = to_bevy(a.point(i as f32 * 0.4 - 0.6, 0.3 - i as f32 * 0.3));
            app.world_mut().resource_mut::<Civilians>().request(
                FALLBACK_DOLL,
                a.clone(),
                at,
                (31, 14),
                i * 17 + 3,
            );
        }
        let started = std::time::Instant::now();
        while started.elapsed().as_secs() < 90 {
            app.update();
            if app.world().resource::<Civilians>().spawned.len() as u32 >= n {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let ids: Vec<Entity> = app
            .world()
            .resource::<Civilians>()
            .spawned
            .iter()
            .map(|(e, _)| *e)
            .collect();
        assert_eq!(ids.len() as u32, n, "civilians spawned");
        Some((app, ids, a))
    }

    /// Four people stroll about their roam area for 30 s: they walk (on the civilian
    /// skeleton with the walk clip) and never leave the box. Needs JC2_TEST_DIR.
    #[test]
    fn civilians_stroll_inside_their_area() {
        let Some((mut app, ids, a)) = people(4) else {
            return;
        };
        let start: Vec<Vec3> = ids
            .iter()
            .map(|e| app.world().get::<Transform>(*e).unwrap().translation)
            .collect();
        let mut walked = vec![0.0f32; ids.len()];
        let mut walking_clip = false;
        for _ in 0..(30 * 64) {
            app.update();
            for (i, e) in ids.iter().enumerate() {
                let t = app.world().get::<Transform>(*e).unwrap().translation;
                assert!(inside(&a, t, 0.6), "{i} left the area: {t:?}");
                walked[i] = walked[i].max(Vec2::new(t.x - start[i].x, t.z - start[i].z).length());
                if let Some(an) = app.world().get::<Animator>(*e) {
                    assert_eq!(an.joints.len(), 40, "civilian_biped");
                    walking_clip |= an.clip == Clip::CivWalk;
                }
            }
        }
        assert!(walked.iter().all(|w| *w > 1.0), "walked {walked:?}");
        assert!(walking_clip, "nobody played the walk clip");
        eprintln!("civilians walked {walked:?} m and stayed in their box");
    }

    /// A blast 12 m away: everyone near it runs, and gets further from it. Then one is
    /// shot dead.
    #[test]
    fn a_blast_makes_them_flee() {
        let Some((mut app, ids, a)) = people(3) else {
            return;
        };
        let centre = to_bevy(a.world.translation());
        let blast = centre + Vec3::new(12.0, 0.0, 0.0);
        let dist = |app: &App| -> Vec<f32> {
            ids.iter()
                .map(|e| {
                    let t = app.world().get::<Transform>(*e).unwrap().translation;
                    Vec2::new(t.x - blast.x, t.z - blast.z).length()
                })
                .collect()
        };
        let before = dist(&app);
        app.world_mut()
            .resource_mut::<crate::explosion::Blasts>()
            .fired
            .push(crate::explosion::Blast {
                at: blast,
                radius: 5.0,
                damage: 0.0,
                impulse: 0.0,
            });
        app.update();
        app.world_mut()
            .resource_mut::<crate::explosion::Blasts>()
            .fired
            .clear();
        for e in &ids {
            assert!(app.world().get::<Civilian>(*e).unwrap().flee.is_some());
        }
        for _ in 0..(3 * 64) {
            app.update();
        }
        let after = dist(&app);
        for (b, a) in before.iter().zip(&after) {
            assert!(a > &(b + 8.0), "from {b:.1} m to {a:.1} m");
        }
        eprintln!("fled: {before:?} → {after:?}");
        // shot dead: one falls, without a collider, and the heat rises by a civilian's worth
        app.world_mut().get_mut::<Health>(ids[0]).unwrap().hp = 0.0;
        app.update();
        app.update();
        assert!(app.world().get::<Dead>(ids[0]).is_some());
        assert!(app.world().get::<Collider>(ids[0]).is_none());
        assert_eq!(app.world().resource::<crate::heat::Heat>().value, KILL_HEAT);
    }
}
