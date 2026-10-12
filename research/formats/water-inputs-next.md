# Ocean inputs that were still open: colour rows, sun specular, LOD, reflection clip, SV_Target1, depth copy (JC2-102)

9 October 2026. Research only: no Cargo, GPU, game launch or runtime edit. Closes the "unrecovered" inputs that
[water-consumers-next.md](water-consumers-next.md) (JC2-095, section 5 and 8) listed as blockers of JC2-013 stage S1.
Probe: `research/tools/water_inputs_probe.py` (120 checks, exits 0 only if every one passes; any missing input is a failure).
All addresses are runtime addresses of `JustCause2.exe` (no ASLR); shader offsets index `DX10_Shaders_F.shader_bundle`.
Pseudo-C was read from the git-ignored `research/analysis/pseudo/_unnamed` export; every function the claims rest on is pinned
by a sha256 of its executable bytes (printed by the probe, 27 functions) and its quoted fragments are asserted.

```
python -B research/tools/water_inputs_probe.py --tool <private copy of target/release/jc2tool.exe> [--game <game>] [--pseudo <_unnamed>] [--verbose]
```

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `DX10_Shaders_F.shader_bundle` | `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98` |
| `DX10_SpecialShaders_F.shader_bundle` (916 B) | `3a1d45857fe6b3e88c9633325a8676e33366ce35e0ef820fb8fb7aba2ea3506d` |
| `textures\atmosphere\gradients.bmp` (1,640 B, via `jc2tool cat`) | `b66bf51075ed538b5a3fe72bc30c3ccd52e1cfcb0b618bde478425864648a45d` |
| tables E / F, 3 variants x 25 entries, RGB to 6 decimals (probe digest) | `346708f6...41bc0` / `9ff40cc7...4d694` |

Key function pins (first 12 hex of the byte sha256): `FUN_00a80300` 5ee6d2eb8405, `FUN_00a809c0` 02e43f6a37b3, `FUN_00a79730` 24e835cd9404,
`FUN_00a67610` 8c00c48cc3d7, `FUN_00a432d0` e4dab1ea66aa, `FUN_00957db0` eccdf84b0fb9, `FUN_00933530` 45977ad0fddf,
`FUN_00949f10` a10cf96863e7 (see the probe JSON for all 27).

## Answers

1. **Water colour rows are not 1.0: they are tables E and F of `gradients.bmp`** (section 1). The "initialised to 1.0, no filler" gap of
   JC2-095 was wrong in its premise: `FUN_00a46bd0` (light-tables-next.md section 1) fills E and F, and `weather.md`'s claim that the
   retail file holds no colours was already corrected there. Fixtures below.
2. **The sun specular base colour `DAT_0108c5e4/e8/ec` is a static white (1, 1, 1)**: initialised data in `.data`, never stored to (section 2).
   The ocean highlight colour is therefore fully determined by the elevation, camera height and precipitation factors; the missing term
   `ctx+0xd08` is the precipitation weight `P.+0x48`.
3. **LOD0/1/2 are chosen by the patch side, not by a camera distance**: side <= 32 m (and camera < 128 m above the sea) is LOD0, 64/128 m is LOD1,
   >= 256 m is LOD2; the sides come from the terrain clip-map (11 nested 7 x 7 patch windows, 16 m .. 16384 m) (section 3).
4. **Reflection clip: a pixel discard at world y < 200 in the four `*reflection` pixel shaders** (no clip distance exists in any of the 899
   shaders). **`SV_Target1` is the dual-source blend factor** (`SRC1_ALPHA` / `INV_SRC1_ALPHA`), a shoreline/depth fade. **The scene-depth copy
   is a runtime-compiled HLSL pixel shader**, `min` over the MSAA samples (a plain load when the sample count is 1) (section 4).
5. Waves, partial: the 48 x 48 wave grid is camera-centred (300 m wrap) and each object is accepted only on a sloping shore; clocks and
   acceptance are pseudo-C reading, so section 5 and the Gaps keep them open.

## 1. Water colour (`cb0[14]`)

`cb0[14]` is the render engine's fog layer 1 (`CRenderEngine+0xd8`, uploaded by `FUN_0095b150`; water-consumers-next.md section 5).
It is rewritten every frame by `FUN_00a432d0` (called from the weather frame `FUN_00a654b0` with the rain and snow weights

* Variant 0 rows are the base; the rain weight blends toward the variant 1 row (`+0x19` entries), then the **result** toward the variant 2 row (`+0x32` entries) by the snow
  weight, both only when the weight is above 0 (`(v - base) * w + base`, evaluated sequentially). Retail E/F variant 2 equals variant 0; variant 1 is a flat grey-green.
* The `w` component is replaced by the optical density **0.3 (camera >= 200.0) / 0.15** (`FUN_00a432d0`, immediates loaded from the image); the shader reads
  `cb0[14].w` as the base-2 absorption `1 - 2^(-thickness * w)` (water-consumers-next.md section 5).
* Colour space: linear 0..1 (`pixel ** 2.2`, `light-tables-next.md` section 1), alpha 1.0 before the density replaces it.

Fixtures (linear RGB, clear weather, probe `fixtures.water_colour_E_F_variant_rows`; independent raw-pixel checks at hours 0/4/12/20 for all three variants):

| hour | E (camera >= 200) | F (camera < 200) | E, rain weight 1 (variant 1) |
|---|---|---|---|
| 0 | 0, 0, 0 | 0, 0.02655, 0.04455 | 0, 0, 0 |
| 4 | 0.00259, 0.00370, 0.00226 | same | same |
| 6 | 0.01040, 0.02655, 0.01607 | same | 0.01188, 0.01699, 0.01040 |
| 12 | 0.00837, 0.13035, 0.13035 | same | 0.01188, 0.01699, 0.01040 |
| 18 | 0.01040, 0.02655, 0.01607 | same | 0.01188, 0.01699, 0.01040 |
| 23.5 | 0, 0, 0 | 0, 0.02655, 0.04455 | 0, 0, 0 |

E and F differ only at hours 0..3 and 21..23. The project's current sea material (`world.rs`, `Color::srgba(0.05, 0.25, 0.45, 0.75)`) is policy; the native
value is the deep-water absorption colour `cb0[14].rgb` that the shader blends toward, not a surface albedo. Repo cross-check: `AtmosphereState::haze_e/haze_f`
(`atmosphere.rs`) already evaluate these tables with the same variant weights (its JC2-139 midnight F test equals the fixture above).

## 2. Sun specular colour `cb1[0]` (PS)

`FUN_00a80300` writes `cb1[0] = (x, y, z, 256.0)`:

```
a = clamp(|sy * 32|, 0, 1)      h = clamp(1 - (camY - 200) * 0.003, 0.6, 1)      c = 1 - wet
f7 = max(0, 0.8 - 1.5 |sy|)     f8 = max(0, 1 - 2 f7)                            m = 8 if sy > 0 else 1
k = 4 * a * h * 0.15 * c * m          x = Rs k      y = (1 - f7) Gs k      z = f8 Bs k
```

* `(Rs, Gs, Bs) = DAT_0108c5e4/e8/ec` = **(1, 1, 1)**: the dwords are 0x3f800000 in the initialised part of `.data` (as is `+0xf0`, alpha); all 19 absolute references in the
  executable are SSE reads (18) plus one `push 0x108c5e4` at 0x87b917 that hands the address to the debug-draw/marker call `FUN_00677ff0` as a colour argument; no store form
  references the range (probe classifies every hit). A write through that pointer inside the callee is not excluded (Gap 12) but nothing suggests one.
* `sy` = ocean object `+0x54` = `-(weather+0x264)` = **the y component of the unit vector towards the sun, unflipped**: `FUN_00a8b790` negates `W+0x260..` (= `S+0x250..`),
  which `FUN_00a58a00` stores before its moon flip (`haze-fog-next.md` section 3). It is the sun even at night: `sy <= 0` selects `m = 1`, so the night highlight is 1/8 of the day one.
  (light-tables-next.md section 3). 0 in clear weather.
* `h` uses `ctx+0xc9c`, the camera position y from the matrix copy in `FUN_009521e0`.
* The half vector of the highlight is not `sy`: the VS `cb1[0]` is `(-L.x, max(-L.y, 0.3), -L.z)` with `L = light+0x44` (`FUN_005351e0`, the **flipped** light, `cb0[4]`),
  so at night the glint follows the moon with the sun's intensity terms.
* PS use: `spec = cb1[0].rgb * (1 - reflection.a) * pow(sat(N.H), cb1[0].w)`, added after the surface colour; `o0.w = 4.5 * cb1[0].x * (1 - reflection.a) * pow(sat(N.H), 8 * cb1[0].w)`.
  `1 - reflection.a` is 1 where the cleared (0,0,0,0) reflection target was not drawn on (sky) and 0 where geometry wrote alpha 1 (section 4).

Fixtures (probe, camera at y = 200 unless noted, wet 0): sy 0.9 -> (4.8, 4.8, 4.8); 0.3 -> (4.8, 3.12, 1.44); 0.1 -> (4.8, 1.68, 0); 0.02 -> (3.072, 0.70656, 0);
0 -> (0, 0, 0); -0.5 -> (0.6, 0.57, 0.54); sy 0.9 at camera y 500 -> x 2.88 (h floors at 0.6); wet 0.5 halves; exponent always 256.

## 3. LOD0/1/2 selection

(+0x14/+0x18 = patch x/z origin, **+0x1c = side = 2^level m**, +0x21 = level) only if the patch's lowest height `patch+0x18 <= 200.0`. The block draw is slot +0x74 of the vtable at
0xf21094 = `FUN_00a809c0` (byte-checked in the vtable). With `side = record+0x1c`, `H = max(camera.y - 200, 0)`:

| Condition | Vertex shader (type +0x1c / +0x20) | Pixel shader (type +0x24 .. +0x30) |
|---|---|---|
| camera.y < 200.0 | WaterBelow (+0x34) | WaterBelow (+0x38) |
| 32 < side <= 128, or H >= 128 | lod0 for side <= 128 | `watershader_lod1` (7551512) |
| side > 128 | `watershader_lod1` (342468) | `watershader_lod2` (7555468) |

The height rule and the side rule are independent (`128.0 <= H` forces PS lod1 for small patches; VS lod1 needs side > 128 only). `side = 128` is still lod0 VS / lod1 PS.
The VS draws a 5 x 5 vertex patch (`cb2[0] = (x, 200.0, z, side)`, `0x43480000` = 200). **VS lod0 displaces the vertices**
(`y = 200 + 0.6 * noise(7.34 * p) * max(0, 1 - 0.04 * d)`, `d` the horizontal distance to `cb0[4].xz`, the camera position: only within 25 m); VS lod1 is flat. PS lod0 = 4 normal layers + foam t6;
lod1 = same 4 normal samples without foam t6 (3,936 B); lod2 = 1 normal sample (3,396 B). **`waterhighend` PS (7537032) is byte-identical to lod2**, and the type constructor
`FUN_00a79730` loads exactly lod0/lod1/lod2/lod0_stereo/WaterBelow, so the `waterhighend` pair has no loader we found (Gap 11). All five PS variants write `SV_Target1`.

**Where the sides come from.** The terrain renderer `FUN_00a67610` keeps 11 clip-map levels (level nibble 4 .. 14, side 16 m .. 16384 m; arrays `0xf0f834`, `0xf0f830`, `0xf0f7e8` hold `1/s`, `2/s`, `s/2`; side tables
`0xf20980` and `0xf0f7b8` are 2^level). Per level it activates the **7 x 7 patches `[X-3, X+3]` x `[Z-3, Z+3]`** of the level's grid, `X = floor((cam - (-16384)) / s)`, re-evaluated when the camera
crosses a half-patch line. A coarse patch is cut where finer active patches cover its quadrants (high nibble of patch byte +2, table `DAT_01365498`; `FUN_00a809c0` skips a patch with mask 0xf
and forces mask 0 for level 4), so a point is drawn by the **finest level whose window contains its cell**. Exact (probe fixtures, camera at x = z = 0, an exact 16 m boundary):

| Level (side) | window along an axis | pixel shader (camera < 128 m high) |
|---|---|---|
| 4 (16 m) | -48 m .. +64 m | lod0 |
| 5 (32 m) | -96 m .. +128 m | lod0 |
| 6 (64 m), 7 (128 m) | -192 .. +256 m, -384 .. +512 m | lod1 |
| >= 8 (256 m ...) | -768 .. +1024 m and beyond | lod2 (VS lod1) |

In general a level-`s` window reaches `(3 + f) s` on the low side and `(4 - f) s` on the high side, `f` = camera position within its cell: **LOD0 is a square (Chebyshev) region guaranteed to 96 m and
never beyond 128 m; LOD1 reaches 384 m guaranteed, at most 512 m**. The region is square, not circular.

## 4. Reflection clip plane, `SV_Target1`, scene colour/depth copies

**Clip plane (proven: shader scan).** No shader of the bundle declares `SV_ClipDistance`/`SV_CullDistance` (899 blobs). Exactly four pixel shaders start with
`add r0.x, v?.y, l(-200)` / `lt r0.x, r0.x, l(0)` / `discard_nz r0.x`, and all four are the reflection variants: `carpaintreflection` (4789728), `carpaintsimplereflection` (4801940),
`fastterrainshader_reflection` (4956204), `generalreflection` (6089308). The tested interpolant is the world position (the reflection terrain VS 133080 writes `object position + cb0[4].xyz`),
so the rule is **discard if world y < 200.0** (strict; y = 200 survives). The terrain variant writes `o0.w = 1`, so the cleared (0,0,0,0) target stays alpha 0 on sky.
The pass flag `ctx+0xd14 |= 4` (`FUN_0095a190`, from `DAT_01191ab4+0xea4`) marks the reflection pass; which other materials, sky, clouds, foliage or particles draw into it, and with
which shaders, is not decoded (Gap 1).

bits 2..6, 7..11, 12..16, 17..21 of the state word at `ctx+0x9c` (keeping bits 0,1 and 22..31); `FUN_00933530` turns the same word into a D3D10 blend description
(`SrcBlend` = bits 2..6, `SrcBlendAlpha` = 7..11, `DestBlend` = 12..16, `DestBlendAlpha` = 17..21, ops 22..27, write mask 28..31; the device call is vtable +0x14c = `CreateBlendState`).
18 and 19 are `D3D10_BLEND_SRC1_ALPHA` and `D3D10_BLEND_INV_SRC1_ALPHA`, so with the default word `0xf2421108` (blend off, ONE/ZERO, ADD, mask 0xf; the only occurrence is at 0x938925) the ocean is:

```
target.rgb   = o0.rgb * o1.w + target.rgb * (1 - o1.w)
target.alpha = o0.w   * o1.w + target.alpha * (1 - o1.w)
```

`o1.xyz` equals `o0.xyz` and is unused by the blend; **`o1.w` is the shoreline/depth fade**. In PS lod0: `t_u = eyeDepth(undistorted uv) - surfaceEye (v5.w)`, `o1.w = sat(sat(4 (t_u - 0.5)) + 0.0005 v5.w)`:
fully transparent (but for the distance term) up to 0.5 m of water, opaque from 0.75 m. LOD1/2/highend end with the same `mad_sat o1.w, v5.w, 0.0005, ...` form. `o0.w` (`4.5 *` highlight) is therefore written into the
scene alpha through the same factor; its later consumer is unknown (Gap 3). The ten distinct literal `FUN_00957db0` argument sets in the export all decode to valid D3D10 blend values (1..19); only
three literal call sites use SRC1 factors (the ocean, the NV ocean `FUN_00a7e400`, and `FUN_00962270`, not analysed).

**Scene colour (t5) and depth (t7).** Pass 44 `COPY_FRAMEBUFFER` calls `FUN_0094ce20` with flags `(bit0) << 8` and low nibble 0, which always issues `ResolveSubresource` (device +0x98) into `back_buffer_copy`
(sRGB RGBA8). Pass 49 `COPY_DEPTHBUFFER` passes `0x14` (low nibble 4): the full-screen path `FUN_00944930` compiles at run time (`D3D10CompileShader`, strings at 0xe5c570 and 0xe5cb18) a full-screen triangle VS
(vertices at (3,-1), (-1,-1), (-1,3) by `SV_VertexID`, z = w = 1) and a PS that loads the source depth texel at the integer pixel position, sample 0, then for each further sample
`i = 1 .. SAMPLE_COUNT-1` keeps the minimum of the loaded values and returns it (the source is declared `Texture2D` when `SAMPLE_COUNT` is 1 and `Texture2DMS` otherwise; the sample
count is baked in with `#define`). The result is written into the R32F `depth_buffer_copy`. With MSAA off (`SAMPLE_COUNT 1`) this is an exact copy of the depth value. Because `1 / (d x + y)` makes `d = 0` the far plane (water-consumers-next.md section 5), the `min` over samples picks the
**farthest** sample. The `DX10_SpecialShaders` bundle (`constantalpha`, `constantalphadepth`, `depthshader`) has no texture sample, so none of it is the copy shader. The unconditional `ResolveSubresource` for the colour copy with MSAA off is
unresolved (Gap 2).

## 5. Waves (partial; everything here is pseudo-C reading with pinned bytes, not instruction-level proof)

* `FUN_00a89230` (each `UpdateWater` when `DAT_0108b8f8`): phase `+0x18 += 0.45 dt` wrapping at `DAT_0108b930` = 2 pi, `+0x1c += 0.25 dt` wrapping at 1; objects farther than 150 m from the camera (x/z) are moved by whole
  300 m steps and flagged; up to 16 flagged objects per frame are re-initialised by `FUN_00a81b00`. So the waves follow the camera in a toroidal 300 m window.
* `FUN_00a81b00` accepts an object only if the terrain height under it is below 199.9 and the shore geometry holds: land (> 200) within about `5 / depth` or `8 / depth` metres uphill (depth = 200 - terrain height, floored at 0.1), water 50 m
  downhill, and both flank samples (+-3.6 m across a 12 m strip) under water; it builds a 4 x 4 vertex strip whose vertex height is 200.3, or `terrain + 0.05` on rows 2..3 where the terrain is above 200, with a fade ramp in the vertex colour. The direction is the horizontal
  terrain-normal direction returned by `FUN_00a5f600` (the output layout is inferred). So these are **shore waves**, not open-water foam.
  `FUN_00a88d60 = 0.6 * noise(7.34 v) * clamp(1 - 0.015 d, 0, 1)` (`d` = distance from the vertex to the reference point), non-zero only for `|y - 200| <= 5` and `d <= 66.67 m`
  (three callers: 0x59a1e0, 0x59ff30, 0x5ac120, which pass the camera position). The GPU fade (0.04, 25 m) and the CPU fade (0.015, 66.7 m) differ; consumers are unidentified (Gap 7).

## 6. Corrections to earlier briefs

| Brief | Was | Now |
|---|---|---|
| water-consumers-next.md section 5/7/10 | water colour rows "initialised to 1.0, no filler"; E/F rows unresolved | E/F filled from `gradients.bmp` (section 1); S1 can read `AtmosphereState::haze_e/f` |
| same, section 5 | `(Rs,Gs,Bs)` producer unresolved; `ctx[0xd08]` untraced | static (1,1,1); `ctx+0xd08 = P.+0x48` (section 2) |
| same, section 3 | clip plane unresolved | discard at world y < 200 in four reflection PS (section 4) |
| same, section 6/10 | `SV_Target1` meaning unresolved; depth-copy shader unknown | dual-source `SRC1_ALPHA` factor; HLSL min-over-samples copy (section 4) |
| same, section 8 | LOD rule not traced | side/height rule + clip-map windows (section 3) |
| water-next.md | "`cb1[0].rgb` sun-specular colour, alpha meanings differ" | confirmed; `o1.w` is the blend factor |

## 7. Proven / Inferred

**Proven** (bytes, shader disassembly, image data, or state decode pinned by the probe): the E/F table contents and their digest; the layer-1 table/density selection and variant blend structure (fragments of `FUN_00a432d0`);
`DAT_0108c5e4..f0` initial values and the absence of store references; the specular formula and its `FUN_00a80300` operands; `P.+0x48` -> `CRenderEngine+0x140` -> `ctx+0xd08`; the shader slots of the type
constructor and the byte-checked vtable slots; the side/height shader rule; the size and clip-map arrays; the four discard shaders and the absence of clip/cull outputs; the ocean blend word, its decode and
the dual-source factors; the shore-alpha instructions; the embedded depth-copy HLSL; the special-bundle negative; the VS displacement instructions.

**Inferred** (one link is pseudo-C reading or an unverified convention): the blend operation of the ocean pass is ADD (default word carried into the pass, ops not set by the ocean setup); the terrain streaming focus
(`terrain manager +0x5c/+0x64`) is the camera; that the finest-window rule equals the drawn LOD (quadrant mask semantics from `FUN_00a5e4e0`/`DAT_01365498` read, not machine-proven); that record `+0x24` of the
source texture is the depth-copy sample count; `d = 0` is the far plane (algebra exact, naming from earlier briefs); that `ctx+0xcf0` (`light+0x44`, `cb0[4]`) carries the moon-flipped light
(cloud-shadow-consumers-next.md section 2); that `patch+0x18` is the patch's lowest height (initialised 0, written by `FUN_00a64910`); all of section 5.

## 8. Gaps (each is a native detail still unknown; none is filled with a guess)

1. **Reflection pass contents.** Only four materials have reflection pixel shaders with the clip. Whether foliage, grass, trees, sky dome, clouds (cirrus uses flag 4 in `FUN_00a409d0`), particles, waves, the horizon fog ring
   and vehicles other than car paint draw into the planar reflection, and with what shader, is not traced (consumers of `ctx+0xd14 & 4`: text_00a40000.c lines 203/214/3747, text_00a50000.c 5515/6422/8182/8294).
   Blocks: reflection content parity.
2. **MSAA-off colour copy.** `FUN_0094ce20` always calls `ResolveSubresource` for `back_buffer_copy`; what the call does (or how the colour attachment is multisampled) with MSAA off is unresolved, as is the source of the
   depth-copy sample count (texture record `+0x24` assumed). Blocks: exact settings parity of t5 only; the semantic (opaque colour after the opaque passes) is known.
3. **Destination-alpha consumer.** The ocean writes `4.5 * highlight` into the scene alpha through the SRC1 factor; no reader of the scene alpha (bloom, lens, exposure?) was identified. Blocks: glint/bloom parity.
4. **Ocean pass state.** Blend op/mask assumed from the default word; the depth test/write, cull and stencil arguments (`FUN_009576b0`, `FUN_0095fe00`, `FUN_009225e0`, `FUN_00952cc0`) and the sampler filters
   (`FUN_009203e0`, `FUN_00970020`) were not decoded. Blocks: depth/cull/filter parity.
5. **Clip-map focus, budget and masks.** The writer of the terrain manager's `+0x5c/+0x64` position (camera or player), the 8-patches-per-frame init budget (`DAT_012ab570 = 8`) and its visual effect, the
   patch minimum-height source (`FUN_00a64910`), the far "global" patches (lod > 9) and the exact quadrant-cut logic. Blocks: exact ring boundaries under fast camera motion and ocean beyond the loaded tiles.
6. **Shore waves.** `FUN_00a81b00` acceptance and the `FUN_00a5f600` output layout are not machine-proven; per-object animation (`CWaveObject` vtable 0xf21118), the waves VS/PS time inputs, fade, and the POST_WATER draw state. Blocks: shoreline wave parity.
7. **Vertex-wave table and CPU height consumers.** VS cb1[5..38] contents (`FUN_00a80da0`), the three callers of `FUN_00a8bba0`, and whether gameplay (boats, splashes) uses the 0.6 m wave height; the "physical sea level stays 200" rule stands until then.
8. **Weather gating.** Rain/snow variants of the water colour and the specular `wet` term stay 0 until the cloud-column and zone rain mapping (haze-fog-next.md G3/G7) are decoded.
9. **Not covered:** NVIDIA water (`nvwater*`, `FUN_00a7e400`), stereo, WaterBelow/underwater post (JC2-094), godrays, `BlurReflection` kernel, reflection target resolution tuning.
10. **Colour of the horizon band and the sky** the ocean reflects at its far edge (haze-fog-next.md Gaps 1 and 5) still blocks pixel-exact open-sea horizons.
11. **`waterhighend` shaders** (VS 337752, PS 7537032, PS byte-identical to lod2) have no loader in the paths found.
12. **`FUN_00677ff0`** receives the address of the white constant once (0x87b917); it is a generic debug/marker call, not decoded.

## 9. Bounded Rust handoff (not implemented; all steps testable headlessly)

1. `crates/jc2-game/src/atmosphere.rs`: add `water_colour(camera_y) -> Vec3` (`haze_e` if `camera_y >= 200.0` else `haze_f`, already variant-weighted) and `water_density(camera_y) -> f32` (0.3 / 0.15). Tests: section 1 fixtures (1e-5).
2. New pure module (suggested `crates/jc2-game/src/ocean_inputs.rs`): `sun_specular(sun_y, camera_y, wet) -> Vec4` (white base, section 2 fixtures), `ocean_shader_variant(side, camera_y, stereo)`,
   `clipmap_level_at(camera, point)` (11 levels, origin -16384, 7 x 7 windows), `shore_alpha(eye_u, surface_eye)`. Inputs already exist: unflipped sun direction `AtmosphereState::sun_dir.y`, `precip` as `wet`. Tests: the probe fixtures and
   boundary cases (side 32/64/128/256, height 127.99/128, y = 200 exactly, level edges at -48/+64 m).
3. Sea geometry: replace the single plane in `world.rs` by clip-map patches (side 16 m .. 16384 m, 5 x 5 vertices, y = 200 plus the lod0 vertex term within 25 m) selected by `clipmap_level_at`, skipping patches whose lowest terrain is above 200. The first visual
   stage may keep one mesh and assign the pixel-shader tier per patch; do not substitute a circular distance.
4. Sea material: port the LOD0/LOD1/LOD2 pixel shaders literally (water-next.md/water-consumers-next.md equations) with `cb0[14]` from step 1, `cb1[0]` from step 2, `o1.w` as alpha under standard alpha blending (equivalent for RGB;
   dual-source blending is only needed for the unconsumed alpha channel, Gap 3). Use a depth copy equal to the farthest sample for t7 (the exact copy with MSAA off).
5. Reflection pass materials (General, terrain, car paint): add `discard when world_y < 200.0` (strict) when rendering the planar reflection; clear the reflection target to (0,0,0,0) and write alpha 1 where drawn; Gap 1 lists what else must be decided.
6. **Superseded by JC2-189 ([water-sea-consumers.md](water-sea-consumers.md)):** keep `SEA_LEVEL = 200` only for state thresholds and triggers; swimming, boat buoyancy, splash/impact placement and the sea-level condition use the CPU wave height (`SEA_LEVEL` + noise, camera-relative). Originally: keep `SEA_LEVEL = 200` for physics; keep vertex waves visual only until Gap 7 is closed. Do not claim shoreline waves or LOD distances beyond section 3 and the open gaps.
