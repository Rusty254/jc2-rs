# Parachute: the velocity of the freefall and chute states (JC2-261, board 9a361f01, gap G11)

Closes Gap 2 of `parachute-canopy-landing-next.md` and row G11 of `parachute-parity.md`. Research only; the paired implementation is on the board
(see section 7). Probe: `research/tools/parachute_state_velocity_probe.py --game <game folder> --tool <jc2tool>` (46 checks, exit 0; the clip
root-motion checks need a `jc2tool` from JC2-220 or later, which prints `RM` lines). Executable fingerprint: `JustCause2.exe` sha256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Labels: **native** = read from code/data and asserted by the probe;
**inferred** = reasoned from native facts; **unknown** = Gaps.

## 1. Answers

* **The state classes write no velocity (native).** `CSkyDiveState` (vtable `0x00eb683c`) update `0x00853f70` and `CParachuteState` (`0x00eb688c`)
  update `0x00713d60` only compute the animation blend weights (no call to the lift writer `0x00867fa0`, the matrix setter `0x008812b0`
  or the context). `CVelocityControlledState` (`0x00eb658c`) has an **empty update** (`0x009a0d00`, `ret 0xc`). The enter/exit slots of all three only
  set / clear the character's flying flag (`+0x36d`: `0x006397c0`, `0x006a1a20`, `0x007f0a90` set; `0x006e0080`, `0x0063a7a0` clear) and the
  parachute exit also closes the canopy. So `S_FALL_TO_SKYDIVE`, `S_JUMP_TO_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_PULL_OPEN_PARACHUTE_VERTICAL`,
  `S_UNFOLD_PARACHUTE_*` set **no start velocity and no unfold motion by themselves**. What moves the body is the *controller* that serves the state
  (`foot-conditions-next.md` section 4: one controller per state) plus the clip's root motion, both through the character context.
* **The flying flag switches the airborne body to the context's Flying state (native; corrects `rico-jump-air-context.md` section 3, which said InAir never
  switches to Flying).** The InAir change slot `0x009b6090` ends, when the body is *not* supported (rec `+0x41` clear), with: flying flag (rec `+0x40`,
  character `+0x36d`) set -> set-state(4) through `0x009934e0` (`push 4; call` at `0x009b60ed`; the only four callers of `0x009934e0` are `0x009b60ed`
  InAir->Flying, `0x009c2025` OnGround->Flying, `0x009c20aa`/`0x009c20c0` ->InAir). Flying update `0x009970c0`: flag clear -> state 2, flag set ->
  **output velocity := desired velocity** (rec `+0x10`), no gravity, no gain, no clamp. Driver `0x009c48a0` runs the change slot first and the update slot
  only if the state did not change, so the switch update **coasts** (keeps the previous velocity) and the next update is the first one that outputs the desired velocity. Leaving Flying
  (flag cleared) also coasts for one update, then InAir applies gravity (`-21`) again.
* **The desired velocity is `lift + root motion`, and the lift is cleared every frame (native).** `0x00901900` (motion mode 0 of the character switch
  `0x00707720`, character `+0xa74`): `+0x270 = +0x1c0 (lift) + +0x1b4 (clip root-motion velocity, copied from the clip object +0x1c..)`, then `+0x1c0..+0x1c8 := 0`.
  Motion mode 2 (`0x008fbe10`) copies the lift only and ignores the root motion. The lift is written by `0x00867fa0` (a world vector rotated into the character
  frame by the 3x3 at `this+0x6c`); in Flying the context rotates `+0x270` back by the physics orientation (`0x0098f960`, rec `+0x10`), so for an unchanged
  orientation the body velocity is exactly the world vector the controller wrote. A controller that does not write the lift in a frame leaves the body at
  root motion only (zero for all clips below), i.e. a Flying body stops dead; the three controllers below write it every frame.
* **Freefall (`CSkyDiveController`, states `S_FALL_TO_SKYDIVE`, `S_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_PULL_OPEN_PARACHUTE_VERTICAL`, `S_TO_SKYDIVE_DASH`,
  `S_FROM_SKYDIVE_DASH`, `S_SKYDIVE_DASH`).** Update `0x00619510` runs for every one of them: open driver `0x0074edd0`, pitch pose `0x00517090`, roll pose
  `0x007687c0`, the freefall model `0x006b9290` (once per frame, one lift write of the *new* world velocity at `0x006b9bb7`), contact check. The model
  (`parachute-canopy-landing-next.md` section 7: `v += (F/90 + (0,-9.82,0)) dt`, cap `S+0x150`) therefore flies the body **from the first frame of
  `S_FALL_TO_SKYDIVE`**, not from `S_SKYDIVE`; the 0.7 s lead-in (`falling_to_freefall`, 0.667 s), the cut-away lead-in (`detatch_chute`, 1.4 s) and the
  2.433 s pull-open all use the same law. Its input is the body velocity of the previous update (`0x008a0bd0`); nothing is added at entry, so the "start
  speed of the skydive" is **the velocity the previous state left**. First-frame details (native): the start routine `0x0065c9a0` zeroes the vertical cap
  `S+0x150` only (movss, not a 16-byte store) and calls the flight-data reset `0x0077f200` (P `S+0x148` := 0.5, R `S+0x14c` := 0), `0x0056b4b0(0)`; with the cap
  at 0 a body still rising at that moment is clamped to `vy = 0` on the first frame. Hand fixture (probe): a level body falling at 10 m/s accelerates at
  `(0, -8.93, -0.43)` m/s^2 in the lead-in, not at the project's ballistic `-21`.
* **Base jump (`CBaseJumpController`, state `S_JUMP_TO_SKYDIVE`, clip `run_jump_to_freefall`, 0.8 s).** Update `0x00756ff0` (lift only, motion mode := 2, so the clip's
  root motion of 3.39 m/s is **ignored**): `speed = c34 + (12.0 - c34) * w`, `lift = forward * speed`, **no gravity** (Flying). `c34` = speed
  `|v|` at the controller start (`0x007fa850`, `0x008a0bd0` + sqrt, stored at `+0x34`), `12.0` = `f32[0x00e3aa6c]` stored at `+0x38` by the constructor `0x005952e0`, `w` =
  progress of the main layer's first clip instance (`0x008c6e70` -> `0x008c9e10`: elapsed `+0x28` over duration `end - start`; 1.0 if there is no instance),
  forward = minus the character matrix row R2 (`+0x17c..+0x184`; the code multiplies R2 by `-0.0 - speed`). On the clip's end the state goes to `S_SKYDIVE`; the freefall
  model then starts from `v = forward * ~12`. The start also reads a per-character record flag (`record+0x64` -> `S+0x2ac`); when it equals 1 the freefall
  reset (`0x0056b4b0(0)`, `0x0077f200`) runs too (meaning of the flag unknown, Gap 3).
* **Chute (`CParachuteController`, states `S_UNFOLD_PARACHUTE_VERTICAL`, `S_UNFOLD_PARACHUTE_HORIZONTAL`, `S_PARACHUTE`).** Update `0x005a9950` (one lift write at
  `0x005ab455`). Its start `0x0054fc90` only builds the camera-position object (no velocity). `S_PULL_OPEN_PARACHUTE_VERTICAL` belongs to the **freefall** controller, so
  the body keeps flying the freefall model for the whole pull-open state; the chute model, including the one-time **opening transform** (40 % of the
  capped fall speed becomes forward speed, `vy = 0.6 vc + (vy - vc)`, `foot-controller-next.md` section 8) runs **for the first time at the first
  frame of `S_UNFOLD_PARACHUTE_*`**, not on the key press. The pull itself (`0x0074edd0`, `ACT_PULLCHORD` accepted) only opens the canopy object and sets the flare
  offset `D+0x28 = -0.825` (`f32[0x01077980]`). Nothing in the state classes adds an unfold motion.
* **Clip root motion (native, `jc2tool anim`).** None in `falling_to_freefall`, `detatch_chute`, `rico_open_chute` (both pull-open and unfold vertical), `mhfreefall`,
  `freefall_trn_lft/rgt`, `birdsuit_fwd`, `parachute_dash`, `parachute_spin`, the four `rico_speed/turn_TB_*` canopy clips. Only `rico_reel_open`
  (`S_UNFOLD_PARACHUTE_HORIZONTAL`, 2.0 s) has a displacement: 0.2083 m along +z in the clip frame (0.104 m/s, sign convention inferred); the motion mode of that controller is
  not decoded (Gap 2), so whether it is added is open and at most 0.1 m/s.

## 2. Per-state rule

| State (class, controller) | Clip (length, root motion) | Body velocity while it plays |
|---|---|---|
| `S_FALL_TO_SKYDIVE` (CSkyDiveState, SkyDive) | `falling_to_freefall` (0.667 s, none) | freefall model every frame from the velocity of `S_FALL`/`S_JUMP_*`; one coast update at the InAir->Flying switch; P=0.5, R=0, cap=0 at entry |
| `S_JUMP_TO_SKYDIVE` (CVelocityControlledState, BaseJump) | `run_jump_to_freefall` (0.8 s, 3.39 m/s ignored) | `forward * (c34 + (12 - c34) * w)`, lift only, no gravity |
| `S_SKYDIVE` (CSkyDiveState, SkyDive) | 4-clip blend (none) | freefall model |
| `S_PARACHUTE_DETACH_TO_FREEFALL` (CSkyDiveState, SkyDive) | `detatch_chute` (1.4 s, none) | freefall model starting from the chute's last velocity; controller start resets P/R/cap (inferred from slot 10 = start) |
| `S_PULL_OPEN_PARACHUTE_VERTICAL` (CVelocityControlledState, SkyDive) | `rico_open_chute` (2.433 s, none), leaves at 0.463 s (margin 1.97) | freefall model for 0.463 s; canopy object opens meanwhile |
| `S_UNFOLD_PARACHUTE_VERTICAL` (CVelocityControlledState, Parachute) | `rico_open_chute` continued from clip time 0.463 s, leaves at 1.633 s (margin 0.8) | chute model; first frame applies the opening transform |
| `S_UNFOLD_PARACHUTE_HORIZONTAL` (CVelocityControlledState, Parachute) | `rico_reel_open` (2.0 s, 0.104 m/s) | chute model (opening transform on its first frame); root motion mode unknown |
| `S_PARACHUTE` (CParachuteState, Parachute) | 4-clip blend (none) | chute model |
| `S_TO/FROM_SKYDIVE_DASH`, `S_SKYDIVE_DASH` (CVelocityControlledState, SkyDive) | `parachute_dash`, `parachute_spin` (none) | freefall model (the dash action's own velocity, if any, is not decoded) |

The vertical-pull and unfold states share the clip `rico_open_chute` and both leave on `C_WAIT_FOR_ANIM_TO_FINISH` (a `CAnimationFinishCondition` on the first
instance of the first body part, `animation-next.md`). **Correction (board caf7b21c, `parachute-deploy-duration.md`):** the finish is at clip duration minus a per-state margin (asb key
`0x96270a00`: 1.97 s for the pull-open, 0.8 s for the unfold) and the unfold continues the clip at the phase the pull-open reached, so the pull-open lasts 0.463 s, the unfold ends at clip
time 1.633 s and the glide starts **1.633 s** after the press (not two clip lengths = 4.9 s). The canopy object's own opening clip is one length (2.433 s, `parachute-deploy-collapse.md`).

## 3. Differences to the runtime (`crates/jc2-game`)

1. `character.rs` flies the freefall model only while `d.state == "S_SKYDIVE"` (`hero.skydiving`), and the lead-in states keep the ballistic fall.
   Native: every state of the SkyDive controller list uses the model, including `S_FALL_TO_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`,
   `S_PULL_OPEN_PARACHUTE_VERTICAL` and the dash states.
2. The chute model and `chute_open_velocity` start at the key press (`hero.parachute = true` and the transform in the same frame). Native: at the first frame of the
   `S_UNFOLD_PARACHUTE_*` state (after the pull-open clip). The canopy shown from the press (`canopy_up`) is already native (close rule (a) and the opening object).
3. `S_JUMP_TO_SKYDIVE` has no model in the runtime; it needs the base-jump law above.
4. A body that is entered into the model with the previous states' velocity must keep that velocity (no zeroing, no 5.5 m/s kill).

**Implemented (board 4d7f7642):** items 1 to 3 are in `character.rs` / `parachute_flight.rs`: the freefall model flies every SkyDive-controller state;
the canopy model, `CanopyFlight::open_from_matrix` and `chute_open_velocity` run on the first tick of a Parachute-controller state (`Hero::canopy_live`);
`S_JUMP_TO_SKYDIVE` uses `base_jump_speed` (clip progress taken from the state clock: inferred); the deployment `C_WAIT_FOR_ANIM_TO_FINISH` margins
(1.97 / 0.8 / 1.2 / 0.4 s) are applied; a cut-away starts the freefall model from the canopy's matrix (pitch and bank carried; inferred from the start routine
keeping the character matrix). Remaining: no foot-machine path reaches `S_JUMP_TO_SKYDIVE` is unverified in the trace tests (unit-tested law only).

## 4. Runtime guidance (task in section 7)

* Controller mapping per state exactly as in section 2; `Skydive::start` is called when the active controller changes **to** the SkyDive controller (also from the chute:
  `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_FALL_TO_SKYDIVE`), not at `S_SKYDIVE`. Reset P=0.5, R=0, cap=0 there.
* The model reads the body velocity, writes the new velocity; no extra gravity term (`v.y += 9.81 dt` in the runtime cancels the engine's own step, keep that bookkeeping).
* Base jump: `speed = v0 + (12 - v0) * (state_time / 0.8)`, direction = the facing, no gravity, root motion ignored; `v0` = horizontal-and-vertical speed at entry.
* Chute: run `CanopyFlight::open` + `chute_open_velocity` on the first frame of `S_UNFOLD_PARACHUTE_VERTICAL/HORIZONTAL`; during `S_PULL_OPEN_PARACHUTE_VERTICAL` keep the freefall
  model and show the opening canopy.
* One coast update at the InAir -> Flying switch can be ignored below the project's tick (it is one update of unchanged velocity); do not apply gravity in the update after the switch.

## 5. Evidence

Probe checks (46): function hashes of 24 cited functions; RTTI names and slots of the three state vtables; empty update `0x009a0d00`; flag stores; no lift/matrix writer in the animated
updates; InAir change-slot bytes and the four callers of `0x009934e0`; Flying update bytes; driver order; `0x00901900` and `0x008fbe10` bodies; mode switch calls; `0x00867fa0` store; controller
vtable slots (SkyDive `0xec2c20`, Parachute `0xec24ac`, BaseJump `0xec2c5c`); one lift write in each of the three models; skydive start / reset / chute start bytes; open driver; base-jump update, ctor,
start and progress bytes; models (base-jump speed lerp, progress, context sequences incl. the coast, lift merge, lead-in hand fixture, rising-body clamp, opening transform); state classes and clips
(`basestatemachine.afsm`, `kc001_base.asb`); clip root motion (`falling_to_freefall` none, `rico_reel_open` (-0.0006, 0, 0.2083) over 2.0 s, `run_jump_to_freefall` (0.005, 0.859, -2.713) over 0.8 s,
`heli_default_to_freefall` (-0.002, -2.854, -3.453) over 0.433 s).

Corrections to earlier documents: `rico-jump-air-context.md` section 3 (Flying *is* reachable from InAir when the flying flag is set); `parachute-canopy-landing-next.md` section 7 / probe
(the skydive start zeroes `S+0x150` only; the model flies from `S_FALL_TO_SKYDIVE`, not from `S_SKYDIVE`).

## 7. Board

Implementation: 4d7f7642 (ready, high, tag `parachute`; owns `character.rs` velocity step and chute open block, `parachute_flight.rs` Skydive start and the base-jump law).
Research gaps filed: b0331056 (Gap 1, orientation round trip of the lift), caf7b21c (Gap 5, deployment state durations). Gaps 2 to 4 and 6 stay listed here (low or out of scope).

## Gaps (unresolved native details)

1. **Orientation round trip of the lift (`0x00867fa0`, `0x0098f960`).** The lift is rotated into the character frame with the 3x3 at `this+0x6c` and rotated back with the physics quaternion
   (`CPfx+0x30`). Whether both are the same orientation in a frame in which the freefall model has just turned the body (`0x008812b0`) decides whether the velocity direction turns with the body's
   incremental rotation. Unknown: which matrix is at `this+0x6c`, and the order against the matrix update. Blocks exact freefall turning (visible in banked dives); the runtime keeps the world vector.
2. **Motion mode (`character+0xa74`) per controller.** The base-jump update sets mode 2 every frame; the skydive and chute models set none, and the writers `0x0061e29d`, `0x00743ff1`, `0x0078565f`,
   `0x00815295` (mode 0) were not tied to controller changes. Matters only for `rico_reel_open` (<= 0.104 m/s) and for whether mode 2 persists after a base jump. Low impact.
3. **Base-jump record flag.** `0x007fa850` copies a per-character record value (`0x005cf980` record `+0x64`) to `S+0x2ac`; the value 1 triggers the freefall reset. The record's source (player data) was not
   decoded; for Rico it is unknown whether it is 1. Low impact (reset of P/R/cap at the start of a base jump).
4. **Controller start trigger.** That vtable slot 10 of each controller runs on every activation (resetting P/R/cap for `S_PARACHUTE_DETACH_TO_FREEFALL`) is inferred from the SkyDive/BaseJump pair; the
   activation call was not found. Blocks nothing visible.
5. **Duration of the two deployment states.** RESOLVED by board caf7b21c (`parachute-deploy-duration.md`, probe `parachute_deploy_duration_probe.py`, 28 checks): pull-open 0.463 s, unfold to clip
   time 1.633 s, glide 1.633 s after the press (margin keys and phase hand-over, native). Residue (instance mode source, tick order, canopy finish evaluator) is listed there.
6. **`S_BELLY_TO_SKYDIVE`** (`heli_default_to_freefall`, 7.97 m/s root motion) and the dash states: their controller / motion mode were not decoded (out of the parachute scope).
