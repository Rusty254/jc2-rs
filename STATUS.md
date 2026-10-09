# jc2-rs — status, workflow and next steps

*Updated 8 October 2026. Read this first when picking the work up again.*

> Work resumed in Codex with delegated agents in git worktrees under
> `.claude/worktrees/` (git-ignored). Builds remain serialized with distinct
> fingerprints. See "Building with parallel agents".

## Current integration, 8 October

**Current user priority: gameplay and graphics at original-game parity first.**
Missions and cutscenes are deferred. Prioritize free-roam movement, combat,
vehicles, animation and rendering, using original data and native consumers.
Keep approximations explicit; a passing fixture or a rendered scene alone does
not establish 1:1 behavior or appearance. Preserve existing mission work without
expanding it. The unfinished JC2-081 audit is saved on `codex/mission-closure`
at `fdd497d` and returned to backlog; it is not merged or accepted.

The two unfinished handover branches and the first polish milestones are merged:

- Original HUD bitmap fonts, including mission titles/subtitles; the crosshair's
  opaque white background is fixed and verified in a screenshot. The damage
  vignette now stretches across widescreen viewports, verified at 1280 x 720.
- Character carrying/aim/recoil/reload layers, gun grips and soldier weapons;
  outgoing masks/playback rates, canopy towing and current-frame muzzle flashes
  have regression tests. Original state-machine decoding, precise stow/muzzle
  sockets and prior-frame tracer origins remain limitations (`animation.md`).
- Crewed boats predict shallow corridors, retain an escape steering direction,
  slow/reverse and exclude their own collider hierarchy. A real-boat physics test
  preserves a 7.0 m centre shore clearance; navigation margins remain assumed.

- Retail vehicle door sounds and a held `H` horn, with stop behavior on release,
  exit and free camera. Numbered EPE engine selectors now resolve original presets;
  switching vehicle instances stops the old voice even for identical models.
  Absent selectors retain an explicit legacy fallback. Door animation timing,
  horn release tails and controlled engine-audio parity remain later work.
- Fatal downing now creates a passive 14-body/13-joint ragdoll from the original
  BRD and render-skeleton mapper. It preserves seated/wire momentum and owns the
  bone pose until respawn, then removes joints before bodies. Missing or unsupported
  data retains a rigid fallback. Circular cone limits and reconciled anchors are
  approximations; powered profiles, get-up and original hit impulses remain open.
- Original cloud-shadow settings retain unknown hashes/types, jc2tool reports them,
  and zero cloud cover suppresses both cloud fields. Cloud-shadow rendering and rain
  are researched future work, not implemented by this settings checkpoint.

- Reflected BRD bodies, joints, skeletons and mappers are decoded and inspectable
  with `jc2tool ragdoll`, and supply the passive fatal runtime.
- Graphics foundations preserve signed tangent bases and UV1, load verified
  General/CarPaint/Bark normal maps as independent linear images, and fall back
  when normals are missing. Ten controlled pine/bus GPU captures now verify
  main/depth consistency, cutout shadows and normal-map response at two light
  angles. Ordinary General/CarPaint/Bark/Foliage colour images now follow the
  original texture-name and authored exception policy for verified DXT1/DXT5 views.
  Colour/normal handles stay independent; missing required diffuse images fail
  model readiness. Ordinary foliage now preserves raw flags and the original
  quantized instance-byte alpha test across main, depth and shadows (JC2-024).
  Exact-half boundary panels and four pine front/reverse/two-light pairs passed;
  distance scheduling, wind and full native lighting remain separate work.
- Shared mission catalogue and launch lifecycle validate availability, gate normal
  launches on explicit resource readiness, suppress stale generations, clean owned
  entities on cancellation/failure and award completion rewards once. Failed
  preparation cleans up even before Rico settles. Production preparation now covers
  f2t10's five silos and three weapon-stash exports, their model/collider dependencies
  and generation-owned cleanup. This is a bounded resource closure; teams, cutscenes,
  objective interactions and broader mission support remain separate work.
- Mission inspection and bounded f2t10 giver access share that launch service.
  `J` opens original mission intelligence with unlock status; waypoint, travel,
  start and cancel remain separate actions. Locked and unsupported missions cannot
  start. Modal input/pause restoration, employer event isolation and independent
  giver geometry ownership are tested. Briefing skip and overlap rules are explicit
  project policy, not recovered retail behavior.
- Authored jeep roof/front-cling destinations now resolve by original object IDs.
  `Z`/`Shift+Z` select supported transfers; the moving support frame and computed
  centre of mass preserve attachment and release velocity. Wire body tilt and cable
  endpoints use the final visual pose. The 0.6-second path is an approximation;
  original traversal animation, timing, events and collision remain open.
- Shared location actors now follow canonical container generations. Late
  registration creates vehicles, NPCs and pickups once; overlapping owners share
  them, and final release removes actors, pending requests and destructible parts.
  Giver-only scenery and player-held vehicle transfer have explicit ownership
  rules. Stale damage and same-tile objects from different archives stay separate.
  Durable saved identities and mission team/cutscene behavior remain later work.
- An opt-in original MC smoke preview resolves the authored graph, particle leaf
  and DDS atlas, with native indexed updates, normalized age, signed opacity,
  size, force/damping and animated atlas playback (JC2-080/087). Existing local
  particles follow the moving source. Five captures show animation, source motion,
  fading and complete drainage. Native birth phase and authored capacity are
  retained; RNG/direction, fitted geometry, material and host timing remain explicit
  fixture approximations. Gameplay effect replacement and child routing remain open.
- Tracked event audio now has one cleanup owner. Natural sound completion no
  longer competes with instance expiry/cancellation, and long-running events
  retire expired players. A final 25-second world capture has zero stale-despawn
  warnings, versus eight in the correlated diagnostic run (JC2-082).

Integration validation: **402 release tests** (152 formats + 250 game), matching
source test markers, against the unmodded Steam data; release workspace/all-targets
clippy with `-D warnings`, formatting and release workspace build all pass.
Master uses isolation 10. This checkpoint includes locations, engines, event tracks,
passive ragdolls, the diagnostic viewer, mission browser, stunt transfers, shared
location actors, smoke preview and ordinary RBM colour policy. The combined
checkpoint is `24079fe`; older counts below are historical. The contextual stunt
prompt has a fresh jeep capture showing its normal HUD controls. Colour-policy
GPU acceptance passed twelve controlled colour-policy captures. The final ten
pine/bus scene crops match the prior accepted baseline exactly; the hook uses its
original linear colour view at both light angles. A fresh 25-second integrated
world capture also passed rendering/exit checks. The observed stale entity-despawn
warnings are now resolved and accepted in JC2-082.

Graphics/material, animation-state/event and performance/streaming research briefs
are committed in `research/formats/{graphics,animation,performance}-next.md`.
True ragdoll research is complete in `research/formats/ragdoll-next.md`. Additional independent researcher
assignments are in [`notes/RESEARCH-HANDOFFS.md`](notes/RESEARCH-HANDOFFS.md).
No recurring automation has been created.

Interrupted research tasks JC2-008 and JC2-018 were resumed and reviewed. Animation
evidence establishes ordered first-match arbitration and original event interval
boundaries; material evidence links foliage instance fade/alpha uploads and
CarPaint damage channels. Read-only reproduction probes accompany the briefs.
Root-motion gameplay ownership and event interruption ownership remain unresolved.
The later JC2-025 research recovers ordinary foliage alpha enable, quantized model
fade ownership and the bounded DDS colour-space policy; alternate device state and
broader scheduler/format variants remain open. Research does not change the runtime
validation count.

Lossless typed event tracks, source/container resolution, native interval diagnostics
and a pure generation-owned playback cursor are integrated (JC2-020). The cursor's
chronological multiwrap behavior is explicit project policy; gameplay consumers
remain JC2-031. Original footstep, vehicle-entry and reload fixtures passed.
Canonical shared geometry locations and resource preparation are integrated
(JC2-009): full container identities, reference-counted owners, late extra-container
CGD loading and mission tile pins. Metropolis proximity uses an explicit inclusive
XZ policy; the original near/far loader rules remain unknown. Passive ragdolls
(JC2-023) and giver/PDA access (JC2-021) are integrated and verified.
The native campaign proposal
in `research/formats/persistence-design.md`
defines bounded validation, restore ownership and disk/migration acceptance.
It delivers design only. Durable object identities/reward ledgers (JC2-028) and
native campaign snapshot/restore (JC2-029) are newly scoped backlog work.

The first-sight alert producer, heat reset, speaker/profile read and programmer
sample callback/release are researched and independently reproduced (JC2-015).
Profile assignment and configured lookup/FEV descriptor binding remain unknown;
typed tables and an opt-in playback fixture are JC2-032. Vehicle engine assignment
(JC2-027) is integrated, using confirmed numeric EPE selectors with explicit invalid,
disabled, absent and selected states. Malformed or unresolved selections stay silent.
Shared location gameplay-spawn ownership (JC2-030) is integrated and verified;
its receipt still does not certify mission teams, scripts or cutscenes.

Weapon consumer research is integrated (JC2-017): authored A/B matrices feed
hand IK, V feeds an emission-effect frame, and stow uses separate bone slots.
Two-stage angular scatter and dimensionless per-shot accuracy loss have native
consumer evidence. A fingerprinted probe independently passed pistol/rifle/launcher
and six-level upgrade fixtures. Ballistic origins, exact slot/hand ownership and
accuracy recovery remain gated in JC2-045/046. Ground/contact IK research is
integrated (JC2-016), with native filtering, pose/world conversion and world-space
planting evidence; activation/tuning/paired feedback remains JC2-075. Stunt transfers
and wire visual tilt are integrated as JC2-011; native action timing/lifecycle is
JC2-078. Shared location gameplay ownership (JC2-030) and the bounded original
particle fixture (JC2-014) and RBM texture policy (JC2-079) are accepted.
Native smoke consumers (JC2-080), free-roam cleanup ownership (JC2-082), bounded
foliage alpha (JC2-024) and smoke update/playback (JC2-087) are accepted. Weapon
accuracy/scatter (JC2-046), General materials (JC2-085), native vegetation LOD
(JC2-086) and deterministic capture receipts (JC2-088) are active work. Mission
dependency auditing and larger playable mission proofs are deferred by the
user's gameplay/graphics priority.
Accepted JC2-025 research reproduces the original texture name/exception policy and
resource/view binding with ten retail fixtures, plus one complete foliage model
fade initialization/update/queue/draw chain. Its raw flags and five quantized fade
fixtures inform JC2-024; original shader parity remains to be implemented and checked.
JC2-086 (awaiting review) recovers the native per-asset model LOD: five `.lod` slots, a
per-descriptor scale (default 10), distance breakpoints 10/25/50/100/500 m times scale, hard
slot switching and a last-slot fade byte (`research/formats/vegetation-lod-native.md`,
`vegetation_lod_probe.py`). `vegetation_lod == 0` (default) now swaps plant models by camera
distance; impostor handover, native presence range and the fade rendering stay open Gaps.

Live ownership, progress and review now use the shared Relay taskboard at
`http://127.0.0.1:4310/api`. Read `AGENTS.md` and `TASKS.md` before taking work;
the old JSON task registry is an archived migration snapshot. Branch completion
awaits integration and required checks before a task is marked done.

Research for items 7–9 is complete for an initial pass: mission givers/PDA access,
larger mission cutscenes and teams, and city streaming/stunts/engine audio. See
[`research/mission-world-roadmap.md`](research/mission-world-roadmap.md) for the
three evidence briefs and coding sequence. Key gaps: separate employer events,
team-only mission spawn sets skipped by the runtime, broader native location
membership/readiness and native stunt action execution. Shared extra-container
gameplay ownership is now covered by JC2-030.
Original PDA direct launch behavior remains unverified. The later JC2-005/027
consumer research and implementation supersede the initial engine-assignment gap.
The original research itself added no runtime changes or test results.

The seven additional handoff assignments now have initial evidence briefs indexed
in [`research/additional-research-roadmap.md`](research/additional-research-roadmap.md):
AI dialogue, original material parameters, water, particles, foot/hand IK, weapon
sockets/recoil, and saves/checkpoints. Decoded facts and unresolved semantics are
kept separate. A read-only save probe validated five private retail save copies
and rejects truncated/invalid boundaries; gameplay persistence/retry remains
unimplemented. No builds or game launches were part of this research batch.

## The goal

A from-scratch reimplementation of **Just Cause 2** in Rust on **Bevy 0.19** with
**Avian3d 0.7** physics, reading the original game's data straight out of the Steam
install (`archives_win32`, `dropzone`, DLC). No game assets are copied into the repo.
Each system is reverse-engineered from the data and from the Ghidra decompilation of
`JustCause2.exe`, written up in `research/formats/*.md`, and covered by headless tests
that run against a small sample of the real files.

## Repository layout

| Path | What |
|---|---|
| `crates/jc2-formats` | Readers for the game's formats: archives (`.tab/.arc`, SARC `.ee/.blz/.nlz`), property files, models (`.rbm`), terrain, placements (`.blo`), Havok collision, skeletons/animations, vehicle dolls, weapons, FMOD banks, map icons… |
| `crates/jc2-game` | The Bevy app (`jc2` binary): world streaming, terrain, vegetation, physics, Rico, vehicles, NPCs, combat, destruction, audio |
| `crates/jc2-tool` | `jc2tool`: a command-line tool for digging through the data (`list`, `cat`, `prop`, `sarc`, `rbm`, `vehicle`, `anim`, `fsb`, `placements`, `terrain`…) |
| `research/formats/*.md` | What each format is and how the game uses it: start here for any system |
| `README.md` | What the game can do, how to run it, the controls, the milestone list |

## Running it

From `jc2-rs\` in the Steam folder (the game folder is the parent, `..`):

```
cargo run --release -p jc2-game -- .. --world 31,14          Rico at the village (a military base)
cargo run --release -p jc2-game -- .. --world 31,14 --heli   …plus a helicopter
cargo run --release -p jc2-game -- .. --world 31,14 --plane  …plus a plane (arve.v071_jet_fighter.ee etc.)
cargo run --release -p jc2-game -- .. --world 30,14 --boat   …plus a boat on the nearest water
cargo run --release -p jc2-game -- .. --world 31,14 --drive  …plus a car
```

Other flags: `--radius`, `--lod`, `--no-hero`, `--no-npcs`, `--no-traffic`,
`--no-vegetation`, `--bush-density 0.3`, `--flat-terrain`, `--no-physics`.
Controls are listed in `README.md`.

Tests: `cargo test --release --workspace` with `JC2_TEST_DIR` set to the Steam folder
(`..`). Every test runs against the real install; none skip. Without `JC2_TEST_DIR` they
return early (silently counted as passed). 244 tests on master (bb00c9a); `cargo clippy --workspace --all-targets` is warning-free.

Smoke runs without a human: `--exit-after <s>` quits cleanly, `--screenshot <png>` saves
the window just before. `--open-map` opens the map at start. Use `--world 31,13` for quick
checks.

## How the work has been done (the workflow)

Until 7 October, Claude worked in a cloud sandbox (a Linux container) with a mirror of
this repo, the Rust toolchain, the Ghidra export of the exe and a sample of game files.
From now on Claude Code works here, on this folder, so steps 1 and 6 below get simpler:
read the game files directly and commit straight into this repo. The loop for every
milestone:

1. **Find the data.** Use `jc2tool` (`cargo run --release -p jc2-tool -- list ..`, then
   `prop`, `sarc`, `cat --raw`, `vehicle`, `anim`, `fsb`, `placements`) to find the
   files and see what's in them. Key names that are only hashes get identified by
   hashing candidate names (Jenkins lookup3: `jc2tool hash`, plus small scripts) and by
   comparing values across many files. The research also used a Ghidra decompilation of
   `JustCause2.exe` (`strings.csv`, `functions.csv`, decompiled pseudo-C); it went with
   the sandbox, so re-export it from Ghidra if it's needed.
2. **Write the reader** in `jc2-formats`, with a test against the real file.
3. **Use it in the game** (`jc2-game`), with a headless test (no window: MinimalPlugins,
   physics, a flat test tile).
4. **Document it** in `research/formats/*.md` and `README.md`.
5. `cargo fmt`, `cargo clippy --workspace --all-targets` (kept warning-free),
   `cargo test --workspace`.
6. **Commit** here, one commit per milestone. Until now that step went through a
   `.tgz` dropped into `_incoming\`; `_incoming/` and `_extract/` are git-ignored
   scratch folders and can go.

You test on your PC (`--release`) and report back with screenshots and panic messages;
those reports have driven most of the fixes.

### Building with parallel agents

See `notes/HANDOVER.md` for the full protocol, the unfinished branches and how the work was run.
In short: all agents share the main checkout's `target` folder, always `--release`, through
`scripts/with-build-lock.sh` (one build at a time) and `scripts/cargo-isolated.sh <n>` (a
per-worktree `codegen-units` so worktrees don't share fingerprints; master uses 10). Copy
`target\release\jc2.exe` right after a build and run the copy.

## What works (milestones done)

1. **Archives and formats**: every archive and container, property files, textures, all
   121,912 `.rbm` models.
2. **Models**: the orbit viewer (`--model`), vehicles assembled from their dolls.
3. **World**: terrain tiles streamed around the player with the game's real terrain
   materials, the sea, the placed objects, procedural vegetation, static geometry, and
   the rest of the island to the horizon from the global height map.
4. **Physics**: Avian with the game's own Havok collision shapes. Cars (raycast wheels
   from the doll), helicopters, boats (buoyancy points) and **planes** (wings, thrust,
   gear springs). The vehicles the tiles' spawn sets park appear with their tiles.
5. **Rico**: his model on the game's skeleton and clips, walk/run/jump/fall/land,
   grappling hook, parachute with a canopy from the game's parachute skeleton,
   swimming, getting in and out of vehicles.
6. **Gameplay (begun)**: soldiers from the tiles' team spawn sets that guard and patrol.
   Five guns from `weapons.bin`, plus grenades, rockets and explosions. Health and
   respawn. Vehicles burn out at the health in `vehicletypes.bin`. The tiles'
   destructible objects (red barrels, towers, antennas) break into their `_dst` debris.
7. **Audio (begun)**: the FMOD `.fsb` banks decode to MP3; guns, explosions,
   footsteps, grapple, parachute and engines play (samples picked by name).
8. **Map and fast travel**: `M` opens the PDA map (`pda_map_dif.dds`) with the game's
   settlement icons and real names (text database), Rico and vehicle markers; a click
   drops Rico 250 m over the spot (an aircraft he flies goes with him). `map.md`.
9. **Day and night**: a clock (`--time`, `--day-speed`, `[` `]` `T`), the sun/moon orbit
   from the exe, sky and fog colours from `skygradient_dif.dds`. `weather.md`.
10. **Gameplay data**: soldier classes (grunt/elite) with weapons and health from
    `enemytypespawnsettings.bin`; ammo, magazines, reloads and fire rates from
    `weapon_upgrades.bin`/`ammunition.bin`; weapon stashes and dropped guns as pickups;
    heat and on-foot reinforcements (`heatsystem.bin`, `heatspawn.bin`). `weapons.md`,
    `spawning.md`.
11. **Text database**: `text\ms15text.csv` (8 languages, plus DLC locator CSVs). `text.md`.

## Earlier milestone record

### Historical master bb00c9a: 244 tests, clippy warning-free

On top of the first day's work (coordinates, passengers, civilians and traffic, stunts):
- **HUD** (`hud.rs`, `hud.md`): the game's own art from `gui\main_menu.blz` — rotating
  minimap from the tiles' map patches with the health arc, weapon and grenade slots,
  prompts, HEAT banner, notifications, world markers, damage vignette, CHAOS popup;
  developer text behind `F3`; `--hud-demo` for screenshots.
- **Helicopters, boats, planes** (`character.md`, `vehicle.md`, `spawning.md`): roof,
  belly and cling stunt positions from the dolls, hanging under a helicopter on the
  wire, seats, drivers and pilots (`crew.rs`), hijacking, heat-2 jeeps on the road graph
  and a heat-3 attack helicopter. Flags `--crewed`, `--ride`, `--heat`.
- **Gameplay** (`chaos.md`): chaos and money from `achievements.bin`'s activities,
  settlement completion, chaos stages (`aidifficulty.bin`), all weapon upgrade levels,
  resource crates, the Black Market (`K`), heat amounts from `aiheatparams.bin`.
- **World detail** (`terrain.md`, `world.md`, `weather.md`): the game's grass generator,
  tree impostors to the horizon, the displacement atlas, sky dome, clouds, stars, moon.
- **Audio** (`audio.md`): events chosen by the game's sound ids (`settings_core.bin`),
  footsteps by ground material, pan/low-pass/fades, voice limits, music by hour, chaos
  and heat from the 3-stem music banks. Flags `--no-music`, `--music-volume`, `--volume`.
- **Asset browser**: `--browse` (bevy_egui) — every file and container member, previews
  of textures, models, property files, text, Havok, animation, sound banks.

### The game data: unmodded

The Steam folder holds the plain game; the modded install moved to
`Desktop\Just Cause 2`. Tests and reference screenshots target the plain game.

### Reference screenshots

`reference\` (git-ignored) holds 692 labelled frames of the original game;
`reference\INDEX.md` says what each shows and lists findings to match. Still missing:
elite soldiers beside regular ones, a reload, a patrol, the helicopter exit animation.

### Open questions

- **Performance.** The user played the integrated build on 8 October and reports
  that performance feels fine. Performance is not a blocker for the next coding
  wave. No formal idle-machine benchmark has been recorded; revisit profiling
  if gameplay shows stalls or later changes regress performance.
- Painted in-world text and traffic direction not checked on screen since the
  coordinate fix.
- Pendulum wire motion, rendered body tilt and bounded stunt-position transfers are
  integrated. Native action timing and traversal collision remain unresolved.
- Controlled engine-audio comparisons remain open; numeric EPE assignment is integrated.
- Heat points → heat levels (100 points = one level) is assumed.
- The browser's open/copy/export/play buttons compile but were not clicked.

## Next steps

1. Finish the active original foliage alpha/fade material (JC2-024), with
   controlled main/depth/shadow checks. Free-roam audio cleanup (JC2-082) is accepted.
2. Implement native MC smoke update/playback (JC2-087) from accepted JC2-080,
   preserving explicit remaining birth, geometry and material limitations.
3. Prioritize original controls and action arbitration, animation clocks/root
   motion, grappling, stunt timing, weapon handling and combat. The live research
   tasks JC2-026/078/083 inform their separate implementation tasks; evidence
   delivery alone does not complete those gameplay systems.
4. Broaden graphics with the original terrain constants/lighting (JC2-084),
   General material channels (JC2-085), vegetation LOD transitions (JC2-086),
   water (JC2-013) and sun-only cloud shadows (JC2-022), respecting dependencies.
5. Continue vehicle behavior, powered ragdolls and ground/contact IK from native
   evidence, and compare animation, sound and rendered results with the original.

Missions, cutscenes and campaign expansion are deferred by the user's latest
priority. Shared location gameplay ownership (JC2-030) is already integrated;
do not reopen it as an unfinished milestone. New gaps go onto Relay as bounded
proposals and are promoted only after checking evidence and current ownership.

The HUD fonts, vehicle horns/doors, downed visual fallback and boat-shore steering
are already integrated. Keep performance checks lightweight unless a regression
appears during these milestones.

## Assumptions to revisit

Values are marked "assumed" in the code and the research notes where the data did not
say:

- Village soldiers are the government's, at chaos stage 0 and normal difficulty.
- Ammo: `#a0d58b16` is the ammo type, `max_ammo_N` counts magazines, `#01dbd94a` means
  automatic, `pickup_count` counts magazines.
- Heat: 100 points = one level, the 20 s cooldown and decay, 6 reinforcements, 3 m/s.
- Vehicle health is `#c097e6ef` in `vehicletypes.bin`.
- The plane lift gain (×2) and the planes' top speed (`#b92afbce` read as m/s).
- Explosion sizes for barrels and burning-out vehicles; the impulse scale of blasts.
- Sun, moon and ambient light strengths (the data has none).
- The canopy colour (matched by eye); tether lengths, speeds and range; the civilians' 50 hp.
