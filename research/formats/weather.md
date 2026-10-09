# Weather, sky and time of day

Reader: `jc2_formats::weather`. In the game: `crates/jc2-game/src/sky.rs`.
`jc2tool weather <game>` prints everything below.

## What the engine has

The world clock is `CWorldTime` (a singleton at `DAT_01191154`): a float **hour 0..24**
at `+0x20`, a time scale at `+0x24` (default 1.0), and a calendar (24 h, 365 days,
12 months, year 2003; it starts at 11:00). Mission scripts set it (`SetTimeOfDay`,
`GetTimeOfDay` in the Lua binding at `0x00a2dc40`; `CSetStartTimeState` and friends
move it with `+= dt` at `0x0082bfe0`). The graphics engine copies it to
`DAT_01191ab4 + 0xac`, and the weather object (`0x00a40000`..`0x00a70000`,
`CRenderBlock{SkyGradient,CirrusClouds,SoftClouds,Stars}`) reads it from there.
Elsewhere in the exe, 5:00 and 19:00 (night for AI), 7:00 and 17:00 (day) and
`(|hour − 12| − 8) · 0.3` (night weight of the clouds, 0 until 20:00, 1 from 23:20) appear
as ad-hoc thresholds.

### Files

| File | What | Used here |
|---|---|---|
| `settings\weathersettings.bin` | nine numbers, the cloud system's constructor arguments | yes (`WeatherSettings`) |
| `textures\atmosphere\skygradient_dif.dds` | 32 × 128 ARGB8888 with mips: the sky's colour | yes |
| `textures\atmosphere\fog_gradient.dds` | 4 × 128: constant colour (255,246,255), alpha falls from 1 to 0 down the rows | read, not used |
| `textures\atmosphere\moon_full_dif.dds` (128 × 128 DXT5: the moon's disc *and* its halo in one picture), `star_dif.dds` (64 × 64 DXT1, one star on black, drawn additively), `sun_dif.dds` (64 × 64 DXT5, the sun's glow) | the moon, the stars, the sun glow | yes (`clouds.rs`) |
| `textures\atmosphere\sun_gradient_dif.dds`, `sunbeam_dif`, `lensflare_dif`, `lens5` | glow gradient, light shafts, flares | not yet |
| `textures\weather\weather_dif.dds` (1024², DXT5: a 4 × 4 atlas of 15 cumulus pictures, the last cell a rain shaft; the alpha is the cloud) | the cumulus cards (`SoftClouds`) | yes |
| `textures\weather\cirrus_clouds_dif.dds` (1024², DXT5: 2 × 2, three cirrus pictures and an empty cell) | the high cirrus (`CirrusClouds`) | yes |
| `textures\atmosphere\precipitation.dds` | rain sprites | no |
| `settings\cloudshadows.bin` | cloud shadow parameters (`cloud_shadow_intensity` 0.85 = `dd25aaff`; the other ten keys unnamed) | parsed, renderer pending |
| `textures\atmosphere\gradients.bmp` | 24 × 22 RGB (one column per hour); an editor image, the exe loads it only if the file exists (`FUN_00a46bd0`, reads pixels into a float table). Its values are not colours as such (the red byte is constant 0x4b, grey `d3d3d3` marks "no data"). Ignored | no |
| `settings\landscape_effects.bin`, `post_effects.bin` | grass/landscape effect tables and the post-processing numbers (`bloom` 15, `exposure` 0.87, `contrast` 1.3, `saturation` 1, `blur` 0.6) | no |

`landscape_effects.bin` has nothing to do with the sky: a dump shows nested tables of
sizes and noise parameters (`5.5`, `18.5`, `12000`…), plausibly wind/grass or landscape
effects; not examined further.

### The sun and moon

fixed unit vector `a = (0, 1, 1) / √2` is rotated about the axis `b = (0, −1, 0.8) / |…|`
by `(hour − 12) · π / 12` (plus an extra angle `DAT_013657e8`, zero, and `b`'s x is
`DAT_013657e4`, also zero). The matrix columns are `a`', `b` and their cross product; the
column with the rotated vector is the direction the weather object uses for the sun
(`+0xdc` negated for the light direction). The sun is called with `hour`, the object at
`0xbc` (moon or stars) with `hour + 12` and the same axis. The cirrus clouds use a
different axis `(0, 0.53, 0.85)` with `hour − 12`.

Consequences (`jc2tool weather` prints the table): at 12:00 the sun is 45° up in the
+z direction (engine axes: x east, y up, z south — `coordinates.md`), at
midnight 32° *below* the horizon on the −z side. It is on the horizon at about 5:40 and
18:20. The orbit is tilted like a latitude of about 50° with a declination near zero, an
equinox sun; the game is not equatorial. The moon is the same orbit twelve hours on.

The row/column-vector convention of the matrix code was not resolved from the
decompilation alone; the sign of the angle was chosen so that the morning sun is on +x
(east) (assumed).

### The sky colour

`CRenderBlockSkyGradient` (`0x00a40000`) writes `hour / 24` (`0x00a40000:4069`:
`*(obj + 0x14) = hour * 0.041666668`) as a shader constant. The sky shader
(`SkyGradientShader`, in the shader bundle) looks `skygradient_dif.dds` up with the hour
across and the view's elevation down: the top rows are the zenith, the bottom the
horizon. Reading the table: columns 0..4 and 27..31 are the night (grey-blue,
(37,47,58) zenith, (78,82,85) horizon), orange at column 8 (6:00) and 24 (18:00) at the
horizon (189,147,100), and 9..23 the day (zenith (41,61,91), horizon (200,218,229)).
Around midday the columns alternate between two slightly different values; the game
filters that away with its bilinear lookup and we smooth it over about an hour.

The table is not dimmed at night (the engine scales it by the sun elsewhere, in the
shader), so the game crate multiplies by its own brightness curve.

### The rest of the lighting: not found

The light the terrain and models receive (sun colour and intensity, ambient) is
computed in the shaders from constants that were not recovered. The weather object holds
six per-hour colour tables of 25 entries × 3 weather variants (`+0x660`, `+0xb10`,
`+0xfc0`, `+0x1470`, `+0x1920`, `+0x1dd0`); they are initialised to 1.0 (`0x00a517f0`) and
lerped between the hour and the next (`0x00a50000:2770`, clouds), and filled from
`gradients.bmp` when present. Nothing in the retail files seems to supply them, so the
game crate invents the sun's colour and strength (below).

## `weathersettings.bin`

One node, nine keys. They are the arguments of the cloud system constructor
(`FUN_00a52d20` → `FUN_00a6f230`), read in this order:

| Key | Name | Value | Role |
|---|---|---|---|
| `#476306ad` | `cloud_base` | 2000 | height of the clouds (m, assumed) |
| `#c4af68f5` | `cloud_height` | 1320 | their vertical extent (assumed) |
| `#21fc2f7c` | `mist_count` | 555 | mist particles |
| `#4f385ead` | `mist_altitude` | 420 | assumed |
| `#b8b11972` | – | 555 | a count (assumed: soft-cloud billboards) |
| `#7ece1f17` | – | 250 | a count (assumed) |
| `#d1e88637` | – | 20000 | extent of the cloud field (m, assumed) |
| `#62279879` | `acceleration` | 1e-6 | stored at `+0x858` (assumed: cloud drift) |
| `#97d853b2` | – | 0.42 | stored at `+0x85c`; missions overwrite it (assumed: cloud cover) |

### How the names were found

Property names are not in the exe as strings (they are hashes computed at build time),
so `strings.csv` does not help. The names `cloud_base`, `cloud_height`, `mist_count`,
`mist_altitude` and `cloud_shadow_intensity` came from hashing short snake_case word
combinations (`cloud`/`mist`/`sky` × `base`/`height`/`count`/`altitude`/…), and
`bloom`, `exposure`, `contrast`, `saturation`, `blur` (`post_effects.bin`) the same way.
Added to `property_names.txt`. The exe's reading order and the values (a 2 km cloud
base, 20 km radius, 555 mist particles) agree with the names.

## What the Rust game does with it (`sky.rs`)

* `TimeOfDay`: hour 0..24, speed in game minutes per real second (default 1 → a full
  day takes 24 minutes), `--time HH:MM` (default 10:00), `--day-speed <factor>`, `[`/`]`
  ±30 min, `T` pauses. The clock is also drawn top right.
* Sun: the exe's orbit, in the engine's axes which are Bevy's (x east, z south): it rises in
  the east (+x) and stands in the south at noon. Illuminance 9000 lux × `smoothstep(−3°, 12°,
  elevation)` (what the world used all along at noon), colour from orange (1, 0.52, 0.25)
  on the horizon to the old warm white above 22°. A fixed-elevation (19°) fill light of
  1200 lux opposite the sun, as the world had, scaled by the daylight.
* Moon: 380 lux, bluish, from the moon's direction, fading in as the sun sets and out
  as it rises. Both discs (440 m / 380 m spheres at 25 km, unlit, no fog) follow the
  camera.
* Ambient: (0.70, 0.80, 1.0) × 140 by day down to (0.32, 0.40, 0.80) × 26 at night.
* Sky and fog colour: the horizon row of the game's table at the hour, smoothed over an
  hour, taken **relative to noon** and applied to the world's old sky colour
  (0.58, 0.72, 0.90), so noon is unchanged and dawn/dusk get the game's orange; blended
  into a near-black blue (0.01, 0.016, 0.04) by `smoothstep(−14°, 8°, elevation)`. Fog
  visibility 12 km by day, 7 km at night; inscattering follows the sky colour.
* Assumed / mine: every light number above, the day/night curves, the disc sizes, the
  fog distances, the moon strength.

### Dome, stars, moon, clouds (`clouds.rs`, `sky_sprite.wgsl`)

* **Dome**: a hemisphere (from −12° to the zenith, 27.5 km, riding with the camera) whose
  vertex colours run from the clear/fog colour at the horizon to a zenith colour: noon's
  (0.16, 0.36, 0.78) scaled by how the game's table at row 0.02 changes with the hour, faded
  to a near-black blue at night; a warm glow around a sun that is within 30° of the horizon.
* **Stars**: 1 600 `star_dif.dds` sprites (2–7 sizes, a few warm or blue) on a 24.5 km sphere
  that turns about the sun's own axis with the hour; added to the dome, fading in as the sun
  sinks below −4° (full at −16°).
* **Moon**: `moon_full_dif.dds` on a card at the moon's direction (replacing the old plain
  disc; the halo is in the texture), 5 200 m wide, reddish when low, faint in daylight. The sun
  gets a `sun_dif.dds` glow added behind its disc.
* **Clouds**: the exe's cloud system (`FUN_00a52d20` → `FUN_00a6f230`, `SoftClouds`,
  `CirrusClouds`) is built from `weathersettings.bin`; here: `cloud_base` (2000 m, absolute
  height above the sea of 200 m) to `cloud_base + cloud_height` (the file's, 800 in the retail
  data, 1320 in the mod), the field 40 km across, cards from the 15 cumulus pictures 2.2 to
  5.2 km wide, `#b8b11972` (555) cards × the cover (default the file's `#97d853b2` = 0.42,
  scaled 2.4 to read as the game's overcast; `--cloud-cover`); a card's colour is the tint
  of the hour (white by day, the sun's colour washed with white when it is low, dark blue by
  night), unlit; the cards drift with a wind (6, 2.5 m/s) and the field wraps around the
  camera in the vertex shader so it has no edge; they fade out near the camera (0.9–2.5 km) and
  at 14.5–18 km. The cirrus is ~30 cards of 14–24 km at 9 km height. Both are alpha-blended in
  vertex order (no sorting) which is fine for the soft sprites.

Not done: cloud shadows (`cloudshadows.bin`), light shafts (`sunbeam_dif`), lens flares,
rain (`precipitation.dds`), lightning, the cirrus' own orbit (`(0, 0.53, 0.85)`), the cloud
lighting from the sun's side, the sun's light colour from the game's tables (not found).
`--no-clouds`, `--cloud-cover`, `--no-stars`; `JC2_CAM_PITCH` / `JC2_CAM_YAW` aim the free
camera's start for screenshots.

## Cloud-shadow checkpoint (8 October 2026)

`CloudShadowSettings` reads the one flat node from `cloudshadows.bin`. It requires
a finite numeric intensity, rejects duplicate intensity properties, preserves the
raw intensity (including mod values outside 0..1), and preserves all other keys and
`Variant` types. It deliberately gives no semantic names to the ten unknown keys.
`jc2tool weather <game>` prints these properties as typed values alongside the
existing sky report; malformed/missing shadow settings do not suppress the report.

The following was extracted from the **plain** Steam install with a private copy of
the previously built `jc2tool`, using `prop <game> cloudshadows.bin --json`. No original
game files or images are committed. Decimal scalars are Float variants in this dump.

| Key | Original type | Plain-game value |
|---|---|---|
| `#005615e6` | Float | 15 |
| `#34408d49` | Vec2 | [0.6, 0.8] |
| `#7e6d3167` | Float | 6000 |
| `#85fb4d69` | Float | 6000 |
| `#8b4f5c68` | Float | 5 |
| `#bad5d30f` | Float | 15 |
| `#bd338c60` | Vec2 | [0, 0.5] |
| `#c55669a6` | Vec2 | [0.9, 1] |
| `#ce82882b` | Float | 5.2 |
| `#dd25aaff` (`cloud_shadow_intensity`) | Float | 0.85 |
| `#f1c8d843` | Vec2 | [0.8, 1] |

`max_opacity(cover)` is an explicitly **assumed renderer policy**:
`clamp(intensity, 0, 1) * clamp(cover, 0, 1)`, returning zero for non-finite inputs.
It is tested as a bound, not claimed as the original shader's intensity formula.
It is not wired to a renderer yet. In the existing sky renderer, zero cover now
skips both cloud fields; previously zero cumulus cover still spawned nine cirrus
cards via `24 * (0.4 + cover)`. Invalid/non-finite overrides also select clear sky.

Verification after merging master `131ace6`: isolation 33, plain-game
`JC2_TEST_DIR`, shared target and shared build lock. Full release workspace suite:
**311 passed** (129 formats + 182 game), matching 311 source test declarations.
Release workspace/all-targets clippy: **no warnings**. The five added weather tests
cover unknown key/type retention, malformed/non-finite/duplicate intensity,
bounded opacity and clear-sky off, the actual original settings, and cloud-cover
normalization. The synthetic fixture writer groups named and hashed properties;
retention assertions therefore compare properties independently of input-list
order. No shader changed and no visual smoke capture was needed for this checkpoint.

### Next graphics agent: sunlight-only cloud-shadow rendering

Evidence and limits:

* Original shader evidence is the `FastTer_Lod0` disassembly already summarized in
  `terrain.md`: t11 is sampled at global world UV `world.xz / 32768 + 0.5` and
  participates in the **sun lighting** term alongside cb0[0]. Ambient/hemisphere AO
  and local lights are separate. The exact map-generation method, cb0[0] layout,
  and intensity transfer remain unresolved. Do not label unknown keys as wind,
  frequency, radius, density or opacity ranges merely from their values.
* Current terrain calls Bevy's `apply_pbr_lighting(pbr_input)` and receives combined
  direct and indirect light. Multiplying base colour, diffuse occlusion, or the final
  colour would also dim ambient, local lights, fill/moon and possibly fog. This
  checkpoint therefore does not add that approximation.
* The top-down original frame `reference/world/daylight-top-down-snow-coast-clouds-from-heli_bu01.jpg`
  has broad blue terrain shading mixed with fog and foreground clouds. It cannot
  isolate a cloud-shadow map or measure the 0.85 setting by itself. Compare with
  `daylight-high-view-clouds-bay-islands-light-shafts_bt01.jpg` and obtain a stationary
  clear/overcast pair before tuning an attenuation curve. Rain reference is
  `civilian-motorbike-headlight-rainy-dusk-overpass_bi07.jpg`.

Version-local integration points (inspect the Cargo.lock version again before coding):

* This checkpoint uses `bevy_pbr 0.19.1`, from the local Cargo registry.
  `src/render/pbr_functions.wgsl:597..635` iterates directional lights, fetches the
  cascade/contact shadow, then accumulates `lighting::directional_light(...) * shadow`
  into direct light. That accumulation is the appropriate semantic hook for adding
  a sun transmittance. Ambient/environment contributions are added separately later.
* `src/render/pbr_types.wgsl` exposes `PbrInput.diffuse_occlusion` and
  `specular_occlusion`; neither distinguishes the sun from other lighting.
  `MaterialExtension` on `terrain_material.rs` selects a custom fragment shader,
  but does not itself insert a callback in Bevy's directional-light loop. Decide
  whether to introduce a narrowly maintained local lighting variant or a shared
  custom renderer path. Budget shader-maintenance and prepass/deferred support.
* Do not assume directional-light index 0 is the sun: `sky.rs` creates Sun, Fill,
  and Moon directional lights. Export an explicit sun identifier/matching contract
  rather than shadowing every directional light. Preserve terrain cascade shadows.
* `terrain_splat.wgsl` uses the custom forward path and a `PREPASS_PIPELINE` branch
  delegating to `deferred_output`. A forward-only implementation needs an explicit
  supported-mode statement and a test that prepass/deferred materials still compile.
* `main_pass_post_lighting_processing` applies Bevy fog after lighting. Cloud
  attenuation belongs before that call. Do not shade haze or sky to fake a shadow.

Implementation contract and acceptance:

1. Use the actual atlas/cloud-card data or a documented deterministic approximation
   to build a world-anchored mask. Camera movement must not drag shadows along the
   ground. Determine whether the original map is camera-relative generated content
   within global t11 rather than assuming the texture itself repeats globally.
2. Keep any assumed wind (existing cards use 6, 2.5 m/s) distinct from the unknown
   settings. Store a bounded offset modulo the chosen field period using f64 CPU
   accumulation and send bounded f32 offsets to shaders; avoid growing
   `globals.time * wind` indefinitely. Test long uptime, wrapping continuity,
   negative-world positions, repeatability and finite output. The existing sky
   cards still use growing shader time; that precision issue remains open.
3. Use `max_opacity` only as a provisional upper bound. Zero cover/`--no-clouds`
   must give exact transmittance 1 with no shadow-map work; invalid settings must
   disable the feature with a useful diagnostic. Preserve ambient, fill/moon,
   point lights, albedo and sky/fog colours. Fade sun attenuation near the horizon
   with a documented assumption until original projection/fade behavior is known.
4. Controlled captures: fixed camera, midday (`--time 12:00 --day-speed 0`), same
   weather seed, no traffic/civilians, covers 0 / 0.42 / 1; then identical scene at
   two elapsed times. Assert no zero-cover darkening, bounded moving shade, no
   camera-relative swimming, seamless tile/horizon boundaries. A dedicated test
   material with only ambient/point lights proves those terms remain unaffected.
   Build and copy a private executable under the shared build lock before capture.

Nearby open graphics research:

* The original shader's per-hour light tables and hemisphere/ambient coefficients
  remain unrecovered; current sun/fill/ambient lux are chosen approximations, not
  global illumination decoded from settings. Probe these independently of shadows.
* `fog_gradient.dds` is decoded but unused; the distance/height meaning of its alpha
  ramp remains an assumption. Existing 12 km day / 7 km night visibility is tuned.
* Rain is the **separate next milestone**. Inspect `precipitation.dds` and shader
  bindings before asserting an atlas arrangement or sprite orientation. Keep
  deterministic camera-local streaks bounded, fade when sheltered/underwater only
  with actual scene evidence, expose a controlled rain amount for captures, and
  leave rain audio to the audio owner. No rain renderer is implemented here.
