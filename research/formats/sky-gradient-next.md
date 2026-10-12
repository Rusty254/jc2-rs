# The sky dome (SkyGradient), the FogGradient colour and the stars (JC2-151)

Status: research brief. No runtime code changed. Probes: `research/tools/sky_gradient_probe.py` (89 checks, exits 0; needs a private
`jc2tool` for the data checks, they report SKIP without one) and, for the render state of section 10 (JC2-173), `research/tools/sky_render_state_probe.py` (89 checks, exe only) and, for the clear, target and pass order of section 12 (JC2-203), `research/tools/atmosphere_pass_probe.py` (64 checks, exe only). Addresses are runtime addresses of `JustCause2.exe` (no ASLR). Every
function the claims rest on is pinned by a sha256 of its bytes, the call sites and constants are byte-checked, and the shader claims
are re-disassembled from `DX10_Shaders_F.shader_bundle` by the probe (`research/tools/sm4dis.py`).

## Headline

1. **The sky dome is a 32-triangle cone, and its horizontal texture coordinate is the azimuth to the sun.** The block
   (`FUN_00a4a220`, drawn by `FUN_00a44d40`) is a cone of radius 100 and height 100 (ring at y = 0, apex (0, 100, 0)), centred on the
   camera, rotated about Y so that its local +z points **at the sun** (away from the unflipped light vector `V = W+0x260`, which is
   -toSun; sign inferred, see Inferred). The mesh `u` runs 0 at that side to 1/32 (one texel) on the opposite side, `v` is 1 at the
   ring and -0.5 at the apex. So the texture's **even columns are the sun-facing colours and the odd columns are the opposite side**;
   the odd columns are real data, not noise. This is the "alternating" pattern that `weather.md` smoothed away.
2. **Hour mapping.** `t = hour / 24 * 16`, `k = floor t`, blend `f = frac t`. The vertex shader adds `A = (k + 0.25)/16` and
   `B = (k + 1.25)/16` to `u` (these are texel centres 2k and 2k+2 of the 32-wide texture, so **one even column per 1.5 hours**); the pixel
   shader samples both and returns `tex(u+A)*(1-f) + tex(u+B)*f`. Columns wrap (hour 22.5 .. 24 blends column 30 to column 0).
3. **Elevation mapping.** The pixel shader clamps `v` to 0.01 .. 0.99 and samples the 128-row texture: the bottom row is the horizon. `v` is
   linear on the cone: `v = 1 - 1.5 y/100`; along a ring-vertex edge the elevation is `atan(y / (100 - y))`, so the zenith row is reached
   from about 62.7 degrees up and the horizon row from about 0.4 degrees.
4. **Colour space and units.** The texture name ends in `_dif` and the texture-name callback `FUN_00946080` returns "sRGB" for that suffix; the
   loader `FUN_009619d0` then adds one to the DXGI format (R8G8B8A8_UNORM 28 -> R8G8B8A8_UNORM_SRGB 29; the loader swizzles the A8R8G8B8 file
   into RGBA order, which is the 28/29 pair). So the sampled colour is the **sRGB-decoded texel, filtered in linear space**, and the pixel
   shader writes it **unscaled** to the scene target (alpha 0). Native scene unit 1.0 is display white (`night-exposure-next.md`), so the dome
   colour in native units is `srgb_to_linear(texel)`; the display shows `sqrt(c * A * tint)`. There is **no sun-elevation dimming, no
   zone dependency and no cb0 input** in the pass: night is dark because the night columns are dark, and the project's `NOON_SKY` /
   `NIGHT_SKY` policy, the "relative to noon" scaling and `smoothstep` dimming in `sky.rs` have no native counterpart.
5. **FogGradient colour writer found (JC2-116 Gap 1).** `FUN_00a45110` (called each frame from the weather update `FUN_00a472f0` on the object
   `O = W + 0x3b8`) writes the FogGradient block's colour `cb1[5]` = (fog colour rgb, 1.0), where the rgb is the atmospheric fog layer colour
   `cloud + 0x54` (= haze `H + 0x10`, the colour `FUN_00a432d0` also uploads as `cb0[13]`) **when the camera y is above 200** and the water layer
   colour `cloud + 0x78` (`H + 0x34`) when it is at or below 200. It also writes the ring scale `cb1[4] = (30, (0.1 + 10 P) * 20, 30, 1)` with `P = [cloud+0x204] + 0x48`
   (the precipitation amount): 2 in clear weather, 202 in full precipitation. Both are double buffered by the frame parity.
6. **Stars.** 2000 stars, generated once by `FUN_00a47750` with `srand(1)` and the MSVC `rand()`, drawn as screen-space quads; their alpha is
   `min(1, byte / 128)` with `byte = clamp(300 (V.y - 0.1), 0, 200)` (`V.y` > 0.1 means the sun is below the horizon).

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `DX10_Shaders_F.shader_bundle` (7,582,748 B) | `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98` |
| `textures\atmosphere\skygradient_dif.dds` (16,512 B, 32 x 128 A8R8G8B8, no mips) | `c33775e543fac3198e1899f8234b6fef217d64b267a9c4b5edbcf35f948520da` |
| `textures\atmosphere\fog_gradient.dds` (640 B) | `e5a877728b77191c13e975112476555b03c15731ae833a3d541e526c5c7a0809` |

Function pins (sha256 of the bytes, in the probe): `FUN_00a45110` (0x149), `FUN_00a44d40` (0x28e), `FUN_00a4a220` (0x4a2), `FUN_00a4a6d0` (0x1d0),
`FUN_00a45330` (0x16e), `FUN_00a47750` (0x357), `FUN_00a45260` (0x90), `FUN_00a472f0`, `FUN_00a44ad0`, `FUN_00946080`, `FUN_009619d0`,
`FUN_004e0f60`, `FUN_004dc8a0`, `FUN_00491e50`.

## 1. Object model

`FUN_00a4a6d0` constructs the sky/fog object `O` (at `W + 0x3b8`, `W` = the weather object): `O[1]` = the SkyGradient render block (type
`FUN_00a40e20`, `DAT_012ab468`), `O[2]` = its data (`FUN_00a4a220`, 0x2c bytes), `O[3]` = the FogGradient render block, `O[4]` = its data
(`FUN_00a49ec0`, 0x5c bytes, type `FUN_00a40c70`, `DAT_012ab460`), `O[0]` = 0.1 initially. Per frame `FUN_00a472f0` calls `FUN_00a45110` with `ecx = O`
(byte-checked: `lea edi,[W+0x3b8]` then `mov ecx,edi; call`), which

* sets the two blocks' transforms (virtual +0x40 with `DAT_0108cd40`),
* writes the FogGradient data (section 4),
* writes the **hour** into the SkyGradient data at +0x14 as `[[0x01191ab4] + 0xac] / 24` (`[0x00f0f2ec]` = 0.0416667), and the three floats of `W+0x260`
  (the unflipped light vector `V`, `S+0x250` in `haze-fog-next.md`) into the data at +0x18, +0x1c, +0x20.

## 2. The dome (`FUN_00a4a220` mesh, `FUN_00a44d40` draw, shaders)

**Mesh (constructor, byte-checked literals and loop).** 32 iterations, three vertices each (96 vertices, stride 0x14 = position xyz + uv,

| vertex | position | uv |
|---|---|---|
| ring i | (100 sin a_i, 0, 100 cos a_i), `a_i = 2 pi i / 32` (the game's polynomial sine, error below 0.001) | `u = |a_i folded into (-pi, pi]| / pi / 32`, `v = 1` |
| apex | (0, 100, 0) | `u = 1/64` (`0x3c800000`), `v = -0.5` |
| ring i+1 | the next ring point | its own `u`, `v = 1` |

`u` is `|atan2(sin, cos)| / pi / 32` (byte-checked `cvtps2pd` operand order: `y` = the x position, `x` = the z position), so it is 0 at local +z and
1/32 at local -z. The apex `u` is half a texel. The vertex layout has no `w`, so `w = 1`.

`FUN_004e0f60` with flag 0: row 2 = normalize(eye - target) = `-V_xz`, row 0 = normalize(up x row 2) = (z.z, 0, -z.x), row 1 = row 2 x row 0
(the same frame as `weapon-ownership-next.md`). It is multiplied into the view-projection with the camera position as the translation row
(`FUN_00491e50` is the row-vector product A * B, byte-pinned), and the VS uses `v * M` (rows of `cb1[0..3]`), so **local +z maps to world `-V_xz`**
and local +x to `(z.z, 0, -z.x)`. The pixel constant `cb1[0] = (f, 0, 0, 0)` and the vertex constant `cb1[4] = (A, B, 0, 0)` are uploaded (slot 1 registers 0 and 4;
`FUN_00958860` is the pixel-constant upload, `FUN_0096beb0` the vertex one). Topology 4 (list), 96/3 = 32 primitives. `|V_xz|^2 < 1e-6`
(sun overhead) leaves the vector unnormalised.

**Vertex shader (`skygradientshader`, disassembled).** `clip.xyw = v.x cb1[0].xyw + v.y cb1[1].xyw + v.z cb1[2].xyw + w cb1[3].xyw`, `clip.z = 0`
(the far plane of the reverse-Z projection), `o1.xz = u + cb1[4].xy`, `o1.yw = v`. **No cb0.**

**Pixel shader.** `r0.x = 1 - cb1[0].x`; `vv = min(max(v, 0.01), 0.99)`; `out.rgb = tex(uA, vv) (1 - f) + tex(uB, vv) f`, `out.a = 0`, one 2D
texture and sampler `s0`, no other input.

**Model (probe functions `hour_inputs`, `dome_uv`, `sky_colour`).** For a view ray with azimuth `az` measured from the sun-facing side and elevation `el`
the ray meets the facet `(ring i, apex, ring i+1)` of the cone at `P = s d = l0 r_i + l1 apex + l2 r_{i+1}`; `u = l0 u_i + l1/64 + l2 u_{i+1}`,
`v = l0 - l1/2 + l2`. Colour = bilinear (wrap in `u`, texel centres at `(n + 0.5)/32`) of the **linear** texels. Fixtures (linear, 1e-5):

| hour | azimuth from sun | elevation | native colour |
|---|---|---|---|
| 06:00 | 0 | 0.01 | (0.51965, 0.09762, 0.01374) (the orange texel (190,88,31)) |
| 06:00 | 180 | 0.01 | (0.42990, 0.49229, 0.49228) |
| 12:00 | 0 | 0.01 | (0.59864, 0.79300, 0.86316) |
| 12:00 | 180 | 0.01 | (0.42991, 0.66540, 0.86316) |
| 12:00 | 0 | 89 | (0.00368, 0.01764, 0.09990) (texel (12,36,89)) |
| 18:00 | 0 | 0.01 | (0.70836, 0.16274, 0.01942) |
| 00:00 | 0 | 10 | (0.04625, 0.07421, 0.09468) |

**Evidence for the sun-facing side.** In the retail texture the even columns carry the orange glow at 06:00 (column 8: (190,88,31)) and 18:00
(column 24: (219,113,38)); the odd neighbours are pale ((176,187,187), (196,194,182)). The construction above puts `u = 0` (the even column)
on world `-V_xz`; with `V = -toSun` (JC2-116) that is the sun, which is exactly where a sunrise glow belongs. The two derivations agree; the dome
sign still carries no run-time capture.

## 3. Colour space (loader chain)

* `FUN_00a4a220` loads `Atmosphere\skygradient_dif.dds` through the texture manager (`DAT_01190b50`, slot +0x28). The file is A8R8G8B8 (masks
  0xff0000/0xff00/0xff/0xff000000, 32 bpp, flags 0x41), 32 x 128, **no mip chain** (so the texture-quality mip skip in the loader never applies).
* `FUN_009619d0` rewrites 32-bit files with blue mask 0xff into RGBA order (the swizzle loop, pseudo-C read), then asks the manager's callback
  (`vtbl+0x58` = `FUN_00946080`). Byte-checked: names starting `gui_`, `hud_`, `fe_`, `pda_` return 0; a name ending `_dif` returns 1 (`mov al,1`); other
  names return 1 only on a hit in the `gammacorrect.bin` hash set. Result 1 adds one to the format, giving the sRGB view.
* The shader resource view is built from that format (`FUN_00950420` / `FUN_0091c630`, `material-parameters-next.md`).

So `skygradient_dif` is sampled **sRGB-decoded** (inferred only in the D3DX10 report of the A8R8G8B8 format: not executed; the swizzle to RGBA order is the
evidence that the format is R8G8B8A8, whose "+1" is the sRGB pair). Contrast `general-terms-next.md`, where the cube loader never adds one.

## 4. The FogGradient colour and scale writer (JC2-116 Gap 1)

Byte-checked in `FUN_00a45110` (pinned):

* `P = [[W+0x4cc] + 0x204]`; `base = P[0x48] * 10.0 + 0.1`; stored to `O[0]`; `cb1[4]` of the FogGradient data (offset +0x3c + 0x10 p, `p` = the frame parity
  `[0x0108c594] & 1`) = `(30.0, base * 20.0, 30.0, 1.0)`. The ring (`haze-fog-next.md` section 6, vertex y -10 .. +1) is therefore 60 m radius and
  between y = -20 .. +2 m at clear weather; the y scale grows to 22 with `P48 = 1`.
* `sel = (camera.y <= 200.0)` with `camera = [[0x011919f4] + 0x10]`, `camera.y = [camera + 0x58]`. The colour read is `[cloud + 0x54 + 0x24 sel]` (three floats): `sel = 0`
  gives `H + 0x10` (the atmospheric layer-0 fog colour of `FUN_00a432d0`, the `cb0[13].rgb`), `sel = 1` gives `H + 0x34` (the water layer). The result is stored as the
  vec4 `(r, g, b, 1.0)` at the data's +0x1c (+0x10 p), i.e. exactly the `cb1[5]` bound by `FUN_00a44ad0`.

Consequence: the horizon band `foggradientshader` paints is **the same colour as the distance fog** (the native fog colour already implemented as `haze.rs`), with the
texture alpha as its opacity, and turns water-coloured below sea level. The read happens before `FUN_00a654b0` in the weather update, so it is the previous frame's fog colour
(one frame of lag, not exercised).

## 5. Stars (`CStarsRenderBlock`)

* **Generation (`FUN_00a47750`).** `srand(1)`; 2000 times: three `rand()` -> `r = rand * 6.103702e-05 - 1` (this is `2 / 32767`; range -1 .. 1) give a direction (normalised unless the squared length
  is below 1e-6), two more give `dist = r5 * r4 * 200 + 300` (100 .. 500), two more give `size = |r7 * r6| * 18 + 2` (2 .. 20); the vertex is `(dir * dist, size)`. The MSVC `rand`
  is the standard LCG (`state * 214013 + 2531011`, `(state >> 16) & 0x7fff`; both constants occur in the exe): the probe reproduces stars 0 and 1,
  (-271.84, 34.66, -167.16, size 2.217) and (144.08, 117.47, 89.73, 2.205).
  the camera position. So the sky sphere turns with the moon's frame.
* **Colour.** `starsshader`: the VS builds a camera-facing quad per star from `SV_VertexID` (corner x = id & 1, y = `(id + 1) mod 6 < 3`), size `size * viewz * cb1[4].xy`, with
  `cb1[4] = ((height / width) / 720, 1 / 720)` (`FUN_00a45330`: `0.0013888889` = 1/720 times the screen aspect), depth forced to 0; the PS returns `star_dif.dds` rgb (a `_dif` name, so sRGB decoded) with
  `alpha = cb1[0].w`. `cb1[0] = (1, 1, 1, min(byte * 0.0078125, 1))`. A star sprite is `size / 2` pixels (720-line units) in radius times the screen height / 720.
* **Visibility (`FUN_00a472f0`).** `byte = int(clamp((W[0x264] - 0.1) * 300, 0, 200))` stored at the stars data +0x14; `W[0x264]` is the y of `V` (-toSun.y). So alpha 0 until
  the sun is 5.7 degrees below the horizon (`V.y = 0.1`), reaching 1 at `V.y = 0.1 + 128 / 300` (sun 31.8 degrees below the horizon).

## 6. What is not in this pass (and where it is)

* The sun disc and glow are not in the dome pass: they are the two `Open` quads of pass `SUN` (0x26, `FUN_00a6ea10`; `Atmosphere\sun_dif.dds` 64 x 64 DXT5 and `sun_gradient_dif.dds` 128 x 128 A8R8G8B8, quads at heights 1500 / 500). **Decoded by JC2-174: `sun-glow-next.md` sections 2 and 6.**
* `sunbeam` (the shader pair with PS `texture * 0.25`) is dead code (no creator for its pass, no name reference); `sunbeam_dif.dds` is the shape of the post-effect sun halo (`FUN_00954f20` / `FUN_00923300`), and `lensflare_dif.dds` (512 x 128 DXT1) the seven flare sprites of `FUN_00966a40`. **`sun-glow-next.md` sections 1, 3 and 4.**

## 7. Proven / Inferred

**Proven** (probe: bytes, hashes, decoded constructor, data and shaders; pseudo-C read where marked)

* Vertex and pixel shaders of the dome, stars, FogGradient (and the sunbeam pixel shader), re-disassembled; no `cb0`.
* The dome mesh literals and loop (bytes), `u` = `|theta|/pi/32`, apex `u` = 1/64, `v` 1 / -0.5; hour/16-column mapping and `A`, `B`, `f` formulas (bytes of `FUN_00a44d40`).
* The per-frame writer of the FogGradient colour and scale and of the sky data (`FUN_00a45110`, bytes); the identification of `O[1..4]`.
* The texture name callback's three outcomes (bytes), the loader's RGBA swizzle and "+1 = sRGB" (pseudo-C), the retail texture (fingerprint, no mips, columns).
* The star generation constants and order; the alpha formula; the MSVC rand constants occur in the exe.

**Inferred**

* That `V = W+0x260` points away from the sun (JC2-116's `S+0x250 = -toSun`); the dome's sun-facing side then follows from the frame, and the orange even columns agree.
* The D3DX10 format report for A8R8G8B8 (28), hence the sRGB view; the sampler `s0` of this block was assumed to be the slot-0 default; JC2-173 proved it: U wrap, V clamp, bilinear (section 10.2).
* That `+0x14` and the matrix upload in `FUN_00a44d40` are read in the same frame as `FUN_00a45110` wrote them (no extra delay between update and draw).
* ~~A star sprite size in pixels (`FUN_00942390` order)~~ closed by JC2-173: (width, height) is proven (section 10.3).

## 8. Gaps (native details still unknown)

   `FUN_009203e0` in the stars draw `FUN_00a45330`, and the FogGradient type): the packed fields are blend / depth / cull selectors not mapped to D3D10 enums. The dome writes alpha 0 and depth 0, so
   the dome is almost certainly drawn opaque with depth test "always / not greater", and the fog band and stars need blend factors for their alpha (`foggradient` uses the texture alpha, the stars `cb1[0].w`);
   none of that is proven. Blocks: exact blending of the horizon band and the stars.
2. ~~**Stars draw call:**~~ **resolved by JC2-173 (section 10.3).** Original text: **Stars draw call:** the vertex-buffer expansion (the VS reads `SV_VertexID` and a per-vertex `float4` while the CPU array has 2000 entries, `FUN_00a48df0` area) and the viewport width/height order. Blocks: pixel-exact star sizes.
3. ~~**Sun disc and glow, `sunbeam`, lens flare:**~~ **resolved by JC2-174 (`sun-glow-next.md`).** Original text: `CSun` at `0x00a60000` (sun_dif, sun_gradient_dif quads, their colours and hour inputs), `sunbeam` (`cb0[4]`, `cb1`), the lens-flare block (`FUN_00950420` neighbour). Blocks: sun glow levels.
4. **Run-time capture** of the dome at 06:00 / 12:00 / 18:00 (sun side orientation and the `V` sign), and the lightning override of `V` (`haze-fog-next.md` Gap 4: during a flash `V` points at the flash and the dome's bright side would follow).
5. **Texture-quality skip and the sRGB format** are inferred (Gap in section 7); the dome has no mips so only the format matters.

## 9. Bounded Rust handoff (not implemented)

Phase 1 (data, no rendering change): `jc2_formats::weather::Atmosphere::sky_colour` becomes the native lookup: keep the 32 x 128 table as sRGB bytes; add
`sky_dome_colour(hour, az_from_sun, elevation) -> Vec3` = the probe's `sky_colour` (`hour_inputs`, `dome_uv`, bilinear in linear space). Test: the fixtures above (1e-5) and the
orange texel at 06:00 / 18:00. `sky.rs`: the clear colour is no longer a policy colour (`NOON_SKY`, `NIGHT_SKY`, `smoothstep` dimming, `display_native`); `native_units.rs`: drop the
"clear colour authored as display value" row for the sky.

Phase 2 (rendering): the **clear colour** of a Bevy camera is one colour, so the dome needs its own mesh: a cone entity (96 vertices as the table in section 2) parented to the camera (translation only),
rotated about Y so local +z faces the sun's azimuth, with a custom unlit material whose fragment shader does the two texture taps with the `A`, `B`, `f` uniforms and the `v` clamp (a 32 x 128 `Rgba8UnormSrgb` image, wrap in `u`, linear filter); drawn first, without depth write; **the clear colour is NOT black: section 12 (JC2-203) proves the scene is cleared to linear (0.498, 0.498, 1.0, 1.0) and depth 0.0 before the dome, and the additive dome sums onto it.** The horizon band is a second ring mesh
(geometry in `haze-fog-next.md` section 6, colour = `HazeFog.colour` above y = 200 and the water colour at or below it, scale from section 4 here) using the state of section 10.2 (alpha blend, depth test on, no write). Stars (2000 quads from `make_stars`) use the moon's orbit matrix and the alpha above, additive SRC_ALPHA / ONE blending (sections 10.2, 10.3); Gaps 1 and 2 are closed, Gap 6 (clear colour, pass order) decides the dome's additive base.

## 10. Render state of the dome, the fog band and the stars (JC2-173)

Probe: `research/tools/sky_render_state_probe.py` (89 checks, exits 0, exe only). It pins 33 more functions by sha256, re-reads the call-site immediates, the vtable
slots, the CRenderPass defaults and the lookup tables from the exe, then replays the setters on a model of the device-state words and decodes the result into the D3D10
descriptors. The task text of JC2-173 contained three mistaken premises, corrected here: (1) `FUN_0095e900`, `FUN_009203e0` and `FUN_00970020` are **sampler** setters (address
modes, filter, LOD and anisotropy), not blend, depth or cull selectors; (2) the SkyGradient state function is `FUN_00a40eb0` (slot +0x30 of the type), `FUN_00a44d00` is a block
function (slot +0x6c) that only repeats the sampler address; (3) `FUN_00a48df0` is the loading-screen builder, it has nothing to do with the stars.

### 10.1 How a block is drawn (all proven)

`FUN_00950ee0` (`CRenderPass::Draw`, pinned) draws a pass in this order: (a) the write mask = pass parameter 3 (default 7: RGB, no alpha) goes into bits 28-31 of the blend word;
(b) per block type: `FUN_0096f820` (reset: rasterizer fill SOLID + cull BACK, depth func GREATER_EQUAL (7), blend factors SRC_ALPHA / ONE / INV_SRC_ALPHA / ZERO, alpha-to-coverage off,
sampler slots 0..10 back to WRAP / WRAP / WRAP with no comparison, shader resources cleared), then `FUN_0095dc20` with ECX = the pass (byte-checked `mov ecx,[ebp-0x10]`), which
copies the pass parameters into the state words: parameter 1 = DepthEnable, 0 = DepthWriteMask, 2 = DepthFunc, 4 = BlendEnable, 5 = a pixel-constant flag, 0x11 = multisample; the sample
mask becomes 0xffff; then the type's slot +0x2c (constant-buffer bindings) and slot +0x30 (state, the functions below); (c) per block, slot +0x6c when the sort key changes and slot +0x74
(the draw). The pass parameters are set with `FUN_0095fb10` (index, value; stored at pass + 0xc84 + 4 x index); `FUN_0095ce90` (the CRenderPass constructor) defaults parameters 0..5 to 1, 1, 7, 7, 0, 0
and parameter 0x11 to 1. Device state defaults (init near 0x938900): depth word 0xb (enable, write, LESS), blend word 0xf2421108 (disabled, ONE / ZERO, ADD, mask 0xf), rasterizer word 0xf.

The passes are created by `FUN_00a4a6d0` (sky pass id 0x28, fog pass id 0x29) and `FUN_00a47750` (stars pass id 0x25); the name table `FUN_00958fc0` maps 0x25..0x2a to STARS, SUN, MOON,
SKY_GRADIENT, FOG_GRADIENT, UNDERWATER_CLOUDS (jump-table bytes at 0x9594dc, checked). Parameters set by those constructors: sky and fog `(0,0) (1,1) (5,0) (4,1) (0x11,0)`; stars `(0,0) (1,1)` only.

### 10.2 Final state per block (probe model, D3D10 enums)

| | SkyGradient dome (pass 0x28) | FogGradient band (pass 0x29) | Stars (pass 0x25) |
|---|---|---|---|
| Depth | test on, write off, GREATER_EQUAL, no stencil | same | same |
| Blend colour | src ONE, dest ONE, ADD: **additive** | src SRC_ALPHA, dest INV_SRC_ALPHA, ADD | src SRC_ALPHA, dest ONE, ADD: additive scaled by the star alpha |
| Blend alpha | src ONE, dest ZERO | src ZERO, dest ONE (target alpha kept) | src SRC_ALPHA, dest ONE (not written, mask excludes alpha) |
| Write mask | RGB (7) | RGB (7) | RGB (7) |
| Sampler 0 address (U, V, W) | WRAP, CLAMP, CLAMP | CLAMP, CLAMP, WRAP | CLAMP, CLAMP, CLAMP |
| Sampler 0 filter | 0x14 MIN_MAG_LINEAR_MIP_POINT, MaxLOD 0 | same | 0x15 MIN_MAG_MIP_LINEAR, MaxLOD FLT_MAX |
| Input layout | POSITION float3 + TEXCOORD float2 (slot 0, per vertex) | same pair | POSITION float4 (slot 0, per vertex, stride 16) |
| Draw | topology 4 (triangle list), 32 primitives (`FUN_00a44d40`) | topology 5 (strip), vertices - 2 primitives (`FUN_00a44ad0`) | topology 4, 4000 primitives, 12000 vertices |

Reading of the filter field (the table at 0xf22858 maps the 4-bit fields to a min or mag linear bit and a mip linear bit; 7 selects anisotropic 0x55 / 0x15): field A (bits 13-16 of the sampler word) gives the
min and mip bits, field B (bits 17-20) the mag bit; A < 3 means MaxLOD 0, A >= 3 means FLT_MAX. The comparison function field is 0 in all three, so the descriptor uses NEVER and no comparison filter. The
descriptor builders are `FUN_00940170` (device vtable +0x158, CreateSamplerState), `FUN_00933530` (+0x14c, blend), `FUN_009603c0` (+0x150, depth-stencil) and `FUN_009338a0` (+0x154, rasterizer).

Consequences for the Rust side. The sky is **additive** onto whatever the target holds (the clear colour; **resolved by JC2-203, section 12: the target is cleared to linear (0.498, 0.498, 1.0, 1.0), not black**), so "dome colour = clear colour" in Bevy is wrong; the native pixel is `saturate(clear + dome)`.
The depth test GREATER_EQUAL with the dome's clip z = 0 (reverse-Z far plane) passes only where nothing has written depth: the dome shows behind terrain and objects, never in front.
The fog band is a plain alpha blend over the already drawn sky, test GREATER_EQUAL, no depth write, no culling. The stars add their colour times `cb1[0].w` (`min(byte / 128, 1)`). Pass 0x25 sorts before 0x28 and 0x29
(ascending id; the pass map is a tree iterated in key order by `FUN_0094fe10`; **proven by JC2-203, section 12**), which does not matter for additive blends below saturation.
The sampler addressing settles an earlier inference: the dome texture is wrapped in u and clamped in v, bilinear, no mips.

### 10.3 The stars draw (Gap 2 resolved)

* **The expansion is on the CPU.** `FUN_00a41570` (stars block slot +0x80, byte-checked) expands the 2000 generated float4 `(x, y, z, size)` into **six identical copies each** (12000 vertices, stride 0x10)
* **Gate.** `FUN_00a454a0` (block slot +0x74) draws only if the byte at stars-type + 0x14 (the alpha byte from section 5) is non-zero; it uploads the block matrix times the view-projection to cb1 register 0 and binds the star texture.
* **Viewport order (inferred item closed).** `FUN_00942390` copies the 40-byte device mode record written at start-up (`FUN_00930000`, near 0x93088d): dword 0 is the back-buffer width, dword 1 the height (the record also stores `width / height` as a float).
  The stars constant is therefore `(height / width) / 720` in x and `1 / 720` in y, exactly as section 5 states.
* The constant-buffer bindings (slot +0x2c): sky and stars bind VS cb1 with 5 registers and PS cb1 with 1; the fog band VS cb1 with 6 registers and no PS cb1. These agree with the shader inputs (matrix + `(A, B)`; matrix + size scale; matrix + scale + colour).

## 11. Remaining native gaps after JC2-173

6. ~~**Clear and ordering of the atmosphere passes.**~~ **resolved by JC2-203 (section 12; remaining questions listed in 12.6).** Original text: The scene colour and depth clear values (pass FIRST / CLEAR), the render target set bound while passes 0x25..0x29 run, and the confirmation that the pass map runs in ascending id order are not traced. They decide whether the additive dome result equals `dome colour` or `dome colour + clear colour`, and whether reverse-Z depth is cleared to 0 (the dome and stars rely on GREATER_EQUAL against a cleared 0).
7. **Sign of the light vector V (`W + 0x260`) and the lightning override** (Gap 4 of section 8): still no run-time capture, so the sun-facing side of the dome rests on the texture evidence.
8. Minor, not blocking: border colour (unused with WRAP / CLAMP), the unused 6th argument of `FUN_009203e0` (8 everywhere), the dead 5th argument of `FUN_00970020` (12.0 in the stars draw), the meaning of pass parameter 5 (`DAT_013d9e2c`, 0 for all three passes).

## 12. Clear values, render target and pass order of the atmosphere passes (JC2-203, Gap 6 resolved)

Probe: `research/tools/atmosphere_pass_probe.py` (64 checks, exits 0, exe only). It pins 26 more functions by sha256, byte-checks the call sites and constants, and replays the clear logic
on small models (`clear_bytes`, `predraw_clear_mask`, `pass_order`). Everything below is read from the exe unless marked inferred.

### 12.1 The frame, in order (`FUN_0096d400`, the render-thread frame task)

1. camera and cb0 set-up (`FUN_009521e0`, `FUN_0095f020`, `FUN_00946c90`; `underwater-render-next.md` section 0);
2. engine vtbl +0x28 (`FUN_0094d770`): a sweep of a plain vector of passes (the shadow and reflection passes are created with flag 0 and are not in the pass map; inferred from their creators);
4. engine vtbl +0x2c (`FUN_0094fe10`): **the pass-map sweep** (12.2). Both sweeps are guarded by the byte at render-thread object + 0xac;
5. scene to back-buffer copy, post effects, back buffer (`underwater-render-next.md`).

### 12.2 Pass order (proven: ascending signed id)

The passes live in a `std::map<int, vector<pass*>>` at engine + 0x48. `CreatePass(id, flag)` (engine vtbl +0x5c, `FUN_00922380`) allocates a 0x1a28-byte `CRenderPass` (`FUN_0095ce90`) and, when flag is 1, calls `AddPass`
(vtbl +0x88, `FUN_009215a0`): `GetId` (pass vtbl +0x7c), `FUN_00932d00` (find-or-insert the vector for that id; the descent compares the node key with `cmp ...; jge`, a **signed** `<`), `FUN_00920500` (`push_back`).
`FUN_0094fe10` walks `begin()` (header->left) to `end()` with the standard in-order successor `FUN_008ac3c0` and, for each pass in the vector, calls vtbl +0x7c (stores the id at engine + 0xa8), +0x18 (context) and **+0x3c (Execute, `FUN_009737a0`)**.
So passes run by ascending signed id, and within an id in creation order. The id -> name table (`FUN_00958fc0`, index = id + 0x100): -256 FIRST, -100 TERRAINMATERIAL, -99 REFLECTION, -98..-96 SHADOW_NEAR/MIDDLE/FAR, -95 MODELS_SHADOW,
-94 VEGETATION_SHADOW, -93 TERRAIN_SHADOW, then 0 CLEAR, 1 Z_PASS, 2 Z_AND_VELOCITY_PASS, ... 0x24 LAST_OPAQUE, **0x25 STARS, 0x26 SUN, 0x27 MOON, 0x28 SKY_GRADIENT, 0x29 FOG_GRADIENT, 0x2a UNDERWATER_CLOUDS**, ... 0x30 WATER, ... 0x54 LAST.
No pass with a negative id was found created with flag 1 (the creators read pass flag 0), so CLEAR (0) is the first pass of the sweep (inferred, Gap 12.6-c).
The sky passes therefore run **after every opaque pass** (LAST_OPAQUE 0x24) and before the water (0x30): opaque terrain and models are already in the colour and depth targets when the stars, sun, moon, dome and band are drawn.

### 12.3 The target bound while passes 0x25..0x29 run

`FUN_00956860` builds the sets from the records made by `FUN_0092f450`. A set is `{[0] depth record, [1..4] colour records, [5] colour count, [6] write mask 0xf}`.

| set (`DAT_013d9c48 + off`) | content | used by |
|---|---|---|
| +0x20 | depth record (`param + 0x18`) + colour record A (`param + 0x10`) | **the scene: every pass between the frame's bind (12.1 step 3) and the post copy, in particular 0x25..0x29** |
| +0x24 | depth `param + 0xc` + swap-chain colour `param + 0x8` | back buffer with depth |
| +0x28 | no depth + swap-chain colour | post output |
| +0x2c, +0x30 | depth + colour records (`+0x1c` R32_FLOAT, `+0x14` UNORM view) when `[0x013d9c43] == 0`; depth only / depth + `+0x14` when it is 1 | Z_PASS (id 1) and Z_AND_VELOCITY_PASS (id 2) only |

* **Colour record A**: one `R8G8B8A8_TYPELESS` (0x1b) texture, bind flag 0x20, render-target view format **`R8G8B8A8_UNORM_SRGB` (0x1d)**; record B is the same texture viewed as 0x1c (UNORM). So the scene is **8-bit, gamma-encoded in storage, linear in blending**: every pass blends in linear space and the result is saturated to 0..1 and quantised after each pass.
* **Depth record**: `D24_UNORM_S8_UINT` (0x2d; 0x2c `R24G8_TYPELESS` with a shader-resource bind flag when `[0x013d9c43] != 0`), DSV dimension TEXTURE2D (3) or TEXTURE2DMS (5). Depth is 24-bit fixed-point, not float.
* `FUN_0095f8a0` = `OMSetRenderTargets` (device vtbl +0x60) with the set's colour views and depth view, then `RSSetViewports` (+0x78): top-left (0, 0), width and height = the uint16 fields at record + 0x14 / + 0x16, **depth range 0.0 .. 1.0**.
* `CRenderPass::Draw` (`FUN_00950ee0`) changes the set only for pass id 1 (binds +0x2c) and id 2 (binds +0x30), saving `[0x013d9c48] + 0x34` first and restoring it at the end; for every other id it only loads the write mask (pass parameter 3 = 7, RGB). The sky passes keep set +0x20.
* `[0x013d9c43]` is 1 when the back-buffer sample count (`FUN_0092f450` argument 4, the anti-aliasing setting) is 1, or when `DAT_013d9c78` is non-null; otherwise 0. The retail default value is **not** established here (Gap 12.6-b); it only changes the Z-pass sets and one extra clear (12.4).

### 12.4 The clear (pass CLEAR, id 0)

`FUN_00a52f30` (called once by `FUN_00a74f80` on the 0x2284-byte weather object `W`; the pass pointer is its last dword, `W + 0x2280`) does, in this order, `CreatePass(0, 1)`, then on the pass: enabled (+0x5c, 1), **clear colour on** (+0x44, 1),
**SetClearColour (+0x50) with the float4 (0.5, 0.5, 1.0, 1.0)** (loaded from `[0xe21a28]` = 0.5 and `[0xe20668]` = 1.0), **clear depth on** (+0x48, 1), **clear stencil on** (+0x4c, 1), **stencil value 0** (+0x58, 0). It never calls +0x54 (the depth value), so the constructor default holds.
No other code references `W + 0x2280` (every dword 0x2280 in .text is inside this function), so the values are constant (inferred beyond that scan, Gap 12.6-d).

* `FUN_00928740` converts each component with `x * 255.0` and a truncating `cvttss2si`: **bytes (127, 127, 255, 255)** at pass + 0x1a26, +0x1a25, +0x1a24, +0x1a27 (r, g, b, a).
* The constructor (`FUN_0095ce90`) defaults: flags word `& 0xff98 | 0x18` (no clears, enabled, sorted), **depth value 0.0** (`[pass + 0x1a20]`), **stencil value 0** (`[pass + 0xccc]`), colour bytes 0.
* Execute (`FUN_009737a0`) runs the pass because the flag bits 0..2 are set; `FUN_00966990` calls PreDraw (vtbl +0x8c, `FUN_00949f10`) and Draw. PreDraw, generic branch (the explicit-target bits +0x64 / +0x6c of the pass are 0): mask `0x3c` (flag bit 0), `|= 1` (bit 1), `|= 2` (bit 2) = **0x3f**,
* **Result on the scene target (set +0x20): colour = (127, 127, 255, 255) / 255 = (0.49804, 0.49804, 1.0, 1.0) given as a linear value to an sRGB view, so the stored bytes are sRGB (187, 187, 255, 255); depth = 0.0, stencil = 0.** Depth 0.0 with the engine default depth function GREATER_EQUAL (`FUN_0096f820`) is the reverse-Z convention the dome relies on (clip z = 0, section 2).
* Extra: PreDraw's tail, for the pass whose id is 0, when `[0x013d9c43] == 0` and the clear-depth flag is set, also clears colour view 0 of set +0x2c (the R32_FLOAT record) to (depth value, 0, 0, 0) = 0.0 (a depth/velocity copy target; only in the `[0x013d9c43] == 0` configuration).

### 12.5 What this does to the sky (consequences for JC2-214)

* The dome is `ONE, ONE` additive (section 10.2) on a target that holds the clear colour, so a covered pixel is `saturate(clear + dome)` per channel, in linear space, before the 8-bit sRGB store. The alpha channel stays 1.0 (write mask 7).
  Worked values with the section 2 fixtures (clear = (0.49804, 0.49804, 1)): noon zenith (0.00368, 0.01764, 0.0999) -> (0.50172, 0.51568, 1.0); noon horizon (0.59864, 0.793, 0.86316) -> (1, 1, 1); midnight, 10 degrees (0.04625, 0.07421, 0.09468) -> (0.54429, 0.57225, 1.0).
* The blue channel is always 1.0 where the sky shows, and no dome colour can darken the target; the dome texture alone is therefore not the displayed sky. **Plausibility flag, not resolved:** these sums read light and desaturated for the deep navy night and zenith colours the dome texture carries; the post pass (exposure, grade, `sqrt`) is applied afterwards,
  and no run-time capture of an empty-sky pixel exists. Implement the exe values 1:1 and verify with a capture (Gap 12.6-a), do not "correct" the clear.
* Order of the draws that touch the sky region: CLEAR (0) -> geometry -> STARS (additive) -> SUN -> MOON -> dome (additive) -> band (alpha over the result, section 10.2) -> UNDERWATER_CLOUDS. Because every pass saturates and re-quantises, the additive passes are order-sensitive only where the sum reaches 1.0, which here is the blue channel everywhere and the other channels near the horizon.
* The depth test GREATER_EQUAL against the cleared 0.0 admits the sky only where no opaque pass wrote depth; terrain and models written earlier hide it.
* For Bevy: clear the camera colour to linear (0.49804, 0.49804, 1.0) with alpha 1 and depth to 0.0 (reverse-Z), keep the sky meshes' blend states from section 10.2, and keep all of it in the pre-post (linear, 8-bit-clamped) target.

### 12.6 Gaps

a. **No run-time capture** of an empty-sky pixel (e.g. looking at the zenith at 00:00 and 12:00) to confirm that the (0.498, 0.498, 1.0) clear plus the additive dome is what reaches the post pass. Blocks: certainty about the sky brightness and tint.
b. **Retail default of `[0x013d9c43]`** (anti-aliasing setting 1 vs more, `DAT_013d9c78`) and therefore whether the R32_FLOAT record and the extra clear of 12.4 exist in the default configuration; the scene set +0x20 itself is the same either way. Blocks: nothing for the sky; Z-pass parity.
c. **Passes FIRST (-256), TERRAINMATERIAL (-100), REFLECTION (-99) and the shadow ids**: no creator with flag 1 was found, so they are not in the sweep; a creator not read would run before CLEAR on the scene target. Blocks: the claim "CLEAR is the first pass" (inferred).
d. Other writers of the CLEAR pass through a map lookup by id (no `GetPass` was found). Blocks: the claim "the clear values are constant" (inferred beyond the `W + 0x2280` scan).

## 13. Implemented by JC2-214 (runtime)

`crates/jc2-formats/src/sky_dome.rs` (dome mesh, hour inputs, linear bilinear colour, star generation and alpha; fixtures of section 2 and the first two stars are tests) and
`crates/jc2-game/src/native_sky.rs` with `sky_dome.wgsl`, `sky_fog_band.wgsl`, `sky_stars.wgsl`, `sky_sun.wgsl` (the sun quads of `sun-glow-next.md` section 2). State per section 10.2 (additive dome, alpha band, SRC_ALPHA / ONE stars and sun quads, depth GREATER_EQUAL no write,
clip z 0, RGB mask). `sky.rs` lost `NOON_SKY`, `NIGHT_SKY`, the relative-to-noon scaling and the dimming; the clear colour is the native linear (127, 127, 255, 255) / 255 (JC2-203 section 12.4, `sky::SCENE_CLEAR`); the passes sort STARS, SUN, dome, band after the opaque phase. `clouds.rs` lost its policy dome, star field and sun glow.

Assumed (labelled in code): `V = -toSun` (JC2-204); sun quad in-plane orientation and the star rest frame (rows of `FUN_00a53460` not decoded); fog band mesh (hand decode); Bevy transparent-phase sort bias standing in for pass order.
Observed (tile 30,14, 300 m, `_extract/jc2-214/clear_*.png`): with the native clear the day sky is a blue-lilac sky with cream clouds (the black-clear version was dark and grey), 18:00 is orange-lit and dark navy. **Midnight is too light and saturated** (violet-blue, about the 0.54, 0.57, 1.0 the 12.5 flag predicts, with dark clouds against it); the native values are kept, not corrected (Gap JC2-224).
