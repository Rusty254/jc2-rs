# Rico's grapple animations in the runtime (JC2-250)

Implements `grapple-action-selection-next.md` (JC2-168/169) and `animation-statemachines-next.md`
(JC2-106) in `crates/jc2-game/src/grapple/anim.rs`. Retail executable SHA-256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Clip names, blend times and
playback speeds come from `research/tools/afsm_probe.py` (`--report` for `kc001_base.asb`; the
same join for `kc001_leftarm.asb` and `kc001_upperbody.asb`, run through `asb_state_clips`).

## What the audit found (before JC2-250)

| Area | Before | Native (now) |
|---|---|---|
| Reel / pull choice | `class.rs` guess: soldier pulled, vehicle anchor, authored 1/2, else a 400 kg mass threshold | `0x007f3db0`: terrain or no object -> Reel; character or category 1 -> Pull; category 2 -> Reel; other categories -> neither; second hook -> both off. Mass is only the labelled fallback for bodies with no authored category |
| Reel on the ground | the flight clip at once | `ACT_REEL` -> `S_REEL_START` (`reel_in_start.ban`, blend 0.1) -> `S_REEL_FLIGHT` |
| Reel in the air / swimming / stunt | flight clip | `ACT_REEL` -> `S_REEL_FLIGHT` (`grpl_reel_flight_part2.ban`, blend 0.2) |
| Arrival classes | normal.y > 0.7 floor, < -0.7 ceiling | `>= cos 50` floor, `<= cos 140` ceiling (`0x00549490`, keys `#00992cfb`, `#8e43b31e`) |
| Floor | nothing: plain fall/land clip | `ACT_REEL_LAND` -> `S_REEL_END_LAND` (`grpl_reel_hori_land.ban`, blend 0.4) |
| Wall | `grpl_hang.ban` | `ACT_REEL_IMPACT_VERTICAL` -> `S_REEL_IMPACT_VERTICAL` (`grpl_reel_impact.ban`, 0.07) -> `S_REELED_IN_IDLE` (`grpl_reeled_timeblend.ban`) |
| Ceiling | `grpl_hang.ban` | `ACT_REEL_IMPACT_HORIZONTAL` -> `S_REEL_IMPACT_HORIZONTAL` (`reel_impact_horizontal.ban`, 0.07) -> `S_REELED_IN_TO_HANG` (`reel_impact_to_hang.ban`, speed 0.9) -> `S_HANGSTUNT_IDLE` (`heli_default_timeblend_hori/diag.ban`, 0.2) |
| Space on a wall | bounce clip only on a ledge | `ACT_REELED_IN_JUMP` -> `S_REELED_IN_JUMP` (`grpl_reel_bounce.ban`) -> fall |
| Ctrl | no clip | wall: `S_REELED_IN_RELEASE_DROP` (`grpl_reeled_release.ban`); ceiling: `S_HANG_TO_FALL` (`heli_default_exit_to_fall.ban`) |
| Pull on the ground | the aim clip | left arm only: `LA_ACT_GRAPPLE_PULL` -> `LA_S_GRAPPLE_PULL` (`grpl_idle_to_pull.ban`, 0.14), body untouched |
| Pull in the air | none (Rico stood still) | body `ACT_GRAPPLE_PULL` -> `S_PULL_IN_AIR_FLIGHT` (same clip as the reel flight), the wire 6 m shorter, then `ACT_FALL` and the hook is cancelled |
| Left arm | one aim loop | `LA_S_RAISE_GRAPPLE` (`grpl_fire_ani_con`, 0.2) -> `LA_S_WAITING_FOR_HOOK` (`grpl_fire_aim_con`) -> `LA_S_HOOK_HIT` -> `LA_S_GRAPPLE`; miss `LA_S_HOOK_MISSED`; cancel `LA_S_GRAPPLE_DETACH` / `LA_S_GRAPPLE_TO_IDLE` (`la_grpl_idle_recoil`, 0.2); second hook `LA_S_DUAL_GRAPPLE` (`grpl_fire_dualhook_con`, 0.07) |
| Hooked soldier | stumble clip while pulled or leaning | `char+0x9e8` counter: `C_GRAPPLED_ATLEAST_ONCE` (1..42) -> `S_GRAPPLED` (`stumble_grpl`), counter back to 0 -> `S_REMOVE_GRAPPLINGHOOK` (`stumble_grpl_release`, blend 0.1) |

`grpl_hang.ban` is not a Rico state at all: it is `S_HANGED` of `npc_combat_base.afsm` (an airborne
hooked ragdoll). Rico's hangs are the states above. `Clip::Hang` stays only as a fallback and for
the stunt hang.

## Design

`anim.rs` is plain data (no ECS): `ActionSet` (the six flags of `CGrapplingActionSet`), the
selector, `classify_arrival`, `BodyState` (layer 0) and `ArmState` (layer 2) with their edges
(`body_accepts` / `arm_accepts`, the layer object's virtual `+4` query), a `GrappleAnim` that owns
both and records a trace, and the NPC `GrappledCount` / `VictimState`. `grapple.rs` posts the same
actions the original posts (hook flight end: selector, `LA_ACT_HOOK_HIT`, then `ACT_REEL` or the pull;
arrival: the dispatchers; `release`: `ACT_DETACH`); `anim_tick` advances the clocks (a clip that
has played out takes its `ACT_UPDATE` exit); `character.rs` plays the clip of the state the graph
is in (`pick_base`, `hero_animation`), and `npc.rs` the victim's clip.

## Headless traces (retail clip lengths)

Produced by `cargo test ... grapple -- --nocapture`; the unit traces are in
`grapple/anim/tests.rs`, the end-to-end ones (physics + graph + animator) in `grapple/tests.rs`
(`trace_*`). Layer 0 = body, layer 2 = left arm; `[clip]` is the state's `.ban`.

Reel from 20 m up to the ground (air: straight into the flight; floor: land):

```
 0.52s L2 LA_ACT_FIRE_GRAPPLE          LA_S_IDLE -> LA_S_RAISE_GRAPPLE  [grpl_fire_ani_con.ban]
 0.77s L2 LA_ACT_HOOK_HIT              LA_S_RAISE_GRAPPLE -> LA_S_HOOK_HIT  [grpl_fire_aim_con.ban]
 0.77s L0 ACT_REEL                     (free) -> S_REEL_FLIGHT  [grpl_reel_flight_part2.ban]
 0.77s L2 LA_ACT_REEL                  LA_S_HOOK_HIT -> LA_S_GRAPPLE  [grpl_fire_aim_con.ban]
 2.20s L0 ACT_REEL_LAND                S_REEL_FLIGHT -> S_REEL_END_LAND  [grpl_reel_hori_land.ban]
 2.20s L2 LA_ACT_DETACH                LA_S_GRAPPLE -> LA_S_GRAPPLE_TO_IDLE  [la_grpl_idle_recoil.ban]
 2.78s L2 LA_ACT_UPDATE (clip finished) LA_S_GRAPPLE_TO_IDLE -> LA_S_IDLE
 2.94s L0 ACT_UPDATE (clip finished)   S_REEL_END_LAND -> (free)
```

Reel from the ground to a wall, then Space (ground: `S_REEL_START` first):

```
 0.80s L0 ACT_REEL                     (free) -> S_REEL_START  [reel_in_start.ban]
 1.20s L0 ACT_UPDATE (clip finished)   S_REEL_START -> S_REEL_FLIGHT  [grpl_reel_flight_part2.ban]
 2.31s L0 ACT_REEL_IMPACT_VERTICAL     S_REEL_FLIGHT -> S_REEL_IMPACT_VERTICAL  [grpl_reel_impact.ban]
 2.75s L0 ACT_UPDATE (clip finished)   S_REEL_IMPACT_VERTICAL -> S_REELED_IN_IDLE  [grpl_reeled_timeblend.ban]
 3.34s L0 ACT_REELED_IN_JUMP           S_REELED_IN_IDLE -> S_REELED_IN_JUMP  [grpl_reel_bounce.ban]
 3.92s L0 ACT_UPDATE (clip finished)   S_REELED_IN_JUMP -> (free)
```

Reel to a ceiling, hang, Ctrl:

```
 2.09s L0 ACT_REEL_IMPACT_HORIZONTAL   S_REEL_FLIGHT -> S_REEL_IMPACT_HORIZONTAL  [reel_impact_horizontal.ban]
 2.27s L0 ACT_UPDATE (clip finished)   S_REEL_IMPACT_HORIZONTAL -> S_REELED_IN_TO_HANG  [reel_impact_to_hang.ban]
 2.86s L0 ACT_UPDATE (clip finished)   S_REELED_IN_TO_HANG -> S_HANGSTUNT_IDLE  [heli_default_timeblend_hori.ban]
 4.12s L0 ACT_REELED_IN_RELEASE        S_HANGSTUNT_IDLE -> S_HANG_TO_FALL  [heli_default_exit_to_fall.ban]
 4.47s L0 ACT_UPDATE (clip finished)   S_HANG_TO_FALL -> (free)
```

Pull a barrel on the ground (the body is not touched; animator base stays on the locomotion clip):

```
 0.95s L2 LA_ACT_FIRE_GRAPPLE          LA_S_IDLE -> LA_S_RAISE_GRAPPLE  [grpl_fire_ani_con.ban]
 1.11s L2 LA_ACT_HOOK_HIT              LA_S_RAISE_GRAPPLE -> LA_S_HOOK_HIT  [grpl_fire_aim_con.ban]
 1.11s L2 LA_ACT_GRAPPLE_PULL          LA_S_HOOK_HIT -> LA_S_GRAPPLE_PULL  [grpl_idle_to_pull.ban]
 2.61s L2 LA_ACT_UPDATE (clip finished) LA_S_GRAPPLE_PULL -> LA_S_IDLE
```

Pull in the air (Rico flies at the barrel, the arm keeps its hit pose until the cancel):

```
 1.30s L2 LA_ACT_HOOK_HIT              LA_S_RAISE_GRAPPLE -> LA_S_HOOK_HIT
 1.30s L0 ACT_GRAPPLE_PULL             (free) -> S_PULL_IN_AIR_FLIGHT  [grpl_reel_flight_part2.ban]
 1.30s L2 LA_ACT_IDLE                  rejected in LA_S_HOOK_HIT
 1.64s L0 ACT_FALL                     S_PULL_IN_AIR_FLIGHT -> (free)
 1.64s L2 LA_ACT_DETACH                LA_S_GRAPPLE -> LA_S_GRAPPLE_TO_IDLE
```

A miss: `LA_S_RAISE_GRAPPLE -> LA_S_WAITING_FOR_HOOK -> LA_S_HOOK_MISSED -> LA_S_IDLE`, body never
touched. A soldier: `S_GRAPPLED` (counter 1) -> `S_REMOVE_GRAPPLINGHOOK` (counter 0) ->
`S_AI_BASIC_NAVIGATION`. The second hook: `LA_S_DUAL_GRAPPLE`, Reel and Pull off.

Not captured: an in-game screen capture (the run is headless); the animator sequences above are
what the renderer plays.

## Native versus inferred

Native-proven (addresses in the two briefs): the selector table, the arrival cosines, the state
graph edges, every state-to-clip binding and its blend and speed, the layer dispatch order
(`LA_ACT_HOOK_HIT`, then `ACT_REEL`/`LA_ACT_REEL` or the pull), the 6 m air-pull shortening (the
constructor default), the `CGrappledCondition` bands.

Inferred or non-native, labelled in the code (`Gap` in comments):

* Vehicles count as heavy (category 2) and a body with no authored category keeps the old mass
  threshold (`class.rs`): the effective categories are open (JC2-245).
* Time-variable and time-blend states hold the clip's middle (`S_REEL_FLIGHT`, `S_PULL_IN_AIR_FLIGHT`,
  `S_REELED_IN_IDLE`, `S_HANGSTUNT_IDLE`): the parameter that picks the pose is not decoded;
  `S_HANGSTUNT_IDLE` blends two clips (`hori`, `diag`), only the first is played.
* `S_REEL_IMPACT_VERTICAL_INTERRUPT` (`CSetStartTimeState`) is collapsed into the following state.
* `S_REEL_END_LAND`, `S_REELED_IN_RELEASE_DROP`, `S_HANG_TO_FALL` have no listed exit: the end of the
  clip hands back to locomotion, and running above 2 m/s (or touching down after the jump/release)
  ends them earlier.
* A jump or release pressed while an impact clip still plays cuts the impact short.
* The air pull moves Rico with the reel's speed ramp (the native tick `0x00515cb0` is undecoded,
  JC2-246); the arrival happens when the Rust reel has braked to rest, not on first contact.
* The `S_REEL_START` blend input (`reel_start_blend`, tested) is not consumed: `reel_in_start` is a
  single clip.
* `LA_ACT_IDLE` at the air-pull start is refused by `LA_S_HOOK_HIT` (the graph has the edge only on
  `LA_S_GRAPPLE`); whether the original's hit state has already finished by then is unknown.
* `grpl_idle_to_pull.ban` has 76 tracks (a whole-body take): it is masked to the left arm as every
  layer-2 clip is.
* The victim counter is decremented when the pull ends (the caller of `0x0069b380` is not located),
  and `S_REMOVE_GRAPPLINGHOOK` is taken to end when the soldier stops lying.

## Gaps (unresolved native details)

1. Effective `grappling_weight_category` of characters, vehicles and their parts, and what a category
   0 / 3+ hit does (JC2-245).
2. Reel flight tick `0x00515cb0`, contact sweep, speed ramp and stop distance (JC2-246; decoded in
   `grapple-reel-flight-next.md`: ramp 3.5/s from 11 m/s (reel) or 6 m/s (pull) to 40, stop distance 1.4).
3. Who moves a hooked soldier or prop to Rico; whether NPCs enter `S_REEL_FLIGHT` (JC2-247).
4. `CFireHookAction` sub-states, second shot from a hang (`S_HANG_FIRE_HOOK`, `S_REELED_FIRE_HOOK`,
   `S_REELED_WAIT_FOR_HOOK` are in the graph but not dispatched here) (JC2-248).
5. `CReeledInJumpAction`, `CReeledInReleaseAction`, `CDeployParachuteWhileReelingAction` bodies and
   `ACT_REELED_IN_JUMP_RELEASE` / `S_REELED_IN_RELEASE` (JC2-249).
6. Pose parameters and exits of the animation state classes the grapple uses:
   `CVelocityTimeVariableState` (what sets clip time in the reel flight, the pull flight, the reeled-in
   idle), `CHangStuntPositionState` (the `hori`/`diag` blend), `CJumpControllerState` (the
   `S_REEL_START` blend input and its jump event), `CSetStartTimeState`, the exits of
   `S_REEL_END_LAND`, `S_REELED_IN_RELEASE_DROP`, `S_HANG_TO_FALL`, and the event tracks
   (`reel_ground_impact`, `reel_wall_impact`, `reel_roof_impact`, `grpl_pull`, `grpl_fire`) (JC2-252).
7. The order of the arm's `LA_S_HOOK_HIT` finish against the pull/reel start within one frame, the
   caller of the victim counter decrement `0x0069b380`, and the vehicle monitor `0x005d2c10` (reel land
   versus detach on a hooked vehicle) (JC2-253).
