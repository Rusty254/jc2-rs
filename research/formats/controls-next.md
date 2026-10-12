# Original PC and gamepad input contract (JC2-083)

Research only. Probe: `research/tools/controls_consumer_probe.py` (52 checks, exit 0):

```
python research/tools/controls_consumer_probe.py --game <game folder> --jc2tool <abs path>/jc2tool.exe [--matrix]
```

Executable fingerprint: `JustCause2.exe` sha256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
Addresses are virtual addresses in that executable. Labels: **native** = read from code/data by
the probe or a cited function; **inferred** = reasoned from native facts; **unknown** = listed in Gaps.
The official manuals could not be read (the PC PDF returned binary only through the fetch tool, the
console PDF was not retrievable), so no manual cross-check is claimed.

## 1. Source files (retail, unmodded)

| File | Archive | Raw sha256 (bytes as `jc2tool cat` returns them) | Rows |
|---|---|---|---|
| `settings/keymap_keyboard.bin` | `archives_win32/pc0`, hash d88b3cda | `3d002d450182c345ee5e2be4bdcb0abe943449e1b8dbf17fb23212e9cd35a0a4` | 98 |
| `settings/keymap_gamepad.bin` | `archives_win32/pc0`, hash a7f2230a | `31fa657efd347e8492071a5d0933510bbf32bb64b83860937b35fe52ac56f0a2` | 94 |

Both are property containers: root `0x49267abc`, one child per row, each with a key-name string
(property `0x7935f00c`) and an action string (property `0x8613ccd3`). The pad file is a PC
file (it contains `GUI_PDA=BACK`, `GUI_PAUSE=START`), so console presets are not needed and must not be
assumed equal to it. Record ids are the engine hash of the action name except eight alias rows
(second `TURN_LEFT`/`TURN_RIGHT`, `ACCELERATE`/`REVERSE`, `GUI_ICONLEGEND`, `GUI_DATABASE`,
`DEBUG_INCREE/DECREE` share an action with another row under a different id): keep every row.

## 2. Native loader (native)

* `0x005092b0` (CSettingsManager reload, source file `SettingsManager.cpp` line 0x367): clears the
  binding vector at `this+0x168`, then loads `Settings/keymap_gamepad.bin` **first** and
  `Settings/keymap_keyboard.bin` second into the same vector (`0x0077bb10`). The Lua functions
  `ResetKeyMapping` (`0x00a2d8b0`), `ConfirmKeyMapping`, `CancelKeyMapping`, `ConfirmSaveKeymap` reach it.
* `0x0077bb10`: for every row it needs both strings to resolve: the action name in the 157-entry
  name table and the key name in the token map. It pushes an 8-byte `{action id, key id}` and sorts the vector.
  Rows that do not resolve are dropped silently. The only such retail row is the pad file's
  `CAMERA_RESET=RSTICKBUTTON` (the table spells it `RESET_CAMERA`), so that binding does not exist at runtime.
* Action ids = index in the pointer table at `0x01071de8` (157 entries, `PAUSE`=0 ... `GUI_ANALOG_RIGHT`=156).
  Examples: `LOOK_UP..RIGHT`=3..6, `SHOULDER_CAM`=9, `FIRE_LEFT/RIGHT`=12/13, `JUMP`=35, `SOUND_HORN_SIREN`=44,
  `PLANE_PITCH_UP/DOWN`=58/59, `PLANE_TURN_L/R`=60/61, `PLANE_ROLL_L/R`=62/63, `ZOOM_IN/OUT`=83/84,
  `REELED_IN_JUMP`=144, `REELED_IN_RELEASE`=145, `DEPLOY_PARACHUTE_WHILE_REELING`=146.
  Not bound in either file (so unbound on PC): `DETACH_GRAPPLE`, `INC/DEC_GRAPPLE_DISTANCE`, `WALK`, and 51 others.
* Key tokens (`0x00595dc0`, 138 names) map to ids; ids map to a device spec (`0x007a59f0`):
  device 1 keyboard (code = DirectInput scancode), 2 mouse, 4 gamepad, with a direction `sub`
  (2 negative, 3 positive). `BACK` is registered twice (keyboard 0x40 then pad 0x87) and the map keeps
  0x87, so the name `BACK` always means the pad Back button.
  Pad ids: `RPADDOWN/RIGHT/LEFT/UP`=0x70..0x73 (XInput A/B/X/Y bits 0x1000/0x2000/0x4000/0x8000, native
  `0x00983bd0`), `LTRIGGER/RTRIGGER`=0x74/0x75, `RSHOULDER/LSHOULDER`=0x76/0x77, `LPAD*`=0x78..0x7b (D-pad),
  `LSTICK*`=0x7c..0x7f, `RSTICK*`=0x80..0x83, stick clicks 0x84/0x85, `START`=0x86, `BACK`=0x87.

## 3. Device value readers (native)

| Device | Function | Behaviour |
|---|---|---|
| Keyboard | `0x00985850` | key byte high bit set = 1.0 |
| Mouse | `0x00982cc0` | DirectInput deltas: counts 1..64 map to count/64, above 64 = 1.0, negative direction negates, active if > 0.01. No sensitivity here. Buttons = 1.0. |
| Pad buttons | `0x00983bd0` | digital 1.0; triggers byte/255 (no deadzone here) |
| Pad sticks | `0x00984790` | /32767, radial deadzone 0.2 then `(m-0.2)*1.25` (not clamped, per-key clamp to 1.0), direction kept via a sin/cos polynomial (0.225 form) |
| Active pad | `0x00982f20` | global `0x010820fc` (init -1) selects the pad; setting `CSettingsManager+0xcc` (default -1; 0 = keyboard/mouse) is applied in `0x00612050` via `0x009836f0(4)` |

Each binding feeds a record (`0x009854a0`): 12-bit value `int(v*4095)`, previous value in bits 12..23, state in bits 24..26.
State machine (native; fixtures in the probe): 0 start -> 1 IDLE; IDLE with value -> 2 PRESSED (exactly one frame);
PRESSED with value -> 3 HELD, **PRESSED with value 0 -> IDLE (a one-frame tap yields no RELEASED edge)**;
HELD with value 0 -> 4 RELEASED (one frame) -> state 0 -> IDLE (idle time accumulates from the following frame);
RELEASED with value -> PRESSED again. Idle time since release is `0x00982f50` (FLT_MAX unless IDLE).
Several bindings per action: `0x009828a0` returns the record with the highest 12-bit value; a RELEASED record
beats a non-RELEASED best. No summing. This is the whole press/hold/release arbitration; there is no global
context stack: each consumer polls actions it cares about (see section 5).
Consumers test "active" as state 2 or 3 and use `value/4095`. `0x00986860` writes an action directly
(value + HELD) and is used for AI/script injection (`0x0075b4b0` sets the horn; `0x00692a00` sets LOOK_*).

## 4. What is fixed and what is remappable

* `0x0080e770` builds three tables in CSettingsManager (+0x1b0): 7 **link groups** (a rebind of one member
  also binds the key to every other member, `ConfirmKeyMapping` `0x007c3a30`), context tags
  (1 foot 30 actions, 2 land vehicle 14, 3 heli 13, 4 plane 13) and a **remappable list of 61 actions**.
  Link groups: {USE_ITEM, EXIT_VEHICLE, GRAPPLING_ACTION, ACTIVATE_BLACK_MARKET_BEACON};
  {FIRE_LEFT, VEHICLE_FIRE_LEFT, THROW_GRENADE, MC_FIRE}; {FIRE_RIGHT, VEHICLE_FIRE_RIGHT};
  {JUMP, REELED_IN_JUMP, PARACHUTE_OPEN_CLOSE, DEPLOY_PARACHUTE_WHILE_REELING}; {STUNT_JUMP, REELED_IN_RELEASE};
  {PLANE_TURN_LEFT, PLANE_ROLL_LEFT}; {PLANE_TURN_RIGHT, PLANE_ROLL_RIGHT}.
  The shipped files do not always honour the groups (keyboard STUNT_JUMP=LALT but REELED_IN_RELEASE=LCONTROL;
  PLANE_TURN=A/D but PLANE_ROLL=Z/X): groups act on edits, not on defaults.
* **Not remappable** (absent from the list): `SOUND_HORN_SIREN`, `PLANE_ROLL_*`, `HELI_TURN_*`, `LOOK_*`,
  GUI navigation, `GRAPPLING_ACTION` (follows USE_ITEM), pickups. Rebinding replaces only keyboard/mouse bindings
  (`0x00552e80` removes device 1/2 entries of the action).
* **Look is hard-wired after the file is applied** (`0x005f6330`, `0x00612050`): `LOOK_UP/DOWN/LEFT/RIGHT` bindings are cleared
  and rebound from stored settings flags (CSettingsManager `+0x8c+4*id`): pad Y (`id 10`, `+0xb4`, `0x0058fb20`
  /`0x005c0250`), pad X (`id 11`, `+0xb8`, `0x00727a90`), mouse X (`id 18`), mouse Y (`id 19`). With a flag of 0 the
  routing is `LOOK_UP <- RSTICKDOWN / MOUSEDOWN` and `LOOK_LEFT <- stick/mouse left`; flag != 0 swaps the up/down pair (left/right likewise).
  Plane pitch (`id 12`, `+0xbc`, `0x005dff90`): flag 0 gives `PLANE_PITCH_UP <- LSTICKDOWN` (file default, flight style).
  Trigger/bumper swap (`id 13`, `+0xc0`, `0x00756c00`): flag 1 exchanges the whole action sets of LTRIGGER<->LSHOULDER
  and RTRIGGER<->RSHOULDER when FIRE_LEFT is on the other side. `CControllerSettingsMetric` (`0x005de2f0`) names the fields
  SX/SY (`+0xc8`/`+0xc4`), AXI, AYI, FPI (`+0xbc`), PPI (`+0x124`), FB (`+0xc0`), Duck (`+0x12c`), Vib (`+0xb0`); it is a
  telemetry report (`0x007a4730`), not a persistence format.
  Constructor defaults zero all of these (`0x006c4230`); the user-settings load path was not traced.

## 5. Native consumer paths (the contexts and gestures)

Found by scanning every `push action-id; call 0x00984140` (get best record); the probe asserts these sets.

| Context | Consumer | Proven behaviour |
|---|---|---|
| Car / bike / boat driver | `0x00548770` (+ `0x005503d0`, `0x00663020`, `0x006e5980`) | throttle = ACCELERATE, brake = REVERSE, steer = TURN_RIGHT - TURN_LEFT (both held cancel), bike tilt F - B, HANDBRAKE digital, VEHICLE_CAM toggles `+0x2c` on the first active frame (latched `+0x66`). **Horn:** level-triggered; held calls the vehicle horn start slot (`+0xf4`) and release calls stop (`+0xf8`) when the vehicle's `+0xe8` is false; for siren vehicles (`+0xe8` true) the first active frame calls the toggle slots (`+0xfc`/`+0x100`) once, latched until release. |
| Plane | `0x00797130` | per axis: negative action first, else positive, `value/4095`: TURN_LEFT/RIGHT -> air-vehicle field `+0x210`; ROLL_LEFT/RIGHT -> `+0x208`; PITCH_DOWN/UP -> `+0x214`; INC/DEC_TRUST -> `+0x200` (negative = DEC); VEHICLE_CAM toggles a flag on first active frame. Getter `0x00686570` returns (thrust, `+0x210`, `+0x214`, `+0x208`) scaled x5.0 when setting id 16 is 0 and `0x0069a0d0` is false. |
| Helicopter | `0x007b0f20` | HELI_TURN_L/R -> `+0x208`, HELI_ROLL_L/R -> `+0x210`, HELI_FORWARD(neg)/BACKWARD -> `+0x214`, INC/DEC_ALTITUDE -> thrust. Same sinks as the plane. |
| Scope | `0x005c2600` | SHOULDER_CAM on PRESSED cycles the scope stage (`0x0058e060`: stage 2 = "gui.sniper.stage1", 3 = "stage2"); GUI_CANCEL on PRESSED leaves stage 2; in stage 3 ZOOM_OUT (held) widens at a rate x dt up to a max, else ZOOM_IN (held) narrows to a min (OUT wins when both held); aim sensitivity is lerped from the normalised zoom. |
| Rope/stunt context actions | `CReeledInReleaseAction` ctor `0x0080a710`, `CReeledInJumpAction` `0x008336b0`, `CDeployParachuteWhileReelingAction` `0x0051ff30` | The action id is data (`+0x24`); constructor defaults: release = 145 (priority 100), jump = 144 (priority 100), deploy = **71 PARACHUTE_OPEN_CLOSE** (not 146; 146 is read by `0x006b8890`). Base default 0x29a = none. Slot 5 `0x008b7b80` = "PRESSED this frame"; release and jump use `0x007b60e0` = PRESSED **or IDLE for less than 0.2 s since the last release**. Slot 4 availability for release: character state is one of three globals (`0x00610490`). |
| Foot / grapple | `0x005a9950`, `0x006b8890`, `0x007b6520` | read MOVE_*, FIRE_GRAPPLE, PARACHUTE_OPEN_CLOSE, KICK, EVADE, CROUCH, WALK; not decomposed (Gaps). |

Inferred from the shared sinks: plane `TURN` (A/D) is the channel heli calls ROLL (bank) and plane `ROLL` (Z/X) is the channel heli
calls TURN (yaw). So the old "A/D bank" wording is right for TURN but Z/X is yaw, not a second bank. Confirm in the flight physics (Gaps).

## 6. Binding matrix (retail defaults; "-" = not bound; full 157-row matrix with consumers: `--matrix`)

| Action | Keyboard / mouse | PC gamepad |
|---|---|---|
| MOVE_F/B/L/R | W/S/A/D | LSTICK up/down/left/right |
| LOOK_* (overridden at runtime, section 4) | mouse axes | RSTICK |
| JUMP, PARACHUTE_OPEN_CLOSE, REELED_IN_JUMP, DEPLOY_PARACHUTE_WHILE_REELING | SPACE | RPADDOWN (A) |
| FIRE_GRAPPLE | F | LSHOULDER |
| GRAPPLING_ACTION, USE_ITEM, EXIT_VEHICLE, pickups | E | RPADUP (pickups: RIGHT=RPADUP, LEFT=RPADLEFT; keyboard both E) |
| REELED_IN_RELEASE_ACTION | LCONTROL | RPADRIGHT |
| STUNT_JUMP / KICK | LALT / Q | RPADRIGHT / RPADRIGHT |
| CROUCH | LCONTROL | LSTICKBUTTON |
| DASH / EVADE | LSHIFT / TAB | RSHOULDER |
| ZOOM_IN / ZOOM_OUT | LSHIFT / LCONTROL | RSHOULDER / LSHOULDER |
| SHOULDER_CAM / VEHICLE_CAM | C | RSTICKBUTTON |
| FIRE_LEFT / FIRE_RIGHT | MOUSE2 / MOUSE1 | LTRIGGER / RTRIGGER |
| VEHICLE_FIRE_LEFT / RIGHT | MOUSE1 / MOUSE2 | LSHOULDER / RSHOULDER |
| THROW_GRENADE | MOUSE2 | LTRIGGER |
| RELOAD | R | RPADLEFT |
| NEXT/PREV_WEAPON | wheel down/up | - |
| EQUIP_RIGHT/LEFT/DUAL/TWOHANDED | 1 / G / 2 / 3 | LPADRIGHT/LPADLEFT/LPADDOWN/LPADUP |
| SOUND_HORN_SIREN | Q | LSTICKBUTTON |
| ACCELERATE / REVERSE | W,UP / S,DOWN | RTRIGGER / LTRIGGER |
| TURN_LEFT / RIGHT | A,LEFT / D,RIGHT | LSTICK left/right |
| HANDBRAKE | X | RPADLEFT |
| BIKE_TILT_FWD/BACK | LSHIFT / LCONTROL | LSTICK up/down |
| HELI_FORWARD/BACKWARD | W/S | LSTICKUP/DOWN |
| HELI_TURN_L/R (yaw) | MOUSELEFT/MOUSERIGHT | RSTICK left/right |
| HELI_ROLL_L/R | A/D | LSTICK left/right |
| HELI_INC/DEC_ALTITUDE | LSHIFT / LCONTROL | RTRIGGER / LTRIGGER |
| PLANE_PITCH_UP/DOWN | S/W | LSTICKDOWN/UP |
| PLANE_TURN_L/R | A/D | LSTICK left/right |
| PLANE_ROLL_L/R | Z/X | not in the pad file |
| PLANE_INC/DEC_TRUST (spelling is native) | LSHIFT / LCONTROL | RTRIGGER / LTRIGGER |
| GUI_PDA / GUI_PAUSE / GUI_OK / GUI_CANCEL | F1 / ESCAPE / RETURN / ESCAPE | BACK / START / RPADDOWN / RPADRIGHT |
| GUI_PDA_ZOOM_IN/OUT | wheel up/down | RSTICKDOWN/UP |

Same-key overlaps in these defaults (LSHIFT, LCONTROL, E, SPACE, Q, MOUSE1/2) are resolved only by each consumer gating on its own state.

## 7. Why the current runtime mismatches (handoff)

* Horn: retail keyboard is **Q** (the runtime's H is not original); pad is LSTICKBUTTON; the horn is level-triggered, siren vehicles toggle on press.
* Rope cut: retail keyboard `REELED_IN_RELEASE_ACTION` is **LCONTROL**, `STUNT_JUMP` is LALT, pad both RPADRIGHT. Release/jump have a 0.2 s post-release grace.
* Zoom: LSHIFT / LCONTROL (pad RSHOULDER / LSHOULDER), OUT beats IN, only in scope stage 3.
* Aircraft: heli yaw is the mouse X (HELI_TURN), plane has no mouse axis; plane TURN (A/D) vs ROLL (Z/X) are separate channels, not duplicates.
* Handoff: JC2-073 should implement the `{action, key}` vector, the section 3 record state machine, best-record arbitration and the section 4 post-load look/flight rewrites; JC2-076 owns the keyboard table above; JC2-077 owns the pad table, deadzone and trigger swap.

## Gaps (each unresolved native detail)

1. Context-action manager: how simultaneous triggered actions are ordered (priority field `+0x28`, tags 1..4 consumer). Blocks exact action precedence.
2. Foot controller `0x005a9950`/`0x007b6520` input decomposition (JUMP vs PARACHUTE vs STUNT_JUMP vs DASH/ZOOM_IN on the shared LSHIFT, KICK/EVADE gestures).
3. Flight sinks `+0x208/+0x210`: which is physical yaw vs roll in the aircraft physics; meaning of the x5.0 scale (setting id 16 / `0x0069a0d0`).
4. Meaning of the stored invert flags (ids 10, 11, 18, 19, 12) versus the options UI, their defaults after the user-settings load, and the resulting default look polarity.
5. Where the options screen writes/reads these settings and `+0xcc` device mode; active-device button-prompt ownership.
6. Data source of context-action `+0x24` ids if they differ from constructor defaults (no named property file matched the six property hashes).
7. `HELI_AI_AUTO_ROLL`, `PICKUP_*` readers (`0x00732d20`), SEQUENCE_BUTTON, MC_FIRE and KICK consumers.
8. Official manuals were not readable; manual-versus-native comparison pending.
