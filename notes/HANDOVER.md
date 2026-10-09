# Handover: orchestrating jc2-rs with parallel agents

*Written 8 October 2026 by the orchestrator session (Claude Opus 5.5) that ran the work from
7 October onwards. Read `STATUS.md` first for what the game does; this file is about how the
work is run and what is unfinished.*

## 1. State right now

- **master = 260498f** (plus this handover commit). `cargo test --release --workspace`:
  **271 tests, all green**; `cargo clippy --release --workspace --all-targets`: **no warnings**.
- **Nothing is running.** No agents, no builds, no scheduled jobs. The user said "stop all
  work for now".
- **Two unfinished branches** (section 4) and one queued task (section 5).
- The game folder (`..`) is the **unmodded** Just Cause 2. The user's modded install
  (Definitive Edition mod in `dropzone\`, ReShade) is at `C:\Users\Rusty\Desktop\Just Cause 2`.
  Tests and reference screenshots target the plain game.

## 2. The job

The user asked for an orchestrator: read README/STATUS, fan the work out to sub-agents with
testable milestones, pick the model that suits each task, keep usage low but quality high,
and avoid wasting time on local builds. The orchestrator itself writes little code: it
briefs agents, merges their branches, runs the integration tests, fixes small things on
master (it fixed the driving bug itself), owns `STATUS.md`, and reports to the user.

### The user's standing instructions (verbatim where it matters)

- Model choice: the model best suited to the work. **A Fable agent needs a justification and
  the user's approval first** (none has been used). In practice: **Opus** for research-heavy or
  multi-system work (reverse-engineering formats, physics bugs, missions, animation), **Sonnet**
  for well-specified implementation, **Haiku** only for quick read-only searches.
- "To mitigate the time wasted waiting for builds, do as much work as possible before
  building, one agent waiting 2 minutes for another to finish before building is better than
  it waiting 10 mins for an in progress build to finish and then wait for its own."
- "Work until usage is used up, then start again once it's reset." (Handled with a state file
  and a recurring check; see section 6.)
- When the user pauses agents to cut build contention: "allow them to continue their work, but
  tell them not to build, they should stop when they need to build and wait until you signal
  them that another agent has finished then they can continue and build/test" (**no-build
  mode**, section 3).
- Merge each agent's branch into master yourself; keep master green.

## 3. How agents work (the protocol in every brief)

**Worktrees.** Each coding agent runs with `isolation: "worktree"` (a git worktree under
`.claude/worktrees/`, git-ignored, branch `worktree-agent-<id>`). The orchestrator's cwd must
be inside `jc2-rs` when spawning, or worktree creation fails.

**One shared target dir, always `--release`.** Dependencies (Bevy, Avian, egui) compile once.
- `export CARGO_TARGET_DIR="<jc2-rs>/target"` and `export JC2_TEST_DIR="<game folder>"`.
- **`scripts/with-build-lock.sh <label> <cmd…>`**: an `mkdir` lock so only one build runs at a time.
  Stale after 45 min. The lock lives in `scripts/.lock/` (git-ignored).
- **`scripts/cargo-isolated.sh <n> <cargo args…>`** passes
  `--config profile.release.package.jc2-{formats,game,tool}.codegen-units=<n>`. Cargo keys
  workspace crates by their *workspace-relative* path, so without this every worktree shares one
  fingerprint and a build can come back "Fresh" with *another worktree's code*. Give every
  worktree its own `<n>`; master uses **10**. Numbers 11–30 have been used; next free is **31**.
- Usage: `scripts/with-build-lock.sh mytask scripts/cargo-isolated.sh 31 test --release --workspace`.
- **Never wrap the lock in `bash -c "…"`.** Claude Code's safety check blocks it (it can't verify
  the script's `rm`). Run the lock-wrapped command directly. `cargo fmt --all` needs no lock and
  must not go through `cargo-isolated.sh`.
- Finish all research and edits for a milestone *before* taking the lock; commit after each
  milestone (survives interruptions and rate limits).
- Copy `target\release\jc2.exe` (or the newest `target\release\deps\jc2-*.exe`) to a private name
  **while holding the lock** and run only that copy: other builds overwrite it, and a running copy
  blocks their link step.
- Data digging: copy the prebuilt `target\release\jc2tool.exe` to a scratch dir.
- Leave no background processes (one agent left a drive-wide `grep -r` running for an hour).

**No-build mode** (when the user wants fewer concurrent builds): the agent may read, run its
private jc2tool copy and Python, and edit; **no `cargo` of any kind and no jc2.exe**. When it
needs a compiler it commits a WIP ("not yet compiled") and stops with a "ready to build"
report. The orchestrator then sends a build signal when another agent finishes.

**Smoke tests without a human.** `--exit-after <s>` quits cleanly and `--screenshot <png>` saves the
window first; agents view the PNG with the Read tool. Use `--world 31,13` for quick checks.
Debug flags exist for states you can't reach without keys: `--hud-demo [vehicle]`,
`--grapple-demo fly|hang`, `--ride spot|wire`, `--open-map`, `--black-market`, `--face-elite`,
`--mission f2t10`, `--heat N`, `--chaos N`, `--crewed`, `--browse-select …`, the model viewer's
`--camera-yaw/--camera-pitch/--hide-parts`, and env vars `JC2_START_OFFSET`, `JC2_CAM_YAW/PITCH`.

**Reference screenshots.** `reference\` (git-ignored, never commit) holds 692 labelled frames of
the original game in topic folders; `reference\INDEX.md` says what each shows, lists findings to
match (HUD layout, canopy, left-hand drive…) and what is still missing (elite soldiers, a reload,
a patrol, the helicopter exit). Point agents at the relevant folders. When the user drops new
bursts there, sort them the same way (contact sheets with PIL, keep 1–4 frames per moment,
rename `<topic>/<description>_<letters><frame>.jpg`, send the rest to the **Recycle Bin** via
`Microsoft.VisualBasic.FileIO.FileSystem.DeleteDirectory(…,'SendToRecycleBin')`, never hard-delete).

### Lessons that went into the briefs

- **Never use `RigidBodyDisabled`/`ColliderDisabled`**: Avian 0.7.0 corrupts its constraint graph
  when they are removed again (crash in `solver/plugin.rs:398`). Passengers go through
  `physics::stow_passenger` / `release_passenger`. A draft upstream issue is in
  `notes/avian-0.7.0-issue-draft.md`; **the user has not decided whether to file it**.
- **Never take `&mut Transform` of a physics body unless you write it.** Avian copies a changed
  (interpolated) Transform back onto `Position`; that made cars lose momentum every frame.
  Interpolation bugs only show when frames are shorter than the physics step (tests use exactly
  one step per frame unless you override `TimeUpdateStrategy`).
- **Headless tests wait by wall clock** (`fleet::testing::wait_for_hero`), never a fixed frame
  count before assets load: several tests were flaky under the parallel suite for that reason.
- **Coordinates:** the engine frame is right-handed like Bevy (x right/east, y up, models face
  −z); all conversions are the identity in `coords.rs`. The old z-mirror made the world a mirror
  image and every model inside out.
- After each merge, check the number of tests that ran equals `grep -rn '#\[test\]' crates | wc -l`
  (catches stale builds).
- Python heredocs: use raw strings for Windows paths (`"target\release"` became a carriage
  return once).
- An agent whose worktree had no changes loses it on `TaskStop`; it re-created one by hand
  (that is how the `missions` branch got its name).

## 4. Unfinished work

Both branches have the latest master merged in and their last edits committed as WIP. Their
agent ids only work inside the session that spawned them; **a new session should start a
fresh agent with the brief below, pointing it at the branch** (`git worktree add` it, or reuse
the existing worktree path).

### 4a. Character animations, shooting, gun grip (Opus)

- Branch `worktree-agent-accfeb99b78adb08a`, worktree `.claude/worktrees/agent-accfeb99b78adb08a`,
  head **a1d80c1** (WIP, may not compile; earlier commits `9c2c1eb` WIP layers, `da0a35b`
  `--pose-soldiers`, master merges 85f4d46/09f1937).
- About 2,800 lines changed: `rig.rs` (+ `rig/tests.rs`), new `poses.rs` (a pose demo for
  screenshots), `character.rs`, `combat.rs`, `npc.rs`, `weapons.rs`, `main.rs`.
- The user's request, verbatim: *"Fix character animations and add the shooting animations
  guns on the character hands are not correctly positioned"*.
- The brief asked for: evidence screenshots against `reference\` before changing anything; guns on
  the right hand bones from the data's grip/attach nodes (two-handed rifles, the launcher on
  the left forearm, soldiers too); the right clip per state, crossfades without pops,
  playback rate matched to speed; partial `.afsm`/`.asb` decoding; an upper-body aim layer
  over locomotion, recoil per shot, the muzzle flash at the muzzle, reloads timed by
  `weapons.bin`, a grenade throw; poses for the grapple states the grapple agent added
  (`GrappleRig::phase` Flight/Pull/Retract/Reel/Hang, `Hero::hanging`, `ledge_top`,
  `Hooked::lean`); tests for gun-to-hand placement, aim yaw, recoil, reload timing, clip facing.
- Status: no final report was received. Next steps: build (isolation 29 is its number), fix
  compile errors, finish the milestones it hadn't reached, run the suite, screenshots vs
  references, commit, merge.

### 4b. Original HUD bitmap fonts (Sonnet)

- Branch `worktree-agent-acdcaa9e9c3105a55`, worktree `.claude/worktrees/agent-acdcaa9e9c3105a55`,
  head **131cc95**.
- Built and tested: `114e639` (`jc2_formats::font`, `jc2tool font`) and `c1132cb` (`bitmap_text.rs`:
  `BitmapText`, layout, shadow, fallback). **Not yet compiled:** `0c18d0e`, `f9b3a6f` (every HUD
  text switched to the original faces) and `131cc95` (mission banner/subtitle fonts in
  `missions.rs`). Master merged in (`0e0f99d`).
- The font format is in `research/formats/hud.md` on that branch (7 `.font` files in
  `gui\fonts\fonts.blz`; advance = width + round(spacing); no kerning).
- Next steps: build (isolation 28), fix compile errors, `cargo fmt`, full suite (expect about
  271 + 12) and clippy, screenshots with `--hud-demo`, `--hud-demo vehicle` and
  `--mission f2t10 --exit-after 5.5` side by side with `reference\ui\`, tune sizes, and **check
  the white box around the HUD crosshair** that the grapple agent saw (suspect
  `hud_alpha_a_dif`'s alpha). Then merge.

### Merge order

Either order works; both touch `combat.rs`/`main.rs`/`npc.rs` (animation) and `hud.rs`/`missions.rs`
(fonts), which barely overlap. Merge one, run the full suite on master (isolation 10), tell the
other to `git merge master`, then merge it.

## 5. Queued and next

1. **Polish pass (Sonnet):** boats steering clear of the shore, a body while Rico is down, cloud
   shadows and rain, vehicle horns and doors, AI barks (`soundeventmap.bin`).
2. Then `STATUS.md` "Next steps": starting missions from givers and the PDA mission list,
   cutscenes and AI teams for the bigger missions, streaming the Panau City location files,
   wire-swing tilt and moving between stunt positions, the vehicle→engine-sound link.
3. Things only the user can do: hands-on play-testing (agents can't press keys), and deciding
   about the Avian issue.

## 6. Running unattended through usage limits

The user wanted work to continue across usage resets. What worked:
- A **state file** (`orchestrator-state.md` in the session scratch folder; the last copy's rules
  are summarised here) listing agents (id, task, model, isolation number, status), the queue
  with its ordering rules, and a log.
- A **recurring `CronCreate` job** at `7,30,53 * * * *` whose prompt says: read the state file,
  resume rate-limited agents with SendMessage (after freeing a stale lock if no cargo process
  holds it), merge finished ones, start the next queued task when a slot frees, and when all is
  merged run the full suite, update STATUS.md and delete itself. Firings during a usage block
  simply don't run; the first after the reset picks everything up. Cron jobs are session-only
  and expire after 7 days.
- Rate-limited agents keep their worktrees; `SendMessage` to the agent id resumes them with
  their context.

## 7. Where things are

| What | Where |
|---|---|
| What the game does, how to run it, roadmap | `README.md` |
| Current state, open questions, next steps, assumptions | `STATUS.md` |
| Each format and system | `research/formats/*.md` (coordinates, missions, chaos, hud, audio, character, vehicle, spawning, civilians, text, weather, map, world, terrain, havok, animation, weapons, rbm) |
| Build scripts | `scripts/with-build-lock.sh`, `scripts/cargo-isolated.sh` |
| Avian bug draft | `notes/avian-0.7.0-issue-draft.md` |
| Reference screenshots | `reference\` + `reference\INDEX.md` (git-ignored) |
| Ghidra export of the exe | `research/analysis/` (strings.csv, functions.csv, pseudo/) |
| Agent worktrees | `.claude/worktrees/` (git-ignored, ~240 MB; merged ones can be removed with `git worktree remove`) |
