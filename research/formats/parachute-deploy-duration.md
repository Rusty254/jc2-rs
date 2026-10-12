# Parachute: duration of the two deployment states (board caf7b21c, Gap 5 of `parachute-state-velocities.md`)

Research only. Probe: `research/tools/parachute_deploy_duration_probe.py --game <game folder> --tool <jc2tool>` (28 checks, exit 0). Executable fingerprint:
`JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`. Labels: **native** = read from code/data and asserted by the probe;
**inferred** = reasoned from native facts; **unknown** = Gaps.

## 1. Answer

The earlier reading ("both states wait for the end of the same 2.433 s clip, so the glide starts about 4.9 s after the press") is **wrong**. Two native facts shorten it:

1. **Every animation descriptor carries a finish margin (native).** The asb key `0x96270a00` of a state's descriptor is stored at descriptor `+0x4c` by the loader `0x00499d60`
   (float stored at `[ebp-0xd0]` after the push at `0x49a731`, passed to the setter `0x004cc520` at `0x49a83e`). When a state starts, `0x008f0430` reads it with `0x004b7290` and
   `0x008aa620` writes it to every new instance's `+8` through `0x004ba6e0` (the playback speed, key `0xb7a9d0b3` at descriptor `+0x54`, goes to instance `+0x60`). The finish
   predicate `0x004ae580` (behind `CAnimationFinishCondition` `0x00817d10` -> `0x0087a230(0)` -> `0x008ef600`, first instance of body part 0) is: mode (`+0x18`) is 1 and
   (rate >= 0 and time >= clip duration - margin, or rate <= 0 and time <= margin). So a state with a margin leaves **margin seconds before the clip end**, not at the end.
   The earlier docs read this key as nothing (`animation-next.md` calls `0x96270a00` an unproven float). The other four keys of the descriptor (`0x57ab7197` "blend", `0x371aba7e`,
   `0xc29bffc6`, `0xfdcf4c12`) are parsed by the same loader into locals that nothing reads again, and no other code references those hashes, so they have no effect on playback
   (**native**; their meaning stays unknown and irrelevant here).
2. **A state change hands the clip phase over (native).** The body-part update `0x00881640` (state change at `0x8817b2`) asks the transition tables `0x004c5350(old, new)`; when the answer is 0 the
   new instance starts at `new clip duration * (old instance time / old duration)` (`0x008a89e0`, `0x00862350` with `0x004b4300`), when it is nonzero the clip starts at 0. The answer is the value of the
   (old, new) pair in asb group `0xb54a90e8` (map `this+0x58`), else the restart flag (asb group `0x801bf48e`, key `0x6872e596`, map `this+0x64`) of either animation, else 0. Neither
   `ANIM_PULL_OPEN_PARACHUTE_VERTICAL` nor `ANIM_UNFOLD_PARACHUTE_VERTICAL` is in either table (the 21 pairs and 38 flags were read from `kc001_base.asb`; only
   `ANIM_UNFOLD_PARACHUTE_HORIZONTAL` is flagged 1), so the pull-open to unfold change **continues the clip at the phase it had reached**.

## 2. The numbers (`kc001_base.asb`, `rico_open_chute.ban` = 74 frames, 2.433 s, inferred exactly 73/30 = 2.4333 s)

| State | Descriptor | Margin | Speed | Instance start | Leaves at clip time | State lasts |
|---|---|---|---|---|---|---|
| `S_PULL_OPEN_PARACHUTE_VERTICAL` | `ANIM_PULL_OPEN_PARACHUTE_VERTICAL` | 1.97 | 1.0 | 0 | 2.4333 - 1.97 = **0.4633 s** | 0.4633 s (+ up to one tick) |
| `S_UNFOLD_PARACHUTE_VERTICAL` | `ANIM_UNFOLD_PARACHUTE_VERTICAL` | 0.8 | 1.0 | 0.4633 s (phase hand-over, same clip) | 2.4333 - 0.8 = **1.6333 s** | 1.17 s |

**Press to glide (`S_PARACHUTE` starts): 1.633 s** (not 4.87 s). The freefall model flies only the first **0.463 s** (not 2.4 s); the chute model and its opening transform start
at 0.463 s after the press, 1.17 s of the unfold state remain. Tick granularity: the order of clip advance and graph update inside a native tick is not decoded, so each boundary is
good to one tick (probe fixtures at 60 and 30 Hz). If the hand-over did not apply (restart), the glide would start at 2.10 s; the canopy object's own opening clip supports the
hand-over reading: `parachute.asb` `PC_ANIM_OPENING_VERTICAL` (`chute_open_chute.ban`, 2.433 s, started by the same pull) has the same margin 0.8, so a canopy graph that uses the same
rule (its finish condition `CParachuteAnimationFinishCondition` `0x007340c0` is still not decoded) is fully open at the same 1.633 s at which the character reaches `S_PARACHUTE`.
(inferred corroboration, not proof.)

Other states with a margin on the same path (data, same rule): `S_UNFOLD_PARACHUTE_HORIZONTAL` margin 1.2 on the 2.0 s `rico_reel_open`, so it leaves at 0.8 s (its pair is
flagged restart 1, start at 0); `S_PARACHUTE_DETACH_TO_FREEFALL` margin 0.4 on `detatch_chute` (1.4 s), leaves at 1.0 s; `S_FALL_TO_SKYDIVE` margin 0, leaves at the clip end.

## 3. Consequences for the runtime (task 4d7f7642)

* `S_PULL_OPEN_PARACHUTE_VERTICAL` lasts 0.4633 s of state time, `S_UNFOLD_PARACHUTE_VERTICAL` ends when the shared clip reaches 1.6333 s, i.e. the unfold state plays the **second part**
  of `rico_open_chute` (clip time 0.4633 to 1.6333): use the clip time, not a fresh clip, when the unfold state starts.
* Run the freefall model for 0.463 s after the press; run `CanopyFlight::open` + `chute_open_velocity` at that moment; glide pose from 1.633 s.
* `S_PARACHUTE_DETACH_TO_FREEFALL` ends at 1.0 s, not 1.4 s.
* A generic rule for the animation layer: a state finishes at `duration - margin` of its first instance; a state change starts the new clip at the old phase unless the pair or an
  animation is flagged restart (this also affects other states; the rule is native for every state).

## 4. Gaps

1. **Instance mode source.** The finish predicate requires mode 1. The mode written by `0x008aa620` comes from descriptor `+0x40` (`0x004a3850`, `0x00495a80`: types 1, 3, 4, 6 are
   looping candidates, type 0 gives speed 0); the loader copies `+0x40` from the clip-set record, not from the asb integers `0xc29bffc6`/`0xfdcf4c12` (unused). Not decoded for
   `rico_open_chute`; inferred mode 1 because the original game does leave both states. Blocks nothing visible.
2. **Tick order.** Where the clip advance (`0x008bce40`/instance update) sits relative to the graph update inside one native frame (decides whether each boundary is early or late by one tick). At most +-17 ms per boundary.
3. **Canopy object finish.** `CParachuteAnimationFinishCondition` (`0x00eb73e8`, evaluator `0x007340c0`) and whether the canopy graph also leaves `PCS_OPENING_VERTICAL` at 1.633 s
   (this decides when the canopy counts as open; the pose clip drives the inflation, so the visible inflation timing follows the clip regardless).
4. **Reference shots** (`canopy-deploying-*`, `deploy-unfolding`, `canopy-just-opened-*`) are single stills with no timestamps and cannot confirm the duration.
