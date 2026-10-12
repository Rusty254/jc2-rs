# On-foot camera placement and the run pull-back (board c0c2758e)

*11 October 2026, `claude/onfoot-cam`. Retail `JustCause2.exe`; addresses are native VAs; decompiler export is the ignored local
`research/analysis/pseudo/_unnamed`. Follows `parachute-camera-placement.md` (same record, same builder `00838720`, same per-frame `0062be20`).*

## Answer

Standing and running use **one** camera record (`OnFoot`, id 0 of `settings/camera_settings.bin`, type `Normal`) with **one** placement. The difference the
user saw is a runtime filter named by the record: key `0x72c30229` (record `+0x100`) = 5, which `0053e7d0` turns into `CCameraRunFilter`
(constructor `006faa50`, vtable `00eb63c0`, update `007a17f0`). It adds an extra distance `run` to the Z of the camera offset (block `+0x90`, the distance behind) every frame.

| item | value | source |
|---|---|---|
| camera offset | (0.55, 1.55, 3.0) from the character matrix at the feet (node 2), rotated by the orbit (yaw, pitch) | record key `0x80cf2614` |
| target offset | (0.55, 1.55, 0.0), rotated by yaw only | key `0x15ed1fea`, target mode 2 (`00640fd0`, key `0x1acabe1c`, `Normal` default) |
| pivot, smoothing | none | record has no pivot / smoothing keys |
| FOV | 40 degrees | key `0x4995a552` |
| pitch limits | -55 .. +85 degrees | `Normal` default key `0x8fc27e6c` |
| movement class | 1 `CCameraMovementFree` (`0053b930`; update `0063d0f0`, vtable `00eb6420` slot 1) | `Normal` default key `0xf2166f3a`; no yaw limits |
| collision, input | class 4 / input type 2 | keys `0x294585fd`, `0xd3185ed6` (class 4 not decoded) |
| run filter | `run` = 0 standing, 1.5 running, 2.5 dashing | `007a17f0`, constants of `006faa50` |

`CCameraMovementFree` (`0063d0f0`) adds the input deltas to its yaw and pitch (`+8`, `+0xc`; wrapped by 2 pi, clamped to the limits), builds matrix A as `004e1470(yaw, pitch, 0)`
keeping the translation, and, when `00640fd0` succeeds (mode 2: `004e1470(yaw, 0, 0)`), **replaces** matrix B's rotation by it. `00688380` then gives
`camera = A.pos + A.R * (camera offset + (0, 0, run))` and `target = B.pos + B.R * target offset`. So the camera is an orbit about the feet point, 1.55 m up and 3 m behind
(4.5 m running, 5.5 m dashing), and the look target does not follow the pitch. The old project values (`CAMERA_DISTANCE` 4.2 m, target 0.6 m above the capsule centre) were not native.

The second camera/target offsets in the record (`0xcdead79e` = (0, 1.8, 3.0), `0xe2083586` = (0, 1.8, 0); record `+0x90`, `+0x3c`, fallbacks to `+0x84`, `+0x30`) are not read by the state
builder or by `CCameraMovementFree`; the only readers found are the collision classes (e.g. `00841a00` for vehicles, `007312c0`), i.e. they are collision probe origins, not a
second placement. Holstered versus drawn weapon changes no camera field (aim cameras 26 and 3 carry the same offsets as id 0 and differ only in FOV and sensitivity), so the closer framing in the user's
screenshots is the standing placement itself (3 m, 1.55 m), not a holster state.

## The run filter `007a17f0` (native, decoded)

Per frame, with `cur` = filter `+8` (starts 0):

1. `00615e60` is true when the character's foot-machine state is `S_DASH`, `S_DASH_TURN_180` or `S_INSTANT_DASH` (`foot-controller-next.md`, `rico-locomotion-motion.md`): target = filter `+0x10` = `DAT_01066a80` = **2.5**.
2. Otherwise, if the squared length of a character vector (read as the velocity, INFERRED) is greater than filter `+4` = **10.0** (speed above 3.16 m/s): target = filter `+0xc` = `DAT_01066a84` = **1.5**.
3. Otherwise target 0.
4. Rate = `|target - cur|` clamped to **0.2 .. 1.0** per second; `cur = 004f0a30(cur, target, rate, dt)`, a constant-speed approach that does not overshoot; the camera Z offset gets `+ cur` when `cur > 0`.

Timing from that law (unit test `run_filter_blend_times_follow_007a17f0`): 0 to 1.5 m takes 3.11 s in all (0.5 s at 1 m/s, 1.61 s of exponential approach from 1.0 to 0.2 m remaining, then 1 s at 0.2 m/s),
and it is within 0.2 m of the end after 2.11 s. Back to 0 is the same 3.11 s (the law is symmetric). To 2.5 (dash) takes 4.11 s. **The user's 2 s / 3 s estimates are consistent with
this in spirit**: about 2 s to be visually close, about 3 s to settle; the native figure is symmetric at 3.1 s and not 2 s one way and 3 s the other. The asymmetry the user saw can come
from when the speed test flips (the character takes time to speed up and slow down, which moves the moment the target changes), which is not part of the filter. Not hard-coded: the rate law is what is ported.

## Implementation

`gameplay_camera.rs`: `FootRig` / `run_target` / `foot_pose` / `GameplayCamera::foot_placement`; `character.rs::hero_camera` places the camera by it when the active setting is `OnFoot`
and Rico is not downed; the mouse pitch clamp uses the record limits (it was -1.3 .. 0.9). Other states (swim, ragdoll, hang, reeled, stunts) keep the previous orbit. Harness: `--onfoot-capture DIR`.

## Gaps

1. **`CCameraCollision` class 4** (record `+0xbc`; factory `00771930`) is not decoded: the project's ray cast (0.3 m margin, 0.6 m minimum) stands in. Blocks only near-geometry shots.
2. **Sign of the native pitch** against the project's (positive = camera below, looking up) is inferred, as in `parachute-camera-placement.md`; the -55 / +85 limits are applied in that convention.
3. **Which vector `007a17f0` reads** for the speed test (a call the decompiler leaves unresolved, filled via `00634a50` / `009c1500`): read as the full 3D velocity.
4. **Crouch (69), aim (26, 3, 5, 67), ladder, swim, ragdoll, reeled cameras** are separate records; their selection and placement (with the run filter only on `OnFoot`) are not wired to `hero_camera` yet.
5. **Camera position transition blend** between records (`camera_transitions.bin` `pos_blend`) is still not applied (see `parachute-camera-placement.md` gap 5).
