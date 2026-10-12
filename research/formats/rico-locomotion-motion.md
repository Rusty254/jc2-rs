# Rico on foot: where walk, run, sprint, turn and jump motion come from (JC2-241)

Research only. Probe: `research/tools/locomotion_motion_probe.py` (84 checks, exit 0). It needs a `jc2tool` built from
the JC2-220 commit or later (it prints `RM i x y z yaw` root-motion lines):

```
python research/tools/locomotion_motion_probe.py --game <game folder> --tool <jc2tool.exe> [--report]
```

Executable fingerprint: `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
Addresses are virtual addresses. Labels: **native** = read from code or data and asserted by the probe; **inferred** =
reasoned from native facts (always said so); **unknown** = listed under Gaps. Closes Gap 1 of `foot-controller-next.md`
(JC2-170) except the airborne part (Gap 1 below). Unblocks JC2-238.

## 1. Short answers

* **There is no speed, turn-rate or jump constant in the on-foot input handler** (`foot-controller-next.md`). The
  controller picks *which animation action* to dispatch; the motion comes from the animation graph.
* **Ground speed = root motion of the playing clips x the clip time scale x the blend weights**, computed by the
  locomotion state classes every frame. The time-scale and blend parameters are data in the state nodes of
  `basestatemachine.afsm`; the clip speeds are the extracted root motion of the `.ban` files (all locomotion clips are
  1.2 s loops sampled at 30 Hz, asb playback speed 1.0).
* **Native speeds (m/s, straight ahead, lean = slope = 0 which is always the case for the player):**

  | Mode | Native | Current guess in `character.rs` |
  |---|---|---|
  | full stick run (`S_UPRIGHT_BASIC_NAVIGATION`) | **5.51** (time scale 1.6525 x `run_fwd` 3.333) | RUN 4.5 |
  | WALK key (axes x 0.425, filtered stick 0.425) | **2.53** (run weight 0.237) | WALK 2.6 |
  | sprint start (`S_DASH`, counter c = 1) | **5.21** (time scale 1.5625 x `run_fwd`) | SPRINT 7.0 |
  | sprint after 1.5 s (c = 0) | **8.33** (time scale 2.0 x `dash_fwd` 4.167) | SPRINT 7.0 |
  | aiming strafe (`S_UPRIGHT_STRAFE`) full stick / WALK key | **5.00** / **2.06** | |

* **Turn rate:** the facing target is set instantly from the camera-relative stick direction, and the character turns
  towards it at a **per-state limit of `2 x rate` rad/s** (default `rate` = 1.5 pi, so 540 deg/s on the ground; sprint, evade and
  every airborne state pi/2, so 180 deg/s; aiming strafe 2 pi; crouch 5.0; swim 1.0). The stick itself is low-passed (section 3).
* **Gravity is -21.0 m/s^2** (`settings/player.bin` key `0xc481b248`, applied to `CPfxCharacterInstance + 0xbc`; the
  constructor default is -9.82).
* **Jump:** the vertical take-off is the **root-motion Y of the start clip** (running: 4.27 m/s plateau from t = 0.1 s for
  0.367 s, +1.564 m; standing: peak 6.25 m/s, +1.383 m in 0.5 s), then the character is ballistic (inferred hand-over): apex
  about 2.0 m running, 1.4 m standing. **Air steering** is a lift velocity of 5.0 m/s x stick (falls and the flight after a
  sprint jump), 7.0 m/s in the aligned sprint-jump states, none in standing jumps (section 7).

## 2. Data flow (native unless marked)

```
raw keys/pad  --0x007b6520-->  S+0x7c/+0x80 (axes, WALK x0.425)
              --0x0066af80-->  motion driver (char+0x9c8, vtable +0x50 = 0x0067f790)
                               S+0x8c/+0x90 = low-pass(S+0x7c/+0x80), S+0x70 = |S+0x8c,+0x90|
              --0x007ab170 / 0x00727840 / 0x0063b110--> animation action (ACT_NAVIGATE, ACT_DASH ...) + facing request
animation states (layer 0), per frame:
   0x007d6790 (navigation)  0x00606cb0 (dash)  0x007ea9a0 (aim strafe)
      -> per clip: time scale (clip object +0x60) and blend weight (+0x5c)
Havok-style extraction -> root-motion velocity (m/s, inferred) at (char+0x208 object)+0x1c..0x24
air steering 0x007f5810 / swim / grapple -> lift vector (char +0x1c0, 0x00867fa0), m/s, in the intent chain before the states run
0x00895f80 / 0x00901900 -> root motion + lift -> CCharacterBase +0x270 vector
0x008672a0 -> 0x0098f960 (CPfxCharacterInstance update: hkpCharacterContext with stock states) -> position
turn: 0x008aa820 (target quaternion char+0x12c from the filtered stick) -> 0x00819930 -> 0x006831f0 (rate limited slerp)
```

## 3. The motion driver at character +0x9c8 (native)

The object stored at character `+0x9c8` by `0x00519070` is the avatar interface: for Rico the sub-object at `CPlayer + 8`
(vtable `0x00e4cd64`, `CPlayer` constructor `0x007359b0` builds a `CAvatar`, `0x007c6810`). NPC bodies use
`CAgentAvatar` / `CCombatantAgentAvatar` (`0x007b4ac0`, `0x007f78c0`) with the same slots. Slots: `+0x50`
`0x0067f790` (update), `+0x54` `0x007470a0` (reset movement filters), `+0x58` `0x00633750` (reset blend filters), `+0x5c`
`0x0074ac40` (write S+0x3c/+0x40 from the blend filters), `+0x60` `0x00510cc0` / `+0x64` `0x0084a8c0` (feed the blend
filters: the AI path), `+0xac` `0x006c7380` (turn rate, section 5), `+0xb0` `0x007ff8e0` (default turn parameter).

* `0x0066af80` stores the axes in S `+0x7c/+0x80`, the clamped length in S `+0x6c`, calls slot `+0x50`, then stores the
  length of S `+0x8c/+0x90` in S `+0x70`.
* Slot `+0x50` pushes S `+0x7c` and S `+0x80` through two **first-order filters** (this+8 and this+0x14, `0x004d6150`):
  `v = (1 - a) * v + a * x` per call (values below 1e-4 snap to 0), writes S `+0x8c` / `+0x90`. After a reset (slot
  `+0x54`, which also re-arms the filter's snap flag) the **first sample snaps** to the input. `a` is `avatar + 0x38` =
  **0.1** (`0x007c6810` stores `f32[0x00e290bc]` there; the filters are first built with 0.02, `f32[0x00f09268]`, and take
  0.1 at the reset). The reset runs whenever the on-foot controller is activated or deactivated (`COnFootController`
  vtable `0x00ec2470` slots `+0x28` = `0x00669e20`, `+0x2c` = `0x00655280`). The constructor builds four filters: move X,
  move Z, and two blend parameters (this+0x20, +0x2c).
* The filter is **per call, not per second**. Native tick rate for the on-foot controller: unknown (animation data is
  authored at 30 Hz). Stick magnitude reaches 0.5 after 7 calls and 0.9 after 22 calls.
* S `+0x3c` and S `+0x40` (lean and slope blend inputs of the navigation and dash states) are written only by slot
  `+0x5c` (AI) and zeroed by the jump, evade, rotate and reset code; **the player never sets them** (the nav/dash models
  below use 0).
* Keyboard diagonals are not normalised before the filter (axes 1, 1 gives length 1.414); every state caps the effect
  (the run weight is clamped), so the speed is the same as straight ahead.

## 4. Ground locomotion states (native)

State parameters are keys of the state node in `basestatemachine.afsm` (key hashes confirmed by the loader tables
`0x00ec46ec..0x00ec46f8`, `0x00ec4724`). Clip slots are the ordinal keys of the clip list of the state's animation id in
`kc001_base.asb` (hash-sorted; the ordinal of each key is fixed by what the update body does with it: asserted by the
probe).

### 4.1 `S_UPRIGHT_BASIC_NAVIGATION` (`CBasicNavigationState`, update `0x007d6790`, loader `0x005c1430`)

Parameters: +0x20 `943b6891` = 0.66 (offset), +0x24 `e6963dbd` = 1.042 (run starts), +0x28 `7ef333c9` = 1.5625 (run
full), +0x2c `dd70f454` = 1.5825 (maximum). Clip slots: 0 `std_idle_var0` (weight always 0, time scale 0), 1 `wk_fwd`,
2 `run_fwd`, 3/4 `run_lean_lft` / `run_lean_rgt` (weights from S+0x3c), 5 `run_fwd_down`, 6 `run_fwd_up` (weights from
S+0x40), 7 `dash_fwd` (weight = carry-over counter this+0x30). With sp = S+0x70, lean = slope = 0:

```
ts   = min(1.5825 * sp + 0.66, 1.5825)
w    = clamp((ts - 1.042) / (1.5625 - 1.042), 0, 1) * sp         # run weight; walk weight = clamp(1 - w)
ts_c = ts + 0.07                                                   # 0.3 * 0.5 - 0.22 subtracted (f32 0xe27cb0, 0xe474d0)
speed = ts_c * (walk_w * 1.3325 + run_w * 3.3333 + dash_carry * 4.1667)
```

| sp | ts_c | run weight | speed m/s |
|---|---|---|---|
| 0.0 | 0.730 | 0 | 0.97 |
| 0.1 | 0.888 | 0 | 1.18 |
| 0.2 | 1.047 | 0 | 1.39 |
| 0.3 | 1.205 | 0.053 | 1.73 |
| 0.425 (WALK key) | 1.403 | 0.237 | 2.53 |
| 0.5 | 1.521 | 0.393 | 3.22 |
| 0.7 | 1.653 | 0.700 | 4.52 |
| 0.9 | 1.653 | 0.900 | 5.18 |
| 1.0 | 1.653 | 1.0 | 5.51 |

When the previous state was a dash state (character `S+0x338`, set by `0x00727840`) the carry-over counter `this+0x30` is
set to 1: the dash clip keeps weight `counter`, the time scale blends from the dash's last scale (`S+0x334`) towards the
navigation one, and the counter falls by dt * 5 (0.2 s).

### 4.2 `S_DASH` (`CDashState`, update `0x00606cb0`, loader `0x00756680`, enter `0x007e3ee0`)

Parameters: +0x20 `7ef333c9` = 1.5625, +0x24 `8dab8a4a` = 2.0, +0x2c `78f64dea` = 1.5. Clips: 0 `run_fwd` (weight c), 1
`dash_fwd` (weight 1 - c), 2/3 `dash_lean_lft/rgt`, 4 `run_fwd_down`, 5 `run_fwd_up`. Enter sets `c = this+0x28 = 1.0`; every frame
`c -= dt / 1.5`; time scale `ts = (1.5625 - 2.0) * max(c, 2|slope|) + 2.0`:

| c | ts | speed m/s |
|---|---|---|
| 1.0 | 1.5625 | 5.21 |
| 0.75 | 1.672 | 5.92 |
| 0.5 | 1.781 | 6.68 |
| 0.25 | 1.891 | 7.48 |
| 0.0 | 2.0 | 8.33 |

`S_INSTANT_DASH` has its own node (1.54 / 1.7 / 1.2 s, steady 7.08 m/s) but `ACT_INSTANT_DASH` (hash global
`0x011970f8`) is **never dispatched by code** (only the static initialiser and the destructor reference it); the flag
`S+0x33a` that would zero the counter is never set. Rico's sprint is `S_DASH`. `ACT_DASH` is dispatched by the sprint helper `0x00534a00`
(called from `0x005ff2f0`); the self-transition `S_DASH -> S_DASH` does not re-enter.

### 4.3 `S_UPRIGHT_STRAFE` (`CAimNavigationState`, update `0x007ea9a0`, loader `0x00824f30`)

Parameters: +0x20 `dd70f454` = 1.5625 (K), +0x24 `943b6891` = 0.6 (offset), +0x28 `e6963dbd` = 0.96 (scale), +0x2c
`7ef333c9` = 0.64. Ten clips (run/walk x forward/left/right + strafe variants) weighted by the angle S+0xc between facing and
movement (not modelled). Straight ahead: `ts = 0.96 * min(1.5625 sp + 0.6, 1.5625)`, run weight
`clamp((min(...) - 1/0.96) / (1/0.64 - 1/0.96)) * sp`. Full stick 5.00 m/s, WALK key 2.06 m/s.

### 4.4 Stopping, starting, evade (clip facts)

`S_UPRIGHT_STOP` (`wk_to_idle_rgt` 0.57 m/s mean, `run_to_idle` -0.275 m net), `S_DASH_STOP` (`dash_to_idle` 0.44 m net,
speed 0.8), `S_EVADE` (`evade_fwd`: 5.69 m in 0.933 s, peak 15.7 m/s). Stop is entered when S+0x6c and S+0x70 are both
<= 0.1 (`0x0063b110`).

## 5. Turning (native)

`0x008aa820` (called with `ecx = character + 0xf0`) builds the look-at quaternion for the camera-relative direction
(`0x007d5350`) and stores it at character `+0x12c`, setting bit 3 of the flags at `+0x268`. Each frame `0x00819930` (when the
bit is set) calls `0x006831f0`: `theta = acos(|q_current . q_target|)` (`0x004e80b0`), `t = rate / theta * dt` with `rate =
char[+0x9c0]` (relative to the character base), slerp by `t` (`0x004f1af0`) when `t < 1`, otherwise snap. `theta` is **half**
the rotation angle, so the angular speed is `2 x rate` (arithmetic on the code; no independent measurement).

`rate` is refreshed by `CPlayer` vtable `+0xac` (`0x006c7380`), a first-match table over the layer-0 animation state:

| State set | rate (rad/s) | speed (deg/s) |
|---|---|---|
| `S_UNCONTROLLED_SKYDIVE`, `S_LESS_UNCONTROLLED_SKYDIVE` (`0x005d68f0`) | 5.0 | 573 |
| 8 swim states including `S_BLEND_TO_SWIM_IDLE` (`0x0082c7e0`), 5 dive states (`0x006b46b0`) | 1.0 | 115 |
| `S_EVADE` (`0x00507a10`) | pi/2 | 180 |
| `S_DASH`, `S_DASH_TURN_180`, `S_INSTANT_DASH` (`0x00615e60`) | pi/2 | 180 |
| every airborne state: `S_JUMP_FALLING_TO_SKYDIVE`, `S_JUMP_START_SPRINT_FORWARD`, `S_JUMP_FLIGHT_SPRINT_FORWARD` (`0x00601710`), `S_FALL`, `S_JUMP_TO_FALL_TRANSITION` (`0x00727b60`) (`0x0062d170`) | S+0xa8 = pi/2 (player.bin `0xf70b7752`) | 180 |
| `S_PARACHUTE` (`0x00849700`) | parachute data block +0x1c | unknown |
| `S_UPRIGHT_STRAFE` while player controlled (`0x00675110`) | 2 pi | 720 |
| `S_CROUCH`, `S_CROUCH_WALK`, `S_DOWN_TO_CROUCH`, `S_UP_FROM_CROUCH` (`0x00572ca0`) | 5.0 | 573 |
| everything else (idle, navigation, jump, fall) | 1.5 pi | 540 |

A 180 degree turn at the default rate takes 10 steps of 1/30 s (0.333 s); while sprinting or airborne it takes 1.0 s. Together with
the stick filter (section 3) this is the whole turn model for the continuous case. While sprinting `0x005ff2f0` uses the raw stick S+0x7c (not the filtered S+0x8c) for the facing target.

Discrete turns are animation driven: reversing (cosine <= -0.6, S+0x70 <= 0.7) plays `run_fwd_turn_180`: yaw -pi reached over
frames 3..8 (0.167 s, 18.85 rad/s) with 0.30 m net travel; standing turns (cosine < 0.67) blend `std_turn_30_cw/ccw` (30 degrees) with
`std_turn_180_cw/ccw` (180 degrees in 0.167 s at 18.85-20 rad/s) by the angle (S+0x47c); both have no travel.

## 6. Clip facts (native, `jc2tool anim --at`)

| Clip | Duration | Root motion |
|---|---|---|
| `wk_fwd`, `wk_lft/rgt`, `wkstrafe_*` | 1.2 s | 1.599 m (1.3325 m/s) |
| `run_fwd`, `run_lft/rgt`, `run_lean_*`, `runstrafe_*`, `run_fwd_down` | 1.2 s | 4.0 m (3.333 m/s); `run_fwd_up` 3.959 m |
| `dash_fwd`, `dash_lean_*` | 1.2 s | 5.0 m (4.167 m/s) |
| `run_fwd_turn_180` | 0.567 s | yaw -pi, 0.30 m |
| `run_jump_start` | 0.467 s | Y 0 for 0.1 s then +4.27 m/s for 0.367 s = +1.564 m, no horizontal |
| `std_still_jump_start` | 0.5 s | Y peak 6.25 m/s, +1.383 m, no horizontal |
| `run_jump_timeblend` (`S_JUMP_TO_FALL_TRANSITION`, asb speed 1.2) | 1.467 s | forward 4.63 -> 4.34 m/s (x 1.2), Y about 0 |
| `std_still_jump_flight` (`S_JUMP_FLIGHT_STANDING`, asb speed 0.3) | 0.233 s | tiny |
| `run_jump_to_run` / `run_jump_land` | 0.333 s | 1.54 m / 0.97 m forward |
| `evade_fwd` | 0.933 s | 5.69 m |

## 7. Jump and air (structure native, hand-over inferred)

### 7.1 Tuning data and the physics instance (native)

`settings/player.bin` is read by the player initialiser `0x00664440`. Under the nested nodes `0x3cc54b15` / `0x51076a25` it
holds three values (keys `0xebbbdc..0xebbbe4`): `0xc481b248` = **-21.0** -> `0x007958f0` writes it to
`[[character + 0x230] + 0xbc]`, the gravity slot of `CPfxCharacterInstance` (constructor `0x009ae780` default **-9.82**,
`f32[0x00e4cc8c]`; the setter is skipped when the value is 0); `0x77fd88be` = **5.0** -> S `+0x9c` (air steering speed);
`0xf70b7752` = **pi/2** -> S `+0xa8` (airborne turn rate). Terminal velocity: not found.

The physics instance wraps a stock Havok character context built by `0x009aaca0`: OnGround (state 0; defaults gain 1.0, speed
10, max acceleration 100), Jumping (1; jumpHeight 1.5), InAir (2; gain 0.05, speed 10, max acceleration 100), Flying (4). Only the
constructors reference these vtables (the probe asserts the constructor constants), so the defaults are not overridden by code.
**Closed by JC2-254 (`rico-jump-air-context.md`): the hand-over is native, not inferred; correction: record `+0x88` is the step dt, gravity is the vector at `+0xb0`.** Two flag bytes of the input record (character `+0x36c`, `+0x36d`) select the state: `CJumpControllerState` enter/exit
(`0x006ae480`, `0x0076f9c0`) set/clear `+0x36c` (`wantJump`), and `CVelocityControlledState` / `CVelocityTimeVariableState` enter/exit
(`0x007f0a90`, `0x006e0080`) set/clear `+0x36d`. OnGround update `0x009c1f90`: `+0x36c` -> Jumping, `+0x36d` -> Flying, unsupported ->
InAir. Jumping update `0x009bfdc0` follows the desired velocity with gain 1.0 and returns to InAir when the flag clears. InAir update
`0x00994c40` keeps the vertical component of the current velocity, adds gravity (input scalar +0x88 times the vector at +0xb0) and
moves the horizontal component towards the desired velocity by gain 0.05 per update (`0x009acd10`). The desired velocity is the
animation root-motion velocity plus the lift vector (`0x00901900` -> `+0x270`, section 2).

### 7.2 The start clips carry the climb

`0x00816700` applies no impulse. `run_jump_start` (S_JUMP_START_SPRINT_FORWARD) has Y root motion 0 for 0.1 s and then 4.27 m/s for
0.367 s (+1.564 m, no horizontal travel); `std_still_jump_start` (S_JUMP_START_STANDING) peaks at 6.25 m/s and totals +1.383 m in
0.5 s. `jump_start.event_track` has one event `jump_start` at t = 0.1 s, the instant the climb starts (handler not located).
Inferred hand-over (consistent with the code above, not proven): during the start state the Jumping state follows the clip's vertical
velocity; when the state exits the character is InAir and keeps the vertical speed it had (4.27 m/s running, 1.14 m/s standing)
while gravity -21 pulls it down: running apex 1.564 + 4.27^2 / 42 = 2.00 m above the take-off, standing 1.383 + 1.14^2 / 42 = 1.41 m.

### 7.3 Air steering `0x007f5810` (native)

The first override of the intent chain `0x007ab170` runs every frame. It is active only in the airborne states (`0x0062d170`):
`S_FALL`, `S_JUMP_TO_FALL_TRANSITION`, `S_JUMP_FALLING_TO_SKYDIVE` (normal branch) and `S_JUMP_START_SPRINT_FORWARD`,
`S_JUMP_FLIGHT_SPRINT_FORWARD` (sprint-jump branch); **not** in `S_JUMP_START_STANDING` / `S_JUMP_FLIGHT_STANDING`. Its output is
the lift vector (`0x00867fa0`: world to character frame, stored at `+0x1c0`, a velocity in m/s added to the root motion; the same vector
the swim controller uses, JC2-220). It also faces the stick (`0x008aa820`) with the airborne turn rate.

* Normal branch: `lift = max(S+0xc8, S+0x9c) * clamp(S+0x70, 0, 1) * stick direction` with `S+0x9c` = 5.0 and `S+0xc8` <= 1 (the length of
  the unit take-off direction saved in S `+0xbc..+0xc4` by `0x00816700`): **5.0 m/s at full stick**. In `S_FALL` the clip has no root motion,
  so this is the whole horizontal air speed; in `S_JUMP_TO_FALL_TRANSITION` it adds to the clip's forward root motion
  (`run_jump_timeblend` 4.63 -> 4.34 m/s times the asb playback speed 1.2).
* Sprint-jump branch (`0x006dfcc0`, `0x006e6400`): target = `table[bucket] * S+0xa0` along the stick, where the bucket comes from the cosine
  between facing and stick (thresholds 0.1 / 0.8 / 1.0, table 0 / 0.5 / 1.0) and `S+0xa0` = **7.0** (constructor `0x005a25e0` defaults, no data
  override found): 7.0 m/s aligned, 3.5 m/s for a diagonal, 0 behind. S `+0xbc..` moves towards the target by 15 * dt / distance per update
  (about 15 m/s of vector change per second); the lift is that vector times the stick magnitude.
* S `+0xcc..+0xd4` is an accumulator added to the normal-branch lift and decayed by 0.8 * dt; its writers were not found (inferred: external
  pushes).

## 8. The five parameter blocks of `COnFootController` (`0x005ea260`)

Bases `+0x50, +0xdc, +0x168, +0x1f4, +0x280`, 35 dwords each, flag word first. Decoded values are (min, max) style pairs
such as 0.025/0.1, 0.01/0.2, 0.4/1.0, 0.2/1.0, 0.4/0.8, 0.35/0.45 with flag bits 1, 2, 0x40, 0x100, 0x200, 0x400, 0x10000. They
are **not speeds** (no value matches a speed, rate or height). Inferred: input response ranges per device; their consumer is
not traced and they do not affect locomotion speed. Controller `+0x34` = 0.1 (stop deadzone), `+0x38` = 0.8, `+0x3c`/`+0x44` = 5 sqrt 2,
`+0x40` = pi, `+0x4c` = 0.25 are other constructor fields of unknown role.

## 9. Runtime guidance for the Rust controller (JC2-238)

1. Drop `WALK/RUN/SPRINT`: keep the stick filter `v += 0.1 * (x - v)` per controller update (axes x 0.425 with WALK) and use
   `sp = |v|`. Convert to time if the port runs at a different rate than the native tick (unknown; 30 Hz is the clip rate).
2. Ground speed: evaluate the navigation formula (4.1) while not sprinting, the dash formula (4.2, counter c from 1 to 0 in 1.5 s
   from entry) while sprinting, the strafe formula (4.3) while aiming; multiply by the clip root-motion speeds above. The
   blend-in/out of states is the asb blend (`S_UPRIGHT_BASIC_NAVIGATION` 0.24 s, `S_DASH` 0.2 s, strafe 0.13 s): not applied to the speed model.
3. Turning: face the camera-relative filtered stick direction with the angular speed `2 x rate` of section 5 (default
   540 deg/s, sprint 180 deg/s). Reverse/standing turns are the clips' yaw.
4. Jump: after a 0.1 s delay follow the start clip's vertical root motion (running 4.27 m/s for 0.367 s, standing the clip's profile), then
   ballistic with **g = 21** keeping the vertical speed reached; label the hand-over as inferred until Gap 1 is closed.
5. Air: while falling or in the flight after a running jump add a steering velocity of 5.0 m/s x stick (camera-relative); during a
   sprint jump use the 7.0 m/s bucket model of 7.3; no steering in a standing jump. Face the stick at 180 deg/s while airborne.
6. This document does not cover the parachute canopy (`foot-controller-next.md` section 8).

## Gaps (unresolved native details)

1. **(Resolved by JC2-254, see `rico-jump-air-context.md`; remaining detail there.) Hand-over and decay in the Havok context.** That the Jumping state follows the start clip's vertical root motion and InAir then
   keeps it (section 7.2) is inferred from the structure of `0x009c1f90`, `0x009bfdc0`, `0x00994c40`, `0x009acd10` and the input record
   layout of `0x0098f960` (+0x10 desired, +0x20 up, +0xa0 current, +0x88 gravity scalar); the record fields, the support check, the
   effect of the InAir gain 0.05 on horizontal speed, terminal velocity and the handler of the `jump_start` event are not decoded. Blocks
   exact jump height/arc and fall speed parity.
2. **Root-motion producer** (`(char+0x208 object)+0x1c..0x24`, JC2-239): that clip speed x time scale x weight passes through unscaled (no
   character or world scale) is inferred from `0x00895f80` and the 30 Hz clip data; the speeds above assume it.
3. **Native tick of the on-foot controller**: the stick filter and the blend counters assume one controller update per frame; the
   frame/fixed-step rate that drives `0x00821a90` is not traced.
4. **Parachute turn rate** (`S+0x170` block `+0x1c`, `0x00849700`) and the predicate `0x00801590` of `0x00675110`.
5. **Strafe angle blend** (`S+0xc`, ten-clip weights in `0x007ea9a0`) and the `CStartBlendState` (`S_UPRIGHT_START`, no clips)
   acceleration from standstill.
6. **Crouch, swim, ladder, cover and vehicle-exit speeds** use the same root-motion rule but their states were not decoded here.
7. **Role of the five 35-dword blocks of `COnFootController`**, the constructor fields `+0x38..+0x4c`, the other `settings/player.bin` values
   read by `0x00664440` (keys `0xebbb58..0xebbbc4`, `0xebbc10..0xebbc20`) and the writers of the accumulator S `+0xcc..+0xd4`.
