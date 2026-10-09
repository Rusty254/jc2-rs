# Animation implementation preparation — 8 October 2026

Research only: no runtime changes, builds, or game launches. Evidence is the
unmodded Steam data read with a private copy of `target/jc2tool-research.exe`,
and the current `animation.rs`, `rig.rs`, `poses.rs` and their tests. This is a
coding brief, not a claim that the original evaluator has been reproduced.

## Main finding: containers are already decoded

`PropertyFile::parse` reads the AFSM, ASB, ACSB and compiled event-track files.
The next work is typed interpretation, reference resolution and evaluation;
do not write another binary container decoder. Keep unknown hashes and values
in the typed model, including repeated keys and ordered lists.

All property hashes below are hexadecimal. Names are case-sensitive. For
example `hash name` gives `d31ab684`, while `hash Name` gives `3a4ca003`.
AFSM/ASB and ACSB use different spellings and structures.

## AFSM evidence and minimum typed schema

Confirmed by hashing exact strings with `jc2tool hash`:

| Key | Exact spelling | Observed role |
|---|---|---|
| bad34c50 | StateMachine | enclosing node |
| 3a4ca003 | Name | state, condition, transition or machine name |
| c69700e5 | InitialState | initial state string |
| 60950642 | States | state definitions |
| 49e4a9fe | Transitions | global definitions or per-state references |
| 4a7ffa82 | Conditions | global definitions or per-transition references |
| 501691b0 | Class | original implementation class string |

Other keys with observed roles, **not confirmed spellings**:
`186fe1a8` carries action strings, `45f9d071` carries target-state strings,
`cc74a009` carries a date/version integer, `60ddfab2` encloses an additional
transition collection. The last collection in the base machine references
death/hit-reaction transitions; treating it as global transitions is a useful
hypothesis, not proven execution semantics.

`basestatemachine.afsm`: 272 state definitions, 388 transition definitions,
85 condition definitions. Conditions have 33 distinct class strings, including
speed/height/contact, raycasts, input, attachment, layer state, animation time,
ragdoll and random chance. A full evaluator therefore needs gameplay inputs and
other layers, not only clip time. Preserve unknown state flags instead of
guessing boolean meanings from integer 0/1 fields.

Start with `parachute.afsm`: five states (`PCS_CLOSED`, `PCS_OPEN`, two opening
directions, `PCS_CLOSING`), six transitions, one condition definition
`CParachuteAnimationFinishCondition`. Closing can be interrupted by opening;
opening can be interrupted by closing or immediate close. `PCA_UPDATE`
transitions wait for completion, and `PCS_OPEN` uses `CParachuteBlendState`.
This fixture is small enough to test every edge before touching the base graph.

Proposed types: `Machine { initial, states, transitions, conditions, extra }`,
`State { name, class, transition_refs, extra }`, `Transition { name,
action, target, condition_refs, extra }`, `Condition { name, class, extra }`.
Reference identifiers are strings inside arbitrary hashed list entries: do
not confuse each entry's hash with the referenced object's name hash.

The file's hash enumeration is not transition priority. The native follow-up
below confirms numbered `Transition0..63` precedence, AND/OR and polarity,
local-before-global arbitration and one state change per action dispatch.
Retain source ordering for diagnostics, then compile numbered references in
ordinal order. Unsupported condition classes should report an explicit
unsupported result rather than silently become true.

## ASB evidence

`795a87b3` hashes exactly to `AnimationSet`. `kc001_base.asb` has these groups:

| Group hash | Entry count | Observed contents / tentative role |
|---|---:|---|
| 28fa9c38 | 490 | named animation descriptors and clip lists |
| be52aa3d | 301 | state-string to animation-string links |
| 1c2223b1 | 91 | pairs of animation names and a float; likely blend overrides |
| 2db28149 | 38 | modifier names and pairs of replacement animation names |
| 801bf48e | 38 | animation-name / integer pairs; role unknown |
| b54a90e8 | 21 | pairs of animation names and an integer; role unknown |
| f46f42d4 | 4 | idle variants plus a float and named choices |
| f3b20ae5 | 0 | empty in this asset |

Descriptor keys: `0703fc2d` contains the animation identifier; `c516d565`
contains `.ban` paths; `4d026d5b` contains event-track paths; `22d6df42`
contains another string (empty in examples); `57ab7197`, `96270a00`,
`b7a9d0b3` carry floating values; `c29bffc6`, `fdcf4c12` carry integers;
`371aba7e` commonly contains -1. These spellings/roles need consumer proof.
`b7a9d0b3` is confirmed by hashing the exact spelling `playback_speed` and is
1 or 1.5 in sampled descriptors;
`57ab7197` is 0–0.7, suggestive of blend duration. Do not make these semantic
guesses global defaults without further evidence.

Examples useful for tests: `ANIM_UPRIGHT_IDLE` has four `std_idle_var*.ban`
paths; `ANIM_PARACHUTE` has speed/turn parameter clips; vehicle entry links
`hjk_getin_lft/rgt.ban` to the same event track. `additivelayer.asb` lists 17
clips under one `RA_ANIM_MISC_ADDITIVE` descriptor, plus an integer association
group `89806451` and `RA_S_IDLE` state link. Preserve the separate list key
spaces rather than zipping two hash-sorted lists and inventing associations.

## Event tracks: immediately useful independent coding task

**Source references end `.event_track`, compiled SARC members end
`.event_trackb`.** The `animations.blz` inventory contains 150 compiled event
tracks. Directly reading the source suffix fails. A resolver should first
respect an actual source/override file, then explicitly try the compiled suffix
through `GameFs::read_any`; do not rename game assets.

Schema: root `Events` (`49f74dfd`); each child contains `time` (`420ff89e`,
float), event string `cb0cc3f4`, argument string `065b3289`. Event/argument
key spellings remain unknown. Child enumeration is hash order, **not time
order**. Decode then stable-sort by `(time, original ordinal)`.

| Compiled track | Observed time/value pairs |
|---|---|
| upright_basic.event_trackb | .05 footstep; .47 fx_right_footstep; .65 footstep; 1.08 fx_left_footstep |
| jump_land.event_trackb | 0 jump_land; 0 character_fall_small |
| hjk_getin.event_trackb | 0 open_door; .44 foottouch; .66 close_door; .9 footstep_car |
| ra_reload_event.event_trackb | .29 reload_rh |
| close_parachute.event_trackb | .16 parachute_release; .5 hide_parachute |

The native consumer below compares these raw floats directly with clip time
and `time + playback_rate * dt`, without duration normalization. Seconds are
consistent with the decoded Havok clip clock; exact caller `dt` units still need
the update path traced. Simultaneous events must both survive. An event name
does not establish its gameplay side effect:
`reload_rh` may be sound/hand prop timing, and is not evidence for refilling ammo.

For a native-compatible interval helper use `[low, high)`, not `(old, new]`:
zero-time events fire on the first positive advance, and an event exactly at the
upper endpoint waits for the next scan. A zero-length interval fires nothing.
The original driver scans stored event order; stable time sorting is a proposed
jc2-rs chronological policy, not recovered native order. Preserve original ordinal.
Its forward loop path emits the wrapped head before the tail and does not
iterate arbitrary wrap counts; see the bounded native trace below.

A robust jc2-rs cursor may deliberately emit chronological intervals, handle
multiple wraps, stop one-shots and discard interrupted clips. Those are explicit
design choices until native clip removal/clamping is traced. An event at exact
duration is not guaranteed to fire when playback clamps there, because the upper
bound is exclusive. Held parameter sampling must be separate from advancement;
time-zero on immediate entry would also be a policy change. Choose a gameplay
event owner during crossfades; native evidence scans all paired active entries
without a weight gate in this consumer. Test equal-time events, exact boundary,
zero rate, reverse playback, interruption, crossfade ownership and deterministic
random variants before wiring consumers.
Vehicle audio currently uses immediate seat transitions (orchestrator report).
The vehicle-entry event timeline above is a later refinement: emit door/contact
sounds from the owner clip's crossings, and explicitly suppress the old immediate
sound path after integration to avoid playing each sound twice. Preserve unknown
payloads/classes and record raw timestamps in diagnostics.

## ACSB and constraints

`animations.blz` contains 101 `.acsb` members. They are generic properties with
named constraints, not skeletal clips. Small first fixture:
`left_hand_ik_100.acsb` has `class = constraint_relation`, `name = LeftHandIK`,
a `84205939` child with keys at `time=0` and `time=1000`, each value 100.
Do not infer that the timeline is seconds or that 1000 is a real clip duration.
It could express a normalized/authoring range.

`rpg_aim.acsb` has `constraint_aim` entries for Spine, Spine1, Head, Neck and both
shoulders. Key `032d50b5` names the driven bone, `56e913ab` is `Reference`,
`59ec04ac` and `7448f80b` are axis-like vectors, `d18a1e0b` is an Euler-like
triple. Constraint weight at time zero is 60 for Spine, 75 for Neck, 100 for
the other sampled entries. Each has a named Target node of weight 100.
Interpretation of rotation axes, Euler order, units, target frame and weight
normalization must be established; simply replacing current aim-chain shares
with these percentages would sum to far more than 100 and be incorrect.

## Additive transforms: evidence and experiments

Current `rig::overlay` performs replacement or interpolation towards sampled
local transforms; it has no additive operation. `add_breathe.ban` decodes as
9 tracks / 76 frames / 2.5 s. At time zero all sampled translations are zero,
and rotations are small deviations from identity. `rico_add_idle_left_leg.ban`
has 6 tracks / 46 frames / 1.5 s, also all zero translations at time zero.
These differ from ordinary clips' reference-pose local translations. Treating
them as absolute poses would collapse the affected hierarchy.

This supports an already-delta interpretation, but does not prove multiply
order or scale semantics. Before integration, sample start/middle/end for the
17 clips and inspect scale defaults. Compare three explicit models: already
local delta; delta relative to first frame; delta relative to bind pose.
For a proven local delta, translation adds `w * delta`, rotation composes a
normalized identity-to-delta interpolation with the base, and scale follows
the proven multiplicative/additive convention. Decide `base * delta` versus
`delta * base` with a rotated limb fixture and original side-view references.
Missing tracks contribute identity delta, not a bind transform. Weight zero
must preserve the base exactly; never subtract a bind pose twice.

Keep carrying/aim/recoil replacement layers separate from true additive layers;
a filename containing `add` is not enough to choose composition mode. Canopy
`chute_add_idle` is interleaved and targets `parachute.bsk`, not Rico's biped.

## Clip coverage, root motion and contact

`LoadedClip` currently stores only `SplineAnimation`; the formats crate already
supports interleaved clips. An enum with shared duration/track_names/sample
methods is a bounded coverage task. Preserve name mapping; the binding indices
target a different export skeleton. Inventory unsupported quaternion formats
and float tracks before adding speculative decoders. Current annotation parsing
reads track names, not the annotation event payloads; external `.event_trackb`
data should not be mistaken for those Havok annotations.

Root motion samples already decode `[x,y,z,yaw]`, but current speed uses only
endpoint horizontal displacement divided by clip duration. That misses curved
paths and turn-in-place motion. Start with an inspection/sampling API and
per-step delta including yaw and loop-boundary accumulation. Verify coordinate
conversion, extracted duration and trajectory before connecting to physics.
Keep existing locomotion physics authoritative until a bounded vehicle entry,
climb or vault owns displacement; applying both controller travel and extracted
translation would double movement. Test collision interruption and exact final
alignment to the moving vehicle/ledge.

Foot contact can begin with phase/contact diagnostics and the decoded event
times. Footstep FX timings are useful phase evidence, not guaranteed foot-lock
windows. Original RTTI includes `NAnimationSystem::CIKFootPlacementImpl`,
`CFootPlacementRaycaster`, `CFootPlacementRaycastFilter` and
`hkaFootPlacementIkSolver`. Those names establish machinery exists, not its
parameters. A practical later milestone is two-bone leg IK, ground raycasts,
bounded pelvis correction and cached planted world contacts, faded out during
jumps/swim/climb. Validate slope, stairs, uneven two-foot ground, moving supports,
teleport/reset and unreachable targets. Recover original weights before claiming
original foot-placement behavior.

## Delegation sequence and acceptance gates

1. Formats agent: typed AFSM/ASB models plus diagnostics; raw hashes and unknown
   values preserved; parse all selected sets and report unresolved references.
2. Separate formats/gameplay agent: compiled event-track resolver, parser and pure
   scheduler; initially diagnostics only, then bounded FX/door/canopy consumers.
3. Rig agent: polymorphic spline/interleaved sampling and explicit additive mode
   with numerical/reference experiments; preserve current mask and fade tests.
4. Evaluator agent: parachute graph first, then selected on-foot conditions and
   ASB clip resolution, with documented unsupported classes and replayable traces.
5. Constraint/root-motion agent: ACSB schema and sampling diagnostics; afterwards
   specific IK/contact or vehicle-entry ownership. Avoid editing `rig.rs` while
   the rig agent owns it; prepare modules and integration patch separately.

Fixtures: plain-install `biped.bsk`, `parachute.bsk`, std_idle/run_fwd/fall/
grpl_hang, chute_add_idle, add_breathe, rico_add_idle_left_leg, parachute.afsm,
base/upper-body/left-arm ASBs and AFSMs, additivelayer.asb, left_hand_ik_100,
rpg_aim and the five event tracks above. Construct tiny synthetic property
fixtures for malformed/unknown/missing-reference cases; keep proprietary asset
bytes and decoded bulk dumps out of commits. Asset-backed tests must show when
they skip without `JC2_TEST_DIR`.

Visual acceptance uses current pose overrides plus genuine transition scenarios:
idle→run with carrying/aim, shot/reload interrupt, grenade throw replaced by aim,
open→close→reopen canopy, contact on slope and vehicle entry on moving support.
Static override screenshots cannot prove event timing or physics prerequisites.
Existing rig/pose tests protect mask replacement, crossfade continuity, aiming
without moving legs, grip/off-hand placement and first-shot muzzle parenting.
No validation was run in this research task; implementation agents use the
shared build lock and their assigned unique isolation numbers.

## Original transition, condition and motion consumers (JC2-008)

Research-only follow-up, 8 October 2026. Evidence is the retail `JustCause2.exe`
SHA-256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`,
its existing Ghidra exports in `research/analysis/pseudo/_unnamed/`, RTTI in
`research/analysis/classes.csv`, and plain-install `parachute.afsm` /
`basestatemachine.afsm`. Addresses below are image VAs, not file offsets.
Decompiler calls frequently omit implicit ECX/this arguments; shared tiny
functions do not establish the type of every caller. No runtime execution or
Cargo validation was performed. This section adds consumer semantics, not a
replacement graph inventory.

### Confirmed arbitration and condition semantics

| Consumer VA | Verified behavior / useful field |
| --- | --- |
| `004b5cb0` | Builds hashes of `Transition0` through `Transition63`, in ascending ordinal, into `013e0f78`; these names are the precedence keys. |
| `00494c40` | Reads a state's transition-reference children using that ordinal table, capped at 64; appends deferred references in that order. Sparse ordinals are searched until enough references were found or the 64-slot bound is reached. |
| `004b47d0` | Resolves deferred references in insertion order; appends state-local transitions via state virtual `+0x0c`, or global transitions into machine `+0x40..+0x44` when the owner state is null. Missing references invoke the loader's diagnostics. |
| `004cb9f0` | Scans runtime vector from begin to end, matches `transition+0x34` against the supplied action ID, calls condition evaluator, returns immediately on first success. Increments instance scan/match/success counters at `+0x18/+0x1c/+0x20`. |
| `004bd5a0` | State virtual `+0x2c` scans its local vector, returns winner's target `transition+0x0c`, otherwise retains instance current state `+0x0c`. |
| `004bfb50` | Dispatches local state handler first. Only if success counter did not change does it scan the machine's global vector. Calls exit (`state vtable+0x38`), assigns target, enter (`+0x30`), then observer (`instance+0x14`, virtual `+4`) when the state pointer changes. |
| `004bf550` | Evaluates condition vector `transition+0x14..+0x18` in order using condition virtual `+8`; compares each result with its expected Boolean bit in `+0x28..+0x2c`. Mode `+0x38 == 0` is AND, mode 1 is OR, both short-circuit. Empty AND succeeds; empty OR fails. |
| `004abcc0` | Loads target/action, sequential `Condition0..` references and expected bits (default true; optional `0x664c2c54` child overrides), plus combination key `0x894029cf`. Hash `AND=44b2377d` sets mode 0, `OR=8551491d` mode 1. Transition construction in `004b47d0` defaults mode to 0. |

Do **not** sort property enumeration hashes to derive precedence.
`Transition0=eeb862e0`, `Transition1=6b8aa8be`, `Transition2=ab643432`,
`Transition3=52745896`; their numeric hash ordering is different. The fixed
condition-key array starts at `00f243bc`: `Condition0=cff8084b`,
`Condition1=5d737ab3`, `Condition2=cf53bce0`. The optional expected-bit child is
Boolean polarity, not another trigger or numeric priority. Unknown combination
strings leave the constructor default; manually corrupted mode values other
than 0/1 fall through to success after the scan in this routine. A safe future
typed parser should preserve/report unsupported values rather than invent modes.

The same action call produces at most one state change in `004bfb50`; there is
no loop to reconsider the new state's transitions. A qualifying local transition
whose target is the current state still increments the success counter and
suppresses global candidates, but produces no exit/enter/observer callback.
`00878170` wraps dispatch, records old state at wrapper `+0x48`, sets changed flag
`+0x40`, and returns changed/not-changed. It does not reveal the ordering of
different actions submitted in one gameplay frame.

**Trigger limit:** action IDs are supplied to dispatch, not cleared from an
AFSM-owned trigger set by the generic evaluator. The scan/evaluation routines
contain no generic trigger reset. Conditions are virtual consumers and can have
side effects; purity, gameplay input-edge consumption, action queues and replay
policy remain unverified. Preserve sequential calls and short-circuit order;
do not repeatedly fire an action until no transition qualifies.

### Parachute and on-foot evaluation traces

Confirmed parachute trace, with finish supplied by the original condition:
`PCS_CLOSED + PCA_OPEN_VERTICAL -> PCS_OPENING_VERTICAL`; an unfinished
`PCA_UPDATE` stays there; a finished `PCA_UPDATE` takes ordinal 0
`PCT_OPENING_TO_OPEN -> PCS_OPEN`. While opening, `PCA_CLOSE` selects ordinal 1
and interrupts to `PCS_CLOSING`; `PCA_CLOSENOW` selects ordinal 2 and immediately
returns `PCS_CLOSED`. Closing has ordinal 0 finish-to-closed, ordinal 1 vertical
reopen, ordinal 2 horizontal reopen and ordinal 3 close-now. These are different
actions, so ordinal precedence does not decide simultaneous close/reopen input;
the caller's dispatch order does. Neither open nor close is implicitly performed
by the mere presence of a topology edge.

The parachute finish condition RTTI vtable `00eb73e8`, evaluator `007340c0`,
reads context `+0x10` (getter `00b1e670`), returns false for null, otherwise calls
that object's virtual `+0x40`. The precise concrete object/implementation of
that virtual remains unresolved; do not substitute the on-foot finish predicate
and claim canopy timing parity. `CAnimationState` vtable `00f2450c` enter/exit
are `009a4cf0` no-ops; `CParachuteBlendState` vtable `00eb7264` enter/exit use
`005145a0`, also a no-op wrapper. Clip start/crossfade therefore needs the driver
or observer consumer; the graph entry callback alone cannot animate the canopy.

On-foot comparison: `basestatemachine.afsm` has `S_JUMP_START_STANDING`
(`CJumpControllerState`) ordinal 0 `T_JUMP_FLIGHT_STANDING`, action `ACT_UPDATE`,
condition `C_WAIT_FOR_ANIM_TO_FINISH`, target `S_JUMP_FLIGHT_STANDING`.
That flight state's `ACT_UPDATE` landing candidates are ordinal 0
`T_JUMP_LAND_HEAVY` (ground contact AND heavy), ordinal 1 `T_JUMP_LAND_RUN`
(ground contact AND minimum input), ordinal 2 `T_JUMP_LAND` (ground contact).
Thus when all three pass, heavy wins; with heavy false and input true, running
landing wins; otherwise contact alone selects ordinary landing. This ordering
is native loader/evaluator evidence combined with actual asset references,
not a guessed preference from names. Remaining flight edges, global edges and
the native ground/heavy/input predicates still need separate consumer work.

### Finish, loops and interruption boundaries

On-foot `CAnimationFinishCondition` vtable `00eb72b0`, evaluator `00817d10`,
returns false for null context `+0x10`; returns true when that object's
`+0x2f8` pointer is null; otherwise calls `0087a230(0)`. This indexes the
animated body-part vector `+0xa0..+0xa4` and delegates to `008ef600`.
The latter checks the **first entry** in the selected body-part's
`+0x38..+0x3c` vector; an empty vector returns Boolean true. A null instance in
that first entry asserts; it does not search for the next non-null entry.
It does not wait for every additive/blended layer. The generic/base finish
condition at `00892630` uses a different object field (`+0x208`); preserve
condition-class distinctions.

`004ae580` gives a precise instance finish predicate. Let `mode=instance+0x18`,
`time=+0x28`, `margin=+8`, `rate=+0x60`. The duration expression is
the instance's animation pointer. This is a data read, not a virtual call;
concrete intermediate ownership remains subject to the decompiler's types.
Finish is:

```text
mode == 1 && ((rate >= 0 && time >= duration - margin)
           || (rate <= 0 && time <= margin))
```

Getter/setter `004a3670/004a5f30` reads/writes time; `004a26e0` writes rate;
duration getter `004c2dd0` follows the same animation pointer. `006289a0`
uses duration and time to maintain synchronized jump blend clips, supporting
these time/rate meanings. Inclusive comparisons and the rate-zero OR are verified;
NaN rate fails both comparisons. Mode values other than 1 never finish here.
**Inference:** mode 1 represents finite playback; the event driver separately
handles forward wrap only for mode 0. The remaining mode-name mapping, exact
clock advancement/wrap behavior, margin setter and
the handling of multiple wraps/large `dt` are still unknown; this predicate does
not prove a loop completion event or wrap count. Preserve raw mode diagnostics.

Interruption uses the same exit/store/enter sequence, with no generic fade,
completion requirement or root displacement in `004bfb50`. Direct state forcing
`004ae930` and reset-to-initial `004c7070` use equivalent callbacks when the
pointer differs; `004c1ec0` explicitly calls exit for teardown. Interruption fade
curves, retained clips and event cancellation belong to the driver/layer system
and are not established by this AFSM dispatch trace.

### Motion ownership evidence and limits

Original jump state vtable `00eb653c`: enter `006ae480` sets context object's
`+0x36c` byte, resets fields `+0x10c/+0x110/+0x40/+0x3c/+0x118` on its `+0xa34`
controller pointer. Exit `0076f9c0` clears that byte, sets controller `+0x114`
and adjusts `+0x108` using `004d3690(0) * 0.35`. Velocity-controlled state
vtable `00eb658c` enter `007f0a90` / exit `006e0080` sets/clears context
`+0x36d`. This proves state-dependent controller ownership switches exist;
it does **not** identify those bytes as root-motion enable flags.
`CRuleState` vtable `00eb64ec` uses no-op entry/exit; both derived states still
share the generic transition scanner. The four-clip jump consumer `006289a0`
also sets instance weights (`004aab10`, data `+0x5c`), rates and synchronized
times. Do not turn decoded root trajectories into unconditional world travel.

**Unknown:** the final original path from Havok extracted-motion samples to
character controller displacement/yaw, its coordinate convention, collision
correction, blend weighting and ownership during vehicle entry/climb. Havok
reference-frame construction (`00b16a10`-area export; search warning text
`Extracting turning motion`) establishes extraction machinery, not a gameplay
consumer. The existing `animation.rs` decoded samples and endpoint speed remain
format facts. Until the consumer is traced, use a proposed explicit ownership
interface: `ControllerTravel | OwnedActionMotion`, with sampled motion diagnostic
only and no concurrent controller plus clip displacement. This is jc2-rs design,
not a recovered original enum. Target the observer around `00878170`, character
driver transition RTTI `00f0d954`, and callers reading controller `+0xa34` next.

### Native event clock, boundaries and ownership (JC2-020 follow-up)

Confirmed loader chain: `004b8280` appends `b` when the extension is
`event_track`, constructs `CTrackEvents`, opens the file and calls `00499330`.
That reader obtains the `Events` child and calls `004a3e50`; it reads raw time,
event selector and argument keys and appends a 0x20-byte record via `00498e30`.
Retail PE words `00f24340/44/48/4c` are respectively
`420ff89e/49f74dfd/cb0cc3f4/065b3289`, independently matching the decoded schema.
The loader does not establish a loose source-file override search policy.

`0049e820(low, high, context)` scans every stored record, testing
`low <= event.time && event.time < high`. Each matching event invokes its
parallel callback entry via `004e3790`, carrying raw timestamp, context and
argument string. Equal-time records are all visited in stored order; there is
no time sort or early break here. This is an interval dispatcher, not a cursor
which remembers previously delivered keys. Callback registration/side effects
remain separate research; no ammo refill or foot locking follows from an ID.

Character consumer `0090ba50(dt)` iterates body parts at `+0xa0..+0xa4`,
obtains their event-track list (`008fb620`, source member `+0x54`), and pairs
each non-null track with the same ordinal animation instance at `+0x38..+0x3c`.
Getter `004a3670` reads instance time `+0x28`, `004a55b0` reads mode `+0x18`,
`004a2b60` reads rate `+0x60`, and `004c2dd0` follows the duration data chain.
The observed call order is:

```text
old = clip_time; delta = playback_rate * dt; next = old + delta
if next > duration && mode == 0:
    dispatch(track, 0, next - duration, character_context)  # wrapped head first
if delta >= 0:
    dispatch(track, old, next, character_context)           # original tail
else:
    dispatch(track, next, old, character_context)           # ascending interval
```

Confirmed: raw event timestamps share the clip's clock; there is no division or
multiplication by duration to normalize event phase. **Inference:** that clock
is seconds, consistent with Havok duration and sampled `.ban` metadata. The
precise `dt` source/unit and relative clip-advance scheduling still need tracing;
`005653c0` calls `008bce40(dt)` before this dispatcher, while other callers also
insert `00872360(dt)`. The dispatcher itself does not advance clip time.

Derived boundary cases, with synthetic duration 1 and records at 0, .25, .75, 1:

| Call / input | Dispatcher result |
| --- | --- |
| interval `[0, .25)` | time 0 only; .25 deferred |
| interval `[.25, .75)` | .25 only; .75 deferred |
| interval `[.75, 1)` | .75 only; exact-duration event deferred |
| interval `[.25, .25)` / rate 0 | none; held sampling cannot repeatedly fire keys |
| reverse .75 to .25 | .25 included, .75 excluded; callbacks retain stored order |
| mode 0, old .75, delta .5 | head `[0, .25)` first (0), then tail `[.75, 1.25)` (.75 and 1) |
| mode 0, next exactly duration | no head scan; the duration event is still upper-exclusive |
| mode 0, old .75, delta 2.5 | only head `[0, 2.25)` plus tail `[.75, 3.25)`; no loop per wrap |

The last case can deliver .75 and 1 twice, but does not represent three correctly
ordered cycles. It demonstrates this driver's bounded single-wrap assumption;
do not claim arbitrary large-`dt` native parity. Reverse boundary wrapping has
no separate branch here. Native end-event behavior depends on later clamping,
removal or another advancement; a scan beyond duration can include the end,
whereas clamping exactly to it cannot. Both time-zero `jump_land.event_trackb`
records fit first-positive-advance delivery. No explicit entry-time dispatch is
established by this path; `0055f450` and `008459e0` can emit named animation
events directly, separately from track playback.

Ownership limit: this routine scans all paired active instance/track entries
without checking their blend weights. It contains no outgoing/incoming dominance
test, event cancellation generation or one-shot stop. Clip-list population and
removal must be traced before describing original crossfade/interruption policy.
For jc2-rs gameplay effects, explicitly assign an owner clip and suppress stale
generations; keep that safety policy labelled as design. Do not reuse a held
parameter sampler as an advancing event clock. The native stored order and
head-before-tail result also differ from a chronological multi-wrap cursor.

### Minimum evaluator milestone and golden cases

Compile ordered references using their numbered keys, conditions with polarity
and combination mode, then implement single-action dispatch with local/global
precedence and a separate `matched` result from `changed`. First replay the
parachute trace using an injected finish adapter; report that the adapter is a
test seam until its virtual consumer is recovered. Next replay only the standing
jump/landing subset with explicit unsupported-condition diagnostics. Store traces
as symbols/ordinals/action/result, not proprietary property bytes.

Golden acceptance cases: two passing candidates select ordinal 0; a failing
first candidate permits the next; nonmatching action skips evaluation; AND/OR
short-circuit and empty-list results; expected-false polarity; successful local
self-target suppresses global with zero callbacks; one dispatch does not chain;
exit/current-store/enter/observer order; finish just below/at/above both margins,
positive/negative/zero rate, mode 1 versus another mode, empty body part, missing
driver versus missing context; open-close-reopen in both caller orders. Keep
large-step loop motion, interruption event cancellation, and collision/alignment
fixtures as explicit pending acceptance until their native consumers are traced.

Reproduce without builds or saved dumps, from `jc2-rs`:

```powershell
$researchTool = Join-Path $env:TEMP 'jc2tool-material-water-research.exe'
& scripts/research-animation-consumers.ps1 -GameDir .. -ToolPath $researchTool
& $researchTool hash Transition0 Transition1 Transition2 Transition3 Condition0 AND OR
& $researchTool prop .. parachute.afsm --json
& $researchTool prop .. basestatemachine.afsm --json
rg -n 'FUN_004b5cb0|FUN_004b47d0|FUN_004bf550|FUN_004bfb50' research/analysis/pseudo/_unnamed/text_004b0000.c
rg -n 'FUN_00494c40' research/analysis/pseudo/_unnamed/text_00490000.c
rg -n 'FUN_004abcc0|FUN_004ae580' research/analysis/pseudo/_unnamed/text_004a0000.c
rg -n 'FUN_004cb9f0' research/analysis/pseudo/_unnamed/text_004c0000.c
rg -n -A 130 'FUN_0049e820' research/analysis/pseudo/_unnamed/text_00490000.c
rg -n -A 190 'FUN_0090ba50' research/analysis/pseudo/_unnamed/text_00900000.c
rg -n -A 70 'FUN_004a3e50' research/analysis/pseudo/_unnamed/text_004a0000.c
```

The helper accepts any existing private prebuilt `jc2tool` path. It checks the
retail executable fingerprint and ten PE words, computes reference hashes, and
prints only selected ordered graph references. It reads graphs in memory,
performs no builds/launches and writes no asset dumps. Matching these probes
does not execute the original evaluator; the native trace and table above are
derived from consumer reads, with injected condition truth values.

Validation performed: decoded both retail graphs in memory, compared numbered
reference hashes with native hash generation and resolution/scan order; read
retail PE vtable words to check callback/evaluator addresses against RTTI and
decompilation. Independent continuation audit corrected first-entry null behavior
and the direct duration data read, and ran the read-only helper successfully
(retail hash, ten PE words, seven reference hashes and both selected graphs).
Read loader/emitter/driver consumers for event boundaries and compared the
synthetic interval cases above. `git diff --check` passed. No Cargo tests,
implementation, gameplay parity or live timing claim.
