//! Versioned standalone user control settings (JC2-089).
//!
//! A small JSON file of its own (never part of a campaign or retail save):
//!
//! ```json
//! { "version": 1, "bindings": { "onfoot.move_forward": ["KeyI", null], "onfoot.sprint": ["Shift+KeyZ"] } }
//! ```
//!
//! Only bindings that differ from the project defaults are stored. Parsing is bounded
//! (file size, entry count, slot count) and never panics: anything wrong leaves the
//! defaults in use and says why. A file from a newer version is read-only for this
//! build: it is never overwritten. Writes go to a temporary file, the old file is kept as
//! `.bak`, then the new one is renamed into place, so a failed write leaves the previous
//! file and the in-memory settings usable.
//!
//! The file layout and its location are project choices, not native ones.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use bevy::prelude::Resource;
use serde_json::{Map, Value, json};

use super::{Binding, Input, Modifiers, Slots};

pub const SETTINGS_VERSION: u64 = 1;
/// Larger files are refused unread.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// More entries than this is treated as a corrupt file.
pub const MAX_ENTRIES: usize = 256;

/// Parsed file contents: every entry, known to this build or not.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControlSettings {
    pub entries: BTreeMap<String, Slots>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadStatus {
    /// No file (or no path): the defaults.
    Missing,
    Loaded,
    /// Present but unusable (reason); the defaults are in use and the file is untouched
    /// until the player saves, which first keeps it as `.invalid`.
    Invalid(String),
    /// Written by a newer build (its version): defaults in use, file never overwritten.
    FutureVersion(u64),
    /// Could not be read at all (reason); the defaults are in use.
    Unreadable(String),
}

#[derive(Debug)]
pub struct LoadOutcome {
    pub settings: ControlSettings,
    pub status: LoadStatus,
    /// Entries skipped while loading, one line each.
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum SaveError {
    /// Nowhere to write (an in-memory store).
    NoPath,
    /// The file is from a newer version; not overwritten.
    FutureVersion(u64),
    Io(std::io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::NoPath => write!(f, "no settings path"),
            SaveError::FutureVersion(v) => {
                write!(f, "settings file is version {v}; not overwriting it")
            }
            SaveError::Io(e) => write!(f, "{e}"),
        }
    }
}

fn binding_text(b: &Binding) -> Option<String> {
    let mut s = String::new();
    if b.modifiers.contains(Modifiers::CTRL) {
        s.push_str("Ctrl+");
    }
    if b.modifiers.contains(Modifiers::SHIFT) {
        s.push_str("Shift+");
    }
    if b.modifiers.contains(Modifiers::ALT) {
        s.push_str("Alt+");
    }
    s.push_str(b.input.name()?);
    Some(s)
}

fn parse_binding(text: &str) -> Result<Binding, String> {
    let mut parts: Vec<&str> = text.split('+').collect();
    let name = parts.pop().unwrap_or("");
    let input = Input::from_name(name).ok_or_else(|| format!("unknown button `{name}`"))?;
    let mut modifiers = Modifiers::NONE;
    for p in parts {
        let m = match p {
            "Ctrl" => Modifiers::CTRL,
            "Shift" => Modifiers::SHIFT,
            "Alt" => Modifiers::ALT,
            other => return Err(format!("unknown modifier `{other}`")),
        };
        if modifiers.contains(m) {
            return Err(format!("modifier `{p}` repeated"));
        }
        modifiers = modifiers.with(m);
    }
    Ok(Binding { input, modifiers })
}

fn parse_slots(v: &Value) -> Result<Slots, String> {
    let arr = v.as_array().ok_or("bindings must be a list")?;
    if arr.is_empty() || arr.len() > 2 {
        return Err("a binding list holds one or two slots".into());
    }
    let mut slots: Slots = [None, None];
    for (i, item) in arr.iter().enumerate() {
        slots[i] = match item {
            Value::Null => None,
            Value::String(s) => Some(parse_binding(s)?),
            _ => return Err("a slot is a string or null".into()),
        };
    }
    Ok(slots)
}

impl ControlSettings {
    /// Parse file text. Never panics; a bad entry is skipped with a warning, a bad file is
    /// reported through the status and yields no entries.
    pub fn parse(text: &str) -> LoadOutcome {
        let bad = |why: String| LoadOutcome {
            settings: ControlSettings::default(),
            status: LoadStatus::Invalid(why),
            warnings: Vec::new(),
        };
        if text.len() as u64 > MAX_FILE_BYTES {
            return bad("file is too large".into());
        }
        let root: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => return bad(format!("not valid JSON: {e}")),
        };
        let Some(obj) = root.as_object() else {
            return bad("top level is not an object".into());
        };
        let version = match obj.get("version").and_then(Value::as_u64) {
            Some(v) if v >= 1 => v,
            _ => return bad("missing or invalid `version`".into()),
        };
        if version > SETTINGS_VERSION {
            return LoadOutcome {
                settings: ControlSettings::default(),
                status: LoadStatus::FutureVersion(version),
                warnings: Vec::new(),
            };
        }
        let Some(map) = obj.get("bindings").and_then(Value::as_object) else {
            return bad("missing `bindings` object".into());
        };
        if map.len() > MAX_ENTRIES {
            return bad(format!("more than {MAX_ENTRIES} entries"));
        }
        let mut settings = ControlSettings::default();
        let mut warnings = Vec::new();
        for (name, v) in map {
            match parse_slots(v) {
                Ok(s) => {
                    settings.entries.insert(name.clone(), s);
                }
                Err(e) => warnings.push(format!("`{name}`: {e}; default kept")),
            }
        }
        LoadOutcome {
            settings,
            status: LoadStatus::Loaded,
            warnings,
        }
    }

    pub fn to_json(&self) -> String {
        let mut bindings = Map::new();
        for (name, slots) in &self.entries {
            let list: Vec<Value> = slots
                .iter()
                .map(|s| match s.as_ref().and_then(binding_text) {
                    Some(t) => Value::String(t),
                    None => Value::Null,
                })
                .collect();
            bindings.insert(name.clone(), Value::Array(list));
        }
        let doc = json!({ "version": SETTINGS_VERSION, "bindings": bindings });
        let mut text = serde_json::to_string_pretty(&doc).unwrap_or_default();
        text.push('\n');
        text
    }
}

/// `%APPDATA%\jc2-rs\controls.json` (Windows), `$XDG_CONFIG_HOME` or `~/.config` elsewhere;
/// `JC2_CONTROLS_FILE` overrides. `None` when no home can be found.
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("JC2_CONTROLS_FILE") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("jc2-rs").join("controls.json"))
}

/// Where the settings file lives and how the last load went.
#[derive(Resource, Debug)]
pub struct SettingsStore {
    pub path: Option<PathBuf>,
    pub status: LoadStatus,
    /// The last failed save, if the most recent save failed.
    pub last_error: Option<String>,
}

impl SettingsStore {
    pub fn in_memory() -> Self {
        SettingsStore {
            path: None,
            status: LoadStatus::Missing,
            last_error: None,
        }
    }

    /// Read `path`. Missing, oversized, unreadable or invalid files give the defaults and
    /// a status that says so.
    pub fn load(path: PathBuf) -> (Self, LoadOutcome) {
        let outcome = match fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => LoadOutcome {
                settings: ControlSettings::default(),
                status: LoadStatus::Missing,
                warnings: Vec::new(),
            },
            Err(e) => unreadable(e),
            Ok(m) if m.len() > MAX_FILE_BYTES => LoadOutcome {
                settings: ControlSettings::default(),
                status: LoadStatus::Invalid("file is too large".into()),
                warnings: Vec::new(),
            },
            Ok(_) => match fs::read(&path) {
                Err(e) => unreadable(e),
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => ControlSettings::parse(&text),
                    Err(_) => LoadOutcome {
                        settings: ControlSettings::default(),
                        status: LoadStatus::Invalid("not UTF-8 text".into()),
                        warnings: Vec::new(),
                    },
                },
            },
        };
        let store = SettingsStore {
            path: Some(path),
            status: outcome.status.clone(),
            last_error: None,
        };
        (store, outcome)
    }

    /// Write `settings` safely. A failure leaves the previous file and whatever the caller
    /// holds in memory untouched.
    pub fn save(&mut self, settings: &ControlSettings) -> Result<(), SaveError> {
        let result = self.try_save(settings);
        self.last_error = result.as_ref().err().map(ToString::to_string);
        if result.is_ok() {
            self.status = LoadStatus::Loaded;
        }
        result
    }

    fn try_save(&self, settings: &ControlSettings) -> Result<(), SaveError> {
        let path = self.path.as_ref().ok_or(SaveError::NoPath)?;
        if let LoadStatus::FutureVersion(v) = self.status {
            return Err(SaveError::FutureVersion(v));
        }
        let keep_as = match self.status {
            LoadStatus::Invalid(_) | LoadStatus::Unreadable(_) => "invalid",
            _ => "bak",
        };
        write_atomically(path, &settings.to_json(), keep_as).map_err(SaveError::Io)
    }
}

fn unreadable(e: std::io::Error) -> LoadOutcome {
    LoadOutcome {
        settings: ControlSettings::default(),
        status: LoadStatus::Unreadable(e.to_string()),
        warnings: Vec::new(),
    }
}

fn sibling(path: &Path, ext: &str) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_owned()).unwrap_or_default();
    name.push(".");
    name.push(ext);
    path.with_file_name(name)
}

fn write_atomically(path: &Path, text: &str, keep_as: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir)?;
    }
    let tmp = sibling(path, "tmp");
    let write = || -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        if path.exists() {
            fs::copy(path, sibling(path, keep_as))?;
        }
        fs::rename(&tmp, path)
    };
    let r = write();
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    r
}
