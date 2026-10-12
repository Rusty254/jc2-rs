# Terrain shader remaining terms (JC2-101)

*9 October 2026. Research only; no runtime change. Probe: `research/tools/terrain_terms_probe.py`
(60 checks, exit 0). Native addresses are retail `JustCause2.exe` VAs (sha256 `df99a479...b74f80`);
shader names are in `DX10_Shaders_F.shader_bundle` (sha256 `db8c67a0...ebc292e98`).*

Questions: (1) why distant hillsides render pale yellow-green in jc2-rs while the original shows dark
green slopes; (2) the colour-table sampler and colour space, the far-distance hollows term, per-vertex
versus per-pixel blend weights; (3) `FUN_004dcb70`.

## Answer to question 1 (priority)

**None of the `terrain_splat.wgsl` terms causes it. The pale hillsides are not drawn by that shader.**
Everything past the streamed tiles is `horizon.rs`, which paints its chunks with the invented
`world::terrain_colour(height, slope)` palette (vertex colours on a plain `StandardMaterial`). The
native far terrain is drawn by `FastTerrainShader_Global`, which is the Lod2 maths (colour table, not
atlas) evaluated on the global layers. Fog is not the cause either.

Evidence:

* **Same camera, same fog, tiles instead of chunks.** Capture A is the failing case, B differs only in
  `--radius 4` (81 tiles instead of 9), so the same hills are drawn by the splat shader's Lod2 branch:

  ```
  JC2_CAM_HEIGHT=311 JC2_CAM_PITCH=5 jc2.exe --world 40,40 [--radius 4] --no-hero --day-speed 0 \
      --time 12:00 --exit-after 40 --screenshot out.png      # unrebuilt target/release/jc2.exe
  ```

  | 100x100 box (1280x720 frame) | radius 1 (horizon chunks) | radius 4 (splat Lod2) |
  |---|---|---|
  | hill-left (100,450) | (235,237,210) | (119,116,122) |
  | hill-mid (600,560) | (182,207,151) | (102,110,112) |
  | hill-right-low (900,560) | (213,219,189) | (115,114,120) |

  The yellow-green cast, the clipped 255s of `_extract/jc2-140/after_jungle_high.png` and the pale
  tone vanish as soon as the splat shader draws the same ground; the fog is identical in both. The far
  snow summit stays white in B because it is still a horizon chunk.
* **The palette is not native and is 6x too bright.** `terrain_colour` (`world.rs`) says it is
  by-eye ("keep these in the range real ground sits in"): grass (0.16, 0.26, 0.09), dry (0.36, 0.31,
  0.18), rock (0.30, 0.28, 0.26), used as *linear* albedo. The native colour of the same ground
  (global layers + `globalcolorpixelmap`, sRGB-decoded, first pass, 2500 samples over the jungle
  highlands north of tile 40,40) has median linear luminance 0.038 (p95 0.079; mean RGB 0.060, 0.048,
  0.029) against 0.226 for the palette's darkest land colour: 5.9x. Lit by the same sun a 0.26 green
  clips; a 0.04 olive does not. `horizon.rs` never reads a global material layer.
* **Native far terrain is the Lod2 maths on global layers.** `FastTerrainShader_Global` PS is
  instruction-identical to `FastTer_Lod2ShNone` from the lighting stage on (34 lines) and shares its
  displacement / band re-selection / colour-table section; only the input layers differ (below). The
  per-patch draw `FUN_00a579d0` picks PS `+0x390` (`FastTerrainShader_Blend`) when the patch lod byte
  (`patch+0x61 & 0x7f`) is 9 and PS `+0x38c` (`FastTerrainShader_Global`) above 9, with VS `+0x394`
  (`FastTerrainShader_Global`), and `FastTer_Lod*` below 9 (decompile of `FUN_00a579d0`; the byte
  loads of `+0x38c/+0x390/+0x394` are in the probe).
* **Fog is not the cause.** Capture B has the same fog function on the same hills and shows them dark.
  (The fog *colour* stays JC2-116's.)
* LOD-dependent blend: not involved (the palette chunks have no LOD blend, tile Lod2 is correct).
  Colour table filtering / colour space: correct in the runtime (below).

What the fix will *not* give you: after the palette is replaced, far slopes read brown/grey olive (capture
B), not the reference's dark green forest. The native game draws a second terrain layer, the forest
canopy (`TerrainShaderForest` / `TerrainShaderForestFin`, `forest_dif.dds`, `forestfin_dif.dds`),
that is not in jc2-rs at all (Gap 1). Its texture is olive green (alpha-weighted sRGB 0.208, 0.248,
0.093, probe) and is drawn as an opaque alpha-tested cutout (no blending, reference 0.5; forest-canopy-next.md, JC2-160) over the far ground, so it is the most likely source of the
reference's green slopes. Far tree impostors and the fog/atmosphere colour add to that.

## Question 2

### Colour table: sampler and colour space (native-proven)

`globalcolorpixelmap.dds` (32 x 2, `A8R8G8B8`) is never sampled. `FUN_00a70fb0` copies its texels
(`row = texture zone`, `column = atlas texture`) into the 16 x 128 `MetaMaterialColors` texture, row
`zone*16 + slot`, column band, and the Lod2/Global PS samples it as `t1` with sampler `s1`.

* **Filter.** The per-frame setup `FUN_00a570d0` loops over terrain sampler slots 0..7 and calls
  `ebp`-relative stores (probe reconstructs it from the bytes):

  | slot | texture | (A, B) | D3D10 filter | address |
  |---|---|---|---|---|
  | s0 | global slopes (Global PS) | 3, 2 | `0x15` MIN_MAG_MIP_LINEAR | 3 = CLAMP |
  | **s1** | **colour table (t1)** | **1, 1** | **`0x00` MIN_MAG_MIP_POINT** | CLAMP |
  | s2 | displacement atlas | 3, 2 | linear | CLAMP |
  | s3 | patch weights | 3, 2 | linear | CLAMP |
  | s4 | patch slots | 1, 1 | point | CLAMP |
  | s5 | patch gradient | 3, 2 | linear | CLAMP |
  | s6 | uv atlas | 1, 1 | point | CLAMP |
  | s7 | texture zone map | 1, 1 | point | CLAMP |

  `FUN_00940170` turns the packed state into `D3D10_SAMPLER_DESC`: `A = flags >> 13 & 0xf`,
  `B = flags >> 17 & 0xf`, filter = `((X[A] << 2 | X[B]) << 2) | Y[A]` from the table at `0xf22858`
  (v1 = (0,0), v2 = (1,0), v3 = (1,1)), address = `flags & 7`, anisotropy `flags >> 21 & 0x1f` (set to 1,
  i.e. off). The shader's `(band / 15.5, row / 127.5)` coordinates land on texel `(band, row)` for every
  band 0..15 and row 0..127 under point filtering (probe), so the colour table is an exact per-entry
  lookup: **terrain_material.rs' uniform `colours` table is equivalent**. For the global path
  (`FUN_00a579d0`) the two layers bound to slots 8/9 are weights (linear) and material slots (point).
* **Colour space.** The `MetaMaterialColors` create call in `FUN_00a70fb0` (`0xa71511`) passes format
  straight into the `D3D10_TEXTURE2D_DESC` (its depth-format tests 0x14/0x28/0x2d/0x37 are the DXGI
  numbers). So the table is **`R8G8B8A8_UNORM_SRGB` (29)**: the hardware linearises each texel before
  the row mix, as `table_colour` (`srgb_to_linear`, then the weighted sum) does. The uv atlas next to
  it is created with flags 0 (UNORM, 28).

### The far-distance hollows term (native-proven, closes the old gap)

`FastTer_Lod2*` and `FastTerrainShader_Global` compute `ao = 1 - 0.6 * f3 * (0.5 - 0.5 * d.z)`, with `f3`
and `d` blended from the same two rows as in `FastTer_Lod0` (probe matches the three instruction
chains, registers differ). `ao` multiplies the hemisphere ambient only (`ao * t11.x`), the sun term is
`cb0[3] * saturate(dot(N, -cb0[4])) * shadow` without it. This is exactly the Lod0 form the runtime
uses at every distance, so `terrain_splat.wgsl` needs **no change** for it. The lighting normal also
uses the same `1.5 * f3` displacement bend and `4.26337` gradient scale in all three shaders.
`terrain.md` "Still unknown" item on the Lod2 hollows term can be struck.

### Global layer formats (correction to `terrain.md`)

`globalmetaweights.dds`, `globalslopes.dds`, `data.dds` are **1024 x 1024 BC1/DXT1** (not 512^2 16-bit);
`globalmetamaterials.dds` is 1024 x 1024 `A4R4G4B4` (masks a f000, r 0f00, g 00f0, b 000f), one slot nibble
per channel (the shader does `round(sample * 15)`). The Global PS reads the gradient from `.xy` (BC1 RG)
where the tile path reads `.xw` (A8L8), and rebuilds the fourth weight as `1 - saturate(r + g + b)`. All
layers are addressed at `world.xz / 32768 + 0.5` (VS `o1.zw`), the texture-zone map (`s7`, point) at the
same UV, with zone row `zone * 16 + slot` (`data.dds` is *not* the zone map; its consumer is Gap 2).

### Per-vertex versus per-pixel (native-proven for the LOD weights)

* `t0`/`t1` (`1 - saturate((cheb - r) * 1/w)`) are **vertex-shader outputs** (`FastTer_Sh*` VS `o3.zw`;
  Chebyshev distance of the geomorphed vertex position to the camera), consumed by the PS as linearly
  interpolated, perspective-correct inputs (every `dcl_input_ps` is `interp=2`). The geomorph factor of
  the global VS is `o3.zw` too. jc2-rs evaluates them per pixel (`terrain_splat.wgsl`,
  `rel`/`cheb`/`t0`/`t1`); with 4 m patch cells at lod 0 this differs from the native result only where
  the max-axis of the Chebyshev norm changes inside one triangle. **Inferred**: visually negligible, but
  not bit-exact; no fixture measures it (Gap 5).
* Everything else (slot nibbles, weights, gradient, band selection, displacement, hollows) is per pixel
  in the native shader, as in the runtime. Fog is per vertex (`o2.w`, then `exp(-v2.w)` in the PS).

## Proven

1. The pale far hills are `horizon.rs` palette chunks, not any `terrain_splat.wgsl` term; the same hills
   drawn by tiles are dark (captures above; static check of `horizon.rs`/`world.rs` in the probe).
2. Native far shading = Lod2 maths on global layers (`FastTerrainShader_Global`), shader text identical;
   patch lod 9 = Blend, > 9 = Global (`FUN_00a579d0`).
3. Colour table sampler s1 is point/point/point, clamp; the table is `R8G8B8A8_UNORM_SRGB`.
4. Lod2/Global hollows term = Lod0 term; ambient only.
5. LOD blend weights are per vertex, linear interpolation; fog per vertex.
6. Global layer formats above; texture zones Jungle [0,1], Arctic [0], DesertCity [1].
7. The forest canopy shader pair exists in the bundle with the inputs/maths listed in the probe.

## Inferred (not proven)

* The reference's dark green slopes need the native forest canopy layer; the shader maths and the
  olive texture colour are proven, its draw gating and data are not (Gap 1).
* Channel order of the colour table's source bytes is natural RGB in the engine (the file is read as
  `A8R8G8B8`, `FUN_00a70fb0` copies four raw bytes per texel into an RGBA-order texture; the loader's
  in-memory order for a 32-bit BGRA DDS was not traced). If it were swapped, far ground would be teal;
  the engine's own Lod12 cross-fade to the atlas would have shown a hue jump (Gap 3).
* The per-vertex versus per-pixel LOD difference is negligible (Gap 5).

## Gaps

1. **Forest canopy layer.** `TerrainShaderForest` / `TerrainShaderForestFin` (PS/VS in the bundle), blocks
   `CTerrainRenderBlockForest` (`FUN_00a785a0`, `FUN_00a7d940` constants `32000/(k*35000)`,
   `1/cb`), `CForestPatch` (`FUN_00a55dd0`), textures `forest_dif.dds` / `forestfin_dif.dds` loaded by
   `FUN_00a67ef0`. Unknown: which tile/global file feeds the density (`t0`, `.x` of a BC1 layer; `data.dds`
   is the candidate), what `t2` is (decoded as a normal: `(0.5 - xy) * 8.56`; `globalslopes` is the
   candidate), the draw range and fade constants (`cb1[4]`, `cb4[3].w`, `cb4[4].x`), and the Fin pass.
   Blocks the dark-green hillsides. Needs its own research task.
2. **`data.dds` (1024^2 BC1) consumer** (`FUN_00a67ef0`, `Landscape\data.dds`): zone map is
   `texturezonemap.dds`; `data.dds` is loaded next to the forest textures only.
3. **Channel order** of the loaded `globalcolorpixelmap` texture memory read in `FUN_00a70fb0` (loader
   `FUN_009442e0` -> `FUN_00966440`/`FUN_00944d80`, not traced).
4. **Patch lod byte semantics and distances** (`patch+0x61`): which tile/global patches are lod < 9, == 9
   (Blend: crossfade of tile layers `t3..t5` with global `t8/t9/t0` by the geomorph factor `v3.z`), > 9, and
   the camera distances at which the streamer switches; blocks the exact tile/horizon hand-over radius.
   `FastTerrainShader_Blend` was only diffed against Global, not transcribed.
5. **Per-vertex LOD weights**: not measured; the runtime keeps per-pixel evaluation.
6. **`FUN_004dcb70` noise**: not examined here; it is the cloud/haze fBm (`haze-fog-next.md` G7,
   `cloud-shadow-consumers-next.md`), not a terrain-shader term, so it does not block JC2-101.
7. Diffuse/normal atlas samplers `s8`/`s9` of the near shaders (set in `FUN_00a579d0`: (3,2) trilinear in
   the near path) and the displacement atlas inset are as already documented; their colour space
   (sRGB flag of the atlas creation) was not looked at.
8. Fog terms in the terrain VS/PS (`cb0[12]`, `cb0[13]`, `cb0[14]`) are JC2-116's.

## Rust handoff (bounded; not applied here)

1. `crates/jc2-game/src/horizon.rs`: stop colouring chunks with `terrain_colour`
   (`chunk_mesh`, the `colours.push(...)` and `ATTRIBUTE_COLOR`) and the shared white `StandardMaterial`
   in `setup_horizon`; give the chunks the terrain splat material in a *global* mode (below). Keep
   `terrain_colour` as the fallback when the global layers are missing.
2. `crates/jc2-game/src/terrain_material.rs`: build a `TerrainSplat` global instance once: the global
   layers (`globalmetamaterials` repacked from `A4R4G4B4` to `Rgba8Unorm`/`R16Uint`, point; `globalmetaweights`
   BC1, linear; `globalslopes` BC1, linear; `texturezonemap` point) and **both** zones' tables (the uniform
   `table`/`colours` arrays doubled to 2 x 256 entries); the colour table stays an sRGB-decoded uniform.
3. `crates/jc2-game/src/terrain_splat.wgsl`, under a `TERRAIN_GLOBAL` shader def:
   * replace the `local`/`texel`/`tc`/`slots`/`w`/`g` block (the lines after `let world_xz`) by
     `uv = world_xz / 32768.0 + 0.5`, `slots = round(textureSample(global_meta, point, uv) * 15)` (R,G,B,A =
     slot 0..3), `w` and `g` from the BC1 layers at `uv` (the gradient `.rg` already is what the BC1 layer
     stores); the zone from the zone map at `uv` (point) selects the table half in `table_entry` /
     `table_colour`;
   * set `t0 = 0` and `t1 = 0` (so only the `colour = p2.c1 * p2.w1 + p2.c2 * p2.w2` colour-table branch
     runs; the atlas and normal-atlas branches are skipped by their existing `t > 0` guards);
   * leave the displacement, the second band selection, the `1.5 * f3` normal bend and the hollows term
     `1 - 0.6 * f3 * (1 - d.z) * 0.5` as they are (proven equal to Lod2/Global above);
   * keep the horizon / AO / sun-correction block unchanged.
4. Do **not** change `table_colour`'s sRGB decode, the point lookup of the table, or the hollows term.
5. Follow-up (new task, not part of this fix): the forest canopy layer (Gap 1); until then expect brown/grey
   olive far slopes, as in capture B.
6. Update `terrain.md`: global layer formats, `s1` filter, strike the Lod2 hollows "unknown".

Fixtures: probe check names above; capture commands in the table; `research/analysis` decompiles
`FUN_00a570d0`, `FUN_00a579d0`, `FUN_00940170`, `FUN_0091c080`, `FUN_00950420`, `FUN_00a70fb0`.
