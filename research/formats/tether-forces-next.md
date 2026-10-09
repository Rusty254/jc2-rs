# Two-ended grapple tether: forces, constraint, cut and release (JC2-090)

Research brief for the JC2-039 runtime port. Nothing here changes gameplay; every claim
carries a native address, and `research/tools/tether_forces_probe.py` re-checks the
byte-level parts against the executable and runs the numeric fixtures.

Scope: the *completed* two-ended tether between two non-character endpoints (props,
terrain, plain rigid objects). Endpoint categories and targeting are opaque inputs
(JC2-037); living-soldier activation is JC2-038/033; gesture timing is JC2-083/036.
Vehicles, motorbikes, helicopters and characters take extra branches that are listed
but not decoded.

## Headline

* **There is no spring.** The original keeps a *one-sided Havok limit constraint* between
  the two bodies and fires a *one-shot impulse* on each end when the second end attaches.
  The traced attach, update and cut code contains no stiffness, damping, stretch or
  break-force term. `jc2-game/src/stunts.rs` (winding 8 m/s toward 2.5 m, snap at 4 m,
  three tethers, 200 m limit) matches none of it.
* The constraint is built from `hkpGenericConstraintData` commands: pivot A, pivot B, a
  **world-frame DOF fixed at creation** (`unit(pivotB - pivotA)`), and command `0x10`
  with limits `[0, L]` on `(pivotB - pivotA) . dof`. Read from the pseudo-C; it is a
  linear limit along a fixed axis, **not** a Euclidean max-distance rope.
* Cut = `wire.strength := 0`; the next device update plays the cut effects, erases the wire
  and destroys it, which removes the constraint. **No velocity is written anywhere on that
  path**: both bodies keep whatever Havok velocity the last step left them.

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `settings/grapplingparams.bin` (raw, 1,036 B, `jc2tool cat --raw`) | `0e11e74173e5558ab330ab487d69517af28aca5696be7b0254da089fd7d567d1` |

The probe pins the sha256 of the bytes of 30 cited functions, 21 direct call edges,
21 `.rdata` constants (each also located inside the function that reads it), 9 loader key
slots and 7 vtable slots. No other asset feeds a fixture; fixture masses and distances are
synthetic parameters, not claims about a specific prop.

## The chain (two dynamic props)

`this` below is the grappling device (`CGrapplingDevice`, ctor `0x005f....`).

### 1. Attach (producer)

| Step | Native | What it does |
|---|---|---|
| fire | `CGrapplingHook` vtable `0x00eba84c` slots 11/12 = `0x00a3b0b0` (world hit, object NULL), `0x0063f150` (object hit) | both call `0x00538470` |
| validity | `0x0079efe0` | refuses (returns 0): hit below `water - this+0x114`; hit object whose `CPhysicsGameObject +0x48 & 0x7f == 0` ("kind 0"); a character already holding the first end. An invalid hit still adds the point but builds no constraint and runs no pair step |
| wire | `0x00686690` | first hit creates a `CGrapplingWire`: `+0x10 strength = this+0xd8 = 120.0` (`#59e07a13`), `+0x0c = 3`, `+0x14 = 0.5`; a second hit pops the arm point (`0x006ed630`) |
| AddPoint | `0x0083dc00` | appends the 0x5c-byte end record and, when an end already exists, builds the constraint with `0x005c6300`; sets `wire+4` (active) when an end object exists, `+8 = 0`, `+0xc = 3` |
| completion | `0x00538470` | `points == this+0xd0 (2)` is "full": store the wire in the list at `this+0x120`; if `this+0x110 (= 1, #7dd628ea)` wires are already stored, **the oldest gets strength 0**. Only then, if the hit was valid, the pair step runs |

An end record stores a game-object reference plus an object-local offset (a terrain hit has
no object: a fixed world position). `0x005eb3f0` recomputes the world point every call as
`body transform * local offset`, so a moving prop carries its anchor. The Havok pivots are
body-local (`0x00b25510`), so they ride the bodies too; a terrain end uses the physics

**Pair step** (`0x00538470` after the store; props only). With `P0`, `P1` the two end world
points:

```
d     = P1 - P0;  if |d|^2 > 1e-6: d = d/|d|            (0x00467ee0; tiny d stays tiny)
dir0  = slerp(d,  up, 0.5)        dir1 = slerp(-d, up, 0.5)    (0x00836a40, polynomial sine)
m     = 85 if CCharacter; 10000 if no body / fixed or keyframed; else getMass()
scale0 = m1/m0 if m1 < m0 else 1         scale1 = m0/m1 if m0 < m1 else 1
S     = this+0xe4 = pull_force = 500     (0x007ed6c0 for a non-character end)
J_i   = clamp(S * scale_i)   where clamp(x, mass) = x/40*mass if 0 < mass < 40 else x
                                                     (0x006eb060; mass is the body's own getMass)
impulse_i = dir_i * J_i   applied as a point impulse at the end's world point (0x009c00e0)
```

`0x009c00e0` first calls `0x00b78700` (hkpEntity activate: clears the deactivation counters
and re-queues the entity), so a sleeping body is woken by the attach. A terrain end
(no object) receives nothing. Objects of `CVehicle` type raise events:
`ply.doubletether.vehicles` (both ends vehicles) or `ply.doubletether.objects` (exactly one);
two plain props raise neither. Vehicle/motorbike extras in the same block (`0x00825620`,
a `+0x94` method, `0x00793d40`) are not decoded.

Direction detail: `slerp` is the exact transcription, including its polynomial sine, so a
horizontal pull gives `(0.70781, 0.70781, 0)` rather than `(0.70711, ...)`, and `d == 0`
(both anchors at one point) gives `0.70781 * up`.

### 2. Constraint (`0x005c6300`, called from AddPoint and from re-length)

* Same body on both ends (or either body missing): nothing is built (early exit).
* Command stream: `1`=pivot A, `2`=pivot B (body-local, transformed at solve time),
  `5`=DOF index 0 stored raw (world frame), `0x10`=limit row `(min, max) = (0, L)`
  (`0x00b711b0` -> `0x00c52e20`); instance priority byte `2`; added to the world with
  `0x00b77a00`. AddPoint passes no extras, so the optional second DOF of `0x005c6300`
  is absent for props.
* `L`: AddPoint passes `0.0` for a pair of non-vehicles and `0x005c6300` then uses the pivot
  distance `D0`, i.e. **the rope starts exactly taut**. If either end is a `CVehicle` it passes
  `|P1-P0| + (|v0| + |v1|)*dt + 2.0` with `v` the velocity at the pivot (`0x0099a5f0`) and
  `dt = 0x004d3690` (1e-6 when the timer is paused, else its frame delta).
* The DOF axis never changes after creation; every re-length rebuilds the constraint from the
  current pivots and so re-aims it.
* Hazard (decompile-read, never observed): `D0 == 0` between two *different* bodies divides by
  `rsqrt(0)` when the axis is normalised.

### 3. Update (`CGrapplingWire` vtable `0x00eba9c8` slot 3 = `0x006a72e0`, once per frame)

Called from the device loop `0x0083fe20` with the frame delta. For a wire with two
non-character, non-helicopter ends:

1. `points < 2` zeroes the strength; `strength <= 0` returns.
2. The constraint is kept in the world (`0x0078be30`/`0x00b77a00`); the speed-based
   "moving" test is overwritten for non-characters, so props never disable it.
3. **Cut guard**: if either end's game object reports an entity record (virtual `+0x64`)
   whose flags word (`+0xc`) has **bit 4 clear**, and no character end rescues it,
   `strength := 0`. Bit 4 is set to 1 (`|= 0x11`) every device update on the two objects at
   `this+0x140/+0x144` and cleared on cut; what else clears it is unresolved.
4. **Ratchet** (`0x007f3520(0.2 - slack)`, `slack = L - dist`): fires only if no end is a
   character, **exactly one end's `CPhysicsGameObject +0x48` kind is 1**, an end is a fixed body
   or has no body (`isFixed`, `0x009ae370`), and `slack >= 0.4` (`< 0.4f` fails, so equal
   passes). The result is `L := dist + 0.2`. `0x007f3520` refuses to shorten below the current
   distance and raises a short lengthening to the distance:
   `if L+delta < dist: if delta <= 0: no-op else delta = dist - L`.
   **Two dynamic props never ratchet**: `L` stays at its creation value until the cut.
5. `0x00561820` (the only force-shaped rule) needs a `CHelicopter` end with flag `+0x728`;
   it cannot run for props. It applies `F = u*m_min*(9.82*u.y + 100*dt)` to B and `-F` to A when
   `9.82*u.y >= 0.2`, sets B's damping to (0.25, 0.25), and may shorten by `0.1*slack`.

### 4. Cut and despawn (device loop `0x0083fe20`, per stored wire)

| Trigger | Mechanism |
|---|---|
| A newer completed tether | strength of the oldest := 0 (above; `this+0x110 = 1`) |
| `0x00570730` | sets strength := 0 on every stored two-point wire. Callers: `0x005137e0 0x005f5c30 0x006b3210 0x006e0100 0x00706c30 0x0079dcb0`, and (inferred from the branch at `0x00539988`) `0x00538470` itself when a shot is invalid and no wire is active |
| Range | last point more than `this+0xec` = `500^2` (`#d57278d7` = 500 m, stored squared) from a global reference position (`DAT_011919f4->+0x10 + 0x54`; probably the player or camera, unresolved): wire erased and destroyed, no cut effects |
| Cut guard in the update | bit 4 test above |
| Age | `wire+8 += dt` only for wires with a living character end; compared with `this+0x16c` (ctor default 5.0). **Two non-character ends never age.** |

When `strength <= 0` the next loop pass spawns two effects at both end points, erases the node,
calls the wire's deleting destructor (`0x0056b250` -> `0x0071fe30` -> `0x00716800` per
end -> `0x005de1e0` -> `0x00b77b50` remove-constraint) and posts the UI event
`gui_aim_dualhook_disable_event`.

### 5. Velocity at release

* The cut path writes **no linear or angular velocity**. The only direct velocity inputs in
  the whole tether lifetime are the attach impulses; after that Havok's solved velocity is
  what survives.
* `0x00716800` (end-record release) **restores the end body's linear and angular damping**
  from the record (`+0x44/+0x48`) and sets a quality type (`2`, or `1` for a `CVehicle`
  passing `0x005d9640`). The attach path appears to call `setQualityType(1)` on the hit body
  (`0x00b79e10` near the top of `0x00538470`; receiver not pinned down). These are the only physics-state side effects of the cut.
* So the "inherited" velocity is the Havok body velocity at the moment of removal, with the
  damping and quality restored. Nothing is zeroed, clamped or reflected.

## Boundary fixtures (probe, `fixtures()`; 41 entries, all pass)

| Fixture | Input | Result |
|---|---|---|
| equal props | 100 kg / 100 kg, 10 m apart on x | `scale 1/1`, `J = 500*0.70781`: impulse `(353.906, 353.906, 0)` and mirror `(-353.906, ...)`, `dv = 3.539` m/s each |
| heavy vs light | 100 kg / 25 kg | scales `0.25 / 1`; magnitudes `125 / 312.5` (25 kg is also <40: `500/40*25`) |
| light clamp | 10 kg / 10 kg | `500 -> 125`; `dv = 8.85` m/s |
| clamp edges | mass 39.9, 40, 0 | `498.75`, `500`, `500` |
| same point | `P0 == P1` | `d = 0` stays 0; `dir = (0, 0.70781, 0)` |
| `|d|^2` gate | `1.21e-6` vs `8.1e-7` | normalised `(0.7078, 0.7078)` vs left tiny `(0.0006, 0.7078)` |
| directly above | `P1` straight above `P0` | parallel branch, `dir0 = up` |
| directly below | `P1 = P0 - y` exactly | antiparallel branch: `dir0 = (0, 0, 7)` (perpendicular of length 7; a decompile-read quirk) |
| near antiparallel | `|dot|` within 1e-6 of -1, not equal | linear branch halves two opposite vectors: `dir ~ (0.0005, 0, 0)` |
| static first end | terrain + 100 kg | no impulse on terrain; the prop gets `scale 1` |
| fixed body | 100 kg + fixed body | `mass 10000` for the ratio: `scale0 = 1`, `scale1 = 0.01` |
| initial rest | props, 10 m | `L0 = D0 = 10.0` (taut) |
| initial rest | vehicle end, speeds 3+4, dt 1/60 | `10 + 7/60 + 2 = 12.1167`; speeds 0 -> `12.0` |
| re-length | `L=10` | shorten to dist 8 by -1 -> 9; dist 9.5 by -1 -> refused; short lengthen to dist 12 by +1 -> 12; +5 at dist 10.6 -> 15 |
| ratchet | `L=10`, dist 9, one kind-1 body + fixed end | `9.2` |
| ratchet gate | slack `0.4f` / `0.4f - 1 ulp` | fires / does not |
| ratchet, two props | kind-1 count 2 | never |
| heli force | A 15 m above B, 8000/500 kg, dt 1/60 | `F = 500*(9.82 + 100/60)` on B, `-F` on A, re-length `-0.1`; gate closes at `9.82*u.y < 0.2` |
| lifetime, more than one frame | frame 0 update; second tether evicts the first; frame 2; frame 3 at 501 m | `[update]`; `[cut effects, wire deleted + constraint removed, UI event, update]`; `[deleted out of range, UI event]`; list empty |

The fixtures lock the transcription and its boundaries. They are **not** runtime captures:
`dt`, the Havok solve and rigid-body inertia (angular part of the point impulse) are inputs the
probe does not model.

## Verified / inferred / unresolved

**Verified in bytes (probe fails if it drifts):** executable fingerprint; the sha256 of the
30 cited functions; the call edges of the chain; the `.rdata` constants and their use sites;
the loader key slots (`#76fea4b2`->`+0xd0`, `#59e07a13`->`+0xd8`, `#537a05d4`->`+0xe0`,
`#2a925441`->`+0xe4`, `#b55417ae`->`+0xe8`, `#d57278d7`->`+0xec`, `#7dd628ea`->`+0x110`,
plus the unresolved `#da24282b`->`+0xd4`, `#a73771bf`->`+0xdc`); wire vtable slot 3; the
`grapplingparams.bin` values. Correction to `character.md`: `#59e07a13 = 120` is the wire's
initial strength, not a flight speed; `#7dd628ea = 1` is the stored-wire limit and
`#76fea4b2 = 2` the points per wire.

**Read from the pseudo-C (transcribed, not runtime-verified):** every formula above, the
branch structure, the world-fixed DOF, the cut ordering.

**Inferred (labels):** `0x0098f6e0`/`0x009ae370` as Havok motion type not 6/7 and exactly 7;
`0x00b6f970` as `1/inverseMass`; type tags (`CCharacter 0x0056aec0`, `CVehicle 0x005265b0`,
`CMotorBike 0x0050cb90`, `CHelicopter 0x00668eb0`, `CSimpleRigidObject 0x00525bb0`,
`CPhysicsGameObject 0x0077e2f0`).

**Unresolved (do not invent):**

1. Meaning of `CPhysicsGameObject +0x48` kinds 0/1/2 (feeds the gate, the ratchet and the
   character strength table). Treat as opaque categories from JC2-037.
2. What clears the entity flag bit 4 besides the cut, and who owns `this+0x140/+0x144`.
3. The global reference position of the 500 m rule.
4. Havok solver parameters of the limit row (tau, damping, priority byte `2`); the
   sign/frame convention of the DOF vector beyond "pivot difference at creation".
5. Whether Havok wakes a sleeping partner when the constraint is added (the traced code
   wakes only through the attach impulse); the saved damping source (`record +0x44/+0x48`)
   and the quality enum numbers.
6. Constants `#da24282b = 16`, `#a73771bf = 3` (`+0xd4`, `+0xdc`), the character angle keys
   `#00992cfb`, `#8e43b31e`, `#320b8cd2`, `#7a4ef581`, `#76a95a64`, the hook range
   `#72104687 = 8100` (applies to the *active* hook wire in the device update, not to a
   stored tether), and `this+0x16c` (age limit, character wires only).
7. Vehicle, motorbike, helicopter and character branches; `FUN_006b1230`, the third
   caller of the constraint builder.
8. The frame delta source for `dt` (variable per frame vs fixed).

## Bounded runtime handoff for JC2-039

Implementable once JC2-037 is reviewed (categories and eligibility are not decided here):

1. **Lifecycle**: keep *one* completed tether; a newer one marks the old one dead, effects
   play, the constraint goes the next device pass. Cut also on the six `0x00570730` callers'
   events, an invalid shot with no active wire, 500 m from the reference, and the entity
   guard. Remove: three-tether cap, 4 m stretch snap, 200 m cap, 8 m/s winding, 2.5 m minimum
   (none appears in the traced code).
2. **Attach impulse** for two non-character ends exactly as the pair step above (`S = 500`,
   slerp-to-up direction, mass ratio, light-body clamp, applied at the end points, bodies
   woken). Terrain ends receive nothing.
3. **Constraint**: limit `[0, L]` on the projection onto a world axis fixed at creation,
   `L0 = D0` for non-vehicles. `DistanceJoint` is Euclidean and differs; parity needs a
   custom limit or a recorded deviation. The probe's `ratchet`/`re_length` are the update law
   for a terrain-plus-prop tether; two props never change `L`.
4. **Release**: remove the constraint; do not touch either body's velocity; restore damping
   and quality type. Test ideas: the 41 probe fixtures plus a two-step trace
   (`attach -> N frames -> cut`) asserting velocities are unchanged across the removal step.
5. Blockers to keep open: the unresolved list above (especially 1, 2 and 4).

## Reproduce

```
python research/tools/tether_forces_probe.py --game "<JC2 dir>" --tool <path>\jc2tool.exe
python research/tools/tether_forces_probe.py --fixtures-only
```

The first needs the unmodified game folder (reads `JustCause2.exe` and
`settings/grapplingparams.bin`, writes nothing) and a prebuilt `jc2tool`; the second runs the
pure ports only. Pseudo-C used for reading lives in `research/analysis/pseudo` (git-ignored).
