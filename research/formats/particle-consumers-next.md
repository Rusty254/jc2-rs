# Native MC smoke consumers — JC2-080

Research on the original `fx_vehicle_mc_smoke_01` leaf named by
`lave_mc_damage_smoke.bin`. This establishes primary execution references for a
later runtime change. It does not enable modifiers, replace gameplay effects or
claim retail visual parity. JC2-014's static cell, world motion, scatter, fixed
quad size/alpha and unlit material remain explicit preview policies.

## Reproduction and provenance

Run from the repository root with an existing `jc2tool` binary:

```powershell
python -B research/tools/particle_consumer_probe.py --boundaries-only
python -B research/tools/particle_consumer_probe.py --game 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2' --tool '../codex-particle-fixture/_extract/particle-fixture/jc2tool-particle46.exe'
```

The second command's private tool path applies from this research worktree;
substitute an existing tool from another checkout when running from main. It
requires the existing Ghidra export directory under main
`research/analysis/pseudo/_unnamed`; `--pseudo` accepts an explicit location.
Neither command launches the game, invokes Cargo, or extracts textures. JSON
records original decimal numeric values, each native address/range/hash, source
contracts and hand-selected binary32 boundary references. Diagnostic text Int
values from `jc2tool particle` are hexadecimal: `32` there means decimal 50.

The executable must be PE32 `JustCause2.exe`, 14,549,656 bytes, SHA256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
The probe rejects other fingerprints. The shader bundle SHA256 is
`db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`.
Primary exports are `text_00a50000.c`, `text_00a60000.c`, `text_00a70000.c`,
`text_00930000.c` and bounded math helpers. Local Windows SDK
`10.0.26100.0/um/d3d10.h` supplies the numeric blend descriptor definitions.
The probe asserts significant export statements and shader instructions, rather
than treating an address inventory alone as behavioral verification.

## Ownership, ordering and windows

`FUN_00a71720` reads modifiers by contiguous `effect_0`, `effect_1`, ... names,
stopping at a gap or 32 entries. Only if that family is absent does it read
`visual_effect_N` followed by `physical_effect_N`. It allocates 0x28-byte entries,
then calls class-index mapper `00a551a0` and reader `00a65a40` for each index.
The MC leaf's execution order is:

| Index | Literal class | Principal original fields |
|---|---|---|
| 0 | CParticleVisualAnimation | lower 30, upper 50, rate 30, loop 0 |
| 1 | CParticleVisualSize | coefficient .2, window [0,1) |
| 2 | CParticleVisualOpacity | coefficient +4, window [0,.2) |
| 3 | CParticleVisualOpacity | coefficient -1.168, window [.4579,1) |
| 4 | CParticlePhysicalForce | vector (0,1,0), window [0,1) |
| 5 | CParticlePhysicalDamping | coefficient 5, window [0,1) |

The lossless PropertyFile children happen to appear in hash order. Their order
must not substitute for this native indexed execution order.

`00a62520` queues a pointer to `CParticleSystemData + 0x10`. Therefore worker
offsets in the following functions are 0x10 less than root reader/render offsets;
this is a pointer view of the same data, not a guessed second configuration.
It calls `00a62980` for existing particles before `00a5ca60` creates births.

`00a62980` first retains particles with entry age+0x1c strictly below 1 and
positive size+0x0c. It advances normalized age by reciprocal lifetime+0x20 * dt,
integrates positions with existing velocity, then dispatches the indexed
modifiers. Window comparisons use that **updated normalized particle age**,
inclusive start and exclusive end. They are neither system seconds nor absolute
birth timestamps. A special full-window path (`start <= 0 && end >= 1`) bypasses
the comparisons. Thus a survivor crossing from .99 to 1.01 can execute a full
window modifier in that job, while the partial opacity window ending at 1 does
not execute. Removal occurs at the next entry check. Newly born particles start
with age zero and do not run these modifiers until the next job.

Emission tests accumulator **strictly greater** than spacing; exactly .04 is not
a birth for this 25/s source. This native phase is distinct from014's deterministic
event-clock policy. Job clock production/time scaling and parallel publication
timing have not been measured in gameplay; do not equate host wall time to dt.

## Signed opacity and physical execution

`00a65a40` reads opacity coefficient via DAT_00f1207c (#9a4a5864) and windows
via DAT_00f12084/88 (#f40793e7/#590f095f). `00a59ef0` executes:

```text
if updated_age in window (or full-window bypass):
    alpha = system_scale * clamp(alpha + coefficient * dt, 0, 1)
```

The scale is worker+0x9c/root+0xac, default 1 in `00a62710`. Do not relocate it
outside the per-modifier update or assume external instance overrides never
occur. The root initial alpha #4885935d is 0; `00a71720` stores the clamped value
as ushort at root+0xcc, and birth reads worker+0xbc divided by 65535. Retail
negative opacity is an incremental fade rate, not a negative endpoint alpha.
At updated age exactly .2 the +4 modifier is excluded; exactly .4579 includes
the negative modifier; exactly 1 excludes it. Probe fixtures use adjacent
binary32 values of the original fields, signed additions and clamp boundaries.

Force `00a54740` adds vector * dt to velocity. Damping `00a5a210` subsequently
multiplies velocity by `1-min(dt*5,1)` for MC. Position integration precedes both,
so their new velocity affects subsequent movement. The .125s zero-velocity
reference gives final Y velocity .046875; reversing their order gives .125.
This is an order-sensitive reference, not a captured native particle trajectory.

## Birth frame, playback and UV selection

Root DAT_00f12190 (#a5318e57) reads initial frame 30 into root+0xb8;
DAT_00f12194 (#97569fc5) reads variation 10 into root+0xbc. Reader narrows each
through signed short before storing a float. `00a5ca60` starts particle+0x4c at
worker+0xa8 and adds a random integer modulo `(int(variation)+1)` when positive:
MC births cover **30 through 40 inclusive**. Shared native RNG sequence/seed
ownership remains a parity prerequisite; deterministic014 RNG is different.

For CParticleVisualAnimation, `00a65a40` reads #c871c24e and #9fba43a8 as Int
bounds, #db53dd7d as float rate, and loop #9ea55007. `00a5a0b0` adds **30 * dt**
to the floating frame and clamps to [30,50] for MC loop=0. Its loop!=0 branch
wraps the interval once and then clamps; it is not general modulo. The extra
#59a79825=0 field is read, but this playback function does not consume it.

Ghidra's inferred `int *` cast around the rate is misleading. Actual instructions
at 00a5a143 are `movss [system+74]`, `mulss [modifier+0c]`, `addss [frame]`;
the probe asserts their exact 14 bytes. Lower/upper Int conversions are separate.

Ordinary draw `00a5bc30` uploads three float4 records per particle, including
unrounded frame as record2.x. Depth draw `00a5b430` uploads two float4 records.
The original shaders `particle` VS at bundle offset246068 (3120 bytes) and
`particledepthonly` VS at249208 (1632 bytes) both use **round_ne**, nearest even.
For example 30.5 selects30, 31.5 selects32, 49.5 selects50. The probe tests each
tie and its two binary32 neighbors. For rounded cell n and verified8×8 dimensions,
the atlas offset is `(frac(n/8), floor(n/8)/8)`; 30=(.75,.375),
32=(0,.5), 50=(.25,.75). The terminal cell50 remains selectable.

UV and quad corners are additionally obtained from packed per-cell cb2 records;
the vertex shader's cell-derived lookup and packed-coordinate decoding are visible.
This research does **not** establish that a full square cell is the exact native
quad. `00a55100` builds packed vertex selectors and `00a54fd0` indices, but the
producer of the per-cell fitted corner data still needs tracing. Atlas offsets
alone do not establish fitted geometry, vertical orientation or texture sampling
policy parity.

## Shape5, direction and moving source space

DAT_00f12114 binds root shape #89c42bb5=5. `00a65be0` loads emitter radial
bounds #c2826ce3=0 and #7c01072c=.15 into emitter+0/+4. Shape5 in `00a5a4e0`
normalizes the supplied direction, multiplies it by a uniformly sampled radius
in that range, adds authored local offset+0x28/+0x2c/+0x30, then calls point
transform `004d7dc0`. A zero direction invokes `004e52c0`: three global `rand()`
components in [-1,1], normalized if squared length >=1e-6. It is not another
sample from the emitter's own LCG. This exception matters to RNG ownership.

`00a5a2d0` samples angular base/variation at emitter+0x50/+0x54 and
+0x58/+0x5c; MC values are 0,0 and 3.1415,.3142 respectively. It samples
speed as base ± variation, then clamps to [-128,127]. Negative instance overrides
fall back to emitter+0x60/+0x64: both MC values .4, meaning base .4 and variation
.4, rather than a fixed speed or equal lower/upper bounds. It creates rotations
through `004eaad0` (Y) and `004dbce0` (X) and matrix composition `00491e50`.
The math helpers use polynomial trigonometric approximations; ordinary libm
sin/cos is not a bitwise native replacement. `00a5ab90` consumes the two output
vectors separately for velocity and shape direction. The direction's exact
composed-matrix convention must be preserved when implementing it, including
emitter byte+0x0d's source-orientation branch; this brief does not substitute +Y.

Root space DAT_00f12108 (#562663c7) is **2** for MC. `00a66490` selects identity
as the birth source frame for that mode. `00a5ca60` skips the world-source origin
interpolation used for modes<2 and only adds inherited source velocity for mode1.
`00a5bc30` recognizes mode2 and transforms each living local position through its
current source matrix. A surviving MC particle therefore moves with a translated
or rotated source. Root bit9 #f989d1c1 is0; the separate velocity-orientation flag
must not be conflated with simulation space. Mode2 moving-source behavior is
primary evidence, but external hierarchy/socket ownership is outside this task.

## Native material and remaining parity gates

Root #27230869, #59d9aeb9 and #71feec86 are0 (render flags bit7, bit6 and bit13).
`00a5bc30` takes the ordinary non-additive/non-distortion branch, writes RGB mask7,
and calls `00923ba0(...,0x12,0x13)`. That helper packs both RGB and alpha source/
destination factors; `00933530` copies them into the D3D10 blend descriptor.
Local SDK constants 18/19 are **SRC1_ALPHA / INV_SRC1_ALPHA**: dual-source alpha,
not SRC_ALPHA / INV_SRC_ALPHA. Blend-enable/op state also depends on renderer
initialization; a future port must preserve the whole descriptor, not only factors.

The ordinary Particle PS at6734036 (664 bytes) multiplies texture RGBA by vertex
RGBA, saturates after *4, applies fog to RGB, and raises alpha to1.18. It outputs
alpha*.25 in target0 and full alpha in source1. ParticleSoft at6736912 (888 bytes)
has the same dual-source outputs plus depth texture t1 attenuation. Both are
fingerprinted/disassembled by the probe. Exact active soft variant is renderer
context-dependent, rather than a leaf field inferred from its name. Ordinary VS
also supplies lighting/emissive and camera-distance alpha; unlit StandardMaterial
is not established parity. Texture sRGB/linear policy, sampler addressing/filtering,
depth state and renderer buffer conventions need explicit downstream evidence.

Actionable follow-up is a named-leaf runtime implementation constrained by these
contracts, followed by original moving-source and frame-boundary captures.
Before claiming1:1, resolve per-cell fitted geometry/corner producer, exact RNG
streams/initialization, native job-clock/publish timing, complete source-direction
matrix convention, shader variant/state/sampling/lighting and original comparison
captures. Whole graph, child/sound routing and gameplay effect replacement remain
separate scope. No new task was claimed for those gaps.

## Validation

The read-only probe passed against the fingerprinted retail executable, original
leaf and shader bundle. Its hand-selected boundary references cover normalized
window neighbors, lifetime crossing, signed/clamped opacity, indexed physical
order, frame clamping/single wrap, nearest-even ties, atlas offsets and moving
source point transforms. Synthetic loop1/scale.5 contrasts are labeled; they are
not MC authored settings. These checks are structural reference fixtures, not
native execution tests, gameplay measurements or GPU comparison evidence.
No Cargo, GPU launch or runtime edit was performed for this research task.

## JC2-087 bounded runtime update

The named-leaf fixture now decodes supported modifier parameters separately from
the unchanged raw property tree and computes the native contiguous indexed order.
Unknown classes stay diagnosed, and unmatched class children remain in the raw
inventory. Scalar reader hashes/types are checked against the original MC data.

Additional size evidence: DAT_00f12040 is #e3b6dc5d; reader00a65a40 type4 stores
it at modifier+0. Consumer00a545e0 (size0xaf) adds
`dt * coefficient * (size_scale < 1 ? size_scale : 1)` to particle size.
Initial size DAT_00f12180/84 (#ef05f293/#adeb7b1a) is base .08 plus uniform
range .04, not a symmetric variation. The probe now records this native window
and asserts reader/add/scale export contracts. DAT_00f1217c (#74175c29) reads
authored maximum25 into root+0x76, consumed as worker+0x66 at birth. Allocation
honors that cap in addition to user owner/global budgets; separate counters report
authored-cap suppression, visual-budget dropping and overload dropping.

Runtime updates entry-live survivors before births, advances normalized age,
integrates old velocity and dispatches indexed animation, size, signed opacity,
force and damping. A lifetime-crossing survivor is removed at the next entry
check. Newborns retain age0/initial alpha0 in their birth job. Floating frames
advance/clamp and use nearest-even cell selection; full unit-cell meshes are
cached once per viewer. Each live drawable owns one mutable alpha material, which
is removed when its particle or final source owner disappears. Shared cell meshes,
texture and template material remain fixed viewer resources across restart.

Emission follows strict binary32 accumulator>spacing and final remainder. Thus
an exact .04 job emits no MC particle and final fmod removes the equality;
repeating exact .04 jobs also emits none. The adjacent binary32 value above .04
does emit. This seemingly surprising phase is retained from the primary consumer,
not adjusted to satisfy the014 event-clock policy. Normal updates intentionally
depend on job dt partition, including intermediate opacity-window visits. Host
clock/publish timing remains unverified. Automatic stop splits a host frame at its
exact configured simulation boundary; that split is a viewer control policy.

At most4096 birth attempts are evaluated per host step. Remaining overload events
are counted/dropped and their ordinals consumed without a later burst. Global
owner iteration order is stable viewer policy. Native shared RNG streams, capacity
suppression's RNG consumption and overload accounting are not asserted bitwise
parity. Nonfinite, negative or steps above1e6 seconds fail before mutating pool
state; counter headroom is checked before birth work. Rate/lifetime/configuration
and finite source-transform bounds remain explicit. Stale owner generations and
missing/invalid frames remove particles and their drawable/material state.

CLI default is animated playback. An explicit `--particle-frame N` is a labeled
static render override; simulation still evaluates native modifiers and caps.
`--particle-move-source` provides reproducible source translation, rotation and
scale for local-space checks without changing gameplay attachment behavior.
Full source-direction/RNG, fitted geometry and native material remain the
previously documented approximations; the fixture does not claim1:1 retail visuals.

Source fixtures cover original decoded coefficients/order, signed opacity and
size, float-frame ties/neighbors, native phase equality/repeated dt, newborn and
lifetime crossing, force-before-damping/Euler partition dependence, literal
rotated/scaled moving-source placement, authored/global/owner/zero budgets,
finite stalls/rejected-input preservation, source generation and material cleanup,
and unchanged60HP/one-blast damage across particle budgets. Isolation51 release
validation passed392 workspace tests (149formats,243game), including the original
Steam fixtures; strict release all-target clippy, workspace formatting and release
build passed. At dt=.2, strict binary32 interval subtraction produces four births;
the inherited damage/owner-cleanup expectations now preserve that native phase.
The updated primary probe passed23 boundary cases,11 export contracts,25 native
windows and4 shader contracts against the fingerprinted retail executable.

GPU acceptance used a private release copy, `RUST_LOG=info`, the real game directory
and `--particle fx_vehicle_mc_smoke_01`. Five runs exited0; both output streams are
preserved in `_extract/particle-playback/*.log`, with screenshots alongside them.
Each log positively confirms the original1024x1024 Bc3 DDS loaded. No warnings,
errors or panics appeared. These are bounded fixture smoke checks, not a retail
visual comparison or an interactive gameplay test.

| Capture | Additional arguments | Inspected evidence |
| --- | --- | --- |
| animated.png | `--exit-after 2` | 19live, cells35..50, opacity.08...80; detailed original smoke |
| moving-positive.png | `--particle-move-source --exit-after 2` | sourcex+.22, cloud moved right, animated cells/opacities |
| moving-negative.png | `--particle-move-source --exit-after 6` | sourcex-.22, cloud moved left;25live at authored cap |
| fading.png | `--particle-stop-after 1 --exit-after 1.7` | draining9live, all cells50, opacity.19...57 |
| drained.png | `--particle-stop-after 1 --exit-after 3` | draining0live, empty cell set, zero opacity and no visible smoke |

For each row use `--screenshot ABSOLUTE_PATH/FILE.png`; private executable and
captures are ignored local evidence, not committed game assets. GPU51 was released
after inspection; no further runtime changes followed the successful build.
