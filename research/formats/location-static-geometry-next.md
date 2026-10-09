# Location static geometry: what supplies the bases (JC2-111)

Evidence brief, 9 October 2026. Probe: `research/tools/location_static_geometry_probe.py`
(exe `df99a479…b74f80`; input SHA-256s of `world.bin`, `conditions.bin`, `km01.base.nlz/.flz`,
`31x14.nlz/.flz` in its JSON output). Runtime: `jc2-formats::location`,
`jc2-game::locations`, `scenery::place_container_statics`, `world::far_companion`.

## Summary

JC2-085's review saw almost none of the start base's buildings. There were two
separate causes, and both are now fixed in the runtime:

1. **Wrong tile.** The start base, Kem Gunung Dataran Tinggi, is not tile 31,14.
   `settlements.bin` names it `km01.base` (text `km01_location_txt_01`). Its icon is at
   (−1052.7, 1625.3, −7010.7), tile 29,18.
   - Its geometry is the world location `locations\keymissions\km01\km01.base.nlz/.flz`.
     It is not a `wc` tile container. Tiles 28..30 × 17..18 have no `wc` container at all.
   - Tile 31,14 is a different military base, Lembah Genting Tinggi (`31x14`, 836 m).
   - The reference daylight shots (`world\daylight-base-on-snowy-ridge-…_br01`,
     `base-buildings-17-numeral-…_br33`) show the 1600–1800 m ridge fortress. That fortress
     is km01.base.
   - jc2-rs streamed only `wc` tiles and `metropolis\` registry locations. It never
     loaded `keymissions\…` or any of the other ~380 non-city world locations.
2. **The far container's `.fl.cgd` was never read.** Every location has a near (`.nlz`)
   and a far (`.flz`) container. The far one carries its own compiled static geometry.
   That geometry holds the large structures:
   - km01.base: 246 instances of 47 models, including the `key036-a/b/c/d/h` fortress
     blocks (`key036_lod1-a.rbm`, 660 KB, the largest model in the base), the `gb066`,
     `gb222` and `gb223` kits, `go022` and the rocks.
   - 31x14: 81 instances, including the `gb223` building kit (22), `gb093`, `gb222` and
     `gb030`, the `go098-a` shaft cover over the hole in the terrain, and the six
     `arctic_g0?_rocksm` rocks. These are the "six `.pfi` rocks without a visual" that
     `world.md` had left unresolved.
   - None of these models appears in the near `.blo` or the near `.cgd`. Their `.lod`
     descriptors and full-detail meshes are in the near container; the far container
     ships only the low LODs (`key036_lod3-a.rbm`, `gb066_lod2-*`). Their collision
     instances are already in the near `.pfi` (`key036_lod1-a_col.pfx` ×3 = the three
     `key036-a` instances).

## Native chain

All of the following is verified in the pseudo-C and checked by the probe:

* **Registry.** `CGameWorld::PostInitializeSystem` (`0x0067ce40`) calls `FUN_007c4840`.
  That function reads `locations\world.bin`; for every node it reads:
  - `file` (`DAT_00eb8a50`);
  - the lower bound `#7df7f859` (`DAT_00eb8a68`);
  - the upper bound `#2b5ef194` (`DAT_00eb8a6c`);
  - an optional margin `#1070a35f` (`DAT_00eb8a70`, default 0).

  It then registers the location through vtable +0x90, `FUN_0060e7d0`. The registry
  holds 1238 locations: 817 `wc`, 39 `keymissions`, 137 `employers`, 124
  `generalinterests`, 45 `factionmissions`, 36 `metropolis`, 23 `roads`, 9
  `strongholds`, 6 `factiontasks` and 2 `challenge`.
* **Two records per location** (`FUN_0060e7d0`, 40-byte records: name, min at +4, flags
  at +0x10, enabled byte at +0x14, max at +0x18):
  - **far** (flags `8`): the raw box, grown by the margin (`FUN_008d92a0` →
  - **near** (flags `0xc`): the raw box **inset by 1000 m on all three axes**
    (`FUN_008bd7e0`; skipped when the box would be empty).

  Both records start enabled (bit 4 of +0x14).
* **Record → CLocation.** `FUN_00830560` builds a `CLocation` (`FUN_006eb550` →
  `FUN_0073b800`), passing `near = (flags & 0xc) == 0xc`. It also sets the location
  centre from the record box and the path `"Locations/" + name`.
  - `CLocation`'s byte +0xc0 holds that flag. `FUN_005f4780` appends `.nl` when it is
    set and `.fl` when it is not (`DAT_00e306b8` / `DAT_00e2fbb8`).
  - Only the far `CLocation` walks a `CModelBatchResource` list (`FUN_0073b800`,
    destructor `FUN_00687110`).
  - `FUN_0075da40` (vtable +0x8c) finds both records by name; the near one is returned
    first.
* **Conditions.** `FUN_005ddcc0` reads the conditions file (named by settings key
  `#0b4605c0`; `locations\conditions.bin` is the only file of that schema). Each node
  has `name`, `enable_event`, `disable_event` and `#6a0d2ca3`, which is read with a
  default of 1 (`DAT_00eb8a64`). A 0 clears the enabled bit of both records, and the
  events toggle it.
  - Exactly five locations start disabled: `km07.submarine`,
    `km07.submarine.animation`, `km04.sheldonscamp`, `f2t02.rooftop` and
    `f3t08.road`.
  - km01.base is not among them, so it is an ordinary, always-enabled world location.
* **What the containers hold** (`jc2tool cgd`/`sarc`, probe):

  | Location | near `.cgd` | far `.cgd` | shared models | far models without a near descriptor |
  |---|---|---|---|---|
  | km01.base | 280 / 41 models | 246 / 47 models | none | `gb006-a/d`, `gb219-a/b/c` (resolve from the shared/areaset sets) |
  | 31x14 | 100 / 22 | 81 / 27 | none | none |

  The far `.fl.blo` holds stripped stubs (418 bytes for km01.base). The near `.nl.blo`
  of km01.base has 5094 objects, 469 of them visible models; the 661 hidden ones are
  `_dst` states.

## What jc2-rs did, and what it does now

| | Before | Now |
|---|---|---|
| Non-city registry locations | ignored (`locations.rs` admitted `metropolis/` only) | every location that is not `wc/` and is enabled at start (`conditions.bin`) is requested by each loaded tile its **near box** (raw − 1000 m, `Location::near_bounds`) intersects, as a geometry-only `LocationOwner::Scenery` |
| City locations | raw-box intersection, `Tile` owner (gameplay) | near-box intersection, `Tile` owner (unchanged ownership, gameplay/settlement containers still excluded) |
| `.fl.cgd` | never read | `place_container_statics` places the far companion's CGD with the near one, for tiles and registry locations alike; models resolve near → shared sets → far |
| Disabled-at-start locations | n/a | skipped (no event system enables them yet) |

Tests (real data, `JC2_TEST_DIR`):

* `location::tests::retail_start_base_is_an_unconditional_registry_location`: checks
  1238 locations, km01.base's near box, that tile 29,18 is in the near box and 31,14
  is not, and the 5 conditions.
* `locations::tests::retail_start_base_streams_as_scenery_geometry`: tile 29,18
  registers km01.base with 995 objects (280 near + 246 far CGD instances and 469 `.blo`
  models), 0 unresolved and no actors.
* `scenery::tests::dresses_a_tile_headless`: tile 31,14 now has 181 statics
  (100 + 81).

Captures (git-ignored, `_extract/jc2-111/`, 12:00, free camera):

* `before_31x14.png` / `after_31x14.png`: `--world 31,14`, `JC2_CAM_YAW=67`,
  `JC2_CAM_PITCH=-35`. Before, only props stand around an open pit. After, the gb223
  buildings, the paved yard and the octagonal shaft cover appear.
* `before_29x18.png`: the old binary at the start base's tile. No base.
* `after_km01_close.png` (`--world 29,18`, `JC2_CAM_HEIGHT=420`, `JC2_CAM_PITCH=-28`)
  and `after_km01.png` (`--world 29,19`, `JC2_CAM_HEIGHT=1100`, `JC2_CAM_PITCH=-25`):
  the ridge fortress with its towers and red-and-white tanks, as in the reference shots
  `br01` and `br33`.
  - The old binary has no `JC2_CAM_HEIGHT`, so it cannot take the same aerial view.
    It draws no km01.base geometry from any position.

## Gaps

* **Per-frame activation test.** `FUN_00584d50` calls `FUN_00830560` for records
  taken from the queues of `FUN_00763470`, `FUN_0060ac80` and `FUN_007ed240`. Not
  decoded: which point is tested (camera, player or vehicle), the containment rule
  (the boxes are 3-D), hysteresis and unload order.
  - jc2-rs keeps its project policy: inclusive XZ overlap between the near box and a
    loaded terrain tile's square.
  - Blocks exact pop-in and range parity.
* **Far-only range.** The far record's box contains the near one, so natively the far
  `CLocation` (raw box, `.flz`, low LODs) should be loaded between the near and far
  boxes; this follows from the record boxes, since the activation test is not decoded.
  - jc2-rs draws the `.fl.cgd` only while the near location is loaded, and never
    loads the `.flz`'s own meshes.
  - Blocks distant silhouettes of bases beyond the near box.
* **LOD switching for CGD instances.** Not decoded: which `.lod` level the engine
  draws at which distance, and how the near and far containers' meshes for the same
  model are chosen. The `CModelBatchResource` handling in `FUN_0073b800` and
  `FUN_00687110` is not decoded either. jc2-rs uses the configured `--lod` level.
* **Margin key type.** `#1070a35f` is read by `FUN_004a1f60` and passed as a float to
  `FUN_008d92a0`. The world.bin values are −75, 100, −40 and 100, on the city gameplay,
  `b2.roads` and `f1m07milehigh` locations. Not proven: whether it is converted from
  int, and its sign convention. It does not apply to km01.base or to any tile, and
  jc2-rs does not use it.
* **Actors of non-city registry locations.** km01.base's soldiers, SAM sites,
  vehicles, triggers and mission objects, and those of the other ~380 locations, are
  not activated: the `Scenery` owner is geometry only.
  - Natively they are live, gated by their own events.
  - Blocks base gameplay parity. It needs per-object event gating (`enable_event` /
    `disable_event`) before it can be enabled safely.
* **Conditional locations.** The five disabled-at-start locations need the mission
  event system (`msy.*.activated` and so on) to enable them.
* **Name of `#0b4605c0`.** The settings file that names `conditions.bin` was not
  opened.
