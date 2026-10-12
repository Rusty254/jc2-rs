# CGD static geometry and model batches: owners, draw distance and `.lod` slots (JC2-146)

Answers Gaps 7 and 8 of `location-activation-next.md` (JC2-119). Evidence: the Ghidra export
(function addresses below), the exe bytes, and a census of every `.cgd` (1885 files, 146 987
instances) and every `.batchlod` (193 files) in the retail containers. Verified by
`research/tools/cgd_model_batch_probe.py` (exit 0). Tags: **[P]** proven from bytes/export/census,
**[I]** inferred, otherwise see Gaps. Addresses are for `JustCause2.exe` (fingerprint in the probe).

## 1. Ownership

| Object | Class (RTTI) | Built by | Held by | Per-frame consumer |
|---|---|---|---|---|
| `.cgd` instances | `CModelCollection` (0x5c bytes, ctor `0x892ae0`, global list head `DAT_01191d30`) | `0x85b440` -> reader `0x8a40f0` | the `CLocation` at `+0x50` (`0x554110`, `0x73b800`) **[P]** | `0x8898d0`, called for every collection from `0x86e730` **[P]** |
| `.batchlod` files | `CModelBatchResource` = the `CModelBatch` object of `0xa52380` (0x5c bytes, vtable `0xf0f774`, global list `DAT_012ab4cc`) | resource type `0x21` in the chunk handler `0x8c21e0` (CResourceLoader slot 9 `0xf0c1e0`; CEntityResourceLoader overrides it with `0x8da340`, which forwards to it), and the creator `0x659be0` (vtable `0xeb52d8` slot 1) -> `0xa52cb0` | a vector of shared pointers at `CLocation+0x5c`; bit 0 of each object's byte `+0x58` is set by `0x554110` and cleared by the far-CLocation constructor `0x73b800` **[P]** | `CModelBatch` vtable slot 1 = `0xa4c6b0`, walked once per frame from the main update `0x781d60` over the whole global list (no per-location gate except that bit) **[P]** |

* `0x8d8a40` (CResourceLoader slot 7, the "finish" step) holds the raw bytes of the chunk stored by
  `0x8c21e0` case 7 and feeds them to `0x85b440`, so the `.cgd` is a chunk of the same resource
  stream as the `.batchlod` (case `0x21`) **[P]**. `.batchlod` is the file extension of resource kind
  `CModelBatchResource` (extension table `0x8acee0`, name table `0x63c8b0` -> id `0x21`).
* The earlier inference that `CModelBatchResource` contains/derives from `CModelBatch` is closed: the
  loader constructs exactly one `CModelBatch` (`0xa52380`) and wraps that pointer in the shared
  pointer whose control block is `sp_counted_impl_p<CModelBatchResource>` (`0x8a2830`) **[P]**.
  The CLocation's `+0x58` bit 0 and the update's `+0x58` bit 0 are the same byte.
* Near `.nl.cgd` is **not** a batch: every `.cgd`, near or far, is a `CModelCollection` of individual
  instances. Batches exist only in 28 far containers (the metropolis districts and bridges), 193 files **[P]**.
* The registries differ: `.cgd` instances register in the instance manager `DAT_012aaac0` (`0x908520`,
  bounds through `0x4f32d0`) and are drawn by the frustum callbacks `0x8e1cc0` (main view) and
  `0x8d8c00` (second pass) through `0x8c9100`; batch objects register in `DAT_012ab230` and are drawn
  only by the batch update **[P]**.

## 2. The `.cgd` fields (version 7 retail layout, reader `0x8a40f0`)

Header words after the magic and version: `count`, `models`, `strings`, four more words. The four
0x14-byte records** appended after the name table (668 of 1845 version-7 files; `leftover bytes ==
20 * word 6` for every one) **[P]**. The reader turns each record into a `u16` through `0xa93450`
(vegetation type lookup `0xa872c0`, patch insert `0xa91480`): baked vegetation, not static
models (format unresolved, Gap 1). Words 4 and 5 are element counts of two skipped streams (`*2`
and `*8` bytes; non-zero only for versions below 7). Versions 1..6 exist (40 files). Version below 5
has a 36-byte block before the matrices, version 5 and up 24 bytes (all zero in version 7).

Per instance, in file order (all proven by the reader and `0x8fc280` / `0x908520`):

| File field | Native meaning | Census |
|---|---|---|
| 64-byte matrix, `u16` model index | owner created by `0x8701b0` -> `0xa50a50(name, 0)` -> owner initialiser `0xa4f4b0` | as `world.md` |
| 12 "zero" bytes (`vec3`) | **distance anchor**: if non-zero, `0xa3f220` allocates a 12-byte position and stores it as the owner position; the camera distance of the instance is measured from it, not from the matrix translation | non-zero for 7 028 of 146 987 instances (427 files) |
| `f32` "-1.0" | **LOD scale override** stored raw at owner `+4` (the float `0xa4f4b0` initialises from the model's `.lod` scale, default 10.0). `-1.0` is the "none" sentinel (`0x8c9100`, `0xa42970`). It is applied only when the instance also has a non-zero anchor | -1.0 for 140 270; set (4..130, mostly 100/50/25/20/15/12) for exactly the 6 540 instances that have an anchor; 488 have an anchor without a scale; no scale without an anchor |
| `u16` flags ("3") | bit 0 -> owner flag `0x80`; bit 1 -> owner flag 2 (**distance cull**) plus `0x800` / not `0x1000`, and a fade-in registration `0x8da880(owner, 0, 0)`. Bit 1 clear (value 1 or 0): no distance cull, `0x1000` set | 3: 144 652 (+333 in v6); 1: 742 (gb036/key0xx base buildings, bridges); 2: 18 |
| 6 floats | world AABB, registered as the instance bounds with `0x4f32d0` (version 7 only; older versions take the model bounds) | as `world.md` |

Anchors are shared by a whole building group (all `gb028-*` of a `34x09` settlement share one anchor
and scale 70): the group switches LOD together **[P]** (census, `0xa3f220`).

## 3. Per-instance draw distance and slot for `.cgd` instances

`0x8c9100` is the shared draw gate of ordinary instances (`0x8e1cc0` passes `x = d^2` from the camera
to the owner position, i.e. the anchor if present; `0x8d8c00` the same for the second pass, only for
owners with bit `0x10`). It is the sibling of the vegetation selector `0xa89b90` and uses the same
constants (`0xf0ea0c` table, `0x108a228..238`, multiplier owner `+8 = 1 / (s F)^2`, `F = max(1,
camera+0x620)`, `s` = owner `+4` unless `-1.0`) **[P]**:

* skip if bit 1 (distance cull) is set and `d^2 * mult > table[count]`; `count` is the model's slot
  count, 5 for every `.lod` once a slot is loaded (vegetation doc). At `F = 1` the instance is culled
  beyond `sqrt(250000) * s = 500 * s` metres: **5000 m** for the default scale 10, 2000 m for 4, 12 500 m for
  25, 50 000 m for 100 (the far `.fl.cgd` structures use 55..130, i.e. never culled in practice);
* automatic LOD (owner bit 2, set by `0xa4f4b0`): continuous level and slot exactly as in
  `vegetation-lod-native.md` (slot boundaries `{10, 25, 50, 100} * s` m); the slot index lives in
  owner flag bits 29..31 and `0xa42740` submits that slot's meshes;
* with flag value 1 (cull off) the instance draws at any distance, still frustum-tested.

For jc2-rs this is the native replacement of a fixed `.cgd` distance: `distance = |camera - (anchor or
translation)|`, `scale = override if the anchor is present and the float is not -1.0, else the
`.lod` scale (sixth token, default 10)`, cull when flag bit 1 and `d > 500 * scale * F`.

## 4. `.batchlod` format and the three-tier update

Reader `0xa52980` (stream helpers `0xa515a0`, `0xa51a70`, `0xa51450`); all 193 retail files parse
completely to five trailing bytes **[P]** (probe):

```
u32 version (1 or 2; all retail 2)
batch object:  64-byte matrix, u32 length + name      (name is an .rbm path under ModelBatch\)
u32 N entries, each:
    entry object:  64-byte matrix, u32 length + name (the part .rbm), u32 K
    K records, each: 64-byte matrix, u32 length + name (a .lod),
        version 2: u32 flag; if non-zero: f32 scale, f32[3] anchor   (same override as the .cgd)
u32 M, M names (models of the parts)
5 bytes not read by 0xa52980 (Gap 3)
```

Each object is created with `0xa50a50(name, 0)`; its enabled and distance-cull bits are cleared, and
the AABB of the batch (`+0x10..+0x24`) and of each entry/record is computed by `0xa424b0` from the
model bounds and matrix. Retail: 707 entries, 5 074 records, record overrides with scales
4, 10, 16, 25, 50 (483 of them).

`0xa4c6b0` (every frame, only while bit 0 of `+0x58` is set), with `d^2` the squared distance from the
camera to the batch AABB (`0x4ebc90`), thresholds `122500 = 350^2` and `14400 = 120^2`
(`0x1089eb0/b4`, scale `0x1089eb8` = 1) **[P]** for the numbers, the tier reading **[I]** where noted:

1. **`d >= 350 m`:** the batch's own merged model is enabled and submitted for the main view
   (box frustum test `0x935c20`) and the three shadow passes (`0xa42740`). The batch object has no
   distance cull bit (cleared by the loader), so there is **no upper distance limit**: the model is
   visible while the far container is loaded. All entries are disabled (`0xa4bac0` -> `0xa4b3d0`
   clears each entry owner's enabled bit and propagates to its records).
2. **`d < 350 m`:** the merged model is disabled and each entry is tested (`0xa4c5a0` -> `0xa4bc90`
   with the record camera, 350^2, 120^2): entry owner enabled when the camera is at least 120 m from
   the entry's box (**[I]**: the box is the entry's AABB; the decompiler hides the call's `this`),
   submitted when frustum-visible; closer than 120 m the entry is disabled and its records are
   submitted individually through `0xa46320` (mode 0: main view, scale reset to the model scale unless
   the record has an anchor).
3. The hand-off to the near objects is therefore not a single 350 m switch: merged model
   (>= 350 m), per-part models (120..350 m), individual records (< 120 m). The previously assumed
   "exclusion list" does not exist: `+0x28` is the entry vector (0x2c bytes per entry) and
   `0xa4bac0` / `0xa4c5a0` are its two loop helpers **[P]**.

## 5. Which `.lod` slot a batch object shows (Gap 8)

Batch objects (batch, entry, record) are never passed to `0x8c9100` or to the vegetation selector
(`0xa89b90` has only the caller `0xa8c9a0`; `0x8c9100` only `0x8d8c00` / `0x8e1cc0`, which walk the
`DAT_012aaac0` registry that batches are not in) **[P]**. Their slot index is the owner flag bits
29..31 as written by `0xa4f4b0` (cleared to 0) and no batch code writes it, so submission
(`0xa42740`) uses **slot 0** for all of them; `0xa42740` falls forward to the first non-empty slot
only if slot 0 is empty. The merged `.rbm` objects have one level anyway. The level of detail of a
batch is the three-tier hand-off above, not the five-slot level **[P]** by absence of writers
(call graph checked in the probe; a runtime trace was not done).

## 6. Project handoff (not done here)

* `jc2-formats::cgd`: keep the byte layout; expose `anchor` (`Option<[f32;3]>` when non-zero),
  `lod_scale` (`Option<f32>` when not -1.0 and an anchor exists), `flags` bit meanings, the trailing
  record count and its 0x14-byte tail as opaque bytes (Gap 1).
* `jc2-game`: draw gate per instance as in section 3; far `.fl.cgd` instances with a 55..130
  scale are effectively unlimited, near `.nl.cgd` instances at the model scale (5000 m default).
  The batch-hide half of JC2-150: a `BatchTier` of `d_box` (point-to-AABB): merged model when
  `d_box >= 350`, entries when `120 <= d_entry_box < 350` (inside 350), records when `d_entry_box < 120`.
  Needs a `.batchlod` reader (section 4) and a loader for the `.rbm` objects under `ModelBatch\`.
  **Done in JC2-150:** `jc2_formats::batchlod` (reader, boxes, `BatchTier`), `scenery::place_batches`
  (batch, entry and record models spawned hidden, resolved from the far container) and `batch_lod.rs`
  (per-frame tier, camera = the fly camera). The boxes are the eight transformed `.rbm` header bounds
  (inferred for `0xa424b0`; records lie within 60 m of their entry's box in >= 90 % of retail entries).

## Gaps

1. **Trailing `.cgd` records.** Format of the 0x14-byte records after the name table (668 version-7
   files), the vegetation type lookup `0xa872c0` and patch insert `0xa91480`; whether they duplicate
   the procedural vegetation of the tile. Blocks: baked trees in location containers.
2. **Owner flags from the `.cgd` flag word.** Consumers of owner flag `0x80` (flag bit 0), of the
   `0x800` / `0x1000` pair, and the fade-in system `0x8da880` / `0x8d26f0` (manager byte `+0xdc`
   bit 1, camera check `0xa42b30`). Blocks: pop-in of `.cgd` instances and the meaning of the
   742 flag-1 and 18 flag-2 instances.
3. **`.batchlod` tail and parts table.** The five unread bytes, the `M` model names (stored at
   batch `+0x4c`, `0x9740d0`) and what consumes them.
4. **Batch pass tests.** `0xa46320` modes 1..3 and the per-pass distances of `0xa4bc90` (pass object
   `+0x1c8` shadow cascade distances), the entry box identity, and whether the record anchor/scale
   override is read by any batch path (records always use slot 0). Blocks: batch shadows and any
   record fade.
5. **`CLocation` reveal protocol.** Unchanged from JC2-119 Gap 5 (`DAT_011958c4 == 3`, who calls slot 7
   `0x554110`); it decides when the batch bit and the collection `+0x4c` bits are set.
6. **Frame budget.** `0x4869d0` (the time-slice test in `0x8898d0`) and the creation order of
   `.cgd` instances; blocks instance pop-in timing.
7. **No capture.** Distances are from bytes, not from a render of the original.

## 7. Merged mode-0 blocks are index ranges of the base model (city buildings, board 1ce45be5)

Cause of the bare city: the batch and part models (`*_batchN.rbm`, `*_partN.rbm`) hold only
Merged mode-0 instance lists and no vertices, and the renderer drew nothing for them, so at
350 m and beyond (merged tier) and in the part tier (120..350 m) no building existed. Native
draw of the Merged block (`0x9458a0`): per 0x34-byte instance the 3x4 matrix is set as shader
constants and `0x9263e0` draws with the two `u16` fields at +0x30/+0x32 as start index and
triangle count (its other caller passes start 0 and a triangle count). The block's `flags` word
`N` selects the buffers of the sibling `<stem>_base_N.rbm` (the base model's own `flags` is `N`).
**[P]** by census: for every instance of every metropolis far container, start + 3 * count never
exceeds the base model's index count (test `retail_merged_instances_are_index_ranges_of_their_base`,
`jc2_formats::rbm::expand_merged_instances`); the loader expands them into ordinary geometry.
Gaps: the second Merged block per model uses the same rule; the 16-bit index split and per-instance
vertex copy are a project implementation, not native. The record tier (< 120 m, `.lod` models)
was already working.
