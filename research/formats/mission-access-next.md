# Mission access: givers and PDA

Research brief, 8 October 2026. Scope: item 7, read-only inspection of the installed
Steam data, current Rust sources and existing executable decompilation. No builds,
game launches or game-data edits. No applicable AGENTS.md was found in the repository,
research subtree or workspace ancestor directories. This document is the only write.

## Findings and confidence

**Verified:** the current runtime starts scripts through `MissionStart`, principally
the `--mission` CLI path. Mission giver locations and interaction actions exist in the
original data, and use a separate `<id>Emp` state/event namespace. The PDA currently
implemented in Rust is the map/fast-travel panel, not a mission browser.

**Verified:** the installed PDA asset contains mission briefing/summary, chaos-bar,
waypoint and extraction UI evidence. **Unknown:** whether the original PDA supports
directly launching a mission from a list. This research did not establish such a
launch action. Direct launch from the requested new list should be treated as a
product requirement unless further bytecode analysis proves original behavior.

**Inferred implementation direction:** one validated launch service should own all
entry points (giver interaction, requested PDA launch and debug CLI), with explicit
briefing/start phases. Merely inserting `MissionStart` on every UI click would skip
unlock rules, giver events, cutscenes, placement and lifecycle cleanup.

## Current source contracts

- `crates/jc2-formats/src/missions.rs:220`: `MissionRecord` already exposes id,
  prerequisite, eight unnamed flag bytes, kind, chaos and faction; `prerequisite()`
  explicitly treats a self-prerequisite as absent (the first mission case).
- `crates/jc2-game/src/missions.rs:739`: `start_mission` waits for settled Rico, removes
  the request, loads the `.blz`, creates and starts `MissionRun`, then assigns
  `active.0 = Some(run)`. It does not reject locked or completed missions, or an
  already-running mission, and does not perform cancellation cleanup before
  replacement. Unknown ids can fall back to `MissionKind::Other(0)` if their file
  nevertheless resolves.
- `crates/jc2-game/src/main.rs:365`: CLI setup obtains `mission_start_point()` and
  selects a tile/start position before world creation. This positioning is outside
  `start_mission`; a runtime PDA request will not inherit it automatically.
- `crates/jc2-game/src/missions.rs:603,928`: completion storage is `Vec<String>`.
  `finish_mission` takes the active run, pays, then pushes its id. Taking the run
  prevents paying the same run every frame, but there is no previously-completed
  guard: launching and completing the same id again can pay twice.
- `crates/jc2-game/src/chaos.rs:40`: `Chaos.total` is the existing unlock input.
- `crates/jc2-game/src/map.rs:83,100`: `MapState` owns modal pause, input and travel;
  `MapData` contains settlement icons. The original `map_icons` result is reduced to
  settlement icons by `settlement_icons`, so employer icons are not retained as a
  mission catalog.
- `crates/jc2-formats/src/icons.rs:34,110`: `MapIcon` retains id, position, kind and
  visible, but drops original enabled/event/extra fields. Keep raw properties or
  extend a dedicated employer model rather than assume `visible == unlocked`.

Existing `research/formats/missions.md` supplies table layout, prerequisite/chaos
interpretation, rule chains, rewards and earlier script research. These are existing
project findings; examples below were independently inspected with `jc2tool`.

## Original data evidence

`settings/icons_global.bin` contains:

| Giver | Engine position | Icon kind |
|---|---|---|
| `km03Emp` | `(-8192.245, 211.52435, 6569.447)` | 2 |
| `f1s07Emp` | `(-1888.6919, 202.22713, -4680.8755)` | 4 |
| `f2t10Emp` | `(6013.292, 248.12773, 10973.441)` | 1 |

All three are `visible = 1`, `enabled = 1` in that file, including missions requiring
progress. Those raw flags alone cannot encode runtime mission availability.
`f2t10` is offered for the Roaches per its BMF faction, although its location sits in
`f2_ularboys` and its internal template is named `mod.empreapers`: use BMF faction,
not directory/template naming, for player-facing faction labels.

### Agency giver: km03

Archive entry:
`locations/employers/keymissionemp/km03emp/km03emp.nlz/km03emp.nl.blo`.
The outer SARC has this 6,322-byte property file and `idle.aiteam` (552 bytes).

- `CSpawnSet sps.employer` is initially disabled, with `spawn_distance = 100`;
  enable events include `msy.km03Emp.activated`. It contains `CTeamComponent` and
  `STeamAgentInfo tai.employer`, role `pelle`, doll
  `missionspecific/pd_ms_doorman.cdoll`.
- The nested `CSmartObject sob.talkToEmp` has `CSmartObjectTrigger.box_size = [5,5,5]`.
  Its action's `#d6d617cc` is `msy.km03Emp.complete`, `#f6d62ea4` is
  `hud_button_18`, and `#eeca5657` is 38. English text lookup confirms
  `hud_button_18 = "Talk"`. Do not equate the box side with a radius without
  confirming smart-object collision semantics.
- A 60 m sphere drives the employer's indicator show/hide events. This is an
  indicator visibility distance, distinct from the interaction box.
- Four `CMissionStatusRouter`s chain active/locked/completed/valid checks, sometimes
  against `km03Emp`, sometimes `km03`. The data associates `#1d00a9cd` values
  0/1/2/3 with named valid/active/completed/locked routers. Names and graph wiring
  support that interpretation, but executable semantics still need confirmation.
- `CGuiMapIcon` names `km03Emp` through `#c3116fcb`, kind 2. An event trigger also
  responds to employer completion by firing `evt.resetHeat.reset`.

### Faction task giver: f2t10

Archive entry:
`locations/employers/f2_ularboys/f2t10emp/f2t10emp.nlz/f2t10emp.nl.blo`.
The 90-entry outer SARC includes the 52,085-byte property file and normal models,
textures and collision assets.

- `CSmartObject sob.talkToEmp` is initially disabled and enabled by its local
  `enableSob` event. Its trigger has the same `[5,5,5]` box. Its action fires three
  events: local `intro.play`, local `disableSob`, and location `vehicle.spawn`.
  `#f6d62ea4 = hud_button_20`, whose English text is **Activate**, not Talk.
- `CCutsceneObject cso.intro.play` listens to the intro event (`#1a518747`) and names
  `f2t10/f2t10.seq`. Its `#9ded931f = msy.f2t10Emp.complete`; `#fa3d1836` names
  `cutscene.started`. Interpretation of #9ded931f as a completion/end event is
  strongly supported by the flow, but this research did not decode its C++ loader.
- Separate spawn sets listen to `msy.f2t10Obj01.activated` and the cutscene's
  `spawnBox` event. Their vehicle/stash effects belong to giver launch coverage.
- Status routers check the employer's lock and mission validity/completion before
  enabling the interaction. Distance triggers carry thresholds 25, 60 and 65.
  A generic 300 m or 60 m mission-start radius would not reproduce this data.

### Global automatic starts

`global/startkeymission.blz/startkeymission.bl.bl` contains a 10-second timer from
`msy.km01.completed` to `msy.km02emp.complete`, and another from `msy.km06.completed`
to `msy.km07emp.complete`. It also listens to autosave loading to finish `km02emp`,
and can re-enable the PDA following mission completion/failure. Thus employer
completion is also used by automatic story progression, beyond physical givers.

## Executable entry points

Addresses refer to this installation's original executable/export, documented in
`research/README.md`; they are analysis entry points, not proposed runtime hooks.

- `research/analysis/pseudo/_unnamed/text_00610000.c:2575`, function **0x00614ac0**:
  debug string identifies `NMissionSystem::CEmployer::HandleEvent`. It tests an
  incoming event against an object event field at `this + 0xa4`, tests bytes at
  `+0xe2` and `+0x94`, looks up a mission through `FUN_00588fc0` with the field at
  `+0x8c`, calls its virtual slot `+0x18`, sets `+0xe2`, and emits
  **`msr.forceupdate`**. The two booleans' exact names and the virtual method are
  unknown; do not label them definitively as completed/locked/start yet.
- Employer RTTI/vftable evidence: `research/analysis/classes.csv`, CEmployer
  `0x00ebdd84`; constructor-like function `0x006823d0` in `text_00680000.c` assigns
  that vftable and initializes event fields.
- Lua API registrations in `text_005b0000.c:4490`: `GetMissionName` **0x00a2ae90**,
  `GetMissionID` **0x00a2af90**, `GetMissionNameByIndex` **0x00a312a0**,
  `GetObjectiveText` **0x00a2b160**, `GetRemainingKeyMissionChaos` **0x00a2b2a0**,
  `GetRemainingStrongholdChaos` **0x00a2b370**, `GetMissionChaosProgress`
  **0x00a2b440**, `GetStrongholdRegion` **0x00a2b640**.
- `0x00a312a0` calls **0x006be9c0** for name-by-index resolution. Several numerical
  Lua wrappers look implausibly thin in this export, so their pseudocode alone is
  insufficient for chaos-bar mathematics; verify machine code/call targets.

## PDA evidence and remaining research

The installed `gui/pda.bl` is 5,020,654 bytes and contains **14 embedded `1b4c7561`
(Lua) signatures**. First header at byte 3,735,764:
`1b4c75615001040404060809`. The version byte is `0x50`; check its full dialect and
chunk layout before using a newer Lua disassembler.

Printable constants include `txt_mission_briefing`, `txt_mission_summary`,
`current_mission`, `GetMissionNameByIndex` (offset 4,897,453),
`agency_mission_bar`, `GetMissionChaosProgress`, `PlaceWayPoint`, `RemoveWayPoint`,
and "Extraction will abort the current mission." There were no literal matches
for `msy.`, `StartMission`, or `start_mission` in this asset. Absence of those strings
does **not** prove absence of a launch action, because generic event calls may build
strings or refer to external code. Decode relevant chunks and icon action routing
before claiming exact original list/start behavior. `gui/pda.agui` and
`gui/pda_database.bin` are additional inventory targets, not decoded here.

## Coding milestones and acceptance

1. **Catalog and availability.** Join BMF records to employer icons and discovered
   archive paths. Retain raw status flags as unknown. Centralize prerequisite,
   chaos, completed and active-state queries. Treat stronghold completion consistently
   with `MissionsDone`/`Strongholds`; do not use faction prefixes as labels. Test
   exact chaos boundary, self-prerequisite, faction mismatch and completed filtering.
2. **Launch/lifecycle contract.** Add a request with origin (giver/PDA/debug), validated
   id, explicit replace/retry policy, location-readiness and briefing phases. Clear
   old objectives, subtitles, markers, entity instances and mission event state
   on cancellation. Reject duplicate requests and make completion rewards idempotent.
   Preserve an explicit debug override where useful. Decide PDA start positioning:
   stay put, travel to giver or travel to restart; CLI currently has its own path.
3. **Data-backed giver access.** Stream employer locations, implement router and
   smart-object predicates/actions, local/global event scoping and original prompt
   labels. First fixtures: km03 (NPC Talk) and f2t10 (Activate + intro + vehicle/stash).
   A temporary cutscene skip must emit the proper completion once; its limitation
   should be explicit until the larger-mission cutscene work is integrated.
4. **PDA mission browser.** Extend modal state and reuse pause/input suppression.
   Show name, faction, description, prerequisite/chaos status and giver location;
   exclude collectibles from the normal mission list. Route the requested Start
   action through the same validated launch service. Distinguish selection/waypoint,
   travel and launch. Refresh after chaos awards and completions; derive "Next"
   bars only after original selection/progress semantics are verified.
5. **Runtime verification later.** Verify locked giver absence, correct prompt box,
   intro completion/cancellation, active-mission rejection, modal input consumption,
   destination streaming and one reward per first completion. No such runtime checks
   were executed during this research pass.

Priority unknowns: BMF flag meanings; router loader/opcode behavior; employer
activation/unlock events; briefing skip semantics; original PDA launch/list behavior;
save/retry persistence; employer location discoverability beyond known names.
