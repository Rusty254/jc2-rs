# Shore waves, CPU wave height consumers, vertex-wave table and ocean pass state (JC2-165)

11 October 2026. Research only: no Cargo, GPU, game launch or runtime edit. Continues
[water-inputs-next.md](water-inputs-next.md) (JC2-102, Gaps 4, 6, 7). Probe: `research/tools/water_waves_probe.py`
(42 checks, exits 0 only if every one passes; pseudo-C fragments are asserted with whitespace normalised, function bytes are
pinned by sha256 prefix). All addresses are runtime addresses of `JustCause2.exe` (sha256 `df99a479...4f80`, no ASLR). Pseudo-C was read
from the git-ignored `research/analysis/pseudo/_unnamed` export.

```
python -B research/tools/water_waves_probe.py [--game <game folder>] [--pseudo <_unnamed>] [--verbose]
```

## Answers

1. **The 0.6 m wave height is used by gameplay** (Gap 7, revises the "physical sea level stays 200" rule of JC2-102 section 9 step 6). The only three
   callers of `FUN_00a8bba0` (CPU wave height = 200 + `FUN_00a88ea0`) are slots of three runtime classes, found by walking back to the RTTI of each vtable:
   `CAtSeaLevelCondition` (0x59a1e0, vtable 0xeb74f0 slot 2, a script/AI condition), `CPfxSea` (0x59ff30, vtable 0xeb8f84 slot 2, the sea particle
   effect query) and `CSwimController` (0x5ac120, vtable 0xec2d20 slot 6, the swimming character update). All three pass the **camera position**
   (`*(DAT_011919f4 + 0x10) + 0x54..0x5c`) as the second argument, the reference point of the 66.67 m fade, so waves exist only near the camera.
2. **Shore-wave acceptance and the terrain query layout are now exact** (Gap 6, first half): section 1 and 2.
3. **Vertex-wave table** (Gap 7, second half): section 4.
4. **Ocean pass depth, cull and sampler state** (Gap 4, most of it): section 5.

## 1. Terrain query output layout (`FUN_00a5f600` -> `FUN_00a56400`)


| index | content |
|---|---|
| 0 | terrain height (sum of out[7] and out[8]) |
| 1..3 | unit normal: `(a, 1, b)` normalised, `a`, `b` = the two slope terms of out[5], out[6] (the sampler stores them negated, `-0.0 - delta`) |
| 4 | pointer to the 0x60-byte material record (`0x137baf0 + type * 0x60`) |
| 5, 6 | the raw slopes before normalisation |
| 7, 8 | the two height components summed into out[0] |
| 10, 11 | further per-cell values (not needed by the sea; not decoded) |

So the shore-wave code's "terrain-normal direction" is `(out[1], out[3])` (x and z of the unit normal): it points **downhill, towards the water**.

## 2. Shore-wave acceptance and strip (`FUN_00a81b00`, one `CWaveObject` of 0x30 bytes)

Input: object position `(+0xc, +0x10, +0x14)`. Steps (all constants are immediates; probe model `accept_shore_wave` reproduces them):

1. `h0` = terrain height at the object, `d = max(200 - h0, 0.1)`, `a = 5 / d`, `b = 8 / d`. The object's flag word (+4) loses bits 0 and 1 (bit 1 = "needs init",
   set by the animation, bit 0 = "flat strip", set at the end). **Reject unless `h0 < 199.9`.**
2. `n` = the horizontal part of the terrain normal, renormalised (`n.x, n.z` over `sqrt(n.x^2 + n.z^2 + 1e-8)`; if that squared length is below 1e-6 the scale is 1).
   Samples: `hA = height(p - n a)`, `hB = height(p - n b)` (uphill, inland), `hF = height(p + 50 n)` (seaward).
   **Accept only if `(hA > 200 or hB > 200)` and `hF < 200`**: land close behind, water 50 m ahead.
3. The strip frame is stored: `+0x24/+0x28/+0x2c = (n.x, 0.0001, n.z)` (the 0.0001 is the y of the direction before renormalisation, stored unrenormalised),
   side vector `+0x18/+0x1c/+0x20 = (-n.z, 0, n.x)`, object y `+0x10 = 200.0`.
4. The two flank points `p +- side * 12 * 0.3` (= +-3.6 m) must both have terrain `< 200` (under water), else the object is rejected (nothing is built).
5. 4 x 4 vertices into a 12-byte-per-vertex buffer (x float, z float, packed colour dword: the vertex y is **not** stored, only its ramp in byte 2, so the wave shader rebuilds the height), origin
   `p + n * 4 - side * 6`; vertex `(i, j)` (row `i` along the direction, column `j` across) sits at `origin + side * 12 * j / 3 - n * 8 * i / 3`: a **12 m wide, 8 m long strip from 4 m seaward to 4 m inland**
   of the object. Height: rows 0 and 1 are 200.3; rows 2 and 3 are 200.3 where terrain is under water, otherwise `terrain + 0.05`.
   Colour dword: byte 0 = `j / 3 * 255`; byte 1 = `int(f / (0.4 f + 1) * 255)` with `f = i / 3` (0, 75, 134, 182); byte 2 = `min(255, (y - 200) * 51)`.
6. If the highest minus the lowest vertex height is below 1.0 the object gets bit 0 ("flat").

Animation (`FUN_00a89230`, unchanged from JC2-102 plus the budget): phase `+0x18 += 0.45 dt` wrapped at 2 pi, `+0x1c += 0.25 dt` wrapped at 1 (`dt` is the getter
moved by whole 300 m steps (`2 * half extent`) and flagged; **at most 16 flagged objects per frame are re-initialised** (the counter decrements only when `FUN_00a81b00` is called).

## 3. CPU wave height consumers

* `CAtSeaLevelCondition` (0x59a1e0): reads an object position by index (`FUN_00677d60` on the entry `+0x41c + index * 4`), builds `(x, z)` plus the camera reference, evaluates
  `v = y - waveHeight(x, z)` and tests it against the condition's threshold (`+8`): `v < threshold` when the byte at `+0xc` is 0, else `v >= threshold and v != threshold`. A script-level "is at sea level" test, so
  characters and vehicles are tested against the **wave surface**, not y = 200.
* `CPfxSea` (0x59ff30): `(x, z)` from the two arguments, camera reference; the pseudo-C discards the value because the function returns it on the x87 stack (inferred: the sea particle placement reads it).
* `CSwimController` (0x5ac120): per frame, with `dt` the getter above, the swimmer's wave offset `w = waveHeight - y`. The vertical correction vector (stored at the
  controller's `+0x1c0/+0x1c4/+0x1c8` by `FUN_00898740`) is `(0, clamp(((K + w) / dt - v) * G, -10, 10), 0)`, `v` = the controller's `+0x1b8`; two branches:
  `K = -0.15, G = 0.1` (when the character's state equals the one compared first against `DAT_01196c34` / `DAT_011970a8`) and `K = 0.05, G = 0.25` (otherwise). The constants are
  `DAT_01079a90 / 8c / 98 / 94`. **Swimmers therefore rise and fall with the wave height**; `-0.15` is the target depth below the surface in the first branch, `+0.05` above it in the second
  (inferred reading of the signs; the arithmetic is proven).

**Corrected by JC2-189** ([water-sea-consumers.md](water-sea-consumers.md)): `CPfxSea` slot 2 is the physics system's wave-height getter, so boats (`CBoyancyAction`, `pfxSeaVehicle`), splashes, projectile
water hits and effect placement *do* use the CPU wave height through an indirect call (the earlier "boats use the flat level" inference was wrong). `CSwimController`'s first branch applies to the single state `S_SWIM_RUN_TO_SWIM`, `S_SWIM_SUBMERGE` gets no correction and every other state takes the second branch.

## 4. Vertex-wave table (`FUN_00a80da0`)

`angle1 = (t & 0x3fffff) / 350`, `angle2 = (t & 0x3fffff) / 225` (immediates 0.0028571428 and 0.0044444446). Then 16 times: an MSVC LCG (`x = x * 0x343fd + 0x269ec3`, seed 0, **the same sequence every call**) draws
a random unit axis (components `(r >> 16 & 0x7fff) * 6.1037e-05 - 1`, renormalised; replaced by the identity rotation if it is within 0.0001 of parallel (|dot| > 0.9999) with the fixed reference axis (0, 1, 0)), builds a rotation matrix about it, applies two further rotations by
`angle1` and `angle2` (`FUN_004eaad0`, `FUN_004dbce0`), and stores the first row `(x, y, z)` with `w = 1..16` into a 16-entry, 16-byte table at ECX+0x64, mirrored at ECX+0x164. Four
8-byte copies at ECX+0x264..0x284 follow. The 0x22 vec4 span ECX+0x64 .. ECX+0x284 is exactly the size of the VS constant range cb1[5..38] bound by the ocean draw
gradient noise: 16 m lattice, smoothstep fade `t^2 (3 - 2 t)`, gradients from the same table (this is the function behind `FUN_00a88ea0`'s `noise(7.34 p)`).

## 5. Ocean pass state (`FUN_00a80300`, vtable slot of the ocean block)

Decoded through the D3D10 description builders (the same state words as the blend word of JC2-102 section 4):

| State | Setter call | Decoded value |
|---|---|---|
| Depth function (+0x88 bits 2..5), stencil | not touched | inherited from the pass defaults `FUN_0095dc20` (render-engine fields +0xc88 enable, +0xc84 write, +0xc8c function) |
| Blend operation / mask | not touched | ADD / 0xf from the default word (JC2-102 "inferred" is now: nothing in the ocean setup changes the ops) |

Samplers (`FUN_0095e900` address modes `WRAP 1 / MIRROR 2 / CLAMP 3`; `FUN_009203e0` filter fields; filter built by `FUN_00940170` from the tables at 0xf22858 / 0xf2285c, read from the image):

| Slot | Address | Filter | Probable use (from earlier briefs) |
|---|---|---|---|
| s0 | WRAP | `MIN_MAG_MIP_LINEAR` (0x15) | normal map layers |
| s1 | MIRROR | `MIN_MAG_LINEAR_MIP_POINT` (0x14) | planar reflection |
| s5 | MIRROR | 0x14 | scene colour copy (refraction) |
| s6 | WRAP | anisotropic field 7: ANISOTROPIC when the setting byte `ctx+0xccc` > 1, else 0x15 | foam |
| s7 | CLAMP | point (0) | scene depth copy |

Mip LOD bias 0 on all (the stored word is 0), maximum anisotropy 1 except s6. The slot-to-texture mapping is from JC2-095/102 and only the slot numbers are proven here.

## Proven / Inferred

**Proven** (pinned bytes plus asserted fragments): the terrain query wrapper and output indices; the whole of the acceptance and strip arithmetic; the animation budget; the RTTI classes of the three
`FUN_00a8bba0` callers and their arithmetic; the swim constants; the table builder's immediates and LCG; the depth, cull and sampler word decodes and the ocean's setter arguments.
**Inferred**: the physical meaning of the swim constants' signs; that the `CPfxSea` result is consumed; that cb1[5..38] binds the ECX+0x64 table; the semantic of out[10], out[11]; slot-to-texture mapping.

## Gaps (each stays a native unknown; none is filled by a guess)

1. **Shore-wave draw.** The `CWaveObject` vtable (0xf21118) draw, the waves vertex and pixel shaders (their time inputs, how the colour bytes and the "flat" bit are used), the index list, the POST_WATER
   draw state and the blend. Blocks: shoreline wave rendering parity.
2. **Wave-table binding and clocks.** The producer of the bound pointer `block+0x4c` for cb1[5..38]; the unit of the engine clock `FUN_004e7220` (millisecond counter? paused clock selected by `+4`). Blocks: exact VS wave animation speed.
3. **Swim and condition callers.** Resolved by JC2-189 ([water-sea-consumers.md](water-sea-consumers.md)); the residue (body-part identity, controller-to-state mapping, buoyancy force model, splash constants) is listed there.
4. **Pass defaults.** The depth function and stencil of the ocean pass (render-engine `+0xc8c`), the meaning of `DAT_013d9e2c`'s constant block, the setting byte `ctx+0xccc` (anisotropy). Blocks: depth-test parity of the ocean.
5. Carried over unchanged from JC2-102 (not touched here): reflection pass contents (1), MSAA-off colour copy (2), destination-alpha consumer (3), clip-map focus and budget (5), weather gating (8), NVIDIA/stereo/WaterBelow/god rays (9),
   horizon colour (10), `waterhighend` loader (11), `FUN_00677ff0` (12).
