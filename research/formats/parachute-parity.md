# Parachute parity audit (board task 909d394d, "parachute-audit")

Audit only, no code changes. Compares the jc2-rs parachute runtime (`crates/jc2-game/src/parachute.rs`,
`parachute_flight.rs`, `character.rs`, `foot_machine.rs`, `gameplay_camera.rs`, `hud.rs`, `audio.rs`) with merged native research
(`parachute-canopy-landing-next.md` JC2-243/244/262/263, `parachute-thruster-item.md` JC2-260, `foot-conditions-next.md` JC2-259,
`rico-jump-air-context.md` JC2-270 and the grapple docs) and the 85 reference shots in `reference/chute/` (original game, 2561x1429).
"native" = asserted by a merged probe; "inferred"/"unknown" are said so. Board ids are given as the first 8 characters.

Note on numbering: board "JC2-270" exists twice. The implementation task (jump and air velocity model, 1354a3d8) is done; the research
gap carrying the title "JC2-270: body orientation in freefall and under the canopy" is 9cfa8abf. The same applies to JC2-269
(done implementation d1d595ff; open research gap 67a52bd7). Tags and priority were applied to the open research gaps.

## 1. What already conforms (do not redo)

* Canopy force law, spring, matrix order `Rz*Rx*Ry`, fixed points 9.60/1.69, 7.24/9.52, 45.1/33.0, 40 m/s hook clamp: `parachute_flight.rs` (JC2-263).
* Skydive `Ca`/`Cb`, area, cap, orientation step, only while the foot machine is in `S_SKYDIVE` (`character.rs`, `Skydive`).
* Landing classified by the graph on body velocity (8.1/8.0/-8.0/-8.1), `ACT_LAND` every grounded frame; cut-chute `ACT_PULLCHORD` with the 10-ray fan,
  30/60 m (`character.rs`, `parachute_flight::forward_fan`); canopy shown while the foot machine is in a canopy state (matches close rule (a)).
* HUD prompts "Open parachute SPACE" / "Close parachute SPACE" exist (`hud.rs`); audio events `chute_open`, `chute_loop`, `wind` are wired (`audio.rs`).
* Thruster: correctly absent (retail canopy has no thruster group, JC2-260).

## 2. Numbered gap list

Format: native evidence, runtime location, files touched, covering task. "I" tasks are implementation (known native), "R" research (unknown).
New tasks (all tags `parachute`, priority high, status ready): R1 15c8aede, R2 28d8e587, R3 60caf984, R4 621ea450, I1 22e54146, I2 d5c39f26,
I3 950d9681, I4 1a28a5c5, I5 20ded2f1, I6 8d8de145, I7 24e98319.

| # | Phase | Gap | Native evidence | Runtime location / files | Task |
|---|---|---|---|---|---|
| G1 | canopy | RESOLVED (97bf7030): the runtime renders the retail skinned model `general.blz/gae05_lod1-gae05.rbm` with `gea05_*` textures (`research/formats/parachute-canopy-model.md`). Open: shader state of the block, `mpm` channels, red trim/night look (R2 28d8e587), cloth (unproven) | native model, inferred shading | `parachute.rs` `attach_canopy_model`; `character.rs` fallback dome only when `parachute.bsk` is missing | R1 15c8aede; 97bf7030 |
| G2 | canopy | RESOLVED (97bf7030): the 16 line tubes, harness straps and brake handles are part of the same skinned mesh and ride the `parachute.bsk` joints | native | `parachute.rs` | R1; 97bf7030 |
| G3 | canopy | Shading, shadow, night look: material family, pass, shadow casting, normal/spec maps unknown. Reference: dark grey-black fabric with cell shading by day, only ambient/spot lit at night, lines stay near-black | **resolved by R2** (`parachute-canopy-shading.md`): opaque double-sided `SkinnedGeneral`, `MPM` R/G/B = light scale/specular/exponent, rim term; open: shadow-caster registration, instance constants | `parachute.rs` StandardMaterial (roughness 0.9, double sided) | R2 28d8e587 done; implementation task filed |
| G4 | deploy | Deploy visuals: pilot-chute bag, crumpled canopy inflating (shots `pilot-chute-*`, `canopy-deploying-*`, `deploy-unfolding`); runtime plays only `chute_open_chute.ban` (2.4 s, 0 to 3.94 m span) | clip known, bag object/trigger/timing unknown | `parachute.rs` `animate_canopy` | R3 60caf984 |
| G5 | collapse | Collapse, cut-away and drape (`canopy-closing`, `canopy-collapsing-*`, `canopy-draped-over-*`, `freefall-after-cutting-away-*`): runtime hides the canopy at once | close triggers native (`0x005ab65e`, `0x006ccd40`, section 3 of the landing doc); visual unknown (clip, cloth or physics), `detatch_chute` clip known | `character.rs` canopy visibility (`canopy_up`), `parachute.rs` | R3 |
| G6 | flight/animation | Pose blend: native `D+4`, `D+8`, flare offset -0.825, `S+0x3c/0x40`, five-clip weights/times; runtime feeds the stick straight to the speed/turn axes and the canopy plays only the opening clip. Slot rates, additive idle, layer weight unknown | native (section 10.2 to 10.4); remaining unknown: 57fed721, writer order 6f204aa8 | `character.rs` chute axis block, `parachute_flight.rs`, `parachute.rs` | I1 22e54146 + research 57fed721, 6f204aa8 |
| G7 | flight/steering | Camera-steered turn `S+4`, target `0.46*S+4` for `|S+4| > 0.9`, setting 0x26 pitch invert; runtime steers by A/D only | native (section 10.5/10.6); default of option 0x26 and the 0x114e events unknown (6f204aa8) | `character.rs` `CanopyInput`, `parachute_flight.rs` | I2 d5c39f26 + 6f204aa8 |
| G8 | camera | Camera ids (SkyDive = 2, Parachute = 1) vs the heuristic `is_skydiving` and the invented `CAMERA_DISTANCE_CHUTE = 7.5`; transition timing consumer unknown | DONE (1a28a5c5): camera now follows the foot-machine state (`apply_foot_state`); distance IS in the camera tables after all: camera offset (0, 0, 6.75) Parachute / (0, 3, 2.5) SkyDive plus pivot, limits and smoothing, see `parachute-camera-placement.md` (3eac0c46); consumer of `controller+4` still unknown | `gameplay_camera.rs`, `character.rs` camera distance | I4 1a28a5c5 + ce7795e7 |
| G9 | hook | Exclusion implemented (`HeroInput::fire_grapple`, `CanopyInput::fire_grapple`, JC2 20ded2f1); how the device pull and assist combine unknown | exclusion native (`0x005aa4d4-0x005aa504`); combination unknown | `character.rs` HookAssist, `grapple.rs` reel on top | I5 20ded2f1 + 67a52bd7 |
| G10 | canopy orientation | Canopy only yawed; native basis `Rz(a38)*Rx(a30)*Ry(a34)` already computed by `CanopyFlight::matrix()` but not applied; constant invented 0.35 rad tilt on Rico | basis native; pivot RESOLVED (bce93556, `parachute-attach-pivot.md`): canopy matrix = character matrix `+0x15c`, pivot = character origin at the feet, plus a three-bone attach (handles to hands, `HarnessExt` to Spine1) the runtime lacks | `character.rs` model-update block (`canopy_up`, `tilt`) | I6 8d8de145; attach task from bce93556 |
| G11 | freefall entry | Velocities written by `S_FALL_TO_SKYDIVE`, `S_JUMP_TO_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_PULL_OPEN_PARACHUTE_VERTICAL`, `S_UNFOLD_PARACHUTE_*`; lead-in states keep the project's ballistic fall | RESOLVED (research 9a361f01, `parachute-state-velocities.md`): state classes write no velocity; the controller of the state does; the freefall model flies `S_FALL_TO_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_PULL_OPEN_PARACHUTE_VERTICAL` too; base jump `forward * lerp(v0, 12, progress)`; chute model and opening transform start at `S_UNFOLD_PARACHUTE_*` | native | `character.rs` (`hero.skydive`, `hero.skydiving`, chute open), `parachute_flight.rs` | implementation task (see the doc, section 7) |
| G12 | effects/audio/HUD | `ParachuteEffects.bin` unread; FMOD steer parameter hard-coded 0.0; prompt rule 0.35 s air time is observation; "Reel in E" under canopy | unknown | `audio.rs`, `hud.rs` `FALL_PROMPT_AFTER` | R4 621ea450 |
| G13 | body orientation | Head-down pitch (to about 56 degrees at P = 0) and bank on the visual body in freefall and under the canopy; matrix at opening; runtime keeps only yaw | RESOLVED in research (`parachute-body-orientation.md`): whole-model matrix about the character origin on top of prone/upright clips, Euler init `0x004de960`, freefall model runs in all seven skydive states; only the mesh-matrix instruction stays inferred (b67acd7f) | `character.rs` model update, `Hero::facing`, `CanopyFlight::open` | 9cfa8abf (research done); implementation task filed from it |
| G14 | grapple entries | Deploy from reel (`CDeployParachuteWhileReelingAction` 0x005819e0, flare -1.4), roof/vehicle exits, reeled release states; also tether physics (039) | partly native, bodies unread | `grapple/`, `foot_machine.rs` | 93e4dd1b (JC2-249), 693dfaa0 (JC2-039) |
| G15 | landing | Skydive crash into `S_HITREACT_UNCONTROLLED_FLIGHT` ragdoll (runtime plays a heavy landing) | graph native, ragdoll behaviour unknown | `character.rs` landing | d5df76da (JC2-259) |
| G16 | conditions | Ray-fan collision filter (layer 9, character filter) behind `C_CAN_FALL_TO_SKYDIVE` and the cut fans: which bodies count | unknown | `character.rs` fan casts | e445be43 (JC2-259) |
| G17 | capture | No repeatable way to produce the reference scenes in the runtime | n/a | new module | I3 950d9681 |

Remaining items listed in the landing doc Gaps and not blocking visible parity (kept in that doc): hook-assist side rules (kind-1 object
branch `0x00740260`, device timer, effect index 2), thruster values (no installed pack), tick rate of the stick filter, names of the
`0x114e` events.

## 3. File ownership (so running agents do not collide)

| Task | Owns | Shared with (sequence) |
|---|---|---|
| I1 22e54146 | `parachute_flight.rs`; `parachute.rs` `animate_canopy` blend; `character.rs` chute axis block | merge first |
| I2 d5c39f26 | `parachute_flight.rs`, `gameplay_camera.rs` (export only), `character.rs` `CanopyInput` | after I1 |
| I5 20ded2f1 | `parachute_flight.rs` HookAssist, `character.rs` HookAssist construction | after I2 |
| I6 8d8de145 | `character.rs` canopy rotation block, `parachute.rs` pivot | after I1 |
| I7 24e98319 | `parachute.rs` materials/colours/mesh | after I1 (same file) |
| I4 1a28a5c5 | `gameplay_camera.rs`; two lines of `character.rs` | independent of I1/I2 except I2's export in `gameplay_camera.rs`: merge I2 first or rebase |
| I3 950d9681 | `chute_capture.rs` (new), `main.rs` flag only, `scripts/chute-capture.ps1` | independent |
| R1..R4 | new research docs/probes only | independent |
| existing research tasks | their own docs/probes only | independent |

`character.rs` is the main collision point (I1, I2, I4, I5, I6 each touch a named block): one agent at a time on it, or rebase before merge.

## 4. Reference shots versus runtime (visual findings)

Shots read for this audit: `canopy-deploying-over-jungle-plain-daylight_ck47`, `canopy-behind-lines-base-rooftops-red-antenna-night_z29`,
`canopy-collapsing-low-snow-landing-zone-night_f04`, `canopy-above-rico-below_a10`; the rest are listed by file name in
`reference/INDEX.md` (not re-opened).

* Canopy shape: ram-air with 9 to 10 visible cells, cell ribs, a pointed leading edge and a slightly arched span about 2 to 2.5 times wider than Rico's
  height; cell fabric is dark grey with fine woven/stripe texture. Runtime: smooth arch, flat colour (G1, G3).
* Lines: thin black lines, about 7 per side from the canopy bottom edge converging on two strap points at Rico's shoulders, plus brake lines. Runtime
  lines are white and unlit (G2).
* Deploy (`ck47`): the canopy opening is crumpled, an angular dark mass with pointed folds above the lines while the prompt already reads "Close parachute".
  Runtime: the arch unfolds along bones (G4).
* Night base (`z29`): canopy a flat grey-black silhouette against clouds, lines dark; Rico arms up holding the handles with a pistol in hand.
* From above (`f04`): only the lines and the edge of the canopy cross the top of the frame while Rico is in freefall below ("Open parachute SPACE" still
  shown, canopy above is the previous or pilot canopy region); cold blue night grade on snow. Runtime camera never shows this view (G8).
* Bank (`a10`): the canopy rolls with the bank, the whole rig leans and Rico hangs rotated under the lines, one arm up. Runtime banks nothing visible (G10, G13).
* Prompts: "Close parachute SPACE" and "Reel in E" are shown together under the canopy; runtime shows prompts from `hud.rs` (G12).

## 5. End goal: capture scenarios versus reference shots (task I3)

Capture each scenario in the runtime and compare with its reference group. Scenarios, trigger script and pass checks (word description, not pixel diff):

| Scenario | Reference shots | Setup | Instants | Pass |
|---|---|---|---|---|
| S1 jungle deploy by day | `canopy-deploying-over-jungle-plain-daylight_ck47`, `canopy-open-jungle-plain-road-daylight_ck49`, `deploy-unfolding_a06`, `pilot-chute-*` | jungle plain near a road, midday, clear; jump from about 200 m, freefall 4 s, deploy | deploy +0.0, +0.3, +1.0 s, steady glide | bag/crumple phase, inflation time, canopy size and colour, lines dark, prompt text |
| S2 canopy behind Rico at night over a base | `canopy-behind-lines-base-rooftops-red-antenna-night_z29`, `canopy-behind-over-base-courtyard-building-17_z50`, `canopy-behind-rico-approaching-base-star-emblem-tanks_z09` | military base (Kem Gunung Dataran Tinggi), night, snow, camera behind | steady glide at 60 m | silhouette against sky, cell shading, line visibility, ambient only lighting |
| S3 banking | `canopy-hard-bank-sideways-stripes-snow-gusts_v28`, `canopy-bank-over-*`, `canopy-side-*` | hold A for 2 s then D | peak bank, recover | canopy roll angle (about 0.65 rad at full input), Rico hangs, yaw rate 0.79 rad/s |
| S4 landing collapse | `canopy-collapsing-low-snow-landing-zone-night_f04`, `canopy-closing_b37`, `canopy-landing-*`, `canopy-draped-over-cruiser-landing_bx11` | glide onto flat ground and onto a vehicle roof | -0.5 s, touchdown, +0.5 s, +2 s | landing clip by speed class, collapse look, canopy removal, no pop |
| S5 canopy from above | `canopy-above-rico-below_a10`, `canopy-above-lines-*`, `top-down-through-lines-*`, `lines-fan-from-above-*` | camera above, looking down through the lines | steady | line fan geometry, canopy edge, camera elevation and FOV |
| S6 freefall poses | `freefall-*` | belly down, back, banking, cut-away | steady, after cut | body pose, camera 2, wind effects |

Tooling a runtime capture needs (all in I3 unless noted):

1. A scenario flag that places Rico (position, altitude, heading, velocity), sets time of day and weather, and fixes the RNG so runs repeat.
   Existing hooks: `--screenshot` / `--exit-after` (`main.rs`), the debug "hold Rico in a state" flag, `--material-capture`.
2. A scripted input timeline (deploy at t, steer left/right, cut) injected into the controls resource, independent of real keyboard time.
3. Fixed-step stepping (screenshots at game times, not wall-clock) and a camera lock: the gameplay camera set to the native controller camera
   (needs I4 for S1/S2/S6) or an explicit pose for S5.
4. Frame naming into `_extract/<task>/<scenario>_<instant>.png`, paired by file name with the reference shot; a markdown table of what to compare.
5. HUD capture on (prompts are part of the comparison); canopy animation time exposed so the opening instants are exact.
6. Positions: the reference positions are not recorded. Landmarks (base star emblem, radar dish, building 17, fuel depot) are chosen by the
   capture agent from the location tables; any approximation is labelled non-native.
7. Reference side: shots are 2561x1429 original frames; compare in words (the quickstart forbids re-opening images repeatedly). Needed from the
   user only if a native-position capture is wanted: a spawn log or console coordinates while recording the original.

**Harness (JC2-950d, built):** `jc2 <game> --chute-scenario jungle-deploy-day|base-night-behind|bank-hard|landing-collapse|canopy-from-above|freefall-poses [--chute-out DIR]`,
or `scripts/chute-capture.ps1` (all six, each under the build lock; frames in `_extract/parachute/`: `<scenario>_<instant>.png`, `<scenario>.md` compare table,
`<scenario>.json` receipt with game time, Rico's state and camera per frame, and a joined `index.md`). Code: `crates/jc2-game/src/chute_capture.rs`. Items 1 to 5 above
are covered: fixed-tick timeline writing the same `HeroInput` fields as the keys, the clock stopped at a fixed hour, NPCs/traffic off, frames taken with the virtual clock
paused (at most one rendered frame of drift), Rico held 6 s at altitude so the scenery streams in before the fall, orbit camera held behind (S1 to S4, S6) or an explicit
pose above the canopy (S5, 9 m up and 3 m behind, a non-native choice). Positions are non-native: jungle tile 40,40 middle (S1, S5, S6), the start base (tile 29,18,
x -1245 z -7050; S2 by night, S3 by day, S4 by night). Limits: frames are window-sized (1280x720), not 2561x1429; S4's "before touchdown" frame is estimated from the feet's
height over the fall speed and lands about 0.1 s before touchdown (the fall speeds up at the end); the vehicle-roof landing of S4 is not scripted; "height above ground"
in the receipts is the first collider below Rico (roofs included); no RNG exists to fix.

A native-resolution comparison of final parity needs R1 (canopy model) and R2 (shading); S2/S4/S5 cannot pass before them.

## 6. Recommended order

1. In parallel: R1, R3, R4, existing 57fed721/6f204aa8/ce7795e7/9a361f01/9cfa8abf, I3, I4.
2. I1 then I2 then I5 and I6 (single-file chain), I7 as a stopgap after I1.
3. After R1/R2/R3: file the model/material/deploy-collapse implementation tasks (not filed now: unknown inputs).
4. Re-run S1 to S6 with I3 and review gap by gap.

## Gaps (of this audit)

1. Reference positions and original-game spawn data for the shots are unknown (affects S1 to S6 fidelity).
2. Reference shots were sampled, not all 85 opened: the freefall and cut-away groups were checked by file name only.
3. RESOLVED (bce93556): the pivot is the character origin at the feet, not the harness (`parachute-attach-pivot.md`).
