# RBMDL — render block models (`.rbm`)

Status: parses all 121,912 models in the game without error (`jc2tool scan-rbm`),
every one of the 13 block types that occur is decoded, 31.5 M triangles. Vertex
bounds of every decoded block were checked against the file's bounding box across
the whole game to validate the position scaling.

## Container

```
u32   5                         (big-endian files read 0x05000000)
char  "RBMDL"
i32   major = 1, minor = 13, revision
f32   bbox_min[3], bbox_max[3]
i32   block_count
block_count ×
  u32   type_hash               hashlittle(type name), e.g. "CarPaint"
  ...   block body
  u32   0x89ABCDEF
```

Block bodies are not length-prefixed: an unknown type stops parsing. The engine
knows 53 types (`rbm::BLOCK_TYPE_NAMES`); counts in this game:

| type | blocks | parsed | used for |
|---|---:|:-:|---|
| General v3 | 190,627 | yes | nearly everything static |
| Lambert v4 | 7,829 | yes | unlit/simple geometry |
| SkinnedGeneral v3 | 4,186 | geometry + uv + normal (skin weights skipped) | characters |
| CarPaint v3 | 3,543 | yes | vehicle bodies |
| Facade v1 | 3,278 | yes | building fronts |
| Halo v0 | 3,170 | positions only (sprite, not drawn) | light glows |
| Merged v0 | 1,798 | yes (mode 1 geometry; mode 0 = instance list) | distant city/road LODs |
| CarPaintSimple v1 | 1,695 | yes | vehicle details |
| VegetationBark v0 | 1,151 | yes | trunks, rocks |
| DeformableWindow v1 | 958 | yes | vehicle glass |
| VegetationFoliage v0 | 710 | yes | leaves |
| Window v0 | 289 | yes | building glass |
| BillboardFoliage v0 | 16 | yes (quads in the XY plane, not billboarded) | canopy sprites |

How the seven later types were found: each block class has a 24-slot vtable
(`classes.csv`, `NGraphicsEngine::CRenderBlock*`); slot 1 is `Read(stream)`.
Reading it gives the header size (`stream->Read(this+N, size)`), the material call
(`FUN_0094c710`), the vertex reader (its stride is the `count * 0x..` in the helper)
and the face reader (`FUN_0092d7f0`). The per-field widths come from the
big-endian swap routine each reader calls when `DAT_0129dbaa` is set (16-bit swaps
= `i16` fields, `movss` = `f32`). Semantics were then confirmed on real data
(`research/tools/rbm_probe.py`).

## Common pieces

**Material** (all decoded types): 8 × (`u32 len`, ASCII) texture names in the order
diffuse, normal, properties ("mpm"), deformed diffuse, deformed normal, deformed
properties, normal_ex, shadow; then `u32 flags` (3 everywhere seen). Texture names
are bare file names: look in the same container first, then the archive top level
(`window_dif.dds`, `lightset_03_dif.dds`, tyres… are shared).

**Faces**: `i32 count` then `count × u16` triangle-list indices.

**Packed vec3 in a float** (`rbm::unpack_normal`): the fractional part ×256 is byte
0, the low byte of the integer part is byte 1, the next byte is byte 2; each byte
maps to [-1, 1] via `b / 127.5 − 1`. The sign of the float carries tangent
handedness. Verified: decoded normals vs. geometric normals of the bus body give a
mean dot product of 0.99.

**Deform table**: 256 × u32, present in CarPaint and DeformableWindow (vehicle
damage); skipped.

## Block layouts

### CarPaint v3 / v4
```
u8  version
f32 ×13, u32              params (unknown; 64.0 / 300.0 look like specular power)
[v4] deform table
material
i32 n0; n0 × { f32 x y z; f32 packed deform weights; i16 ×4 deform vectors }   (24 B)
i32 n1; n1 × { f32 u v; f32 light index (−1 = none); f32 normal; f32 deformed normal;
               f32 tangent (signed); f32 bitangent }                             (28 B)
faces
[v3] deform table
```

### CarPaintSimple v1
```
u8 version; f32 ×14; material (its own 8 names + u32)
u32 n; n × { f32 x y z w; f32 u v; f32 normal; f32 tangent }   (32 B)
u32 face_count; u16 × face_count
```

### DeformableWindow v0–2
```
u8 version; material; [v2: u32, deform table]
i32 n; n × { f32 x y z w; i16 ×4 deform; f32 normal; f32 deformed normal;
             f32 tangent; f32 bitangent; f32 u v }               (48 B)
faces; [v0/v1: deform table]; [v1: u32]
```

### General v2/v3
```
u8 version
f32 ×10                   (f[9] = 16.0 in every file seen — not the vertex scale)
u32 small_vertices        0 = big (f32 positions), 1 = small (i16 positions)
f32 vertex_scale, uv0_scale_u, uv0_scale_v, uv1_scale_u, uv1_scale_v, unused
[v3] i32 ×2
material
small: i32 n; n × { i16 u0 v0 u1 v1 (snorm × scales); f32 normal; f32 tangent;
                    f32 packed colour; i16 x y z w (snorm × vertex_scale) }     (28 B)
big:   i32 n; n × { f32 x y z; f32 u0 v0 u1 v1; f32 normal; f32 tangent; f32 colour } (40 B)
faces
```
Confirmed: all 183k General blocks have vertex bounds inside the model bbox with
`position = snorm × vertex_scale`. (The reference tools scaled by f[9] = 16 and were
16× too large.)

### Lambert v4
```
u8 version; u32 small_vertices; f32 vertex_scale, uv0_scale_u, uv0_scale_v,
uv1_scale_u, uv1_scale_v; f32 ×5 unused (often NaN/−inf); material; vertices as
General; faces
```

### SkinnedGeneral v0/v3
```
u8 version; u32 flags (bit 0x80000 with v3 = big vertices); f32 ×6; material
i32 n0; n0 × { f32 x y z; u8 ×8 bone weights/indices [+ u8 ×8 when big] }  (20/28 B)
i32 n1; n1 × { u32 normal (bytes → [-1,1]); u32 tangent; u32 bitangent; f32 u v } (20 B)
i32 batches; batches × { i32 face_count; i32 face_index; i32 bone_count; i16 × bone_count }
faces
```

### Facade v1
```
u8 version; f32 ×12 (f[11] = 16.0); u32 small_vertices; f32 vertex_scale; u32; material
small (40 B): i16 x y z w (snorm × scale); f32 u0 v0 u1 v1; f32 normal, tangent, colour; f32 extra
big   (44 B): f32 x y z;              f32 u0 v0 u1 v1; f32 normal, tangent, colour; f32 extra
faces
```

### Window v0
```
u8 version; f32 (16.0); u32; material; General big vertices (40 B); faces
```

### VegetationBark v0
```
u8 version; u32 flags; material
flags & 4 == 0 (40 B): f32 x y z; f32 u0 v0 (tiling bark); f32 u1 v1; f32 normal, tangent, bitangent
flags & 4 != 0 (32 B): f32 x y z; f32 normal, tangent, bitangent; f32 u v
faces
```

### VegetationFoliage v0
```
u8 version; f32 ×6; material; 40 B: f32 x y z; f32 u0 v0 u1 v1; f32 normal, tangent, bitangent; faces
```

### BillboardFoliage v0
```
u8 version; material; 28 B: f32 centre x y z; f32 u v; f32 offset x y; faces
```
The quad corners are `centre + offset` in view space (a billboard); the parser
applies the offset in the model's XY plane so the geometry has area.

### Halo v0
```
u8 version; material; 20 B: f32 x y z; u8 ×8 (colour / size, undecoded); faces
```

### Merged v0
```
u8 version; material; u8 mode; u32 flags
mode 1 (`*_base_N`, `*_part_N`): i32 n; n × { f32 x y z; f32 normal; f16 u0 v0; f16 u1 v1 }; faces
mode 0 (`*_batchN`):              i32 n; n × { f32 3×4 matrix (rows x, y, z with translation); u16 part; u16 extra }
```
Mode 1's vertex 0 is an in-memory header the exporter wrote out (faces index from
1). Mode 0 blocks carry no geometry: they are placements of `*_part*.rbm` models
from the same container, with the `.batchlod` text files describing their LODs.
Merged models have an all-zero bounding box.

## `.lod` descriptors and shared parts

Each container holds a `.lod` text file per part: five lines of model paths (one
per LOD level, `-` for none) and a trailing number. Only the file name matters.
Vehicle tyres, seats and steering wheels point at `Vehicles\04_SharedAssets\…`
models that live at the archive top level. Their placement on the vehicle is **not
in the container**: the `.epe` (a PCBB blackboard property file, now parsed) holds
only the `CCar` parameters, and the `.mvdoll` holds the part hierarchy
(`CReferenceProp` nodes such as `V046-T_L_ref`) with two hashes per node and no
transforms. The transforms are presumably in the physics setup; until that is
decoded the viewer loads only the container's own parts.

## Coordinates

Engine space is right-handed like Bevy's: X right, Y up, and a model's front at −Z
(`coordinates.md`). The triangles are counter-clockwise about their normals (3837 of 3837
on the jeep's body), Bevy's front-face rule, so the loader takes positions, normals and
indices as they are. (It used to mirror Z, which turned every model inside out.) The bus is 13.7 m long on Z,
3.9 m tall on Y, 3 m wide on X; parts (doors, windows) are stored already placed in
vehicle space, so loading every `lod1` model of a container assembles the vehicle.
Shared parts (tyres) are referenced through the `.lod` text files and live in
`Vehicles\04_SharedAssets\…` (top-level archive entries, by bare name).

## Sources

Block layouts started from Gibbed.Avalanche's `RenderBlockModel` (zlib licence) and
were corrected/extended against the game data as described above.
