# Explosion effects: blast, force pulse, camera shake, light, decal, vehicle death (task 55a08887, 12 October 2026)

Research only. Probe: `research/tools/explosion_effects_probe.py` (exit 0, 125 checks, 127 with `--census`; run with
`--game GAME --tool jc2tool --pseudo EXPORT`, add `--json` for the full catalogue; the disassembly checks need the
`capstone` Python package and are reported as skipped without it). Executable SHA-256 `df99a479...f74`.
Background: `particles-next.md`, `particle-consumers-next.md`, `point-light-model.md`, `weapon-fire-effects.md` (weapon to
impact id, per-surface selection tables, which are not repeated). The generic spawner (effect id -> graph -> emitters; component lifecycle in `effects-pipeline.md` section 1.3, which agrees with the class map below) is task
b0ed9f8e (fx-found); this document is the explosion *content and behaviour* layer on top of it. *Inferred* marks a reading of
data or code that was not traced to a consumer.

## 1. Effect-graph component types are native classes (proven)

The effect-graph loader **0x007f6070** reads the component list (`#f6a9f638`) and switches on each component's `type`:

| type | native class (ctor) | role |
|---|---|---|
| 0 | `CECD_Sound` (0x007c0070) | one-shot sound: `bank_id`, `sound_id` |
| 1 | `CECD_VisualParticleSystem` (0x005f54b0) | particle system by name; variants = per-surface choices |
| 2 | `CECD_VisualPostEffect` (0x006b0fc0) | no parameters, **unused in retail data** |
| 3 | `CECD_VisualDecal` (0x00740e70) | ground decal (`Decals/Explosion_*.dds`, size, lifetime) |
| 4 | `CECD_PhysicalForcePulse` (0x00678de0) | **radial impulse + camera shake + rumble** (section 2) |
| 5 | `CECD_HapticsRumble` (0x008469c0) | controller rumble |
| 6 | `CECD_InGameCutScene` | cutscene hook, no data parameters seen |
| 7 | `CECD_LightSource` (0x007e9350) | point light (see `point-light-model.md`) |
| 8 | `CECD_CameraShake` (0x006b67e0) | camera shake |
| 9 | `CECD_SoundPostEffect` (0x00505dc0) | ducking/post gain (`gain`, `attack`, `release`, `target`) |
| 10 | `CECD_AreaDamage` (0x00599960) | area damage; **no retail effect uses it** (probe: 0 graphs) |

Each class has an ECD (description) vtable whose slot 3 is its parameter loader: force pulse 0x007e94a0, camera shake 0x0052f700,
haptics 0x0071a110, area damage 0x005afbc0, and an ECI (instance) vtable whose slot 0 is Start (force pulse 0x005fc5f0, camera shake
0x0062b300). Census (probe): 37 effect files contain a force pulse, 11 a camera shake, 10 haptics, none an area-damage or post-effect
component. So **explosion damage is not in the effect data**: damage comes from the explosive records / weapon data (section 4) and
the effect only adds visuals, sound, push and shake.

## 2. Force pulse (type 4) and the camera shake it drives (proven code, inferred names)

Data fields of a type-4 component (hashed keys, DAT words at 0x00eba088..0x00eba094 checked by the probe):
`#41874e97` (float, **A**), `#4402eb79` (float, **C**), `#fb8c4ec3` (vec3, **half extents**); a second float `#1d454297` is read but absent
from retail data (default 0). Start (0x005fc5f0) constructs a `CForcePulse` (0x0062e5b0), places it at the effect transform, and sets
extents = the vec3 (0x00755f20 writes +0x4c..+0x54), **+0xb4 and +0xb8 = A**, **+0xbc = C**, then applies it (0x0060f680).
`CForcePulse` defaults (0x0062e5b0): +0xb4/+0xb8/+0xbc = 500, owner +0xb0 = -1, **+0xc4 = 0.5 and +0xc8 = 1.0** (never overwritten by the
effect path).

Apply (0x0060f680, text verified by the probe):
1. Query an AABB phantom (position +- extents, 0x009c0b60) and call 0x008074e0 per rigid body found: unit direction from the pulse centre to the
   body, then the body's virtual "apply pulse" with strengths A (+0xb4), B (+0xb8), C (+0xbc), the owner and `rand() * 3.05e-5 * 0.1` (a random 0..0.1
   factor); `CBreakableTreePatch` objects get the same call and are flagged. The exact unit of A/B/C (impulse or velocity) is not traced (Gap 3).
2. **Camera shake for every pulse**: if the camera is closer than **3.0 x max(extent)** (constant 0x0106fcf8) and the camera manager
   (`CCameraControlManager`, global 0x011900f8, built at 0x006f0250/0x00762eb0) exists, call 0x00698de0 with intensity input
   `+0xc4 * 0.5` (= 0.25), speed 1.5 and decay -0.1. 0x00698de0 sets the manager's accumulator **+0x24 = max(old, (input / max(1, 0.1 * distance)) * 0.2)**,
   +0x28 = speed, +0x2c = decay and colour/axes words 1,1,1 (0x0106a73c = 0.1, 0x0106a738 = 0.2). So a default pulse shakes at **0.05** inside 10 m and
   falls off as 1/distance beyond. Then the rumble manager (global 0x0118fb98) gets `+0xc8 * +0xb4 * 0.02 / max(1, distance)` through 0x008e1280.
3. Pulses created with owner < 0 also notify 0x008e5610 (stimulus/AI side, Gap 6).

Explicit camera-shake components (type 8; loader 0x0052f700; defaults from its stack): `#73440d3e` attenuate-by-distance flag (default 1),
`speed` (5.0), `scale` (0.5), `#f1e162dd` (0.1 = distance attenuation per metre), `#27750b42` (0.2 = gain), `#6afb428a` (-0.1 = decay). Start (0x0062b300):
intensity = `scale`, divided by max(1, distance x `#f1e162dd`) when the flag is set, then 0x0070d460 stores
`max(+0x24, intensity x #27750b42)`, speed and decay on the manager. Type 5 haptics fields: `#be0b5e5f`, `#263580f8`, `#015128a1`, `#3f1d2ee2`
(two left/right pairs, *inferred* from the pairing in the 0x0071a110 stores). Retail: `rumble_and_shake_light/medium/large` (ids 205-207: shake
scale 1, speed 5, decay -0.6, gain 0.02/0.04/0.08, haptics 0.75-1.0), `vehicle_collision_light/medium/heavy` (425, 449, 450), `char_melee_impact`
(327), `muzzle_bigcannon` (115), `muzzle_flak` (432), `km06_domefalling` (188), `f3m05_dishexplosion` (132). **Explosion effects themselves carry no
type-8 component**; their shake is the force-pulse coupling above, so a blast without a pulse (grenade, missile, vehicle effects 128/164) shakes only through
the code-built pulse (section 4) or not at all.

## 3. Explosion effect catalogue (data, proven)

One row per effect (probe `--json` -> `catalogue`): sound = bank/id of the type-0 component; light = peak multiplier, radius ("None" = derived from
c, l, q, see `point-light-model.md`) and the last key time; pulse = A / C / half extents; decal = size quad @ lifetime. `fx_` prefix omitted from particle names.

| id | effect | sound | particle system(s) | light | force pulse | decal |
|---|---|---|---|---|---|---|
| 35 | grenade_explosion | 11/13 | exp_grenade_01 + dirt / sand / snow / water variants | x8 r16, 0.2 s | - | 4.1-4.3 @100 |
| 411 | cluster_grenade_explosion | 11/21 | same five as 35 | same | - | same |
| 86 | grenade_explosion_air | 11/13 | exp_grenade_air_01 | derived, 1.2 s | - | - |
| 5 | missile_explosion | 11/9 | exp_missile_01 + exp_grenade_water_01 | x8 r10, 0.5 s | - | 3.1-4.3 @100 |
| 168 | missile_explosion_small | 11/9 | exp_fuel_small_01 | derived, 1.2 s | - | - |
| 20 | explosion_triggered_mine (C4) | 11/14 | exp_c4_01 | derived, 0.3 s | - | 6x6 @100 |
| 280 | c4_flash | 11/1 | c4_flash_01 | derived, 0.1 s | - | - |
| 19 | explosion_cloaked_mine | 11/12 | exp_mine_01 | derived, 0.4 s | - | 3.1-3.3 @100 |
| 377 | explosion_cannon_impact | 11/22 | exp_c4_01 | derived, 0.3 s | 500 / 8000 / (5,5,5) | 6x6 @100 |
| 431 | bullet_impact_flak | 17/40 | the five grenade variants | x8 r16, 0.15 s | - | 4.1-4.3 @100 |
| 128 | explosion_lave (car) | 37/57 | exp_vehicle_lave_medium_01 | x8 r12, 1.3 s (extra keys x0 at 1.3, x4 at 0.4) | - | - |
| 152 | explosion_lave_large | 37/63 | exp_firebomb_large_01 | x8 r20, 1.5 s | - | - |
| 164 | explosion_mc (motorbike) | 37/57 | exp_vehicle_lave_mc_01 | x8 r8, 0.8 s | - | - |
| 156 | explosion_seve (boat) | 37/64 | exp_vehicle_explode_boat_01 + lave_medium | derived, 1 s | - | - |
| 154 / 155 | explosion_arve_plane / _large | 37/65, 37/67 | exp_vehicle_explode_plane(_large)_01 | derived 2.1 s / x8 r70, 2 s | - | - |
| 157 | explosion_arve_helicopter | 37/65 | exp_vehicle_helicopter_2r_01 | x8 r27, 1.3 s | - | - |
| 43 / 44 | arve_land / arve_water_explosion | 37/63 | exp_vehicle_explode_ground_large_01 / water_splash_big_01 | x8 r30, 1.3 s / - | - | - |
| 46 / 87 | arve_destroy_air / explosion_vehicle_broken_air_fire | -, 37/18 | exp_metalchamber_small_01 / exp_vehicle_broken_air_fire_01 | derived 0.1 s / - | - | - |
| 27 | explosion_barrel | 37/15 | exp_fuel_small_01 | x8 r8, 1.5 s | 120 / 1000 / (5,5,5) | - |
| 14 / 15 | explosion_fuel_depot_small / large | 11/11 | exp_fuel_small_01 / exp_fuel_med_01 | derived 0.5 / 1.3 s | - / 250 / 800 / (5,5,5) | - |
| 126 | explosion_resource_fueldepot (209 is its prefire) | 37/55 | exp_resource_fueldepot_01 + _smoke_01 | derived, 4 s | - | - |
| 363 | env_gastank | 37/7 | env_gastank_01 | derived 0.8 s | - | - |
| 285 | gasstation_explode | 37/67 | gasstation_01 | derived 2.5 s | - | - |
| 82 / 83 / 97 / 84 | explosion_metalchamber huge / large / medium / small | 37/9, 10, 10, 12 | exp_metalchamber_*_01 | derived 1.1 / 1 / 0.7 / 0.6 s | 400/400/(11), 400/400/(8), 400/400/(5.5), 150/1500/(5.5) | - |
| 21 / 22 | explosion_silo_cylinder / _sphere | 37/10, 37/0 | exp_metalchamber_large_01 / exp_silosphere_01 | derived 0.5 / 2 s | 400/400/(8), 0/0/(11) | - |
| 75 / 76 | explosion_mortar dirt / snow | 11/10 | exp_mortar_*_01 | derived 1 s | 1000 / 5000 / (3.25) | - |
| 85 | explosion_firebomb_large | 11/11 | exp_firebomb_large_01 | derived 2.5 s | 400/400/(8) | - |
| 91 / 92 / 93 | explosion_electric large / medium / small | 37/3, 2, 1 | exp_electric_*_01 | derived 0.4 / 0.4 / 0.2 s | - | - |
| 26 / 149 | explosion_tire / _big | 37/35 | exp_tire_01 / exp_tire_big_01 | - | 0 / 400000 / (6.5) (26) | - |
| 28 | explosion_headlight | 37/21 | bulhit_headlight_small_01 | - | - | - |
| 33 / 194 | explosion_bombbox / thebombinthebox | -, 11/8 | -, test_fire_exp_01 | -, derived 1.2 s | 400/400/(6.5), 1000/400/(8) | - |
| 199 | explosion_crate | 37/23 | exp_crate_01 | - | - | - |
| 369 / 429 | sentry_gun_death / fx_char_explode_01 | 37/2 + 8/7, - | exp_electric_small_01 / char_explode_01 | derived 0.4 s / - | - | - |
| 0 / 1 | explosion / destruction_default | 11/8 | exp_grenade_01 | - | - | - |
| 3 | fireball | - | `firemedium_static` (**missing from particles.blz**) | - | - | - |

Notes. (a) Per-surface particle selection (Default/Soil/Sand/Snow/Water in 35, 411, 431; two variants in 5) uses the same `(flag, material id)` selection as the
bullet impacts (`weapon-fire-effects.md` section 3); the selection and parameter hand-off belong to fx-found. (b) The `exp_*` systems are large
(the grenade ground blast is a single dense chain; the shock/smoke/debris are *inside* the particle system and its child chain, not separate components), so
"debris spawn" and "smoke column" parity is a particle-runtime question, not an effect-graph one. (c) The scorch decal is the type-3 component (`Decals/Explosion_DIF/SPC/NRM.dds`,
lifetime 100, size quad e.g. 4.1/4.3/4.3/4.1), present on grenade, missile, C4, mine and cannon impacts only; vehicle and barrel blasts leave none.
(d) Light timings are the effect clock: black at 0, peak at the first key, black again at the last key (point-light-model.md has the shader side).
(e) `fireball` (id 3, referenced by `#ca9a42b4` on 22 weapons) names a particle system that does not exist; the original skips it, reproduce by skipping.

## 4. Which effect each source plays (what is proven)

| source | effect id | evidence |
|---|---|---|
| hand grenade WEA33 | **35** | `weapons.bin` explosives `detonation_effect_id`; loader 0x005d45a0 stores it in the explosive's id vector slot 0 (DAT 0x00eb23d4) |
| cluster bomb | **411** | same field |
| triggered charge (C4) WEA35 | **20**, plus **280 c4_flash** | same field; 280 is the static default at 0x01072ae0 used by the placed-charge update 0x00606350 (*inferred* role: the arming flash) |
| flare | -1 (none) | same field |
| rocket launcher, Panay, SAM | 5 | weapon impact id (`weapon-fire-effects.md`) |
| vehicle rockets | 168 (ARVE 5) | same |
| grenade launcher | 35 | same |
| cannons | 377 (BigCannon 97 = metalchamber_medium); flak 431 | same |
| barrels, crates, gas tanks, fuel depots, silos, buildings | per placed `CEffectPointEmitter` (key `#a90d6c5c` = effect id, event-driven) | census (probe `--census`): 70,709 emitters in 1,682 `.nlz`/`.ee` containers; most frequent explosion-ish ids: 92 (5845), 199 (5559), 27 (3919), 93 (3458), 84 (2271), 363 (2144), 83 (1531), 91 (1520), 15 (496), 126 (477), 285 (192) |
| vehicle tires / headlights | 26 or 149 / 28 | vehicle `.epe` keys `#bd438cc4` (26 on 57 vehicles, 149 on 2) and `#094ee3ff` (28 on 58) |
| vehicle death | the code-built pulse and flash below, plus the vehicle-class effect | see below and Gap 1 |

**Explosive detonation producers (code, proven structure).** The two explosive detonation bodies 0x005569a0 and 0x0069d750 (class per record *inferred*: hand grenade and triggered charge; C4 also 0x006d9fc0) play an effect
resource held in the object (`in_ECX[0x30]`, +0xc0; *inferred* to be the record's `detonation_effect_id`, the data field is proven, the copy into +0xc0 was not traced) through 0x0077f940, build a `CForcePulse` with extents (R,R,R), pulse A/B from the object's two virtual getters,
C = `in_ECX[0x2b]` and owner, apply it (0x0060f680, so **an explosive shakes the camera by the section 2 rule**) and finally call 0x008e5610 with the blast radius
(damage/AI stimulus, `CStimulusProvider_Explosion`). Bullets with an explosive payload do the same in 0x007579f0 (A = +4, B = +8, C = +0xac, extent = +0xe8).
How `damage`, `impulse` and `range` of the explosive records map to A/B/C/R is *not traced* (Gap 2). The weapons doc table lists `detonation_effect_id` 64 for the grenade; that is
wrong: 64 is `#14b862d2` = `handgrenade_warning` (the fuse warning), 61 `#5a9b3b81` = `grenade_trail` (slot 1 of the id vector), the detonation id is 35.

**Vehicle death (CVehicle slot 78 = 0x00559150, proven structure).** The routine (a) stops sound components, (b) builds a **code-defined `CForcePulse`** at the vehicle
position: extents (7, 7, 7) (constant 0x00ebeddc = 7.0), A = B = 300 (0x43960000), C = 500 (0x43fa0000), owner = the byte at vehicle +0x36e, applies it
(so it pushes bodies and shakes the camera inside 21 m by section 2) and (c) inserts a managed three-key point light through 0x00942ec0: black at 0, colour (2.0, 1.4, 0.6) at 0.05 s,
black at 1.5 s, (c, l, q) = (0.1, 0.008, 0.04), priority 0. **JC2-210 / G9 answer:** key word 10 (explicit radius) of the three keys lives at stack slots ebp-0x118, ebp-0xe4, ebp-0xb0 and
is **never written**: disassembly shows no instruction touching those slots, and the only objects whose address is taken near them (the `CForcePulse` at ebp-0x300, the shared pointer at ebp-0x150) do not
cover them. The key is only "derive from c, l, q" when the word is exactly -1.0 (`point-light-model.md` section 3), so the radius is **stack residue**: the 9.77 m derived
radius is *not* what the code guarantees, and the residue cannot be computed statically (it depends on the call chain below the three wrappers 0x00647bd0, 0x0079ed50, 0x007ff860). Status: unresolved (Gap 4).
Every other key word is written (position, colour, c/l/q, time 0 / 0.05 / 1.5, priority 0); only word 10 is not.

## 5. Current runtime vs original

FX-EXP-1 (board be4654ec, implemented): `explosion.rs` `Blast` carries an `effect` id (`explosion::effect_id`: grenade 35 and
rocket 5 are the sources the game has; cluster 411, small missile 168 and cannon 377 are defined but unused; the red barrel plays
27 per section 4's emitter census; a burnt-out vehicle plays 128 *inferred* as the car-class default, other classes need the
per-vehicle paper-doll lookup of `vehicle-fire-explosion.md`). `detonate` plays it through `EffectSystem::play` at the blast
point with the `(flag, material)` pair: water `(0,1)` below `SEA_LEVEL`, else `(1, game material of the terrain tile)` from
`audio::Ground`; missing particle names are skipped by the spawner. The stand-in `missile_explosion` light is gone; the old placeholder fireball sphere (0.7 s) is KEPT as a labelled non-native
fallback (`explosion::FALLBACK_FIREBALL`, one constant) because the native grenade effect still renders small and faint (particle
size/velocity units unresolved, board dc95ceb0); remove it when that is fixed (the light now comes from each effect's own type-7 component; the keyed-light stand-in is
used only if the effect registry could not be read). Update (task f986e6ce): the decal (type 3, forward decal), force pulse
(camera and rumble coupling, no impulse), camera shake and haptics adapters exist (`effects-pipeline.md` section 12); the shake accumulator is read by 0x005e7b90 (`camera-shake.md`). Still not visible: sound (type 0; `audio.rs` still picks its own blast sound). Wreck smoke
puffs, the debris speed and the blast damage/push numbers are unchanged (Gap 2). Opt-in capture fixture:
`JC2_BLAST_FIXTURE=<delay>` (+ `JC2_BLAST_FIXTURE_DIST`) sets off one grenade blast in front of Rico. Not done: before/after
captures (see the board comment): the particle billboards were not visible in the hero-mode world in a test run, while the
effect and its 100+ particles were created and simulated (log: `particles drawn: 332`).

Update (task 94cc0f60, FX-EXP-2): code-built pulses are queued through `PulseQueue::queue_pulse`. Vehicle death (`wreck_vehicles`) queues the native pulse (extents 7, A 300, C 500; shakes inside 21 m) and spawns the native three-key flash (2.0/1.4/0.6, 0/0.05/1.5 s, c/l/q 0.1/0.008/0.04) with the radius as the labelled NON-NATIVE constant `VEHICLE_FLASH_RADIUS` = 9.77 m (the derived value; Gap 4). Explosive blasts of effects 35/411/5/168 (no pulse component of their own) queue a pulse with extents (R,R,R) per 0x005569a0 / 0x007579f0; pulse A is the record impulse (NON-NATIVE stand-in, Gap 2; only feeds the unread rumble term). Impulses on bodies stay with the tuned `explosion.rs` push (Gap 3). Scorch decals come from the effects' own type-3 component (already adapted). No before/after capture: the change is camera shake and a short light, not a static frame.

## Gaps (unresolved; none guessed)
1. **Vehicle class -> explosion/damage effect ids**: *answered* in `vehicle-fire-explosion.md` (task bd393831): the ids live in each vehicle's paper doll (`*.mvdoll` / `*.vdoll`, `CEffectProp` nodes with a slot enum), not in `.epe` or `.bin` files; the table, the damage state machine and the death routine are there.
2. **Explosive record -> pulse mapping**: how `damage`/`impulse`/`range`/level map to the pulse A/B/C/extents and the 0x008e5610 damage radius; the blast damage falloff. Blocks: gameplay-accurate push and damage (current `explosion.rs` is tuned).
3. **Pulse strength unit**: the virtual at body vtable +0x20/+0x24 (rigid body, character ragdoll, vehicle) that consumes A/B/C and the random 0..0.1 factor. Blocks: how hard blasts throw cars and ragdolls.
4. **Vehicle death flash radius** (JC2-210 / G9): stack residue, not derivable statically; the analysis and the dynamic recipe are in `vehicle-fire-explosion.md` section 7. Blocks: vehicle explosion light size.
5. **Camera-shake waveform (camera part resolved 12 Oct 2026, `camera-shake.md`; only the rumble reader remains)**: the consumer of the `CCameraControlManager` accumulators (+0x24 intensity, +0x28 speed, +0x2c decay) and the rumble accumulator (0x0118fb98 +0x1c). Blocks: shake amplitude/frequency in metres/degrees.
6. **0x008e5610 / `CStimulusProvider_Explosion` / `CAIExplosionHandler`**: the AI/heat side of a blast (witnesses, `HitByExplosion`). Blocks: NPC reactions (covered partly by heat research).
7. **`exp_*` particle chains**: the chain structure (children, shockwave/smoke/debris) of the explosion particle systems was not inspected; they run on the particle runtime (fx-found).
8. **Placed-emitter event semantics** (`event_enable/event_disable/event_stop`, pre-fire effects 209/239): fx-found.
9. **Sound banks 11 and 37** (explosion sound ids above) need the FSB mapping (audio task).
10. **Meaning of `#ca9a42b4` (fireball id 3)** and the missing `firemedium_static` (carried from `weapon-fire-effects.md` Gap 6).

## Paired implementation tasks (filed `backlog`, tags effects, priority high; they depend on the effect spawner b0ed9f8e)
* **FX-EXP-1 explosion visuals**: replace `explosion.rs` placeholders by spawning the real effect id per source (grenade 35, cluster 411, C4 20 + 280, rocket 5, small 168, cannon 377, tires 26/149, headlight 28) with surface selection, decal, light and sound; skip missing particle names.
* **FX-EXP-2 force pulse and camera shake**: implement the type-4 component and the code-built pulses as one `ForcePulse` (AABB query, directional impulse A/B/C, owner, the 3x-extent camera shake rule with the 0.2/0.1 constants, rumble); vehicle death pulse (7 / 300 / 500) and the 2.0/1.4/0.6 flash with the radius left as an explicit labelled unknown.
* **FX-EXP-3 placed destruction emitters**: drive `CEffectPointEmitter` effects (barrels 27, gas tank 363, fuel depots 15/126, crates 199, building 83/84/92/93) from the destruction events instead of the name heuristic in `destruction.rs`.
