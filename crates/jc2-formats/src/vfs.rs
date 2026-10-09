//! `GameFs`: the game's virtual file system as the engine sees it.
//!
//! Mounts every `archives_win32\*.tab` + `.arc` pair and every `DLC\*.tab`,
//! and the flat `dropzone\` override folder. Files are addressed by name
//! hash (hash of the bare file name). Precedence: dropzone, then archives in
//! reverse mount order (so DLC and later packs override the base game).

use crate::compress;
use crate::error::{Error, Result};
use crate::hash::name_hash;
use crate::names::NameTable;
use crate::sarc::SmallArchive;
use crate::tab::{ArchiveTable, TabEntry};
use memmap2::Mmap;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

/// SARC containers whose members are looked up by bare name by [`GameFs::read_any`].
pub const MEMBER_CONTAINERS: &[&str] = &["animations.blz"];

pub struct Archive {
    /// e.g. `archives_win32/pc0` or `DLC/pc_60`
    pub name: String,
    pub tab_path: PathBuf,
    pub arc_path: PathBuf,
    pub table: ArchiveTable,
    data: Mmap,
}

impl Archive {
    pub fn open(tab_path: &Path) -> Result<Self> {
        let arc_path = tab_path.with_extension("arc");
        let tab_bytes = std::fs::read(tab_path).map_err(|e| Error::io(tab_path, e))?;
        let table = ArchiveTable::parse(&tab_bytes)?;
        let file = File::open(&arc_path).map_err(|e| Error::io(&arc_path, e))?;
        // SAFETY: the archive files are treated as read-only; the game is not
        // expected to rewrite them while we have them mapped.
        let data = unsafe { Mmap::map(&file) }.map_err(|e| Error::io(&arc_path, e))?;
        let name = {
            let parent = tab_path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let stem = tab_path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if parent.is_empty() {
                stem
            } else {
                format!("{parent}/{stem}")
            }
        };
        Ok(Self {
            name,
            tab_path: tab_path.to_path_buf(),
            arc_path,
            table,
            data,
        })
    }

    pub fn slice(&self, e: &TabEntry) -> Result<&[u8]> {
        let (o, s) = (e.offset as usize, e.size as usize);
        self.data.get(o..o + s).ok_or(Error::Truncated {
            offset: o,
            needed: s,
            len: self.data.len(),
        })
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Dropzone,
    Archive { archive: usize, entry: usize },
}

#[derive(Clone, Debug)]
pub struct Located {
    pub name_hash: u32,
    pub size: u64,
    pub source: Source,
}

pub struct GameFs {
    pub root: PathBuf,
    pub archives: Vec<Archive>,
    pub dropzone: Option<PathBuf>,
    dropzone_files: HashMap<u32, PathBuf>,
    index: HashMap<u32, (usize, usize)>,
    pub names: NameTable,
}

impl GameFs {
    /// Mount a game install (the folder containing `JustCause2.exe`).
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let mut tabs = Vec::new();
        for sub in ["archives_win32", "DLC"] {
            let dir = root.join(sub);
            if let Ok(rd) = std::fs::read_dir(&dir) {
                let mut found: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| {
                        p.extension().is_some_and(|x| x.eq_ignore_ascii_case("tab"))
                            && p.with_extension("arc").exists()
                    })
                    .collect();
                found.sort();
                tabs.extend(found);
            }
        }
        if tabs.is_empty() {
            return Err(Error::format(format!(
                "no .tab/.arc archives under {}",
                root.display()
            )));
        }
        let mut fs = Self {
            root: root.clone(),
            archives: Vec::new(),
            dropzone: None,
            dropzone_files: HashMap::new(),
            index: HashMap::new(),
            names: NameTable::bundled(),
        };
        for t in &tabs {
            fs.mount_archive(t)?;
        }
        let dz = root.join("dropzone");
        if dz.is_dir() {
            fs.mount_dropzone(&dz)?;
        }
        Ok(fs)
    }

    /// Later mounts take precedence over earlier ones.
    pub fn mount_archive(&mut self, tab_path: &Path) -> Result<()> {
        let a = Archive::open(tab_path)?;
        let ai = self.archives.len();
        for (ei, e) in a.table.entries.iter().enumerate() {
            self.index.insert(e.name_hash, (ai, ei));
        }
        self.archives.push(a);
        Ok(())
    }

    pub fn mount_dropzone(&mut self, dir: &Path) -> Result<()> {
        let rd = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_file()
                && let Some(name) = p.file_name().and_then(|n| n.to_str())
            {
                self.dropzone_files.insert(name_hash(name), p.clone());
                self.names.add_file(name);
            }
        }
        self.dropzone = Some(dir.to_path_buf());
        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.locate_hash(name_hash(name)).is_some()
    }

    /// A file by name, wherever the game keeps it: at the top level of the file system
    /// (dropzone, DLC, archives; zlib removed) or, failing that, as a member of one of
    /// the SARC containers that hold loose files ([`MEMBER_CONTAINERS`]: the animation
    /// clips such as `std_idle.ban` live in `globalnimations.blz`).
    pub fn read_any(&self, name: &str) -> Result<Vec<u8>> {
        if let Ok(d) = self.read_inflated(name) {
            return Ok(d);
        }
        for container in MEMBER_CONTAINERS {
            let Ok(blob) = self.read_inflated(container) else {
                continue;
            };
            let Ok(sarc) = SmallArchive::parse(&blob) else {
                continue;
            };
            if let Some(e) = sarc.get(name) {
                let bytes = sarc.data(&blob, e)?;
                return Ok(compress::maybe_inflate(bytes)?.into_owned());
            }
        }
        Err(Error::NotFound(name.to_string()))
    }

    /// Whether [`read_any`](Self::read_any) would find `name`.
    pub fn contains_any(&self, name: &str) -> bool {
        if self.contains(name) {
            return true;
        }
        MEMBER_CONTAINERS.iter().any(|c| {
            let Ok(blob) = self.read_inflated(c) else {
                return false;
            };
            SmallArchive::parse(&blob).is_ok_and(|s| s.get(name).is_some())
        })
    }

    pub fn locate(&self, name: &str) -> Option<Located> {
        self.locate_hash(name_hash(name))
    }

    pub fn locate_hash(&self, h: u32) -> Option<Located> {
        if let Some(p) = self.dropzone_files.get(&h) {
            let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            return Some(Located {
                name_hash: h,
                size,
                source: Source::Dropzone,
            });
        }
        let &(ai, ei) = self.index.get(&h)?;
        let e = &self.archives[ai].table.entries[ei];
        Some(Located {
            name_hash: h,
            size: e.size as u64,
            source: Source::Archive {
                archive: ai,
                entry: ei,
            },
        })
    }

    /// Raw bytes exactly as stored (may be a zlib stream).
    pub fn read_hash(&self, h: u32) -> Result<Vec<u8>> {
        match self.locate_hash(h) {
            None => Err(Error::NotFound(
                self.names
                    .file(h)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{h:08x}")),
            )),
            Some(Located {
                source: Source::Dropzone,
                ..
            }) => {
                let p = &self.dropzone_files[&h];
                std::fs::read(p).map_err(|e| Error::io(p, e))
            }
            Some(Located {
                source: Source::Archive { archive, entry },
                ..
            }) => {
                let a = &self.archives[archive];
                Ok(a.slice(&a.table.entries[entry])?.to_vec())
            }
        }
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        self.read_hash(name_hash(name))
    }

    /// Bytes with any zlib wrapper removed — what the engine actually parses.
    pub fn read_inflated(&self, name: &str) -> Result<Vec<u8>> {
        let raw = self.read(name)?;
        match compress::maybe_inflate(&raw)? {
            std::borrow::Cow::Borrowed(_) => Ok(raw),
            std::borrow::Cow::Owned(v) => Ok(v),
        }
    }

    /// Every unique file the engine can see, after precedence is applied.
    pub fn entries(&self) -> Vec<Located> {
        let mut out: Vec<Located> = self
            .index
            .keys()
            .chain(self.dropzone_files.keys())
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .filter_map(|h| self.locate_hash(h))
            .collect();
        out.sort_by_key(|l| {
            (
                match l.source {
                    Source::Archive { archive, entry } => (archive, entry),
                    Source::Dropzone => (usize::MAX, 0),
                },
                l.name_hash,
            )
        });
        out
    }

    pub fn archive_name(&self, source: Source) -> &str {
        match source {
            Source::Dropzone => "dropzone",
            Source::Archive { archive, .. } => &self.archives[archive].name,
        }
    }

    /// Scan every file for names (SARC entries, property string values).
    /// Returns the number of names learned.
    pub fn harvest_names(&mut self) -> usize {
        let hashes: Vec<u32> = self
            .index
            .keys()
            .chain(self.dropzone_files.keys())
            .copied()
            .collect();
        let mut learned = 0;
        for h in hashes {
            if let Ok(data) = self.read_hash(h) {
                learned += self.names.harvest(&data);
            }
        }
        learned
    }
}
