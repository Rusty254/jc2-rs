# Post composite families: blur chain, bloom, overlay, depth of field, heat haze (JC2-138)

11 October 2026. Runtime task with the shader detail it needed recovered first. Addresses are retail VAs of `JustCause2.exe`
sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`; shader offsets index `DX10_Shaders_F.shader_bundle`
sha256 `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98` (disassembled with `research/tools/sm4dis.py`; nothing
copied). Builds on `night-exposure-next.md` (settings table, JC2-124) and `post-composite-variant-next.md` (shipped variant, JC2-103).
Legend: **proven** = read from the bytes/disassembly; **inferred** = one unchecked link.

## 1. The composite that ships above water (proven)

`DepthOfField_EdgeFade` (PS at bundle offset 4863324, 1900 bytes; `HeatHazeDOF_EdgeFade` at 6142692, 2104 bytes) is, per pixel, with
`t0` scene, `t1` blurred scene, `t2` depth, `t3` adapted luminance A, `t4` bloom (all linear, sRGB views) and `uv` the screen position:

1. `eye = 1 / (depth * cb1[2].z + cb1[2].w)`; `ramp = sat((cb1[5].z - eye) * cb1[5].w) + sat((eye - cb1[5].x) * cb1[5].y)`;
   `m = sat(ramp * cb1[0].w + cb1[0].z)`; `C = t0 + m * (t1 - t0)`. With `cb1[5] = (S5, 1/S5, S6, S6 > 0 ? 1/(0.5 S6) : 0)`,
   `cb1[0].z = S4`, `cb1[0].w = S25` (all from `FUN_009256d0`). When `S5 <= 0` the CPU writes `cb1[5] = 0`, so the `Scene` family is the
   same maths with `m = sat(S4)`; no separate branch is needed.
2. Overlay with `b = t4(uv)`: `b < 0.5 ? 2 C (S3 (b - 0.5) + 0.5) : 1 - (2 - 2 C) (1 - (S2 (b - 0.5) + 0.5))` per channel
   (`cb1[3].y = S3` multiply branch, `cb1[3].x = S2` screen branch).
3. Saturation clamp, `* (A + S7 - 1) * tint`, the radial vignette `1 - sat((length(uv - 0.5) - 0.35) * 2.2)`, `sqrt` (as in the earlier briefs).

The `HeatHazeDOF` variant differs in one place: it samples `t5` at `4 * v1.zw` (`v1.zw = 0.1 * uv`, so `0.4 * uv`), forms
`d = (t5.rg * 2 - 1) * cb1[0].x + uv` with `cb1[0].x = S26 * 0.004`, and samples **`t1` at `d`** instead of `uv`; every other term uses
the undistorted `uv`. `t5` is `Q[3]` (`water_normal_texture`, WRAP, mip chosen by derivative: `FUN_009256d0` binds `in_ECX[3]` to slot 5 on
the heat-haze branch). The family is chosen when `S26 > 0.001` (compared as a double) and `S5 > 0`: only the Desert block (`S26 = 1`).

## 2. The blur chain (proven)

`FUN_0096f360` calls `FUN_00967ee0` (single call site) when the alternate (CUDA) path is off. The blur shader is **`Blur`**
(PS 965692 / VS 44952; the string at 0xe5c320 is `Blur`; `BlurHorizontal/BlurVertical` belong to the separate sun-halo object
`FUN_00974480`). The PS is seven taps with weight 1/7 each; the taps are the four-vector vertex attributes written by `FUN_0092ec50`
(three quads, one per pass). With `s` the spread and `(W, H)` the dimensions of the source of that pass the tap offsets (in uv) are
`(c_x * s / W, c_y * s / H)` for `c` = (0, 0), (0.060393, 0.15534), (-0.298919, -0.147507), (0.480085, -0.139708),
(-0.346193, 0.569732), (-0.161941, -0.817447), (0.805884, 0.592074); the float32 constants are read at 0x92ecd4..0x92edbb (bit patterns pinned in
`post_effects_render.rs::the_blur_tap_table_matches_the_exe_constants`). Spreads: **1.75** (pass 0, full-size source), **2.625**
(pass 1, half-size source), 3.5 (pass 2, underwater only). Pass 0 writes `large_temp_texture[0]`, pass 1 writes `[1]` (both
w/2 x h/2, DXGI 0x1d); the composite's `t1` is `[1]`. Sampler: clamp, `MIN_MAG_LINEAR_MIP_POINT`.
HighPass (PS 6147124, `FUN_00953060`) reads `[1]` and writes `small_temp_texture[0]` (w/4 x h/4, 0x1d): `pow(c, S1)`, saturation mix `S0`,
`* A` (A = the adapted luminance, `t1` of HighPass); that texture is `t4`, so **the overlay's `b` is the blurred scene times A**
(S0 = S1 = 1 in the shipped data). The only `0.142857` weight literal is the disassembly's 6-digit print of 1/7 (inferred as `1/7f`;
the difference is below 8-bit resolution).

## 3. Implementation notes (`post_effects.rs`, `post_effects_render.rs`, `post_effects.wgsl`, `post_blur.wgsl`)

* Targets are 8-bit sRGB like the native ones; half = size / 2, quarter = size / 4 (integer division, at least 1).
* Eye distance: Bevy's reverse-Z infinite depth gives `eye = near / d`; the camera near plane travels in `PostPass.view.x`. Sky (`d = 0`)
  gets the full ramp as the native far plane does. Depth is Bevy's depth prepass (**non-native stand-in**, as `sea.rs`); MSAA samples: the
  nearest is used (inferred; the native depth copy takes a min over samples at the sea).
* The vignette is now rendered (it is in every shipped composite). Film grain (cutscenes only) is not.

## 4. Gaps

1. **Heat haze (S26)**: needs the generated water normal texture (`Q[3]`; JC2-095 composite normal map, not implemented in the port).
   `PostPass.blend.x = 0.004 * S26` is carried; the shader transcription is above. Blocks the Desert look.
2. **Underwater composite family** (camera y < 200): the port uses the DepthOfField maths there (no distortion, caustics or WaterBelow
   coupling); Underwater block values (`S5 = 8`, `S25 = 0.65`, `blur 0.15`) are applied to it.
3. The native depth copy contains the water surface seen from below and uses the engine's own MSAA resolve; Bevy's prepass depth does not
   contain transparent geometry (the sea).
4. Whether the `Blur` weight is exactly `1/7f` (the disassembly prints 0.142857).
5. Film grain in cutscenes (JC2-103 gap 4) and the CUDA/bokeh path (S(0x1c)) remain open.

## 6. Runtime of gaps 1-3 (JC2-194, with the part of JC2-193 it needs)

* **Water normal texture (`Q[3]`)**: `water_normal.rs` / `water_normal.wgsl` / `post_effects_render.rs`. A 256 x 256 `Rgba8Unorm`
  texture with 9 mips; each frame four WRAP samples of `textures\water\water_bump.dds` (kept as BC3, full 10-mip chain) at the layer
  table of `water-consumers-next.md` section 2 (rgb = mean, a = mean alpha), the phase `dt * 0.45` per frame (the same smoothed
  alpha strength 1.0, clock, size, format, mip count. **Inferred**: the source sampler filter (linear, derivative-chosen mip), the
  box `GenerateMips`, no reset of the phase by the reflection pass (JC2-095: accumulator restarts on empty reflection frames).
  The composite binds it as `t5`/`t6` with a WRAP, linear, point-mip, unbounded-LOD sampler (the decoded s5/s6).
* **Heat haze** (`HeatHazeDOF_EdgeFade`, desert block): the blurred-scene lookup is displaced by `(t5(0.4 uv).rg 2 - 1) * 0.004 S26`
  when `S26 > 0.001` and `S5 > 0`. Rendered. JC2-193 capture (tile 26,41, 12:00, 166 m up, `_extract/JC2-193/before.png` with the haze branch forced off, `after.png` with it on, `diff.png`): the two differ only along edges and fine detail (up to ~5 px, the 0.004 uv scale) across the lit terrain, none in the open sea; the wiring (`S26 = 1`, `S5 = 50` desert day, `0.4 uv`, WRAP, `rg * 2 - 1`) and the layer table were re-read against this document and `water-consumers-next.md` section 2 and match. The pair is not a controlled A/B (two runs, the 40 s clock and moving vehicles also differ), so the size of the effect is unproven.
* **Underwater family** (`Underwater_EdgeFade`, PS 7485260, disassembled again for this task): selection `200 > camera.y` strict (CPU,
  `PostPass.view.y`); `uv' = uv + 0.01 (t5(0.1 uv).rg 2 - 1)`; depth, scene, blur and bloom read at `uv'`; the world position comes from
  `uv'` and that depth (`world_from_clip`: camera rotation times the inverse of Bevy's reverse-Z projection, camera-relative so the f32
  error stays small at 16 km); gain `1 + B D Sg A2^3` on the lit colour, then the vignette at the **undisturbed** `uv` and one `sqrt`.
  The cb1 values (`S5, S6, S4, S25`, ...) are the ones the DepthOfField family takes (the Underwater block of the settings table).
  `cb0[6].y` = minus the camera forward Y.
* **Depth source decision**: Bevy's depth prepass lacks the sea (alpha blended), the native copy has it. Non-native stand-in: in the
  Underwater branch the flat sea plane `y = 200` is intersected analytically along the pixel ray and the nearer of it and the prepass
  depth is used; the sky is limited to the native far plane (40000 m). It ignores the ocean's vertex waves.
* Captures (`_extract/JC2-194`): `probe_b.png` / `up_before.png` = before (DepthOfField composite under the sea), `probe_a.png` /
  `up_after.png` = after, tile 40,3, camera y 175 / 190: the visible change is the screen wobble of up to 0.005 uv on near objects
  (right edge of `up_*`); the caustic gain only shows within a few metres of geometry below the surface (`D <= 0.4` inside 6 m,
  `Sg = 0` at the surface), none was in these shots.

Remaining gaps: (a) the native water surface in the depth copy (ocean displacement, WaterBelow draw) versus the plane; (b) the
`GenerateMips` and source-sampler filters; (c) phase reset by the reflection pass; (d) a below-sea capture with geometry within
6 m and a desert capture for heat haze; (e) film grain, stereo `t9`, the CUDA path (unchanged from section 5).
