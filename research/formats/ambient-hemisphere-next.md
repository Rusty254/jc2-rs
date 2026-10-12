# Which native shaders use the hemisphere ambient (JC2-133)

Status: research brief plus one runtime change (`grass.wgsl`). Evidence: the pixel and vertex shaders of
`DX10_Shaders_F.shader_bundle` (sha256 `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`),
disassembled with `research/tools/shader_inventory.py --filter . --disasm-dir <dir>` (`sm4dis.py`), then a scan of every
pixel shader for its use of `cb0[10]`, `cb0[11]` and `cb0[12].xyz`. The constants are those of `cloud-shadow-consumers-next.md`:
PS `cb0[10]` sky ambient, `cb0[11]` ground ambient, `cb0[12]` = `[10] + [11]`.

## Two native forms

* **Hemisphere:** `amb = lerp(cb0[11], cb0[10], sat(N.y / 2 + 1/2))` = mean + `(sky - ground) / 2 * N.y`. The Rust side carries the mean
  in Bevy's `AmbientLight` and adds `tilt * N.y` (`sun_horizon.wgsl` `ambient_tilt`, `SunHorizon.tilt`).
* **Uniform:** `amb = cb0[12].xyz * 0.5 * AO` (the mean, no `N.y` term). Bevy's uniform ambient is already exact; no tilt.

## Scan result (pixel shader families, whole bundle)

| Form | Families |
|---|---|
| Hemisphere (cb0[11] used) | FastTer / terrain forest, General (+ plspec, reflection), Lambert, LambertFlatNormal, SkinnedGeneral (+ eyegloss, hair, shnear, 1..4spec), CarPaint (all LOD / deform / reflection variants), VegetationBark and the Bark family (N = the normal-mapped `N.y`), Window, WindowSimple, DeformableWindow, Facade, Merged, DecalSimple, DecalDeformable, SplineRoad, Box, TreeImpostorTrunk, TreeSingleImpostorTrunk |
| Uniform (cb0[12].xyz, cb0[11] unused) | Foliage family (VegetationFoliage, Foliage*, FoliageSimple, BillboardFoliage), Leaves, Flag, DecalSkinned, TreeImpostorTop, TreeSingleImpostorTop |
| Grass | the **vertex** shaders `grass`, `grass_lod0`, `grass_lod1`, `grass_lod2` (VS 194644 ff.) compute the hemisphere: `o2.xyz = lerp(VS cb0[10], VS cb0[9], sat(0.5 (n.y - 10 sway) + 0.5))`, `n.y = 2 v2.y - 1`, `sway = sin(wind phase) * v1.w`; the pixel shaders (PS 6102356, 6103892, 6110964) multiply it by `t11.x` and add the point-light colour from `v3`. Inferred, not proven: VS `cb0[9]` / `[10]` are the sky / ground ambients (the lerp orientation and the sun vector at VS `cb0[8]` fit, the VS buffer layout differs from the PS one) |

## What changed in the runtime

| Material | Before | After | Native evidence |
|---|---|---|---|
| Terrain, General | tilt (JC2-122) | unchanged | hemisphere |
| Forest canopy | tilt | unchanged | hemisphere (FastTer forest) |
| **Grass** | uniform mean | mean + `tilt * N.y` (`grass.wgsl`), N = the terrain normal each blade carries | hemisphere in the VS |
| Foliage, tree-impostor crowns | uniform mean | unchanged, **correct as is** | uniform `cb0[12] * 0.5` |

## Gaps (what stays uniform although the native form is a hemisphere)

1. **Plain StandardMaterial models** (Lambert, SkinnedGeneral characters, CarPaint vehicles, Window, VegetationBark, Facade, decals):
   native hemisphere, Rust uniform mean. Only the `General` header route has a custom shader. Fixing it needs these meshes on an
   extended material (or a shared Bevy hook) that adds `ambient_tilt`; the visible effect is the up/down shading of characters,
   cars and trees at dawn, dusk and night.
2. **Tree-impostor trunk cards:** the native `N.y` is the card normal map's `y` (`TrunkAtlas` normal atlas, `t1`, dotted with the
   constant (0, 1, 0)); the Rust cards have a fixed upward normal and no normal atlas, so a tilt would be wrong (full sky ambient).
3. **Grass `n.y`:** the native value is the blade's own vertex normal minus `10 *` the wind sway (the wind phase of the VS, not the
   Rust sway of `grass.wgsl`); the Rust uses the terrain normal and no bend. The effect is small and temporal.
4. VS `cb0[9]` / `[10]` of the grass VS being sky / ground is inferred (see above).

## Update (JC2-177): plain models take the tilt

Gap 1 is closed for the families of the Hemisphere row except the impostor trunks. `hemisphere_material.rs` /
`hemisphere_main.wgsl` is an `ExtendedMaterial` over `StandardMaterial` that runs Bevy's own lighting and fog and adds
`ambient_tilt` (`tilt * N.y`, the `SunHorizon.tilt` field, binding 101 through `haze::add_haze_binding`). The RBM loader
(`rbm_loader.rs`) creates it for `General` (legacy fallback blocks only), `Lambert*`, `SkinnedGeneral*` (characters),
`CarPaint*` (vehicles), `Window*`, `DeformableWindow`, `Facade`, `Merged*`, `DecalSimple`, `DecalDeformable`, `SplineRoad`,
`Box` and `VegetationBark`; `RbmMaterial` routes foliage first, then the native General material, then this one.
`JC2_NO_HEMISPHERE=1` restores the uniform ambient (before / after captures). Without a terrain (model viewer) the buffer is
the all-zero fallback and the look is unchanged. Wrecked vehicles still char their parts (`explosion.rs`).
Captures: `_extract/JC2-177/` (tile 31,14, 22:00, Rico on a jeep roof): sides of the car and Rico's legs are darker, up-facing
roof and shoulders keep the sky ambient.

Remaining gaps after JC2-177:

1. **Terrain AO on the ambient:** closed by JC2-186 below.
2. **Bark `N.y`:** Bevy's normal-mapped `N` is used for VegetationBark (same intent as the native normal-mapped `N.y`).
3. Gaps 2 to 4 above (impostor trunk cards, grass `n.y`, the grass VS constants) are unchanged.

## Update (JC2-186): terrain AO on the plain-model ambient

Probe: `research/tools/ambient_ao_probe.py` (exit 0; bundle sha256 as above). It finds, in every pixel shader, the hemisphere
ambient (`lerp(cb0[11], cb0[10], .)`) and the `t11` sample register, and follows the ambient into the multiply with `t11.x`
(directly, or through one scalar product). Result, pixel shaders per family (heuristic register dataflow, hand-checked on
`lambert`, `skinnedgeneral`, `splineroad`, `decalsimple`, `deformablewindow`):

| Family | Ambient multiplied by `t11.x` |
|---|---|
| Lambert (36), LambertFlatNormal (12), CarPaint (291), Facade (4), Merged (8), Box, SplineRoad, Window, WindowSimple, VegetationBark, Bark (60), General (3) | yes, directly (`mad ambient, ambient_weight, t11.x`) |
| SkinnedGeneral and variants (52) | yes, as `ambient * (t2.x * t11.x)`: the factor `t2.x` (the R channel of the `t2` map, the same scalar that scales the sun's N.L term and the spec) multiplies it as well |
| DecalSimple (1), DecalDeformable (1) | **no**: these shaders sample no `t11` at all; the ambient is uniform, no AO |
| DeformableWindow (1) | **no**: `t11` is sampled but only feeds the sun shadow (`cb0[0]` term); the ambient is not multiplied |
| FastTer terrain family | own route (`terrain_splat.wgsl`), not part of this change |

Runtime: `HemisphereExtension` binds the shared map (binding 102 / 103, `HorizonShadow::ao`, filled by `bind_ao`; the default white
image until a terrain exists, i.e. AO 1) and a per-material flag (104, `reads_terrain_ao(type)`); `hemisphere_main.wgsl` multiplies
`diffuse_occlusion` by `mix(1, t11.x, flag)` at `ao_uv(world.xz)`, the same as `general_main.wgsl`. `JC2_NO_HEMISPHERE_AO=1` turns it
off (before / after captures). Captures: `_extract/JC2-186/` (tile 31,14, 19:00, Rico on a slope): the dark slope and the character
are slightly darker where the map holds occlusion (mean brightness about -0.8 %); the sun-lit parts are unchanged because only the
ambient term takes the factor.

Gaps after JC2-186:

1. **SkinnedGeneral `t2.x` factor:** native ambient and sun both scale by the `t2` map R channel (PS `skinnedgeneral`, `mul r2.w, r2.x, r4.x`
   after the hemisphere lerp); the Rust character materials do not apply a `t2.x` scale (the channel's meaning, the map and whether the
   loader reads it are unresolved). Blocks the exact ambient on characters.
2. **Sun horizon shadow:** native direct sun on these shaders is scaled by the horizon shadow `max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w)`
   (PS `lambert`, then mixed by `cb1[0].w`; some families also by `t11.x / 2 + 1/2`, as `splineroad`); the plain route leaves Bevy's sun unscaled
   (the terrain / General routes use `sun_correction` of `terrain_splat.wgsl`). Visible as missing sun shadowing on cars and characters in valleys.
3. The remaining gaps of JC2-177 (bark `N.y`, impostor trunk cards, grass) are unchanged.
