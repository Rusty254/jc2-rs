# Parachute body orientation: the character matrix in freefall and under the canopy (board task 9cfa8abf, "JC2-270" body-orientation gap, G13 of `parachute-parity.md`)

Research only. Every address below is asserted by `research/tools/parachute_body_orientation_probe.py` (32 checks: function hashes, instruction anchors, constants,
a pure-Python model of the Euler extraction, retail clip values read through `jc2tool anim`) unless it is marked "inferred" or "not probe-asserted".
Executable fingerprint: `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.

## 1. Answer in one page

* **There is one orientation, a full 3x3, and the whole model turns with it about the character origin.** No bone carries the lean. The retail clips used in freefall and under
  the canopy (`mhfreefall`, `rico_speed_tb_up`, `rico_open_chute`) have **no root motion and a static identity Reference bone**, so the pose is evaluated in model space
  and the world matrix (`character + 0x15c`, entity `+0x6c`) is applied on top of it. The canopy copies the same matrix (`parachute-attach-pivot.md`), so body and canopy
  tilt together.
* **What the clip does and what the matrix does.** `mhfreefall` lays Rico **prone in model space** (Hips rotated 90 degrees about -X, hips only 0.136 m above the origin).
  The freefall model's matrix then adds the pitch and bank: pitch settles at `R2.y = -(5/3)(P - 0.5)`, so at the dive pose `P = 0` the body is pitched
  **56.44 degrees head down** (`asin(5/6)`), at `P = 1` 56.44 degrees nose up, and a full roll settles at a bank of 41.8 degrees. `rico_open_chute` starts on exactly that prone
  frame and stands the body up in model space within about 0.6 s (hips 0.15 m at t = 0.6 s, 1.14 m at t = 1.2 s, 0.885 m at the end); the **matrix keeps its dive pitch** into the canopy
  and the flight spring then unwinds it.
* **The opening frame.** The first canopy flight frame takes the character matrix (a copy of `character + 0x15c`) and converts it with `0x004de960` into the flight
  block angles: **pitch `a30 = asin(-R2.y)`, yaw `a34 = atan2(R2.x, R2.z)`, bank `a38 = atan2(R0.y, R1.y)`**, exactly the inverse of `M = Rz(a38) * Rx(a30) * Ry(a34)`. A canopy
  opened from the dive therefore starts at about `a30 = -0.985` (head down), not at zero; rates start at zero.
* **The matrix at that frame is the freefall model's, not an upright one.** The freefall model `0x006b9290` has no state test and its caller (the skydive controller update
  `0x00619510`) also serves `S_PULL_OPEN_PARACHUTE_VERTICAL`; the canopy controller (`S_UNFOLD_*`, `S_PARACHUTE`) takes over at the end of that state. So the freefall orientation
  keeps updating (pose `P`/`R` from the stick) through the whole chord-pull clip and is what the flight model reads.
* **Under the canopy the flight model writes the matrix every frame:** `M` goes matrix -> quaternion -> normalise -> matrix into the canopy transform (an identity round trip for an
  orthonormal `M`), translation from the character copy, and then the character entity is written: **matrix and quaternion when the layer-0 state is `S_PARACHUTE`, only the
  quaternion when it is an unfold state** (section 4).
* **Runtime today:** `Hero::facing` yaw only on the body, an invented 0.35 rad forward tilt under the canopy (`character.rs`), and `CanopyFlight::open(yaw)` starting all angles at zero.
  The paired implementation task replaces all three (section 6).

## 2. The Euler extraction `0x004de960` (native)

Input `this` = a 4x4 matrix (rows `R0` = floats 0..2, `R1` = 4..6, `R2` = 8..10); outputs through three pointers (pitch, yaw, bank). Normal arm (always taken for a rotation matrix):

| output | formula | bytes |
|---|---|---|
| pitch | `asin(clamp(-R2.y, -1, 1))` (`t = -0.0 - m[9]`) | `0x4de974`, clamp with `-1`/`1` at `0xe24e78`/`0xe20668` |
| yaw | `atan2(R2.x, R2.z)` = `atan2(m[8], m[10])` (y = `xmm0` = `[esi+0x20]`) | `0x4de9c9`, `0x4de9ce` |
| bank | `atan2(R0.y, R1.y)` = `atan2(m[1], m[5])` | `0x4dea25`, `0x4dea33` |

This is the exact inverse of the flight basis: with row vectors, `R2 = (cos a30 sin a34, -sin a30, cos a30 cos a34)` and `R0.y / R1.y = tan a38`; the probe round-trips 1,050 triples to
1e-9. The runtime's `Skydive::yaw()` (`atan2(R2.x, R2.z)`) already uses the same yaw convention.

**A native oddity worth keeping:** the "gimbal" arm (pitch = `t * pi/2`, yaw = `atan2(-R0.z, R0.x)`, bank 0) is guarded by `|t| > 9.99999` (the float at `0x00e30954`; 0.99999 may have been intended, unproven). For a rotation matrix `|t| <= 1`, so the arm is **dead code**; at a pitch of exactly +-90 degrees the normal arm returns the clamped pitch and `atan2(0, 0)` terms. The runtime must not
add a gimbal special case.

Fixtures (probe): terminal dive `P = 0` (`R2.y = 5/6`): `a30 = -0.98511` rad (-56.44 degrees), `a38 = 0`; full flare `P = 1`: +56.44 degrees; level body with `R0.y = 2/3` (the steady state of a full roll,
`th_r = 0` at `R0.y = R/1.5`): bank 41.81 degrees.

## 3. The opening frame in the flight model `0x005a9950` (native)

`CParachuteController` update (vtable `0x00ec24ac`). At `0x005a9a03` the character matrix is copied (`lea ecx,[edi+0x15c]`, copy-constructor `0x004541f0` into a local); `0x005a9a53` calls `0x005c36a0`
on the canopy (read-and-clear of the byte at canopy `+0x49`, the opening flag); if it was set, `0x005a9a72` calls `0x004de960` on the copy with the three output pointers `D+0x30` (pitch), `D+0x34`
(yaw), `D+0x38` (bank), `D = [S+0x170]`. `0x005a9aa3` then clears the rates `D+0x3c..+0x44`, and the existing opening velocity transform follows (`foot-controller-next.md` section 8).
The flag is consumed on the **first flight-model frame of the canopy controller**, i.e. when the machine enters `S_UNFOLD_PARACHUTE_*`, not at the chord pull.

## 4. The per-frame write (native)

After the spring has updated the angles (`0x005ab3bc`): `Rx(D+0x30)` (`0x4dbce0`), `Ry(D+0x34)` (`0x4eaad0`), `Rz(D+0x38)` (`0x4f1790`), two products (`0x491e50`) give `M`, which is
converted to a quaternion (`0x4d3060`, standard trace branch), normalised (`0x4f1600`) and turned back into a matrix by `0x4d5910` **into the canopy transform at `canopy + 8`**; the canopy
translation (`+0x30/+0x34/+0x38`) is then set from the translation of the character-matrix copy. Next, `0x00849700` (layer-0 state equals `S_PARACHUTE`, global `0x011965ac`) selects the character write:

| layer-0 state | write | effect |
|---|---|---|
| `S_PARACHUTE` | `0x008812b0(canopy + 8, 1)` | entity matrix `+0x6c` := the matrix with the translation row restored from entity `+0x9c/+0xa0/+0xa4`, and entity quaternion `+0x12c` := matrix -> quaternion |
| `S_UNFOLD_PARACHUTE_VERTICAL/HORIZONTAL` | `0x008f6480(&quaternion)` | entity quaternion `+0x12c` := the normalised quaternion, dirty bit `|= 8` at `+0x268`; **entity `+0x6c` is not written here** |

Consequence (inferred, Gap 2): during the unfold the canopy, which reads `character + 0x15c` every frame, only turns if something copies the quaternion into the matrix; the
the runtime should apply `M` every frame in all three canopy states (inferred: equal in effect once the quaternion has been synchronised).

`0x008812b0(matrix, flag)` is also the freefall model's write (`0x006ba139`). The freefall matrix is `Rx(th_p) * Rz(th_r) * (C * Ry(th_y))` (`parachute-canopy-landing-next.md` section 7), where `C` is the
previous character matrix.

## 5. Where the freefall model runs (native)

`0x00619510` (the `CSkyDiveController` update) calls the open/close driver `0x0074edd0`, the pose steps `0x00517090` and `0x007687c0`, and then the freefall model `0x006b9290`, in that order with no
state test. The model itself (`0x006b9290..0x006ba5be`) contains **no reference to a foot-machine state global**. The player's controller registration (`foot-conditions-next.md` section 4) assigns
`CSkyDiveController` to `S_FALL_TO_SKYDIVE`, `S_SKYDIVE`, `S_PULL_OPEN_PARACHUTE_VERTICAL`, `S_PARACHUTE_DETACH_TO_FREEFALL` and the three skydive-dash states; `S_JUMP_TO_SKYDIVE` belongs to the base-jump controller.
So the freefall orientation, pitch, bank and the world-yaw roll turn run in all seven states, including the chord pull, and the matrix the canopy reads at its opening frame is the result of those steps. The runtime
runs the model only while `hero.skydiving && hero.free_air` (`character.rs`), which is narrower.

## 6. Runtime guidance (the paired implementation task)

1. **Matrix to body.** Row-vector convention as `parachute_flight::Rows` (`R0` = image of local x, `R1` of local y, `R2` of local z; model forward is -z, so `R2` is the back axis). The body
   rotation is `Mat3::from_cols(R0, R1, R2)` as a quaternion, applied to the Rico model **about its own origin** (the feet, where the model is already placed): the child's translation stays `(0, -HEIGHT/2, 0)`, only
   its rotation changes. Freefall: `Skydive::matrix`. Canopy: `CanopyFlight::matrix()` (native `M`). Delete the 0.35 rad tilt; keep the yaw-only path for every other state. The clips already carry the prone
   and upright poses (section 1), so **no extra lean is added on top of them**.
2. **Opening.** `CanopyFlight::open_from_matrix(&Rows)` with `a30 = asin(clamp(-R2.y))`, `a34 = atan2(R2.x, R2.z)`, `a38 = atan2(R0.y, R1.y)`, no gimbal arm; rates zero. When the canopy opens from the
   dive, pass `hero.skydive`'s matrix; otherwise (open from a state the freefall controller does not serve) pass `Ry(facing)` (inferred: the on-foot matrix is yaw only).
3. **Freefall model span.** Run `Skydive::step` in all seven skydive-controller states (at least through `S_PULL_OPEN_PARACHUTE_VERTICAL`, which matters most), carrying the matrix into the canopy.
4. **Test fixtures:** dive matrix to `a30 = -0.98511`; open then step: `a30` leaves -0.985 towards the spring's fixed point instead of starting at 0; round trip `flight_matrix -> euler -> flight_matrix`.
5. `S+4` (the signed camera yaw feeding the canopy turn term) is **already resolved** by JC2-262 section 10.5 and implemented (d5c39f26); nothing remains of that part of the original gap text.

## 7. Correction to an earlier claim

`parachute-attach-pivot.md` section 8 and its probe (`parachute_attach_gaps_probe.py`) list `0x8c4257 -> 0x875790` as a "scene proxy bounds" use of `character + 0x15c`. The function containing that call
(`0x008c3d80`, pseudo-C in `research/analysis/pseudo`) is a different class: it has its own state field at `+0xa8` (0..4), a position at `+0x9c`, a model at `+0x88` and a matrix at `+0x15c`, and tests an area-of-operations
interface; it is **not shown to be the character** and cannot be used as evidence for the body mesh path. (This is a reading of the pseudo-C, not probe-asserted.) Only `0x5ceee0 -> 0x4a7820` (the physics
body) remains as a proven second consumer of `character + 0x15c`.

## Gaps

1. **The instruction that gives the visible body mesh its world matrix** (still open; board task b67acd7f). Proven here: the clips carry no root rotation and the freefall hips (`mhfreefall`, 90 degrees about -X,
   hips 0.136 m) only make sense with a world matrix applied to the whole model about the origin; the pose submission (`0x008ba220`, `0x004c7270`) carries no matrix. The "model uses `character + 0x15c`" statement stays
   inferred (strongly). Blocks nothing in section 6, which applies the matrix to the model as a whole.
2. **Who copies entity quaternion `+0x12c` into the matrix `+0x6c` during the unfold states** (the non-`S_PARACHUTE` branch of section 4, `0x008f6480`), and whether it happens the same frame. Blocks the exact body and canopy
   orientation during the 2.4 s unfold (visible), and the exact value of the matrix on the frame the machine enters `S_PARACHUTE`.
3. **Who levels the matrix after the canopy closes or Rico lands** (pitch and bank back to a yaw-only matrix): no writer of entity `+0x6c` with an upright matrix was located for the player (`0x8812b0` callers:
   `0x5780d5`, `0x586d0e`, `0x586fe0`, `0x605563`, `0x643024`, `0x756337`, `0x815af4` (a civilian component), `0x6ba139`, `0x5ab52d`). Blocks the exact end of the lean (visible at the cut-away and the landing).
4. **Character origin against the physics body** (also gap 4 of `parachute-attach-pivot.md`): whether the prone freefall body is centred on the native physics body, which decides camera and collision placement while prone. Blocks
   nothing in the model rotation; relevant to the runtime's capsule.
