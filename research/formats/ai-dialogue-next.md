# AI barks and dialogue: data and scheduler handoff

Research only, 8 October 2026. Scope follows `notes/RESEARCH-HANDOFFS.md`.
Read alongside `audio.md`, `missions-next.md`, `crates/jc2-formats/src/soundmap.rs`,
`fsb.rs`, `fev.rs` and `crates/jc2-game/src/audio.rs`. The existing SFX sound map
is a separate format. No Cargo, game execution or code changes were used here.

## Verified retail data

`ai/sound/soundeventmap.bin` (303,111 bytes) is a property file, not a flat binary
sample table. Its root `#ec608194` holds 13 named profile nodes. Each has profile
name `#060d7ea3` and `Sounds` (`#cd2785fc`) children. Each sound record has:

| Property | Meaning established by data/consumer |
| --- | --- |
| `#8e415103` | Numbered AI variant name, e.g. `FirstLosSinceHeatReset1` |
| `Sound` / `#ca6ec803` | Dialogue cue name |
| `Stream` / `#011e0245` | Integer 0 or 1; selects the lookup/bank family in the observed joins |

Profile counts: GovernmentFaction/Elite 400 each; Civilian 276,
Civilian_Elite 285; GovernmentFaction_Female 7; Japanese/Elite 128 each;
Ular/Elite 470 each; Roaches/Elite 470 each; Reaper/Elite 472 each.
Total 4,448 mappings, 1,150 distinct numbered variant names; 687 Stream=0,
3,761 Stream=1. Generic and other-faction cue names occur inside faction
profiles: choosing the sample by an inferred voice-prefix rule is unsafe.

`ai/sound/aisoundevents.bin` (42,659 bytes) supplies 59 event groups and a
separate Parameters node with 10 still unnamed numeric fields. Groups have
`Priority` #3b0f098f, `Delay` #2d94e355, `Probability` #c78edd08,
`GlobalCooldown` #d89e2ef6, `IndividualCooldown` #dedb60cc,
unknown #8c543418 and optional animation-string #a56ffdaa. Each Sounds variant
has numbered ID #8e415103, `Type` #56c7145a, `Cooldown` #781d44ea,
and `Probability` #c78edd08. Preserve spelling, including `Incomming`.

| Group | Global / individual cooldown | Representative variant |
| --- | --- | --- |
| FIRST_LOS_SINCE_HEAT_RESET (#65ba961c) | 8 / 0 | 4 choices; Type=0, cooldown=10, weight=1 |
| ENGAGE_ENEMY (#7087cc82) | 8 / 0 | 28 choices; sample Type=1, cooldown=80, weight=1 |
| GAINED_LOS_IN_COMBAT | 2 / 0 | 17 choices; sample Type=1, cooldown=30 |
| LOST_LOS_IN_COMBAT | 2 / 0 | 13 choices; sample Type=0, cooldown=70 |
| ALLIED_DEAD | 8 / 0; Delay=1.2 | 12 choices; cooldown=120, mixed Type |
| DEATH (#13459c90) | 0 / 0 | 16 choices; Type=0, cooldown=6 |
| INJURED | 0 / 0.35 | HitByBullet variants; cooldown and weights vary |

These groups have Priority=127 and group Probability=1 in the examined records.
Variant weights are not constrained to [0,1]: ENEMY_DOWN includes a weight 30.
Do not treat variant Probability as an independent chance gate.

## A resolved alert sample chain, and its missing link

The asset chain for a GovernmentFaction actor is fully resolved:

1. FIRST_LOS_SINCE_HEAT_RESET group -> `FirstLosSinceHeatReset1`.
2. GovernmentFaction profile maps that variant to `combat_govgrunt1_m_113`, Stream=1.
3. `sound/dialog_mission_specific_lookup.bin` maps that exact cue to
   `event_id=0`, `mode_3d=1`, `sound_id=2455` (zero-based bank index).
4. `sound/fmod/Dialog_Mission Specific_eng.fsb`, sample 2455: file offset
   **70,285,152**, compressed size **6,912**, length **27,648 samples**, inherited
   **32,000 Hz**, mono, mode **0x220**, duration **0.864 seconds**.
5. `dialog_eng` FEV event `Mission Specific/combat_` has programmer waveform
   kind=3. Binding the selected sample to this event is required; a normal FEV
   named-waveform resolver does not supply it. Event volume .562, category
   `dialog/mission_specific` volume .708, 3D range 0..160, max playbacks 2,
   behavior=1, steal priority=10000, event priority=0, fade-out=200 ms.

The follow-up below traces the sensory producer and programmer callback. The
configured dialogue lookup-family binding and complete profile-index assignment
still require evidence before claiming end-to-end original runtime parity.

The language assets are eng/fre/ger/ita/rus/spa. The loose mission-specific English
bank contains 5,301 samples. The archived pc2 bank is spelled
`sound/fmod/dialog_exclaimation_eng.fsb` (161 samples), while the lookup is spelled
`sound/dialog_exclamation_lookup.bin` (161 entries). The mission lookup has 5,301
entries. Lookup records: event_id #72d58956, mode_3d #8d79e6c7,
sound_id #8e51e9e6, cue_name #a6a74f5e. FSB BASICHEADERS inherit names from sample
0; those names must not replace authoritative lookup indices.

Exact-string joins: Stream=0 -> exclamation lookup resolves 640/687 occurrences;
Stream=1 -> mission lookup resolves 3,721/3,761. Thus 87 mappings (28 distinct
cue strings) remain unresolved. Examples include `combat_f2grunt1_m_05_ALT`,
`combat_f2elite1_m05`, and `combat_genfaction1_m_07`. Preserve and report these;
silently correcting spellings or substituting a random cue changes the data.

## Original executable consumer evidence

Addresses refer to this retail `JustCause2.exe` at image base 0x00400000; exported
decompilation is under ignored `research/analysis/pseudo/_unnamed/`. Follow
`research/analysis/functions.csv` and `strings.csv`; do not generalize to another build.

| Address | Verified role |
| --- | --- |
| 0x008da950 | Loads aisoundevents and soundeventmap; 0x34-byte group, 0x10-byte profile records; variant map records are 12 bytes: cue ID, numbered-event ID, Stream |
| 0x007f2de0 | CAISoundEventSystem profile setup; faction/Elite/Female names and faction fallback to GovernmentFaction |
| 0x008d6e00 | Base construction, interns ENGAGE_ENEMY; queue storage strides 0x1c |
| 0x0086fae0 | Base scheduler/update; cooldown guard, group chance, choice, eligibility, recipient resolution and commit |
| 0x0087f540 | Advances cooldowns by dt and accumulates elapsed group time |
| 0x0090ac40 | Weighted variant selection and per-variant cooldown availability |
| 0x008f3740 | Global/group cooldown guard |
| 0x0086a4c0 | Individual cooldown guard |
| 0x008a3020 | Recipient resolution; Type!=0 can resolve a different actor |
| 0x00889c00 | Commits cooldowns and queues output with delay/priority |
| 0x007497b0 | Character/speaker eligibility predicates, not yet semantically named |

Filename pointer references: aisoundevents string at 0x00e3714c is used at
0x007f3183 and 0x008da9bf; soundeventmap string 0x00e58de4 at 0x008db08f.
Hash table begins 0x00f0e188; Sounds at 0x00f0e1a4, Probability 0x00f0e1ac,
Cooldown 0x00f0e1b0, Type 0x00f0e1b4, root map hash 0x00f0e1c0.
Base vtable 0x00f0e990 +0xc loader, +0x10 update; derived vtable 0x00ec5d4c
+0xc profile loader, +0x10 0x006c0070 update, +0x1c speaker eligibility.

Loader normalizes variant weights by their sum. Defaults include variant cooldown
3, Type=-1, group probability=1, priority=0, Delay=0; #8c543418 defaults to Delay.
Variant remaining cooldowns begin at zero. Selection decrements them by accumulated
group time, tries a weighted cumulative draw among available entries, then falls
back to the first available entry if no weighted choice succeeds. It resets the
accumulator. Preserve this fallback if reproducing the original.

The scheduler passes `(Type==0)` into virtual speaker eligibility; nonzero Type
can trigger alternate-recipient resolution. **Type is not proven to mean radio,
3D mode, or interruptibility.** Those labels remain unknown. Group probability
is an actual rand()/32767 gate; weights have separate normalized selection semantics.

Commit sets global/individual cooldowns and the chosen remaining cooldown and
checks speaker validity/output capacity. Its exported delay expression is
`(Delay - unknown_end_delay) * (2*rand()/32767 - 1) + unknown_end_delay`.
The interpretation of #8c543418 and delay interval is unresolved. Cooldown time
units are likely seconds because they are decremented by update dt, but the
upstream clock contract has not been traced. FEV concurrency is independently
verified and must not be replaced by the AI scheduler cooldowns.

## Parser and first runtime milestone (proposal)

```rust
struct BarkProfile { name: String, variants: Vec<BarkMapping> }
struct BarkMapping { variant: String, cue: String, stream: i32 }
struct BarkGroup {
    id: u32, priority: i32, probability: f32,
    global_cooldown: f32, individual_cooldown: f32,
    delay: f32, unknown_8c543418: Option<f32>, variants: Vec<BarkVariant>,
}
struct BarkVariant { id: String, raw_type: i32, cooldown: f32, weight: f32 }
struct DialogueLookup { cue: String, event_id: i32, mode_3d: i32, sample: u32 }
```

Retain unknown Parameters/properties and missing mappings. Separate immutable
decoded tables from per-speaker/group cooldown state. Audio owns playback handles,
programmer sample binding and event concurrency. NPC behavior emits a typed event
only within a reviewed subset of the producer trace below; mission dialogue uses the same
lookup/playback service without going through AI variation. Death/despawn must
clean up actor references and pending requests. Reuse the existing FSB/MPEG/FEV
parsers rather than creating a second SFX path.

Milestone 1: parser plus exhaustive lookup diagnostics and an explicit research
fixture which plays the resolved alert sample through the programmer-event adapter.
This can establish correct bank/index/3D attachment without inventing a detection
trigger. Milestone 2: resolve remaining producer gates, clock units, Type routing,
profile assignment and configured lookup binding; then enable one original alert
rule. Milestone 3: add combat/injury/death with
bounded scheduler queues and shared audio ownership.

Acceptance: 13 profiles/4,448 mappings, 59 groups and exact 87 unresolved occurrence
report; sample2455 range/duration matches headers and valid MPEG decode; deterministic
fake-clock/RNG cases distinguish group gate from weighted selection, exhaustion and
fallback; repeated alerts with two soldiers exercise global/individual/variant gates;
death before delayed playback leaves no dangling speaker. Playback must respect
FEV concurrency and stop/release ownership independently of AI eligibility.

## Reproducible inspection

From the game root, use a private ignored copy (existing in this session):

```powershell
$tool = '.\jc2-rs\research\analysis\mission-access-tools\jc2tool.exe'
& $tool prop . 'ai/sound/soundeventmap.bin' --json
& $tool prop . 'ai/sound/aisoundevents.bin' --json
& $tool prop . 'sound/dialog_mission_specific_lookup.bin' --json
& $tool prop . 'sound/dialog_exclamation_lookup.bin' --json
& $tool fev . dialog_eng --event 'Mission Specific/combat_'
& $tool fev . dialog_eng --event 'Exclaimation/combat_'
& $tool hash GlobalCooldown IndividualCooldown Probability Type Sound
rg -n 'FUN_008da950|FUN_0086fae0|FUN_0090ac40|FUN_00889c00' jc2-rs/research/analysis/pseudo
```

JSON is `{nodes:[...]}`; each node has `children:[[hash,node],...]` and
`values:[[hash,value],...]`. Capture entire subprocess output before filtering;
early shell pipeline termination can give jc2tool a broken pipe. Sample range
reconstruction follows `fsb.rs`: 48-byte FSB4 header, first 80-byte sample header,
then 8-byte BASICHEADERS; data advances by `ceil(size/32)*32`. Raw dumps belong
only in ignored scratch. No audio asset bytes accompany this brief.

## JC2-015 follow-up: sensory producer and programmer callback

Research only, 8 October 2026. The reproducible probe fingerprints the retail EXE
as SHA-256 `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.
Addresses below apply to this build. Static initializers are outside the supplied
function export: raw PE bytes, rather than an invented decompilation, establish
the binding at **0x00c89160**. The initializer starts `mov ecx,0x011fe080`, calls
construction, pushes string pointer **0x00e435fc**
(`FIRST_LOS_SINCE_HEAT_RESET`), calls interning, and stores EAX at
**0x011fe080**. Thus the previously unnamed event queued by 0x00832e60 is this
exact bark; this is stronger evidence than matching its name to an LOS heuristic.

### Detection and heat reset

**0x00560040** processes a sensory callback whose incoming record must have
`+0x10 == 1` and a live weak/shared target. Its branch at `+0x100` includes
visibility/recipient checks whose semantics are not all named. In the accepted
branch it tests **0x006175a0**, a direct getter of the character byte `+0x728`,
"is soldier" or "is enemy" test.

0x00832e60 first requires a nonnull actor resolution (0x00553d40), then a
positive float at `+4` of the faction heat record returned by
0x0074b580(0x00850bc0()). The latter getter reads actor `+0x480`; the heat record
accessor is `this + 0x2c + faction_index*8`. This is a faction heat guard, not a
comparison against the reimplementation's assumed 100 points per heat level.

It obtains blackboard key **LineOfSightSinceHeatReset** (0x00e3ab24), reads its
old bool, and obtains **FactionLineOfSight** (0x00e36cf4). It writes `true` for
FactionLineOfSight with lifetime **1.5**, and `true` for the first-LOS flag with
lifetime **0** through 0x00480e80. That setter stores the supplied lifetime in
the entry, clears its age field, and constructs/updates a bool variant. A zero
lifetime must not be interpreted as immediate expiry without the blackboard
the second flag's exact early-return condition needs machine-level confirmation.

When the *old first-LOS flag is false*, the function proceeds to queue
DAT_011fe080 through 0x0050f870 and **0x00900a20**. The latter only appends if
both input/output capacities and loaded tables permit it; queues stride **0x1c**.
The passed speaker is obtained from the sensory actor via virtual `+0x34` when
squared distance to the selected reference position is **<1600**. At >=1600,
the path uses 0x0057a920 on the separately resolved character instead. Therefore
"the detecting NPC always speaks" is not supported even for a Type=0 variant.
The origin of that reference character/position remains a named trace target.

State at `DAT_01191710+0x84` also selects different events when the old first-LOS
flag was true; those event IDs are deliberately outside this first-bark scope.
The bool is set before scheduler probability/cooldown/capacity acceptance, so
an ineligible bark does **not** imply the next sight callback should retry it.

**0x005790a0** performs a periodic update: it subtracts dt from its timer, resets
it to **0.5**, and examines **11 faction slots**. When a heat record's `+4` float
is <=0 and its prior-positive flag was set, it obtains LineOfSightSinceHeatReset
through **0x007c2a30** and calls blackboard erase operation **0x0046a430**.
0x00810840 selects `this + 0x21c + faction_index*0x10` before the erase, linking
the reset to a faction-indexed blackboard. The export does not establish how
character-local reads share this storage, so do not claim a separate erase for
every character. 0x006c0070 calls this update before scheduler 0x0086fae0 and
delayed-output update 0x007ed9d0. These timers consume the supplied update delta;
the complete upstream clock scaling remains unverified.

### Speaker/profile and lookup routing

Delayed output **0x007ed9d0** reacquires its weak speaker, checks runtime type
CCharacter, and calls **0x0086f2f0(numbered_variant, speaker+0xa84, ...)**.
The resolver subtracts **1** from that profile value, bounds-checks against
the loaded profile count, and searches 12-byte mapping records by their
numbered variant ID. On success it returns the exact cue text and the record's
third word, **Stream**. On failure it returns empty text and Stream=0, without
an arbitrary substitute. This identifies the concrete profile *read*; the
character initialization path that assigns the one-based profile value still
needs tracing. 0x007f2de0's faction/sex/elite profile table alone does not prove
that assignment or fallback for every character class.

0x007ed9d0 applies a separate priority/distance predicate **0x0062f210**, then
sets the dialogue request discriminator to `(Stream == 0)` and sends it through
**0x0081f210**. On the main permitted thread this reaches the character expression
at `+0xb70` via **0x006af200**; another thread queues through 0x007cacd0.
0x006af200 maps discriminator **0 -> dialogue channel 1**, **1 -> channel 2**
in **0x005a3100**. Consequently Stream=1 takes channel 1 and Stream=0 takes
channel 2. These are dialogue admission channels, not a proven direct index
into the exclamation/mission bank tables.

0x0062ffd0 performs channel priority/capacity admission; 0x0075dbc0 constructs
CDialogueControllerImpl (0x00784010) retaining cue and request fields.
0x00719030 constructs a sound request (0x009d8de0), with explicit sample index
initially **-1** and the cue string retained. CSMod_VoiceOver dispatch
**0x00a12630** distinguishes explicit index (0x009db320) from nonempty cue
(0x00a0bbd0); the latter reaches a bank/interface virtual cue lookup through
0x009d8970 (`+0x3c`, versus `+0x34` for explicit index).
This traces the cue-versus-index selector. The exact configured channel ->
VoiceOver descriptor -> lookup file -> FEV event binding remains unresolved;
the earlier exact-string bank joins are data evidence, not executable proof
that all Stream values directly select those two filenames.

### Programmer sample callback and release

The Event::setCallback call in **0x009e98d0** installs **0x00a15970** with the
sound instance as context. Its raw callback selector **6** calls
**0x009fa9e0**. That function obtains a bank Sound via **0x00a07d50**, then calls
FMOD::Sound::getSubSound with the selected signed short at descriptor **+0x60**
and writes the callback's sound output. A cached sub-sound at instance **+0x94**
is reused. This consumer establishes programmer waveform binding by index rather
than FSB inherited header name. The opt-in fixture uses index 2455 from the verified
data join; native configured routing to that descriptor remains unresolved.

Callback selector **7** clears the cached sub-sound pointer; selector **3** calls
0x009c8e20 to detach the event callback and continue cleanup. **0x00a04e80**
detaches Event::setCallback, releases the event through 0x009f8fd0, and for bank
types 2/3 returns the held bank use through **0x009ee1e0**, which decrements its
slot's use byte. 0x00a07d50 distinguishes type 2's rotating streamed slots with
getOpenState checks from type 3's shared first slot. A Bevy adapter can use the
existing decoded bank cache, but must maintain equivalent event/sample ownership;
clearing AI cooldowns is not audio stop/release.

Decompiled imported FMOD signatures often lose the implicit object argument or
shift displayed parameter types. The stable evidence is callback selector,
descriptor offsets, getSubSound use and ownership operations; do not transcribe
the pseudo-C call signatures as working FMOD FFI declarations.

### Bounded follow-up proposal: JC2-032

Implement typed AI groups/profiles/lookup readers, diagnostics, and one explicit
GovernmentFaction first-LOS fixture through a programmer-event adapter. The
fixture fixes profile GovernmentFaction and variant FirstLosSinceHeatReset1 only
for the playback identity check; normal selection retains the four original
weighted choices, global cooldown 8, individual cooldown 0, variant cooldown 10.
Expose a typed producer receipt containing actor generation, original group,
explicit profile and delivery position; do not silently infer profile from a
voice prefix. Enable automatic behavior only for a reviewed subset of the
producer gates. Until the unresolved byte/profile/descriptor assignments are
established, keep this fixture opt-in and label automatic behavior unsupported.

Acceptance criteria for implementation (not tests run in this research task):

- Preserve 13 profiles, 4,448 mappings, 59 groups, raw Type/Stream/unknown fields,
  and the exact unresolved join report; bounds-check index and malformed values.
- Deterministic clock/RNG checks separate first-LOS suppression, reset, group
  probability, normalized variant selection, cooldown exhaustion and original
  fallback. A rejected delivery still consumes the producer's first-LOS flag.
- The explicit original fixture resolves the exact cue, bank/index 2455, and
  programmer FEV event; MPEG decode, duration and moving-speaker position verified.
- Two soldiers and three simultaneous requests exercise FEV max playbacks 2,
  behavior/priority handling independently from scheduler cooldowns. Do not claim
  matching stealing behavior from the raw behavior=1 value without its consumer.
- Actor death/despawn, generation replacement and delayed cancellation release
  requests and audio handles exactly once; stale callbacks cannot restart speech.
- Unsupported profile/lookup or full output queue produces an explicit diagnostic
  and bounded behavior, with no spelling repair or random cue substitution.

Dependencies: reviewed JC2-015 evidence and existing FSB/FEV/audio parsers. This
is a proposed next task, not an implemented audio feature or a claimed full
sensory/FMOD parity milestone.

### Read-only reproduction and validation

```powershell
python -B research/tools/ai_bark_probe.py .. `
  --analysis 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/research/analysis' `
  --tool 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/target/jc2tool-research.exe'
```

The probe reads the EXE/export, checks 13 function contracts and the unexported
static initializer bytes, lists pointer references with containing exported
functions, and optionally inspects original properties/FEV using an existing
private tool. It passed on this install: four alert variants, the exact
GovernmentFaction cue, lookup `(event_id, mode_3d, sample) = (0,1,2455)`, and
FEV programmer waveform/max playbacks 2. It writes no raw dumps/audio assets.
This task did not build Rust, launch the game, play audio, prove callback ordering
in live FMOD, or test implementation acceptance cases. Remaining gaps above are
retained explicitly rather than inferred from event names.
