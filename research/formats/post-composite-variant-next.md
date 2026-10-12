# Post composite: shipped variant, flag/setting writers, Q[3] order, samplers, grain (JC2-103)

11 October 2026. Research only: no Cargo, GPU, game launch, runtime edit or asset copy. Closes the gaps listed in
`underwater-render-next.md` (JC2-094); the settings VALUES per hour are `night-exposure-next.md` (JC2-124) and are not
repeated. Addresses are retail VAs of `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`;
shader offsets index `DX10_Shaders_F.shader_bundle` sha256 `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`.
The one data file read is `textures\filmgrain.dds` (sha256 `5ca76ddf623e52d69f0643fc465cb6d5763fcbfe2cca67bfad553bcb7b816334`,
2,176 bytes; nothing copied).

Reproduce (prints one JSON document, exit 0):

```
python -B research/tools/post_composite_variant_probe.py --models-only            # 45 model cases, no game needed
python -B research/tools/post_composite_variant_probe.py --game <game folder> [--pseudo <export>] [--jc2tool <exe>]
```

Full mode asserts both hashes, 3 image constants, 19 byte contracts, the xrefs of the `edgefade`/`filmgrain` literals, the byte stores
to the post-enable flag, both callers of the P+0xdc setter, 27 decompiler-export function bodies (fragments), the EdgeFade/FilmGrain
deltas of all four shader families, the eight WaterFog shaders and the grain texture statistics. Legend: **proven** = read from the
binary and pinned by the probe; **inferred** = follows from proven facts with one unchecked link; **unresolved** = not traced.

## 1. Headline

1. **Shipped variant (answers the JC2-094 question): normal play uses the `_EdgeFade` family, cutscenes the `_EdgeFade_FilmGrain` family.**
   Underwater gameplay is therefore **`Underwater_EdgeFade` (PS 7485260)**, not the base PS 7482432; above water it is
   `DepthOfField_EdgeFade` by day and night (`HeatHazeDOF_EdgeFade` in the desert block, `Scene_EdgeFade` only if S5 <= 0). The base
   shaders run only when `/edgefade=0` is passed on the command line.
2. The two flags and the grain setting are fully traced (section 2). Defaults: edge-fade option 1, film-grain option 1, `setting[0x1b]` 0;
   `setting[0x1b]` is forced to 1.0 only while a cutscene runs.
3. **`Q[3] = water_normal_texture` (H1) holds in the shipped order** (inferred, section 4); H2 (null SRV) is out unless the system order
   differs from the registration order.
4. **Correction:** the EdgeFade vignette is **radial** (`length(uv - 0.5)`), not per-axis; it multiplies only the colour; the grain is added after it.
5. The sampler state of the composite is decoded to `D3D10_SAMPLER_DESC` fields (section 5): t5/t6 are `MIN_MAG_LINEAR_MIP_POINT`, WRAP,
   `MaxLOD = FLT_MAX`; every other composite input is a mip-0-only sampler. The "3.0" argument at the s5/s6 call is not read.
6. Object water fog: the CPU picks the `Merged*WaterFog` variant when the context camera Y is **below 200.75** (not 200). Those are the
   only WaterFog shaders in the bundle.

## 2. The three inputs of the variant choice (proven)

Choice (FUN_009256d0, JC2-094): `p = (Q+0x69 == 0 || S(0x1b) <= 0.01) ? (Q+0x68 != 0) : 2`; p selects base / `_EdgeFade` / `_EdgeFade_FilmGrain`
in each family (names from FUN_00939320: `Scene` Q[0xb..0xd], `DepthOfField` Q[8..10], `HeatHazeDOF` Q[0x14..0x16], `Underwater` Q[0xe..0x10],
stereo +3). FilmGrain variants contain the vignette too (probe: grain delta is 7 lines on top of the 6/7-line EdgeFade delta in all four families).

| Input | Writer | Value |
|---|---|---|

Consequences: gameplay variant = `*_EdgeFade`; cutscene = `*_EdgeFade_FilmGrain` (probe fixtures: the boundary `S(0x1b) = 0.01` stays EdgeFade,
0.0100001 switches; `/filmgrain=0` keeps EdgeFade in a cutscene; `/edgefade=0` in a cutscene still selects the FilmGrain shader because it contains the vignette).
"Cutscene start/end" is inferred from the callers (`cutscene.skip`, `gui.cutscene`, `NCutscene::CCutscene` in boost assertions).

state machine FUN_00554b00 locks S(4), S(0xb..0x10)).

## 3. Defaults and writers of the settings table (proven except where marked)

* **P is constructed by FUN_009413a0** (allocation 0x16c bytes in the CRenderEngine constructor FUN_00940e60: `P = engine[0x31]`). It sets
  `S(0x12) = 200.0`, `S(0x13) = 1.0`, `S(0x14) = 12.0`, `S(0x15) = 0.03`, `S(0x16) = 0.006`, `S(0x17) = S(0x18) = 0`, `S(0x19) = 1.0`,
  `S(0x1a) = S(0x1b) = S(0x1c) = 0`, all locks 0, `P+0xf0 = 0`, `P+0xdc = P+0xdd = 0`, `P+0x24 = P+0x25 = 0` (bytes: export fragments + `P[9]`/`P+0x25` stores).
  S(0..0x11) are not written by the constructor.
  P+0xdc enable (the other caller is FUN_007c9770, also with 1; probe: exactly two call sites). File values (JC2-124): `blur 0.3`,
  `saturation 1`, `#679def2a 1`, `#84a1ec42 0.8`, `#ab2627cc 1`, `#ae110681` (S(5)) **0.000002**, `#e57d0a03 1`; the keys `bloom/exposure/contrast`
  not write S(0x12), S(0x13), S(0x14), S(0x1b), S(0x1c).
  read; the caps dword identity is not).
* First frames: until FUN_0091d160 first runs (hour-block flag `Z+0x1ace`, JC2-124) S(5) is the file's 0.000002, i.e. a depth-of-field ramp that is full at any
  distance. Whether any rendered frame precedes the first gradient update is **unresolved** (Gap 3).

## 4. Q[3] / water normal order and the post-enable chain

* `CRenderEngine::CRenderEngine` FUN_00956430 stores **byte `engine+0xec4 = 1`** (bytes asserted); the only byte store to that offset in the 0x90..0x9f range is that
  **P's resources, the luminance object and Q (`P[0xc]`, FUN_009370b0 + FUN_00939320) are always created**.
* `FUN_0095dd00` (the water renderer's call, `FUN_00a8b220` end) stores `water_normal_texture` into `Q[3]` only `if (P+0x30 != 0)`. The registry is a
  vector: `CGameSystemManager` slot 5 (FUN_0074ec90 -> FUN_00982820) pushes (name, creator) pairs; FUN_005b0db0 registers `CRenderEnginePostInitProxy` **before**
  `CLandscapeManager` (call order in the function); FUN_005043a0 creates in vector order and FUN_0066de10 runs every system's slot-2 init in vector order (loop bodies read).
  So Q exists when the landscape init stores the texture: **H1** (inferred; the caller of the registration slot `+0x30` and a single create/init pass were not located, Gap 2).
* Nothing re-creates Q after init (FUN_009531d0 has one caller), so the stored pointer is not lost on resize.
  at engine init, bytes of both gates asserted at 0x96d502 and 0x96f366) are non-zero, and `P[0xc] != 0`; otherwise FUN_0096d400 resolves the scene directly into the swap chain.
* `P[9]` (alternate source) and `P+0x25` stay 0: constructor stores asserted, no other writer found by two pattern scans (direct `P+0x24/0x25` stores and `[9]` stores in the post
  functions). So the composite binds `back_buffer_copy` as t0 with mode 0, as in the JC2-094 chain (inferred: a writer reached only through a pointer alias would escape the scan).
* `P+0xdd` (sun-halo / copy gate) = `(P[0xd] != 0) & option[8]`, option 8 defaults to 1 (FUN_006010b0).

## 5. Samplers (proven bytes; enum meaning = the descriptor builder, not a name table)

The state words are decoded by FUN_00940170 straight into the D3D10 descriptor (`Filter, AddressU/V/W, MipLODBias, MaxAnisotropy, ComparisonFunc, BorderColor[4], MinLOD,
MaxLOD`) and created through `[device+0x158]` (= `ID3D10Device::CreateSamplerState`, index 86). Address values are passed unchanged (1 WRAP, 3 CLAMP). The engine
filter enum indexes a table at 0xF22858 (min/mag linear bit, mip bit): 1 = point/point-mip, 2 and 4 = linear, point mip, 3 = linear, linear mip, 5 = point, linear mip, 7 = anisotropic.
`MaxLOD = 0` when the min enum is <= 2, FLT_MAX otherwise; `ComparisonFunc` 0 maps to NEVER-with-no-compare-bit (value 1, `Filter` unchanged).

| Slot | Used for | Address | Filter | MinLOD / MaxLOD |
|---|---|---|---|---|
| s0, s1, s4 | t0 colour, t1 blur, t4 bloom | CLAMP | `MIN_MAG_LINEAR_MIP_POINT` (0x14) | 0 / 0 (mip 0 only) |
| s2, s3 | t2 depth, t3 luminance (R32F) | CLAMP | `MIN_MAG_MIP_POINT` (0) | 0 / 0 |
| s5, s6 | t5/t6 water normal (distortion RG, caustic alpha) | **WRAP** | `MIN_MAG_LINEAR_MIP_POINT` (0x14) | 0 / **FLT_MAX** |
| s8 | t8 film grain | WRAP | `MIN_MAG_LINEAR_MIP_POINT` | 0 / 0 |

`MaxAnisotropy = 1` everywhere (FUN_00970020 stores the 4th argument, 0.0, as MinLOD; the float 3.0 pushed at the s5/s6 sites is the 5th argument and is **not read**, bytes of the
callee asserted). The normal texture has 9 mips: its sampler picks one mip per pixel by derivative, point-sampling the chain (no trilinear blend); a port using one fixed mip or trilinear
differs. s7 is unused, s9 (stereo) is not decoded.

## 6. Film grain (proven formula; texture proven; SRV format inferred)

`C = C * vignette + (grain.x - 0.5) * 0.018` just before `sqrt`, `grain = t8(uv * (16, 8) + cb1[3].zw)`. `cb1[3].zw` are LCG-random offsets per frame (JC2-094). Texture:
`textures\filmgrain.dds` is **64 x 64, DXT1, no mip chain**; decoded red channel 90..164 (mean 127.17/255 = 0.4987, stddev 20.84/255). If the SRV is UNORM the additive term is
-0.00265..+0.00258 linear, stddev 0.00147 (before the `sqrt`): a faint, 16 x 8 repeated screen-space grain, mean about -0.00002. Whether the loader creates an sRGB SRV is **unresolved**
(it would change the numbers). Grain runs only when `S(0x1b) > 0.01`, i.e. in cutscenes.

Vignette (all four families, identical text): `1 - sat((length(uv - 0.5) - 0.35) * 2.2)`; centre 1.0, 1.0 out to r = 0.35, 0.67 at r = 0.5 (edge middles), 0.215 in the corners, 0 beyond r = 0.805.
It is a circle in uv space, so an ellipse in pixels on a 16:9 screen.

## 7. Object water fog (Merged block type) CPU selection (proven)

appended (sprintf `Merged%s`; strings `WaterFog`, `SH1WaterFog` ...). The draw FUN_009458a0 selects `index = (200.75 <= ctx_cam_y ? 0 : 4) + class` (class from FUN_00956f70, 0..3);
`ctx+0xc50..0xc58` is the context camera position (copied from camera `+0x54..` by FUN_009521e0; the float 200.75 is at 0xEB817C). The shader then gates the fog itself on the
cb0 camera `y < 200` (JC2-094). Hence between y = 200.0 and 200.75 the WaterFog shader runs with the fog term off (harmless), and above 200.75 the plain variant runs. Probe: the bundle contains
exactly these 8 WaterFog shaders; no other block type has a WaterFog variant (terrain/foliage water terms are not in shaders of that name).

## 8. Output override, third blur pass (proven / partly)

* `T+0x60`/`T+0x64` (output override): FUN_0096d400 uses `T+0x64` as the composite target only while the countdown `T+0x60` is 0; at 0 it calls **FUN_0092d650 =
  `D3DX10SaveTextureToFileA`** (the screenshot/capture path) and sets the countdown to -1; a positive value counts down. So it is the **screenshot path, not gameplay**; the swap-chain buffer
  stays the target in play. The arming write and the producer of `T+0x64` were not located (Gap 1). The `PostEffects` task also receives the target as `P[0x59]` (FUN_0096f360 first store).
* Third blur pass (underwater): FUN_00967ee0 pass k writes target set `B+0x28+(k&1)*4` and reads t0 = scene / B[3] / B[4]; pass 2 reads tex1 and writes tex0. The composite and HighPass get tex1 from
  (inferred dead output; the "SeparableBlur" object FUN_00921780 uses its own `sun_halo_*` textures).

## 9. For JC2-099 (non-underwater composite)

All 12 composite PS variants of Scene / DepthOfField / HeatHazeDOF with EdgeFade/FilmGrain (and the Underwater ones, 15 in total) end in exactly one `sqrt o0.xyz` (probe asserts the 12 non-stereo
ones again; JC2-094 asserts all 15). The non-underwater families read the same t0 (sRGB view of `back_buffer_copy`) and write the UNORM swap chain, so decode (SRV) -> grade -> one `sqrt` -> UNORM holds for them too.

## 10. Corrections to earlier briefs

1. JC2-094 section 7/8 wrote the vignette as `|uv - 0.5|`; the shader computes `length(uv - 0.5)` (`dp2` + `sqrt`). Also applies to the three other families.
2. JC2-094 section 1/8 called the base PS 7482432 "the maths" and EdgeFade the constructor default; the shipped gameplay shader is the EdgeFade variant (this brief, section 1); the film-grain shader runs only in cutscenes.
3. JC2-094 section 3 sampler enum meanings (point/linear/aniso, "inferred") are now decoded (section 5); s5/s6 are not "(1,0,3.0f,1)" in effect: the 3.0 is unused.
4. JC2-094 "object WaterFog selection untraced": section 7.

## 11. Gaps

1. `T+0x60` arming site and `T+0x64` producer (screenshot path only; does not affect gameplay output).
2. Driver of `CGameSystemManager` registration slot (+0x30) and proof that init is one forward pass over the vector: needed to upgrade H1 (Q[3] = water normal) from inferred to proven. Blocks the underwater distortion/caustic parity (a null Q[3] would remove both).
3. Whether any frame renders before the first hour-block application (`S(5) = 0.000002` file default) and the exact first-frame S values.
4. Film-grain SRV format (UNORM vs sRGB) and the LCG jitter ownership (`DAT_012a1450` writers).
5. The CUDA/bokeh path (S(0x1c), option 0x32, FUN_00950910 / FUN_0092a510 / FUN_0096f200) and the device-caps dword that gates it; not the shipped default.
6. Stereo `t9` and `cb0[1]/[2]` (unchanged from JC2-094).
8. Mip LOD of the t5/t6 lookups at the 0.1*uv scale is derivative-driven (section 5); the screen-space footprint at 1280x720 vs 1920x1080 was not computed.

## 12. Capture handoff additions (to JC2-094 section 11, not executed)

At FUN_009256d0 dump Q+0x68, Q+0x69, Q[3], S(0x1b) (P+0xb0) during gameplay and during a cutscene; record the PS name each time (expect `DepthOfField_EdgeFade`/`Underwater_EdgeFade` and
`*_EdgeFade_FilmGrain`). `PSGetSamplers` for s5/s6 should show MinLOD 0, MaxLOD FLT_MAX, filter 0x14, WRAP; s8 WRAP 0x14. Record the filmgrain SRV format.
