# Authored weapon accuracy: bounded player path (JC2-046)

Validated headless: `cargo test --release --workspace` (450 passed, 0 failed) and clippy clean on the merge with JC2-045 master. No GPU capture. This implements the
regular player handheld path on foot with direct camera aim. It does not certify original
AI, aim assistance, special weapon modifier behavior or ballistic origins.

## Original evidence

Executable SHA256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
The reusable read-only `research/tools/weapon_accuracy_probe.py` verifies native
byte contracts and the retail player configuration with an existing jc2tool.
Use the accepted `weapon_consumer_probe.py` for the six-level weapon fixtures.

* `006e83a0`: CWeaponSettings loads raw `#6a1fb54c` into six levels at +33c,
  converting degrees to radians, and `#52aa95ac` into six dimensionless per-round
  decrements at +36c. Independent `max_bullet_spread` at +2d8 converts degrees
  to radians. `aim_type` raw hash0d59d85f at pointereb26fc maps to +2ec.
* Global player construction0067-file caller obtains the player child `#a0f5dc0e` from
  settings/player.bin, passing it through00664440 to00788450. The recovery hash
  pointerebbb34 contains4f5ab59d. Its native default 0.5 is overridden by retail
  player child value 5.0. Values8473a36a=.3 and8baf45c4=.25 feed separate native
  fields, not this recovery rate.
* CPlayer secondary vtablee4cd64 +30 invokes007141e0 (bounds), +34 invokes
  0081fac0 (recovery). Character005c40b0 stores the recovery callback result at
  character+610. Formula is clamp(current + rate*dt, lower+618, upper+61c).
  Character constructor007ebd20 starts current 0, lower 0, upper 1.
* 007141e0 obtains the ordinary maximum from005911b0: player weapon_accuracy,
  except aim_type 1 returns1.0. Its control object at secondary+17c is full
  CPlayer+184: constructor007359b0 creates it through00775f00, explicitly
  CPlayerAimControl, with assisted target+3c initialized 0. With no assisted
  target, retail player+2f8=150 prevents the optional zero-target override.
  Thus ordinary direct aiming/movement uses the weapon bound; no speculative
  speed or right-mouse multiplier is added. A nonzero assisted target uses a
  separate +9c/+94 ratio and retail3433db87=0; that controller is unsupported.
* 005c38a0 passes character accuracy to avatar/player callback006be970; both
  output angles are baseRadians*(1-accuracy). Its indirect outputs are real
  despite misleading zero assignments in the decompiler. Native modifier types
  10/12 override callback input to1.0/.9 in particular branches. Current game
  pose/hand strings do not implement these original controller identities.
* Weapon0083f860 evaluates the callback once before its firing state machine.
  Per round006ec6e0 executes0054c980 (two aim samples), then0059b0c0 decrement,
  clamp 0..character upper, then0072aae0 (per-projectile loop). There is one loss
  even when a round emits multiple pellets. AI accessors force upgrade level 0
  and use different settings/controller policies; unchanged here.
* 0054c980 samples (004ebaf0(2)-1) independently on the original transverse/up
  aim axes. Native operand bytes54cde8 prove composition yaw*pitch. Rebuilding
  the shot frame uses004dc8a0 world-up look geometry, not a shortest-arc rotation
  with arbitrary oblique roll.
* 0072aae0 samples CRT rand values 0..32767, maps each to[-1,1], and multiplies
  all three by max_bullet_spreadRadians. Draw orderZ,X,Y reaches004e1470.
  Native row-vector Rz*Rx*Ry becomes column-vector Ry*Rx*Rz, then the world-up
  aim frame. This stage is independent of character accuracy.

## Runtime and boundary

Grapple/parachute/stunt riding shots retain their existing scatter approximation and do not
mutate the supported regular accuracy state; native modifier ownership remains
unproven. Vehicle/swimming shots retain their existing rejection behavior.

PlayerAccuracy tracks the actual Entity generation. Switching/upgrading a gun
retains character accuracy and applies the next gun's bound; replacing the owner
starts at 0. Recovery runs before input/cooldown rejection. A dry/reloading or
otherwise rejected shot performs no loss or random shot sampling. A successful
round samples aim before losing accuracy, then samples each pellet separately.
Launchers use the same two stages while retaining existing origins, damage,
gravity, velocity, blast, and lifetime formulas. Missing upgrade inputs produce
zero spread/loss, with no fabricated field values.

The dedicated random stream matches native uniform ranges/discreteness, but does
not claim the original global Mersenne Twister/CRT sequence or seed parity.
Quaternion math uses the engine library sin/cos rather than native polynomial
bit parity. Assisted-target controllers, original modifier overrides and AI
accuracy remain explicit gaps. Weapon/socket/ballistic origin parity belongs to
JC2-045. No camera kick, pose, recoil, mission or cutscene behavior is changed.

## Meaningful fixtures (passing)

Retail parser tests verify all six pistol/rifle/launcher angular/loss arrays and
player recovery 5.0, plus reject a recovery-like value outside the native player
child. State fixtures cover split delta, clamping, switching without resetting,
owner generation replacement and aim_type 1 bypass. Fixed-seed sampling checks
range, moment and repeatability; literal oblique direction/roll values were
independently computed using Rodrigues/native row matrices, catching reversed
axis order and shortest-arc roll. A real-data fire-only headless fixture verifies
one decrement through the actual pistol, shotgun pellet and automatic rifle
paths, tracer pellet count, dry rejection, owner replacement and preserved parachute/roof firing, including
a roof launcher with unchanged random stream and normalized base direction.

The shot origin/muzzle comes from JC2-045 (`gun_muzzle`); scatter is applied to the camera aim ray and, for ROCKET/LOBBED, to the direction from that muzzle. `authored_pistol_loss_recovery_and_scatter_sequence` pins authored Beretta level-0 (2.2 degrees, loss 0.07) with recovery 5.0 across a six-shot sequence.
