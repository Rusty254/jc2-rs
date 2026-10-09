# Agent entry point

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
Post completed work with an awaiting-review tag and handoff comment; the coordinator
marks done after integration and required checks. Preserve unrelated local edits
and never commit game assets. See TASKS.md for endpoints and examples.

## Coordinator and review (from 9 October 2026)

**`claude/orchestrator` is the main orchestrator and the only reviewer and integrator.**
No other agent (including `codex/orchestrator` and every `codex/*` worker) merges into
`master`, commits directly on `master`, marks tasks `done` or promotes backlog tasks to
`ready`. Finish your work on your own branch/worktree, hand it in, then stop and wait.

Hand-in (every task, research or runtime):
1. Commit on your branch; run `git merge master` first so the branch contains current master.
2. Runtime work: run the full `cargo test --release --workspace` and
   `cargo clippy --release --workspace --all-targets` through the build lock with your own
   isolation number; both must pass. Research work: run your probe; it must exit 0.
3. Keep the task `in_progress`, add the `awaiting-review` tag (keep existing tags) and post a
   handoff comment: branch, commits, files changed, tests/probe results with counts, what is
   native-proven vs assumed vs unresolved, remaining gaps.
4. Stop. Do not start a dependent task on your own work until it is accepted.

Review (done by `claude/orchestrator`, in this order):
1. Scope: the diff touches only what the task covers; no game assets, dumps or unrelated edits.
2. Evidence: claims cite native addresses/files and fingerprints; no guessed constants
   presented as original (the user's priority is 1:1 parity: unknowns stay labelled unknown).
3. Checks: probe exits 0 / tests and clippy pass on the merged result (re-run on master).
4. Outcome, posted as a comment from `claude/orchestrator`:
   - **accepted**: merged with `--no-ff`, task set `done`, progress 100, tag `awaiting-review`
     replaced by `accepted`;
   - **changes requested**: tag `changes-requested`, a numbered list of required changes; the
     assignee fixes them on the same branch and hands in again.
5. If master moves under a waiting branch, the reviewer merges master during integration; the
   agent does not need to rebase unless asked.

Parity gaps become research tasks (user instruction, 9 October 2026: "for missing details on tasks
create new research tasks to fill the gaps to get parity to 1 to 1"). Every hand-in lists each
unresolved native detail (address/file, what is unknown, what it blocks) under a "Gaps" heading.
On acceptance the reviewer files them as `ready` research tasks (tags `research`, `parity-gap`),
grouped per system and linked to the task they unblock. Never close a gap by guessing.

New proposals still go to `backlog`; the orchestrator promotes them. Ask questions as task
comments addressed to `claude/orchestrator`.

## Public GitHub mirror (from 9 October 2026)

`origin` is the PUBLIC repo https://github.com/Rusty254/jc2-rs. It must never receive research
derived from JustCause2.exe (research/analysis, probes, decompiler excerpts). Therefore:
- **Never push `master`, worktree branches or any other branch to `origin`.** Their history
  contains executable-derived research.
- Only `claude/orchestrator` publishes, and only the orphan `public` branch:
  `python scripts/publish_public.py` (exports master, drops research/analysis, research/tools
  probes and research/ghidra, strips decompiler excerpts, refuses if any remain), review the
  commit, then `git push origin public:main`.
- In committed documents cite native addresses and describe behaviour in prose; keep raw
  decompiler output out of code blocks where possible (the publisher strips it anyway).

