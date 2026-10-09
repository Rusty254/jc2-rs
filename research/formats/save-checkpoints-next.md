# Save containers, checkpoints and persistence

Research: 8 October 2026, following `notes/RESEARCH-HANDOFFS.md` assignment 7.
Primary evidence: original executable/decompiler export, local retail PC saves,
and current Rust source. Save examination continued from private copies under
ignored `target/save-research/` after the handoff requirement was supplied. No
live save writes, game launches or Cargo builds. Raw save bytes are not included
in this brief or the reusable helper.

## Confirmed format facts

Retail files use `JustCause2Save%.2d.sav`; the save manager's state switch at
**0x00870670** names numbered slots, slot zero, and a separate `JC2Settings` file.
Do not infer all state-switch labels or slot-zero semantics from names alone.
Five local full saves have length **891904 = 0xd9c00** bytes. One additional
one-byte file is rejected as truncated, not treated as a valid empty save.
The local `JC2Settings` inventory contains a 72192-byte file; its fields and
profile schema were not decoded in this pass.

`CSaveManager` construction at **0x00885bc0** creates a `CSaveContainer` through
**0x0087b3f0(0x39800, 0xa0000)**. The constructor arguments identify separate
fixed index and payload allocations. Slot I/O at **0x0087b290** writes metadata
and a **0xd9a00-byte** body. **0x00894c00** rounds metadata up to the next 512-byte
block (even for an already aligned input). The five samples have the section
directory at **file offset 0x200**, and payload at **0x39a00**. An additional
512 bytes remain beyond the decoded fixed allocations; their role is unresolved.
Metadata's first u32 is 2 in the inspected slot-zero sample; it is not yet a
verified version field. No magic or compression wrapper is needed to read the
observed section directory and fields.

```text
metadata / alignment area: observed 0x200 bytes, not a universal constant
index allocation: 0x39800 bytes
  u32 section_count
  section_count * { u32 key, u32 instance, u32 payload_offset, u32 payload_length }
payload allocation: 0xa0000 bytes
  each indexed section: repeated { u16 field_id, u32 byte_length, bytes value }
remaining body padding/overhead: not semantically decoded
```

Offsets in directory entries are relative to the payload allocation. Sections
are identified by **both key and instance**; samples contain many nonzero instance
keys, and repeated class keys with different instances. Never collapse them into
a map keyed only by the first hash.

Original routines corroborating the schema:

| Address | Observed operation |
|---|---|
| 0x0089a4c0 | Begin/select write section by `(key, instance)` |
| 0x008b1250 | Finalize section length and register its 16-byte directory record |
| 0x0088ff20 | Append `u16 id`, `u32 length`, then raw field bytes; update mode has a separate branch |
| 0x008f95e0 | Find/read section using both identifiers |
| 0x00893fd0 | Find field and return id, length and payload offset |
| 0x008dfe10 | Load fixed directory, iterate 16-byte records, then copy payload |
| 0x008b4e60 | Slot loading, metadata/version-validation virtual call, invalid-version error |

These are format readers/writers; unresolved virtual validation calls do not
establish checksums, encryption, supported versions or corruption-repair rules.

## Confirmed section identities and partial state schema

Names below are verified by Jenkins `hashlittle` matches against executable
strings and the save directory, rather than guessed labels.

| Key | Name |
|---|---|
| 24543a6f | SaveBitStorage |
| 83a89d0d | MissionSystem |
| e9bb5fd1 | CPlayer |
| df107f97 | BlackmarketHook |
| b0f23d91 | ResourceItemHandler |
| 8315344b | DripfeedManager |
| 753ec2e1 | HeatSystem |
| 419aabd2 | HeatLevels |
| 269ab061 | CHeatSpawnManager |
| 2bba6b01 | CSpawnManager |
| 5b5fba2f | CSheldonService |
| 89215d85 | Weather |
| 07763c5a | WorldTime |

**SaveBitStorage:** write routine **0x008f5660**, read routine **0x008625f0**.
Field 0 is a four-byte slot count; field 1 is a four-byte scalar from object +0xc
(semantic name unknown). Field 9 holds `count * 8` bytes of records; field 10 holds
`(count >> 3) + 1` bytes of bits. All five samples contain **23833 slots**, field 9
length **190664**, field 10 length **2980**. Loader initializes **190664** record
bytes to `0xff` and **2979** bit-buffer bytes to zero before applying saved fields.
Initialization of the additional saved bit byte is not established. Mapping slots to world objects and
the exact meaning of each eight-byte record/bit remain unresolved. This structure
is evidence of persistent object storage, not proof that each bit means destroyed.

**MissionSystem:** **0x005357d0** writes, **0x0063e980** reads. Field 0 holds the
current mission object's +0xdc identifier or zero; the reader resolves it through
`0x00588fc0`. Field 1 is the mission-table count. Fields 2–16 include scalar/boolean
manager state with unresolved names. Starting at field 100, each mission writes
its identifier (four bytes), a bit-packed status byte, and objective count (four
bytes), followed by each objective identifier/status pair (four/one bytes).
The bits originate from mission offsets +0x99, +0xe1, +0xe2, +0x94, +0xe3 and a
virtual query; objective bits originate from +0x20/+0x21. Exact active/completed/
unlocked labels require tracing setters. This stores more than completed ids.

**CPlayer:** **0x00695ae0** writes a character scalar (field 0), a 64-byte transform
(field 1), two other scalars, weapon-related slot values around fields 30–34,
nineteen values from a weapon-manager query at fields 61–79, four values at 90–93,
additional selected/state fields and a vehicle-related identifier/transform at
107/108. A slot is written only when occupied; slot index 3 is explicitly excluded
in that loop. Equipment/transform persistence is supported by these consumers;
exact weapon ids, ammo pool vs magazine meanings, health semantics and load-time
fallbacks are still unknown. Do not assign the nineteen values to ammo solely
because their query lives near weapon code.

Follow-up consumer **0x005e96d0(type)** sums a per-type pool at +0x78/type*4
with contributions from matching equipped weapons through a virtual +0x20 query;
it returns zero without a matching weapon and has a special 100000 branch. Thus
those saved values are aggregate type queries, not plainly independent magazine
counts. Confirm the query's ammo semantics and restore/distribution path before
splitting them into reserve/magazine state in a retail importer.

**BlackmarketHook:** **0x007bd5e0** writes five leading four-byte values from
object +8 onward, counts, then item identifiers and byte flags. Getter
**0x0062e470(index)** reads that same +8/indexed array. Registered Lua wrappers
`GetWeaponparts` (0x00a33670), `GetArmorparts` (0x00a337d0), `GetVehicleparts`
(0x00a33720), `GetMoney` (0x00a335c0), `GetChaos` (0x00a33510) call it with
indices **0, 1, 2, 3, 4**, respectively. Thus the matching leading save-field
order is weapon parts, armor parts, vehicle parts, money, chaos. Item upgrade
flags still need their reader/getters traced. **ResourceItemHandler**, **0x00688280**,
writes a count followed by eight-byte map records. Settlement-like counters also
appear in section **56e9fcec**, written at **0x00607510** and read at **0x00621690**:
a count and repeated identifier/counter/flag records. Their full semantic identity
is a lead, not a recovered settlement-state specification.

## Checkpoint lifecycle: confirmed hooks, unresolved rollback rules

Lua `SaveGame` is registered to **0x00a31820**, which delegates to **0x0064b020**.
That path queues a `CSaveMetric` (constructor **0x008060d0**); its printable XML at
**0x00594100** includes player position and optional Mission/Checkpoint strings.
This XML is a metric/debug representation, **not the binary disk save format**.

Lua `LoadCheckpoint` maps to **0x00a2c4f0**, calling **0x006aaa40** and requesting
state `0xb`; **0x008585d0** contains the checkpoint-loading path. `CheckpointExists`
at **0x00a2c430** uses **0x007a32d0**, which checks two runtime bytes, rather than
simply checking for a numbered `.sav` file. Mission-manager construction in
`text_005d0000.c` allocates a separate **0x8c00/0x8c00** container and registers
`msy.trigger_autosave`, mission abort/fail, player death and restart events.
This supports separate snapshot/lifecycle machinery, but allocation alone does
not prove which state the checkpoint container retains.

**0x0062ab10** emits `autosave.loaded` when its alternate event string is absent,
otherwise emits that string. Giver/global mission scripts consume `autosave.loaded`
(see mission-access brief). Restoring serialized values without the corresponding
mission/event/resource lifecycle will therefore be incomplete.

Unknown and necessary follow-ups: exact checkpoint capture timing; which money,
chaos, pickups, equipment and world changes survive a failed objective; mission
entity reconstruction vs serialization; retry vs abort behavior; dead/destroyed
object GUID mapping; profile/metadata/version validation and checksum policy.
No controlled before/after gameplay captures were performed, so no rollback rules
are claimed from comparing saves made under unknown conditions.

## Proposed jc2-rs schema and ownership (design, not retail facts)

Separate a versioned campaign state from an active mission snapshot. Campaign state
should carry stable asset/object ids, completion/progression, money/parts/upgrades,
equipment/ammo and world completion. Mission state needs explicit objective,
checkpoint, spawned-agent/resource ownership, local events, timers and counters.
Define merge/rollback policies only after the retail lifecycle study above.
Serialize identifiers and values, never ECS Entity ids, collider handles or GPU data.
Rebuild derived totals/aliases, physics and resource bindings on restore.

Current Rust resources (`chaos.rs::Chaos/Wallet/Progress`,
`missions.rs::MissionsDone/Strongholds/ActiveMission`, combat loadout and upgrades)
hold in-memory state; no game-save writer/reader was found. Following JC2-003,
`MissionRun::start` sets active state; normal launch resource readiness is gated
by a separate generation receipt. Emitted checkpoint events do not implement
snapshots. Coordinate restore with that launch/resource cleanup contract.

First coding milestone: a bounded **read-only retail container parser** preserving
unknown sections/fields and composite identifiers, plus typed SaveBitStorage and
partial MissionSystem records. The separate native campaign proposal is now
documented in `persistence-design.md` (JC2-012); it is not an implemented writer.
Retail-save writing/import compatibility is not established by this research.

Acceptance: malformed header/length/offset rejection; roundtrip unknown bytes and
composite keys in isolated fixtures; version migration with backup and atomic replace;
load without double rewards; settlement and pickup persistence across tile reload;
equipment/ammo restoration; objective retry reconstruction and cleanup; resource
readiness before resumed events. Determine expected retry outcomes with controlled
pre-checkpoint/post-checkpoint/death/retry/abort/save-reload comparisons first.

## Reusable diagnostic and verification

`research/tools/save_probe.py` is read-only and defaults metadata to the observed
512 bytes (override with `--metadata-bytes`). It emits filenames, fingerprints,
section/field boundaries and SaveBitStorage length checks, without raw save payloads.

```powershell
python research/tools/save_probe.py target/save-research/JustCause2Save00.sav
target/jc2tool-research.exe hash SaveBitStorage
```

Five private full saves passed every section/field boundary check: respectively
37/59/125/43/30 sections and 5525/15822/5127/16011/11844 fields. All had matching
SaveBitStorage lengths. The one-byte sample was rejected. This validates the helper
on these fixtures. Private scratch mutations of directory count, section offset
and field length were also rejected; those temporary files were removed. These
checks do not establish retail version validation, full state semantics or gameplay.
