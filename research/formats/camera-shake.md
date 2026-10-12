# Camera shake consumer (task b6762d43, 12 October 2026)

Closes gap 5 of `explosion-effects.md` for the camera. Probe: `research/tools/camera_shake_probe.py` (44 checks, exit 0 with
`--game` and `--pseudo`; the bytes, the disassembly and the decompiler text). Port: `crates/jc2-game/src/effects/camera_shake.rs`.

## 1. Where it runs (proven)

The accumulators (+0x24 intensity, +0x28 speed, +0x2c decay, +0x30/+0x34/+0x38 axis words, always 1.0 after a setter) are on the
`CCameraControlManager` itself: the effect's Start loads the global 0x011900f8 into ECX before calling 0x0070d460, and 0x00698de0
is the same object. The constructor 0x00762eb0 stores vtable 0x00eb8774; **slot 6 is the per-frame update 0x0081c960**. It
runs, in this order: 0x0067c330, 0x0062be20 (builds the final camera matrix for the active mode and blend), 0x005164d0, 0x005a2450,
**0x005e7b90 (the shake)**, 0x006021f0 (clamps the camera height against a floor near y = 200), and copies the 4x4 matrix into the
camera object at +0x24 and +0x264. 0x005e7b90 has exactly one caller (this update), so the shake applies to **every camera mode**,
after the mode blend, and nothing else reads the accumulators. The frame delta is 0x004d3690: the frame time at +0x14, or 1e-6
while its pause flag is set.

## 2. The waveform of 0x005e7b90 (proven)

State: offset S and target T, three floats each in globals (0x011971d8 and 0x011971cc), both zero at start. Per frame, only
while intensity I > 0:

1. If S equals T on all three axes (initially true): draw a new target `T_i = axis_i * I * (rand() * 6.103702e-05 - 1.0)`
   (0x00e2f620; MSVC `rand()` 0..32767, so uniform in [-I, +I]; one `rand()` per axis, x then y then z).
2. Move each axis toward its target with 0x004f0a30 (clamped): `S += (T - S) * min(1, speed / |S - T| * dt)`. This is a constant
   speed of `speed` (+0x28) units per second per axis, stopping at the target. Axes arrive at different times; a new target is drawn
   only when all three have arrived.
3. Rotate the camera matrix by the Euler angles `(S_x, S_y, S_z)` radians through 0x004e1470 (multipliers 1.0 at 0x0106a740..48;
   a = yaw about Y, b = pitch, c = roll; its sine is a parabola with the 0.225 correction, error below 1.2e-3) and multiply with
   0x00491e50 as `R * M` in the row-vector convention, i.e. **a rotation in the camera's own frame**. Then add `S` (the same
   three numbers, now **metres**) to the world position row (matrix +0x30/+0x34/+0x38).
4. Decay: `I = I + decay * dt` (decay is negative: -0.1 for a pulse, -0.6 for the retail rumble_and_shake effects); at I <= 0 it is
   set to 0. S and T are not reset when the shake ends, so the next shake starts from the old offset.

Amplitude, then: a pulse at 10 m or closer sets I = 0.05 (speed 1.5, decay -0.1): up to 0.05 rad (2.9 degrees) and 5 cm,
re-aimed about every 30 ms, dying after 0.5 s. `rumble_and_shake_large` (scale 1, gain 0.08, speed 5, decay -0.6) peaks at
0.08 and lasts 0.13 s unattenuated. Beyond 10 m the setter divides by distance / 10.

## 3. Port notes

`ShakeWave::step` is the function above; `shaken` applies it (engine rows are Bevy columns, `coords::mat_to_bevy`, so `R * M`
becomes `q_cam * q_R`). The Bevy camera is placed by several systems (`hero_camera`, `stunt_camera`, capture drivers), some of which
read their own previous transform, so the shake is applied in `PostUpdate` before transform propagation to the main camera
(`Camera3d` + `PostPass`) and taken off again in `First` (only if nothing else moved the camera meanwhile). Non-native: the
`rand()` seed (a fixed LCG seed; the executable's seed is set elsewhere).

## 4. Gaps

1. **Rumble accumulator reader.** The `CRumbleManager` (global 0x0118fb98, constructor 0x008bdff0, vtable 0x00f0aa70) +0x1c
   accumulator is filled by 0x008e1280 (`+= input * 0.02 / max(1, distance)`), but the function that reads and decays it (and
   forwards to the pad motors) was not traced. Blocks only gamepad force feedback; keyboard and mouse play has no rumble device.
2. **`rand()` seed.** Unknown; the shake direction sequence differs from the original run to run. No visible parity effect.
3. **Frame delta source (+0x14 of the 0x004d3690 object).** Taken as the game's scaled frame time; slow motion is assumed to scale
   it. Not traced.
