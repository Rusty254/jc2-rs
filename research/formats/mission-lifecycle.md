# Mission catalogue and launch lifecycle

Implementation checkpoint, 8 October 2026. Runtime:
`jc2-game/src/{missions,mission_access}.rs`; narrowly scoped lifetime hooks in
`world.rs`, ordinary `scenery.rs` initializers and `destruction.rs`.

## Implemented contract

`MissionDb.catalog` joins the 65 Agency/faction mission/faction task/stronghold
BMF records to localized names/descriptions, employer icons and discoverable
script/employer archive paths. Collectables and races are excluded from this
catalogue. The original BMF flag bytes and employer icon node remain intact;
icon `visible`/`enabled` defaults do not substitute for runtime availability.
Faction labels come from the BMF record, including the f2t10/Roaches mismatch.
Catalog presence is not evidence of an end-to-end playable mission.

`availability` is the common policy used before preparation and again before
activation. The current **jc2-rs policy** is:

- Known supported mission kind and a discoverable script are required.
- A giver origin must name that mission's actual employer icon.
- One running or preparing mission blocks all new launch requests. Requests
  never implicitly replace the active mission; duplicate pending requests leave
  the original generation/manifest intact.
- Normal access requires the prerequisite and exact inclusive chaos threshold;
  self-prerequisites are absent. Completed missions are unavailable.
- Completion queries unify `MissionsDone` and taken `Strongholds`.
- Only an explicitly Debug-origin request can bypass unlock/completion checks.
  It still requires a known script/kind and cannot replace an active mission.
- Completion rewards are granted only on first completion, including debug
  replays and the existing simplified stronghold takeover. Failed/cancelled
  runs can be retried from fresh script state.

These decisions are explicit project behavior. Unknown retail BMF flags,
employer router opcodes and replay semantics have not been guessed.

## Resource readiness

`MissionLaunches::request` prepares the parsed script and allocates a unique
`MissionOwner`. `preparation()` exposes the generation, origin, script path,
the corresponding `.location` identity when a full archive path is discovered,
start position, discovered employer archive, unsupported classes and team spawn
set count. This is a preparation manifest, **not a complete dependency closure**.

Normal giver/PDA requests stay pending until a trusted resource owner calls
`acknowledge(owner, result)`. A success must mean its explicitly supported
locations/resources are usable; enqueueing loads, parsing the script or seeing
a terrain tile is insufficient. Failed acknowledgements yield a rejection and
HUD notice. Stale or duplicate acknowledgements are rejected. Availability is
rechecked before the prepared run activates.

The bounded production provider in `prepare_location_resources` supports the
existing f2t10 exported-geometry fixture: actual pending-generation collider/unit
initialization, model entity registration and nested asset readiness precede its
receipt. It requests/pins the exported resources' tiles and releases them on
cancellation/failure. Other mission closures, team spawn sets, nested entity spawns
and cutscenes explicitly fail. Distance-trigger/warning behavior and audio are
outside this geometry receipt. See [world-locations.md](world-locations.md) for
shared city geometry and the remaining gameplay ownership migration.
`MissionRun::start` only sets ACTIVE. `resources_loaded` separately sets
RESOURCE_LOADED and evaluates the rules; the f2t10 proof verifies objective one
cannot activate without that acknowledgement.

Existing `--mission` becomes an explicit Debug-origin override through the same
service, preserving CLI positioning and settled-Rico waiting. Its readiness is
deliberately labelled **DebugScriptOnly** in a warning: parsed script plus the
existing startup gate, with no certification of mission locations, teams or
cutscenes. The CLI remains a partial mission debugging workflow.

## Cancellation and object lifetime

`cancel(owner)` applies only to that pending/running generation. A stale
cancellation cannot stop a later retry. Cancellation clears the run, subtitle,
objective, mission presentation, queued script events and newly introduced
script-wired world flags. Pre-existing world flags and unrelated events survive.
The HUD then rebuilds unrelated colonel markers/pins normally.

Exported mission entities own their immediate colliders, asynchronous queued
models, destructible units and generated debris via `MissionOwner`. Cleanup
despawns their roots/children, drops their pending model work and releases only
the model references introduced by that generation. Shared tile references and
ordinary scenery remain. Cancelled units become inert tombstones so existing
Piece indices remain stable. `spawn_models` admits prepared or active generations
and rejects an old generation's queued work after both lifetimes end. UnitDestroyed carries
the generation so a stale destruction event cannot advance another run.

End/failure cleans owned geometry too. Failure restores transient event flags
like cancellation. Successful completion keeps emitted world progression flags;
the completed-ID/taken-state ledger controls subsequent unlocks and first rewards.
NPC/team/cutscene/pickup ownership is outside this currently supported entity
path and must be added by the corresponding runtime milestones.

No checkpoint snapshot or save persistence is implemented. Retry begins from the
original script, while world destruction/chaos progress remains as before. This
does not claim to reproduce retail retry rollback or sabotage payout semantics.

## Validation

Eleven new headless checks cover the real 65-entry catalogue/f2t10 giver,
threshold/self-prerequisite/taken-stronghold policy, explicit debug override,
active/duplicate rejection, pending readiness, resource failure, late completion
revalidation, cancel/retry/stale receipts, event/presentation/entity cleanup,
idempotent rewards, shared queued-model reference preservation and actual silo
EPE collider/model/unit ownership. The existing full f2t10 objective-by-objective
proof remains the mission behavior fixture. Review extended the failure fixture
to cover cleanup before hero settling and the cancellation fixture to cover
nested children. The branch baseline passed 324 retail-data release tests;
integration on master (isolation 10) passed all 333 tests, matching source markers.
Release workspace clippy with warnings denied passed.

## Remaining evidence handoff

- JC2-019 delivered `mission-resources-next.md`: RESOURCE_LOADED is a bound
  mission boolean driven by its named location completion; accepted/queued loads
  differ from location object initialization/registration (state 4). Generation
  suppression is an explicit project policy, not proven retail behavior. The
  initial producer of normal readiness is the bounded JC2-009 f2t10 geometry provider; broad closures remain unsupported.
- JC2-004 owns original PDA/Lua routing; the catalogue does not implement a guessed
  retail launch list. JC2-021 giver/PDA implementation remains a later dependency.
- Original employer status/router activation, BMF flag meanings, intro lifecycle,
  mission-specific resource dependency closure and restart/checkpoint rollback
  remain unresolved. Existing mission/world/save briefs cover their initial leads;
  coordinate a bounded follow-up before adding new board tasks.
