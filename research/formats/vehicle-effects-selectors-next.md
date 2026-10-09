# Vehicle chassis, effects and siren sound selectors (JC2-097)

9 October 2026. Research only: no Cargo, game launch, capture, mixer or runtime edit.
Extends `vehicle-engine-next.md` (JC2-005 / JC2-027); the engine assignments there are the
baseline and are not re-investigated. Reproduce with
`research/tools/vehicle_effects_selector_probe.py` (commands at the end).

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B, unmodded) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `sound/settings_smod_sfx_vehicle.bin` (`jc2tool cat`, zlib removed) | `6a11f8bbd9b857f534a0e9b479395241018619789804658c0a7a58ebc4ca4b48` |
| `lave.v014_jeep.ee/lave.v014_jeep.ee.epe` | `ef1076ef3588175ec6ce92a8299429b26393f3212ed4b87e8f63bf06916baa03` |
| `lave.v047_jeep_small.ee/...epe` | `6e74b2ce4cd464175e671d99ff9e80bc00cd7a026c481f2f7038e7f552088c6c` |
| `lave.v024_sportscar.ee/...epe` | `71c270aaac99b92d356de0b1f4fc650ab38087f8200014b8b8cceba23a6f6b79` |
| `seve.v082_motorboat_standard_small.ee/...epe` | `b6e8b869815464b92954d7e57eb7a71a76572f12a46d04523ddd6f5a3989d714` |
| `arve.v059_civilian_helicopter.ee/...epe` | `990107f74702b03c8d65e3bf2fc2409e07eea681b4efa59c8c17cd3af1eafdaa` |
| `arve.v010_personal_propellerplane.ee/...epe` | `f315b88b090c46c5aa845c68addae100d1675925d1b17a7b7b645b10a9853fbd` |
| `f1m02.nlz/f1m02.nl.blo` | `06878dafed610787060080f79e4b989fbbad592cb21ef1d56de879972976936e` |
| all 90 vehicle `.epe` + smod + f1m02 blo | digest of the sorted `hash path` lines: `9d2d37c4afabebb2dfe14d30aa5e3d683b7365522d10d8bea0f10831880fcee2` over 92 inputs (`--fingerprint`) |

## Verified: the property names

`jc2tool hash`-style Jenkins hashes of these strings match the retail keys exactly
(exact matches of a hash, not guesses). They replace the "authoring names unknown"
gap in `vehicle-engine-next.md` for everything except the three module keys.

| Key | Name | Type / default | Read by |
|---|---|---|---|
| `0x9f996ed1` | `sound_preset_engine` | int | pair 0 preset, 0x00780ab0 |
| `0x78bda2b6` | `sound_preset_chassis` | int | pair 1 preset |
| `0x834b3da2` | `sound_preset_siren` | int | **pair 2 preset AND the siren loader (same key)** |
| `0xbe5386e2` | `sound_preset_fx` | int | land (0x007557e0, default **-1**) and air (0x007cd740, default **0**) |
| `0x3da4075f` | `has_sirens` | int/bool | CCar loader 0x006913f0 -> byte at object+0xd9b |
| `0xd0f50183`, `0x1894a3e8`, `0x0c956561` | module keys of pairs 0/1/2 | int, default -1 | no hash-recovered name |

Descriptor keys (static initialisers at 0x00c8bxxx store `hash("string")` into BSS;
the strings are in the exe): `uid` 0x6a9dfd76 (module id / secondary-sound id),
`soundbank_id` 0x260a4de6, `presets`, `primary_sound`, `secondary_sounds` 0x22496f7d,
`key_on`/`sustain`/`key_off`, `sound_id` 0x8e51e9e6, `min_duration_ms`, `is_loop`
0xb7c1b604 (item flag bit 1), `key_off_on_stop` 0x316ff946 (flag bit 2),
`var_id_cursor` 0x8780020d (byte at item+5, default -1), `min_audibilliy_rank_play`
(sic) 0xd6938c66 (byte at item+6; item value, else the component's, else 0),
`trigger_on_speed` 0x685b4d34 (desc+0x24, flag desc+0x20), `traffic_type_id` 0x65bb42c0.
`secondary_sound_00/01/02` (0xb2fe1655, 0x08abfd8c, 0x6b5ec683) are only child-node
labels: the id is the child's `uid`, so label and name need not agree (module 4:
`_00`=horn, `_01`=door_open; module 5: `_00`=door_open, `_01`=door_close).

## Verified: the complete chain (loader -> bank/role)

All loaders are vtable slot 6 of the secondary vtable (the same slot 0x00780ab0 occupies
in `CVehicle`); classes were identified by finding each address in `classes.csv` vtables.

| Step | Native | Verified behaviour |
|---|---|---|
| 1 base | `CVehicle` 0x00780ab0 | For pair i in 0..2: module = int(key) default -1; if >= 0, `entries[module].module = module` (vector bound-checked), then preset = int(key); if >= 0, `entries[module].preset = preset`. Slot index == module id. |
| 3 component | 0x00824810 -> 0x009df900 -> ctor 0x009e8d90 | module indexes the descriptor vector (+0x7c); an id with no descriptor yields an empty component (silent). Preset stored at impl+0x10, **not validated here**. |
| 4c sea | `CSeaVehicle` 0x00520db0 (`CBoat` thunk 0x0052d260) | Only 0x00780ab0. No fx/siren install; module 6 `seve_effects` has no roles and no secondary sounds. |
| 5 descriptors | 0x00a055a0 / 0x009df770 / 0x009caed0 | Per `component_list` entry (key `uid`): `primary_sound` children `key_on`=role 0, `sustain`=1, `key_off`=2; `secondary_sounds` children indexed by their own `uid`. Count = number of records in the component's `presets` node (cap 100); entry NN is read as `preset_%02d` from the **role/item's** `presets`, either a plain int sound id or a node with `sound_id`/`min_duration_ms`. A role/item with `soundbank_id` absent or < 0 loads nothing (silent). |
| 6 runtime | 0x009e84e0 | Entry at impl+0x10 of the item's vector; index >= size or sound id < 0 => silent; else callback(bank byte, sound id). |

Worked example, `lave.v014_jeep.ee` (probe output): pair 0 (0,5) -> slot 0 `lave_engine`
preset `SUV`; pair 1 (1,2) -> slot 1 `lave_chassis` `wind_and_chassi_noise`; `sound_preset_fx`=0
-> slot 4 `lave_effects` `big_vehicle`; `has_sirens`=0 -> no slot 7.
Chassis: `key_on` has no `soundbank_id` (no sound); `sustain` bank 4 sound 2
(`Vehicles/Chassis/Chassi Wind & Noise`); `trigger_on_speed`=5.0 on the module.
Effects: secondary uid0 horn bank 6 sound 12 (`Horn big`, `is_loop`, `key_off_on_stop`,
`var_id_cursor` 2), uid1 door_open 6:1 (`Door Big Open`), uid2 door_close 6:2.

### Module / role tables (smod, all numeric)

| uid | module | presets (n) | roles |
|---|---|---|---|
| 1 | lave_chassis | 3: 0 wind, 1 chassi_noise, 2 wind_and_chassi_noise | sustain bank 4 sounds 0/1/2; key_on and key_off silent |
| 4 | lave_effects | 6: 0 big_vehicle, 1 small_vehicle, 2 truck, 3 motorcycle, 4 medium_vehicle, 5 truck2 | secondary uid0 horn (bank 6; sounds 12,13,14,13,0,15 for presets 0..5), uid1 door_open (1,3,1,-1,1,1), uid2 door_close (2,4,2,-1,2,2) |
| 5 | arve_effects | 2: 0 jet_plane, 1 helicopter | uid0 door_open (6:9 `Arve_Door_Slide`, 6:1), uid1 door_close (-1, 6:2) |
| 6 | seve_effects | 0 | none: **unsupported, no id exists** |
| 7 | siren | 4: 0 default, 1 ice_cream_truck, 2 ice_cream_truck_special, 3 military_siren | sustain bank 6 sounds 10, 6, 11, 16 (`LAVE_Siren`, `Ice Cream`, `Ice Cream Pimped`, `Military Siren`); key_on/key_off absent |
| 0, 2, 3 | engines | see JC2-005 | engines also carry secondary uids (lave: 0 gear_change, 1 load_keyoff on bank 5; only `astonmartin` has a sound: 5:1 `Hummer Engine Bludder`) - not selected by this task |

## Verified: precedence rules (probe `--selftest`, 34 cases, plus in-memory jeep overrides)

1. **Absent, malformed, disabled.** Only an *int* record counts (0x00420cf0); float/string
   leaves the default (module -1 => pair skipped). A pair with module < 0 is disabled even
   if its preset is set (retail `lave.v052_motorcross`: module1 -1, preset_chassis 0 =>
   no chassis). A positive module with absent/negative preset leaves preset -1 => cleared.
   Duplicate keys: the first record in the chain wins (the older helper used last-wins `dict()`).
   Module >= 8 trips a checked-vector failure in the original (size 8 inferred from the 0x0080d450 loop).
2. **Later pair, same module.** Entries are keyed by module, so two pairs naming one module
   share a slot; the later pair overwrites the preset only when its preset is >= 0 and never
   clears an earlier one. Different modules never compete for a slot.
   `(5,5,fx)` / `(7,7,sp)` run afterwards and replace the slot. If fx is < 0 (land) the
   pair-installed slot survives; air default is 0.
4. **Siren peculiarities.** `has_sirens` gates only the explicit siren install; a siren pair
   (`module2`=7) installs without the flag; `sound_preset_siren` is both pair-2 preset and the
   siren preset; explicit -1 under `has_sirens` installs a component that is silent (preset
   -1 fails the runtime index check) and *replaces* any pair-installed siren. `CMotorBike`
   never reads `has_sirens`; boats ignore fx/siren keys.
5. **Preset count.** Presets at or beyond the module's declared `presets` count load nothing,
   even when a key exists; the probe reports this as `preset_beyond_declared_count`
   (does not occur in retail). Preset display names are diagnostic only.

In-memory jeep overlays (`--overrides`): fx 3 -> slot 4 `motorcycle`; fx -1 or a float fx
-> no slot 4 (land); chassis module -1 -> no slot 1; `module1`->0 makes the chassis
pair retarget the engine slot (engine preset becomes `sedan`, chassis slot absent);
`has_sirens` 1 -> slot 7 preset 0 `default`, siren preset 3 -> `military_siren`.

## Fixture inventory (90 exports + 2 embedded)

`--all --table`: 50 `CCar`, 9 `CMotorBike`, 14 `CBoat`, 10 `CHelicopter`, 7 `CAirPlane`;
zero parse errors. Slot counts over the 90 exports: engine slot 0 on all 59 land, slot 1
chassis on 47 `CCar` (23 preset 0 `wind`, 24 preset 2 `wind_and_chassi_noise`, none preset
1; absent on firetruck, coach and large super truck) plus one `CHelicopter`
(`arve.v009_military_helicopter`: module1 1, preset 0) and no `CMotorBike`; slot 4 effects
on 49 `CCar` and 8 `CMotorBike` (presets 0:16, 1:4, 2:10, 3:10, 4:14, 5:3); slot 5 on 9 air
exports (preset 0: 4, preset 1: 5; the other 8 air exports set `sound_preset_fx` to -1
explicitly, so the air default-0 branch has no retail export fixture); slot 7 on 8 `CCar`
(`has_sirens`=1; presets 0:2, 1:1, 2:1, 3:4); slot 2/3 engines as in JC2-005. No export sets `module2`; no export uses an out-of-range module, a missing
descriptor or a count overrun.

Six default fixtures (exact, bank:sound):

| Fixture | chassis slot 1 | effects | siren | horn / door ids |
|---|---|---|---|---|
| `lave.v014_jeep` CCar | (1,2) wind_and_chassi_noise, sustain 4:2 | slot 4 preset 0 big_vehicle | none (`has_sirens` 0) | horn 6:12 loop; door 6:1 / 6:2 |
| `lave.v047_jeep_small` CCar | (1,0) wind, sustain 4:0 | slot 4 preset 4 medium_vehicle | slot 7 preset 3 military_siren, sustain 6:16 | horn 6:0; door 6:1 / 6:2 |
| `lave.v024_sportscar` CCar | (1,0) wind, 4:0 | slot 4 preset 4 | none (siren preset -1, `has_sirens` 0) | horn 6:0; door 6:1 / 6:2 |
| `seve.v082_motorboat_standard_small` CBoat | module1 -1: none | **unsupported**: no loader, module 6 has no ids | **unsupported** | none (vslots are assert stubs) |
| `arve.v059_civilian_helicopter` CHelicopter | module1 -1: none | slot 5 preset 1 helicopter | none | door 6:1 / 6:2, no horn |
| `arve.v010_personal_propellerplane` CAirPlane | none | slot 5 preset 0 jet_plane | none | door_open 6:9; door_close **silent (-1)** |

Boundary fixtures: `lave.v012_military_tank_small` (fx -1 and siren -1, has_sirens 0 -> no
slot 4/7); `lave.v052_motorcross` (module1 -1 with preset_chassis 0); `arve.v062_cutscene`
(engine, chassis and fx all disabled -> no slots); all 10 vehicles using fx preset 3
`motorcycle` (8 `CMotorBike` plus the two `CCar` ATVs) have **silent doors** (`-1`) but a
horn (6:13); no `CMotorBike` has a chassis slot.

## Verified: per-placement override evidence

Across every property member of the install (1,236 inside `.nlz` + 7,996 in `.bin`,
`.ee`, `.flz`, `.blz`; raw hash scan with a positive control) the selector keys appear
only in the 90 vehicle exports and in **`f1m02.nlz/f1m02.nl.blo`**, which embeds two complete
vehicle objects (not `CVehicleComponent` references): `CHelicopter1` (class CHelicopter,
engine (3,0), chassis -1, **no `sound_preset_fx`** -> air default: slot 5 preset 0
`jet_plane`, door_open 6:9, door_close silent) and `V001_Sedan` (CCar, engine (0,2),
chassis (1,2), no fx -> no effects). A `CVehicleComponent` (six examined in
31x14; the full-install scan found no selector key on any other) carries only the export path `fa726895`, never a selector; the loader receives
the property section of the object itself and there is no merge layer. `.dat` (4,097) and
untyped (`?`, 2,444) archive entries were not scanned; mission scripts could still call
setters at runtime (see unresolved).

## Verified: horn, door, siren ownership (decompile)

* The component for slot 4/5 lives in the vehicle's component vector (+0x4c4, 8-byte
  entries; slot 4 at +0x20, slot 5 at +0x28). Voices are owned by the component
  instance: one held-loop handle (impl+0x2c) and a one-shot list (impl+0x3c).
* Requests are *pending ids*, not immediate plays: 0x009d3390 stores the uid in a byte
  at impl+0x20 (0xff = none); the component's per-frame update (tail of 0x009e3790,
  the same function as the chassis state machine) looks the item up by
  uid (0x009e2cf0), plays it via 0x009f0470 (gated on the primary state 2/4 and the item's
  mode field; exact condition not decoded) and writes 0xff. One pending id per component: a later
  request in the same frame replaces an earlier one. `StopSecondary` (0x009e3490 ->
  0x00a0a7b0) is immediate.
* Horn (land only): `CLandVehicle`/`CCar`/`CMotorBike` vslot 61 = 0x00624650 (press: if
  flag +0xd9e clear, set it and request uid 0 on slot 4) and vslot 62 =
  0x00562a00 (release: if flag set, clear it and `StopSecondary(0)`).
  Edge-triggered through +0xd9e. `CVehicle`, `CAirVehicle` and `CBoat` map
  these slots to the 0x00677ff0 assert stub. Caller: 0x00548770 (vslots 0xf4/0xf8, only when
  vslot 58 = byte +0xd9f is false; the else branch goes through a singleton and is not decoded).
* StopSecondary(uid) 0x00a0a7b0: non-loop item => stop every live voice of that item; loop
  item that is the held handle => without `key_off_on_stop` hard stop (vt+0x3c(0)),
  with it a keyed stop passing `var_id_cursor` (horn: 2) to vt+0x24. Hence horn = held loop
  (6:12/13/14/15/0 `is_loop`) released by key-off, not a one-shot.
* Doors: vslot 79/80. Land 0x007570f0 / 0x007c1380 = secondary uid 1 / uid 2 on slot 4;
  air 0x007d3df0 / 0x00727f40 = uid 0 / uid 1 on slot 5 (consistent with the per-module uid
  tables above). Called from character states 0x00637260 (vslot 79) and 0x0057a990
  (vslot 80); which animation phase each is (enter vs exit) is not decoded.
* Chassis (module 1) uses the primary state machine 0x009e3790: with `trigger_on_speed`
  set it starts when vehicle speed >= 5.0 (unit of 0x009dc0d0 undecoded) and stops below.
  The `key_on` role has no bank, so entering state 1 is silent and sustain follows.
* **Transfer**: nothing in the descriptor chain moves voices between vehicles. The
  vehicle's own components are created in its loader and own their voices, so a vehicle
  change creates/ends voices with the vehicle object; the runtime's occupied-entity
  voice ownership (JC2-027) matches this. Cross-vehicle transfer of a held horn voice
  is **not evidenced** either way.

## Unresolved (do not infer)

* Release-tail timing, `min_duration_ms` use, gain/rank arbitration, loop crossfade, FMOD
  event parameter wiring of `var_id_cursor` and `88096532` item blocks: JC2-035 (listening).
  A selector match proves the (bank, sound) pair only.
* `has_sirens` runtime setter (0x007f0080) and the writer of +0xd9f/the siren toggle path
  (singleton vslots 0xfc/0x100): not located; mission/AI use unknown.
* The land capability gate (`FUN_007c9990` + node capability 4) in front of fx/siren and its
  effect on vehicles that fail it (probe assumes it passes).
* Entry-vector initial values and exact size 8 (constructor untraced); duplicate-key chain order
  assumed equal to file order.
* The three module key names; the meaning of the `88096532` item child (up to 8 ints).
* Door call-site phase; unit of the speed trigger; whether `CMotorBike` ever reaches `has_sirens`.
* Unscanned `.dat`/`?` entries and DLC/modded overlays: inventory is the installed VFS only.

## JC2-034 handoff (bounded)

Scope to implement, nothing else: extend `soundmap::VehicleSoundSelection` consumers so the
already-decoded raw fields feed a **slot table** `[Option<(module, preset)>; 8]` built exactly
as above (pairs by module index, same-module later-positive overwrite; then class-specific
fx/siren). Add the new raw reads: `sound_preset_fx` (0xbe5386e2), `has_sirens` (0x3da4075f),
`sound_preset_siren` as the siren preset (0x834b3da2) and class from `_class`. Class rules:
land default fx -1; air default fx **0**; boat none; siren only `CCar` with `has_sirens`.

Assignments to expose per slot, all numeric (module id + preset index + descriptor role/uid):
chassis slot 1 sustain (bank 4); effects horn/door_open/door_close by **module-specific uid**
(module 4: 0/1/2, module 5: door_open 0, door_close 1 - never by label); siren slot 7 sustain
(bank 6). Silent states (explicit): -1 preset or sound id, missing `soundbank_id`,
no descriptor, preset beyond declared count, boats. Horn: edge press starts the held loop
once, release triggers key-off with `var_id_cursor`; doors are one-shots. Wire only after the
existing occupied-entity ownership; keep the legacy fallback explicit.

Acceptance fixtures: the six rows above, `v052_motorcross` (no chassis), `v012` tank (no
effects), `v045_firetruck` (siren preset 0), `v006_moped` (silent doors, horn 6:13), `f1m02`
helicopter (air default 0), `arve.v062_cutscene` (all silent). Do not claim release-tail or
parity; those remain JC2-035. Unsafe to implement without more evidence: siren on/off
toggling, door phase triggers, any voice transfer between vehicles.

## Reproduction (PowerShell from the worktree; copy prebuilt `jc2tool.exe` first)

```powershell
$g='C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2'; $t='<private>\jc2tool.exe'
python -B research/tools/vehicle_effects_selector_probe.py --selftest
python -B research/tools/vehicle_effects_selector_probe.py --tool $t --game $g --overrides
python -B research/tools/vehicle_effects_selector_probe.py --tool $t --game $g --all --table `
    --fingerprint --blo f1m02.nlz/f1m02.nl.blo
Get-FileHash "$g/JustCause2.exe" -Algorithm SHA256
rg -n '00824810' research/analysis/pseudo/_unnamed/text_0075*.c research/analysis/pseudo/_unnamed/text_007c*.c
```

No game asset or decompiler dump is committed; addresses cite the local Ghidra export.
