# The player character

Implemented in `crates/jc2-formats/src/player.rs` (settings) and
`crates/jc2-game/src/character.rs` (the controller); `jc2tool prop <game> 'settings\player.bin'`
and `… 'settings\grapplingparams.bin'` dump the files.

## Where Rico is

| What | Where | Notes |
|---|---|---|
| Settings | `settings\player.bin` | `model = "Characters\MainCharacters\Rico\mc01_lod1-rico"` (no extension), `#26fa86fe = "animations\skeletons\syncreadthis\biped.bsk"` (the skeleton), the animation sets (`kc001_base.asb`, `kc001_upperbody.asb`, `kc001_leftarm.asb`, `additivelayer.asb`) and state machines (`BaseStateMachine.afsm`, …), `#05aaa9b5 = [1281, 663, -12143]` (a position — the default start?), `#2c121d50 = "settings\rico_voiceover.bin"`, `#b868e836 = "Effects\CharacterEffectSettings\RicosEffects.bin"`; ~80 hashed numbers (speeds, timers) still unnamed |
| Doll | `exported\cdoll\mc01_rico.ee` | `mc01_rico.cdoll` (a property file: the `.lod`, the skeleton, and a texture list — `mc01_clothes_dif`, `mc01_hair_dif`, `mc01_armgrappling_dif`, `mc01_parachute_dif`, …), `mc01-rico.lod` (five levels, all the same file), `mc01_rico.ee.epe` |
| Model | `characters\maincharacters\rico\mc01_lod1-rico.rbm` | a top-level archive file (hashed by its bare name, like everything else): 16 `SkinnedGeneral` blocks, 12 801 vertices, 16 064 triangles, bind pose with the arms out, 1.82 m tall, feet at the origin |
| Textures | `characters\maincharacters\rico\mc01_*.dds`, `ava_eye02_*.dds` | top-level; `_dif`, `_nrm`, `_mpm` |
| Hook | `settings\grapplingparams.bin` → `hook_model = "Models/Weapons/GAE04/WEA04-Grapplinghook.lod"` | the `.lod` and `wea04_lod1-grapplinghook.rbm` (146 vertices) live in `general.blz` with `hook_diff/nrm/mpm.dds`; `wea04_lod1-a.rbm` there is the launcher on his arm |
| Parachute pack | `general.blz/gae06_lod1-a.rbm` | the backpack (76 vertices, 0.28 m wide) with `gae06_parachute_dif/nrm/mpm.dds` (11 KB each, a small atlas of the pack's straps and fabric — **not** a canopy texture; laid over the lofted canopy it looks like a brown sheet with a white outline drawing), `gae06_lod1-parachute_col.pfx` its collision |
| Canopy | — | **not in the data, model or texture**. `jc2tool scan-rbm <game> --names <substr>` lists every file, every member of every container (nested ones too) and every model with a texture whose name contains the substring: `chute` finds 142 hits — the animations, `parachute.bsk`, sounds, effects, the cutscene skeletons (`cutscene_parachute.bsk`, `km05_02_*_obj_amparachute.ban`, which animate a chute object whose model is not there either), the DLC heavy-drop `.epe` files (entity blackboards, no model) and `gae06_*` — and the only model with a `*chute*` texture is `gae06_lod1-a.rbm`; `para` and `canopy` add nothing (`jungle_canopy_*` are trees, `pd_ms_paparazzi` a pedestrian). `mc01_rico.cdoll` lists `mc01_parachute_dif` (and `mc01_armgrappling_dif`) among its textures but neither file exists: `characters\maincharacters\rico\` has only `mc01_clothes/hair/shirt_*`, `ava_eye02_*`. `settings\parachuteparams.bin` (`parachute00`, `model = ""`, the skeleton, `parachute.asb`, `ParachuteEffects.bin`) names no model either. `parachute.bsk` is a 17-bone skeleton: `base` 1.23 m above the harness, three chains of four bones (`lft_1..4`, `mid_1..4`, `rgt_1..4`) fanning up and out from it — the canopy's left edge, centre line and right edge — plus `Para_Left/RightHandle` and `HarnessExt`; the `chute_*.ban` clips (interleaved, `animation.md`) animate those chains (`chute_open_chute` unfolds them). The game builds the surface at runtime (`CAnimatedParachute`; `hkFxCloth*` suggests cloth). `jc2-game/src/parachute.rs` lofts it: an arch through the three chain tips extruded 2.6 m front to back (the depth is a guess), in a plain red-orange fabric colour (no texture; the colour is a guess), the lines along the chains, unfolding with `chute_open_chute` (0 → 3.94 m span in 2.4 s). |
| Other characters | `exported\cdoll\*.ee` | `mc02_jadetan`, `mc03_generalmasayo`, `pd_*` (pedestrians): same layout, the models next to them in the archives |

## `grapplingparams.bin`

Found by hashing candidate names against the keys (`jc2tool hash`):

| Key | Name | Value |
|---|---|---|
| `2a925441` | `pull_force` | 500 — the pull on a hooked object |
| `b55417ae` | `pull_force_character` | 850 |
| `537a05d4` | `reel_in_speed` | 40 |
| `e6aaf3be` | `wire_width` | 0.006 |
| `4e3f0a7d` | `hook_model` | the `.lod` above |
| `72104687` | ? | 8100 = 90² — taken as the maximum range squared (assumed) |
| `59e07a13` | ? | 120 — taken as the hook's flight speed in m/s (**assumed**: the key has no name; among the unnamed 50, 50, 120, 140, 40, 500 it is the one that fits) |
| `76a95a64` | ? | 0.91 (cos 24.5°) — kept as `detach_cos`, unused (assumed to be an angle) |
| `c42c3b5c { … }` | the hook's impact | `hook_material = "Metal"` (`2300df95`), `damage = 76`, **`character_impulse = 200`** and **`object_impulse = 200`** (found by hashing; N·s), and 1000, 327 (unnamed) |
| `ca400f12 { … }` | `grappling_device_effects` | eight effect ids (203 – 215, 410) |
| `0e62e301`, `96e5b772`, `e6a244b4` | wire textures | `Textures/Wire_mpm/DIF/nrm.dds` |
| `2b8dfb8d`, `cad60785`, `442da943`, `ae2d91e0` | hook textures | `Hook_Diff/Nrm/Mpm/Dummy.dds` |
| `1cb6ba12 { … }` | ? (`auto_detach_limits` hashes to it, a three-word hit) | ten numbers under ten unnamed keys. **Not weight thresholds**: the loader (`FUN_00802ec0`) stores them in `DAT_0106ce58…ce78`; `0106ce5c…ce68` and `ce6c…ce78` are two sets of four angle limits in degrees (55/100/60/60 and 65/65/70/80, picked by a flag) used by `FUN_0082cbe0` as `180 − x` / `x + 180` wire-direction limits, `44ad22f7` = 3 (`ce58`) and `fa0b2b36` = 0 (`011f9c78`) are an alternative pair `FUN_0070e400` (a ray sweep around the aim) picks between |
| `00992cfb`, `320b8cd2`, `8e43b31e`, `7a4ef581`, `d57278d7` | ? | 50, 50, 140, 40, 500: unnamed, unused (a search of one to three words of the exe's vocabulary found nothing) |
| `76fea4b2`, `7dd628ea`, `a73771bf`, `da24282b` | ? | 2, 1, 3, 16 |
| `actions { … }` | the grappling actions | `priority`, five hashed flags each |

## In jc2-game

* Body: a dynamic capsule (1.8 m, r 0.3, 80 kg, rotation locked, zero friction) with a
  sphere cast down for the ground; walk 2.6, run 5.5, sprint 8 m/s on the ground plane;
  jump 5.5 m/s; slopes over 50° are walls.
* Camera: an orbit behind him, pulled in by a ray when something is in the way.
* Grapple (`grapple.rs`; the state is `GrappleRig` on Rico, `Hero::grapple` is still "the
  wire holds him"): see **The grappling hook** below.
* Parachute: opens in the air (`Space`, with 3 m of room), glides at 11 m/s (6–15 with
  S/W) and sinks at 5.5 m/s (3–9), turns at 1.3 rad/s; closes on landing or `Space`.
  Grappling while under the canopy pulls him along it (the game's parasailing).
* Swimming: below the surface (y = 200) with no ground to wade on he treads water
  0.35 m under it and paddles at 2.2 m/s (`swim_idle`/`swim_fwd`); no jump, no chute.
* Vehicles: `E` within 4.5 m climbs in (his body is disabled and rides along), `E` again
  steps out beside the car with its velocity.

Headless test `character::tests::rico_walks_jumps_chutes_and_grapples`: lands on a flat
tile, walks 10 m in 2 s, jumps, flies the chute from 80 m at the glide/descent speeds,
is reeled 30 m to an anchor and let go beside it.

## Grapple target classes (`grapple/class.rs`, JC2-037)

`grappling_weight_category` (`#692808c5`, named in `property_names.txt`) is an integer
authored per object in the entity blackboards (`.epe` inside `.ee`) and vehicle dolls
(`.mvdoll`/`.vdoll`); the exe's base object reader (`FUN_008af820`, plus the vehicle-side
`FUN_009144e0`) stores it in the low 7 bits of a byte at `+0x30`. Scan of all 328
containers that carry the key (1,796 nodes):

| Class | 0 | 1 | 2 |
|---|---|---|---|
| `CSimpleRigidObject` | 392 (default, destroyed parts, scenery; 95 fixed) | 408 (389 dynamic: barrels `go001`, tubes `go129`, `go003`, `go040/041`) | 511 (200 fixed: pillars, pile `go164`, `go143`; 243 dynamic) |
| `CSmartObject` (every vehicle doll) | 383 | – | – |
| `CItem` / `CWeapon` / `CWorldsimSpawnableSmartObject` | 9 / 70 / 2 | – | – |
| `CSentryGunLogic` | – | 1 | – |
| `CSAMSite` / `CBigCannon` / `CDamageableObject` / `CVehicleComponent` | – | – | 11 / 1 / 2 / 6 |

Characters never carry it. What is **not** found: any native branch that compares the
category, or a mass against Rico's. The decompiled code has no reader of the field; the
pull/reel split lives in the undecoded `.afsm` machines (`S_GRAPPLED` and
`T_HIT_BY_GRAPPLINGHOOK` are only in `npc_combat_*.afsm`, `LA_ACT_GRAPPLE_PULL` pulls,
`LA_ACT_FIRE_GRAPPLE` / `LA_ACT_FIRE_GRAPPLE_SO` (vehicle smart objects) reel Rico). So
"lighter than Rico gets pulled" is **not** established; the implementation uses

| Target | Result | Basis |
|---|---|---|
| soldier | pulled | `States` (soldier machines own the grappled states) |
| vehicle | anchor | `States` (smart-object grapple, tether) |
| not a dynamic body | anchor | `Physics` |
| category 1 | pulled | `Authored` (data-correlated: nearly all dynamic small props; the mapping is not a native branch) |
| category 2 | anchor | `Authored` (as above) |
| category 0, absent or other | pulled iff `0 < mass ≤ 400 kg` | `Fallback` (explicitly unknown; unusable mass → anchor) |

`Placement::grapple_weight` carries the authored value from the placement files; spawners
attach it as `GrappleWeight(u8)`. `grapple::classify` / `body_class` are the API for
JC2-038 and JC2-039.

## The grappling hook (`grapple.rs`)

`F` (without `Ctrl`) fires; `F` again lets go (or cuts a flight short).

* **Flight.** The hook leaves the gauntlet (the `grapple launcher` mesh's joint,
  `GrappleLauncher`; the wire starts there) at `hook_speed` (`#59e07a13` = 120 m/s, assumed)
  along the line to the target — the lock-on's point, else the camera ray's hit within
  `range` (90 m) — and attaches on arrival, `distance / speed` seconds later (60 m: 0.51 s).
  A shot with nothing in range flies to the range and comes home at twice the speed
  (`Phase::Retract`); Rico is never held. A target on a moving body is followed in flight.
  The wire is a `Cable` (below): slack while the hook flies, taut when it pulls.
* **Lock-on** (`LockOn` resource, `update_lock_on`): the camera's ray; the target is *special*
  when it is a vehicle (`StuntSpots`/`Enterable`), a soldier (`Npc`) or a dynamic prop (green
  reticle through `hud::tint_crosshair`), or a wall with an edge within 2.5 m above the hit
  (`find_ledge`: a ray down from 25 cm into the wall, 2.5 m up). **Magnetism**: when the ray hits
  nothing special, the special body whose middle is nearest the ray within 4° (`MAGNET_DEG`,
  widened by its size: `angle − atan(radius/distance)`) takes the aim, and the hook flies to
  its surface; ledges likewise from four rays 3° around the crosshair. `LockOn::target` /
  `locked()` / `tint()` are what the HUD reads; the dev text line says what is locked.
* **Pull or reel.** Classified by `grapple/class.rs` (see **Grapple target classes** below): a
  soldier and an authored light prop (category 1) are **pulled** to Rico; a vehicle, a static
  body and an authored heavy one (category 2) **reel Rico in**; a body with no usable category
  falls back to the unknown mass rule (up to 400 kg pulled). A prop gets
  `object_impulse / mass` at once and then `pull_force` (500 N) along the wire (speed cap 20
  m/s, let go 2.5 m from him, after 6 s or on `F`). A soldier (`Hooked`; `npc_behaviour`
  skips him) gets `character_impulse / 80 kg`, accelerates at `pull_force_character / 80 kg`
  (850 N: 10.6 m/s², cap 14 m/s), leans back to lie on the ground, is let go 1.8 m from Rico
  and lies 1.4 s before standing up (`Hooked` leaves). The caps, the stops and the soldier's
  mass are assumed.
* **Reeling.** The speed along the wire rises at 45 m/s² to `reel_in_speed` (40 m/s) and brakes at
  50 m/s² into the goal (`v ≤ sqrt(2·50·distance)`), so he stops on it: no overshoot
  (a wall test arrives 0.08 m off and holds within 0.07 m), the distance to the goal never
  grows. Sideways motion dies with a 0.15 s time constant. The goal and what happens there
  depend on the surface (`begin_reel`): a **floor** (normal up): stand 0.12 m above it, the wire
  lets go, he lands; a **wall**: his middle 0.55 m below the hook and 0.4 m off the surface,
  then he **hangs** (`Phase::Hang`, `Hero::hanging`, `Clip::Hang`; a wall so low that his feet
  would touch the ground is a landing); a **ceiling**: 0.75 m below it, hanging; a **vehicle**
  or another moving body: the wire follows it and lets go 2 m from it, `stunts.rs` lands him on
  the stunt point. A reel that gets nowhere near the goal for 0.3 s counts as arrived;
  one longer than 8 s lets go.
* **Hanging.** Pinned by a spring with the step's gravity cancelled; he can shoot (`combat.rs`
  allows fire while `hanging`). Prompts **Jump `SPACE`** / **Drop `LCONTROL`**
  (`PromptContext::hanging`). `Space` at a ledge climbs: up at `sqrt(2·g·rise)` and 2.2 m/s into
  the wall for up to 1.2 s (`climb_assist`; the wall would otherwise kill the sideways speed) —
  he ends on the roof; `Space` at a plain wall leaps off it (4.5 m/s out, 5.5 up; `Space` again
  opens the chute); `Ctrl` (or `F`) drops. `HeroInput::drop` carries the key.
* **Releasing mid-reel** keeps his velocity (no braking, no damping). **`Space` mid-reel**
  opens the chute and the wire keeps pulling; letting go then leaves him gliding at far more
  than the chute's own 11 m/s (25 m/s measured 0.3 s after) because the chute branch fades the
  excess with a 1.2 s time constant and a rising climb with 2.5 s: the grapple-and-chute boost.
* **The cable** (`Cable { a, b, slack }`): 14 short cylinders along a parabola sagging
  `slack · 6 %` of the length; a pale, unlit thin line whose radius grows with the camera
  distance to stay about a pixel across (`wire_width` 0.006 m is the minimum). The hook's wire
  (from the hand to the hook: slack 0.6 → 0 in flight, 0.8 retracting, 0 reeling, 0.02 hanging),
  the helicopter's wire (`HangWire`, slack 0.02) and the tethers (`TetherWire`, white now: slack
  by the length to spare) all use it.
* **Pose states** for the animation (Hang is still the only clip; `hero_animation` is
  unchanged): `Hero::hanging` (reeled in and hanging from the hook), `Hero::grapple` Some and
  not hanging (reeling), `GrappleRig::phase` (`Flight`: the arm out; `Pull`: pulling a soldier
  or prop; `Retract`), `GrappleRig::reel` (speed along the wire) and `GrappleRig::ledge_top`
  (a ledge climb is on offer), `Hooked::lean` on a soldier. A soldier's own clip is left alone.
* Debug: `--grapple-demo fly|hang`.
* Headless tests (`grapple::tests`, `stunts::tests`): the flight takes `distance / speed`
  (60.8 m at 120 m/s: 0.516 s against 0.506 s) and attaches; a shot past 90 m retracts and
  never holds him; a 40 m reel to a wall arrives within 0.2 m, never backs off and holds
  still; releasing mid-reel keeps over 90 % of the speed; the chute opened at 28 m/s of reel
  keeps 25 m/s forward after the release; a ledge is hung from, dropped with `Ctrl` and climbed
  with `Space` onto the roof; `Space` leaps off a wall; a floor lands him; a barrel is pulled
  10 m or more to Rico while he stays put; a soldier is pulled, lies, and stands; the lock-on
  picks the crate and the soldier at 0, 2.5 and 3° and none at 12°; the cable's sag; the
  pendulum under a helicopter that circles for 12 s and brakes stays inside 45° (24° measured)
  and decays below 3° (0.2° after 40 s); the jeep's roof is reached by firing the hook.

## `grpl_hang.ban` faces backwards

Taken at mid-clip and put through the skeleton, the pelvis's left-to-right axis (left
thigh to right thigh) points along +x in the reference pose, `std_idle`, `run_fwd`,
`wk_fwd`, `fall`, `land_chute` — and along −x in `grpl_hang` (and its hips' forward
vector is flipped): the clip is authored with its root turned 180° about the vertical.
Played as is, a body that faces the way it is reeled in (yaw from the direction to the
hook, the same convention as walking) looks back at the camera, and so did the chute pose,
which uses the same clip. `rig::clip_bones` turns the root (bone 0) of
`Clip::faces_backwards()` clips half way round; the test
`rig::tests::every_clip_faces_the_same_way` checks the left-right axis of every clip
against the idle's.

## Other characters: dolls

`exported\cdoll\<name>.ee` holds `<name>.cdoll` (and the part models): `#797211b8` the
base `.lod`, `#f53a7638` the skeleton (`biped.bsk` for Rico and the soldiers,
`civilian_biped.bsk` for villagers), `#205e72aa` body-shape morphs (`Fatness`, `Unisize`:
per-bone scale ranges), and under `#68ddf8de` the part groups — `Heads` (`Head`,
`Head_Camo`), `Hats` (`Hat`, `HatnG`, `Beret`, `BeretnG`) on `pd_gov_base01`; `Skin`, `Hat`,
`Jacket`, `Pants`, `Collar` on `pd_arcticvillage_male1` — each variant a `#a5ae7a6c` `.lod`.
One variant per group is worn (`jc2_formats::character::CharacterDoll::outfit`).

## Soldiers: team spawn sets

A `CSpawnSet` with a `CTeamComponent`: `#f5afca6d` the AI team (`bofcx1.aiteam`, a PCBB
file of senses, actions and goals — `CAction_GuardAOO`, `CWalkPatrolRouteAction`, … — and
an agent name, no model), `#a870db99` the area of operation it guards
(`CAOOBoxGameObject` with `SGuardPoint` children) or `#dfbe4772` the patrol route
(`CPatrolRouteGameObject`, `route = floats[4n]`: x, y, z relative to the object and 0.5),
one `STeamAgentInfo` per soldier (`#78efe1bc` the role `Guard`/`Patrol`, `#1df63231` 100 —
health?). Tile 31,14 has six teams of one, all `bofcx1`, five areas, two routes (seven
and four points). jc2-game (`npc.rs`) rolls each a grunt or an elite of the government
with its weapon and health from `enemytypespawnsettings.bin` and dresses it in the
class's doll, `pd_gov_base01` or `pd_gov_elite` (`spawning.md`; that `bofcx1` teams are
the government's is assumed), skins it to its skeleton,
lets guards stroll between their guard points and patrols walk their routes at 1.4 m/s
with `wk_fwd.ban` (root motion 1.33 m/s), and drops them with their tile.

## Still unknown / later

* The canopy's real model (if the game builds it from code), depth and cloth, the `chute_speed/turn_tb_*` blend poses (interleaved
  clips on the same skeleton), the arm launcher's exact placement; the hook's flight speed is the unnamed #59e07a13 (assumed).
* Animation: decoded (`animation.md`) — the clip choice is hand-written; the game's
  `.afsm` state machines and `.asb` animation sets are not read yet.
* The ~80 unnamed numbers in `player.bin` (speeds, jump, fall damage, stunt timers).
* Pedestrians and the other `cdoll`s, `charactertypes.bin`.

## Stunts

### Data: the stunt positions of a vehicle

Every vehicle doll (`.mvdoll`) carries its **smart objects** (`CSmartObject`): the seats
(`SO_Driver`, `SO_Shotgun`, `SO_Gunner1`) and the places to stand outside. A *stunt
position* is a smart object that has a `CRoofTopStuntController` among its children.
`jc2tool vehicle <game> lave.v014_jeep.ee` lists them (`jc2_formats::vehicle::StuntPosition`):

| Jeep | `SO_Hardtop_RD` | `SO_Hood_RD` |
|---|---|---|
| position in the vehicle frame (engine axes, nose = −z) | (0.005, 1.864, 0.284) | (0.005, 1.264, −1.616) |
| controller `#cceaa327` (area tag) | empty | `CAB` |
| `GroundToRoofTrigger` box | centre (0, 2.60, 0.07), half extents (9.62, 2.58, 11.95) | the same |
| `ParachuteToRoofTrigger` box | centre (0, 13.7, 0), half extents (9.70, 11.54, 12.16) | the same |

The trigger boxes' third key `#2be10cce` is read as **half** extents: the ground box's
centre height equals its half height (it stands on the ground) and the parachute box
spans 2–25 m above the vehicle. The smart object's children (`CStuntAction`s —
`RoofToFrontClingAction`, `CRoofToGroundJumpAction`, `CParachuteFromRoofAction`,
`CEnterDriverSeatFromRoofAction`, `CEnterMGAction`, with the clip states
`S_ROOFTOP_STUNTPOSITION_IDLE`, `S_PERFORMING_STUNT`, the `vhl.defaultstunt.enter/exit`
events) name the game's transitions; their own `world` matrices are leftovers of the
editor (they repeat the vehicle's origin) and only the smart object's `world` is the
spot. `#5bc0764c`/`#742cdd23` (6) are unnamed. The clips behind the idle states are in an
`.afsm` that is not decoded; the nearest plain clips are `std_idle` and `cr_idle`
(crouch), which is how the original's screenshots show him: knees bent, facing the nose.

`grapplingparams.bin` has no tether keys that could be named: the numbers 50, 50, 120,
140, 40, 500, 3, 16, 2 and 0.91 stay unresolved (no candidate in `strings.csv` or any
word combination hashes to them). The events `ply.doubletether.objects` and
`ply.doubletether.vehicles` (Lua) are the game's names for the feature.

### Stunt positions of the other vehicle kinds

The controller class says what the position is (`jc2_formats::vehicle::StuntKind`):

| Kind | Class | Where (vehicle frame, engine axes) |
|---|---|---|
| Roof | `CRoofTopStuntController` | cars: hardtop and hood; boats: `SO_Body_BD`/`SO_Body_RD` on the deck (motorboat v083 (−0.04, 0.81, −3.55), the bow; charter (−0.13, 2.11, 0.23); yacht (−0.04, 1.16, −6.23); military boat (0, 3.13, −0.12)); planes: `SO_Body_RD` on the fuselage (speed plane (−0.12, 1.92, 0.01), fighter (−0.12, 2.86, 0.01), jumbo jet (0.01, 5.81, −6.92)) |
| Belly | `CBellyStuntController` | every helicopter, under the hull: chopper v062 `SO_Body_RD` (0.00, −1.40, −2.69), attack helis v060 (0, 0.73, −2.92) and v061 (0, 0.67, −2.22), v059 (0, 0.44, −1.40), military v009 (0, 0.57, −1.05) |
| Cling | `CThreeNodeClingController` | the chopper and the military helicopter only, at the nose (`SO_Body_CL_FR_4PT_MID`, v062 (0, −0.53, −5.10)), with `CHelperPoint`s for the hands and feet (`HP_Body_CL_FR_LEFT/RIGHT/DWN`) |

Helicopters have no roof position: the original hangs Rico under the skids or the belly
(by one hand, or on the hook's wire) and lets him climb to a "clinging position" at the
cabin (`CClingToRoofAction`, `CEnterDriverSeatFromClingAction`, the
`heli_cling_*`/`hjk_*`/`cockpit_*` clips). `ReelToBellyTrigger` (a box 2.7 m over the
belly point) is where the hook reels him to it. The clip `heli_cling_dwn_idle.ban` turned
out to hang him upside down; the grapple hang (`grpl_hang.ban`) matches the screenshots.

### In jc2-game (`stunts.rs`)

* Every vehicle the fleet builds gets a `StuntSpots` component from its doll (an
  observer on `fleet::Container`): cars, helicopters, boats and planes, driven, crewed
  or parked, traffic included.
* Getting up: `Q` within 6 m of a stunt point (and inside its grapple box when it has
  one), or `F` at the vehicle: the hook's end rides on it (`HookedVehicle`) and when the
  wire has reeled him in he lands on the nearest stunt point.
* Pinned: `physics::stow_passenger` makes his body inert (no collision, no gravity; never
  `RigidBodyDisabled`), and after each physics step (`PhysicsSystems::Last`) his
  `Position` and `Transform` are set to the spot in the vehicle's frame, his velocity to
  the spot's (`v + ω × r`). His own controller is skipped (`Hero::stunt`). On a roof his
  feet are on the point, he faces the nose, idles and crouches (`cr_idle`) above 3 m/s;
  under a helicopter his middle hangs 2 m below the belly point on a short wire from his
  forearm (`HANG_DROP`, `Clip::Hang` through `Hero::stunt_clip`); at the nose he clings
  0.45 m ahead of it, facing the cabin. Guns work everywhere.
* The wire: a flying helicopter hooked from below does not reel him all the way up. Once
  he is within `WIRE_LENGTH` (6 m) of the hook point, at least 1 m below it, with 2 m of
  air under his feet, he hangs on the wire (`OnStunt::wire`, at least 2.5 m), pinned
  on a pendulum under the hook point (`WireSwing`, `swing_step`: gravity, damping of the
  motion relative to the moving hook point at 0.5 /s (assumed), the wire's length held; the
  velocity comes from the constrained move, so the helicopter's turns and climbs drive the
  swing), drawn as a `Cable`. `Space`, `Q`, `F`, `Alt` or `Ctrl` let go (he falls with his own swinging velocity; `Space`
  again opens the chute), `E` climbs to the helicopter's stunt position.
* Off a stunt position: `Space` is a jump with the spot's velocity plus 5.5 m/s up
  (`physics::release_passenger`; `Space` once more opens the chute), `Q` steps off, `E`
  takes the seat — throwing out the driver of a crewed vehicle first (`crew.rs`) — and
  `F` fires the hook. A despawned vehicle or a respawn lets him go where he is.
* The orbit camera ignores the vehicle he is on (its hull pulled the camera into his
  head) and stands 1.2× further back on a roof, 1.7× under a helicopter.
* `--ride [spot|wire]` puts him on the first vehicle the command line asks for, for
  smoke tests (`--heli --crewed --ride wire`: on the wire under a circling chopper).

Headless tests (`stunts::tests`): the jeep's roof (below); the motorboat's bow deck at full
throttle in a turn for 12 s — 104 m at up to 78 km/h, pitching −1.5°…0.3° — with Rico at
most 0.001 m off it, and the jump-off with the boat's speed; the chopper's belly while it
spins up, climbs 86 m and turns 107°: at most 0.001 m off; the hook fired at a hovering
chopper from 15 m below: caught on the wire at 5.8 m, pinned through 4 s of turning
flight, let go, falling, chute open.

### Tethers

`Ctrl`+`F` shoots the tether gun along the camera's ray (as the hook does). The first hit
is kept; the second makes the cable: an Avian `DistanceJoint` between the two points, each
in its body's frame (`limits` 0 .. length). A dynamic body (a car, a crate, a soldier) is
held directly; static ground or buildings get a fixed anchor point entity (`TetherAnchor`).
Every fixed step the length winds in to `min(length, current distance) − 8 m/s·dt`, down to
2.5 m, so the two things are pulled together; a cable stretched 4 m past its length snaps,
none is longer than 200 m, three at most (the fourth drops the oldest), `X` cuts them all.
When a body or the thing a static end was shot on is despawned (a tile unloading, a
vehicle bursting) the cable goes with it at once (`Remove` observer on `RigidBody`, and a
liveness check each step), and takes its anchor point along. Drawn as a `Cable` (thin, white, sagging with the
length to spare). The numbers (winding speed 8 m/s, minimum 2.5 m, snap at 4 m, range 90 m
as the hook's) are assumptions: the data's tether strength could not be found.

Headless tests (`stunts::tests`): the jeep's roof, 50 m of driving and turning with Rico
within 0.2 m of his point, the jump-off velocity, the hook landing and `E` into the seat;
two crates 20 m apart wound to the minimum length; despawned bodies (dynamic, static,
a tile-like wall) leave no joint or anchor; the limit of three, `X` and the snap.

### Rico while down

At zero health, `combat::hero_down` marks Rico `Dead` before his input handlers run.
His visual child topples backwards over 0.5 seconds and stays visible for the existing
four-second respawn delay. The skeleton holds a neutral idle pose; gun/aim layers are
cleared. This is an assumed visual fallback, not a decoded death clip or a ragdoll.
The capsule retains ordinary physics contacts and gravity; the visual rotation does
not change the physics body's rotation or disable its collider.

Movement, jump, parachute, hook, vehicle entry/hijack, stunt mounting, tether fire,
shooting, grenades, weapon selection and reload input are suppressed while down.
Vehicle/stunt deaths release the normal passenger state, drop occupancy, remove
wire swing and hooked-vehicle tracking, and clear hanging, ledge, flight and pull state.
Respawn restores health, removes `Dead`, restores the visual model's ordinary feet
offset and upright pose, then settles the character at the original spawn point.

Headless tests check visible horizontal body pose, untouched physics rotation,
ammo/grenade/input suppression, stationary horizontal position, restored movement
after respawn, grapple/pull cleanup, and the full roof/driver-seat death lifecycle.
`--pose down --pose-view 75 --exit-after 8 --screenshot <png>` exercises the real
zero-health path while holding the respawn timer for screenshot inspection.
Verified 8 October 2026 against plain Steam data: all 308 workspace tests passed
(125 formats, 183 game), matching 308 source test markers, and release clippy had
no warnings (isolation 34). The private eight-second `--pose down` screenshot shows
Rico's model horizontal and visible on the snowy ground. A rectangular red damage
overlay is also visible in that screenshot; this milestone does not alter the HUD's
existing vignette rendering. No original-game death reference was available for a
pose comparison.
