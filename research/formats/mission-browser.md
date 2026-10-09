# Mission browser and bounded giver access

JC2-021 implementation on codex/mission-browser, baseline3dbc524.
The shared catalogue and generation-aware launch service from JC2-003/009/019
remain authoritative. Browser rows use stable mission IDs, not native icon ordinals.

## Player actions and resource boundary

J opens the mission inspector; M opens the map and Tab opens its mission tab.
Original names, descriptions, BMF faction, prerequisite/chaos/completion status and
giver positions come from MissionDb.catalog. Selection does not launch.
Waypoint only changes an independent MapState.waypoint. Travel uses existing map
travel and is refused while a mission is preparing/running. Start uses normal
LaunchOrigin::Pda or Giver without debug_override and waits for the shared provider.
Start is absent when locked/completed/busy or unsupported. Cancel queues a shared
owner-specific cancellation; stale acknowledgements cannot resurrect cancelled work.
M/Esc closes, releases the cursor, restores virtual time and consumes the close key.

Only f2t10 has a normal provider, and only the supported geometric closure from
JC2-009. This includes registered geometry, owned exported entities and collision
before activation; it does not certify teams, dialogue, cutscenes, scripted warnings
or complete mission gameplay. Unsupported missions can be inspected/marked/travelled
to, but cannot be started here. Original PDA Select opens intelligence;
direct Start here is a new project feature, not retail-parity evidence.

## Original giver evidence and bounded policy

The f2t10 and km03 employer BLO fixtures retain smart-object/action nodes, trigger
box [5,5,5], local/world transforms, original prompt keys, action event strings,
cutscene-end employer event and raw status-router fields. f2t10 has three stored
routers (0 valid,3 locked,2 completed by graph names); km03 has four including
active. These associations are diagnostic evidence, not a decoded native predicate.

Giver geometry holds a distinct monotonic LocationOwner::Giver ticket. Unavailable or unloaded giver tiles release it before location reconciliation; independent Tile/Mission references retain shared resources. New tickets receive new owner identities.

Production giver interaction currently supports f2t10 only. Its owning tile must
be loaded, the shared employer geometry generation Ready, the hero alive/on foot,
and shared mission unlock checks Available. A transformed half-box OBB check is
explicit project policy; no indicator sphere or fabricated large-radius trigger is
used. The original prompt is Activate; km03's stored Talk prompt is parsed for
fixtures but not exposed until its actor/team/briefing resources are supported.

Employer phase uses an independent normalized <id>Emp namespace and tracks
Available/Preparing(owner)/Active(owner). A giver launch explicitly skips its
unavailable briefing: only after the shared service reports Started does the
observed msy.f2t10Emp.complete event fire, once per employer state. It does not
mark MissionsDone or pay a reward. Vehicle.spawn, intro.play and other original
action events are retained, not simulated by this skip. First-completion reward
idempotence remains in the existing mission lifecycle.

Remaining retail parity (JC2-047): native smart-object overlap/enable and status-router
opcode/target semantics; original briefing activation/skip consumers; late employer
actors/vehicles under shared gameplay ownership (JC2-030); broader teams/cutscenes
and mission gameplay closure (JC2-010). Geometry readiness alone cannot close them.

## Validation

Eight new fixtures require JC2_TEST_DIR pointing to the installed game:
original f2t10/km03 prompt/box/router/action identity, inspect/waypoint/travel
separation, locked/unsupported launch rejection, normal giver receipt wait and
cancellation/retry, one employer skip event without mission completion, and a
headless modal movement/fire/mouse suppression + pause restoration check, stale employer-context selection and real employer geometry shared-owner teardown.
Run release workspace tests with the shared build lock and isolation42.

Validated isolation42 release workspace: 360 tests passed (141 formats,219 game), matching source marker count360; eight new browser fixtures pass with installed game data. Clippy --release --workspace --all-targets -- -D warnings, workspace release build and cargo fmt --all -- --check pass. The real employer fixture exposed CCutsceneObject.filename being misclassified as render geometry; placement now excludes this class from model resolution while its raw properties and script support checks remain intact.

Initial private GPU capture: jc2-rs/_extract/mission-browser-validation/browser-f2t10-locked.png, using a copied executable under the shared build lock. Capture findings corrected in delivery: display-only escaped newline conversion, ASCII separators/selection marker, hidden inactive map travel header and selected mission page for --open-missions/giver entry.

Coordinator acceptance on integrated main30314eb: 372 real-data release tests
(143 formats +229 game), matching source markers; strict release all-target Clippy,
formatting and workspace build passed. Fresh copied-executable capture
`target/browser-stunts-acceptance10/browser-locked.png` exited cleanly and was inspected
at1280x720. Selected Offensive Action appears on page5/9; original description
paragraphs, Roaches faction, Paradise Valley prerequisite,390000 chaos and giver
coordinates are readable. Separate waypoint/travel controls fit; locked Start and
the inactive map travel header are absent. These checks accept the bounded browser,
not unimplemented larger-mission gameplay or original native router predicates.
