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
