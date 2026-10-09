# Ordinary foliage alpha, JC2-024

This bounded wave replaces
VegetationFoliage's provisional .5 cutoff with original shader-enable/reference and
initialized instance-byte semantics. It does not complete original foliage rendering.

## Native contract

The accepted JC2-025 material_view_probe.py records retail executable SHA256
`df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80`,
original read/submission ownership and pine parameter offset 0xb707 flags 0x40.
0095a8a0 reads five floats and an exact u32 word, not six semantic floats.
00968370 ordinary shader path00952cc0 enables discard when bit5 is clear.
009236d0 initializes model-header byte+0x0b to 255. 00a89b90's proven last-LOD
writer maps packed fractions0/127/128/192/254 to 255/255/253/125/1;
255 skips submission. This skip differs from submitting byte 0.
009672a0 multiplies the byte by native f32(1/255), rounds fraction*128,
and00949aa0 uploads `(2*reference_code-1)*f32(1/510)`. Byte0 has a negative
reference. Main ordinary shader 7510628 doubles texture alpha then multiplies
instance/distance fade; prez 7521584 and shadow 7522144 use a repeated-alpha dp2.
Enabled discard is strict `<`: equality survives, alpha0/byte 0 survives,
and disabled testing retains alpha0. Main/depth/shadow use one imported helper.

## Simple VS selection and distance limit

009672a0 flags bit6 chooses the table at renderer+0x134, initialized by0094b360
with `FoliageSimple%s%s%s` and Water/NoSnow/SH suffixes. Native suffix addresses
e5c61c/e5c39c/e1fe9c contain SH1/SH2/empty. Simple VS 176544/181100/184352
(foliagesimple/pl0/pl2) evaluates the length of model-origin XZ combined with
transformed/deformed vertex world Y, minus camera position. It applies
`1-saturate(distance*cb1[1].y-cb1[1].x)` times instance fraction cb2[16].x.
This is not full vertex-world distance and must not be replaced with a guessed
coefficient. Runtime distance factor remains 1; original per-asset coefficient and
LOD scheduler ownership is explicitly JC2-086. Existing Bevy visibility-range
dither remains consistent in main/prepass. Exact SH/PL runtime variant is open.

Billboard main PS 958056 proves texture alpha*4*v4.z and the same gated strict
reference test, in addition to prior prez/shadow evidence. Its enable/instance
ownership and camera-facing geometry remain unsupported; it keeps the explicit
legacy route. x4 is not applied universally.

Read-only reproduction (no assets/disassemblies written):

```powershell
& $py -B research/tools/foliage_selector_probe.py $game --export "$root/research/analysis/pseudo/_unnamed"
```

Result: two native selector/table contracts, three PE suffix byte checks, three
simple vertex shader contracts, one billboard main PS contract. Bundle SHA256
`db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98`.

## Runtime ownership and supported scope

RBM sources retain raw foliage_flags and immutable source FoliageMaterial handles.
RbmMaterial on_insert routes each mesh exclusively to StandardMaterial or foliage;
replacement removes the opposite typed material and stale instance state.
FoliageInstance only clones an asset for an explicit nondefault instance state.
Reset/removal restores the source, reuse updates its private clone, and despawn
releases the clone. Missing source assets remain pending and retry after loading.
Updates run before PostUpdate AssetEventSystems and render extraction.
Normal safety/linear handles remain unchanged; ordinary foliage has no verified
normal map. Both viewer normal comparison modes use this same no-normal source.
Forward rendering is explicit; deferred/meshlet/bindless foliage paths are not
supported. Main retains unlit behavior; depth/shadow never add a second Standard
cutoff or alpha-to-coverage. Alternate native device-state context stays unresolved.

Narrow generic attachment callsites: boat, browser/model3d, character, combat,
destruction, heli, npc, pickups, plane, vehicle, viewer and world. Headless custom
asset registration: loader, fleet, character, npc, scenery and world. No grass,
terrain, billboard scheduler, wind, full lighting or mission changes.

## Validation, isolation 49

399 release workspace tests passed (152 formats + 247 game), with the real Steam
JC2_TEST_DIR. Targeted formats 3, game foliage 6 and native pine 1 passed.
Strict all-target workspace Clippy with -D warnings, fmt and release workspace
build passed. Compilation caught one missing mutable binding; strict lint caught
constant chunks_exact usage in the fixture test. Both were corrected and their
affected checks rerun. Validated source: c96c6b6 + 5bc8a74.

Coverage includes native quantization/reference/inverse-bit/equality, exact raw decoder flags,
real pine custom source/colour/normal preservation, delayed source retry,
exclusive replacement, independent siblings and reset/removal/reuse/despawn.
The hidden `--foliage-check [--foliage-check-depth]` fixture uses Rgba16Float:
alpha0x33ff/.24987793,0x3400/.25,0x3401/.25024414 and exactzero. Nine panels
cover discarded/retained neighbors, disabled testing, byte 0,125,1 and shared-zero
source panels7/8. Panel8 resets to 255 at 1s then reuses privatebyte 0 at 2s;
panel 7 stays255. Private executable `target/jc2-foliage49.exe` SHA256:
`8d1ff8172995bde57f90b65fab16dba833ea567af44625ed6a46c9b09c53a4a6`.

GPU: AMD Radeon RX 9070 XT, driver 26.8.1, Vulkan. Receipts remain ignored under
`target/foliage49/`, including complete PNGs/stdout/stderr and synthetic-checks.json.
Synthetic main/depth captures are 1280x720 and pixel-identical across the entire
RGB image. All nine 24x30 centre ROIs match: discarded panels 0/7 have 0 retained
pixels, the seven retained panels have 720 each. Corresponding shadow silhouettes
were inspected. Depth logs confirm the source reset at 1s and private reuse at 2s,
without changing panel 7. Equality uses exact half 0x3400, not a quantized 8-bit input.

Twelve captures exited successfully: two synthetic, eight native pine, and two
identical repeats of reverse/light A. All capture logs were checked for ERROR/WARN;
none were present. Native logs confirm two spawned blocks, actual leaf resource
Bc3RgbaUnormSrgb, flags 0x40, enabled=true, byte 255 and reference0.5.

| Native view / light | Main receipt | Depth receipt | Changed RGB scene pixels (y>=60) |
|---|---|---|---:|
| Front 35°, light 30° | pine-front-a.png | pine-front-a-depth.png | 0 |
| Front 35°, light 135° | pine-front-b.png | pine-front-b-depth.png | 0 |
| Reverse 215°, light 30° | pine-reverse-a-repeat.png | pine-reverse-a-depth-repeat.png | 0 |
| Reverse 215°, light 135° | pine-reverse-b.png | pine-reverse-b-depth.png | 0 |

The 620x520 tree region at x350..970/y100..620 changes 212818 pixels between
front lights and 197292 between reverse lights. Visible leaf holes, both viewing
sides and close shadows were inspected; main/depth equality includes shadows.
Native-checks.json records these comparisons, loaded-state checks and PNG hashes.

The first reverse/light A depth capture had a whole-scene camera mismatch:
217852 scene pixels differ from its main image. Both identical repeat captures
match first main exactly, so no shader source change was justified. The original
pine-reverse-a-depth.png/stdout/stderr remain preserved. Model bounds agree.
viewer::orbit_camera currently accepts mouse/pan/scroll during material checks;
final camera/input logs were not recorded, so the cause remains unproven. A
separate deterministic capture/input-lock follow-up JC2-088
(`70da6b6b-6cc4-4714-b2aa-45c2234ceedb`) tracks stricter parity receipts.

Reproduce the supported fixture from the private executable (absolute receipt
paths recommended):

```powershell
& $privateExe $game --foliage-check --exit-after 18 --screenshot $syntheticMain
& $privateExe $game --foliage-check --foliage-check-depth --exit-after 18 --screenshot $syntheticDepth
& $privateExe $game --model vegetation_1.blz/Arctic_T22_PineL_LOD1-Whole.rbm --material-check --camera-yaw 35 --camera-pitch 20 --light-yaw 30 --exit-after 18 --screenshot $pineMain
# Depth: add --depth-prepass. Reverse: camera-yaw 215. Second light: light-yaw 135.
```

Set RUST_LOG=info,jc2=debug,wgpu_core=warn,wgpu_hal=warn,naga=warn for diagnostics.
The first synthetic main inherited suppressed logging; its valid image is exactly
identical to the fully logged depth receipt. No further production edits were
needed after GPU testing.
These captures prove the supported Bevy main/depth/shadow material behavior,
not retail GPU pixel parity or complete original foliage lighting/distance/wind.
