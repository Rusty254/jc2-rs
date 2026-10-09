# Draft upstream issue for avian3d (not filed)

**Title:** Removing `RigidBodyDisabled` leaves collider proxies flagged `BODY_DISABLED`; touching
pairs then get their manifolds pushed into the `ConstraintGraph` twice (panic in
`prepare_contact_constraints`)

**Version:** avian3d 0.7.0

**Repro:** insert `(RigidBodyDisabled, ColliderDisabled)` on a dynamic body, step, remove both,
then let it touch a compound collider. Within a few frames the solver panics at
`dynamics/solver/plugin.rs:398` (`contact_pair.manifolds[manifold_index]` out of bounds).
A headless repro is `enter_exit(Seat::Disable, …)` in `crates/jc2-game/src/physics.rs`
(tests `avian_reenable_leaves_proxy_flagged_disabled`,
`avian_disable_reenable_still_breaks_the_solver`).

**Cause 1:** `add_to_tree_on::<Discard, ColliderDisabled>` and the `On<Discard, RigidBodyDisabled>`
observer in `collider_tree/update.rs` read `Has<RigidBodyDisabled>` while the component is still
present (Discard runs before removal), so the proxy keeps `BODY_DISABLED`. New pairs are then
created without `GENERATE_CONSTRAINTS`, and the narrow phase sets `STARTED_GENERATING_CONSTRAINTS`.

**Cause 2:** in `NarrowPhase::update`, the `collision_started()` branch pushes the manifolds but
never clears `STARTED_GENERATING_CONSTRAINTS`. The next status change while touching (a
manifold-count change) takes the `STARTED_GENERATING_CONSTRAINTS` branch and pushes every
manifold again, so `constraint_handles.len() > manifolds.len()`.

**Suggested fixes:** treat "being removed" as not disabled in those Discard observers (or use
`Remove` and query after removal); clear `STARTED_GENERATING_CONSTRAINTS` in the
`collision_started()` branch (or check it first).

**Our workaround:** `physics::stow_passenger` / `release_passenger` (empty `CollisionLayers`,
`GravityScale(0)`) instead of disabling. The two canary tests fail once Avian fixes the bug:
that is the signal to drop the workaround.
