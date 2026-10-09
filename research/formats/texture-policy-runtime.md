# Ordinary RBM DDS colour views — JC2-079

The original load/view chain and exact fixture fingerprints are in
`material-parameters-next.md` (JC2-025). This implementation applies that bounded
policy to General, CarPaint, VegetationBark and VegetationFoliage colour images.
Existing RGB normal maps retain their independent linear image values. Foliage
alpha, fade, wind, MPM channels, exact original material shaders and broader
colour policy remain separate work.

`texture_policy.rs` decodes the authored `Settings/gammacorrect.bin` root string
list into the original lowercased-name hashes. It rejects malformed node layouts,
non-string entries and unsupported names rather than accepting unrelated strings.
The callback uses the unchanged material texture name: exact lowercase UI prefixes
first, backslash-only basename stripping, ASCII lowercase, `_dif` immediately
before the last dot, then exception hash membership. Path normalization happens
separately for archive lookup. `_diff` does not match. Unicode/512-byte names are
explicitly unsupported because retail's locale-dependent byte behavior is unproven.

Only verified legacy ordinary 2D DDS DXT1 and DXT5 format pairs are selected:
71/72 and 77/78, respectively. The header must identify an ordinary image, not
a cubemap/volume; pixel/mip decoding remains Bevy's image loader's responsibility.
DX10 headers, DXT3, uncompressed DDS and other formats/families retain the legacy
sRGB loader behavior with diagnostics. No arbitrary numeric format increment is
performed. No resource format is inferred from material trailer3.

`RbmLoader::default()` uses authored exceptions. `RbmLoader::new` accepts
`GammaMode::OnlyTextureNames` for the original name-only configuration; the
viewer currently registers the default mode and adds no new CLI setting. Missing
or malformed authored settings emit a warning and retain an explicit name-only
fallback limitation on the selected view. A suffix-proven fixture can still be
selected when the exception set is unavailable. This is a renderer fallback,
not proof that a missing authored set was the original configuration.

The exception cache belongs to one loader and is keyed by full asset-source
identity. It does not share settings between independently mounted asset servers
or source namespaces. Concurrent initial loads can race to read the small settings
file before one cache entry wins; subsequent loads reuse it. Cache entries describe
the fixed mount, including failures. A changed settings/dropzone mount requires
a fresh loader/application; dynamic exception-file hot reload is unsupported.

Colour loading retains both the raw name and full source-qualified lookup path.
Within each RBM, cached handles use both values and the material family, preserving
differing raw-policy inputs and verified-family eligibility even when path lookup
is equivalent. Verified and unsupported blocks cannot inherit each other's view
or fallback diagnostics in either block order. `load_value` supplies explicit image
settings; each value receives an RBM-local `colourN` label. This prevents Bevy's
path-only deferred handle identity from aliasing different colour interpretations,
normal/data views or an independently loaded default-sRGB image. Labels are
per-model, so identical colour bytes in different models may occupy independent
assets; this follows the accepted normal-view isolation approach. This change
does not introduce a global image cache or rewrite the UI/terrain asset loaders.

`RbmBlock.diffuse_view` records raw name, lookup path, source/selected DXGI format,
requested sRGB setting and any fallback. Debug logs also report actual Bevy
resource/view format. A decoder disagreement is diagnosed. Missing or corrupt
requested diffuse images return a load error naming the raw texture and resolved
path, preserving failed model readiness instead of publishing an untextured
material. The existing verified normal
family/tangent safety checks and geometric-normal fallback are retained.

Real-data tests use the installed archives via `JC2_TEST_DIR`:

- Hook `hook_diff` stays71; a simultaneously default-loaded copy stays sRGB,
  with equal pixels/dimensions and distinct handles.
- Jeep body and pine bark select72; pine foliage selects78. Their verified
  normal images remain linear and cannot share colour handles.
- Normal, MPM and AOC DDS fixtures stay71 under the original authored list.
- Two asset sources with different authored exception sets produce different
  hook views in one app; sequential later model loads reuse each source's set.
- Missing/malformed exception assets retain a diagnosed name-only result.
- Explicit `OnlyTextureNames` mode skips the authored exception asset entirely.
- A virtual RBM retaining retail bus CarPaint/window block payloads points both
  families at the real hook DDS; both block orders retain distinct linear/legacy
  sRGB handles and their own diagnostics.
- Missing and corrupt real-hook diffuse overrides fail model readiness; the
  separate missing-normal regression still permits geometric normals.
- Exact prefix/case/path/suffix/hash rules and unsupported DDS kinds are checked.

Validation on 8 October 2026 used isolation47, the absolute shared target/build
lock and the real installed game through `JC2_TEST_DIR`. Decoder5 and loader14
passed, as did targeted scenery3964-instance, boat and weapon-ready fixtures.
The final release workspace run passed383 tests (148 formats +235 game), strict
all-target workspace clippy with `-D warnings`, format check and release workspace
build. The private `target/jc2-texture47.exe` copy was made under the held lock.

Existing fleet (two constructors), character, npc and scenery headless fixtures
now register a BC-capable ImageLoader. Concrete original failures were missing
image-loader matching for required bus/boat and scenery DDS dependencies; the
previous deferred handles allowed model publication without decoding those files.
These are test-only registration changes; required diffuse failure semantics and
normal fallback remain intact. A Windows path assertion was normalized, and
strict clippy required naming the raw/path/family cache identity ColourImageKey.

The controlled GPU harness is
`scripts/check-materials.ps1 -IncludeHook`, extending the existing ten pine/bus
cases with the hook at light yaw30/140, camera yaw25/pitch35 and depth prepass.
Source commit `f84f3e41862afb9af6c45df6208e7dee95168d2c` was integrated by the
coordinator as `da5a7dd`; the evidence below comes from that source checkpoint,
not the subsequently combined main branch.

Controlled GPU verification used the private executable SHA256
`30351c74fc5b858cbbe7e7d6879539e36d889f4454ceedaf920e3f52e99d50f5`,
AMD Radeon RX9070XT/Vulkan driver26.8.1, 1280x720 images and18-second fixtures.
The sole renderer slot was held through the sequence, then explicitly released.
Reproduction from the worktree:

```powershell
$env:RUST_LOG='info,jc2::rbm_loader=debug'
& ./scripts/check-materials.ps1 -Executable 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/target/jc2-texture47.exe' -GameDirectory 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2' -OutputDirectory 'C:/Program Files (x86)/Steam/steamapps/common/Just Cause 2/jc2-rs/target/material-texture47' -IncludeHook
```

All twelve cases produced fresh loaded-model screenshots and successful exits.
Actual resource/view diagnostics in the `.err` logs confirm:

| Fixture colour | Original selected DXGI | Actual Bevy resource/view |
| --- | --- | --- |
| Pine bark | 71->72 | Bc1RgbaUnormSrgb |
| Pine foliage | 77->78 | Bc3RgbaUnormSrgb |
| Bus CarPaint (three maps) | 71->72 | Bc1RgbaUnormSrgb |
| Hook at light yaw30 and140 | 71->71 | Bc1RgbaUnorm |
| Bus DeformableWindow | Unverified family, explicit legacy fallback | Bc1RgbaUnormSrgb |

The exception count is32; verified maps have no policy limitation. No renderer
error occurred. The bus window's one expected unsupported-family warning appears
per bus case. Inspected pine, bus and both hook images have loaded geometry,
visible texture/light response and shadows. The hook has138 triangles and remains
a linear view under both lights; this does not establish original-game shader parity.

Scene-only pixel comparisons crop away HUD rows0..59, then count RGB pixels with
at least one differing channel. All ten final pine/bus captures exactly match the
previous `target/material-acceptance10` acceptance images. Pine main versus depth
also differs by0 pixels. Normal-versus-flat differences are13236/10609 pixels for
pine at the two light angles and81829/145620 for the bus, preserving the accepted
normal response. Hook light-angle differences include the ground and shadow, so
no object-only quantitative claim is inferred from that comparison.

The first `pine-main` capture had different camera framing (151976 scene pixels
against the accepted image). Its original PNG/log/stderr are retained as
`pine-main-first.*`. Repeating only that case with identical settings produced the
exact baseline/main-depth result. The cause of the startup framing variation is
unproven; the capture was repeated because it could not support a like-for-like
comparison. Images and logs are ignored local evidence and are not committed.

This checkpoint proves the bounded original colour-view selection and preserves
the accepted normal/alpha baseline. It does not implement original General,
CarPaint or foliage shaders, full MPM/alpha/fade behavior, unsupported DDS format
pairs, dynamic exception hot reload, or original-game GPU parity. JC2-024 owns the
recovered foliage instance flags/fade implementation.
