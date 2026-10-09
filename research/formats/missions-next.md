# Larger mission research: cutscenes, teams and coverage

Research date: 2026-10-08. Research only: no game launches, builds, or game-data edits.
This brief uses the shipped archives, existing `target/jc2tool-research.exe`, current
Rust source, and the original executable's local decompiler export as primary evidence.
No external claims or browsing were necessary. No AGENTS.md was found inside jc2-rs.

## Findings verified in current source

The system already decodes the objective chain and a useful subset of script objects.
It does not implement the larger missions' dependencies. Successfully loading a script
or displaying its first objective is not evidence that a mission is playable end to end.

* `crates/jc2-formats/src/missions.rs:728` defines the supported `ScriptKind`s:
  event/time/count/shape triggers, objective messages, target indicators, map icons,
  dialogue, exported-entity spawns, and restart points. `decode` at line 856 maps every
  other class to `Other`.
* `ScriptKind::Spawn` records a `team: bool`, but no agents. `script_objects` at line 933
  deliberately stops recursion at CSpawnSet. Its agents' own triggers and markers are
  consequently unavailable to a mission runner.
* `crates/jc2-game/src/missions.rs:789`, `spawn_entities`, only matches
  `entity: Some(entity)`. Team-only spawn sets never reach spawning here. A set with an
  exported entity follows the placement/destruction path; this is not a scripted-team
  runtime.
* `crates/jc2-formats/src/character.rs:83`, `TeamSpawn`, and `AiPlacements::from_property`
  already decode location teams, patrol routes, guard areas, faction and on/off events.
  Agent data is reduced to role strings. `npc.rs:363` queues soldiers from streamed
  location teams. This provides reusable NPC creation pieces, but not mission teams,
  escort controllers, vehicle occupants, per-agent death events, or wave contracts.
* The colonel reader in `missions.rs:1209` proves richer STeamAgentInfo fields can be
  extracted: health #1df63231, weapon_name, doll #e47c51fc, death event #71e44fd6,
  role #78efe1bc. Extend this rather than inventing a second agent representation.
* `MissionRun::start` at game missions.rs:335 sets RESOURCE_LOADED immediately. Loading
  referenced mission locations/resources has no asynchronous readiness contract here.
* The mission rule evaluator emits checkpoint events (line 374), but restart-point
  support currently chooses the initial position. The code does recognize
  `msy.mission.fail` (line 414); producers such as distance warnings are missing.
* Strongholds use an assumed 60-second hold and heat increase in place of their
  technician escort missions. See `research/formats/missions.md`, Strongholds section.

## Archive survey verified with the existing diagnostic tool

Ran `missions . --all`, then `missions . <id>` for all 65 Agency/faction mission/
faction task/stronghold records. Counts below are top-level script objects exposed by
that existing tool, not nested agent counts or a transitive dependency audit. These are
archive diagnostics; the binary can lag current source. They do not claim playability.

| Mission | Script objects | CCutsceneObject | Team spawn sets |
|---|---:|---:|---:|
| f2t10, current proof task | 33 | 0 | 0 |
| f1t12, Stop the Press | 19 | 0 | 0 |
| f3t12 | 17 | 0 | 0 |
| f1m03, Airport Troubles | 110 | 0 | 12 |
| f2m05, Save the Forest | 108 | 1 | 7 |
| f2m08, Taking Care of the Dishes | 125 | 1 | 2 |
| f1s07, harbor takeover | 162 | 7 | 18 |
| km01 | 527 | 4 | 31 |
| km02 | 374 | 9 | 13 |
| km03 | 439 | 10 | 75 |
| km04 | 173 | 8 | 6 |
| km05 | 2658 | 8 | 13 |
| km06 | 508 | 8 | 43 |
| km07 | 160 | 5 | 5 |

All seven Agency scripts and all nine stronghold scripts contain cutscene objects.
Two of the eleven faction mission scripts contain them (f2m05 and f2m08). None of the
38 faction task scripts has a top-level cutscene object. This does not mean faction
tasks lack introductory cutscenes: employer smart objects have their own dependencies.
All nine strongholds contain 18-26 team spawn sets. Agencies range from 5 to 75.
Simple tasks are better next proof cases than km01 or km05.

Additional unsupported classes seen in the survey include CAIAttackTarget,
CEnterAndWaitForAnimSOAction, CSmartObject/CSmartObjectTrigger/CSmartObjectAction,
CDisarmAction, CHealthControl, CFactionRelationSetter, CPlayerStateObject,
CHeatStatusRouter, CAreaDamage, CForcePulse, CCameraObject/CCameraTransitionObject,
CAnimSpline/CAnimTranslation/CAnimRotation, and CLocalSpawnArea/CSpawnNode.
Many missions therefore require interactions, vehicles, animation or scene resources
in addition to cutscenes and soldiers.

## Concrete cutscene hooks

Verified raw property nodes from `km01.blz/km01.bl.bl`:

| Field | Example value | Interpretation status |
|---|---|---|
| #32b409e0 | km01_micro_01.seq | Sequence reference, verified value; semantic name unknown |
| #1a518747 | km01.miniCutscene02.start | Likely play input; inferred from event naming/data |
| #3947ec14 | msy.km01Obj12.activated | Additional input; precise lifecycle action unknown |
| #9ded931f | msy.km01Obj12.complete km01.sam.cutscene.complete | Likely completion output; inferred |
| #fa3d1836 | hatchOpen ply.kill.dualhook | Other emitted/lifecycle events; exact phase unknown |
| #25114d20 | 500.0 | Possible distance; unknown |

The same class references km01_01c\\km01_01c.seq, km01_01b\\km01_01b.seq and
km01_01\\km01_01.seq. For 01c its #9ded931f value is msy.km01Obj02.complete; for
01b it is msy.km01Obj01.complete. Ignoring those nodes leaves objective transitions
without their intended event producer. The mission-access research also found
#9ded931f on the giver's CCutsceneObject `cso.intro.play`, the same class nested in
the giver graph. Class-specific event semantics still require loader tracing.

Original consumer: `research/analysis/pseudo/_unnamed/text_00910000.c`,
#1523c5bf/#af2c988f/#3947ec14/#22fbb398 and output hashes #0a19cd4e/#fa3d1836/
#9ded931f. It constructs a path under `global\\cutscenes\\` and also derives a
`.location` name. Exact per-event semantics and property defaults still need tracing.
Class getter: FUN_0056e630 at 0x0056e630 returns CCutsceneObject.

Sequence archives are named `<sequence>.seq.blz`, not just `<sequence>.seq` in the
global index. `km01_micro_01.seq.blz` is 1374 bytes in pc1; its member
`km01_micro_01.seq` parses with the existing property reader. It contains
CCutsceneTrackModifierTransform, CCutsceneTrackCamera, and CCutsceneTrackSound.
Camera keys explicitly carry `time` (0, 3.6, 3.9, 5, 5.3, 7), active, and unknown
#e10bc769 values (45, 30, 15). These resemble field-of-view values but are not decoded
as such without consumer verification. Sound track fields include soundbank_id=28,
sound_id=65, perception_mode=2. This is an excellent small cutscene decoding fixture.

`km01_01.seq.blz` is much larger: SARC v2 with 1380 entries, including camera and
character .ban animations, .bsk skeletons, models, textures, dolls, the 318181-byte
sequence property member and a 335-byte `.seq.bl.bl` member. The full opening sequence
requires resource/entity/character animation tracks, not merely playing a video file.
`f1m03_01.seq.blz` has only its sequence and `.seq.bl.bl` members and is another
compact camera/modifier/spline fixture.

## Team hooks and remaining unknowns

Original loader candidate: `research/analysis/pseudo/_unnamed/text_005f0000.c`,
Related CTeamComponent consumers are in text_005b0000.c:13545 and
text_00650000.c:1937/1986; `.aiteam` consumers are in text_00740000.c:3310,
text_00840000.c:6250 and text_008a0000.c:13967. These are leads, not fully identified
behavior functions. Existing TeamSpawn fields: #f5afca6d team definition,
#dfbe4772 patrol route, #a870db99 operation area, #78efe1bc role.

Unknown: .aiteam agent/default merge precedence, AI orders and role controllers,
weapon_name enumeration, team creation/death/all-dead outputs, respawn/wave policies,
mission-owned vs streamed-location lifetime, technician routing, vehicle seats and
vehicle AI. Decode mission-specific overrides before reusing generic enemy-roster
health/weapon/doll selection; those defaults can erase scripted identity.

## Implementation milestones inferred from the evidence

1. Build a transitive coverage audit before claiming more supported missions. Start
   from all 65 records; include employer containers, scripts, exported .ee/.epe,
   mission locations, team definitions and sequences. Emit unsupported classes AND
   unresolved event producers/consumers, parse failures and missing resources. Presence
   counts alone are insufficient. Keep supported/partial/blocked status per objective.
2. Expand the script representation to retain typed team agents and class-specific
   cutscene lifecycle fields plus unknown/raw properties. Keep instance GUID scoping
   and lowercase event matching from the existing mission runner.
3. Implement distance/fail warning, creation/status routing, teleport and resource
   readiness. Prove start/fail/restart/completion and mission-owned resource cleanup.
   Connect mission-specific locations to the world.bin registry being investigated by
   world-streaming research. RESOURCE_LOADED should reflect actual readiness.
4. Implement a cutscene clock and sorted keys using km01_micro_01; retain camera state,
   suppress player control, deliver lifecycle outputs exactly once, and restore state
   on completion/skip/failure. Verify event phase semantics against the original loader
   before enabling automatic skip. Then add camera spline/modifier, event, dialogue,
   subtitle and audio tracks, followed by character/entity/resource animation tracks.
5. Connect mission team creation to the existing NPC infrastructure with per-agent
   health/doll/weapon/death events, faction, route, area, and mission lifetime. Use one
   explicit team fixture, then a wave and friendly escort fixture. Do not call the
   strongholds implemented until the technician route/death/defend stages work.
6. Broaden playable proofs from f2t10 to small candidates f1t12/f3t12 after their full
   dependency audit; then f1m03 for teams, f2m05/f2m08 for cutscenes, and a stronghold.
   The Panau City harbor f1s07 depends on additional city streaming, so another
   stronghold can isolate escort work first. km01 remains a later integrated proof.

Meaningful acceptance checks: genuine gameplay events advance each objective; entity
spawns/deaths and subtitles produce required outputs once; no synthetic completion to
hide unsupported classes; leave-area and protected-agent death fail correctly;
checkpoints restore mission entities/events; cutscene skip reaches the same mission
state as normal completion; resources disappear on end/restart without losing world
progress. No such runtime checks were executed in this research pass.

## Reproducible read-only commands

Run from the game directory with the existing binary (no Cargo required):

```
jc2-rs/target/jc2tool-research.exe missions . --all
jc2-rs/target/jc2tool-research.exe missions . km01
jc2-rs/target/jc2tool-research.exe prop . km01.blz/km01.bl.bl --json
jc2-rs/target/jc2tool-research.exe sarc . km01_01.seq.blz
jc2-rs/target/jc2tool-research.exe prop . km01_micro_01.seq.blz/km01_micro_01.seq
```

Avoid truncating a live tool pipe with Select-Object -First: Rust stdout can report a
broken-pipe panic. Capture the complete text first when taking a short excerpt.
