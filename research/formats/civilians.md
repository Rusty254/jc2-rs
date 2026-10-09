# Civilians and road traffic

Implemented in `crates/jc2-formats/src/civilians.rs` (the world simulation's spawn
tables, roam areas, game zones) and `roads.rs` (the road graph and road splines);
`jc2tool civilians <game> [X Z]` and `jc2tool roads <game> [X Z]` print them. The game
side is `crates/jc2-game/src/civilians.rs` (people on foot) and `traffic.rs` (cars on
the roads). Key names marked ✓ were found by hashing candidates (`research/README.md`);
the candidates came from `strings.csv` and word lists.

## Where it all comes from

| What | Where | Found by |
|---|---|---|
| Who spawns, where, how many | `settings\worldsim.bin` | `jc2.avproj`'s `CWorldSim` system; the file names `Terrain\GameZone.bin` and `worldsim.aiteam` |
| Game zones of the ground | `terrain\gamezone.bin` | `worldsim.bin`'s `gamezone_file` ✓ |
| Where people walk | `CCivilianRoamArea` objects in the tiles' `.blo` (`26x41.nlz` has 2, the city tile `33x26.nlz` 9; the base `31x14.nlz` none) | `strings.csv` (`CivilianRoamArea`, `SCivilianRoamAreaSpawnPoint`) |
| The road graph | `ai\default.rg` (428 KB) | `jc2.avproj`: `CAISystem { _class = CAISystemProperties, RoadNetworkFile = "default.rg" }`; read by `FUN_00477da0` once the AI system has looked up `RoadNetworkFile` |
| Road geometry | `CRoadObject`s in `locations\roads\roo_*.nlz` (24 regional containers), `locations\metropolis\city.district.*.roads.nlz` (9) and some tiles' own `.nlz` | `jc2tool list | grep roads`; `CRoadManager` reads `settings\roads.bin` (the road *render* types: widths 12/24/48, textures) |
| People's skeleton and clips | `animations\skeletons\syncreadthis\civilian_biped.bsk` (40 bones, a subset of Rico's 84 with the same names); `pose_shared_idle/walk/fem_walk/panicrun/drive/mc_idle.ban` | `strings.csv` (the full paths) |
| The `.aitile` files | `ai\patches\XX\NNNNN.aitile`, 16,384 of them, 465 MB | `%i.aitile`, `NCtgAI::CAITile` — the PathEngine navigation mesh for people on foot. **Not read**: the roam areas and road graph were enough |

## `settings\worldsim.bin`

```
root
  gamezone_file ✓ = "Terrain\GameZone.bin", #72c67918 = "worldsim.aiteam", #0a46dba1 = 100, …
  character_types ✓ {                                    31 kinds of pedestrian
    #2273a2b5 { type ✓ = "PD_Generic_Female_1.cdoll",
                spawnzones ✓ = "city:5 village:5 religious:5 default:5",
                gamezones ✓  = "jungle:5 sea:5 city:5 grass:5 default:5" }
    … the generic men and women (5 each), fishermen (village), arctic and desert villagers
      (their climate only), city people, tourists, oil-rig workers (industry), a monk
  }
  lave ✓ {                                               cars
    max_count ✓ = 15, #ae93c288 = 15, #a02260fb = 3, #7635628a = 0.2, #8448ae90 = 20
    #15edfea4 { min_inner_spawn_range ✓ 100, max_inner_spawn_range ✓ 120,
                min_outer_spawn_range ✓ 140, max_outer_spawn_range ✓ 160,
                front_despawn_range ✓ 160, back_despawn_range ✓ 30,
                height_at_max ✓ 50, speed_at_max ✓ 25, xz ✓ 1 }
    spawners ✓ { #06215d8d { order ✓ 0, type = "road", #3966cddb 0.8,
                             road_types ✓ { #83e97302 = "spline", #eed1fa05 = "ground_roam" } } }
    vehicle_types ✓ { #2b9c5536 { type = "lave.v001_sedan.ee",
                                   spawnzones = "city:30 default:30", gamezones = "city:30 sea:30" }
                      … 35 kinds (one entry has no `type`) }
  }
  seve ✓ { … boats on "open_water", "water_roam" }, arve ✓ { … planes "open_air", altitude 200–2150 }
  #7c49be4c { … people: max_count 15, ranges 30–120 m, a "spline"/"ground_roam" spawner }
  traffic_density ✓ { five levels: index 0..4 → density ✓ 0, 3, 4, 8, 16 }
  night_time ✓ { start = "01:00", end ✓ = "05:00", factor ✓ = 0.34 }   fewer at night
  parked_vehicles ✓ { #0772240b 80, #29401cc5 2, #b1080259 130 }
```

`spawnzones` weigh a type by the kind of settlement (`city`, `village`, `industry`,
`tourist`, `religious`, `default` — the second table of `gamezone.bin`, see below),
`gamezones` by the ground. jc2-rs picks by `gamezones` only (falling back to `default`):
the settlement kind of a place is in a map not decoded yet.

## `terrain\gamezone.bin`

Two tables under `root` ✓ (`#587abedf`/`#ddd45f27` = 64 × 64): the game zones
(`None`, `Arctic` 1, `Desert` 2, `Jungle` 3, `Sea` 4, `City` 5, `Grass` 6, `Default` 7,
each with a `description`, a debug colour and two levels down `materials` ✓, the
terrain's gameplay material bytes that make the zone: City = [33], Arctic = [64 … 84],
Desert = [128 … 150], …) and the spawn zones (`#f9a2ede4`: Default 0, City 1, Industry
2, Tourist 3, Military 4, Village 5, Religious 6, Ice 7). jc2-rs looks the ground's zone
up from the terrain tile's material byte under the point
(`civilians::GameZones::of_material`). The tiles' own 8 m "game zone" byte map
(`TerrainTile::gamezone`) holds other values (16, 30, 54, 80, 88 around the village
26,41) — probably the spawn-zone map, packed; not decoded.

## `CCivilianRoamArea`

```
CCivilianRoamArea { name = "CCivilianRoamArea1", world, box_size ✓ = [15, 5, 65] (half
                    extents along its own axes), capacity ✓ = 8,
                    _child_objects ✓ / _child_objects1..3 ✓ = names of spawn points,
  SCivilianRoamAreaSpawnPoint { world (relative), name }  … 3–6 of them }
```

Next to them: `CWorldsimArea` (`box_size` [50, 25, 50], `civilians` ✓ = 10 — a local
override of how many), `CParkedVehiclePoint`, and many `CCivilianSOController` /
`CEnterCivilianSOAction` smart objects (benches, doors; not used).

## `ai\default.rg`: the road graph

```
u32  0x06000df0      low 24 bits 0xdf0; the high byte is the version, 5 or 6 accepted
u32  nodes (3653), u32 edges (4168)
nodes × 16  { f32 x, y, z; u16 flags (0, 1, 3, 5, 13); u16 extra (2 on 95 %) }
edges × 18  { u16 from, to; u32 length (m along the road);
              u32 object_id, instance_id — the `_object_id` and `_instance_id` of the
              CRoadObject the edge lies on; u8 a, b (0..2 each, not decoded) }
u32 64, f32 512      a node grid over the world: 64 × 64 × { u32 first, u32 count },
                     cell index z × 64 + x; the nodes are stored sorted by cell
version 6 (FUN_00462960):
u16 128, u16 count (38179); count × { u16, u16 height (m), u16 edge }; 128 × 128 u16
                     running end index per 256 m cell — the edges near a point
                     (`ClosestRoad`); the first u16 is not decoded
```

The file ends exactly there. Nodes are junctions and road ends; an edge has no
geometry of its own, it is the piece of one road spline between its two nodes. Only
130 of 4168 edges also exist reversed: edges are undirected roads, and traffic uses
both directions. The total length of the edges (1145 km) matches the splines' (1162 km
in the road containers). `adjacency` is built on load.

## `CRoadObject`

```
CRoadObject { name = "roo.road", world (identity rotation in the regional sets, rotated in
              the city), spline ✓ = floats[4n]: x, y, z relative to `world`, then a value
              0..1 (0.5 on most points, not decoded), road_type ✓, size ✓ (1, 2, 5),
              _object_id, _instance_id, valid ✓, a dozen unnamed flags }
```

The curve passes through its control points (10–30 m apart on bends); jc2-rs uses the
polyline through them. **Matching edges to splines** (`roads::edge_shape`): a spline
whose polyline passes within 6 m of both nodes and whose arc length between them is
within 15 % of the edge's `length` — the `_instance_id`s are not unique across
containers, so the geometry decides. Over the island 2569 edges get their spline piece
from the 33 road containers (median length error 0.2 %); 1184 more are as long as
their chord (≤ 5 % longer) and are taken straight; the remaining 415 lie on splines in
the tiles' own containers (36 of a sample of 40 were found there), added as tiles
stream in.

## In jc2-game

**People** (`civilians.rs`). `TileSim` reads each streamed tile's placement file once
for its roam areas and road splines (and looks up each area's game zone from the
terrain material under its centre). Each tile gets up to `--civilians N` people (12),
spread over its areas up to their `capacity`, standing at the areas' spawn points
first. The doll is picked by `character_types`' `gamezones` weight for the area's zone
(`pd_generic_*`, fishermen, monks, oil-rig workers, tourists in the jungle…); its parts
load through `npc::request_outfit` (shared with the soldiers; one variant per part
group); the `.cdoll` names `civilian_biped.skeleton`, which is the `.bsk` of that name
(40 bones; the models' joint indices stay below 40). They walk at 1.1 m/s with
`pose_shared_walk.ban` (rate scaled from its 0.93 m/s root motion) between random
points inside 90 % of their box, pausing 2–8 s with `pose_shared_idle.ban`; a shot by
Rico within 30 m of them, a blast within 30 m or being hurt makes them run directly
away at 4.5 m/s (`pose_shared_panicrun.ban`) for 6–8 s, after which they walk back into
their box. A kinematic capsule with 50 health (assumed) takes Rico's bullets; at zero
they topple, lose their collider, add 4 heat (assumed: the data's civilian events are
in the unnamed `aiheatparams.bin` table) and lie 30 s. They leave with their tile.

**Traffic** (`traffic.rs`). At startup the road graph and the 33 road containers'
splines load (`RoadNetwork`); tiles add theirs as they stream. Every second, while
fewer than `--traffic N` (5) AI cars are about, one is asked of the fleet at a random
lane point 80–250 m from Rico, on a loaded tile and 40 m clear of the other traffic
cars, facing along the lane; its kind is picked from `lave.vehicle_types` by the
`gamezones` weight of the ground there (two-wheelers dropped). When the fleet has built
it (`Added<Vehicle>` within 1.5 m of the asked spot) it gets an `AiDriver` and
`vehicle::AiDriven`, which keeps the keyboard input from zeroing its pedals.

The driver: the lane is the edge's spline piece moved 2 m right in engine axes
(`roads::offset_right`, a miter at the bends), resampled every 10 m, converted with
`to_bevy`. Pure pursuit on a point 6 m + 0.6 s ahead (steering gain 2.5 per radian).
Speed: its cruise (40–60 km/h, random per car), limited by the sharpest bend in the
next 30 m + 2 s (√(3 m/s² / curvature)), 3 m/s for the last 15 m of a route, and a
ray along the lane at bonnet height that sees only things that move (cars, people,
Rico — not the posts and walls beside a bend): it stops 3 m short. Throttle and brake
are proportional to the speed error. With 60 m of route left it appends a random other
road at the node it drives to; at a dead end the route just ends. Cars more than 600 m
from Rico, or that have not passed a route point (≤ 10 m apart) for 15 s — stuck,
wedged or at a dead end — are removed and new ones come. Each car has a villager at the
wheel (`crew.rs`, `vehicle.md`: a generic civilian doll on the `SO_Driver` seat playing
`civ_driving_fwd`). `E` beside it hijacks it: the villager is thrown out, Rico is
seated (`Occupied`), the AI lets go and the car is his. The heat's cars use the same AI
with a planned way to Rico (`AiDriver::plan`, `spawning.md`).

Assumptions:

* People are picked by the ground's game zone (`gamezones` weights), not the settlement
  kind (`spawnzones`).
* Cars keep to the right-hand lane (the original drives on the right), 2 m right of the
  spline in engine axes (`roads::offset_right`). Lane widths from `size`/`roads.bin`
  are not used.
* Two-wheelers (`moped`, `vespa`, the `v007` motorbikes) are left out: the raycast car
  needs three or more wheels.
* `worldsim.bin`'s own counts and ranges (`max_count` 15, spawn 100–160 m, the
  `traffic_density` levels, `night_time`) are read but the counts and distances above
  are ours.

## Still open

* The `.aitile` navigation mesh (people walk straight lines inside their box), the
  settlement-kind (`spawnzones`) map, `CWorldsimArea`'s local `civilians` count, the
  civilian smart objects (benches, doors), the drivers' own dolls (`character_types`
  says who walks, not who drives: jc2-rs seats generic villagers), traffic lights (`CTrafficLight`), the edges' `a`/`b` bytes and the nodes'
  flags (lanes? one-way?), the edge lookup's first field, the boats and planes of the
  world simulation, the `roads.bin` render types (road meshes are drawn by the tiles'
  own models).
