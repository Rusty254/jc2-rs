# Per-hour light tables and the sun/ambient writers (JC2-100)

Status: research brief. No runtime code changed. Probe: `research/tools/light_tables_probe.py`
(46 checks with `--deep`, 45 without; exits 0). All addresses are runtime addresses of
`JustCause2.exe` (no ASLR). Pseudo-C was read from `research/analysis/pseudo`; every instruction the
claims depend on was additionally decoded from the exe bytes and is pinned by a byte check in the probe.

## Headline

1. **The per-hour tables exist and have a source: `textures\atmosphere\gradients.bmp`.** The exe loads it
   at start-up (`FUN_00a46bd0`, path string `0x00f0f3b8`) through the archive file layer
   (`FUN_004d8f50` -> `FUN_004f1490`, which looks the path up in the mounted archives when they are
   active). `weather.md` was wrong to call it an editor image that is "ignored".
   It is a 24 x 22 picture: 24 hours (columns) x 22 rows. It fills **ten tables**: one global scalar table, three scalar
   tables, and six RGB tables each with three weather variants.
2. **None of those tables feeds the sun colour or the ambient.** Their consumers are the cloud card colours, the
   ground-haze/fog colours (`cb0[13]`/`cb0[14]`), a point-light intensity scale and a facade/merged-block
   constant (section 4).
3. **`weather+0x204..+0x243` (sun colour, the `x4` light colour, sky ambient, ground ambient) is only ever
   initialised, to 1.0.** The 16 words are set by `FUN_00a58640` and read by `FUN_00a58a00`. No instruction in
   the weather module (0xa40000..0xa80000) stores to them again, and no other function passes their
   address anywhere (`--deep` sweep). So `cb0[3]` is (1,1,1) plus the lightning boost, and
   `cb0[10]`/`cb0[11]` are (1,1,1). **This contradicts the dark, blue nights in the original footage**
   (`reference/world/night-city-lights-sea-moon-from-base-ledge_s38.jpg`). The mechanism that darkens night is
   **unresolved** (Gaps G1). Nothing here may be used to justify a hand-tuned night curve as native.
4. `cb0[9]` and `cb0[12].w` are **shadow-system** constants, not light tables.
5. The weather variant depends on **cloud cover only indirectly** and on the weather zone and snow factor
   (section 3).

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `textures\atmosphere\gradients.bmp` (pc0, 1,640 B, BMP 24 x 22, 24 bpp) | `b66bf51075ed538b5a3fe72bc30c3ccd52e1cfcb0b618bde478425864648a45d` |
| all ten tables, 6-decimal text rendering (probe `tables_text_sha256`) | `03d92bf5b8ffcba34f3026d9ba4ebdd882aae1c060645ca988a09f2f17334e11` |

The bmp was read with `jc2tool cat`. It is not committed.

## 1. Source file and loader (verified)

`FUN_00a45790` / `FUN_00a419b0` convert the 24-bit BMP to a float image (4 floats per pixel:
**R = byte 2, G = byte 1, B = byte 0, A = 1.0**, each byte x 1/255). BMP rows are stored bottom-up and the
loader walks them with a negative stride, so **memory row 0 is the top of the picture**. Rows below are
*memory* rows (file row = 21 - memory row).

`FUN_00a46bd0` (this = weather object W) then copies, for each of the 24 columns (hour 0..23):

| Memory row | Destination (offsets in W) | Conversion |
|---|---|---|
| 0 | scalar table `S0` at `+0x4d0` (25 floats) | R only, raw |
| `1 + 7v` (v = 0,1,2) | scalar table `S[v]` at `+0x534 + 0x64*v` | R only, raw |
| `2 + 7v` | vec4 table `A` at `+0x660 + 0x190*v` | RGB = pixel**2.2, w stays 1.0 |
| `3 + 7v` | `B` at `+0xb10 + 0x190*v` | same |
| `4 + 7v` | `C` at `+0xfc0 + 0x190*v` | same |
| `5 + 7v` | `D` at `+0x1470 + 0x190*v` | same |
| `6 + 7v` | `E` at `+0x1920 + 0x190*v` | same |
| `7 + 7v` | `F` at `+0x1dd0 + 0x190*v` | same |

* Gamma: `pow(x, 2.2)` with the double at `0x00e4aab0` (the float 2.2 promoted: 2.200000047683716). The tables are
  therefore **linear RGB in 0..1**; the picture is sRGB-like.
* A vec4 table is 25 entries x 16 B per variant (stride `0x10` per hour, `0x190` per variant). A scalar table is 25 floats.
* Entry 24 is a copy of entry 0 (code at `0x00a47167`..), so a lerp across 23->24 is continuous.
* If `gradients.bmp` cannot be opened, the tables keep their constructor value (every float 1.0,
  `FUN_00a517f0`). With the retail archive it is present.

## 2. Time source and interpolation (verified)

* Time is the float **hour 0..24** at `CRenderEngine+0xac` (`weather.md`: copied from `CWorldTime+0x20`).
* All table readers do the same: `i = trunc(hour)`, `j = i + 1` (**wrapping to 0 when above 23**, `cmp ebx,0x17` at
  `0x00a53a9d`), `f = hour - i`, **linear** `a[i] + (a[j] - a[i]) * f` per component. No smoothstep, no other
  time input. `FUN_00a472f0` and `FUN_0091dcc0` index entries `i`, `i+1` without the wrap, which is equivalent
  because of entry 24.
* The tables hold no seasonal or day-of-year term; the only inputs are the hour and the variant weights.

## 3. Weather variants (verified from pseudo-C; the variant meanings are inferred)

The three variants of the tables are blended with two weights from `FUN_00a40080` on the precipitation
object `P = *(cloud+0x204)` (the `CRenderBlockWeather` owner, constructed by `FUN_00a49aa0`):

```
w1 = (1 - P.+0x50) * P.+0x48          w2 = P.+0x50 * P.+0x48
x  = lerp_hour(variant 0)
if w1 > 0:  x += (lerp_hour(variant 1) - x) * w1
if w2 > 0:  x += (lerp_hour(variant 2) - x) * w2
```

(`FUN_00a53a80` for cloud colours; the haze `FUN_00a432d0` receives `w1`, `w2` as its two last arguments from
`FUN_00a654b0` at `0x00a657b2`..`0x00a657da`.) `FUN_00a472f0` uses the same weights for the scalar table.

Writer of `P.+0x48`/`+0x4c`/`+0x50`: `FUN_00a6db30` (cloud object, end of the function):

```
P.+0x48 = min(1, rain_base * column)       rain_base = cloud+0x870, smoothed to a zone target at rate 0.1*dt
P.+0x4c = max(0, rain_base - 1)
P.+0x50 = snow_factor = cloud+0x1fc        smoothed toward FUN_00a63650 at rate 0.2*dt
```

* `column` = `cloud+0x200` is the smoothed cloud-column density (`FUN_00a59970`, which uses the cover
  `+0x85c`; see `cloud-shadow-consumers-next.md` section 4). **So the weather variant depends on cloud cover only through
  this product.** At zero cover `column` is 0, `P.+0x48` is 0, `w1 = w2 = 0` and only variant 0 is used.
* The zone term is **position dependent**: `FUN_00a5eb50` indexes a toroidal map of 8 x 8 tiles of 64 x 64 bytes
  (cell `(x - DAT_0108ab34) / 8 m`, tile `512 m`, the stride `0x100c` per tile) around the camera. Bits 3..5 of the byte
  are a zone type (0..7), bits 6..7 an intensity (0..3). `rain_base` and `snow_factor` are
  `lerp(a[type], b[type], intensity / 3)` of two per-type tables in the global manager `DAT_01191c48`
  (`+0x1964/+0x1968` and `+0x199c/+0x19a0`, 0x10 stride). *Origin of the map and of those tables: unresolved (G3).*
* Which variant is "rain" and which "snow" is inferred from the formula (`P.+0x50` is the snow factor), not from names.
  In the picture variant 1 has flat grey cloud rows (file rows 9 and 10) and a bluish-grey `A`/`B`, and variant 2
  is identical to variant 0 for the scalar row and for `C`, `D`, `E`, `F` (file rows 6,3,2,1,0 equal 20,17,16,15,14) and differs only in a greyer `A`/`B`.

## 4. Consumers of the tables (verified unless marked)

| Table(s) | Consumer | Result |
|---|---|---|
| `C` (+0xfc0), `D` (+0x1470), with the variant blend | `FUN_00a53a80` (cloud object `+0x284`/`+0x288`) | cloud card lit / shaded colour, written to the two cloud render blocks (`+0x88c` at `+0x98..+0xb4`, `+0x890` at `+0x3c..+0x58`) |
| `A` (+0x660), `B` (+0xb10), `E` (+0x1920) or `F` (+0x1dd0) | `FUN_00a432d0` (haze object `= cloud+0x44`; `[0..3]` are the table pointers; `E` above the sea, `F` when the camera is below 200 m) | ground-haze/fog colours, see below |
| `S[0..2]` (+0x534...) | `FUN_00a472f0` -> `FUN_0091ea30` -> `CRenderEngine+0x29b8` | scalar `s`; `FUN_0091dcc0` multiplies each point light's colour by `s * CRenderEngine+0x830`, caps each channel at 8 (point-light intensity: 1.0 at night, 0.25 by day in variant 0) |
| `S0` (+0x4d0, via `FUN_0095d160` -> `CRenderEngine+0x29b4`) | `FUN_0091dcc0` lerps it by hour into the per-frame slot, `FUN_0092ebe0` -> render context `+0xcd0` -> **pixel cb2[0].x of the merged-geometry and facade render blocks** (`FUN_00928d30`, `FUN_009260f0`, virtuals of `CRenderBlockMerged` / `CRenderBlockFacade`) | 0 from 08:00 to 15:00, 0.25..0.30 at night. **Inferred:** a night "lights on" factor for facades (not verified in the shaders) |

and five parameters at `CRenderEngine+0xc8 + 0x10*i` / `+0xe8..` (device method on `CRenderEngine`). `FUN_0095b150`
(called last by `FUN_0095f020`) copies those two vec4 to the register file: **`cb0[13] = CRenderEngine+0xc8`,
`cb0[14] = CRenderEngine+0xd8`**. FastTer uses `cb0[13].xyz` as the fog colour:
`out = fog + (lit * albedo - fog) * min(exp(-v2.w), 1)`, and `cb0[14]` only in the branch where `v3.y >= 0`. For
clear weather the fog colour comes from static presets in the haze object (`H[0x16..0x22]`, `H[0x28..]`, `H[0x31..]`) blended by a sun-view
factor and a dawn/dusk hour window (`H[0x5e..0x61]`), while `cb0[14]` takes table `E`/`F` variant 0. In rain and snow the
weighted terms lerp in the variant-1/2 rows of `A`/`B`/`E`/`F`. *Pseudo-C reading, not machine-checked.* The values of the static presets and the
use of `A`/`B` variant 0 are **unresolved** (G2).

Hour-interpolated values (probe, linear RGB; clear weather, variant 0; `S*` raw):

| hour | S0 | S[0] | A | B | C (cloud lit) | D (cloud shaded) | E = F |
|---|---|---|---|---|---|---|---|
| 0 | 0.247 | 1.0 | .028 .072 .128 | .028 .072 .128 | .109 .109 .109 | .012 .019 .027 | 0 0 0 / 0 .027 .045 |
| 6 | 0.184 | 1.0 | .470 .180 .119 | .142 .157 .198 | 1.0 .302 .136 | .320 .205 .133 | .010 .027 .016 |
| 12 | 0.0 | 0.251 | 0 .320 .941 | 0 .320 .941 | .561 .548 .517 | .399 .389 .364 | .008 .130 .130 |
| 18 | 0.196 | 1.0 | .459 .092 .017 | .187 .187 .154 | 1.0 .389 .066 | .227 .164 .076 | .010 .027 .016 |

## 5. Sun and ambient: the writers of `weather+0x204..+0x234` (negative result)

`FUN_00a58a00` (called from `FUN_00a472f0` every frame; `this` = W):

* inputs, all 1.0 forever: sun colour `+0x204..+0x210`; light colour #2 `+0x214..+0x220` (read `x 4.0`);
  sky ambient `+0x224..+0x230`; ground ambient `+0x234..+0x240`.
* **Corrects `cloud-shadow-consumers-next.md` section 5:** the weather fields passed to `FUN_0091f810` are the raw `+0x224` / `+0x234`
  (`lea eax,[edi+0x234]` / `lea ecx,[edi+0x224]` at `0x00a594a5`/`ac`), which become `CRenderEngine+0x110` (`cb0[10].xyz`) and
  `+0x120` (`cb0[11].xyz`). The `+0x214 * 4` value (plus the lightning `* 20` term, `DAT_0108b514`) goes through
  `FUN_00972b90` into `light+0x28..+0x34`, which the terrain pixel shader does not read.
* sun colour -> `FUN_00932990` -> `light+0x18` (= `cb0[3]`, via the accessor `FUN_004813d0`): plus the lightning boost
  `(0.654, 0.572, 0.921) * 5 * min(2 * flash, 1)` when `weather+0xb4 > 0` (`DAT_0108b518` = 5); underwater (camera y below 200
  and `DAT_01190290` set) lerped toward the cloud object's `+0x78..+0x80` by `(200 - y) * 0.01`.
* The direction (`light+0x44` = `cb0[4]`) and `cb0[0]` follow `cloud-shadow-consumers-next.md` section 2 unchanged.
* `FUN_00a58640` is the only initialiser. A function-aware sweep (`--deep`) of every function in 0xa40000..0xa80000
  finds stores to `+0x204..+0x243` only in `FUN_00a58640` and in cloud-object code (`FUN_00a54160`,
  `FUN_00a6d9c0`, `FUN_00a6f230`; the cloud object has its own `+0x204` pointer and `+0x208`). A whole-exe sweep for
  `lea` of those displacements finds only `FUN_00a58a00`. **Limits:** writes through an aliased pointer, or from a code path the
  static sweep cannot see, are not excluded.

Consequence as traced: `cb0[3] = (1,1,1)`, `cb0[10] = cb0[11] = (1,1,1)`, so `cb0[12] = (2,2,2)`.
The shader multiplies these directly (`sun = cb0[3] * sat(N.-L) * shadow * ...`, `amb = lerp(cb0[11], cb0[10], ...) * ao`).

### Night: the Moon flip and attenuation (verified)

* `FUN_00a58a00` builds `L = -(sun direction)`. If `L.y > 0` (sun below the horizon) it negates `L` (the **same single light now
  points from the opposite body, the moon**, orbit twelve hours later) and squares the horizon constant `k` (0.7071 -> 0.5).
* **Attenuation terms:** none on the colour. The only terms are the sun-shadow floor `cb0[10].w = min(|L.y| * 20, 1) * 0.15`
  (taken from the unflipped copy, so it is 0.15 whenever the body is more than 2.9 degrees from the horizon) and the
  `cb0[0].z = -L.y / k * 30` horizon-shadow height. The probe prints the values at 0/6/12/18 (`cb0[0]` at hour 0 is (0, 30, 32.08, 1)).
* Separately, `FUN_00a594e0` tints the sun/moon *billboards*: `f = max(0.9 - 1.5|L.y|, 0)`; sun (2, 2(1 - .75f), 2(1 - f), 1)
  (`DAT_0108b520` = 2), moon (0.4, 0.4(1 - .4f), 0.4(1 - f), 1) (`DAT_0108b51c` = 0.4). Cosmetic only.

## 6. `cb0[9]`, `cb0[12].w`, `cb0[13]`, `cb0[14]` summary

| cb0 | Source | Meaning |
|---|---|---|
| [9] | `FUN_0095f020`: device-state object `CGraphicsEnginePC_D3D` vtable `+0x54`: byte `+8`, float `f` at `+0x1f4` | `(flag ? 1 : 0, 0, flag ? 1 - f : 0, flag ? f : 1)`. Shaders use `sat(depth * cb0[9].z + cb0[9].w)` in the shadow-cascade variants (`fastter_lod01sh0`...): a **shadow fade**, not lighting. The owner object is not named (G4) |
| [13] | `CRenderEngine+0xc8` (`FUN_0095b150`) | fog colour A, section 4 |
| [14] | `CRenderEngine+0xd8` | fog colour B, section 4 |

## 7. Verified, inferred, unresolved

* **Verified (byte-checked in the probe):** the file path, the 24 x 22 layout and the row-to-table map, the loader
  (R,G,B order, flip, 1/255), the gamma double, the 24-column / 3-variant / 7-row loop constants, the stores to each
  table, the table pointer hand-off to the cloud/haze objects, the hour wrap, `weather+0x204..0x243 = 1.0` initialisation, the
  `FUN_0091f810` argument addresses, the lightning/billboard constants, and the full table fingerprint.
* **Verified (pseudo-C reading):** the variant weights and their producer, the pseudo-formulae for cloud, point-light and
  haze consumers, the moon flip, the cb0[13]/[14] upload path.
* **Inferred:** variant meanings (clear / rain / snow), `S0` = facade night factor, the colour semantics of `A`/`B`.
* **Unresolved:** see Gaps.

## 8. Gaps (native details still unknown)

* **G1. What makes night dark.** With the traced chain `cb0[3]`/`cb0[10]`/`cb0[11]` are constant (1,1,1) while the original
  footage has dim blue nights. Candidates (all unverified): a writer to `weather+0x204..` through an alias; a
  lighting term in the shaders that depends on an input we have not mapped; the haze/fog colours doing the work.
  Blocks: native sun/ambient colours for JC2-084/085/022. Next: dump the General and FastTer deferred/forward variants' uses of `cb0[1..2]`,
  trace the runtime light object (`DAT_011907c4+0x834`) writers by disassembling every function that stores to `light+0x18..0x24`.
* **G2. Haze presets.** Writers and values of the haze object's static sets (`cloud+0x44 + 0x58..0x88`, `+0xa0..`, `+0xc4..`, windows `+0x178..+0x184`) and the exact
  role of `A`/`B` variant 0 in clear weather. Blocks: native fog colour per hour.
* **G3. Weather zones.** Source of the 8 x 8 x 64 x 64 zone map (weather-zone `.nlz` files? `gamezone_gradients.bmp`?) and of the two per-type
  tables at `DAT_01191c48+0x1964..0x19a0`. Blocks: where rain/snow variants apply.
* **G4. The `cb0[9]` owner** (device state object at vtable `+0x54`), its `+0x1f4` value, and the `0xc0`-byte block copied to `DAT_013db080`.
* **G5. `CRenderEngine+0x830`** (point-light scale factor) and the exact fragment of `FUN_00928d30` / `FUN_009260f0` that reads `cb2[0].x` (confirm "facade lights").
* **G6. Cloud-light consumers:** what `light+0x28` (the `x4` colour) is used by, if anything.
* **G7.** The `FUN_004dcb70` noise (from `cloud-shadow-consumers-next.md`) still blocks the cloud-column to rain mapping.

## 9. Bounded runtime handoff (not implemented)

Do **not** wire the six RGB tables to Bevy's sun or ambient: the native sun/ambient are not table driven (G1).

1. `jc2_formats::weather::GradientTables` (new): parse `gradients.bmp` exactly as section 1 (R,G,B, flip, `** 2.2` for the six RGB
   tables) into `{ s0: [f32; 25], s: [[f32; 25]; 3], v: [[[[f32; 4]; 25]; 3]; 6] }` with entry 24 = entry 0. Tests: the probe's spot
   values at 0/6/12/18, hour 23.5 wrap, variant weights.
2. `jc2-game` resource `AtmosphereState { hour, precip: f32 (P.+0x48), snow: f32 (P.+0x50) }`; `w1 = (1 - snow) * precip`,
   `w2 = snow * precip`; `precip` stays 0 until the cloud column/zone map exist (G3, G7), which keeps variant 0.
3. Shader uniforms that can be filled now with native values: cloud-card lit/shaded colours (`C`, `D` blended), a point-light
   scale `S` (times the still-unknown `+0x830`), and the facade/merged factor `S0` (labelled inferred). Fog colours `cb0[13]`/`cb0[14]` only after G2.
4. Moon: keep a **single** directional light that flips to the moon when the sun height is negative (formulas of section 5 and
   `cloud-shadow-consumers-next.md` section 2); remove the independent Bevy "Moon" light for parity. Keep any night dimming
   clearly labelled **non-native** until G1 is closed.
