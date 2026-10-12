# Sun disc, sun glow, sun halo ("sunbeam") and lens flare (JC2-174)

11 October 2026. Research only: no Cargo, GPU, game launch, runtime edit or asset copy. Closes Gap 3 of `sky-gradient-next.md`
(JC2-151, section 8) and the sun half of Gap 3 in `weather.md`. Addresses are retail VAs of `JustCause2.exe` sha256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`; shader offsets index `DX10_Shaders_F.shader_bundle` sha256
`db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`; four textures of `textures\atmosphere\` are read through `jc2tool`
(nothing is copied or committed).

Reproduce (161 checks, exit 0; the exe and bundle are read, the texture checks need `jc2tool`):

```
python -B research/tools/sun_glow_probe.py <game folder> [--jc2tool <exe>] [--json] [--allow-skip]
```

The probe pins 36 function bodies by sha256 (plus the 33 of `sky_render_state_probe.py`), decodes the twelve vertex additions of the
two quads from the exe bytes (a small SSE interpreter), re-reads every float constant, call order, vtable slot and call-site immediate the
formulae rest on, disassembles eleven shader stages of six bundle entries, replays the D3D10 state of the sun pass with the setters of JC2-173,
checks the four textures and runs fixtures of every model below. Legend: **proven** = read from the binary and pinned by the probe; **inferred** = one
unchecked link; **unresolved** = not traced (see Gaps).

## 1. Headline

1. **There are three separate sun effects, and the task text's "sunbeam block" is the smallest of them.**
   * **Disc and glow billboards** (the "CSun" of the task): two camera-centred quads in the *scene* pass `SUN` (pass id 0x26), built by
     `FUN_00a6ea10` and tinted every frame by `FUN_00a594e0`. `sun_gradient_dif.dds` (128 x 128 glow) sits at 500 m with half-size 800;
     `sun_dif.dds` (64 x 64 disc) at 1500 m with half-size 80.
   * **Sun halo** (the real user of `sunbeam_dif.dds`): a screen-space post effect, `FUN_00954f20` (prepass, "PreApplySunHalo") +
     `FUN_00923300` ("ApplySunHalo"): sixteen additive quads centred on the sun, shaped by the sunbeam alpha and masked by a blurred
     occlusion test.
   * **Lens flare**: seven additive sprites on the line through the screen centre and the sun, `FUN_00966a40`, the last effect of the
     PostEffects task (`FUN_0096f360`).
2. **The `sunbeam` shader pair (PS = texture * 0.25, VS adds `cb0[4].xyz`) is dead.** No code references the name string `SUNBEAM` except the
   pass-name table, and no `CreatePass(0x47)` exists. It never draws in retail. The sun halo uses the shader `HaloMask`, which multiplies the
   mask by the colour by `sunbeam_dif.dds` **alpha**.
3. **Native levels (before the scene clamp), sun colour `c` = `cb0[3]` = light+0x18, `L` = the light direction `cb0[4]` (moon-flipped):**
   disc adds about **(1.97, 1.93, 2.0) x alpha** on the sun axis at noon (tint 2, 2.0 clamps in an 8-bit target); glow adds up to **0.39 x alpha**;
   halo adds **16 x I x c** at the sun centre with `I = 0.04 x max(1 - 3|L.y|, 0.1)` (0.004 near noon, 0.0297 at 06:00 and 18:00);
   each lens flare adds `texture x min(1, c x a)` with `a <= 0.04` for the shipped S(0x13) = 1 (a 0.2 cap is never reached).
4. **Everything hour dependent is a function of three numbers:** the sun orbit (position), `|L.y|` (colours and halo strength) and the game hour
   (lens-flare night fade). The disc and glow tints do **not** use the sun colour `c`; the halo and flare do.
5. **Correction of `light-tables-next.md` section 5 (last bullet):** the billboard tint (`FUN_00a594e0`) is disc scale 2.0 and **glow** scale 0.4;
   the "moon billboard" 0.4 there is the sun gradient quad. The moon is a separate pass (0x27, JC2-130).

## 2. Disc and glow billboards (pass `SUN`, 0x26)

### 2.1 Construction (`FUN_00a6ea10`, called once from `FUN_00a52f30`; `this` = the sky object S = W + 0x10)

* Pass: `CreatePass(0x26, 1)`; `S[0x7f]` = the pass. Calls in order: `+0x5c(1)` (enable, flag bit 3 of the pass flag word at pass+0xcd0),
  `+0x48(0)` (bit 1 cleared), `SetParam(0, 0)` (depth write off), `SetParam(1, 1)` (depth test on), `+0x60(0)` (bit 4 cleared). All other
  parameters are the `CRenderPass` defaults of JC2-173 (1, 1, 7, 7, 0, 0 and 0x11 = 1). `FUN_00a472f0` toggles the pass with `W[1]` through
  `FUN_00a53440` (`+0x5c`), then runs `FUN_00a58a00` and `FUN_00a594e0` while `W[1]` is set (default 1).
* Two `CRenderBlockOpen` blocks (`FUN_0095d020`, type name `Open`, shader pair `open`), each a 6-vertex triangle list (no index buffer):
  `S[0x31]` the **glow** (texture `sun_gradient_dif.dds`) and `S[0]` the **disc** (`sun_dif.dds`). Vertices (byte-interpreted, identical order):
  `(-h, y, -h) uv (0,0)`, `(h, y, h) (1,1)`, `(-h, y, h) (0,1)`, `(-h, y, -h) (0,0)`, `(h, y, -h) (1,0)`, `(h, y, h) (1,1)`, colour (1,1,1,1).

| block | texture | h | y | half angle |
|---|---|---|---|---|
| glow `S[0x31]` | `sun_gradient_dif.dds` A8R8G8B8 128 x 128 | 800 | 500 | atan(800/500) = 58.0 deg (visible glow: inner 36.5 texels = 0.57 of the half width, about 42 deg) |
| disc `S[0]` | `sun_dif.dds` DXT5 64 x 64 | 80 | 1500 | atan(80/1500) = 3.05 deg (alpha is 1 out to about 0.3 of the half width, 0.85 at 0.5 = 1.5 deg) |

* Block flags (bytes `+0x33 = 1`, dword `+0x58 = 2`, byte `+0x2d = 1`, dword `+0x40 = 1`, `byte +0x10 |= 8` on both): blend factors
  (src = block+0x54 = **SRC_ALPHA** by default, dst = **ONE**), depth test on, and the byte `+0x10 = 0x0e`: bit 1 **camera-relative**,
  bit 2 **CLAMP** addressing, bit 3 **depth forced to 0**.
* Instances: the disc instance (`S+4`) takes the matrix `S+0x178` (`FUN_00a53460` of the hour, the sun orbit), the glow instance (`S+0xc8`)
  `S+0x1b8` (a copy made at the end of `FUN_00a58a00`); `FUN_00a594e0` re-sets both every frame with `FUN_007682e0`. The matrix is the
  3 x 3 orbit rotation with zero translation: row 1 is **toSun**, rows 0 and 2 span the plane perpendicular to it (both quads are flat squares in that
  plane, face-on when the camera looks at the sun).
* Draw (`FUN_00927e80`, block vtable +0x74): camera-relative flag -> translation = the camera position; `world = cam + x row0 + y row1 + z row2`,
  so the **disc is centred at camera + 1500 toSun and the glow at camera + 500 toSun**. The vertex shader forces clip z = 0 (cb1[4].x = 1), which
  with the reverse-Z depth (GREATER_EQUAL, JC2-173) means the quads draw only where no geometry wrote depth: **the sun is behind everything.**

### 2.2 Shader (`open`, proven by disassembly)

VS: `clip = pos * (cb1[0..3]) `, z replaced by 0 when `cb1[4].x != 0`; `o1 = uv`; the vertex colour is packed in `v1.z`: `FUN_0092c800` stores
`z = q3 * 4096 + q2 * 64 + q1 / 64 + q0` with `q = floor(63 c)` and the VS unpacks `frc(z * (1, 1/64, 1/4096, 2^-18))`. **White (1,1,1,1) unpacks to
(0.984375, 0.999756, 0.999996, 0.99999994)**: the red channel of every `Open` block is 1.6 % under white (reproduced in the probe).
PS: `out = (tex + cb1[1].x (1 - tex)) * cb1[0] * vertexColour`; `cb1[1] = (0,0,0,0)` here, `cb1[0]` = the block tint (`block+0x1c..0x28`).
The target receives `out.rgb x out.a` (SRC_ALPHA, ONE): **`dst += tex.rgb x tint.rgb x vc.rgb x tex.a x tint.a x vc.a`**. The textures are white
(`sun_dif` rgb (255,251,255) everywhere, the shape is the alpha), so the profile is linear in the alpha.

### 2.3 State (probe replay of `FUN_00950ee0` with the setters of JC2-173)

Depth test on, write off, GREATER_EQUAL; blend enable, src SRC_ALPHA, dst ONE, alpha ONE / ONE; write mask RGB; cull NONE, solid, depth clip on,
MultisampleEnable 1 (pass parameter 0x11 = 1, the default; the sky passes set it to 0; the replay assumes the engine's global MSAA gate is on, as in JC2-173); sampler 0: CLAMP on all axes, `MIN_MAG_LINEAR_MIP_POINT` (0x14),
MaxLOD 0, anisotropy 1. The type state is `FUN_0094d6d0`, the block state `FUN_00968090`.

### 2.4 Tint (`FUN_00a594e0`, every frame after `FUN_00a58a00`)

`f = max(0.9 - 1.5 |L.y|, 0)` with `L` the **flipped** light (`S+0x244`, the moon after 05:40 / 18:20 of JC2-100):
disc tint `(2, 2 (1 - 0.75 f), 2 (1 - f), 1)` (`DAT_0108b520` = 2.0), glow tint `(0.4, 0.4 (1 - 0.4 f), 0.4 (1 - f), 1)` (`DAT_0108b51c` = 0.4); both
stored as rgba at block+0x1c. The sun colour `c` is **not** used. Probe table (sun elevation by `jc2tool weather` orbit, `f`, tints):

| hour | sun elev | `abs(L.y)` | f | disc tint | glow tint |
|---|---|---|---|---|---|
| 05:30 / 18:30 | 0.3 deg | 0.0052 | 0.892 | (2, 0.662, 0.216) | (0.4, 0.257, 0.043) |
| 06:00 / 18:00 | 4.95 | 0.0862 | 0.771 | (2, 0.844, 0.459) | (0.4, 0.277, 0.092) |
| 07:00 / 17:00 | 14.3 | 0.2469 | 0.530 | (2, 1.206, 0.941) | (0.4, 0.315, 0.188) |
| 08:00 / 16:00 | 23.4 | 0.3967 | 0.305 | (2, 1.543, 1.390) | (0.4, 0.351, 0.278) |
| 10:00 .. 14:00 | 38.6 .. 45 | 0.62 .. 0.71 | 0 | (2, 2, 2) | (0.4, 0.4, 0.4) |
| 00:00 | -32.3 | 0.5346 (moon) | 0.098 | (2, 1.853, 1.804) | (0.4, 0.384, 0.361) |

### 2.5 Levels (probe, noon, on the sun axis, before the target clamp)

Disc `dst += (1.9687, 1.9289, 2.0)` (alpha 1 at the centre; the 8-bit scene target clamps it, whether the scene target is 8-bit is Gap 2). Glow
`dst += (0.3887, 0.3948, 0.3949)` at the axis (bilinear mean alpha 251.75/255; the peak texel is 255). Alpha along x in steps of 1/16 of the
half width (0, 0.0625, ..., 0.5): glow 0.987, 0.914, 0.810, 0.685, 0.550, 0.414, 0.280, 0.159, 0.062; disc 1.0 (six steps), 0.997, 0.966, 0.849.

## 3. Sun halo (post effect, `FUN_00954f20` + `FUN_00923300`)

Object `H` = `P[0xe]` (0x74 bytes, ctor `FUN_0091be90`: `H+0x60 = 0.04`, `H+0x64 = 0.07`; resources `FUN_009329c0`), `P` = the post manager.

### 3.1 Gate and sun position (prepass)

* Off (and `H[0x1a] = 0`) if the camera is below `S(0x12)` = 200 m (`FUN_009390e0`: camera y < `P+0x8c`) **or** `V.y > 0.07`, where `V` = `P+0xf8` is the
  unflipped light vector (`W+0x260`, copied there every frame by `FUN_005907b0`, which also writes `S(0x14)` = hour and `S(0x12)` = 200): the halo
  shines while the sun is above about -4.0 deg.
  normalize(eye - target)); view space by the inverted camera matrix (`thunk FUN_004cf830`, inferred to be the inverse), then the camera projection
  (`camera+0x64..0xa0`, row-vector convention); it must be in front (`view z < 0`). Screen position `s = (W (x + 1) / 2, H (1 - (y + 1) / 2))`.

### 3.2 Mask (`AlphaMask` + two `Blur` passes into `sun_halo_mask_texture`, 255 x 256, R8G8B8A8)

* Square of 255 x 255 screen pixels around `(int)s` (half = 127), clipped to the screen; the intensity `I = S(0x13)` (constructor 1.0, not written by the hour
  block, JC2-103/124) loses `overshoot / 254` for every side that leaves the screen (`x0 < 0`, `y0 < 0`, `x1 >= W`, `y1 >= H`); `I < 0` or a
  rectangle off the screen switches the halo off.
* The source rectangle is snapped to a 32-pixel grid (`(x0 >> 5) * 32`), 255 x 255, and drawn into the mask with `AlphaMask`:
  arguments); with reverse Z any geometry hides the sun, and the scene **alpha** at the pixel halves it per 0.25 (0.5 alpha hides it fully).
* Two 7-tap blur passes (`Blur` PS: seven taps of weight 1/7, sparse hexagonal pattern: offsets `(0,0) (r,0) (-0.5r, 0.866r) (-0.5r, -0.866r) (1.5r,
  0.866r) (-1.5r, 0.866r) (0, -1.732r)`) with `r = 1.3125` texels (mask to temp) and `0.875` texels (temp to mask).

### 3.3 Apply (`FUN_00923300`, after the composite)

* 16 quads (96 vertices x 24 bytes, `Draw(4, 0, 32 primitives)`), centred on `s / (W, H)`, **half height `0.13 x 1.1^j` of the screen height**
  (0.13 .. 0.543), half width = height / aspect (square in pixels). Mask uv of every quad = the same rectangle
  `((x0 - snapx + 32) / 255, (y0 - snapy + 32) / 255, (x1 - snapx - 32) / 255, (y1 - snapy - 32) / 255)` (plus the off-screen overshoot terms): the mask is
  stretched over each layer. Sunbeam uv = the full 0..1 square.
* PS (`HaloMask`): `out.rgb = mask.x x cb1[0].rgb x sunbeam.a`, `out.a = 1`; state: blend enable, SRC_ALPHA / ONE (so **additive**), pixel flag 1, CLAMP, linear
  filter on both slots, t0 = mask (`H[6]`), t1 = `sunbeam_dif.dds` (shared pointer at `H+0x10`).
* Colour: `a = max(0.8 - 1.8 |L.y|, 0)`, `H[0x17] <- H[0x18] (0.04) x max(1 - 3 |L.y|, 0.1) x H[0x17]`, `cb1[0] = (c.x I, (1 - a) c.y I, c.z max(1 - 2a, 0) I, 1)`
  with `c` = `light+0x18` (`FUN_004813d0`; the sun colour of `night-light-next.md` including the lightning boost and the underwater blend) and
  `L` = `light+0x44` (`FUN_005351e0`, moon-flipped).
* Native levels, 16 layers at the centre with mask 1 and sunbeam alpha 1 (the alpha is 255 over about 31 x 37 central texels of the 64 x 64 picture);
  radial sum of the sunbeam alpha over the layers (720p, sun mid-screen, mask 1): 16.0 at 0 and 25 px, 15.2 at 50, 8.7 at 100, 1.6 at 200, 0.02 at 300, 0 beyond.

| case (`c`) | layer colour (`I c`) | centre level (x 16) |
|---|---|---|
| `c = (1,1,1)`, 12:00 (`I = 0.004`) | 0.004 | 0.064 |
| `c = (1,1,1)`, 06:00 / 18:00 (`I = 0.0297`, `a = 0.645`) | (0.0297, 0.0105, 0) | (0.474, 0.169, 0) |
| Default zone, noon, `c = (5.67, 5.05, 3.70)` | (0.0227, 0.0202, 0.0148) | (0.363, 0.323, 0.237) |
| Arctic, 06:00, `c = (2.70, 0.91, 0.47)` | (0.0801, 0.0096, 0) | (1.281, 0.153, 0) |

The halo is added **after** the composite (tone mapping, `sqrt` encode, auto-exposure, MotionBlur follows), so these are display-referred additions to the
output target, not scene-referred ones.

## 4. Lens flare (`FUN_00966a40`)

Object `P[0xf]` (0x20 bytes, ctor `FUN_009353d0`: `+0x1c = 1.0`; resources `FUN_00950c70`: `lensflare_dif.dds` 512 x 128 DXT1 = four 128 x 128 cells, shader `LensFlare`,
vertex buffer 42 x 24 bytes, texture `+8` = the halo's mask texture).

* Gates: camera y > 200 m, sun point (camera + 10000 toSun) in front of the camera, `V` as above is **not** required (the flare has its own night fade).
  Offset from the screen centre `(dx, dy) = (sx - W/2, sy - H/2)` in pixels; `fade = 1 - hypot(dx, dy) / (0.4 W)`.
* Alpha: `a = clamp(S(0x13) x fade x 0.04, 0, 0.2) x lens[+0x1c] x nightfade`, `nightfade = max(1 - max(|hour - 12| - 6, 0)^3, 0)` (pow, exponent 3.0 at `0xe5c530`):
  1 from 06:00 to 18:00, 0.875 at 18:30, 0 from 19:00 and before 05:00. With the shipped values the maximum is **0.04** (sun on the screen centre).
* Colour `(min(1, c.x a), min(1, (1 - b) c.y a), min(1, c.z max(1 - 2b, 0) a))`, `b = max(0.8 - 1.8 |L.y|, 0)` (the halo's terms), PS cb1[0].
* Seven sprites `i = 0..6` with position `r_i = (1, 0.5, 1/3, 0.125, -0.5, -0.25, -0.1818)`, size `s_i = (1, 0.4, 0.25, 0.6, 0.8, 0.25, 0.25)` and strip cell
  `(0, 1, 2, 1, 2, 3, 1)` (tables at `0xf22e50` / `0xf22e34`, cell bytes by `mov al,1; mov cl,2` at `0x966c52`): centre `(r_i dx + W/2, r_i dy + H/2)`, **half size `s_i x W x 0.2` pixels**,
  uv `u = cell/4 .. cell/4 + 1/4`, `v = 0 .. 1`; second uv `(0.5, 0.5)`.
* PS (`LensFlare`): `out.rgb = strip.rgb x cb1[0].rgb`, `out.a = mask_texture(0.5, 0.5).a` (inferred binding: t0 = the strip bound with `vtable+4(ctx, 0)`, t1 = the
  halo mask at slot 1): the **blurred sun visibility** (depth and scene alpha) at the sun pixel multiplies every flare. Blend SRC_ALPHA / ONE, pixel flag 1, CLAMP, linear.
  Draw `Draw(4, 0, 14)`. Cell centres (255-level): (241,240,241), (230,227,214), (252,253,252), (247,247,247); the strip is opaque DXT1.
* Levels, sun at the screen centre, white `c`: 0.04 per channel at noon (0.04, 0.0142, 0) at 06:00 / 18:00; with the Default-zone noon `c`: (0.2268, 0.202, 0.148)
  times the strip colours and the mask.

## 5. Frame order and where each effect lives

`FUN_0096f360` (PostEffects): halo prepass `FUN_00954f20` -> blur chain / HighPass -> **composite** -> halo apply `FUN_00923300` -> exposure update `FUN_0093e660` ->
(MotionBlur `FUN_0096a420`) -> lens flare `FUN_00966a40` (call order proven in the probe). The sun disc and glow draw earlier, in pass `SUN` (0x26), between `STARS`
(0x25) and `MOON` (0x27) by id (ascending order proven, JC2-203: `sky-gradient-next.md` section 12.2), on the scene colour before the post composite, so the composite, exposure and the `sqrt` encode act on
them and the halo and flare do not get exposure-scaled.

## 6. For the Rust side (what the findings allow)

* Disc and glow: two unlit quads in the sun orbit frame (rows `x`, `toSun`, `z`), camera-relative, depth forced to the far plane, additive `SRC_ALPHA, ONE`
  on the scene colour with the tint of section 2.4, CLAMP bilinear sampling, textures read as sRGB colour (inferred) with the alpha as the shape, vertex colour
  `(0.984375, 0.999756, 0.999996, 1)`. The glow tint (0.4 -> reddening) and the disc tint (2.0) do not depend on the sun colour table.
* Halo and flare: screen-space additions after the composite using the sun colour `cb0[3]`; do not add them to the scene-referred colour.
* Not yet: the mask's scene-alpha input (Gap 1) and the output target (Gap 2) decide the exact visible result.

## 7. Proven / inferred

**Proven** (probe): the byte pins; both vertex lists; the pass setup calls, the block flags and flag-word bits; the `Open` ctor defaults, vtable slots and state replay; the shader
arithmetic of `open`, `lensflare`, `halomask`, `alphamask`, `blur`, `sunbeam`; every float constant named above; the call order of the PostEffects task; the cell, position and
size tables; the night-fade exponent; the `SUNBEAM` / pass 0x47 absence; the four texture fingerprints and the alpha facts; the models' fixtures.

**Inferred:** (a) texture colour spaces (`_dif` rule, `FUN_00946080`); (b) `FUN_004cf830` is the in-place matrix inverse; (c) the lens-flare texture binding order (strip at slot 0 by
`vtable+4`, mask at slot 1 by `FUN_0094da50`); (d) `FUN_009390e0` is "camera below water level"; (e) pass 0x26 runs between 0x25 and 0x27; (f) the render target and depth view of
the halo / flare draws (Gap 2); (g) S(0x13) stays 1.0 (no writer other than the cutscene tracks seen, JC2-181).

## 8. Gaps

1. **Scene alpha at the sun pixel and depth-copy semantics (AlphaMask inputs).** `FUN_00954f20` masks the halo with `sat(1 - 2 a_scene)` unless any depth was written; which passes write the alpha channel of
   `back_buffer_copy` (clouds? terrain?) and its clear value are not traced. Decides whether clouds and haze dim the halo and the flare. Also: the lens flare reads the mask even when the prepass skipped
   the frame (`V.y > 0.07`, 18:20 to 19:00), i.e. a stale mask. Filed as JC2-212.
2. **Output target and draw context of the halo and the lens flare.** The target `P[0x59]` (format, sRGB view, depth view bound by `FUN_0095f8a0`), the write mask / depth state not set by the draws
   the same target). Filed as JC2-213.
3. ~~Scene target format and clear values and the execution order of pass 0x26~~ **resolved by JC2-203 (`sky-gradient-next.md` section 12)**: the scene is an sRGB-viewed `R8G8B8A8` target (blending in linear space, saturated and re-quantised to 8 bits after every pass), cleared to linear (0.498, 0.498, 1.0, 1.0) with depth 0.0, and pass 0x26 runs after the opaque passes and between STARS (0x25) and MOON (0x27) in ascending id order; the disc (2.0 tint) and the glow therefore clamp at 1.0 per channel per pass.
4. Sign of `V` (`W+0x260`) and the lightning override: covered by JC2-204; the halo gate, the halo and flare sun position and the dome all rest on it.
5. Minor, listed only: writers of `S(0x13)` other than the constructor and the cutscene tracks; writers of `lens+0x1c`; the exact view / projection conventions of `FUN_0091c810`; the writers of
   `W[1]` (sun pass enable) during cutscenes; the runtime colour space of the four textures; the camera field of view (pixel sizes of the quads).
