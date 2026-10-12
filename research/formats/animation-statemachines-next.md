# Animation state machines: grapple, stunt and combat graphs (JC2-106)

Research only, 11 October 2026. Reproduce with `research/tools/afsm_probe.py` (read-only; it
reads graphs in memory through `jc2tool prop --json`, writes no assets, exits 0 only when all
fixture assertions hold). Retail executable SHA-256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Addresses are image VAs.
Machines are members of `archives_win32/pc0`: `animations\statemachine\*.afsm` and
`animations\animationsets\*.asb` (sizes in `jc2tool list`).

```powershell
python research/tools/afsm_probe.py --tool <jc2tool.exe> --game <game folder>            # fixtures
python research/tools/afsm_probe.py --tool ... --game ... --report                       # clip table
python research/tools/afsm_probe.py --tool ... --game ... --dump npc_combat_base --grep GRAPPL
```

## What was already decoded and what this adds

The property container and the evaluator semantics (numbered `Transition0..63` precedence, AND/OR,
polarity, local-before-global, one change per dispatch) are in `animation-next.md` (JC2-008). This task
adds the typed graph reader and the content of the gameplay machines:

* `afsm_probe.decode`: machine -> states (class, ordinal-ordered transition references),
  transitions (action, target, ordered condition references, combine mode), conditions (class and
  raw parameter keys), and the extra `60ddfab2` collection, which is the **global** transition list
  (its entries are references resolved against the same transition table; see below).
* `afsm_probe.asb_state_clips`: joins a state to its clip via the `.asb` groups below.

Graph sizes (states / transitions / conditions / global refs), all references resolve (0 dangling):

| Machine | S | T | C | G |
|---|---:|---:|---:|---:|
| parachute | 5 | 6 | 1 | 0 |
| basestatemachine (Rico, on foot) | 272 | 388 | 85 | 4 |
| upperbodystatemachine | 28 | 86 | 36 | 2 |
| leftarmstatemachine | 28 | 50 | 18 | 0 |
| additivestatemachine | 1 | 0 | 0 | 0 |
| npc_combat_base | 186 | 268 | 90 | 4 |
| npc_combat_upper_body | 18 | 55 | 30 | 2 |
| npc_combat_left_arm | 24 | 37 | 12 | 0 |

Key spellings confirmed by hashing (Jenkins `hashlittle`): `StateMachine`, `Name`, `InitialState`,
`States`, `Transitions`, `Conditions`, `Class`, `Transition0..63`, `Condition0..15`, `AND`, `OR`.
Still **unconfirmed spellings** (role from content and from native loader `004abcc0`): `186fe1a8` action,
`45f9d071` target state, `894029cf` combine key, `664c2c54` polarity child, `60ddfab2` global
collection, `cc74a009` version (20061025 in every machine).

### ASB state-to-clip join (`kc001_base.asb`, Rico's set)

Group `be52aa3d` (301 links): each entry has `0703fc2d` = animation id string and `bd...` key
`3175636446` (0xBD43A25E; spelling unknown) = state name. Group `28fa9c38` (490 descriptors) is keyed by
the same `0703fc2d` id and holds the `.ban` list (child `c516d565`), event track `4d026d5b`,
`57ab7197` (0.2 or 0.1 on these rows; suggestive of blend time, unproven) and `playback_speed`
`b7a9d0b3`. The probe's report is the clip table for every REEL/PULL/GRAPPL/STUNT/HOOK state.

## Finding 1: pull versus reel is not decided by the graph

Rico's machine has two separate action IDs that enter two different states from the same
airborne/idle states:

* `ACT_REEL` -> `S_REEL_FLIGHT` (and `S_REEL_START` from stand-still strafe states).
* `ACT_GRAPPLE_PULL` -> `S_PULL_IN_AIR_FLIGHT` (`CVelocityTimeVariableState`, only exit `ACT_FALL`
  -> `S_FALL`). Offered by `S_FALL`, `S_JUMP_FLIGHT_STANDING`, `S_JUMP_TO_SKYDIVE`, `S_SKYDIVE`,
  `S_SWIM_DIVE_IDLE`, `S_BLEND_TO_SWIM_IDLE`, `S_REEL_FLIGHT`, `S_PARACHUTE_DETACH_TO_FREEFALL`.
  None of these edges has a condition: the choice is entirely which action the caller dispatches.

So the graph cannot answer "pulled or reeled": it only defines what each action does. The caller is
native code: `CPullGrappledAction` (vftable `00ec301c`, start slot 6 = `007ae2e0`) and `CReelInAction`
(vftable `00ec7834`, slot 6 = `008098c0`). Evidence: the action-name hash globals are created in the
static initialiser block near `00c81e01`: `ACT_REEL` -> `011969d8` (consumed at `0080991c`),
`ACT_GRAPPLE_PULL` -> `01196808` (`007ae35e`, `007ae3aa`), `LA_ACT_REEL` -> `01196c7c` (`00809bde`),
`LA_ACT_GRAPPLE_PULL` -> `01196774` (`007ae372`), `ACT_REELED_FIRE_HOOK` -> `01196f64`
(`0061d9a8`, `0061e0c7`), `ACT_FORCE_RELEASE_GRAPPLINGHOOK` -> `01196b58` (`006b3524`),
`ACT_GRAPPLE_SMASH` -> `01196780` (`00617b36`), `LA_ACT_GRAPPLE_SMASH` -> `01196b48` (`00617b20`).
Character layers sit in an array at character `+0x48c` (stride 8, count `+0x46c`; layer 0 main body,
layer 2 left arm per the `LA_` prefix, 5 = all layers). `0073f7f0(id, layer)` dispatches an action through
the layer object's virtual `+0`; `00810360(id, layer)` calls virtual `+4` and returns a Boolean (a
query; its exact meaning is unresolved). `CPullGrappledAction::start` (`007ae2e0`): if the query for
`ACT_GRAPPLE_PULL` on layer 0 is false it posts `LA_ACT_GRAPPLE_PULL` to layer 2, sets action state `+0x34 = 0`
and returns; otherwise it dispatches `ACT_GRAPPLE_PULL` on layer 0, a second left-arm action (id global
`01197104`, name unrecovered) on layer 2, sets character field `+0xa74 = 9` and action state `+0x34 = 1`.
`CReelInAction::start` (`008098c0`) requires the layer's current state to equal global `011967c4`
or the `ACT_REEL` dispatch to succeed, then posts `LA_ACT_REEL` to layer 2. **Which action class the
grapple device selects for a given hooked target** is not in the graphs; see Gap 1.

Data corollary: in Rico's set the pull-flight state plays the **same clip** as the reel flight:
`S_PULL_IN_AIR_FLIGHT`, `S_REEL_FLIGHT` and `S_REEL_TO_SO` all bind `ANIM_REEL_FLIGHT` /
`ANIM_PULL_IN_AIR_FLIGHT` to `grpl_reel_flight_part2.ban` (probe check 9/10). The velocity is supplied
by the state class (`CVelocityTimeVariableState`), not by a different clip.

## Finding 2: the grappled victim lives only in the NPC machines

`S_GRAPPLED` and `T_HIT_BY_GRAPPLINGHOOK` exist only in `npc_combat_base.afsm` (the base machine
has no `S_GRAPPLED`; probe check 7). Decoded edges:

* Entry: `T_HIT_BY_GRAPPLINGHOOK` (ACT_UPDATE, condition `C_GRAPPLED_ATLEAST_ONCE`) is offered by NPC
  states such as `S_AI_BASIC_NAVIGATION` (ordinal 24, after the stumble transitions and before the
  mount/pickup ones: precedence is ordinal, so a satisfied earlier stumble edge wins).
* `S_GRAPPLED` (`CAnimationState`): ordinal 0 `ACT_UPDATE` + `C_NOT_GRAPPLED` -> `S_REMOVE_GRAPPLINGHOOK`;
  ordinals 1 and 2 `ACT_ROTATE_CW/CCW` + `C_GRAPPLED_ATLEAST_ONCE` -> `S_UPRIGHT_GRAPPLED_ROTATE_CW/CCW`;
  ordinal 3 `ACT_FORCE_RELEASE_GRAPPLINGHOOK` -> `S_UPRIGHT_IDLE` (no condition).
* `S_REMOVE_GRAPPLINGHOOK`: `ACT_UPDATE` + `C_WAIT_FOR_UPPERBODY_TO_FINISH` -> `S_UPRIGHT_IDLE`, or force release.
* Global list (checked after the local list): `ACT_DIE` + `C_GRAPPLED_ATLEAST_ONCE` -> `S_DEAD`;
  `ACT_DIE` + `C_NOT_GRAPPLED` -> `S_DYING_AIRBOUND`; `ACT_REACT_HIT_FLY` -> `S_HITREACT_UNCONTROLLED_FLIGHT`;
  `ACT_REACT_HIT_FALL` -> `S_FALL`. (Rico's base machine global list is die, hit-fly, hit-fall to idle if
  on the ground, hit-fall to fall.) A dying NPC that is grappled goes straight to `S_DEAD`: no
  airborne death animation while held.
* Upper-body NPC machine mirrors it: `UB_S_GRAPPLED` -> `UB_S_REMOVE_GRAPPLINGHOOK` on `C_NOT_GRAPPLED`;
  `UB_T_ABORT_GRAPPLED_IF_AIRBORNE` (`C_NOT_GROUND_CONTACT`) -> `UB_S_WIELD_WAIT_MB`.
* Conditions: `C_GRAPPLED_ATLEAST_ONCE` and `C_NOT_GRAPPLED` are both class `CGrappledCondition`
  (RTTI `00eb7448`) with raw parameters `689d08f9` = 1 / 0 and `d7f292c6` = 42 / 0 (roles unresolved:
  plausibly a count and a threshold). Reel landing uses `CReelLandCondition` (`00eb72f8`) and the surface
  tests `CGrappledSurfaceNormal` (`00eb82e0`) with `C_RULLEBUSS_CONDITION` params `867709d9`=0,
  `fad9419e`=60 and `C_IMPACT_CONDITION` 60 and 180 (inferred: normal-angle bands, degrees; unproven).

NPC reel flight (`npc_combat_base`): `S_REEL_FLIGHT` -> `S_REEL_IMPACT` (wait finish AND impact
condition) or `S_REEL_END_LAND` (reel-land condition, or wait finish AND rulle-buss condition);
`S_REEL_IMPACT` -> `S_REELED_IN_IDLE` -> (`ACT_REELED_IN_JUMP`) `S_REELED_IN_JUMP` -> `S_FALL`, or
(`ACT_REELED_IN_RELEASE`) `S_REELED_IN_IDLE_END` -> fall / spin-jump / skydive by finish plus raycast
conditions `C_RELEASE_UPSIDEDOWN_CONDITION`, `C_REELED_IN_TO_UPRIGHT` (`CRayCastFanCondition`).

## Finding 3: Rico's reel and stunt chain (base machine)

Reel: `S_REEL_START` (`CJumpControllerState`, `reel_in_start.ban`) --ACT_UPDATE+finish--> `S_REEL_FLIGHT`
(`CVelocityTimeVariableState`) whose exits are, in ordinal order: `ACT_REEL_IMPACT_VERTICAL` ->
`S_REEL_IMPACT_VERTICAL` (`grpl_reel_impact.ban`), `ACT_REEL_IMPACT_HORIZONTAL` -> `S_REEL_IMPACT_HORIZONTAL`
(`reel_impact_horizontal.ban`) -> `S_REELED_IN_TO_HANG` (finish), `ACT_REEL_LAND` -> `S_REEL_END_LAND`
(`grpl_reel_hori_land.ban`), `ACT_DETACH` -> `S_FALL`, `ACT_REEL_TO_SO` -> `S_REEL_TO_SO`,
`ACT_PULLCHORD` -> `S_UNFOLD_PARACHUTE_HORIZONTAL`, `ACT_SUPER_STUNT_JUMP` -> `S_SUPER_STUNT_JUMP_FLIGHT`,
`ACT_LADDER_GRIP` -> ladder, `ACT_GRAPPLE_PULL` -> `S_PULL_IN_AIR_FLIGHT`. Every reel exit is a different
action, so *which* impact/land edge is taken is decided by the native dispatcher (reel-end contact
classification), not by graph conditions (the edges have none). Fire: `S_REELED_FIRE_HOOK` (`ACT_REELED_FIRE_HOOK`,
`grpl_fire_fwd_jump.ban`) -> `S_REELED_WAIT_FOR_HOOK` (`grpl_fire_fwd_loop.ban`) on finish; `ACT_RELEASE_TO_FALL` ->
`S_REELED_RELEASE_TO_FALL`. Melee: `ACT_GRAPPLE_SMASH` -> `S_GRAPPLE_SMASH_LEFT/RIGHT` (`melee_strike_left/right.ban`)
gated by `C_LEFT_FOOT_DOWN`/`C_RIGHT_FOOT_DOWN` from run states and alternating on finish.

Stunt positions: `S_SUPER_STUNT_JUMP_START` (`run_jump_start.ban`, `CJumpControllerState`) -> `S_SUPER_STUNT_JUMP_FLIGHT`
(`stunt_jump_timeblend.ban`, `CVelocityTimeVariableState`) -> `ACT_SUPER_STUNT_LAND` `S_SUPER_STUNT_JUMP_LAND`
(`std_roof_landing_light.ban`) -> `S_SUPER_STUNT_JUMP_LAND_INTERUPT` (finish) -> `ACT_ROOFTOP_IDLE`
`S_DEFAULT_STUNTPOSITION_IDLE` (`std_roof_timeblend_*.ban`) or `ACT_CLING_IDLE` `S_CLING_STUNTPOSITION_IDLE`
(`hng_mid_timeblend_*.ban`) or `ACT_IDLE` -> `S_UPRIGHT_IDLE`; variants `_MC` (`mc_racer_stunt_land`) and `_MG`
(mounted-gun climb). From the stunt idles: `ACT_REEL` -> `S_REEL_FLIGHT`, `ACT_PULLCHORD` -> parachute,
`ACT_SUPER_STUNT_JUMP`, `ACT_DEFAULT_TO_CLING` -> `S_ROOF_TO_CLING`, `ACT_DEFAULT_TO_CLING_ALT` -> `S_CARTWHEEL`,
rotations 180 CW/CCW (`std_roof_nav_jump_180_*`, gated by `C_WAIT_FOR_MAINBODY_BLEND_TO_FINISH`); hanging
(`S_HANGSTUNT_IDLE`, `CHangStuntPositionState`): `ACT_EXIT_STUNTPOSITION` / `ACT_REELED_IN_RELEASE` -> `S_HANG_TO_FALL`,
`ACT_REELED_FIRE_HOOK` -> `S_HANG_FIRE_HOOK` (`heli_default_exit_to_fall.ban`). Rico's idle states with a
stunt-reel start use `S_STUNT_REEL_FLIGHT` (`stunt_reel_flight.ban`), whose exits are the same super-stunt landings.
The probe `--report` prints the full table. `S_STUNT_JUMP_*`, `S_PERFORMING_STUNT`, `S_STUNT_FALL`,
`S_VICTIM_PULLED_OUT` have no kc001 clip link (legacy or other sets).

## Gaps (unresolved native details)

**Update (JC2-168/169):** Gaps 1, 3 and the grapple parts of 2 and 4 are resolved in
`grapple-action-selection-next.md` (selector `0x007f3db0`, availability predicates, layer object
`+0` dispatch / `+4` query, the unnamed left-arm id `01197104` = `LA_ACT_IDLE`, the arrival
dispatchers `0x00564b20` / `0x005e6b90` and the `CGrappledCondition` / `CGrappledSurfaceNormal`
parameter semantics). The text below is kept as the original list; read it with that brief.

1. Which action class and which of `ACT_REEL` / `ACT_GRAPPLE_PULL` the grapple device selects per
   target (Rico reeled vs target pulled; vehicles/objects/NPCs, mass or category). Native callers of
   `CReelInAction` / `CPullGrappledAction` creators (`CContextActionCreator<...>`, vftables `00ec3270`,
   `00ec3300`) and of `CGrapplingActionSet` need tracing. The graph and `grapple_weight` do not decide it.
   Blocks pull/reel parity (JC2-037/038/039/090/093).
2. Layer-object virtual `+0` / `+4` semantics behind `0073f7f0` / `00810360` (dispatch versus "can accept"),
   and the second left-arm action id at global `01197104`. Blocks implementing dispatch/query exactly.
3. Native dispatchers of `ACT_REEL_IMPACT_VERTICAL/HORIZONTAL`, `ACT_REEL_LAND`, `ACT_REEL_TO_SO`, `ACT_DETACH`,
   `ACT_REELED_FIRE_HOOK` (consumers around `0061d9a8`, `006b3524`): how contact classification maps to the
   action. The graph edges carry no conditions here.
4. Condition parameter semantics: `CGrappledCondition` (`689d08f9`, `d7f292c6`), `CGrappledSurfaceNormal`
   (`867709d9`, `fad9419e`), and the evaluators of the 33 base condition classes; `664c2c54`/`894029cf`
   spellings; state class behaviours (`CVelocityTimeVariableState`, `CVelocityControlledState`, `CGrappledController`).
5. ASB field roles beyond clip and event track: `57ab7197`, `c29bffc6`, `fdcf4c12`, `371aba7e`; groups
   `1c2223b1`, `2db28149`, `801bf48e`, `b54a90e8`, `f46f42d4`; `3175636446` spelling. Stunt/jump timeblend clip
   weighting is not derived here.
6. Which NPC archetypes use `npc_combat_*` versus `basestatemachine`, and the upper-body/left-arm
   (`LA_`) machines' own grapple edges beyond `UB_S_GRAPPLED`. Not dumped in detail.
