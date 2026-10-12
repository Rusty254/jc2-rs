# Particle size unit, fitted quads, emitter shapes, birth direction and velocity units (board task dc95ceb0, 12 October 2026)

Research only (no Cargo, no game launch). Answers gaps 3, 4, 1 and 9 of `effects-pipeline.md` section 11 and the open items
of `particle-consumers-next.md` ("fitted geometry", "source direction"). Evidence: `JustCause2.exe`
(SHA-256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`), `DX10_Shaders_F.shader_bundle`
(SHA-256 `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`), the 2600 retail leaves in
`particles.blz`, and the decompiler export. Probe: `python research/tools/particle_units_probe.py --game <game folder>
--tool <jc2tool.exe> [--pseudo <MAIN>/research/analysis/pseudo/_unnamed]` (exit 0 = every assertion holds; prints the
decoded tables and the effect 35 fixture as JSON). Labels: **native** = read from the executable/shader/data;
**inferred** = deduced, not read; **open** = unresolved (see Gaps).

## 0. Answers

| Question | Answer |
|---|---|
| Size unit | **Half-extent in world metres.** The vertex shader (bundle offset 246068) places a corner at `corner_offset * size`, and a full cell has corner offsets of exactly +-1. The quad edge is therefore `2 * size`, **not** `size`. The old renderer drew every quad half as wide as native. |
| Fitted quads | Each particle is an **8-vertex polygon** (fan of 6 triangles; vertices 8 per particle, indices from 0xa54fd0), not a 4-vertex quad. Its vertices come from per-texture, per-cell tables in the executable. Every vertex has `offset = 2*uv - 1` (cell-relative uv, asserted for all 19 tables), so the polygon is the part of the unit cell square that contains the opaque texels. Drawing the full cell square with half-extent `size` is the same picture except for the trimmed near-transparent texels (alpha mass outside the polygon: 1.5 % for gradient, 0.08 % for fumefx_bomb_01). |
| Velocity, force, damping, size/opacity rates | SI: metres, seconds. `pos += vel*dt`; force adds `F*dt` (m/s^2); damping `v -= v*min(c*dt,1)` (1/s); size modifier `size += c*dt` (m/s of half-extent); opacity `alpha += c*dt` (1/s). Normalised age advances by `dt/lifetime`. |
| Birth direction | **Not +Y.** Direction = row 2 of `Xrot(pitch) * Yrot(yaw)` (see 3), times `speed`; pitch and yaw are random in `base +- variation`. Local +Z at zero angles. The retail grenade uses pitch in `[-pi, 0]`, which is the upper hemisphere (`dir.y = -sin(pitch) >= 0`). |
| Shapes | 0 point, 3 box surface, 4 radial disc/cylinder (xz) with height, 5 sphere shell; 1 and 2 are unimplemented natively (retail uses only 0 (684 leaves), 3 (365), 4 (459), 5 (1092)). |
| Child chain | The "ten string slots" are `linked_system_N` for N = 0, 1, 2, ... **until the first missing one (no limit of ten; up to 33 in retail)**. All linked systems start **at the same moment as the parent** (and recursively start their own links). They are not death spawns or trails. |
| Lifetime variation | `lifetime = clamp(base + (2u-1)*variation, 0.001, 10000)`, so a non-positive base lifetime is legal (30 retail leaves). |

## 1. Size and the fitted polygon (native)

Draw `0x00a5bc30` uploads three float4 per particle: `[pos.xyz, size_x]`, `[colour.rgb, alpha*distance_fade]`,
`[frame, roll, emissive, size_y]`. `size_y == size_x` unless velocity stretch applies (section 6).
Vertex shader (bundle 246068, 3120 bytes), for vertex `k = 4*y+2*z+w` (vertex buffer from 0xa55100: `x=particle`, `y,z,w` bits of k):

1. `cell = round_ne(frame)`; load float `cb2[cell*2 + y]` component `(z,w)` = float `k` of 8 floats of that cell.
2. `i = trunc(f)`, `fr = f - i`; `u = i*0.00097704 - 0.5`, `v = fr*2.00098 - 0.5`, `off_x = i*0.00195408 - 2`, `off_y = fr*4.00195 - 2`.
   Hence `off = 2*uv - 1`; the packed integer is `~1023.5*(u+0.5)` (truncated: plain-square corners decode to -0.0007 / 0.9998, error < 0.002).
3. `ox = off_x*size_x`, `oy = off_y*size_y`; roll `th = particle roll`: `rx = ox*cos(th) + oy*sin(th)`, `ry = ox*sin(th) - oy*cos(th)`.
4. world = `particle_pos + Right*rx + Up*ry` for the default orientation flags (camera right/up). `v=0` (texture top) maps to `+size` up, `u=0` to `-size` left: upright, unmirrored; positive roll turns counter-clockwise as seen by the viewer.
5. Atlas uv = `(col/cols + u/cols, row/rows + v/rows)` with `col = cell mod cols`, `row = cell div cols` (top origin), `cols = b0a2484f`, `rows = e751ee6f` (`cb3[4].xy = 1/cols, 1/rows`).

Tables: 19 constant buffers created in `0xa5b280` from `{data pointer, count of float4}` pairs at `0x00f0f8e8`
(count 128/64/2/64/128/2/2/2/128/2/2/2/2/2/32/32/16/8/2 = 64/32/1/32/64/1/1/1/64/1/1/1/1/1/16/16/8/4/1 cells; the data
sits at `0x00f0f980..`). Index = id from `0xa544e0`, called by `0xa65f20` with the texture name (a 22-entry `{id, name}` table
at `0x00f1f7c0`: muzzleflashes 1, distort_normal 2, fire 3, fumefx_bomb_01 4, gradient 5, particle_blobb 6, spherify_normal and
ring 7, 360_snow 8, snow_bomb/dirt_bomb/blood_bomb 9, grass 10, flare 11, engine 12, chockwave_normal/color 13, metalflake 14,
bulletshells 15, bullethits 16, blood_drops 17, spark_dif 18); **unknown names use table 0, whose cells are plain unit squares**
(the retail grenade leaf `spherify_circular_normal.dds` is not in the list). Mean polygon area per table: 1.00 (table 0),
0.66 (gradient), 0.61 (fumefx), 0.26 (metalflake), 0.10 (bulletshells). Some polygons reach 4 % outside the cell. Name matching
assumes the texture object returns the bare file name (virtual call +0x34 not traced, **open**).

**Implementation recommendation:** phase 1 draws the full cell (uv rectangle as now) with half-extent `size` (edge `2*size`).
Phase 2 (optional, overdraw only) decodes the tables above at start-up from the user's `JustCause2.exe` (fingerprint-checked) and draws the 8-gon.

## 2. Per-leaf data the old simulation ignored (native; hashes verified in the executable's DAT words)

* `particle_lifetime` `bc4e9cf9` + variation `7535093b`: `life = clamp((2u-1)*var + base, 0.001, 10000)`, `u` in `[0,1]`; stored as `1/life` (`0xa5ca60`).
* Initial size `ef05f293 + u*adeb7b1a` (u in [0,1]); colour `(99030548, b3c17b1d, 25e4e2ca)`; alpha `4885935d`; emissive `24683665`.
* Emission window = `system_lifetime` `79c213bb` (0 = unbounded; an instance `duration` parameter overrides it, `0xa6d290`). The emission clock increments
  before the check, so births stop once `elapsed >= window`. Births need `alive < cap`, a camera distance below `d1e88637` (800 m default) and a global particle count < 0x4000.
* Cap `74175c29` (short). Reader tail (`0xa71720` end): `n = (int)(ceil(life_base + 0.25*life_var) / root[+0x154])`; if `0 < n < cap` then `cap = n`; `<= 150` when mesh entries are present, `<= 1000`.
  `root[+0x154]` is divided by the density scale in `0xa6b120` together with the cap, so it is the emission spacing (inferred; its writer is not in the reader).
* A leaf that lacks the shape field `89c42bb5` never reads its emitter node (`0xa71720` condition): the emitter keeps the defaults (spacing 1/30 s, no angles, speed 0).

## 3. Emitter, birth direction and position (native, `0xa5ab90` -> `0xa5a2d0` -> `0xa5a4e0`)

One LCG per emitter (MSVC `rand`): `s = s*0x343fd + 0x269ec3`, `raw = (s >> 16) & 0x7fff`; "signed draw" = `raw*6.103702e-05 - 1` in [-1, 1),
"unit draw" = `raw*3.051851e-05` in [0, 1]. The initial seed is not located (**open**), so tests must use envelopes.

Per birth, draws in this order:

1. pitch `ax = signed*e5c + e54`  (`bedbc04d`, `8391b740`); yaw `ay = signed*e58 + e50` (`815d0180`, `df6a7f80`);
   speed `sp = clamp(signed*e64 + e60, -128, 127)` (`38d18085`, `e4021a11` = property name `velocity`). An instance override <0 falls back to e60/e64.
2. `dir_local = (cos ax * sin ay, -sin ax, cos ax * cos ay)` = row 2 of `Xrot(ax)*Yrot(ay)` (`0xa5a3de..0xa5a40d`: first matrix argument = X rotation, second = Y rotation).
   `Yrot` rows (c,0,-s),(0,1,0),(s,0,c); `Xrot` rows (1,0,0),(0,c,s),(0,-s,c); the engine's sin/cos are the parabolic 0.225 approximation (error < 0.002).
3. velocity = `sp * dir_local * M3` where `M3` is the 3x3 of the source matrix (world modes 0 and 1; row-vector product, rotates local into world) when emitter byte `+0x0d == 0` (always 0 in the export; no writer found), identity (local frame) for space 2. Space 1 then adds the source velocity.
4. one draw (roll rate lerp for particle +0x40..+0x48: `e[0x1c..]*s + e[0x10..]`, shared by 3 axes).
5. Shape offset `S` from `dir_local` (the **un-oriented** direction), switch on `+0xc`:
   * 0: `S = 0`.
   * 5: `S = dir_local * (inner + (outer-inner)*u)` with `inner = c2826ce3`, `outer = 7c01072c` (draw: 1 unit). Zero direction: CRT `rand()` unit vector (not the LCG).
   * 4: `(x, z) = normalize(dir_local.x, dir_local.z)`; `S.x = (inner + (outer-inner)*u1) * x`, `S.z = (inner + (outer-inner)*u2) * z`, `S.y = (0.5 - u3) * height` (`d2cbd704`, property name `height`); draws `u1, u2, u3` in that order; zero direction draws two signed values for x, z first. Defaults inner 0, outer 10, height 10.
   * 3: box with extents `(11b73cc0, d2cbd704, 2a2907f2)`; if exactly one extent is zero and the other two positive: uniform signed draws on the two non-zero axes; otherwise the cube-surface point in the direction of `dir_local` (divide by the dominant |component|); then `S *= extent*0.5`.
6. position `P = M4 * (offset + S)` with `offset = (5c50f180, 90e04ae5 = offset_y, de1afd07)`; `M4` = source matrix (identity for space 2).
   Modes 0 and 1 then add `u * (previous_source_pos - current_source_pos)` (one unit draw): births spread along the source's last step.
7. roll rate / initial roll draws, lifetime (signed draw), size (unit draw), frame variation (integer draw modulo `variation+1`).

Particle update (`0xa62980`, unchanged from `particle-consumers-next.md`) plus: roll `+= rollrate*dt`; the `CParticleVisualRotate` modifier adds spin.

## 4. Lighting and the pixel stage (native)

* `lit = r0 + max(bit6, emissive)*(1 - r0)` per colour channel, no clamp of `r0` (VS bundle 246068); `r0 = cb3[5].xyz + cb3[6].x * sum`. **Producers (native, closes the old Gap 5; JC2 task 0e4e1f8d):** the draw `0xa5bc30` (the non-distortion branch) sets `cb3[5] = (bytes data+0x29c..0x29e * 0.003921569 * 4, bit6 ? 1 : 0)` and `cb3[6] = (bit7 ? 0 : 1, 0, 0, 0)`, and calls `0x95fb60(ctx, system_source, 5.0, 3, 7, 4)` which writes `cb3[7..14]`.
  * Ambient bytes: written every frame for every particle data object by `0xa6fc10` (loop over the table at `0x12b8490`, count at `this+0x28`) through `0xa6e6c0`: `byte = trunc(clamp01(v*0.25)*255)`, `v = 0.75*(cb0[10].xyz + cb0[11].xyz) + 0.75*sun_colour*|y|` where `cb0[10]/cb0[11]` come from `0x93afc0` on the render engine `[0x1191ab4]` (+0x110, +0x120), `sun_colour` is light `[0x11907c4]+0x834`, `+0x18` (`0x4813d0`) and `y` the float at `+0x264` of the landscape object `[0x1191c48]+0x2c` (the main light direction). Constants `0xe30d18` = 0.75, `0xe2599c` = -0.0.
  * Light list: `0x95fb60` walks the frame's record list (`ctx+0xce4`, count `ctx+0xce8`, 0x20-byte records `{pos, radius, colour*fade (cap 8), 0}` from `CLightManager+0x29bc+(frame%3)*0x1c`, filled by `0x91dcc0`, the same first-64 list as `point-light-model.md` section 3), takes in list order up to 4 whose centre is within `radius + 5.0` (strict) of the **system source**, and uploads `{pos, 1/r^2}` + `{colour}` pairs, zero-filled.
  * Vertex term per particle (centre `cb1[3k]`): `sum = sum_i colour_i * saturate(1 - |pos_i - centre|^2 / r_i^2)^4` (the window is squared twice, unlike the meshes' squared window; no normal term).
  * Emissive = leaf `24683665` (root `+0xce`, ushort/65535, default 0, `0xa71720`), copied to the particle (`0xa5a4e0`) and uploaded in `cb1[3k+2].z`.
  Flag bit7 `27230869` zeroes the light sum only; the ambient still applies.
* Colour `c = clamp01(4 * tex.rgb * lit * particle_colour)`, then lerped to the fog colour by `v1.z`.
* Alpha `a = clamp01(tex.a * particle_alpha * distance_fade * near_fade)` (`0.25` in the VS cancels the PS `*4`); `distance_fade = 1 - max(0, (D - 0.8R)/(0.2R))` with `D` = distance from the camera to the system's source position and `R = d1e88637` (skipped when below 0.01);
  `near_fade = clamp01((d - c9a96718) / 5af3683a)` with `d` the particle's camera distance (VS `cb3[4].zw`; 1 when both fields are 0); a vertex with `0.25*alpha*fades < 0.0025` is culled.
* Default blend (bit7 clear): `out = c*a' + dst*(1-a')` with `a' = a^1.18` (dual-source `SRC1_ALPHA`, `INV_SRC1_ALPHA`).
* **Bit7 leaves** (flash, sparks, muzzle flashes): blend state `SrcBlend = INV_DEST_COLOR (10)`, `DestBlend = ONE`, alpha factors ONE (`0x957db0(.., 10, 2, 2, 2)`, `0x933530` field layout): `out = s*(1 - dst) + dst`, i.e. a screen blend. The adjacent pixel shader at bundle 6734720 computes `s = c * 1.35 * a^2` (alpha squared, **not** alpha-weighted linearly); that this blob is `ParticleAdditive` is **inferred** (the bundle index hash was not located). The old renderer's linear `AlphaMode::Add`, white tint and missing `*4` colour are why the flash and sparks look faint.

## 5. Child systems (native, `0xa71720` and `0xa6d290`)

`linked_system_N` (hashes `841408bf, f47c95e6, 02bcc1f0, 00e0c94d, 0f6944c8, 92010226, 84401592, 5de2258b, ee3aea16, e32a9c36, b1334086, b71ac6b5, ...`
checked with `jc2tool hash`) is read for N = 0, 1, ... while present. System start (`0xa6d290`) creates one instance per link with the same source
and the same `duration` argument, immediately, and each link starts its own links in turn. Start-delay fields at instance `+0x14c/+0x150` give a per-instance random
delay `base + signed*range` (>= 0); their hashes were **not located**, so the implementation starts children at 0 until they are (Gap 1). Existing code stops at ten
slots (`CHILD_SLOTS`) and misses `fx_exp_grenade_12/13`; slot order is *slot number*, not the name suffix (`linked_system_5` is `_08`, `_9` is `_07`).

## 6. Velocity stretch and roll (native)

When the maximum stretch `b294c343` (data+0x170) exceeds 1, or flag bit12 `a1028df6` is set, the draw projects the particle velocity to view space `(vx, vy)`.
`size_y = size * min(b8fd2b9e + hypot(vx,vy) * 4f2b9eee, b294c343)`; with bit12 the roll becomes `-atan2(vx, vy)` so the texture top points along the screen velocity.
Retail sparks (`fx_exp_grenade_05`: size 0.02..0.035 m, stretch up to 30) are therefore up to 1 m long streaks, not dots.

## 7. Effect 35 (`grenade_explosion`, default component `fx_exp_grenade_01`) retail fixture

Values below come from the retail leaves through the formulas above; the reference simulation in the probe runs each leaf for 40 seeds at dt = 1/60 with the source static at the origin
(births, growth and spread; no ground collision, no stretch). Test expectations should be envelopes, not exact numbers.

| Leaf | Texture, shape, atlas | Rate/s, cap, window s | Life s | Speed m/s | Half-extent at birth m | Half-extent at t=0.25 / 1.0 (sim) | Max radius at 1.0 (sim) |
|---|---|---|---|---|---|---|---|
| `_01` flash | gradient, 0, 1x1 | 100, 1, 0.1 | 0.2 | 0..10 | **8..12** (edge 16..24) | dead at 0.2 s; at 0.1 s: 9.7..13.7 | - |
| `_02` | fumefx, 4, 8x8 | 1000, 4, 0.2 | 2..4 | 20..60 | 0..0.3 | 1.5..3.1 / 3.0..4.6 | 7.6 |
| `_03` | muzzleflashes, 5, 4x2 | 1000, 2, 0.1 | 0.3 | 0 | 2..2.5 (+15/s) | 5.5..6.0 | - |
| `_04` | bullethits, 5, 4x2 | 1000, 7, 0.1 | 0.1 | 1 | 0..2 (+30/s) | - | - |
| `_05` sparks | bullethits, 0, 4x2 | 9998, 250, 0.1 | 0.001..0.5 | 25..65 | 0.02..0.035 (-0.025/s) | stretched to 30x | 7.7 at 0.5 |
| `_06` | metalflake, 4, 4x4 | 3000, 300, 0.1 | 3..5 | 5..25 | 0.02..0.04 | 0.02..0.04 | 16.5 (gravity -11) |
| `_07` shock | spherify_circular_normal, 5, 1x1 | 9998, 1, 0.1 | 0.3 | 0 | 3 (+35/s) | 11.2 at 0.25 | - (distortion pass) |
| `_08` | fumefx, 4, 8x8 | 1000, 4, 0.1 | 1.5..4.5 | -50..50 | 0..0.5 | 1.1..3.6 / 2.6..5.1 | 6.5 |
| `_09` | snow_bomb, 4, 1x1 | 1000, 25, 1.0 | 1..3 | 4..20 | 0.1 | 0.1 | 17.9, y -17.8 without a ground (Gap 3) |
| `_10` | fumefx, 0, 8x8 | 500, 10, 0.1 | 0.5 | 0..12 | 0..0.2 (+10, +1.7/s) | 1.5..1.8 | - |
| `_11` | metalflake, 5, 4x4 | 9998, 10, 0.1 | 0.05..0.55 | 5..45 | 0.06..0.09 | 0.06..0.09 | 13.7 at 0.5 |
| `_12` | fumefx, 0, 8x8 | 10000, 75, 0.1 | 0.5..1.5 | 28..32 | 0..0.2 | 0.4..0.6 / 1.7..1.9 | 7.1 |
| `_13` | metalflake, 5, 4x4 | 1000, 5, 0.2 | 0.2..0.8 | -12.5..12.5 | 0.02..0.06 | 0.02..0.06 | 6.1 at 0.5 |

Must hold: (1) the flash quad edge is 16..24 m at birth (twice the old value); (2) no smoke leaf of this effect gets more than about 7 m from the origin in the first second
(`_02` reaches 7.6 m, all of it upward; `_08` 6.5 m, almost all horizontal; `_12` 7.1 m horizontal). The reported "smoke rising about 20 m in the first second" is **not reproduced** by the native
formulas, not even with the old straight-up direction (the same modifiers with a straight-up direction give about 3 to 5 m for `_02` and 7.2 m for `_12`), so that symptom has another cause (suspect the matrix scale
of the effect source, a long frame `dt` in the stall path, or the radial-offset misuse of the shape 4 fields: `_02` has inner 0, outer 0.5, height 4); (3) 12 linked systems start together (the old code drops `_12` and `_13`);
(4) `_05` has a lifetime that is non-positive for about half of its particles (clamped to 0.001 s), so its leaf must not be refused; (5) the blend of `_01`, `_03`, `_04`, `_05` is the screen blend of section 4, colour
`(0.423, 0.141, 0.027) * 4` clamped for `_01`.

## Gaps

1. Start-delay fields (instance `+0x14c/+0x150`, `0xa6d290`): hash names/readers not located; stagger of linked systems unknown.
2. `DAT_0108aaf9` gate that enables linked systems in `0xa6d290`: meaning not located (likely a particle quality switch).
4. Texture name -> fit id: bare-name assumption; the `muzzleflashes` table (id 1, 32 cells) does not match a 4x2, 8x4 or 4x8 split of the 512x256 texture (8.6-29 % alpha outside), so id-to-table or cell order is unconfirmed for that texture (gradient and fumefx verified).
5. Soft-particle depth variant selection; identity of the additive blob (inferred). (Lighting inputs `cb3[5]`, `cb3[6]` are resolved, section 4; the exact sun/moon direction stand-in is `|AtmosphereState::sun_dir.y|`, the main light direction object itself was not re-traced.) Open after the lighting port: a hero-mode capture still shows no orange fire body like `reference/effects/grenade-explosion-fireball-lights-wall-orange_y34.jpg`; the grenade light (x8, r16) is black at 0.2 s so the lit period is short, and the 0.1 s flash `_01` was not visible in the snow-forest capture (camera/placement), so the non-native fallback fireball stays.
6. Emitter LCG seed (`+0x4c`) and the CRT `rand()` fallback; velocity-orientation flags (bits 9..11 of data+0x194) and emitter byte `+0x0d`; box-shape (3) draw order of the unit draws.
7. Job dt scaling and publish timing (unchanged from `particle-consumers-next.md`).

## Paired implementation task

Filed on the board (see the task comment for the id): owned files `crates/jc2-game/src/effects/particle_sim.rs` and `particle_render.rs`
(plus the `jc2-formats` particle parser for `linked_system_N`, non-positive lifetime and the new fields). It implements sections 1-7 and the fixture above.
