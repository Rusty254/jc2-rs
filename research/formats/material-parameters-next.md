# Material parameters: decoded contracts and next coding work

8 October 2026. Extends `graphics-next.md` and follows
`notes/RESEARCH-HANDOFFS.md`. Research only: no Cargo, tests or game launch.
Evidence: original DXBC bundle, retail RBMs, existing Ghidra export under
`research/analysis/pseudo/_unnamed/`, and current parser. Addresses below are
retail executable virtual addresses; shader offsets are bundle byte offsets.
Decompiled signatures often omit implicit ECX/arguments: verify assembly before
implementing native hooks. Findings concern the named shader variant, not all variants.

## Major decoded result: General header flags have names

`FUN_00935840` imports named material parameters into CRenderBlockGeneral. This
provides explicit names, not texture-name guesses. Its `FUN_00973da0` binary reader
maps v3's 0x4c-byte header into object offsets +0x34..+0x7f. Current `rbm.rs`
`parse_general` stores this as `params`, including integer fields cast to floats.

| General params index | Original object offset | Confirmed meaning |
|---|---|---|
| 0..3 | +0x34..+0x40 | ChannelTextureMask float4 |
| 4..6 | +0x44..+0x4c | ChannelAOMask float3 |
| 7 | +0x50 | Channel AO factor; importer sets 1 when UseChannelAO |
| 8 | +0x54 | DepthOffset; imported integer multiplied by 1e-5 |
| 9 | +0x58 | SpecularPower |
| 10 | +0x5c | packed/small vertex selector; not material flags |
| 11..16 | +0x60..+0x74 | geometry/UV/colour packing extents; `FUN_0092e450` names _CompiledPackPosExtent, _CompiledPackUV0Extent, _CompiledPackUV1Extent, _CompiledPackColorExtent |
| 17 | +0x78 | packed four-byte _CompiledPackColor; not an f32 scalar |
| 18 | +0x7c | render-block flags (u32) |

General flags decoded by explicit string comparisons in `FUN_00935840`:

| Bit | Meaning and consumer |
|---|---|
| 0x1 | NoCulling; NoCullingWithAlpha = 3, NoCullingWithAdditiveAlpha = 7 |
| 0x2 | WithAlpha; `FUN_0094d010` returns inverse of this bit for an opaque classification |
| 0x4 | additive alpha variant; importer WithAdditiveAlpha sets 6 |
| 0x20 | UseChannelTextures; changes selected channel in `FUN_00949b50` |
| 0x40 | UseSnow |
| 0x80 | AnimateTexture; contributes draw-sort key in `FUN_00972b20` |
| 0x100 | AlphaTest; changes draw key and depth path (`FUN_00970930`) |
| 0x200 | UseChannelAO; ChannelAOMask contributes variant selection |

**Separate field:** the u32 after the eight material texture strings, currently
`Material.flags` (usually 3), is read by `FUN_0094c710` through another virtual
setter. It is not General's +0x7c bitset. Its meaning remains unresolved; applying
the table above to every material trailer would break ordinary opaque geometry.
Preserve actual integer bits in a typed header rather than roundtrip through f32.

## Per-block texture and shader channel contracts

The names MPM and AOC are not interchangeable PBR channel specifications.
Main-pass mappings below follow actual instruction data flow.

| Family / retail fixture | Verified texture role/channel | Consumers and limits |
|---|---|---|
| General; `general.blz/wea04_lod1-grapplinghook.rbm`, block0 | t0 hook_diff.dds RGBA, multiplied by vertex COLOR; t1 hook_nrm.dds RGB normal decoded 2*RGB-1; t2 hook_mpm.dds R controls environment reflection with view-angle factor, G direct specular strength, B remaps ambient+sun lighting as `L + B*(2-L)` (a lerp toward the constant 2); t3 hook_dummy.dds RGB UV1 AO/color multiplier with optional ChannelAOMask dot product | General PS **6062008**; `FUN_00972ce0` binds material slots 0/1/2/3 and uploads cb2 channel masks/power. B is an emissive-like lighting remap, not metalness. Alpha channel of t2 was not consumed in this variant. Retail fixture is General v3, 146 vertices/138 triangles, SpecularPower=120 and render flags=0. |
| CarPaint; `lave.v014_jeep.ee/v014_lod1-cover.rbm` | t0 diffuse, t1 RGB normal 2*RGB-1, t2 MPM; damage variants sample additional diffuse/normal/properties slots | CarPaint PS **973444**, binary read `FUN_00955ef0`, type vtable 0x00f2382c. Full MPM channel roles and 13 parameter names remain **unknown** in this pass; do not copy General/Skinned mappings. Retail cover params include two RGB triples [0.187,0.146,0.065], index6=10, flags index13=1024. |
| SkinnedGeneral; `characters/maincharacters/rico/mc01_lod1-rico.rbm`, block0 | t0 mc01_clothes_dif.dds, t1 mc01_clothes_nrm.dds, t2 mc01_clothes_mpm.dds: B -> base exponent `32*B+4` (additional shader term can add up to 64); R scales diffuse ambient/sun and specular terms; G scales a separately coloured view/normal lighting contribution | SkinnedGeneral PS **6753928**, normal t1 RGB 2*RGB-1, frontface flip. G is a secondary appearance contribution, not proven metalness/roughness. Exact authoring names still unknown. Block0 is v3, 997 vertices/1452 triangles; eye block12 and hair blocks14/15 provide separate alpha/variant fixtures. |
| VegetationBark; `vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm`, block0 | primary diffuse/normal are arctic_pine_bark_dif/nrm; t0 diffuse, t1 RGB normal; later slots contain arctic_t22_trunk_dif/nrm for secondary bark representation | Bark_Water PS **946732**, type setup `FUN_0095c890`, binary read **0x00966840**. No MPM semantics in this shader; slot named properties can hold a diffuse texture. No universal AOC channel assignment established for bark variants. |
| VegetationFoliage; same fixture block1 | t0 arctic_pine_foliage_dif RGBA; t1 repeats diffuse (!); t2 arctic_t22_foliage_aoc at UV1: **R** multiplies ambient terms and affects a nonlinear colour adjustment; GBA were not consumed in this named shader | VegetationFoliage PS **7510628**, type constant writer **0x00971da0**, read **0x0095a8a0**. t1 RGB-0.5 transformed by basis and blended toward geometric normal using cb1[0].y; t1 alpha enters an additional lighting weight. Suppress guessed normal hookup until blend value is resolved. |

General PS precise equations (suppressing fog/local lights for clarity):

```
C = diffuse * vertex_colour * selected_UV1_AO
L = ambient_plus_sun + MPM.b * (2 - ambient_plus_sun) + point_lights
reflection = MPM.r * (1 - saturate(dot(N,V))^8) * sampled_environment * lighting_colour
spec = pow(max(dot(N,H), .001), SpecularPower) * shadow * MPM.g * other_mask
result = C*L + reflection + spec
```

**JC2-085 correction:** PS6062008 computes `add r8.xyz, neg(r4.xyzx),
l(2,2,2,0)` followed by `mad r4.xyz, r2.zzzz, r8.xyzx, r4.xyzx`.
This is interpolation toward the constant2, not toward `2-L`. For L=.5/B=.5,
the original yields1.25; the previous brief's expression yielded1.0. With B=1,
the original yields2 regardless of L before the separate point-light addition.

These are legacy forward-lighting contracts. A PBR roughness approximation needs
documented conversion and controlled comparisons; the original exponent is not
already perceptual roughness. Skinned exponent and General SpecularPower differ.

## Constant-buffer linkage and alpha parity

General `FUN_00972ce0` uploads cb2[0].x from +0x58 (SpecularPower), .y = 1 for
UseChannelTextures, cb2[1] from +0x44 (AO mask/factor), and cb2[2] from +0x34
(texture mask). This matches General PS channel extraction at its beginning and
the exponent near its end. `FUN_00949b50` encodes active mask channels into variant
selection; preserve all masks rather than choose only one texture convention.

Foliage `FUN_00971da0` uploads VS cb1[0] using type-global parameters and writes
cb1[1] from render-context values d2c/d30/ca8. cb1[1].x/y appear as specular
amplitude/power in the PS, but stage/overlapping state buffers must be verified
before declaring both uploads identical. Model params[0..5] are not simply copied
into those six PS constants. Their exact per-instance consumers remain unresolved.

foliage main and depth setup `FUN_00971da0` / `FUN_0092bf30` pass 0x80, yielding
0.5. The continuation below confirms the cb4 backing value through its final
contiguous upload and initial binding. Main foliage
tests **2*fade*diffuse_alpha**, billboard depth/shadow **4*fade*diffuse_alpha**.
Thus even a recovered 0.5 reference does not justify plain diffuse-alpha Mask(0.5).
The gate cb4[3].w also needs matching render-state enablement.

Main/depth/shadow milestone must share texture choice, UVs, animated deformation,
fade multiplier, alpha reference and enabled state. General has separate opaque /
alpha-tested depth paths selected by 0x100; 0x2 alpha blending is distinct. Bark
opaque solidity and foliage two-sided normals must remain type-specific. Inspect
CarPaint shadow-alpha and Skinned hair/eye variants independently.

## Colour-space evidence and unresolved rules

Normal and packed-channel values must remain numerically linear to reproduce the
decoded shader math: 0.5-neutral normals and channel dot-products cannot survive
unconditional sRGB decode. This is a numeric requirement, not proof of the retail
loader's exact SRV policy for every DDS. General/CarPaint shaders do not themselves
contain universal diffuse gamma conversion. Legacy DDS FourCC alone has no sRGB
tag; separate resource-view creation decides hardware conversion.

Concrete trace entry points: `FUN_00950420` creates textures and then
`FUN_0091c630` builds the SRV. `FUN_00968b80` groups typed formats into typeless
families. Water reflection is requested as 0x1d, generated water normal as 0x1c:
those are respectively RGBA8_UNORM_SRGB and RGBA8_UNORM. Enum meanings verified
against [Microsoft DXGI_FORMAT](https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format).
This proves mixed colour/data formats exist, not that all material diffuse maps use
sRGB. Trace the texture manager load slot +0x28 and per-slot view +0x14, retaining
view identity. Do not infer common material trailer 3 is a gamma flag.

## Typed schema, ownership and first milestones

Suggested schema: `GeneralHeader { channel_texture_mask:[f32;4],
channel_ao_mask:[f32;3], channel_ao_factor:f32, depth_offset:f32,
specular_power:f32, packing:..., packed_colour:u32, render_flags:u32 }`.
Keep `Material.texture_flags_raw` separate; unknown bits preserved. Other families
need separate headers and binding contracts. Asset cache key includes colour/data
view policy; one DDS name may need multiple settings and handles.

1. Decode General typed header/flags with fixture diagnostics; no renderer rewrite.
   Verify named import offsets against binary bytes and do not lose high integer bits.
2. Wire per-family material policy, correct culling/blend/alpha-test distinctions,
   tangent/UV1 and proven normal data. Treat foliage duplicate normal as unresolved.
3. Add General MPM/AO custom-material math with optional fallback; then separate
   Skinned/CarPaint/bark/foliage implementations after their gaps are resolved.
4. Controlled fixtures: isolated General prop with synthetic R/G/B channel toggles,
   jeep cover, Rico skin, Arctic pine bark/foliage. Capture fixed camera/sun/time from
   front/back, main/depth/shadow, transparent silhouette and channel-disabled cases.
   Numeric test: MPM R alters reflection, G highlight amplitude, B lighting remap;
   normal neutral texel yields geometric normal; AO mask selects documented channel.
   CPU tests cannot establish GPU parity.

## Reproduction

Use a private copy of the prebuilt tool (this pass copied it into TEMP as
`jc2tool-material-water-research.exe`), never build during research. `rbm` prints
the retail texture/parameter fixtures above. Read the named decompiler functions
from text_00930000.c / text_00940000.c / text_00970000.c. For shaders, invoke
`shader_inventory.inventory` and `sm4dis.disasm` in memory on the original bundle
with exact-name filter `^general$|^carpaint$|^vegetationfoliage$|^skinnedgeneral$`.
Python `-B` suppresses cache writes. Existing ignored `target/render-research`
disassemblies also cover foliage/bark/skinned. Raw DDS/DXBC/decompiler dumps must
stay uncommitted. No native consumer or GPU validation was run here.

Exact PowerShell reproduction from `jc2-rs` (tool is a private copy; all shader
output stays under ignored `target`):

```powershell
$game='C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2'
$tool=Join-Path $env:TEMP 'jc2tool-material-water-research.exe'
$py='C:/Users/Rusty/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe'
& $tool rbm $game 'general.blz/wea04_lod1-grapplinghook.rbm'
& $tool rbm $game 'characters/maincharacters/rico/mc01_lod1-rico.rbm'
& $tool rbm $game 'lave.v014_jeep.ee/v014_lod1-cover.rbm'
& $tool rbm $game 'climate/zone_1/vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm'
& $py -B research/tools/shader_inventory.py "$game/DX10_Shaders_F.shader_bundle" --filter '^general$|^carpaint$|^vegetationfoliage$|^skinnedgeneral$|^bark_water$' --out target/render-research/material-shaders.json --disasm-dir target/render-research/material-disasm
```

## JC2-018 continuation: final alpha upload and CarPaint damage

Research continuation on 8 October 2026. Independently reread the named original
consumers and regenerated ten narrowly selected shaders from the retail bundle.
This section supersedes the earlier provisional cb4 linkage and unresolved
CarPaint channel summary for **PS 973444 only**. No runtime edits, Cargo builds,
game launches or GPU parity validation belong to this continuation.

### Alpha reference and enable now reach the final buffer

| Original consumer | Confirmed static linkage |
|---|---|
| `00949aa0` | For comparison 7, writes `(2*reference-1)/510` to `013d9e30`; marks dirty byte `013d9db1` when changed. Reference 128 yields exactly 0.5. |
| `00952cc0` | Writes integer boolean enable to `013d9e2c` and marks the same dirty byte. This is not a floating-point 1.0: shaders use integer `ine` against zero. |
| `00922930` | Dirty-buffer path uploads 80 bytes starting at `013d9df0` into buffer `013d9db8`, then clears dirty. Two sibling paths in `text_00920000.c` repeat this upload. |
| `00938020` | Zeroes that 80-byte storage, creates the buffer and binds it through pixel-stage virtual slot `+0x28` with arguments `(4,1,buffer)`. |
| `0095dc20` | Copies render-state implicit ECX `+0xc98 != 0` into the same alpha-enable global. The final foliage-specific state owner is still unresolved. |

Relative offsets prove `013d9e2c - 013d9df0 = 0x3c = cb4[3].w` and
`013d9e30 - 013d9df0 = 0x40 = cb4[4].x`. Main/depth/shadow setup functions
`00971da0`, `0092bf30`, `0094e6e0` all call the comparison-7 setter with 128.
`0095fe00(context,1)` in the main setup changes another packed render-state
field; it must not be renamed the alpha-enable setter.

| Named pass and original PS offset | Alpha presented to the shared discard |
|---|---|
| vegetationfoliage main **7510628** | `2 * texture_alpha * v10.w` |
| vegetationfoliageprez **7521584** | `2 * texture_alpha * v1.z` (`dp2` of repeated alpha/fade) |
| vegetationfoliageshadow **7522144** | `2 * texture_alpha * v1.z` |
| billboardfoliageprez **959304** | `4 * texture_alpha * v1.z` |
| billboardfoliageshadow **959900** | `4 * texture_alpha * v1.z` |

Each listed PS discards when **enable != 0 and scaled_alpha < reference**.
Equality survives. Vegetation VS **320752** writes
`o10.w = (1-saturate(distance*cb1[1].y-cb1[1].x))*cb2[16].x`.
Thus, conditional on enable, reference 0.5 and fade 1, the effective texture
cutoff is 0.25 for vegetation and 0.125 for billboard; the existing plain
Mask(0.5) is still an approximation. This is algebra, not a recovered final
instance configuration. Keep enable, reference and instance fade explicit in
the next shared main/depth/shadow implementation.

**Foliage instance writer, now linked by the retail PE vtable:** `009672a0`
writes VS buffer slot 2, register 16, x from instance byte `+0x0b / 255`, then
passes `round(fraction*128)` to the comparison-7 reference setter. It obtains a
visibility scalar through `0096a8b0` and skips its draw when that scalar is at
most 0.5019608. The decompiler declares the latter return as int, while callers
consume an x87 floating result; do not copy that signature into a typed API.
`009672a0` references shader selectors under `013e0ee0` and object flags `+0x3c`.
The coordinator independently found its sole four-byte pointer occurrence in
retail `JustCause2.exe` at file offset **0x00b221c0**, mapped by PE sections to
VA **0x00f23bc0**. `research/analysis/classes.csv` identifies
**NGraphicsEngine::CRenderBlockFoliage** at vtable **0x00f23b4c**; the pointer is
its **+0x74** slot, before the next Font vtable at 0x00f23bd4. This proves the
foliage render-block writer ownership, separately from the shared foliage type
object vtable 0x00f2327c. The instance-byte `+0x0b` authoring/writer and final
enable-state initialization remain unresolved. Register matching alone was not
used to infer ownership; do not conflate the block and type vtables.

### CarPaint has separate RGB damage weights and alpha lighting

Verified directly in the beginning and final lighting instructions of CarPaint
PS **973444**; `v5.w` is the interpolated damage control. Slots t3/t4/t5 use
`v1.zw`, independently of primary t0/t1/t2 at `v1.xy`.

```
D = v5.w
damage_weights = (saturate(2*D), saturate(2*D), saturate(1.5*D))
diffuse_rgb *= lerp(1, t3.rgb, damage_weights.y)
tangent_normal = (2*t1.rgb-1) + (2*t4.rgb-1)*damage_weights.z
tangent_normal.z *= 1 - 0.8*damage_weights.z
MPM.rgb *= lerp((1,1,1), t5.rgb, damage_weights)
if MPM.a <= 0: MPM.rgb = (0,1,1)
environment_strength = saturate(1.5*MPM.r)
alpha_lighting = (1-MPM.a)*cb1[5].x + v4.w
```

The normal is subsequently transformed by the basis and normalized, with a
front-face sign. MPM **G** scales direct specular strength and **B** scales the
diffuse lighting contribution; **R** supplies the environment-strength factor.
The final alpha-derived lighting coefficient participates in later shading,
including snow suppression. MPM alpha is not discarded alpha: the earlier
shared alpha-test samples diffuse alpha from t0. Damage t3 changes diffuse RGB,
and t5 changes MPM RGB; neither multiplication above modifies their primary
alpha channels. These roles do not decode every parameter or the complete
lighting equation and must not be relabeled metallic/roughness/AO.

This narrows the CarPaint custom-material milestone: preserve both UV sets and
primary/damage slots; test damage 0/0.5/1 independently from MPM alpha 0/positive,
and verify shared silhouette handling in all passes. Final damage-control
CPU-to-VS upload, CarPaint parameter names, other named variants and shadow
variant selection remain follow-ups.

### Colour-space policy: explicit view format, unresolved DDS ownership

`0091c630` uses its explicit format parameter when nonzero; otherwise it uses
the underlying resource descriptor's format. `00950420` forwards the original
requested format into ordinary SRV creation, with separate depth-format handling.
Neither function inspected here applies a universal diffuse-to-sRGB conversion.
This confirms resource/view format ownership, not the caller's material policy.
The per-slot texture-manager/load path and DDS-to-format mapping still need a
specific diffuse/normal/MPM fixture trace. Preserve explicit colour/data view
identity; do not infer gamma policy from common material trailer value 3, texture
filename or the render-block vtable identity.

### Reproducible static verification

`research/tools/material_contract_probe.py` reads the original bundle in memory
and checks shader offset/name/stage identities, the five gated discard contracts,
VS fade linkage, key CarPaint damage/fallback instructions, original alpha setters,
final upload/bind fragments, byte-offset arithmetic and explicit SRV format
fallback. It writes no assets or disassemblies. Run from `jc2-rs`:

```powershell
& $py -B research/tools/material_contract_probe.py "$game/DX10_Shaders_F.shader_bundle"
```

The probe's static checks are narrower than complete shader equivalence. The
continuation leaves final foliage enable/instance-byte authoring, original DDS view policy,
full CarPaint variant/parameter semantics and GPU main/depth/shadow parity open.

Static probe validation completed successfully against the installed retail
bundle/export: ten shader records, five alpha PS identities, reference 128 = 0.5,
enable/reference offsets 60/64, CarPaint shader 973444. `git diff --check` passed.
The PE vtable pointer proof above is a separate coordinator check, not an assertion
performed by this helper. No Cargo test, native consumer execution or GPU test
result is implied by these static checks.

## JC2-025 continuation: original DDS resource/view ownership

This continuation closes the bounded ordinary DDS load/view/bind path below.
The foliage instance-state ownership chain is recovered in the final subsection
below. No renderer or normal decode change is proposed, and the accepted normal
foundation remains intact.

### Exact original name policy and exception asset

Retail executable SHA256 is
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
`CTextureManager` primary vtable `00f228bc + 58` points to **00946080**.
The native bytes return AL=1 (`5f 5e b0 01 5b`) on the positive suffix/list
branch and AL=0 (`33 cd 32 c0 5b`) on the excluded-prefix branch; the exported
`void` signature omits these returns. Do not infer a void policy from that export.

The callback rejects case-sensitive leading `gui_`, `hud_`, `fe_`, `pda_`
**before** removing the last backslash-delimited path component. It lowercases
the resulting basename via **004d3010** (`tolower`), hashes it via **00a165a0**,
then accepts exactly `_dif` immediately preceding the final dot, or membership
in the loaded hash set. `_diff` is not `_dif`. Forward slashes are not stripped
by this callback. These are callback-input rules; the manager's prior cache/name
canonicalization **004ddd60** is a distinct step, not a reason to collapse them.

**0096e040** loads `Settings\\gammacorrect.bin` unless the `UseMode` string is
`OnlyTextureNames`, lowercases each entry, hashes it, stores the hashes and sorts
the container searched by **0093eb00**. Its interface/base pointer adjustments
must be retained when tracing the list storage; an identical offset on an
unrelated object is not this set. The installed asset has 32 names, including
`flare.dds`, `metalflake.dds`, `G_Roadsigns.dds` and `G_Stairs DIF.dds`.
Its decompressed SHA256 is
`4fda3dc3a30c22400493575363ba4645e6fe7e4df831e8f2975154581097f799`.
None of the ten fixture basenames below hashes to this set. Thus both observed
exception-loading modes select the same formats for these fixtures. Unknown
configuration/list population must remain explicit for other textures. The
original uses hashes, including their collision behavior, rather than a string
equality set.

### Complete ordinary load/view/bind chain

**00919860** performs manager lookup/load and invokes the texture's virtual
`+50`. `CTexture` vtable **00f2247c + 50 = 009619d0**. That loader calls
`D3DX10GetImageInfoFromMemory` for the DDS's numeric format, width, height and
mips; reads FourCC at DDS byte **54 hex**; selects payload **80 hex** or
**94 hex** for DX10 headers; optionally drops one top mip under the configured
size limit **013d9dc8**. Legacy uncompressed conversion branches are separate.
If the manager callback accepts a non-null name, the numeric format is incremented
by **one** before resource creation. This is an original per-texture policy,
not a semantic conclusion from material trailer value 3 or diffuse slot naming.

For these ordinary non-depth DDS fixtures, **00950420** places that selected
format in `D3D10_TEXTURE2D_DESC`, calls device virtual **+124** to create the
to **0091c630**, and stores the resulting SRV at resource-wrapper **+8**.
**0091c630** uses a nonzero explicit view format; only zero falls back to the
resource descriptor's format. Separate depth/typeless handling is not this path.
`CTexture +4` owns the wrapper. Texture virtual **+14** resolves to **009573c0**
and returns that wrapper. **00921c60** selects the material's eight-byte texture
handle entry, calls that getter and **0094da50**; the latter reads wrapper **+8**
and invokes device **+10**, `PSSetShaderResources(slot,1,&srv)`.

General setup/draw **00972ce0** maps material entries 0/1/2/3 to PS slots 0/1/2/3
on its corresponding branch (existing variants/optional AO condition remain).
The RBM fixtures identify primary diffuse/normal/MPM slots; CarPaint damage UVs
and foliage's repeated diffuse t1 are governed by the previously established
shader bindings, not remapped by colour-space selection. Resource identity and
view format belong to the loaded texture handle; a renderer must not choose a
new gamma interpretation independently for each occurrence of its filename.

| DDS member | Size pixels; mips | FourCC | DDS format -> requested resource/SRV format |
|---|---|---|---|
| `general.blz/hook_diff.dds` | 128x64; 8 | DXT1 | 71 -> 71 |
| `general.blz/hook_nrm.dds` | 128x64; 8 | DXT1 | 71 -> 71 |
| `general.blz/hook_mpm.dds` | 64x32; 7 | DXT1 | 71 -> 71 |
| `lave.v014_jeep.ee/v014_body_dif.dds` | 1024x1024; 11 | DXT1 | 71 -> 72 |
| `lave.v014_jeep.ee/flattangentspace_nrm.dds` | 128x128; 8 | DXT1 | 71 -> 71 |
| `lave.v014_jeep.ee/v014_body_mpm.dds` | 512x512; 9 | DXT1 | 71 -> 71 |
| `climate/zone_1/vegetation_1.blz/arctic_pine_bark_dif.dds` | 256x256; 9 | DXT1 | 71 -> 72 |
| same container, `arctic_pine_bark_nrm.dds` | 128x128; 8 | DXT1 | 71 -> 71 |
| same container, `arctic_pine_foliage_dif.dds` | 512x512; 10 | DXT5 | 77 -> 78 |
| same container, `arctic_t22_foliage_aoc.dds` | 128x128; 8 | DXT1 | 71 -> 71 |

Formats are raw DXGI enum values. All fixtures are one-array-element 2D images
(resource dimension **3**). Dimensions/mips above are authored metadata before
the optional top-mip reduction. General's exact member is
`general.blz/wea04_lod1-grapplinghook.rbm`; Jeep is
`lave.v014_jeep.ee/v014_lod1-cover.rbm`; pine is
`climate/zone_1/vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm`.
Pine foliage material binds the same `arctic_pine_foliage_dif` handle to t0 and
t1; both therefore use 78, while t2 AOC uses 71. No normal reinterpretation is
implied by the foliage t1 entry.

### Foliage initialization boundaries and reproduction

**00938020** zeroes the 80-byte shader-global staging block **013d9df0** and
binds its PS constant buffer to slot **4**. `CRenderPass` constructor
**0095ce90** installs vtable **00f22664** and initializes its dword **+c98**
to zero (`in_ECX[326 hex]`). **0095dc20** copies that pass-owned field's boolean
value into shader alpha-enable **013d9e2c**, marking dirty **013d9db1** on change.
This proves default initialization plus state propagation, not that every
foliage draw subsequently keeps or changes that value. Render-context **+c98**
is a different object's camera float and must not be conflated with pass state.
**00971da0** writes the reference 128 and sampler/render states; its
**0095e900** calls configure sampler addressing, not alpha enable. Calls labelled
**00677ff0** are not established alpha setters.

**009672a0** reads instance byte **+0b**, normalizes by 255, uploads the fraction
to VS cb2 register16.x, and derives the rounded alpha reference from fraction
times128. Its independent visibility/LOD calculations are not that byte's writer.
At the first DDS evidence checkpoint, this exact instance's authoring path remained open;
endian-swap routines and unrelated UI records that happen to write byte11 are
not sufficient ownership evidence. Final per-foliage enable, lifetime/updates
of byte11 were open. The follow-up below resolves bounded ownership; complete
main/depth/shadow GPU parity remains untested.

Run the read-only reproduction from the material worktree:

```powershell
& $py -B research/tools/material_view_probe.py '../../../..' --export '../../../../jc2-rs/research/analysis/pseudo/_unnamed' --tool 'C:/Users/Rusty/AppData/Local/Temp/jc2tool-material-water-research.exe'
```

The probe checks the retail fingerprint, four PE vtable pointers, two native AL
return patterns, fifteen local export contracts, seven policy cases, the 32-entry
authored exception set and ten real DDS fixtures, emitting per-DDS SHA256s.
Metadata validation calls only `D3DX10GetImageInfoFromMemory` in the installed
**x64** `C:/Windows/System32/d3dx10_42.dll`; the retail PE delayed import names
the same DLL release, but the original game is **x86**. This is CPU-only counterpart
metadata validation, not execution of the retail loader or D3D device creation.
The installed helper SHA256 is
`dfb4baaa5f06467183839f14f5d28466e7c6e7eaa90f292c4516ed6926f5da8a`.
The first DDS checkpoint passed all counts above. It extracts no files, creates
no device, launches no game and runs no Cargo/GPU tests. The expanded probe and
completed static ownership evidence follow.

### Recovered foliage material state and model-owned instance fade

Foliage block vtable **00f23b4c +6c = 00968370**, **+70 = 0091f470**,
**+74 = 009672a0**. Render queue consumer **00950ee0** applies pass state via
**0095dc20**, type setup, and the block's state-application callback before
calling its main draw at virtual+74 with the queued instance pointer. The
state callback **00968370** sets shader alpha-enable via **00952cc0** to
`~(block.flags >> 5) & 1`, using the block-owned flags at **+3c**. Thus bit5
clear enables shader discard; bit5 set disables it. This completes the ordinary
foliage enable chain absent from type setup alone.

When render-context byte **+d14** lacks bit1 and foliage-type byte **+170** is
nonzero, the main state callback instead calls **00928670**. That helper updates
bit1 in the device-state word at **+9c**; it does **not** write shader enable
**013d9e2c**. The alternate callback at virtual+70 selects the same two helpers
according to type byte+170. Preserve both branches; prior shader enable is not
automatically rewritten on the alternate path. GPU coverage/sample behavior of
this device-state bit is not proven here. Type-global option initialization and
all platform/variant paths remain separate from this ordinary shader-discard
chain.

Foliage constructor **0094b1c0** initially zeroes block **+3c**. Reader
**0095a8a0** reads **24 bytes** into block **+28..+3f**, so the last dword is
the raw flags at +3c. The pine model's packet starts at file offset **b707 hex**
and is `[00000000,00000000,3fcccccd,3fe66666,3f800000,00000040]` in little-endian
u32 words. The first five float interpretations are `[0,0,1.6,1.8,1]`; the sixth
is **uint32 flags64**, not a float zero. `jc2tool rbm` formats that final word as
`0.0`, which loses the meaningful flag. This real fixture has bit5 clear and
bit6 set: ordinary state enables shader discard and selects the bit6 shader/
constant branch in **00968370**. Do not rename unproven bits or treat all six
packet members as floats. Model SHA256 is
`5f27657bcc422efd43c3134bb6b3c87166c70f68958ede15c4d515c5bf6bbd2e`.

The original model-instance initializer **00a4f4b0** owns the model handle at
**+c4** and calls shared header initializer **009236d0** with native
`lea ecx,[esi+14]`. That initializer stores `ffffffff` at header **+8**, hence
header byte **+0b = 255**. This is owner byte **+1f**, initially full fraction.
The initializer's exported void return is immaterial; the native pointer
adjustment establishes which header it initializes.

Vegetation update **00a89b90** takes a patch record whose **+10** points to this
model owner. It computes squared world-camera distance from the owner's selected
position buffer, applies model LOD scale/flags and selection, and writes the
owner's byte **+1f** before submission. On the last-LOD/missing-next-LOD branch,
packed owner flags **+0c bits18..25** contain LOD fraction `k`. `k=255` returns
before submission. Otherwise `f = 2 - k*(2/255)`; when f>=1 it writes255,
otherwise it truncates `f*255` into the byte, using single-precision arithmetic.
The other branch writes255. This byte is a model LOD-transition fraction; the
patch's wind/bend motion and the draw's independent visibility test are different
calculations. No authored alpha-enable curve owns it.

The native call at **00a8a297** reloads ECX from patch-record **+10** before
calling **00a42740**, so its receiver is the model owner. That submission obtains
the model's selected-LOD render blocks, then passes **owner+14** into the
pass's virtual **+40** alongside each render block. Pass vtable
**00f22664 +40 = 0094ca50** forwards both values into **00934440**. The queue
stores the header pointer as record dword0 and render block as dword1, including
the bounded overflow fallback. **00950ee0** passes that same dword0 to foliage
draw virtual+74. Consequently **009672a0** reads precisely
`(owner+14)+0b = owner+1f`, normalizes it by255 and uploads VS fraction plus
rounded alpha reference. This is a complete initialization/update/submission/
queue/draw consumer chain, not coincidence between two byte offsets.

The patch update driver **00a8c9a0** supplies the camera position from
**0138202c +54/+58/+5c** and invokes **00a89b90** on selected model-patch records.
Per-frame selected position/LOD buffering must remain paired with these original
update/submission phases. The queue retains a pointer rather than a byte snapshot;
this evidence does not establish a safe arbitrary later writer or permit changing
that owner between submission and draw. Full scheduler synchronization, every
other model fade writer, vegetation spawning/lifetime and LOD/impostor streaming
are outside this bounded chain.

| Last-LOD fraction k | Submitted instance byte | VS fraction | Reference |
|---|---|---|---|
| 0 / 127 | 255 | 1 | 128 |
| 128 | 253 | 253/255 | 127 |
| 192 | 125 | 125/255 | 63 |
| 254 | 1 | 1/255 | 1 |
| 255 | no submission on this branch | n/a | n/a |

These fixture values reproduce original single-precision multiply/subtract and
truncation rather than rounding to a guessed byte. They are CPU arithmetic
checks backed by native writer patterns; no original draw was executed. LOD
breakpoints in the update are squared/scaled distance **100,625,2500,10000**;
their interpolation constants in the retail PE at **0108a228..0108a238** are
approximately **.01, .001904761884, .000533333339, .000133333335, .000004166667**.
The model's scale, LOD count and availability gates must be respected; these
numbers do not define a universal distance or fade for every vegetation asset.

The expanded `material_view_probe.py` now passes **27 export contracts, 7 PE
vtable pointers, 8 native byte-pattern checks, 7 name-policy cases, 32 exceptions,
10 DDS fixtures, the exact pine packet and 5 fade-byte fixtures**. Retail and
helper fingerprints remain as above. The shader-contract probe separately
checks alpha-enable/reference linkage and existing shader identities. This
fulfills bounded load/view binding and one complete instance-state ownership
chain. Original execution, alternate-device-state semantics and final foliage
main/depth/shadow GPU parity remain untested; JC2-025 is ready for review of the
research deliverable rather than a claim of renderer parity.
