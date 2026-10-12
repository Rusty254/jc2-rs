# Post-manager defaults, frame clock (dt), meter geometry, ring cadence, set selector (JC2-135)

11 October 2026. Research only: no Cargo, GPU, game launch, runtime edit or asset copy. Closes gaps 1, 2, 3, 5, 6, 7 and 9 of
`night-exposure-next.md` (JC2-124) as far as the binary allows; builds on `post-composite-variant-next.md` (JC2-103), whose
tables (settings writer, lock bytes, `S(0x1b)` cutscene value, constructor values of `S(0x12..0x1a)`) are not repeated.
Addresses are retail VAs of `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`;
shader offsets index `DX10_Shaders_F.shader_bundle` sha256 `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`;
data read: `settings\landscape_effects.bin` (sha256 `43fa8c17...`) and `settings\post_effects.bin` (nothing copied).

Reproduce (86 checks, exit 0; the exe, bundle and, optionally, `jc2tool` are read; the models need nothing):

```
python -B research/tools/post_manager_defaults_probe.py <game folder> [--jc2tool <exe>] [--json] [--allow-skip]
```

The probe pins 26 function bodies by sha256 plus the load-bearing instructions, the float constants of the clock, the call
sites, the block-constructor defaults (decoded from the SSE stores), the shader constant-buffer reads of all 15 composite
shaders and the data values below, and runs fixtures for every model in this brief. Legend: **proven** = read from the
binary or data and pinned by the probe; **inferred** = one unchecked link; **unresolved** = not traced.

## 1. Headline

   float at timer+0x14 of the `CClock` singleton (`[0x118fd24] + 4`, about 224 call sites in the game use it). The one writer is the
   tick `FUN_004e1d90`, called exactly once per frame from the main frame function `FUN_0054e3e0`. Each tick takes the real
   elapsed milliseconds (`timeGetTime`), clamps that one sample to **67 ms**, multiplies by **0.001 and the time scale**
   (`CClock+0x24`, 1.0 unless a slow-motion track or the death sequence changes it) and moves the stored dt **1/11 of the way**
   to that sample (smoothing factor 10, divisor 11). Constructor value 1/60. The pause flag (timer+4, set only by the Lua
2. **The exposure meter geometry of JC2-137 is correct (gap 5, arithmetic proven).** Offsets -0.5 / +1.5 source texels, per-axis
   final weights (0, 0.375, 0.5, 0.125): the first cell is ignored and the centroid is 0.25 cell (6.25 % of the frame) towards
   +x/+y. New facts: stages 2 and 3 (and stage 1 when the frame size is a multiple of 64) read **four exact texel centres**
   (texels 4m+1 and 4m+3 of each 4-block), so bilinear filtering of the R32F maps is irrelevant for them; only the final stage and
   stage 1 at other sizes blend. See section 3.
3. **The composite and HighPass read one ring texture that is rewritten every second frame, and its content on frame 0 is zero
   (gap 7).** Frame 0 therefore multiplies the colour by `A + S7 - 1 = 0` (black) unless a frame was rendered before the first tone
   mapping pass (section 4).
4. **Settings defaults (gap 2).** The hour-block writer overwrites 24 of the 29 settings every gradient update; the other five
   are `S(0x12..0x14)` (constructor 200 / 1 / 12), `S(0x1b)` (grain, 0 or 1 in cutscenes) and `S(0x1c)` (caps flag). Four of the
   block slots (S21..S24) are absent from both shipped hour blocks, so they take the **block constructor defaults
   `FUN_0092d540`: S21 = 0.03, S22 = 0.006, S23 = S24 = 0** (identical to the settings constructor values for S21/S22). The
   key-name mismatch of S21/S23 between the file loader and the block parser (JC2-124) is therefore inert in the shipped data.
   `S21..S23` are consumed by the **MotionBlur** pass `FUN_0096a420`.
5. **Smoothing `P+0xf0` is 0 except in two grade states (gap 2).** Positive writers: the player-state grade `FUN_005c5b10`
   (value 8.0, decays by the game dt when the state ends) and nothing else; the death sequence `FUN_00554b00`, the cutscene
   start `FUN_0088cbf0` and the cutscene end `FUN_008fb3e0` write 0 (section 5).
6. **Height / underwater selector (gap 6, proven).** Set 8 (height) is selected for a camera more than 300 m above
   max(ground, 200 m) with `b = int(3 * clamp((h - 300) / 700))`; set 9 (water) below 200 m with `b = int(3 * clamp((200 - y) / 120))`.
   The blend weight is `w = b / 3` (all ten `max_gradient_n` entries are 3). For set 8 the depth-of-field start `S5` is 150 m at
   `b = 0` and 1500 m at `b = 3`, in both the day and the night pair (section 7).
7. **S0..S3 consumers in the other families (gap 9, proven by shader disassembly).** Every composite family (Scene, DepthOfField,
   HeatHazeDOF, Underwater, with or without EdgeFade / FilmGrain / Stereo, 15 shaders) reads the overlay strengths `S2`, `S3` from
   `cb1[3].xy`; **none reads `cb1[0].y`**, so `S0`/`S1` (HighPass saturation mix and exponent, `cb1[0].xy`) are consumed only by
   HighPass. Stereo variants add only `t9`. The cutscene effect tracks can write any of the 29 settings (section 5).
8. **Gap 1 (alternate-mode flags) stays inferred** (section 6): no new writer of `P[9]` / `P+0x25` was found.

## 2. The frame clock (gap 3)

Objects. `CClock` (`FUN_004cf520`, constructor; destructor `FUN_004db8d0`; vftable 0xf08d68, singleton `[0x118fd24]`,
created by `FUN_005067b0`) contains a timer sub-object at `this+4` built by the IClock constructor `FUN_004e3a40`. The timer
vftable 0xe31848 has three slots: slot 1 = the `timeGetTime` import thunk 0x4e50b0, slot 2 = a local-time getter 0x4e7040. Timer
fields (all proven by the constructor bytes and the tick body):

| timer offset | constructor value | meaning (tick FUN_004e1d90) |
|---|---|---|
| +0x04 byte | 0 | pause flag: getters return 1e-6 / 100000 / the paused time stamps while set (`FUN_004cec90` writes it) |
| +0x08, +0x0c | 60.0, 1/60 | previous fps and dt (copied from +0x10/+0x14 at the start of each tick) |
| +0x10, +0x14 | 60.0, 1/60 | **fps and dt of the game clock** (`FUN_004d3690` returns +0x14) |
| +0x18, +0x1c | 60.0, 1/60 | smoothed real (unclamped, unscaled) fps and dt |
| +0x20 | 1.0 | time scale (= `CClock+0x24`) |
| +0x24 | 10.0 | smoothing factor (divisor 11) |
| +0x28 | 25.0 | fixed-step fps, used only if byte +0x2c is set (constructor 0; `FUN_004ce070` clears it once per frame just before the tick, so the branch is dead in the main loop) |
| +0x30 | 67 | clamp of one elapsed sample, milliseconds |

Tick, per frame (the order of the main frame function is `FUN_004ce070`, then `FUN_004e1d90`, then a deferred call on the timer):

1. `elapsed = timeGetTime() - last` as an unsigned 32-bit difference (wrap handled), `last = now`; the very first tick has elapsed 0.
2. Real clock: `dtReal = dtReal + (elapsed * 0.001 - dtReal) / 11`, `fps = 1 / max(dtReal, 0.0001)` (stored at +0x1c / +0x18).
3. Game clock: `sample = min(elapsed, 67) * 0.001 * timeScale`; `dt = dt + (sample - dt) / 11` stored at +0x14 **without a floor**;
   `fps = 1 / max(dt, 0.0001)` at +0x10. In the fixed-step mode (never used) `dt = (1 / 25) * timeScale`.

Fixtures (probe): the first tick moves 1/60 to 0.0151515; steady 17 ms frames converge to 0.017; a 500 ms hitch moves dt by
(0.067 - 1/60) / 11 only; time scale 0.5 halves the steady dt.

Consumers. The PostEffects task `FUN_0096f360` reads `dt` once at 0x96f40d (mov ecx,[0x118fd24]; add ecx,4; call) and passes the same
value to the tone-mapping pass, the sun-halo apply, MotionBlur (`FUN_0096a420`) and `FUN_00966a40`. `FUN_0093e660` clamps it to
[0.01, 0.5] (JC2-124). Consequences for `auto_exposure.rs` (JC2-137 used `Time<Real>`): use the smoothed value `dt_n = dt_(n-1) +
(min(elapsed_ms, 67) * 0.001 * scale - dt_(n-1)) / 11` (scale 1.0 until slow motion is ported), then the 0.01..0.5 clamp; when the
game is paused through the tutorial pause the value is 1e-6 and the clamp makes it 0.01, i.e. **adaptation keeps running at 0.01 s per
frame**. At 60 fps the two choices differ by well under 1 %; at 144 fps both reach the 0.01 floor.

Time scale writers (proven call sites, semantics of the sources not decoded): the cutscene track key 5 (`FUN_008961d0`), the death
sequence `FUN_00554b00` (writes the state's scale, writes 1.0 on leaving it), `FUN_0089b0c0` (reset to 1.0).

## 3. Exposure meter geometry (gap 5)

(-0.5, -0.5), (-0.5, +1.5), (+1.5, -0.5), (+1.5, +1.5) source texels, in uv units `value / w` and `value / h`. The constructor
`FUN_009432c0` builds the table for the swap-chain size (stage 1, from the arguments of `FUN_009531d0`) and, from the read-back
size of the maps, for 64 (stage 2), 16 (stage 3) and 4 (stage 4); `FUN_0093e660` uploads each table as `cb1[0..3].xy`. The
sampler of the sources is slot 0 with engine filter code 2 (linear, point mip) and CLAMP addressing, slot 1 point (JC2-103
decoding; the 64/16/4 maps and the previous-A texture are R32F).

Arithmetic (probe fixtures, 1280 x 720 and the map chain):

* Target pixel (i, j) of a W-wide target has its centre at `(i + 0.5) * N / W` source texels; the taps sit at that coordinate minus
  0.5 and plus 1.5. For `N / W = 4` (stages 2 and 3) the centre is `4 i + 2`, the taps are at `4 i + 1.5` and `4 i + 3.5` =
  the **centres of texels 4i+1 and 4i+3**: four exact fetches, no blending. Stage 1 at 1280 columns (N/W = 20) gives texels
  20 i + 9 and 20 i + 11, at 1920 columns 30 i + 14 and 30 i + 16; vertical at 720 rows (11.25) blends.
* Mean tap position (a linear test image) equals the target pixel centre plus 0.5 source texel, in every stage.
* Final stage (SampleLumFinal, target 1x1, uv 0.5): taps at `(0.5 + off) * 0.5 + 0.25`, i.e. 4x4-map coordinates 1.75 and 2.75
  per axis, bilinear: per-axis weights (0, 0.375, 0.5, 0.125) (one-hot fixture), centroid 1.75 cells against the map centre 1.5.
  The meter ignores the first row and column of the 4x4 map (the left and top quarter of the frame: a bright left/top quarter
  changes nothing, fixture) and weights the third cell most. The result is then `T` of JC2-124 1.3 (the uniform-0.05 frame
  reproduces 1.432 night / 1.540 day).

What remains unproven: whether the hardware filters the R32F maps linearly for the final stage and for stage 1 at sizes that
are not a multiple of 64 (a driver property; it matters only there), and the size passed to the constructor when the window is
resized (the table is built once). JC2-137's implementation already matches this arithmetic.

## 4. Ring-texture cadence (gap 7)

Proven from `FUN_009432c0`, `FUN_0093e660` and the P init (`FUN_009531d0`): two R32F render
targets `L[0x52]` and `L[0x53]` (both created by `FUN_00950420` with **no initial data**, so D3D10 zero-fills them) with
render-target views `L[4]`, `L[5]`; ring index `L[0x5a] = 0`, first-frame flag `L+0x16c = 1`. The composite and HighPass receive
`*(L + 0x148 + ring * 4)` **at init**, i.e. `L[0x52]`, and keep it. Each frame the pass writes `L[4 + ring]` reading `L[0x53 - ring]`
(the other texture; the very first pass reads the final 1x1 result instead), then flips `ring`.

So the texture the composite reads changes only on frames where `ring == 0` (frames 0, 2, 4, ...), one pass after it is written:
the composite sees A at frame n = the value written by the last even frame before n. Frame 0 reads zero (probe model). With a moving
target the visible value changes on every second frame only (fixture); the adaptation itself still advances every frame
(the odd frames update the other texture, from which the next even frame continues). Visible effect: one to two frames of extra
latency and no change in the converged value. **Inferred:** that a rendered frame exists before the first pass (a loading screen
may run the composite many times with A = 0; the multiplier is `A + S7 - 1`).

## 5. Defaults, locks, smoothing and cutscene overrides (gap 2)

Settings table `S(n)` at `P + 0x44 + 4 n`; P is built by `FUN_009413a0` (JC2-103). Effective values in the shipped game:

| S | source of the value after the first gradient update |
|---|---|
| 0..17 | not written by the constructor; the file loader writes seven of them from `post_effects.bin`, then **every one is overwritten** by the hour block (JC2-124 table) |
| 21, 22 | block constructor defaults 0.03, 0.006 (keys absent from both blocks); equal to the settings constructor values |
| 23, 24 | block defaults 0, 0 (keys absent) |
| 25 | the block value (0.3 night, 0.4 day, overrides) |
| 26 | the block value (0, desert 1) |
| 27 | 0, forced 1.0 at cutscene start, 0.0 at cutscene end (JC2-103) |
| 28 | 0 (caps flag, JC2-103) |

Block constructor `FUN_0092d540` (25 floats): slots 0..5 = 1.0, 6 and 7 = 0.5, 12 and 14 = 1.0, 13 = 0.22, 18 and 19 = 1.0,
20 = 0.006, 21 = 0.03, all others 0 (probe decodes the stores). Slot 3 is parsed and written by nothing. The writer
`FUN_0091d160` writes 24 settings; with the JC2-124 maps S21 comes from block slot 21, S22 from 20, S23 from 22, S24 from 23,
S25 from 19, S26 from 24.

cutscene's saved block): **MotionBlur (`FUN_0096a420`)** reads S21 (strength), S22 and S23 (S23 > 0.0001 selects the second shader of
its pair); strength `2 * S21 * min(1, 1 / (30 dt))` is 0.06 at 60 fps (decoded in `motion-blur-next.md`, JC2-179). S24
has no getter call (only the render-side copy).

Smoothing and locks (all proven by call sites):

* **Player-state grade `FUN_005c5b10`** (called through `FUN_00775480` when the player object at +0x3570 exists; the state is
  `object+0x300`): while the character's health is above 0 and state 1, 2 or 3 it sets `P+0xf0 = 8.0`, forces and locks `S25 = 1.0`,
  `S5 = 200` (state 1) or 450 (states 2, 3), and for states 2/3 also `S4 = 0.5` (blur) locked; in every other state it decays
  `P+0xf0 = max(0, min(P+0xf0, 4) - dt)` and unlocks S25, S5, S4. The meaning of the state values is not decoded (Gap B).
* **Death / game-over sequence `FUN_00554b00`** (switch over the state, `gui.gameovermenu.show` in case 5): it sets `P+0xf0 = 0`, force-writes
  and locks S4 (blur) and S11..S16 (tint RGB, saturation clamp end, offset, slope) every frame from interpolated state values (object
  offsets 0x488..0x4a0, targets 0x39c..0x3f8, 0x3d8.., rate `FUN_004f0a30` with the game dt), writes the time scale `CClock+0x24` from the
  state, and unlocks all of them again in case 5 with a fade through `FUN_00944d40`. Targets and timings not decoded (Gap B).
* **Cutscene start `FUN_0088cbf0` / end `FUN_008fb3e0`:** clears the locks (`FUN_0093b020`), `P+0xf0 = 0`, `S(0x1b)` 1.0 / 0.0 (JC2-103); at
  start it saves the 24 current settings into the cutscene's block object (`FUN_0096b8c0`) and sets the gradient-manager bytes at
  `DAT_01191c48 + 0x1afe` and (`+0x1afd` in `FUN_008961d0`) to 0; at the end `FUN_0091d160` writes the saved block back and the bytes are
  restored. Whether `DAT_01191c48` is the zone manager `Z` whose hour updater (gated by `Z+0x1acc/0x1ace`) these bytes pause is
  not proven (Gap C).
* **Cutscene effect tracks `FUN_008961d0`:** for each index 0..0x1c (29 settings) it evaluates a keyframe curve (`FUN_008ce5b0`) and, if the
  (`P+0x114..0x120`, `FUN_00971bd0`), a depth-of-field/focus set and other camera effects; which track name maps to which index is not decoded (Gap C).

## 6. Alternate-mode flags (gap 1)

JC2-103 found no writer of `P[9]` (dword at P+0x24, byte tested) and `P+0x25` besides the constructor. This pass re-checked the
fifty-one windows after calls of the P accessor `FUN_009260e0` and the three direct engine accesses; none writes either offset (the
`+0x24`/`+0x25` stores that occur are on other objects: the clock at `[0x118fd24] + 0x24`, a render-state pointer
`[0x1196074] + 0x25`, `[0x118fb98] + 0x24`). `FUN_0096f360` passes `P[9] != 0` to the composite as source mode 1 with the
alternate source `P[7]`. **Still inferred:** both flags stay 0, tone mapping runs. A writer reached only through a stored pointer
alias would escape any static scan; a runtime watch of `[P+0x24]` / `[P+0x25]` would close it.

## 7. The set selector (gap 6)

`FUN_00a69830` from the container `#e675b7c8` of `landscape_effects.bin` (the constructor defaults are 1000, 100, 30 and 70 and are overwritten): `#12c1139f` = 30 (`Z+0x1aec`), `#e2ac8f85` = 300 (`+0x1af0`), `#7157f3a4` = 1000 (`+0x1ae4`), `#d91f594f` = 120
(`+0x1ae8`), `#5aa8f383` = 8 (`+0x1ad0`, height set), `#33c95e4b` = 9 (`+0x1ad4`, water set), and the eight zone sets
`gradient_for_zone_00..07`. Selection:

  (0 below 533 m, 1 from 534, 2 from 767, 3 at 1000 m or more); otherwise the set and `b` come from the zone byte (`FUN_00a5eb50`: bits 3..5
  select one of 8 zone sets, bits 6..7 are `b`).
* camera y < 200: set 9, `b = int(3 * clamp((200 - y) / 120))` (0 for a depth under 40 m, 1 from 40 m, 2 from 80 m, 3 from 120 m).
  `w = b / 3`; day pair `lerp(slot 2, slot 1, w)`, night pair `lerp(slot 4, slot 3, w)`; dawn/dusk blends as in JC2-124.
* Set 8 data: slot 1 `#8eba4969` and slot 3 `#de6c5fe4` have `S5 = 1500`, slot 2 `#2e674fd1` and slot 4 `#1eeb7740` have `S5 = 150`. So the
  depth-of-field far start is 150 m, 600 m, 1050 m, 1500 m for `b` = 0..3 (all other fields of set 8 equal the shared default blocks),
  in both pairs. Set 9 (water): all four slots identical, so `b` has no effect on the post settings (it does on the sky lights).
* The same `(set, b, h - 30, depth, two weather values, dt)` tuple is passed to `FUN_00a5d390`, which interpolates the sky/fog gradient
  rows (JC2-123 / JC2-132); it is not part of the post blocks.

## 8. Proven, inferred

Proven (pinned in the probe): everything in the headline except items noted below; the 26 function bodies; the clock constructor,
tick call site, vtable slot; the block constructor defaults; the shader reads of all 15 composite shaders; the data values of section 7.
Inferred: the model of `timeGetTime` as a millisecond tick (import thunk, name from the analysis export); that
the PostEffects task reads the clock value written by the previous frame's tick (order of threads not decoded); the zero content of the
ring texture on frame 0 (D3D10 API guarantee, initial-data pointer 0 proven); the hardware filtering of R32F; the role of the player
state values at `object+0x300`; flags `P[9]`, `P+0x25` off.

## 9. Corrections and notes for earlier briefs

1. JC2-124 / JC2-137: `dt` is the smoothed, 67 ms-clamped, time-scaled clock, not `Time<Real>` (section 2).
2. JC2-124 gap 2: smoothing `P+0xf0` is 0 outside the player-state grade; S21..S24 consumers are the MotionBlur pass; S21/S23
   file keys are never present, so the effective values are the block defaults.
3. JC2-103 section 3 "S(0..0x11) whatever the allocation held": all 18 are written by the first hour-block application, and the
   file loader writes seven of them first; the block default values apply only to keys missing from a block.

## 10. Gaps

Filed as board tasks (blocking visible or gameplay parity): **A** MotionBlur pass and velocity buffer (S21..S23); **B** player-state
and death-sequence grading (`FUN_005c5b10`, `FUN_00554b00`); **C** cutscene effect tracks, saved-block override and gradient-manager
gate. Listed here only:

1. `P[9]` / `P+0x25` writers (section 6): needs a runtime watch.
2. R32F linear filtering and swap-chain size at construction (section 3): needs one retail capture at a known scene.
3. Frame 0 / loading-screen composite with A = 0 (section 4).
4. Thread order of the clock tick against the render-thread read of dt; `FUN_004ce070`'s byte at timer+0x2c and the writer of the
   fixed-step byte (+0x2b/+0x2c; none found, treated 0).
5. S24 consumer (none found), S18..S20 consumers beyond JC2-103.

## 11. Bounded handoff for the runtime

1. `auto_exposure.rs`: replace the delta with `ClockModel::tick(elapsed_ms, scale)` (section 2) and the paused value; keep the clamp.
2. Optional, native cadence: keep the exposure value the composite reads in two slots and refresh the visible one on even frames only
   (section 4); frame 0 black is a loading-order artefact and is not worth reproducing.
3. Meter stays as implemented; add the stage-2/3 exact-texel property to its tests (section 3).
4. `PostEffectBlocks`: block defaults per slot as in section 5 (S21 = 0.03, S22 = 0.006, S23 = S24 = 0) for keys a block lacks.
5. Height/underwater: `b` and `w` as in section 7 once camera height above ground is available. **Done in JC2-183:** `sky_light::update_zone_inputs` overrides the zone set with `height_water_set` (set 8/9, `b` carried in the intensity, so `variant_weight` is `b / 3` for the sky rows and the post blocks alike); the ground height is a bilinear sample of the resident tile's height grid (**inferred** stand-in for `FUN_00a645a0`: native 4 m snapping and units unproven; 0 where no tile is loaded). Still not ported: the `h - 30` and depth arguments, and the rain/snow weights, that `FUN_00a5d390` also takes (they feed the sky/fog rows, not the post blocks).
