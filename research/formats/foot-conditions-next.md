# Foot machine conditions: ray fans, swim depth, skydive landing, skydive camera (JC2-259, research only)

Probe: `research/tools/foot_conditions_probe.py --tool <jc2tool.exe> --game <game folder> --export <research/analysis/pseudo/_unnamed>`
(130 checks, exit 0). Executable fingerprint `df99a479...b74f80`. Closes Gaps 7 to 10 of
`rico-controller-next.md` (JC2-238) in research; nothing here is wired in the runtime yet.
Labels: **native** = read from the exe or the graph data and asserted by the probe; **inferred** = my
assembly of native pieces, said so; **Gap** = unresolved.

Every condition is an object whose vtable (RTTI) slot 2 evaluates and slot 4 loads its parameters. The
character the conditions read is `state block +0x10` (`00b1e670`); `S` below is `character +0xa34`.

## 1. Ray fans (Gap 7)

### 1.1 `CRayCastFanCondition` (`00717c60`, loader `00505e40`)

Loaded fields (the graph keys are hashed names; the exe holds the hashes at `0x00ec2a24..a54`):

| field | key | meaning |
|---|---|---|
| `+8`, `+0xc` | `4ee6e3fc` (`ec2a48`) | cos and sin of the **start** angle, degrees to radians (`* 1/180 * pi`) |
| `+0x10`, `+0x14` | `376ee745` (`ec2a4c`) | cos and sin of the **end** angle |
| `+0x18` | `c40dcfd2` | ray length (m) |
| `+0x1c..+0x24` | `4aaa87db` | origin offset vec3, in the character frame |
| `+0x28` | `ca14dc32` | the value returned when a ray hits |
| `+0x29` | `4a83e24c` | 0 = yaw-only frame (all five graph fans), non-zero = a second frame (unread; no graph data uses it) |
| `+0x2c` | `ba4ef577` | number of rays n |

The loader does not call `sin` and `cos`: it evaluates the parabola `y = (4/pi) x - (4/pi^2) x |x|`, then
`0.225 (y|y| - y) + y`, with the cosine as the sine of `x + pi/2`. The probe's model shows the worst
deviation from `sin` is under 0.0011 over -360 to 360 degrees, so using `sin`/`cos` is within 0.06 degrees.

**Frame** (flag `+0x29` = 0, native): from the right axis `(rx, ry, rz)` at `character +0x15c` the code builds
right = `(rx, 0, rz)/|.|`, up = `(0, 1, 0)`, back = `(-rz, 0, rx)/|.|`. This is the character's heading
without pitch or roll, and **local +z is behind the character** (forward is local -z). A local `(0, y, z)` is
`y * up + z * back`. The origin is the character position (`+0x18c, +0x190, +0x194`) plus the rotated offset.

**Rays** (native): the start vector `(0, cos a_s, sin a_s)` and end vector `(0, cos a_e, sin a_e)` are
rotated into the world, then for `i = 0 .. n - 1` the direction is `lerp(start, end, i / (n - 1))`
**normalised** (`004eb2f0` is a plain vec3 lerp, so the rays are not evenly spaced in angle), times the
length. Each is cast with `009a9a50` (collision filter info 9, `CCharacterRaycastFilter`); the loop stops at the
first ray with hit fraction below 1. A ray whose start or end lies outside the world bounds box
(`0098c5c0`) is not cast, so it never hits.

**Result** (native, the tail at `00718363..78`): any hit returns the flag byte `+0x28`; no hit returns
`flag == 0`. No character returns false.

### 1.2 `CForwardRayCastFanCondition` (`00799600`, loader `0084d850`)

Fields: length `+8`, hit flag `+0xc`, count `+0x10`. Same hit rule. Origin = the character position raised by
`0.25 m` (`01077c8c`). The start vector is the **negated** horizontal part of the character's third matrix
row (`+0x17c`, `+0x184`), normalised: the heading. The end vector is `(0, -1, 0)`. So the fan sweeps from the
horizontal heading to straight down in lerp steps (n = 10: elevations 0, -7.1, -15.9, -26.6, -38.7, -51.3,
-63.4, -74.1, -82.9, -90 degrees). Casts use `009c4540` (the default closest-hit collector, same filter info 9);
there is **no character filter** here.

### 1.3 The graph's fans

Elevation is below (-) or above (+) the horizontal, along the heading; "behind" flips the heading.

| condition | class | angles (start to end) | origin offset | length | n | hit returns | rays (elevation, degrees) |
|---|---|---|---|---|---|---|---|
| `C_CAN_FALL_TO_SKYDIVE` | fan | -180 to -110 | (0, 1.5, 0) | 30 | 5 | 0 | -90, -74.3, -55.0, -35.7, -20.0 |
| `C_CAN_JUMP_TO_SKYDIVE` | fan | -135 to -100 | (0, 1.5, -0.5) (0.5 m ahead) | 30 | 5 | 0 | -45, -36.5, -27.5, -18.5, -10.0 |
| `C_CAN_JUMP_CLEAR_AHEAD` | fan | -180 to -80 | (0, 1, 0) | 3.5 | 5 | 0 | -90, -70.8, -40.0, -9.2, +10.0 |
| `C_REELED_IN_TO_UPRIGHT` | fan | 150 to 100 | (0, 0, 0.75) (0.75 m behind) | 20 | 10 | 1 | -60 to -10, behind the character |
| `C_PARACHUTE_TO_FALL` | forward fan | heading to down | +0.25 m | 30 | 10 | 1 | 0 to -90 (above) |
| `C_PARACHUTE_TO_SKYDIVE_FANCY` | forward fan | same | +0.25 m | 60 | 10 | 0 | same |
| `C_PARACHUTE_TO_SKYDIVE_SIMPLE` | forward fan | same | +0.25 m | 60 | 10 | 1 | same |

Reading (native rule, so the sense is certain; the English is mine): the skydive entries from the fall need
**no hit at all** in the cone from straight down to 20 degrees below the horizontal within 30 m (and the jump
entry in the cone from 45 to 10 degrees below, plus nothing within 3.5 m in the cone from straight down to 10
degrees above). The chute release edges: `S_PARACHUTE` + `ACT_PULLCHORD` goes to `S_FALL` if **anything** is within 30 m in
the forward-down cone, otherwise to the free-fall detach if nothing within 60 m (`FANCY`, hit returns 0), else to
`S_FALL_TO_SKYDIVE` (`SIMPLE`, hit returns 1); the edge order in the graph decides which fires.

**Runtime today** (`foot_machine.rs`): every fan is answered by the straight-down clearance or one forward
ray. The replacement is a handful of rays per condition exactly as in the table; `S_JUMP_START_SPRINT_FORWARD`
-> `S_JUMP_TO_SKYDIVE` needs all of its three conditions at once, so one clear cone is not enough.

**Inferred**: that the character position is the feet (the physics capsule has its feet at the origin,
JC2-264), which makes the 1.5 m fall origin 0.2 m under the top of the 1.7 m capsule; that the second frame
(`+0x29` != 0) is unused.

## 2. Water: `CAtSeaLevelCondition`, the swim-depth family, steering and view pitch (Gap 9)

### 2.1 `CAtSeaLevelCondition` (`0059a1e0`, loader `005e0c00`, constructor `00714ec0`)

(`water-sea-consumers.md` section 5 already resolved the object: a body-part slot of the `CCharacter`; the
parts of it that matter here, plus what is new.)

* Position: world transform (`00677d60`) of the joint in character slot `+0x41c + index*4`; slot 0 is the
  root (`Reference`), **slot 1 is `Hips`** (table in `water-sea-consumers.md`).
* `diff = joint.y - wave(joint.x, joint.z)`; flag 0 is `diff < offset`, flag 1 is `offset < diff`, both strict.
  Defaults: offset 0, flag 1, slot 0. Keys: flag `3c9c0ff1`, offset `49b4a2d6`, slot `77cfdc9b`.
* **New**: the wave is `a8bba0 = 200 + a88ea0` called **directly** with the joint position and the camera
  position (`camera object +0x54/+0x58/+0x5c`), not through the sea interface's slot 2. `a88ea0` gates on the
  **point's** y (`|y - 200| <= 5`) and on the xz distance to the camera (`<= 66.67 m`, constant `4444.4453`);
  outside it the answer is 0, so the wave is 200 exactly. For the sea interface (`water-cpu-wave.md`) the
  point's y was the camera's, so the existing `SeaWave::offset(x, z, gate_y, r)` takes the **hips' y** as `gate_y`
  here. The wave is scaled by 0.6 and fades with the vertex's distance from the camera.

Instances (graph data, all asserted):

| condition | flag | offset | slot | true when |
|---|---|---|---|---|
| `C_BELOW_SWIM_DEPTH` | 0 | -0.4 | 1 (Hips) | hips are more than 0.4 m below the wave |
| `C_ABOVE_SWIM_DEPTH` | 1 | -0.05 | 1 (Hips) | hips are less than 0.05 m below the wave, or above it |
| `C_AT_SURFACE` | 1 | -0.4 | 0 (root) | the **root** (feet) is less than 0.4 m below the wave |
| `C_ENOUGH_ABOVE_WATER_TO_FALL` | 1 | 1.0 | 0 | root more than 1 m above the wave |
| `C_ENOUGH_ABOVE_WATER_TO_SKYDIVE` | 1 | 28.5 | 0 | root more than 28.5 m above the wave |

The runtime answers `C_BELOW_SWIM_DEPTH` and `C_ABOVE_SWIM_DEPTH` from `hero.swimming` (the capsule centre
against the flat sea with its own 0.3 m margin, `character.rs` line 1103) and the other three from
`feet_above_sea` / `hips_above_sea` taken against the **flat** sea (`character.rs` line 1790). The native
tests are the hips (slot 1) for the first two and the root (slot 0) for the last three, all against the
**wave** height. Since the wave is at most 0.6 m and only exists within 5 m of the sea level, the flat sea is
the right answer outside that band and off by up to 0.6 m inside it.

Where they act (graph, asserted): many ground, fall, chute and ragdoll states have `C_BELOW_SWIM_DEPTH`
edges (to `S_SWIM_RUN_TO_SWIM` from the ground states; from the air to `S_BLEND_TO_SWIM_IDLE` when also
`C_VERTICAL_SPEED_LOWER_THAN_SUBMERGE` (vertical speed above -15 m/s) and otherwise to `S_SWIM_DIVE_IDLE`);
`S_SWIM_DIVE_IDLE` and `S_SWIM_DIVE_NAVIGATION` come up on `C_AT_SURFACE`; `S_SWIM_NAVIGATION` and
`S_SWIM_RUN_TO_SWIM` stand up on `C_ABOVE_SWIM_DEPTH` + `C_REACH_SEA_FLOOR` (height over ground <= 1.1, flag 1)
+ `C_SWIM_TO_UPRIGHT` (a single ray, class `CRayCastCondition` `007caa40`, up 1.8 m from the root; not decoded here, Gap 5).

### 2.2 Submerge: steering and view pitch

`S_SWIM_NAVIGATION` + `ACT_NAVIGATE` -> `S_SWIM_SUBMERGE` needs `C_STEERING_FORWARD` and
`C_VIEW_PITCH_TO_SURFACE` (the name is misleading: it gates going **down**).

* `CMovementInputCondition` (`007ecc80`): `dot(S + 0x7c/0x80, the condition's vec2 (normalised
  at load, (0, 1)))` against `0.8`, flag 1 meaning `0.8 < dot`. `0067f790` reads this pair and writes the
  filtered magnitude at `S + 0x8c/0x90`, so it is the (inferred) unfiltered stick. Steering forward is then
  **stick y above 0.8** (a mostly straight-ahead push).
* `CViewPitchCondition` (`007fa7d0`): `S + 0x68` against `-25 deg` (radians at load), flag 0 meaning
  `pitch < -25 deg`. `S + 0x68` is written each update by the player-input updater `0077fc80` from the
  controller's camera object (`controller + 0x9c`, vtable slot 3 through `0062fba0`) and zeroed for AI.
  **Inferred**: negative pitch is looking down (the edge dives), so submerging needs the camera pitched more
  than 25 degrees down while pushing forward.

These two are the only conditions of the dive entry. `S_SWIM_DIVE_*` (served by `CDiveController`, not the
swim controller) is entered from the air by `C_BELOW_SWIM_DEPTH` alone (above). Everything about the dive's
motion is Gap 3 below.

## 3. Skydive landing (Gap 10)

`S_SKYDIVE` + `ACT_LAND` -> `S_HITREACT_UNCONTROLLED_FLIGHT`, no condition (graph, asserted). The state is a
`CAnimationState` with the `getup_up` / `getup_down` clips (blend 0, speed 1), served by
`CRagdollingController` with the other ragdoll states (`ragdoll-next.md`). The chain (graph, conditions
asserted):

* `S_HITREACT_UNCONTROLLED_FLIGHT` -> `S_AIRBORNE_RAGDOLL` when the height over ground is at least 0.8 m and
  the character is alive; -> `S_DEAD` when the mean ragdoll body speed is under 0.7 m/s and the character is
  dead (health at `+0xe8` <= 0); -> `S_HITREACT_UNCONTROLLED_FLIGHT_POSEMATCHING` when the mean speed is
  under 3.0 m/s and alive; and the swim edges.
* `S_AIRBORNE_RAGDOLL` -> `S_UNCONTROLLED_FLIGHT_PROTECT` at 0.4 m or less over the ground; ->
  `S_HITREACT_GET_UP_BLENDIN` when slow (< 0.7 m/s), alive and the 6.0 s flight timer has run out.
* `S_UNCONTROLLED_FLIGHT_PROTECT` -> `S_AIRBORNE_RAGDOLL` at 1.8 m or more; -> `S_HITREACT_UNCONTROLLED_FLIGHT`
  at the end of its clip or when the hip speed is under 3.0 m/s.
* `S_HITREACT_UNCONTROLLED_FLIGHT_POSEMATCHING` -> `S_HITREACT_GET_UP_BLENDIN` when slow and near the ground
  or timed out; -> back to flight when the hip speed is 5.0 m/s or more. `S_HITREACT_GET_UP_BLENDIN`
  (speed 0.6) -> `S_HITREACT_GET_UP` when the ragdoll has stopped blending.

So the skydive landing is the **ragdoll** path, not `land_chute_hard`. Death is not in this graph: the dead
branch needs the character dead already (health, set by fall damage or the impact elsewhere).

`ACT_LAND` has three senders in the exe: the canopy update `005a9950`, the fall handler `007f5810` (only from
`S_JUMP_TO_FALL_TRANSITION`) and `00619510`, the generic one: **whenever the
ground-contact test holds** it sends `ACT_LAND` to the character machine (`+0xf0`, vtable `+0x74`) and plays
the "on foot" sound. So the action arrives on the first frame of contact (and, inferred from the unconditional send, on each frame
while in contact; states without the edge ignore it). The runtime sends it once on `touched_down`
(`foot_machine.rs` line 698), which is equivalent for the skydive. Not shown: how often `00619510` runs.

**Runtime today** plays the heavy landing for a skydiver. Replacing it needs the ragdoll controller (Gap 2).

## 4. Which controller selects the skydive camera (Gap 8)

The camera id lives in the **controller** (`controller + 4`, vtable slot 7 `009573c0`), filled once at start-up
by `006f77a0` from `player.bin` `cameras` (`camera-fov-next.md`; JC2-244): each key is written to the
controller selected by the Jenkins hash of its class name. Asserted here: `SkyDive` (`ee07c884`, id 2) goes to
`CSkyDiveController` (`d9de768f`), `BaseJump` (2) to `CBaseJumpController`, `Parachute` (1) to
`CParachuteController`. What was open is **which states each controller serves**; the player's registration
(`00766260`, 52 pairs, parsed by the probe) says:

| controller | camera | player states |
|---|---|---|
| `CSkyDiveController` | 2 (SkyDive) | `S_PULL_OPEN_PARACHUTE_VERTICAL`, `S_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_TO_SKYDIVE_DASH`, `S_FROM_SKYDIVE_DASH`, `S_SKYDIVE_DASH`, `S_FALL_TO_SKYDIVE` |
| `CBaseJumpController` | 2 (BaseJump) | `S_JUMP_TO_SKYDIVE` |
| `CParachuteController` | 1 (Parachute) | `S_UNFOLD_PARACHUTE_VERTICAL`, `S_UNFOLD_PARACHUTE_HORIZONTAL`, `S_PARACHUTE` |

(`CAgentSkyDiveController` serves the AI variants; the ragdoll `S_UNCONTROLLED_SKYDIVE` states belong to
`CRagdollingController`, camera 25.) The conclusion: the skydive camera is on the moment the main-layer state
is any of those seven, i.e. from **`S_FALL_TO_SKYDIVE` (the entry that comes with 28.5 m of free air)**,
not after 1.5 s of air time and 12 m/s of fall. `gameplay_camera::is_skydiving` should return `state in {the
seven}`; the chute camera on the three canopy states, base-jump camera on `S_JUMP_TO_SKYDIVE`. Precedence over
swim, hang and the rest stays as the runtime has it (the native one is the state map: one controller per
state, so no two can compete).

**Inferred**: that the camera system follows the active controller's `+4` each frame (the on-foot controller
rewrites its own `+4` per frame for the crouch camera, which only makes sense if something polls it). The one
call that hands an id to the camera manager (`005f0000`, start-up) was found; the per-controller-change
consumer was not (Gap 4).

## Gaps

1. **Ray collision semantics.** Filter info 9 (7 when a flag is set) of `009a9a50`, the default collector of
   `009c4540`, and the object method `+0x18` (arguments 1 and 3) that `CCharacterRaycastFilter` (`004b1950`)
   tests are not decoded (shared with JC2-264 Gap 2). Blocks: whether vehicles, characters, props and water
   stop a fan; the runtime can only treat static world geometry as blocking.
2. **Ragdoll entry from the skydive landing.** What `CRagdollingController` does on entering
   `S_HITREACT_UNCONTROLLED_FLIGHT` for the player (inherited velocity, profile, fatality, which of `getup_up`/
   `getup_down`), and the height measure `00557050(+0xd)` behind `CHeightOverGroundCondition` in the ragdoll
   states (`f3ca4a36`). Blocks: replacing the heavy landing; see also `ragdoll-next.md` unknowns.
3. **Dive and submerge motion.** `S_SWIM_SUBMERGE`, `S_SWIM_DIVE_*` (`CDiveController`): the vertical and pitch
   control, the clips and speeds; the sign and update of `S + 0x68` (inferred negative = down) and the
   non-player value. Blocks: diving.
4. **Camera consumer.** Which code applies the active controller's camera id (`controller + 4`) to the camera
   manager on a controller change, and the blend start. Blocks: exact timing of the skydive and chute camera swap.
5. **Single ray and second frame.** `CRayCastCondition` `007caa40` (`C_CAN_FALL_A/B`, `C_SWIM_TO_UPRIGHT`,
   `C_REELED_IN_TO_FREEFALL`) and the fan's second frame (`+0x29` != 0, no graph uses it) were not decoded.
   The runtime's downward-ray stand-in for the first group is unverified.
