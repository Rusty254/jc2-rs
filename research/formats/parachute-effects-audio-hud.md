# Parachute effects, audio parameters and HUD prompts (board task 621ea450, "R4", gap G12 of `parachute-parity.md`)

Probe: `research/tools/parachute_effects_probe.py` (46 checks, exit 0; `python parachute_effects_probe.py --game <game folder> [--tool jc2tool.exe] [--report]`).
Gap probe (board task c6fd8fd6, gaps 1-4, sections 1.2, 2.2, 3.5, 4.1 and the corrected 2.1): `research/tools/parachute_effects_gaps_probe.py` (37 checks, exit 0; same arguments plus `--scan-data` for the whole named-data reference scan).
Every function cited below is fingerprinted by sha256 in the probe; names such as `S_SKYDIVE` are resolved from the static initialiser block `0x00c70000`
(push name; hash; `mov [global], eax`). "native" = asserted by the probe from the executable bytes or retail data. "inferred" is said so.

## 0. Answers

| Question | Answer |
|---|---|
| What is in `ParachuteEffects.bin`? | Three entries, all with effect id -1: `parachute_start` -> bone `HarnessExt`, `deploy` -> bone `mid_4`, `parachute_release` -> bone `HarnessExt`. Identity offset matrices (section 1). |
| Which effect file does each entry select? | None by id: the loader `0x0082eaf0` skips every entry whose effect id is -1 (`cmp ebx,-1; je` at `0x0082ec0b`), so the three entries register nothing. The only effect file that bears the event name is `deploy` -> `Effects/parachute_deploy.bin` (dust, 0.2 s); no effect is named `parachute_start` or `parachute_release`. **The deploy dust is not started natively** (section 1.2: no registration, no other consumer of the event name, no constant for the name or id 95 anywhere): do not add it for parity. |
| What do `char_parachute_sparks.bin` (id 202) and `fx_char_parachute_leafs_01.bin` (id 448) do? | Leaves: spawned by `0x00662c20` (foliage brush) with id 448 **only while the layer-0 state is `S_PARACHUTE`** (section 2.1, corrected). Sparks: **never spawned natively** (section 2.2): effect 202 is an orphan in the retail data. |
| Native source of the FMOD `steer` parameter | `0x00590c90`: `steer = min(1, abs(D+4) + abs(D+8))` where D is the canopy flight block (`[[char+0xa34]+0x170]`), `D+4` = pitch axis, `D+8` = bank axis (see `parachute-canopy-landing-next.md` 10.3). 0 stops the loop. |
| Which sound is the canopy loop? | `Parachute_InFlight` (bank 14, sound 1), started only in canopy state `PCS_OPEN`; `Parachute_steer` (14:3) is the steering overlay, not the loop. The runtime plays `Parachute_steer` as the loop with steer 0.0. |
| Wind gating | `Wind_skydive` (14:2) plays in ten skydive/canopy states (section 3.3), not on air time; `Wind_height` (14:4) is a separate ambience above 200 m. |
| HUD "Open parachute" condition | Active character controller is `CSkyDiveController` and the player byte `character+0x47c` is set. No air-time threshold exists. |
| HUD "Close parachute" / "Reel in" | Active controller is `CParachuteController`: "Close parachute" (action 0x47, SPACE) when no wire is attached, "Reel in" (action 0x52, E) when the grapple device holds a wire. The function posts one of them per frame. Both labels appear together in the reference shots because "Close parachute" is also the GUI parachute button, driven by separate `gui.parachutebutton.open/close` events (section 4.1). |
| Audio fade-in curves | None: all four slot curves are the default curve (constant 1.0), gain argument is 1.0; the volume ramp, if any, lives in the .fev (section 3.1, 3.5). |

## 1. `ParachuteEffects.bin`

Path `effects\parachuteeffectsettings\parachuteeffects.bin` (372 bytes), named by `settings\parachuteparams.bin` property `#b868e836` and loaded by the
canopy load function `0x007fe910` (call to `0x0082eaf0` on the model's effect component `model+0x5c`) and also pre-loaded by the player constructor
(`0x00730000` block, next to `RicosEffects.bin`).

Root node `#bcf79e68` (count field `#730217c5` = 3). Children are keyed by the Jenkins hash of the event name (`parachute_start` = `0966293f`, `deploy` = `19d82efc`,
`parachute_release` = `59cf1094`; probe asserts the hashes):

| key (event) | bone `#9ab0ffd8` | slot `#6a9dfd76` | effect id `#a90d6c5c` | matrix |
|---|---|---|---|---|
| `parachute_start` | `HarnessExt` | 2 | -1 | identity |
| `deploy` | `mid_4` | 0 | -1 | identity |
| `parachute_release` | `HarnessExt` | 1 | -1 | identity |

The events fire from the animation event tracks (`deploy_parachute_vertical/horizontal`: `parachute_start` 0.07 s, `deploy` 0.30 s; `close_parachute`: `parachute_release` 0.16 s;
the probe scans all 148 event tracks: no other track fires the start or release names).

### 1.1 How the file is consumed (native, decompiled reading plus byte checks)

* `0x0082eaf0(settings, file)` walks the root's children. For each child: `0x0082ea00` initialises a 0x14-byte record (event hash +0, bone +4, matrix +8, **effect id +0xc = -1**,
  flag bytes +0x10..+0x13 = 1,0,0,0); `0x00789da0` fills bone (string `#6be89c08`/`#9ab0ffd8` -> bone index), matrix, effect id (`#a90d6c5c`) and the four flag bytes.
* `cmp ebx, -1; je` at `0x0082ec0b` (ebx = record `+0xc`) jumps over the three registration calls `0x005a1410` (store the record), `0x007f5f80` and `0x007f2130`
  (register event hash and effect with the effect system). So **an entry with effect id -1 is not registered**. `RicosEffects.bin`, which uses the same loader, carries real ids
  (11, 94, 205, 206, 207, 267, 358; the probe asserts none is -1).
* The slot field `#6a9dfd76` (0, 1, 2 here; 2 to 8 in Rico's file) is not read by `0x00789da0`; it is an index only.
* The runtime consumer of registered records is the `CAnimatedModelEffects` handler `0x005a0810` (vtable `0x00eb87c0`, slot 1): it fires only for events whose source is the owning model
  (`event.source == this+0x38`) and that are in the registration table, then starts the effect.

Consequence: in the retail data the three ParachuteEffects entries are inert through this loader. Section 1.2 closes the question whether another path starts the `deploy` dust.

### 1.2 The deploy dust is not started (gap 1, native, c6fd8fd6)

Entry parser keys (static initialisers at `0x00eb5c1c..0x00eb5c2c`): `#6a9dfd76` = slot index, `#9d5ffbb3` = start-event name, `#6be89c08` = stop-event name, `#a90d6c5c` = effect id. For an entry whose id is not -1 the loader
(`0x0082ec17..0x0082ec58`) stores the record (`0x005a1410`), registers the start-event name for the slot with flag 1 (`0x007f5f80`, `push 1` at `0x0082ec2d`) and, only when a stop-event name key is also present, registers it with flag 0 and
registers the slot's effect id (`0x007f2130`). `0x007f5f80` and `0x007f2130` only write into the event system and the slot map; neither spawns anything.
The only runtime reader is the handler `0x005a0810`: it ignores events whose source is not the owning model (`cmp edx,[ecx+0x38]` at `0x005a0836`), walks the 12-byte entries {event set, slot, flags} at `this+8`, and for the first entry whose event set contains the
event hash (`0x004e5be0`) starts the slot (`0x00799aa0` when flag bit 0 is set) or stops it (`0x0052db10`). The parachute model's table is empty because all three entries carry id -1.

Nothing else can pick the event up by name:
* the executable contains no NUL-delimited string `deploy`, `parachute_start`, `parachute_release`, `char_parachute_sparks`, `fx_char_parachute_leafs_01` or `parachute_deploy`, and none of their Jenkins hashes (`19d82efc`, `0966293f`, `59cf1094`, `ee0c3687`,
  `04b3a7b8`, `4eb774e7`) occurs as a constant anywhere in the image: no code compares or registers these names;
* the `CAnimatedParachute` constructor `0x007e0250` registers listeners for exactly two event names, `show_parachute` (`0x007e0337`) and `hide_parachute` (`0x007e0371`);
* the id 95 never appears as a data value in any named property, state or track file (the probe's `--scan-data` greps 809 files): the `deploy` entry of `effectsettings.bin` (id 95, `Effects/parachute_deploy.bin`) is referenced by nothing.

So in the original game the opening canopy emits **no dust**; the `deploy`, `parachute_start` and `parachute_release` animation events only drive sound/model logic elsewhere. (Unnamed data archives were not scanned; a producer that passes the id 95 from an unnamed table
cannot be excluded, but the executable has no literal for it either: the 23 call sites of the spawn function `0x00579f50` pass literals only at `0x00662e0b` (448) and `0x0068e40e` (370).)

## 2. The effect files

`effects\effectsettings.bin` maps names and numeric ids to effect files (468 entries). The three parachute-related files:

| id | name key | file | content |
|---|---|---|---|
| 95 | hash of `deploy` (`#19d82efc`) | `Effects/parachute_deploy.bin` | one-shot, `fx_parachute_deploy_dust_01`, duration 0.2 s (node `#e62de7fa`, `#488c291b` = 10) |
| 202 | `char_parachute_sparks` | `Effects/char_parachute_sparks.bin` | looping (`looping = 1`), `fx_char_parachute_sparks_01` |
| 448 | `fx_char_parachute_leafs_01` | `Effects/fx_char_parachute_leafs_01.bin` | one-shot, `fx_char_parachute_leafs_01` |

No `effectsettings.bin` entry hashes to `parachute_start` or `parachute_release`. Among all event selectors in the 148 event tracks, the names that coincide with an
`effectsettings.bin` entry are `deploy`, `footstep`, `character_fall_small` and `rumble_and_shake_medium`: the event-name = effect-name convention exists in the data.

### 2.1 Leaves (id 448), native

`0x00662c20(dt, speed, position)` (called from the player-sound update `0x00718e80` every frame with the player's speed): when the character's height above ground is at most 80.0
(`CCharacterRules+0x14`), speed is above 0.1, and the 64 m terrain-grid cell under the position (index `(x - DAT_01076b88) * 0.015625`, 0x200 cells) passes the foliage test `0x00672e00`,
it arms the cooldown (`this+0x8c`, `0.25 + random * 0.25` s, set whenever the foliage test returns 1, spawned or not). The effect itself is spawned **only while the layer-0 state is `S_PARACHUTE`** (`0x00849700`, copied to `bl` at `0x00662c3c`;
`test bl,bl; je` at `0x00662dca` skips the spawn block otherwise), and then at `position + (0, 0.45 + 2.5, 0)` using the character's own transform matrix (`character.sub-object vtable +0x48`, called at `0x00662dd7`) with the translation replaced (under the canopy the foliage test itself also gets a larger probe: 2.0 and 0.45 x 2.5). The
earlier reading (spawn also on foot or in a skydive, canopy only changing the height) was wrong: on foot and in freefall only the sound side of the function runs (sound-controller parameter 2 and event 0x14). The runtime has no such effect.

### 2.2 Sparks (id 202): never spawned (gap 2, native negative, c6fd8fd6)

* `.text` holds exactly two instructions with the immediate 202: `push 0xca` at `0x00beb3b9` (the line-number argument of a Havok assert, string `hkpConstraintUtils.cpp` pushed next) and `mov [0x0109b124], 0xca` at `0x00cf777a`, one slot of a 40-entry integer
  table (42, 34, 38, 78, 4, 126, ... 94, 202, 266, 54 ...) written by a static initialiser and read only at `0x00ae2085` inside a library routine that rewrites the same table with other values (`0x00ae1f00`): not an effect id table.
* No code references the name `char_parachute_sparks` or its hash `ee0c3687` (section 1.2); the spawn function `0x00579f50` passes ids from tables, but no named data file contains 202 as an effect id (the `= 202` hits are a dialog sound id and a PDA entry order; probe `--scan-data`).
* `RicosEffects.bin` (ids 11, 94, 205, 206, 207, 267, 358) and `ParachuteEffects.bin` (-1) do not carry it, and no animation event selector is `char_parachute_sparks`.

Conclusion: `char_parachute_sparks` (looping, `fx_char_parachute_sparks_01`) is an orphan asset in retail; the original game shows no sparks under the canopy. Do not implement it. (Same residual caveat as 1.2 about unnamed archives.)

## 3. Audio

### 3.1 The four sound slots (native)

`CPlayerSounds` (RTTI `CPlayerSounds`, update `0x00718e80`) keeps four slots; `0x006f4930` creates slot n from the table `0x00ec20a8` (bank) / `0x00ec20b8` (sound):

| slot | bank:sound | FMOD event | event parameters (in .fev order) |
|---|---|---|---|
| 0 | 14:2 | `Parachute/Parachute/Wind_skydive` | speed, focus, height, speed vertical |
| 1 | 14:1 | `Parachute/Parachute/Parachute_InFlight` | speed, focus, param04 |
| 2 | 26:1 | grapple reel loop (not parachute) | |
| 3 | 14:3 | `Parachute/Parachute/Parachute_steer` | steer, focus, (distance), var |

Plus `Wind_height` (14:4: variation, focus, height) created separately by `0x00842630`. The parameters are set by index (`getParameterByIndex`, `0x00a0a270`), which is why
the strings "steer" and "speed vertical" do not appear in the executable.

`0x00671960(slot, dt, value, gain, position, mode)`: ensures the slot's sound exists (`0x006f4930`), sets **parameter 0 = value**, accumulates `dt` into a per-slot timer (`this+0x74`), sets
**parameter 1 = curve(timer) * gain** (curve object per slot at `this+0x64`, evaluator `0x009e90f0`: types 0 linear, 1 square, 2 inverse square between a start and an end value over a duration),
sets the 3D position, and with mode 2 starts the sound if it is not playing (mode 1 starts unconditionally, mode 0 only updates). The curve values: see section 3.5 (all four are the default curve, constant 1.0).

### 3.2 Canopy loops, `0x00590c90(dt, speed, gain, position)` (native)

Canopy object = `[[this+0x28]+0x484]` (weapon/item holder, `0x005cf980`) `+0xcc`; vtable `+0x24` is "state is PCS_OPEN", `+0x28` "is opening", `+0x30` "is closing" (slots 9, 10, 12 of the `CAnimatedParachute` table in `parachute-deploy-collapse.md`).

| canopy state | slot 1 (`Parachute_InFlight`) | slot 3 (`Parachute_steer`) |
|---|---|---|
| open (`PCS_OPEN`) | `0x00671960(1, dt, speed, gain, pos, mode 2)`: **started**, param 0 = speed | steer logic below |
| opening | mode 0: parameters only, never started here | steer logic below |
| closing | mode 0 (parameters only); `0x00637a60(3)` marks `this+0x34` = 3 | untouched |
| closed or no canopy | stopped (`0x00853060(1)`) | stopped (`0x00853060(3)`) |

Steer logic: `s = abs(D+4) + abs(D+8)`; `s <= 0` -> slot 3 parameter 0 = 0 and the slot is stopped; otherwise `0x00671960(3, dt, min(s, 1.0), gain, pos, mode 2)`: param 0 = `min(s, 1)`, started if needed.
`D` is `[[[this+0x28]+0xa34]+0x170]` (`0x00590d10`, `0x00590d16`; `this` = `CPlayerSounds`, `this+0x28` = the player character), the block whose `+4`/`+8` are the pitch and bank axes recomputed every flight frame (`0x0071a190`, landing doc 10.3).
Neutral glide (`D+4 = D+8 = 0`) therefore means **no steer loop at all**.

### 3.3 Wind, `0x00568530(dt, speed, gain, position)` (native)

Calls `0x00590c90` first, then plays **slot 0 (`Wind_skydive`)** when the layer-0 state of the player's graph is:

* one of ten states (`0x0065ff80`): `S_SKYDIVE`, `S_FALL_TO_SKYDIVE`, `S_JUMP_TO_SKYDIVE`, `S_TO_SKYDIVE_DASH`, `S_SKYDIVE_DASH`, `S_FROM_SKYDIVE_DASH`, `S_PULL_OPEN_PARACHUTE_VERTICAL`,
  `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_UNFOLD_PARACHUTE_HORIZONTAL`, `S_UNFOLD_PARACHUTE_VERTICAL`; or
* one of three canopy states (`0x0076fc30`): `S_UNFOLD_PARACHUTE_HORIZONTAL`, `S_UNFOLD_PARACHUTE_VERTICAL`, `S_PARACHUTE`; or
* `S_REEL_FLIGHT` (gain factor 0.6, constant `0x00e2c350`).

Any other state stops slot 0 (`0x00853060(0)`). So the wind plays during the canopy too (not only freefall), and also while being reeled by the grapple, and **never in a plain fall** (`S_FALL`).
Parameters: 0 = speed (length of the velocity `0x008a0bd0`), 1 = curve(time since start) * gain, and event parameter 3 ("speed vertical") = `max(0, -vy)` (downward speed only; the runtime passes `|vy|`).

`Wind_height` (`0x00842630`, once per frame from `0x00718e80`): only when the player's world y exceeds **200.0** (`0x00ec205c`); gain parameter 1 = `(1 - 0.05 * min(|vy|, 5)) * dt-scale` and 0.75 of that while already playing; parameter 2 = a smoothed
(factor 0.1) wind-exposure value from terrain heights sampled at +-40 m around the player (`DAT_01190290` terrain sampler `0x00a645a0`; a value of 50 m above the terrain scales it). Outside 200 m the sound is stopped.
It is an ambient for high altitude (jets, base jumps), not tied to the parachute states.

### 3.4 Runtime today (`audio.rs`)

* `chute_open`: `Parachute_Vert` on deploy (the character event `parachute_open_vertical`; the native sends `parachute_open_horizontal` via `0x0055f450` for horizontal entries, which `soundmap` maps to 14:0 `Parachute_Horiz`).
* `chute_loop` = `Parachute_steer` held whenever `hero.parachute`, `steer` = 0.0. Native: `Parachute_InFlight` as the loop (open only) and `Parachute_steer` only while steering, with steer from the flight block.
* `wind`: held when `air_time > 0.3` outside vehicles; params `speed`, `speed vertical = |vy|`. Native: by graph state (section 3.3), vertical speed `max(0, -vy)`, not in a plain fall, and during the canopy.

### 3.5 Fade-in curves and the update loop (gap 3, native, c6fd8fd6)

* The per-slot curve is 16 bytes {type, duration, start, end}; the evaluator `0x009e90f0(t)` returns `end` when `t >= duration`, `start` when `t <= 0`, otherwise `start + (end - start) * f` with f = t/duration (type 0), f squared (type 1),
  `1 - (1 - t/duration)^2` (type 2) and 1.0 for any other type.
* The `CPlayerSounds` constructor `0x007bd8f0` fills the four-element curve vector (`this+0x60`) with copies of the default curve built by `0x009fa260`: **type -1, duration 0, start 0, end 1.0**, and zeroes the four timers (`this+0x70`). Nothing ever assigns the
  curves afterwards (`0x009fa260` has one caller; the only code that indexes the vector is the slot update `0x00671960`, the stop-with-fade routines `0x006a0290` and `0x0051bbc0`, all readers). With duration 0 the evaluator returns 1.0 for every time, so
  event parameter 1 (`focus`) = `gain` exactly, with no software fade-in or fade-out.
* The gain is 1.0 at every call: `0x00718e80` pushes `fld1` for the wind updater `0x00568530` (and through it the canopy loops `0x00590c90`) and for `Wind_height` (call sites `0x00718fac`, `0x00718fef`). The only other factor is 0.6 for `S_REEL_FLIGHT` (section 3.3).
  So `Wind_skydive`, `Parachute_InFlight` and `Parachute_steer` all start at full `focus`; any attack envelope is inside the .fev (not parsed here).
* Parameter 0 ("speed") is the length of the velocity vector (`0x008a0bd0`), raised to 0.01 (`[0x00f66934]`) when it is in (0, 0.01]; a zero velocity stays 0.
* Stopping is immediate: `0x00853060(slot)` asks the instance whether it is playing and calls its stop method with argument 0 (vtable +0x3c at `0x0085312a`), otherwise releases it (`0x004810f0`). The timer is not reset by it.
* Global silence: when `[0x0118fb84]` is non-null with `[+0x5c] > 0`, or byte `[0x011958db]` is non-zero, `0x00718e80` skips all updates, stops the controller at `this+0x44` and stops slots 0 to 3 (`0x00719013`). The meaning of those two globals (pause/cutscene-like) is unresolved.

## 4. HUD prompts, `0x0052a650` (native)

Per-frame player context-prompt function. Preconditions: the HUD input device (`DAT_01196074`, `0x00747b70(5)`) and a controlled character (`DAT_0118fb24+0x3570`). Let `C` be the active character controller (its class is tested through the `IsA` slot at vtable `+8`) and `F = character+0x47c`.

* `C` is not `CParachuteController`:
  * `F != 0` and `C` is `CSkyDiveController`: **"Open parachute"** (`hud_button_04`) on action 0x47 (PARACHUTE_OPEN_CLOSE, the SPACE chord).
  * else `C` is `CUnmountedGunController`: `hud_button_08` ("Drop") on action 0x26. Otherwise the weapon branches (`CWeaponController`, `CDriverSeatController`, `weapon_%i`, `fe_txt_153`); not parachute related.
* `C` is `CParachuteController`: device = `[character+0x484]`, wire pointer `[[device+0xd0]+0x128]`: **zero -> "Close parachute"** (`hud_button_05`, action 0x47); **non-zero -> "Reel in"** (`hud_button_07`, action 0x52 = E, `hud_button_07` text "Reel in").
* The prompt's key shown is the binding of the action id (`0x006f73a0(action, text)`; the action id is looked up in the input map: `0x00983ad0`).

What `F` is: `character+0x47c` is the "parachute allowed" flag. The player script-event handler `0x00706c30` sets it to 1 on `ply.parachute.enable` and clears it (closing the canopy first through `ACT_PULLCHORD`) on `ply.parachute.disable`
(handlers registered at `0x00730000`: events `ply.parachute.disable` at `this+0xbc`, `ply.parachute.enable` at `this+0xc0`; the pairing with the 0/1 stores is read from the code shape and names, inferred). The flight model `0x005a9950` is gated by the same byte.

When is `C` the skydive controller? Native table (`foot-conditions-next.md` section 4): `CSkyDiveController` serves `S_PULL_OPEN_PARACHUTE_VERTICAL`, `S_SKYDIVE`, `S_PARACHUTE_DETACH_TO_FREEFALL`, `S_TO_SKYDIVE_DASH`, `S_FROM_SKYDIVE_DASH`, `S_SKYDIVE_DASH`, `S_FALL_TO_SKYDIVE`;
`CParachuteController` serves `S_UNFOLD_PARACHUTE_VERTICAL/HORIZONTAL`, `S_PARACHUTE`. So the prompt appears the moment the foot machine enters `S_FALL_TO_SKYDIVE` (28.5 m of free air, per that doc) and is replaced by "Close parachute" when the canopy states are entered.
`S_JUMP_TO_SKYDIVE` is the base-jump controller: no prompt.

The runtime rule (`hud.rs`: `hero.air_time > FALL_PROMPT_AFTER` 0.35 s, plus airborne/not on a wire exclusions) is therefore not the native condition; it should read the foot-machine state (the same state the camera and the wind gate use) and the allowed flag. The prompt
"Open parachute" on a grapple wire (existing test `labels(&wire) == ["Open parachute SPACE", "Drop F"]`) is the reeled-flight entry `CDeployParachuteWhileReelingAction` (control id 146, icon `hud_button_04`, `grapple-action-selection-next.md`), a different producer.

### 4.1 Why "Close parachute SPACE" and "Reel in E" are shown together (gap 4, native + inferred GUI side, c6fd8fd6)

Two independent producers feed the HUD:

1. **The face-button prompt buffer** (the producer of section 4). Per frame the HUD update calls `0x0083d520` (call at `0x0052b2e7`) which flips bit 0 of the buffer's first byte, compares the two buffers' six slot strings (event `gui_facebuttons_changed`
   when they differ) and sets the six slots of the new current buffer to the string `"-1"`; then `0x00837120` and, when the HUD state `[this+0x1a4]` is 4, `0x0052a650` post this frame's prompts (`0x0052b3e7`). The GUI (Lua `GetFacebuttonString`,
   `0x005a0730`) reads the six strings of the **other** buffer, i.e. last frame's posts. A post (`0x006f73a0(action, text)`) happens only if the action has a binding code 0x70..0x73 (`0x00983ad0(action, 4)`); `0x007e6530` writes it to slot `code - 0x70`
   when input device type 0x10 is present, else to the first free of the six slots, so several prompts posted in one frame coexist. Under the canopy `0x0052a650` makes exactly **one** post per frame at `0x0052a971`: `hud_button_07` "Reel in" (action 0x52, E) when
   the grapple device holds a wire, else `hud_button_05` "Close parachute" (action 0x47, SPACE). Each of `hud_button_04/05/07` has a single code reference (all in `0x0052a650`), and the grapple `CReelInAction` context action has an empty prompt text in
   `grapplingparams.bin`, so no other native producer of "Reel in" exists.
2. **The GUI parachute button.** The per-frame HUD update `0x00792510` posts GUI events through `0x004e88b0(name, "", 0)` whenever the character may use a parachute (canopy holder present, byte `character+0x47c` set):
   * layer-0 state in the three canopy states (`0x0076fc30`): canopy `PCS_CLOSED` (vtable +0x2c) -> `gui.parachutebutton.open` (`push 0x00e48954`, `0x007927a1`); canopy `PCS_OPEN` (vtable +0x24) -> `gui.parachutebutton.close` (`0x00e3f09c`, `0x007927bc`); opening/closing: no event;
   * canopy object is not a `CAnimatedParachute` (IsA check with the class name at `0x00e3e024`) and the layer-0 state is one of the ten skydive states (`0x0065ff80`): `gui.parachutebutton.open` every frame (`0x0079286d`); for a `CAnimatedParachute` the thruster
     branch runs instead (`gui_thruster_show` event, `ACTIVATE_PARACHUTE_THRUSTERS`).

So with a wire attached under an open canopy the buffer says "Reel in E" while the Anark GUI parachute button (event `gui.parachutebutton.close`) shows the close control: both on screen at once. (Inferred, not native-proven: the label and key text of the GUI button live in
the Anark GUI package, which is not parsed; its text being "Close parachute" with the SPACE key is read from the reference shots. Without a wire the buffer posts "Close parachute" as well, so the GUI side must either hide the duplicate or show both; unknown.)

## 5. Runtime guidance (paired implementation task: audio.rs and hud.rs only)

Depends on I1 22e54146 for the real `D+4`/`D+8`; until then feed the existing stick axes and label it non-native.

1. Canopy loops: replace `chute_loop` by `Parachute/Parachute_InFlight` (event key from the soundmap 14:1, a `soundmap` lookup like `chute_open`) held while the canopy state is `PCS_OPEN` (not during unfolding); add `Parachute_steer` (14:3) held while `min(1, |D+4| + |D+8|) > 0` with `steer` = that value; stop both at `closed`.
2. Wind: hold `Wind_skydive` (14:2) when the foot-machine state is in the ten skydive/canopy states, `S_PARACHUTE` or `S_REEL_FLIGHT` (x0.6), parameters `speed` = |v|, `speed vertical` = max(0, -vy); remove the 0.3 s air-time rule. Optionally add `Wind_height` above y = 200 (section 3.3).
3. Prompts: "Open parachute" when the foot-machine state is a `CSkyDiveController` state and the allowed flag is set (default true; `ply.parachute.disable` clears it); under the canopy "Close parachute" without a wire, "Reel in" (action E) with a wire attached; drop `FALL_PROMPT_AFTER`. Under the canopy the native frame posts exactly one face-button prompt (section 4.1); the GUI parachute button (`gui.parachutebutton.open/close`) is a separate control that shows the close/open action by canopy state, so the HUD may show both "Close parachute SPACE" and "Reel in E" when a wire is attached (do not make `hud.rs` pick one).
4. Effects: leaves effect 448 (section 2.1, **only while `S_PARACHUTE`**, needs the foliage-cell test; position +0.45+2.5 m). Do **not** implement `parachute_start`/`parachute_release` effects (nothing registers them), the `deploy` dust (section 1.2: never started) or the `char_parachute_sparks` effect (section 2.2: never spawned). Audio focus is 1.0 with no software fade (section 3.5): use full volume on start, ×0.6 only in `S_REEL_FLIGHT`.

## Gaps

Closed by c6fd8fd6 (kept for traceability): 1 deploy dust trigger (none, section 1.2); 2 sparks producer (none, section 2.2); 3 fade-in curves (constant 1.0, section 3.5); 4 Close + Reel in overlap (two producers, section 4.1).

1. (open, blocks exact HUD parity of the parachute button) The GUI parachute button's content: the Anark GUI package behind `gui.parachutebutton.open/close` (label text, key glyph, whether it hides while the face-button prompt shows the same action) is not parsed. The event conditions are native (section 4.1); what the GUI draws for them is inferred from the reference shots.
2. (open, low) Meaning of the two globals that silence all four sounds in `0x00718e80`: `[0x0118fb84]` (object with a counter at `+0x5c`) and byte `[0x011958db]`; and of `[0x011958c4] == 2`, which skips the prompt-buffer flip at `0x0052b2d4`. Probably pause/cutscene states; unproven.
3. (open, low) Whether unnamed archive members reference effect ids 95 or 202 (sections 1.2, 2.2 are exhaustive for the executable and the 809 named property/track files only).
4. Roles of `this+0x34` in `0x00590c90` (values 0, 1, 3: canopy open/opening, none, closing) beyond stopping the loops, and of `0x00637a60(3)`.
5. Wind_skydive parameter 2 ("height") and Parachute_InFlight parameter 3 ("param04") are not set by `0x00671960`/`0x00568530`; the sound-side data (event defaults) decides their value.
6. `Wind_height` parameter 2's exact formula (terrain-exposure sum, `0x00842630` tail); only its gate, caps and constants are asserted.
7. Whether `ply.parachute.disable/enable` map to the 0/1 stores exactly as inferred (event-handler shape).
