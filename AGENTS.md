# Agent entry point

**Working on a board task? Read `AGENT_QUICKSTART.md` instead of this file** (plus your task and
the research doc it names). This file is the full reference; the orchestrator reads
`notes/ORCHESTRATOR.md`.

Read `STATUS.md` for integration state and `TASKS.md` for shared task coordination.
Use the Relay API at `http://127.0.0.1:4310/api` to read and claim work before
editing. All local agents/worktrees share this live board. The old
`notes/task-board.json` is an archived migration snapshot, not live state.
Research evidence and build/worktree instructions remain in `research/` and
`notes/HANDOVER.md`; the board tracks ownership and delivery, not format facts.

Only claim ready tasks. Backlog items are proposals, not authorization to dispatch
new agents. Check dependencies in descriptions and coordinate overlapping modules
with existing owners before changes. Keep release builds serialized using the
existing build-lock and isolation scripts. Record branch/worktree, changed modules,
validation and remaining gaps in API comments. Use expectedVersion on updates.
Merge completed work yourself after the required checks (see "Coordinator and merging")
and mark the task done with a handoff comment. Preserve unrelated local edits
and never commit game assets. See TASKS.md for endpoints and examples.

## Coordinator and merging (from 11 October 2026: no review step)

User instruction, 11 October 2026: remove the review step for code to be merged in. Agents
now merge their own finished work into `master`; `claude/orchestrator` assigns tasks and keeps
the board but no longer reviews before merge. `codex/*` workers and agents without an assigned
board task still do not merge.

**First step in every new worktree:** run `git merge-base --is-ancestor master HEAD`. If it fails, your
worktree was created from the public snapshot (no shared history); run `git reset --hard master`
before doing anything else.

Finish and merge (every task, research or runtime):
1. Commit on your branch; run `git merge master` so the branch contains current master.
2. Runtime work: run the full `cargo test --release --workspace` and
   `cargo clippy --release --workspace --all-targets -- -D warnings` through the build lock with
   your own isolation number; both must pass. Research work: run your probe; it must exit 0.
   Set `JC2_TEST_DIR` to the game folder (`..`) first: without it the retail-file tests skip
   silently and report `ok` in 0.00s. A test that fails only under load and passes alone
   (e.g. the stunts respawn test, JC2-162) may be re-run; note it in the board comment.
3. Merge into master while holding the merge lock (`with-merge-lock.sh`; builds use the 2-slot `with-build-lock.sh`), so merges never overlap (MAIN is the main
   checkout, `C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs`):
   `scripts/with-merge-lock.sh merge-<actor> git -C "$MAIN" merge --no-ff <your branch> -m "Merge JC2-NNN: <summary>"`
   The main checkout must be clean and on `master`; never edit files there directly. If the merge
   conflicts, run `git -C "$MAIN" merge --abort`, merge master into your branch, re-run step 2,
   then retry. If master moved after your test run, merge it into your branch and re-test first.
4. On the board: post one comment (branch, merge commit, files, test/probe counts, what is
   native-proven vs assumed vs unresolved, numbered Gaps), set status `done`, progress 100,
   add tag `merged`. File each Gap yourself as a `ready` research task (tags `research`,
   `parity-gap`, grouped per system, linked to the task it unblocks).
5. Never push anything to GitHub; only `claude/orchestrator` publishes (and not at all while the
   user has paused GitHub updates).

The standards the review used to check still apply to your own work: the diff touches only what
the task covers (no game assets, dumps or unrelated edits); claims cite native addresses/files
and fingerprints; no guessed constants are presented as original (the user's priority is 1:1
parity: unknowns stay labelled unknown).

Parity gaps become research tasks (user instruction, 9 October 2026: "for missing details on tasks
create new research tasks to fill the gaps to get parity to 1 to 1"). Every hand-in lists each
unresolved native detail (address/file, what is unknown, what it blocks) under a "Gaps" heading.
When merging, the agent files the ones that block visible or gameplay parity as `ready` research tasks (the rest stay listed in the research doc) (tags `research`, `parity-gap`),
grouped per system and linked to the task they unblock. Never close a gap by guessing.

New proposals still go to `backlog`; the orchestrator promotes them. Ask questions as task
comments addressed to `claude/orchestrator`.

## Public GitHub mirror (from 9 October 2026)

`github` (remote name; not `origin`) is the public GitHub repo https://github.com/Rusty254/jc2-rs (public again since 10 October 2026). It must never receive research
derived from JustCause2.exe (research/analysis, probes, decompiler excerpts). Therefore:
- **Never push `master`, worktree branches or any other branch to `github`.** Their history
  contains executable-derived research.
- Only `claude/orchestrator` publishes, and only the orphan `public` branch:
  `python scripts/publish_public.py` (exports master, drops research/analysis, research/tools
  probes and research/ghidra, strips decompiler excerpts, refuses if any remain), review the
  commit, then `git push github public:main`.
- In committed documents cite native addresses and describe behaviour in prose; keep raw
  decompiler output out of code blocks where possible (the publisher strips it anyway).

## GitHub issues and outside pull requests (from 10 October 2026)

The repo is public again. Every `ready` board task is mirrored as a GitHub issue (board tag
`gh-<issue number>`), written in prose with native addresses only (no decompiler names), so
outside contributors can pick work up. Only `claude/orchestrator` exports or closes issues:
- Before claiming a board task that has a `gh-N` tag, check the issue for someone already on it.
- When the task is accepted and done, close its issue with the merge commit.
- Outside pull requests land on the public `main` snapshot, not on `master`. The orchestrator
  reviews them like any hand-in, brings accepted changes into `master`, then republishes;
  otherwise the next snapshot would undo them. The README text above the title is the
  author's own and is kept verbatim.
