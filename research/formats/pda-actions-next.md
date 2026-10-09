# Original PDA actions: Lua 5.0 callback trace

Research handoff, 8 October 2026; Relay **JC2-004**, owner `mission_access`.
Extends `mission-access-next.md` under `notes/RESEARCH-HANDOFFS.md`.
No runtime code, Cargo, game launches, saves or game assets were modified.

**Verified:** the traced retail map Select action opens an intelligence file;
waypoint placement/removal has separate callbacks. Black Market extraction
selection enters map destination mode and calls the native extraction service.
**Not established:** a retail PDA mission-list action that directly starts a
mission, or the original extraction-to-mission-abort side effect. Requested direct
PDA mission launch is a new jc2-rs feature unless a different original path is
subsequently proved. This conclusion comes from decoded routes and consumers,
not just absence of a `StartMission` string.

## Dialect, prototype schema and reusable probe

`gui/pda.bl` is 5,020,654 bytes. Its 14 embedded signatures are all valid chunks
with header `1b4c7561500104040406080909043bafef4b`: Lua version0x50,
little endian, int/size_t/instruction=4 bytes, opcode/A/B/C widths6/8/9/9,
**lua_Number=float32**, and numeric header test31,415,926. They are not Lua5.1
chunks: **A is at bit24**, C at bit6, B at bit15; constant RK operands use base250.
Prototype layout: source string, defined line, four bytes(nups/params/vararg/stack),
line array, locals(name/startpc/endpc), upvalue names, typed constants,
recursive child prototypes, instruction words. Strings use uint32 byte length
including terminal NUL; constant tags0=nil,3=float32,4=string.
This matches [Lua5.0 undump](https://www.lua.org/source/5.0/lundump.c.html),
[instruction definitions](https://www.lua.org/source/5.0/lopcodes.h.html), and
[register limit](https://www.lua.org/source/5.0/llimits.h.html), with the retail
header establishing its 4-byte number customization.

`research/tools/lua50_probe.py` parses these embedded chunks without executing
them. It prints prototype/constant/instruction offsets, ABC/Bx/sBx fields,
constant references and jump targets; it is a probe, not a source decompiler or
complete bytecode verifier. Header/count/string/bounds/opcode checks reject bad
input. Source/debug text is mostly stripped; method names are recovered from
root CLOSURE -> self SETTABLE assignments. Prototype paths in this brief are
relative to their chunk root, e.g. `0.14` means child14.

| Chunk byte offset | Parsed size | Prototypes | Identified root function group |
| --- | --- | --- | --- |
| 3735764 | 3339 | 9 | Help/demo time |
| 4090618 | 5046 | 7 | Mouse/input support |
| 4152048 | 4794 | 11 | Marker input |
| 4157305 | 7900 | 12 | Progression bars |
| 4210091 | 797 | 3 | UIContract initialize/select |
| 4429521 | 1614 | 8 | Marker movement |
| 4434601 | 14563 | 8 | Help text/events |
| 4457493 | 1770 | 6 | Subtitles |
| 4476255 | 12984 | 21 | Icon legend |
| 4654433 | 14914 | 15 | Intelligence file |
| 4712122 | 54494 | 40 | Database |
| 4782670 | 39760 | 32 | Black Market |
| 4874084 | 59981 | 34 | Icons/map/actions |
| 4965986 | 21741 | 28 | Main PDA routing |

All14 parsed, total234 prototypes and32,560 instructions. Every printed constant
reference resolved; all JMP/FORLOOP/TFORPREP targets were within their prototypes.
Truncating the first chunk by one byte and changing version0x50 to0x51 were rejected.
Those checks validate structural decoding, not gameplay execution.

## Icon identity and state contract (verified)

Main chunk4965986 child23 (`read_icon_data2`, code@4985546) calls
`Game:SetIconIndex(i-1)` at4986346..4986354, then `Game:GetIconInfo()` at
4986362..4986366. It stores nine returns in arrays indexed by Lua **i=1..N**:

```text
type, faction, completed, restricted, is_new, x, y, z, discovered
```

Native wrapper **0x00a30e10** calls **0x0067cbd0** with nine outputs in this same
order. The UI negates map y at4986430, clamps displayed z to at least200, and
subtracts10 for icon type<5. Discovery is honored for6<type<13; outside that
range the script makes the local discovered flag true. Exact native type labels
and hidden/filtered index generation still need their consumer/table join.

Icons chunk4874084 update child4, instructions1346..1348 at4904144..4904152:
`v.current_icon=i; current_jc_icon=i-1`. This distinction is essential: the native
index is **not the mission id**, and UI one-based indices must not be passed unchanged.
The same update uses `Game:GetMissionNameByIndex(i-1)` at4904720..4904728 for label
display. It handles nil/empty/`-1` as an error name; this is a read operation.
It displays completed/restricted/new flags and rewards independently of launch.

## Action selection -> intelligence file (verified)

Main `ok`, chunk4965986 child11, code@4979327, checks g_input. When
g_current_menu=0(map) or11(extraction destination mode), it fires event[1],
created as `select_map` in main initialization. Icons initialization registers
that event -> `select` at4889408..4889420.

Icons `select`, child14, code@4922256:

```text
if v.coll == true and v.icon_is_discovered == true:
    if g_current_menu == 0 and v.show_collision == true:
        fireEvent(v.open_intelligence_file_event)
    elseif g_current_menu == 11:
        self:extract()
```

Intelligence chunk4654433 initialization registers `open_intelligence_file`
from script_icons_beta -> `open` at4660803..4660811. Child4 `open`, code@4664704,
reads current_jc_icon; calls GetPDASurveillanceText, GetChallengeType/Difficulty,
GetPDAText(index,3), GetPDAText(index,1), and reward text fields5/6. It sets
g_current_menu=10 at4667208..4667212. Closing child8 returns to menu0 at4668423.
The traced callback fills/scrolls an intelligence panel; it does not call a
mission-start API. `open_obj_int_file` child7 is a RETURN-only stub in this asset.

Database chunk4712122 is a separate Operation/Controls/Profiles/Statistics/
Vehicles/Colonels file browser using GetPDAFile APIs. It is not proved to be a
mission-launch list merely because it contains mission-related prose.

## Separate waypoint operations (verified)

Icons initialization binds `gui_waypoint` -> child17 `toggle_waypoint` at
4889296..4889308, and mouse support `toggle_waypoint_event_pc` -> child18 at
4889636..4889648. Toggle is allowed in menu0; it clears an active waypoint or
sets one using gamepad/PC method, then refreshes help text.

Child19 `set_waypoint`, code@4925007, calls
`Game:PlaceWayPoint(v.coll_box_target_x, -v.coll_box_target_y, icon_z+10)`
at4925107..4925127. icon_z defaults0, comes from g_icon_z for a valid icon,
and is0 for a settlement. It then calls GetWayPoint for its returned display
height and marks local waypoint_active. Child20 handles the PC cursor coordinate
path separately; do not assume its positioning is identical to gamepad snapping.
Child21 `clear_waypoint`, code@4926300, calls RemoveWayPoint at4926336..4926340
and clears its UI flag. None of those observed operations starts the selected mission.

Native PlaceWayPoint wrapper **0x00a3aac0** -> **0x005e0bd0** -> **0x005f1ea0**:
stores x at object+0x10, third argument(height) at+0x14, second(horizontal z) at
+0x18. It raises height to the larger of200 and a terrain query, updates the
CTargetIndicator transform/flags and fires show/hide indicator events. Thus Lua
argument order is `(world_x, world_z, height)`; the map's negated y is horizontal
world z, not world vertical y. The helper's optional height sentinel differs
from the explicit icon_z+10 path.

## Extraction route and native destination (verified)

Black Market chunk4782670 child10 `select`, code@4808370: with cur_win=0 and
start_type_sel>=2 it fires `extraction_selection_event` at4808618..4808626,
then sets g_current_menu=11 at4808630..4808634. Initialization creates that event
as `enter_extraction_mode` at4795621..4795637. Icons initialization binds it to
`hide_stuff_for_extraction`; child23 invokes hide_blackmarket_stuff(true).

Black Market reset child4 obtains **three booleans** from IsBlackmarketActive at
4805554..4805558 and selects the second/third return for extraction_available:
if second==false use second, otherwise use third. Movement child17 limits the
start menu to the first two entries when extraction_available=false. Exact names
and writer logic for those booleans are unresolved; do not turn them into guessed
heat/mission predicates. Native **0x00a391a0** -> **0x00747650** returns globals
0x01197ac8/0x01197ac9/0x01197aca; trace their writers for the real eligibility rules.

In mode11, the shared map Select route reaches icons child16 `extract`,
code@4923310. It sets menu12, hides the collision box and, when debug=false,
calls `Game:Extraction(current_jc_icon)` at4923386..4923398. The rest animates
the map trip and displays settlement name/completion/GetPDAText(index,5/6).

Native bindings are in `text_005b0000.c`:

| Address | Evidence |
| --- | --- |
| 0x00a311e0 | CLuaGame Extraction wrapper, integer index ->0x007f9e80 |
| 0x007f9e80 | index=-1 ->0x00555f70; otherwise lookup0x00699510, virtual+0x14 gets position ->0x006cf2f0 when manager0x0118fb30 exists |
| 0x00699510 | Enumerates a filtered icon collection; tests flags0x20 and bit2, parent bit2, excludes special record at manager+0x54; exact symbolic meanings unknown |
| 0x006cf2f0 | Extraction start gate manager+0x16; sets+0x15=1,+0x17=0,+0x3e9=1; destination+0x41c/420/424 uses input[x,y+200,z]; creates CExtractionMetric and queues state codes |
| 0x00a2aaf0 | IsExtraction returns extraction manager+0x3e9 |

The native code proves a destination/transition operation, not direct mission
launch. Caller-specific implicit ECX arguments need checking in machine code
before naming managers beyond the supported fields. The -1 branch changes the
player transform through0x00555f70; it is not the normal selected-icon path.

## Legacy confirmation and mission-state uncertainty

Icons child11 `show_extraction_dialogue`, code@4920710, requests two values from
`Game:GetExtractionStatus`, chooses blocked/normal/abort-warning content and sets
menu2. The read_txt table contains the abort-current-mission warning. **No caller
or event registration for show_extraction_dialogue was found in any of the14
decoded chunks**, and GetExtractionStatus is absent from the inspected CLuaGame
registration/export. Main still creates select_extract, but icons registrations
do not bind it. These are consistent with an unused older UI path; external
Gameface/native invocation remains possible, so dead-code status is an inference.

No original extraction mission cancellation call has been identified. The observed
native transition writes and UI warning do not establish which objectives,
checkpoint, reward state or employer state are changed. Further bounded work:
trace writers of availability globals and extraction manager+0x16/+0x3e9,
subsequent extraction update/vtable and mission abort/retry handlers. Also decode
`gui/pda.agui` event metadata to cover native/Gameface callbacks outside these chunks.

## Runtime handoff and first coding milestone (proposal)

```rust
struct PdaIconSnapshot {
    native_index: u32, kind: i32, faction: i32,
    completed: bool, restricted: bool, is_new: bool,
    world_position: Vec3, discovered: bool,
}
enum PdaAction {
    Inspect { icon: u32 }, SetWaypoint { world_position: Vec3 }, ClearWaypoint,
    EnterExtractionSelection, Extract { icon: u32 },
    StartRequestedMission { mission_id: String },
}
```

Keep original action identities separate from the requested new Start button.
Join mission ids to employer/catalog data via an explicit stable mapping, not
the filtered native icon ordinal. The first runtime milestone is an inspect-only
mission browser plus separate waypoint/extraction action requests with modal input
ownership and world-space coordinates. New direct launch must reuse the validated
mission lifecycle service described in mission-access-next.md (unlock/active-run
policy, cleanup, destination readiness, briefing, idempotent rewards). Selection
must not itself teleport or launch. Original extraction behavior can be modeled
only after eligibility and cancellation are resolved, or clearly labeled as a
new policy. No runtime behavior was implemented or tested by this research.

Acceptance later: native index i-1 roundtrip; changing filtered icon order cannot
change mission id; map/world sign and height are correct; inspected locked/completed
icons do not accidentally request launch; waypoint does not mutate mission state;
extraction callback fires once and uses destination readiness/cancellation policy;
modal Back/Select does not leak into combat. New launch tests cover chaos boundary,
active run, repeated request and repeated completion reward guards.

## Reproduction and validation

From game root, private prebuilt tool and ignored asset storage:

```powershell
$tool = '.\jc2-rs\research\analysis\mission-access-tools\jc2tool.exe'
# Capture stdout as binary with Python; PowerShell text redirection can corrupt it.
python -c "import subprocess,pathlib; pathlib.Path('jc2-rs/_extract/mission-access/pda.bl').write_bytes(subprocess.check_output([r'$tool','cat','.','gui/pda.bl']))"
python jc2-rs/research/tools/lua50_probe.py jc2-rs/_extract/mission-access/pda.bl --summary
python jc2-rs/research/tools/lua50_probe.py jc2-rs/_extract/mission-access/pda.bl --chunk 4874084 --match '^0\.14$|^0\.16$|^0\.19$'
git -C jc2-rs check-ignore _extract/mission-access/pda.bl
git -C jc2-rs diff --check
```

The extraction command assumes the ignored directory already exists. Raw assets
and redirected instruction dumps belong only in `_extract/` or another verified
ignored location; `research/analysis/` alone is **not** universally ignored.
The probe itself contains no retail bytes. Local native addresses refer to the
retail executable at image base0x00400000, exported under ignored
`research/analysis/pseudo/_unnamed/` with functions.csv/strings.csv provenance.
