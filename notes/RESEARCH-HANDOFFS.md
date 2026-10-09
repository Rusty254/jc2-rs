# Additional research handovers — 8 October 2026

These are research assignments for later coding agents, not implemented milestones.
Read STATUS.md and the relevant research notes first. The mission/world initial
pass is in `research/mission-world-roadmap.md`; do not repeat its inventory.

## Shared task coordination

[Relay](http://127.0.0.1:4310/) is the authoritative taskboard. Read `AGENTS.md`
and `TASKS.md`, then use `http://127.0.0.1:4310/api` from every worktree. Claim
only ready tasks atomically; use fresh expectedVersion values for updates and
comments. Keep branch/worktree, owned modules, validation and remaining gaps on
the task. Completed branches await coordinator review and integration before done.
The archived `notes/task-board.json` is not live state. This document holds
research scope/evidence; add or amend corresponding live tasks when scope changes.

| Work | Live task |
|---|---|
| Ragdoll reflected decoder | JC2-001 |
| Graphics material foundations | JC2-002 |
| Mission access lifecycle | JC2-003 |
| Original PDA / engine / teams-cutscenes research | JC2-004 / JC2-005 / JC2-006 |
| Ragdoll activation/recovery research (assignment 8) | JC2-007 |
| Animation transition/root-motion research (assignment 9) | JC2-008 |
| Remaining material channels and original alpha/fade upload | JC2-018 |
| Mission resource readiness/cancellation research | JC2-019 |
| Typed animation event tracks and cursor | JC2-020 |
| Mission giver/PDA integration | JC2-021 |
| Sun-only cloud-shadow rendering | JC2-022 |
| Passive articulated Rico ragdoll | JC2-023 |
| Original foliage fade/alpha rendering and state research | JC2-024 / JC2-025 |
| Animation clock/ownership consumers | JC2-026 |
| Data-backed vehicle engine assignment | JC2-027 |
| Durable object identity/reward ledger and native campaign restore | JC2-028 / JC2-029 |
| Shared location gameplay-spawn ownership | JC2-030 |
| Rig event-track ownership and gameplay dispatch | JC2-031 |
| Typed AI bark tables and programmer playback fixture | JC2-032 |

Task states and owners can change; read the API rather than treating this table
or a worktree's documentation snapshot as proof that work is available.

## Current coding wave

The user reports that performance feels fine in the integrated build. Profiling
is therefore off the critical path; revisit it if a regression appears.

The first three bounded deliveries are integrated:

- Reflected BRD body/joint membership, skeleton/mapper descriptors and inspection
  output are merged. Passive physics/gameplay integration follows.
- Foliage alpha policy, tangent/UV preservation and verified normal-map handling
  are merged; JC2-002 retains its detailed depth/shadow visual review gaps.
- Catalogue/availability, validated launch/cancellation and idempotent rewards
  are merged. A production location-readiness provider and giver/PDA UI follow.

The interrupted JC2-008 and JC2-018 research reservations were resumed by the
current team at the user's request. Their reviewed consumer evidence is in
`animation-next.md` and `material-parameters-next.md`, with reusable read-only
probes. Remaining unknowns are explicit; research acceptance does not authorize
backlog runtime milestones or certify GPU/gameplay parity.

Builds are serialized. Researchers can keep preparing later milestones without
building or editing the coding agents' owned modules. Typed event tracks (JC2-020)
are integrated; current implementations are JC2-009 location readiness, JC2-023
passive ragdoll and JC2-027 numeric vehicle engine assignments.
JC2-015's first-sight producer and programmer callback brief is reviewed, with
an independently reproduced read-only probe; automatic bark playback remains open.
JC2-012's native campaign design is in `persistence-design.md`; durable
identities and a writer/restore barrier remain separate backlog implementations.
Send concrete findings and remaining unknowns to the orchestrator for this queue.

Initial evidence briefs for the seven independent assignments below are indexed
in [`research/additional-research-roadmap.md`](../research/additional-research-roadmap.md).
They separate decoded facts from remaining research and proposed coding work;
their existence does not mean every requested semantic has been recovered.

## Already covered by this orchestration

- Graphics foundations: `research/formats/graphics-next.md`.
- Animation graphs, constraints and event tracks: `research/formats/animation-next.md`.
- Performance/streaming capture plan: `research/formats/performance-next.md`.
- Weather settings and sunlight-shadow integration contract: `research/formats/weather.md`.
- True ragdoll: `research/formats/ragdoll-next.md` prepares original body/joint
  decoding and physics ownership; the downed body remains a rigid visual fallback.

## Independent assignments to hand to other researchers

The first seven assignments now have initial briefs in
`research/additional-research-roadmap.md`. Read those findings before taking a
follow-up; the work below defines scope, not a request to repeat completed passes.

| Assignment | Initial brief |
|---|---|
| AI dialogue | `research/formats/ai-dialogue-next.md` |
| Material constants | `research/formats/material-parameters-next.md` |
| Water | `research/formats/water-next.md` |
| Particles | `research/formats/particles-next.md` |
| IK | `research/formats/foot-ik-next.md` |
| Weapon transforms | `research/formats/weapon-sockets-next.md` |
| Saves/checkpoints | `research/formats/save-checkpoints-next.md` |

New follow-ups for the next implementation stages are assignments **8 and 9**.

The current graphics agent added concrete follow-ups on JC2-018: recover original
foliage alpha enable/reference and instance-fade uploads across main/depth/shadow
passes; keep typed General header flags distinct from `Material.flags`; resolve
UV1/properties/AOC and CarPaint damage channels. The current Mask(0.5) repair is
provisional and does not establish original fade behavior. Read that task's latest
comment before extending the material research.

### 1. AI barks and dialogue

Decode `ai/sound/soundeventmap.bin`, identify its record layout and references,
resolve representative voice samples/events and trace alert/combat/death triggers.
Recover per-speaker cooldown, variation and concurrency semantics where possible.
Start with `audio.md`, `soundmap.rs`, the NPC/combat systems and the original
consumers. Deliver one fully traced alert bark and a typed parser/runtime brief;
do not confuse the AI dialogue map with the already decoded SFX sound map.

### 2. Material channel and shader constant semantics

Extend `graphics-next.md` rather than repeat the shader inventory. Trace original
material constant-buffer bindings, flags, MPM/AOC channels and texture colour-space
rules for General, CarPaint, SkinnedGeneral, bark and foliage. Existing evidence
shows foliage can repeat diffuse in the normal slot; blanket normal-map loading
is unsafe. Deliver per-block mapping tables with texture examples and consumer
addresses, marking unresolved channels explicitly. Include main/depth/shadow parity.

### 3. Water and shoreline rendering

Use the water shader offsets and contracts in `graphics-next.md` to trace texture
and constant-buffer setup. Recover wave motion, depth/colour attenuation, shoreline
foam, reflections and underwater behavior in separate stages. Identify a minimum
material milestone and controlled ocean/coast/underwater/boat captures. The current
sea is a transparent plane; keep physical SEA_LEVEL separate from render effects.

### 4. Particles, fire and destruction visuals

Inventory original effect definitions used by explosions, smoke, vehicle fire,
tracers and debris; trace emitter shapes, texture atlas frames, attachment space,
spawn rates, lifetime and velocity/colour curves. Compare `explosion.rs` and
`destruction.rs` with actual data consumers. Deliver a bounded original effect
fixture and parser/emitter proposal, with deterministic particle-budget and cleanup
acceptance. Visual effect counts must not alter gameplay damage or impulses.

### 5. Foot/hand placement and contact constraints

Extend `animation-next.md` with original foot-placement/IK consumer evidence:
contact windows, target spaces, two-bone limb lengths, pelvis correction, layer
weights and moving-support behavior. Footstep event times are phase evidence,
not proven foot-lock windows. Deliver slope/stair/platform fixtures and explicit
world/local/physics ownership. Coordinate rig.rs ownership with animation coding.

### 6. Exact weapon sockets, spread and recoil

Trace original gun grip/muzzle/stow transforms and weapon-to-hand correction,
including soldiers, two-handed weapons and launcher. Current muzzle points come
from model bounds and stow mounts are approximate. Resolve the outstanding upgrade
fields listed in weapons.md before assigning them spread/recoil meanings. Deliver
representative pistol/rifle/launcher mappings, known units and first-shot/reload
acceptance. Do not duplicate the already prepared animation graph/event decoder.

### 7. Save/checkpoint formats and persistence rules

Inventory original save/profile records and the game’s save/checkpoint consumers.
Determine what persists for equipment/ammo, money/chaos, settlement progress,
mission completion/retry and active mission entities. Produce a schema/lifecycle
brief with roundtrip and migration acceptance; distinguish discovered format facts
from proposed jc2-rs save design. Work from private copies when examining user saves;
do not modify live saves or include personal/game asset bytes in commits.

### 8. Original ragdoll activation and recovery behavior

Extend `ragdoll-next.md` by tracing the original character/ragdoll consumers, not
repeating the reflected BRD inventory now being implemented. Establish when fatal,
airborne and stunt profiles activate, how impulses and inherited velocity are
applied, how powered motors blend with animation, and the conditions for settling
or get-up. Use the RTTI/xref entry points in the brief. Deliver state/ownership
transitions with evidence and a bounded powered-profile follow-up; do not assume
passive Avian cone limits reproduce Havok motors or all original angular limits.

### 9. Animation condition, transition and root-motion consumers

Extend `animation-next.md` beyond its already prepared schemas and event tracks.
Trace priority when several transitions qualify, condition evaluation and trigger
consumption, finish/loop boundaries, interruption and root-motion ownership. Begin
with the small parachute graph, then one on-foot action. Deliver an evaluation
trace and golden boundary cases for the future evaluator; distinguish decoded
graph topology from confirmed original runtime semantics.

## Targeted follow-ups to the mission/world initial pass

- Original PDA action routing and launch behavior: decode the identified Lua 5.0
  chunks and icon callbacks; absence of a literal StartMission string is not proof.
- Vehicle-to-engine preset assignments: trace the original selector; preset
  existence does not establish a correct vehicle assignment.
- AI-team default/override precedence and cutscene completion/skip semantics:
  trace the specific consumers already identified in missions-next.md.
- Original mission resource readiness and cancellation: trace what actually
  emits RESOURCE_LOADED, which queues/locations it waits for and how cancelled
  asynchronous completions are suppressed. The coding agent is adding owned
  cleanup for live and pending entities; do not duplicate that implementation.

## Deliverable contract for each researcher

Produce a concise evidence brief with exact asset names, hashes/offsets and
reproducible commands; separate confirmed facts, inference and unknowns. Include
typed schema or pseudocode, runtime ownership/dependencies, a first coding
milestone, representative fixtures and meaningful acceptance checks. Use a private
prebuilt jc2tool for data inspection; no Cargo or game launches are needed for
these first passes. Keep raw assets/dumps in ignored scratch directories. Commit
only documentation and reusable research helpers; preserve existing code and saves.
