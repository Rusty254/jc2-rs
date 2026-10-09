# Rico articulated ragdoll preparation — 8 October 2026

## Decoder checkpoint (verified, awaiting integration)

The `codex/ragdoll-decoder` milestone adds `jc2_formats::ragdoll::Ragdoll`
and `jc2tool ragdoll <game> <name.brd>`. It uses checked cross-section fixups
and original reflection to select instance membership, inherited rigid-body
motion fields, original skeleton identities, mapper directions/simple/chain
transforms and original hinge/twist/cone/plane limits. Bodies carry mass/inertia
in inverse form, center of mass, rest pose, shape and damping; joint descriptors
retain unsupported motor/friction atom identities instead of inventing behavior.

An independent read-only Python probe confirmed **normal membership is 14
bodies/13 joints** (7 ragdoll, 6 hinge) and **stunt membership is 16 bodies/15
joints** (11 ragdoll, 4 hinge). Extra reflected constraints are outside the
instance array. Bone-to-body maps are identity in these two originals. Both
selected ragdoll skeletons are named `ragdoll_Hips`; selecting by the display
name alone would be ambiguous across files. Maximum rest-pivot disagreement is
about 3.55e-7 m using the decoded transforms. These are data checks, not runtime
physics acceptance. Release workspace tests passed **315** (131 formats,
184 game), matching source test markers; release workspace/all-targets clippy
completed without warnings. Both original BRDs inspected successfully using
the private tool built/copied under the lock, isolation 35, plain game assets
in `JC2_TEST_DIR`. Invalid counts, mapping indices/duplicates, repeated/self
constraints, NaN values, nonrigid bases and invalid QS quaternions are rejected.

Tool inspection also confirms both mapper directions: normal Reference→ragdoll
has 14 simple mappings; reverse has 14 mappings and 70 unmapped render bones.
Stunt has 16 simple mappings each direction and 68 unmapped render bones in
reverse. Original chain arrays are empty; chain decoding has reference/range
checks but no positive original-data fixture. Selecting by raw skeleton identity
and source/target name keeps Rico separate from baby/civilian mappers.

Explicit remaining decoder work: motor/friction parameter payloads, collision
filter metadata and profile semantics. Original activation/recovery consumers
remain separate research; no articulated gameplay is implemented here.

This is an implementation brief, not implemented physics. The current downed
fallback freezes an Idle pose and topples the visual model as one piece. The
next milestone is independently simulated, constrained limbs driving the
existing skinned mesh. Research used the plain Steam install, current source,
original RTTI/string exports and locally installed Avian 0.7.0 source. No
Cargo builds, game launches or runtime edits were performed.

## Original assets: concrete starting point

`ragdoll_params.bin` already parses with `jc2tool prop`. It declares
`default_group = normal_male`, maps the `Reference` skeleton to that group,
and references two actual top-level assets:

| Asset | Bytes | Reflected object inventory |
|---|---:|---|
| triplemapped_normal_ragdoll.brd | 104080 | 14 rigid bodies; 14 ragdoll constraint-data objects; 12 limited-hinge data; 26 constraint instances; 4 skeletons; 6 skeleton mappers |
| stunt_ragdoll_positionmotor.brd | 88080 | 16 rigid bodies; 22 ragdoll constraint-data objects; 8 limited-hinge data; 30 constraint instances; 2 skeletons; 2 mappers |

Both start directly with Havok magic, without a PFX wrapper. Each contains one
`hkaRagdollInstance`, one `hkpPhysicsSystem`, one `hkpPhysicsData`, one
`hkaAnimationContainer`, and one `hkpPositionConstraintMotor`.

**Both BRDs retain reflection:** `__types__` contains 34160 payload bytes, with
89 `hkClass` and 13 `hkClassEnum` objects. Do not assume the stripped-reflection
description in collision notes applies here. Recover exact member offsets,
array forms, enums and atom layouts from this reflection instead of guessing
them from a newer Havok version. Inventory helper:

```powershell
jc2tool extract '<plain game>' '<private scratch>' --filter '.brd'
python research/tools/brd_inventory.py '<scratch>/triplemapped_normal_ragdoll.brd'
```

The helper inventories virtual-fixup objects only; it does not decode membership
or joint parameters. Notably constraint-instance counts exceed the simple
tree's expected number of edges. Traverse the arrays in `hkaRagdollInstance`
and `hkpPhysicsSystem` and identify enabled constraints before instantiating
them. Never create a joint for every reflected object indiscriminately.

Normal BRD contains readable body/bone names for Hips, Spine, Spine1, Head,
left/right UpLeg, Leg, Foot, Arm and ForeArm, with `ragdoll_` prefixes. This is
strong correspondence evidence, not proof that stripping the prefix supplies
the complete binding. Multiple skeletons/mappers exist for Reference,
civilian and baby groups; choose the mapper for Rico's `Reference` skeleton.
`Skeleton::parse` currently selects the first `hkaSkeleton` object; it must
gain object/name selection before reuse with these multi-skeleton files.

The parameter file defines named profiles, including `AirborneRagdoll`,
`PosematchingFlightProtect`, `UncontrolledSkydive`, and mode strings
`POWERED_CONSTRAINT` / `RIGID_BODY`. Numeric profile keys are largely unresolved.
Its file-descriptor `amount` values are 21 and 8, **not** the observed BRD body
counts; their meaning must remain unknown. Passive fatal death is a bounded
first runtime target, followed by powered fall/flight/recovery behavior.

## Evidence trail in the original executable

`research/analysis/classes.csv` lists `NAnimationSystem::CRagdollInstanceImpl`,
`IRagdoll`, `CRagdollDescription`, `CRagdollProxyPhantom`,
`CPfxRagdollProxyPhantom`, `CRagdollCollisionListener`,
`hkaRagdollInstance`, `hkpRagdollConstraintData`, `hkpLimitedHingeConstraintData`
and `hkpRagdollLimitsData`. Animation conditions include ragdoll velocity,
timer, blending and settled checks. Strings include `RagdollInstance.cpp`
at `0x00e2d348`, `ragdoll_` at `0x00e2dcf8`, and `ply.ragdoll.getup` at
`0x00e52680`; these are useful bounded Ghidra xref entry points.
RTTI presence does not prove a particular death impulse, joint mapping or
recovery rule. No such consumer behavior was recovered in this task.

## Decoder milestone, independently reviewable

Add a formats module reusing `havok::Packfile`, the existing shape decoder and
`Transform3` conversions. Current collision support exposes rigid-body shape
at +16 and transform at +224, but `CollisionBody` does not provide enough
ragdoll mass/inertia/joint metadata. Do not treat those known offsets as a
complete rigid-body layout. Use BRD reflection for:

- Rigid-body identity, rest transform, center of mass, inverse mass/inertia,
  damping, shape, filter and motion settings.
- Selected ragdoll skeleton, body ordering, constraint ordering and mapper
  source/target skeletons, simple mappings, chain mappings and unmapped bones.
- Joint pair identities, local frames/axes, limits, motors, wrapping/malleable
  constraints and disabled flags. Preserve unsupported class names and raw IDs.

Output owned typed descriptors plus diagnostics. Validate finite transforms,
positive mass/inertia where required, valid shape references, normalized axes,
known skeleton names, unique body mapping and closed references. Verify matched
rest-pose pivots agree in model space. Test malformed arrays and unresolved
references with synthetic bytes; original-asset tests require `JC2_TEST_DIR`.
Keep original BRD bytes and bulk decompilation outside commits.

## Avian 0.7.0: verified local capabilities and gaps

Pinned source is under the Cargo registry's `avian3d-0.7.0/src`. Relevant paths
are `dynamics/joints/{spherical,revolute,mod}.rs`,
`dynamics/rigid_body/mod.rs`, and `physics_transform/transform.rs`.

`SphericalJoint` exposes `frame1/frame2`, `twist_axis`, optional `swing_limit`
and `twist_limit`, plus point/swing/twist compliance. Its swing envelope is a
**circular cone**; an original ragdoll's noncircular/plane cone or independent
limits must not be presented as exact parity. `RevoluteJoint` supplies local
frames, hinge axis, angle limit and `AngularMotor`. Both accept
`with_local_frame1/2`. Defaults differ: spherical twist uses Y, revolute hinge
uses Z. Decode and transform the actual axes rather than rely on defaults.

`JointCollisionDisabled` disables contacts between a joint's connected bodies.
That covers adjacent pairs, not every undesirable self-collision pair; define
and test explicit group filtering. `JointDamping` and compliance are available,
but this research did not establish faithful Havok powered-ragdoll equivalents.
Do not promise position-motor parity from a passive spherical joint.

Avian stores world physics poses in `Position` and `Rotation`; Bevy `Transform`
under a parent is local. Avoid parenting physics limbs under the animated hero
root or moving vehicle. Keep physics bodies as world entities with owner IDs,
and explicitly reconstruct skeleton local transforms after simulation.

Use explicit lifecycle ownership rather than generic Bevy `Disabled` toggles.
Avian provides `JointDisabled`, `RigidBodyDisabled`, `ColliderDisabled`, but
their existence is not proof that arbitrary enable/disable order is safe for
joint graphs. First implementation should create/despawn a complete assembly:
remove owned joints, apply deferred changes, then remove bodies. Repeatedly
toggling `Disabled` on bodies with live constraints is not an acceptable design.

## World/body/bone reconciliation

Preserve inverse bind matrices and the render skeleton. For each driven bone
recover the fixed body-to-bone offset from the selected rest-pose mapper:
if `C = inverse(B_rest) * J_rest`, then runtime bone world pose is
`J_world = B_world * C`. Reconstruct local pose with
`inverse(parent_bone_world) * J_world`, processing the skeleton hierarchy in
parent order. The mapped skeleton may omit shoulder roll/fingers/face; propagate
unmapped descendants using a retained pose, not identity translations.
Validate round-trip rest pose, differing body pivots, nonidentity model/root
rotation and the current model's `-HEIGHT/2` vertical offset.

Capture the **current propagated animated pose** on activation. Initializing
every body in rest T-pose creates a visible snap and impossible anchor errors.
Map current bones back to bodies through the inverse fixed offsets, and start
constraints consistently with those world poses. Stop `Animator` from writing
the same skeleton while physics owns it. Choose system ordering explicitly:
capture after animation/propagation, create before physics, reconstruct after
physics, propagate before skinning. Inspect interpolation to prevent jitter or
double application of parent motion.

Inherit character linear/angular motion. For a moving support, body point
velocity includes `vehicle_linear + vehicle_angular cross offset`, not merely
the hero's cached velocity. Bound added hit impulses and choose application
body/point from a documented hit mapping; do not multiply the same impulse into
every limb. Preserve decoded mass proportions and total mass deliberately.

## Lifecycle integration and independent coding stages

1. Decoder/inventory agent: exact BRD membership, transforms, mapper and joint
   metadata. No gameplay integration required for this reviewable milestone.
2. Physics rig agent: passive world-space assembly with a debug view, rest/pose
   initialization, constraints and skinned pose reconstruction. Own a new
   `ragdoll` module; coordinate `rig.rs` integration with animation work.
3. Gameplay agent: fatal-downing handover plus cleanup for all attachment modes.
   Preserve existing weapon/canopy/grapple cleanup and downed tests.
4. Later: powered airborne/stunt profiles, collision reactions, settling and
   optional get-up blending. Fatal death need not recover automatically.

Activation is idempotent. Atomically cancel seat controls, exit/wire/stunt
ownership, gun shooting/reload/throw actions, grapple/reel/canopy controllers,
and hero-owned tether joints. Decide drop/stow/hide behavior for carried weapons;
do not leave weapon physics fighting a hand attachment. Death on a moving
vehicle must retain momentum while removing any transform parenting.

The old hero capsule/controller must cease applying movement and duplicating
collisions. Keep a logical hero/camera anchor that follows a selected torso/hips
pose without translating the world-space limbs. Choose whether enemies can hit
ragdoll bodies and how ownership routes damage; collider filtering should not
accidentally disable world collisions. A wire/tether linking external objects
needs explicit policy: preserve unrelated object-object links, clear hero-owned
links, and never leave a constraint referring to a deleted hero limb.

Respawn/load/scene removal despawns every owned joint/body/debug entity, clears
mapping and event cursors, restores animator ownership, capsule/controller and
equipment exactly once, and places the model consistently at the new root.
If later get-up exists, blend simulated pose into an appropriate get-up clip;
re-enable the capsule only after a clear placement check. Do not solve recovery
by teleporting a still-constrained assembly.

## Acceptance criteria before calling it a ragdoll

- Visible independent hip/spine/head and bilateral arm/leg movement; no rigid
  toppled Idle substitute, exploded anchors, collapsed mesh or T-pose snap.
- Rest mapping round trip; posed activation at rotation and world offset;
  joint pivot drift/limit tolerances logged over fixed physics steps.
- Flat ground, stairs, slopes, wall impact and high-speed fall remain finite and
  settle; all body/constraint references remain valid.
- Down from standing, running, seated moving vehicle, wire ride, rooftop stunt,
  grapple reel/hang, canopy and water. No residual locomotion/seat/weapon input.
- Moving-support angular velocity inheritance, no duplicate movement impulse,
  no controller capsule collision against the ragdoll itself.
- Two activations in one update, repeat death/respawn cycles, missing assets,
  deferred entity removal and world unload leave zero owned bodies/joints.
- Existing downed cleanup regressions pass. Explicit tests run physics schedule
  and constraint creation/removal; merely querying components cannot detect
  stale Avian graph edges.

No runtime acceptance was performed here. The helper ran against both original
BRDs and the documentation passed `git diff --check`; implementation builds and
smoke tests must follow the shared lock/private executable protocol.

## Original activation/recovery consumer follow-up — JC2-007

Research-only follow-up, 2026-10-08. Relay JC2-007 was atomically claimed by
`larger_missions`; only this section was added. Sources are the local retail
executable, `ragdoll_params.bin`, and `research/analysis/pseudo/_unnamed/`.
No builds, game launches, runtime edits or raw-asset commits. **Verified** below
means directly visible in those sources; no gameplay capture validated it.

### Profile modes and original ownership

The passive fatal milestone above remains a proposed bounded implementation.
It is **not** the original Dead profile: the shipped file explicitly uses
`POWERED_CONSTRAINT` for Dead. Fatal behavior must not be declared retail parity
merely because passive joints settle.

| Authored profile | Actual mode | Authored file selector |
| --- | --- | --- |
| Dead, PosematchingFlightDeath | POWERED_CONSTRAINT | ragdoll_file_0 |
| AirborneRagdoll, PosematchingFlight, PosematchingFlightProtect | POWERED_CONSTRAINT | ragdoll_file_0 |
| UncontrolledSkydive | RIGID_BODY | ragdoll_file_1 |
| ReeledStuntHang, HangOnVehicleKeyframedTransition, HangOnVehicle | RIGID_BODY | ragdoll_file_1 |
| HangOnVehicleNPC | RIGID_BODY | ragdoll_file_0 |
| HookedHang | POWERED_CONSTRAINT | ragdoll_file_0 |

These entries are verified in `jc2tool prop . ragdoll_params.bin`. Matching names
to every gameplay activation is **unknown**; profile names are not trigger proof.
`FUN_004a61d0` (size 0xc1f, `text_004a0000.c`) consumes the parameter file.
The value of global 0x00f247bc is #4cda2671. Its parsed mode-string hash selects
1 for RIGID_BODY (#d62ec1c7, also the missing-mode default), 3 for
POWERED_CONSTRAINT (#cd8331b0), and two other unresolved modes (2/0).

**Verified parameter application:** 004b3180 (size 0x2dd, `text_004b0000.c`)
copies the profile's first five floats to an optional controller at instance
+0x20, the next twelve floats to the control-data block behind +0x30, and the
mode dword to instance +0x70. Three profile bytes populate +0x74/+0x75/+0xe0.
Their loader keys are #925987bf/#dad4054a/#ac1fd636 respectively. The first gates
the update's animation-driven control path. Profile float +0x54 updates instance
+0xc0 unless negative, in which case it preserves the old value. Its exact key
name and blend-direction meaning remain unresolved; do not label +0xc0 a general
“ragdoll weight” without tracing the output blend.

Mode 1 invokes 004c2eb0, marking bodies +0x2a = 4 and constraint priority 2.
Other modes invoke 004b9bb0, marking bodies +0x2a = 3 and priority 1. These are
verified writes; their filter/quality enum names were not recovered here.
The same application routine builds a bone-index exception list and switches
selected bodies to motion type 6; unselected type-6 bodies return to type 1.
This establishes per-bone ownership exceptions, not an all-limbs identical mode.

**Verified update consumer:** CRagdollInstanceImpl's primary vtable
0x00f255cc +4 points to 004b5790 (size 0x354). When instance +0x14 is enabled,
it checks world membership, updates pending filters, and, if +0x74 is true:

* Mode 1 calls 00b211d0: body controller consumes the animation target pose,
  timestep and root transform. Its deeper 00b23c20 computes corrections from
  pose errors and existing linear/angular velocities. Scalar gain names remain
  unresolved; this is not evidence of Avian position-motor equivalence.
* Mode 2 calls 00b213a0 then 00b21360; its authored name is unknown.
* Mode 3 calls 00b21460 to enable constraint motors and 00b21580 to write their
  targets from the animation pose. Both iterate constraint objects through the
  ragdoll's skeleton/body/constraint mapping and branch on data type 2 or 7.
  00b21460 sets the motor enable state and clears an angle target; 00b21580
  derives new target values from each joint's local frames and target transform.

The decoder/coding agent must provide exact frames and compatible motors before
mode 3 can be approximated meaningfully. Animating only the mesh over passive
bodies does not reproduce this consumer. The per-bone exception list then takes
animation pose targets through 00bebdc0 with inverse timestep, writing motion
velocity slots through vtable +0x40/+0x44. This is another source of animation
authority over selected physics bodies.

### Activation, velocity and impulse evidence limits

**Verified pose handover:** wrapper 004a7820 leads to 004ba310 (size 0x24a).
It stores the supplied root transform, removes the engine world-origin offset,
optionally maps an existing skeleton pose into hkaPose with 00b08ea0, transforms
the pose with 00b1fec0, then writes the global vector at 0x01403770 through each
mapped body's motion vtable +0x40/+0x44 and ensures motion type 1. The global
falls in a zero-initialized PE virtual tail and no direct assignments were found
in the export: interpreting it as the zero vector is **inferred**, because
indirect startup initialization was not excluded. This pose setup writes both
motion vectors at this point; any subsequent inheritance must be traced separately.
It does not initialize from a mandatory rest pose. Activation consumer
00858820 (size 0x250) obtains/creates the ragdoll, applies this pose, applies the
selected profile, filter group and world ownership. If the new profile selects a
different underlying ragdoll resource it calls its recreation slot; with the same
resource it updates the profile in place. 008754f0 (size 0x299) also transfers an
existing instance/pose through the same wrappers.

**Verified controller coverage:** original `CRagdollingController` vtable
0x00ec2b68 +0x18 = 006a6860 (size 0xb0). The registration block in
`text_00760000.c` associates this controller with S_DEAD, S_AIRBORNE_RAGDOLL,
S_HITREACT_UNCONTROLLED_FLIGHT, S_HITREACT_UNCONTROLLED_FLIGHT_POSEMATCHING,
S_HITREACT_GET_UP_BLENDIN, S_UNCONTROLLED_FLIGHT_PROTECT, S_HANGED,
S_EJECT_FROM_VEHICLE_ALIVE and S_EJECT_FROM_VEHICLE_DEAD, among others. This
proves those states share a controller, not which health/contact input selects
them or which parameter profile each state references. 006a6860 runs a recovery
timer/blocked check and separate player pose-adjustment logic; these paths are
not a single automatic “settled means get up” transition.

**Verified raw velocity access:** 004a03b0 maps a skeleton index to a body and
returns +0x1a0/+0x1a4/+0x1a8; 004b5af0 returns +0x1b0/+0x1b4/+0x1b8.
These slots participate in linear/angular controller corrections in 00b23c20.
**Unknown:** exact activation inherited-velocity source; moving-vehicle point
velocity and angular inheritance; fatal impact body/point, impulse magnitude and
clamping; whether pose-derived velocity runs before/after the impact impulse.
The existing recommended `v + omega cross offset` policy is physically motivated
implementation guidance, not a recovered retail rule. Do not promote it to
verified. Impulse application requires further xrefs from the character collision
and hit-react callers into the ragdoll wrappers.

### Settling and get-up are separate consumers

**Verified settling:** `CRagdollIsSettledCondition` vtable 0x00eb7670 +8 =
00726280 (size 0x18e, `text_00720000.c`). It obtains the ragdoll, loops over
mapped indices, gets each linear velocity via 004a03b0, and **sums speed magnitudes**.
It also reads another character-rule scalar via 00557050(0), whose consumer
returns CCharacterRules +0x18. If sum-speed >= condition +0xc or rule-scalar >=
condition +0x10, it resets a shared dwell accumulator at character extension
+0x488. Otherwise it accumulates update time. It returns true only after dwell
is **strictly greater** than condition +8, then resets that accumulator.
The parameter loader is 006114f0: +8 uses #9ad4c2fe, +0xc #9b88675a,
+0x10 #ab18c527. Rule-scalar meaning/units, authored numerical values for a
particular graph, and sharing between simultaneous condition evaluations remain
unknown. This is neither max-limb speed nor average speed, and no angular-speed
threshold is demonstrated in this condition.

Other verified condition consumers:

* CRagdollBlendingCondition (0x00eb75e0 +8 = 0081ca70) queries 004aa7a0:
  true exactly for instance +0xc0 in the open interval (0,1), optionally inverted.
* CRagdollingTimerCondition (0x00eb75c8 +8 = 00556c60) checks extension +0x484
  only when nonnegative, with a configured less-than or strict greater-than test.
* CBlockedGetUpCondition (0x00eb75f8 +8 = 0054de50) refreshes a cached clearance
  result through 006aed30 and reads +0x2a0, optionally inverted. Refresh helper
  00555970 calls a geometric query from character position up through a height
  derived from `1.8 - 2*radius`. Collision masks/return polarity still need trace.
* CRagdollVelocityCondition (0x00eb7490 +8 = 00677820) averages a result from
  0049bc60/004af110. That helper sums vector differences in a stored pose/control
  array, not the direct +0x1a0 velocity accessor. Treating the condition's name as
  proof of metres-per-second physical speed is unsafe.

**Verified get-up event:** 00821a90 (size 0x1ae) emits `ply.ragdoll.getup` when
the player branch sees predicate 005518e0 transition from true to false, using
extension byte +0x315 as its latch. 005518e0 compares a character-rule state ID
to two globals; those IDs have not been resolved to named states here. The event
is therefore a state-exit notification, not proven to initiate get-up or be the
settling condition's output. 00586b60 contains separate player pose-adjustment
branches at 0.8/0.6-second boundaries; their global state IDs and full blending
ownership remain unresolved. Fatal death must not recover just because the body
passes the settling test.

### Bounded powered-profile follow-up and checks

Proposed contract (not original ABI):

```rust
enum RagdollOwner { Animated, Powered { profile: ProfileId }, Passive, Recovering }
struct Profile {
    mode: RawMode, // verified RIGID_BODY=1, POWERED_CONSTRAINT=3
    controls: Vec<TypedProperty>,
    driven_bone_exceptions: Vec<BoneId>,
}
struct SettleCondition { dwell: f32, sum_speed_limit: f32, rule_limit: f32 }
fn update_settle(sum_speed: f32, rule: f32, dt: f32, p: &SettleCondition,
                 accumulated: &mut f32) -> bool {
    if sum_speed >= p.sum_speed_limit || rule >= p.rule_limit { *accumulated = 0.0; }
    else if *accumulated > p.dwell { *accumulated = 0.0; return true; }
    else { *accumulated += dt; }
    false
}
```

First powered milestone: one explicit opt-in AirborneRagdoll fixture, independent
of automatic gameplay activation. Decode the profile mode/unknown controls;
feed the propagated animation pose to selected joint targets in fixed physics
steps, with explicit bounded motor authority and per-bone exclusions. Retain
the passive fallback when a motor class/mapper is unsupported and report that
limitation. Render from the simulated pose after physics; animation owns targets,
physics owns driven world bodies, and recovery owns the later pose blend/capsule
handover. Do not introduce get-up solely from this fixture's settling result.

Meaningful acceptance checks for that later implementation (not run here):

* Current-pose activation with translated/rotated root and world-origin offset;
  no T-pose snap. Profile change with same/different BRD has explicit lifetime
  behavior and zero stale motor/constraint references.
* Animated target step on one supported joint: torque/force bound, finite body
  velocities and pivot errors, control weight 0 and 1, and timestep variation.
  Compare with motors disabled; visible mesh motion alone cannot pass this check.
* Partial driven-bone exceptions cannot receive both capsule/animation teleport
  and motor/body correction in the same step. Fatal and airborne fixtures retain
  different recovery authorization even when both settle.
* Sum-speed versus maximum/mean speed fixture; thresholds at exact equality;
  interrupted dwell resets; strict dwell boundary; missing ragdoll and unresolved
  rule scalar fail predictably. Test repeated evaluation of the shared dwell state.
* Blocked get-up under a low ceiling, beside a wall and on a moving support.
  Capsule reinstatement requires a clear placement; state-exit event fires once,
  not repeatedly while stationary or during fatal death.
* Moving-vehicle activation and localized impact are required future checks once
  inherited motion/impulse consumers are recovered; log body velocities before
  activation, after inheritance and after impact to detect double application.

Reproduction: use the private prebuilt
`_extract/particles-ik-research/jc2tool.exe prop . ragdoll_params.bin` and
`hash RIGID_BODY POWERED_CONSTRAINT`; inspect the named functions in the grouped
`text_004a0000.c`, `text_004b0000.c`, `text_00720000.c`, `text_00820000.c` and
`text_00b20000.c` exports. PE vtable/hash words above were cross-checked with
Python `struct` and section-based VA translation. Next required traces are the
state-to-profile lookup, collision/impulse callsites, activation velocity setter,
render output blend and clearance query polarity. This section supplies consumers
for powered/settled fixtures; it does not claim the complete original lifecycle.

## Passive fatal runtime milestone — JC2-023

`jc2-game::ragdoll` constructs the normal original BRD's selected 14-body,
13-joint tree for Rico's fatal downing. It selects the ragdoll-to-render mapper
by original skeleton indices and the render skeleton name, validates its
`aFromB` rest transforms against the live render skeleton, and retains the
body-to-bone offsets. Missing assets, unsupported chains/constraints or rest
skeleton mismatches retain the explicit rigid downed fallback.

Activation follows transform propagation. Fatal cleanup suspends animation and
model-controller writes before that propagation, preserving the current posed
bone hierarchy instead of sampling Idle or the reference pose. Each limb is an
unparented world entity initialized from those propagated bone transforms.
Before the following propagation, simulated Position/Rotation poses reconstruct
mapped local bones in parent order; unmapped descendants keep captured local
transforms, with unmapped roots compensated for the logical anchor's movement.
Inverse bind matrices remain unchanged. The logical hero follows the hips for
camera/world streaming without moving the bodies.

Authored inverse mass/inertia, center of mass and damping become explicit Avian
mass properties with automatic collider mass contributions off. Coordinates
remain the project's verified identity basis. Initial velocity is support COM
linear velocity plus angular velocity crossed with each limb COM's offset;
angular velocity is inherited too. WireSwing already stores world velocity at
Rico's capsule centre, including pivot motion; that velocity is retained at its
recorded position and angular inheritance applies only each limb's offset from
that position, avoiding duplicated support momentum. This physical policy is an implementation
approximation, not recovered retail activation behavior. No extra fatal hit
impulse is fabricated or multiplied across limbs.

Authored hinge/twist/cone axes and frames become revolute/spherical joints.
The passive milestone omits powered motors and friction atom behavior;
Avian's circular swing cone approximates the authored cone while the independent
plane envelope remains unsupported. Animated poses have not passed a constraint
projection, so paired anchors are reconciled at their current world midpoint on
activation while bases/limits retain their original frames. This avoids an
initial anchor snap but changes effective local anchors. This passive assembly
must not be described as Havok Dead-profile parity.

Collision membership bit 31 disables ragdoll self contacts while preserving
contacts with default world/vehicle colliders. The logical hero keeps its
capsule/body graph alive as a noncolliding, gravity-free kinematic anchor;
no generic Disabled component or collider reactivation is used. Respawn restores
previous collision layers, gravity and body type once. Owner records remove
joints before bodies after respawn and hero/world removal. Hero-linked completed
tethers and an unfinished hand-linked first shot are cleared; independent
completed object-to-object cables remain. Existing fatal cleanup still cancels
weapons, vehicle/stunt/wire, grapple and canopy controllers.

Validation is recorded in the Relay handoff. Original-descriptor tests cover
pose/rest mapping against actual retail `biped.bsk`, translated/rotated root and
model offset, angular point velocity, repeated activation, falling/settling and
repeated constraint graph cleanup. Powered profiles, get-up recovery, original
activation impulses, exact plane/cone limits and NPC ragdoll damage routing
remain separate milestones. Physical settling never authorizes fatal recovery.

Branch validation (isolation 38, plain Steam assets): 337 release workspace tests
passed (131 formats + 206 game), matching all 337 source test markers. Release
workspace/all-target clippy passed with warnings denied; workspace formatting and
patch whitespace checks passed. The 13-joint pivot test measured maximum drift
0.000176 m over three fall/respawn cycles and required independent limb motion.
The production fatal producer test covers seated support COM/angular inheritance,
wire world velocity without double application, repeated respawn cleanup and an
explicit missing-asset fallback. Legacy standing/grapple and actual jeep roof/seat
fatal cleanup regressions pass with the articulated rig enabled. These are
headless physics/ECS checks. Coordinator GPU smoke on integrated main30314eb used
the private release executable with `--world31,13 --radius0 --pose down`, camera
yaw100/pitch-20, midday and an18-second exit. The fresh1280x720 capture
`target/browser-stunts-acceptance10/down-full-viewport.png` shows the articulated body
resting on terrain and the corrected full-screen damage tint; the process exited0.
This accepts a bounded fatal-pose visual smoke, not stairs/slopes/wall impacts or
high-speed CCD coverage. The integrated372-test checkpoint additionally exercises
the producer with a contributing child mass and the solver's computed support COM.
