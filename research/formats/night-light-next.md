# What makes night dark: the real writer of sun and ambient colours (JC2-115)

Status: research brief. No runtime code changed. Probe: `research/tools/night_light_probe.py` (98 checks, exits 0;
needs a private `jc2tool` for the data checks). All addresses are runtime addresses of `JustCause2.exe` (no ASLR).
Pseudo-C was read from `research/analysis/pseudo`; every instruction that a claim depends on was also decoded from the
exe bytes and is pinned by a byte check in the probe.

## Headline

1. **The negative result of JC2-100 (`light-tables-next.md` section 5, gap G1) is wrong, and the night mechanism is found.**
   The sun colour, the second light colour and the two ambients (`cb0[3]`, `cb0[10]`, `cb0[11]`) are **not constants**. The 1.0
   values stored by the initialiser `FUN_00a58640` are overwritten **every frame**. The writer is a *call*, not a store through
   an address, which is why the store sweep of JC2-100 could not see it:
   `FUN_00a751b0` (per frame) -> `FUN_00a66c30` (gradient update) -> `FUN_00a5d390` (blend) -> `FUN_0089f9d0` (setter, sixteen
   `fstp` stores to `[ecx+0x204 .. +0x240]`) on the sky object.
2. **The source is a picture, not code: `textures\landscape\gamezone_gradients.bmp`** (48 x 96, 24 bpp), named by
   `settings\landscape_effects.bin` (the file `weather.md` called "nothing to do with the sky"). It holds **12 gradient sets x 6 colour
   rows x 2 variants x 24 hours**. Row 0 of a set is the sun/moon colour, row 4 the sky ambient (`cb0[10]`), row 5 the ground ambient
   (`cb0[11]`). The set is chosen by the **game zone type under the camera** (Arctic, Desert, Jungle, Sea, City, Grass, Default), with
   extra sets for height, underwater, rain and snow.
3. **Night is dark because of three table-driven factors** (values in section 5, Arctic and Default zone, intensity 3, open ground):
   - the sun/moon colour falls from (1.92, 1.76, 1.38) [Arctic] or (5.67, 5.05, 3.70) [Default] at noon to (0.53, 0.62, 0.71)
     [Arctic] or (0.33, 0.72, 1.22) [Default] at night. The single light flips to the moon at about 05:40 and 18:20 (JC2-100), but its
     colour comes from the **same row 0**; for hours 19..23 and 0..4 it is one constant bluish night colour;
   - the **ground ambient (`cb0[11]`) collapses** from (0.88, 0.94, 0.87) to (0.006, 0.006, 0.032) (Arctic noon to night), so walls and
     undersides lose almost all fill light;
   - the sun/moon height: the moon at midnight stands 32 degrees up (N.L of flat ground 0.53) against 45 degrees for the noon sun (0.71).
   Flat open ground gets luminance 0.43 at midnight against 1.27 at noon (Arctic, 0.34x) and 0.49 against 3.61 (Default, 0.14x);
   a wall on the shadow side gets 0.056 against 0.47 (Arctic) and 0.068 against 0.29 (Default).
4. The hue is table-driven too: the night sun/moon colour is blue-grey, the night sky ambient is blue (0.05, 0.11, 0.17 up to
   0.01, 0.15, 0.32), the dawn and dusk colours are orange (sun (2.70, 0.91, 0.47) at 06:00 Arctic).
5. **Whether these numbers alone reproduce the footage is not proven** (the reference shots are uncalibrated and the tone-mapper,
   auto-exposure and the hour-dependent post-effect blocks were not fully decoded, section 7). Nothing here may be presented as a
   finished night look.

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `textures\landscape\gamezone_gradients.bmp` (pc0, 13,880 B, BMP 48 x 96, 24 bpp) | `b43e8c1b56f53a981861ee0377c26ab5f2d151f800fcec73495601b3f1fcb329` |
| `settings\landscape_effects.bin` (2,637 B) | `43fa8c178318d1aadc48d531f5a619f558137c9ea5a7e2093f4476a7d7caa2fd` |
| all 16 x 6 x 2 x 25 linear tables, 6-decimal text (probe `tables_text_sha256`) | `b8de88e09afc37e212d9f74e4b4a0e97f069a4e7bb14c0d4c00e25192bba814d` |

The files were read with `jc2tool cat`/`prop`. They are not committed.

## 1. The object model, corrected

JC2-100 and `cloud-shadow-consumers-next.md` called the object that holds `+0x204..0x243` "the weather object W" (`this` of
`FUN_00a58640`/`FUN_00a58a00`). The byte checks show it is a **sub-object of W at W+0x10**, the *sky object* S:

* `FUN_00a517f0` (W's constructor) runs `lea ecx,[esi+0x10]; call FUN_00a58640` (0x00a517f3/0x00a517f6); `FUN_00a472f0` (W's per-frame
  update) runs `lea edi,[esi+0x10]` ... `call FUN_00a58a00` (0x00a472fc/0x00a4730f). All "W+0x204" of JC2-100 are S+0x204; the field
  names below use S.
* The manager `CLandscapeManager` (singleton `DAT_01191c48`, ctor `FUN_00a5e1d0`) holds W at +0x2c and embeds the **gradient
  sub-object Z at +0x30**. `FUN_00a74f80` stores `W+0x10` at `[manager_view+0x1af4]`, which is **Z+0x1ac8** (0x00a75180/0x00a7518a);
  `FUN_00a5d390` reads it back as the `this` of the setter (0x00a5dfe4).
* `FUN_00a751b0` (called every frame) runs, in this order, `FUN_00a66c30` on Z (0x00a75302/0x00a75305), then `FUN_00a472f0` on W
  (0x00a7530a/0x00a75314), so the new colours are in S **before** `FUN_00a58a00` copies them to `light+0x18` (`cb0[3]`) and passes
  `S+0x224`/`S+0x234` raw to `CRenderEngine+0x110/+0x120` (`cb0[10]`/`cb0[11]`).

## 2. The writer chain (verified)

_(decompiler excerpt omitted from the public snapshot)_

* **Complete caller list (whole-`.text` scan of every `call rel32`, probe-checked):** `FUN_0089f9d0` is called only from
  `FUN_00a5d390` (0x00a5dff7) and from the cutscene effect key `FUN_008961d0` (0x008966e8); `FUN_00a5d390` only from `FUN_00a66c30`
  and from the loader `FUN_00a69830` (0x00a699b6, first frame: set 0, intensity 0, dt 10 so it snaps); `FUN_00a66c30` from the manager
  and from two other sites (0x00683a40 game code, 0x009000f3 cutscene code, **not decoded**).
* `FUN_0089f9d0` is exactly sixteen `fstp dword [ecx+disp]` for disp 0x204..0x240 and nothing else (probe).
* `FUN_00a69830` (the loader, keys read from `settings\landscape_effects.bin`) fills Z: the image path (key `0c479c2f`, string
  `textures\landscape\gamezone_gradients.bmp`), the eight `gradient_for_zone_NN` values (zone type N -> set N, the hashes are those of the names),
  the heights 30, 300, 1000 (m) at Z+0x1aec/+0x1af0/+0x1ae4, the underwater depth 120 at Z+0x1ae8, the sets 8, 9, 10, 11 (height,
  underwater, rain, snow) at Z+0x1ad0/+0x1ad4/+0x1ad8/+0x1adc and the **gain 2.2** at Z+0x147e0 (constructor default 1.7).

## 3. The tables (verified)

`FUN_00a632b0` loads the picture with the loader of JC2-100 (R = byte 2, memory row 0 = top of the picture, bytes/255) and requires
48 x 96 (0x00a6334f/0x00a6335c). Then, for 16 sets x 6 rows x 2 variants x 24 hours (loop counts at 0x00a63376, 0x00a63390, 0x00a633a6):

* pixel `(set*6 + row, variant*24 + hour)` -> `rgb ** 2.2` (exponent from 0x00e4aab0, 0x00a633b4), stored at
  `Z+0x1afc + ((set*6 + row)*50 + variant*25 + hour)*16`; entry 24 of each 25-entry variant = entry 0; w stays 1.0.
* Sets 0..11 hold data, **12..15 are black filler** (probe), which fixes the row order. The sets are: 0..7 the gradient per game zone
  type (`terrain\gamezone.bin`: Arctic 1, Desert 2, Jungle 3, Sea 4, City 5, Grass 6, Default 7, None 0), 8 height, 9 underwater (blue-green),
  10 rain (flat grey-blue, 0.35, 0.38, 0.52 sun colour all day), 11 snow (grey).
* The two variants differ **only** in set 2 (row 4), set 3 (row 5) and set 9 (rows 0, 1, 4, 5); in every other set they are identical, so the zone
  intensity does not matter there (probe-derived; the picture itself is the evidence). Which variant is "high intensity" is *inferred* (section 4).
* Row meaning (sets 0..8, linear after the pow): row 0 sun/moon colour (before the final transform), row 1 colour 2, row 2 and row 3 the
  cloud colours (copied to the cloud object), row 4 **sky ambient** (up-facing), row 5 **ground ambient** (down-facing, in the
  zones the colour of the ground: Arctic near-white, Desert sand, Jungle green).

## 4. What `FUN_00a5d390` does

Proven by bytes (probe) and by reading the pseudo-C against the asm for the base case:

* hour `h = CRenderEngine+0xac`, `i = floor(h)`, `f = h - i`; every entry is linear between hour `i` and `i+1` (entry 24 wraps).
* **Base case (on the ground, no rain or snow):** the six rows are `wv * variant0 + (1 - wv) * variant1` with `wv = clamp(intensity * Z[0x1a80 + set*4])`.
  The weights come from `settings\landscape_effects.bin`, container `game_zones` (hash `94bfdbd4`), keys `max_gradient_0..9` = 3, so
  **wv = intensity / 3** for the zone sets (the unused weights of sets 10 and 11 never matter: they enter through the rain/snow weights).
* **Smoothing:** the result is lerped into a state array (Z+0x146fc) with factor `clamp(Z[0x1ac0] * dt)`, Z[0x1ac0] = 1.0 (key
  `a08d04c3`), i.e. about a one second time constant; it snaps (factor 1) on the frame after leaving the underwater set.
* **Final transform, vector 0 only (the sun colour):** `x -> ((x ** (1/2.2)) * 2.2) ** 2.2` per channel (constants 0x00e5b860 and
  0x00e4aab0, gain from Z+0x147e0 = 2.2). In effect the table pixel `p` becomes `(2.2 p) ** 2.2` (blend taken in the linear domain), at most
  2.2 ** 2.2 = 5.667. **The other vectors (colour 2, ambients) are used unchanged**.
* The four vectors 0, 1, 4, 5 go to S+0x204, +0x214, +0x224, +0x234.

Read from the pseudo-C only (not byte-checked line by line; labelled inferred in the lists):

* **Height (camera y >= 200):** `up = max(0, (y - max(ground, 200)) - 30)`; set 8 is mixed in with weight `up / 270` (both variants again mixed by
  `up / 970`).
* **Underwater (y < 200):** `down = 200 - y`; the set-9 colours replace the state, mixed by `down / 120`.
* **Rain / snow:** the result is lerped toward the set-10 colours by the rain weight and toward the set-11 colours by the snow weight (the
  two weights of JC2-100 section 3; the lerp target itself uses the weight as its variant weight).

## 5. Values by hour (probe output; steady state, intensity 3, open ground)

Sun/moon colour is the final transform of row 0; `sky` is `cb0[10]`, `gnd` is `cb0[11]`; `lit(flat)` is the luminance (weights of the scene
shader, 0.2125 / 0.7154 / 0.0721) of `cb0[3] * N.L + cb0[10]` for an up-facing surface with shadow factor 1 and ao 1 (the light is the moon when
the sun height is below zero; for the moon `N.L` is the absolute height).

| Zone set | hour | body (sun y) | sun/moon colour | sky ambient `cb0[10]` | ground ambient `cb0[11]` | lit(flat) |
|---|---|---|---|---|---|---|
| 1 Arctic | 0 | moon (-0.535) | 0.532 0.616 0.707 | 0.048 0.114 0.170 | 0.006 0.006 0.032 | 0.43 |
| 1 Arctic | 4 | moon (-0.224) | 0.532 0.616 0.707 | 0.122 0.201 0.298 | 0.006 0.006 0.032 | 0.33 |
| 1 Arctic | 6 | sun (0.086) | 2.698 0.909 0.467 | 0.284 0.251 0.298 | 0.235 0.263 0.293 | 0.37 |
| 1 Arctic | 9 | sun (0.525) | 2.383 2.175 1.662 | 0.000 0.080 0.085 | 0.723 0.789 0.745 | 1.21 |
| 1 Arctic | 12 | sun (0.707) | 1.922 1.763 1.376 | 0.000 0.021 0.070 | 0.883 0.941 0.867 | 1.27 |
| 1 Arctic | 18 | sun (0.086) | 1.977 0.631 0.164 | 0.148 0.231 0.284 | 0.247 0.164 0.099 | 0.29 |
| 1 Arctic | 21 | moon (-0.353) | 0.532 0.616 0.707 | 0.048 0.114 0.170 | 0.006 0.006 0.032 | 0.32 |
| 7 Default | 0 | moon (-0.535) | 0.330 0.723 1.223 | 0.008 0.145 0.316 | 0.006 0.006 0.032 | 0.49 |
| 7 Default | 6 | sun (0.086) | 4.646 1.563 0.805 | 0.284 0.251 0.298 | 0.028 0.028 0.028 | 0.45 |
| 7 Default | 12 | sun (0.707) | 5.667 5.051 3.697 | 0.000 0.017 0.041 | 0.730 0.554 0.263 | 3.61 |
| 7 Default | 18 | sun (0.086) | 3.394 1.079 0.280 | 0.148 0.231 0.284 | 0.145 0.062 0.024 | 0.35 |
| 7 Default | 21 | moon (-0.353) | 0.330 0.723 1.223 | 0.008 0.145 0.316 | 0.006 0.006 0.032 | 0.37 |

(The probe prints sets 1, 7, 3 and 5 at 0, 4, 6, 9, 12, 15, 18, 21; `--hours` changes the list.) Facts to note:

* Hours 19..23 and 0..4 use one constant night colour in sets 0..8; the sun table is orange at 06:00 and 18:00 and the light flips body at
  about 05:40 / 18:20, so the handover from moon to sun passes through a half-orange colour.
* At night the **up-facing ambient is brighter** than at noon (a sky glow), but the **ground ambient is about 0.7 % of its noon value**; the
  terrain shader mixes them by `N.y`, so slopes and walls lose most ambient while flat ground does not.
* Shadow factor: `cb0[0]`/`cb0[10].w` are as in JC2-100 (terrain horizon shadow, 0.15 floor); away from the dawn/dusk flips the terrain shadow term is 1 on open
  ground. Cloud cover, rain and snow only enter through the variant weights.

## 6. What the earlier gaps say now

* **JC2-100 G1 (what makes night dark): closed at the mechanism level.** Sun/moon colour, sky ambient and ground ambient are hour-and-zone-table-driven (section 5).
* **JC2-100 G5 (`CRenderEngine+0x830`):** in this exe there is no store to disp 0x830 except the constructor (0x00956a9e, value 1.0) and
  the reader `FUN_0091dcc0`; so the point-light scale factor is constant 1.0 *as far as a displacement sweep can show*. Point lights are
  scaled per hour by the `gradients.bmp` scalar tables only (JC2-100 section 4).
* **JC2-100 G3 (weather zones):** the zone cell byte comes from the `CGameZoneMap` (`DAT_01381af8`, size 0x40308), and the type/intensity bits are the
  ones JC2-100 section 3 describes. The source of the map and the default for cells without data are still open.

## 7. Other night terms checked (and what they do and do not explain)

* **Tone mapping.** `Scene` pixel shader: `colour -> (adapted + cb1[2].x) * cb1[1].rgb * (saturation(lum) and contrast mixing)`, output **`sqrt`** (the gamma-2.0 encode of
  JC2-099); `cb1[1]` is a tint, `cb1[2].x` = parameter 7 minus 1. All post parameters except the death-screen fades (`FUN_00554b00`) come from
  `settings\post_effects.bin` (`exposure` 0.87, `contrast` 1.1, `saturation` 1, `bloom` 0) and are **not** hour-driven there.
* **Auto-exposure.** `SampleLumInitial/Final` and `CalculateAdaptedLum` compute a key/luminance exposure value (the adapted value is clamped to 0..4 and smoothed with
  `1 - exp(-1.74878 dt)` per frame, both read from the shader bytes). Its parameters (`cb1[4]` of the final pass) are produced elsewhere and **were
  not decoded**. It works *against* a dark frame (a dark average brightens the picture, up to its clamp), so it cannot be what makes night dark, but its
  strength, and therefore how much of the table difference survives on screen, is unquantified (gap 7).
* **Hour-dependent post-effect blocks.** `settings\landscape_effects.bin` (`post_effects`) holds two parameter blocks (`#0a10a074`,
  `#0a48aa9b`) per gradient set and four hour windows 4.5 / 5.5 / 18.5 / 19.5 (key `#96232764`): `FUN_00a5e010` blends the two blocks across
  dawn (4.5 to 5.5) and dusk (18.5 to 19.5). The block contents (for example `#679def2a` 1.6 against 1, `saturation` 0.95 against 1,
  `#a4d31fce` 0 against 0.3, `#fad89ab2` 10 against -0.8) are *unmapped to the post shaders*; this is another night term that is **not** quantified here.
* **Fog/haze.** The fog colours are `cb0[13]`/`cb0[14]` from the haze object (JC2-100). They are not part of this chain, and they cannot darken
  near terrain at the distances of the reference shot; they are separate (JC2-100 G2).
* **Moon.** There is no separate moon light: the single light flips to the moon (JC2-100) and takes its colour from the same row. A separate Bevy "Moon"
  light is therefore not native.
* **Point lights.** The scalar tables of `gradients.bmp` make point lights four times stronger at night than at noon (JC2-100); this is separate.
* **Other `cb0` slots.** A scan of every pixel shader for its `cb0` registers gives: `cb0[1]`, `cb0[2]` camera/projection terms (369 and 8
  shaders), `cb0[3]` sun colour (626), `cb0[4]` light direction, `cb0[7]`/`cb0[8]` light grid, `cb0[9]` shadow fade, `cb0[10]`/`cb0[11]` ambients,
  `cb0[12]` their sum and `.w` shadow-map texel, `cb0[13]`/`cb0[14]` fog, `cb0[15..79]`/`cb0[80..144]` point lights. No other register carries a
  night term; the terrain, General, character, car-paint and facade shaders all read `cb0[3]`, `cb0[10]`, `cb0[11]`.
* **The cutscene override.** The cutscene effect key `FUN_008961d0` sets S+0x204..0x240 through the same setter (0x008966e8), so cutscenes can
  replace the day/night colours; not decoded.

## 8. Corrections to earlier documents

* `light-tables-next.md` section 5 and gap G1: superseded by this brief (the writer exists). The object is S = W+0x10.
* `cloud-shadow-consumers-next.md` sections 5 and 6 ("where weather+0x204..+0x234 come from"): section 2 above.
* `weather.md`: "`landscape_effects.bin` has nothing to do with the sky" and "The rest of the lighting: not found" are wrong; the sun and ambient
  come from `gamezone_gradients.bmp` through `landscape_effects.bin`. (`gradients.bmp` only feeds the cloud card colours, the haze colours,
  the point-light scalar and the facade scalar.)
* `terrain.md` ("the producers of `cb0[3]`, `cb0[10]`, `cb0[11]` were not traced"): section 2 above.

## 9. Proven, inferred, unresolved

* **Proven (byte-checked in the probe):** every address and instruction of the writer chain (sections 1 and 2) including the object
  identity S = W+0x10 and Z = manager+0x30, the complete call-site lists, the sixteen-store setter, the final transform constants, the table
  loader (dimensions, loop counts, exponent, element order), the file fingerprints, the property values (gradient keys, heights, gain 2.2,
  `game_zones` weights 3, hour windows), the hash identities of the key names, the zone ids, the black filler sets and the table values.
* **Inferred:** the variant meaning (variant 0 = high zone intensity), rows 1, 2, 3 consumers, the height/underwater/rain/snow branch arithmetic
  whole-exe displacement sweep and the `cb0` register-file writers agree, JC2-100 and this brief).
* **Unresolved:** see Gaps.

## 10. Gaps (native details still unknown)

1. **Zone cell source.** `FUN_00a5eb50` reads the cell byte from the `CGameZoneMap` (`DAT_01381af8`). The data file behind the map, the default byte for a
   cell without data and the world-to-cell mapping (JC2-100 G3) are unresolved. Blocks: which gradient set a position gets (without it only the
   Default set 7 can be used with confidence).
2. **Branches of `FUN_00a5d390`** (height set 8, underwater 9, rain 10, snow 11): decoded from pseudo-C only; need a byte-level model and the exact rain/snow
   lerp. Blocks: rain/snow/underwater colours.
3. **Variant meaning and `Z[0x1a80+set*4]`** for sets >= 10; which cell intensities occur in the map. Blocks: the variant mix per place.
4. **Rows 2 and 3** go to the cloud object (`FUN_00a55cd0`, +0x9c..+0xcc): which cloud colours they are, and their relation to the cloud tables C/D of `gradients.bmp`.
5. **Row 1** (S+0x214, times 4 at 0x00a58a00 into `light+0x28`): no consumer found in the terrain, General or character shaders; whether anything uses `light+0x28`.
6. **Hour-dependent post-effect blocks** (`FUN_00a5e010`, `FUN_00a6a550`, `FUN_00a55c30`, `FUN_0091d160`): the mapping of the two blocks'
   hashed keys to the post parameters, and which block is night. Blocks: contrast/saturation/exposure at night.
7. **Auto-exposure** (`FUN_00954f20` and the `SampleLum*` constants, cb1[4]): the key, the clamp and the strength. Blocks: the final brightness of night frames.
8. **Landscape effect keys `#178dbdf8`** (values 3.5 and 6; `FUN_00a66c30` end, `FUN_00954f00`, an object at `CLightManager+0x2a2c`, +0x14): a day/night factor
   (3.5 by day, 6.0 at night, ramped 04..06 and 18..22) whose consumer is unknown.
9. **Other callers of `FUN_00a66c30`** (0x00683a40, 0x009000f3) and the cutscene effect key `FUN_008961d0`: when the gradient is recomputed for another
   position, and how cutscenes override the colours.
10. **Quantitative validation.** No calibrated capture exists; the footage cannot yet confirm the absolute night brightness.

## 11. Bounded runtime handoff (not implemented)

Replace the invented sun/moon/ambient curves of `sky.rs` for hours 0..24 by the native tables, with the native layout:

1. `jc2_formats::landscape::GameZoneGradients` (new): parse `gamezone_gradients.bmp` as section 3 (R = byte 2, memory row 0 = top, `rgb ** 2.2`,
   entry 24 = entry 0) into `[[[[[f32; 3]; 25]; 2]; 6]; 12]` (sets 12..15 are filler). Parse `settings\landscape_effects.bin` for the keys of section 2
   (zone type -> set, the gain 2.2, the heights, the set indices 8..11, `max_gradient_N` = 3). Tests: the probe's tables at hours 0, 4, 6, 12, 18, 21 and
   the fingerprint.
2. `jc2-game`: resource `SkyLighting { zone_type, intensity, hour }`; function `light_state(set, intensity, hour)` returning sun/moon colour, sky ambient,
   ground ambient exactly as section 4 base case plus the final transform. Until the zone map is found use set 7 (Default) and intensity 3, and label it
   non-native.
3. Wire to the **single** directional light (the moon is the same light after the flip, `cloud-shadow-consumers-next.md` section 2), to the hemisphere
   ambient `lerp(cb0[11], cb0[10], N.y/2 + 1/2)` and drop the independent Bevy "Moon" light. The native values are HDR colours with a maximum of 5.67; a
   mapping to Bevy units (illuminance, exposure) is a **non-native** choice and must stay labelled, and the 1 s smoothing may be added.
4. Not part of this handoff: the height/underwater/rain/snow branches, the post-effect hour blocks, the auto-exposure and the cloud rows (gaps 2, 4, 6, 7).
