# Weapon-fire effects: muzzle, shells, tracers, impacts (task e4083e96, 12 October 2026)

Research only. Probe: `research/tools/weapon_fire_effects_probe.py` (exit 0, 65 checks; run with
`--game GAME --tool jc2tool --pseudo EXPORT`, add `--json` for every table below). Executable
SHA-256 `df99a479...f74` (see `weapon-sockets-next.md`). Graph/particle format background is in
`particles-next.md`, `particle-consumers-next.md`, `point-light-model.md`; the general effect
pipeline is task b0ed9f8e (fx-found) and is not repeated here. Everything marked *inferred* is a
reading of data, not a traced consumer.

## 1. Weapon -> effect ids (proven: data + loader)

`weapons.bin` has no key named "muzzle" or "impact"; the two hashed keys are:

| key | CWeapon-type field | meaning | evidence |
|---|---|---|---|
| `#04374f9d` | +0x2e8 | **muzzle effect id**, -1 = none | loader stores it at `in_ECX + 0xba` (hash word `DAT_00eb2678`); consumer 0x006fae80 tests `!= -1` and passes it to the effect-by-id lookup 0x0062a7a0 |
| `#469ba70d` | +0x2e4 | **bullet/projectile impact effect id** | loader stores `in_ECX + 0xb9` (`DAT_00eb2674`); every value names an impact/explosion effect in `effectsettings.bin` (data-level proof; see Gap 1 for the bullet-side copy) |
| `#ca9a42b4` | (second loader, 0x007e0000 export) | 3 = `fireball` on 22 weapons, -1 on the rest | role unknown (Gap 6) |

Both keys exist on all 49 weapons and every id resolves in `effectsettings.bin` (probe). Table
(weapon id = `id`; the player's guns in bold). Tracer = `tracer_fire_velocity` / `tracer_scale_factor`.

| weapon (id) | muzzle id -> effect | impact id -> effect | tracer |
|---|---|---|---|
| **Beretta (2)** | 48 muzzle_beretta | 2 bullet_impact_small | 100/0.3 |
| Revolver (4) | 269 muzzle_revolver | 2 | 90/0.4 |
| **Scorpion SMG (5)** | 51 muzzle_smg | 2 | 100/0.3 |
| Onehand shotgun (6), **pump shotgun (13)** | 49 muzzle_shotgun | 2 | 100/0.4 |
| **Assault (11)** | 50 muzzle_assault | 36 bullet_impact_medium | 120/0.2 |
| Sniper (14) | 50 muzzle_assault | 36 | 600/0 |
| **RocketLauncher (16)**, Panay (66) | 53 muzzle_rocketlauncher | 5 missile_explosion | 100/0.5 |
| GrenadeLauncher (17), V039 GL (139) | 54 muzzle_grenadelauncher | 35 grenade_explosion | 100/0.5 |
| Minigun/Vulcan family (26), sentry (32) | 52 muzzle_minigun | 37 large (Vulcan 40 huge; ARVE 161/169 `_pc`) | 150-200 |
| MachineGun (28), HeavyMG (129) | 50 muzzle_assault | 36 | 150-200 |
| Vehicle MGs (128) | 50 / 52 / 49 per vehicle | 36, 37 or 169 | 150-200 |
| Vehicle rockets (116) | 53 | 168 missile_explosion_small (ARVE 5) | 100-200 |
| SAM (31) | 255 muzzle_rocketlauncher_large | 5 | 100/0.5 |
| Cannons (34, 134) | 115 muzzle_bigCannon | 377 explosion_cannon_impact (BigCannon 97) | 400-800 |
| Flak (34) | 432 muzzle_flak | 431 bullet_impact_flak | 500/0.05 |
| Bubble gun (43) | 414 bubblegun | -1 | - |
| Grappling hook (52) | -1 | 212 hook_impact | 100/0.5 |

This **closes the JC2-221 gap**: the three light-carrying muzzle effects (115, 255, 432) are the
vehicle cannons, the SAM and the flak gun. Hand guns have no muzzle light (probe asserts lights
exist only on 115/255/432).

## 2. Native shot chain (proven: probe asserts the decompiler text)

* 0x006ec6e0 (per-shot) -> 0x006fae80 muzzle effect, 0x0072aae0 builds the bullets, 0x00765b30 (continuous fire, minigun-type) -> 0x006fae80 again.
* **0x006fae80**: if the type's +0x2e8 != -1, lazily creates a `shared_ptr<IEffectController>` at weapon+0xe10 from the id (0x0062a7a0 -> 0x00523240), then every shot restarts it: transform from the emission frame at weapon+0x164 (0x00631d00), a second setter (0x00780690 on weapon+0x228), then play (0x0072beb0). So the muzzle effect is **one persistent controller per weapon, restarted per shot**, not a new instance.
* It is skipped when the weapon's owner flag (weapon+0x728, via 0x006175a0, meaning unresolved) says so in 0x006ec6e0; the exact local-player path is Gap 3.
* **0x0072aae0**: per bullet (`bulletspershot`, type +0x94) draws three `rand()` offsets scaled by type +0x2d8 (+0x2a4 when the shooter is not the local player), builds the aim matrices and calls the bullet factory **0x004f9560**; type +0x2dc == 2 stops after one bullet.
* **0x004f9560**: creates a `CBullet`, and a `CBulletTrail` (0x0056f420) only when `(int)type[+0x19c]` divides the weapon's shot counter (`#04f3f8c6`, stored as 1/x by the loader; 1 = every shot, 0 = never: rockets, grenades). The tracer takes type +0x1a0 (`tracer_fire_velocity`) and +0x198 (`#57d5fa61`). Loader offsets: +0x1a4 `#6837be43`, +0x1a8 `#84ecc4af` (impact force), +0x1ac `tracer_scale_factor`, +0x1b0 `#a5d17f96`, +0x1b4 `#409d3556` (1 only on the ARVE/LAVE cannons).
* **Correction (FX-GUN-3, 12 October 2026)**: 0x00657600 is not a draw routine. It is the bullet manager's initialiser: it preloads `bullet_trail_new_dif.dds` and `bullet_tracer_dif.dds`, allocates two 0x1800-vertex (20-byte) buffers, and reads `ammunition.bin` (`max_ammo_N`, `whiz_sounds`, `ricochet_sounds`). `CBulletTrail` (ctor 0x0056f420, size 0x124, vtable 0x00eba7bc, 17 slots) is a moving object, not a mesh: its update (slot 5, 0x005d8c50) advances it, ends it when elapsed time exceeds range/speed (+0xa0/+0xa8), and, below altitude 200, subtracts a global (0x0106d518, raw file value 200.0, meaning unresolved) times dt from the speed. The trail's draw path (reading the manager's vertex buffers) was not located; slots 0-16 contain no vertex writer. `bullet_trail_new_dif.dds` (128x512 DXT5) is a glow head near the top with an orange streak running down: it is the sprite for a trail. `bullet_tracer_dif.dds` (32x64) is a short white capsule whose consumer is unknown.
* Impact: `CBullet` (vtable of 0x006faa90) keeps its impact effect id at **bullet+0xc0** and fires it through 0x00658e10; terrain hits (0x00613eb0) play it only when `rand <= surface record +0x60` and end the bullet on a ricochet test with surface record +0x5c; the water hit (0x0068fec0) always plays it with the water pair. Parameter pair handed to the effect: water = (0, 1), ground = (1, surface material id) (`water-splash-impact.md` section 8).

## 3. Effect content (data, proven)

### Muzzle graphs
Each `muzzle_*.bin` is one particle component (type 1, variant id 0) and no light, except 115, 255, 432. The particle system is the head of a chain: `#841408bf` plus four more child slots (`#f47c95e6`, `#02bcc1f0`, `#00e0c94d`, `#0f6944c8`) name further `fx_*` systems (probe prints every chain). Textures: `muzzleflashes.dds` (flash cells; the per-system 8x4, 4x2 grids are the two shorts at +168/+16a of `particle-consumers-next.md`), `fumefx_bomb_01.dds` (smoke), `bulletshells.dds`, `spark_dif.dds`, `particle_blobb.dds`.

* Beretta 48: `fx_muzzle_gun_01` (**bulletshells.dds**, particle life 1 s, system life 0.1 s, physical force + damping = the ejected casing) -> child `fx_muzzle_gun_02` (flash, 0.1 s).
* SMG 51: `fx_muzzle_smg_01` (shells) + flash children 02-06. Assault 50: flash `fx_muzzle_assaultrifle_01` + shells `_02` + flash variants 04-07. Minigun 52: flash 01, smoke 03, flash 04, shells `fx_muzzle_minigun_08`, plus the assault chain. Shotgun 49: smoke 01 (0.5 s) + five flashes (no shells). Revolver 269: flash + `fx_muzzle_gun_03` smoke (no shells). Grenade launcher 54: smoke + blob + smoke. Rocket 53: `fx_fire_missilelaunch_01..03` (smoke, blob, `distort_normal.dds` heat distortion).
* **Shell ejection is not a separate mechanism**: casings are `bulletshells.dds` particles inside the muzzle chains above. `eject_clip_gun` (264) and `eject_clip_rifle` (265) are `fx_eject_clip_*` (`metalflake.dds`); no weapons.bin key points at them (Gap 5).
* Lit muzzles: 115 (offset (0,1,0), radius 10, (1,0.65,0.2) x8, 0 -> 0.2 s), 255 (key times 0/0.3/0.35/0.75, (0.5,0.25,0)->(1,0.5,0)), 432 flak (radius 10, 0.1 s). Values are in the probe's `light_nodes`; component layout per `point-light-model.md`.
* **Retail shows lit ground under pistol fire** (`reference/combat/pistol-muzzle-flash-lights-ground-yellow-..._ai16.jpg`, `pistol-fire-flash-lights-grating-...o14.jpg`, smg `...af04.jpg`) although `muzzle_beretta` has no light. The matching asset is **`bullet_fire.bin` (effect id 10): a light-only graph, radius 3, (1, 0.65, 0.2) key 0 -> black**. Its producer was not found (Gap 2); do not give hand guns a light of any other colour/radius.

### Impact graphs (`bullet_impact_small/medium/large/huge/_pc`, ids 2, 36, 37, 40, 161, 169)
Structure (all of them, probe-checked on 2/36/37/40): components = sound (type 0, bank 17), particle (type 1, ~16 variants), decal (type 3), two lights (type 7, comps 3 and 4). Selections (hash `#84085477`): `#ea0e1560` is a **material table keyed by surface-material id** (32-34 entries: Sand 0, Soil 1, Snow 2, Gravel 3, Stone 4, Ice 5, Wood 6, Water 7, Metal 8, Glass 9, Paper 10, Plastic 11, Rubber 12, Textile 13, ..., Metal_Car 21, Asphalt 30, Grass 34); each entry holds slot -> `"component id:variant id"`. `#a28df775` is the fallback when the material has no entry (dirt particle `1:8`, dim light `4:0`, sound 0:0) and `#55ba2d5a` the water-flag selection (water particle `1:3`, water sound `0:3`) — the water/material reading follows the `(0,1)`/`(1,id)` pair; the exact slot hashes are *inferred* (loader belongs to fx-found). Slot keys: `Sound` type 0, `#54236b22` type 1, `#bbc51e2b` type 3, `#570310c8` type 7.

Resolved example (probe `impact_36_resolved`): Metal (8) -> `fx_bulhit_metal_medium_01`, bright light, decal Metal, sound bank 17/id 5; Soil -> `fx_bulhit_soil_medium_01`, no light, decal Dirt; Glass -> `fx_bulhit_glass_medium_01`; Flesh/Textile/Veg/Water/Sand/Snow/Grass/Electrical/SteamMetal/Wood/Stone each have their own `fx_bulhit_<surface>_<size>_01`.
* **Lights**: comp 3 = (1,0.5,0) x8, comp 4 = same colour x2; radius small 1.5, medium 2, large 3, huge 5 (`_pc` 5); offset (0,0.2,0) (huge 0.75); key 0 colour -> key 0.05 s black. Metal-like materials select the x8 light, the fallback the x2 light, soil/stone/wood none.
* **Decals**: `Decals/BulletImpacts_DIF/SPC/NRM.dds`, one atlas cell per material (cells quarter-width, half-height: a 4x2 atlas, *inferred* from the uv pairs), world size by class: small 0.05-0.2, medium 0.1-0.4, large 0.25-1.2, huge 0.4; lifetime 10 s (small, medium), 30 s (large), 100 (huge); the size vector's last two entries 0.3/0.3 are unexplained.
* **Sounds** bank 17, sound id per material class (Metal 5, Stone/Dirt 2, Wood 8, Flesh 3, Water 7, Glass 4) with raw spacing fields; hand-off to the audio task, bank 17 needs its FSB mapping.
* Heavier impacts: 377 `explosion_cannon_impact` (`fx_exp_c4_01`, decal `Explosion_DIF`, light, a type-4 and a type-9 SoundPostEffect component, bank 11/22), 5/168/35/431 explosion graphs (missile 0.5 s light radius 10; grenade radius 16, 0.2 s), 212 `hook_impact`.
* Character hits (`character_hit*` 6, 38, 39, 330, 331: `fx_bulhit_flesh*`, `fx_bulhit_headshot_01`, `headsniper`, `headremove`, bank 17) are separate effects; how the game picks them over the surface impact is not traced (Gap 1).
* **Data holes**: the impact graphs name `fx_bulhit_flesh_{small,medium,large,huge}_01`, `fx_bulhit_dirt_medium_01` and `fireball.bin` names `firemedium_static`, none of which exists in `particles.blz` (2600 systems; probe lists them). The native loader must skip them (matches "skips id -1"/missing); reproduce by skipping, not by substituting.

## 4. Current runtime vs original (`crates/jc2-game/src/combat.rs`)
Coloured-cylinder tracers (0.06 s), crossed-quad flash (0.05 s, no shells, no smoke), no impact effect, no decals, no impact lights, no bullet_fire light. Everything above replaces these.

## Gaps (unresolved; none guessed)
1. Native copy of weapon-type +0x2e4 into `CBullet+0xc0` (initialiser not located), and the choice between surface impact and `character_hit*` for flesh targets. Blocks impact parity on soldiers.
2. Producer of effect id 10 `bullet_fire` (the pistol ground light seen in reference shots). Blocks muzzle-light parity for all hand guns.
3. Meaning of weapon+0x728 and +0xe18 in 0x006ec6e0, i.e. exactly when the local player's muzzle effect is started.
4. `CBulletTrail` draw path (not 0x00657600, see the correction in section 2) and the fields +0x198/+0x1a4/+0x1ac/+0x1b0 -> tracer length/width/colour; the consumer of `bullet_tracer_dif.dds`; the meaning of the speed-loss global 0x0106d518. Blocks exact tracer geometry; the port's quad size/blend are inferred.
5. Consumer of `eject_clip_gun`/`eject_clip_rifle` (264/265) (likely reload animation events).
6. `#ca9a42b4` (fireball id 3) role; `fireball.bin` itself references a missing particle.
7. Slot-key/selection semantics (`#a28df775`, `#55ba2d5a`, the six slot hashes) are inferred; fx-found owns the loader trace. Material id -> name table beyond the 35 observed entries and surface record +0x5c/+0x60 (ricochet / spawn probability) values.
8. Sound bank 17 / 11 mapping to FSB events.

## FX-GUN-1 implementation notes (task 446ad726, 12 October 2026)
Built: `Weapon::muzzle_effect` (`#04374f9d`, -1 none; tests: Beretta 48, SMG 51, Assault 50), `crates/jc2-game/src/muzzle_fx.rs`
(one `EffectController` per weapon: the player's gun slot, a soldier's own gun; every shot sets the emission-frame transform and
calls `play_controller(restart = true)`, native 0x006fae80). `combat.rs` queues `MuzzleShot`s in `Arsenal::muzzle_shots`; the crossed-quad
placeholder flash is used only when no effect system exists. Hand guns get no light (Gap 2 unchanged). Captures: `_extract/446ad726/`
(SMG, world 31,14; effects slowed to 1/20 speed to catch the 0.09 s flash on a 4 fps machine).

Findings that needed two inferred particle-runtime changes (both labelled in code, neither native-proven):
* The one-particle flash leaves (`fx_muzzle_smg_02..06`, assault, shotgun) have emission rate 1/s and a 0.09 s window; an emitter that
  births only after one spacing never shows them. The first particle is now born on the first step (accumulator primed).
* A controller restart (`Start`) while the previous start's emitters are alive kept running, so the 1 s shell casing of the head leaf
  blocked every restart of a gun firing faster than 1 shot/s. Emitters still emitting keep running; ones whose window is over are retired
  (their particles finish) and the chain restarts.

Gaps: (10) the effect-local forward axis and roll of the emission matrix (weapon+0x164) are unresolved; the port passes yaw/pitch of the barrel
(-Z), so shell/smoke drift direction is inferred; (11) native "emitter alive" test for Start restart and the first-birth timing of rate-1
emitters (particle runtime, Gap 2 of the renderer notes in `effects-pipeline.md` section 11); (12) the muzzle frame for vehicle weapons and
the AI's weapon owner flag (+0x728) are not covered (only the player's guns and soldiers' hand guns use the effect).

## Paired implementation tasks (to be filed `backlog`/`ready` by the orchestrator; tags effects, priority high)
* **FX-GUN-1 muzzle effects**: parse the `#04374f9d` id, resolve graph -> particle chain, drive it from the existing preview particle runtime at the emission frame, one restartable controller per weapon; shells included. Owned: `jc2-formats::weapons`, `jc2-game::combat`, `particle_fixture`/new effects module. Capture before/after per weapon.
* **FX-GUN-2 impact effects**: parse `#469ba70d`; on a hit resolve (surface material id or water) through the graph's material table; spawn particle, decal (4x2 atlas), x8/x2 light (`point_lights::NativeLightEffects`), skip missing particle names. Depends on material ids from the physics hit (Gap 7).
* **FX-GUN-3 tracers**: done as `crates/jc2-game/src/tracer_fx.rs` (see below).

## FX-GUN-3 implementation notes (task aa9947ad, 12 October 2026)
Shell casings: nothing separate to implement; they are the `bulletshells.dds` particles of the muzzle chains (section 3) and play through `muzzle_fx.rs`.
Tracers: `Weapon` now carries `tracer_fire_velocity`, `tracer_scale_factor` and `tracer_every_shot` (`#04f3f8c6` non-zero: every retail value is 1 or 0; 0 on rockets and grenades). A shot makes a moving sprite (crossed quads, `bullet_trail_new_dif.dds`, additive) from the muzzle to the end of the shot at `tracer_fire_velocity` (native: trail speed, 0x004f9560). Replaces the coloured cylinder that lasted 0.06 s.
Inferred (not native): quad size 2.4 x 0.6 m, head position inside the texture (v 0.12), crossed-quad billboard, additive blend; `tracer_scale_factor` and the other tracer fields are loaded but unused; the altitude speed loss is not applied.

## FX-GUN-2 implementation note (task 2974c809, 12 October 2026)

`weapons.bin` `#469ba70d` is parsed (`Weapon::impact_effect`). The player's bullet hits are queued by
`combat::fire` and played by `impact_fx.rs` through the spawner: terrain hits pass the tile's game
material (`audio::Ground`) as the `(1, id)` pair, a ray crossing the sea plane passes the water pair
`(0, 1)` at the crossing point (the terrain/object hit below it is suppressed). Particles and lights come
from the existing adapters, sound from the sound adapters; decals stay a stub. Not done (gaps): surface ids
for non-terrain colliders (props, buildings, vehicles play with no interaction, i.e. the fallback
selection); character hits are skipped (Gap 1); play probability (+0x60) and ricochet (+0x5c) unknown, every
hit plays (Gap 7); the effect matrix uses the reversed bullet direction as the normal (inferred); soldiers'
shots raise no impacts. No before/after capture: the in-game firing path cannot be driven headlessly.
