# Native Heat contract: phases, incident gates, points and waves

Research only (JC2-091, 9 October 2026). No runtime change, build, game launch or save
write was made. Executable `JustCause2.exe` sha256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`; addresses are this
build's. Reproduce with `research/tools/heat_contract_probe.py` (below). It checks the
fingerprints, 42 pseudo-C anchors, the `.rdata` key tables, the data files and the fixtures
named here. It embeds no game data.

| Input (raw bytes via `jc2tool cat --raw`) | bytes | sha256 |
|---|---|---|
| `settings\heatsystem.bin` | 221 | `49987bcb744e9a36d86822af273c4367e82f066dd0ca620847edcde8c19d31a5` |
| `ai\aiheatparams.bin` | 2829 | `b519de82f8ce261b2279cf9a4b343159cff49e8cdad0a021f1657f2cdb214bc0` |
| `settings\heatspawn.bin` | 14033 | `d037200af7504675c87881b9e6cb47e921efa9965b9b1b3ee61c6f33bb6622e1` |

Tags: **[V]** verified (decompiled code and data agree), **[I]** inferred, **[U]** unresolved.
Reuses the accepted JC2-015 result (`ai-dialogue-next.md`): the first-LOS flag
`LineOfSightSinceHeatReset` is erased by the 0.5 s poll at `0x005790a0` once a faction's heat
record is <= 0. That is the only reset fact repeated here.

## 1. What changes in `heat.rs` assumptions

| Assumption in `heat.rs`/`spawning.md` | Native result |
|---|---|
| 100 points = one level (20 heat) | **Wrong.** Points are accumulator units: every full 100 accumulated points add **+2.5** heat on the 0..100 scale (`0x006e7bd0`). A level is a 20-point heat band, so one level is 8 full events. [V] |
| A kill adds 100 points at once | An event is queued, waits its `Delay` (0.5 s for kills) and is applied on the first update after the delay reaches 0. In the hot table (heat > 0) most events are worth 0; kills (idx 1) and idx 15 stay at 100. [V] |
| Heat holds while seen, after 20 s falls 2/s | Combat holds while any heat agent engages. Out of contact: a 3 s timer and a 20 s timer (the 20 s timer only survives while Rico stays within 30 m of the last-known position). Then Search decays heat **1.25/s**. [V] |
| Reinforcements start at level 1 | `heatspawn.bin` has a `heat_000` band for heat 0 (ambient vehicles). The manager runs in every band, gated only outside restricted areas. [V] |
| Helicopters from level 3 | Ungated MG helicopter wave first appears in band (40,60] = level 3; a helicopter wave gated by a height rule is present from level 1. [V] |
| No unnoticed state | Native "Undetected" exists but is entered by restricted areas or scripted heat, not by an unseen incident (section 4). [V] |

## 2. Heat object (`DAT_01191710`, CHeatSystem)

| Offset | Meaning | Evidence |
|---|---|---|
| `+0x0c` int | restricted-area counter (`enter_restricted_area` / `exit_restricted_area` UI events at 0 to 1 and 1 to 0) | `0x007cb350` [V] |
| `+0x14` | `max_heat` = 100 | `0x0071f220` [V] |
| `+0x18`,`+0x1c` | search-ping interval from 10 s at heat 0 down to 1 s at heat 100 (`#fcaa61dc`=10, `#ad5429bd`=1) | `0x005e2500`, `0x006c8840` [V] |
| `+0x20`,`+0x24` | `#93b4ac49`=100, `#b05e8c7b`=200 (search radius, re-ping distance) | `0x00569f10` [V], meaning [I] |
| `+0x28` | `#51f8f485`=5, the Combat heat floor | `0x006c8840` [V] |
| `+0x2c + f*8` | byte: heat frozen by script; `+0x30 + f*8` float heat for faction `f`, 11 factions | `0x0074b580`, `0x007eb3a0` [V] |
| `+0x84` | state, `+0x88..` the four state objects | `0x00729150` [V] |
| `+0x18c` | unlock index 0..4; `+0x190..` `heat_level_chaos_1..5`; `+0x1a4..` `heat_level_max_heat_1..5` | `0x005e0880`, `0x0078f6b0` [V] |

Heat faction indices (hash table at `0x00ebd538`): 0 government, 1 reapers, 2 roaches, 3 ular_boys,
8 civilian, 9 player; 4,5,6,7,10 are unnamed hashes (`#901ca7bf #000a3c5e #f31a99c3 #e8abe2a9
#84fdcc12`). `heatspawn.bin` faction slots 0..6 use the same order. [V] The numeric heat that drives
states, HUD fraction and music is **faction 0's** record. `GetHeatState` (Lua) returns the state and
`heat / max_heat` (`0x00523080`, `0x0073bdb0`); `GetCurrentMaxHeat` returns
`heat_level_max_heat[idx] / max_heat` (`0x0055cd80`). [V]

## 3. Phase and transition table

States: 0 None, 1 Undetected, 2 Search, 3 Combat (`0x00708b50`; classes `CDefaultState`,
`CUndetectedState`, `CSearchState`, `CCombatState`). `SetState(to)` (`0x00729150`) does nothing when
`to` equals the current state; otherwise it calls the old state's exit, the new state's enter, posts the UI
event `heat_state_changed`, fires the global event string `gsy.heatChange.<from><to>` from a 4x4 table
at `0x0106fe60` (all twelve names verified in the probe), and, if the music director exists
(`DAT_011919b8`), calls `0x007c85b0(from, to, heat)`. "Engaged" below is `*(DAT_0118fd1c+0x78)`:
cleared at the start of every AI update (`0x00489090`) and set (with the last-known position
`+0x6c..+0x74` = the target's position) by an agent's attack action when it has the player as a visible
target and its character byte `+0x728` is set (`0x00698760` to `0x00620c90`). The same action sets
`+0x79`. [V] The byte's name is [I] (`IsHeatAgent` is an interned key near it).

Config values below are `heatsystem.bin`: `undetected_time` 2, `combat_time` 1, `combat_delay` 2,
`#22437136` 15, `#fee0470a` 20, `#95f11b2c` 30, `#ded16638` 1.25. [V]

| From | Condition (checked in this order, once per update, `dt` = frame delta) | To / action |
|---|---|---|
| None (`0x006d9140`) | heat <= 0 and restricted < 1 | stay |
| None | heat > 0 and engaged | Combat |
| None | otherwise (heat > 0 unseen, or restricted >= 1) | Undetected; enter sets timer = 2 |
| Undetected (`0x006ffad0`) | engaged and (heat not frozen or heat > 0) | Combat |
| Undetected | restricted < 1 and no frozen+positive faction: `timer -= dt`; else `timer = 2` | |
| Undetected | `timer <= 0` | `0x006f3770` reset (below) |
| Combat enter (`0x006c8840`) | heat < 5 and not frozen: heat = min(5, cap). Ping timer = lerp(10 to 1 by heat/100). Then refresh | |
| Combat refresh (`0x006c8810`) | every update while engaged, and on every Add-Heat | `T2 = 20`; `T1 = combat_time + (restricted >= 1 ? 15 : combat_delay)` = **3 s** (16 s in a restricted area) |
| Combat (`0x008439f0`,`0x00519390`) | not engaged: `T1 -= dt`; if `T2 > 0`: `T2 -= dt` and, if the player is **not** strictly within 30 m of the last-known position (`dist^2 < 30^2`), `T2 = 0` | |
| Combat | `T1 <= 0` and `T2 <= 0` | Search |
| Search (`0x0061cf30`) | heat not frozen: `heat = clamp(heat - 1.25*dt, 0, cap)` | |
| Search | engaged | Combat |
| Search | heat <= 0 and restricted >= 1 | Undetected |
| Search | heat <= 0 and restricted < 1 | reset to None |
| Reset (`0x006f3770`) | no faction with (frozen byte and heat > 0): zero all unfrozen faction heats, State = None | otherwise nothing |

The Search and Combat ping (`0x00569f10`, every lerp(10 to 1) s) runs a road search with radius 100
(`0x006d1c90`) and re-arms only after the player moved 200 m. What it spawns is [U].

**Add-Heat forces Combat.** `0x006e7bd0` calls `0x0050c3d0` after every Add-Heat; if faction 0 heat
is positive it enters Combat (or refreshes the timers if already there). So a points-driven incident
never produces a quiet Undetected phase; it jumps straight to Combat, floors heat to 5 (level 1) and
needs no witness unless the event itself demands one. [V]

## 4. Unnoticed incident, search and confirmation: what each phase is natively

* **Unnoticed incident [V].** Events whose record has `UseWitness = 1` (shots fired, explosions,
  the "seen" event 2) are dropped when no living candidate witness stands strictly inside `Range`
  (`0x006c0590`). Events with `UseWitness = 0` apply with no witness (a kill, a stolen car, soldier
  proximity). There is no separate stored "unnoticed" incident phase.
* **Pre-Heat-like phase [V/I].** The only native Undetected (state 1) entries are a restricted-area
  count > 0 (CHeatControl events from location data) and heat that is positive but unengaged
  (script-set). Its fraction/timer is the yellow-warning candidate (section 9). Leaving the area runs
  the 2 s timer then resets.
* **Confirmation [V].** Confirmation is the engaged flag: an agent attack action with the player
  visible. In Combat it refreshes T1/T2; in Undetected/None/Search it moves to Combat and lifts
  heat 0 to 5.
* **Search [V].** Combat that lost contact for the timers above. Heat falls 1.25/s; engaged again
  returns to Combat; heat 0 resets.

## 5. Points to level

1. An incident event has `Points` (soldier credit) and `CivilianPoints` (civilian credit), table
   chosen at **apply** time by `faction-0 heat > 0` (cold `#7f245b02`, hot `#8c4886f1`;
   `0x00564930`, `0x006e7bd0`). [V]
2. `0x006e7bd0` per update: expired entries add to an accumulator `A[kind]` (kind 7 ignored; kind 8 civilian
   adds `CivilianPoints` to `A[0]`; any other kind adds `Points` to `A[kind]`). Then for each of 11 factions:
   if `A >= 100`, Add-Heat(`2.5 * floor(A/100)`), call `0x0050c3d0`, `A = 0`; then
   `A = max(A - 2*dt, 0)`. `2` and `2.5` are `Parameters` `#14e6141a` and `#1e0e90e0`. [V]
3. Add-Heat (`0x007eb3a0`) is skipped for frozen factions and clamps to
   `[0, heat_level_max_heat[unlock idx]]`. [V]
4. Level = index of the band with `min < heat <= max` (`0x00681bc0`) in `heatspawn.bin`'s government
   bands: heat 0 is `heat_000` (index 0); (0,20] 1; (20,40] 2; (40,60] 3; (60,80] 4; (80,100] 5.
   `0x0071d060` fires `heat.level.increased` and `heat.level.<index>` when the band index rises. [V]
   Whether the HUD digit is the same index is [I]; it reads the same band data.
5. Consequences [V]: one soldier kill from cold = 100 points, +2.5, Combat floors it to **5**. A
   second kill in the hot table is 100 again, +2.5. Level 2 needs heat above 20: seven more
   events (5 + 7 x 2.5 = 22.5). Two civilian kills (50 each) never reach 100 (49.5 then 97.5 under decay and the
   one-pending-per-kind cap), so they alone give no heat (fixture `stale_incident`).
6. The per-kind cap: at most `Parameters #5fca5910` = **1** delayed entry per witness-faction kind at a
   time (`0x0070a330`). [V]

Extra faction-0 accumulator input at handler `+0x68` exists; its writer was not located [U].

## 6. Chaos cap boundaries

`heat_level_chaos_1..5` = 0, 0, 100000, 200000, 300000; `heat_level_max_heat_1..5` = 20, 40, 60, 80,
100. `0x0078f6b0` (called each frame from `0x00548080`) moves the unlock index up by **one per call** while
threshold is reached at `chaos >= threshold`. Initial index 0 gives cap 20 for the first update only; at chaos 0 the
steady state is index 1, cap **40** (level 2). [V]

| chaos | cap | top level |
|---|---|---|
| 0 to 99,999 | 40 | 2 |
| 100,000 to 199,999 | 60 | 3 |
| 200,000 to 299,999 | 80 | 4 |
| >= 300,000 | 100 | 5 |

Statistic slot 4 is the chaos value: [I] (same slot feeds the spawn call and matches
`chaos.md`). The cap is applied at write time (Add-Heat, search decay, Combat floor, script
set), so it never lowers an existing heat. The text id `gui_txt_250` is loaded by `0x0071f220` [V]; that it
is the unlock toast shown via `gui_heat_level_changed` (`0x00548080`) is [I].

## 7. Timer and reset ownership

| Timer / state | Owner | Value | Reset by |
|---|---|---|---|
| Event delay | pending/delayed entry, handler `+0x3c` | record `Delay` (0.5, 1, 2) | consumed on application |
| Accumulator `A[f]` | handler `+0x48+4f` | 0..100, decay 2/s | zeroed on crossing 100; not cleared by heat reset [V] |
| Undetected timer | state object `+8` | 2 s | enter, restricted area, frozen faction |
| Combat T1/T2 | state object `+0x10/+0x14` | 3 or 16 s / 20 s | refresh (engaged or Add-Heat) |
| Search ping | state/object `+8`, Combat `+0x24` | 10 s to 1 s by heat | each ping |
| Search decay | Search state | 1.25 heat/s | leaving Search |
| Engaged flag, last-known pos | `DAT_0118fd1c +0x78`, `+0x6c` | per AI update | cleared by `0x00489090` each update; position persists |
| First-LOS flag | blackboard (JC2-015) | until heat <= 0 poll | `0x005790a0` every 0.5 s |
| Reinforcement waves | manager `0x0071d060` | section 11 | band change |

Reset (`0x006f3770`) is the only path that returns to None. It runs from Undetected expiry, Search at heat 0, a
script event and `0x007cb350` (CHeatControl). It does not cancel delayed entries or accumulators. [V]

## 8. Last-known position and witness rules

* Last-known position: the position of the target seen by the engaging agent, written only at engagement
  frames; consumed by the 30 m rule in Combat. No decay or ageing. [V]
* Witness for `UseWitness = 1` events (`0x0054d1a0` to `0x006c0590`): candidate must have health > 0
  (`+0xe8`), `dist^2 < Range^2` (strict), and for event 1 must not be in relation 1 to the event faction
  (`0x0071f6d0` 12x12 matrix, source [U]). The credited faction is the **witness's** faction. At
  most two witnessed events are processed per update; the rest wait a frame. [V]
* No raycast or occlusion test appears in this path, so a visible and an occluded soldier at the same
  distance are indistinguishable here. Any line-of-sight dependence lives in the producers (event 2 is posted by
  the sight code `0x007d6f80`) and in the engaged flag. Fixture `witness_visible_vs_occluded` records the
  distance-only behaviour, and that gap is the [U] to keep in the runtime. [V/U]
* `UseWitness = 0`: no witness; the credited faction is the poster's `kind` (victim faction for a kill,
  the driver's for a stolen car). Event 4 is only accepted with `kind == 9` (player). [V]
* `7608d0` proximity generator posts directly with kind 0 (section 10).

## 9. Yellow warning and search music, separate from numeric heat

Numeric heat is faction-0's float. The presentation is driven by **state**, not by heat level:

* UI: `heat_state_changed` on every SetState, `enter_restricted_area`/`exit_restricted_area` on the
  counter edge, `gui_heat_level_changed` (`0x00548080`) on level toasts, Lua `GetHeatState` =
  (state, heat/max_heat). Mapping a state to yellow/red HUD colours is in the Anark UI, not decoded: **[U]**.
  Natively the only quiet pre-confirmation inputs are state 1 and the restricted-area edge, so the
  yellow warning is [I] state 1 (Undetected) / restricted area. The HUD textures (`hud_heat_*`) exist but
  were not mapped.
* Music director (`0x007c85b0`, field `+0x78` = current heat music state): to None stops heat music; to
  Undetected resets it and stores state 1; to Combat from None or Undetected posts `mus.heatdialogue.play` and
  posts the heat-dialogue event only if `*(heat+0xc) < 1` (not in a restricted area) and `0x007430c0` is false (and starts a
  cue when the director has one); to Combat from Search restarts the director with the heat; to Search only stores state 2. The track assignment for state 2 is
  not in this function, **[U]**. [V] for the transition rules, [U] for cue/asset IDs.
* `gsy.heatChange.*` strings are global events consumed by the sound system; their listeners are not
  decoded.

## 10. Stimulus and producer matrix

Event index follows the load order in `0x0058b4d0`; values from the data (cold = faction-0 heat <= 0,
fields Points / CivilianPoints / Range / Delay / UseWitness).

| idx | key | cold | hot | civilian sound |
|---|---|---|---|---|
| 0 | `#4075c8b0` | 100/25/30/2/0 | 0/0/0/2/0 | |
| 1 | `#0d038313` | 100/50/40/0.5/0 | 100/100/100/0.5/0 | CIVILIAN_NPC_KILLED_BY_PLAYER |
| 2 | `#965039e1` | 100/0/0/0.5/1 | 0/0/0/0.5/0 | |
| 3 | `#50e6844a` | 100/50/50/0.5/1 | 0/0/50/0.5/1 | CIVILIAN_EXPLOSION_TRIGGERED_BY_PLAYER |
| 4 | `#c433a5fd` | 100/30/70/0.5/1 | 0/0/70/0.5/1 | CIVILIAN_SHOTS_FIRED |
| 5 | `#355df81d` | 100/0/0/0.5/0 | 0/0/0/0.5/0 | CIVILIAN_PLAYER_ON_VEHICLE |
| 6 | `#321deed6` | 25/0/0/0.5/0 | 0/0/0/0.5/0 | CIVILIAN_VEHICLE_HIT_BY_VEHICLE |
| 7 | `#3eb61980` | 100/20/0/0.5/0 | 0/0/0/0.5/0 | |
| 8 | `#fb08a449` | 100/25/0/0.5/0 | 0/0/0/0.5/0 | CIVILIAN_STOLE_MY_VEHICLE |
| 9 | `#5aaf7c23` | 100/20/0/0.5/0 | 0/0/0/0.5/0 | |
| 10 | `#268cf8ce` | 100/50/0/0.5/0 | 0/0/0/0.5/0 | |
| 11 | `#2c6eafca` | 100/50/0/0.5/0 | 0/0/0/0.5/0 | |
| 12 | `#5afe914f` | 100/50/0/0.5/0 | 0/0/0/0.5/0 | |
| 13 | `#d249e946` | 100/25/0/0.5/0 | 0/0/0/0.5/0 | |
| 14 | `#1463c544` | 100/0/0/0.5/0 | absent (defaults 0/0/0/0.05/1) | |
| 15 | `#96a83491` | 100/100/0/1/0 | 100/100/0/1/0 | |
| 16 | `#b2d2f1ed` | 100/0/0/1/0 | 0/0/0/1/0 | |
| 17 | `#55b0626f` | 100/0/0/1/0 | absent | |

Record defaults when a field is absent: Points 0, CivilianPoints 0, Range 0, Delay 0.05, UseWitness 1
(`0x00561ce0`). `HeatSoundEvent`/`#d5c5cdfb` are empty in every record. [V]

Producer sites (every row is checked by literal call-site fragments in the probe):

| Stimulus (JC2-043 list) | Native producer | Status |
|---|---|---|
| Kill a soldier / civilian | idx 1, `0x0056d940` damage handler: attacker faction 9 (player), victim killed this hit; posted at the victim's position with the victim's faction | [V] producer; faction 9 = player [I]. Soldier = 100 pts, civilian = 50 cold, both credit `A[0]`; hot table 100/100 |
| Weapon discharge near soldiers | idx 4 via `0x005dbbd0` fire input (rate limit 1000 ms per character, `+0xa08`), accepted only for kind 9; witness within 70 m, cold only | [V] |
| Explosion | idx 3: `0x00566480` (cold only, `0x0078c000` false) and `0x007e3bc0` (kind 9 callback); witness within 50 m | [V] |
| Drive a military vehicle | idx 5, `0x00719cf0`: one latch per vehicle while it is the player's vehicle; credit = vehicle's faction (0 = 100 pts, civilian = 0) | [V] mechanism; which vehicles are faction 0 [U] |
| Steal a vehicle | idx 8, seven sites (`0x004fb750 0x005bd180 0x005fb010 0x00612ec0 0x00765680 0x0078f9d0 0x008002d0`); credit = driver's faction (soldier 100, civilian 25) | [V] sites; per-site trigger [U] |
| Collision with an occupied vehicle | idx 6, `0x0078ae80` (cone test on `Parameters` 73 and 120 deg); 25 pts | [V] site, trigger shape [I] |
| Hurt a non-heat-agent NPC | idx 7, `0x0058c8d0` damage hook, character byte `+0x728` clear | [V] site, name [I] |
| Object damaged while alive | idx 12, `0x0051a3f0` | [V] site, class [U] |
| Soldier proximity | idx 14, `0x007608d0`: every 1.0 s, cold only, a faction-0 agent within **15 m** of the player and player state `+0x49c < 4` | [V] |
| "Seen" by a soldier | idx 2, `0x007d6f80` sight code (tolerance radius 5 m, from `Parameters`) | [V] site; semantics [I] |
| Weapon event 9 and 0 | `0x00736a50` posts 9 then 0 at its own position | [V] site, [U] meaning |
| Restricted-area intrusion | CHeatControl events enter/exit in `0x007cb350`; counter drives Undetected | [V] mechanism; which areas [U] |
| Military property destruction | no heat producer found; chaos activity only (`chaos.md`) | **[U]** |
| Propaganda truck proximity | no native proximity producer found | **[U]** |
| Visible faction association | no producer found | **[U]** |
| idx 10, 11, 13, 15, 16, 17 | `0x00664100`, `0x005d1e70`, none found, none found, `0x00692b50`, `0x00704e50` | [V] sites, meanings [U] |

## 11. Authored wave selection (`heatspawn.bin`, manager `0x0071d060`)

Chain at government heat 45:

1. Heat float to band: `min < 45 <= max` selects `heat_02` (40,60], index 3 (`0x00681bc0`). [V]
2. Manager update (every frame while restricted-area count < 1; if a wave is mid-spawn, `0x0066f130` runs
   instead): factions processed are 0 and one random of 1..3 (rerolled when the timer at manager `+0xec8` expires,
3. Waves of `heat_02` (wave keys `wave0 wave3 wave1 wave2`; hashes `c47cf8bd` = wave2, `85eae4b5` = wave3):
   * wave0: initial 0, min/max 1/2, respawn 6: `ut_ATVPolice` x2 (template Police), `ut_jeepsmall` x2
   * wave3: initial 8, respawn 60: `ut_militaryhelicopter_mg` x1 (template Armed), ungated
   * wave1: initial 0, respawn 4: `ut_attackboat` x2
   * wave2: initial 8, respawn 60, **gated**: `ut_militaryhelicopter_mg_rocket_air` x1 (template Dome)
4. Wave gate: waves with `#64747c9e = 1` run only while `0x0054dd30` is true: two character predicates
   (`0x007b88b0`, `0x0064daa0`, both [U]) and the height of the object at `DAT_011919f4+0x10` above the
   terrain at its x,z exceeds the root value `#e541c5f8` = 20. [V] code, [U] predicates.
5. Unit choice (`0x007092c0`): every unit with `alive < max_units` has weight 1, all others 0; roll =
   `rand15 * 3.051851e-05` over a per-call LCG (`seed*0x343fd+0x269ec3`, bits 16..30); cumulative subtraction
   of `1/eligible`. A roll of exactly 0.0 takes unit 0 even if it is full (1 in 32768). **There are no authored
   weights** in this table; the unit's `#edf23733` float (absent in the data, so 0) is a per-slot cooldown. [V]
6. After the unit pick, `0x008464b0` starts the spawn: it searches a place (`0x0080b1d0`), snapshots the
   root distance block and starts the step machine in `0x0066f130` (cases 0..3, state at manager `+0x10dc`); for
   unit type value 2 it fires `heat.helicopter.spawn`. `0x007092c0` picks one of two palettes with a random draw
   against a table indexed by the player's state [I]. Type values: land 0, sea 1 (needs the extra predicate
   `0x00795f90`), air 2 [I]. [V] for the call structure.

All government bands (units x max, templates): heat_000 mc x1 + jeepsmall x1, attackboat x1;
heat_00 mc x2 + jeepsmall x1, attackboat x2, gated MG heli x1; heat_01 mc x2 + jeepsmall x2, attackboat x2,
gated MG heli x2; heat_02 above; heat_03 (60,80] ATVPolice x2 + militaryjeep x2, rocket heli x1,
militaryboat x2, gated (MG heli x1 + rocket heli x1); heat_04 (80,100] ATVPolice x3 + militaryvehicle x2,
rocket heli x1, militaryboat x2, gated rocket heli x2. Other factions have a single band (-1,30] with one
wave. `max_units` is the count per unit; the wave total is the sum (`wave+0x18`). Spawning stops for a wave
when `alive == total`. [V]

Delays: `initial_delay` is a real countdown before the first spawn [V]. `min_spawn_delay`, `max_spawn_delay`,
`respawn_delay` and `#19f1aa22` (30/25/600) are stored at wave `+4/+8/+0x10/+0xc` but no consumer was
found in `0x0071d060`, `0x007092c0` or `0x0066f130`: **[U]**, so do not reuse `heat.rs`'s fixed 6 s.

Assets (archive key = `hashlittle(lower-case basename)`, container from `jc2tool list`):

| unit type | seats | entity | key | container |
|---|---|---|---|---|
| `ut_mc` | 1 | `lave.v007_military.ee` | `7736de5b` | pc1 |
| `ut_jeepsmall` | 2 | `LAVE.V047_Jeep_Small.ee` | `3c4ae213` | pc1 |
| `ut_ATVPolice` | 1 | `LAVE.V056_ATV.ee` | `9d727d8c` | pc1 |
| `ut_militaryjeep` | 4 | `lave.v017_military_jeep.ee` | `10d24e35` | pc1 |
| `ut_militaryvehicle` | 2 | `LAVE.V015_Military_Vehicle.ee` | `f97e65c1` | pc1 |
| `ut_attackboat` | 2 | `seve.v104_attack_boat.ee` | `81eb3897` | pc1 |
| `ut_militaryboat` | 3 | `SEVE.V105_Military_Boat.ee` | `342451b6` | pc1 |
| `ut_militaryhelicopter_mg`, `_mg_air`, `_mg_rocket`, `_mg_rocket_air` | 2 | `arve.v009_military_helicopter.ee` | `64d43b9e` | pc0 |

The four helicopter types share one entity; they differ only in the `unit_template` string
(Armed, Dome) and the gate. Template semantics (Police, Armed, Dome) are read by `0x007efc60` into the unit
palette record and not further traced: **[U]**. Crew: `num_seats` is data; no crew-count or
elite-crew rule exists in this path, and no elite or heavy rule tied to heat bands was proven. Level 4/5
"heavy" units in the data are `ut_militaryjeep` (4 seats) and `ut_militaryvehicle`; the user-cited display
names (Chepachet PVD, Wilforce) have **no evidence** here. Elite share stays a chaos-stage matter
(`enemytypespawnsettings.bin`). Root distance blocks `#4b6a6a25` (250/300/600, `spawn_speed` 30) and `#8e171ef7`
(450/550/600, `spawn_speed` 20) are copied at `0x008464b0`; which applies is `manager+0xd4` [U].

Resource ownership: the manager owns the wave slots (`manager+0x1b4 + faction*300`) and the single
in-progress spawn (`+0x10ec`, `+0x1088..`); a wave's alive count is `wave+0x34`. Despawn functors
(`manager+0x1180/0x1184`) release slots. Crew creation and lifetime past that are [U].

## 12. Deterministic fixtures (all pass in the probe)

| Fixture | Asserted result |
|---|---|
| `points_to_level` | heat 0, 0.001, 20, 20.001, 40, 60, 80.5, 100 give level 0,1,1,2,2,3,5,5 (bounds (min, max]) |
| `chaos_caps` | 0/99999 to 40, 100000/199999 to 60, 200000/299999 to 80, >=300000 to 100 |
| `levels_1_to_5` | caps 20,40,60,80,100 are levels 1..5; start-of-game cap is level 2 |
| `first_update_cap` | cap 20 before the first unlock update, 40 after |
| `hundred_points` | `A=100` gives +2.5, Combat, floored to 5 |
| `chain_kill_soldier` (primary) | post at t=0; applied t=0.75 (0.5 s delay then next update); Combat, heat 5; sight until t=1.0; Search at **21.0 s** (T1 3 s done at 4.0, T2 20 s with Rico 10 m from the last-known position); heat 5 falls at 1.25/s to 0 at **25.0 s**; reset to None |
| `timer_boundary_far` | Rico exactly 30.0 m away: `T2=0`, Search after 6 half-second ticks (T1 = 3.0 exactly) |
| `timer_boundary_near` | 29.999 m: Search after 40 ticks (T2 20 s) |
| `undetected_enter` / `undetected_expiry` | restricted area holds timer at 2; leaving it expires in 4 half-second ticks, reset |
| `restricted_area_confirm` | Undetected, then an engaged tick: Combat, heat 0 to 5, `T1` = 16 s, last-known stored |
| `repeated_incident` | second kill in the hot table: 5 to 7.5 and timers refreshed |
| `stale_incident` | civilian 50: A=49.5 after the first; second inside a second: 97.5 and no heat; a lone 50 is 0 after 25 s |
| `witness_visible_vs_occluded` | 69.99 m accepted; 70.0 m rejected; dead witness rejected; non-player shooter rejected; no witness rejected (distance-only; no occlusion test exists on this path) |
| `pending_cap` | second entry of the same kind rejected, other kind accepted |
| `reset_to_none` | after the Search decay heat 0, state None, log ends with reset |
| `wave_band`, `wave_contents`, `wave_unit_selection`, `selection_exhaustion`, `selection_quirk_roll_zero` | band 3 for heat 45; the four waves; four LCG draws from the arbitrary seed 1 give ATV, jeep, ATV, jeep and end at 2+2 alive; full units weight 0; roll 0.0 quirk |

The model's tick order is an assumption: delayed entries and accumulators first, then the state
machine. Native order between `0x0054d1a0`, the state object and the AI update is [U]; fixtures avoid
boundaries that depend on it (the chain keeps events and sight on separate ticks).

## 13. Unresolved, listed once

1. Meanings of events 0, 2, 7, 9..13, 15..17 beyond their producer sites; what IDs 4..7,10 factions are;
   source of the 12x12 relation matrix; handler `+0x68` writer.
2. Property destruction, propaganda proximity and visible-association triggers: no native producer found.
3. UI state to yellow/red mapping; stealth/search music asset IDs and the Search-state cue; listeners of
   `gsy.heatChange.*`.
4. Search/Combat ping spawn (`0x00569f10` and `0x006d1c90`), wave delay fields beyond `initial_delay`,
   spawn place and palette rules, crew creation, templates, `0x0054dd30` predicates, `manager+0xd4`.
5. Native update order between handler and state objects; chaos statistic slot 4 identity (inferred).

## 14. Bounded runtime handoff for JC2-042/043/044

JC2-042 (state, warning, music): replace the 20 s/2 per s model with the section 3 machine,
including T1 3 s / T2 20 s / 30 m last-known rule, Search decay 1.25/s, Undetected 2 s with the
restricted-area hold, and the `gsy.heatChange.*` event stream. Keep yellow HUD colour and the search
cue behind explicit "unresolved" stubs. Do not invent a pre-confirmation state for points-driven incidents:
Add-Heat enters Combat.

JC2-043 (stimuli): one shared posting service with the 18-record cold/hot tables, delay queue, per-kind cap
of 1, witness rule (strict range, health, faction, distance only), and the accumulator
(100 points = +2.5, decay 2/s). Enable only [V] producers: kills, shots (1000 ms rate limit, 70 m), explosions
(50 m), driving a faction-0 vehicle, stolen vehicle, soldier proximity (15 m, 1 s, cold only), restricted area.
Leave property destruction, propaganda and visible association disabled with diagnostics.

JC2-044 (waves): bands by `min < heat <= max`, per-unit `max_units`, uniform pick among eligible units,
heat_000 ambient waves, gated waves behind the height rule, the table in section 11, assets above. Leave
delay fields other than `initial_delay`, crew templates and elite/heavy replacement behind explicit
unsupported diagnostics until the [U] items close. Preserve road pursuit and dismount work in `heat.rs`
only as a documented non-original fallback.

## 15. Reproduce

```powershell
# private copy of the prebuilt tool, never inside the repo
Copy-Item .\jc2-rs\target\release\jc2tool.exe $env:TEMP\heat91\jc2tool.exe
python -B jc2-rs/research/tools/heat_contract_probe.py . `
  --tool $env:TEMP\heat91\jc2tool.exe --analysis jc2-rs/research/analysis
```

The probe exits non-zero on any fingerprint, anchor, table or fixture mismatch. Without
`--analysis` it skips the pseudo-C anchors (the export is git-ignored). Keep decompiler dumps and
game bytes out of commits; every fragment quoted above is a short function-local anchor.
