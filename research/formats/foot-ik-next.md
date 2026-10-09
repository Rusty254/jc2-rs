# Foot/hand contact constraints — 8 October 2026

Research only, following `notes/RESEARCH-HANDOFFS.md` assignment 5. This extends
animation-next.md with authored limb-weight fixtures and original consumers;
it does not repeat its AFSM/event-track inventory. Read STATUS and current rig.rs.
JC2-016 continuation adds confirmed consumers and a read-only reproduction probe.
No Cargo, game launches or runtime edits. Local retail data and decompiler
exports are primary sources. Diagnostic tool copy is ignored under
`_extract/particles-ik-research/`.

## Verified: two separate original limb mechanisms

Original RTTI alone was previously the lead. The original constructor now gives
concrete separation: `research/analysis/pseudo/_unnamed/text_00870000.c`,
LeftUpLeg/LeftLeg/LeftFoot and RightUpLeg/RightLeg/RightFoot, naming them
LeftFootIK and RightFootIK. It separately builds IIKFootPlacement controllers
with those same bone triplets, a shared owner transform, owner identifier and an
through FUN_008e9770. Ground foot placement and authored named FootIK constraints
are not automatically the same controller.

The two-joint setup uses a (1,0,0) axis, calls FUN_004a9650 with float bits
0xbf7d70a4 (~-.99) and FUN_004a0750 with 0x3f733333 (~.95), then assigns left/right
values 4/8 through FUN_004a0250. Those may be knee angle bounds and target selectors;
that interpretation is not established. The constructor proves the bone bindings
and separate controllers, not the interpretation of every constant.

CIKFootPlacementImpl vtable 0x00f24a90 has slot +4=0x004b9210. That function
(size 0xd8) reads a model-space bone transform from a pose, copies it to +0xb0,
subtracts the float at +0x138 from its vertical translation and invokes
FUN_00b09fd0 with that transform, parameters at +0x130 and the pose. Thus there
is concrete vertical correction machinery. Whether +0x138 is a pelvis correction,
shared feedback or some other offset remains unknown; no original pelvis algorithm
is claimed here.

CFootPlacementRaycaster vtable 0x00f24a84 has slot +4=0x004cabd0 (size 0x1ac).
It constructs a ray from two 3D points, calls FUN_009a9a50 with their distance,
returns a normal-like vector, a fraction and hit iff fraction<1. The decompiler
has uninitialized-looking temporaries; retype its ray structures before interpreting
filter masks and moving-body output. CFootPlacementRaycastFilter exists, but this
initial pass did not recover its exclusions or support-body tracking. The
continuation below recovers its raw predicate/owner rejection and API limits.

the raycaster/filter and an hkaFootPlacementIkSolver via FUN_00b09a90. It contains
values .35/.35/-.35, .8, .3 and .2 among many offsets. They are verified literals,
not decoded joint, smoothing or ray-distance parameters. Do not copy them as
tuning constants without struct mapping and unit verification.

## Verified authored weights and timing fixtures

`animations.blz` has 101 `.acsb` members. All were inspected for named FootIK/HandIK
constraints. Numerous vehicle/heli/hanging curves exist; use them before making
up per-state weights. Existing PropertyFile parsing is sufficient.

ACSB records have name, #7aff9210="constraint_relation" and nested #84205939
(exact hash of `weight`) with child time/weight keys. Key enumeration is hash order,
not temporal order. Stable-sort by (time, original ordinal), as for event tracks.
Time values in these fixtures correspond naturally to clip seconds and weights
span 0-100; interpreting them as seconds/percent is strongly suggested, but original
curve interpolation and its final blend application still require consumer tracing.

| Fixture | Verified authored values |
|---|---|
| left_hand_ik_100.acsb | LeftHandIK=100 at time 0 and 1000 |
| mc_racer_stunt_land.acsb | Both FootIK=0; both HandIK=100 at 0, .53333336, .56666666 |
| heli_cling_dwn_idle.acsb | Both FootIK=100 at 0, 1.3, 1.3333334 |
| npc_heli_left_sit_to_stand.acsb | RightFootIK 0 through .36666667, 33.333332 at .4, 66.666664 at .43333334, 100 from .46666667; LeftFootIK 0 through .16666667, ramps in .2-.36666667; LeftHandIK has a different nonmonotonic curve |

The helicopter left-hand curve is especially useful: 0 at .06666667, 50 at .1,
100 at .13333334, decreases to 0 at .4, then returns to 100 at .5. One global
"contact weight" would erase this hand release/regrip behavior. ASB descriptors
reference left_hand_ik_100.acs under #22d6df42 and #376f89ec, including hijack and
vehicle animations. Follow the existing ASB/ACSB reference resolver contract rather
than hard-coding an ACSB by clip filename.

These are authored **two-joint constraint weights**, not proven ground planting
windows. For example heli_cling_dwn_idle has full FootIK while attached to a helicopter.
Likewise animation-next.md's upright_basic footstep/FX events are phase evidence,
not foot-lock intervals. `hjk_getin` foottouch is one authored event, not an interval.

## Remaining original decoding tasks

size 0x27c) in text_004b0000.c. Its stored hash words, read from the original exe:

| Hash word address | Hash | Destination in implementation |
|---|---|---|
| 00f246b8/bc/c0 | e5a43194 / 6ad671fd / c56a19a6 | +34/+36/+38 (16-bit bone-like indices) |
| 00f246c4/c8 | 0d1dac62 / ee9acc15 | +40/+50 (vectors) |
| 00f246cc/d0/d4/d8 | 637b31d1 / 486d8c94 / d2f36d02 / e769e888 | +84/+88/+8c/+90 |
| 00f246dc/e0/e4/e8 | b44c657f / 31da1a52 / 6b15521b / 3687e42e | +94/+98/+9c/+a0 |
| 00f246ec/f0/f4/f8/fc | b72e8352 / 49351b4c / 36a15136 / 867dae80 / cccc6ed7 | +118/+11c/+120/+128/+124 |

Property names and source tuning asset remain unresolved. The continuation maps
several fields to direct operations/units rather than guessed property names.
The consumer rebuilds the
Havok solver from +0x30 after changing its parameters. Useful Havok reflection
strings exist at 0x00f7049c/0x00f704b0 (raycastDistanceDown/Up), 0x00f70558/6c
(cosineMin/MaxKneeAngle), 0x00f70580/94 (min/maxAnkleHeightMS), 0x00f705a8/c0
(footRaised/PlantedAnkleHeightMS), 0x00f705dc/e8 (footEndLS/kneeAxisLS), and
0x00f7063c (pelvisFeedbackGain). These are library reflection leads, not proof that
the Avalanche hashes above have those names or identical meanings.

Unknown: retail ground-lock windows, solver weights across blended layers, target
socket selection, body filters, ray up/down units, feedback clamp/smoothing, pelvis
ownership, moving-support persistence, limb scale adaptation, ankle orientation,
and unreachable-target handling. Trace these explicitly; Havok behavior strings
alone cannot establish how this game configures its own wrapper.

## Current runtime gap and proposed ownership

`jc2-game/src/rig.rs:548` defines AimIk: spine/head aim. Animator samples/blends
bone transforms, then applies this aim in rig.rs around line 856. No leg ground
raycasts, named FootIK/HandIK solver, support contacts or pelvis correction exists
in the inspected rig path. Static aim-chain shares derived from ACSB evidence are
not a general ACSB constraint evaluator.

Proposed explicit contract, coordinated with the animation agent before editing rig.rs:

* Physics/controller owns capsule motion, gravity, collision and authoritative support
  body. IK must not write capsule/root-body position to correct a visual foot.
* Animation owns base local pose, layer time/rate, blending and authored constraint
  curves. It exposes pre-IK model-space joints and evaluated named weights.
* Contact acquisition owns world ray results, normals and an optional support entity;
  self/character filtering and distance limits are explicit. Ground and vehicle/socket
  target acquisition are separate.
* On a moving support, store the contact in support-local coordinates and derive its
  world target from the current support transform; never parent the whole rig to a
  foot. This is a proposed design, not recovered original behavior. Static contacts
  may use cached world points. Invalidate on teleport, detach, despawn or ownership
  change before reuse.
* Rig solver transforms world targets into HeroModel/rig model space with the full
  current transform, derives limb lengths from the appropriate reference/current
  scaled skeleton, solves hip-knee-ankle or arm chains, and writes local rotations
  after parent-space conversion. Prevent double application of coordinate conversion
  or controller/root motion. Ground queries use world metres; data geometry conversion
  follows coordinates.md.
* Bounded pelvis correction is a visual model/bone operation. Choose one named owner,
  calculate it once for the paired legs, then solve both chains. Keep aim, hands,
  stunt contact and ragdoll evaluation ordering explicit; rigid attachment/root motion
  remains with its existing owner. Disable/fade ground IK when air/swim/climb owns
  motion; do not erase vehicle-contact FootIK curves at the same time.

Proposed typed schema: ConstraintCurve{name,kind,ordered keys,raw},
LimbChain{hip,knee,ankle,bend_axis,limits,lengths},
Contact{target_space,point,normal,support,last_valid_frame},
GroundIkState{leg weights,paired pelvis offset,contacts,enabled}.
Keep unknown curve interpolation and solver tuning separately diagnosed.

## First coding milestone and meaningful validation

First implement read-only curve diagnostics for the helicopter sit-to-stand fixture,
bone-name mapping and world/model/local conversion diagnostics; confirm timing at
rate changes and transitions. Then one two-bone target solver, followed by paired
ground contacts and bounded pelvis adjustment. Use visual debug rays/targets and
metrics before enabling it for all NPCs.

Controlled fixtures: flat surface baseline; slope with nonvertical normals; one
stair supporting only one foot; different two-foot heights; translating and rotating
platform; platform removal and teleport. Add helicopter sit-to-stand/full FootIK and
the hand release/regrip curve as a separate moving socket fixture.

Acceptance: valid joint lengths/finite transforms; no knee flips or overextension;
local reconstruction reaches reachable world targets; unreachable targets clamp
without pulling physics; body trajectory identical with IK on/off; planted contacts
do not skate with support motion; no cached target after support removal; pelvis
applied exactly once and smoothly bounded; airborne transitions release ground
contacts; event footsteps never implicitly create lock windows; blended-layer weights
are visible and never cancel an unrelated hand target; update-order and frame-rate
changes do not double-transform supports. No runtime validation ran in this pass.

## Reproducible commands

```
jc2-rs/_extract/particles-ik-research/jc2tool.exe sarc . animations.blz
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . mc_racer_stunt_land.acsb
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . npc_heli_left_sit_to_stand.acsb
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . heli_cling_dwn_idle.acsb
jc2-rs/_extract/particles-ik-research/jc2tool.exe prop . left_hand_ik_100.acsb
jc2-rs/_extract/particles-ik-research/jc2tool.exe hash weight
```

## JC2-016 continuation: ground-placement consumer contract

All native addresses refer to retail `JustCause2.exe`, SHA256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
Static native/export checks and mathematical diagnostic fixtures below do not
claim original solver execution, gameplay activation or rendered parity.
JC2-017's `weapon-sockets-next.md` separately confirms authored A/B hand-IK
targets and weapon ownership branches; continue those in JC2-045, not this ground
solver. Animation clock/transition delivery belongs to JC2-026.

### Wrapper pose input and result ownership

**0x004b9210**, bound at CIKFootPlacementImpl vtable **0x00f24a90+4**, refreshes
the owner transform when **implementation+0x140** is nonzero. Setter
**0x004ab9a0** assigns this pointer; character construction **0x0087dc80** passes
copies the 4x4 matrix into Havok transform form; **0x00b26690** stores its
translation/quaternion/scale at **implementation+0xe0**. Native ECX setup
`lea ecx,[esi+0xe0]` confirms the destination omitted from pseudocode.

The wrapper reads the solver ankle index at **solver+0x18**, accesses the pose's
model-space transform (or calls **0x00afff80** when its model cache is dirty),
and copies its 0x30-byte QsTransform to **implementation+0xb0**. It subtracts
**implementation+0x138** from the ankle's model-space y, then calls
**0x00b09fd0** with contiguous input at +0xb0, result at +0x130 and the pose.
The input includes the owner world transform at +0xe0 and raycaster at +0x114.
This is a model-pose/world-query conversion, not a support-body transform.

The result consists of scalar **+0x130** and byte **+0x134**. Native return
instructions compare that byte and return whether it is nonzero; the void
pseudocode signature loses this result. The solver writes scalar vertical
error as ground height minus owner height/ankle offset, and clears it on a failed
or inactive solve. **+0x138 is a separate input correction**, not this output.
Its writer/paired-leg aggregation and any pelvis consumer remain unresolved.
No confirmed path here writes the character capsule or physics root; do not
promote a visual pose correction into controller motion.

**0x0049d850** sets input enable byte **implementation+0x110**. Construction
disables both legs with zero. **0x00899c90** updates both through one shared
argument, proving paired enable ownership, but the gameplay callers deciding
when to enable still need tracing. **0x004ae560** sets +0x13c and **0x004c80a0**
adds/clamps it to [0,1]; no direct scalar-to-solver use was recovered here.
Do not call it a pelvis weight merely from its adjacency to vertical error.

### Raw tuning mapped through actual solver operations

**0x00b09a90** copies 0x80 bytes from implementation+0x30 to solver+0x10.
**0x004bedf0** reloads raw properties, then destroys/rebuilds the solver from
that same settings block before updating input gains. The subnode selector at
**0x00f246b4** is **#3da29d18**; its retail source asset/name is unresolved.
The following meanings are inferred from direct operations in 0x00b09fd0 and
0x00b09ce0, not from assigning Havok reflection strings to Avalanche hashes.

| Raw hash | Implementation / solver offset | Confirmed operation / units |
| --- | --- | --- |
| #e5a43194 / #6ad671fd / #c56a19a6 | +34/+36/+38 -> +14/+16/+18 | Hip/knee/ankle 16-bit indices; named setters 0049c460/0049e030/004c2600 bind the constructor's corresponding bone triplets directly into the solver |
| #0d1dac62 | +40 -> +20 | Local bend-axis vector passed to two-joint solve; dimensionless direction |
| #ee9acc15 | +50 -> +30 | Foot-end local point transformed alongside ankle; skeleton distance units |
| #637b31d1 | +84 -> +64 | Low-height threshold for planted-state decision, compared with ankle height along model up |
| #486d8c94 | +88 -> +68 | High-height endpoint in `1-(ankleHeight-low)/(high-low)`; denominator must not be assumed nonzero for malformed data |
| #d2f36d02 | +8c -> +6c | Upper ankle target height clamp along model up |
| #e769e888 | +90 -> +70 | Lower target-height test; too-low target suppresses active solve |
| #b44c657f / #31da1a52 | +94/+98 -> +74/+78 | Two-joint limits passed unchanged to 00b09440; cosine-like dimensionless values, no degree conversion in wrapper |
| #6b15521b | +9c -> +7c | Ray start distance above candidate along world up |
| #3687e42e | +a0 -> +80 | Ray end distance below candidate along world up |
| #b72e8352 | +118 input | Active blend gain in `blend += gain*(desired-blend)`; no delta_time factor in this solver operation |
| #49351b4c / #36a15136 | +11c/+120 input | Opposite ground-height smoothing branches; dimensionless gains multiplied by height-derived blend |
| #867dae80 / #cccc6ed7 | +128/+124 input | Lower/upper clamps on height-derived blend, respectively |

World/model up vectors at implementation+0x60/+0x70 default to [0,1,0].
The wrapper defaults give +0x84 about .064, +0x88=.35, +0x8c=.35, +0x90=-.35,
+0x94=-.9, +0x98=.8, +0x9c/+0xa0=1.0; input gains +0x118=.3,
+0x11c/+0x120/+0x124=1, +0x128=.2. These are construction defaults, **not**
proof of effective retail tuning after reload. Distances share the skeleton/world
length convention; absolute SI metres remain the current reconstruction's
coordinate convention, not an independently established native asset unit.
The default constructor settings can differ from general Havok defaults.

### Raycaster acceptance, output and support identity

**0x004cabd0** is bound at CFootPlacementRaycaster **0x00f24a84+4**.
Native code saves ECX, builds a ray from the two supplied world points and calls
**0x009a9a50** with Euclidean segment length and collector at **raycaster+0x10**.
The collector is the embedded CFootPlacementRaycastFilter at
**implementation+0x160**. The raycaster resets fraction to 1 and copies back
fraction plus three normal components (normal.w=0). Hit is strictly **fraction<1**;
a hit exactly at the end is rejected. This interface has no returned support
entity, local body point, velocity or contact lifetime.

Filter vtable **0x00f248dc+8 -> 0x004b1950** invokes the candidate's virtual
slot +0x18 with raw category selectors **1** and **3**. At least one must return
true, then **candidate+0xc must differ from filter+0x90**. Setter
**0x0049f3f0** writes the owner identifier to implementation+0x1f0, exactly that
filter+0x90 location. **0x009b1d60** calls this filter slot and returns rejection
when the callback returns false; **0x009a2a90** uses that rejection before updating
the closest hit. This proves owner exclusion and raw category gating. The
category-to-class mapping remains unknown: do not label selector 1 as terrain or
3 as vehicles from its number. Geometry lacking the queried owner metadata has
other branches in the general ray collector, so do not claim this excludes every
character collider hierarchy or every trigger automatically.

**0x00b09ce0** constructs the primary ray as candidate+worldUp*upDistance to
candidate-worldUp*downDistance. A second foot-end ray is used when its point
differs; a normal/up dot-product gate participates in that branch. Full fallback
selection/normal arithmetic contains untyped temporaries, so no guessed slope
cutoff or averaging formula is proposed.

### Planting state and world/model handling

**0x00b09fd0** has persistent state: solver+0xb4 planted flag, +0xc0 cached point,
+0xd0 cached auxiliary vector, +0xe0 release displacement, +0x90 active blend and
+0x94 prior height. Input enable and a settings flag at solver+0x84 gate this
plant path; low authored ankle height starts/retains a cached target. Cache values
participate in world-ray construction and are transformed back with the inverse
owner transform before pose solving. This proves an internal height/state planting
mechanism; named FootIK curve values and footstep events do not establish its
lock windows. Native ground planting is not simply a held footstep event.

On release, the solver forms the cached-current displacement, removes its world-up
component, and decays it by `1-inputGainAt+0x12c`. Height smoothing and active
blend have their own gains; active blend below .01 snaps to zero. The solver calls
**0x00b09440** for hip/knee/ankle, converts the ray normal into model coordinates,
blends ankle orientation and writes the pose ankle rotation through
**0x00b00db0(index,1)**. The query is world space, the pose output is model/local
pose data, and owner scale is included in the inverse transform calculation.
No support body's transform/id is supplied to this solver through the recovered
ray interface. Consequently **support-local persistence is still a proposed
reconstruction design**, not recovered original moving-platform behavior.

### Concrete diagnostics and ownership fixtures

These inputs are deliberately synthetic, with distance in reconstruction metres;
they are mathematical/query diagnostics, not retail tuning or game observations.

| Fixture | Exact input / expected diagnostic | Ownership requirement / limit |
| --- | --- | --- |
| Flat | Ray y=1 to -1, ground y=0, fraction=.5; normal [0,1,0] | Convert world hit through current owner inverse; physics trajectory unchanged by visual pose solve |
| Stair | Same ray, one foot ground y=.2 -> fraction=.4; other foot y=0 -> .5 | Keep independent per-leg result/error; paired pelvis owner still unresolved, never sum/apply twice |
| Slope | Plane y=.5*x at x=.5 -> hit y=.25, fraction=.375; normalized normal [-.5,1,0] | Preserve hit normal and target space; no native slope threshold claimed |
| Ray endpoint | Same ray, ground y=-1 -> fraction=1 | Native wrapper rejects this end hit, rather than accepting <=1 |
| Owner filtering | Selector1 or3 true plus equal owner id -> reject; different id -> accept; neither predicate -> reject | Tests raw callback truth table, not guessed physical category names |
| Moving support proposal | Local point [1,0,0], support R_y(pi/2) plus [3,.2,4] -> world [3,.2,3]; inverse recovers [1,0,0] | Requires explicit support identity acquisition, unavailable in recovered ray API; cached world [1,0,0] alone cannot follow it |
| Support removal/teleport | Retire generation before reusing previous target | Proposed invalidation contract; native persistence/invalidators unresolved |
| Helicopter contact | Full named FootIK in heli_cling_dwn_idle; sit-to-stand hand 0 at .4 then100 at .5 | Named two-joint socket weights remain independent of enabled ground planting |

Animation owns evaluated authored weights and sampled pose; ground acquisition
owns world-ray results and filtering; the visual solver owns its pose writes.
Existing physics/controller retains root motion. Frame clock and blended-layer
weight routing require JC2-026; exact hand target ownership requires JC2-045.
Any future pelvis correction must identify one proven owner of +0x138/result
feedback before integration. The moving-support design fixture cannot certify
native platform parity or be used as a reason to attach the whole rig to a foot.

### Reproduction and remaining limits

```powershell
python -B research/tools/foot_ik_consumer_probe.py --game 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2' --tool 'C:/Users/Rusty/AppData/Local/Temp/jc2tool-material-water-research.exe' --export 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/research/analysis/pseudo/_unnamed'
```

Observed **PASS**: retail executable fingerprint, three vtable bindings, all 18
reload hash words, native vertical-correction/result/owner-comparison instruction patterns,
eight function/export consumer contracts and four real authored curve documents.
The probe stable-sorts curve keys by time/original ordinal and verifies the hand
release/regrip and full heli FootIK counterexample. It also checks four synthetic
ownership fixtures plus ray-end rejection and five raw filter cases, explicitly
labeled design/math.
JSON reports native function SHA256 blocks and exact curve values; no assets are
extracted or committed. Original math/game execution and visual validation did
not run; no Cargo or runtime files changed.

Unresolved: source tuning asset/raw property names; full enable callers and flag
configuration; +0x138 writer/paired pelvis feedback ownership; slope fallback
arithmetic; ray selector class names and all collider exclusions; source/output
weight routing from blended layers; original support-motion/teleport invalidation;
limb-scale/unreachable behavior and exact NPC activation. These are bounded
consumer gaps, not reasons to rename every reflection string as an authored key.
