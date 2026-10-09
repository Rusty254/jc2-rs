# Haze presets, the fog colour `cb0[13]` and the fog factor of the vertex shaders (JC2-116)

Status: research brief. No runtime code changed. Probe: `research/tools/haze_fog_probe.py` (71 checks, exits 0; needs a private
`jc2tool` for the data checks, they report SKIP without one). All addresses are runtime addresses of `JustCause2.exe`
(no ASLR). Pseudo-C was read from `research/analysis/pseudo`; every function the claims rest on is pinned by a sha256 of its
bytes, the call sites and the arithmetic that the formulae depend on are byte-checked, and the shader claims are re-disassembled
from `DX10_Shaders_F.shader_bundle` by the probe.

## Headline

1. **The static presets are not constants and not a settings colour: the two colours come from the game-zone gradient picture.**
   The haze object `H = cloud + 0x44` (`cloud = [W + 0x4cc]`) has eight 9-float preset groups (+0x58 .. +0x174: rgba and five
   numbers). The constructor `FUN_00a3f8b0` sets every colour to (1,1,1,1) and the numbers to (0, 32000, 0, 0, 100). Then:
   * the **five numbers** of the six groups at +0xa0, +0xc4, +0xe8, +0x10c, +0x130, +0x154 are read from
     `settings\landscape_effects.bin` (section `19a8fc4a`: `day_params`, `night_params`, `rain_day_params`, `rain_night_params`,
     `snow_day_params`, `snow_night_params`, plus four hour windows at +0x178..+0x184), by `FUN_00a4d1b0` (section 2);
   * the **two colours** that matter, `H+0x58` (P1) and `H+0x7c` (P2), are overwritten **every frame** by `FUN_00a55cd0`
     (called from `FUN_00a66c30`) with **rows 2 and 3 of the game-zone gradient** `gamezone_gradients.bmp` (the same smoothed
     state that gives the sun and ambient, `night-light-next.md`). The colours of the other six groups are never used.
2. **Clear-weather fog colour** (`cb0[13].rgb`) = `lerp(row3, row2, bc)`, where `bc = (1 + fwd_xz . toSun_xz/|toSun_xz|) / 2`:
   row 2 toward the sun, row 3 away. **Hour enters only through the gradient rows**; the haze tables `A`/`B` variant 0 are
   computed and **never read**. `cb0[13].w` is the **density** `p2`, not an alpha.
3. **`cb0[14]` (tables `E`/`F` variant 0) is the water colour** of fog layer 1 (density 0.3 above y = 200, 0.15 below), not a haze
   colour. This is why `E`/`F` alone are a dark teal at noon (JC2-139): they were never the horizon haze.
4. **The fog factor** is computed in the vertex shader (91 vertex shaders carry the same seven instructions) as a **base-2
   exponent** `e = (d - p0)/(p1 - p0) + sat(d * p2) * (1 - min((y - p3)/p4, hc))`, `hc = sat((camY - p3)/p4)`,
   `d = clip.w - clip.z` (about the eye depth), `y` the world height (sea level is y = 200). The pixel shader does
   `out = fog + (lit - fog) * min(2^-e, 1)`. `p0..p4` are the five numbers above, lerped between night and day by a smooth
   window (day 200 m / 12 km / 0.001, night 300 m / 9 km / 0.007).
5. `fog_gradient.dds` is a **horizon band texture**, not part of the distance fog: a `CRenderBlockFogGradient` ring
   drawn at the far plane whose pixel alpha is the texture alpha (section 6).

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `DX10_Shaders_F.shader_bundle` (7,582,748 B) | `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98` |
| `settings\landscape_effects.bin` (2,637 B) | `43fa8c178318d1aadc48d531f5a619f558137c9ea5a7e2093f4476a7d7caa2fd` |
| `textures\landscape\gamezone_gradients.bmp` | `b43e8c1b56f53a981861ee0377c26ab5f2d151f800fcec73495601b3f1fcb329` |
| `textures\atmosphere\gradients.bmp` | `b66bf51075ed538b5a3fe72bc30c3ccd52e1cfcb0b618bde478425864648a45d` |
| `textures\atmosphere\fog_gradient.dds` (640 B, DXT5 4 x 128) | `e5a877728b77191c13e975112476555b03c15731ae833a3d541e526c5c7a0809` |

Function pins (sha256 of the bytes, in the probe): `FUN_00a432d0` (0xe76), `FUN_00a3f8b0` (0x362), `FUN_00a4d1b0` (0x192),
`FUN_00a55cd0` (0x71), `FUN_0096c800` (0x7c), `FUN_0095b150` (0x20d), `FUN_00a40080`, `FUN_00a3fc60`, `FUN_00a3fc20`,
`FUN_00a654b0`, `FUN_00a472f0`, `FUN_00a66f50`, `FUN_00a63650`, `FUN_00a44ad0`, `FUN_00a49ec0`, `FUN_00a73fc0`.

## 1. Object model and call chain (byte-verified)

_(decompiler excerpt omitted from the public snapshot)_

* `H = [[manager+0x2c] + 0x4cc] + 0x44`: `FUN_00a73fc0` (the one-shot settings load, called at start-up from `FUN_00a74f80` and on a
  reload flag from `FUN_00a751b0`) builds exactly this `ecx` before `FUN_00a4d1b0` (bytes pinned, 0x00a7408d).
* `FUN_0096c800` has **one** caller (`FUN_00a432d0`), `FUN_00a55cd0` one caller (`FUN_00a66c30`), `FUN_00a432d0` one caller
  (`FUN_00a654b0`); `FUN_00a654b0` one caller (`FUN_00a472f0`) (Ghidra call table; vtable calls are not in it).
* `H[0..3]` (dword indices) are the table pointers `A` (+0x660), `B` (+0xb10), `E` (+0x1920), `F` (+0x1dd0) of the weather object
  (`light-tables-next.md` section 4, bytes already pinned there). `H[2]` (E) is used when the camera is at or above y = 200,
  `H[3]` (F) below.
* Weights: `w_rain = (1 - snow) * P.+0x48`, `w_snow = snow * P.+0x48` (`FUN_00a40080`, `light-tables-next.md` section 3).

## 2. The preset groups: layout and sources

Nine floats per group: `r g b a p0 p1 p2 p3 p4`. Offsets are from `H` (cloud + 0x44).

| Group | Offset | Role in `FUN_00a432d0` | Source of the colour | Source of p0..p4 |
|---|---|---|---|---|
| P1 | +0x58 | **toward-sun fog colour** (weight `bc`) | **`FUN_00a55cd0`, gradient row 2, every frame** | unused |
| P2 | +0x7c | **away-from-sun fog colour** (weight `1 - bc`) | **`FUN_00a55cd0`, gradient row 3, every frame** | unused |
| day | +0xa0 | clear, day | unused (ctor 1.0) | `day_params` |
| night | +0xc4 | clear, night | unused | `night_params` |
| rain day / night | +0xe8 / +0x10c | rain weights `w_rain` | unused | `rain_day_params` / `rain_night_params` |
| snow day / night | +0x130 / +0x154 | snow weight `w_snow` | unused | `snow_day_params` / `snow_night_params` |

* The "current" group `H+0x10 .. +0x30` (`H[4..0xc]`: rgba + p0..p4) is the working copy: constructor defaults
  colour (0.8, 0.9, 1.0, 0), numbers (0, 20000, 0.0002, -150, 300); the water group `H+0x34..+0x54`: colour (0.34, 0.64, 0.64, 0),
  numbers (0, 60, 0.2, 193, 7) (water density `H[0x13]` is replaced every frame by 0.3 / 0.15). All decoded from the constructor
  bytes by the probe. After the first frame the working colour is replaced by the P1/P2 mix; only its five numbers matter.
* `FUN_00a4d1b0` reads (hash keys from `DAT_00f0eaf8..1c`, probe-checked): section `19a8fc4a` of `settings\landscape_effects.bin`;
  windows `220137a5`, `88d36ba1`, `f56ffcb7`, `f10e7ec1` into `H+0x178, 0x17c, 0x180, 0x184`; per group the numbers
  `6f526f85` (p0), `532808b7` (p1), `7b425f7a` (p2), `d5090a62` (p3), `e8422ea1` (p4). Constructor defaults for the windows
  (8, 17, 19, 5) are overwritten by the file.

Retail values (`jc2tool prop settings\landscape_effects.bin`, probe-asserted):

| Window | Key | Value (h) | | |
|---|---|---|---|---|
| `H+0x184` (night ends / dawn starts) | `f10e7ec1` | 4.0 | | |
| `H+0x178` (day starts) | `220137a5` | 5.5 | | |
| `H+0x17c` (day ends / dusk starts) | `88d36ba1` | 18.5 | | |
| `H+0x180` (night starts) | `f56ffcb7` | 20.0 | | |

| Group | p0 start (m) | p1 end (m) | p2 density (1/m) | p3 | p4 |
|---|---|---|---|---|---|
| `day_params` | 200 | 12000 | 0.001 | -1806 | 3006 |
| `night_params` | 300 | 9000 | 0.007 | -1800 | 4000 |
| `rain_day_params` | -50 | 4869 | 0.005 | -200 | 800 |
| `rain_night_params` | 0 | 1500 | 0.007 | -200 | 800 |
| `snow_day_params` | 0 | 3000 | 0.01 | -1806 | 6000 |
| `snow_night_params` | 0 | 3000 | 0.01 | -1800 | 6000 |

(The names p0..p4 come from the use in section 4; the file only has hashes for the keys.)

## 3. `FUN_00a432d0`: the exact computation (pseudo-C reading, function hash and key arithmetic byte-pinned)

Inputs: hour `h` = `CRenderEngine+0xac` (0..24); camera object `[DAT_011919f4 + 0x10]` (position y at +0x58, matrix row at
+0x44..+0x4c); `L` = the argument = `S+0x250` = -(unit direction to the sun), stored by `FUN_00a58a00` after the lightning override and before the
moon flip (so it is the **sun**, never flipped to the moon); `w_rain`, `w_snow`.

**Day factor** `t` (windows `w61 = 4`, `w5e = 5.5`, `w5f = 18.5`, `w60 = 20`):

```
h <= 4 or h >= 20        t = 0                       (night group)
4 < h < 5.5              t = sat((h - 4) / 1.5)      (dawn)
5.5 <= h <= 18.5         t = 1                       (day group)
18.5 < h < 20            t = 1 - sat((h - 18.5) / 1.5)  (dusk)
```

**Numbers**: `p = lerp(night, day, t)`; then if `w_rain > 0`: `p = lerp(p, lerp(rain_night, rain_day, t), w_rain)`; then if
`w_snow > 0` the same with the snow groups (`FUN_00a3fc60` does `a + (b - a) * t`). All five numbers blend linearly.

**Colour**: `bc = (dot + 1) / 2`, `dot = (r.x * L.x + r.z * L.z) / sqrt(L.x^2 + L.z^2)` (if `L.x^2 + L.z^2 < 1e-6` the divisor is 1), where
`r` is the camera matrix row at +0x44..+0x4c. Row +0x44 is **minus the forward vector** (`underwater-render-next.md` section 5/6) and
`L = -toSun`, so `dot = fwd_xz . toSun_xz / |toSun_xz|` and `bc = 1` when the camera looks along the sun's azimuth, 0 opposite,
0.5 sideways or straight up/down (only the horizontal part of the forward vector counts, it is **not** renormalised).

```
col      = lerp(P2, P1, bc)                          # clear: rows 3 and 2 of the zone gradient (variant 0 of A/B unused)
if w_rain > 0:  col = lerp(col, lerp(B_v1(h), A_v1(h), bc), w_rain)
if w_snow > 0:  col = lerp(col, lerp(B_v2(h), A_v2(h), bc), w_snow)
cb0[13] = (col.rgb, p2)        # FUN_0096c800 stores col at CRenderEngine+0xc8 and p0..p4 at +0xe8,+0xf0,+0xf8,+0x100,+0x108;
                               # FUN_0095b150 overwrites the alpha (+0xd4) with p2 (+0xf8)
```

`A_v1(h)` etc. are the hour-interpolated `gradients.bmp` tables (`light-tables-next.md` section 2). At noon: variant 1 (`A = B`)
(0.080, 0.082, 0.096), variant 2 (0.307, 0.410, 0.470); at midnight (0.045, 0.045, 0.064) and (0.034, 0.042, 0.046).
The rain variant is the **dark grey-blue** fog, the snow variant a **pale grey-blue**.

**Water layer** (`cb0[14]`, not the atmosphere): `E` (camera y >= 200) or `F` (below), variant 0, hour-lerped, blended toward variants
1/2 by the same weights (rows `+25` / `+50` entries); alpha slot replaced by 0.3 (y >= 200) / 0.15. See `water-consumers-next.md`.

### Fixtures: clear weather, game-zone gradient, intensity 3 (probe-asserted, linear scene colour, 1e-4)

Zone sets 2..7 (Desert, Jungle, Sea, City, Grass, Default) have **identical** rows 2 and 3 (probe); Arctic (set 1, the start base)
differs. Rows are the steady state of the 1 s smoothing.

| Zone / hour | row 2 = `bc` 1 (toward the sun) | `bc` 0.5 | row 3 = `bc` 0 (away) |
|---|---|---|---|
| Default 00 | (0.0278, 0.0718, 0.1275) | same | same |
| Default 06 | (0.4704, 0.1801, 0.1193) | (0.3062, 0.1687, 0.1584) | (0.1420, 0.1573, 0.1975) |
| Default 12 | (0.0000, 0.3204, 0.9406) | same | same |
| Default 18 | (0.4591, 0.0915, 0.0170) | (0.3230, 0.1393, 0.0856) | (0.1870, 0.1870, 0.1542) |
| Arctic 00 | (0.0677, 0.0846, 0.1036) | same | same |
| Arctic 06 | (0.4205, 0.2716, 0.2349) | (0.2986, 0.2259, 0.2092) | (0.1768, 0.1801, 0.1835) |
| Arctic 12 | (0.4368, 0.5542, 0.6320) | (0.4077, 0.5239, 0.6026) | (0.3787, 0.4936, 0.5732) |
| Arctic 18 | (0.3636, 0.2271, 0.1768) | (0.2719, 0.2036, 0.1768) | (0.1801, 0.1801, 0.1768) |

(Noon Default is a saturated blue; displayed through the final `sqrt` it is (0, 0.566, 0.970).)

Numbers `(p0, p1, p2, p3, p4)` and day factor over the windows (clear):

| h | t | p0 | p1 | p2 | p3 | p4 |
|---|---|---|---|---|---|---|
| 0, 4, 20, 23 | 0 | 300 | 9000 | 0.007 | -1800 | 4000 |
| 5.0 | 0.6667 | 233.33 | 11000 | 0.003 | -1804 | 3337.33 |
| 5.5 .. 18.5 (6, 12, 18) | 1 | 200 | 12000 | 0.001 | -1806 | 3006 |
| 19.25 | 0.5 | 250 | 10500 | 0.004 | -1803 | 3503 |

## 4. The fog factor in the shaders (verified from the bundle by the probe)

`FUN_0095b150` (per scene, bytes pinned) turns the stored layer into 8 floats per layer and `memcpy`s them to the **vertex** constant
block (`DAT_013daf80`, 28 vec4): layer 0 to **VS `cb0[12..13]`**, layer 1 to `cb0[14..15]` (destination `0x013daf80 + 16 * (12 + 2 * (layer != 0))`).
The pixel block (`0x013db140`) gets `cb0[13]` / `cb0[14]` = colour + alpha. For layer 0 with `camY` = `ctx+0xc54` (camera y):

```
VS cb0[12] = ( p0/(p1-p0),  p1,  1/(p1-p0),  camY )
VS cb0[13] = ( p3/p4,       sat((camY - p3)/p4),  1/p4,  p2 )
PS cb0[13] = ( col.r, col.g, col.b, p2 )
```

Vertex shader (identical in `general`, `fastterrainshader` and `merged`, probe-asserted line by line; 91 vertex-shader names in the
bundle contain the same sequence and none reads `cb0[12]` / `cb0[13]` otherwise):

```
d  = clip.w - clip.z                                   # "add r0.x, neg(r0.z), r0.w"
h  = world.y * cb0[13].z - cb0[13].x                   # (y - p3) / p4
e  = d * cb0[12].z + ( sat(d * cb0[13].w) * (1 - min(h, cb0[13].y)) - cb0[12].x )
   = (d - p0)/(p1 - p0) + sat(d * p2) * (1 - min((y - p3)/p4, hc))
```

Pixel shader: `out = cb0[13].rgb + (lit - cb0[13].rgb) * min(exp(-e), 1)` where SM4 `exp` is **2^x** (`exp r1.x, neg(v8.w)` /
`min r1.x, r1.x, l(1)` / `mad ..., cb0[13]`; `v2.w` in FastTerrain, `v4.w` in Merged, `v8.w` in General). So **transmittance
`T = min(2^-e, 1)`**, with no cap at 0: the fog never reaches the full colour by itself (it tends to 0 as d grows). The term `e` can be
negative before `p0`, where `T` clamps to 1.

* `d` is `w - z` of the reverse-Z projection (`underwater-render-next.md` section 6): `d = far * (t - near) / (far - near)` for eye depth `t`,
  i.e. **eye depth minus about `near`** (0.1 m with the camera-manager defaults), not radial distance. Near/far in play: Gap 2.
* Height: `y` is the **absolute world height; the sea surface is y = 200**. The density term uses `min(h_vertex, hc)`, so the haze
  is thickest low (h small) and absent above `p3 + p4` (day y = 1200 = 1000 m above the sea). `hc` is the camera's own height factor:
  looking up from low ground still sees `1 - hc`.
* The density ramp `sat(d * p2)` is 1 beyond `1/p2` (1000 m by day, 143 m at night); beyond it `e` is linear in `d`
  (`T` is a pure exponential with rate `ln 2/(p1 - p0)` per metre times a constant `2^-(1 - hmin)`).

Fixtures (camera and ground at y = 202, clear weather, `T = min(2^-e, 1)`; probe-asserted):

| h | d (m) | e | T |
|---|---|---|---|
| 12 | 100 | 0.0247 | 0.983 |
| 12 | 1000 | 0.3998 | 0.758 |
| 12 | 5000 | 0.7388 | 0.599 |
| 12 | 20000 | 2.0100 | 0.248 |
| 0 | 100 | 0.3267 | 0.797 |
| 0 | 1000 | 0.5800 | 0.669 |
| 0 | 5000 | 1.0397 | 0.486 |
| 0 | 20000 | 2.7639 | 0.147 |

VS constants at y = 202: noon `cb0[12] = (0.016949, 12000, 8.4746e-05, 202)`, `cb0[13] = (-0.600798, 0.667997, 0.000333, 0.001)`;
midnight `(0.034483, 9000, 0.000115, 202)` and `(-0.45, 0.5005, 0.00025, 0.007)`.

## 5. Where the colours of other consumers come from (status)

* The terrain, objects, vegetation, vehicles, particles, cirrus/clouds, grass, water shaders (628 pixel shaders read `cb0[13].xyz`)
  all mix toward the **same** `cb0[13]`; there is no per-material fog colour.
* The sky dome (`skygradientshader`) reads no `cb0` register at all (its colours come from its own constants, Gap 5).

## 6. `fog_gradient.dds` and the `FogGradient` block

* `CRenderBlockFogGradient` (type created by `FUN_00a40c70`, instance by `FUN_00a49ec0`, drawn by `FUN_00a44ad0`; all pinned) is a
  **ring of 16 segments** (17 vertex pairs, vertex stride 20 B: position, uv; constructor decoded by hand, **inferred**) of
  radius 2 (sine-approximated, so 60 m after the scale) times the scale `cb1[4]` = (30, 1, 30, 1) (stored by the constructor, byte-decoded),
  vertically from y = -10 to y = +1 with `v = y`, centred on the camera (matrix translation = camera position, `FUN_00a44ad0`).
* Vertex shader: `pos = v0 * cb1[4]` then the camera-translated view-projection, and **`o0.z = 0`** (the far plane in the reverse-Z
  projection); `COLOR0 = cb1[5]`. Pixel shader: `rgb = cb1[5].rgb`, **`a = texture alpha`**, so it paints a flat colour whose
  opacity is the texture's alpha ramp, at depth 0 (the far plane in reverse-Z); the depth and blend state were not decoded.
* The texture is DXT5, 4 x 128, constant colour (255, 246, 255); **alpha is 255 for rows 0..22, falls smoothly to 0 by row 92 and stays 0**
  (so over `v` = 0.18 .. 0.72). With `v = y`, the band is opaque below the horizon of the ring and fades out above it: a **horizon
  fog band** between the geometry's end and the sky.
* The colour of `cb1[5]` is a double-buffered block field (`block+0x1c` / `+0x2c`, selected by the frame parity in `FUN_00a44ad0`); its
  writer was **not found** (Gap 1). It is almost certainly the same fog colour as `cb0[13]`, but that is **not proven**.

## 7. G3 (zone map and per-type tables) and G7 (rain noise): what was closed

* **G3 source of the map: the game-zone map.** `FUN_00a5eb50` returns the byte of the zone-map cell under a position (bits 3..5 =
  zone type 0..7, bits 6..7 = intensity 0..3); the same byte selects the gradient set in `FUN_00a66c30` (`night-light-next.md`,
  `gamezone-map-next.md`: the bytes are stored in every terrain tile). It is not a separate weather map.
* **G3 per-type tables: `settings\landscape_effects.bin`, container `precipitation`** (read by `FUN_00a66f50`, pinned), one child
  per zone, `57677bdc` = type id, four numbers per type (hash keys from `DAT_00f12228..38`, probe-checked):

  | key | stored at | used by | value by type (1 Arctic .. 9 Underwater) |
  |---|---|---|---|
  | `224afaf6` | Z+0x1964 + 0x10 t | snow target a (`FUN_00a63650`) | Arctic 1, all others 0 |
  | `a564332f` | Z+0x1968 + 0x10 t | snow target b | Arctic 1, all others 0 |
  | `60856e37` | Z+0x196c + 0x10 t | rain target a (`FUN_00a6db30`, smoothed into cloud+0x870) | Arctic 2, Desert 0, Underwater 0, others 1 |
  | `c67ba7f8` | Z+0x1970 + 0x10 t | rain target b | same as `60856e37` |

  Both targets are `lerp(a, b, intensity / 3)`; `a = b` everywhere in retail so the intensity does not matter. Types 8 (`e691f700`)
  and 9 (Underwater) are never selected by the 3-bit field. **Consequence:** Arctic always targets snow factor 1 (the start base:
  precipitation is snow), every other selectable zone targets 0 (rain); the rain base is 1 (Arctic 2) except in the Desert, where it is 0 (no rain).
  Precipitation additionally needs a non-zero cloud column (`P.+0x48 = min(1, rain_base * column)`).
* **G7 (partial).** `FUN_004dcb70` is a **fractional-octave fBm sum** of the gradient noise `FUN_004de470` (sum over `n` octaves of
  `noise(x, y, z) * w[i]`, coordinates multiplied by the lacunarity per octave, plus a fractional last octave). Its owner is created
  are 0.1, 2.3, 3 octaves and seed/flag 1. The octave weights come from a `pow` whose arguments are in registers (Gap 6).

## 8. Proven / Inferred

**Proven** (probe: bytes, hashes, decoded constructor, data and shaders; arithmetic pseudo-C-read where marked)

* Layout and sources of the eight preset groups; constructor defaults decoded from bytes; settings keys and retail values; the colours
  of P1/P2 come from `FUN_00a55cd0` (rows 2/3 of the gradient state, bytes pinned) and no instruction writes any other preset colour
  (call table; the six numeric groups are the only settings consumers).
* `cb0[13]` = colour + `p2` (alpha replaced, `FUN_0095b150` bytes); the VS upload layout (`cb0[12..13]`); the exponent formula and
  the pixel mix, re-disassembled from three vertex and three pixel shaders; base-2 `exp`.
* The window/day-factor and the parameter blends, the colour blends (pseudo-C reading of `FUN_00a432d0`, function hash pinned);
  tables `A`/`B` variant 0 are not read (pseudo-C reading).
* The fixtures above (model evaluated on the retail files), `cb0[14]` is water (consistent with `water-consumers-next.md`).
* The precipitation tables and keys; the fog gradient block geometry/shader/texture facts.

**Inferred**

* The sign of `bc` (row 2 toward the sun): from `S+0x250 = -toSun`, camera row +0x44 = -forward (JC2-094) and the hue (row 2 is the
  orange sunset colour at 06/18); no run-time capture.
* `d` is the eye depth (reverse-Z algebra); near/far values.
* Variant 1 = rain / variant 2 = snow naming (weights and the `rain_*` / `snow_*` settings names agree).
* The fog gradient band is a fog-coloured horizon strip (shader and texture are exact, the colour source is not).

## 9. Gaps (native details still unknown)

1. **Fog-gradient colour writer** (`cb1[5]` = block field `+0x1c`/`+0x2c`, `FUN_00a44ad0`): nobody found that stores to it for the
   FogGradient block (`FUN_00a44240` writes a sunset-tinted colour into a block at `S+0x18`; whether that is this block is unproven).
   Blocks: the horizon band colour/parity. Next: trace the owner of `FUN_00a49ec0`'s result (`FUN_00a4a6d0`, `FUN_00a4b990`) and its virtual
   `+0x80` writers.
2. **Camera near/far in play** (`camera-fov-next.md` Gap 7). The fog `d = far (t - near)/(far - near)`; with 0.1/40000 or 0.5/50000 the offset
   is below 0.5 m. Blocks exactness of the first metres only.
3. **Rows 2/3 for height, underwater, rain and snow sets** (sets 8..11 and their weights in `FUN_00a5d390`): `light_state` models only the
   base case (JC2-132 tracks it); the fog colour inherits the same limitation. Blocks fog colour in rain/snow/high altitude/underwater.
4. **Lightning:** `S+0x250` already includes the lightning override (`S+0xb4 > 0`, direction toward the flash, `FUN_00a58a00`); it is not modelled, so during a flash the sun-facing blend points at the flash.
5. **Sky dome colours** (`skygradientshader`, no `cb0`): the clear/horizon colours remain policy in `sky.rs`. Blocks: the sky colour the
   fog meets.
   used by `FUN_004de470`, and the mapping from the cloud column to `P.+0x48`: needed to know when it rains/snows. Blocks the rain/snow
   fog variants (they stay weight 0, i.e. clear).
7. **CPU consumer `FUN_00a63a20`** reads the stored fog layers through the getter `FUN_00936060` (it inverts the density with `log`): what
   it uses the visibility for (AI sight / occlusion?) is not decoded.
8. **Pixel/vertex shaders other than the three asserted** are covered by pattern only (the scan finds the same seven lines in 91 vertex
   shader names); their per-material PS tail (`water`, `particle`, sky) was not read one by one.

## 10. Bounded Rust handoff (not implemented)

Replace the policy fog of `sky.rs` (`FOG_VISIBILITY_*`, `FOG_EXTINCTION`, `FOG_INSCATTER`, `DistanceFog`) in two phases.

**Phase 1, data (jc2-formats + atmosphere.rs, testable against the probe fixtures):**

1. `jc2_formats::LandscapeEffects` gets `haze: HazeSettings { windows: [f32; 4] /* 5.5, 18.5, 20, 4 in file order 220137a5, 88d36ba1,
   f56ffcb7, f10e7ec1 -> (day starts, day ends, night starts, dawn starts) */, groups: [[f32; 5]; 6] }` parsed from section `19a8fc4a`
   (hash keys in section 2). Test: the table of section 2.
2. `atmosphere.rs`: pure functions `haze_day_factor(hour, &windows)`, `haze_params(hour, &settings, w_rain, w_snow) -> [f32; 5]`,
   `haze_vs_constants(p, cam_y) -> ([f32; 4], [f32; 4])`, `fog_exponent(c12, c13, d, y)`, `fog_transmittance(e) = min(2^-e, 1)`,
   `sun_blend(cam_forward, to_sun)`, and `fog_colour(rows, ab_variants, hour, bc, w_rain, w_snow)`. Tests: sections 3 and 4 fixtures
   (1e-4), the window edges (4, 5.5, 18.5, 20), `bc` for toward/away/side/up.
3. `sky_light.rs::LightState` already holds the gradient state; expose **row 2** (`haze_toward_sun`) and **row 3** (`haze_away`) with no gain
   (only vector 0 has the 2.2 gain) and the same 1 s smoothing. `AtmosphereState` keeps `precip`/`snow` at 0 until G7, so only the
   clear path is live; the variant-1/2 path is implemented but unfed.
4. Resource `HazeFog { colour: Vec3 /* native scene units */, p: [f32; 5], c12, c13 }` updated each frame in `sky.rs` from the hour, the
   camera y/forward and the sun direction (`SkyState` already has it).

**Phase 2, rendering (exact fog needs a shader term; Bevy's `DistanceFog` cannot express height or the 2^-e shape):**

* In the custom WGSL already owned by the project (`terrain_splat.wgsl`, the General material, the vegetation/other custom materials) add
  the seven vertex instructions as a per-fragment `fog_exponent(view_depth, world_y)` and mix
  `out = fog + (lit - fog) * min(exp2(-e), 1)` with `HazeFog` as a uniform (`colour` unscaled: fog is a scene colour, `native_units::native_colour`).
  `d` = `view_depth - near` (view-space z, not radial distance).
* For materials that remain `StandardMaterial`, keep Bevy `DistanceFog` as a **labelled approximation**: `FogFalloff::Exponential` with
  density `ln 2/(p1 - p0)` and colour = `HazeFog.colour`; in `native_units.rs` drop the "fog authored as display value" path for the
  haze (keep it for the clear colour / sky dome, still policy: Gap 5). Remove `FOG_*` and the 12 km / 7 km constants and the `horizon.rs`
  draw-distance comment; set the horizon draw distance from `p1`.
* Do not feed `E`/`F` (`haze_e`, `haze_f` in `AtmosphereState`) into the atmospheric fog: they are the **water** colour (`cb0[14]`), to
  be used by the water/underwater paths (`water-consumers-next.md`).
* Keep `FogGradient` (horizon band) out of scope until Gap 1.
