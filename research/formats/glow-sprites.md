# Light-source glow sprites: `all_lights.light_info` and the Lightsource pass (JC2-136) - 11 October 2026

Research only (task JC2-136, gap 8 of JC2-124). Evidence: the unmodded `JustCause2.exe`, the `lightsource` shader pair in
`DX10_Shaders_F.shader_bundle`, `global/all_lights.light_info` and `textures/atmosphere/lens5.dds`. Probe:
`research/tools/glow_sprite_probe.py` (40 checks, exit 0; every number below marked "probe" is asserted there). No Cargo, GPU or game launch.

Labels: **native** = read from the exe / shader / data and asserted by the probe; **decompiler-read** = pseudo-C only, not byte-checked;
**inferred** = reasoning, not proven; **unknown** = listed under Gaps.

## 1. What the file is (native)

`global/all_lights.light_info` is **not** a table of lamps with radii; it is a list of 32012 pre-placed *glow sprites*: the far halos of
street lamps, windows and base lights, drawn as additive-looking camera-facing quads. (The dynamic point lights of explosions and muzzle
flashes are a separate system, `point-light-model.md`; the `LightIndex` texture there is unrelated.)

Layout (probe): u32 version = 1, u32 count = 32012, then 16-byte records: `x, y, z` as f32 (world metres), then one u32 whose **low three
bytes are R, G, B** (R lowest) and whose **top byte is a type id**. Loader `0x00930000` (string at 0x00e5c080) keeps them in a temporary
array and builds one mesh per tile.

* **Tiles.** The world is an 8 x 8 grid of 4096 m tiles over +-16384 m (`CLightManager` ctor 0x00956a70 stores 32768.0 into
  DAT_0108c5a0 from the constant at 0x00e2733c). Tile index `0x0096d7b0` = `floor((z + 16384) / 4096) * 8 + floor((x + 16384) / 4096)` (probe:
  the x87 code is pinned, the float-to-int conversion is a truncating `fistp`, so it equals floor for the always-positive operand).
  The file is **sorted by tile** (probe: 63 non-empty tiles, one run each), which the loader relies on (it starts a new mesh whenever
  the tile index changes). Tile 25 (x 1, z 3) holds 3739 sprites, tile 24 holds 2952; most tiles hold 100..1000.
* Per tile the loader also keeps the smallest and largest sprite `y` (the two floats per tile at `CLightManager+0x2ae0 + 8 * tile`), used as
  the y extent of the tile's culling box. The arrays are not initialised by the constructor; if the object is zero-filled the minimum stays 0
  (inferred; harmless, all `y` >= 153.1 m).
* **Heights:** `y` spans 153.1 to 1862.2 m (probe), coordinates are 0.5 m-quantised in the GPU copy (next section), world range +-15771 m.
* **Types:** 18 distinct type ids (probe): 1, 2, 3, 4, 5, 7, 8, 9, 10, 12, 14, 16, 19, 20, 25, 26, 29, 39. Counts: type 14 = 18308 sprites,
  7 = 4570, 19 = 3424, 1 = 1795, 3 = 1512, 2 = 722, 9 = 394, 10 = 340, 20 = 204, 8 = 115, 5 = 73, 29 = 71, 4 = 427, 16 = 28, 12 = 16,
  26 = 8, 25 = 3, 39 = 2. **The type is the sprite size class: size S = 1 + type / 2 metres** (1.5 m for type 1, 8 m for type 14, 20.5 m for
  type 39; section 3). 142 distinct colours exist; each type has one to a few (type 2 is always R 0xa5, G 0xb2, B 0xff).
  What a type means in the world (lamp, window, floodlight) is not stored; it is only the size class.

## 2. GPU mesh (native, `0x00934860`, called from `0x00946d70`)

For each tile run (count `n`, first record `p`): positions are converted as `short(int(coord * 2.0))` (truncation; so 0.5 m resolution, `2.0` is
the float at 0x00e2d510), `w = short(int(float(type) - 32767.0))`, colour = bytes R, G, B of the record plus a zero byte; 12 bytes per vertex
(8-byte position block + 4-byte colour; probe pins the loads and the stride 0xc). **Each sprite is stored six times** (a non-indexed triangle
list, topology code 7 via vtable `+0x60`), and the vertex shader tells the six apart with `SV_VertexID`. The tile's mesh also binds
`Atmosphere\lens5.dds` as texture 0 (string in the exe, probe). Per tile the engine keeps the mesh handle in `CLightManager+0x2ce0 + 4 * tile`.

The input element formats are not read from the layout table (decoded only by the arithmetic below): position is **snorm16 x4**
(inferred, the shader multiplies by 16384 and the stored value is `2 x` metres, so full scale = 32767 / 2 = 16383.5 m; the 1/32767 scale
means decoded positions are `value / 2 * (16384 / 16383.5)`, a +0.003 % scale error, up to 0.5 m at the world edge, that a port can ignore), colour
unorm8 x4 (alpha unused).

## 3. The shader pair `lightsource` (native, disassembly pinned by the probe)

Vertex shader (`cb0[0..3]` view-projection in the layout `clip = x*cb0[0] + y*cb0[1] + z*cb0[2] + cb0[3]`, `cb0[4].xyz` camera position,
`cb1[0] = (A, B, C, D)`, `cb1[1] = (E, F, ., .)`):

| Step | Formula |
|---|---|
| world position | `P = v0.xyz * 16384` |
| size class | `S = (v0.w + 1) * 16383.5 + 1 = 1 + type / 2` |
| corner | `x = +1 if (id & 1) else -1`; `y = +1 if ((id + 1) mod 6) < 3 else -1` (two triangles, uv = corner * 0.5 + 0.5) |
| clip position | `c = ViewProj * P`, `w = c.w` (view depth) |
| half extent (NDC-y) | `max(A * S / w, 3) / 720`; NDC-x = that `* B` (`B` = height / width) |
| clip z | `z' = c.z + S / w` (pushed slightly away before the divide) |
| distance | `d = |camera - P|`, `t = saturate(d * D - C)` |
| intensity | `I = t^F * E` |
| kill | if `I < 0.002` clip z is set to `-2 w` (the sprite is clipped) |
| output | colour = `I * vertexColour.rgb`, alpha = `I` |

Pixel shader: `out = sprite.r * gain * (I * rgb, I)`, then alpha `* 1.5`; `gain` = `cb1[0].x` of the pixel stage = the day/night key `#178dbdf8`
(3.5 / 6.0, JC2-124). Only the **red channel** of the 128 x 128 DXT1 `lens5.dds` is read (8 mips, probe).

**The constants (native).** `FUN_0093d940` (vtable slot 12 of `CRenderBlockTypeLights`, vftable 0x00f2337c, probe) writes them each frame from the block
type object (ctor 0x00969b50 sets the defaults, probe): `C = [+0x18] / [+0x1c] = 200 / 800 = 0.25`, `D = 1 / [+0x1c] = 1/800`,
`E = [+0x20] = 0.25`, `F = [+0x24] = 1.0`, `B = height / width` of the back buffer, `A = [renderctx + 0xb24] * height * 0.5`.
`[+0x14] = gain` (default 1.0, overwritten by `FUN_00954f00` from the clock: 6.0 from 22:00 to 04:00, ramps 04:00..06:00 and 18:00..22:00 to
3.5 by day, see `night-exposure-next.md` section 3).

Consequences:

* **Distance fade.** `I = 0.25 * saturate((d - 200) / 800)`: invisible inside 206.4 m (the kill threshold 0.002 / 0.25 * 800 + 200), rising linearly to
  `0.25` at 1000 m and constant beyond (there is no far cut-off in the shader; the glow of a city is visible from the whole draw range). So the
  halos are a **far-distance impostor**: the near lights come from the real light / geometry systems.
* **Brightness.** Final colour factor at >= 1000 m = `0.25 * gain * sprite.r * colour`: 1.5 at night, 0.875 by day (before the scene composite
  and the sRGB scene target clamp; native scene unit 1.0 = white, `night-exposure-next.md` section 4), so night glows saturate toward white
  at the sprite centre.
* **Size.** The quad half-extent in NDC-y is `A * S / (720 w)` while that is at least 3 / 720. If `A / height / 0.5` is the vertical projection scale
  `cot(fovY / 2)` (inferred, see G2) the sprite is `S` metres across at 720 p and `S * height / 720` at other heights (a resolution-dependent size,
  as the constant 720 is fixed). The floor of 3 / 720 (3 px of a 720 p frame across) keeps far small sprites visible. The sprite is square on
  screen (x scaled by height / width).
* **Depth.** The clip-z push `S / w` (NDC `S / w^2`) moves the quad behind its true depth, so a depth test against terrain would hide
  sprites behind a hill but not for a sprite sitting on a wall; whether the depth test is on is a Gap (G1).

## 4. Draw order and culling (decompiler-read, `0x00970120` tail)

Each frame, after the managed-light update, the engine walks the 64 tile slots; for every tile that has a mesh it builds the box
`(x0, minY, z0) .. (x0 + 4096, maxY, z0 + 4096)` (`x0 = (i & 7) * 4096 - 16384`, `z0 = (i >> 3) * 4096 - 16384`) and tests it against the camera
frustum planes (`0x00935c20`, the same six-plane box test used by many systems); visible tiles are submitted through vtable `+0x40` of the object at
`CLightManager+0x2de0` (created by the loader, `0x38` bytes). There is **no per-sprite CPU culling**; the GPU kills sprites inside 206 m and
the quads off screen. The block uses vertex shader `[type+0x28]`, pixel shader `[+0x2c]` and input layout `[+0x30]` (names "Lightsource" for both shaders).

## 5. What jc2-rs should do (not implemented here)

* Ingest `all_lights.light_info` (16-byte records: x, y, z, then R, G, B, type). Spawn **one quad batch per tile** (or one mesh for the whole file; 32012
  sprites are cheap) with per-instance colour and size `1 + type / 2` m.
* Shader (custom, unlit, camera-facing): `I = 0.25 * saturate((d - 200) / 800)`, discard `I < 0.002`, colour `= I * rgb * gain * lens5.r`, gain from the
  existing night key (3.5 / 6.0 with the 04..06 / 18..22 ramps), alpha `* 1.5`, min half-size 3 / 720 NDC-y, size scale `S * height / 720` (labelled inferred
  until G2 is closed). Blend: engine mode 3 colour / mode 2 alpha, **unknown** until G1; the transparent-additive family is the working guess (non-native).
* A Bevy `PointLight` is the wrong tool: these are 2 D halos, not lights, and they do not light anything.

### Implemented (JC2-197, 11 October 2026)

`crates/jc2-formats/src/light_info.rs` parses the file; `crates/jc2-game/src/glow.rs` + `glow.wgsl` draw it: one mesh per tile (63 of 64
non-empty), six vertices per sprite, `SV_VertexID` corners in the vertex shader, the formulas of section 3 verbatim, gain from the clock
(`glow_gain`). `JC2_NO_GLOW=1` disables it. Assumptions, each labelled in the code and still open: additive One / One blend, depth test on
without write (JC2-195); `A = cot(fovY / 2) * height / 2` and the reverse-Z form of the clip-z push, `ndc - S / w^2` clamped at 0 (JC2-196);
the `lens5.dds` sampler (linear, clamped) and linear colour space (no `_dif` suffix; JC2-195). No haze fog is applied (the native pass writes
none that is decoded). Night capture: `_extract/JC2-197/` (`--world 11,28 --no-hero --time 23:00`, with and without `JC2_NO_GLOW`): the halos are a few
pixels across at kilometre range, as the formula says (3 / 720 NDC floor).

## 6. Verified / inferred / unresolved

* **Verified (probe):** file layout, tile sort, tile index, type and size statistics, vertex builder arithmetic, vtable slot, constants and defaults, the
  `lens5.dds` format, the whole vertex and pixel shader arithmetic above.
* **Decompiler-read:** the per-frame tile culling (section 4), the uploading of `cb1` (the arguments of calls 0x0096beb0 / 0x00958860), the mapping of
  `FUN_00942390` to width/height.
* **Inferred:** snorm16/unorm8 input formats, `A = cot(fovY/2) * height / 2`, the min-y initialisation, that the `[+0x14]` gain is the pixel `cb1[0].x`
  for every frame (the writer sits at the pixel-constant call).

## 7. Gaps

* **G1. Blend and depth state.** The pass sets engine blend modes (colour 3, alpha 2: the same pair is used by nine other passes, whose meaning is
  unknown; `FUN_00940170` maps the packed word through the table at 0x00f22858 into a driver blend state), the depth test/write bits in
  decides the look of every glow. Blocks: correct compositing. (Filed.)
* **G2. Size constant and pass placement.** Identity of the float at render-context `+0xb24` (written into `cb1[0].x` as `+0xb24 * height * 0.5`) and why
  the shader divides by the fixed 720; where in the frame order the Lights block is drawn (before/after water, clouds and post), whether any setting or
  time of day disables it, and the vertex `cb0` layout (view-projection and camera position) being the standard per-frame buffer. Blocks: sprite size
  parity and draw order. (Filed together with G1's follow-up if the board prefers; see the board.)
* **G3. Type meaning.** Nothing in the file names a type; the 18 ids are size classes only. Which world objects (street lamps, windows) the records came from
  is lost (offline-baked). Not blocking.
* **G4. Min-y initial value** of the per-tile arrays and the sampler state of texture slot 0 (filter / address mode, "sampler code 2" elsewhere). Minor.
