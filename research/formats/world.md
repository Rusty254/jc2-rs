# World content: location containers and placements

Implemented in `crates/jc2-formats/src/placement.rs` (placements), `cgd.rs` (static
geometry) and `vegetation.rs` (the procedural trees, bushes and rocks); `jc2tool
placements / cgd / vegetation` list them, `jc2 --world X,Z` draws them. Companion to
`terrain.md`.

## Location containers

Every terrain tile that has content owns two zlib-wrapped SARC containers:

| File | What is inside |
|---|---|
| `locations\wc\XX\XXxZZ.nlz` (821 of them, "near") | the tile's models (`.rbm`, `.lod`, textures), collision shapes (`.pfx`), `XXxZZ.nl.blo` placements, `XXxZZ.nl.cgd` static geometry (below), `XXxZZ.pfs` (PFX-wrapped Havok packfile: static collision for the tile), `XXxZZ.pfi` (`"PFI\0"`, u32 1, u32 2, u32 count, then per instance a 4×3 matrix, a translation, a hash and the `.pfx` path), `XXxZZ.obc` (u32 1, u32 count, then per box 19 floats: half extents, then a 4 × 4 row-vector matrix with the centre in the last row — 8 occluder boxes around the village's big buildings in `31x14`), `.aiteam` blackboards |
| `locations\wc\XX\XXxZZ.flz` (814, "far") | low-detail models, a `.fl.blo` whose objects are stripped stubs (no transforms), and a `XXxZZ.fl.cgd`: the location's large structures (`31x14`: 81 instances, the `gb223` building kit, the shaft cover `go098-a`, the six `.pfi` rocks), whose descriptors and full meshes sit in the `.nlz` (`location-static-geometry-next.md`) |

Container members are keyed by bare file name; references inside the placement file
use full engine paths (`structures\general_structures\props\gp001\gp001-b.lod`) and
resolve by the file-name part. What a tile does not ship itself comes from the shared
containers:

* **`general.blz`** (`global\areasets\`, 2223 entries: 707 models, 468 `.lod`, the
  `g_*` textures, the global grass and meta-material textures) — always loaded.
  `31x14.nlz` resolves every one of its 112 visible models with it (99 local + 13 lamp
  posts).
* **`areasetNN.blz`** (15 regional kits, 0.2–11 MB each): which one a tile uses is
  painted in `areasets.bmp`, a 64 × 64 8-bit BMP with one pixel per tile (column = x,
  row from the top = z); `areasets.bin` maps each colour to a set (`folder` =
  container name, keys `#66dd312b/#745eb2e4/#e91487dc` = r, g, b). Most of the map is
  areaset05; areaset09 is a 2 × 2-tile bunker. Verified on the tiles whose references
  miss both their own container and `general.blz` (22,1 / 17,2 / 58,5 → areaset08;
  25,6 → areaset04). 45,9 still wants two `gb017` models found only in areaset03/07,
  so `jc2-game` falls back to the other sets after the painted one.

Objects are not strictly cut at tile borders: `31x14.nlz` holds a mountain village in
x ∈ [−576, −380), z ∈ [−8941, −8768) (tile 31,14) *and* a second cluster at
x ≈ −940, z ≈ −8500 (tile 30,15, which has no container of its own).

## Placement file (`XXxZZ.nl.blo`)

An ordinary property container (see `property.rs`): the root is a list of nodes, each
object is a node with `_class`; objects nest (a `CGeometryObject` building contains its
`CSimpleRigidObject` parts, `CDamageControl`, effect emitters, AI cover nodes…).

| Key | Meaning |
|---|---|
| `_class` | engine class (`CSimpleRigidObject`, `CGeometryObject`, `CDamageableObject`, `CSpawnSet`, `CAICoverGameObject`, `CDynamicLightObject`, `CEffectPointEmitter`, `CShapeTrigger`, …) |
| `name` | designer name (`sro.roof.2.dst`, `geo.tower`) |
| `world` | `mat4x3`: three basis rows then the translation, row-vector convention (`p' = p·M + t`); absolute on root objects, relative to the parent below that |
| `filename` | `.lod` descriptor (occasionally a `.rbm`) to draw |
| `collision` | `.pfx` Havok shape |
| `visible` | 0 on parts that only appear later: the `_dst` destroyed-state pieces of damageable buildings (134 of the 246 model-bearing objects in `31x14.nlz`) |
| `dynamic`, `fixed`, `health`, `invulnerable`, `save`, `fade_out`, `grappling_weight_category`, `_object_id`, `_instance_id` | physics / damage / persistence parameters, not used yet |

Engine axes are right-handed and the same as Bevy's: x east, y up, z south; a model faces
−z with +x on its right. `jc2-game` takes placements as they are: the basis rows become
the Bevy matrix's columns (`coords::mat_to_bevy`, `coordinates.md`).

## Checks

* `jc2tool placements <game> 31x14.nlz --models`: 1374 objects, 246 with models.
* Ground classes (`CAICoverGameObject`, `CSpawnSet`, `CDamageableObject`) of the village
  sit on the terrain height field with a 0.4 m median error.
* `cargo test -p jc2-game` with `JC2_TEST_DIR` pointing at a folder whose dropzone has
  the five terrain tiles, `31x14.nlz` and `general.blz` streams the tile in headless:
  112 objects placed, 50 863 triangles, 277 collision shapes (167 from the `.pfi`, 110
  from the objects' own `collision` entries), nothing unresolved — then
  moves the camera three tiles away and checks the tile and its entities are gone.
  `scenery::tests::dresses_a_tile_headless` adds the 100 + 81 static-geometry instances
  (`.nl.cgd` + `.fl.cgd`) and
  the 652 trees + 3100 bushes/rocks with their 3752 colliders, all resolved and spawned.

## Streaming (in jc2-game)

`world::Streamer` keeps the tiles within `--radius` (Chebyshev distance in tile indices)
of the focus loaded: the driven vehicle if there is one, else the camera. Each frame it
drops the tiles further than `radius + 1` (hysteresis, so a tile border isn't a
load/unload flicker) and loads the nearest missing tile — one per frame, since a tile is
a terrain mesh and collider, up to a few hundred collision shapes and the model requests
(the models themselves arrive through the asset server over the next frames). Shared
between tiles: the open containers (the tile's own is forgotten on unload), model handles
reference-counted by path so a prop used by two tiles is loaded once, and the converted
`.pfx` collider sets. The sea plane follows the focus tile, and the `SpawnPoint` for the
car appears once the start tile has loaded. A tile at 4 m cells is 128 × 128 cells, so a
radius of 1 keeps 1.5 km of ground in every direction; the global 64 m height map would
do for the horizon beyond that (not drawn yet).

### What the objects without a `filename` would draw

Of `31x14`'s 1374 objects, 246 carry `filename`. The rest, by class: `CGeometryObject`
(64) bodies are in the `.cgd` (below); `CDamageableObject` (24) has `filename` plus
`#3736ef24`, the same `.lod` again (the damaged state, presumably); `CSAMSite` (2) names
its launcher parts with seven hashed keys (`#f9dcf6ab` … `weapons\wea31\wea31-a.lod`,
`-b`, `-c_dst`, `-e`) — the only models in the file not placed through `filename`,
not drawn yet; `CResourceItem` (8) names a pickup container (`#f25a8a76 =
exported\pickups\pickup.boost.weapon.ee`) and `CSpawnSet`/`CVehicleComponent` a vehicle
(`#fa726895 = vehicles\lave\lave.v044truck.ee`) — gameplay spawns; `CDynamicLightObject`
(42), `CEffectPointEmitter` (82), `CParticleObject` (6) are lights and effects; the rest
(`CAICoverGameObject`, `CShapeTrigger`, `CTimeTrigger`, `CDialogue`, `CForcePulse`,
`CRoadObject`, …) are invisible logic.

## Static geometry (`XXxZZ.nl.cgd`)

`crates/jc2-formats/src/cgd.rs`; `jc2tool cgd <game> 31x14.nlz`. The props a location
draws that have no object of their own in the `.blo`: fences, poles, crates, the bodies
of `CGeometryObject`s whose doors and parts *are* in the `.blo`. `31x14.nl.cgd` has 100
instances of 22 models (`go173-*` fences, `gp070/gp071` crates, `go063-e` lamp posts,
`gp001-a` the health-locker body whose door `gp001-b` and medicine kit `gp001-c` sit in
the `.blo` at the same spot); none of its 22 models appears in the `.blo` and the
`.pfi` collision list carries the same shapes at the same places.

```
u32   0x83030314       magic
u32   7                version
u32   count            instances
u32   models           distinct .lod names
u32   strings          size of the name table at the end
u8    [40]             zero in 1177 of 1845 version-7 files; the 4th word of the 10 is the count of
                       trailing 0x14-byte records after the name table (JC2-146)
f32   [count × 16]     world matrix per instance, 4 × 4, row vectors, translation in the
                       last row (absolute, same convention as the .blo)
u16   [count]          model index into the name table
u8    [count × 12]     distance anchor vec3 (zero = none; cgd-model-batch-next.md)
f32   [count]          −1.0 = no LOD-scale override (the metre scale applies only with an anchor)
u16   [count]          flags: bit 0 -> owner flag 0x80, bit 1 -> distance cull + fade-in (3 for 144 652)
f32   [count × 6]      world-space AABB min, max
char  [strings]        NUL-separated bare .lod names
```

`jc2-game` (`scenery.rs`) places them through the tile's pending list like the
placements; the collision is already there from the `.pfi`. The far container's
`XXxZZ.fl.cgd` (same format) is placed with it: it carries the big structures that
neither the `.blo` nor the near `.cgd` names (JC2-111, `location-static-geometry-next.md`).

## Vegetation

There is no list of trees anywhere: the engine *generates* them. `CVegetationManager`
(`FUN_00a85eb0` is the per-patch generator) grows the trees, bushes and rocks of every
64 m patch from the terrain material map with a fixed-seed random generator, so every
machine sees the same forest. `crates/jc2-formats/src/vegetation.rs` re-implements it;
`jc2tool vegetation <game>` lists the zones, sets and types, `jc2tool vegetation <game>
31 14 [--list]` the plants of a tile; `jc2-game` grows every streamed tile
(`scenery.rs`, `--no-vegetation`, `--vegetation-lod`).

Inputs (all property files, all keys hashed — the key names below are ours):

| File | Content |
|---|---|
| `climate\num_zones.bin` | `#837b8c6a` = 4 climate zones |
| `textures\landscape\climatezonemap.dds` | 64 × 64 A8R8G8B8, one texel per tile (`[tz × 64 + tx]`), red = zone of the tile. `texturezonemap.dds` next to it is the same thing for the terrain textures (not used yet) |
| `climate\zone_N\vegetation_types_N.bin` (`FUN_00a979a0`) | root `#a653deb7`, one node per plant type: `name`, `#cfa2e112` model (`Arctic_T20_PineS-Whole.lod`, bare, in `climate\zone_N\vegetation_N.blz`), `#6a510b0f` stump `.lod`, `#efa84452` trunk pieces (`;`-separated), `#5955dcde` slope range in degrees (stored as cosines quantised to 0.001, see the loader's `(int)(cos × 1000) × 0.001`), `#853892e8` height range in metres, `#a3809ad6` noise frequency, `#b282def7` coverage (≥ 1 grows everywhere), `#ad1572e6` exclusion radius, `#5272e003` has collision, `#6803c8c0` breakable, `#bf2041b7` align to ground (set on rocks), `#87eeb199` sway factor, `billboard` (`#7f0240b5`), and a dozen unnamed ints |
| `climate\zone_N\vegetation_set_N.bin` (`FUN_00a917a0`) | `set_K` nodes: `name` (`Arctic_PineForest`), `#5c326dec` list of entries `{ type, group = tree/bush/grass, #d8a03db0 probability, #df8d2697 count }`. Each set becomes three 64-slot tables (tree at +0x34, bush at +0xb4, grass at +0x134 of a 0x1b8-byte set): an entry takes `count + ⌊probability × 64⌋` slots in order, the rest stay empty (0xffff), then each table is shuffled with `rand()` seeded 3 (one sequence across the three tables: `for i in 0..64: swap(t[i], t[(rand >> 16) & 63])`). Every set is loaded three times: as is, as `name_notrees` (grass entries only) and as `name_noveg` (empty) |
| `climate\terrain_materials.bin` | 66 materials, each with `Material` (`#89ddcd69`, the index byte used in the tile material maps) and `#259a5bb8` the vegetation set name; `FUN_00a89b30` resolves the name through the zones in order (the first zone that has a set of that name wins) |
| `climate\zone_N\grass_types_N.bin` | the grass types (below); `leaf_types_N.bin` (213 bytes, nothing in it) and `climate\common\treeimpostors.bin` (tree impostors) |

128 m = 2 × 2 of the same 64 m patches, table `0x00f20980` = `1, 2, 4 … 512`):

* Seed `(z / 4) << 13 + x / 4` from the patch origin (`int` truncation, arithmetic
  shift), stepped with the MSVC LCG `seed × 214013 + 2531011`; `rand()` is bits 16..30.
* **Trees**: 4 × 4 cells of 16 m, per cell two draws: `x = cell + rand1 × 12/32767`,
  `z = cell + rand2 × 12/32767`, yaw `= rand2/32767 × 2π`, table slot `(rand1 >> 16) & 63`.
* **Bushes and rocks** (level 6 only): the same seed again, 15 × 15 cells of 4 m,
  skipping every cell whose two indices are both multiples of 4; `cell + rand × 3`,
  slot and yaw as above; a candidate is dropped when a circle of its type's radius
  overlaps one already placed in a window of the patch's bookkeeping array (the trees
  sit in rows 0..3 of that array, indexed by their cell, the bushes by theirs — the
  window is `rows i−4..=min(i,15)`, columns likewise, with the raw loop counters).
* The ground query (`FUN_00a5f600` → `FUN_00a56400`, a bicubic sample of the tile) gives
  the height, the normal and the material at the cell `floor(local / 4) + 2`; the
  material's set must belong to the patch's climate zone (or zone 0) and the slot
  must not be empty.
* A type is accepted when `slope_cos_min ≤ normal.y ≤ slope_cos_max`,
  `height_min ≤ y ≤ height_max` and, when coverage < 1,
  `(fBm(⌊x⌋ × f, ⌊z⌋ × f) + 1) / 2 ≤ coverage` (`FUN_00a7c3a0` is the same test again for
  bushes). The fBm is Musgrave's over Perlin's reference `noise2` (`CNoise`,
  `FUN_004d4d90`: gradient/permutation tables built with the engine's `rand()`, seed 3;
  `CfBmFractal` H 0.1, lacunarity 2.3, 3 octaves).
* The instance matrix (`FUN_00a85820` + `FUN_004e1470`): yaw about y; types that align to
  the ground (rocks) add `pitch = atan((nz·cos(−yaw) − nx·sin(−yaw)) / ny)`,
  `roll = atan(−(nx·cos(−yaw) + nz·sin(−yaw)) / ny)`, composed row-vector style
  (`Instance::world`). At most 256 instances per patch (`CVegetationModelPatch`).
* Grass is a third table used by a separate 8 m generator (next section).
* The four zones share one type list (`vegetation_types_0/1/2` are identical, zone 3's
  has 69 of the 101) but a zone container only ships the models its own sets use, and
  sets of zone 0 are allowed on every tile (`zone == 0` passes the zone test): the city
  zone's sets are jungle sets whose trees are in `vegetation_0.blz`, and the arctic
  border tile 38,12 grows jungle plants on its `Rain_Forest` cells. `jc2-game` looks a
  type's `.lod` up in its zone container first, then the other zones'; on a full install
  all 96 type models are in one of the four.

Verified: `rand()` reproduces MSVC's `41, 18467, 6334`; the fixture's
`Arctic_PineForest` gives the table counts the engine's rule predicts (22 slots of
`Arctic_T20_PineS` = ⌊0.35 × 64⌋, 1 empty tree slot, 4 empty bush slots); material
`0x41` (`default_arctic`) maps to zone 1 set 0, `0x52` (`road_arctic_asphalt`) to none;
the rock type's flags, radius 4.6, coverage 0.3 and slope cosine 0.707 read back; tile
31,14 grows 652 trees and 3100 bushes/rocks, deterministic per patch, all inside the
tile and inside their types' height ranges; the jungle tile 40,40 641 trees and 4818
bushes/rocks (`Jungle_T01_CanopyM`, `MossyRocks`…), the desert tile 10,50 70 and 894
(`Desert_T0x_NeedleBush`, beach rocks), the city-zone tile 8,24 534 and 4382 (jungle
plants, kelp and corals under the sea). Assumed, not verified against the running
game (no way to dump its instances here): that our bicubic-vs-bilinear height and
central-difference normal pass exactly the same candidates at the slope/coverage
margins; that the "+8 ≠ 0" test on a type in the generator is "model loaded" (it is not
set by the type loader). The six hand-placed `arctic_g0?_rocksm-whole_col.pfx`
instances in `31x14.pfi` (identity rotation, village ground level) are drawn by the far
container's `31x14.fl.cgd` (3 × `arctic_g01_rocksm-whole`, 3 × `arctic_g02_rocksm-whole`;
JC2-111).

### Grass

Done in `crates/jc2-formats/src/grass.rs` (data and generator) and `crates/jc2-game/src/grass.rs`
(drawing); `jc2tool vegetation <game>` lists each set's grass entries with their types.

**Where it is in the exe.** `UpdateVegetationGrass` (`FUN_00a8c880`) keeps a 13 × 13 ring of
for a patch and `FUN_00a87090` → `FUN_00a82620` generates it into a dynamic vertex buffer of
`CRenderBlockGrass` (shaders `Grass_Lod0/1/2`). `FUN_00a97250` loads
`climate\zone_%d\grass_types_%d.bin` (156-byte records) and `FUN_00a950d0` one record.

**Types** (`grass_types_N.bin`: `#fbdb7d47` = `global_grass_DIF.dds`, `#d99a4e2d` = the list):
28 types in the jungle zones (`Grass_Thin`, `Jungle_Grass_Leafs_Red`, `Lawn_Grass_Thick`,
`Poppy_Field_Grass_Thick`, `Rice_Field`…), 1 in the arctic (`Grass_Arctic_00`), 2 in the
desert. Keys read by `FUN_00a950d0` (the hashes are the exe's constants at `0x00f20a5c…`; the
names are ours except where the data has them): `name`, `width` / `height` (`#11b73cc0`,
`#d2cbd704`: the quad's half width and its height before the size factor), `#30f8f3e1
#4a584a79 #d83854c9 #090d5ee6` = `u0 v0 u1 v1`, the type's rectangle in the atlas,
`#146014af` / `#2ca49d2e` = size factor min / max (0.3..1.5), `#f873e6d4` = lean (0.3..0.8),
`#87eeb199` = sway (0.01..0.1), `#2ead7fc9` / `#33f3ef7b` = the slope range's maximum and
minimum in degrees, `#8643db23` = how far the blade follows the ground normal (1 in all),
`#755a3e9e` a file name for a custom blade shape (empty in all 28: every type is the plain
quad), `billboard` (1). The atlas `general.blz\global_grass_dif.dds` is 1024 × 512 DXT5 with
alpha, 2 columns × 4 rows of 512 × 128 cells (the rectangles above are those cells: thin
grass, thick grass, leafy plants with red leaves, shrubs, dry grass, a bare bush, tall
flowers, a poppy meadow).

**Sets.** A set's grass entries (`group = "grass"`, `type` = a *grass* type's name, `#df8d2697`
= slots out of the table's 64; `probability` is 0) fill the third table, shuffled with the
others (the existing `build_tables`); `Rain_Forest` = 12 `Jungle_Grass_Thin`, 26
`Jungle_Grass_Leafs`, 26 `Jungle_Grass_Leafs_Red`. The material under the cell picks the set
(`terrain_materials.bin`, `Vegetation::materials`); the set's zone is the material's, not the
tile's climate zone. Of the materials that name a set, 25 have none (asphalt, pavement, bare
rock…: no grass) and 30 grow some. `Slope_Rocks` (desert) grows dry shrub.

**Generator** (per 8 m patch, origin `(px, pz)` a multiple of 8 from −16384):

* seed `(int)(pz × 435 + px × 231)`, the MSVC LCG; **four passes** of 16 cells each (the exe
  draws as many passes as its density setting asks for); in pass `p`, row `r` (1 m, 0..7) the
  columns are `c0 = ((r & 3) > 1) + p + 2r` mod 4 and `c0 + 4`: 64 cells per patch, each once.
* per cell six draws, always all six: (a) slot, (b) a phase byte (`200 + b mod 56`, the
  shader's sway phase), (c) size lerp 0..1, (d) yaw `±2π`, (e, f) position
  `cell + (2 r − 1) × 0.5`. The cell's origin is its corner, so a blade sits within ±0.5 m of it.
* the ground query gives height, normal and material; the height must be above 200 (the sea);
  the material's zone must have grass types; slot `a mod count` (count = the zone's grass
  types, not the table size), then the following slots in turn (count tries, wrapping) until a
  non-empty one is accepted by the slope test: with `m_x = |sin n.x|`, `m_z = |sin n.z|` (the
  sine of the normal's *components*, by the exe's parabola approximation),
  `max(m_x, m_z) ≤ slope_max` **and** `slope_min ≤ min(m_x, m_z)` — a type with a minimum of
  4° does not grow on level ground, only on slopes tilted in both directions; the lawn, field
  and road-edge types have minimum 0.
* a blade is **three quads**, each rotated by the next 120° from the yaw; a quad is
  `2 × width × size` wide and `height × size` tall (`size = lerp(min, max, c)`); its four
  vertices `(vertical 0|1, across −1|+1)` carry the atlas rectangle's corners (vertical 1 =
  `v0`, the top of the cell); the top is sheared out of the plane by `height × lean × cos 45°`
  (the exe tilts the quad by 45° and applies `lean`) and follows the ground normal's horizontal
  part; **every vertex's y is the terrain height at its own x, z plus 0.05 plus the vertical
  offset**, so blades bend with the ground. The vertices also carry the ground normal (the
  shader lights the grass with it), a sway weight (`sway × 255 × vertical`) and the phase byte.

**Drawing here.** The 8 m patches are grouped into 32 m blocks, one static mesh each, built
within `--grass-radius` (80 m by default; the exe's ring is ±52 m) of the camera, a few per
frame (≤ 4 ms of building a frame, well under a millisecond a block) and dropped behind it;
the ground comes from the terrain tile files, parsed once per tile near the camera. The
material is StandardMaterial lighting with `AlphaToCoverage`, a distance fade (60 % to 100 % of
the radius) and a vertex-shader sway (`grass.wgsl`). One draw call per block, no entity per
blade. GPU instancing in the strict sense does not apply, because no two blades have the same
vertex positions: the exe's buffer is built the same way. A jungle tile's 16 384 cells give
about 9 000 blades at full density (`--grass-density 1`; 0.5 draws two of the four passes).
Not done: the exe's `Grass_Lod1/2` shaders (fewer, bigger quads with distance), the tint by
the ground colour and the cloud shadows.

### Tree impostors

`crates/jc2-formats/src/impostors.rs` (data), `crates/jc2-game/src/impostors.rs` +
`impostor.wgsl` (drawing). The exe's `CTreeImpostorTopRB` / `CTreeImpostorTrunkRB` (shaders
`TreeImpostorTop`, `TreeSingleImpostorTop`, `…Trunk`, `…Shadow`, `…Snow`) draw every tree
beyond its model range as two cards.

* `climate\common\treeimpostors.bin`: `#94095ab4` / `#c7ff282e` / `#f1afb0d7` = the atlases
  `TrunkAtlas_NRM.dds`, `TrunkAtlas_DIF.dds` (1024 × 512 DXT5) and `TopAtlas_DIF.dds`
  (2048 × 1024 DXT5); two lists of entries: `#567238eb` the trunks and `#f325bb6e` the crowns
  (tops). An entry: `id`, `#190d9496` (0 for the palm and pine crowns, which are *views from
  above* and lie flat; 1 for the others, side views that face the camera), `offset` (centre
  height above the tree's foot, m; only `y`), `#755a3e9e` the `.tm` polygon name,
  `#b5b709ad` the atlas rectangle `u0 v0 u1 v1` (v down), `#b97abf28` the card's size (m).
  34 ids; ids 20..22 (rock pillars) have no crown, 30/31 (bananas) no trunk; a few entries are
  placeholders (size 0). The atlases' alpha is a random 0.5..1 noise inside the shapes
  (meant for the shaders' dithering), a cutoff of 0.4 gives solid silhouettes.
* a vegetation type's `billboard` key (`VegetationType::billboard`) is its id (−1 for rocks,
  stumps, kelp, most bushes: 34 types with one, e.g. `Jungle_T01_CanopyM` = 0,
  `Arctic_T21_PineM` = 10, `jungle_T14_palmCLS` = 6).
* `climate\common\impostor_{top,trunk}_lod1-<name>.bin`: the `.tm` polygon of each entry, a
  five-point hull inside the cell (keys `#5f4716e4` / `#a3a1a9d1` = min / max uv, `#f9e7e959` the
  points, `#ebdf4cc8` the indices; the same layout as a custom grass shape); a fill-rate
  saver, not used here (the alpha does the same job).

**Drawing here.** Trees within 520–580 m are models (`scenery::TREE_RANGE`, a
`VisibilityRange` on the model, 380–440 m for bushes, rocks and trees without an impostor);
beyond that and out over every tile within `--impostor-radius` (5 by default) the trees are
cards, from the same generator (`Vegetation::generate_tile_trees`: the trees of a patch come out
the same without the bushes). A tile is built on a worker thread, tens of ms (read, parse,
grow, mesh), into one crown mesh and one trunk mesh per 128 m patch; a patch fades in
between 450 and 600 m (`VisibilityRange`, crossfading with the models). Each card is stored as
one centre, the tree's yaw and four corner offsets; the vertex shader builds it: upright and
facing the camera (trunks, side-view crowns) or lying on the ground, turned by the yaw (palm and
pine crowns). StandardMaterial lighting with a normal pointing up and the atlas in `Mask(0.4)`.
A jungle tile has 400–650 trees: 1 300 cards, 5 000 vertices to a tile.
`JC2_IMPOSTOR_FADE=0,1` shows the cards at any distance. Not done: the polygons, the normal
map of the trunks, the snow variant, shadows, the exe's second kind (`TreeSingleImpostorTop`,
for lone trees).

## Vehicle spawn sets

A `CSpawnSet` in the `.blo` with a `CVehicleComponent` child parks a vehicle: the
component's `#fa726895` names the container (`vehicles\arve\arve.v060_attackheli.ee`),
the set's `world` is where, `spawn_distance` (100–300 m) how close the player comes before
it appears, `disable_event`/`#61db11cc` the mission events that switch it off, `faction`,
`grappling_weight_category`, `invulnerable`. A `CPlacementComponent` child with
`SSpawnPoint`s gives the exact points (identity on the village's). Tile 31,14 parks six:
the attack helicopter, two `v044truck`s, a `v016_military_apc`, a `v050_large_super_truck`
and a `v017_military_jeep` (`jc2_formats::placement::vehicle_spawns`). Other spawn sets
carry `CTeamComponent`s (soldier patrols and guards), `CExportedEntityComponent`s
(weapon stashes) and pickups (`#f25a8a76 = exported\pickups\pickup.boost.cash.ee`).

`jc2-game/src/fleet.rs` requests each tile's vehicles when it loads and drops them when
it unloads (unless the player is in one); the doll decides car, helicopter or boat.

## Destruction

`CDamageControl` nodes make objects destructible: `health` (and `max_health`),
`invulnerable`, and `on_destroy` (`#138b2a05`), the event they fire — a trigger or an
emitter and an event name, `t{gb010.10x10({036206DB-…})}::destroyed` or
`e{cov.redbarrel.b({B35CCC29-…})}::fire`. Objects react to events by their own strings:
the intact parts carry `#c7ca479b` (hide on: `…::switch` on the buildings, whose trigger
turns `destroyed` into `switch`; `…::destroyed` on the barrels), the debris placed with
`visible = 0` carries `#f68f92d4` (show on) or sits under the damage control, with
`dynamic = 1`, `fade_out = …::fade` and an `SPhysicsParams` child (`mass`, `drag`,
`angular_damping`). `#1b9f7a6c` names the damaged object (`sro.roof{GUID}/…`),
`#8e4014ef` the debris, `#57c810a5` the fade event. Effect emitters switch with
`event_enable` / `event_disable` (`epe.fire`, `epe.explosion`).

Damage controls nest for stages: the red barrel's outer one (16 health) fires `::fire`,
the inner (70) `::destroyed`, which enables `epe.explosion` and shows two halves
(`go001-b.lod`, 50 kg each). The village's watch tower (`dac.tower`, 450) hides its
corner plates and throws 18 roof and platform pieces; the antenna (`dac.antennaDestroyed`,
300) swaps `go043j_on`. `CDamageableObject`s (the `dao` fences, `go164`, 50–200 health)
are a second, simpler kind: one object with its own `health`, not handled yet.

`jc2_formats::destruction::destructibles` groups each outermost damage control with its
nested ones (summed health), the intact placements that go (the named target and every
listener on its events, same root) and the debris; tile 31,14 has 17 units.
`jc2-game/src/destruction.rs` tags the intact models and their collision with `Piece`,
takes bullet hits and blasts off their health, and at zero despawns them and drops the
debris as dynamic bodies with their masses; units with an explosion emitter blow up
(7 m, 350 damage — the effect graph is not decoded), which sets off their neighbours.

## Still unknown

* Grass's LOD shaders and ground tint, leaf particles,
  the single-tree impostor kind (`TreeSingleImpostorTop`), the
  breakable-tree pieces (`stump`, `trunks`), the sway animation.
* The trigger objects themselves (what turns `destroyed` into `switch`, counts, timers),
  `CDamageableObject`, fading the debris, the fire stage, `CConstraint` hinges.
