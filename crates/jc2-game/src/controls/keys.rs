//! Physical inputs the service can bind, with their stable settings-file names and the
//! on-screen labels. The labels follow the style the HUD prompts already use (`SPACE`,
//! `LCONTROL`, `SHIFT+Z`). This table is a project choice, not a native one.

use bevy::input::keyboard::KeyCode;
use bevy::input::mouse::MouseButton;

/// One physical button: a keyboard key (by position, not layout) or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    Key(KeyCode),
    Mouse(MouseButton),
}

impl Input {
    /// The modifier class this input itself belongs to, if it is a modifier key.
    pub(super) fn own_modifier(self) -> Modifiers {
        match self {
            Input::Key(KeyCode::ShiftLeft | KeyCode::ShiftRight) => Modifiers::SHIFT,
            Input::Key(KeyCode::ControlLeft | KeyCode::ControlRight) => Modifiers::CTRL,
            Input::Key(KeyCode::AltLeft | KeyCode::AltRight) => Modifiers::ALT,
            _ => Modifiers::NONE,
        }
    }

    /// Name used in the settings file (`KeyW`, `ShiftLeft`, `MouseLeft`).
    pub fn name(self) -> Option<&'static str> {
        match self {
            Input::Key(k) => KEYS.iter().find(|e| e.0 == k).map(|e| e.1),
            Input::Mouse(b) => MOUSE.iter().find(|e| e.0 == b).map(|e| e.1),
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        KEYS.iter()
            .find(|e| e.1 == name)
            .map(|e| Input::Key(e.0))
            .or_else(|| {
                MOUSE
                    .iter()
                    .find(|e| e.1 == name)
                    .map(|e| Input::Mouse(e.0))
            })
    }

    /// On-screen label (`W`, `LSHIFT`, `MOUSE1`).
    pub fn label(self) -> &'static str {
        match self {
            Input::Key(k) => KEYS.iter().find(|e| e.0 == k).map_or("?", |e| e.2),
            Input::Mouse(b) => MOUSE.iter().find(|e| e.0 == b).map_or("?", |e| e.2),
        }
    }

    /// Can this input be stored in the settings file?
    pub fn is_supported(self) -> bool {
        self.name().is_some()
    }
}

/// Which modifier keys must be held for a binding to match. Extra held modifiers do not
/// stop a match (Shift is held while sprinting); when two bindings want the same button the
/// one asking for more modifiers gets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(pub u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const SHIFT: Modifiers = Modifiers(1);
    pub const CTRL: Modifiers = Modifiers(2);
    pub const ALT: Modifiers = Modifiers(4);

    pub const fn with(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 | other.0)
    }
    pub const fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn without(self, other: Modifiers) -> Modifiers {
        Modifiers(self.0 & !other.0)
    }
}

macro_rules! keys {
    ($($v:ident => $label:expr),* $(,)?) => {
        /// (key, settings name, label)
        const KEYS: &[(KeyCode, &str, &str)] =
            &[$((KeyCode::$v, stringify!($v), $label)),*];
    };
}

keys! {
    KeyA => "A", KeyB => "B", KeyC => "C", KeyD => "D", KeyE => "E", KeyF => "F",
    KeyG => "G", KeyH => "H", KeyI => "I", KeyJ => "J", KeyK => "K", KeyL => "L",
    KeyM => "M", KeyN => "N", KeyO => "O", KeyP => "P", KeyQ => "Q", KeyR => "R",
    KeyS => "S", KeyT => "T", KeyU => "U", KeyV => "V", KeyW => "W", KeyX => "X",
    KeyY => "Y", KeyZ => "Z",
    Digit0 => "0", Digit1 => "1", Digit2 => "2", Digit3 => "3", Digit4 => "4",
    Digit5 => "5", Digit6 => "6", Digit7 => "7", Digit8 => "8", Digit9 => "9",
    F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6",
    F7 => "F7", F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
    ArrowUp => "UP", ArrowDown => "DOWN", ArrowLeft => "LEFT", ArrowRight => "RIGHT",
    ShiftLeft => "LSHIFT", ShiftRight => "RSHIFT",
    ControlLeft => "LCONTROL", ControlRight => "RCONTROL",
    AltLeft => "LALT", AltRight => "RALT",
    Space => "SPACE", Tab => "TAB", Enter => "ENTER", Backspace => "BACKSPACE",
    CapsLock => "CAPSLOCK",
}

const MOUSE: &[(MouseButton, &str, &str)] = &[
    (MouseButton::Left, "MouseLeft", "MOUSE1"),
    (MouseButton::Right, "MouseRight", "MOUSE2"),
    (MouseButton::Middle, "MouseMiddle", "MOUSE3"),
    (MouseButton::Back, "MouseBack", "MOUSE4"),
    (MouseButton::Forward, "MouseForward", "MOUSE5"),
];
