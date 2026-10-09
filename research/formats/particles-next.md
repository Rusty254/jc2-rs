# Particles, fire and destruction visuals — 8 October 2026

Research only, following `notes/RESEARCH-HANDOFFS.md` assignment 4. Read STATUS,
animation-next.md, world.md, weapons.md, explosion.rs and destruction.rs. No Cargo,
game launches, runtime edits or commits. Primary evidence is local retail data and
the original executable's decompiler export. After the initial inventory, diagnostic
reads used a private prebuilt tool at `_extract/particles-ik-research/jc2tool.exe`;
scratch is ignored. Nothing below establishes visual parity with retail captures.

## Verified data chain

`effects/effectsettings.bin` maps effect names to `filename` and numeric
#6a9dfd76 IDs. Examples: fire_medium=29, explosion_metalchamber_large=83,
explosion_metalchamber_small=84, explosion_lave=128, lave_damage_smoke=150,
fueltank_burning=163, env_gastank=363. Effect definitions parse with the existing
PropertyFile reader; a new binary container decoder is unnecessary.

Effects are component graphs, not individual sprites. Sample `explosion_laser.bin`
contains type 1 named particle fx_laser_impact_01, type 0 sound bank 11/sound 23,
type 3 decal with lifetime 100 and diffuse/specular/normal textures, type 7 colour
light-like keys, type 4 physical-looking fields, and type 9 SoundPostEffect.
Only particle/sound/decal/post-effect roles have clear data evidence here; do not
assign gameplay explosion damage to type 4 merely from its numerical fields.
Graph selections use strings such as `1:0`; their exact index/routing semantics need
the effect-description consumer. `footstep.bin` adds material-specific selections
(Sand, Snow, Ice, Gravel, Grass, Vegetation) and particle/decal combinations.

`global/particles.blz` is SARC v2 with 2625 entries: 2600 `.psmb` property members,
24 DDS textures and `particles.bl.bl`. The latter contains CResourceObject references
to `particlesystems/<name>.psm`. The factory's `.psmb` suffix is present in
`research/analysis/pseudo/_unnamed/text_008a0000.c:14032`. Resolve the graph's particle
name through this resource scheme and archive members; ordinary GameFs filename
lookups alone will not find every particle-system leaf.

## Verified fields and consumer addresses

Exact spellings were checked with `jc2tool hash`:

| Hash | Field | Observed value / original reader |
|---|---|---|
| bc4e9cf9 | particle_lifetime | smoke_01=1; exp_barrel_01=.75; hash word at 0x00f12130 |
| 79c213bb | system_lifetime | smoke_01=0; exp_barrel_01=.1; word at 0x00f12138 |
| 487f6fc3 | particle_rate | smoke_01=25; smoke_02=100; exp_barrel_01=5000; word at 0x00f120e8 |
| f40793e7 | start_time | modifier window; word at 0x00f12084 |
| 590f095f | end_time | modifier window; word at 0x00f12088 |

The words were located in the original exe and mapped from PE file offsets to VAs,
particle_lifetime into +0x8c and system_lifetime into +0x88. FUN_00a65be0
(0x00a65be0, size 0x248) reads particle_rate and stores its reciprocal at +0x70,
evidence that it controls emission spacing. Zero-rate behavior needs a separate
start_time/end_time after switching on modifier type. Window time units and whether
the windows use particle-normalized or system time remain unverified.

PhysicalForce=0, PhysicalDamping=1, VisualAnimation=2, VisualRotate=3,
VisualSize=4, VisualColor=5, VisualOpacity=6, VisualEmissive=7. Preserve source order,
window fields and raw parameters. Negative opacity values occur in retail modifiers;
they are not necessarily final alpha values. Do not clamp them during parsing or
replace modifier execution with generic endpoint interpolation without tracing it.

## Bounded fixture and atlas/space uncertainties

Use `lave_mc_damage_smoke.bin` with `fx_vehicle_mc_smoke_01.psmb`, then its referenced
`fx_vehicle_mc_smoke_02.psmb`. The graph's particle component names smoke_01; root
#841408bf names smoke_02. That relationship is verified, but whether it is child
spawn, trail, death-spawn or another phase is unknown. Stage the first milestone as
a named leaf viewer, with that unsupported relationship explicitly reported, then
add its semantics before claiming the whole original smoke graph works.

smoke_01: fumefx_bomb_01.dds texture, lifetime 1, rate 25, velocity .4, size modifier
.2; physical force/damping, visual animation, two opacity windows and size are
present. smoke_02: same texture, lifetime 1, rate 100, velocity .2, size modifier .4.
Smoke animation fields include #9fba43a8=50, #c871c24e=30, #db53dd7d=30 and loop=0.
Their frame-count/start-frame/rate meanings are not proven. Outer #e751ee6f=8 and
#e9ef9e7c=4 also occur on the barrel fixture and may describe atlas layout, but that
is inference. Inspect DDS dimensions and the original UV setup; do not present these
numbers as decoded columns/rows. `fx_exp_barrel_01.psmb` uses fire.dds and references
many other named particle systems, making it a later transitive fixture.

Attachment space is likewise unresolved. Preserve each emitter's absolute composed
world matrix, source owner and authored local matrix. Class-specific local/world
simulation flags must be recovered before attaching all living particles to their
emitter. Moving a smoke source should usually change new emission positions while
old particles stay in their simulation space; this is a proposed runtime contract,
not verified retail behavior. Bone/vehicle sockets and child-system transforms need
separate tests.

## Verified destruction wiring and current implementation gap

Tile `31x14.nlz/31x14.nl.blo` contains CEffectPointEmitter nodes. A go129a instance
has epe.fire: effect_id #a90d6c5c=363; event_enable #8b3a416c listens to its stage.1;
event_disable #8161a636 listens to destroyed. epe.explosion for that same instance
uses effect_id 84 and listens to destroyed. Building gb010 has fire emitters using
ID29 and disable on fade/fade.N, plus explosion ID83. The local matrices show that
effects belong to actual authored locations on their owners. Preserve GUID scoping
so two instances do not start/stop each other's effects.

Original CEffectPointEmitter vtable at 0x00eb9c6c points +0x18 to FUN_006c93c0
(size 0x144). That loader reads four input hashes via DAT_00eb9574, done-output
#e472e3c3 and effect_id #a90d6c5c. It also reads another value via DAT_00eb9568.
The exact play/stop/restart semantics need the other vtable consumers; do not map
every input to enable simply because their sample strings match.

Current `jc2-formats/src/destruction.rs:231` infers an explosion from emitter name
containing "explosion" and matching its events. It extracts hidden model debris and
mass, but does not retain the effect graph or all staged fire lifetimes.
`jc2-game/src/explosion.rs` uses assumed fireball .7s, wreck-smoke 25s, puff interval
.2s and puff life 3s with sphere meshes. `destruction.rs` throws actual hidden model
pieces with default mass 100 and assumed speed 5; its debris entities have no decoded
fade-event lifetime. `combat.rs` creates coloured-cylinder tracers lasting .06s and
crossed-quad flashes lasting .05s. The retail bullet_tracer_dif.dds has a consumer
in text_00650000.c:6875, separate from the .psmb inventory. Weapon data already
contains tracer_fire_velocity and tracer_scale_factor; current fixed segment timing
does not recover that renderer. No general particle/effect runtime was found in
the inspected source.

## Proposed parser and runtime contract

```
EffectRegistry: id -> {name, filename}
EffectGraph: settings, components[type,id,variants,raw], routing[raw]
ParticleSystem: name, texture, particle_lifetime, system_lifetime,
                emission[rate,shape_raw,velocity_raw], modifiers[class,window,raw],
                references[hash,name], render_flags_raw
EmitterInstance: owner, source_transform, enabled, seed, clock, emission_remainder
Particle: owning_emitter, simulation_space, position, velocity, age, lifespan, frame
```

Formats agent owns graph/modifier decoding and transitive reference diagnostics;
rendering agent owns atlas sampling, blend/depth/distortion rules and batching;
world/destruction owns events and emitter owner lifetime. Physics/gameplay damage
and impulses remain authoritative independently of visual creation success, visual
quality, culling and budget. Collision-aware debris remains in physics; particle
debris is a separate visual type until the data proves otherwise.

First milestone: deterministic named smoke-leaf viewer + decoder, exact rate/lifetime
and texture resolution, diagnosed unknown atlas/window/space fields. Then original
graph routing, staged fire start/stop, moving owner, and destruction-event integration.
Do not replace the current effects until the bounded fixture can be compared visually.

Proposed acceptance: fixed seed reproduces spawn sequence; fractional emission survives
different dt splits; global/per-emitter limits deterministically drop or defer by
documented priority without changing damage/impulses; age/removal never leaks pools;
stop has explicit immediate-vs-drain behavior; tile unload/owner removal stops emission
and clears references; restart cannot duplicate loop sounds; atlas last frame and wrap
are correct; changed source motion cannot drag world-space survivors; two matching
object GUIDs stay isolated; smoke/fire/decal/dynamic debris cleanup are measured
separately. No runtime acceptance was executed in this pass.

## Reproducible commands

Run from the game folder using the ignored private copy:

```
jc2-rs/_extract/particles-ik-research/jc2tool.exe sarc . particles.blz
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . effectsettings.bin --json
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . lave_mc_damage_smoke.bin
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . particles.blz/fx_vehicle_mc_smoke_01.psmb
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . particles.blz/fx_vehicle_mc_smoke_02.psmb
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . 31x14.nlz/31x14.nl.blo --json
jc2-rs/_extract/particles-ik-research/jc2tool.exe hash particle_lifetime system_lifetime particle_rate start_time end_time effect_id
```

## JC2-014 bounded runtime fixture — 8 October 2026

`jc2-formats::particle` now retains the complete parsed property tree alongside
ordered typed effect components and particle modifiers. The opt-in
`--particle fx_vehicle_mc_smoke_01` fixture verifies a unique type-1 particle
reference in **lave_mc_damage_smoke.bin**, resolves the compiled
`particles.blz/fx_vehicle_mc_smoke_01.psmb`, then the exact referenced
`textures\particles\fumefx_bomb_01.dds` member. The original DDS is DXT5,
1024×1024. Registry ID 150 belongs to the distinct **lave_damage_smoke.bin**;
this implementation does not claim that registry ID selects the MC graph.
Component IDs, unknown routing strings, all modifier parameters, negative opacity
and the smoke02 link remain lossless. Sound/decals and child emission are not run.

The earlier 8×4 atlas suggestion above is superseded by native consumer evidence:
`DAT_00f12128` contains hash **b0a2484f**, written as a short at config **+168**;
`DAT_00f1212c` contains **e751ee6f**, written at **+16a**. Both fields are integer
8 in smoke01. **FUN_00a5b430** computes inverse values of these two shorts and
same. **e9ef9e7c** (`DAT_00f12164`) is instead a float written at **+180** and
its value 4 is not an atlas row count. `particle-fixture-probe.py` checks the DAT
bindings in the local PE without copying executable/export data. The checked
retail executable is 14,549,656 bytes, SHA-256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.

**FUN_00a65a40** reads the VisualAnimation raw values via DAT f12060/f12064/
f12068/f1206c (hashes c871c24e/9fba43a8/db53dd7d/59a79825) into modifier
integer slots +0/+4, float +12 and integer +16. The original values 30/50/30/0
are preserved. Their execution, modifier window units, source-local/world flags,
shape-5 direction/spread and render-blend flags are not established here. No
universal interpolation is inferred. The viewer selects a **static** row-major,
top-origin cell (default 30, selectable 0..63); native frame orientation/playback
is still unsupported. Bevy Rectangle UVs use its top-right/top-left/bottom-left/
bottom-right vertex order. Unlit alpha blending, 0.45 m quads, constant 0.7 alpha,
0.035 m deterministic source scatter and +Y 0.4 m/s motion are explicit preview
policies. They are not visual parity claims or native modifier execution.

The pool uses a fractional birth clock (`ordinal/rate`) rather than rounding
births per frame. Events sort by time then owner identity, expire before each
budget check, and drop new arrivals when global/per-owner budgets are exhausted.
The default fixture uses 64/32 particles; the viewer caps supplied budgets at
4096 and the pool accepts at most 64 owner instances. Seeds are stateless by
birth ordinal; dropped arrivals are consumed, not deferred. Zero rate produces
no births and zero budgets produce no particles. Rate/lifetime/finite transform
validation rejects unsupported configurations; nonzero system-lifetime modes
are retained by the decoder but unsupported by this preview.

For ordinary steps, partitioning time with the same linearly moving source gives
the same births, seeds and budget decisions. The owner world transform composes
its authored local source offset, with translation/scale interpolation and
rotation slerp between supplied frames; particles already born remain world
space. The standalone viewer has no hierarchical/gameplay attachment consumer.
A pathological stall executes at most 4096 birth events, explicitly drops the
rest and consumes their ordinals. Such overload dropping is deliberately not
frame-partition invariant. Unrepresentable finite clocks/counts fail before
mutation instead of overflowing or queuing delayed bursts.

Space stops emission but drains live particles; R restarts with a fresh source
clock and clears that instance's old particles; X destroys the final owner and
removes its pool state and drawable entities. Entity generation participates in
owner identity so replacement entities cannot inherit stale particles. The
`--particle-stop-after` clock is split at its exact simulation-time boundary;
remaining time in that frame drains without further births. Shared mesh/material
handles are allocated once per viewer, not once per particle. No audio bank,
looping sound, graph-recursive owner or gameplay damage dependency is introduced.

Validation on isolation46 with the actual retail `JC2_TEST_DIR`: all five new
particle tests passed, followed by **377 release workspace tests** (144 formats,
233 game), strict release clippy across all targets with `-D warnings`, Cargo
format check and release workspace build. Original-data tests exercised the
real MC graph/leaf/DDS and full authored `CParticle...` class strings, lossless
raw fields, deterministic clock/composed moving local source, zero/instance/
stall/huge clocks, actual ECS drawable cleanup and replacement generations.
The real ExplosionPlugin produced identical **60 HP / one blast** with particle
budgets zero and 32. No original-data case was skipped in this run.

Private executables were copied while the shared lock was held, then Cargo was
released before GPU acceptance. Both GPU launches exited 0 on the Radeon RX9070XT
Vulkan adapter and their captures were inspected:

- `jc2-particle46.exe GAME --particle fx_vehicle_mc_smoke_01 --exit-after 20
  --screenshot _extract/particle-fixture/live46.png`: visible original smoke
  texture, **Live 25**, zero budget/stall drops, `emitting`, **DDS loaded**.
- `jc2-particle46.exe GAME --particle fx_vehicle_mc_smoke_01
  --particle-stop-after 3 --exit-after 8
  --screenshot _extract/particle-fixture/drained46.png`: empty preview,
  **Live 0**, zero drops, `draining`, **DDS loaded**. With `RUST_LOG=info`, the
  drain log confirms the exact `particles.blz/fumefx_bomb_01.dds` Image descriptor
  **1024×1024 Bc3RgbaUnormSrgb** and successful saved capture, with no WARN/ERROR.

The ignored capture/log/private-binary paths above are under the
`.claude/worktrees/codex-particle-fixture` worktree. A private `jc2tool particle
GAME` diagnostic confirmed the exact graph/leaf/texture chain and ordered raw
modifier fields. GPU ownership was released after both processes exited.
These establish bounded loaded-texture preview and drain behavior, not retail
visual parity, interactive control coverage or native modifier/shape/space/
material execution.

The viewer logs the actual loaded Image descriptor and shows `DDS loaded` only
once its referenced image asset is present, allowing GPU captures to distinguish
loaded retail texture from the loading phase. Native execution follow-up is
tracked as **JC2-080** (`34bb0c8f-88ec-4d5b-957d-0ed10e5ae49f`), dependency014:
trace and measure windows, frame playback, shape5/space/material consumers before
enabling them. Graph routing and gameplay integration remain outside that scope.

JC2-080's [native consumer handoff](particle-consumers-next.md) now establishes
indexed modifier execution, updated normalized-age windows, signed incremental
opacity, birth-frame variation, frame advance/clamping with nearest-even shader
selection, shape5 radial input, MC local space2, and dual-source blend factors.
Its read-only probe and original boundary references distinguish these facts from
remaining fitted geometry, source direction/RNG, job timing and full material
parity requirements. Earlier uncertainties above are historical inventory;
the handoff is the current consumer evidence. The014 runtime is unchanged.
