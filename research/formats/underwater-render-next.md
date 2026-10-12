# Underwater postprocess: selection, bindings, constants and colour chain (JC2-094)

9 October 2026. Research only: no Cargo, GPU, game launch, runtime edit or asset copy. Extends
[water-next.md](water-next.md) and the JC2-095 brief (`water-consumers-next.md`, branch
`worktree-agent-aa2fd9fbbe77bdc10`), whose clock, fog-layer, reflection-camera and pass-44/49 findings are reused
and not repeated. Terrain/cloud findings of `cloud-shadow-consumers-next.md` (branch
`worktree-agent-ab610af73978831af`) are reused for the pixel `cb0` upload.

**Update (JC2-103, `post-composite-variant-next.md`):** the shipped gameplay variant is the `_EdgeFade` family (film grain only in
cutscenes), the vignette is radial `length(uv - 0.5)` (corrects section 7/8), the sampler enums and the writers of Q+0x68/0x69,
setting[0x1b] and Q[3] are resolved there.

Executable `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`
(14,549,656 bytes). `DX10_Shaders_F.shader_bundle` sha256
`db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`. Addresses are retail VAs; shader offsets index
the bundle. Evidence: the ignored Ghidra export, raw exe bytes, and re-disassembled DXBC. Nothing proprietary is
committed. There is no other fixture input: no textures, saves or captures were read.

Reproduce (read-only, prints one JSON document):

```
python -B research/tools/underwater_render_probe.py --boundaries-only        # pure models, 181 cases, no game needed
python -B research/tools/underwater_render_probe.py --game <game folder> \
       [--pseudo <game>/jc2-rs/research/analysis/pseudo/_unnamed]
```

Full mode asserts both hashes, 7 image constants, 21 byte patterns at the call sites that carry the argument/ECX
proofs, 123 export fragments in 24 function bodies (two with ordering checks), the instruction strings of 13 shaders,
the single final `sqrt o0.xyz` of all 15 composite PS variants, the register sets read by the six Underwater variants, and prints the sha256 of the bytes of 45 traced functions and of
13 shader blobs. Verified: both modes exit 0.

Legend. **Proven**: read from the binary and pinned by the probe. **Inferred**: follows from proven facts, one link not
machine-checked. **Unresolved**: not traced; no value is substituted anywhere in this brief.

## 0. The primary chain in one page (all proven unless marked)

_(decompiler excerpt omitted from the public snapshot)_

T = the render-thread object (`this` of `FUN_0096d400`), P = `CRenderEngine[0x31]` (`*(DAT_01191ab4+0xc4)`, the
post-effects manager; also the ECX of `FUN_009390e0`, `FUN_009373b0`), Q = `P[0xc]` (the composite object, 0x6c
bytes, ctor `FUN_009370b0`, init `FUN_00939320`).

## 1. Camera-crossing selection (proven; probe `selection_row`, `boundary_cases`)

| Consumer | Predicate | Camera used | y = 250 | y = 200.0 exactly | y = 199.99998 | y = 150 |
|---|---|---|---|---|---|---|
| Post composite shader family (`FUN_009256d0`, `FUN_009390e0`, bytes `a1f41919018b50...` at 0x9390e0) | `setting[0x12] > camera.y` (`comiss`, `jbe` -> 0) | global current camera = `*(DAT_011919f4+0x14)`, Y at +0x58 | Scene/DOF | **Scene/DOF** | Underwater | Underwater |
| WaterBelow VS 333312 / PS 7532292 instead of the ocean LOD (`FUN_00a809c0`, `comiss` at 0xa80a20, `jb`) | `ctx+0xc9c < 200.0` strict | context camera (`ctx+0xc68` copy of camera world matrix, row 3 Y) | ocean | **ocean** | WaterBelow | WaterBelow |
| WaterBelow PS fog mask; object WaterFog VS mask | `cb0[5].y < 200` (PS), `cb0[4].y < 200` (VS) literal in the shader | cb0 camera position (global current camera) | off | **off** | on | on |
| Ocean density `cb0[14].w` (JC2-095) | `200.0 <= y` -> 0.3 else 0.15 | | 0.3 | **0.3** | 0.15 | 0.15 |
| Ocean distortion `cb1[1].zw` (JC2-095) | `200.0 <= y` -> (0.24,0.045) else (0.8,0.1) | | above | **above** | below | below |
| Reflection mirror branch (JC2-095) | `y > 200` | | mirror | **other branch** | other | other |

* `setting[0x12]` is the float at `P+0x44+0x12*4 = P+0x8c`. It is written with the image dword at 0xEB5AA4 /
  0xEB23AC = `0x43480000` = 200.0 by `FUN_00812c50` (engine init, string "post_effects") and `FUN_005907b0`, both through
  buoyancy, swim or oxygen code is touched or modelled here.
* **At y == 200.0 exactly every water consumer above treats the camera as above water** (post, WaterBelow, both fog
  masks, density, distortion). Only the JC2-095 reflection camera branch differs (`y <= 200` leaves the mirror
  branch). That inherited inconsistency is not resolved here. y compared are float32: 200.0 and the neighbours
  200.00001526 / 199.99998474 are fixtures.
* The post predicate and the two fog masks read the **global current camera**, the WaterBelow selection reads the
  **context camera** (the mirrored camera during the reflection pass). They agree except while the reflection pass
  runs. The predicate is evaluated once per frame at step 1 (ctx+0xccd bit 0) and again inside the post functions
  (`FUN_009256d0`, `FUN_00967ee0`, `FUN_00954f20`, `FUN_00973360`), all with the same inputs.
* Shader-variant handle choice inside `FUN_009256d0`: `p = (Q+0x69 == 0 || setting[0x1b] <= 0.01) ? (Q+0x68 != 0) : 2`;
  underwater handle = `Q[0xe + p (+3 if DAT_013d9b97 stereo)]`; otherwise `setting[5] <= 0` -> Scene `Q[0xb+p]`,
  `setting[0x1a] <= 0.001` (compared as a double) -> DepthOfField `Q[8+p]`, else HeatHazeDOF `Q[0x14+p]`. Names come
  from `FUN_00939320`. `FUN_009370b0` initialises `Q+0x68 = Q+0x69 = 1`, so **the constructor-default family is
  `Underwater_EdgeFade` (PS 7485260), becoming `_FilmGrain` when `setting[0x1b] > 0.01`**; the base PS 7482432 needs the
  edge-fade flag cleared. Who later writes the two flags and `setting[0x1b]` (graphics options 0x2e/0x34 are set from the
  `/edgefade` and `/filmgrain` command line) was not traced; see section 9.

## 2. Framebuffer ownership and the colour chain (proven; the policy conclusion is stated in section 6)

| Object | Created by | Format / role |
|---|---|---|
| `depth_buffer_copy` | `FUN_0092ca30` | 0x29 R32_FLOAT; refreshed by pass 44 and again by pass 49 (`FUN_0092ca30` configures both with the depth-copy flag); with ascending-id execution (inferred, JC2-095) it then contains the water surface seen from below |

Colour accounting (each step is applied once):

1. Scene passes write **linear** values through the sRGB RTV (hardware encode on store). WaterBelow, the ocean and the
   object WaterFog shaders emit linear colour and contain no `sqrt` (probe: WaterBelow PS tail `mov o0.xyz` /
   `mov o1.xyz`).
2. The post pass reads `back_buffer_copy` through an sRGB SRV (hardware decode: linear), blends/grades in linear space,
   and ends with exactly one `sqrt o0.xyz` (probe: all 15 composite PS variants, i.e. DepthOfField, HeatHazeDOF, Scene and
   Underwater, each with the _EdgeFade / _FilmGrain / _Stereo combinations present in the bundle). The destination is the UNORM swap-chain buffer, so nothing
   encodes after the `sqrt`.
3. When post effects are off, step 4 of the frame resolves the scene target straight into the back buffer (byte copy of
   the sRGB-encoded bytes, **inferred** from `FUN_0094ce20` flags; no decode or `sqrt`).
4. `sqrt(x)` is gamma 2.0, not the sRGB curve. Difference `sqrt(x) - srgb(x)`: 0.01 -> +0.0001, 0.05 -> -0.0242,
   0.2 -> -0.0373, 0.5 -> -0.0283, 0.8 -> -0.0119, equal at 0 and 1 (probe). A port that uses a true sRGB
   output would be up to 0.037 (about 9/255) off; a port that keeps a linear target must write `sqrt` exactly once and
   must not also encode.

## 3. Underwater PS t0..t6 producer identities (proven; none inferred from slot numbers)

the stray 4th pushed argument at some call sites is ignored by the callee). Byte patterns for every call are
asserted. Variant PS texture lists were read from the DXBC: base/_EdgeFade t0..t6; _FilmGrain adds t8; _Stereo adds t9.

| Slot | Bound value in `FUN_009256d0` | Identity | Producer chain | Format / size |
|---|---|---|---|---|
| t3 | `Q[0]` | adapted-luminance ring texture `*(P[0]+0x148+ring*4)` captured **once** at init (`FUN_009531d0`), i.e. `A[0x52+idx0]` | luminance object `FUN_009432c0` (1x1 `tone_mapping_downsampling` textures); ring toggled by the adaptation pass `FUN_0093e660`, which runs after the composite. Whether Q[0] is stale on alternate frames is inferred | 0x29 R32_FLOAT, 1x1, sampled at (0.5,0.5) |
| t4 | `Q[2]` | `P[10][1]` = `small_temp_texture[0]` (`P[0x4f]`) | HighPass/bloom object `FUN_0094d520` ("HighPass"), `FUN_00953060` | 0x1d, w/4 x h/4 |
| t6 | `Q[3]` | same texture as t5 | same | same |
| t8 (FilmGrain variants) | `Q[4]` shared_ptr (`vtbl+4(ctx,8)`) | `filmgrain.dds` loaded by `FUN_00939320` into the shared_ptr at `Q+0x10` (bytes `8d4e10 e8` asserted) | | not inspected |
| t9 (Stereo variants) | not found in `FUN_009256d0` | **unresolved** | | |

Consequences: t5/t6 are the same generated texture used two ways: RG as a screen-space distortion offset, alpha as the
caustic pattern (the ocean uses the same alpha at the same 0.08/0.3 scales, JC2-095). The ocean's `water_bump.dds`
composite pass therefore feeds the underwater look; its clock is the JC2-095 phase.
Both initialisers are AppSystem slot-2 methods (`CRenderEnginePostInitProxy` slot 2 = `FUN_0082fdf0` -> `FUN_0091cb90`
-> `FUN_009531d0`; `CLandscapeManager` slot 2 = `FUN_00a74f80` -> `FUN_00a8b220`); their relative order was not
established. **Hypotheses**: H1 `Q[3] = water_normal_texture` (assumed below); H2 `Q[3] = 0`, a null SRV, giving
n = (0,0) -> uv offset (-0.01,-0.01) and caustic alpha 0 (gain 1). Section 8 item 1 decides it.

s0..s4 `(1,0,0,1)`, s5,s6 `(1,0,3.0f,1)`. Reading them as D3D10 CLAMP / WRAP and point / linear / anisotropic is
**inferred** (enum names not decoded); wrap on t5/t6 and point on the R32F t2/t3 is consistent with the shader.

## 4. Fullscreen quad and vertex shader (proven)

VS 121340 (`depthoffield`, 360 B) passes `o0.xy = v0.xy`, `o0.zw = (1, 1)`, `o1 = v1`. The quad is
(-1, 1, 0, 0, 0, 0), (1, 1, 1, 0, 0.1, 0), (-1, -1, 0, 1, 0, 0.1), (1, -1, 1, 1, 0.1, 0.1); (0.1 = `0x3dcccccd`).
So `v1.xy` = screen uv with v = 0 at the top, and **`v1.zw = 0.1 * uv`**, the coordinate of the t5 distortion lookup
(the first 10% of the normal texture, fixed to the screen; the texture content itself animates).

## 5. Constants (proven: sources and arithmetic; settings VALUES unresolved)


| Reg | Value | Used by the Underwater PS as |
|---|---|---|
| cb1[0] | (S(0x1a)*0.004, 0, S(4), S(0x19)) | `blend = sat(ramp * w + z)`: .z = S(4), .w = S(0x19) (x unused here) |
| cb1[1] | (S(0xb), S(0xc), S(0xd), 0) | RGB multiplier |
| cb1[2] | (S(7) - 1, 0, 1/near - 1/far, 1/far) | exposure offset; depth reconstruction `.z .w` |
| cb1[3] | (S(2), S(3), rnd_a, rnd_b) | overlay strengths x (screen branch) and y (multiply branch); z,w = film-grain UV offset (LCG `s = s*0x343fd + 0x269ec3` twice from `DAT_012a1450`, `((s>>16)&0x7fff)/32767`; z from the second state, w from the first) |
| cb1[4] | (S(0xf), S(0x10), min(S(0xe),S(0x11)), max(...)) | saturation factor `clamp(lum*y + x, z, w)` |
| cb1[5] | S(5) <= 0 ? (0,0,0,0) : (S(5), 1/S(5), S(6), S(6) > 0 ? 1/(0.5*S(6)) : 0) | focus ramp: `sat((eye - x)*y) + sat((z - eye)*w)` |
| cb1[6..9] | rows of `S x inverse(ViewProj)` (section 6) | clip -> world |
| cb0[5], cb0[6] | (camera +0x54, +0x58, +0x5c, 1), (camera +0x44, +0x48, +0x4c, 1) from `FUN_0095f020` (`_DAT_013db190`, `_DAT_013db1a0`; cb0 base `0x13db140`) | camera position; `cb0[6].y` = row +0x44 Y = **minus** forward Y (the projection has `m[11] = -1`, so the camera looks down -z; text_00610000 negates the same row for the forward vector), positive when looking down |

If `S(5) <= 0` the ramp terms are 0 and `blend = sat(S(4))`. Writers of the settings are untraced as a table; observed
sites (all gameplay/cutscene/UI code owned elsewhere, not touched): `FUN_005bf690` (S(4) from the air-supply timer),
`FUN_005c5b10` (S(0x19) = 1, S(5) = 200 or 450, S(4) = 0.5), the cutscene state machine `FUN_00554b00` (S(4), S(0xb..0x10)
with smoothing `P+0xf0` and lock bytes via `FUN_00937310`), `FUN_0088cbf0` (S(0x1b) = 1.0). **Defaults of the table
(P+0x44..) are unresolved.**

## 6. Depth reconstruction and world position (proven; probe numbers)

  near=+0x624, far=+0x628)` (m10 = f/(n-f), m11 = -1, m14 = m10*n), then `FUN_00944070` (called from the pass draw
  `FUN_009737a0`, `FUN_00942f60`, `FUN_0097a3d0`) replaces **m10 = n/(f-n), m14 = f*n/(f-n)**. Hence `d = n(f - t)/((f - n) t)`:
  d = 1 at eye distance t = near, d = 0 at t = far.
* The PS: `eye = 1 / (d*cb1[2].z + cb1[2].w)` with `z = 1/near - 1/far`, `w = 1/far`. Exact inverse of the line above
  (probe: round trip within 1e-9 relative for t in 0.1..39999). Defaults for illustration only: near 0.1
  (`0x3dcccccd`, camera-manager init), far 40000 (`0x471c4000`): z = 9.999975, w = 2.5e-5; d(t) = 1.0 (0.1), 0.2 (0.5),
  0.0999978 (1), 0.00999753 (10), 9.75e-5 (1000), 0 (40000). float32 storage of z,w gives <= 3.6e-8 relative error.
  The constants do not depend on the camera Y (fixtures 250, 200, 199.99998, 150).
  (-1, 1, 0): [u,v,d,1] -> [2u-1, 1-2v, d, 1]. `ViewProj` is camera `+0xe4` = view(+0x2e4) x proj(+0x2a4) in
  4x4 inverse. Probe: project / reconstruct round trip < 1e-9 for five cameras including y = 200.0 and 199.99998, clip
  w = eye distance, view z = -eye.
* The PS samples `t2` and the colour textures at the **perturbed** uv (`uv + 0.01*(t5.rg*2 - 1)`), and reconstructs the world
  position from the perturbed uv and that depth.
* `depth_buffer_copy` comes from the pass-44/49 copies, so the post depth includes the water surface seen from below
  (surface pixels have world y = 200 -> caustic gate 0).

## 7. Underwater PS 7482432 transcription, overlay and caustic boundaries (proven; probe `underwater_ps`)

```
uv'   = uv + 0.01 * (t5(0.1*uv).rg * 2 - 1)                           eye = 1/(t2(uv').x*cb1[2].z + cb1[2].w)
ramp  = sat((cb1[5].z - eye)*cb1[5].w) + sat((eye - cb1[5].x)*cb1[5].y)
m     = sat(ramp*cb1[0].w + cb1[0].z);   C = t0 + m*(t1 - t0)                       (linear, per channel)
b     = t4(uv');  C = b < 0.5 ? 2*C*(cb1[3].y*(b-0.5)+0.5) : 1 - (2-2*C)*(1-(cb1[3].x*(b-0.5)+0.5))   (overlay)
lum   = dot(C, (0.2125, 0.7154, 0.0721));  k = clamp(lum*cb1[4].y + cb1[4].x, cb1[4].z, cb1[4].w);  C = lum + k*(C - lum)
C    *= (t3(0.5,0.5).x + cb1[2].x) * cb1[1].xyz
world = reconstruct(uv', t2(uv'));   D = min(1/(0.07*|cb0[5].xyz - world|^2), 0.4);  Sg = sat(200 - world.y)
B     = min(2*max(cb0[6].y + 0.2, 0.1), 1);   uvc = (world.x - 0.25 world.y, world.z - 0.25 world.y)
A2    = 2*(t6(0.08*uvc).a + 0.5*t6(0.3*uvc).a);   C *= 1 + B*D*Sg*A2^3
out   = (sqrt(C), 1)
```

Boundary fixtures (probe, all asserted):

| Quantity | Case -> value |
|---|---|
| `Sg` (200 is a literal in the shader, independent of `setting[0x12]`) | world y 250 -> 0, 200.00001 -> 0, **200.0 -> 0**, 199.5 -> 0.5, 199.0 -> 1, 100 -> 1 |
| `D` cap radius | `sqrt(1/(0.07*0.4))` = 5.976143 m; distance 0 -> 0.4, 5 m -> 0.4, 10 m -> 0.142857, 100 m -> 0.0014286 |
| `B` | cb0[6].y -1 -> 0.2, -0.1 -> 0.2, 0 -> 0.4, 0.3 -> 1, 0.8 -> 1, 1 -> 1 |
| gain | `A2 = 0` -> exactly 1; cubic in A2; `A2 = 2*(a08 + 0.5*a30)` |
| overlay | b = 0.5 is the identity; ks = kp = 0 leaves C unchanged on both branches; b < 0.5 -> `2*C*(kp*(b-0.5)+0.5)` |
| camera cases | y = 250, 200.00001, **200.0**: Underwater PS does not run (Scene/DOF family); y = 199.99998 and 150: runs; canonical pixel (uv 0.5,0.5, eye 20 m, pitch -0.7) world y = y - 12.88; eye = 20 |

Illustrative pixel (probe `end_to_end_canonical_pixel`, made-up texture/cb values, so only the arithmetic path is
claimed): camera y 199.99998, eye 20, `D = 0.0357`, `Sg = 1`, `B = 1`, `A2 = 0.9` -> gain 1.026, linear
(0.1909, 0.2630, 0.3415), encoded by `sqrt` (0.4370, 0.5128, 0.5844).

## 8. Variants: verified deltas, unknown enabling state

| PS (bundle offset, bytes) | Delta vs base PS 7482432 (DXBC diff, asserted) | State |
|---|---|---|
| `underwater_edgefade` 7485260 (3012) | before the `sqrt`: `C *= 1 - sat((|uv - 0.5| - 0.35) * 2.2)` (vignette on the linear value) | delta verified; this is the constructor-default family (section 1) |
| `underwater_edgefade_filmgrain` 7488304 (3192) | adds `t8`/`s8`: `C = C*vignette + (t8(uv*(16,8) + cb1[3].zw).x - 0.5)*0.018` before `sqrt` | delta verified; `setting[0x1b] > 0.01` selects it; texture content/size **not inspected** |
| `underwater_stereo` 7498308 (3068), `_edgefade_stereo` 7495016, `_edgefade_filmgrain_stereo` 7491536 | adds `t9`/`s9`: two `sample_l` at (0,0) and (0.125,0), `cb0[1].w`, `cb0[2].w`, shifts a UV component by `0.5*(...)`; reads cb0[1], cb0[2] | selected when `DAT_013d9b97` (NVIDIA 3D Vision) is on; **t9 producer and cb0[1]/[2] meaning unresolved** |
| godrays (`watergodraysshader`, pass 67) | not part of this pass | **unresolved** (JC2-095 section 8) |

No variant reads `cb0[13]` or `cb0[14]`: probe asserts each Underwater shader reads only cb0[5], cb0[6] (stereo adds
cb0[1], cb0[2]).

## 9. Fog accounting and ordering (exactly-once)

| Effect | Where applied | Gate |
|---|---|---|
| Atmospheric fog (fog layer 0, `cb0[13]`) | each scene shader (e.g. `merged` PS tail `mad o0, r0, r0.w, cb0[13]`) | always |
| Water fog (fog layer 1, `cb0[14]`: colour, density 0.15 below / 0.3 above, JC2-095) on the ocean surface from below | WaterBelow PS 7532292: `colour += (cb0[14].rgb - colour) * max(1 - 2^(-dist*cb0[14].w), 0)` masked by `cb0[5].y < 200` | y < 200 |
| Water fog on opaque objects | `Merged*WaterFog` VS/PS variants (`FUN_009598c0` builds the 8 handles): VS `o2.z = (cb0[4].y < 200) ? max(1 - 2^(-dist*cb0[15].w), 0) : 0`, `o2.w = 200.3 + 0.001*r0.w - r0.y` (r0.w is the VS distance-like temporary used by the exp term); PS `lerp(colour, cb0[14].rgb, sat(v2.z))` unless `v2.w < 0` | camera below 200 **and** fragment below ~200.3 |
| Post Underwater composite | desaturate/exposure/overlay/caustics, **no fog term** | y < 200 (section 1) |

So each pixel receives layer-0 fog once and layer-1 fog at most once, in the scene pass that shades it; the post pass adds
no fog. The conversion pair (sRGB decode at the SRV, `sqrt` at the end) is also once per frame. Not traced: the CPU
choice of the `*WaterFog` handle (index +4) for object draws (the shader gates make an above-water selection harmless),
the VS-side register `cb0[15].w`, and terrain/foliage/skinned water terms (JC2-084 owns terrain).
Ordering relative to the crossing: WaterBelow is drawn in the pass chain **before** the composite and uses the context
camera; the post flag uses the global camera; on a frame where the camera moves across 200 both are evaluated from the
same frame's camera, so a given frame uses WaterBelow + Underwater together or the ocean + Scene/DOF together
(inferred: the context camera equals the global camera except in the reflection pass).

## 10. Verified, inferred, unresolved

**Verified**: sections 1 (all predicates, setting writers, at-200 behaviour), 2 (formats, target sets, single `sqrt`),
3 (t0..t6 identities and call bytes; `Q[3]` store path), 4 (quad, uv*0.1), 5 (cb1 sources and arithmetic), 6 (reverse-Z,
constants, world reconstruction), 7 (shader transcription), 8 deltas of EdgeFade/FilmGrain/Stereo, 9 shader fog terms.
**Inferred**: `back_buffer_copy`-to-backbuffer byte copy when post is off; sampler enum meanings; Q[0] staleness; H1;
WaterBelow/Underwater coherence outside the reflection pass; the `cb0[6].y` sign meaning (needs `m[11] = -1` and row
orientation, both read, but not captured).
**Unresolved**: the defaults and runtime writers of the P settings table (S(2..0x1b)), `P+0xf0`, `Q+0x68/0x69`
writers (options 0x2e/0x34), `setting[0x1b]`; whether `Q[3]` is set (H1/H2); `T+0x64`/`T+0x60` output override; `P[9]`
linear-source path; consumer of the underwater 3rd blur pass; t9 and cb0[1]/[2] (stereo); `filmgrain.dds` content;
the object WaterFog handle selection and `cb0[15].w`; sampler filter enums; MSAA-off resolve; godrays.

## 11. Bounded runtime / capture handoff (not executed here; falsifiable, no gameplay writes)

One frame above water and one below (camera y = 150) in the same location, plus three camera stops at 200.00001,
200.0 and 199.99998 (teleport/free camera only, no swimming or sea-level change). At `FUN_009256d0` (0x9256d0):

1. Dump Q (ECX): Q[0..5]; expect Q[3] == water owner+0x34 (0x1c, 256x256, 9 mips). Null decides H2.
2. `PSGetShaderResources(0..6)` on the draw: expect formats t0 0x1d, t1 0x1d (w/2), t2 R32F, t3 R32F 1x1, t4 0x1d (w/4),
   t5 = t6 = 0x1c; record sizes and the SRV formats.
3. cb1[0..9] and cb0[5], cb0[6] bytes; compare with section 5 and with `underwater_render_probe.py` (feed the
   dumped near/far and ViewProj to `pixel_constants`).
4. P+0x44..P+0xb3 (28 settings) and P+0xf0, Q+0x68/Q+0x69, P+0x24/P+0x25; selects the real variant and values.
5. Output RTV: DXGI 0x1c and the swap-chain resource; compare pixels against `sqrt` of the linear intermediate.
6. At the three crossing stops record: Underwater vs Scene/DOF PS, WaterBelow vs ocean, `setting[0x12]`.
7. Sampler descriptors s0..s8.
8. Q[0] texture identity on two consecutive frames (ring staleness).

## 12. Implementation contract for the parent (JC2-013 below-surface stage; not implemented here)

Supported variant S2: non-stereo, post effects on, the base PS 7482432 maths with the edge-fade vignette available;
anything else keeps the current approximation and logs the gate. Selection `camera_y < 200` strict (single predicate
used for post, WaterBelow and the fog masks; keep `y == 200` above water). Bind t0..t6 exactly as in section 3 (one
generated normal texture for t5/t6, UNORM data view; colour textures through sRGB decode; depth as reverse-Z `d`).
Build cb1[2] and cb1[6..9] from near/far and ViewProj as in section 6. Write `sqrt(C)` once to a non-sRGB target, or output
`C` to an sRGB target only with an explicit documented deviation (<= 0.037). Do not add post fog, and do not touch
sea level, buoyancy, swimming, oxygen or drowning. Parity cannot be claimed until sections 10/11 settings, H1/H2 and
the default variant are captured.

## 13. Corrections to water-next.md

1. t0..t6 of the Underwater post-process are now identified (section 3); "CPU binding identities unknown" is closed for
   the non-stereo variants.
2. The `sqrt` is the single final encode into a UNORM swap-chain buffer after an sRGB decode at t0, not a free-floating
   approximation; the scene itself is rendered through an sRGB view of a typeless texture.
3. The Underwater selection is `200.0 > camera.y` on the global camera (strict), not a WaterBelow side effect.
