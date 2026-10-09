# The text database

Every localized string of the game — HUD and menu words, mission titles and briefings,
dialogue subtitles, settlement names, credits, DLC descriptions — is in **one CSV**.
Reader: `crates/jc2-formats/src/text.rs` (`TextDb`); inspect with
`jc2tool text <game> [--lang en] [--key K | --grep substr]`.

## Where it is

`text\ms15text.csv`, ~11 190 rows, in archive `pc4` (5401545 bytes in the unmodded game). A mod
may put its own copy in `dropzone\` (the Definitive Edition mod ships a 6 305 404-byte one),
which then wins; the file system finds either by `hashlittle("text\ms15text.csv")`. Its name comes from
`settings\locator.bin` (`pc0`), a property file that lists the text files per section:

```
#8e3863f7 { #0ed2cd7b / #375faead / #e1ccb9dc = "Text/Escape_sequences.csv"
            #de8abb82 { #ad5c96c2 = "Text/ms15text.csv" } }
```

The other two `text\` files are `escape_sequences.csv` (the glyph escapes, `{EXECUTE}`,
`{LEFT}`, per language: the keyboard / gamepad button names) and `button_icons.csv`
(`"GUI_OK",1128`: button name → icon index, plain CSV). `missions\regions.csv` is not
text (`f1s07_r1;0`: region timings).

DLC packs add `<n>_locator.csv` (`DLC\pc_60\6_locator.csv`, `7_`, `14_`) in the same
format with their own keys (`dlc_name_txt_14` "Chaos Parachute"); languages a DLC does
not translate say `No localization`. `TextDb::load` merges them (keys already present
win). The exe builds the name with `%s_locator.csv` and has an error string
`Missing localization` and a Lua binding `GetLocalizedString` (strings.csv).

## Layout

One row per line (CRLF), every field wrapped in `#D#` marks, the fields joined by a
comma:

```
#D#set_mil_txt_11#D#,#D#Lembah Genting Tinggi#D#,#D#Lembah Genting Tinggi#D#,… ×8,#D#,,,,,,,,,,(spreadsheet padding)\r\n
```

i.e. split a line (without its leading `#D#`) at `#D#,#D#`; the first field is the key,
the next eight the languages, and the last carries a closing `#D#` followed by a run
of commas (an Excel export) which is cut off. No quoting or escaping exists; a text
never contains a line break (one row = one line) or `#D#`. There is no header row.

| column | language |
|---|---|
| 1 | English |
| 2 | French |
| 3 | Italian |
| 4 | German |
| 5 | Spanish |
| 6 | Russian |
| 7 | Polish |
| 8 | Japanese |

(Order read off the data: the Russian column is Cyrillic, the last is Japanese; the
dialogue FMOD banks ship as `eng fre ger ita rus spa`, the same languages.) Encoding is
UTF-8; one English field contains a Windows-1252 ellipsis (`0x85`), which `TextDb`
decodes per field with that fallback. The Steam build's non-English columns are
mostly translated, but a number of rows carry English in every column (names such as
settlements: the Malay names are the same everywhere), and the DLC rows may say
`No localization`. `TextDb` falls back to English for an empty or `No localization`
field.

## Keys

Plain strings, not hashes; the case is the authors' (`km02_cut02B_blaine_m_05`), looked
up here without regard to case. 11 183 unique keys; 8 appear twice (credits, `fe_txt_340`;
the first row is kept). The families (digits as `N`):

| key | what |
|---|---|
| `set_mil_txt_N`, `set_civ_txt_N`, `set_city_txt_N`, `set_demo_txt_N` | settlement names (military 120, civilian 218, the three city districts `set_city_txt_01…04`) |
| `fNsN_title_txt` (`f1s07_title_txt`) | stronghold names (faction 1–3) |
| `fNmN_name_txt`, `fNtN_name_txt`, `fNsN_name_txt`, `cchN_name_txt`, `age_/cbb_/csk_…_name_txt` | mission, take-down and challenge titles |
| `fNmN_obj_txt_N`, `…_tin_txt_N`, `…_fail_txt_N`, `…_intfile_desc_txt` | objectives, in-mission notices, fail messages, intel-file descriptions |
| `gui_txt_N`, `gui_wea_txt_N`, `gui_v_txt_N`, `gui_desc_txt_N`, `fe_txt_N`, `hud_*`, `pc_controls_txt_N` | menus and HUD |
| `statistic_txt_N`, `statistic_hud_txt_N`, `gen_hint_txt_N` | stats, hints |
| `cred_name_N`, `cred_type_N` | credits (649 entries) |
| `dlc_name_txt_N`, `dlc_desc_txt_N`, `dlc_title_txt_N` | DLC items |
| `<speaker>_m_NN`, `combat_*`, `wsim_*`, … (the other ~7 000) | spoken lines with subtitle cues |

Dialogue texts start with cues, `{0 - 2.5}Ah, you come to race lah? {2.7 - 8.1}Then …`:
seconds from the start of the sound, apparently when each subtitle chunk appears.
`text::strip_cues` removes them. `{EXECUTE}`-style braces are button glyphs from
`escape_sequences.csv`.

## What the map uses

Settlement name keys come from `settlements.bin` (`text`, see `map.md`). All 370
settlements on the map resolve (tested). `31x14`, the start base, is
`set_mil_txt_11` = **Lembah Genting Tinggi**; `32x14` is Bukit Rendah; `city.c` is
"Panau City - Financial District". Names are the same in every language except the
stronghold titles and the Japanese column (katakana).

## Weapon and vehicle names

Most of the HUD's names are here: `gui_txt_59…69` = Pistol (59), Revolver (60),
Submachine Gun (61), Sawed-Off Shotgun (62), Grenade Launcher (63), Assault Rifle (64),
Shotgun (65), Sniper Rifle (66), Rocket Launcher (67), Machine Gun (68), Fragmentation
Grenade (69) (`TextDb::weapon_name` maps the engine's short names to them);
`gui_wea_txt_01…08` the stat captions (Damage, Rate of fire, …); `gui_v_txt_01…04` the
vehicle stat captions (Top speed, Acceleration, Handling, Armor);
`fe_txt_140/143` Parachute and Grappling hook. Vehicle names are **not** in this file
(except `v015_military_vehicle` = "Chepachet PVD" and the mounted weapons
`weapon_16/26/28/34` Rockets, Minigun, Machine Gun, Autocannon): the black market's
names are elsewhere (not found; probably in the GUI containers).

## Open

* The Anark GUI containers (`gui\*.blz`) and where the vehicle names live.
* How the engine handles a key missing in a language beyond the DLC's
  `No localization` (the `Missing localization` string suggests it prints that).
* Which of the 8 columns the engine picks (`default_language`, `languages` in
  `settings`): English is the default here.
