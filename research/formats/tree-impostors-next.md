# Tree impostors, vegetation fade ranges and the fade byte route, JC2-126

Retail `JustCause2.exe` SHA256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`,
`DX10_Shaders_F.shader_bundle` SHA256
`db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`. Reproduce (read-only, exit 0
only if every check passes; needs the ignored local Ghidra export):

```powershell
& $py -B research/tools/tree_impostors_probe.py $game --export "$root/research/analysis/pseudo/_unnamed"
```

Builds on JC2-086 (`vegetation-lod-native.md`), JC2-024/025 (`foliage-alpha-runtime.md`) and
the existing data description in `world.md` ("Tree impostors"). Research only, no runtime change.

## Proven

**1. Who owns the impostors.** `CVegetationPatchPS` (vftable 00f21794; ctor 00a86850, dtor
00a86900) embeds one `CTreeImpostorTrunkRB` at +0xc0 and one `CTreeImpostorTopRB` at +0xe0. Each
RB holds a vertex buffer (+4), an index buffer (+8), a first-index (+0x14), two quad-list counts
(+0x18 reverse-allocated, +0x1a forward) and a flag byte (+0x1c). Both RBs draw from a **shared
process-wide pool** (their vtable slots 9/10 return it: Top 00a7aeb0 -> 01381fb8, Trunk 00a7b160 ->
01381fd4); the pool is a two-ended 0x18-byte vertex array with u16 indices. The RB Types
(`CTreeImpostorTopRBType` 00f216f4, Trunk 00f21744) provide the create slot (00a83700 / 00a84360)
and the per-pass state setup (slot 12: 00a83740 / 00a843a0). The patch finalise slot (vtable +0x10,
00a91600) runs 00a90dd0 (build all cards of the patch), then 00a7af20 (upload Top) and 00a7b1e0
(upload Trunk). `TreeImpostors.bin` is read once by 00a96af0 (it also fetches the atlases); the
record tables are 0x4c-byte entries (Top table first, Trunk table second, ids = the `billboard`
key of the vegetation type).

**2. How cards are fed.** The patch holds a tree list of 0x10-byte entries (position xyz, yaw byte
at +0xc, type index at +0xd, active at +0xe, vegetation-set index at +0xf), kept in a ring of 19 x 19
cells of 128 m (`index % 0x13`). Entries are registered by 00a91480, whose only caller is 00a93450
(type looked up by hash in the set's 0xd0-byte type table, 00a872c0), whose only caller is the
tree-list loader 008a40f0 (record stride 0x14, fewer than 2000 per file). 00a90dd0 walks the cell,
and for every entry whose type record has a billboard id (short at record +0x94 >= 0) calls 00a869c0
with position, id, yaw = byte x 2 pi / 255 and the type extents (+0x34/+0x38). 00a869c0 adds the
crown cards (00a83c90, Top pool) and trunk cards (00a84950, Trunk pool) and grows the patch bounds in
the spatial index. A table entry whose flag byte (+0x4a, the `#190d9496` key of `world.md`) is 0 is
yaw-rotated and goes to the forward list; any other value is camera-facing and goes to the
reverse-allocated list; cards are baked per patch, **not per frame** (nothing in the card stream is
distance dependent). Vertex colour bytes: record value, yaw byte, `00a63730(position)` x 255 (a
ground factor; also sets RB flag +0x1c when > 0.001 for Top), record value.

**3. Where the distances come from.** The per-frame vegetation manager update 00a92ca0 writes eight
floats into the render engine (`DAT_01191ab4`, byte +4..+0x20) every frame; 0095a190 copies them to
the render context (+0xd1c..+0xd38):

| ctx | engine | value |
|---|---|---|
| d1c | +4 | 450.0 |
| d20 | +8 | 350.0 |
| d24 | +0xc | 480.0 |
| d28 | +0x10 | 120.0 |
| d2c | +0x14 | 55.0 for lodfactor 0 and 1, 119.0 for 2, 183.0 for 3 |
| d30 | +0x18 | 55.0 |
| d34 | +0x1c | 60.0 for lodfactor 0 and 1, 124.0 for 2, 188.0 for 3 |
| d38 | +0x20 | 60.0 |

The values live in float tables (00f20a20: 450, 350, 480, 120, 55, 60; 00f21c00: 188, 183, 124,
119). The vegetation level is `DAT_0108b924`, set by 00a7b730 from the graphics setting id 0x17
(`/lodfactor`, "Geometric detail 0-3, default 0"; stored by 005b5530 and applied by 00612050).
00612050 also sets the model-LOD distance unit `DAT_01089eb8` = 1.0, 1.5, 2.0, 3.0 for lodfactor
0..3, so **the JC2-086 breakpoints are the lodfactor 0 case** (default). 00a92ca0 also sets patch
radius `DAT_0108b8fb` = 7 / 11 / 13 (meaning unresolved, Gap 3) and `DAT_0138aaac = 1/d2c^2`.

**4. How the VS uses them.** Pass setup builds VS constant buffer 1 vector 1 from the context and
the camera FOV `f = max(fov^2, 0.38)` (`ctx+0xca8` is camera +0x618, the FOV in radians stored by
0092c620; the camera code converts degrees with 0.017453292):

* Top (00a83740): `cb1[1] = (d2c/(d30 f), 1/d30, d1c/(d20 f), 1/d20)`.
* Trunk (00a843a0): with `a = d38 f`: `cb1[1] = ((a/3 + d34)/a, 3/(2 d38), d24/(d28 f), 1/d28)`.
* Both vertex shaders (TreeImpostorTop 296816, TreeImpostorTrunk 303736 in the bundle) take the
  card centre (`v0.x` height, `v1.xy` x/z times `cb2[0]`), subtract the camera (`cb0[4]`) and compute
  a Chebyshev distance `c = max(|dy|/16, |dx|, |dz|)` and a Euclid distance `e`
  (Top: `sqrt(dx^2+dy^2+dz^2)`; Trunk: `sqrt(dx^2+dz^2+(dy/4)^2)`), then
  `alpha = min(1 - sat(c*w - z), sat(e*y - x)) * 3 + 0.99`. When `alpha < 0.996078` the vertex is
  clipped (`z := -w`, the whole card goes, no fade left); otherwise `alpha` (0.99..3.99) is passed on
  and the PS multiplies it with the atlas alpha (`o.w = tex.a * alpha - 0.996078`, discard below the
  `cb4[4].x` reference when the alpha gate `cb4[3].w` is on; the reference is 0xfe when context pass
  bit 2 is set, which 0095a190 takes from engine +0xea5; that pass's identity is unproven).
* Foliage models: VegetationFoliage VS 320752 and BillboardFoliage VS 39144 (and the
  shadow/velocity/simple variants) multiply `1 - sat(dist*cb1[1].y - cb1[1].x)` into the instance
  fade `cb2[16].x`, where `cb1[1].xy` are the **same d2c/(d30 f), 1/d30 as the Top impostor**, and
  `dist` = length of (model-origin x/z, transformed vertex y) minus camera. A fade of 0 clips the
  vertex (JC2-024/025). So foliage-material meshes (leaf and needle models, billboard foliage) fade
  out over exactly the range in which Top cards fade in; General/Bark meshes are not touched.

Resulting distances (probe fixtures, f32 arithmetic; `f = 1` for any FOV >= 1 rad, `f = 0.38` for
FOV <= 0.6164 rad):

| lodfactor | f | foliage full -> gone | Top card visible from | Top card gone (Chebyshev) | Trunk card visible from | Trunk card gone (Chebyshev) |
|---|---|---|---|---|---|---|
| 0 | 1 | 55 -> 110 m | 55.1 m | 799.3 m | 53.4 m | 599.8 m |
| 2 | 1 | 119 -> 174 m | 119.1 m | 799.3 m | 96.1 m | 599.8 m |
| 3 | 1 | 183 -> 238 m | 183.1 m | 799.3 m | 138.8 m | 599.8 m |
| 0 | 0.38 | 144.7 -> 199.7 m | 144.9 m | 1533.5 m | 118.7 m | 1382.9 m |

Start distances scale with 1/f, widths (55, 350, 120, 40 for the trunk fade-in) do not.

**5. The last-slot fade byte (JC2-025) native route.** 00a89b90 writes the owner byte +0x1f
(JC2-086); 00a42740 passes `owner + 0x14` as the model-instance header to the model's submit slot, so
the owner byte is header byte +0xb (initialised 255 by 009236d0). The foliage submit 009672a0 reads
that byte, uploads `byte/255` as VS constant buffer 2 vector 0x10 (`cb2[16].x`, the multiplier in the
foliage VS) and `round(byte/255*128)` as the alpha-test reference (00949aa0 slot 7). It is **per
model instance**, uploaded with that instance's other constants, never part of the shared material.

## Inferred

1. **Handover.** Both are drawn: a foliage mesh fades out 55 -> 110 m (lodfactor 0, f = 1) while the
   cards fade in over the same interval, then the card is hard-clipped beyond 800 m (crowns) or
   600 m (trunks) Chebyshev. Cards are not culled by the model's slot/2000 m rule. The project
   `TREE_RANGE` 520..580, `BUSH_RANGE` 380..440 and the impostor 450..600 fade do not exist
   natively. The nearest native numbers are the card cut-offs (800 / 600 m Chebyshev at f = 1) and
   the foliage fade (55 -> 110 m); none of them is a model "range" in the project sense.
2. **Bushes and rocks** have no impostor (billboard id -1): their foliage meshes vanish by the same
   55 -> 110 m while rocks (General) follow only the JC2-086 slot rule. Whether any bush model uses
   non-foliage materials is not censused (Gap 6).
3. **FOV factor.** `f` follows the camera FOV; at the 35 degree floor all ranges stretch by 2.63.
   A narrower zoom therefore draws trees as models/cards further out. The default gameplay FOV is
   not proven (Gap 1).
4. The 128 m cell ring is 19 x 19 (+-1152 m): it covers the 800 m cut-off at f >= 0.7 only. This
   suggests the ring or the PS patch ring (15 x 15 patches of 512 m, 00a86f10) limits distant cards
   rather than the shader when f is small (Gap 3).
5. The TreeSingleImpostor / Snow / Shadow shader families are separate shaders in the same
   bundle selected by the RB pass setup; their selection rule is not decoded (Gap 4).

## Gaps

1. **FOV value and axis.** Default gameplay FOV written through 0092c620 (callers 008fd280,
   008ffdd0, 00903ef0, 0067c330, 00980ef0, 00a79d80; one converts degrees at 008f0000) and whether
   `camera+0x618` is vertical or horizontal. Blocks the absolute start distances (`f`).
2. **Which trees are registered.** The impostor ring is filled from the tree list loaded by 008a40f0
   (file name and format not recovered) through 00a93450. The generator 00a85eb0 calls a six-argument
   slot 1 (CVegetationModelPatch 00a8a760); PS slot 1 (00a86ec0, four arguments) has no direct caller.
   Whether generated trees reach the cards through this list or 00a86ec0 is unproven; `world.md`
   currently assumes the generator. Blocks "same list as the models".
3. **Ring and radius.** Meaning of `DAT_0108b8fb` (7/11/13), the 19 x 19 x 128 m ring versus the
   15 x 15 x 512 m PS ring and which one bounds distant cards; load/unload hysteresis.
4. **Pass state of the cards.** Blend / alpha-to-coverage / write state set by 00a83740 and 00a843a0
   (FUN_0095e900, 00970020, 009203e0, 0091c270, 00952cc0, 00928670), Snow variant choice, `Single`
   variant use, and shadow-pass participation. Blocks exact card alpha rendering.
5. **Other consumers of d24/d28/d34/d38.** Trunk cards use them; other users (grass LOD, leaves)
   are not mapped.
6. **Model/trunk overlap census.** Which retail `.lod` slot models use VegetationFoliage /
   BillboardFoliage versus General materials (does a pine's bark stay as model out to 2000 m while
   the trunk card also shows from 53 m?).
7. **Runtime route for the fade byte** in Bevy: `MeshTag` exists in the pinned `bevy_mesh` 0.19.1 and
   `mesh_functions::get_tag` in the pinned `bevy_pbr`, but use from the foliage extension shader
   (main, prepass, shadow) is unverified.
8. **Visual receipts.** No captures at 55/110 m or 450..800 m; the lodfactor setting is not exposed in
   the project; wind attributes in the card vertex (`v2`/`v3`) beyond the yaw rotation are not decoded.

## Bounded Rust handoff (proposal, nothing implemented here)

1. Add pure fades to `crates/jc2-formats/src/veglod.rs`: `engine_table(lodfactor)`,
   `top_constants / trunk_constants / foliage_constants(table, fov)` and `card_alpha`, `foliage_fade`,
   unit tests copied from the probe fixtures (lodfactor 0, f 1 and 0.38). Constants only from the
   tables above; FOV is an input (Gap 1), default to the project camera's vertical FOV in radians and
   label it provisional.
2. One shared `Resource` (extracted to the render world once per frame) holding the three
   `Vec4`s; the foliage extension (`foliage_main.wgsl`, `foliage_prepass.wgsl`) multiplies
   `1 - sat(dist*y - x)` into its fade, using the entity origin x/z and the vertex world y. No per-plant
   material clones: the material stays shared, the constants are global.
3. Fade byte: replace the private clone of `FoliageInstance` by `MeshTag(byte)` written by
   `plant_lod.rs` from `veglod::instance_byte`, read in the extension (`get_tag(instance_index)`),
   multiplied with the distance fade and used for the alpha reference. First confirm that the main,
   prepass and shadow passes of the extension see the tag (Gap 7).
4. After 2 and 3 work, drop `scenery::TREE_RANGE` / `BUSH_RANGE` `VisibilityRange` for models that
   carry the foliage material; keep the JC2-086 slot rule (and its cull) for everything else. Do not
   change non-foliage meshes without the Gap 6 census.
5. Impostors (`impostors.rs`, `impostor.wgsl`): replace `FADE_FROM/FADE_TO` `VisibilityRange` by a
   per-card test in the shader using the Top / Trunk constants and the card centre
   (alpha formula and `< 0.996078` clip above); keep `Mask(0.4)` until Gap 4 is closed. The
   `--impostor-radius` tile window must reach 800/f m (Gap 3).
6. Receipts: headless unit tests for the formulas; captures at 40/80/120/300/700 m with lodfactor 0
   (Gap 8) before the old constants are removed.
