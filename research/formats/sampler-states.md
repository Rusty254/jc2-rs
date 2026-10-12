# Native D3D10 sampler states of the model material families

Board: c728494e (textures blocky when magnified) and 5402e098 (JC2-257). Probe:
`research/tools/sampler_state_probe.py` (exit 0). Implementation: `crates/jc2-game/src/native_sampler.rs`.

## 1. Why the port looked blocky (proven)

`rbm_loader::wrap_sampler()` built `ImageSamplerDescriptor { address_mode_*: Repeat, ..default() }`.
In Bevy 0.19 `ImageFilterMode` defaults to `Nearest` and `ImageSamplerDescriptor::default()` leaves
mag, min and mip filters at `Nearest` with anisotropy 1 (probe, section 5). The loader applied this
sampler to every UV0 diffuse, normal and MPM image of every block family, so:

* magnification (boulder moss, banana leaf, ferns seen close) showed the individual texels: the
  "grid of blocks" of the six user screenshots (rewrite 19, 21, 22 against original 20, 23, 24);
* the DDS mip chains were present (Bevy loads them) but a NEAREST mip filter snaps to one level,
  popping and shimmering instead of blending;
* no anisotropy, so oblique surfaces (ground-hugging leaves, rock tops) blurred or aliased.

This is a sampler-state defect, not a missing mip chain, a clamped bias or a streaming choice.
Other samplers in the port (terrain layers, impostors, sky, forest canopy) already set Linear.

## 2. The packed sampler word (native evidence)

Pass setup code writes one 16-byte entry per texture slot at `ctx + 0x1c0 + slot * 0x10`:

| setter | writes |
|---|---|

`FUN_00940170` (device vtable `+0x158` = `CreateSamplerState`) decodes the word:

* if `f5 == 7` or `f6 == 7`: filter `0x15` (MIN_MAG_MIP_LINEAR), plus `0x40` (= `0x55` ANISOTROPIC)
  when the anisotropy field is above 1;
* otherwise `filter = ((LINEAR[f5] * 4 | LINEAR[f6]) * 4) | MIP_LINEAR[f5]` with `LINEAR = {0,0,1,1,1,0,0,1}`
  (table `0xf22858`) and `MIP_LINEAR = {0,0,0,1,0,1,0,1}` (table `0xf2285c`);
* MaxAnisotropy = bits 21-25 (min 1); comparison func = bits 9-12 (0 = off); MaxLOD = FLT_MAX when `f5 >= 3`,
  else 0 (fields 0 to 2 are single-level samplers).

The decoded pairs: (7,7) trilinear / anisotropic; (3,2) trilinear (0x15); (4,2) 0x14 linear with a POINT
mip filter (the cube sampler already documented in `general-terms-next.md` section 1); (2,2) bilinear base
level only; (1,1) point base level; (5,2) point min with linear mag/mip.

The constructor of the pass context (`text_00920000.c`, near the `0x49` / `0xee000` constants) gives the
first slots the default word: WRAP in U, V, W, fields (7,7), anisotropy `ctx + 0xccc` (a byte). That byte is
written by `FUN_0092cde0`, vtable slot 22 (+0x58) of five context classes. The command line documents
`/aniso=n  Anisotropic filter (0-7, default: 7)`; settings value 0x16 is stored by `FUN_005b5530`.

## 3. Per-family states (the main-pass setup of each block type)

The main pass function is vtable slot `+0x30` of each `CRenderBlockType*` vftable (probe checks the bytes).
`S` = anisotropy from `ctx + 0xccc`, `1` = literal 1. All addresses WRAP unless stated.

| family (rbm type) | function | slots (fields; anisotropy) |
|---|---|---|
| General | `0096eed0` | 0..3 (7,7) S; 4 = environment cube CLAMP (4,2) 1 |
| VegetationBark | `0095c890` | 0, 1 (7,7) S; 2, 3 (3,2) 1 (the dirt diffuse and dirt normal at UV1) |
| VegetationFoliage | `00971da0` | 0, 1 (7,7) S; 2 (4,2) 1 (the `*_aoc.dds`) |
| Facade | `0093dac0` | 0 and the loop slot (7,7) S; 4 CLAMP (4,2); 10 (1,1) |
| Merged | `00928d30` | 0, 2 (7,7) S with MipLODBias -1.0; 4 CLAMP (4,2) bias -1.0; 10 (1,1); 11, 14, 15 (2,2) |
| SkinnedGeneral | `0093f2e0` | 0, 1, 2 (7,7) S; 6 CLAMP (2,2); 7 (4,2) |
| BillboardFoliage | `0093deb0` | 0 (3,2) 1 (and a (4,2) variant) |
| CarPaint, Lambert, DecalSimple | `0093b800`, `00937ec0`, `0096bc90` | (7,7) S |
| Window / DeformableWindow | `00959ff0`, `0095a2d0` | 0 (3,2); 4 (2,2); 5 (4,2) |
| Grass | `00a821d0` | 0 CLAMP U/V (7,7) S |
| Leaves (tree leaf cards) | `00a83550` | 0 (3,2) 1 |
| Forest | `00a7d5c0` | 0..2 (3,2) 1; fin `00a7dad0` (5,2) CLAMP |
| Terrain | `00a570d0` / `00a579d0` | per-slot from a data table / (3,2) CLAMP |
| Particle | `00a54d40` | 0 (4,2) CLAMP; 1, 2 (1,1) |
| Clouds, sky, post | various | (4,2), (3,2), (1,1) as listed in the probe's family dictionary |

So every model family that carries a diffuse or normal map samples it trilinear or anisotropic; none uses
point filtering for those slots.

## 4. Implementation (the port)

`native_sampler.rs` encodes the decode and the family table with unit tests; `rbm_loader.rs` uses it per
slot: diffuse by family (`BillboardFoliage`, `Window`, `DeformableWindow` trilinear, all others anisotropic),
normal / MPM / General AO anisotropic, VegetationFoliage AO wrap with a linear min/mag and POINT mip (as the
native slot 2). Terrain, impostor, sky and forest samplers already used Linear and are unchanged.

Frame-time and capture results: see the board comment of c728494e.

## 5. Gaps (unresolved native detail)

1. The mapping of the `/aniso=n` setting (0-7) to the byte `ctx + 0xccc` was not found (vtable slot 22 setter
   `FUN_0092cde0` has only vtable callers). The port uses the raw default 7 as MaxAnisotropy (INFERRED).
   Blocks: exact anisotropy level; visible only at grazing angles.
2. MipLODBias -1.0 on the Merged family (slots 0, 2, 4) cannot live in a wgpu sampler; it needs a shader-side
   `textureSampleBias`. Not applied. Blocks: Merged-geometry texture sharpness parity.
3. The CarPaint family passes `max(ctx + 0xccc, block + 0x688)` as anisotropy (`FUN_0093b800`, `FUN_00948650`,
   `FUN_00953da0`); the per-block byte at `+0x688` is not decoded, so CarPaint may use a higher level than the
   user setting (the port uses the setting).
4. (closed in this change) The bark dirt slots 2 and 3 now use the native WRAP (3,2) trilinear sampler
   (`load_dirt_image`).
