//! `.lod` descriptors: small text files inside model containers that map LOD
//! levels to `.rbm` models.
//!
//! ```text
//! vehicles\01_landvehicles\v046_lave_special\vehicle_merge\v046-body_m_LOD1.rbm
//! vehicles\01_landvehicles\v046_lave_special\vehicle_merge\v046-body_m_LOD1.rbm
//! vehicles\01_landvehicles\v046_lave_special\vehicle_merge\v046-body_m_LOD3.rbm
//! vehicles\01_landvehicles\v046_lave_special\vehicle_merge\v046-body_m_LOD4.rbm
//! vehicles\01_landvehicles\v046_lave_special\vehicle_merge\v046-body_m_LOD4.rbm
//! 0.5
//! ```
//!
//! One line per LOD level (five levels in the shipped data; `-` means none),
//! then an optional trailing number. Only the file name matters: the model is
//! looked up by name hash, in the same container first, then at the top level
//! (shared tyres, seats and steering wheels live in `Vehicles\04_SharedAssets`).

use crate::hash::file_name;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LodDescriptor {
    /// Model file name per LOD level, outermost detail first.
    pub levels: Vec<Option<String>>,
    pub trailing: Option<f32>,
}

impl LodDescriptor {
    pub fn parse(text: &str) -> Self {
        let mut levels = Vec::new();
        let mut trailing = None;
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            if let Ok(v) = l.parse::<f32>() {
                trailing = Some(v);
                continue;
            }
            if l == "-" {
                levels.push(None);
            } else {
                levels.push(Some(file_name(l).to_string()));
            }
        }
        Self { levels, trailing }
    }

    /// Model for a 1-based LOD level, falling back to the nearest coarser level
    /// that exists.
    pub fn model_for_level(&self, level: usize) -> Option<&str> {
        let idx = level.max(1) - 1;
        self.levels
            .get(idx)
            .and_then(|l| l.as_deref())
            .or_else(|| self.levels.iter().skip(idx).find_map(|l| l.as_deref()))
            .or_else(|| self.levels.iter().rev().find_map(|l| l.as_deref()))
    }

    pub fn looks_like_lod(text: &[u8]) -> bool {
        let s = String::from_utf8_lossy(text);
        let mut lines = s.lines().map(str::trim).filter(|l| !l.is_empty());
        matches!(lines.next(), Some(l) if l == "-" || l.to_ascii_lowercase().ends_with(".rbm"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vehicle_lod() {
        let t = "a\\b\\Body_LOD1.rbm\r\na\\b\\Body_LOD1.rbm\r\n-\r\na\\b\\Body_LOD4.rbm\r\n-\r\n0.5\r\n";
        let d = LodDescriptor::parse(t);
        assert_eq!(d.levels.len(), 5);
        assert_eq!(d.trailing, Some(0.5));
        assert_eq!(d.model_for_level(1), Some("Body_LOD1.rbm"));
        assert_eq!(d.model_for_level(3), Some("Body_LOD4.rbm"));
        assert_eq!(d.model_for_level(5), Some("Body_LOD4.rbm"));
        assert!(LodDescriptor::looks_like_lod(t.as_bytes()));
        assert!(!LodDescriptor::looks_like_lod(b"DDS "));
    }
}
