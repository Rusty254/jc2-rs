# Terrain

Worked out from the data plus the functions below in the Ghidra project
(`research/ghidra/JustCause2.exe.gzf`, pseudo-C in `research/analysis/pseudo`) and, for
the materials, from the game's own terrain pixel shaders (`DX10_Shaders_F.shader_bundle`
in the game folder, disassembled with `research/tools/sm4dis.py`).
Implemented in `crates/jc2-formats/src/terrain.rs` (tiles), `landscape.rs` (materials)
and `dds.rs`; `jc2tool terrain` scans and dumps it; `crates/jc2-game/src/terrain_material.rs`
+ `terrain_splat.wgsl` draw it.

## World layout

| Thing | Value | Evidence |
|---|---|---|
| World extent | x, z ∈ [−16384, 16384) m, y up | constants `0x0108ab34 = -16384.0`, `0x0108ab38 = 16384.0`, `0x01089ee8 = -16384.0` |
| Tile grid | 64 × 64 tiles of 512 m; `X = floor((x + 16384) / 512)`, `Z` likewise from z | `FUN_00a5e990` (zone-map patch index: `(pos − (−16384)) × 0.001953125`, floored); the loaded-tile bitset in `FUN_00a3f7b0` is 64 wide |
| Tile file | `terrain\terrain_win32_%02d_%02d.dat` with `(X, Z)` | `FUN_00a3f640`: `sprintf("%s_%02d_%02d.dat", prefix, X, Z)`; 4096 files exist |
| Cells | 4 m; a patch holds 132 × 132 `u16` heights and a 132 × 132 byte material map, both with a 2-sample margin | `FUN_00a56b70` resets `0x4410` (= 132²) heights to 300 and materials to 0x11; `FUN_00a5f390` turns a position into `(frac(x/512)·128, frac(z/512)·128)` and `FUN_00a563a0` reads `materials[(row + 2)·132 + col + 2]` with the row from z and the column from x |
| Height scale | metres = raw × 2200 / 65535 | `FUN_00a5f210` multiplies the bilinear sample by `0x0108ab40 = 0.03356985`; `0x0108ab44 = 29.7886` is its inverse; the string `max_terrain_height` is read by `FUN_009b….` |
| Sea level | y = 200 m | `FUN_00a88ea0` tests `|y − 200| ≤ 5` before sampling the water surface (`CAtSeaLevelCondition`) |
| Global map | `terrain_win32.dat`: `"VER\0"`, u32 12, 512 × 512 `u16` at 64 m, then ~1 MB of undecoded records | `FUN_00a5f210`: `u = (x + 16384) / 64`, `v` from z, index `v·512 + u`, bilinear, 300 outside |

The terrain *renderer* (`FUN_00a49320`) keeps an 8 × 8 window of patches around the camera
indexed with a half-tile offset, `floor((pos + 256 + 16384) / 512)`; that is a render
cache convention, the data grid is the aligned one above.

## Tile file (`terrain_win32_XX_ZZ.dat`, 175–1500 KB)

A tile is a sequence of *stream patches*, each `u32 12` (version) followed by its
payload. `FUN_00a64830` reads the first one (one u32, `0x4410` u16 heights, `0x4410`
bytes); the texture patches are "size-prefixed DDS" and `CMapTextureStreamPatch` is one
more of those. All 4096 tiles have exactly this layout (`jc2tool terrain` with no
arguments prints the section layouts it saw):

```
u32   12            version
u32   123           (the first word of the game patch; always 123)
u16   [132 × 132]   heights, row-major, row = z, column = x, 4 m apart; the tile's own
                    128 cells are rows/columns 2..130, the rest is the neighbours' edge
u8    [132 × 132]   gameplay material per sample (same grid): the `Material` index into
                    climate\terrain_materials.bin — vegetation set, sounds, "is road"
u32   12            ─ texture patch ────────────────────────────────────────────────
u32   size, DDS     gradient map    A8L8     128 × 128, 8 mips   L = 128 + ∂h/∂x, A = 128 + ∂h/∂z,
                                                                 in raw height units per metre, clamped to a byte
u32   size, DDS     material slots  A4R4G4B4 128 × 128          four 4-bit meta-material slots per cell
u32   size, DDS     slot weights    A4R4G4B4 128 × 128          their weights, 0..15, summing to 15
u32   12            ─ map texture patch ────────────────────────────────────────────
u32   size, DDS     PDA map tile    DXT1     128 × 128          baked colours (lighting, roads, vegetation)
u32   12            ─ terrain mesh data (CTerrainDataStreamPatch, FUN_00a562c0): 21 pieces
                      (quadtree 1 + 4 + 16), each u32 bits, u32[bits / 32 + 1], 4 × u16 ─
u32   12            ─ game zone map: 64 × 64 bytes at 8 m (CGameZoneMapStreamPatch) ──
u32   12
```

Layer texel `(r, c)` sits on height sample `(r + 2, c + 2)`, i.e. on the vertex at local
`(4c, 4r)`; the renderer maps the patch's vertices to `v0.xz × cb2[2].z + cb2[2].xy` and
samples the layers there.

Checks that pin this down (`jc2tool terrain --seams`): every tile's columns 128..132
equal the +x neighbour's columns 0..4 and rows 128..132 equal the +z neighbour's rows
0..4 (0 mismatches over the game); the global map equals the tiles sampled every 64 m
(mean difference 0.02 m); and the ground-level objects of `31x14.nlz` (AI cover points,
spawn points) sit on the height field with a median error of 0.4 m once the right scale
and the 2-sample margin are used. The gradient map is the central difference of the
tile's own heights: mean |layer − (h[c+1] − h[c−1]) / 8 m| = 0.59 raw/m over all 4096
tiles (quantisation), with the x derivative in L and the z derivative in A; the
alternatives (forward differences, swapped channels, other scales) are off by 15–100.
The slot layer's dominant slot equals the gameplay byte map's material → `MetaMaterial`
→ `meta_index` in 99.5 % of all 67 M cells, and every slot any tile uses is defined in
walked on the five fixture tiles (31,14 31,15 32,14 38,12 40,11): every DDS header sits
where its size word says, the 21 mesh pieces end exactly at the zone map's version
word, and the zone map ends 4 bytes before the end of the file. The material map is
what the vegetation generator reads (`world.md`, "Vegetation"): on 31,14 the village
road is `0x52` (`road_arctic_asphalt`, no plants) and the slopes `0x40/0x41`
(`default_arctic`, the pine forest).

Sample positions: sample `(r, c)` of tile `(X, Z)` is at
`x = 512·X − 16384 + 4·(c − 2)`, `z = 512·Z − 16384 + 4·(r − 2)`.

## Materials

Three tables, all property containers under `climate\`, plus a few `textures\landscape\`
maps. `CTerrainMapGame`'s constructor (`FUN_00a74e40` → `FUN_00a740e0`) reads the first,
`CLandscapeManager` (`FUN_00a733b0`, then `FUN_00a70030`, `FUN_00a70fb0`) the rest.

**`terrain_materials.bin`** — 66 *gameplay* materials in a `materials` list, each with
`Material` (the byte-map index, 0..150 with gaps), `MaterialName`, `VegetationSet`,
`MetaMaterial` (render material id, below), `GameMaterial` (surface id for sounds and
particles), `Road`, `DefaultMaterialMask`, and the hashed keys `#884c9d87` (kind: 2
natural, 3 road, 0/1 flat man-made), `#ae498a0f` (priority 8/4/2/0), `#bd3ceeab`
(displacement slot 0..3), `#02bae781`/`#2ebeb969` (displacement amplitude and offset,
always `−amplitude/2`), `#7da08d72` (set on the arctic ground). Indices 0..39 are the
Jungle set (`default_jungle_rainforest`, `road_jungle_asphalt` = 18 → meta 5,
`paintable_global_asphalt` = 24 → meta 8 …), 64..84 Arctic (→ metas 20..23), 128..150
Desert (→ metas 30..34). The scan found byte-map values the table does not define
(40..63, 67..125, 142; ~0.2 % of cells) — they look like deleted paintable entries.

**`meta_materials.bin`** — 20 *render* materials (`materials`) and 4 climate `zones`.
A material has `global_index` (what `MetaMaterial` refers to), `meta_index` (the slot
0..15 the slot layer stores), `zone_index` (its climate zone) and `slope_slots`: 64
floats = `num_slope_slots` (16, root key) × `(f0, f1, f2, f3)`, one group per *slope
band*. A zone has `zone_index`, `zone_name` and `#787b04b5`, a float4 of the *texture
zones* it is drawn with (−1 = unused): Jungle 0 → [0, 1], Arctic 1 → [0], DesertCity 2
→ [1], City 3 → [1]. The engine builds one slot table per texture zone: every meta
material of every climate zone drawn with that atlas, at its `meta_index`
(`FUN_00a733b0` pushes each material into its climate zone's texture zones' lists,
`FUN_00a70fb0` writes row `texture_zone × 16 + meta_index` of the 16 × 128
`terrain_meta_material_uv_atlas` texture with the 16 bands as texels
`(f0, f1, f2 × 255, f3 × 255)`). So atlas 0 holds slots 0..10 (Jungle) + 11..14
(Arctic), atlas 1 slots 0..10 (Jungle) + 11..15 (DesertCity). The pixel shader (next
section) reads the four bytes as:

| byte | meaning | values in the data |
|---|---|---|
| `f0` | which 1024 px tile of `globaldisplaceatlas.dds` bends this band's normal (`frac(f0 × 63.75)` = `(f0 mod 4) / 4`) | 0..3 |
| `f1` | the atlas texture: index `i` is row `i / 8`, column `i % 8` of the 8 × 4 atlas | 0..31 |
| `f2` | how far the displacement normal bends the gradient before the band is re-chosen | 0.03..0.6 |
| `f3` | how far it bends the lighting normal (`× 1.5`) and darkens its hollows (`1 − 0.6·f3·(1 − n.z)/2`) | 0..0.6 |

The shipped tables (`jc2tool terrain X Z` prints a tile's):

| climate zone | slot | id | atlas texture per band 0..15 | displace tile | f2 | f3 |
|---|---|---|---|---|---|---|
| 0 Jungle | 0 | 0 | 11 8 5 27 7×2 6 27 25 14×2 13×2 12×2 26 | 2×8 3 2×2 3×2 1 2×2 | 0.25×2 0.30 0.40×5 0.20×6 0.25 0.30 | 0.40×8 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 1 | 1 | 15 9 11 27 24 15 25×3 14×2 13×2 12×2 26 | 1×8 3 2×2 3×2 1 2×2 | 0.20×14 0.25 0.30 | 0.30×10 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 2 | 2 | 1 0 1 2 0 26 12 13 25 14×2 13×2 12×2 26 | 3×9 2×2 3×2 1 2×2 | 0.10×8 0.20×6 0.25 0.30 | 0.07×5 0.10 0.15 0.20 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 3 | 3 | 29 3 28 4 27 11 15 25×2 14×2 13×2 12×2 26 | 0×8 3 2×2 3×2 1 2×2 | 0.20×14 0.25 0.30 | 0.15×7 0.20 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 4 | 4 | 6 4 8 28 27 11 15 25×2 14×2 13×2 12×2 26 | 0×8 3 2×2 3×2 1 2×2 | 0.20×14 0.25 0.30 | 0.20×7 0.25 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 5 | 5 | 10 9 11 15×2 25 15 14 25 14×2 13×2 12×2 26 | 3×3 2×5 3 2×2 3×2 1 2×2 | 0.15×3 0.20×11 0.25 0.30 | 0.03×8 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 6 | 6 | 28 4 27 9 11 15×2 25×2 14×2 13×2 12×2 26 | 1×8 3 2×2 3×2 1 2×2 | 0.05×2 0.10 0.15×2 0.20×9 0.25 0.30 | 0.10×8 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 7 | 7 | 1 2 29 3 27 15 25×3 14×2 13×2 12×2 26 | 3×5 0×3 3 2×2 3×2 1 2×2 | 0.20×14 0.25 0.30 | 0.15×7 0.20 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 8 | 8 | 30×3 11 15×4 25 14×2 13×2 12×2 26 | 3×9 2×2 3×2 1 2×2 | 0.10×8 0.20×6 0.25 0.30 | 0.05×8 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 9 | 9 | 30 31×7 25 14×2 13×2 12×2 26 | 3×9 2×2 3×2 1 2×2 | 0.10×8 0.20×6 0.25 0.30 | 0.05×8 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 0 | 10 | 10 | 9 23 11 23 11 15×3 25 14×2 13×2 12×2 26 | 3×9 2×2 3×2 1 2×2 | 0.10×8 0.20×6 0.25 0.30 | 0.05×8 0.30×2 0.40×2 0.50 0.40 0.50 0.60 |
| 1 Arctic | 11 | 20 | 16×2 17 18 17 19 15 25×2 14×2 13×2 12×2 26 | 1×6 2×2 3 2×2 3×2 1 2×2 | 0.07×2 0.10×2 0.13 0.15×3 0.20×6 0.25 0.30 | 0.00×2 0.08 0.15 0.20×2 0.30×4 0.40×2 0.50 0.40 0.50 0.60 |
| 1 | 12 | 21 | 9×2 21 20 22 19 15 25×2 14×2 13×2 12×2 26 | 3×6 2×2 3 2×2 3×2 1 2×2 | 0.15×8 0.20×6 0.25 0.30 | 0.00×2 0.07×4 0.30×4 0.40×2 0.50 0.40 0.50 0.60 |
| 1 | 13 | 22 | 19×2 22 18 17 19 15 25×2 14×2 13×2 12×2 26 | 1×6 2×2 3 2×2 3×2 1 2×2 | 0.15×8 0.20×6 0.25 0.30 | 0.05×2 0.07 0.10×3 0.30×4 0.40×2 0.50 0.40 0.50 0.60 |
| 1 | 14 | 23 | 22×2 20 21 14 13 15 25×2 14×2 13×2 12×2 26 | 3×4 2×4 3 2×2 3×2 1 2×2 | 0.15×8 0.20×6 0.25 0.30 | 0.05×4 0.20×2 0.30×4 0.40×2 0.50 0.40 0.50 0.60 |
| 2 DesertCity | 11 | 30 | 0 17 1 2 18 19×2 20×2 21 22 21 22 21 22×2 | 2×7 3 2×2 3×2 1 2×3 | 0.10×4 0.15 0.20 0.30×6 0.40 0.50 0.60×2 | 0.05×4 0.10 0.20 0.30×3 0.40×2 0.50 0.40 0.50 0.60×2 |
| 2 | 12 | 31 | 2 29 1 2 18 19×2 20×2 21 22 21 22 21 22×2 | 2×7 3 2×2 3×2 1 2×3 | 0.10×4 0.15 0.20 0.30×6 0.40 0.50 0.60×2 | 0.05×4 0.10 0.20 0.30×3 0.40×2 0.50 0.40 0.50 0.60×2 |
| 2 | 13 | 32 | 2 29 3 28 29 19×2 20×2 21 22 21 22 21 22×2 | 2×7 3 2×2 3×2 1 2×3 | 0.10×4 0.15 0.20 0.30×6 0.40 0.50 0.60×2 | 0.05×4 0.10 0.20 0.30×3 0.40×2 0.50 0.40 0.50 0.60×2 |
| 2 | 14 | 33 | 18 10 11 15 22 15 22 20×2 21 22 21 22 21 22×2 | 2×7 3 2×2 3×2 1 2×3 | 0.15×5 0.20 0.30×6 0.40 0.50 0.60×2 | 0.20×4 0.30×5 0.40×2 0.50 0.40 0.50 0.60×2 |
| 2 | 15 | 34 | 11 18 1 2 18 19×2 20×2 21 22 21 22 21 22×2 | 3×4 2×3 3 2×2 3×2 1 2×3 | 0.10×4 0.15 0.20 0.30×6 0.40 0.50 0.60×2 | 0.05×4 0.10 0.20 0.30×3 0.40×2 0.50 0.40 0.50 0.60×2 |

(Textures 12..15 and 25..26 are the shared cliff rock that every material turns into
on steep ground; 16..22 are snow in atlas 0 and desert sand/rock in atlas 1; 30/31
asphalt and concrete; 4, 7, 24, 27 grass and jungle floor.)

**Maps and textures** (`textures\landscape\`, copies of the zone maps also in
`general.blz`; `FUN_00a70030` loads them as `Landscape\...`):

| File | Format | Meaning |
|---|---|---|
| `texturezonemap.dds` | 64 × 64 `A8R8G8B8`, value in the red byte | texture zone (atlas) per tile, row = Z; 0 or 1 |
| `climatezonemap.dds` | same | climate zone per tile, 0..3 (`FUN_00a555a0` reads a packed 4-bit copy) |
| `climate\zone_N\diffuseatlas_N_dif.dds` | 2048 × 1024 DXT1, 8 × 4 textures of 256 px | the 32 terrain textures of texture zone N (`FUN_00a69370` formats the name; the archive keys it by the bare file name) |
| `climate\zone_N\normalatlas_N.dds` | same | their normal maps (shader scales the decoded xy by 4) |
| `globaldisplaceatlas.dds` | 4096 × 1024 DXT5, 4 tiles of 1024 px | normal-map "displacement" detail, each tile repeating every 256 m |
| `globalcolorpixelmap.dds` | 32 × 2 `A8R8G8B8` | mean colour of every atlas texture, row = texture zone; it is what the engine copies into its 16 × 128 colour table (the second buffer `FUN_00a70fb0` fills next to the uv atlas) and what the far shaders draw. Matches the atlas means to within ±0.1 (slightly desaturated) |
| `globalmetamaterials.dds`, `globalmetaweights.dds`, `globalslopes.dds`, `data.dds` | 1024², 512², 512², 512² 16-bit | the global (whole-world) slot, weight and gradient layers for the far LODs |
| `rtdisplacement_N.raw` | 65536 bytes | sources of the displacement tiles (`FUN_00a5f450`) |
| `default_material/weights/normals/map.dds` | tiny | what a patch shows before its data arrives |

## The terrain shader

`FUN_00a573f0` (the terrain renderer's shader setup) fetches the
pixel shaders `FastTer_<Lod><Shadow>` for Lod ∈ {Lod0, Lod01, Lod1, Lod12, Lod2} and
Shadow ∈ {ShNone, Sh0, Sh01, Sh012, Sh1, Sh12, Sh2} (cascade combinations), the vertex
shaders `FastTer_<Shadow>`, and `FastTerrainShader_Global/_Blend/_Reflection` and
`TerrainShadowSimple`. They are in `DX10_Shaders_F.shader_bundle` as stripped DXBC
(no constant names); `research/tools/sm4dis.py` reads them. Everything below is from
`fastter_lod0shnone` (the near terrain) unless said otherwise.

**Vertex shader** (`fastter_shnone`): the vertex is `(x, h0, z, h1)` in patch space;
`r0 = v0 × cb2[0].xyxy + cb2[1]` scales/offsets it, the height is geomorphed between
`h0` and `h1` by `saturate(max(|x|, |z|) × cb2[0].z + cb2[0].w)`, and the position goes
through the camera-relative view-projection `cb1`. Outputs: `TEXCOORD0.xy = v0.xz ×
cb2[2].z + cb2[2].xy` (patch UV into the tile layers), `TEXCOORD0.zw = world.xz / 32768
+ 0.5` (UV into the global maps), `TEXCOORD1.xyz` = world position, `.w` fog,
`TEXCOORD2` water/underwater terms, `TEXCOORD4..6` three shadow-space positions.

**Pixel shader, `FastTer_Lod0`**, with the texture slots as the shader numbers them
(the engine binds them by number; the names are ours):

```
t7  texture zone map (64 × 64)      t5  patch gradient map (A8L8)      t3  patch weights
t4  patch slots                     t6  terrain_meta_material_uv_atlas  t2  globaldisplaceatlas
t8  normal atlas                    t9  diffuse atlas                   t10 light grid (int)
t11 cloud shadow / AO map at the global UV
```

```
zone  = round(t7(globalUV).x × 255)
g     = t5(patchUV).xw × 2 − 1                       // gradient / 4.26337  (4.26337 = 127 × 2200/65535)
w     = t3(patchUV);  w.a = 1 − saturate(w.r + w.g + w.b)
slot  = round(t4(patchUV) × 15)                      // four slots (R, G, B, A nibbles)
w     = w < 0.5 ? w / 8 : w × 1.875 − 0.875          // contrast curve
(A, B) = the two heaviest slots; wA, wB = their weights / (wA + wB)
rowA, rowB = zone × 16 + slotA/B
s     = min(|g|, 1) × 36;  i = floor(s);  f = frac(s)        // slope band and fraction
cand  = {(A, i, wA(1−f)), (B, i, wB(1−f)), (A, i+1, wA f), (B, i+1, wB f)}
pick the two heaviest candidates, normalise                   // "first pass"
uvA  = t6(bandA / 15.5, rowA / 127.5), uvB likewise           // bytes f0, f1, f2, f3
f2   = uvA.z wA + uvB.z wB;  f3 = uvA.w wA + uvB.w wB
d    = blend of t2 tiles (f0A mod 4, f0B mod 4) sampled at frac(world.xz / 256) with mip
       clamp(log2|∂world.xz| + 0.5, 0, 6), decoded xyw × 2 − 1
g'   = −g + f2 × (d.x, −d.y)                                  // bend the gradient
repeat the band selection with |g'|                           // "second pass" → two (slot, band) pairs
texA = uv(bandA', rowA').y × 255, texB likewise               // atlas texture indices
uv   = frac(world.xz × 0.328125) → column texA % 8, row texA / 8 of the 8 × 4 atlas,
       inset by 1/384 of a tile per mip level, mip clamp(log2|∂world.xz| + 5.5, 0, 6)
albedo = lerp(t9(texA), t9(texB), wB)
n_t  = (t8(texA).xy, t8(texB).xy blended) × 2 − 1, × 4; z = sqrt(1 − |xy|²)
N    = normalize of the slope normal (g'' = −g + 1.5 f3 d, N0 = (g''.x, 1, g''.y) / √(1+|g''|²)) with n_t in its tangent frame
light = saturate(dot(N, −sun cb0[4])) × cloud shadow (t11, cb0[0]) × hemisphere;
        ambient lerp(cb0[11], cb0[10], N.y/2+0.5) × ao, ao = 1 − 0.6 f3 (1 − d.z)/2;
        up to 4 point lights from the t10 grid (cb0[15+i] pos/radius, cb0[80+i] colour)
colour = (sun cb0[3] × light + ambient + points) × albedo, fogged towards cb0[13] by exp(−fog),
         tinted cb0[14] under water
```

What that means for us:

* **Tiling**: every texture repeats every `1 / 0.328125 = 3.05 m` in world x/z, no
  per-material scale. `FastTer_Lod1` uses `× 0.046875` instead (21.3 m, exactly 7×
  coarser) and drops the normal atlas; `Lod01` blends the two; `Lod2` (= the
  `_Global`/`_Blend` shaders) draws no atlas at all, only the colour table
  (`globalcolorpixelmap`) plus the displacement normal. So distant terrain is flat
  colour, which is also why the PDA map correlates with the mean colours.
* **Slope bands**: the measure is `|∇h| / 4.26337 × 36` with `|∇h|` the true gradient
  (tan of the slope angle), clamped so bands past 15 read column 15 (the uv atlas is 16
  wide; we assume a clamping sampler). Band k starts at `atan(k × 0.1184)`: 6.8°, 13°,
  20°, 25°, 31°, 35°, 40°, 43°, 47°, 50°, 52°, 55°, 57°, 59°, 61°. Adjacent bands blend
  by the fraction.
* **Weights**: the stored four nibbles sum to 15; the shader's contrast curve means a
  cell at 10/15 shows ~88 % of its first material, and only near 8/7 do two show.
  At most two (slot, band) pairs are drawn per pixel.
* The gradient comes from the A8L8 layer (bilinear), not from the mesh normal; the
  displacement atlas perturbs it (and the lighting normal) with f2/f3 (next section).

### The displacement atlas, as drawn here

`textures\landscape\globaldisplaceatlas.dds`: 4096 × 1024 DXT5 with 13 mips, four 1024 px
tiles side by side (a pale relief map of eroded ridges: tile 0 sparse and sharp, 1 and 3
rough, 2 big plates and gullies), each repeating every 256 m. Decoded as the shader does:
`d = (r, g, a) × 2 − 1` (x, y the tangent-plane tilt, a third channel the relief height, +1
high). Used as in `FastTer_Lod0` (`terrain_splat.wgsl`, `pick_layers`, `displacement`):

1. the first selection of slot × band (from the gradient map, below) names, through the
   table's `f0` / `f2` / `f3` bytes (packed into the uniform table next to the texture index,
   `terrain_material::pack_entry`: bits 8..9 the displacement tile, 17..24 `f2 × 255`,
   25..31 `f3 × 200`), the tile(s) to sample (blend of two by the two layers' weights) and the
   strengths `f2`, `f3` (both from this **first** pass);
2. the gradient is bent, `g′ = −g + f2 · (d.x, −d.y)`, and the band selection is repeated
   with `|g′|`: this is what makes the snow/rock boundary ragged and breaks the smooth
   contour-following blobs of the first pass;
3. the lighting normal is built from `G = 4.26337 (−g + 1.5 f3 (d.x, −d.y))`,
   `N0 = (G.x, 1, G.y) / √(1 + |G|²)`, then bent by the normal atlas; the hollows term
   `ao = 1 − 0.6 f3 (1 − d.z) / 2` multiplies **only the ambient** term (next section).
   There is no extra scale on the displacement in the normal: the full `1.5 f3` figure is
   the game's (the old `DETAIL_LIGHT = 0.5` was a by-eye guess and is gone).

There is no distance fade of the displacement in any `FastTer_*` shader; the far LODs use
it too, and only its mip (clamped at 6) softens it. The texture is bound once for all tiles
(`TerrainSplat::displace`, binding 108/109, one BC3 image with the whole chain).
`JC2_NO_DISPLACE=1` draws without it, for comparison.

### LOD blend distances, mips and lighting terms (JC2-084, native-proven)

Producers and units, all in the unmodded `JustCause2.exe` (pseudo-C addresses) and the
`DX10_Shaders_F.shader_bundle` blobs (`sm4dis.py`):

| Constant | Producer | Value / units | Consumer |
|---|---|---|---|
| renderer `+0x4b0` w0 | `FUN_00a60d30` (CTerrainRenderBlockType ctor), `[300] = 0x41c80000` | 25.0 m, Lod0→Lod1 blend width | VS `cb1[4].y = 1/w0` |
| renderer `+0x4b4` R0 | same, `[0x12d] = 0x41c80000` | 25.0 m, Lod0 radius before FOV scale | `r0 = R0·k` |
| renderer `+0x4b8` w1 | same, `[0x12e] = 0x42480000` | 50.0 m, Lod1→Lod2 blend width | VS `cb1[4].w = 1/w1` |
| renderer `+0x4bc` R1 | same, `[0x12f] = 0x437a0000` | 250.0 m, Lod1 radius before FOV scale | `r1 = R1·k` |
| boxes `+0x3c8`, `+0x428` | `FUN_00a61220` | camera ± (r0 + w0), camera ± (r1 + w1) (axis-aligned cubes) | per-patch shader choice in `FUN_00a579d0` (`FastTer_Lod0/01/1/12/2` via `+0x300 + (lod·7 + shadow)·4`) |

The vertex shader `fastter_sh*` computes, from the camera-relative vertex `p`
(`r0 = v0·cb2[0] + cb2[1]`, height minus camera height), the **Chebyshev** distance
`d = max(|p.x|, |p.y|, |p.z|)` and outputs `t0 = 1 − saturate((d − r0)/w0)` (`TEXCOORD2.z`)
and `t1 = 1 − saturate((d − r1)/w1)` (`.w`). The pixel shaders then draw:

* `Lod0`: atlas at `× 0.328125` with the normal atlas (`× 4`);
* `Lod01`: `lerp(Lod1 colour, Lod0 colour, t0)`, normal atlas strength `4 t0`;
* `Lod1`: atlas at `× 0.046875`, no normal atlas;
* `Lod12`: `lerp(Lod2 colour, Lod1 colour, t1)`;
* `Lod2`: no atlas; colour = `t1` "MetaMaterialColors" (16 × 128, built by `FUN_00a70fb0`:
  row `zone·16 + slot`, column band, texel = `globalcolorpixelmap.dds[zone][texture]`, i.e.
  `Landscape::slot_colour`) sampled at the second-pass (band, row) pairs and weights.

So at the default 65° the near atlas is pure within 19.4 m (Chebyshev), gone by 44.4 m; the
21 m atlas holds to 194 m and is replaced by the colour table by 244 m. With Bevy's 45°
default camera, k = 1.621: 40.5 / 65.5 m and 405 / 455 m.

Explicit mips (`sample_l`), with `D = ½ log2(|∂x/∂X|² + |∂z/∂X|² + |∂x/∂Y|² + |∂z/∂Y|²)` of
the world position (`deriv_rtx/rty` of `TEXCOORD1.xz`, one `dp4`):

* displacement atlas: `clamp(D + 0.5, 0, 6)`, tile UV inset by one texel of that mip
  (`u = tile/4 + 2^m/4096 + ¼ frac(x/256)(1 − 2^m/512)`, `v = 2^m/1024 + frac(z/256)(1 − 2^m/512)`);
* Lod0 atlas (diffuse and normal): `clamp(D + 5.5, 0, 6)`; Lod1 atlas `clamp(D + 2.8, 0, 6)`
  (2.8 ≈ log2 7, the repeat ratio).

The gradient `g` is `t5(patchUV).xw × 2 − 1` (A8L8 L, A; bilinear), the slope measure
`min(|g|, 1) × 36`; the mesh normal is not used. The tangent `z` of the normal atlas is
`sqrt(|1 − |4n.xy|²|)` (absolute value, not a clamp).

Lighting, `cb0` = pixel-shader buffer 0 (`FastTer_Lod0ShNone`, `cb0[145]`):

```
shadow = max(saturate(cb0[0].z − dot(t11.wy, cb0[0].xy)), cb0[10].w)     // cloud shadow
sun    = cb0[3] × saturate(dot(N, −cb0[4])) × shadow × (t11.x/2 + 1/2)
amb    = lerp(cb0[11], cb0[10], saturate(N.y/2 + 1/2)) × t11.x × ao       // hemisphere
points = Σ≤4 (t10 grid) cb0[80+i] × saturate(dot(N, L/|L|)) × saturate(1 − |L|² cb0[15+i].w)²
colour = (sun + amb + points) × albedo, fog lerp to cb0[13] by exp(−fog), cb0[14] under water
```

`ao` multiplies the ambient only; the sun term is plain Lambert (no specular). The
producers of `cb0[3]` (sun colour), `cb0[10]`/`cb0[11]` (sky/ground ambient), `cb0[0]` and
`t11` were **not** traced (the per-hour light tables are unrecovered, `weather.md`; **correction, JC2-115:** `cb0[3]`, `cb0[10]`, `cb0[11]` are
written every frame from `gamezone_gradients.bmp`, `night-light-next.md` section 2), so the
runtime keeps Bevy's lights (`apply_pbr_lighting`, roughness 1, metallic 0) and applies the
native terms it can prove: `ao` as diffuse (ambient-only) occlusion, the native normal and
the native albedo.

**Runtime** (`terrain_material.rs`, `terrain_splat.wgsl`): the fragment shader computes
`t0`/`t1` per pixel (the game interpolates them per vertex) from the camera's real vertical
FOV (`clip_from_view`), so Bevy's 45° camera gets 40.5/455 m instead of the game camera's
19.4/244 m until the camera owner adopts the 65° FOV. Lod1 colour, Lod0 colour and the
colour table are mixed exactly as Lod01/Lod12 do; atlas and displacement use the explicit
mips; the gradient comes from the A8L8 layer (`gradient_image`, binding 110/111, the file's
mip chain); the tables' colours ride in the uniform (`colour_table`). Not reproduced: the
atlas texel inset (our atlas is a texture array, nothing to bleed), the per-patch shader
switch (the blend weights cover it), the fog/underwater terms (Bevy's fog). Tests:
`lod_blend_matches_the_native_boundaries`, `native_mips_and_displacement_inset`,
`shader_mirrors_the_native_constants`, `original_climate_fixtures` (Jungle 4,2, Arctic
23,6, DesertCity 12,36: the gradient texture equals the file's layer and decodes to the
heights' central difference within 0.03 m/m; every used slot has its table colour).

GPU comparison (2026-10-09, `--world X,Z --no-hero --no-models --no-vegetation --no-grass
--no-clouds --time 12:00 --day-speed 0 --exit-after 30 --screenshot`, the same start camera
for the old and new binaries, 31,14 / 12,36 / 40,40): far rock now reads as the flat
table colour shaped by the displacement lighting, the way the original's distant
mountains do (`reference/world/daylight-start-base-snow-mountains-from-freefall_bn05`),
instead of the 21 m atlas pattern; rock relief is stronger at the full `1.5 f3`; snow, sand
and grass placement is unchanged. The reference frames have no recorded camera, so this
is a like-for-like check of our two builds plus a visual check against the original, not
a pixel match.

`crates/jc2-formats/src/landscape.rs` has `slope_band`, `weight_curve`, `blend_layers`
(the two-pass selection without the displacement) and the constants;
`terrain_splat.wgsl` is the same selection on the GPU with Bevy's lighting, the
LOD 0/1/2 blend and mips above, and the normal atlas bend; the vertex-colour
fallback and `jc2tool terrain --materials` use `Landscape::cell_colour` (the same
selection with the mean colours).

Verification of the whole chain: `jc2tool terrain X Z --materials m.ppm --map pda.ppm`
and the printed luminance correlation between the two — 0.65 / 0.66 / 0.55 / 0.74 /
0.34 for the fixture tiles 31,14 / 31,15 / 32,14 / 38,12 / 40,11 (the PDA map has
lighting, roads and vegetation baked in; the mean over all 4096 tiles is 0.34 because
sea-floor tiles are drawn as water in the map). Rock ridges, snow fields and roads land
where the map has them.

## The distant terrain (in jc2-game)

`horizon.rs` draws the global map beyond the streamed tiles: 1 km chunks (16 × 16 of its
64 m cells, 32 × 32 chunks over the world), streamed out to 14 km around the focus (the
fog's visibility is 12 km), at 64 m within 5 km and 128 m beyond. Of the 1024 chunks,
798 have some ground above the sea; the rest are never built. A chunk is hidden once its
four tiles are loaded in detail; otherwise it sits up to 18 m low (fading to 0 at the
shore, so the coastline does not move) so that a detailed neighbour draws over the
seam. The sea plane reaches past the fog. Nothing of the global file's tail is used.

## Still unknown

* Which `rtdisplacement_N.raw` made which atlas tile (the atlas itself is used, above).
* The producers of the pixel `cb0` lighting registers (`cb0[0]`, `[3]`, `[10]`, `[11]`,
  `[13]`, `[14]`; `[3]`, `[10]`, `[11]` are resolved in `night-light-next.md`) and of `t11`; sampler filter of `s1`/`s6` (point is implied for `s6`,
  which reads indices; `s1` shares its coordinates). `cb2[2]` is `FUN_00a579d0`'s
* The Lod2 hollows term (`1 − 0.6 f3 · r3.z`) was not decoded register by register; the
  runtime uses the Lod0 form at every distance.
* What the mesh-data bits say (per piece 60–3600 bits; the owner is the terrain mesh
  LOD code, so probably a triangulation/hole mask).
* The global file's tail (~1 MB after its height map).
* Water: the surface is a displaced plane at 200 m (`Landscape\RTDisplacement_%d.raw`
  in `FUN_00a5f450`); shoreline/foam data unknown.
