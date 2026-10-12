# Rico in the air: the Havok character-context hand-over (JC2-254)

Research only. Probe: `research/tools/jump_context_probe.py` (44 checks, exit 0, needs capstone; `--tool <jc2tool.exe>` also
re-reads the two embedded clip fixtures and the `jump_start` event track from the game files):

```
python research/tools/jump_context_probe.py --game <game folder> [--tool <jc2tool.exe>] [--report]
```

Executable fingerprint: `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Labels:
**native** = read from code/data and asserted by the probe; **inferred** = reasoned from native facts; **unknown** = Gaps.
Closes Gap 1 of `rico-locomotion-motion.md` (JC2-241) except the items under Gaps. Unblocks JC2-238 jump/air parity.

## 1. Answers

* **Jumping follows the animation exactly; InAir keeps it (native).** While the start state plays (`CJumpControllerState`: `S_JUMP_START_STANDING`,
  `S_JUMP_START_SPRINT_FORWARD`, also the stunt and reel starts) the on-foot code holds character `+0x36c` (`wantJump`) at 1. The context
  is in the Jumping state and sets the body velocity to the **desired velocity of the frame** (gain 1.0, max acceleration 100 never clamps),
  on all three axes, so the vertical speed is the clip's root-motion Y speed (0 for the first 0.1 s, then 4.28 m/s running; standing:
  its own profile) and the position follows the clip. When the state exits `+0x36c` is cleared; the next update leaves Jumping for InAir,
  which **keeps the vertical component of the current velocity and adds dt x gravity**, discarding the desired Y entirely.
* **No jump impulse exists.** `jumpHeight` 1.5 (Jumping ctor `0x009c0320`) is never read: update `0x009bfdc0` does not touch `this`, enter `0x009b4f00` only clears `this+0xc`.
* **Gravity** `-21 m/s^2` is in the record as the vector `(0, CPfx+0xbc, 0, 0)` (rec `+0xb0`); rec `+0x88` is the **step dt**, not a gravity scalar (correction of JC2-241 section 7.1).
* **Apex / flight time / landing speed (law, continuous):** with exit speed `v` and clip rise `h`: apex `h + v^2/42`. Running: `1.564 + 4.277^2/42` =
  **2.000 m**; standing: `1.383 + 1.143^2/42` = **1.414 m**. Time from take-off to the take-off height: running 0.467 + 0.204 + 0.436 = **1.107 s**,
  standing 0.5 + 0.054 + 0.367 = **0.921 s**; speed there 9.17 / 7.71 m/s. The discrete model of the state machine (one extra coast update at
  the Jumping -> InAir switch, forward-difference sampling of the clip) gives 2.07 / 1.45 m: the +0.07 / +0.04 m is the unresolved
  sampling and ordering detail (Gap 1), not a different law.
* **Fall speed:** `v = -21 t`, **no terminal velocity** in the character context: InAir has no clamp (no compare, sqrt, min or max instruction in
  `0x00994c40`), and the context's speed cap (`+0x20` = 20 m/s, `+0x1c` = 625, gain `+0x24` = 1) sits behind the filter byte `+0x18`, which the
  constructor `0x00996b00` clears and nothing sets (the only call on the context is `0x009c48a0` from `0x0098fcc2`; no code in
  `0x860000..0x8f0000` / `0x980000..0x9c8000` writes `+0x18/+0x1c/+0x20/+0x24` after loading `[x+0x84]`). 3 s of free fall = 63 m/s. Parachute and
  skydive states are velocity-controlled and are outside this layer.
* **Horizontal speed in the air:** per update `v += 0.05 (desired - v)` (relative to the surface velocity), horizontally only. `desired` is the animation
  root motion plus the lift vector (JC2-241 section 7.3). After n updates `1 - 0.95^n` of the way: 64 % after 20, 95 % after 60. The gain is **per update, not
  per second** (time constant 20 updates; 0.67 s at 30 Hz, 0.33 s at 60 Hz); the update rate is JC2-255. In Jumping (start states) the horizontal
  velocity is the desired velocity itself (gain 1.0), so a sprint jump's lift reaches the body at once and then the InAir law applies.
* **`jump_start` event:** `jump_start.event_track` (t = 0.1 s, one record) is **not** a physics trigger: the string `jump_start` does not occur in the executable, the
  Jumping state has no timer or event input, and the climb is the clip's Y velocity (starts at frame 3-4 = 0.1-0.133 s, which is why the event sits there). The event name is
  looked up as data in the character sound table `event_lookup_table` (`audio.md`: `jump_start` -> `sound_set_id`); delivery is the track consumer `0x0090ba50` -> `0x0049e820` (`animation-next.md`).

## 2. The input record built by `0x0098f960` (native)

`0x0098f960(this = CPfxCharacterInstance, in = character + 0x270 (+0xf0 +0x360), dt)`; called by `0x008672a0` (and `0x00890b90`, `0x008ddb40`). Record base `ebp-0x120`, 16-byte aligned fields:

| Rec offset | Content |
|---|---|
| `+0x00` byte | `wantJump` = `in+0xc` (character `+0x36c`) |
| `+0x10` | desired velocity: `(-in.x, in.y, in.z, 0)` (`-0.0 - x`, `f32[0xe2599c]`) on the ground/air path; if `in+0xd` is set it is rotated by the CPfx orientation (`+0x30`) instead |
| `+0x20` | up = `(0, 1, 0, 0)` |
| `+0x30` | forward = rotation of `(0, 0, -1)` by the orientation quaternion `CPfx+0x30` |
| `+0x40` byte | flying flag = `in+0xd` (character `+0x36d`) |
| `+0x41` byte | **supported** = (`CPfx+0x40` == 2) |
| `+0x50` / `+0x60` | support surface normal (`CPfx+0x50`) / surface velocity (`CPfx+0x60` when supported; else `CPfx+0xc0..` decayed by `exp(-0.1 dt)` per update, `f32[0xe3dfe8]`) |
| `+0x70` | surface type (the ground-stick code skips values 6 and 7) |
| `+0x88` / `+0x8c` | `dt` / `1/dt` |
| `+0x90` | position `CPfx+0xe0` |
| `+0xa0` | **current velocity** = the proxy's velocity (`0x00be2280` = `&proxy+0x10`; the proxy is `CPfx+0x80`) |
| `+0xb0` | gravity vector `(0, CPfx+0xbc, 0, 0)` |

**Support check** (native): at the top of `0x0098f960`, `0x00be53c0(proxy, direction = (-0,-1,-0,-0), out = CPfx+0x40)` fills the surface-info block
(supported state at `+0x40`, 2 = supported, normal `+0x50`, velocity `+0x60`). After the context runs, `0x00be2290` stores the output at `proxy+0x10`
(a plain 16-byte copy, no clamping). The velocity the proxy keeps between updates is therefore exactly what the last update wrote.

## 3. The context driver `0x009c48a0` and the states (native)

Order per update: output := current velocity; state slot `+0x1c` (**change**); if the state still equals the previous update's (`ctx+0x14`), slot `+0x18`
(**update**); `ctx+0x14 := ctx+0x10`; ground-stick (type 1 contexts only; this one is type 0, so not applied); the optional filter (byte `+0x18`, off).
Change slots: OnGround, Jumping, Flying `0x009a0d00` (empty `ret 0xc`); **InAir `0x009b6090`: if supported (rec `+0x41`) leave InAir, state := OnGround; else if the flying flag (rec `+0x40`) is set, state := Flying (JC2-261)**.

| State | Update | Behaviour |
|---|---|---|
| OnGround 0 | `0x009c1f90` | `wantJump` -> state 1 (no velocity written that update); flying flag -> state 4; unsupported -> state 2 (if ctx-state byte `+0x14` is set it first removes a positive up speed); else walks with gain 1.0 |
| Jumping 1 | `0x009bfdc0` | `wantJump` == 0 -> state 2 (no velocity written that update); else `0x009acd10(gain 1.0, max 100, up (0,1,0), desired rec+0x10, surface rec+0x60)` |
| InAir 2 | `0x00994c40` | `0x009acd10(gain 0.05, max 100, up rec+0x20, ...)`, then remove the up part and re-add the up part of rec `+0xa0`, then `+ dt * rec+0xb0` |
| Flying 4 | `0x009970c0` | flying flag clear -> state 2; else output = desired (no gravity) |

`0x009acd10(gain, max, ...)`: `rel = current - surface`, taken into an up-aligned basis; `step = desired - rel`; if `max^2 < gain * |step|^2` the step is
rescaled to length `max / gain`; `new = rel + gain * step`; back-transformed, `+ surface`. It does nothing when the basis cross product is degenerate. With gain 1.0 /
max 100 (Jumping) the result is the desired velocity for any step below 100 m/s; with gain 0.05 the clamp needs a step above 447 m/s, so it never fires in play.

Flag sources: `CJumpControllerState` enter `0x006ae480` / exit `0x0076f9c0` set / clear `+0x36c` (vtable `0xeb653c` slots `0xeb656c` / `0xeb6574`; the exit also sets controller `+0x114` = 1 and adds a random x 0.35 to `+0x108`,
role unknown). `CVelocityControlledState` / `CVelocityTimeVariableState` set `+0x36d` (JC2-241). `S_JUMP_FLIGHT_*` are `CRuleState`, `S_JUMP_TO_FALL_TRANSITION` is `CState`, `S_FALL` is `CFallState`:
none sets either flag, so a normal jump stays in InAir from the start state's exit to landing. **Correction (JC2-261, `parachute-state-velocities.md`): Flying IS reachable from InAir.** The InAir change slot `0x009b6090` ends, for an unsupported body with the flying flag (rec `+0x40`) set, with set-state(4) (`push 4; call 0x009934e0` at `0x009b60ed`); the skydive, chute and base-jump states set the flag, so an airborne body in those states is in Flying (output = desired, no gravity). A normal jump is unaffected.

## 4. Jump timeline (native structure, sampling inferred)

1. OnGround, `wantJump` set at the start state's enter: first update switches to Jumping (velocity unchanged).
2. Jumping for the clip (0.467 s running / 0.5 s standing): velocity = desired. Y = 0 for 0.1 s (the character is held on the ground, **no gravity applies in Jumping**), then the clip's speed.
   The standing clip first dips to -0.035 m and returns (frames 1-3) before the climb; the body follows it.
3. Exit (`C_WAIT_FOR_ANIM_TO_FINISH`): `+0x36c` cleared; one update switches to InAir leaving the velocity as it was (a one-update coast); then InAir: `vy -= 21 dt` per update, horizontal gain 0.05.
4. Landing: the first update after the support check reports supported -> OnGround (`0x009b6090`); the landing animation states (JC2-241 section 7.3 / `animation-next.md`) are separate.

## 5. Runtime guidance for JC2-238

* Drive the vertical take-off from the start clip's Y root motion at the update rate; no impulse, no gravity until the start state exits; then `vy` keeps the last value and `-21 dt` per update, no cap.
* Horizontal in the air: `v_h += 0.05 (desired_h - v_h)` per update with `desired_h` = root-motion horizontal + lift (JC2-241 7.3); in the start states use `desired_h` directly.
* Ignore any Y part of the lift and of flight-clip root motion while InAir.
* Switch back to ground handling on the first update the support probe (straight down) reports supported.

## Gaps (unresolved native details)

1. **Proxy integration and sampling order.** Whether `hkpCharacterProxy` integration (`0x00b7f820` family, driven after `0x0098f960`) moves by the velocity written this update or by its collision-adjusted
   value, whether it alters `proxy+0x10`, and which clip sample (frame start, end or interpolated) is the desired Y of an update. Blocks the last 0.07 m / 0.04 m of apex and one-frame timing parity.
2. **Update rate** of `0x008672a0` (the `dt` and the gain 0.05 per update): JC2-255 (on-foot controller tick). Blocks the real-time horizontal air time constant.
3. **OnGround leave-ground rule** (ctx-state byte `+0x14` removes a positive up speed when walking off a ledge) and the OnGround walking branch; not needed for jumps, needed for ledge/walk-off parity.
4. **Fall states beyond the context:** what `C_IN_AIR`, `S_FALL` -> `S_FALL_TO_SKYDIVE` and the heavy-landing test (`T_JUMP_LAND_HEAVY`) read (fall speed, fall time) was not decoded here.
5. **Exit side effects of `CJumpControllerState`** (controller `+0xa34`: `+0x114` = 1, `+0x108 += random x 0.35`, enter clears `+0x3c/+0x40/+0x10c/+0x110/+0x118`): role unknown.
6. **`jump_start` sound hookup** is taken from `audio.md` (data table); the callback registration itself was not traced.
