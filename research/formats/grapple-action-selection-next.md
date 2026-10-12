# Grapple action selection, reel/pull dispatch and arrival classification (JC2-168, JC2-169)

Research brief for the Rico grapple runtime (JC2-036/038/039/238). Nothing here changes
gameplay; every claim carries a native address and `research/tools/grapple_action_probe.py`
re-checks the byte-level parts against the executable, reads the shipped data and runs the
fixtures. It closes Gaps 1-4 of `animation-statemachines-next.md` (JC2-106) except where the
Gaps list at the end says otherwise.

## Headline

* **The device decides, once, at hook attach.** `0x00538470` (hook hit) calls the selector
  `0x007f3db0` before it creates the wire. The selector flips two flags of Rico's
  `CGrapplingActionSet`: *ReelIn* (CReelInAction) or *PullGrappled* (CPullGrappledAction).
  Rule: world/terrain hit or a hit on a `CBreakableTreePatch` -> **Reel**; hit object is a
  `CCharacter` -> **Pull**; otherwise the hit object's `grappling_weight_category`
  (JC2-037, byte `+0x34 & 0x7f`, getter `0x0068be90`): **1 -> Pull, 2 -> Reel**, every other
  value -> neither (only FireHook stays enabled). With a wire already present (second hook of
  a dual hook) Reel, Pull and DeployParachute are all switched off. The user's "lighter than
  Rico" rule is not in this code: mass is never compared.
* Both actions are **forced triggers** in the shipped data (`#2cf00dcb = 1`): once enabled and
  available they start on their own, no key press is needed. All six actions carry
  `priority = 10` in `settings/grapplingparams.bin`; the constructor defaults (reel 100, pull
  5) are overwritten by it. Reel and Pull share control id 0, so the per-id arbitration
  (`0x008dbf90`) would keep the earlier-registered Reel on a tie, but the selector never
  enables both, so the tie does not occur.
* **Arrival is classified by the end-surface normal's Y** against two device angles: `>= cos
  50 deg` floor-like -> `ACT_REEL_LAND`; `cos 140 deg < y < cos 50 deg` wall -> `ACT_REEL_IMPACT_VERTICAL`;
  `y <= cos 140 deg` ceiling-like -> `ACT_REEL_IMPACT_HORIZONTAL`. `ACT_DETACH` is the abort.
* The grappled victim has no action of its own: NPC states `S_GRAPPLED` / `S_REMOVE_GRAPPLINGHOOK`
  are driven by a per-character hook counter (`char+0x9e8`) through `CGrappledCondition`.

## Fingerprints

`JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
The probe pins the bytes of 45 cited functions, 28 direct call edges, 24 vtable slots, 5
`.rdata` floats, the six-name action table at `0x010777ac`, the control-id table entries
(`0x51 FIRE_GRAPPLE`, `0x52 GRAPPLING_ACTION`, `0x90..0x92`), the context-action property keys,
and the **name of 43 animation action/state globals** (see "Naming the DAT globals").
Data: `settings/grapplingparams.bin` (`jc2tool prop`) and the eight `.afsm` machines
(through the JC2-106 decoder). Fixtures: 12 selector, 10 surface, 3 arbitration, 7 pull
availability, 17 reel availability cases and the NPC condition bands.

### Naming the DAT globals

The pseudo-C refers to action and state ids as `DAT_01196xxx`. Each is created by a static
initialiser of the form `push "NAME"; call hash; push handler; mov [global], eax`
(block near `0x00c81e01`, one record per name). Scanning the code for that pattern gives the
name of every global (1,889 in total); the probe asserts the 43 used here, e.g. `0x011969d8`
ACT_REEL, `0x01196808` ACT_GRAPPLE_PULL, `0x011967c4` S_REEL_FLIGHT, `0x01197104` LA_ACT_IDLE
(this is the "second left-arm action" `01197104` of JC2-106 Gap 2: it is `LA_ACT_IDLE`).
The layer object API of Gap 2: `0x0073f7f0(id, layer)` = virtual `+0` = post/dispatch the
action, returns accepted; `0x00810360(id, layer)` = virtual `+4` = would the layer accept it
(inferred from the two call patterns; layer 5 = all layers).

## 1. Machinery

* **Registry.** `0x0059d460` registers one `CContextActionCreator<T>` per action class in
  `CContextActionFactory`, keyed by the hash of the class name (vftables `0x00ec3270` ReelIn,
  `0x00ec3300` PullGrappled, and the others). The creators only allocate; nothing chooses
  between actions there.
* **The set.** `0x005beed0` (player-style init, no direct caller) loads grapplingparams
  (`0x006dca90` -> `0x00842080`). `0x00842080` creates six actions by name, in this order and
  with this enable index: 0 `CFireHookAction`, 1 `CReelInAction`, 2 `CPullGrappledAction`,
  3 `CDeployParachuteWhileReelingAction`, 4 `CReeledInJumpAction`, 5 `CReeledInReleaseAction`
  (string table `0x010777ac`), and calls each action's property loader (vtable slot 8,
  `0x0089f290`) with the matching child of the `actions` node. The set lives at `character+0x628`
  (`CGrapplingActionSet`, entries 12 bytes: action, refcount, control id; enable flags at
  `+0x48+i`). `0x005beed0` enables **only index 0**. `0x00732c90(i, b)` sets a flag.
* **Per frame.** `0x007d5320` -> `0x005748b0` registers every *enabled* action with the
  controller (component slot 11). `CCharacterController::update` (`0x00882070`, slot 3 of all
  31 controller classes) then: (1) `0x008dbf90` asks every registered action `slot 4` =
  *available(character)* and keeps the available ones in a vector sorted by control id
  (slot 3 = id, `+0x24`); if two share an id, the new one replaces the old **only if its
  priority (slot 11, `+0x28`) is strictly greater**; (2) `0x008d2ba0` walks that vector and, for
  the first action whose slot 5 *triggered* is true (`0x008b7b80`: forced flag `+0x30`, else the
  control's record is PRESSED), interrupts the current action (`+0x234`, its slot 7 once with
  the abort flag `+0x33`) and calls slot 6 *start*; (3) `0x0088f530` ticks the current action's
  slot 7 *update*; true = finished.
* **Data (`actions` node of grapplingparams, keys per `0x0089f290`).** For each action: `#2cf00dcb`
  -> `+0x30` forced trigger, `#70f24645` -> `+0x32`, `priority` -> `+0x28`, `#eeca5657` ->
  `+0x24` control id, `#742cdd23` -> `+0x2c`, `#f6d62ea4` -> `+4` hud icon string.

  | Class | forced | priority | control id | hud icon |
  |---|---|---|---|---|
  | CPullGrappledAction | 1 | 10 | 0 | - |
  | CReelInAction | 1 | 10 | 0 | - |
  | CReeledInJumpAction | 0 | 10 | 144 (REELED_IN_JUMP_ACTION) | hud_button_09 |
  | CReeledInReleaseAction | 0 | 10 | 145 (REELED_IN_RELEASE_ACTION) | hud_button_08 |
  | CFireHookAction | 0 | 10 | 81 (FIRE_GRAPPLE) | - |
  | CDeployParachuteWhileReelingAction | 0 | 10 | 146 (DEPLOY_PARACHUTE_WHILE_REELING_ACTION) | hud_button_04 |

  This answers controls-next.md Gap 6 for the grapple actions (the ids are data, and the
  parachute action is 146, not 71) and Gap 1's ordering rule for them. Constructor defaults
  (`0x007f2c90` reel: id 0x52, priority 100; `0x0070f830` pull: id 0x52, priority 5, pull length
  `+0x38 = 6.0`) are overwritten by the data.

## 2. The selector (`0x007f3db0`, called only from `0x00538470`)

`hit` is the game object that was hit; `kind` is `grappling_weight_category` (7-bit, authored in
the entity data, JC2-037: 0 default/destroyed/scenery, 1 barrels/tubes/small props and the
sentry gun, 2 structure/heavy objects, SAM sites, big cannons, vehicle components). The
validity check `0x0079efe0` refuses kind 0 for the pair step, but the selector runs before and
regardless of it.

| Condition (in order) | Flags written |
|---|---|
| (always) | FireHook := on |
| a wire already exists (`device+0x128 != 0`, second hook) | Pull, Reel, DeployParachute := off; stop |
| no hit object (terrain, static geometry, or a `CBreakableTreePatch`) | **Reel := on** |
| hit object is a `CCharacter`, or `kind == 1` | **Pull := on** |
| `kind == 2` | **Reel := on** |
| any other kind (0, 3..127) | nothing touched |

Flags are cleared elsewhere: Reel is cleared by `CReelInAction::start` itself
(`0x008098c0`); the dual-hook branch of `CFireHookAction` update (`0x0061d6b0`) clears Reel and
Pull; the device cancel `0x005fbcb0` restores the initial state (only FireHook on) and
`0x005a0530` (device detach, posts `ACT_DETACH`) clears Pull, Reel and Deploy; the arrival
dispatchers clear Deploy and Reel and enable Release/Jump (section 5).

## 3. Per-frame availability (vtable slot 4)

**Pull `0x0065c800`.** False when `character+0x324 != 0` (a vehicle/seat related field;
meaning inferred), or when the layer-0 state is `S_JUMP_START_SPRINT_FORWARD` or
`S_JUMP_TO_FALL_TRANSITION`, or when the first wire end (`0x00687c30`: end record
`count-2`, the hooked object) is a `CCharacter` whose type flag `char+0x9b8` bit 0 is clear.
Otherwise true (also true with no device). That flag is NOT bit 1 of byte `+10` of the
character type record (copied in `0x0077ce50`) and is forced clear for record type 7; a hook on
such a character also gets its wire age set to the limit minus 0.06 s (`0x00538470`), i.e. the
wire expires at once.

**Reel `0x005c2df0`.** In order: false unless every layer is ready (`0x00575950`, state flag
`0x8000`) and `character+0x324 == 0`. If the layer-0 state is a parachute state
(`S_PARACHUTE`, `S_UNFOLD_PARACHUTE_HORIZONTAL/VERTICAL`, `0x0076fc30`): when the hooked object
is a `CVehicle` that is not a `CSeaVehicle` it dispatches `ACT_PULLCHORD` (the chute is cut) and
continues, otherwise it is false while `device+0x40 <= character+0x2c8` (two timers; roles
unresolved). If `[x+0x40] == 2` (`0x0099a440`, owner unresolved) with a wire, FIRE_GRAPPLE not
held and the squared distance to the hook point `< 1.0`, the hook is cancelled and it is false.
In a default stunt position (`S_DEFAULT_STUNTPOSITION_IDLE` and its two 180 degree turns,
`0x00817ac0`) with a wire: false (hook cancelled unless held) when the hooked vehicle is the one
Rico stands on; true for another vehicle; for a non-vehicle end with the floor-like class and
squared distance `< 16.0` it is false and the hook is cancelled. Otherwise true.

## 4. Start and update: which animations are driven

Layers: character `+0x48c` array, stride 8: 0 main body, 1 upper body, 2 left arm, 5 = all.

**Reel start `0x008098c0`.** Requires the device and (layer-0 state already `S_REEL_FLIGHT` or
`ACT_REEL` accepted on layer 0); otherwise the action ends at once (`+0x33 = 1`). Graph: from
ground idle/strafe/landing states `ACT_REEL` -> `S_REEL_START` (`reel_in_start.ban`, then
`S_REEL_FLIGHT`); from airborne, swim-dive and stunt-position states `ACT_REEL` -> `S_REEL_FLIGHT`
(`grpl_reel_flight_part2.ban`). When the state is `S_REEL_START` it writes the blend input
`controller+0x3c = clamp(signedAngle(facing, direction to hook) / 140 deg, -1, 1)` (constant
`0x010803fc` = 2.4435 rad; sign = cross product y; acos of the normalised horizontal dot).
Then `LA_ACT_REEL` on layer 2 and the Reel flag off.

**Reel update `0x006b8890`** (action field `+0x34` = case). It cleans up and finishes at once
when the abort flag `+0x33` is set or both wire/state predicates `0x005d2a40` and `0x0065a030`
are false (meaning unresolved).

| case | does |
|---|---|
| 0 | wait for layer 0 = `S_REEL_FLIGHT`; then set character mode 9, start velocity, `LA_ACT_IDLE` on layer 2, **enable DeployParachute**; case 1 |
| 1 | upper-body script `UB_REELED_IN_MOD` on layer 1, mode 9, tick `0x00515cb0` each frame. Tick 1 = flying; 3 = ended without arrival; 2 = arrived. FIRE_GRAPPLE pressed while flying, or tick 3 -> `ACT_DETACH`. Arrival -> classification of section 5 |
| 2 | (horizontal impact) wait for `S_REELED_IN_TO_HANG`, set the stunt/hang pose; case 4 |
| 3 | wait for `S_REEL_IMPACT_VERTICAL` / `S_REEL_END_LAND`; clear Deploy, Jump, Release; case 4 |
| 4 | finish when the layer reports finished (restores mode 0 on the ground) |

The tick `0x00515cb0` ramps speed `v *= (3.5*dt + 1)` (`0x0106d8ac` = 3.5) up to `reel_in_speed` (40), moves
the owner along the wire toward the first end, and runs the contact sweep `0x00722cc0`.

**Pull start `0x007ae2e0`.** Needs the device and a wire. If layer 0 would **not** accept
`ACT_GRAPPLE_PULL` (i.e. Rico is on the ground: the graph offers it only from `S_FALL`,
`S_JUMP_FLIGHT_STANDING`, `S_JUMP_TO_SKYDIVE`, `S_SKYDIVE`, `S_SWIM_DIVE_IDLE`,
`S_BLEND_TO_SWIM_IDLE`, `S_REEL_FLIGHT`, `S_PARACHUTE_DETACH_TO_FREEFALL`): post
`LA_ACT_GRAPPLE_PULL` on layer 2 only (left arm `LA_S_GRAPPLE_PULL`, `grpl_idle_to_pull.ban`,
`grpl_pull.event_track`, blend 0.14), sub-state 0. If it would accept: post `ACT_GRAPPLE_PULL`
on layer 0 (`S_PULL_IN_AIR_FLIGHT`, clip `grpl_reel_flight_part2.ban`, same clip as the reel
flight) and `LA_ACT_IDLE` on layer 2, mode 9, ground IK off, and remember the distance minus 6 m.
**Pull update `0x00510a60`.** Sub-state 0 (ground): keep aiming at the hook, finish when layer
2 leaves `LA_S_GRAPPLE_PULL`. Sub-state 1 (air): runs the same device tick `0x00515cb0` (so the
owner is pulled along the wire) until the wire has shortened by 6 m or the tick is not 1, then
posts `ACT_FALL` on layer 0 and cancels the hook.

**Left arm graph** (`kc001_leftarm.asb`, `leftarmstatemachine.afsm`): `LA_ACT_FIRE_GRAPPLE`
-> `LA_S_RAISE_GRAPPLE` (`grpl_fire_ani_con.ban`, `grpl_fire.event_track`) -> `LA_S_WAITING_FOR_HOOK`
(`grpl_fire_aim_con.ban`) -> `LA_ACT_HOOK_HIT` (posted by `0x00538470`) -> `LA_S_HOOK_HIT`
-> `LA_ACT_REEL` -> `LA_S_GRAPPLE` or `LA_ACT_GRAPPLE_PULL` -> `LA_S_GRAPPLE_PULL`;
`LA_ACT_HOOK_MISSED` is posted by the device update `0x0083fe20`, `LA_ACT_DETACH` by the cancel
`0x005fbcb0`; `LA_ACT_DUAL_GRAPPLE` -> `LA_S_DUAL_GRAPPLE` (`grpl_fire_dualhook_con.ban`).
`LA_ACT_FIRE_GRAPPLE_SO` and `ACT_RELEASE_TO_FALL` have **no dispatcher** in the code.

**Fire (`CFireHookAction::update`, `0x0061d6b0`).** Standing: optional `ACT_ROTATE_CW/CCW`
(turn toward the aim, sign from the cross product) then `LA_ACT_FIRE_GRAPPLE` on layer 2;
reeled or hanging: `ACT_REELED_FIRE_HOOK` on layer 0 (`S_REELED_FIRE_HOOK`
`grpl_fire_fwd_jump.ban`, `S_REELED_WAIT_FOR_HOOK` `grpl_fire_fwd_loop.ban`, or from a hang
`S_HANG_FIRE_HOOK`); second shot with a wire: `LA_ACT_DUAL_GRAPPLE`, Reel and Pull off.

## 5. Arrival classification and dispatchers (JC2-169)

`0x00549490` = classify(`0x007fddd0`): the Y of the first wire end's hit normal (record `+0x30`,
object-local, rotated by the object's matrix; default `(0,1,0)`; characters give `+Y`) with the
device cosines loaded from grapplingparams: `+0xf4 = cos(#00992cfb = 50 deg)` and
`+0xf0 = cos(#8e43b31e = 140 deg)` (`0x00802ec0`, engine polynomial sine).

| Class | normal.y | Dispatcher | Graph state / clip |
|---|---|---|---|
| 0 | `>= cos 50` | `ACT_REEL_LAND` from `0x006b8890`, then velocity clamp `0x006a26c0`; case 3 | `S_REEL_END_LAND` `grpl_reel_hori_land.ban` |
| 1 | `cos 140 < y < cos 50` | `0x00564b20`: `ACT_REEL_IMPACT_VERTICAL`, Deploy/Reel off, **Release + Jump on**, mode 0xb | `S_REEL_IMPACT_VERTICAL` `grpl_reel_impact.ban` -> `S_REELED_IN_IDLE` `grpl_reeled_timeblend.ban` |
| 2 | `<= cos 140` | `0x005e6b90`: `ACT_REEL_IMPACT_HORIZONTAL`, Deploy/Reel off, **Release on**, hang pose `S_HANGSTUNT_IDLE` | `S_REEL_IMPACT_HORIZONTAL` `reel_impact_horizontal.ban` -> `S_REELED_IN_TO_HANG` `reel_impact_to_hang.ban` |

Other dispatchers: `ACT_DETACH` from the reel update, from `CReeledInController` slot 6
(`0x00570810`, when the wire is gone), from `0x005a0530` (device detach: resets the flags, posts
`ACT_DETACH`, cancel), from `CGrapplingJumpAction` slot 4 and from the monitor `0x005d2c10`, which (for a hook on a
vehicle, within `0x0108041c` = 16.0 squared metres of the end) cancels and posts `ACT_REEL_LAND`
when `0x0099a440` holds, else `ACT_DETACH`. `ACT_REELED_FIRE_HOOK` only from the fire action. `ACT_REEL_TO_SO` only from
`CReelToSOAction::start` (`0x0061ee20`; that action is registered but is not part of the
grappling set, its trigger is unresolved). `ACT_FORCE_RELEASE_GRAPPLINGHOOK` from `CCharacter`
slot 6 (`0x006b3210`). `ACT_GRAPPLE_SMASH` / `LA_ACT_GRAPPLE_SMASH` from `0x00617ae0` (searches
targets within 2.5 m).

**Condition classes** (loaders; the NPC graphs):

* `CGrappledCondition` (`0x00677760`): `min <= character+0x9e8 <= max`, keys `689d08f9` (min)
  and `d7f292c6` (max). `C_GRAPPLED_ATLEAST_ONCE` = 1..42, `C_NOT_GRAPPLED` = 0..0. The counter is
  incremented by `0x007513d0(4)` (the hook-hit handler does this for a hit character) and
  decremented by `0x0069b380`.
* `CGrappledSurfaceNormal` (`0x0077a720`): `cos(a) - 1e-4 <= normal.y <= cos(b) + 1e-4` with
  `a` = key `fad9419e`, `b` = key `867709d9` in degrees, same normal as above. Shipped: IMPACT
  (180, 60) i.e. y in [-1, 0.5]; RULLEBUSS (60, 0) [0.5, 1]; RELEASE (135, 0) [-0.707, 1];
  RELEASE_UPSIDEDOWN (180, 135) [-1, -0.707]. These NPC bands (60 degrees) are **not** the 50/140
  device angles that drive Rico.
* `CReelLandCondition` (`0x007b80b0`): evaluated on the character's reel state; semantics
  unresolved (it reads `character+0x230/+0x238` and a 0.2 threshold).

## 6. What a Rust implementer needs

1. A per-Rico `GrappleActionSet { enabled: [bool; 6] }` in the order of section 1, FireHook on at
   spawn. No priority/ordering code is needed for Reel versus Pull: the selector enables at most one.
2. At hook attach (valid or not, before the wire exists) run the section 2 selector with
   `hit = None | {is_character, grappling_weight_category}`. Treat tree patches as `None`. Use the
   authored category (JC2-037 reader), never mass.
3. Each frame, a started action needs `enabled && available`: implement the two predicates of
   section 3 (the opaque terms may be wired to `false`/`true` per the table, noted as such).
   Forced triggers: they start in the same frame the hook attaches. Reel start disables Reel.
4. Reel: dispatch `ACT_REEL` on the body layer (graph picks `S_REEL_START` or `S_REEL_FLIGHT`),
   `LA_ACT_REEL` on the left arm, the S_REEL_START blend input of section 4, then the case machine
   of the reel update with the classification table of section 5 and the clips listed there.
5. Pull: ground = left-arm `LA_ACT_GRAPPLE_PULL` only; air = `ACT_GRAPPLE_PULL` on the body layer
   (`S_PULL_IN_AIR_FLIGHT`), 6 m of wire shortening, then `ACT_FALL`.
6. Hook flow on the left arm: fire -> raise -> waiting -> `HOOK_HIT` / `HOOK_MISSED`; `DETACH` on
   cancel. Hit characters get `+0x9e8 += 1` so the NPC graph enters `S_GRAPPLED`.
7. Do not invent a "heavy/light" mass test, a priority ladder, or a player key press for Reel/Pull.

## Gaps (unresolved native details)

1. **Effective `grappling_weight_category` of `CCharacter`, vehicles and their body parts.** The
   getter reads byte `+0x34 & 0x7f` (default 0, ctor `0x008f0ab0`; `CCompositAnimatedObject` slot 18
   `0x007bef10` returns the parent's). JC2-037 found no key on characters; yet the validity check
   `0x0079efe0` refuses kind 0. Pull for characters does not depend on it (isA CCharacter), but
   which hits are *valid* does. Where the character/vehicle value is set is not located.
2. **(Decoded in `grapple-reel-flight-next.md`, JC2-246; residual gaps listed there.)
   Reel flight tick `0x00515cb0` and contact sweep `0x00722cc0`:** speed ramp constant
   `DAT_0106d8ac`, stop distance `device+0x50`, the `0.71` direction test, the timers at
   `device+0x24/+0x28/+0x2c/+0x30/+0x34` and the meaning of return value 3 versus 2 inside the
   sweep. Blocks reel flight timing parity.
3. **(Mostly decoded in `grapple-reel-flight-next.md`, JC2-246: flag `0x8000` is `CanReelInState`,
   `device+0x168` is never set, `0x0099a440` is `[x+0x40] == 2`.) Reel availability opaque terms:** `0x00575950` flag `0x8000`, the `device+0x40` /
   `character+0x2c8` timers, the owner of `0x0099a440`, `character+0x324`, `character+0x9b8`
   source field in the character type record, and what clears the hook-valid flag `device+0x168`.
4. **Effect of Pull on the target** (light prop or soldier): the pair impulse is in JC2-090, but the
   air pull also runs the owner reel tick; who flies a hooked soldier to Rico, and whether the NPC
   states `S_REEL_FLIGHT`/`S_REEL_IMPACT` are ever entered (no dispatcher of `ACT_REEL` outside
   `CReelInAction` exists, and the set is created only by `0x005beed0`), is open.
5. **`CFireHookAction` sub-state machine** (ten sub-states, aim thresholds `0x01079878` = 1.3 rad (rotate
   instead of fire above it), `0x010777cc` = pi/2, range `device+0x118`, dual-hook gate `0x00816300`) and the animation event
   (`character+0x930`) that also posts `LA_ACT_FIRE_GRAPPLE`.
6. **`CReelToSOAction` trigger** (smart objects), `CGrapplingJumpAction`, `CReeledInJumpAction`,
   `CReeledInReleaseAction` and `CDeployParachuteWhileReelingAction` bodies (input rules are in
   controls-next.md; the animation dispatch of the last three is not decoded here), and the
   controllers `CReeledInController` / `CReeledHangController` / `CGrappledController` beyond
   the dispatches listed.
7. `CReelLandCondition` semantics; `0x005d2c10`'s caller; `0x005beed0`'s caller (which class
   owns the player set).
