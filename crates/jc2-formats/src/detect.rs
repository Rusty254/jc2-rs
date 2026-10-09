//! Identify a blob by its leading bytes. Extensions in the archives are only
//! known through the name lists, so the content check is the source of truth.

use crate::{compress, property, sarc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// zlib stream, usually wrapping a SARC
    Zlib,
    Sarc,
    /// binary property container (.bin/.asb/.epe/.pfs/.blo/...)
    Property,
    /// RBMDL render-block model
    Rbm,
    /// "PCBB" blackboard property container (.epe and editor exports)
    Blackboard,
    Dds,
    /// Anark Gameface UI binary
    Anark,
    /// Avalanche Data Format (" FDA" / "ADF ")
    Adf,
    /// Havok packfile (0x57E0E057 tag)
    Havok,
    /// FMOD event project / sound bank
    Fev,
    Fsb,
    /// DirectX shader bundle (CTAB constant table)
    Shader,
    Xml,
    Bmp,
    Bink,
    Csv,
    Text,
    Empty,
    Unknown,
}

impl FileKind {
    pub fn extension(self) -> &'static str {
        match self {
            FileKind::Zlib => "z",
            FileKind::Sarc => "sarc",
            FileKind::Property => "bin",
            FileKind::Rbm => "rbm",
            FileKind::Blackboard => "epe",
            FileKind::Dds => "dds",
            FileKind::Anark => "agui",
            FileKind::Adf => "adf",
            FileKind::Havok => "hkx",
            FileKind::Fev => "fev",
            FileKind::Fsb => "fsb",
            FileKind::Shader => "shader",
            FileKind::Xml => "xml",
            FileKind::Bmp => "bmp",
            FileKind::Bink => "bik",
            FileKind::Csv => "csv",
            FileKind::Text => "txt",
            FileKind::Empty => "empty",
            FileKind::Unknown => "bin",
        }
    }
}

pub fn detect(data: &[u8]) -> FileKind {
    if data.is_empty() {
        return FileKind::Empty;
    }
    if compress::is_zlib(data) {
        return FileKind::Zlib;
    }
    if sarc::is_sarc(data) {
        return FileKind::Sarc;
    }
    if data.starts_with(b"DDS ") {
        return FileKind::Dds;
    }
    if crate::rbm::is_rbm(data) {
        return FileKind::Rbm;
    }
    if property::is_blackboard(data) {
        return FileKind::Blackboard;
    }
    if data.starts_with(b"FEV1") || data.starts_with(b"FEV") {
        return FileKind::Fev;
    }
    if data.starts_with(b"FSB") {
        return FileKind::Fsb;
    }
    if data.starts_with(b"BIK") {
        return FileKind::Bink;
    }
    if data.starts_with(b"BM") && data.len() > 14 {
        return FileKind::Bmp;
    }
    if data.starts_with(b" FDA") || data.starts_with(b"ADF ") {
        return FileKind::Adf;
    }
    if data.len() >= 4 && data[0..4] == [0x57, 0xE0, 0xE0, 0x57] {
        return FileKind::Havok;
    }
    if data.len() >= 16
        && data[0..4] == [0; 4]
        && data[4..8] == [0x1c, 0, 0, 0]
        && &data[8..16] == b"AnarkBGF"
    {
        return FileKind::Anark;
    }
    if data.len() >= 12 && &data[8..12] == b"CTAB" {
        return FileKind::Shader;
    }
    if data.starts_with(b"<?xml") || data.starts_with(b"<object") {
        return FileKind::Xml;
    }
    if property::looks_like_property(data) {
        return FileKind::Property;
    }
    let sample = &data[..data.len().min(512)];
    if sample
        .iter()
        .all(|&b| b == b'\t' || b == b'\n' || b == b'\r' || (0x20..0x7f).contains(&b))
    {
        return if sample.contains(&b',') {
            FileKind::Csv
        } else {
            FileKind::Text
        };
    }
    FileKind::Unknown
}
