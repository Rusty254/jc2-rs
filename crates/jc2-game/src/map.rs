//! The map and fast travel. `M` opens the PDA's map of Panau full screen
//! (`pda_map_dif.dds`, 2048² at 16 m a pixel, north up), with the settlements from
//! `settings\settlements.bin` / `icons_global.bin` drawn with their `icon_types.bin`
//! icons, an arrow for Rico (or the free camera) and the game's car / helicopter / boat
//! icons for the vehicles in the world. Clicking the map travels there: Rico is dropped
//! [`DROP_HEIGHT`] above the ground so the tiles stream in while he falls and he can
//! open the chute; a helicopter or plane he is flying goes with him, at a safe height
//! over the hills around; from a car or a boat he goes alone. `M` or `Esc` closes it.
//!
//! While the map is open the game is paused (the clock, so physics and the soldiers
//! stop, as with the original's PDA), the cursor is free, and the frame's keyboard and
//! mouse input is swallowed after the map has read it, so nothing else — guns, Rico,
//! the vehicles, the cameras — sees a key or a click. After a jump the clock stays
//! stopped until the tile under the destination has loaded.
//!
//! The pixel of engine `(x, z)` is `((x + 16384) / 16, (z + 16384) / 16)`: the image's
//! rows run with engine z like the height maps (`research/formats/map.md`). The engine's
//! axes are Bevy's (`coords.rs`): x east (right on the map), z south (down the map).

use crate::Game;
use crate::boat::Watercraft;
use crate::character::Hero;
use crate::coords::{MODEL_FORWARD, bevy_xyz, engine_xz, facing_dir, forward_of, yaw_of};
use crate::heli::Heli;
use crate::plane::Plane;
use crate::vehicle::{CameraMode, Enterable, Occupied};
use crate::world::{FlyCamera, SEA_LEVEL, Streamer};
use avian3d::prelude::*;
use bevy::asset::{LoadState, RenderAssetUsages};
use bevy::input::InputSystems;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::{RelativeCursorPosition, UiSystems};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use jc2_formats::GameFs;
use jc2_formats::icons::{
    SettlementIcon, icon_types, kind_name, map_icons, map_uv, map_uv_to_world, settlement_icons,
    settlements,
};
use jc2_formats::terrain::{GlobalTerrain, TerrainTile, tile_file_name, tile_index, tile_origin};
use jc2_formats::text::TextDb;
use std::collections::HashMap;

/// The map image, a top-level file in `pc3`.
pub const MAP_TEXTURE: &str = "pda_map_dif.dds";
/// How high above the ground Rico is dropped (m).
pub const DROP_HEIGHT: f32 = 250.0;
/// A helicopter or plane arrives this far above the highest ground within
/// [`AIR_RADIUS`] (and never lower than [`DROP_HEIGHT`] over the spot).
pub const AIR_CLEARANCE: f32 = 150.0;
pub const AIR_RADIUS: f32 = 1500.0;
/// A plane slower than this × its stall speed is given that speed along its nose.
const PLANE_SPEED_MARGIN: f32 = 1.3;
/// Frames to wait for the destination tile before the clock runs again regardless.
const ARRIVAL_TIMEOUT: u32 = 600;
/// Icon sizes on screen (px).
const SETTLEMENT_ICON: f32 = 18.0;
const VEHICLE_ICON: f32 = 14.0;
const HERO_ICON: f32 = 24.0;
/// The settlement names drawn beside the icons of the larger kinds: the font is this
/// fraction of the map's side (the map is 94 vmin), so the layout is the same at any
/// window size (~8.5 px on a 720p window, 16 px on 1440p); the gap between icon and text
/// in px.
const NAME_FONT_FRAC: f32 = 0.0125;
const NAME_GAP: f32 = 3.0;
/// An average glyph is this wide, in fonts (for placing the labels without overlaps).
const NAME_GLYPH_WIDTH: f32 = 0.56;
/// The vehicle icons (`icon_types.bin` kinds 34, 36, 37: the yellow car, helicopter
/// and boat).
const CAR_ICON: &str = "hud_icon_car_yellow_dif.dds";
const HELI_ICON: &str = "hud_icon_heli_yellow_dif.dds";
const BOAT_ICON: &str = "hud_icon_boat_yellow_dif.dds";

/// Whether the map is open, and a fast travel on its way.
#[derive(Resource, Default, Debug)]
pub struct MapState {
    pub open: bool,
    /// Independent navigation marker in engine coordinates.
    pub waypoint: Option<Vec3>,
    /// A jump asked for, to engine `(x, z)`; `fast_travel` carries it out.
    pub travel: Option<Vec2>,
    /// After a jump: the destination tile and how many frames we have waited for it.
    pub arriving: Option<((u32, u32), u32)>,
    /// The map (or a jump) stopped the clock, so it starts it again.
    paused: bool,
    /// The cursor as it was before the map freed it.
    cursor: Option<(CursorGrabMode, bool)>,
}

/// What the map shows that comes from the data.
#[derive(Resource, Default)]
pub struct MapData {
    pub settlements: Vec<SettlementIcon>,
    /// The English text database, for the settlements' names (`None`: ids are shown).
    pub names: Option<TextDb>,
    /// The 64 m overview height map, for the ground under a click.
    pub global: Option<GlobalTerrain>,
}

impl MapData {
    pub fn load(fs: &GameFs) -> Self {
        let settlements = match (
            fs.read_inflated("icon_types.bin")
                .ok()
                .and_then(|d| icon_types(&d).ok()),
            fs.read_inflated("icons_global.bin")
                .ok()
                .and_then(|d| map_icons(&d).ok()),
            fs.read_inflated("settlements.bin")
                .ok()
                .and_then(|d| settlements(&d).ok()),
        ) {
            (Some(t), Some(i), Some(s)) => settlement_icons(&t, &i, &s),
            _ => {
                warn!("map: icon_types/icons_global/settlements.bin not readable, no settlements");
                Vec::new()
            }
        };
        let global = fs
            .read_inflated("terrain_win32.dat")
            .ok()
            .and_then(|d| GlobalTerrain::parse(&d).ok());
        let names = TextDb::load(fs, "en");
        if names.is_none() {
            warn!("map: text database not readable, settlements are shown by id");
        }
        Self {
            settlements,
            names,
            global,
        }
    }

    /// A settlement's name from the text database, or its id where there is none.
    pub fn name_of<'a>(&'a self, s: &'a SettlementIcon) -> &'a str {
        self.names
            .as_ref()
            .and_then(|n| n.get(&s.text))
            .unwrap_or(&s.id)
    }
}

/// Settlements of these kinds get their name written beside the icon: military bases,
/// airfields and harbours, airfields, harbours, cities and strongholds (not the 175
/// villages, whose names would swamp the map; they show on hover).
fn labelled(kind: i32) -> bool {
    matches!(kind, 24..=29 | 32)
}

/// Which labels win where they would overlap: cities, then strongholds and the bases,
/// then the harbours and airfields.
fn label_priority(kind: i32) -> u8 {
    match kind {
        29 => 0,
        32 => 1,
        24 => 2,
        25 | 26 => 3,
        _ => 4,
    }
}

/// The settlements whose names fit on the map without covering one another, in the
/// order of [`label_priority`] (the rest show on hover). Rectangles are in map units
/// (0..1 on each axis) for a label font of [`NAME_FONT_FRAC`] of the map.
fn visible_labels(data: &MapData) -> Vec<&SettlementIcon> {
    let mut cands: Vec<&SettlementIcon> = data
        .settlements
        .iter()
        .filter(|s| labelled(s.kind))
        .collect();
    cands.sort_by_key(|s| label_priority(s.kind));
    let mut taken: Vec<(Vec2, Vec2)> = Vec::new();
    let mut out = Vec::new();
    for s in cands {
        let uv = world_to_map(s.position[0], s.position[2]);
        let w = data.name_of(s).chars().count() as f32 * NAME_GLYPH_WIDTH * NAME_FONT_FRAC;
        let h = NAME_FONT_FRAC * 1.1;
        let min = Vec2::new(uv.x + 0.014, uv.y - h / 2.0);
        let max = min + Vec2::new(w, h);
        if taken
            .iter()
            .any(|(a, b)| min.x < b.x && max.x > a.x && min.y < b.y && max.y > a.y)
        {
            continue;
        }
        taken.push((min, max));
        out.push(s);
    }
    out
}

/// Engine `(x, z)` → the map (0..1 each way, top-left origin).
pub fn world_to_map(x: f32, z: f32) -> Vec2 {
    let (u, v) = map_uv(x, z);
    Vec2::new(u, v)
}

/// The map (0..1 each way) → engine `(x, z)`.
pub fn map_to_world(uv: Vec2) -> Vec2 {
    let (x, z) = map_uv_to_world(uv.x, uv.y);
    Vec2::new(x, z)
}

/// A Bevy position → the map.
pub fn bevy_to_map(p: Vec3) -> Vec2 {
    let (x, z) = engine_xz(p);
    world_to_map(x, z)
}

/// Heading of a Bevy direction as the map's clockwise rotation from north (up): the
/// map's up is engine −z (Bevy's too), its right +x.
pub fn map_heading(dir: Vec3) -> f32 {
    dir.x.atan2(-dir.z)
}

/// The arrow's heading on the map for a character facing `yaw` (`Hero::facing`).
pub fn facing_heading(yaw: f32) -> f32 {
    map_heading(facing_dir(yaw))
}

/// The ground (or the sea) at engine `(x, z)`: the detailed tile when it reads, else
/// the 64 m overview map.
pub fn ground_at(fs: &GameFs, global: Option<&GlobalTerrain>, x: f32, z: f32) -> f32 {
    let exact = tile_index(x, z).and_then(|(tx, tz)| {
        let (x0, z0) = tile_origin(tx, tz);
        fs.read_inflated(&tile_file_name(tx, tz))
            .ok()
            .and_then(|d| TerrainTile::parse(&d).ok())
            .and_then(|t| t.sample_local(x - x0, z - z0))
    });
    exact
        .or_else(|| global.map(|g| g.height_at(x, z)))
        .unwrap_or(SEA_LEVEL)
        .max(SEA_LEVEL)
}

/// The highest ground within `radius` of engine `(x, z)` on the overview map.
pub fn highest_around(global: &GlobalTerrain, x: f32, z: f32, radius: f32) -> f32 {
    let step = jc2_formats::terrain::GLOBAL_CELL;
    let n = (radius / step).ceil() as i32;
    let mut top = SEA_LEVEL;
    for j in -n..=n {
        for i in -n..=n {
            let (dx, dz) = (i as f32 * step, j as f32 * step);
            if dx * dx + dz * dz <= radius * radius {
                top = top.max(global.height_at(x + dx, z + dz));
            }
        }
    }
    top
}

#[derive(Component)]
struct MapRoot;
#[derive(Component)]
struct MapImage;
#[derive(Component)]
struct HeroMarker;
#[derive(Component)]
struct VehicleMarker;
#[derive(Component)]
struct MapLabel;
#[derive(Component)]
struct MapHeader;
/// A settlement's icon on the map, by its id (a taken stronghold changes its texture).
#[derive(Component)]
struct SettlementIconOf(String);
/// An icon of [`MapPins`].
#[derive(Component)]
struct PinMarker;

/// Icons other modules put on the map: where (Bevy axes) and the texture (a colonel, the
/// running mission's objectives; `missions.rs`).
#[derive(Resource, Default, Debug)]
pub struct MapPins(pub Vec<(Vec3, String)>);

/// The vehicle icons, `None` where the texture is missing (a coloured dot instead).
#[derive(Resource)]
struct VehicleIcons {
    car: Option<Handle<Image>>,
    heli: Option<Handle<Image>>,
    boat: Option<Handle<Image>>,
}

/// Open the map as soon as it can (`--open-map`, for screenshots).
#[derive(Resource)]
pub struct OpenAtStart;

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapState>()
            .init_resource::<MapPins>()
            .add_systems(Startup, setup_map)
            .add_systems(
                PreUpdate,
                (map_input.in_set(MapInput), fast_travel)
                    .chain()
                    .after(InputSystems)
                    .after(UiSystems::Focus),
            )
            .add_systems(
                Update,
                (
                    show_map,
                    update_markers,
                    hover_label,
                    refresh_settlement_icons,
                    update_pins,
                ),
            )
            .add_systems(PostUpdate, arrive);
    }
}

fn setup_map(
    mut commands: Commands,
    game: Res<Game>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    let data = MapData::load(&game.fs);
    info!(
        "map: {} settlements, overview heights {}",
        data.settlements.len(),
        if data.global.is_some() {
            "read"
        } else {
            "missing"
        }
    );
    let texture = |name: &str| {
        game.fs
            .contains(name)
            .then(|| assets.load::<Image>(format!("jc2://{name}")))
    };
    if !game.fs.contains(MAP_TEXTURE) {
        warn!("{MAP_TEXTURE} not found: the map is blank");
    }
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.03, 0.05, 0.94)),
            GlobalZIndex(100),
            Visibility::Hidden,
            MapRoot,
            Name::new("map"),
        ))
        .id();
    // letter-boxed: a square as big as the window's shorter side allows
    let mut map = commands.spawn((
        Node {
            width: Val::VMin(94.0),
            height: Val::VMin(94.0),
            ..default()
        },
        RelativeCursorPosition::default(),
        MapImage,
        Name::new("map image"),
    ));
    match texture(MAP_TEXTURE) {
        Some(h) => map.insert(ImageNode::new(h)),
        None => map.insert(BackgroundColor(Color::srgb(0.05, 0.15, 0.25))),
    };
    let map = map.id();
    commands.entity(root).add_child(map);
    for s in &data.settlements {
        let uv = world_to_map(s.position[0], s.position[2]);
        let mut icon = commands.spawn((
            marker_node(uv, SETTLEMENT_ICON),
            SettlementIconOf(s.id.clone()),
            Name::new(format!("settlement {}", s.id)),
        ));
        match s.texture.as_deref().and_then(texture) {
            Some(h) => icon.insert(ImageNode::new(h)),
            None => icon.insert((
                dot_node(uv, SETTLEMENT_ICON),
                BackgroundColor(kind_colour(s.kind)),
            )),
        };
        let icon = icon.id();
        commands.entity(map).add_child(icon);
    }
    for s in visible_labels(&data) {
        let uv = world_to_map(s.position[0], s.position[2]);
        let name = commands
            .spawn((
                Text::new(data.name_of(s)),
                TextFont {
                    font_size: FontSize::VMin(NAME_FONT_FRAC * 94.0),
                    ..default()
                },
                TextColor(Color::srgb(0.96, 0.96, 0.9)),
                TextShadow {
                    offset: Vec2::splat(1.0),
                    color: Color::srgba(0.0, 0.0, 0.0, 0.9),
                },
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(uv.x * 100.0),
                    top: Val::Percent(uv.y * 100.0),
                    margin: UiRect {
                        left: Val::Px(SETTLEMENT_ICON / 2.0 + NAME_GAP),
                        top: Val::VMin(-NAME_FONT_FRAC * 94.0 * 0.6),
                        ..default()
                    },
                    ..default()
                },
                Name::new(format!("name {}", s.id)),
            ))
            .id();
        commands.entity(map).add_child(name);
    }
    let arrow = images.add(arrow_image());
    let hero = commands
        .spawn((
            marker_node(Vec2::splat(0.5), HERO_ICON),
            ImageNode::new(arrow),
            UiTransform::IDENTITY,
            ZIndex(10),
            HeroMarker,
            Name::new("map: rico"),
        ))
        .id();
    commands.entity(map).add_child(hero);
    let label = commands
        .spawn((
            Text::new(""),
            TextFont::from_font_size(16.0),
            TextColor(Color::srgb(0.92, 0.94, 0.90)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                bottom: Val::Px(10.0),
                ..default()
            },
            MapLabel,
        ))
        .id();
    let hint = commands
        .spawn((
            Text::new("MAP   click to travel there  ·  M or Esc closes"),
            MapHeader,
            TextFont::from_font_size(16.0),
            TextColor(Color::srgb(0.92, 0.94, 0.90)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                top: Val::Px(10.0),
                ..default()
            },
        ))
        .id();
    commands.entity(root).add_children(&[label, hint]);
    commands.insert_resource(VehicleIcons {
        car: texture(CAR_ICON),
        heli: texture(HELI_ICON),
        boat: texture(BOAT_ICON),
    });
    commands.insert_resource(data);
}

/// An icon of `size` px centred on map position `uv`.
fn marker_node(uv: Vec2, size: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(uv.x * 100.0),
        top: Val::Percent(uv.y * 100.0),
        width: Val::Px(size),
        height: Val::Px(size),
        margin: UiRect {
            left: Val::Px(-size / 2.0),
            top: Val::Px(-size / 2.0),
            ..default()
        },
        ..default()
    }
}

/// A round dot of `size` px at `uv` (when an icon texture is missing).
fn dot_node(uv: Vec2, size: f32) -> Node {
    Node {
        border_radius: BorderRadius::MAX,
        ..marker_node(uv, size * 0.7)
    }
}

/// The dot a settlement gets when its icon texture is missing.
fn kind_colour(kind: i32) -> Color {
    match kind {
        24..=26 => Color::srgb(0.85, 0.15, 0.1),
        27 => Color::srgb(0.3, 0.6, 1.0),
        28 => Color::srgb(0.2, 0.8, 0.8),
        29 => Color::srgb(0.95, 0.95, 0.95),
        _ => Color::srgb(0.95, 0.8, 0.2),
    }
}

/// A 32 × 32 arrow pointing up (north at no rotation), yellow with a dark rim.
fn arrow_image() -> Image {
    const N: usize = 32;
    let (apex, left, right) = (
        Vec2::new(16.0, 2.0),
        Vec2::new(4.0, 29.0),
        Vec2::new(28.0, 29.0),
    );
    let notch = Vec2::new(16.0, 22.0);
    let edge = |a: Vec2, b: Vec2, p: Vec2| (b - a).perp_dot(p - a);
    // the arrow is the quad apex-right-notch-left, concave at the notch: the union of
    // its two halves
    let in_triangle = |p: Vec2, a: Vec2, b: Vec2, c: Vec2| {
        let d = [edge(a, b, p), edge(b, c, p), edge(c, a, p)];
        d.iter().all(|&v| v >= 0.0) || d.iter().all(|&v| v <= 0.0)
    };
    let in_arrow =
        |p: Vec2| in_triangle(p, apex, right, notch) || in_triangle(p, apex, notch, left);
    // inside by at least `inset`: every point that far around is inside too
    let inside = |p: Vec2, inset: f32| {
        in_arrow(p)
            && (0..8).all(|k| {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                in_arrow(p + inset * Vec2::new(a.cos(), a.sin()))
            })
    };
    let mut data = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let c = if inside(p, 1.6) {
                [255, 214, 40, 255]
            } else if inside(p, 0.0) {
                [20, 20, 20, 255]
            } else {
                [0, 0, 0, 0]
            };
            data[(y * N + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    Image::new(
        Extent3d {
            width: N as u32,
            height: N as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
}

type WindowCursor<'w, 's> = Query<'w, 's, &'static mut CursorOptions, With<PrimaryWindow>>;

/// `M` / `Esc` / a click on the map; while the map is open, nothing else gets the keys
/// or the mouse.
#[allow(clippy::too_many_arguments)]
fn map_input(
    mut commands: Commands,
    mut state: ResMut<MapState>,
    keys: Option<ResMut<ButtonInput<KeyCode>>>,
    buttons: Option<ResMut<ButtonInput<MouseButton>>>,
    motion: Option<ResMut<AccumulatedMouseMotion>>,
    scroll: Option<ResMut<AccumulatedMouseScroll>>,
    open_at_start: Option<Res<OpenAtStart>>,
    browser: Option<ResMut<crate::mission_browser::MissionBrowser>>,
    maps: Query<&RelativeCursorPosition, With<MapImage>>,
    pictures: Query<Option<&ImageNode>, With<MapImage>>,
    assets: Res<AssetServer>,
    mut cursors: WindowCursor,
    mut time: ResMut<Time<Virtual>>,
) {
    let was_open = state.open;
    let mut browser = browser;
    let (Some(mut keys), Some(mut buttons)) = (keys, buttons) else {
        return;
    };
    let mut cursor = cursors.single_mut().ok();
    if !state.open {
        // `--open-map` waits until there is a map to show: its UI is spawned and its
        // picture (loaded on another thread) has arrived, else the screenshot would be
        // of a dark empty panel
        let ready = pictures.single().is_ok_and(|p| {
            p.is_none_or(|n| {
                !matches!(
                    assets.get_load_state(n.image.id()),
                    Some(LoadState::NotLoaded | LoadState::Loading)
                )
            })
        });
        let asked = keys.just_pressed(KeyCode::KeyM)
            || (open_at_start.is_some() && ready)
            || browser.as_ref().is_some_and(|b| b.open_requested);
        if asked && state.arriving.is_none() && state.travel.is_none() {
            commands.remove_resource::<OpenAtStart>();
            state.open = true;
            if let Some(b) = browser.as_mut() {
                b.open_requested = false;
            }
            if !time.is_paused() {
                time.pause();
                state.paused = true;
            }
            if let Some(c) = cursor.as_mut() {
                state.cursor = Some((c.grab_mode, c.visible));
                c.grab_mode = CursorGrabMode::None;
                c.visible = true;
            }
        }
    } else {
        let close = keys.just_pressed(KeyCode::KeyM)
            || keys.just_pressed(KeyCode::Escape)
            || browser.as_ref().is_some_and(|b| b.close_requested);
        let click = (buttons.just_pressed(MouseButton::Left)
            && !close
            && browser
                .as_ref()
                .is_none_or(|b| b.view == crate::mission_browser::BrowserView::Map))
        .then(|| maps.single().ok())
        .flatten()
        .filter(|r| r.cursor_over())
        .and_then(|r| r.normalized);
        if let Some(n) = click {
            let uv = (n + Vec2::splat(0.5)).clamp(Vec2::ZERO, Vec2::ONE);
            let to = map_to_world(uv);
            info!("map: travelling to {:.0} {:.0}", to.x, to.y);
            state.travel = Some(to);
        }
        if close || click.is_some() {
            state.open = false;
            if let Some(b) = browser.as_mut() {
                b.close_requested = false;
                b.open_requested = false;
                b.view = crate::mission_browser::BrowserView::Map;
                b.giver_origin = None;
            }
            if let (Some(c), Some((grab, visible))) = (cursor.as_mut(), state.cursor.take()) {
                c.grab_mode = grab;
                c.visible = visible;
            }
            // a jump keeps the clock stopped until the destination has loaded
            if state.travel.is_none() && state.paused {
                time.unpause();
                state.paused = false;
            }
        }
    }
    if was_open || state.open {
        keys.reset_all();
        buttons.reset_all();
        if let Some(mut m) = motion {
            m.delta = Vec2::ZERO;
        }
        if let Some(mut s) = scroll {
            s.delta = Vec2::ZERO;
        }
    }
}

type HeroItem = (
    Entity,
    &'static mut Hero,
    &'static mut Transform,
    Option<&'static mut Visibility>,
    Option<&'static mut LinearVelocity>,
    Option<&'static mut Position>,
);
type VehicleItem = (
    Entity,
    &'static mut Transform,
    Option<&'static mut LinearVelocity>,
    Option<&'static mut AngularVelocity>,
    Option<&'static mut Position>,
    Has<Heli>,
    Option<&'static Plane>,
    Has<Occupied>,
);
type VehicleQuery<'w, 's> = Query<'w, 's, VehicleItem, (With<Enterable>, Without<Hero>)>;

/// Send a vehicle to `aloft` (a helicopter or plane: keeps its velocity, a plane at
/// least a little over its stall speed) or `landed` (the rest: stopped, upright).
fn send_vehicle(v: Entity, vehicles: &mut VehicleQuery, aloft: Vec3, landed: Vec3) -> Option<Vec3> {
    let (_, mut tf, lin, ang, pos, heli, plane, _) = vehicles.get_mut(v).ok()?;
    let flying = heli || plane.is_some();
    let at = if flying { aloft } else { landed };
    tf.translation = at;
    if let Some(mut p) = pos {
        p.0 = at;
    }
    if let Some(mut a) = ang {
        a.0 = Vec3::ZERO;
    }
    if let Some(mut lin) = lin {
        if let Some(p) = plane {
            let fwd = forward_of(tf.rotation);
            let need = p.stall_speed() * PLANE_SPEED_MARGIN;
            if lin.0.dot(fwd) < need {
                lin.0 = fwd * need;
            }
        } else if !flying {
            lin.0 = Vec3::ZERO;
        }
    }
    if !flying {
        tf.rotation = Quat::from_rotation_y(yaw_of(tf.rotation));
    }
    Some(at)
}

/// Carry out a jump: Rico (with the helicopter or plane he flies), else the vehicle
/// being driven, else the free camera. The camera keeps its offset from whoever went.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn fast_travel(
    mut commands: Commands,
    mut state: ResMut<MapState>,
    data: Option<Res<MapData>>,
    game: Res<Game>,
    mut time: ResMut<Time<Virtual>>,
    loadscreen: Option<ResMut<crate::loadscreen::LoadScreen>>,
    mode: Option<ResMut<CameraMode>>,
    mut heroes: Query<HeroItem, Without<Enterable>>,
    mut vehicles: VehicleQuery,
    mut cams: Query<&mut Transform, (With<FlyCamera>, Without<Hero>, Without<Enterable>)>,
) {
    let Some(to) = state.travel.take() else {
        return;
    };
    let global = data.as_ref().and_then(|d| d.global.as_ref());
    let (x, z) = (to.x, to.y);
    let ground = ground_at(&game.fs, global, x, z);
    let air = global
        .map(|g| highest_around(g, x, z, AIR_RADIUS) + AIR_CLEARANCE)
        .unwrap_or(0.0)
        .max(ground + DROP_HEIGHT);
    let drop = bevy_xyz(x, ground + DROP_HEIGHT, z);
    let aloft = bevy_xyz(x, air, z);
    let landed = bevy_xyz(x, ground + 3.0, z);
    let mut from = None;
    let mut arrived = drop;
    if let Ok((hero_e, mut hero, mut tf, vis, lin, pos)) = heroes.single_mut() {
        from = Some(tf.translation);
        let ride = hero
            .in_vehicle
            .filter(|v| vehicles.get(*v).is_ok_and(|i| i.5 || i.6.is_some()));
        if let Some(v) = ride {
            // the helicopter or plane goes, with him in it
            arrived = send_vehicle(v, &mut vehicles, aloft, landed).unwrap_or(drop);
            tf.translation = arrived;
            if let Some(mut p) = pos {
                p.0 = arrived;
            }
        } else {
            if let Some(v) = hero.in_vehicle.take() {
                // out of the car or the boat: it stays behind
                commands.entity(v).remove::<Occupied>();
                crate::physics::release_passenger(&mut commands, hero_e, drop, Vec3::ZERO);
                if let Some(mut vis) = vis {
                    *vis = Visibility::Inherited;
                }
                if let Some(mut m) = mode {
                    *m = CameraMode::Hero;
                }
            }
            tf.translation = drop;
            if let Some(mut p) = pos {
                p.0 = drop;
            }
            if let Some(mut v) = lin {
                v.0 = Vec3::ZERO;
            }
            hero.grapple = None;
            hero.parachute = false;
            hero.grounded = false;
            hero.swimming = false;
            hero.air_time = 0.0;
        }
        info!("rico: fast travel to {arrived:?} (ground {ground:.0} m)");
    } else if let Some((v, at)) = vehicles
        .iter()
        .find(|i| i.7)
        .map(|i| (i.0, i.1.translation))
    {
        from = Some(at);
        arrived = send_vehicle(v, &mut vehicles, aloft, landed).unwrap_or(drop);
    }
    for mut cam in &mut cams {
        let offset = from.map_or(Vec3::ZERO, |f| cam.translation - f);
        cam.translation = arrived + offset;
    }
    if let Some(mut ls) = loadscreen {
        ls.request(crate::loadscreen::LoadReason::FastTravel);
    }
    if let Some(tile) = tile_index(x, z) {
        state.arriving = Some((tile, 0));
        if !time.is_paused() {
            time.pause();
            state.paused = true;
        }
    } else if state.paused {
        time.unpause();
        state.paused = false;
    }
}

/// After a jump: start the clock again once the tile under the destination is in.
fn arrive(
    mut state: ResMut<MapState>,
    streamer: Option<Res<Streamer>>,
    loadscreen: Option<Res<crate::loadscreen::LoadScreen>>,
    mut time: ResMut<Time<Virtual>>,
) {
    let Some((tile, frames)) = state.arriving else {
        return;
    };
    // the loading screen (`loadscreen.rs`) holds the clock until the area around is in
    let ready = streamer.is_none_or(|s| s.loaded.contains_key(&tile))
        && loadscreen.is_none_or(|l| !l.active);
    if ready || frames >= ARRIVAL_TIMEOUT {
        if !ready {
            warn!("map: tile {tile:?} still not loaded, going on anyway");
        }
        state.arriving = None;
        if state.paused && !state.open {
            time.unpause();
            state.paused = false;
        }
    } else {
        state.arriving = Some((tile, frames + 1));
    }
}

fn show_map(
    state: Res<MapState>,
    mut roots: Query<&mut Visibility, With<MapRoot>>,
    browser: Option<Res<crate::mission_browser::MissionBrowser>>,
    mut headers: Query<&mut Node, With<MapHeader>>,
) {
    // every frame, not only when the state changes: whatever order the UI was spawned in
    for mut v in &mut roots {
        v.set_if_neq(if state.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }
    for mut header in &mut headers {
        header.display = if browser
            .as_ref()
            .is_none_or(|b| b.view == crate::mission_browser::BrowserView::Map)
        {
            Display::Flex
        } else {
            Display::None
        };
    }
}

type VehicleMarkers<'w, 's> =
    Query<'w, 's, &'static mut Node, (With<VehicleMarker>, Without<HeroMarker>, Without<MapImage>)>;

/// Rico's arrow and the vehicles' icons follow them while the map is open.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_markers(
    mut commands: Commands,
    state: Res<MapState>,
    icons: Option<Res<VehicleIcons>>,
    heroes: Query<(&Hero, &Transform)>,
    cams: Query<&Transform, With<FlyCamera>>,
    vehicles: Query<
        (
            Entity,
            &Transform,
            Has<Heli>,
            Has<Plane>,
            Has<Watercraft>,
            Has<Occupied>,
        ),
        With<Enterable>,
    >,
    maps: Query<Entity, With<MapImage>>,
    mut hero_marker: Query<(&mut Node, &mut UiTransform), With<HeroMarker>>,
    mut markers: VehicleMarkers,
    mut known: Local<HashMap<Entity, Entity>>,
) {
    if !state.open {
        return;
    }
    let Ok(map) = maps.single() else { return };
    // who the arrow stands for, and which way it points
    let heading_of = |tf: &Transform, forward: Vec3| map_heading(tf.rotation * forward);
    let player = match heroes.single() {
        Ok((hero, tf)) => match hero.in_vehicle.and_then(|v| vehicles.get(v).ok()) {
            Some((_, vtf, ..)) => Some((vtf.translation, heading_of(vtf, MODEL_FORWARD))),
            None => Some((tf.translation, facing_heading(hero.facing))),
        },
        Err(_) => vehicles
            .iter()
            .find(|v| v.5)
            .map(|(_, vtf, ..)| (vtf.translation, heading_of(vtf, MODEL_FORWARD)))
            .or_else(|| {
                cams.single()
                    .ok()
                    .map(|c| (c.translation, heading_of(c, Vec3::NEG_Z)))
            }),
    };
    if let (Some((p, yaw)), Ok((mut node, mut ui))) = (player, hero_marker.single_mut()) {
        let uv = bevy_to_map(p);
        node.left = Val::Percent(uv.x * 100.0);
        node.top = Val::Percent(uv.y * 100.0);
        ui.rotation = Rot2::radians(yaw);
    }
    // vehicles: a marker each, gone with the vehicle; the one Rico is in is his arrow
    let ridden = heroes.single().ok().and_then(|(h, _)| h.in_vehicle);
    for (e, tf, heli, plane, boat, occupied) in &vehicles {
        let hidden = occupied || Some(e) == ridden;
        let uv = bevy_to_map(tf.translation);
        let marker = *known.entry(e).or_insert_with(|| {
            let (image, colour) = match (heli || plane, boat, icons.as_deref()) {
                (true, _, Some(i)) => (i.heli.clone(), Color::srgb(0.3, 0.7, 1.0)),
                (_, true, Some(i)) => (i.boat.clone(), Color::srgb(0.2, 0.9, 0.9)),
                (_, _, Some(i)) => (i.car.clone(), Color::srgb(1.0, 0.85, 0.2)),
                _ => (None, Color::WHITE),
            };
            let mut m = commands.spawn((
                marker_node(uv, VEHICLE_ICON),
                VehicleMarker,
                Name::new("map: vehicle"),
            ));
            match image {
                Some(h) => m.insert(ImageNode::new(h)),
                None => m.insert((dot_node(uv, VEHICLE_ICON), BackgroundColor(colour))),
            };
            let id = m.id();
            commands.entity(map).add_child(id);
            id
        });
        if let Ok(mut node) = markers.get_mut(marker) {
            node.left = Val::Percent(uv.x * 100.0);
            node.top = Val::Percent(uv.y * 100.0);
            node.display = if hidden { Display::None } else { Display::Flex };
        }
    }
    known.retain(|v, m| {
        let alive = vehicles.contains(*v);
        if !alive {
            commands.entity(*m).despawn();
        }
        alive
    });
}

/// A settlement's texture changed in [`MapData`] (a stronghold taken by a faction):
/// its icon follows.
fn refresh_settlement_icons(
    data: Option<Res<MapData>>,
    game: Res<Game>,
    assets: Res<AssetServer>,
    mut icons: Query<(&SettlementIconOf, &mut ImageNode)>,
    mut shown: Local<HashMap<String, String>>,
) {
    let Some(data) = data else { return };
    if !data.is_changed() {
        return;
    }
    for (of, mut image) in &mut icons {
        let Some(s) = data.settlements.iter().find(|s| s.id == of.0) else {
            continue;
        };
        let Some(tex) = &s.texture else { continue };
        if shown.get(&of.0) == Some(tex) {
            continue;
        }
        // the first look only learns what `setup_map` drew
        if shown.contains_key(&of.0) && game.fs.contains(tex) {
            image.image = assets.load(format!("jc2://{tex}"));
        }
        shown.insert(of.0.clone(), tex.clone());
    }
}

/// [`MapPins`] while the map is open: rebuilt when they change.
#[allow(clippy::too_many_arguments)]
fn update_pins(
    mut commands: Commands,
    state: Res<MapState>,
    pins: Res<MapPins>,
    game: Res<Game>,
    assets: Res<AssetServer>,
    maps: Query<Entity, With<MapImage>>,
    old: Query<Entity, With<PinMarker>>,
    mut was_open: Local<bool>,
) {
    let opened = state.open && !*was_open;
    *was_open = state.open;
    if !state.open || !(opened || pins.is_changed() || state.is_changed()) {
        return;
    }
    let Ok(map) = maps.single() else { return };
    for e in &old {
        commands.entity(e).despawn();
    }
    let mut markers = pins.0.clone();
    if let Some(p) = state.waypoint {
        markers.push((p, String::new()));
    }
    for (p, tex) in &markers {
        let uv = bevy_to_map(*p);
        let mut m = commands.spawn((
            marker_node(uv, SETTLEMENT_ICON),
            PinMarker,
            ZIndex(5),
            Name::new(format!("map pin {tex}")),
        ));
        if game.fs.contains(tex) {
            m.insert(ImageNode::new(assets.load(format!("jc2://{tex}"))));
        } else {
            m.insert((
                dot_node(uv, SETTLEMENT_ICON),
                BackgroundColor(Color::srgb(0.9, 0.1, 0.1)),
            ));
        }
        let id = m.id();
        commands.entity(map).add_child(id);
    }
}

/// Under the cursor: the engine position, the tile, the ground and a settlement.
fn hover_label(
    state: Res<MapState>,
    data: Option<Res<MapData>>,
    maps: Query<&RelativeCursorPosition, With<MapImage>>,
    mut labels: Query<&mut Text, With<MapLabel>>,
) {
    if !state.open {
        return;
    }
    let Ok(mut text) = labels.single_mut() else {
        return;
    };
    let over = maps
        .single()
        .ok()
        .filter(|r| r.cursor_over())
        .and_then(|r| r.normalized);
    let Some(n) = over else {
        text.0.clear();
        return;
    };
    let uv = n + Vec2::splat(0.5);
    let p = map_to_world(uv);
    let mut s = format!("{:.0}, {:.0}", p.x, p.y);
    if let Some((tx, tz)) = tile_index(p.x, p.y) {
        s += &format!("   tile {tx},{tz}");
    }
    if let Some(d) = &data {
        if let Some(g) = &d.global {
            s += &format!("   ground {:.0} m", g.height_at(p.x, p.y) - SEA_LEVEL);
        }
        // the nearest settlement within 1% of the map (~330 m)
        let near = d
            .settlements
            .iter()
            .map(|t| (t, world_to_map(t.position[0], t.position[2]).distance(uv)))
            .filter(|(_, dist)| *dist < 0.01)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((t, _)) = near {
            s += &format!(
                "   {} — {} ({})",
                d.name_of(t),
                kind_name(t.kind).unwrap_or("settlement"),
                t.id
            );
        }
    }
    text.0 = s;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::{HEIGHT, RADIUS};
    use crate::world::SpawnPoint;

    #[test]
    fn map_and_world_round_trip() {
        for p in [
            Vec2::new(-451.8, -8860.4),
            Vec2::new(0.0, 0.0),
            Vec2::new(16000.0, -15000.0),
        ] {
            let uv = world_to_map(p.x, p.y);
            assert!((map_to_world(uv) - p).length() < 1e-2);
            // Bevy's axes are the engine's
            assert!((bevy_to_map(Vec3::new(p.x, 300.0, p.y)) - uv).length() < 1e-6);
        }
        assert_eq!(world_to_map(-16384.0, -16384.0), Vec2::ZERO);
        assert_eq!(world_to_map(16384.0, 16384.0), Vec2::ONE);
        // north (up the map) is engine and Bevy −z: heading 0; east (+x) is a quarter
        // turn clockwise
        assert!(map_heading(Vec3::NEG_Z).abs() < 1e-6);
        assert!((map_heading(Vec3::X) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!(facing_heading(0.0).abs() < 1e-6);
        // the arrow image: opaque at its middle, clear in the corners
        let img = arrow_image();
        let data = img.data.as_ref().unwrap();
        let alpha = |x: usize, y: usize| data[(y * 32 + x) * 4 + 3];
        assert_eq!(alpha(16, 14), 255);
        assert_eq!(alpha(1, 1), 0);
        assert_eq!(alpha(16, 27), 0, "the notch");
    }

    /// Rico faces north (up the map) and walks to his left — the way `A` moves him with
    /// the camera behind him: the arrow points up, and moves left on the map. (With the
    /// old z mirror it moved right: the world was the map's mirror image.)
    #[test]
    fn walking_left_facing_north_moves_the_arrow_left() {
        use std::f32::consts::FRAC_PI_2;
        // north as the map has it: from a point to the one straight above it on the map
        let here = Vec2::new(-451.8, -8860.4);
        let up = map_to_world(world_to_map(here.x, here.y) - Vec2::new(0.0, 0.05));
        let north = (bevy_xyz(up.x, 0.0, up.y) - bevy_xyz(here.x, 0.0, here.y)).normalize();
        assert!(map_heading(north).abs() < 1e-4, "{north:?}");
        // Rico turned that way, the chase camera behind him looking the same way
        let facing = crate::coords::yaw_toward(north);
        assert!(facing_heading(facing).abs() < 1e-4);
        let cam = Transform::default().looking_to(north, Vec3::Y);
        // `A`: the camera's left (`character::hero_input`: `mv -= right`)
        let left = -cam.right().as_vec3();
        let p0 = bevy_xyz(here.x, 300.0, here.y);
        let p1 = p0 + left * 200.0;
        let (m0, m1) = (bevy_to_map(p0), bevy_to_map(p1));
        assert!(m1.x < m0.x, "the arrow went right: {m0:?} → {m1:?}");
        assert!((m1.y - m0.y).abs() < 1e-6);
        // and turned to walk that way, his arrow points left (west)
        let west = facing_heading(crate::coords::yaw_toward(left));
        assert!((west + FRAC_PI_2).abs() < 1e-4, "{west}");
    }

    /// The start base's icon lands inside tile 31,14 on the map. Needs JC2_TEST_DIR.
    #[test]
    fn the_base_is_on_its_tile() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = GameFs::open(dir) else { return };
        let data = MapData::load(&fs);
        if data.settlements.is_empty() {
            return;
        }
        let base = data.settlements.iter().find(|s| s.id == "31x14").unwrap();
        let uv = world_to_map(base.position[0], base.position[2]);
        // 64 tiles across the map
        assert_eq!(((uv.x * 64.0) as u32, (uv.y * 64.0) as u32), (31, 14));
        // and the map's pixel back to the world is the tile's ground
        let p = map_to_world(uv);
        assert_eq!(tile_index(p.x, p.y), Some((31, 14)));
        let g = data.global.as_ref().expect("terrain_win32.dat");
        let ground = ground_at(&fs, Some(g), p.x, p.y);
        assert!(
            (ground - base.position[1]).abs() < 30.0,
            "ground {ground} at the icon at {}",
            base.position[1]
        );
        assert!(highest_around(g, p.x, p.y, AIR_RADIUS) >= ground - 5.0);
    }

    /// `--open-map`: the map opens once its UI exists and its picture has arrived (not on
    /// the first frame regardless, which could leave a screenshot of an empty panel), the
    /// clock stops, and it stays open until `Esc` / `M`. Needs JC2_TEST_DIR with the map
    /// picture.
    #[test]
    fn open_at_start_opens_with_its_picture_and_stays_open() {
        use crate::fleet::testing;
        let Some((mut app, _, _)) = testing::app(&[MAP_TEXTURE], |app| {
            app.init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<AccumulatedMouseMotion>()
                .init_resource::<AccumulatedMouseScroll>()
                .insert_resource(OpenAtStart)
                .add_plugins(MapPlugin);
        }) else {
            return;
        };
        // the picture loads on another thread: wait for the map by the wall clock
        testing::wait_until(&mut app, "the map to open", |app| {
            app.world().resource::<MapState>().open
        });
        let mut pics = app
            .world_mut()
            .query_filtered::<&ImageNode, With<MapImage>>();
        let picture = pics.single(app.world()).unwrap().image.clone();
        let load = app
            .world()
            .resource::<AssetServer>()
            .get_load_state(&picture);
        assert!(
            !matches!(load, Some(LoadState::NotLoaded | LoadState::Loading)),
            "opened before the picture arrived: {load:?}"
        );
        assert!(app.world().get_resource::<OpenAtStart>().is_none());
        let visible = |app: &mut App| {
            let mut q = app
                .world_mut()
                .query_filtered::<&Visibility, With<MapRoot>>();
            *q.single(app.world()).unwrap()
        };
        assert_eq!(visible(&mut app), Visibility::Visible);
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        for _ in 0..120 {
            app.update();
        }
        assert!(app.world().resource::<MapState>().open, "closed by itself");
        // Esc closes it and the clock runs; M opens it again
        let tap = |app: &mut App, key: KeyCode| {
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(key);
            app.update();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .bypass_change_detection()
                .clear();
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .release(key);
            app.update();
        };
        tap(&mut app, KeyCode::Escape);
        assert!(!app.world().resource::<MapState>().open);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(visible(&mut app), Visibility::Hidden);
        tap(&mut app, KeyCode::KeyM);
        assert!(app.world().resource::<MapState>().open);
        assert_eq!(visible(&mut app), Visibility::Visible);
    }

    /// Headless: Rico on tile 31,14 jumps two tiles east. He is put at the target x/z,
    /// `DROP_HEIGHT` above its ground, with the clock stopped; the streamer follows him
    /// (the destination tile loads, the old one goes) and the clock runs again, and he
    /// falls. Needs JC2_TEST_DIR with the tiles.
    #[test]
    fn fast_travel_moves_rico_and_the_streaming() {
        let Some(mut app) = crate::world::tests::tile_app(|app| {
            app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1.0 / 64.0),
            ))
            .add_plugins(MapPlugin);
        }) else {
            return;
        };
        let spawn = app.world().resource::<SpawnPoint>().0;
        let hero = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::capsule(RADIUS, HEIGHT - 2.0 * RADIUS),
                LockedAxes::ROTATION_LOCKED,
                Transform::from_translation(spawn + Vec3::new(3.0, HEIGHT, 0.0)),
                Hero::default(),
            ))
            .id();
        app.update();
        assert_eq!(app.world().resource::<Streamer>().focus, (31, 14));
        let (x0, z0) = tile_origin(33, 14);
        let to = Vec2::new(x0 + 200.0, z0 + 300.0);
        app.world_mut().resource_mut::<MapState>().travel = Some(to);
        app.update();
        let ground = {
            let fs = &app.world().resource::<Game>().fs;
            let data = app.world().resource::<MapData>();
            ground_at(fs, data.global.as_ref(), to.x, to.y)
        };
        let p = app.world().get::<Transform>(hero).unwrap().translation;
        assert!(
            (p.x - to.x).abs() < 1e-3 && (p.z - to.y).abs() < 1e-3,
            "at {p:?}, wanted {to:?}"
        );
        assert!(
            (p.y - (ground + DROP_HEIGHT)).abs() < 0.5,
            "{} above the ground",
            p.y - ground
        );
        let pos = app.world().get::<Position>(hero).unwrap().0;
        assert!((pos - p).length() < 1e-3, "physics position {pos:?}");
        {
            let st = app.world().resource::<Streamer>();
            assert_eq!(st.focus, (33, 14));
            assert!(st.loaded.contains_key(&(33, 14)), "destination not loaded");
            assert!(
                !st.loaded.contains_key(&(31, 14)),
                "the old tile is still in"
            );
        }
        assert!(app.world().resource::<MapState>().arriving.is_none());
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        for _ in 0..64 {
            app.update();
        }
        let p2 = app.world().get::<Transform>(hero).unwrap().translation;
        assert!(
            p2.y < p.y - 2.0 && p2.y > ground + 150.0,
            "a second later he is at {} (ground {ground})",
            p2.y
        );
        assert!((p2.x - to.x).abs() < 0.5 && (p2.z - to.y).abs() < 0.5);
        eprintln!(
            "rico: dropped at {:.0} m over the ground of tile 33,14, a second later {:.0} m",
            p.y - ground,
            p2.y - ground
        );
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct MapInput;
