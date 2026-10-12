# Rico's eyes: why they look wrong, and what the native eye materials do (board 21ff361e)

Probe: `research/tools/rico_eyes_probe.py --game <game folder> [--tool <jc2tool>] [--print]` (41 checks, all pass; reads
`mc01_lod1-rico.rbm`, the eye / hair / clothes DDS files, `biped.bsk` through `jc2tool`, the shader bundle and
`JustCause2.exe`; pure Python, no GPU). Related: `rbm.md` (Rico's blocks), `point-light-model.md` section 10 (the eye
shaders take the point-light term before albedo and have no fog step), `parachute-canopy-shading.md` (the
`SkinnedGeneral` mpm channel roles and the shadow-pass tests), `water-next.md` / `water-consumers-next.md` (the scene
colour copy `ctx+0xcc4`), `ambient-hemisphere-next.md`. Reference shots: the user's screenshots
`images/18.png` (rewrite, sunset) and `images/15.webp` (original), copied to `_extract/21ff361e/ref/`.

## 1. Verdict

1. **The dark rings are not the eyes at all. They are Rico's eyelash cards drawn as solid black.** The hair blocks
   (14, 15) carry 168 triangles at the eyes whose UVs lie in the two eyelash strips at the bottom of `mc01_hair_dif.dds`;
   the texels there are black with partial alpha. The rewrite draws every `SkinnedGeneral` block through
   `hemisphere_main.wgsl` (`hemisphere_material::is_hemisphere_family` accepts `SkinnedGeneral*`), and that fragment
   shader calls `pbr_input_from_standard_material` but **never `alpha_discard`** (Bevy calls it in `pbr.wgsl`, which the
   extension replaces). So `AlphaMode::Mask` has no effect on any hemisphere-route material: hair pass 1 (block 14,
   `Mask(196/255)` by `character_state`) is drawn fully opaque, cards, lashes and all.
2. **Experimental proof (uncommitted, reverted):** adding the one line
   `input.material.base_color = alpha_discard(input.material, input.material.base_color)` to the fragment function makes
   the rings vanish, shows the hair as separate near-black strands with gaps (instead of a solid brown cap), and leaves
   the eyes as the narrow lid slit with sclera and iris seen through it. The same capture harness (`--rico-capture`),
   same spot and time, branch build, with and without the line: `_extract/21ff361e/before_face-front.png`,
   `experiment_hemisphere-alpha-discard_face-front.png`.
3. **The eyeball, iris and sclera are present and correct; the eye is a narrow slit because that is Rico's idle face.**
   The eyeball is in the head block (9), textured from the clothes atlas; the animation `std_idle.ban` carries
   facial tracks (lids, eyes, brows, jaw) that are all *static identity*, i.e. the bind pose is the idle face.
   `images/15.webp` shows the same slit.
4. **The cornea block (12) is a clear gloss shell**, not a visible eye part: with Rico's textures its pixel shader outputs
   the scene already behind it plus a faint reflection and a tight specular highlight. It never darkens or outlines
   anything. Hiding it changes the picture very little (captures `JC2_RICO_HIDE=12`).
5. What remains different from the native look after the fix is lighting and the cornea's gloss terms (section 5),
   and the two-pass blended hair (section 4), listed as the implementation task.

Evidence matrix (rewrite, `--rico-capture` face-front, pure-black pixels in the eye region of the crop; all with the
unfixed hemisphere shader unless noted):

| Variation | Result |
|---|---|
| hide block 14 / hide blocks 14+15 | rings gone / rings gone |
| hide block 15 only | ring unchanged (solid black) |
| block 14 `Mask(0.1)`, `Mask(0.77)` (default), `Mask(0.99)`, `Mask(2.0)` | identical solid ring (a cutoff of 2.0 must discard everything): the test is not running |
| block 14 as `Blend` | no ring; soft dark lashes |
| only block 14 visible, `Mask(2.0)` | the whole hair cap and the lash rings still drawn |
| cornea (block 12) hidden | rings unchanged; eyeball unchanged |
| depth prepass removed from the camera (`DepthPrepass`) | ring unchanged (not a prepass artefact) |
| the ring's pixels | exactly (0, 0, 0): black texels at full coverage |
| `alpha_discard` added to `hemisphere_main.wgsl` | rings gone, strands visible |

## 2. The geometry (native data, probe part a)

`mc01_lod1-rico.rbm` has 16 `SkinnedGeneral` blocks. Flags (header `u32`): clothes and shirt `0x100000`, head, neck, chest
skin `0x80000` (big vertices), eyes `4`, hair `0x10001` and `0x50001`.

| Part | Where | Facts |
|---|---|---|
| Eyeball (iris, sclera) | head block 9 | 372 vertices (186 per eye) with UVs in the eyeball tile of `mc01_clothes_dif.dds` (u 0.11-0.25, v 0.56-0.70, the round brown/green iris on the bloodshot white), 624 triangles; sphere fit radius 15.2 mm, centres (-0.0309, 1.6886, -0.0640) and (0.0315, 1.6890, -0.0641) (model space, front is -z); vertex normals radial (mean dot 0.994); weight 255 on batch-local bone 0 or 1 = skeleton bones 73 `EyeLeft` and 74 `EyeRight` |
| Cornea shell | eye block 12 | two closed shells of 175 vertices and 312 triangles; sphere radius 15.4 mm (0.2 mm over the eyeball), centre 0.1-1.2 mm in front of it; same bones 73 and 74; textures `ava_eye02_dif/nrm/mpm.dds` (64x64 white BC1, 128x128 BC1, 128x128 BC3); header floats (10, 1, 1, 1, 1, 1) like every block |
| Lids and skin | blocks 9-11 | bound with the lid and face bones (`EyeLeftLid` 75, `EyeRightLid` 76, brows, cheeks, jaw: the head batch's bone map is 73, 74, 67, 65, 66, 19, 30, 61, 62, 70, 69, 72, 64, 63, 75, 71, 76, 18); the face texture has the lid and lash line painted as a blurred grey shape around the opening |
| Lash cards | hair blocks 14 and 15 | the same 1787 vertices, 1820 triangles and UVs in both blocks; 168 triangles at the eyes (centroid |x| < 0.06, y 1.665-1.725, z < -0.07) with UV u 0.006-0.356, v 0.909-0.981: the two lash strips of `mc01_hair_dif.dds`; 140 of the 168 face +z (into the head), so a viewer in front sees their back sides: both sides must be drawn (cull code 1, low byte 1) |

The lash strips (rows 0.90-0.975, u < 0.37 of the 512x512 BC3 atlas): 61 % of the texels have alpha > 0, **1 % reach
alpha 196 (0.77)** at mip 0, none at mips 1-3 (the maximum alpha is 189, 157, 100); every texel with alpha > 0 is black
(each channel <= 5 of 255). A correct alpha test therefore leaves a sparse scatter of lash dots; a missing test fills the
whole card with black, which is the ring.

## 3. The native eye shader (probe part c)

`SkinnedGeneralEyeGloss` (pixel shader, 412 instructions; vertex shader 85) is the shader of block 12 (low byte 4).
Inputs: t0 diffuse, t1 normal, t2 mpm, t3 point-light grid, t6 the scene colour copy (`ctx+0xcc4`, the same copy the
water reads as t5), t7 an environment cube (`ctx+0xcc0`), t11 horizon shadow, t12 shadow map. The vertex shader skins with
the four bones, passes the clip-space `w` as `TEXCOORD1.z` (`o2.xyz = clip.xyw`) and the world normal; there is **no tangent
frame**.

Pixel colour, in order (all asserted in the probe):

1. `E` = sun term + hemisphere term + point lights, scaled by mpm.A (`t2.w`); `albedo = t0 * cb1[1]`; `lit = albedo * E`.
2. Refraction: `uv' = screenUV + (-n.x, n.y) * 0.001 / w`, with `n = 2 * t1.xy - 1`; `scene = t6(uv')`.
3. `colour = lerp(lit, scene, mpm.B)`: **Rico's eye mpm has B = 255 everywhere, so the lit diffuse is replaced by the scene
   behind the shell, shifted by at most 0.26 * 0.001 / w of the screen** (under 0.35 px at 1 m). The eye's own lighting
   (term 1) does not reach the output.
4. Reflection: `+ cube(reflect(-V, N)) * cb0[3] * f`, `f = min(0.1 * (1 - N.V)^3 + 0.003, 0.025)`: at most 2.5 %.
5. Specular: `+ cb0[1] * G * sat(|H.N| * 100 - 99) * G` with `G = mpm.G` (0..0.59 in this texture, so `G^2 <= 0.35`);
   the highlight only exists where `|H.N| > 0.99`.
6. Output alpha = `diffuse.a * cb1[1].w` (= 1 for the white texture). The alpha discard is `alpha * cb1[1].w < cb4[4].x`
   gated by `cb4[3].w`, as in every `SkinnedGeneral` shader. **No fog step** (no `cb0[13]` / `cb0[14]`).
   mpm.R is unused, mpm.G and mpm.B are never overwritten after the sample.

Textures: `ava_eye02_dif` is plain white (every texel 255); `ava_eye02_mpm` R 57-120, G 0-~150, B 255, A 255;
`ava_eye02_nrm` x, y within 63-190 of 255. The iris and sclera are **not** in the eye block's textures: they are the clothes
atlas tile used by the head block.

Consequence: natively the cornea is an invisible glass shell that adds a rare sharp highlight and a faint reflection.
Its output alpha is 1, so whether it is blended or opaque makes no difference to the result: this closes **Gap 2 of
`rbm.md` (the eye's blend factors)**. The rewrite's version (`StandardMaterial` black base, `AlphaMode::Add`, roughness
0.15, reflectance 0.5, drawn through the hemisphere shader) adds Bevy's PBR specular and Fresnel instead: a broad lobe and
a Fresnel that rises to 1 at grazing angles, where the native term stops at 0.025. It is a different (and brighter at the
silhouette) term, not a darkening one.

## 4. Native draw state of the character blocks (probe part d)

`CRenderBlockSkinnedGeneral` (vtable `0x00f22e74`): slot `+0x6c` = `0x00931080` (main state), `+0x70` = `0x009308a0`
(the alternate state callback of the depth and shadow submission), `+0x20` = `0x00969220` (a per-pass block predicate).
Flags are the block header `u32` at `+0x28`.

* **Cull** (`0x0095fe00`): code 1 (none) when the low byte is 1 or bit 16 is set, else 3 (back faces culled).
* **Blend** (`0x0091c270` bit 0, `0x00928670` bit 1 of the state word at `+0x9c`, layout from `0x00933530`: bit 0 enable,
  bit 1 alpha-to-coverage, bits 2/7/12/17 source, source-alpha, destination, destination-alpha factor): for **every**
  `SkinnedGeneral` main-pass block (clothes, skin, hair, eyes alike) blend enable = `[013e0e28+0x178] == 0` and
  alpha-to-coverage = `[013e0e28+0x178] != 0`. The byte has no writer in the decompiler export (inferred: zero, so blending
  is on). The default factors every block-type group starts from (`0x0096f820`: word = `(old & 0xffc26117) | 0x26114`,
  alpha-to-coverage cleared) decode to colour `SRC_ALPHA, INV_SRC_ALPHA`, alpha `ONE, ZERO`: the standard "over".
  Clothes and skin have alpha 1, so blending is invisible there; hair and lashes are blended by their texture alpha.
* **Alpha test** (`0x00949aa0`, slot 5): reference = `(2 * code + 1) / 510` with `code` = `0xc4` for a hair block without bit 18
  (reference **393/510 = 0.7706**), else 1 (reference **3/510 = 0.0059**: the bit-18 hair pass and every non-hair block;
  the rewrite's 196/255 and "none" are the rounded values). `0x00931080` skips a bit-18 block entirely in pass id `0x50`
  and, when the byte is non-zero, leaves the blend state of the bit-18 pass untouched (alpha-to-coverage inherited).
  The alpha shader discards below the reference when `cb4[3].w` is set.
* **Hair is two blended, alpha-tested passes**: pass 1 (flags `0x10001`, reference 0.77) draws the dense strand cores almost
  opaque, pass 2 (flags `0x50001`, reference 0.0059) adds every faint texel over it, soft. The lash strips are black texels
  of mostly low alpha, so natively the lashes are a faint, soft dark fringe plus a sparse darker scatter, never a solid
  band. The rewrite has pass 1 as `Mask` (opaque after the cut) and pass 2 as `Blend` with no test.
* **Eye branch** (low byte 4): sampler slot 6 clamp (3, 3, 3), filter codes (2, 2); slot 7 wrap (1, 1, 1), filter codes (4, 2);
  texture slot 6 = `[ctx+0xcc4]` (scene colour copy), slot 7 = `[ctx+0xcc0]` (cube); the eye shader pair
  `[013e0e28+0x144 / +0x148]` (small / big vertices); `0x009576b0(1)` sets bit 1 of the word at `+0x88` (the helper the decals
  call with "alpha == 1.0": meaning undecoded). Reference code 1 (no test).
* **Shadow and depth passes**: `0x009308a0` and `0x00969220` return at once for low byte 4: **the cornea never enters a depth
  or shadow pass**, and a bit-18 block does not either unless the byte is non-zero (hair pass 2 casts no shadow). The other
  hair block is tested in the depth variants with reference `(2 * round(a * 0.7686) + 1) / 510` (`a` = the instance alpha
  byte; pass `0x4f`).

## 5. What differs in the rewrite, ranked by visibility

1. **Hair and lash cards are opaque** (the missing `alpha_discard`, section 1): the black rings and a solid hair cap. Fix: one
   line in `hemisphere_main.wgsl` restores `Mask`; it also affects every other `Mask` material on the hemisphere route
   (`General` blocks with alpha test that stay on the fallback, `Facade`, ...): check them in the same change.
2. Hair pass 1 should be alpha-**blended** with the 0.7706 test and pass 2 with the 0.0059 test (section 4); the rewrite's
   `Mask` + unclipped `Blend` is an approximation with hard strand edges.
3. Cornea gloss: replace the PBR `Add` by the native two terms (section 3, 4-5), with the reference taken from the sun
   specular colour and the environment; no Fresnel to 1.
4. Eyeball and lid shading belong to the head block's `SkinnedGeneral` pixel shader (mpm R scales the diffuse light, G the
   specular, B the exponent; `parachute-canopy-shading.md`): the eyeball tile's mpm has R mean 0.47, so the native eyeball
   is half as bright as an unscaled PBR one, and the socket's painted AO (mpm R) is likewise missing. That is task
   `edb9fdda` (skin lighting), not repeated here.

## 6. Gaps (unresolved native detail, what it blocks)

1. **Hair pass depth and sort state, and the real value of `[013e0e28+0x178]`.** No writer was found in the export; the
   depth/stencil word (`0x009603c0` layout) the hair blocks run with, the draw order of the two hair passes against the cornea
   and the head, and whether the byte is ever set (alpha-to-coverage instead of blending) are not decoded. Blocks the exact
   hair/lash look (task `edb9fdda` items 3 and 4). Filed.
2. **Where `COPY_FRAMEBUFFER` (pass id 44) sits relative to the character draw.** The cornea reads that copy; the output is
   the eyeball only if the copy already contains it (inferred: it does, since the water reads the same copy for the opaque scene).
   If the copy predates the character, the eye would show the background. Visible effect on the pixel colour: none if inferred right.
3. Sampler filter and anisotropy codes (`0x009203e0`, `0x00970020`) and the meaning of the `+0x88` bit 1: cosmetic.
4. The eight undecoded bytes of big vertices (head, neck, chest): probably the facial rig (unchanged from `rbm.md`).

## 7. Implementation task (filed, low priority)

Rico eyes and lashes (board): add `alpha_discard` to `hemisphere_main.wgsl` and verify all hemisphere-route alpha users;
render hair pass 1 as blended + test 393/510 and pass 2 as blended + test 3/510 (native), or at minimum keep `Mask` working;
replace the cornea's PBR `Add` by the native reflection (`<= 0.025`) and tight specular terms; correct the constants in
`character_state` (`(2v+1)/510`); before/after captures with `--rico-capture` (face-front and face-side), noting that Rico's
facing in the harness varies from run to run (re-run until the front view). Do not touch skin/hair lighting (`edb9fdda`).
