# jc2-rs — Just Cause 2, reimplemented in Rust on Bevy

Agent coordination: read [AGENTS.md](AGENTS.md) and the shared
[task board guide](TASKS.md) before picking up work.

A from-scratch engine that plays Just Cause 2 using the data files from your own
Steam install, in the tradition of OpenMW, OpenRCT2 and re3. Nothing from the original
executable is copied: file formats and game logic are worked out from the
reverse-engineering notes in `research/` and rewritten in Rust.

**Status: milestone 5.** The archives mount, every file the engine can see is listed
with its real name, containers and compression are transparent, both property-file
layouts decode, and a Bevy app renders textures and **models** straight out of the
archives through a `jc2://` asset source: `jc2 --model lave.v046_buscoach.ee` opens the
bus in an orbit viewer, textured and assembled from its vehicle doll. All 121,912 `.rbm`
models in the game parse and every block type in the data is decoded
(`research/formats/rbm.md`). The **world** is readable: `jc2 --world 31,14` flies you
over a piece of Panau built from the terrain height tiles, the sea and the tiles' placed
buildings and props (`terrain.md`, `world.md`). And it has **physics**: the terrain and
the game's own Havok collision shapes collide (Avian3d; `havok.md`), and `--drive` drops a
car built from the game's own vehicle definition — wheels, suspension, engine, gears,
steering (`vehicle.md`) — that you can drive around the village. And **Rico** is in it:
`jc2 --world 31,14` puts you on foot in his skin, animated from the game's own skeleton
and clips (`animation.md`) — walk, sprint, jump, fire the grappling hook at anything in
reach and get reeled in, open the parachute and glide, swim, climb into the car and out
again (`character.md`).

## Layout

```
jc2-rs/
  Cargo.toml                 workspace (edition 2024, Bevy 0.19)
  crates/
    jc2-formats/             parsers: .tab/.arc, SARC, zlib, property containers, name hashing,
                             GameFs (archives + DLC + dropzone as one virtual file system)
    jc2-tool/                jc2tool CLI: info / list / cat / extract / prop / sarc / rbm / obj /
                             scan-rbm / terrain / placements / vegetation / cgd / vehicle /
                             collision / icons / roads / civilians / text / missions / hash / harvest
    jc2-game/                the Bevy app (binary `jc2`) with the jc2:// asset source; `--browse` is the asset browser
  research/                  not code: Ghidra project + exports of the original exe, fingerprint,
                             class list, strings, call graph, per-function pseudo-C
```

## Building

1. Install Rust: https://rustup.rs (stable, 1.95 or newer; `rustup update` if older).
2. From this folder:

```
cargo build --release -p jc2-tool
cargo run --release -p jc2-tool -- info ..
cargo run --release -p jc2-tool -- list .. --kind | more
cargo run --release -p jc2-tool -- prop .. camera_settings.bin
cargo run --release -p jc2-tool -- sarc .. lave.v046_buscoach.ee
cargo run --release -p jc2-tool -- rbm .. lave.v046_buscoach.ee
cargo run --release -p jc2-tool -- obj .. lave.v046_buscoach.ee C:\tmp\bus.obj      (open in Blender)
cargo run --release -p jc2-tool -- scan-rbm ..                                   (every model in the game)
cargo run --release -p jc2-tool -- terrain .. --seams                            (all 4096 terrain tiles)
cargo run --release -p jc2-tool -- terrain .. 31 14 --pgm C:\tmp\tile.pgm
cargo run --release -p jc2-tool -- terrain .. 31 14 --materials C:\tmp\mat.ppm --map C:\tmp\pda.ppm
cargo run --release -p jc2-tool -- placements .. 31x14.nlz --models
cargo run --release -p jc2-tool -- vegetation ..                                 (climate zones, plant sets, types)
cargo run --release -p jc2-tool -- vegetation .. 31 14 [--list]                  (the trees and rocks the game grows there)
cargo run --release -p jc2-tool -- cgd .. 31x14.nlz                              (a tile's static geometry)
cargo run --release -p jc2-tool -- vehicle .. lave.v014_jeep.ee
cargo run --release -p jc2-tool -- collision .. 31x14.nlz                        (every .pfx in it)
cargo run --release -p jc2-tool -- weather ..                                      (sun orbit, sky colours by hour)
cargo run --release -p jc2-tool -- icons .. [--all]                              (the map's settlements / every map icon)
cargo run --release -p jc2-tool -- roads .. [31 13]                              (the road graph; a tile's junctions and roads)
cargo run --release -p jc2-tool -- civilians .. [26 41]                          (people and cars by zone; a tile's roam areas)
cargo run --release -p jc2-tool -- text .. [--lang de] [--key set_mil_txt_11 | --grep harbor]   (the localized strings)
cargo run --release -p jc2-tool -- font .. [jc2_smallfont_18] [--png C:\tmp\font.png]        (the HUD's bitmap fonts)
cargo run --release -p jc2-tool -- missions .. [f2t10] [--all | --strongholds | --colonels]  (missions, strongholds, colonels)
cargo run --release -p jc2-tool -- extract .. C:\jc2-extract --unpack
```

(`..` is the game folder when this repo sits inside it; otherwise pass the path to the
folder that contains `JustCause2.exe`.)

The Bevy app takes a few minutes to compile the first time:

```
cargo run --release -p jc2-game -- ..                                   milestone 1 (texture)
cargo run --release -p jc2-game -- .. --model lave.v046_buscoach.ee     milestone 2 (model viewer)
cargo run --release -p jc2-game -- .. --browse                          the asset browser
cargo run --release -p jc2-game -- .. --model areaset07.blz --lod 1     a whole area set of buildings
cargo run --release -p jc2-game -- .. --model lave.v005_tuktuk_dlc.ee/v005arty_lod1-weapon.rbm
cargo run --release -p jc2-game -- .. --world 31,14 --radius 1       milestone 3 (world viewer)
cargo run --release -p jc2-game -- .. --world 31,14 --drive          milestone 4 (the jeep, parked next to Rico)
cargo run --release -p jc2-game -- .. --world 31,14 --drive lave.v002_sport.ee
cargo run --release -p jc2-game -- .. --world 31,14                  milestone 5 (Rico on foot)
cargo run --release -p jc2-game -- .. --world 31,14 --heli           a helicopter parked beside him
cargo run --release -p jc2-game -- .. --world 30,14 --boat           a boat on the nearest open water
cargo run --release -p jc2-game -- .. --world 31,14 --time 22:00     night (also --time 06:00 dawn, 18:30 dusk)
cargo run --release -p jc2-game -- .. --world 31,14 --day-speed 30   the day passes 30× faster
cargo run --release -p jc2-game -- .. --world 40,40                  jungle: grass, forests out to the horizon
cargo run --release -p jc2-game -- .. --mission f2t10                 a mission, from its restart point
```

Original particle leaf preview: `jc2 --particle fx_vehicle_mc_smoke_01` loads the
MC damage-smoke graph, authored 25/s rate and 1 s lifetime, and its original DDS.
Default playback evaluates the original indexed modifiers and animated8×8 atlas,
including signed alpha/size updates, force before damping, and native local space2.
`--particle-frame 30` deliberately overrides rendering with a static diagnostic cell;
`--particle-seed 1`, `--particle-budget 64` and `--particle-owner-budget 32`
control deterministic visual emission. A zero budget drops every birth.
`--particle-stop-after 3` stops at exactly 3 simulation seconds and drains survivors.
Space stops/drains, R restarts, M moves the source, X removes its final owner.
`--particle-move-source` starts the same bounded translation/rotation/scale exercise
as M, making surviving local particles follow the source. On-screen diagnostics
show cells, alpha range and separate authored-cap, visual-budget and stall drops.
The original source cap is25; lower visual budgets still work. Births use the
verified strict accumulator phase; native Euler updates depend on dt partition.
Deterministic RNG, upward birth direction/scatter, full-cell quad geometry, unlit
blending and host job timing remain approximations. Child routing is unimplemented.
Current gameplay fire/smoke and damage are unchanged. Inspect the decoded fields
with `jc2tool particle GAME_DIR [--json]`.

Model viewer controls: left-drag orbit, right-drag pan, wheel zoom, `F` re-frame. `--camera-yaw 0
--camera-pitch 40` starts the camera at a fixed angle (yaw 0 is behind a vehicle), and
`--hide-parts hardtop,door,hatch` leaves doll parts out to look into a cabin.

**Coordinates.** The engine's axes are right-handed and the same as Bevy's — x east, y
up, z south, a model facing −z with +x on its right — so game data goes into Bevy
unchanged; `crates/jc2-game/src/coords.rs` is the one place that says so and every
conversion goes through it (`research/formats/coordinates.md`: the evidence, the jeep's
left-hand drive, and what each system does). Until then the world was drawn mirrored.
World viewer: right-drag look, `WASD` move, `Space`/`C` up/down, `Shift` ×10, `Alt` ×0.1,
wheel changes the speed, `R` back to the start, `B` throws a ball. The sun and moon
follow a clock (`--time HH:MM`, default 10:00; one game minute per real second, `--day-speed`
scales that, `--sun-shadows` casts shadows): `[` and `]` step it by 30 minutes, `T` stops and
starts it (`weather.md`). Tiles stream around
you as you fly or drive, one per frame: `--radius 1` keeps the 3 × 3 around you loaded
(`--radius 2` → 5 × 5, with the pop-in further away), `--no-models` shows terrain only,
`--no-physics` skips the colliders, `--flat-terrain` colours the terrain by height and
slope instead of drawing its materials, `--no-vegetation` leaves out the trees, bushes
and rocks the game grows on every tile (`--vegetation-lod 1` draws them in full detail,
2 is the default; `--bush-density 0.3` keeps a third of the bushes and rocks when the
jungle's five thousand per tile are too many). `--no-grass` leaves out the ground cover the engine scatters around the camera (1 m cells, three
crossed quads per blade, from `grass_types_N.bin`; `--grass-density 0.5` draws half of its four
passes, `--grass-radius 60` shortens the 80 m range). The forests reach the horizon chunks as
tree impostors, two cards per tree cut from `TopAtlas_DIF.dds` / `TrunkAtlas_DIF.dds`: plant
models are hidden beyond ~550 m (bushes ~400 m) and the cards take over, out to
`--impostor-radius 5` tiles (`--no-impostors` keeps every model in view). The sky has a gradient
dome, stars, a textured moon, cumulus and cirrus clouds (`--no-clouds`, `--cloud-cover 0.8`,
`--no-stars`). Debug aids for screenshots: `JC2_START_OFFSET=dx,dz` moves the start from the
tile's middle, `JC2_CAM_PITCH` / `JC2_CAM_YAW` (degrees) aim the free camera (`--no-hero`) and
`JC2_CAM_HEIGHT` (metres above the tile middle's ground, default 160) raises it,
`JC2_NO_DISPLACE=1` draws the terrain without its displacement atlas, `JC2_IMPOSTOR_FADE=0,1`
shows the impostors at any distance; the smoke runs (`--exit-after`) print a frame-time summary.
Tile indices run 0..64 in x and z; `31,14` is the mountain base Lembah Genting Tinggi,
`29,18` the start base Kem Gunung Dataran Tinggi (the registry location
`keymissions\km01\km01.base`, not a tile container), the overview
map (`jc2tool terrain .. --pgm map.pgm`) shows where the land is.
Rico: right-drag (or `Tab` to grab the cursor) looks around, `WASD` walks, `Shift`
sprints, `Space` jumps — in the air it opens and closes the parachute (`W`/`S` dive or
float, `A`/`D` turn) — `F` fires the grappling hook where the camera looks (the hook flies out
of the gauntlet, the reticle goes green over a vehicle, soldier, light prop or ledge, and the
aim is pulled a few degrees towards one) and lets go; reeled to a wall he hangs there (`Space`
leaps off, or climbs a ledge; `Ctrl` drops; `Space` mid-reel opens the chute and keeps the
speed), a soldier or a barrel is pulled to him instead (`character.md`),
`E` gets into a vehicle within reach and out again (with door sounds); hold `H` while driving a car to sound its horn, left mouse fires where the camera
looks (with the cursor grabbed or the right button held), `1`–`5` pick the Beretta,
SMG, shotgun, assault rifle or rocket launcher (`6`–`9` the guns he picks up), `R`
reloads (an empty magazine reloads by itself; the round counter in the HUD shows the magazine), `G`
throws a grenade (10 to start with), `V` drops you into the free camera (`--no-hero`
leaves him out). Walk over a gun to take it: weapon stashes in the villages, and every
soldier drops his.
Stunts: `Q` close to a vehicle (or `F` at it: the hook sticks to it and reels you up) puts
you on its stunt position, pinned to it as it moves — the roof or hood of a car, the deck
of a boat, the top of a plane, under the belly of a helicopter (or at the nose of the
chopper). You crouch once it is quick, can shoot from there, `Space` jumps off with its
speed (`Space` again opens the chute), `Q` steps off, `E` takes the seat, `F` fires the
hook. Hooked from below, a flying helicopter leaves you hanging on the wire under it,
swinging like a pendulum as it turns: shoot, `Space`/`Ctrl`/`Alt` let go, `E` climbs up.
The model tilts toward the wire (an approximation). `Z` follows the first supported
authored stunt transfer, `Shift`+`Z` the last; the jeep can jump from its roof/hood to
front cling and return to its hood or roof. The HUD shows available targets. Transfer
timing and pose are approximate; unsupported actions stay diagnostic.
**Hijacking**: traffic cars have a villager at
the wheel; `E` beside a crewed car or boat — or on any crewed vehicle's stunt position
(a helicopter's pilot only from there) — throws the driver out and seats you
("Hijackings: N"). `--crewed` puts a driver in the vehicles `--heli`/`--boat`/`--plane`/
`--drive` ask for (the helicopter circles its start, the boat cruises), and `--ride
[spot|wire]` puts Rico on the first of them at the start: `jc2 .. --world 31,13 --heli
--crewed --ride wire` (`character.md`, `vehicle.md`). `Ctrl`+`F` fires the
tether gun: the first shot ties one end to what the camera looks at (a car, a prop, a
soldier, the ground or a wall), the second ties the other end and the cable winds the
two together (up to three at once, `X` cuts them all; `research/formats/character.md`).
Driving: `W`/`S` throttle and brake (reverse once stopped), `A`/`D` steer, `Space`
handbrake, `E` gets out, `V` switches to the free camera, `R` puts the car back on its
wheels. Any `lave.*.ee` land vehicle works (`jc2tool list .. | findstr vehicles\lave`).
Flying (`--heli`, any `arve.*` helicopter such as `arve.v060_attackheli.ee`): `E` to get
in, `Space` climb, `C` descend, `W`/`S` nose down/up, `A`/`D` turn, `←`/`→` bank.
Boats (`--boat`, any `seve.*`): swim to it, `E`, `W`/`S` throttle and reverse, `A`/`D`
rudder.
Planes (`--plane`, the speed plane by default, or `arve.v071_jet_fighter.ee`,
`arve.v064_personal_jet.ee`…): `E` to get in, `Space`/`C` throttle up/down, `W`/`S` nose
down/up, `A`/`D` roll, `←`/`→` rudder; roll down a runway past the stall speed the HUD
shows and pull up (`vehicle.md`). The locations park their own vehicles too — the spawn sets of each tile (the
village has an attack helicopter, two trucks, an APC, a jeep); `E` takes any of them,
`--no-traffic` leaves them out. Soldiers stand guard and walk their patrols where the
team spawn sets put them (`--no-npcs` for an empty island) — grunts with pistols,
shotguns and SMGs, now and then an elite with a rifle, as `enemytypespawnsettings.bin`
says; within range and in sight they turn on Rico (or the vehicle he is in) and fire
bursts, pausing to reload, and they go down to his guns (`weapons.md`,
`spawning.md`). Kills and blasts near them raise the heat (HEAT Level 1–5: the minimap glows, a banner slides in):
reinforcements come running from out of view until he has been out of their sight for
a while; from level 2 a jeep of soldiers drives the roads to him and they get out near
him, from level 3 an attack helicopter circles him and fires (`--heat N` starts at that
heat, e.g. `--heat 50`; `spawning.md`). Vehicles have the health from `vehicletypes.bin`: shoot one, rocket it
or grenade it enough and it bursts, throws you out and burns. The tiles' destructible
objects come apart too: red barrels blow up (and set off their neighbours), the watch
tower and the antennas fall to pieces from their own `_dst` debris (`world.md`). Rico has his
400 health from `player.bin`, heals once out of the fight, and gets up at the start when
he drops.
**Civilians and traffic** (`civilians.md`): the towns and villages have people on foot —
each tile's civilian roam areas get up to 12 (`--civilians N`, `--no-civilians`), dressed
in the dolls `worldsim.bin` picks for the ground (fishermen and generic villagers in the
jungle, arctic villagers in the north, city people in the capital), walking about on the
civilian skeleton; shots or a blast within 30 m send them running, and they die to
Rico's guns (a little heat). Cars drive the roads around him (`--traffic N`, 5 by
default; `--no-traffic` leaves them and the parked vehicles out): the AI road graph
`ai\default.rg` shaped by the road splines, the kinds of car by zone from
`worldsim.bin`, keeping to the right, 40–60 km/h, slowing for bends and stopping for
what is in front. `E` hijacks one like any car. Try `--world 32,7` (the capital's
southern district) or `--world 5,41`, `--world 51,10` (villages).
**Chaos** (`chaos.md`): blowing up the government's property pays what
`achievements.bin` says — a gas pump +100 chaos and $500, a generator or a radar +250 and
$1250, a radio mast +1000 and $5000, a red barrel +10 — with the original's CHAOS popup at
the bottom of the screen; a burnt-out military vehicle +50. The resource crates in the
settlements give weapon, vehicle and armour parts or $2500, and every settlement's
sabotage targets and crates make up its "N % complete". The chaos total sets the stage of
the game: every 100 000 points the soldiers get tougher and more of them are elites
(`enemytypespawnsettings.bin`), the heat may climb a level higher (`heatsystem.bin`:
level 2 at the start, 5 from 300 000), and new Black Market items come on sale.
**`K` opens the Black Market** (`blackmarket.bin`; the game pauses): weapons, explosives
and vehicles with their names, prices, stat bars and pictures; `←`/`→` category, `↑`/`↓`
item, `Enter` buys (a gun goes into Rico's arsenal with a full reserve, a vehicle is put
down beside him), `U` upgrades a gun with weapon parts (5, 10, 15, 20, 25 for levels 1–5:
damage, fire rate and magazine from `weapon_upgrades.bin`, a star more on the HUD each),
`K` or `Esc` closes it. `--chaos 200000 --money 100000` starts further on;
`--black-market` opens the shop at the start, `--face-elite` (debug) puts Rico in front of
an elite soldier for a screenshot.
**Mission inspector**: press J, or open the map with M and choose MISSIONS / Tab.
The list shows original mission names, descriptions, faction, unlock conditions and
giver positions. Inspect, Set waypoint, Travel to giver and Start are separate
actions. The map and inspector pause the game and consume movement/fire input.
Only unlocked f2t10 currently offers Start, as a limited mission preview; dialogue,
briefings and scripted warnings remain unavailable. At its loaded giver, E opens
the same inspector using the original Activate prompt and stored interaction box.
Cancel current mission cancels a preparation or run through the shared lifecycle.
Use --open-missions f2t10 for a capture. Direct launch from this panel and skipping
the giver briefing are jc2-rs features, not established original PDA behavior.
See research/formats/mission-browser.md for the supported resource boundary.

**Missions** (`missions.md`): `--mission f2t10` starts the faction task "Offensive Action" at
its restart point (`jc2tool missions ..` lists the others): the mission's own script runs —
its triggers, counters, shape triggers, objective lines, markers, map icons, the employer's
lines as subtitles, the five silos it puts down — through `missionrules.brf`'s objective
chain to the "Faction Mission Completed" banner and `missiondata.bin`'s reward. Fly to the
base (`M` and a click), destroy the silos. **Strongholds**: the nine factions' strongholds
load their bases; destroying their targets fills their "N % complete", and with them down
Rico holds the place for a minute: then it is the faction's (its icon, its soldiers, the
government's gone). **Colonels** stand in the bases and villages with a guard: 500 health,
their own doll, a red icon on the maps; killing one pays "Colonel Assassinated".
The map: `M` opens the PDA's map of Panau (`pda_map_dif.dds`, north up) with the
settlements' icons, Rico's arrow and the vehicles in the world; the game pauses while it
is open. Click anywhere on it to travel there: Rico drops in 250 m above the ground (open
the chute), a helicopter or plane he flies comes along at a safe height, from a car or
boat he goes alone. `M` or `Esc` closes it; `--open-map` opens it at the start
(`map.md`).

**The HUD** (`hud.rs`, `research/formats/hud.md`) is the original's, built from its own art
(`gui\main_menu.blz`): a round minimap top left that turns with the camera (the tiles'
map patches around Rico, the game's icons, a health arc, the alert triangle, a glow and
"HEAT Level N" as the heat rises), the objective line and the stack of notifications
beside it ("N objects destroyed in the last 60 seconds", "N kills …", "Unique vehicles
driven: N", "Locations discovered: N / 368"), a banner on entering a settlement, the weapon
slot bottom right (icon, a round magazine counter with dashes, the name) and the grenade
slot bottom left, key prompts right of the centre on dark pills — **Enter `E`**, **Exit
`E`**, **Open / Close parachute `SPACE`**, **Drop `F`** on the wire, **Jump `SPACE`** and **Drop `LCONTROL`** hanging from the hook, **Stunt jump `Q`**,
**Jump off `SPACE`** on a roof, **Pick up** (walk over it) — with jc2-rs's own keys and
only the ones that apply, world markers (the nearest vehicle within 60 m in green, soldiers
shooting at Rico in red, a chevron at the screen edge when they are off screen), a small
crosshair and a red damage vignette. **`F3`** shows the developer stats (tiles, triangles,
position, speed and the old per-vehicle debug lines); they are off by default.
The HUD's words and numbers are set in the **game's own bitmap fonts** (`gui\fonts\fonts.blz`,
decoded in `jc2_formats::font`: a DXT5 glyph atlas and a glyph table; `jc2tool font ..
[jc2_smallfont_18] [--png out.png]` prints the metrics and renders a sample): the bold
humanist sans for prompts, notifications, names, markers and the HEAT banner, bold digits for the ammo balls, the chrome italic numerals and the
scorpion for the CHAOS popup. Bevy cannot draw bitmap fonts, so `bitmap_text.rs` lays the
glyphs out as `ImageNode`s with atlas rectangles (`BitmapText` component: font, size, colour,
alignment, wrap, shadow; Bevy's default font stands in for a character a face lacks).
`--grapple-demo fly|hang` (debug only) aims at the nearest wall and fires the hook for a
`--screenshot`: the hook in flight (slowed down) or Rico hanging from it. `--hud-demo` (debug only) fakes heat 2, notifications, an objective, a settlement banner, a
prompt and two soldiers' markers a few seconds in, for screenshots; `--hud-demo vehicle`
shows the HUD of a car.

**The asset browser** (`--browse`, `crates/jc2-game/src/browser/`, the logic in
`jc2_formats::browse`) is an egui window (`bevy_egui` 0.42, the release built for Bevy 0.19)
over everything the file system can see. The left pane is a tree of the ~28k files: folders
from the resolved names (or archives first: `pc0` ... `pc4`, `DLC/pc_60`, `dropzone`, then
the folders), the unresolved hashes under `unknown`, bare names under `(top level)`; only
the open part of the tree is built or drawn, SARC containers (`.ee .blz .nlz .bl .fl`) open
into their members, and a background thread reads the start of every file so the kind
filter works on content (not extensions) and the search finds container members. The
filter box takes words (all must match the name, hash or archive: `pda_map`, `parachute
general`, `d0a17bcb`) and the kind menu narrows to one `detect.rs` kind; with either set the
tree gives way to a flat list of matches. The right pane previews by kind: a **texture** at
fit size with its size, format, a mip selector, an alpha toggle and actual size; a **model**
(`.rbm`, or a container with models: a vehicle assembled from its doll, else the LOD level
chosen) rendered in an orbit viewport next to its block types, vertex / triangle counts and
textures; **property files** as the node tree with resolved key names (`#hash` when
unknown), the same text as `jc2tool prop`; **text / CSV / XML** (the first 64 KB);
**Havok packfiles**, **animations and skeletons** and **FMOD event projects** as the
summaries `jc2tool collision / anim / fev` print; **sound banks** as a sample list with a
play button each; a **container** as its member list (click one to preview it); anything
else as a hex dump of the first 512 bytes. Actions: *Copy name*, *Copy hash* (to the
clipboard, and to the log), *Export* (the file as stored, as `jc2tool extract` writes it,
into the folder in the box, `%TEMP%\jc2-export` by default; *Export inflated* removes the
zlib wrapper), *Open in model viewer* (starts `jc2 --model ...`) and, for a location tile
`31x14.nlz`, *Open in world* (starts `jc2 --world 31,14`). Keys: `Up` / `Down` /
`PageUp` / `PageDown` move through the list, `Ctrl+F` focuses the filter, `Esc` clears it;
in the model view left-drag orbits, right-drag pans, the wheel zooms, `F` or a double-click
frames. `--browse-select general.blz/gae06_parachute_dif.dds` (a name, `container/member`
or a hash) opens with that file selected and scrolled to, `--browse-filter parachute` with that
text in the filter box, for screenshots:
`jc2 .. --browse --browse-select lave.v046_buscoach.ee --exit-after 25 --screenshot C:	mp.png`.

Smoke tests: `--exit-after <seconds>` quits cleanly after that many seconds, and
`--screenshot <path.png>` saves the window to a PNG just before it does (once the frame
has rendered; without `--exit-after` the shot is taken after 20 s), e.g.
`jc2 .. --world 31,13 --exit-after 45 --screenshot C:\tmp\smoke.png`.

For faster edit-compile cycles while hacking on `jc2-game`, use the dynamic-linking
feature: `cargo run -p jc2-game -F dynamic -- ..`.

## What's in the data (and what the engine needs to learn to read)

| Container / format | Where | Parsed? |
|---|---|---|
| `.tab` + `.arc` archives, `0x800`-aligned, keyed by `hashlittle(file name)` | `archives_win32\pc0-4`, `DLC\` | yes |
| `dropzone\` flat override folder, same hash lookup | game root | yes |
| SARC small archives (`.ee .bl .nl .fl`) and zlib-wrapped twins (`.eez .blz .nlz .flz`) | everywhere | yes |
| Property containers (`.bin .asb .pfs .blo …`): nested objects of typed values keyed by name hash | settings, world, vehicles, weapons, AI, missions | yes (names ~partly resolved) |
| "PCBB" blackboard property containers (`.epe` exported entities) | inside `.ee` containers | yes |
| DDS textures (DXT1/3/5) | everywhere | via Bevy's loader |
| RBM render-block models (vehicles, props, characters, buildings, vegetation) | inside `.ee`/`.bl`/`.fl`/`.nl` containers | yes, all 13 block types in the data |
| `.lod` descriptors (LOD level → model) | inside containers | yes; shared-part placement still open |
| Terrain: `terrain_win32.dat` overview + 4096 `terrain_win32_XX_ZZ.dat` tiles (heights, gameplay materials, gradient/slot/weight layers, PDA map, mesh data, game zones) | `terrain\` | yes; the overview's tail not yet |
| Terrain materials: `climate\terrain_materials.bin`, `meta_materials.bin`, the zone maps, the `diffuseatlas_N` / `normalatlas_N` atlases and `globaldisplaceatlas.dds`, drawn the way the game's `FastTer_Lod0` shader does | `climate\`, `textures\landscape\` | yes |
| Location containers `locations\wc\XX\XXxZZ.nlz` with `.blo` placements, `.cgd` static geometry, `.pfs/.pfi` collision, `.obc` occluders | `pc0`, `pc2` | yes |
| Vegetation: `climate\zone_N\vegetation_{types,set}_N.bin`, `grass_types_N.bin`, `climate\common\treeimpostors.bin`, `terrain_materials.bin`, `climatezonemap.dds` — the trees, bushes, rocks and grass are *generated* from these, not stored | `pc0` | yes (trees, bushes, rocks, grass, the tree impostors) |
| Shared kits `general.blz` + `areasetNN.blz`, assigned per tile by `areasets.bmp` | `global\areasets\` | yes |
| Vehicle dolls `.mvdoll` (part tree, wheels, suspension, engine, gears, steering) | vehicle `.ee` containers | yes |
| `.aitile` (16,384 of them) navigation tiles for people on foot | `pc0` | later (the roam areas did) |
| AI road graph `ai\default.rg` (3653 junctions, 4168 roads) and the road splines (`CRoadObject` in `locations\roads\roo_*.nlz`, the city's `*.roads.nlz`, the tiles) | `pc0`, `pc1` | yes (`civilians.md`) |
| World simulation `settings\worldsim.bin` (people, cars, boats, planes by zone), `terrain\gamezone.bin`, `CCivilianRoamArea` | `pc0`, tiles | yes, the people and cars (`civilians.md`) |
| ADF (`" FDA"`) effect definitions, `.pfx` collision | `.ee` containers | later |
| Missions: `missions\missions.bmf` (60-byte records), `missiondata.bin` (rewards), `rules\missionrules.brf` (its own tree format: the objective chains), `regions.csv`, the scripts `<id>.blz` → `<id>.bl.bl`, the strongholds' `<id>.base.nlz`, the colonels' spawn sets | `pc0`, `pc1`, `pc2` | yes (`missions.md`, `jc2tool missions`); one mission runs, strongholds and colonels work |
| Text database `text\ms15text.csv` (+ DLC `*_locator.csv`): `#D#key#D#,#D#English#D#,…` rows, 8 languages, ~11 200 keys (`text.md`) | `pc4`, `DLC` | yes (`jc2tool text`) |
| Anark Gameface UI binaries (`.agui` / `.asb`) | `gui\*.blz` | later (replace with Bevy UI) |
| Havok packfiles (`0x57E0E057`): collision `.pfx` shapes + tile `.pfi` instance lists | `.ee`/`.nlz` containers | yes (boxes, spheres, capsules, cylinders, hulls, meshes) |
| Characters: `settings\player.bin`, `exported\cdoll\*.ee` dolls, `grapplingparams.bin` | `pc0` | yes (model, skeleton name, hook data; most numbers unnamed) |
| Havok animation: `.bsk` skeletons, `.ban` spline-compressed clips | `animations\skeletons\`, `global\animations.blz` | yes (`animation.md`); `.afsm`/`.asb` state machines later |
| Sky: `settings\weathersettings.bin` (cloud constants), `textures\atmosphere\skygradient_dif.dds` (sky colour by hour), the sun's orbit from the exe | `pc0` | yes (`weather.md`; sun and sky colour drive the day/night cycle; light strengths are ours; the clouds, cirrus, stars, moon and sun glow are drawn from `weather_dif.dds` and friends, flares and cloud shadows are not) |
| FMOD sample banks `.fsb` (FSB4, MPEG) and FMOD Designer event projects `.fev` (`FEV1` `0x3b0000`: events, layers, envelopes, parameters, sound definitions, waveforms) | `pc0`, `pc2`, `sound\fmod\` | yes (`audio.md`, `jc2tool fsb`, `jc2tool fev`); the events drive the game's sounds |
| Sound ids: `sound\settings_core.bin` (banks = event groups), `settings_smod_*.bin` (the weapon, character, vehicle, environment and music sound modules), `game_materials.bin`, the `Sound` component of `effects\*.bin` | `pc0` | yes (`audio.md`, `jc2tool soundmap`): a `sound_fire_id`, an effect's sound or a footstep names its FMOD event by id |

Name resolution: the archives store only hashes. `jc2-formats` ships ~25.8k known file
names (from the Gibbed.Avalanche project, zlib licence) which resolve about 91% of
entries; `jc2tool harvest` learns more from the data itself (SARC member names and
file references inside property files). Property-key names are the weaker side
(~700 known) — `research/analysis/strings.csv` from the original exe is the best
source for guessing the rest: hash each candidate string and match.

## Roadmap

1. ~~**RBM model viewer**~~ — done, all block types. Open: where a vehicle's shared
   parts (tyres, seats) are placed — the `.mvdoll` names the attachment nodes but the
   transforms live elsewhere (physics data is the suspect).
2. ~~**Asset browser**~~ — done (`--browse`): egui (`bevy_egui`) tree of every archive,
   DLC and dropzone file with containers expanded into members, a name / kind filter, and
   previews of textures, models (orbit render), property files, text, Havok, animations,
   FMOD events and sound banks (with playback), hex otherwise; copy, export, open in the
   model viewer / world. Open: no native file dialog for the export folder (a text box),
   no preview of Anark UI (`.agui`) or ADF.
3. ~~**World**~~ — done: terrain tiles streamed around the player, sea, placed models,
   the `.cgd` static props, the procedural vegetation (trees, bushes, rocks with their
   collision), free camera, the terrain drawn with the game's materials (slot/weight
   layers, slope bands, diffuse, normal and displacement atlases) through a splat shader,
   grass (`grass.rs`), and tree impostors (`impostors.rs`) so the forests reach the
   distance. Open: the global map's extra data (the file's tail), grass LODs and tint, the
   impostors' polygons, normal maps and snow variants. The rest of the island out to the fog
   is drawn from the global height map (1 km chunks, `horizon.rs`).
   A **day/night cycle** runs (`sky.rs`, `clouds.rs`, `weather.md`): the game's sun orbit,
   its sky colour table by hour on a gradient dome, a textured moon with its halo, 1 600 stars,
   cumulus and cirrus clouds from `weathersettings.bin`, dusk and night. Open there: cloud
   shadows, light shafts and lens flares, rain, the game's own light strengths (not found in
   the data).
4. ~~**Physics**~~ — done: Avian3d, terrain height fields, the game's Havok collision
   shapes placed by the tiles' `.pfi` lists (render-mesh trimeshes only as a fallback), a
   raycast car driven from the `.mvdoll` data with its real chassis shape. Open:
   damage/destruction, planes, matching Havok's feel. Helicopters fly (`--heli`) and
   boats float on their dolls' buoyancy points (`--boat`).
5. ~~**Character**~~ — done: Rico's model on the game's skeleton with its clips
   (idle, run, jump, fall, land, hang, parachute), a capsule third-person controller,
   the grappling hook (data from `grapplingparams.bin`), the parachute, in and out of
   cars, a canopy lofted over the game's parachute skeleton that unfolds with its clip.
   Open: the animation state machines (`.afsm`), additive clips, cloth, the hook's
   flight. Pedestrians walk the roam areas and cars drive the road graph
   (`civilians.rs`, `traffic.rs`); open there: the `.aitile` navigation mesh, the
   settlement-kind map (`spawnzones`), traffic lights, lanes by road width, boats and
   planes of the world simulation.
   Stunts: the stunt positions every vehicle doll names (on the roof, deck or fuselage;
   under a helicopter's belly or at its nose), hanging on the wire under a flying
   helicopter, drivers and pilots in the seats the dolls name, hijacking (`crew.rs`),
   and the tether gun that cables two things together.
6. **Gameplay data** — begun: the weapon table (`weapons.bin`: damage, range, muzzle
   velocity, pellets, reload time, the AI's ranges, bursts and magazines; the grenades'
   damage, impulse, radius and fuse), the upgrade table (fire intervals, magazines) and
   `ammunition.bin` (reserves) drive Rico's guns with magazines and reloads, his
   grenades and the soldiers' guns; soldiers are grunts or elites with their class's
   health, weapon and doll (`enemytypespawnsettings.bin`); weapon stashes and dropped
   guns can be picked up; heat rises and sends reinforcements on foot, a car of
   soldiers along the roads and an attack helicopter (`heatspawn.bin`). Hitscan shots, rockets, blasts, health, deaths, respawn, vehicles
   that burn out (`vehicletypes.bin`). Destructible objects (`CDamageControl`, their
   `_dst` debris) work. **Chaos** (`chaos.md`): the activity each destroyed object fires
   (`gsy.activity.typeNN` → `achievements.bin`'s chaos and money), the stages by chaos
   (`aidifficulty.bin`) driving the soldiers' classes and the heat cap
   (`heat_level_chaos_N`), settlement completion (`<id>.count.up`), the resource crates
   (parts, cash), the CHAOS popup. **Weapon upgrade levels** 0–5 (damage, fire rate,
   magazine; the HUD's stars). **The Black Market** (`K`: buy guns, grenades, vehicles;
   upgrade with parts). **Heat events** from `aiheatparams.bin` (a kill, an explosion:
   points, range, delay). Open: the names of the other heat events and how points become
   heat, the bands' boats, vehicle upgrades' effect, the armour parts' health,
   the effect graphs (`effects\*.bin`), saving. **Missions** (`missions.md`): the mission
   table, rewards, the rule chains and the scripts are read; one mission (`f2t10`) runs end
   to end; strongholds can be taken; colonels spawn and pay. Open there: the AI teams,
   cutscenes and the technician escort of the bigger missions, starting from the givers.
7. **UI / audio** — the FMOD sample banks decode and the FMOD Designer event projects
   (`.fev`) are read (`audio.md`). Which event plays comes from the game's own ids
   (`jc2tool soundmap`): a weapon's `sound_fire_id` gives the shot, tail, shell and reload
   (the rifles speed up with their upgrades), an effect file the blast or the grapple, a
   vehicle its engine start, run and stop, the character module a footstep **per ground
   material** (the terrain tile under Rico: sand, soil, gravel, snow, asphalt …; the car
   roof rings as metal), the environment module the ambience of the climate and game zone
   (`--no-ambience`). The events run with their layers crossfaded by rpm/load/speed,
   weighted variations, randomisation, rolloff, fades, **max playbacks with stealing**
   (50 pistol shots start no more than the event allows), a 3D **pan** whose level the
   event's envelopes set, and an **occlusion low-pass** (sounds behind you are dulled).
   **Music**: the adaptive music (14 track groups of undetected / combat / evasion tracks)
   picked by the hour and the chaos and moved by the heat (`--no-music`, `--music-volume`,
   `--volume`). The map (`M`) with the settlements' icons and click-to-travel is in Bevy UI
   (`map.md`). Open: reverbs, distortion and channel-mix DSP, surfaces other than the
   terrain (Havok shapes carry no material), which preset each vehicle uses (the data keys
   them by unexplained hashes), dialogue, the front end, map zoom, the HUD's chaos /
   tutorial / subtitle popups and settlement progress. The settlements' names come from the text database (`text.md`).

Each step is driven by the same loop: find the data with `jc2tool`, find the code that
consumes it in `research/analysis/pseudo` (search `strings.csv` for the file name or a
property name, then follow the xrefs in the Ghidra project), write the Rust reader,
render it.

## Licence

Code: MIT OR Apache-2.0. `crates/jc2-formats/data/*` name lists derive from
Gibbed.Avalanche (zlib licence, see `LICENSE-gibbed.txt`). You need your own copy of
Just Cause 2; no game assets are or may be committed here.

> Public snapshot: research extracted from JustCause2.exe (function/string lists, decompiler excerpts and probes) is kept out of this repository. Native addresses cited in the documents refer to the retail Steam executable.
