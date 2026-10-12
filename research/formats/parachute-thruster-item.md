# Parachute thruster item (JC2-260)

Resolves gap 1 of `parachute-canopy-landing-next.md`: where the object at `canopy+0x6c` (updated by `0x005bcbf0`) is created, from
which data its numbers come, and what is and is not in the retail files. Probe: `research/tools/parachute_thruster_probe.py`
(exit 0, 24 checks; `--tool` adds the retail-data checks, `--scan-all` dumps every `.bin`).

## 1. Creation chain (native, byte-checked)

1. DLC pack loader `0x005352a0(flags, packIndex, isServer)`: with flag 8 it formats `"%i_parachute.bin"` (string `0x00e3a4ea`) and loads it
   with `0x007751c0` into the character's parachute table; it also registers `"%i_parachute_location"`.
2. `0x007751c0(table, name)` opens the property file and, for each element, constructs an entry (`0x005acfc0`), parses it
   (`0x0067d340`) and appends it to a vector of `0x108`-byte entries (table at `character+0x674`).
3. `0x005beed0` (called through `0x005ad5f0` -> `0x005ad653` with the local player, `[0x118fb24]+0x3570`) loads the table from the file
   named by `character+0x2a0`, finds the entry whose key equals `character+0x2bc` (`0x00507560`), constructs the canopy
   (`0x007e0250`, CAnimatedParachute, `+0x6c = 0`) and calls the canopy setup `0x00676860(entry)`.
4. `0x00676860`: **if `entry+0x94 != 0`** it allocates `0x88` bytes, runs the object constructor `0x0079b9f0`, stores the pointer in
   `canopy+0x6c` (`0x00676c1c`) and calls `0x005d7fd0(entry+0x98)`. An entry without the thruster group therefore never gets a
   thruster object (the canopy update skips `0x005bcbf0` when `canopy+0x6c == 0`, `0x006ccd40`). The canopy destructor
   (`0x005922e0`) frees it via `0x0066ff00`.
5. `0x005d7fd0` copies the params block `entry+0x98 .. +0xcc` (13 dwords) plus two strings into the object at `object+0x18`
   (`0x0067bd60`; so entry offset = object offset + 0x80), attaches two models named by the strings (`object+0x4c`, `+0x68` ->
   model slots `object+0` and `+4`, `0x008701b0`) and creates two effect handles from the effect id at `object+0x48` through the effect
   manager `[0x1190c48]` (`0x0062a7a0`, `0x00523240`) into `object+0x08` and `object+0x10` (the handles `0x005bcbf0` starts and stops).

## 2. Property keys (entry parser `0x0067d340`)

Key hashes are Jenkins lookup3 of the lowercase name, held in globals `0x00eb2850..0x00eb2894`. Names marked * were recovered by
dictionary search and verified by hash; unnamed ones stay unknown.

Entry level (also present in the retail `14_parachute.bin`): `item_id`* 0x3b03f508, `model`* 0x0f94740b, 0x26fa86fe, 0x475d8996
(statemachine), 0x4e25dc69 (pickup execute), 0x4247feaa (animation set), 0xb868e836 (effect settings).

Thruster group, hash **0xb314fdc5** (name unknown). Present = `entry+0x94 = 1`, then:

| key | hash | read into entry | object field | role in `0x005bcbf0` |
|---|---|---|---|---|
| `boost_strength`* | 0x355ea7cb | +0x9c | +0x1c | thrust added to the flight thrust in state 1 |
| `cooldown_time`* | 0x2c727e62 | +0xa0 | +0x20 | cooldown length |
| (not read) | - | +0xa4 | +0x24 | cooldown timer, starts 0 |
| (not read) | - | +0xa8 | +0x28 | charge: starts at the entry default, never read from a file |
| `min_energy`* | 0x7bf8e946 | +0xac | +0x2c | start threshold (charge must exceed it) |
| `max_energy`* | 0x0aca21e6 | +0xb0 | +0x30 | maximum charge |
| `consumption_speed`* | 0xdef7add2 | +0xb4 | +0x34 | drain per second |
| `recharge_speed`* | 0xd3b54f8c | +0xb8 | +0x38 | recharge per second |
| vec3, hash 0x13e64fd4 (name unknown) | | +0xbc..+0xc4 | +0x3c..+0x44 | steering scale vector used by the flight model |
| `effect_id`* | 0xa90d6c5c | +0xc8 | +0x48 | effect index for both handles |
| `left_model`* | 0x97395535 | +0xcc (string, suffix `[0x012ab4c8]` appended) | +0x4c | model attached as `object+0` |
| `right_model`* | 0x25232863 | +0xe8 (string, same suffix) | +0x68 | model attached as `object+4` |

## 3. Defaults (entry constructor `0x005acfc0`, effective when a key is absent)

Charge 5.0 (`0x00e2d1b4`), min_energy 0.5 (`0x00e21a28`), max_energy 5.0, consumption_speed 1.0 (`0x00e20668`), recharge_speed 2.0
(`0x00e2d510`), boost_strength 0, cooldown_time 0, timer 0, state 0, steering vector 1,1,1, effect_id -1. The probe interprets the
constructor's stores and asserts this table. Note boost_strength defaults to 0: a group without that key adds no thrust.

## 4. What the retail data holds (and does not)

* `14_parachute.bin` (DLC pack 14, Heavy Drop `parachute03`, model `DLC\Weapons\GAE18\GAE18_LOD1-GAE18`) is the **only** `N_parachute.bin`
  in the mounted archives. It has the entry keys above and **no thruster group**: that parachute has no thruster.
* No mounted `.bin` contains the group hash or any tuning key (full scan with `--scan-all`: 640 `.bin` files, zero hits).
* The effect `effects\dlc_personal_thruster.bin` (base archive pc0) and the HUD texture `hud_thruster_meter_dif.dds` (pc3) ship, but the
  item file that carries the numbers (a `N_parachute.bin` of a DLC pack that is not installed) is absent. The numbers are therefore
  **not recoverable from the game folder**; only the mechanism, keys, offsets and defaults are. Nothing in the executable or
  `parachuteparams.bin` / `grapplingparams.bin` supplies them.

## 5. Consequence for the runtime

The native default parachute (no thruster group) has no thruster at all: `canopy+0x6c = 0`, ACTIVATE_PARACHUTE_THRUSTERS is never
consumed. A faithful port should model the thruster as an optional item whose absence is the retail state, with the schema and
defaults above available if a pack file is ever supplied. Do not invent boost_strength / energy numbers for the standard canopy.

## Gaps

1. Values of the thruster group (boost_strength, energy, speeds, cooldown, steering vector, effect id, model names): the DLC pack file
   is not in the game folder. Unresolved, blocks nothing in the shipped game (no installed entry has a thruster).
2. Names of the group key 0xb314fdc5 and of the steering vector key 0x13e64fd4 (dictionary search exhausted ~0.5 M candidates).
3. Which character property writes the table file name (`character+0x2a0`) and entry key (`character+0x2bc`), i.e. how the equipped
   parachute is selected (inventory / black-market pick) - not needed for the canopy itself.
4. Entry field `+0x104` (`0x0067d340` leaves it at its constructor default 0; copied by `0x008055e0`) and the suffix global `[0x012ab4c8]` appended to the model names.
5. What the effect manager index `effect_id` selects in `[0x1190c48]` (which effect file) - the shipped `dlc_personal_thruster.bin` is the likely target, unproven.
