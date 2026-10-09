# The HUD: art, text and layout

What the game's heads-up display is made of, where the pictures live, and how
`crates/jc2-game/src/hud.rs` (`HudPlugin`) draws it. The original HUD is an Anark
Gameface movie (`main_menu.agui`, not decoded); its vector shapes have no texture, so
those parts are generated in code and everything with a texture uses the game's own.

## Where the art is

The in-game HUD shares one Anark container with the main menu: **`gui\main_menu.blz`**
(`pc4`; its uncompressed twin `gui\main_menu.bl` is in `pc1`). `jc2tool sarc <game>
gui\main_menu.blz` lists 107 members, 82 of them `hud_*`. They load through the asset
source as `jc2://main_menu.blz/hud_tin_dif.dds` (the name hash covers the base name, so
`jc2://main_menu.blz/…` works from anywhere). All are DXT5 with alpha unless noted; most
are white-on-alpha so they can be tinted. `gui\pda.blz` carries four of them again
(`hud_alpha_a`, `hud_sniper_add`, `hud_ch_out_en`, `hud_general_bg`).

Used by `hud.rs`:

| texture | size | what it is | how it is cut |
|---|---|---|---|
| `hud_weapons_a_dif` | 256 × 128 | the **weapon icons**, white | 4 × 4 cells of 64 × 32, row-major: 0 Beretta, 1 revolver, 2 SMG, 3 sawn-off, 4 assault rifle, 5 pump shotgun, 6 sniper, 7 rocket launcher, 8 grenade launcher, 9 machine gun, 10 a second rifle, **11 the fragmentation grenade**, 12 minigun, 13 triggered explosive, 14 a charge, 15 a rocket |
| `hud_weapons_b_dif` | 256 × 32 | more icons (magazines, vehicle guns) | not used |
| `hud_wea_bg_dif` | 128 × 64 | the **slot background**: a dark pill with a glass ball at its right end | ball centre (94, 31), radius 20; the grenade slot mirrors it |
| `hud_mm_frame_dif` | 256 × 256 | the **minimap frame** | the ring is (7, 7)–(146, 147), 13 px thick (outer diameter 139, hole 113); two "hourglass" pieces at x 160–202 and 202–244 (y 8–105) are the corner ears; a green tick at the bottom left, a rounded panel, a quarter disc |
| `hud_tin_dif` | 512 × 128 | the **badge atlas**: round map/world icons | 40 × 40 cells, 12 across: (0, 0) the red ring, (1, 0)–(3, 0) the three faction triangles, (5, 0) chequered flag, (6, 0) red cross, (5, 1) green cross (health cabinet), (4, 2) the green ring, … |
| `hud_alpha_a_dif` | 256 × 256 | misc: objective pill, the red "no" sign, crosshair brackets, a red ring, bars, a progress bar | the small crosshair bracket is (128, 48)–(152, 66); the red target ring (96, 80)–(116, 98) |
| `hud_temp_clamp_dif` | 256 × 32 | **edge chevrons** pointing right | 32 px cells: 0 grey, 1 red, 2 yellow, 3 orange with a bomb, 4 green |
| `hud_heat_dif` | 128 × 32 | **the alert triangles** (yellow, orange, red) and the words "Free space" | 32 px cells 0–2 |
| `temp_star_dif` | 16 × 16 | the upgrade star under a weapon | one lit for the gun, one more per upgrade level |
| `hud_ch_out_en_dif` | 256 × 64 | the flaming **CHAOS** logo of the chaos popup | drawn additively by the game (flames on black): its black is keyed out on load (alpha = the brightest channel) |
| `hud_reward_dif` | 256 × 128 | reward glyphs: `$` (4, 4)–(63, 102), weapon part (pistol + wrench), vehicle part (wrench + gear), armour (shield) | the `$` beside the money |
| `pda_icon_compassnorth_dif` | 16 × 16 | the "N" badge on the minimap ring | top level, `pc0` |
| `pda_map_dif` | 2048 × 2048 | the minimap's picture too | (`map.md`) |
| `hud_icon_*_dif` | 32 × 32 / 16² | the settlement and vehicle icons on the minimap | (`map.md`) |

Present but not used (yet): `hud_chaos_en_dif` (a 128 × 32 dark-red stencil of the word:
the mask the Anark movie fills with fire), the reward atlas's part icons,
`hud_tin_dif`'s other badges (cell (1, 1) — the red scorpion — is used for chaos), `hud_transmission_00…11` (the radio caller portraits),
`hud_sniper_dif`, `hud_dpad_dif` (+ the pad button icons), `hud_bars_dif`,
`hud_mm_black_dif` (the dark glass), `hud_heat_bg/meter/siren/flashlight` (thin vertical
heat meters: red, orange, green gradients), `hud_health_max_dif`, `hud_mm_heat_max_dif`,
`hud_dlc_p00…19` (DLC badges), `hud_flash_dif`, `hud_tut_dif` (tutorial popup),
`hud_warning_dif`, `hud_thruster_meter_dif`, `hud_seq_b_dif`.
`textures\gui\hud\hud_minimap_mask.dds` (`pc0`, 128², a white disc with a soft edge) is the
stencil the original clips the minimap with; `jc2-rs` clips with a rounded
`ImageNode` instead.

### What has no texture

Drawn here instead: the **health arc** (a cyan band on the ring, red for what was just
lost, grey for the rest: `arc_pixels`), the **dashes round the ammo ball** (12 dashes, as
many lit as the magazine is full: `dash_pixels`), the heat **glow** round the minimap, the
**vignette**, the **arrow** at the centre, the green **cross**, the dark **pills** behind
prompts and notifications, the blue notification dot.

### Fonts

`gui\fonts\fonts.blz` (`jc2tool sarc`: 7 entries) holds seven bitmap fonts, decoded by
`jc2_formats::font` (`jc2tool font <game> [name] [--png out.png]` prints the metrics and
renders a sample). A `.font` is, little-endian:

```
u32  dds_size
     a DDS (DXT5, one mip): the glyph atlas
f32  line_gap     added to line_height between lines (-1 in the 12 point face)
f32  spacing      (int)(spacing + 0.5) is added to every advance (2 in the 18 point face)
u16  line_height  baseline to baseline, atlas pixels
u16  first        the code point of lookup[0]
u16  glyph_count
u16  lookup_len
glyph_count x 20 bytes: f32 u0, v0, u1, v1 (the atlas rectangle, normalised)
                        u8 width, height (the same in pixels), u8 y_offset, u8 0
lookup_len x i16:       glyph index of code point first + i, -1 for none
```

Read off the engine (`FUN_0095a470` loads it; `FUN_00553720` measures a string as the sum of
`width + (int)(spacing + 0.5)`; `FUN_00939810` builds the quads: the quad is `width x height`
at the pen and `line top + y_offset`, the pen advances by `width + spacing + the caller's extra
letter spacing`, the next line is `line_height + line_gap + extra line spacing` lower, a code
point outside the lookup draws a default all-zero glyph). There is **no kerning, no bearing
and no separate advance**: the rectangles are tight, the atlas keeps 2-4 px between them.
Every table is consumed exactly, every rectangle lies inside its atlas, `jc2tool font` and
the tests check it.

| file | atlas | line | glyphs | what it is | the HUD uses it for |
|---|---|---|---|---|---|
| `jc2_smallfont_8` | 1024 x 128 | 32 | 277 | the Latin / Cyrillic set **on a dark gradient strip** (every cell is the pill slice with the letter on it), plus vehicle icons, an O2 icon and the trophy medallion | not used (the HUD draws its pills itself) |
| `jc2_smallfont_11` | 256 x 16 | 13 | 12 | `+`, `0`-`9`, and `∞` on the code of `E` | the infinity sign in the ammo ball |
| `jc2_smallfont_12` | 1024 x 128 | 23 (-1 gap) | 321 | regular-weight humanist sans (white in alpha), Latin, Latin-1, Ext-A, Cyrillic, **and the pad button icons** (U+0464..048D, in colour) | the reserve number, "Next Black Market Item" (the references show the bold face for prompts and labels) |
| `jc2_smallfont_14` | 512 x 64 | 64 | 14 | the CHAOS numerals: `+ , 0-9 $` in a chrome gradient with a dark outline (colour in RGB, shape in alpha), and the scorpion badge at U+0464 | the popup's "scorpion +250" and "$ +1250" |
| `jc2_smallfont_16` | 256 x 32 | 21 | 12 | bold digits, `/` and `:` (code 47..58) | the numbers in the ammo and grenade balls |
| `jc2_smallfont_18` | 1024 x 256 | 51 | 272 | the bold humanist sans (Myriad-like), Latin, Latin-1, Ext-A, Cyrillic (white in alpha) | prompts, marker labels, notifications, the objective, weapon and grenade names, the settlement banner, the HEAT banner, the popup's object name, the "Rico is down" message |
| `jc2_subtitle_12` | 1024 x 128 | 25 | 274 | white letters in a dark outline (the outline lives in alpha) | the subtitles; not used by the HUD yet |

The atlases are DXT5 with the glyph shape in alpha and white colour (so a text colour
multiplies in), except as noted. `lookup` is a plain array indexed by code point minus `first`,
no search. Printable ASCII is complete in 8, 12, 18 and the subtitle face; the numeral faces
have what the table above lists. The decoded layout puts a capital `A` 28 px below the line
top in the 18 point face (the line has a lot of room above): `bitmap_text` starts the text
box at the capital's top and ends it under the descenders.

`crates/jc2-game/src/bitmap_text.rs` draws them in Bevy UI: a `BitmapText { font, text, size,
colour, align, shadow, max_width }` component (needs `BitmapTextPlugin`, which `HudPlugin`
adds) on a UI node; `size` is the capital height in reference pixels (`HudFont::native()` is the
original's own size: the game draws its fonts 1:1 on 2561 px wide frames, 1/1.28 of the
2000 px reference frame). A system lays the string out (`layout_text`: advance = glyph width +
spacing, alignment inside the widest line, greedy wrapping at spaces) and keeps one child
`ImageNode` per glyph with `ImageNode::rect` set to its atlas rectangle and `color` to the
text colour, the node's own `width`/`height` set to the text's size so a flex parent wraps it;
a drop shadow is a second, dark copy behind every glyph. A character the font lacks (or any
text, when `fonts.blz` cannot be read) is drawn with Bevy's default font, one `Text` child
each. The developer text (F3) keeps the default font; so does the Black Market screen.

## Words

All from `text\ms15text.csv` (`text.md`); `HudStrings` reads them and falls back on the
same English if the database is missing:

| key | text |
|---|---|
| `hud_button_01 … 36` | the prompt verbs: `02` Enter, `03` Exit, `04` Open parachute, `05` Close parachute, `06` Jump off, `07` Reel in, `08` Drop, `09` Jump, `12` Pick up, … |
| `fe_txt_156` | Stunt jump |
| `gui_txt_59 … 69` | Pistol, Revolver, Submachine Gun, Sawed-Off Shotgun, Grenade Launcher, Assault Rifle, Shotgun, Sniper Rifle, Rocket Launcher, Machine Gun, Fragmentation Grenade |
| `gui_txt_87` | Mounted Gun |
| `gui_txt_134` | `% complete` |
| `act_type_txt_NN` | the CHAOS popup's object names (Gas Pump, Generator, Radar …: `chaos.md`) |
| `gui_txt_249` | New Black Market Items Unlocked |
| `statistic_hud_txt_47` | `%count objects destroyed in the last 60 seconds` |
| `statistic_hud_txt_46` | `%count kills in the last 60 seconds` |
| `statistic_hud_txt_13` | `Unique vehicles driven: %count` |
| `statistic_hud_txt_17` | `Locations discovered: %count / 368` |
| `statistic_hud_txt_02/05/14/16/18/33` | fall kills, perfect kill combo, hijackings, base jump, headshots, fall height |
| `v014_jeep`, `v009_military_helicopter`, … (105 keys) | **the vehicle names** ("Marten Storm III", "UH-10 Chippewa"): the key is the container name without the family prefix and `.ee` |
| `set_mil_txt_N` … | settlement names (`map.md`) |

"HEAT" and "Level N" have no key of their own (the Anark movie composes them), so they
are literals in the code.

## The layout

Measured on the reference screenshots (2561 × 1429, shown here in a 2000 × 1116 frame; all
HUD numbers below are in those "reference pixels", which `hud.rs` turns into `vmin`, so the
HUD scales with the window height):

* **Minimap** (top left): ring outer diameter 213, its top-left corner at (35, 10); the map
  circle inside is 184 across; the corner "ears" (signal icon top left, objective ring top
  right, green cross bottom left, red alert triangle bottom right). The picture turns so
  the camera's forward is up; a small "N" badge rides on the ring where north is.
  Health arc: on the ring at 0.93–0.97 of its radius, from 284° (left, clockwise from the
  top) round the bottom to 110° (right): cyan up to the health fraction, red for what
  was just lost, grey after. In `bq04`: arc cyan from the left to the bottom with the
  red segment and the grey remainder to the right.
* **Heat**: the minimap glows white-blue as the heat rises; for a few seconds black glass
  covers the map with "HEAT / Level N"; the same words, huge and white, slide over the left
  edge of the screen (the "H" and "L" are cut off at first). The triangle at the ring's
  bottom right goes yellow, orange, red.
* **Objective** (top, from x 224): red ring + white text on a dark pill ("Retrieve memory
  card #3"). **Notifications** stack at x 258, y 56, 37 px apart, each 15 px further
  right: a blue pill with a trophy ("12 kills in the last 60 seconds"). **Settlement**
  banner at x 262, y 240 (house icon, name, "0 % complete"); the small panel under the
  ring (icon + "28%") is the stronghold / settlement progress.
* **Weapon slot** (bottom right): the slot background 198 × 99 at the right edge, its
  ball centred (1942, 1031) with radius 31: the round counter, 12 dashes round it; the
  weapon icon left of it (77 wide), six stars under the icon, and the name on a pill
  below (right edge 1972, 31 high). **Grenade slot** (bottom left): the mirror image with
  the count in the ball and "Fragmentation Grenade" on the pill.
* **CHAOS popup** (`reference/ui/chaos-radar-250-money-1250-popup…`, `chaos-gas-pump-plus-100-500…`):
  centred at the bottom, its box (600 × 220, top at 862) holds the logo (305 × 76, centred
  at x 1000), the object's name on a faint pill under it, the scorpion badge and "+250" to
  the right, the `$` glyph and "+1250" under them; jc2-rs adds the "Next Black Market Item"
  bar under it (the original shows that bar in its chaos-progress panel). It fades in and
  holds about 4 s; awards within 2.5 s add up (the screenshots' "+50" for five barrels).
* **Prompts**: dark pills right of the centre: right edge at x 1950, y 474 and 40 px apart
  (white text: "Open parachute SPACE").
* **World markers**: a red ring (green for friendly) 38 px with a dark pill "Guard 5 m" to
  its right; off-screen targets get a chevron on the screen edge pointing at them.
* **Crosshair**: small, at (1000, 548): a bracket and four ticks. Tint the bracket image only; its required `BackgroundColor` stays transparent. Tinting that background too produces a white box despite the correct DDS alpha.
* **Damage**: the whole frame goes red and grey from the edges inwards, fading in about
  three seconds; at low health a heavier dark-red vignette stays.

## Key bindings the prompts show (jc2-rs's, not the original's)

| prompt | key | when |
|---|---|---|
| Enter | `E` | on foot within reach of a vehicle |
| Exit | `E` | in any vehicle |
| Handbrake | `SPACE` | driving a car |
| Open parachute | `SPACE` | in the air (> 0.35 s), or on the wire (the slingshot) |
| Close parachute | `SPACE` | the chute is open |
| Drop | `F` | on the wire (`F` lets go) |
| Stunt jump | `Q` | within reach of a car's stunt point (`Q` climbs onto it) |
| Jump off / Enter / Drop | `SPACE` / `E` / `Q` | standing on a vehicle's roof |
| Pick up | `WALK` | within 6 m of a weapon (there is no key: he takes it by walking over it) |
| `F3` | | the developer stats (tiles, triangles, position, speed, the old debug lines) |

## Data behind the notifications

* Objects destroyed: `StructureHits::destroyed` (buildings, props, barrels) plus every
  vehicle that burns out (`Wreck`), counted over a sliding 60 s.
* Kills: `Arsenal::kills`, the same window. Both announce at 3 and then every 5 (assumed:
  the original's thresholds are not in the data we read; the screenshots show 5, 10, 15,
  20, 25 and 12).
* Unique vehicles: the set of container names Rico has sat in.
* Locations discovered: the first time he enters a settlement's area, which is a disc
  around its icon (130 m villages, 250 m airfields and harbours, 300 m bases and
  strongholds, 450 m cities: assumed, the data has no radius). "N % complete" (banner and
  the panel under the minimap): the settlement's `<id>.count.up` targets and crates done,
  out of those in its location file (`chaos::Progress`, `chaos.md`).
* The CHAOS popup: `chaos::ChaosPopup`, set when a unit with an activity is destroyed.
* The HEAT banner: a rise of `Heat::level()`.

## The minimap's picture

A circular `ImageNode` (rounded corners clip the image; `UiTransform` turns it clockwise by
minus the camera's heading) shows a sub-rectangle (`ImageNode::rect`) of a **patchwork**
(`MinimapAtlas`): the 3 × 3 terrain tiles around Rico, each tile's own 128 × 128 DXT1 map
patch (`TerrainTile::map`, 4 m a pixel, oriented like the 2048² overview: pixel x east, row
z south, checked by correlation against the overview), read on the thread pool as he moves.
Until a patch arrives the overview picture (`pda_map_dif.dds`, 16 m a pixel, decoded once)
fills its place, bilinear. Mid-tones are pulled down (`tone`, gamma 1.35) because the
original's minimap is contrasty. The view is 300 m in radius. `MinimapView::center_px` is
Rico's pixel on the overview exactly as `map.rs` maps it.

## Debug flag

`--hud-demo` (debug only, ignored without Rico) fakes heat 2, three notifications, an
objective, a settlement banner, an "Open parachute SPACE" prompt, two soldiers' markers
(one on screen, one as an edge chevron) and the CHAOS popup of the data's gas pump three
seconds in and keeps them from fading;
`--hud-demo vehicle` shows the car HUD (speed in the ball, the vehicle's name on the pill,
Exit / Handbrake).

## Tests

`font.rs`: a synthetic font parses; every game font has sane metrics, glyph rectangles inside
the atlas and as large as the table says, valid lookups, printable ASCII (text faces) or
their digits and symbols (numeral faces); known glyph metrics. `bitmap_text.rs`: a string's
width is the sum of the advances (scaled with the size, equal to the font's own measure),
glyph offsets, alignment, wrapping, the default-font fallback. Headless: a prompt's text is one
glyph node and one shadow node per visible character, as wide as the font says, and falls
back to the default font without the game data. `hud.rs` unit tests: the prompts for each state; the minimap's rotation follows the
camera's heading and Rico's pixel is `map.rs`'s mapping × 2048; the heat banner rises,
holds and fades; notification stacking and fading; the damage flash; generated art; screen
edge points; the patchwork's geometry. In a headless `MinimalPlugins` app (`hud::headless`):
the prompts for a car in reach, falling, the chute open and sitting in the car; the heat
banner appearing on a rise and fading (and again on the next rise); the minimap's rotation
following the camera's yaw and Rico's map pixel; `F3` showing and hiding the old debug
text; kills / destroyed objects raising their notifications and fading. With the game data
(`JC2_TEST_DIR`; skipped without): the ammo counter following firing, reloads and a gun
change through the real `Arsenal`; the settlement banner and "Locations discovered" on
entering the start base, once.

## Open

Widescreen damage vignette correction (JC2-074,8 October2026): the square generated
image uses explicit `NodeImageMode::Stretch` over its full-screen node. Bevy's Auto
mode had fitted it as a centered720px square in1280x720, leaving vertical edges.
Integrated372-test release/lint/build checks passed; the fresh private world
`--pose down` capture `target/browser-stunts-acceptance10/down-full-viewport.png`
confirms a smooth full-viewport tint. Other image nodes retain their sizing policy.

* The Anark text styles: the glassy gradient pills (`jc2_smallfont_8` carries a pill strip
  in every cell: the original may draw its notification and prompt text on it), per-word font
  and size changes ("4 % complete" has a bigger bold digit; the banners animate their size).
  The font sizes and offsets were matched by eye against the reference screenshots.
* Missions feed the objective line (the script's `CMessageObject`, with a counter's
  "n/max"), world markers (`hud::ExtraMarkers`: the enabled `CTargetIndicator`s, a
  colonel within 40 m) and minimap icons (`hud::ExtraBlips`: objective icons, colonels);
  `missions.rs` draws the mission banner ("Faction Mission Started" / the name on an inner
  panel, 940 reference px wide from y 265, the reward lines under it) and the dialogue
  subtitle itself, with `hud.rs`'s helpers (`missions.md`). The banner's badge (the
  Agency star, the green tick) and the caller's portrait are not drawn.
* The activity messages ("Military communications has been disrupted.", the radio mast's
  banner), the agency / stronghold / faction chaos bars, tutorial popups, subtitles with the caller's portrait, "Checkpoint /
  Saving", the stat popups (fall height, perfect kill combo), black-market banners.
