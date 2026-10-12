
11 October 2026. Research only: no Cargo, GPU, game launch or runtime edit. Resolves Gap 2 of
[water-sea-consumers.md](water-sea-consumers.md) (JC2-189): the remainder of `FUN_009a5c60` after the cap volume, the sphere radius/centre
source, the base term at `+0x38`, the meaning of 1100.0 and the consumer of the summed volume. Probe: `research/tools/water_buoyancy_probe.py`
(62 checks, exits 0 only if every one passes; pseudo-C fragments are asserted with whitespace normalised, bytes and key hashes are read from the
image, the boat data comes from the retail `.mvdoll` / `.vdoll` files through `jc2tool`). All addresses are runtime addresses of
`JustCause2.exe` (sha256 `df99a479...4f80`, no ASLR).

```
python -B research/tools/water_buoyancy_probe.py [--game <game folder>] [--pseudo <git-ignored _unnamed export dir>] [--tool <jc2tool>] [--json] [--verbose]
```

## Answers

1. **Two owners, one driver.** `FUN_009893c0` (the driver) is called from exactly two places, both Havok per-step action functions
   (vtable slot 3): `CBoyancyAction` (0x9b6710, used for props, ragdolls and any entity with a buoyancy parameter set) and `pfxSeaVehicle` (0x9c4770,
   the boat/sea-vehicle action). It calls `FUN_009a5c60` once per buoyancy sphere. The object the driver works on is the **hkBoyancy block** (RTTI
   `hkBoyancy`, 0x80 bytes): `[CBoyancyAction + 0x1c]` or `[pfxSeaVehicle + 8]`. The rigid body is a `CPfxRigidBody`-style wrapper (`block + 0x70`; its
   Havok object is at wrapper `+0xd0`).
2. **The block layout** (built by `FUN_009a24d0` from a parameter struct, or by the boat loader `FUN_009b6c70`):

   | Offset | Meaning | Source |
   |---|---|---|
   | +8 / +0xc | array of sphere centres in body space, 16-byte entries / count | `position` of each `CBoyancyModule_i` |
   | +0x14 | **radius r, shared by every sphere** | `radius` of `CBoyancyModule_0` only |
   | +0x18, +0x1c | r squared, r cubed | computed |
   | +0x20 | buoyancy multiplier | `boyancy` (boats: `CBoatGeneralModule`, props: `buoyancy`) |
   | +0x24 | drag coefficient | `drag` of `CBoyancyModule_0` |
   | +0x28 | side area (body x axis) | `side_area` of module 0 (key 0x30a35b86) |
   | +0x2c | bottom area (body y axis), default 1.0 | `bottom_area` (key 0x578ccc6b) |
   | +0x30 | up-velocity cap, default 20.0 | `up_velocity_cap` (key 0xdc7c4744) |
   | +0x34 | summed submerged volume of the last step | driver |
   | +0x38 | dry mass | body mass (`1 / inverse mass`, body `+0xcc`), captured at set-up |
   | +0x3c | mass used on the previous step | driver |
   | +0x40 | stored original inertia (3 x 4 floats) | body getter at set-up |
   | +0x70 | body wrapper | set-up |
   | +0x74 | **buoyancy bias** B; the force scale is `B + 1.0` | `CSeaVehicle` (see 7), default 0 |
   | +0x7c | "in water" byte: 1 when the summed volume is positive | driver |

3. **The data is read by hashed key from the vehicle's property tree.** The sphere list is the group `CGroupModule_CBoyancyModule` (the module count is
   the 32-bit key at `0xf26b10`, 0x21af1963, whose name is not resolved), children are selected with the name `CBoyancyModule_<index>` (`FUN_009c0560`
   formats `%s_%d` and hashes it). **Only child 0 supplies radius, drag, side area, bottom area and the cap; children 1..n-1 only supply their `position`.**
   All 14 retail boats have this shape (the other children carry radius 0 and drag 0, which the loader never reads). The swamp boat has no
   `bottom_area` and no `up_velocity_cap` key: the loader presets 1.0 and 20.0 before reading, so those defaults apply. The key spellings are inferred by hash match
   (the strings are not in the image): `radius`, `drag`, `position`, `boyancy`, `CBoatGeneralModule` appear in the retail files by name, `side_area`,
   `bottom_area` and `up_velocity_cap` reproduce 0x30a35b86, 0x578ccc6b and 0xdc7c4744 from a name search. Props use the same names in `SPhysicsParams`
   (`radius`, `buoyancy`, `drag`, `side_area`, `bottom_area`; presets side/bottom 1.0, cap 20.0) with the sphere centres in `SPhysicsBuoy`
   (loader `FUN_008ed840`, CSimpleRigidObject). The ragdoll (`FUN_004cbae0`) builds the block from constants: **one sphere at the body origin, radius 0.1,
   boyancy 2.7, drag 150, side area 1, bottom area 1, cap 2.0**.
4. **Per sphere (`FUN_009a5c60`).** Let P be the sphere centre in world space relative to the physics origin (body matrix at wrapper `+0xe0` times the
   centre). The plane is the driver's flat plane `(0, 1, 0, -200)` with the offset replaced by minus the **wave height at P** (JC2-189). Then, with
   `dist = P.y - wave` (the signed height of the centre above the water):
   * `dist >= r`: the sphere is dry, nothing is applied.
   * `-r <= dist < r`: with `h = r - dist` (centre above water) or `h = r + dist` (centre below), the cap volume is `pi/3 h^2 (3r - h)`; for a centre below
     water the volume is the whole sphere `4pi/3 r^3` minus that cap. The application offset is `c = 3 (2r - h)^2 / (4 (3r - h))`. The wetted area is the
     circular segment `r^2 acos((r - h)/r) - (r - h) sqrt(2rh - h^2)` **for the same h in both cases** (as decompiled; see Gap 4 for the consequence).
   * `dist < -r`: volume `4pi/3 r^3`, offset 0, area `pi r^2`.
   * **Lift**: `F = boyancy * V * (B + 1.0) * 9820.0`, a force along +y (see Gap 1 for the branch that uses the plane normal), applied with the Havok
     "apply force at point" slot (body object `+0x54`, step time as first argument) at the point `P + (0, c, 0)` (the decompiled `P - (0, -c, 0)`; the sign is
     listed in Gap 2). 9820.0 is read as 1000 kg/m^3 times 9.82 (inferred split).
   * **Drag**: the velocity of the body at P (linear + angular x arm to the centre of mass) is projected on the three body rows (`+0xe0`, `+0xf0`,
     `+0x100`); it is skipped when its square is below 0.0001. The row-`+0xf0` component is clamped to +-cap. The force along each row is
     `-0.5 |v| v * drag * area_wet * coefficient` with coefficient `side_area` for `+0xe0`, `bottom_area` for `+0xf0` and an implicit **1.0** for `+0x100`
     (the hull's length axis, inferred from the positions: the stern/bow spread is along z), applied at P with the same slot. The wetted area is the one
     computed above.
   * The function returns the volume (for the driver's sum). A call through `[physics + 0x40]` gated by `[physics + 0x64]` draws the sphere in the debug
     path; its result is not used.
5. **The driver (`FUN_009893c0`).** Per step: wake the body and push its cached linear velocity (body `+0x1a0`) into Havok (the wake-up reading of `FUN_00b78700` is inferred); fetch the flat sea level
   (200.0) and build the plane; sum the sphere volumes into `+0x34`; set the in-water byte; then the **added-mass rule**:
   * `m = sum V * 1100.0 + dry mass` is written as the body's mass (body slot `+0xc`) every step;
   * the inertia is the stored original times `m / dry mass` (body slot `+0x1c`);
   * the vertical (world y) velocity is multiplied by `min(previous mass / m, 1)` and written back (body slot `+0x40`): water that is taken along slows a
     plunging hull (momentum conservation), and a rising mass ratio never speeds it up.
   So the 1100.0 is **not** a lift density: it is the mass of water carried along per m^3 submerged. With the lift scale 9820 (1000 x 9.82) the hull floats
   (for g = 9.82, equal-depth spheres) when `boyancy * 1000 > 1100`, and the immersed volume at rest is `mass / (1000 * boyancy - 1100)` (v083:
   2500 / 1300 = 1.92 m^3, each of the nine r = 0.7 spheres immersed 0.34 m; derived from the formulas, not measured).
6. **`CBoyancyAction` vs `pfxSeaVehicle`.** `CBoyancyAction` first re-reads body, dry mass and original inertia whenever the block holds no volume
   (`+0x34 == 0`), so the mass the body has while dry is always the dry mass, and skips bodies whose type byte (`+0xd8`) is 6. `pfxSeaVehicle` (0x30
   bytes: block `+8`, engine action `+0xc`, body `+0x10`, three inertia factors `+0x14/+0x18/+0x1c`, mass `+0x20`, `+0x24`, `+0x28`, byte `+0x2c`) runs the driver,
   then the engine action (slot 3 of the engine object at `+0xc`), then a **linear quadratic drag** `-(k28 * k24 * |v|) * v` at the centre of mass (body slot
   `+0x58`), and if the byte `+0x2c` is set zeroes the horizontal velocity. Its set-up (`FUN_009ad7c0`) writes mass `+0x20` as the body mass and the inertia as
   `diag(f18, f14, f1c) * mass`. Loader keys (`FUN_0099f8a0`, section `CBoatGeneralModule`): +0x20 mass (0x4365a91d), +0x24 0x8d840498, +0x28 `drag`,
   +0x14 0xa94eda6c, +0x18 0xd9336205, +0x1c 0xc1f9d442 (unnamed keys keep their hash).
7. **Sinking.** `CSeaVehicle` (0x627610, every frame) lowers the bias B by `0.2 * frame time` while the ratio of its fields `+0xe4 / +0xe0` is `<= 0`
   (inferred: current health over maximum health; both start at 1) and clamps at **-0.9**; the lift scale `B + 1` therefore falls from 1.0 to 0.1 at 0.2 per
   (1e-6 while paused).
8. **Retail data** (module 0; `jc2tool` on the 14 retail `seve.*` dolls; count = number of spheres; bottom area for the swamp boat is the default):

   | Boat | Spheres | r | drag | side | bottom | boyancy | mass |
   |---|---|---|---|---|---|---|---|
   | v011 charter / houseboat / sailboat / yacht | 9 | 0.85 | 20 | 60 | 120 | 2.25 | 10000 |
   | v082 motorboat standard | 9 | 0.75 | 20 | 50 | 100 | 2.4 | 5000 |
   | v083 motorboat small | 9 | 0.7 | 20 | 75 | 125 | 2.4 | 2500 |
   | v086 barge | 8 | 1.65 | 75 | 25 | 150 | 2.4 | 60000 |
   | v087 catamaran | 10 | 1.0 | 15 | 75 | 150 | 1.9 | 7000 |
   | v088 djonk | 9 | 1.0 | 20 | 75 | 100 | 2.0 | 9000 |
   | v089 raceboat | 10 | 0.6 | 15 | 70 | 250 | 2.25 | 3000 |
   | v095 swamp boat | 9 | 0.35 | 25 | 15 | 1 (default) | 2.5 | 890 |
   | v098 indo | 9 | 1.2 | 40 | 75 | 125 | 2.0 | 20000 |
   | v104 attack boat | 9 | 0.75 | 20 | 75 | 150 | 1.9 | 4000 |
   | v105 military boat | 9 | 1.4 | 20 | 75 | 125 | 1.75 | 20000 |

   The up-velocity cap is 20 wherever present. The older `vehicle.md` reading ("radius 0 for most points") described the unused module values; the native
   model uses the module-0 radius for every sphere.

## What this changes for the runtime (no code changed here)

* The jc2-game boat float model (`boat.rs`: depth-scaled lift per point, heave damping, keel, quadratic drag tuned to a speed) is not the native
  model. The native one is: spheres of one shared radius at the module positions, lift from the cap volume against the **wave height at each sphere**
  (JC2-189), `boyancy * V * 9820 * (B + 1)`, per-axis quadratic water drag scaled by the wetted segment area, plus the mass/inertia/vertical-speed
  added-mass update. The engine, steering and rudder modules are separate (`pfxBoatEngine`, not decoded here).
* Hull parameters to carry per boat: `boyancy` and `mass` (`CBoatGeneralModule`) and, from `CBoyancyModule_0`, `radius`, `drag`, `side_area`,
  `bottom_area` (default 1), `up_velocity_cap` (default 20), plus every module's `position`.
* Sinking is the bias falling at 0.2 per second to -0.9 while the vehicle's health ratio is 0 or less.

## Proven / Inferred

**Proven** (asserted fragments, bytes and hashes): the two action owners and their call sites; the driver's plane, per-sphere sum, 1100.0 / dry-mass rule,
ratio clamp, inertia scale and vertical-velocity scale; every immediate of the per-sphere function (cap volume, centroid, segment area, 9820.0, the
0.0001 gate, the clamp, the three drag formulas); the block layout and its loaders; module 0 as the only source of shared parameters (decompiled loader and all 14
retail dolls); the ragdoll and prop parameter paths; the sinking bias (rate 0.2, floor -0.9); the key words that hash-match.
**Inferred**: that the body object's slots `+0xc`, `+0x14`, `+0x1c`, `+0x40`, `+0x54`, `+0x58` are set-mass, get-inertia, set-inertia, set-linear-velocity,
apply-force-at-point and apply-force (named from how they are used); 9820 = 1000 x 9.82; body rows `+0xe0/+0xf0/+0x100` = x/y/z with z the hull's length axis; the
health reading of `+0xe4 / +0xe0`; the key spellings (hash match); `+0x74` as a "bias" (its only writer subtracts frame time).
**Equilibrium numbers** in answer 5 and the probe are derived from the formulas assuming a world gravity of 9.82 (not located), not measured in the game.

## Gaps (each stays a native unknown; none is filled by a guess)

1. **Body property key 4 and the ray-cast plane.** `FUN_009a5c60` asks the body's property table (array at body `+0x74`, count `+0x78`, 16-byte entries;
   lookup `FUN_00495e80`) for key 4. When set, it casts a short ray from the sphere centre (length r + 2, filter from key 1) and may replace the water plane with
   the hit (`FUN_009c4540`), and the lift is then multiplied by the plane normal instead of +y. Who writes key 4 (and key 1 for the filter) is not decoded.
   Blocks: whether any shipped boat uses the collision-surface plane (shallow or enclosed water).
2. **Body object slots and the lift application point.** The Havok body slots are named by use only, and the sign of the application offset (`P + (0, c, 0)` as
   decompiled) was not checked against the instruction bytes. Blocks: exact torque from the lift.
3. **World gravity and the 9820 split.** The physics world gravity was not located, so the 9.82 factor and the water density 1000 are a reading of the
   constant. Blocks: numeric draft and the stability of the equilibrium.
4. **Wetted-area behaviour below the surface.** For a centre below the water the decompiled area is the segment of the above-water cap (it falls to 0 as the sphere
   sinks, then jumps to `pi r^2`). Whether this is the original behaviour is not cross-checked by bytes. Blocks: drag while a sphere is mostly submerged.
5. **CSeaVehicle bias restore and the `pfxSeaVehicle` extras.** Writers of `+0xe0/+0xe4` (health meaning), any code raising the bias back, the byte `+0x2c`
   (horizontal velocity kill) writer, and the values/names of `+0x24` (key 0x8d840498) and `+0x28`. Blocks: boat sinking and wreck parity.
6. **Helicopter variant.** `FUN_0099f400` loads the same block for aircraft from a different section (key 0xc47a759c, name not resolved) with the `boyancy`
   key; its consumer for helicopters and planes on water is not decoded. Blocks: helicopter water behaviour (not boats).
