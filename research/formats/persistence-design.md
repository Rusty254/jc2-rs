# Native campaign persistence proposal (JC2-012)

8 October 2026. This is a jc2-rs design, not a retail save decoder or an
implemented writer. It follows `save-checkpoints-next.md` and current runtime
state at master 0346df3. No live saves are read or changed by this proposal.

## First deliverable and boundary

Implement an opt-in, versioned native campaign document first. Store campaign
progress and equipment; loading returns the player to a supported safe spawn
with no active mission, vehicle attachment, grapple, reload or ragdoll. Do not
offer active-objective continuation until mission resource reconstruction and
checkpoint ownership have passed their own acceptance checks.

Retail `.sav` files and `JC2Settings` are outside this writer. Use a separate
user-selected native directory and recognizable `jc2-rs-campaign` format tag;
never autodetect and overwrite a retail slot. A later importer is a separate,
read-only operation with explicit compatibility evidence.

## Version 1 schema

The envelope contains `format`, integer `schema_version`, `campaign_id`,
`snapshot_revision`, `content_identity` and `campaign`. Decode into staging
state, validate everything, then install one coherent snapshot. Proposed limits
are 16 MiB per document and bounded collections; these are implementation policy,
not original game values. Reject duplicate logical keys, non-finite positions,
negative/overflowing counters and unsupported required fields.

| Stored field | Current source / reconstruction |
|---|---|
| Money and weapon/vehicle/armor parts | `chaos.rs::Wallet`, four named u32 values |
| Chaos total and sabotage count | `Chaos.total/sabotaged`; recompute stage from `ChaosLevels`, do not persist stage as authority |
| Completed mission IDs / taken strongholds | `MissionsDone` and `Strongholds`; canonical mission identity, no payment events on restore |
| Purchased weapon levels | `combat.rs::Arsenal.levels`, keyed by original weapon name; resolve against current tables before installing |
| Equipment, selected weapon, magazine/reserve, grenades | `Arsenal` and each `ammo::WeaponState`; persist rounds and identities, recompute capacity/unlimited/reload duration from tables and upgrade level |
| Collected resources/stashes and destroyed objects | Canonical source object IDs plus separate collection/destruction/reward flags; requires stable identity milestone below |
| Settlement contributions | Canonical contribution IDs grouped by settlement ID; reconstruct `Progress.totals/alias` from content, reconcile contributions without firing count-up events |
| Safe spawn hint | Finite world position plus supported location identity; perform collision/terrain readiness and clearance checks before control resumes |

Do not serialize ECS entities, `Piece` indices, asset handles, physics joints,
GPU data, `MissionOwner` generations, cached upgrade capacities, model reference
counts, pending loads, transient heat/audio/HUD effects or runtime event queues.
Rebuild those from the validated campaign and original content.

Optional additive fields may be retained for a supported schema version.
An unknown future schema version must be refused without rewriting the input;
preserving unknown fields is not enough to guarantee future semantic safety.
Missing optional fields have documented defaults; missing required identity or
reward-ledger fields are an error, not an empty campaign.

## Stable object identities are a prerequisite

Current `Chaos.awarded` uses `(tile, unit)` and `Pickups.resources_taken/taken`
uses `(tile, ordinal)`. `Progress.done` embeds those ordinals in strings.
`MissionOwner` protects asynchronous lifetimes, but is not a persistent identity.
Mission exports and shared locations can change unit insertion order. Copying
these keys directly to disk can restore or suppress the wrong object.

Proposed `ObjectId` retains a full source tuple:

```text
canonical container + member path
source object namespace (tile placement / location / mission export)
authored identifier where available, otherwise source-record ordinal
logical placement-instance identifier
content revision/fingerprint
```

Use stable source-record ordinals only within a validated content revision.
Do not use runtime append order, spawn count, floats alone or hash alone as
identity. Detect duplicate authored identifiers and distinguish two instances
of the same exported asset. A logical mission object identity can remain stable
across retries while its runtime generation changes. The full tuple prevents
silent hash collisions; store a compact hash only as an accelerator.

Content changes require explicit reconciliation or rejection. A changed archive
manifest must not silently map an old ordinal to a new object. Asset names and
identifiers are metadata; this schema does not embed proprietary asset bytes.

## Restore, rewards and transaction order

1. Read the bounded native document into staging memory; validate version,
   content identity, unique IDs, table references, counts and finite values.
2. Enter a restore barrier: suppress combat/pickups/mission event processing.
   Cancel active/prepared mission ownership, attachments and transient physics.
3. Install wallet/progression and durable reward/collection ledgers together.
   Rebuild stage, aliases, capacities and availability from tables.
4. Request supported world/location resources. Apply durable object state after
   source identities resolve; late loads consult the same ledger. Do not treat
   terrain presence or queued requests as a readiness receipt.
5. Place the player safely, realize equipment, and resume control. A failed
   staging validation leaves the current campaign unchanged. A failed world
   reconstruction leaves restore paused with a recoverable error, not partial
   control or replayed rewards.

Snapshot revision covers all durable fields in one document. A destruction or
completion transaction records the object/completion ledger and its wallet/chaos
delta atomically in memory before a snapshot. Restore never replays reward events.
Repeated load, tile unload/reload or retry must not add money/chaos again. Rebuild
visuals without using gameplay messages as a convenient restoration mechanism.

## Disk safety and migration

Serialize a consistent snapshot to a uniquely named temporary file in the same
native directory. Flush, close and independently decode/validate it before
replacing the destination. Preserve a verified previous snapshot as a backup;
replacement semantics and durability must be tested on Windows with open-handle
and interruption cases. Avoid cross-volume replacement or deleting the only
valid copy first. Report failure without claiming that a snapshot was saved.

Migration is a pure `old document -> new staging document` transformation,
validated before the same replacement protocol. Never migrate a retail slot in
place. Future-version refusal, malformed input and unavailable content do not
trigger a destructive fallback. No migration is implemented by this brief.

## Checkpoint/retry proposal and unresolved retail behavior

Version 1 does not write or restore an active mission. A later checkpoint needs
mission ID, script/content revision, objective variables/timers, durable versus
transient world flags, logical spawned-object identities and a supported resource
manifest. It allocates a fresh runtime generation on restore; old acknowledgements,
events and entities cannot advance it. Save no raw ECS or physics snapshot.

Exact retail rollback of money, chaos, pickups, equipment and destruction remains
unverified. Define a named jc2-rs retry policy only after controlled comparisons;
do not imply that campaign-sticky rewards reproduce retail behavior. Keep abort,
death retry, checkpoint resume and campaign load as separate operations.

## Acceptance for later implementation

- A synthetic campaign roundtrips wallet, upgrades, magazines/reserve, completed
  IDs and full object tuples; recomputed capacities/stage match original tables.
- Truncated/oversized input, duplicate IDs, unsupported version, missing content,
  non-finite positions and out-of-range upgrade/weapon values fail before mutation.
- Load the same snapshot twice, then unload/reload its tiles: no repeated awards,
  resource pickups or completed-mission rewards, and settlement counts stay equal.
- Two identical assets with different placement identities restore independently;
  reordered content is rejected/reconciled rather than matched by runtime index.
- Simulated interruption at each disk step leaves an old or new valid snapshot;
  failed migration/replacement and an open Windows handle retain a recoverable copy.
- With resource failure/late receipt, control stays gated; cancellation removes
  owned bodies/joints/models and late results cannot leak into the resumed campaign.
- Checkpoint implementation separately proves objective/owned-entity reconstruction,
  fresh generation, zero duplicate rewards, retry/abort policy and cleared transients.

Validation of this proposal: inspected the current state types and reward/collection
bookkeeping plus the retail research brief. No roundtrip, disk interruption,
checkpoint gameplay or Cargo checks were run; the bullets are required acceptance
for implementation. Stable identity/ledger and native writer/restore are distinct
follow-up tasks, not delivered code.
