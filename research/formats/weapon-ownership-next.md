# Weapon ownership: equip slots, paired hands and projectile frames (JC2-092)

Research only, 9 October 2026. Extends the accepted JC2-017 brief
`weapon-sockets-next.md` (read that first: A/B construction, V loader, 0x004e1470
formula, row-vector convention). No runtime, Cargo, game launch or GPU work was done.
Addresses are retail `JustCause2.exe` (image base 0x00400000). Probe:
`research/tools/weapon_ownership_probe.py`.

## Inputs and fingerprints

| Input | Bytes | SHA256 |
| --- | --- | --- |
| `JustCause2.exe` | - | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `settings/weapons.bin` (via `jc2tool cat`) | 39626 | `b8f986d4d996bff32025ed6075700d798237dfad8805b10ad00f9e461047850f` |
| `animations/skeletons/syncreadthis/biped.bsk` | 20656 | `080c7e6648cbfd0f81fabae179e587bf03f4a20960052738f6d727648f202710` |

The probe also reports a SHA256 for every native block it checks.

## 1. `weapon+0x254` is a skeleton bone index

The original type is `NBones::EBoneIndex`. RTTI string 0x00eb4268 names
`CHandGrenade::AttachToWielder(CCharacter*, NBones::EBoneIndex)`. Every consumer
passes `weapon+0x254` to the bone-matrix getter **0x008eb140**. The getter's ECX is
the character's skeleton holder. The wrapper **0x008e1080** is
`mov ecx,[ecx+0x208]; jmp 0x008eb140`, called with character+0xf0, so the skeleton
holder is at character+0x2f8. The getter checks `0 <= index < bone_count`, where the
count is read from the skeleton (`+0x14 -> +0x10`). Otherwise it returns identity.
The values written by the holder are biped.bsk indices (84 bones; Rico and soldiers
use biped.bsk, see `character.md`):

| Value | biped.bsk bone | Role (verified writer) |
| --- | --- | --- |
| 0x04 | LeftUpLeg | stow of holder slot 0 (+0x3c) |
| 0x07 | RightUpLeg | stow of holder slot 1 (+0x44) |
| 0x4d | Harness | stow of holder slot 2 (+0x4c) |
| 0x18 | AttachHandLeft | draw, slot 0 when dual-wielding (state 4) |
| 0x19 | AttachHandRight | draw, every other state; also "stow" of slot 4 (+0x5c) |
| 0x01 | Hips | consumer case only; no writer found among the traced setters |

`civilian_biped.bsk` has a different (40-bone) order. These constants are valid only
for biped.bsk characters.

## 2. Equip / switch / unequip chain (holder at character+0x484)

The holder has five `shared_ptr<CWeapon>` slots at +0x3c, +0x44, +0x4c, +0x54 and
+0x5c (8 bytes each; slot getter **0x0062b750** = `+0x3c + i*8`). Its draw state is
at +0x64.

* **Draw** **0x005d6710(state, stow_others)**. This optionally stows everything first
  (0x00648f20). It then calls **0x0071ff60(bone)** on the slot weapons. The native
  ECX comes from `lea ecx,[esi+slot]` and shared_ptr deref 0x00582900:
  * state 1: slot0 -> 0x19;
  * state 2: slot1 -> 0x19;
  * state 3: slot2 -> 0x19;
  * state 4: slot0 -> 0x18, then slot1 -> 0x19;
  * state 5: slot4 -> 0x19.

  Holder+0x64 becomes the state only if the required slot(s) are non-null. It always
  writes character+0x610 = 0, the accuracy state owned by JC2-046. States 1-4 are
  remembered at +0x68, and states 1/2 also at +0x6c.
* **0x0071ff60** writes `+0x254 = bone` and calls `vtbl2+0x94(1)` and `vtbl+0x60(1)`.
  When there is no owner it calls 0x0059d410 and clears bit 1 of +0xe1d.
* **Stow one** **0x005c0ba0(state)**. Calls **0x005533c0(bone)** on the slot weapons:
  * state 1: slot0 -> 4;
  * state 2: slot1 -> 7;
  * state 3: slot2 -> 0x4d;
  * state 4: slot0 -> 4 and slot1 -> 7;
  * state 5: slot4 -> 0x19.

  Then it sets holder+0x64 = 0 (holstered).
* **Stow all** **0x00648f20**, **0x00666670** and **0x0067a050**: slot0 -> 4,
  slot1 -> 7, slot2 -> 0x4d.
* **0x005533c0** writes `+0x254 = bone`. It clears +0xe00 when character+0x728 is set
  and calls `vtbl2+0x94(0)` and 0x007c94f0(owner). It clears bit 1 of +0xe1d and sets
  +0x281 = 1.
* **Active weapon index 0x007b6f20**:
  * state 3 with slot2 -> 2;
  * state 4 or 2 with slot1 -> 1;
  * state 1 with slot0 -> 0;
  * state 5 with slot4 -> 4;
  * otherwise **5**. Its consumer 0x0076b440 maps 5 to a static empty shared_ptr.
* **Pose owner** **0x005e3100** is the weapon world update, which writes weapon+0x54.
  It does nothing unless the owner (+0x94) is non-null and bit 1 of +0x42 is set. In
  that case the previous world matrix stays stale. Cases:
  * **0x18**: `R(+pi/2, 0, -pi/2) * Bone * CharWorld` through 0x0076bd90.
  * **0x19**: when settings+0x1ca is set, `R(-pi/2, 0, +pi/2) * Bone * CharWorld`.
    Otherwise the default path 0x00677d60 computes `Bone * CharWorld`.
  * **4/1 and 7**: `T(corr) * Bone' * CharWorld`, gated on bit 4 of
    `[weapon+0x98]+0xc`. The corrections are [-.11,.06,.06] with rotation [0,-1.7,0]
    (cases 4/1), and [.14,.03,.06] with [.2,-1.7,0] (case 7).
  * **0x4d**: correction translation is settings+0x3b8 and rotation is +0x3bc. If
    either pointer is null, the correction is identity.
  * **any other value**: `Bone * CharWorld`.
  * When bit 0 of weapon+0xe1d is set, a separate vehicle/mount branch runs instead.

  Every angle in this function is passed to 0x004e1470 **in radians, unscaled**.
* **Order within a character frame**, from native call order in **0x005ceee0**:
  1. 0x005a0e20 at 0x005cf393 updates the world of all five slots via 0x00568eb0
     -> 0x005e3100.
  2. 0x00770a10 at 0x005cf3a1 sets the hand-IK targets.
  3. 0x008f09c0.
  4. 0x005a0e20 again at 0x005cf3d0 recomputes the world from the post-IK bones.
  5. 0x0057d480 at 0x005cf3d7 runs 0x0050a8f0 -> 0x006f5d50, the V emission frame,
     for all slots.

  0x00741620 repeats 5e3100 then 6f5d50 for weapons whose owner is not flagged at
  character+0x728, and for ownerless weapons.

### Stow smoothing (cases 1/4/7/0x4d)

0x00908940 returns the bone as {translation, quaternion, scale}. When character
predicate **0x006ca590** is true, the quaternion is replaced by
`slerp(cached, bone_q, t)` (0x004f1af0). The ECX is a **process-global** static
cache: 0x01195f90 for case 1 and 0x01195ff0 for case 0x4d (native `mov ecx,0x01195ff0`).
Cases 4 and 7 use analogous statics. Values of t:

* t=.4 at 0x01063c90 (case 1);
* t=.4 at 0x01063c94 (case 4);
* t=.5 at 0x01063c98 (case 7);
* t=.3 at 0x01063c9c (case 0x4d).

The cache is shared by all characters. This is an original quirk; keep it explicit
if reproduced. 0x006ca590 is true when every entry of the character's +0x48c list
(count +0x46c) passes 0x005c39d0. Its gameplay meaning is unresolved. An invalid
bone index makes 0x00908940 return zero translation, an identity quaternion and
**scale 0**, which collapses the stowed rotation rows.

## 3. Settings fields (0x005dcde0 constructor, then 0x006e83a0 loader)

0x00569400 allocates 0x3c0 bytes, runs `mov ecx,eax; call 0x005dcde0`, then
`mov ecx,ebx; call 0x006e83a0`, all on the same object.

| Offset | Hash (pointer) | Default | Meaning |
| --- | --- | --- | --- |
| +0x8 | #118bd8f2 (0x00eb2664) | 2 | weapon id. Beretta 2, Assault 11, RocketLauncher 16. **Selector 0x1a = id 26** (getter 0x00609d00). |
| +0xc | #a0d58b16 (0x00eb2668) | 1 | ammo pool index (holder+0x78+4*n, 0x005e96d0) |
| +0x1c9 | #2477f09c (0x00eb26dc) | 0 | two-handed category. If set, allocates the harness corrections. |
| +0x1ca | #982afd0c (0x00eb2724) | **1** (`mov ecx,1; mov [esi+0x1ca],cl`) | right-hand rotation flag. Absent for every hand weapon, so true. Vehicle guns author 0. |
| +0x3b8 / +0x3bc | #a05ef628 / #1116e1e0 | [-.15,.15,.15] / [1.5,-1.9,3.14] when +0x1c9 is set | harness translation and rotation (radians) |
| +0x3b4 | not traced | null (only id 26 copies 100 bytes in 0x00700d90) | id-26 handle matrix (0x00639810 uses `*(+0x3b4)+0xc`) |

The id-26 (selector 0x1a) domain in retail data is MachineGunLAVE, Minigun,
MinigunARVE/LAVE/Tank/Vehicle, V022/V023/V024/V089-VHLMG_L/R and Vulcan. It is
**not** RocketLauncher.

## 4. Hand IK ownership (0x00770a10, IK chains built in 0x0087dc80)

0x0087dc80 builds `LeftHandIK` from LeftArm, LeftForeArm and AttachHandLeft at
component+0x34, and `RightHandIK` from RightArm, RightForeArm and AttachHandRight at
component+0x3c. It also builds foot and head chains. Component+0x208 is the same
skeleton holder used by 0x008e1080, which takes ECX = character+0xf0. Therefore
character+0x124 is LeftHandIK and character+0x12c is RightHandIK. This identification
rests on offset consistency; the virtual caller of 0x0087dc80 was not traced. The
native `lea ecx,[ebx+0x124]` / `[ebx+0x12c]` precede 0x008474d0 -> 0x004b7ac0:

* Holder state 5, after forcing 0x005e3100:
  * if bit 0 of weapon+0xe1d is set: A -> **LeftHandIK** and B -> **RightHandIK**;
  * else if id == 26: 0x00634a80 (the id-26 matrix or A, times the weapon world)
    -> **RightHandIK**;
  * else: A -> **LeftHandIK**.
* Any other state: A via 0x0062a640 -> **LeftHandIK** (`mov ecx,[ebx+0x124]`).
  The A getter returns `A * weapon.world` for state 3 (slot2) or state 5 (slot4).
  Otherwise it returns **identity**. The B getter only returns for state 5.

So **A is the support (left) hand target of a two-handed weapon held in the right
hand**. B is used only for mounted/state-5 weapons. Pistols and dual wield (states
1, 2 and 4) get an identity A target. How identity is weighted in the IK solve is
unresolved. The IK inputs are taken into character space with the inverse character
world, as JC2-017 found.

## 5. Paired-hand case (state 4)

With two one-handed guns, slot0 -> AttachHandLeft with `R(+pi/2,0,-pi/2)` and slot1
-> AttachHandRight with `R(-pi/2,0,+pi/2)`. The rotation rows are the model axes
expressed in the bone frame:

* right hand: model X -> bone +Y, model Y -> bone -Z, model Z -> bone -X;
* left hand: model X -> bone -Y, model Y -> bone -Z, model Z -> bone +X.

So the barrel (model -Z) points along +X in the right bone and -X in the left. The
active index is 1, the slot1/right gun. A is identity. How the two guns alternate
fire was not traced.

## 6. Supported projectile origin/direction chain

1. **0x006f5d50**: `weapon+0x194..0x19c = V * weapon.world`, the emission frame at
   +0x164 (JC2-017).
2. **0x0083f860** is a method on the weapon+4 subobject (`lea edi,[esi-4]`). It
   copies `[esi+0x190..0x198]` (weapon+0x194..0x19c) to a local, then calls
   **0x006ec6e0(&origin)** with ECX = the weapon. This happens through the firing
   switch on weapon+0xc4c.
3. **0x006ec6e0 -> 0x0054c980(origin)**: `eye = origin`. One exception: for id 14
   (Sniper), owned by a character with flag +0x728 and with `[0x0040e670()]+0x300`
   equal to 2 or 3 (a scoped mode; inferred), the eye is the camera position
   `[[0x011919f4]+0x10]+0x54`.

   With an owner, the endpoints are `E1 = char_pos + weapon+0x20c` and
   `E2 = char_pos + weapon+0x218`. When settings+0x2f0 != 0, both are the absolute
   point weapon+0x200. Both are rotated about the eye by the two scatter quaternions
   from weapon+0xdf0/+0xdf4 (JC2-046).

   Then **0x004dc8a0 -> 0x004e0f60** builds look-at frames:
   `weapon+0xe4 = LookAt(eye, E1, up +Y)` and `weapon+0x124 = LookAt(eye, E2, up +Y)`.
   0x004e0f60 sets row2 = normalize(eye - target), so **-Z is the shot direction**.
   Row0 = normalize(up x row2), row1 = row2 x row0, and translation = eye. If
   |dir . up| > 0.9999 the frame is **not rewritten** and stays stale.
4. **0x0072aae0**: for each projectile, the jitter is
   `J = 0x004e1470(rand*settings+0x2d8, ...)` (JC2-046). Then `M1 = J * weapon+0xe4`
   and `M2 = J * weapon+0x124`, followed by **0x004f9560(weapon, M1, M2, ...)**.
5. **0x004f9560** creates the projectile with
   `factory(0x005ef190)->vtbl[0x10](M1)`. This is native-verified:
   `mov ecx,[ebp+0xc]; ... push ecx; call edx`. When settings+0x19c > 0 and the shot
   counter weapon+0x238 is divisible by it, it also builds a tracer with
   0x0056f420(..., **M2**, ...): `mov ecx,[ebp+0x10]; push ecx`.
6. FX: 0x0072aae0/0x006ec6e0 call 0x006fae80(weapon+0x164), the emission effect
   frame.

**Verified:** the projectile origin is the V emission point of the current weapon
world, and its direction is the -Z row of the jittered look-at M1. The tracer seeds
from M2 and the muzzle FX from frame +0x164.

**Unresolved:**
* the writers of weapon+0x200, +0x20c and +0x218 (the aim endpoint offsets) and the
  meaning of settings+0x2f0;
* how each projectile class consumes the M1 rows (rocket, grenade and pellet virtual
  methods);
* the frame order of 0x005b5c90 -> 0x0083f860 relative to 0x005ceee0. The fire path
  reads whatever V frame is current; whether that is the frame updated after the
  post-IK pose is not proven;
* interruption cleanup (draw/stow mid-shot) is not traced.

## 7. Transform order and units (reproduced by the probe)

* Row-vector convention throughout. The weapon world is
  `R_hand * Bone_model * Char_world` (0x0076bd90: native pushes give
  `0x00491e50(R, bone, out)` then `(out, char+0x15c, final)`). The stow world is
  `T(corr)*R(corr) * Bone' * Char_world`. The A target is `A * weapon_world`. The
  shot frame is `J * LookAt`.
* Half-turns versus radians:
  * A/B authored rotations are multiplied by pi (Assault .92 -> 2.890265 rad);
  * hand rotations (pi/2 floats at 0x00e2f898 and 0x00e48a74), stow corrections
    (-1.7, .2) and harness corrections ([1.5,-1.9,3.14] default; launcher
    [1.8,1.5,1.5]) are **radians, unscaled**. 3.14 is a literal float, not pi.
* Fixtures with synthetic non-commuting bone and character matrices (probe JSON):

| Fixture | Slot / state | Hand | A owner | Stow bone and correction |
| --- | --- | --- | --- | --- |
| Beretta (id 2) | 0 / 1 | AttachHandRight + R(-pi/2,0,pi/2) | none (identity) | LeftUpLeg, [-.11,.06,.06] r[0,-1.7,0] |
| Assault (id 11) | 2 / 3 | same | slot2 -> LeftHandIK, A r=[0,0,2.890265] | Harness, default [-.15,.15,.15] r[1.5,-1.9,3.14] |
| RocketLauncher (id 16) | 2 / 3 | same | slot2, A r=[2.199115,-.628319,-.471239] | Harness, [.05,.175,.07] r[1.8,1.5,1.5] |
| Dual Beretta | 0+1 / 4 | 0x18 + R(pi/2,0,-pi/2), 0x19 + R(-pi/2,0,pi/2) | identity | - |

* Rejections, verified:
  * drawing an empty slot is a no-op;
  * an active index whose state and slot do not match returns 5, then an empty
    pointer;
  * A/B in the wrong state is identity;
  * a bone index < 0 or >= count gives the identity bone;
  * a null owner keeps the stale world;
  * the vehicle flag 0 skips the hand rotation.

**Inference, not traced:** slot 0/1 hold one-handed weapons and slot 2 holds
two-handed (+0x1c9) weapons. In the pickup prompt 0x00732d20, id 26 is category 5,
two-handed is 2, and one-handed tries slot 1 then slot 0. The actual slot-insertion
writer was not found.

## 8. Runtime handoff for JC2-045 (bounded)

1. Held gun: parent to **AttachHandRight** with model rotation `R(-pi/2,0,pi/2)`.
   Current `rig::weapon_in_right_hand` (columns [Y,-Z,-X]) **matches** this. The
   empirical axes are now proven. For a dual left gun use AttachHandLeft with
   `R(pi/2,0,-pi/2)`.
2. Support hand: for two-handed guns in state 3, drive a left-arm two-joint IK
   (LeftArm/LeftForeArm/AttachHandLeft) to `A * weapon_world`, where A is from the
   settings, angles x pi. This runs after the weapon world is computed from the
   pre-IK pose. Then recompute the weapon world, then V. There is no support-hand IK
   for pistols or dual wield.
3. Stow: replace the eye-placed `Stow` mounts.
   * Slot0 -> LeftUpLeg with `T[-.11,.06,.06] R(0,-1.7,0)`.
   * Slot1 -> RightUpLeg with `T[.14,.03,.06] R(.2,-1.7,0)`.
   * Two-handed -> **Harness** (not Spine1) with the per-weapon +0x3b8/+0x3bc
     corrections (defaults above).
   * Optional slerp lag of t = .4/.4/.5/.3.
4. Shots: origin = V transformed by the same-frame weapon world; direction = -Z of
   LookAt(origin, aim endpoint, +Y). Use the bounding-box `poses::muzzle_of` only
   where V is absent. FX uses the +0x164 frame; the tracer uses M2.
5. Keep JC2-046 (scatter J, the angles at +0xdf0/+0xdf4, accuracy +0x610) and
   JC2-026 (clock) separate.

**Contradictions in current master and the unreviewed branch
`worktree-agent-accfeb99b78adb08a` (rig.rs/poses.rs/combat.rs):**

* `poses::Stow` says "the data has no holster bones". In fact the exe stows by
  EBoneIndex: thighs 4/7 and **Harness 77**.
* Its back mount on `Spine1` with T[.1,-.12,.17] and its thigh mounts
  [+/-.11,-.09,.07] with rotation columns (X,-Z,Y) are not the original transforms.
* The first one-handed gun goes to `RightThigh`. That agrees only with the untraced
  pickup inference (slot 1 = RightUpLeg) and is not proven.
* `muzzle_of`, which is derived from the bounding box, is not the original origin
  (V).
* The branch has no left-hand IK target from A.

The hand rotation itself is correct.

## Reproduce

Run from the `research/tools` directory, where `jenkins.py` lives. Use a private
copy of the prebuilt jc2tool:

```powershell
python -B weapon_ownership_probe.py --game 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2' --tool '<scratch>/jc2tool.exe'
```

Observed result on 9 October 2026: exit 0. The probe reported:

* the executable fingerprint matched;
* 18 native blocks matched their needles;
* 18 rel32 call sites decoded to the expected callees;
* the 0x005ceee0 update order held;
* 5 hash-pointer bindings, 4 slerp constants, 2 hand-angle constants and 5
  harness-default loads matched;
* biped.bsk had 84 bones with the six slot names at the expected indices;
* the selector-0x1a domain had 15 weapons, all id 26;
* the three fixtures, the paired case and 7 rejection checks passed.

The probe reads assets into memory and prints JSON only; nothing is extracted or
committed.
