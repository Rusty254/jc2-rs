# Rico on foot: audit of the controller and parachute against the native graph (JC2-238)

11 October 2026. Runtime work plus research. Probe `research/tools/rico_controller_probe.py`
(exit 0; with `--export` it also checks the native evaluators), headless traces in
`crates/jc2-game/src/character.rs` (`character::tests::trace_*`) and graph tests in
`crates/jc2-game/src/foot_machine.rs`:

```
python research/tools/rico_controller_probe.py --tool <jc2tool.exe> --game <game folder> [--export <research/analysis/pseudo/_unnamed>]
cargo test --release -p jc2-game foot_machine character::tests::trace   # JC2_TEST_DIR = game folder
```

Executable fingerprint `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Labels:
**native** = read from the graph/data/code and asserted by the probe or a test; **inferred** = reasoned
from native facts; **non-native** = this project's own rule, kept and said so; **Gap** = unresolved.

## 1. What changed

Rico's main-layer pose is no longer a hand-written clip picker. `foot_machine.rs` reads
`basestatemachine.afsm` and `kc001_base.asb` from the game, evaluates the graph the way the native
evaluator does (numbered `Transition0..` precedence, local list before the global one, one change per
dispatch, **polarity**), and a `FootDriver` feeds it the physical events of the capsule (touch-down,
take-off, the chute going up or down, input). The state names the clip, blend time and playback speed
(`kc001_base.asb`). The ground locomotion states still pick their clip by speed and strafe (Gap 4).

New facts found on the way (all asserted by the probe):

* **Polarity is a child of the transition, not of its `Conditions` node** (`664c2c54`, one value per
  `Condition<N>` key; 0 = the condition must be false). The first reader (`afsm_probe`, JC2-106) looked
  under `Conditions` and saw none. Only three base transitions carry it: `T_JUMP_AT` (not in the wielding-up or
  wielding-down upper-body state), `T_DOWN_TO_CROUCH` (input magnitude not above 0.7) and
  `T_DEFAULT_STUNTPOSITION_IDLE_PLANE` (not attached to a helicopter). `afsm_probe.py` now reads it.
* **Condition evaluators** (vftable slot 2 of each class): the speed, vertical-speed and input-magnitude
  conditions compare **strictly**: flag 0 means "value < threshold", otherwise "threshold < value"
  (`007f2960`, `00725020`, `007a3230`). Horizontal speed is sqrt(x*x + z*z) of the character velocity;
  input magnitude is the length of the planar input vector at state-block `+0x8c/+0x90`;
  `CGroundContactCondition` is the contact mode test `0088b250`, `CInAirCondition` its negation;
  `CLayerIsInStateCondition` (`00583170`) is "the layer's current state equals the named state";
  `CAtSeaLevelCondition` (`0059a1e0`) is joint height minus the water height at the joint (joint index in
  the condition, 1 = Hips in the swim-depth conditions) against the offset.
* **The heavy landing is a fall timer, not a speed.** `CFallDamageCondition` (`007f84c0`) is true when the
  character float at `+0xa1c` is above **1.5**. Its writer `007a97f0` adds the frame time while the main
  layer is in `S_FALL` or `S_JUMP_TO_FALL_TRANSITION` and clears it on ground contact (any other state).
  The two state ids are the globals `011968b0` and `01197030`. Method for naming such globals (used by
  the probe): the static initialiser stores `push <string>; call hash; mov [global], eax`, so scanning the
  exe for the store and the preceding string push names the global (`S_FALL`, `S_JUMP_TO_FALL_TRANSITION`,
  `ACT_PULLCHORD 01196ed0`, `ACT_LAND 011967c8`, `ACT_JUMP_AT 01196648` ...).
* **Chute landing bands** (graph data): `S_PARACHUTE` on `ACT_LAND`, in order: nice landing (`land_chute`)
  if horizontal < 8.1 and vertical > -8.0; roll (`land_chute_roll`) if horizontal > 8.0; hard
  (`land_chute_hard`) if vertical < -8.1. The speeds are those at touch-down. The bands leave a dead gap
  between -8.0 and -8.1 m/s that no edge covers.
* **`S_FALL` and `S_FALL_TO_SKYDIVE` have no `ACT_PULLCHORD` edge**; `S_SKYDIVE` does. With
  `foot-controller-next.md` (the chord opens only if the state takes `ACT_PULLCHORD`, on the frame of the
  press), a press in the 0.7 s lead-in (`falling_to_freefall`) or in an ordinary fall is lost.
  `S_FALL` -> `S_FALL_TO_SKYDIVE` needs 28.5 m of open air below and 28.5 m above the sea
  (a 30 m ray from 1.5 m up; `C_CAN_FALL_TO_SKYDIVE`, `C_ENOUGH_ABOVE_WATER_TO_SKYDIVE`).
* **The player's jump is always `ACT_JUMP_AT`** (the only dispatcher, `00816700`, JC2-170); the standing
  jump clips (`std_still_jump_*`, `ACT_JUMP_UP`) have no caller in the exe. From any state the jump is
  `S_JUMP_START_SPRINT_FORWARD` (`run_jump_start`) -> `S_JUMP_TO_FALL_TRANSITION` (`run_jump_timeblend`
  at 1.2). The landing then splits: heavy (fall timer), running (input magnitude > 0.95), light.
* **The sprint is the `S_DASH` state** (explicit `ACT_DASH`, `dash_fwd` with lean and slope clips), not a
  speed threshold; key held and input magnitude or speed above 0.9 (`005ff2f0`, JC2-170).
* **`CParachuteState::update` (`00713d60`)**: the canopy pose is a time blend of four 1.0 s clips
  (`rico_speed_tb_up/dn`, `rico_turn_tb_up/dn`): each clip's time is its length times (axis + 1) / 2, the
  axes being the flight data block's `+8` (pull) for the speed pair and `+4` (turn) for the turn pair; the
  pair weights are `|axis8|` for the speed pair and `1 - |axis8|` for the turn pair, split inside a pair by
  two per-state numbers (`+0x3c`, `+0x40`; Gap 5).
* `std_idle_var0..4` are **2-frame static stances** (0.033 s) and `S_FALL` is `fall_base_pose` (2 frames)
  plus additive clips; the 5.3 s `std_idle.ban` the runtime used belongs to `S_UPRIGHT_IDLE_VARIED`.

## 2. Audit: wrong clips, transitions, speeds (before -> now)

| # | Item | Before | Native | Now |
|---|---|---|---|---|
| 1 | Jump clip | `run_jump_flight` (2.7 s, bound only to `S_PARACHUTE_TO_SO_START`) for every jump | `run_jump_start` then `run_jump_timeblend` (x1.2) | **fixed** (graph) |
| 2 | Landing from a fall | speed > 2 m/s: running land; fall speed > 11 m/s heavy; > 17 m/s moving: chute roll | heavy = fall timer > 1.5 s in `S_FALL`; running = input > 0.95; light otherwise; the roll is chute-only | **fixed**; the roll after a ground fall is gone |
| 3 | Chute landing | picked from the **fastest** speed since leaving the ground, hard first (a chute opened from free fall always landed "hard") | bands on the speed at touch-down, nice -> roll -> hard, strict | **fixed**, thresholds read from the graph |
| 4 | Chute pose time | clip time 0.5 + 0.4 x axis | length x (axis + 1) / 2 | **fixed** (turn axis same law) |
| 5 | Chute open | canopy and glide pose at once | pulled chord (`rico_open_chute`) -> unfold -> flight; canopy shown from the chord on | **fixed** (clips, order); the unfold replay is Gap 6 |
| 6 | Chute release | `Fall`/`Dive` by speed | `S_PARACHUTE` + `ACT_PULLCHORD`: `S_FALL` if something is within 30 m ahead, else `detatch_chute` then skydive (nothing within 60 m), else skydive via fall | **fixed** with one forward ray (fan geometry Gap 7) |
| 7 | Chute only from the skydive | any airborne frame with 3 m of room | needs `ACT_PULLCHORD` accepted (skydive; not the fall or its lead-in) | **fixed** (gate), the 3 m room test is kept as non-native |
| 8 | Chute open velocity | fall killed to 5.5 m/s at once | 40 % of the capped fall speed becomes forward speed, vy = 0.6 vc + (vy - vc) | **fixed** (formula); the glide that follows is Gap 1 |
| 9 | Skydive entry | animation after 1.5 s and < -12 m/s | `S_FALL_TO_SKYDIVE` as soon as 28.5 m of air is below | **fixed** for the pose; the camera's `is_skydiving` keeps the old rule (Gap 8) |
| 10 | Sprint | speed >= 5.6 m/s plays `dash_fwd` | the `S_DASH` state, key + input above 0.9 | **fixed** |
| 11 | Stopping | instant idle | `S_UPRIGHT_STOP` (`wk_to_idle_rgt` / `run_to_idle`, 0.4 s) or `S_DASH_STOP` (`dash_to_idle`) | **fixed** (pose); the velocity still drops at once (Gap 3) |
| 12 | Landings hold him | a light landing gave way to running | `S_JUMP_LAND`, `S_JUMP_LAND_HEAVY`, `S_LANDING_HARD` take no navigate or jump action until they end | **fixed**, **inferred** that no root motion means no walking (`Hero::move_lock`) |
| 13 | Jump gating | a press jumped at once | needs `ACT_JUMP_AT` accepted; a press is retried for 0.3 s (`01077710`) | **fixed**: lost during the first half of a landing, taken at its end |
| 14 | Swimming | clip by speed | `S_FALL` -> `S_BLEND_TO_SWIM_IDLE` (vertical > -15) or `S_SWIM_DIVE_IDLE`, `S_SWIM_RUN_TO_SWIM`, `S_SWIM_IDLE`/`S_SWIM_NAVIGATION` | **fixed** for the states met; dive/submerge Gap 9 |
| 15 | Walk 2.6 / run 4.5 / sprint 7.0 m/s | invented | root motion of the playing clips times the state's time scale (JC2-241): run 5.51, WALK key 2.53, sprint 5.21 rising to 8.33 over 1.5 s, aim strafe 5.00 | **fixed** (`navigation_speed`, `dash_speed`, `strafe_speed`, stick filter 0.1 per 30 Hz update); swim 2.2 m/s kept (Gap 3) |
| 16 | Chute glide 11, sink 5.5, turn 1.3 rad/s | invented | a lift/drag glider model (`005a9950`, `foot-controller-next.md` section 8) | **not changed**: Gap 1 (the neutral glide needs the canopy basis; with the inferred one it is anywhere from 5 to 25 m/s for a few degrees of pitch) |
| 17 | Jump 5.5 m/s impulse, gravity 9.81, air control +8 m/s^2 uncapped | start-clip climb 4.27 m/s for 0.367 s from t = 0.1 s, gravity -21, air steering 5.0 m/s x stick (JC2-241) | **fixed** (climb, gravity, steering; the hand-over, the no-braking rule and the sprint-jump 7.0 table are inferred/not modelled: Gap 3) |
| 18 | Facing | snapped to the input direction | turns at 2 x rate: 540 deg/s ground, 180 sprint and air (JC2-241); rotate/turn-180 states | **fixed** for the rate; the discrete turn states are Gap 4; aiming still turns instantly (720 deg/s native) |

With the default glide of 11 m/s every parachute landing is the roll (11 > 8.0); the nice landing needs the
slow glide. In the original the nice landing is the ordinary case, one more sign that the glide constants
are not native (Gap 1).

## 3. Headless traces (one scenario per path, 64 Hz, flat test tile)

`state` is the graph state, `clip` what the animator plays. Times are from the start of the scenario
(the trace tests assert these sequences).

**Walk, sprint, stop**

```
 0.03 s  S_UPRIGHT_BASIC_NAVIGATION  Walk -> Run (0.08 s)
 1.11 s  S_DASH                      Sprint           (Shift)
 2.59 s  S_UPRIGHT_BASIC_NAVIGATION  Run              (Shift released)
 2.83 s  S_UPRIGHT_STOP              RunToIdle        (W released)
 3.27 s  S_UPRIGHT_IDLE              Idle
```

**Jump from rest** (`ACT_JUMP_AT`; the landing holds him 0.55 s)

```
 0.02 s  S_JUMP_START_SPRINT_FORWARD  JumpStartRun    v.y +5.3
 0.48 s  S_JUMP_TO_FALL_TRANSITION    JumpTimeBlend
 1.12 s  S_JUMP_LAND                  LandLight       [move lock]
 1.67 s  S_JUMP_LAND_INTERRUPT        LandLight
 1.69 s  S_UPRIGHT_IDLE               Idle
```

A press at the start of the landing is lost; a press 0.25 s before it ends fires at `S_JUMP_LAND_INTERRUPT`
(3.34 s -> `S_JUMP_START_SPRINT_FORWARD` 3.36 s). **Running jump**: `S_JUMP_START_SPRINT_FORWARD` 7.98 s ->
`S_JUMP_TO_FALL_TRANSITION` 8.45 s -> `S_JUMP_RUNNING_LAND` (`run_jump_to_run`) 9.06 s ->
`S_JUMP_RUNNING_LAND_INTERRUPT` 9.41 s -> `S_UPRIGHT_BASIC_NAVIGATION` 9.42 s.

**Falls**

```
 3 m drop    S_FALL 0.03 s -> S_JUMP_LAND 0.64 s [lock] -> S_JUMP_LAND_INTERRUPT 1.19 s -> S_UPRIGHT_IDLE 1.20 s
 22 m drop   S_FALL 1.59 s -> S_JUMP_LAND_HEAVY 3.47 s (LandHeavy, lock, fall timer 1.9 s) -> S_UPRIGHT_IDLE 3.81 s
 130 m drop  S_FALL 0.03 -> S_FALL_TO_SKYDIVE 0.05 (FallToSkydive) -> S_SKYDIVE 0.72 s (Dive)
```

**Chute** (press at 2.5 s in the skydive; opening transform on the velocity)

```
 2.52 s  S_PULL_OPEN_PARACHUTE_VERTICAL  ChuteOpen   v=(-3.4,-16.1,-10.1)  canopy shown
 4.95 s  S_UNFOLD_PARACHUTE_VERTICAL     ChuteOpen   (the same clip, already finished: no replay)
 4.97 s  S_PARACHUTE                     Chute       v=(-0.4,-5.7,-10.9)
18.48 s  S_LANDING_ROLL                  LandRoll    horizontal 10.4 m/s at touch-down
19.45 s  S_UPRIGHT_IDLE                  Idle
```

Slow glide (S held, 5.4 m/s along): `S_PARACHUTE_LANDING` (`ChuteLand`, 0.8 s). Dive glide (W held, 14 m/s):
`S_LANDING_ROLL`. A second press cuts the canopy: `S_PARACHUTE_DETACH_TO_FREEFALL` (`ChuteDetach`) at
7.67 s, `S_SKYDIVE` 9.08 s. Presses in the 0.7 s lead-in and during a 22 m fall do nothing
(`trace_the_chord_needs_the_skydive`).

**Swim**: `S_FALL` 0.03 s -> `S_BLEND_TO_SWIM_IDLE` 0.64 s (vertical -7.6 m/s, above -15) -> `S_SWIM_IDLE`
0.84 s (`SwimIdle`) -> `S_SWIM_NAVIGATION` with W (`Swim`). The only graph condition met that the
evaluator cannot decide is `C_STEERING_FORWARD` (the submerge edge).

Visual captures were not taken: the change is clip choice and timing, which the traces pin; one frame of
a pulled chord or a roll adds nothing the state names do not say.

## 4. Gaps (unresolved native details)

1. **Parachute flight model parity.** `005a9950` (JC2-170 section 8) gives the force law, the opening
   transform (now used), the curves and the spring steering, but the neutral glide depends on the canopy
   basis (`R0/R1/R2`, bind pose, JC2-243); with the inferred basis a few degrees of canopy pitch move the
   level glide from 5 to 25 m/s (a model run in this task), so it was not ported. Blocks `CHUTE_*`
   constants and the turn rate (inferred from the spring in section 8 of that document: a steady yaw rate
   of about 0.8 rad/s per unit of steering, against the 1.3 in use). Existing task JC2-243.
2. **Landing assist and thrusters** (`005aa5db-005aa7f3`), covered by JC2-243.
3. **Remaining locomotion/jump values** (JC2-241 supplied the rest; its Gaps stay open): the swim, crouch and
   ladder speeds (swim 2.2 m/s is still this project's), the hand-over of the climb to the ballistic flight
   and whether the in-air context brakes horizontal speed (the runtime steers but does not brake: non-native),
   the sprint-jump 7.0 m/s steering table, the root motion of the landing clips (`land_chute` 0.69,
   `land_chute_roll` 3.77, `run_jump_to_run` 4.63 m/s), the native update tick (the stick filter runs at an
   assumed 30 Hz), and a stop that lasts as long as the stick's decay (the filtered stick takes ~0.7 s to fall
   under the 0.1 deadzone; the velocity follows the formula meanwhile). Tasks JC2-254/255/256.
4. **Ground blends.** `CBasicNavigationState` (`007d6790`: eight clips, weights from the state block fields
   `+0x3c/+0x40/+0x70/+0x334`), `CAimNavigationState`, `CDashState` lean/slope clips, `CIdleState`
   (`005fba50`: static `std_idle_var*` stance plus the additive breathing layer), `S_FALL` (base pose plus
   additive `fall_add*`), and the rotate/turn states (`S_UPRIGHT_ROTATE_*`, `S_UPRIGHT_TURN_180`,
   `S_RUN_STRAFE_*_TURN`). The runtime has no additive blend, so the stance and the fall use full clips.
5. **Chute pose weights**: the `+0x3c/+0x40` pair split between the `up` and `dn` clips of each axis, and
   the `S_SKYDIVE` four-clip blend (`CSkyDiveState`, `00853f70`): the runtime plays `rico_speed_tb_up`
   with the turn clip as a layer and `mhfreefall`.
6. **Whether the unfold replays**: `S_PULL_OPEN_PARACHUTE_VERTICAL` and `S_UNFOLD_PARACHUTE_VERTICAL` both
   bind `rico_open_chute` (2.43 s). The runtime plays it once (the second state finds it finished). Also
   the canopy graph (`parachute.afsm`) is not driving the canopy mesh: closing is instant and the
   horizontal opening (`chute_reel_open`) is unused.
7. **Ray fans** (RESOLVED in research by JC2-259, `foot-conditions-next.md` section 1; not yet wired). `C_PARACHUTE_TO_FALL/SKYDIVE_*` (10 rays, 30 m and 60 m), `C_CAN_FALL_TO_SKYDIVE`,
   `C_CAN_JUMP_TO_SKYDIVE`, `C_CAN_JUMP_CLEAR_AHEAD` have fan angles the evaluator does not read; one
   forward ray and the straight-down clearance stand in. `S_JUMP_START_SPRINT_FORWARD` can therefore never
   take its `S_JUMP_TO_SKYDIVE` edge as strictly as the game does.
8. **Camera and graph disagree** (RESOLVED in research by JC2-259, section 4: `CSkyDiveController` serves the skydive states; not yet wired): `gameplay_camera::is_skydiving` still uses airtime > 1.5 s and a vertical
   speed under -12 m/s; the skydive camera should follow the graph state (which controller selects the
   skydive camera is the open part of JC2-244).
9. **Water** (RESOLVED in research by JC2-259, section 2; dive motion remains open there): The swim-depth conditions are the Hips against the *wave* height with a -0.4 / -0.05
   hysteresis (the runtime uses the capsule centre against the flat sea and its own 0.05 m threshold);
   `S_SWIM_SUBMERGE`, `S_SWIM_DIVE_*` (`C_STEERING_FORWARD`, `C_VIEW_PITCH_TO_SURFACE`) are not driven.
   Related: JC2-220/JC2-228 (waves, lift).
10. **Skydive landing** (RESOLVED in research by JC2-259, section 3; the ragdoll entry remains open there): `S_SKYDIVE` + `ACT_LAND` goes to `S_HITREACT_UNCONTROLLED_FLIGHT` (a ragdoll
    state), the runtime plays the heavy landing instead.
11. **Dispatch order.** The order in which the controller sends its actions in one frame (jump, land,
    chord, navigate, stop, update) is inferred; so is `ACT_STRAFE` for aiming and the stop actions.
    Evade (TAB, `S_EVADE`, `evade_fwd`) and crouch are not driven at all.
12. **Collider seams.** On the flat test tile a running Rico loses ground contact for a frame every 4 m
    (v.y +1.1 m/s), which used to flash the jump clip; the machine now ignores a one-frame loss (a
    take-off needs the physics' own jump flag). The seam hop itself is in the terrain collider, not
    investigated.

## 5. Files

`crates/jc2-game/src/foot_machine.rs` (new), `character.rs`, `rig.rs` (14 clips), `main.rs`;
`research/tools/rico_controller_probe.py` (new), `afsm_probe.py` (polarity); this document and a pointer
in `character.md`.

## 6. Crouch and evasive roll (JC2-051)

Both now run through the foot machine (`foot_machine.rs` dispatch, `foot_actions.rs` for the key
timing and the clip-driven displacement, `rig.rs` for three more clips). Probe:
`research/tools/rico_controller_probe.py` (extended, exit 0). Traces: `trace_crouch`, `trace_evade`.

**Native (read from `kc001_base.asb` and `basestatemachine.afsm`, asserted by the probe and the traces)**
- Crouch (hold mode): `S_DOWN_TO_CROUCH` (`crouch_DN`, 0.4 s, blend 0.2) -> `S_CROUCH` (`cr_idle`) ->
  `S_UP_FROM_CROUCH` (`crouch_UP`, 0.5 s, 0.23 m of root motion) -> idle. `ACT_CROUCH` is accepted only while
  the input magnitude is not above 0.7 (polarity read from the graph as for the other 0.7 tests); above it
  the graph stands him into the run. The graph has no `S_CROUCH_WALK`, so there is no crouched walking, and
  `S_CROUCH` leaves straight into `S_UPRIGHT_BASIC_NAVIGATION` (not through `S_UP_FROM_CROUCH`) when the
  stick passes 0.7. The jump edge is open from `S_CROUCH`.
- Evade: `S_EVADE` (`evade_fwd`, 0.933 s, 5.69 m of root motion) -> `S_EVADE_INTERRUPT` -> idle, or the dash
  when sprint is held. `S_EVADE` takes no jump, crouch or second evade. The press is the key edge
  (keyboard) or a tap released within 0.25 s (pad, threshold at `+0x30c`); the sprint is locked for 0.25 s
  after an evade press. `00507a10` tests `S_EVADE` only; `005ebc90` blocks the crouch in the left-arm
  grenade-throw states; `00572ca0` is the crouch family. Turn rates: 10 rad/s crouching, pi rad/s rolling.
- Displacement: the root motion of the clip drives him (frame-rate independent, `RootDrive`); the roll
  travels 5.69 m +- 0.7 on the flat test tile.

**Project choices (non-native)**: Left Ctrl is crouch, C is the evade (Tab is taken by another binding).
The roll tail uses the `ACT_NONE` fallback because `S_EVADE_INTERRUPT` has no navigate edge.

**Test correction.** `trace_crouch` failed on its own assertion, not on behaviour: the recorder (`follow`)
logs state changes only, so the `S_CROUCH` he leaves on the W press had been logged by the previous
follow and the new log began at `S_UPRIGHT_BASIC_NAVIGATION`. The test now reads the state before the
press (`S_CROUCH`) and asserts that the first state after it is the navigation state and that
`S_UP_FROM_CROUCH` is not visited, which is the graph edge. No constant was changed.

**Gaps (unresolved)**
1. Capsule height and ceiling clearance: RESOLVED in research by JC2-264 (section 7 below); not yet wired
   in the runtime (the capsule is unchanged and there is no low-ceiling behaviour).
2. Crouch guards not wired: RESOLVED in research by JC2-264 (section 7 below); not yet wired in the runtime.
3. Roll heading with no input and how the executor (`00a3d500`/`0051f670`) targets the direction
   (inferred: the raw live stick at pi rad/s).
4. The roll tail dispatch (`S_EVADE_INTERRUPT` has no navigate edge; `ACT_NONE` is non-native), and whether
   `S_EVADE_INTERRUPT` replays `evade_fwd` (treated as no replay, like the other `_INTERRUPT` states).
5. The `+0x30c` tap threshold may be overridden by data.
6. Input magnitude uses the raw stick; native uses the filtered `S+0x8c`.
7. Camera 69 for the crouch states is not applied.
8. `S_ROLL_OUT_LEFT` (the vehicle bail-out roll, `evade_fwd`) is not driven.
9. Left Ctrl is also the hang-drop key; native bindings for crouch and evade are not read from the
   input-map data.

## 7. Crouch collider, stand check and crouch guards (JC2-264, research only)

Probe: `research/tools/rico_controller_probe.py --export ...` (`crouch_collider_checks`, 52 of the 132 checks, exit 0).
Executable fingerprint `df99a479...b74f80` (see the probe). Nothing here is wired in the runtime yet.

**What changes in the collider.** The graph does not move the collider; the character object
(`CPfxCharacterInstance`, constructor `009ae780`, vtable `00f27744`) owns a phantom (`this+0x9c`) and five
shapes built once by `009ba850`, selected by an index. Metres, +Y up, feet at the origin; capsule = segment
plus radius, so the top of a capsule is its top segment point plus the radius:

| idx | slot | shape | height |
|---|---|---|---|
| 0 | `+0x88` | capsule, segment y 0.3 to 1.4, radius 0.3 (`e27cb0`, `e5e820`) | 1.7 |
| 1 | `+0x8c` | capsule, segment y 0.3 to 0.7 (`e30748`), radius 0.3 | 1.0 |
| 2 | `+0x90` | capsule, horizontal: a = (0, -0.2, -0.5), b = (0, -0.6, 0.5), radius 0.5 (`e56bf0`, `ebd04c`, `e3bfcc`, `e21a28`) | swim |
| 3 | `+0x94` | the same object as idx 0 (stored twice) | 1.7 |
| 4 | `+0x98` | shared convex ring (`DAT_013eb1e0`), no known writer selects it | unknown |

**Which index per state** (`0070a8a0`, CPlayer vtable `00e4cd64` slot 6, runs every frame; it only stores the
desired index at `character+0x5ac` through `0071c910`, which skips the store when it equals the current index
`this+0xa0`, read by `0098f180`). First match wins:
1. idx 2: `006b46b0` (the five `S_SWIM_DIVE_*` states) or layer-0 state in `S_SKYDIVE`, `S_SWIM_IDLE`,
   `S_SWIM_NAVIGATION`, `S_SWIM_ROTATE_CW/CCW`, `S_SWIM_DIE`, `S_SWIM_SUBMERGE`, `S_SWIM_RUN_TO_SWIM`.
2. idx 1: `00572ca0` (`S_CROUCH`, `S_CROUCH_WALK`, `S_DOWN_TO_CROUCH`, `S_UP_FROM_CROUCH`), `006a5e90` (the 19
   `S_CLING_*` states), `0064daa0` (`S_RIDING_MC`, `S_REVERSE_MC`, `S_IDLE_VEHICLE`,
   `S_IDLE_PASSENGER_VEHICLE`, `S_SWITCH_DRIVING_DIRECTION`) or `S_REEL_FLIGHT`, `S_REEL_END_LAND`,
   `S_REEL_IMPACT_VERTICAL(_INTERRUPT)`, `S_REEL_IMPACT_HORIZONTAL`, `S_REEL_TO_SO(_IMPACT)`,
   `S_REELED_IN_IDLE`, `S_REELED_IN_TO_HANG`.
3. idx 3 (same shape as 0): `0076fc30` (`S_UNFOLD_PARACHUTE_HORIZONTAL/VERTICAL`, `S_PARACHUTE`).
4. idx 0 otherwise.
So the collider is crouch-sized exactly while the crouch family is the layer-0 state, and also during the
cling, riding and reel states. There is no height blend: the shape is swapped whole.

**The stand check.** Per frame `005ceee0` (CCombatantComponent vtable `00ec293c` slot 4) calls `00696990`:
if the desired index differs from the current one, `009b6180(idx)` decides and only a true result lets
`009a20e0(idx)` swap the phantom's shape (remove from the world, set shape, re-add; no-op without a physics
world, `005db8e0`). The swap check applies to every change, not only to standing up. `009b6180`:
1. false if the pose `this+0xe0` (xyz) is outside the world bounds box (`0098c5c0`, margin 0).
2. places the candidate shape `this[0x88 + idx*4]` at that pose with the phantom's own collision filter
   (`phantom+0x2c`) and runs one overlap query (`00b7aec0`, tolerance 1.19e-7 (`e2f088`), max distance
   3.4e38) into two contact collectors (`hkpAllCdPointCollector`).
3. true only if neither collector is blocked. `00993690` (blocked): any contact (0x50 bytes each) with
   distance below -0.2 (`e56bf0`, i.e. penetration deeper than 0.2 m) whose body has property key 1 and whose
   object's virtual method at +0x18, called with 1, returns true.
Consequence: under a low ceiling the graph still plays `S_UP_FROM_CROUCH` and goes to idle (it has no
predicate), but the collider stays at idx 1 (1.0 m) until the 1.7 m capsule would penetrate nothing deeper
than 0.2 m; the check repeats every frame. Inferred (flat ceiling, contact distance = penetration depth): a
ceiling is tolerated down to 0.2 m inside the stand capsule, i.e. about 1.5 m of clearance, not a hard 1.7 m.

**ACT_CROUCH guards** (`007b6520`, called by `00821a90`, COnFootController vtable `00ec2470` slot 6, controller
`this`; input action 0x24 is the crouch key, down = state bits 2 or 3, pressed edge = 2):
| guard | where | effect |
|---|---|---|
| `this+0x20` != 0 | `007b6520` | no crouch at all (hold and toggle). It is the scope flag set by `0058e060` when the zoom mode is 2 or 3 (`gui.sniper.stage1`/`stage2`) |
| `this+0x1c` == 1 | `007b6520` | no crouch at all. `this+0x1c` is that zoom mode (0 off, 2 stage 1, 3 stage 2); the meaning of 1 is unresolved (the `gui.sniper.off` branch, set by `006f18a0`) |
| `005ebc90` | both paths | no crouch while layer 1 is in `LA_S_UNDER_THROW_GRENADE`, `LA_S_OVER_THROW_GRENADE` or `LA_S_THROW_TRIGGERED_EXPLOSIVE` |
| busy | both paths | no crouch while input latch `char->0xa34 + 0x258` or `+0x264` is set (meaning of the latches unresolved) |
| setting 0x28 == 0 | `007b6520` | hold: dispatch `005d4e60` every frame the key is down |
| setting 0x28 != 0 | `007b6520` | toggle (identification is inferred from the code, no option string found): the press edge flips `char->0xa34 + 0x33b`; while set, dispatch each frame unless `004f93d0` (`00675110` or `00576890`), `00816300` (grapple device attached), `0075c080` (the five landing states `S_JUMP_LAND*`) or `007369c0(0)` (`0067d1e0(0)` or `006fd9a0(0)`) |
| grapple device `char+0x484 -> +0xd0` | `005d4e60` | `+0x2ad` != 0: crouch refused. Otherwise, if `+0x128` != 0 (a hook attached), `007504b0` detaches it first |
| ACT_CROUCH dispatch | `005d4e60` | `vtable+0x74(ACT_CROUCH)` on the char state machine; only if accepted: `00534ce0`, input latches `+0x259 = +0x265 = 1`, `+0x258 = +0x264 = 0`, other latches cleared, `005ac3f0(0)` |

**Native-proven**: everything above is read from the executable and asserted by the probe, except the points
marked unresolved or inferred. **Inferred**: that the shape swap is the "stand check" the doc named (the
function is unnamed; behaviour read), that setting 0x28 is the crouch hold/toggle option, the labels
"scope flag" and "grapple device".

**Gaps (JC2-264)**
1. Meaning of zoom mode 1 (`this+0x1c == 1`) and who sets it (`006f18a0`); blocks exact scoped-crouch parity.
2. The object method at vtable +0x18 called by `00993690` and the meaning of body property key 1 (which
   bodies block standing); until known the runtime can only treat world geometry as blocking.
3. Meaning of the input latches `+0x258`, `+0x264` and what raises them (`005a25e0`, `0067c740`, `0065b3f0`...).
4. Predicates `00675110`, `00576890`, `0067d1e0(0)`, `006fd9a0(0)` of the toggle path.
5. `006a5e90` is read through the state machine vtable `+0x80` (a different layer or machine than
   `008df200`); which layer is unresolved.
6. Shape idx 4 (`+0x98`, convex ring) has no known selector; the runtime never needs it unless one is found.
7. The visible effect of the 0.2 m tolerance depends on Havok contact distances, which the Rust capsule
   backend may not reproduce exactly (non-native).
