# Orchestrator handover (claude/orchestrator) — read this instead of the old session

Keep this session small: no recaps, no images unless a merge looks wrong, short replies.
Restart the orchestrator session (with this file) whenever context passes ~150k tokens.

## User rules
- Priority: 1:1 graphics and gameplay parity before new systems; never guess constants.
- Spend tokens on coding and research only. Minimise orchestration, reporting and admin.
- Do not report agent progress to the user unless asked; the user follows the task board, where
  agents post their concise updates.
- No review step: agents merge their own work (AGENT_QUICKSTART.md "Finish"). The orchestrator
  assigns tasks, keeps up to 3 agents busy and tidies the board. No integration re-runs.
- Usage: work until usage runs out ("go until all usage is up", no watchdog). Do not resume an
  agent after a limit stop; start a fresh agent on its branch instead (resumes reload big context).
- GitHub: paused — no pushes, no issue changes until the user says otherwise. When allowed again:
  `python scripts/publish_public.py` then `git push github public:main`; close issues for done
  tasks (board tag `gh-N`); merge any GitHub-side edits into master first (push is rejected if so).
- Fable agents need the user's approval. Sonnet for code and research, Haiku for mechanical work.
- Builds: 2 build slots (`scripts/with-build-lock.sh`, 6 jobs each, shared across worktrees) plus a separate single
  merge lock (`scripts/with-merge-lock.sh`, used only for `git -C MAIN merge`), so up to 3 agents rarely wait. The compiler
  cache (sccache) is set up only once installed and named in `.cargo/config.toml`; see notes/HANDOVER.md if it is missing.
- Captures for timelines (user, 11 October 2026): every visual change, and any bug fix the user can see, saves
  before/after captures from the same spot (agents follow AGENT_QUICKSTART.md): `_extract/<task>/before_<scenario>.png`
  and `after_<scenario>.png`, plus one JSON line per pair appended to `_extract/timeline/index.jsonl`. Put this in the brief
  for any agent started before it was in the quickstart. Never overwrite or delete earlier captures: the user builds
  project timelines from them. When an agent hands in a visual change without a pair, ask it for one before closing the task.

## Starting an agent
Agent tool: model sonnet, isolation worktree, run_in_background. Claim the task first
(`POST /tasks/<id>/claim {"assignee": "claude/<actor>"}`). Brief (3–5 lines):
> You are `claude/<actor>` on jc2-rs. Read `AGENT_QUICKSTART.md`, then board task JC2-NNN
> (id …, already claimed) and the research doc it names. Isolation number N. <one line of scope or
> coordination if needed>. Finish per the quickstart.
Use a fresh isolation number per agent (pick from 100 upwards and note it in the brief). Avoid giving two
running agents the same files (e.g. terrain_splat.wgsl, general_material.rs).

## State (11 October 2026, 11:10)
- master 2364bb7. Board: 127 done, 119 ready (24 high), 42 backlog, nothing running
  (JC2-088 is a stale codex task; leave it unless asked).
- Recently merged: native sea, forest canopy, General material terms, sky dome/fog band/stars,
  native point lights, boat buoyancy + CPU waves, Rico foot machine + locomotion, grapple reel/pull,
  parachute canopy/skydive, crouch and evasive roll, ground IK activation, grass and tiled-texture fixes.
- Ready high is mostly on-foot/parachute/animation research (JC2-253..266 range) plus JC2-112,
  JC2-106, JC2-104, JC2-103, JC2-083 (research) and JC2-089, JC2-028 (implementation).
  Pair each research task's implementation follow-up once it merges.
- GitHub paused. When unpaused: publish, then close issues whose board task is done (tag gh-N).
- Known flaky test under load: stunts respawn test (JC2-162).
