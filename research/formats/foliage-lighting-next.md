# VegetationFoliage lighting (board 7625eb04, parent f3772304 gap G4)

Executable `JustCause2.exe` (sha256 `df99a479...`), shader bundle `DX10_Shaders_F.shader_bundle`
(sha256 `db8c67a0...`), PS `vegetationfoliage` 7510628 (disassembly: `research/tools/shader_inventory.py
--filter ^vegetationfoliage$ --disasm-dir <dir>`, 338 lines). Port: `crates/jc2-game/src/foliage_main.wgsl`
(`native_light`), inputs in `foliage_alpha.rs` (`NativeFoliage`, `FoliageExtension::set_native`), loader
`rbm_loader.rs`. Switch for before / after captures: `JC2_NO_NATIVE_FOLIAGE=1`.

## 1. The open question of the parent task, answered: where PS cb1[0] and cb1[1] come from

constant file; `FUN_0096beb0` is the vertex file, which is why `FUN_00971da0` only ever touched the VS cb1).

* **PS cb1[1] = (block +0x28, block +0x2c, 0, 0)** = model header floats 0 and 1 (`CRenderBlockFoliage`
  constructor `0x94b1c0` defaults: 0.5 and 10.0; the retail file overwrites them through the 24-byte read of
  `0x95a8a0` into +0x28..+0x3f). Read in `0x968416..0x968447` (main-pass setup `0x968370`): `movss [edi+0x28]`,
  `movss [edi+0x2c]`, then `call 0x958860` with register 1. The PS uses `.x` as the specular amplitude and `.y` as its
  power. Retail values: (0.15, 15), (0.15, 10), (0.15, 25), (0.05, 8), (0.1, 20), (0, 0) = no highlight (pines).
  Header floats 2..4 (1.6, 1.8, 1.0; defaults identical) are not read by this function: consumer unresolved (Gap 1).
* **PS cb1[0] = (1 - f, 0, 0, w)** written per instance by `FUN_009672a0` (`0x967415..0x96754b`): `f` is the
  instance byte at +0xc times 1/127 for a byte in 1..0x7f and 0 otherwise (`0x967415 mov al,[esi+0xc]`, `cmp al,0x7f`,
  `mulss [0xe5d4b0]` = 1/127); `.y` and `.z` are literal zeros; `.w` is the distance fade `FUN_0096a8b0`
  (the PS does not read it). The header initialiser `FUN_009236d0` sets the dword at +0xc to `0xffffffff`, so an
  untouched instance has `f = 0`: **cb1[0] = (1, 0)**. Consequences in the PS: the normal map is used in full
  (`lerp(.., geometric N, cb1[0].y)` with y = 0), the translucency weight is `1 * f(N.L, N.V)`, the saturated mix
  toward `light * (0.508, 0.54, 0.645)` has factor `sat((3 t2.r)^1.5 ((1 - t1.a) - 1)) = 0`, and the specular is
  scaled by `1 - 0 = 1`. The 1..127 range of the byte (a growth / snow / damage value) has no producer in the
  rewrite yet (Gap 2), exactly like the General snow byte.

## 2. The ported pixel shader (all in native colour units; no Bevy lighting on this route)

```
N      = sign(front) * normalize(t1.x' T + t1.y' B + t1.z' Ng)        t1' = t1.rgb - 0.5, (T, B, Ng) = VS o3, o4, o5
shadow = max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w) * cascade     S = shadow * (t11.x / 2 + 1/2)
w      = (1 - cb1[0].y) * cb1[0].x * (clamp(10 (N.L)(-N.V) - 2, -1, 1) / 2 + 1/2)
sun    = mix(sat(N.L) S cb0[3], (1 - t1.a) S cb0[3] (0.5, 0.6, 0.15), w)         back-lit transmission colour
light  = cb0[12] t2.r t11.x + sun + point lights                                 cb0[12] = cb0[10] + cb0[11], NO 0.5
colour = mix(diffuse light, light (0.508, 0.54, 0.645), sat((3 t2.r)^1.5 ((1 - t1.a) - cb1[0].x)))
colour += cb0[1] cb1[1].x pow(max(N.H, 0.001), cb1[1].y) max(shadow, 0.3) (1 - cb1[0].y)     H = normalize(V + L)
```

* `t0` = diffuse (alpha test unchanged, JC2-024), `t1` = `*_nrm.dds` (RGB minus 0.5, A = translucency), `t2` =
  `*_aoc.dds` at UV1 (R only), `t11` = terrain AO map. In the RBM material the textures are the `normal` and
  `properties` slots (probe: `jungle_T16_palmCLL_LOD1-Whole.rbm` block 0 = fanpalm dif / nrm / `dummy_foliage128_aoc.dds`).
  For some models (arctic pine) `normal` repeats the diffuse; the PS reads it as a normal anyway and so does the port.
* The ambient is the **sum** `cb0[10] + cb0[11]` (twice the mean the other families use) times `t2.r` and `t11.x`: the
  aoc map is what brings it back down. `GeneralLight` gained `ambient_sum` for it.
* Tangent basis: `rbm.rs` now derives the tangent handedness from the stored bitangent (the VS decodes tangent and
  bitangent as two packed vectors). Retail check (`jungle_vegetation_foliage_basis_follows_the_uv_layout`): the decoded
  tangent follows dP/du (mean dot 0.995, never negative) and the handedness is consistent for all 3087 vertices.
* Route selection: native only when a sky lighting (`GeneralLight.sun` non-zero) and the terrain horizon constants
  (`SunHorizon.sun` non-zero) exist; viewer, fixtures and `JC2_NO_NATIVE_FOLIAGE=1` keep the previous Bevy PBR route.

## 3. Native vs inferred vs open

| Piece | Status |
|---|---|
| PS terms, constants (0.5/0.6/0.15, 0.508/0.54/0.645, 10x-2, (3 t2.r)^1.5, max(S,0.3)), `cb0[12]` sum | native (disassembly) |
| cb1[1].xy = header floats 0, 1; cb1[0] = (1, 0) for an untouched instance | native (addresses above) |
| shadow floor `0.1 + 0.9 c` | inferred (`cb0[9].w` default 0.1, general-terms-next.md section 4) |
| cascade `c` | Bevy's cascades (non-native, general-terms-next.md Gap 2) |
| vertex shader point-light sum (v10) | the shared point-light loop (JC2-221) instead of the VS evaluation |
| sampler states of t1 / t2 | wrap / clamp by file kind (inferred; native `s1`, `s2` states not read) |
| underwater blend `cb0[14]` and the alpha output | not ported (output alpha is 1, the discard is the shared test) |

## 4. Result

Whole-frame luma, tile 40,40 jungle, same camera and hour 10:00 (`_extract/7625eb04`): see the board comment. Only the
VegetationFoliage plants change (ferns, palms, lianas, red understory plants): the canopy, which dominates the brightness of
the user's shot, is BillboardFoliage (board f012a36f) and stays untouched.

## Gaps

1. Header floats 2..4 (1.6, 1.8, 1.0): no reader found in the main-pass setup `0x968370` / instance draw `0x9672a0`;
   likely the vertex stage or an unrelated field. Blocks nothing visible known.
2. Instance byte +0xc (cb1[0].x = 1 - byte/127): producer unresolved (which system writes 1..127: damage, growth, snow?).
   Without it the translucency / see-through mix `(3 t2.r)^1.5 (...)` stays at its neutral value.
3. Samplers `s1` / `s2` (filter, address) and the colour space of t1 / t2 views.
4. Native cascade / PCF (shared gap, general-terms-next.md Gap 2) and the VS point-light sum.
