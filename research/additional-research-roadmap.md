# Additional coding-agent research briefs

Research batch: 8 October 2026. Scope and acceptance requirements come from
[`notes/RESEARCH-HANDOFFS.md`](../notes/RESEARCH-HANDOFFS.md). Three research agents
handled six topics; the coordinating researcher handled saves/checkpoints.
These are initial evidence passes with unresolved semantics explicitly retained.
No Cargo builds or gameplay launches are part of this batch.

| Assignment | Evidence and coding brief |
|---|---|
| AI barks and dialogue | [ai-dialogue-next.md](formats/ai-dialogue-next.md) |
| Original material parameters | [material-parameters-next.md](formats/material-parameters-next.md) |
| Water rendering | [water-next.md](formats/water-next.md) |
| Particles and damage effects | [particles-next.md](formats/particles-next.md) |
| Foot placement and IK | [foot-ik-next.md](formats/foot-ik-next.md) |
| Weapon sockets and recoil | [weapon-sockets-next.md](formats/weapon-sockets-next.md) |
| Save/checkpoint behavior | [save-checkpoints-next.md](formats/save-checkpoints-next.md) |

## Concrete findings and open work

- AI voice profiles join numbered variants to separate dialogue lookup/bank
  families. The companion table supplies group and variant cooldowns, weighted
  choice and recipient routing. One alert asset chain is resolved; the sensory
  producer-to-queue link and full programmer-sound callback remain untraced.
- General material headers now have named flag bits and parameter offsets.
  General, Skinned and CarPaint MPM shaders use different channel math. The
  follow-up verifies CarPaint damage-channel weights and foliage alpha/fade
  uploads; full texture colour-space policy and instance-byte authoring remain open.
- Water texture bindings, four normal-scroll vectors, depth attenuation and the
  planar reflection path have consumer evidence. Shoreline wave acceptance and
  several underwater bindings still need tracing.
- Original particle lifetimes/rates and modifier classes have verified consumers.
  Atlas layout, time-window units and simulation/attachment space remain open.
- Authored foot/hand weights and separate two-joint/ground-placement controllers
  are identified. Ground lock windows, pelvis tuning and moving-support behavior
  are not yet recovered.
- Weapon settings provide raw per-weapon vectors, two authored transforms and
  rotation conversion rules. Representative pistol/rifle/launcher values are
  recorded; exact grip/muzzle/stow roles and two upgrade fields remain unproven.
- Retail save directory/field layout is decoded, including named state sections
  and parts/money/chaos field order. Exact checkpoint rollback rules and full
  metadata/version semantics remain open.

## Integration boundaries

- Material flags and channel meanings are block-specific. Coordinate changes to
  formats, shader bindings and main/depth/shadow paths with the existing graphics
  brief. Water rendering must leave physical sea level and boat buoyancy independent.
- Foot/hand constraints and weapon corrections share rig ownership. Coordinate
  their application order with animation layers and the existing event/graph decoder;
  footstep events alone do not establish contact lock windows.
- Effect emitters should own visuals and cleanup independently of gameplay damage,
  impulses and destruction rewards. Original AI voice events should reuse the
  existing sound-bank/event player while preserving speaker and scheduling identity.
- Persistence depends on stable world-object identities and mission resource/event
  lifecycle. Coordinate with the [mission/world roadmap](mission-world-roadmap.md)
  before implementing retries or restoring mission-owned entities.

The read-only [save_probe.py](tools/save_probe.py) helper validates observed container
and field boundaries. Retail-save writing and complete import compatibility remain
unproven. Raw assets and private save copies stay in ignored scratch storage.

## Dispatch ledger

The seven independent assignments above have completed an initial research pass;
unresolved meanings remain in each brief. On each subsequent agent completion,
the coordinator rereads `notes/RESEARCH-HANDOFFS.md` and the live Relay board,
reviews new requests and promotes authorized research to ready before dispatch.
An unknown in a finished brief alone does not restart its inventory. Ownership,
progress and review state live in Relay; this ledger is a research index only.

Current targeted follow-ups from the handoff:

| Request | Owner | State / output |
|---|---|---|
| Original PDA Lua/action routing (JC2-004) | mission_access | Reviewed; [pda-actions-next.md](formats/pda-actions-next.md) |
| Original vehicle engine selector (JC2-005) | world_vehicles | Reviewed; [vehicle-engine-next.md](formats/vehicle-engine-next.md) |
| AI-team precedence and cutscene event semantics (JC2-006) | larger_missions | Reviewed; [mission-consumers-next.md](formats/mission-consumers-next.md) |
| Ragdoll activation and recovery (JC2-007) | larger_missions | Reviewed; [ragdoll-next.md](formats/ragdoll-next.md#original-activationrecovery-consumer-follow-up--jc2-007) |
| Animation transition/root-motion consumers (JC2-008) | codex/animation-research | Reviewed; [animation-next.md](formats/animation-next.md) and `../scripts/research-animation-consumers.ps1` |
| Mission resource readiness/cancellation (JC2-019) | mission_access | Reviewed; [mission-resources-next.md](formats/mission-resources-next.md) |
| Foliage alpha/fade and material channel consumers (JC2-018) | codex/material-research | Reviewed; [material-parameters-next.md](formats/material-parameters-next.md) and [material_contract_probe.py](tools/material_contract_probe.py) |

Last handoff and board check: 8 October 2026, after resuming the interrupted
JC2-008/018 reservations. Assignments 8 and 9 and mission resource-readiness now
have reviewed evidence checkpoints with remaining gaps. Existing
coding tasks retain their separate owners; research completion does not complete
their implementation milestones. See `../AGENTS.md` and `../TASKS.md` for API
claim, versioning and coordinator review rules.
