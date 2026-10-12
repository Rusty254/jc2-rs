# Parachute canopy model, texture and lines (board 15c8aede, gaps G1 and G2 of parachute-parity.md)

Probe: `research/tools/parachute_canopy_model_probe.py --game <game folder> [--report]` (32 checks, exit 0).

## Result in one paragraph

The canopy is **not procedural and not missing**. Earlier notes (`character.md`, `parachute-canopy-landing-next.md` Gap 5) said
`weapons\gae05\GAE05_LOD1-GAE05` and the texture were not in the mounted archives. They are: `jc2tool scan-rbm` lists the file
(`general.blz/gae05_lod1-gae05.rbm`, 82 919 bytes) as a member but prints no `model` line for it, because `scan-rbm` prints a
`model` line only when one of the model's *textures* matches the substring, and the textures are spelled **`gea05`** (letters
swapped, a retail typo), so a search for `gae05` or `chute` never showed the model. `jc2tool rbm <game> general.blz/gae05_lod1-gae05.rbm`
and `scan-rbm --names gea05` show it. One mesh holds everything that is drawn:
the ram-air canopy with its cells and ribs, the suspension lines, the harness straps and the two brake handles, all skinned to the
17 bones of `parachute.bsk`. The runtime's loft and cylinder lines (`parachute.rs` `canopy_mesh`, `DEPTH`, `LINE_SPREAD`) can be
replaced by this model.

## Where the game gets it (native-proven)

| Fact | Evidence |
|---|---|
| The retail parachute entry names the model | `settings\parachuteparams.bin` entry `parachute00`: `model = weapons\gae05\GAE05_LOD1-GAE05`, skeleton `parachute.bsk`; string at exe `0x00e42bc0`; the entry parser `0x0067d340` reads key `model` (hash `0x0f94740b`, global `0x00eb285c`) |
| The model file | `general.blz/gae05_lod1-gae05.rbm`, RBMDL 1.13.0, one `SkinnedGeneral` v3 block, footer ok, parsed to the last byte |
| LOD file | `general.blz/gae05-gae05.lod` lists the same `.rbm` five times: there is no lower level |
| Textures (block material) | `gea05_dif.dds`, `gea05_nrm.dds`, `gea05_mpm.dds` in `general.blz`, each DXT1 512x256, 10 mips, 87 528 bytes. Remaining five material slots empty, material flags 3 |
| Other model facts | 1748 vertices, 2129 triangles (6387 indices, one skin batch), flags `0x10000` (not "big" vertices), params `10.0, 1, 1, 1, 1, 1`; bbox x -2.993..2.991, y 1.475..5.836, z -1.482..1.577 (span 5.98 m, depth 3.06 m, in the skeleton's rest-pose frame, root at the harness) |
| Skin | 4 byte weights (sum 252..255) and 4 batch-local bone indices per vertex; the batch bone table holds the 15 skeleton bones 2..16 (everything except `Reference` and `base`) plus three 0xffff pads. The existing `rbm.rs` `parse_skinned_general` already resolves it |
| Bind pose = skeleton reference pose | rest-pose `Para_RightHandle` (0.306, 1.511, -0.355) is within 0.15 m of the centroid of the vertices it dominates; `rgt_1`/`lft_1` rest y 1.951 equals the lowest edge of their vertices and of the line tubes. So inverse bind matrices are the inverse of the `parachute.bsk` reference pose (no extra bind data) |

## What the mesh contains (read from the data)

The faces split into 71 unwelded pieces (vertex counts in brackets):

* **Canopy surface**: one piece (346 vertices) with UV u 0.25..0.99, v 0..1 (the top skin with the emblem), plus the cell side/end pieces
  (75, 75, 58, 57 vertices at the wing tips, and 16 pieces of 12 vertices around the rib lines; the exact cell/rib roles of these pieces are inferred from their positions) and nine pieces of 15 vertices on the
  trailing edge (UV u 0.107..0.214, the tan trim strip). Nine cells between ten ribs (the lines attach at four x positions per side,
  |x| about 0.27, 0.87, 1.60, 2.56 m, plus the tips at 2.99 m). Skinned mostly to `mid_4`, `rgt_4`, `lft_4` (the tip chain bones),
  `rgt_1`/`lft_1` and lighter on `_2`/`_3`.
* **Lines**: 16 tube pieces (8 of 27 vertices, 8 of 33), 8 per side, 4 in front of and 4 behind the canopy centre plane. They start
  at y 1.95..1.96 next to `rgt_1`/`lft_1` (the shoulder strap points, x 0.11..0.27) and end on the canopy underside at y 3.90..5.04.
  Every line vertex samples one thin strip of the diffuse (u 0.231..0.245), whose colour is near black (mean RGB about 22).
  This matches the reference shots (dark thin lines, about 7 per side visible from the front, the others hidden behind them).
* **Harness and handles**: the `HarnessExt` strap pieces (4-vertex strips, 16, 19, 56-vertex pieces, y 1.52..1.96, |x| < 0.23) and one
  22-vertex piece per handle (skinned to `Para_RightHandle`/`Para_LeftHandle`, the brake grips at (+-0.306, 1.51, -0.355)); two more
  8-vertex pieces per side link them. These are the "brake lines to the handles" and the "small harness strap at the shoulders" of
  the reference shots.

So the runtime's "3 chains x 2 offsets as white unlit cylinders" is wrong in count (16 line tubes plus handles and straps), in
colour (black strip of the diffuse, lit) and in geometry.

## Texture (`gea05_dif.dds`, decoded)

Dark neutral grey fabric (mean RGB about 45, 45, 44) with seven pill-shaped cell panels each carrying a light vertical stripe
(faint blue-grey in the reference shots), an emblem (a scorpion) across the middle cells, a tan
strip (mean about 100, 94, 84; trim of the 15-vertex pieces), a black strip (lines) and two small light patches at the bottom (corner
tabs). The canopy surface UVs are one island (u 0.25..0.99, v 0..1); the lines and trim use the left quarter of the atlas. Normal map
mean RGB (131, 110, 254) looks tangent-space; `mpm` mean (163, 9, 31).
The red trim seen in the reference shots is **not** the tan strip colour as stored; it is unexplained by the diffuse alone (see Gaps
3). The DLC variants differ only in these textures (GAE18 diffuse mean (58, 34, 32), reddish).

## DLC canopies (same layout)

Each DLC parachute pack carries `N_parachute.bin` and `N_parachute_location.blz` with a model of the same shape and the same
skeleton (names hashed in the archives, readable with `jc2tool prop <game> N_parachute.bin`):

| Pack | item_id | model | 
|---|---|---|
| 13 | parachute02 | `DLC\Weapons\GAE16\GAE16_LOD1-GAE16` |
| 14 | parachute03 | `DLC\Weapons\GAE18\GAE18_LOD1-GAE18` |
| 15 | parachute04 | `DLC\Weapons\GAE19\GAE19_LOD1-GAE19` |
| 16 | parachute05 | `DLC\Weapons\GAE20\GAE20_LOD1-GAE20` |
| 17 | parachute06 | `DLC\Weapons\GAE21\GAE21_LOD1-GAE21` |
| 18 | parachute07 | `DLC\Weapons\GAE22\GAE22_LOD1-GAE22` |

Each is 1748 vertices, 1 block, 1832 triangles (the probe asserts this for all six); the DLC models are a reordered mesh with fewer
triangles (the retail gae05 has 2129). `parachute-thruster-item.md` line 58 says 14 is the only `N_parachute.bin`: it is the only
one with a plain name in the archive listing, the others are present under hashed names. The retail game has no way to equip a DLC
canopy in this project (no DLC selection in scope), so only gae05 matters for parity.

## Cloth

`CAnimatedParachute` (`0x007e0250`) and the canopy setup `0x00676860` build the object from the entry: model, skeleton, animation set,
state machine. Nothing in the data requires cloth: the surface is skinned to the bones, and `chute_open_chute.ban`, `chute_*_tb_*.ban`
animate those bones (the deploy shape with the canopy "crumpled" in `canopy-deploying-*` shots is the opening clip, not a physics
build). Whether the engine adds any runtime deformation on top is **unknown** (no `hkFxCloth` use was traced to the canopy).

## Gaps (unknown, labelled)

1. Shader state of the block: params `10.0, 1, 1, 1, 1, 1` (GAE18: third value 0.5), double-sidedness, alpha/blend. Owner: shading task
   28d8e587 (R2). The mesh is closed (top and bottom skins), so culling is unlikely to matter for the canopy; the lines are thin tubes.
2. Meaning of the `mpm` channels and whether the normal map is tangent space. R2 28d8e587.
3. Red trim colour and night look of the reference shots versus the stored diffuse: could be the lit result of the `mpm`/trim material,
   or a different variant; R2.
4. Whether any cloth or physics deformation is applied at runtime on top of the skin: not seen in the data.
5. Deploy bag and collapse look: not in the mesh (60caf984, R3).
6. `mc01_parachute_dif`, named by `mc01_rico.cdoll`, is absent from every archive; it is not what the canopy samples (`gea05_dif`) and
   the character texture list names it only for the doll. Not needed for parity of the canopy.

## Implementation (paired task)

Replace `canopy_mesh()`, the line cylinders and `CANOPY_COLOUR` in `crates/jc2-game/src/parachute.rs` with the skinned model:
load `general.blz/gae05_lod1-gae05.rbm` through the existing RBM loader (`block_to_mesh`, joints/weights resolved already), the three
textures through the project's texture policy, a skinned mesh whose joints are the 17 `parachute.bsk` bones in the existing canopy
rig, inverse bind = inverse reference pose; drive bone local transforms from the clip as `ChuteRig::bone_positions` does now.

## Implemented (97bf7030)

`parachute.rs` `attach_canopy_model` loads the rbm through `RbmLoader` (so the texture policy and the General route apply),
spawns the 17 `parachute.bsk` joints with `rig::spawn_joints` (reference pose, inverse bind = its inverse) and a `SkinnedMesh`
for the block; `animate_canopy` writes the clip or five-clip blend local transforms onto the joints. Capture
(`_extract/97bf7030/after3.png`, forced canopy at 0.4 scale for framing): the ram-air canopy shows its cells and ribs, dark grey fabric
with light cell stripes and tan trim at the cell mouths, thin dark lines to the harness and handles. Not verified: emblem
visibility from below, red trim (Gap 3). The old loft, white cylinder lines and flat colour are gone.
