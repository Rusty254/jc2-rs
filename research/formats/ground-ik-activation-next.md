# Ground foot-placement IK: activation, tuning, update order and feedback (JC2-093)

Research only. Continues `foot-ik-next.md` (JC2-016, accepted); that file's raw
reload hashes, wrapper offsets and raycaster facts are reused, not repeated. No
runtime edit, Cargo build, game launch or save write. Authored FootIK/HandIK
socket curves (JC2-026/JC2-045 territory) are a different mechanism and stay out
of scope; section 5 shows exactly where the two meet.

Executable: `JustCause2.exe`, sha256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
All addresses are runtime addresses. "Native" = verified from exe bytes by
`research/tools/ground_ik_activation_probe.py`; "pseudo" = read from the ignored
Ghidra export (checked by the probe's optional `--export` mode); "inferred" =
reasoned from pseudo and not executed.

## 0. Result in one paragraph

Ground placement is a per-leg Havok `hkaFootPlacementIkSolver` wrapper
(`CIKFootPlacementImpl`) whose only gameplay switch is `0x00899c90(flag)`. It is
called from 19 sites; the flag is 1 in six state-entry functions (rooftop stunt,
cling to roof, stunt jump, grappling jump x2, rocket-baby) and 0 elsewhere, and the
constructor leaves both legs off, so ordinary locomotion has no enable site.
Effective tuning is the constructor's compile-time defaults: the reload function
`0x004bedf0` has no direct caller and none of its 18 keys (now all named by exact
hash) occurs in any of 11,194 retail archive entries. The two legs run one after the
other, each with private state, and **there is no paired-leg feedback**: `+0x138`
is never written after construction, `+0x130/+0x134` are never read except inside the
update function itself, and the `+0x13c` weight is never read by the solver.
The planting/cache path (the only world-space persistence) is gated by a flag that is
0 and has no writer. So retail has no support identity and no support-local
persistence: it re-casts a world ray from the live pose every frame.

## 1. Activation (native)

* `0x0049d850(byte)` stores `[impl+0x110]` (see wrapper: `[wrapper+8]` = impl).
  Callers: two in the character-component builder `0x0087dc80` (value 0, both legs)
  and two inside `0x00899c90`. No inlined copy of the store exists (the ten other
  `mov byte [reg+0x110]` sites, e.g. in `0x485dc0`, `0x5777b0`, `0x8f30f0`, belong to
  unrelated objects; their preceding bytes show no wrapper-handle `[reg+8]` load).
* `0x00899c90(flag)` (ECX = character-component, i.e. character+0xf0) applies the
  same flag to left (`component+0x24`) and right (`component+0x2c`). The probe
  finds exactly 19 `call 0x899c90` sites (18 in `calls.csv`, plus `0x7d00f6` in a
  range Ghidra has no function for).
* Impl defaults: input block constructor `0xb09ca0` sets `+0x110=1`, then
  `0x4bb480` stores 1 again and `0x87dc80` immediately calls `0x49d850(0)` for both
  legs. Effective default: **off**.

| Site | Function | RTTI class / vtable slot | Flag |
| --- | --- | --- | --- |
| 0x5a1b87 | 0x5a1b30 | CRocketBabyController / 10 (string tag "OnEnter") | 1 |
| 0x701f8c | 0x701f30 | CRoofTopStuntController / 10 (raises "ply.rooftop_entered") | 1 |
| 0x8089fd | 0x8088e0 | CClingToRoofAction / 6 | 1 |
| 0x6b015b | 0x6b0130 | CStuntJumpAction / 21 | 1 |
| 0x57d78e | 0x57d620 | CGrapplingJumpAction / 15 | 1 |
| 0x5fb5e7 | 0x5fb010 | CGrapplingJumpAction / 7 | 1 |
| 0x60ce14 | 0x60ce00 | CRoofTopStuntController / 11 | 0 |
| 0x8073f7 | 0x8073e0 | CRocketBabyController / 11 | 0 |
| 0x7dc7c5 | 0x7dc790 | CClingToRoofAction / 21 | 0 |
| 0x7202d0 | 0x720100 | CStuntJumpAction / 6 | 0 |
| 0x502423 | 0x5023a0 | CMCController / 10 | 0 |
| 0x6130d6 | 0x612ec0 | CEnterMCFromRoofAction / 7 | 0 |
| 0x6b896a | 0x6b8890 | CReelInAction / 7 | 0 |
| 0x7604af | 0x760140 | CEnterDriverSeatFromRoofAction / 7 | register (0 per pseudo) |
| 0x7ae40b | 0x7ae2e0 | CPullGrappledAction / 6 | 0 |
| 0x65b4ef, 0x6e032f, 0x7d193c | 0x65b3f0, 0x6e0100, 0x7d1900 | no vtable entry | register (0 per pseudo) |
| 0x7d00f6 | (not in functions.csv) | unknown | 0 |

Class names come from walking each vtable slot back to its RTTI complete-object
locator. Slots 10/11 of CRocketBabyController and CRoofTopStuntController are an
enter/exit pair (the enable and disable sit in adjacent slots). Gating inside the
enable functions (pseudo): `0x701f30` and `0x5a1b30` enable only when the tracked
object at `character+500` is valid and `[character+0x1f0]->(+0x1c)->(+4)` is
non-null (the vehicle/target the state is standing on); `0x8088e0` enables only when a
virtual predicate (slot +8) on its target object is true, subject to two helper
predicates (`0x817ac0`, `0x81d330`). What those objects are is not decoded. Meaning of
"MC" in CMCController is unknown. Verified-negative: no enable site exists for the
default on-foot controller, so retail does not run ground placement during ordinary
walking (the probe cannot see enables through other flags; none was found).

## 2. Tuning (native defaults, exact-hash names)

The reload keys were previously only raw hashes. Using the engine's Jenkins hash
(`research/tools/jenkins.py`), exact case-sensitive PascalCase names hash to all 18
words stored at `0x00f246b8..0x00f246fc` (the probe asserts each):

| Key | Hash | Impl offset | Default (decoded from `0x4bb480` immediates) | Role (pseudo) |
| --- | --- | --- | --- | --- |
| HipIndex / KneeIndex / AnkleIndex | e5a43194 / 6ad671fd / c56a19a6 | +0x34/+0x36/+0x38 | bound in `0x87dc80` by bone name (LeftUpLeg/LeftLeg/LeftFoot, Right*) | bone indices |
| KneeAxis | 0d1dac62 | +0x40 | (1,0,0) | local hinge axis |
| FootEnd | ee9acc15 | +0x50 | (0, 0.0647, -0.1172) from stored immediates | toe point, local |
| FootPlantedAnkleHeight | 637b31d1 | +0x84 | 0.0642 | low end of height-blend ramp |
| FootRaisedAnkleHeight | 486d8c94 | +0x88 | 0.35 | high end of ramp |
| MaxAnkleHeight | d2f36d02 | +0x8c | 0.35 | target ankle height cap (model up) |
| MinAnkleHeight | e769e888 | +0x90 | -0.35 | below this the leg is rejected |
| CosineMaxKneeAngle | b44c657f | +0x94 | -0.9 | passed unchanged to two-joint solve |
| CosineMinKneeAngle | 31da1a52 | +0x98 | 0.8 | passed unchanged |
| RaycastDistanceUp | 6b15521b | +0x9c | 1.0 | ray starts above ankle |
| RaycastDistanceDown | 3687e42e | +0xa0 | 1.0 | ray ends below ankle |
| OnOffGain | b72e8352 | +0x118 | 0.3 | blend gain per solver call |
| GroundAscendingGain | 49351b4c | +0x11c | 1.0 | smoothing when ground rises |
| GroundDescendingGain | 36a15136 | +0x120 | 1.0 | smoothing when ground falls |
| FootPlantedGain | cccc6ed7 | +0x124 | 1.0 | upper clamp of height blend |
| FootRaisedGain | 867dae80 | +0x128 | 0.2 | lower clamp of height blend |

Other native defaults: world-up and model-up `+0x60/+0x70` = (0,1,0); `+0x80`
(offset subtracted from ankle height) = 0; unlock gain `+0x12c` = 1.0 (set by
`0xb09ca0`, not reloadable); weight `+0x13c` = 1.0; correction `+0x138` = 0.0;
**plant flag `impl+0xa4` (settings+0x74) = 0** (`c6 40 74 00` at `0xb09b90`).
Units are skeleton distance units; the code never converts them. Treating them as
metres is a reconstruction convention that is only valid for a unit-scale rig.

Selector `#3da29d18` (subnode of the node passed to reload) has no recovered name
(1.5 million PascalCase/snake/upper combinations of foot/IK/limb words tried).
Reload also reads `ID` (hash of the two characters at `0xe2c96c`) into the IK name
string/hash at `impl+8/+4` and `Filter` into `impl+0x24` (`0x4bcd70`); foot
placements are never named by the constructor, so the by-name lookup used for
authored weights (section 5) cannot find them.

Reload `0x4bedf0` is wrapper vtable `0xf24a9c` slot 1 (native). The probe finds no
`call 0x4bedf0`, and the raw-key scan (`--scan-archives`; observed entries 11194, hits 0)
found none of SELECTOR/HipIndex/OnOffGain/RaycastDistanceUp, little- or big-endian, in
11,194 decompressed archive entries (all but dds/aitile/fsb, up to 80 MB; the method
was checked once by hand by finding the `weight` key 3,089 times in `animations.blz`). With absent
keys the property getter `0x48e630` returns a static zero object (pseudo), so a
reload with the wrong node would zero the indices and gains; the shipped game
evidently never does. Conclusion (strong inference, not a proof of absence of an
indirect caller): the constructor defaults above are the retail tuning. Reload also
rebuilds the solver from `impl+0x30`, which would reset blend and prior-height state.

## 3. Per-frame order (native call order, phase labels from pseudo)

`0x005ceee0(frame, phase, character)` switch:

* phase 2 tail: `0x7cc220` -> `0x8d7fe0` -> `0x49c350` calls wrapper vtable slot 2
  (`setWeight`) with 0 on every IK in the vector; then `0x8b80e0` -> `0x905860`
  applies authored constraint weights through `FindByName(hash) -> slot 3`
  (`addWeight`, clamped to [0,1]).
* phase 3: `0x5a0e20` (weapon/world refresh), `0x770a10` (hand targets), `0x8f09c0`
  (IK solve), `0x5a0e20` again, `0x57d480` (weapon emission).
* `0x8f09c0` -> `0x8b1320` -> `0x4aec90` -> `0x4c51b0` iterates the vector of
  `shared_ptr<IIKBase>` and calls impl vtable slot 1 with the character pose
  (`holder+0x28`); the return value is never tested (`ff d2 3b 77 08`).
* Vector order is registration order in `0x87dc80` (seven `0x8e9770` calls): left
  foot placement, right foot placement, look-at, LeftHandIK, RightHandIK, then the
  two-joint LeftFootIK and RightFootIK. Order basis: pseudo; count: native.
  So ground placement runs before, and is separate from, the authored two-joint
  FootIK/HandIK chains.

## 4. Update, result fields and the (absent) feedback

`0x4b9210` (impl slot 1): refresh owner world transform from `[impl+0x140]`
(character matrix, set once by `0x4ab9a0`), copy the pose ankle QsTransform to
`impl+0xb0`, subtract `impl+0x138` from its model y, call solver `0xb09fd0(input
+0xb0, result +0x130, pose)`, return `[impl+0x134] != 0`.

Feedback verdict (native, whole-image where stated):

* `impl+0x138`: the only store in the foot-placement code is the constructor
  (`0x4bb4d7`, zero); the only read is the subtraction in `0x4b9210`. No wrapper
  method exists for it (wrapper vtable = dtor, reload, setWeight, addWeight).
  Wrapper-reached stores (`[wrapper+4/+8]+disp`) in the whole pseudo export:
  only `0x13c` at `0x4ae560`. Value in retail: 0.0 for the object's life.
* `impl+0x130` (vertical error) and `+0x134` (accepted flag): written by the
  solver, read only as a pointer pass and the `cmp/setne` return in `0x4b9210`.
  The caller discards the return. Other `0x130/0x134` accesses
  (`0x4a57d0`, `0x4cc700`) subtract a 4-float vector at `0x130..0x13c` in an
  unrelated class.
* `impl+0x13c` weight: written by `0x4ae560`/`0x4c80a0` (and the ACSB path), never
  read by this solver.
* No pelvis/hip consumer, no sum or min over legs, no write of the capsule or
  physics root. "No double feedback" therefore holds trivially: there is no
  feedback. The Havok reflection string `pelvisFeedbackGain` is library metadata
  only. The proposal below must not invent one and call it original.

Solver `0xb09fd0` active path (planted flag clear; pseudo, inferred):

1. Disabled input, or no ray hit, or target below MinAnkleHeight: take the
   inactive path: error 0, accepted false, target = animated ankle plus the last
   stored vertical offset (`solver+0xb0`), then blend decays.
2. Ray (`0xb09ce0`): start = ankle + up*RaycastDistanceUp, end = ankle -
   up*RaycastDistanceDown; `0x4cabd0` returns fraction and normal. **Hit iff
   fraction < 1.0 strictly** (native: `comiss 1.0,fraction; jbe miss`); a hit exactly
   at the end is rejected. Hit height = start - fraction*length along up. If the
   foot-end point differs from the ankle by more than 0.001 on any axis a second
   ray is cast there and its plane (normal.up > 0) is intersected with the ankle
   ray; the nearer fraction wins. The plane branch's fraction is not clamped to <1
   in this function (arithmetic partly untyped; unresolved).
3. `error(+0x130) = groundH - (ownerH + offset+0x80)` along world up.
4. `k = clamp(1 - (ankleH - Planted)/(Raised - Planted), FootRaisedGain,
   FootPlantedGain)`. If the blend state is nonzero:
   `S = prior + gain*k*(groundH - prior)` with gain = Ascending if prior <= groundH
   else Descending; otherwise `S = groundH`. `prior` (`solver+0x94`) is a world
   up-height, not support-relative.
5. Target ankle model height = (S - ownerH) + animated ankle height (identity owner
   rotation, unit scale; inferred), pulled down to MaxAnkleHeight, rejected if
   below MinAnkleHeight (`Min <= value` is active).
6. `blend += OnOffGain*(target - blend)` with target 1 when accepted else 0, no
   delta-time factor; below 0.01 it snaps to 0. When blend > 0 the hip-knee-ankle
   solve (`0xb09440`) runs with weight = blend, the ankle rotation is blended toward
   the ground normal and written with `0xb00db0(index,1)`.

Retire: disabling does not clear state; the ankle offset is held at the last stored
value while blend decays by 0.7 per solver call (13 calls from 1.0 to the snap).
The call is per animation update, so decay time is frame-rate dependent.

## 5. Interaction with authored FootIK curves

`0x905860` evaluates each authored constraint, finds the IK whose name hash equals
`entry+0x124` (`0x4c2330`, comparing `impl+4`) and calls slot 3 with
`curve*layer`. Names are LeftFootIK/RightFootIK/LeftHandIK/RightHandIK, which
`0x87dc80` gives only to the two-joint chains. Foot placement is not named, so the
authored curves in `foot-ik-next.md` do not scale ground placement, and ground
placement's `+0x13c` is inert anyway. Do not feed `heli_cling_dwn_idle` style weights
into the ground solver.

## 6. Support identity and persistence

* The raycaster interface returns fraction and normal only. The filter accepts a
  candidate when predicate 1 or 3 holds and `candidate+0xc != owner id` (owner id =
  holder vtable slot 1 result, stored at `impl+0x1f0`). Predicate meanings are
  unresolved.
* Nothing stores a support entity, body, local point, velocity or lifetime. The
  states that enable placement hold the vehicle/target themselves (`character+0x1f0`
  region) but pass nothing to the foot solver.
* The only world-space cache (`solver+0xc0` point, `+0xe0` release displacement) is
  inside the plant branch, gated by `settings+0x74` = 0 with no writer. Planted
  anchoring is therefore dormant in retail.
* On a moving vehicle roof the retail behaviour is: owner matrix re-read each
  frame, fresh world ray each frame, and a world-height smoothing prior. That is the
  entire "moving platform" story; any support-local anchor is new design.

## 7. Proposed bounded runtime contract for JC2-075 (design, not original)

Scope: per-leg visual pose correction only; no generic IK framework; no capsule
writes; no pelvis offset (not original; if wanted later it is a flagged extension
computed once from both legs and applied once).

* Enable: default false per character; set true by exactly the state entries in
  section 1 (rooftop stunt, cling to roof, stunt jump, grappling jump, rocket-baby),
  false on their exits and the listed disable states. One call sets both legs.
* Inputs per leg per update: enabled, pre-IK ankle model transform, bone chain,
  live world-from-model transform, up vectors (0,1,0), the tuning table (section 2
  defaults), ray query `(start,end) -> Option<(fraction, normal)>` with strict
  `fraction < 1` and owner exclusion supplied by the physics layer.
* State per leg: `blend=0`, `prior=0`, `offset=0`. Reset all three on teleport,
  despawn or rig rebuild (the original resets only on reload; this is a safety
  addition and must be labelled).
* Order: after weapon/hand target refresh, left leg then right leg, before authored
  two-joint foot/hand constraints, then re-derive world transforms.
* Outputs per leg: `error` (+0x130), `accepted` (+0x134), `weight`, and a corrected
  ankle target; diagnostics only, never fed to another leg.
* Fixtures (synthetic, produced by the probe): flat ground error 0 with fraction
  0.75 for a 0.5 m ankle; stair (left ground 0.2, right 0) gives independent errors
  0.2 and 0 and no pelvis term; slope y=0.5x at x=0.5 gives hit y=0.25, fraction
  0.425, normal (-0.5,1,0); ground exactly at the ray end gives fraction 1.0 and is
  rejected, 0.001 above it is accepted; for ankle height 0.10 and steady state (smoothing ignored) a
  ground offset above +0.25 clamps at MaxAnkleHeight and below -0.45 rejects the leg
  (per MinAnkleHeight);
  blend ramps 0.3, 0.51, 0.657 ... and decays 0.70, 0.49, 0.34 ... to 0 after 13
  calls.
* Moving support: use the live character matrix and a fresh ray each frame, keep
  `prior` in world height as original, and do not attach anchors. Do not claim
  parity for platform anchoring.

## 8. Fingerprints and reproduction

* `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
* Prebuilt `jc2tool.exe` copy used for the archive scan: sha256
  `3bc5e39e33f4261364cf82bd1c8d9277e6bbb08a07a95577a0cbc2c28aed4b89`.
* Archives scanned (sha256), `archives_win32/`: pc0.arc
  `e2c2a1d2c96924928058590ad7c2fb6690d6bd5306f70e7fdebdb88dc1ff8067`, pc0.tab
  `0d5a2abeb33f917118fed277f42ed7533bfa1e92815986a57e5f6ff3e8e13da8`, pc1.arc
  `cb1c3c581e179d4baea5c2813d74376138edfeb984502f19313b0aee210c75c0`, pc1.tab
  `aa25504681438d195cffc2f29cac87e96b7db0332ad3c357722e6c01e65a34b6`, pc2.arc
  `6bc7ad0b62408334d52d4866df03c20649c2a1797fea6e242abcac371e6be6d2`, pc2.tab
  `569e630b34f1dbbf0bde5a66946d4e6f4bac03e0825692a8d69a988c497fc25c`, pc3.arc
  `423c3504b11425845479628869f22ee97ada3ffc3fb576f1a0bece50f47dfbed`, pc3.tab
  `9686813e9b44d76310d5dd76cd9870c97f813c2bf446707e4e5280448fc13cd0`, pc4.arc
  `b60ffaeb225ffa10fe02a359e5ac8bf0c871c3b603a9814c384961b56be02249`, pc4.tab
  `b9a83879ffe45fa5d820584232ac1e64eb08fe955e78a77d5d1ed3945412f52b`.
* DLC (sha256) `DLC/`: pc_60.arc `06e80e17333da60bb9c8023025989587a22d239b5e97afc54a494406370a8778`,
  pc_60.tab `664a6f7d4a99f387249f16212746206256125ac38352934515c1cd4c8de43604`,
  pc_70.arc `5b4e9f02814f913c93ebc6c55dec782806f559313286de21b0f876f250a3e55e`,
  pc_70.tab `6a4c64bdeca8a26b03974379323c1b08fd180824af3ba8440ab2b788c1cde1d4`,
  pc_130.arc `2dd54a5e66eeecfb0dab4ce8ebccea9903fb862a4f1c364a22fab7b6c07545f6`,
  pc_130.tab `0762ca444455b28cde0674f8ad49f5a7574002e2eb23c8a8229917bdbef31de4`,
  pc_140.arc `aa59190901b7f206f208ad9933d176755aaa7ec8acf2a132367d214000eed3a6`,
  pc_140.tab `b864891583ed28dc0d8d639c288e0f53a53dce115a0d72613392894752a8659b`,
  pc_150.arc `4035dcdff76c985d09fb86807b996efcf95dd54bdafaada3aa9caf4f1f88f22d`,
  pc_150.tab `edcc9a4497c9a6c850e838da64e189d2bdc1187ed183e45d6bdebb1f2096fa93`,
  pc_160.arc `7cd539abc6c35dd54a0adc57dd2dd9f15e70183ea30c4fb0f783da0bb8eaf832`,
  pc_160.tab `d1f10fd4a31aaf7365c6b4790152fba91b82de786407a5c0d4b42ce3a91d3da9`,
  pc_170.arc `d9a0d0fd8e3ec0056aecdc9109eb1ace57e9e8d5091659009d34d837e6169501`,
  pc_170.tab `4e594e4b2d901ad0efc2af74c63d2f33a7f6a8026b698e40d7c0614bdbf25cc9`,
  pc_180.arc `ea60b020b330eb5076d019af310357dabc7e7a331f2e8a41f48503a96c504736`,
  pc_180.tab `2721443528312b85182a553e9fe783a35d4855e6046a1eba0c1ebdb5b3ab1a8a`.
  The scan covered the 28,160-line `jc2tool list` output, 11,194 entries after the
  exclusions in section 2.
* Probe fixtures contain no external inputs: native checks read only the exe;
  synthetic checks are literal numbers in the script.

```powershell
python -B research/tools/ground_ik_activation_probe.py --game 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2' --export 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/research/analysis/pseudo/_unnamed'
# optional, slow: add  --tool <copy of jc2tool.exe> --scan-archives
```

Observed result: PASS (all native assertions and, with `--export`, the pseudo
cross-checks); JSON lists the 19 sites, exact-hash names, defaults, frame order,
`+0x138/+0x134` accesses and the labelled synthetic cases.

## 9. Verified / inferred / unresolved

Verified (native): vtable bindings; 18 exact key names; selector value; constructor
defaults; plant flag default; 19 activation sites with flags; sole writer of
`+0x110`; update order and discarded return; `+0x138`, `+0x134`, `+0x13c` access
patterns in the implementation code; strict `fraction < 1`; no direct caller of
reload; absence of the keys in the retail archives.

Inferred: solver arithmetic (sections 4.2-4.6, from pseudo); retire behaviour;
no-pelvis conclusion beyond the implementation's own code (pointer reachability
argument: the impl is reachable only through the wrapper handle and the IK vector,
and no accessor of those writes `+0x138`); retail tuning equals defaults.

Unresolved: name of selector `#3da29d18`; whether any indirect path ever calls
reload; filter predicates 1 and 3 and what the collider categories are; second-ray
plane-intersection arithmetic and ankle rotation blend; value of `prior` after
inactive frames; what the enabling states' `+0x1f0` target is; the state behind
`0x7d00f6`; whether NPCs reach the same enables; meaning of CMCController; frame
rate the blend gain assumed.

## 10. Runtime (JC2-075)

Implemented in `crates/jc2-game/src/ground_ik.rs` (tests in `ground_ik/tests.rs`). Native
constructor defaults (0x004bb480), strict `fraction < 1`, up/down 1.0 rays, ramp, smoothing,
ankle height limits, on/off gain blend and 13-call decay follow sections 4 and 5. The default
mode (`--ground-ik native`) enables the solver only while Rico stands on a vehicle roof (the
rooftop-stunt controller 0x00701f30), the one enabling state with a runtime counterpart; retail has
no ground-IK enable for ordinary walking. `--ground-ik all` (walking slopes/stairs) is a non-native
extension, `off` disables it. Inferred, labelled in the code: filter predicates 1/3 (all but Rico),
second-ray plane arithmetic and ankle blend, ankle target formula, knee-limit widening, per-update
blend gain, no support identity (moving-platform anchoring unproven). Stunt-jump, grappling-jump,
cling-to-roof and rocket-baby enables have no runtime counterpart yet.
