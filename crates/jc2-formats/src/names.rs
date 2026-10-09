//! Hash → name recovery.
//!
//! The archives only store hashes, so names come from (a) the bundled lists,
//! (b) names found inside the data itself (SARC entries, string values in
//! property containers), and (c) anything you add. `harvest` feeds (b).

use crate::hash::{file_name, hash_str, name_hash};
use std::collections::HashMap;

const FILE_NAMES: &str = include_str!("../data/file_names.txt");
const PROPERTY_NAMES: &str = include_str!("../data/property_names.txt");
const SOUND_PROPERTY_NAMES: &str = include_str!("../data/sound_property_names.txt");

#[derive(Clone, Debug, Default)]
pub struct NameTable {
    /// archive key (hash of bare file name) -> full path as listed
    files: HashMap<u32, String>,
    /// hash of a property key name -> name
    props: HashMap<u32, String>,
}

impl NameTable {
    pub fn empty() -> Self {
        Self::default()
    }

    /// The bundled lists: ~25k file names, ~700 property names.
    pub fn bundled() -> Self {
        let mut t = Self::default();
        t.add_file_list(FILE_NAMES);
        t.add_property_list(PROPERTY_NAMES);
        t.add_property_list(SOUND_PROPERTY_NAMES);
        t
    }

    pub fn add_file_list(&mut self, text: &str) {
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with(';') || l.starts_with('#') {
                continue;
            }
            self.add_file(l);
        }
    }

    pub fn add_property_list(&mut self, text: &str) {
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with(';') || l.starts_with('#') {
                continue;
            }
            self.add_property(l);
        }
    }

    /// Register a file path; returns true if it was new. Keeps the first
    /// (usually most descriptive) path seen for a given file name.
    pub fn add_file(&mut self, path: &str) -> bool {
        let h = name_hash(path);
        if let Some(existing) = self.files.get(&h) {
            // prefer a path with a directory over a bare name
            if !existing.contains(['\\', '/']) && path.contains(['\\', '/']) {
                self.files.insert(h, path.to_string());
            }
            return false;
        }
        self.files.insert(h, path.to_string());
        true
    }

    pub fn add_property(&mut self, name: &str) -> bool {
        self.props
            .insert(hash_str(name), name.to_string())
            .is_none()
    }

    pub fn file(&self, hash: u32) -> Option<&str> {
        self.files.get(&hash).map(String::as_str)
    }

    /// Bare file name for a hash, if known.
    pub fn file_short(&self, hash: u32) -> Option<&str> {
        self.file(hash).map(file_name)
    }

    pub fn property(&self, hash: u32) -> Option<&str> {
        self.props.get(&hash).map(String::as_str)
    }

    /// `name` if known, else `unknown/<hash>.<ext>` so output stays unique.
    pub fn file_or_placeholder(&self, hash: u32, ext: &str) -> String {
        match self.file(hash) {
            Some(n) => n.to_string(),
            None => format!("__unknown\\{hash:08x}.{ext}"),
        }
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
    pub fn property_count(&self) -> usize {
        self.props.len()
    }

    /// Pull candidate names out of a blob: SARC entry names and string values
    /// inside property containers (which often reference other files).
    pub fn harvest(&mut self, data: &[u8]) -> usize {
        let mut added = 0;
        if crate::sarc::is_sarc(data) {
            if let Ok(s) = crate::sarc::SmallArchive::parse(data) {
                for e in &s.entries {
                    if self.add_file(&e.name) {
                        added += 1;
                    }
                    if let Ok(inner) = s.data(data, e) {
                        added += self.harvest(inner);
                    }
                }
            }
        } else if crate::compress::is_zlib(data) {
            if let Ok(raw) = crate::compress::inflate(data) {
                added += self.harvest(&raw);
            }
        } else if crate::property::looks_like_property(data)
            && let Ok(pf) = crate::property::PropertyFile::parse(data)
        {
            for n in &pf.nodes {
                n.walk(&mut |node| {
                    for (k, v) in &node.values {
                        if let crate::property::Key::Name(name) = k
                            && self.add_property(name)
                        {
                            added += 1;
                        }
                        if let crate::property::Variant::Str(s) = v
                            && looks_like_file_ref(s)
                            && self.add_file(s)
                        {
                            added += 1;
                        }
                    }
                    for (k, _) in &node.children {
                        if let crate::property::Key::Name(name) = k
                            && self.add_property(name)
                        {
                            added += 1;
                        }
                    }
                });
            }
        }
        added
    }
}

/// A string value that is plausibly a file reference: has an extension and
/// no spaces, e.g. `exported/vehicles/lave/lave.v046_buscoach.ee`.
pub fn looks_like_file_ref(s: &str) -> bool {
    let name = file_name(s);
    s.len() < 260
        && !s.contains(' ')
        && name.contains('.')
        && name
            .rsplit('.')
            .next()
            .map(|e| {
                (1..=12).contains(&e.len())
                    && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_lists_load() {
        let t = NameTable::bundled();
        assert!(t.file_count() > 25_000);
        assert!(t.property_count() > 600);
        assert_eq!(
            t.file_short(name_hash("gui\\credits.blz")),
            Some("credits.blz")
        );
    }

    #[test]
    fn file_ref_heuristic() {
        assert!(looks_like_file_ref(
            "exported/vehicles/lave/lave.v046_buscoach.ee"
        ));
        assert!(looks_like_file_ref("camera_settings.bin"));
        assert!(!looks_like_file_ref("PLANE_PITCH_UP"));
        assert!(!looks_like_file_ref("Hello world."));
    }
}
