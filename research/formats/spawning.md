# Soldiers, pickups and heat

Implemented in `crates/jc2-formats/src/enemies.rs`, `pickups.rs`, `heat.rs` (readers) and
`crates/jc2-game/src/npc.rs`, `combat.rs`, `ammo.rs`, `pickups.rs`, `heat.rs` (the game).
Weapons, upgrades and ammunition are in `weapons.md`. Key names marked ✓ were found by
hashing candidates (`research/README.md`; the candidates came from `strings.csv`, word
lists and combinations); the rest are read by hash.

## Who the soldiers are: `settings\enemytypespawnsettings.bin`

```
root
  #b893ea9b/#ceecfe17/#d3159eae = "pursuitandsearch.aiteam", "bofcx1.aiteam", "heatboat.aiteam"
  enemy_types ✓ {                         one node per stage of the game, by chaos points
    chaos_00 ✓ {                          (#16d9534e; chaos_01 … chaos_04, as aidifficulty.bin's chaos_levels)
      min_chaos ✓ = 0, max_chaos ✓ = 100000
      elite_probability_easy ✓ = 0, _normal ✓ = 10, _hard ✓ = 20, _extreme ✓ = 30   (percent)
      grunt ✓ { health ✓ = 100, weapon_0 { weapon_name ✓ = "Beretta Handgun", probability ✓ = 60 },
                weapon_1 { "Onehand Shotgun" 20 }, weapon_2 { "Revolver" 10 }, weapon_3 { "Scorpion_SMG" 10 },
                eleven unnamed numbers, all 0 }
      elite ✓ { health = 250, weapon_0 { "Assault" 70 }, weapon_1 { "Shotgun (pump action)" 20 },
                weapon_2 { "MachineGun" 10 }, #02c28a95 = 0.01, #4e6057f3 = 0.1, #7291c6ae = 1,
                #88216a5e = 100 (armour?) }
    }
    … four more: 100000–200000, 200000–300000, 300000–400000, 400000–500000; the elite share
    rises (normal 10 → 45 %), the weapon weights even out, the last stage has 120/265 health
  }
  factions ✓ {                            the dolls each faction's classes wear
    #4fad2d93 { elite_cdoll ✓ = "pd_gov_elite.cdoll", grunt_cdoll ✓ = "pd_gov_base01.cdoll" }
    roaches ✓, reapers ✓, ularboys ✓ { PD_<Faction>Elite1 / PD_<Faction>Base1 }
    custom1..4 ✓ { thug boss / thugs, Japanese veterans, factory workers, Black Hand }
  }
```

`#4fad2d93` is the government (its dolls); its name did not hash from the candidates
(`government` is `factions.bin`'s key, a different hash). `jc2_formats::enemies`.

jc2-game rolls each soldier of the location teams (`npc.rs`): elite with
`elite_probability_normal` at the stage of the chaos total (`chaos.rs` keeps
`EnemyRoster`'s chaos up to date: 10 % at the start, 45 % at the last stage), else grunt;
the weapon by the class's weights; the class's health; the government's doll for the class
(`pd_gov_base01.ee` / `pd_gov_elite.ee`). Soldiers already standing keep their roll; the
ones that appear after the chaos crosses into a new stage (new tiles, reinforcements) are
of the new stage. **Assumed:** the village teams (`bofcx1.aiteam`) are the government's
(the team data names an AI agent, not a faction), the difficulty is normal.

**The elites' look** (checked on screen: `--world 31,14 --chaos 450000 --face-elite`): the
`pd_gov_elite` doll is a helmet with a red-lensed visor, a dark plate carrier with pouches
over a red shirt, dark trousers and boots — not the casino's Demolition Officer (tan vest,
green goggles), who is a mission character. At the last stage they carry the pump shotgun,
the machine gun or the assault rifle (weights 33/34/33) and have 265 health.

## Pickups

* `exported\pickups\pickup.<weapon>.ee` (one per weapon and item; `.crated` variants put
  the gun in a box): one `.epe` blackboard, a `CWeapon` with `weaponname` ✓ (`Scorpion_SMG`),
  `pickup_count` ✓ (`#dc0f0e6c`: Assault 1, Beretta 5, SMG 4, pump 10, rockets 5,
  revolver 50, GL 50, sniper 1, MG 1, minigun -1) and a smart object
  (`SOWeaponPickup` → `CSmartObjectTrigger` → `CWeaponPickupAction`, prompt
  `hud_button_11`). Grenades and charges are a `CItem` with `modelname` ✓
  (`weapons\wea33\wea33-wea33.lod`) and a `CGrenadePickupAction`.
* `settings\weapon_pickup.bin` (and `grenade_pickup.bin`, `triggered_pickup.bin`) is the
  template of the same object: `weaponname = "Assault"`, `pickup_count = -1`;
  `player.bin` and the root `weapons` node of `weapons.bin` point at them — the object
  the game spawns when someone drops a weapon (assumed).
* **Weapon stashes**: a location's `CSpawnSet` with a `CExportedEntityComponent`
  (`#f25a8a76` = `weaponstashes\mod.stash.smg.ee`, `#9f62f96f` the event) places a crate
  (`mod.stash.<weapon>.ee`: `CSimpleRigidObject` `wea00-b.lod`, the lid `wea00-b1.lod`
  under a `CGeometryObject` with a `CAnimRotation`, trigger and distance objects) with the
  `CWeapon` lying in it (`pickup_count` 5). Tile 31,14 has three: SMG, grenades, Beretta
  (`jc2_formats::placement::exported_spawns`, `jc2_formats::pickups::PickupEntity`).
* `CResourceItem`s (`pickup.boost.weapon/armor/vehicle/cash.ee`) are the upgrade parts,
  armour and cash: they stand where the location puts them and walking over one gives a
  part or $2500 (`chaos.md`).

jc2-game (`pickups.rs`): the stashes appear with their tile (crate + gun models, no
collision), soldiers drop their gun beside them (gone after 90 s), and Rico takes a pickup
by walking over it (1.4 m): the same gun → `pickup_count` magazines into the reserve (up
to what he can carry); a new gun → it joins his guns on the next number key, magazine
full and the rest in reserve; grenades → added up to 10. A taken stash stays empty
while its tile is loaded and after. **Assumed:** `pickup_count` counts magazines (the
game's unit is not known; the reserve cap makes the difference small), a dropped gun
gives its `pickup.<weapon>.ee` count (1 when there is none), walking over picks up
(the game asks for a button).

## Heat: `settings\heatsystem.bin`, `heatspawn.bin`, `ai\aiheatparams.bin`

`heatsystem.bin`: `heatsystem ✓ { max_heat ✓ = 100, combat_time ✓ = 1, combat_delay ✓ =
2, undetected_time ✓ = 2, heat_level_max_heat_1…5 ✓ = 20, 40, 60, 80, 100,
heat_level_chaos_1…5 ✓ = 0, 0, 100000, 200000, 300000, … }` plus unnamed 5, 10, 15
(`#22437136`: perhaps `combat_delay_restricted`, a depth-3 hash hit, unconfirmed), 20, 30,
100, 200, 1.25, 1. A heat level can only be reached once the chaos passes its
`heat_level_chaos_N`: levels 1 and 2 from the start, 3 at 100 000 (`chaos.md`).

`heatspawn.bin`, per faction (`spawn_event` ✓ = `__heatspawn_government`, roaches,
reapers, ularboys, custom1–3):

```
heat_000 ✓ { min_heat_level ✓ = -1, max_heat_level ✓ = 0, waves ✓ { wave0 ✓ {…}, wave1 ✓ {…} } }
heat_00 ✓  { 0 … 20,  waves { wave0 { initial_delay ✓ 0, min_spawn_delay ✓ 1, max_spawn_delay ✓ 2,
                                      respawn_delay ✓ 6, #19f1aa22 30, #64747c9e 0,
                                      unit_00 ✓ { unit_type ✓ "ut_mc", max_units ✓ 2,
                                                  unit_decal ✓, unit_palette_file ✓ "Mil_Djungle.pal",
                                                  game_zones ✓ { per climate zone (game_zone_id ✓) palette } },
                                      #54dc4296 { "ut_jeepsmall" 1 } },
                              wave1 { … "ut_attackboat" } } }
… 20–40, 40–60, 60–80 (heat_03 ✓), 80–100: ATVs, military jeeps (2 each), helicopters, boats
unit_types ✓ { ut_mc { name, type ✓ "land", entity_file ✓ "exported\vehicles\lave\lave.v007_military.ee",
                       num_seats ✓ 1, game_zones "all" }, ut_militaryjeep 4 seats, ut_jeepsmall 2, … }
```

The other factions have one band (−1 … 30) with a jeep. The root's `#4b6a6a25` block
(250/300/600, `spawn_speed` ✓ 30, `min_speed`/`max_speed` ✓ children) is probably the
spawn distances and speeds; `#ad54d00f`/`#b45ef0fb`/`#e541c5f8` (10, 10, 20) unknown.
`jc2_formats::heat`.

`ai\aiheatparams.bin`: two tables (`#7f245b02` with 18 events, `#8c4886f1` with 16) keyed
by the hash of an event name, each with `Points` ✓ (100, or 25 for a vehicle hitting a
vehicle; in the second table 0 except two), `CivilianPoints` ✓ (`#af48444d`, 0–100),
`Range` ✓ (0–100 m), `Delay` ✓ (`#2d94e355`, 0.5–2 s), `UseWitness` ✓ (`#8164a165`),
`HeatSoundEvent` ✓ (`#46cf0430`, empty), `#d5c5cdfb` (empty) and `CivilianSoundEvent` ✓
(`#e6a2043f`), the civilians' reaction. The event names did not hash from any candidate
(every substring of the exe's identifiers in four casings, two-word combinations of its
upper-case words, numbered patterns): the exe registers its AI event names
(`CIVILIAN_SHOTS_FIRED`, `PLAYER_STOLE_MILITARY_CAR`, `FIRST_LOS_SINCE_HEAT_RESET` …) at
start-up, and none of those match either. Six are identified by their reaction:

| key | reaction | Points | CivilianPoints | Range | Delay | UseWitness |
|---|---|---|---|---|---|---|
| `#0d038313` | `CIVILIAN_NPC_KILLED_BY_PLAYER` | 100 | 50 | 40 | 0.5 | 0 |
| `#50e6844a` | `CIVILIAN_EXPLOSION_TRIGGERED_BY_PLAYER` | 100 | 50 | 50 | 0.5 | 1 |
| `#c433a5fd` | `CIVILIAN_SHOTS_FIRED` | 100 | 30 | 70 | 0.5 | 1 |
| `#321deed6` | `CIVILIAN_VEHICLE_HIT_BY_VEHICLE` | 25 | 0 | 0 | 0.5 | 0 |
| `#355df81d` | `CIVILIAN_PLAYER_ON_VEHICLE` | 100 | 0 | 0 | 0.5 | 0 |
| `#fb08a449` | `CIVILIAN_STOLE_MY_VEHICLE` | 100 | 25 | 0 | 0.5 | 0 |

The other twelve (`#96a83491`: 100 points, CivilianPoints 100, 1 s, and worth points in both
tables; `#4075c8b0`: 2 s delay, 30 m; …) stay unnamed. The root's `Parameters` ✓
(`#f16ab128`: 5, 2, 2.5, 1, 120, 73, 5) are not understood.

jc2-game (`heat.rs`): a `Heat` resource 0 … `max_heat`, level = the 20-wide band (0–5,
shown on the HUD as `HEAT n`), capped by the chaos (`Heat::cap`, `heat_level_*`). A
soldier's death raises it by the kill event (100 points) when another soldier is within
its 40 m, after its 0.5 s delay; a blast by the explosion event (100) within 50 m of a
living soldier. 100 points are taken as one level's width (20): **assumed**, the game's
conversion is not in the data. While any soldier sees Rico it holds, after 20 s out of
sight it falls by 2 a second (a level in 10 s) — **both assumed**. Without
`aiheatparams.bin` the old numbers (12 a kill, 8 a blast within 40 m) apply. From level 1 the
government band of the current heat sends soldiers on foot: at most the band's land
seats (level 1: two motorcycles of 1 and a jeep of 2 = 4; never more than 6), one every
`respawn_delay` (6 s), each rolled like the garrison (`enemytypespawnsettings.bin`),
appearing 80–120 m away behind the camera (±60°), running at 3 m/s to Rico's position
and re-aimed at him every 4 s until they see him. They stay when the heat is gone and
leave with their tile.

**Vehicle reinforcements** (`heat.rs`, `HeatVehicles`). The band's units (`unit_type` →
`unit_types` → `entity_file`, `num_seats`, `type` land/air/sea) bring vehicles too:

* From level 2, a car of soldiers: the band's four-wheeled land unit with the most
  seats (bikes and scooters left out: the raycast car needs four wheels) —
  `lave.v047_jeep_small.ee` (2 seats) in 20–40, the police ATV `lave.v056_atv.ee` (1) in
  40–60, `lave.v047_jeep_small.ee` when the band has none. It is asked of the fleet on a
  lane point 150–300 m from Rico on a loaded tile, on a road that reaches the road node
  nearest him in at most 1.5 × the straight distance + 60 m (Dijkstra over
  `default.rg`'s edges and lengths, `road_distances`; no road winding away up a valley),
  with a soldier at the wheel (`crew.rs`). It drives the planned way with the traffic AI
  (`traffic::AiDriver`: `plan` the edges junction by junction, `stop_at_end`, `target`
  Rico for greedy choices off the plan) at 50 km/h and stops within 30 m of Rico, after
  6 s without headway, or at the end of its way (the node nearest him: Rico is often off
  the road): its seats get out as foot reinforcements beside it (at most 4) and run at
  him. More than 600 m away it has lost its way and goes. One at a time, at most every
  20 s; the traffic's own clean-up leaves it alone.
* From level 3, an attack helicopter: the band's air unit
  (`arve.v009_military_helicopter.ee`, the fallback too), placed 150 m away behind the
  camera and lifted 40 m, with a soldier pilot (`crew::HeliPilot::hunting`) that circles
  Rico at 50 m, 30 m over the ground under him, at 10 m/s, and a door gun: 6-shot bursts
  every 3 s while Rico is within 80 m and in sight, a quarter of the shots hitting for 10
  (**all assumed**; orange tracers). It stops firing (and circles where it is) once the
  heat is gone. One at a time. Level 3 needs 100 000 chaos (the cap); `--heat 50` starts
  above the cap, which only stops rises.
* Both are crewed vehicles: `E` from the car's side or from the helicopter's stunt
  position hijacks them (`vehicle.md`).

Headless test (`heat::tests::heat_sends_a_car_along_the_road_and_a_helicopter`): on a
straight test road the heat-2 car is asked for 180 m away, drives to 30 m of Rico and
its two soldiers get out; the heat-3 helicopter circles him at 54 m, 30 m up, and fires.

## Crewed boat shore avoidance

`crew::BoatPilot` keeps the demonstration boat's slow cruise circle in open water.
Every 0.25 s it probes the bow corridor and alternative headings, including the hull's
width, using both `terrain_win32.dat` (64 m samples) and loaded collision geometry.
The higher ground wins: a rock or detailed shore absent from the coarse map still
blocks the route. The vehicle and all its descendants (extra hull colliders and seated
crew) are excluded from the collision query. Unknown
ground is treated as blocked when neither source is available.

The lookahead grows with speed (eight seconds of travel plus four times the hull length,
clamped to 60–200 m). On finding shallows, the skipper lowers the throttle and chooses
the clearer escape side; it retains that side until the bow corridor clears, unless
that side becomes blocked and the other is clear. Close to a blocked bow it reverses
only if the stern corridor is clear, otherwise cuts thrust. Minimum depth is 3 m,
with 3 m longitudinal margin and a quarter-hull-length side margin (at least 3 m).
These navigation margins and steering rules are **assumed**, not decoded original AI.
`Occupied` boats are excluded, so hijacking hands all steering back to Rico.

Tests cover open-water cruise, early shore turning, side persistence, hull-width
clearance, reverse escape and fully blocked water. A nested child-collider ray regression
checks that own geometry is excluded while independent obstacles remain visible.
A headless test runs the original
motorboat against a synthetic shore collider absent from the global map, checks its
minimum clearance and turn, then checks that occupying it stops AI control.
On 8 October 2026 the original motorboat cleared the synthetic shore by at least
7.0 m at its centre (the assertion requires half the hull length plus 2 m), turned
1.55 rad and escaped sideways. All 276 workspace tests passed; release clippy was
warning-free (isolation 31, plain Steam game data).

This is local avoidance, not global sea route planning: it does not guarantee a route
out of arbitrary enclosed inlets, and dynamic ships are treated as obstacles rather
than coordinated traffic.
The corridor uses nine longitudinal stations (up to 25 m apart at the maximum
lookahead) and three lateral rays per station; small obstacles between samples can
be missed. This does not provide continuous collision clearance.

## Still open

* The faction key `#4fad2d93`, which aiteam belongs to which faction, the difficulty
  setting.
* Heat: the names of the unidentified events of `aiheatparams.bin` (a vehicle destroyed, a
  weapon seen, trespassing), how points become heat, what the second table is for, the
  boats of the bands (`ut_attackboat`, `ut_militaryboat`) and the helicopters' rockets,
  how many units of a wave come at once (`max_units`; jc2-rs sends one vehicle of each
  kind), `heat_000`'s meaning, `#19f1aa22` (30, 60: a lifetime?), the root's
  spawn-distance block (250/300/600 m, `spawn_speed` 30: perhaps where and how fast the
  vehicles appear).
* Pickups: the button prompt and outline (`COutline`), the stash lid animation
  (`CAnimRotation`) and collision, the resource crates' own crate (the `.crated` spawn
  sets), the triggered charge.
