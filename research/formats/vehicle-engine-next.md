# Original vehicle engine selector: numbered EPE presets

8 October 2026. JC2-005 follow-up to `world-vehicles-next.md` / `audio.md`.
The initial research/helper pass used no Cargo, game launch, audio capture or
runtime changes. The separate JC2-027 runtime milestone is recorded below.
Retail executable SHA256:
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`.

## Confirmed result

The opaque sound preset hashes are **hashes of `preset_00`, `preset_01`, ...**.
Vehicle EPE exports contain **module ID and numeric preset index**, rather than
the preset hash or human-readable preset name. This resolves the earlier failed
search described in `audio.md`. Vehicle/container names are unnecessary for this
join; they locate the export, not choose its sound.

`FUN_009caed0` (`text_009c0000.c`) explicitly generates `preset_0%d` for indices
below 10 and `preset_%d` otherwise. It iterates the declared preset count, capped
at 100, reading sound IDs into a vector of 8-byte entries. Independent
`jc2tool hash preset_00 preset_01 ...` exactly matches the retail module keys.
Examples: index0 -> 0x0be926bf; index3 -> 0x28f21a64; index4 -> 0x0b617e8d;
index5 -> 0x4b9a887a; index8 -> 0xb4c02d8b; index18 -> 0xa1a3e0dd.

### Native selection chain

| Address / source | Verified operation |
|---|---|
| **0x00780ab0**, `text_00780000.c` | Base vehicle property loader reads three module/preset pairs; module defaults to -1, preset starts at -1; stores valid pairs indexed by module and invokes sound initialization. |
| **0x01072158** | Three u32 property hashes in original PE: `d0f50183`, `1894a3e8`, `0c956561` (module selectors). |
| **0x01072164** | Corresponding preset hashes: `9f996ed1`, `78bda2b6`, `834b3da2`. These are property names' hashes, **not** hashes of the selected preset names. |
| **0x009df900**, `text_009d0000.c` | Module ID indexes the descriptor vector at +0x7c. Allocates CVSCImplementation using the selected module descriptor and numeric preset argument. |
| **0x009e8d90**, `text_009e0000.c` | Implementation constructor stores descriptor at +0xc, **numeric preset at +0x10**. |
| **0x009e84e0**, same file | Uses implementation+0x10 to select descriptor's +0x1c vector entry at `index*8`, passes its bank byte and sound ID to callback. Negative sound ID remains silent. |
| **0x00a070d0** -> **0x009db320** | Callback creates the original sound instance from the bank/sound pair and shared vehicle node context. |
| **0x00a055a0**, `text_00a00000.c` | Parses component descriptors and primary roles, gets declared preset count, builds numbered role vectors using 0x009df770 -> 0x009caed0. |

Object-relative offsets vary by base-subobject pointer: 0x00780ab0 shows selector
vector +0x4b0, while 0x0080d450 shows +0x4b4. Do not copy both as offsets from one
unverified native `this`. Ghidra omits implicit ECX and some arguments; the index
flow is verified across these functions, but native hook implementations need
assembly confirmation.

The raw module ID field in the sound settings is **0x6a9dfd76**. Settings identify
0=lave_engine, 1=lave_chassis, 2=seve_engine, 3=arve_engine, 4=lave_effects,
5=arve_effects, 6=seve_effects, 7=siren. Module identity is separate from FMOD bank:
all these sampled engine presets use bank5.

## Exact retail fixtures and assignments

Each source EPE below is `<container>/<container>.epe`, for example
`lave.v014_jeep.ee/lave.v014_jeep.ee.epe`. The first pair is
`#d0f50183=module`, `#9f996ed1=preset_index`.

| Container / class | Raw pair | Selected preset key / name | key_on; sustain; key_off (bank:sound) |
|---|---|---|---|
| `lave.v014_jeep.ee` / CCar | 0,5 | 4b9a887a / lave_engine SUV | 5:21; **5:20 Cheyenne**; 5:22 |
| `lave.v047_jeep_small.ee` / CCar | 0,8 | b4c02d8b / lave_engine SmJeep | 5:36; **5:35 Jeep**; 5:37 |
| `lave.v024_sportscar.ee` / CCar | 0,18 | a1a3e0dd / lave_engine astonmartin | 5:79; **5:78 Ferrari 430**; 5:80 |
| `seve.v082_motorboat_standard_small.ee` / CBoat | 2,4 | 0b617e8d / seve_engine longtail | 5:88; **5:87 Longtail**; 5:105 |
| `arve.v059_civilian_helicopter.ee` / CHelicopter | 3,3 | 28f21a64 / arve_engine heli_r44 | 5:99; **5:70 Helicopter R44**; 5:100 |
| `arve.v010_personal_propellerplane.ee` / CAirPlane | 3,4 | 0b617e8d / arve_engine cessna | 5:85; **5:84 Cessna**; 5:86 |
| `arve.v009_military_helicopter.ee` / CHelicopter | 3,2 | 96521217 / arve_engine jet_heli | 5:42; **5:41**; 5:43 |
| `arve.v071_jet_fighter.ee` / CAirPlane | 3,1 | 5d0a46c6 / arve_engine jet_plane_afterburner | 5:101; **5:18**; 5:102 |

These names/pairs are verified by the helper and `jc2tool soundmap --list vehicles`.
The preset's display name does not necessarily describe the event/sample: retail
`astonmartin` plays Ferrari, `longtail` key_off is named Hovercraft Engstop (5:105).
Preserve the data rather than correct names by intuition.

Current `audio.rs::engine_preset` instead assigns v014 hummer, v024 Corvette,
v082 speedboat, v009 military heli_transport. The native selector plus actual EPE
join establishes these differences, beyond merely proving presets exist.
The v047, v059, propeller plane and fighter examples happen to agree.

Second pair adds chassis: jeep has `#1894a3e8=1`, `#78bda2b6=2`, selecting
wind_and_chassi_noise (4:2). Small jeep and sports car select chassis preset0 wind
(4:0). Sampled boat/heli/plane have -1/-1 here. Third module property 0x0c956561
is absent in all 90 scanned exports; do not interpret 0x834b3da2 alone as an engine
selector. Land and air derived loaders separately select effects using fixed
module IDs (`text_00750000.c`'s **0x007557e0** loader invokes module4;
**0x007cd740** in `text_007c0000.c` invokes module5). Effects/siren setup is outside
this engine milestone; exact enclosing addresses should be checked before hooks.

## Helper validation and boundaries

`research/tools/vehicle_sound_assignments.py` parses jc2tool property JSON entirely
in memory. It restricts module discovery to `component_list`, joins by numeric
module ID and hash(`preset_%02d`), and reports every raw source/property pair plus
role IDs. It emits absent/disabled/invalid/unresolved statuses separately and
never falls back to a name guess. JSON/text go to stdout; it extracts no assets.

Executed default six fixtures successfully, then `--all --json`: **90 visible
.ee exports**, **270 selector rows**, **137 joined component assignments**,
**43 disabled pairs**, **90 absent third-module properties**, **zero extraction/
parse failures**. There are **89 joined engine assignments**; the remaining
export `arve.v062_cutscene.ee` explicitly disables engine pair (-1,-1). This is
the installed VFS inventory, not proof of every DLC/version/override combination.
The helper assumes the conventional top-level `<container>.epe` member and reports
an error if a later export differs. It does not resolve EPE inheritance or placement
overrides, or load/test FMOD samples. No synthesized mapping file is required.

Unknown: original authoring names of the six selector properties; spawn-specific
override precedence; exact native RPM/load smoothing/engine-state timing; mixer
and audible original-game parity. Shared preset display names are unsafe identities:
land preset2 and preset19 both say sedan, but their secondary roles differ.

## Proposed ownership and first coding milestone

Decode `VehicleSoundSelection { slots: [Option<ModulePreset>;3], raw_fields:... }`
from the **vehicle export EPE**, alongside the already loaded doll/model physics.
`ModulePreset { module_id:u32, preset_index:u32 }`; resolution uses module ID and
the numbered key, never case-insensitive display-name lookup. Retain distinction
between explicit -1 silence, absent data and failed lookup. Preserve raw properties
for diagnostics until names/overrides are recovered.

Attach resolved selection to the spawned vehicle entity; audio owns start/sustain/
stop voices and uses the existing soundmap/FEV player. Keep name heuristic only as
an explicit legacy fallback for missing export support, with provenance diagnostics;
do not override a decoded disabled selector. Cache export selection by resolved
VFS asset identity while tracking voice lifecycle by occupied **entity**, so
switching between two identical vehicles restarts the correct instance.

Minimum milestone: load pair0 from EPE, resolve module/preset by numeric IDs, and
replace engine selection for the six fixtures. Meaningful checks: exact raw IDs and
event pairs above; negative cutscene selector stays silent; duplicate sedan names
resolve their own raw keys; missing/bad indices are diagnostic; identical-model
vehicle transfer stops previous voice and starts next once. Future fixed RPM/load
audio capture must compare engine layers and transitions, since joins alone cannot
prove playback behavior. Add chassis/effects only as separate subsequent work.

## Reproduction (PowerShell, from jc2-rs)

```powershell
$game='C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2'
$tool=Join-Path $env:TEMP 'jc2tool-material-water-research.exe'
$py='C:/Users/Rusty/.cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe'
# Prepare only if this private copy does not exist; never invoke Cargo here.
if (!(Test-Path -LiteralPath $tool)) { Copy-Item -LiteralPath target/release/jc2tool.exe -Destination $tool }
& $py -B research/tools/vehicle_sound_assignments.py --tool $tool --game $game
& $py -B research/tools/vehicle_sound_assignments.py --tool $tool --game $game --all --json
& $tool hash preset_00 preset_03 preset_04 preset_05 preset_08 preset_18
& $tool prop $game 'lave.v014_jeep.ee/lave.v014_jeep.ee.epe'
& $tool soundmap $game --list vehicles
Get-FileHash -LiteralPath "$game/JustCause2.exe" -Algorithm SHA256
rg -n 'preset_0%d|preset_%d' research/analysis/pseudo/_unnamed/text_009c0000.c
rg -n '01072158|01072164|0080d450' research/analysis/pseudo/_unnamed/text_00780000.c
```

No repository runtime files were modified during that research pass. Raw PE/assets and native decompiler
dumps remain outside committed deliverables.

## JC2-027 runtime milestone, 8 October 2026

`soundmap::VehicleSoundSelection` now retains all three raw property pairs and
typed absent/disabled/selected/invalid states. Numeric resolution uses module ID
and hash(`preset_%02d`), with the original numbered-loop bound and ambiguous
module/key rejection. Display names remain diagnostic only. Engine scope starts
from pair 0; later positive selections for the same engine module replace its
preset. Re-reading `0x00780ab0` confirms this module-indexed overwrite and confirms
that a later negative preset does not clear the earlier preset. An additional
different engine module in later slots is unsupported and silent in this milestone.
Chassis/effects decoding is retained but does not create voices.

`audio.rs` resolves conventional `<container>/<bare-container>.epe` through the
mounted VFS, caches by full bare asset name in the immutable `Sounds` filesystem
namespace, and attaches the resolved engine to its vehicle on first occupied
activation. Retaining names prevents a hash-only cache collision from borrowing
another asset's selection; archive hash collision behavior itself remains the
existing VFS behavior. Mounted archive/dropzone precedence is unchanged. No EPE
inheritance or placement-specific override support is claimed; absent support
uses explicitly logged legacy fallback. Malformed/unreadable exports, invalid or
unresolved numeric selectors and explicit disabled exports never guess an engine.

Voice ownership tracks occupied entity, so changing between identical models
stops the old held voice and fires old key_off then new key_on once. Exit/free
camera stops once, and an explicitly disabled vehicle creates no sustain/start
voice. Existing RPM/load calculations, event layers and parameter timing are
unchanged. Tests cover eight retail numeric assignments, cutscene (-1,-1),
duplicate `sedan` preset identities, malformed/absent/unknown/bounded selectors,
same-module overwrite and identical-model transfer lifetime. Isolation 41 passed
all four targeted tests and the full **345 release tests** (141 formats + 204 game,
matching source markers), with `JC2_TEST_DIR` pointing at the unmodded Steam install.
Release workspace clippy with `-D warnings`, workspace formatting and workspace
release build passed. A private `target/jc2-engine41.exe` was copied under the
build lock; it has not been launched for audio parity.
No original-game audio
capture or audible parity comparison has been performed.
