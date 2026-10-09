# Performance and streaming capture preparation

2026-10-08, bounded source/data audit after merging master `71c24d0` into the
graphics research worktree. No game launch, Cargo build, profiling or runtime
edit was performed. [graphics-next.md](graphics-next.md) covers material/shader
contracts; this brief gives the next agent a measurement and optimization order.

## Baseline status and measurements already supported

`STATUS.md` records approximately **4 fps while other agents were compiling**.
This is a historical observation under contention, not an idle-machine baseline
and not evidence of a GPU bottleneck. Stop builds and other game instances before
collecting the first baseline. Record running background processes, GPU/driver,
window dimensions, render backend, binary hash, code revision and exact flags.
Do not infer throughput from CPU utilization or GPU memory alone.

`crates/jc2-game/src/smoke.rs` already collects `Time<Real>::delta_secs()*1000`
after **12 seconds** of warm-up, stopping when the capture is requested. At exit
it logs frame count, average, median, p95 and worst frame time. `--exit-after 45`
gives approximately 33 seconds of samples, subject to actual startup progress.
These are wall-clock application frame intervals, **not GPU timestamps, draw
counts or a split of update/render/presentation time**. Estimated average FPS is
1000/average_ms, not the mean of instantaneous frame reciprocals. A screenshot
adds capture work after the recorded sample window. Short 8-second smoke runs
have no post-warm-up samples and cannot establish a steady baseline.

World free-camera HUD currently displays loaded/dropped tiles, the last tile-load
milliseconds, terrain/model triangle totals, pending/failed object counts,
collision shapes and unresolved references. Triangle totals count loaded assets,
not the camera's visible rasterized triangles. Per-tile INFO logs include near
vegetation counts, compiled-static counts and impostor entity/tree counts.
`GrassStats` tracks blocks/blades/build_ms internally, but this audit found no
equivalent periodic INFO logger for it. No existing per-system CPU/GPU profiler
or draw-count CSV was identified. If baseline is slow, instrumentation is the
first coding assignment rather than a speculative optimization.

## Concrete capture commands and scene matrix

Root currently has `target/jc2-integrated.exe`; its filename does not establish
which commits it contains. Check `--help`, save SHA256 and record its build owner.
Use the coding agents' final integrated binary once available. Commands below are
future runs, not executed in this research assignment. Create the ignored output
directory before launch; GUI execution requires the normal environment approval.

```powershell
$exe='C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/target/jc2-integrated.exe'
$game='C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2'
New-Item -ItemType Directory -Force target/perf-capture | Out-Null
Get-FileHash -Algorithm SHA256 -LiteralPath $exe
& $exe $game --world 31,13 --radius 1 --no-hero --no-physics --no-npcs --no-traffic --no-civilians --no-music --time 12:00 --day-speed 0 --exit-after 45 --screenshot target/perf-capture/arctic-base.png *> target/perf-capture/arctic-base.log
Select-String -LiteralPath target/perf-capture/arctic-base.log -Pattern 'smoke:|tile |impostors |ERROR|WARN'
```

Use the same command with a changed world tile/output names for every scene.
Keep camera still for the first run, then repeat three times with identical
settings. Report all trials rather than picking the fastest. Shader compilation
and cold archive access can affect the first trial; label cold/warm runs. Check
pending object count and ongoing impostor arrivals before assuming 12 seconds
was enough to stabilize this particular scene. Extend to 75 seconds if necessary.
Keep INFO logging consistent between trials; verbose render logging is a separate
diagnostic run and can perturb timing.

| Scene | Evidence / intended stress | Capture qualification |
|---|---|---|
| 31,13 | Arctic fixture: tool confirms 4769 plants, 841 trees/3928 bushes+rocks | existing gameplay test start; inspect near leaf cards, road, distant forest |
| 32,7 | README documents capital fixture; `32x07.nlz` CGD probe confirms 283 instances/28 models | inspect actual screen framing for buildings/roads; CGD is only one part of total placements |
| 40,40 | Jungle fixture: tool confirms 5459 plants, 641 trees/4818 bushes+rocks | dense canopy, grass and cutout fill; inspect loaded camera view before accepting it |

Use **32,7** for the capital benchmark. An older `civilians.md` calls 33,26 a city
tile, but this audit did not verify its settlement identity. A climate label or
roam-area count does not establish that a capture shows the capital skyline.
Counts above are parser/generator evidence, not rendered entity or draw counts.

After these baseline runs, vary **one control at a time**:

| Variation | Purpose / caveat |
|---|---|
| `--radius 0`, then 2 | loaded-tile pressure: nominal desired grids 1,9,25 at radius 0,1,2; unloading hysteresis can retain more while moving |
| `--no-grass` | grass mesh creation/alpha fill contribution |
| `--grass-density 0.25` or `--grass-radius 40` | density rounds up to one of four passes; it is not a continuous fractional blade budget |
| `--vegetation-lod 1` versus default 2 | near triangle/vertex load at fixed vegetation placement |
| `--bush-density 0.5` | deterministic removal of bushes/rocks, trees remain |
| `--no-impostors` | distant cards disabled **and near model VisibilityRange disabled**; loaded trees remain visible farther, so it is not a clean far-only subtraction |
| `--no-vegetation --no-impostors` | remove near vegetation and distant tree cards together; grass remains unless separately disabled |
| `--flat-terrain` | approximate simple-terrain shading comparison; appearance intentionally differs |
| `--no-models --no-grass --no-impostors --no-clouds` | coarse isolation of terrain/horizon/sea/sky; no-models also prevents scenery dressing; not a bare terrain-only renderer |

There is no current `--no-horizon` or `--no-static-geometry` CLI flag identified.
Do not invent those flags in a benchmark script. Hold sun, audio/AI choices,
MSAA/presentation and window size constant. No explicit CLI presentation/MSAA
control was found in this audit; capture observed/configured values before
comparing AlphaToCoverage output.

## Source-grounded work and lifetime inventory

| System / owner files | Existing structure | Potential cost to measure |
|---|---|---|
| terrain, world.rs::stream_tiles/load_tile/terrain_mesh | synchronous nearest missing tile once/frame; 129² vertices and 32768 triangles/tile; collider separate; slot/weight images and material per tile; atlas handles shared | archive inflation, normal/vertex-colour sampling, mesh and collider build/upload |
| near scenery, scenery.rs::dress_tiles/grow/place_statics | one undressed loaded tile/frame, synchronous plant generation/collision/resolution; shared RBM handles queued | duplicate terrain parsing, placement/collision CPU burst |
| model spawn, world.rs::spawn_models | scans pending objects across loaded tiles; every ready object gets parent plus one mesh child per render block; uncapped ready queue drain | deferred ECS creation and rendering-extraction burst after async asset arrivals |
| grass, grass.rs::update_grass/build_block | 32m blocks, 16 8m patches/block; 3 quads/blade, 12 vertices/6 triangles/blade; one mesh/entity per nonempty block and shared material | sampling each candidate, corner height sampling, vector allocation, scans/sort and mesh upload |
| impostors, impostors.rs::stream_impostors/build_tile | AsyncComputeTaskPool, two in-flight tiles; 16 patches/tile, up to crown+trunk entity each; same tree generator, atlases shared | worker terrain parse/generation plus whole completed-tile upload/spawn on main thread |
| horizon, horizon.rs::stream_horizon/hide_under_tiles | 1024m chunks; step1 within5km, step2 outside; six chunks built/frame; fine512tris/chunk, coarse128; FAR14km; no physics | scan/sort each update, mesh rebuilds when crossing5km, visibility scan |

Grass's 4ms `BUILD_BUDGET` checks elapsed time **before** starting each complete
block. Tile reads and a long block can overrun it; wanted-list/drop scans and
sorting occur before its timer, so 4ms is not a hard whole-system budget. Parsed
grass terrain tiles retain a 3x3 neighborhood around the camera; blocks unload
past radius+32m and empty-block markers past radius+64m. Grass shader fades alpha
over 0.6*radius..radius, uses AlphaToCoverage and GPU tip sway. Mesh culling bounds
are generated from unswayed vertices here; this audit saw no wind padding, unlike
the impostor padding described below. This is a visual correctness check before
tightening culling, not a confirmed popping bug.

Impostor tiles unload past radius+1 (Chebyshev), including removal of any Building
task handle; task cancellation details were not traced. Finished tiles can add
up to32 mesh entities in one update. Their AABBs receive22m padding for expanded
GPU cards and VisibilityRange uses the padded AABB. Model-tree fade520..580m and
impostor fade450..600m deliberately overlap. Impostor WGSL uses
visibility_range_dither and Mask(0.4); grass alpha behavior is different.
Do not substitute a single cutoff/fade policy across both systems.

Horizon keeps ocean-only markers, swaps old fine/coarse entities when detail
changes, sinks coarse geometry up to18m, and hides a chunk only if all four
512m detail tiles are loaded. Existing world tile unload is radius+1, decrements
per-path model reference counts, drops zero-reference handles, despawns tracked
entities and forgets that location container in `AssetIndex`. Generated terrain,
grass, impostor and horizon meshes are held by entities; investigate Bevy's actual
asset lifetime after handle drop instead of assuming despawn leaks or manually
removing still-shared assets. `Streamer.colliders` and `Scenery.plants` have no
per-tile eviction observed in the audited paths: measure growth during long routes.

Two separate container caches exist: world's `AssetIndex` and
`archive_source.rs::JcAssetReader`. The latter caps at40 containers and evicts an
arbitrary first HashMap key, not an LRU or byte budget. Its inflation occurs
outside the mutex; concurrent cold requests can duplicate work. Container data
can also be copied to members. World tile parse, scenery grow, grass cache and
impostor workers each read/parse terrain independently. Sharing immutable terrain
could reduce duplicate work, but requires bounded lifetime and worker ownership.
No cost or resident-memory magnitude has been measured for these candidates.

## Next coding assignments, after the idle baseline

| Priority / owner | Dependency | Proposed change and evidence gate |
|---|---|---|
| P0 measurement owner: smoke.rs + small diagnostics module | final integrated binary; no active builds | baseline wall-frame stats first; if slow, add spans/timers/counters for systems above, pending/ready queue, assets and upload; distinguish GPU timing support from wall time |
| P1 streaming owner: world.rs/scenery.rs | CPU spikes correlate with load/spawn | cap spawn work by objects/time, preserve pending order; move immutable terrain/placement generation to workers with explicit cancellation and main-thread apply budget |
| P1 grass owner: grass.rs/formats grass.rs | grass-specific CPU samples dominate | split oversized block build or async immutable generation; include scan/load/apply in measured budget; reuse sampling only if deterministic heights/placement preserved |
| P1 cache owner: archive_source.rs + world AssetIndex | inflated cache bytes/misses measured | bounded byte/LRU policy, protect shared assets; consider in-flight dedup and shared parsed terrain; coordinate asset reader changes with normal-map image setting owner |
| P2 draw/visibility owner: scenery/impostors/renderer | renderer submission or GPU vertex/fill dominates | inspect Bevy automatic batching first; merge static compatible instances only if culling/collision/destruction identity survives; adjust patch size with evidence, preserve expanded card bounds |
| P2 terrain shader owner: terrain_material.rs/terrain_splat.wgsl | GPU material cost measured | profile atlas/displacement/normal sampling variants; existing shader already chooses two dominant layers and fades detail by distance; compare against flat-terrain only as an ablation |
| P2 horizon owner: horizon.rs | scan/rebuild cost measured or boundary artifacts | movement/detail hysteresis and changed-focus scheduling; preserve chunk coverage and coastline; no distant mesh reduction without seam capture |

No assignment is a claim that its target is presently the bottleneck. Shared
mesh/material handles do not prove a single GPU draw; separate entities also do
not prove Bevy failed to batch them. Obtain actual render-pass counts before
custom instancing. More texture samples and masked foliage can trade CPU savings
for GPU fill; measure the full change against the same scene and presentation.

## Movement, visual acceptance and rollback

After still-camera baseline, record a manual free-camera route with HUD positions
and speed: traverse a512m tile boundary, return twice, then fast travel several
tiles and settle. World free-camera controls are WASD, right-drag look, Space/C,
Shift faster, Alt slower, wheel speed and R reset. No deterministic camera-path
CLI was found, so manual route timing is exploratory until a measurement owner
adds a replay path. Repeat distance sweeps around380/440/450/520/580/600m, grass
48/80m at default radius, horizon5km and detailed/coarse terrain boundaries.
Capture memory after settling on the same origin following each return.

Preserve before/after PNGs and logs under ignored `target/perf-capture`. Render
acceptance: leaf/grass silhouettes have no solid rectangular cards, mask
cutoffs preserve canopy density, faded tree model+impostor ranges never leave
holes, wind/shadows remain aligned, tile unload/reload leaves no missing geometry,
coastline/terrain seams do not open, and image colour-space changes preserve real
normal detail. Test low/grazing camera and elevated helicopter views for culling.

Each shader/material/streaming optimization should land as a separate reviewable
commit with metrics and the same capture arguments. Keep the prior integrated
binary under a distinct ignored filename before replacement; use it for a visual
comparison. If an optimization fails acceptance, revert that isolated commit
through the orchestrator rather than resetting other agents' work. CLI toggles
can isolate regressions but disabling materials/vegetation is not a production
visual fix. Runtime/GPU results remain outstanding for the assigned coding agent.
