# Native vegetation model LOD selection, JC2-086

Retail `JustCause2.exe` SHA256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Reproduce (read-only,
exit 0 only if every check passes):

```powershell
& $py -B research/tools/vegetation_lod_probe.py $game --export "$root/research/analysis/pseudo/_unnamed" --tool $jc2tool
```

Runtime consumer: `crates/jc2-formats/src/veglod.rs` (pure functions + `SlotPlan`),
`crates/jc2-game/src/plant_lod.rs` (hard model swap), `scenery.rs` (`vegetation_lod == 0`).

## Proven

**Descriptor.** A `.lod` file is five slot lines (`-` or `Models\-` = empty) and an optional
sixth line. The parser (00a4fa60) feeds the sixth token through `atof` into model field
`+0x1c`, the model's **LOD scale**. `TLodModel` construction (00a47ab0) initialises that
field to 0x41200000 = **10.0**, so a descriptor without a sixth token keeps 10.
Retail census: 191 vegetation `.lod` files in the four zone containers; scales seen 1, 1.5,
1.6, 2, 3, 3.5, 4, 4.5, 6, 7, 10 (112 files use 1, 26 use 4, 22 use 3). The slot count that
selection uses is always **5** (`TLodModel` vtable+8 = 00a47e60 returns 5 once any slot is
loaded). Fixtures: `Arctic_T22_PineL-Whole.lod` and `T21_PineM`: LOD1, LOD2 x4, scale 4;
`Arctic_G01_RocksM`: LOD1, LOD2, LOD2, -, - scale 4; `Arctic_G03_RocksS`: LOD1, LOD2, -, -, -
scale 3; `Arctic_B11_bushtreeL`: LOD1 x5, scale 3.

**Owner distance multiplier.** The model instance owner initialiser 00a4f4b0 sets owner flags
(`+0c`) to `| 0x40b7` (bit0 enabled, bit1 distance cull, bit2 automatic LOD, all set), stores
the slot count (5) in flag bits 26..28, copies model `+0x1c` to owner `+4`, and writes owner
`+8 = 1 / (s*F)^2` with `s` the scale and `F = max(1, camera+0x620)` (00a42970 repeats this
for later scale changes). Camera `+0x620` is produced by 00920240 as
`pow(camera+0x61c * tan-ratio, 1.0)`; at rest `camera+0x61c` is 1.0, so **F = 1** in ordinary
play and rises only with a high zoom factor (value path not fully traced, Gap 3).

**Selection** (00a89b90, per instance and frame, camera position from 0138202c+54): with
`x = owner+8 * d^2` (squared distance from the camera to the model's selected position):

* cull: if bit1 is set and `x > table[count]` nothing is submitted. Table 00f0ea0c =
  0, 100, 625, 2500, 10000, 250000 (count 5 culls at 250000).
* level `L` (continuous): `x*0.01` below 100; then linear with constants 1/525 (100..625),
  1/1875 (625..2500), 1/7500 (2500..10000), 1/240000 (10000..250000) so `L` is 1, 2, 3, 4, 5 at
  100, 625, 2500, 10000, 250000 (constants at 0108a228..0108a238).
* slot = `min(count-1, trunc(L))`, fraction `k = trunc((L - trunc L) * 255)`; stored in owner
  flag bits 29..31 and 18..25 (also copied into a frame-parity buffer, `0108c594 & 1`).
  If `L >= count` it is clamped to `count-1` with k = 0.
* In metres at F = 1 the level reaches 1..5 at `{10, 25, 50, 100, 500} * s`. Arctic pine
  (s=4): slot boundaries 40, 100, 200, 400 m, cull 2000 m. Bush/rock s=3: 30, 75, 150, 300 m,
  cull 1500 m. Hard switch between slots: no cross-fade and no hysteresis in the original.
* Submission (00a42740) asks the model for the slot's meshes through vtable+0x10 = 00a47b40,
  which scans **forward** from the selected slot to the first non-empty one and returns nothing
  if none is left. A rock with an empty tail therefore draws nothing once the selection passes
  its last filled slot.
* Fade: only when the selected slot is the last one (4) or the next slot is empty
  (vtable+0x18 = 00a47bc0 returns 0) does the owner byte `+1f` follow `k` (k > 254: not
  submitted; else `f = 2 - k*2/255`, byte `trunc(f*255)` or 255 if f >= 1). All other slots write
  255. This is the JC2-025 fade byte; so native fade of a pine starts at about 400 m (slot 4,
  half the level span still byte 255) and the tree is gone at 2000 m, a rock fades inside its
  last filled slot's span.

## Project policy (not native, kept)

`scenery::TREE_RANGE` 520..580, `BUSH_RANGE` 380..440 and the VisibilityRange dither remain the
presence/hand-over to impostors; the native presence (up to 2000 m for a pine, nothing past
the last filled slot for rocks) is **not** reproduced. `MAX_SWAPS_PER_FRAME = 128` is a work
bound after a camera jump. Config: `vegetation_lod == 0` (new default) = native slot choice;
`--vegetation-lod N` (N >= 1) forces one `.lod` level for all plants as before.

## Runtime step delivered

Per plant type with more than one distinct slot model the tile builds a `PlantLodSet` (the
distinct models' handles, one reference per tile through `Streamer::model_handle` and
`tile.models`) shared by its instances. The plant keeps **one entity**: spawn uses the far
(last filled) model; `update_plant_lods` selects `SlotPlan::model_at(d^2)` from the camera and
replaces the entity's child meshes when the model changes (not-yet-loaded model: keep current).
Colliders are placed once per instance by `grow` and never touched; the set is dropped with
the entities; no per-instance handle cache. Plants whose slots all name one model (most bushes)
get no `PlantLod` at all. Headless retail test (tile 31,14, 3752 plants) checks: plants with
a switchable plan exist, spawn on the far model, a camera 5 m away swaps to the near model and
the child count equals that model's blocks.

## Gaps

1. **Impostor ownership/handover.** Which code decides tree impostor cards (CTreeImpostorTopRB /
   TrunkRB) versus the model, and their own distance/fade, is untraced; 00a96af0 only loads
   `TreeImpostors.bin`. Blocks removing TREE_RANGE/BUSH_RANGE and the 450..600 impostor fade.
2. **Last-slot fade not rendered.** The byte (formats `veglod::instance_byte`) is proven but not
   fed to `FoliageInstance`: per-plant private material clones across hundreds of metres are
   not bounded; needs a shared-state route (JC2-024 follow-up).
3. **Camera zoom factor F.** `camera+0x61c`/`+0x620` writers (00710000:3010, 12141;
   007f0000:8838) and the tan-ratio are not mapped to gameplay zoom; runtime uses F = 1.
4. **Patch record gate.** 00a89b90 only fades when record byte `+0xe` low byte is 0xff, and uses
   record `+0xe` bits 12..15 / 0x8000 for wind; their writers are unresolved. Also the pass
5. **Selection scope.** 00a8c9a0/00a8cb40 iterate a sorted patch list; which instances the
   native streams (vegetation patch radius, density cull `0x2584..0x2594` random gate, shadow
   pass) is not recovered, so near-radius cost measurements (radius 0/1) are not claimed.
6. **Non-vegetation consumers.** The same 00a4f4b0/00a89b90 chain serves other `.lod` models
   (static objects, vehicles); `world.rs` still resolves one level once for those.
7. **Visual captures.** No close full-detail/transition captures were taken; only the headless
   retail swap test and the formats fixtures.

## Non-vegetation consumers, JC2-128 (closes Gap 6)

The chain 00a4f4b0 / 00a89b90 is per model instance and does not depend on the model kind, so
placed static objects and vehicle parts use the same `veglod::SlotPlan` as plants (selector
proven by the JC2-086 evidence above; no separate selector was found). Runtime:
`world::ModelIndex::resolve_plan` (placed `.lod`: each slot's model searched in the tile
container, then the shared ones, as the single-level resolve did) and
`vehicle::load_doll_parts` (each doll part's `.lod`, slot models from the vehicle's own
container), both feeding the existing `plant_lod.rs` swap. `--object-lod 0` (new default) =
native slots for placed objects and vehicles; `N >= 1` = one fixed level as before.
`--lodfactor` feeds the distance unit.

Project policy, not native: an object whose collider is its render mesh (no `.pfi` list and no
own shape) keeps the single full-detail level (the swap replaces child meshes and would drop
the collider); the far cull (`x > table[5]`) is not applied (the current model stays, as for
plants); a placed object spawns on its far model and swaps near in the first frames; vehicle
parts spawn on the full-detail model. A vehicle body's bounds and collider come from the
full-detail model. Headless retail tests: tile 31,14 has placed objects with switchable slots
(none at `object_lod` 1); the APC has parts with slots.

Gaps: (1) camera zoom factor F is still 1 (Gap 3 above), which matters most for vehicles
close to the chase camera; (2) the native cull of placed objects (500 x scale m, scale default
10) is not reproduced; (3) mesh-collider objects stay single-level; (4) far metropolis batch
models (`scenery.rs` `resolve` with `cfg.lod`) and debris keep one level.

## Camera zoom factor F, vehicle parts (JC2-259, closes Gap 3 for dynamic models)

`00920240` (called by the projection update, `00960000` line 5448, so `camera+0x620` follows the
realised FOV every frame) returns `pow(camera+0x61c * R, E)` with `camera+0x61c = cot(fov/2)`
(`0092c620`), `R = tan(0.40840706 rad = 23.4 deg)` (the exe's own sine/cosine polynomials give
0.4327), the exponent `E` the float `0108c590` = 1.0 and the enabling byte `0108c58c` = 1 (both read
from the shipped image; only reads of the byte exist in the export). Consumers clamp F up to 1, so F
is above 1 only below a 46.8 degree vertical FOV: 1.1889 on foot (40 deg), 2.454 aiming (20 deg),
exactly 1 in the vehicle cameras (50 to 85 deg). The writers of `+0x61c` at `007141e0` / `007fa6e0` /
`0071f120` belong to character objects, not the camera.
Dynamic model instances re-take F at every submission (`00a85a60` -> `00a42970`); plants and placed
static objects take it once at owner creation (`00a4f4b0`).
Runtime: `veglod::camera_zoom`, `SlotPlan::model_at_zoom`, and `PlantLod.camera_zoom` (vehicle parts
only) in `plant_lod.rs`.

Observed (vehicle `.lod` descriptors, retail): the sixth token of vehicle parts is small
(`v062` helicopter body and rotors 0.2, APC body 0.4, motorboat and civilian helicopter bodies
0.75, most car parts 1.0), so the native boundaries at F = 1 are `{10, 25, 50, 100} * s` m: the
helicopter body is on its fourth slot beyond 10 m and the APC body leaves full detail at 4 m.
A driving or flying camera sits 6 to 25 m behind, so these parts leave slot 0 there while the
on-foot camera stands closer. Whether the original really renders its vehicles this way, or the
vehicle owner scale is overridden somewhere (`owner+4`, `00a42970` first lines) is NOT resolved.

Gaps: (1) the writer/override of owner `+4` for vehicle instances (the descriptor scale 0.2 would
put a helicopter on LOD4 at 10 m); (2) F for plants and placed objects is read once at owner
creation (when, with which FOV, is untraced), so they stay at F = 1; (3) the exact polynomial `tan`
(last digits only).

## Vehicle LOD level, closes the owner-scale Gap of JC2-259 (board 27da3660)

The owner path above (`00a4f4b0` / `00a89b90`, owner `+4` = model scale) is NOT what vehicles use for
their body. Findings:

* Owner `+4` is only ever the model scale or the `-1.0` "none" sentinel; `00a42970` writes it only while the
  distance-cull flag (bit 1) is set. No vehicle override exists. Camera distance only: nothing in the
  selection or in the vehicle code reads the occupant (`008c9100`, callers `008d8c00`, `008e1cc0`: distance from
  camera `+0x54..0x5c`).
* The vehicle code clears the automatic-LOD and distance-cull bits (`&= ~4`, `&= ~2`, `|= 0x800 | 0x1000`) on
  the models it loads by name (`006c5530`, wheel models; `00821c90`, generic named-model attach) so those
  owners are not auto-selected.
* The vehicle object's per-frame update `006a0e80` reads the model scale of its FIRST part
  (`006e58d0(0)` = element 0 of the 0x84-byte part vector, `+0x80` model owner, `+0xc4` model, `+0x1c`
  scale), calls a virtual (`vtable+0x12c`, takes `(scale, &position)`), clamps/splits the result into an
  integer level and a 0..255 fraction and applies it with `vtable+0xb4(1, level, fraction)`.
* The level function matching that signature is `0061bb00` (a twin with the same body, without the debug
  override, is `00855fd0`, used by roads): `x = d^2 (g s)^2 / F^2` (`d` camera to position, `s` the scale,
  `g` = `DAT_0107214c`, `F = max(1, camera+0x620)`), then linear in x between 196, 625, 4900 and 18225 for
  levels 1..4 (slopes 0.0051020407, 0.0023310024, 0.00023391812, 7.5046904e-5), 4.0 beyond, no cull.
  `g` is written by `006a07b0` from the graphics-setting handler (`00610000:1699`: 1, 0.6667, 0.5, 0.3333
  for the same four lodfactor values that set the owner unit 1, 1.5, 2, 3), i.e. `g = 1 / unit`.
* So the scale DIVIDES the distance for vehicles: breakpoints at `{14, 25, 70, 135} unit / s` m. Helicopter
  body (s 0.2): 70, 125, 350, 675 m. APC (0.4): 35, 62, 175, 337 m. Boat/civilian helicopter (0.75): 19, 33,
  93, 180 m. Cars (about 1): 14, 25, 70, 135 m. This explains the "implausible" tiny scales: a larger
  vehicle keeps detail longer. The earlier reading (owner multiplier, `{10, 25, 50, 100} s` m) put the
  helicopter body on its crudest model beyond 10 m.
* Runtime: `veglod::vehicle_scaled_d2`, `vehicle_level`, `SlotPlan::model_at_vehicle_level` (slot
  `trunc(level)`, forward scan to the first filled slot), `plant_lod::VehicleLod` (body scale from the first
  doll part, distance unit from `--lodfactor`). One level per vehicle drives every part's own slot list.

Captures (`--vehicle-capture DIR`, `vehicle_capture.rs`, with `--heli`/`--boat`/`--drive CONTAINER`): a
fixed camera 14 m from the parked vehicle, Rico outside, then Rico teleported next to it and `E` pressed
(the real `vehicle_entry`), the same frame again, plus `slots.txt` (model, multiplier, level, distance per
part). Before the fix the heli body was `LOD4` and the APC body `LOD3` at 14.6 m both outside and inside; the
visible drop on entering came from the chase camera standing further away (and F 1.19 on foot -> 1.0
driving). After: both are `LOD1` (levels 0.03/0.04 heli, 0.12/0.17 APC).

Gaps: (1) the vtable binding `006a0e80 +0x12c -> 0061bb00` and the application `+0xb4` (what the level and
the 0..255 fraction do to the parts: slot per part vs a fade between two) are inferred, not traced; we hard
switch. (2) Two state branches of `006a0e80` are not decoded: byte `+0xba6` != 0 skips the computation and
applies level 0 (full detail), and a float ratio `[0x3a]/[0x39] <= 0` clamps the level to 0..1; their
meaning (damage, occupant, cinematic?) is unknown, so neither is modelled. (3) The wheel models and the
`+0x128..` variant models have auto-LOD off and their level source is not traced, they stay as before.
(4) `006e58d0` element 0 is assumed to be the doll's first part (the body).
