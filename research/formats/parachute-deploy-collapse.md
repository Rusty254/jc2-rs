# Parachute deploy and collapse visuals (board task 60caf984, gaps G4/G5 of `parachute-parity.md`)

Research only. Every address below is asserted by `research/tools/parachute_deploy_collapse_probe.py` (32 checks: function hashes, vtable,
name-hash initialiser, instruction anchors, clip/asb/event-track data, a pure-Python timeline model). "native" = asserted by the probe;
"inferred" says so. Reference shots read: `pilot-chute-bag-opening-above-rico-daylight_bw39`, `canopy-draped-over-cruiser-landing_bx11` (others by name only).

## 1. Answers in one page

* There is **no pilot chute / bag object**. The canopy is one animated model (`CAnimatedParachute`, vtable `0x00eb4224`, constructor `0x007e0250`, model
  `weapons\gae05\GAE05_LOD1-GAE05`, skeleton `parachute.bsk`) driven by its own state machine `animations/statemachine/parachute.afsm`
  (`PCS_CLOSED`, `PCS_OPENING_VERTICAL`, `PCS_OPENING_HORIZONTAL`, `PCS_OPEN`, `PCS_CLOSING`). The "bag" in the reference is the same mesh with all chain bones
  packed to about 0.1 m (section 3), sitting at the harness; the "crumpled canopy" is the same mesh part-way through the inflation.
* **Inflation is bone animation only** (translations/scales of `base`, `lft_1..4`, `mid_1..4`, `rgt_1..4` and the two handles): vertical deploy packs for 0.5 s then
  inflates between 0.55 and 1.0 s; horizontal inflates in 0.4 s. No cloth, no Havok body is created for the canopy (constructor and loader build only the model, the
  effects object and a camera-collision phantom).
* **Collapse is the 1.4 s clip `parachute_release.ban`** (state `PCS_CLOSING`), then the model is hidden. The canopy keeps following the character matrix while it plays.
  No fall, no drape physics (inferred from absence; see Gaps). The cut-away additionally hides the model at 0.5 s through the `hide_parachute` event of the character clip
  `detatch_chute.ban`.
* **Visibility**: the model is hidden while inactive and shown by the `show_parachute` event of the opening clip (0.33 s vertical, 0.03 s horizontal), hidden by
  `hide_parachute` or when `PCS_CLOSED` is reached.

## 2. The class (native)

Vtable `0x00eb4224` (4-byte slots; offsets in brackets):

| slot | address | role |
|---|---|---|
| 6 [0x18] | `0x0075aa00` -> `0x005ed720` | open **vertical**: sends action `PCA_OPEN_VERTICAL` to the model's rule system; on acceptance `+0x48 = 1` (active), `+0x49 = 1`, else `+0x49 = 0`; then the camera phantom is enabled and a character callback (`character+0x9c8`, slot `+0x9c`) runs |
| 7 [0x1c] | `0x005a7230` | close: sends `PCA_CLOSE` (valid from opening and open states) |
| 8 [0x20] | `0x00749000` | close now: sends `PCA_CLOSENOW` and clears the model flag bits `0x11` (hidden) |
| 9 [0x24] / 10 [0x28] | `0x00777160` / `0x00774c50` | state is `PCS_OPEN` / state is an opening state |
| 11 [0x2c] / 12 [0x30] | `0x007fb5b0` / `0x007fccb0` | state is `PCS_CLOSED` / `PCS_CLOSING` |
| 13 [0x34] | `0x0067b2d0` | **show**: sets bits `0x11` of the model flags (`+0xc`) and of the two parts of the effects pair at `this+0x6c` |
| 14 [0x38] | `0x0061ac60` | **hide**: clears bits `0x11`; if the effects pair exists tail-jumps to `0x00621a00` |
| 15 [0x3c] | `0x006ccd40` | update (below) |

Open **horizontal** is the non-virtual `0x00759c90` (`PCA_OPEN_HORIZONTAL`, same body otherwise). Callers: `0x00581a8c` (`CDeployParachuteWhileReelingAction` `0x005819e0`),
`0x005a90f5` (action `0x005a9030`, ACT_PULLCHORD accepted and the canopy is a `CAnimatedParachute`), `0x0067b67b`, `0x0082b171`. The free-fall chord goes
through the driver `0x0074edd0` (slot 6, vertical). Mapping: free-fall pull = vertical clips (character `rico_open_chute`, state `S_UNFOLD_PARACHUTE_VERTICAL`);
reel/roof/vehicle entries = horizontal (`rico_reel_open`, `S_UNFOLD_PARACHUTE_HORIZONTAL`). The action name hashes live in globals set by the initialiser `0x00c749c0`
(`0x0119591c` open V, `0x01195c44` open H, `0x01195920` close, `0x0119593c` close now, `0x01195e90` open, `0x01195e8c`/`0x01195940` opening V/H, `0x0119595c` closed, `0x01195968` closing).

Update `0x006ccd40` per frame: (1) if `this+0x48 == 0` call close-now (keeps the model hidden) and return; (2) close rule (b) of the landing doc (contact, `+0x4a`, state not
unfold/pullchord) calls close; (3) model update, bone attach (`0x00657010`: handle and harness bones follow the character hand bones while open/opening), camera
phantom (`0x005a60a0`), effects; (4) the 16-float matrix argument is copied to `this+8` **every frame, also while closing**; (5) if the state is `PCS_CLOSED`: hide and
`+0x48 = 0`; (6) two bytes of the model are written from the character (`+0x23` from `0x00557050(1)` via `0x00a5e420` times 255, `+0x20` from `character+0xa64` times 127;
meaning unknown, a lead for R2 shading/night).

The driver `0x0074edd0` opens only when the canopy is neither open nor opening, closes only from `PCS_OPEN` (chord pressed); an opening canopy is not closed by the driver,
but rule (a) of the flight model (state left the parachute states) and rule (b) (touchdown) close from any state.

## 3. Clips and timing (native; sampled with `jc2tool anim --at`)

`parachute.asb` states: `PC_ANIM_OPENING_VERTICAL` = `chute_open_chute.ban` (74 frames, **2.433 s**, event track `deploy_parachute_vertical`), `PC_ANIM_OPENING_HORIZONTAL` =
`chute_reel_open.ban` (**2.0 s**, `deploy_parachute_horizontal`), `PC_ANIM_CLOSING` = `parachute_release.ban` (**1.4 s**, no event track), `PC_ANIM_CLOSED` =
`chute_open_chute.ban` (rest pose). The afsm leaves `PCS_OPENING_*` and `PCS_CLOSING` on `C_WAIT_FOR_ANIM_TO_FINISH` (clip end). Character clips are the same lengths
(`rico_open_chute` 2.433, `rico_reel_open` 2.0, `detatch_chute` 1.4). The field roles `371aba7e`/`57ab7197`/`96270a00` of the asb are still unproven (not used here).

Inflation metric = sum of bone offsets `|lft_2..4| + |rgt_2..4|` (metres; full canopy 5.91):

| clip | t (s): length |
|---|---|
| vertical | 0 to 0.5: 0.10 (packed bundle at the harness, `base` y 0.15) / 0.6: 0.32 / 0.8: 3.81 / 1.0: 5.91 (`base` rises to y 1.42) / 2.4: 5.91 |
| horizontal | 0: 0.47 / 0.1: 1.68 / 0.2: 3.80 / 0.3: 5.50 / 0.4: 5.96 |
| release | 0: 6.09 / 0.5: 5.18 / 1.4: 4.81 (21 percent contraction); `base` static at (0, 1.144, -0.012) |

Release also throws the handle bones up and forward in canopy space: left handle y 0.48 (0 s), 1.05 (0.5 s), 3.69 (0.8 s), 8.03 (1.4 s); z -0.15 to +4.56. Reading: the lines whip up
over the canopy while it contracts.

Events (stored raw times on the clip clock): vertical `parachute_start` 0.07, `deploy` 0.30, `show_parachute` 0.33; horizontal `parachute_start` 0.07, `deploy` 0.30, `show_parachute` 0.03;
character `close_parachute` (on `ANIM_PARACHUTE_TO_SKYDIVE` = `S_PARACHUTE_DETACH_TO_FREEFALL`, clip `detatch_chute.ban`): `parachute_release` 0.16, `hide_parachute` 0.50.
`ParachuteEffects.bin` maps `parachute_start` -> bone `HarnessExt` effect index 2, `deploy` -> `mid_4` index 0, `parachute_release` -> `HarnessExt` index 1; `parachute_deploy.bin`
spawns `fx_parachute_deploy_dust_01` (0.2 s). Which effect file each index selects is unproven (R4 621ea450). The canopy's own closing state has no event track, so the
`parachute_release` effect is only known to fire on the cut-away path.

## 4. Timelines (model in the probe; times from the command)

* **Free-fall deploy**: command at t0 (ACT_PULLCHORD accepted, same call opens the canopy, D+0x28 = -0.825). Hidden until t0 + 0.33 s, shown as a 0.1 m bundle at the harness;
  dust effect at 0.30; inflation 0.55 to 1.0 s (the crumpled mass of `bw39`/`ck47`); steady open from 1.0 s; `PCS_OPEN` (blend clip set) from 2.433 s.
* **Horizontal deploy**: shown at 0.03 s, 0.47 m, fully inflated by 0.4 s, `PCS_OPEN` at 2.0 s.
* **Landing close**: touchdown with contact -> close; model stays visible while `parachute_release` plays, hidden when `PCS_CLOSED` is reached (about 1.4 s after touchdown, no
  hide event on this path); meanwhile the character plays `S_PARACHUTE_LANDING` etc.
* **Cut-away** (chord pressed while open, ACT_PULLCHORD to freefall): canopy close starts the same frame (rule (a), the state leaves the parachute states); the character plays
  `detatch_chute` (1.4 s); `hide_parachute` removes the model at 0.5 s (contracted 15 percent, handles about 1 m up); the canopy never falls in world space.
* **Script**: the mission event `ply.parachute.closenow` (registered in the player constructor `0x00730000` block) is the hard-close entry (`PCA_CLOSENOW`); its handler was not located.

## 5. Reference shots (words)

`bw39`: a dark crumpled mass above Rico with lines already taut and the prompt "Close parachute" visible: matches the mid-inflation window (0.6 to 0.9 s, length 0.3 to 3.8 m).
`bx11` ("draped over cruiser"): extreme close view of the ribbed canopy with large flat cut-out polygons: the camera is inside/against the canopy mesh while landing; it shows
the mesh intersecting geometry (near-plane/camera, bank), not cloth draping. No evidence of a persisting canopy after landing.

## 6. Runtime guidance (paired implementation task)

Today `parachute.rs` `animate_canopy` plays `chute_open_chute.ban` from visibility (already correct clip, but shows the canopy at 0 s and clamps at 2.433 s) and `character.rs`
hides the canopy at once. Replace with a small `CanopyState` (Closed, OpeningVertical, OpeningHorizontal, Open, Closing) following section 2/4: open command starts the clip
(vertical unless the entry is a reel/roof/vehicle entry), visibility from the event times, closing plays `parachute_release.ban` while following the character matrix, hide at
`PCS_CLOSED` or the 0.5 s event on the cut-away. Keep the existing `rig.bone_positions(clip, t)` sampler (it already lerps scale).

## Gaps

1. Dispatcher of `show_parachute` / `hide_parachute` (a `CEventHandler` base is built at `this+0x4c`, subscriptions at `+0x54/+0x58`): the call site that maps them to slots 13/14 is not
   located; the mapping is inferred from names and the slot bodies. Does not block parity (the semantics and times are known).
2. Roles of the open-state `+0x49` byte and of the character callback at `character+0x9c8` slot `0x9c` (probably the `player_on_open_parachute_event` script event, `0x007ee980`). Not blocking.
3. Effect file behind each `ParachuteEffects.bin` index and whether the closing canopy fires `parachute_release` on landing: R4 621ea450.
4. Meaning of `model+0x20/+0x23` bytes written every frame (shading/night factor): R2 28d8e587.
5. The clip metric uses translations; scale-spline values are not printed by `jc2tool anim` (the runtime sampler applies them). The visible bag size therefore also depends on the mesh (R1 15c8aede).
6. Whether canopy collision with scenery exists for `S_PARACHUTE_TO_SO_*` (stunt-object) entries: not checked; the camera phantom at (0, 14.3, -0.4) over the character exists while open/opening (`0x005a60a0`), relevant to G8.
