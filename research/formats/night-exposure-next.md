# Auto-exposure, hour-dependent post-effect blocks and key #178dbdf8 (JC2-124)

Status: research brief. No runtime code changed. Probe: `research/tools/night_exposure_probe.py` (91 checks, exits 0; needs a
private `jc2tool` for the data checks, `--allow-skip` turns a missing tool into a skip). All addresses are runtime addresses of
`JustCause2.exe` (no ASLR). Pseudo-C was read from `research/analysis/pseudo`; every instruction or table a claim depends on was
also pinned by a byte check in the probe unless it is listed under *Inferred*.

## Headline

1. **Auto-exposure is real, small and entirely data-driven.** The producer is the `CalculateToneMapping` pass `FUN_0093e660`
   (called once per frame from `FUN_0096f360`, "PostEffects", after the composite). It averages the *clamped LDR* scene luminance,
   forms a target `T = 1 + y * (lerp(key / avg, key / darkest tap, x) - 1)` and eases a 1x1 float texture toward it. The
   composite multiplies the colour by `A + S7 - 1` (`S7 = 1`, so exactly `A`). The numbers are **key 0.32, y 0.08 (night) / 0.10
   (day, 0.04 in Arctic), x 0 (night) / 0.3 (day), A clamped to 0..4, rate `1 - exp(-1.74878 dt)` with `dt` clamped to
   0.01..0.5 s.** With `y` this small it never brightens a night frame by more than about 1.43x linear (1.20x in display value)
   for a scene mean of 0.05 or brighter, and it dims a bright day frame by at most 6 % (mean 0.8: A = 0.94).
2. **The "night block" is `#0a10a074`, the day block is `#0a48aa9b`** (proven by the copy loops of `FUN_00a6a550`). Night before 04:30 and
   from 19:30, day from 05:30 to 18:30, linear blends 04:30..05:30 and 18:30..19:30 (`FUN_00a5e010`). Every one of the 25 settings of
   the post-effect manager that the composite reads is overwritten each frame from these blocks. The night block adds a **blue tint
   (B x 1.6)**, a **luminance-dependent desaturation** (factor `clamp(10 lum + 0.6, 0, 0.95)`), a slightly stronger overlay,
   a later depth-of-field start (150 m against 120 m), and a lower auto-exposure strength.
3. **`#178dbdf8` (3.5 day / 6.0 night) is the brightness gain of the light-source glow sprites** ("Lightsource" pixel shader,
   `cb1[0].x`, owner `CRenderBlockTypeLights` = the object at `CLightManager+0x2a2c`). It does not touch sun, ambient or exposure.
4. **The native display chain is: lit colour (albedo x (sun N.L + ambient + points), fogged) -> clamp to 0..1 and store in an 8-bit sRGB
   target -> post: mix, overlay, saturation, `* A * tint`, `sqrt` -> UNORM.** There is no tone-mapping curve at all. Native colour
   unit 1.0 is display white (before `A` and the tint). That gives a derivable replacement for `LUX_PER_UNIT` (section 5): it is a
   *camera exposure* choice, `LUX_PER_UNIT = pi / e0`, `AMBIENT_PER_UNIT = 1 / e0`, and the current 1,770 is about 0.57x of native
   brightness with Bevy's default camera exposure. The luminance chain and the post parameters can then be added on top.

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `DX10_Shaders_F.shader_bundle` | `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98` |
| `settings\landscape_effects.bin` (2,637 B) | `43fa8c178318d1aadc48d531f5a619f558137c9ea5a7e2093f4476a7d7caa2fd` |
| `settings\post_effects.bin` | `1cdd0728b80c895be3cad9815a7dfdbadf9179e10c87f9ebd30636a8ab1f59c2` |

The probe also pins the sha256 of the eight pixel shaders it reads (SampleLumInitial, SampleLumFinal, CalculateAdaptedLum,
DownScale2x2, HighPass, Scene, DepthOfField, Lightsource). Files were read with `jc2tool cat`/`prop`; nothing is committed.

## 1. Auto-exposure (answer to question 1)

### 1.1 Objects and call chain (proven)

* `P` = the post-effects manager (`CRenderEngine[0x31]`). Its constructor path `FUN_009531d0` (single caller `0x0091cbbc`) allocates the
  0x170-byte **luminance object L** and stores it as `P[0]` **unconditionally** (bytes pinned at 0x953409), then runs `FUN_009432c0`
  (single caller 0x953438), which fetches the shaders by name (`SampleLumInitial`, `SampleLumFinal`, `CalculateAdaptedLum`,
  `DownScale2x2`) and creates the textures named `tone_mapping_downsampling`: 64x64, 16x16, 4x4 and three 1x1, all format 0x29
  (`R32_FLOAT`).
* `FUN_0096f360` (PostEffects) calls `FUN_0093e660` (the only call site, 0x96f5d5) when `P[0] != 0` and the local flag at `[ebp+0x13]` is
  0 (bytes pinned). The flag is true when `P[9]` or the byte `P+0x25` is set (alternate output modes; see Gap 1); it also skips the
  sun halo, `FUN_0096a420` and `FUN_00966a40`. **`FUN_00954f20`, called earlier from the same function, is the sun halo prepass
  ("PreApplySunHalo"), not an exposure routine** (the label in `night-light-next.md` gap 7 was a guess).
* Order inside one frame: sun-halo prepass `FUN_00954f20`, blur chain, HighPass (bloom, reads A), **composite (reads A)**, halo apply `FUN_00923300`,
  then **`FUN_0093e660` updates A for the next read**. The composite and HighPass bind the ring texture that was current at init (`L[0x52]`); the pass writes `L[4 + ring]` and
  toggles `ring`, so the bound texture is rewritten only every second frame (one frame of extra lag, Gap 7).

### 1.2 The passes of `FUN_0093e660` (target, shader, source)

| # | Target | Shader | Source `t0` | `cb1` |
|---|---|---|---|---|
| 1 | 64x64 | SampleLumInitial | the pre-post scene colour (`back_buffer_copy`, 8-bit sRGB view, so linear after decode) | `cb1[0..3].xy` = four tap offsets for the screen size |
| 2 | 16x16 | DownScale2x2 | 64x64 | offsets for a 64-texel source |
| 3 | 4x4 | DownScale2x2 | 16x16 | offsets for a 16-texel source |
| 4 | 1x1 (`L[0x51]`) | SampleLumFinal | 4x4 | offsets for a 4-texel source; `cb1[4] = (S8, S9, S10, 0)` |
| 5 | 1x1 ring (`L[4 + ring]`) | CalculateAdaptedLum | `t0` = previous A, `t1` = the pass-4 result | `cb1[0].x = clamp(dt, 0.01, 0.5)` |

The tap offsets (`FUN_00933700`, pseudo-C) are `(-0.5/N, -0.5/N)`, `(-0.5/N, +1.5/N)`, `(+1.5/N, -0.5/N)`, `(+1.5/N, +1.5/N)` for a source
`N` texels wide/high, added to the target pixel's uv. Samplers: s0 is **bilinear** (`MIN_MAG_LINEAR_MIP_POINT`), s1 point (the code table
`0xf22858` is decoded and byte-checked; `FUN_0093e660` passes (2,2) and (1,1)).

### 1.3 The shaders (proven, disassembly pinned)

* **SampleLumInitial**: output = mean of 4 taps of `dot(rgb, (0.2125, 0.7154, 0.0721))`.
* **DownScale2x2**: mean of 4 taps.
* **SampleLumFinal**: the 4 taps are sampled at `uv' = (uv + off) * 0.5 + 0.25` (the *centre* of the 4x4 texture; with a 1x1 target
  `uv = 0.5` so `uv' = 0.4375 / 0.6875` per axis, texel coordinates 1.75 / 2.75). With `l_i` the four sampled values:

  `avg = max(mean(l_i), 0.01)`, `rAvg = key / avg`, `rMax = max_i max(key / max(l_i, 0.01), 0)`,
  `T = 1 + y * ((rAvg + x * (rMax - rAvg)) - 1)`, with `x = cb1[4].x = S8`, `y = cb1[4].y = S9`, `key = cb1[4].z = S10`.

  Per axis the bilinear weights of the 4x4 texels are (0, 0.375, 0.5, 0.125): a centre-weighted meter that ignores the first row and
  column. (Inferred arithmetic; the taps are what the decoded code produces.)
* **CalculateAdaptedLum**: `A' = prev + (T - prev) * (1 - exp(-1.74878 * dt'))`, NaN -> `T`, still NaN -> 0.5, then `clamp(A', 0, 4)`.
  First frame: the previous texture is the target itself (flag byte `L+0x16c`), so `A = T`.
* **Consumers of A** (`t3` of the composite family, `t1` of HighPass): composite tail (Scene and DepthOfField, identical):
  `C = lum + k (C - lum)` with `k = clamp(lum * cb1[4].y + cb1[4].x, cb1[4].z, cb1[4].w)`, then `C *= (A + cb1[2].x) * cb1[1].rgb`, `out = sqrt(C)`.
  `cb1[2].x = S7 - 1`, `cb1[1] = (S11, S12, S13)` (tint), `cb1[4] = (S15, S16, min(S14, S17), max(S14, S17))`. HighPass:
  `pow(C, S1)`, saturation lerp `S0`, `* A` (feeds the bloom overlay).

### 1.4 Parameters (proven)

**key `S10` = 0.32 in both blocks; strength `S9` = 0.08 (night) / 0.10 (day); weight of the darkest tap `S8` = 0 (night) / 0.3 (day);
`S7` = 1 (so no extra exposure offset); Arctic day slots override `S9 = 0.04`.** Clamps: floors 0.01 on every luminance in the ratio,
A in 0..4, dt in 0.01..0.5. `T` can reach the 4.0 clamp only for `y >= 0.0968` at a black frame, so with these `y` it never does
(maximum 3.48 for night at a black frame).

Steady state `A = T` for a uniform scene (probe table; tap-spread raises `T` only through `x`):

| scene mean luminance (linear, native units) | 0.02 | 0.05 | 0.1 | 0.2 | 0.32 | 0.5 | 0.8 |
|---|---|---|---|---|---|---|---|
| night block (y 0.08, x 0) | 2.200 | 1.432 | 1.176 | 1.048 | 1.000 | 0.971 | 0.952 |
| day block (y 0.10, x 0.3) | 2.500 | 1.540 | 1.220 | 1.060 | 1.000 | 0.964 | 0.940 |

Display effect is the square root: A = 1.43 is +20 % in the encoded value, A = 0.94 is -3 %.

### 1.5 Is it on in the shipped PC variant?

Proven: the luminance object is created for every run and the pass is called each frame unless the alternate-mode flags are set. The
shader family selected for the composite (Scene, DepthOfField, HeatHazeDOF, Underwater) all read A (`t3`). Not proven: the writers of the
two flags (Gap 1). Treated as **on**.

## 2. The hour-dependent blocks of `landscape_effects.bin` (answer to question 2)

### 2.1 Mechanics (proven)

  `Z+0x1ace` is set; the loader `FUN_00a6a550` sets it because the container `post_effects` (hash `d69d4965`) exists in the file. `hour` is
  `CRenderEngine+0xac`, the same clock as the sky colours (`night-light-next.md`).
* The loader keeps four blocks of 25 floats (100 bytes) per gradient set at `Z + (set*4 + 1..4) * 100`. Block `#0a48aa9b` (**day**) is copied to
  slots 1 and 2, block `#0a10a074` (**night**) to slots 3 and 4, for all 16 sets (bytes pinned: `lea esi,[ebx+0xc8]` / `[ebx+0x190]`,
  `lea ecx,[esi-0x64]`, step 0x190). Per-zone overrides patch single keys into slot 1 (`#8eba4969`), 2 (`#2e674fd1`), 3 (`#de6c5fe4`) of
  the set named by `#57677bdc` (bytes pinned; slot 4 `#1eeb7740` follows by sequence).
* `FUN_00a5e010`: with `n` = number of thresholds `<= hour` over `{4.5, 5.5, 18.5, 19.5}` (`#96232764`, pinned): `n` in {2, 3} uses the
  day pair (slot 2 weight 1-w, slot 1 weight w), otherwise the night pair (slot 4, slot 3), `w = intensity byte * Z[0x1a80 + set*4]`;
  for odd `n` the result is then lerped with the *other* pair by `t = (hour - T[n]) / (T[n+1] - T[n])` (dawn night -> day, dusk day -> night).
  Then `FUN_0091d160` writes the 25 floats to the settings (see 2.2).
* Both slots of a pair hold the same block, and the Arctic and Desert overrides are identical across their two slots, so `w` has no effect
  in the shipped data except set 8 (height, `S5` 1500 against 150 m, orientation and use as pseudo-C only).

### 2.2 Key-to-setting-to-consumer table (proven: key hashes, offsets and shader reads are byte/disassembly pinned)

The block parser `FUN_0091a970` (24 keys), the settings loader `FUN_0096f920` (24 keys) and the writer `FUN_0091d160` (24 load/store pairs)
agree on every setting except two key names (S21, S23). `blur` and `saturation` resolve by name (hashes `c07a98af`, `8b639e1e`).

| S | key | role (consumer) | night `#0a10a074` | day `#0a48aa9b` |
|---|---|---|---|---|
| 0 | `#84a1ec42` | HighPass saturation mix `cb1[0].x` | 1 | 1 |
| 1 | `#141874c9` | HighPass exponent `cb1[0].y` | 1 | 1 |
| 2 | `#b98917f0` | overlay strength, bright branch `cb1[3].x` | 0.1 | 0.1 |
| 3 | `#2098968f` | overlay strength, dark branch `cb1[3].y` | 0.8 | 0.7 |
| 4 | `blur` | blur blend (Scene: `cb1[0].z`; DoF: offset of the blend) | 0 | 0 |
| 5 | `#ae110681` | DoF far focus start (m); ramp `sat((eye - S5) / S5)` | 150 | 120 |
| 6 | `#b862814f` | DoF near focus (0 = off) | 0 | 0 |
| 7 | `#463eece2` | exposure offset + 1 (`cb1[2].x = S7 - 1`) | 1 | 1 |
| 8 | `#a4d31fce` | auto-exposure x (weight of the darkest tap) | 0 | 0.3 |
| 9 | `#77724563` | auto-exposure y (strength) | 0.08 | 0.1 |
| 10 | `#566bd67c` | auto-exposure key | 0.32 | 0.32 |
| 11, 12, 13 | `#ab2627cc`, `#e57d0a03`, `#679def2a` | tint R, G, B | 1, 1, **1.6** | 1, 1, 1 |
| 14 | `saturation` | saturation clamp end | 0.95 | 1 |
| 15 | `#cc44e277` | saturation offset | 0.6 | 0.9 |
| 16 | `#fad89ab2` | saturation slope vs luminance | 10 | -0.8 |
| 17 | `#15ba5c5b` | other clamp end | 0 | 1 |
| 25 | `#bd54de7b` | DoF blend gain `cb1[0].w` | 0.3 | 0.4 |
| 26 | `#91fedf25` | heat-haze selector (`> 0.001` picks the HeatHazeDOF family) and scale `0.004 * S26` | 0 | 0 |

Derived: saturation factor night `k = clamp(10 lum + 0.6, 0, 0.95)` (0.6 for black pixels, 0.95 from lum 0.035), day `k = 1` (clamp range
[1, 1]); far depth-of-field blends `S25` of the blurred image from `S5` m (full at `2 S5`); night B channel x1.6 is `sqrt(1.6) = 1.265`
in display value.
Overrides: Arctic day slots (1, 2) tint B 1.25, `S9 = 0.04`, saturation 0.9, `S2 = 0.25`, night slots neutral (1, 1, 1); Desert slots 1, 2
tint (1.34, 1.13, 0.89), `S5 = 50`, `S26 = 1` (heat haze), saturation 0.87 and others; **Underwater** (set 9) all slots tint B 1.3, `S5 = 8`,
`S25 = 0.65`, `blur 0.15`; set 8 (height) `S5` 150 / 1500. All verified in the probe from the file.

### 2.3 Corrections

`settings\post_effects.bin` keys `bloom`, `exposure`, `contrast` (the "0.87 exposure, 1.1 contrast" values quoted in `weather.md`,
`night-light-next.md` section 7 and `underwater-render-next.md`) are **never read**: the hashes `52adb0ec`, `c2c0a469`, `f575d25e` occur nowhere in
the exe (probe). The file's live keys are `blur`, `saturation`, `#679def2a`, `#84a1ec42`, `#ab2627cc`, `#ae110681`, `#e57d0a03`, and they
are all overwritten by the hour blocks every frame. The `cb1[2].x` exposure offset is therefore `S7 - 1 = 0`, not -0.13.

## 3. Key `#178dbdf8` (answer to question 3)

* File: container `#178dbdf8` with `#7441ddd4 = 3.5` (day) and `#b8053eee = 6` (night); the loader stores them at `Z+0x1af8` / `Z+0x1af4`.
* Producer: `FUN_00a66c30` end (only call site of `FUN_00954f00`, 0xa66f42, pinned; it runs when `DAT_011907c4` = `CLightManager` exists). With
  `h` = hour: `f = 1` for `h >= 22`, `(h - 18) / 4` for `18 <= h < 22`, 0 for `6 < h < 18`, and for `h <= 6` `1 - clamp((h - 4) / 2)`.
  The value is `3.5 + (6 - 3.5) f` (constants 18, 0.25, 6, 4, 0.5 pinned; branch structure from pseudo-C). So 6.0 from 22:00 to 04:00,
  ramping down 04:00..06:00 and up 18:00..22:00.
* Sink: `FUN_00954f00` stores it at `[CLightManager+0x2a2c] + 0x14`. That object is `CRenderBlockTypeLights` (ctor `FUN_00969b50`, which sets the
  default `[+0x14] = 1.0`, `+0x18 = 200`, `+0x1c = 800`, `+0x20 = 0.25`, `+0x24 = 1.0`; vftable 0xf2337c).
* Consumer: vtable slot 12 `FUN_0093d940` writes it unchanged as `cb1[0] = (this[+0x14], 0, 0, 0)` of the **Lightsource** shader pair (bytes pinned).
  The Lightsource pixel shader outputs `sprite.x * cb1[0].x * vertex colour`, alpha x 1.5 (disassembly pinned). It is the **intensity gain of
  the light-source glow sprites** (point-light halos), 3.5 by day and 6.0 at night, not a light colour, not exposure, not ambient.

## 4. Display chain, native (proven unless marked)

1. Scene shaders write linear lit colour to the scene target, an `R8G8B8A8_TYPELESS` texture viewed as **sRGB** (`underwater-render-next.md`
   section 2), so every channel is clamped to [0, 1] and quantised to 8 bits (sRGB-encoded) at store. There is no HDR scene buffer.
2. The post pass reads it through an sRGB view (linear again), mixes the blur, overlays the bloom, applies the saturation clamp, multiplies by
   `(A + S7 - 1) * tint`, writes `sqrt(C)` into a UNORM swap-chain buffer (gamma 2.0, not the sRGB curve).
3. Therefore native colour unit 1.0 = display white when `A = 1` and the tint is 1. The sun colour of 5.667 is only a multiplier of the albedo: for
   example the noon `Default` sun at `N.L` 0.707 is 4.0 in red, so any albedo above 0.25 clips in that channel. The auto-exposure scales *after* the clip.

## 5. Replacing `LUX_PER_UNIT` (answer to question 4)

Provable: the native pixel is `sqrt(clamp01(albedo * (sun N.L shadow + ambient + points)) * A * tint)` after the post mix. The Bevy mapping is a
camera/exposure identity, not an arbitrary scale:

* Bevy's diffuse term for a directional light is `albedo / pi * lux * N.L * e0` and the ambient term is `albedo * brightness * e0`, where `e0` is the
  camera exposure multiplier (for Bevy's `Exposure` this is `1 / (1.2 * 2^ev100)` in the versions known to the author; **verify against 0.19**). To
  reproduce the native product with `A = 1`: **`LUX_PER_UNIT = pi / e0` and `AMBIENT_PER_UNIT = 1 / e0`** (the existing `AMBIENT_PER_UNIT =
  LUX_PER_UNIT / pi` already has this ratio). With the default `ev100 = 9.7` (`e0 = 1.0e-3`) that is about 3,135 lux and 998 per unit; the
  current 1,770 renders about 0.565x of native (display value 0.75x). This conclusion is independent of any guessed constant.
* What the replacement cannot supply by itself: the clamp at 1.0 (needs a post pass or an LDR target), the `sqrt` output curve (Bevy encodes sRGB:
  up to 0.037 apart), the tint/saturation/overlay/DoF blocks and `A`.

Not provable here: absolute `A` in a running game, because it depends on the average native-unit luminance of the frame (Gap 4); the table
in 1.4 gives `A` as a function of it.

## 6. Proven, inferred

* **Proven (byte-checked in the probe):** the call sites of `FUN_0093e660`, `FUN_00954f20`, `FUN_00954f00`, `FUN_00a5e010`, `FUN_0091d160`,
  `FUN_009531d0`, `FUN_009432c0`; unconditional creation of L; the guard bytes before the call; the `cb1[4] = (S8, S9, S10, 0)` and dt-clamp
  code; the texture sizes; sampler codes; the 24-key tables of the three post-setting functions and their agreement; the slot layout and the
  night/day copy loops; the threshold loads; the ramp and clamp constants; the vtable slot and the reader of `+0x14`; fingerprints; the
  property values of both files; the shader disassembly of all eight shaders; the dead `bloom`/`exposure`/`contrast` keys.
  pseudo-C), pass order and bound sources of `FUN_0093e660` (pseudo-C), slot 4 key `#1eeb7740` by sequence, the structure of the
  `#178dbdf8` ramp and of the height set, the bilinear reading of sampler code 2, the on-state of the pass in the shipped build, Bevy's
  exposure formula.

## 7. Gaps (native details still unknown)

1. **Alternate-mode flags `P[9]` and `P+0x25`** (`FUN_0096f360`): who sets them and their defaults. If either is set the tone-mapping pass, sun
   halo, `FUN_0096a420`, `FUN_00966a40` are skipped. Blocks: certainty that auto-exposure runs in the shipped PC build (assumed on).
2. **Post-manager defaults and locks:** the constructor values of `S(0..27)` before the first block application, `S(21..24)` consumers and
   keys (`d8bb32a4`, `1b04bb06` vs `be6bbd60`, `c0a7bfea`, the latter two at 0x5642da/0x5642e8), the smoothing factor `P+0xf0` of `FUN_0091d160`
   (instant if 0), and the lock bytes `P+0xb8..` set by cutscene code (`FUN_008fb3e0` calls `FUN_0091d160`, `FUN_00554b00`, `FUN_0088cbf0`). Blocks:
   cutscene and zone-change transitions of exposure/tint.
   adaptation speed.
4. **No calibrated capture:** the mean native-unit luminance of real frames is unmeasured, so absolute day/night `A` and the visibility of the
   blue night tint on screen are unvalidated. A fixture must render a frame with the native chain and compare a retail capture.
5. **Meter geometry:** the tap offsets (`FUN_00933700`), the 64x64 stage's source size (swap-chain size passed to `FUN_009531d0`), and bilinear
   filtering of `R32_FLOAT` need a pixel-level fixture (one frame through the ported chain against a retail capture).
6. **Zone overrides:** the blend orientation for set 8 (height, `S5` 150 against 1500) and which camera altitude selects it (`FUN_00a66c30`
   pseudo-C only); the underwater/rain/snow interplay with `FUN_00a5d390`. Blocks: DoF distance at altitude.
7. **Ring-texture staleness:** the composite and HighPass keep the init-time ring texture (`L[0x52]`), the pass alternates `L[0x52]`/`L[0x53]`; the
   frame-level effect (A refreshed every second frame) and the content of the texture on frame 0 are inferred, not run.
8. **Lightsource glow:** the vertex-shader constants `+0x18..+0x24` (200, 800, 0.25, 1.0) and their meaning (size/fade), the light list
   (`global/all_lights.light_info`), and the visible effect of 3.5 against 6.0. Only the pixel-shader role of `#178dbdf8` is proven.
9. **Other consumers of S2/S3/S0/S1** in the Heat-haze, Stereo and Underwater families, and cutscene effect keys that set S (`FUN_008961d0`
   writes S through `FUN_00922630`). Blocks: full composite parity in those families.
10. **Final encode:** the exact `sqrt` output against Bevy's sRGB encode is proven to differ (underwater brief); how to reproduce it in the
    Bevy pipeline (custom final pass) is an implementation decision.

## 8. Bounded Rust handoff (not implemented)

Order of work; each step is independently testable and none needs the unproven parts.

1. **`jc2_formats::landscape::PostEffectBlocks` (new).** Parse `settings\landscape_effects.bin` `post_effects`: the two blocks `#0a10a074` (night)
   and `#0a48aa9b` (day), the thresholds `#96232764` (`#f10e7ec1`, `#220137a5`, `#88d36ba1`, `#f56ffcb7`), the per-zone override containers
   (`#57677bdc` = set, slots `#8eba4969`, `#2e674fd1`, `#de6c5fe4`, `#1eeb7740` = 1..4). Represent a block as `[f32; 27]` indexed by S(n) using the
   key table of section 2.2 (keys not present keep the previous value). Tests: the probe's night/day values, the 04:30..05:30 / 18:30..19:30
   linear blends, Arctic overrides.
2. **`jc2-game` resource `PostEffectState`** updated each frame from the hour and the gradient set already used by `sky_light.rs`
   (stand-in set 7 until the zone map is wired, as there). No smoothing (Gap 2).
3. **Exposure identity (replaces the policy):** fix the camera exposure `e0`, set `LUX_PER_UNIT = pi / e0` and `AMBIENT_PER_UNIT = 1 / e0` in `sky_light.rs`
   (derived constants, document section 5), keep `Tonemapping::None`. This alone corrects the global brightness to the native product.
4. **Phase 1 post pass (custom fullscreen node):** clamp the HDR scene to [0, 1], optionally 8-bit sRGB quantise, then the composite tail of
   section 1.3 with `A = 1`: saturation clamp `(S15, S16, min(S14, S17), max(S14, S17))`, `* (S7 - 1 + A)`, `* (S11, S12, S13)`, `sqrt`. This gives the night
   blue tint and dark-pixel desaturation immediately. Overlay (S2, S3) and DoF (S5, S25) follow in a second step.
5. **Phase 2 auto-exposure:** a compute (or three small fullscreen) passes implementing 1.2/1.3 on the previous frame's clamped colour, 1x1 float
   state, `dt` clamp 0.01..0.5, `A` clamp 0..4; feed `A` to step 4 and to the custom materials' `exposure` uniforms. Label the meter geometry
   as decoded-not-validated (Gap 5).
6. **Lights:** `#178dbdf8` as the gain of the glow-sprite shader when those sprites exist (Gap 8); not part of the exposure work.
7. Not part of this handoff: the cutscene overrides, heat haze, stereo, the unknown flags (Gaps 1, 2, 9).

## 9. Implementation status (JC2-134 phase 1, on branch)

Steps 1 to 4 of section 8 are implemented; step 5 (auto-exposure `A`, phase 2) is not (`A = 1`, labelled non-native).

* `jc2_formats::weather::PostEffectBlocks` (in `weather.rs` next to `LandscapeEffects`, not in `landscape.rs`, which is the terrain
  materials module): night/day blocks, thresholds, zone overrides (slots 1..4), `params(set, w, hour)` = `FUN_00a5e010`.
* `jc2-game` `post_effects.rs` (`PostEffectState`, `PostPass`, `post_effects.wgsl`): the settings by hour and zone set, and the final
  fullscreen pass (clamp, saturation clamp, `* (A + S7 - 1) * tint`, `sqrt`). The game camera has no HDR (the 8-bit sRGB main texture
  is the native clamped 8-bit scene target), `Tonemapping::None`, `DebandDither::Disabled`, `Exposure::BLENDER`.
* Gap 10 decision: the surface is sRGB, so the pass returns the sRGB decode of `sqrt(C)`; the hardware encode restores `sqrt(C)`
  exactly (no custom swap-chain format, no double encoding).
* `sky_light.rs`: `lux_per_unit() = pi / e0`, `ambient_per_unit() = 1 / e0`, `e0 = exp2(-ev100) / 1.2` verified in Bevy 0.19.1
  (`bevy_camera::Exposure::exposure`; `pbr_functions.wgsl`: `view.exposure * (direct + indirect)`; `Fd_Burley` ends in `1 / PI`;
  ambient `colour * brightness`): 3,135.6 lux and 998.1 per unit at ev100 9.7 (`e0` = 1.0019e-3).

## 10. Implementation status (JC2-137, phase 2 auto-exposure, on branch)

Step 5 of section 8 is implemented. Code: `auto_exposure.rs` (pure maths and CPU reference with unit tests against the probe
table), `auto_exposure.wgsl` (the same arithmetic as four compute dispatches), `post_effects_render.rs` (render-graph node, replaces
Bevy's `FullscreenMaterialPlugin` because the draw needs the exposure buffer), `post_effects.rs/.wgsl` (`PostPass` gains the meter
vector `(S8, S9, S10, dt)`; the composite multiplies by `applied + S7 - 1`).

* **Design:** GPU only, no readback. Per view one storage buffer holds the 64x64, 16x16, 4x4 luminance maps and the state
  (`applied`, `latest`, `inited`). Each frame one node runs the compute pass on the pre-post scene (`post_process_write().source`, the 8-bit
  sRGB main texture) and then the composite. The composite uses `applied` = the value metered in the previous frame: **one frame of
  latency** (the native pass runs after the composite; a possible second native frame from the ring texture, Gap 7, is not modelled).
* **Native (brief 1.3, shader disassembly):** luminance weights, floors 0.01, `T`, rate 1.74878, dt clamp 0.01..0.5, A clamp 0..4,
  NaN rules, first frame `A = T`, `x`/`y`/`key` from the hour blocks (night 0 / 0.08, day 0.3 / 0.10, Arctic day y 0.04, same blend).
* **Inferred (Gaps 3, 5, 7):** tap geometry (offsets -0.5 / +1.5 source texels, bilinear, final stage `0.5 uv + 0.25`), 64x64 map from the
  full frame, `dt` = real frame delta (`Time<Real>`), the latency, bilinear filtering of the float maps (done by hand on buffers).
  **Non-native choice:** on frame 0 the frame itself already uses `A = T` (the native texture content is unknown).
* **Materials:** `A` is applied after the 0..1 clamp (section 4), so the material `exposure` uniforms (`view.exposure`, the camera
  identity) are unchanged and take no `A`.
* **Capture check** (`_extract/jc2-137/`, start base tile 29,18, 1280x720, converged after 40 s): noon (Arctic, y 0.04) measured
  `A` = 1.065 (median of after^2 / before^2) against 1.063 from the CPU meter on the phase-1 capture; 22:00 measured 1.733 against 1.757
  (the proxy uses display values after saturation and tint, so about 1.5 % off is expected).
