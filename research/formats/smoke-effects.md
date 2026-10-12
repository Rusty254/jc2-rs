# Smoke effects: sources, the native emitter model, child routing, placed smoke (board task 4e32e9af, "fx-smoke", 12 October 2026)

Research only. Probe: `research/tools/smoke_effects_probe.py` (see Reproduce; exit 0). Builds on `effects-pipeline.md`
(spawner, component contract, section 11 particle renderer), `particles-next.md` / `particle-consumers-next.md` (leaf
format, modifier execution), `vehicle-fire-explosion.md` (vehicle smoke and fire slots) and `explosion-effects.md`.
Evidence: the retail `JustCause2.exe` (sha256 `df99a479...f74`), its Ghidra export and the retail data (`global/particles.blz`,
`effects/*.bin`, the location containers). "native" = read from the executable or the data and asserted by the probe;
"inferred" is said so. Addresses are retail virtual addresses; names of functions and fields are working labels.
Reading the particle manager rules of the native code, one name is used throughout: a **leaf** is one `.psmb` particle
system, an **emitter** is one live instance of a leaf in the manager's pool of 1024.

## 0. Answers in one table

| Question | Answer |
|---|---|
| What are the `#841408bf`, `#f47c95e6`, ... string slots of a leaf? | `linked_system_0`, `linked_system_1`, ... (Jenkins; the first ten hashes are `CHILD_SLOTS`). Native reader 0x00a71720 reads them in order and **stops at the first missing slot; there is no limit of ten** (retail goes up to 33 links, for example `fx_exp_grenade_12/13` sit behind slot ten; task dc95ceb0 found the same). No retail leaf has a slot behind a gap. The resource ids are stored at leaf+0x164. |
| How are the linked systems routed (the open gap of JC2-087)? | **Parallel emitters started with the parent.** The parent's emitter start 0x00a6d290 starts every linked system at once with the same matrix argument (shared live pointer when the parent uses an external matrix, a copy otherwise), the same duration override, flags 0 (no self release), and records itself as their parent (child+0x28c). When the parent stops or finishes (0x00a6af80) every child is stopped too. No spawn-on-death, no timing offset. Section 2. |
| What is the separate leaf key `#1de3fca7`? | The **per-particle trail system** (leaf+0xc, flag bit 14). 273 leaves have one. Every particle born gets its own trail emitter of that system that follows the particle and receives the particle's alpha as its system scale. Section 3. |
| Emission window, stop, release | Window = Start `duration` if > 0, else the leaf `system_lifetime`; **0 = unbounded**. Stop never kills particles: it sets a stop flag, an unbounded emitter stops birthing at once, a bounded one finishes its window. The emitter is released when the flag byte `#b0124c0a` (default **1**) allows it and no particle is alive. The component's `Stop(immediate)` is the same as graceful (it only drops the handle). Section 4. |
| Gates and limits | Births need (a) the global live particle count below **16384** (0x4000), (b) the squared distance from the context position (the camera side, inferred) to the emitter below the leaf's `#d1e88637` squared (**default 800 m**; retail 5 to 5000), (c) elapsed < window. Capacity = min(authored, int(ceil(life + 0.25 life_var) / spacing)), at most 150 with debris slots and 1000. 1024 emitters. Section 5. |
| Where does retail smoke come from? | 17 effects reach a leaf with `smoke` in its name (vehicle damage and bursts, environment smoke, fuel depot, trucksmoke, dome smoke); **chimneys, damaged-building smoke and steam are not effects**: they are 6916 `CParticleObject` world emitters that name a leaf directly (1305 `fx_env_smoke_damage_large_01`, 283 chimneys). Sections 6 and 7. |
| Vehicle speed input | The damage smoke and fire effects (150, 241, 151, 240, 166, 167, 385, 388) carry **no input binding**: the squared speed they receive is not read. Only 386 `seve_damage_fire_large` and 235 `lave_mc_damage_tailfire` bind `speed` (rate). Section 8. |
| Wind | None. The particle job has no wind term; modifiers are fixed force vectors and damping. Section 5.4. |
| Smoke lighting, fog | Fog on particles is done (JC2-163). Lighting / emissive / soft-depth remain with task 8bb6ab45. Section 9. |

## 1. Particle component start parameters (native, answers gap 7 of `effects-pipeline.md`)

The particle variation record is loaded by 0x005b9960 into 0x44 bytes (probe: allocation, defaults, key words at 0x00eba17c):

| Record | Key | Default | Use |
|---|---|---|---|
| +0x00 | `name` (`#d31ab684`) | -1 | resource id of the leaf (path and extension stripped, hashed) |
| +0x08 byte | `#b0124c0a` | **1** | flags byte handed to the emitter start; bit 0 becomes emitter flag bit 13 = **may release itself** (section 4). Retail: 1 on 451 variations, absent (=1) on 103, **0 on 22** |
| +0x14, +0x18, +0x1c, +0x30, +0x34 | `#d623c711`, `#85ee8f21`, `#d9fad10c`, `#7c4a3b08`, `#67867668` | -1 | input bindings (section 8) |
| +0x38 / +0x3c / +0x40 / +0x2c | `#fc405fe1`, `#3c9be34c`, `#4de5cc5c`, `#9b356e5f` | 20 / 100 / 0 / -1 | binding range minimum, maximum, floor and cut-off |

The component Start (0x0063d2b0) passes the variation data, the component matrix, the flags byte and the duration to 0x00a6e2a0 (spawn with the
**external** matrix: the emitter keeps a pointer to the component's matrix, so SetMatrix moves the emitter, 0x00725300). A second Start replaces the
emitter only if the old one has flag 2 or 4 (finished or stopping).

## 2. Linked systems: the child routing (native)

0x00a6d290 (the emitter start, called from the admission 0x00a6cb40 / 0x00a6e1d0):
1. flags: emitter flag bit 13 <- bit 0 of the Start flags byte; bit 15 (first-frame hold) set; start delay = base + random variation; elapsed = 0; matrix translation copied; `duration` > 0 overrides the system lifetime.
2. if the global switch `DAT_0108aaf9` is set (it is 1 in the file image) and the leaf has linked systems: for each id, look the leaf data up (0x00a69bb0); then start it with
   `0x00a6e1d0(data, matrix, flags 0, duration, 0, 0)` (matrix copied) when the parent was started with a copied matrix, else `0x00a6cb40(...)` (the parent's matrix pointer is shared);
   each child is admitted like any emitter (**1024 emitters, its own density scale**, so every child counts in the pool); the parent is stored in the child (+0x28c) and added to the parent's child list (+0x280).
3. Children therefore start in the same job as the parent, follow the same live matrix, run with the parent's duration, never self-release (flag bit 13 clear) and end when the parent ends.

0x00a6af80 (stop or finish, argument 0 = finished, else stop requested) sets word[+0x2a2] |= 2 or |= 4, detaches the matrix pointer into a private copy, then for every child: clears its parent
link and calls `0x00a6af80(1)` (stop requested). The manager recycles an emitter once flag 2 is set (0x00a6c930). Travelling effects therefore keep their children alive exactly as long as the head.

Retail shapes (probe): 477 leaves have `linked_system_0` (links per leaf: 107 have one, 85 two, ... 11 have ten, 2 have 23, one has 33); the biggest tree is `fx_exp_vehicle_explode_boat_01` with 54 leaves (links and trails counted); no cycles; four dangling child names (`fx_env_food_pea_02` x5,
`fx_f2m06_emptowerpulse_10`, `fx_env_fountain_drop_02/03`: the lookup returns no data and that slot is skipped). Vehicle smoke example (effect 150): head `fx_vehicle_smoke_medium_01`
(`distort_normal.dds`, a heat-haze leaf with 4 particles) + `_02` (smoke, local space 2) + `_04` (smoke, world space 1); 241 uses `fx_vehicle_smoke_large_01/_02/_04`; the MC pair is
`fx_vehicle_mc_smoke_01` -> `_02` (so `lave_mc_damage_smoke` is two parallel smoke emitters, as the JC2-014 fixture already guessed).

## 3. Per-particle trail systems (native)

Leaf+0xc (key `#1de3fca7`, flag bit 14 set when the id is not -1) names a **trail system**. 0x00a6e2f0 runs in the emitter's end-test pass (0x00a6e6c0) for every particle slot:
* a slot without a trail whose particle has age <= 1.19e-7 (born this job), size > 0, while the emitter is not stopping (flags & 6 clear), gets a new emitter of the trail system (`0x00a6e1d0`, matrix copy,
  flags 1) placed at the particle position (composed with the source matrix for space 2); the emitter is stored in the particle (slot +0x5c);
* a slot with a trail whose particle died (age >= 1, size <= 0) or whose emitter is stopping, or whose trail has finished, releases the trail (`0x00a6af80(1)`);
* otherwise the trail is moved to the particle position every job (0x00a66320 position, 0x00a66230 matrix) and, when the particle alpha is below 1, the **particle's alpha becomes the trail's system scale**
  (+0xac, and that of its own linked systems): the trail fades with the particle.
273 retail leaves have a trail (fire embers and sparks: `fx_trail_fire_mini_01`, `fx_trail_sparks_small_01`, ...). Every trail counts as an emitter of the 1024 pool.

## 4. Emission window, stop, release (native)

Emitter fields (instance, worker view = instance + 0x10): lifetime `L` (+0x88, from the leaf `system_lifetime` or the Start duration), elapsed `T` (+0x98, reset by Start, advanced by dt each job).
* **Births** (job 0x00a66490): allowed while `T < L` or `L <= 0` (unbounded); never while the start delay (+0x94) is positive or the first-frame hold (flag 0x8000) is set (the hold is cleared after the first job).
* **Stop** (0x00a6cac0 -> 0x00a6af80(1) -> flag 4): the end-test pass 0x00a6e6c0 then sets bit 13 and, if `L == 0`, sets `L = T = 1.0`, which closes the window at once. A bounded emitter (`L > 0`) **keeps its window** (a
  stopped 20 s fuel-depot smoke still emits until its 20 s are over). Live particles always run to the end of their life; nothing in the component or effect stop path removes them
  (the only global clear is the manager-wide flush 0x00a6ca10 -> 0x00a6c930, whose callers were not traced). So `Stop(immediate)` of the effect system equals graceful for particles: the component only drops its handle.
* **Finish**: when no first-frame hold, delay <= 0, no live particles, system time over (`L < T`, or `L == T != 0`) and bit 13 is set, the emitter calls `0x00a6af80(0)` (flag 2) and the manager recycles it.
  Bit 13 comes from the flags byte, so a variation with `#b0124c0a = 0` (22 retail variations) **never releases by itself**: its pool slot stays until Stop.
* **Unbounded leaves**: 667 of 2600 leaves have `system_lifetime = 0`. They emit until Stop; this is how vehicle smoke and fire work (the owner Plays the controller and Stops it).

## 5. Gates and limits (native unless marked)

1. **Emitter pool**: 1024 (0x400) emitters, density scale `max(0.25, 1 - (live+1)/1024)` (already in the runtime). Linked systems and trails are emitters.
2. **Global particle gate**: the job context (0x00a62520, built once per frame) carries the sum of the live particle counts of all emitters (+0x74 of each); a job births only if it is below **0x4000 = 16384**.
3. **Camera distance gate**: the context carries a position taken from `[[DAT_011919f4 + 0x10] + 0x54 .. 0x5c]` (inferred: the render camera position; the getter was not traced). A job births only if the squared distance
   from the emitter's matrix translation to it is below the leaf's `#d1e88637` squared. Default (0x00a62710 stores 800.0): **800 m** (2200 leaves state it, 61 omit it); retail range 10 m to 5000 m: 5000 on 82 leaves, 2000 on 13,
   50 on 45, 100 on 41, 30 on 30, 25 on 26, 15 on 20, 10 on 8. Existing particles keep simulating when the camera moves away; only births stop. (Draw-time distance alpha is in the vertex shader, see `particle-consumers-next.md`.)
4. **Wind**: absent. Searches of the executable for wind find only `input_wind_max_delta`; the job's modifiers are `PhysicalForce` (a fixed vector times dt) and `PhysicalDamping`. A port must not add drift to smoke.
5. **Capacity per emitter** (0x00a71720 tail): `cap = authored (#74175c29, default 100)`; `steady = int(ceil(life + 0.25 * lifetime_variation) / spacing)` with `spacing = 1/rate` in f32 (leaf+0x154); if `0 < steady < cap` the cap becomes `steady`; with debris slots (`mesh_N`/`physics_N`) it is at most 150; at most 1000.
   The probe counts 200+ retail leaves whose cap this lowers (casino smoke 60 -> 59, fuel depot smoke 02/03 -> 44/120 or 89).
6. The density-scale cap and spacing division, the survivors-then-births job order and the strict `accumulator > spacing` birth rule are as in `effects-pipeline.md` section 11 and `particle-consumers-next.md`.

## 6. Where smoke comes from in the effect data

17 effects reach a smoke leaf (name contains `smoke`) directly or through linked systems (probe). All other smoke-looking output is inside chains of leaves without that word (fire, `exp_*`, trails, `fx_fire_missile_*`).

| Group | Effect ids | Leaf (chain) | Notes |
|---|---|---|---|
| Vehicle damage smoke | 150 `lave_damage_smoke`, 241 `_large`, 166 `lave_mc_damage_smoke`, 167 `arve_smoke`, 385 `seve_damage_smoke_large`, 388 `arve_damage_smoke_large` | `fx_vehicle_smoke_medium_01/_large_01` (heat-haze head) + 2 smoke leaves; `fx_vehicle_mc_smoke_01` + `_02`; `fx_vehicle_arve_smoke_01` + `_02` | persistent controllers (vehicle slot 12); unbounded; **no input bindings**; ids per vehicle in `vehicle-fire-explosion.md` |
| Smoke burst | 153, 389 | `fx_vehicle_smoke_burst_01` (0.25 s particles, window 0.2 s) + `_02` (`muzzleflashes.dds`, window 0.1 s) | vehicle slot 14, 0.8 % per frame while the damaged engine runs |
| Environment | 281 `env_gunsmoke` (input `impulse`, no binding), 274 / 282 casino smoke, 341 `fx_env_gb087_smoke`, 236 `cut_trucksmoke`, 444 `cs_km06_dome_smoke_01` (looping, cull 2000 m, cap 500) | one leaf each | only 341 is placed by a `CEffectPointEmitter` (100x); 281, 274, 282, 236, 279, 444 have no placement (cutscene or script producers, not traced). Particle and sound components only: **no smoke effect carries a light**; the fuel depot fire/explosion effects (447, 126) do |
| Fuel depot | 126 `explosion_resource_fueldepot` (+ `_smoke_01..05`, 4 s light), 447 `fueldepot_fire` (looping, light) | `fx_exp_resource_fueldepot_smoke_01` -> `fx_fire_fuel_large_01` (6 linked, one with a trail) | placed 477 and 634 times by `CEffectPointEmitter` (events `::fuelDepot.dest.delay`, `::tankDestroyed`) |
| Placed steam and vents | 177 `env_steam_large` x23, 186 `env_steam_medium` x366, 242 `env_pipeline_ventpop` x168 | steam leaves | placed `CEffectPointEmitter`s with start/stop events |

Other sources that carry smoke inside the chain: the muzzle effects (`weapon-fire-effects.md`: minigun smoke 03, shotgun smoke 01, revolver `fx_muzzle_gun_03`, grenade launcher smoke children; done in FX-GUN-1),
rocket and missile trails (`fx_fire_missile_01/02/03`: blob head, smoke 500 particles, 5 s), car trail fires (`fx_fire_car_trail_03`), explosion plumes (`exp_*`).

## 7. World particle objects: chimneys, damaged-building smoke (native loader, census)

`CParticleObject` (vtable 0x00eb990c; loader slot 6 = 0x00649f10) is a game object in the location data that names a leaf directly (no effect id):
* `#bf0369b0` the leaf (`fx_env_chimney_large_01.psm`; directory and extension stripped, hashed to a resource id, resolved with 0x00a6a470);
* `#122b5b13` -> byte at +0xa8, constructor default **1**: **autostart** (inferred name; the data supports it: 660 of the 666 objects with the byte set have no start event, 6249 of the 6250 with it clear have one); `#75d345ca` -> +0xa9 (0 on every retail object; SetMatrix 0x008466e0 then keeps only the translation);
* three event strings `#d2ab39ea` (start), `#904273ab` (stop), `#d3e83257` (third event, set on 345 objects: role open) registered through 0x004e2eb0 into the object's event slots (+0x9c, +0xa0, +0xa4);
* the matrix at +0x38 (SetMatrix). Changing the name stops the emitters of the old leaf (0x00a6ca40: every emitter with this data and this matrix pointer gets a stop).
Census (probe `--census`, 1682 containers): **6916 objects, 75 distinct leaves**. Autostart (`#122b5b13` = 1) holds for 666 objects; the exceptions to "autostart = no start event" are 6 objects with both and 1 object with neither. Smoke related:

| Leaf | Objects | Start event | Where |
|---|---|---|---|
| `fx_env_smoke_damage_large_01` (life 5 s, window 6 s, 20 particles) | **1305** | none autostart; all 1305 have a start event: 425 `destroyed`, 248 `dac.jammer.destroyed`, 560 `explosion1..5` (112 each), 44 `generator_destroyed`, 28 other | 1123 world chunks, 182 mission and stronghold containers |
| `fx_env_chimney_idyllic_medium_01` / `_large_01` / `_medium_01` | 222 / 34 / 27 | mostly none (autostart: 219, 23, 15); mission-scripted ones start on events such as `bridgeSmoke`, `startExplosionSequence`, `spawned` | factionmissions 148, wc 108, metropolis 20 |
| `fx_km03_smoke_medium_01` | 116 | 50 on `::drop` (stop `::completed`), 66 autostart | harbour and stronghold buildings |
| `fx_env_smokeyarea_small_01` | 23 | 7 event driven | missions |
| `fx_env_steam_large_01` / `fx_env_pipeline_ventpop_360_01` / `fx_f2m07_vent_smoking_01` | 16 / 8 / 2 | `playSteam01`, `smoke` events | f1m07 |
| `fx_fire_missile_01` (+ smoke chain, "smokeTrail") / `fx_trail_fire_small_01` / `fx_fire_car_trail_03` | 79 / 48 / 50 | barrel `dac.barrel.destroyed` (stop `tit.barrelSmoke.stop`), `destroyed` | explosive barrels and wrecks |
| `fx_f2m06_smokeplume_01`, `fx_exp_resource_fueldepot2_smoke_01`, `fx_km03_junglefog_01`, `fx_env_milehighclub_fog_02` | 2 / 1 / 5 / 1 | autostart | f2m06 camp fire, km03 |

(The most numerous object, `fx_env_wooden_fence_01` x2089, is destruction debris, not smoke.) The CParticleObject start/stop plumbing (the event slots' handlers and the spawn call for autostart objects) was not traced: the class has only the
loader, SetMatrix and destructor in its own vtable (Gap 1). Leaf properties of the smoke leaves: all use `fumefx_bomb_01.dds` (807 leaves share this atlas, fire included), life 1 to 14 s, 12 to 500 particles.

## 8. Input bindings of particle variations (native, answers gap 5 of `vehicle-fire-explosion.md` for smoke)

The particle component's SetInput (0x007761a0) finds the effect's input value for a key, finds the variation record, and drives up to five emitter parameters, each selected by an **integer input key** stored in the variation
(`#d623c711` -> emitter+0xa4 = base(+0x144) * v; `#85ee8f21` -> +0xa8 = base(+0x148) * v; `#d9fad10c` -> +0xac (the system scale that multiplies every opacity change) = raw value in [0,1];
`#7c4a3b08` -> emission spacing = base spacing / v (v <= 0 stops births); `#67867668` -> +0xa0 = v), applied to the emitter and every linked child.
`v = clamp((value - min) / (max - min), 0, 1)` raised to the floor `#4de5cc5c` (range fields `#fc405fe1` default 20 and `#3c9be34c` default 100); for key 0 the value is gated: below `min`, or above the cut-off `#9b356e5f` when it is > 0, gives 0.
Retail: 86 variations carry a binding, keys only 0, 1, 2. They are wheel rolling / skidding / spinning dust, collision impacts and slides, boat wakes and drives, tail pipes, jets, `char_speed_ambience`, fire 386 and tail fire 235.
**None of the smoke damage effects binds anything**; fire 386's moving variation scales its rate with speed squared between 10 and 70, its stationary variation (`fx_vehicle_fire_stationary_01`) is on only while speed squared is in 0..30.
The vehicle owner passes key 0 = squared speed (`vehicle-fire-explosion.md` section 2). How the keys 1 and 2 of the tail pipe effects (`opacity`, `velocity`) and the named inputs of the `inputs` block map to the integer keys is open (Gap 2).

## 9. Lighting, fog, distance draw, draw passes

* Fog on particles (`particle` VS/PS with the haze factor) is applied since JC2-163 (`haze-fog-next.md`); smoke uses the ordinary Particle shader: dual-source alpha blend, no lighting term traced in the PS, vertex colour with the emissive modifier.
* Leaves with `distort_normal.dds` (129 leaves, render flag bit 13) are the heat-haze heads of vehicle smoke (4 particles); the visible smoke is the linked leaves, so the current "distortion leaves simulate but are not drawn" rule loses no smoke.
* Remaining lighting/soft-depth/PS arithmetic is task 8bb6ab45. The vehicle and environment smoke effects have no light component (only particle and sound); the fire effects carry the orange light.

## 10. Current runtime versus native (updated by FX-SMOKE-1, task 484554ae)

`effects/particle.rs` + `particle_sim.rs` now follow sections 2 to 5:
* linked systems: all started with the head (no depth limit and no repeat guard; the pool of 1024 is the only bound, deepest retail linked tree: 5 levels, up to 33 links per leaf), with the head's duration, flags 0 (never self-release, a stop or the head's finish stops them), and every `SetMatrix`/`SetVelocity` of the head moves them (the native shared matrix pointer);
* trails: leaf key `#1de3fca7` parsed (`ParticleSystem::trail`); a trail emitter per newborn particle, placed at the particle (source orientation, **inferred**), moved with it, stopped with it (death, emitter stop) and receiving the particle's alpha as system scale (the scale multiplies every opacity modifier step, native +0xac; where the draw path reads it is not traced); trail chains are stepped by the owning emitter and count in the pool;
* window and stop: window = variation `duration`, else `system_lifetime`, 0 = unbounded (the one-particle-life stand-in is gone); a stop only sets the flag (unbounded: window closed at once, bounded: window kept), live particles are never removed; `Stop(immediate)` and dropping the component hand the emitters to the manager (`update_detached`, run from the plugin) instead of killing them; release needs the flags byte `#b0124c0a` (default 1; read from the variation) or a stop, a closed window and no particle;
* gates: first-frame hold (no births in the first job), global 16384 particle gate, cull distance `#d1e88637` (default 800 m) against the camera (inferred origin, Gap 3), capacity rule `effective_capacity` (authored, steady state, 150 with `mesh_N`/`physics_N` slots, 1000). The start-delay fields are unknown (Gap 3): delay 0.
Still absent: input bindings (FX-SMOKE-2).

## 10b. World particle objects at runtime (FX-SMOKE-3, task afcac2d9)

`jc2_formats::placed_particle` parses every `CParticleObject` (leaf, autostart default 1, translation-only byte, three event strings, composed matrix; the matrix key `#6ca6d4b9` is the ordinary `world`). `jc2-game/world_particles.rs` gives each object an entity owned by the tile that placed it (`world::place_objects`, also exported entities with their base transform), so a tile unload drops it and the drop stops its emitters (native destructor 0x00a6ca40; particles already flying run out in the manager).
* **Native**: leaf name stripped of directory and extension; autostart byte semantics from the data (660 of 666 autostart objects have no start event); the destructor stop.
* **Wired (inferred)**: autostart objects start when their tile loads, flags 1 and duration 0 (as the other world emitter callers); event objects start on a unit-destroyed event. Evidence (retail test over 22x19): the damage smoke's start string (`e{go043h.10x10({GUID})}::destroyed`) is exactly one whitespace token of the `on_destroy` string of the building's damage control, which `UnitDestroyed::fires` already carries; stop strings are matched the same way (the chimneys of `f2m05.stffront` stop on `::destroyed` of their building). Tokens are matched per tile (exported entity copies sharing a GUID would all start: not distinguished).
* **Idle by design** (no producer of the event exists yet): mission script events (`bridgeSmoke`, `::drop`, `playSteam01`, `startExplosionSequence`), barrel stage events (`::fire`, `dac.barrel.destroyed`), the third event slot (role unknown, Gap 1), the +0xa9 translation-only byte (parsed and honoured by `matrix()`, 0 on every retail object). The pool of 1024, the global gate and the 800 m birth gate are the manager's.
* Capture hook: `JC2_NO_WORLD_PARTICLES=1` leaves the objects inert (before/after captures). Plume size and velocity still wait for dc95ceb0: the chimney wisps are faint.

## 11. Paired implementation tasks (filed on the board, status `ready`: FX-SMOKE-1 `484554ae`, FX-SMOKE-2 `afa2fb7e`, FX-SMOKE-3 `afcac2d9`; gap tasks: Gap 1 `c90b9918`, Gap 2 `b92ead43`, Gap 3 `d893931d`)

* **FX-SMOKE-1** native emitter lifecycle in the particle simulation (sections 2 to 5): shared live matrix for linked systems and trails, trail emitters, the stop/release/window rules, the cull distance, the global particle gate and the capacity rule, `Stop(immediate)` without kill. Owned: `crates/jc2-game/src/effects/particle*.rs`, `crates/jc2-formats/src/particle.rs` (trail name, cull distance, flags). Needs only the merged spawner and renderer. Independent of the units task dc95ceb0 (lifecycle, not size or velocity); visible parity of the plumes needs dc95ceb0.
* **FX-SMOKE-2** particle input bindings (section 8): the five bindings, range/floor/cut-off, applied to the emitter and its linked children; key 0 (`speed`) is native-proven; keys 1 and 2 wait for Gap 2. Owned: `effects/particle.rs`, `effects/system.rs` SetInput forwarding, `jc2-formats/src/effect.rs` (variation fields). Needs only the merged spawner.
* **FX-SMOKE-3** world particle objects (section 7): parse `CParticleObject` from the location containers (leaf name, autostart, three event strings, matrix), spawn the autostart ones when their tile loads (283 chimneys, 116 km03 smoke, steam, camp fires), start the event driven ones from the existing destruction events (`::destroyed` of the building: 1305 damage smoke objects) and stop them with their stop event. Owned: new `crates/jc2-formats/src/placed_particle.rs`, new `crates/jc2-game/src/world_particles.rs`, hook in `destruction.rs`. Needs the merged renderer; the event plumbing is shared with FX-EXP-3 (d5ea23ca, backlog): coordinate. Visible size/velocity parity waits for dc95ceb0.
* Vehicle smoke and fire controllers: already filed as **FX-VEH-1** (6db149c5, backlog). Corrections for it from this document: the damage smoke effects have no input bindings (key 0 is unread by 150/241/151/240/166/167/385/388), `Stop` does not kill particles, and the slot effects are unbounded leaves that must be Stopped when the smoke flag clears. The orchestrator should promote it once FX-SMOKE-1 is in.

## Gaps (unresolved; none guessed)

1. **CParticleObject triggers**: the handlers of the three event slots (+0x9c/+0xa0/+0xa4, registered by 0x00649f10 through 0x004e2eb0), the spawn call of an autostart object (none exists in the class's own methods), the role of event 3 (`#d3e83257`, 345 objects) and of the byte at +0xa9. Blocks: exact start/stop timing of chimneys, damage smoke, steam.
2. **Effect input keys and binding targets**: how the named `inputs` block of an effect file maps to the integer keys of the input map (keys 1 and 2 on the tail pipe effects 371 to 376, 41, 42) and what the targets +0xa4/+0xa8/+0xa0 of the emitter drive (speed, size or opacity scale; worker +0x94/+0x98/+0x90 consumers in the job). Blocks: exhaust smoke, wheel and boat spray at speed.
3. **Emitter job context**: which object `[[DAT_011919f4+0x10]+0x54]` is (camera or player position) and the role of the context constant 200.0 (0x01366848, read by the per-particle update through context word 0x406); the start-delay fields of the leaf (base/variation at +0x14c/+0x150) and the key names (the same open item as Gap 1 of `particle-units-next.md`); flag bit 12 (set from `DAT_0108aafa` for spaces below 2; inferred: source position interpolation). Blocks: exact cull behaviour near the camera, burst timing.
4. Smoke draw details stay with task 8bb6ab45 (lighting, soft depth, PS arithmetic) and the unit questions with dc95ceb0 (size/velocity unit, fitted quads, shapes). Not repeated here.
5. Key names of the leaf: `#1de3fca7` (trail system), `#d1e88637` (cull distance), `#b0124c0a` (flags) are unresolved hashes; semantics are proven, spellings are not.
6. The `max_instances` throttle (10 for most smoke effects) still has no reader (be538832).

## Reproduce

```
python -B research/tools/smoke_effects_probe.py --game "<game folder>" --tool <jc2tool.exe> --pseudo MAIN/research/analysis/pseudo/_unnamed [--census]
```
Without `--tool` the executable checks run alone; `--census` (about 3 minutes) adds the container counts of section 7. 22 cited functions are checked by sha256; the statements of the decompiler export are
asserted with `--pseudo`; the models (window, stop, finish, capacity, input scale) are executable references with fixtures.
