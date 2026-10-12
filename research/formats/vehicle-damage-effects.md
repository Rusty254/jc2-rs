# Vehicle damage presentation: collision damage, sparks, parts, lamps, tyres, windows, body deformation, the wreck look (board tasks bc26288e and 4e39cad9, 12 October 2026)

Research only. Probe: `research/tools/vehicle_damage_effects_probe.py` (250 checks, exit 0; run with
`--game GAME --tool jc2tool --pseudo EXPORT`, add `--json` for the tables; the disassembly and shader checks need `capstone` and the
repo's `shader_inventory.py` / `sm4dis.py` and are reported as skipped when missing). Executable SHA-256 `df99a479...f74`.
Builds on `vehicle-fire-explosion.md` (smoke/fire thresholds, slots 12-15, the death routine, burn timers: **not repeated here**),
`vehicle-effects-selectors-next.md`, `smoke-effects.md`, `effects-pipeline.md` (controllers, the 14 call sites; section 5.1 row for 0x0073efe0 is corrected below),
`explosion-effects.md` and `material-parameters-next.md` (JC2-018: CarPaint PS 973444 damage weights, now joined to the vertex stage).
"native" = read from the executable or the retail data and asserted by the probe; "inferred" is said so. Addresses are retail virtual addresses;
names of functions and fields are working labels.

## 0. Answers in one table

| Question | Answer |
|---|---|
| What are effects 425 / 449 / 450 "at 15 and 25"? | Not sparks. Pure **camera shake + haptics** (shake scale 0.5 / 0.9 / 1.4, speed 0.4; no particle, sound or light). Played by `CVehicle` slot 12 (0x0073efe0) only when a seat holds the player (byte +0xba6) and the contact impulse is above **2000**; picked by the vehicle's **speed**: below 15 m/s 425, below 25 m/s 449, else 450. Section 2.1. |
| Where do scrape and collision sparks come from? | Ground-aligned contact (contact normal . vehicle up > 0.9, land vehicles): persistent effect **293** `lave_chassis_impact` (`fx_env_metal_impact_01` + sound 17/26, input 0 = impulse). Other contacts: the surface-aware **67** impact (and 68 slide, 69 roll; boats 254 / 294) with `metalflake` particles. Wheel removed + touching ground + slip: **158** rim sparks (`flare`). Helicopter rotor strikes world geometry: **195** rotor sparks. Sections 2.2 to 2.5. |
| How does the vehicle lose health from a crash? | `damage = 3.0 * impulse / mass * multiplier` (multiplier = module key, 0.5 to 1.0), through a per-type threshold table; above an **instant-wreck threshold** the vehicle dies at once (health 0, burn time 0.1 s). Section 1 and 2.1. |
| Do shots kill a vehicle outright? | No. A hit that leaves it below 10 % (and not yet exploded, burn timer not expired) **pins health at 5 %**; death comes from the burn timer (`vehicle-fire-explosion.md` 4.2). Section 3. |
| Per-part damage and detach | Every vehicle's doll carries 968 `CPartDamageModule` records (638 are lamps). Lamps break on one hit (`explosion_headlight` effect 28). Doors, hoods, mirrors, bumpers, flaps use templates with health / fall-off values; the consumer of the six fields is not traced. Section 4. |
| Tyre blow-outs | `RemoveWheel(index, shot)` (0x00505270, land vehicles only): wheel flagged detached, tyre model hidden and the **rim model** shown, effect **26** `explosion_tire` (149 on two cars), "ply.tireshot" stat; then **rim sparks 158** while driving. The trigger that calls it is not traced. Section 5. |
| Window breakage | None. Vehicle glass is a `DeformableWindow` render block (473 blocks) that uses the **same deform table** as the paint: windows follow the body deformation, no glass-break effect is referenced by any vehicle. Section 6.2. |
| Damaged model or material swaps | **None.** No vehicle archive holds a damaged model, texture or material. Damage is (1) a 256-cell **vertex deform table** (CarPaint and DeformableWindow vertex stage: up to 2.5 m along stored vectors), (2) a **damage texture layer** (`destruct1map_*`, 512 x 512, shared) blended by the same amount, (3) detachable parts, lamps, wheels. Section 6. |
| The charred look (task 4e39cad9) | The wreck sets the paint colours of every model material set to constant linear **0.094 / 0.078** (and their luma scalar to 0), sets a per-material float array (the damage amount, all 256 cells) to **1.0**, and writes two instance tint blocks. It swaps no texture or model; the damage layer (`destruct1map`) is what darkens and scuffs. Section 7. |
| Per class | Table in section 8: only land vehicles lose wheels; boats add one call after the shared damage code; aircraft have no scrape effect but helicopters have the rotor strike. |

## 1. Max health, damage types and the PD variant damage block (native)

The paper doll's `CVehicleDamageModule` node (key `#a6db0131` of the vehicle node) is read by the PD variant loader **0x008cb720** into ten dwords at **variant +0x210..+0x237**
(defaults when the node is missing, 0x008cb720: 500, 300, 10, 50, 50, 75, 10, 2, 1/900, 1.0). Vehicle slot 69 (**0x0056dcd0**, identical in all nine vtables) applies them:

| variant | key | default | applied to | retail values |
|---|---|---|---|---|
| +0x210 | `#f42307c2` (`max_health`) | 500 | `SetMaxHealth` 0x006c7770 (vehicle +0xe4, clamps +0xe8) then `SetHealth(max)` | 200 (cutscene), 500, 750, 1000, 1500, 2000, 3000, 4000, 5000 |
| +0x214 | `#1a4be591` | 300 | vehicle **+0x388**: instant-wreck threshold (section 2.1) | 50, 75, 100, 200, 500 |
| +0x218 | `#2b8a8075` | 10 | damage type 1 threshold (table below), also doll object +0x180 `+0x14` | 5, 10, 20, 50, 60 |
| +0x21c | `#05de7e98` | 50 | damage type 3 threshold, also doll object +0x180 `+0x10` | 2, 5, 7, 10, 15, 25, 50, 80 |
| +0x220 | `#57739d9c` | 50 | not read by 0x56dcd0 | 0, 50, 250 |
| +0x224 | (none) | 75 | not read from data | - |
| +0x228 | `#63298a34` | 10 | vehicle +0x38c | 5, 10, 20, 25, 40, 50, 100 |
| +0x22c | `#e11f2a5e` | 2 | vehicle +0x390 | 1, 2, 10, 50, 100 |
| +0x230 | `#254b9014` | 1/900 | not read by 0x56dcd0 | 0.00025, 0.00075, 0.003, 0.0033 |
| +0x234 | `#e8cbda9d` | 1.0 | vehicle **+0x338**: collision damage multiplier | 0.5, 0.6, 0.75, 1.0 |

The same hash `#f42307c2` is `max_health` in the vehicle `.epe` (100 on all 90, read with `health` = 100 by the `CDamageable` loader 0x00741890, vtable slot 6 of 0x00e346b4).
So the effective maximum is **the module value (750 for the jeep) if slot 69 runs after the loader, else 100**; the order of the two calls was not found (Gap 2).
All thresholds in `vehicle-fire-explosion.md` are ratios and are unaffected; absolute damage numbers are.

**per-type table** (12-byte entries at vehicle `+ (type * 3 + 0x1e) * 4`: threshold, multiplier, id, mode; written by 0x007f04d0): id 9 passes the amount through; mode 0 returns `mult * amount` only when `amount > threshold`;
mode 1 returns `mult * max(0, amount - threshold)`. With `apply` set, health becomes `current - result` through `SetHealth` (0x007e87a0, which posts `gsy.object_destroyed` at 0).
Slot 69 installs types 1, 2 and 3 as mode 1, multiplier 1.0: type 1 threshold = +0x218 value, **type 2 threshold 0**, type 3 threshold = +0x21c value. Type 2 is the direct hit (slot 9, 0x00626440); type 3 is collision (0x00805700) and the helicopter rotor strike (0x007e1400). Types 4 and 5 have
callers (0x00638c70, an object at +0xd4) but are not mapped.

Retail census (probe `--json`, `vehicle_damage_templates`): 88 dolls carry the node (only `lave.v006_moped` and the doll-less `arve.v062_chopper_mg` do not; the moped therefore takes the loader defaults, and the swamp boat lacks `#e8cbda9d`, default 1.0).
Templates: CCar `CarDamage_Low` (22 x 500 hp, thresholds 75 / 5 / 2, multiplier 0.75; `v030_super_sportcar` 200 / 10 / 15, 1.0), `CarDamage_Medium` (17 x 750 hp, 100 / 5 / 7, 0.75; two 500 hp, 200 / 20 / 25), `CarDamage_Medium_Truck` (5 x 2000 hp, 100 / 5 / 7, 0.6),
`CarDamage_SuperTruck` (2000 hp, 200 / 10 / 15, 0.5), `CarDamage_Heavy` (tank, APC: 3000 hp, 500 / 5 / 50, 1.0); CMotorBike `CarDamage_MC` (5 x 500 hp, 75 / 5 / 5, 0.75) and `CarDamage_Low`; CBoat `BoatDamage_Light` (8 x 500 hp, 50 / 5 / 2), `_Light_Mil` 1000,
`_High` (1500 hp, 75), swamp boat `CarDamage` (500 hp, 200 / 60 / 80); CHelicopter `HeliDamage_Low` 2000, `_Medium` 2000, `_Heavy` 3000, `_V059` 1000, `_V062` 4000 (type 3 thresholds 2 or 10); CAirPlane `ARVEDamage_Low` 1000, `_Medium` 2000, `_Heavy` 5000, jet fighter `HeliDamage_Heavy`. All multipliers on aircraft and boats are 1.0 (0.75 on `CarDamage_Low` boat `v089`).

## 2. Collision pipeline (native)

### 2.1 The contact callback per class

`CVehicle` vtable slot 12 is the contact callback `(contact, impulse, ...)`; the argument block holds the contact position at +0 and the normal at +0xc.

| Class | Slot 12 | What it does |
|---|---|---|
| `CVehicle` (base) | 0x0073efe0 | feedback and contact effects only (below) |
| `CAirVehicle`, `CHelicopter`, `CAirPlane`, `CSeaVehicle` | 0x006ee8a0 | `0x00805700` (damage) then `0x0073efe0` |
| `CBoat` | 0x00682fc0 | `0x00805700`, `0x0073efe0`, then, above the same impulse gate, one more call on the physics object (0x009c3ea0, not traced) |
| `CLandVehicle`, `CCar`, `CMotorBike` | 0x00700400 | player-ram assist, `0x00805700`, then **either** `0x0073efe0` (normal . up <= 0.9) **or** the scrape effect |

**Damage (0x00805700).** Returns early when the other object is of the class tested by 0x0056aec0 (not identified) or when health is 0 and the debug byte is clear. Otherwise
`local = (3.0 / mass) * |impulse|` (DAT 0x01093c04; mass = physics object virtual +8, inferred), `local *= vehicle +0x338`. Instant wreck:
`(local - threshold(+0x388) * f > 0 || byte +0x369) && !invulnerable` with `f` = **2.0** when the player occupies the vehicle (+0xba6) and +0x36a is clear, **0.05** when +0x36a is set, else 1.0: then burn seconds (+0x340) = **0.1** and `SetHealth(0)`
(the death routine of `vehicle-fire-explosion.md` follows). Else `ApplyDamage(local, 3, -1, apply)`; when something was dealt: vehicle sound call 0x0079da70(1, 0), the physics object is told the **side** (0..3 from two dot products of the contact normal with the vehicle axes) and the integer damage
(`vtable +0x94`, not traced), and a table of **20 pop-part entries** (stride 0x10 at vehicle +0xa68; flag, part index, event id) is scanned: every unfired entry whose direction to the hit point has a dot product above 0.5 with the part's axis triggers
`vtable +0x18` of the object at +0xf0 (event id, local hit offset). +0x369 is set by the air shot-down starter 0x0073d740; +0x36a by the seat scan 0x0067b030 (condition not decoded).

**Feedback and contact effects (0x0073efe0).** Needs the physics body (+0x4e4). Gate: `0.01 * |impulse| >= 5.0` (impulse >= 500). Then (a) the **contact effect entry 0** (0x007244c0: impact, id from the vehicle `.epe`, section 2.2) is updated every time, and (b) when byte **+0xba6** (a seat holds a
character for which character virtual +0x28 is true, from 0x007d58c0; inferred "the player") is set and `impulse > 2000.0` (DAT 0x01072644): `0x00579f50(id, contact transform)` one-shot with id 425 below speed **15.0**, 449 below **25.0**, else 450 (DAT 0x01072630..0x01072640; speed = length of the vehicle velocity).
`effects-pipeline.md` section 5.1 lists this site; its reading "the velocity length" is confirmed, the player gate and the 2000 floor were already there, the missing piece is that the three effects hold only shake and haptics (probe: `shake` scale 0.5 / 0.9 / 1.4, speed 0.4, haptics 0.75 / 0.2).

**Land vehicles (0x00700400).** (1) Player-ram assist: when the other object is a vehicle with the player inside (+0xba6), both vehicles move faster than 10 m/s and the cooldown +0x36c is 0, this vehicle receives an extra impulse built from the constants 4000 / 1.5 / 2.0 / 2500 / 300 / 0.1 and a floor of 500
(DAT 0x01072870..0x01072888; 0x0107286c = 0.5 is the pop-part dot threshold of 0x00805700) and the cooldown becomes 0.3 s (inferred: nudges rammed traffic). (2) `0x00805700`. (3) Contact normal . up (+0x2f8..+0x300) **<= 0.9**: `0x0073efe0`; otherwise the **scrape effect**: the id at **+0xd94** (key `#e89622fb`, default 293 = `lave_chassis_impact`, set by the CLandVehicle loader 0x007557e0), created once with 0x0062a7a0 and kept at +0xdc8, then SetInput 0 = impulse, SetMatrix and `Play` each time (not restarted while running).

### 2.2 Per-vehicle effect ids in the `.epe` (native, probe census)

| Role | Key (DAT) | Cars (50) | Bikes (9) | Boats (14) | Helis (10) | Planes (7) |
|---|---|---|---|---|---|---|
| contact impact | `#a132f7da` (0x0106c174) | 67 | 67 | 254 (13), -1 (1) | 67 (8), -1 (2) | 67 |
| contact slide | `#eb001f37` (0x0106c178) | 68 | 68 | 294 (13), -1 | 68 (8), -1 (2) | 68 |
| contact roll | `#396c6abd` (0x0106c17c) | 69 | 69 | -1 | 69 (8), -1 (2) | 69 |
| hinged part impact / slide | `#98ee7f54` / `#a94556b1` (0xebedc0 / 0xebedc4, CVehicle loader 0x00780ab0) | 379 / 380 | 379 / 380 | 379 / 380 | 379 / 380 | 379 / 380 |
| chassis scrape | `#e89622fb` (CLandVehicle loader) | 293 | default 293 | - | - | - |
| tyre blow-out | `#bd438cc4` (CLandVehicle loader, default 26) | 26 (48), 149 (2) | 26 | - | - | - |
| rim sparks (wheel effect 3) | `#d56bbd1e` (0x01072184) | 158 (48), -1 (2) | 158 | - | - | - |

Lamp break (`#094ee3ff`, slot 86 loader 0x00563df0, default 28): 28 on 58 vehicles, -1 on one, absent (default 28) on 31. Wheel rolling / skidding / spinning (`#fee137c7` `#578e3bb1` `#3ebe41a7`, 0x01072178..): 16 / 17 / 18 on the 59 land vehicles.

Each contact entry is a 0x60-byte record at vehicle +4 + i*0x60 loaded by **0x0056ea20** (key `#a132f7da`... at 0x0106c174, a second value per entry at 0x0106c180 = `#ed3340a7` `#e2a3741b` `#a280098d`, absent in retail, default 0).
Entry 0 is driven by 0x0073efe0, entry 2 (roll) by 0x00742490 (when the contact normal and the vehicle axis are nearly parallel, |dot| > 0.9999), entry 1 (slide) by a caller not traced. The entry updater 0x007244c0 only plays when the contact is above **200.1** in world y
(inferred: the water surface; `effects-pipeline.md` uses the same 200 m for the water input), then runs a small state machine (0x0058b9c0, 0x00669140) with a 1.5 x impulse hold time.

Contents (probe `effects`): **67** `lave_collision_impact` = 8 surface variants (Asphalt, Snow, Mud, Sand, Water, Objects, Terrain, impact cloud) of `fx_vehicle_impact_*` (metal-flake particles, `fx_vehicle_impactcloud_asphalt_01` is missing from the particle archive) + 8 sounds of bank 15, post gain 0.6;
**68** slide = `fx_vehicle_slide_asphalt_medium_01` + sound 15/5; **69** roll = empty; **254** / **294** boats = the same particle sets with bank 17 / 15 sounds; **379** / **380** hinged = copies of 67 / 68 without the post gain; **293** chassis = `fx_env_metal_impact_01` + sound 17/26; **232** `lave_aggroll` (15/27) and **200**
`lave_wheel_suspension` (4/3) belong to other owners.

### 2.3 Rim sparks and the wheel effect controllers (native)

`enable = wheel detached (+0x181) && wheel touching && |slip| > threshold[0x346]` (key `#cb1cf725`, 0.1 on 56 vehicles). 0x00546250 holds a 5-frame hysteresis counter per kind (+0x1d0 vector), starts the controller with input 0 = value (`SetMatrix` from the wheel matrix at +0x100) and stops it when the counter runs out.
So rim sparks need three things: the wheel was removed (section 5), it touches the ground and the slip exceeds 0.1.

### 2.4 Helicopter rotor strike (native)

`CHelicopter` secondary vtable (0x00e56f7c) slot 7 = 0x006b31a0 calls **0x007e1400**: a sweep of rays (angle step from +0xcbc over 2 pi, 8 m long, static geometry only) from the rotor; every hit applies
`ApplyDamage(200 * (1 - t)^2, 3, ...)` (t = hit fraction of the ray), a physics kick proportional to the mass (factors 10.0 and -0.2, DAT 0x010726f0 / 0x010726ec) and plays the literal effect **195** `arve_rotorsparks` (`fx_vehicle_arve_rotorsparks_01`, `flare` particles, sound 7/4, orange 0.1 s light) at the hit, created on demand at +0xc98.

### 2.5 Other effects named by data

`arve_destroy_air` (46, `fx_exp_metalchamber_small_01`, 0.3 m orange flash): played by the aircraft removal/destroy function 0x0079eb20 from id +0x490 when not silent; its key and when it runs are not traced (Gap 7).

## 3. Shots: the hit path and the 5 % clamp (native)

`CVehicle` slot 9 (**0x00626440**, `(direction, damage, ..., source)`): applies the impulse to the physics object, then `ApplyDamage(damage, 2, source, apply)`; when something was dealt: physics call (0x8c) and the vehicle sound call 0x0079da70(1, 0).
Then, **if** `health / max < 0.1`, the vehicle has not exploded (+0x35c) and the burn timer has not passed `burn_seconds` (+0x33c <= +0x340) **and** either a player occupies the vehicle (+0xba6) or a check on the source (0x005b9880, not decoded) passes:
the last-damage-source byte (+0x36e) is stored and **`SetHealth(max * 0.05)`**. So a shot vehicle never dies from the hit that crosses 10 %: it is pinned at 5 % health, keeps burning and explodes when the timer of `vehicle-fire-explosion.md` 4.2 ends.
Separately, health <= 0 or health below the burn threshold also stores the source byte. Collisions can still kill instantly (section 2.1).

## 4. Parts: lamps, detachable parts, hinges (native data, inferred semantics)

### 4.1 Lamps

Slot 86 (**0x00563df0**) builds a **20-entry lamp table** (16-byte entries at vehicle +0xa60: destroyed flag, float, part index, part id) by looking up physics parts by hashes of the part tags `#71a73c4a` (lower case). The 20 hashes are, in order (probe: exact Jenkins matches):
`hl_l hl_r i_fl i_fr lt_1 lt_2 lt_3 lt_4 lt_5 lt_6 lt_7 lt_8 rl_l rl_r i_bl i_br t_l t_r bk_l bk_r` (head lights, front indicators, light bars 1-8, rear lights, rear indicators, tail lights, brake lights).
638 lamp parts exist (cars 526, bikes 17, boats 45, helicopters 26, planes 24); 626 have **no model** (`model = ""`, collision volume only; 12 land and boat parts do have one). All use one template: **health 2, second value 1, third 2** (`Light_Damage` / `Lights_Damage`),
byte +0x45 = 1, byte +0x46 = 1 (lamp), byte +0x47 = 0: one hit breaks a lamp.
`IDamagePartOwner` slot 6 (**0x005d1b30**, the same function in every class) receives the destroyed part id and position: it finds the unfired table entry with that id, sets its flag, stores -1.0, marks the part record (`this + 0x470 + index * 0x74` = 1) and the dirty byte +0xab4, and **plays effect `+0x88` (= key `#094ee3ff`, 28 `explosion_headlight`: `fx_bulhit_headlight_small_01` + sound 37/21) at the lamp position**. The per-frame reset 0x00526a40 clears the float of every intact lamp entry and raises the dirty byte +0xba4, so -1.0 marks a broken lamp for the light code (the light side belongs to JC2-207).

### 4.2 Breakable and detachable parts

`CPartDamageModule` node fields (part loader **0x008d3a30**, part-info offsets): `#f42307c2` +0x28 (health value), `#7a7e9413` +0x2c, `#7f9d8eb9` +0x40 (int), bytes `#e51a94cf` +0x45, `#6d87f8e4` +0x46, `#8bf982d4` +0x47. Retail templates (968 records; per class counts in the probe `part_damage_templates`):

| Template | +0x28 / +0x2c / +0x40 | flags 45 / 46 / 47 | Parts (examples) | Classes |
|---|---|---|---|---|
| `Light_Damage`, `Lights_Damage` | 2 / 1 / 2 | 1 / 1 / 0 | lamps | all |
| `Door_Low`, `Door_Medium` | 500 / 450 / 1 | 1 / 0 / 0 | doors, hoods, trunk, hatches (65 + 55) | cars, boat, heli |
| `CarDamage_Door_Low` | 300 / 250 / 1 or 500 / 450 / 1 | 1 / 0 / 0 | doors, compartments | cars |
| `CarDamage_Door_Medium` | 750 / 650 / 1 | 1 / 0 / 0 | tailgate, doors, hatches, roof parts | cars |
| `CarDamage_Door_Heavy`, `Damage_Door`, `HeliDamage_Door_*` | 250 / 200 / 650 | 1 / 0 / 0 | heavy doors, cockpit, flaps | cars, boats, helis, planes |
| `Hood_Stunt_Medium` | 650 / 600 / -1 | 1 / 0 / 0 | hoods on stunt-capable cars (5) | cars |
| `Attach_Damage` | 20 / 10 / 10 | 1 / 0 / 0 | mirrors, wipers, spare tyres (`Tire_*`), barrels, antennas, fuel tanks (63) | cars, boats |
| `Bumper_Damage` | 100000 / 99750 (or 99950) / 100000 or -1 | 1 / 0 / 0 | bumpers (14) | cars |
| `Falloff_OnExplode`, `Falloff_on_Explode_Damage` | 100000 / 185 (or 10) / 100000 or -1 | 1 / 0 / 0 | mirrors, sirens, cranes, wood (9) | cars |
| `Hinge_Always`, `Hinge_neverfalloff_Damage` | 10000 / 9990 / -1 | 1 / 0 / **1** | mud flaps, ropes, masts, suspensions, loops (14) | cars, boats, planes |
| unnamed `CPartDamageModule` (default name) | 100 / 50 / -1 | 0 / 0 / 0 | door parts of two cars and four helicopter parts (6) | cars, helicopters |
| `Damage_Tail`, `Damage_Tail_Heavy`, `Damage_Wing_Heavy`, `PlaneDamage_*` | 700 to 99999 / 1 to 2250 / -1 to 1900 | 1 / 0 / 0 | tails, wings, flaps, airbrakes, cockpit | helis, planes |
| `Engine_Damage` | 930 / 10 / 930 | 1 / 1 / 0 | gun carriage (1 boat) | boat |

In every record the second value is below the first (probe). *Inferred* reading, not proven: the first is the health the part has, the second the damage that must remain/has to be dealt before it comes off (door 500 / 450: comes off after 450 damage); flag 47 marks a part that never falls off; flag 46 the lamp class.
The consumer is the physics side: the part contact listener **0x009bba30** (`CPfxDamagePartListener`) sums the contact impulses of the part (ignores totals <= 10), converts them with `3.0 / mass * total`, and queues `(part, damage, damage, 1)` into the breakable's 16-entry atomic queue (0x00990650, `CPfxBreakable` at 0x00f27534);
how the breakable turns the queue and the six fields into a detach, which model is spawned (`IDamagePartOwner` slot 1 = 0x00821c90 creates a part model from a `.lod` name) and the hinge behaviour (Havok breakable constraints) are Gap 1. The `379` / `380` hinged-collision effects are the contact effects of detached or hinged parts.

## 5. Tyre blow-outs (native, trigger open)

`IDamagePartOwner` slot 4 (**0x00505270**, `RemoveWheel(wheelIndex, shot)`) exists for land vehicles only (vtables 0x00e4f19c, 0x00e3ad2c, 0x00e33b44; boats, aircraft and the base class hold the no-op 0x0094aa00). It: releases the three wheel effect handles at +0x1e0..+0x1f0, clears the visible bits of the tyre model (+0x198, `&= ~0x11`) and sets them on the **rim model** (+0x1a4, `|= 0x11`),
sets wheel scalar +0x184 = 0 (meaning not decoded; slot 5, 0x005ed110, writes it when +0xcaa is clear), sets the **detached byte +0x181**, plays the effect id at vehicle **+0xd90** (key `#bd438cc4`: **26 `explosion_tire`** on 57 land vehicles, **149 `explosion_tire_big`** on two cars; default 26) with the wheel matrix, and, when `shot` is true, increments the statistic `ply.tireshot`.
Effect 26 is `fx_exp_tire_01` + sound 37/35 + a 6.5 m force pulse; 149 the big variant. 67 vehicle archives carry `*rim.lod` models (59 land vehicles with wheels, 8 aircraft with landing gear); no boat does. The wheel stays physically in the vehicle until the death routine throws three wheels (`vehicle-fire-explosion.md` section 5). After removal the wheel scalar and the removed flag let the wheel updater (section 2.3) emit rim sparks.
Not traced: who calls slot 4 with `shot = true` (the stat name says a shot tyre) and how a wheel is selected (Gap 3).

## 6. The body damage look

### 6.1 No swaps; deformation and a damage layer

No vehicle archive contains a model, texture or material whose name says damaged, broken, wreck, burnt or destroyed (probe over all members of the 90 archives). The only extra meshes are the optional `CrashRig` parts of the two jeeps (`lave.v014_jeep`, `lave.v047_jeep_small`; collision-and-model rigs without a damage template).
What exists is rendering state:

* **Deform table.** The CarPaint vertex stage (VS offset 47080) reads a 256-float table in **cb2[64]** and a deform-enable flag `cb4[0].x`. Per vertex: four cell indices from `v1` (v1 x 127.996: integer part + 128 = cell, split into a cb2 row and lane; fractional part = weight), weights normalised, `amount = saturate(dot(table cells, weights))`, position += `amount * 2.5 * v` where `v` is the per-vertex vector packed in `v0.w`
  (components in [-1, 1]); the normal blends to the stored deformed normal by `min(1.5 * amount, 1)`; `amount` leaves the stage in `o5.w`. (Probe asserts the instructions.) The `DeformableWindow` vertex stage (473 vehicle glass blocks) contains the identical table, cell lookup and 2.5 factor.
* **Damage texture layer.** CarPaint v3 blocks carry a second texture triple (slots t3 diffuse, t4 normal, t5 MPM at the second UV set). In 1188 of the 1756 vehicle CarPaint blocks it is `destruct1map_dif / _nrm / _mpm.dds`, in 480 `nodestruct_*` (interiors, engines, undercarriage, decals), in 88 their own maps.
  The pixel stage (PS 973444, `material-parameters-next.md`) weights it with `D = v5.w`: diffuse `*= lerp(1, t3, saturate(2D))`, normal `+= (2 * t4 - 1) * saturate(1.5D)` (z scaled by `1 - 0.8 * saturate(1.5D)`), MPM `*= lerp(1, t5, saturate(2D))`.
  The three textures live in `global/areasets/general.blz` (512 x 512 DXT1, 174,904 bytes each). `destruct1map_dif` is white with grey and black scuff and scratch strokes: a darkening multiply that reads as scorched, scratched metal at full amount.
* There are also `Dirt`, `Dull`, `Flat` shader permutations (names in the bundle and exe: `CarPaintLod%i...`, `_Dirt`, `_Snow`, `car_dirt.dds`); which instance data selects them is not traced.

What writes the cells during normal play (dents) is not found: the collision code calls an unnamed physics-object virtual (+0x94) with the side and damage (section 2.1) and the shared model-instance setter 0x00933020, which sets float `i` of a per-instance array and a dirty bit when the value exceeds 0.05, has only two callers (the wreck function below and the deform-decal copy 0x008cf600). Dents and
their exact amounts are Gap 4.

### 6.2 Windows

Separate window models exist only on `lave.v015_military_vehicle` (`Window`, with a `Door_Medium` module) and `arve.v010_personal_propellerplane` (`WindowCockpit_FU1`, no module). Glass is otherwise inside the body or the `DeformableWindow` blocks. No vehicle `.epe` effect id names a glass effect (`km05_glassbreak` 113, `f2m08_glassbreak` 261, `cs_carwindow_01` 434 and the other glass breaks are set pieces);
`Bullet_Hole_Glass` decals belong to the surface system. **There is no window-breaking visual for vehicles**: glass dents with the body.

## 7. The charred look (task 4e39cad9: answer)

The death routine 0x00559150 (`vehicle-fire-explosion.md` section 5) does three things to the visuals (probe asserts each):

   The array is the per-instance damage/deform data (inferred link: the shader reads a 256-cell table with the same dirty-upload shape; the write that makes cell `i` equal the stored array entry was not located, Gap 6). Effect if the link holds: the entire body at maximum damage amount: damage texture layer fully applied **and** vertices displaced by the full stored vectors (the crushed shape the artists authored).
2. **Paint colours.** Slot 31 (0x00631710, flag 1), called for every part model of the vehicle (0x84-byte entries at +0x3a0, attached objects +0x3c4; the body is one of them), writes into the model's **material metadata**: `CMetaDataDeformLight` (type 10; `+0xd0..+0xdc` first paint colour rgb + scalar, `+0xe0..+0xec` second paint colour rgb + scalar, bits of `+0xf4` |= 3) or `CMetaDataTwoTone` (type 4; `+0x5c..+0x68`, `+0x6c..+0x78`, bytes `+0x7c`, `+0x7d` = 1)
   the constants **(0.094, 0.094, 0.094)** and **(0.078, 0.078, 0.078)**. The scalar stored beside each colour is `clamp(0.425 r + 1.4308 g + 0.1442 b - 0.2, 0, 1)` (0x00926f40), **0 for both** (the coefficients sum to 2.0). In the undamaged state (flag 0) the same fields get the vehicle's **palette colours** (`vehicles\palette\*.pal` entries of two colours, 0x24-byte stride, 0x005d9580) decoded with **pow(c, 2.2)**;
   the wreck colours are therefore the grey 0.343 and 0.316 in palette space. The `CarPaint` block parameters carry the same two rgb triples (the jeep: 0.187, 0.146, 0.065 twice).
3. **Instance tint blocks.** Slot 85 (0x006e15c0 for cars) and slot 31 write two byte groups into the model instances: RGBA **(28, 28, 25, 252)** at +0x24..+0x27 and bytes **(25, 255)** at +0x20, +0x21 (data 0x01072298..0x0107229d); the undamaged state writes (0, 0, 0, 0) and (255, 0). For cars slot 85 applies them to the four model slots of every wheel record (+0x198 tyre, +0x1a4 rim, +0x19c, +0x1a0).
   Their consumer was not traced; the colour (a dark soot brown) and the shader permutation names (`CarPaint...Dirt`, `car_dirt.dds`) suggest a dirt/soot tint (Gap 6, inferred).

So the original **does not swap any texture or model** and does not only darken: it darkens the two paint colours to constant dark greys with zero gloss scalar, and drives the damage layer (and probably the vertex deformation) to its maximum. The placeholder in `explosion.rs` (base colour 0.09 / 0.08 / 0.075 on every material) is close to the grey but loses the scuff layer, the gloss change and the deformation.

## 8. Per-class summary

| | CCar | CMotorBike | CBoat | CHelicopter | CAirPlane |
|---|---|---|---|---|---|
| contact callback | 0x00700400 | 0x00700400 | 0x00682fc0 | 0x006ee8a0 | 0x006ee8a0 |
| collision damage (0x00805700) | yes | yes | yes | yes | yes |
| player-ram assist, ground scrape 293 | yes | yes | no | no | no |
| shake effects 425/449/450 | player-occupied | same | same | same | same |
| contact impact / slide / roll | 67 / 68 / 69 | same | 254 / 294 / none | 67 / 68 / 69 (8 of 10) | same |
| hinged-part effects 379 / 380 | yes | yes | yes | yes | yes |
| lamps | 526 parts | 17 | 45 | 26 | 24 |
| wheel removal, rim model, rim sparks, tyre effect 26 / 149 | yes | yes | no | no (no-op) | no (no-op; rim models exist) |
| rotor strike 195 | - | - | - | yes | - |
| damage template family | CarDamage_* | CarDamage_MC / _Low | BoatDamage_* | HeliDamage_* | ARVEDamage_* |
| max health (module) | 500 to 3000 | 500 | 500 to 1500 | 1000 to 4000 | 1000 to 5000 |
| extra part templates | Door, Attach, Bumper, Falloff, Hinge, Hood | lamps only | Door, Attach, Hinge, Engine | Damage_Door, Tail, Flaps, Hinge | Plane*, Wing, Tail, Hinge |
| body deform (CarPaint / DeformableWindow shaders) | yes | yes | yes | yes | yes |

## 9. Current runtime versus native

`crates/jc2-game/src/explosion.rs` kills a vehicle at `hp <= 0` of the generic `combat::Health` (bullets and explosions only), then darkens every material and plays the code pulse and flash. Missing: the PD damage block (max health, instant-wreck threshold, multiplier, type table), collision damage and the contact callback, the 5 % hit clamp,
the 425 / 449 / 450 shake and the contact effects (67 / 68 / 69 / 293 / 379 / 380), rotor strikes, lamp breaking, part detachment, tyre removal and rim sparks, the CarPaint damage layer and the deform table, and the vehicle-specific wreck colour pair. `impact_fx.rs` (bullet impacts per surface) is separate. The avian `ContactGraph` is already read in `physics.rs`.

## 9b. FX-VEH-3 implementation status (board 53206030)

`crates/jc2-formats/src/vehicle.rs` parses the module (`DamageModule`, `VehicleDoll::damage`) and the `.epe` burn keys (`BurnTuning`); `crates/jc2-game/src/vehicle_damage.rs` holds the model:
type table (mode 1, types 1 / 2 / 3), collision damage from the avian `ContactGraph` (summed normal impulse of a touching pair, `FixedPostUpdate` in `PhysicsSystems::Last`), the instant-wreck rule with the occupant factor 2.0 (flag +0x36a: factor 0.05, never set yet),
the shot pin at 5 % (`combat.rs` `hit` and the soldiers' shots at the player's vehicle) and the burn timer that finishes a pinned vehicle (without it the pin would make shot vehicles immortal). The effective maximum health is the labelled switch `MaxHealthSource`
(`JC2_VEHICLE_MAX_HEALTH=module|epe|types`, default the doll module value; Gap 2 stays open). Inferred or non-native: avian's per-pair impulse stands for Havok's per-callback impulse; the pin's source check (0x005b9880) is taken as true for the player's shots;
the burn timer's "exploded" test is the existing `Wreck` marker; explosions (blast damage in `explosion.rs`) subtract health directly and are not pinned (the pin lives in slot 9 only; the blast damage type is not mapped, Gap 2). Not done: the vehicle sound call, the side/damage call to the physics object,
the 20-entry pop-part scan, the shake and contact effects (FX-VEH-4), the fire/smoke effects that follow `VehicleDamage::burning` (FX-VEH-1; they must keep their effect handles and Stop them on cleanup, 484554ae).

## 9c. FX-VEH-4 implementation status (board 5403e32b)

`crates/jc2-game/src/vehicle_contact_fx.rs` (plus `ContactFxIds` in `jc2-formats` vehicle.rs for the `.epe` keys `#a132f7da` `#eb001f37` `#396c6abd` `#e89622fb`): the per-class contact callback (section 2.1) on avian contact pairs, run after the damage call. Land + normal . up > 0.9: scrape effect (input `impulse`, verified present on effect 293 by a retail test), no base feedback; otherwise the impulse >= 500 gate, the contact impact effect (67 / 254, surface interaction from the terrain material, only above world y 200.1) and, for a player-occupied vehicle with impulse > 2000, shake 425 / 449 / 450 by pre-step speed (15 / 25 m/s).
**Rotor strike, corrected from 0x007e1400 (decompile re-read):** the sweep is **two** rays per frame, at the sweep angle and sweep angle + pi (loop step 3.1415927, limit DAT 0x010721b4 = 2 pi), the sweep angle (+0x32f) advances 2 pi per second and wraps at 2 pi; rays are 8 m, from the first `CRotorInfo` (+0x2f1 list), run only while the player occupies the vehicle (+0xba6) and the physics-object value +0xb8 is >= 5.0 (or there is no object; value not identified, the port uses the blade spool 0.55). The first ray that hits applies `200 * (1 - t)^2` type 3 damage, two physics kicks (0x009b6930 with the rotor axis x mass x 10, 0x009c00e0 with the ray direction x mass x -0.2 at the hit point; read as angular and linear impulses, not traced), plays/keeps effect 195 at the hit (created at +0xc98 on demand), and resets the timer +0x330; with no hit the timer grows and the effect is stopped at **1.0 s** (DAT 0x010726f8). Constants read from the exe: 200.0, 10.0, -0.2, 1.0.
Non-native: avian pair impulse for Havok's per-callback impulse; resting filter (impulse / mass < 0.4 m/s is ignored); replay holds 0.3 s (impact) and 0.15 s (shake); pre-step speed stands for the pre-solve velocity; normal sign convention (from the other object into the vehicle). Not done (Gap 5 stays open): slide caller, roll conditions (effect 69 is empty), the state machine, the boat extra call 0x009c3ea0, the stored-impulse rule. Controllers (scrape, rotor) are stopped on cleanup.

## Gaps (unresolved; none guessed)
1. **Part damage consumer**: how `CPfxBreakable` (0x00f27534, builder 0x00f275d0) turns the 16-entry damage queue and the six `CPartDamageModule` fields (+0x28, +0x2c, +0x40, +0x45, +0x46, +0x47; part loader 0x008d3a30) into detach events; `IDamagePartOwner` slots 1, 2, 7, 8 (0x00821c90, 0x0066b5a0, 0x005a3ce0, 0x0081d7b0); the 20-entry pop table (+0xa68) and `vtable +0x18` of the object at +0xf0; explosion fall-off. Blocks: part detachment parity.
2. **Effective health and damage types**: whether slot 69 (0x0056dcd0) runs after the `CDamageable` loader (max 750 vs 100); meaning of damage types 1, 4, 5 and of table ids; what `+0x36a` and `0x0056aec0` test; per-weapon damage values. Blocks: absolute damage numbers.
3. **Tyre trigger**: callers of `IDamagePartOwner` slot 4 with `shot` = true and slot 5; wheel scalar +0x184; the wheel-effect thresholds per kind (keys `#8b446354` `#7ef7211f` `#1e698c74` `#d2a98d2d` `#cb1cf725`); spare wheel models +0x19c / +0x1a0. Blocks: tyre shooting.
4. **Deformation writers**: the physics-object `vtable +0x94` (side, damage), 0x009b82f0 / 0x009bd6a0, doll object +0x180 floats +0x10 / +0x14, `CDeformTable` (0x00f21fb0), the upload of cb2 and `cb4[0].x`, the deform-decal copy 0x008cf600. Blocks: dents.
5. **Contact effect state machine**: the entry updater 0x007244c0 / 0x0058b9c0 / 0x00669140, the slide caller, the roll conditions, the second value per entry, the world-y 200.1 rule, boat extra call 0x009c3ea0, the stored-impulse rule (+0x2e4 with 0x00648720). Blocks: contact effect parity.
6. **Wreck look inputs**: the render-state constants behind `CMetaDataDeformLight` +0xd0..+0xf4 and `CMetaDataTwoTone` +0x5c..+0x7e, the instance tint bytes (+0x20, +0x21, +0x24..0x27), the link between the 0x008c51f0 array and the cb2 table, the two interpolated lists. Blocks: exact wreck appearance (successor of 4e39cad9).
7. **Air removal effect** 46 `arve_destroy_air` (id +0x490, function 0x0079eb20): key and trigger. Minor.
8. The lamp **light** side (flags read by 0x00526a40 and the lamp records): see JC2-207 (vehicle light placement).

## Paired implementation tasks (filed on the board; ready ones need only merged adapters)
* **FX-VEH-3 vehicle damage model**: parse the doll `CVehicleDamageModule` (`jc2-formats` vehicle.rs), max health and the type table, collision damage from avian contacts (3 * impulse / mass * multiplier, instant-wreck rule with the occupant factor), the 5 % hit clamp; hooks into `combat::Health`. Overlaps FX-VEH-1 (thresholds) and FX-VEH-2 (death): coordinate.
* **FX-VEH-4 collision feedback and contact effects**: 425 / 449 / 450 shake and haptics (player-occupied, impulse > 2000, 15 / 25 m/s bands), scrape 293 (normal . up > 0.9, input 0 = impulse), contact impact 67 / 254 and slide / roll through the effect system with the 200 m rule, helicopter rotor strike 195.
* **FX-VEH-5 lamps and detachable parts** (backlog: needs per-part collision bodies), **FX-VEH-6 tyre removal and rim sparks** (backlog: needs wheel hit tests and rim models), **FX-VEH-7 CarPaint damage layer, deform table and the wreck colours** (backlog: needs the CarPaint material milestone of JC2-018).

## Reproduce
```
python -B research/tools/vehicle_damage_effects_probe.py --game GAME --tool JC2TOOL --pseudo MAIN/research/analysis/pseudo/_unnamed
```
