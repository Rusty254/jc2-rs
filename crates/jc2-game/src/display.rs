//! Window size and fullscreen options (board task 5dc5fadd; project choices, not native ones).
//!
//! * Command line: `--resolution WxH` (or `--width W` / `--height H`), `--fullscreen`
//!   (borderless on the current monitor), `--exclusive` (exclusive fullscreen at the
//!   monitor's current video mode) and `--windowed`.
//! * `F11` toggles between fullscreen and windowed while playing; the choice is saved.
//! * Settings file: `display.json` next to `controls.json` (`%APPDATA%\jc2-rs\` on Windows,
//!   `$XDG_CONFIG_HOME` or `~/.config/jc2-rs/` elsewhere; `JC2_DISPLAY_FILE` overrides):
//!   `{ "version": 1, "width": 1920, "height": 1080, "mode": "windowed" }` with `mode` one of
//!   `windowed`, `borderless`, `exclusive`. Flags beat the file, the file beats the default
//!   (1280 x 720 windowed, Bevy's default). Flags are never written to the file; only an
//!   `F11` toggle saves, and it saves the windowed size and the new mode.
//!
//! The size is in physical pixels (the window's scale factor is pinned to 1 when a size is
//! given), so `--resolution 2561x1429` gives a 2561 x 1429 frame whatever the desktop's DPI
//! scaling: this is what the capture harnesses (`--screenshot`, `--chute-scenario`,
//! `--onfoot-capture`, `--rico-capture`) rely on, as they all capture the primary window.
//! The HUD, map and loading screen are laid out in `vmin` units and so follow any size and
//! aspect ratio; cameras, the post chain (`post_effects_render.rs`) and Bevy's depth, SSAO
//! and motion buffers follow the window's physical size on their own.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, VideoModeSelection, WindowMode, WindowResolution};
use serde_json::{Value, json};
use std::path::PathBuf;

pub const DEFAULT_SIZE: (u32, u32) = (1280, 720);
/// Smallest and largest frame accepted (a typo guard, not a native limit).
pub const MIN_DIM: u32 = 320;
pub const MAX_DIM: u32 = 16384;
const SETTINGS_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Windowed,
    Borderless,
    Exclusive,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Windowed => "windowed",
            Mode::Borderless => "borderless",
            Mode::Exclusive => "exclusive",
        }
    }
    fn parse(s: &str) -> Option<Mode> {
        match s {
            "windowed" => Some(Mode::Windowed),
            "borderless" => Some(Mode::Borderless),
            "exclusive" => Some(Mode::Exclusive),
            _ => None,
        }
    }
    fn window_mode(self) -> WindowMode {
        match self {
            Mode::Windowed => WindowMode::Windowed,
            Mode::Borderless => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            Mode::Exclusive => {
                WindowMode::Fullscreen(MonitorSelection::Current, VideoModeSelection::Current)
            }
        }
    }
}

/// The options in force. `size` is the windowed size in physical pixels; `explicit_size` is
/// false while it is only the default (then the OS scale factor is left alone).
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct DisplayOptions {
    pub size: (u32, u32),
    pub explicit_size: bool,
    pub mode: Mode,
    /// The fullscreen mode `F11` returns to (the last non-windowed one, else borderless).
    pub fullscreen_mode: Mode,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        DisplayOptions {
            size: DEFAULT_SIZE,
            explicit_size: false,
            mode: Mode::Windowed,
            fullscreen_mode: Mode::Borderless,
        }
    }
}

/// The flags as given (already validated by [`parse_size`]).
#[derive(Clone, Debug, Default)]
pub struct DisplayFlags {
    pub resolution: Option<(u32, u32)>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fullscreen: bool,
    pub exclusive: bool,
    pub windowed: bool,
}

/// Parses `WxH` (`x` or `X`), both in [`MIN_DIM`]..=[`MAX_DIM`].
pub fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("`{s}`: expected WIDTHxHEIGHT, e.g. 1920x1080"))?;
    let w = parse_dim(w.trim())?;
    let h = parse_dim(h.trim())?;
    Ok((w, h))
}

/// Parses one dimension in [`MIN_DIM`]..=[`MAX_DIM`].
pub fn parse_dim(s: &str) -> Result<u32, String> {
    let v: u32 = s
        .parse()
        .map_err(|_| format!("`{s}`: not a whole number of pixels"))?;
    if !(MIN_DIM..=MAX_DIM).contains(&v) {
        return Err(format!("{v}: must be between {MIN_DIM} and {MAX_DIM} pixels"));
    }
    Ok(v)
}

/// `display.json` beside `controls.json`.
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("JC2_DISPLAY_FILE") {
        return Some(PathBuf::from(p));
    }
    crate::controls::settings::default_path().map(|p| p.with_file_name("display.json"))
}

fn from_json(text: &str) -> Result<DisplayOptions, String> {
    if text.len() > 4096 {
        return Err("file too large".into());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let version = v.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version != SETTINGS_VERSION {
        return Err(format!("unsupported version {version}"));
    }
    let mut o = DisplayOptions::default();
    let dim = |key: &str| v.get(key).and_then(Value::as_u64);
    if let (Some(w), Some(h)) = (dim("width"), dim("height")) {
        let w = parse_dim(&w.to_string())?;
        let h = parse_dim(&h.to_string())?;
        o.size = (w, h);
        o.explicit_size = true;
    }
    if let Some(m) = v.get("mode").and_then(Value::as_str) {
        o.mode = Mode::parse(m).ok_or_else(|| format!("unknown mode `{m}`"))?;
        if o.mode != Mode::Windowed {
            o.fullscreen_mode = o.mode;
        }
    }
    Ok(o)
}

fn to_json(o: &DisplayOptions) -> String {
    // the mode saved is the one in force; a windowed save remembers the fullscreen kind only
    // through the next toggle, which is borderless unless a flag said otherwise
    let mut text = serde_json::to_string_pretty(&json!({
        "version": SETTINGS_VERSION,
        "width": o.size.0,
        "height": o.size.1,
        "mode": o.mode.name(),
    }))
    .unwrap_or_default();
    text.push('\n');
    text
}

/// The options for this run: the file (if any), then the flags on top.
pub fn resolve(flags: &DisplayFlags, path: Option<&PathBuf>) -> DisplayOptions {
    let mut o = DisplayOptions::default();
    if let Some(path) = path
        && let Ok(text) = std::fs::read_to_string(path)
    {
        match from_json(&text) {
            Ok(saved) => o = saved,
            Err(e) => warn!("{}: {e}; using the defaults", path.display()),
        }
    }
    if let Some(size) = flags.resolution {
        o.size = size;
        o.explicit_size = true;
    }
    if let Some(w) = flags.width {
        o.size.0 = w;
        o.explicit_size = true;
    }
    if let Some(h) = flags.height {
        o.size.1 = h;
        o.explicit_size = true;
    }
    if flags.windowed {
        o.mode = Mode::Windowed;
    } else if flags.exclusive {
        o.mode = Mode::Exclusive;
        o.fullscreen_mode = Mode::Exclusive;
    } else if flags.fullscreen {
        o.mode = Mode::Borderless;
        o.fullscreen_mode = Mode::Borderless;
    }
    o
}

/// The primary window for these options.
pub fn window(options: &DisplayOptions, title: String) -> Window {
    Window {
        title,
        resolution: resolution(options),
        mode: options.mode.window_mode(),
        // `JC2_NO_VSYNC=1`: uncapped presentation, so frame times in `JC2_FRAME_STATS` measure the
        // GPU/CPU cost rather than the 60 Hz cap (measurement aid)
        present_mode: if std::env::var_os("JC2_NO_VSYNC").is_some() {
            bevy::window::PresentMode::AutoNoVsync
        } else {
            default()
        },
        ..default()
    }
}

fn resolution(options: &DisplayOptions) -> WindowResolution {
    let r = WindowResolution::new(options.size.0, options.size.1);
    if options.explicit_size {
        r.with_scale_factor_override(1.0)
    } else {
        r
    }
}

pub struct DisplayPlugin {
    pub options: DisplayOptions,
    pub path: Option<PathBuf>,
}

#[derive(Resource)]
struct SettingsPath(Option<PathBuf>);

impl Plugin for DisplayPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.options.clone())
            .insert_resource(SettingsPath(self.path.clone()))
            .add_systems(Update, toggle_fullscreen);
    }
}

fn toggle_fullscreen(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut options: ResMut<DisplayOptions>,
    path: Res<SettingsPath>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !keys.is_some_and(|k| k.just_pressed(KeyCode::F11)) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    options.mode = if options.mode == Mode::Windowed {
        options.fullscreen_mode
    } else {
        Mode::Windowed
    };
    window.mode = options.mode.window_mode();
    if options.mode == Mode::Windowed {
        // back to the windowed size (the OS may have kept the monitor's)
        window.resolution = resolution(&options);
    }
    info!("display: {}", options.mode.name());
    if let Some(path) = &path.0 {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(path, to_json(&options)) {
            warn!("display: could not save {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_and_validate() {
        assert_eq!(parse_size("2561x1429"), Ok((2561, 1429)));
        assert_eq!(parse_size("3440X1440"), Ok((3440, 1440)));
        assert!(parse_size("1920").is_err());
        assert!(parse_size("0x720").is_err());
        assert!(parse_size("99999x720").is_err());
        assert!(parse_size("axb").is_err());
    }

    #[test]
    fn flags_beat_the_file_and_the_file_beats_the_default() {
        let file = from_json(r#"{"version":1,"width":1920,"height":1080,"mode":"borderless"}"#)
            .unwrap();
        assert_eq!(file.size, (1920, 1080));
        assert_eq!(file.mode, Mode::Borderless);
        let dir = std::env::temp_dir().join("jc2_display_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("display.json");
        std::fs::write(&path, to_json(&file)).unwrap();
        let none = DisplayFlags::default();
        assert_eq!(resolve(&none, Some(&path)), file);
        let w = DisplayFlags {
            windowed: true,
            width: Some(2560),
            ..default()
        };
        let o = resolve(&w, Some(&path));
        assert_eq!((o.size, o.mode), ((2560, 1080), Mode::Windowed));
        let x = DisplayFlags {
            exclusive: true,
            resolution: Some((800, 600)),
            ..default()
        };
        let o = resolve(&x, None);
        assert_eq!((o.size, o.mode), ((800, 600), Mode::Exclusive));
        assert_eq!(resolve(&none, None), DisplayOptions::default());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bad_files_are_refused() {
        assert!(from_json("{").is_err());
        assert!(from_json(r#"{"version":2}"#).is_err());
        assert!(from_json(r#"{"version":1,"mode":"huge"}"#).is_err());
        assert!(from_json(r#"{"version":1,"width":10,"height":10}"#).is_err());
    }
}
