# Water clocks, optical constants and render-target ownership (JC2-095)

9 October 2026. Research only: no Cargo, GPU, game launch, runtime or physics edit. Extends
[water-next.md](water-next.md); the corrections to that brief are listed in section 9.
Executable `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`
(14,549,656 bytes); `DX10_Shaders_F.shader_bundle` sha256
`db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`. Addresses are retail VAs;
shader offsets index the bundle. Evidence is the ignored Ghidra export plus raw executable bytes
and the re-disassembled shaders; nothing proprietary is committed.

Reproduce (read-only, prints one JSON document with every fixture and hash):

```
python -B research/tools/water_consumers_probe.py --boundaries-only        # models only, no game
python -B research/tools/water_consumers_probe.py --game <game folder> \
       --tool <private copy of target/release/jc2tool.exe> [--pseudo <_unnamed export dir>]
```

Full mode asserts the exe/bundle hashes, the byte patterns quoted below, export fragments in 28
function bodies, the pass-name table, shader instruction strings and both DDS headers, and
prints the sha256 of every traced function's bytes (37 functions).

Legend. **Proven**: read from the binary/shader and pinned by the probe. **Inferred**: follows
from proven facts but one link is not machine-checked. **Unresolved**: not traced; no value is
substituted anywhere in this brief.

## 1. Fixture inputs

| Input | Identity |
|---|---|
| `textures\water\water_bump.dds` (decompressed via `jc2tool cat`) | 349,680 bytes, sha256 `f1ccd5cbbc6da23f51816f5b450024794c0a2a5d0859ed3faf7637a033d954f4`; DDS header: 512 x 512, DXT5, 10 mips |
| `textures\water\wave_foam_dif.dds` | 43,832 bytes, sha256 `9271b5f822a3d5c92c804732ad75f1868f055432b9e34ffaa2a08f9c11b16bb4`; 256 x 256, DXT1, 9 mips |
| WaterBumpComposite VS / PS | bundle offsets 336476 (572 B) / 7535708 (680 B); sha256 `98f1a749...cf331` / `719a7162...81a6` |
| WaterShader_lod0 PS | offset 7540448 (5384 B); sha256 `c9f4d833...ad8e` |

(The archive lengths in water-next.md are exactly the DDS file sizes; the dimensions are new.)

## 2. Clock to composite normal (proven)

Chain, in frame order. "native" means the instruction-level behaviour was read from bytes.

| Step | Producer / consumer | Units, defaults |
|---|---|---|
| 1 | `CClock` singleton `DAT_0118fd24` (`FUN_004cf520`, vtables `0xf08d68`/`0xe31848`) embeds an `IClock` at +4 (ctor `FUN_004e3a40`). Frame begin `FUN_0054e3e0` (called from `FUN_00745310`) runs `FUN_004ce070` then the tick `FUN_004e1d90` with `ecx = [0x118fd24] + 4` (bytes `8b 0d 24 fd 18 01 83 c1 04 e8` at 0x54e4a0) | |
| 2 | Tick: `elapsed = timeGetTime() - last` (u32, wrap corrected by +2^32), `last = now`. Sim delta field `IClock+0x14`: `e = min(elapsed, cap)`; `raw = e * 0.001 * timescale`; `dt = (raw - dt_prev) / (smoothing + 1) + dt_prev` | seconds. Defaults from ctor: dt = `0x3c888889` = 1/60, smoothing `IClock+0x24` = 10.0 (so EMA gain 1/11), cap `IClock+0x30` = 67 ms, timescale `IClock+0x20` = 1.0 (`DAT_0118fd24+0x24`, written by the slow-motion/cutscene code). Fixed-step flag byte `+0x2c` (default 0): `dt = timescale / 25.0` (`+0x28` = 25.0). Real-time (unscaled) variant `+0x1c`, read by `FUN_004e3690` |
| 4 | Reflection pass object (`CReflectionPass`, vtable `0xf20eec`, ctor `FUN_00a7ed20`) keeps the accumulator at `+0x1a28` (zeroed in the ctor). Execute (vtable slot 36, `FUN_00a7f080`) does `phase = dt * 0.45f + phase`, stores it as float, then **if 10000.0 < the unrounded sum, phase = 0.0** (bytes `d8 0d 4c 08 e3 00 d8 03 d9 13 d9 05 54 1f e2 00`; constants `0xe3084c` = 0x3ee66666, `0xe21f54` = 0x461c4000) | phase unit = 0.45 x simulation seconds. Reaches 10000 after 22,222.2 s (6.17 h); every scroll product (x0.03..x0.07) is an integer at 10000, so the wrap is a seam of at most 0.0075 x 0.07 = 5.3e-4 UV |
| 8 | PS (7535708): four `t0` samples (layer pairs), RGB = mean, `a = mean_alpha`; `o.a = cb1[0].w * (mean_alpha - 0.5) + 0.5` = **mean alpha** with w = 1 | |
| 10 | Source `t0` of the composite = shared texture at owner `+0x2c` = `Water\water_bump.dds` (init bytes `8d 4e 2c e8` / `8d 5e 24` bind bump to +0x2c and foam to +0x24) | 512 x 512 DXT5 |
| 11 | Ocean consumer: type object `DAT_01381fa0` +0x44 = owner `+0x34` (this normal texture) bound to PS t0 by `FUN_00a80300` | |

Layer geometry (proven, phase 0, vertices ordered (0,0),(1,0),(0,1),(1,1)):

| Layer | VS attribute | Base UV | Scroll (per phase unit) | UV velocity (per sim second, x0.45) |
|---|---|---|---|---|
| 0 | v1.xy | (x, y) | (+0.06, +0.05) | (+0.0270, +0.0225) |
| 1 | v1.zw | (1-x, y) | (-0.03, -0.07) | (-0.0135, -0.0315) |
| 2 | v2.xy | (x, 1-y) | (-0.04, +0.07) | (-0.0180, +0.0315) |
| 3 | v2.zw | (x, y) | (-0.07, +0.03) | (-0.0315, +0.0135) |

So layers 1 and 2 are mirrored copies of the same bump image, not four independent maps.
Ocean-side world scales of the generated normal image (formula-level, from the LOD0 PS):
0.00425438, 0.0638157, 0.2 and 0.0212719 per world unit give repeat lengths of about 235 m,
15.7 m, 5 m and 47 m; the alpha caustic lookups use 0.08 and 0.3 (12.5 m, 3.3 m).

**Phase fixtures** (probe `boundary_cases`; float32 stores emulated, x87 intermediates not):

| Fixture | Value |
|---|---|
| one default frame (dt = 1/60 f32 = 0.01666666754) | phase 0.0075000003; layer offsets (0.00045, 0.000375), (-0.000225, -0.000525), (-0.0003, 0.000525), (-0.000525, 0.000225) |
| 600 default frames | phase 4.4999995; layer offsets (0.27, 0.225), (-0.135, -0.315), (-0.18, 0.315), (-0.315, 0.135); wrapped sample origin (0.27, 0.225), (0.865, 0.685), (0.82, 0.315), (0.685, 0.135) |
| 16.667 ms frame from the default dt | dt 0.0166666973, phase step 0.0075000138 |
| first frame (elapsed 0) | dt = dt_prev x 10/11 = 0.0151515156 |
| 100 ms stall | capped at 67 ms: dt 0.0212424248 |
| wrap | start 9999.9990234 (10000 - 2^-10), dt 2/60: wraps to **0.0**; start 9999.5, dt 1/60: 9999.5078125 kept; comparison strict (10000.0 does not wrap) |

The accumulator is also reset to 0 by the reflection pass predraw (`FUN_00a7eff0`, vtable slot 15)
(`200.0 < y || y == 200.0`). **Inferred** consequence: on frames where the reflection pass has
nothing queued the phase does not advance and restarts from 0.

Separate vertex-wave clock (partly traced): `FUN_00a80da0`, called from init and from every
`UpdateWater` while `camera.y - 200 <= 66.6667`, rebuilds 16 rotation matrices (`owner+0x64`,
total (`IClock+0x40` while paused). Angles are `(ms & 0x3fffff) / 350` and `/ 225` (wrap 4,194,304 ms
= 69.9 min). Radian interpretation of the rotation builders (`FUN_004eaad0`, `FUN_004dbce0`) is
every frame without wrap (`FUN_00a78e70`) - a different clock, see section 8.

## 3. Reflection camera, clip and lifetime (proven unless marked)

| Item | Fact |
|---|---|
| Equality conventions differ | reflection mirror branch needs y > 200; ocean optical constants use `200.0 <= y` as "above"; predraw skip uses y >= 200. Only y == 200 exactly is affected |
| Clip plane | **Unresolved.** No clip plane or discard keyed on y = 200 was found in the pass setup; the pass flags `ctx+0xd14 |= 4` (reflection) via `FUN_0095a190` are the only per-draw marker found. Whether materials clip below the water is untraced |
| Lifetime | resources and pass live as long as the water renderer (`DAT_011919c0`); camera data is rewritten every frame; no per-view or per-tile allocation found |

The consumer of the `Advanced Display Settings/Water Detail` option (string `0xe1a097`) was not traced;
only the godrays submission gate in `FUN_00a8b790` is documented in section 8.

## 4. t5 scene colour and t7 depth ownership (proven)

* Bind: `FUN_00a80300` sets PS t5 = `ctx+0xcc4`, t7 = `ctx+0xcc8` (`FUN_0094da50`), with `ctx+0xcc4/cc8`
  filled by `FUN_009426f0` (called from the pass draw with `ecx = DAT_01191ab4`, the `CRenderEngine`):
  `ctx+0xcc4 = engine[0x144 + ring*4]`, `ctx+0xcc8 = engine+0x15c`.
* Creation (`CRenderEngine` buffers, `FUN_0092ca30`): `back_buffer_copy` x N (N = settings field
  clamped to 4 when above 3, ring index `engine+0x154` advanced and wrapped once per frame by
  `FUN_00942f60`), size `DAT_013d9aac` (swap-chain width/height), format = high dword of `DAT_013d9abc`
  = literal **0x1d (R8G8B8A8_UNORM_SRGB)** (`text_00930000.c` line 12904); `depth_buffer_copy` = format
  **0x29 (R32_FLOAT)**, same size; `motion_blur_velocity_buffer` 0x1c.
* Owner: the pass with id 44 `COPY_FRAMEBUFFER` (created here with `CreatePass(0x2c)`); it is configured
  with `+0x68(1, 0)` (colour-copy flag, bit 5 of the pass word at `+0xcd0`, `FUN_00942a40`) and `+0x70(1)`
  (depth-copy flag, bit 6, `FUN_0094bbd0`). `FUN_00949f10` (generic predraw, vtable slot 35) runs
  `ResolveSubresource(dst, 0, current colour attachment 0, 0, dst format)` (device vtable +0x98); with
  `0x14` the low nibble 4 path renders the (resolved, `mgr+0x1c`, when the set's depth is `mgr+0x18`)
  depth texture into the R32F target with a full-screen draw. Pass bits 0/1 are the *clear* switches
  (flags 0x100/0x200 in `FUN_0094ce20`), not copies.
* Pass order: passes live in a `std::map<int, ...>` (`FUN_00932d00`); ids from the name table:
  -99 REFLECTION, 44 COPY_FRAMEBUFFER, 48 **WATER** (`CreatePass(0x30)`, the ocean), 49 COPY_DEPTHBUFFER,
  53 POST_WATER (waves), 67 WATER_GODRAYS. Ascending-id execution is **inferred** from the sorted map.
  Consequence: the composite normal and the reflection exist before the opaque passes; colour/depth
  copies happen after the opaque passes and before WATER; the ocean pass itself requests no copy.
* Consequence for colour: the scene colour is an **8-bit sRGB** resource, so hardware decode makes
  `t5` linear and the ocean writes linear values to an sRGB target. The `sqrt` in the underwater
  post-process is the explicit gamma of a non-sRGB write (water-next.md, JC2-094).
  s5 scene 2,2,2; s6 foam 1,1,1; s7 depth 3,3,3. Reading them as D3D10 WRAP/MIRROR/CLAMP is
  **inferred**; filter arguments (`FUN_009203e0`) are not decoded.
* Unresolved: behaviour when MSAA is off (the resolve branch of `FUN_0094ce20` is unconditional in the
  function body; other `CopyResource` sites exist), the shader used by the depth copy, and the settings
  field that gives the ring length.

## 5. Ocean `cb0` / `cb1` producers (LOD0, PS 7540448)

`cb0` is the PS global block `DAT_013daf80 + 0x1c0` (145 vec4, uploaded by `FUN_00946c90`; the VS block is
the first 28 vec4 via `FUN_0092d460`). Built per frame by `FUN_0095f020`, which ends with `FUN_0095b150`.

Water colour / density = **fog layer 1** of the render engine (`DAT_01191ab4 = CRenderEngine`):

| `cb0` reg | Source | Notes |
|---|---|---|
| `cb0[13]` | layer 0: colour `engine+0xc8..+0xd3`, `w` = `engine+0xf8` (copied into `+0xd4`) | scene fog colour; the PS mixes toward it with `exp(-v2.w)` (that `v2.w` comes from this density is **inferred**) |
| `cb0[14]` | layer 1: colour `engine+0xd8..+0xe0`, **`w` = `engine+0xfc`** (copied into `+0xe4`) | **water colour and optical density** |

Getter/setter pair: `FUN_00936060` (get, layer index `p7`) / `FUN_0096c800` (set:
colour4 at `200 + p7*16`, start `0xe8`, end `0xf0`, density `0xf8`, `0x100`, `0x108`, each `+ p7*4`).
The only setter call: end of weather update `FUN_00a432d0` (from the weather frame `FUN_00a654b0`):

* **Density (w)**: `engine[0x13] = (200.0 <= camera.y) ? 0x3e99999a (0.3) : 0x3e19999a (0.15)`, written
  reciprocal world units in `exp2(-thickness * density)` (base 2 corroborated on the CPU side by
  `FUN_00a63a20`, which inverts it with `log(1 - k/512) / (log 2 * density)`).
* **Colour (rgb)**: lerp of a per-hour RGBA row, hour = `DAT_01191ab4 + 0xac` (0..24), rows
  `floor(hour)` and `floor+1` (wrapped to 0 when above 23), table pointer `[2]` when camera y >= 200 or
  (3 variants x 25 rows x 16 B, tables at weather-object `+0x1920` and `+0x1dd0`).
  The tables are **initialised to 1.0** by `FUN_00a517f0` and no filler was found in this pass
  (`gradients.bmp` loader `FUN_00a46bd0`/`FUN_00a45790` is a candidate; weather.md says the retail file's
  values are not colours). **Unresolved: the actual runtime RGB rows.** A parity implementation must not
  invent them (see section 7).

`cb1` of the ocean PS (`FUN_00a80300`, `ecx = DAT_01381fa0`; PS table writes `FUN_00958860`, VS table
writes `FUN_0096beb0`):

| PS reg | Content | Source / defaults |
|---|---|---|
| `cb1[0]` | sun specular colour `.xyz`, exponent `.w` = **256.0** | `.x = Rs*4*a*h*0.15*c*m`, `.y = (1-f7)*Gs*...`, `.z = f8*Bs*...` with `a = clamp(|sy*32|,0,1)`, `h = clamp(1-(camy-200)*0.003, 0.6, 1)`, `c = 1 - ctx[0xd08]`, `f7 = max(0, 0.8-|sy|*1.5)`, `f8 = max(0, 1-2*f7)`, `m = 1 (sy <= 0) or 8`; `sy` = object+0x54, `(Rs,Gs,Bs)` = `DAT_0108c5e4/e8/ec` (producer unresolved) |
| `cb1[1]` | `.x = 1/ctx[0xcb0] - 1/ctx[0xcb4]`, `.y = 1/ctx[0xcb4]`, `.zw` distortion | `.zw` = **(0.24, 0.045)** when `200.0 <= camera.y`, else **(0.8, 0.1)** (`0x3e75c28f/0x3d3851ec`, `0x3f4ccccd/0x3dcccccd`) |
| `cb1[2].xyz` | negated light vector | object `+0x50..` written by `FUN_00a8b790` from the main light (`DAT_01191c48+0x2c`, `+0x260..`) |
| `cb1[3..6]` | 4x4 matrix from `ctx+0xb50` after `thunk_FUN_004cf830` (**inferred**: inverse view-projection; the shader uses it for clip -> world) | |

Depth conversion (all depth consumers): `eye = 1 / (d * cb1[1].x + cb1[1].y)`. With `ctx+0xcb0 = camera+0x624`
and `ctx+0xcb4 = camera+0x628` (initialised to 40000.0) this gives eye = `ctx[0xcb4]` at d = 0 and
`ctx[0xcb0]` at d = 1 (reverse-Z reading if `+0x624` is the near plane - the field names are
**inferred**, the algebra is exact).

### Optical equations (from the disassembly, checked in the probe)

```
eye_u = depth(undistorted uv)        eye_d = depth(uv + n.xy * cb1[1].w * v3.x)
chosen = (eye_d < surface_eye) ? eye_u : eye_d          # strict <, movc
thickness = chosen - surface_eye                         # v5.w, not clamped
a = max(0, 1 - 2^(-thickness * cb0[14].w))               # SM4 `exp` is exp2
colour = lerp(caustic_refracted, cb0[14].rgb, a)         # refracted = t5 * (0.8, 0.97, 0.98) * caustic
```

| Boundary fixture | Result |
|---|---|
| thickness 0 | a = 0 |
| thickness = 1/density (3.333 at 0.3) | a = 0.5 exactly |
| thickness 3.0, density 0.3 / 10.0 at 0.3 / 10.0 at 0.15 | 0.4641133 / 0.8750000 / 0.6464466 |
| negative thickness (both samples in front of the surface) | a = 0 |
| `eye_d` 99.9999 vs surface 100, `eye_u` 130 | thickness 30 (foreground rejected) |
| `eye_d` = surface | not rejected: thickness 0 |
| density at camera y 200.0 / 199.999 | 0.3 / 0.15 |
| distortion at y 200.0 / 199.99 | (0.24, 0.045) / (0.8, 0.1) |
| `exp2` vs natural exp at thickness x density = 1 | 0.5 vs 0.632; they differ |

## 6. Colour and view identity: established versus untraced

Established: normal = UNORM data (0x1c, 9 mips); reflection = 0x1d sRGB (1 mip); scene copy = 0x1d sRGB;
depth copy = R32F; depth = D24S8 (0x2d). **The generated normal and the reflection do not share a
colour view**: separate resources, separate view records (`FUN_0094a850` clones the texture record's
SRV/RTV, `FUN_00963630` builds target sets), and different formats; the ocean binds each texture record,
not the other's view. The reflection RTV is the sRGB view, used again for the blur write-back.

Not traced: how SRV/RTV DXGI formats are derived inside `FUN_00950420` (assumed to be the typed resource
format, no typeless reinterpretation seen); the meaning of target-set argument 7 (0xf for colour sets,
0x7 for the blur RT sets; possibly a write mask); `o1` of the ocean PS (`SV_Target1`) and its alpha
semantics; whether the final colour is written through an sRGB view in every pass.

## 7. Proposed first stage for JC2-013 (implementable, exact variants)

**Supported variant S1** - all of: non-NVIDIA path (`WaterShader_lod0` VS 339512 / PS 7540448, not `nv*`),
camera `y >= 200` (optical constants) and `y > 200` (planar mirror), `watershader_lod0` only (not lod1/lod2,
not `waterhighend`, not `_stereo`), no waves layer, no godrays, no underwater/edgefade variant, default
quality. Anything else keeps the current approximate material and logs which gate failed.

1. **Clock**: a `WaterClock` resource reproducing section 2 steps 2-4 (EMA dt with cap 67 ms, timescale,
   pause = 1e-6, phase x0.45, strict wrap above 10000). Never substitute wall seconds. Expose the
   timescale hook so slow-motion matches.
2. **Composite pass**: 256 x 256 UNORM target with 9 mips, quad from the phase-0 table, the exact VS/PS
   above, `water_bump.dds` (512 x 512 DXT5, wrap sampler) and `GenerateMips` after the draw; alpha
   strength constant 1.0.
3. **Reflection**: 256 x 256 sRGB colour + D24S8, mirror camera as section 3 (up flipped, y' = 400 - y,
   projection copied), clear (0,0,0,0); the two-sub-pass blur may be a flagged follow-up. Clip plane is
   **unresolved**: render only geometry that survives the existing Bevy depth rule and record the gap.
4. **Ocean material**: port the LOD0 PS maths literally (4-layer normal blend with weights 1, 2.4, 1.5 and
   `max(v1.z, 0.4)`, exp2 attenuation, foreground rejection, caustic term, the non-Schlick view-angle
   terms, specular `pow(.., cb1[0].w)`), `cb1` distortion/specular constants from section 5, density
   0.3 above y = 200. Scene colour/depth: an opaque-pass colour copy and a depth copy (Bevy reverse-Z
   needs its own `x,y` pair - the formula is unchanged). Treat sRGB scene colour as already linear in a
   linear pipeline; do not add a second encode.
5. **Colour table**: expose `OceanOptics.water_colour_rows` and `sun_spec_colour` as explicit inputs; the
   retail producers are unresolved (section 5), so the default must be labelled "unrecovered" and parity
   must not be claimed until the rows or a capture are obtained.
6. **Acceptance** (headless): the probe's phase, exp2, foreground and branch fixtures reproduced in Rust unit
   tests; a texture-identity test (normal 9 mips UNORM, reflection sRGB, no shared view); a pass-order test
   (composite before opaque, copies before ocean). GPU captures remain required for sampler filters,
   blend state and `SV_Target1`.

## 8. Separate work items (not covered, not substituted)

* **Underwater / JC2-094**: `FUN_00a8b790` camera `y <= 200` branch (negated axes, `FUN_004541f0`),
  WaterBelow VS 333312 / PS 7532292, `Underwater*` post-process PS 7482432 and variants, density 0.15,
  distortion (0.8, 0.1), the postprocess t0..t6 bindings, `SHnWaterFog`/`MergedWaterFog` merged shaders,
  and the `sqrt` conversion.
  48)` (48 x 48 wave objects), gate `DAT_0108b8f8` and `FUN_00a89230`, waves VS 344216 / PS 7558872.
  Not shoreline-proven.
  rays, `Water\water_godrays_dif.dds`), submission gate `((y <= 200 && hour >= 6) || hour <= 18)` as
  decompiled (`DAT_01191ab4+0xac` = hour); density/volume untraced.
* **NVIDIA variants**: `CNvWaterHighEndRenderBlock` (`FUN_00a78b90`, vendor id 0x10de path, CUDA waveworks,
  gate `DAT_0108ab04 && DAT_013657fb && DAT_0108ab05`), simulation time accumulated unwrapped from
* **Other regular variants**: `watershader_lod1` (PS 7551512), `lod2` (7555468), `waterhighend` (7537032),
  `watershader_lod0_stereo`; the LOD selection rule was not traced.
* **Vertex waves**: VS regs 5..38 matrices from `FUN_00a80da0` (section 2 end).
* **Untraced producers**: `DAT_0108c5e4/e8/ec`, `ctx+0xd08`, `FUN_00a40080` weather scalars, weather colour
  rows, `DAT_01191ab4` ring length, depth-copy shader.

## 9. Corrections and additions to water-next.md

1. `cb1[0].w` of WaterBumpComposite is no longer a free alpha knob: it is the literal 1.0.
2. `cb0[14].w` is not weather-dependent: 0.3 above / 0.15 below y = 200, written every frame by
   `FUN_00a432d0`; the colour comes from fog layer 1 (hour-lerped table).
3. `t5`/`t7` are `back_buffer_copy` (sRGB 8-bit) and `depth_buffer_copy` (R32F), produced by pass 44.
4. Composite phase is `0.45 x` smoothed simulation seconds with a wrap at 10000, not a seconds clock.
5. The normal image has 9 mips (`GenerateMips` after the composite) and layers 1-2 are mirrored.
6. Texture dimensions: bump 512 x 512 DXT5 x10 mips, foam 256 x 256 DXT1 x9.

## 10. Verified, inferred, unresolved (summary)

Verified: sections 2 (steps 1-11), 3 (creation, camera maths, pass config), 4 (ownership and formats),
5 (cb0 layers, density, distortion, exponent, equations), pass-name table, shader strings, DDS headers.
Inferred: ascending pass-order execution; sampler enums; pointer-sized view-format derivation;
reverse-Z naming of `ctx+0xcb0/cb4`; `cb1[3..6]` is an inverse view-projection; slot-15 phase reset
consequence; radian units of the vertex-wave clock.
Unresolved: water colour table rows; sun colour globals; reflection clip plane; MSAA-off copy path;
depth-copy shader; `SV_Target1` meaning; LOD selection; everything in section 8.
