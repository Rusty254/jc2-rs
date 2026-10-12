# Weapon accuracy: AI, assisted target, modifiers, upgrade level (JC2-112)

Research only; no runtime change. Executable SHA256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
Probe: `research/tools/weapon_accuracy_remaining_probe.py` (exit 0; byte contracts, vtable slots,
property-name hashes, retail player.bin and weapons.bin facts). Extends `weapon-accuracy-next.md` (JC2-046).

Naming: "owner flag" is the byte at character+0x728, read by accessor `006175a0` (bytes
`8a8128070000c3`). Inferred as "owner is the locally controlled player character": the
weapon-ownership brief shows it gating the player camera eye and `0x005533c0`. Clear = NPC.

## Proven

1. **Bound selector `005911b0`** (byte-exact in the probe). Owner flag clear: the bound is
   `settings+0x278` (the property `weapon_accuracy`, hash `478c677a`, loaded by a second
   settings loader near `00733f00`). Flag set and `aim_type` (+0x2ec) != 1: `settings+0x2d4`
   (also `weapon_accuracy`, loaded by `006e83a0`). Flag set and `aim_type` == 1, or no owner:
   1.0. Retail: 48 of 49 `weapons.bin` weapons have `weapon_accuracy` and every one is 1.0
   (SAM has none). So the ordinary maximum is 1.0 for every retail weapon, player and NPC.
2. **AI bound `007fa6e0`** (CCombatantAgentAvatar vtable +0x30). Input 1.0, replaced by
   `005911b0` when `00631be0` is nonzero; result `upper = clamp(min(current upper +0x61c, input), 0, 1)`.
   It can only lower the upper clamp, unlike the player's `007141e0` which assigns it.
3. **AI recovery `0072ba10`** (CCombatantAgentAvatar vtable +0x34) replaces the player's
   linear `rate*dt`. It keeps a cached weapon (+0x224), two timers (+0x228/+0x22c) refilled from
   a parameter block at +0x230 (+0x38/+0x3c) and bounds from the same block (+0x30 while the
   first timer runs, +0x34 while the second runs, else the character upper). Rise rate is
   `[+0x234][0]` scaled by `[+0x234][2]` when moving; fall is `[+0x234][1]`. The result written to
   character+0x614 is clamped to `[0, bound]` and the returned accuracy is
   `lerp(+0x618, +0x61c, +0x614)`. Bit 1 of +0x21c forces 1.0. Globals `0107b064/0107b060`
   toggle bit 2 (hysteresis, calls `005a2ef0`/`006e79c0`).
4. **AI spread callback `0074c780`** (CAgentAvatar and CCombatantAgentAvatar vtable +0xa8). With
   accuracy `t` and the owner-to-target distance `d` (0.1 when the squared distance <= 0.01):
   `w = (min_w - max_w)*t + max_w`, `h = (min_h - max_h)*t + max_h`; if `d < spread_scale_distance`
   both scale by `d / spread_scale_distance`; outputs are `atan(w/d)` and `atan(h/d)`. Settings:
   +0x264 `min_spread_width`, +0x268 `min_spread_height`, +0x26c `max_spread_width`,
   +0x270 `max_spread_height`, +0x274 `spread_scale_distance` (names recovered by hash; all five
   verified in the probe). The player/avatar callback `006be970` is `angle*(1-t)` with minimum
   `006afdf0` = constant 0. Retail `weapons.bin` has none of these keys on any weapon.
5. **Modifier ladder in `005c38a0`** (byte-exact). Input starts as character+0x610. The state
   object at this+0xe0 gives a short id via `007c2bb0` (object+0xe0): id 10 sets input 1.0 for
   any owner (zero spread); id 12 sets 0.9 only when the owner flag is set. Then
   `vtbl[+0xa8](owner, input, &yaw, &pitch)` stores the angles at +0xdf0/+0xdf4 and they are
   cleared when no callback object exists. NPC owners never get the 0.9 term.
6. **Player bound `007141e0`** (CPlayer vtable +0x30, formula re-read). Gate: control (+0x17c) and
   `00631be0` nonzero. Start `b = 005911b0`. Only if control+6 == 0 and (control+0x58 is null,
   or its +4 is null, or control+0x50 is null):
   * assisted target (control+0x3c) nonzero: `b = (1 - control[+0x9c]/control[+0x94]*(1 - F)) * b`;
   * else if player+0x2f8 == 0 and weapon `aim_type` != 1: `b = F`.
   `F` is global `0106ec7c`: static default 0.5, overridden by player child key `3433db87`, which
   retail sets to 0.0 (probe). Result clamped to [0,1] into character+0x61c. Retail
   player+0x2f8 = 150, so the no-target override never fires; the assisted branch, if reached,
   is `(1 - ratio) * b`.
7. **Upgrade level `0082f550`** (byte-exact). Owner flag clear (NPC): level 0 always
   (`settings+0x33c`). Otherwise level index `L = 006017f0(entry)` where entry is, in order:
   the object at controller+0xe00 when nonzero, else the player's Arsenal entry for the weapon id
   (`00661720` -> `005fe540`, 0x140-byte entries, key = id at +0). `L` is byte +0x1b of the entry,
   or 5 when byte +0x1a is nonzero. The angle read is `settings+0x33c + 4*L`. Setter `00615200`
   writes +0x1b. So the runtime's Arsenal level is the native source; there is no extra
   player-state term beyond the +0x1a override.

## Inferred

* Owner flag = local player (above). Type ids 10 and 12 are state ids of unknown owner class.
* The second loader's +0x278 value feeding NPC bounds is the same `weapon_accuracy` key.

## Gaps (what is unknown, what it blocks)

1. Identity of the state object at weapon this+0xe0 and of ids 10 and 12 (set where; likely an
   animation or action state; vehicle, grapple, parachute, swim and stunt entry points not tied
   to ids). Blocks replacing the approximate jitter in special states.
2. Source and meaning of control+0x94 / +0x9c (assisted-target ratio) and of the
   +0x50/+0x58/+6 gate; `CPlayerAimControl` constructor `00775f00` leaves +0x94/+0x9c untouched.
   Blocks aim-assist accuracy (no lock-on exists in jc2-rs yet).
3. Source of the AI parameter blocks at +0x230/+0x234 for `0072ba10` (rates, bounds, timers) and
   which settings file feeds them. Blocks AI recovery numbers.
4. Constructor defaults of `min/max_spread_width/height` and `spread_scale_distance`
   (absent from every retail weapon, so defaults decide AI spread). Blocks AI shot scatter.
5. Meaning of entry byte +0x1a (forces level 5). Blocks nothing visible today.
6. Which callers invoke vtable +0x30/+0x34 for NPCs (indirect calls; only the slots are proven).

Not applied to the runtime: the facts above are inputs for a later implementation task.
