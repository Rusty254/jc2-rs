# Chaos, money, settlements and the Black Market

What sabotage is worth, how the game is staged by chaos, how a settlement's completion
is counted and what the Black Market sells. Readers: `crates/jc2-formats/src/chaos.rs`,
`blackmarket.rs`, `destruction.rs` (activities and counters of the destructible units),
`pickups.rs` (resource crates), `heat.rs` (heat levels by chaos). Game:
`crates/jc2-game/src/chaos.rs`, `blackmarket.rs`, `pickups.rs`, `hud.rs`.

Key names marked ✓ were found by hashing candidates (the exe's strings and word
combinations; `research/README.md`). There is no `settings\chaos*.bin`: chaos lives in
`achievements.bin`, `aidifficulty.bin`, `heatsystem.bin`, `blackmarket.bin` and the location
files' event strings.

## How destruction turns into chaos: activities

A destructible object's damage control fires an event string when it goes. Next to the
events that hide the intact model and show the `_dst` debris, the government's property
fires **`gsy.activity.typeNN`** (the exe formats it as `gsy.activity.type%.02d`) and, when it
belongs to a settlement, **`<settlement>.count.up`**:

```
CDamageControl dac.GO098b   #138b2a05 = "gsy.activity.type14 e{go098b.10x10({…})}::destroyed 31x14.count.up"
CDamageControl (gas pump)   #138b2a05 = "e{gb192b.10x10({…})}::destroyed gsy.activity.type11"
CEventTrigger evt.reward    #63f8bec3 (listens) = "e{go043.01.10x10({…})}::destroyed"
                            #bd811f1d (fires)   = "gsy.activity.type19 31x14.count.up"
```

Sometimes the damage control fires them itself, sometimes a `CEventTrigger` listening to
the damage control's own event does (the broadcast towers). `destruction::destructibles`
follows up to three trigger hops and gives each unit its `activity` and `counter`.

### `settings\achievements.bin` → `statistics ✓ { activities ✓ { … } }`

One node per activity type: `type`, `money`, `chaos`, `title ✓` (a text key, or English for
the unlocalised ones), `text` (a message key), `sabotage ✓` (1: a government installation).

| type | title (English) | chaos | money | sabotage |
|---|---|---|---|---|
| 1 | Fuel Depot | 150 | 750 | ✓ |
| 2 | Mobile Radar | 500 | 2500 | ✓ |
| 3 | Baby Panay Statue Destroyed | 1000 | 5000 | ✓ |
| 4 | Communication Station Destroyed | 1500 | 5000 | ✓ |
| 5 | Bio Fuel Shaft Destroyed | 500 | 7500 | ✓ |
| 6 | Propaganda Trailer Destroyed | 1000 | 5000 | ✓ |
| 7 | (`gui_txt_265`) | 500 | 2000 | |
| 8 | Colonel Assassinated | 1000 | 5000 | |
| 9 | Pipeline Destroyed | 1000 | 5000 | ✓ |
| 10 | Satellite Dish | 250 | 1250 | ✓ |
| 11 | **Gas Pump** | 100 | 500 | ✓ |
| 12 | Gas Holder | 500 | 2500 | ✓ |
| 13 | Small Fuel Container (red barrels, gas tubes) | 10 | 50 | |
| 14 | Water Tower | 250 | 1250 | ✓ |
| 15 | Radar | 250 | 1250 | ✓ |
| 16 | Wind Turbine | 500 | 2000 | ✓ |
| 17 | Generator | 250 | 1250 | ✓ |
| 18 | Transformer | 250 | 1250 | ✓ |
| 19 | Broadcast Tower | 250 | 1250 | ✓ |
| 20 | Radio Mast Destroyed | 1000 | 5000 | ✓ |
| 21 | (`act_type_txt_21`) | 0 | 0 | |
| 22 | SAM Site | 500 | 2500 | ✓ |
| 23 | Military Vehicle Destroyed | 50 | 300 | |
| 24–27 | Bomb Expert / Elite / Ninja / Shielder Assassinated | 0 | 0 | |
| 28 | Offshore Rig Destroyed | 1500 | 7500 | ✓ |
| 29 | Industrial Chimney | 500 | 2500 | ✓ |
| 30 | Crane | 500 | 2500 | ✓ |
| 31 | Silo | 500 | 2500 | ✓ |

The reference screenshots agree: gas pump +100/$500, radar and satellite dish
+250/$1250, fuel depot +150/$750, radio mast +1000/$5000, a red barrel +10. The big ones
carry a message (`act_message_txt_01…08`: "Military communications has been disrupted.").
The same file's `#21493d82` (the counts per achievement), `statistics.game_progress ✓`
(totals such as 368 locations, 2700, 3501) and `gui_params ✓` (the HUD statistics popups:
`interval`, `first ✓`, `text`) are not used yet.

## Stages: `settings\aidifficulty.bin` → `chaos_levels ✓`

`chaos_00 … chaos_04 { name, max_chaos }`: 100 000, 200 000, 300 000, 400 000, 99 999 999.
The stage nodes of `enemytypespawnsettings.bin` are keyed by the same names
(`#16d9534e` = `chaos_00`), with `min_chaos`/`max_chaos` bands of 100 000: the soldiers'
classes, health, weapons and elite share follow the chaos (`spawning.md`).

`heatsystem.bin` caps the heat by chaos: `heat_level_chaos_N ✓` (0, 0, 100 000, 200 000,
300 000) and `heat_level_max_heat_N ✓` (20, 40, 60, 80, 100) — at the start the heat can
reach level 2, level 3 needs 100 000 chaos, level 5 300 000 (the exe's
`heat_level_chaos_%d` / `heat_level_max_heat_%d`, read in a loop at `0x005e0880`; the text
database has "A new Heat level has been unlocked", `gen_tut_txt_06`).

## Settlement completion: `<id>.count.up`

Each location file counts its own: every event string that fires `31x14.count.up` is one
thing to do. In `31x14` (Lembah Genting Tinggi) there are 15 — eight resource crates (their
pickup triggers' `enter_event`), a water tower, two generators, two broadcast towers, two SAM
sites; `32x14` has 22, `51x10` 11. The id is the settlement's `id` in `settlements.bin`, so
"N % complete" is `done / total` (`chaos::Progress`). Not everything that pays counts:
gas pumps and red barrels pay chaos without a `count.up`; colonels count.

## Resource crates: `CResourceItem`

```
CResourceItem rei.weaponBoostPickup { type = 1, #f25a8a76 = "exported\pickups\pickup.boost.weapon.ee",
                                      #ec22ee85 = "t{sps.pickup.boost.weapon.crated({…})}::sht.pickup.enter" }
```

`type` 0 armour part, 1 weapon part, 2 vehicle part, 3 cash. The entity is the box
(`pu04-a.lod` for parts, `pu05-a.lod` for cash). What one gives is not in the data: the
exe's handler (`FUN_00854510`) adds **1 part** each and **$2500** (`0x9c4`) for cash, and
every fifth armour part raises Rico's health (`ply.health.upgrade`, `FUN_00751430`).

## The Black Market: `settings\blackmarket.bin`

```
costs ✓ { 0 { level = 0, parts ✓ = 5 } 1 { 10 } 2 { 15 } 3 { 20 } 4 { 25 } }
parameters { set_0 { parameter_set ✓ = 0, parameter_0…3 = "gui_wea_txt_01…04" }   damage, rate of fire, accuracy, magazine
             set_1 { 1, "gui_wea_txt_06…08" }                                      explosive damage, range, quantity
             set_2 { 2, "gui_v_txt_01…04" } }                                      top speed, acceleration, handling, armour
weapons  { weapon_N  { id, type = 0, entity = "heavydrops/mod.heavydrop.revolver.ee", item_id ✓ = "WEA04",
                       chaos = 40000, price = 40000, name = "gui_txt_60", description = "gui_desc_txt_60",
                       parameter_set = 0, parameter_0…3 = [min, max], #6c9e0112 = 0, image ✓ = "",
                       #fba8025d { level_0 = "bm_revolver.dds" } } }
vehicles { vehicle_N { … type = 1, entity = "Vehicles/LAVE/LAVE.V056_ATV.ee", item_id = "v056", … } }
```

| item | price | chaos | | item | price | chaos |
|---|---|---|---|---|---|---|
| Pistol (WEA02) | 20 000 | 0 | | Makoto MZ 260X (v052) | 25 000 | 0 |
| Fragmentation Grenade (WEA33) | 25 000 | 0 | | Pell Silverbolt 6 (v067) | 30 000 | 40 000 |
| Revolver (WEA04) | 40 000 | 40 000 | | MTA Powerrun 77 (v089) | 30 000 | 70 000 |
| Sniper Rifle (WEA14) | 45 000 | 40 000 | | Rowlinson K22 (v059) | 35 000 | 95 000 |
| Assault Rifle (WEA11) | 55 000 | 70 000 | | Shimuzu ATV110 (v056) | 25 000 | 140 000 |
| Triggered Explosive (WEA35) | 30 000 | 95 000 | | Garret Traver-Z (v024) | 40 000 | 195 000 |
| Sawed-Off Shotgun (WEA06) | 35 000 | 140 000 | | Poloma Renegade (v023) | 50 000 | 255 000 |
| Shotgun (WEA13) | 45 000 | 195 000 | | Hedge Wildchild (v022) | 50 000 | 310 000 |
| Submachine Gun (WEA05) | 35 000 | 255 000 | | GV-104 Razorback (v012) | 100 000 | 360 000 |
| Machine Gun (WEA28) | 70 000 | 310 000 | | Sivirkin 15 Havoc (v061) | 100 000 | 415 000 |
| Grenade Launcher (WEA17) | 75 000 | 360 000 | | | | |
| Rocket Launcher (WEA16) | 80 000 | 415 000 | | | | |

The names are from the text database (`gui_txt_59…82`). An item goes on sale once the chaos
reaches its `chaos` (`gui_txt_249` "New Black Market Items Unlocked"; the exe's
`GetChaosUntilNextItem` feeds the "Next Black Market Item" bar). The weapons' heavy drops
(`mod.heavydrop.*.ee`) are a crate with a `CWeapon` (`weaponname`, `pickup_count`), so the
item's gun is read from there (`Handgrenade`, 20; `Triggered Explosive`). Upgrades cost parts
— weapon parts for guns, vehicle parts for vehicles (`db_agent_summary_txt_52/53`) — 5, 10,
15, 20, 25 for the steps 0→1 … 4→5; the pictures are `bm_*.dds` in `pc3` (`level_5` shows
the upgraded vehicle). `#6c9e0112` (0 everywhere, a DLC flag?) and the original's ordering
by beacon and heavy-drop delivery are not modelled.

## What jc2-rs does

* `chaos.rs`: a destroyed unit pays its activity once (`Chaos.total`, `Wallet.money`), counts
  towards its settlement, and shows the CHAOS popup (quick successive awards add up, as in
  "+50" for five barrels); a burnt-out military vehicle (a `heatspawn.bin` unit type or a
  container named `military`) pays type 23. The total sets the stage, the soldiers' stage
  (`EnemyRoster`) and the heat cap.
* `pickups.rs`: the resource crates stand where the tiles put them; walking over one adds to
  the `Wallet` and counts towards the settlement.
* `blackmarket.rs`: `K` opens the shop (paused like the map). Buying: a gun goes into the
  arsenal with a full reserve, grenades fill up, a vehicle is put down beside Rico (the
  original drops it by parachute). Upgrading a gun raises its level in `weapon_upgrades.bin`
  (`weapons.md`); a vehicle's level is kept but changes nothing yet.
* HUD: the popup (the flaming logo `hud_ch_out_en_dif`, the scorpion badge `hud_tin_dif`
  (1, 1), the "$" of `hud_reward_dif`), the next-item bar under it, "N % complete".

## Assumptions and open

* Quick awards merge in the popup for 2.5 s, it stays 4 s (from the screenshots).
* A vehicle bought appears 6 m from Rico; a gun bought comes with a full reserve; buying a
  gun he has with a full reserve is refused (the original sells ammunition that way).
* Missions, strongholds and colonels are in `missions.md`: a stronghold's targets fire its
  own counter (`f1s07.CountUp`, `f1s02.incree`), which `counter_in` returns whole and
  `Progress::alias` maps to the settlement (`03x26`) with the data's total; a colonel's
  death fires activity 8 and counts for his settlement; a finished mission pays
  `missiondata.bin`'s reward. Not in: assassinations other than colonels, Offshore rigs,
  faction and agency progress bars beyond the Black Market's, saving which units were
  destroyed (a reloaded tile rebuilds them, but pays nothing again).
* The armour parts' health upgrade size, the Black Market's vehicle upgrades' effect.
