# Parachute canopy shading, lighting, shadow and night look (board 28d8e587, gap G3 of parachute-parity.md)

Probes: `research/tools/parachute_canopy_shading_probe.py --game <game folder> [--report]` (20 checks, exit 0; run from `research/tools`
without `python -I`, it imports the other probes) and, for the instance constants and shadow submission (board 540ac1d1),
`research/tools/parachute_instance_defaults_probe.py` (15 checks, same usage; see the section "Instance constants and shadow submission"). Builds on `parachute-canopy-model.md` (the canopy is the skinned mesh
`general.blz/gae05_lod1-gae05.rbm`, one `SkinnedGeneral` block).

## Result in one paragraph

The canopy has **no special material**. It is a plain `SkinnedGeneral` render block, the same shader family as Rico's clothes
(pixel shader `skinnedgeneral`, PS offset 6753928, plus the light-count variants `1spec..4spec`, `pl*`, `shnear*`, `snow*` that the
engine picks per draw). Its block flags `0x00010000` mean: **opaque** (low byte 0, not the blend value 4), **double sided** (bit 16 forces
the cull code to "none"), small vertices (bit 19 clear), and the character shader-table bit 20 clear. It is lit by the sun, the hemisphere
ambient, the terrain AO / horizon shadow, point lights and fog like every other lit object, and it adds a **rim term**. The dark grey
look comes from the data: diffuse mean about (45, 45, 44) of 255 scaled by `MPM.r` about 0.64. Nothing in the block says it is
transparent, emissive or unlit.

## Native-proven facts

| Fact | Evidence |
|---|---|
| Block class and `Read` | `CRenderBlockSkinnedGeneral` vtable `0x00f22e74`, slot 1 `0x0092d350`: reads `u32` flags and six `f32` into block `+0x28..+0x40`, then material (`FUN_0094c710`), vertices (`0x00958a60`, or `0x00934fb0` when flag bit 19 = large vertices), faces |
| Type draw functions | type vtable `0x00f22ee0`: slot 0 `0x00931080` main-pass setup, slot 1 `0x009308a0` depth/shadow setup, slot 2 `0x009657c0` main per-instance draw, slot 3 `0x009717c0` depth/shadow per-instance draw |
| Canopy flags | `0x00010000`, floats `(10.0, 1, 1, 1, 1, 1)` (GAE18 differs only in the third float, 0.5). Rico's blocks use `0x100000` (clothes), `0x80000` (hair, large vertices), `4` (eyes, blend) and low byte 1 variants; none uses `0x10000` |
| **Double sided** | `0x00931080` and `0x009308a0`: `cull = (low byte == 1 \|\| bit 16) ? 1 : 3`, stored by `0x0095fe00` into bits 2..3 of the render-state word (General uses the same call with 1 = none, 3 = back-face cull). Canopy: bit 16 set, so cull none in the main pass and in the depth/shadow pass |
| **Opaque, no blend** | the blend branch (extra blend states, 2 more render targets, `0x0095e900`) is taken only for low byte 4 (`0x00931080`); low byte 0 takes the opaque branch. The pixel shader writes alpha `diffuse.a * cb1[1].w` and discards only when the shared alpha-test enable `cb4[3].w` is set (`discard_nz`), as for every opaque family |
| Which maps | PS `skinnedgeneral`: `t0` diffuse (times instance colour `cb1[1]`), `t1` normal decoded `2*rgb-1` on the tangent basis (`v6, v7, v8`), `t2` properties; the front-face flag flips the normal for back faces, which is what makes the double-sided canopy lit correctly from below |
| `gea05_nrm` is a tangent-space normal map | the PS multiplies it by the interpolated tangent/bitangent/normal basis; the canopy mesh carries tangent and bitangent streams (rbm.md SkinnedGeneral layout) |
| Depth/shadow participation | `0x009308a0` and `0x009717c0` return early only for low byte 4 (blend) and, when manager `+0x178` is clear (the constructor clears it), for blocks with bit 18; the opaque canopy block therefore **is drawn by the depth/shadow type functions** (it is a shadow-capable block) and, being double sided, is drawn there with cull none |
| Lighting inputs | same constant buffers as the other lit families (cb0 sun `cb0[3]`, light direction `cb0[4]`, specular colour `cb0[1]`, ambient `cb0[10]`/`[11]`, fog `cb0[13]`, underwater `cb0[14]`, point-light grid `t3`, terrain AO/horizon map `t11`, `general-terms-next.md` sections 2 to 4) |

## The lit colour, decoded from PS 6753928

Notation: `R, G, B` = `gea05_mpm.dds` channels, `D` = diffuse rgb times `cb1[1].rgb`, `N` = normal-mapped normal flipped for back faces,
`V` = unit vector to the camera, `L` = `-cb0[4]`, `H = normalize(V + L)`. With the AO option off (see Gaps) `ao = 1`.

```
hemi    = lerp(cb0[11], cb0[10], sat(0.5 N.y + 0.5))
horizon = 1 + cb1[2].w * (max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w) - 1)     ; cb1[2].w = instance byte 0xf / 255
direct  = horizon * R * sat(N.L) * cb0[3]  +  R * t11.x * hemi  +  R * point_lights(t3 grid)
s       = sat(cb1[2].y - 3.3333 * v4.w)          ; cb1[2].y = instance byte 0xd * 6 / 255; v4.w = vertex term (see Gaps)
Dm      = D + s * (D*D - D)
spec    = G * R * max(horizon, 0.25) * pow(sat(N.H), 32 B + 4 + 64 s) * cb0[1] * (1 + 4 s)
rimcol  = ((hemi - cb0[1]) * 0.5 + cb0[1]) / max_component(...)   * (0.75 D + 0.125)
rim     = sat( horizon * sat(1.2 * pow(sat(1 - N.V), e) - 0.2) * sat(N.y) * k * sat(2 R - 1) * rimcol )
color   = Dm * direct + spec + rim ; then fog (cb0[13], exp(-v5.w)) and underwater (cb0[14])
```

Meaning of the three `mpm` channels for this family (new for `SkinnedGeneral`, proven by the data flow above): **R** scales every
diffuse light term (sun, ambient, point lights) and gates the rim term (zero below 0.5), **G** scales the direct specular, **B** sets
the specular exponent `32 B + 4`. This is the same assignment `material-parameters-next.md` records for Rico's clothes.

The rim constants `e` and `k` (manager `0x013e0e28`, set by the constructor `0x00932250`, probe-verified from its `movss` stores):
`t = clamp((camera_distance / lod_scale - 4.2) / (40 - 4.2), 0, 1)`; `e = 1.5 / lerp(1.0, 3.0, t)` (so 1.5 near, 0.5 far);
`k = lerp(3.1, 10.0, t) * block[+0x30]`, and block `+0x30` is the **second header float, 1.0** for the canopy. The first header float
(10.0) is not read by any of the four draw functions (inferred unused). `lod_scale` is render-state `+0xcac`.

## Instance constants and shadow submission (board 540ac1d1, probe `research/tools/parachute_instance_defaults_probe.py`, 15 checks)

Closes Gaps 2 and 3 of the first version of this document and narrows Gaps 1 and 6.

**Where the constants come from (native-proven).** The draw function `0x009657c0` reads 16 bytes of a per-instance *draw record*. The record
is the sub-object at **+0x14 of the 0xcc-byte model instance** that `0x00a4f4b0` builds (the instance constructor does
`lea ecx,[esi+0x14]; call 0x009236d0`; instance fields are record fields plus 0x14: position at instance +0x34, matrices +0x58). The record
constructor `0x009236d0` writes the dwords `0, 0, 0xffffffff, 0xffffffff, 0, 0, 0` at record +0..+0x18, so by default:

| Record bytes | Default | Reaches the shader as |
|---|---|---|
| +8..+0xb | 255, 255, 255, 255 | `cb1[1] = (1, 1, 1, 1)` (diffuse multiplier and alpha; no colour change) |
| +0xc | overwritten at creation (below) | `cb1[2].x` (not used by the formula in this document) |
| +0xd | 255 (record ctor, and again `0x008909b0`) | `cb1[2].y = 255/255 * 6.0 = 6.0` |
| +0xe | 255 | `cb1[2].z = 1.0` |
| +0xf | 255 | `cb1[2].w = 1.0`: full horizon-shadow receive |
| +0x10..+0x13 | 0 | not read by the `SkinnedGeneral` non-0x42 branch |
| +0x14..+0x17 | 0 | `cb1[0] = (r*a, g*a, b*a, 1 - a)` with `a` = byte +0x17 = 0, so `cb1[0] = (0, 0, 0, 1)`: no tint |

`0x008909b0` (called from `0x008701b0` after every model-instance creation) then stores record byte +0xc = `(int)(x*127)` of a component of the
translation of the owner matrix (zero for the identity matrix the canopy owner starts with, `0x00601fb0`) and byte +0xd = 255, ORs the
instance flag word (+0xc) with `0x80` when its fourth argument is non-zero and clears `0x100` when its fifth is zero. The shared loader
`0x006af990` (the `CAnimatedModel` load that `CAnimatedParachute` reaches through vtable slots 4 and 5 `0x007a4e60`/`0x007d5260` and the canopy set-up
`0x00676860`; other callers `0x0070cc20`) passes `(1, 0, 0, 1)`, so every animated model, the canopy included, ends with flag `0x80` set and `0x100`
clear. The parachute set-up (`0x00676860`, `0x007fe910`: model name `weapons\gae05\GAE05_LOD1-GAE05` plus the extension string, state machine
`Parachute.afsm`, skeleton `Parachute.bsk`, joints `Para_LeftHandle`, `Para_RightHandle`, `lft_1`, `rgt_1`, `HarnessExt`) never calls a colour,
tint or receive setter. **Conclusion: the canopy runs with `cb1[0] = (0,0,0,1)`, `cb1[1] = (1,1,1,1)`, `cb1[2] = (c, 6.0, 1, 1)`.**

**The vertex term (Gap 3, solved).** In the vertex shader `skinnedgeneral` (disassembly 0038) the output `o4.w` is `v0.y`: the **raw model-space
vertex height** (v0 is the position input, before skinning; the rbm positions are in the skeleton's rest-pose frame, root at the harness).
So `s = sat(cb1[2].y - 3.3333 * v4.w) = sat(6.0 - 3.3333 * y)`: `s = 1` for `y <= 1.5`, falling linearly to 0 at `y = 1.8`, and `0` above.
The canopy's vertices span `y = 1.4747 .. 5.8364`: **1 624 of 1 748 vertices have `s = 0`** (every vertex of the cloth and the lines, all above 3.9
except the strap ends); 124 have `s > 0` (48 below 0.5, 64 between 0.5 and 1, 12 exactly 1), all of them the harness and strap pieces at the
bottom (uv `u 0.116..0.336`). Therefore for the canopy surface `Dm = D` (diffuse is **not** squared), the specular exponent is
**`32 B + 4`** (mean 8.1, as the first version stated) and the specular scale is 1; only the harness straps near the handles are squared
(`Dm = D*D`, exponent up to `32 B + 68`, scale up to 5). This is the same height ramp Rico's clothes use (his model origin is at his feet, so the
low part of his body gets the squared diffuse and the head does not).

**Trim colour (Gap 6, narrowed, still not closed).** The instance constants cannot make the trim red: colour is white, tint is zero. The trim
(texture `u 0.10 .. 0.20`, mean (84..119, 79..111, 72..100) of 255, used by 157 vertices at `y 4.2 .. 5.3`, i.e. the canopy edge band) is
stored as a warm brown/tan; `gea05_dif.dds` contains no red-dominant texel (0.0 % with `r > 1.5 g` and `r > 1.5 b`). Remaining candidates
for the red look in the reference shots, all **inferred, not proven**: warm sun/ambient/specular colour constants at the capture time of day
(`cb0[3]`, `cb0[10]`, `cb0[1]`), and the post tone-mapper's saturation/contrast mixing (`night-light-next.md`). An instance-constant
explanation is ruled out.

**Shadow submission (Gap 1, partly answered).** No per-instance shadow-caster gate exists in the draw path: the depth/shadow draws
(`0x009717c0`, and `0x009657c0` when the context flag `+0xd14 & 2` is set) test only the *block* (low byte 4 = blend skips; bit 18 skips unless
manager `+0x178`), the pass id (`+0xd0c`; `1` and `2` are the shadow cascades `CShadowPass` renders, `0x4f` and `0x50` the depth variants)
and the instance fade byte; no code reads the instance flags `0x80`/`0x100` as a caster switch in the decompiler exports (no reader found, so their
meaning stays unknown). The canopy instance is created by the same loader and with the same flags as every `CAnimatedModel`
instance. **Verdict (inferred, not run-verified): the canopy is submitted and cast like any other animated model; nothing in the retail code
suppresses it.** What was not traced: the list registration of an instance's blocks into the per-pass lists (`0x00a4daf0` stores the
deform/skinning metadata at instance +0x2c; the per-pass list insertion was not found) and the shadow-range culling for the pass.

## What this means for the reference shots

* **Day: dark grey-black with visible cell shading.** `D` is a dark neutral grey (fabric mean 45, 45, 44) and `R` averages 0.635
  (0.81 of the fabric texels have `R > 0.5`), so the sun and ambient reach only about 64 % of the already dark diffuse. The cell panels
  and their light stripes come from the diffuse texture and the normal map; specular is almost absent (mean `G` 0.042, exponent
  `32 B + 4` mean 8.1, so a broad faint sheen only on the few texels with higher `G`). The soft edge is the normal-mapped,
  double-sided closed mesh, plus the rim term at grazing angles.
* **Night over the base: lit only by ambient and spots, faint rib highlights.** The direct, ambient and point-light terms are all
  `R`-scaled and use the dark night constants, so the fabric stays dark. The rim term is the exception: its colour is
  *normalised* (divided by its largest component) times `0.75 D + 0.125`, so it does not shrink with the night ambient; it appears only on
  surfaces facing up (`sat(N.y)`) at grazing view angles and where `R > 0.5`. That reproduces faint highlights along cell edges. The
  correlation with the screenshots is by reasoning from the shader, not a pixel comparison.
* **Lines.** The line tubes are part of the same block and sample the near-black strip of the diffuse (`MPM` there: R 0.68, G 0.001,
  B 0.036), so they are lit by the same shader and stay near-black, double-sided, with essentially no specular. There is **no separate
  line material**.
* **Shadow on the ground.** The block is eligible for the depth/shadow passes, so the retail game can cast a canopy shadow when the
  object is submitted to them (inferred yes, see "Shadow submission"). The jungle reference shot is from a height where Rico's own shadow is the only one
  visible, which says nothing against casting: the shadow map only covers the near cascades.
* **Colour of the trim.** Not explained by the shader either: the stored trim strip is tan (mean 100, 94, 84). Reddish tones in the shots
  can come from `cb0[1]` specular colour at dusk, the warm sun/ambient colours, or a DLC variant (GAE18 diffuse is reddish); `cb1[1]` is white,
  so the instance colour is ruled out (see "Instance constants and shadow submission"). Left as Gap 3.

## Implementation recipe (paired task)

1. Material state: opaque, **double sided** (no culling in the main and shadow passes), no alpha blend, no alpha test, no
   emissive, no unlit flag. Textures: `gea05_dif` (sRGB colour), `gea05_nrm` (tangent-space normal, `2*rgb-1`), `gea05_mpm` (data, linear).
2. Lighting: port the formula above on top of the existing General/hemisphere route (`hemisphere_main.wgsl`, `general_main.wgsl`, light and
   fog buffers already carry `cb0[0,1,3,4,10,11,13,14]`, `t3`, `t11`). The skinned families currently run on `StandardMaterial` with only the
   hemisphere tilt (`hemisphere_material.rs`); this is a new `SkinnedGeneral` material with `R/G/B` roles above.
3. Rim term: constants 1.0/3.0, 3.1/10.0, 4.2/40.0, 1.5 as above; block `+0x30` = 1.0.
4. Until a native port lands, a labelled stopgap is a `StandardMaterial` with `cull_mode: None`, `double_sided: true`, base colour texture
   `gea05_dif`, normal map `gea05_nrm`, metallic 0, and a roughness derived from the mean exponent 8.1 (`sqrt(2 / (n + 2))` is about 0.45;
   inferred, non-native); do not set `unlit` or alpha.
5. Instance constants (native defaults, see "Instance constants and shadow submission"): `cb1[0] = (0,0,0,1)` (no tint), `cb1[1] = (1,1,1,1)`,
   `cb1[2] = (0, 6.0, 1, 1)`, so horizon-shadow receive is full and `s = sat(6 - 3.3333 * y)` with `y` the **model-space (rest-pose) vertex
   height**: `s = 0` for every vertex with `y >= 1.8` (the whole cloth and the lines), so use `Dm = D`, exponent `32 B + 4`, specular scale 1; only the
   124 harness/strap vertices below `y = 1.8` blend toward `D*D` / exponent `32 B + 68` / scale 5. A stopgap may ignore `s` entirely.
6. Shadow: cast the canopy mesh into the sun shadow pass with culling off like any other animated model (inferred from the shared loader and flags,
   not run-verified); keep it behind a flag.

## Gaps (unknown, labelled)

1. **Per-pass list registration and shadow-range cull of an animated-model instance** (partly answered, `0x00a4daf0` stores the deform metadata
   at instance +0x2c; the insertion of the instance's blocks into the per-pass lists and the meaning of instance flags `0x80`/`0x100`, no reader
   found, were not traced; the same applies to Rico). Not visible-blocking: the verdict that the canopy casts like every `CAnimatedModel` is inferred.
2. ~~Instance constants~~ **closed** by the section "Instance constants and shadow submission" (record constructor `0x009236d0`, probe).
   Not closed inside it: what *later* code (not the parachute set-up) may write into the record of an already-created canopy instance (a per-frame
   fade writer would change byte +0xb; none was found for animated models).
3. ~~Vertex term `v4.w`~~ **closed**: it is the raw model-space vertex y (vertex shader output `o4.w = v0.y`).
4. **AO / cascade option `cb4[3].x/.y`**: the two branches of the shadow-AO block (1-tap vs 12-tap blurred t12 sampling) are chosen by the
   user shadow option; the default for the shipped graphics settings is not known. The recipe above assumes the simplest path.
5. **First header float (10.0)** is not read by the four draw functions; its consumer, if any, is unknown (same in Rico's blocks).
6. **Red trim colour** in the reference shots: not produced by the instance constants (white, no tint) nor by the stored texture (tan/brown, no red  [Analysed in the section below: it is a local coloured point light in specific shots, not the material.]
   texel). Candidate causes (warm light constants at capture time, the post tone-mapper saturation, specular colour at dusk) are untested; the DLC canopy
   variants are out of scope.
7. **Record byte +0xc** (`cb1[2].x`): `0x008909b0` writes `(int)(component * 127)` of the owner matrix translation at creation; the shader
   family terms that read it (snow in General) are separate; unused by the `SkinnedGeneral` formula here.

## Trim colour gap (board 90918f6f, Gap 6): what the reference shots show

Method: survey of `reference/chute/` shots with the canopy in view (daylight `ck49`, `bl01`, night `z29`, `aa18`; the other file names
containing "red" in the index were checked by name only). No native address was newly traced in this pass; this is a reference-shot
analysis plus the lit formula above. Findings:

1. **Daylight: no red anywhere.** In `ck49` (jungle, sun) the trim at the cell mouths reads olive/tan, which is what the stored tan strip
   (mean 100, 94, 84) lit by a warm sun gives. In `bl01` (snow base, blue sky, red beacon mast beside the canopy) the whole canopy is
   neutral black-grey with blue sky tint on the underside. So no tone-mapper, saturation or instance tint turns the trim red by day.
2. **Night, red antennas close by (`z29`): still no red.** The canopy is grey-black and cold blue. Red beacons within about 50 m did not
   tint it. A global warm ambient or a global post mix is therefore excluded as the source of the red.
3. **Night `aa18`: red appears only on the right-hand cells and on the light stripes there**, fading to neutral dark towards the left,
   with a red mission marker beside the right wing tip. A one-sided, stripe-following red is the signature of a **local coloured point
   light** (point-light grid `t3`, R-scaled direct term) hitting the high-albedo stripe texels, not of a material or post effect. Inferred
   from the image; the light's identity (mission marker light, vehicle or beacon) is not established.
4. Consequence: the "red trim" of the earlier notes is a lighting effect of nearby coloured lights in specific shots, not a property of
   the canopy material, the instance constants (stored tan trim is right) or the grade. Nothing needs adding to the canopy material;
   parity follows from the point-light route once the shared `SkinnedGeneral` material samples the `t3` grid.


Gap 8 (from 9091): the exact light behind `aa18` (position, colour, source object) is unknown; it blocks only a pixel comparison of that shot, not the material.

## Runtime implementation (board 94315ae0)

`crates/jc2-game/src/skinned_general_material.rs` and `skinned_general_main.wgsl`: an `ExtendedMaterial` for the opaque double-sided
`SkinnedGeneral` blocks (header flags low byte 0 and bit 16, i.e. the canopy; Rico's blocks keep the hemisphere route). Bevy's lighting is
replaced by the native terms of "The lit colour": `mpm` R scales sun, hemisphere ambient (`sky - tilt (1 - N.y)`, times the terrain AO `t11.x`)
and the shared point-light grid, and gates the rim; G scales the specular; B sets the exponent `32 B + 4`; the rim uses the constructor constants;
fog is the native haze. The tangent-space normal map `gea05_nrm` is bound (back faces flip N via the front-face flag). Cull none in the main
pass; the canopy casts into the sun shadow pass (inferred; `JC2_CANOPY_NO_SHADOW=1` removes it). Instance constants are the native defaults;
`Dm = D`, specular scale 1 (the s = 0 cloth path); no red tint.

Inferred / not done: LOD scale `+0xcac` taken as 1; sun cascade floor `0.1 + 0.9 c` (as the General route); the `s` ramp for the 124 harness
vertices (needs the model-space height in a vertex override); underwater `cb0[14]`; AO option `cb4[3]` (simplest path).
Viewer note: `--model` cannot show the canopy (a skinned mesh without a skin component is a Bevy pipeline error, not this material); a capture
needs `--world X,Z --pose chute` with the canopy forced up.
