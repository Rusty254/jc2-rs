# Agent quickstart (read this, your board task and the research doc it names — nothing else)

Goal: 1:1 graphics/gameplay parity with Just Cause 2. Never invent constants: cite the native
address/file; label anything unproven "inferred"/"non-native" in a comment.

**Paths:** MAIN = `C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs` (on `master`,
never edit it directly). Game = its parent folder. Board = `http://127.0.0.1:4310/api` (Python
urllib for writes, `expectedVersion` on every PATCH/comment; see TASKS.md only if stuck).

**Start:** `git merge-base --is-ancestor master HEAD || git reset --hard master`.

**Every shell that runs tests:** `export JC2_TEST_DIR="<game folder>"` (else retail tests skip silently).

**Build/test** (always the lock, your isolation number N, release):
```
# builds hold one of 2 shared slots (CARGO_BUILD_JOBS=6 inside the lock)
scripts/with-build-lock.sh <actor> scripts/cargo-isolated.sh N test --release -p jc2-game
scripts/with-build-lock.sh <actor> scripts/cargo-isolated.sh N test --release --workspace
scripts/with-build-lock.sh <actor> scripts/cargo-isolated.sh N clippy --release --workspace --all-targets -- -D warnings
```
Never wrap these in `bash -c`. Each worktree has its own `target/`. Probes that call jc2tool need
absolute paths; `--export` = MAIN/research/analysis/pseudo/_unnamed.

**Work rules (token budget):**
- Read only what the task needs; grep before reading whole files.
- Commit WIP after each working step (work must survive an interruption).
- Captures (user instruction, 11 Oct 2026: the user will build project timelines from them): every visual
  change (and any bug fix the user can see) saves a **before** capture (from master, before your edit) and an
  **after** capture from the same spot, camera, time of day and resolution, into
  MAIN/_extract/<task>/ as `before_<scenario>.png` / `after_<scenario>.png` (scenario = short slug such as
  `jungle-ridge`; several pairs are fine, keep them small). Non-visual or research tasks need none. Also
  append one JSON line per pair to MAIN/_extract/timeline/index.jsonl (create it if missing; append only):
  `{"date":"YYYY-MM-DD","task":"JC2-NNN or board id","commit":"<merge or branch commit>","scenario":"...","before":"<task>/before_x.png","after":"<task>/after_x.png","note":"one sentence on what changed"}`.
  Never overwrite or delete earlier captures. Describe the difference in words; do not re-open images unless
  something looks wrong.
- No background monitors or watchers; end with nothing running.
- Scratch files only in a scratchpad subfolder named after your actor.
- Never push to GitHub or touch GitHub issues.

**Finish (no review step):**
1. `git merge master`, then full workspace test + clippy must pass (research: probe exits 0).
   A test that fails only under load and passes alone (JC2-162) may be re-run once.
2. Merge under the merge lock (separate from the build slots):
   `scripts/with-merge-lock.sh merge-<actor> git -C "$MAIN" merge --no-ff <branch> -m "Merge JC2-NNN: <summary>"`
   On conflict: `git -C "$MAIN" merge --abort`, merge master into your branch, re-test, retry.
3. Board: one comment (merge commit, files, test/probe counts, native vs inferred, Gaps), status
   `done`, progress 100, tag `merged`.
4. Gaps: file a `ready` research task only for a gap that blocks visible or gameplay parity
   (tags `research`, `parity-gap`); list the rest in your research doc.
5. Final reply to the orchestrator: at most 5 lines — merge commit, test count, tasks filed, one
   line on anything that needs a decision.
