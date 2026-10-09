# Typed event-track checkpoint (JC2-020)

The `event_track` formats module retains the complete property tree and raw
record nodes, including unknown/repeated keys and equal-time records. Typed
fields are optional: malformed or unknown values remain visible, rather than
being assigned invented defaults. Record ordinal identifies stored order.
`jc2tool event-track <game_dir> <name.event_track>` resolves the compiled suffix
and prints the typed fields plus each raw record. An actual source override is
preferred as project policy; corrupt present direct overrides fail visibly.

`native_interval` reproduces `[low, high)` in stored order. `native_step` is a
read-only diagnostic of 0090ba50: forward mode 0 wraps the head first exactly
once, then dispatches the original tail; reverse selects an ascending interval.
Neither helper advances a clip. See `animation-next.md` for native evidence.

`EventCursor` is an explicit jc2-rs policy. It advances an unwrapped clock,
orders occurrences chronologically (backwards in reverse), preserves source
ordinal for ties and handles multiple wraps. Both directions use lower-inclusive,
upper-exclusive crossing intervals. Zero-time records fire on first positive
advance. Once playback clamps at duration, leaving exact-duration records
unscheduled; loop scheduling uses `[0,duration)` to avoid duplicate seam events.
The decoder retains exact-duration and invalid timestamps for inspection.
Seek emits nothing and can replay earlier events; cancellation is terminal.
One explicitly selected clip/generation owns events during crossfades. Mismatched
owners do not advance; deferred consumers must recheck returned ownership tokens.
Invalid clocks and steps exceeding 100,000 events fail without moving the cursor.

Tests cover retail footstep, vehicle-entry, reload and simultaneous landing
tracks, stored versus time ordering, exact boundaries, reverse wrapping, long
frames, seek, zero/negative/NaN rates, cancellation and generation ownership.
No gameplay consumers are wired: reload does not refill ammo, vehicle door
sounds are not duplicated, and root motion remains untouched. Remaining native
clock advancement, outgoing clip removal and consumer semantics are JC2-026;
actual rig integration is the separate JC2-031 follow-up after this review.

Validation (8 October 2026): isolation 40 full release workspace suite passed
341 tests (139 formats + 202 game), matching source test markers, with
`JC2_TEST_DIR` set to the unmodded Steam install. Release workspace all-target
clippy with `-D warnings`, workspace format check, and release jc2tool build
passed. A private tool copy made under the shared build lock inspected
upright_basic, explicit animations.blz/hjk_getin and ra_reload_event source
references successfully. No game window or runtime event consumer was used.
