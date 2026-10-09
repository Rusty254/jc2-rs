# Weapon sockets, spread and recoil: consumer handoff

Research only, 8 October 2026; follows `notes/RESEARCH-HANDOFFS.md`. Read alongside
`weapons.md`, `animation.md`, `animation-next.md`, `poses.rs`, `rig.rs`,
`combat.rs`, `crates/jc2-formats/src/weapons.rs`. This extends transform/weapon
consumer evidence; it does not replace the planned animation graph/event decoder.
JC2-017 continuation adds consumer evidence and a read-only probe. No Cargo,
game launch or runtime source changes were performed.

## Findings and limits

The original weapon settings loader **0x006e83a0** provides exact authored
translation/rotation inputs for two per-weapon matrices and a separate three-float
vector. Matrix rotations are multiplied by pi; the angular upgrade array and
`max_bullet_spread` are multiplied by pi/180. This establishes units/construction,
and the continuation below establishes A/B as hand-IK target transforms, V as
the point used in a weapon emission frame, and two upgrade consumers. A/B are
not the stow transforms: stow/held attachment has a separate bone-slot path.
Exact hand ownership, launcher special cases, soldier correction and the complete
ballistic-origin chain remain unresolved. The representative mapping below is an
exact data-to-loader mapping, not a completed attachment implementation.

Current `poses::muzzle_of` uses model bounds: `[0, min(.11,bbox_max.y*.8), bbox_min.z]`.
`rig::weapon_in_right_hand` uses columns `[Y,-Z,-X]`, derived from clip geometry.
`Stow::mount` uses zero translation at AttachHandRight, approximate Spine1 back
translation `[.1,-.12,.17]`, and thigh mounts `[+/-.11,-.09,.07]`. These are current
reconstruction choices. Original loader data should be decoded before replacing
them, and only confirmed consumers should determine the transform chain.

## Exact representative raw mappings

Values below come from `settings/weapons.bin`. Missing matrix fields default to
zero translation/rotation in 0x006e83a0; that yields identity.

| Weapon | id / model | Vector V (#70f54065, #7619481b, #05cc8d39) | Matrix A translation / authored rotation |
| --- | --- | --- | --- |
| Beretta Handgun | 2 / `Weapons\WEA02\WEA02_LOD1-WEA02.rbm` | [0,.11,-.35] | [0,0,0] / [0,0,0], fields absent |
| Assault | 11 / `Weapons\WEA11\WEA11_LOD1-WEA11.rbm` | [-.03,.115,-.75] | [-.06,.08,-.35] / [0,0,.92] |
| RocketLauncher | 16 / `Weapons\WEA16\WEA16_LOD1-WEA16_01.rbm` | [0,.1,.7] | [-.1,.06,.1] / [.7,-.2,-.15] |

All six Matrix B authoring fields are absent for these three entries, so each
loads identity B. #2477f09c is 0 for Beretta and 1 for Assault/RocketLauncher.

The launcher vector's positive z is a reason not to enforce the current blanket
barrel=-z assumption without checking its model/caller. Translations/V are model
space raw floats; their physical unit is inferred to match engine model units
(metres in current jc2-rs), not independently proven by this loader.

| Raw property hashes | Loader output byte offsets in its settings object |
| --- | --- |
| V: #70f54065, #7619481b, #05cc8d39 | +0xd4, +0xd8, +0xdc |
| A translation: #43992e3c, #02c69df6, #48165d1b | +0x11c, +0x120, +0x124 |
| A rotation: #0c5790dc, #1df6f7f6, #a0b40053 | Matrix beginning +0xec, rotation arguments multiplied by pi |
| B translation: #3c2e2ddf, #c3d756d0, #0a6c9c51 | +0x15c, +0x160, +0x164 |
| B rotation: #a2f86494, #da1ff260, #0d3adfae | Matrix beginning +0x12c, rotation arguments multiplied by pi |

Do not assume column/row convention or choose a library Euler ordering by name.
The loader calls **0x004e1470** after initializing an identity matrix. With
`a,b=sin/cos(r1)`, `c,d=sin/cos(r2)`, `e,f=sin/cos(r3)`, that helper writes these
flat matrix indices (its trigonometric polynomial approximations are visible):

```text
m[0]=e*c*a+f*b  m[1]=e*d  m[2]=e*c*b-f*a
m[4]=f*c*a-e*b  m[5]=f*d  m[6]=f*c*b+e*a
m[8]=d*a        m[9]=-c   m[10]=d*b
m[3,7,11,12,13,14]=0; m[15]=1
```

The loader subsequently sets translation indices12..14. Authored angles are
**half-turn multiples**: Assault .92 -> 2.890265 radians; launcher [.7,-.2,-.15]
-> approximately [2.199115,-.628319,-.471239] radians. Matrix A/B semantic roles
and multiplication order with bone/character/model matrices are still unknown.

Assault has `clipmodel` WEA11_CLIP.rbm and #e0db9520=[0,.01,-.15], loaded at
settings +0x80 (a separate vector). Its #f04db018 helper filename is
`Weapons\wea11\WEA11_LOD1-AR_helpers.xml`; RocketLauncher repeats this same rifle
helper reference. VFS lookup fails (file key #2034e2e9), and inspected general.blz
listing contains gun models but no helper XML. Treat this as an unresolved asset
reference, not proof that the rifle helper is valid for a launcher. Launcher also
has #a05ef628=[.05,.175,.07], #1116e1e0=[1.8,1.5,1.5]; the loader conditionally
allocates these vectors when #2477f09c is true. Their meaning remains unknown.

## Upgrade and accuracy evidence

`settings/weapon_upgrades.bin` has per-weapon-name hash nodes, six levels 0..5.
Original 0x006e83a0 loads arrays into settings object offsets:

| Array | Byte start | Conversion / certainty |
| --- | --- | --- |
| damage | +0x30c | Scalar |
| firerate | +0x324 | Scalar; current reconstruction treats as shot interval |
| #6a1fb54c angular array | +0x33c | Degrees -> radians; native MOVSS/ECX chain confirmed below; base aim-scatter angle |
| magazinesize | +0x354 | Scalar |
| #52aa95ac scalar array | +0x36c | Native MOVSS/ECX chain confirmed; per-shot character accuracy decrement |
| impulse | +0x384 | Scalar |
| damage_max | +0x39c | Falls back to damage if absent |

Hash pointers #6a1fb54c at .rdata **0x00eb2654**, #52aa95ac **0x00eb265c**;
their code references are **0x006e845d**, **0x006e847e**. Original decompilation
omits several ECX property contexts; its int-typed destinations can represent
float bits, so an apparent `(int)` cast is not enough to infer numerical truncation.

| Weapon | #6a1fb54c levels0..5 | #52aa95ac levels0..5 |
| --- | --- | --- |
| Beretta | 2.2,1.96,1.72,1.48,1.24,1 | .07 at every level |
| Assault | 2.5,2.3,2.1,1.9,1.7,1.5 | .07 at every level |
| RocketLauncher | 0 at every level | 0 at every level |

The continuation traces these to aim-scatter and per-shot accuracy loss. Neither
is established as a camera recoil field. Zero launcher values do not establish
absence of visual launcher recoil. Preserve raw hashes alongside semantic names.

Separately verified: `max_bullet_spread` #6d69456b at .rdata0x00eb26d4 loads to
+0x2d8 with degrees->radians conversion; `weapon_accuracy` #478c677a at
0x00eb26d8 loads to +0x2d4 without conversion. The continuation identifies a
separate per-projectile jitter path for +0x2d8; full AI/movement policy remains
unresolved. Current combat jitter
is .08 for pellets, .012 otherwise, scaled by `2-clamp(accuracy)`; it does not
apply these original angular arrays. Existing `Upgrades` retains named damage,
fire interval, impulse and magazine fields only.

Animation timing names `StartRecoilTime` and `EndRecoilTime` are read at
**0x007a3c70**, defaults250/750, exported at **0x006b7820**. Their unit/phase
contract is unproved; they must not be equated with physical projectile spread or
camera kick. Resolve them through the animation event work already prepared.

## Attachment consumer leads and exact remaining work

The original character caches AttachHandRight at +0x44c and AttachHandLeft at
+0x450 in `text_007e0000.c` (search names). **0x00684b70** reads the +0x44c bone
matrix via **0x008eb140**, then combines with character +0x15c via **0x00491e50**.
Lua `GetGunPivotPos` wrapper **0x00a34170** -> **0x00828350** ->0x00684b70 then
projection 0x004d7dc0. This proves a right-hand pivot/world chain, not a muzzle.
Lua `GetWeaponPosition` **0x00a3a620** -> **0x00685c30** includes character bone
+0x420 and special slots3/4; **0x006e5380** returns constants0x0106eda8/0x0106edc0.
These are screen-position paths and should not be mistaken for ballistic origin.

Next trace the settings V/A/B readers through confirmed CWeapon instances, not
every unrelated +0xd4 occurrence. Identify original equip/unequip and soldier
weapon correction, support-hand target, bullet origin, projectile direction,
stow slot and clip detach/reattach call sites. Follow +0x33c/+0x36c arrays into
shot-generation and recoil recovery. A logical `settings\accuracydefaults`
string exists in original character setup, but direct VFS lookup with/without
.bin failed; whether it is a logical property path is unresolved.

## Typed schema and coding order (proposal)

```rust
struct AuthoredTransform { translation: Vec3, rotation_half_turns: Vec3 }
struct RawWeaponMounts {
    vector_70f54065_7619481b_05cc8d39: Vec3,
    transform_a: AuthoredTransform, transform_b: AuthoredTransform,
    helper_reference: Option<String>, unknown_vectors: Vec<(u32, Vec3)>,
}
struct RawAccuracyUpgrades {
    field_6a1fb54c_degrees: [f32; 6], field_52aa95ac_decrement: [f32; 6],
}
```

1. Extend parser with raw fields, exact defaults and explicit authoring-unit
   types; report missing assets. Add a reusable matrix conversion implementing the
   verified helper formula. Apply only the confirmed semantic categories below.
2. Complete attachment ownership, ballistic origin and accuracy policy consumers;
   retain separate hand targets, emission frame, stow and camera recoil.
   Provide fixtures for Rico and a soldier holding
   pistol/rifle/launcher, including mirrored/offhand ownership.
3. Integrate one socket service: sampled pose -> final bone matrices -> weapon
   transform -> muzzle/direction. Projectile origin, flash and tracer must use
   the same frame/sample; use prior-frame transforms only for explicitly intended
   interpolation. Combat owns shot timing/projectiles; animation owns pose/mount;
   rendering consumes immutable shot endpoints. Camera kick has separate ownership.

Meaningful acceptance: raw fixtures match all three table rows and six upgrade
levels; zero rotations produce identity, each single-axis/combined rotation matches
original helper indices, pi authoring conversion occurs once. First shot immediately
after draw/switch and during a moving pose starts at that frame's resolved muzzle,
with tracer/flash agreement; test at different frame rates. Reload clip detach and
reattach preserve bone/local transforms and cleanup through interrupted reload,
weapon switch and death. Soldier two-hand grip and launcher bore alignment need
controlled captures once game execution is separately authorized. Fixed-seed
spread tests must check angular distribution and aim/motion/level transitions;
recoil tests must check kick/recovery independently after those formulas are traced.

## Reproducible inspection

```powershell
$tool = '.\jc2-rs\research\analysis\mission-access-tools\jc2tool.exe'
& $tool prop . 'settings/weapons.bin' --json
& $tool prop . 'settings/weapon_upgrades.bin' --json
& $tool hash max_bullet_spread weapon_accuracy
& $tool cat . 'Weapons\wea11\WEA11_LOD1-AR_helpers.xml'
rg -n 'FUN_006e83a0|FUN_00684b70|FUN_00828350|AttachHandRight' jc2-rs/research/analysis/pseudo
```

Addresses refer to the local retail executable, image base0x00400000. Source
exports are ignored `research/analysis/pseudo/_unnamed/text_006e0000.c`,
`text_004e0000.c`, `text_00680000.c`, `text_00820000.c`, `text_00a30000.c`.
Use private prebuilt jc2tool; retain raw assets/dumps only in ignored scratch.

## JC2-017 continuation: confirmed consumers

The executable used for every address below is retail `JustCause2.exe`, SHA256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
These are static consumer proofs, not gameplay captures. Source exports remain
ignored; the committed probe contains only structural checks and fixture values.

### Object identity, emission point and IK

**0x00700d90** constructs CWeapon (allocation size 0xe24 in its factory), installs
its vtables and assigns the passed settings pointer to **weapon+0x224**. Native
vtable writes at 0x00700dac/0xdb3/0xdba/0xdc4 contain 0x00eb455c, 0x00e3830c,
0x00e3c7a4, 0x00e441ac respectively; `classes.csv` identifies them as CWeapon.
This establishes identity before following offsets: unrelated classes also use
+0x224, so a global offset search alone does not establish a weapon consumer.

**0x006f5d50** calls point transform **0x004d8970** with settings+0xd4 (V) and
weapon+0x54 (weapon world matrix). The result is written to weapon+0x194/198/19c,
the translation row of the frame at **weapon+0x164**. The holder update
0x0050a8f0 asserts a `shared_ptr<CWeapon>` before invoking this update;
0x006f5950 invokes it in firing preparation. **0x0072aae0** passes this frame to
**0x006fae80**, whose `shared_ptr<IEffectController>` receives it through
0x00631d00. This proves the authored point contributes to weapon emission FX.
It does **not** prove all projectile origins equal V: shot factory **0x004f9560**
passes different aim matrices through projectile virtual methods. Keep the
ballistic-origin/effect-origin distinction until those methods are traced.

**0x006d1760** returns `settings.matrix_A * weapon.world`;
**0x00604870** returns `settings.matrix_B * weapon.world`.
Holder A getter **0x0062a640** uses state 3/+0x4c or state 5/+0x5c, otherwise
identity; B getter **0x00652d10** uses state 5/+0x5c, otherwise identity.
**0x00770a10** takes these targets into character-local coordinates using the
inverse character world matrix, then sends them to **0x004b7ac0**.
**0x008474d0** checks/unwraps `shared_ptr<NAnimationSystem::IIKTwoJoints>`.
An ordinary single-weapon path uses A; a weapon+0xe1d bit-0 branch uses A and B
for separate IK targets. Another branch, selector value **0x1a**, uses
0x00634a80/0x00639810 instead. Do not rename that branch RocketLauncher without
identifying the selector's domain. Exact left/right component ownership remains
unproven because several implicit ECX arguments are lost in pseudocode.
The proven semantic category is **hand-IK target transform**, not stow matrix.

### Held/stowed transforms are a separate bone-slot path

**0x005e3100** selects by **weapon+0x254** and writes weapon world at +0x54.
The ordinary path **0x00677d60** obtains a slot bone matrix through
**0x008eb140**, then multiplies by character world at +0x15c. Rotation path
**0x0076bd90** obtains the bone through **0x008e1080** (wrapper for 0x008eb140),
constructs an extra rotation through 0x004e1470 and composes bone/character world.
Recognized slot values include 1, 4, 7, 0x18, 0x19 and **0x4d**. Their exact
attachment enum/name mapping and equip/unequip writers remain unresolved.

Slot 1/4 use correction translation [-.11,.06,.06] and rotation [0,-1.7,0];
slot 7 uses [.14,.03,.06] and [.2,-1.7,0]. Slot 0x18 uses [pi/2,0,-pi/2];
0x19 conditionally uses [-pi/2,0,pi/2] when settings+0x1ca is set.
Slot **0x4d** reads the conditional vectors **settings+0x3b8/+0x3bc**, loaded
from **#a05ef628/#1116e1e0**, as translation and rotation correction.
For RocketLauncher these are [.05,.175,.07] and [1.8,1.5,1.5]. The rotation is
passed directly to 0x004e1470 in **radians**, without A/B's pi conversion.
This is a proven special attachment correction; naming its bone or applying it
to every launcher would require the unresolved slot/ownership chain.

### Upgrade units and accuracy-to-scatter chain

Native loader contexts establish #6a1fb54c at +0x33c and #52aa95ac at +0x36c.
The retained hash pointers are 0x00eb2654/265c; the respective child contexts
are saved at EBP-0x90/EBP-0x78. The conversion block around **0x006e8559**
uses MOVSS scalar stores at current-level base+0x18/+0x30, and multiplies only
the angular value by float **0x00e26f4c = pi/180**. These are floating-point
stores, not numeric integer truncation despite pseudocode casts.

**0x0082f550** selects the angular array; **0x0059b0c0** selects the scalar
array, with player/owner and upgrade-level branches. **0x006ec6e0** prepares
aim via 0x0054c980, then for a player-owned weapon subtracts the scalar from
**CCharacter+0x610**, clamping to `[0, character+0x61c]`, before shot generation
0x0072aae0. Thus #52aa95ac is a **per-shot accuracy-state decrement**. Its unit
is dimensionless accuracy state, not seconds or radians. Character construction
initializes +0x610 to 0 and +0x61c to 1. Recovery **0x0081fac0** adds
`DAT_0106ec78 * delta_time`, clamped to +0x618/+0x61c; 0x005c40b0 stores the
controller's recovered result at +0x610. The global coefficient's initialization
and movement/aiming modifiers remain unresolved; no recovery constant is proposed.

CAvatar vtable **0x00e3dc34+0xa8** and CPlayer **0x00e4cd64+0xa8** both point
to **0x006be970**. Native argument/ECX instructions show this callback receives
the weapon and accuracy state, invokes 0x0082f550, and produces equal horizontal
and vertical angular outputs:

`angle = base_angle_radians * (1 - accuracy)`

Its second endpoint getter 0x006afdf0 returns zero. **0x005c38a0** invokes the
callback with weapon, character accuracy and two output pointers, then stores
them at **weapon+0xdf0/+0xdf4**. Its pseudocode appears to store zeros because
the indirect call's output arguments are omitted. Native sequence
`56 ff d2 f3 0f 10 45 f8 f3 0f 11 86 f0 0d 00 00 ...` confirms the output loads
and stores; copying the zero-valued pseudocode would lose the real scatter.
Some weapon-modifier branches override the accuracy input; the formula describes
this callback, not every weapon/AI accuracy policy.

**0x0054c980** samples and scales these two angles into aim rotations.
**0x0072aae0** applies a separate three-axis random rotation scaled by
**settings+0x2d8**, the degree-converted `max_bullet_spread`, then passes matrices
derived from weapon+0xe4/+0x124 to shot factory 0x004f9560. These are two distinct
scatter stages. No camera kick/visual recoil equivalence is established.
HUD/Lua accuracy chain **0x00a34410 -> 0x00792d90** normalizes the maximum of
weapon+0xdf0/+0xdf4 against a configured angle and clamps to [0,1]; stored
weapon scatter values themselves are radians, not normalized HUD accuracy.

### Original transform convention and reproducible checks

0x004d8970 uses `out.x = m0*x+m4*y+m8*z+m12` (analogous y/z rows).
0x00491e50 computes flattened row-major `C=A*B`; translation occupies indices
12..14, so point-row-vector application is `v*A*B`. A column-vector library
needs the transpose/product-order conversion once, not a guessed axis swap.
The structural rotation formula already listed above equals
**Rz(r3) * Rx(r2) * Ry(r1)** in this row-vector convention. Its three input
angles correspond to Y, X, Z rotations in that order. Native sine/cosine are
polynomial approximations: the Python probe checks the structural formula with
libm, not original floating-point bit parity.

Run from the repository root, using an existing private tool:

```powershell
python -B research/tools/weapon_consumer_probe.py --game 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2' --tool 'C:/Users/Rusty/AppData/Local/Temp/jc2tool-material-water-research.exe'
```

Observed result: **PASS**, executable fingerprint matched, both vtable bindings,
both hash-pointer bindings, degree conversion and three native instruction
contracts matched; all three representative V/A/B fixtures and six levels of
both arrays matched; identity, three single-axis and one combined-axis rotation
cases matched. JSON reports native block SHA256 values for independent comparison.
The probe reads both property documents into memory and writes JSON only.
No extracted game assets are committed; no Cargo, native helper execution, game
launch, socket implementation or rendered/ballistic parity is claimed.

Remaining bounded work: attachment-slot enum and equip/unequip writers; exact
left/right/paired and selector-0x1a IK ownership; soldier corrections; projectile
virtual-method origin/direction; accuracy recovery/configuration and overrides;
physical camera recoil; missing helper XML resolution. Runtime integration must
retain those limits rather than substituting A/B as stow or V as every origin.
