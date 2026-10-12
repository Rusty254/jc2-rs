# Water-entry splash, impact magnitude and projectile water hits (JC2-217)

11 October 2026. Research only: no Cargo, GPU, game launch or runtime edit. Resolves Gap 3 of [water-sea-consumers.md](water-sea-consumers.md) (JC2-189):
the gates, magnitudes and effect hand-off of the water-entry code in `FUN_00572770`, `FUN_006d7130`, `FUN_0077db80`, the "impulse" virtual (vtable `+0x2c`),
the projectile water hit `FUN_006e3f60`, the character effects `FUN_006fa510` / `FUN_0052e6b0` and the effect placement of `FUN_00551ec0` / `FUN_007c8790`.
Probe: `research/tools/water_splash_impact_probe.py` (51 checks, exits 0 only if every one passes; pseudo-C fragments are asserted with whitespace
normalised, vtables and call sites are found by scanning the image). Addresses are runtime addresses of `JustCause2.exe` (sha256 `df99a479...4f80`, no ASLR).

```
python -B research/tools/water_splash_impact_probe.py [--game <game folder>] [--pseudo <git-ignored _unnamed export dir>] [--json] [--verbose]
```

## Answers

1. **The "impulse" virtual is not a physics impulse. It is an impact-effect trigger.** Every consumer calls `vtable + 0x2c` as
   `(position, unused, direction, magnitude)` at `(x, wave height, z)` with direction `(0, 1, 0)`. The three implementations only build a
   transform from the position and direction (a basis from `FUN_004dc8a0`) and hand `magnitude` to `FUN_00543ec0`, which drives an effect slot:
   * `CSimpleRigidObject` / `CTrafficLight` (and the animated subclasses): `FUN_00558e30`, effect object at `this + 0x120`.
   * `CDamageableObject`: `FUN_0054f9b0`, effect object at `this + 0x1a0`.
   * every vehicle class (`CVehicle`, `CAirVehicle`, `CHelicopter`, `CLandVehicle`, `CMotorBike`, `CSeaVehicle`, `CAirPlane`, `CCar`, `CBoat`): `FUN_0072c2a0`,
     effect object at `this + 0x4e4`. It stores the magnitude at `this + 0x2e4`, **clamps it to 0 .. 100000, and fires only when the vehicle's speed squared
     (virtual `+0xc4`) is above 1.0**.
   No Havok call is made. The object is therefore never pushed by a splash; the entry only plays an effect. (`FUN_00543ec0` has exactly these three callers.)
2. **Effect slot behaviour (`FUN_00543ec0`, `FUN_0058b9c0`).** The magnitude is written to the effect's variable only when it is strictly larger than the
   slot's last magnitude (`+0x18`); the effect asset is resolved lazily from its id (`FUN_0064fed0`). The effect starts when it never ran, and restarts when the
   magnitude is strictly above a threshold; after each start the threshold is **1.5 x that magnitude** (`+0x140`). A smaller or similar splash in quick
3. **Simple rigid objects (`FUN_006d7130`, per update; ECX is the object's secondary base, object + 4, and every offset here is ECX-relative).** Position `(+0x70, +0x74, +0x78)`, flag word `+0xbc` (bit 2 = latch).
   * `y >= 200.2` clears the latch. Otherwise, with the latch clear and **`y < 200.0`**, the latch is set and the entry test runs.
   * Entry test: the body's cached linear velocity (`FUN_009b06f0`: body `+0x1a0..`) has **`vy < -0.5`**.
   * `depth = 200 - terrainHeight(x, z)` (`FUN_00a646a0`, the terrain query of water-waves-next.md section 1), capped at **1.0** (when `1 - depth < 0` the cap is used; a
     negative depth where the terrain is above the sea is kept and squared).
   * **magnitude = depth^2 x |vy| x mass x 10.0**, with mass = `1 / inverse mass` of the physics object (virtual slot 2, see 6), or 1.0 when the object has no physics body.
   * The effect is placed at `(x, waveHeight(x, z), z)` (the wave term is the CPfxSea slot 2 of JC2-189).
4. **Damageable objects (`FUN_0077db80`).** Same shape with different gates: re-arm and test on `y >= 200.0`, **`vy < -1.0`**, flag word `+0x192` (ECX-relative) bit 0, cap `DAT_0106fae8` = 1.0 and
   scale `DAT_0106fae4` = 10.0. **Quirk, confirmed at the byte level (`0x77dc37`, `0x77dd90`): the latch is cleared when it is found set**, so while the object stays below 200.0 the test runs on
   every second frame, not once. (The rigid-object version keeps the latch until 200.2.)
5. **Vehicles (`FUN_00572770`, called each frame from the slot-27 update `FUN_00782c90` of every vehicle class, from `CSeaVehicle`'s own update `0x627610`, from `CAirVehicle` `0x7d0de0` and from `0x56d890`).**
   `f` = the submerged fraction, the virtual at **`+0xa8` of the vehicle's physics object (the pointer at vehicle `+0xbd8`)**: the summed submerged sphere volume
   of the last buoyancy step (hkBoyancy `+0x34`, JC2-216) divided by the total sphere volume `r^3 x 4.1887903 x count` (0x9a02f0 car/bike, 0x994e00 helicopter/plane, 0x9b9d60 boat; same quotient).
   * `f_prev < 0.05` arms the detector (`this + 0x358`). While armed, an **increasing** fraction adds itself to an accumulator (`+0x350`); a fraction that is not above the previous one
     (or an unarmed detector) zeroes both. So `acc` is the sum of the fraction over the consecutive rising frames.
   * Fires when **`acc > 0.5`** (`DAT_0107281c`) and the cooldown (`+0x494`) is not above 0: depth capped at 1.0 as above; **magnitude = mass x depth^2 x acc x 10.0** (`DAT_01072814`),
     placed at `(x, wave - 0.1, z)`. The cooldown is only decremented here (end of the function); its writer is not found (Gap 3).
   * After each frame `f` is stored (`+0x34c`), and the function toggles a water state on the fraction (state byte `+0x361`): above **0.8** with the byte clear it clears `+0x35a` / `+0x35b`, sets the byte and calls virtual `+0x148` (once); at or below 0.8 with the byte set and the fraction below **0.6**
     it clears the byte, sets `+0x14c` and calls virtual `+0x144`. The land vehicles' virtuals notify the vehicle's smart objects; their meaning is not decoded (Gap 3).
6. **Mass.** Virtual slot 2 of every `CPfx*` physics class (car, bike, helicopter, plane, boat, vehicle) is the thunk `0x9ac380`: body (`+0x10`) `+0xd0` -> `FUN_00b6f970`
   = `1 / inverse mass` (`+0xcc`), 0 for an immovable body. Cross-check with JC2-216 (body `+0xcc`). The simple-rigid and damageable code reads the same slot through the object's body handle (`vtable +0x1c` returns `this + 0x8c`; its `+4` member is the body, inferred
   to be the same `CPfxRigidBody` family; the retail call chain through the thunk is the proven part).
7. **Projectiles (`FUN_006e3f60`, slot 3 of `CAmmunition`, `CBulletTrail`, `CBullet`, `CGrapplingHook`, `CGrenade`, `CMissile`).** After the trace (the end point `+0x60` is the full distance or the hit point) the
   **segment from the previous position (`+0x7c`) to the end point is tested against the flat sea level 200.0** (`FUN_009c1620`: inclusive at both ends, so touching counts, with linear interpolation of the crossing point;
   it is a plain segment-plane test, so a projectile that starts under water and stays there never triggers). The hit point gets `y = waveHeight(x, z)` (JC2-189 slot 2), the normal is `(0, 1, 0)` and **virtual `+0x34`** runs. It is skipped only
   when the earlier hit handling leaves through `FUN_0066c810` (the projectile was consumed). Per class:
   * `CBullet` `0x68fec0`: position := hit + the origin offset, **`+0xa8` (speed/energy term, unnamed) halved**, impact effect with the water flag, and the follow-up `FUN_007579f0` when `+0xe4` is set.
   * `CGrenade` `0x7b47f0`: position := hit; `+0x13c := DAT_0106cea4` = **3.0** (the field is unnamed; it is the only state change).
   * `CMissile` `0x6aa2d0`: sets the weapon's state word (`+0xe04`) to 4, rebuilds an identity matrix at the hit point and calls the `+0x44` virtual (the explode/hit path).
   * `CGrapplingHook` `0x5705f0`: impact effect with the water flag, then position := hit + offset.
   * `CAmmunition` `0x9a4cf0`: `ret 4` (nothing); `CBulletTrail` `0x742000` only moves a vector (not decoded).
   Terrain hits (`FUN_00613eb0` for bullets) spawn their effect only with a probability from the surface record (`+0x60`) and apply a ricochet/stop test with `+0x5c`; **the water hit has no random gate**, it always spawns.
8. **The effect hand-off for water is a (key, value) pair, `(0, 1)`.** `FUN_00658e10` / `FUN_005f6660` build one 8-byte entry `{key, value}`: water gives **`(0, 1)`**, ground gives `(1, surface material id)`
   (the id comes from the terrain query, `FUN_00a5f390 -> +0x24`, or the hit object); `FUN_006774b0` appends it to the parameter list and `FUN_006cd100` applies each entry with the effect-variable setter before the effect starts.
   `FUN_00551ec0` (the per-character effect controller) and `FUN_007c8790` (model effects, `CAnimatedModelEffects`) use the same pair: a point **below 200.0 gets the pair `(0, 1)`**, a point above it the ground material.
   Which effect asset reads that variable, and what it does with it, is asset data (Gap 1).
9. **Effect placement snap.**
   * `FUN_00551ec0` (each character-effect entry, 100 bytes): when the effect's world y is **strictly below 200.0** it is replaced by `waveHeight(x, z)` (the entry's tracked point keeps updating); the velocity used to drag the effect
     towards its target is `(target - current) / max(dt, 0.001)` with the target y **floored at 200.0** and the speed **capped at 5.0 m/s**.
   * `FUN_007c8790` (a model effect): casts a 10 m ray down (`0x41200000`); with the effect point at `y >= 200.0` it uses the ray hit (material from the hit object or terrain); below 200.0 it uses the water pair and, when the
     entry's flag byte (`+0xc`) is set, places the effect at `(x, waveHeight, z)` with the normal `(0, 1, 0)`.
10. **Character water effects (`FUN_006fa510`, called once per update by `0x5c5b10`).**
    * **Underwater effect:** while the camera's y (`camera + 0x58`) is below 200.0 an effect (id at `this + 0x764`) is kept at the camera; at or above 200.0 it is released. (Flat 200.0, not the wave height.)
    * **Surface effects while swimming** (`FUN_0082c7e0` = the character is in one of eight swim states, among them `S_SWIM_RUN_TO_SWIM` and `S_SWIM_SUBMERGE`): an effect is kept at `(x, waveHeight(x, z), z)` of the
      character. A velocity vector with **squared length <= 1.0** selects the calm effect (id `this + 0x758`), a faster one the moving/wake effect (id `this + 0x75c`); leaving the swim states releases both.
    * **Entry splash:** when the character's y (translation of its matrix copy at `this + 0x6a4`) is below 200.0, the latch (`this + 0x748` bit 0) is clear and the character is **not yet** in a swim state (so it is landing from the air), the latch is set and `FUN_0052e6b0` runs with
      `magnitude = (character field +0x2c8) x DAT_0106f350 = 80.0`. The latch clears at `y >= 200.0`.
    * `FUN_0052e6b0`: **gate `magnitude > 300.0`** (`DAT_0106eca8`). With a signed effect id at `this + 0x74c` >= 0: `depth = max(200 - terrainHeight, 0)` and, **only if `depth > 0.2`** (`DAT_0106f34c`),
      the effect is spawned at the character with variable 0 = depth (`FUN_00579f50`). With no such id it plays the generic particle effect id at `this + 0x760`. Second part (any magnitude): a fall-height term
      `h = (this + 0x98) - 200.0 - 20.0` (`DAT_0106f348`); `this + 0x98` is reset to 200.0, and if `h > 0` the character takes a hit through virtual `+0x6c` of the character with value
      `(this + 0x284 x -y + this + 0x288 x h) x 0.5` (`DAT_0106f344`), mode 3. This part runs only while `FUN_0065ff80` is true (the character's state is one of ten state ids); the operands are not decoded (Gap 2).

## Constants (all initialised image values)

| Constant | Value | Where |
|---|---|---|
| `DAT_0106fae4` / `DAT_01072814` | 10.0 | magnitude scale (damageable / vehicle; the rigid object has the immediate 10.0) |
| `DAT_0106fae8` / `DAT_01072818` | 1.0 | depth cap |
| `DAT_0107281c` | 0.5 | vehicle accumulator gate |
| `DAT_01072820` | 0.05 | vehicle re-arm fraction |
| immediates 0.8 / 0.6 | | vehicle water-state toggle |
| immediates 200.2 / 200.0, -0.5 / -1.0 | | rigid / damageable re-arm and velocity gates |
| immediate 0.1 | | vehicle effect height below the wave |
| `DAT_0106cea4` | 3.0 | grenade water-hit field `+0x13c` |
| `DAT_0106eca8` / `DAT_0106f34c` / `DAT_0106f348` / `DAT_0106f344` / `DAT_0106f350` | 300.0 / 0.2 / 20.0 / 0.5 / 80.0 | character splash |
| immediate 100000.0 | | vehicle effect magnitude clamp (`FUN_0072c2a0`) |

## Proven / Inferred / Unresolved

**Proven** (asserted fragments plus pinned bytes): the three entry functions' gates, constants and formulas; that vtable `+0x2c` is the effect trigger and the exact three implementations; the clamp and speed
gate of the vehicle version; the effect slot's strict-greater rules and 1.5 threshold; the projectile segment-plane test, its call order and the per-class virtual addresses and bodies; the water pair `(0, 1)`; the 200.0
effect snap in `FUN_00551ec0` / `FUN_007c8790`; the character gates (300, 0.2, 80, camera 200); the mass thunk and the submerged-fraction quotient.
**Inferred**: that the object's body in the rigid and damageable code is the same `CPfxRigidBody` family as the vehicles' (the handle is reached through `vtable + 0x1c`); the names "submerged fraction", "latch", "accumulator"
(offsets and arithmetic are proven, the labels are mine); that key 0 means "water" and key 1 "material" in the pair (only the values are proven); that `FUN_00551ec0`'s `(0,1)` flag has the same meaning.
**Not native-proven here:** the retail frame rate and exact `dt` of these updates; the default value of the vehicle cooldown at construction.

## Gaps (each stays a native unknown; none is filled by a guess)

1. **Which effect assets play and how they use the variables.** The ids behind the three effect slots (`this + 0x120`, `+0x1a0`, `+0x4e4`), the projectile impact set (`this + 0xc0`) and the
   character ids (`this + 0x74c / 0x758 / 0x75c / 0x760 / 0x764`): where each is filled (property keys), what the effect-variable keys 0 and 1 and the magnitude variable (slot `+4`) drive inside the effect
   components, and whether a dedicated water effect exists per class. Blocks: visible splash, bullet-hit-water spray and wake parity. Filed as a research task.
2. **Character splash operands.** The scalar `character + 0x2c8` (and its neighbouring vector `+0x2bc..`), the fall-height tracker `this + 0x98`, the damage-style call through virtual `+0x6c` and its operands `+0x284 / +0x288`,
   and the wetness ramps `+0xa64 / +0xa68 / +0xa6c` of `FUN_00551ec0` (neck depth against 201.8 over 1.8 m). Blocks: character splash strength, fall-into-water damage and wet-skin parity. Filed as a research task.
3. **Vehicle water state.** The writer of the cooldown `vehicle + 0x494`, and what the virtuals `+0x144` / `+0x148` do at the 0.8 / 0.6 fraction thresholds for each class (land vehicles notify their smart objects; the sea and air versions are stubs).
   Blocks: engine-stall / sinking cues and exact splash repetition for cars and bikes. Filed as a research task.
4. Not blocking visible parity, kept here: the meaning of `+0xa8` on `CBullet` and of `+0x13c` on `CGrenade`; `CBulletTrail`'s water-hit virtual; what `FUN_009b50b0`'s record type (0x18 / 0xe) denotes; the identity
   of the 16-byte vector `FUN_009c1500` adds to the underwater-effect position (inferred: the physics origin offset); the exact effect transform of `FUN_00558e30` (a basis built from the direction).
