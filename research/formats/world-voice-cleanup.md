# World voice cleanup ownership (JC2-082)

Baseline `8ae7373` (391 tests). Root's two combined world31,14/radius0 captures
rendered and exited successfully but contained stale EntityCommands::despawn warnings.
This change concerns free-roam audio entity ownership; it changes no mission,
cutscene, actor damage/reward, source-generation or rendering behavior.

A private 25-second reproduction with temporary exact-ID tracing confirmed the cause.
All eight warnings in `trace-correlated.err` matched immediately preceding finished
Shot cleanup for original `character.fev#0.4` footstep voices. Examples: 21383v0,
21475v0 and 21237v1. For 21383v0 the instance age was 0.28000173 and bookkeeping
end was 0.37997347. Bevy's PostUpdate `cleanup_finished_audio<Voice>` had already
removed its finite `PlaybackMode::Despawn` player after decoder completion; the
instance then queued a despawn for the same retired generation. The stale IDs are
sample voices, not competing world actor hierarchy owners. Temporary tracing is
removed from the delivered source.

Every Instance-tracked shot, loop and held fallback now uses `PlaybackMode::Once`:
the decoder still ends finite samples naturally, while the Instance exclusively owns
entity retirement on expiration, finish, stop, steal or loss of its attached source.
Expired shots are explicitly despawned before being removed from bookkeeping, even
when their held event remains active. Finished cleanup cannot visit those handles a
second time. Untracked Ask::Fire named sample fallbacks retain DESPAWN because they
have no Instance owner. No try_despawn or warning suppression is introduced.
Simulation-age estimates, pitch, fades, admission and mixer budgets retain their
existing contract; decoder-start latency/tail fidelity is not expanded by this fix.

Two original-data headless regressions use a deterministic decoder-EOS marker with
the actual Bevy playback-mode cleanup policy, and a panic ECS error handler. They
exercise natural completion before estimated end, scene churn/generation reuse,
held-event expired-player retirement, stop, oldest-policy steal, loops_once/budget
bookkeeping and natural cleanup of the untracked named fallback. Existing original
horn/engine/shot tests remain required.

The reusable private capture command is:

```powershell
./scripts/check-world-cleanup.ps1 -Executable '<private copied jc2.exe>' -GameDirectory '<actual game directory>' -OutputDirectory '<capture directory>'
```

It starts only its own hidden private process, runs world31,14/radius0 for25seconds,
enables world registration INFO, reads both stdout and stderr, and requires exit0,
a fresh screenshot, registered terrain and exactly zero stale-despawn warning lines.
A JSON receipt records executable SHA256, paths and warning count. Existing unsupported
texture-family fallback diagnostics remain outside this ownership fix.

Validation completed on isolation50: both targeted ownership tests passed; all393
release workspace tests (149 formats +244 game) passed with actual JC2_TEST_DIR;
strict release workspace all-target clippy with -D warnings, workspace fmt check
and release workspace build passed. Source marker count is393. The private binary
was copied under the shared lock, then Cargo50 was explicitly released22:52:26.
The final25-second private world31,14/radius0 capture exited0 with registered terrain,
a fresh1280x720 screenshot, and exactly **zero** stale-despawn warning lines across
stdout/stderr. The correlated before-fix run had eight. Saved screenshot was visually
inspected. GPU/audio50 was explicitly released after the capture. No user-owned
process was touched; no game assets or capture files are committed.

Evidence directory (main project root, not this worktree):
`C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/_extract/world-cleanup-validation50/`.
Private executable `jc2-cleanup-fixed.exe`, SHA256
`FBCE5B69590146640DD39223BAD8C320973540AAFF66480B7715847707C886CF`.
Correlated reproduction `trace-correlated.err`; final receipt `final/receipt.json`;
final screenshot `final/world-31-14.png`; final stdout/stderr
`final/world-31-14.out` and `final/world-31-14.err`.

The coordinator independently inspected the final image, receipt, binary SHA and
both logs and accepted source/test review. Integration and combined main checks
remain coordinator-owned; this branch posts awaiting-review rather than marking done.