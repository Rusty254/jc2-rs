# General material, remaining terms: env cubemap, specular colour, snow, shadow receive, instance constants (JC2-110)

Status: research brief. No runtime code changed. Probe: `research/tools/general_terms_probe.py` (111 checks, exits 0;
`python -B research/tools/general_terms_probe.py <game> --jc2tool <jc2tool.exe> [--export <pseudo-C dir>]`). Addresses are
runtime addresses of `JustCause2.exe` (no ASLR). PS/VS names follow `general_main.wgsl`: PS 6062008, VS 188280 of
`DX10_Shaders_F.shader_bundle`. Builds on `material-parameters-next.md` (JC2-025/085), `night-light-next.md` (cb0[3]/[10]/[11]),
`haze-fog-next.md` (fog), `light-tables-next.md`, `cloud-shadow-consumers-next.md`.

## Headline

1. **t4 (MPM-R environment cube) is one static file, `textures\genericenvcube_dif.dds`**, for every zone, hour and weather. No dynamic
   producer, no per-zone choice. It is bound once per General pass; it is sampled with the world-space reflection vector, no matrix.
2. **cb0[1].xyz (the specular colour) is `4 x` gradient row 1** (`gamezone_gradients.bmp` row 1, the "colour 2" of `night-light-next.md`
   section 3), written to `light+0x28` every frame. This closes `night-light-next.md` gap 5 and `light-tables-next.md` G6 for the
   consumer: the "unused" row 1 is the General specular colour. It is almost zone independent (about 3.0 white at noon, 0.8 grey-green at night).
3. **Snow is not a global term.** The General shaders contain no snow constant in `cb0`. The only snow input is a per-instance byte
   (instance +0xc) gated by render flag 0x40. The shader maths and constants are proven (section 3); the writer of that byte is **not found** (Gap 1).
4. Corrections to `general_material.rs`: `v6.w` is not 0 but the instance LOD fade; the shadow-receive strength `cb1[2].w` scales only the
   terrain **horizon** shadow, not the cascade shadow; `cb0[1]` is solved.

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `DX10_Shaders_F.shader_bundle` (7,582,748 B) | `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98` |
| General VS 188280 (3,152 B) | `5ed378106af191c52b4fc20e3dbd33de6b94f39c808b89622f691b5b427cbf61` |
| General PS 6062008 (13,128 B) | `e69515b373591104be072f18b9600ba7d85eaf2a149072988737e5ce62ec632d` |
| `textures\genericenvcube_dif.dds` (pc0, 49,280 B) | `bfe8db3800c140dda3db974a80e72397a68b772487d5e0aa9776757014146e33` |
| `textures\glossyenvcube_dif.dds` (pc0, 49,280 B) | `0ffa7fffda0ff728d4661d91dc7148f61c0c59cec0073b3ca263dda4ac456faf` |
| `textures\landscape\gamezone_gradients.bmp` | `b43e8c1b56f53a981861ee0377c26ab5f2d151f800fcec73495601b3f1fcb329` |

Files were read with `jc2tool cat`; none is committed.

## 1. The environment cube (t4)

**Shader (proven, PS listing).** `t4` is a `TextureCube` (`dcl_resource_sat t4, dim=6`) with sampler `s4`. With `V = normalize(cb0[5].xyz - world)`
and `N` the world normal after the normal map:

```
R    = reflect(-V, N)                        # world space; sampled directly, no rotation
refl = sat(MPM.r * (1 - sat(N.V)^8)) * cube(R).rgb * (cb0[3].rgb + cb0[10].rgb)
rgb  = C * L + refl                           # then + specular (section 2)
```

`cb0[3]` is the sun/moon colour and `cb0[10]` the sky ambient (both already native, JC2-115/122), so the missing reflection needs no new provider.
The cube's alpha is unused.

**Binding (proven, bytes + decompile).** The General type's per-pass setup `FUN_0096eed0` (vtable pointer `0xf23c8c`) calls
`ctx+0xcbc` is filled by the per-frame snapshot `FUN_009426f0`: `ctx[+0xcbc] = tm->slot2()->vtbl[0x14]()`, `ctx[+0xcc0] = tm->slot3()->vtbl[0x14]()`
with `tm = DAT_01190b50` (CTextureManager) and slots 2/3 = `0x93b7a0` / `0x93d890`, which return `tm+0x54` / `tm+0x5c`. Those two shared_ptr
fields are loaded once by the IAppSystem initialiser `FUN_0096e040` (secondary vtable `0xe5cc9c` slot 2; its `this` is `tm+4`, hence its `+0x50`/`+0x58`):
`+0x50` = `GetCubeTexture("GenericEnvCube_dif.dds")` (string `0xe5c99c`, CTextureManager vtable `0xf228bc` slot 13 = `FUN_00942ac0`), `+0x58` = `"GlossyEnvCube_dif.dds"`
(`0xe5d3c4`). Nothing else writes `ctx+0xcbc` except the context copy `FUN_0094f3f0`; there is no zone/hour/dynamic selection.
The Glossy cube goes to slot 7 for the SkinnedGeneral block (`FUN_00931080`, vtable `0xf22ee0`); Merged (`FUN_00928d30`), the block at `0xf23a74` and Window (slot 5) use the same snapshot.

**Asset (proven).** 128 x 128 DXT1 (`DXT1` FourCC), `caps2 = 0xfe00` (all six faces), mip field 0, payload 6 x 8192 B. Face means (DDS order +X -X +Y -Y +Z -Z): +Y
(168, 194, 224) blue sky, -Y (58, 48, 36) brown ground, sides about (104, 125, 125). The Glossy cube is a darker, high-contrast one (means 38..112).

**Sampler s4 (proven by bytes + table arithmetic).** `FUN_0096eed0` sets address mode (3,3,3) = CLAMP (slots 0..3 use (1,1,1) = WRAP), packed filter fields (4, 2),
aniso 1. `FUN_00940170` turns the pair (4,2) into D3D10 filter `0x14` = MIN_MAG_LINEAR_MIP_POINT using the table at `0xf22858` (probe re-computes it); MaxLOD
is FLT_MAX for field >= 3. Slots 0..3 use field 7 = anisotropic (the user's anisotropy setting `ctx+0xccc`).

**Colour space (decompile; inferred from absence).** The 2D loader `FUN_009619d0` asks the manager's name callback (`vtbl+0x58` = `0x946080`; `_dif` => format + 1 = sRGB view,
`texture-policy-runtime.md`). The **cube loader `FUN_00966560` never touches `DAT_01190b50`**: it passes the `D3DX10GetImageInfoFromMemory` format unchanged to the cube
creator `FUN_009741a0` (TEXTURECUBE misc flag 4, array size 6, bind flags 8) and builds the SRV with explicit format 0 (= resource format, `FUN_0091c630`). A DXT1 file is
`BC1_UNORM` (71): **the cube is sampled as linear UNORM, not sRGB, in spite of `_dif`** (probe: pseudo-C absence check). Do not give the Bevy image an sRGB view.

## 2. The specular colour cb0[1] (MPM-G)

**Shader (proven).** `rgb += cb0[1].rgb * pow(max(dot(N, H), 0.001), SpecularPower) * shadow * MPM.g * (1 - v6.w)` with `H = normalize(V - cb0[4])`
(`cb0[4]` is the light direction pointing away from the sun, so `-cb0[4]` is the vector to the sun or moon), `shadow` = (cascade term) x (horizon term, section 4),
`SpecularPower = cb2[0].x`. There is no `N.L` factor; the highlight is not clamped to the lit side.

**Writer chain (proven, bytes).**
_(decompiler excerpt omitted from the public snapshot)_
Row 1 is *used unchanged* by `FUN_00a5d390` (only vector 0, the sun, gets the `x**(1/2.2)*2.2` transform), so `cb0[1].xyz = 4 * row1` where `row1`
is `light_state(...)["row1"]` of `night_light_probe.py`. Underwater/height/rain/snow sets 8..11 use the same vectors (not modelled here, Gap 7).

**Values (probe-asserted, steady state, zone intensity 3; row 1 does not depend on intensity).**

| Zone set | 0 h | 6 h | 9 h | 12 h | 15 h | 18 h | 21 h |
|---|---|---|---|---|---|---|---|
| 0,1,2,4..8 (Arctic = 1, Desert = 2, Default = 7) | 0.776 0.863 0.848 | 2.720 1.928 1.086 | 2.949 2.609 1.928 | 3.037 3.037 2.920 | 2.949 2.609 1.928 | 2.720 1.928 1.086 | 0.776 0.863 0.848 |
| 3 Jungle | same | same | 1.814 2.318 1.725 | 2.167 2.528 2.318 | 1.814 2.318 1.725 | same | same |

(Sun colour at Arctic noon is 1.92 1.76 1.38, so the old stand-in `SPEC_STANDIN` was 1.6x too dim and the wrong hue.) Cutscenes can override S+0x204..0x240 (`night-light-next.md` section 7).

## 3. The snow term and the instance constants

**Shader (proven).** `r0.x` at the snow lines is still the first channel of the AO value (`t3`, or its channel dot):
```
x = sat(cb1[11].x * v3.w + AO.r)            # cb1[11].x = +1 / -1 / 0 (PS); v3.w = fade^2 (VS cb1[11].y)
s = sat((N.y + x - 1.7 - cb1[1].w) * 5)     # N = world normal after the normal map
rgb = lerp(rgb, cb1[1].rgb * L, s)          # L = r4: ambient + sun + B remap + point lights; BEFORE fog
```
Fixtures (probe): N.y 1, AO 1, fade 0: f = 0.3 -> s = 0, f = 0.4 -> 0.5, f = 0.45 -> 1; N.y 0.5, f = 0.45 -> 0.

**Constants (proven, `FUN_009188e0`, vtable `0xf23d10`, called per instance).** Gate: block render flag 0x40 (`test byte [block+0x7c],0x40` at `0x918be9`) AND instance byte
`+0xc` (`f = byte/255`) non-zero. Otherwise `cb1[1] = (0,0,0,1)`, `cb1[11].x = 0`, so `s = 0`.
* `f < 0.5`: `cb1[1] = (0.508, 0.54, 0.645, 1 - 2f)`, `cb1[11].x = +1` (snow white-blue; floats at `0xe5bc80`, `0xe5c208`, `0xe45a68`).
* `f >= 0.5`: `f' = f - 0.5`, `cb1[1] = (0.2932, 0.2011, 0.1247, 1 - 2f')`, `cb1[11].x = -1` (brown; `0xe5d268`, `0xe5beb8`, `0xe5d5f4`). Its meaning (mud/wet earth?) is not named in the binary.

**Other per-instance constants written by the same function (proven).**

| Instance byte | PS/VS register | Use |
|---|---|---|
| `+0xb` | VS `cb1[11].w` -> `v9.w` | **instance alpha**; the vertex-colour alpha is not read at all (`mov o9.w, cb1[11].w`). Alpha test: `diffuse.a * v9.w - cb4[4].x < 0` discards when `cb4[3].w`; `o1.w = diffuse.a * v9.w` |
| `+0xf` | PS `cb1[2] = (0, 1, 2, b/255)`, VS `cb1[15]` same vector | `.w` = shadow-receive strength (section 4); VS uses `cb1[15].xyz = (0,1,2)` as the cascade-matrix indices |
| `+0x14..0x16`, `+0x17` | PS `cb1[0] = (rgb*a, 1-a)` | **instance tint after fog**: `out = fogged * cb1[0].w + cb1[0].xyz` = `lerp(fogged, tint, a)` (`a = byte/255`) |
| `+0x1c` mode, `+0x1e` value | VS `cb1[11] = (fade, fade^2, 0, alpha)` | LOD cross-fade: mode 0 -> 0, mode 1 -> `max(v - 0.75, 0) * 4` (`v = byte/255`; 0.75 at `0xe30d18`), else 1. `v6.w = fade` -> specular `x (1 - fade)`; `v3.w = fade^2` -> snow mask |

The instance record is double-buffered (`DAT_0108c598 & 1` for the fade bytes; matrices at `+0x44 + DAT_0108c59c * 0x24`).

**Where the snow byte comes from: not found.** Evidence collected for the follow-up: the General shaders read no weather constant; `cb0` slots read by the PS are
[0] [1] [3] [4] [5] [7] [8] [9] [10] [11] [12].w [13] [14] [15+i] [80+i] only. `FUN_00a40080` (rain/snow weights, `light-tables-next.md` section 3) has direct callers only in weather code
plus the wrappers `FUN_006b63b0` (CMaterialSystem vtable `0xeb91e8` slot 4, returns rain and snow), `FUN_007f0990`, `FUN_008b4900`. Whether the Arctic zone snow factor
(`haze-fog-next.md`, snow target table: Arctic = 1) reaches instance +0xc is **unresolved**. Until then the runtime must keep "no snow" (JC2-085 behaviour).

## 4. Shadow receive and the in-shader shadow path

* `cb1[2].w` (instance +0xf / 255) scales **only the horizon term**: `h = max(sat(cb0[0].z - dot(t11.wy, cb0[0].xy)), cb0[10].w); h' = 1 + w (h - 1); shadow = h' * c`
  with `c` the cascade term `r0.y`. So `w = 0` ignores the terrain horizon shadow, but the cascade shadow always applies. The default byte is unknown (Gap 1); JC2-085's 1.0 is still an assumption.
* Cascade term (proven by listing; structure only): enabled by `cb4[3].x`, soft mode by `cb4[3].y`; if disabled `c = 1`. The three cascades are vertical tiles of one atlas `t12`
  (UV transforms `x*10-5` and `y*30-{5,15,25}`; edge distance = Chebyshev distance, 5 at the tile edge). Soft mode: 13 blocker taps (`s13`, scale 70, radius 0.008) and a
  12-tap rotated Poisson PCF for cascade 0, 4-tap PCF (offsets in `cb0[12].w` texels) for cascades 1 and 2, rotation from a screen-space dither
  `fract(dot(frag.xy, (0.467944, -0.703648)))`, radius fades over the last unit of the tile (3.9..4.9). Hard mode: dithered cascade pick at distance 4.95, 4-tap PCF. End:
  `c = sat(sat(d2 - 4) + lit) * cb0[9].z + cb0[9].w)`, i.e. the far fade then a floor.
* `cb0[9] = (1, 0, 1 - f, f)` with `f = [shadow object + 0x1f4]` (`FUN_0095f020`, object from `(*DAT_0118fbdc + 0x54)()`), `f` is initialised to **0.1** by `FUN_0091a6e0`
  (float at `0xe290bc`); other writers were not found, so the cascade shadow floor is 0.1 of the sun (inferred). `cb4[3].x/.y` = shadow object bytes +8/+9 (`FUN_009734a0`
  stores `DAT_013d9e20/e24`; the `cb4` base `0x013d9df0` is inferred from the four-for-four match of `e20 e24 e2c e30` with `cb4[3].x .y .w` and `cb4[4].x`).
* Cascade matrices (VS `cb0[16 + 4i .. 19 + 4i]`, selected by VS `cb1[15].xyz = (0,1,2)`) and their producer: Gap 2.

## 5. Point lights, fog, instance colour (confirmation)

* **Point lights (proven in the listing, producer open).** `cell = (world.xz - cb0[7].xz) * cb0[8].x + cb0[8].y` (cell 4 m, offset 64, `cb0[8].xy = (0.25, 64)`), `ld` from `t6` (integer, 4 light
  indices per texel, 0 = none; the second pair only when the third index is non-zero). Per light: `d = cb0[15+i].xyz - p`, `w = sat(1 - d.d * cb0[15+i].w)^2`,
  `c = sat(w * (d.N) / |d|)`, `sum += c * cb0[80+i].xyz`. `L = (ambient * AO + sun N.L shadow)`, `L += B (2 - L)`, **then** `L += sum`: the B remap does not touch point lights.
  Light lists are `memcpy`'d from `DAT_011907c4 + 0x10 / + 0x420` (0x410 B each, 64 lights + a reserved slot 0); the grid texture producer is open (Gap 3).
* **Fog (matches `haze-fog-next.md`).** The General VS exponent and PS blend are exactly the section 4 formulas (probe: line-for-line); the underwater blend uses `v2.z` and is skipped when
  `v2.w < 0`. Order in the PS: lighting -> reflection -> specular -> snow -> fog (`cb0[13]`) -> underwater (`cb0[14]`) -> instance tint -> alpha test. Fog for General is JC2-155's.
* `o0.w = MPM.b * 0.1` and a second render target `o1 = (rgb, diffuse.a * instance alpha)` exist; their consumers are open (Gap 4).

## Proven / Inferred

* **Proven (bytes, listings, decompile):** sections 1 to 3 shader maths; cb0[1] chain; t4 binding path and file; sampler s4; instance byte map; fog identity; light-grid maths.
* **Inferred:** cube sampled as UNORM (absence of the name callback in the cube loader); `f = 0.1` shadow floor; `cb4` layout; cascade atlas interpretation; the brown variant of the byte.
* **Not claimed:** snow provider, default instance bytes, shadow matrices, light-grid contents, variants.

## Bounded Rust handoff (not implemented)

1. `general_material.rs`: load `textures/genericenvcube_dif.dds` as a 6-layer cube image with **non-sRGB BC1** and a clamp, linear, no-mip sampler; bind it as binding 108/109 and add
   `refl` above, using the native `cb0[3]`, `cb0[10]` values of `light_state` through the exposure identity.
2. Expose `row1` from the atmosphere state (`night_light_probe.light_state` has it) and use `4 * row1` as the specular colour; remove the `JC2_GENERAL_SPEC_STANDIN` stand-in.
   Specular factor `x (1 - lod_fade)` (0 until instance LOD fade exists), `H = normalize(V + to_sun)`, no `N.L`.
3. Apply `shadow_receive` only to the horizon factor; keep it 1 until Gap 1; cascade shadow floor `0.1 + 0.9 c` (inferred).
4. Keep snow off; implement `s`/`cb1[1]` maths behind a per-instance `snow` field defaulting to 0 so the provider can be dropped in.
5. Instance tint after fog (`mix(fogged, tint.rgb, tint.a)`, default `a = 0`) and instance alpha in the alpha test, again defaulting to the identity.
6. Tests: CPU mirrors of section 1 (reflection weight), section 2 (table above) and section 3 (snow fixtures).

## Gaps

   Blocks snow (Arctic props), receive default, tint, LOD fade. Leads: section 3 last paragraph.
2. **Shadow settings**: writers of `[shadow object + 0x1f4]` (cb0[9]) other than the 0.1 default, the source of `cb4[3].x/.y` (user options?), cascade matrices (VS `cb0[16..27]`), split distances, atlas size and format of `t12`, the meaning of `cb0[12].w`. Blocks an exact cascade/PCF port.
3. **Point-light grid** (`t6` producer, light-list fields, `CRenderEngine+0x830` scale, `light-tables-next.md` G5). Blocks point lights on General.
   AnimateTexture 0x80 (VS `cb1[8..10]` affine UV matrix: `FUN_00972ce0` writes identity only when the flag is clear; the animator that writes it is not found), instance alpha != 1 beyond the alpha test, channel textures + alpha test (preZ/shadow shaders test raw `t0` alpha).
5. **Env cube**: runtime capture to confirm UNORM sampling; the fallback cube used when `FUN_00966560` fails; whether `CCubeTexture` vtable +0x14 has the same SRV wrapper layout as `CTexture`.
6. **MPM alpha / framebuffer alpha** `o0.w` consumer.
7. **Row 1 in the height / underwater / rain / snow branches** of `FUN_00a5d390` (inherits `night-light-next.md` gap 2) and under cutscene override.
8. **Brown instance variant** (byte >= 128): what authored system uses it.
