# Coordinates: the engine frame and Bevy's

**The engine's frame is right-handed, the same as Bevy's, and `jc2-game` converts with the
identity.** x is east and a model's right, y is up, z is south and a model's back: a
vehicle's nose and a character's face look along −z, which is also Bevy's
`Transform::forward`. All conversions live in `crates/jc2-game/src/coords.rs`.

Until October 2026 the code took the engine as left-handed and mirrored z on everything
it read (`(x, y, z) → (x, y, −z)`, quaternions' x and y negated, placement matrices
conjugated). That drew Panau as its mirror image, put every car's steering wheel on the
right, and — because the triangle indices were not swapped — turned every model inside
out (hidden by `cull_mode: None`, but the double-sided shading then flipped the normals
inwards: the reason Rico looked almost black).

## Evidence

| what | where | says |
|---|---|---|
| The jeep's doll | `jc2tool vehicle <game> lave.v014_jeep.ee` | front axle (axle 0) at z ≈ −1.7, so the nose is −z; steering wheel at x = −0.56 (sport car −0.32, bus −0.61) |
| The jeep's doors | `Door_CL1` / `Door_CR1` geometry | the "L" door spans x −1.29..−0.87, the "R" door +0.87..+1.29 |
| The bus | `Door_FR1` | the front door ("R") at x +1.23 |
| The original game | `reference/vehicles/car-driver-seat-LEFT-hand-drive_b05.jpg` | cars are left-hand drive: Rico enters the left door, sits at a left-hand wheel |
| Winding | rbm triangles vs their stored normals | 3837 of 3837 jeep-body triangles are counter-clockwise about their normals (right-handed cross product) |
| Rico's skeleton | `biped.bsk`, idle | `RightUpLeg` sits at +x of `LeftUpLeg` |
| The map | `pda_map_dif.dds` vs `terrain_win32.dat` | north-up image matches the height map as stored with x east, z south (97% sea/land agreement); the start area is snowy north-centre (`reference/ui/pda-map-overview-…-snow-north_l10.jpg`) |
| The sun | `jc2_formats::weather::sun_direction` | up on +x in the morning, in +z at noon: east and south, as the map has them |

With the nose at −z and the driver's left at −x, right = forward × up = (−z) × y = +x: a
right-handed frame. The counter-evidence — "in-world text reads correctly" — does not
hold up: nothing mirrored twice (UVs are taken as stored, and the winding was not flipped
either), so under the old mirror painted text such as the base's "043"
(`reference/world/base-rooftop-yellow-lamp-base-043-emblem_h41.jpg`) must have read
backwards (not checked on screen); the HUD's text is drawn by Bevy UI and always read
correctly, which is the likeliest source of the remark.

The proof is automated: `coords::tests::jeep_is_left_hand_drive_in_bevy` transforms the
jeep's doll and door models through the pipeline and checks, with Bevy's camera
conventions (a camera looking along `f` with +y up has `f × y` on its right), that the
steering wheel and `Door_CL1` are on the driver's left. On the old mirror it failed
("steering wheel on the driver's right, +0.56 m"); `front_faces_are_counter_clockwise_in_bevy`
failed with 0 of 3837.

## The conversion (`coords.rs`)

| function | does |
|---|---|
| `to_bevy`, `to_engine`, `bevy_xyz`, `engine_xz` | points, directions, normals: identity |
| `quat_to_bevy` | `[x, y, z, w]` as is |
| `mat_to_bevy` | a `Transform3` (basis rows + translation, row vectors) → `Transform`: the rows become the columns |
| `qs_to_bevy` | bone / clip transforms (rig, parachute) |
| `winding_to_bevy` | nothing to do: both are counter-clockwise |
| `MODEL_FORWARD` = −z, `MODEL_RIGHT` = +x | which way an engine model faces |
| `forward_of`, `yaw_of`, `yaw_toward`, `facing_dir` | facing and yaw: `Quat::from_rotation_y(yaw)` turns −z onto the direction; the same angle as a Bevy camera's yaw; positive turns left |

## What each site does

* **Models** (`rbm_loader`): positions, normals, indices as stored; computed normals from
  the stored winding.
* **Placements** (`world`, `scenery`, `fleet`, `npc`, `pickups`, vehicle and boat parts):
  `mat_to_bevy`.
* **Terrain** (`world::terrain_mesh`, `horizon`): vertex (cx, cz) at engine
  (x0 + 8cx, h, z0 + 8cz); normal (−∂h/∂x, 1, −∂h/∂z); triangles `a, c, b` / `b, c, d` so
  they face up. **Height-field collider** (`physics::terrain_collider`): parry row i is cell
  row i, centred at (x0 + 512, z0 + 512). **Splat shader**: world xz is engine xz, the
  normal bend is in Bevy's frame directly.
* **Havok shapes** (`physics`): boxes, capsules, cylinders, hulls and meshes as stored.
* **Skeletons and clips** (`rig`, `parachute`): `qs_to_bevy`; inverse bind poses follow;
  `grpl_hang`'s 180° root turn is still needed (it is in the data, not the convention).
* **Facing**: characters (`character`, `npc`, `civilians`, `combat`) and vehicles use
  `yaw_toward` / `yaw_of` / `facing_dir`. The chase camera behind Rico has the same yaw as
  his facing. Hand offsets: the launcher and wire on his left hand (−x), the gun on his
  right (+x), both ahead (−z). Leaning forward under the chute, a corpse falling
  backwards: −/+ turns about x.
* **Vehicles** (`vehicle`): forward `forward_of(rot)`; a positive steer turns left (about
  +y) whatever the forward axis is, so `A` still turns left; the tyres roll with a negative
  turn about x. **Helicopter**: nose down is −x-turn, bank right (→) a −z-turn, yaw left
  positive. **Plane**: forward −z, right +x, the rest is written in those axes. **Boat**:
  forward −z; the rudder's push comes from the doll's stern position, signs unchanged.
* **Traffic** (`jc2_formats::roads::offset_right`): the lane is `d × y` of the direction
  of travel — driving south (+z) the lane is at −x (west). It used to be +x, the left.
* **Map** (`map`): `bevy_to_map` = `world_to_map(engine_xz)`; `map_heading` is clockwise
  from north (−z): `atan2(x, −z)`; the hero's arrow is `map_heading(facing_dir(facing))`.
  Fast travel puts him at `bevy_xyz(x, y, z)`.
* **Sky**: the sun and moon directions as the exe computes them — east +x, south +z.
* **Tile lookups** (`world`, `character`, `audio`, `heat`, `traffic`, `horizon`):
  `engine_xz`.
* **World camera**: starts south of the tile's middle looking north (yaw 0).
* **Model viewer**: `--camera-yaw 0` sits behind a vehicle; `--hide-parts hardtop,door`
  opens a cabin.

## Tests

`coords::tests` (`conventions`, `jeep_is_left_hand_drive_in_bevy`,
`front_faces_are_counter_clockwise_in_bevy`), `vehicle::tests::a_car_steered_left_turns_left`,
`map::tests::walking_left_facing_north_moves_the_arrow_left`,
`sky::tests::the_morning_sun_rises_in_the_east`, `traffic::tests::the_lane_is_on_the_right`,
the helicopter's bank check and the plane's right turn, and `rig`'s right-leg-on-+x check.
