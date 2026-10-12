# BillboardFoliage: camera-facing cards and native lighting (board f012a36f, parent f3772304 gap G5)

Executable `JustCause2.exe`, shader bundle `DX10_Shaders_F.shader_bundle`; shaders `billboardfoliage` VS (128
lines) / PS (45 lines), `billboardfoliageshadow` VS / PS, `billboardfoliageprez` PS (disassembly:
`research/tools/shader_inventory.py --filter ^billboardfoliage --disasm-dir <dir>`). Port:
`crates/jc2-game/src/foliage_billboard.wgsl` (vertex stage, main + depth + shadow), `foliage_main.wgsl`
(`billboard_light`), `foliage_alpha.wgsl` (4x alpha test, parameters), `foliage_alpha.rs` (`set_billboard`,
pipeline key `FoliageKey`), `rbm.rs` / `rbm_loader.rs` (mesh layout, centroid). Switch for before / after
captures: `JC2_NO_NATIVE_BILLBOARD=1` (static XY-plane quads with Bevy lighting, as before).

The jungle canopy models (`Jungle_T0x_CanopyM_*`, `vegetation_0.blz`) are made of these cards. Before this task
they were drawn as static quads in the model XY plane, so most were seen edge-on (speckled dither) and lit by
Bevy PBR with a fixed normal.

## 1. Mesh and block constants (native)

* Vertex (28 bytes): centre `xyz`, `uv`, corner offset `xy` = the VS inputs `v0` and `v1 = (u, v, ox, oy)`.
  Retail check (`jungle_billboard_foliage_cards_share_a_centre_and_carry_corner_offsets`): every card is four
  vertices with one shared centre and one vertex per corner. The Rust mesh keeps `position = centre + offset`
  in the XY plane (bounds) and puts the offset in UV1; the vertex shader rebuilds the centre.
* Block +0x28 (`cb2[5].xyz`) = the mean of all vertex centres, computed by the loader `0x960490` (reads
  material, vertex buffer, indices, then `sum += centre / max(count, 1)`). `rbm_loader::billboard_centroid`.
* Block constructor `0x970b20` (vtable `0xf2371c`, type `0xf2317c`): +0x34..0x3c = (0, 5, 0) = `cb2[6].xyz`,
  +0x40 and +0x44 = 10 and 10 = `cb2[7].xy`; no other writer found, so they are constants (the file has no
  header floats for this block).
* Draw `0x954340` (vtable slot 29; the shadow/depth sibling is `0x95f990`): uploads `cb2[0..3]` = the instance
  matrix and translation, `cb2[4].x = byte(instance +0xb) / 255`, `cb2[5..7]` from the block.
* Type setups (type vtable slots 11..13): `0x93deb0` (main, also the shadow variant by the context flag
  `+0xd14 & 2`) and `0x91bb30` write `cb1[0]` = context `+0xc78` (xyz), `cb1[1]` = the distance fade
  `(d2c / (d30 max(ca8^2, 0.38)), 1 / d30)` (the same as VegetationFoliage, `VegetationFade::foliage`),
  `cb1[2] = (context time * 2, 0.03, 0 or 10000 in the shadow variant, 0)`, and set the alpha test to
  reference constant 128 (comparison 7), i.e. `(2 * 128 - 1) / 510 = 0.5`.
* The render-context field `+0xc78` is row 1 of the 4x4 camera matrix copied by `0x9521e0` to `+0xc68`
  (default init `0x780000`: row 0 = (1,0,0), row 1 = (0,1,0), row 2 = the view direction): the **camera up
  vector**.

## 2. Vertex shader (`billboardfoliage`, ported)

```
c      = M v0                                           world card centre (cb2[0..3])
ang    = fract((c.x + c.y - c.z + t2) / 2pi) 2pi - pi    t2 = cb1[2].x
(s,k)  = 7th order Taylor sin / cos of ang (the disassembly's constants, so the range ends carry the original's error)
U      = normalize(cb1[0] + 0.03 (s, 0, k))              swayed up axis (the compiler stores it as (z, x, y) swizzles)
V      = normalize(c - cam)
R      = cross(U, V);  T = cross(V, R)
corner = c + v1.z R + v1.w T
o0     = viewproj corner + (0, 0, 0.01, 0)
o2.xyz = -V
o2.w   = sat(2 (|(origin.x, corner.y, origin.z) - cam| cb1[1].y - cb1[1].x) + 1)
o3.w   = the haze fog exponent                           (the rewrite applies haze_fog in the pixel shader)
P      = origin + (0, 5, 0);  Vc = normalize(cam - P);  right = normalize(cross(cb1[0], Vc))
(rx, ry) = (dot(right, corner - P), dot(cb1[0], corner - P)) / (10, 10)
n      = normalize(rx right + ry cross(Vc, right) + 0.25 Vc)             fake crown sphere normal
o4.x   = mix(max(0.5 dot(normalize(-offset), cb0[8]) + 0.5, 0), 1, o2.w) * sat((dot(n, -cb0[8]) + 1) / 2)
o4.y   = sat(|M centroid - corner| / 10)                                 darker towards the crown centroid
o4.z   = (1 - sat(dist cb1[1].y - cb1[1].x)) cb2[4].x                    distance fade * instance byte
```

(The `l(1, 1, 0.01, 1)` immediate that the disassembler prints for `add o0` holds the integer 1 (a denormal) in
three lanes: it adds only 0.01 to clip z.) `cb0[8]` is the direction the Sun's light travels, so `-cb0[8]` is
the rewrite's `SunHorizon.sun.xyz`.

## 3. Pixel shader (`billboardfoliage`, ported)

```
rgb   = tex * v4.y
S     = max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w) * v4.x * (t11.x / 2 + 1/2)     (sat(v2.w + 1) is always 1)
light = cb0[3] S + 0.5 cb0[12] t11.x                       no N.L, no normal map, no shadow map, no point lights
alpha = 4 tex.a v4.z;  discard when enabled and alpha < cb4[4].x (0.5)
fog   = the shared exp fog
```

Differences from VegetationFoliage: no cascade shadow term (the PS never samples a shadow map: canopy cards cast
shadows but receive none), no translucency and no specular.

## 4. Rewrite mapping

| Native | Rewrite |
|---|---|
| `cb0[3]`, `cb0[12]`, `cb0[0]`, `cb0[10].w`, `t11` | `GeneralLight` / `SunHorizon` as for the other foliage routes |
| `cb0[8]` | `-SunHorizon.sun.xyz` |
| `cb0[4]`, `cb1[0]` | `view.world_position`, `view.world_from_view[1]` (main camera) |
| `cb1[1]` | `foliage.fade` (`veg_fade`), shared with VegetationFoliage |
| `cb2[4].x`, discard | `MeshTag` instance byte, `foliage_should_discard` (4x, fixed reference 0.5) |
| `cb1[2].x` time | `SunHorizon.tilt.w` = Bevy elapsed seconds (the render-context time is inferred to be a seconds clock) |
| v4.x, v4.y | `uv_b` of the vertex output |

## 5. Native vs inferred vs open

| Piece | Status |
|---|---|
| VS construction, PS terms, constants, 4x alpha test, reference 0.5, block constants (0,5,0) and (10,10), centroid | native (disassembly, addresses above) |
| `cb1[0]` = camera up in the main pass | native (data flow `0x93deb0` <- context `+0xc78` <- camera matrix row 1); the matrix layout is read from the default init and the copy, not from a named field |
| `cb1[2].x`: context `+0xcd4` read as seconds | inferred |
| depth bias `+0.01` of clip z | **not ported**: Bevy's reversed-Z has no equal; on the original it pushes cards back by `0.01 w^2 / near` metres (large at range) so trunks and terrain win ties |
| Bevy bounds | the mesh AABB is the quad extent in XY; cards extend in Z as they turn, so edge culling can be early by up to the card size |

## Gaps

1. Depth bias `+0.01` (clip z) of the VS: needs the original projection's near / far to translate (blocks nothing visible at the
   captured ranges; trunks may show through canopy cards at grazing angles).
3. Render-context time `+0xcd4` producer (seconds? scaled?): the sway phase speed.
4. Native `billboardfoliage_snow` / `_only_*` permutations (flags that select them are not decoded).
5. Cascade-free canopy: the original cards receive no shadow-map term, the rewrite follows; trunk / ground shadows
   are Bevy's cascades (shared gap, general-terms-next.md Gap 2).
