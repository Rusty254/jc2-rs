# World and vehicle research: next milestones

Research date: 8 October 2026. Scope: Panau City location streaming, wire-swing
body tilt, movement between stunt positions, and vehicle-to-engine-sound assignment.
No game files changed, no builds/tests/game launches performed. Evidence is current
source plus read-only queries using the existing `target/release/jc2tool.exe` against
the plain retail install. Tool output verifies asset structure; it does not establish
that the compiled tool matches every current source change. No applicable AGENTS.md
was found inside this repository during the file inventory.

## Verified: the city is outside the terrain-tile container naming scheme

`jc2tool list .` finds **72 files under `locations\metropolis\`**, including:

- `bridge.1.2`, `bridge.1.4`, `bridge.1.4b`, `bridge.2.3`, `bridge.3.4` location pairs.
- `city.district.a1.houses.nlz` (398606 compressed bytes) and `.flz` (176104).
- House/road districts, city gameplay and settlement containers.

`locations\world.bin` is already readable with `PropertyFile`; root children carry
`file` values such as `metropolis\city.district.b1.houses`, plus vector keys
`#2b5ef194` and `#7df7f859`. For that example:

```
#2b5ef194 = [-10848.1045, 2146.8567, 816.7408]
#7df7f859 = [-15211.349, -1393.2965, -3219.276]
```

Other examples include `metropolis\city.gameplay.b1`,
`metropolis\city.settlement.b`, `metropolis\bridge.3.4`, and `roads\roo_city`.
Some registry records have additional values: `city.gameplay.b1` has
`#1070a35f = -75`; `city.district.b2.roads` has `#1070a35f = -40`.
Do not discard these fields or treat the root child hashes as coordinates.

`jc2tool sarc . city.district.b1.houses.nlz` confirms ordinary location members:
`city.district.b1.houses.nl.blo` (233123 bytes), `.nl.cgd` (98070), `.pfi`
(281794), `.obc` (35044), and model collision assets. `placements --models` reads
absolute positions such as a water tower at (-13072.93, 217.25, -1247.33).
This is reusable location loading, not evidence of a new city-specific model format.

### Inferred and unknown

The two vectors are ordered componentwise like maximum/minimum bounds. Their large
vertical span and roughly kilometre-scale padding suggest streaming bounds rather
than tight geometry bounds. Exact margin, near/far activation rules, and the meaning
of `#1070a35f` remain **unknown**. Do not subtract padding or silently assign a guessed
semantic name. Compare registry values to placement/CGD bounds and trace the retail
loader before claiming parity. No trustworthy executable function address for this
registry loader was established in this research.

### Current code hooks and milestones

`crates/jc2-game/src/world.rs:75` defines `ExtraContainers`, currently keyed by one
terrain tile. `load_tile` (around 476) loads the ordinary WC container and the extras;
`load_container` (559) handles BLO/PFI and other location content. `stream_tiles`
(1056) loads one nearest missing tile per frame and unloads outside radius+1. Its
actual focus order is hero, occupied vehicle, camera (older prose says vehicle first).

1. Add a typed world-location registry reader in `jc2-formats`, retaining raw unknown
   fields. Produce a diagnostic inventory of registry entries, bounds and NLZ/FLZ
   existence. Use the verified b1 houses fixture above.
2. Add location streaming keyed by canonical container identity, independently of
   terrain ownership. A district spans many tiles: mapping it to every intersecting
   tile without deduplication would duplicate geometry, collision and gameplay spawns.
   Track loading, pending models, entity ownership, shared asset references and unload.
3. Reuse `load_container` placement logic for metropolis content, with distinct owner
   records. Measure resolved models/CGD/PFI counts, then verify city buildings, roads
   and bridge traversal visually when game execution is authorized.
4. Define near/far and hysteresis policy explicitly as an approximation until loader
   semantics are confirmed. Registry bounds must not be confused with mission gates.

Changing `ExtraContainers` for an already loaded tile currently does nothing until
reload: `stream_tiles` copies extras only during `load_tile`. Runtime registration
needs explicit loaded-location handling. `load_container` finds the first BLO/PFI;
confirm multi-member containers before generalizing. Audit `scenery.rs` CGD ownership
as well: its static geometry path (`scenery.rs:409`, container selected at 419)
opens only `tile_container_name(x, z)`. Extra-container CGD is therefore omitted by
that existing path, even when its BLO/PFI is loaded. Refactor the CGD placement helper
to accept each actual location container and its ownership record.

## Verified: pendulum motion exists; swing body tilt does not

`crates/jc2-game/src/stunts.rs:165` defines `WireSwing`; `swing_step` (176) applies
gravity, relative damping (`WIRE_DAMPING = 0.5`, explicitly assumed), and projects the
position onto the fixed-length sphere around a moving hook. `follow_vehicle` (473)
uses the hook's linear plus angular point velocity and stores resulting world position
and velocity. Existing tests at `stunts/tests.rs:967` cover bounded swing and decay;
they were inspected, not rerun. STATUS's older “no swinging” statement is stale.

`character.rs:1462` rotates the HeroModel toward yaw; its only extra X tilt is 0.35
radians under the parachute. A wire swing still renders an upright hanging body.
The animation remains `Clip::Hang`, with `grpl_hang.ban` root-yaw correction documented
in `rig.rs:290`.

Milestone: expose a visual orientation derived from hook-minus-swing position, compose
it with facing and the hanging clip's correction, and smooth by elapsed time. Keep
physics position/velocity and the cable's hand endpoint authoritative. Decide whether
to rotate the whole model or selected rig bones only after inspecting the hang pose;
changing body orientation changes its hand position and aim frame. Validate vertical
rest, acceleration/turning, release, death, vehicle loss and camera aiming. A body-up
alignment to the wire is a reasonable **implementation inference**, not a recovered
retail formula. Retail damping/tilt limits and collision response remain unknown.

## Baseline before JC2-011: stunt actions lost by typed parsing

Before JC2-011, `jc2-formats/src/vehicle.rs:210` stores a stunt's name, kind, world transform, zone,
ground/parachute triggers. `stunt_position` (659) recognizes roof/belly/cling
controllers but does not preserve their sibling action nodes. `StuntSpot` in the
runtime strips this further to position/kind and the ground trigger. `OnStunt`
stores a local point, but no source spot identity or active transfer action.
`mount_input` (379) mounts, detaches or enters a seat; it has no spot-to-spot transfer.

Read-only retail jeep doll evidence:

```
CSmartObject name = SO_Hardtop_RD
CStuntAction name = RoofToFrontClingAction
#37757a6d = "jump_to_front"
#3eff86a9 = "vhl.frontcling.enter"
#5d655631 = "S_PERFORMING_STUNT"
#d0003e19 = "S_CLING_STUNTPOSITION_IDLE"
#d808bfc1 = ""  (one action), "CAB" (another action)
priority = 1
enable_event = ""; disable_event = ""
```

The action also has transforms, a UUID, numeric flags and several unknown string
fields. Do not infer a destination solely from child order or nearest spatial point.
Other sibling classes include `CEnterDriverSeatFromRoofAction` (animation-like value
`roof_to_driver`), grappling jump and parachute actions. The parser's `StuntKind`
only recognizing helicopter-like cling controllers does not prove every vehicle
front-cling destination has already been found.

1. Preserve raw action classes, identifiers, transforms, hashes, state/event strings
   in a typed action inventory. Compare jeep hardtop/hood, bus and helicopter dolls.
2. Resolve destinations from evidence (controller zone/event/state associations and
   identifiers). Record unresolved links diagnostically rather than inventing edges.
3. Add current spot identity plus a transfer state carrying source/destination and
   progress. Keep the hero stowed, follow the moving vehicle frame, and inherit its
   point velocity on cancellation/release. Reserve controls after auditing existing
   mount/seat/hijack bindings.
4. Drive known transfer animations through the animation-state work. A simple local
   blend can be labeled an approximation; asset presence alone does not establish
   its duration, root motion or collision policy.

## Verified: sound presets resolve; vehicle assignment is heuristic

`jc2-formats/src/soundmap.rs:625` preserves each `VehiclePreset.key`, name and sound
roles. The vehicle module parser (683) joins `presets`, `primary_sound` and secondary
roles by that key. `audio.rs:436` resolves key_on/sustain/key_off; `engine_preset`
(1981) chooses presets using container-name substrings. Example choices are
v014_jeep -> lave_engine/hummer; v047 -> SmJeep; v024 -> Corvette; fighter ->
arve_engine/jet_plane_afterburner. These choices are **inferred**, not retail vehicle
assignments. Existing `engines_by_id` tests prove the selected presets exist and
resolve, not that the assignments match the original game.

`research/formats/audio.md` documents 22 land, 6 sea and 8 air engine presets in
`sound\settings_smod_sfx_vehicle.bin`; the event-pair chain is known. It also records
an earlier unsuccessful search for preset-key strings/hashes in dolls, EPE,
vehicletypes and the executable. That negative result is prior research, not an
exhaustive proof. The sampled retail `v014_jeep.sst` contains two CCollectionProp
references (`V014-Hardtop_ref`, `V014-CrashRig_ref`) and no obvious sound assignment.

Milestones: inventory raw keys and every vehicle's candidate sound-related properties;
trace the retail vehicle audio component's preset selection using raw keys, retaining
provenance for recovered assignments. No executable address was verified here.
Introduce a data-backed mapping only for confirmed links; keep the current heuristic
as an explicit fallback. Compare all land/air/sea/DLC assignments, duplicate-name
presets (the parser currently returns the first case-insensitive match), and RPM/load
curves. Reuse the existing soundmap/event player rather than replace decoded events.
Also audit switching between identical vehicle models: `engines` observes names,
while vehicle door effects already observe occupied entity identity.

## Evidence commands

All commands ran from the plain game directory and only read archives:

```
jc2tool list .
jc2tool prop . locations\world.bin
jc2tool sarc . city.district.b1.houses.nlz
jc2tool placements . city.district.b1.houses.nlz --models
jc2tool sarc . lave.v014_jeep.ee
jc2tool prop . lave.v014_jeep.ee/v014_jeep.mvdoll
jc2tool prop . lave.v014_jeep.ee/v014_jeep.sst
```

Source line numbers refer to this research snapshot and can shift as other work lands.
No web sources were needed: findings are anchored in local primary retail assets and
the repository implementation. All proposed runtime validation remains outstanding.


## JC2-011 authored transfer/runtime handoff (2026-10-08)

The original jeep provides exact smart-object joins: hardtop object 365841728 and
hood 238058710 each have `CStuntAction`/`jump_to_front` with #63f9ea61 = -1607847811,
the front object's `_object_id`. That front object carries `CClingStuntController`,
previously omitted by the decoder. Its `CClingToRoofAction`/`cling_to_roof` actions
`FrontToHood` and `FrontToRoof` point back to 238058710 and 365841728. These unique
ID joins, compatible observed controller/clip associations, and empty enabling
events are the supported association evidence; no native consumer trace is claimed.

`StuntPosition` retains the entire original smart-object node and its object ID.
Each action retains its child key and complete original node: UUIDs, priority,
world/local transforms, animation identifiers, enter/exit events, transitional and
resulting state strings, and unknown flags. Nested trigger actions are included.
Real bus `v046_busscoach.mvdoll` and attack helicopter `v060_attackheli.mvdoll`
inventories preserve driver, ground/parachute and reel actions, but runtime transfers
are limited to the observed jeep roof/front joins. Missing/zero/dangling/duplicate IDs
and conditional enabling events do not select a guessed spot. Source identity stays
at the authored current object until completion, independent of vector order.

`Z` chooses the first supported action by action name/key; `Shift`+`Z` chooses the
last (front-to-hood versus front-to-roof). This is a new binding, not a recovered
retail input. A 0.6-second smoothstep interpolation of the hero-centre endpoints is
an explicit approximation: no authored duration, root-motion playback, traversal
collision sweep, front-cling IK or original action-event dispatch is claimed. The
raw clip/state/event identifiers remain available for a subsequent evidence-backed
consumer. Each physics step evaluates the path in the moving support frame and adds
support translation, angular point velocity about the solver's ComputedCenterOfMass and path derivative. Cancellation uses
the current world velocity, including the derivative; death capture uses that
velocity at the hero centre and angular motion only for limb offsets. Stow/release
uses existing Avian collision-layer discipline; removing OnStunt also removes all
transient identity, transfer, swing and tilt components, including crew-seat paths.

Wire body-up follows hook-minus-hero-centre with fixed-step exponential quaternion
smoothing (10/s, approximate), then composes with facing and clip body-yaw in the
existing single model visual writer. Model translation rotates about the logical
capsule centre. Vertical rest is identity. There is no additional render-frame tilt
smoothing or separate interpolation of wire endpoints: the cable uses the final
propagated launcher GlobalTransform for the displayed frame. Cable segment local
and global transforms are published after propagation in that same frame to avoid
a one-frame geometric lag. Pending/active ragdoll markers exclude stunt follow,
transfer input and hang-wire generation, while model visuals retain their existing
ragdoll ownership guards. Release clears the correction immediately; body physics
and world pendulum velocity remain authoritative.

Isolation44 release workspace validation passed 364 tests (143 formats +221 game),
matching source markers. Four new regressions cover original
inventory/link preservation and rejection, translating/rotating Avian transfers and
cancel/support loss, vertical-rest/time-step smoothing/cleanup, and final launcher
segment geometry plus pending/active ragdoll model ownership. Existing original
jeep stunt counts now include front cling (three rather than two). No visual/GPU
or retail-parity acceptance is asserted by these headless tests. The existing actual
CombatPlugin/BRD/biped fatal producer regression now includes a contributing child
mass and checks that capture uses the aggregated solver COM; it passed along with
all existing grapple, stunt, hand/aim and passenger soak tests. Strict release
Clippy across all targets, formatting/diff checks and the release workspace build
passed. The exact executable was copied under the build lock to ignored
`_extract/stunt-transitions/jc2-stunt44.exe` for a separately serialized GPU smoke.


Contextual HUD follow-up: supported exact-ID destinations also populate the existing
player-facing `PromptList` (Z and SHIFT+Z), without enabling developer text. Active
transfers suppress those two options while retaining cancellation keys; downed or
pending/active ragdoll states suppress contextual prompts. The existing state-prompt
fixture now checks this behavior and the real front-cling fixture checks target
labels and wire exclusion. No prompt scheduling hooks changed. This small separate
follow-up is statically formatted/diff-checked here and awaits coordinator main10
combined test/lint/build validation; no additional isolation44 Cargo or GPU run.
## Coordinator integration acceptance, 8 October 2026

JC2-011 commits5838ceb andd853bb4 are integrated through main6d6fd47. All372
release tests (143 formats +229 game) passed against the installed retail data,
matching source markers. Strict all-target release Clippy, formatting and the
workspace release build passed after the contextual HUD follow-up.

The branch private45-second helicopter wire smoke exited0; its inspected capture
`_extract/stunt-transitions/wire-smoke.png` shows Rico and the cable at the final
left launcher, with the helicopter partially outside the frame. The main private
20-second jeep `--crewed --ride spot` capture
`target/browser-stunts-acceptance10/roof-context-prompts.png` exited0 and visibly
shows `Transfer to front cling Z` in the normal HUD withoutF3. These are bounded
visual checks; no interactive transfer or original animation/timing parity is
claimed. JC2-078 tracks the original vehicle action timing/root-motion/lifecycle
consumer separately from JC2-026's on-foot clock research.
