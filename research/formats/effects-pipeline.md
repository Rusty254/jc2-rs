# Effects pipeline: how the native game spawns gameplay effects (board task b0ed9f8e, "fx-found")

Research for the effect spawner (12 October 2026). Probe: `research/tools/effects_pipeline_probe.py` (56 checks,
exit 0). Paired implementation task: filed on the board as "Effects: runtime effect spawner" (section 9).
Read with `particles-next.md`, `particle-consumers-next.md`, `point-light-model.md`,
`parachute-effects-audio-hud.md` (the per-model effect registration path) and `vehicle-effects-selectors-next.md`.
The weapon-fire side (muzzle flash, tracers, impacts per weapon) is task e4083e96 (`weapon-fire-effects.md`);
it builds on this document.

Evidence: the retail `JustCause2.exe` (14,549,656 bytes, sha256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`), its Ghidra export,
`classes.csv`/`calls.csv`, and the retail files `effects/effectsettings.bin` and `effects/*.bin`.
"native" = read from the executable or the retail data and asserted by the probe; "inferred" is said so.
Addresses are retail virtual addresses. Property names were recovered by hashing strings of the executable with
the engine's Jenkins `hashlittle`; an unresolved hash stays `#xxxxxxxx`.

## 0. Answers in one table

| Question | Answer |
|---|---|
| What is an effect id? | The `#6a9dfd76` (`uid`) of each entry of `effectsettings.bin`. Ids are dense 0..467 (468 entries) and are the **index** into a table of 0x48-byte `CEffectDescription` slots in the effect system. |
| Where does the id come from at a call site? | Mostly from data the caller owns (weapon/projectile/vehicle/object records); a few literals (448 parachute leaves, 370 altitude fire). Section 5. |
| The effect system | `CEffectSystem` (RTTI), singleton of 0x68 bytes, global `DAT_01190c48` (created 0x0055c1c0, ctor 0x007a02e0). Every caller tests that pointer first. |
| How is an effect spawned? | `Play(id, matrix*, velocity*, inputs*)` at **0x00579f50** (23 call sites), or `Play` with a surface/interaction entry at **0x006cd100** (one entry, matrix, velocity, inputs, object array, direction, component mask) through 0x006774b0 / **0x0077f940** (14 call sites). Both are fire and forget; no handle returned. **Persistent (controller) effects** use the handle getter 0x0062a7a0 and the controller `Play(interactions, restart)` 0x005c1cc0 (section 5.4). |
| What does an effect consist of? | A parsed `effects/<name>.bin`: settings, up to 11 typed components with numbered variations, an interaction matrix choosing the variation of each component per surface, optional named float inputs. Section 1. |
| Component classes | Eleven (`CECD_*` description / `CECI_*` instance); file `type` 0..10; 8 are used by retail files. Section 1.3. |
| When does a component pick its variation? | Once, at start, from the interaction list (surface id) resolved against the matrix (section 3). Inputs only drive parameters afterwards. |
| Lifetime | Six-state machine in `CEffect::Update` (0x006e6bc0): one-shot effects end when the last component dies plus a tail time; looping effects need an explicit Stop. Section 4. |
| Cap / throttle | Global cap on live effects (255, from the registry file). `max_instances`, `priority` and the throttle mode are parsed but **no reader was found** (section 6, gap 1). |
| Culling / draw | Not in the effect system: components hand their work to managers (particles, lights, decals, sound). Particle manager rules (1024 emitters, density scale) are native (section 6.1). |
| Threads | A critical section (`CEffectSystem+0x64`) guards Play, queue flush and Update. |

## 1. Data

### 1.1 The registry `effects/effectsettings.bin` (native, loader 0x0073d250)

* Root `#ee927e3d`; its `settings` child holds `#6a32433a = 255`, stored in the low 31 bits of
  `CEffectSystem+0xc`: the **cap on simultaneously live effects** (tested in `CreateEffect` 0x00694fc0 against
  the live counter `DAT_011f9c2c`, `ja` fails creation only when the counter is above the cap). Bit 31 of that word
  is set by the constructor (0x80000000) and read by 0x0076fd40; its meaning is open.
* `#4cbb1cc1` is the effect list. Its entry count becomes `CEffectSystem+0x14` and `count * 0x48` bytes are allocated at
  `+0x10`; each entry has `filename` and the id (`#6a9dfd76`). The loader reads every file and parses it into slot `id`
  with 0x007f6070. All ids and files are loaded at start-up; nothing is streamed per Play.
* Retail: 468 entries, ids 0..467 dense, no duplicates. 460 resolve; **8 name `.xml` files that are not in the
  install** (`tramway_glassbreak_big/small`, `cs_km07_windtunnel_01`, `cs_km07_glassbreak_01`, `cs_km07_nukeengine`,
  `cs_km07_nukeloop`, `cs_impactdust_01`, `fx_km07_nuke_rain_01`): their slots are never parsed, so `CreateEffect` fails
  for them (slot id check) and Play returns 0. File names are case-insensitive in the engine VFS (`jc2tool` needs the lower-case
  spelling for some).
* The loader ends by pre-loading two per-model event bindings (`Effects\ParachuteEffectSettings\ParachuteEffects.bin`,
  `Effects\CharacterEffectSettings\RicosEffects.bin`) via 0x005f3cb0. Vehicles use the same model-binding loader 0x0082eaf0
  (see `parachute-effects-audio-hud.md`: an entry with effect id -1 registers nothing).

### 1.2 One effect file (native, parser 0x007f6070)

```
#e62de7fa {                       effect root
  settings { priority; max_instances (#488c291b); tail time (#50be431d); looping;
             #553bd311 (1..3) + #1cdc8b90, or #8bd4af70 }
  interaction_matrix (#84085477) { ... }       section 3
  inputs { impulse = 0 ... }                    optional: named float inputs with a default (108 of 460 files)
  effect_components (#f6a9f638) { <role-name> { type; uid; variations { <name> { uid; ... } } } ... }
}
```

`CEffectDescription` (0x48 bytes, vftable 0x00e31fd8), native:

| Offset | Source | Meaning |
|---|---|---|
| +0x04 | slot | the id (-1 = unused) |
| +0x08 | `priority` (0x43676e89) | retail: 0 in every parsed file |
| +0x0c bits 0..26 | `max_instances` (`#488c291b`) | retail 1:19 2:9 3:3 4:11 5:3 8:1 10:381 16:30 20:2 50:1 |
| +0x0c bits 27..30 | `#553bd311` (1..3), or 1 when `#8bd4af70` > 0 | throttle mode selector (reader not found) |
| +0x0c bit 31 | `looping` | 89 retail files loop; the others are one-shot |
| +0x10 | `#50be431d` (float) | **tail time** (seconds) used by the state machine and Stop (section 4); retail 0.1 .. 30 |
| +0x14 | `#1cdc8b90` (mode 2/3) or `#8bd4af70` (mode 1) | float of the throttle mode (reader not found) |
| +0x18 | components | pointer vector indexed by the component uid |
| +0x28 | `interaction_matrix` | section 3 |
| +0x44 | runtime | seconds since this effect last began: `Update` adds dt for every used slot, `Begin` stores 0 |

Components in retail files, counting components (not variations): Sound 339, VisualParticleSystem 412, Decal 20,
PhysicalForcePulse 37, HapticsRumble 10, LightSource 129, CameraShake 11, SoundPostEffect 138; **types 2, 6 and 10 are
unused**. A component has one variation in 1032 of 1081 cases; the rest have 2..16 (footsteps, impacts, wheels).

Variation fields, by `type`. Names are given where the hash was recovered; counts are variations carrying the field.

| type | Fields (names or `#hash`) |
|---|---|
| 0 Sound | `bank_id`, `sound_id` (the (bank, sound) pair of `soundmap`), `var_id_focus`, `var_id_cursor`, `#9967eecc`, `#525ff155`, `#b97dd06b`, `#ba5392e7`, `#50a66de8`, `#8fb3963d`, `#47a8a80d`, `#769ffd72`, a nested list `#36ceb99f` (11) |
| 1 Particle | `name` (the particle system, e.g. `fx_laser_impact_01`), `duration` (163 variations), `#b0124c0a` (473; byte passed to the particle manager), `#3c9be34c`, `#85ee8f21`, `#d623c711`, `#fc405fe1` (79 each), `#4de5cc5c`, `#7c4a3b08`, `#67867668`, `#3a43956a`, `#d9fad10c`, `#9b356e5f` |
| 3 Decal | `#0338eafb` diffuse, `#9045606f` specular, `#f0f75c7f` normal (texture paths `Decals/*.dds`), `lifetime`, `size` (4 floats), `#7bb996a0`, `#99bfeaf1` (2 floats), `#9b006d73` |
| 4 PhysicalForcePulse | `#41874e97`, `#4402eb79` (e.g. 800 / 1800), `#fb8c4ec3` (vec3) |
| 5 HapticsRumble | `#015128a1`, `#263580f8`, `#3f1d2ee2`, `#be0b5e5f` |
| 7 LightSource | `normal_offset` (vec3, = hash 1b20850d "offset" of `point-light-model.md`), `looping`, key nodes `#331ad791`, `#a775ab55`, `#e03e6d54`, ... (attenuation or radius style; see `point-light-model.md`) |
| 8 CameraShake | `speed`, `scale`, `distance_scale`, `#27750b42`, `#6afb428a`, `#73440d3e` |
| 9 SoundPostEffect | `attack`, `release`, `gain`, `type`, `target`, `looping`, `#50be431d`, `delay`, `dsp_variable_id`, `dsp_variable_value0/1`, `#e8abc55f`, `#0f429f78`, `#c6fdc17a`, `#6e55b646`, ... |

Particle `name` values are particle systems of `global/particles.blz` (`.psmb` members, 2600). **Sixteen names do not resolve**
(probe): the empty name, `bloodsplat`, `firemedium_static`, `fx_bulhit_dirt_medium_01`, `fx_bulhit_flesh_small/medium/large/huge_01`,
`fx_bulhit_metal_big_01`, `fx_flare`, `fx_km07_nukeexplosion_01.psm`, `fx_km07_nukeshockwave_01.psm`, `fx_vehicle_impactcloud_asphalt_01`,
`fx_vehicle_rolling_snow_03`, `fx_vehicle_water_drive_large_01`, `spawntest_01`. A name that ends in `.psm` is stripped before the `.psmb` lookup.
So the shipped game has **no flesh bullet-hit particle** (the flesh variations of the bullet impacts point at missing systems and spawn nothing
visible; sounds and the rest of the effect still run).

### 1.3 Component classes (native)

The type switch is at 0x007f6490 in 0x007f6070 (jump table 0x007f6598); each case calls the listed constructor, which writes the
`CECD_*` vftable (all checked).

| `type` | Description (vftable, ctor) | Instance `CECI_*` (vftable) | Start | Stop | IsAlive (slot 3) | Update (slot 10) |
|---|---|---|---|---|---|---|
| 0 | `CECD_Sound` (0xeba92c, 0x007c0070) | `CECI_Sound` (0xeba61c) | 0x007bed40 | 0x00852220 | state at +0x28 != 0 (0x00822690) | 0x0072b040 (sound sequence timeline) |
| 1 | `CECD_VisualParticleSystem` (0xeba954, 0x005f54b0) | (0xeba4c8) | 0x0063d2b0 | 0x007c4460 | emitter present and `(word[emitter+0x2a2] & 6) == 0` | none (manager updates) |
| 2 | `CECD_VisualPostEffect` (0xeba3c4, 0x006b0fc0) | no instance class | - | - | - | - |
| 3 | `CECD_VisualDecal` (0xebaa1c, 0x00740e70) | (0xeba490) | 0x00573150 | no-op | **always false** (0x00a414e0) | none |
| 4 | `CECD_PhysicalForcePulse` (0xeba974, 0x00678de0) | (0xeba458) | 0x005fc5f0 | no-op | always false | none |
| 5 | `CECD_HapticsRumble` (0xeb9f30, 0x008469c0) | (0xeba420) | 0x007992e0 | no-op | always false | none |
| 6 | `CECD_InGameCutScene` (0xeb9a04, inline) | no instance class | - | - | - | - |
| 7 | `CECD_LightSource` (0xeba064, 0x007e9350) | (0xeba988) | 0x00724c50 | 0x007cced0 (sets the managed light's flag `[light+0x1c] = 1`) | managed-light handle valid (0x007f7130) | none (light manager) |
| 8 | `CECD_CameraShake` (0xeb9f1c, 0x006b67e0) | (0xeba3e8) | 0x0062b300 | no-op | always false | none |
| 9 | `CECD_SoundPostEffect` (0xeba940, 0x00505dc0) | (0xeba9e4) | 0x0078c020 | 0x0075aff0 | handle `[+0x24]` not 0 and not -1 (0x005114c0) | 0x00619ca0 |
| 10 | `CECD_AreaDamage` (0xeb9f08, 0x00599960) | (0xeba72c) | 0x0050d700 | no-op | 0x00522630 | none |

`PhysicalForcePulse` is the type formerly guessed as "type 4: physical-looking fields" in `particles-next.md`.
**`AreaDamage` exists natively but no retail effect uses it**: damage is not driven from effect files; keep damage out of the effect module.

Instance vtable (base `CEffectComponentInstance`, vftable 0x00eb99cc, 13 slots; roles read from the callers): slot 0 `Start() -> bool`,
slot 1 `Stop(immediate)`, slot 3 `IsAlive() -> bool`, slot 4 `SetInput(key, value)`, slot 7 `SetMatrix(m)` (called by 0x00671310 as `[vtable+0x1c]`),
slot 8 `SetVelocity(v)` (0x00735760, `+0x20`), slot 10 `Update(dt)` (`+0x28`, only called when IsAlive), slot 11 `SetEnabled(flag)` (`this+0xc = flag`;
`CECI_Sound` overrides it with 0x00821480), slot 12 destructor. Slots 2, 5, 6, 9 are assert stubs / no-ops in most classes (roles open).

The one-shot classes (decal, force pulse, haptics, camera shake) do their work inside `Start` and report **not alive**; the effect's liveness then
comes only from sounds, lights and particles. This is why many effects are "alive" for as long as their particle emitter or sound is.

## 2. Native runtime model

```
CEffectSystem (0x68 bytes, singleton DAT_01190c48; vftable 0x00eba01c)
  +0x0c  low 31 bits live-effect cap (255), bit 31 flag
  +0x10  CEffectDescription[count] (0x48 each)       +0x14 count
  +0x18..  strong list: fire-and-forget effects (vector<shared_ptr<CEffect>>), removed when their state reaches 0
  +0x28..  weak list: controller effects (vector<weak_ptr<CEffect>>), ticked while a handle keeps them alive
  +0x38..  incoming weak entries (filled by the handle getter, merged into the weak list at the start of Update)
  +0x48  start queue: vector of {shared_ptr<CEffect>, vector<{kind,uid}>}  (0x18 bytes each)
  +0x64  critical section
CEffect (200 bytes, one-slot vtables CEffect : IEffectController : CEffectImplementation)
  "this" of its methods is CEffect+4 (offsets below are for that sub-object):
  +0x0c state 0..5   +0x08 elapsed (s)   +0x78 CEffectDescription*   +0x14 map<hash,float> inputs
  +0x2c 4x4 matrix (row-major, translation in row 3)   +0x6c velocity (3 floats)
  +0x7c.. vector<shared_ptr<CEffectComponentInstance>> (created lazily by Begin)
  +0x90.. object pointer array   +0xa0..0xa8 direction vector   +0xa8 component mask   +0xb0.. interaction list
```

### 2.1 Spawn: `Play` 0x00579f50 (native)

`Play(effect_id, matrix*, velocity*, inputs*) -> bool` (lock held throughout):

1. `CreateEffect` 0x00694fc0: fails (returns 0, silently) unless `id < count`, `slot.id == id`, the live counter is not above the cap and a
   200-byte object can be allocated. **A new `CEffect` is built from the shared description on every Play**; nothing is pooled per id.
2. matrix: copy 64 bytes to the effect, forward to components that already exist (none yet).
3. velocity: 3 floats; inputs: for each `(key, value)` of the caller's `map<uint,float>`: store in the effect's input map and forward to components.
4. `0x00660a80`: push `{effect, empty interaction list}` on the start queue. Nothing else runs.

The matrix is a full 4x4 (rotation, scale and translation pass through unchanged to components that read it). The effect has no attach
point: owners that need a moving effect keep a handle (0x0062a7a0) and call SetMatrix (0x00671310) every frame (13 callers).

`Play` with interactions, 0x006cd100 (reached only through 0x006774b0, called only by 0x0077f940, which has 14 call sites):
`(id, interaction_list*, matrix*, velocity*, inputs*, objects*, n_objects, direction*, component_mask)`. In addition to the above it stores
`component_mask` (a bit per component `type`: a component runs only if `(mask & (1 << type)) == 0`, native table 0x0064f730), `direction*` (3 floats; the callers pass the
matrix's second row, inferred to be the surface normal), copies the object array (pointers whose flag word at `+0xc` must not have bit 3 set, otherwise the whole Play is
skipped), and if the effect now has objects or a non-empty interaction list it calls `Begin` immediately instead of leaving the state at 4.

### 2.2 Frame: flush, then update (native)

`0x00781d60` (and `0x00748ab0`) call `0x00836840` (flush) then `0x005a34f0(dt)` (Update) when the singleton exists.

* **Flush**: for each queued effect not already in state 1, copy its interaction list into the effect; if the effect has an object array or an interaction list, `Begin` runs now,
  otherwise the state becomes 4 (live counter +1 when it was 0); the effect is pushed on the active list; the queue is cleared.
* **Update**: add dt to `desc+0x44` of every used slot; merge the incoming weak entries (0x007fdea0); then (1) the weak list: each `weak_ptr` is locked (increment of the use count only if non-zero); if
  the effect is still held by a handle it gets `CEffect::Update(dt)` whatever its state (including 0), otherwise the entry is swap-removed; (2) the strong list: state 0 -> swap-remove (which frees the effect),
  else `CEffect::Update(dt)`. So a fire-and-forget effect is freed the frame its state reaches 0, a controller effect lives as long as its handle and can be restarted from state 0.

### 2.3 Stop (native, 0x00569d00)

`Stop(immediate)`: graceful (0) and state 1 -> state 3 and `Stop(0)` on every component when `elapsed >= tail`, else state 2 (tail); other states unchanged. Immediate (non-zero):
`Stop(1)` on every component, state 0, live counter -1. `Stop(flag)` of a component: particle -> emitter stop (graceful = stop emitting and let the particles die; immediate = kill);
light -> sets the managed light's fade flag; sound -> stop / keyed stop of its sequence.

## 3. The interaction matrix (native)

`interaction_matrix` (0x84085477) is parsed by 0x00746c00 (recursive) and has up to three children:

| Key | Meaning | Retail |
|---|---|---|
| `#a28df775` | default matrix: map of **role key -> `"component_uid:variation_uid"`**; `"-1"` = no instance | all 460 files |
| `#55ba2d5a` | second matrix used when the caller passes interaction entry kind 0, uid 1 instead of a surface (the `flag` argument of 0x005f6660 and of the missile/character callers, section 5.2). For the bullet impacts it selects the **water** variations (particle variation 3 is `fx_bulhit_water_*`, sound `0:3`), confirmed by the weapon task's water/ground pair (water = (0, 1), ground = (1, surface id)) | 35 files (bullet impacts, melee, footsteps, wheels) |
| `material_interactions` (0xea0e1560) | children named by surface, each with `#6a9dfd76` = **surface id** and a nested `#a28df775` matrix that overrides the default for that surface | 50 names / 35 ids |

The role key (`Sound`, `#54236b22`, `#570310c8`, `#bbc51e2b` = `Decal`, `#584f80d9`, `SoundPostEffect`, ...) is a label per component class: the native code only uses the numbers
(the keyed map is `map<u8 component_uid, u8 variation_uid>`). The probe checked that all 2169 `"c:v"` strings resolve to an existing component and variation uid
except one retail inconsistency (`hook_detach`, `#55ba2d5a`, particle variation 3 of 1).

Surface ids (names recovered from the file; some ids have several aliases whose hash is unnamed): 0 Sand, 1 Soil, 2 Snow, 3 Gravel, 4 Stone, 5 Ice, 6 Wood, 7 Water, 8 Metal, 9 Glass,
10 Paper, 11 Plastic, 12 Rubber, 13 Textile, 14 Flesh, 15 SteamMetal, 16 Concrete (also Stone_Concrete), 17 Stone_Rock, 18 Wood_Solid, 19 Wood_Hollow, 20 Metal_Solid, 21 Metal_Car,
22 Metal_CatWalk, 23 Metal_Fence, 24 Metal_Steam, 25 Metal_Oil, 26 Glass_Security, 27 Glass_Holy, 28 Electrical, 29 Vegetation, 30 Asphalt, 31 Undefined, 32 Arctic Dirt Road,
33 Flower_Field, 34 Grass (also Grass_Field). (`Big`/`Medium` appear as keys in other contexts.) Whether the surface id of the engine's collision/material system is the same number space is
confirmed by the vehicle wheel files that list exactly these names; the producer of the id at a call site is per caller (section 5.2).

**Resolve** (0x00671bf0, called by Begin's builder 0x0078bd40): start with the default matrix; for each interaction entry `{kind, uid}`: kind 0 with uid != 0 (the water flag) applies the `#55ba2d5a` map and stops;
kind 1 applies `material_interactions[uid]` (recursively) on top of the map so far. The result is a `map<component_uid, variation_uid>`.
**Build** (0x006048b0): for each pair whose component exists, `description.component[uid].CreateInstance(variation_uid)` (slot 1 of the CECD) and append it to the effect.
A component with no entry in the resolved map is not instantiated. Component creation therefore happens at `Begin`, the first frame after the flush.

## 4. State machine (native, `CEffect::Update` 0x006e6bc0, `SetState` 0x00583100)

`SetState(n)`: live counter +1 when leaving 0 and -1 on entering 0; action by new state: 1 elapsed = 0, 3 `Stop(0)` on all components, 5 `Begin`.

| State | Name (inferred) | Behaviour per `Update(dt)` |
|---|---|---|
| 4 | start pending | `Begin` (nothing else this frame) |
| 1 | running | every component with IsAlive true gets `Update(dt)`; `elapsed += dt`; if **no component was alive** and bit 31 (looping) is clear -> state 2 |
| 2 | tail | `elapsed += dt`; when `elapsed >= desc.tail` -> state 3 (`Stop(0)` on every component) |
| 3 | stopping | when no component is alive -> state 0 |
| 0 | dead | removed from the lists by `CEffectSystem::Update` |

`Begin` (0x0058f2b0): skipped when the state is 2 (it jumps to the restart code: a tail-state effect can be restarted); otherwise builds the components if the vector is empty (section 3), then for
each component `SetEnabled((mask & (1 << type)) == 0)` followed by `Start()`; counts the components whose `Start` returned true. Zero started -> `SetState(0)` (the effect dies at once);
else `desc+0x44 = 0` and `SetState(1)`.

Because the one-shot classes are never alive, an impact effect with a decal, a force pulse and a particle emitter lives exactly as long as its emitter or sound is alive plus the tail time.

## 5. Who spawns effects

### 5.1 The 23 direct call sites of `Play` 0x00579f50 (native addresses; "context" is inferred from the surrounding code and event names)

| Site (function) | Context | Effect id source |
|---|---|---|
| 0x0050541f (CLandVehicle slot 4, CCar, CMotorBike) | a vehicle part flag changes (inferred) | `object+0xca0`, -1 skips |
| 0x005089e3 (0x00507db0) | a distance threshold `[[object+0x3c]+0xec]` is exceeded and the state byte `0x18` becomes 1 (inferred: one-shot proximity trigger) | `[object+0x3c]+0xc` |
| 0x0052e758 (0x0052e6b0) | looping effect with one input, key 0 = `200 - altitude` (0 when below), only above a threshold | short at `object+0x74c` |
| 0x005ae59b (CAmmunition/CBulletTrail/CBullet slot 9) | **bullet end of life** (native class membership) | `bullet+0xbc` (inferred: copied from the weapon/projectile record) |
| 0x005cd57f (0x005cd200) | object state change | `[object+0x3c]+0xc` |
| 0x005d1c41 (vehicle classes slot 6, 0x005d1b30) | vehicle event with a position only (identity orientation) | `object+0x88` |
| 0x005d222a (CCharacter slot 9, 0x005d1e70) | character event, velocity scaled by 10 | local table (open) |
| 0x00646a28 (0x006466d0) | object destroyed (`gsy.object_destroyed`) | `object+0x5b8`, -1 skips |
| 0x00662e15 (0x00662c20) | parachute leaves (`parachute-effects-audio-hud.md`) | literal 448 `fx_char_parachute_leafs_01` |
| 0x006688ab (0x00668730) | batch: a list of 0x44-byte records, id = first short, matrix at +4 | record data |
| 0x0067c2f1 (CSentryGunStateCooldown slot 0) | sentry gun state change | `object+0x2b4`, -1 skips |
| 0x00686e6c (CDamageableObject slot 24) | damageable object destroyed; passes an input map | `object+0x19c` |
| 0x0068e413 (0x0068e370) | height-attenuated effect, input key 0 = `200 - height` | literal 370 `char_water_splash` |
| 0x006da682 (CExplosives / CTriggeredExplosive slot 24, 0x006d9fc0) | **explosion** at height >= 200 m (passes an input map); the other branch uses the interaction wrapper | element 0 of the explosive's table |
| 0x006fb351, 0x006fb507 (0x006fb320) | player event (two effects) | globals `DAT_011fc4a8`, `DAT_011fc4ac` (unset in the file image) |
| 0x007031f5 (0x00703100) | threshold effect: a value above `entry+0x8c` | short at `entry+0x90` |
| 0x007064e7 (CTriggeredExplosive slot 7) | arm / trigger effect | list element 2 of the explosive's data |
| 0x0073f389 (CVehicle slot 12, 0x0073efe0) | **vehicle collision** (only when the impulse argument exceeds `[0x01072644]` = 2000.0): picks by the velocity length | `[0x01072630]` = 425 `vehicle_collision_light` below 15.0, `[0x01072634]` = 449 `vehicle_collision_medium` below 25.0 (`[0x01072638]`, `[0x01072640]`), else `[0x0107263c]` = 450 `vehicle_collision_heavy`; these three hold only camera shake and haptics and play only when a seat holds the player, see `vehicle-damage-effects.md` 2.1 |
| 0x00799b80 (0x00799aa0) | per-model slot start (`ParachuteEffects`, `RicosEffects`, vehicle `.epe`) | the slot's registered id |
| 0x0079ed35 (0x0079eb20) | object destroyed | `object+0x490` |
| 0x007b6fe8 (0x007b6f70) | scorpion killed (`gsy.scorpion.killed`) | `[object+0x3c]+0x10` |
| 0x007c8f10 (0x007c8790) | slot-start variant | slot table |

### 5.2 The 14 call sites of the interaction wrapper 0x0077f940 (surface-aware effects)

At 0x0054b88a / 0x0054b9c1 (0x0054a670), 0x00556a15 (CMissile slot 17: missile detonation), 0x00577640 (0x005775a0), 0x0057dd5e (0x0057dbb0), 0x005f6800 (0x005f6660, a thin wrapper that
builds the entry from `flag, surface` and orients by the hit normal), 0x00658faf (0x00658e10), 0x006da543 and 0x006da65d (the explosive 0x006d9fc0), 0x00736eba (CCharacter slot 2), 0x007c8ef4
(0x007c8790), 0x007d5f9d (0x007d5ab0), 0x008408f9 / 0x008409d1 (0x0083fe20). The missile call shows the shape: `effect id = missile data +0xc0`, interaction entry
`{kind = flag ? 0 : 1, uid = flag ? 1 : surface_id}`, matrix = the hit transform, velocity none, inputs none, object array = the hit objects, direction = matrix row 1.
So **bullet, missile and explosion impacts are surface-aware, vehicle and parachute effects are not**.

### 5.3 Persistent effects

Looping effects held by an object (engine smoke, fires, the grapple) and the per-weapon muzzle effect (task e4083e96: `0x006fae80` creates the controller once per weapon, then restarts it per shot) use
`0x0062a7a0(id)` to get a `shared_ptr` handle (the effect is created and put on the **weak** list; its state is 0 until the first Play), keep it in a field (`+0xec`, `+0x120`, `+0x104`, `+0x4b8`, `+0x8a0` in
0x00549660, 0x006b4790, 0x007ce450, 0x005137e0, 0x00639930; `weapon+0xe10`), call SetMatrix every frame and end it with Stop or by dropping the handle. Their spawn rules belong with the owner's task.

### 5.4 Controller operations (native)

`Play(interactions, restart)` 0x005c1cc0 (called through 0x0072beb0 for the muzzle effects): if the state is 1 (running): without `restart` it returns 0 and does nothing; with `restart` it copies the interaction list
(0x0061c730), sets the state to 5 (live counter +1 if it was 0) and runs `Begin` at once, i.e. calls `Start()` again on every existing component and returns to state 1 with the elapsed time cleared. Otherwise
(state 0, 2, 3, 4): copies the interactions; if the effect has no object array it becomes state 4 (Begin on the next Update) and returns 1, else `Begin` runs now. Components decide what a second `Start` means: the
particle component spawns a new emitter only if it has none or the old one is finished (`(word[emitter+0x2a2] & 6) != 0`, in which case it kills the old one first, `Stop(1)`); otherwise the live emitter simply keeps running.
`Stop(immediate)` is 0x00569d00 (section 2.3). Components receive SetMatrix / SetVelocity / SetInput from the controller exactly as from Play (0x00671310, 0x00735760, 0x006f67d0).

## 6. Components in a port

### 6.1 Particle manager rules (native, 0x00a6cb40)

`ParticleManager::Spawn(system, matrix, ...)` returns 0 (nothing spawned) unless: live emitter count `[mgr+0x28] < 0x400` (1024 emitters), the system data is non-null, spawning is enabled (bit 2 of `[mgr+0x40]`
clear) and the system's maximum particle count (word at `+0x76`) is above 0. It then applies a **density scale** `s = max(0.25, 1 - (live + 1) / 1024)` (0x00a6b120): the system's particle cap becomes `ceil(cap * s)` when
smaller and the emission interval (`+0x154`) is divided by `s`. The component's flags byte (`#b0124c0a`) and `duration` go to the emitter start (0x00a6d290).

### 6.2 Light, sound, and the one-shots

* Light: `Start` creates a managed light from the keyed nodes (`point-light-model.md`, 0x00724c50) and stays alive while the light exists; `Stop` sets its fade flag. The light manager expires non-looping lights after the last key.
* Sound: the component is a small sequencer over `(bank_id, sound_id)` items with a timeline (`Update` 0x0072b040), alive while its state field is non-zero; its enable flag is checked before `Start` (0x007bed40
  reads `[this+0xc]`). Variation fields `var_id_focus` / `var_id_cursor` are the FMOD event parameter indices (see `vehicle-effects-selectors-next.md` for the same names). Details are a gap.
* Decal, force pulse, haptics, camera shake: act in `Start`, never alive afterwards.

## 7. Port architecture (what the implementation task builds)

```
jc2-formats::effect
    EffectRegistry::load(fs)          id -> {name, file}; ids dense 0..467; cap from settings
    EffectDescription::parse(bytes)   settings {priority, max_instances, tail, looping, mode, mode_float},
                                      components[uid] {kind: u8, name, variations: BTreeMap<uid, raw PropertyNode>},
                                      matrix {default, flagged (#55ba2d5a), materials: BTreeMap<surface, Matrix>},
                                      inputs {name_hash -> default}
                                      unknown keys stay raw; unresolved particle names are kept and reported.
jc2-game::effects (new module, Bevy plugin)
    EffectSystem (Resource): registry + parsed descriptions (lazy parse per id, same results as eager),
        live counter, cap, active list, start queue, per-description `since_start` timers.
        play(PlayRequest { id, transform: Mat4, velocity: Vec3, inputs: &[(u32, f32)], interactions: &[Interaction],
                           direction: Vec3, component_mask: u16, objects: &[Entity] }) -> Option<EffectHandle>
        stop(handle, immediate), set_transform(handle, Mat4), set_input(handle, key, f32)
    systems (ordered): flush queue -> update(dt)  (the sections 2.2 and 4 state machine, bit for bit)
    trait EffectComponent { start() -> bool; stop(immediate); is_alive() -> bool; update(dt);
                            set_transform(Mat4); set_velocity(Vec3); set_input(u32, f32); set_enabled(bool) }
    adapters registered per kind; unimplemented kinds register a stub that follows the native one-shot contract
    (start() = true for decal/pulse/haptics/shake so the effect still ends, is_alive() = false).
```

Rules a port must keep: every Play builds a fresh effect and respects the live cap; the queue is flushed before Update; components are created at Begin from the resolved matrix; a variation is chosen once;
the category mask disables by component type; one-shot effects end `tail` after the last live component; Stop semantics of section 2.3; the 1024-emitter / density-scale rules at the particle adapter. Content stays
out of the module: callers pass an id and read the registry.

Decisions that need a call from the orchestrator: none; the `max_instances` / throttle behaviour is intentionally absent until gap 1 is closed (native reader not found).

## 8. Open items (Gaps)

Filed as `ready` research tasks (tags `research`, `parity-gap`, `effects`): gap 1 `be538832`, gaps 2 and 3 `c82f3b20`, gap 4 `f7426df4`, gap 5 `76cadbea`, gaps 6 to 8 `aedb2e5c`. Gaps 9 and 10 stay listed here.

1. **Throttle / instance cap consumer** (blocks exact burst behaviour): `max_instances`, `priority`, the throttle mode (`desc+0xc` bits 27..30) and its float (`desc+0x14`) are parsed but a search of the executable
   found no reader: Play, CreateEffect, flush, Begin and Update do not read them, and `desc+0x44` (time since last begin) is written by Update and Begin only. They may be dead data, or read through the slot array by a path not found.
2. **Sound component** (0x007bed40 / 0x0072b040 / 0x00852220): item sequencing, `var_id_*` and the unnamed fields (`#9967eecc`, `#525ff155`, `#b97dd06b`, ...) per FMOD parameter. Blocks audio parity of all gameplay effects.
3. **SoundPostEffect** (type 9, 138 components): attack/release/gain/target/DSP variable semantics (0x0078c020, 0x00619ca0).
4. **Decal component** (type 3, 20 components; `CDecalManager`, 0x00573150): projection, size use, lifetime, fade, normal source, object attachment.
5. **CameraShake** (0x0062b300) and **PhysicalForcePulse** (0x005fc5f0) and **Haptics**: parameter meaning and gameplay effect (ragdoll / vehicle impulses from effects).
6. The meaning of the object array and the direction vector passed by 0x0077f940 callers (section 2.1), and of the `component_mask` values callers actually pass.
7. Particle component start parameters (`#b0124c0a`, `duration`, the `#3c9be34c` group) as consumed by 0x00a6d290.
8. Instance vtable slots 2, 5, 6, 9 and the unresolved input keys (`impulse`, `speed`, `damage`, ...): which component reads which input.
9. Light component details are in `point-light-model.md`; the intersection with the managed-light limits (64 lights, 90,000 m2 cull) is not an effect-system rule.
10. CEffectSystem `+0xc` bit 31 (read by 0x0076fd40) and the globals `DAT_01072630..40` (vehicle impact id table and speed thresholds): values not yet read.

## 9. Paired implementation task

Filed on the board as `e268da82` **"Effects: runtime effect spawner (EffectSystem, description parser, lifecycle, component adapters)"**, status `ready`, tags `effects`, priority high. Owned files: new `crates/jc2-formats/src/effect.rs`, new
`crates/jc2-game/src/effects/` (module), and the one-line plugin registration in `crates/jc2-game/src/main.rs`. Acceptance: all 460 retail files parse; the model tests of the probe (state machine, mask, matrix resolve)
pass as Rust unit tests; a smoke fixture plays an explosion effect and a looping effect through the real lifecycle (light and particle adapters on the existing runtime, others as native-contract stubs); no per-effect content in code.

## 10. Implementation notes (task e268da82, 12 October 2026)

Built: `jc2_formats::effect` (registry, description parser, interaction resolve; all 460 retail files parse, the
type counts, 2169 pairs, 35 flagged matrices and the single `hook_detach` inconsistency are asserted by
`retail_registry_and_files`) and `jc2-game/src/effects/` (`EffectSystem`, the CEffect state machine of section 4,
controllers of section 5.4, light and particle adapters, counted stubs for the rest, opt-in `JC2_EFFECT_FIXTURE=1`).
API summary is in the module header of `effects/mod.rs`.

Findings while building it (native behaviour, all from the retail files):
* The tail time is compared with `elapsed`, which counts from `Begin` (reset only on entering state 1), not from the
  start of the tail state. A component that dies earlier than `tail` keeps the effect alive until `elapsed >= tail`.
* 18 retail effects (for example `grenade_fire`, `km03_fire`, `cut_barbeque`, `fx_ballonengine_01`) have `looping = 0`
  but a light track with `looping = 1`: the light stays alive, so the effect only ends through Stop. They are meant
  for owners that keep a handle.

Inferred (not native-proven): resolve order of the flagged map over the default (later entry wins per role key, `"-1"`
removes the role); new components receive matrix, velocity and inputs before `Start`; the live counter is released
when a controller's last handle is dropped while the effect is not in state 0; a light's Stop removes it at once (the
fade curve is open); a particle `Start` returns false when nothing spawned.

Gaps (new): the particle adapter has no renderer (the only particle runtime is the single-leaf viewer
`particle_fixture`), so emitters are headless records with a stand-in lifetime (`duration`, else 0); the sound
adapter is a counting stub; decal, force pulse, haptics, camera shake and sound post effects are counting stubs.

Update (task dc4653ae, 12 October 2026): `effects/sound.rs` replaces the sound and sound post effect stubs. A Sound
component starts its variation's (bank_id, sound_id) event through the project audio (the spawner's matrix has already
chosen the surface or water variation); alive while its audio instance is (inferred), looping effects hold the event
until Stop, others play it once (inferred). SoundPostEffect fields are parsed (attack, release, gain, type, target,
looping, delay, dsp_variable_*) and queued but no DSP is applied: target and the gain's meaning are undecoded
(0x0078c020, 0x00619ca0; gap task c82f3b20). The unnamed Sound fields, var_id_focus / var_id_cursor and named inputs
are unread. Retail check: 377 post effect = attack 0.01, release 0.5, gain 0.35.

## 11. Particle renderer (task 4e66a75d, 12 October 2026)

Built in `crates/jc2-game/src/effects/`: `particle.rs` (manager, admission, chains, component), `particle_sim.rs`
(emitter simulation), `particle_render.rs` (billboard meshes). `jc2_formats::particle` gained `children` and
`render_flags()`.

Native-proven in use: admission and density scale of doc 6.1 (the scale now also divides the emission rate and
reduces the cap), the JC2-080 job order and modifier execution (survivors then births, windows on the updated
normalized age, signed opacity, force then damping, animation frames, nearest-even cell), local space 2 following the
current source matrix, space 1 adding the source velocity, radial birth bounds and speed `base +- variation`.

Findings from the retail leaves: 2561 of 2600 parse; 39 do not (36 with a non-positive lifetime, 3 without a texture
string) and are refused as unusable. Ten string slots (`fx_exp_barrel_01` uses all ten) name follow-on systems. Render
flag `#71feec86` (bit13) is set on exactly the 129 leaves whose texture is a normal map (distortion); `#27230869`
(bit7) is set on every flare, muzzle flash, spark, lightning and gradient leaf.

Inferred (labelled in code): every follow-on slot starts together with the head as its own emitter (depth 3); the
emission window is the variation `duration`, else `system_lifetime`, else one particle lifetime for a one-shot effect
(unbounded for a looping effect); bit7 draws additive; distortion leaves simulate but are not drawn; the birth
direction is +Y and the birth offset a random direction (the angular fields and shapes 0..4 are unresolved); private
RNG; the quad is a full atlas cell, `size` an edge length, unlit alpha blend, `DistanceFog` from the existing haze.

Fixture: `JC2_EFFECT_FIXTURE=explosion_metalchamber_large,fire_medium JC2_EFFECT_FIXTURE_DELAY=8
JC2_EFFECT_FIXTURE_HOLD=30 JC2_EFFECT_FIXTURE_DIST=14` with `--world 31,14 --no-hero --exit-after 14`;
`JC2_EFFECT_NO_PARTICLE_DRAW=1` gives the before capture. Captures: `_extract/4e66a75d/`.

Gaps: (1) chain semantics of the ten slots (head chain, spawn on death, trail) and their timing; (2) the emitter
start parameters (`#b0124c0a`, `duration`, `#3c9be34c` group, doc gap 7) that fix the emission window; (3) the shape
0..4 consumers and angular birth fields (explosion plumes are +Y columns here); (4) the native size unit and fitted
per-cell quad corners; (5) the exact Particle PS arithmetic (the `*4` saturate, alpha 1.18, dual-source blend) and the
additive/blend flag decode (bit6, bit7); (6) the distortion pass; (7) particle lighting and soft-particle depth
attenuation; (8) the 39 unparsable leaves; (9) the `lifetime_variation` field (`#7535093b`) use.

Update (task dc95ceb0, `particle-units-next.md`): gaps (1), (3), (4) and (9) are answered with native evidence. `size` is a half-extent
(quad edge `2*size`), the fitted corners are an 8-gon whose offsets are `2*uv-1` (full cell square is equivalent apart from trimmed
transparent texels), birth direction is row 2 of `Xrot(pitch)*Yrot(yaw)` (up is pitch `-pi/2`), shapes 0/3/4/5 have the formulas of its section 3,
the follow-on slots are `linked_system_N` (unbounded, all started with the parent), and lifetime is `base +- variation`. The inferred lines above that
say "birth direction +Y", "full atlas cell, size an edge length" and "every follow-on slot ... depth 3" are superseded.

## 12. Update from the smoke research (task 4e32e9af, `smoke-effects.md`, probe `smoke_effects_probe.py`)

Closes or corrects, with native evidence:
* Section 11 gap 1 (chain semantics): the slots are `linked_system_N` (N unbounded, up to 33 in retail); they are **parallel emitters started with the head** (0x00a6d290), sharing its matrix and duration, stopped when the head stops; a separate
  per-particle **trail system** (`#1de3fca7`, 273 leaves) spawns one follow-on emitter per particle. The inferred "every slot starts together with the head" is now proven; "depth 3" is not native (trees go 8 levels deep).
* Section 11 gap 2 and gap 7 of section 8 (start parameters): `#b0124c0a` is a flags byte (default **1**) whose bit 0 allows the emitter to release itself; `duration` overrides the leaf `system_lifetime` of the emitter and its children;
  a lifetime of 0 means unbounded; the other record fields are input bindings (smoke-effects.md sections 1 and 8).
* Section 2.3 (Stop): the particle component's Stop(0) and Stop(1) both end in `0x00a6af80(1)` (stop requested): births stop (at once for an unbounded emitter, at the end of the window for a bounded one), **live particles are never killed**;
  "immediate = kill" was wrong for particles.
* Section 6.1 additions: births are also gated by the global live particle count (< 16384) and by the camera distance (leaf `#d1e88637`, default 800 m); capacity is further limited by `ceil(life + 0.25 var) / spacing`.
* Input bindings of particle variations (the unnamed variation fields) and the CParticleObject world emitter: smoke-effects.md sections 7 and 8.

## Reproduce

```
python research/tools/effects_pipeline_probe.py --game "<game folder>" --tool <jc2tool.exe>
```

## 12. Decal, force pulse, camera shake and haptics adapters (task f986e6ce, 12 October 2026)

Built in `crates/jc2-game/src/effects/`: `decal.rs` (kind 3) and `pulse.rs` (kinds 4, 8 and 5). All four act in `Start`
and are never alive (native, section 1.3). Components only queue ops (`DecalQueue`, `PulseQueue`); Bevy systems
(`decal::apply_decal_ops`, `pulse::apply_pulse_ops`) consume them. Retail checks are tests in `effects/tests.rs`
(95 decal variations, effects 35, 207, 212, 377).

**Native-proven in use.** Force pulse fields `#41874e97` = A, `#4402eb79` = C, `#fb8c4ec3` = half extents; the pulse's camera
coupling (camera closer than 3 x the largest extent: shake intensity `(0.5 * 0.5 / max(1, 0.1 d)) * 0.2`, speed 1.5, decay
-0.1; rumble input `1.0 * A * 0.02 / max(1, d)`; explosion-effects.md section 2). Camera shake Start: `scale` over
`max(1, d * distance_scale)` when `#73440d3e` is set, times gain `#27750b42`; the setters store `max(old, new)` intensity and
overwrite speed and decay (`CameraShakeState`, mirrors CCameraControlManager +0x24/+0x28/+0x2c). The key names `scale`
(hash 4255180916), `distance_scale` (0xf1e162dd) and `speed` are the hashes of those names; `distance_scale`
is 1.0 in the retail rumble_and_shake files and `#73440d3e` is 0 in every retail shake, so the retail shakes are **not**
distance-attenuated (the loader defaults 0.1 and 1 are only used for absent fields). Haptics fields come in two equal pairs
(`#be0b5e5f` = `#015128a1`, `#263580f8` = `#3f1d2ee2`); PC has no device, so a start is only counted.

**Inferred from the data (labelled in code).** Decal `#7bb996a0` / `#99bfeaf1` are the min / max uv corners of the texture
cell, not offset and scale: all 95 retail variations then lie inside the sheet (bullet marks are the 4 x 2 cells of the
256 x 128 `BulletImpacts` sheet, explosions the full texture); read as scale the cell would reach uv 1.5. `size` =
(width, height, depth, ?) in metres (bullet marks 0.25 / 0.4 wide, 0.3 deep; scorch 4.1 to 6 m).

**Non-native stand-ins.** The decal renderer is Bevy's `ForwardDecal` (a flat quad blended against the depth prepass, so the
depth value is not used and `#9b006d73`, the specular and the normal texture are carried but unused); it is lit by the
Bevy lights, so it does not take the project's point-light or hemisphere terms. A decal is removed when its lifetime ends
(no fade) and at most 256 decals live (oldest first). Opt-in fixtures: `JC2_DECAL_FIXTURE=<delay>` (`JC2_DECAL_FIXTURE_DIST`)
ray-casts to the ground and plays a scorch plus eight bullet marks; `JC2_EFFECT_NO_DECAL_DRAW=1` consumes decals without
drawing them (the before capture).

**Gaps (not guessed; the existing gap tasks cover them).**
1. f7426df4: the decal manager (0x00573150, vftable 0x00eba00c): projection volume, the fourth size value, `#9b006d73`,
   fade-out, count cap, attachment to the hit objects, the original shader (DecalSimple / DecalDeformable) and the use of
   the specular and normal textures.
2. b6762d43: the per-frame consumer of the shake accumulator and the rumble accumulator. `CameraShakeState` and
   `RumbleState` are filled as the natives do but **no camera reads them** at the time of that task; the shake consumer is now `effects/camera_shake.rs` (b6762d43, `camera-shake.md`); the rumble reader is still untraced.
3. b73c5209: what the rigid-body virtual (+0x20/+0x24 from 0x008074e0) does with A, B, C. **No impulse is applied by the
   force pulse component**; it only counts the bodies inside its box (`PulseStats`) so it cannot double the tuned pushes in
   `explosion.rs`. Rumble: which of the two haptic pairs is the left motor is unknown.
