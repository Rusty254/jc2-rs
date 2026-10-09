//! `jc2://` asset source: lets Bevy load assets straight out of the game's
//! archives through [`jc2_formats::GameFs`].
//!
//! Paths:
//! * `jc2://fe_bg_rico_dif.dds`                        — a file by name (dropzone, DLC, archives)
//! * `jc2://lave.v046_buscoach.ee/v046_body_dif.dds`   — a member of a SARC container
//!
//! zlib wrappers are removed transparently, so `.eez/.blz/...` behave like
//! their uncompressed twins.
//!
//! A member that a container does not hold itself is looked for at the top level
//! (shared vehicle parts) and then in the shared containers (`general.blz`: the
//! `g_*` textures, lamp posts and other props every location may reference).
//! Inflated containers are cached, since a world tile asks for hundreds of members.

use bevy::asset::io::{AssetReader, AssetReaderError, PathStream, Reader, VecReader};
use bevy::tasks::futures_lite::stream;
use jc2_formats::GameFs;
use jc2_formats::sarc::{SmallArchive, is_sarc};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Inflated containers kept around at once (the shared ones are the big ones:
/// `general.blz` and up to 15 `areasetNN.blz`).
const CACHE_SIZE: usize = 40;

struct Container {
    data: Vec<u8>,
    sarc: SmallArchive,
}

pub struct JcAssetReader {
    pub fs: Arc<GameFs>,
    /// Containers whose members any other container may reference by bare name:
    /// `general.blz`, then the regional area sets (`jc2_formats::areaset`).
    pub shared: Vec<String>,
    cache: Mutex<HashMap<String, Arc<Container>>>,
}

impl JcAssetReader {
    pub fn new(fs: Arc<GameFs>) -> Self {
        let mut shared = vec!["general.blz".to_string()];
        if let (Ok(bin), Ok(bmp)) = (
            fs.read_inflated("areasets.bin"),
            fs.read_inflated("areasets.bmp"),
        ) && let Ok(sets) = jc2_formats::areaset::AreaSets::parse(&bin, &bmp)
        {
            shared.extend(sets.regional_containers());
        } else {
            shared.extend((1..=15).map(|i| format!("areaset{i:02}.blz")));
        }
        shared.retain(|c| fs.contains(c));
        Self {
            fs,
            shared,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn split(path: &Path) -> (String, Option<String>) {
        let s = path.to_string_lossy().replace('\\', "/");
        // Location identities include folders. Split after the archive filename,
        // rather than treating the first directory as a container.
        for (i, _) in s.match_indices('/') {
            let prefix = s[..i].to_ascii_lowercase();
            if [".ee", ".eez", ".bl", ".blz", ".nl", ".nlz", ".fl", ".flz"]
                .iter()
                .any(|ext| prefix.ends_with(ext))
            {
                return (s[..i].to_string(), Some(s[i + 1..].to_string()));
            }
        }
        (s, None)
    }
    /// An inflated, parsed container by name; `None` when the file is missing or is
    /// not a SARC.
    fn container(&self, name: &str) -> Option<Arc<Container>> {
        let key = name.to_ascii_lowercase();
        if let Some(c) = self.cache.lock().unwrap().get(&key) {
            return Some(c.clone());
        }
        let data = self.fs.read_inflated(name).ok()?;
        if !is_sarc(&data) {
            return None;
        }
        let sarc = SmallArchive::parse(&data).ok()?;
        let c = Arc::new(Container { data, sarc });
        let mut cache = self.cache.lock().unwrap();
        if cache.len() >= CACHE_SIZE
            && let Some(victim) = cache.keys().next().cloned()
        {
            cache.remove(&victim);
        }
        cache.insert(key, c.clone());
        Some(c)
    }

    fn member(c: &Container, name: &str) -> Option<Result<Vec<u8>, AssetReaderError>> {
        let entry = c.sarc.get(name)?;
        Some(
            c.sarc
                .data(&c.data, entry)
                .and_then(|b| jc2_formats::compress::maybe_inflate(b).map(|b| b.into_owned()))
                .map_err(|e| AssetReaderError::Io(Arc::new(std::io::Error::other(e.to_string())))),
        )
    }

    fn load(&self, path: &Path) -> Result<Vec<u8>, AssetReaderError> {
        let (container, inner) = Self::split(path);
        let not_found = || AssetReaderError::NotFound(path.to_path_buf());
        let Some(inner) = inner else {
            return self.fs.read_inflated(&container).map_err(|_| not_found());
        };
        let c = self.container(&container).ok_or_else(not_found)?;
        if let Some(r) = Self::member(&c, &inner) {
            return r;
        }
        // Shared assets (window glass, light sets, tyres…) are referenced by bare
        // name from inside a container but live at the top level…
        if let Ok(d) = self.fs.read_inflated(&inner) {
            return Ok(d);
        }
        // …or in the shared containers.
        for shared in &self.shared {
            if shared.eq_ignore_ascii_case(&container) {
                continue;
            }
            if let Some(sc) = self.container(shared)
                && let Some(r) = Self::member(&sc, &inner)
            {
                return r;
            }
        }
        Err(not_found())
    }

    fn sarc_listing(&self, path: &Path) -> Option<Vec<PathBuf>> {
        let (container, inner) = Self::split(path);
        if inner.is_some() {
            return None;
        }
        let c = self.container(&container)?;
        Some(
            c.sarc
                .entries
                .iter()
                .map(|e| Path::new(&container).join(&e.name))
                .collect(),
        )
    }
}

impl AssetReader for JcAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        self.load(path).map(VecReader::new)
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        // No .meta files live in the archives; Bevy falls back to loader defaults.
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_path_buf()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        match self.sarc_listing(path) {
            Some(list) => {
                let s: Box<PathStream> = Box::new(stream::iter(list));
                Ok(s)
            }
            None => Err(AssetReaderError::NotFound(path.to_path_buf())),
        }
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(self.sarc_listing(path).is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a game folder: set JC2_TEST_DIR to one (a subset with pc4.* is enough).
    #[test]
    fn loads_sarc_member_from_archive() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = Arc::new(GameFs::open(dir).unwrap());
        let r = JcAssetReader::new(fs);
        let dds = r
            .load(Path::new("frontend.blz/fe_bg_rico_dif.dds"))
            .unwrap();
        assert!(
            dds.starts_with(b"DDS "),
            "expected a DDS, got {:?}",
            &dds[..4]
        );
        assert!(r.sarc_listing(Path::new("frontend.blz")).unwrap().len() > 10);
        assert!(
            r.load(Path::new("frontend.blz/does_not_exist.dds"))
                .is_err()
        );
        assert!(r.load(Path::new("nope.bin")).is_err());
        // a shared texture referenced from a tile container resolves through general.blz
        if r.fs.contains("general.blz") && r.fs.contains("31x14.nlz") {
            let t = r
                .load(Path::new("31x14.nlz/g_metal_panel_dif.dds"))
                .unwrap();
            assert!(t.starts_with(b"DDS "));
        }
        assert!(!r.cache.lock().unwrap().is_empty());
    }
}

#[cfg(test)]
mod location_path_tests {
    use super::*;
    #[test]
    fn nested_location_path_preserves_container_and_member_identity() {
        assert_eq!(
            JcAssetReader::split(Path::new(
                "locations/metropolis/houses.nlz/models/house.rbm"
            )),
            (
                "locations/metropolis/houses.nlz".into(),
                Some("models/house.rbm".into())
            )
        );
        assert_eq!(
            JcAssetReader::split(Path::new("locations/world.bin")),
            ("locations/world.bin".into(), None)
        );
    }
}
