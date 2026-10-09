# Vehicles: the `.mvdoll` doll

Implemented in `crates/jc2-formats/src/vehicle.rs`; `jc2tool vehicle <game> lave.v014_jeep.ee`
prints one. The Bevy side (`jc2-game/src/vehicle.rs`) drives a raycast car from it.

Every vehicle container (`exported\vehicles\lave\lave.v014_jeep.ee` …) holds, besides the
models, textures, `.lod` descriptors and `.pfx` collision shapes, an `.epe` blackboard with
the `CCar` entity parameters and a **`.mvdoll`**: an ordinary property container that is
the vehicle's assembly and physics definition. Its root has three groups:

| Group | Classes | What it says |
|---|---|---|
| `CFamilyProp` | nested `CReferenceProp` stubs | the part hierarchy by name and `_GUID`; no transforms |
| modules | `CCarSteeringModule`, `CCarGlobalModule`, one `CWheelModule` per axle (with a `CSuspensionModule` and a `CBrakeModule` inside), `CCarEngineModule`, `CCarRaycastModule`, `CCarPhysicalBaseModule`, `CCarTransmissionModule`, `CAiModule`, `CVehicleDamageModule`, `CBoyancyModule`s | the physics setup; every module names the editor preset it came from (`#3142b133`, `#f3e36d12` = `Editor\Vehicles\LAVE\Modules\Axis\Axis_V014_Rear.mod`) |
| parts | a tree of `CPartProp` rooted at the body, with `CPartDamageModule`s underneath | what to draw where |

## Parts (`CPartProp`)

| Key | Meaning |
|---|---|
| `#3142b133` | name (`V014-Body`, `Truck1` for a tyre, `Seat01`) |
| `model` | full `.lod` path; `#376e117f` the bare file name the container keys. Instanced parts carry an `N_` prefix (`0_v014-seats01-seat01.lod`) that the container does not — strip it |
| `pfx` / `#6511a361` | full / bare `.pfx` collision shape |
| `world` | the part's transform **in the vehicle frame**. `local` is always identity and nested parts simply repeat their parent's `world`, so nothing needs composing (unlike `.blo` placements) |
| `mass` | 100 for every part in the data |
| `is_wheel`, `#44da2bab`, `#42131844` | tyres: `is_wheel = 1`, wheel number 0…n, uniform scale of the shared tyre model (0.66 sports car, 0.98 jeep, 1.10 bus) |

This closes the question left open in milestone 2: the shared tyres, seats and steering
wheels (`Vehicles\04_SharedAssets`) are placed by these `world` matrices; the model
viewer now assembles a vehicle from them. Right-hand tyres carry a 180° turn about y so the
one tyre model serves both sides.

**Axes:** engine x right, y up, z back (right-handed, Bevy's own convention,
`coordinates.md`): a vehicle's nose points down **−z** in its own frame, its right is +x —
the driver sits on the left (−x), as in the original game's left-hand-drive cars: the driver's seat, steering wheel and the front axle (`CWheelModule` with axle index
0, `wheel_nr` 0) all sit at negative z. The doll's origin is on the ground: the hub height of
a tyre equals its scaled visual radius, so the tyre bottom is at y = 0 and the body model's
lowest vertex (0.19 m for the bus) is the ground clearance.

## Modules

Names in `code` are resolved property names (added to `property_names.txt`); `#hash` keys
are still unnamed and read **by value** across the sedan / jeep / bus:

| Module | Keys |
|---|---|
| `CWheelModule` (one per axle) | `radius` (0.31 / 0.46 / 0.51 — Havok's physics wheel, about ⅔ of the visual tyre; the car sits on the visual radius), `width`, `mass`, `friction` (1.45 / 1.25 / 1.15), `slip_angle`, `wheel_nr` (first wheel of the axle), `#3dd9d99c` axle index (0 front, 1 rear), `#c06c5451` grip 0.75–0.9 |
| `CSuspensionModule` | `direction` (≈ [−0.1, −1, 0]), `length` (travel: 0.16 sedan, 0.35 jeep, 0.45–0.5 bus), `strength` (27–35 sedan, 25 jeep, 6–14 bus: relative stiffness, front stiffer), `compression`, `relaxation` (damping) |
| `CBrakeModule` | `max_brake_torque` (600–15000), `#d201f1d5` = 1 on the axle the handbrake locks (rear) |
| `CCarEngineModule` | `rpm_min` (idle), `rpm_max`, `#c78badab` peak torque (400 / 1000 / 3500 N·m), `#c5156b97` rpm of the peak, `upshift_rpm` / `downshift_rpm` lists per gear |
| `CCarTransmissionModule` | `#c9d728c7` gear ratios (`"3.25, 2.0, 1.4, 1,0.8"`), `#0739740b` reverse (≈ final drive, 3–3.25), `#97d6cc7a` gear count, `#dd70f454` top speed km/h (125–140) |
| `CCarSteeringModule` | `max_angle` (30–35°), `#c71a85bb` km/h above which the lock shrinks (35–55); the preset name says it: `Str_35D_55kmh` |
| `CCarPhysicalBaseModule` | `#a2b2218e` chassis mass (1300 / 4000 / 20000 kg), `#3e5f5109` centre of mass (`[0, 0.65, 0.05]` jeep), `#537e1767/#68eae866/#8482038f` three lengths (inertia box?) |
| `CCarRaycastModule`, `CCarGlobalModule` | Havok raycast-vehicle tuning (`#c46d206a = −5000`, …); not used yet |

## How jc2-game uses it

Chassis: a dynamic Avian body with the convex hull of the body model, the doll's mass
and centre of mass, and the hull's inertia scaled to that mass (Avian's
`AngularInertia::from_shape(collider, mass)` passes the mass to parry as a density and
comes out a hundred times too large for a car-sized hull — the bus would not turn).
Wheels: one ray per tyre from the hub plus 45 % of the travel downwards, length
travel + radius; spring constant per wheel from its share of the weight (axle strengths)
so the car rests where the artist put the wheels; damping from the module's
compression/relaxation. Tyres: lateral force saturates at a slip angle of 8° (a fixed
sideways-speed threshold makes a 20 t bus crab instead of turn), friction circle with the
drive/brake force, handbrake halves the rear grip. Engine: a flat torque curve around the
peak, automatic gear choice from wheel rpm, quadratic drag tuned so the top gear balances
at the data's top speed.

## Helicopters (`arve.*` dolls)

The same `CPartProp` tree (body, doors, `RotorStill4` / `RotorBlurred4` / `RotorAxel` at
the hub, `RotorStillTail` / `RotorBlurredTail` on the tail boom, `Air01` landing-gear
tyres, seats), the rotor models shared from `general.blz` (`rotor1_lod1-*.rbm`). The
modules — names found by hashing word combinations, the rest inferred from how they
scale across v009, v059, v060, v061 and v062:

| Module | Fields |
|---|---|
| `CHelicopterPhysicalBaseModule` | `#4365a91d` **mass** (1000 v059 … 10000 v062), `center_of_mass` (`[0, −0.5, 0]`), `#b92afbce` 150–155 (top speed km/h, assumed), `linear_damping` 0.11–0.15, `angular_damping` 0.75, `inertia_pitch/roll/yaw` (4/2/6, scale factors), `rotor_max_rpm` 155–160, `#4a1c6ec6` 20000–25000, a dozen more |
| `CHelicopterEngineModule` | `#ec5a5e47` 16–20 × the mass (**full lift**, N: 2 g on the v062), `#2936b4b4` 2.5–4.8 × the mass (push down), `#1ad460f8` 10–16 × the mass, `#28a5d089` 90000–100000, `#d40d6067` (torque?), `#bd6148c1` 0.1, `#c522f10d` 20; v059 and v061 carry two |
| `CHelicopterSteeringModule` | `angular_yaw_speed` 1.25, `angular_pitch_speed` 0.23, `angular_roll_speed` 0.2, `max_steer_angle` 0.3 rad (the body's tilt) |
| `CHelicopterRotorModule` (×3) | `rotor_axis` (`[−1, 0, 0]` tail), `rotor_direction`, `rotor_steer`, `torque`, `#39188b7e` the main rotor's up `[0, 1, 0]` |
| `CHelicopterRudderModule` (×2), `CHelicopterGlobalModule` (`air_density` 1.3, `boyancy` 1.5), `CBoyancyModule` (×4: `position`, `radius`, `drag` 1000) | not used yet |

`jc2tool vehicle <game> arve.v062_chopper.ee` prints them. jc2-game (`heli.rs`) flies it
arcade-style with those numbers: the rotor spools up in 4 s while someone is in it, the
collective sets a vertical speed (14 m/s up, 10 down) that the rotor reaches within its
full lift, the stick tilts the body up to `max_tilt` and the tilted lift carries it,
quadratic drag makes full tilt top out near the top speed (120 km/h measured on the
v062 with its damping), the pedals turn it at `angular_yaw_speed`; blurred rotor discs
replace the still blades above half speed.

## Boats (`seve.*` dolls)

| Module | Fields |
|---|---|
| `CBoatGeneralModule` | `#4365a91d` mass (2500 v083 … 10000 yacht, the helicopter's key), `center_of_mass`, `boyancy` 1.9–2.4, `linear_damping`, `angular_damping`, `#a94eda6c/#c1f9d442/#d9336205` (2.4–3.25 / 1.5–1.75 / 2.1–3.5), `#054597f6` 100–500 |
| `CBoatEngineModule` | `acceleration` 24000–60000, `#7b40a502` 29000–50000 (forward thrust, assumed), `#ed1dff74` 25000–40000 (reverse, assumed), `#b1330cf5` `[0, −0.75, 0]`, two direction vectors |
| `CBoatSteeringModule` | `#6fd2bd7f` 15–35 (rudder lock, degrees), `#42c5ccc5` 0.75 |
| `CBoatRudderModule` | `#11d7375d` the rudder/propeller position (`[0, −0.5, 4.5]`: aft, below the waterline) |
| `CBoyancyModule` (9 on the v083) | `position` — eight hull points at y ≈ 0.3 around the hull and one at the bow with `radius` 0.7 and `drag` 20 |

jc2-game (`boat.rs`): each point lifts `boyancy × weight / points` scaled by its depth
(full at 0.25 m) with heave damping, the hull resists sideways motion (keel) and meets
quadratic drag tuned so the thrust tops out near 80 km/h, the propeller pushes at the
rudder's position turned by the rudder (applied at the centre-of-mass height: lower,
it stood the hull on its tail), the rudder bites on the flow. `--boat` moors one on the
nearest open water within 6 km of the start (found in the global height map). Helicopter
dolls carry buoyancy points too (`drag` 1000) — not used.

## Planes (`arve.*` dolls with a `CAirPlaneController`)

Planes are built from the helicopter modules (the speed plane `v067` even ships a
`.vdoll` instead of an `.mvdoll`, with five upgrade levels per module under
`CNullProp`s) plus surfaces. What tells them apart is the `CAirPlaneController` (the
driver's seat: `Up`/`Down`/`Left`/`Right` actions, `local::weapon1` mounts).

| Module | Fields |
|---|---|
| `CHelicopterPhysicalBaseModule` | `#4365a91d` mass (1500 speed plane, 5000 fighter, 12500 business jet, 100000 cargo plane), `center_of_mass`, `inertia_pitch/roll/yaw`, `#b92afbce` 150 everywhere (jc2-rs: top speed, m/s), `#c135b0b0` 160 |
| `CHelicopterEngineModule` | `#ec5a5e47` the thrust (31000 / 52500 / 110000 / 1000000 N: 10 m/s² on all of them), `#1ad460f8`, `#28a5d089`, `#d40d6067`, `#2936b4b4` |
| `CHelicopterSteeringModule` | `angular_pitch_speed`, `angular_roll_speed`, `angular_yaw_speed` (the fighter 1.1 / 1.2 / 0) |
| `CHelicopterAerodynamicModule` ×3 | the wings and the tailplane: `#57582d19` position (x = ±5…10 m; the tail at z = 6…20), `normal` up, `#8d840498` area (1 … 35), `#abd2b2ba` lift coefficient (1.25–2.5), `#ba5d47fc` drag (0–1), `#f1637111` share (0.47/0.53 left/right, 0 on the tail), `#6f8dc0f5` 1.3 — the air's density |
| `CHelicopterRudderModule` ×4 | the same plus `#cd9322f1` the axis it turns about (`[0,1,0]` yaw, `[1,0,0]` pitch) |
| `CHelicopterRotorModule` | the propeller or jet: position, direction `[0,0,−1]` |
| `CWheelModule`/`CSuspensionModule` | the landing gear, as on cars (`jc2tool vehicle` lists the tyre parts) |

The helicopters have aerodynamic and rudder modules too (the chopper two rudders), so a
wing alone does not make a plane.

jc2-game (`plane.rs`): thrust × throttle along the nose, quadratic drag that tops out at
150 m/s, lift = Σ(½ · density · area · lift coefficient) × v² × (1 + α/0.12) up to a
35° stall, doubled (`LIFT_GAIN`, tuning: the fighter stalls near 230 km/h, the data
alone put it above 330), along the body's up; the stick turns the body at the doll's
rates (scaled down below the stall speed), the nose weathervanes into the airflow and
side-slip bleeds away. On the ground a raycast spring under each tyre (sagging 0.12 m
under the weight, critically damped ×0.6) carries it, rolling freely forward and gripping
sideways; the rudder and ailerons steer the nose wheel. The fighter reaches 175 km/h in
6 s of runway. `--plane
[arve.v067_speedplane.ee]` parks one by Rico; the parked planes of the airfields come
with their tiles' spawn sets.

## Seats, crews and hijacking

**Seats.** Besides the stunt positions (`character.md`, Stunts) every doll's
`CSmartObject`s name its seats (`jc2_formats::vehicle::Seat`, `VehicleDoll::seats`;
`jc2tool vehicle` lists them): `SO_Driver` carries the vehicle's controller
(`CHelicopterController`, `CAirPlaneController`, the car's) with `CEnterDriverSeatAction`,
`CExitDriverSeatAction`, `CParachuteFromDriverSeatAction` and, on helicopters,
`CEnterDriverSeatFromRoofAction` from the belly; `SO_Shotgun`, `SO_L1`, `SO_R1` carry a
`CPassengerSeatController` (zone tag `HELI` on helicopters) and enter/exit triggers. The
smart object's `world` is the seat (engine axes): the jeep's driver on the left (−x),
the chopper's at (−0.33, −0.82, −3.91), the attack helicopter's at the centre line
(0, 1.81, −3.59), the motorboat's (−0.48, 0.46, 0.14). The v009 doll lists `SO_Driver`
twice (kept once).

**Who drives: not in the data.** The tiles' vehicle spawn sets (`CSpawnSet` with a
`CVehicleComponent`: `#fa726895` the container, `grappling_weight_category`,
`invulnerable`, disable events) carry no team, character or seat: the village's attack
helicopter, trucks and APC stand parked and empty, as in the original. `worldsim.bin`
spawns its cars, boats and planes driven (by `worldsim.aiteam`, people from
`character_types`); the vehicle types' `#6ffccc84` (1 on cars, bikes, small boats and
planes, 0 on buses, trucks, the barge, the junk and the indo boat) is unexplained.
`heatspawn.bin`'s units carry `num_seats` (the military jeep 4, the small jeep 2, the
ATV and the bikes 1, the helicopters 2, the military boat 3).

**In jc2-game (`crew.rs`).** A `CrewRequest(CrewKind)` on a vehicle (from the fleet's
`Request::crew`) makes it crewed (`Crew`) at once and asks for the driver's outfit: a
soldier rolled from `enemytypespawnsettings.bin` (`pd_gov_base01`/`pd_gov_elite`) or a
generic civilian (`pd_generic_male_1/2`, `pd_generic_female_1`), dressed with the
soldiers' and civilians' code (`npc::request_outfit`, `npc::dress`). He is a child entity
of the vehicle with his model's feet frame on the `SO_Driver` point (no collider, no
physics), playing `mv_driving_fwd` (biped) or `civ_driving_fwd` (civilian skeleton). Who
gets one: every traffic car (a villager), the heat's vehicles (soldiers, `spawning.md`),
and with `--crewed` the vehicles of `--heli`/`--plane`/`--drive` (soldiers) and `--boat`
(a villager). A crewed helicopter or boat that nobody else steers gets a pilot: the
helicopter circles its start 60 m out and 35 m over the ground at 10 m/s
(`HeliPilot`: it steers on the velocity's error with stick and bank, since the hull
slides; round 50 m the helicopters' 17° tilt holds about 10 m/s), the boat cruises in a
tight circle (`BoatPilot`: throttle 0.4, rudder 0.7; it does not look out for the shore). Both use `AiDriven`, which the
helicopter, boat and plane inputs now honour like the cars' (and the rotor spins up for
an AI pilot).

**Hijacking.** `E` with a crewed car or boat nearest (the ordinary `E` range), or `E` on
any crewed vehicle's stunt position: the driver is thrown out — to the left of a car or
helicopter, off the stern of a boat, 4.5 m/s out and 3 m/s up plus the vehicle's
velocity, turning over at 3 rad/s (`Thrown`: a ballistic fall without a physics body
until a ray finds the ground or the sea; he lies on his back 30 s and goes) — and Rico
is seated and in control. A crewed helicopter or plane is hijacked only from its stunt
position ("When all guards are dead, press E to pull out the pilot"; the guards
condition is skipped: nobody else rides along); `E` beside it does nothing. Every
hijack counts in the `Hijackings` resource (`count`, `last`), which the HUD announces
as "Hijackings: N" (`statistic_hud_txt_14`). The thrown-out animations of the original
(`cockpit_victim_throw_out`, `boat_victim_thrown_out`, `hjk_sit_pull_rgt_out`, …) are not
played: a falling clip and a tumble stand in.

Headless tests (`crew::tests`): the jeep with a villager at the wheel — seated on the
left, `E` beside it throws him 5.6 m out to the left where he lies on the ground, Rico
drives it 3 s, the driver is gone after 30 s; the crewed chopper — `E` from the ground
does nothing, `Q` hangs Rico under it, the pilot flies off with him, `E` throws the pilot
out (he lands on the ground) and Rico climbs with `Space`.
