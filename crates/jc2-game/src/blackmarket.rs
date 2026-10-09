//! The Black Market (`K`): the game's shop, from `settings\blackmarket.bin`
//! (`jc2_formats::blackmarket`). Three categories — weapons, explosives, vehicles — each a
//! list of items with their names and descriptions from the text database, a price, the
//! chaos that unlocks them, stat bars and a picture per upgrade level (`bm_*.dds`).
//! Buying costs the price and hands the item over: a gun goes straight into Rico's
//! arsenal with a full reserve (the original drops it on a crate), grenades fill up, a
//! vehicle is put down beside him. Upgrading costs weapon (or vehicle) parts, the
//! `costs` table's 5, 10, 15, 20, 25 for the steps from level 0 to 5, and raises a gun's
//! upgrade level (`weapon_upgrades.bin`: damage, fire rate, magazine). The game stands
//! still while it is open, as with the map.
//!
//! Keys: `←`/`→` category, `↑`/`↓` item, `Enter` buy, `U` upgrade, `K` or `Esc` close.
//! `--black-market` opens it at the start (screenshots).

use crate::Game;
use crate::chaos::{Chaos, Wallet};
use crate::character::Hero;
use crate::combat::Arsenal;
use crate::fleet::{Fleet, Place, Request};
use crate::hud::{HudStrings, Notices, child, plain_text, r, rf, rim, white_text};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use jc2_formats::blackmarket::{BlackMarket, Category, MarketItem};
use jc2_formats::pickups::{PickupEntity, PickupKind};
use jc2_formats::sarc::SmallArchive;
use std::collections::{HashMap, HashSet};

pub const KEY: KeyCode = KeyCode::KeyK;
const CATEGORIES: [Category; 3] = [Category::Weapon, Category::Explosive, Category::Vehicle];
/// Where a bought vehicle is put down, from Rico (m, to his right and ahead).
const DROP_OFFSET: Vec3 = Vec3::new(6.0, 0.0, -4.0);

/// What an item gives.
#[derive(Clone, Debug, PartialEq)]
pub enum Goods {
    /// A gun, by its `weapons.bin` name (the heavy drop's `CWeapon`).
    Gun(String),
    /// Hand grenades (the heavy drop's `CItem` is `wea33`).
    Grenades,
    /// A vehicle container (`lave.v056_atv.ee`).
    Vehicle(String),
    /// Not in jc2-rs yet (the triggered charge).
    Unavailable,
}

/// The shop and its state.
#[derive(Resource, Default)]
pub struct Market {
    pub data: BlackMarket,
    /// What each item gives (same order as `data.items`).
    pub goods: Vec<Goods>,
    pub open: bool,
    pub category: usize,
    pub selected: usize,
    /// Vehicles bought, by container; their upgrade levels.
    pub vehicles: HashMap<String, usize>,
    /// The last thing that happened ("Order weapon", "Not enough money").
    pub message: String,
    /// Chaos thresholds already announced as unlocked.
    announced: HashSet<u32>,
    paused: bool,
    cursor: Option<(CursorGrabMode, bool)>,
    /// Bumped on every change, so the panel is rebuilt.
    generation: u32,
    built: u32,
}

/// `--black-market`: open the shop once everything is in.
#[derive(Resource)]
pub struct OpenMarketAtStart;

impl Market {
    /// The items of the current category, as indices into `data.items`.
    pub fn listed(&self) -> Vec<usize> {
        let c = CATEGORIES[self.category % CATEGORIES.len()];
        (0..self.data.items.len())
            .filter(|i| self.data.items[*i].category == c)
            .collect()
    }

    pub fn current(&self) -> Option<usize> {
        self.listed().get(self.selected).copied()
    }

    /// The index of an item by `item_id` (`WEA04`, `v056`).
    #[cfg(test)]
    pub fn find(&self, item_id: &str) -> Option<usize> {
        self.data
            .items
            .iter()
            .position(|i| i.item_id.eq_ignore_ascii_case(item_id))
    }

    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// The upgrade level of an item.
    pub fn level(&self, i: usize, arsenal: Option<&Arsenal>) -> usize {
        match &self.goods[i] {
            Goods::Gun(name) => arsenal.map_or(0, |a| a.level_of(name)),
            Goods::Vehicle(c) => self.vehicles.get(c).copied().unwrap_or(0),
            _ => 0,
        }
    }

    /// Rico has it.
    pub fn owned(&self, i: usize, arsenal: Option<&Arsenal>) -> bool {
        match &self.goods[i] {
            Goods::Gun(name) => {
                arsenal.is_some_and(|a| a.guns.iter().any(|g| &g.weapon.name == name))
            }
            Goods::Vehicle(c) => self.vehicles.contains_key(c),
            _ => false,
        }
    }

    /// Buy item `i`: the price comes off, the goods are handed over. `Err` says why not
    /// (a text key and its English).
    pub fn buy(
        &mut self,
        i: usize,
        chaos: u32,
        wallet: &mut Wallet,
        arsenal: Option<&mut Arsenal>,
        fleet: Option<&mut Fleet>,
        rico: Option<Transform>,
    ) -> Result<(), (&'static str, &'static str)> {
        let item = self
            .data
            .items
            .get(i)
            .ok_or(("gui_txt_24", "Not available"))?;
        if chaos < item.chaos {
            return Err(("gui_txt_141", "Chaos required"));
        }
        if wallet.money < item.price {
            return Err(("gui_txt_241", "Order Denied"));
        }
        match self.goods[i].clone() {
            Goods::Gun(name) => {
                let a = arsenal.ok_or(("gui_txt_24", "Not available"))?;
                // a full reserve (more than fits is cut to what he can carry)
                a.pick_up(None, &name, u32::MAX / 4)
                    .ok_or(("gui_txt_241", "Order Denied"))?;
            }
            Goods::Grenades => {
                let a = arsenal.ok_or(("gui_txt_24", "Not available"))?;
                if a.pick_up_grenades(a.max_grenades) == 0 {
                    return Err(("gui_txt_241", "Order Denied"));
                }
            }
            Goods::Vehicle(container) => {
                let (f, rico) = fleet.zip(rico).ok_or(("gui_txt_24", "Not available"))?;
                let at = rico.translation + rico.rotation * DROP_OFFSET;
                f.requests.push(Request {
                    container: container.clone(),
                    place: Place::At(Transform::from_translation(at).with_rotation(rico.rotation)),
                    tile: None,
                    drive: false,
                    crew: None,
                });
                self.vehicles.entry(container).or_insert(0);
            }
            Goods::Unavailable => return Err(("gui_txt_24", "Not available")),
        }
        wallet.money -= item.price;
        info!("black market: bought {} for ${}", item.item_id, item.price);
        self.touch();
        Ok(())
    }

    /// Upgrade item `i` one level: parts come off, the level goes up.
    pub fn upgrade(
        &mut self,
        i: usize,
        chaos: u32,
        wallet: &mut Wallet,
        arsenal: Option<&mut Arsenal>,
    ) -> Result<usize, (&'static str, &'static str)> {
        let item = self
            .data
            .items
            .get(i)
            .ok_or(("gui_txt_24", "Not available"))?;
        if chaos < item.chaos {
            return Err(("gui_txt_141", "Chaos required"));
        }
        let level = self.level(i, arsenal.as_deref());
        let cost = self
            .data
            .upgrade_cost(level)
            .ok_or(("gui_txt_240", "Upgrade Denied"))?;
        match self.goods[i].clone() {
            Goods::Gun(name) => {
                if wallet.weapon_parts < cost {
                    return Err((
                        "gui_txt_223",
                        "You don't have enough weapon parts to upgrade this weapon.",
                    ));
                }
                let a = arsenal.ok_or(("gui_txt_24", "Not available"))?;
                wallet.weapon_parts -= cost;
                a.set_level(&name, level + 1);
            }
            Goods::Vehicle(c) => {
                if wallet.vehicle_parts < cost {
                    return Err((
                        "gui_txt_224",
                        "You don't have enough vehicle parts to upgrade this vehicle.",
                    ));
                }
                wallet.vehicle_parts -= cost;
                self.vehicles.insert(c, level + 1);
            }
            _ => return Err(("gui_txt_240", "Upgrade Denied")),
        }
        self.touch();
        Ok(level + 1)
    }
}

pub struct BlackMarketPlugin;

impl Plugin for BlackMarketPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Market>()
            .add_systems(Startup, (read_market, build_panel).chain())
            .add_systems(
                PreUpdate,
                (open_close, shop)
                    .chain()
                    .after(bevy::input::InputSystems)
                    .in_set(BlackMarketInput),
            )
            .add_systems(Update, (announce_unlocks, show).chain());
    }
}

/// What a heavy drop holds.
fn heavy_drop(game: &Game, container: &str) -> Option<PickupKind> {
    let data = game.fs.read_inflated(container).ok()?;
    let sarc = SmallArchive::parse(&data).ok()?;
    let e = sarc
        .entries
        .iter()
        .find(|e| e.name.to_ascii_lowercase().ends_with(".epe"))?;
    Some(
        PickupEntity::parse(sarc.data(&data, e).ok()?)
            .ok()??
            .item
            .kind,
    )
}

fn goods_of(game: &Game, item: &MarketItem) -> Goods {
    if item.category == Category::Vehicle {
        return Goods::Vehicle(item.container());
    }
    match heavy_drop(game, &item.container()) {
        // the explosives' heavy drops are `CWeapon`s too: "Handgrenade" (20) and
        // "Triggered Explosive", which jc2-rs does not have yet
        Some(PickupKind::Weapon(w)) if item.category == Category::Explosive => {
            if w.to_ascii_lowercase().contains("grenade") {
                Goods::Grenades
            } else {
                Goods::Unavailable
            }
        }
        Some(PickupKind::Weapon(w)) => Goods::Gun(w),
        Some(PickupKind::Item(lod)) if lod.to_ascii_lowercase().contains("wea33") => {
            Goods::Grenades
        }
        _ => Goods::Unavailable,
    }
}

fn read_market(mut market: ResMut<Market>, game: Option<Res<Game>>) {
    let Some(game) = game else { return };
    let Some(data) = game
        .fs
        .read_inflated("blackmarket.bin")
        .ok()
        .and_then(|d| BlackMarket::parse(&d).ok())
    else {
        warn!("blackmarket.bin not read: the Black Market is empty");
        return;
    };
    market.goods = data.items.iter().map(|i| goods_of(&game, i)).collect();
    info!(
        "black market: {} items ({} guns, {} vehicles), upgrades cost {:?} parts",
        data.items.len(),
        market
            .goods
            .iter()
            .filter(|g| matches!(g, Goods::Gun(_)))
            .count(),
        market
            .goods
            .iter()
            .filter(|g| matches!(g, Goods::Vehicle(_)))
            .count(),
        data.costs
    );
    market.data = data;
    market.touch();
}

/// "New Black Market Items Unlocked" as the chaos passes their thresholds.
fn announce_unlocks(
    chaos: Option<Res<Chaos>>,
    mut market: ResMut<Market>,
    strings: Option<Res<HudStrings>>,
    notices: Option<ResMut<Notices>>,
) {
    let Some(chaos) = chaos else { return };
    let fresh: Vec<u32> = market
        .data
        .items
        .iter()
        .map(|i| i.chaos)
        .filter(|c| *c > 0 && *c <= chaos.total && !market.announced.contains(c))
        .collect();
    if fresh.is_empty() {
        return;
    }
    let start = market.announced.is_empty() && chaos.total > 0 && chaos.is_added();
    market.announced.extend(fresh.iter().copied());
    if start {
        return;
    }
    let names: Vec<String> = market
        .data
        .items
        .iter()
        .filter(|i| fresh.contains(&i.chaos))
        .map(|i| {
            strings
                .as_ref()
                .map_or(i.name.clone(), |s| s.get(&i.name, &i.name))
        })
        .collect();
    let head = strings
        .as_ref()
        .map_or("New Black Market Items Unlocked".to_string(), |s| {
            s.get("gui_txt_249", "New Black Market Items Unlocked")
        });
    if let Some(mut n) = notices {
        n.push(format!("{head}: {}", names.join(", ")));
    }
    market.touch();
}

#[allow(clippy::too_many_arguments)]
fn open_close(
    mut commands: Commands,
    keys: Option<ResMut<ButtonInput<KeyCode>>>,
    mut market: ResMut<Market>,
    at_start: Option<Res<OpenMarketAtStart>>,
    map: Option<Res<crate::map::MapState>>,
    mut time: ResMut<Time<Virtual>>,
    mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>,
    heroes: Query<(), With<Hero>>,
) {
    let Some(keys) = keys else { return };
    let map_open = map.is_some_and(|m| m.open);
    let toggle = keys.just_pressed(KEY) || (market.open && keys.just_pressed(KeyCode::Escape));
    let start = at_start.is_some() && !market.data.items.is_empty() && !heroes.is_empty();
    if !(toggle || start) || (map_open && !market.open) {
        return;
    }
    if start {
        commands.remove_resource::<OpenMarketAtStart>();
        if market.open {
            return;
        }
    }
    market.open = !market.open;
    market.message.clear();
    let mut cursor = cursors.single_mut().ok();
    if market.open {
        if !time.is_paused() {
            time.pause();
            market.paused = true;
        }
        if let Some(c) = cursor.as_mut() {
            market.cursor = Some((c.grab_mode, c.visible));
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
    } else {
        if market.paused {
            time.unpause();
            market.paused = false;
        }
        if let (Some(c), Some((grab, visible))) = (cursor.as_mut(), market.cursor.take()) {
            c.grab_mode = grab;
            c.visible = visible;
        }
    }
    market.touch();
}

/// The keys while the shop is open; the rest of the game does not see them.
#[allow(clippy::too_many_arguments)]
fn shop(
    keys: Option<ResMut<ButtonInput<KeyCode>>>,
    buttons: Option<ResMut<ButtonInput<MouseButton>>>,
    mut market: ResMut<Market>,
    chaos: Option<Res<Chaos>>,
    wallet: Option<ResMut<Wallet>>,
    arsenal: Option<ResMut<Arsenal>>,
    fleet: Option<ResMut<Fleet>>,
    strings: Option<Res<HudStrings>>,
    heroes: Query<&Transform, With<Hero>>,
) {
    if !market.open {
        return;
    }
    let (Some(mut keys), Some(mut wallet)) = (keys, wallet) else {
        return;
    };
    let chaos = chaos.map_or(0, |c| c.total);
    let n = market.listed().len().max(1);
    let mut moved = false;
    if keys.just_pressed(KeyCode::ArrowRight) {
        market.category = (market.category + 1) % CATEGORIES.len();
        market.selected = 0;
        moved = true;
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        market.category = (market.category + CATEGORIES.len() - 1) % CATEGORIES.len();
        market.selected = 0;
        moved = true;
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        market.selected = (market.selected + 1) % n;
        moved = true;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        market.selected = (market.selected + n - 1) % n;
        moved = true;
    }
    if moved {
        market.message.clear();
        market.touch();
    }
    let say = |k: &str, en: &str| strings.as_ref().map_or(en.to_string(), |s| s.get(k, en));
    let mut arsenal = arsenal;
    let mut fleet = fleet;
    if keys.just_pressed(KeyCode::Enter)
        && let Some(i) = market.current()
    {
        let rico = heroes.single().ok().copied();
        let result = market.buy(
            i,
            chaos,
            &mut wallet,
            arsenal.as_deref_mut(),
            fleet.as_deref_mut(),
            rico,
        );
        market.message = match result {
            Ok(()) => match market.goods[i] {
                Goods::Vehicle(_) => say("gui_txt_238", "Order vehicle"),
                _ => say("gui_txt_235", "Order weapon"),
            },
            Err((k, en)) => say(k, en),
        };
        market.touch();
    }
    if keys.just_pressed(KeyCode::KeyU)
        && let Some(i) = market.current()
    {
        let result = market.upgrade(i, chaos, &mut wallet, arsenal.as_deref_mut());
        market.message = match result {
            Ok(l) => format!("{} {l}", say("gui_txt_178", "Level")),
            Err((k, en)) => say(k, en),
        };
        market.touch();
    }
    // the game does not see what the shop used
    keys.reset_all();
    if let Some(mut b) = buttons {
        b.reset_all();
    }
}

#[derive(Component)]
struct PanelRoot;

#[derive(Component)]
struct PanelBody;

fn build_panel(mut commands: Commands) {
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            GlobalZIndex(100),
            PanelRoot,
            Name::new("black market"),
        ))
        .id();
    child(
        &mut commands,
        root,
        (
            Node {
                width: r(1560.0),
                height: r(900.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(r(22.0)),
                row_gap: r(14.0),
                border: UiRect::all(r(2.0)),
                border_radius: BorderRadius::all(r(26.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.08, 0.1, 0.94)),
            rim(),
            PanelBody,
        ),
    );
}

/// Rebuild the panel when anything changed.
#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    mut market: ResMut<Market>,
    chaos: Option<Res<Chaos>>,
    wallet: Option<Res<Wallet>>,
    arsenal: Option<Res<Arsenal>>,
    strings: Option<Res<HudStrings>>,
    assets: Res<AssetServer>,
    game: Option<Res<Game>>,
    mut roots: Query<&mut Node, With<PanelRoot>>,
    bodies: Query<(Entity, Option<&Children>), With<PanelBody>>,
) {
    for mut n in &mut roots {
        let d = if market.open {
            Display::Flex
        } else {
            Display::None
        };
        if n.display != d {
            n.display = d;
        }
    }
    let changed = wallet.as_ref().is_some_and(|w| w.is_changed())
        || chaos.as_ref().is_some_and(|c| c.is_changed());
    if !market.open || (market.built == market.generation && !changed) {
        return;
    }
    market.built = market.generation;
    let Ok((body, kids)) = bodies.single() else {
        return;
    };
    for k in kids.into_iter().flatten() {
        commands.entity(*k).despawn();
    }
    let say = |k: &str| strings.as_ref().map_or(k.to_string(), |s| s.get(k, k));
    let say_or = |k: &str, en: &str| strings.as_ref().map_or(en.to_string(), |s| s.get(k, en));
    let chaos_total = chaos.as_ref().map_or(0, |c| c.total);
    let w = wallet.as_deref().cloned().unwrap_or_default();
    let a = arsenal.as_deref();
    let accent = Color::srgb(0.95, 0.55, 0.15);

    // the title and the resources
    let head = child(
        &mut commands,
        body,
        Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        },
    );
    child(
        &mut commands,
        head,
        (
            Text::new(say_or("gui_txt_139", "Black Market")),
            white_text(44.0),
        ),
    );
    child(
        &mut commands,
        head,
        (
            Text::new(format!(
                "{} ${}    {} {}    {} {}    {} {}",
                say_or("gui_txt_08", "Money"),
                w.money,
                say_or("gui_txt_83", "Weapon parts"),
                w.weapon_parts,
                say_or("gui_txt_09", "Vehicle parts"),
                w.vehicle_parts,
                say_or("gui_txt_07", "Chaos"),
                chaos_total
            )),
            plain_text(24.0),
        ),
    );
    // the categories
    let tabs = child(
        &mut commands,
        body,
        Node {
            column_gap: r(12.0),
            ..default()
        },
    );
    let tab_names = [
        "Weapons".to_string(),
        say_or("gui_txt_03", "Explosives"),
        say_or("gui_txt_17", "Vehicles"),
    ];
    for (i, t) in tab_names.iter().enumerate() {
        let sel = i == market.category;
        let tab = child(
            &mut commands,
            tabs,
            (
                Node {
                    padding: UiRect::axes(r(26.0), r(8.0)),
                    border: UiRect::all(r(1.5)),
                    border_radius: BorderRadius::all(r(14.0)),
                    ..default()
                },
                BackgroundColor(if sel {
                    accent.with_alpha(0.85)
                } else {
                    Color::srgba(0.2, 0.25, 0.31, 0.7)
                }),
                rim(),
            ),
        );
        child(&mut commands, tab, (Text::new(t.clone()), plain_text(26.0)));
    }
    // the list and the details
    let main = child(
        &mut commands,
        body,
        Node {
            flex_grow: 1.0,
            column_gap: r(24.0),
            ..default()
        },
    );
    let list = child(
        &mut commands,
        main,
        Node {
            width: r(640.0),
            flex_direction: FlexDirection::Column,
            row_gap: r(6.0),
            ..default()
        },
    );
    let listed = market.listed();
    let max_level = market.data.max_level().max(1);
    for (row_i, &i) in listed.iter().enumerate() {
        let item = &market.data.items[i];
        let sel = row_i == market.selected;
        let locked = chaos_total < item.chaos;
        let row = child(
            &mut commands,
            list,
            (
                Node {
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(r(16.0), r(7.0)),
                    border_radius: BorderRadius::all(r(10.0)),
                    ..default()
                },
                BackgroundColor(if sel {
                    accent.with_alpha(0.55)
                } else {
                    Color::srgba(0.12, 0.15, 0.19, 0.75)
                }),
            ),
        );
        let (font, color) = plain_text(25.0);
        child(
            &mut commands,
            row,
            (
                Text::new(say(&item.name)),
                font,
                TextColor(if locked {
                    Color::srgba(1.0, 1.0, 1.0, 0.45)
                } else {
                    color.0
                }),
            ),
        );
        let level = market.level(i, a);
        let status = if market.owned(i, a) {
            format!("Owned  L{level}/{max_level}")
        } else if locked {
            format!("{} {}", say_or("gui_txt_229", "Locked"), item.chaos)
        } else {
            format!("${}", item.price)
        };
        child(&mut commands, row, (Text::new(status), plain_text(22.0)));
    }
    if let Some(i) = market.current() {
        let item = &market.data.items[i];
        let level = market.level(i, a);
        let detail = child(
            &mut commands,
            main,
            Node {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                row_gap: r(10.0),
                ..default()
            },
        );
        child(
            &mut commands,
            detail,
            (Text::new(say(&item.name)), white_text(36.0)),
        );
        let picture = item
            .images
            .get(level.min(item.images.len().saturating_sub(1)))
            .filter(|p| game.as_ref().is_some_and(|g| g.fs.contains(p.as_str())));
        if let Some(p) = picture {
            child(
                &mut commands,
                detail,
                (
                    Node {
                        width: r(512.0),
                        height: r(256.0),
                        ..default()
                    },
                    ImageNode::new(assets.load(format!("jc2://{}", p.to_ascii_lowercase()))),
                ),
            );
        }
        child(
            &mut commands,
            detail,
            (
                Text::new(say(&item.description)),
                plain_text(20.0),
                Node {
                    max_width: r(820.0),
                    ..default()
                },
            ),
        );
        // the stat bars: the range of each, filled to where this level stands
        let captions = market.data.captions(item.category).to_vec();
        for (k, cap) in captions.iter().enumerate() {
            let Some(Some([lo, hi])) = item.stats.get(k) else {
                continue;
            };
            let t = level as f32 / max_level as f32;
            let v = lo + (hi - lo) * t;
            let stat = child(
                &mut commands,
                detail,
                Node {
                    align_items: AlignItems::Center,
                    column_gap: r(14.0),
                    ..default()
                },
            );
            child(
                &mut commands,
                stat,
                (
                    Text::new(say(cap)),
                    plain_text(20.0),
                    Node {
                        width: r(240.0),
                        ..default()
                    },
                ),
            );
            let bar = child(
                &mut commands,
                stat,
                (
                    Node {
                        width: r(420.0),
                        height: r(14.0),
                        border_radius: BorderRadius::all(r(7.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.3, 0.35, 0.4, 0.6)),
                ),
            );
            child(
                &mut commands,
                bar,
                (
                    Node {
                        width: Val::Percent(v.clamp(0.0, 100.0)),
                        height: Val::Percent(100.0),
                        border_radius: BorderRadius::all(r(7.0)),
                        ..default()
                    },
                    BackgroundColor(accent),
                ),
            );
        }
        let mut lines = vec![format!("{} ${}", say_or("gui_txt_153", "Cost"), item.price)];
        if chaos_total < item.chaos {
            lines.push(format!(
                "{} {}",
                say_or("gui_txt_141", "Chaos required"),
                item.chaos
            ));
        }
        if matches!(market.goods[i], Goods::Gun(_) | Goods::Vehicle(_)) {
            lines.push(match market.data.upgrade_cost(level) {
                Some(c) => format!(
                    "{}: {c} {}   ({} {level}/{max_level})",
                    say_or("gui_txt_19", "Upgrade cost"),
                    if matches!(market.goods[i], Goods::Gun(_)) {
                        say_or("gui_txt_83", "Weapon parts")
                    } else {
                        say_or("gui_txt_09", "Vehicle parts")
                    },
                    say_or("gui_txt_178", "Level"),
                ),
                None => format!("{} {level}/{max_level}", say_or("gui_txt_178", "Level")),
            });
        }
        if market.goods[i] == Goods::Unavailable {
            lines.push(say_or("gui_txt_24", "Not available"));
        }
        child(
            &mut commands,
            detail,
            (Text::new(lines.join("\n")), plain_text(24.0)),
        );
        if !market.message.is_empty() {
            child(
                &mut commands,
                detail,
                (
                    Text::new(market.message.clone()),
                    TextFont {
                        font_size: rf(26.0),
                        ..default()
                    },
                    TextColor(accent),
                ),
            );
        }
    }
    child(
        &mut commands,
        body,
        (
            Text::new(format!(
                "Enter {}    U {}    Left/Right category    Up/Down item    K {}",
                say_or("gui_txt_140", "Buy"),
                say_or("gui_txt_234", "Upgrade weapon"),
                say_or("gui_txt_146", "Close"),
            )),
            plain_text(21.0),
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::combat::CombatPlugin;

    /// Headless, with the game's tables: at enough chaos and money, buying the revolver
    /// takes its price and puts it in Rico's arsenal; the grenade launcher is locked
    /// below its chaos; an upgrade of the pistol spends five weapon parts and raises its
    /// level (magazine 12 → 14, damage 16 → 18, as `weapon_upgrades.bin` says). Needs
    /// JC2_TEST_DIR.
    #[test]
    fn buying_and_upgrading() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = jc2_formats::GameFs::open(dir).unwrap();
        for f in [
            "blackmarket.bin",
            "weapons.bin",
            "weapon_upgrades.bin",
            "ammunition.bin",
            "mod.heavydrop.revolver.ee",
        ] {
            if !fs.contains(f) {
                return;
            }
        }
        let Some((mut app, _, _)) = crate::fleet::testing::app(&[], |app| {
            app.init_resource::<ButtonInput<MouseButton>>()
                .add_plugins((CombatPlugin, crate::chaos::ChaosPlugin, BlackMarketPlugin));
        }) else {
            return;
        };
        app.update();
        let rev = app.world().resource::<Market>().find("WEA04").unwrap();
        let gl = app.world().resource::<Market>().find("WEA17").unwrap();
        let pistol = app.world().resource::<Market>().find("WEA02").unwrap();
        assert_eq!(
            app.world().resource::<Market>().goods[rev],
            Goods::Gun("Revolver".into())
        );
        let grenades = app.world().resource::<Market>().find("WEA33").unwrap();
        assert_eq!(
            app.world().resource::<Market>().goods[grenades],
            Goods::Grenades
        );
        app.world_mut().resource_mut::<Chaos>().add(50_000);
        app.world_mut().resource_mut::<Wallet>().money = 100_000;
        app.world_mut()
            .resource_scope(|w, mut market: Mut<Market>| {
                w.resource_scope(|w, mut wallet: Mut<Wallet>| {
                    let mut arsenal = w.resource_mut::<Arsenal>();
                    assert!(!arsenal.guns.iter().any(|g| g.weapon.name == "Revolver"));
                    market
                        .buy(rev, 50_000, &mut wallet, Some(&mut arsenal), None, None)
                        .unwrap();
                    assert_eq!(wallet.money, 60_000, "the revolver's $40 000");
                    let g = arsenal
                        .guns
                        .iter()
                        .find(|g| g.weapon.name == "Revolver")
                        .expect("the revolver is his");
                    assert_eq!(g.ammo.reserve, g.ammo.max_reserve, "a full reserve");
                    // the grenade launcher needs 360 000 chaos
                    assert_eq!(
                        market.buy(gl, 50_000, &mut wallet, Some(&mut arsenal), None, None),
                        Err(("gui_txt_141", "Chaos required"))
                    );
                    assert_eq!(wallet.money, 60_000);
                    // the pistol: level 0 → 1 for five weapon parts
                    let beretta = arsenal
                        .guns
                        .iter()
                        .find(|g| g.weapon.name == "Beretta Handgun")
                        .unwrap();
                    assert_eq!(
                        (beretta.level, beretta.ammo.magazine_size, beretta.damage),
                        (0, 12, 16.0)
                    );
                    assert!(
                        market
                            .upgrade(pistol, 50_000, &mut wallet, Some(&mut arsenal))
                            .is_err()
                    );
                    wallet.weapon_parts = 7;
                    assert_eq!(
                        market.upgrade(pistol, 50_000, &mut wallet, Some(&mut arsenal)),
                        Ok(1)
                    );
                    assert_eq!(wallet.weapon_parts, 2);
                    let up = arsenal.upgrades().get("Beretta Handgun", 1).unwrap();
                    let beretta = arsenal
                        .guns
                        .iter()
                        .find(|g| g.weapon.name == "Beretta Handgun")
                        .unwrap();
                    assert_eq!(beretta.level, 1);
                    assert_eq!(beretta.ammo.magazine_size, up.magazine);
                    assert_eq!(beretta.damage, up.damage);
                    assert_eq!(beretta.interval, up.fire_interval);
                    assert!(up.magazine > 12 && up.damage > 16.0, "{up:?}");
                    assert_eq!(market.level(pistol, Some(&arsenal)), 1);
                })
            });
    }
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BlackMarketInput;
