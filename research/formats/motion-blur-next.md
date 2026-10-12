# MotionBlur pass, velocity buffer and the "Motion blur" option (JC2-179)

Evidence brief for the last PostEffects pass `0x0096a420`. Builds on `post-manager-defaults-next.md` (JC2-135: S21..S23
defaults and the clock dt) and `night-exposure-next.md` (pass order). Probe: `research/tools/motion_blur_probe.py`
(54 checks, exit 0; pins nine function bodies by sha256, the load-bearing instructions, and the shader listings).
Addresses are runtime addresses of the retail `JustCause2.exe` (sha256 `df99a479...4f80`); shaders are from
`DX10_Shaders_F.shader_bundle` (sha256 `db8c67a0...2e98`). "Proven" = pinned by the probe; "inferred" = stated as such.

## 1. Summary (answers to the task questions)

* The pass is a **camera-reprojection plus object-velocity blur**, one full-screen triangle, pixel shader pair
  **`MotionBlur`** (used) and **`RadialBlur`** (selected only when S23 > 0.0001, which never happens in shipped data).
  The vertex shader is `ScreenSpaceTex`, not the bundle's `motionblur` vertex shader (proven: init `0x00922160`).
* It runs **every frame in normal play**: the gate is the byte `P+0xdd` (P = PostEffects manager), which is the user option
  "User/Display settings/Motion blur" and **defaults to on** (`0x006010b0` stores 1 at settings `+0xac`). It also needs the
  alternate-mode flags `P[9]`, `P+0x25` clear (inferred off, JC2-135 Gap 1). Order: tone-map (`0x0093e660`) < MotionBlur <
  final pass `0x00966a40` (proven by the probe from call positions).
* At 60 fps the strength is **0.06** (`2 * S21 * min(1, 1/(30 dt))`), the blur is **7 taps** (centre plus 3 each way) along
  the per-pixel velocity, the step length is clamped to **S22 = 0.006 uv**, and motion below a **1/127 uv soft dead zone**
  produces no blur. With the shipped constants the blur is subtle (steps of about 0.001 uv for object motion).

## 2. Enable chain

1. Option registration at `0x00440000` region (string `User/Display settings/Motion blur`, string table `0x00e1e054`,
   boolean type 5, option id 8). Option values are an int array in the settings object; id 3 (Gamma) is `+0x98` and
   id 8 is `+0xac` (**id -> offset is inferred** from those two; `0x00612050` consumes `+0xac` for the post manager).
2. Defaults `0x006010b0` (reset): `+0xac = 1` (proven bytes `bb 01000000 ... 89 9e ac000000`). Gamma `+0x98 = 0x32`.
4. `FUN_00946aa0`: `P+0xdd = (P+0x34 != 0) & v` (23-byte body pinned). `P+0x34` is the pass object created by the manager
   constructor `0x009531d0` (`FUN_00929f40`, 0x54 bytes). The manager constructor also forces `P+0xdd = 1` before the first
   apply.
5. `0x0096f360` (PostEffects render): `P+0xdd` is tested exactly twice, for the full-screen clear `0x0094ce20` (before the
   tone-map pass) and for the MotionBlur call, which additionally requires `!(P[9] || P+0x25)`.

The whole post stage is called from `0x0096d400` only when the render manager's post flag and `post+0xdc` are set, with
arguments (final target `R+0xdec`, colour `back_buffer_copy[R[0x55]]`, depth `depth_buffer_copy` `R[0x57]`, velocity
`motion_blur_velocity_buffer` `R[0x58]`).

## 3. Pass object and draw (`0x00922160`, `0x0096a420`)

Pass object fields (proven bytes): `[0] = [P+0x158]` (vertex layout), `[1] = [P+0x160]` (vertex shader `ScreenSpaceTex`),
`[2] = pixel shader "MotionBlur"`, `[3] = pixel shader "RadialBlur"`, `[0x14] = [P+0x154]` (full-screen triangle vertex
buffer: (-1,1), (3,1), (-1,-3)), `[4..0x13]` = **previous matrix**, initially identity.


* Samplers t0, t1, t2 = colour, depth, velocity (slots 0..2). Address-mode codes 2, 1, 2 and filter code 3 on all three
  (engine enum values; not decoded further, inferred mirror / wrap / mirror and linear).
* cb1 slot 4 (one vec4) = `(strength, S22, 1/near - 1/far, 1/far)` with `strength = 2 * S21 * f`,
  `f = 1` for `dt <= 1/30`, else `(1/dt) * (1/30)` (constant `0x00e4b83c`); all three S-values are zero when `dt <= 0`.
  The two clip distances are read from the camera block (`+0x624`, `+0x628`) and ordered min/max (`1/far` uses the larger).
  `S1` = uv to ndc (scale 2, -2, translate -1, 1), `C` = 16 floats at camera `+0xe4`, `Pm` = this pass's stored previous
  matrix, `S2` = ndc to uv (scale 0.5, -0.5, translate 0.5, 0.5).
* After the draw: `Pm = C` (rep movs of 16 dwords from camera `+0xe4`), so the pass stores the **previous frame's `+0xe4`
  block**. `+0xe4` is a copy of `view * projection` (`FUN_00933a10`: `+0x2e4 = +0x264`, `+0x324 = +0x2e4 * +0x2a4`,
  then `+0xe4 = +0x324`). That the matrix equals view-projection while the pass multiplies two such matrices is **not
  reconciled** (Gap 1).
* Draw `0x0092f950` (primitive 4, one instance of the 3-vertex triangle).

## 4. Pixel shader `MotionBlur` (listing pinned)

All maths per pixel (uv = v1):

1. `d = t1(uv).x` (depth_buffer_copy, R32_FLOAT, the raw rasteriser depth). `z = 1 / (d * cb4.z + cb4.w)` (linear view depth).
2. Reprojection: `p = (uv.x * row0 + uv.y * row1 + d * row2 + row3).xyw` from the matrix; `cam = p.xy / p.w - uv`
   (a uv offset; both this and the velocity buffer give "previous uv minus current uv").
3. Object velocity `t2(uv)`: `vel = t2.xy * 0.0625 - 0.03125`. The velocity pixel is used instead of `cam` when
   `t2.z < sat(z * 0.01 + 0.00588235)` (`0.00588235 = 1.5/255`; `t2.z` is the writer's `w * 0.01`, so this keeps velocity
   writers that are nearer than or equal to the scene depth within 1.5 quantisation steps), otherwise `cam` (static scene).
4. Soft dead zone per component: positive `v -> sat(v - 0.00787402)`, negative `v -> -sat(-v - 0.00787402)` (1/127).
5. `step = v * cb4.x` (strength). `n2 = |step|^2`; if `n2 <= 1e-8` the output is `sqrt(colour(uv))` (no taps).
6. Otherwise `step *= sat(cb4.y / |step|)` (**S22 is the maximum step length in uv**), then taps
   `uv + k * step` for `k = -3, -2, -1, +1, +2, +3` (`sample_l`, level 0) plus the centre, summed and multiplied by
   `1/7` (0.142857).
7. Output `rgb = sqrt(mean)`, `a = 1`. The `sqrt` means the pass writes a gamma-2 style encoding of whatever t0 holds;
   whether t0 is linear at that point is part of Gap 2.

`RadialBlur` (never selected with shipped data): centre-to-pixel vector `c = uv*2 - 1`, `r = sat(|c| - cb8.w)`, direction
`-normalize(c) * r * cb8.z` clamped to length `cb8.y`, 7 taps `k = -3..3` (loop counter starts at raw -3), weight 1/7,
output without `sqrt`. It reads **cb1[8].yzw**, which the MotionBlur pass **never uploads** (only cb1 0..4), and the bundle
has no vertex shader named `radialblur`. Combined with S23 = 0 in every shipped block (JC2-135) this variant is dead code in
the shipped game. S23 has no writer other than the block loader.

## 5. Velocity buffer

* Targets (`0x0092ca30`, proven formats): `motion_blur_velocity_buffer` format 0x1c (R8G8B8A8_UNORM) and
  `depth_buffer_copy` format 0x29 (R32_FLOAT), both at screen size through `0x00950420`.
* Writers (all pixel shaders in the bundle with `velocity` in the name: `RenderVelocityBuffer`, `...Clamp`, `...Mask`,
  `...NoTex`, `...NoTexClamp`, `...Opacity`, `...FadingVelocityBuffer`, plus `CarPaintPreZVelocity*`,
  `VegetationFoliageVelocity`; vertex shaders `SkinnedDepthVelocity(8)`, `GeneralPreZVelocity`, `RenderPreZVelocity*`,
  `Tex0Velocity`, ...): each vertex shader outputs the **current clip position** (`o2`) and the **previous clip position**
  (`o3`, from a second matrix; `SkinnedDepthVelocity` uses cb1[54..57] for the current and cb1[58..61] for the previous
  view-projection, skinned with the palette in cb1[0..53]). The pixel shaders compute
  `delta = cur.xy/cur.w - prev.xy/prev.w` (ndc), optionally clamp each component to `+-cb1[0].x` (the `Clamp` variants),
  and write `o0.xy = delta * (-8, +8) + 0.5`, `o0.z = cur.w * 0.01`, `o0.w = 0` (or the texture alpha for the alpha-tested
  `RenderVelocityBuffer` / `...Opacity` variants, which `discard` on `cb4[3].w` / `cb4[4].x`), and `o1.x = depth`
  (target 1 = `depth_buffer_copy`). `RenderVelocityBufferMask` writes `(0.5, 0.5, 1, alpha)`: zero velocity at the far
  depth code.
* Decode in the MotionBlur shader: `t2.xy * 0.0625 - 0.03125` = `(prev - cur) / 2` in x and `(cur - prev) / 2` in y,
  i.e. previous uv minus current uv. The 8-bit range makes the buffer saturate at +-0.03125 uv per frame.
* Pass names (enum strings at `0x00959xxx`): `Z_PASS` (0xa), `Z_AND_VELOCITY_PASS` (0xb), `COPY_VELOCITY_PASS` (0xc).

## 6. Not decoded here (Gaps)

1. **Matrix semantics (blocks camera-motion blur parity).** The camera block `+0xe4` is `view * projection` as built, yet
   the pass multiplies `S1 * C * Pm * S2` with `Pm` the previous `+0xe4`. A reprojection needs an inverse of the current
   view-projection; either the block is an inverse (the same block feeds other full-screen passes at `0x00925xxx` after the
   same `S1` product, which suggests an inverse) or `+0x264`/`+0x2a4` hold inverse matrices. Needs the writers of
   `+0x264`/`+0x2a4` (`FUN_00963940`, `FUN_00967660`, the camera copy at `0x0091xxxx`) and which of `+0x624`/`+0x628` is
   near. Without it the camera term cannot be implemented.
2. **Colour input and ordering.** Both the final composite (`0x009256d0`) and MotionBlur read the same colour argument
   (`back_buffer_copy`, call chain `0x0096d400 -> 0x0096f360 -> 0x0096a420`) and write the same target (`P[0x59]`). Whether
   an intermediate copy (`FUN_0092cf80`, `FUN_0095baf0`) refreshes the copy between them, i.e. what MotionBlur actually
   blurs (raw scene or composited image), and the blend/depth state of its draw are not decoded.
3. **Velocity writers, CPU side.** Which render blocks use which velocity shader, the source of the per-object previous
   matrices (cb1[58..61]), the `Clamp` constant cb1[0].x, the binding of the velocity buffer as target 0 of the
   `Z_AND_VELOCITY_PASS`, the clear value of the velocity buffer (the shader encoding implies `(0.5, 0.5, 1)`), and whether
   velocity is written for dynamic objects only.
4. Option storage: id -> offset map is inferred; the user setting is persisted in the options file (key `Motion blur`),
   not read here. Default on is proven.
5. Sampler enum values (address codes 2/1, filter code 3) of `FUN_009203e0` / `FUN_0095e900`.
