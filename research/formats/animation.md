# Skeletons and animation (Havok 5.5)

Implemented in `crates/jc2-formats/src/animation.rs` (`Skeleton`, `SplineAnimation`,
`InterleavedAnimation`), used by `crates/jc2-game/src/rig.rs` (joints, skinning, the
animator) and `parachute.rs`; `jc2tool anim <game> biped.bsk` prints a skeleton's bone
tree, `jc2tool anim <game> 'animations.blz/run_fwd.ban' --at 0.3` a clip's tracks and the
pose at a time. The packfile container itself is in `havok.md`.

## Files

| File | What | Where |
|---|---|---|
| `animations\skeletons\syncreadthis\biped.bsk` | Rico's skeleton: `hkaSkeleton` "Reference", 84 bones | top level (`pc0`) |
| `civilian_biped.bsk`, `parachute.bsk`, `cutscenegrapple_reference.bsk` | other skeletons | top level |
| `global\animations.blz` | 969 members: the character clips (`std_idle.ban`, `run_fwd.ban`, `fall.ban`, `grpl_hang.ban`, `parachute_dash.ban`, `land_chute.ban`, `run_jump_*.ban`, `swim_*.ban`, `hng_*`, `mc_racer_*`, …), the `.afsm` state machines and `.asb` animation sets | `pc0` |
| `animations\parachute\*.ban`, `animations\character\npc\…` | the canopy's own clips (for `parachute.bsk`), pedestrian poses | top level |
| `global\cutscenes\*.seq.blz` | cutscene takes (`km01_00_take01_…_char_mc01rico.ban`) | `pc0` |

The `.bsk` files ship **with** Havok's reflection data (`hkClass`/`hkClassMember` objects
in `__types__`), so their layouts can be read off the file: `jc2tool` does not yet
print them, the Python in the session did. The `.ban` files have an empty `__types__`;
the animation layout below was matched against the 6.x/7.x `hkaSplineCompressedAnimation`
and verified by the checks at the end.

## `hkaSkeleton` (36 bytes, 5.5 "simple arrays" = pointer + count)

| Offset | Member |
|---|---|
| 0 | `name` (cstring) |
| 4, 8 | `parentIndices` (i16; −1 = root) |
| 12, 16 | `bones` (pointers to `hkaBone` {name, lockTranslation}) — global fixups |
| 20, 24 | `referencePose` (`hkQsTransform`: translation vec4, rotation quat xyzw, scale vec4 = 48 bytes) |
| 28, 32 | `floatSlots` |

Rico's biped: `Reference` → `Hips` (0, 0.948, 0) → `Spine`, `Spine1`, `Neck`, `Head`
(and 15 face bones), `Left/RightShoulder`… `Hand`, fingers, `Left/RightUpLeg`… `ToeBase`,
plus `Harness`, `Pouch`, `xChain1/2`, `xCrusifix`, `Armband1/2`. The reference pose has
every rotation at identity: the T-pose the model is modelled in.

## `hkaSplineSkeletalAnimation` (type 5 — later renamed `hkaSplineCompressedAnimation`)

| Offset | Member |
|---|---|
| 12 | `duration` (s) |
| 16 | `numberOfTransformTracks` |
| 20 | `numberOfFloatTracks` |
| 24 | `extractedMotion` → `hkaDefaultAnimatedReferenceFrame`: +16 up, +32 forward, +48 duration, +52/+56 `referenceFrameSamples` (one `[x, y, z, yaw]` per frame; `run_fwd` travels −4 m in z over 1.2 s = 3.33 m/s, the speed its playback is scaled against) |
| 28, 32 | `annotationTracks` (pointers; each starts with its name, `Hips_AnimationTrack` — **this is the track → bone mapping**, the binding's `transformTrackToBoneIndices` refers to a 90-bone export skeleton) |
| 36 | `numFrames` |
| 40 | `numBlocks` |
| 44 | `maxFramesPerBlock` (256) |
| 48 | `maskAndQuantizationSize` (= 4 × tracks) |
| 52 | `blockDuration` (= 255 × frameDuration) |
| 56 | `blockInverseDuration` |
| 60 | `frameDuration` (1/30 s) |
| 64 | `blockOffsets` hkArray<u32> (12 bytes) |
| 76 | `floatBlockOffsets` |
| 88 | `transformOffsets` |
| 100 | `floatOffsets` |
| 112 | `data` hkArray<u8> |
| 124 | `endian` |

### Block data

```
mask[tracks]      4 bytes each: quantization (bits 0-1 position: 0 = 8-bit, 1 = 16-bit;
                  bits 2-5 rotation: 0 POLAR32, 1 THREECOMP40, 2 THREECOMP48, 3 THREECOMP24,
                  4 STRAIGHT16, 5 uncompressed; bits 6-7 scale), then the position,
                  rotation and scale types: bit i (0..3) = component i is static,
                  bit 4+i = component i is a spline; nothing = identity (0, or 1 for scale)
align 4
per track:
  position:  if any spline: u16 n (control points − 1), u8 degree, u8 knots[n + degree + 2], align 4
             per component x, y, z: spline → f32 min, f32 max; static → f32; identity → nothing
             if any spline: (n + 1) × (one u8 or u16 per spline component), align 4
  rotation:  spline: knots as above (no alignment), (n + 1) quantized quaternions, align 4
             static: one quantized quaternion, align 4
  scale:     like position
```

THREECOMP40: 40 bits; three 12-bit values `v × 0.000345436 − 1/√2`, bits 36-37 the index
of the omitted (largest) component, bit 38 its sign. Only THREECOMP40 and 16-bit
positions occur in the clips looked at; the others are left unimplemented.

Knots are frame indices within the block; sampling at frame `u` is de Boor on the
clamped knot vector, quaternions interpolated component-wise and renormalised.

`hkaInterleavedSkeletalAnimation` (type 1, uncompressed): +12 duration, +16 tracks,
+28/+32 annotation tracks, +36/+40 `transforms` — one hkQsTransform (48 bytes) per
track per frame, frame-major. `chute_add_idle.ban` is one, and it animates the
**canopy's** skeleton (`parachute.bsk`: `Reference`, `base`, `lft_1..4`, `rgt_1..4`…),
not Rico — the `animations\parachute\*.ban` clips are the canopy's.

## Checks

* Every track's static/first position equals the reference-pose translation of the bone
  its annotation names (84 bones, 79 tracks of `std_idle`, within 6 cm — the Hips sit
  5 cm lower in the idle than in the T-pose).
* Consecutive rotation control points of every track in `std_idle`, `run_fwd`, `fall`,
  `grpl_hang` have |dot| > 0.95 with this quaternion decode; flipping the sign bit or
  the index convention breaks `grpl_hang` (0.77, 0.00).
* The data of each block is consumed to within 8 bytes of the array's end.
* `cargo test -p jc2-formats animation` and the character test in jc2-game (joints
  spawned, the thigh swings when he runs).

## In jc2-game

`rig::spawn_joints` builds the reference-pose hierarchy and inverse bind matrices;
`SkinnedGeneral` blocks get `SkinnedMesh` components. `Animator` now evaluates a base
clip plus carrying, aiming, action and finger layers. A layer affects only the bones
with tracks; the left-arm grenade throw additionally masks descendants of
`LeftShoulder`. Crossfades retain the outgoing layer's mask and playback rate, so
replacing a throw cannot move the legs. Moving carrying layers synchronize with the
base gait phase; held canopy parameters retain their sampled time.

`character::hero_animation` chooses idle, walking/running/sprinting, strafes,
jump/fall/skydive, fall-speed-dependent landings, swimming, grapple reel/hang/climb,
and canopy poses. Playback speed uses each clip's extracted root-motion speed.
A deployed canopy keeps its pose while the grapple tows Rico. The canopy speed and
turn clips are held at parameter-selected times rather than played as locomotion.
Soldiers use `npc_std_idle`, `npc_wk_fwd`, `npc_tactical_fwd` and their own aiming/recoil
clips where available; a dragged soldier gets the stumbling layer.

`poses::WeaponClass` selects carrying, aim, recoil and reload clips from weapon hand
and AI modifiers. `Arms` counters trigger per-shot recoil, reloads, grenade throws and
grapple firing. Reload playback stretches the clip to `weapons.bin`'s reload time.
The aim constraint distributes bounded yaw/pitch over the weapon class's spine/arm
chain while the base legs keep moving; the head looks towards the target.

Guns attach to `AttachHandRight`: model barrel -z becomes the hand's +x, model top
+y becomes hand -z. The attachment bone places the grip about 6 cm along the palm.
Two-handed clips place the off hand at the handguard. Inactive guns use approximate
back/thigh mounts; two-handed guns stow while Rico hangs or holds canopy handles.
The grapple launcher is Rico's existing left-forearm gauntlet mesh, with the wire
origin attached to the left hand. Muzzle flashes/tracers use the visible gun's
transformed muzzle (estimated from its model bounds). Flashes are gun children, so
they follow the current animated hand after transform propagation, including the
first shot from a lowered gun. Tracer/round origins use the last propagated frame;
projectile/hitscan aim remains
the crosshair target. Model bounds are not a decoded muzzle socket.

The code's named clip/state mapping and blend-time/constraint comments record the
previous branch's interpretation of `.asb`, `.afsm` and `.acsb` data. There is no
runtime parser for those files yet; the mapping is handwritten. In particular this
implementation must not be described as a complete original state-machine decoder.

Checks include all requested clips loading/mapping, consistent facing, gun-to-hand
placement and rifle off-hand alignment, bounded aim with unchanged running legs,
recoil, data-timed reloads, crossfade continuity, masked throw replacement, towing
canopy priority, and an asset-backed headless gun/flash integration test. These tests
require `JC2_TEST_DIR` for the original assets; without it data-dependent tests skip.

Screenshot helpers: `--pose idle|walk|run|sprint|strafe|hang|reel|climb|chute|crouch`
with `--weapon 1..5`, `--aim`, `--firing`, `--pose-view DEG`, `--pose-pitch DEG`, and
`--pose-soldiers N`. These are visual overrides for inspection, not simulations of
all the physics prerequisites of a pose.

## Still unknown / later

* Applying root motion itself, float tracks and the other quaternion quantizations.
* General runtime `.afsm`/`.asb`/`.acsb` decoding and full original transition graphs.
* Data-defined stow/muzzle sockets, accurate weapon-specific muzzle heights, dual wielding.
* Complete vehicle entry/exit animation and original procedural IK behavior.

## Validation, 8 October 2026

The resumed animation branch passed `cargo test --release --workspace` with the
original Steam install in `JC2_TEST_DIR`: 120 format tests and 164 game tests,
284 total (matching the source test count). `cargo clippy --release --workspace
--all-targets` completed without warnings. Both commands used isolation 29 and the
shared build lock. The GUI binary was built and copied to a private name while the
lock was held; no shared executable was run.

Eight-second private-binary smoke checks saved `crouch-pistol.png`, `run-rifle.png`
and `hang-pistol.png` under the worktree's ignored `target/animation-check/` folder.
All exited successfully. The side views show the pistol grip following the palm,
the rifle aligned across the right hand with the off hand at the fore-end while the
legs run, and the pistol aiming one-handed over the hanging base pose. Reference
comparisons used `onfoot/rico-crouched-on-truck-roof-closeup-rifle-pistol-daylight_bc27.jpg`,
`combat/assault-rifle-muzzle-flash-casings-ejecting-rico-vs-soldier-terrace-daylight_an09.jpg`
and `combat/hang-shoot-pistol_c41.jpg`. The originals support the grip and stow
conventions; pose overrides use different cameras, surroundings and sampled clip
times, so these are qualitative comparisons rather than pixel-parity checks.

The running firing screenshot caught a tracer but no sustained muzzle flash; exact
flash-to-muzzle alignment, including the first shot from idle, is verified in the
headless integration test. The render checks do not prove every original transition
or the soldier reload pose matches the original (the reference set has no soldier
reload). Approximate stow mounts, bounds-derived muzzle points and handwritten state
mapping remain the limitations listed above.
