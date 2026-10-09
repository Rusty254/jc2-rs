//! Road traffic: civilian cars driving the island's roads around Rico.
//!
//! The roads are the AI's road graph `ai\default.rg` with each edge shaped by its road
//! spline (`jc2_formats::roads`: the 33 road containers at startup, the tiles' own as
//! they stream in). A few cars (`--traffic N`, 5 by default) are kept on the roads
//! 80–250 m from Rico, of the kinds `worldsim.bin` weighs for the ground's game zone
//! (`lave.vehicle_types`; two-wheelers are left out, the raycast car needs four). Each
//! is an ordinary vehicle from `fleet.rs` with an [`AiDriver`]: it keeps to the right-hand
//! lane (2 m right of the spline, worked out in engine axes), steers for a point ahead
//! on it, cruises at 40–60 km/h, slows for bends and stops for what is in front; at a
//! junction it takes a random other road. `E` takes one over like any car: the AI lets go
//! once someone is in it. Cars more than 600 m from Rico go, and so do cars that have made
//! no headway for a while (a dead end, wedged against a wall, waiting behind a wreck).
//! See `research/formats/civilians.md`.

use crate::Game;
use crate::civilians::{TileSim, TileSimPlugin};
use crate::coords::{forward_of, to_bevy, to_engine, yaw_of, yaw_toward};
use crate::fleet::{Fleet, Place, Request};
use crate::vehicle::{AiDriven, Occupied, Vehicle};
use avian3d::prelude::*;
use bevy::prelude::*;
use jc2_formats::civilians::{GameZones, SpawnType, WorldSim, pick};
use jc2_formats::roads::{
    ROAD_CONTAINERS, RoadGraph, RoadNetwork, container_placements, offset_right, road_splines,
};
use jc2_formats::terrain::{TerrainTile, tile_file_name, tile_index, tile_origin};
use std::collections::{HashSet, VecDeque};

/// Cars kept on the roads around Rico unless `--traffic` says otherwise.
pub const DEFAULT_CARS: usize = 5;
/// The lane's centre, right of the road's centre line (m).
pub const LANE_OFFSET: f32 = 2.0;
/// Cruising speeds (m/s): 40–60 km/h.
const CRUISE_MIN: f32 = 40.0 / 3.6;
const CRUISE_MAX: f32 = 60.0 / 3.6;
/// Sideways acceleration the AI takes bends at (m/s²).
const BEND_ACCEL: f32 = 3.0;
/// New cars appear this far from Rico (m), on a road.
const SPAWN_MIN: f32 = 80.0;
const SPAWN_MAX: f32 = 250.0;
/// Cars this far from Rico go (m).
pub const DESPAWN: f32 = 600.0;
/// Look this far ahead for something to stop for (m, plus the stopping distance).
const CLEAR_AHEAD: f32 = 6.0;
/// No new car closer than this to another (m).
const SPACING: f32 = 40.0;
/// A car that has not passed a point of its route (they are at most 10 m apart) for this
/// long is stuck or lost and goes (s).
pub const GIVE_UP: f32 = 15.0;
/// Seconds between spawn attempts.
const SPAWN_INTERVAL: f32 = 1.0;
/// Fallback kinds when `worldsim.bin` can't be read.
const FALLBACK_CARS: [&str; 3] = [
    "lave.v001_sedan.ee",
    "lave.v002_sedan.ee",
    "lave.v004_truck_regular.ee",
];

#[derive(Resource, Clone, Copy, Debug)]
pub struct TrafficConfig {
    pub cars: usize,
}

impl Default for TrafficConfig {
    fn default() -> Self {
        Self { cars: DEFAULT_CARS }
    }
}

/// The road network and the spawning state.
#[derive(Resource, Default)]
pub struct Traffic {
    pub net: Option<RoadNetwork>,
    cars: Vec<SpawnType>,
    zones: Option<GameZones>,
    tiles_added: HashSet<(u32, u32)>,
    /// Cars asked of the fleet and not on the road yet: where, the route, when asked.
    pending: Vec<(Vec3, AiDriver, f32)>,
    timer: f32,
    seed: u32,
}

impl Traffic {
    #[cfg(test)]
    pub fn with_network(net: RoadNetwork) -> Self {
        Self {
            net: Some(net),
            seed: 0x1234_5678,
            ..Default::default()
        }
    }

    fn rand(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        crate::npc::unit_rand(self.seed)
    }
}

/// The traffic AI of one car.
#[derive(Component, Clone, Debug)]
pub struct AiDriver {
    /// The lane ahead (Bevy axes), from about where the car is.
    pub route: VecDeque<Vec3>,
    /// The edge it is on and the node it is driving to.
    pub edge: usize,
    pub toward: usize,
    /// Its cruising speed (m/s).
    pub cruise: f32,
    /// The speed it wants now, for the HUD and tests.
    pub target_speed: f32,
    /// Seconds since it last passed a point of its route; past `GIVE_UP` it is removed.
    pub since_progress: f32,
    /// Where it is headed (the heat's vehicles: Rico): at a junction it takes the road
    /// that ends nearest to it instead of a random one.
    pub target: Option<Vec3>,
    /// The roads (edges) to take at the next junctions, in order, when it follows a
    /// planned path (`heat.rs`); empty: `target` or chance decide.
    pub plan: VecDeque<usize>,
    /// Stop where the plan runs out instead of driving on (the heat's cars stop at the
    /// road node nearest Rico).
    pub stop_at_end: bool,
    seed: u32,
}

impl AiDriver {
    pub fn new(route: Vec<Vec3>, edge: usize, toward: usize, cruise: f32, seed: u32) -> Self {
        Self {
            route: route.into(),
            edge,
            toward,
            cruise,
            target_speed: cruise,
            since_progress: 0.0,
            target: None,
            plan: VecDeque::new(),
            stop_at_end: false,
            seed,
        }
    }

    /// What is left of the route (m).
    pub fn route_left(&self) -> f32 {
        self.remaining()
    }

    fn rand(&mut self) -> f32 {
        self.seed = self
            .seed
            .wrapping_mul(1_664_525)
            .wrapping_add(1_013_904_223);
        crate::npc::unit_rand(self.seed)
    }

    fn remaining(&self) -> f32 {
        self.route
            .iter()
            .zip(self.route.iter().skip(1))
            .map(|(a, b)| a.distance(*b))
            .sum()
    }

    /// Drop the points behind `pos`, keeping the segment it is on.
    fn advance(&mut self, pos: Vec3) {
        while self.route.len() > 2 {
            let (a, b) = (self.route[0], self.route[1]);
            let ab = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
            let ap = Vec3::new(pos.x - a.x, 0.0, pos.z - a.z);
            if ab.length_squared() < 1e-6 || ap.dot(ab) >= ab.length_squared() {
                self.route.pop_front();
                self.since_progress = 0.0;
            } else {
                break;
            }
        }
    }

    /// The point `ahead` metres along the route from the car's projection onto it.
    fn look_ahead(&self, pos: Vec3, ahead: f32) -> Option<Vec3> {
        let (a, b) = (*self.route.front()?, *self.route.get(1)?);
        let ab = b - a;
        let t = ((pos - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let mut at = a + ab * t;
        let mut left = ahead;
        let mut next = 1;
        while let Some(p) = self.route.get(next) {
            let d = at.distance(*p);
            if d >= left {
                return Some(at + (*p - at) * (left / d.max(1e-6)));
            }
            left -= d;
            at = *p;
            next += 1;
        }
        Some(at)
    }

    /// The sharpest turn (radians per metre) over the next `span` metres.
    fn curvature_ahead(&self, span: f32) -> f32 {
        let mut worst: f32 = 0.0;
        let mut travelled = 0.0;
        let pts: Vec<Vec3> = self.route.iter().copied().collect();
        for w in pts.windows(3) {
            let d0 = Vec3::new(w[1].x - w[0].x, 0.0, w[1].z - w[0].z);
            let d1 = Vec3::new(w[2].x - w[1].x, 0.0, w[2].z - w[1].z);
            let (l0, l1) = (d0.length(), d1.length());
            if l0 > 0.1 && l1 > 0.1 {
                let angle = d0.angle_between(d1);
                worst = worst.max(angle / ((l0 + l1) * 0.5).max(4.0));
            }
            travelled += l0;
            if travelled > span {
                break;
            }
        }
        worst
    }
}

pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<TileSimPlugin>() {
            app.add_plugins(TileSimPlugin);
        }
        app.init_resource::<Traffic>()
            .init_resource::<TrafficConfig>()
            .add_systems(Startup, load_roads)
            .add_systems(
                Update,
                (
                    add_tile_roads,
                    spawn_traffic,
                    adopt,
                    hand_over,
                    despawn_far,
                    ai_drive,
                    report,
                )
                    .chain(),
            );
    }
}

/// The road graph and the road containers' splines; the car kinds from `worldsim.bin`.
fn load_roads(mut traffic: ResMut<Traffic>, game: Res<Game>) {
    traffic.seed = 0x2545_f491;
    let graph = match game
        .fs
        .read_inflated("default.rg")
        .map_err(|e| e.to_string())
        .and_then(|d| RoadGraph::parse(&d).map_err(|e| e.to_string()))
    {
        Ok(g) => g,
        Err(e) => {
            warn!("traffic: no road graph ({e})");
            return;
        }
    };
    let mut net = RoadNetwork::new(graph);
    let mut splines = 0;
    for c in ROAD_CONTAINERS {
        if let Ok(f) = container_placements(&game.fs, c) {
            splines += net.add_splines(road_splines(&f));
        }
    }
    info!(
        "traffic: {} road nodes, {} edges, {splines} road splines",
        net.graph.nodes.len(),
        net.graph.edges.len()
    );
    traffic.net = Some(net);
    if let Some(ws) = game
        .fs
        .read_inflated("worldsim.bin")
        .ok()
        .and_then(|d| WorldSim::parse(&d).ok())
    {
        traffic.cars = ws
            .cars
            .into_iter()
            .filter(|c| four_wheels(&c.name) && game.fs.contains(&c.name.to_ascii_lowercase()))
            .collect();
    }
    traffic.zones = game
        .fs
        .read_inflated("gamezone.bin")
        .ok()
        .and_then(|d| GameZones::parse(&d).ok());
}

/// Not a motorbike or scooter (the raycast car tips over on two wheels).
fn four_wheels(container: &str) -> bool {
    let c = container.to_ascii_lowercase();
    !["moped", "vespa", "_mc", "v007_"]
        .iter()
        .any(|k| c.contains(k))
}

/// The road splines of tiles that streamed in.
fn add_tile_roads(mut traffic: ResMut<Traffic>, sim: Option<Res<TileSim>>) {
    let Some(sim) = sim else { return };
    let fresh: Vec<(u32, u32)> = sim
        .tiles
        .keys()
        .filter(|k| !traffic.tiles_added.contains(*k))
        .copied()
        .collect();
    for key in fresh {
        traffic.tiles_added.insert(key);
        let roads = sim.tiles[&key].roads.clone();
        if let Some(net) = traffic.net.as_mut()
            && !roads.is_empty()
        {
            net.add_splines(roads);
        }
    }
}

/// The lane of `edge` driven towards `toward`, in Bevy axes; `None` when the edge's
/// shape is not known yet.
pub fn lane(net: &mut RoadNetwork, edge: usize, toward: usize) -> Option<Vec<Vec3>> {
    let mut shape = net.shape(edge)?;
    if net.graph.edges[edge].from as usize == toward {
        shape.reverse();
    }
    // points every 10 m or closer, so a car can start anywhere along it
    let lane = offset_right(&shape, LANE_OFFSET);
    let mut out: Vec<Vec3> = Vec::with_capacity(lane.len());
    for w in lane.windows(2) {
        let (a, b) = (to_bevy(w[0]), to_bevy(w[1]));
        let n = (a.distance(b) / 10.0).ceil().max(1.0) as usize;
        for k in 0..n {
            out.push(a.lerp(b, k as f32 / n as f32));
        }
    }
    out.extend(lane.last().map(|p| to_bevy(*p)));
    Some(out)
}

/// Where Rico is (or the camera, without him).
fn focus(
    heroes: &Query<&Transform, With<crate::character::Hero>>,
    cams: &Query<
        &Transform,
        (
            With<crate::world::FlyCamera>,
            Without<crate::character::Hero>,
        ),
    >,
) -> Option<Vec3> {
    heroes
        .single()
        .or_else(|_| cams.single())
        .ok()
        .map(|t| t.translation)
}

/// Keep `TrafficConfig::cars` on the roads: ask the fleet for one at a time, on a road
/// 80–250 m away whose tile has loaded.
#[allow(clippy::too_many_arguments)]
fn spawn_traffic(
    time: Res<Time>,
    config: Res<TrafficConfig>,
    mut traffic: ResMut<Traffic>,
    mut fleet: ResMut<Fleet>,
    game: Res<Game>,
    streamer: Option<Res<crate::world::Streamer>>,
    drivers: Query<&Transform, With<AiDriver>>,
    heroes: Query<&Transform, With<crate::character::Hero>>,
    cams: Query<
        &Transform,
        (
            With<crate::world::FlyCamera>,
            Without<crate::character::Hero>,
        ),
    >,
) {
    let now = time.elapsed_secs();
    traffic.pending.retain(|p| now - p.2 < 30.0);
    traffic.timer -= time.delta_secs();
    if traffic.timer > 0.0 || drivers.iter().count() + traffic.pending.len() >= config.cars {
        return;
    }
    traffic.timer = SPAWN_INTERVAL;
    let (Some(rico), Some(st)) = (focus(&heroes, &cams), streamer) else {
        return;
    };
    let e = to_engine(rico);
    let taken: Vec<Vec3> = drivers
        .iter()
        .map(|t| t.translation)
        .chain(traffic.pending.iter().map(|p| p.0))
        .collect();
    let r1 = traffic.rand();
    let r2 = traffic.rand();
    let r3 = traffic.rand();
    let r4 = traffic.rand();
    let Some(net) = traffic.net.as_mut() else {
        return;
    };
    // a road near enough, far enough
    let mut edges: Vec<usize> = net
        .graph
        .nodes_near(e[0], e[2], SPAWN_MAX + 200.0)
        .into_iter()
        .flat_map(|n| net.graph.adjacency[n].clone())
        .collect();
    edges.sort();
    edges.dedup();
    if edges.is_empty() {
        return;
    }
    // every point on those lanes at the right distance, on a loaded tile, clear of the
    // other cars; one of them at random
    let mut candidates: Vec<(Vec3, Vec3, Vec<Vec3>, usize, usize)> = Vec::new();
    for (k, &edge) in edges.iter().enumerate() {
        let ends = (
            net.graph.edges[edge].from as usize,
            net.graph.edges[edge].to as usize,
        );
        let toward = if (k as f32 * 0.618 + r2).fract() < 0.5 {
            ends.0
        } else {
            ends.1
        };
        let Some(route) = lane(net, edge, toward) else {
            continue;
        };
        for (i, p) in route.iter().enumerate() {
            let d = Vec2::new(p.x - rico.x, p.z - rico.z).length();
            let en = to_engine(*p);
            if i + 1 < route.len()
                && (SPAWN_MIN..SPAWN_MAX).contains(&d)
                && tile_index(en[0], en[2]).is_some_and(|t| st.loaded.contains_key(&t))
                && taken.iter().all(|o| o.distance(*p) > SPACING)
            {
                candidates.push((*p, route[i + 1] - *p, route[i..].to_vec(), edge, toward));
            }
        }
    }
    if candidates.is_empty() {
        return;
    }
    let pick_at = (r1 * candidates.len() as f32) as usize % candidates.len();
    let (at, dir, rest, edge, toward) = candidates.swap_remove(pick_at);
    let rot = Quat::from_rotation_y(yaw_toward(dir));
    // what kind: by the ground's game zone there
    let en = to_engine(at);
    let Some(tile) = tile_index(en[0], en[2]) else {
        return;
    };
    let zone = game
        .fs
        .read_inflated(&tile_file_name(tile.0, tile.1))
        .ok()
        .and_then(|d| TerrainTile::parse(&d).ok())
        .map(|t| {
            let (x0, z0) = tile_origin(tile.0, tile.1);
            t.material_local(en[0] - x0, en[2] - z0)
        })
        .and_then(|m| traffic.zones.as_ref().map(|z| z.of_material(m).to_string()))
        .unwrap_or_else(|| "default".into());
    let container = pick(&traffic.cars, &zone, r3)
        .map(|t| t.name.to_ascii_lowercase())
        .unwrap_or_else(|| FALLBACK_CARS[(r3 * 3.0) as usize % 3].to_string());
    let cruise = CRUISE_MIN + (CRUISE_MAX - CRUISE_MIN) * r4;
    let seed = traffic.seed;
    debug!("traffic: a {container} on edge {edge} at {at:?} ({zone})");
    fleet.requests.push(Request {
        container,
        place: Place::At(Transform::from_translation(at).with_rotation(rot)),
        tile: Some(tile),
        drive: false,
        // a villager at the wheel (`crew.rs`; `E` throws him out)
        crew: Some(crate::crew::CrewKind::Civilian),
    });
    traffic
        .pending
        .push((at, AiDriver::new(rest, edge, toward, cruise, seed), now));
}

/// A car the fleet just built where traffic asked for one gets its driver.
fn adopt(
    mut commands: Commands,
    mut traffic: ResMut<Traffic>,
    fresh: Query<(Entity, &Transform), Added<Vehicle>>,
) {
    for (e, tf) in &fresh {
        let p = tf.translation;
        if let Some(i) = traffic
            .pending
            .iter()
            .position(|(at, _, _)| Vec2::new(at.x - p.x, at.z - p.z).length() < 1.5)
        {
            let (_, driver, _) = traffic.pending.swap_remove(i);
            commands.entity(e).insert((driver, AiDriven));
        }
    }
}

/// Someone got in: the AI lets go and the car is theirs.
#[allow(clippy::type_complexity)]
pub(crate) fn hand_over(
    mut commands: Commands,
    mut taken: Query<(Entity, &mut Vehicle), (With<AiDriver>, With<Occupied>)>,
) {
    for (e, mut car) in &mut taken {
        car.throttle = 0.0;
        car.brake = 0.0;
        car.steer_input = 0.0;
        commands.entity(e).remove::<(AiDriver, AiDriven)>();
        info!("traffic: a car taken over");
    }
}

/// Cars that fell far behind go.
#[allow(clippy::type_complexity)]
fn despawn_far(
    mut commands: Commands,
    mut fleet: ResMut<Fleet>,
    // (the heat's cars are its own business: `heat.rs` stops them)
    cars: Query<
        (Entity, &Transform, &AiDriver),
        (Without<Occupied>, Without<crate::heat::HeatVehicle>),
    >,
    heroes: Query<&Transform, With<crate::character::Hero>>,
    cams: Query<
        &Transform,
        (
            With<crate::world::FlyCamera>,
            Without<crate::character::Hero>,
        ),
    >,
) {
    let Some(rico) = focus(&heroes, &cams) else {
        return;
    };
    for (e, tf, ai) in &cars {
        if tf.translation.distance(rico) > DESPAWN || ai.since_progress > GIVE_UP {
            debug!(
                "traffic: {e} goes ({:.0} s without headway)",
                ai.since_progress
            );
            fleet.spawned.retain(|(s, _)| *s != e);
            commands.entity(e).despawn();
        }
    }
}

/// Now and then, how the traffic is doing (debug log).
fn report(time: Res<Time>, mut next: Local<f32>, cars: Query<(&Vehicle, &AiDriver, &Transform)>) {
    if time.elapsed_secs() < *next {
        return;
    }
    *next = time.elapsed_secs() + 10.0;
    for (v, ai, tf) in &cars {
        debug!(
            "traffic: {} at {:?}, {:.0} km/h (wants {:.0}), edge {}",
            v.spec.name,
            tf.translation,
            v.speed * 3.6,
            ai.target_speed * 3.6,
            ai.edge
        );
    }
}

/// Follow the lane: steer for a point ahead, hold the cruise, slow for bends and for
/// what is in front, take another road at the end of this one.
pub(crate) fn ai_drive(
    time: Res<Time>,
    mut traffic: Option<ResMut<Traffic>>,
    spatial: SpatialQuery,
    colliders: Query<&ColliderOf>,
    bodies: Query<&RigidBody>,
    mut cars: Query<(Entity, &mut AiDriver, &mut Vehicle, &Transform), Without<Occupied>>,
) {
    for (e, mut ai, mut car, tf) in &mut cars {
        let pos = tf.translation;
        ai.since_progress += time.delta_secs();
        ai.advance(pos);
        // the road ahead runs out: the next edge from the node it drives to
        if ai.remaining() < 60.0
            && !(ai.stop_at_end && ai.plan.is_empty())
            && let Some(net) = traffic.as_mut().and_then(|t| t.net.as_mut())
        {
            let node = ai.toward;
            let here = ai.edge;
            let mut options: Vec<usize> = net.graph.adjacency[node]
                .iter()
                .copied()
                .filter(|&x| x != here)
                .collect();
            let mut next = None;
            if !options.is_empty() {
                let mut k = (ai.rand() * options.len() as f32) as usize % options.len();
                let planned = ai.plan.pop_front().filter(|e| options.contains(e));
                if let Some(p) = planned {
                    // the planned road (`plan`), the others after it should it be unknown
                    options.retain(|e| *e != p);
                    options.insert(0, p);
                    k = 0;
                } else if let Some(t) = ai.target {
                    ai.plan.clear();
                    // heading somewhere: the road whose far end is nearest to it first
                    let t = to_engine(t);
                    let d = |e: usize| {
                        let p = net.graph.nodes[net.graph.other_end(e, node)].pos;
                        (p[0] - t[0]).powi(2) + (p[2] - t[2]).powi(2)
                    };
                    options.sort_by(|a, b| d(*a).total_cmp(&d(*b)));
                    k = 0;
                }
                for j in 0..options.len() {
                    let edge = options[(k + j) % options.len()];
                    let far = net.graph.other_end(edge, node);
                    if let Some(l) = lane(net, edge, far) {
                        next = Some((edge, far, l));
                        break;
                    }
                }
            }
            // at a dead end (or where the next road's shape is not known) the route
            // simply ends: the car stops there and is given up on
            if let Some((edge, far, l)) = next {
                ai.edge = edge;
                ai.toward = far;
                ai.route.extend(l.into_iter().skip(1));
            }
        }
        let speed = car.speed;
        let forward = forward_of(tf.rotation);
        // steer for a point ahead on the lane
        let ahead = 6.0 + 0.6 * speed.abs();
        let Some(goal) = ai.look_ahead(pos, ahead) else {
            car.throttle = 0.0;
            car.brake = 1.0;
            continue;
        };
        let to = Vec3::new(goal.x - pos.x, 0.0, goal.z - pos.z);
        // both anticlockwise from above: a positive error is a turn to the left, which
        // is a positive steer
        let yaw = yaw_of(tf.rotation);
        let want = yaw_toward(to);
        let err = (want - yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        car.steer_input = (err * 2.5).clamp(-1.0, 1.0);
        // how fast: the cruise, the bend ahead, the end of the road, what is in front
        let bend = ai.curvature_ahead(30.0 + speed * 2.0);
        let mut target = ai.cruise.min((BEND_ACCEL / bend.max(1e-4)).sqrt());
        if ai.remaining() < 15.0 {
            target = target.min(3.0);
        }
        let stopping = speed.max(0.0).powi(2) / (2.0 * 4.0);
        let reach = CLEAR_AHEAD + stopping + car.length * 0.5;
        let nose = pos + forward * (car.length * 0.5 + 0.3) + Vec3::Y * 0.8;
        let moves = |body: Entity| bodies.get(body).is_ok_and(|b| !b.is_static());
        let mut skip = vec![e];
        skip.extend(car.collider_children.iter().copied());
        // look along the lane, not the bonnet, so a bend's kerb-side posts don't count
        let look = Vec3::new(goal.x - nose.x, 0.0, goal.z - nose.z);
        if let Ok(dir) = Dir3::new(if look.length() > 1.0 {
            look
        } else {
            Vec3::new(forward.x, 0.0, forward.z)
        }) && let Some(hit) = spatial.cast_ray(
            nose,
            dir,
            reach,
            true,
            &SpatialQueryFilter::from_excluded_entities(skip),
        ) && hit.normal.y.abs() < 0.7
            && moves(colliders.get(hit.entity).map_or(hit.entity, |c| c.body))
        {
            // something stands in the way: stop short of it
            let room = (hit.distance - 3.0).max(0.0);
            target = target.min((2.0 * 4.0 * room).sqrt() * 0.8);
            if room < 1.0 {
                target = 0.0;
            }
        }
        ai.target_speed = target;
        let diff = target - speed;
        car.throttle = (diff * 0.35).clamp(0.0, 1.0);
        car.brake = (-diff * 0.4).clamp(0.0, 1.0);
        if target < 0.1 && speed < 1.0 {
            car.throttle = 0.0;
            car.brake = 1.0;
        }
        car.handbrake = false;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::fleet::testing;
    use jc2_formats::roads::{RoadEdge, RoadNode};

    /// A straight road of two nodes, `a` to `b` (Bevy axes).
    pub(crate) fn straight_road(a: Vec3, b: Vec3) -> RoadNetwork {
        let (ea, eb) = (to_engine(a), to_engine(b));
        let length = a.distance(b) as u32;
        let graph = RoadGraph {
            version: 6,
            nodes: vec![
                RoadNode {
                    pos: ea,
                    flags: 1,
                    extra: 2,
                },
                RoadNode {
                    pos: eb,
                    flags: 1,
                    extra: 2,
                },
            ],
            edges: vec![RoadEdge {
                from: 0,
                to: 1,
                length,
                object_id: 0,
                instance_id: 0,
                a: 0,
                b: 1,
            }],
            grid: (1, 32768.0),
            cells: vec![(0, 2)],
            lookup: None,
            adjacency: vec![vec![0], vec![0]],
        };
        RoadNetwork::new(graph)
    }

    /// Distance from `p` to a polyline in the ground plane.
    fn off_line(p: Vec3, line: &[Vec3]) -> f32 {
        line.windows(2)
            .map(|w| {
                let (a, b) = (Vec2::new(w[0].x, w[0].z), Vec2::new(w[1].x, w[1].z));
                let q = Vec2::new(p.x, p.z);
                let t = ((q - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                (a + (b - a) * t).distance(q)
            })
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn the_lane_is_on_the_right() {
        // driving engine +z (south), the lane is at engine −x (west): the right
        let net_a = Vec3::new(0.0, 0.0, 0.0);
        let net_b = to_bevy([0.0, 0.0, 100.0]);
        let mut net = straight_road(net_a, net_b);
        let l = lane(&mut net, 0, 1).unwrap();
        assert!((to_engine(l[0])[0] + LANE_OFFSET).abs() < 1e-4, "{l:?}");
        // and on the right as Bevy draws it: a camera looking along the road (with +y
        // up) has `forward × y` on its right
        let right = (net_b - net_a).normalize().cross(Vec3::Y);
        assert!((l[0] - net_a).dot(right) > LANE_OFFSET - 1e-3, "{l:?}");
        let cam = Transform::default().looking_to(net_b - net_a, Vec3::Y);
        assert!((cam.right().as_vec3() - right).length() < 1e-5);
        // the other way, the other side
        let back = lane(&mut net, 0, 0).unwrap();
        assert!(
            (to_engine(back[0])[0] - LANE_OFFSET).abs() < 1e-4,
            "{back:?}"
        );
        assert!(four_wheels("lave.v001_sedan.ee") && !four_wheels("lave.v006_moped.ee"));
    }

    /// An AI sedan on a straight 450 m road across the flat test tile: it gets going,
    /// cruises at 40–60 km/h and stays within 2 m of its lane for 20 s; then something
    /// across the road ahead makes it stop, and when someone gets in the AI lets go.
    /// Needs JC2_TEST_DIR.
    #[test]
    fn an_ai_car_follows_a_straight_road() {
        let car_name = "lave.v001_sedan.ee";
        let Some((mut app, ground, spawn)) = testing::app(&[car_name], |app| {
            app.add_systems(FixedUpdate, crate::vehicle::vehicle_physics)
                .add_systems(Update, (hand_over, ai_drive).chain());
        }) else {
            return;
        };
        let a = Vec3::new(spawn.x, ground, spawn.z + 220.0);
        let b = Vec3::new(spawn.x, ground, spawn.z - 230.0);
        let mut net = straight_road(a, b);
        let route = lane(&mut net, 0, 1).unwrap();
        let start = route[0] + (route[1] - route[0]).normalize() * 10.0;
        let rot = Quat::from_rotation_y(yaw_toward(b - a));

        app.insert_resource(Traffic::with_network(net));
        let car = testing::spawn(
            &mut app,
            car_name,
            Place::At(Transform::from_translation(start).with_rotation(rot)),
        );
        app.world_mut()
            .entity_mut(car)
            .insert((AiDriver::new(route.clone(), 0, 1, 14.0, 7), AiDriven))
            .remove::<Occupied>();
        let mut worst: f32 = 0.0;
        let mut top: f32 = 0.0;
        for i in 0..(20 * 64) {
            app.update();
            let t = app.world().get::<Transform>(car).unwrap().translation;
            if i > 64 {
                worst = worst.max(off_line(t, &route));
            }
            top = top.max(app.world().get::<Vehicle>(car).unwrap().speed);
        }
        let t = app.world().get::<Transform>(car).unwrap().translation;
        let travelled = t.distance(start);
        eprintln!(
            "AI car: {travelled:.0} m in 20 s, top {:.0} km/h, at most {worst:.2} m off its lane",
            top * 3.6
        );
        assert!(worst < 2.0, "{worst} m off the lane");
        assert!(travelled > 150.0, "only {travelled} m");
        assert!(top > 11.0 && top < 17.5, "top speed {top} m/s");

        // a stalled truck (a kinematic block) across the lane 40 m ahead: it stops short
        let fwd = (b - a).normalize();
        let wall_at = t + fwd * 40.0 + Vec3::Y * 1.5;
        app.world_mut().spawn((
            RigidBody::Kinematic,
            Collider::cuboid(12.0, 3.0, 0.5),
            Transform::from_translation(wall_at).with_rotation(rot),
        ));
        for _ in 0..(6 * 64) {
            app.update();
        }
        let v = app.world().get::<Vehicle>(car).unwrap().speed;
        let t2 = app.world().get::<Transform>(car).unwrap().translation;
        let gap = (wall_at - t2).dot(fwd);
        eprintln!("stopped {gap:.1} m short of the wall at {v:.2} m/s");
        assert!(v.abs() < 0.5, "still moving at {v}");
        assert!(gap > 2.0, "hit the wall: {gap}");

        // Rico gets in (`E` inserts `Occupied`): the AI lets go of it
        app.world_mut().entity_mut(car).insert(Occupied);
        app.update();
        assert!(app.world().get::<AiDriver>(car).is_none());
        assert!(app.world().get::<AiDriven>(car).is_none());
    }
}
