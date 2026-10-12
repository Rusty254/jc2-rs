# Native location activation, far-only range and batch handling (JC2-119)

Evidence brief, 9 October 2026. Retail `JustCause2.exe` SHA-256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Probe (read-only, exit 0
only if every check passes; absolute paths on Windows):

```powershell
python research/tools/location_activation_probe.py --export <repo>/research/analysis/pseudo/_unnamed `
  --tool <repo>/target/release/jc2tool.exe <game dir>
```

The probe pins the SHA-256 of 14 hand-decoded functions, checks Ghidra-export fragments and
call-graph facts, reads `locations\world.bin` through `jc2tool prop`, and runs a model of the
rule over all 1238 registry locations. Follows the accepted `location-static-geometry-next.md`
(JC2-111), whose Gaps 1-3 this answers. Tags: **[P]** proven from bytes/export, **[I]**
inferred, otherwise see Gaps.

## Summary

* The activation test uses the **main camera's world position** (not the player or a vehicle),
  tested in **XZ only** (the boxes' Y range is never compared), strictly inside the record box
  **shrunk by 10 m** to start a load, and strictly inside the **raw box (no margin)** to keep it.
  The 10 m band is the only hysteresis.
* Each location has a far record (raw box, `.flz`/`.fl`) and a near record (raw box inset by
  1000 m on every axis, `.nlz`/`.nl`). Camera inside the near box: both load, near first.
  Camera in the 1000 m ring between near and raw box: **only the far container** is loaded.
  Beyond the raw box: nothing. So "far-only" is a 1000 m ring, not a long-range LOD.
* Loading is serialised: one spatial record is started at a time, only when nothing else is in
  flight; unloads are one at a time and block new loads. Candidates that overlap are started
  near-before-far, then the one whose nearest box face is farthest from the camera.
* `.cgd` instances do not use the vegetation selector `0xa89b90` (its only caller is the
  vegetation patch callback `0xa8c9a0`). The CGD loader hands its model names to the shared
  `.lod` factory; the per-frame draw gate of a batch is a different function (`0xa4c6b0`).

## 1. The activation test

### Native chain **[P]** (all in `JustCause2.exe`)

| Step | Address | What it does |
|---|---|---|
| Owner | `0x7fe0b0` | `CGameWorld` constructor; it constructs the location manager as the **sub-object at `this+0x58`** (`lea ecx,[esi+0x58]` then `0x5e21f0`). Called "T" below; `CGameWorld+0xbc` is T+0x64. |
| Manager constructor | `0x5e21f0` | Zeroes six 12-byte lists at T+0x04, 0x14, 0x24, 0x34, 0x44, 0x54 (begin, end, capacity) and T+0x60; T+0x64 = (old & 0x8c) \| 0x0c; **T+0x68, T+0x6c, T+0x70 = 10.0** (`0x41200000`, loaded from `0xe251e8`); reserves 16 entries for the vector at T+0x24 (`0x642af0`) and for the pointer list at T+0x50 (`0x6f90e0`). |
| Frame entry | `0x5df830` (CGameWorld vtable `0xeb9224` slot `0x98`); `0x7439f0` (slot `0x94`) repeats it with extra work | Skips the whole pass if bit 0 of T+0x64 is set or `0x545490` is non-zero (it only drains a destroy-request list at T+0x44 and returns 0). Otherwise `0x520070` (poll), then `0x7f7440(pos)` (unload); only if that returns 0, `0x584d50(pos)` (load). `0x640760` is the adjuster thunk (`add ecx,0x58; jmp 0x584d50`) in vtable slot `0x84`. |
| Position | callers `0x55da90`, `0x568700`, `0x663ff0`, `0x6bb920`, `0x748ab0`, `0x7bdbe0`, `0x7faf40` (slot `0x98`); `0x781d60` (slot `0x94`) | All pass a pointer to three floats read from **`*(DAT_011919f4 + 0x10) + 0x54/+0x58/+0x5c`**, or (0,0,0) if the camera manager does not exist. `DAT_011919f4` is `NGraphicsEngine::CCameraManager` (constructor `0x92e130`), `+0x10` its main camera, and `+0x54` the translation row of the camera matrix **[I]** (rows `+0x34/+0x44` are up/forward in `water-consumers-next.md`; the batch update `0xa4c6b0` and the tether range use the same three floats as the camera position). |
| Loader pass | `0x584d50` | Runs only if the in-flight list (T+0x54) is not full (size != capacity; capacity 16). Drains `0x763470` (vector T+0x24) and `0x60ac80` (vector T+0x04) into `0x830560`: records with `(flags & 3) == 0`, `+0x14 & 1`, `+0x14 & 4`, no CLocation yet and not already in flight; these are the non-spatial ("Global/" and named) records. Then, only if bit 0x10 of T+0x64 is set **and the in-flight list is empty**, calls `0x7ed240(pos)` and starts **at most one** spatial record. Finally, if bit 0x40 is set it is replaced by 0x10. |
| Enable | `0x618340` | Called from `CGameWorld::PostInitializeSystem` (`0x67ce40`); sets bit 0x40, i.e. spatial loading starts after the first pass. (Bit 0x10 is clear after the constructor.) |
| Spatial query | `0x7ed240` | Scans the registry vector (T+0x14, 40-byte records) **from the last record to the first**. A record is a candidate if: no CLocation (`+0x24 == 0`), `(flags & 3) == 0`, `+0x14 & 4` (enabled), it is a near record (flags>>2 == -1) and T+0x64 has bit 8, or a far record (== -2) and bit 4 (both set by the constructor), it is not in the in-flight list, and the camera satisfies `min.x + 10 < x < max.x - 10` and `min.z + 10 < z < max.z - 10` (T+0x68 / T+0x70; strict `comiss`/`jbe`). **Y is never read** (the probe checks the code contains no access to the record's Y fields). |
| Choice | `0x620510` (stdcall: best so far, candidate, position) with `0xa30ab0` | Near beats far: best near and candidate far keeps the best; best far and candidate near takes the candidate. Otherwise the record with the **larger** value of `0xa30ab0` wins, ties go to the candidate. `0xa30ab0` is the minimum, over x, y and z and both faces, of |camera - face| (SSE, constant `-0.0` at `0xe2599c` makes it an abs): the distance to the nearest of the six face planes (3-D, so the camera's Y does enter the tie-break). |
| Start | `0x830560` | Builds the CLocation (`0x6eb550` -> `0x73b800`, `near = (flags & 0xc) == 0xc`), centre from the record box, path `"Locations/" + name` opened as `.nl` (near) or `.fl` (far) by `0x5f4780`; sets the record's CLocation (`+0x24`) and inserts it into the in-flight list. On open failure it sets flags bit 0 and drops it. |
| Poll | `0x520070` | Each pass: if no unload is pending, takes the **front** of the in-flight list, calls CLocation vtable `+0x10`, and when the state (`+0x0c`) is **4** clears `+0x14` bit 0, moves the record out of the in-flight list (front erase, and `0x920500` appends it to a second list **[I]**) and posts the events `LocationFinishedLoading` and the location name. If an unload is pending (T+0x60 != 0) it waits while the CLocation state is **5**, then clears `+0x24`, `+0x14` bit 1, T+0x60 and releases the CLocation. |
| Unload | `0x7f7440` | Returns 1 (blocking `0x584d50`) if T+0x60 is set. Otherwise, for each in-flight record that is spatial (`flags & 0xc`): if `+0x14` bit 1 is set or the camera is **not strictly inside the raw box in XZ with no margin** (`x <= min.x \|\| z <= min.z \|\| max.x <= x \|\| max.z <= z`), erase it from the list, set T+0x60 and call CLocation vtable `+8` (release); return 1. If none, scan the second list (T+0x34) **backwards** for the last record that is disabled or no longer contains the camera, and do the same without erasing. One unload per call. |
| Capacity | `0x7c4840` | Before registering it reserves `2 * node count + 16` records (`0x642af0`); each registration appends two records (far, then near) via vtable `+0x90` = `0x60e7d0`, which refuses to grow beyond capacity - 2. Registration does not filter by name: the 817 `wc` tile containers are in the same vector as bases and roads. |

Proven by byte: the call sequence and operands of `0x584d50` (two drain loops, one query),
the `[ebx+0x68]` / `[ebx+0x70]` additions and subtractions in `0x7ed240`, the near-over-far
branches and `f(a) > f(b)` selector in `0x620510`, the constructor stores, the vtable slots, and
the thunk. Pinned SHA-256 of all of them is in the probe.

### What this means for a camera

| Event | Rule |
|---|---|
| Start loading a record | camera XZ strictly inside the record box shrunk by 10 m; no in-flight load; no unload pending; bit 0x10 set |
| Order among eligible records | near before far, then largest nearest-face distance (the camera's Y counts), ties to the earlier record |
| Keep | camera XZ strictly inside the **raw** record box |
| Unload | first record (in-flight list order, then last-loaded first) that fails Keep; one at a time; nothing new starts until its CLocation has left state 5 |
| Hysteresis | 10 m on every side, no time delay, no distance beyond the box |

## 2. Far-only range **[P by rule, I for what the far container draws]**

Per location the two records are the **far** one (raw box +/- margin; margin key
`#1070a35f`, only four locations have one: -75, -40, 100, 100) and the **near** one (raw
inset by 1000 m; kept as the raw box only if the inset box would be empty, which is true for
none of the 1238 locations: the probe counts 1238 non-empty insets). Consequences with the
rule above:

| Camera XZ | Loaded |
|---|---|
| inside near box (shrunk 10 m) | near `.nl` **and** far `.fl` (near started first) |
| between near and raw box (the 1000 m ring) | far `.fl` only |
| outside the raw box | none; far unloads when the camera leaves the raw box |
| inside the raw box shrunk by 10 m but within 10 m of the edge | far only, with the 10 m band as hysteresis |

The far container is the same kind of CLocation as the near one, opened with `.fl`; the
only code that distinguishes them is the near flag in `0x73b800` (and see section 3). The
far `.fl.cgd` is therefore drawn in the ring exactly as the near one is drawn inside the
near box; there is no larger "silhouette" range natively. For km01.base the ring is x in
(-3160.6, -2160.6) and (-329.0, 671.0), z in (-8906.7, -7906.7) and (-6194.7, -5194.7). The
other 817 `wc` tiles and all other locations use the same two records, so a loaded
terrain tile is not what natively decides container loading: a camera that stays inside one
location's near box also holds the neighbouring locations whose boxes overlap it (the model
loads 30 records around km01's centre and 36 around tile 31,14).

### Fixtures (metres; the probe prints them all)

| | km01.base (`keymissions\km01\km01.base`, tile 29,18) | `wc\31\31x14` (contains terrain tile 31,14) |
|---|---|---|
| raw (far) box x | -3170.632 .. 680.955 | -2452.979 .. 1173.927 |
| raw (far) box z | -8916.654 .. -5184.733 | -10466.992 .. -6944.185 |
| raw size x, z | 3851.6, 3731.9 | 3626.9, 3522.8 |
| near box x (keep) | -2170.632 .. -319.045 | -1452.979 .. 173.927 |
| near box z (keep) | -7916.654 .. -6184.733 | -9466.992 .. -7944.185 |
| near size x, z | 1851.6, 1731.9 | 1626.9, 1522.8 |
| far starts loading | x in (-3160.632, 670.955), z in (-8906.654, -5194.733) | x in (-2442.979, 1163.927), z in (-10456.992, -6954.185) |
| near starts loading | x in (-2160.632, -329.045), z in (-7906.654, -6194.733) | x in (-1442.979, 163.927), z in (-9456.992, -7954.185) |
| Y range of raw box (never tested) | -254.2 .. 4884.9 | -720.0 .. 4142.8 |

Terrain tile 31,14 is [-512, 0] x [-9216, -8704] (`TILE_SIZE` 512, origin -16384): it lies
entirely inside the 31x14 near load window, so any camera standing in that tile loads the
31x14 near container (and far). The start base's tile 29,18 is [-1536, -1024] x [-7168, -6656],
inside the km01.base near window x (-2160.6, -329.0), z (-7906.7, -6194.7).

## 3. Per-instance `.cgd` LOD and model-batch handling

### Proven

* **The CGD stream.** `0x8a40f0` checks the magic `0x83030314` (the world.md header), reads a
  version (1..7 take different field sets), allocates per-instance arrays, and resolves every
  distinct model name through `0xa50a50(name, 4)`. `0xa50a50` is the shared "load `.lod`
  descriptor and build a TLodModel" factory (twelve callers), which calls the owner initialiser
  `0xa4f4b0` (`vegetation-lod-native.md`). The reader's only caller is `0x85b440`, which first
  allocates a 0x5c-byte object into `this+0x50` (constructor `0x892ae0`, vtable `0xf0c6ac`, next to
  `CSingle<IMatrixProxyFactory>` at `0xf0c6a4`). The class that owns this reader is **not**
  identified (Gap 7); in particular it is not proven to be `CModelBatchResource`.
* **No vegetation selector.** `0xa89b90` (slot selection and fade) has exactly one caller,
  `0xa8c9a0`, the vegetation patch callback (its address is only passed as a function
  pointer at `0xa8cf21`). Nothing in the CGD loader or in the batch update calls it, so the
  five-slot continuous level of the vegetation does not apply to `.cgd` instances.
* **The batch update.** `CModelBatch` (RTTI vtable `0xf0f774`) has two entries: destructor
  `0xa52950`, update `0xa4c6b0`. The update runs only while bit 0 of its flag byte
  (`this+0x58`) is set. It computes the squared distance from the camera to the batch's
  AABB (`0x4ebc90`, point-to-box, 0 inside). If that is **at least 122500 (350 m)** it marks
  the owner enabled and, when the owner's distance cull bit is set, submits it only if
  `d^2 * owner multiplier <= table[count]` (`0xf0ea0c` = 0, 100, 625, 2500, 10000, 250000, the
  vegetation table), through the shared submission `0xa42740`, for three passes. If the
  camera is **closer than 350 m** it clears the owner's enabled bit and records an exclusion
  (camera, 122500, 14400 = 120 m squared, constants `0x1089eb0/b4`, scale `0x1089eb8` = 1.0) in
  the list at `this+0x28` for the later consumer `0xa4bac0` / `0xa4c5a0`.
* **Model batches in the CLocation.** `0x73b800` (constructor) and `0x687110` (destructor)
  special-case the **far** CLocation (`near == 0`): they look the location's name up in the
  table at `*(DAT_011911f0 + 0xc4)` (`0x8ed120`), set or clear bits `0x11` on the matching
  entries (`0x8fbaf0` -> `0x8c7840`), clear bit 0 of the owned object's byte `+0x4c` and bit 0
  of every `CModelBatchResource`'s byte `+0x58` (the same flag the update above tests). The
  matching enable is the CLocation vtable slot 7 `0x554110`: for a far location it sets those
  bits again and, while `DAT_011958c4 == 3`, waits until the object's counter reaches its
  limit before clearing the table flags. The near CLocation skips all of this.

### Inferred

* **Resolved by JC2-146** (`cgd-model-batch-next.md`): the `+0x58` byte is the `CModelBatch` object
  that the loader wraps as `CModelBatchResource` (proven, not inferred). The 350 m switch is the first
  of three tiers (merged model from 350 m, per-part models 120..350 m, individual records below 120 m),
  the "exclusion list" is the entry vector, near `.nl.cgd` is a plain `CModelCollection` and not a batch.

## Project rule today vs native

| | jc2-rs now (`locations.rs`) | native |
|---|---|---|
| Position | any loaded terrain tile | main camera XZ |
| Test | near box overlaps a loaded 512 m tile (inclusive) | camera strictly inside the near box shrunk by 10 m |
| Far `.fl.cgd` | only together with near | independently, camera strictly inside the raw box (shrunk 10 m); also far-only ring |
| Unload | owner set empties (tile unloaded) | camera outside the raw box (no margin) |
| Pace | one location per update | one at a time, FIFO; next only after the previous is loaded |

## Bounded Rust handoff (not done here)

1. `jc2-formats::location`: add `far_bounds()` (raw expanded by the margin when the result stays
   non-empty; `Location` already has `near_bounds()`), `contains_load(kind, x, z)` (strict, 10 m)
   and `contains_keep(kind, x, z)` (strict, raw), with a `LOAD_MARGIN = 10.0` constant citing
   `0x5e21f0`. Y-free.
2. `jc2-game::locations`: add a pure `ActivationState` keyed by (location, kind): `pick(camera)`
   = records scanned last to first, near before far, larger `min |camera - face|` wins; allow it
   only when nothing is loading; `keep` pass first (one unload per frame). Drive it from the
   main camera position, not from loaded tiles. Keep the terrain streamer unchanged.
3. Split `place_container_statics` into near-container and far-container placement so that the
   far `.flz` (`.fl.cgd` and its low LOD models) can be requested alone for the ring; owners are
   (location, near|far) rather than tile keys.
4. Fixtures to copy: the probe's km01.base and 31x14 windows; the three camera positions and the
   expected near/far order of the probe (`activation_order`).
5. Out of scope here: actors, `wc` tiles' terrain, the batch hide/exclusion (Gap 7), the
   `--lod` level of far models.

## Gaps

1. **Writers of T+0x64 bit 0 and of the margins.** `CGameWorld+0xbc` bit 0 (pass disabled) and any
   change of `+0xc0/+0xc8` (10 m) after the constructor were not searched exhaustively; only the
   constructor value and the `0x618340` setter of bit 0x40 are proven. Blocks claiming 10 m is
   constant in every mode (loading screens, teleport, mission starts).
2. **CLocation load stages.** State `4` (loaded) and `5` (unloading) are used by `0x520070`; states
   0..3, the per-frame work budget, asynchronous IO and the vtable `+0x10` poll (`0x8c6bd0` family)
   are not decoded. Blocks pop-in timing: native arrival is slower than "same frame".
3. **The second list (T+0x34) and the pending list (T+0x44).** Role inferred (loaded locations in
   load order; explicit destroy requests); the append site in `0x520070` (`0x920500` with
   `0x9178b0` / `0x69c350` guards) and the writers of T+0x44 are not traced. Blocks unload order for
   records that finish loading out of order.
4. **Non-spatial queues.** What the unconditional records (vectors T+0x04 and T+0x24, type 0 `Global/`
   and type 1 named paths) load and who registers them (`0x618340`, file named by T+0x74) is not decoded.
5. **Far-location visibility protocol.** The table at `*(DAT_011911f0 + 0xc4)` (`0x8ed120`, `0x8c7840`),
   who calls CLocation vtable slot 7 (`0x554110`) and what `DAT_011958c4 == 3` is are unresolved; the
   effect is that a far location's batches and object start hidden and are revealed later. Blocks
   exact far pop-in and fade.
6. **Camera field.** `+0x54..+0x5c` of the main camera is treated as the world position from its
   use in `0xa4c6b0`, the tether range and the water mirror; the writer of that row for the free
   camera, cutscenes and the character camera is not traced. Blocks claims for modes where the
   main camera is not the gameplay camera.
7. **Model-batch ownership.** Answered in `cgd-model-batch-next.md` (JC2-146): the reader belongs to
   `CModelCollection`, the batch object is `CModelBatch`/`CModelBatchResource` created from `.batchlod`
   files (resource type 0x21); the CGD `-1.0` is a LOD-scale override valid with a non-zero distance
   anchor (the 12 "zero" bytes), the `3` is a flag word. Remaining details are that document's Gaps.
8. **Which `.lod` slot a batch instance shows.** Answered there: batch objects never run the continuous
   selector and always submit slot 0; `.cgd` instances use the shared gate `0x8c9100`.
9. **Margin key** `#1070a35f` (carried from JC2-111): sign and int/float conversion unproven;
   four locations use it, none a tile or km01.base.
10. **Actors, conditional locations, `conditions.bin` key name** (carried from JC2-111, unchanged).
11. **No capture.** The ring was not rendered; the doc is bytes plus a model, not a visual.
