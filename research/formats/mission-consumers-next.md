# Mission consumers: team precedence and cutscene phases

Research date: 2026-10-08. Follow-up to `missions-next.md`, restricted to original
consumers rather than another mission inventory. Relay task JC2-006 was atomically
claimed by `larger_missions`. No Cargo, game launch, runtime edits or commits.
Evidence is the local retail executable, shipped property containers and Ghidra
exports in `research/analysis/pseudo/_unnamed/`. Addresses below are executable VAs.
“Verified” means visible in those consumers/data; it does not mean runtime-tested.

## Team precedence recovered

**Verified:** `FUN_00786280` (`text_00780000.c`, size 0x1372) is a character/AI-agent
creation consumer. It creates a CCharacter, CPlanningAgent, CAgentControl and
CAIAgent, and accepts an optional sixth argument with the 0x11c agent-record layout.
`FUN_0050a080` (`text_00500000.c`, size 0x860) reads template AgentTypeIndex, scans
records in strides of 0x47 dwords for a matching first dword, and passes the matched
record, or null, as this sixth argument. This is index matching, not a demonstrated
role-name merge. Its alternate spawn path uses a freshly initialized record.

The consumer reads template properties, except where record flags select a value:

| Value | Template key/hash | Record selection in 00786280 | Exact fallback |
| --- | --- | --- | --- |
| Agent index | AgentTypeIndex / #43e242e1 | Used to find the record in 0050a080 | Missing match passes null |
| Agent name | Name / #3a4ca003 | Any nonnull record: string at +0x44 | Null record: template Name |
| Character resource string | Template baseline prepared by 0060e580 | +8 bit 0x20: string +0x60 replaces baseline | Unset bit retains prepared baseline; details of 0060e580 remain untraced |
| Health | Health / #c0887dae | +8 bit 0x40: float +0x10 | Template Health, seeded with 100 if getter does not write |
| Six float parameters | #69838e32, #ac28cc0d, #7b3e8022, #7909ab0c, #89bd8846, #ffd380fb | +8 bit 0x80: floats +0x14 through +0x28 | Six template reads, each seeded with zero; semantic names unknown |
| Weapon | Weapon / #722c96de | +8 bit 0x02: integer +0x7c | Template weapon string passed through 004ffea0 |

These are actual branches, not a proposed precedence policy. A record can contain
a health/weapon value without selecting it. A present record also changes several
other boolean features directly; therefore a generic dictionary overlay is unsafe.
The authored property that controls each override bit is **unknown**: the
CTeamComponent loader export loses many getter arguments. In particular, do not
claim that STeamAgentInfo #1df63231 by itself enables bit 0x40.

**Verified:** `FUN_00661440` (size 0x17f, `text_00660000.c`) initializes agent records:
+0x10 = 100.0, +0x3c/+0x40 = 1.0, many numeric fields zero, strings constructed.
`FUN_005f84a0` (size 0x189d, `text_005f0000.c`) initializes each STeamAgentInfo this
way, then reads authored data. Numerous boolean reads modify bits only if the
getter succeeds. `FUN_004a1f60` (size 0x79) writes its output only for a matching
float property; missing or wrong-typed input preserves the seeded value. These
facts justify preserving presence and type instead of coercing absence to zero.

**Verified separate fallback:** in 005f84a0, a missing/out-of-range team integer
(valid 0..10) falls back to `*(component[2] + 0x8c)`. Naming that integer faction or
calling this a general template merge would be speculation. Resource lookups
0069aa20/00549330 and 00523240 assign shared resource references; 00523240 is a
shared-pointer assignment routine, not a property-overlay operation.

**Verified data fixture:** `global\aiteams.blz/bofcx1.aiteam` has one team agent
whose #21587072 subtree contains AgentTypeIndex = 0 and Weapon = "Beretta Handgun".
`km01.blz` includes bofcx1/2/3 and several mission-specific .aiteam members. The
km01 script contains STeamAgentInfo #1df63231 values 30, 40 and 100. This gives
concrete conflicting authored/template candidates; the link from every script
override property to the bits above still requires instruction-level recovery.

## Cutscene completion and skip recovered

**Verified loader:** CCutsceneObject vtable 0x00f0aa7c slot +0x18 points to
00915450 (size 0x419). It constructs, in order, CCutsceneStateLoading (008bffb0),
CCutsceneStatePlay (008c1a30), CCutsceneStateUnloading (0088bd80). Their primary
vtables are 0x00f0b7f0, 0x00f0b5d4 and 0x00f0b814 respectively.
009152d0 updates the current state through +0x0c; when it returns false the state
exit +0x10 runs, index advances, and the next state enters through +0x08. Only when
the final state exits does it call owner slot +0xc0 = 0088f230.

| Authored output | Owner offset loaded by 00915450 | Verified emission phase |
| --- | --- | --- |
| #fa3d1836 | +0x54 | Output registration verified; emitter not recovered here |
| #0a19cd4e | +0x58 | Playback exit: 0089d380 calls 0086c870; that routine emits this registered output, then `remove_spawnset_block` |
| #9ded931f | +0x5c | 0088f230, called after the final state exits; emits registered output, sets +0x39 bit 1, clears active +0x38 bit 4 and calls 009024e0 |

`004e3790` emits each member of an output-event list. These are distinct phases:
the output at +0x58 can run before unloading has completed. The former inventory's
“completion-looking” #9ded931f is now backed by a terminal consumer; it must not be
emitted simply because the sequence clock reaches its end. 008d7ed0, reached from
unloading through owner +0xc4, unloads the associated location/resource, clears the
sequence shared pointer at +0xf8/+0xfc and resets flags. Other branches can extend
unloading, including a three-update delay and a fade gate.

**Verified skip path:** playback update 00900010 registers `cutscene.skip` only
when its event handle is empty, global counter +0x60 is below 1 and owner +0x39
bit 0x40 is clear. It then emits `gui.cutscene.allowskip`. Loader key #e51fe639
sets this owner bit; it is a skip-disabling gate in this consumer.
The Play secondary vtable 0x00e5ae38 has callback 00879960 at +4. Its adjusted
receiver uses `004e5be0` to match the registered event, sets Play +0x10 to 1, and
calls 008b32c0. Update then returns false once `008e03b0` returns zero. This function
queries a camera-manager flag/float; interpreting it as a fade gate is **inferred**
from neighboring 008b32c0/0087aca0 use, not a fully decoded camera API.

Playback exit 0089d380 runs the output above, stops the sequence via 008fb3e0,
clears the playback flag, resets the camera transition and unregisters the skip
handle. 008fb3e0 clears sequence +0x86/+0x87 and guards teardown with +0x85.
Normal playback also exits when sequence +0x86 becomes zero. Thus an accepted skip
and normal end traverse the same state-exit/unloading/final-output chain.
**Unknown:** sequence key/event catch-up when skipped, seek-to-end behavior,
external stop/cancel routing, failure during resource loading, replay reset and
whether every finalization path is deduplicated. No claim of equal gameplay state
after skip is warranted until the skipped authored sequence keys are traced.

## Typed implementation contract and first milestone

The following is proposed Rust-facing structure, not a recovered ABI:

```rust
struct AgentOverride {
    agent_type_index: i32,
    raw_flags: [u8; 3],
    name: String,
    character: String,
    health: f32,
    weapon_id: i32,
    parameters: [f32; 6],
    unknown: Vec<TypedProperty>,
}
fn resolve_health(template: &Template, record: Option<&AgentOverride>) -> f32 {
    match record {
        Some(r) if r.raw_flags[0] & 0x40 != 0 => r.health,
        _ => template.health.unwrap_or(100.0),
    }
}
enum CutscenePhase { Loading, Playing, Unloading, Finished }
struct CutsceneRun {
    phase: CutscenePhase,
    skip_requested: bool,
    playback_exit_sent: bool,
    final_output_sent: bool,
    outputs: RawHashEventMap,
    resources: OwnedResources,
}
```

First milestone: implement a pure inspected-record resolver and a three-phase
cutscene lifecycle fixture. Preserve unsupported properties and require proof of
override-bit mapping before feeding real mission records to the resolver. Preserve
raw output hashes, emitting #0a19cd4e at playback exit and #9ded931f after resource
release. Cancellation should have its own explicit proposed policy until its
original consumer is traced. Keep world mission reward/completion ownership in the
mission runner, downstream from authored events.

Fixtures/checks for the coding agent (not executed in this research):

* Health default 100, template 75, record 30: clear override bit resolves 75; set
  bit resolves 30. Include wrong-typed/missing template Health and explicit zero.
* Matching AgentTypeIndex selects one override; unmatched index passes no record;
  duplicate index ordering and invalid index behavior need separate original trace.
* Weapon template Beretta versus override ID: verify flag clear/set and missing
  weapon resolution independently; do not replace invalid IDs with a guessed gun.
* `km01_micro_01.seq.blz/km01_micro_01.seq`: normal end and accepted skip must both
  log playback-exit output before resource-release and terminal output. Repeat skip
  during unload must not duplicate proposed runtime outputs. Check skip-disabled
  bit, asynchronous load, unload delay, missing sequence and cancellation separately.
* Audit authored event keys between current time and sequence end before asserting
  mission-state parity for skip. This is the next original sequence-consumer trace.

## Reproduction and remaining trace targets

Run from the game directory using the privately copied prebuilt tool (no build):

```powershell
$tool = 'jc2-rs/_extract/particles-ik-research/jc2tool.exe'
& $tool sarc . km01.blz
& $tool prop . 'global\aiteams.blz/bofcx1.aiteam'
& $tool prop . km01.blz/km01.bl.bl
& $tool hash Health Name Weapon AgentTypeIndex
```

PE vtable/hash words were checked directly with Python standard-library `struct`
after translating VA through the PE sections (image base from optional header).
No disassembler installation was needed. Next precise targets: recover the omitted
key arguments in 005f84a0 around STeamAgentInfo boolean reads; trace 0060e580's
character-resource baseline, 005ca2e0's final weapon application, and sequence
00879be0's key dispatch/008fb3e0's teardown callbacks. The #fa3d1836 emitter remains
unresolved. The resolver branches and two event phases above are sufficient for a
bounded fixture, not for declaring km01 fully playable.
