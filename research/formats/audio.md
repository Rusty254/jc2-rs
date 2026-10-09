# Audio (`sound\fmod\*.fsb`, `*.fev`)

JC2 uses FMOD Ex 4.x with FMOD Event (FMOD Designer 4). `sound\fmod\` (in `pc0`; the
dialogue per language in `pc2`) holds 17 event projects (`*.fev`) and the sample banks
they play from (`*.fsb`). The big streamed banks (music, cutscenes, mission dialogue)
lie loose in `sound\fmod\` of the game folder.

## FSB4

| offset | size | |
|---|---|---|
| 0 | 4 | `FSB4` |
| 4 | 4 | sample count |
| 8 | 4 | size of the sample headers |
| 12 | 4 | size of the data |
| 16 | 4 | version, `0x00040000` |
| 20 | 4 | flags: `0x2` basic headers (all after the first are just two lengths), `0x20` MPEG frames padded — JC2's banks are `0x20` |
| 24 | 8 | zero |
| 32 | 16 | hash |
| 48 | | the sample headers, then the data |

A sample header starts with its own size (80 bytes, sometimes 92):

| offset | size | |
|---|---|---|
| 0 | 2 | header size |
| 2 | 30 | name (`Pistol_New_Full_2_Mid.wav`, cut at 30 characters) |
| 32 | 4 | length in samples |
| 36 | 4 | length in bytes |
| 40 | 4 | loop start |
| 44 | 4 | loop end |
| 48 | 4 | mode: `0x2` loop, `0x20` mono, `0x40` stereo, `0x200` MPEG, `0x2000` 2D, `0x100000` 3D, `0x80000000` sample-direct |
| 52 | 4 | default frequency (16000–44100) |
| 56 | 2 | volume (255) |
| 58 | 2 | pan (128) |
| 60 | 2 | priority (128) |
| 62 | 2 | channels |
| 64 | 4 | min distance (f32) |
| 68 | 4 | max distance (f32, 10000) |

The data follows the headers, sample after sample, each a multiple of 32 bytes (the
lengths in the retail banks already are). Every sample in `weapons_common`,
`effects_common`, `vehicles_common`, `character_foley`, `parachute` and `structures` is
MPEG audio (layer III), its frames padded to an even length: drop the padding and the
frames are a plain MP3 stream (`jc2_formats::fsb::SoundBank::mpeg`; `jc2tool fsb ..
weapons_common.fsb --out dir` writes them as `.mp3`).

Names say what they are: `AR_625_HiLo_L` is the assault rifle's loop at 625 rounds a
minute (close, `HiLo`; `Mid` for further off), `Footstep_Dirt_Run_01…05` the
variations of a step, `jc7_idle_eng`/`jc7_onlow_eng1`… the engine layers of one car by
load and rpm (`on`/`off`/`free`, `verylow`…`veryhigh`, `eng`/`ex` engine and exhaust),
`h1_*` a helicopter's, `speedboateng_*` a boat's, `Rotor_03` a rotor loop.

## FEV1: the event projects

Read by `jc2_formats::fev` (every byte of all 17 files). `jc2tool fev <game>` lists the
projects, `jc2tool fev <game> weapons` the events, and `--event Single/Pistol_Shot`
resolves one event to its layers, sound definitions, banks and samples.

The layout was worked out from the bytes of the retail files. xoreos's FEV loader
(`src/sound/fmodeventfile.cpp` in <https://github.com/xoreos/xoreos>, written for an
older FEV1 used by BioWare games) guided the field order of the header, the
categories, the event properties, the sound-definition templates and the reverbs.
JC2's version differs from it: events carry a 16-byte id, the steal priority, an extra
word after the 3D distances, real layers, envelopes and parameters, and the sound
definitions list their waveforms. No public description of this version was found.
Everything not in xoreos is from the bytes, checked across all files: parsing ends
exactly at the end of each file, and every index is in range. 1822 of the 1853 sample
waveforms of the sound-effect projects name the sample at their index; the other 31
name samples the banks were rebuilt without (GUI swishes, the flak, a siren, a
tractor layer).

Everything is little-endian. A string is a `u32` length that counts its NUL, then the
bytes.

| part | layout |
|---|---|
| header | `FEV1`, `u32` version `0x003b0000`, `u32` ×2 that grow with the file (`0x534`, `0x654` in `parachute.fev`; unexplained), string project name (`Weapons`) |
| wave banks | `u32` n, then each: `u32` load mode (`0x200` decompress into memory, `0x80` stream), `u32` max streams, string name (`Weapons_Common` → `weapons_common.fsb`) |
| category tree | string name, `f32` volume, `f32` pitch, `u32` ×2 (0), `u32` n children, the children: `master` → `sfx` → `weapons` → `shots` |
| event groups | `u32` n, then each: string name, `u32` n user properties (+ the properties), `u32` n subgroups, `u32` n events, the subgroups, the events |
| sound-definition templates | `u32` n, 60 bytes each (below) |
| sound definitions | `u32` n, then each: string name (`/Single Shots/Pistol_Final_2_Mid`), `u32` template, `u32` n waveforms, the waveforms |
| reverbs | `u32` n, then each: string name (`Jungle Reverb`), 132 bytes (room, room HF, …, decay time at +12) |
| trailer | `08 00 00 00` `comp`: an empty chunk closing the file |

### Events

An event starts with `u32` kind (`0x8` complex: 1690 of them; `0x10` simple: 9, in
`music.fev`), a string name, and 16 bytes (an id shared by events made from one
template). Then come 33 words:

| fields | meaning |
|---|---|
| `f32` volume, pitch, pitch randomisation, volume randomisation | Linear gain. Pitch is in FMOD's raw units (assumed 4 octaves per unit: the designers' values are multiples of 0.0025) |
| `u32` priority, max playbacks, steal priority (0–10000) | |
| `u32` mode | FMOD Ex `FMOD_MODE`: `0x8` 2D, `0x10` 3D, `0x80000` world relative, `0x100000` log rolloff, `0x200000` linear rolloff, `0x4000000` custom rolloff, `0x40000000` ignore geometry. Most JC2 events are `0x4080010`: 3D with a custom rolloff drawn as volume envelopes on the `(distance)` parameter |
| `f32` min, max distance | Metres. With a custom rolloff the max is the end of the distance parameter |
| `u32` | `0x80000`, `0`, `0x80000000` or `0x40080000`; unexplained |
| `f32` ×8 | 2D speaker levels |
| the rest | `f32` cone inside, outside, outside volume; `u32` max-playbacks behaviour; `f32` doppler, reverb dry, reverb wet (dB, −60), speaker spread; `u32` fade in, fade out (ms); `f32` spawn intensity, its randomisation, pan level; `u32` position randomisation. This is xoreos's order |

A **simple** event then has `u32` n sound instances (one layer without envelopes),
followed by the category. A **complex** event has `u32` n layers and the layers,
`u32` n parameters and the parameters, and `u32` n user properties (0 everywhere). It
ends with `u32` 1 and a string category (`sfx/weapons/shots`).

A **layer**: `u16` flags (2), `i16` priority (−1, 128, 256), `u16` the parameter its
sounds are laid out on, `u16` n sound instances, `u16` n envelopes, the instances, then
the envelopes.

A **sound instance** (58 bytes):
- `u16` sound definition.
- `f32` start and `f32` length on the layer's parameter (normalised 0..1).
- `u32` start mode.
- `u32` loop mode: 0 loop and cut off, 1 one-shot, 2 loop and play to the end.
- `i32` loop count (−1).
- `u32` automatic pitch on/off, `f32` its reference point, `f32` its value at zero. On
  engines the sample plays at its own pitch at the reference rpm.
- `f32` fine tune, `f32` volume.
- `f32` crossfade-in and crossfade-out widths (−1 for none), `u32` ×2 their curve
  shapes.

An **envelope**:
- `i32` DSP unit (−1 for the built-in effects), string the DSP's name (`Channel Mix`,
  `FMOD Distortion`, or empty), `u32` its parameter.
- `u32` kind, `u32` 0.
- `u32` n points × (`f32` x normalised, `f32` y, `u32` curve shape).
- `u32` the event parameter, `u32` 0 or 1.

The bits of the kind above the low three name the effect:

| bits | effect |
|---|---|
| `0x8` | Volume. Every curve rests at 1; on `focus`, `(distance)`, `variation`… |
| `0x10` | Pitch. Rests at 0.5; ±0.5 is read as ±1 octave (assumed) |
| `0x400` | 0..1 on `(distance)`: 3D pan level (assumed from the shape; applied) |
| `0x800` | 0..1 on `(distance)` in the ambiences: spread (assumed, not applied) |
| `0x2000` | On `(listener angle)` and `(distance)`: occlusion, a low-pass (assumed; applied) |
| `0x1000`, `0x100`, `0x20000`, `0x10000` (DSP parameter), `0x40`, `0x200` | Not applied |

A **parameter**:
- string name.
- `f32` velocity: units per second it moves by itself (`timeline`, `variation`).
- `f32` min, `f32` max.
- `u32` flags: 2, 3, 5, 8 or 9. 8 marks the self-moving ones that loop; the rest are
  unexplained.
- `u32` ×2.
- `u32` n sustain points, then an `f32` each.

The game sets `rpm`, `load`, `damage`, `speed`, `steer`, `height`, `depth`,
`timeofday`, `precipitation` and `waterdepth`. FMOD sets its own `(distance)` and
`(listener angle)`.

### Sound definitions

A **template** (60 bytes):
- `u32` play mode (0–4).
- `u32` spawn time min and max (ms). A definition with a spawn time re-triggers a
  waveform this often instead of looping, as the ambiences' birds and crickets do.
- `u32` max spawned, `f32` volume, `u32` ×3 (1, 1.0, 1.0).
- `f32` volume randomisation: the lowest random gain, 1 for none.
- `f32` pitch, `u32` ×3 (1, 0, 0), `f32` pitch randomisation (±).
- `f32` 3D position randomisation (m).

A **waveform** is `u32` kind and `u32` weight (0..100, the chance relative to the
others), then by kind:

| kind | then |
|---|---|
| 0 sample | string file (`weapon/weapons_readytogo/flak/FLAK_….wav`, the designer's path), string bank, `u32` index in the bank's `.fsb`, `u32` length (ms) |
| 1 oscillator | `u32` shape, `f32` frequency |
| 2 silence | nothing. Weighted gaps: the `MonoSilence_…` entries make the crickets skip |
| 3 programmer sound | nothing (dialogue) |

## How the game names its events: sound ids

The exe asks FMOD for events through numbers, not paths (`sound_id`, `soundbank_id`,
`sound/settings_core.bin`, `settings_smod_*.bin`). `strings.csv` holds no event paths; the
key names of the sound files were found by hashing the exe's strings against them
(`data/sound_property_names.txt`) and a few word lists (`fire_shot`, `Metal_Solid` …).
`jc2_formats::soundmap` reads all of it; `jc2tool soundmap <game>` prints it
(`--id N` a weapon sound set, `--bank B --sound S` one pair, `--list weapons | characters |
vehicles | environment | music | materials | effects`).

**A pair is (bank, sound).** `settings_core.bin` lists the projects (`projects`: id → `.fev`)
and the banks (`soundbanks`: a global id, the `eventgroup_name`, the `project_id`, the
`eventgroup_id` and the `.fsb` it plays from). A bank is one top-level *event group*: bank 0
is `weapons.fev` group 0 `Auto`, 1 `Single`, 2 `Shells`, 3 `Acoustics`, 8 `Mechanics`, 9
`Handling`, 11 `effects.fev`'s `Explosions`, 12 `character.fev`'s `Foley`, 37 `Explos`.
The sound is the event's position in that group, in file order (`fev::EventFile::event_in_group`):
`Single` event 5 is `Pistol_Shot`. No retail group has subgroups. Checked on every pair
in the sound modules and the 340 effects that carry one (test `every_pair_resolves`).

**Weapons** (`settings_smod_sfx_weapon.bin`). `weapons.bin`'s `sound_fire_id` is the id of a
*sound set* (`sound_sets`: `pistol_fire` = 6, `minigun_fire` = 7 … 24 sets; the vehicle guns
use 1, 8, 10, 13 …). A set has layers, each a pair with an `event_type` (0 `key_on`, 1
`key_off`, 2 `reload`, from `#53edc049`), `looping`, `max_playbacks` and a `setup_id` (the
module's `connection_setups`: which event parameter the module drives): `fire_shot` and
`fire_loop` (the shot; the automatics' is `looping`), `fire_shell` (the shell on the
ground, its `hardness` parameter from `ground_hardness_maps` by ground material),
`fire_reload`, and the tail (`key_off`: the slap-back or early reflection that sounds
when the trigger is released), plus `hold_ms`. The Assault, Scorpion SMG and machine guns
list `#1c6b76f9` (`#06ea0642` the upgrade level, `sound_set_id` the faster set): the rifle
is `AR_LowRate`, from upgrade 2 `AR_MidRate`, from 4 `AR_HighRate`.

**Effects** (`effects\*.bin`). The effect's `Sound` component (`type = 0`) holds variants keyed
by the hash of a game material's name (or one for any ground), each with `bank_id` and
`sound_id`: `grenade_explosion` plays `Explosions/13 - Grenade Explo`, `explosion_barrel`
`Explos/Barrel Explode`, `flying_hook` / `hook_detach` the grapple's `grappling_fire_wire` /
`grappling_detatch`, `missile_fire` the `Rocket Launcher Loop`. `SoundPostEffect`
(`type = 9`) is the mixer's ducking, not a sound. The weapons' `detonation_effect_id`
indexes a table the data does not hold; the game picks the effect by the blast's size.

**Characters** (`settings_smod_sfx_characters.bin`). `event_lookup_table`: the character's event
name (`footstep`, `footstep_car`, `jump_start`, `jump_land`, `swim`, `parachute_open_vertical` …)
→ a `sound_set_id` and sometimes an `interaction_param` (a ground material the event forces:
21 `Metal_Car` for `footstep_car`). `primary_sounds`: each set is one pair (`id_set`) or an
`id_set_map`: a default pair and a `map` from game material (`map_slot_id`, the key the hash of
its name) to the pair. `footstep` maps 17 materials to the `Footsteps_*` events of `Foley`
(Soil → `Footsteps_Dirt`, Sand → `Footsteps_Sand_Dry`, Snow, Gravel, Stone/Concrete/Asphalt →
`Footsteps_Stone`, Metal, Metal_Car, Metal_CatWalk, Wood_Solid, Ice …).

**Ground materials** (`settings\game_materials.bin`): 35, `Sand` 0, `Soil` 1, `Snow` 2, `Gravel` 3,
`Stone` 4, `Ice` 5, `Wood` 6, `Water` 7, `Metal` 8 … `Asphalt` 30, `Default` 31, `Grass_Field` 34,
with a friction. A terrain tile's gameplay byte (`terrain.md`) leads to one through
`terrain_materials.bin`'s `GameMaterial` (`default_desert` → 0, `road_jungle_dirt` → 3). Havok
rigid bodies and shapes carry **no** material (the bodies of vehicles, rocks and buildings are
identical in every field but their names; `userData` is 0), so only the terrain has a surface.

**Vehicles** (`settings_smod_sfx_vehicle.bin`). `component_list` has modules `lave_engine` (land),
`seve_engine` (sea), `arve_engine` (air), `*_effects` (doors, horns), `lave_chassis`, `siren`. Each
has `presets` (a key → a name: `hummer`, `SmJeep`, `Corvette`, `bus`, `tank`, `atv`, `harley`,
`speedboat`, `longtail`, `cessna`, `heli_transport`, `jet_plane_afterburner` …; 22 land, 6 sea,
8 air) and `primary_sound` with `key_on`, `sustain` and `key_off`: per preset the sound id of
`Hummer Engine`, `Hummer Start`… (`Engines` is bank 5; `Jeep` is 35, its start 36, its stop 37).
Vehicle export EPEs select numeric module/preset pairs. Preset keys hash
`preset_00` ... `preset_99`, independently of display names. Runtime engine selection
joins these pairs directly: v014 uses SUV/Cheyenne, v047 SmJeep/Jeep, v024
astonmartin/Ferrari 430. Explicit disabled exports stay silent; malformed or unresolved
selectors stay diagnostic and silent. Container-name guesses remain an explicit legacy
fallback only for missing export support. See `vehicle-engine-next.md` for evidence,
scope, lifecycle and remaining audible-parity checks.

**Environment** (`settings_smod_sfx_environment.bin`). `zones.sound_sets`: `field`, `jungle`, `desert`,
`mountain`, `city`, `ice` and the settlement sets, each a bed (`sound_id` in bank 10 `zones`),
`detail_looping` and `detail_non_looping` (the sprinkle, with `spawn_dt`) and a `rain_sound_set`.
`lookup_table`: per `climate_zone_id` a `default_sound_set` (0 `field`, 1 `mountain`, 2 `desert`, 3
`city`) and `game_zone_sound_sets` overriding it by `game_zone_id` (the zones of `gamezone.bin`:
1 arctic, 2 desert, 3 jungle, 4 sea, 5 city, 6 grass, 7 default; 8–14 settlement kinds). The
weather has `rain_*` and `thunder` (bank 36).

**Music** (`settings_smod_music.bin`, bank 25 = `music.fev` `Adaptive Music`). 14 `track_groups`;
group `g` has slots `0, 1, 2` = events `3g, 3g+1, 3g+2` = `…_Undetected`, `…_Combat`,
`…_Evasion` (`Def_Weird`, `Nig_Scary`, `Def_Techy`, `Def_Agentish`, `Def_Stressful`, `Def_Nervous`,
`Def_Bondish`, `Def_Military`, `Def_Tribal`, `Def_TheBrass`, `Cha_Prime`, `Cha_Peak`, `Cha_Heat`,
`Cha_Finale`). Two inputs choose the groups: input 0 is the hour (`translation_rules`: from 18
condition 1, from 6 condition 0; before 6 the default 1: night) and input 2 the chaos (50 000
→ 1, 100 000 → 2, 175 000 → 3, 290 000 → 4, 400 000 → 5). `track_groups_selections` is keyed
`"<time>:<chaos>"` and lists the groups allowed (the night's set has `Nig_Scary`; `Cha_Prime` joins at
chaos 2, `Cha_Peak` at 3, `Cha_Heat` at 4, `Cha_Finale` at 5). `state_hold` = 5 s. The samples
lie in the loose `Music_Common.fsb` / `Music_stereo.fsb` (100+ MB; `fsb::SoundBank::parse_headers`
and `sample_range` read them by offset). **Each sample is a 6-channel MPEG: three stereo
stems.** The frames of the stems alternate in the data, each frame padded to 16 bytes (a
418-byte frame, then 14 bytes that begin like a header and are padding: scanning for sync
words reads garbage, so `fsb::SoundBank::mpeg_pairs` walks frame by frame and deals them out
round-robin; each stem then decodes to the sample's full length). The events take `focus`
and `heat` parameters; three envelopes on `heat` (kinds `0x30000` with the DSP name `Channel
Mix`, `0xc0000`, `0x300000`: the channel pairs 0, 1, 2) mix the stems: pair 2 (calm) is
full below a heat of 0.2 and gone by 0.4, pair 1 (medium) peaks at 0.4–0.6, pair 0
(intense) comes in from 0.6 to 0.8. `music.rs` applies them (`stem_levels`), with the heat
as `Heat.value / max`.

The other modules (`gui`, `crowds`, `voiceover`, the AI barks of `ai\sound\soundeventmap.bin`:
event names → dialogue sample names) are not used yet. `soundeventmap.bin` is the AI's bark
table, not the SFX map.

## DSP envelopes: what the kinds are

The bits of an envelope's `kind` above the low three, counted over all projects
(`jc2tool fev <file> --envelopes`): `0x8` volume, `0x10` pitch; `0x400` on `(distance)` /
`distance` 0 close up and 1 from a few metres: **3D pan level** (a gun at Rico's feet stays
centred; 241 on the vehicles, 412 in the effects); `0x2000` on `(listener angle)` (0.4 behind,
0 ahead) and `(distance)`: **occlusion**, applied as a low-pass; `0x800` on `(distance)` in
the ambiences: spread; `0x1000`, `0x100`, `0x200`, `0x40`: reverb, cones, other (not applied).
`0x10000` / `0x20000` with the DSP name `Channel Mix` and `FMOD Distortion`, `FMOD Lowpass
Simple` (`kind 0`) are real DSP units on a few events, not applied. These readings come from the
shapes of the curves.

## In jc2-rs

`audio.rs` asks `soundmap` for the event of every sound it plays and loads it (and its samples)
the first time. `audio_dsp.rs` is the sample player; `music.rs` the music.

| sound | where it comes from |
|---|---|
| Rico's guns | `weapons.bin` name → `sound_fire_id` → set (at the gun's upgrade level): shot or loop, tail after the hold, shell with the ground's hardness, reload |
| soldiers' shots | the `Assault` set's shot at the soldier, at most 3 at once |
| rocket | the `RocketLauncher` set's shot and the `missile_fire` effect's loop |
| blasts | effect by size: `grenade_explosion`, `explosion_fuel_depot_small`, `explosion_barrel`, `explosion_lave`; structures `destruction_default` |
| footsteps, jump, landing | character events `footstep` / `jump_start` / `jump_land` by the ground material under Rico (terrain tile, a worker thread reads the tile's byte map); `footstep_car` on a car's roof; `swim` held while swimming |
| grapple | effects `flying_hook`, `hook_detach` |
| canopy opening | character event `parachute_open_vertical` |
| engines | export numeric module/preset → `key_on` at instance entry, `sustain` with existing `rpm`, `load` held, `key_off` at exit or vehicle transfer |
| ambience | `zone_set(climate, game zone under Rico)`: bed, details, sprinkles (`--no-ambience`) |
| music | `music.rs`: group chosen by hour and chaos, mood by the heat (`--no-music`, `--music-volume`, `--volume`) |

Still by name (`EVENTS`, logged once when first played with `audio: … plays by name`): the
canopy's flap `Parachute/Parachute_steer` and the fall's `Wind_skydive` (the data names only the
canopy's opening; the loops are driven from code), and any sound whose id fails to resolve (the
`FALLBACK` sample names are the last resort).

**What the player runs**, per start: the parameters (`rpm`, `load`, `speed`, `speed vertical`,
`steer`, `hardness`, `(distance)`, **`(listener angle)`** from the camera, `timeofday`, the
self-moving `timeline` and `variation`), each sound instance's gain (event × category × the
layer's volume envelopes × instance × template × crossfade × the 3D rolloff × the event's
**fade in / fade out**) and pitch, the waveform picked by weight, the randomisation, loops,
re-triggered spawns. DSP: a Bevy `Decodable` (`Voice`) decodes the MP3 and applies, from
atomics the game updates each frame, a smoothed gain, an equal-power pan (the position's
sine to the listener's right, times the layer's pan level) and a one-pole low-pass
(occlusion 0.4 → about 5 kHz, 1 → 600 Hz). Loops loop inside the source (rodio's
`repeat_infinite` buffers the processed samples). Pitch is the sink's speed.

**Max playbacks**: an event has `max_playbacks` and a behaviour (0 steal the oldest, 1 the
newest, 2 the quietest, 3 just fail, 4 fail if it would be the quietest: the order of the
designer's options, assumed); every event used here has behaviour 3, the pistol's limit is 7. An instance over the limit steals (the stolen fades out) or fails; held
instances (an engine) are never stolen; the weapon module's own `max_playbacks` of the shot
lowers the limit; the mixer holds 48 sample players.

Not done: the reverbs, `FMOD Distortion`, `Channel Mix`, the cone envelopes, the spread, occlusion
by walls and water planes (the environment module has `water_planes` occlusion), the weapons'
`ground_hardness` for the impacts, the AI barks, the
crowds, the settlement-kind ambiences (game zones 8–14) and dialogue.

### Vehicle door and horn playback

Retail `jc2tool soundmap --list vehicles` resolves `lave_effects` secondary roles:
medium_vehicle uses horn 6:0 and door open/close 6:1/6:2; big_vehicle horn 6:12
with those same doors; small_vehicle horn 6:13 and doors 6:3/6:4; motorcycle has
horn 6:13 only; truck/truck2 use horns 6:14/6:15 and big doors. These are effects
preset names, not engine preset names or hashes. `seve_effects` has no presets;
`arve_effects` helicopter has big doors, jet_plane has door_open 6:9 only.

`audio/vehicle_fx.rs` still assigns these effects presets by inferred body type:
jeep/bus big, small jeep/sports small, trucks truck, ordinary
sedans medium, motorcycles motorcycle; ATV/tank/tractor have no established effects
assignment. It observes actual Occupied entity transitions after gameplay updates,
including hijacks and changing between identical car models. Each seat entry/exit
plays open immediately, close after 0.65s (the small open sample is 618ms); this
spacing is assumed while seat transfers remain instantaneous. Missing roles stay
silent; streaming/despawning does not produce door sounds. Doors are spatial and
follow the vehicle entity. Physical door animation remains open.

Hold H while driving a land vehicle for its horn; release, leaving the seat, or
switching to free camera stops it with the event's 30ms fade. No other control uses
H. Horn playback is at the driver's ears like the engine. Retail Horn medium/big/
small events have `velocity` 0..1, automatically moving at 1/s; this is an envelope
time control, not vehicle speed. Their loop runs at 0..0.139, with its full-volume
plateau before 0.06; the acoustic one-shot starts around 0.059/0.064. Held playback
pins velocity to 0.03. Explicit caller-controlled parameters now override automatic
parameter motion; uncontrolled FMOD timelines continue normally. Release fades the
loop without triggering a separate acoustic tail (tail timing is still open).
