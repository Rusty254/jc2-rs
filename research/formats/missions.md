# Missions, strongholds and colonels

Where the mission system's data is, how much of it is decoded, and what jc2-rs runs.
Readers: `crates/jc2-formats/src/missions.rs` (`jc2tool missions <game> [id] [--all]
[--strongholds] [--colonels]`). Game: `crates/jc2-game/src/missions.rs`, `npc.rs`
(colonels, faction soldiers, spawn sets switched by events), `world.rs` (the strongholds'
bases, mission entities), `chaos.rs` (`Progress::alias`). Key names marked ✓ were found
by hashing candidates (`research/README.md`).

The exe's side is small: `NMissionSystem` (`CMissionManager`, `CKeyMission`, `CEmployer`)
reads `Missions\missions.bmf`, `Missions\MissionData.bin`, `rules\MissionRules.brf` (and
`MissionRuleTemplates.brf`, not shipped) and the per-mission folders
(`missions\KeyMissions\`, `missions\FactionMissions\`, `missions\FactionTasks\`,
`missions\Strongholds\`, `missions\Challenges\`); its Lua-facing functions
(`GetObjectiveText`, `GetRemainingStrongholdChaos`, `GetColonelFile`, …) are what the
PDA asks. Everything a mission *does* is data: world objects wired by event strings.

## The mission table: `missions\missions.bmf`

441 records of 60 bytes, little-endian, no header:

```
0x00 char[16]  id           "f2t10\0" + stack garbage
0x10 char[16]  requires     the mission/stronghold done first (km01 names itself)
0x20 u32       hashlittle(id)            (checked on all 441)
0x24 u8[8]     flags        unnamed (01 01 00 00 on km01, 01 00 01 01 on the collectables…)
0x2c u32       objectives   how many objectives (= the rule chain's length: f2t10 2, f1m03 4,
                            the strongholds 5, km01 17, km02 10 of which 9 are chained)
0x30 u32       kind         3 Agency, 4 faction mission, 5 faction task, 6 stronghold,
                            8 skull (csk), 9 black box (cbb), 10 drug drop (cdd), 18 race (cch)
0x34 u32       chaos        unlocked at this chaos
0x38 u32       faction      0 Roaches (f1…), 1 Ular Boys (f2…), 2 Reapers (f3…), 3 Agency
```

7 Agency missions, 11 faction missions, 38 faction tasks, 9 strongholds, 76 races, 300
collectables. The faction field is the faction of the mission's region (the stronghold
it `requires`), who offers it there; the id's `f1`/`f2`/`f3` is only roughly that:
`f2t10` is in a Roaches region (`f1s03`, faction 0, Razak's lines), `f3m04` in an Ular
Boys one (`f2s04`). The faction of each id
prefix comes from the text database: `f1s07_intfile_desc_txt` "The Roaches want…",
`f2s04` the Ular Boys (Sri Irawan), `f3s01` the Reapers (Bolo Santosi).

**How they chain and unlock.** `requires` + `chaos`: `km01 → km02 → f1s07, f2s04, f3s01`
(the first stronghold of each faction, chaos 0) `→ f1s02` (after `f1s07`, 110 000) `→
f1s03` (240 000) …; Agency missions by chaos: `km03` 75 000, `km04` 160 000, `km05`
280 000, `km06`/`km07` 435 000. Faction missions, tasks, races and collectables name a
stronghold as `requires`: they belong to its **region**, and their chaos is one of that
region's thresholds in `missions\regions.csv` (`f1s07_r1;0`, `f1s07_r2;60000`,
`f1s07_r3;300000`; 21 rows, 2–3 per stronghold) — taking a stronghold opens its area of
influence, and chaos opens it further ("Influence has Expanded", `mission_msg_05`).

## What they pay: `missions\missiondata.bin`

```
#60e5608f { 28 unnamed keys = "mission_msg_01…17", "interest_msg_02…11", "challenge_msg_01/02",
            "gui_txt_248/249" }                      the banner texts
#86fb0231 { two nodes of eleven numbers }             not understood
rewards ✓ { max_chaos = 809750,
  key_missions ✓         { money = 20000, chaos = 10000, vehicle_parts ✓ = 3, weapon_parts ✓ = 3,
                           armor_parts ✓ = 0 }
  faction_missions ✓     { 17500, 7500, 1, 1, 0 }
  faction_tasks ✓        { 10000, 5000, 1, 1, 0 }
  stronghold_takeovers ✓ { 17500, 7500, 2, 2, 0 }
  collect_skulls ✓ / collect_black_boxes ✓ / collect_drug_drops ✓ { 2500, 500 }
  settlement_chaos ✓ { light ✓ 2500/500, medium ✓ 5000/1500, heavy ✓ 7500/3000 }
  tutorial_missions ✓ and eight more unnamed nodes: all zero except #314b5cde ($20 000)
}
```

The reference screenshot of "Agency Mission Completed" shows "+3 Vehicle part, +3 Weapon
part" under the banner: `key_missions`. The banner texts: `mission_msg_14` "Agency
Mission Started", `_15` "Faction Mission Started", `_16` "Stronghold Takeover Started",
`_17` "Challenge Started", `_08` "Agency Mission Completed", `_09` "Faction Mission
Completed", `_11` "Stronghold Takeover Completed", `_01` "Mission Completed", `_02`
"Mission Failed", `_03` "New Agency Mission Available", `_04` "New Stronghold Takeover
Available", `_05` "Influence has Expanded"; `interest_msg_08` "Colonel Assassinated".

## The flow: `rules\missionrules.brf`

Its own binary tree (not a property file):

```
node  := u8 groups, per group: u16 type, u16 count, then count × (u32 length, key, item)
item  := type 1: a node;  type 2: u8 tag + value (1 = i32, 2 = f32, 3 = u16 length + string)
```

`rules { rule1D3B78F0 { operation { type = "operation", opcode = "AND",
left { type = "input", var = "f1m03Obj04.COMPLETED" }, right {…} }, outputs { out… {
type = "operation", opcode = "=", var = "f1m03.COMPLETED" } }, name = "f1m03Complete",
prio = 0, inputs = 1 } … }`: 325 rules, every one an AND of inputs setting variables.
Per mission they are a chain:

```
<id>.ACTIVE AND <id>.RESOURCE_LOADED  ⇒ <id>Obj01.ACTIVE
<id>Obj01.COMPLETED                   ⇒ <id>Obj02.ACTIVE      …
<id>ObjNN.COMPLETED                   ⇒ <id>.COMPLETED
```

in any numbering order (`f1s07`: Obj05, 01, 02, 03, 04; `km02`: 02, 08, 01, 03, 07, 04, 09,
10, 05). `objective_chain` follows it. The script and the mission system talk with
events: the script fires `msy.<obj>.complete`; the system announces
`msy.<obj>.activated` (and `.completed`, `.checkpoint` for the restart points);
`msy.mission.fail` fails it, `msy.objective.fakecomplete` flashes "complete" without
advancing.

## The scripts: `missions\<kind>\<id>\<id>.blz` → `<id>.bl.bl`

A PCBB blackboard (`PropertyFile::parse_any`) of world objects, each a root node; nested
objects carry relative `world`s. The classes and keys jc2-rs reads
(`jc2_formats::missions::script_objects`):

| class | in | out | other |
|---|---|---|---|
| `CEventTrigger` | `#63f8bec3` (or `#09cf8b80`) | `#bd811f1d` | `enabled`, `enable_event`, `disable_event` |
| `CTimeTrigger` | start `#9d5ffbb3` (or `#26db8770`), stop `#6be89c08` | `#e349054e` after `time` s | `loop`, `active` |
| `CCountTrigger` | up `#5aeeb914`, down `#885f02a4` | `#43f1b36e` at `max_count` | `text` (the HUD counter's label) |
| `CShapeTrigger` | Rico (`player`) entering / leaving its `CShapeTriggerComponent` (`type` 0 box of `size`, 1 sphere of radius `size[0]`) | `enter_event` / `exit_event` | `vehicle` |
| `CMessageObject` | `#f09abd78` | sets the objective line `#382b5a0c` (`f2t10_obj_txt_01`) | `priority` |
| `CTargetIndicator` | `enable` / `disable` (or `#f09abd78` / `#d0bd211a`) | a world marker `text` (`f2t10_tin_txt_01`) with distance | `type` (13 on colonels) |
| `CLocalGuiMapIcon` | `enable_event` / `disable_event` | a map icon of kind `type` (15: objective) | `#c3116fcb` its objective |
| `CDialogue` | `#bd8853ca` | `#e472e3c3` when the line is done | `cue_name` (a text key: the subtitle, with `{start - end}` cues) |
| `CSpawnSet` + `CExportedEntityComponent` | `enabled`, `enable_event`, within `spawn_distance` | `#f57c8a0e` when spawned | `#f25a8a76` the entity (`factiontaskresources\f2t10\f2t10.gb017b.ee`) |
| `CRestartPoint` | `enable_event` (`msy.<obj>.checkpoint`) | | `#3dad288a` the mission: where it restarts |

Also present and not run: `CMissionStatusRouter` (`#8581b032` a mission id; fires
`#837e3e30` while it is not done, `#885427d2` once it is: the strongholds' `before.enabled`
/ `before.disable`), `CMissionFailWarning`, `CDistanceTrigger`, `CFactionRelationSetter`,
`CHeatControl`, `CMusicTrackGroupSelector`, `CCutsceneObject`, `CTeleport`, `CDayCycle`,
the AI teams (`CTeamComponent`, `STeamAgentInfo`: `#1df63231` health, `weapon_name`,
`#e47c51fc` the doll, `#71e44fd6` the death event, `#78efe1bc` the role).

Event names: several per string, separated by spaces; case differs between writer and
listener (`msy.f1s07Obj04.completed` / `msy.f1s07obj04.completed`), so they compare in
lower case. `t{name({GUID})}::x` and `e{…}::x` are a trigger's or an emitter's own; in an
exported entity they belong to that instance (five silos of the same `.ee` fire five
`e{gb017b.10x10({D820…})}::destroyed`), so jc2-rs scopes them per instance.

The text keys of a mission: `<id>_name_txt`, `<id>_obj_txt_NN`, `<id>_tin_txt_NN`
(markers), `<id>_fail_txt_NN`, `<id>_intfile_desc_txt` / `_sugact_txt` (the PDA),
`<id>_counter_txt_NN`, `<id>_bar_txt` ("Securing Stronghold"), `<id>_title_txt` (a
stronghold's place), the lines `<id>_<speaker>_<m|f>_NN`.

The givers are map icons in `icons_global.bin`: `<id>Emp` with `#c9f36d94` 1 (faction
mission/task, `hud_icon_me_*`), 2 (Agency, `hud_icon_km_brief`), 4 (stronghold,
`hud_icon_sh_t_*`); their locations are `locations\employers\…\<id>emp.nlz`.

### The proof mission: `f2t10` "Offensive Action"

A faction task (Roaches, after `f1s03`, 390 000 chaos): get to the military base at
(5848, 251, 10392), destroy five silos. Restart point (9135, 221, 9875) on tile 49,51.

```
start: f2t10.ACTIVE ∧ RESOURCE_LOADED ⇒ Obj01 → msy.f2t10Obj01.activated
  CTimeTrigger 4.5 s → f2t10.Obj01.tinMicMob.show → objective "Get to the military base",
      marker "Military base" and the objective icon at the base
  CShapeTrigger (250 m box round the marker, on at activation) → f2t10.base.entered
  5 × CSpawnSet (1500 m, enabled) put f2t10.gb017b.ee down → f2t10.destructable.spawned
  CCountTrigger 5 × spawned → f2t10.all.destructables.spawned
  CCountTrigger spawned-all + base.entered (2) → msy.f2t10Obj01.complete
Obj02 → msy.f2t10Obj02.activated
  CDialogue f2t10_razak_m_xx_04 → f2t10.destructables.show → "Destroy the silos";
      every silo's own trigger → its marker ("act_silo_txt_01") and icon
  each silo: CDamageControl (500) → e{gb017b…}::destroyed gsy.activity.type13
      → its CEventTrigger → f2t10.destructable.complete
  CCountTrigger 5 × complete ("f2t10_counter_txt_01") → f2t10.all.destructables.complete
  CDialogue f2t10_razak_m_xx_02 → msy.f2t10Obj02.complete ⇒ f2t10.COMPLETED
reward: faction_tasks ($10 000, 5000 chaos, 1 weapon part, 1 vehicle part)
```

Checked on screen (`jc2 .. --mission f2t10 --exit-after 5.5 --screenshot …`): the banner
"Faction Mission Started / Offensive Action" sits where the reference's "Agency Mission
Started / Welcome to Panau" does (24–37 % down, 47 % of the width, the title at 36 % from
the left), the objective line "Get to the military base" next to the minimap as in the
reference's "Land at the indicated location", and the base's marker as a red chevron on
the screen's edge (it is 3.3 km away). Not drawn: the banner's badge, the grey of the
second line.

## Strongholds

The nine settlements of kind 32 (`map.md`); their `icons_global.bin` entries carry
`#0fba8173` the takeover mission (`f1s07`), `#12252e33` the event their targets fire
(`f1s07.CountUp`, `f1s02.incree`, `f1s03.count.up`), `#483e0726` how many (29 for
`03x26`), `#7a5ce7b9` the faction that takes it (1 Reapers, 2 Roaches, 3 Ular Boys —
`icon_types.bin`'s numbering, also the `CTeamComponent`s'), `#80bdb73e` the completion
event (`f1s07.complete`), `#0d9a95f0` the enter event. `jc2tool missions --strongholds`:

| id | mission | faction | targets | name |
|---|---|---|---|---|
| 03x26 | f1s07 | Roaches | 29 | Pelabuhan Saudagar Harbor |
| 53x13 | f1s02 | Roaches | 33 | Emas Hitam Oil Refinery |
| 37x50 | f1s03 | Roaches | 10 | Lembah Firdaus Compound |
| 49x35 | f2s04 | Ular Boys | 24 | Awan Cendawan Power Plant |
| 18x12 | f2s01 | Ular Boys | 13 | Kampung Bahari Village |
| 26x56 | f2s07 | Ular Boys | 29 | Kem Hutan Supply Depot |
| 33x17 | f3s01 | Reapers | 23 | Gunung Gila Pangkat Facility |
| 11x53 | f3s04 | Reapers | 21 | Kem Gunung Gurun Supply Depot |
| 25x49 | f3s06 | Reapers | 33 | Pemainan Racun Facility |

The base is its own location container, `locations\strongholds\<id>\<id>.base.nlz`
(`.blo` placements, collision, models), which the tile streamer did not load before. Its
targets are the usual destructibles and crates, firing the stronghold's counter
(`f1s07.base`: 3 SAM sites, 3 fuel tanks, 2 cranes, 6 crates = 14); the others are in the
city's `city.harbor.a1.nlz` (15 more for `f1s07`) and tile `19x12` (3 for `f2s01`) — every
total checks out against the files. Its spawn sets are switched by events: the
government's by `f1s07.before.enabled` / `before.disable` (the status router), the
faction's (`CTeamComponent.faction = 2`, Roaches) by `msy.f1s07obj04.completed`, the
takeover mission's last objective. The takeover mission (`missions\strongholds\f1s07`)
is an escort: "Talk to the technician", escort him into and through the harbour, defend
him (`f1s07_obj_txt_01…20`, `stronghold.technician.aiprop`, `stronghold.bofc3x.aiteam`
waves, "Kill the Stronghold Commander").

**What jc2-rs does** (`missions.rs`): the base containers load with the stronghold's
tile (`world::ExtraContainers`); the counters count for the settlement
(`chaos::Progress::alias`), so its banner and the panel under the minimap show "N %
complete" of the data's total; the government's spawn sets are on
(`<id>.before.enabled` fired at the start). With every loaded target down (or the total
reached), at least the takeover mission's unlock chaos, and Rico within 300 m, the
takeover starts ("Stronghold Takeover Started", the heat +40) and he holds it for 60 s
(the objective line "Securing Stronghold 37 s"; leaving resets it); then "Stronghold
Takeover Completed", `stronghold_takeovers`' reward, the icon becomes the faction's
(`hud_icon_sh_roaches_c_dif.dds`) on the map and the minimap, `<id>.before.disable`
takes the government's soldiers away and `msy.<last objective>.completed` brings the
faction's, in its dolls (`enemytypespawnsettings.bin`'s `roaches`/`reapers`/`ularboys`),
friendly (they don't shoot Rico). **Assumed / hand-written:** the 60 s defence and the
heat instead of the technician mission; "all loaded targets" (the Panau City files are
not streamed, so `f1s07` stops at 14 of 29); the 300 m reach; nothing is saved.

## Colonels

Not a file of their own: the 50 colonels (`age01` … `age50` in the text database) are
`CSpawnSet`s named `sps.general` in the location files (40 tiles and the city's
`city.gameplay.*.nlz`, found with a scan of every location container —
`jc2tool missions --colonels`), each with
team `age.generalguards.aiteam`: the colonel (role `general`, `#1df63231` = 500 health,
`weapon_name` 11, doll `missionspecific\pd_ms_gov_shielder.cdoll`, death event
`t{mod.age({…})}::generalKilled`, a `CTargetIndicator` `ageNN_tin_txt_01` "Colonel Rakyat"
shown within a 40 m sphere) and a guard (100 health, `pd_factioninterest_general`). The
event triggers listening to his death fire `gsy.activity.type08 51x10.count.up` (the
activity "Colonel Assassinated": +1000 chaos, $5000; he counts towards the settlement)
and `msy.age40.complete` (`ageNN_msg_01` "Colonel … Assassinated"). The set is switched on
by the location's own logic (a creation trigger, an event router that remembers the
kill) — jc2-rs takes it as on until he is killed. The exported `geo.agegeneral*.ee`
entities are the same thing as a template (`msy.MIDtoReplace.complete`).

jc2-rs: `npc.rs` spawns him with the data's health and doll (an elite's weapon: the
`weapon_name` table is unknown) and his guards as elites; `ColonelTag` marks him; a kill
pays the activity (the CHAOS popup), counts for the settlement, shows the message as a
notification and fires his events (so his set stays off); the loaded tiles' colonels
are on the map and the minimap (`hud_icon_foot_red`: the data has no colonel icon kind),
and his marker shows within 40 m.

## Open

* The flags of `missions.bmf`, `missiondata.bin`'s unnamed nodes (`#86fb0231`,
  `#314b5cde` $20 000) and its message keys.
* The classes not run: AI teams and their waves, cutscenes, the status router in general
  (only the strongholds' is modelled), fail warnings, distance triggers, restart points
  beyond the start, music, heat control, smart objects, the mission's location
  containers (`locations\factiontasks\f3t16\f3t16.shaft.nlz`).
* Starting missions from their givers, the PDA's mission list and "Next …" bars,
  `MissionRuleTemplates.brf`, saving.
* Which `weapon_name` index is which gun; the colonel's escort size beyond the data's
  team; the transmission portraits (`hud_transmission_NN`) for the subtitles.
