# Grapple reel flight tick, contact sweep and reel availability terms (JC2-246)

Research brief for the Rico reel/pull flight (JC2-039, JC2-238). Closes Gaps 2 and 3 of
`grapple-action-selection-next.md` (JC2-168/169) as far as the code allows. Nothing here changes
gameplay. `research/tools/grapple_reel_flight_probe.py` pins the bytes of 26 functions, 22
direct call edges, 21 `.rdata` constants (each also found inside the function that loads it),
the device constructor defaults, the instruction patterns the decode relies on, reads
`settings/stateruleflags.bin` and the Rico animation machines, ports the rules and runs 59
boundary fixtures. The ports are transcriptions of the disassembly (and the pseudo-C in
`research/analysis`), not runtime captures. `this` of the tick, the sweep and the init
functions is the grappling device (`CGrapplingDevice`, constructor `0x005fdbc0`).

## Headline

* **Speed ramp.** Each frame `v = (3.5 * dt + 1) * v` (`0x0106d8ac` = 3.5 per second), then
  `v = min(v, reel_in_speed)` (`device+0xe0` = 40, grapplingparams `#537a05d4`). The start
  speed is **11.0** for the reel (`device+0x38`, copied into `+0x18` by the reel init
  `0x008233c0`, called from the reel update case 0) and **6.0** for the air pull (`device+0x3c`,
  pull init `0x005ea6e0`). At 60 Hz the cap is reached after 23 frames (reel) or 34 frames (pull).
  Nothing else limits it: there is no acceleration constant such as the Rust `REEL_ACCEL = 45`.
* **Rico is moved kinematically.** The tick sets the owner's desired velocity to `speed * dir`
  (`0x00867fa0` stores it at `controller+0x1c0..0x1c8` of the object at `character+0xf0`);
  `dir` is the unit vector from the wire's arm point (record `0x0057e930`, the last 0x5c-byte
  record) to the hook end (record `count-2`), both through `0x005eb3f0` (world point).
* **Result codes** (`device+0xc`, returned by `0x00515cb0`): 1 flying, 2 arrived, 3 ended
  (the reel update then does `ACT_DETACH`). 3 is also returned when the wire has fewer than
  two end records or the owner reference is dead. A frame with `dt < 1e-6` (`0x00eba07c`,
  also what a paused clock returns) returns 1 and changes nothing.
* **Arrival has three sources**: the sweep reports 2 (a contact that was re-anchored), the tick
  overshoots the hook (`remaining < 0`: velocity becomes `dist / dt`, i.e. it lands exactly on
  the hook), or the sweep's own rules say 3 (ended).
* **`CanReelInState`** is the state flag `0x8000` tested by `0x00575950`: reel is available
  only while the current state of every animation layer carries it. Tabulated below.

## 1. Tick `0x00515cb0` (1083 bytes)

Per frame, in order (all offsets are device fields):

1. Return 3 unless the wire (`+0x128`) has at least two 0x5c-byte end records and the owner
   weak reference (`+0x1f4/+0x1f8`) is alive.
2. `dt = 0x004d3690(0)` (frame step; 1e-6 when the clock is paused). `dt < 1e-6` -> return 1.
3. `dir = unit(hook point - arm point)` (`0x00467ee0`; vectors of squared length `<= 1e-6` stay
   unnormalised); `dist = |hook point - arm point|` is stored at `+0x14` **before** the move.
4. `+0x18 = min((3.5*dt + 1) * +0x18, +0xe0)`.
5. `char+0xa34`'s `+0x2bc` (an animation parameter of the controller object) receives
   `0x0052e610 * +0x1c`, where `0x0052e610 = clamp(1 - +0x14 * +0x10, 0, 1)` and
   `+0x10 = 1 / start distance` (set by the inits): the reel *progress* 0..1 times `+0x1c`.
   `+0x1c` has no writer (see Gaps), so the value is presumably an authoring/blend scale.
6. `velocity = +0x18 * dir`; `+0xc = 1`.
7. **Moving anchor.** If the hook end's game object reference is alive (`0x006dfca0` false) the
   physics point velocity `n = v_body + w x (p - com)` of the hooked rigid body at the hook
   point (`0x0099a5f0`/`0x004cc700` over the `CPfxRigidBody`) is read; if `|n|^2 > 0` then
   `velocity += dir * max(0, n . dir)` and `+0x18 = |velocity|` (= old speed plus the positive
   part). A hook on a prop or vehicle that flies away therefore speeds the reel up; one that
   moves toward Rico changes nothing.
8. `remaining = +0x14 - +0x18 * dt` (distance left after this step) and
   `+0xc = sweep(remaining, &dir, dt)` (section 2).
9. If the sweep said 1: when `Rico controller +0x454` (byte, section 3) is non-zero and
   `+0x50 > remaining` -> **3**; else if `remaining < 0` -> velocity := `dir * (+0x14 / dt)` and
   **2**. (`+0x50` = 1.4 m, constructor default `0x00e2ccf0`; no loader key writes it.)
10. Always: the velocity is stored (`0x00867fa0`), `0x005a38c0(dir, dt)` (owner aim/orient
    helper, `0x008aa820`), `0x007518e0(dt)` and `0x00648f70` (per-frame presentation effects)
    run, and `+0xc` is returned.

Constructor defaults (`0x005fdbc0`, pinned by the probe; none of them is a grapplingparams key):
`+0x20 = 1.2`, `+0x24 = 0`, `+0x28 = 0.35`, `+0x2c = 10.0`, `+0x30 = 0`, `+0x34 = 0.2`,
`+0x38 = 11.0`, `+0x3c = 6.0`, `+0x40 = 1.0`, `+0x4c = 0.2`, `+0x50 = 1.4`. The hook hit
(`0x00538470`, tail) resets `+0x0c/+0x14/+0x24/+0x30/+0x48` to 0 and `+0x10` to 1.0 on every
attach; it does not touch `+0x18`. The reel init (`0x008233c0`) also stores the start distance
(`+0x14`, `+0x10 = 1/dist`), the target point (`+0x54..+0x5c` = hook point + normal * 1.1 and
0.4 up, or normal * 1.5 for a ceiling-like end, `0x0106d184/0d180/0d188`) and its class (`+0x64`).

## 2. Contact sweep `0x00722cc0` (1914 bytes): what 1, 2 and 3 mean

Arguments: `remaining`, `dir`, `dt`. Default return 1.

1. **Stop gate.** If `remaining < device+0x50 (1.4)` and `dir . hit_normal(first end) > 0.71`
   (`0x00e4d888`; `0x007fddd0`, default `(0,1,0)`) -> **3**. Meaning: Rico is inside 1.4 m of a
   hook whose surface faces away from the approach (he would pass through the surface).
2. `vel = character +0x2bc..+0x2c4` normalised (left unchanged when `|vel|^2 <= 1e-6`).
3. **Branch select** (`device+0x2c = 10.0` against `character+0x2c8`, opaque):
   * **A, `+0x2c <= char+0x2c8`:** if `vel . dir < 0.71` the timer `+0x30 += dt`; once
     `+0x30 > +0x34 (0.2 s)` -> **2** if any current contact can be re-anchored (`0x005c4ec0`
     true for one of them) else **3**. A velocity aligned with the hook direction accumulates
     nothing; a (near) zero velocity counts as misaligned. The timer is never reset in flight
     (only by the hook hit). Otherwise 1.
   * **B, `+0x2c > char+0x2c8`:** the owner's contact list (`character+0x230` ->`+0x80` ->
     `0x00be2230`, records of 0x50 bytes, normal at `+0x10..+0x18`) is bucketed by the
     normal's Y with the device cosines (`0x00714ad0`: 0 floor-like `y >= cos 50`, 1 wall-like,
     2 ceiling-like `y <= cos 140`). Then, in this order:
     a. a bucket-0 contact with `dir . normal < -0.71` (`0x0106d868`) -> **2** if the first
        end's own normal is floor-like, else **3**;
     b. each bucket-1 contact through `0x007e6160`, each bucket-2 contact through `0x0074d690`
        (identical logic and constants): if `dir . normal < -0.71` and the probe `0x006ed690`
        fails -> 3 when `device+0x14 <= 0.6` else 1; probe succeeds and the contact record's
        object reference passes the (opaque) body test -> **2**; else 1. A handler result other
        than 1 is returned at once; result 2 first calls `0x005d83d0`, which **re-anchors the
        hook at the contact point** through the normal hit path `0x00538470` (new end record
        with the contact normal);
     c. the slow timer `+0x24 += dt`; once `> +0x28 (0.35 s)` -> 2 if any contact can be
        re-anchored (`0x005c4ec0`) else 3. Never reset in flight.
4. Result 2 from the sweep is followed by the reel update's `0x00549490` classification of the
   (possibly new) end normal: floor -> `ACT_REEL_LAND`, wall -> `ACT_REEL_IMPACT_VERTICAL`,
   ceiling -> `ACT_REEL_IMPACT_HORIZONTAL` (JC2-169 section 5). Result 3 -> `ACT_DETACH`.

## 3. Stop-short flag `Rico controller +0x454`

Written in the hook hit tail (`0x00538470`): when the attach path took its valid branch, the owner
has health (`+0xe8 > 0`) and a wire exists: `flag = (0x00649aa0(hit) == 0)`. `0x00649aa0`
classifies the end normal: **floor-like -> returns 1 -> flag false** (the reel always lands by
overshoot or contact); wall-like -> `0x0065af70`; ceiling-like -> `0x006f3ae0` (not decoded).
`0x0065af70` returns 0 whenever the hook point is not above 200 m; above it, 1 if one of two
probes (`0x009b9400`, a character ray cast) hits. So a wall hook below 200 m sets the flag, and
Rico's reel ends (result 3, detach) as soon as he is within 1.4 m of the hook without having
produced a sweep result. Whether the shipped game relies on this for walls is unverified; the code
path is exact, the intent is not. No other writer of the flag was found.

## 4. Availability terms of `CReelInAction` (`0x005c2df0`), resolved

| Term | Result |
|---|---|
| `0x00575950` flag `0x8000` | Loops `character+0x46c` animation layers (array `+0x48c`, stride 8) and requires `state.flags & 0x8000` for each current state (`0x0063b5c0` -> `0x006b40b0`). The 32 flag names are read from `settings/stateruleflags.bin` by `0x004c8a20` (`Flag0..Flag31`); **Flag15 = `CanReelInState`**, default true, and every state of the Rico machines overrides it explicitly (`0x004ac900` builds the mask from the per-state keys). Table in section 5. |
| `character+0x324 != 0` -> false | Zeroed by the `CCharacterBase` constructor (`0x008d2f50`) and the reset block of `0x008d3d90`. No `CCharacter`-level writer of a non-zero value was isolated (the other `+0x324` writers belong to the CPlayer constructor `0x007359b0`, which stores 5 in its own layout, and to the `character+0xa34` controller). Treat as 0. Unresolved. |
| `0x0099a440` | Returns `[x+0x40] == 2` where `x = character+0x230`, the physics character body whose contact list the sweep reads (`+0x80`). 2 is the in-air value of the Havok character state enumeration (on ground 0, jumping 1, in air 2, climbing 3); the numbering is inferred, no name string was found. Used by `0x0088b250` as "or" with `0x009c2c30`. |
| `device+0x40 <= character+0x2c8` (parachute only) | `device+0x40` is 1.0 from the constructor and has **no other writer**. `character+0x2c8` is a float next to the vec3 at `character+0x2bc..+0x2c4` that the sweep also reads (branch select, section 2); its writer on `CCharacter` was not isolated (many classes use that displacement). So while parachuting the reel is refused unless `1.0 > character+0x2c8`. Meaning of the quantity: unresolved. |
| `device+0x168` | Cleared in the constructor, in the hook hit (`0x005389e6`) and in the cancel (`0x005fbdfb`); a whole-image scan of every `c6`/`88` byte store with displacement `0x168` finds no other device writer (the three `mov byte [..+0x168], 1` sites are in the Rico controller object at `character+0xa34`, `0x00586b60` and `0x0073e2d0`, and in an unrelated class, `0x005ca190`). `0x00751860` (`wire && device+0x168`) is therefore always false in the shipped code (assuming no copy overwrites it), so a hook on the vehicle Rico stands on is always cancelled when FIRE_GRAPPLE is not held. |
| owner of `0x0099a440`, `character+0x2c8`, `+0x324` | see rows above. |

## 5. `CanReelInState` per state (`stateruleflags` bit 15)

(`reel_available`'s first term: every layer's current state must be in the true set.)

| Machine | true | false |
|---|---|---|
| `basestatemachine` | 99 of 272, including `S_FALL`, `S_JUMP_FLIGHT_STANDING`, `S_JUMP_FLIGHT_SPRINT_FORWARD`, `S_SKYDIVE`, `S_PARACHUTE`, `S_UNFOLD_PARACHUTE_*`, `S_PULL_IN_AIR_FLIGHT`, `S_REEL_FLIGHT`, `S_REEL_START`, `S_REEL_END_LAND`, `S_REEL_IMPACT_VERTICAL`, `S_REELED_IN_IDLE`, `S_REELED_IN_JUMP`, `S_REELED_IN_RELEASE(_DROP)`, `S_REELED_FIRE_HOOK`, `S_REELED_WAIT_FOR_HOOK`, `S_HANG_FIRE_HOOK`, `S_UPRIGHT_*` idle/navigation/turn/strafe states, `S_SWIM_*`, cover states, `S_DEFAULT_STUNTPOSITION_*` | 173, including `S_HANGSTUNT_IDLE`, `S_REELED_IN_TO_HANG`, `S_HANG_TO_FALL`, `S_REEL_TO_SO`, `S_DASH`, `S_SUPER_STUNT_JUMP_START/FLIGHT`, `S_CLING_*`, ladder, vehicle enter/exit, hit reactions, struggle/victim, `S_LANDING_HARD`, `S_JUMP_RUNNING_LAND` |
| `upperbodystatemachine` | all 28 | none |
| `leftarmstatemachine` | 21 of 28 (incl. `LA_S_GRAPPLE`, `LA_S_HOOK_HIT`, `LA_S_IDLE`) | `LA_S_GRAPPLE_PULL`, `LA_S_GRAPPLE_REEL_OUT`, beacon states, `LA_S_DROP_WEAPON`, `LA_S_PLACE_TRIGGERED_EXPLOSIVE` |
| `additivestatemachine` | 1 of 1 | none |

Consequences: a new reel cannot start from a hang or from the end of one (`S_REELED_IN_TO_HANG`),
during a dash, a super stunt jump start, climbing/ladders, a hard landing, and **not while the
left arm is in the ground pull** (`LA_S_GRAPPLE_PULL`). Which machines are Rico's layers at
runtime (`character+0x46c` count) is inferred as the four machines above; the layer count itself
was not read.

## 6. What a Rust implementer needs

1. Replace the `REEL_ACCEL`/`REEL_BRAKE` model for the reel and the air pull by the native ramp:
   `v0 = 11` (reel) or `6` (pull), `v = min(v * (1 + 3.5 dt), 40)`, velocity `= v * dir` toward the
   hook from the arm point, no braking. Arrival is `remaining = dist - v dt < 0` (land exactly on the
   hook with `dist / dt`) or a sweep result.
2. Implement the sweep as listed in section 2 with the contact set of the character body; the
   opaque gate (`character+0x2c8`) may be wired to "branch B" until it is decoded. The
   re-anchor path creates a new end record at the contact point with the contact normal, so the
   arrival classification (floor / wall / ceiling) uses the contact normal.
3. Add the per-state `CanReelInState` table from section 5 to `reel_available` (probe table is
   data-driven from the shipped `.afsm` files, so read it from the game, not from constants).
4. Keep `device+0x168` false and `character+0x324` zero until a writer is found.
5. Do not invent the unresolved pieces: the moving-anchor term needs the hooked body's point
   velocity (props/vehicles); static terrain contributes nothing.

## Gaps (unresolved native details)

1. **Contact re-anchor test `0x005c4ec0` and the wall/ceiling handlers `0x007e6160/0x0074d690`:**
   the ray probe `0x006ed690` (origin, direction, length), the contact record's object test
   (`0x004b2b50`, `0x00495e80`, the isA at vtable `+8` and the virtual at `+0x48 == 2`) and the
   0x50-byte contact record layout beyond position `+0x00` and normal `+0x10`. Blocks exact
   wall/ceiling arrival parity (which contacts re-anchor and which end the reel).
2. **`character+0x2c8` and `character+0x2bc..+0x2c4` meaning and writers** (the branch A/B gate
   of the sweep and the parachute reel refusal). Without it the sweep branch cannot be chosen.
3. **Stop-short flag details:** `0x006f3ae0` (ceiling-class test) and the exact meaning of the
   `0x0065af70` probes; whether the result-3 detach at 1.4 m is really the shipped wall behaviour.
4. **`device+0x1c`** (the multiplier of the progress parameter) has no writer found; `0x007518e0`,
   `0x005a38c0` / `0x008aa820` and `0x00648f70` (owner orientation and per-frame effects of the
   tick) are not decoded.
5. **`character+0x324` (CCharacter)** writer; `0x0099a440` state numbering is inferred.
6. Layer set of `0x00575950` at runtime (count and which machines).
