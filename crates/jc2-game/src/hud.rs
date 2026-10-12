//! The HUD of the original game, in Bevy UI: the round rotating minimap with its health
//! arc, heat glow and icons, the weapon and grenade slots with their round ammo counters,
//! the key prompts, world markers, notifications, banners, the heat banner, the damage
//! vignette and the crosshair (`research/formats/hud.md` has the art and the layout).
//!
//! The pictures are the game's own, out of `gui\main_menu.blz` (`hud_*.dds`: the
//! weapon-icon atlas, the minimap frame, the badge atlas, the slot background, the heat
//! triangles) and the minimap shows `pda_map_dif.dds` through a circular
//! [`ImageNode`] (rounded corners clip the picture) turned by the camera's heading. What
//! the game draws as Anark vectors — the health arc, the ammo dashes, the glow, the
//! vignette — is generated into images here.
//!
//! The flow: [`collect`] and its neighbours read the game's state (Rico, [`Arsenal`],
//! [`Heat`], the vehicles) into view models ([`HudView`], [`MinimapView`],
//! [`PromptList`], [`Markers`], [`Notices`] …), [`apply_hud`] writes them into the UI
//! entities built at startup. Layout is in "reference pixels" of a 2000 × 1116 frame and
//! becomes `vmin`, so it looks the same at any window size.
//!
//! Words and numbers are [`BitmapText`]s in the game's own bitmap fonts (`bitmap_text.rs`,
//! `research/formats/hud.md`): the bold sans for notifications, names and the HEAT banner,
//! bold digits in the ammo balls, the chrome
//! numerals for the CHAOS popup.
//! The old debug text of the other modules is tagged [`DevText`] and hidden until `F3`.

use crate::Game;
use crate::bitmap_text::{BitmapText, HudFont, TextAlign};
use crate::boat::Watercraft;
use crate::chaos::{Chaos, ChaosPopup, ChaosTables, Progress};
use crate::character::{ENTER_RANGE, Hero};
use crate::combat::{Arsenal, Dead, Health};
use crate::destruction::StructureHits;
use crate::explosion::Wreck;
use crate::fleet::Container;
use crate::heat::Heat;
use crate::heli::Heli;
use crate::map::{MAP_TEXTURE, MapData, bevy_to_map, map_heading};
use crate::npc::Npc;
use crate::pickups::Pickup;
use crate::plane::Plane;
use crate::stunts::{MOUNT_REACH, StuntSpots, nearest_spot};
use crate::vehicle::{CameraMode, Enterable, Occupied};
use crate::world::FlyCamera;
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, futures_lite::future};
use jc2_formats::text::TextDb;
use std::collections::{HashMap, HashSet};
use std::f32::consts::{PI, TAU};

// ---------------------------------------------------------------------------------
// layout: reference pixels of a 2000 × 1116 frame, as `vmin`
// ---------------------------------------------------------------------------------

const REF_H: f32 = 1116.0;

/// Reference pixels → `vmin` (1 % of the window's shorter side).
pub(crate) fn r(px: f32) -> Val {
    Val::VMin(px * 100.0 / REF_H)
}

/// A font size of `px` reference pixels.
pub(crate) fn rf(px: f32) -> FontSize {
    FontSize::VMin(px * 100.0 / REF_H)
}

/// The minimap's frame: outer diameter and the top-left corner, and the diameter of the
/// map circle inside it.
const MM_LEFT: f32 = 35.0;
const MM_TOP: f32 = 10.0;
const MM_SIZE: f32 = 213.0;
const MM_MAP: f32 = 184.0;
/// What the minimap shows: the radius of the picture (m) and the map's pixel size.
pub const MINIMAP_RADIUS_M: f32 = 300.0;
const MAP_PX: f32 = 2048.0;
/// Metres to a pixel of `pda_map_dif.dds`.
const MAP_M_PER_PX: f32 = 16.0;
/// Icons on the minimap at once.
const MM_ICONS: usize = 28;
/// Where the health arc starts and ends on the ring (degrees clockwise from the top):
/// the left, round the bottom, to the right.
const ARC_START_DEG: f32 = 284.0;
const ARC_END_DEG: f32 = 110.0;

/// The weapon slot (bottom right) and the grenade slot (bottom left): the background's
/// size and the centre of its ball.
const SLOT_W: f32 = 198.0;
const SLOT_H: f32 = 99.0;
const SLOT_BALL: (f32, f32) = (146.0, 48.0);
const SLOT_TOP: f32 = 983.0;
/// The ball's diameter on screen.
const BALL_D: f32 = 70.0;
/// The grenade's pixels in the weapon atlas (the cell's own 64 × 32 is mostly empty).
const GRENADE_RECT: Rect = Rect {
    min: Vec2::new(194.0, 62.0),
    max: Vec2::new(222.0, 92.0),
};

const PROMPT_ROWS: usize = 5;
const NOTICE_ROWS: usize = 5;
const MARKER_ROWS: usize = 5;

// ---------------------------------------------------------------------------------
// timings and distances
// ---------------------------------------------------------------------------------

/// How long a notification stays (s), and how long it fades in and out.
const NOTICE_LIFE: f32 = 7.0;
const NOTICE_FADE: f32 = 1.0;
/// The banner for a heat level: slide in, hold, fade (s), and the glass on the minimap.
const HEAT_BANNER_LIFE: f32 = 5.0;
const HEAT_GLASS_LIFE: f32 = 4.0;
/// The settlement banner (s).
const AREA_BANNER_LIFE: f32 = 6.0;
/// "In the last 60 seconds".
const WINDOW: f32 = 60.0;
/// The foot-machine states served by `CSkyDiveController` (native `0x0052a650`; table in
/// `foot-conditions-next.md` section 4): in these "Open parachute" is offered. `S_JUMP_TO_SKYDIVE`
/// is the base-jump controller and has no prompt.
pub const SKYDIVE_CONTROLLER_STATES: [&str; 7] = [
    "S_PULL_OPEN_PARACHUTE_VERTICAL",
    "S_SKYDIVE",
    "S_PARACHUTE_DETACH_TO_FREEFALL",
    "S_TO_SKYDIVE_DASH",
    "S_FROM_SKYDIVE_DASH",
    "S_SKYDIVE_DASH",
    "S_FALL_TO_SKYDIVE",
];

/// Whether the foot-machine state is a `CSkyDiveController` state.
pub fn is_skydive_controller_state(state: &str) -> bool {
    SKYDIVE_CONTROLLER_STATES.contains(&state)
}

/// `character+0x47c`, "parachute allowed" (native default true; `ply.parachute.disable` clears
/// it, `ply.parachute.enable` sets it, handler `0x00706c30`). Nothing writes it yet: the
/// script-event hook is not wired, so it stays at its default.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParachuteAllowed(pub bool);

impl Default for ParachuteAllowed {
    fn default() -> Self {
        Self(true)
    }
}
/// Where a vehicle's marker shows, and a soldier's (m).
const VEHICLE_MARKER_RANGE: f32 = 60.0;
const FOE_MARKER_RANGE: f32 = 120.0;
/// A pickup is "near" this close (m).
const PICKUP_NEAR: f32 = 6.0;
/// The damage vignette fades in this long (s), and "low health" starts below this.
const DAMAGE_FADE: f32 = 3.0;
const LOW_HEALTH: f32 = 0.3;
/// How far a settlement's area reaches (m) by its icon kind (assumed: the data has no
/// radius).
fn area_radius(kind: i32) -> f32 {
    match kind {
        29 => 450.0,
        24..=26 | 32 => 300.0,
        27 | 28 => 250.0,
        _ => 130.0,
    }
}

// ---------------------------------------------------------------------------------
// public pieces other modules use
// ---------------------------------------------------------------------------------

/// The developer stats (tiles, triangles, position, speed, the per-vehicle debug lines)
/// show while this is on; `F3` flips it. Off by default.
#[derive(Resource, Default, Debug)]
pub struct DevStats(pub bool);

/// Marks the old debug text of the other modules: hidden unless [`DevStats`] is on.
#[derive(Component)]
pub struct DevText;

/// `--hud-demo` (debug only): fakes heat 2, notifications, an objective, a settlement
/// banner, a prompt and two soldiers' markers so a screenshot shows them without playing.
/// With `vehicle` it shows the HUD of a car instead of the one on foot.
#[derive(Resource)]
pub struct HudDemo {
    pub vehicle: bool,
}

/// The objective line next to the minimap (`None`: hidden): the running mission's
/// (`missions.rs`); `--hud-demo` sets one.
#[derive(Resource, Default, Debug)]
pub struct Objective(pub Option<String>);

/// The game's words for the HUD, from the text database (`TextDb`); the English the code
/// falls back on is the same text.
#[derive(Resource, Default)]
pub struct HudStrings {
    db: Option<TextDb>,
}

impl HudStrings {
    /// The text under `key`, else `fallback`.
    pub fn get(&self, key: &str, fallback: &str) -> String {
        self.db
            .as_ref()
            .and_then(|d| d.get(key))
            .filter(|s| !s.is_empty())
            .unwrap_or(fallback)
            .to_string()
    }

    /// `%count` replaced.
    pub fn counted(&self, key: &str, fallback: &str, n: u32) -> String {
        self.get(key, fallback).replace("%count", &n.to_string())
    }

    /// The name of a vehicle by its container (`lave.v014_jeep.ee` → "Marten Storm III").
    pub fn vehicle_name(&self, container: &str) -> String {
        let key = vehicle_key(container);
        let pretty = key
            .split_once('_')
            .map(|(_, rest)| rest.replace('_', " "))
            .unwrap_or_else(|| key.clone());
        let mut chars = pretty.chars();
        let fallback: String = match chars.next() {
            Some(c) => c.to_uppercase().chain(chars).collect(),
            None => String::new(),
        };
        self.get(&key, &fallback)
    }
}

/// The text key of a vehicle's name: its container's middle part (`v014_jeep`).
fn vehicle_key(container: &str) -> String {
    let c = container.to_ascii_lowercase();
    let c = c.strip_suffix(".ee").unwrap_or(&c);
    c.split_once('.').map_or(c, |(_, rest)| rest).to_string()
}

/// A gun of the table: the text key of its name (`gui_txt_59`…), the English the key
/// holds and its cell in the weapon-icon atlas (`hud_weapons_a_dif.dds`, 4 × 4 cells of
/// 64 × 32).
struct GunArt {
    weapon: &'static str,
    key: &'static str,
    english: &'static str,
    cell: u32,
}

const GUNS: &[GunArt] = &[
    GunArt {
        weapon: "Beretta Handgun",
        key: "gui_txt_59",
        english: "Pistol",
        cell: 0,
    },
    GunArt {
        weapon: "Revolver",
        key: "gui_txt_60",
        english: "Revolver",
        cell: 1,
    },
    GunArt {
        weapon: "Scorpion_SMG",
        key: "gui_txt_61",
        english: "Submachine Gun",
        cell: 2,
    },
    GunArt {
        weapon: "Onehand Shotgun",
        key: "gui_txt_62",
        english: "Sawed-Off Shotgun",
        cell: 3,
    },
    GunArt {
        weapon: "GrenadeLauncher",
        key: "gui_txt_63",
        english: "Grenade Launcher",
        cell: 8,
    },
    GunArt {
        weapon: "Assault",
        key: "gui_txt_64",
        english: "Assault Rifle",
        cell: 4,
    },
    GunArt {
        weapon: "Shotgun (pump action)",
        key: "gui_txt_65",
        english: "Shotgun",
        cell: 5,
    },
    GunArt {
        weapon: "Sniper",
        key: "gui_txt_66",
        english: "Sniper Rifle",
        cell: 6,
    },
    GunArt {
        weapon: "RocketLauncher",
        key: "gui_txt_67",
        english: "Rocket Launcher",
        cell: 7,
    },
    GunArt {
        weapon: "MachineGun",
        key: "gui_txt_68",
        english: "Machine Gun",
        cell: 9,
    },
];

/// The atlas cell of a gun the table has no art for (the grenade, cell 11, is
/// [`GRENADE_RECT`]).
const GENERIC_GUN_CELL: u32 = 10;

fn gun_art(weapon: &str) -> Option<&'static GunArt> {
    GUNS.iter().find(|g| g.weapon.eq_ignore_ascii_case(weapon))
}

// ---------------------------------------------------------------------------------
// view models
// ---------------------------------------------------------------------------------

/// What the weapon slot shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SlotView {
    pub shown: bool,
    /// The weapon-icon atlas cell, if there is a gun to draw.
    pub cell: Option<u32>,
    /// The number in the ball: the magazine's rounds (or the speed in a vehicle).
    pub number: String,
    /// The share of the magazine left, 0..1 (the dashes round the ball).
    pub fraction: f32,
    pub name: String,
    pub stars_lit: u32,
    pub reloading: bool,
    pub reserve: String,
}

/// The grenade slot.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GrenadeView {
    pub shown: bool,
    pub count: u32,
    pub name: String,
}

/// Everything the HUD reads of the game this frame.
#[derive(Resource, Debug, Default)]
pub struct HudView {
    /// Rico is in play (a hero exists): the slots, the crosshair and the minimap show.
    pub active: bool,
    pub on_foot: bool,
    pub in_vehicle: bool,
    /// A gun is drawn (`Arsenal::drawn`, native holder+0x64 != 0): the weapon crosshair
    /// shows; holstered, the small plain cross does.
    pub drawn: bool,
    pub slot: SlotView,
    pub grenade: GrenadeView,
    pub health_frac: f32,
    /// The part of the arc that was just lost (red), fading.
    pub lost_frac: f32,
    pub heat_level: u32,
    pub down: bool,
}

/// The minimap: where Rico is on the map image and how the picture is turned.
#[derive(Resource, Debug, Clone, Default)]
pub struct MinimapView {
    /// Rico's pixel on `pda_map_dif.dds` (`map.rs`'s mapping); the tests pin it.
    #[allow(dead_code)]
    pub center_px: Vec2,
    /// The camera's heading as the map has it: clockwise from north (up).
    #[allow(dead_code)]
    pub heading: f32,
    /// The picture's clockwise turn on screen: the camera's forward points up.
    pub rotation: f32,
    /// The part of the map image drawn, in its pixels.
    pub rect: Rect,
    /// The part of the tile patchwork ([`MinimapAtlas`]) to draw instead, once it exists.
    pub atlas_rect: Option<Rect>,
    /// The things to draw on it, nearest first.
    pub blips: Vec<Blip>,
    /// Where the "N" sits on the ring: a unit vector, y down.
    pub north: Vec2,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Blip {
    /// In the minimap circle's units: 1 is the edge, y down.
    pub offset: Vec2,
    pub icon: String,
    pub tint: Color,
}

/// A key prompt: what it says and the key.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub key: &'static str,
}

#[derive(Resource, Default, Debug)]
pub struct PromptList(pub Vec<Prompt>);

/// What the hero is doing, as far as the prompts care.
#[derive(Debug, Clone, Copy, Default)]
pub struct PromptContext {
    pub downed: bool,
    pub transferring: bool,
    /// Supported authored destinations, not inferred proximity.
    pub transfer_target: Option<&'static str>,
    pub transfer_other: Option<&'static str>,
    pub free_camera: bool,
    pub in_vehicle: bool,
    pub vehicle_is_car: bool,
    pub on_roof: bool,
    pub on_wire: bool,
    /// Reeled in to a wall, ledge or overhang and hanging from the hook.
    pub hanging: bool,
    pub parachute: bool,
    /// The canopy is `PCS_OPEN` (out and not unfolding or closing): the GUI parachute button
    /// shows "close" (native `0x00792510`, event `gui.parachutebutton.close`).
    pub canopy_open: bool,
    /// In the air (hides the on-foot prompts).
    pub airborne: bool,
    /// The foot machine is in a skydive-controller state and the parachute is allowed:
    /// "Open parachute" (native `0x0052a650`).
    pub skydive: bool,
    pub near_vehicle: bool,
    pub near_stunt: bool,
    pub near_pickup: bool,
}

/// The GUI parachute button state (native `0x00792510`: events `gui.parachutebutton.open` /
/// `.close`), independent of the face-button prompt buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParachuteButton {
    /// `gui.parachutebutton.open`: canopy `PCS_CLOSED` inside the canopy states, or a
    /// skydive state.
    Open,
    /// `gui.parachutebutton.close`: canopy `PCS_OPEN`.
    Close,
}

/// The GUI parachute button for this context. The native `PCS_CLOSED`-inside-the-canopy-states
/// branch has no runtime counterpart (the canopy flag clears with the state), so only the
/// open-canopy and skydive branches occur.
pub fn parachute_button(c: &PromptContext) -> Option<ParachuteButton> {
    if c.parachute {
        c.canopy_open.then_some(ParachuteButton::Close)
    } else if c.skydive {
        Some(ParachuteButton::Open)
    } else {
        None
    }
}

/// The prompts that apply now, in the order they are drawn.
pub fn prompts_for(c: &PromptContext, s: &HudStrings) -> Vec<Prompt> {
    let mut out = Vec::new();
    let mut add = |label: String, key: &'static str| out.push(Prompt { label, key });
    if c.free_camera || c.downed {
        return out;
    }
    if c.in_vehicle {
        add(s.get("hud_button_03", "Exit"), "E");
        if c.vehicle_is_car {
            add("Handbrake".to_string(), "SPACE");
        }
        return out;
    }
    if c.on_roof {
        add(s.get("hud_button_06", "Jump off"), "SPACE");
        add(s.get("hud_button_02", "Enter"), "E");
        add(s.get("hud_button_08", "Drop"), "Q");
        if !c.transferring {
            if let Some(target) = c.transfer_target {
                add(format!("Transfer to {target}"), "Z");
            }
            if let Some(target) = c.transfer_other {
                add(format!("Transfer to {target}"), "SHIFT+Z");
            }
        }
        return out;
    }
    if c.hanging {
        // the original's reeled-in prompts: jump (or climb) and drop
        add(s.get("hud_button_09", "Jump"), "SPACE");
        add(s.get("hud_button_08", "Drop"), "LCONTROL");
        return out;
    }
    if c.parachute {
        if c.on_wire {
            // the GUI parachute button (`gui.parachutebutton.close`) is a second producer next to
            // the face-button buffer, so "Close parachute SPACE" and "Reel in E" are on screen
            // together (research 4.1). INFERRED: the button's label/key are read from the
            // reference shots; the GUI package is unparsed. Without a wire the buffer already
            // posts the same text, so the duplicate is dropped.
            if parachute_button(c) == Some(ParachuteButton::Close) {
                add(s.get("hud_button_05", "Close parachute"), "SPACE");
            }
            // under the canopy with a wire attached (native: wire pointer non-zero)
            add(s.get("hud_button_07", "Reel in"), "E");
        } else {
            add(s.get("hud_button_05", "Close parachute"), "SPACE");
        }
    } else if c.on_wire || c.skydive {
        // a skydive-controller state, and on the wire (the reeled-flight entry)
        add(s.get("hud_button_04", "Open parachute"), "SPACE");
    }
    if c.on_wire {
        add(s.get("hud_button_08", "Drop"), "F");
    }
    if !c.airborne && !c.on_wire && !c.parachute {
        if c.near_vehicle {
            add(s.get("hud_button_02", "Enter"), "E");
        }
        if c.near_stunt {
            add(s.get("fe_txt_156", "Stunt jump"), "Q");
        }
        if c.near_pickup {
            // weapons are taken by walking over them: there is no key
            add(s.get("hud_button_12", "Pick up"), "WALK");
        }
    }
    out
}

/// One world marker: where, what it says, what colour, and whether it is a foe.
#[derive(Debug, Clone)]
pub struct MarkerView {
    pub world: Vec3,
    pub label: String,
    pub foe: bool,
}

#[derive(Resource, Default, Debug)]
pub struct Markers(pub Vec<MarkerView>);

/// World markers other modules ask for (a mission's targets, a colonel: `missions.rs`),
/// drawn before the HUD's own.
#[derive(Resource, Default, Debug)]
pub struct ExtraMarkers(pub Vec<MarkerView>);

/// Minimap icons other modules ask for: where (Bevy axes) and the icon texture
/// (`hud_icon_objective_dif.dds`); kept on the ring when out of range.
#[derive(Resource, Default, Debug)]
pub struct ExtraBlips(pub Vec<(Vec3, String)>);

// ----- notifications -----

#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub text: String,
    pub age: f32,
}

#[derive(Resource, Default, Debug)]
pub struct Notices(pub Vec<Notice>);

impl Notices {
    pub fn push(&mut self, text: impl Into<String>) {
        self.0.push(Notice {
            text: text.into(),
            age: 0.0,
        });
        // the stack is as tall as the rows: the oldest goes
        while self.0.len() > NOTICE_ROWS {
            self.0.remove(0);
        }
    }

    pub fn tick(&mut self, dt: f32) {
        for n in &mut self.0 {
            n.age += dt;
        }
        self.0.retain(|n| n.age < NOTICE_LIFE);
    }
}

/// How opaque a notice is at `age`: fades in, holds, fades out.
pub fn notice_alpha(age: f32) -> f32 {
    let fade_in = (age / 0.25).clamp(0.0, 1.0);
    let fade_out = ((NOTICE_LIFE - age) / NOTICE_FADE).clamp(0.0, 1.0);
    fade_in.min(fade_out)
}

/// The heat banner: which level it announces and how long ago.
#[derive(Resource, Debug)]
pub struct HeatBanner {
    pub level: u32,
    pub age: f32,
    /// The level seen last frame.
    seen: u32,
}

impl Default for HeatBanner {
    fn default() -> Self {
        Self {
            level: 0,
            age: f32::INFINITY,
            seen: 0,
        }
    }
}

impl HeatBanner {
    /// The heat is now at `level`: a rise starts the banner.
    pub fn update(&mut self, level: u32, dt: f32) {
        if level > self.seen {
            self.level = level;
            self.age = 0.0;
        } else {
            self.age += dt;
        }
        self.seen = level;
    }

    pub fn active(&self) -> bool {
        self.age < HEAT_BANNER_LIFE
    }

    /// 0..1: opacity of the big text (slides in, holds, fades).
    pub fn alpha(&self) -> f32 {
        if !self.active() {
            return 0.0;
        }
        let fade_in = (self.age / 0.4).clamp(0.0, 1.0);
        let fade_out = ((HEAT_BANNER_LIFE - self.age) / 1.2).clamp(0.0, 1.0);
        fade_in.min(fade_out)
    }

    /// How far the text has slid in, 0 (off the left edge) … 1.
    pub fn slide(&self) -> f32 {
        let t = (self.age / 0.5).clamp(0.0, 1.0);
        1.0 - (1.0 - t) * (1.0 - t)
    }

    /// The glass on the minimap with "HEAT Level N".
    pub fn glass_alpha(&self) -> f32 {
        if self.age >= HEAT_GLASS_LIFE {
            return 0.0;
        }
        let fade_in = (self.age / 0.3).clamp(0.0, 1.0);
        let fade_out = ((HEAT_GLASS_LIFE - self.age) / 0.8).clamp(0.0, 1.0);
        fade_in.min(fade_out)
    }
}

/// A settlement banner: the name, and how long ago it appeared.
#[derive(Resource, Debug, Default)]
pub struct AreaBanner {
    pub name: String,
    pub icon: Option<String>,
    /// The settlement's id (`31x14`), for its completion.
    pub id: String,
    pub age: f32,
    pub active: bool,
}

impl AreaBanner {
    pub fn alpha(&self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let fade_in = (self.age / 0.3).clamp(0.0, 1.0);
        let fade_out = ((AREA_BANNER_LIFE - self.age) / NOTICE_FADE).clamp(0.0, 1.0);
        fade_in.min(fade_out)
    }
}

/// The settlement Rico stands in (its name, icon and id), for the little panel under
/// the minimap.
#[derive(Resource, Debug, Default)]
pub struct InArea(pub Option<(String, Option<String>, String)>);

/// Counters behind the notifications.
#[derive(Resource, Default, Debug)]
pub struct HudStats {
    /// The HUD's clock (s).
    pub clock: f32,
    pub kill_times: Vec<f32>,
    pub destroyed_times: Vec<f32>,
    seen_kills: u32,
    seen_destroyed: u32,
    seen_hijacks: u32,
    pub vehicles_driven: HashSet<String>,
    pub locations: HashSet<String>,
    last_vehicle: Option<Entity>,
    area: Option<String>,
}

/// How many of `times` fall in the 60 s before `now`.
pub fn count_recent(times: &[f32], now: f32) -> u32 {
    times.iter().filter(|t| now - **t < WINDOW).count() as u32
}

/// The count worth announcing when it goes from `prev` to `now`: 3, then every 5.
pub fn milestone_in(prev: u32, now: u32) -> Option<u32> {
    (prev + 1..=now)
        .rev()
        .find(|c| *c == 3 || (*c >= 5 && c % 5 == 0))
}

/// The damage flash: the hero's health last frame and the red that is left.
#[derive(Resource, Debug, Default)]
pub struct DamageFlash {
    pub last_hp: Option<f32>,
    /// 0..1, fading over [`DAMAGE_FADE`] seconds.
    pub intensity: f32,
    /// The health lost recently, as a share of the maximum, for the arc's red part.
    pub lost: f32,
}

impl DamageFlash {
    pub fn update(&mut self, hp: f32, max: f32, dt: f32) {
        let max = max.max(1.0);
        if let Some(last) = self.last_hp
            && hp < last - 0.5
        {
            let share = (last - hp) / max;
            self.intensity = (self.intensity + 0.45 + share * 2.0).min(1.0);
            self.lost = (self.lost + share).min(1.0);
        }
        self.last_hp = Some(hp);
        self.intensity = (self.intensity - dt / DAMAGE_FADE).max(0.0);
        self.lost = (self.lost - dt * 0.12).max(0.0);
        self.lost = self.lost.min(1.0 - (hp / max).clamp(0.0, 1.0));
    }

    /// The vignette's opacity: the flash, plus a steady heavy one at low health.
    pub fn vignette(&self, health_frac: f32) -> f32 {
        let low = if health_frac < LOW_HEALTH {
            0.85 * (1.0 - health_frac / LOW_HEALTH)
        } else {
            0.0
        };
        self.intensity.max(low).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------------
// geometry the tests pin
// ---------------------------------------------------------------------------------

/// The unit vector `a` radians clockwise from up, on screen (y down).
pub fn ring_point(a: f32) -> Vec2 {
    Vec2::new(a.sin(), -a.cos())
}

/// Where a point `rel` metres from Rico (x east, y south, as the map has it) lands on the
/// minimap, in circle units (1 = the edge) with y down, for the picture turned by
/// `rotation` clockwise.
pub fn minimap_offset(rel: Vec2, rotation: f32, radius_m: f32) -> Vec2 {
    let (s, c) = rotation.sin_cos();
    Vec2::new(rel.x * c - rel.y * s, rel.x * s + rel.y * c) / radius_m
}

/// The point on the screen's edge (inset by `margin`) in direction `dir` from the centre.
pub fn edge_point(dir: Vec2, size: Vec2, margin: f32) -> Vec2 {
    let half = size / 2.0 - Vec2::splat(margin);
    let d = if dir.length_squared() < 1e-9 {
        Vec2::NEG_Y
    } else {
        dir.normalize()
    };
    let tx = if d.x.abs() > 1e-6 {
        half.x / d.x.abs()
    } else {
        f32::MAX
    };
    let ty = if d.y.abs() > 1e-6 {
        half.y / d.y.abs()
    } else {
        f32::MAX
    };
    size / 2.0 + d * tx.min(ty)
}

/// The minimap's numbers for Rico at `pos`, the camera looking along `forward`.
pub fn minimap_for(pos: Vec3, forward: Vec3) -> MinimapView {
    let uv = bevy_to_map(pos);
    let center_px = uv * MAP_PX;
    let heading = map_heading(forward);
    let rotation = -heading;
    let half = MINIMAP_RADIUS_M / MAP_M_PER_PX;
    MinimapView {
        center_px,
        heading,
        rotation,
        rect: Rect::from_center_half_size(center_px, Vec2::splat(half)),
        atlas_rect: None,
        blips: Vec::new(),
        north: ring_point(-heading),
    }
}

// ---------------------------------------------------------------------------------
// generated art
// ---------------------------------------------------------------------------------

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn rgba_image(w: u32, h: u32, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

fn put(data: &mut [u8], n: usize, x: usize, y: usize, c: [f32; 4]) {
    let i = (y * n + x) * 4;
    data[i] = (c[0].clamp(0.0, 1.0) * 255.0) as u8;
    data[i + 1] = (c[1].clamp(0.0, 1.0) * 255.0) as u8;
    data[i + 2] = (c[2].clamp(0.0, 1.0) * 255.0) as u8;
    data[i + 3] = (c[3].clamp(0.0, 1.0) * 255.0) as u8;
}

/// The health arc over the ring: cyan for the health left, red for what was just lost,
/// grey for the rest, on the ring's band from the left round the bottom to the right.
/// `n` pixels square, the ring's outer edge at the image's.
pub fn arc_pixels(n: usize, fill: f32, lost: f32) -> Vec<u8> {
    let mut data = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    let span = ARC_START_DEG - ARC_END_DEG;
    let fill = fill.clamp(0.0, 1.0);
    let lost = lost.clamp(0.0, 1.0 - fill);
    let low = fill < LOW_HEALTH;
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 + 0.5 - c) / c;
            let dy = (y as f32 + 0.5 - c) / c;
            let rad = (dx * dx + dy * dy).sqrt();
            // the band, with soft edges and a faint glow outside it
            let band = smooth(0.918, 0.926, rad) * (1.0 - smooth(0.946, 0.954, rad));
            let groove = smooth(0.905, 0.915, rad) * (1.0 - smooth(0.957, 0.967, rad));
            let glow = (1.0 - smooth(0.0, 0.05, (rad - 0.936).abs())) * 0.22;
            let cover = band.max(glow);
            let deg = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
            let t = (ARC_START_DEG - deg) / span;
            if !(-0.004..=1.004).contains(&t) {
                continue;
            }
            if groove > 0.0 {
                // the dark groove the arc runs in
                put(&mut data, n, x, y, [0.0, 0.01, 0.02, groove * 0.7]);
            }
            if cover <= 0.0 {
                continue;
            }
            let colour = if t <= fill {
                if low {
                    [1.0, 0.45, 0.25]
                } else {
                    [0.1, 0.95, 0.85]
                }
            } else if t <= fill + lost {
                [0.95, 0.12, 0.1]
            } else {
                [0.72, 0.74, 0.78]
            };
            let a = if t <= fill + lost { cover } else { cover * 0.8 };
            put(&mut data, n, x, y, [colour[0], colour[1], colour[2], a]);
        }
    }
    data
}

/// The dashes round the ammo ball: `dashes` of them, the first `lit` bright.
pub fn dash_pixels(n: usize, dashes: usize, lit: usize) -> Vec<u8> {
    let mut data = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 + 0.5 - c) / c;
            let dy = (y as f32 + 0.5 - c) / c;
            let rad = (dx * dx + dy * dy).sqrt();
            let band = smooth(0.82, 0.86, rad) * (1.0 - smooth(0.95, 0.99, rad));
            if band <= 0.0 {
                continue;
            }
            let a = dx.atan2(-dy).rem_euclid(TAU) / TAU * dashes as f32;
            let k = a.floor() as usize;
            let within = a - a.floor();
            // each dash fills 70 % of its slot
            let dash = smooth(0.12, 0.18, within) * (1.0 - smooth(0.82, 0.88, within));
            if dash <= 0.0 {
                continue;
            }
            let on = k < lit;
            let (col, alpha) = if on {
                ([1.0, 1.0, 1.0], 1.0)
            } else {
                ([0.7, 0.75, 0.8], 0.4)
            };
            put(
                &mut data,
                n,
                x,
                y,
                [col[0], col[1], col[2], band * dash * alpha],
            );
        }
    }
    data
}

/// A soft round glow (the minimap's heat glow).
fn glow_pixels(n: usize) -> Vec<u8> {
    let mut data = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 + 0.5 - c) / c;
            let dy = (y as f32 + 0.5 - c) / c;
            let rad = (dx * dx + dy * dy).sqrt();
            // brightest just outside the ring (0.82 of the image), gone at the edge
            let a = smooth(0.55, 0.8, rad) * (1.0 - smooth(0.82, 1.0, rad));
            put(&mut data, n, x, y, [0.75, 0.9, 1.0, a]);
        }
    }
    data
}

/// The damage vignette: clear in the middle, opaque at the corners.
fn vignette_pixels(n: usize) -> Vec<u8> {
    let mut data = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 + 0.5 - c) / c;
            let dy = (y as f32 + 0.5 - c) / c;
            let d = (dx * dx + dy * dy).sqrt() / std::f32::consts::SQRT_2;
            let a = smooth(0.32, 1.0, d).powf(1.3);
            put(&mut data, n, x, y, [1.0, 1.0, 1.0, a]);
        }
    }
    data
}

/// The arrow at the minimap's centre: black with a white edge, pointing up.
fn arrow_pixels(n: usize) -> Vec<u8> {
    let mut data = vec![0u8; n * n * 4];
    let s = n as f32;
    let (apex, left, right, notch) = (
        Vec2::new(0.5, 0.04) * s,
        Vec2::new(0.14, 0.94) * s,
        Vec2::new(0.86, 0.94) * s,
        Vec2::new(0.5, 0.72) * s,
    );
    let edge = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
    let in_tri = |p: Vec2, a: Vec2, b: Vec2, c: Vec2| {
        let d = [edge(a, b, p), edge(b, c, p), edge(c, a, p)];
        d.iter().all(|&v| v >= 0.0) || d.iter().all(|&v| v <= 0.0)
    };
    let inside = |p: Vec2| in_tri(p, apex, right, notch) || in_tri(p, apex, notch, left);
    let deep = |p: Vec2, k: f32| {
        inside(p)
            && (0..8).all(|i| {
                let a = i as f32 * PI / 4.0;
                inside(p + k * Vec2::new(a.cos(), a.sin()))
            })
    };
    for y in 0..n {
        for x in 0..n {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let k = s * 0.05;
            if deep(p, k) {
                put(&mut data, n, x, y, [0.02, 0.02, 0.03, 1.0]);
            } else if inside(p) {
                put(&mut data, n, x, y, [0.95, 0.95, 0.95, 1.0]);
            }
        }
    }
    data
}

/// The green four-lobed cross at the bottom left of the ring.
fn plus_pixels(n: usize) -> Vec<u8> {
    let mut data = vec![0u8; n * n * 4];
    let c = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let dx = ((x as f32 + 0.5 - c) / c).abs();
            let dy = ((y as f32 + 0.5 - c) / c).abs();
            // two crossing bars, thinner towards their ends
            let arm = |along: f32, across: f32| {
                let w = 0.30 * (1.0 - along * 0.7);
                (1.0 - smooth(w - 0.05, w + 0.05, across)) * (1.0 - smooth(0.82, 0.95, along))
            };
            let core = arm(dx, dy).max(arm(dy, dx));
            let halo = (1.0 - smooth(0.0, 0.55, (dx.max(dy) - 0.2).max(0.0))) * 0.25;
            let a = core.max(halo);
            let inner = (core * 0.55).min(0.55);
            put(&mut data, n, x, y, [0.2 + inner, 1.0, 0.35 + inner, a]);
        }
    }
    data
}

// ---------------------------------------------------------------------------------
// the plugin
// ---------------------------------------------------------------------------------

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::bitmap_text::BitmapTextPlugin)
            .init_resource::<DevStats>()
            .init_resource::<HudStrings>()
            .init_resource::<HudView>()
            .init_resource::<MinimapView>()
            .init_resource::<PromptList>()
            .init_resource::<ParachuteAllowed>()
            .init_resource::<Markers>()
            .init_resource::<ExtraMarkers>()
            .init_resource::<ExtraBlips>()
            .init_resource::<Notices>()
            .init_resource::<HeatBanner>()
            .init_resource::<AreaBanner>()
            .init_resource::<InArea>()
            .init_resource::<HudStats>()
            .init_resource::<DamageFlash>()
            .init_resource::<Objective>()
            .init_resource::<CameraMode>()
            .add_systems(
                Startup,
                (load_strings, build_hud, build_chaos_hud, setup_atlas).chain(),
            )
            .add_systems(Update, tint_crosshair.after(apply_hud))
            .add_systems(
                Update,
                (
                    toggle_dev_stats,
                    collect,
                    track_events,
                    update_atlas,
                    compute_minimap,
                    compute_prompts.in_set(PromptComputed),
                    compute_markers,
                    demo,
                    apply_hud.in_set(HudApplied),
                    apply_chaos_hud,
                )
                    .chain(),
            );
    }
}

fn load_strings(mut strings: ResMut<HudStrings>, game: Option<Res<Game>>) {
    if let Some(g) = game {
        strings.db = TextDb::load(&g.fs, "en");
        if strings.db.is_none() {
            warn!("hud: text database not readable, using the built-in English");
        }
    }
}

/// `F3` flips the developer stats; the old debug text follows.
fn toggle_dev_stats(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut dev: ResMut<DevStats>,
    mut texts: Query<&mut Visibility, With<DevText>>,
) {
    if keys.is_some_and(|k| k.just_pressed(KeyCode::F3)) {
        dev.0 = !dev.0;
        info!("hud: developer stats {}", if dev.0 { "on" } else { "off" });
    }
    let want = if dev.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut texts {
        v.set_if_neq(want);
    }
}

// ---------------------------------------------------------------------------------
// the UI: built once
// ---------------------------------------------------------------------------------

/// The textures of the HUD (the game's own, or transparent where they are missing).
#[derive(Resource, Clone)]
pub struct HudArt {
    pub weapons: Handle<Image>,
    pub frame: Handle<Image>,
    pub tin: Handle<Image>,
    pub wea_bg: Handle<Image>,
    pub clamp: Handle<Image>,
    pub alpha: Handle<Image>,
    pub heat: Handle<Image>,
    pub star: Handle<Image>,
    pub map: Handle<Image>,
    pub north: Handle<Image>,
}

/// The entities of the UI and its generated images.
#[derive(Resource)]
pub struct HudNodes {
    pub minimap_map: Entity,
    pub glow: Entity,
    pub arc_image: Handle<Image>,
    pub north: Entity,
    pub heat_glass: Entity,
    pub heat_glass_title: Entity,
    pub heat_glass_level: Entity,
    pub triangle: Entity,
    pub icons: Vec<Entity>,
    pub slot: Entity,
    pub slot_icon: Entity,
    pub slot_number: Entity,
    pub ring_image: Handle<Image>,
    pub slot_stars: Vec<Entity>,
    pub slot_reserve: Entity,
    pub name_pill: Entity,
    pub name_text: Entity,
    pub grenade: Entity,
    pub grenade_icon: Entity,
    pub grenade_count: Entity,
    pub grenade_pill: Entity,
    pub grenade_text: Entity,
    pub prompts: Vec<(Entity, Entity)>,
    pub notices: Vec<(Entity, Entity)>,
    pub markers: Vec<MarkerUi>,
    pub vignette: Entity,
    pub desat: Entity,
    pub crosshair: Entity,
    /// The small plain cross shown with no gun drawn.
    pub crosshair_plain: Entity,
    pub heat_big: Entity,
    pub heat_big_text: Entity,
    pub heat_big_level: Entity,
    pub area_banner: Entity,
    pub area_icon: Entity,
    pub area_name: Entity,
    pub area_panel: Entity,
    pub area_panel_icon: Entity,
    pub area_panel_text: Entity,
    /// "N % complete" in the settlement banner.
    pub area_complete: Entity,
    pub objective: Entity,
    pub objective_text: Entity,
    pub down_text: Entity,
}

pub struct MarkerUi {
    pub root: Entity,
    pub ring: Entity,
    pub label_text: Entity,
    pub chevron: Entity,
}

/// The dark pill behind prompts and labels, with its pale rim.
pub(crate) fn pill(alpha: f32) -> (BackgroundColor, BorderColor) {
    (
        BackgroundColor(Color::srgba(0.07, 0.09, 0.12, alpha)),
        rim(),
    )
}

/// The lighter slate of the weapon slots.
fn slate(alpha: f32) -> BackgroundColor {
    BackgroundColor(Color::srgba(0.2, 0.25, 0.31, alpha))
}

pub(crate) fn rim() -> BorderColor {
    BorderColor::all(Color::srgba(0.85, 0.9, 0.95, 0.4))
}

/// Text without a shadow, for what fades (a shadow would not fade with it).
pub(crate) fn plain_text(size: f32) -> (TextFont, TextColor) {
    (
        TextFont {
            font_size: rf(size),
            ..default()
        },
        TextColor(Color::WHITE),
    )
}

pub(crate) fn white_text(size: f32) -> (TextFont, TextColor, TextShadow) {
    (
        TextFont {
            font_size: rf(size),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow {
            offset: Vec2::splat(1.0),
            color: Color::srgba(0.0, 0.0, 0.0, 0.8),
        },
    )
}

/// Text in the game's own bitmap font `font`, `k` times its native size (the original
/// draws them 1:1 on its frames; see [`HudFont::native`]), white, no shadow: for what
/// fades (a shadow is faded with the text by [`BitmapText`] anyway, but plain is lighter).
pub(crate) fn bt(font: HudFont, k: f32) -> BitmapText {
    BitmapText::new(font, "", font.native() * k)
}

/// [`bt`] with the soft drop shadow the Anark HUD gives its words.
pub(crate) fn bts(font: HudFont, k: f32) -> BitmapText {
    bt(font, k).with_shadow(1.0, Color::srgba(0.0, 0.0, 0.0, 0.8))
}

/// An absolute node at `(left, top)` of `w × h` reference pixels.
fn at(left: f32, top: f32, w: f32, h: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: r(left),
        top: r(top),
        width: r(w),
        height: r(h),
        ..default()
    }
}

pub(crate) fn atlas(image: &Handle<Image>, x: f32, y: f32, w: f32, h: f32) -> ImageNode {
    ImageNode {
        image: image.clone(),
        rect: Some(Rect::new(x, y, x + w, y + h)),
        ..default()
    }
}

/// Spawn `bundle` as a child of `parent`.
pub(crate) fn child(commands: &mut Commands, parent: Entity, bundle: impl Bundle) -> Entity {
    let e = commands.spawn(bundle).id();
    commands.entity(parent).add_child(e);
    e
}

fn build_hud(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    assets: Option<Res<AssetServer>>,
    game: Option<Res<Game>>,
) {
    let blank = ImageNode::default().image;
    let tex = |container: &str, member: &str| -> Handle<Image> {
        match (&assets, &game) {
            (Some(a), Some(g)) if g.fs.contains(container) => {
                a.load(format!("jc2://{container}/{member}"))
            }
            _ => blank.clone(),
        }
    };
    let top = |name: &str| -> Handle<Image> {
        match (&assets, &game) {
            (Some(a), Some(g)) if g.fs.contains(name) => a.load(format!("jc2://{name}")),
            _ => blank.clone(),
        }
    };
    let art = HudArt {
        weapons: tex("main_menu.blz", "hud_weapons_a_dif.dds"),
        frame: tex("main_menu.blz", "hud_mm_frame_dif.dds"),
        tin: tex("main_menu.blz", "hud_tin_dif.dds"),
        wea_bg: tex("main_menu.blz", "hud_wea_bg_dif.dds"),
        clamp: tex("main_menu.blz", "hud_temp_clamp_dif.dds"),
        alpha: tex("main_menu.blz", "hud_alpha_a_dif.dds"),
        heat: tex("main_menu.blz", "hud_heat_dif.dds"),
        star: tex("main_menu.blz", "temp_star_dif.dds"),
        map: top(MAP_TEXTURE),
        north: top("pda_icon_compassnorth_dif.dds"),
    };
    let arc_image = images.add(rgba_image(256, 256, arc_pixels(256, 1.0, 0.0)));
    let ring_image = images.add(rgba_image(96, 96, dash_pixels(96, 12, 12)));
    let glow_image = images.add(rgba_image(128, 128, glow_pixels(128)));
    let vignette_image = images.add(rgba_image(64, 64, vignette_pixels(64)));
    let arrow_image = images.add(rgba_image(48, 48, arrow_pixels(48)));
    let plus_image = images.add(rgba_image(48, 48, plus_pixels(48)));

    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            GlobalZIndex(10),
            Name::new("hud"),
        ))
        .id();
    // full-screen damage layers, under everything else
    let vignette = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode {
                image: vignette_image,
                color: Color::srgba(0.8, 0.02, 0.02, 0.0),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Name::new("hud: vignette"),
        ),
    );
    let desat = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.42, 0.40, 0.40, 0.0)),
            Name::new("hud: desaturate"),
        ),
    );

    // ----- the minimap -----
    let mm = child(
        &mut commands,
        root,
        (
            at(MM_LEFT, MM_TOP, MM_SIZE, MM_SIZE),
            Name::new("hud: minimap"),
        ),
    );
    let glow = child(
        &mut commands,
        mm,
        (
            at(-40.0, -40.0, MM_SIZE + 80.0, MM_SIZE + 80.0),
            ImageNode {
                image: glow_image,
                color: Color::srgba(1.0, 1.0, 1.0, 0.0),
                ..default()
            },
        ),
    );
    // the ears of the frame: four steel pads in the corners, behind the ring
    for (x, y) in [
        (0.0, 8.0),
        (MM_SIZE - 58.0, 8.0),
        (0.0, MM_SIZE - 66.0),
        (MM_SIZE - 58.0, MM_SIZE - 66.0),
    ] {
        child(
            &mut commands,
            mm,
            (
                Node {
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(r(16.0)),
                    ..at(x, y, 58.0, 58.0)
                },
                BackgroundColor(Color::srgba(0.13, 0.17, 0.2, 0.94)),
                BorderColor::all(Color::srgba(0.5, 0.58, 0.66, 0.55)),
            ),
        );
    }
    let inset = (MM_SIZE - MM_MAP) / 2.0;
    let map_node = child(
        &mut commands,
        mm,
        (
            Node {
                border_radius: BorderRadius::MAX,
                ..at(inset, inset, MM_MAP, MM_MAP)
            },
            ImageNode {
                image: art.map.clone(),
                rect: Some(Rect::from_center_half_size(
                    Vec2::splat(MAP_PX / 2.0),
                    Vec2::splat(MINIMAP_RADIUS_M / MAP_M_PER_PX),
                )),
                color: Color::srgb(0.88, 0.88, 0.88),
                ..default()
            },
            UiTransform::IDENTITY,
            Name::new("hud: minimap map"),
        ),
    );
    // a plain circle behind the map for where the picture is missing
    commands
        .entity(map_node)
        .insert(BackgroundColor(Color::srgb(0.05, 0.1, 0.14)));
    // the icon pool, then Rico's arrow
    let icons_layer = child(
        &mut commands,
        mm,
        (
            at(inset, inset, MM_MAP, MM_MAP),
            Name::new("hud: minimap icons"),
        ),
    );
    let mut icons = Vec::new();
    for i in 0..MM_ICONS {
        let e = child(
            &mut commands,
            icons_layer,
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(50.0),
                    top: Val::Percent(50.0),
                    width: r(20.0),
                    height: r(20.0),
                    margin: UiRect::all(r(-10.0)),
                    display: Display::None,
                    ..default()
                },
                ImageNode::default(),
                Name::new(format!("hud: blip {i}")),
            ),
        );
        icons.push(e);
    }
    child(
        &mut commands,
        icons_layer,
        (
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                width: r(22.0),
                height: r(22.0),
                margin: UiRect::all(r(-11.0)),
                ..default()
            },
            ImageNode::new(arrow_image),
        ),
    );
    child(
        &mut commands,
        mm,
        (
            at(0.0, 0.0, MM_SIZE, MM_SIZE),
            atlas(&art.frame, 7.0, 7.0, 139.0, 140.0),
            Name::new("hud: minimap ring"),
        ),
    );
    child(
        &mut commands,
        mm,
        (
            at(0.0, 0.0, MM_SIZE, MM_SIZE),
            ImageNode::new(arc_image.clone()),
            Name::new("hud: health arc"),
        ),
    );
    let north = child(
        &mut commands,
        mm,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(MM_SIZE / 2.0),
                top: r(MM_SIZE / 2.0),
                width: r(30.0),
                height: r(30.0),
                margin: UiRect::all(r(-15.0)),
                ..default()
            },
            ImageNode::new(art.north.clone()),
        ),
    );
    child(
        &mut commands,
        mm,
        (
            at(8.0, MM_SIZE - 40.0, 30.0, 30.0),
            ImageNode::new(plus_image),
        ),
    );
    let triangle = child(
        &mut commands,
        mm,
        (
            at(MM_SIZE - 38.0, MM_SIZE - 40.0, 30.0, 30.0),
            atlas(&art.heat, 0.0, 0.0, 32.0, 32.0),
        ),
    );
    // "HEAT Level N" on dark glass over the map while the banner runs
    let glass = child(
        &mut commands,
        mm,
        (
            Node {
                border_radius: BorderRadius::MAX,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..at(inset, inset, MM_MAP, MM_MAP)
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)),
            Name::new("hud: heat glass"),
        ),
    );
    let glass_ink = Color::srgba(0.8, 0.82, 0.85, 0.0);
    let heat_glass_title = child(
        &mut commands,
        glass,
        bt(HudFont::Bold, 1.9)
            .with_colour(glass_ink)
            .with_text("HEAT"),
    );
    let heat_glass_level = child(
        &mut commands,
        glass,
        bt(HudFont::Bold, 1.3).with_colour(glass_ink),
    );

    // the settlement panel under the minimap
    let area_panel = child(
        &mut commands,
        root,
        (
            Node {
                align_items: AlignItems::Center,
                column_gap: r(8.0),
                display: Display::None,
                ..at(103.0, 231.0, 150.0, 30.0)
            },
            Name::new("hud: area panel"),
        ),
    );
    let area_panel_icon = child(
        &mut commands,
        area_panel,
        (
            Node {
                width: r(27.0),
                height: r(27.0),
                ..default()
            },
            ImageNode::default(),
        ),
    );
    let area_panel_text = child(&mut commands, area_panel, bts(HudFont::Bold, 1.05));

    // ----- the objective line -----
    let objective = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(224.0),
                top: r(12.0),
                height: r(36.0),
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(r(14.0)),
                column_gap: r(8.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(r(18.0)),
                display: Display::None,
                ..default()
            },
            pill(0.55),
            Name::new("hud: objective"),
        ),
    );
    child(
        &mut commands,
        objective,
        (
            Node {
                width: r(26.0),
                height: r(26.0),
                ..default()
            },
            atlas(&art.tin, 0.0, 0.0, 40.0, 40.0),
        ),
    );
    let objective_text = child(&mut commands, objective, bts(HudFont::Bold, 1.18));

    // ----- notifications (top, right of the minimap) -----
    let mut notices = Vec::new();
    for i in 0..NOTICE_ROWS {
        let row = child(
            &mut commands,
            root,
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: r(258.0 + 15.0 * i as f32),
                    top: r(56.0 + 37.0 * i as f32),
                    height: r(32.0),
                    align_items: AlignItems::Center,
                    column_gap: r(8.0),
                    padding: UiRect {
                        left: r(4.0),
                        right: r(18.0),
                        ..default()
                    },
                    border_radius: BorderRadius::all(r(16.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.16, 0.3, 0.7)),
                Name::new(format!("hud: notice {i}")),
            ),
        );
        child(
            &mut commands,
            row,
            (
                Node {
                    width: r(26.0),
                    height: r(26.0),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.25, 0.4, 0.85)),
            ),
        );
        let text = child(&mut commands, row, bt(HudFont::Bold, 1.0));
        notices.push((row, text));
    }
    // the settlement banner, below them
    let area_banner = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(262.0),
                top: r(240.0),
                height: r(78.0),
                align_items: AlignItems::Center,
                column_gap: r(12.0),
                padding: UiRect::all(r(8.0)),
                border_radius: BorderRadius::all(r(8.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.06, 0.08, 0.6)),
            Name::new("hud: settlement banner"),
        ),
    );
    let area_icon = child(
        &mut commands,
        area_banner,
        (
            Node {
                width: r(64.0),
                height: r(64.0),
                ..default()
            },
            ImageNode::default(),
        ),
    );
    let area_col = child(
        &mut commands,
        area_banner,
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::right(r(16.0)),
            ..default()
        },
    );
    let area_name = child(&mut commands, area_col, bt(HudFont::Bold, 1.2));
    let area_complete = child(
        &mut commands,
        area_col,
        bt(HudFont::Bold, 1.0).with_text("0 % complete"),
    );

    // ----- the heat banner at the left of the screen -----
    let heat_big = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(-150.0),
                top: r(190.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            Name::new("hud: heat banner"),
        ),
    );
    let heat_ink = Color::srgba(1.0, 1.0, 1.0, 0.0);
    let heat_big_text = child(
        &mut commands,
        heat_big,
        bt(HudFont::Bold, 13.5)
            .with_colour(heat_ink)
            .with_text("HEAT"),
    );
    let heat_big_level = child(
        &mut commands,
        heat_big,
        bt(HudFont::Bold, 7.9).with_colour(heat_ink),
    );

    // ----- the weapon slot (bottom right) -----
    // a slate pill with the glass ball of `hud_wea_bg_dif` at its right end, the round
    // counter in the ball with its dashes, the gun's icon, six stars
    let slot_right = 6.0;
    let slot = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                right: r(slot_right),
                top: r(SLOT_TOP),
                width: r(SLOT_W),
                height: r(SLOT_H),
                ..default()
            },
            Name::new("hud: weapon slot"),
        ),
    );
    child(
        &mut commands,
        slot,
        (
            Node {
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(r(35.0)),
                ..at(20.0, 11.0, 156.0, 70.0)
            },
            slate(0.62),
            rim(),
        ),
    );
    child(
        &mut commands,
        slot,
        (
            Node {
                border_radius: BorderRadius::MAX,
                ..at(
                    SLOT_BALL.0 - BALL_D / 2.0,
                    SLOT_BALL.1 - BALL_D / 2.0,
                    BALL_D,
                    BALL_D,
                )
            },
            atlas(&art.wea_bg, 73.5, 10.5, 42.0, 42.0),
        ),
    );
    let ring_d = BALL_D - 4.0;
    child(
        &mut commands,
        slot,
        (
            at(
                SLOT_BALL.0 - ring_d / 2.0,
                SLOT_BALL.1 - ring_d / 2.0,
                ring_d,
                ring_d,
            ),
            ImageNode::new(ring_image.clone()),
        ),
    );
    let slot_icon = child(
        &mut commands,
        slot,
        (
            at(26.0, 10.0, 100.0, 50.0),
            atlas(&art.weapons, 0.0, 0.0, 64.0, 32.0),
        ),
    );
    let slot_number = child(
        &mut commands,
        slot,
        (
            Node {
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..at(SLOT_BALL.0 - 32.0, SLOT_BALL.1 - 20.0, 64.0, 40.0)
            },
            Name::new("hud: ammo"),
        ),
    );
    let slot_number_text = child(&mut commands, slot_number, bts(HudFont::Digits, 1.25));
    let mut slot_stars = Vec::new();
    for i in 0..6 {
        let e = child(
            &mut commands,
            slot,
            (
                at(22.0 + 19.5 * i as f32, 58.0, 19.0, 19.0),
                ImageNode {
                    image: art.star.clone(),
                    color: Color::srgba(1.0, 1.0, 1.0, 0.5),
                    ..default()
                },
            ),
        );
        slot_stars.push(e);
    }
    let slot_reserve = child(
        &mut commands,
        slot,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(34.0),
                top: r(-6.0),
                ..default()
            },
            bts(HudFont::Light, 0.85).with_colour(Color::srgba(0.85, 0.9, 0.95, 0.75)),
        ),
    );
    let name_pill = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                right: r(28.0),
                bottom: r(17.0),
                height: r(31.0),
                min_width: r(118.0),
                justify_content: JustifyContent::End,
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(r(16.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(r(16.0)),
                ..default()
            },
            slate(0.66),
            rim(),
            Name::new("hud: weapon name"),
        ),
    );
    let name_text = child(&mut commands, name_pill, bts(HudFont::Bold, 1.1));

    // ----- the grenade slot (bottom left): the mirror image -----
    let grenade = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(slot_right + 1.0),
                top: r(SLOT_TOP),
                width: r(SLOT_W),
                height: r(SLOT_H),
                ..default()
            },
            Name::new("hud: grenade slot"),
        ),
    );
    let ball_x = SLOT_W - SLOT_BALL.0;
    child(
        &mut commands,
        grenade,
        (
            Node {
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(r(35.0)),
                ..at(22.0, 11.0, 156.0, 70.0)
            },
            slate(0.62),
            rim(),
        ),
    );
    child(
        &mut commands,
        grenade,
        (
            Node {
                border_radius: BorderRadius::MAX,
                ..at(
                    ball_x - BALL_D / 2.0,
                    SLOT_BALL.1 - BALL_D / 2.0,
                    BALL_D,
                    BALL_D,
                )
            },
            ImageNode {
                image: art.wea_bg.clone(),
                rect: Some(Rect::new(73.5, 10.5, 115.5, 52.5)),
                flip_x: true,
                ..default()
            },
        ),
    );
    let grenade_icon = child(
        &mut commands,
        grenade,
        (
            at(ball_x + 62.0, 14.0, 42.0, 44.0),
            ImageNode {
                image: art.weapons.clone(),
                rect: Some(GRENADE_RECT),
                ..default()
            },
        ),
    );
    let grenade_count_box = child(
        &mut commands,
        grenade,
        (
            Node {
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..at(ball_x - 32.0, SLOT_BALL.1 - 20.0, 64.0, 40.0)
            },
            Name::new("hud: grenade count"),
        ),
    );
    let grenade_count = child(&mut commands, grenade_count_box, bts(HudFont::Digits, 1.2));
    for i in 0..6 {
        child(
            &mut commands,
            grenade,
            (
                at(ball_x + 40.0 + 19.5 * i as f32, 58.0, 19.0, 19.0),
                ImageNode {
                    image: art.star.clone(),
                    color: Color::srgba(1.0, 1.0, 1.0, if i == 0 { 1.0 } else { 0.5 }),
                    ..default()
                },
            ),
        );
    }
    let grenade_pill = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(30.0),
                bottom: r(17.0),
                height: r(31.0),
                min_width: r(200.0),
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(r(16.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(r(16.0)),
                ..default()
            },
            slate(0.66),
            rim(),
            Name::new("hud: grenade name"),
        ),
    );
    let grenade_text = child(&mut commands, grenade_pill, bts(HudFont::Bold, 1.0));

    // ----- key prompts: right of the centre -----
    let mut prompts = Vec::new();
    for i in 0..PROMPT_ROWS {
        let row = child(
            &mut commands,
            root,
            (
                Node {
                    position_type: PositionType::Absolute,
                    right: r(48.0),
                    top: r(474.0 + 40.0 * i as f32),
                    height: r(31.0),
                    align_items: AlignItems::Center,
                    padding: UiRect::horizontal(r(16.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(r(16.0)),
                    display: Display::None,
                    ..default()
                },
                pill(0.58),
                Name::new(format!("hud: prompt {i}")),
            ),
        );
        let text = child(&mut commands, row, bts(HudFont::Bold, 1.05));
        prompts.push((row, text));
    }

    // ----- world markers -----
    let mut markers = Vec::new();
    for i in 0..MARKER_ROWS {
        let mroot = child(
            &mut commands,
            root,
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: r(0.0),
                    top: r(0.0),
                    display: Display::None,
                    ..default()
                },
                Name::new(format!("hud: marker {i}")),
            ),
        );
        let ring = child(
            &mut commands,
            mroot,
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: r(-19.0),
                    top: r(-19.0),
                    width: r(38.0),
                    height: r(38.0),
                    ..default()
                },
                atlas(&art.tin, 0.0, 0.0, 40.0, 40.0),
            ),
        );
        let label = child(
            &mut commands,
            mroot,
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: r(22.0),
                    top: r(-15.0),
                    height: r(30.0),
                    align_items: AlignItems::Center,
                    padding: UiRect::horizontal(r(14.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(r(15.0)),
                    ..default()
                },
                pill(0.58),
            ),
        );
        let label_text = child(&mut commands, label, bts(HudFont::Bold, 1.05));
        let chevron = child(
            &mut commands,
            root,
            (
                Node {
                    position_type: PositionType::Absolute,
                    left: r(0.0),
                    top: r(0.0),
                    width: r(40.0),
                    height: r(40.0),
                    margin: UiRect::all(r(-20.0)),
                    display: Display::None,
                    ..default()
                },
                atlas(&art.clamp, 32.0, 0.0, 32.0, 32.0),
                UiTransform::IDENTITY,
                Name::new(format!("hud: chevron {i}")),
            ),
        );
        markers.push(MarkerUi {
            root: mroot,
            ring,
            label_text,
            chevron,
        });
    }

    // ----- the crosshair, small, in the middle -----
    let crosshair = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: r(548.0),
                width: r(0.0),
                height: r(0.0),
                ..default()
            },
            Name::new("hud: crosshair"),
        ),
    );
    child(
        &mut commands,
        crosshair,
        (
            at(-21.0, -15.0, 42.0, 30.0),
            atlas(&art.alpha, 128.0, 48.0, 24.0, 18.0),
        ),
    );
    for (x, y, w, h) in [
        (-46.0, -1.5, 10.0, 3.0),
        (36.0, -1.5, 10.0, 3.0),
        (-1.5, -46.0, 3.0, 10.0),
        (-1.5, 36.0, 3.0, 10.0),
    ] {
        child(
            &mut commands,
            crosshair,
            (
                at(x, y, w, h),
                BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            ),
        );
    }

    // ----- the holstered crosshair: a plain '+' -----
    // The retail screenshots with no gun drawn (`_extract/crosshair/ref/6.webp`, `7.webp`)
    // show four dark-outlined light bars round a dim centre dot. Measured at 1084 px high,
    // each bar is 11 x 5.6 px, i.e. the 6 x 3 dash sprite of `hud_alpha_a_dif` (at
    // (230, 18): white 4 x 1 inside a 1 px dark outline) at the HUD scale, and the dot is the
    // 3 x 3 sprite at (240, 18). The 1 unit gaps are measured from the same pictures.
    let plain = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: r(548.0),
                width: r(0.0),
                height: r(0.0),
                display: Display::None,
                ..default()
            },
            Name::new("hud: crosshair (holstered)"),
        ),
    );
    child(
        &mut commands,
        plain,
        (
            at(-1.5, -1.5, 3.0, 3.0),
            atlas(&art.alpha, 240.0, 18.0, 3.0, 3.0),
        ),
    );
    for (x, y, vertical) in [
        (-8.5, -1.5, false),
        (2.5, -1.5, false),
        (-3.0, -7.0, true),
        (-3.0, 4.0, true),
    ] {
        child(
            &mut commands,
            plain,
            (
                at(x, y, 6.0, 3.0),
                atlas(&art.alpha, 230.0, 18.0, 6.0, 3.0),
                if vertical {
                    UiTransform::from_rotation(Rot2::degrees(90.0))
                } else {
                    UiTransform::IDENTITY
                },
            ),
        );
    }

    // "down" message in the middle
    let down_text = child(
        &mut commands,
        root,
        (
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: r(400.0),
                margin: UiRect::left(r(-300.0)),
                width: r(600.0),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            bts(HudFont::Bold, 1.8)
                .with_text("Rico is down: back at the start in a moment")
                .with_align(TextAlign::Center)
                .with_wrap(600.0),
        ),
    );

    commands.insert_resource(art);
    commands.insert_resource(HudNodes {
        minimap_map: map_node,
        glow,
        arc_image,
        north,
        heat_glass: glass,
        heat_glass_title,
        heat_glass_level,
        triangle,
        icons,
        slot,
        slot_icon,
        slot_number: slot_number_text,
        ring_image,
        slot_stars,
        slot_reserve,
        name_pill,
        name_text,
        grenade,
        grenade_icon,
        grenade_count,
        grenade_pill,
        grenade_text,
        prompts,
        notices,
        markers,
        vignette,
        desat,
        crosshair,
        crosshair_plain: plain,
        heat_big,
        heat_big_text,
        heat_big_level,
        area_banner,
        area_icon,
        area_name,
        area_panel,
        area_panel_icon,
        area_panel_text,
        area_complete,
        objective,
        objective_text,
        down_text,
    });
}

// ---------------------------------------------------------------------------------
// the minimap's picture: the tiles' own map patches
// ---------------------------------------------------------------------------------

/// Tiles across the picture (the one Rico stands on in the middle), the pixels of one
/// tile's patch, the picture's pixels and the metres a pixel covers.
const ATLAS_TILES: i32 = 3;
const TILE_PX: usize = 128;
const ATLAS_PX: usize = ATLAS_TILES as usize * TILE_PX;
const TILE_M: f32 = 512.0;
const ATLAS_M_PER_PX: f32 = TILE_M / TILE_PX as f32;
/// The world's corner (engine x and z of tile 0's corner).
const WORLD_MIN: f32 = -16384.0;

/// A 3 × 3 patchwork of the terrain tiles' 128 × 128 map patches (4 m a pixel) around
/// Rico, read in the background as he moves; until a patch is in, the overview picture
/// (`pda_map_dif.dds`, 16 m a pixel) fills its place.
#[derive(Resource)]
pub struct MinimapAtlas {
    pub image: Handle<Image>,
    base: Option<Vec<u8>>,
    base_task: Option<Task<Option<Vec<u8>>>>,
    patches: HashMap<(i32, i32), Vec<u8>>,
    pending: HashMap<(i32, i32), Task<Option<Vec<u8>>>>,
    failed: HashSet<(i32, i32)>,
    /// The tile in the middle.
    pub centre: (i32, i32),
    dirty: bool,
}

/// The tile (not clamped to the world) an engine position is on.
pub fn tile_of(x: f32, z: f32) -> (i32, i32) {
    (
        ((x - WORLD_MIN) / TILE_M).floor() as i32,
        ((z - WORLD_MIN) / TILE_M).floor() as i32,
    )
}

/// Where the patchwork around tile `centre` starts, in engine x and z.
pub fn atlas_origin(centre: (i32, i32)) -> Vec2 {
    Vec2::new(
        WORLD_MIN + (centre.0 - 1) as f32 * TILE_M,
        WORLD_MIN + (centre.1 - 1) as f32 * TILE_M,
    )
}

/// The part of the patchwork the minimap shows for Rico at engine `(x, z)`.
pub fn atlas_rect(centre: (i32, i32), x: f32, z: f32) -> Rect {
    let o = atlas_origin(centre);
    let c = Vec2::new((x - o.x) / ATLAS_M_PER_PX, (z - o.y) / ATLAS_M_PER_PX);
    Rect::from_center_half_size(c, Vec2::splat(MINIMAP_RADIUS_M / ATLAS_M_PER_PX))
}

/// One RGBA pixel of the overview picture at engine `(x, z)`, bilinear.
fn sample_base(base: &[u8], x: f32, z: f32) -> [f32; 4] {
    let n = MAP_PX as i32;
    let fx = ((x - WORLD_MIN) / MAP_M_PER_PX - 0.5).clamp(0.0, MAP_PX - 1.001);
    let fz = ((z - WORLD_MIN) / MAP_M_PER_PX - 0.5).clamp(0.0, MAP_PX - 1.001);
    let (x0, z0) = (fx.floor() as i32, fz.floor() as i32);
    let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
    let at = |xi: i32, zi: i32, c: usize| {
        base[((zi.clamp(0, n - 1) * n + xi.clamp(0, n - 1)) as usize) * 4 + c] as f32
    };
    let mut out = [0.0; 4];
    for (c, o) in out.iter_mut().enumerate() {
        let top = at(x0, z0, c) * (1.0 - tx) + at(x0 + 1, z0, c) * tx;
        let bottom = at(x0, z0 + 1, c) * (1.0 - tx) + at(x0 + 1, z0 + 1, c) * tx;
        *o = top * (1.0 - tz) + bottom * tz;
    }
    out
}

/// The map's colours with the mid-tones pulled down: the original's minimap is contrasty.
fn tone(v: u8) -> u8 {
    ((v as f32 / 255.0).powf(1.35) * 255.0) as u8
}

/// The patchwork's pixels around tile `centre`.
fn build_atlas(
    centre: (i32, i32),
    base: Option<&[u8]>,
    patches: &HashMap<(i32, i32), Vec<u8>>,
) -> Vec<u8> {
    let mut out = vec![0u8; ATLAS_PX * ATLAS_PX * 4];
    let o = atlas_origin(centre);
    for j in 0..ATLAS_PX {
        for i in 0..ATLAS_PX {
            let tile = (
                centre.0 - 1 + (i / TILE_PX) as i32,
                centre.1 - 1 + (j / TILE_PX) as i32,
            );
            let px: [u8; 4] = if let Some(p) = patches.get(&tile) {
                let k = ((j % TILE_PX) * TILE_PX + i % TILE_PX) * 4;
                [tone(p[k]), tone(p[k + 1]), tone(p[k + 2]), 255]
            } else if let Some(b) = base {
                let c = sample_base(
                    b,
                    o.x + (i as f32 + 0.5) * ATLAS_M_PER_PX,
                    o.y + (j as f32 + 0.5) * ATLAS_M_PER_PX,
                );
                [tone(c[0] as u8), tone(c[1] as u8), tone(c[2] as u8), 255]
            } else {
                [12, 26, 38, 255]
            };
            out[(j * ATLAS_PX + i) * 4..][..4].copy_from_slice(&px);
        }
    }
    out
}

fn setup_atlas(mut commands: Commands, mut images: ResMut<Assets<Image>>, game: Option<Res<Game>>) {
    let image = images.add(rgba_image(
        ATLAS_PX as u32,
        ATLAS_PX as u32,
        build_atlas((0, 0), None, &HashMap::new()),
    ));
    // the overview picture, decoded once off the main thread
    let base_task = game.map(|g| {
        let fs = g.fs.clone();
        AsyncComputeTaskPool::get().spawn(async move {
            let data = fs.read_inflated(MAP_TEXTURE).ok()?;
            jc2_formats::dds::DdsImage::parse(&data)
                .ok()?
                .decode_rgba(0)
                .filter(|p| p.len() == (MAP_PX as usize).pow(2) * 4)
        })
    });
    commands.insert_resource(MinimapAtlas {
        image,
        base: None,
        base_task,
        patches: HashMap::new(),
        pending: HashMap::new(),
        failed: HashSet::new(),
        centre: (i32::MIN, i32::MIN),
        dirty: false,
    });
}

/// Follow Rico: ask for the patches around him, take in what has been read, redraw the
/// patchwork when something changed.
fn update_atlas(
    mut atlas: ResMut<MinimapAtlas>,
    mut images: ResMut<Assets<Image>>,
    game: Option<Res<Game>>,
    heroes: Query<&Transform, With<Hero>>,
    cams: Query<&Transform, (With<FlyCamera>, Without<Hero>)>,
) {
    let atlas = &mut *atlas;
    if let Some(t) = atlas.base_task.as_mut()
        && let Some(done) = future::block_on(future::poll_once(t))
    {
        atlas.base = done;
        atlas.base_task = None;
        atlas.dirty = true;
    }
    let mut arrived = Vec::new();
    for (tile, t) in atlas.pending.iter_mut() {
        if let Some(done) = future::block_on(future::poll_once(t)) {
            arrived.push((*tile, done));
        }
    }
    for (tile, done) in arrived {
        atlas.pending.remove(&tile);
        match done {
            Some(p) => {
                atlas.patches.insert(tile, p);
                atlas.dirty = true;
            }
            None => {
                atlas.failed.insert(tile);
            }
        }
    }
    let Some(pos) = heroes
        .single()
        .map(|t| t.translation)
        .or_else(|_| cams.single().map(|t| t.translation))
        .ok()
    else {
        return;
    };
    let centre = tile_of(pos.x, pos.z);
    if centre != atlas.centre {
        atlas.centre = centre;
        atlas.dirty = true;
        // only the neighbourhood is kept
        atlas
            .patches
            .retain(|t, _| (t.0 - centre.0).abs() <= 2 && (t.1 - centre.1).abs() <= 2);
    }
    if let Some(g) = game {
        for dz in -1..=1 {
            for dx in -1..=1 {
                let tile = (centre.0 + dx, centre.1 + dz);
                if !(0..64).contains(&tile.0)
                    || !(0..64).contains(&tile.1)
                    || atlas.patches.contains_key(&tile)
                    || atlas.pending.contains_key(&tile)
                    || atlas.failed.contains(&tile)
                {
                    continue;
                }
                let fs = g.fs.clone();
                let (tx, tz) = (tile.0 as u32, tile.1 as u32);
                let task = AsyncComputeTaskPool::get().spawn(async move {
                    let data = fs
                        .read_inflated(&jc2_formats::terrain::tile_file_name(tx, tz))
                        .ok()?;
                    let tile = jc2_formats::terrain::TerrainTile::parse(&data).ok()?;
                    tile.map?
                        .decode_rgba(0)
                        .filter(|p| p.len() == TILE_PX * TILE_PX * 4)
                });
                atlas.pending.insert(tile, task);
            }
        }
    }
    if atlas.dirty {
        atlas.dirty = false;
        let data = build_atlas(centre, atlas.base.as_deref(), &atlas.patches);
        if let Some(mut img) = images.get_mut(&atlas.image) {
            img.data = Some(data);
        }
    }
}

// ---------------------------------------------------------------------------------
// reading the game
// ---------------------------------------------------------------------------------

/// A vehicle's kind for the HUD.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Ride {
    Car,
    Heli,
    Plane,
    Boat,
}

type VehicleItem<'a> = (
    Entity,
    &'a Transform,
    &'a Enterable,
    Option<&'a Container>,
    Option<&'a StuntSpots>,
    Has<Heli>,
    Has<Plane>,
    Has<Watercraft>,
    Has<Occupied>,
    Option<&'a avian3d::prelude::LinearVelocity>,
);

fn ride_of(heli: bool, plane: bool, boat: bool) -> Ride {
    if heli {
        Ride::Heli
    } else if plane {
        Ride::Plane
    } else if boat {
        Ride::Boat
    } else {
        Ride::Car
    }
}

/// Rico, his guns, the heat, the vehicle he is in → [`HudView`].
#[allow(clippy::too_many_arguments)]
fn collect(
    demo: Option<Res<HudDemo>>,
    time: Res<Time>,
    mode: Res<CameraMode>,
    strings: Res<HudStrings>,
    arsenal: Option<Res<Arsenal>>,
    heat: Option<Res<Heat>>,
    mut view: ResMut<HudView>,
    mut flash: ResMut<DamageFlash>,
    heroes: Query<(&Hero, &Transform, Option<&Health>)>,
    vehicles: Query<VehicleItem>,
) {
    let dt = time.delta_secs();
    let Ok((hero, _, health)) = heroes.single() else {
        view.active = false;
        return;
    };
    view.active = true;
    view.in_vehicle = hero.in_vehicle.is_some();
    view.on_foot = !view.in_vehicle && *mode == CameraMode::Hero;
    view.drawn = arsenal.as_ref().is_none_or(|a| a.drawn);
    view.heat_level = heat.as_ref().map_or(0, |h| h.level());
    let (hp, max) = health.map_or((1.0, 1.0), |h| (h.hp, h.max));
    view.health_frac = (hp / max.max(1.0)).clamp(0.0, 1.0);
    view.down = hp <= 0.0;
    flash.update(hp, max, dt);
    view.lost_frac = flash.lost;

    // the slot: the gun in hand, or the vehicle he is in
    let mut slot = SlotView::default();
    if let Some(v) = hero.in_vehicle.and_then(|e| vehicles.get(e).ok()) {
        let container = v.3.map(|c| c.0.as_str()).unwrap_or("");
        let kmh = v.9.map_or(0.0, |l| l.0.length() * 3.6);
        slot = SlotView {
            shown: true,
            cell: None,
            number: format!("{kmh:.0}"),
            fraction: 1.0,
            name: if container.is_empty() {
                "Vehicle".to_string()
            } else {
                strings.vehicle_name(container)
            },
            stars_lit: 0,
            reloading: false,
            reserve: "km/h".to_string(),
        };
    } else if let Some(a) = arsenal.as_ref()
        && view.on_foot
    {
        let g = a.gun();
        let ammo: &crate::ammo::WeaponState = &g.ammo;
        let art = gun_art(&g.weapon.name);
        slot = SlotView {
            shown: true,
            cell: Some(art.map_or(GENERIC_GUN_CELL, |g| g.cell)),
            number: if ammo.unlimited {
                "∞".to_string()
            } else {
                ammo.magazine.to_string()
            },
            fraction: ammo.magazine as f32 / ammo.magazine_size.max(1) as f32,
            name: match art {
                Some(g) => strings.get(g.key, g.english),
                None => g.weapon.label().to_string(),
            },
            // one star for the gun, one more per upgrade level
            stars_lit: g.level as u32 + 1,
            reloading: ammo.reloading.is_some(),
            reserve: if ammo.unlimited {
                String::new()
            } else {
                ammo.reserve.to_string()
            },
        };
    }
    view.slot = slot;
    view.grenade = match arsenal.as_ref() {
        Some(a) if view.on_foot => GrenadeView {
            shown: true,
            count: a.grenades,
            name: strings.get("gui_txt_69", "Fragmentation Grenade"),
        },
        _ => GrenadeView::default(),
    };
    if demo.is_some_and(|d| d.vehicle) {
        view.in_vehicle = true;
        view.on_foot = false;
        view.grenade = GrenadeView::default();
        view.slot = SlotView {
            shown: true,
            number: "62".to_string(),
            fraction: 1.0,
            name: strings.vehicle_name("lave.v014_jeep.ee"),
            reserve: "km/h".to_string(),
            ..default()
        };
    }
}

/// Kills, blasts, vehicles entered and places found → notifications, banners.
#[allow(clippy::too_many_arguments)]
fn track_events(
    time: Res<Time>,
    strings: Res<HudStrings>,
    arsenal: Option<Res<Arsenal>>,
    hits: Option<Res<StructureHits>>,
    heat: Option<Res<Heat>>,
    hijacks: Option<Res<crate::crew::Hijackings>>,
    map: Option<Res<MapData>>,
    mut stats: ResMut<HudStats>,
    mut notices: ResMut<Notices>,
    mut banner: ResMut<HeatBanner>,
    mut area: ResMut<AreaBanner>,
    mut in_area: ResMut<InArea>,
    wrecks: Query<(), Added<Wreck>>,
    heroes: Query<(&Hero, &Transform)>,
    containers: Query<&Container>,
) {
    let dt = time.delta_secs();
    stats.clock += dt;
    notices.tick(dt);
    banner.update(heat.as_ref().map_or(0, |h| h.level()), dt);
    if area.active {
        area.age += dt;
        if area.age >= AREA_BANNER_LIFE {
            area.active = false;
        }
    }
    let now = stats.clock;
    stats.kill_times.retain(|t| now - *t < WINDOW);
    stats.destroyed_times.retain(|t| now - *t < WINDOW);

    // kills
    if let Some(a) = &arsenal {
        if a.kills > stats.seen_kills {
            let before = count_recent(&stats.kill_times, now);
            for _ in stats.seen_kills..a.kills {
                stats.kill_times.push(now);
            }
            let after = count_recent(&stats.kill_times, now);
            if milestone_in(before, after).is_some() {
                notices.push(strings.counted(
                    "statistic_hud_txt_46",
                    "%count kills in the last 60 seconds",
                    after,
                ));
            }
        }
        stats.seen_kills = a.kills;
    }

    // objects destroyed: units (barrels, towers, props) and vehicles burnt out
    let units = hits.as_ref().map_or(0, |h| h.destroyed);
    let mut fresh = units.saturating_sub(stats.seen_destroyed);
    stats.seen_destroyed = units.max(stats.seen_destroyed);
    fresh += wrecks.iter().count() as u32;
    if fresh > 0 {
        let before = count_recent(&stats.destroyed_times, now);
        for _ in 0..fresh {
            stats.destroyed_times.push(now);
        }
        let after = count_recent(&stats.destroyed_times, now);
        if milestone_in(before, after).is_some() {
            notices.push(strings.counted(
                "statistic_hud_txt_47",
                "%count objects destroyed in the last 60 seconds",
                after,
            ));
        }
    }

    // hijackings (`crew.rs`): every one is announced, as the original's "Hijackings: N"
    if let Some(h) = &hijacks {
        if h.count > stats.seen_hijacks {
            notices.push(strings.counted("statistic_hud_txt_14", "Hijackings: %count", h.count));
        }
        stats.seen_hijacks = h.count;
    }

    // unique vehicles driven, places discovered
    let Ok((hero, tf)) = heroes.single() else {
        return;
    };
    if hero.in_vehicle != stats.last_vehicle {
        stats.last_vehicle = hero.in_vehicle;
        if let Some(v) = hero.in_vehicle {
            let id = containers
                .get(v)
                .map(|c| c.0.clone())
                .unwrap_or_else(|_| format!("{v:?}"));
            if stats.vehicles_driven.insert(id) {
                let n = stats.vehicles_driven.len() as u32;
                notices.push(strings.counted(
                    "statistic_hud_txt_13",
                    "Unique vehicles driven: %count",
                    n,
                ));
            }
        }
    }
    let pos = Vec3::new(tf.translation.x, 0.0, tf.translation.z);
    let found = map.as_ref().and_then(|m| {
        m.settlements
            .iter()
            .filter_map(|s| {
                let d = Vec3::new(s.position[0], 0.0, s.position[2]).distance(pos);
                (d < area_radius(s.kind)).then_some((d, s))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, s)| (s.id.clone(), m.name_of(s).to_string(), s.texture.clone()))
    });
    match found {
        Some((id, name, icon)) => {
            in_area.0 = Some((name.clone(), icon.clone(), id.clone()));
            if stats.area.as_deref() != Some(&id) {
                stats.area = Some(id.clone());
                *area = AreaBanner {
                    name,
                    icon,
                    id: id.clone(),
                    age: 0.0,
                    active: true,
                };
                if stats.locations.insert(id) {
                    let n = stats.locations.len() as u32;
                    notices.push(strings.counted(
                        "statistic_hud_txt_17",
                        "Locations discovered: %count / 368",
                        n,
                    ));
                }
            }
        }
        None => {
            in_area.0 = None;
            stats.area = None;
        }
    }
}

/// The minimap: Rico's pixel, the camera's heading, and what to draw on it.
#[allow(clippy::too_many_arguments)]
fn compute_minimap(
    mut view: ResMut<MinimapView>,
    map: Option<Res<MapData>>,
    atlas: Option<Res<MinimapAtlas>>,
    heroes: Query<(&Hero, &Transform)>,
    cams: Query<&Transform, With<FlyCamera>>,
    vehicles: Query<VehicleItem>,
    foes: Query<(&Npc, &Transform), Without<Dead>>,
    extra: Option<Res<ExtraBlips>>,
) {
    let Ok(cam) = cams.single() else { return };
    let player = heroes
        .single()
        .map(|(h, t)| {
            h.in_vehicle
                .and_then(|v| vehicles.get(v).ok())
                .map_or(t.translation, |v| v.1.translation)
        })
        .unwrap_or(cam.translation);
    let mut v = minimap_for(player, cam.forward().as_vec3());
    v.atlas_rect = atlas
        .filter(|a| a.centre.0 != i32::MIN)
        .map(|a| atlas_rect(a.centre, player.x, player.z));
    let mut blips: Vec<(f32, Blip)> = Vec::new();
    let mut push = |pos: Vec3, icon: &str, tint: Color, keep_off: bool| {
        let rel = Vec2::new(pos.x - player.x, pos.z - player.z);
        let off = minimap_offset(rel, v.rotation, MINIMAP_RADIUS_M);
        let len = off.length();
        if len <= 0.97 {
            blips.push((
                len,
                Blip {
                    offset: off,
                    icon: icon.to_string(),
                    tint,
                },
            ));
        } else if keep_off {
            blips.push((
                len,
                Blip {
                    offset: off / len * 0.97,
                    icon: icon.to_string(),
                    tint,
                },
            ));
        }
    };
    if let Some(m) = &map {
        for s in &m.settlements {
            if let Some(tex) = &s.texture {
                push(
                    Vec3::new(s.position[0], 0.0, s.position[2]),
                    tex,
                    Color::WHITE,
                    false,
                );
            }
        }
    }
    for (e, tf, _, _, _, heli, plane, boat, occupied, _) in &vehicles {
        if occupied || heroes.single().is_ok_and(|(h, _)| h.in_vehicle == Some(e)) {
            continue;
        }
        let icon = match ride_of(heli, plane, boat) {
            Ride::Heli | Ride::Plane => "hud_icon_heli_yellow_dif.dds",
            Ride::Boat => "hud_icon_boat_yellow_dif.dds",
            Ride::Car => "hud_icon_car_yellow_dif.dds",
        };
        push(tf.translation, icon, Color::WHITE, false);
    }
    for (n, tf) in &foes {
        if n.alert {
            push(
                tf.translation,
                "hud_icon_foot_red_dif.dds",
                Color::WHITE,
                true,
            );
        }
    }
    if let Some(x) = &extra {
        for (p, icon) in &x.0 {
            push(*p, icon, Color::WHITE, true);
        }
    }
    blips.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.blips = blips.into_iter().take(MM_ICONS).map(|b| b.1).collect();
    *view = v;
}

/// The prompts that apply now.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn compute_prompts(
    demo: Option<Res<HudDemo>>,
    mode: Res<CameraMode>,
    strings: Res<HudStrings>,
    mut list: ResMut<PromptList>,
    heroes: Query<(
        &Hero,
        &Transform,
        Option<&crate::stunts::OnStunt>,
        Option<&crate::stunts::CurrentStuntSpot>,
        Has<crate::stunts::StuntTransfer>,
        Has<crate::ragdoll::PendingRagdoll>,
        Has<crate::ragdoll::PhysicsPose>,
        Option<&crate::foot_machine::FootDriver>,
    )>,
    vehicles: Query<VehicleItem>,
    pickups: Query<&GlobalTransform, With<Pickup>>,
    allowed: Option<Res<ParachuteAllowed>>,
) {
    let Ok((hero, tf, on, current, transferring, pending, physics_pose, foot)) = heroes.single()
    else {
        list.0.clear();
        return;
    };
    let mut c = PromptContext {
        downed: hero.downed || pending || physics_pose,
        transferring,
        free_camera: *mode == CameraMode::Free,
        in_vehicle: hero.in_vehicle.is_some(),
        on_roof: hero.stunt.is_some(),
        on_wire: hero.grapple.is_some(),
        hanging: hero.hanging && hero.grapple.is_some(),
        parachute: hero.parachute,
        canopy_open: hero.parachute
            && !foot.is_some_and(|f| {
                matches!(
                    f.state.as_str(),
                    "S_UNFOLD_PARACHUTE_HORIZONTAL" | "S_UNFOLD_PARACHUTE_VERTICAL"
                )
            }),
        airborne: !hero.grounded
            && !hero.swimming
            && hero.stunt.is_none()
            && hero.in_vehicle.is_none()
            && hero.in_vehicle.is_none(),
        skydive: foot.is_some_and(|f| is_skydive_controller_state(&f.state))
            && allowed.is_none_or(|a| a.0)
            && !hero.parachute,
        ..default()
    };
    if !c.downed
        && !c.transferring
        && let (Some(on), Some(current)) = (on, current)
        && let Ok(v) = vehicles.get(on.vehicle)
        && let Some(spots) = v.4
    {
        (c.transfer_target, c.transfer_other) = crate::stunts::transfer_targets(on, current, spots);
    }
    if let Some(v) = hero.in_vehicle.and_then(|e| vehicles.get(e).ok()) {
        c.vehicle_is_car = ride_of(v.5, v.6, v.7) == Ride::Car;
    }
    if !c.in_vehicle && !c.on_roof {
        let here = tf.translation;
        c.near_vehicle = vehicles
            .iter()
            .any(|v| !v.8 && v.1.translation.distance(here) - v.2.reach < ENTER_RANGE);
        c.near_stunt = vehicles.iter().any(|v| {
            v.4.is_some_and(|spots| {
                nearest_spot(
                    here,
                    v.1.translation,
                    v.1.rotation,
                    spots,
                    MOUNT_REACH,
                    true,
                )
                .is_some()
            })
        });
        c.near_pickup = pickups.iter().any(|p| {
            let d = p.translation() - here;
            Vec2::new(d.x, d.z).length() < PICKUP_NEAR && d.y.abs() < 4.0
        });
    }
    if let Some(d) = &demo {
        if d.vehicle {
            c = PromptContext {
                in_vehicle: true,
                vehicle_is_car: true,
                ..default()
            };
        } else if prompts_for(&c, &strings).is_empty() {
            c.airborne = true;
            c.skydive = true;
        }
    }
    let next = prompts_for(&c, &strings);
    if list.0 != next {
        list.0 = next;
    }
}

/// The world markers: the nearest vehicle to climb into, soldiers who shoot at Rico.
#[allow(clippy::too_many_arguments)]
fn compute_markers(
    demo: Option<Res<HudDemo>>,
    cams: Query<&Transform, With<FlyCamera>>,
    strings: Res<HudStrings>,
    mut out: ResMut<Markers>,
    heroes: Query<(&Hero, &Transform)>,
    vehicles: Query<VehicleItem>,
    foes: Query<(&Npc, &Transform), Without<Dead>>,
    extra: Option<Res<ExtraMarkers>>,
) {
    out.0.clear();
    if let Some(x) = &extra {
        out.0.extend(x.0.iter().cloned());
    }
    let Ok((hero, tf)) = heroes.single() else {
        return;
    };
    let here = tf.translation;
    if hero.in_vehicle.is_none() {
        let nearest = vehicles
            .iter()
            .filter(|v| !v.8)
            .map(|v| (v, v.1.translation.distance(here)))
            .filter(|(_, d)| *d < VEHICLE_MARKER_RANGE)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((v, d)) = nearest {
            let name =
                v.3.map(|c| strings.vehicle_name(&c.0))
                    .unwrap_or_else(|| "Vehicle".to_string());
            out.0.push(MarkerView {
                world: v.1.translation + Vec3::Y * 1.6,
                label: format!("{name} {d:.0} m"),
                foe: false,
            });
        }
    }
    if let (Some(_), Ok(cam)) = (&demo, cams.single()) {
        // two soldiers: one ahead and to the left, one behind
        for (offset, d) in [
            (Vec3::new(-8.0, 0.0, -26.0), 27.0),
            (Vec3::new(10.0, 0.0, 34.0), 35.0),
        ] {
            out.0.push(MarkerView {
                world: cam.translation + cam.rotation * offset,
                label: format!("Guard {d:.0} m"),
                foe: true,
            });
        }
    }
    let mut shooters: Vec<(f32, Vec3)> = foes
        .iter()
        .filter(|(n, _)| n.alert)
        .map(|(_, t)| (t.translation.distance(here), t.translation))
        .filter(|(d, _)| *d < FOE_MARKER_RANGE)
        .collect();
    shooters.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, p) in shooters.into_iter().take(3) {
        out.0.push(MarkerView {
            world: p + Vec3::Y * 1.2,
            label: format!("Guard {d:.0} m"),
            foe: true,
        });
    }
}

/// `--hud-demo`: heat 2, a notification, an objective, the area banner, once.
#[allow(clippy::too_many_arguments)]
fn demo(
    demo: Option<Res<HudDemo>>,
    popup: Option<ResMut<ChaosPopup>>,
    tables: Option<Res<ChaosTables>>,
    mut objective: ResMut<Objective>,
    mut notices: ResMut<Notices>,
    mut banner: ResMut<HeatBanner>,
    mut area: ResMut<AreaBanner>,
    mut done: Local<bool>,
    time: Res<Time>,
) {
    if demo.is_none() {
        return;
    }
    if *done {
        // keep them from fading while the screenshot is awaited
        for n in &mut notices.0 {
            n.age = n.age.min(2.0);
        }
        area.age = area.age.min(1.5);
        banner.age = banner.age.min(0.9);
        if let Some(mut p) = popup {
            p.age = p.age.min(1.0);
        }
        return;
    }
    if time.elapsed_secs() < 3.0 {
        return;
    }
    *done = true;
    objective.0 = Some("Retrieve memory card #3".to_string());
    notices.push("Unique vehicles driven: 1");
    notices.push("12 kills in the last 60 seconds");
    notices.push("10 objects destroyed in the last 60 seconds");
    banner.level = 2;
    banner.age = 0.0;
    // the data's gas pump
    if let (Some(mut p), Some(t)) = (popup, tables)
        && let Some(a) = t.activities.get(11)
    {
        p.push(a);
    }
    *area = AreaBanner {
        name: "Kampung Tiang Emas".to_string(),
        icon: Some("hud_icon_s_u_village_dif.dds".to_string()),
        id: String::new(),
        age: 0.0,
        active: true,
    };
}

// ---------------------------------------------------------------------------------
// writing the UI
// ---------------------------------------------------------------------------------

/// What [`apply_hud`] reads.
#[derive(SystemParam)]
struct Inputs<'w, 's> {
    view: Res<'w, HudView>,
    mm: Res<'w, MinimapView>,
    prompts: Res<'w, PromptList>,
    markers: Res<'w, Markers>,
    notices: Res<'w, Notices>,
    banner: Res<'w, HeatBanner>,
    area: Res<'w, AreaBanner>,
    in_area: Res<'w, InArea>,
    flash: Res<'w, DamageFlash>,
    objective: Res<'w, Objective>,
    cams: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<FlyCamera>>,
    atlas: Option<Res<'w, MinimapAtlas>>,
}

/// What it writes: the UI's components, one query each (they are told apart by entity).
#[derive(SystemParam)]
struct UiQueries<'w, 's> {
    node: Query<'w, 's, &'static mut Node>,
    text: Query<'w, 's, &'static mut BitmapText>,
    image: Query<'w, 's, &'static mut ImageNode>,
    bg: Query<'w, 's, &'static mut BackgroundColor>,
    tf: Query<'w, 's, &'static mut UiTransform>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_hud(
    nodes: Option<Res<HudNodes>>,
    art: Option<Res<HudArt>>,
    assets: Option<Res<AssetServer>>,
    game: Option<Res<Game>>,
    input: Inputs,
    ui: UiQueries,
    mut images: ResMut<Assets<Image>>,
    mut quant: Local<(i32, i32, i32, i32)>,
    mut icon_cache: Local<HashMap<String, Handle<Image>>>,
) {
    let Inputs {
        view,
        mm,
        prompts,
        markers,
        notices,
        banner,
        area,
        in_area,
        flash,
        objective,
        cams,
        atlas,
    } = input;
    let UiQueries {
        node: mut node_q,
        text: mut text_q,
        image: mut image_q,
        bg: mut bg_q,
        tf: mut tf_q,
    } = ui;
    let (Some(n), Some(art)) = (nodes, art) else {
        return;
    };
    let set_display = |q: &mut Query<&mut Node>, e: Entity, show: bool| {
        if let Ok(mut node) = q.get_mut(e) {
            let want = if show { Display::Flex } else { Display::None };
            if node.display != want {
                node.display = want;
            }
        }
    };
    let mut icon_handle = |name: &str| -> Handle<Image> {
        icon_cache
            .entry(name.to_string())
            .or_insert_with(|| match (&assets, &game) {
                (Some(a), Some(g)) if g.fs.contains(name) => a.load(format!("jc2://{name}")),
                _ => ImageNode::default().image,
            })
            .clone()
    };

    let live = view.active;

    // ----- minimap -----
    if let Ok(mut img) = image_q.get_mut(n.minimap_map) {
        match (&atlas, mm.atlas_rect) {
            (Some(a), Some(rect)) => {
                if img.image != a.image {
                    img.image = a.image.clone();
                }
                img.rect = Some(rect);
            }
            _ => {
                if img.image != art.map {
                    img.image = art.map.clone();
                }
                img.rect = Some(mm.rect);
            }
        }
    }
    if let Ok(mut t) = tf_q.get_mut(n.minimap_map) {
        t.rotation = Rot2::radians(mm.rotation);
    }
    for (i, e) in n.icons.iter().enumerate() {
        let blip = mm.blips.get(i);
        if let Ok(mut node) = node_q.get_mut(*e) {
            match blip {
                Some(b) if live => {
                    node.display = Display::Flex;
                    node.left = Val::Percent(50.0 + 50.0 * b.offset.x);
                    node.top = Val::Percent(50.0 + 50.0 * b.offset.y);
                }
                _ => node.display = Display::None,
            }
        }
        if let (Some(b), Ok(mut img)) = (blip, image_q.get_mut(*e)) {
            let h = icon_handle(&b.icon);
            if img.image != h {
                img.image = h;
            }
            img.color = b.tint;
        }
    }
    if let Ok(mut node) = node_q.get_mut(n.north) {
        let p = mm.north * (MM_SIZE * 0.5 * 0.9);
        node.left = r(MM_SIZE / 2.0 + p.x);
        node.top = r(MM_SIZE / 2.0 + p.y);
    }
    // the arc, redrawn when the health moves
    let key = (
        (view.health_frac * 200.0).round() as i32,
        (view.lost_frac * 200.0).round() as i32,
        0,
        0,
    );
    if quant.0 != key.0 || quant.1 != key.1 {
        quant.0 = key.0;
        quant.1 = key.1;
        if let Some(mut i) = images.get_mut(&n.arc_image) {
            i.data = Some(arc_pixels(256, view.health_frac, view.lost_frac));
        }
    }
    // heat: the glow, the triangle, the glass
    let glow_a = if view.heat_level > 0 {
        0.30 + 0.5 * banner.glass_alpha()
    } else {
        0.0
    };
    if let Ok(mut img) = image_q.get_mut(n.glow) {
        img.color = Color::srgba(0.8, 0.92, 1.0, glow_a);
    }
    if let Ok(mut img) = image_q.get_mut(n.triangle) {
        let (cell, a) = match view.heat_level {
            0 => (0.0, 0.3),
            1 => (0.0, 1.0),
            2 | 3 => (32.0, 1.0),
            _ => (64.0, 1.0),
        };
        img.rect = Some(Rect::new(cell, 0.0, cell + 32.0, 32.0));
        img.color = Color::srgba(1.0, 1.0, 1.0, a);
    }
    let glass = banner.glass_alpha();
    if let Ok(mut bg) = bg_q.get_mut(n.heat_glass) {
        bg.0 = Color::srgba(0.0, 0.01, 0.02, 0.8 * glass);
    }
    let level_txt = format!("Level {}", banner.level);
    set_text(&mut text_q, n.heat_glass_level, &level_txt);
    set_alpha(&mut text_q, n.heat_glass_level, [0.85, 0.87, 0.9], glass);
    set_alpha(&mut text_q, n.heat_glass_title, [0.85, 0.87, 0.9], glass);
    // ----- the big heat banner -----
    let a = banner.alpha();
    set_alpha(&mut text_q, n.heat_big_text, [1.0, 1.0, 1.0], 0.62 * a);
    set_alpha(&mut text_q, n.heat_big_level, [1.0, 1.0, 1.0], 0.62 * a);
    set_text(&mut text_q, n.heat_big_level, &level_txt);
    if let Ok(mut node) = node_q.get_mut(n.heat_big) {
        node.left = r(-150.0 + 140.0 * banner.slide());
    }

    // ----- area panel and banner -----
    set_display(&mut node_q, n.area_panel, live && in_area.0.is_some());
    if let Some((_, Some(icon), _)) = &in_area.0
        && let Ok(mut img) = image_q.get_mut(n.area_panel_icon)
    {
        let h = icon_handle(icon);
        if img.image != h {
            img.image = h;
        }
    }
    let ba = area.alpha();
    set_display(&mut node_q, n.area_banner, live && ba > 0.0);
    if ba > 0.0 {
        set_text(&mut text_q, n.area_name, &area.name);
        if let (Some(icon), Ok(mut img)) = (&area.icon, image_q.get_mut(n.area_icon)) {
            let h = icon_handle(icon);
            if img.image != h {
                img.image = h;
            }
            img.color = Color::srgba(0.8, 0.8, 0.8, ba);
        }
        if let Ok(mut bg) = bg_q.get_mut(n.area_banner) {
            bg.0 = Color::srgba(0.05, 0.06, 0.08, 0.6 * ba);
        }
    }

    // ----- objective -----
    set_display(&mut node_q, n.objective, live && objective.0.is_some());
    if let Some(o) = &objective.0 {
        set_text(&mut text_q, n.objective_text, o);
    }

    // ----- notifications -----
    for (i, (row, text)) in n.notices.iter().enumerate() {
        match notices.0.get(i) {
            Some(notice) if live => {
                set_display(&mut node_q, *row, true);
                set_text(&mut text_q, *text, &notice.text);
                let a = notice_alpha(notice.age);
                if let Ok(mut bg) = bg_q.get_mut(*row) {
                    bg.0 = Color::srgba(0.1, 0.16, 0.3, 0.7 * a);
                }
                set_alpha(&mut text_q, *text, [1.0, 1.0, 1.0], a);
            }
            _ => set_display(&mut node_q, *row, false),
        }
    }

    // ----- the weapon slot, the name pill and the grenade slot -----
    let s = &view.slot;
    let show_slot = live && s.shown;
    for e in [n.slot, n.name_pill] {
        set_display(&mut node_q, e, show_slot);
    }
    if show_slot {
        set_number(&mut text_q, n.slot_number, &s.number);
        set_text(&mut text_q, n.name_text, &s.name);
        set_text(&mut text_q, n.slot_reserve, &s.reserve);
        if let Ok(mut img) = image_q.get_mut(n.slot_icon) {
            match s.cell {
                Some(cell) => {
                    let (cx, cy) = ((cell % 4) as f32 * 64.0, (cell / 4) as f32 * 32.0);
                    img.rect = Some(Rect::new(cx, cy, cx + 64.0, cy + 32.0));
                    img.image = art.weapons.clone();
                }
                None => img.image = ImageNode::default().image,
            }
        }
        for (i, e) in n.slot_stars.iter().enumerate() {
            if let Ok(mut img) = image_q.get_mut(*e) {
                let lit = (i as u32) < s.stars_lit;
                img.color = Color::srgba(
                    1.0,
                    1.0,
                    1.0,
                    if s.cell.is_none() {
                        0.0
                    } else if lit {
                        0.95
                    } else {
                        0.45
                    },
                );
            }
        }
        let dashes = 12usize;
        let lit = if s.cell.is_none() {
            dashes
        } else {
            ((s.fraction * dashes as f32).ceil() as usize).min(dashes)
        };
        let k = (lit as i32) * 2 + i32::from(s.reloading);
        if quant.2 != k {
            quant.2 = k;
            if let Some(mut i) = images.get_mut(&n.ring_image) {
                // a reload shows the ring empty
                i.data = Some(dash_pixels(96, dashes, if s.reloading { 0 } else { lit }));
            }
        }
        set_alpha(
            &mut text_q,
            n.slot_number,
            [1.0, 1.0, 1.0],
            if s.reloading { 0.5 } else { 1.0 },
        );
    }
    let g = &view.grenade;
    for e in [n.grenade, n.grenade_pill] {
        set_display(&mut node_q, e, live && g.shown);
    }
    if g.shown {
        set_number(&mut text_q, n.grenade_count, &g.count.to_string());
        set_text(&mut text_q, n.grenade_text, &g.name);
        if let Ok(mut img) = image_q.get_mut(n.grenade_icon) {
            img.color = Color::srgba(1.0, 1.0, 1.0, if g.count > 0 { 1.0 } else { 0.4 });
        }
    }

    // ----- prompts -----
    for (i, (row, text)) in n.prompts.iter().enumerate() {
        match prompts.0.get(i) {
            Some(p) if live => {
                set_display(&mut node_q, *row, true);
                set_text(&mut text_q, *text, &format!("{} {}", p.label, p.key));
            }
            _ => set_display(&mut node_q, *row, false),
        }
    }

    // ----- crosshair -----
    let cross = live && view.on_foot && !view.down;
    set_display(&mut node_q, n.crosshair, cross && view.drawn);
    set_display(&mut node_q, n.crosshair_plain, cross && !view.drawn);
    set_display(&mut node_q, n.down_text, live && view.down);

    // ----- damage -----
    let v = flash.vignette(view.health_frac);
    if let Ok(mut img) = image_q.get_mut(n.vignette) {
        let low = view.health_frac < LOW_HEALTH;
        img.color = if low {
            Color::srgba(0.45, 0.0, 0.0, v)
        } else {
            Color::srgba(0.78, 0.03, 0.03, v)
        };
    }
    if let Ok(mut bg) = bg_q.get_mut(n.desat) {
        bg.0 = Color::srgba(0.42, 0.40, 0.40, 0.3 * v);
    }

    // ----- world markers -----
    let cam = cams.single().ok();
    let viewport = cam.and_then(|(c, _)| c.logical_viewport_size());
    for (i, m) in n.markers.iter().enumerate() {
        let mv = markers.0.get(i).filter(|_| live);
        let projected = match (mv, cam, viewport) {
            (Some(mv), Some((camera, gt)), Some(size)) => {
                let local = gt.affine().inverse().transform_point3(mv.world);
                let behind = local.z > 0.0;
                let screen = camera.world_to_viewport(gt, mv.world).ok();
                Some(match screen {
                    Some(p)
                        if !behind
                            && p.x > 20.0
                            && p.y > 20.0
                            && p.x < size.x - 20.0
                            && p.y < size.y - 20.0 =>
                    {
                        (Some(p), Vec2::ZERO, size)
                    }
                    _ => {
                        let dir = if behind {
                            Vec2::new(-local.x, local.y.abs().max(0.3))
                        } else {
                            Vec2::new(local.x, -local.y)
                        };
                        (None, dir, size)
                    }
                })
            }
            _ => None,
        };
        match (mv, projected) {
            (Some(mv), Some((Some(p), _, _))) => {
                set_display(&mut node_q, m.root, true);
                if let Ok(mut node) = node_q.get_mut(m.root) {
                    node.left = Val::Px(p.x);
                    node.top = Val::Px(p.y);
                }
                set_display(&mut node_q, m.chevron, false);
                set_text(&mut text_q, m.label_text, &mv.label);
                if let Ok(mut img) = image_q.get_mut(m.ring) {
                    // the red ring is the badge atlas's first cell, the green one its
                    // third row's fifth
                    img.rect = Some(if mv.foe {
                        Rect::new(0.0, 0.0, 40.0, 40.0)
                    } else {
                        Rect::new(160.0, 80.0, 200.0, 120.0)
                    });
                    img.color = Color::WHITE;
                }
            }
            (Some(mv), Some((None, dir, size))) => {
                set_display(&mut node_q, m.root, false);
                set_display(&mut node_q, m.chevron, true);
                let p = edge_point(dir, size, 50.0);
                if let Ok(mut node) = node_q.get_mut(m.chevron) {
                    node.left = Val::Px(p.x);
                    node.top = Val::Px(p.y);
                }
                if let Ok(mut t) = tf_q.get_mut(m.chevron) {
                    let d = (p - size / 2.0).normalize_or_zero();
                    t.rotation = Rot2::radians(d.y.atan2(d.x));
                }
                if let Ok(mut img) = image_q.get_mut(m.chevron) {
                    img.rect = Some(if mv.foe {
                        Rect::new(32.0, 0.0, 64.0, 32.0)
                    } else {
                        Rect::new(128.0, 0.0, 160.0, 32.0)
                    });
                    img.color = Color::WHITE;
                }
            }
            _ => {
                set_display(&mut node_q, m.root, false);
                set_display(&mut node_q, m.chevron, false);
            }
        }
    }
}

// ---------------------------------------------------------------------------------
// chaos: the CHAOS popup, settlement completion, the next Black Market item
// ---------------------------------------------------------------------------------

/// The popup's box (reference pixels): centred at the bottom, the flaming logo
/// (`hud_ch_out_en_dif`, 256 × 64) over the object's name on a dark pill, the chaos
/// (the scorpion badge, `hud_tin_dif` cell (1, 1)) and the money (`hud_reward_dif`'s "$")
/// to the right, as in `reference/ui/chaos-radar-250-money-1250-popup…`.
const POPUP_BOX: (f32, f32) = (600.0, 220.0);
const POPUP_TOP: f32 = 862.0;
/// Where the numerals start in the box: left of them the scorpion and the dollar sign, glyphs
/// of the numeral face (`jc2_smallfont_14`).
const CHAOS_NUMBERS_X: f32 = 372.0;

#[derive(Resource)]
pub struct ChaosHud {
    pub root: Entity,
    pub logo: Entity,
    pub title_pill: Entity,
    pub title: Entity,
    pub chaos_icon: Entity,
    pub chaos: Entity,
    pub money_icon: Entity,
    pub money: Entity,
    pub bar_row: Entity,
    pub bar_fill: Entity,
}

/// The CHAOS logo: `hud_ch_out_en_dif` is drawn additively in the game (orange flames on
/// black), so its black becomes transparency here (alpha = the brightest channel, the
/// colour divided by it).
fn chaos_logo(game: &Game) -> Option<Image> {
    let data = game.fs.read_inflated("main_menu.blz").ok()?;
    let sarc = jc2_formats::sarc::SmallArchive::parse(&data).ok()?;
    let e = sarc.get("hud_ch_out_en_dif.dds")?;
    let dds = jc2_formats::dds::DdsImage::parse(sarc.data(&data, e).ok()?).ok()?;
    let mut px = dds.decode_rgba(0)?;
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[0].max(p[1]).max(p[2]);
        if a > 0 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255) / a as u32) as u8;
            }
        }
        p[3] = ((a as u32 * p[3] as u32) / 255) as u8;
    }
    Some(rgba_image(dds.width, dds.height, px))
}

fn build_chaos_hud(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    art: Option<Res<HudArt>>,
    assets: Option<Res<AssetServer>>,
    game: Option<Res<Game>>,
) {
    let blank = ImageNode::default().image;
    let tex = |member: &str| -> Handle<Image> {
        match (&assets, &game) {
            (Some(a), Some(g)) if g.fs.contains("main_menu.blz") => {
                a.load(format!("jc2://main_menu.blz/{member}"))
            }
            _ => blank.clone(),
        }
    };
    let logo_img = game
        .as_ref()
        .and_then(|g| chaos_logo(g))
        .map_or_else(|| tex("hud_ch_out_en_dif.dds"), |i| images.add(i));
    let reward = tex("hud_reward_dif.dds");
    let tin = art.map_or(blank.clone(), |a| a.tin.clone());
    // a full-width strip so the box can be centred whatever the window's shape
    let strip = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: r(POPUP_TOP),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            Name::new("hud: chaos popup"),
        ))
        .id();
    let b = child(
        &mut commands,
        strip,
        Node {
            width: r(POPUP_BOX.0),
            height: r(POPUP_BOX.1),
            ..default()
        },
    );
    let logo = child(
        &mut commands,
        b,
        (at(147.0, 8.0, 305.0, 76.0), ImageNode::new(logo_img)),
    );
    let title_pill = child(
        &mut commands,
        b,
        (
            Node {
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(r(16.0)),
                ..at(110.0, 88.0, 255.0, 36.0)
            },
            BackgroundColor(Color::srgba(0.07, 0.09, 0.12, 0.55)),
        ),
    );
    let title = child(&mut commands, title_pill, bt(HudFont::Bold, 1.15));
    let chaos_icon = child(
        &mut commands,
        b,
        (
            Node {
                display: Display::None,
                ..at(372.0, 58.0, 50.0, 50.0)
            },
            atlas(&tin, 40.0, 40.0, 40.0, 40.0),
        ),
    );
    let chaos = child(
        &mut commands,
        b,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(CHAOS_NUMBERS_X),
                top: r(48.0),
                ..default()
            },
            bt(HudFont::Chaos, 1.15),
        ),
    );
    let money_icon = child(
        &mut commands,
        b,
        (
            Node {
                display: Display::None,
                ..at(382.0, 112.0, 31.0, 52.0)
            },
            atlas(&reward, 2.0, 2.0, 62.0, 102.0),
        ),
    );
    let money = child(
        &mut commands,
        b,
        (
            Node {
                position_type: PositionType::Absolute,
                left: r(CHAOS_NUMBERS_X),
                top: r(106.0),
                ..default()
            },
            bt(HudFont::Chaos, 1.15),
        ),
    );
    // "Next Black Market Item": how far the chaos is from the next unlock
    let bar_row = child(
        &mut commands,
        b,
        (
            Node {
                align_items: AlignItems::Center,
                column_gap: r(8.0),
                ..at(40.0, 176.0, 560.0, 30.0)
            },
            BackgroundColor(Color::srgba(0.07, 0.09, 0.12, 0.45)),
        ),
    );
    child(
        &mut commands,
        bar_row,
        (
            Node {
                width: r(28.0),
                height: r(28.0),
                ..default()
            },
            atlas(&tin, 40.0, 40.0, 40.0, 40.0),
        ),
    );
    let bar = child(
        &mut commands,
        bar_row,
        (
            Node {
                width: r(240.0),
                height: r(12.0),
                border: UiRect::all(r(1.0)),
                border_radius: BorderRadius::all(r(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            rim(),
        ),
    );
    let bar_fill = child(
        &mut commands,
        bar,
        (
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                border_radius: BorderRadius::all(r(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.85, 0.6, 0.15)),
        ),
    );
    child(
        &mut commands,
        bar_row,
        bt(HudFont::Light, 1.0).with_text("Next Black Market Item"),
    );
    commands.insert_resource(ChaosHud {
        root: strip,
        logo,
        title_pill,
        title,
        chaos_icon,
        chaos,
        money_icon,
        money,
        bar_row,
        bar_fill,
    });
}

fn set_display(q: &mut Query<&mut Node>, e: Entity, show: bool) {
    if let Ok(mut node) = q.get_mut(e) {
        let want = if show { Display::Flex } else { Display::None };
        if node.display != want {
            node.display = want;
        }
    }
}

fn set_text(q: &mut Query<&mut BitmapText>, e: Entity, s: &str) {
    if let Ok(mut t) = q.get_mut(e)
        && t.text != s
    {
        t.text = s.to_string();
    }
}

fn set_alpha(q: &mut Query<&mut BitmapText>, e: Entity, rgb: [f32; 3], a: f32) {
    if let Ok(mut t) = q.get_mut(e) {
        let want = Color::srgba(rgb[0], rgb[1], rgb[2], a);
        if t.colour != want {
            t.colour = want;
        }
    }
}

/// The big number in a slot's ball: bold digits, or for the unlimited gun's infinity sign
/// the small face, whose `E` is an `∞`.
fn set_number(q: &mut Query<&mut BitmapText>, e: Entity, number: &str) {
    if let Ok(mut t) = q.get_mut(e) {
        let (font, text, k) = if number == "∞" {
            (HudFont::Ammo, "E", 1.9)
        } else {
            (HudFont::Digits, number, 1.25)
        };
        let size = font.native() * k;
        if t.font != font {
            t.font = font;
        }
        if t.text != text {
            t.text = text.to_string();
        }
        if t.size != size {
            t.size = size;
        }
    }
}

/// The crosshair turns green while the hook has a lock on a vehicle, soldier, light prop or
/// ledge (`grapple::LockOn`, as the original's reticle on grappleable things).
fn tint_crosshair(
    lock: Option<Res<crate::grapple::LockOn>>,
    nodes: Option<Res<HudNodes>>,
    kids: Query<&Children>,
    mut images: Query<&mut ImageNode>,
    mut bars: Query<&mut BackgroundColor>,
) {
    let (Some(lock), Some(n)) = (lock, nodes) else {
        return;
    };
    let tint = lock.tint();
    for cross in [n.crosshair, n.crosshair_plain] {
        let Ok(children) = kids.get(cross) else {
            continue;
        };
        for c in children.iter() {
            if let Ok(mut img) = images.get_mut(c) {
                img.color = tint;
            } else if let Ok(mut bg) = bars.get_mut(c) {
                bg.0 = tint.with_alpha(0.85);
            }
        }
    }
}

/// The popup, the settlement's "N % complete" and the next-item bar.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_chaos_hud(
    hud: Option<Res<ChaosHud>>,
    nodes: Option<Res<HudNodes>>,
    view: Res<HudView>,
    strings: Res<HudStrings>,
    popup: Option<Res<ChaosPopup>>,
    chaos: Option<Res<Chaos>>,
    progress: Option<Res<Progress>>,
    market: Option<Res<crate::blackmarket::Market>>,
    in_area: Res<InArea>,
    area: Res<AreaBanner>,
    mut node_q: Query<&mut Node>,
    mut text_q: Query<&mut BitmapText>,
    mut image_q: Query<&mut ImageNode>,
    mut bg_q: Query<&mut BackgroundColor>,
) {
    // settlement completion: the share of its targets and crates done
    if let Some(n) = nodes {
        let pct = |id: &str| progress.as_ref().and_then(|p| p.percent(id)).unwrap_or(0);
        if let Some((_, _, id)) = &in_area.0 {
            set_text(&mut text_q, n.area_panel_text, &format!("{}%", pct(id)));
        }
        if area.active {
            set_text(
                &mut text_q,
                n.area_complete,
                &format!(
                    "{} {}",
                    pct(&area.id),
                    strings.get("gui_txt_134", "% complete")
                ),
            );
        }
    }
    let Some(h) = hud else { return };
    let a = popup.as_ref().map_or(0.0, |p| p.alpha());
    let live = view.active && a > 0.0;
    set_display(&mut node_q, h.root, live);
    if !live {
        return;
    }
    let p = popup.unwrap();
    let title = strings.get(&p.title, &p.title);
    set_text(&mut text_q, h.title, &title);
    set_text(
        &mut text_q,
        h.chaos,
        &format!("{}+{}", jc2_formats::font::SCORPION_CHAR, p.chaos),
    );
    set_text(&mut text_q, h.money, &format!("$+{}", p.money));
    for e in [h.title, h.chaos, h.money] {
        set_alpha(&mut text_q, e, [1.0, 1.0, 1.0], a);
    }
    for e in [h.logo, h.chaos_icon, h.money_icon] {
        if let Ok(mut img) = image_q.get_mut(e) {
            img.color = Color::srgba(1.0, 1.0, 1.0, a);
        }
    }
    if let Ok(mut bg) = bg_q.get_mut(h.title_pill) {
        bg.0 = Color::srgba(0.07, 0.09, 0.12, 0.55 * a);
    }
    // the next unlock
    let total = chaos.as_ref().map_or(0, |c| c.total);
    let next = market.as_ref().and_then(|m| {
        m.data
            .next_unlock(total)
            .map(|n| (m.data.last_unlock(total), n))
    });
    set_display(&mut node_q, h.bar_row, next.is_some());
    if let Some((lo, hi)) = next {
        let f = (total - lo) as f32 / (hi - lo).max(1) as f32;
        if let Ok(mut node) = node_q.get_mut(h.bar_fill) {
            node.width = Val::Percent(100.0 * f.clamp(0.0, 1.0));
        }
        if let Ok(mut bg) = bg_q.get_mut(h.bar_row) {
            bg.0 = Color::srgba(0.07, 0.09, 0.12, 0.45 * a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings() -> HudStrings {
        HudStrings::default()
    }

    fn labels(p: &[Prompt]) -> Vec<String> {
        p.iter().map(|p| format!("{} {}", p.label, p.key)).collect()
    }

    /// The skydive-controller states (native `0x0052a650`); the base jump and the canopy are not.
    #[test]
    fn skydive_controller_states() {
        for s in SKYDIVE_CONTROLLER_STATES {
            assert!(is_skydive_controller_state(s), "{s}");
        }
        for s in [
            "S_FALL",
            "S_JUMP_TO_SKYDIVE",
            "S_PARACHUTE",
            "S_UNFOLD_PARACHUTE_VERTICAL",
            "S_REEL_FLIGHT",
        ] {
            assert!(!is_skydive_controller_state(s), "{s}");
        }
    }

    #[test]
    fn prompts_follow_the_state() {
        let s = strings();
        let none = prompts_for(&PromptContext::default(), &s);
        assert!(none.is_empty(), "{none:?}");
        let near = prompts_for(
            &PromptContext {
                near_vehicle: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&near), ["Enter E"]);
        let falling = prompts_for(
            &PromptContext {
                airborne: true,
                skydive: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&falling), ["Open parachute SPACE"]);
        // airborne without a skydive-controller state (a plain fall): no prompt
        let plain = prompts_for(
            &PromptContext {
                airborne: true,
                ..default()
            },
            &s,
        );
        assert!(plain.is_empty(), "{plain:?}");
        // under the canopy with a wire: Reel in (E)
        let reel = prompts_for(
            &PromptContext {
                airborne: true,
                parachute: true,
                on_wire: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&reel), ["Reel in E", "Drop F"]);
        // open canopy with a wire: the GUI button adds "Close parachute" beside "Reel in"
        let both = PromptContext {
            airborne: true,
            parachute: true,
            canopy_open: true,
            on_wire: true,
            ..default()
        };
        assert_eq!(
            labels(&prompts_for(&both, &s)),
            ["Close parachute SPACE", "Reel in E", "Drop F"]
        );
        // button state table
        assert_eq!(parachute_button(&both), Some(ParachuteButton::Close));
        let unfolding = PromptContext {
            canopy_open: false,
            ..both
        };
        assert_eq!(parachute_button(&unfolding), None);
        let sky = PromptContext {
            skydive: true,
            ..default()
        };
        assert_eq!(parachute_button(&sky), Some(ParachuteButton::Open));
        assert_eq!(parachute_button(&PromptContext::default()), None);
        // no wire: the single face-button post, no duplicate
        let open_no_wire = PromptContext {
            canopy_open: true,
            on_wire: false,
            ..both
        };
        assert_eq!(labels(&prompts_for(&open_no_wire, &s)), ["Close parachute SPACE"]);
        let chute = prompts_for(
            &PromptContext {
                airborne: true,
                parachute: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&chute), ["Close parachute SPACE"]);
        let driving = prompts_for(
            &PromptContext {
                in_vehicle: true,
                vehicle_is_car: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&driving), ["Exit E", "Handbrake SPACE"]);
        let flying = prompts_for(
            &PromptContext {
                in_vehicle: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&flying), ["Exit E"]);
        let wire = prompts_for(
            &PromptContext {
                on_wire: true,
                airborne: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&wire), ["Open parachute SPACE", "Drop F"]);
        let roof = prompts_for(
            &PromptContext {
                on_roof: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&roof), ["Jump off SPACE", "Enter E", "Drop Q"]);
        let mut transfer = PromptContext {
            on_roof: true,
            transfer_target: Some("hood"),
            transfer_other: Some("roof"),
            ..default()
        };
        assert_eq!(
            labels(&prompts_for(&transfer, &s)),
            [
                "Jump off SPACE",
                "Enter E",
                "Drop Q",
                "Transfer to hood Z",
                "Transfer to roof SHIFT+Z"
            ]
        );
        transfer.transferring = true;
        assert_eq!(labels(&prompts_for(&transfer, &s)), labels(&roof)); // cancellation stays available
        transfer.downed = true;
        assert!(prompts_for(&transfer, &s).is_empty());

        // in the air the vehicle at his feet is out of reach
        let hop = prompts_for(
            &PromptContext {
                airborne: true,
                skydive: true,
                near_vehicle: true,
                ..default()
            },
            &s,
        );
        assert_eq!(labels(&hop), ["Open parachute SPACE"]);
        let free = prompts_for(
            &PromptContext {
                free_camera: true,
                near_vehicle: true,
                ..default()
            },
            &s,
        );
        assert!(free.is_empty());
    }

    #[test]
    fn minimap_turns_with_the_camera_and_keeps_the_maps_pixel() {
        let p = Vec3::new(-451.8, 100.0, -8860.4);
        // looking north: the picture is upright, north on top
        let v = minimap_for(p, Vec3::NEG_Z);
        assert!(v.rotation.abs() < 1e-5 && v.heading.abs() < 1e-5);
        assert!((v.north - Vec2::NEG_Y).length() < 1e-5);
        // Rico's pixel is map.rs's, in the 2048² image
        let uv = bevy_to_map(p);
        assert!((v.center_px - uv * MAP_PX).length() < 1e-3);
        assert!((v.center_px - Vec2::new(996.0, 470.0)).length() < 1.0);
        // the rect is centred on him
        assert!((v.rect.center() - v.center_px).length() < 1e-3);
        // looking east: north is on his left, the picture turned a quarter anticlockwise
        let e = minimap_for(p, Vec3::X);
        assert!((e.heading - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((e.rotation + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((e.north - Vec2::NEG_X).length() < 1e-5, "{:?}", e.north);
        // a thing 100 m north of him is straight ahead (up) looking north, to his right
        // looking west… and to his left looking east
        let north = Vec2::new(0.0, -100.0);
        let up = minimap_offset(north, v.rotation, MINIMAP_RADIUS_M);
        assert!(up.x.abs() < 1e-5 && up.y < 0.0);
        let left = minimap_offset(north, e.rotation, MINIMAP_RADIUS_M);
        assert!(left.x < 0.0 && left.y.abs() < 1e-5, "{left:?}");
        let w = minimap_for(p, Vec3::NEG_X);
        let right = minimap_offset(north, w.rotation, MINIMAP_RADIUS_M);
        assert!(right.x > 0.0 && right.y.abs() < 1e-5, "{right:?}");
        // east of him looking north is on the right
        let east = minimap_offset(Vec2::new(100.0, 0.0), v.rotation, MINIMAP_RADIUS_M);
        assert!(east.x > 0.0 && east.y.abs() < 1e-5);
    }

    #[test]
    fn the_atlas_follows_the_tile_rico_is_on() {
        // tile 31,14 has its corner at (-512, -9216)
        assert_eq!(tile_of(-451.8, -8860.4), (31, 14));
        assert_eq!(tile_of(-512.0, -9216.0), (31, 14));
        assert_eq!(tile_of(-512.1, -9216.1), (30, 13));
        assert_eq!(atlas_origin((31, 14)), Vec2::new(-1024.0, -9728.0));
        // Rico in the middle of the middle tile: the picture is centred on the atlas
        let r = atlas_rect((31, 14), -256.0, -8960.0);
        assert!((r.center() - Vec2::splat(ATLAS_PX as f32 / 2.0)).length() < 1e-3);
        assert!((r.size() - Vec2::splat(2.0 * MINIMAP_RADIUS_M / ATLAS_M_PER_PX)).length() < 1e-3);
        // a patch lands where its tile is: the middle tile's pixels fill the middle third
        let mut patches = HashMap::new();
        patches.insert((31, 14), vec![200u8; TILE_PX * TILE_PX * 4]);
        let img = build_atlas((31, 14), None, &patches);
        let at = |x: usize, y: usize| img[(y * ATLAS_PX + x) * 4];
        assert_eq!(at(ATLAS_PX / 2, ATLAS_PX / 2), tone(200));
        assert_ne!(at(10, 10), tone(200));
        // the overview picture fills in where a patch is missing, bilinear
        let mut base = vec![0u8; (MAP_PX as usize).pow(2) * 4];
        for px in base.as_chunks_mut::<4>().0 {
            *px = [10, 20, 30, 255];
        }
        let img = build_atlas((31, 14), Some(&base), &HashMap::new());
        assert_eq!(&img[..4], &[tone(10), tone(20), tone(30), 255]);
        assert!(tone(128) < 128 && tone(255) == 255 && tone(0) == 0);
    }

    #[test]
    fn edge_points_sit_on_the_screen_edge() {
        let size = Vec2::new(1000.0, 500.0);
        let right = edge_point(Vec2::X, size, 50.0);
        assert!((right - Vec2::new(950.0, 250.0)).length() < 1e-3);
        let up = edge_point(Vec2::NEG_Y, size, 50.0);
        assert!((up - Vec2::new(500.0, 50.0)).length() < 1e-3);
        let corner = edge_point(Vec2::new(1.0, 1.0), size, 50.0);
        assert!(corner.x <= 950.0 + 1e-3 && corner.y <= 450.0 + 1e-3);
        assert!(
            (corner.y - 450.0).abs() < 1e-3,
            "the short side limits: {corner:?}"
        );
    }

    #[test]
    fn counts_and_milestones() {
        assert_eq!(count_recent(&[1.0, 45.0, 80.0], 100.0), 2);
        assert_eq!(milestone_in(0, 2), None);
        assert_eq!(milestone_in(2, 3), Some(3));
        assert_eq!(milestone_in(3, 4), None);
        assert_eq!(milestone_in(4, 5), Some(5));
        assert_eq!(milestone_in(5, 12), Some(10));
    }

    #[test]
    fn heat_banner_rises_and_fades() {
        let mut b = HeatBanner::default();
        assert!(!b.active() && b.alpha() == 0.0);
        b.update(0, 0.1);
        assert!(!b.active());
        b.update(1, 0.1);
        assert!(b.active() && b.level == 1);
        b.update(1, 0.5);
        assert!(b.alpha() > 0.9 && b.glass_alpha() > 0.9);
        b.update(1, HEAT_BANNER_LIFE);
        assert!(!b.active() && b.alpha() == 0.0 && b.glass_alpha() == 0.0);
        // a fall of the heat starts nothing; the next rise does
        b.update(0, 0.1);
        assert!(!b.active());
        b.update(2, 0.1);
        assert!(b.active() && b.level == 2);
    }

    #[test]
    fn damage_flash_fades_and_low_health_stays() {
        let mut f = DamageFlash::default();
        f.update(400.0, 400.0, 0.016);
        assert_eq!(f.vignette(1.0), 0.0);
        f.update(300.0, 400.0, 0.016);
        assert!(f.vignette(0.75) > 0.4 && f.lost > 0.2);
        for _ in 0..400 {
            f.update(300.0, 400.0, 0.016);
        }
        assert!(f.vignette(0.75) < 0.01, "gone after a few seconds");
        // low health keeps a heavy vignette
        assert!(f.vignette(0.1) > 0.5);
    }

    #[test]
    fn notices_stack_and_fade() {
        let mut n = Notices::default();
        for i in 0..7 {
            n.push(format!("n{i}"));
        }
        assert_eq!(n.0.len(), NOTICE_ROWS);
        assert_eq!(n.0[0].text, "n2");
        assert!(notice_alpha(0.0) < 0.1);
        assert!((notice_alpha(2.0) - 1.0).abs() < 1e-5);
        assert!(notice_alpha(NOTICE_LIFE - 0.1) < 0.2);
        n.tick(NOTICE_LIFE + 1.0);
        assert!(n.0.is_empty());
    }

    #[test]
    fn generated_art_has_the_right_shape() {
        let n = 128;
        let px = |d: &[u8], x: usize, y: usize| {
            let i = (y * n + x) * 4;
            [d[i], d[i + 1], d[i + 2], d[i + 3]]
        };
        // full health: cyan at the bottom of the ring, nothing at the top or in the middle
        let full = arc_pixels(n, 1.0, 0.0);
        let bottom = px(&full, n / 2, n / 2 + (0.947 * n as f32 / 2.0) as usize);
        assert!(
            bottom[3] > 200 && bottom[1] > 200 && bottom[0] < 80,
            "{bottom:?}"
        );
        assert_eq!(px(&full, n / 2, 3)[3], 0, "no arc over the top");
        assert_eq!(px(&full, n / 2, n / 2)[3], 0);
        // no health: grey there
        let empty = arc_pixels(n, 0.0, 0.0);
        let grey = px(&empty, n / 2, n / 2 + (0.947 * n as f32 / 2.0) as usize);
        assert!(grey[3] > 100 && grey[0] > 150 && grey[2] > 150, "{grey:?}");
        // lost health shows red between
        let lost = arc_pixels(n, 0.0, 1.0);
        let red = px(&lost, n / 2, n / 2 + (0.947 * n as f32 / 2.0) as usize);
        assert!(red[0] > 200 && red[1] < 80, "{red:?}");
        // the dashes: all lit vs none lit differ, the middle stays clear
        let a = dash_pixels(64, 12, 12);
        let b = dash_pixels(64, 12, 0);
        assert_ne!(a, b);
        assert_eq!(a[(32 * 64 + 32) * 4 + 3], 0);
    }

    #[test]
    fn names_come_from_the_text_keys() {
        let s = strings();
        assert_eq!(s.vehicle_name("lave.v014_jeep.ee"), "Jeep");
        assert_eq!(
            vehicle_key("arve.v009_military_helicopter.ee"),
            "v009_military_helicopter"
        );
        assert_eq!(gun_art("Beretta Handgun").unwrap().key, "gui_txt_59");
        assert_eq!(gun_art("Assault").unwrap().cell, 4);
        assert!(gun_art("Flak").is_none());
        assert_eq!(s.counted("x", "%count kills", 12), "12 kills");
    }
}

/// The HUD in a headless app: the systems run on real entities, the UI is read back.
#[cfg(test)]
mod headless {
    use super::*;
    use crate::combat::CombatPlugin;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// A bare app with the HUD plugin; every `update` is a tenth of a second.
    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
                0.1,
            )))
            .init_resource::<ButtonInput<KeyCode>>()
            .add_plugins(HudPlugin);
        app
    }

    fn run(app: &mut App, frames: usize) {
        for _ in 0..frames {
            app.update();
        }
    }

    #[test]
    fn crosshair_tint_preserves_the_brackets_transparent_background() {
        let mut app = app();
        app.init_resource::<crate::grapple::LockOn>();
        run(&mut app, 2);
        let crosshair = app.world().resource::<HudNodes>().crosshair;
        let children = app.world().get::<Children>(crosshair).unwrap();
        let mut images = 0;
        let mut ticks = 0;
        for child in children.iter() {
            let bg = app.world().get::<BackgroundColor>(child).unwrap();
            if app.world().get::<ImageNode>(child).is_some() {
                images += 1;
                assert_eq!(
                    bg.0.alpha(),
                    0.0,
                    "atlas alpha must show the world behind it"
                );
            } else {
                ticks += 1;
                assert_eq!(bg.0, Color::WHITE.with_alpha(0.85));
            }
        }
        assert_eq!((images, ticks), (1, 4));
    }

    fn text_of(app: &App, e: Entity) -> String {
        app.world()
            .get::<BitmapText>(e)
            .map(|t| t.text.clone())
            .unwrap()
    }

    fn rico(app: &mut App, at: Vec3) -> Entity {
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Hero;
        let mut hero = Hero::default();
        hero.grounded = true;
        app.world_mut()
            .spawn((hero, Transform::from_translation(at), Health::new(400.0)))
            .id()
    }

    fn camera(app: &mut App, yaw: f32) -> Entity {
        app.world_mut()
            .spawn((
                Transform::from_rotation(Quat::from_rotation_y(yaw)),
                FlyCamera {
                    yaw,
                    pitch: 0.0,
                    speed: 1.0,
                },
            ))
            .id()
    }

    fn prompts(app: &App) -> Vec<String> {
        app.world()
            .resource::<PromptList>()
            .0
            .iter()
            .map(|p| format!("{} {}", p.label, p.key))
            .collect()
    }

    /// The prompts and the vehicle marker follow what Rico is doing: next to a car, in
    /// the air, under the chute, in the car.
    #[test]
    fn the_right_prompt_shows_for_the_state() {
        let mut app = app();
        let hero = rico(&mut app, Vec3::new(0.0, 100.0, 0.0));
        camera(&mut app, 0.0);
        let car = app
            .world_mut()
            .spawn((
                Transform::from_xyz(3.0, 100.0, 0.0),
                Enterable { reach: 1.0 },
                Container("lave.v014_jeep.ee".into()),
            ))
            .id();
        run(&mut app, 3);
        assert_eq!(prompts(&app), ["Enter E"]);
        let m = &app.world().resource::<Markers>().0;
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].label, "Jeep 3 m");
        assert!(!m[0].foe);
        // far from it: nothing to enter, no prompt
        app.world_mut()
            .get_mut::<Transform>(hero)
            .unwrap()
            .translation
            .x = -50.0;
        run(&mut app, 2);
        assert!(prompts(&app).is_empty(), "{:?}", prompts(&app));
        // falling
        {
            let mut h = app.world_mut().get_mut::<Hero>(hero).unwrap();
            h.grounded = false;
            h.air_time = 1.0;
        }
        // a plain fall (foot state S_FALL): no prompt yet
        app.world_mut()
            .entity_mut(hero)
            .insert(crate::foot_machine::FootDriver::new("S_FALL"));
        run(&mut app, 2);
        assert!(prompts(&app).is_empty(), "{:?}", prompts(&app));
        // the skydive-controller state offers the chute
        app.world_mut()
            .entity_mut(hero)
            .insert(crate::foot_machine::FootDriver::new("S_FALL_TO_SKYDIVE"));
        run(&mut app, 2);
        assert_eq!(prompts(&app), ["Open parachute SPACE"]);
        // the script disabled it
        app.insert_resource(ParachuteAllowed(false));
        run(&mut app, 2);
        assert!(prompts(&app).is_empty(), "{:?}", prompts(&app));
        app.insert_resource(ParachuteAllowed(true));
        run(&mut app, 2);
        // the chute is open
        app.world_mut().get_mut::<Hero>(hero).unwrap().parachute = true;
        run(&mut app, 2);
        assert_eq!(prompts(&app), ["Close parachute SPACE"]);
        // in the car
        {
            let mut h = app.world_mut().get_mut::<Hero>(hero).unwrap();
            h.parachute = false;
            h.in_vehicle = Some(car);
        }
        app.world_mut().entity_mut(car).insert(Occupied);
        run(&mut app, 2);
        assert_eq!(prompts(&app), ["Exit E", "Handbrake SPACE"]);
        // the UI shows them: the first row says it, the second is a pill
        let n = app.world().resource::<HudNodes>();
        let (row, text) = n.prompts[0];
        assert_eq!(text_of(&app, text), "Exit E");
        assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::Flex);
        let (row2, _) = n.prompts[2];
        assert_eq!(
            app.world().get::<Node>(row2).unwrap().display,
            Display::None
        );
        // and the speed slot names the vehicle the player is in ("Jeep" without the text
        // database), with the number in the ball
        let v = app.world().resource::<HudView>();
        assert!(
            v.in_vehicle && v.slot.shown && v.slot.name == "Jeep",
            "{v:?}"
        );
        // the unique vehicles driven so far: this one
        let stats = app.world().resource::<HudStats>();
        assert_eq!(stats.vehicles_driven.len(), 1);
        let notices = &app.world().resource::<Notices>().0;
        assert!(
            notices
                .iter()
                .any(|n| n.text.starts_with("Unique vehicles driven: 1")),
            "{notices:?}"
        );
    }

    /// Rico in the skydive-controller state that offers the parachute prompt.
    fn falling_rico(app: &mut App) {
        let hero = rico(app, Vec3::new(0.0, 100.0, 0.0));
        camera(app, 0.0);
        {
            let mut h = app.world_mut().get_mut::<Hero>(hero).unwrap();
            h.grounded = false;
            h.air_time = 1.0;
        }
        app.world_mut()
            .entity_mut(hero)
            .insert(crate::foot_machine::FootDriver::new("S_FALL_TO_SKYDIVE"));
    }

    /// With the game's fonts a prompt's text is bitmap glyphs: one node per visible
    /// character (and one more for its shadow), the node as wide as the font's advances
    /// say, every glyph an atlas rectangle. Needs JC2_TEST_DIR.
    #[test]
    fn prompts_are_drawn_with_the_game_font() {
        use crate::bitmap_text::{BitmapFonts, Rendered};
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = jc2_formats::GameFs::open(dir) else {
            return;
        };
        if !fs.contains("fonts.blz") {
            return;
        }
        let mut app = app();
        app.insert_resource(Game {
            fs: std::sync::Arc::new(fs),
        });
        falling_rico(&mut app);
        run(&mut app, 4);
        let (row, text) = app.world().resource::<HudNodes>().prompts[0];
        assert_eq!(app.world().get::<Node>(row).unwrap().display, Display::Flex);
        let bt = app.world().get::<BitmapText>(text).unwrap().clone();
        assert_eq!(bt.text, "Open parachute SPACE");
        assert_eq!(bt.font, HudFont::Bold);
        let rendered = app.world().get::<Rendered>(text).unwrap();
        let visible = bt.text.chars().filter(|c| !c.is_whitespace()).count();
        assert_eq!(rendered.drawn(), visible);
        assert_eq!(rendered.fallback_count(), 0);
        for e in rendered.glyph_entities() {
            let img = app.world().get::<ImageNode>(e).unwrap();
            assert!(img.rect.is_some(), "a glyph shows an atlas rectangle");
        }
        let kids = app.world().get::<Children>(text).unwrap();
        assert_eq!(kids.len(), visible * 2, "a glyph and its shadow each");
        // as wide as the font's advances, scaled to the size
        let font = app
            .world()
            .resource::<BitmapFonts>()
            .font(HudFont::Bold)
            .clone();
        let scale = bt.size / font.cap_box().1 as f32;
        let want = font.line_width(&bt.text) as f32 * scale;
        assert!(
            (rendered.size.x - want).abs() < 0.01,
            "{} vs {want}",
            rendered.size.x
        );
    }

    /// Without the fonts (no game data) the default font stands in, one `Text` per
    /// character.
    #[test]
    fn prompts_fall_back_to_the_default_font() {
        use crate::bitmap_text::Rendered;
        let mut app = app();
        falling_rico(&mut app);
        run(&mut app, 4);
        let (_, text) = app.world().resource::<HudNodes>().prompts[0];
        let rendered = app.world().get::<Rendered>(text).unwrap();
        let visible = text_of(&app, text)
            .chars()
            .filter(|c| !c.is_whitespace())
            .count();
        assert_eq!(visible, "Open parachute SPACE".replace(' ', "").len());
        assert_eq!(rendered.fallback_count(), visible);
        assert_eq!(rendered.drawn(), visible);
    }

    /// The heat banner appears when the level rises and fades after a few seconds.
    #[test]
    fn the_heat_banner_appears_and_fades() {
        let mut app = app();
        rico(&mut app, Vec3::ZERO);
        camera(&mut app, 0.0);
        app.init_resource::<Heat>();
        run(&mut app, 3);
        assert!(!app.world().resource::<HeatBanner>().active());
        app.world_mut().resource_mut::<Heat>().value = 25.0;
        run(&mut app, 6);
        let b = app.world().resource::<HeatBanner>();
        assert!(b.active() && b.level == 2, "{b:?}");
        let n = app.world().resource::<HudNodes>();
        assert_eq!(text_of(&app, n.heat_big_level), "Level 2");
        let alpha = |app: &App, e: Entity| app.world().get::<BitmapText>(e).unwrap().colour.alpha();
        let big = app.world().resource::<HudNodes>().heat_big_text;
        assert!(alpha(&app, big) > 0.3, "{}", alpha(&app, big));
        let glass = app.world().resource::<HudNodes>().heat_glass_level;
        assert!(alpha(&app, glass) > 0.5);
        // a level that falls announces nothing; time passes and the banner is gone
        run(&mut app, 70);
        let b = app.world().resource::<HeatBanner>();
        assert!(!b.active());
        assert_eq!(alpha(&app, big), 0.0);
        assert_eq!(alpha(&app, glass), 0.0);
        // a rise to level 3 announces again
        app.world_mut().resource_mut::<Heat>().value = 45.0;
        run(&mut app, 3);
        assert_eq!(
            text_of(&app, app.world().resource::<HudNodes>().heat_big_level),
            "Level 3"
        );
        assert!(app.world().resource::<HeatBanner>().active());
    }

    /// The minimap turns with the camera, and Rico's pixel is `map.rs`'s.
    #[test]
    fn the_minimap_follows_the_camera_and_the_map_pixel() {
        let mut app = app();
        let at = Vec3::new(-451.8, 120.0, -8860.4);
        rico(&mut app, at);
        let cam = camera(&mut app, 0.0);
        run(&mut app, 3);
        let nodes = |app: &App| app.world().resource::<HudNodes>().minimap_map;
        let angle = |app: &App| {
            app.world()
                .get::<UiTransform>(nodes(app))
                .unwrap()
                .rotation
                .as_radians()
        };
        assert!(angle(&app).abs() < 1e-4);
        for yaw in [0.7_f32, -1.2, 2.5] {
            app.world_mut().get_mut::<Transform>(cam).unwrap().rotation =
                Quat::from_rotation_y(yaw);
            run(&mut app, 2);
            let v = app.world().resource::<MinimapView>();
            // the camera turned left by `yaw`: the picture turns the same way round
            assert!((v.rotation - yaw).abs() < 1e-4, "{} vs {yaw}", v.rotation);
            let ui = angle(&app);
            assert!((ui - yaw).abs() < 1e-4, "{ui} vs {yaw}");
            // the pixel is the map's, 2048 across
            let uv = bevy_to_map(at);
            assert!((v.center_px - uv * 2048.0).length() < 1e-2);
            assert!((v.center_px - Vec2::new(996.0, 470.0)).length() < 1.0);
            // the part drawn is centred on it, in the overview and in the patchwork
            assert!((v.rect.center() - v.center_px).length() < 1e-2);
            let a = v.atlas_rect.unwrap();
            assert!(
                (a.center() - Vec2::splat(ATLAS_PX as f32 / 2.0)).length() < ATLAS_PX as f32 / 3.0
            );
        }
        // the compass "N" sits where north has gone
        let v = app.world().resource::<MinimapView>();
        assert!((v.north - ring_point(-v.heading)).length() < 1e-5);
    }

    /// `F3` shows the developer stats and hides them again; they start hidden.
    #[test]
    fn f3_toggles_the_developer_text() {
        let mut app = app();
        let old = app
            .world_mut()
            .spawn((Text::new("tiles 12"), DevText, Visibility::Inherited))
            .id();
        let vis = |app: &App| *app.world().get::<Visibility>(old).unwrap();
        run(&mut app, 2);
        assert_eq!(vis(&app), Visibility::Hidden, "hidden by default");
        assert!(!app.world().resource::<DevStats>().0);
        let tap = |app: &mut App| {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::F3);
            app.update();
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::F3);
            keys.bypass_change_detection().clear();
        };
        tap(&mut app);
        assert_eq!(vis(&app), Visibility::Inherited);
        assert!(app.world().resource::<DevStats>().0);
        run(&mut app, 2);
        assert_eq!(vis(&app), Visibility::Inherited, "stays on");
        tap(&mut app);
        assert_eq!(vis(&app), Visibility::Hidden);
    }

    /// Kills and destroyed objects raise the 60-second notices once the counts reach 3
    /// and then every 5.
    #[test]
    fn counts_raise_notifications() {
        let mut app = app();
        rico(&mut app, Vec3::ZERO);
        camera(&mut app, 0.0);
        app.init_resource::<StructureHits>();
        run(&mut app, 2);
        for destroyed in [1, 2, 3] {
            app.world_mut().resource_mut::<StructureHits>().destroyed = destroyed;
            run(&mut app, 2);
        }
        let notices = |app: &App| {
            app.world()
                .resource::<Notices>()
                .0
                .iter()
                .map(|n| n.text.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            notices(&app),
            ["3 objects destroyed in the last 60 seconds"]
        );
        app.world_mut().resource_mut::<StructureHits>().destroyed = 5;
        run(&mut app, 2);
        assert_eq!(
            notices(&app).last().unwrap(),
            "5 objects destroyed in the last 60 seconds"
        );
        // the UI rows show them
        let n = app.world().resource::<HudNodes>();
        assert_eq!(
            text_of(&app, n.notices[0].1),
            "3 objects destroyed in the last 60 seconds"
        );
        assert_eq!(
            app.world().get::<Node>(n.notices[1].0).unwrap().display,
            Display::Flex
        );
        // they fade away
        run(&mut app, 90);
        assert!(notices(&app).is_empty());
        assert_eq!(
            app.world()
                .get::<Node>(app.world().resource::<HudNodes>().notices[0].0)
                .unwrap()
                .display,
            Display::None
        );
    }

    /// The ammo counter follows the gun: shots count it down, a reload fills it. Needs
    /// JC2_TEST_DIR (the weapon tables and the text database).
    #[test]
    fn the_ammo_counter_follows_firing_and_reloads() {
        use bevy::input::mouse::AccumulatedMouseMotion;
        let Some((mut app, _, _)) =
            crate::fleet::testing::app(&["weapons.bin", "general.blz"], |app| {
                app.init_resource::<ButtonInput<MouseButton>>()
                    .init_resource::<AccumulatedMouseMotion>()
                    .add_plugins((CombatPlugin, HudPlugin));
            })
        else {
            return;
        };
        rico(&mut app, Vec3::new(-256.0, 300.0, -8960.0));
        camera(&mut app, 0.0);
        let number = |app: &App| text_of(app, app.world().resource::<HudNodes>().slot_number);
        let name = |app: &App| text_of(app, app.world().resource::<HudNodes>().name_text);
        // wall-clock wait: the data loads on the frames that follow
        run(&mut app, 1);
        let mut tries = 0;
        while number(&app) != "12" && tries < 400 {
            run(&mut app, 1);
            std::thread::sleep(Duration::from_millis(5));
            tries += 1;
        }
        assert_eq!(number(&app), "12", "a full Beretta magazine");
        assert_eq!(name(&app), "Pistol");
        // Rico starts holstered: the small plain cross shows, not the weapon bracket
        let shown = |app: &App, e: Entity| {
            app.world().get::<Node>(e).unwrap().display != Display::None
        };
        let (bracket, plain) = {
            let n = app.world().resource::<HudNodes>();
            (n.crosshair, n.crosshair_plain)
        };
        assert!(!shown(&app, bracket) && shown(&app, plain));
        assert_eq!(app.world().get::<Children>(plain).unwrap().len(), 5);
        app.world_mut().resource_mut::<Arsenal>().drawn = true;
        run(&mut app, 2);
        assert!(shown(&app, bracket) && !shown(&app, plain));
        app.world_mut().resource_mut::<Arsenal>().drawn = false;
        assert_eq!(
            text_of(&app, app.world().resource::<HudNodes>().grenade_text),
            "Fragmentation Grenade"
        );
        assert_eq!(
            text_of(&app, app.world().resource::<HudNodes>().grenade_count),
            "10"
        );
        // three shots
        for _ in 0..3 {
            assert!(
                app.world_mut().resource_mut::<Arsenal>().guns[0]
                    .ammo
                    .fire()
            );
        }
        run(&mut app, 2);
        assert_eq!(number(&app), "9");
        let v = app.world().resource::<HudView>();
        assert!((v.slot.fraction - 9.0 / 12.0).abs() < 1e-5);
        // the reload: empties the ring while it runs, then the magazine is full again
        assert!(
            app.world_mut().resource_mut::<Arsenal>().guns[0]
                .ammo
                .start_reload()
        );
        run(&mut app, 2);
        assert!(app.world().resource::<HudView>().slot.reloading);
        // (the test app steps 1/64 s a frame; a reload takes 0.8 s)
        run(&mut app, 64);
        assert_eq!(number(&app), "12");
        assert!(!app.world().resource::<HudView>().slot.reloading);
        // another gun: the name and the icon follow
        {
            let mut a = app.world_mut().resource_mut::<Arsenal>();
            a.current = 1;
        }
        run(&mut app, 2);
        assert_eq!(name(&app), "Submachine Gun");
        assert_eq!(number(&app), "30");
        let icon = app.world().resource::<HudNodes>().slot_icon;
        let rect = app.world().get::<ImageNode>(icon).unwrap().rect.unwrap();
        assert_eq!(
            rect.min,
            Vec2::new(128.0, 0.0),
            "the SMG is cell 2 of the atlas"
        );
        eprintln!("hud: the ammo counter test ran against the game data");
    }

    /// Entering a settlement's area shows its banner, once, and counts the discovery.
    /// Needs JC2_TEST_DIR.
    #[test]
    fn a_settlement_banner_shows_on_entering() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = jc2_formats::GameFs::open(dir) else {
            return;
        };
        let data = MapData::load(&fs);
        let Some(base) = data.settlements.iter().find(|s| s.id == "31x14") else {
            return;
        };
        let at = Vec3::new(base.position[0], 100.0, base.position[2]);
        let mut app = app();
        app.insert_resource(data);
        // the text database for the name
        app.insert_resource(Game {
            fs: std::sync::Arc::new(fs),
        });
        let hero = rico(&mut app, at);
        camera(&mut app, 0.0);
        run(&mut app, 3);
        let b = app.world().resource::<AreaBanner>();
        assert!(b.active && !b.name.is_empty(), "{b:?}");
        assert_eq!(b.name, "Lembah Genting Tinggi");
        let notices = &app.world().resource::<Notices>().0;
        assert!(
            notices
                .iter()
                .any(|n| n.text == "Locations discovered: 1 / 368"),
            "{notices:?}"
        );
        let n = app.world().resource::<HudNodes>();
        assert_eq!(text_of(&app, n.area_name), "Lembah Genting Tinggi");
        assert_eq!(
            app.world().get::<Node>(n.area_banner).unwrap().display,
            Display::Flex
        );
        assert!(app.world().resource::<InArea>().0.is_some());
        // leaving and coming back does not count it twice
        app.world_mut()
            .get_mut::<Transform>(hero)
            .unwrap()
            .translation = Vec3::new(15000.0, 100.0, 15000.0);
        run(&mut app, 2);
        assert!(app.world().resource::<InArea>().0.is_none());
        app.world_mut()
            .get_mut::<Transform>(hero)
            .unwrap()
            .translation = at;
        run(&mut app, 2);
        assert_eq!(app.world().resource::<HudStats>().locations.len(), 1);
        eprintln!("hud: the settlement test ran against the game data");
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PromptComputed;
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct HudApplied;
