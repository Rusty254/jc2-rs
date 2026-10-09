# Mission and world research: items 7–9

Research started 8 October 2026 against the local unmodded Steam data, Rust
source and Ghidra export. This is a research deliverable, not an implementation
or a claim that new gameplay has passed tests.

## Focused briefs

- [Mission access](formats/mission-access-next.md): employer/giver interaction,
  unlock checks, mission launch and PDA integration.
- [Larger missions](formats/missions-next.md): cutscene events, AI teams,
  unsupported script classes and proof missions.
- [World and vehicles](formats/world-vehicles-next.md): location registration,
  Panau City, wire tilt, stunt transitions and engine audio links.

Each brief separates data observations from proposed behavior and unresolved
semantics. Retail assets and decompiled code remain outside the committed notes.

## Integration contracts to establish first

The following are source-backed gaps found during the coordinating review;
the implementation recommendations are proposals.

1. **One mission launch path.** `jc2-game/src/missions.rs::start_mission` currently
   consumes a `MissionStart` resource, builds a run and replaces `ActiveMission`.
   Giver and PDA requests should share validation, an active-mission policy,
   resource readiness and cleanup. The CLI can remain a deliberate debug entry
   point. `finish_mission` pays rewards and appends to `MissionsDone` without a
   prior-completion guard; decide replay/reward policy before exposing repeated
   launches through the UI.
2. **Location ownership and readiness.** `world.rs::stream_tiles` reads
   `ExtraContainers` only when loading a missing tile. Adding a mission location
   to an already loaded tile therefore needs an explicit load/reconcile path.
   Define ownership and unload rules for shared city and mission resources.
   The world brief also identifies a separate CGD gap: `scenery.rs` selects
   only the terrain tile container, omitting extra-container static geometry.
   `MissionRun::start` currently sets `RESOURCE_LOADED` immediately; a broader
   runtime needs a verified readiness condition before starting objectives.
3. **Employer state and mission state.** Giver scripts use employer-completion
   events distinct from mission completion. Preserve that distinction rather
   than interpreting every completion event as an objective or mission finish.
4. **Coverage measured by behavior.** A script parsing successfully does not mean
   its mission is playable: `ScriptKind::Other` retains unsupported classes,
   and the runtime's `spawn_entities` handles exported entities rather than a
   complete mission AI-team lifecycle. Track unsupported classes and blocking
   event paths per mission before advertising support.

## Suggested coding sequence

1. Build the shared availability/launch lifecycle and a small giver/PDA proof,
   while auditing script coverage and decoding the location registry in parallel.
2. Implement location registration, deduplication and reconciliation for loaded
   tiles. Use the missing Panau City stronghold targets as a concrete coverage
   check; see the world brief for the registry evidence and remaining semantics.
3. Implement cutscene completion/skip event contracts and mission AI-team
   lifecycle, then prove a complete larger mission before expanding coverage.
4. Preserve vehicle stunt-action data, implement transitions and wire body tilt.
   Research engine audio preset linkage as a separate task so an unresolved
   mapping does not block mission and world work.

## Validation when implementation begins

Extend the existing real-data proof mission test
`the_proof_mission_runs_objective_by_objective`; cover unlock boundaries,
active-mission rejection, employer completion, cleanup and duplicate rewards.
For locations, test registry resolution, boundary crossings, shared ownership,
loaded-tile reconciliation and unload/reload behavior. For larger missions,
verify cutscene completion and skipping emit the correct events, team death and
escort conditions drive objectives, and unsupported classes are reported.
For stunts, check permitted transitions, attachment continuity and physics-frame
behavior. An engine sound mapping requires representative vehicle-to-preset
evidence before replacing the current fallback.

## Reviewed consumer follow-ups

The [PDA callback trace](formats/pda-actions-next.md) separates intelligence-file
selection, waypoint operations and extraction. Direct mission launch and original
extraction cancellation remain unproven. The [vehicle selector brief](formats/vehicle-engine-next.md)
recovers numeric EPE module/preset assignments and identifies several mismatches
in the current name heuristic. The [mission consumer brief](formats/mission-consumers-next.md)
recovers selective AI-agent overrides and separate playback-exit/final cutscene
outputs, including the skip path; authored override-bit mapping and skipped-key
catch-up remain open. These supersede the corresponding inventory-only limits
above without implementing runtime behavior.

Relay tracks ownership/review state; further requests are indexed in
[additional-research-roadmap.md](additional-research-roadmap.md).

No build, gameplay launch or runtime test run was performed for this research batch.
