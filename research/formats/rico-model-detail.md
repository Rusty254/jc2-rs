# Why Rico looked low-resolution: LOD, textures, sampler, normal and property maps (board 2280444f)

Probe: `research/tools/rico_detail_probe.py --game <game folder> [--tool <jc2tool>] [--print]` (14 checks, exit 0;
reads the Rico `.lod`, `mc01_lod1-rico.rbm`, the 12 DDS files; pure Python). Related: `rbm.md` (Rico's blocks),
`rico-eyes.md`, `sampler-states.md` (the filter fix of c728494e), `parachute-canopy-shading.md` (the native
`SkinnedGeneral` pixel shader: t1 tangent-space normal, t2 properties).

## 1. Verdict

| Suspect | Finding |
|---|---|
| LOD choice | **Not a cause.** `mc01-rico.lod` lists `mc01_lod1-rico.rbm` for all five LOD levels (then a trailing 1) and the archive holds no other Rico model. The one mesh has 12,801 vertices and 16,064 triangles in 16 blocks: there is no lod0 to select. |
| Texture resolution / format | **Not a cause.** Clothes and shirt diffuse/normal 1024x1024 BC1, shirt mpm and hair diffuse/mpm 512x512, hair normal 256x256, eye 64x64 / 128x128; every file has a complete mip chain (probe). About 560 texels per metre of height. |
| Sampler (blocky, shimmering) | **Partly the cause, already fixed** by c728494e (`sampler-states.md`): the port sampled these images NEAREST with anisotropy 1; native `SkinnedGeneral` slots 0 to 2 are WRAP trilinear/anisotropic. |
| Normal map | **The remaining cause.** The port loaded a normal map only for `General`, `CarPaint`, `VegetationBark` and the double-sided canopy; Rico's blocks (flags `0x100000`, `0x80000`, `0x10001`...) did not qualify (`normal_texture_name`), and the `SkinnedGeneral` vertex reader discarded the tangent and bitangent. All 1024x1024 `mc01_clothes_nrm` / `mc01_shirt_nrm` detail (folds, seams, pockets, skin pores) was therefore missing: flat vertex-normal shading of a 16 k triangle mesh. |
| Property (mpm) map | **Applied (board edb9fdda, section 5).** |

## 2. Native evidence

* The native `SkinnedGeneral` pixel shader samples t0 diffuse, t1 normal decoded `2 * rgb - 1` on the interpolated
  tangent frame, t2 properties (`parachute-canopy-shading.md`; the sampler slots 0, 1, 2 of `FUN_0093f2e0`). The
  cornea (low byte 4) is the one variant without a tangent frame (`rico-eyes.md` section 3).
  Inferred: the clothes, skin and hair blocks use the same pixel shader as the canopy; the main-pass state
  `0x00931080` only varies cull, alpha test and blend by the flags (`rbm.md`), and every block names its own
  `*_nrm` texture.
* Rico's vertices store the tangent frame: each 20-byte stream-1 vertex is `u32 normal, u32 tangent, u32 bitangent,
  f32 u, f32 v` (bytes x, y, z are `2b/255 - 1`). The probe finds the tangent orthogonal to the normal (mean
  |cos| 0.002 to 0.007) and unit length (0.996), and the bitangent parallel to `N x T` (|cos| above 0.95) with
  either sign, so the handedness is the sign of `dot(N x T, B)` (mirrored UV islands: 0 to 100 % left-handed per block).
* The normal maps are tangent-space, blue-dominant (mean 244/255 clothes, 248 shirt, 255 hair) and near unit length.

## 3. Implementation (this branch)

* `jc2-formats` `parse_skinned_general` now keeps the tangent and the handedness sign.
* `rbm_loader::normal_texture_name` accepts the character blocks (everything but the eye gloss cornea).
* Captures: `_extract/2280444f/` (before: tangent-less, flat; after: normal-mapped), see the board comment.

## 4. Gaps (unresolved native detail)

1. The mpm channel roles for the character blocks are proven only through the canopy's data flow; Rico's own
   lighting terms (R scales sun/hemisphere, G specular, B exponent) are not applied (skin/hair lighting, `edb9fdda`).
   Blocks: Rico's specular and the painted AO darkening of the sockets.
2. Whether the hair blocks sample the normal map with the same tangent decode (assumed: same shader).
3. Instance-dependent terms (`cb1[1]` colour, the alpha byte) are unchanged.

## 5. Native lit colour for Rico (board edb9fdda)

Rico's clothes, skin, hair and eyeball-tile blocks now run the `SkinnedGeneral` lit-colour formula of
`parachute-canopy-shading.md` (`skinned_general_material.rs`, `skinned_general_main.wgsl`): mpm R scales sun, hemisphere
ambient and point lights and gates the rim, G scales the specular, B sets the exponent `32 B + 4`, rim with the constructor
constants, native haze. Hair is near-black again (its mpm R is low) and the harsh brow/nose highlights of the Bevy PBR
look are gone.
* Native-proven (canopy research): the formula and the three mpm roles. The height ramp `s = sat(6 - 3.3333 y)` with `y` the
  raw model-space vertex height is applied to both Rico and the canopy harness (UV1.x carries `y`; the SkinnedGeneral vertex
  has no second UV set).
* Inferred: that Rico's blocks, hair included, use the same pixel shader as the canopy (the flags only change cull, alpha
  test, blend state); instance constants at the native defaults; LOD scale 1; no underwater term; the cornea (low byte 4)
  keeps its separate route.
* The `JC2_RICO_LEGACY` switch is removed; `JC2_NO_RICO_NATIVE_LIGHT=1` restores the old route for before captures.
* Remaining gaps: the hair PS may be a different variant (`1spec`..`4spec`, hair specular gating, `rbm.md`); the light-count
  variants are not selected per draw; the eyebrow highlights are still bright (mpm G or the hair texture; unverified).
