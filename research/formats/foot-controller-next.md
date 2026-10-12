# Rico on foot: input decomposition, jump, evade/sprint, kick, parachute and skydive (JC2-170)

Research only. Probe: `research/tools/foot_controller_probe.py` (49 checks, exit 0):

```
python research/tools/foot_controller_probe.py --game <game folder> [--report]
```

Executable fingerprint: `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
Addresses are virtual addresses. Labels: **native** = read from code/data and asserted by the probe;
**inferred** = reasoned from native facts (always said so); **unknown** = listed under Gaps.
Closes Gap 2 and the KICK part of Gap 7 of `controls-next.md` (JC2-083). Action ids are indexes into the
157-entry table at `0x01071de8` (MOVE_FORWARD 30, BACKWARD 31, LEFT 32, RIGHT 33, WALK 34, JUMP 35,
CROUCH 36, PARACHUTE_OPEN_CLOSE 71, FIRE_GRAPPLE 81, ZOOM_IN/OUT 83/84, DASH 118, STUNT_JUMP 119, KICK 124,
EVADE 129, ACTIVATE_PARACHUTE_THRUSTERS 152). "Active" below means the arbitrated record is PRESSED (one
frame) or HELD (`controls-next.md` section 3); analog value = 12-bit value / 4095.

## 1. The short answers

* **There is no routine that decomposes the shared keys.** There is no context stack either. Every
  consumer polls its own action ids and gates on its own state (animation state, controller, flags).
  The keymap overlaps (SPACE = JUMP + PARACHUTE_OPEN_CLOSE, LSHIFT = DASH + ZOOM_IN, ...) are harmless
  because each action has very few readers (table in section 6).
* `0x005a9950` is **not** an input decoder: it is the **parachute flight model** (`CParachuteController`
  update). `0x007b6520` is the **on-foot input handler** (`COnFootController` update calls it).
  `0x006b8890` is the reel-in controller, not on-foot.
* **Jump** = JUMP pressed (or buffered 0.3 s after the press while in a landing-interrupt state) ->
  animation action `ACT_JUMP_AT` accepted by the current layer-0 state. The handler applies no impulse.
* **Evade** = tap EVADE (release within 0.25 s) or, with the keyboard/mouse device, press; **sprint** = DASH
  held while moving, never within 0.25 s of an EVADE press. **Kick** = KICK pressed, buffered 0.7 s,
  executes the grapple-smash melee action on a target within 2.5 m.
* **Parachute** opens from the skydive controller on PARACHUTE_OPEN_CLOSE (needs `ACT_PULLCHORD`
  accepted); the flight model is a lift/drag glider with the curves and constants in section 8. 40 % of the
  (capped) fall speed is converted into forward speed at the moment it opens.
* **Stunt jump** (STUNT_JUMP / pad B) has no literal reader. It is a context action whose input ids are
  data (section 7).

## 2. Controller map (native)

| Controller | vftable | update (slot 6) | input work in the update |
|---|---|---|---|
| `COnFootController` | `0x00ec2470` | `0x00821a90` | calls `0x005dd210` (scope/aim, `controls-next.md`) then `0x007b6520` (this document) |
| `CParachuteController` | `0x00ec24ac` | `0x005a9950` | the glider model (section 8); reads MOVE_*, PARACHUTE_OPEN_CLOSE, FIRE_GRAPPLE |
| `CSkyDiveController` | `0x00ec2c20` | `0x00619510` | calls `0x0074edd0` (open/close driver), `0x00517090` (pitch pose), `0x007687c0` (roll pose), `0x006b9290` (freefall model) |
| `CPedestrianController` | `0x00ec2b2c` | `0x00581850` | a second input handler (`0x0063aec0`: movement, EVADE, FIRE_LEFT/RIGHT, HANDBRAKE, JUMP, CROUCH mapped to other actions); not Rico's on-foot path (inferred: a possessed or scripted pedestrian body) |
| `CStuntAreaController`, `CBaseJumpController`, `CClingStuntController`, `CRoofTopStuntController`, `CBellyStuntController` | `0x00ec2ce4`, `0x00ec2c5c`, ... | `0x005913d0`, `0x00756ff0`, ... | stunt positions; entered through context actions (section 7), not decoded here |

Animation actions are dispatched by `0x0073f7f0(hash, layer)` and the result is "accepted by the current
state's transitions" (`animation-statemachines-next.md`). The hashes are created by the static
initialiser block `0x00c80000-0x00c84000` (`push name; call hash; push dtor; mov [global], eax`); the probe
asserts these: `ACT_JUMP_AT 0x01196648`, `ACT_EVADE 0x01196e3c`, `ACT_PULLCHORD 0x01196ed0`,
`ACT_PARAGLIDE 0x011970cc`, `ACT_CROUCH 0x011970d4`, `ACT_UPRIGHT_STOP 0x011965dc`,
`ACT_UPRIGHT_TURN_180 0x011967ac`, `ACT_NONE 0x011967d0`, `ACT_SUPER_STUNT_JUMP 0x01196e18`,
`ACT_GRAPPLE_SMASH 0x01196780`, `LA_ACT_GRAPPLE_SMASH 0x01196b48`, `ACT_LAND 0x011967c8`. State hashes used as
gates: `S_JUMP_LAND_INTERRUPT 0x01196490`, `S_JUMP_RUNNING_LAND_INTERRUPT 0x01196d8c`, `S_EVADE 0x01197148`,
`S_UP_FROM_CROUCH 0x0119672c`, `S_PARACHUTE 0x011965ac`, `S_UNFOLD_PARACHUTE_HORIZONTAL 0x01196620`,
`S_UNFOLD_PARACHUTE_VERTICAL 0x011966ac`, `S_SKYDIVE_PULLCHORD 0x01196c78`,
`S_PULL_OPEN_PARACHUTE_VERTICAL 0x01196e84`.

## 3. On-foot input handler `0x007b6520` (native)

`this` = COnFootController, argument 1 = the character, argument 2 = the input record set. Character field
`+0xa34` is the per-character state block "S" used below. Order of work each frame:

1. **Movement axes.** Scale `s` = 1.0, or **0.425** when WALK (34) is active, or when `0x006175a0()` (character
   `+0x728`, player-controlled) is true and the player object's `+0x300` is non-zero. forward = FORWARD * s / 4095;
   if BACKWARD is active it **replaces** it with -BACKWARD * s / 4095 (backward wins when both are held). left =
   LEFT * s / 4095, replaced by -RIGHT * s / 4095 when RIGHT is active (right wins). The pair (left, forward)
   goes to `0x0066af80`, which stores it in S `+0x7c/+0x80`, stores its length clamped to [0,1] in S `+0x6c`
   (input magnitude), and stores in S `+0x70` the length of the planar vector S `+0x8c/+0x90` that the character
   motion driver (character `+0x9c8`, vtable slot `+0x50`) has just written (inferred: its desired planar velocity;
   it is called "speed" below and in the probe, but it is not proven to be the measured speed).
2. **Timers** `0x008019a0` (called with S): `+0x250` seconds since JUMP was pressed (0 on the PRESSED frame, else
   `+= dt`); `+0x254` seconds since EVADE was pressed (same); `+0x24c -= dt` and, while DASH (118) is not active,
   it is reset to **0.01 s** (so it is <= 0 only while DASH is held); `+0x274 -= dt` (kick buffer). S `+0x94/+0x98`
   are cleared afterwards.
3. **Jump**, 4. **Kick**, 5. **Crouch**, 6. **Evade** as below.
7. `0x007ab170(character, S)`: the locomotion intent update (next subsection).

### Locomotion intent after the input (native, `0x007ab170` -> `0x00727840`)

`0x007ab170` runs two overrides first (`0x007f5810`, `0x00612700`, and a motion-driver query `+0x7c` that diverts to
`0x005a8880`), then `0x00727840`, then sets character mode `+0xa74 = 2` when `0x00572ca0` or `0x005ebc90` is true, and
finally copies S `+0x70 -> +0x78` and `+0x7c/+0x80 -> +0x84/+0x88` (previous-frame values). `0x00727840` computes
`c` = cosine between the character facing (`-character+0x17c/+0x184`, normalised) and the desired planar direction
(`0x007d5350` from S `+0x5c`, `+0x7c`), the side flag (right axis dot direction > 0), sets S `+0x338` when the current
state is `S_DASH`, `S_DASH_TURN_180` or `S_INSTANT_DASH` (`0x00615e60`), and then picks one of three actions
by the current layer-0 animation state:

* idle (`S_UPRIGHT_IDLE`, `S_ROCKET_BABY_IDLE`; `0x007b64b0` -> `0x0075db30`): when input magnitude S `+0x6c` and S `+0x70`
  are both above the controller deadzone `this+0x34` (0.1): if `c < 0.67` (`0x01076df8`) dispatch `ACT_ROTATE_CW` or
  `ACT_ROTATE_CCW` (by side) and store `c` in S `+0x47c`, otherwise dispatch `ACT_NAVIGATE` (`0x01196f9c`) on layers 0 and 1;
* rotating (`S_UPRIGHT_ROTATE_CW/CCW` and their `_INTERUPT` states; `0x0051bce0` -> `0x007d2590`): while above the deadzone
  face the desired direction (`0x008aa820`) and dispatch `ACT_NAVIGATE`;
* everything else (`0x0063b110`): first the sprint test `0x005ff2f0` (section below). If input and speed are both <= 0.1:
  stand (`ACT_UPRIGHT_STOP` while in a dash state). If `c <= -0.6` (`0x01077468`) and S `+0x70 <= 0.7` (`0x01077464`):
  `ACT_UPRIGHT_TURN_180` (blend stored in S `+0x48c`). Otherwise face the desired direction and dispatch
  `ACT_NAVIGATE`.

So the animation graph, not code constants, decides walk/run/turn clips; the code decides *which action* from the
input, the current state, the 0.1 / 0.67 / -0.6 / 0.7 / 0.9 thresholds and the sprint rule.

### Jump (native)

Fires `0x00816700` when JUMP is PRESSED, or when the current layer-0 animation state is
`S_JUMP_LAND_INTERRUPT` or `S_JUMP_RUNNING_LAND_INTERRUPT` (`0x00797a80`) and the press is younger than
**0.3 s** (`0x01077710`). The buffered attempt repeats every frame until it succeeds or expires.
`0x00816700` requires every attached layer object to be ready (`0x006d2060`) and `ACT_JUMP_AT` accepted on
layer 0. On success it: takes the movement vector from `0x008a0bd0` (normalised when its length >= 0.2) into S
`+0xbc/+0xc0/+0xc4`; zeroes S `+0xac, +0x104, +0xc8, +0x40, +0x3c, +0x118`; plays sound `ply.jump`; sets
character mode `+0xa74 = 8`; dispatches `ACT_JUMP_AT` a second time (layer argument 1); then calls `0x0067c740(0)` and
`0x0058e060(character, 0)`. **No velocity is written here**: the take-off speed and the flight come from the
animation states (`CJumpControllerState` chooses clip blends only; flight states are velocity-controlled,
`animation-statemachines-next.md`) and the character motion driver at character `+0x9c8` (Gap 1).

### Kick (native)

On KICK PRESSED, when `this+0x20 == 0` and the global byte `0x011958c0 == 0`: S `+0x270 = 1` (pending) and S
`+0x274 = 0.7 s` (`0x0107770c`). While pending, `0x00617ae0` (a COnFootController method taking the character) is
tried every frame; success clears `+0x270`. It requires `0x007bd080`, then (`0x0059be20` or `LA_ACT_GRAPPLE_SMASH`
(`0x01196b48`) accepted on layer 2) and `ACT_GRAPPLE_SMASH` (`0x01196780`) accepted on layer 0; it then picks the best
candidate in front of the character within squared distance 6.25 (2.5 m) from the nearby-character list
(`0x00728280`) and turns the character toward it.
Kick is therefore the melee/grapple-smash action on a short buffer. (`+0x274` is decremented but the expiry of the
pending flag is not tested in `0x007b6520`: inferred that `0x00617ae0` or the animation side clears it.)

### Crouch (native)

Setting 0x28 selects the mode (defaults are zero: **hold**). Crouch is blocked when `this+0x20 != 0`
or `this+0x1c == 1`, when `0x005ebc90(character)` is true, or when S `+0x258`/`+0x264` (already crouching) are set.
Hold mode (setting 0x28 == 0): while CROUCH is active the crouch routine `0x005d4e60` runs every frame (dispatches
`ACT_CROUCH`, sets S `+0x259/+0x265`, clears `+0x258/+0x264`). Toggle mode: CROUCH PRESSED flips S `+0x33b`; the
routine runs while it is set (additionally `0x004f93d0`, `0x00816300`, `0x0075c080`, `0x007369c0(0)` must be false).
When `S_UP_FROM_CROUCH` is current (`0x005b21d0`) and a crouch flag is set the handler calls `0x005301a0(character,
0.2)`.

### Evade and sprint (native)

`0x00a3d500` (evade executor) requires `0x0051f670` and `ACT_EVADE` accepted on layer 0, then faces along the S
`+0x5c/+0x7c` direction and ends like the jump executor. The handler calls it when any of:

* EVADE is **RELEASED** (state 4) and the press lasted no more than the controller's `+0x30c` (constructor default
  **0.25 s**, from `0x00e2c7d0`; may be overridden by data, Gap 5);
* the current state is `S_EVADE` (`0x00507a10`) and the press is younger than **0.2 s** (`0x01077708`): a second
  evade queued during the first;
* setting id 0x10 equals 0 and EVADE is PRESSED. Setting 0x10 is `CSettingsManager+0xcc`, the input device
  (`controls-next.md` section 3: default -1, 0 = keyboard/mouse). **Keyboard/mouse evades on press; a pad evades on
  tap-release**, because RSHOULDER is both EVADE and DASH (hold = sprint).

Sprint is decided in `0x005ff2f0` (shared with NPC locomotion): sprint (S `+0x249`) starts only when DASH is held
(`S+0x24c <= 0`), the input magnitude S `+0x6c` or speed S `+0x70` exceeds **0.9** (`0x01076dfc`), and the time
since the last EVADE press exceeds the same `+0x30c` threshold. Otherwise S `+0x249` is cleared and `+0x24c`
reset to 0.01. WALK scales the axes to 0.425, so walking can never reach 0.9 and never sprints. Controller
`+0x34` (**0.1**, `0x00e290bc`) is the stop deadzone used by `0x0063b110`: input and speed both <= 0.1 means
standing (`ACT_UPRIGHT_STOP` / `ACT_UPRIGHT_TURN_180` are dispatched there for NPCs; turn-180 uses the
direction dot <= -0.6, `0x01077468`, with speed <= 0.7, `0x01077464`).

## 4. Parachute open / close (native)

`0x0074edd0`, called each frame by the skydive controller `0x00619510` (and gated by character `+0x47c`), with
`P` = the player's canopy object (`player+0xcc`; canopy vtable slot 9 = "is open", 6 = open, 7 = close):

* canopy **not open**, PARACHUTE_OPEN_CLOSE PRESSED, `ACT_PULLCHORD` accepted on the character: open the canopy (slot
  `+0x18`) and set the flight data block (`S+0x170`) field `+0x28` to **-0.825** (`0x01077980`);
* canopy **open**: PRESSED closes it (slot `+0x1c`); otherwise `ACT_PARAGLIDE` is dispatched every frame.

Inside the flight model `0x005a9950` PARACHUTE_OPEN_CLOSE PRESSED also closes (cut) the canopy
(`0x0073a670` then the controller vtable slot `+0x74`), and **FIRE_GRAPPLE held** selects the branch that skips
the vehicle-landing assist and uses the free-flight turn logic. The thruster item (ACTIVATE_PARACHUTE_THRUSTERS,
`0x005bcbf0`) charges/drains a resource on the thruster object at canopy `+0x6c` (state `+0x18` 0/1, thrust value
`+0x1c`, charge `+0x28`, start threshold `+0x2c`, maximum `+0x30`, drain rate `+0x34`, recharge rate `+0x38`,
cooldown `+0x24/+0x20`); while on it adds `thrust value` to the thrust of section 8 and replaces the steering scale
vector (default 1,1,1) by the thruster object's vector at `+0x3c` (values are data: Gap 6).

## 5. Per-state facts for the audit

| Event | Native condition | Source |
|---|---|---|
| Jump | PRESSED, or land-interrupt state and press < 0.3 s | `0x007b6520`, `0x00797a80` |
| Pull the chord | PARACHUTE_OPEN_CLOSE PRESSED in the skydive controller and `ACT_PULLCHORD` accepted | `0x0074edd0` |
| Close/cut chute | PARACHUTE_OPEN_CLOSE PRESSED while open | `0x0074edd0`, `0x005a9950` |
| Evade | tap-release <= 0.25 s, or press on keyboard/mouse, or buffered 0.2 s | `0x007b6520` |
| Sprint | DASH held, magnitude/speed > 0.9, no evade press in the last 0.25 s | `0x005ff2f0` |
| Kick | PRESSED, then up to 0.7 s of retries; melee target within 2.5 m | `0x007b6520`, `0x00617ae0` |
| Walk | WALK held: axes x0.425 | `0x007b6520` |
| Zoom | scope stage 3 only | `0x005c2600` |

## 6. Shared keys: who reads what (native, asserted)

| Key (keyboard / pad) | Actions on it | Readers |
|---|---|---|
| SPACE / A | JUMP, PARACHUTE_OPEN_CLOSE, REELED_IN_JUMP, DEPLOY_PARACHUTE_WHILE_REELING | JUMP: `0x007b6520`, `0x008019a0`, `0x005ae110`, `0x0063aec0`; chord: `0x0074edd0`, `0x005a9950`; reel actions are context actions |
| LSHIFT / RSHOULDER | DASH, ZOOM_IN, heli/plane/bike axes | DASH: `0x008019a0` only; ZOOM: `0x005c2600` only; vehicle axes in their own controllers |
| TAB / RSHOULDER | EVADE | `0x008019a0` (timer), `0x007b6520`, `0x0063aec0`, `0x005c7a30`, `0x008243b0` |
| Q / B | KICK | `0x007b6520` only |
| LALT / B | STUNT_JUMP | no literal reader (context action data) |
| LCONTROL / L3 | CROUCH, REELED_IN_RELEASE_ACTION, ZOOM_OUT | CROUCH: `0x007b6520` and 4 others; release is a context action; ZOOM_OUT: scope |
| F / LSHOULDER | FIRE_GRAPPLE | `0x005a9950`, grapple code (`0x005c2df0`, `0x006b8890`, `0x00844b30`) |
| E / Y | USE_ITEM, EXIT_VEHICLE, pickups | not on-foot movement |

The overlaps never conflict inside one reader, because the readers of one key are different controllers/states
(on foot, skydive, parachute, vehicle, scope) and only the active controller's update runs.

## 7. Stunt jump (native structure, data unresolved)

`CStuntJumpAction` (vftable `0x00ec7ae4`, a `CSmartObjectAction`; ctor `0x006b6b70`) is a context action
(`CContextAction` slots: 4 availability, 5 triggered, 6 start). Availability `0x0070b7e0`: the stunt-position
smart object is enabled (`+0x7d`), the character's layer-0 state is one of `S_UPRIGHT_IDLE`,
`S_UPRIGHT_BASIC_NAVIGATION`, `S_UPRIGHT_STRAFE`, `S_DASH`, `S_DEFAULT_STUNTPOSITION_IDLE`, the two
`S_DEFAULT_STUNTPOSITION_ROTATE_180_*`, `S_CLING_STUNTPOSITION_IDLE`, `S_CLING_LEAN_LEFT/RIGHT`, and the facing
test against the target passes (sets `+0x121` when the dot is at least the field at `+0xa4`). Triggered
`0x007857d0`: true when `+0x30` is set; else, when `+0x121` is set, when the input id stored at `+0x24` is
active with analog value **> 0.6** (`0x010800f8`); else, when `+0x120` is set, the same with the id at `+0x11c`;
else when mode `+0x114 == 1` and S `+0x14` (automatic jump-off). Start `0x00720100` requires `ACT_SUPER_STUNT_JUMP`
(`0x01196e18`) accepted and stores the target data; the flight is `S_SUPER_STUNT_JUMP_FLIGHT`
(`animation-statemachines-next.md`). The two input ids are data on the smart object, not constants in code:
keymap actions STUNTPOS_JUMP_OFF (73), STUNTPOS_TO_PARACHUTE (74), STUNTPOS_ENTER_VEHICLE (75),
EXIT_TO_STUNTPOS_PARACHUTE (46) and STUNT_JUMP (119) have no literal reader either (Gap 2). The HUD prompt for a
stunt position (`GetStuntButtonInfo`, `0x00a29540` -> `0x0059fff0`) shows the glyph token `{GUI_CANCEL}`, which
suggests the prompt is the cancel key; unverified.

## 8. Parachute flight model `0x005a9950` (native formulas, inferred axis naming)

Per frame (dt from `0x004d3690`), `S` = character state, `D` = flight data block at `S+0x170`, canopy transform
`M` = rows `R0, R1, R2` (canopy object transform at `+8`), velocity `v` from `0x008a0bd0`. The model is skipped
unless the canopy is open, character `+0x47c` is set and the canopy vtable slot 2 query (`0x00686fd0`) passes.

**Axis naming (inferred from the formulas below)**: `R0` = canopy side axis, `R1` = wing normal, `R2` = canopy back
axis; canopy forward `A = normalize(-R2)` (falls back to `R1` when `-R2` has no horizontal part).

**Opening frame** (flag at controller `+0x49`, consumed by `0x005c36a0`): the Euler angles `D+0x30/+0x34/+0x38` are
initialised from the character matrix (`0x004de960`), `D+0x3c..+0x44` (angular rates) are zeroed, and when the
vertical speed `vy < 0` the velocity becomes: `vc = max(vy, -30)` (`0x010776fc`); `h` = normalised horizontal
heading of `v`; `vx' = vx + 0.4*|vc|*hx`, `vz' = vz + 0.4*|vc|*hz`, `vy' = 0.6*vc + (vy - vc)`
(`0x010776f8` = 0.4).

**Forces** (`g` = 9.82 at `0x00e48bc0`; mass scale 1/90 = `0x010776cc`):

* `va = v + 3.5^2 * A` (3.5 at `0x010776f4`, squared 12.25) and `s2 = |va|^2` (also stored in `D+0x20`);
* wind `W = normalize(-v)`; lift angle `thl = asin(W.R1)` degrees; drag angle `thd = acos(W.R2)` degrees
  (57.295776 at `0x00e2ec20`);
* `p = W.R0`; `Wp = W - p*R0`; drag `Fd = Wp * s2 * 3.0 * Cd(thd) + R0 * p * s2 * 10.0` (3.0 at `0x010776f0`,
  10.0 at `0x010776ec`);
* lift direction `Ld = normalize((W x R1) x W)`; lift `Fl = Ld * s2 * 1.7 * Cl(thl)` (1.7 at `0x010776e8`);
* thrust `Ft = A * m` with `m = 1.0` (`0x010776e4`) unless `W.R1 >= 0.7` (`0x00e30748`), in which case
  `m = (W.R1)^16 * 400` (exponent double `0x00e42e50`, 400 at `0x010776e0`): an auto-recovery push when the canopy
  is stalled. With the thruster item active `m += thrust value`;
* `a = (Fd + Fl + Ft) / 90`, `ay -= 9.82`, `v += a * dt`. With a hooked grapple wire (device `+0x128`; any FIRE_GRAPPLE
  state; **corrected by `parachute-canopy-landing-next.md`**) the speed is clamped to `device+0xfc` = 40.0; finally the component of `v` against `A` is
  removed (`v -= A * min(0, v.A)`), so the canopy never flies backwards.
* **Cd(x)**, x = |angle| in degrees (`0x00843900`; table x at `0x010773ac`, y at `0x01077388`): points
  (0, 0.032), (4, 0.052), (8, 0.0904), (12, 0.124), (16, 0.194), (20, 0.25), (24, 0.328), (90, 0.8), (180, 1.0);
  linear between points, 0 beyond 180.
* **Cl(x)** (`0x0056fca0`; x at `0x01076e00`, y at `0x01076e2c`): (-8, -0.54), (-4, -0.2), (0, 0.2), (4, 0.57),
  (8, 0.92), (12, 1.21), (16, 1.43), (20, 1.4), (54, 1.0), (80, 0.0), (180, 0.0); linear between points, -0.54
  outside.

**Steering** (free flight; the landing-target branch is not decoded, Gap 4). Inputs: `pitchIn = FORWARD - BACKWARD`
(swapped when setting 0x26 is non-zero), `rollIn = RIGHT - LEFT` (analog values, no extra smoothing);
`turn = D+0x2c` approaches (`0x004f0a30`, linear at 0.7 units/s, `0x010776d0`) the value `0.46 * S+4` when
`|S+4| > 0.9` (`0x010776d4`, `0x010776d8`; requires player setting byte `+0x114e`), otherwise 0. `S+4` is the signed
yaw angle between the character and the camera, written once per frame by `0x0077fc80` (**corrected by
`parachute-canopy-landing-next.md` section 10.5**; the earlier inference "facing against movement direction" was wrong). The three Euler angles `a30 (pitch), a34 (yaw), a38 (roll)` with rates `w30, w34, w38` follow a
damped spring, with `sx, sy, sz` = 1 (or the thruster vector):

```
e30 = (0 - pitchIn*sx) * 0.65 - a30                # statics 0x011fcef8/efc/f00 = 0.65, 0.75, 0.65
e34 = (turn - rollIn*sy) * 0.75 + heading - a34    # heading = atan2(R2.x, R2.z)
e38 = (turn - rollIn*sz) * 0.65 - a38              # errors wrapped to [-pi, pi]
damp30 = 0.2 + (1.8-0.2)*|a30|*(2/pi);  damp38 = 0.2 + (1.8-0.2)*|a38|*(2/pi)   # 0.2/0.2, 1.8/1.8 statics
w30 *= exp(-damp30*dt);  w34 *= exp(-0.95*dt);  w38 *= exp(-damp38*dt)          # 0.95 at 0x010776bc
w30 += 1.2*e30*dt;  w34 += 1.0*e34*dt;  w38 += 1.2*e38*dt                       # statics 1.2, 1.0, 1.2
a30 += w30*dt;  a34 += w34*dt;  a38 += w38*dt
```

The matrix is rebuilt from the three angles (`0x004dbce0`, `0x004eaad0`, `0x004f1790`, composed `Z*X` then `*Y`),
converted to a quaternion and written to the canopy; the velocity is written back with `0x00867fa0`. The
fixtures in the probe pin the force law (stalled canopy, sideslip), the opening transform and the spring step.

## 9. Skydive (freefall) `0x00619510` (native constants, inferred vectors)

* Pitch pose `P` in [0,1] (`S+0x148`, `0x00517090`): a **static** target `t` approaches
  `((BACKWARD - FORWARD) + 1) * 0.5` at rate 1.5/s (initial 0.5, `0x01077c3c`), then `P += (t - P) * dt * 4.0`, clamped.
* Roll pose `R` in [-1,1] (`S+0x14c`, `0x007687c0`): a static target approaches `2 * (LEFT - RIGHT)` at 4.0/s, then
  `R += (target - R) * dt * 3.0`.
* Model `0x006b9290`: frontal area `(0.8 + 1.1 P) * (0.4 + 0.8 P)` (`0x01077c60..68`), pressure
  `q = area * |v|^2 * 0.645`; drag-like coefficient piecewise in P (written with P itself, not a segment
  parameter): P <= 0.25 -> `0.5 + 0.3 P`; 0.25 < P <= 0.5 -> `0.8 - 0.5 P`; P > 0.5 -> `0.3 + 0.1 P`; lift-like
  coefficient: P <= 0.25 -> `0.6 + 0.3 P`; <= 0.5 -> `0.9 + 0.5 P`; else `1.4 - 0.4 P` (`0x01077984..9a0`); forces
  divided by 90 (`0x01077c5c`), gravity -9.82, roll angle clamp **2.1991148 rad** (`0x00e4bda0`), per-frame roll-pose nudge
  clamp +-0.05 (`0x01077c54`), vertical speed cap field `S+0x150` with 0.4 (`0x01077c58`). Which force acts along `-v` and
  which along the lift axis: **resolved and reversed** in `parachute-canopy-landing-next.md` section 7 (the 0.6..1.4 curve is the drag along `-v`).

## 10. Runtime guidance (JC2-238)

* Replace invented foot constants (WALK 2.6, RUN 4.5, SPRINT 7.0, JUMP_SPEED 5.5, CHUTE_GLIDE* in
  `character.rs`) by the native structure: **no numeric run/jump speed exists in the input handler**; speeds come
  from animation states (`rico-locomotion-motion.md` has the formulas and numbers). Do implement now: the input scale 0.425, the 0.3/0.2/0.7 s buffers, the evade
  tap/press rule per device, sprint conditions (0.9, 0.25 s), jump gating by animation action acceptance, the
  parachute opening transform, the glider force law and the spring steering above.
* Keyboard: SPACE = jump and chord, LSHIFT = dash, TAB = evade (press), Q = kick, LALT = stunt jump context,
  LCONTROL = crouch (hold by default).

## Gaps (unresolved native details)

**Update (JC2-243/244):** Gaps 4, 6 and 7 are resolved in `parachute-canopy-landing-next.md` (hook assist = grapple device, landing and cut-chute rules
from the animation graph, canopy basis, neutral glide, skydive force law/cap/orientation/entry); what remains of them is listed under that brief's Gaps.
The text below is kept as the original list.

1. **Locomotion and jump motion source.** Resolved by JC2-241 (`rico-locomotion-motion.md`, probe
   `locomotion_motion_probe.py`): the motion driver only low-passes the stick (alpha 0.1 per call); walk/run/sprint speeds are clip
   root motion x time scale x blend weight in the navigation/dash/strafe state classes (run 5.51 m/s, walk key 2.53, sprint 5.21
   rising to 8.33 m/s); turning is a per-state rate limited slerp (default 540 deg/s, sprint 180 deg/s); the jump climb is the
   start clip's root-motion Y. The five `COnFootController` parameter blocks are not speeds. Still open: the airborne arc and
   horizontal carry-over (Gap 1 of that document).
2. **Stunt jump input ids and trigger geometry**: the ids at action `+0x24/+0x11c` and the angle threshold at `+0xa4`
   come from smart-object data (no property file matched); STUNTPOS_* ids have no literal readers.
3. **Context-action precedence** (priority field `+0x28`): still unresolved (controls Gap 1).
4. **Parachute landing assist and landing/close logic** (`0x005aa5db-0x005aa7f3`, vehicle target `+0x128/+0xfc`).
5. Data overrides of `COnFootController` `+0x30c` (0.25), `+0x34` (0.1), crouch deadzones; names/defaults after user
   settings load for settings 0x10, 0x26, 0x28; meaning of `this+0x20` and `0x011958c0` in the kick gate.
6. **Parachute canopy basis and tuning**: row meaning of `R0/R1/R2` is inferred; the canopy bind pose, hence the
   neutral glide (sink and forward speed), and the thruster object values (`+0x20..+0x3c`) need the canopy asset/data.
7. **Skydive model**: vector roles, the `S+0x150` cap, entry transitions from fall (`S_SKYDIVE*`), start speed.
8. Kick executor candidate scoring (`0x00617ae0` list `0x00728280`, threshold from `0x01076ad0-ad8`) beyond the 2.5 m range.
