# CPU wave height and the boat runtime (JC2-228)

11 October 2026. Runtime task: `crates/jc2-game/src/sea_wave.rs` (the CPU wave height `FUN_00a8bba0`) and the rework of
`crates/jc2-game/src/boat.rs` to the model of [water-buoyancy.md](water-buoyancy.md) (JC2-216) and
[water-sea-consumers.md](water-sea-consumers.md) (JC2-189). Decompiler fragments read from the git-ignored export
(`FUN_00a80da0`, `FUN_00a81190`, `FUN_00a88d60`, `FUN_00a88ea0`, `FUN_00a8bba0`, `FUN_00a5e680`, `FUN_0059ff30`, `FUN_004e0f60`, `FUN_004eaad0`,
`FUN_004dbce0`, `FUN_00491e50`); constants read from the image (`DAT_01081d9c` = 2 pi, `DAT_01081da0` = pi/2, `DAT_01081da4` = 1/pi,
`DAT_0108b948` = -16384, table `0xf20980` = 2^n).

## What the wave function is

   (camera x, y, z), all read from the camera object (`[DAT_011919f4 + 0x10] + 0x54/0x58/0x5c`). The two gates `|y - 200| <= 5` (in `FUN_00a88ea0` and
   `FUN_00a88d60`) therefore test the **camera's** height, not the boat's: with the camera more than 5 m above or below the sea the boat sees a flat sea (200.0).
   Likewise the 66.67 m reach (`dx^2 + dz^2 <= 4444.4453`) is measured from the camera to the queried point.
2. **Gradient table (`FUN_00a80da0`).** Sixteen entries from the MSVC LCG (seed 0, restarted every call; per entry three draws r1, r2, r3 give the axis
   `(z, y, x) = ((r >> 16 & 0x7fff) * 6.103702e-05 - 1)` for r1, r2, r3 in that order, normalised). Each axis becomes the first row of a "look along the axis,
   up = +y" matrix (`FUN_004e0f60`), is turned by `RotY(angle1)` with `angle1 = (clock & 0x3fffff) * 0.0028571428` (`FUN_004eaad0`: the engine's
   parabola-plus-0.225 sine, not libm), then by `RotX(angle2)` (`FUN_004dbce0`) in the product `RotX * (M * RotY)`; the table stores the first row. The first row of
   `RotX * N` is the first row of `N`, so `angle2` does not reach the table. For up = +y the row's y component is 0, so the noise gradient has only an x component.
   Entries 16..31 repeat 0..15 (same fourth float, the running index 1..16), entries 32 and 33 repeat 0 and 1.
3. **Noise (`FUN_00a81190`).** Input `7.34 * p` (absolute values). With `i = int(|x| mod 16)`, `zmod = |z| mod 16` and `w(k) = (k mod 16) + 1`, the four corner
   gradients are table entries `int(zmod + w(i))`, `int(zmod + w(i + 1))` and each `+ 1`; corner products `g . (dx, dz)`, smoothstep `t^2 (3 - 2 t)` on both axes.
4. **Vertex height (`FUN_00a88d60`)** `0.6 * noise * clamp(1 - 0.015 d, 0, 1)`; **point height (`FUN_00a88ea0`)** the plane through the three vertices of its triangle
   in the cell grid (cell = patch side / 4, split on `(z - z0) + (x - x0) < cell`) of the patch `FUN_00a5e680` returns, the first of levels 4, 5, ... whose
   cached patch (flag 0x10) contains the point (= the finest clip-map level, [`ocean_inputs::clipmap_level_at`]); `FUN_00a8bba0` adds 200.

Implemented and unit tested in `sea_wave.rs` (amplitude <= 0.6, gate, 66.67 m reach, continuity, mirrored table, the sine against libm within 0.002).

## Boat runtime (boat.rs)

* Spheres at every `CBoyancyModule_<i>` position with the radius of module 0 (parser: `jc2_formats::vehicle::Boat::{floats, hull}`), centre against the wave
  height at the sphere; cap volume / centroid / segment area, lift `boyancy * V * (B + 1) * 9820`, per-axis quadratic drag with `side_area`, `bottom_area`
  (clamped body-y velocity) and 1.0, skipped below a point speed of 0.01 m/s.
* Added-mass rule every step: mass `1100 * V + dry`, inertia `diag(factors) * mass`, vertical speed `* min(previous mass / mass, 1)`.
* `pfxSeaVehicle` extras that have data: the inertia factors (`#d9336205`, `#a94eda6c`, `#c1f9d442` as body x, y, z) and the whole-hull quadratic drag
  `-(drag * #8d840498 * |v|) * v` at the centre of mass (values from the retail dolls: drag 0 .. 4, area factor 1 .. 20).
* Sinking: while `Health.hp <= 0` the bias falls 0.2 per second to -0.9 (the health reading of `CSeaVehicle +0xe4 / +0xe0` is inferred).

## Proven / inferred / not native

**Proven by the decompiled text and image constants:** the gates and reach, the noise arithmetic, the table builder's LCG and axis order, the matrix helpers' layouts, the
engine sine, the vertex and triangle formulas, the level search of `FUN_00a5e680` (the walk over levels 4 .. 13), the point built from the camera's height.
**Inferred:** that the matrix helpers' `this` pointers are the local scratch matrices as the product order implies (the pointers are not in the decompilation); the engine
`CSeaVehicle` health ratio; the axis order of the inertia factors.
**Not native (stand-ins):** the propeller thrust (`pfxBoatEngine` undecoded), steering, the Havok body damping taken from the doll.

## Gaps

1. **GPU sea displacement.** `sea.wgsl` draws a flat sea, but the native vertex shader displaces with the same table (water-inputs-next.md section 3, VS lod0). The boat now rides
   0.6 m waves the sea surface does not show. Blocks: visible parity of floating objects.
4. **Matrix-helper `this` pointers of the table builder** (inferred call order above), **and the `FUN_00a5e680` fallback** (a point with no patch returns its own y).
5. **Open items of water-buoyancy.md still open:** body property key 4 (ray-cast plane), the Havok slots and lift application sign, the world gravity (Avian 9.81 is used, native 9.82
   inferred), the wetted-area behaviour below the surface, `CSeaVehicle` restore paths and the +0x2c horizontal-velocity kill byte, the helicopter variant.
6. **Boat engine, steering and rudder** (`pfxBoatEngine`, `CBoatEngineModule` `acceleration` and the unnamed keys): the thrust here is the earlier stand-in.
