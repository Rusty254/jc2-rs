//! Native D3D10 sampler states of the model material families (board c728494e, JC2-257;
//! research `research/formats/sampler-states.md`, probe `research/tools/sampler_state_probe.py`).
//!
//! Each render-block pass writes a packed word per texture slot (`FUN_0095e900` address mode,
//! `FUN_009203e0` bias + the two filter fields, `FUN_00970020` MinLOD + anisotropy) and
//! `FUN_00940170` turns it into a D3D10 `SAMPLER_DESC`:
//!
//! * address: three 3-bit codes, 1 = WRAP, 3 = CLAMP;
//! * filter fields `(f5, f6)`: field 7 on either side is `MIN_MAG_MIP_LINEAR` (0x15), promoted to
//!   `ANISOTROPIC` (0x55) when the slot's anisotropy exceeds 1; otherwise `min = LINEAR[f5]`,
//!   `mag = LINEAR[f6]`, `mip = MIP_LINEAR[f5]` (tables at 0xf22858 / 0xf2285c); MaxLOD is 0 for
//!   `f5 < 3` (no mip chain) and FLT_MAX otherwise;
//! * anisotropy: the user's `/aniso=n` setting (`ctx + 0xccc`, "0-7, default: 7") for the diffuse,
//!   normal and property slots of every model family, a fixed 1 elsewhere.
//!
//! The port used `ImageSamplerDescriptor { address: Repeat, ..default() }`, whose filters are all
//! NEAREST in Bevy: every magnified model texture was a visible grid of texels and the mip chain
//! was never blended. Retail never uses point filtering on these slots.

use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};

/// D3D10 address codes as the packed word stores them.
pub const WRAP: u8 = 1;
pub const CLAMP: u8 = 3;

/// `/aniso=n` default (0-7) as the executable's command-line help states it. The mapping from the
/// setting to `ctx + 0xccc` (the byte `FUN_00970020` writes into the 5-bit anisotropy field) was
/// not found: the raw value is assumed (INFERRED; research gap 1).
pub const ANISO_SETTING_DEFAULT: u16 = 7;

/// `PTR_00f22858[f * 2]`: the field selects linear (1) or point (0) for min / mag.
const LINEAR: [u8; 8] = [0, 0, 1, 1, 1, 0, 0, 1];
/// `DAT_00f2285c[f * 2]`: the min field also selects a linear (1) or point (0) mip filter.
const MIP_LINEAR: [u8; 8] = [0, 0, 0, 1, 0, 1, 0, 1];

/// One texture slot's sampler state as the pass setup writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotState {
    pub address: u8,
    /// `FUN_009203e0` fourth argument (bits 13-16): min filter and mip filter.
    pub min_field: u8,
    /// `FUN_009203e0` fifth argument (bits 17-20): mag filter.
    pub mag_field: u8,
    /// True when `FUN_00970020` receives the `/aniso` setting (`ctx + 0xccc`), false for the
    /// literal 1.
    pub uses_aniso_setting: bool,
}

/// The decoded D3D10 description (filter bits as `FUN_00940170` builds them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoded {
    pub filter: u32,
    pub min_linear: bool,
    pub mag_linear: bool,
    pub mip_linear: bool,
    pub anisotropy: u16,
    /// 0.0 when the min field is below 3 (the native desc clamps MaxLOD to the base level).
    pub max_lod: f32,
}

impl SlotState {
    pub const fn new(address: u8, min_field: u8, mag_field: u8, uses_aniso_setting: bool) -> Self {
        Self {
            address,
            min_field,
            mag_field,
            uses_aniso_setting,
        }
    }

    /// `FUN_00940170` for this slot with the given effective anisotropy.
    pub fn decode(self, aniso: u16) -> Decoded {
        let aniso = aniso.clamp(1, 16);
        let (f5, f6) = (self.min_field as usize, self.mag_field as usize);
        if f5 == 7 || f6 == 7 {
            return Decoded {
                filter: if aniso > 1 { 0x55 } else { 0x15 },
                min_linear: true,
                mag_linear: true,
                mip_linear: true,
                anisotropy: aniso,
                max_lod: f32::MAX,
            };
        }
        let (min, mag, mip) = (LINEAR[f5], LINEAR[f6], MIP_LINEAR[f5]);
        Decoded {
            filter: (u32::from(min) << 4) | (u32::from(mag) << 2) | u32::from(mip),
            min_linear: min == 1,
            mag_linear: mag == 1,
            mip_linear: mip == 1,
            anisotropy: 1,
            max_lod: if f5 < 3 { 0.0 } else { f32::MAX },
        }
    }

    /// The Bevy sampler for this slot at the given user anisotropy.
    pub fn sampler_with(self, aniso_setting: u16) -> ImageSampler {
        let d = self.decode(if self.uses_aniso_setting {
            aniso_setting
        } else {
            1
        });
        let address = match self.address {
            CLAMP => ImageAddressMode::ClampToEdge,
            _ => ImageAddressMode::Repeat,
        };
        let filter = |linear: bool| {
            if linear {
                ImageFilterMode::Linear
            } else {
                ImageFilterMode::Nearest
            }
        };
        ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: filter(d.mag_linear),
            min_filter: filter(d.min_linear),
            mipmap_filter: filter(d.mip_linear),
            lod_min_clamp: 0.0,
            lod_max_clamp: if d.max_lod == 0.0 { 0.0 } else { 32.0 },
            anisotropy_clamp: d.anisotropy,
            ..Default::default()
        })
    }

    /// The Bevy sampler at the native default anisotropy.
    pub fn sampler(self) -> ImageSampler {
        self.sampler_with(ANISO_SETTING_DEFAULT)
    }
}

/// Wrap, fields (7, 7), anisotropy from the setting: the diffuse / normal / property slots of
/// General, Bark, Foliage, Facade, Merged, SkinnedGeneral, CarPaint, Lambert, DecalSimple, Grass
/// (`FUN_0096eed0`, `FUN_0095c890`, `FUN_00971da0`, `FUN_0093dac0`, `FUN_00928d30`,
/// `FUN_0093f2e0`, `FUN_0093b800`, `FUN_00937ec0`, `FUN_0096bc90`, `FUN_00a821d0`).
pub const WRAP_ANISO: SlotState = SlotState::new(WRAP, 7, 7, true);
/// Wrap, fields (3, 2), anisotropy 1 (trilinear): BillboardFoliage, Window, DeformableWindow,
/// Leaves, the forest meshes, and the Bark dirt slots 2 and 3 (`FUN_0095c890`).
pub const WRAP_TRILINEAR: SlotState = SlotState::new(WRAP, 3, 2, false);
/// Wrap, fields (4, 2): linear min / mag with a POINT mip filter (0x14). VegetationFoliage slot 2
/// (the `*_aoc.dds` at UV1, `FUN_00971da0`) and the SkinnedGeneral slot 7.
pub const WRAP_LINEAR_MIP_POINT: SlotState = SlotState::new(WRAP, 4, 2, false);
/// Clamp, fields (4, 2): the environment cubes (General slot 4, `FUN_0096eed0`).
pub const CLAMP_LINEAR_MIP_POINT: SlotState = SlotState::new(CLAMP, 4, 2, false);

/// Block type names (as `RenderBlock::type_name`) whose first texture slot is trilinear, not
/// anisotropic (`FUN_0093deb0` BillboardFoliage, `FUN_00959ff0` Window, `FUN_0095a2d0`
/// DeformableWindow).
fn colour_is_trilinear(type_name: &str) -> bool {
    matches!(
        type_name,
        "BillboardFoliage" | "Window" | "DeformableWindow"
    )
}

/// Sampler of the UV0 diffuse (slot 0) of a render block family.
pub fn colour(type_name: &str) -> SlotState {
    if colour_is_trilinear(type_name) {
        WRAP_TRILINEAR
    } else {
        WRAP_ANISO
    }
}

/// Sampler of the UV0 normal map (slot 1) and of the General property map (slot 2): wrap,
/// anisotropic in every family that has them.
pub fn normal_or_property(type_name: &str) -> SlotState {
    colour(type_name)
}

/// Sampler of the UV1 ambient-occlusion map: VegetationFoliage slot 2 is WRAP with fields (4, 2)
/// (`FUN_00971da0`); the General family's slot 3 is wrap and anisotropic like slots 0 to 2.
pub fn ambient_occlusion(type_name: &str) -> SlotState {
    match type_name {
        "VegetationFoliage" => WRAP_LINEAR_MIP_POINT,
        _ => WRAP_ANISO,
    }
}

/// The MipLODBias the native pass writes for the family's slot (third argument of
/// `FUN_009203e0`). Only the Merged family (slots 0, 2 and 4) biases sharper, by -1.0
/// (`FUN_00928d30`). A wgpu sampler has no bias: it is a shader-side `textureSampleBias` (gap).
#[allow(dead_code)] // documents the native value; no consumer until the shader-side bias exists
pub fn lod_bias(type_name: &str, slot: u8) -> f32 {
    match (type_name, slot) {
        ("Merged", 0 | 2 | 4) => -1.0,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(s: ImageSampler) -> ImageSamplerDescriptor {
        match s {
            ImageSampler::Descriptor(d) => d,
            other => panic!("not a descriptor: {other:?}"),
        }
    }

    #[test]
    fn decodes_the_proven_d3d10_filters() {
        // general-terms-next.md section 1: (4, 2) is 0x14 MIN_MAG_LINEAR_MIP_POINT
        let d = CLAMP_LINEAR_MIP_POINT.decode(1);
        assert_eq!((d.filter, d.mip_linear, d.max_lod), (0x14, false, f32::MAX));
        // (7, 7) is trilinear at anisotropy 1 and ANISOTROPIC above it
        assert_eq!(WRAP_ANISO.decode(1).filter, 0x15);
        let a = WRAP_ANISO.decode(7);
        assert_eq!((a.filter, a.anisotropy), (0x55, 7));
        assert_eq!(WRAP_TRILINEAR.decode(1).filter, 0x15);
        // fields below 3 have no mip chain: (2, 2) bilinear, (1, 1) point
        let b = SlotState::new(CLAMP, 2, 2, false).decode(1);
        assert_eq!((b.filter, b.max_lod), (0x14, 0.0));
        let p = SlotState::new(WRAP, 1, 1, false).decode(1);
        assert_eq!((p.filter, p.max_lod), (0x00, 0.0));
        // (5, 2): point min, linear mag, linear mip
        assert_eq!(SlotState::new(WRAP, 5, 2, false).decode(1).filter, 0x05);
    }

    #[test]
    fn model_samplers_are_linear_wrapped_and_anisotropic() {
        let d = descriptor(colour("VegetationBark").sampler());
        assert_eq!(d.address_mode_u, ImageAddressMode::Repeat);
        assert_eq!(d.address_mode_v, ImageAddressMode::Repeat);
        assert_eq!(d.mag_filter, ImageFilterMode::Linear);
        assert_eq!(d.min_filter, ImageFilterMode::Linear);
        assert_eq!(d.mipmap_filter, ImageFilterMode::Linear);
        assert_eq!(d.anisotropy_clamp, ANISO_SETTING_DEFAULT);
        assert_eq!(d.lod_max_clamp, 32.0);
        // never the Bevy descriptor default (all NEAREST) that caused the blocky textures
        assert_ne!(d, ImageSamplerDescriptor::default());
    }

    #[test]
    fn family_slots_follow_the_native_setup_functions() {
        for t in ["General", "VegetationBark", "VegetationFoliage", "Facade", "Merged", "SkinnedGeneral", "CarPaint", "DecalSimple"] {
            assert_eq!(colour(t), WRAP_ANISO, "{t}");
        }
        for t in ["BillboardFoliage", "Window", "DeformableWindow"] {
            let d = descriptor(colour(t).sampler());
            assert_eq!(d.anisotropy_clamp, 1, "{t}");
            assert_eq!(d.mipmap_filter, ImageFilterMode::Linear, "{t}");
        }
        // the foliage AO atlas: wrap, linear, POINT mip (0x14)
        let ao = descriptor(ambient_occlusion("VegetationFoliage").sampler());
        assert_eq!(ao.address_mode_u, ImageAddressMode::Repeat);
        assert_eq!(ao.mipmap_filter, ImageFilterMode::Nearest);
        assert_eq!(ao.mag_filter, ImageFilterMode::Linear);
        assert_eq!(lod_bias("Merged", 2), -1.0);
        assert_eq!(lod_bias("General", 0), 0.0);
    }

    #[test]
    fn clamp_cube_and_non_mip_slots() {
        let c = descriptor(CLAMP_LINEAR_MIP_POINT.sampler());
        assert_eq!(c.address_mode_u, ImageAddressMode::ClampToEdge);
        assert_eq!(c.mipmap_filter, ImageFilterMode::Nearest);
        let single = descriptor(SlotState::new(CLAMP, 2, 2, false).sampler());
        assert_eq!(single.lod_max_clamp, 0.0);
    }

    #[test]
    fn anisotropy_is_wgpu_valid() {
        // wgpu rejects anisotropy with a non-linear filter and values outside 1..=16
        for s in [WRAP_ANISO, WRAP_TRILINEAR, WRAP_LINEAR_MIP_POINT] {
            for setting in [0u16, 1, 7, 16, 200] {
                let d = descriptor(s.sampler_with(setting));
                assert!((1..=16).contains(&d.anisotropy_clamp));
                if d.anisotropy_clamp > 1 {
                    assert_eq!(d.mag_filter, ImageFilterMode::Linear);
                    assert_eq!(d.min_filter, ImageFilterMode::Linear);
                    assert_eq!(d.mipmap_filter, ImageFilterMode::Linear);
                }
            }
        }
    }
}
