//! JC2-089: the shared action service. One typed registry of logical actions (buttons and
//! axes) in contexts, keyboard/mouse bindings with modifiers, one arbiter, remap /
//! conflict / reset operations, and the per-frame state consumers read.
//!
//! Everything here is a project mechanism, not a native one: the original's binding
//! presets, gesture timing and controller curves are other tasks (JC2-076, JC2-077,
//! JC2-083, JC2-036) and nothing below guesses them. The default bindings are labelled
//! "project controls" ([`DEFAULT_ACTIONS`]); they reproduce the keys the on-foot code read
//! before this service existed.
//!
//! # Dispatch order
//!
//! [`ControlsDispatch`] runs in `PreUpdate`, after Bevy's input systems and after the
//! modal owners that own the screen (`map_input`, the black market: they `reset_all` the raw
//! input while open), and so before `RunFixedMainLoop` and `Update`. Consumers read:
//!
//! * `Update` systems: [`Controls::down`], [`Controls::value`], [`Controls::just_pressed`]
//!   (true for exactly one dispatch);
//! * fixed-step systems: [`Controls::take_pressed`] / [`Controls::take_released`], latched
//!   edges that survive zero fixed steps and are delivered once however many run.
//!
//! # Neutralisation
//!
//! A modal owner open (`MapState`, the black market, or the virtual clock they pause), the
//! window losing focus, the hero going down, or a [`ControlsCancel`] message releases every
//! action and *suppresses* the physical buttons that were down: they count as up for the
//! service until the player really lets go (a Released message), so a key held through a
//! menu cannot fire a fresh press on resume (key-repeat events included).
//!
//! # Contexts
//!
//! Each action lives in one [`Context`]. Each physical button goes to one binding per frame:
//! the one asking for the most modifiers, then the one in the higher-priority active
//! context; the others do not see it. Remap conflicts are checked only between contexts that can be active together
//! ([`Context::coexists`]).

// The remap/conflict/reset/settings operations and the axis interface are the service's
// public API for the settings UI (JC2-073) and other adapters; until those consume them
// only the tests do.
#![allow(dead_code)]

pub mod keys;
pub mod settings;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use bevy::ecs::message::MessageCursor;
use bevy::ecs::system::SystemParam;
use bevy::input::ButtonState;
use bevy::input::keyboard::{KeyCode, KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::{MouseButton, MouseButtonInput};
use bevy::input::{ButtonInput, InputSystems};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

pub use keys::{Input, Modifiers};
use settings::{ControlSettings, LoadStatus, SettingsStore};

/// Where an action applies. Priority decides who consumes a shared physical button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    /// Always active (no modal owner open).
    Global,
    /// Rico walking, running, falling, under a parachute.
    OnFoot,
    /// Driving or flying a vehicle. Reserved: no action is registered here yet.
    Vehicle,
    /// A menu or other modal screen. Reserved for the settings UI (JC2-073).
    Menu,
}

impl Context {
    fn priority(self) -> u8 {
        match self {
            Context::Global => 0,
            Context::OnFoot | Context::Vehicle => 1,
            Context::Menu => 2,
        }
    }

    /// Can both contexts be active in the same frame? (`OnFoot` and `Vehicle` cannot.)
    pub fn coexists(self, other: Context) -> bool {
        self == other || self == Context::Global || other == Context::Global
    }
}

/// Stable action name, also the key in the settings file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionId(pub &'static str);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// Down / held / up with edges.
    Button,
    /// A value in `-1..=1`. Keyboard slots are `[negative, positive]`; other adapters push
    /// values with [`Controls::push_axis`]. There is no controller response curve here:
    /// that is native research (JC2-077/083) and is not guessed.
    Axis,
}

/// A physical gesture: one button plus the modifiers that must be held with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Binding {
    pub input: Input,
    pub modifiers: Modifiers,
}

impl Binding {
    pub const fn key(k: KeyCode) -> Self {
        Binding {
            input: Input::Key(k),
            modifiers: Modifiers::NONE,
        }
    }
    pub const fn mouse(b: MouseButton) -> Self {
        Binding {
            input: Input::Mouse(b),
            modifiers: Modifiers::NONE,
        }
    }
    pub const fn with(mut self, m: Modifiers) -> Self {
        self.modifiers = self.modifiers.with(m);
        self
    }

    /// `SHIFT+Z`, `W`, `MOUSE1`.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.modifiers.contains(Modifiers::CTRL) {
            s.push_str("CTRL+");
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            s.push_str("SHIFT+");
        }
        if self.modifiers.contains(Modifiers::ALT) {
            s.push_str("ALT+");
        }
        s.push_str(self.input.label());
        s
    }
}

/// Two binding slots per action (primary, secondary); an axis uses them as
/// `[negative, positive]`.
pub type Slots = [Option<Binding>; 2];

#[derive(Clone, Copy, Debug)]
pub struct ActionDef {
    pub id: ActionId,
    pub context: Context,
    pub kind: ActionKind,
    /// Player-facing name for a future remap screen.
    pub name: &'static str,
    /// Project defaults (not the original's preset).
    pub defaults: Slots,
}

pub const MOVE_FORWARD: ActionId = ActionId("onfoot.move_forward");
pub const MOVE_BACK: ActionId = ActionId("onfoot.move_back");
pub const MOVE_LEFT: ActionId = ActionId("onfoot.move_left");
pub const MOVE_RIGHT: ActionId = ActionId("onfoot.move_right");
pub const SPRINT: ActionId = ActionId("onfoot.sprint");
pub const CROUCH: ActionId = ActionId("onfoot.crouch");
pub const EVADE: ActionId = ActionId("onfoot.evade");

const fn on_foot(id: ActionId, name: &'static str, a: KeyCode, b: KeyCode) -> ActionDef {
    ActionDef {
        id,
        context: Context::OnFoot,
        kind: ActionKind::Button,
        name,
        defaults: [Some(Binding::key(a)), Some(Binding::key(b))],
    }
}

/// The migrated on-foot movement and sprint path. Defaults are the project's existing
/// WASD / arrows / Shift keys (`character.rs` read them directly before JC2-089); they are
/// *project controls*, not the verified original PC preset (JC2-076).
pub const DEFAULT_ACTIONS: &[ActionDef] = &[
    on_foot(
        MOVE_FORWARD,
        "Move forward",
        KeyCode::KeyW,
        KeyCode::ArrowUp,
    ),
    on_foot(MOVE_BACK, "Move back", KeyCode::KeyS, KeyCode::ArrowDown),
    on_foot(MOVE_LEFT, "Move left", KeyCode::KeyA, KeyCode::ArrowLeft),
    on_foot(MOVE_RIGHT, "Move right", KeyCode::KeyD, KeyCode::ArrowRight),
    on_foot(SPRINT, "Sprint", KeyCode::ShiftLeft, KeyCode::ShiftRight),
    // JC2-051. The native keyboard default for CROUCH is LCONTROL (`controls-next.md`); EVADE is
    // TAB there, which this project's mouse-look toggle already uses, so EVADE is C here (a
    // project choice). Both have one slot.
    ActionDef {
        id: CROUCH,
        context: Context::OnFoot,
        kind: ActionKind::Button,
        name: "Crouch",
        defaults: [Some(Binding::key(KeyCode::ControlLeft)), None],
    },
    ActionDef {
        id: EVADE,
        context: Context::OnFoot,
        kind: ActionKind::Button,
        name: "Evade",
        defaults: [Some(Binding::key(KeyCode::KeyC)), None],
    },
];

/// What one action reads as this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActionState {
    down: bool,
    just_pressed: bool,
    just_released: bool,
    value: f32,
    /// Latched until a fixed-step consumer takes it.
    pressed_latch: bool,
    released_latch: bool,
    /// Value pushed by a non-keyboard adapter (axes only).
    external: f32,
}

/// Why every action was released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Neutralize {
    /// A modal owner holds the screen (the map/mission browser, the black market, or the
    /// virtual clock they stop).
    Modal,
    FocusLost,
    /// The hero is down.
    Death,
    /// An explicit [`ControlsCancel`] (a device or context going away).
    Cancel,
}

/// Send this to release every control, e.g. when a device disconnects.
#[derive(Message, Debug, Clone, Copy)]
pub struct ControlsCancel;

/// The raw input one dispatch sees.
#[derive(Clone, Debug, Default)]
pub struct InputFrame {
    /// Buttons currently down (`ButtonInput::pressed`).
    pub down: HashSet<Input>,
    /// Buttons whose Released message arrived since the last dispatch.
    pub released: Vec<Input>,
    /// No Released messages exist (synthetic or test input): a button missing from `down` is
    /// then taken as really up.
    pub authoritative_up: bool,
}

impl InputFrame {
    pub fn held(buttons: impl IntoIterator<Item = Input>) -> Self {
        InputFrame {
            down: buttons.into_iter().collect(),
            released: Vec::new(),
            authoritative_up: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemapError {
    UnknownAction,
    /// Slot index must be 0 or 1.
    BadSlot,
    /// The button has no name in the settings file, so it could not be stored.
    Unsupported,
    /// Another action that can be active at the same time already uses the gesture.
    Conflict {
        with: ActionId,
        slot: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// Refuse, naming the other action.
    Reject,
    /// Clear the conflicting slot of the other action.
    Replace,
}

#[derive(Resource)]
pub struct Controls {
    registry: Vec<ActionDef>,
    bindings: HashMap<ActionId, Slots>,
    state: HashMap<ActionId, ActionState>,
    active: HashSet<Context>,
    suppressed: HashSet<Input>,
    last_down: HashSet<Input>,
    neutralized: Option<Neutralize>,
    /// Settings entries for actions this build does not know: kept so saving does not drop
    /// them (a newer build's actions), and reported rather than silently ignored.
    foreign: BTreeMap<String, Slots>,
}

impl Default for Controls {
    fn default() -> Self {
        let mut c = Controls::empty();
        for def in DEFAULT_ACTIONS {
            c.register(*def);
        }
        c
    }
}

impl Controls {
    /// No actions registered (tests, or a caller that registers its own).
    pub fn empty() -> Self {
        Controls {
            registry: Vec::new(),
            bindings: HashMap::new(),
            state: HashMap::new(),
            active: HashSet::new(),
            suppressed: HashSet::new(),
            last_down: HashSet::new(),
            neutralized: None,
            foreign: BTreeMap::new(),
        }
    }

    /// Add an action with its default bindings. Re-registering an id replaces nothing and
    /// returns `false`.
    pub fn register(&mut self, def: ActionDef) -> bool {
        if self.registry.iter().any(|d| d.id == def.id) {
            return false;
        }
        self.bindings.insert(def.id, def.defaults);
        self.state.insert(def.id, ActionState::default());
        self.registry.push(def);
        true
    }

    pub fn actions(&self) -> &[ActionDef] {
        &self.registry
    }

    pub fn def(&self, id: ActionId) -> Option<&ActionDef> {
        self.registry.iter().find(|d| d.id == id)
    }

    /// Resolve a settings-file name to a registered action.
    pub fn id_of(&self, name: &str) -> Option<ActionId> {
        self.registry.iter().map(|d| d.id).find(|i| i.0 == name)
    }

    pub fn is_registered(&self, id: ActionId) -> bool {
        self.def(id).is_some()
    }

    pub fn why_neutralized(&self) -> Option<Neutralize> {
        self.neutralized
    }

    pub fn active_contexts(&self) -> &HashSet<Context> {
        &self.active
    }

    // -- reading --------------------------------------------------------------------

    /// Held now. An unregistered action reads `false` (check [`Controls::is_registered`]).
    pub fn down(&self, id: ActionId) -> bool {
        self.state.get(&id).is_some_and(|s| s.down)
    }
    /// `-1..=1` for an axis; `0` or `1` for a button.
    pub fn value(&self, id: ActionId) -> f32 {
        self.state.get(&id).map_or(0.0, |s| s.value)
    }
    /// True for exactly one dispatch (frame-level edge for `Update` consumers).
    pub fn just_pressed(&self, id: ActionId) -> bool {
        self.state.get(&id).is_some_and(|s| s.just_pressed)
    }
    pub fn just_released(&self, id: ActionId) -> bool {
        self.state.get(&id).is_some_and(|s| s.just_released)
    }
    /// Fixed-step edge: true once per press, however many fixed steps ran (or none) since.
    pub fn take_pressed(&mut self, id: ActionId) -> bool {
        self.state
            .get_mut(&id)
            .is_some_and(|s| std::mem::take(&mut s.pressed_latch))
    }
    pub fn take_released(&mut self, id: ActionId) -> bool {
        self.state
            .get_mut(&id)
            .is_some_and(|s| std::mem::take(&mut s.released_latch))
    }

    /// For non-keyboard adapters: the value of an axis action this frame (clamped).
    /// Ignored for button actions and unknown ids.
    pub fn push_axis(&mut self, id: ActionId, value: f32) {
        if self.def(id).is_some_and(|d| d.kind == ActionKind::Axis)
            && let Some(s) = self.state.get_mut(&id)
        {
            s.external = value.clamp(-1.0, 1.0);
        }
    }

    // -- bindings and prompts -------------------------------------------------------

    pub fn bindings(&self, id: ActionId) -> Option<Slots> {
        self.bindings.get(&id).copied()
    }

    /// Prompt label for the first bound slot (`W`), `UNBOUND` or `UNKNOWN ACTION`. Prompts
    /// use this so they always show the key the action is really on.
    pub fn label(&self, id: ActionId) -> String {
        match self.bindings.get(&id) {
            None => "UNKNOWN ACTION".to_string(),
            Some(slots) => slots
                .iter()
                .flatten()
                .next()
                .map_or_else(|| "UNBOUND".to_string(), Binding::label),
        }
    }

    /// All bound slots, joined: `W / UP`.
    pub fn labels(&self, id: ActionId) -> String {
        match self.bindings.get(&id) {
            None => "UNKNOWN ACTION".to_string(),
            Some(slots) => {
                let all: Vec<String> = slots.iter().flatten().map(Binding::label).collect();
                if all.is_empty() {
                    "UNBOUND".to_string()
                } else {
                    all.join(" / ")
                }
            }
        }
    }

    /// Where would `binding` collide for `id`? Checked only against actions whose context
    /// can be active together with `id`'s. `skip` is a slot of `id` about to be replaced.
    pub fn conflict_for(
        &self,
        id: ActionId,
        binding: Binding,
        skip: Option<usize>,
    ) -> Option<(ActionId, usize)> {
        let ctx = self.def(id)?.context;
        for def in &self.registry {
            if !def.context.coexists(ctx) {
                continue;
            }
            let Some(slots) = self.bindings.get(&def.id) else {
                continue;
            };
            for (i, s) in slots.iter().enumerate() {
                if def.id == id && Some(i) == skip {
                    continue;
                }
                if *s == Some(binding) {
                    return Some((def.id, i));
                }
            }
        }
        None
    }

    /// Every colliding pair in the current bindings (should be empty).
    pub fn all_conflicts(&self) -> Vec<((ActionId, usize), (ActionId, usize))> {
        let mut out = Vec::new();
        for (ai, a) in self.registry.iter().enumerate() {
            for (bi, b) in self.registry.iter().enumerate() {
                if bi < ai || !a.context.coexists(b.context) {
                    continue;
                }
                let (Some(sa), Some(sb)) = (self.bindings.get(&a.id), self.bindings.get(&b.id))
                else {
                    continue;
                };
                for (i, x) in sa.iter().enumerate() {
                    for (j, y) in sb.iter().enumerate() {
                        if bi == ai && j <= i {
                            continue;
                        }
                        if x.is_some() && x == y {
                            out.push(((a.id, i), (b.id, j)));
                        }
                    }
                }
            }
        }
        out
    }

    /// Put `binding` (or nothing) in a slot. Returns the actions whose slot was cleared by
    /// [`ConflictPolicy::Replace`].
    pub fn remap(
        &mut self,
        id: ActionId,
        slot: usize,
        binding: Option<Binding>,
        policy: ConflictPolicy,
    ) -> Result<Vec<ActionId>, RemapError> {
        if !self.is_registered(id) {
            return Err(RemapError::UnknownAction);
        }
        if slot > 1 {
            return Err(RemapError::BadSlot);
        }
        let mut cleared = Vec::new();
        if let Some(b) = binding {
            if !b.input.is_supported() {
                return Err(RemapError::Unsupported);
            }
            while let Some((other, os)) = self.conflict_for(id, b, Some(slot)) {
                if policy == ConflictPolicy::Reject {
                    return Err(RemapError::Conflict {
                        with: other,
                        slot: os,
                    });
                }
                if let Some(s) = self.bindings.get_mut(&other) {
                    s[os] = None;
                }
                cleared.push(other);
            }
        }
        if let Some(s) = self.bindings.get_mut(&id) {
            s[slot] = binding;
        }
        Ok(cleared)
    }

    pub fn reset(&mut self, id: ActionId) -> bool {
        let Some(def) = self.def(id).copied() else {
            return false;
        };
        self.bindings.insert(id, def.defaults);
        true
    }

    pub fn reset_all(&mut self) {
        for def in self.registry.clone() {
            self.bindings.insert(def.id, def.defaults);
        }
    }

    // -- settings -------------------------------------------------------------------

    /// Only the bindings that differ from the defaults, plus the entries of actions this
    /// build does not know (carried through untouched).
    pub fn export_settings(&self) -> ControlSettings {
        let mut s = ControlSettings::default();
        for def in &self.registry {
            if let Some(b) = self.bindings.get(&def.id)
                && *b != def.defaults
            {
                s.entries.insert(def.id.0.to_string(), *b);
            }
        }
        for (k, v) in &self.foreign {
            s.entries.insert(k.clone(), *v);
        }
        s
    }

    /// Apply stored settings over the defaults. Entries that are not registered here are
    /// kept aside and reported; entries that would collide (with a default or each other)
    /// are dropped, naming them; the rest apply. Returns one line per problem.
    pub fn apply_settings(&mut self, settings: &ControlSettings) -> Vec<String> {
        let mut warnings = Vec::new();
        self.reset_all();
        self.foreign.clear();
        let mut known: BTreeMap<ActionId, Slots> = BTreeMap::new();
        for (name, slots) in &settings.entries {
            match self.id_of(name) {
                Some(id) => {
                    known.insert(id, *slots);
                }
                None => {
                    warnings.push(format!("unknown action `{name}` kept but not applied"));
                    self.foreign.insert(name.clone(), *slots);
                }
            }
        }
        // drop overrides until the whole set is conflict-free (a swap of two keys is valid
        // as a whole although each half collides with a default)
        loop {
            let mut trial = Controls::empty();
            for def in &self.registry {
                trial.register(*def);
            }
            for (id, slots) in &known {
                trial.bindings.insert(*id, *slots);
            }
            let Some(((a, _), (b, _))) = trial.all_conflicts().into_iter().next() else {
                for (id, slots) in known {
                    self.bindings.insert(id, slots);
                }
                break;
            };
            let mut dropped = false;
            for id in [a, b] {
                if known.remove(&id).is_some() {
                    warnings.push(format!(
                        "`{}` conflicts with `{}`: its stored binding was ignored",
                        id.0,
                        if id == a { b.0 } else { a.0 }
                    ));
                    dropped = true;
                }
            }
            if !dropped {
                // the defaults themselves collide: nothing stored to blame
                break;
            }
        }
        warnings
    }

    // -- evaluation -----------------------------------------------------------------

    fn neutralize_all(&mut self) {
        for s in self.state.values_mut() {
            s.just_released = s.down;
            s.just_pressed = false;
            s.down = false;
            s.value = 0.0;
            s.pressed_latch = false;
            s.released_latch = false;
            s.external = 0.0;
        }
    }

    /// Is the button down, not suppressed, and every modifier the binding asks for held?
    /// Extra modifiers do not stop a match (Shift is held while sprinting and walking); the
    /// more specific binding wins the button instead, see `evaluate`.
    fn eligible(&self, b: &Binding, frame: &InputFrame, mods: Modifiers) -> bool {
        frame.down.contains(&b.input)
            && !self.suppressed.contains(&b.input)
            && mods.without(b.input.own_modifier()).contains(b.modifiers)
    }

    /// One dispatch: fold this frame's raw input into every action's state.
    pub fn evaluate(
        &mut self,
        frame: &InputFrame,
        contexts: &[Context],
        neutralize: Option<Neutralize>,
    ) {
        for i in &frame.released {
            self.suppressed.remove(i);
        }
        if frame.authoritative_up {
            self.suppressed.retain(|i| frame.down.contains(i));
        }
        if let Some(why) = neutralize {
            if why == Neutralize::FocusLost {
                // the OS drops the releases that happen while unfocused
                self.suppressed.clear();
            } else {
                self.suppressed.extend(self.last_down.iter().copied());
                self.suppressed.extend(frame.down.iter().copied());
                for i in &frame.released {
                    self.suppressed.remove(i);
                }
            }
            self.neutralize_all();
            self.last_down = frame.down.clone();
            self.neutralized = Some(why);
            return;
        }
        self.neutralized = None;

        let now: HashSet<Context> = contexts.iter().copied().collect();
        let mut mods = Modifiers::NONE;
        for i in &frame.down {
            mods = mods.with(i.own_modifier());
        }
        // every binding that matches, most specific first (more modifiers), then the higher
        // priority context, then registry order: each physical button goes to the first
        // candidate that asks for it and nobody else sees it this frame
        let mut candidates: Vec<(u32, u8, usize, usize, Input)> = Vec::new();
        for (ri, def) in self.registry.iter().enumerate() {
            if !now.contains(&def.context) {
                continue;
            }
            let slots = self.bindings.get(&def.id).copied().unwrap_or_default();
            for (si, slot) in slots.iter().enumerate() {
                if let Some(b) = slot
                    && self.eligible(b, frame, mods)
                {
                    candidates.push((
                        b.modifiers.0.count_ones(),
                        def.context.priority(),
                        ri,
                        si,
                        b.input,
                    ));
                }
            }
        }
        candidates.sort_by_key(|c| (std::cmp::Reverse(c.0), std::cmp::Reverse(c.1), c.2, c.3));
        let mut claims: HashSet<Input> = HashSet::new();
        let mut hits: HashSet<(usize, usize)> = HashSet::new();
        for (_, _, ri, si, input) in candidates {
            if claims.insert(input) {
                hits.insert((ri, si));
            }
        }
        let mut result: HashMap<ActionId, (bool, f32)> = HashMap::new();
        for (ri, def) in self.registry.iter().enumerate() {
            let hit = |si: usize| hits.contains(&(ri, si));
            result.insert(
                def.id,
                match def.kind {
                    ActionKind::Button => {
                        let down = hit(0) || hit(1);
                        (down, if down { 1.0 } else { 0.0 })
                    }
                    ActionKind::Axis => {
                        let ext = self.state.get(&def.id).map_or(0.0, |s| s.external);
                        let v = ((hit(1) as i8 - hit(0) as i8) as f32 + ext).clamp(-1.0, 1.0);
                        (v != 0.0, v)
                    }
                },
            );
        }

        for def in &self.registry {
            let Some(s) = self.state.get_mut(&def.id) else {
                continue;
            };
            let (down, value) = result.get(&def.id).copied().unwrap_or((false, 0.0));
            if !now.contains(&def.context) {
                // context gone: released, nothing pending
                s.just_released = s.down;
                s.just_pressed = false;
                s.down = false;
                s.value = 0.0;
                s.pressed_latch = false;
                s.released_latch = false;
                continue;
            }
            // a context that has only just become active starts from what is already held:
            // held buttons read as held but did not "just press"
            let before = if self.active.contains(&def.context) {
                s.down
            } else {
                down
            };
            s.just_pressed = down && !before;
            s.just_released = !down && before;
            if s.just_pressed {
                s.pressed_latch = true;
            }
            if s.just_released {
                s.released_latch = true;
            }
            s.down = down;
            s.value = value;
        }
        self.active = now;
        self.last_down = frame.down.clone();
    }
}

// -- Bevy integration ----------------------------------------------------------------

/// The dispatch system's set: `PreUpdate`, after input and after the modal owners.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControlsDispatch;

/// Where the standalone control settings live, and how the plugin treats them.
#[derive(Default)]
pub struct ControlsPlugin {
    /// Settings file to load at startup. `None` (the default, and what tests use) keeps
    /// the defaults and never touches the disk.
    pub settings_path: Option<PathBuf>,
}

impl ControlsPlugin {
    /// Load and save the player's own file ([`settings::default_path`]).
    pub fn with_user_settings() -> Self {
        ControlsPlugin {
            settings_path: settings::default_path(),
        }
    }
}

impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        let mut controls = Controls::default();
        let mut store = SettingsStore::in_memory();
        if let Some(path) = &self.settings_path {
            let (loaded, outcome) = SettingsStore::load(path.clone());
            store = loaded;
            let mut warnings = outcome.warnings.clone();
            warnings.extend(controls.apply_settings(&outcome.settings));
            for w in &warnings {
                warn!("controls: {w}");
            }
            match &store.status {
                LoadStatus::Missing | LoadStatus::Loaded => {}
                other => warn!("controls: settings {other:?}; project defaults are in use"),
            }
        }
        app.insert_resource(controls)
            .insert_resource(store)
            .add_message::<ControlsCancel>()
            .add_systems(
                PreUpdate,
                dispatch
                    .in_set(ControlsDispatch)
                    .after(InputSystems)
                    .after(crate::map::MapInput)
                    .after(crate::blackmarket::BlackMarketInput),
            );
    }
}

/// Which contexts the hero is in right now (the same conditions `hero_input` uses to give
/// the keys to the hero, the car or the free camera).
fn contexts_for(hero: Option<&crate::character::Hero>, mode: Option<CameraMode>) -> Vec<Context> {
    let mut out = vec![Context::Global];
    if let Some(h) = hero {
        if h.in_vehicle.is_some() {
            out.push(Context::Vehicle);
        } else if h.stunt.is_none() && mode == Some(CameraMode::Hero) {
            out.push(Context::OnFoot);
        }
    }
    out
}

use crate::vehicle::CameraMode;

/// Raw device messages and their read positions.
#[derive(SystemParam)]
struct RawEvents<'w, 's> {
    key: Option<Res<'w, Messages<KeyboardInput>>>,
    mouse: Option<Res<'w, Messages<MouseButtonInput>>>,
    focus: Option<Res<'w, Messages<KeyboardFocusLost>>>,
    cancel: Option<Res<'w, Messages<ControlsCancel>>>,
    key_cursor: Local<'s, MessageCursor<KeyboardInput>>,
    mouse_cursor: Local<'s, MessageCursor<MouseButtonInput>>,
    focus_cursor: Local<'s, MessageCursor<KeyboardFocusLost>>,
    cancel_cursor: Local<'s, MessageCursor<ControlsCancel>>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn dispatch(
    mut controls: ResMut<Controls>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    mut raw: RawEvents,
    windows: Query<&Window, With<PrimaryWindow>>,
    heroes: Query<&crate::character::Hero>,
    mode: Option<Res<CameraMode>>,
    map: Option<Res<crate::map::MapState>>,
    market: Option<Res<crate::blackmarket::Market>>,
    time: Option<Res<Time<Virtual>>>,
) {
    let mut frame = InputFrame {
        authoritative_up: raw.key.is_none(),
        ..default()
    };
    if let Some(k) = &keys {
        frame.down.extend(k.get_pressed().map(|k| Input::Key(*k)));
    }
    if let Some(b) = &buttons {
        frame.down.extend(b.get_pressed().map(|b| Input::Mouse(*b)));
    }
    if let Some(m) = &raw.key {
        for e in raw.key_cursor.read(m) {
            if e.state == ButtonState::Released {
                frame.released.push(Input::Key(e.key_code));
            }
        }
    }
    if let Some(m) = &raw.mouse {
        for e in raw.mouse_cursor.read(m) {
            if e.state == ButtonState::Released {
                frame.released.push(Input::Mouse(e.button));
            }
        }
    }
    let focus_msg = raw
        .focus
        .as_ref()
        .is_some_and(|m| raw.focus_cursor.read(m).count() > 0);
    let focus_lost = focus_msg || windows.iter().any(|w| !w.focused);
    let cancelled = raw
        .cancel
        .as_ref()
        .is_some_and(|m| raw.cancel_cursor.read(m).count() > 0);

    let hero = heroes.single().ok();
    let modal = map.is_some_and(|m| m.open)
        || market.is_some_and(|m| m.open)
        // the clock the modal owners stop: no second pause state of our own
        || time.is_some_and(|t| t.is_paused());
    let neutralize = if focus_lost {
        Some(Neutralize::FocusLost)
    } else if modal {
        Some(Neutralize::Modal)
    } else if hero.is_some_and(|h| h.downed) {
        Some(Neutralize::Death)
    } else if cancelled {
        Some(Neutralize::Cancel)
    } else {
        None
    };
    let contexts = contexts_for(hero, mode.map(|m| *m));
    controls.evaluate(&frame, &contexts, neutralize);
}
