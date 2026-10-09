# Forest canopy terrain layer (TerrainShaderForest / TerrainShaderForestFin), JC2-158

*9 October 2026. Research only; no runtime change. Probe: `research/tools/forest_canopy_probe.py`
(98 checks, exit 0). Native addresses are retail `JustCause2.exe` VAs (sha256 `df99a479...b74f80`);
shader names are in `DX10_Shaders_F.shader_bundle` (sha256 `db8c67a0...ebc292e98`). Closes Gap 1 of
`terrain-terms-next.md` (JC2-101) except where the Gaps below say otherwise.*

```powershell
$env:JC2_TEST_DIR = '<game folder>'
python -B research/tools/forest_canopy_probe.py [--json]     # needs target/release/jc2tool.exe; exit 0 = all pass
```

## Answer first

**Data source.** There is no per-patch or per-tile canopy data. The layer reads **four global textures** at
the global UV `(world.xz + 16384) / 32768`:

| slot | file (archive member `textures\landscape\...`) | role | native proof |
|---|---|---|---|
| t0 | `data.dds` (1024 x 1024 DXT1, no mips, grey: R = G = B; one texel = 32 m) | **canopy density**, `.x` | `FUN_00a67ef0` loads `Landscape\data.dds` into global `0138aaa4`; both blocks bind it as t0; PS `alpha = t0.x * ...` |
| t1 | `forest_dif.dds` (256 x 256 DXT5, 9 mips) at `world.xz / 400` (repeats every 400 m) | canopy **colour (rgb) and cutout mask (alpha)** | loaded into `0138aa98`; bound as t1 |
| t2 | `globalslopes.dds` (the terrain's own global gradient layer) | **normal for the lighting**: `normalize(8.56031 (0.5 - x), 0.5, 8.56031 (0.5 - y))` | owner ctor `FUN_00a8f600` loads `Landscape/globalslopes.dds` into `[01191d98 + 0xd08]`; both blocks bind that as t2; the terrain Global draw binds the same slot to slot 0 where `FastTerrainShader_Global` decodes `(x, y) * 2 - 1` as the gradient |
| t11 | `AmbientOcclusionMap.dds` | sun-shadow / AO exactly as the terrain (`cloud-shadow-consumers-next.md`) | `FUN_00a67ef0` registers it in the global slot 11 |

`globalmetaweights`, `globalmetamaterials`, the texture-zone map and `CForestPatch` carry **no density**:
`CForestPatch` is a 0x18-byte object whose only payload is a pointer to the terrain patch (`+0x14`).
`data.dds` is the density: 73 % of the map is 0, 14 % is >= 27/31, the rest are soft edges. It is white over
the jungle highlands and forested islands, black over the open ground and the Arctic start base.

**Draw range.** With `f = max(fov_rad^2, 0.38)` (the same `f` as the tree impostors, `ctx + 0xca8`) and
`c` the **horizontal Chebyshev distance** from the camera to the (geomorphed) terrain vertex:

```
fade_in  = sat((c - 450 / f) / 350)            // renderer +0xd1c = 450, +0xd20 = 350 (same pair as the Top impostor)
fade_out = 1 - sat((c - 32000 / f) / 35000)    // gone at 67 km / f: irrelevant on a 32 km world
fade     = min(fade_in, fade_out)              // per vertex (VS), interpolated linearly
alpha    = data.x * forest_dif.a * fade
pixel is discarded when alpha < 0.5            // alpha test on, reference 0x80 -> 0.5
```

| f | fade starts | fade reaches 1 | `alpha >= 0.5` possible (density 1, tex alpha 1) |
|---|---|---|---|
| 1.0 (FOV 1 rad) | 450 m | 800 m | 625 m |
| 0.4874 (native 40 deg on foot) | 923.3 m | 1273.3 m | 1098.3 m |
| 0.38 (FOV <= 0.6164 rad) | 1184.2 m | 1534.2 m | 1359.2 m |

**The canopy takes over exactly where the tree cards fade**: `fade_in` is `1 - fade_out` of the
`TreeImpostorTop` VS (same `cb1` pair `(d1c / (d20 f), 1 / d20)`, probe), so the Top cards (gone at 799.3 m for
f = 1) and the canopy cross-fade over the same 350 m. The ~800 m of the cards is the `f = 1` case only.

**It is not alpha blended.** The state set by the block's begin function (`FUN_00a7d5c0`) is blend *enabled*
with factors ONE / ZERO (colour) and ONE / ONE (alpha), alpha test on with reference 0.5, depth test and
write on, back-face culling. The canopy is an **opaque cutout**: a pixel either survives the 0.5 test and
overwrites the terrain colour, or is discarded. The mid-density look comes from the mip-averaged
`forest_dif` alpha (mean 0.596 on every mip), so far away a pixel survives only where
`data.x >= 0.5 / 0.596 = 0.84`.

## Geometry

* **The canopy is the terrain patch mesh drawn a second time, 10 m higher.** `FUN_00a7d940` (vtable slot 29 of
  `CTerrainRenderBlockForest`, vftable `00f20c6c`) fetches the patch's vertex/index buffers from the terrain mesh
  table `01368150 + patch_index * 0x70` and uploads `cb2[0] = (patch.x - cam.x, (10 - cam.y) + 1100, patch.z - cam.z, 0)`,
  `cb2[1] = (side/64, 0.033570, 1 / (side/2), -4.5)` with `side = 2^(patch byte +2 & 0xf)` m. The VS is the terrain
  VS (position `(x, h0, z, h1)` scaled by `32767 * cb2[1].xyxy`, `h = lerp(h0, h1, sat(c * 2/side - 4.5))`, the
  same geomorph, see `terrain.md`) with `+10` in the height; `+1100` undoes the signed-short bias
  (`32768 * 0.0335704 = 1100.03`). Index stitching (`FUN_00a78160`, 15 cases from `byte +2 >> 4`) is the
  terrain's. **So any terrain mesh the runtime already has can be re-drawn at +10 m.**
* **Which patches.** `FUN_00a67360` (patch init) creates the block (`FUN_00a55dd0`: allocates 0x18 bytes, vftable
  `CForestPatch` `00f1fb04`, base ctor `FUN_00a785a0`) iff the patch level (`byte +2 & 0xf`) is >= 8
  (side >= 256 m) and `0 <= ix, iz < 8 << (14 - level)`; otherwise the block is released. Draw guards: the
  `CVegetationManager` bytes `+4` and `+5` (both set to 1 by its ctor `FUN_00a94250`) and `data.dds` loaded.
* **Fins (`TerrainShaderForestFin`).** One block per **512 m tile** in a 15 x 15 ring around the camera tile
  (`FUN_00a89950`: slot `((ix mod 15) * 15 + (iz mod 15)) * 0x30`, rebuilt when the stored tile differs;
  builder `FUN_00a86f10` -> generator `FUN_00a7df90`). A fin cell is two interleaved 7 x 7 jittered point grids
  (step 64 m; second grid offset 32 m in x, 16 m in z; +-16 m jitter from the MSVC `rand` LCG `0x343fd/0x269ec3`
  seeded `x * 231 + z * 435`), 544 m extent, 196 vertices of `(float y, u32 colour)`; each point is a vertical
  pair: bottom `y = h + 11`, top `y = h + 56` if `h + 11 >= 200` (sea level), else equal to the bottom
  (degenerate). The VB is created only if a top exceeds 200 m. The colour bytes are `(x, z, u, v)` with
  `x = (px - cell.x) * 255 / 544`; VS: `xz = v1.xy * 544 + cell.xz - cam.xz`, `uv_global = v1.xy * (544/32768, 528/32768)
  + tile uv`, `uv_fin = v1.zw`. Index buffer `FUN_00a78740`: 864 u16 (two 6 x 6 quad sets of 12 indices).
  Fin PS = canopy PS with `t1 = forestfin_dif.dds` (256 x 64 DXT5; alpha tops out at 0.471), fade doubled
  (`o2.w = 2 * min(...)`, up to 2), back-faces drawn (cull NONE), horizontal Euclid distance
  `< d1c - 64 = 386 m` clipped, fade-in the same `450/f` ramp, fade-out
  `1 - sat((c - 1500/f) / 2000)` (gone at `3500/f` m; the 15-tile ring is +-3.84 km).

## Pixel shader (forest and fin identical after the alpha)

```
N      = normalize(8.56031 * (0.5 - t2.x), 0.5, 8.56031 * (0.5 - t2.y)); hemi = 0.5 + 0.25 / length(...)   // = 0.5 + 0.5 N.y
shadow = max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w)
sun    = sat(dot(N, -sunDir)) * shadow * 0.7 * (0.5 + 0.5 * ao)           // ao = t11.x; the 0.7 is canopy-only
amb    = ao * mix(cb0[11].xyz, cb0[10].xyz, hemi)
rgb    = fog((sun * cb0[3].xyz + amb) * t1.rgb)       // lerp to cb0[13] with exp(-fogExponent), exponent per vertex as the terrain VS
```

`FastTerrainShader_Global`'s tail is the same expression without the `0.7` (probe). Vertex depth is
`z + 1e-6 w` (bias); the fog distance is `w - z'` as in the terrain VS.

## State (native)

| | forest | fin |
|---|---|---|
| blend | enable, src ONE, src-alpha ONE, dst ZERO, dst-alpha ONE | same |
| depth | test + write | same |
| cull | BACK (3) | NONE (1) |

State word layouts decoded from `FUN_00933530` (blend: bit0 enable, bit1 alpha-to-coverage, bits 2/7/12/17
src / src-alpha / dst / dst-alpha, 22 op, 25 op-alpha, 28 write mask), `FUN_009603c0` (depth) and
`FUN_009338a0` (rasterizer: `& 3` fill, `>> 2 & 3` cull); setters `FUN_0091c270`, `FUN_00957db0`, `FUN_00952cc0`,
`FUN_0095fe00`, `FUN_009225e0`, `FUN_009576b0` (all pinned in the probe).

## Proven

1. t0 = `data.dds`, t1 = `forest_dif.dds` / `forestfin_dif.dds`, t2 = `globalslopes.dds`, t11 = AO map; names,
   destinations and bind slots from `FUN_00a67ef0`, `FUN_00a8f600`, `FUN_00a7d5c0`, `FUN_00a7dad0` bytes; there is
   no other canopy data input (shader texts and function bytes, probe P2/P3).
2. The canopy ramp `(c - 450/f)/350` with `f = max(fov^2, 0.38)`, the far ramp `(c - 32000/f)/35000`, `c` =
   horizontal Chebyshev distance, alpha `data.x * forest_dif.a * fade`, discard below 0.5 (probe P2/P4/P6).
   `450/350` are the renderer floats `+0xd1c/+0xd20` (table `00f20a20`, `tree-impostors-next.md`).
3. Opaque cutout, not alpha blend (state above); depth write; back-face cull (forest) / none (fin).
4. Geometry = the terrain patch mesh +10 m (`cb2[0].y = 10 - cam.y + 1100`); forest blocks exist for patch levels
   >= 8 only; the gate `FUN_00a67360` and the vtable slots are byte-checked.
5. Fin cell generator, ring and vertex pair rule above (constants 2200, 512, 0.125, 255, 231, 435, 16, -11, 200, 56).
6. Lighting formula and the canopy-only 0.7 sun factor.
7. The canopy fade-in is the complement of the Top-impostor fade-out; `forest_dif` alpha statistics; canopy colour.

## Inferred (not proven)

* The terrain VB format is 4 x signed 16-bit normalised (`* 32767` in the VS); the input-layout element code (4)
  was not decoded.
* `forest_dif`/`forestfin_dif` colour is sRGB-decoded (the project's diffuse policy); the retail SRV format was not
  traced (Gap 2). `data.dds` density and `globalslopes` are read as plain UNORM.
* The `cb4[3].w` / `cb4[4].x` PS values are `DAT_013d9e2c` / `DAT_013d9e30` (alpha-test enable / reference); the
  setters and the shader gate are proven, the constant-buffer upload of those two globals was not walked
  (consistent with `tree-impostors-next.md`).
* `CVegetationManager +4/+5` are user-facing toggles (both default 1); only their draw-guard role is proven.
* The fin ring's 15 tiles is the 7 + 1 + 7 vegetation patch ring (`DAT_0108b8fb = 7` at lodfactor 0); the
  hard-coded `mod 15` is proven, the relation is not.

## Fixtures (probe output)

| fixture | value |
|---|---|
| `data.dds` jungle tile 40,40 (16 x 16 texels = 512 m) | mean density 0.792; 78.9 % of texels >= 0.5; 65.2 % >= 0.84; 0.8 % zero. 5 x 5 tiles around it: mean 0.718, 72.7 % >= 0.5 |
| `data.dds` start base tile 31,13 (Arctic) | **0 on the whole tile** (no canopy at the base); 5 x 5 tiles around it: 6.6 % >= 0.5, 90 % zero |
| `data.dds` whole map | 73.2 % zero, mean 0.188, 19 % >= 0.5 |
| `forest_dif` alpha | mean 0.596 (0.589-0.598 on every mip); texels >= 0.5: 63.7 % (mip 0), 60.8 % (1), 62.6 % (2), 67.0 % (3), 71.5 % (4), 79.7 % (5), 100 % (6) |
| `forest_dif` colour | alpha-weighted sRGB (0.2086, 0.2481, 0.0930); linear (0.0394, 0.0548, 0.0101); mean linear of texels with alpha >= 0.5 (0.0401, 0.0557, 0.0102), luminance 0.049 (JC2-101's far ground median was 0.038) |
| `forestfin_dif` alpha | mean 0.153, max 0.471 (so only the doubled fade passes the test) |
| coverage at full density, mip 0, f = 1 | fade 0.43 at 600 m: 0 %; 650 m (fade 0.57): 27.4 %; 700 m: 47.4 %; 750 m: 56.9 %; 800 m: 63.6 % |

Expected look at jungle tile 40,40 from the highlands at > 1 km: opaque olive (linear ~0.04, 0.056, 0.010 before
lighting, times `0.7 * sun + ambient`) over 65 % of the area where density >= 0.84, bare terrain elsewhere.

## Gaps (each blocks only the stated detail)

1. **Blend op, write mask and alpha-to-coverage.** The begin functions set only enable + the four factors;
   `BlendOp` (bits 22-27), `RenderTargetWriteMask` (28-31) and bit 1 (alpha-to-coverage) are inherited from the
   pass default, not traced. Blocks: exact alpha-channel result of the canopy in the G-buffer/alpha target.
2. **Texture view colour space** of `forest_dif` / `forestfin_dif` (and `data.dds`): the texture manager load slot
   `+0x28` / SRV policy (`material-parameters-next.md` open item). Blocks: exact canopy brightness.
3. **Render passes.** Which passes call the forest/fin blocks (main only, or also depth pre-pass / shadow cascades /
   water reflection); block-type vtable slot 11 `FUN_00a78520` was not walked. Blocks: whether the canopy casts shadows
   or appears in reflections; draw order relative to the terrain.
4. **Vertex/index layout.** Input-layout element code 4 of `FUN_00a88aa0`, the index ranges behind the 15 stitching
   cases of `FUN_00a78160` and the fin topology of `FUN_00a78740` (864 indices) were read, not rendered. Blocks: a
   bit-exact fin mesh.
5. **`CVegetationManager +4/+5`** meaning and setters (graphics options?). Blocks: the user-visible toggle only.
6. **Fin details.** UV z-extent `528/32768` against the 544 m position extent, the u byte pattern `(0, 0x7f, 0xff, 0x7f)`,
   v byte `0xff`/`0`, and whether the fin ring honours vegetation patch radius 7/11/13. Blocks: fin texture mapping.
7. **Depth convention.** The VS adds `1e-6 * w` to clip z; its sign relative to the runtime's (reverse-Z?) projection.
8. **Fin sampler filter code (5,2)** through the table at `0xf22858` (the probe decodes (3,2) only).
9. **Patch level to distance** (JC2-101 Gap 4): at which camera distance the streamer switches patch levels; the canopy
   needs only levels >= 8 but the exact tessellation seen by the player depends on it.
10. **Relation of `data.dds` to the vegetation/tree data** (is it baked from the tree or the `Rain_Forest` cells?).
    Measured only as fixtures above; no correlation computed.

## Rust handoff (bounded; not applied here)

1. New `crates/jc2-game/src/forest_canopy.rs` + `forest_canopy.wgsl`: a canopy material drawn as an **opaque pass with
   `discard`** (no blending), depth write on, back-face culling, over the existing terrain meshes.
2. Geometry: re-use the tile meshes and the horizon chunks: a second entity/instance per mesh lifted by **+10.0 m**
   (or `y += 10` in a `FOREST_CANOPY` vertex variant of the terrain splat shader). Do not draw for patches finer than
   256 m. `horizon.rs` chunks sink up to 18 m (`SINK`): apply the canopy offset to the unsunk height (Gap 3 covers the
   native pass behaviour).
3. Data inputs (load once): `textures\landscape\data.dds` (BC1, UNORM linear, bilinear, global UV
   `(x + 16384) / 32768`), `forest_dif.dds` (BC3 with its 9 mips, repeat, trilinear, UV `world.xz / 400`, colour
   texture), the already planned global `globalslopes` (BC1 linear), the AO map (`cloud-shadow-consumers-next.md`).
4. Fragment: the formulas above (`N` from `globalslopes` with 8.56031, `shadow`, `0.7 * sun`, `ao`, fog as the splat
   shader), `alpha = data.x * tex.a * fade; if alpha < 0.5 { discard }`.
5. Fade in the vertex (or per pixel): `f = max(fov_rad^2, 0.38)` (the same value the impostor code uses),
   constants `450.0`, `350.0`, `32000.0`, `35000.0`; Chebyshev horizontal distance from the camera. Unit-test the table
   above (0 / 0.5 / 1 at `450/f`, `450/f + 175`, `450/f + 350`) and the coverage numbers.
6. Fins (second step, only after the canopy is accepted): one mesh per 512 m tile in a 15 x 15 ring, generator as
   specified (Gap 6 first); the fin fade and clip constants are in the Geometry section.
7. Validate with fixtures: jungle 40,40 from the highlands (olive cutout over about two thirds of the dense area; none
   inside ~600 m) and the Arctic base 31,13 (no canopy on the tile itself).
8. Update `terrain.md` / `terrain-terms-next.md` Gap 1 pointers once accepted; correct the earlier statement that the
   canopy is alpha blended.
