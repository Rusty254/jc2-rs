//! Heat and reinforcements. The heat runs 0 … `max_heat` (100, `heatsystem.bin`) in five
//! levels of 20 — the bands of `heatspawn.bin`. How high it can go depends on the chaos
//! total (`heat_level_chaos_N`: levels 1 and 2 from the start, 3 from 100 000 chaos, 4
//! from 200 000, 5 from 300 000; `chaos.rs` sets [`Heat::cap`]). The events that raise it
//! are `ai\aiheatparams.bin`'s: a kill (`Points` 100 within `Range` 40 m of a soldier,
//! after `Delay` 0.5 s) and an explosion (100 within 50 m), identified by the civilians'
//! reaction they carry; `Points` 100 is taken as one heat level (the event names did not
//! hash, and how the game turns points into heat is not in the data: assumed). Once no
//! soldier has seen Rico for a while it cools off. From level 1 the government sends
//! reinforcements: soldiers on foot, one every `respawn_delay`, appearing 80–120 m away
//! behind the camera and running to where Rico is.
//!
//! **Vehicles.** From level 2 a government vehicle comes as well: the band's
//! four-wheeled land unit with the most seats (`unit_type` → `entity_file`:
//! `lave.v047_jeep_small.ee` with 2 `num_seats` in the 20–40 band, the police ATV in
//! 40–60), asked of the fleet on a road 150–300 m from Rico (a loaded tile), with a
//! soldier at the wheel (`crew.rs`). It drives with the traffic AI
//! (`traffic::AiDriver`) along the shortest way by road to the road node nearest Rico
//! (Dijkstra over the road graph, `road_distances`), then towards him at each junction;
//! within `DISMOUNT` m of him (or stuck, or at the end of the road) it stops and its
//! seats get out as foot reinforcements. From level 3 an attack
//! helicopter (the band's air unit, `arve.v009_military_helicopter.ee`) appears 150 m
//! away, 40 m up, and circles Rico at `GUNSHIP_RADIUS` m and `GUNSHIP_ALTITUDE` m over
//! him, its door gun firing bursts (`crew::HeliPilot`). One of each at a time; both can
//! be hijacked like any crewed vehicle. (The gunship needs level 3, so 100 000 chaos —
//! or `--heat 50`, which starts above the cap and only cools.)

use crate::Game;
use crate::character::Hero;
use crate::combat::Dead;
use crate::coords::{to_engine, yaw_toward};
use crate::crew::{Crew, CrewKind, CrewRequest, HeliPilot};
use crate::explosion::Blasts;
use crate::fleet::{Fleet, Place, Request};
use crate::heli::Heli;
use crate::npc::{EnemyRoster, Npc, Npcs};
use crate::traffic::{AiDriver, Traffic};
use crate::vehicle::{AiDriven, Occupied, Vehicle};
use crate::world::{FlyCamera, Streamer};
use avian3d::prelude::Position;
use bevy::prelude::*;
use jc2_formats::heat::{GOVERNMENT_HEAT, HeatEvent, HeatEvents, HeatLevels, HeatSpawn};
use jc2_formats::terrain::tile_index;

/// Without `aiheatparams.bin`: what a soldier's death adds (assumed).
const KILL_HEAT: f32 = 12.0;
/// Without `aiheatparams.bin`: what a blast within `BLAST_NOTICE` of a living soldier
/// adds (assumed).
const BLAST_HEAT: f32 = 8.0;
const BLAST_NOTICE: f32 = 40.0;
/// Seconds out of every soldier's sight before the heat starts falling (assumed), and
/// how fast it falls then (per second; a level in 10 s).
const COOLDOWN: f32 = 20.0;
const DECAY: f32 = 2.0;
/// Reinforcements appear this far from Rico (m).
const SPAWN_MIN: f32 = 80.0;
const SPAWN_MAX: f32 = 120.0;
/// Never more than this many reinforcements up at once, whatever the band says.
const MAX_REINFORCEMENTS: u32 = 6;
/// Between reinforcements when the band has no `respawn_delay` (s).
const FALLBACK_DELAY: f32 = 8.0;
/// They re-aim at Rico's position this often (s).
const RETARGET: f32 = 4.0;

/// The government's attention.
#[derive(Resource, Debug)]
pub struct Heat {
    pub value: f32,
    pub max: f32,
    /// The most it can reach at the chaos so far (`heat_level_chaos_N`).
    pub cap: f32,
    /// Event heat waiting out its `Delay`: (seconds left, amount).
    pending: Vec<(f32, f32)>,
    /// Seconds since a soldier last saw Rico.
    pub since_contact: f32,
    spawn_timer: f32,
    retarget: f32,
    seed: u32,
    /// Reinforcements sent so far.
    pub sent: u32,
}

impl Default for Heat {
    fn default() -> Self {
        Self {
            value: 0.0,
            max: 100.0,
            cap: 100.0,
            pending: Vec::new(),
            since_contact: 0.0,
            spawn_timer: 0.0,
            retarget: 0.0,
            seed: 0x2545_f491,
            sent: 0,
        }
    }
}

impl Heat {
    /// Starting at `value` (`--heat`), as if a soldier had just seen Rico.
    pub fn starting_at(value: f32) -> Self {
        Self {
            value: value.max(0.0),
            ..Self::default()
        }
    }

    /// 0 (none) … 5.
    pub fn level(&self) -> u32 {
        if self.value <= 0.0 {
            0
        } else {
            ((self.value / (self.max / 5.0)).ceil() as u32).clamp(1, 5)
        }
    }

    pub fn add(&mut self, v: f32) {
        let top = self.cap.min(self.max);
        // the cap stops a rise; heat already above it (the cap fell) only cools
        if v > 0.0 && self.value < top {
            self.value = (self.value + v).min(top);
        } else if v < 0.0 {
            self.value = (self.value + v).max(0.0);
        }
        self.since_contact = 0.0;
    }

    /// Heat that counts after `delay` seconds.
    pub fn add_later(&mut self, v: f32, delay: f32) {
        if delay <= 0.0 {
            self.add(v);
        } else {
            self.pending.push((delay, v));
        }
    }

    /// Time passes; `contact` when a soldier sees Rico now.
    pub fn tick(&mut self, dt: f32, contact: bool) {
        let mut due = 0.0;
        self.pending.retain_mut(|(t, v)| {
            *t -= dt;
            if *t <= 0.0 {
                due += *v;
                false
            } else {
                true
            }
        });
        if due > 0.0 {
            self.add(due);
        }
        if contact {
            self.since_contact = 0.0;
        } else {
            self.since_contact += dt;
        }
        if self.since_contact > COOLDOWN {
            self.value = (self.value - DECAY * dt).max(0.0);
        }
    }
}

/// `heatspawn.bin`, `aiheatparams.bin`'s events and `heatsystem.bin`'s levels, read once.
#[derive(Resource, Default)]
pub struct HeatTables(
    pub Option<HeatSpawn>,
    pub Option<HeatEvents>,
    pub HeatLevels,
);

impl HeatTables {
    /// The heat an event adds: its `Points` with 100 points one level's width (20).
    pub fn heat_of(&self, e: &HeatEvent) -> f32 {
        e.points / 100.0 * self.2.level_width()
    }
}

/// A soldier sent by the heat.
#[derive(Component)]
pub struct Reinforcement;

pub struct HeatPlugin;

impl Plugin for HeatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Heat>()
            .init_resource::<HeatTables>()
            .init_resource::<HeatVehicles>()
            .add_systems(Startup, read_heat)
            .add_systems(Update, (heat_events, reinforce).chain())
            .add_systems(
                Update,
                (send_vehicles, adopt_vehicles, convoys)
                    .chain()
                    .after(reinforce),
            );
    }
}

fn read_heat(mut heat: ResMut<Heat>, mut tables: ResMut<HeatTables>, game: Option<Res<Game>>) {
    let Some(game) = game else { return };
    if let Some(m) = game
        .fs
        .read_inflated("heatsystem.bin")
        .ok()
        .and_then(|d| jc2_formats::heat::max_heat(&d).ok().flatten())
    {
        heat.max = m.max(1.0);
    }
    tables.0 = game
        .fs
        .read_inflated("heatspawn.bin")
        .ok()
        .and_then(|d| HeatSpawn::parse(&d).ok());
    tables.1 = game
        .fs
        .read_inflated("aiheatparams.bin")
        .ok()
        .and_then(|d| HeatEvents::parse(&d).ok())
        .filter(|e| e.kill().is_some());
    if let Some(l) = game
        .fs
        .read_inflated("heatsystem.bin")
        .ok()
        .and_then(|d| HeatLevels::parse(&d).ok())
    {
        tables.2 = l;
    }
    match &tables.1 {
        Some(e) => info!(
            "heat events: a kill {} within {} m, a blast {} within {} m",
            e.kill().map_or(0.0, |k| tables.heat_of(k)),
            e.kill().map_or(0.0, |k| k.range),
            e.explosion().map_or(0.0, |k| tables.heat_of(k)),
            e.explosion().map_or(0.0, |k| k.range),
        ),
        None => warn!("aiheatparams.bin not read: heat from the assumed numbers"),
    }
}

/// Some soldier other than `except` stands within `range` of `at` (any range when 0).
fn noticed(
    soldiers: &Query<(Entity, &Npc, &Transform), Without<Dead>>,
    at: Vec3,
    range: f32,
    except: Option<Entity>,
) -> bool {
    soldiers
        .iter()
        .any(|(e, _, t)| Some(e) != except && (range <= 0.0 || t.translation.distance(at) < range))
}

/// Deaths and blasts raise the heat; soldiers who see Rico keep it up.
#[allow(clippy::type_complexity)]
fn heat_events(
    time: Res<Time>,
    mut heat: ResMut<Heat>,
    tables: Res<HeatTables>,
    blasts: Option<Res<Blasts>>,
    fallen: Query<(Entity, &Transform), (With<Npc>, Added<Dead>)>,
    soldiers: Query<(Entity, &Npc, &Transform), Without<Dead>>,
) {
    let before = heat.level();
    let events = tables.1.as_ref();
    for (who, tf) in &fallen {
        match events.and_then(|e| e.kill()) {
            // the data's kill: noticed by a soldier within its range
            Some(k) => {
                if noticed(&soldiers, tf.translation, k.range, Some(who)) {
                    let v = tables.heat_of(k);
                    heat.add_later(v, k.delay);
                }
            }
            None => heat.add(KILL_HEAT),
        }
    }
    if let Some(b) = blasts {
        let (amount, range, delay) = match events.and_then(|e| e.explosion()) {
            Some(x) => (tables.heat_of(x), x.range, x.delay),
            None => (BLAST_HEAT, BLAST_NOTICE, 0.0),
        };
        for blast in &b.fired {
            if noticed(&soldiers, blast.at, range, None) {
                heat.add_later(amount, delay);
            }
        }
    }
    let contact = soldiers.iter().any(|(_, n, _)| n.alert);
    heat.tick(time.delta_secs(), contact);
    if heat.level() != before {
        info!("heat: level {} ({:.0})", heat.level(), heat.value);
    }
}

/// Send soldiers while the heat lasts, and keep them coming at Rico.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn reinforce(
    time: Res<Time>,
    mut heat: ResMut<Heat>,
    tables: Res<HeatTables>,
    roster: Res<EnemyRoster>,
    mut npcs: ResMut<Npcs>,
    heroes: Query<&Transform, With<Hero>>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    mut sent: Query<&mut Npc, (With<Reinforcement>, Without<Dead>)>,
) {
    let Ok(rico) = heroes.single().map(|t| t.translation) else {
        return;
    };
    let dt = time.delta_secs();
    // the ones on their way follow him
    heat.retarget -= dt;
    if heat.retarget <= 0.0 {
        heat.retarget = RETARGET;
        for mut n in &mut sent {
            if !n.alert {
                n.send_to(rico);
            }
        }
    }
    if heat.level() == 0 {
        heat.spawn_timer = heat.spawn_timer.min(2.0);
        return;
    }
    let tables = tables.0.as_ref();
    let band = tables.and_then(|t| t.band(GOVERNMENT_HEAT, heat.value));
    let cap = band
        .zip(tables)
        .map_or(heat.level() * 2, |(b, t)| t.land_soldiers(b))
        .clamp(1, MAX_REINFORCEMENTS);
    let delay = band
        .and_then(|b| b.waves.first())
        .map(|w| w.respawn_delay)
        .filter(|d| *d > 0.0)
        .unwrap_or(FALLBACK_DELAY);
    heat.spawn_timer -= dt;
    let up = sent.iter().count() as u32 + npcs.reinforcements_waiting() as u32;
    if heat.spawn_timer > 0.0 || up >= cap {
        return;
    }
    heat.spawn_timer = delay;
    // out of view: behind the camera, give or take 60°
    heat.seed = heat
        .seed
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    let r = crate::npc::unit_rand(heat.seed);
    let back = cams
        .single()
        .map(|c| {
            let f = c.forward().as_vec3();
            (-f.x).atan2(-f.z)
        })
        .unwrap_or(r * std::f32::consts::TAU);
    let angle = back + (r - 0.5) * std::f32::consts::FRAC_PI_3 * 2.0;
    let dist = SPAWN_MIN + (SPAWN_MAX - SPAWN_MIN) * crate::npc::unit_rand(heat.seed ^ 0xa5a5);
    let at = rico + Vec3::new(angle.sin(), 0.0, angle.cos()) * dist;
    let (ax, az) = crate::coords::engine_xz(at);
    let Some(tile) = jc2_formats::terrain::tile_index(ax, az) else {
        return;
    };
    npcs.reinforce(&roster, at, rico, tile, heat.seed);
    heat.sent += 1;
    debug!("heat {}: a soldier sent from {at:?}", heat.level());
}

// ---------------------------------------------------------------- vehicles

/// From this heat level a car of soldiers comes along the roads …
pub const CONVOY_LEVEL: u32 = 2;
/// … and from this one an attack helicopter.
pub const GUNSHIP_LEVEL: u32 = 3;
/// The car is asked for this far from Rico along a road (m).
const CONVOY_MIN: f32 = 150.0;
const CONVOY_MAX: f32 = 300.0;
/// Its cruising speed (m/s).
const CONVOY_CRUISE: f32 = 14.0;
/// It stops and the soldiers get out this near Rico (m), or after this long without
/// headway (s), or this near the end of its road (m).
pub const DISMOUNT: f32 = 30.0;
const CONVOY_STUCK: f32 = 6.0;
const CONVOY_ROAD_END: f32 = 12.0;
/// A car this far from Rico has lost its way and goes (m).
const CONVOY_LOST: f32 = 600.0;
/// At most this many soldiers get out of one car.
const MAX_SEATS: u32 = 4;
/// A new vehicle of a kind at most this often (s).
const VEHICLE_INTERVAL: f32 = 20.0;
/// The helicopter: appears this far away and this high over the ground there, then
/// circles Rico at the radius, the altitude over the ground under him.
const GUNSHIP_SPAWN: f32 = 150.0;
const GUNSHIP_START_HEIGHT: f32 = 40.0;
pub const GUNSHIP_RADIUS: f32 = 50.0;
pub const GUNSHIP_ALTITUDE: f32 = 30.0;
/// When the band names none that can be built.
const FALLBACK_CAR: &str = "lave.v047_jeep_small.ee";
const FALLBACK_HELI: &str = "arve.v009_military_helicopter.ee";

/// A vehicle the heat sent.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum HeatVehicle {
    /// A car driving at Rico with `seats` soldiers aboard (0 once they are out, or once
    /// Rico took it).
    Convoy { seats: u32 },
    /// An attack helicopter hunting him.
    Gunship,
}

enum Asked {
    Convoy { driver: AiDriver, seats: u32 },
    Gunship,
}

/// The vehicles asked of the fleet and not built yet (where, what, when), the timers,
/// and how many were sent.
#[derive(Resource, Default)]
pub struct HeatVehicles {
    pending: Vec<(Vec3, Asked, f32)>,
    convoy_timer: f32,
    gunship_timer: f32,
    pub convoys: u32,
    pub gunships: u32,
}

/// The container of a band's unit: its `entity_file`, bare and lower case.
fn unit_container(file: &str) -> String {
    jc2_formats::placement::bare_name(&file.replace('\\', "/")).to_ascii_lowercase()
}

/// Not a motorbike or a scooter (the raycast car needs four wheels).
fn four_wheels(container: &str) -> bool {
    !["_mc", "v007_", "moped", "vespa"]
        .iter()
        .any(|k| container.contains(k))
}

/// The band's land unit with four wheels and the most seats (its container and seats) and
/// first air unit, each one the game has.
fn band_units(
    tables: Option<&HeatSpawn>,
    heat: f32,
    game: &Game,
) -> (Option<(String, u32)>, Option<String>) {
    let mut car = None;
    let mut heli = None;
    if let Some(t) = tables
        && let Some(band) = t.band(GOVERNMENT_HEAT, heat)
    {
        for (name, _) in band.waves.iter().flat_map(|w| &w.units) {
            let Some(u) = t.unit(name) else { continue };
            let c = unit_container(&u.entity_file);
            if !game.fs.contains(&c) {
                continue;
            }
            match u.kind.as_str() {
                "land"
                    if four_wheels(&c)
                        && u.num_seats > car.as_ref().map_or(0, |(_, n): &(String, u32)| *n) =>
                {
                    car = Some((c, u.num_seats));
                }
                "air" if heli.is_none() => heli = Some(c),
                _ => {}
            }
        }
    }
    (
        car.or_else(|| {
            game.fs
                .contains(FALLBACK_CAR)
                .then(|| (FALLBACK_CAR.into(), 2))
        }),
        heli.or_else(|| {
            game.fs
                .contains(FALLBACK_HELI)
                .then(|| FALLBACK_HELI.into())
        }),
    )
}

/// The road distance (m, by the edges' lengths) from every node within `limit` of the
/// `goal` node, and the edge each takes towards it (Dijkstra over the road graph).
pub fn road_distances(
    graph: &jc2_formats::roads::RoadGraph,
    goal: usize,
    limit: f32,
) -> std::collections::HashMap<usize, (f32, Option<usize>)> {
    use std::cmp::Reverse;
    use std::collections::{BinaryHeap, HashMap};
    let mut best: HashMap<usize, (f32, Option<usize>)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    best.insert(goal, (0.0, None));
    heap.push(Reverse((0u32, goal)));
    while let Some(Reverse((dm, n))) = heap.pop() {
        let d = dm as f32 / 10.0;
        if best.get(&n).is_some_and(|b| b.0 < d) {
            continue;
        }
        for &e in &graph.adjacency[n] {
            let m = graph.other_end(e, n);
            let nd = d + graph.edges[e].length.max(1) as f32;
            if nd > limit || best.get(&m).is_some_and(|b| b.0 <= nd) {
                continue;
            }
            best.insert(m, (nd, Some(e)));
            heap.push(Reverse(((nd * 10.0) as u32, m)));
        }
    }
    best
}

/// A lane point 150–300 m from Rico on a road whose tile is loaded, headed for the end
/// of its road with the shorter way by road to him, with that way planned: where,
/// facing, the driver and the tile.
fn road_spot(
    net: &mut jc2_formats::roads::RoadNetwork,
    rico: Vec3,
    loaded: &dyn Fn(Vec3) -> Option<(u32, u32)>,
    r: f32,
) -> Option<(Vec3, Quat, AiDriver, (u32, u32))> {
    let e = to_engine(rico);
    let flat = |p: [f32; 3]| (p[0] - e[0]).powi(2) + (p[2] - e[2]).powi(2);
    let near = net.graph.nodes_near(e[0], e[2], CONVOY_MAX + 300.0);
    let goal = near
        .iter()
        .copied()
        .min_by(|a, b| flat(net.graph.nodes[*a].pos).total_cmp(&flat(net.graph.nodes[*b].pos)))?;
    let ways = road_distances(&net.graph, goal, 3.0 * CONVOY_MAX);
    let mut edges: Vec<usize> = near
        .into_iter()
        .flat_map(|n| net.graph.adjacency[n].clone())
        .collect();
    edges.sort();
    edges.dedup();
    let mut candidates = Vec::new();
    for edge in edges {
        let (a, b) = (
            net.graph.edges[edge].from as usize,
            net.graph.edges[edge].to as usize,
        );
        // the end it gets to Rico from soonest by road (both unknown: not this road)
        let toward = match (ways.get(&a), ways.get(&b)) {
            (Some(da), Some(db)) if da.0 <= db.0 => a,
            (Some(_), Some(_)) | (None, Some(_)) => b,
            (Some(_), None) => a,
            (None, None) => continue,
        };
        let Some(route) = crate::traffic::lane(net, edge, toward) else {
            continue;
        };
        let beyond = ways[&toward].0;
        // what is left of this road from each point on it
        let mut left = vec![0.0f32; route.len()];
        for i in (0..route.len().saturating_sub(1)).rev() {
            left[i] = left[i + 1] + route[i].distance(route[i + 1]);
        }
        for (i, p) in route.iter().enumerate() {
            let d = Vec2::new(p.x - rico.x, p.z - rico.z).length();
            // a way that is not much longer than the straight line (no road that winds
            // away up a valley), and enough of this road left to set off on
            let by_road = left[i] + beyond;
            if i + 1 < route.len()
                && left[i] > 40.0
                && by_road < 1.5 * d + 60.0
                && (CONVOY_MIN..CONVOY_MAX).contains(&d)
                && let Some(tile) = loaded(*p)
            {
                candidates.push((
                    *p,
                    route[i + 1] - *p,
                    route[i..].to_vec(),
                    edge,
                    toward,
                    tile,
                ));
            }
        }
    }
    if candidates.is_empty() {
        debug!("heat: no road {CONVOY_MIN}–{CONVOY_MAX} m from Rico leads to him");
        return None;
    }
    let k = (r * candidates.len() as f32) as usize % candidates.len();
    let (at, dir, rest, edge, toward, tile) = candidates.swap_remove(k);
    debug!(
        "heat: road from {at:?} on edge {edge} to node {toward} at {:?} ({} m on, {:.0} m beyond), Rico's node {goal} at {:?}, route ends {:?}",
        crate::coords::to_bevy(net.graph.nodes[toward].pos),
        net.graph.edges[edge].length,
        ways[&toward].0,
        crate::coords::to_bevy(net.graph.nodes[goal].pos),
        rest.last()
    );
    let mut driver = AiDriver::new(rest, edge, toward, CONVOY_CRUISE, (r * 1e6) as u32);
    driver.target = Some(rico);
    driver.stop_at_end = true;
    // the way from there, junction by junction, to the road node nearest Rico (where it
    // stops if it has not come near him before)
    let mut n = toward;
    while let Some((_, Some(next))) = ways.get(&n) {
        driver.plan.push_back(*next);
        n = net.graph.other_end(*next, n);
    }
    Some((at, Quat::from_rotation_y(yaw_toward(dir)), driver, tile))
}

/// Ask the fleet for the heat's vehicles: a car of soldiers on a road from level 2, an
/// attack helicopter from level 3, one of each at a time.
#[allow(clippy::too_many_arguments)]
fn send_vehicles(
    time: Res<Time>,
    mut heat: ResMut<Heat>,
    tables: Res<HeatTables>,
    game: Option<Res<Game>>,
    mut hv: ResMut<HeatVehicles>,
    fleet: Option<ResMut<Fleet>>,
    traffic: Option<ResMut<Traffic>>,
    streamer: Option<Res<Streamer>>,
    heroes: Query<&Transform, With<Hero>>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
    live: Query<&HeatVehicle>,
) {
    let (Some(mut fleet), Some(game)) = (fleet, game) else {
        return;
    };
    let Ok(rico) = heroes.single().map(|t| t.translation) else {
        return;
    };
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    hv.pending.retain(|p| now - p.2 < 30.0);
    hv.convoy_timer -= dt;
    hv.gunship_timer -= dt;
    let level = heat.level();
    if level < CONVOY_LEVEL {
        return;
    }
    let (car, heli) = band_units(tables.0.as_ref(), heat.value, &game);
    let loaded = |p: Vec3| {
        let en = to_engine(p);
        let tile = tile_index(en[0], en[2])?;
        match &streamer {
            Some(st) => st.loaded.contains_key(&tile).then_some(tile),
            None => Some(tile),
        }
    };
    heat.seed = heat
        .seed
        .wrapping_mul(1_664_525)
        .wrapping_add(1_013_904_223);
    let r = crate::npc::unit_rand(heat.seed);
    // a car of soldiers on the roads
    let driving = live
        .iter()
        .any(|v| matches!(v, HeatVehicle::Convoy { seats } if *seats > 0))
        || hv
            .pending
            .iter()
            .any(|p| matches!(p.1, Asked::Convoy { .. }));
    if !driving
        && hv.convoy_timer <= 0.0
        && let (Some((container, seats)), Some(mut traffic)) = (car, traffic)
        && let Some(net) = traffic.net.as_mut()
    {
        hv.convoy_timer = VEHICLE_INTERVAL;
        if let Some((at, rot, driver, tile)) = road_spot(net, rico, &loaded, r) {
            info!("heat {level}: a {container} with {seats} soldiers from {at:?}");
            fleet.requests.push(Request {
                container,
                place: Place::At(Transform::from_translation(at).with_rotation(rot)),
                tile: Some(tile),
                drive: false,
                crew: None,
            });
            hv.pending.push((
                at,
                Asked::Convoy {
                    driver,
                    seats: seats.min(MAX_SEATS),
                },
                now,
            ));
            hv.convoys += 1;
        }
    }
    // an attack helicopter
    let flying = live.iter().any(|v| *v == HeatVehicle::Gunship)
        || hv.pending.iter().any(|p| matches!(p.1, Asked::Gunship));
    if level >= GUNSHIP_LEVEL
        && !flying
        && hv.gunship_timer <= 0.0
        && let Some(container) = heli
    {
        hv.gunship_timer = VEHICLE_INTERVAL;
        // behind the camera, give or take 60°
        let back = cams
            .single()
            .map(|c| yaw_toward(-c.forward().as_vec3()))
            .unwrap_or(r * std::f32::consts::TAU);
        let yaw = back + (r - 0.5) * std::f32::consts::FRAC_PI_3 * 2.0;
        let at = rico + crate::coords::facing_dir(yaw) * GUNSHIP_SPAWN;
        if let Some(tile) = loaded(at) {
            info!("heat {level}: a {container} from {at:?}");
            fleet.requests.push(Request {
                container,
                place: Place::At(
                    Transform::from_translation(at)
                        .with_rotation(Quat::from_rotation_y(yaw_toward(rico - at))),
                ),
                tile: Some(tile),
                drive: false,
                crew: None,
            });
            hv.pending.push((at, Asked::Gunship, now));
            hv.gunships += 1;
        }
    }
}

/// The fleet built one: the car gets its driver (the traffic AI, headed for Rico) and
/// soldiers, the helicopter its pilot, lifted into the air.
#[allow(clippy::type_complexity)]
fn adopt_vehicles(
    mut commands: Commands,
    mut hv: ResMut<HeatVehicles>,
    cars: Query<(Entity, &Transform), Added<Vehicle>>,
    helis: Query<(Entity, &Transform), Added<Heli>>,
) {
    if hv.pending.is_empty() {
        return;
    }
    let near = |a: Vec3, b: Vec3| Vec2::new(a.x - b.x, a.z - b.z).length() < 1.5;
    for (e, tf) in &cars {
        if let Some(i) = hv
            .pending
            .iter()
            .position(|p| matches!(p.1, Asked::Convoy { .. }) && near(p.0, tf.translation))
            && let (_, Asked::Convoy { driver, seats }, _) = hv.pending.swap_remove(i)
        {
            info!(
                "heat: the car {e:?} sets off at {:?}, {} junctions planned",
                tf.translation,
                driver.plan.len()
            );
            commands.entity(e).insert((
                driver,
                AiDriven,
                CrewRequest(CrewKind::Soldier),
                HeatVehicle::Convoy { seats },
            ));
        }
    }
    for (e, tf) in &helis {
        if let Some(i) = hv
            .pending
            .iter()
            .position(|p| matches!(p.1, Asked::Gunship) && near(p.0, tf.translation))
        {
            hv.pending.swap_remove(i);
            let up = tf.translation + Vec3::Y * GUNSHIP_START_HEIGHT;
            info!("heat: the helicopter {e:?} takes off from {up:?}");
            commands.entity(e).insert((
                Transform::from_translation(up).with_rotation(tf.rotation),
                Position(up),
                HeliPilot::hunting(GUNSHIP_RADIUS, GUNSHIP_ALTITUDE),
                AiDriven,
                CrewRequest(CrewKind::Soldier),
                HeatVehicle::Gunship,
            ));
        }
    }
}

/// The car drives at Rico; near him (or stuck, or at the end of its road) it stops and
/// its soldiers get out. The helicopter stops shooting once the heat is gone.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn convoys(
    mut commands: Commands,
    mut heat: ResMut<Heat>,
    roster: Res<EnemyRoster>,
    mut npcs: ResMut<Npcs>,
    mut fleet: Option<ResMut<Fleet>>,
    heroes: Query<&Transform, With<Hero>>,
    mut cars: Query<
        (
            Entity,
            &mut HeatVehicle,
            &Transform,
            Option<&mut AiDriver>,
            Option<&mut Vehicle>,
            Option<&Crew>,
            Has<Occupied>,
        ),
        Without<Hero>,
    >,
    mut gunships: Query<&mut HeliPilot, (With<HeatVehicle>, Without<Hero>)>,
) {
    let calm = heat.level() == 0;
    for mut p in &mut gunships {
        p.hunt = !calm;
    }
    let Ok(rico) = heroes.single().map(|t| t.translation) else {
        return;
    };
    for (e, mut hv, tf, ai, car, crew, occupied) in &mut cars {
        let HeatVehicle::Convoy { seats } = *hv else {
            continue;
        };
        if seats == 0 {
            continue;
        }
        let Some(mut ai) = ai else {
            // taken (Rico hijacked it): nobody gets out
            *hv = HeatVehicle::Convoy { seats: 0 };
            continue;
        };
        if occupied {
            *hv = HeatVehicle::Convoy { seats: 0 };
            continue;
        }
        ai.target = Some(rico);
        let close = Vec2::new(tf.translation.x - rico.x, tf.translation.z - rico.z).length();
        if close > CONVOY_LOST {
            // lost on the roads (or Rico moved on): it goes, another comes
            if let Some(f) = fleet.as_mut() {
                f.spawned.retain(|(s, _)| *s != e);
            }
            commands.entity(e).despawn();
            info!("heat: the car {e:?} lost its way {close:.0} m from Rico");
            continue;
        }
        if close > DISMOUNT && ai.since_progress < CONVOY_STUCK && ai.route_left() > CONVOY_ROAD_END
        {
            continue;
        }
        // stop and get out
        commands.entity(e).remove::<(AiDriver, AiDriven, Crew)>();
        if let Some(d) = crew.and_then(|c| c.driver) {
            commands.entity(d).despawn();
        }
        if let Some(mut car) = car {
            car.throttle = 0.0;
            car.brake = 1.0;
        }
        let right = tf.rotation * crate::coords::MODEL_RIGHT;
        let back = -crate::coords::forward_of(tf.rotation);
        for k in 0..seats {
            let side = if k % 2 == 0 { -2.5 } else { 2.5 };
            let at = tf.translation + right * side + back * (k / 2) as f32 * 1.5;
            let en = to_engine(at);
            let Some(tile) = tile_index(en[0], en[2]) else {
                continue;
            };
            heat.seed = heat
                .seed
                .wrapping_mul(1_664_525)
                .wrapping_add(1_013_904_223);
            npcs.reinforce(&roster, at, rico, tile, heat.seed);
        }
        info!(
            "heat: {seats} soldiers out of their car {close:.0} m from Rico at {:?}",
            tf.translation
        );
        *hv = HeatVehicle::Convoy { seats: 0 };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_rise_and_cool() {
        let mut h = Heat::default();
        assert_eq!(h.level(), 0);
        h.add(KILL_HEAT);
        assert_eq!(h.level(), 1);
        for _ in 0..3 {
            h.add(KILL_HEAT);
        }
        assert_eq!(h.level(), 3);
        // seen: it holds
        for _ in 0..600 {
            h.tick(0.1, true);
        }
        assert_eq!(h.level(), 3);
        // out of sight: it holds for the cooldown, then falls to nothing
        for _ in 0..(COOLDOWN * 10.0) as usize {
            h.tick(0.1, false);
        }
        assert_eq!(h.level(), 3);
        for _ in 0..400 {
            h.tick(0.1, false);
        }
        assert_eq!(h.level(), 0);
    }

    /// Headless: a soldier falls and the heat rises; with nobody seeing Rico it cools
    /// back to nothing.
    #[test]
    fn a_kill_raises_heat_and_it_decays() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(0.25),
            ))
            .init_resource::<Npcs>()
            .init_resource::<EnemyRoster>()
            .add_plugins(HeatPlugin);
        let soldier = app
            .world_mut()
            .spawn((Transform::default(), Npc::guard(Vec3::ZERO, 0.0)))
            .id();
        app.update();
        assert_eq!(app.world().resource::<Heat>().level(), 0);
        app.world_mut().entity_mut(soldier).insert(Dead(0.0));
        app.update();
        let h = app.world().resource::<Heat>();
        assert_eq!((h.level(), h.value), (1, KILL_HEAT));
        // a minute and a half with no one looking
        for _ in 0..(90.0 / 0.25) as usize {
            app.update();
        }
        assert_eq!(app.world().resource::<Heat>().level(), 0);
    }

    /// Heat 2 with Rico at the end of a straight road: a government car comes along it
    /// towards him and its soldiers get out near him. Heat 3: an attack helicopter comes
    /// and circles him 30 m up, firing. Needs JC2_TEST_DIR.
    #[test]
    fn heat_sends_a_car_along_the_road_and_a_helicopter() {
        use crate::fleet::testing;
        let Some((mut app, ground, spawn)) = testing::app(
            &[
                FALLBACK_CAR,
                FALLBACK_HELI,
                "pd_gov_base01.ee",
                "heatspawn.bin",
            ],
            |app| {
                app.init_resource::<Npcs>()
                    .init_resource::<EnemyRoster>()
                    .add_plugins((HeatPlugin, crate::heli::HeliPlugin, crate::crew::CrewPlugin))
                    .add_systems(FixedUpdate, crate::vehicle::vehicle_physics)
                    .add_systems(
                        Update,
                        (crate::traffic::hand_over, crate::traffic::ai_drive).chain(),
                    );
            },
        ) else {
            return;
        };
        // a road along the tile, Rico near its far end
        let a = Vec3::new(spawn.x, ground, spawn.z + 220.0);
        let b = Vec3::new(spawn.x, ground, spawn.z - 230.0);
        let net = crate::traffic::tests::straight_road(a, b);
        app.insert_resource(Traffic::with_network(net));
        let rico = Vec3::new(spawn.x + 6.0, ground + 0.9, spawn.z - 190.0);
        app.world_mut()
            .spawn((Hero::default(), Transform::from_translation(rico)));
        let set_heat = |app: &mut App, v: f32| {
            let mut h = app.world_mut().resource_mut::<Heat>();
            h.value = v;
            h.since_contact = 0.0;
        };
        // level 2: a car
        let mut car = None;
        testing::wait_until(&mut app, "the heat's car", |app| {
            set_heat(app, 30.0);
            let mut q = app
                .world_mut()
                .query::<(Entity, &HeatVehicle, Has<AiDriver>)>();
            car = q
                .iter(app.world())
                .find(|(_, v, ai)| matches!(v, HeatVehicle::Convoy { .. }) && *ai)
                .map(|(e, ..)| e);
            car.is_some()
        });
        let car = car.unwrap();
        let start = app.world().get::<Transform>(car).unwrap().translation;
        let d0 = start.distance(rico);
        assert!(
            (CONVOY_MIN - 5.0..CONVOY_MAX + 5.0).contains(&d0),
            "asked for {d0:.0} m away"
        );
        for _ in 0..3 {
            app.update();
        }
        assert!(
            app.world().get::<Crew>(car).is_some(),
            "no soldier at the wheel"
        );
        let mut out = false;
        for _ in 0..64 * 40 {
            set_heat(&mut app, 30.0);
            app.update();
            if app.world().get::<HeatVehicle>(car) == Some(&HeatVehicle::Convoy { seats: 0 }) {
                out = true;
                break;
            }
        }
        let at = app.world().get::<Transform>(car).unwrap().translation;
        let d1 = Vec2::new(at.x - rico.x, at.z - rico.z).length();
        eprintln!("heat car: from {d0:.0} m to {d1:.0} m of Rico, soldiers out: {out}");
        assert!(out, "nobody got out");
        assert!(d1 < DISMOUNT + 15.0, "stopped {d1:.0} m away");
        assert!(app.world().get::<AiDriver>(car).is_none());
        assert!(app.world().resource::<Npcs>().reinforcements_waiting() >= 2);
        // level 3: a helicopter that circles him
        let mut heli = None;
        testing::wait_until(&mut app, "the heat's helicopter", |app| {
            set_heat(app, 50.0);
            let mut q = app.world_mut().query::<(Entity, &HeatVehicle)>();
            heli = q
                .iter(app.world())
                .find(|(_, v)| **v == HeatVehicle::Gunship)
                .map(|(e, _)| e);
            heli.is_some()
        });
        let heli = heli.unwrap();
        let mut ring = (f32::MAX, f32::MIN);
        let mut height = (f32::MAX, f32::MIN);
        for i in 0..64 * 40 {
            set_heat(&mut app, 50.0);
            app.update();
            if i > 64 * 25 {
                let p = app.world().get::<Transform>(heli).unwrap().translation;
                let r = Vec2::new(p.x - rico.x, p.z - rico.z).length();
                ring = (ring.0.min(r), ring.1.max(r));
                height = (height.0.min(p.y - ground), height.1.max(p.y - ground));
            }
        }
        let shots = app.world().get::<HeliPilot>(heli).unwrap().shots;
        eprintln!(
            "heat helicopter: {:.0}–{:.0} m from Rico, {:.0}–{:.0} m up, {shots} shots",
            ring.0, ring.1, height.0, height.1
        );
        assert!(ring.0 > 30.0 && ring.1 < 70.0, "not circling him: {ring:?}");
        assert!(height.0 > 15.0 && height.1 < 50.0, "height {height:?}");
        assert!(shots > 0, "it never fired");
    }

    /// Headless, with `aiheatparams.bin` and `heatsystem.bin`: a soldier killed beside
    /// another raises the heat by the data's kill (100 points, one level) once its 0.5 s
    /// delay is over; a kill nobody is near (beyond its 40 m) adds nothing; at chaos 0 the
    /// heat stops at level 2's top (`heat_level_max_heat_2`).
    #[test]
    fn heat_from_the_data() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = std::sync::Arc::new(jc2_formats::GameFs::open(dir).unwrap());
        if !fs.contains("aiheatparams.bin") || !fs.contains("heatsystem.bin") {
            return;
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(0.25),
            ))
            .insert_resource(Game { fs })
            .init_resource::<Npcs>()
            .init_resource::<EnemyRoster>()
            .add_plugins(HeatPlugin);
        let guard = |app: &mut App, x: f32| {
            app.world_mut()
                .spawn((
                    Transform::from_xyz(x, 0.0, 0.0),
                    Npc::guard(Vec3::new(x, 0.0, 0.0), 0.0),
                ))
                .id()
        };
        let a = guard(&mut app, 0.0);
        let _b = guard(&mut app, 10.0);
        let far = guard(&mut app, 500.0);
        app.update();
        let tables = app.world().resource::<HeatTables>();
        let kill = tables.1.as_ref().expect("aiheatparams.bin").kill().unwrap();
        let amount = tables.heat_of(kill);
        assert_eq!(amount, 20.0);
        // the cap at chaos 0, as chaos.rs sets it
        let cap = tables.2.cap(0.0);
        app.world_mut().resource_mut::<Heat>().cap = cap;
        // the lone soldier far away: nobody notices
        app.world_mut().entity_mut(far).insert(Dead(0.0));
        for _ in 0..4 {
            app.update();
        }
        assert_eq!(app.world().resource::<Heat>().value, 0.0);
        // one beside another: after the delay, one level
        app.world_mut().entity_mut(a).insert(Dead(0.0));
        app.update();
        assert_eq!(app.world().resource::<Heat>().value, 0.0, "the delay");
        for _ in 0..3 {
            app.update();
        }
        let h = app.world().resource::<Heat>();
        assert_eq!((h.value, h.level()), (amount, 1));
        // more than the cap allows: level 2 at most
        for _ in 0..3 {
            app.world_mut().resource_mut::<Heat>().add(amount);
        }
        let h = app.world().resource::<Heat>();
        assert_eq!((h.value, h.level()), (40.0, 2));
    }
}
