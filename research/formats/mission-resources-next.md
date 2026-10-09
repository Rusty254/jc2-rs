# Mission resource readiness and cancellation

Research date: 8 October 2026. JC2-019; read-only retail executable/decompiler
inspection, extending `mission-access-next.md` and `missions-next.md`. No Cargo,
game launch, runtime changes or extracted assets. Addresses below are virtual
addresses in the installed `JustCause2.exe` (image base `0x00400000`), not Rust
symbols. Existing local decompiler exports are primary evidence, with direct PE
reads for vtable slots and suffix strings. Decompiler signatures/implicit ECX
arguments are imperfect; symbolic names assigned below describe observed use.

## Result and confidence

**Verified:** `RESOURCE_LOADED` is a bound mission boolean, not a literal event
emitted by the world. It becomes true in the mission's named location-completion
handler. The world emits that completion only when a `CLocation` reports state 4,
after its object initialization and registration/finalization stages. Reading the
mission script archive alone does not meet this boundary.

**Verified:** the world distinguishes accepted load/unload requests, queued work,
live location references and an active unload record. Removal can detach records
from these structures. **Unknown:** complete worker cancellation, a request
generation token, and mission-side stale-completion suppression. The inspected
abort path unloads only an already-loaded location; its pending-load branch does
not issue the equivalent cancellation request. This is a concrete trace limit,
not proof that every aborted retail mission can subsequently reactivate.

## Mission-facing contract

| Consumer VA | Evidence |
| --- | --- |
| `0x0056db60` | Registers `RESOURCE_LOADED` via `0x008a2ec0`/`0x008d7b20` as a boolean at `CMission+0x95`; other flags include LOCKED+94, ACTIVE+e1 and COMPLETED+e2. |
| `0x0059ea30` | Faction task constructor initializes resource bytes+97/+98 and builds `missions/FactionTasks/{id}/{id}.location`. |
| `0x00699190` | Formats `{id}.location.finishedLoading`, storing the event identity at mission+f8. |
| `0x0053ea80` | Activation requests world virtual+38 when requires-resource+97 is set and loaded+95 is clear. Otherwise it emits activation immediately. |
| `0x0052dc40` | Faction completion handler first calls base `0x005af620`, matches+f8, conditionally emits activation/starts processing, then sets+95=1. Its local matched-event branch contains no ACTIVE+e1 or generation check. |
| `0x0071dac0` | Key-mission named resource completion consumer. |
| `0x00796370` / `0x00785f20` | Challenge completion consumer / initialization of corresponding event identity at+fc. |
| `0x00560300` | Abort clears ACTIVE+e1. Only if +97 and +95 are both true does it schedule a delayed unloader and clear+95. The shown branch does not cancel a not-yet-loaded request. |

Do not transfer faction-handler offsets to all mission subclasses. Base event
dispatch, subscription lifetime and the challenge/key special cases remain
separate prerequisites for a claim that stale completion is suppressed globally.

## Which location/resource work is awaited

`CGameWorld` primary vtable `0x00eb9224` has these directly verified entries:
load+38=`0x005ae930` -> `0x005bdd00`; normal unload+44=`0x0070d060` ->
`0x005fd0f0`; force unload+48=`0x0081e670` -> `0x0062fc60`.
Mission forced-unload caller: `0x00525320`.

`0x005bdd00(name, 1)` is request acceptance. An existing record without an
instance gets pending-load bit 0 set and pending-unload bit 1 cleared at record+14.
An existing instance can cause immediate successful return; this return value is
not a new completion event. A new record gets name, bounds, mode bits at+10,
pending bits at+14 and instance pointer at+24.

`0x00830560` creates `CLocation` via `0x006eb550`/`0x0073b800` and transfers bounds.
The mission mode 1 uses the supplied path with `0x0073f530`; mode 0 prepends
`Global/`; other modes use `Locations/` and `0x005f4780`. `0x0064ec00` normalizes
the name, strips `.location` when present and appends the selected suffix:

| String VA | Bytes | Wrapper |
| --- | --- | --- |
| `0x00e314fc` | `.bl` | `0x0073f530` (mission path) |
| `0x00e306b8` | `.nl` | `0x005f4780` when location+c0 set |
| `0x00e2fbb8` | `.fl` | `0x005f4780` when location+c0 clear |

`0x008c4ab0` constructs `CResourceLoader` (`0x008ff250`) at location+10 and starts
`0x0088b880`. The latter appends `z`, opens an asserted
`NLibrary::CAsyncFile`, obtains the length and enqueues `CStreamQueueItem` chunks
named `CHUNK_%d`. Mission backing input therefore resolves through the `.blz`
path, rather than treating `.location` as a distinct eagerly loaded resource.
Successful start sets location state+0c=1. Full nested-resource residency and
worker errors are not established merely by this successful start.

`CLocation` primary vtable `0x00eb5804`: update+10=`0x005fd470` -> `0x008c6bd0`;
unload step+14=`0x008879c0`; finalization+1c=`0x00554110`;
alternate resource branch+20=`0x00793f90`. The observed states are:

| State at+0c | Consumer and observed boundary |
| --- | --- |
| 0 | Constructor `0x008b50a0` and teardown `0x0058f3c0`: idle/reset. |
| 1 | `0x0089a870`: advances streaming/resource work. With resource+48 present, waits for object+40 virtual+2c success and resource+48's+2c to reach zero; services it through `0x008b3ed0`. Also waits for outstanding+28 bookkeeping to clear; absent-resource paths use stream-queue gates or alternate finalization. |
| 2 | `0x008c6620`: advances over shared objects in `CGameObjectContainer` at+14, calling each object virtual+c under a time budget, then sets state3. |
| 3 | `0x00554110`: finalizes resource/cache work. A gated branch sets flags on `CModelBatchResource` entries and resource+50, then can wait until resource+50's counter+2c >= counter+c. Calls `0x0086d0b0` when ready. |
| 4 | `0x0086d0b0`: reached after each object's manager registration, world virtual+cc and object virtual+10; then loader/buffer finalization. This is the world completion condition. |
| 5 | `0x008879c0`: removes container objects incrementally within budget; once empty sets state0 and calls teardown. It is ongoing unload, not unload complete. |

The state labels are inferred descriptive names; numeric comparisons/calls are
verified. Exact ownership/types of state1's +40/+48 objects, counter meanings,
global mode `0x011958c4` and cache predicate `0x008ed120` remain unresolved.
This trace does not assert that all possible physics, texture, audio or external
mission-team dependencies are resident at state4, or that every path visits all
intermediate states. In particular, state1 can call finalization directly.

`0x00520070` updates the head of world's wait queue (+54 begin/+58 end) through
the location virtual+10. When state4 is observed, it clears record+14 bit0,
performs mode-specific registration (`0x0076bf20`), conditionally pops the queue
only if its current front is the same record, and emits
`__LocationFinishedLoading` followed by a dynamic location-name event. The
mission's independently constructed `.finishedLoading` identity is the matched
consumer. The omitted `std::operator+` suffix argument in this decompiler export
has not been revalidated from the instruction stream in this follow-up.

## Cancellation and ownership boundaries

| VA | What removal actually establishes |
| --- | --- |
| `0x005fd0f0` | Normal name unload removes matching waiting entries through `0x00551700`; live records clear load bit and set unload bit iff an instance exists. A match against current unload record+60 records success, without directly calling the removal helper in that branch. |
| `0x0062fc60` | Force unload additionally removes a matching current+60 and matching live records. Table entries are removed when instance absent or force argument set. |
| `0x00551700` | Clears pending bits0/1; removes exact record pointer from wait queue with memmove; clears matching current+60; removes a live-vector entry, decrements instance refcount/destructs at zero, and clears instance+24. Mode-dependent name-table erasure is also present. |
| `0x00520070` | Active unload updates the instance and waits while state5. Once instance absent or state !=5, it clears record/current pointers and pending-unload bit, removes the live entry, and releases its reference. |
| `0x006285b0` / `0x006103a0` | Creates `CDelayedResourceUnloader`, submits it to world virtual+18, and its destructor calls world normal unload+44. Constructor stores the name pointer at+30; delay duration and independent ownership of that pointer remain unproved. |

These functions prove queue detachment/reference release. They do not by
themselves prove OS I/O cancellation, removal of every already-posted completion,
or invalidation of a callback already executing. The queue-front pointer check
protects a particular pop operation; it is not evidence of a request generation.
Likewise, refcounts prove lifetime accounting, not semantic permission to activate
a mission. Normal unload's mode-dependent early exits must not be generalized
into unconditional removal of every duplicate name.

## Lifecycle pseudocode

Observed retail boundary, with intentionally unresolved steps:

```text
activate(m):
  mark ACTIVE; emit state change
  if requires_location and not RESOURCE_LOADED:
    world.accept_load(location_name)   // accepted != ready
  else: emit activation / begin mission processing

world_tick:
  update current load location
  if location.state == 4:
    clear pending_load; pop only if head still matches
    emit generic completion; emit named completion

faction_named_completion(m):
  base dispatch; match m.location_event
  conditionally activate; RESOURCE_LOADED = true
  // no local active/request-generation check traced here

abort(m):
  ACTIVE = false
  if requires_location and RESOURCE_LOADED:
    schedule delayed normal unload; RESOURCE_LOADED = false
  // pending cancellation / subscription invalidation not established
```

Proposed jc2-rs design, **not recovered retail behavior**: a launch owns a typed
`LocationRequest { owner: MissionRunId, generation, name, state }`; readiness
includes requested location objects reaching their agreed initialization boundary.
Abort/replacement invalidates generation before asynchronous work can report
ready. Completion must match current owner+generation and uncancelled state.
Resource failures produce an explicit failure, not a synthetic RESOURCE_LOADED.
Reference ownership of shared world locations is separate from mission activation
permission. Coordinate these checks with JC2-003's existing owned-entity cleanup;
this brief does not duplicate that implementation.

## First coding milestone and acceptance

Replace unconditional script-start `RESOURCE_LOADED` with one explicit readiness
ticket for a representative faction mission location. Preserve archive parse,
object initialization/registration, failure and cancellation as distinct phases.
Do not initially promise complete broader-mission coverage or retail worker
cancellation parity.

- Delay a location's object registration after its bytes finish loading: no
  RESOURCE_LOADED or mission activation before the final agreed boundary.
- Abort before location completion; deliver its queued completion afterwards:
  no activation, no resurrection, no resources attributed to the aborted owner.
- Abort A, start B; complete A afterwards, including same-name A/B requests:
  owner/generation checks reject the stale completion without cancelling B.
- Repeat load/abort/load; distinguish accepted request from ready notification,
  avoid duplicate completion delivery and retain other owners' shared resources.
- Inject read/parse/object-init failure: report failure and release owned work;
  avoid a mission waiting forever or treating failure as ready.
- Unload across several budgeted frames: retain an unloading state until teardown
  completes. Queue detachment and gameplay invisibility alone are not completion.

Use deterministic delayed/failing loader fixtures for these boundaries. No tests
were run here because no runtime implementation was changed.

## Reproduction and remaining trace leads

All exports below are ignored local research scratch, not committed assets:
`research/analysis/pseudo/_unnamed/text_XXXXXXXX.c` groups VAs by first four hex
digits. Locate consumers with:

```powershell
rg -n 'RESOURCE_LOADED|finishedLoading|__LocationFinishedLoading' research/analysis/pseudo/_unnamed
rg -n 'FUN_005bdd00|FUN_00551700|FUN_00830560|FUN_0086d0b0' research/analysis/pseudo/_unnamed
rg -n 'CLocation|CResourceLoader|CDelayedResourceUnloader' research/analysis/classes.csv
```

For a VA, read the corresponding export from its `// VA size=...` header to the
next separator. Direct PE verification maps RVA `(VA - image_base)` through the
section table (`PointerToRawData + RVA - VirtualAddress`), then reads little-endian
32-bit vtable words or NUL-terminated suffix bytes. Never iterate the short
secondary `CLocation` vtable `0x00e3ca44` as if it were the primary table.

Next bounded reverse-engineering targets: base event dispatch `0x005af620` and
mission destruction/subscription removal; `CStreamQueueItem` completion and
`0x008e8da0`/`0x008ebb30` ownership to establish worker cancellation; caller-side
state2 entry and identities of +40/+48; machine-code suffix argument at
`0x00520070`. Until these links are complete, retain the explicit unknowns above.
