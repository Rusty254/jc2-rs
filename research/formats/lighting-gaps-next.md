# Jungle lighting gap list: original vs rewrite (board f3772304)

Status: gap list with native evidence, plus the pieces implemented under this task (see "Implemented").
Evidence: `DX10_Shaders_F.shader_bundle` (sha256 `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`)
pixel and vertex shaders disassembled with `research/tools/shader_inventory.py --filter <names> --disasm-dir <dir>`
(names used: `vegetationbark`, `bark_water`, `vegetationfoliage`, `billboardfoliage`), the retail jungle models of
`vegetation_0.blz`, and the two user screenshots (`_extract/f3772304/ref/original_jungle.webp`, `rewrite_jungle.webp`).
Nothing below is tuned by eye; every number is a measurement or a quoted shader constant.

## 1. What was measured

The two screenshots are the same kind of place (tile 40,40 jungle), not the same pixel position, so only whole-frame
statistics are compared (640 x 360 downscale, gamma-encoded luma, `scratchpad/lighting/stats.py`):

| Frame | luma mean | median | p10 | p90 | p99 | share < 0.1 | mean saturation |
|---|---:|---:|---:|---:|---:|---:|---:|
| original (user, `10.webp`) | 0.136 | 0.099 | 0.060 | 0.244 | 0.810 | 51 % | 0.466 |
| rewrite (user, `11.webp`) | 0.255 | 0.202 | 0.072 | 0.526 | 0.804 | 22 % | 0.464 |
| rewrite, hero in grass, default flags | 0.237 | 0.199 | 0.090 | 0.426 | 0.823 | 13 % | 0.642 |
| same spot, `--sun-shadows` | 0.175 | 0.140 | 0.015 | 0.361 | 0.803 | 36 % | 0.602 |

Reading: the sunlit pixels agree (p99 0.81 vs 0.80: the sun term is right), the saturation of the whole frame agrees, but the
**shade is twice as bright in the rewrite** (median 0.202 vs 0.099, p90 0.526 vs 0.244). The rewrite has too little shadow, not too
much sun. With cascade shadows on the median moves to 0.140 (toward the original) but the darkest decile falls to 0.015 (original
0.060): the shadowed pixels lose the shadow floor (Gap G2).

## 2. Findings (gap list)

Each entry: native evidence, what the rewrite does, status. "Implemented" entries are in section 3.

### G0 (implemented). VegetationBark 32-byte vertices were decoded in the wrong order

* Native: VS `vegetationbark` (and `bark_water`) inputs are `v0 = (x y z, w)`, `v1 = (a, b, u, v)`. Outputs: `o6 = frc(v0.w * (1, 1/256, 1/65536))`
  (the vertex colour), `o9` from `v1.x` (the normal), `o7` / `o8` from `v1.y` (the tangent and `sign * cross`, the sign is the sign of `v1.y`),
  `o1.xy = v1.zw` (the UV). PS `vegetationbark`: `diffuse = t0 * v6`, `L = N.L * shadow * (t11.x / 2 + 1/2) * cb0[3] + t11.x * hemisphere(cb0[10], cb0[11], N.y)`,
  then `cb0[13]` fog. **No specular, no point lights, no reflection** in this pixel shader.
* Rewrite before: `rbm.rs` read the 32-byte layout (`flags & 4`, every Jungle trunk) as `x y z, normal, tangent, bitangent, u, v`, i.e.
  the **colour float was decoded as the normal** and the normal as the tangent. Measured on `Jungle_T02_CanopyM_LOD1-Whole.rbm`, four bark blocks:
  the 4th float agrees with the geometric normal at median |dot| 0.44 to 0.56 (random), the 5th at 0.87 to 0.997, the 6th at 0.04 to 0.40.
  The decoded colour spans 0.28 to 1.0 (block means 0.62, 0.74, 0.90, 0.98: baked darkening, most on the moss block). The colour was dropped.
  Bevy's GGX highlight (F0 0.04) was on top. Result: speckled, bright, tan trunks (the user's screenshot).
* Not changed: the 40-byte layout (`flags & 4 == 0`, Arctic pine): its normal slot (7th float) checks out (|dot| 0.99), its 10th float is orthogonal
  to the normal (0.05) and is read as the bitangent; whether it is a colour and which VS the 40-byte layout feeds is unresolved (Gap G9).

### G1. Sun cascade shadows are off in the default run

* Native: every vegetation / bark / General pixel shader carries the cascade block gated by `cb4[3].x` (3 cascades as vertical tiles of the atlas `t12`,
  12-tap rotated Poisson PCF with 13 blocker taps for cascade 0 and 4-tap PCF for cascades 1 and 2 in soft mode `cb4[3].y`, the Chebyshev edge distance
  fades over 3.9..5, listing in `vegetationbark` PS lines 39 to 251; same text in `bark_water`, `vegetationfoliage`). Gap of the option's default and the matrices:
  `general-terms-next.md` section 4 and its Gap 2.
* Rewrite: `SkyConfig::sun_shadows` is false unless `--sun-shadows` is passed (`sky.rs`, `main.rs`); the user's screenshot has no tree shadows at all.
  With the flag the frame statistics move toward the original (table above) at a frame cost of 16.7 ms to 28.3 ms on the reference machine.
* Status: open (task filed). The original screenshot proves cascade shadows are active in normal play; the cascade splits, atlas size and the soft-mode
  default are unresolved, so Bevy's cascade configuration (4 cascades, 25 m first bound, 800 m) stays labelled non-native.

### G2. Cascade shadow floor missing outside the General route

* Native: the cascade term ends `c = sat(sat(d - 4) + lit) * cb0[9].z + cb0[9].w` with `cb0[9] = (1, 0, 1 - f, f)`, `f = 0.1` (`general-terms-next.md` section 4,
  inferred default). So shadowed pixels still receive 10 % of the sun. The General route applies it (`general_main.wgsl` `sun_terms`: `0.1 + 0.9 cascade`);
  terrain (`terrain_splat.wgsl` `sun_correction`), the hemisphere models (bark, Lambert, ...), foliage, grass and the forest canopy do not.
* Effect: with `--sun-shadows` the shadowed decile is 0.015 against the original 0.060 (table above).
* Status: open (task filed), blocked on G1 being enabled by default to be visible.

### G3. Plain (hemisphere) models skip the Sun's horizon shadow and the `(AO / 2 + 1/2)` factor

* Native: `vegetationbark` PS lines 252 to 271: `shadow = max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w) * cascade`, Sun term `* (t11.x / 2 + 1/2)`.
  The same factor is in `bark_water`, `billboardfoliage` (lines 27 to 36) and `vegetationfoliage` (lines 275 to 290); `ambient-hemisphere-next.md` Gap 2.
* Census (`research/tools/sun_terms_probe.py`, exit 0, all 600+ pixel shaders by family; heuristic text match on the disassembly): the **horizon term is present in every
  family that samples `t11`** (vegetationbark, bark (59 of 59), foliage (76 of 76), lambert (36 of 36), lambertflatnormal, carpaint (291 of 298), skinnedgeneral (52 of 52), facade,
  merged (8 of 8), window, windowsimple, deformablewindow, splineroad, box, decalskinned, general, grass, fastter, billboard and tree impostors); DecalSimple and DecalDeformable
  sample no `t11`. The **`(t11.x / 2 + 1/2)` Sun factor** is present only in bark (59 of 59), vegetationbark, bark_water, foliage (76 of 76), vegetationfoliage, billboardfoliage (5 of 7),
  grass, fastter (39 of 39), the tree impostors and one of the two splineroad shaders; it is **absent** from lambert, carpaint, skinnedgeneral, facade, merged, window, general and box.
  The cascade block (`cb4[3].x`) is in all the lit families above except billboard, impostor and grass-less ones.
* Rewrite: `hemisphere_main.wgsl` multiplies only the ambient by `t11.x`; the Sun is Bevy's, unscaled. Valleys and canopy hollows keep full sun.
* Status: open (task filed).

### G4. VegetationFoliage is lit by Bevy PBR, not by the native model

* Native PS `vegetationfoliage` (lines 275 to 337), terms as read from the disassembly:
  `n = normalize(lerp(basis * (t1.rgb - 0.5), geometric N, cb1[0].y))` (two-sided: flipped by the front-face flag), `t2.r` (the `*_aoc.dds`, UV1) scales the ambient;
  ambient `= cb0[12].xyz * t2.r * t11.x` (note `cb0[12] = cb0[10] + cb0[11]`, no 0.5 here), Sun `= sat(N.L) * S * cb0[3]` blended toward a back-lit term
  `(1 - t1.a) * S * cb0[3] * (0.5, 0.6, 0.15)` by `(1 - cb1[0].y) * cb1[0].x * f(N.L * N.V)` (`f = clamp(10 x - 2, -1, 1) / 2 + 1/2`), `S = shadow * (t11.x / 2 + 1/2)`;
  plus the VS point-light sum `v10.xyz`; a final saturated mix toward `light * (0.508, 0.54, 0.645)` (lines 300 to 307, not interpreted here);
  specular `cb0[1] * cb1[1].x * pow(max(N.H, 0.001), cb1[1].y) * max(S', 0.3) * (1 - cb1[0].y)`; fog `cb0[13]`, underwater `cb0[14]`.
  Header floats of the retail model (`params`): jungle liana `[0.15, 25.0, 1.6, 1.8, 1.0]` / `[0, 0, 1.6, 1.8, 1.0]`; their map onto `cb1[0]`, `cb1[1]` (writer `FUN_00971da0`)
  is the open part (`material-parameters-next.md` line 90).
* Rewrite: `foliage_main.wgsl` is `apply_pbr_lighting` (GGX highlight, no translucency, no `t2` ambient scale, no horizon/AO). The bright speckled leaf highlights
  in the rewrite screenshot (hero in jungle grass, `_extract/f3772304`) are Bevy's specular lobe on the leaf normal map.
* Status: open (task filed).

### G5. BillboardFoliage (canopy sprites) lighting

* Native PS `billboardfoliage` (45 lines, complete): `rgb = tex * v4.yyy`, `light = cb0[3] * (sat(v2.w + 1) * horizon * cascade-free * v4.x * (t11.x / 2 + 1/2)) + 0.5 * cb0[12] * t11.x`
  (no N.L, no normal), alpha `4 * tex.a * v4.z`, fog `cb0[13]`. Rewrite: legacy Bevy lighting of a card with a fixed normal. Status: open (task filed).

### G6. Terrain, grass, forest canopy, sea: verified native elsewhere

* Terrain splat, the forest canopy layer (JC2-161), grass (JC2-133), the sky ambient / sun colours (`night-light-next.md`, JC2-122), the fog (JC2-155), the post chain with auto-exposure
  (JC2-134/137/138) are implemented from native evidence; this task found no contradiction. Not re-measured here.

### G7. Green-tinted bounce, haze shafts, contact shadows in the original

* No native term found for a bounce light or for light shafts. What reads as green bounce in the original is the trunk / leaf albedo times the (now correct)
  hemisphere ambient and `t2` scale; the "air light" is the exp fog (`cb0[13]`) plus the canopy layer. No extra native pass was found, so none is invented
  (the `bark_water` / foliage PS contain no scattering term).
* Soft contact shadows are the cascade PCF (G1) plus the AO map factor (G3).

### G8. Rewrite red-leaf saturation

* Not explained by a lighting term: the red leaves are `VegetationFoliage` / `General` cards whose colour is the texture. The foliage translucency colour
  `(0.5, 0.6, 0.15)` and the mix toward `(0.508, 0.54, 0.645)` (G4) would desaturate them; unproven until G4 is implemented.

### G9. VegetationBark 40-byte layout and the `bark*` shader permutations (board 646639f2, resolved by `research/tools/bark_permutation_probe.py`)

**Layout (a).** The 40-byte layout (`flags & 4 == 0`) is the input layout of the `_dirt` vertex shaders: `barkpl0_dirt`, `barkpl0_dirt_lowresshadows`, `barkpl0_snow_dirt*`,
`bark_dirt_water` take `v0 = (x y z)`, `v1 = (u0 v0 u1 v1)`, `v2 = (packed normal, packed tangent, packed bitangent)` = 12 + 16 + 12 bytes. The parser order is right and the 10th
float **is the bitangent**, three independent packed vectors (`mul v2.x/y/z * (1, 1/256, 1/65536)`, `frc`), no handedness bit. There is **no colour** in this layout: `o6.xy`
is the second UV set (`v1.zw`), where the 32-byte VS puts the colour. The existing test `retail_bark_signed_basis_matches_its_authored_bitangent_stream` (agreement 0.95) also
measures it from the data. Our tangent sign is derived from the sign of the tangent float; the shader needs none (inferred approximation, passes the test).

**Dirt pixel shader.** The `_dirt` PS samples four textures: `t0` bark diffuse and `t1` bark normal at UV0 (`v1.xy`), `t2` dirt diffuse (alpha = blend mask) and `t3` dirt normal
at UV1 (`v6.xy`). `diffuse = mix(t0, t2.rgb, t2.a)`; normal = `normalize(n0.xy / |n0| + 2 * t3.xy - 1, 2 * t3.z - 1)` with `n0 = 2 * t1 - 1`; the rest (hemisphere, Sun, cascades)
equals `vegetationbark`; the vertex colour multiply is absent. In the retail models the four material slots are filled exactly as the shader reads them:
`diffuse, normal, properties(=dirt diffuse), deformed_diffuse(=dirt normal)`, e.g. `jungle_canopy_bark2_dif/nrm + jungle_t08_trunk_dif/nrm`, `desert_rockpillar_* + desert_formation_*`.
Retail census (`jc2tool scan-rbm`): 1151 `VegetationBark` blocks: 454 (flags 0) + 12 (flags 2) + 6 (flags 8) are 40-byte/dirt, 663 (flags 4) + 16 (flags 6) are 32-byte; all have slots 0 to 3 filled.
The renderer currently ignores `t2`, `t3` and UV1 for these blocks (the Arctic pine and the Jungle understory trunks): the dirt blend is a visible gap.

**Permutation selection (b)**, native `CRenderBlockBarkType` constructor `0x952530` (builds the name tables) and block draw `0x94b840` (picks the entries):

* Vertex shader `Bark` + `PL0` + `_Snow` (bit 1) + `_Dirt` (bit 2) + `_LowResShadows` (bit 4), table index = the 3 bits (`sprintf "Bark%s%s%s%s"`); water: `Bark_Water`, `Bark_Dirt_Water`.
* Pixel shader table `[pl][snow | dirt << 1][cascade]`, names `Bark` + `PL0 | PL2 | PL4 | (none)` + `_Snow` + `_Dirt` + `SH1 | SH2 | (none)`; water `Bark<pl>_Water`, `Bark<pl>_Dirt_Water`.
* Per block (from the draw function): **dirt = `layout flags & 4 == 0`** (so the `_dirt` VS + PS go with the 40-byte layout; this is the only reason the layouts differ); **water = `layout flags & 2`**
  (the `bark_water` path: `Bark_Water` / `Bark_Dirt_Water`, blends to `cb0[14]` by `v10.x` after the underwater test `v10.y < 0`; flags 2 and 6 are the 12 + 16 Coral blocks); **snow** = the
  0 lights `PL0`, 1 or 2 `PL2`, 3 or 4 `PL4`, 5 to 7 the `(none)` variant; **cascades** = the number of the 3 shadow cascades the instance overlaps (`0x956f70`, bits 27 to 29 of the same word):
  0 or 1 `SH1`, 2 `SH2`, 3 the full PS, and the `_LowResShadows` VS when fewer than 2 (it emits one shadow coordinate instead of three; `sh1` reads it).
* `pl2` / `pl4` add 2 / 4 point lights (`cb1[2 + 2k]` position + 1/r^2, `cb1[3 + 2k]` colour; uploaded by `0x957070`: 8 floats each); the `(none)` family replaces them by the light grid (`t4`, `cb0[145]`), i.e. it
  is the clustered path for 5 or more lights. **Native quirk read in the constructor:** the combination `(none)` + no snow + no dirt + full cascades is named `VegetationBark`, whose PS is
  identical to `barkpl0` (no plain `Bark` PS exists in the bundle), so that one case has no light grid. `bark_only_ao / diffuse / normals(_dirt)` are debug views no table reaches.
* Snow constants uploaded to `cb1[0]` before the draw (draw function `0x94b840`): snow amount `f = byte / 255`; for `f < 0.5` the colour is `(0.508, 0.54, 0.645)` and the threshold `1 - 2 f`; for `f >= 0.5` the colour is
  `(0.2932, 0.2011, 0.1247)` and the threshold `1 - 2 (f - 0.65)`. The PS blends that colour in where `dot(N, (-0.2, -0.2, 0.6)) - threshold` is positive (`mul_sat(.. , 5)`). The colour `(0.508, 0.54, 0.645)` is the
  one of G4's foliage mix; why a second brown colour exists (wet / mud?) is unknown.

Open (see Gaps below): the per-instance bytes and bits above come from the instance/draw-item structure whose writer was not traced (who sets `+0xc`, the light count and the cascade bits); `flags & 8` (6 desert blocks)
is read by nothing in the draw function; the `sh1` / `sh2` pixel shaders read `cb1[1].x` in their cascade code and every bark PS writes `cb1[1].w` as the alpha; the writers of `cb1[1]` are untraced.

## 3. Implemented under this task

All captures are in `_extract/f3772304/` (`before_*` / `after_*`), listed in `_extract/timeline/index.jsonl`. Branch commits d6f134f .. 2566ae9.

| # | Piece | Files | Evidence |
|---|---|---|---|
| 1 | VegetationBark 32-byte layout fixed (normal and tangent right), vertex colour applied, no specular | `jc2-formats/src/rbm.rs` (`parse_vegetation_bark`, `vegetation_bark_colour`), `jc2-game/src/rbm_loader.rs` | G0; tests `jungle_vegetation_bark_32_byte_layout_normal_follows_geometry`, `vegetation_bark_colour_is_the_vertex_shader_fraction_chain`; capture `jungle-trunks` (tile 40,40, free camera 2 m above ground, yaw 0, 10:00): trunks lose the tan speckle, take the baked darkening and smooth shading; whole-frame mean luma 0.373 to 0.352, median 0.335 to 0.305 |
| 2 | Sun term of plain (hemisphere) models and terrain: horizon shadow (all families that sample `t11` but the decals), `t11.x / 2 + 1/2` for bark only, cascade floor `0.1 + 0.9 c` on the hemisphere route and on terrain | `hemisphere_main.wgsl` (`sun_correction`), `hemisphere_material.rs` (`reads_sun_horizon`, `sun_uses_ao_half`, `JC2_NO_HEMISPHERE_SUN=1` to switch off), `terrain_splat.wgsl` | G2, G3 and `sun_terms_probe.py`; captures `hemisphere-sun-trunks`, `hemisphere-sun-grass` (both with cascade shadows on): the darkest decile rises (0.064 to 0.069 and 0.028 to 0.034), mean unchanged: a small, correct term. The floor value 0.1 is **inferred** (`general-terms-next.md` section 4) |
| 3 | Cascade sun shadows on by default in the world viewer (`--no-sun-shadows` opts out; `--sun-shadows` stays accepted) | `main.rs`, `README.md` | G1; captures `sun-shadows-forest`, `sun-shadows-grass`: forest median luma 0.295 to 0.182 (original 0.099), grass 0.225 to 0.142; frame time 16.7 ms to 20.1 ms (forest) and 17.2 to 19.4 ms (grass). The cascade splits (4 cascades, 25 m first bound, 800 m) are Bevy's, not native (unresolved: `general-terms-next.md` Gap 2) |

Unproven in piece 1: the native vertex colour is applied in linear space as the General route does (`Mesh::ATTRIBUTE_COLOR`); the native PS multiplies the sRGB-decoded
`t0` by `v6` directly, which is the same space. Unproven in piece 2: the hemisphere shader was verified by running the capture (it compiles and the effect is the expected sign);
there is no naga unit test for `hemisphere_main.wgsl` (the terrain shader has one).

After pieces 1 to 3 the forest scenario (hero, tile 40,40, +200,-300) reads median luma 0.182, p10 0.079, p90 0.406 against the original's 0.099 / 0.060 / 0.244: the
remaining excess is in the mid and upper tones, i.e. lit foliage (G4, G5).

## 3b. Floor, horizon and AO factors on grass, impostors, foliage, billboards, roads (board 8c156938)

Disassembled with `shader_inventory.py --filter "^(grass|treeimpostor|treesingleimpostor)"` and `--filter splineroad`.

| Route | Native pixel shader terms | Rewrite after this task |
|---|---|---|
| Grass (`grass`, `grass_lod0/1/2`, PS 0012 to 0015) | `light = v2.rgb * t11.x + v3.rgb + cb0[3] * (sat(avg * 0.9 + 0.1) * horizon * v2.w * (t11.x / 2 + 1/2))`; the cascade block (4-tap PCF) is present in all four | `grass.wgsl`: ambient `diffuse_occlusion *= t11.x`, Sun correction `horizon * (t11.x / 2 + 1/2) * (0.1 + 0.9 cascade)` (shared `sun_correction.wgsl`) |
| Tree impostors top / trunk (`treeimpostortop`, `treeimpostortrunk`, single variants) | Sun `cb0[3] * sat(half-lambert or N.L) * horizon * (t11.x / 2 + 1/2)`, ambient `t11.x * cb0[12] * 0.5` (top) or the hemisphere lerp (trunk); **no cascade block** | `impostor.wgsl`: same two factors; Bevy's cascade is cancelled (`cascaded = false`) because the native shader has none |
| VegetationFoliage, BillboardFoliage | already native in `foliage_main.wgsl` (`native_light`, `billboard_light`): horizon, `t11.x / 2 + 1/2`, floor 0.1 (foliage), no cascade (billboard) | unchanged; verified against the shader text above |
| SplineRoad | two pixel shaders: `splineroad` (PS index 2, lit, has horizon, cascade floor and `t11.x * 0.5 + 0.5` on the Sun) and `splineroadsimple` (PS index 3, 25 lines: a constant grey 0.8 with only the alpha test; no `t11`, no lighting). The earlier census counted both under the prefix. | resolved: the lit variant is the one that lights the road, so `sun_uses_ao_half("SplineRoad")` is now true (`hemisphere_material.rs`) |

Impostor and grass statistics, tile 40,40 at 640 x 360 gamma luma, before (master) / after, same camera (`_extract/8c156938`):
forest +200,-300 10:00 mean 0.181 / 0.182, median 0.137 / 0.138, p10 0.049 / 0.049; open ground 10:00 mean 0.306 / 0.307, p10 0.075 / 0.078;
open ground 07:30 (low Sun, long horizon shadows) mean 0.136 / 0.139, median 0.131 / 0.129, p10 0.027 / 0.045, p90 0.227 / 0.221, p99 0.541 / 0.521.
The change is small at noon and visible at low Sun, as the terms predict (horizon shadow pulls the lit tail down, the floor and the AO map lift the darkest decile).

Unproven: the grass Sun's `N.L` is the vertex shader's `v2.w` (not decoded), the rewrite keeps Bevy's `N.L` on the terrain normal; the impostor top/trunk
vertex normals (half-lambert on a vertex normal) are not modelled, the cards keep their up normal. The floor 0.1 stays inferred (see section 2, G2).

## 4. Remaining pieces (filed as board tasks, tags `lighting`, `parity-gap`)

* G4 native VegetationFoliage shading. New evidence: across the 111 `VegetationFoliage` blocks of `vegetation_0.blz` the first two header floats are one of
  (0.15, 15), (0.15, 10), (0.15, 25), (0.05, 8), (0.1, 20) or (0, 0): the plausible reading is specular amplitude and power (`cb1[1].xy` of the PS), (0, 0) = no highlight (pines).
  The other three floats are constant (1.6, 1.8, 1.0) and cannot be `cb1[0].xy` as they stand (`1 - cb1[0].y` would be negative), so the `cb1[0]` source
  (translucency weight and the normal blend) stays unresolved; the writer `FUN_00971da0` uploads the VS cb1 (wind / fade), not these.
* G5 BillboardFoliage: PS complete (above); the VS (`billboardfoliage`, 128 lines) builds the camera-facing corner offset in the shader and a fake sphere light
  `o4.x`, a pivot-distance ramp `o4.y`, a fade `o4.z`. The Rust side draws the cards as static quads in the model's XY plane (`rbm.rs` BillboardFoliage), so both the facing and the
  lighting are non-native. Needs a vertex shader on the material (and the prepass).
* Cascade floor on foliage, grass and billboards (only the hemisphere, General, skinned and terrain routes have it now).
* G9 resolved (above). Dirt blend DONE (board 6d65ed04): `hemisphere_main.wgsl` mixes `t0` with the dirt diffuse by its alpha and rebuilds the normal as `normalize(n0.xy/|n0| + 2 t3.xy - 1, 2 t3.z - 1)` (the native PS lines 264 to 277; the bark `n0` is recovered from the mapped N in the TBN, no Y flip on this material, so equal to the texture's own); loader `is_bark_dirt` / `set_dirt` (`JC2_NO_BARK_DIRT=1` for before captures), test `retail_40_byte_bark_blocks_carry_the_dirt_blend` (Arctic pine trunk), capture `arctic-pine-trunk`. The coral `Bark_Dirt_Water` underwater blend is not done. Remaining pieces: the bark snow permutation, and the bark point-light permutations (`pl2`, `pl4`).
