# Sun cascade shadows: native set-up (board 78ba941f)

Status: native values found and the transferable part implemented (`crates/jc2-game/src/sun_shadow.rs`).
Evidence: `JustCause2.exe` (image base 0x400000), decompiler listing `research/analysis/pseudo/_unnamed/text_007c0000.c`,
`text_00960000.c`, `text_00930000.c`, `text_00970000.c`, `text_00610000.c`, `text_00420000.c`, and direct disassembly of
`0x007c6ba0` (every constant below was read from its data address in the image).

## 1. The shadow manager

One object (size 0x1ffc, constructor `0x0091a6e0`, reached through `(*DAT_0118fbdc + 0x54)()`). Fields used here:

| Offset | Meaning | Writer |
|---|---|---|
| `+0x00` | tile edge in texels | `0x0096ffea` area: `0x400`, or `0x800` when settings index 0x2a (`/shadowres`, default 0) is non-zero; settings apply in `text_00610000.c` ~line 1700 |
| `+0x08`, `+0x09` | enabled; soft mode | settings: `/shadows` (index 0x18, default 1): enabled when > 0, soft when > 1 (`*(bool*)(m+9) = 1 < quality`) |
| `+0x38` (`[0xe]`) | depth format code `0x37` | `0x0096ffea` area |
| `+0x1bc[3]` | cascade radius `s_i` (half extent of the square box) | `0x00974340(i, value, force)` |
| `+0x1c8[3]` | cull distance `= +0x1d4[i] * s_i`, `+0x1d4 = (0.05, 0.04, 0.02) * /shadowcull*` factors (default 1.0) | same |
| `+0x1f0` | Sun-height clamp (1.0 = inactive; -0.9 when byte `0x0105f7a6` is 0; the image holds 1) | `0x007c6ba0` end |
| `+0x1f4` | shadow floor `f = 0.1` (float `0x00e290bc`) | **written every frame by `0x007c6ba0`** and by the constructor |
| `+0x1f8[3]` | depth bias per cascade | `0x00952b00` |
| `+0x204[3 x 3]` | cascade centres | `0x00923160` |
| `+0x228[3]` | depth range per cascade | `0x00950eb0` |

The atlas (`0x009315f0`): width `N`, height `3 N`, three cascades as vertical tiles, `shadow_buffer`, depth format 0x37.

## 2. Per-frame cascade values (`0x007c6ba0`, the dynamic branch; the debug branch under byte `0x01195904` is not taken: 0)

Inputs: camera position `c = (x, y, z)` from the camera object (`+0x54..+0x5c`) with `y` capped to `max(ground, 200) + 1200`
(`ground` = terrain height under the camera from `0x00a646a0`, 0 without terrain); `f` = camera `-(+0x44, +0x48, +0x4c)` with `y` replaced by 0.0001
and normalised (the horizontal forward); `v = 0x00557050(1)` of the player's character = the character-rule scalar at rules `+0x14`; `h` = capped camera height
minus ground. `v`'s identity is **inferred** as the character's height over the ground: `0x00557050(0xd)` is what `CHeightOverGroundCondition` (`0x006cccb0`) reads
(`foot-conditions-next.md` Gap 2); also consistent with its other readers (parachute alpha, ragdoll thresholds).

| Cascade | radius `s` (m) | centre | range (m) | bias (ndc) |
|---|---|---|---|---|
| 0 | `10 + 20 (clamp(v, 5, 100) - 5) / 95` (10 .. 30) | `c + 0.6 s0 f` | 1250 | `s0 * 0.0045 * 0.0008` |
| 1 | `50 (1 + (clamp(v, 7, 150) - 7) / 143)` (50 .. 100) | `c + 0.55 s1 f` | 1250 | `s1 * 0.0045 * 0.0008` |
| 2 | `110 + 490 (clamp(h, 10, 300) - 10) / 290` (110 .. 600) | `centre1 + 0.5 s2 f` | `1250 (1 + 3 (clamp(h, 200, 1000) - 200) / 800)` (1250 .. 5000) | `s2 * 0.0045 / range2` |

So on the ground (the player's usual state) the three cascades are 10, 50 and 110 m radius boxes at 6, 27.5 and 82.5 m ahead; in the air they grow to 30 / 100 / 600 m.
The box depth is about `range` metres, so the biases are `0.0045 s` metres (4.5 cm .. 2.7 m) for every cascade.

Matrix builder `0x0096c020`: per cascade `eye = centre - sun * range * 4/7`, look-at to the centre (up = +y, or +x when `|sun.y| > 0.9999`), orthographic `2 s x 2 s`,
near 10, far `range`; the projected world origin is snapped to the texel grid (`N / 2` texels across the box) by a translation; the atlas matrix is
`scale (0.5, -1/6, 1)`, translate `(0.5, (i + 0.5) / 3, -bias_i)`. The pixel shader's per-cascade edge fade (edge distance 3.9..5 of 5, i.e. the outer 22 %) and
the hard / soft PCF are in `general-terms-next.md` section 4. The floor is `0.1` of the Sun (confirmed: written every frame, nothing else writes it).

## 3. Implemented

`sun_shadow.rs` (`update_cascades`, run every frame after the camera follow): 3 cascades instead of Bevy's 4, tile size 1024 (the native default; Bevy's 2048 before),
cascade far bounds `centre_distance + 0.78 s` (where the native edge fade starts: 13.8, 66.5, 168 m on the ground; up to about 820 m high up), overlap proportion 0.22,
minimum distance 0.1, and the Sun's `shadow_depth_bias = 0.0045 s0` (Bevy has one bias per light, so the cascade 0 value stands). The filter stays Bevy's Gaussian
(the closest to a 4-tap hardware PCF; the default hard mode has no blocker search). `JC2_NO_NATIVE_CASCADES=1` restores Bevy's split for comparison.
`JC2_FRAME_STATS=1` and `JC2_NO_VSYNC=1` print frame times without the 60 Hz cap (measurement aids).

Measurement (tile 40,40, +200,-300 forest and +0,0 open ground, hero view, 10:00, 640 x 360 gamma luma, 4 x 5 s windows after 10 s warm-up, machine shared with other agents, so
absolute numbers are noisy but the order is stable):

| Scenario | Bevy split (before) | Native set-up (after) |
|---|---|---|
| forest frame time, run 1 | mean 18.2 .. 22.4 ms | mean 10.6 .. 10.7 ms |
| forest frame time, run 2 (order swapped) | 16.2 .. 18.0 ms | 11.1 ms |
| open ground frame time | 30.9 .. 35.3 ms | 21.2 .. 22.9 ms |
| forest luma mean / median / p10 / p90 / p99 | 0.187 / 0.160 / 0.066 / 0.342 / 0.600 | 0.188 / 0.160 / 0.066 / 0.342 / 0.632 |
| open luma mean / median / p10 / p90 | 0.320 / 0.256 / 0.225 / 0.539 | 0.321 / 0.258 / 0.226 / 0.538 |

The look is unchanged (brightness statistics equal within 0.002 apart from the highlight tail); the saving is the smaller map (1024 against 2048 texels, three against four cascades).

## 4. Gaps

1. **Nested light-space boxes.** Bevy fits each cascade to a camera-frustum slice; the native boxes are nested squares of radius 10 / 50 / 110 m (air: 30 / 100 / 600 m) around moving centres with texel snapping, so the shadow texel density
   and the cascade boundaries differ. Reproducing them needs a custom shadow pass or a shader port of `0x0096c020` (visible in shadow sharpness near Rico). Not filed (needs engine work, not research).
2. **Per-cascade bias** (`0.0045 s` m, 4.5 cm / 22 cm / 50 cm on the ground) cannot be set per cascade in Bevy.
3. **Kernel**: hard mode = 4-tap PCF with `cb0[12].w` texel offsets; the value of `cb0[12].w` and whether the taps use hardware comparison are unresolved (`general-terms-next.md` Gap 2); the filter stays Bevy's Gaussian.
4. `v` (`0x00557050(1)`) is inferred to be the character's height over the ground; the writer of rules `+0x14` was not traced. Only affects cascades 0 and 1 while Rico is airborne.
5. The writer of byte `0x0108c681` that the shadow-resolution path may read was not found; 1024 is the setting default (`/shadowres` default 0).
6. The soft mode (quality 2, 12-tap PCF with blocker search) is not offered; the runtime has no shadow-quality option.
