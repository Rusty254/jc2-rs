//! Ordinary RBM colour views proven by retail 00946080 -> 009619d0 -> 00950420.
//! Exact callback-input names are separate from archive lookup paths. Only
//! legacy 2D DXT1/DXT5 format pairs are supported; no general format+1 rule.

use crate::property::{PropertyFile, Variant};
use crate::{Error, Result, hash::hash_str};
use std::collections::HashSet;

pub const GAMMA_CORRECT_FILE: &str = "Settings/gammacorrect.bin";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GammaMode {
    #[default]
    AuthoredExceptions,
    /// Original UseMode=OnlyTextureNames: do not load the exception set.
    OnlyTextureNames,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameRule {
    ExcludedPrefix,
    DiffuseSuffix,
    AuthoredException,
    LinearName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NameSelection {
    pub is_srgb: bool,
    pub rule: NameRule,
}

#[derive(Clone, Debug, Default)]
pub struct GammaExceptions {
    hashes: HashSet<u32>,
}

impl GammaExceptions {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        if file.nodes.len() != 1 {
            return Err(Error::format("gamma exceptions require one outer node"));
        }
        let outer = &file.nodes[0];
        if !outer.values.is_empty()
            || outer.children.len() != 1
            || outer.children[0].0.hash() != hash_str("root")
            || !outer.children[0].1.children.is_empty()
        {
            return Err(Error::format("unsupported gamma exception node layout"));
        }
        let mut hashes = HashSet::new();
        for (_, value) in &outer.children[0].1.values {
            let Variant::Str(name) = value else {
                return Err(Error::format(
                    "gamma exception must be a texture-name string",
                ));
            };
            valid_name(name)?;
            hashes.insert(hash_str(&name.to_ascii_lowercase()));
        }
        Ok(Self { hashes })
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn select(&self, raw_name: &str) -> Result<NameSelection> {
        valid_name(raw_name)?;
        let rule = if ["gui_", "hud_", "fe_", "pda_"]
            .iter()
            .any(|prefix| raw_name.starts_with(prefix))
        {
            NameRule::ExcludedPrefix
        } else {
            // The original strips only backslashes, after prefix exclusion.
            let basename = raw_name.rsplit('\\').next().unwrap().to_ascii_lowercase();
            if basename
                .rfind('.')
                .is_some_and(|dot| basename[..dot].ends_with("_dif"))
            {
                NameRule::DiffuseSuffix
            } else if self.hashes.contains(&hash_str(&basename)) {
                NameRule::AuthoredException
            } else {
                NameRule::LinearName
            }
        };
        Ok(NameSelection {
            is_srgb: matches!(rule, NameRule::DiffuseSuffix | NameRule::AuthoredException),
            rule,
        })
    }
}

fn valid_name(name: &str) -> Result<()> {
    // Retail uses a 512-byte temporary and locale-dependent byte tolower.
    // Do not pretend to reproduce that for Unicode or overlong names.
    if name.is_empty() || !name.is_ascii() || name.len() >= 512 || name.contains('\0') {
        return Err(Error::format("unsupported gamma-policy texture name"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DdsViewPair {
    pub source_dxgi: u32,
    pub selected_dxgi: u32,
}

/// Read enough metadata to identify a proven ordinary resource/view pair.
/// Pixel/mip payload validation still belongs to the image loader.
pub fn dds_view_pair(bytes: &[u8], is_srgb: bool) -> Result<DdsViewPair> {
    let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if bytes.len() < 128 || &bytes[..4] != b"DDS " {
        return Err(Error::format("colour policy needs a legacy DDS header"));
    }
    if word(4) != 124 || word(76) != 32 || word(12) == 0 || word(16) == 0 {
        return Err(Error::format("invalid ordinary DDS header"));
    }
    if word(24) > 1 || word(28) > 32 || word(112) & 0x0020_fe00 != 0 {
        return Err(Error::format("array/cube/volume DDS policy is unverified"));
    }
    if word(80) & 4 == 0 {
        return Err(Error::format(
            "uncompressed DDS colour policy is unverified",
        ));
    }
    let source_dxgi = match &bytes[84..88] {
        b"DXT1" => 71,
        b"DXT5" => 77,
        _ => return Err(Error::format("DDS colour format pair is unverified")),
    };
    Ok(DdsViewPair {
        source_dxgi,
        selected_dxgi: match (source_dxgi, is_srgb) {
            (71, false) => 71,
            (71, true) => 72,
            (77, false) => 77,
            (77, true) => 78,
            _ => unreachable!(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::{Key, Node};

    #[test]
    fn preserves_exact_prefix_suffix_case_and_backslash_order() {
        let policy = GammaExceptions::default();
        for (name, srgb) in [
            ("gui_x_dif.dds", false),
            ("hud_x_dif.dds", false),
            ("fe_x_dif.dds", false),
            ("pda_x_dif.dds", false),
            ("GUI_x_dif.dds", true),
            ("dir\\gui_x_dif.dds", true),
            ("HOOK_DIFF.dds", false),
            ("v014_body_DIF.dds", true),
            ("nrm.dds", false),
            ("x_dif", false),
        ] {
            assert_eq!(policy.select(name).unwrap().is_srgb, srgb, "{name}");
        }
        assert!(policy.select("é_dif.dds").is_err());
        assert!(policy.select(&"a".repeat(512)).is_err());
    }

    #[test]
    fn original_exception_hashing_does_not_normalize_forward_slashes() {
        let policy = GammaExceptions {
            hashes: HashSet::from([hash_str("flare.dds")]),
        };
        assert_eq!(
            policy.select("FLARE.dds").unwrap().rule,
            NameRule::AuthoredException
        );
        assert!(policy.select("dir\\flare.dds").unwrap().is_srgb);
        assert!(!policy.select("dir/flare.dds").unwrap().is_srgb);
        assert!(!policy.select("gui_flare.dds").unwrap().is_srgb);
    }

    #[test]
    fn rejects_nonordinary_or_unproven_dds_pairs() {
        let mut data = vec![0; 128];
        data[..4].copy_from_slice(b"DDS ");
        for (offset, value) in [(4, 124_u32), (12, 4), (16, 4), (76, 32), (80, 4)] {
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (fourcc, linear, srgb) in [(b"DXT1", 71, 72), (b"DXT5", 77, 78)] {
            data[84..88].copy_from_slice(fourcc);
            assert_eq!(dds_view_pair(&data, false).unwrap().selected_dxgi, linear);
            assert_eq!(dds_view_pair(&data, true).unwrap().selected_dxgi, srgb);
        }
        for fourcc in [b"DX10", b"DXT3", b"ATI2"] {
            data[84..88].copy_from_slice(fourcc);
            assert!(dds_view_pair(&data, true).is_err());
        }
        data[84..88].copy_from_slice(b"DXT1");
        data[112..116].copy_from_slice(&0x200_u32.to_le_bytes());
        assert!(dds_view_pair(&data, false).is_err());
        assert!(dds_view_pair(&data[..100], false).is_err());
    }

    #[test]
    fn requires_original_exception_structure_and_string_values() {
        let wrong = PropertyFile {
            nodes: vec![Node {
                values: vec![(Key::Hash(1), Variant::Str("flare.dds".into()))],
                ..Default::default()
            }],
        };
        assert!(GammaExceptions::parse(&wrong.to_bytes()).is_err());
        let non_string = PropertyFile {
            nodes: vec![Node {
                children: vec![(
                    Key::Name("root".into()),
                    Node {
                        values: vec![(Key::Hash(1), Variant::Int(1))],
                        ..Default::default()
                    },
                )],
                ..Default::default()
            }],
        };
        assert!(GammaExceptions::parse(&non_string.to_bytes()).is_err());
    }

    #[test]
    fn retail_exceptions_and_colour_data_fixtures() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let policy =
            GammaExceptions::parse(&fs.read_inflated(GAMMA_CORRECT_FILE).unwrap()).unwrap();
        assert_eq!(policy.len(), 32);
        assert_eq!(
            policy.select("FLARE.dds").unwrap().rule,
            NameRule::AuthoredException
        );
        for (container, name, source, selected) in [
            ("general.blz", "hook_diff.dds", 71, 71),
            ("general.blz", "hook_nrm.dds", 71, 71),
            ("general.blz", "hook_mpm.dds", 71, 71),
            ("lave.v014_jeep.ee", "v014_body_dif.dds", 71, 72),
            ("lave.v014_jeep.ee", "v014_body_mpm.dds", 71, 71),
            ("vegetation_1.blz", "arctic_pine_bark_dif.dds", 71, 72),
            ("vegetation_1.blz", "arctic_pine_bark_nrm.dds", 71, 71),
            ("vegetation_1.blz", "arctic_pine_foliage_dif.dds", 77, 78),
            ("vegetation_1.blz", "arctic_t22_foliage_aoc.dds", 71, 71),
        ] {
            let bytes = fs.read_inflated(container).unwrap();
            let archive = crate::sarc::SmallArchive::parse(&bytes).unwrap();
            let data = archive.data(&bytes, archive.get(name).unwrap()).unwrap();
            let pair = dds_view_pair(data, policy.select(name).unwrap().is_srgb).unwrap();
            assert_eq!(
                pair,
                DdsViewPair {
                    source_dxgi: source,
                    selected_dxgi: selected
                },
                "{name}"
            );
        }
    }
}
