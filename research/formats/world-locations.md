# Shared world locations and supported mission preparation

Implementation checkpoint, 8 October 2026. This is project policy, not a claim
of original near/far streaming parity or complete mission support.

`jc2-formats::location` retains each `locations/world.bin` record, both raw vector
bounds and the original property node. Invalid or absent bounds fail explicitly.
Bare archive aliases resolve only when the index identifies one full path. Model
container caches and asset paths now retain full identities; the asset reader
splits nested paths after the archive filename rather than the first directory.

`jc2-game::locations` loads metropolis geometry whose near box (the raw bounds inset by
1000 m, the exe's near record: `location-static-geometry-next.md`) intersects a loaded
terrain tile in XZ. Since JC2-111 every other non-`wc` registry location that starts
enabled (`conditions.bin`) loads the same way as geometry only (`LocationOwner::Scenery`):
the start base `keymissions/km01/km01.base`, strongholds, general interests, roads… One canonical container has a set of tile and
explicit mission owners, one generation and one geometry record. One new location
starts per update. Registration is independent of terrain loading, including
already present tiles. Near-only geometry (plus the far `.fl.cgd`) and inclusive near-box intersection
are explicit approximations; vertical bounds are not mission gates. Gameplay and
settlement containers are excluded from automatic city geometry streaming.

The location contract is geometry only: BLO models, PFI colliders and every CGD
member. Unknown multi-BLO/multi-PFI containers fail rather than certify a subset.
Readiness requires successful parsing/resolution, actual registered model and
collider entities, no remaining model queue, and asset dependencies loaded. Read,
parse, collision/model resolution and asset failures are explicit. Geometry
records retain actor and destructible metadata independently of terrain. The
geometry receipt still does not certify those actors, their scripts or cutscenes.

Shared owners do not duplicate geometry. Final release drops pending work, requests
entity teardown, retains an unloading state until entity teardown is visible,
then releases only the location's model references. A new request during unloading
waits for old teardown and receives a fresh generation. Old state queries cannot
observe the retry. Cancelling a mission releases its explicit location owner while
retaining other owners. Failed partial geometry releases its entities/handles.

## Shared gameplay source ownership (JC2-030)

Startup and late `ExtraContainers` registrations now request the same canonical
location record as explicit mission owners, using independent `ExtraTile` references.
Removing an extra registration releases its reference without releasing a separate
Tile/Mission/Giver reference. Extras naming an already loaded terrain archive are
ignored, preventing a second copy of its geometry and actors.

Vehicles, team NPCs (including authored colonels), weapon stashes, resource crates,
and destructibles reconcile each active container generation. Their queued requests
and spawned roots retain a `SourceOrigin` with canonical archive and raw authored
object name. The numeric generation and ordinal are strictly in-session; they are
not a durable object/save identity. Taken pickup memory uses canonical container
plus ordinal to preserve this session's empty stashes through unload/reload; durable
identity, compatibility/migration and saved world-state restoration remain JC2-028.

Gameplay is enabled after successful geometry registration and while a Tile,
ExtraTile or Mission reference exists. A Giver reference retains geometry only.
Dropping the last gameplay reference removes NPCs, unoccupied vehicles, pickup roots
(including source NPC weapon drops) and pending requests while giver scenery stays.
Re-enabling beneath retained geometry receives a new gameplay generation and retags
surviving destructible parts; old requests and hits cannot address the new lifetime.
Final geometry release also removes intact parts/debris and releases its model refs.

The existing player-vehicle policy is preserved: an `Occupied` vehicle transfers
out of source ownership on release and remains with the player. AI `Crew`/`AiDriven`
does not qualify for this exception. Ordinary source actors cannot survive release.
Settlement and stronghold counters aggregate active sources once. The 14 targets in
the retail f1s07 base remain available after startup or late registration.

Source activation provides ownership for existing gameplay implementations. It is
not a readiness acknowledgement for mission teams, controller scripts, objective
interactions or cutscenes; JC2-010 and the bounded provider below remain unchanged.

## Bounded production mission readiness

The provider in `mission_access::prepare_location_resources` supports only the
existing f2t10 exported-geometry proof. Its mission archive contains the script;
its five silo and three weapon-stash exported EPE placements must be initialized separately. The provider
requests and pins their actual terrain tiles, initializes collider/unit/model
ownership under the pending mission generation, preserves entity script scopes,
and waits for model registration plus recursive asset dependencies. It carries
spawn metadata into the active run, avoiding duplicate spawns, and defers the
spawn-completion events until activation. Registration submission or a terrain
tile alone cannot produce RESOURCE_LOADED. Cancellation/failure drops preparation
and its tile/model/entity ownership; stale receipts cannot start a later retry.

This receipt covers exported geometry; it does not implement collecting the mission weapon stashes. f2t10's existing unimplemented distance
trigger/failure-warning behavior and audio are not certified by it. Team spawn
sets, cutscenes, deferred resource spawns and all other mission closures fail
explicitly. The existing DebugScriptOnly route remains an explicit debug override.
Full world/mission gameplay parity, giver/PDA access and checkpoint rollback are
not claimed by this milestone.

## Validation and gaps

New retail fixtures cover registry identity/raw bounds; a city houses container
with actual CGD entities; shared owner release/refcount preservation; unload/retry;
missing archives; late legacy extra reconciliation/dedup; preactivation f2t10
entity registration; cancellation followed by late assets/stale receipts/retry;
and partial exported-resource read failure cleanup. A deterministic ownership
fixture covers generations during unloading, and an asset-path fixture covers
nested location directories. Build/test results are recorded in the Relay handoff.

JC2-030 adds retail stronghold late registration, shared owner release, giver-only
retention/reactivation, retired damage and delayed assets, canonical terrain-extra
deduplication and APC stale-request tests. Existing colonel, stronghold, CGD, giver,
and bounded f2t10 preparation/cancellation fixtures remain required regressions.

Branch validation (isolation 39): 344 real-data release workspace tests passed (133 formats, 211 game), matching 344 source test markers; release workspace clippy with warnings denied and cargo fmt --all --check passed. The older headless world fixture now registers the established DDS ImageLoader so readiness tests verify nested texture dependencies. Final build outcome is recorded in the Relay handoff.

JC2-030 validation (isolation 45): 371 real-data release workspace tests passed
(143 formats, 228 game), matching 371 source test markers. Strict all-target
release clippy with warnings denied, workspace release build and cargo fmt check
passed. The retail f1s07 source initialized six vehicles, six resource pickups and
three grounded NPCs after late registration; no duplicate owner spawn, retired
request/hit resurrection or source debris remained after release. Occupied player
vehicles retained independently while ordinary AiDriven vehicles were removed.
Existing colonel and bounded f2t10 preparation/cancellation/provider rejection tests
also passed. No GPU capture was required for this ownership-only milestone.
