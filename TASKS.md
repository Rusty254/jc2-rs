# Shared agent task board

The authoritative board is [Relay](http://127.0.0.1:4310/).
Agents read and update `http://127.0.0.1:4310/api`. All 18 tasks from the former
JSON board were migrated with status, assignee, scope, acceptance criteria and
work-log notes. Tasks have the `jc2-rs` tag and their original `JC2-NNN` ID in
the title and tags. [ID mapping](notes/task-board-migration.json) records API IDs.

`notes/task-board.json` is a frozen migration snapshot. Do not update it or merge
worktree copies as live state. Use the API from every local agent/worktree.

## Read and claim

```powershell
$api = 'http://127.0.0.1:4310/api'
$tasks = Invoke-RestMethod "$api/tasks"
$tasks | Where-Object { $_.tags -contains 'jc2-rs' } |
    Select-Object id, title, status, assignee, version

# Select a ready project task; inspect scope and dependencies first.
$task = $tasks | Where-Object { $_.tags -contains 'JC2-007' }
Invoke-RestMethod "$api/tasks/$($task.id)"
Invoke-RestMethod "$api/tasks/$($task.id)/claim" -Method Post `
    -ContentType 'application/json' -Body '{ "assignee": "my-agent" }'
```

Only claim ready tasks (promoted by `claude/orchestrator`). The server's claim endpoint provides atomic ownership.
Do not assign yourself with PATCH in place of claiming. Backlog tasks are proposals;
the coordinator checks scope/current evidence before promoting them to ready.
Six imported assignments remain blocked reservations pending verification of their
actual branch/delivery state. Their presence does not imply agents are running.

Dependencies are preserved in descriptions, including API IDs for JC2-010.
Check each prerequisite is done before claiming; do not assume the server enforces
these description-based dependencies. Coordinate overlapping modules.

## Update and report

Fetch a task's current version before each write. Pass `expectedVersion` on PATCH
and comments; a conflict requires rereading and reconciling the change.

```powershell
$task = Invoke-RestMethod "$api/tasks/$($task.id)"
$body = @{ text = 'Branch/worktree, files changed, validation and remaining gaps'
    actor = 'my-agent'; expectedVersion = $task.version } | ConvertTo-Json
Invoke-RestMethod "$api/tasks/$($task.id)/comments" -Method Post `
    -ContentType 'application/json' -Body $body

$task = Invoke-RestMethod "$api/tasks/$($task.id)"
$body = @{ status = 'blocked'; actor = 'my-agent'
    expectedVersion = $task.version } | ConvertTo-Json
Invoke-RestMethod "$api/tasks/$($task.id)" -Method Patch `
    -ContentType 'application/json' -Body $body
```

Supported states: `backlog`, `ready`, `in_progress`, `blocked`, `done`.
For review, keep work `in_progress`, add an `awaiting-review` tag (preserving other
tags), and post commit/branch, checks/results and a reviewer handoff comment.
The coordinator (`claude/orchestrator`, see AGENTS.md "Coordinator and review") reviews,
merges and marks done with progress 100 after review/integration and required checks;
agents never merge to master or mark their own work done. Research completion does not imply runtime implementation completion.
Record blockers in comments and keep the assignee. Verify stale work before
releasing/reassigning it; board access is coordination, not proof of authority.

Create with POST `/tasks`: title, description, status, priority, type, assignee,
tags, progress and actor. Use a `jc2-rs` tag, bounded scope, acceptance criteria
and dependency references. New proposals belong in backlog. Read activity with
GET `/activity`; GET `/events` streams live updates.

This API is local to the machine. If unavailable, preserve your handoff and retry;
do not resume writes to the retired JSON board. Task claims do not grant build
slots: follow STATUS and HANDOVER's existing serialized build protocol.
