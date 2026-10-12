# Grapple tether and hook visuals: cable tube, sag, hooks, cut, effects (board task 5f1b8bdc, "fx-tether")

Research for the original grappling cable and hook (12 October 2026). Probe:
`research/tools/grapple_tether_visuals_probe.py` (50 checks, exit 0; run with `--game GAME --tool jc2tool`).
Read with `effects-pipeline.md` (the effect spawner this builds on), `tether-forces-next.md` (JC2-090: forces and
lifecycle of the wire), `grapple-action-selection-next.md` (JC2-248: `CFireHookAction`), `character.md` (the
`grapplingparams.bin` key table) and `material-parameters-next.md` (the hook mesh material).

Evidence: the retail `JustCause2.exe` (sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`), its
Ghidra export, and the retail files `settings/grapplingparams.bin`, `effects/*.bin`, `weapons.bin`,
`effects/characteffectsettings/ricoseffects.bin`, `animations/eventtracks/grpl_fire.event_trackb`. "native" = read from
the executable or retail data and asserted by the probe; "inferred" is said so. Addresses are retail virtual addresses.

## 0. Answers in one table

| Question | Answer |
|---|---|
| What is the cable? | A **tube**, not a line: eight cross-section vertices (an octagon, circumradius `wire_width * 1.4142 = 0.006 * 1.4142 = 0.008485 m`) swept along a polyline, rebuilt **every frame** as a 0x144-byte renderable (`0x00939130`) and queued (`0x00920500`) by the wire draw `0x007DE2E0`. Textures `Wire_DIF`, `Wire_nrm`, `Wire_mpm` (`textures/wire_*.dds`, same size, 11,064 bytes each in the archive), stored at device `+0x1a4/+0x1ac/+0x1b4`. Section 2. |
| Sag / tension? | A completed two-ended wire is a **parabola of 11 points** (10 segments): `y += ((2t-1)^2 - 1) * (0.02 * length) * 0.5`, so it dips **1 percent of its length** at the middle. The sag amplitude is multiplied by `(1 - elapsed/max)` while the device tension timer `+0x2e8` is set (pull and parachute pull), which is **0 from the first frame** (the timer starts at `max`): taut. Section 2.2. |
| In flight, retracting? | Any other polyline (flight, cut pieces) is passed to the tube builder as it is (no sag). Section 3. |
| The hook? | Two model instances (`hook_model` = `Models/Weapons/GAE04/WEA04-Grapplinghook.lod`, four `hook_texture_N`), device `+0x140/+0x144`, placed at the two ends of the polyline by the wire draw. The hook in flight is **not** an ammunition model: weapon 52 has `model = ""`. Section 4. |
| Cut? | When a wire's strength reaches 0 the device plays `hook_detach` (213) at **both** ends with the end's surface id, plays the `hook_detach_dual` sound (410), stops `dual_hook_deployed` (215), and replaces the wire by **two retracting pieces** that snap back to their anchors at 100 .. 300 m/s. Section 3.2. |
| Which effects exist? | Eight ids in the device effect block (`#ca400f12` of `grapplingparams.bin`): 213, 203, 203, 204, -1, 215, 410, 214; plus the hook projectile impact 212 (weapons.bin) and the left-hand fire sound 11 (`RicosEffects.bin`). Sections 5 and 6. |
| Hook impact? | Effect 212 `hook_impact` is played by the hook's hit slots (world and object) through the same surface-aware path as bullet impacts (`0x005F6660` reads the id from the projectile `+0xc0`). Per surface particle, decal and sound tables in section 5.3. |

## 1. Data (native)

### 1.1 `settings/grapplingparams.bin` (loader `0x00802EC0`)

Visual keys (names recovered by hashing; the rest is in `character.md` and `tether-forces-next.md`):

| Key | Value | Consumer |
|---|---|---|
| `wire_width` (`e6aaf3be`, key word `0x00EBA23C`) | 0.006 (loader default 0.01, `0x3c23d70a`) | octagon profile, section 2.1 |
| `#96e5b772` / `#e6a244b4` / `#0e62e301` (`0x00EBA22C/30/34`) | `Textures/Wire_DIF.dds`, `Wire_nrm.dds`, `Wire_mpm.dds` | device `+0x1a4`, `+0x1ac`, `+0x1b4` (shared pointers) |
| `hook_model` (`4e3f0a7d`, `0x00EBA238`) | `Models/Weapons/GAE04/WEA04-Grapplinghook.lod` | loaded twice (`0x008701B0`) into device `+0x140`, `+0x144`; the file and `Hook_Diff/Nrm/Mpm/Dummy.dds` are in `general.blz` (see `character.md`) |
| `hook_texture_0..3` (built with `sprintf`, hashes `2b8dfb8d cad60785 442da943 ae2d91e0`) | `Hook_Diff.dds`, `Hook_Nrm.dds`, `Hook_Mpm.dds`, `Hook_Dummy.dds` | device `+0x14c..` pairs (probe asserts the four hashes) |
| `#ca400f12 { ... }` | the effect-id block, 1.2 | `0x0059CF70` into device `+0x218` |

### 1.2 The effect-id block (native: reader `0x0059CF70`, controller creation `0x007F55D0`)

The loader passes the block to `0x0059CF70`, which stores eight ids at byte offsets 0x00..0x1c of a struct at **device `+0x218`**
(`-1` default); `0x007F55D0` then creates, for offsets 0x04..0x1c whose id is not -1, a persistent **controller**
(`0x0062A7A0`, the handle getter of `effects-pipeline.md` section 5.3) stored at `struct + 0x20 + 8 * (offset/4 - 1)`.
Offset 0x00 has no controller: it is the id of a fire-and-forget one-shot.

| Struct offset | Key | Id | Effect file | Use |
|---|---|---|---|---|
| 0x00 | `#6bcc7c5a` | 213 | `hook_detach` | one-shot at a wire end (cut, detach), section 3.2 |
| 0x04 (slot 0) | `#7deaffa6` | 203 | `char_reelin_sparks` | looping sparks, started by the reel start `0x008233C0` |
| 0x08 (slot 1) | `#f538a02d` | 203 | `char_reelin_sparks` | looping sparks, started by the attach impulse step `0x00740260` (also called from `0x0055CDC0`, `0x005A9950`, `0x005EA6E0`) |
| 0x0c (slot 2) | `#7c387bc0` | 204 | `char_slingshot_sparks` | looping sparks, started by the parachute controller `0x005A9950` (slot 6) |
| 0x10 (slot 3) | `#0820ebcd` | -1 | none | `CFireHookAction` update `0x0061D6B0` plays it twice; with -1 it does nothing |
| 0x14 (slot 4) | `#bc79634f` | 215 | `dual_hook_deployed` | played at tether completion `0x00538470`; stopped at the cut |
| 0x18 (slot 5) | `#adb6d249` | 410 | `hook_detach_dual` | played at the cut |
| 0x1c (slot 6) | `#db9d6690` | 214 | `hook_miss` | played by the device detach `0x005A0530` |

Helpers (all native, probe asserts the byte patterns and every call site): `PlaySlot(i)` `0x00735700` plays the controller
of slot `i` through `0x0072BEB0(0)` (= `Play(interactions = none, restart = 0)`, effects-pipeline section 5.4) **only if it is not playing**
(`IsPlaying` `0x005141F0` = `[controller + 0x10] != 0`, the state word); `StopSlot(i)` `0x005E9670` stops it (`0x00544950(0)`, graceful)
only if it is playing; `PlayOneShot(field, pos, surface)` `0x005775A0` plays the id of struct field `field` through the interaction wrapper
`0x0077F940` with `matrix = translation(pos)` and one interaction entry `{kind 1, uid = surface}`.

Call sites (probe): slot 0 played by the reel start (called from `CReelInAction` slot 7 `0x006B8890`, state 0), stopped by `0x006B8890`
at the end of the reel and by the cancel `0x005FBCB0`; slot 1 played by the impulse step `0x00740260`, stopped by the cancel; slot 2
played and stopped by the parachute controller `0x005A9950` (and stopped by `0x005FBCB0`, `0x006CAE80`, `0x00706C30`); slot 3 stopped by `0x005FBFF0`;
slot 4 played at `0x00539762` (end of the tether completion `0x00538470`, after the attach impulses and the `ply.doubletether.*` events),
stopped in the device update `0x0083FE20` at `0x00840A36`; slot 5 played in `0x0083FE20` at `0x00840A80` when it is not already playing;
slot 6 played by the device detach `0x005A0530` at `0x005A05B0`; one-shot 213 played by `0x005A0530`, `0x007504B0` and the cut path.

## 2. The cable (native: `0x007DE2E0`, `0x00802EC0`, `0x00939130`)

### 2.1 Tube profile and renderable

The loader pushes **eight** `vec3` into the device vector at `+0x1e8` (`0x0063DEB0`), `r = wire_width * 1.4142` (the constant at `0x00E49C44`):
`(r,0) (w,w) (0,r) (-w,w) (-r,0) (-w,-w) (0,-r) (w,-w)` with `w = wire_width`. All eight lie on one circle of radius
`r = 0.0084852 m` (a regular octagon, 1.7 cm across). `0x007DE2E0(polyline, flag)` builds a **new** tube object per call:
`0x00939130(point_count, profile_count)` (0x144 bytes), texture `device+0x1a4` (`0x0061BCF0`), profile and points copied
(`0x005CD8A0`), then two floats are written at `+0xf8` and `+0xfc` (**10.0** and **3.0**, the initial value of two function-local
statics set from `.rdata` `0x00E251E8`/`0x00E23AD4`; the constructor's own defaults are 3.0 and 1.0) and the object is queued with `0x00920500`.
Nothing is cached between frames. The meaning of the two floats (probably texture tiling along and around the tube) is not decoded (Gap 1).

### 2.2 Two paths

`0x007DE2E0` takes the general path when the polyline does not have exactly two points, when the device field `+0xc` is 1, or when its second argument is NULL;
the general path builds the tube from the polyline as given and (device `+0x140` set) positions hook model 0 at the first point with a basis from the direction of the first segment.
The **sag path** (exactly two points, device `+0xc != 1`, second argument non-null) replaces the two points by an 11-point polyline:

```
d = P1 - P0;  L = |d|;  amp = L * 0.02            (.rdata 0x00F09268)
if device+0x2e8 (tension timer set):  amp *= (1 - device+0x2ec / device+0x2f0)
for i in 0..=10  (10 segments, 0x0106D6B8):  t = i / 10
    point_i = P0 + d * t;   point_i.y += (((2t - 1) ^ 2) - 1) * amp * 0.5      (pow with exponent 2.0 at 0x00E4C9A0; 0.5 at 0x0106D6B4)
```

So a 100 m wire dips 1 m at the middle (world Y, downwards) and the ends do not move. Callers that pass the sag flag (the second argument is the wire's own `+0x40`,
always non-null): the completed wire `+0x128` (`0x0084149B`, `0x00718CF4`) and (inferred from the loop over the list `+0x120`) every stored tether (`0x00840FA0`);
callers that pass NULL (general path): the in-flight wire `+0x12c` (`0x0084151B`, `0x00655660`), the cut pieces (`0x00840105`, `0x008403C8`) and the single dropped
record (`+0x13c`, section 3.1). `device+0x2e8 / +0x2ec / +0x2f0` are set by `0x0073DFE0(duration)` (`+0x2e8 := 1`, both floats := duration, only when it was clear), called
from the parachute controller `0x005A9950` and the pull start `0x007AE2E0`; the frame update then adds the frame delta to `+0x2ec` and clamps it to `+0x2f0`
(`0x0083FE20`, last block before the final call `0x006A1750`). As read, `elapsed = max` from the first frame, so the factor is exactly 0: **while a pull or the parachute pull is active the wire is a straight line**.
`0x005FBCB0` (cancel) clears `+0x2e8`. (Inferred: the timer was meant to relax the sag over a duration; the code as shipped never changes the factor from 0.)

## 3. Wire life cycle as seen

### 3.1 In flight and after a missed or dropped hook

`0x0083FE20` (device update, every frame, `dt`) keeps: a list at `+0x130/+0x134` of **retract records** (0x7c bytes, constructor `0x0061B770`), a single record at `+0x13c`
(the same class), a wire object at `+0x12c` (updated through its vtable slot 3, i.e. `CGrapplingWire::Update` `0x006A72E0`, then drawn with the general path; inferred to be the in-flight hook wire), the completed wire at `+0x128`, and
the stored tethers in the list at `+0x120`. A retract record holds an anchor (an end record, `0x0083BC60` copy), a unit direction and two lengths `total` (`+0x74`) and `len` (`+0x78`) and two speeds
**`+0x00 = 300.0` and `+0x04 = 100.0`** (`.rdata 0x00E38304`, `0x00E22EE8`). Each frame:

```
len -= ((len / total) * (100 - 300) + 300) * dt        # 100 m/s at the full length, rising linearly to 300 m/s as the piece shrinks
polyline = [anchor_world, anchor_world + dir * len]    # drawn without sag
remove the record when len <= 0.001
```

(The first loop also sets the hook models' flag words `|= 0x11` while a piece is alive and `&= 0xffffffee` when none is, i.e. the hook model instances at `+0x140/+0x144` are shown only while there is something to draw; inferred from the flag
writes, native for the writes.) A 10 m piece shrinks to nothing in about 0.07 s at 60 Hz (probe model).

### 3.2 Cut (`strength <= 0`, in `0x0083FE20`, one pass after the strength reaches 0)

Native sequence for a wire with at least two end records (`A` = second-to-last record, `B` = last):

1. `d = normalize(B - A)`; `T = A + d * wire[+0x14]` where `wire[+0x14] = 0.5` (the constant stored by the wire constructor `0x00686690`).
2. Two retract records are created (`0x0061B770`, flag byte 1): one anchored at `A` and one at `B`, **both aimed at `T`** (so the wire snaps 0.5 m from the first end: a 0.5 m piece at `A`, a `|AB| - 0.5` piece at `B`), then both pieces retract as in 3.1.
3. `PlayOneShot(field 0)` = effect **213 `hook_detach`** at `A` and at `B`: matrix = identity rotation with the end's world position, one interaction `{kind 1, uid = the end record's surface id}` (record `+0x40`, inferred as the surface id of the hit; the same value feeds the interaction selector of effects-pipeline section 3).
4. `StopSlot(4)` (effect 215 `dual_hook_deployed`, graceful) if playing; `PlaySlot(5)` (effect 410 `hook_detach_dual`) if not playing. Both happen on every cut, not only for dual hooks.
5. The wire is erased and destroyed (its constraint with it, `tether-forces-next.md` section 4); the UI event `gui_aim_dualhook_disable_event` is posted.

The device detach `0x005A0530` (hook missed, cancelled) plays slot 6 (effect 214 `hook_miss`) and one-shot 213 at the single end; `0x007504B0` plays 213 at the first end when the wire is removed with one end.

## 4. Hook models (native pieces, inferred placement)

* Two model instances are created at load from `hook_model` (`0x008701B0`, identity transform) and kept as device `+0x140`, `+0x144`; both get flag bits `0x11` set every frame in which a wire or retract piece is drawn (section 3.1) and cleared when nothing remains.
* In the draw `0x007DE2E0`: model 0 is moved to the **first** polyline point and model 1 to the **last** (the accessor `0x008D34E0(0|1)` returns point 0 and point 1 of the two-point polyline), each with a basis built from the wire direction (the second reversed, `-d`) and the transform setter `0x00A464F0`.
  In the sag path the first hook position is blended with a second vector (`0x0069C350` / `0x0052C310`, weights 0.7 and 0.3, `.rdata 0x0106D6BC`) and pushed along the direction by 0.06 m (`0x0106D6C0`). What those vectors are (the hand bone? the hit normal?) is unresolved (Gap 1).
* Hook mesh and textures: `general.blz/wea04_lod1-grapplinghook.rbm` (146 vertices, material in `material-parameters-next.md`). The projectile itself (`CGrapplingHook`, vtable `0x00EBA84C`) is a `CAmmunition`: factory `0x006CA7F0` case 10 -> constructor `0x005878A0`;
  weapon 52 `Grapplinghook` has `model = ""`, flight id `#e56e25f4 = -1`, muzzle `#04374f9d = -1`, impact `#469ba70d = 212`. The hook's per-frame slot 9 (`0x00549660`) only keeps an effect controller for the **flight id** (`+0xbc`, -1 here, so nothing) restarted with the hook's matrix.
  So the projectile has no model and no flight particle (native); the visible hook in flight is, inferred, the second device model instance at the last point of the in-flight wire.

## 5. Effects (native data)

All eight files are `effects/*.bin` parsed as in `effects-pipeline.md`; ids resolve in `effectsettings.bin`.

### 5.1 Short effects

| Id | File | Content |
|---|---|---|
| 11 | `flying_hook` | one sound, bank 26 sound 0 = `Parachute/grappling_hook/grappling_fire_wire`, `var_id_focus` 0. **Played by Rico's `grapplefire` slot** (7 in `RicosEffects.bin`, bone `AttachHandLeft`), triggered by the animation event `grapplefire` at **0.19 s** of `grpl_fire.event_trackb` (the fire clip). Not played by the device. |
| 214 | `hook_miss` | sound 26:2 `grappling_reel_back`, `max_instances` 10 |
| 215 | `dual_hook_deployed` | sound 26:8 `grappling_impact_dual` |
| 410 | `hook_detach_dual` | sound 26:11 `grappling_detatch_dual` + a SoundPostEffect (type 9: attack 0.01, release 3, gain 0.95, `#50be431d` 1, `#c6fdc17a` 50, `#e8abc55f` 3, `#0f429f78` 5, target 0, looping 0) |
| 203, 204 | `char_reelin_sparks`, `char_slingshot_sparks` | `looping = 1`, one particle component (`fx_char_reelin_sparks_01`, flag byte `#b0124c0a` 1), no sound. Both files name the same particle system. The particle systems `fx_char_reelin_sparks_01..04` and `fx_hook_*` exist in `particles.blz`. |
| 213 | `hook_detach` | default matrix: particle `fx_hook_detach_01` (variation 0 of component 1) and sound 26:9 `grappling_detatch` (`#b97dd06b` 0.1); water pair (`#55ba2d5a`): sound 17:7 `Bullet_Hit_Water` and the particle pair `1:3`, which does not exist (the single retail inconsistency noted in `effects-pipeline.md`), so **no particle under water** |

Effects 203/204/214/215/410 have no per-surface table. None of the slot controllers receives a matrix in any traced code (no `SetMatrix` helper for the slot array exists; the only
slot helpers are `PlaySlot`/`StopSlot`/`PlayOneShot`): where the looping sparks appear is unresolved (Gap 2).

### 5.2 Sound names (soundmap)

Bank 26: 0 `grappling_fire_wire`, 2 `grappling_reel_back`, 3 `grappling_impact_def`, 4 `_rock`, 5 `_hardmetal`, 6 `_hollowmetal`, 7 `_body`, 8 `grappling_impact_dual`, 9 `grappling_detatch`, 11 `grappling_detatch_dual`
(all under `Parachute/grappling_hook/`, max 1 playback, steal 10000); bank 17 sound 7 `Effects/Impacts/Bullet_Hit_Water`.

### 5.3 `hook_impact` (212): the hook's hit effect

Played by `CGrapplingHook` vtable slots 11 (world hit, `0x00A3B0B0`) and 12 (object hit, `0x0063F150`), which call the surface impact spawner `0x005F6660` (reads the id from `[projectile + 0xc0]` = weapon type `+0x2e4` =
`#469ba70d` = 212 for weapon 52, builds `{kind 0/1, uid}` from the water flag or the surface id, orients by the hit normal, ends in `0x0077F940`) and then the attach completion `0x00538470`.
Components: particle (type 1, 12 variations), decal (type 3, 10 variations), sound (type 0, 6 variations). Resolve per `effects-pipeline.md` section 3: default matrix = particle 0, sound 0; water pair (flag) = particle 3, sound 5.

| Surface (id) | Particle (variation) | Decal | Sound |
|---|---|---|---|
| default and Flesh (14), Undefined (31), Holy glass (27) and any surface without an entry | 0 `fx_bulhit_snow_medium_01` | none | 0 `grappling_impact_def` (26:3) |
| Metal (8), Metal_Solid (20), Metal_Car (21), Metal_CatWalk (22), Metal_Fence (23) | 1 `fx_bulhit_metal_medium_01` | 0 (metal, 0.25 m) except Metal_Fence | 2 `_hardmetal` (26:5) |
| SteamMetal (15), Metal_Steam (24), Metal_Oil (25) | 7 `fx_bulhit_steammetal_medium_01` (Metal_Oil: 1 `metal`) | 0 | 3 `_hollowmetal` (26:6) |
| Stone (4), Stone_Concrete (16), Stone_Rock (17) | 4 `fx_hook_concrete_01` | 1 (concrete, 0.4 m) | 1 `_rock` (26:4) |
| Soil (1), Gravel (3), Flower_Field (33), Grass (34) | 8 `fx_hook_concrete_01` | 6 (dirt, 0.5 m) | default |
| Wood (6), Wood_Solid (18), Wood_Hollow (19) | 6 `fx_hook_concrete_01` | 3 (wood, 0.4 m) | default |
| Sand (0) | 9 `fx_bulhit_sand_medium_01` | 5 (the "Snow" decal, 0.5 m; the 0.06 m "Sand" decal 9 is referenced by no surface) | default |
| Snow (2), Arctic dirt road (32) | 0 | 5 (snow, 0.5 m) | default |
| Ice (5) | 0 | 4 (ice) | default |
| Glass (9) / Glass_Security (26) | 5 `fx_bulhit_glass_medium_01` | 2 / 7 | default |
| Asphalt (30) | 4 `fx_hook_concrete_01` | 8 (asphalt, 0.5 m) | default |
| Electrical (28) | 10 `fx_bulhit_electric_medium_01` | 0 | default |
| Vegetation (29) | 11 `fx_bulhit_veg_medium_01` | none | default |
| Rubber (12) | 4 `fx_hook_concrete_01` | none | default |
| Textile (13), Paper (10), Plastic (11) | none (`-1`) | none | default |
| Water (7) and the water flag | 3 `fx_bulhit_water_medium_01` | none | 5 (17:7 water) |

Decal variations all use `Decals/BulletImpacts_DIF/SPC/NRM.dds`, `lifetime = 10`, `size` = (width, height, 0.3, 0.3) with the widths given in the table (Metal 0.25, Concrete and Wood 0.4, the others 0.5, Sand 0.06). Particle variation 2 (`Flesh`, `fx_bulhit_flesh_medium_01`) is **unreachable**
(no matrix string refers to `1:2`) and the system does not exist in `particles.blz`; sound variation 4 (`_body`, 26:7) is not referenced by any matrix string either. They belong to a path
that passes the hit object (the object array of the interaction wrapper, effects Gap 6 of `effects-pipeline.md`); a hook that hits a soldier is handled by the character branch of the hook hit, not decoded here.

## 6. What the current port draws (for the paired task)

`crates/jc2-game/src/grapple.rs` draws every wire as `Cable { a, b, slack }`: 14 short cylinders along a parabola sagging `slack * 6 %` of the length, a pale unlit line whose radius grows with
camera distance (`wire_width` is the minimum), the same for `HangWire` and `TetherWire` in `stunts.rs`; no hook mesh follows the wire ends, no cut pieces, no tether effects. Differences from the original:
11 points (10 segments) instead of 15; 1 percent sag of the length (0 under pull) instead of 6 percent times slack; an **eight-sided tube of fixed 0.0085 m radius**, textured with `Wire_DIF/nrm/mpm`, instead of a pale line of growing radius; two hook models at the ends;
the cut animates two pieces retracting at 100..300 m/s with `hook_detach` at both ends.

## 7. Port architecture (what the implementation tasks build)

```
jc2-game::grapple (cable module)
    WireVisual { points: Vec<Vec3>, sag: bool, tension: Option<(elapsed, max)> }
        polyline(p0, p1)  -> 11 points (section 2.2) or the points as given
        tube(points, profile = octagon(wire_width)) -> mesh rebuilt every frame (8 sides, sweep with a stable frame, texture Wire_DIF/Wire_nrm/Wire_mpm)
    RetractPiece { anchor, dir, total, len }  step(dt): len -= ((len / total) * (100 - 300) + 300) * dt; polyline [anchor, anchor + dir * len]; despawn at len <= 0.001
    on cut: pieces for A and B aimed at T = A + normalize(B - A) * 0.5; effects: one-shot 213 at A and B with the end's surface id; stop 215; play 410
    hook models: two instances at the first and last point, oriented along the wire (section 4)
jc2-game::effects (existing spawner)
    slots: 0 -> 203, 1 -> 203, 2 -> 204, 4 -> 215, 5 -> 410, 6 -> 214 as controllers with PlaySlot / StopSlot rules (play only when not playing; stop only when playing)
    hook hit: effect 212 through the interaction path (surface id or the water flag)
    Rico animation event "grapplefire" (0.19 s of grpl_fire): effect 11
```

Decisions that need a call from the orchestrator: none. `tether-forces-next.md` already says the original has no spring; this document adds only visuals.

## 8. Gaps

1. **Tube renderable and hook placement** (blocks exact wire look and hook position): the class built by `0x00939130` (vertex builder, UV use of the floats 10.0 / 3.0, lighting model and shader of the three wire textures, mip/filter), the device field `+0xc` (1 selects the general path), the vectors `0x0069C350` / `0x0052C310`
   used for hook orientation in `0x007DE2E0`, the hand attach point of the in-flight wire (`+0x12c`, end record 0) and the exact use of `0x007461E0`. Filed as a research task.
2. **Effect controller transforms** (blocks position of 203/204 sparks and 214/215/410 3D sounds): no traced code sets a matrix on slots 0..6; the native may rely on the controller default. Whether the sparks are visible at all, and the sound positions, are unresolved. Also the object/body sound variation (26:7) and the hook-versus-soldier branch of the hit. Filed as a research task.
3. Surface id at the cut (end record `+0x40`) is inferred from the copy into the interaction entry; the writer of that field in `AddPoint` `0x0083DC00` was not traced.
4. Who sets the tension duration (`0x0073DFE0` argument at the two callers) and whether any caller ever passes a value that makes the factor non-zero.
5. Retract speeds `300/100` are two `.rdata` constants (`0x00E38304`, `0x00E22EE8`) with no key in a data file; no way to change them from data was found.
6. `Wire_nrm` / `Wire_mpm` semantics follow the general material (`material-parameters-next.md`) only by name; unconfirmed for the tube shader.

## 9. Port wiring of the effect calls (board task 7bd60e82)

`crates/jc2-game/src/grapple/effects.rs`: `PlaySlot` / `StopSlot` / one-shot calls go through the effect spawner with the native
rules (play only when not playing, stop only when playing). Call sites: reel start (`begin_reel`) plays slot 0, the end of the reel
(`arrive`) stops it; the hook's hit plays 212 and slot 1 (inferred: for reel and pull alike); the cancel (`GrappleRig::retract`)
stops slots 0..2 and the device detach plays slot 6 (214) and one-shot 213 at the hook (only when a wire was out); a pull with the
canopy open plays slot 2 (204; inferred stand-in for the parachute controller); the tether gun's completion plays slot 4 (215), its
shots play 212, and a removed tether is the cut (213 at both ends, stop 215, play 410); `grapplefire` plays 11 at 0.19 s after the
shot at the hand position. Non-native: the position given to the slot controllers (gap 2), the missing surface id of a hit (default
matrix entry), the hit normal of tether shots (up), and the clip time of `grapplefire` (time since the shot, not the clip player).
The pre-existing placeholder sounds of `audio.rs` (`grapple_fire`, `grapple_detach`) still play beside these until the sound adapter lands.

## Reproduce

```
python research/tools/grapple_tether_visuals_probe.py --game "<game folder>" --tool <path>\jc2tool.exe
```
