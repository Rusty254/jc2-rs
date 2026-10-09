# The PDA map, its icons, and fast travel

What the game's map screen is made of, how a world position lands on it, and what
`jc2-game`'s map (`crates/jc2-game/src/map.rs`, key `M`) does with it. Readers:
`crates/jc2-formats/src/icons.rs`; inspect with `jc2tool icons <game> [--all]`.

## The image

`pda_map_dif.dds` (top level in `pc3`, 2 097 280 bytes): 2048 × 2048 DXT1, no mips. It
covers the whole 32 768 m world at **16 m a pixel**, 32 pixels a terrain tile. Engine
`(x, z)` falls on pixel

```
px = (x + 16384) / 16        (column, left to right)
pz = (z + 16384) / 16        (row, top to bottom)
```

so the image's rows run with engine z exactly like the height maps (`terrain.md`: the
global map's row is `v = (z + 16384) / 64`, a tile file's row is z too).

Checked against `terrain_win32.dat`: with the image box-filtered to 512², "blue
pixel" (B > R + 15) against "height ≤ 201 m" (sea) agrees on **97.0 %** of the pixels
as stored, 63.1 % flipped vertically, 58.6 % flipped horizontally and 65.5 %
transposed. Drawn as stored, the picture is the familiar map of Panau — the desert in
the south-west, the snow mountains in the north — so **north is up with rows running
with engine z**: engine +z is south, +x east. The start base `31x14`
(−451.8, 836.6, −8860.4) is pixel (996, 470), inside tile 31,14's square
(992..1024, 448..480).

Each terrain tile file also carries its own 128 × 128 DXT1 piece of the same picture
(the "map texture patch", `terrain.md`), which is how the in-game minimap streams it at
4 m a pixel. The full-screen map here uses the 2048² image only.

### A note on handedness

x east, y up, z south is a *right-handed* frame, and so is the engine's: the vehicle
dolls (nose −z, steering wheel and "L" doors at −x, left-hand drive like the original's
cars) say the same. `jc2-game` used to mirror z on everything it read, which drew the
3D world as the mirror image of this map (walking left moved the arrow right). Since the
`coords.rs` change the conversion is the identity: Bevy's −z is north (up the map), +x
east (right), and walking left facing north moves the arrow left
(`map::tests::walking_left_facing_north_moves_the_arrow_left`; `coordinates.md`).

## The icon files

All three are property files in `settings\` (`pc0`); key names in `#xxxxxxxx` are hashes
nobody has named yet.

`icon_types.bin` — 73 icon kinds. Per entry: `id`, `type` (the kind number the other
files use), `faction` (0 none, 1 Reapers, 2 Roaches, 3 Ular), `#3acab26e` the texture
(`gui\MiniMapIcons\hud_icon_s_u_village_dif.dds`; the files themselves are top-level
`textures\gui\minimapicons\*.dds`, DXT5 16², 32² or 64², reachable by bare name),
`#fd7350ca` 1 on the "completed" variant (`…_c_dif.dds`: liberated settlement, done
challenge), `#0376c1af` 1 on the regime-held kinds 24–26 (`hud_icon_s_r_*`, "r" for
regime against "u" for the others), `#f1da0260`/`#7a8b3dff` a second "clamp" texture
on objectives and waypoints (drawn at the screen edge when off the minimap, presumably),
`#a0bdc527` on the vehicle and foot markers.

| kind | what | texture (plain variant) |
|---|---|---|
| 1, 3 | faction missions (Reapers, Roaches, Ular) | `hud_icon_me_*` |
| 2 | Kane's briefings | `hud_icon_km_brief` |
| 4 | faction targets | `hud_icon_sh_t_*` |
| 8, 9, 10 | collectables (supply crates, weapon/vehicle parts) | `hud_icon_collect` |
| 15, 52 | objective markers | `hud_icon_objective(_green)` |
| 17, 19–23 | challenge / race checkpoints | `hud_icon_cp_*` |
| 24 | military base | `hud_icon_s_r_military_base` |
| 25 | military airfield | `hud_icon_s_r_airfield` |
| 26 | military harbour | `hud_icon_s_r_harbour` |
| 27 | airfield | `hud_icon_s_u_airfield` |
| 28 | harbour | `hud_icon_s_u_harbour` |
| 29 | city | `hud_icon_s_u_city` |
| 30 | village | `hud_icon_s_u_village` |
| 32 | stronghold (one per faction; the faction-less `hud_icon_sh` is used) | `hud_icon_sh(_ular/_reapers/_roaches)` |
| 33 | waypoint | `hud_icon_waypoint` |
| 34–37, 40–43, 45–49, 51 | car / foot / heli / boat / gun markers in yellow, orange, red | `hud_icon_{car,foot,heli,boat,gun}_{yellow,orange,red}` |

`icons_global.bin` — 899 placed icons: `id`, `#01dfc541` the position (engine axes, a
`vec3`), `#c9f36d94` the kind (0 on settlements, whose kind is in the next file),
`visible` and `enabled` (0 on the settlements: the game turns them on as they are
discovered), `enable_event`/`disable_event`, `#0d9a95f0`/`#12252e33` the events a
settlement fires (`31x14.enter`, `31x14.count.up`), `#483e0726` a count on settlements
(16 at `31x14`: perhaps the things to destroy there), `#71cfa24b` a vec3 (0, 100, 0) on
settlements. 100 icons each of kinds 8, 9 and 10, 84 of kind 31 (`sm…`, side missions,
no icon type), 75 challenges (17), 51 faction missions (1), …

`settlements.bin` — 405 settlements: `id` (the icon's: `31x14`, `14x50base`, `dcs40`),
`#4c4b2806` the kind, `text` the key of the name in the text database (`set_mil_txt_11`,
`set_civ_txt_134`, `f1s07_title_txt`), which is in `text\ms15text.csv`
(`text.md`): `31x14` is "Lembah Genting Tinggi". `jc2tool icons` prints the name
column.

**The join** (`jc2_formats::icons::settlement_icons`): a settlement with an icon of the
same `id` gets that icon's position and the plain texture of its kind
(`texture_for_kind`: not completed, faction 0 if there is one). 370 of the 405 have an
icon: 86 military bases, 13 military airfields, 18 military harbours, 47 airfields, 11
harbours, 11 cities, 175 villages and 9 strongholds. `31x14` is a military base
(`hud_icon_s_r_military_base_dif.dds`) on tile 31,14.

## The map in jc2-game

`M` opens it full screen (Bevy UI): the image letter-boxed to a square of 94 % of the
window's shorter side, north up, the 370 settlement icons at their positions (a dot in
a colour per kind if a texture were missing), the yellow car / helicopter / boat icons
of the vehicles in the world, and an arrow for Rico pointing the way he faces (the
vehicle's nose when he drives; with no Rico the driven vehicle or the free camera).
The larger settlements (cities, strongholds, military bases / airfields / harbours,
airfields, harbours) carry their English name beside the icon, in a font that is 1.25 %
of the map's side; labels are placed greedily in that order and dropped where they
would cover one that is placed, so most of the 195 show (the villages never do; every name shows on hover).
They follow while it is open. Under the cursor: the engine position, the tile, the
ground's height and the nearest settlement (name, kind, id).

While it is open the virtual clock is paused — physics, the soldiers and the vehicles
stop, as the original pauses for the PDA — the cursor is released and visible, and the
keyboard and mouse input of the frame is cleared once the map has read it
(`PreUpdate`, after Bevy's input and UI focus systems), so no gun fires and neither
Rico nor a vehicle nor a camera moves. `M` or `Esc` closes it and puts the cursor back
as it was.

**Fast travel.** A click on the map turns the cursor's place on the image into engine
`(x, z)` (`map_uv_to_world`) and travels there:

* On foot, Rico is put `DROP_HEIGHT` = 250 m above the ground (the detailed tile's
  height there, else the global map's, never below sea level), his velocity, hook and
  chute cleared, so he falls, can open the chute, and lands on tiles that have loaded
  meanwhile.
* In a helicopter or plane, the aircraft goes with him in it, keeping its velocity and
  attitude, at 150 m over the highest ground within 1.5 km (at least 250 m over the
  spot); a plane slower than 1.3 × its stall speed is given that speed along its nose.
* In a car or boat he gets out (it stays behind and leaves with its tile) and goes on
  foot.
* Without Rico (`--no-hero`) the driven vehicle goes (land vehicles 3 m over the
  ground, stopped and upright), else the free camera. The camera keeps its offset from
  whoever travelled.

Both Avian's `Position` and the `Transform` are set, the velocities zeroed where they
should be, and the jump happens before the world's `stream_tiles` runs that frame,
which streams around Rico's transform: the tile under the destination is the nearest
missing one, so it loads that same frame and the old tiles beyond `radius + 1` drop.
The clock stays paused until that tile is in (at most 600 frames), so he can't fall
through ground that isn't there yet. `TransformInterpolation` resets itself on a
transform written in a variable-rate schedule, so nothing is eased across the jump.

Tests: `jc2_formats::icons::tests` (pixel round trip; the join: 31x14 a military base on
tile 31,14 with its texture), `jc2-game` `map::tests` (map ↔ world ↔ Bevy round trips,
headings, the arrow image; the base's icon inside its tile; Rico on tile 31,14 jumps to
33,14: at the target x/z 250 m over its ground, the streamer's focus and loaded tiles
follow, the clock runs again and he falls).

## Open

* Which icons are visible (`visible`, `enabled`, the discovery events) — everything is
  shown.
* Zoom and pan; the minimap in the HUD (from the tiles' 128² map patches).
