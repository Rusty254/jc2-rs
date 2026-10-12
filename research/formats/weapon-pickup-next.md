# Weapon pickup, slot assignment, stow gates and hand IK (JC2-104)

Research only, 11 October 2026. Continues `weapon-ownership-next.md` (JC2-092) and
`weapon-sockets-next.md` (JC2-017); read those first. No runtime, Cargo or asset work was
done. Addresses are retail `JustCause2.exe` (image base 0x00400000), SHA-256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Probe:
`research/tools/weapon_pickup_probe.py` (reads only the executable; 70 checks; exit 0).

Evidence tags: **[V]** verified against executable bytes by the probe, **[D]** read from the
decompiler listing but not byte-checked, **[I]** inferred, **[?]** unresolved.

## 1. Pickup assigns a slot by category (answers "pickup-slot-assignment")

The pickup is the animation-driven action class **CWeaponPickupAction** (vtable 0x00ec2f0c;
slot 6 = 0x006f91e0, slot 7 = 0x00732d20 "update", slot 8 = 0x00541590 "end", slot 20 =
0x006d49b0, slot 21 = 0x00507a60 "cancel/start reset"; [V]). Its dword `+0xcc` is the chosen
holder slot (5 = none yet; the cancel paths write 5), dword `+0x34*4`=`+0xd0` is the mode, bytes
`+0x30..+0x33` are flags.

**Offer (state 1 tail of 0x00732d20).** The character keeps a bit mask at `character+0x30c`
(set by 0x00694b50(cat): `mask |= 1 << cat`; the first non-5 bit while the mask was empty
raises the UI event `gui_show_weapon_pickup_menu`; tested by 0x0071f1b0(cat)). The categories
offered for the weapon on the ground (id from 0x00609d00, two-handed flag byte `settings+0x1c9`
from 0x00833360, type pointer `[settings+0]` from 0x0076f9b0) are: [V call sites]

| Weapon | Categories offered |
| --- | --- |
| id 26 (the id-26 domain of weapon-ownership-next.md section 3) | 5 |
| two-handed (`+0x1c9`) | 2 |
| one-handed | 1 if holder slot 1 is empty **or holds a different type**; 0 if slot 0 is empty or holds a different type |

So a second pistol of the same type as slot 1 is offered slot 0 only, and a third identical
one is offered nothing.

**Choose (state 2).** The pickup button is input action 0x94 (record fetched with
0x00984140; record type = `byte[+3] & 7`, value float at `+4`; hold threshold float
`[0x01079aa4]` = 0.35 [V]). Mapping category to the stored slot `+0xcc`, in code order
0x7330b7/0x7330d7/0x7330f7/0x733117: cat 0 -> slot 0, cat 1 -> slot 1, **cat 5 -> slot 4**, cat 2
-> slot 2 [V]. Two paths: [D]

* record type 4 with 0 < value < 0.35 (short press): first offered of cat 1, cat 5, cat 2;
* record type 3 or 2 with value >= 0.35 (hold, read through 0x00982460/0x0073a550): cat 0 if
  offered, else cat 2.

A short press with only slot 0 offered picks nothing (the model asserts this; it is a
consequence of the decoded branches, not a game observation). The meaning of record types
2/3/4 is **[?]** (see Gap 1). On success the action dispatches `ACT_PICKUP` on layer 0 through
0x0073f7f0 and `ACT_WAIT` on layers 1 and 2 through 0x0073a670 [V names, D flow], and the mode
moves to 3.

**Commit (0x004fd260, from mode 3 once the animation allows it).** [V]

1. `0x007d60c0(&weapon, slot)` is the **slot-insertion writer** (a holder method): the new
   `shared_ptr` replaces `holder+0x3c+slot*8`; a previous occupant is released through the
   object manager `[0x01191720]` vtable `+0x18` (drop versus delete **[?]**); the new weapon
   gets owner `weapon+0x94 = holder+0xc8` and bit 1 of `weapon+0x42` (the bit 0x005e3100
   tests). The same writer is used by the savegame restore (0x00710f60) and the vehicle-gun
   entry (0x0077eb20, 0x00763fe0).
2. Draw state chosen by slot, with the previous state `p = holder+0x64`:

   | Slot | New draw state |
   | --- | --- |
   | 0 | 4 if p == 4, else 1 |
   | 1 | 4 if p == 4, else 2 |
   | 2 | 3 (also builds a local vector (0, 0, 2.0) through 0x004eb5c0/0x0041f780, purpose **[?]**) |
   | 4 | 5 (also clears bit 0 of `weapon+0xe1d`, the mount branch flag) |
   | 3 | no case: the jump table 0x004fd714 routes slot 3 to the default entry |

   Picking a pistol up never *enters* dual wield; 4 only persists if it was already active.
3. 0x0056b4b0(state) then draws: gate 0x007247e0, stow-all 0x00666670 for non-5 states, layer
   states posted through 0x00873440 (`UB_S_WIELDING_IDLE` on layer 1, and `LA_S_WIELDING` on
   layer 2 for state 4), 0x005d6710 for the draw proper, `holder+0x70 = state`. State 5 instead
   posts `S_IDLE_MG` through the character vtable `+0x7c` [V names]. The outline update
   0x005b8430 and ammo transfer (0x004fd160, 0x00696060, 0x0075afe0) run around it.

**Slot 3 (`holder+0x54`) is never written.** The savegame restore loop (0x00710f60) iterates
slots 0..4 with keys `0x1e+slot` and skips 3 [V]; the pickup commit has no case for it; draw
states never read it. The slot array has no live fourth slot. The restore also reads the saved
draw state from key 0x64 and calls 0x0056b4b0 with it.

Consequences for `weapon_attach::UNSUPPORTED`: `pickup-slot-assignment` is resolved. The
existing runtime order (first one-handed gun in slot 1 / RightUpLeg, second in slot 0 /
LeftUpLeg, two-handed in slot 2 / Harness) equals the short-press default when both are free,
and the hold path reaches slot 0; the runtime does not yet model the "same type as slot 1" and
replace-on-different-type rules above. `dual-wield-gameplay` stays correct: no pickup path
produces state 4.

## 2. State 5 is the vehicle/mounted gun (partly answers "state-5-mounted-and-b")

* 0x0077eb20 (entering a vehicle with a gun) takes the vehicle's weapon through the vehicle
  component vtable `+0x5c(0)`, inserts it into **slot 4** with 0x007d60c0, sets bit 0 of
  `weapon+0xe1d`, calls `0x005d6710(5, 1)` and then `vtbl+0x94(1)` on the weapon's base. The
  simpler entry 0x00763fe0 also calls `0x007d60c0(.., 4)` then `0x005d6710(5, 1)`. [D]
* 0x00507a60 (action reset) with holder state 5 posts `S_IDLE_MG` on layer 0, `UB_S_IDLE` on
  layer 1 and `LA_S_IDLE` on layer 2 [V names]. So state 5 is the "mounted gun" body pose on
  all three layers, not a hand pose.
* The pickup offer for id 26 (category 5 -> slot 4) is what 0x0071f1b0(5) gates; id-26 weapons
  therefore enter slot 4 and state 5 from a ground pickup as well. [D]

What state 5 does with the B target and the id-26 IK branch (section 4 of
weapon-ownership-next.md) is **[?]**.

## 3. `character+0x728` is the heat-agent byte (answers "character-0x728-gate")

0x006175a0 is a one-instruction getter of `character+0x728` [V]. The interned key
**`IsHeatAgent`** (0x00e51c94, key getter 0x0078cf30) is read as a bool property by 0x005e4ad0,
and 0x00620e00 ORs "0x728 byte set" with "IsHeatAgent property set" (gated by global
0x011fc4b5), which fixes the byte's role: **the character is a heat agent** (an AI the heat
system tracks; `heat-contract-next.md` shows the heat attack action requires it). Two
weapon effects follow, both already in weapon-ownership-next.md section 2:

* 0x005533c0 (stow) clears `weapon+0xe00` for such owners;
* 0x00741620 (weapon world thunk) skips its own 0x005e3100/0x006f5d50 for such owners, because
  their weapons are refreshed by the character update (0x0057d480) instead.

No instruction in `.text` stores a byte to `[reg+0x728]`; the byte is filled by the
reflection/bulk property load (writer **[?]**). The three dword stores to `+0x728` found by
byte search (0x735e52, 0x67aeeb, 0x6b0862) belong to other classes (helicopter flag in
tether-forces-next.md) and were not followed.

## 4. Stow smoothing predicate 0x006ca590 (answers "stow-slerp-predicate")

0x006ca590 is not a combat predicate. It iterates the character's **animation layer array**
(`+0x48c`, 8-byte shared_ptr entries, count `+0x46c`; layers 0 main body, 1 upper body, 2 left
arm, 5 = all per 0x0073f7f0 [V]) and returns true only if **every layer's current state has
bit 0x200000 set in its flags dword** (0x005c39d0 -> `state = [layer+0xc]`, test of
`[state+4] & 0x200000` through 0x006b40b0; all [V]). Callers: the stow world update 0x005e3100
and 0x00826230. So the stow slerp lag runs only while all layers are in states carrying that
flag; otherwise the stowed weapon follows the bone rigidly. The state base constructor
0x004b4000 zeroes `+4` [V]; the writer that sets 0x200000 on specific states (afsm loader or
per-class constructor) is **[?]** (Gap 4). The runtime's "always true" assumption is therefore
only valid if every grounded Rico state carries the flag, which is unproven.

## 5. Hand IK: chain setup and solver identity (partly answers "ik-weighting", "solver-0x008f09c0")

* **0x008f09c0** is a thin dispatcher: if `[skeleton+0x208]+0x61` is set it calls 0x008b1320,
  which (unless global 0x012aae23) tail-calls 0x004aec90 -> 0x004c51b0, a loop over the chain
  list `[+0x60,+0x64)` calling each chain's `vtbl+4` with `this+0x28`. [V]
* Each chain is a **CIKTwoJointsImpl** (vtable 0x00f248f8, ctor 0x004b0dc0; slot 1 = 0x004953f0
  "solve once per frame", flag byte `+0x70`). 0x004953f0 calls **0x00b09440**, which carries
  the Havok profiler tag `LtTWOJOINTSOLVER`: the engine uses Havok Animation's two-joint IK
  solver; the math is Havok's, not engine code. [V]
* 0x0087dc80 builds four chains and their parameters: [V]

  | Chain | Joints (bone names) | Hinge axis (local) | Chain id (0x004a0250) |
  | --- | --- | --- | --- |
  | LeftHandIK | LeftArm, LeftForeArm, AttachHandLeft | (0, +1, 0) | 1 |
  | RightHandIK | RightArm, RightForeArm, AttachHandRight | (0, -1, 0) | 2 |
  | legs (two) | UpLeg, Leg, Foot | (1, 0, 0) | 4, 8 |

  All four set cosine limits **-0.99** (float 0x00e2b224) and **+0.95** (float 0x00e2e008)
  through 0x004a9650 / 0x004a0750, then call the handle's `vtbl+8(0)` ([I]: disable/reset; the per-frame
  target writers are assumed to re-enable them). The ctor defaults the same two values and gains of
  1.0 (float 0x00e20668) at `+0x58/+0x5c`. The names of the chain fields (hinge max/min cosine,
  gains) follow Havok's documented `hkaTwoJointsIkSolver::Setup` layout and are **[I]**; the
  engine's offsets only fit it.
* The runtime's plain two-bone solve should use hinge axes and limits above instead of a free
  elbow: left arm hinge +Y, right arm hinge -Y (mirrored), cosines -0.99 / 0.95. The per-frame
  end-effector position, rotation enforcement and gain values written by 0x00770a10 are
  **[?]** (Gap 3).

## 6. Weapon update, fire state machine and frame order (partly answers "firing order")

* The weapon's second-base vtable (0x00e441ac, class name CWeapon; its thunks first
  `sub ecx,0x9c`) has `+0x14` -> 0x005b5c90 -> **0x0083f860 (per-frame weapon update)** and
  `+0x2c` -> 0x005ef960 -> 0x00741620 (world/transform update). [V]
* 0x0083f860 is the **fire state machine** on `weapon+0xc48` (not 0xc4c): case 1 (trigger
  pressed) sets `+0xb4 = 1`, calls the shot builder 0x006ec6e0 with the V point copied from
  `+0x190..+0x198`, then state 3; case 2 repeats by cadence (`settings+0x2dc` mode 1 or 2,
  `settings+0x8c` interval, ammo through 0x0059b020, `weapon+0x230`); case 3 waits one
  period (or fires again in mode 2 while the trigger flag `+0xe19` bit 2 is set); case 4 resets
  (`+0xc48 = 0`, `+0x230 = 0`). The V point is therefore **whatever 0x006f5d50 last wrote**:
  the character update (CCombatantComponent vtbl `+0x10` = 0x005ceee0 [V], section 2 of
  weapon-ownership-next.md) refreshes it at its end, the world thunk 0x00741620 refreshes it for
  non-heat-agent owners. Whether the weapon update runs before or after the character update
  within a frame depends on the entity scheduler, which was not found (Gap 5). Interruption
  cleanup: 0x00507a60/0x006d49b0 (pickup cancel) and 0x00747700 (leave dual wield: redraw
  state 2 without stow when the remembered state was 4 or 2, then `LA` action 0x01196918) were
  read, but mid-shot draw/stow cleanup of `+0xc48` is not traced.

## 7. Aim endpoints (partly answers "aim-endpoints")

0x0054c980 correction to weapon-ownership-next.md section 6: with `settings+0x2f0 == 0` the
endpoints are `E1 = origin + weapon+0x20c` and `E2 = origin + weapon+0x218` where `origin` is
the V point (or the camera position for id 14 in scoped mode); the owner branch then
re-expresses both relative to the owner position at `owner+0x18c..0x194`. With
`settings+0x2f0 != 0` both endpoints are the absolute point `weapon+0x200..0x208`. The unscoped
no-owner case therefore does **not** add the character position. The writers of
`weapon+0x200/+0x20c/+0x218` were searched (all byte-store candidates in weapon-class
functions) and none was found; they are populated outside the functions scanned, plausibly by
a virtual setter reached from the player's aim code (**[?]**, Gap 2). The meaning of
`settings+0x2f0` stays unresolved.

## Gaps (unresolved native details)

1. Input record semantics for action 0x94 (record type bits 2/3/4, value units; which physical
   button) and where the offer mask is cleared. Blocks exact pickup controls.
2. Writers of `weapon+0x200/+0x20c/+0x218` and the meaning of `settings+0x2f0`. Blocks
   shot-direction parity (JC2-046/112 scatter consumes them).
3. Per-frame hand-IK drive: end target, rotation enforcement and gain written by 0x00770a10,
   the chain enable call (`vtbl+8`), the flag bytes `+0x70/+0x71` of CIKTwoJointsImpl, and how
   identity targets for pistols are treated. Blocks hand placement parity (JC2-045).
4. Which animation states carry flag 0x200000 (the stow-smoothing gate) and the writer of the
   state flags dword `[state+4]`; the other 25 bit tests in the 0x005c3xxx predicate family
   are the same table. Blocks the slerp gate.
5. Entity scheduling order of weapon update (0x0083f860) versus character update (0x005ceee0)
   and the interruption cleanup of `+0xc48` on draw/stow. Blocks the one-frame ordering of the
   V frame.
6. State 5 internals: vehicle weapon lookup `vtbl+0x5c`, the B target, the id-26 IK branch,
   and the effect of bit 0 of `weapon+0xe1d`. Blocks vehicle-gun hand/pose parity.
7. Fate of the replaced weapon in 0x007d60c0 (`[0x01191720]` vtbl `+0x18`: delete or world
   drop), weapon+0x98 `+0xc` bit 4 (the stow-correction gate; set by an unlocated writer), and
   the byte writer of `character+0x728`. Minor; not filed as tasks.
8. Soldier weapon ownership (equip, stow, support hand): the spawn path 0x00786280 builds
   agents with character data but how the holder is filled for soldiers was not traced.
   Blocks soldier weapon parity.

## Reproduce

```powershell
python research/tools/weapon_pickup_probe.py --game 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2'
```

Observed on 11 October 2026: exit 0, 70 checks (executable fingerprint, slot-insertion bytes,
pickup vtable, offer/choose call sites and slot stores 0,1,4,2, commit switch and jump table,
slot-3 skip in the restore loop, static-init names `S_IDLE_MG`, `UB_S_IDLE`, `LA_S_IDLE`,
`ACT_PICKUP`, `ACT_WAIT`, `UB_S_WIELDING_IDLE`, `LA_S_WIELDING`, `ACT_WAIT_FOR_LA`, the
0x006ca590 chain, the IK chain constants and ids 1/2/4/8, the weapon update thunks and the
0x005ceee0 order, plus 15 fixtures of the offer/choose/commit models). The probe reads only
the executable and prints JSON.
