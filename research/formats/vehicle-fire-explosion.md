# Burning and exploding vehicles: effect slots, damage states, death sequence, wreck (board task bd393831, 12 October 2026)

Research only. Probe: `research/tools/vehicle_fire_explosion_probe.py` (98 checks, exit 0; run with
`--game GAME --tool jc2tool --pseudo EXPORT`, add `--json` for the per-vehicle table; the disassembly checks need the `capstone`
Python package and are reported as skipped without it). Executable SHA-256 `df99a479...f74`.
Builds on `explosion-effects.md` (task 55a08887: effect graph components, force pulse, the code-built death pulse and flash),
`effects-pipeline.md` (controllers, `Play`, `SetMatrix`, `Stop`, `SetInput`, section 5.4), `point-light-model.md` and
`vehicle-effects-selectors-next.md`. This document answers the open Gap 1 of `explosion-effects.md` (which effect each vehicle class
plays) and adds the damage state machine, burn timers, wreck look and the death-flash analysis (board task 6c837ac1, G9).
"native" = read from the executable or the retail data and asserted by the probe; "inferred" is said so. Addresses are retail
virtual addresses. Names of functions and fields are working labels (the executable has no symbols besides RTTI).

## 0. Answers in one table

| Question | Answer |
|---|---|
| Where are the per-vehicle effect ids? | In the vehicle's **paper doll** inside its export archive (`<name>.ee`), file `*.mvdoll` (80 vehicles) or `*.vdoll` (9): the body `CPartProp` has a child list of `CEffectProp` nodes, each with an **effect id** (key `#8370ea1c`), a **slot** (key `#9cd34fdc`) and a local transform. The earlier search of `.epe` and `.bin` files could not find them because the doll is a separate archive member. Section 1. |
| What are the slots? | The enum string at 0x00e5ee60: 1 Explosion, 2 EngineIdle, 3 EngineRunning, 4 Contrail, 5 WaterExplosion, 8 ShotdownEffect, 9 WaterFrontDrive, 10 WaterFrontCollision, 11 WaterSideTrails, 12 EngineSmoking, 13 EngineBurning, 14 EngineSmokeBurst, 15 EnginePostExplosionFire. Section 2. |
| Explosion by class | Cars 128 (37 vehicles), 152 (8: buses, APC, tank, trucks), 164 (5: tuk-tuks, ATVs); motorbikes 164; boats 156 (12) or 152 (1); helicopters 157; planes 43 / 128 / 155. Section 3. |
| Fire, smoke | Smoke slot 12 (150, 241, 166 bikes, 167 air, 385 boat), fire slot 13 (151, 240, 165 + 235 bikes, 386 boat, 406-409 air), smoke burst slot 14 (153 on every land vehicle and boat), post-explosion fire slot 15 (405-409). Section 3. |
| When does a vehicle smoke, burn, explode? | Health ratio below the **smoke threshold** (data key, 0.4 / 0.6): smoke. Below the **burn threshold** (0.1 / 0.2 / 0.25, one car 0.01): fire, and a burn timer of **8 s** (9 cars: 4 s) runs; when it expires the health is set to 0. Health 0: the death routine. Section 4. |
| After the explosion | The slot 15 fire (and the flags) run for **8 s** (every vehicle), the body parts and wheels get a dark tint, three wheels are thrown, the code pulse and flash fire. The vehicle object stays (no destruction in the death routine). Sections 5 and 6. |
| Water | Above 80 % submerged (cleared below 60 %) smoke and fire are forced off and the explosion plays the slot 5 effect instead of slot 1; only air vehicles have slot 5 (effect 44), so a submerged car or boat explosion plays **no** blast effect. Section 4.3. |
| Death flash radius (G9) | Still not derivable statically; the analysis narrows it (section 7). |

## 1. Where the ids live (native, data)

Every vehicle export (`exported/vehicles/<class>/<name>.ee`) holds a property file `*.mvdoll` or `*.vdoll` (the paper doll,
class `CVehicleProp`). Its body part (`CPartProp` named like `V014-Body`) owns a node whose children are `CEffectProp` records: a
`local` and `world` 4x3 matrix, the effect id `#8370ea1c` and the slot `#9cd34fdc`. Both keys are the data words
`DAT_00f0bc78` / `DAT_00f0bc7c`, read by the part-effect loader 0x0086e4d0 into 0x48-byte records (id at +0, slot at +4, matrix from +8).
The slot enum string at 0x00e5ee60 belongs to the same editor descriptor (pointer at 0x01092ab4).
Census (probe): 90 vehicle archives; 89 have a doll (`arve.v062_chopper_mg` has none and plays no effects); a vehicle has one node per mount point (the jeep: explosion 128 at the body origin (0, 0.89, 0),
smoke 150 at (-0.98, 0.88, -1.32), burst 153 at (0.98, 0.88, -1.32), tail pipes at (+-0.34, 0.37, 2.15)).
The `.epe` of the vehicle carries the **tuning** keys (section 4.2), not the ids.

## 2. How the runtime uses the slots (native)

* The vehicle builder (vtable slot 70 of `CVehicle`, 0x00ebf51c + 0x118) walks the doll's effect list once at creation and, for each record, gets the controller
  (`0x0062a7a0(id)` then the shared-pointer assign 0x00523240; see `effects-pipeline.md` 5.3) and appends a **0x4c-byte instance**
  (transform + controller handle at +0x44) to a **per-slot vector** in the vehicle. Records of other slots are skipped (default case).
  Base builder **0x0067a8f0** (land: `CLandVehicle`, `CCar`, `CMotorBike`; also called first by the others): slot 1 -> vector at vehicle +0x284, 2 -> +0x264, 3 -> +0x274, 5 -> +0x294,
  12 -> +0x2a4, 13 -> +0x2c4, 14 -> +0x2b4, 15 -> +0x2d4 (begin pointer at offset + 4; probe checks the jump table at 0x0067ad48 / 0x0067ad6c and the offsets).
  Air builder **0x0079d220** (`CAirVehicle`, `CHelicopter`, `CAirPlane`): base, then slot 8 -> +0xbd0. Sea builder **0x007ef910** (`CSeaVehicle`, `CBoat`): base, then slot 9 -> +0xc80, 10 -> +0xc90, 11 -> +0xca0.
  Slot 4 (Contrail) has no handler anywhere (retail value is -1 on 7 planes).
* All slot effects are **persistent controllers**: every frame the owner calls SetMatrix (0x00631d00: local transform composed with the vehicle matrix at +0x2e8), starts
  the effect with `Play(0)` (0x0072beb0) when it is not running (0x005141f0 tests that the controller has an active effect), and ends it with `Stop(0)` (0x00544950, graceful).
  Smoke and fire controllers also receive the **velocity** (0x00780690 -> SetVelocity 0x00735760) and **named input 0**
  (0x00638f00 -> SetInput 0x006f67d0) set to the vehicle's squared speed `vx*vx + vy*vy + vz*vz` (values below 0.001 become 0; note: squared, not the speed).
  The effect files declare the input (`lave_damage_smoke.bin`: `inputs { speed = 0 }`); what the particle systems do with it is not traced (Gap 5).
* Per-frame owners (CVehicle vtable): slot 65 = **0x00516640** (smoke: slots 12 and 14, argument = the "smoking" byte vehicle +0x35a), slot 66 = **0x0063fda0** (fire: slots 13 and 15, arguments = the "burning" byte +0x35b and the "exploded" byte +0x35c),
  both called from the vehicle visual update 0x006a0e80 (CVehicle slot 26; whether the class overrides of that slot reach it was not checked). Both force their first argument to 0 when the byte at +0x361 (submerged, section 4.3) is set.
  * 0x00516640 with smoking set: slot 12 effects run (transform every frame, `Play`, velocity, input 0). With smoking clear: slots 12 and 14 are stopped. In addition, while smoking and the engine-on byte +0x35e is set,
    each call draws `rand() % 500` and, when it is above 0x1ef (495, i.e. **4 in 500 calls, 0.8 % per call**), runs the slot 14 effects the same way: the smoke burst (153) fires at random while the damaged engine runs.
  * 0x0063fda0 with `exploded` = 0: slot 13 effects run when burning is set, otherwise stopped. With `exploded` = 1: slot 13 is stopped and **slot 15 runs** when burning is still set (otherwise stopped).
* The engine-on byte +0x35e is set by `CLandVehicle` slot 83 (0x006faac0, "engine started"); the smoke burst therefore needs the engine running.
* Engine idle/running (slots 2 and 3, effects 372/374/376 and 371/373/375 tail pipes, 41/42 jet thrust), water trails (9, 11) and contrails (4) have per-frame owners that were not traced (Gap 4); the table is in section 3.

## 3. Effect ids per class and per vehicle (native, data)

Counts are vehicles. "Light" is the effect's own light component (peak colour, multiplier, radius, last key time). Effects carry **no force pulse and no camera shake**:
the vehicle blast pushes and shakes only through the code-built pulse (`explosion-effects.md` section 4: extents 7 m, A = B = 300, C = 500, so the shake reaches 21 m).

### 3.1 Explosion (slot 1) and water explosion (slot 5)

| id | effect | sound (bank/id) | particle systems | light | used by |
|---|---|---|---|---|---|
| 128 | explosion_lave | 37/57 | `exp_vehicle_lave_medium_01` | x8 r12, 1.3 s | 37 `CCar` (sedans, trucks, jeeps, sports cars, buggies, tractor, limo), planes `v010_personal_propellerplane`, `v067_speedplane` |
| 152 | explosion_lave_large | 37/63 | `exp_firebomb_large_01` | x8 r20, 1.5 s | 8 `CCar` (`v012` tank, `v016` APC, `v037`, `v044`, `v045` firetruck, `v046` coach, `v048` bus, `v050` super truck) and `seve.v098_indo` |
| 164 | explosion_mc | 37/57 | `exp_vehicle_lave_mc_01` | x8 r8, 0.8 s | 9 `CMotorBike` and 5 `CCar` (tuk-tuks `v005_*`, ATVs `v056_*`) |
| 156 | explosion_seve | 37/64 | `exp_vehicle_explode_boat_01` + `exp_vehicle_lave_medium_01` | peak (1.0, 0.5, 0.0) at 0.1 s, derived radius, 1 s | 12 `CBoat` (`v011_*`, `v082`, `v083`, `v087` to `v089`, `v095`, `v104`, `v105`) |
| 157 | explosion_arve_helicopter | 37/65 | `exp_vehicle_helicopter_2r_01` | x8 r27, 1.3 s | 8 `CHelicopter` |
| 155 | explosion_arve_plane_large | 37/67 | `exp_vehicle_explode_plane_large_01` | x8 r70, 2 s | `v072_cargo_plane`, `v114_jumbojet`, `v076_jet_blackplane` (with 43) |
| 43 | arve_land_explosion | 37/63 | `exp_vehicle_explode_ground_large_01` | x8 r30, 1.3 s | `v064_personal_jet`, `v071_jet_fighter`, `v076_jet_blackplane` |
| 44 | arve_water_explosion (slot 5) | 37/63 | `water_splash_big` | none | every air vehicle that has a doll |

No slot 1 effect: `arve.v062_chopper_mg`, `arve.v062_cutscene` (no effects at all), `seve.v086_barge` (smoke and fire only). Effect 154 (`explosion_arve_plane`) is never referenced by a doll.
`seve.v098_indo` uses 152 (boats: 12 x 156, 1 x 152, the barge none).

### 3.2 Damage smoke (12), fire (13), smoke burst (14), post-explosion fire (15), shot-down (8)

| family (vehicles) | slot 12 smoke | slot 13 fire | slot 14 burst | slot 15 post-explosion fire |
|---|---|---|---|---|
| cars, standard (30: sedans, jeeps, sports cars, small trucks) | 150 `lave_damage_smoke` | 151 `lave_damage_fire` | 153 | 406 `fire_lave_medium` |
| cars, large (5 trucks/buses, `v015`) | 241 `lave_damage_smoke_large` | 240 `lave_damage_fire_large` | 153 | 406 |
| cars, heavy (4: `v016` APC, `v046`, `v048`, `v050`) | 241 | 240 | 153 | 407 `fire_lave_large` |
| cars, tank-class (3: `v012`, `v037`, `v044`) | 241 | 240 | 153 | 406 |
| tuk-tuks and ATVs (5) | 150 | 151 | 153 | 405 `fire_lave_small` |
| tractor, armoured transport (2) | 241 | 151 | 153 | 406 |
| firetruck (1) | 150 | 151 | 153 | 407 |
| motorbikes (9) | 166 `lave_mc_damage_smoke` | 165 `lave_mc_damage_fire` (+ 235 `lave_mc_damage_tailfire` on 8) | none | 405 |
| boats, large (6: `v011_*`, `v088`, `v105`) | 167 `arve_smoke` | 406 | none | 406 |
| boats, speed (5: `v082`, `v083`, `v087`, `v089`, `v104`) | 150 | 151 | 153 | 406 on `v083` only |
| `seve.v095_swamp_boat` / `v086_barge` | 150 | 151 / 406 | none | none / 406+407 |
| `seve.v098_indo` | 385 `seve_damage_smoke_large` | 386 `seve_damage_fire_large` | 153 | none |
| helicopters (8) | 167 | 240 (6) or 408 `fire_arve_medium` (2) | none | 408 |
| planes (7) | 167 (`v010`: 150) | 151 (`v010`), 406, 407, 408, 409 | none | 408 or 409 `fire_arve_large` |

Slot 8 (ShotdownEffect, air only): 87 on 14 of the 15 air vehicles that have effects (`#4844f203`, `fx_exp_vehicle_broken_air_fire_01`, sound 37/18); `v064_personal_jet` has 167 and 240 instead. It starts when an air vehicle begins to burn (section 4.2 step 2) or is shot down.
Effect contents: 150/241/166/167/385 are smoke particle systems (`fx_vehicle_smoke_medium_01`, `_large_01`, `fx_vehicle_mc_smoke_01`, `fx_vehicle_arve_smoke_01`) with sound 37/49 on the land ones; 151/240/165/386 are fire systems
(`fx_vehicle_fire_01`, `_large_01`, `fx_vehicle_mc_fire_01`) with sound 37/4 and an orange light (peak at 0.45 s of colour (0.4, 0.21, 0.0), derived radius, last key 0.8 s; 165 and 386 peak at t = 0 with (0.5, 0.4, 0.2) and (0.6, 0.4, 0.1)); 153 is `fx_vehicle_smoke_burst_01` (sound 37/56);
405-409 are `fire_lave_small/medium/large_01` and `fire_arve_medium/large_01` (sound 37/4, light peak at 0.45 s of colour (1.0, 0.5, 0.0) x 4.75 with radius 1.5 / 5 / 8 / 5 / 8 for 405 / 406 / 407 / 408 / 409, last key 0.8 s). Effect 150 has `max_instances` 10 (`#488c291b`).
Engine slots: 2 idle 372 / 374 / 376 (light / medium / heavy tail pipe), 3 running 371 / 373 / 375, jet 42 idle and 41 running; boats 107 rear drive, 105 front drive, 106 wake, 108 / 275 side drive.
Probe `--json` lists every vehicle's slot table, class and tuning values.

## 4. The damage state machine (native)

### 4.1 Where it runs

`CVehicle` update (vtable slot 27, 0x00782c90) calls **0x00572770** (also called directly from 0x0056d890, 0x00627610 and 0x007d0de0, the per-class update functions) every frame with the frame time.
Health is a pair of floats at vehicle +0xe4 (maximum) and +0xe8 (current); all retail vehicles have `health = 100`, so the ratio is health / 100. 0x007e87a0 is `SetHealth(value)`: it saves the old
value, stores the new one clamped to the maximum, and when it is <= 0 for the first time sets the dead byte (+0x40) and calls vtable slots 24 (0x006b61e0, posts the game event **`gsy.object_destroyed`**) and 25 (0x0067a010).

### 4.2 The sequence (ratio r = health / maximum)

1. **Smoking.** If neither burning (+0x35b) nor dead (+0x40): when not yet smoking and `r < smoke_threshold` (vehicle +0x330) the vehicle calls vtable slot 76 (0x007d8400), which only sets the byte +0x35a. The same step also runs
   0x0079e310: if the vehicle is **upside down** (matrix element at +0x2fc below -0.5, inferred as the up axis) and **nearly stopped** (speed squared < 20, i.e. below 4.5 m/s), not submerged, it accumulates time at +0x484; when the sum passes +0x488 it sets the health to
   `burn_threshold * max - 1` (so a flipped vehicle starts to burn). The accumulator is reset otherwise; +0x488's data source is not traced (Gap 6).
2. **Burning.** When `r < burn_threshold` (+0x334): smoking is cleared (+0x35a = 0); if the vehicle is not burning and has not exploded (+0x35c) it calls vtable slot 77 (0x005b21c0 sets +0x35b = 1; **air vehicles use 0x00765c60, which also sets it and then calls vtable slot 91, the shot-down starter 0x0073d740, which plays the slot 8 effects** unless an object at +0xc6c reports a state through 0x00991830);
   the **burn timer** at +0x33c advances by the frame time; when it exceeds `burn_seconds` (+0x340) it is clamped to it and `SetHealth(0)` is called every frame, i.e. the vehicle dies when the timer expires.
   The timer never decreases (no repair path was found), so a vehicle that dips below the burn threshold always dies after `burn_seconds` of cumulated time below it.
3. **Death.** When `r <= 0` and the exploded byte (+0x35c) is clear: the vehicle sets +0x35c = 1 and +0x362 = 1 and calls vtable slot 78 with arguments (1, 1, 1, 1) (section 5).
4. **After the explosion.** The post-explosion timer +0x344 advances; when it reaches `post_explosion_fire_seconds` (+0x348) the bytes +0x35a and +0x35b are cleared, which stops the slot 15 fire (0x0063fda0 with burning clear). The vehicle object is not removed.
5. **Water.** Each update reads a submersion ratio from the physics object (vtable +0xa8 of the object at the argument's +4). Above **0.8**, if the submerged byte (+0x361) is clear: +0x35a = +0x35b = 0, +0x361 = 1 and vtable slot 82 (0x148) runs;
   at or below 0.8, if +0x361 is set and the ratio is below **0.6**, +0x361 = 0 and slot 81 (0x144) runs. While +0x361 is set the smoke and fire owners (section 2) stop their effects.

Data keys (read by the vehicle loader 0x00780ab0, slot 6 of the secondary vftable 0x00e3c5dc; the loader's `this` is the vehicle + 4, established from the null-check pattern in the loader and because only this reading gives
the constructor defaults their meaning), all floats in the vehicle `.epe`; constructor 0x005cfe40 defaults in brackets:

| key | field | role (inferred name) | retail values (vehicles) |
|---|---|---|---|
| `#01429682` | +0x330 | smoke threshold (ratio) [0.4] | 0.4 (68: 26 + 42 stored as 0.40000004), 0.6 (22: boats, large air) |
| `#189e862f` | +0x334 | burn threshold (ratio) [0.1] | 0.2 (60), 0.25 (22), 0.1 (7), 0.01 (1: `lave.v020_sugga_blaine`) |
| `#4476a911` | +0x340 | burn seconds [8.0] | 8.0 (81), 4.0 (9: `v003_cover`, `_hardtop`, `_icecream`, `_icecreamdlc`, `_lorry`, `v015`, `v017`, `v023`, `v025`) |
| `#9011eda3` | +0x348 | post-explosion fire seconds [no constructor default] | 8.0 (90) |

The smoke threshold exceeds the burn threshold on every vehicle (probe). Fields are constructed as +0x33c = +0x344 = 0.
A 100-health vehicle therefore smokes below 40 hp (60 hp for boats and large aircraft), burns below 20 hp (25 hp) and then dies 8 s later, or earlier when damaged to 0.

### 4.3 Occupants and nearby objects

The damage update does nothing to the occupants. What kills them, and what happens to characters near the blast, is outside this routine (Gap 3): the death routine applies the 7 m force pulse (push and shake) and posts no area damage,
and no retail effect has an area-damage component (`explosion-effects.md` section 1).

## 5. The death routine (native)

`CVehicle` slot 78. Per-class entry points (probe, vtable slot 78): car and `CLandVehicle` **0x00647bd0**, motorbike **0x0080a7a0** (calls the car one, then applies the part tint of section 5.1 to two extra models at +0xea8 and +0xeac and calls 0x00445340),
helicopter and air **0x0079ed50**, plane **0x0061fad0** (first releases the seven lamp records, 5 + 2 of 0xb4 bytes, the same arrays the car update manages, then calls the air one), boat and sea **0x007ff860**, base **0x00559150**. Order:

1. Class prologue. Car: when the byte +0x36d is clear and 0x007990c0 returns true (meaning not traced) it posts the event **`government.landvehicle.destroyed`**; it then makes three passes over consecutive wheels (random start, wheel count at +0x370 as modulus),
   and for each wheel that is not already detached (byte +0x181 of the 0x208-byte wheel record) creates a simple rigid body for it and launches it (0x00612ac0: a value 20.0 (`0x41a00000`, inferred mass), the vehicle's velocity, and when the vehicle's speed squared is below 16 a random 0..1 scaling of a fixed vector, start time 0.05 s early) - **three wheels are thrown**. Air: event **`government.airvehicle.destroyed`**, stops the slot 8 shot-down effects, vtable slot 88 (0x160), two calls on the object at +0xc6c (0x009abda0, 0x009bbfb0) and 0x0067ca20. Sea: event **`government.seavehicle.destroyed`**.
2. Base 0x00559150: stops the engine sound component (0x0079da70), releases the vehicle's smart objects (seat triggers, 0x005d1320), sets a per-material float array to **1.0** on every mesh of the model
   (0x008c51f0, whose only caller is this routine; what it feeds is not traced, Gap 2), applies a dark tint to the vehicle body (vtable slot 85 with the constants of section 5.1), applies the same tint to every part model
   (vtable slot 31 = 0x00631710 with argument 1, over the 0x84-byte part list at +0x3a0 and the attached composite objects at +0x3c4),
   when the second argument is set sets two physics-body values to 1e7 (0x009beef0, 0x009bf9c0; meaning not traced), then deactivates attached weapons and mounted guns (vtable +0x60 / +0x94).
3. **Explosion effects.** When neither the submerged byte (+0x361) nor +0x35d (never set to 1 in the vehicle code) is set: every slot 1 instance is placed (SetMatrix with the vehicle matrix, 0x00631d00) and played (0x0072beb0). Otherwise the slot 5 instances
   are placed and played the same way. (The slot 1 loop contains a redundant second test of +0x361 whose else branch calls 0x0079f470; it is unreachable because +0x361 was just found clear.) A vehicle without the needed slot plays nothing.
4. A debug-draw cross of half-length 0.5 at the vehicle position (0x00c1efe0; *inferred* debug visual), then the code-built **force pulse** (extents 7, A = B = 300, C = 500, owner +0x36e = 11, applied through 0x0060f680, so it pushes bodies and shakes the camera inside 21 m)
   and the three-key **flash light** (section 7).

### 5.1 The charred look (native constants, inferred meaning)

Vtable slot 31 (0x00631710) with the flag set writes into each part model: 4 bytes (28, 28, 25, 252) at +0x24..+0x27 and 2 bytes (25, 255) at +0x20, +0x21 (data 0x01072298..0x0107229d), then two vec3 setters
(0x00740e20 / 0x00769150 for model type 10, 0x005de5b0 / 0x00583c30 for type 4) with **(0.094, 0.094, 0.094)** (0x010722a0) and **(0.078, 0.078, 0.078)** (0x010722ac), and sets a dirty bit. Vehicle slot 85 (0x006e15c0 for cars) writes the same
6 bytes into the four per-wheel model slots. The values are a near-black grey: the original chars the body, doors, hood and wheels by tinting, not by swapping models. The shader constants the two vec3 values feed were not traced (Gap 2).
The placeholder colour in `explosion.rs` (0.09, 0.08, 0.075) is close to these numbers but is not derived from them.

## 6. Burn and explosion timeline of a typical car (native sequence, retail values)

Health 100 -> hits bring it under 40: smoke (150) plus random bursts (153) while the engine runs; under 20: smoke stops, fire (151) starts, burn timer 8 s runs, the vehicle is still drivable;
timer ends or health 0: event `gsy.object_destroyed`, wheels fly, explosion 128 plays with its 12 m light and sound 37/57, the pulse shakes the camera inside 21 m, body goes charcoal, fire 406 burns for 8 s, the object stays.

## 7. The death flash radius (G9, board task 6c837ac1)

The three flash keys are built on the stack of 0x00559150 (frame 0x308 bytes, entered after `and esp, -16`). Words 0 to 9 and 11, 12 are written: position (vehicle position), colour (black / (2.0, 1.4, 0.6) / black), (c, l, q) = (0.1, 0.008, 0.04),
`explosion_effects_probe.py` additionally proves no instruction touches them). The three keys are copied by 0x00942ec0 (a thunk to 0x006f9dc0) into the light manager.
Findings that narrow the answer:

* The value is **stack residue**: dead stack of earlier, deeper calls on the same thread. The frame position relative to the caller depends on the stack alignment at entry, and the preceding calls differ by entry path (the three class wrappers above, called from 0x00572770 after
  0x0079e310 and the vtable calls), so no single static value can be derived. This is why the gap stays open.
* What the residue does if it takes common shapes: the interpolator 0x00959590 derives the radius from (c, l, q) (9.77 m) only if **either** neighbouring key is exactly -1.0; otherwise it interpolates the two residue words linearly (the keys' words are distinct slots, so the values may differ).
  The light manager drops any light whose radius is **<= 0.001** (0x0091dcc0). A residue of 0 or of a small integer or pointer reinterpreted as a float (all below 1e-30) therefore removes the flash entirely; a residue that is a plausible float gives a flash of that radius.
  Both outcomes are possible; which one occurs in retail cannot be decided without running the original.
* Visible consequence is small: the effect itself carries a stronger light (explosion 128: x8 r12 for 1.3 s; 152: r20; 157: r27; 155: r70), so the code flash is at most a short (1.5 s) extra warm pulse.
* Needed to close it (dynamic): break at the first `call 0x00942ec0` (0x0055a1b2, the keys are at ebp-0x140, ebp-0x10c, ebp-0xd8 with stride 0x34) in a vehicle explosion and read the dwords at ebp-0x118, ebp-0xe4 and ebp-0xb0, for each of the three class wrappers
  (car 0x00647bd0, air 0x0079ed50, boat 0x007ff860 / base) and for repeated explosions, to see whether the residue is stable.
  An implementation must keep the radius an explicit labelled unknown (default: skip the flash, or the derived 9.77 m flagged "non-native").

## 8. Current runtime versus native
(Written before FX-VEH-1; smoke thresholds, the burn timer, the slot 12 to 15 controllers, the burst, the post-explosion fire and the water behaviour are now implemented, see section 9; the rest still stands.)

`crates/jc2-game/src/explosion.rs` (`wreck_vehicles`, `burn`) blows a vehicle up at `hp <= 0` with an assumed blast (radius 9, damage 450, impulse 1500), evicts the driver, replaces every material with a
dark shared copy, smokes for 25 s with sphere puffs and plays the missile's light for any class. Missing: smoke/burn thresholds, the burn timer, per-class explosion and fire effects, the random smoke burst, post-explosion fire (8 s), water behaviour,
wheel debris, air shot-down, the events and the code pulse/flash. Retail `health` is 100 on all 90 vehicles.

## 9. Implementation status and the burn timer reconciled (FX-VEH-1, board task 6db149c5)

Code: `crates/jc2-formats/src/vehicle.rs` (`EffectProp`, `effect_slot`, `BurnTuning` with the four `.epe` keys), `crates/jc2-game/src/vehicle_damage.rs`
(`VehicleDamage::step`, `VehicleFx`, systems `damage_state` and `drive_effects`, observer `stop_on_remove`), `explosion.rs` (the 25 s sphere smoke is gone; `Wreck` is a plain marker).
Tests: formats `retail_effect_props` (jeep slots 128/150/151/153/406 and the smoke mount at (-0.98, ., -1.32), moped 166 without burst, helicopter 157 and 87, tuning keys);
game `smoke_then_fire_at_the_thresholds`, `burn_timer_kills_after_burn_seconds`, `explosion_then_post_explosion_timer`, `water_hysteresis`, `flip_burn_limit_is_unknown`,
`burst_rate_is_four_in_five_hundred` and the fixture `burning_fixture` (a car, a bike, a boat and a helicopter through the retail effect files; run with `-- --nocapture` to print the controller state of every mount per phase).

**Lifecycle of the controllers.** The slot 12 / 13 / 14 / 15 / 8 effects are unbounded leaves that never end by themselves (`smoke-effects.md` section 4), so `VehicleFx` creates each controller on first use, keeps the handle,
runs `SetMatrix`, `Play`, `SetVelocity`, `SetInput(speed squared)` every frame while the owner's condition holds, and calls `Stop` when it stops holding: smoke when `smoking` clears (burning starts, repair, water),
fire when `burning` clears or the vehicle explodes, the post-explosion fire when `burning` clears (the 8 s post timer), the slot 8 shot-down effect at the explosion or when burning clears; and `Stop(immediate)` on every running controller when the `VehicleFx` component is removed (despawn). Particles already alive run out on their own (native Stop never kills them).
Input 0 is set for every effect although the slot effects bind no input (`smoke-effects.md`), as the native owners do.

**Burn timer versus the native state machine 0x00572770.** Native (section 4.2): step 1 smoking, step 2 burning with the timer, step 3 death, step 4 post-explosion timer, step 5 water. The FX-VEH-3 burn timer (`burn_step`) is now `VehicleDamage::step` and differs from its first version as follows (all labelled in the code):

| point | native | first FX-VEH-3 version | now |
|---|---|---|---|
| burning flag | set below the burn threshold only while not burning and not exploded (+0x35c) | set always | as native (`exploded` = the `Wreck` marker) |
| smoking flag | set below the smoke threshold while neither burning nor dead; cleared by the burn step | none | as native |
| timer on the frame health reaches 0 | step 2 still runs before the death step, so `burning` is set and the timer clamps | skipped when health <= 0 | runs (so a collision wreck sets `burning` and slot 15 plays); `damage_state` is ordered before `wreck_vehicles` for this |
| timer after death | clamped, `SetHealth(0)` repeated (no-op) | not run for wrecks | runs, no-op |
| timer decrease | never (no repair path found) | never | never, except the **non-native repair** below |
| post-explosion timer (+0x344) | advances; at `post_explosion_fire_seconds` clears smoking and burning (step 4) | none | advances only after the explosion (**inferred** gating: the constructor value 0 and the "after the explosion" reading), 8.0 s from the `.epe` key; a missing key falls back to 8.0 (**non-native**: no constructor default) |
| burning after the post timer | not re-set because step 2 needs `!exploded` | n/a | same |
| water | above 0.8 clears both flags and sets +0x361; at or below 0.8 and below 0.6 clears it; owners force their effects off while set | none | as native; the submersion ratio itself is **inferred**: the fraction of the collision box below the sea level of 200 m (native: physics object vtable +0xa8, not traced) |
| flip burn (0x0079e310) | up element < -0.5, speed squared < 20, not submerged: accumulator at +0x484, health set to `burn_threshold * max - 1` past +0x488 | none | accumulator and conditions implemented, up axis inferred from the world transform; **the limit is `None` (unknown, Gap 6): a flipped vehicle never starts to burn by itself** |
| instant wreck burn seconds 0.1 | written by 0x00805700 with health 0 | same | same (irrelevant once health is 0) |

Non-native additions: repair. Native has no repair path, so a port repair (health back at or above the burn threshold) clears `burning` and resets the burn timer, and at or above the smoke threshold clears `smoking`; the controllers then stop.
Engine-on for the slot 14 burst (+0x35e, set by `CLandVehicle` slot 83) is **inferred** as "the vehicle is occupied by the player"; AI-driven and parked vehicles do not burst. The burst draw is a xorshift stream (`% 500 > 495`), not the C runtime's `rand`.
Air vehicles (a doll with a slot 8 mount): `SetBurning` plays slot 8 once on the rising edge and the mount follows the vehicle while it runs (**inferred**; the native starter 0x0073d740 was not traced for the matrix); the death routine's stop of slot 8 is done at the explosion.

**Captures** (`_extract/6db149c5/`, `JC2_VEHICLE_FIRE_FIXTURE=<health fraction> JC2_VEHICLE_FIRE_VIEW=5,2,7`, `--world 40,40 --no-hero --drive --no-npcs --no-civilians --no-traffic`, `JC2_START_OFFSET=-80,0`): the "before" frames are the same build with
`JC2_VEHICLE_FX_OFF=1`, which leaves the controllers out and reproduces master for a smoking or burning vehicle (master drew nothing until the explosion). At 30 % and 15 % health the jeep smokes: two thin dark plumes at the engine mounts
(the smoke particle systems produce a plume only about 1.4 m tall at rest; the shape is limited by the particle runtime, `smoke-effects.md` gaps, not by this code). Below 10 % at night the two fire mounts burn visibly and the burn timer ends the vehicle after 8 s.

## 10. Death sequence implemented (FX-VEH-2, board task 51713846)

Code: `crates/jc2-game/src/vehicle_death.rs` (system `vehicle_death`, runs just before `explosion::wreck_vehicles`), `VehicleFx::explode / death_slot / DeathClass` in `vehicle_damage.rs`, `explosion.rs` (wreck colour, the placeholder blast no longer plays an effect).
Tests: `event_names_by_class`, `explosion_slot_follows_the_water`, `three_consecutive_wheels_from_a_random_start`, `a_dead_car_posts_events_and_throws_three_wheels` (ECS), `the_wreck_colour_is_the_native_constant`, and the retail fixture `death_fixture` (jeep 128, moped 164, boat 156, helicopter 157, propeller plane 128; submerged: only the helicopter and plane play slot 5 effect 44, car, bike and boat play nothing).

* **Explosion by class**: every slot 1 mount of the vehicle's own doll is placed with the vehicle matrix and played with no interactions (native `Play(0)`), slot 5 when the damage state is submerged. The old shared effect 128 and its soil/water interaction selection are gone for vehicles; the placeholder blast (radius 9, damage 450, impulse 1500, Gap 3) stays but plays no effect. The non-native fallback fireball of `detonate` still spawns with it.
* **Events**: `gsy.object_destroyed`, then `government.landvehicle|airvehicle|seavehicle.destroyed` by class, posted as `GameEvent` messages (the port has no script bus yet; nothing consumes them). The land condition 0x007990c0 is untraced and treated as true.
* **Wheels** (cars and bikes, `Vehicle` with wheel parts): three consecutive wheels from a random start (the "consecutive" reading of the three passes is inferred), each wheel's parts reparented to a dynamic sphere body, mass 20.0 (inferred meaning), vehicle velocity inherited; below speed squared 16 a random 0..1 share of a kick vector: the **vector is non-native** (the native fixed vector at 0x00612ac0 was not read, Gap 8). The bodies are freed after 30 s (non-native).
* **Charred look**: material base colour linear (0.094, 0.094, 0.094) (the first paint constant, decoded like the palette entries, so linear), replacing the shared copy (0.09, 0.08, 0.075 sRGB, about ten times darker). The second paint constant 0.078, the instance tint bytes and the damage-layer array are not used (the port's materials have one base colour; CarPaint milestone f0cd863b). The mapping is non-native.
* Pulse (7 / 300 / 500) and flash were already in place (FX-EXP-2); the flash radius stays the labelled 9.77 m non-native value (Gap 1).
* Post-explosion fire: `VehicleFx::drive` (FX-VEH-1). The effect handles of slots 1 and 5 are kept and stopped by `stop_all` when the vehicle goes away.

Gap 8: the fixed vector of the thrown-wheel launch (0x00612ac0) and the two extra tinted bike models (+0xea8, +0xeac, 0x0080a7a0) are not traced. Captures: `_extract/51713846/` (jeep death, before = master with the death hook enabled in the fixture, after = this branch; the vehicle explosion is now the doll's own effect without soil interaction, so it has less dirt and fire than the old soil-selected one).

## Gaps (unresolved; none guessed)
1. **Death flash radius** (G9, task 6c837ac1): stack residue, needs a dynamic read (section 7). Blocks: vehicle explosion flash size.
2. **Charred look shader inputs**: what the per-mesh array set to 1.0 by 0x008c51f0 and the two vec3 tints (0.094 / 0.078) feed in the material shaders; whether the original swaps any texture. Blocks: exact wreck appearance. **Answered in `vehicle-damage-effects.md` section 7** (paint colours of the CarPaint material metadata, damage texture layer and deform table; no texture swap; the remaining unknown is the instance tint bytes and the exact array-to-table link).
3. **Occupants and blast damage**: who kills or ejects the driver and passengers at death, the damage the vehicle blast deals to characters and vehicles nearby (no area damage in the effect; the 7 m pulse only pushes), the 1e7 physics values (0x009beef0 / 0x009bf9c0), and the smart-object release (0x005d1320). Blocks: gameplay parity of vehicle kills.
4. **Other slot owners**: engine idle/running (2, 3: choice and input), contrail (4), shot-down (8: the air-vehicle triggers; 0x0073d740 is CAirVehicle slot 91 called from 0x004fb750, 0x0072f440, 0x007aa1c0, 0x008098c0, apparently missile impacts), water trails (9, 11), and the slot 10 owner.
   Blocks: air shot-down visuals, boat trails, engine effects.
5. **Effect input 0 = squared speed**: what the smoke and fire particle systems do with it (rate, size, velocity inheritance). Blocks: smoke plume shape at speed.
6. **Flip burn**: the data source of the 0x0079e310 limit (+0x488) and which vehicle classes call it; **wreck removal/persistence**: the manager rule that deletes a dead vehicle. Blocks: wreck lifetime.
7. **Particle chains** of the fire and smoke systems (child chains, 405-409, `fx_exp_vehicle_*`): same open item as `explosion-effects.md` Gap 7 (particle runtime).

## Paired implementation tasks (filed `backlog`, tags effects, priority high; the effect spawner e268da82 is on master: `crates/jc2-game/src/effects`, `EffectSystem::create / play_controller / set_transform / set_velocity / set_input / stop`; the controller `SetInput` of the port takes a key, the native call passes input index 0, the declared input `speed`)
* **FX-VEH-1 vehicle damage states and fire effects**: parse the doll's `CEffectProp` list (id, slot, local matrix) and the four tuning keys; implement the section 4 state machine (smoke, burn timer, flip burn, water hysteresis, post-explosion timer) and the slot 12/13/14/15 controllers
  with per-frame SetMatrix, velocity and input 0 = squared speed, the 0.8 % burst rule; replaces the 25 s sphere smoke in `explosion.rs`. Owned: `crates/jc2-formats` (new doll effects module), `crates/jc2-game/src/explosion.rs`, `vehicle.rs` hooks.
* **FX-VEH-2 vehicle death sequence**: explosion effect by class from slot 1 (water: slot 5), events `gsy.object_destroyed` and `government.*.destroyed`, three thrown wheels, the charred tint constants of section 5.1, the code pulse and flash (through FX-EXP-2) with the flash radius an explicit labelled unknown,
  post-explosion fire for `post_explosion_fire_seconds`. Replaces the missile-light stand-in. Owned: `crates/jc2-game/src/explosion.rs`, `destruction.rs`.

## Reproduce
```
python -B research/tools/vehicle_fire_explosion_probe.py --game GAME --tool JC2TOOL --pseudo MAIN/research/analysis/pseudo/_unnamed
```
