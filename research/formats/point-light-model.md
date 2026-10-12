# Native point-light model: muzzle flashes, explosions, fires (JC2-152) - 11 October 2026

Research only (task JC2-152, gap 4 of JC2-139). Evidence: the unmodded `JustCause2.exe` (sha256 in the probe), the shader bundle
`DX10_Shaders_F.shader_bundle` and the retail effect files. Probe: `research/tools/point_light_probe.py` (45 checks, exit 0; every
address and number below that is marked "probe" is asserted there). No Cargo, GPU or game launch.

Labels: **native** = read from the exe / shader / data and asserted by the probe; **decompiler-read** = from the pseudo-C only, not
byte-checked; **inferred** = reasoning, not proven; **unknown** = listed under Gaps.

## 1. What a "point light" is in JC2

The light object is `NGraphicsEngine::CLightManager` (constructor 0x00956a70; the object that earlier notes called "CRenderEngine+0x830"
is this class, and its `+0x830` scale is set to 1.0 by that constructor, so gap G5 of `light-tables-next.md` is closed: the factor is 1.0).
There are three unrelated things that look like lights; only the first is a lighting term:

1. **Managed point lights** (this document): a 0x74-byte light struct (created by 0x00949250) fed from keyframed "light source" effect
   components. They light geometry through the `cb0[15..144]` pixel-shader registers (section 4).
2. **`global/all_lights.light_info`** (string at 0x00e5c080, loader 0x00930000): 32012 records of 16 bytes
   (x, y, z as f32, one packed u32: top byte a small type id with 18 distinct values, low three bytes RGB), bucketed into an 8 x 8 grid
   of 4096 m tiles (0x0096d7b0) and drawn with the texture `Atmosphere\lens5.dds` (0x00946d70). These are **glow sprites** (halos), not
   lighting: they belong to JC2-136 and have no radius/attenuation of a point light. (Probe: header and size; the sprite reading is
   decompiler-read.)
3. Per-vertex/mesh emissive: not covered.

## 2. Where muzzle-flash / explosion lights come from (native)

Effect files `effects/*.bin` are component graphs (see `particles-next.md`). Component **type 7** is `CECI_LightSource`
(vftable 0x00eba988, constructor 0x0066e0d0; slot 0 Start 0x00724c50, slot 1 Stop 0x007cced0, slot 7 transform update 0x00546050). Its
parser is 0x006532a0 (the key names are hashed; the engine hash is lookup3 `hashlittle`, so `diffuse` = f460ed4f, `radius` = c8eb1006,
`multiplier` = 51e2e0de, `type` = 2f6488d6, `looping` = 5df1f8bf, node keys are `node_0`, `node_1`, ... built with a `"%s%d"` format and the
string "node_" at 0x00e4ac7c).

**Component fields** (one component = one light):

| Field | Hash | Meaning |
|---|---|---|
| `offset` | 1b20850d | vec3 added to the effect transform (e.g. (0,1,0) for muzzle_bigcannon); default 0 |
| `looping` | 5df1f8bf | int; 1 = the key list repeats; else the light dies after the last key |
| `node_i` | hashes of "node_i" | one key each; they are used **in node index order** |

**Key fields** (parser 0x006532a0 -> 13-word / 0x34-byte node; layout read by the interpolator 0x00959590):

| Node word | Field (hash) | Default | Role |
|---|---|---|---|
| 0..2 | (derived) | - | world position (effect transform x offset), written at Start/Update |
| 3..6 | `diffuse` f460ed4f (vec3), `multiplier` 51e2e0de (f32) | colour 0 (black) if `diffuse` absent; no multiplier = x1 | colour rgb = diffuse x multiplier; 4th word 0 |
| 7, 8, 9 | c5ec85ba, 12ca621e, 599fe7f1 | 1, 1, 1 | attenuation constants c, l, q (names unknown) |
| 10 | `radius` c8eb1006 | -1.0 | explicit radius; -1 = derive from c, l, q |
| 11 | 103d27f0 | 1.0 (only read when `type` = 0) | key time in seconds |
| 12 | 43676e89 (int) | 1 | stored into the light's `+2` short, used as sort priority (section 3); **no retail effect sets it** |
| - | `type` 2f6488d6 | - | key interpolation type; **all 537 retail keys are 0** |

Two authoring styles exist in retail data (probe): **radius style** (177 keys: `radius` + `multiplier`, e.g. muzzle_bigcannon) and **attenuation
style** (360 keys: c, l, q only, no radius, colour <= 1, e.g. explosion_laser). 124 of 467 effect files contain a light component (130
groups; the bullet_impact_* effects hold two groups with multipliers 8 and 2).

**Radius derivation (native, 0x00957450).** With radius -1 the light's radius is the positive root of c + l*r + q*r^2 = 4, i.e. the distance
where the classic attenuation 1/(c + l r + q r^2) falls to 1/4 (q = 0 gives (4 - c) / l; no root gives 0). The probe verifies the residual
over all 360 keys. Examples: (1, 0.04, 0.001) -> 38.31 m (explosion_laser); the (0, 0.1, 0.01) a fresh light struct starts with (0x00959590) -> 15.6 m. The same function also stores
c, l, q at light `+0x58/+0x5c/+0x60`; the shader path does **not** use them (section 4), only the resulting radius.

**Time evolution (native, 0x00959590).** Per frame the light advances its clock by the frame delta (the function called there is
0x004d3690; that it is the frame delta is inferred) and linearly interpolates between node k and k+1 with fraction
(t - t_k) / (t_{k+1} - t_k); when that interval is <= 0 or the fraction reaches 1 the index advances (fraction 1). Interpolated: position,
colour (all four words) and radius (radius is interpolated linearly only when both nodes carry an explicit radius; otherwise c, l, q are
interpolated and 0x00957450 is re-run). After the last node: if `looping` the clock and index reset to 0, else the light is removed from the
manager (its slot is swap-removed). Node times are ascending in node order in all inspected files; nothing sorts them.

## 3. Per-frame light selection and caps

| Stage | Native fact | Source |
|---|---|---|
| Managed light wrappers | at most **0x200 (512)**; creation refused above that | 0x0096b1e0 (probe) |
| Light structs | at most 0x3c0 (960) with priority < 1, 0x400 (1024) with priority >= 1; struct 0x74 bytes, byte 0 = active, short at +2 = priority, colour +0x18..0x24, position +0x38.., radius +0x50 | 0x00949250 (probe) |
| Distance cull | light kept only if squared distance to the camera < **90000** (300 m) | 0x00970120 and 0x0091dcc0 (probe: 90000.0 at 0x0108c698) |
| View test | sphere against the camera **cone** (section 10.2), not six planes | 0x00970120 (JC2-176: byte-checked field set) |
| Sort key | stored at light `+0x70` = trunc(max(r^2, 90000) * 300) / trunc(max(d^2, 1)) + priority * 300, unsigned 32-bit, every conversion truncates | 0x00970120 (JC2-176: **native**, byte-checked; the floor is real) |
| Frame list | cleared every frame, then sorted insertion into the list at `CLightManager+0x2a14`: **descending key**, ties keep insertion order; section 10.1 | 0x00911d80 / 0x0095f780 (JC2-176: **native**) |
| Records sent to the GPU | the first **64** entries (loop stops when the count reaches 0x40) | 0x0091dd80 (probe) |
| Distance fade | colour is multiplied by s * (1 - d^2 / 90000), where d^2/90000 is `[light+0x6c] * [0x013c179c]` and `[0x013c179c]` = 1.0 / 90000.0 (static initialiser 0x00c8e2c0). Lights with d^2/90000 >= 1 are skipped | 0x0091dcc0 (probe) |
| Global scale s | `[this+0x29b8] * [this+0x830]`: the gradients scalar (1.0 at night, 0.25 by day in variant 0; `light-tables-next.md` section 3) times 1.0 | 0x0091ea30, 0x00956a70 |
| Colour cap | every channel clamped to [0, **8.0**] (float 0x00ebf270). A light is dropped if **all** channels end below 0.01 (float 0x00f66934). A light whose radius is <= 0.001 is dropped | 0x0091dcc0 (probe) |

So the effective emitted colour is `clamp(diffuse x multiplier x s x (1 - d^2/300^2), 0, 8)`; for example muzzle_bigcannon's (1, 0.65, 0.2) x 8
= (8, 5.2, 1.6) at the muzzle, at night (s = 1) and 150 m away: x 0.75 = (6, 3.9, 1.2).

## 4. How the pixel shaders use a light (native for the car-paint family)

`CLightManager` builds per frame (function 0x0094bc10): a **128 x 128 grid of 4 m cells** centred on the camera
(`cb0[7]` origin, `cb0[8]` scale/offset), each cell an RGBA8 texel `LightIndex` (t7) holding up to **4 light indices**; when a cell would
get a fifth light the dimmest by `luminance x (1 - dist^2/r^2)^2` (luminance 0.3 R + 0.59 G + 0.11 B) is replaced. The constant buffer holds
64 lights in slots 1..64: `cb0[15 + i]` = (position - grid origin, **1 / (radius x radius)**) and `cb0[80 + i]` = (colour, 0); slot 0 is
unused (index 0 in the texel means "no light"). Probe: `carpaintlod2` pixel shader reads `cb0[r + 15].w` and `cb0[r + 80]` only, `cb0[145]`.

Per light the shader computes (a = saturate(1 - |L|^2 x w), L = light - surface point, N = unit normal):

    contribution = colour x a^2 x saturate(N . L / |L|)

(the instruction sequence in the probe: `mad_sat a = 1 - d^2 * w`, square, `rsq`, multiply, `dp3 L, N`, `mul_sat`). There is **no inverse-square
term**: the falloff is a smooth window that reaches exactly 0 at the radius, brightest at the light, weighted by the cosine. Bevy's
`PointLight` (inverse square with a range cut) is therefore a shape mismatch, not a unit mismatch. The shader sums the four terms into one
vector that the surface shader adds to its lighting. (JC2-176, section 10.3: it is added to the irradiance sum **before** the albedo product, before
fog, with no exposure step in the shader; the `plspec` specular is a separate term added after the albedo product, present only in the PLSpec / NSpec shader forms.)

## 5. Colour/radius table (native, effect data)

Node order, times in seconds, colour (rgb) x multiplier, radius ("d" = derived from c, l, q), offset in metres. Full list: run the probe
with `--table`.

| Effect | Offset | Keys |
|---|---|---|
| muzzle_bigcannon | (0,1,0) | t0 (1, 0.65, 0.2) x8 r10; t0.2 black |
| muzzle_flak | (0,1,0) | t0 (1, 0.65, 0.2) x8 r10; t0.1 black |
| muzzle_rocketlauncher_large | - | t0 black; t0.3 (0.5, 0.25, 0) ; t0.35 (1, 0.5, 0); t0.75 black; r13.03 d |
| every other `muzzle_*` (smg, assault, shotgun, revolver, beretta, minigun, rocketlauncher, grenadelauncher, airzooka) | - | **no light component** |
| bullet_impact_small / medium / large / huge / flak | (0,0.2..0.75,0) | t0 (1, 0.5, 0) [flak (1, 0.65, 0.2)] x8 and x2 groups; t0.05 (flak t0.15) black; r 1.5 / 2 / 3 / 5 / 16 |
| bullet_fire | - | t0 (1, 0.65, 0.2) x8 r3; t0.05 black |
| grenade_explosion, cluster_grenade_explosion | (0,0.5,0) | t0 (1, 0.65, 0.2) x8 r16; t0.2 black |
| missile_explosion | (0,2,0) | black; t0.1 (1, 0.5, 0) x8 r10; t0.5 black |
| explosion_barrel, explosion_mc | (0,2,0) | black; t0.15 (1, 0.5, 0) x8 r8; black at t1.5 / 0.8 |
| explosion_lave / lave_large | (0,1.75..2,0) | black; t0.15 (1, 0.5, 0) x8 r12 / r20; lave also t0.4 x4, t1.3 x0 |
| explosion_metalchamber small / medium / large / huge | (0,0.5..1.5,0) | black; t0.1 (0.7,0.3,0) / (1,0.5,0.1) / (1,0.7,0.2) / (1,0.5,0); black at t0.6..1.1; r 13.0 / 16.1 / 22.0 / 53.5 d |
| explosion_laser | (0,1.5,0) | black; t0.05 (1, 0.7, 0.3); black t0.3; r38.31 d |
| c4_flash | (0,0.2,0) | t0.05 (1, 0, 0); t0.1 black; r2.35 d |
| fire_small / medium / large / huge, car, lave_* | (0,0.4..2,0) | looping, 7 keys, t0..0.8; (1, 0.5, 0) [small (1, 0.4, 0)] x2.0..4.75; r 4.5 / 5 / 8 / 18 |
| big mission blasts (arve_land_explosion r30, explosion_arve_plane_large r70, f1m07_bomb_explosion r509 d, space_rocket_takeoff r586 d) | - | (1, 0.5, 0) family, long fades |

Radii in the data span 1 m to 70 m (explicit) and 2.35 m to 586 m (derived); multipliers 0..8. The overwhelmingly common colours are
orange (1, 0.5, 0) and (1, 0.65, 0.2); blue-white (0.3..0.8, 0.3..0.8, 0.7..1.0) for electric effects.

## 6. What jc2-rs should do (not implemented here)

* A Bevy `PointLight` cannot reproduce section 4 (no inverse-square in the original). The honest options are a custom lighting term with the
  window `(1 - d^2/r^2)^2 x cos` or, as a stand-in, a `PointLight` with `range = radius` and an intensity chosen per light, labelled
  non-native. The earlier policy strengths (native 1.0 / 1000 at 1 m) have no native basis and should be replaced by the table above.
* Data to ingest: component type 7 of every effect (`offset`, `looping`, ordered nodes with time, diffuse x multiplier, radius or c/l/q ->
  radius by section 2). Everything is already decoded by `point_light_probe.py`; the parser logic is a few lines.
* Runtime rules to copy: 300 m cull and (1 - d^2/90000) fade, per-channel cap 8, drop when radius <= 0.001 or colour < 0.01, 64 lights per
  frame, 512 managed lights, <= 4 lights per 4 m cell.
* Coordinate with JC2-136: the 32012 `all_lights.light_info` records are the glow sprites; the lens texture is `Atmosphere\lens5.dds`.

## 7. Verified / inferred / unresolved

* **Verified (probe):** the effect-component fields and defaults, node order, radius derivation, the 8.0 cap, 0.01 and 0.001 thresholds,
  300 m cull and fade, the 64 / 512 / 960 / 1024 limits, the 4-lights-per-cell grid and 1/r^2 packing, the car-paint shader's formula, all
  retail data statistics, the muzzle and explosion values above, and the `all_lights.light_info` header.
* **Decompiler-read, not byte-checked:** the view test, the sort key and its radius floor, the grid fill order, the replacement rule.
* **Inferred:** the clock is the frame delta; the sort is descending; `all_lights.light_info` = glow sprites.

## 8. Gaps

* **G1. Other light producers.** Only effect light components are decoded. `CDynamicLightObject`, `CDynamicLightFlasher` (RTTI
  0x00eb978c / 0x00eb9bac), vehicle head/tail lights ("Lightsource", "LightIndex" strings at 0x00e5cc8c / 0x00e5cd7c), traffic lights and
  the cutscene light track create or feed light structs through paths not traced; the colours/radii of street lamps, headlights and
  mission lights at night come from there. Blocks: native night lighting parity beyond effects. (Filed as a research task.)
* **G2. Selection order.** Direction and key of the sorted insertion (0x0095f780, 0x00911d80), the view-test planes and the odd
  `max(r^2, 90000)` floor of the sort key (0x00970120). Blocks: which 64 lights win when more than 64 are active (large fights).
* **G3. Shader coverage.** Only the car-paint pixel shader was diffed; General, terrain, character, facade, foliage and water families read
  the same `cb0` slots (568 pixel-shader blobs) but their combination with albedo/specular/exposure was not compared. Blocks: whether the
  light term is added before or after albedo and the exposure step.
* **G4. Effect graph selection.** Which of two light groups (bullet_impact_* x8 / x2) is spawned and how selection strings (`4:0`) map to
  groups; priority field 43676e89 has no retail user, `type` != 0 is unused. Blocks only duplicate-light cases.
* **G5. Names.** c5ec85ba, 12ca621e, 599fe7f1 (c, l, q by behaviour), 103d27f0 (time) and 43676e89 (priority) have no recovered name strings.
* **G6. Clock source.** Confirm 0x004d3690 is the unscaled/scaled frame delta (affects slow motion).

(JC2-176 update: G2 and G3 are closed by section 10; new gaps G15 and G16 follow section 10.5.)

(JC2-175 update: G1 is closed by section 9 - every creator is enumerated and decoded; the constant / linear / quadratic attenuation and time-less names of G5 are recovered,
the remaining names are in G7 below.)

## 9. Every other light producer (JC2-175) - 11 October 2026

Research only. Probe: `research/tools/light_producers_probe.py` (75 checks, exit 0, about 40 s with jc2tool; `--quick` runs the exe checks and two containers).
It reads the unmodded exe plus, through jc2tool, the retail location containers (1219 `*.nlz`), exported objects (463 `.ee`), cutscene sequences (224 `*.seq.blz`),
`settings\gamestaterun.bin` and `settings\worlddecorations.bin`; the static call-graph checks use `research/analysis/calls.csv` when present (SKIP otherwise). Labels as above:
**native** = asserted by the probe, **decompiler-read** = pseudo-C only, **inferred**, **unknown** (see Gaps G7 onward).

### 9.1 Completeness: the creators are exhausted

`CLightManager` create-light (0x00949250, takes the priority; refuses above 960 structs with priority < 1 or 1024 with priority >= 1) is called from exactly 10 sites and the managed-light
wrapper constructor (0x0096b1e0, effect-style keyed light) from exactly 2; neither address occurs as a data pointer, so there is no virtual or table-driven creator (**native**, byte scan of
every E8/E9 rel32 in `.text` and of all sections). The 12 sites sit in 9 functions:

| Creator function | Owner | Priority | What it lights |
|---|---|---|---|
| 0x007d3e70 | `CDynamicLightObject` and `CDynamicLightFlasher` (called from 7 sites in their methods, the set-colour 0x00692250 and the event listener 0x0057dfb0) | field +0xa8, 0 or 1 in data | placed lamps, mission lights, decoration lamps, traffic-light bulbs (9.2, 9.3) |
| 0x0059be90 | `CVehicle` vtable slot 67 (air, sea, plane, helicopter, boat keep it) | 5 (head), 1 (tail), 5, 5 | vehicle night lights (9.4) |
| 0x00547520 | `CLandVehicle` / `CCar` / `CMotorBike` slot 67 override (calls 0x0059be90 first) | 5 | siren light (9.4) |
| 0x00559150 | `CVehicle` slot 78 (reached through three wrappers) | 0 | vehicle death flash, a managed light (9.5) |
| 0x0090f4c0 with constructor 0x008b9120 | `NCutscene::CCutsceneTrackLight` (slot 8) | track `priority` (default 1) | cutscene lights (9.6) |
| 0x00694610 (creator), 0x006a2310 (per frame) | game-system init / main update | 99 | player-following night fill light (9.7) |
| 0x00724c50, 0x00959590 | `CECI_LightSource` and the managed-light interpolator | node word 12 | effect lights (sections 2-5) |

Not created through these: a light struct embedded in the light manager at `[0x011907c4] + 0x834`, written by the sky object (0x00a58640 constructor, 0x00a58a00 per frame; 9.8). `CLightCharacter`
and `global\all_lights.light_info` (glow sprites, JC2-136) are not point-light producers. The colour / active setters (0x00932990, 0x0093ac50) are also used by 0x005d8bc0 (`CDynamicLightObject`
slot 37, switch off), 0x008edf10 (`CCutsceneTrackLight` slot 5, switch off), 0x007e98d0 (brake light), 0x00804770 (vehicle fade dimming) and the sky object - none of them creates a light (**native**).

### 9.2 Placed light objects: `CDynamicLightObject` and `CDynamicLightFlasher`

Ordinary game objects (the class name is in the property data). Retail census (**native**): **30580 `CDynamicLightObject` and 27 `CDynamicLightFlasher`** in 892 of the 1219 location containers
(19462 lamps in `wc` world cells, 6217 in `metropolis`, 1651 in faction missions, 1210 key missions, 1004 strongholds, 704 general interests); 19 of the 463 exported `.ee` objects embed 27 more lamps and
4 flashers (roadblocks, SAM sites, submarine, fuel depot, campfire, beacon, balloon fighter, ...); `settings\worlddecorations.bin` defines 6 more (9.3). One container holds up to 573 lamps
(`city.district.c2.roads`). There is **no `CTrafficLight` placement in any location container or exported object.**

**Property keys.** Names are lookup3 hashes; the key objects are static data read by the property reader 0x007daaf0 in this order (**native**):

| Key | Object field | Meaning / constructor default (0x005dd880) |
|---|---|---|
| `world` | +0x3c (4x4 matrix; translation at +0x6c..+0x74) | light position = matrix translation, no offset; identity |
| `enabled` | +0x38 byte | 1 |
| `diffuse` (vec4) | +0x80..+0x83 bytes = (int)(x * 255) each | 0; the 4th value (alpha) goes into the light's 4th colour word, which no shader reads |
| `multiplier` | +0x84 | 1.0 (key absent on 236 lamps) |
| `constant_attenuation`, `linear_attenuation`, `quadratic_attenuation` | +0x88, +0x8c, +0x90 | **1, 0.1, 0.01** (names recovered here); used only when `radius` is -1 |
| `radius` | +0x94 | -1 = derive from c, l, q (13.03 m at the defaults); key present on 30427 of 30607 lights, written as -1 on 2182 |
| `fade_in_duration` (d93fd02e) | +0x98 | 0.2 s |
| `duration` | +0x9c | -1 = never expires (30576 of 30580 lamps) |
| `fade_out_duration` (8b8062f5) | +0xa0 | 0.2 s |
| `priority` | +0xa8 (short) | 0; data uses 0 (27945 lamps) or 1 (2635) |
| `flash_pause` (b2485315, flasher only) | +0xbc | 1.0 s |
| 1523c5bf, af2c988f, 9bd6fa33 (unnamed) | listener fields +0xac, +0xb0, +0xb4 | enable-event name, disable-event name, set-transform event name |

**Per-frame behaviour.** `0x00778a30(dt)` (slot 7, decompiler-read): while `enabled`, intensity (+0xa4) rises by dt / fade_in_duration (jumps to 1 when fade_in <= 0); while disabled it falls by
dt / fade_out_duration; it is clamped to 0..1; when `duration` >= 0 and intensity has reached 1, `duration` counts down and `enabled` is cleared at 0. Then `0x007d3e70` refreshes the light (**native**):
intensity < 1/256 switches the light off; otherwise the light struct is created on demand with priority = +0xa8, the colour is `(diffuse bytes / 255) * multiplier * intensity` in all four words (0x932990), the
radius is `0x91f270(radius)` when the radius field is not -1 and otherwise `0x957450(c, l, q)` (derived radius), the position is the matrix translation (0x946e70) and the light is switched on. If creation fails
(struct cap reached) the handle stays null and is retried at every refresh. **There is no time-of-day test in these objects**: lamps burn all day, the day/night difference is the global scale `s` of section 3
(1.0 at night, lower by day, from the gradients), plus the 300 m fade and the 8.0 cap. Only the disable event (power failure, destruction) or `enabled = 0` darkens a lamp. Set-colour 0x00692250 (called from
the traffic-light code) rewrites the four bytes as (int)(x * 255) and refreshes; 0x005fe510 sets the matrix; 0x0057dfb0 handles the enable event (resets intensity to 0 so the fade-in replays), the disable event and
the set-transform event by name.

**Flasher** (state machine 0x0084ee90, decompiler-read): state 0 fades in over `fade_in_duration`, state 1 holds for `duration`, state 2 fades out over `fade_out_duration`, state 3 stays dark for
`flash_pause`, then it repeats while `enabled`; clearing `enabled` forces the fade-out. Retail flashers are strobes: fades 0.01 to 0.2 s, hold 0.1 s (18 of 27) or 0.01 to 0.05 s, pause 0.5 s (18) or 0.1 s (5),
colours pure red, green, blue or cyan, multiplier 1 (21) or 8 (6), radius 15 (18), all priority 0; they sit in 5 world cells and the two Mile High Club mission containers.

**Retail lamp values (native census).** Most common configurations (count, `diffuse`, multiplier, radius, priority):

| Count | Diffuse | Mult | Radius | Prio |
|---|---|---|---|---|
| 3314 | (1.0, 1.0, 0.6) | 3.5 | 11 | 0 |
| 2607 | (0.8, 1.0, 0.8) | 4 | 13 | 0 |
| 2072 | (1.0, 1.0, 0.6) | 3 | 5 | 0 |
| 2000 | (0.7, 1.0, 0.5), alpha 0 | 8 | 15 | 0 |
| 994 | (1.0, 0.85, 0.55) | 3 | 5 | 0 |
| 769 | (0.8, 0.8, 1.0) | 4 | 5 | 0 |
| 752 | (0.9, 1.0, 0.75) | 2 | 3 | 1 |
| 722 | (0.65, 0.7, 1.0), alpha 0 | 2 | 3.5 | 0 |
| 696 | (1.0, 0.0, 0.0) | 3 | 4 | 0 |
| 582 | (1.0, 0.9, 0.09) | 1 | derived | 0 |

Multipliers run 0.4 to 8.0 and effective radii 1.7 m to 150 m (none <= 0.001, the drop threshold); alpha is 1.0 on 25169 lamps and 0.0 on 5133. The commonest street lamp emits (3.5, 3.5, 2.1) at the lamp at night
(before the 8.0 cap and the distance fade) over an 11 m window. 1773 lamps start disabled; 17954 listen for a disable event whose name contains `::power...` (location power-off), 2115 for an enable event
(e.g. `cutscene_lights_on`), 2 for a transform event. `constant / linear / quadratic` take c = 1 on 30468 lamps, l = 0.1 on 29301, q varying from 0.004 to 0.5 (0.01 on 15417): they only set the radius of the
2182 lamps written with radius -1.

### 9.3 Runtime street furniture and traffic lights

`settings\worlddecorations.bin` is a catalogue of procedurally placed objects (spawn_distance 120; the road-marker group 150) that embeds full object definitions (**native**). Five lamp-post groups each carry one
`CDynamicLightObject` child: a short post (`go063-f`) (1.0, 0.85, 0.55) x3 r5, c, l, q = (1, 0.1, 0.2), at (0, 0.75, 0); `go063-k` (0.7, 1.0, 0.5, alpha 0) x8 r15 at (0, 8, -5); the medium streetlight `go063-b`
(0.95, 0.9, 0.5) x4 r8 at (0, 4, -4); a second `go063-b` post (0.9647, 0.8863, 0.4784) x2 r6 at (0, 4, -4); and `go063-l` (1.0, 1.0, 0.6) x3.5 r11, c, l, q = (1, 0.1, 0.05), at (0, 4.6, -1.5). Their disable event is
`local::destroyed location::poweroff` (or `local::lightoff ...` for the short post): the lamp goes dark when its post is destroyed or the location loses power. The same file defines the only `CTrafficLight`; its child
bulb is a red (1, 0, 0) light with radius -1, multiplier default 1 and c, l, q = (1, 0.2, 0.02) (derived radius 8.2 m) at (0, 2, 0).

`CTrafficLight` (vftable 0x00ebe1ec) has **no light creator of its own** (no call chain within 6 levels to either creator in the static call graph; indirect calls are not followed). Its
`CTrafficLightCallback` (vftable 0x00e45b40, slot 0 = 0x00529040) receives a junction state and calls 0x00822a00, which walks the traffic light's 0x48-byte child records and, for every child of class
`CDynamicLightObject`, calls set-colour 0x00692250 with: state 2 -> green (0, 1, 0), state 1 -> yellow (1, 1, 0), state 0 -> red (1, 0, 0) (any other value white). The visible bulb colour is therefore driven by the
junction state machine; the data above only gives the initial red, radius and falloff. The state semantics and timing are in the road/traffic system (Gap G11).

### 9.4 Vehicle lights (`CVehicle` slot 67 = 0x0059be90, siren in 0x00547520)

Native facts (probe) unless marked. The lights are active only while the **world hour** (`CWorldTime`, 0x01191154, +0x20) is below 7.0 or above 17.0 and the vehicle's lights flag (+0x35e) is set (plus part-damage
checks, decompiler-read). Four lights are created lazily by this function and afterwards only switched:

| Handle | Priority | Colour | c, l, q | Derived radius | Position |
|---|---|---|---|---|---|
| head (+0x150) | 5 | (1, 1, 1) | (1, 0.05, 0.07) | 6.2 m | midpoint of the two headlight part positions when both exist (decompiler-read), else pos - axis * 4.5 + up * 0.7 |
| tail (+0x154) | 1 | (0.6, 0, 0); **(1, 0, 0)** when a condition flag is set in slot 68, 0x007e98d0 (inferred: braking) | (1, 0.05, 0.5) | 2.4 m | pos + axis * 4.5 + up * 0.7 (or the tail part pair) |
| fixed A (+0x158) | 5 | (1, 1, 1) | (1, 0.05, 0.2) | 3.75 m | vector at vehicle +0x160, only if the component at +0xb0c exists and its flag +0xb00 is clear |
| fixed B (+0x15c) | 5 | (1, 0, 0) | (1, 0.05, 0.2) | 3.75 m | vector at vehicle +0x16c, same rule with +0xb1c / +0xb10 |

(Colours, c, l, q and the 4.5 / 0.7 / 2.0 / 0.5 offsets are static data read only by the vehicle code; handle offsets and conditions are decompiler-read; "axis" is the vehicle's longitudinal basis vector at
+0x308..+0x310, sign convention not independently checked.) The headlight is therefore a **point** light 0.7 m above the vehicle origin with a 6.2 m window, not a cone. Slot 45 (0x00804770 for `CVehicle`,
`CSeaVehicle`, `CBoat`) dims the four lights while a vehicle fades (mode 2: colour * (1 - level / 255)) and switches them off for mode >= 3 (decompiler-read). Slot 68 also drives emissive material values 0.1 (idle) / 1.0
(flag set) on the lamp parts.

**Siren** (`CLandVehicle`, `CCar`, `CMotorBike`): a fifth light (+0xbc0, priority 5) with explicit radius **12.0 m** (data 0x00e3aa6c) while the siren flag (+0xd9f) and its part conditions hold. Colour by phase:
phase advances 3.5 per second and wraps from 1.0 by subtracting 2.0, colour = ((1 - |phase|) * 1.5, 0, |phase| * 1.5) (red to blue sweep); the position moves around the roof centre with an amplitude of 3.5 m driven
by a sine of the phase (decompiler-read).

### 9.5 Vehicle death flash (`CVehicle` slot 78 = 0x00559150)

The death routine ends by building a managed light with three keys at the vehicle position (**native** constants, decompiler-read structure): t = 0 black, t = 0.05 s colour (2.0, 1.4, 0.6), t = 1.5 s black,
(c, l, q) = (0.1, 0.008, 0.04) on every key (derived radius **9.77 m**), priority 0, not looping. The radius word (key word 10) of the three keys is never written in the function body, so whether the flash uses the
derived radius or a stack leftover is **unknown** (Gap G9).

### 9.6 Cutscene lights (`NCutscene::CCutsceneTrackLight`)

563 tracks with 2586 keys in 136 of the 224 sequences (**native** census). Per frame the track evaluates eight animated channels: colour R, G, B (keys hold 0..255; the parser multiplies by 1/255), `Multiplier`,
offset x, y, z (transformed by the track and scene transform) and `Range` (explicit radius); the light colour is rgb * channel 3 and the light is switched off as soon as any channel fails to evaluate. The light's
priority is the track `priority` (default 1; data: 1 on 475 tracks, 0 on 72, 50 on 7, 99 on 6, 10 on 3). Key fields: `time`, `Range` (0 to 500 m), `active` (always 1), `Multiplier` (absent on 86 keys), the colour key
49f3ca26 (e.g. [200, 10, 10, 255]), the offset key 182a3d96, an interpolation type 38a10c69 (4 on 2309 keys, 0 on 220) and four flag keys (all 1 in the data).

### 9.7 The player-following night fill light (priority 99)

Created inactive at start-up (0x00694610, handle in 0x01195900) and updated every frame (0x006a2310, called from 0x00781d60). Parameters come from `Settings/GameStateRun.bin` over built-in defaults
(hours 5, 6, 18, 19; offsets 0; colour (1, 1, 1, 1); range 3). **Retail file (native):** hours 5 / 8 / 18 / 20, vertical offset 2.5 m, back-off 0, colour **(0.2, 0.3, 0.5)**, range **15 m**. Decompiler-read: the
light sits at the transform held at `[0x011919f4] + 0x10` (inferred: the active camera) minus its forward vector * back-off plus (0, offset, 0), radius = range, and intensity follows the world hour: 1.0 for
hour <= 5 or >= 20; falling linearly from 1 to 0 between 5 and 8; **off from 8 to 18**; rising from 0 to 1 between 18 and 20; colour = colour * intensity. It is forced off while a game-state field
(`[0x0118fb84] + 0x5c >= 1`, meaning inferred: pause or cutscene) is set. With priority 99 its sort key beats every other light. This is a **non-diegetic bluish fill** with no counterpart in the effect or object data.

### 9.8 Not a point-light producer: the sun light

`0x00a58640` (sky object constructor) points at a light struct embedded in the light manager (`[0x011907c4] + 0x834`), gives it colour (0.5, 0.5, 0.5), a normalised direction (0.577, -0.577, 0.577) through the
direction setter 0x009727c0 and priority 100 (decompiler-read); 0x00a58a00 rewrites it per frame and the lens-flare pass reads its screen position. It is the directional sun/moon light of the sky system, outside
the 0x74-byte point-light creators and the cb0 point-light registers (Gap G12).

### 9.9 What jc2-rs should do

* Load the 30607 placed lights (plus the exported and decoration ones) with the field table in 9.2, convert the matrix translation, and treat lamps as always on with the gradient scale `s`, not a clock test.
* Final colour `(diffuse bytes / 255) * multiplier * intensity`, radius by the section 2 rule, then the manager rules of section 3 (300 m cull, 64 records per frame, 8.0 cap). With thousands of overlapping
  lamps the 960 / 1024 struct caps and the 64-per-frame cap decide what is visible (Gap G8).
* Vehicles: the four lights of 9.4 between 17:00 and 07:00, siren and brake variants, and the 9.5 flash.
* Add the priority-99 bluish fill (9.7) as a camera-attached light; it is why open ground at night is not black.
* Nothing here supports a Bevy `PointLight` intensity; use the window falloff of section 4.

### 9.10 Verified / inferred / unresolved

* **Verified (probe):** the complete creator call list; object and cutscene key names, offsets and defaults; lamp, flasher, decoration, exported-object and sequence statistics; vehicle colours, (c, l, q), offsets, priorities, the
  hour thresholds and the siren radius; the night-fill defaults and retail settings; the vtable homes of every producer; the absence of a creator path from `CTrafficLight`.
* **Decompiler-read:** the lamp / flasher lifecycles, vehicle part-based positions and damage conditions, the siren orbit, the night-fill hour ramp, the dimming mode and brake details, the traffic callback state mapping.
* **Inferred:** the meaning of the traffic-light callback states, the pause gate of the fill light, the camera identity of `[0x011919f4] + 0x10`, braking as the slot-68 condition, the sun-light identity of the embedded light.

### 9.11 Gaps (new; numbering continues section 8)

* **G7. Name strings.** Event keys 1523c5bf, af2c988f, 9bd6fa33; cutscene colour key 49f3ca26, offset 182a3d96, type 38a10c69 and the flag keys 6ab54a82, 72573f92, 180fe12c, 933e6ce8; GameStateRun keys e12f9c1b,
  78ed85bb, 5abff115, f62cf153, a97e8e92, a7009817 have no recovered names (values are known). Blocks nothing visible.
* **G8. Which lights win in dense scenes.** The streaming radius of location containers and how many placed lamps are enabled at once (up to 573 per container, 30k world-wide) against the 960 / 1024 struct caps and the
  64 records per frame (with G2). Blocks: faithful city night appearance.
* **G9. Death-flash radius.** Key word 10 of the three flash keys (0x00559150) is not written in the function body. Blocks: the vehicle explosion light radius (9.77 m derived or a leftover).
* **G10. Cutscene key semantics.** The `Multiplier` default for the 86 keys without it, the track `priority` loader, interpolation types 0..4 of the channels (0x00872f80). Blocks: cutscene light brightness and timing.
* **G11. Traffic light state machine.** Which `CTrafficLight` instances exist at run time (the spawn rule of the catalogue entry), junction state numbering and timing, and the bulb outside the active colour. Blocks: traffic
  light appearance.
* **G12. Sun / moon light.** The embedded light at light manager +0x834 (0x00a58640 / 0x00a58a00): colour per hour, direction, and how the pixel shaders use it versus the gradient tables. Blocks: directional lighting parity
  (belongs with the sky / gradient work).
* **G13. Vehicle light placement and conditions.** The source of the part indices (vehicle +0xa68, +0xa78, +0xb68, +0xb78), the vectors at +0x160 / +0x16c, the lights flag +0x35e, the damage conditions and the slot-68
  condition in 0x007e98d0. Blocks: exact headlight / taillight placement per vehicle model and braking behaviour.
* **G14. Lamp tick scope.** Whether placed lamps tick at any distance or only inside an activation radius, and whether dt is slow-motion scaled. Blocks: only fade timing.

## 10. Which lights win, and where the light term enters the surface colour (JC2-176) - 11 October 2026

Research only. Probe: `research/tools/light_selection_probe.py <game dir>` (63 checks, exit 0, about 20 s; reads only `JustCause2.exe` and
`DX10_Shaders_F.shader_bundle`). Part (a) is checked against exe bytes plus a Python model of the key and the insertion; part (b) disassembles all 731
pixel and 168 vertex shader blobs and taints the light colour forward to the output. Labels as in the header.

### 10.1 Selection order (closes G2) - native

Per frame `CLightManager::update` (0x00970120) does, in order (probe: bytes of each step):

1. Dead managed lights are removed (0x00959590 returned false); the vector at `CLightManager+0x2a14` is **erased** (0x008787f0 is `vector::erase(begin, end)`).
2. The active-light array (pointers at `+0x9ac`, count at `+0x19ac`) is walked from the **last index down to 0**. A light whose byte 0 is clear is skipped
   (byte 1 set means release the slot, 0x00440350).
3. Squared distance `d2` to the camera position (`camera+0x54..0x5c`, camera = the object at `[0x011919f4] + 0x10`); `d2 >= 90000` rejects.
4. A sphere-in-cone view test (10.2); a light that fails is **not listed at all** (it is not merely demoted).
5. Survivors get `[light+0x6c] = max(d2, 1)` (the fade input of section 3) and the key at `[light+0x70]`:

       key = trunc(max(r*r, 90000) * 300) / trunc(max(d2, 1))  -  trunc(priority * -300)       (unsigned 32-bit; x87 fistp with the control word set to round toward zero)

   `r` is the light radius at `+0x50`, `priority` the signed short at `+2`. The `max(r*r, 90000)` floor is real (the SSE compare/branch bytes are pinned): the numerator is
   27,000,000 for every light up to 300 m radius, so **radius is irrelevant to the ranking below 300 m**; only the few mission blasts with radius 509 m and 586 m
   (section 5) get a larger numerator. The ranking is therefore distance and priority.
6. The key is inserted with an `upper_bound` binary search (0x0095f780) whose test is `new_key > element_key` (unsigned), followed by `vector::insert(pos, 1, value)`
   (0x00911d80 calling 0x00615680): the list is sorted by **descending key**, and an equal key goes **after** the existing equal ones. Because step 2 walks the array
   backwards, among equal keys the light with the **higher array index comes first**.

Then `0x0091dcc0` walks the list from the **front** and writes records until 64 have been accepted (a light dropped for radius <= 0.001 or colour below 0.01 does not
use a slot, section 3); the grid builder `0x0094bc10` reads those same records (triple-buffered, slot `frame % 3`) and applies the 4-per-cell luminance rule of section 4.
So in a crowded scene the 64 survivors are the 64 largest keys.

What the key means (model in the probe; lamp radius up to 300 m): a priority-0 light at 50 m has key 10,800, at 100 m 2,700, at 200 m 675, at 299 m 302. Priority adds
300 per step, so a **priority-1 light 106 m away ties a priority-0 lamp at 100 m**; a **priority-5 light (vehicle head / fixed / siren lights, 9.4) 150 m away ties
a priority-0 lamp at 100 m**; the **priority-99 night fill light (9.7) adds 29,700**, which only a priority-0 lamp closer than 30.2 m can exceed. Effect lights use the node's
priority word (always 1 in retail data, section 2), placed lamps 0 or 1, cutscene tracks 0 / 1 / 10 / 50 / 99. A distance below 1 m counts as 1, and `trunc(d2)` makes
lights within the same integer d2 tie.

### 10.2 The view test is a cone, not planes (closes the view-test part of G2) - decompiler-read, field set byte-checked

The test (inline in 0x00970120, identical in form to the helper 0x00927830 that other callers use) reads camera-object fields written by the projection update 0x00963940:
apex `+0x5e4..0x5ec` (the camera position), unit axis `+0x5f0..0x5f8` (view forward), `+0x5fc` = tan(half-angle), `+0x600` = 1 / cos(half-angle), near plane `+0x624`.
With `v = light - apex`, `f = dot(v, axis)`, `lat2 = |v - axis * f|^2`, the light passes when

    f + r > near     and     lat2 < (tan * f + r / cos)^2

which is the exact sphere-against-cone test. The cone half-angle is the angle subtending the screen **diagonal**: `tan(half) = tan(fov/2) * sqrt(1 + a^2)` where
`a` = 16/9 if `[camera+0x634] > 1.0` else 4/3 (the code builds it from the constants 1280 x 720 or 640 x 480, not from the real window size; the meaning of +0x634 is unresolved,
and the vertical reading of the FOV at +0x618 is inferred). Sine and cosine come from a parabola approximation with the 0.225 correction (constants 2*pi, pi/2, 1/pi at
0x0108c5a4..ac), so the cone differs from an exact one by about 0.1 %. Lights behind the camera by more than their radius, and lights outside the diagonal cone, never
enter the list.

### 10.3 Where the light term enters the colour (closes G3) - native for all 163 shaders

**Census.** 731 pixel shaders; exactly **163** run the point-light loop (all declare `cb0[145]`; the "568" quoted in section 8 is not reproduced and should be disregarded). They are:
29 terrain (`FastTer_*`, `FastTerrainShader`), 13 tree bark (`Bark*`), 75 vehicle paint (`CarPaint*`), 23 characters (`SkinnedGeneral*`), 12 `Lambert*`, 3 `General*`,
4 `Facade*`, and one each of `Window`, `DecalSimple`, plus `SplineRoad` and `DirtRoad`. **No vertex shader indexes the light registers**, and **no foliage, grass, billboard,
particle, water, cloud, tree-impostor, vegetation, merged or underwater pixel shader runs the loop** (they declare at most `cb0[15]`): leaves, grass, water and sprites are not lit by
these registers. (10.4 adds a second light path.) The texture holding the 4 light ids per cell is bound at `t10` (terrain), `t4` (bark), `t7` (vehicle paint), `t3` (characters, decal)
and `t6` (general, lambert, facade, window, roads); the loop and the per-light window are the same in all 163 (probe).

**Rule.** In every one of the 163 shaders the summed light colour `L = sum_i colour_i * a_i^2 * saturate(N . Lhat_i)` (section 4) meets the surface like this
(24 distinct chains, fingerprinted in the probe; the representative sequences below are asserted literally):

| Family (shaders) | Chain after the loop |
|---|---|
| Terrain 29, Lambert 12, Bark 13, DirtRoad | `E = sun * N.L * shadow + hemisphere * ao + L`; `colour = albedo * E`; fog lerp toward `cb0[13]`; the terrain forms (and `Bark_Water`) add the underwater lerp toward `cb0[14]`; snow forms add a snow lerp |
| General, GeneralReflection | `E = sun term + hemisphere term`; `E = lerp(E, 2, material channel)`; **`E += L`**; `colour = albedo * E + environment/sun specular`; snow mix `lerp(.., cb1[1] * E, snow)` (the snow layer is lit by the same `E`); fog; underwater lerp; instance tint `colour * cb1[0].w + cb1[0].xyz` (`general-terms-next.md`) |
| Facade (+snow), Window, SplineRoad, DecalSimple | the same `E += L` then `albedo * E`; the window then blends toward a reflection sample by a Fresnel-like term and a luminance-normalised mix, all after `E` |
| Vehicle paint, 65 shaders (plain, lod0, lod1) | `E = sun + hemisphere + L`; `diffuse = E * k + base`; `paint = albedo * diffuse + sun specular + point specular`; environment-reflection lerp; fog; underwater |
| Vehicle paint lod2 (8), `CarPaintReflection`, `CarPaintSimpleReflection` | `albedo * (E * k + base)` (lod2); no light specular |
| Characters (`SkinnedGeneral*` without eyes) | `E = (sun term + hemisphere term) + L * m` where **`m` is the material texture channel `t2.x`**, the same scalar that scales the sun and hemisphere terms; then `albedo * E` + spec, fog, underwater add. Snow forms add the snow lerp |
| Eyes (`SkinnedGeneralEyeGloss*`, 5) | `L` joins the diffuse sum before the eye albedo product; **no fog step** in these shaders |

Consequences (all **native**):

* The point light is **diffuse irradiance**: it is added to the sun + hemisphere sum and multiplied by **albedo** with them. It is not an emissive add after albedo. Unlike the sun it
  is **not** multiplied by the cloud / sun shadow factor (`cb0[0]`, `t11`) and, unlike the hemisphere, **not** by the AO texel (characters scale it by `t2.x` instead).
* **Fog** (`cb0[13]`) and the underwater lerp (`cb0[14]`) act on the lit colour including `L`, so distant lit surfaces fade toward the fog colour; the eye shaders are the only exception.
* There is **no exposure step inside any surface shader**. The value is written to the sRGB 8-bit scene target (clamped to 1 per channel at the store); auto-exposure and tone
  mapping are later post passes (`night-exposure-next.md` section 4: the native pixel is `sqrt(clamp01(albedo * (sun + ambient + points)) * A * tint)`), which the chains above match.
* The light reaches the **alpha** output only in the two PLSpec facade shaders (specular luminance, 10.4).
* Clipping is per channel after the albedo product: with the 8.0 cap of section 3 a light of (8, 5.2, 1.6) on albedo 0.5 clips red and green.

### 10.4 The plspec option, the NSpec forms and the second light path (partly closes G3) - native shader facts, runtime selection unresolved

* **Point-light specular is optional.** The command line has `/plspec=b` ("Enable/disable pointlight specular", **default: disabled**; the parser stores it through 0x005b5530 as setting 0x2c)
  and the saved `GraphicsSettings` XML has a `PLSpec` attribute. The General block (constructor 0x00951920) registers `General` and `GeneralPLSpec`; the Facade block (constructor
  0x0095aa40) registers `Facade`, `FacadeWithSnow`, `FacadePLSpec`, `FacadePLSpecWithSnow`. By default the plain forms run, which have **no** point-light specular.
* The PLSpec shaders keep the diffuse chain unchanged and add `spec = sum_i colour_i * a_i^2 * saturate(R . Lhat_i)^n` over the same four lights (`R` is the view vector
  reflected about N, `n` the material exponent `cb2[0].x` for General), multiplied by the gloss mask, **after** the albedo product and **without** the sun shadow term. The facade form
  also writes the spec luminance into `o0.w`.
* **Vehicle paint lod0 / lod1 and the plain `CarPaint` always compute the four-lobe point specular** (65 of the 75 paint shaders; lod2 and the two reflection forms never), independent of the option.
* **Characters**: `SkinnedGeneral{,Hair,SHNear}{,1Spec,2Spec,3Spec,4Spec}` run N Phong lobes (N = 0..4; the probe counts one `log` per lobe in the loop); the eye shaders have none. What
  picks N was not traced (0x00932250 builds the names with `sprintf("SkinnedGeneral%s%s%s")`).
* **Second path: per-object lights (new).** 323 pixel shaders carry a `PL0` / `PL2` / `PL4` suffix (bark, vehicle paint, lambert, skinned characters; 105 / 109 / 109; the 2Spec / 4Spec
  skinned forms exist for PL2 / PL4 only). They declare only `cb0[15]`, have **no cell grid**, and read **2 (PL2) or 4 (PL4) point lights from the per-draw buffer `cb1`** (positions at
  `cb1[9]`, `cb1[11]`, ..., `1/r^2` in `.w`, colours in the following vectors) using the identical window and cosine formula; PL0 reads none. A runtime flag `cb4[0].x` switches a
  PL2/PL4 shader between a version that adds specular lobes and one that does not. Which draws bind PL0/2/4 instead of the grid shaders, and what fills `cb1[9..]`, is unresolved
  (the suffix strings are at 0x00e5bc7c "PL0", 0x00e5d6a0 "PL2", 0x00e5bcf4 "PL4", used by 0x00932250 (characters), 0x0092b8f0, 0x009489e0 and 0x00952530).

### 10.5 What jc2-rs should do (not implemented)

* **Shading:** replace "a Bevy point light" with a custom irradiance term `E += sum_i colour_i * (1 - |P_i - X|^2 / r_i^2)^2 * saturate(N . Lhat_i)` over the selected set, applied
  to terrain, bark, props (General / Lambert / Facade / Window), roads, decals, vehicles and characters, and **not** to leaves, grass, water, particles. Multiply it with albedo
  (then fog) together with the sun and hemisphere terms; do not shadow it with the sun shadow; scale characters by their `t2.x` channel. Point specular for vehicle paint (lod0/1)
  and, only when the option is on, General and Facade.
* **Selection:** implement 10.1 literally: cull `d2 >= 90000`, cone test, integer key, stable descending sort (ties by higher array index first), take the first 64 that pass the
  colour/radius thresholds, then the 4-per-cell rule. With thousands of lamps the key means the **nearest lamps always win** and radius does not matter; vehicle lights (priority 5) and the
  fill light (priority 99) override distance as computed above.

### 10.6 Verified / inferred / unresolved

* **Verified (probe):** comparator direction and signedness, the key formula and its truncating conversions, both constants (300, 90000), the clear / walk-down / insert sequence, the consumer's
  front-to-back read and 64 limit, the cone field set shared with 0x00927830 and its writer 0x00963940, the 163-shader census, loop shape and index texture slot of every shader,
  the 24 chain fingerprints, the literal chains of General, GeneralPLSpec, CarPaint lod2, Lambert, terrain, SkinnedGeneral and the eye shader, the lobe counts, the PL0/2/4 census and light counts.
* **Decompiler-read:** the cone equations and the aspect/resolution constants, the half-angle derivation, the byte-1 release branch.
* **Inferred:** vertical FOV at camera +0x618; `m = t2.x` as a material scalar (only its use is proven); that each chain's "albedo" is the diffuse colour (named by role from the
  surrounding disassembly, not by a resource name); the sun-specular colour role of `cb0[1]`.

### 10.7 Gaps (new; numbering continues section 9.11)

* **G15. Per-object light path (PL0 / PL2 / PL4).** Which draw submissions select the PL shader forms instead of the cell grid, how the 2 or 4 lights are chosen per object, and where
  `cb1[9..12]` is filled. Blocks: lighting parity for vehicles, characters and bark whenever that path is taken (visible). Filed.
* **G16. Point-light specular selection.** Where the PLSpec setting (slot 0x2c, default off) is consumed to choose `General` vs `GeneralPLSpec` and the Facade forms, and what selects the NSpec
  lobe count (0..4) of characters. Blocks: whether default-settings frames show any point specular on props and characters. Filed.
* **G17. Camera object +0x634 and +0x618.** What the `> 1.0` field is (aspect, widescreen flag) and the FOV reading; only decides the cone width. Listed, not filed.
* **G18. Array order.** The slot reuse order of the active-light array (+0x9ac) decides ties among equal keys (lights with the same priority and integer d2). Listed, not filed.
* **G19. Material scalars.** What `t2.x` (characters) and the paint shader's diffuse scale are in the material format. Listed, not filed.

## 11. Runtime implementation (JC2-221) - 11 October 2026

Code: `crates/jc2-game/src/point_lights.rs` and `point_lights.wgsl` (selection, window term, buffers), `crates/jc2-formats/src/point_light.rs`
(effect light tracks, placed lamps). It replaces the JC2-139 policy of Bevy `PointLight`s (inverse square plus a range cut). Labels as in the header.

**Done, native (cited above):**

* Per frame, in `PostUpdate` after transform propagation: candidates = enabled placed lamps (`LampLight`) and keyed effect lights (`EffectLight`); the 300 m cull
  (`d^2 >= 90000`), the sphere-against-cone view test (10.2), the key `trunc(max(r^2, 90000) * 300) / trunc(max(d^2, 1)) + 300 * priority` (10.1), the descending
  stable insertion walked from the last candidate down, the first 64 that pass `colour >= 0.01` and `radius > 0.001`, the colour
  `clamp(c * S * (1 - d^2 / 90000), 0, 8)` with `S = AtmosphereState::point_light_factor()` (the gradients scalar, 1.0 at night, 0.25 by day).
* The shader term (section 4, 10.3) in three families: General (`general_main.wgsl`), the hemisphere models (`hemisphere_main.wgsl`: Lambert, characters,
  vehicle paint, windows, facades, bark, roads, decals) and the terrain (`terrain_splat.wgsl`): `colour * saturate(1 - |L|^2 / r^2)^2 * saturate(N . L / |L|)` summed over
  the (at most four) lights of the 4 m cell, added to the irradiance **before the albedo product** (as `diffuse * L`), unshadowed, before the fog. Leaves, grass, water, particles
  and sprites are not lit (10.3). The lit output of this crate is already in native units (the sun lux and the ambient brightness carry `pi / e0` and `1 / e0`), so the term is
  added without the exposure.
* Producers: **placed `CDynamicLightObject` lamps** (9.2): `diffuse` bytes `(int)(x * 255)`, `multiplier`, explicit radius or the derived one, priority, fade-in ramp, no time-of-day
  test (they burn all day; only `S` dims them by day). Created by `world::place_objects` for every enabled lamp of a streamed container (and of exported entities), despawned with the tile.
  **Effect lights** (section 2): `jc2_formats::point_light::effect_light_tracks` decodes the type-7 component (offset, looping, `node_i` keys in order, diffuse x multiplier, radius or c/l/q ->
  radius, key interpolation as in 0x00959590, removal after the last key); `NativeLightEffects` spawns them. Retail check: `muzzle_bigcannon` and `missile_explosion` keys equal section 5.
* The muzzle-flash light is gone: only `muzzle_bigcannon`, `muzzle_flak` and `muzzle_rocketlauncher_large` carry a light component, and no weapon of the game is mapped to them.

**Inferred / non-native (labelled in code):**

* The cone's vertical FOV reading and `a = 16/9` for a wide aspect; the grid origin snapping (corner = camera cell snapped to 4 m, 64 cells either way), which cells a light
  reaches (centre within `r` plus the cell half-diagonal, horizontal distance) and the weight used to replace a fifth light (the native luminance rule, cell centre distance);
  ties between equal keys follow the Rust candidate order, not the native array order (G18); the effect clock is the Bevy frame delta (G6).
* **Which effect a blast spawns:** every blast uses `missile_explosion` (r 10, (1, 0.5, 0) x 8, 0.5 s) as a stand-in; grenade, rocket, barrel and vehicle blasts have their own effects natively.
* Lamps that start disabled (1773 in retail) and the enable / disable events (`::power`, `cutscene_lights_on`) are not wired: those lamps are not created.

**Not done (Gaps, none presented as native):**

* **G20. Per-object light path.** The PL0 / PL2 / PL4 shader forms (G15) are not used; vehicles, characters and bark take the grid forms. Point specular (`plspec`, vehicle paint lobes, character Phong lobes, G16) is not implemented.
* **G21. Character scale.** The characters' light term is multiplied by the material channel `t2.x` natively (10.3); the hemisphere shader adds it unscaled. Vehicle-paint and window chains after the loop (reflection lerps, 10.3) are not reproduced.
* **G22. Other producers.** Vehicle head / tail / siren lights (9.4), the vehicle death flash (9.5), cutscene lights (9.6), the night fill light (9.7), flashers, traffic-light bulb colours (9.3) and the per-hour sun light are not spawned.
* **G23. Blast to effect mapping** (above) and the muzzle / impact / fire effect lights (`bullet_impact_*`, `fire_*`, `c4_flash`) that the game does not spawn yet.
* **G24. Lamp events and activation** (G14): enable / disable events, the 1773 initially disabled lamps, whether lamps tick outside an activation radius.
* **G25. Cell grid details**: the native origin snapping and cell membership rule (`0x0094bc10` fill order) are inferred, so the 4-light truncation in crowded cells may differ from the original.
* **Side note (JC2-085 General material, not changed here):** `general_main.wgsl` multiplies the environment reflection term `(cb0[3] + cb0[10])` by `view.exposure`; those colours are native units, whose lit
  output needs no exposure, so the reflection is probably far too dark (by about the camera exposure, roughly 1000 times). Left as found; worth a check by the owner of that material.
