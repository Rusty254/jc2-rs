# Water rendering: recovered bindings, motion and staged implementation

8 October 2026. Research-only extension of `graphics-next.md`, aligned with
`notes/RESEARCH-HANDOFFS.md`. No Cargo, tests, game or GPU capture was run.
Primary evidence: retail archives/DXBC and existing Ghidra export. Addresses are
retail executable VAs; shader offsets refer to DX10_Shaders_F.shader_bundle.
Keep raw assets/dumps ignored and use a private prebuilt tool copy.

## Verified resources and CPU ownership

`FUN_00a8b220` initializes the water renderer. It allocates:

| Resource | Creation arguments / owner |
|---|---|
| reflection_texture | 256x256, format 0x1d, one mip/sample path; owner +0x10 |
| reflection_depth_buffer | 256x256, format 0x2d; owner +0x14 |
| water_normal_texture | 256x256, format 0x1c; owner +0x34 |
| raw bump | loads Water\water_bump.dds |
| foam | loads Water\wave_foam_dif.dds |

The initializer writes the CWaterHighEndRenderBlockType singleton
`DAT_01381fa0`: +0x44 = generated normal texture, +0x48 = reflection texture,
+0x40 = view obtained from the foam shared texture, +0x4c = owner+0x64 (matrix
storage). Therefore the ocean normal input **is not directly water_bump.dds**;
that asset supplies an earlier composite pass.

`FUN_00a80300` configures the high-end ocean type and binds:

| PS slot | Confirmed identity or provenance |
|---|---|
| t0 | DAT_01381fa0+0x44: generated water_normal_texture |
| t1 | +0x48: reflection_texture |
| t5 | render context +0xcc4: scene-colour input; shader samples behind refracted pixel |
| t6 | +0x40: wave_foam_dif texture view |
| t7 | render context +0xcc8: depth input; reciprocal-depth reconstruction proved |

Context identities t5/t7 are confirmed by usage as colour/depth, but producer
render-target names, copy/resolve timing and multisample behaviour remain unknown.
The type vtable is **0x00f21044**, instance vtable **0x00f21094**. Its render
instance code chooses WaterBelow for below-surface camera/pass; high-end paths
and NVIDIA variants need separate validation.

Format 0x1c = RGBA8_UNORM, 0x1d = RGBA8_UNORM_SRGB, 0x2d = D24_UNORM_S8_UINT,
verified against [Microsoft DXGI_FORMAT](https://learn.microsoft.com/en-us/windows/win32/api/dxgiformat/ne-dxgiformat-dxgi_format).
`FUN_00950420` creates the textures and `FUN_0091c630` their SRVs. The generated
normal and reflection resource formats differ, so retaining colour/data view
identity is necessary. Full colour conversion across reflection and final output
is not established solely by the resource creation request.

Retail archive `jc2tool list` resolves six textures under `textures\water\`:
water_bump.dds (349680 compressed bytes), wave_foam_dif.dds (43832),
wave_dif.dds (349672), wave_mask.dds (16512), water_godrays_dif.dds (8320),
water_godrays_alphamask.dds (16512). These are archived lengths, not DDS dimensions.

## Verified motion and optical operations

### Normal generation

WaterBumpComposite VS **336476** scrolls four UV pairs using cb1[0].x:

```
uv0 += phase * (+0.06, +0.05)
uv1 += phase * (-0.03, -0.07)
uv2 += phase * (-0.04, +0.07)
uv3 += phase * (-0.07, +0.03)
```

WaterBumpComposite PS **7535708** samples t0 four times, averages RGB, and
outputs alpha `0.5 + cb1[0].w*(average_alpha-0.5)`. This yields animated composite
data. cb1[0].x is clearly a phase/motion parameter; exact CPU clock units and its
writer were not traced, so “seconds” remains an inference. RGB and alpha encode
different controls. Do not regenerate normals by interpreting RGB alone as height.

WaterShader_lod0 PS **7540448** reads generated t0 at world-coordinate scales
0.00425438, 0.0638157, 0.2 and 0.0212719. Swizzle is yxz minus 0.5. The last
two layers combine with weights 1 and 2.4, then the medium layer contributes 1.5.
A blend factor max(v1.z,0.4) and v1.w control the resulting normalized vector.
The unusual disassembly literals `2147483648` in zero-like components are raw
bit-pattern -0.0 printed by the research disassembler, not enormous wave offsets.

### Refraction, depth colour, caustics and reflection

LOD0 reconstructs eye-space distance from t7 using:

```
eye_depth = 1 / (sampled_depth*cb1[1].x + cb1[1].y)
thickness = selected_eye_depth - surface_eye_depth(v5.w)
```

It compares distorted-pixel depth against surface depth and substitutes the
undistorted depth if refraction would sample foreground. This is a concrete
anti-foreground-refraction rule. cb1[1].z/w scale reflection/refraction distortion.
`FUN_00a80300` computes x=`1/context[cb0]-1/context[cb4]`, y=`1/context[cb4]`
(object byte offsets +0xcb0/+0xcb4, not shader cb registers). z/w defaults above
water are **0.24/0.045**; below water **0.8/0.1**.

Colour t5 is multiplied by (0.8,0.97,0.98). The shader reconstructs world position
with cb1[3..6], applies two t0-alpha lookups at projected world XZ scales 0.08/0.3,
and uses `((a0 + 0.5*a1)*2)^3` with shallow-under-surface gating to modulate colour.
This is **caustic-like amplification**, distinct from the foam t6 red/green usage.
It blends toward cb0[14].rgb with attenuation:

```
attenuation = max(0, 1 - exp2(-thickness*cb0[14].w))
colour = lerp(refracted/caustic_colour, cb0[14].rgb, attenuation)
```

This is base-2 exponentiation (SM4 exp), not natural exp unless converted by ln(2).
cb0[14].w is optical density in reciprocal distance for this equation; its original
authoring name and weather-dependent CPU writer remain unresolved.

t1 supplies planar reflection. LOD0 combines reflection/refraction with luminance
weights, view-angle powers, normal/view dot products and near-surface controls.
It is not simply standard Schlick Fresnel. cb1[0].rgb supplies sun-specular colour;
.w is exponent. `FUN_00a80300` writes exponent **256**; secondary target alpha uses
a highlight with eight times that exponent and scale 4.5. Both SV_Target0 and
SV_Target1 get colour, but their alpha meanings differ. Output alpha is not a
single ordinary transparency value suitable for copying into StandardMaterial.

### Planar reflection camera

`UpdateWater` is registered with `FUN_00a8e480`, which calls **FUN_00a8b790**.
For an above-water camera it reflects camera height about y=200:
`reflected_y = camera_y - 2*abs(camera_y-200)`, builds a reflected camera basis,
and submits the reflection camera. Below-water handling changes camera basis and
render setup. `FUN_00a79d80` copies camera projection parameters. Thus planar
capture is a recovered technique, not a proposed generic cube-map replacement.

## Verified separate wave/foam layer

`CWaterWavesRenderBlockType` vtable **0x00f2165c**, instance **0x00f21124**.
Its initializer in text_00a80000.c (search `Water\\wave_dif.dds`) loads wave_dif,
wave_foam_dif, wave_mask. **FUN_00a814a0** binds them respectively t0/t1/t2.
Waves PS **7558872** computes:

```
a = w0*wave_red(uv0) + w1*wave_red(uv1) + w2*wave_red(uv2)
alpha = saturate(4*a*foam_red(foam_uv)*mask_red(mask_uv))
```

Colour comes from sun/ambient sky constants and fog. Waves VS **344216** uses
vertex z and cb1[3].w to form world y, and four UV transforms. The mesh is not
merely an extra normal texture.

horizontal jitter, base y=200, 16 vertices per wave and a shared strip index
pattern. The initializer supplies those arguments from `FUN_00a8b220`. This is
proof of a bounded generated wave-object layer. It does **not** yet establish
shoreline-only spawning: terrain/coast acceptance, time evolution, wave lifetime,
camera-relative wrapping and the distinction from open-water foam need further
tracing. Do not claim shoreline foam is fully decoded from texture names.

The main LOD0 shader also samples t6 foam: green enters a thickness/fade ratio,
surface coverage is multiplied by depth-derived gates, and resulting colour is blended toward
ambient/sun tint. This screen-depth foam path and the separate Waves draw are
different effects. Preserve both distinctions in the renderer design.

## Verified underwater treatment and open consumer gaps

Underwater PS **7482432** uses t5 RG perturbation to offset screen UV by +/-0.01,
t2 depth reconstruction, t0/t1 colour blend controlled by depth ranges, t4 colour
overlay, luminance-dependent saturation, global colour multiplier, and reconstructed
world position. It adds another t6-alpha caustic modulation at 0.08/0.3 scales,
then outputs `sqrt(colour)`. This is an explicit approximate display conversion
in this pass; applying another sRGB encode would require deliberate pipeline design.
Underwater edgefade/filmgrain/stereo variants also exist; their extra behaviour
was not decoded. CPU binding identities t0..t6 for this postprocess remain unknown.

WaterBelow PS **7532292** has the underwater-surface optical path, outputs two
targets, uses t0/t5/t6/t7, and applies depth-based colour fog when camera.y<200.
General, skinned and foliage shaders independently apply water-colour/fog terms;
underwater behaviour is not confined to a final fullscreen effect. Godrays have
their own renderer and two explicitly loaded textures; density/volume setup remains
unresolved. The visual camera crossing must coordinate these passes.

## Typed/runtime proposal and staged minimum milestone

Suggested configuration separates `WaterNormalMotion { phase, scrolls[4],
alpha_strength }`, `OceanOptics { depth_reconstruction, density, water_colour,
distortion_above/below, specular_power }`, `ReflectionCapture`, `WaveLayer`, and
`UnderwaterPostprocess`. Retain original raw constants and variant identity.
Render targets are camera-owned; the sea mesh/render region follows streaming.
Never make generated waves, shader height, foam or refraction alter boat buoyancy,
terrain collision or **physical SEA_LEVEL=200**.

1. Minimum material: dedicated ocean shader with deterministic four-layer normal
   motion, shared normal image, sun highlight and explicitly approximate Fresnel.
   Start without undocumented alpha/MRT reuse. Record its approximation clearly.
2. Add resolved scene colour/depth textures, world-position reconstruction and
   foreground rejection. Port exp2 density and separate shallow/deep colour.
3. Add planar reflection camera/clip plane/resolve ownership and per-view lifetime.
   Then screen-depth foam and bounded wave objects; confirm shoreline acceptance
   before labeling a procedural layer coastline-specific.
4. Add below-surface and underwater postprocess with fog coordination and explicit
   colour-space chain. Godrays and filmgrain follow after the crossing is stable.

Future controlled captures (not yet executed): fixed time/day-speed and camera,
no traffic/civilians/music, identical graphics settings for before/after:

| Fixture | Meaningful acceptance |
|---|---|
| Open ocean at noon and grazing sunset | deterministic motion, no world-origin jump, highlight/reflection follows sun/camera |
| Coast over shallow sand then deep shelf | thickness monotonic, no foreground refraction bleed, bounded foam and no shore gaps |
| Camera crossing y=200 from both directions | stable pass transition, consistent colour conversion, no reflection inversion or abrupt extra fog |
| Boat crossing a tile boundary and stopping offshore | rendering follows region/camera while buoyancy/physical water level remains unchanged; no duplicated wave layer or leaked targets |

Numerical validation should reconstruct known camera/world points, check exp2
attenuation at zero and increasing thickness, prevent sampling foreground depth,
and compare normal phase at two known times. Headless math proves those contracts;
GPU captures are still required for resource binding, blend order and MSAA parity.

## Reproduction and remaining research

Private TEMP tool: `jc2tool-material-water-research.exe list <game>` with water
path filter. Inspect FUN_00a8b220 / FUN_00a8b790 in text_00a80000.c,
FUN_00a80300 / FUN_00a814a0 in that export, FUN_00a79fe0 in text_00a70000.c.
Use `shader_inventory.inventory` plus `sm4dis.disasm` in memory (Python -B) with
exact filters `^waves$|^waterbumpcomposite$`; existing ignored disassembly covers
LOD0/WaterBelow/Underwater. No proprietary data is saved in this brief.

Outstanding: raw bump-to-generated-normal pass writer/clock, texture DDS sizes and
channel statistics, full colour-space chain, cb0 water-colour authoring fields,
shoreline generation/lifetimes, all underwater bindings, reflection clip/resolve
details, NVIDIA wave-displacement path and per-quality LOD transition rules.
