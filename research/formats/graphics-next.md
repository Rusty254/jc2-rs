# Graphics coding preparation: materials, foliage, water and LOD

Research checkpoint 2026-10-08. This is a handoff brief, not a claim of implemented
visual parity. No game build or runtime edit was performed. Code observations are
against the graphics-research branch starting at `4df0dd5`. Shader offsets below
are absolute byte offsets of the `DXBC` magic in the shipped
`DX10_Shaders_F.shader_bundle`; names alone are ambiguous because VS and PS share
names. Existing format foundations: [rbm.md](rbm.md), [coordinates.md](coordinates.md),
[terrain.md](terrain.md), [world.md](world.md). Weather/cloud shadows have their own
research handoff and are outside this brief.

## Implemented material foundation, 8 October 2026

`codex/graphics-materials` implements the bounded P0 loader changes below. This
section supersedes the historical loader deficiency description; the shader
evidence and later assignments remain applicable. Validation results are recorded
after the serialized release checks and captures, not inferred from this research.

- VegetationFoliage and BillboardFoliage use `Mask(0.5)`; Window and
  DeformableWindow retain Blend; bark and other solid families remain Opaque.
  The cutoff is a provisional silhouette repair: original fade scaling/gating is
  still absent. Existing two-sided rendering is retained pending typed culling flags.
- Valid tangent streams and finite UV1 reach Bevy meshes. Tangents are projected
  off the normal and normalized, preserving either authored sign. Invalid or
  degenerate bases withhold the stream and prevent normal-map hookup.
- General, CarPaint and VegetationBark bind their verified RGB normal slot when
  tangents/UV0 are valid. Empty/missing/undecodable maps retain geometric normals;
  diffuse-equals-normal is excluded case-insensitively. SkinnedGeneral and other
  unverified families await complete basis/variant handling. Properties/AOC,
  secondary bark/damage textures, roughness/metallic and culling remain unchanged.
- Normal images use Bevy 0.19.1 `NestedLoadBuilder::load_value` with
  `ImageLoaderSettings.is_srgb=false`, then become labeled RBM subassets. Repeated
  normal slots share one label within that RBM. Ordinary colour loads retain their
  independent path handle and settings. A deferred settings override alone is
  insufficient: Bevy asset infos are keyed by path/type, not settings. Independent
  models can still duplicate normal images; a shared typed view cache is later work.

**Handedness cross-check:** General VS **188280** outputs the tangent and a sign
through `lt/iadd/itof`; PS **6062008** uses that sign with `cross(N,T)`. DXBC true
comparisons are all-bit masks (`-1`), so the reconstructed sign equals the packed
float sign. CarPaint VS **47080** and Bark_Water VS use the same convention after
their basis component permutations. Bevy PBR 0.19.1 uses `B=tangent.w*cross(N,T)`;
no additional coordinate reflection or sign inversion is needed. A read-only
Arctic pine bark audit over 982 vertices gives mean dot **0.96776** between that
reconstructed B and the separately packed authored bitangent. A small subset of
the unused bitangent stream disagrees; the shader's reconstruction wins. Tangent
Gram-Schmidt projection handles quantization without changing sign.

Headless regression fixtures include Arctic pine bark/foliage and the bus body,
and explicitly load the bark normal DDS a second time as an sRGB colour image.
They check distinct handles, identical stored pixels with different format views,
masked foliage/opaque bark, preserved UV1 and basis agreement. Release validation on the existing branch (base `9880844`), isolation 36, with
`JC2_TEST_DIR` set to the unmodded Steam folder: **320 tests passed** (129 formats,
191 game), including seven new loader tests; release workspace clippy/all-targets,
release game build and `cargo fmt --all -- --check` passed. Shared builds were
serialized; the built executable was copied under the lock to
`target/jc2-graphics36.exe` and all capture processes exited.

Actual GPU captures inspected under ignored `target/graphics36/`:

- `pine-front.png` and `pine-back.png`: `--model
  vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm`, yaw 0/180, pitch 15,
  exit-after 8. Needles cut out from both sides; bark stays solid. Needles look
  sparse, consistent with the provisional 0.5 cutoff; this is not native parity.
- `bus.png`: assembled `lave.v046_buscoach.ee`, yaw 75, pitch 10, exit-after 8.
  CarPaint/body and separate glass render; this wide shot does not prove the
  normal-map relief magnitude or original glass appearance.
- `world-noon.png` / `world-afternoon.png`: tile 31,13, radius 0, vegetation LOD1,
  no hero/traffic/civilians/NPCs/music, sun-shadows on, fixed 12:00/16:00,
  `JC2_CAM_PITCH=-15`, exit-after 12. Both show all 4769 instances and changed
  sunlight/shadows. The distant foliage is too small for leaf-shadow parity.

The five saved screenshots establish main-pass rendering and a world shadow
smoke check. That checkpoint did not exercise a depth prepass or close foliage
shadows. The viewer now supports `--material-check` (matte shadow receiver),
`--depth-prepass`, `--no-normal-maps`, and `--light-yaw` (world-up rotation retaining
light elevation). All require `--model`; normal viewer defaults remain unchanged.
Nested archive paths now share the asset reader's canonical splitting rule.

`scripts/check-materials.ps1` takes a private executable, game directory and output
directory and captures ten pine/bus comparisons at 18 seconds each. It waits
for each owned process, records stdout/stderr and requires a screenshot and clean
exit. The viewer waits for labeled material assets before cloning a no-normal
comparison and logs source normal counts. Captures use the default shadow bias;
the diagnostic receiver does not cast shadows. Earlier experiments with bias 0.1
produced visible acne and are not the acceptance baseline. Original fade/cutoff
parity remains JC2-024. Captures stay
under ignored target directories, with no GPU timing or comprehensive validation-
layer claim.

### Integrated GPU acceptance, 8 October 2026

Isolation 10 passed all **360 real-data release tests** (143 formats + 217 game),
workspace/all-targets clippy with warnings denied, formatting and release workspace
build. The executable was copied under the lock to `target/jc2-main10-checked.exe`.
The ten captures in `target/material-acceptance10/` all exited cleanly, produced fresh
loaded-model screenshots and had empty stderr. All ten images were inspected.

Reproduce with `scripts/check-materials.ps1 -Executable ./target/jc2-main10-checked.exe
-GameDirectory <Steam-game-folder> -OutputDirectory ./target/material-acceptance10`.
Pine uses full `climate/zone_1/vegetation_1.blz` identity, yaw 25/205 and pitch 35;
bus body uses yaw 120/pitch 20. Key-light world yaw is 30/140 at constant elevation,
with fixed illumination and a matte receiver; each process runs 18 seconds.

- `pine-main` versus `pine-depth`: identical scene pixels below the 60-pixel HUD
  band. Front and back captures preserve needle cutouts and solid bark; receiver
  shadows have branch/needle gaps rather than opaque card silhouettes.
- `pine-depth`/`pine-light2-depth` versus `pine-flat-a/b`: removing the one bark
  normal map changes 13,236 / 10,609 scene pixels, maximum RGB channel delta
  66 / 73. The shape/silhouette remains stable; fine bark shading responds at both
  light angles.
- `bus-normal-a/b` versus `bus-flat-a/b`: removing the three CarPaint normal maps
  changes 81,829 / 145,620 scene pixels, maximum channel delta 13 / 16. Panel/vent
  shading changes at both angles; the separate window material still renders.

Pixel counts compare original 1280x720 RGB captures below row 60 without image
resampling. They demonstrate that the bound normal images affect rendering, not
that their strength matches retail. Combined with the original DDS/basis fixtures,
this completes the bounded material-foundation main/depth/shadow acceptance.
The provisional 0.5 foliage cutoff, original fade/alpha scaling, exact glass and
native roughness/metallic, wind, and other shader families remain separate work.

## Proven deficiencies and the first coding assignment

`crates/jc2-game/src/rbm_loader.rs::block_to_mesh` transfers position, normal, UV0,
joint index and joint weight. It does **not** transfer the parsed `MeshData.tangents`
or UV1. The loader reads only `material.diffuse`. Every non-window material is
Opaque; every block has no culling and `double_sided=true`. Roughness is 0.35 for
CarPaint and 0.7 otherwise, metallic 0.2/0.0. These are current approximations,
not decoded engine material properties.

P0 assignment: a type-aware alpha/material policy, followed by tangent transfer.
Own `rbm_loader.rs` and narrow mesh/material tests; coordinate with any other
agent editing this file. Use masked foliage as an explicitly documented first
approximation, retain window Blend, and leave opaque bark/solids alone. Carry
finite, normalized tangent xyz plus handedness into `Mesh::ATTRIBUTE_TANGENT`.
Coordinates here are identity/right-handed; do not add another Z reflection.
Validate tangent/normal orthogonality and sign instead of silently making every
tangent +1. Preserve UV1 now if adding the attribute is straightforward, but do
not wire AO or material channels until the shader binding is understood.

**Do not automatically bind all RBM normal slots.** The real Arctic foliage
sample below puts its diffuse texture into the normal slot too. A safe initial
policy binds known real normal textures for General/CarPaint/bark and suppresses
diffuse-equals-normal placeholders for foliage. Empty/missing maps must preserve
the old fallback material. The same path may need separate image settings for
sRGB colour and linear data. Prove settings and handle identity in the repository's
actual Bevy dependency before implementing; RGBA colour and normal data must not
share a wrongly sRGB-decoded image handle.

## Alpha testing: shader and texture evidence

Inventory tool correction: `shader_inventory.py` previously used `\bdiscard\b`
and missed `discard_nz`/`discard_z`, reporting false negatives. The corrected
expression recognizes the suffixes. Regenerating with filter
`foliage|skinned|water|terrain|general|bark` yields 298 records: 62 VS, 236 PS,
of which 146 PS contain a discard instruction. This count belongs to that exact
filter, not to all shaders in the bundle. The earlier 524-record inventory used a
different filter and must be regenerated before treating its booleans as evidence.

| Shader | Stage | DXBC offset | Proven instructions |
|---|---|---:|---|
| billboardfoliageprez | PS | 959304 | sample t0 at v1.xy; multiply alpha by v1.z and 4; discard if below cb4[4].x and cb4[3].w != 0 |
| billboardfoliageshadow | PS | 959900 | same alpha/gate/cutoff contract for shadow pass |
| vegetationfoliage | PS | 7510628 | reads front-face flag, uses t0 alpha and v10.w, gates discard with cb4[3].w and cutoff cb4[4].x |
| vegetationfoliage | VS | 320752 | procedural position perturbation before world/projection; see wind section |

For vegetationfoliage PS, near its end `dp2 r0.w, v10.wwww, r1.wwww`
computes `2 * v10.w * t0_alpha`. It then compares that output against cb4[4].x.
Billboard PS uses `4 * v1.z * t0_alpha`. Therefore **0.5 is not a proved original
threshold**: constants, interpolated fade, render state and possible alpha-to-
coverage must be traced before claiming parity. A hardcoded Mask(0.5) is suitable
only as a provisional visual repair, and may make these low-alpha needles sparse.
Do not choose Blend for all leaves simply because DDS alpha is continuous:
depth and shadow pixel shaders explicitly discard as well.

Texture measurements use mip zero decoded by Pillow 12.3.0; they do not determine
shader threshold or blend state. All names are members of
`climate/zone_1/vegetation_1.blz`:

| Asset | Size, format, mips | alpha=0 | alpha=255 | intermediate | alpha<128 |
|---|---|---:|---:|---:|---:|
| arctic_pine_foliage_dif.dds | 512² DXT5, 10 | 166616 | 354 | 95174 | 219350 |
| arctic_bushtree_foliage_dif.dds | 512² DXT5, 10 | 174737 | 576 | 86831 | 214998 |
| arctic_pine_bark_nrm.dds | 128² DXT1, 8 | 0 | 16384 | 0 | 0 |
| arctic_t22_foliage_aoc.dds | 128² DXT1, 8 | 0 | 16384 | 0 | 0 |

Pine diffuse SHA256:
`a4b3db4f3da9fc8c4cc92866f2d0ce88b1de07511f55687f67431dc7701be2ac`.
Alpha is materially present: 63.56% of its texels are exactly transparent. Opaque
leaf cards cannot reproduce this asset's intended silhouette.

`Arctic_T22_PineL_LOD1-Whole.rbm` has 2 blocks and 2851 triangles:
VegetationBark (1228) and VegetationFoliage (1623). Foliage diffuse **and normal**
are `arctic_pine_foliage_dif.dds`, properties is `arctic_t22_foliage_aoc.dds`.
Its header contains five floats `[0, 0, 1.6, 1.8, 1]` plus the exact flags word `0x40`; the old sixth float print reinterpreted that word. Float names remain unproved. See [ordinary foliage runtime evidence](foliage-alpha-runtime.md) for the recovered inverse-bit5 alpha enable and instance reference contract.
Bark diffuse/normal are `arctic_pine_bark_dif.dds`/`arctic_pine_bark_nrm.dds`,
and later slots contain `arctic_t22_trunk_dif.dds`/`arctic_t22_trunk_nrm.dds`.
The same material slot names are used by multiple block-specific contracts;
never infer that every properties slot is a metallic/roughness texture.

## Normal mapping, sidedness and skinned influence contracts

vegetationfoliage PS samples t1, subtracts 0.5 from RGB, projects through basis
interpolants v3/v4/v5, then blends the result towards v5 by cb1[0].y, normalizes,
and multiplies by +1/-1 based on SV_IsFrontFace. Thus two-sided normal handling
is proved for this leaf shader, but universal double-sided solids are not.
The repeated diffuse normal slot could be neutralized by that blend constant;
its actual CPU binding/value remains unknown. An `.nrm` filename alone also does
not prove swizzle, green-channel sign or DXT5nm encoding for every block family.
Trace the corresponding pixel shader, then capture relief on bark, vehicle
panels and the character at two sun directions. Do not guess mpm channels or
convert historical specular powers directly into PBR roughness without marking
the conversion approximate.

SkinnedGeneral parsing in `crates/jc2-formats/src/rbm.rs::parse_skinned_general`
currently consumes but discards packed tangent and bitangent u32 values. Big
vertices (v3 flag 0x80000) contain another eight bytes of weights/indices which
are skipped. Four weights are then normalized by `rbm_loader.rs`.

Original `skinnedgeneral8` VS at offset **270652** declares weight float4 inputs
v1/v2 and index groups v3/v4. It adds v1+v2, dp4s with ones, takes reciprocal,
normalizes BOTH groups by the combined sum, and accumulates both groups into the
bone transform using index*3 into cb1. `skinnedgeneral` VS at **267232** has just
one weight and one index group. This proves an original eight-influence path.
It does not prove which sample models materially use the second group.

P1 parser assignment: retain both groups and packed tangent/basis, report extra
weight occupancy and dropped-weight mass over Rico/soldier/civilian samples,
and measure pose difference. If Bevy's runtime path remains four influences,
select the top four across all eight then renormalize as a measured fallback;
blindly taking the first four is not equivalent. Validate batch-local remapping
and vertices shared across batches before adding another joint array. Full
eight-influence shader work belongs in a separate assignment after that audit.

## Foliage wind and billboard work

vegetationfoliage VS at 320752 first forms a phase scalar from local position
and cb1[0].x/w. Three phase rates (0.2, 0.14264, 0.25634) are wrapped with `frc`,
smoothed with polynomial operations, remapped with `*32-1`, scaled by cb1[0].y,
then multiplied by `min(length(local.xz), 3)` and added to local xyz. This is
proved procedural deformation; **time/wind interpretation of cb1 is inferred**
until its binding code is traced. It has no bone dependency.

P2 assignment: inspect the foliage render-block constant writers; preserve raw
vertex attributes and params first. Implement consistent perturbation in main,
shadow and depth passes with fixed-time captures. Acceptance: trunks/base stay
stable; crown motion is continuous; no shadow swimming or world-origin phase
explosions. Investigate BillboardFoliage centre+view-space offset separately:
parser currently expands offsets in model XY (rbm.md); do not conflate this
near/model block with the existing distant-tree impostor implementation.

## Water: exact gap and staging

Current sea in `crates/jc2-game/src/world.rs::setup` is a Plane3d at SEA_LEVEL=200
with StandardMaterial colour (0.05,0.25,0.45,0.75), roughness 0.15 and Blend.
It has no animation, depth-dependent colour, refraction or underwater treatment.
That surface is independent of boat physics and should remain so during graphics
changes. Original water is multipass: `watershader_lod0` PS at **7540448** has two
SV_Target outputs and texture slots 0,1,5,6,7; `waterhighend` PS is **7537032**;
`waterbumpcomposite` PS is **7535708**. NVIDIA variants have VS texture fetches,
while regular `watershader_lod0` VS at **339512** has none.

lod0 PS samples t0 at four scales of v1.xy (0.00425438, 0.0638157, 0.2,
0.0212719), applies y/x/z swizzle minus 0.5, combines vectors and normalizes.
Later it samples t7 in screen coordinates and reconstructs depth using cb1[1].x/y.
Texture binding identities and other cb values remain unknown; do not call t7
a specific engine asset. Multiple normal frequencies and scene-depth dependence
are proved; exact asset/animation/optical meaning is still a research task.

P2 water assignment: first inventory actual ocean textures and cb/resource
binding routines, then add a dedicated material with deterministic time, modest
normal animation and validated depth handling. Keep reflection/refraction capture
work separate from the initial material. Acceptance captures: flat noon ocean,
grazing sunset, shallow shore versus deep water, underwater crossing, a boat
crossing a tile boundary. Verify no terrain holes, alpha sorting flashes, or
physics sea-level drift. These are future checks, not completed tests.

## LOD and performance: extend the existing systems

`scenery.rs` already selects one configured vegetation LOD (default 2), uses
VisibilityRange 520..580m for trees with an impostor and 380..440m otherwise.
`impostors.rs` batches crowns/trunks into mesh pairs per 128m patch, builds tiles
asynchronously (IN_FLIGHT=2), default radius 5 tiles; fade is 450..600m. Do not
propose rebuilding these systems as if absent. Near models share cached handles.
`world.rs` resolves configured static-object LOD once; that is distinct from
camera-dependent switching among full-detail models.

Tile 31,13 reproduces 4769 procedural instances: 841 trees and 3928 bushes/rocks.
It is a useful alpha/overdraw stress fixture. P1 performance assignment: collect
CPU/GPU frame time, draw count, visible triangles, asset memory and load latency
at radius 0/1/2, vegetation LOD 1/2 and impostors on/off before changing policy.
Follow a fixed camera path through 380/440/450/520/580/600m boundaries and test
rapid tile unload/reload. Prefer hysteresis for later model LOD switching;
cache keys must include selected LOD. Separate alpha fill-rate from draw count:
shared meshes reduce asset load, but thousands of visible masked cards can still
cost GPU time. Backface culling changes require model-type visual validation.

## Reproduction and capture plan

From any worktree, set executable/game paths to the existing prebuilt tool and
game root (no Cargo needed). Example PowerShell research commands:

```powershell
$py='C:/Users/Rusty/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe'
$tool='C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/target/jc2tool-research.exe'
$game='C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2'
& $py research/tools/shader_inventory.py "$game/DX10_Shaders_F.shader_bundle" --filter 'foliage|skinned|water|terrain|general|bark' --out target/graphics-research/shaders.json --disasm-dir target/graphics-research/disasm
& $tool rbm $game 'climate/zone_1/vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm'
& $tool vegetation $game 31 13
& $py research/tools/dds_alpha_probe.py --tool $tool --game $game --name 'climate/zone_1/vegetation_1.blz/arctic_pine_foliage_dif.dds'
```

`dds_alpha_probe.py` decodes in memory and returns dimensions, alpha histogram,
counts and SHA256. It writes no assets. Pillow is the only extra dependency.
Do not commit DDS/DXBC/disassembly dumps; target outputs are ignored.

Future coding validation (requires built game and authorized GUI run): use model
viewer `--model container/member`, `--camera-yaw`/`--camera-pitch`, fixed time/day speed,
`--exit-after`/`--screenshot` for near foliage at both sides. World fixture 31,13
should hold `--time 12:00 --day-speed 0 --no-traffic --no-civilians --no-music`
and compare `--vegetation-lod 1` versus 2. Check silhouettes against bright sky,
leaf shadows on terrain, distance fade and camera rotations. For normal mapping
capture bark and bus panel closeups; for skinning capture crouch/run/hang with
weapon and aim combinations from animation research. Store before/after PNGs and
logs under ignored target directories, and record image settings/camera/sun/time.
Do not claim GPU correctness from a headless loader test alone.

## Dispatch summary

| Priority / owner | Dependency | Concrete acceptance |
|---|---|---|
| P0 RBM material policy | reserve rbm_loader.rs | leaf silhouettes/shadows cut out, windows still transparent, opaque bark remains solid; cutoff explicitly provisional |
| P0 tangent/normal hookup | coordinate loader edits; inspect Bevy image settings | finite signed basis reaches meshes, real normal maps linear, foliage placeholder excluded, two sun-angle captures |
| P1 skin influence audit | reserve rbm.rs; animation owner coordinates | extra4 occupancy/dropped mass report, batch remap verified, top-four fallback measured or full-eight planned |
| P1 LOD/performance audit | alpha fix capture baseline | metrics plus repeatable boundary path; existing impostor crossfade survives tile churn |
| P2 foliage wind | trace cb1 binding; alpha repair done | fixed-time main/depth/shadow consistency; base stable; view-space billboard tracked separately |
| P2 water material | trace texture/cb binding; reserve world sea setup | deterministic ocean/shallow/deep/underwater/boat captures; graphics sea level remains 200 |

Research helper validation completed: corrected inventory regenerated against
original bundle; original billboard depth/shadow records assert discard detection;
DDS alpha probe exercised on four original textures and a synthetic 1x1 RGBA DDS
(alpha 127, histogram sum and threshold count checked). No runtime, Cargo or GPU
validation was performed in this research assignment.

## JC2-251: model textures were clamped, not wrapped

User report: tree bark and the fallen log showed stretched/smeared bands, rocks looked smooth,
buildings looked wrong. Root cause: `rbm_loader` loaded every colour, normal and MPM image with
Bevy's default sampler (clamp-to-edge) although the retail meshes tile their textures beyond
0..1 (Arctic pine bark UV0 spans about -1.3..26.6 by -10..10.8). Every tile outside 0..1 repeated
the border texel. The loader now sets a wrap (repeat) sampler on those images (`wrap_sampler`);
the UV1 AO atlas keeps the default. Wrap is inferred from the authored UV range; the native
D3D10 sampler states for the Bark/Facade/General shaders are not decoded (gap: native
address/filter modes, anisotropy and mip filter; Bevy's default mip filter is nearest).
Regression: `retail_tiled_bark_and_normal_images_wrap` in `rbm_loader.rs`. Captures:
`_extract/JC2-251/{before,after}-pineL.png`, `-log.png`, `-hotel.png`, `-hero-31-14.png`.
The hotel facade barely changes (its UVs stay mostly inside 0..1); trunks, branches, logs and rocks change a lot.
