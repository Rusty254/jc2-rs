# Parachute and skydive camera placement (board 3eac0c46, closes Gap G8 distance part of 909d394d)

*11 October 2026, `claude/chute-3eac`. Retail `JustCause2.exe` sha256 `df99a479...4f80`. Addresses are native VAs; the decompiler export is the
ignored local `research/analysis/pseudo/_unnamed`. Probe: `python -B research/tools/chute_camera_probe.py $game --export $export --tool $jc2tool`
(exit 0 only if every check below passes).*

## Answer

`camera-fov-next.md` said the camera tables hold only FOV, filter and spring. That was wrong: each `settings/camera_settings.bin` record carries
the placement vectors, limits and filter-type ids, consumed by the camera manager state builder `00838720` and the per-frame update `0062be20`.
The retail values for the two chute cameras (ids from `settings/player.bin` `cameras`: Parachute = 1, SkyDive = BaseJump = 2):

| key (hash) | record offset | Parachute (id 1) | SkyDive (id 2) | meaning |
|---|---|---|---|---|
| `0x93d9cae6` | `+0x04` | (0, 3.75, -0.5) | absent (0) | pivot offset in the node's local frame (only read when both node ids are equal and non-zero) |
| `0x80cf2614` | `+0x84` | **(0, 0, 6.75)** | **(0, 3.0, 2.5)** | camera offset, rotated by the orbit orientation, +Z = behind the view direction |
| `0x15ed1fea` | `+0x30` | (0, -1, 0) | (0, 0.5, -1) | look-at (target) offset |
| `0x82bbc61c` | `+0xcc` | yaw -70 .. +70 deg | -85 .. +85 deg | yaw limits (converted to rad by `007d44b0`) |
| `0x8fc27e6c` | `+0xd4` | pitch -65 .. +75 deg | -25 .. +45 deg | pitch limits |
| `0x3e727c53` / `0xa4c2bdc` | `+0x2c` / `+0x80` | 2 / 2 | 2 / 2 | target node id / camera node id (`0058c0a0`) |
| `0x50692f83`, `0x8e260e93` | `+0x9c`, `+0xa0` | 2 (interpolation), 5.0 | absent (none) | camera pivot smoothing type and rate (per axis, 1/s) |
| `0x294585fd` | `+0xbc` | 8 (`CCameraCollisionParachute`) | 1 (`CCameraCollisionFull`) | collision filter |
| `0xd3185ed6` | `+0x104` | 2 (`CCameraInputNonlinearAccel`) | 1 (`CCameraInputNonlinear`) | input filter |
| `0xf2166f3a` | `+0x108` | 3 (`CCameraMovementParachute`) | 5 (`CCameraMovementFreefall`) | movement (orbit) class |
| `0xc4afb170` / `0xc26c0ec2` | `+0xf0` / `+0xf4` | 12.0 / 0.1 | 2.0 / 0.1 | movement parameters (Parachute: 12.0 is the yaw recentre duration, section 3) |

Neither record has `Fov` (45 deg default) or a FOV filter. For contrast the on-foot record (id 0) has camera offset (0.55, 1.55, 3.0), target
(0.55, 1.55, 0) and no pivot, so 3.0 m behind; the project's `CAMERA_DISTANCE = 4.2` and `CAMERA_DISTANCE_CHUTE = 7.5` are both non-native.

## Proven

### 1. State builder `00838720` and per-frame order `0062be20`
1. Builder: `state+0x04 = def+0x04` (pivot), `state+0x10 = def+0x84` (camera offset), `state+0x1c = def+0x30` (target offset); two matrix providers are
   fetched by `0058c0a0(id, params)` (camera node from `def+0x80`, target node from `def+0x2c`, both id 2 for ids 1 and 2). The filters are built
   from `def+0x104` (input, `00816230`), `def+0x108` (movement, `005be250`), `def+0xbc` (collision, `00771930`), `def+0x10c` (crosshair),
   `def+0x9c` / `def+0x48` (smoothing, `00660f40`), `def+0xfc` (FOV, `005d9760`), `def+0x100` (`0053e7d0`).
2. Per frame (`0062be20`): pivot world position `P = node.pos + node.R * state+4` (`005ea0b0`); the smoothing object (vtable slot 0) updates
   `state+0x28` (camera side, node `+0x80`) and `state+0x34` (target side, node `+0x2c`); **both results are written into the matrix translations**
   (camera), matrix B (target), the input deltas (`block[0x20]`, `block[0x21]`), `state+0x10` camera offset, `state+0x1c` target offset. Filters then
   run in this order: collision pre (`+0x90`, vt+0xc), input (`+0x94`), `+0xa4`, crosshair (`+0x98`), movement (`+0x9c`), FOV (`+0xa0`), collision post.
3. The movement update ends in `00688380`, which composes the final pose: **camera position = A.pos + A.R * camera offset**, **target = B.pos + B.R * target
   offset** (`block+0x88`, `block+0x94`). So the follow distance is the Z of the camera offset, measured from the smoothed pivot along the camera's
   own orientation (a true orbit: the offset rotates with yaw and pitch).

### 2. Pivot smoothing `00576360` (CCameraSmoothingInterpolation, factory `00660f40` type 2)
The previous smoothed position is expressed in the node's local frame relative to the new pivot; each local axis is scaled by `1 - clamp(rate * dt, 0, 1)`
(rates `def+0xa0/+0xa4/+0xa8`, all equal to key `0x8e260e93` = 5.0 for the Parachute camera; an optional per-axis clamp `+0x8/+0xc/+0x10` is disabled, default -1),
then moved back to world space. The Parachute camera therefore lags the pivot with an exponential factor `1 - 5 dt` per frame in the character frame. SkyDive and
the target side of both have smoothing type 0 (the base object `006b78e0`; no lag seen in its fields).

### 3. CCameraMovementParachute update `006b26a0` (vtable `00eb8204`, slot 1; constructor `00813390`)
* Yaw offset (`+0x3c`) and pitch offset (`+0x40`) accumulate the input deltas, are wrapped by 2*pi (`0x01065a4c`) and clamped to the yaw limits (`+0x10/+0x14`)
  and pitch limits (`+0x60/+0x64`, copies of `+0x18/+0x1c`) from the table above.
* Absolute orientation: yaw = `[D+0x40] + yaw offset + base yaw`, pitch = `base pitch + pitch offset`, where the base yaw/pitch (`+0x20/+0x24`) are the Euler angles of the
  incoming matrix (`004de960`), `D = [S+0x170]` is the canopy flight block and `S = [[player+0x48]+0xa34]`. The matrix A orientation is then **replaced** by
  `004e1470(yaw, pitch, 0)` (translation kept).
* Auto recentre: with no input (`block[0x20] == block[0x21] == 0`) and `|S+4| > 0.9` (`0x0106a930`; `S+4` is the chute turn axis of `parachute-canopy-landing-next.md` 10.5)
  a timer `+0x30` runs; `t = timer / 12.0` (`+0x28`, key `0xc4afb170`); when `t < 1` the yaw offset is reduced by `offset * bezier(0,0,1,1; t)` (`004c1ae0`, cubic with
  control values 0, 0, 1, 1), at `t >= 1` it is set to 0. Any input zeroes the timer.
* Extra offset: `+0x58` approaches 0.5 and `+0x5c` approaches 1.3 (`0x0106a928`, `0x0106a924`) at constant speed 0.5 per second (`004f0a30`: step `rate*dt`), starting at 0 when the
  movement object is built; the vector `(0, +0x58, +0x5c)` (plus a zero `+0x48..`) is rotated by the new orientation and added to A.pos. Steady state: camera offset
  `(0, 0.5, 1.3)` on top of `(0, 0, 6.75)`, i.e. (0, 0.5, 8.05); the up part is reached after 1 s, the back part after 2.6 s.

### 4. CCameraMovementFreefall update `00652160` (vtable `00eb8230`; constructor `006ebed0`)
* Yaw and pitch offsets (`+8`, `+0xc`) accumulate the input deltas, wrapped by 2*pi and clamped to the limits above (`+0x10/+0x14`, `+0x18/+0x1c`).
* Orientation = incoming node matrix times a tilt rotation `004e1470(0, tilt, 0)` times the input rotation `004e1470(yaw, pitch, 0)` (matrix products `00491e50`);
  the target matrix B is multiplied by a further yaw/pitch rotation when `00640fd0` succeeds (mode key `0x1acabe1c`; true for SkyDive, addendum C).
* Tilt (`+0x54`) is a damped spring `v += (-k x - c v) dt'; x += v dt'` with `k = c = 10.0` (constructor `+0x5c/+0x60`), dead band 1e-5 (`+0x58`), `dt' = min(dt, 0.1)`, about a
  target set by `f = 1 - P` (`P = S+0x148`, the pitch pose of `foot-controller-next.md` section 9): `f >= 0.8` gives +0.4 rad, `0.6 < f < 0.8` ramps linearly 0 to +0.4,
  `0.4 <= f <= 0.6` gives 0, `0.2 < f < 0.4` ramps 0 to -0.4, `f <= 0.2` gives -0.4 rad (constants `+0x3c..+0x50` = 0.4, -0.4, 0.8, 0.6, 0.4, 0.2).
* If the delay (`+0x2c`, key `0xc26c0ec2` = 0.1) is positive, `00534e70` runs first: it is the idle recentre of the offsets (addendum C), not an input filter.

## Inferred (from code, not run-verified)
1. Camera-local +Z is "behind" (matches the on-foot offset 3.0 and the project's `back = rot * Vec3::Z`), so the Parachute camera sits 6.75 m (8.05 m settled) behind the pivot
   along the view direction, the SkyDive camera 3.0 m above and 2.5 m behind.
2. (Resolved, addendum A) Node id 2 is the player character's matrix provider (`CMatrixProxy_Player`, character + 0x15c).
3. (Resolved, addendum B) `[D+0x40]` is the canopy yaw rate w34, added to the yaw as a plain float.
4. A fixed-step port that uses the real `dt` reproduces the smoothing and the 1 s / 2.6 s approach at any frame rate by construction.

## Addendum: gaps 1 to 4 resolved (board b8462122, `claude/chute-b846`)
Probe: `python -B research/tools/chute_camera_gap_probe.py $game --export $export --tool $jc2tool` (exit 0, 5 check groups). Everything below is native-proven unless marked
"inferred"; it supersedes Inferred 2 and 3 above and the "not decoded" notes in sections 3 and 4.

### A. Camera node id 2 is the player character's world matrix (closes Gap 1)
* `0058c0a0(id)` calls `00903530(id)`, a factory lookup (registrations in `007b2fc0`: 0 None, 1 ControlledObject, **2 Player**, 3 PlayerHip, 4 Vehicle, 6 PlayerRightHand, 8 PlayerHead,
  9 Freefly, 10 MovieCam, ...). Id 2 therefore creates a `CMatrixProxy_Player` (vtable `00e4bc58`, constructor `00830110`, holds a weak reference to the local CPlayer). Its slot 0
  (`005b04a0`) fetches the player's CCharacter and calls the secondary-base virtual at +0x48 (`0062aaf0`), which copies the **16 floats at character + 0x15c**; slot 1 is "is valid".
* `character + 0x15c` is the character's whole 4x4 world matrix, the same one the canopy model copies verbatim and the canopy flight model writes (`parachute-attach-pivot.md` section 2:
  position at the feet, orientation `M = Rz(bank) Rx(pitch) Ry(yaw)` while the canopy flies; skydive writes its own pose through the same entity). So for camera ids 1 and 2:
  matrix A = matrix B = the character matrix at the feet, with its full orientation, and the pivot (0, 3.75, -0.5) is a **character-local** offset (3.75 m above the feet along the
  character's up axis, 0.5 m along local Z). It follows the body/canopy orientation, not just the heading. Frames: translation of A and B are replaced by the smoothed pivot results
  (section 1.2), the rotation part is the character rotation (then replaced by the movement class: Parachute section 3, Freefall section 4).
* Inferred: the character matrix rotation columns follow the project's `Quat` of the player body in flight (rows R0 side, R1 up, R2 back as in `parachute-canopy-landing-next.md`).

### B. D+0x40 is the canopy yaw rate (closes Gap 4a)
`D = [S+0x170]` is the flight data block of `foot-controller-next.md`; `D+0x34` is the yaw angle a34 and **`D+0x40` its rate w34** (rad/s). The flight model (`005a9950`) multiplies it by
`exp(-0.95 dt)`, adds the spring/input term and integrates it into `D+0x34`. `006b26a0` adds `w34` to the camera yaw as if it were an angle (native quirk, proven by the formula
`yaw = w34 + yaw offset + base yaw`). Practical effect: the Parachute camera yaw leads/lags the character yaw by a number of radians equal to the instantaneous turn rate in rad/s, which is
small in glide and larger in a hard turn. The Rust port should read the equivalent canopy yaw rate and add it unscaled.

### C. Freefall target matrix mode (closes Gap 4b)
`00640fd0` is a method of the movement base class (`006fe400`): base `+4` is the table value at record `+0xe8` (key `0x1acabe1c`, loader `00808070`), base `+8`/`+0xc` are the Freefall
yaw/pitch offsets. Mode 1 builds the rotation `004e1470(yaw offset, pitch offset, 0)`, mode 2 yaw only, mode 3 pitch only, any other value returns false (no multiply). **SkyDive (id 2) has
the key = 1; Parachute and the on-foot camera do not (mode 0).** So after A receives tilt and input rotation, **the target matrix B is also rotated by the input yaw and pitch**: the
look target orbits with the same offsets. The target is `B.pos + B.R * (0, 0.5, -1)`; with B rotated, the look point turns with the stick together with the camera. (The multiplication
order of `00491e50` operands, as for A, follows the section 4 reading; not independently verified.)
`00534e70` is **not** an input filter: it is the Freefall idle recentre. When both input deltas are exactly 0 the timer (`+0x30`) accumulates dt; after the delay `+0x2c` (key `0xc26c0ec2`,
0.1 s) the yaw/pitch offsets (`+8`/`+0xc`) move towards the rest values `+0x20/+0x24` (zero: the Freefall constructor `006ebed0` clears them) by `offset += (rest - offset) * w` every frame with
`w = clamp((timer - 0.1) / duration)`, duration `+0x28` = key `0xc4afb170` = 2.0 s for SkyDive; any non-zero input resets the timer and the active state. Because `w` grows with the
timer and is applied to the remaining difference each frame, the offsets converge on the rest pose within about 2 s of idle after the 0.1 s delay (the active flag `+0x36` is set once the delay
elapses and `+0x34`, record `+0xf8`, is 1, which both chute records have).

### D. Collision filters (closes Gap 2 for the Full class; Parachute decoded, geometry details open)
Both classes have a no-op pre slot (`009a4cf0`, `ret 4`) and do their work in the post slot (vtable slot 4). Both call the same shape cast `007b9fe0` from the target to the camera
position and get back a hit fraction `h` (1 = free). Camera block positions are camera `(+0xa0, +0xa4, +0xa8)` and target `(+0xac, +0xb0, +0xb4)`. Distance clamp `[record +0xc4, +0xc8]` is
disabled for ids 1 and 2 (defaults 0 and -1, clamp only when min < max; key `0x49b4a2d6` absent).
* **CCameraCollisionFull** (id 2, `00633c00`; constructor `007d13c0`): the stored fraction `f` (`+0x24`, starts 1.0) snaps to `h` when `h < f` (instantly pulled in). When `h >= f` it relaxes
  with a damped spring: `a = -k (f - h) - c v`, `v += a dt'`, `f += v dt'` where `k = c = 10.0`, `dt' = min(dt, 0.1)`, snapped to `h` once `|f - h| < 1e-5`; `f` is kept above 0.000999 (`0x3a83126f`
  replaces an exact 0). The camera is then `target + unit(camera - target) * |camera - target| * f`; the target is untouched. So geometry pulls the camera in at once and it eases back out
  critically damped (spring `k = c = 10`, about 0.5 s to settle).
* **CCameraCollisionParachute** (id 1, `00536800`; constructor `0057e730`, constants `+0x30` f = 1.0, `+0x34` = 3.8, `+0x38` = 0.9, `+0x3c` = 2.0, `+0x40` = 0.5): it first builds a sweep
  volume from the camera state vector `state+0x64/+0x68` doubled (and 0.5 deep) and places it along the target-to-camera line. The fraction `f` moves towards `h` at constant speed
  (`004f0a30`: step = speed * dt toward the target, no overshoot): **3.8 per second when closing, 0.9 per second when opening**, then is floored at 0.5 and capped at 1 (the
  filter object `005f51e0(.., 1.0)` has weight 1.0, i.e. a pass-through). Pose: with `u` = unit(camera - target) and `D` = |camera - target|, the camera becomes `target + u * f^2 * D` and the target
  moves back along the line by `(f - f^2) * D` (separation `f D`); then both camera and target are lowered by `(1 - f) * 2.0` in Y. If `f` was floored at 0.5 the camera is re-placed at
  `1 + D * (0.5 - f)` along u from the new target. The function returns true when the remaining separation minus `h` distance falls below 1.5 (`0x010655bc`) - the caller's "camera
  blocked" flag. Unit of the Y dip and component naming are inferred from the field layout; the exact sweep shape (`007b9fe0` arguments) is not decoded.

### E. Camera input filters (closes Gap 3)
Filters are built by `00816230`: type 1 `CCameraInputNonlinear` (SkyDive), type 2 `CCameraInputNonlinearAccel` (Parachute). Common base (`0072a380`): sensitivity `s = record +0xe4` (key
`0x24868714`, default 1.0, absent for ids 1 and 2) and, for camera id 2 when option 0x10 is off, `s * 0.6`; per-axis device scales `X`, `Y` come from the options object (`007a92c0`, `0065e4c0`:
user sensitivity integer * 0.1, times a further 0.2 in the second device mode). The raw stick `(x, y)` is the look axes of the player input (indices 5-6 and 4-3, through a smoothing helper). The
output deltas written to the filter block are `block+0x80 = X * s * out.x * dt` (yaw) and `block+0x84 = Y * s * out.y * dt` (pitch), added by the movement classes to their offsets.
* **Nonlinear** (`0074e830`): `m = |stick|`; if `m != 0`: `t = clamp(m^4 + 0.3, 0, 1)`, `out = 1.5 * stick * t`.
* **NonlinearAccel** (`0079b680`): dead zone `m > 0.01`; `t = clamp(m^2, 0, 1)`, `out = lerp(0.05 * dir, 1.2 * dir, t)` with `dir = stick / m`. Acceleration: if `m > 0.98` and the dot product of the
  previous and current stick vectors exceeds 0.98 the hold timer `+0x34` grows by dt, weight `w = clamp(timer / 0.75)`, and `out = lerp(out, (4.0 + (2.5 - 4.0) * X/2) * stick.x ..)` (per axis,
  using that axis's device scale halved); otherwise the timer resets to 0. Below the dead zone `out = 0`.
These are rad per second scaled by the per-frame dt as given; since the movement limits are in rad the units are consistent. The stick-to-delta mapping for mouse motion goes through the same axes
(inferred: the mouse look axes feed indices 3 to 6).

## Gaps (remaining after the addendum; none closed by guessing)
   record `+0xb8` camera-start modes 2/3/4) decide the first frame after a camera switch (no jump). Minor.
2. **Sweep volume of the collision classes**: `007b9fe0` arguments and the `state+0x64/+0x68` vector (set outside the builder) define the shape cast; the runtime keeps its project ray cast (0.3 m margin).
   Blocks only the near-geometry shots.
3. **Device scales and the stick smoothing helper** (`007c5220`, `004d6150` object alpha) for mouse input; record key `0x24868714` is absent for both chute cameras so `s` is 1.0 (0.6 for SkyDive unless option 0x10).
4. **Record `+0xdc`** (key `0xac8f3c89`, 1 for SkyDive) is not read by the builder; its consumer is unknown (not needed for placement).
5. **Camera transition position blend** (freefall group 0.5 s, `camera_transitions.bin`) is read by `camera-fov-next.md` for FOV only; the position blend `manager+0x104`
   weight is not applied to the pose by the runtime.
6. **No in-game capture** of the realised chute camera; the reference shots in `reference/chute/` were not measured against these numbers.

## Bounded Rust handoff (task filed)
1. `jc2-formats` camera data: read the keys of the table (`0x93d9cae6`, `0x80cf2614`, `0x15ed1fea`, `0x82bbc61c`, `0x8fc27e6c`, `0x50692f83`, `0x8e260e93`) into the camera definition.
2. `gameplay_camera.rs` / `character.rs` `hero_camera`: for camera ids 1 and 2 replace the orbit distance by the native model: pivot with local-frame exponential smoothing (rate 5.0 for id 1,
   none for id 2), camera position `pivot + R(yaw,pitch) * offset`, target `pivot_target + offset`, yaw/pitch offsets clamped to the limits, the Parachute 12 s yaw recentre, the (0, 0.5, 1.3)
   ease-in and the freefall tilt spring. Remove `CAMERA_DISTANCE_CHUTE`.
3. Tests: fixtures from the probe output (offsets, limits, 0.9 gate, 1.0 s / 2.6 s approach, smoothing step).
4. Follow-up (addendum, board b8462122): pivot frame = the full character matrix (A); add `w34` to the Parachute yaw; SkyDive target orbits with the input offsets (mode 1) and the idle
   recentre (0.1 s delay, 2 s); Full collision spring (k = c = 10, snap-in) and the Parachute fraction law (3.8 in / 0.9 out, floor 0.5, 2.0 dip); input curves of addendum E.
