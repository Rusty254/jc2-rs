# Cloud shadows and Sun attenuation: native consumers (JC2-096)

Status: research brief. No runtime code changed. Probe:
`research/tools/cloud_shadow_consumers_probe.py` (read-only; 28 checks plus numeric tables).

## Headline

1. **The PC executable has no consumer of `settings\cloudshadows.bin` that this research could find.** The
   file is in the archives, but the exe contains neither its name nor any of its 11 key hashes.
2. **FastTer `t11` is not a cloud-shadow map.** It is the static global texture
   `Landscape\AmbientOcclusionMap.dds`. Its `.x` channel is ambient occlusion, and its `.y`/`.w`
   channels are terrain-horizon heights for the two sun sides.
3. **The Sun attenuation that FastTer applies is terrain self-shadowing from that map.** It
   depends only on the hour (through the light direction), and never on cloud cover or
   elapsed time. The traced native chain has no cloud-driven Sun attenuation.

The project's assumption (in `weather.md`) that t11 is the cloud shadow, and its
`intensity * cover` policy, therefore have no native counterpart in the traced chain. In
the original, the cloud system's density field drives **rain**. It does not drive the sun
(see §4).

## Fingerprints

| Input | sha256 |
|---|---|
| `JustCause2.exe` (14,549,656 B) | `df99a479354cbd46ed322bcbd6651938c39fc1e7432734586486dd9f53b74f80` |
| `DX10_Shaders_F.shader_bundle` | `db8c67a0672668623d7bf9222c008523083b9f4bd01c6665f8dea8cebc292e98` |
| `DX10_SpecialShaders_F.shader_bundle` (only `constantalpha*`, `depthshader`) | `3a1d45857fe6b3e88c9633325a8676e33366ce35e0ef820fb8fb7aba2ea3506d` |
| `textures\landscape\ambientocclusionmap.dds` (pc0, 1,048,704 B) | `8e8bd8345680690a68d1eca8cd508a04bbb4f123baf9f7c7c8bad73415178505` |
| `settings\cloudshadows.bin` (pc2, 120 B) | `19e7613e1b933f35d8fb14c8913979ce9efb179114c51242bfc6113972c01d9c` |

The archive files were extracted to a private scratch directory with a copied
`jc2tool extract --filter`. They are not committed.

## 1. `cloudshadows.bin` has no native reader (verified negative, with a control)

* The exe has no `cloudshadow`/`cloud_shadow` text, case-insensitive, in ASCII or UTF-16LE.
  `strings.csv` agrees.
* The exe has none of the 11 key hashes (`005615e6` … `f1c8d843`, `dd25aaff`) as raw u32.
  **Positive control:** the `weathersettings.bin` keys that `FUN_00a52d20` reads are present
  in the same scan at `DAT_00f0ead4` (`476306ad`), `+4` (`c4af68f5`) and `DAT_00f0eaf4`
  (`97d853b2`). The engine looks properties up by these stored hashes, so a reader of
  `cloudshadows.bin` would leave its hashes in the binary.
* The raw scan of `pc0..pc4.arc` finds `c55669a6` and `dd25aaff` only inside `cloudshadows.bin`
  itself (pc2 `0x2cd6504c`/`0x2cd65062`). A single pc1 `dd25aaff` hit lies inside texel data.
  No textual `cloudshadow` occurs anywhere in the raw archives.
  *Limit:* zlib-compressed payloads were not inflated for this scan.
* No shader in either bundle is named `*cloudshadow*` (785 names).

Native zero-cover and "disabled" behaviour follow from this. Because nothing reads the file,
the 0.85 `cloud_shadow_intensity` and the ten unnamed keys have **no effect** on PC rendering.
That holds for every cover value, and the unknown keys stay unlabeled. The file is plausibly a
leftover of a cross-platform or console build. That is an inference: no other platform binary
was examined.

## 2. Primary chain: settings → map → Sun consumer (verified)

_(decompiler excerpt omitted from the public snapshot)_

`FUN_00958860` is the pixel-stage register file (`DAT_013d9f80`, table at `ctx+0x50`).
`FUN_0096beb0` is the vertex-stage one (`DAT_013dfb00`). Buffer index 0 is cb0. The FastTer
PS declares only `cb0[145]`, so for it every register is this global lighting block.

### Producer: `cb0[0]` (`FUN_00a58a00`, decompiled lines ~7470–7560)

Let `L` be the unit light direction (pointing away from the sun):

```
floor = min(|L.y| · 20, 1) · 0.15            → cb0[10].w   (from the unflipped copy +0x254)
k     = 0.70710677                            (DAT_0108ae60._4_4_, the axis y after normalisation)
if L.y > 0:  L = −L;  k = k²                  (sun below horizon: use the opposite body, the moon)
L.y   = min(L.y, 0.01); L = normalize(L)      (the clamp cannot fire after the flip: recorded no-op)
(a,b) = L.x ≥ 0 ? (0, 30) : (30, 0)           (30.0 = rdata 0x00e24c30)
c     = −L.y / k · 30
cb0[0] = (a, b, c, 1)
```

### Consumer: the FastTer pixel shader (`fastter_lod0shnone`, verified by `sm4dis`)

```
t = t11.Sample(s11, v1.zw)                        // v1.zw = world.xz / 32768 + 0.5 (terrain.md VS)
shadow = max(saturate(cb0[0].z − dot(t.wy, cb0[0].xy)), cb0[10].w)
sun    = cb0[3].rgb · saturate(dot(N, −cb0[4])) · shadow · (t.x · 0.5 + 0.5)
amb    = lerp(cb0[11], cb0[10], saturate(N.y·0.5 + 0.5)) · t.x · ao_hollow
```

So `shadow = max(saturate(30·(|L.y|/k − h)), floor)`. Here `h = t11.y` when `L.x ≥ 0` (the sun
on the −x side), else `t11.w`. The map stores the sine of the terrain horizon divided by
`k = sin 45°`, the noon elevation, and 1/30 of that unit is the penumbra width.

The same `dot(t11.wy, cb0[0].xy)` term occurs in **608** of the 614 shaders that declare
`t11` (bark, car paint, foliage, characters, buildings, roads, water). It is global, not
terrain-only.

### The map (verified from the fixture)

`ambientocclusionmap.dds` is 1024 × 1024 DXT5 with no mips, so one texel covers 32 m of the
32,768 m world. The sampler clamps and filters linearly.

| Channel | Min | Mean | Max | Notes |
|---|---|---|---|---|
| x (R) | 0 | 0.833 | 1.0 | ambient occlusion |
| y (G) | 0 | 0.154 | 1.0 | horizon height, one side |
| z (B) | 0 | 0 | 0 | unused |
| w (A) | 0 | 0.162 | 0.984 | horizon height, other side |

`corr(y, w)` is 0.089: the two directional horizons are largely independent, as expected.
**Inference:** which of y and w belongs to east and which to west depends on the
rotation-sign assumption already recorded in `weather.md`. With the morning sun on +x, `L.x < 0`
in the morning, so `t11.w` is the eastern horizon and `t11.y` the western one.

## 3. Units, defaults and numeric reproduction (probe output)

Units: cb0[0].xy are dimensionless gains (30). cb0[0].z is `30·sin(elev)/sin45°`. cb0[10].w
is a fraction of the sun term. t11 is UNORM.

Defaults:
* `CRenderEngine` constructor values: identity-like `+0x110 = (0,1,0,0)`, `+0x130 = (0,0,0,1)`.
  These are from `FUN_00939130`'s pattern; *its class identity was not confirmed*.
* Runtime `DAT_0108c9a8` initial value: 0.15.

| hour | sun y | cb0[0] | cb0[10].w |
|---|---|---|---|
| 5:30 | 0.0052 | (30, 0, 0.220, 1) | 0.0156 |
| 5:42 | 0.0375 | (30, 0, 1.592, 1) | 0.1126 |
| 6:00 | 0.0862 | (30, 0, 3.659, 1) | 0.15 |
| 8:00 | 0.3967 | (30, 0, 16.83, 1) | 0.15 |
| 12:00 | 0.7071 | (0, 30, 30.00, 1) | 0.15 |
| 16:00 | 0.3967 | (0, 30, 16.83, 1) | 0.15 |
| 18:18 | 0.0375 | (0, 30, 1.592, 1) | 0.1126 |
| 0:00 (moon) | −0.535 | (0, 30, 32.08, 1) | 0.15 |

Sun factor (`shadow · (x/2 + 1/2)`) for sample texels (x, y, w):

| Texel (x, y, w) | Noon | 16:00 | 8:00 |
|---|---|---|---|
| (1, 0, 0), open ground | 1 | 1 | 1 |
| (1, 1, 1), fully shadowed | 0.15 (floor) | 0.15 | 0.15 |
| (0, 0.2, 0.9) | 0.5 | 0.5 | 0.075 (eastern horizon above the sun) |

**Cover and elapsed time.** Cover 0, 0.42 and 1, and elapsed time 0, 1 h and 1e7 s, give
bit-identical results, because the chain has no such input. The only time input is the hour.
Long uptime therefore cannot drift this term.

## 4. What cloud cover does drive natively (verified, partial)


  ```
  (cover·(1 + 0.5·night) − 1 + N((p.x − 1·t)/30000, (p.y − 0.1·t)·1e-4, (p.z − 1·t)/30000) − 0.1) · 12
  ```

  * `cover` is the cloud object's `+0x85c`, which `weathersettings` `#97d853b2` (0.42) sets.
  * `night` is `saturate((|hour−12| − 8)·0.3)`.
  * `t` is the parameter, or `+0x854` when it is negative.
  * `N` is `FUN_004dcb70`, a 3-D noise whose implementation was **not** decoded.

  Its five callers place clouds and cards.
  `max(y, cloud_base)` to `cloud_base + cloud_height`. It then maps the result through
  `saturate((Σ·step·0.0002 − 0.9)·5)`. `FUN_00a6db30` smooths this into `+0x200` and multiplies
  it by `+0x870` to give the **rain intensity** (`precip+0x48`). Nothing in this path writes
  cb0[0], cb0[3] or the light object.
* Cloud cover reaches the Sun only if the per-hour colour tables (`+0x660 … +0x1dd0`, three
  weather variants) select by weather. Those tables feed `weather+0x204..+0x234` (sun colour
  and ambient). Their writer was **not** traced, so this remains open (§6).
  > **Correction (JC2-115, implemented in JC2-122):** the writer is traced in `night-light-next.md` section 2: the fields live in the sky object S = W+0x10 and
  > are written every frame from `gamezone_gradients.bmp` (not from these six tables) through `FUN_0089f9d0`.

## 5. Other FastTer light constants (separate from clouds; for the terrain work)

| cb0 | Producer (`FUN_0095f020`) | Origin |
|---|---|---|
| [0] | CRenderEngine+0x130 | §2, `FUN_0091f870` from `FUN_00a58a00` |
| [4] | light+0x44 | light direction `L` (§2) |
| [5], [6] | camera `+0x54..`, `+0x44..` with w = 1 | camera vectors |
| [7], [8] | light-grid origin (`DAT_011907c4+0x29cc`, triple-buffered), `(0.25, 64, 1/w, 1/h)` | point-light grid (t10) |
| [9] | `(flag, 0, 1 − f, f)` from `vtbl+0x54` object `+0x1f4` | not identified |
| [10] | CRenderEngine+0x110 xyz, w = `DAT_0108c9a8` | sky ambient = `weather+0x224` (clamped 0..1 by `FUN_00a585a0`) |
| [11] | CRenderEngine+0x120 | ground ambient = `weather+0x234` (`+0x214..+0x220 × 4`, plus lightning `·20`) |
| [13], [14] | `memcpy(DAT_013db230/640, …)` from `DAT_011907c4+0x10/+0x420` | fog / underwater (copied wholesale, not traced) |
| [15+i], [80+i] | same `DAT_011907c4` light lists | point lights |

**Unresolved:** where `weather+0x204..+0x234` come from. They come from the per-hour tables,
but that writer was not traced. This is still the "per-hour light tables" gap in `weather.md`.
> **Correction (JC2-115, implemented in JC2-122):** resolved in `night-light-next.md` section 2: S = W+0x10, written by `FUN_0089f9d0` from the `gamezone_gradients.bmp`
> blend (`FUN_00a5d390`); sun/moon colour = row 0 through the `((x ** (1/2.2)) * 2.2) ** 2.2` transform, `cb0[10]` = row 4, `cb0[11]` = row 5.

## 6. Verified vs. inferred vs. unresolved

* **Verified (probe):**
  * The negative scans and their positive control.
  * The t11 = `AmbientOcclusionMap.dds` slot-11 registration.
  * The cb0 upload size and source.
  * The `FUN_0095f020` ECX = CRenderEngine.
  * The four setter calls from `FUN_00a58a00` and the 30.0 constant.
  * The axis `(0,1,1)` and the 0.15 floor.
  * The FastTer PS instructions, and the counts of 614/608 shaders.
  * The fixture hashes and the AO channel statistics.
* **Verified (pseudo-C reading, not machine-checked):**
  * The `cb0[0]` arithmetic order, the flip and the no-op clamp.
  * The `cb0[10].w` formula.
  * The cloud density and column formulas, and their rain consumer.
* **Inferred:**
  * The y/w east–west assignment (it depends on `weather.md`'s assumed rotation sign).
  * The light at night is the moon.
  * `cloudshadows.bin` is a non-PC leftover.
* **Unresolved:**
  * The noise `FUN_004dcb70`.
  * The writer of `weather+0x204..+0x234` (sun colour and ambient tables) and whether the
    weather variant depends on cover. *(Writer found: `night-light-next.md`; the variant weights are still open.)*
  * cb0[9], [13], [14] in detail.
  * Compressed archive payloads were not scanned for references.

## 7. Bounded handoff for JC2-022

1. Remove the idea of a cloud-shadow map for parity. The native PC renderer applies **no**
   cloud shadow on the sun. Any cloud shadow would be a deliberate non-native enhancement and
   must be labelled as one. The project's "zero cover → exact transmittance 1, no work"
   contract is *consistent* with native, because native has transmittance 1 at every cover.
2. The native Sun attenuation to implement is the **terrain horizon shadow**:
   * Load `textures\landscape\ambientocclusionmap.dds` once and keep it for the whole session.
     It is static.
   * Sample it at `world.xz/32768 + 0.5` with clamp and linear filtering.
   * Compute `cb0[0]` and the floor on the CPU from the light direction every frame (§2
     formulas, the probe's `cb0_terms`).
   * Multiply **only the sun's** direct term by `max(saturate(c − 30·h), floor)·(ao/2 + 1/2)`.
   * Multiply ambient by `ao` (`t11.x`).
   * Leave Fill, Moon (it is a separate Bevy light; native flips the single light), point
     lights and fog untouched.
3. Hook: Bevy's directional-light accumulation, as in `weather.md` §"Next graphics agent". The
   sun light needs an explicit identifier. Keep cascade shadows.
4. Acceptance fixtures:
   * The probe tables in §3. Cover-independence: 0 / 0.42 / 1 must give bit-identical frames
     for the terrain lighting.
   * The 5:30–6:00 floor ramp.
   * Morning versus afternoon must sample different channels (w versus y).
5. Out of scope here: per-hour colours and ambient (tables untraced), and the rain amount from
   the density column (§4). The latter needs `FUN_004dcb70`.
