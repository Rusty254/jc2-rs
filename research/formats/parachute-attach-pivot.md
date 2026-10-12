# Parachute canopy attachment and turn pivot (board task bce93556, gap 3 of `parachute-parity.md`)

Research only. Every address is asserted by `research/tools/parachute_attach_probe.py` (20 checks: function hashes, instruction anchors,
constants, loader field map, skeleton numbers). "native" = asserted by the probe; "inferred" says so. Question: how does the original attach
the canopy root to Rico and about which point does the flight basis turn it (audit gap 3 / G10: the runtime places the entity at the feet and
pivots about the entity origin, both inferred).

## 1. Answer in one page

* **The canopy root is not attached to a bone. The canopy model's world matrix is the character's world matrix, verbatim.** The canopy update
  (`0x006ccd40`, vtable slot 15) ends by pushing `character + 0x15c` (16 floats, the whole 4x4 including the translation row) to the model-matrix
  setter `0x0084cd10` (call at `0x006cceb6`), every frame, after the pose update and the bone attach. No offset is added. The matrix argument the
  update itself receives is only stored at `this + 8` and never reaches the model.
* **The turn pivot is the character origin.** The canopy flight model (the function holding `0x005ab3f2..0x005ab52d`) builds
  `M = Rz(a38) * Rx(a30) * Ry(a34)` and writes it with `0x008812b0(M, 1)` into the character entity (`character + 0xf0`). That call copies the 16
  floats into `entity + 0x6c` and restores the translation row from `entity + 0x9c..0xa4`. `entity + 0x6c = character + 0x15c`, the matrix the
  canopy copies. So rotation keeps the character position fixed: the canopy turns about the character position, not about the harness.
* **The character position is at Rico's feet** (the bind pose and idle clip stand on the Reference bone: idle Hips 0.895 m, `mc01_lod1-rico` "feet at the
  origin"). The canopy skeleton is authored in the same frame: its Reference bone is at the origin, `base` and `HarnessExt` are 1.227 m above it, which is the
  height of Rico's Spine1 at rest (Hips 0.948 + Spine 0.154 + Spine1 0.126 = 1.228). The mesh bounding box (y 1.475..5.836) is above the feet.
* **What follows Rico's body is three bones, set every frame by `0x00657010`** (called from the update, only while the canopy is open or opening):
  `Para_RightHandle` and `Para_LeftHandle` go to Rico's `AttachHandRight` (bone `0x19`) / `AttachHandLeft` (`0x18`) when the hand holds, else to a stowed spot
  next to the canopy bones `rgt_1` / `lft_1`; `HarnessExt` is set to Rico's `Spine1` (bone 3). That is how the straps and brake lines reach the hands and
  the chest. Nothing else in the skeleton is touched by the attach.
* **Consequence for the runtime.** Entity at the feet with the pivot at the entity origin (`CANOPY_PIVOT = ZERO`, `canopy_orientation`) is native-correct for the canopy
  root: no change. What is missing is the three-bone attach (section 3), and the visible body: the same matrix `M` is the character's, so Rico's body turns with the
  canopy about the same point (inferred for the body mesh, section 5; this is the data behind the invented 0.35 rad tilt in `character.rs` and task 9cfa8abf).

## 2. Evidence: root matrix (native)

| Fact | Address | Probe check |
|---|---|---|
| `esi = [this + 4] + 0x15c` (`this + 4` = the owning character), `ecx = [this + 0x5c]` (the model), `call 0x0084cd10` | `0x006cce87`, `0x006cceb2`, `0x006cceb6` | "update 0x006ccd40: esi = ..." |
| order: pose update `0x005653c0`, bone attach `0x00657010`, `0x005419b0`, model matrix | `0x006cce35`, `0x006cce3c`, `0x006cce82`, `0x006cceb6` | "order inside the update" |
| `0x0084cd10`: `rep movsd` of 16 dwords into `model + 4`, then `0x00a464f0` (render-object notify) | `0x0084cd1a..0x0084cd31` | "0x0084cd10 copies 16 dwords" |
| the update's own argument: `lea edi,[ebx+8]`, 16 dwords, never used for the model | `0x006ccebb..0x006ccecb` | "the update's own matrix argument" |
| flight: `lea ebx,[edi+0xf0]` (edi = character), `push 1; push esi; call 0x008812b0` | `0x005ab44c`, `0x005ab52a..0x005ab52d` | "canopy flight" |
| `0x008812b0(matrix, flag)`: flag set -> copy to `this + 0x6c`, translation from `this + 0x9c/0xa0/0xa4` | `0x008812b9..0x008812f2` | "0x008812b0(matrix, flag)" |

The only caller of `0x0084cd10` in the whole image is this one (`0x006cceb6`). A look-alike function at `0x005154f0` that copies a character-sized matrix with a
`+3.0` offset to a `+0xcc` object belongs to the weapon holder (`character + 0x484`, via `0x00743100`), not to the canopy; it is not part of this path.

## 3. Evidence: the bone attach `0x00657010` (native)

Entry guard: return unless vtable slot 9 (is open) or slot 10 (is opening) is true, and return when the foot-machine state hash (`0x007f5240(0)`) equals the global
`0x01196e84` (name unknown). Bone fields (loader `0x007fe910`, probe check "loader"): `this + 0x8c` = `Para_LeftHandle`, `+0x90` = `Para_RightHandle`, `+0x94` = `lft_1`,
`+0x98` = `rgt_1`, `+0x9c` = `HarnessExt`. Pose records are 10 floats: translation (3), quaternion xyzw (4), scale (3); `0x00908940(bone, out)` reads one, `0x008dc380(bone, pose)` writes
one into the model's pose producer. For the character the object is `character + 0x2f8`; for the canopy it is `[[this + 0x5c] + 0x5c]`.

| Canopy bone | Holding hand (flag true) | Not holding |
|---|---|---|
| `Para_RightHandle` (`+0x90`) | pose of character bone `0x19` `AttachHandRight` (read at `0x657201`) | pose of canopy bone `rgt_1` (`+0x98`), translation + (-0.05, -0.4, -0.05), rotation quaternion (0, -0.24254, 0, 0.97014) (-28.07 degrees about Y) |
| `Para_LeftHandle` (`+0x8c`) | pose of character bone `0x18` `AttachHandLeft` (read at `0x6572e0`) | pose of canopy bone `lft_1` (`+0x94`), translation + (+0.05, -0.4, -0.05), rotation quaternion (0, +0.24254, 0, 0.97014) (+28.07 degrees about Y) |
| `HarnessExt` (`+0x9c`) | always: pose of character bone 3 `Spine1` (read at `0x657401`), with the scale (floats 7..9) kept from the canopy bone's own pose | - |

The constants are the floats at `0x00e4abac` (-0.05), `0x00e50cc0` (-0.4), `0x00e3fde0` (+0.05), `0x00e3ac3c` / `0x00e3b2fc` (-/+0.24254), `0x00e4a498` (0.97014), copied into
static initialisers at `0x01195ee0..0x01195f0c` on first use.

**Space (inferred, consistent with every number).** The pose given to the canopy is character model space (root at the feet), shared by both skeletons because both models carry
the same world matrix: Spine1 at rest is 1.228 m high and `HarnessExt` rests at 1.227 m; the stowed right handle is `base + rgt_1 + offset` = (0.166, 1.551, -0.054), 0.08 m above the
shoulder line (1.469) and inside the chest width, which is where the reference shots show the strap points ("two strap points at Rico's shoulders"); `AttachHandRight` read in
local space would be a 6 cm vector and make no sense. The probe asserts these numbers. That `0x008dc380` stores the value as an absolute model-space override (rather than a delta) is
not proven (gap 2).

**Holding flag (resolved by board task 39662382, section 6).** The flag pair is computed from the weapon holder and the foot-machine layers; the full truth table is in section 6. Short form: nothing drawn and nothing pending: both hands hold; a steady drawn one-hand weapon: right hand holds while the upper-body layer is in a calm wielding state, left hand holds unless a reload runs; a steady two-handed weapon (holder state 3): right hand never, left hand unless reloading; a pending weapon change: neither.

## 4. What this means for the runtime

1. Keep the canopy entity at the feet with `CANOPY_PIVOT = ZERO` (`parachute.rs`); the doc comment's "pivot equals the harness" guess is now replaced by this section: native, the character origin at the feet.
2. Add the three-bone attach after the canopy pose is built and before skinning: `Para_RightHandle` := Rico's `AttachHandRight` model-space pose, `Para_LeftHandle` := `AttachHandLeft`, `HarnessExt` :=
   `Spine1` (translation and rotation, canopy scale kept); stowed fallback with the constants above when the hand is not holding (hold flags: section 6; override semantics: section 7). Rico's model-space bone poses come from the character rig animator;
   both skeletons use the same world matrix.
3. The rig turning as one (body and canopy about the feet) is the open body-orientation item 9cfa8abf; this document supplies its pivot.

## 6. Gap 1 resolved: which hand keeps its handle (probe `research/tools/parachute_attach_gaps_probe.py`, 25 checks, native)

Inputs of `0x00657010` (after the entry guards of section 3):

* **Weapon holder** `H = [character + 0x484]` (`0x005cf980`). `H + 0x64` is the draw state (0 holstered, 1 slot 0 right hand, 2 slot 1 right hand, 3 slot 2, 4 slot 0 left plus slot 1 right
  = dual wield, 5 slot 4; see `weapon-ownership-next.md` section 2). `H + 0x70` is the state the player last selected and `H + 0x74` the state to restore. The select routine `0x0056b4b0`
  draws a state with `0x005d6710(state, 1)`, stores `H + 0x70 = state` (`0x0056b54b`), then copies it to `H + 0x74` with `0x0056ba10`; stow-all `0x00666670` zeroes both.
  Predicates: `0x005802c0` = `H+0x64 != 0` (something drawn); `0x00848690` = `+0x64`, `+0x70`, `+0x74` not all equal ("a change is pending": current draw state differs from the selected or restore
  state; `0x0051d680` and the jump/land code `0x0067c740(0)`, `0x0073a670(0)` use it to redraw `H + 0x74`); `0x0073bcf0` = `H+0x64 == 3`.
* **Foot-machine layers**: `0x007f5240(i)` returns the current-state pointer of the machine at `character + 0x48c + i*8`: layer 0 (`+0x48c`, whole body), layer 1 (`+0x494`, upper body,
  `UB_*` states), layer 2 (`+0x49c`, left arm, `LA_*` states). The compared globals, named by their static initialisers (probe check "resolve"): layer 0 against `S_PULL_OPEN_PARACHUTE_VERTICAL`
  (`0x01196e84`, the early return); layer 1 in {`UB_S_IDLE` `0x01196930`, `UB_S_WIELDING_IDLE` `0x01196c74`, `UB_S_WIELD_WAIT_MB` `0x011969b4`, `UB_S_WIELD_WAIT_LA` `0x011969d0`} (call it
  "calm"); layer 2 in {`LA_S_IDLE` `0x01196560`, `LA_S_WIELDING` `0x011968d8`}. `0x00635840` = layer 1 is a reload state {`UB_S_RELOADING` `0x01196ca8`, `UB_S_RELOAD_RH` `0x0119713c`,
  `UB_S_RELOAD_SECONDARY_LOOP` `0x01196b20`, `UB_S_RELOAD_SECONDARY_START` `0x011969c8`, `UB_S_RELOAD_SECONDARY_END` `0x01196f94`}; `0x00618690` = layer 2 is `LA_S_RELOAD` (`0x01196dbc`);
  `0x005d6d80` = either of the two. (Layer meaning is the same one the select routine uses: it posts `UB_S_WIELDING_IDLE` to layer 1 and, for state 4, `LA_S_WIELDING` to layer 2.)

Outputs: `R` = right hand holds (then `Para_RightHandle` := `AttachHandRight`, bone `0x19`; else stowed beside `rgt_1`), `L` = left hand holds (bone `0x18`; else beside `lft_1`).

| Situation | R | L |
|---|---|---|
| nothing drawn (`+0x64 == 0`) and no change pending | yes | yes |
| change pending (`0x00848690`), any draw state | no | no |
| drawn, state not 3 (pistol, dual pistols, state 5), no change pending | layer 1 calm | layer 2 in {`LA_S_IDLE`, `LA_S_WIELDING`} **and** layer 1 not reloading |
| drawn, state 3 (two-handed class: slot 2, stowed on the Harness bone), no change pending | never | not reloading on layer 1 or 2 |

So a pistol drawn and held steady keeps both hands on the handles (reference shot `z29`), a pistol being fired or reloaded releases the right hand (and reloading the left), and a state-3
weapon always leaves the right hand off the handle. Which weapons are state 3 and which UB states a fired pistol uses are asset/state-graph facts not read here. The probe pins the control flow
(`0x657052..0x65710c`, sha256) and checks an 11-row table against a transcription of it; the transcription is a model of the native branches, not an emulation.

## 7. Gap 2 resolved: `0x008dc380` is an absolute model-space override applied before the pose is submitted (native)

* `0x008dc380(bone, pose)` drops bone indices that are negative or beyond the skeleton count, then calls `0x004c59b0`, which does `ecx = [ecx]` and tail-jumps to `0x004b2fe0` with the pose
  cache `ecx + 0x28`.
* `0x004b2fe0` obtains a **writable record** with `0x00b00db0(bone, 1)` and stores the 10 floats into it: translation (3), quaternion xyzw (as one 16-byte store at `+0x10`), scale (3 at `+0x20`).
  The record returned is `[cache + 0x10] + bone * 0x30`: the **model-space** array. The reader `0x00908940 -> 0x004ad230` returns a record from the **same array** (`[cache + 0x10] + bone * 0x30`, refreshing it
  with `0x00afff80` when the bone is flagged stale). So the pose read from Rico's `AttachHandRight` (model space) and written to `Para_RightHandle` is an absolute model-space transform, not a delta.
* `0x00b00db0` also sets flag bit 4 ("overridden") on the bone and, for every descendant whose model-space record is valid, rewrites its **local** record (cache `+4` array, parent-relative) from its
  current model-space pose and flags it for recompute (bit 4 cleared, bit 2 set). The consequence is that the bone's whole subtree (the strap and brake-line bones under a handle) moves rigidly with
  the override and keeps its local offsets.
* **Timing.** Inside the canopy update the order is: pose update `0x5653c0`, bone attach `0x657010` (the overrides), then `0x5419b0` -> `0x008ba220` -> `0x004c6b10`, which flushes the cache
  (`0x00b00150`, `0x00b00380`: locals and model space reconciled, overridden bones kept) and queues the pose for the renderer (`0x004c7270`), then the model matrix `0x84cd10`. So the override is applied
  after animation sampling and before the pose is handed to skinning (the render-side consumer of the queued pose was not read, so "before skinning" rests on the queue being the only hand-off).

Runtime: write the three handle/harness bone transforms as absolute canopy-model-space transforms after the animation pose of the canopy is evaluated, and let the descendants follow (in the runtime's own
skeleton code: re-derive the subtree's model matrices from the unchanged local transforms under the overridden parent).

## 8. Gap 3 partially resolved: what is native about the body's world matrix

Proven: the character entity (`character + 0xf0`) stores its world matrix at `entity + 0x6c` (= `character + 0x15c`, section 2). Its frame hook `0x00909340` calls the **same** pose submission `0x008ba220`
as the canopy (`0x5419b0`), with ecx = the skeleton holder at `character + 0x2f8` and the entity matrix pointer as the argument (the canopy passes `model + 4`, its world matrix, in the same position), and then
copies the matrix to `entity + 0xac` (`character + 0x19c`, the previous-frame matrix; `0x807b59` does the same copy). `0x5ceee0` also hands `character + 0x15c` to the physics body (`0x4a7820`) and the scene
proxy bounds (`0x8c4257 -> 0x875790`). So everything that belongs to the character, body included, is keyed by that one matrix, and the canopy copies it too: they turn together about the character origin.
**Not located:** the instruction that gives the visible body mesh its world matrix. `0x008ba220` ignores its argument and `0x004c7270` queues only the pose, and `0x0084cd10` (the canopy's setter) has a single caller,
so the body uses another path. The statement "the body mesh is drawn with `character + 0x15c`" therefore stays inferred (strongly: weapons use `R * Bone * CharWorld` with the same matrix, `weapon-ownership-next.md`).

## Gaps

1. **Which weapons are holder state 3, and the UB state names a fired pistol uses**: the table of section 6 is exact in terms of states; mapping real situations (firing, aiming) to the "calm" set needs
   `animations/statemachine` data. Blocks nothing visible for the idle case; blocks the exact release time of the right hand when shooting under the canopy.
2. **Does the parachute flow ever leave a change pending** (`0x00848690` true, both hands stowed)? Stow without sync only happens via `0x005c0ba0` (sole caller `0x007aa870`) and the jump/land code redraws `H + 0x74`
   with `0x0067c740(0)`/`0x0073a670(0)`; no stow call was found in the canopy modules. Reference `z29` (pistol drawn, hands on handles) agrees with "never pending". Not proven for every entry path.
3. **Render-side consumers**: (a) the skinning/draw read of the queued pose and the body mesh's world matrix (section 8); (b) the role of the canopy update's own matrix argument (earlier gap 5).
4. **Character origin vs the physics body** (earlier gap 4): not read.

Filed: implementation 0fa77c72 (three-bone attach; sections 6 and 7 are its spec), body lean 9cfa8abf; research 39662382 (this document, closed).
