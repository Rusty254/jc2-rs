# The game zone map: which light set a place gets, and what the blend does (JC2-123)

Status: research brief. No runtime code changed. Probe: `research/tools/gamezone_map_probe.py` (103 checks, exits 0; needs a
private `jc2tool`, extracts the 4096 terrain tiles into a temporary folder and deletes it). All addresses are runtime
addresses of `JustCause2.exe` (no ASLR). Pseudo-C was read from `research/analysis/pseudo`; every instruction a claim depends
on was decoded by hand from the exe bytes and is pinned by a byte check in the probe. Builds on `night-light-next.md` (JC2-115).

## Headline

1. **The zone map is already in the game's terrain tiles.** `terrain\terrain_win32_XX_ZZ.dat` (4096 files, archive
   `archives_win32/pc0`) ends with the section the engine calls `CGameZoneMapStreamPatch`: `u32 12 | 64 x 64 bytes | u32 12` and
   then end of file. `jc2-formats` already keeps these bytes (`TerrainTile::gamezone`); nothing new has to be parsed.
2. **One byte per 8 m cell, row = z, column = x:** `type = (b >> 3) & 7` (the `gamezone.bin` ids: 0 None, 1 Arctic, 2 Desert,
   3 Jungle, 4 Sea, 5 City, 6 Grass, 7 Default; the light set is the same number), `intensity = b >> 6` (0..3), and the low three bits are a
   third field (`b & 7`). These three accessors exist in the exe as `FUN_00a5e3f0`, `FUN_00a5e400`, `FUN_00a5e410`.
3. **World to cell (float32, no bounds check):** `u = (x + 16384) / 512`, tile `X = floor(u)`, `cell = trunc(frac(u) * 64)`, the same for z; byte
   = `tile(X, Z)[cell_z * 64 + cell_x]`. The engine holds only an 8 x 8 ring of tiles (`(X & 7) + (Z & 7) * 8`) and does **not** check
   that the resident tile is the one asked for (`FUN_00a5eb50`).
4. **Default bytes:** a freshly constructed map is all zero (tile coordinates -100000, so nothing matches); the vtable "reset" fills a patch with
   **0xE0 = Sea, intensity 3** (`FUN_00a56090`, `memset(..., 0xe0, 0x1000)`). 0xE0 is also the most common value in the data (44.7 % of all cells).
5. **Fixtures:** the start base `km01.base` (-1280, -6712) is tile 29,18, cell (x 32, z 57), byte **0xC8 = Arctic, intensity 3**, so its light set is
   Arctic (1), variant weight 1. Desert 0xD0, Jungle 0xD8, Sea 0xE0, City 0xE8, Grass 0xF0, Default 0xF8 at intensity 3 (section 5).
6. **The branches of `FUN_00a5d390`** are now a complete input model (section 7): height (set 8, from 30 m above ground, fully set 8 from 300 m), underwater
   (set 9, depth 120 m, replaces the zone intensity by the depth ratio), rain (set 10) and snow (set 11) are lerps over the zone colours in a fixed order, with the
   weights `up / 270`, `down / 120`, rain, snow. The zone **intensity only changes colours in two table rows** (Desert sky ambient, Jungle ground ambient) and in set 9.

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| the 4096 zone maps, tiles X-major then Z, 16,777,216 bytes (probe `ZONE_MAP_SHA256`) | `6cfe77230e996f1cae7e82db1d943be3dd58dc986767125832aad3b9ce8f7cab` |

Tile files were read with `jc2tool extract --filter terrain_win32_` (zlib removed). They are not committed. The cell-value histogram is in the probe (`HIST`, 59 distinct values).

## 1. The data (proven)

* **File and place.** `terrain\terrain_win32_%02d_%02d.dat` with `(X, Z)` (`FUN_00a3f640`; `terrain.md`). Every one of the 4096 files starts with `u32 12, u32 123` and ends with
  `u32 12 | 4096 zone bytes | u32 12` (probe: 0 exceptions). The sections before it are the five stream patches in order; the zone map is the last.
  `DAT_01190290` (terrain heights/materials), `DAT_01191d98`, `DAT_01191224`, `DAT_011f9c68` and **`DAT_01381af8` (the game zone map)**, then reads one last `u32`. The pointer order is byte-checked.
* **Map object.** `CGameZoneMap`, `0x40308` bytes (`push 0x40308` at `0x00a750aa`), constructor `FUN_00a581a0`: a vtable word, then **64 patches of stride `0x100c`** starting at +4. A patch is
  `{ vtable (CGameZoneMapStreamPatch, 0x00f1fc1c), tile x (i32), tile z (i32), 0x1000 data bytes }`, so the data starts at map+0x14+patch*0x100c. The constructor sets both tile numbers to
  `0xfffe7960` (-100000) and zeroes the data.
* **Per-tile load.** Map slot 0 (`FUN_00a58200`) picks patch `(X & 7) + (Z & 7) * 8` and calls patch slot 1 (`FUN_00a530e0`): store X, Z at +4/+8, then slot 2 (`FUN_00a560b0`): `reader->read(patch+0xc, 1, 0x1000)`.
  The 4096 bytes are copied unchanged (no flip, no transform).
* **Reset.** Map slot 1 (`FUN_00a58230`) picks the same patch and calls patch slot 3 (`FUN_00a53130`: store X, Z, call slot 4 = `FUN_00a56090`, **`memset(patch+0xc, 0xe0, 0x1000)`**). The map's reset is
  reached from `FUN_00a56000`, slot 1 of `CTerrainPatchReader`, which resets all five maps for one tile coordinate. Its caller is an indirect call that was not located (gap 1).
* **Accessors.** `FUN_00a5e3f0` returns bits 3..5, `FUN_00a5e400` bits 6..7, `FUN_00a5e410` bits 0..2, `FUN_00a56080` returns 8 (number of type ids). The game-code callers of the wrapper

## 2. World to cell: `FUN_00a5eb50` (proven, all 0xb4 bytes decoded)

```
u = (x - (-16384.0f)) * 0.001953125f          tile_x = floor(u)    (cvttss2si, then -1 when the fraction is negative)
v = (z - (-16384.0f)) * 0.001953125f          tile_z = floor(v)
fx = frac(u) (+1 when negative), fz = frac(v)
cell_x = trunc(fx * 64), cell_z = trunc(fz * 64)                    (the code multiplies by -64.0f and negates)
byte = map[0x14 + ((tile_x & 7) + (tile_z & 7) * 8) * 0x100c + cell_z * 64 + cell_x]
```

* The constants are `0x0108ab34 = -16384.0`, `0x00f0f854 = 1/512`, `0x00f2008c = -64.0`. Only `x` and `z` of the position are used.
* **Cell size 8 m, tile 512 m, origin (-16384, -16384), axis order row = z, column = x.** The probe emulates the float32 arithmetic and reads the real tile files at 17 fixture positions.
* **No bounds check.** `x = 16384.0` gives tile 64, ring slot 0, cell 0; `x < -16384` gives tile -1, ring slot 7 (`-1 & 7`), cell 63. The sibling `FUN_00a5ea70` (callers `0x00a63669`, `0x00a636d9`)
  does compare the patch's stored tile numbers and returns a flag; `FUN_00a5eb50` (callers `0x00a66c52` = the light chain, `0x00a6ddd5` = the cloud object `FUN_00a6db30`) does not.
* **Independent check of the axes and alignment:** the type bits of a cell agree with the `gamezone.bin` zone of the four 4 m material samples under it (rows `2+2cz, 3+2cz`, columns `2+2cx, 3+2cx` of the
  132 x 132 grid) in 98.5 % of the cases where the four samples agree (64 tiles); the transposed reading gives 85.8 %.

## 3. Default and no-data bytes (proven for the two constants, gap for when each applies)

| Situation | Byte | Evidence |
|---|---|---|
| Map constructed, nothing streamed yet | `0x00` (type None, set 0) | `FUN_00a581a0` zero fill; tile numbers -100000 |
| Patch reset through the vtable | `0xE0` (Sea, intensity 3) | `FUN_00a56090` |
| Tile file read | the file's 4096 bytes | `FUN_00a560b0` |
| Position in a tile that is not resident (outside the 8 x 8 window or outside the world) | whatever the ring slot holds (not the asked tile) | `FUN_00a5eb50` has no check |

Every one of the 4096 tile files carries a full 4096-byte map, so a position inside the world never lacks data in the file set. For a Rust lookup use the file bytes inside the world and treat
`0xE0` as the fallback (non-native outside the world; the native value there is an unrelated ring slot).

## 4. The byte (proven by the accessors; interpretation inferred)

```
bit  7 6 | 5 4 3 | 2 1 0
     intensity | type | low3
```

* **type** selects the gradient set through `gradient_for_zone_NN` of `landscape_effects.bin` (identity: type N is set N), so a place's light set is simply `(b >> 3) & 7`; type 0 (None) uses set 0 and occurs in 241 cells only (byte 0).
* **intensity** is the zone depth: it is 0 on a cell touching another type and grows with the distance to the nearest cell of another type (probe: distance 1 gives only 0, distance 2 at most 1, distance 3 at most 2;
  cells far from any border use all four values). 3 is the interior value. The exact authoring rule is unknown (gap 3).
* **low3** is nonzero only when intensity is 0 (never 7) and takes the values 1..6 (`0x09, 0x0c, 0x12, 0x14, ...`). `gamezone.bin` lists eight spawn zones with ids Default 0, City 1, Industry 2, Tourist 3, Military 4,
  Village 5, Religious 6, Ice 7; the engine's combined id (`8 - 1 + low3` for `low3 >= 2`) suggests low3 is such a sub-zone. **The naming is not proven**: in the fixtures a military base icon sits on low3 = 4 (45x25) but
  another on low3 = 2 (14x50base). It does not matter for lighting (the light chain masks it out).

Cell counts by type x intensity (whole world):

| type | int 0 | int 1 | int 2 | int 3 |
|---|---|---|---|---|
| 0 None | 241 | 0 | 0 | 0 |
| 1 Arctic | 92,489 | 211,726 | 164,021 | 1,165,599 |
| 2 Desert | 158,119 | 307,697 | 219,247 | 777,553 |
| 3 Jungle | 933,260 | 680,710 | 465,577 | 1,540,534 |
| 4 Sea | 760,927 | 393,705 | 328,851 | 7,504,363 |
| 5 City | 47,485 | 31,714 | 19,249 | 20,951 |
| 6 Grass | 361,849 | 145,626 | 80,272 | 119,604 |
| 7 Default | 142,601 | 35,554 | 22,888 | 44,804 |

All 28 combinations of types 1..7 with intensities 0..3 occur, so every variant mix of section 6 is reachable.

## 5. Fixtures (probe, float32 lookup)

| Place | World (x, z) | Tile | Cell (x, z) | Byte | Type | Int | low3 |
|---|---|---|---|---|---|---|---|
| start base km01.base (task coordinates) | -1280.0, -6712.0 | 29,18 | 32, 57 | 0xC8 | 1 Arctic | 3 | 0 |
| km01 icon position (`settlements.bin`) | -1052.7, -7010.7 | 29,18 | 60, 19 | 0xC8 | 1 Arctic | 3 | 0 |
| research facility f2m07 | 3164.6, -3777.0 | 38,24 | 11, 39 | 0xC8 | 1 Arctic | 3 | 0 |
| military base 45x25 | 6884.9, -3508.6 | 45,25 | 28, 9 | 0x0C | 1 Arctic | 0 | 4 |
| desert base 13x42 | -9493.4, 5583.0 | 13,42 | 29, 57 | 0xD0 | 2 Desert | 3 | 0 |
| village 26x41 (Tanjung Rumah Api) | -2730.4, 4622.7 | 26,41 | 42, 1 | 0x50 | 2 Desert | 1 | 0 |
| jungle interior | -15092.0, -14524.0 | 2,3 | 33, 40 | 0xD8 | 3 Jungle | 3 | 0 |
| village 57x33 | 12970.3, 606.3 | 57,33 | 21, 11 | 0x18 | 3 Jungle | 0 | 0 |
| village 35x44 | 1589.6, 6726.6 | 35,45 | 6, 8 | 0x58 | 3 Jungle | 1 | 0 |
| open ocean south-west | -16000.0, -16000.0 | 0,0 | 48, 48 | 0xE0 | 4 Sea | 3 | 0 |
| village 47x59 | 8131.2, 14065.3 | 47,59 | 56, 30 | 0xA0 | 4 Sea | 2 | 0 |
| city.a | -15315.7, -2818.9 | 2,26 | 5, 31 | 0x68 | 5 City | 1 | 0 |
| city interior | -15364.0, -3092.0 | 1,25 | 63, 61 | 0xE8 | 5 City | 3 | 0 |
| village 24x50 | -3749.1, 9594.4 | 24,50 | 43, 47 | 0xF0 | 6 Grass | 3 | 0 |
| base 56x54 | 12473.3, 11692.6 | 56,54 | 23, 53 | 0xF8 | 7 Default | 3 | 0 |
| Panau International Airport | -6635.8, -3585.5 | 19,24 | 2, 63 | 0x3A | 7 Default | 0 | 2 |
| world corner | -16384.0, -16384.0 | 0,0 | 0, 0 | 0xE0 | 4 Sea | 3 | 0 |

The ring patch of each fixture, `(X & 7) + (Z & 7) * 8`, is in the probe's JSON output. The start base is Arctic at full intensity; Arctic set 1 is therefore the table to use for
the reference footage of the base.

## 6. The light chain around the byte: `FUN_00a66c30` (proven by bytes)

Inputs: camera position `pos`, `y = pos.y`. Locals: `Z` = gradient sub-object (JC2-115).

   `up = dh - Z[0x1aec]` (30) when positive else 0. If `dh > Z[0x1af0]` (300): `set = Z[0x1ad0]` (8) and `intensity = (int)(clamp((dh - 300) / (Z[0x1ae4] - 300), 0, 1) * 3)` (1000 - 300 = 700).
4. **y < 200.0:** `down = 200 - y` (always positive here), `set = Z[0x1ad4]` (9), `intensity = (int)(clamp(down / Z[0x1ae8], 0, 1) * 3)` (120). The zone byte is not used.
6. Then `FUN_00a5e010` (post-effect blocks, flag `Z[0x1ace]`), `FUN_00a55cd0` (cloud rows) and the day/night factor handed to `FUN_00954f00` (JC2-115 gap 8); none of these is part of this brief.

Values from `landscape_effects.bin` (probe, JC2-115): heights 30, 300, 1000; depth 120; sets 8, 9, 10, 11 for height, underwater, rain, snow.

## 7. What `FUN_00a5d390` does with them (inputs byte-anchored, arithmetic from pseudo-C)

Let `h = CRenderEngine+0xac` (hour), `i = floor(h)`, `f = h - i`, `lerp(v) = v[i] * (1 - f) + v[i+1] * f` (entry 24 = entry 0), tables `T[set][row][variant][hour]` (JC2-115 section 3). Six rows (0 sun,
1 colour 2, 2/3 clouds, 4 sky ambient `cb0[10]`, 5 ground ambient `cb0[11]`) are processed with the same arithmetic, in this order:

1. **Zone mix (always).** `wv = clamp(intensity * Z[0x1a80 + set*4], 0, 1)`; the weight array is `1 / (max_gradient_N & 15)` for the keys of `game_zones` (all 3, so **`wv = intensity / 3`** for sets 0..9; byte check of the
   multiply and clamp, loader arithmetic from pseudo-C). `row0 = wv * lerp(T[set][row][0]) + (1 - wv) * lerp(T[set][row][1])`: **variant 0 is the high-intensity table, variant 1 the low-intensity one** (byte-checked: wv
   multiplies the displacements of variant 0).
2. **Height block, only when `up > 0`.** `l = clamp(up / (300 - 30))` (up/270), `p = clamp(up / (1000 - 30))` (up/970),
   `s8 = p * lerp(T[8][row][0]) + (1 - p) * lerp(T[8][row][1])`, `row0 = l * s8 + (1 - l) * row0`. Because the two variants of set 8 are identical, `s8` is simply set 8. At `dh = 300` (up = 270) the zone
   colours are completely replaced by set 8; above that `FUN_00a66c30` also makes set 8 the base.
3. **Smoothing.** `state = alpha * row0 + (1 - alpha) * state` for all six rows (state at Z+0x146fc); `alpha = clamp(Z[0x1ac0] * dt)` (Z[0x1ac0] = 1.0 from the file), replaced by `clamp((1 / dt) * dt)` (= 1, a snap) when the previous call's set was the underwater set (`Z[0x1ac4] == Z[0x1ad4]`); `Z[0x1ac4]` is then overwritten with the current set.
4. **Output copy.** If `down <= 0`: `out = state`. Otherwise (**underwater block**) `d = clamp(down / 120)`, `out = d * lerp(T[9][row][0]) + (1 - d) * lerp(T[9][row][1])`: the smoothed state is *not* used (the original code adds
   `0.0 * state`, which has no effect).
5. **Rain block, only when `rain > 0`:** `r = rain`, `t = r * lerp(T[10][row][0]) + (1 - r) * lerp(T[10][row][1])`, `out = out + (t - out) * r`.
6. **Snow block, only when `snow > 0`** (after the rain block): the same with set 11 and `s = snow`.
7. **Sun transform** on `out[0]` only (JC2-115), then `FUN_0089f9d0` stores vectors 0, 1, 4, 5 into the sky object.

Byte anchors: the displacement dwords for the height, underwater, rain and snow set indices occur once each, in the order height `0x00a5d6a5` < underwater `0x00a5d91c` < rain `0x00a5dc02` < snow `0x00a5ddb2` < gain `0x00a5df16`; the three
gates (`up <= 0`, `down <= 0`, `rain <= 0`, `snow <= 0`) are `jbe` instructions on the arguments at `[ebp+0x10]`, `[ebp+0x14]`, `[ebp+0x18]`, `[ebp+0x1c]`; the base-row x component is decoded instruction by instruction.
The y/z/w components and the loops of the other blocks use the same displacement pattern (the loop bodies are pseudo-C-derived, not decoded one by one: gap 5).

**What intensity changes.** The variants of sets 0..8 and 10, 11 are identical except **set 2 row 4** (Desert sky ambient, max difference 0.0088), **set 3 row 5** (Jungle ground ambient, 0.1945, noon 0.73 against 0.54) and
**set 9** rows 0, 1, 4, 5 (underwater, where the intensity is the depth ratio). Everywhere else the intensity has no effect on the light colours (probe). Intensity also feeds `lerp(a, b, intensity / 3)` of the per-type precipitation
table (`FUN_00a63650`, used for the snow target), but that table has a = b = 1 for Arctic and 0 for the other types (`landscape_effects.bin`, container `precipitation`), so it is constant there.

**`Z[0x1a80 + set*4]` for sets 10 and up.** `game_zones` has only `max_gradient_0..9`; sets 10 and 11 are never passed as the `set` argument (the only values are the eight `gradient_for_zone` entries 0..7 and `Z[0x1ad0]`, `Z[0x1ad4]`, plus 0
from the loader), so those entries are never read. Rain and snow use explicit weights.

Example registers (probe, model output, steady state; sun colour after the transform): Arctic intensity 3, hour 0: sun 0.532 0.616 0.707, sky 0.048 0.114 0.170, ground 0.006 0.006 0.032; hour 12: sun 1.922 1.763 1.376, sky 0.000 0.021 0.070,
ground 0.883 0.941 0.867 (equal to the JC2-115 table). Height 1000 m above ground at noon gives set 8 (5.667 5.051 3.697); rain weight 1 at noon: sun 0.351 0.384 0.519, ground 0.006 0.006 0.006; snow weight 1 at noon: sun 3.284 2.932 2.146;
underwater 120 m at noon: sun 0.000 0.340 0.856, sky 0.018 0.334 0.359. The probe prints more (`--hours`).

## 8. Corrections to earlier documents

* `terrain.md` / `civilians.md`: "the tiles' own 8 m game zone byte map holds other values (16, 30, 54, 80, 88), probably the spawn-zone map, packed; not decoded" is resolved: bits 3..5 are the game-zone type, bits 6..7 the intensity,
  bits 0..2 a third field (16 = Desert, 30 = Jungle with low3 6, 54 = Grass with low3 6, 80 = Desert intensity 1, 88 = Jungle intensity 1).
* `night-light-next.md` gaps 1 and 3 and the first half of gap 2 are closed here; `light-tables-next.md` G3 ("source of the 8 x 8 x 64 x 64 zone map") likewise: it is the terrain tile tail, a ring of 8 x 8 tiles of 64 x 64 bytes.

## 9. Proven, inferred

* **Proven (probe, byte or data checked):** the file and layout of the map (all 4096 tiles), the whole of `FUN_00a5eb50` and its constants, the object layout and sizes, the constructor and reset values, the loader order, the accessors, the
  cell counts and sha256, the fixtures, the type ids and the type = set mapping, `FUN_00a66c30`'s input choice (branch conditions, thresholds from the file, argument order), the `wv` computation, the variant-0 pairing, the
  `up/270` and `up/970` weights' source displacements, the gates and block order of `FUN_00a5d390`, the variants that differ in the picture.
* **Inferred:** the meaning of `low3` and the authoring rule of intensity (empirical), the arithmetic of the height/underwater/rain/snow loops (pseudo-C), the weight array being `1 / (value & 15)` (pseudo-C of the loader; value 3 in the file),
  that `FUN_00a645a0` returns the terrain height in metres at the 4 m cell under the position, and that the camera position is what the manager passes (JC2-115).
* **Not modelled:** the float rounding of the evaluation order (Python doubles in the probe; the `1/dt * dt` snap can differ from 1 by one ulp, which the clamp hides).

## 10. Gaps (native details still unknown)

1. **Who resets patches to 0xE0 and when.** `FUN_00a56000` (slot 1 of `CTerrainPatchReader`, vtable `0x00f1fc08`) has no `call` site; the indirect caller in the tile streamer was not found, so it is unproven whether the
   camera's tile can ever hold 0xE0 or 0x00 while the player is inside the world (teleport, fast travel, tile streaming lag). Blocks: the lighting at streaming seams; the fallback for a not-yet-loaded tile.
2. **Streaming window.** Which tiles are resident (the ring is 8 x 8; the streamer's radius and the half-tile offset in `terrain.md`) and what `FUN_00a5eb50` returns for positions near the ring edge. Blocks: exact behaviour away from the camera
   (other callers, section 2).
3. **Authoring of intensity and low3.** The rule that produced the bytes (apparently distance to another zone, capped at 3, not exact) and the names of the low3 values; the consumers of `FUN_00a63640` (game code, audio
   and civilian logic) and of `FUN_00a636c0` (returns 0.9 when the zone type equals a manager field) were not decoded. Blocks: audio/civilian zone parity, not lighting.
5. **Loop arithmetic of `FUN_00a5d390`** for the y/z/w components and for the height, underwater, rain and snow blocks is read from pseudo-C (anchored by displacement counts and the gates, not decoded line by line). Blocks: bit-exact parity
   of the branch colours; the structure is certain.
6. **Other callers of `FUN_00a66c30`** (`0x00683a40`, `0x009000f3`) and the cutscene effect key `FUN_008961d0`: which position they pass and whether they bypass the zone byte (carried from JC2-115 gap 9).
7. **Consumers of rows 1, 2, 3 and of the precipitation table's second values** (`#60856e37`, `#c67ba7f8`): carried from JC2-115 gaps 4 and 5.

## 11. Bounded runtime handoff for JC2-122 (not implemented)

Parser: none needed beyond the existing `TerrainTile::parse` (it already returns `gamezone: Vec<u8>` of 4096 bytes, row = z). Add in `jc2-formats::terrain` (or a new `gamezone` module):

1. `pub struct GameZoneCell(pub u8)` with `zone_type() -> u8 { (b >> 3) & 7 }`, `intensity() -> u8 { b >> 6 }`, `low3() -> u8 { b & 7 }` and `pub const RESET_BYTE: u8 = 0xE0` (documented as the native reset fill).
2. `pub fn zone_cell(x: f32, z: f32) -> (i32, i32, usize, usize)` returning `(tile_x, tile_z, cell_x, cell_z)` with the **float32 sequence of section 2** (subtract `-16384.0`, multiply by `0.001953125`, truncate, subtract one and add one
   when the fraction is negative, multiply by 64, truncate). Note `TerrainTile::gamezone_local` divides by 8 and clamps; it is equal inside the world but not bit-identical at cell edges, so prefer the native sequence.
3. `pub struct GameZoneMap` over the tiles the world streamer already holds: `byte_at(x, z) -> u8` = the resident tile's `gamezone[cell_z * 64 + cell_x]` when tile `(X, Z)` is loaded and inside 0..64, else `RESET_BYTE`
   (**non-native** outside the world and for tiles not loaded; the native value is an unrelated ring slot). Do not model the 8 x 8 ring; indexing by tile number is equivalent whenever the tile is resident.
4. Tests (need `JC2_TEST_DIR`): the 17 fixtures of section 5 (the byte for each position), `zone_cell(-1280.0, -6712.0) == (29, 18, 32, 57)`, `zone_cell(16384.0, 0.0).0 == 64`, and the histogram total (16,777,216 cells; 7,504,363 cells of 0xE0).
5. Consumer in `jc2-game` (separate, after JC2-122 accepts the tables): `LightSetInputs { set, intensity, up, down, rain, snow }` produced by section 6 (`set = type`, the thresholds from `landscape_effects.bin`), and `blend()` of section 7. The first
   milestone is only `set = type, intensity, up = down = rain = snow = 0`, with the start base on Arctic (set 1, intensity 3). Keep the height/underwater/rain/snow blocks behind their inputs and label the dt smoothing non-native until gaps 4 and 5 close.
