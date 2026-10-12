# Sea height consumers: swimming, boats, splashes and the sea-level condition (JC2-189)

11 October 2026. Research only: no Cargo, GPU, game launch or runtime edit. Resolves Gap 3 of
[water-waves-next.md](water-waves-next.md) (JC2-165) and **replaces the "physical sea level stays 200" rule** of
[water-inputs-next.md](water-inputs-next.md) section 9 step 6. Probe: `research/tools/water_sea_consumers_probe.py` (138 checks, exits 0 only if
every one passes; pseudo-C fragments are asserted with whitespace normalised, call sites are found by scanning `.text` bytes and cross-checked
against the decompiled text). All addresses are runtime addresses of `JustCause2.exe` (sha256 `df99a479...4f80`, no ASLR).

```
python -B research/tools/water_sea_consumers_probe.py [--game <game folder>] [--pseudo <git-ignored _unnamed export dir>] [--json] [--verbose]
```

## Answers

1. **The sea is an interface owned by the physics system.** World setup (0x67ce40) creates a 4-byte `CPfxSea` (vtable 0xeb8f84, RTTI base
   `CPhysicsSystem::IPfxSea`), keeps a shared pointer to it at `this + 0x1f8` and publishes the raw pointer at **physics system + 0x38**
   (`DAT_0118fff4`, the singleton created by 0x5a76c0). Everything below reaches the sea through that pointer.
   * **slot 1** (`FUN_00744240`, no arguments): the flat sea level, always **200.0**.
   * **slot 2** (`FUN_0059ff30`, arguments x and z): the **wave height** `FUN_00a8bba0`, i.e. 200 + the 0.6 m gradient noise, faded to nothing
     66.67 m from the camera (the camera position is the reference passed to it). Beyond that distance it is exactly 200.
2. **Twenty-one call sites in twenty consumer functions** (byte scan of `.text`, matched one-for-one by the decompiled text). Nine
   functions use the wave height, eleven the flat level (table below). Neither rule of thumb ("all gameplay uses 200", "everything follows the wave") is
   right: **placement and buoyancy use the wave, state thresholds and triggers use 200**.
3. **Boats follow the waves** (resolves the "boats" half of Gap 3). The buoyancy driver `FUN_009893c0` (called from `CBoyancyAction` and `pfxSeaVehicle`)
   builds a default water plane from the flat level (slot 1) and then, per buoyancy sphere (`FUN_009a5c60`), replaces it with the plane
   `(0, 1, 0, -h)` where `h` is the **wave height at the sphere's own world x and z** (slot 2, with the physics-system origin offset +0x28/+0x30
   added). The submerged volume is the spherical cap `pi/3 h^2 (3r - h)` (immediates 1.0471976 and 4.1887903), summed per sphere; the total is scaled
   by 1100.0 plus a base term. So a boat bobs on the 0.6 m waves, and only within about 66 m of the camera. JC2-216 ([water-buoyancy.md](water-buoyancy.md)) resolves the rest: the 1100.0 is the mass of water carried per m^3 submerged (an added-mass rule), the force is `boyancy * V * 9820`, and the sphere radius is one shared value.
4. **Swimmers follow the waves**, with a state-dependent anchor (resolves most of the "swim" half of Gap 3). `CSwimController` (0x5ac120) compares the
   names:
   * `DAT_01196c34` = **`S_SWIM_SUBMERGE`** (written once, at 0xc7bc8f, by a name lookup): **no correction at all**.
   * `DAT_011970a8` = **`S_SWIM_RUN_TO_SWIM`** (written once, at 0xc7bc5f): the correction uses the world position of **body part slot 1 = the `Hips`
     joint** of the character (handle at the swim object `+0x330` = character `+0x420`; slot table in "JC2-215 results" below), `K = -0.15`, `G = 0.1`.
   * **every other state** (the controller is the one the factory registers beside the parachute controller; swim idle, navigation, rotate and so on)
     uses the character's own position (`+0x9c .. +0xa4`), `K = 0.05`, `G = 0.25`, and is skipped only when `dt` is exactly 0 (the paused-frame getter
     returns 1e-6, so in practice never).
   The vector is `(0, clamp(((K + w) / dt - v) * G, -10, 10), 0)` with `w = waveHeight(x, z) - y`, `v` = the controller's `+0x1b8` (the previous root-motion Y in m/s, see "JC2-220 results"),
   stored at `+0x1c0..+0x1c8`. The earlier note read the first branch as a "state" in general; it is the single state RUN_TO_SWIM.
5. **`CAtSeaLevelCondition` tests a body part of a `CCharacter`** against the wave surface (resolves the position index). The owner object is checked with
   the `CCharacter` type token (`FUN_0056aec0`); the position is the world transform of the handle in the character's **20-slot body-part handle array at
   `+0x41c`** (sentinel 0x14 means "none"), selected by the condition's `+0x10` index; the test is `y - wave(x, z)` against the threshold at `+8`
   (`<` when the byte at `+0xc` is 0, strictly `>` when it is 1). Defaults from the constructor (0x714ec0): threshold 0.0, byte 1, index 0, so a condition with
   no data means "body part 0 is above the wave surface". The three properties are registered by hashed keys at 0xec2a0c / 0xec2a14 / 0xec2af0 (one
   32-bit word each; resolved in "JC2-215 results": `IsLowerLimit`, `DistanceLimit`, `TestBoneIndex`).
6. **`CPfxSea` is not a particle effect.** The class implements the physics system's sea interface (`CPhysicsSystem::IPfxSea`); the
   "does CPfxSea use the return" question is moot: slot 2 *is* the return, and the consumers are the nine functions below.

## The consumers (slot: 1 flat 200, 2 wave height)

| Function | Owner (RTTI, from vtable or caller walk) | Slot | Use |
|---|---|---|---|
| 0x9a5c60 | CBoyancyAction, pfxSeaVehicle | 2 | per-sphere water plane `(0,1,0,-h)`, spherical cap volume |
| 0x9893c0 | CBoyancyAction, pfxSeaVehicle | 1 | default plane `-200`, sums the sphere volumes, x 1100.0 |
| 0x572770 | CVehicle family (CBoat, CSeaVehicle, CCar, ...) | 2 | water-entry splash impulse at `wave - 0.1`, scaled by entry speed |
| 0x6d7130 | CSimpleRigidObject, CTrafficLight (shared slot) | 2 | trigger at `y < 200.0` (cleared at `>= 200.2`), impulse placed at wave height |
| 0x77db80 | CDamageableObject | 2 | same water-entry impulse, placed at wave height |
| 0x6e3f60 | CBullet, CMissile, CGrenade, CGrapplingHook, CAmmunition | 2 | projectile hit on water: impact point y = wave height, then the impact virtual |
| 0x6fa510 | CCharacter water effect | 2 | effect origin y = wave height (default 200.0 when not in water) |
| 0x551ec0 | CCombatantComponent, CCivilianComponent | 2 | effect controller: an effect below 200 is snapped to the wave height |
| 0x7c8790 | CAnimatedModelEffects | 2 | model effect placement at the wave height |
| 0x518fe0 | CGrapplingHook | 2 | "submerged" when the wave is more than 2.0 m above the hook |
| 0x51c080 | CBellyStuntController | 1 | air/stunt test: a tracked height against `200 + 1.5` (use not fully decoded) |
| 0x51e620 | CMountedGunController | 1 | an object's depth below the flat level against a threshold (`DAT_010776a0`) |
| 0x64b4f0 | CSeaVehicle (engine/propeller update) | 1 | ground-ray hit height against the flat level |
| 0x794240, 0x79efe0, 0x814b80, 0x764c90, 0x67b4f0, 0x78eb50, 0x81d330 | stunt controllers, CGrapplingHook, CRagdollingController, CPlayer, CParachuteFromDriverSeatAction, CEnterVehicleAction and the other enter/exit/stunt actions | 1 | flat-level users; the comparison is not decoded here |

(The "use" column states only what the quoted fragments prove; the flat-level users other than the first three are listed without a decode, see Gap 5.)

## What this changes for the runtime

* The `jc2-game` sea level used for **swimming**, **boat buoyancy**, **splash/impact placement** and **sea-level script conditions** must be the CPU wave height
  (`water::sea_height_at(x, z, camera)` once it exists), not the constant 200. State machines and triggers keep the flat 200.
* The wave height is exactly 200.0 farther than 66.67 m from the camera, so the camera-relative evaluation is required for parity (the same function
  feeds the shader's displaced surface).
* Vehicles other than boats (cars, bikes) only use the flat level (stunts, enter/exit).

## JC2-215 results: the swim anchor joint, the controller-to-state map and the condition data

Probe section 8 (38 further checks; reads `biped.bsk` and the `.afsm` machines through `jc2tool`, pass `--tool` or build `target/release/jc2tool.exe`).

**1. The 20 body-part slots are skeleton joint indices, filled once per character by `FUN_007ee560`** (called first by the skeleton setup
world transform (the joint's model-space matrix from `FUN_008eb140` times the character matrix at `+0x15c`; a negative or too large index gives the identity).

| Slot | Offset | Joint | Biped joint | | Slot | Offset | Joint | Biped joint |
|---|---|---|---|---|---|---|---|---|
| 0 | 0x41c | literal 0 (root) | 0 `Reference` | | 10 | 0x444 | `LeftShoulder` | 10 |
| **1** | 0x420 | **`Hips`** | **1** | | 11 | 0x448 | `RightShoulder` | 14 |
| 2 | 0x424 | `LeftFoot` | 29 | | 12 | 0x44c | `AttachHandRight` | 25 |
| 3 | 0x428 | `RightFoot` | 28 | | 13 | 0x450 | `AttachHandLeft` | 24 |
| 4 | 0x42c | `LeftHand` | 22 | | 14 | 0x454 | `RightLeg` | 9 |
| 5 | 0x430 | `RightHand` | 23 | | 15 | 0x458 | `LeftLeg` | 6 |
| 6 | 0x434 | `Head` | 19 | | 16 | 0x45c | `RightUpLeg` | 7 |
| 7 | 0x438 | `Neck` | 18 | | 17 | 0x460 | `LeftUpLeg` | 4 |
| 8 | 0x43c | `Spine` | 2 | | 18 | 0x464 | `RightArm` | 15 |
| 9 | 0x440 | `Spine1` | 3 | | 19 | 0x468 | `LeftArm` | 11 |

So the RUN_TO_SWIM anchor is the **pelvis (`Hips`)**. Arithmetic of the proven formula `lift = ((K + w) / dt - v) * G`, `w = wave - y` (inferred reading as a rest height, with `v` the vertical-speed term): the lift is zero
when `w = -K`, i.e. the hips settle 0.15 m **below** the wave surface (`K = -0.15`); every other swim state anchors the character's own position (`+0x9c..+0xa4`) and settles at 0.05 m **above** the surface (`K = 0.05`). The aim-point function `FUN_005c2490` defaults to slot 9 (`Spine1`); `FUN_0059de00` overrides it with slot 8 (`Spine`,
raised by `DAT_0107d218`) in four states, slot 1 (`Hips`) and slot 6 (`Head`) under two predicates `FUN_00817ac0` / `FUN_006a5e90` (their meaning is not decoded here). The civilian
skeleton (`civilian_biped.bsk`) has `Hips` (also joint 1) but no `AttachHand*` joints, so those two slots resolve to joint 0.

**2. State to controller (the "controller-to-state data").** A character's controller is chosen by its current layer-0 state through the component at character `+0xa70`
(the player/agent component, vtable 0xec293c, class label `CCombatantComponent`; built by `FUN_00766260`, slot 5 `FUN_00541430` maps a state id to a controller hash, slot 6 gives the
default). The map is **native code, not data files**: state names are interned by `FUN_004b88e0`, controllers by the Jenkins hash of the class name (`FUN_00a165a0`, registered by the
factory `FUN_007e07c0`). `CSwimController` serves exactly seven states: `S_SWIM_IDLE`, `S_SWIM_NAVIGATION`, `S_SWIM_ROTATE_CW`, `S_SWIM_ROTATE_CCW`, `S_SWIM_RUN_TO_SWIM`, `S_SWIM_DIE`
and `S_SWIM_SUBMERGE` (these seven are listed before the agent/player branch, so both get them). The five `S_SWIM_DIVE_*` states (IDLE, NAVIGATION, ROTATE_CW, ROTATE_CCW,
DIE) go to `CDiveController`, so **diving has no wave correction from `CSwimController`**. The default is `COnFootController` (player) or `CAgentOnFootController` (agent branch,
chosen by a virtual at `+0x9c8 -> +0xd8 == 1`; what that predicate means is not decoded). Pedestrians (component vtable 0xec26fc, `FUN_005114e0`, character type `+0x161 = 1`) install only
`CPedestrianController` and `CRagdollingController`, so a civilian in `S_SWIM_IDLE` runs no swim controller.
Other mapped groups (not swim): mounted gun (`CUnmountedGunController`, 8 states), ragdoll (`CRagdollingController`, 12), skydive/parachute, grappled, reeled in/hanging.

**3. `CAtSeaLevelCondition` data.** The three properties are single 32-bit Jenkins keys: `0x3c9c0ff1` (byte `+0xc`), `0x49b4a2d6` (float `+8`), `0x77cfdc9b` (int `+0x10`). The spellings
**`IsLowerLimit`, `DistanceLimit`, `TestBoneIndex`** reproduce all three hashes (inferred: found by a name search, the strings are not in the image; each fits the field it
fills). Reading: with `diff = joint_y - wave(x, z)`, `IsLowerLimit = 1` (the default) means true when `diff > DistanceLimit`, `0` means true when `diff < DistanceLimit` (both strict);
`TestBoneIndex` is the body-part slot (default 0 = root). Instances (flag, limit, slot), all in `animations\statemachine\`:

| Machine | Condition | Flag | Limit (m) | Slot |
|---|---|---|---|---|
| basestatemachine (player) | `C_BELOW_SWIM_DEPTH` | 0 | -0.4 | 1 (Hips) |
| | `C_ABOVE_SWIM_DEPTH` | 1 | -0.05 | 1 (Hips) |
| | `C_AT_SURFACE` | 1 | -0.4 | 0 (root) |
| | `C_ENOUGH_ABOVE_WATER_TO_FALL` | 1 | 1.0 | 0 |
| | `C_ENOUGH_ABOVE_WATER_TO_SKYDIVE` | 1 | 28.5 | 0 |
| npc_combat_base | `C_BELOW_SWIM_DEPTH` / `C_ABOVE_SWIM_DEPTH` / `C_AT_SURFACE` | 0 / 1 / 1 | -1.2 / -0.5 / -0.4 | 0 |
| civilian (`animations.blz/civilian.afsm`) | `C_BELOW_SWIM_DEPTH` / `C_IN_WATER` | 0 / 0 | -1.2 / -0.5 | 0 |

Player entry and exit: the entry transitions `T_SWIM_ENTER_WATER` (from the upright, crouch and mounted-gun walk states), `_FROM_DASH`, `_FROM_STRAFE` and `_UPDATE` (evade, grapple smash,
crouch) all enter `S_SWIM_RUN_TO_SWIM` when the **hips are more than 0.4 m below the wave surface**; `T_AIRBORNE_TO_SWIMMING` enters `S_BLEND_TO_SWIM_IDLE` (with
`C_VERTICAL_SPEED_LOWER_THAN_SUBMERGE`) and `T_AIRBORNE_TO_DIVING` / `T_SWIM_DIVE_IDLE_DEPTH_CHECKED` enter `S_SWIM_DIVE_IDLE` on the same condition. Exit is `T_SWIM_EXIT_WATER` from `S_SWIM_NAVIGATION` /
`S_SWIM_RUN_TO_SWIM` to `S_UPRIGHT_BASIC_NAVIGATION` when the hips are above wave - 0.05 m (plus `C_REACH_SEA_FLOOR` and `C_SWIM_TO_UPRIGHT`). `T_SWIM_SURFACE` and `T_SWIM_IDLE_SURFACE_UPDATE` (root above wave - 0.4)
bring `S_SWIM_DIVE_NAVIGATION` / `S_SWIM_DIVE_IDLE` up to the surface states. NPCs enter `S_SWIM_IDLE` directly from the air (no blend state) with the root 1.2 m below the wave; civilians likewise. Falling is
refused (`C_ENOUGH_ABOVE_WATER_TO_FALL`) until the root is 1 m above the wave, the skydive until 28.5 m.

## JC2-220 results: what the swim lift does, what `v` is, and where the swimmer rests

Probe section 9 (33 further checks; needs a `jc2tool` that prints root-motion samples, built from the JC2-220 commit or later: `anim <game> animations.blz/<clip>.ban --at 0` now also prints `RM i x y z yaw` lines).

**1. `FUN_00898740` is a plain vector store, and the vector is added to the animation's root-motion velocity.** The function (0x24 bytes, three call sites: `CSwimController` update once, `CAutoParachuteAction`
(0x86bbf0, `step` passed down unchanged from the owner update 0x89c310) when `step > 0`:

```
root = (this+0x208)->vec[+0x1c..+0x24]     stored at this+0x1b4..0x1bc  (x negated when mirror != 0)
d    = (this+0x1c0..0x1c8) + root          (the lift vector plus the root motion)
pos += (d rotated by the character matrix at this+0x6c)  * step        (pos = this+0x9c..0xa4 = the matrix translation row)
```

So the lift is **a velocity in the character's local frame, integrated over the step, added to the animation's own root motion; not an impulse, and not on a physics body** (the move is a direct write of the
character matrix translation). `FUN_00670b50` (`CCharacter` virtual `+0x64`) clears it to zero (state resets). For the swim states the matrix is the yaw-oriented character matrix, so `(0, lift, 0)` stays vertical
(inferred: swim states do not pitch the matrix; the dive states are not served by this controller). The parachute action uses the same channel for its own vertical term (not decoded).

**2. `v` (`+0x1b8`) is the vertical component of the root-motion velocity from the previous `FUN_00895f80` call** (the Y of the vector the animation object at `+0x208` supplies, in m/s). It is the value
`FUN_00895f80` itself stored at `+0x1b8`, so `CSwimController` reads whatever the most recent animation step stored (the update ordering between the controller and `CLightCharacter` is not traced). Units fit: `(K + w) / dt` is a speed, `lift` is a speed, and the clips'
extracted motion is a speed (swim_fwd travels 4.4 m in 1.333 s = 3.30 m/s). The term subtracts the clip's own vertical speed from the correction so the animation does not fight it. Extracted root motion in the
swim clips (`kc001_base.asb` -> `animations\character\shared\inwater\*.ban`, sampled at 30 Hz, engine axes; forward is -z):

| State (clip) | Root-motion Y (v) | Notes |
|---|---|---|
| `S_SWIM_IDLE` (swim_idle) | none, 0 | Hips y -0.568 .. -0.527 below the root over the loop |
| `S_SWIM_NAVIGATION` (swim_fwd/lft/rgt) | 0 (z only, 4.4 m in 41 samples) | Hips y -0.360 .. -0.196 |
| `S_SWIM_ROTATE_CW/CCW` (swim_turn_*) | none, 0 | Hips y about -0.51 |
| `S_SWIM_RUN_TO_SWIM` (run_to_swim) | 0 for the first 4 and last 3 intervals, **+1.43 m/s** between; total +1.3828 m | hips start 0.932 above the root |
| `S_SWIM_SUBMERGE` (swim_submerge) | -0.59 m/s (-1.752 m in 3 s) | no correction in this state anyway |
| `S_SWIM_DIE` (swim_die) | none, 0 | 2-frame pose, no Reference track |

**3. The character position coincides with the root joint (slot 0) in the swim clips.** The position used by every non-RUN_TO_SWIM state is the translation row of the character matrix (`this + 0x6c`, which is
the `+0x15c` matrix `FUN_00677d60` multiplies a joint's model-space matrix with). In all eight swim clips the `Reference` track (joint 0, body-part slot 0) is static with translation (0, 0, 0) and identity rotation, so
the root joint's world transform is the character matrix itself: **slot 0 = the character position**, no offset in the clips. (Runtime IK, ragdoll or blend with a non-swim clip could still offset joint 0; unmodelled.)
The Hips joint (parent: Reference) carries the body offset instead: -0.53 m in idle.

clock's step, 1e-6 when paused), one frame moves the root by `(v + lift) * step = G * (K + w)` and `dt` cancels: **the swimmer closes exactly G of the gap to its rest height every frame** (0.25 in idle, navigation and
rotate: 0.75 of the error remains per frame; 0.1 in RUN_TO_SWIM), with the +-10 m/s clamp limiting a frame to 10 * step. The rest height is `w = -K`:

* idle, navigation, rotate and die (character position): **root at wave + 0.05 m**, hips at wave + 0.05 - (0.527 .. 0.568) = wave - 0.518 .. wave - 0.477 in idle, so hips stay below the player machine's
  `C_BELOW_SWIM_DEPTH` limit of -0.4 and far below `C_ABOVE_SWIM_DEPTH`'s -0.05 (a resting swimmer does not fall back to the upright states).
* `S_SWIM_RUN_TO_SWIM` (Hips): the lift is zero at hips = wave - 0.15, but `v` is +1.43 m/s for 29 of 36 intervals, so the clip's own rise dominates and the controller only pulls with weight G = 0.1 (net
  `(1 - G) * v` = 1.29 m/s when the gap error is zero). The clip is authored for the entry threshold: the swimmer enters when the hips are 0.4 below the wave, the hips are 0.932 above the root at clip start, so
  the root starts at wave - 1.332; the clip adds 1.3828, ending at **wave + 0.051**, the idle rest height K = 0.05 to within 1 mm. This agreement (a 1 mm match between two independent data sources) is the
  strongest evidence that the character branch's rest is `root = wave + 0.05`.
* bobbing: the root follows `wave(x, z)` at that rest height with first-order lag (25 % of the gap per frame); the horizontal root motion of navigation moves `x, z`, so the sampled wave changes under the swimmer.

For the runtime: apply `pos.y += (rootmotion_y + clamp(((K + wave - anchor_y) / dt - rootmotion_y_prev) * G, -10, 10)) * dt` per frame, anchor = Hips world y (RUN_TO_SWIM, K -0.15, G 0.1) or the character
position y (all other CSwimController states, K 0.05, G 0.25; skipped when `dt == 0`), with the previous frame's root-motion Y as `v`.

**Proven / inferred (JC2-220).** **Proven** (bytes plus asserted fragments): `FUN_00898740` is the three-float store at `this+0x1c0` (three call sites); `FUN_00895f80` (CCharacterBase virtual `+0x70`) adds that vector to the root-motion vector,
rotates it by the matrix at `this+0x6c` and adds `* step` to the matrix translation `this+0x9c..`; `+0x1b8` is the Y of the root-motion vector stored by that same function; the swim clip root joint is static at the
origin; the root-motion Y of every swim clip (table above). **Inferred**: that the vector read from the `+0x208` object is the clip's extracted root motion in m/s (units and the clip data fit, the writer is not
traced); `step` equals the controller's `dt`; the swim matrix keeps `(0, lift, 0)` vertical; the rest height `wave + 0.05` is the fixed point of the proven formula, corroborated by the 1 mm entry-arithmetic match.

## Proven / Inferred

**Proven** (pinned bytes plus asserted fragments): the sea pointer chain, slot 1 = 200.0, slot 2 = `FUN_00a8bba0`; the 21 call sites and the 20
function set (byte scan and decompiled text agree); the swim state names and their ids' single writers; the three-way branch and its constants; the
`CCharacter` type check and the 20-slot array with the 0x14 sentinel; the condition's defaults, comparison and property offsets; the buoyancy plane,
cap-volume immediates and scale 1100.
**Inferred**: the meaning of K's sign as a rest height (below/above the surface; the formula itself is proven); that `CBoyancyAction`
and `pfxSeaVehicle` pass sphere positions in the physics-origin-relative frame (consistent with the +0x28/+0x30 additions); the semantic labels in the
"use" column that are not quoted from constants; the spellings `IsLowerLimit` / `DistanceLimit` / `TestBoneIndex` (hash-equal to the three native keys, but the
strings are not in the image). The anchor handle being slot 1 is now consistent with the constructor: `CCharacter` also installs its vftable at word 0x3c (offset 0xf0),

## Gaps (each stays a native unknown; none is filled by a guess)

Resolved by JC2-215: the identity and producer of the body-part slots (old Gap 1), the controller-to-state mapping and the `CAtSeaLevelCondition` keys and instances (old Gap 2).

1. **Swim correction application and rest height in the animation.** Resolved by JC2-220 (section above): the vector is a velocity added to the animation root motion by `FUN_00895f80`; `v` is the
   previous root-motion Y; slot 0 = the character position in the swim clips; the rest root height is wave + 0.05 (character branch). What remains is listed in the JC2-220 gaps below.
2. **Buoyancy model.** Resolved by JC2-216: see [water-buoyancy.md](water-buoyancy.md) (force, drag, added mass, sphere radius and centre sources, the base term, the sinking bias); its own Gaps list what remains.
3. **Splash and impact responses.** Resolved by JC2-217: see [water-splash-impact.md](water-splash-impact.md) (gates, magnitudes, the +0x2c virtual = impact-effect trigger, projectile water hits); its own Gaps list what remains (effect assets, character operands, vehicle water state). Original text: the constants and effects behind the water-entry impulses and effect placement (0x572770, 0x6d7130, 0x77db80, 0x6e3f60,
   0x6fa510, 0x551ec0, 0x7c8790): speed thresholds (200.2 / -0.5 / -1.0 entry gates, DAT_0106fae8/0106fae4, DAT_0107281c/10728xx), the impulse virtual
   (vtable +0x2c) and which effect it spawns. Blocks: splash, bullet-hit-water and wake parity (board task JC2-217).
4. Not blocking visible parity, kept here: the flat-level users' exact thresholds (stunt, parachute, enter-vehicle, ragdoll, grappling hook), `CSeaVehicle`'s use
   of the surface probe, and the unresolved `CPfxSea` slots 0, 3 and later (0x796f10 destructor, 0x744240 aside; slot 3 `0x100002c` is data); the meaning of the predicates
   `FUN_00817ac0` / `FUN_006a5e90` (aim-point joint choice in `FUN_0059de00`) and of the agent/player discriminator (`+0x9c8` virtual `+0xd8`); the string spellings of the three condition keys
   (hash-equal names only); `C_VERTICAL_SPEED_LOWER_THAN_SUBMERGE`, `C_REACH_SEA_FLOOR` and `C_SWIM_TO_UPRIGHT` (other condition classes, not decoded here).

JC2-220 gaps (all native unknowns, none guessed):

5. **Producer of the root-motion vector at `(this+0x208)+0x1c..0x24`** (`NCharacterSystem::CCharacterBase`, read by `FUN_00895f80`): who writes it, how the extracted-motion samples of the playing clip(s) are
   sampled, scaled by playback rate and blended across transitions and layers, and what the alternative path `FUN_008869d0` (taken when byte `+0x61` is set) does. Blocks: the exact `v` during clip
   cross-fades (the first frames of a swim state) and, more broadly, root-motion parity for every animated state. Filed as a research task.
   `CSwimController` and the `CLightCharacter` animation step (decides whether `v` is the previous or the current frame's root motion). Not blocking visible parity at 30/60 fps.
7. Runtime offsets of joint 0 after pose blending (foot IK, ragdoll blend, upper-body layers): the swim clips themselves leave the root at the origin. Not blocking.
8. `CAutoParachuteAction` (0x771690) states 2 and 3 use the same channel with `(0, y, 0)`, `y = field(+0x38) * DAT_01079888 (1.0) + DAT_01079884 (25.0)`; its meaning is not decoded here (parachute scope).
