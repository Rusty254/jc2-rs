# Parachute: hook assist, landing/close rules, canopy basis, neutral glide; skydive model; canopy animation blend (JC2-243, JC2-244, JC2-262)

Research only. Probes: `research/tools/parachute_landing_probe.py` (50 checks, exit 0) and, for section 10 (canopy animation blend,
steering inputs, JC2-262; section 11 = Gap 8, the engine side of the blend), `research/tools/parachute_blend_probe.py` (97 checks, exit 0, same arguments):

```
python research/tools/parachute_landing_probe.py --game <game folder> [--tool <jc2tool.exe>] [--report]
```

Executable fingerprint: `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
Addresses are virtual addresses. **native** = read from code/data and asserted by the probe; **inferred** = reasoned from
native facts (always said so); **unknown** = listed under Gaps. Builds on `foot-controller-next.md` (JC2-170, sections 4, 8, 9),
`animation-statemachines-next.md` and `grapple-action-selection-next.md`. Gaps 4, 6 (JC2-243) and 7 (JC2-244) of
`foot-controller-next.md` are closed here except where listed under Gaps.

## 1. Answers in one page

* The "vehicle landing assist" of `0x005a9950` is a **grapple-hook assist**, not a vehicle feature. Its target is the
  grappling device (`CGrapplingDevice`, holder `character+0x484`, `holder+0xd0`); `device+0x128 != 0` means a wire is attached;
  the aim point is the world position of the wire end record `count-2` (`0x00678000` -> `0x005eb3f0`). Gain `device+0xf8` = **50.0**
  and speed limit `device+0xfc` = **40.0** come from `settings/grapplingparams.bin` (keys `0x320b8cd2`, `0x7a4ef581`;
  loader `0x00802ec0`). `parachuteparams.bin` contains **no numbers** (model `weapons\gae05\GAE05_LOD1-GAE05`, `parachute.bsk`,
  `parachute.asb`, `parachute.afsm`, `ParachuteEffects.bin`, a pickup name): every flight constant is in code.
* **Landing is decided by the animation graph**, using the character body velocity: nice landing needs horizontal speed < 8.1
  and vertical speed > -8.0; roll when horizontal > 8.0; hard when vertical < -8.1 (section 3).
* **Cutting the chute** (PARACHUTE_OPEN_CLOSE pressed in flight) dispatches `ACT_PULLCHORD`; the graph picks the destination with
  forward ray fans: ground within 30 m -> `S_FALL`; nothing within 60 m -> `S_PARACHUTE_DETACH_TO_FREEFALL`; otherwise
  `S_FALL_TO_SKYDIVE` (section 4).
* **Canopy basis (native, from the matrix builders):** `M = Rz(a38) * Rx(a30) * Ry(a34)` with row-vector matrices; rows are `R0` side,
  `R1` wing normal, `R2` back; forward `A = -R2`; **positive a30 = nose up** (`A.y = sin a30`); the bind pose in `parachute.bsk`
  agrees (section 5).
* **Neutral glide (a30 = 0, no wind): 9.60 m/s forward, 1.69 m/s sink.** Full back input (a30 = +0.65): 7.24 forward / 9.52 sink;
  full forward (a30 = -0.65): 45.1 forward / 33.0 sink. The yaw rate at full side input settles at 0.75/0.95 = 0.789 rad/s
  (0.796 rad/s at 60 fps, discrete) with a bank of 0.65 rad (section 5).
* **Skydive model, corrected roles (section 7):** `Cb(P)` (0.6 at P=0, 0.675 at .25, 1.15 at .5, 1.0 at 1) multiplies the drag direction
  `W = -unit(v)`; `Ca(P)` (0.5, 0.575, 0.55, 0.4 at the same P) multiplies a direction perpendicular to the airflow `L`. Terminal velocities for P = 0, .25, .5, .75, 1:
  (47.4 h / -56.9), (31.7 / -37.3), (13.6 / -28.5), (8.7 / -25.5), (8.8 / -21.9) m/s.

## 2. Hook assist (native, `0x005a9950`)

`ebx` = device = `holder+0xd0` (`0x005aa4d4`). Input id `0x51` (FIRE_GRAPPLE): state PRESSED or HELD sets a local flag. Let
`assist = device != 0 && !flag && device+0x128 != 0` (`0x005aa4f0-0x005aa504`).

When `assist`:

* `0x0073dfe0(0.2)` on the device (sets `device+0x2e8 = 1` and `+0x2ec = +0x2f0 = 0.2` once; role unknown).
* the hooked object (weak ref from `0x0071fce0`) selects what happens next: a physics object whose virtual `+0x48` returns kind 1
  calls `0x00740260`; anything else (terrain/static, other kinds) starts effect index 2 of the device effect array
  (`0x00735700(device+0x218, 2)`) and sets controller `+0x14c = 1` (the first time, for the player a debug-log call). Gap.
* `d = unit(hook - pos)`, `dist = |hook - pos|`. **Thrust replaces `A*m`:** `Ft = d * 50.0 * dist` (the free-flight thrust of
  section 8 of `foot-controller-next.md` is overwritten at `0x005aa6e1`). The thruster item (if its state is 1) still adds
  `-R2 * thrustValue` afterwards (`0x005aa8c3-0x005aa951`).
* **pitch term:** `t = clamp(1 + d.y, 0, 1)` (up vector static `0x011fcf08` = (0,1,0)); the pitch spring becomes
  `e30 = (t - pitchIn*sx) * 0.65 - a30` (free flight uses `t = 0`). A hook straight below gives `t = 0`, a hook at eye level `t = 1`.
* **turn:** `turn = 1.4 * wrapPi(atan2(-d.x, -d.z) - heading)`, `heading = atan2(R2.x, R2.z)` (`0x010776dc` = 1.4). No rate limit
  (free flight approaches `0.46 * S+4` at 0.7 units/s, or 0).
* **speed clamp** (`0x005aaa4e-0x005aaafd`): `v = unit(v) * min(|v|, 40.0)`. It is gated only by `device != 0 && device+0x128 != 0`,
  **not** by FIRE_GRAPPLE (correcting `foot-controller-next.md` section 8).

When not `assist` and a device exists, effect 2 is stopped (`0x005e9670`) and `controller+0x14c = 0`.

Fixture (probe): char (0,100,0), hook (30,60,40): `dist = 64.031`, `d = (0.4685, -0.6247, 0.6247)`, `|Ft| = 3201.56`, `t = 0.3753`.

## 3. Landing (native)

Per frame, after the spring and transform write (`0x005ab56f-0x005ab5db`): if `character+0x590` bit 8 is clear **and**
`0x0088b250` (body contact state field `+0x40` of the object at `character+0xf0` is 1 or 2, the same predicate as
`C_GROUND_CONTACT`, evaluator `0x007f42a0`) holds, the model dispatches `ACT_LAND` (`0x011967c8`) through vtable `+0x74`.
If accepted and the new layer-0 state is `S_HITREACT_UNCONTROLLED_FLIGHT` (`0x01196ad8`) it also runs `0x007ace60`
(spine effect `0x15f` when `character+0xa64 > 0.5`, then x0.25) and `0x007513d0(2)`; in every accepted case it then cancels the
grapple device (`0x007504b0` -> `0x005fbcb0`, only when a wire exists), dispatches `ACT_WAIT` on the upper layers
(`0x0073a670(0)`, objects `character+0x494/+0x49c`) and clears vector fields (`0x008e6450`).

Acceptance is the graph. `S_PARACHUTE` and `S_UNFOLD_PARACHUTE_HORIZONTAL`, `ACT_LAND` edges in ordinal order (first true wins;
evaluators `0x007f2960` horizontal speed = `sqrt(vx^2+vz^2)` and `0x00725020` vertical speed = `vy` of the character body; polarity
0 means value < limit, 1 means value > limit, both strict):

| edge | destination | conditions |
|---|---|---|
| `T_PARACHUTE_LAND_NICE` | `S_PARACHUTE_LANDING` (-> `S_UPRIGHT_IDLE` on clip finish) | horizontal < 8.1 and vertical > -8.0 |
| `T_PARACHUTE_LAND_FAST` | `S_LANDING_ROLL` | horizontal > 8.0 |
| `T_PARACHUTE_LAND_HARD` | `S_LANDING_HARD` | vertical < -8.1 |

No edge matches for horizontal <= 8.0 with -8.1 <= vertical <= -8.0 (ACT_LAND is then not accepted and is re-sent next frame).
`S_UNFOLD_PARACHUTE_VERTICAL` and `S_SKYDIVE` map `ACT_LAND` unconditionally to `S_HITREACT_UNCONTROLLED_FLIGHT` (the crash).

**Close rules (native).** (a) `0x005ab65e`: if `0x0076fc30` (layer-0 state is `S_UNFOLD_PARACHUTE_HORIZONTAL/VERTICAL` or
`S_PARACHUTE`) is false, the flight model closes the canopy (vtable `+0x1c`), clears the device effect and `controller+0x14c`.
(b) canopy update `0x006ccd40`: if canopy `+0x4a` (ctor default 1, `0x007e0250`) and the body is in contact (`0x0088b250`) and
the state is none of `S_UNFOLD_PARACHUTE_VERTICAL`, `S_UNFOLD_PARACHUTE_HORIZONTAL`, `S_SKYDIVE_PULLCHORD`,
`S_PULL_OPEN_PARACHUTE_VERTICAL`, it closes. (c) the player cut below.

## 4. Cutting the chute (native)

`0x005ab546`: PARACHUTE_OPEN_CLOSE (id `0x47`) PRESSED dispatches `ACT_PULLCHORD` (`0x01196ed0`) on `character+0xf0` (vtable `+0x74`).
`S_PARACHUTE`, `S_UNFOLD_PARACHUTE_HORIZONTAL/VERTICAL` handle it with three edges in this order (conditions are
`CForwardRayCastFanCondition`, evaluator `0x00799600`):

* fan geometry: origin = body position + (0, **0.25**, 0) (`0x01077c8c`); `n = 10` rays (`ba4ef577`); ray i = `unit(lerp(planar forward,
  (0,-1,0), i/(n-1)))` times the length (`c40dcfd2`), planar forward = `unit(-(R2.x, 0, R2.z))`; from horizontal to straight down;
* result: any hit -> the polarity byte (`ca14dc32`), no hit -> `polarity == 0` (`0x0079994a` `sete`, `0x0079995b`).

| edge | destination | length | polarity | true when |
|---|---|---|---|---|
| `T_PARACHUTE_TO_FALL` | `S_FALL` | 30 | 1 | a ray hits within 30 m |
| `T_PARACHUTE_TO_SKYDIVE_FANCY` | `S_PARACHUTE_DETACH_TO_FREEFALL` (CSkyDiveState, then `S_SKYDIVE`) | 60 | 0 | nothing within 60 m |
| `T_PARACHUTE_TO_SKYDIVE_SIMPLE` | `S_FALL_TO_SKYDIVE` | 60 | 1 | something within 60 m (but nothing within 30) |

The three are exhaustive. (`ACT_PARAGLIDE` is sent every frame by `0x0074edd0` while the canopy is open; no edge of `basestatemachine.afsm` accepts it: harmless.) The open side (`0x0074edd0`) is unchanged from `foot-controller-next.md` section 4.

## 5. Canopy basis and neutral glide (native)

Builders (decompiled element assignments, verified by function hashes): `Rx(a)` (`0x004dbce0`): rows (1,0,0),(0,cos,sin),(0,-sin,cos);
`Ry(a)` (`0x004eaad0`): (cos,0,-sin),(0,1,0),(sin,0,cos); `Rz(a)` (`0x004f1790`): (cos,sin,0),(-sin,cos,0),(0,0,1). `0x00491e50(p1,p2,out)` is
`out = p1 * p2` (row-vector). The flight model builds `T = Rz(a38) * Rx(a30)`, then `M = T * Ry(a34)` (`0x005ab3f2-0x005ab421`).
Consequences: `R2 = (0,-sin a30, cos a30) * Ry(a34)`, so `atan2(R2.x, R2.z) = a34` exactly (the `heading - a34` term of the yaw
spring is always zero: `e34 = 0.75 * (turn - rollIn*sy)`), and `A = -R2` has `A.y = sin a30`. Pitch **input** is
`FORWARD - BACKWARD` and the target is `-0.65 * pitchIn`: forward = nose down, back = nose up/brake.

Bind pose (`jc2tool anim parachute.bsk`, reference pose): `base` is 1.227 m above the root (+Y), right wing bones go to +X and
left to -X, `Para_RightHandle` is at (0.306, 0.284, **-0.351**) relative to `base`, i.e. in front of the rider (-Z), the same
convention as the character (`facing = -(R2.x, R2.z)`). The skeleton frame is therefore X side, Y up/normal, Z back, matching
`R0/R1/R2`. The mesh itself (`weapons\gae05\GAE05_LOD1-GAE05`) is not in the mounted archives (Gap 5).

Fixed points of the integrated force law (probe, 60 fps and 30 fps agree; `s2 = |v + 12.25 A|^2`, so lift and drag exist at rest):

| a30 (rad) | forward (m/s) | sink (m/s) |
|---|---|---|
| 0 (neutral) | 9.60 | 1.69 |
| +0.65 (back/brake) | 7.24 | 9.52 |
| -0.65 (forward/dive) | 45.1 | 33.0 |

The pitch angle itself follows the damped spring of `foot-controller-next.md` section 8 to `-0.65 * pitchIn`; bank target
`-0.65 * rollIn`; yaw rate continuous limit `0.75*(turn - rollIn)/0.95`.

## 6. Thruster object (native fields, unresolved source)

`canopy+0x6c` is the thruster object (a plain struct; no vtable). Update `0x005bcbf0(openFlag, ..., dt)` is called from the canopy
update every frame when it exists. Fields: `+0x08, +0x10` effect handles; `+0x18` state (0 idle, 1 thrusting); `+0x1c` thrust value
(added to the flight thrust only in state 1: `0x00757380`); `+0x20` cooldown length; `+0x24` cooldown timer (counts down while idle;
while > 0 the charge does not recharge); `+0x28` charge; `+0x2c` start threshold; `+0x30` maximum charge; `+0x34` drain rate;
`+0x38` recharge rate; `+0x3c..+0x44` steering scale vector (default 1,1,1 for the flight model). Idle: `charge += recharge * dt`
clamped to [0, max]; ACTIVATE_PARACHUTE_THRUSTERS (id 152) PRESSED or HELD while the canopy is open and `charge > start` switches to
state 1 (effects started). Thrusting: `charge -= drain * dt`; at `charge <= 0` or when the button is released the state returns to 0,
effects stop and `+0x24 = +0x20`. Creation site, data keys and defaults are resolved in `parachute-thruster-item.md` (JC2-260): the object exists only
when the parachute entry of a DLC `N_parachute.bin` has a thruster group; the retail `14_parachute.bin` has none, so the standard canopy has no thruster.

## 7. Skydive (freefall) model `0x006b9290` (native; supersedes section 9 of `foot-controller-next.md`)

Per frame (`0x00619510` calls the open/close driver `0x0074edd0`, the pitch pose `0x00517090`, the roll pose `0x007687c0`, the model, then
the contact check). `S` = `character+0xa34`. Poses: `P = S+0x148` (pitch) and `R = S+0x14c` (roll) as in section 9 of the foot doc.

**Forces.** `v` = body velocity (if `|v|^2 == 0`, `v = 0.1 * R2` of the character). `W = -unit(v)`. `s2 = |v|^2`.
`area = (0.8 + 1.1P) * (0.4 + 0.8P)`, `q = area * s2 * 0.645` ("Air influence"). `L`: if `(R0.W)^2 <= (R2.W)^2` then `u = unit(R0 x W)`,
else `u = unit((W x R2) x W)` (the part of `R2` perpendicular to `W`); `L = -u`. Then

* `F = Ca(P) * q * L + Cb(P) * q * W`; `a = F/90 + (0, -9.82, 0)`; `v += a * dt`;
* `Ca`: P <= 0.25: `0.5 + 0.3P`; 0.25 < P <= 0.5: `0.8 - 0.5P`; P > 0.5: `0.3 + 0.1P` (statics `0x01077984..90`);
* `Cb` (the **drag** along `-v`): P <= 0.25: `0.6 + 0.3P`; <= 0.5: `0.9 + 0.5P`; > 0.5: `1.4 - 0.4P` (`0x01077994..a0`).

(The earlier doc had the two roles swapped: the 0.6..1.4 curve multiplies `W`.)

**Vertical cap `S+0x150`** (`0x006b9b2b-0x006b9ba6`): the start routine `0x0065c9a0` zeroes `S+0x150..+0x15c` and, for the player, stores the start
altitude in `player+0x98`. Each frame: `if vy < cap*2.5 then cap = 0.4*vy` (a new fastest fall); `if vy >= cap then vy = cap`. So any upward velocity is
removed at once and the vertical speed can never fall below 40 % of the fastest fall speed so far. It never binds at the terminal speeds below.

**Terminal velocities** (equilibrium pitch below; independent of the start velocity and of the frame rate):

| P | horizontal | vertical | body pitch sin a |
|---|---|---|---|
| 0 (FORWARD) | 47.40 | -56.88 | -0.833 (head down) |
| 0.25 | 31.74 | -37.26 | -0.417 |
| 0.5 (neutral) | 13.61 | -28.46 | 0 |
| 0.75 | 8.69 | -25.50 | +0.417 |
| 1 (BACKWARD) | 8.77 | -21.93 | +0.833 (flare) |

**Orientation** (`0x006b9ef8-0x006ba139`): `C` = character matrix (rows R0 side, R1, R2 back; forward = -R2). `M = Rx(th_p) * Rz(th_r) * (C * Ry(th_y))`
with `th_y = clamp(R + aim, +-2.1991148) * dt` (world yaw), `th_r = (R - 1.5*R0.y) * dt` (local bank, self-levelling), `th_p = ((P - 0.5)*5 + 3*R2.y) * dt` (local pitch);
position kept; written with `0x008812b0(matrix, 1)`. Pitch settles where `R2.y = -(5/3)(P - 0.5)`. `aim` = signed angle between the character's planar `R2`
and the direction from the device aim point (`0x006934b0`) to the character, only while the left-arm layer is in an `LA_S_GRAPPLE*`/`LA_S_*HOOK*` state (`0x0071bf60`:
`LA_S_GRAPPLE_PULL, LA_S_GRAPPLE, LA_S_GRAPPLE_TO_IDLE, LA_S_GRAPPLE_DETACH, LA_S_RAISE_GRAPPLE, LA_S_DUAL_GRAPPLE, LA_S_WAITING_FOR_HOOK, LA_S_HOOK_MISSED,
LA_S_HOOK_HIT, LA_S_RAISE_GRAPPLE_SO`); otherwise 0, and when the player setting byte (`+0x114e`) is set `R += clamp(S+4, +-0.05)` each frame (`S+4`
= signed yaw angle from the locomotion code, inferred).

**Entry and exit (native graph, `basestatemachine.afsm`):**

* into `S_FALL_TO_SKYDIVE` (edge `T_FALL_TO_SKYDIVE`, `ACT_UPDATE`) from `S_FALL`, `S_JUMP_FLIGHT_STANDING`, `S_JUMP_FLIGHT_SPRINT_FORWARD`,
  `S_JUMP_TO_FALL_TRANSITION`, `S_JUMP_FROM_SO_START`, `S_REELED_RELEASE_TO_FALL`, `S_ROLL_OUT_LEFT`, `S_EXITING_VEHICLE_INTERUPT` when
  `C_CAN_FALL_TO_SKYDIVE` (`CRayCastFanCondition` evaluator `0x00717c60`: 5 rays, 30 m, fan from -110 to -180 degrees in the yaw-only character frame:
  direction `(0, cos a, sin a)` with local -Z forward, i.e. from 20 degrees below forward to straight down; origin offset (0,1.5,0); polarity 0, so true
  only when **nothing** is hit) and `C_ENOUGH_ABOVE_WATER_TO_SKYDIVE` (`CAtSeaLevelCondition` `0x0059a1e0`: height of bone 0 over the water > 28.5 m);
* into `S_JUMP_TO_SKYDIVE` from `S_JUMP_START_SPRINT_FORWARD`/`S_JUMP_FROM_SO_START` (on clip finish) and `S_JUMP_TO_FALL_TRANSITION`, with `C_CAN_JUMP_TO_SKYDIVE`
  (5 rays, 30 m, -100 to -135 degrees, origin (0,1.5,-0.5)), `C_CAN_JUMP_CLEAR_AHEAD` (5 rays, 3.5 m, -80 to -180, origin (0,1,0)) and the water test;
* `ACT_SKYDIVE` -> `S_SKYDIVE` from `S_UPRIGHT_IDLE` and `S_EXIT_STUNTPOSITION`; `S_FALL_TO_SKYDIVE`, `S_JUMP_TO_SKYDIVE`, `S_BELLY_TO_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`,
  `S_FROM_SKYDIVE_DASH` go to `S_SKYDIVE` on clip finish; `S_FALL_TO_SKYDIVE` exits to `S_JUMP_LAND_HEAVY` when grounded and heavy;
* chute: `ACT_PULLCHORD` -> `S_PULL_OPEN_PARACHUTE_VERTICAL` (no condition) -> `S_UNFOLD_PARACHUTE_VERTICAL` -> `S_PARACHUTE`; the open driver needs `ACT_PULLCHORD` accepted;
* crash: `ACT_LAND` (sent every frame on contact by `0x00619510`) -> `S_HITREACT_UNCONTROLLED_FLIGHT`; water: `T_AIRBORNE_TO_SWIMMING/DIVING` (after the land edge).
* start velocity: the controller writes **none**; the body keeps the velocity it had; the state classes write none either (JC2-261, `parachute-state-velocities.md`); the model flies every state of the SkyDive controller list, from `S_FALL_TO_SKYDIVE` on.

## 8. Runtime guidance (JC2-238)

* Parachute: keep `foot-controller-next.md` section 8, with the hook assist (section 2 here), the matrix order `Rz*Rx*Ry` and the corrected clamp gate.
  Do not invent a neutral glide: the law above yields 9.60 / 1.69 by itself; do not clamp the sink.
* Landing: dispatch `ACT_LAND` every grounded frame and let the graph classify with the 8.1/8.0/-8.0/-8.1 thresholds on body velocity.
* Cut: send `ACT_PULLCHORD` and evaluate the two forward fans (30 m / 60 m) with the 10-ray vertical fan; no hard-coded "near ground" test.
* Freefall: implement `Ca`/`Cb`, the area, `/90`, the cap and the three-matrix orientation step; `P`/`R` laws as in section 9 of the foot doc.

## 9. Runtime implementation (JC2-263)

`crates/jc2-game/src/parachute_flight.rs` (pure maths, unit tests) and `character.rs`:

* **Canopy** (`CanopyFlight`): section 8 of `foot-controller-next.md` with the corrections of this brief (matrix `Rz*Rx*Ry`, forces, `Cd`/`Cl`
  tables, the pitch/yaw/bank spring, the no-backwards clamp, hook assist with the 40.0 cap). Tests assert the fixed points 9.60/1.69, 7.24/9.52,
  45.1/33.0 at 60 and 30 fps and the 0.789 rad/s turn with 0.65 rad bank. The old invented glide constants (11 m/s, 5.5 m/s sink, 1.3 rad/s turn) are gone.
  Inputs: pitch = W - S, roll = D - A (keyboard values are -1/0/1; the analog stick range is not modelled). The opening frame keeps the
  native velocity transform (`chute_open_velocity`); the Euler angles start from the yaw only (inferred, Gap 2).
* **Skydive** (`Skydive`): section 7 as written (poses `P`/`R`, area, `Ca`/`Cb`, `L`, the cap, the three-matrix orientation step). It flies the body only in
  `S_SKYDIVE` (the lead-in states keep the project's ballistic fall until Gap 2 of this brief is resolved). Terminal speeds are asserted for P = 0, 0.2, 0.3, 0.5, 0.75, 1 (the values at
  P = 0.25 exactly are not reproducible by a settling pose: both coefficient curves jump at 0.25 and at 0.5).
* **Landing**: unchanged from the foot machine: `ACT_LAND` on touchdown, thresholds read from the graph on the last airborne body velocity. With the native
  neutral glide (9.60 m/s along) the graph gives the roll landing (horizontal > 8.0), with full back the hard landing (vertical < -8.1), with the dive the roll.
* **Cut**: PARACHUTE_OPEN_CLOSE sends `ACT_PULLCHORD`; the forward fan (10 rays, planar forward to straight down, 60 m, origin = body + 0.25 m, body taken at the feet:
  inferred) answers the three edges. Test: 20 m -> `S_FALL`, 45 m -> `S_FALL_TO_SKYDIVE`, 300 m -> `S_PARACHUTE_DETACH_TO_FREEFALL`.
  The "3 m room under the feet" rule is removed; opening needs only the graph's `ACT_PULLCHORD`.
* **Hook assist** applies whenever `Hero::grapple` is set while the canopy is open and F (FIRE_GRAPPLE, id 0x51) is not pressed or held
  (`HeroInput::fire_grapple`, JC2 task 20ded2f1; the wire end is the anchor); the 40 m/s clamp is wire-gated only. The reel code in `grapple.rs` still runs after the model (see Gap 6).

## 10. Canopy animation blend and the steering inputs it shares (JC2-262, native unless marked)

Closes Gap 3. Every address below is asserted by `research/tools/parachute_blend_probe.py` (function hashes, instruction anchors,
constants, hand-derived fixtures). Naming: `S` = per-character state block (`character+0xa34`), `D` = flight data block (`S+0x170`, allocated by
the S constructor `0x005a25e0`: 0x50 bytes, all zero except `D+0xc = 1.0` and `D+0x1c = 100.0`).

### 10.1 The class and its five clips

* `CParachuteBlendState` is the state class of `PCS_OPEN` in `parachute.afsm`. Its vtable `0x00eb7264` has 18 slots; enter/exit (slots 12 and 14) are
  the no-op wrapper `0x005145a0`; **slot 17 (`0x00778c50`) is the per-frame blend update** (`ret 0xc`: this, a float dt, the clip vector). The base
  `CAnimationState` has an empty function in the same slot. The factory is `0x0050cf90` (0x20-byte object).
* The clip vector holds 8-byte shared pointers to animation instances (instance fields: time `+0x28`, weight `+0x5c`, rate `+0x60`). The update
  requires **exactly five** clips. The animation-set loader `0x00499d60` fills it in the order of the slot keys `hash("anim_file N")`
  (`0x004a73c0` builds them with `sprintf("anim_file %d")`; the probe recomputes the hashes). In `animations/animationsets/parachute.asb`, state `PC_ANIM_OPEN`:

| slot | key | file | length |
|---|---|---|---|
| 0 | `anim_file 0` (0x3538c755) | `chute_turn_TB_UP.ban` | 31 frames, 1.000 s |
| 1 | `anim_file 1` (0x25759a4f) | `chute_turn_TB_DN.ban` | 1.000 s |
| 2 | `anim_file 2` (0x077436bf) | `chute_speed_TB_UP.ban` | 1.000 s |
| 3 | `anim_file 3` (0x3df4b2b3) | `chute_speed_TB_DN.ban` | 1.000 s |
| 4 | `anim_file 4` (0x857f306c) | `chute_add_idle.ban` | 41 frames, 1.333 s |

  (The property dump lists the keys sorted by hash, which is not the slot order: slot 0 is `turn_UP`, slot 2 `speed_UP`.) All clips have 16 tracks
  (Reference, base, lft_1..4, mid_1..4, rgt_1..4, Para_LeftHandle, Para_RightHandle) on the `parachute.bsk` skeleton. The four "TB" clips are
  pose ranges: the update never advances them, it **places** them at a normalised position of their length.

### 10.2 The blend law (`0x00778c50`)

Inputs: `D+4`, `D+8` (section 10.3), `D+0x24` (10.4), `S+0x3c`, `S+0x40`, dt. With `a = |D+8|`:

* weights (instance `+0x5c`): slot 0 = `S+0x3c * a`, slot 1 = `(1 - S+0x3c) * a`, slot 2 = `S+0x40 * (1 - a)`, slot 3 = `(1 - S+0x40) * (1 - a)`; they sum to 1;
* TB clip times (instance `+0x28`, seconds): slots 0 and 1 = `length * (D+8 + 1) / 2`; slots 2 and 3 = `length * (D+4 + 1) / 2`;
* idle clip (slot 4): weight `0.25 + (1 - D+0x24) * 0.60` (constants `0x01060530` and `0x01060534` = 0.85), rate (`+0x60`) `1.5 + 3.5 * D+0x24`
  (`0x01060528`, `0x0106052c` = 5.0); its time is advanced elsewhere, but if `time + dt >= length` this update sets `time := time - length` (the
  pre-dt time, a small negative number; the unwrapped case leaves the time alone). The rate of slots 0..3 is not written here.
* So the **turn pair** (slots 0/1) is scrubbed by the bank axis `D+8` and dominates when `|D+8|` is large; the **speed pair** (slots 2/3) is scrubbed by the
  pitch axis `D+4` and dominates near zero bank. `S+0x3c` / `S+0x40` choose UP versus DN inside a pair (10.3 step 6). The dead variant: the function contains a second
  branch (`0x00778f8f`) that would use `D+4`/`D+8` as five clamped positive/negative weights, but it re-tests the same "5 clips" condition and can never run.

Fixtures (probe): neutral glide (`D+4 = D+8 = 0`, `S+0x3c = S+0x40 = 0.5`, `D+0x24 = 0.1`): weights (0, 0, 0.5, 0.5), TB clips at 0.5 s, idle weight 0.79, rate 1.85;
full bank with `D+8 = -1`, `D+4 = 1`, `S+0x3c = 1`, `D+0x24 = 1`: only slot 0 has weight 1 at time 0.0, speed clips at 1.0 s, idle weight 0.25 and rate 5.0.

### 10.3 The producer of `D+4`, `D+8`, `D+0x28` and the `S+0x3c`/`S+0x40` pair (`0x0071a190`)

Called once per flight frame by the flight model `0x005a9950` (call at `0x005ab4e6`), after the spring has updated the angles `D+0x30/+0x34/+0x38` (pitch a30,
yaw a34, bank a38) and rates `D+0x3c/+0x40/+0x44` (w30, w34, w38) of section 8 of `foot-controller-next.md`, and before the glide ratio (10.4). Arguments: the four
analog inputs p1 = FORWARD (0x1e), p2 = BACKWARD (0x1f), p3 = LEFT (0x20), p4 = RIGHT (0x21) (p1 and p2 swapped when setting 0x26 is non-zero, 10.6), the character,
the canopy, `S`, dt and "FIRE_GRAPPLE pressed or held". In order:

1. `S+0x7c = LEFT - RIGHT`, `S+0x80 = BACKWARD - FORWARD` (as given, swap included), `S+0x70 = clamp01(length)`; the character motion driver (`character+0x9c8`, slot `+0x50`)
   runs the first-order stick filter (`rico-locomotion-motion.md` section 3: 0.1 per call) and writes `S+0x8c/+0x90`.
2. `D+4 = clamp(-0.9 * w30 - 0.9 * a30, -1, 1)` and `D+8 = clamp(-0.5 * a38 - 1.0 * w38, -1, 1)` (constants `0x010773fc`, `0x01077400`, `0x010773f8`, `0x010773f4`):
   recomputed every frame from the physical pitch and bank, not accumulated. Nose up (back input) moves `D+4` negative, nose down positive; a right bank (a38 < 0)
   gives `D+8 > 0`.
3. `(D+0x48, D+0x4c)` follows `(S+0x8c, S+0x90)` (filtered LEFT - RIGHT, BACKWARD - FORWARD) at 1.0 units per second (`0x010773f0`; `0x004e6a50` equality, `0x004d6090` lerp
   by `min(1, dt / distance)`).
4. `D+0x28 = approach(D+0x28, target, rate, dt)` (`0x004f0a30`, linear, never overshoots): target 0.78 at 1.55/s while the thruster object (`canopy+0x6c`) is in state 1; else -0.8 at
   0.7/s while a hook wire is attached (`device+0x128`) and FIRE_GRAPPLE is not held; else 0 at 0.7/s (constants `0x010773d8..e4`).
   `D+0x28` is the **flare offset**: set to -0.825 when the canopy opens (`0x0074edd0`) or to -1.4 by `CDeployParachuteWhileReelingAction` (`0x005819e0`, which also resets D),
   it decays to 0 linearly, so 71 frames at 60 fps after a normal opening.
5. `D+4 = clamp(D+0x4c * (1 - |D+4|) * -0.7 + D+4 + D+0x28)` and `D+8 = clamp(D+0x2c * -0.5 + D+0x48 * (1 - |D+8|) * -0.7 + D+8)` (`0x010773ec`, `0x010773e8`, `0x010773d4`):
   a stick feed-forward (pull back lowers `D+4`, left lowers `D+8`), the flare offset on the pitch axis and the chute turn `D+0x2c` (10.5) on the bank axis.
6. `S+0x40 = clamp01(3.5 * w30 * a30 + 0.5)` and `S+0x3c = clamp01(3.5 * w38 * a38 + 0.5)` (`0x010773d0`): above 0.5 while the angle grows, below 0.5 while it unwinds.
   The simulation in the probe (native spring, roll held then released) shows both. These two fields are otherwise written by the on-foot avatar (slot `+0x5c`, `0x0074ac40`).

Steady states (fixed points of the spring): full back (a30 = +0.65) gives `D+4 = -0.585` (scrub position 0.2075), full forward +0.585 (0.7925), full right bank (a38 = -0.65) `D+8 = +0.325`
(0.6625); neutral input gives 0.5/0.5 and the centre positions.

### 10.4 `D+0x24` and `D+0x1c`

`D+0x24` is written by `0x0050aa00` (call at `0x005ab518`, right after 10.3): `min(10, horizontal speed / |vy|) * 0.1` of the velocity just written back (0 when `vy` is 0): neutral glide
(9.60, -1.69) gives 0.568, the full dive (45.1, -33.0) 0.137. It drives only the idle clip (weight, rate). `D+0x1c = 100.0` is read by the turn-rate getter `0x006c7380` (see `rico-locomotion-motion.md`);
it is not an animation input.

### 10.5 The chute turn `D+0x2c` and its producer of `S+4` (`0x0077fc80`)

* Flight model (free flight only: **not** while the hook assist is active, the previous value is kept then): `D+0x2c = approach(D+0x2c, target, 0.7, dt)` with target `0.46 * S+4` when
  `|S+4| > 0.9` and `CPlayer+0x114e` is non-zero, else 0. It feeds the yaw spring (section 8 of `foot-controller-next.md`) and, scaled by -0.5, the bank axis `D+8` (10.3 step 5).
* **`S+4` is the signed yaw of the character against the camera** (this corrects the earlier inference "facing against movement direction"). The producer is `0x0077fc80`, called once per frame by
  the CPlayer per-frame update `0x00853ae0` (`0x00672ce0`): with `c` = the planar (x, z) vector `[camera+0x24], [camera+0x2c]` of the active camera (`[[0x011919f4]+0x10]`; its rows are right at `+0x24`, up `+0x34`,
  forward `+0x44`, position `+0x54`) and `h` = the character's planar right axis (`character+0x15c`, `+0x164`), both normalised: `angle = acos(clamp(h.c)) * (h.z*c.x - c.z*h.x >= 0 ? +1 : -1)`
  (NaN becomes 0); `S+8 := old S+4`; `S+4 = wrapPi(CPlayer vtable slot 2 + angle)`. Slot 2 (`0x005a7310`) is an aim yaw offset chosen by the aim mode at `player+0x70` (rows at `0x0106f0cc`, `0x0106f0e0`,
  `0x0106f0f4`; **0.0 outside the aim modes 1..4**). `S+0xc` is a second angle (facing against the direction to `S+0x50/+0x58`), not used by the chute.
  Result: steering the camera more than 0.9 rad (51.6 degrees) away from the body heading makes the canopy yaw towards the camera (target 0.46 * S+4, up to 1.45 for a half turn), and the bank axis follows.
* `0x006e4fc0` also writes `S+4`, but only through `0x00681190 <- 0x007e0e50` and only when the character's controller is a `CAgentOnFootController` (AI agents); the player's `COnFootController` is not one
  (its isA table is {COnFootController, CCharacterController, CRtti}), so this path never runs for Rico.
* `CPlayer+0x114e` (byte): set to 1 by the CPlayer constructor (`0x007359b0`) and by `0x00797b30`, toggled 0/1 by the CPlayer event handler `0x00706c30` (two event hashes, names unresolved), read by the flight and
  skydive models and by the input-accepted predicate `0x00440da0`, `0x0054e3e0`, `0x0057cb90`. Interpretation (inferred): "player input is live"; scripted control locks clear it.

### 10.6 Setting 0x26

Settings live in the `CSettingsManager` (`[0x01196074]`, constructor `0x006c4230`); `GetSetting(id)` (`0x00747b70`) reads `this+0x8c+id*4`; the constructor zero-fills the array (code default 0 for every id).
The controller-settings metric (`0x005de2f0`, text `0x00e48690`: `SX SY AXI AYI FPI PPI FB Duck Vib`) copies setting 0x26 into the **PPI** bit, its neighbours being AXI = id 11, AYI = id 10, FPI = id 12, FB = id 13, Duck = id 0x28
(the crouch toggle of `rico-controller-next.md`), Vib = id 9, SX/SY = ids 15/14. **Setting 0x26 is therefore the "PPI" option; the expansion "parachute pitch invert" is inferred** from the neighbouring "FPI" and from its only
reader, the flight model, which swaps FORWARD and BACKWARD when it is non-zero. With 0 (code default) FORWARD is nose down (dive), BACKWARD nose up (brake). The options screen writes the user value through the Lua binding
`Game:WriteSetting` (`0x00a2d470` -> `0x005d9af0` -> `SetSetting` `0x005b5530`); the shipped default of the option is in the compiled GUI scripts, not in the executable.

### 10.7 Runtime guidance (JC2-238 / JC2-263)

* Per flight frame, after the spring: derive `D+4`, `D+8`, `S+0x3c`, `S+0x40`, `D+0x24` with the formulas above and feed them to the canopy pose: the five clips, weights and times of 10.2. Keep `D+0x28 = -0.825` at opening.
  The TB clips must be positioned (not played); the idle clip plays at rate `1.5 + 3.5 * D+0x24`.
* Camera steering: compute `S+4` from the camera right vector and the body right vector each frame and apply `0.46 * S+4` as the turn target when `|S+4| > 0.9` (skip while the hook assist runs).
* The stick filter for `D+0x48/+0x4c` needs the 0.1 per-call low-pass at the flight frame rate (native tick rate unknown, `rico-locomotion-motion.md` section 3).
* Do not use the stick directly for the pose: the native pose is mostly the physical pitch and bank plus a stick feed-forward and the flare offset.
* The engine side (time step, rates, additive idle, layer weight) is section 11; the sampled TB time is the blend time plus dt (rate +1.0), the idle clip is additive.

## 11. Applying the blend: instance time, rates, additive idle, layer weight (JC2-262 Gap 8, native unless marked)

Closes Gap 8 of section 10. Everything below is asserted by `research/tools/parachute_blend_probe.py` (function hashes, vtables, instruction anchors,
call orders, the clips' own binding data, pure-Python models with hand-derived fixtures).

### 11.1 What an "animation instance" is

The clip vector handed to the blend update (section 10.1) holds shared pointers to a 4-byte handle whose target is a 0xc0-byte object built by
`0x004a0c90` / `0x004c18f0`. That object contains Havok's `hkaDefaultAnimationControl` at **instance + 0x20** (constructor `0x00b0ec80`, base `0x00b0e5c0`; the
`CDefaultAnimationControl` vtable `0x00f2558c` is installed over it). So the instance fields used by the blend are control fields shifted by 0x20:

| instance | control | meaning | written by |
|---|---|---|---|
| +0x28 | +0x08 | local time (s) | the blend (`0x004a5f30`), the step (`0x00b0e980`) |
| +0x2c | +0x0c | eased weight the sampler reads | the step |
| +0x48 | +0x28 | binding (clip data); binding byte +0x14 = blendHint | loader |
| +0x58 | +0x38 | extra weight, 1.0 | constructor |
| +0x5c | +0x3c | master weight | the blend (`0x004aab10`) |
| +0x60 | +0x40 | playback speed ("rate") | activation, and the blend for the idle clip (`0x004a26e0`) |
| +0x6c | +0x4c | max cycles, -1 (no limit) | constructor |
| +0x70 / +0x80 | +0x50 / +0x60 | ease-in curve (0,0,1,1), ease-out curve (1,1,0,0) | constructor |
| +0x90 / +0x94 / +0x98 | +0x70 / +0x74 / +0x78 | ease rate (10.0), progress (1.0), state (0 in, 1 full, 2 out, 3 gone) | constructor; ease-in `0x004c0d20`, ease-out `0x004996b0` |
| +0x9c / +0xa0 | +0x7c / +0x80 | crop start / end (0) | constructor |

### 11.2 The per-frame order, and the step that advances the time

`0x005653c0(dt)` (the character animation update) calls, with the same dt: `0x008bce40` (the body-part loop; `0x00881640` per part, which ends with the
vtable-slot-17 call of the current state, section 10.1: this is the blend update), then `0x0090ba50` (events), `0x0086e930`, and `0x0087c7a0`, which reaches
`0x0049f370`: first `0x004c5250` (the step: for each skeleton, `hkaAnimatedSkeleton::stepDeltaTime` `0x00b05d60`, skeleton vtable `0x00f64978` slot 3, which calls
every control's vtable slot 3, `0x00b0e980`, with dt), then `0x0049f5f0` (the sampler: `0x00b077c0`, skeleton slot 4). **So within one frame the blend writes weights and
times first, the step advances every time next, and the pose is sampled after the step.**

The step `0x00b0e980`, per control (no max-cycle limit applies, the value is -1):
1. time := time - crop start, clamped to >= 0 (a negative time written by the blend is lost: it becomes 0);
2. time := time + dt * speed; if time > length (length = clip duration - crops) the time wraps by whole lengths and keeps the remainder, never below 0, and the forward loop count (+0x44) grows;
3. ease: while progress < 1, progress += |dt| * ease rate; at >= 1 progress := 1 and state := (state != 0) * 2 + 1 (in -> full, otherwise gone); the weight is a cubic Bezier of the progress
   (curve +0x50 for states 0/1, +0x60 otherwise; with the stored control points it is 3u^2 - 2u^3 in, 1 - (3u^2 - 2u^3) out), times the master weight (+0x3c);
4. the crop start is added back to the time. (The progress step reads the x87 stack top left by the wrap code; it is dt in the traced path: inferred, Gap 8c.)

The blend reads the idle clip's time with the getter `0x004a3670` next frame, so the idle time persists as the post-step time.

### 11.3 Rates (instance +0x60): the TB clips run at +1.0, not 0

The constructor sets speed 1.0. At state activation (`0x00881640` -> `0x00862350` -> `0x008f0430` -> `0x008aa620`) every new instance gets its speed multiplied by a scale taken from the state's **track data**
(the 0x68-byte record built per asb state: defaults from `0x004acaf0`: +0x40 mode = 1, +0x44 = 0, +0x48 = -1.0, +0x4c = 0, +0x50 = 0, +0x54 = 1.0): the scale is data +0x54, negated when data +0x40 is 4, 5 or 6
(`0x004a32f0`), and 0 when data +0x40 is 0. The asb loader `0x00499d60` stores exactly two properties into that record: `b7a9d0b3` (the `playback_speed` of `animation-statemachines-next.md`) into +0x54 via `0x004a0750`, and
`96270a00` into +0x4c via `0x004cc520`; `0x004ba6e0` then copies the +0x4c value to the instance's start field (+0x08, not the speed). The values `c29bffc6`, `fdcf4c12`, `371aba7e` and `57ab7197` are read into locals and never used (disassembly-verified).
For `PC_ANIM_OPEN` (data read with `jc2tool prop`): `b7a9d0b3` = 1, `96270a00` = 0. **Result: the speed of all five instances starts at +1.0** (the earlier guess in Gap 8, "96270a00 = 0 means rate 0", was wrong:
`96270a00` is not a rate). For the idle clip the blend overwrites the speed every frame (section 10.2). For slots 0..3 the blend sets only the time, so the step moves each TB clip by dt * 1.0 after the blend:
**the sampled TB time is blend time + dt (about 0.0167 s at 60 fps), wrapping at the extreme scrub position 1.0 to dt** (D+4 or D+8 saturated at +1: the pose jumps to the first frames for that frame).
Nothing else writes the speed of these instances (the only other writer in the activation code, the `0x004bf890` branch of `0x00881640` for a start with no synchronised blend, uses the same data +0x54 value).

### 11.4 Layer weight: none beyond the clip weights (hard cut at state entry)

New instances are created in state 3 (gone) and eased in with the transition blend time `this+0x78` of the body part (`0x004c0d20`; the old state's instances are eased out, `0x008bf2e0` -> `0x004996b0`).
That time comes from `0x004b75e0`: the table of the animation set at +0x4c (filled by the loader from group `0x1c2223b1`: from `2cd26c76`, to `8cd66e1f`, time `420ff89e`), then the wildcard rows, then the from-state's data +0x48 (-1.0 = unset)
and the to-state's data +0x44 (0). **`parachute.asb` has that group empty** (probe) and no data record sets +0x44/+0x48, so the blend time into `PCS_OPEN` is 0.0: the ease rate is FLT_MAX, the first step
completes the ease and the weight is exactly the master weight (the clip weights of section 10.2) from the first frame; the previous state's clips are gone after their first step. There is no state-level weight; the
per-clip weights of section 10.2 are the layer weights. (`57ab7197` = 0.3 on `PC_ANIM_OPEN` is parsed and discarded by this loader, see 11.3; nothing reads it.)

### 11.5 Combining: normal clips normalised, `chute_add_idle` additive on top

The sampler `0x00b077c0` runs two passes over the skeleton's controls (index 0, 1). A control takes part when `[ctrl+0x38] * [ctrl+0xc] > 1.19e-7` and its binding's **blendHint** (byte +0x14) equals the pass.
The binding data of the clips (their reflection data names the enum NORMAL = 0, ADDITIVE = 1): **the four TB clips are NORMAL, `chute_add_idle` is ADDITIVE** (probe, from `jc2tool cat`).
* Pass 0 (`0x00b01560`): per bone, translation, scale and the quaternion are summed with weight w (the quaternion with sign chosen against the accumulator); the per-bone weight sums are kept. Then, if the skeleton's
  reference-pose threshold (0.1, constructor `0x00b06740`) exceeds the sum, the reference pose fills the difference; then `0x00b261d0` divides translation and scale by the sum and renormalises the quaternion.
  For the canopy the four TB weights sum to 1 (section 10.2), so the result is the plain weighted blend.
* Pass 1 (`0x00b01870`), no normalisation: per bone with w = (track mask byte / 255, default 1) * the control's weight (`0.25 .. 0.85` for the idle clip): `composed.t = rotate(sample.q, pose.t) + sample.t`,
  `composed.q = sample.q * pose.q` (the delta on the **left**, product `0x00494b80`), `composed.s = sample.s * pose.s`; the pose becomes the lerp `(1 - w) * pose + w * composed` per component, the quaternion lerp taking the short
  arc (sign by the dot product) and renormalised. The clip is a delta: its translations are zero at time 0 (`animation-next.md`, additive section); **do not subtract the reference pose from it**.

The current runtime (`parachute.rs` `blended_locals`) adds `(sample.t - ref.t) * w` and slerps `sample.q * ref.q^-1` from identity: that is a reference-relative delta applied to a delta clip, which is not the native law above.
It also samples the TB clips at the blend time without the `+ dt * 1.0` of 11.3.

### 11.6 Pose-exact recipe (for JC2-238 / JC2-263)

Per frame, after the flight model has produced `D+4`, `D+8`, `S+0x3c`, `S+0x40`, `D+0x24` (sections 10.3 and 10.4) and with the previous frame's idle time `T`:
1. blend (section 10.2): weights `w0..w3`, TB times `t0..t3`, idle weight `wi`, idle rate `ri`; idle wrap: if `T + dt >= L` then `T := T - L`;
2. step: TB time `t_k := wrap(t_k + dt)` (rate 1.0, wrap by the 1.0 s length); idle `T := wrap(max(T, 0) + dt * ri)` with `L = 1.3333 s`;
3. sample: NORMAL pass over slots 0..3 with weights `w_k` (skip `w_k` below 1.19e-7), weighted sum, normalise by the weight sum (about 1); then ADDITIVE pass: nlerp from the pose toward `idle_sample * pose` by `wi`.
The state is a hard cut: the weights are exact from the first frame after `PCS_OPEN` is entered; the previous state's clips are not blended in.

### 11.7 Fixtures (probe)

Scrubbed TB clip at 0.5 s, dt = 1/60: 0.516667 s, weight = master; scrub 1.0: 0.016667 s with the loop count 1; rate 0 would leave the time (not the retail state); idle at 1.33 s (length 1.3333, d24 = 0.1):
the blend writes -0.003333, the step clamps to 0 and adds dt * 1.85 = 0.030833; a new instance with blend time 0: weight = master after one step; with 0.3 s: progress 0.055556, weight 0.79 * 0.008916 = 0.007044;
additive with base (t = (1,0,0), 90 degrees about x) and delta (90 degrees about z, scale 2): w = 1 gives t = (0,1,0), q = (0.5, 0.5, 0.5, 0.5) (the reverse product would be (0.5, -0.5, 0.5, 0.5)); w = 0.5 gives t = (0.5, 0.5, 0), q = (0.65328, 0.27060, 0.27060, 0.65328), scale 1.5.

## Gaps (unresolved native details)

1. **Thruster item** (JC2-260, resolved): creation chain, keys and defaults in `parachute-thruster-item.md`; the tuning values themselves live in a DLC pack file that is not installed.
   The shipped game has no thruster.
2. **Animation-state velocities** (RESOLVED by JC2-261, `parachute-state-velocities.md`: the state classes write nothing; the controller of the state writes the lift each frame; the freefall model flies from `S_FALL_TO_SKYDIVE`, base jump has its own law, the chute model and its opening transform start at `S_UNFOLD_PARACHUTE_*`) (was filed as JC2-261) around the chute and freefall: `S_FALL_TO_SKYDIVE`/`S_JUMP_TO_SKYDIVE`/`S_PARACHUTE_DETACH_TO_FREEFALL`
   (`CSkyDiveState`), `S_PULL_OPEN_PARACHUTE_VERTICAL`, `S_UNFOLD_PARACHUTE_*` (`CVelocityControlledState`): what the state classes write to the body while they play
   (start velocity of the skydive, unfold motion).
3. **Canopy animation blend** (JC2-262, resolved in section 10): blend law, five clip slots, the producers of `D+4`, `D+8`, `D+0x24`, `D+0x28`, `D+0x2c`, `S+4`, `S+0x3c`/`S+0x40`, setting 0x26 and `CPlayer+0x114e`. Open details are Gaps 8 to 11.
4. Hook-assist side rules: kind-1 object branch (`0x00740260`), device timer `0x0073dfe0(0.2)` and its consumer, effect index 2 of the device, `0x007ace60` effect `0x15f`.
5. Canopy mesh `weapons\gae05\GAE05_LOD1-GAE05` is not in the mounted archives, so the extents/bind-pose check uses `parachute.bsk` only.
6. (JC2-263) How the grapple device's own pull and the canopy hook assist combine natively: the runtime applies the assist and then `grapple.rs`'s reel on top.
7. (JC2-263) Body orientation in freefall and under the canopy: the native character matrix (head-down pitch, bank) drives the pose; the runtime keeps only the yaw on `Hero::facing`.
8. (JC2-262) **Applying the blend**: RESOLVED in section 11 (the step `0x00b0e980` runs after the blend in the same frame; the rate of slots 0..3 is +1.0 from the asb value `b7a9d0b3`; `chute_add_idle` is ADDITIVE with the
   delta-on-the-left law of `0x00b01870`; no state layer weight, the state is a hard cut because the transition blend table is empty). The canopy mesh `GAE05_LOD1-GAE05` is still unavailable (Gap 5). Remaining sub-gaps:
   (8a) data +0x40 (the play mode, default 1) has no writer in the asb loader and none was found elsewhere (decompile pattern search); a runtime writer would change the rate sign/zero rule of 11.3 (the default gives +1.0);
   (8b) that the parachute state machine is one of the body parts iterated by `0x008bce40` was not traced to the registration (the shared update, the call chain and the context are verified);
   (8c) the ease progress step reads the x87 top left by the wrap code (|dt| in the traced path); visible only for a non-zero blend time, which `PCS_OPEN` does not have;
   (8d) the Bezier constants (3.0, 6.0) and curve points are asserted, the exact x87 evaluation order is not (irrelevant at progress 1).
9. (JC2-262) **Writer order of `S+0x3c`/`S+0x40`**: the avatar slot `+0x5c` (`0x0074ac40`, from the move-filter blends) and the flight model both write them; the callers of slot `+0x5c` were not found, so whether
   the blend update sees the flight values or the on-foot ones when both run in a frame is unproven (the flight model runs in the CPlayer/controller update, the blend in the animation update).
10. (JC2-262) **Names of the two events that toggle `CPlayer+0x114e`** (hash fields of CPlayer read at `0x00706c30`) and the default of the options-screen value of setting 0x26 (compiled GUI, outside the executable).
11. (JC2-262) **Native tick rate of the stick filter** feeding `D+0x48/+0x4c` and of the whole flight update (shared with `rico-locomotion-motion.md`).
