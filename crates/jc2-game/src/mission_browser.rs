//! Mission inspection, separate waypoint/travel/start actions and bounded giver access.
//! Direct PDA launch and intro skip are explicit jc2-rs policy, not retail parity.
use crate::chaos::Chaos;
use crate::character::Hero;
use crate::missions::access::{
    Availability, LaunchOrigin, LaunchStatus, MissionLaunchRequest, MissionLaunches, availability,
};
use crate::missions::{
    ActiveMission, EventBus, MissionDb, MissionOwner, MissionsDone, Strongholds,
};
use crate::{Game, map::MapState};
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::ui::UiSystems;
use jc2_formats::placement::Transform3;
use jc2_formats::property::{Node as PropertyNode, PropertyFile, Variant};
use std::collections::HashMap;

#[derive(Clone, Debug)]
#[allow(dead_code)] // Diagnostic data retains unconfirmed retail router semantics.
pub struct GiverRouter {
    pub code: i32,
    pub target: String,
    pub raw: PropertyNode,
}
#[derive(Clone, Debug)]
pub struct GiverDefinition {
    pub mission: String,
    pub employer: String,
    pub container: String,
    pub interaction: Transform,
    pub box_size: Vec3,
    pub prompt: String,
    #[allow(dead_code)] // Original actions retained for diagnostics and parity work.
    pub action_events: Vec<String>,
    pub completion_event: Option<String>,
    #[allow(dead_code)]
    pub routers: Vec<GiverRouter>,
}
impl GiverDefinition {
    /// Full box size and local transforms come from data. This OBB test is project
    /// policy until the original smart-object overlap consumer has been traced.
    pub fn contains(&self, position: Vec3) -> bool {
        let local = self
            .interaction
            .compute_affine()
            .inverse()
            .transform_point3(position);
        local.is_finite() && local.abs().cmple(self.box_size * 0.5).all()
    }
    fn parse(db: &MissionDb, mission: &str, container: &str, file: &PropertyFile) -> Option<Self> {
        let giver = db.catalog.get(mission)?.giver.as_ref()?;
        let expected = format!("msy.{}.complete", giver.id.to_ascii_lowercase());
        let mut smart = None;
        let mut routers = Vec::new();
        let mut cutscenes = Vec::new();
        fn string(n: &PropertyNode, h: u32) -> String {
            match n.value_hash(h) {
                Some(Variant::Str(s)) => s.clone(),
                _ => String::new(),
            }
        }
        fn walk(
            n: &PropertyNode,
            parent: &Transform3,
            smart: &mut Option<(Transform3, PropertyNode)>,
            routers: &mut Vec<GiverRouter>,
            cutscenes: &mut Vec<PropertyNode>,
        ) {
            let local = match n.value("world") {
                Some(Variant::Mat4x3(m)) => Some(Transform3::from_mat4x3(*m)),
                Some(Variant::Mat4x4(m)) => Some(Transform3::from_mat4x4(*m)),
                _ => None,
            };
            let world = local.map_or(*parent, |l| l.then(parent));
            match n.str("_class") {
                Some("CSmartObject") if n.str("name") == Some("sob.talkToEmp") => {
                    *smart = Some((world, n.clone()))
                }
                Some("CMissionStatusRouter") => routers.push(GiverRouter {
                    code: match n.value_hash(0x1d00a9cd) {
                        Some(Variant::Int(i)) => *i,
                        _ => -1,
                    },
                    target: string(n, 0x8581b032),
                    raw: n.clone(),
                }),
                Some("CCutsceneObject") => cutscenes.push(n.clone()),
                _ => {}
            }
            for (_, child) in &n.children {
                walk(child, &world, smart, routers, cutscenes);
            }
        }
        for root in &file.nodes {
            walk(
                root,
                &Transform3::IDENTITY,
                &mut smart,
                &mut routers,
                &mut cutscenes,
            );
        }
        let (world, smart) = smart?;
        let mut trigger = None;
        let mut action = None;
        smart.walk(&mut |n| match n.str("_class") {
            Some("CSmartObjectTrigger") => trigger = Some(n.clone()),
            Some("CSmartObjectAction") => action = Some(n.clone()),
            _ => {}
        });
        let trigger = trigger?;
        let action = action?;
        let Some(Variant::Vec3(size)) = trigger.value("box_size") else {
            return None;
        };
        if size.iter().any(|v| !v.is_finite() || *v <= 0.) {
            return None;
        }
        let local = match trigger.value("local") {
            Some(Variant::Mat4x3(m)) => Transform3::from_mat4x3(*m),
            _ => Transform3::IDENTITY,
        };
        let world = local.then(&world);
        let events: Vec<_> = jc2_formats::missions::events(&string(&action, 0xd6d617cc)).collect();
        let completion_event = events
            .iter()
            .find(|e| **e == expected)
            .cloned()
            .or_else(|| {
                cutscenes.iter().find_map(|c| {
                    let play = string(c, 0x1a518747).to_ascii_lowercase();
                    let done = string(c, 0x9ded931f).to_ascii_lowercase();
                    (events.contains(&play) && done == expected).then_some(done)
                })
            });
        let key = string(&action, 0xf6d62ea4);
        Some(Self {
            mission: mission.to_string(),
            employer: giver.id.clone(),
            container: container.to_string(),
            interaction: crate::coords::mat_to_bevy(&world),
            box_size: Vec3::from(*size),
            prompt: db.text(&key, &key),
            action_events: events,
            completion_event,
            routers,
        })
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EmployerPhase {
    #[default]
    Available,
    Preparing(MissionOwner),
    Active(MissionOwner),
}
#[derive(Default, Debug)]
pub struct EmployerState {
    pub phase: EmployerPhase,
    pub introduction_completed: bool,
}
#[derive(Resource, Default)]
pub struct GiverAccess {
    pub definitions: HashMap<String, GiverDefinition>,
    pub employers: HashMap<String, EmployerState>,
    tickets: HashMap<String, GiverTicket>,
    next_owner: u64,
    pub near: Option<String>,
}
struct GiverTicket {
    owner: crate::locations::LocationOwner,
    container: String,
}
fn giver_ticket(
    access: &mut GiverAccess,
    locations: &mut crate::locations::Locations,
    id: &str,
    container: &str,
    anchor: Option<(u32, u32)>,
) -> Option<u64> {
    let Some(anchor) = anchor else {
        if let Some(ticket) = access.tickets.remove(id) {
            locations.release(ticket.owner);
        }
        return None;
    };
    if access
        .tickets
        .get(id)
        .is_some_and(|t| t.container != container)
    {
        let ticket = access.tickets.remove(id).unwrap();
        locations.release(ticket.owner);
    }
    let owner = if let Some(ticket) = access.tickets.get(id) {
        ticket.owner
    } else {
        access.next_owner = access
            .next_owner
            .checked_add(1)
            .expect("giver owner exhausted");
        crate::locations::LocationOwner::Giver(access.next_owner)
    };
    let generation = locations.request(owner, container, anchor);
    access.tickets.insert(
        id.into(),
        GiverTicket {
            owner,
            container: container.into(),
        },
    );
    Some(generation)
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BrowserView {
    #[default]
    Map,
    Missions,
    Giver,
}
#[derive(Resource, Debug, Default)]
pub struct MissionBrowser {
    pub view: BrowserView,
    pub selected: Option<String>,
    pub page: usize,
    pub message: String,
    pub open_requested: bool,
    pub close_requested: bool,
    pub giver_origin: Option<String>,
}
#[derive(Resource)]
pub struct OpenMissionsAtStart(pub String);
#[derive(Clone, Debug, PartialEq)]
pub enum BrowserAction {
    Inspect(String),
    Waypoint,
    ClearWaypoint,
    Travel,
    Start,
    Cancel,
    Next,
    Previous,
    Map,
    Missions,
}
#[derive(Component)]
struct Action(BrowserAction);
#[derive(Component)]
struct BrowserRoot;
#[derive(Component)]
struct BrowserPanel;
#[derive(Component)]
struct Details;
#[derive(Component)]
struct MissionRows;
#[derive(Component)]
struct PageLabel;
#[derive(Resource, Default)]
struct UiSignature(String);

#[allow(clippy::too_many_arguments)]
pub fn state_for(
    db: &MissionDb,
    id: &str,
    origin: LaunchOrigin,
    chaos: u32,
    done: &MissionsDone,
    holds: &Strongholds,
    active: &ActiveMission,
    launches: &MissionLaunches,
) -> Availability {
    availability(
        db,
        &MissionLaunchRequest {
            id: id.into(),
            origin,
            debug_override: false,
        },
        chaos,
        done,
        holds,
        active.0.as_ref().map(|r| r.id.as_str()),
        launches.preparation().map(|p| p.id.as_str()),
    )
}
fn label(db: &MissionDb, state: &Availability) -> String {
    match state {
        Availability::Available => "Available".into(),
        Availability::Completed => "Completed".into(),
        Availability::Prerequisite(id) => format!(
            "Complete {} first",
            db.catalog.get(id).map_or(id.as_str(), |m| m.name.as_str())
        ),
        Availability::Chaos { required, current } => format!("Chaos: {current} / {required}"),
        Availability::ActiveMission(_) => "A mission is running".into(),
        Availability::Preparing(_) => "Preparing a mission".into(),
        Availability::ResourcesFailed(_) | Availability::LoadFailed(_) => {
            "Mission resources could not be loaded".into()
        }
        _ => "Not available".into(),
    }
}
/// Selection and navigation never implicitly request a launch. Every action is
/// addressed by stable mission ID, never by a filtered native icon ordinal.
#[allow(clippy::too_many_arguments)]
pub fn act(
    action: BrowserAction,
    browser: &mut MissionBrowser,
    map: &mut MapState,
    db: &MissionDb,
    chaos: u32,
    done: &MissionsDone,
    holds: &Strongholds,
    active: &ActiveMission,
    launches: &mut MissionLaunches,
) {
    match action {
        BrowserAction::Inspect(id) => {
            if db.catalog.get(&id).is_some() {
                if browser.view == BrowserView::Giver
                    && browser
                        .selected
                        .as_ref()
                        .is_none_or(|old| !old.eq_ignore_ascii_case(&id))
                {
                    browser.view = BrowserView::Missions;
                    browser.giver_origin = None;
                }
                browser.selected = Some(id.to_ascii_lowercase());
                browser.message.clear();
            }
        }
        BrowserAction::Next => {
            browser.page = (browser.page + 1).min(db.catalog.0.len().saturating_sub(1) / 8);
        }
        BrowserAction::Previous => browser.page = browser.page.saturating_sub(1),
        BrowserAction::Map => {
            browser.view = BrowserView::Map;
            browser.giver_origin = None;
        }
        BrowserAction::Missions => {
            browser.view = BrowserView::Missions;
            browser.giver_origin = None;
        }
        BrowserAction::Cancel => {
            let owner = launches
                .preparation()
                .map(|p| p.owner)
                .or_else(|| active.0.as_ref().map(|r| r.owner));
            if let Some(owner) = owner {
                launches.cancel(owner);
                browser.message = "Mission cancellation requested".into();
            }
        }
        _ => {
            let Some(entry) = browser.selected.as_ref().and_then(|id| db.catalog.get(id)) else {
                return;
            };
            let origin = browser
                .giver_origin
                .as_ref()
                .map_or(LaunchOrigin::Pda, |id| LaunchOrigin::Giver {
                    employer: id.clone(),
                });
            match action {
                BrowserAction::Waypoint => {
                    if let Some(g) = &entry.giver {
                        map.waypoint = Some(Vec3::from(g.position));
                        browser.message = "Waypoint placed at the mission giver".into();
                    }
                }
                BrowserAction::ClearWaypoint => {
                    map.waypoint = None;
                    browser.message = "Waypoint removed".into();
                }
                BrowserAction::Travel => {
                    if active.0.is_some() || launches.preparation().is_some() {
                        browser.message = "Cancel the current mission before travelling".into();
                        return;
                    }
                    if let Some(g) = &entry.giver {
                        map.travel = Some(Vec2::new(g.position[0], g.position[2]));
                        browser.close_requested = true;
                    }
                }
                BrowserAction::Start => {
                    let state = state_for(
                        db,
                        &entry.record.id,
                        origin.clone(),
                        chaos,
                        done,
                        holds,
                        active,
                        launches,
                    );
                    if state != Availability::Available {
                        browser.message = label(db, &state);
                        return;
                    }
                    if !crate::missions::access::normal_launch_supported(&entry.record.id) {
                        browser.message = "This mission cannot be started in this build".into();
                        return;
                    }
                    launches.request(MissionLaunchRequest {
                        id: entry.record.id.clone(),
                        origin,
                        debug_override: false,
                    });
                    browser.message = "Preparing mission".into();
                    browser.close_requested = true;
                }
                _ => {}
            }
        }
    }
}

pub struct MissionBrowserPlugin;
impl Plugin for MissionBrowserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MissionBrowser>()
            .init_resource::<GiverAccess>()
            .init_resource::<UiSignature>()
            .add_systems(PostStartup, (load_givers, build_ui).chain())
            .add_systems(
                PreUpdate,
                input
                    .after(InputSystems)
                    .after(UiSystems::Focus)
                    .after(crate::blackmarket::BlackMarketInput)
                    .before(crate::map::MapInput),
            )
            .add_systems(
                Update,
                refresh_givers
                    .after(crate::world::stream_tiles)
                    .after(crate::missions::access::service_launches)
                    .before(crate::locations::LocationReconciled),
            )
            .add_systems(
                Update,
                sync_employers.after(crate::missions::access::service_launches),
            )
            .add_systems(
                Update,
                prompts
                    .after(crate::hud::PromptComputed)
                    .before(crate::hud::HudApplied),
            )
            .add_systems(
                Update,
                (refresh_ui, action_visibility)
                    .chain()
                    .after(sync_employers),
            );
    }
}
fn load_givers(game: Res<Game>, db: Res<MissionDb>, mut access: ResMut<GiverAccess>) {
    for id in ["f2t10", "km03"] {
        let Some(container) = db
            .catalog
            .get(id)
            .and_then(|m| m.giver.as_ref())
            .and_then(|g| g.container.as_ref())
        else {
            continue;
        };
        let Some(file) = game
            .fs
            .read_inflated(container)
            .ok()
            .and_then(|b| jc2_formats::missions::first_property_member(&b, &[".blo"]))
        else {
            continue;
        };
        if let Some(def) = GiverDefinition::parse(&db, id, container, &file) {
            access.definitions.insert(id.into(), def);
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn refresh_givers(
    mut access: ResMut<GiverAccess>,
    db: Res<MissionDb>,
    chaos: Option<Res<Chaos>>,
    done: Res<MissionsDone>,
    holds: Res<Strongholds>,
    active: Res<ActiveMission>,
    launches: Res<MissionLaunches>,
    st: Option<Res<crate::world::Streamer>>,
    locations: Option<ResMut<crate::locations::Locations>>,
    heroes: Query<(&Hero, &Transform)>,
    mode: Option<Res<crate::vehicle::CameraMode>>,
) {
    access.near = None;
    let (Some(st), Some(mut locations)) = (st, locations) else {
        return;
    };
    let definitions: Vec<_> = access.definitions.values().cloned().collect();
    for def in definitions {
        let state = state_for(
            &db,
            &def.mission,
            LaunchOrigin::Giver {
                employer: def.employer.clone(),
            },
            chaos.as_ref().map_or(0, |c| c.total),
            &done,
            &holds,
            &active,
            &launches,
        );
        let p = def.interaction.translation;
        let anchor = jc2_formats::terrain::tile_index(p.x, p.z).filter(|key| {
            st.loaded.contains_key(key)
                && state == Availability::Available
                && crate::missions::access::normal_launch_supported(&def.mission)
        });
        let Some(generation) = giver_ticket(
            &mut access,
            &mut locations,
            &def.mission,
            &def.container,
            anchor,
        ) else {
            continue;
        };
        if locations.state(&def.container, generation)
            != Some(&crate::locations::LocationState::Ready)
        {
            continue;
        }
        if mode
            .as_deref()
            .is_some_and(|m| *m != crate::vehicle::CameraMode::Hero)
        {
            continue;
        }
        if heroes
            .single()
            .is_ok_and(|(h, t)| !h.downed && h.in_vehicle.is_none() && def.contains(t.translation))
        {
            access.near = Some(def.mission);
        }
    }
}
fn sync_employers(
    mut access: ResMut<GiverAccess>,
    launches: Res<MissionLaunches>,
    active: Res<ActiveMission>,
    mut bus: ResMut<EventBus>,
) {
    for state in access.employers.values_mut() {
        state.phase = EmployerPhase::Available;
    }
    if let Some(p) = launches.preparation()
        && let LaunchOrigin::Giver { employer } = &p.origin
    {
        access
            .employers
            .entry(employer.to_ascii_lowercase())
            .or_default()
            .phase = EmployerPhase::Preparing(p.owner);
    }
    for status in &launches.history {
        let LaunchStatus::Started {
            owner,
            id,
            origin: LaunchOrigin::Giver { employer },
        } = status
        else {
            continue;
        };
        let event = access
            .definitions
            .get(id)
            .and_then(|d| d.completion_event.clone());
        let state = access
            .employers
            .entry(employer.to_ascii_lowercase())
            .or_default();
        if active.0.as_ref().is_some_and(|r| r.owner == *owner) {
            state.phase = EmployerPhase::Active(*owner);
        }
        if !state.introduction_completed
            && let Some(event) = event
        {
            state.introduction_completed = true;
            bus.queue.push(event);
        }
    }
}
fn prompts(
    access: Res<GiverAccess>,
    db: Res<MissionDb>,
    map: Res<MapState>,
    list: Option<ResMut<crate::hud::PromptList>>,
) {
    let Some(mut list) = list else { return };
    if map.open {
        return;
    }
    if let Some(def) = access
        .near
        .as_ref()
        .and_then(|id| access.definitions.get(id))
    {
        list.0.insert(
            0,
            crate::hud::Prompt {
                label: format!(
                    "{}: {}",
                    def.prompt,
                    db.catalog
                        .get(&def.mission)
                        .map_or(def.mission.as_str(), |m| m.name.as_str())
                ),
                key: "E",
            },
        );
    }
}
#[allow(clippy::too_many_arguments)]
fn input(
    mut commands: Commands,
    mut browser: ResMut<MissionBrowser>,
    mut map: ResMut<MapState>,
    access: Res<GiverAccess>,
    db: Res<MissionDb>,
    chaos: Option<Res<Chaos>>,
    done: Res<MissionsDone>,
    holds: Res<Strongholds>,
    active: Res<ActiveMission>,
    mut launches: ResMut<MissionLaunches>,
    keys: Option<ResMut<ButtonInput<KeyCode>>>,
    buttons: Option<ResMut<ButtonInput<MouseButton>>>,
    actions: Query<(&Interaction, &Action), Changed<Interaction>>,
    at_start: Option<Res<OpenMissionsAtStart>>,
    market: Option<Res<crate::blackmarket::Market>>,
) {
    let Some(mut keys) = keys else { return };
    if market.as_ref().is_some_and(|m| m.open) {
        return;
    }
    let mut consumed = false;
    if let Some(start) = at_start {
        browser.selected = Some(start.0.to_ascii_lowercase());
        browser.page = ordered_entries(&db)
            .iter()
            .position(|m| m.record.id.eq_ignore_ascii_case(&start.0))
            .unwrap_or(0)
            / 8;
        browser.view = BrowserView::Missions;
        browser.open_requested = true;
        commands.remove_resource::<OpenMissionsAtStart>();
    }
    if !map.open
        && keys.just_pressed(KeyCode::KeyE)
        && let Some(def) = access
            .near
            .as_ref()
            .and_then(|id| access.definitions.get(id))
    {
        browser.selected = Some(def.mission.clone());
        browser.page = ordered_entries(&db)
            .iter()
            .position(|m| m.record.id == def.mission)
            .unwrap_or(0)
            / 8;
        browser.giver_origin = Some(def.employer.clone());
        browser.view = BrowserView::Giver;
        browser.open_requested = true;
        browser.message.clear();
        consumed = true;
    }
    if keys.just_pressed(KeyCode::KeyJ) || (map.open && keys.just_pressed(KeyCode::Tab)) {
        browser.view = BrowserView::Missions;
        browser.giver_origin = None;
        browser.open_requested = true;
        consumed = true;
    }
    if map.open {
        let mut requested: Vec<_> = actions
            .iter()
            .filter(|(i, _)| **i == Interaction::Pressed)
            .map(|(_, a)| a.0.clone())
            .collect();
        if browser.view != BrowserView::Map {
            if keys.just_pressed(KeyCode::ArrowRight) || keys.just_pressed(KeyCode::PageDown) {
                requested.push(BrowserAction::Next);
            }
            if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::PageUp) {
                requested.push(BrowserAction::Previous);
            }
        }
        for action in requested {
            act(
                action,
                &mut browser,
                &mut map,
                &db,
                chaos.as_ref().map_or(0, |c| c.total),
                &done,
                &holds,
                &active,
                &mut launches,
            );
            consumed = true;
        }
    }
    if consumed {
        keys.reset_all();
        if let Some(mut buttons) = buttons {
            buttons.reset_all();
        }
    }
}

fn button(commands: &mut Commands, parent: Entity, label: &str, action: BrowserAction) -> Entity {
    let row = matches!(action, BrowserAction::Inspect(_));
    let e = commands
        .spawn((
            Button,
            Action(action),
            Node {
                padding: UiRect::axes(Val::Px(12.), Val::Px(if row { 5. } else { 9. })),
                margin: UiRect::all(Val::Px(3.)),
                min_height: Val::Px(34.),
                border: UiRect::all(Val::Px(1.)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.10, 0.18, 0.24)),
            BorderColor::all(Color::srgb(0.24, 0.46, 0.55)),
        ))
        .id();
    let text = commands
        .spawn((
            Text::new(label),
            TextFont::from_font_size(15.),
            TextColor(Color::srgb(0.94, 0.96, 0.95)),
        ))
        .id();
    commands.entity(e).add_child(text);
    commands.entity(parent).add_child(e);
    e
}
fn build_ui(mut commands: Commands) {
    let root = commands
        .spawn((
            BrowserRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.),
                height: Val::Percent(100.),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            GlobalZIndex(110),
            Visibility::Hidden,
        ))
        .id();
    let tabs = commands
        .spawn(Node {
            width: Val::Percent(94.),
            height: Val::Px(64.),
            justify_content: JustifyContent::FlexEnd,
            align_items: AlignItems::Center,
            ..default()
        })
        .id();
    commands.entity(root).add_child(tabs);
    button(&mut commands, tabs, "MAP", BrowserAction::Map);
    button(
        &mut commands,
        tabs,
        "MISSIONS   [Tab]",
        BrowserAction::Missions,
    );
    let panel = commands
        .spawn((
            BrowserPanel,
            Node {
                width: Val::Percent(94.),
                height: Val::Percent(82.),
                padding: UiRect::all(Val::Px(20.)),
                column_gap: Val::Px(22.),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.055, 0.08, 0.98)),
        ))
        .id();
    commands.entity(root).add_child(panel);
    let left = commands
        .spawn(Node {
            width: Val::Percent(44.),
            height: Val::Percent(100.),
            flex_direction: FlexDirection::Column,
            ..default()
        })
        .id();
    commands.entity(panel).add_child(left);
    let heading = commands
        .spawn((
            Text::new("MISSIONS"),
            TextFont::from_font_size(26.),
            TextColor(Color::srgb(0.63, 0.88, 0.90)),
            Node {
                margin: UiRect::bottom(Val::Px(12.)),
                ..default()
            },
        ))
        .id();
    commands.entity(left).add_child(heading);
    let rows = commands
        .spawn((
            MissionRows,
            Node {
                width: Val::Percent(100.),
                flex_grow: 1.,
                flex_direction: FlexDirection::Column,
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .id();
    commands.entity(left).add_child(rows);
    let pages = commands
        .spawn(Node {
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            ..default()
        })
        .id();
    commands.entity(left).add_child(pages);
    button(&mut commands, pages, "Previous", BrowserAction::Previous);
    let p = commands
        .spawn((PageLabel, Text::new(""), TextFont::from_font_size(14.)))
        .id();
    commands.entity(pages).add_child(p);
    button(&mut commands, pages, "Next", BrowserAction::Next);
    let right = commands
        .spawn(Node {
            width: Val::Percent(56.),
            height: Val::Percent(100.),
            flex_direction: FlexDirection::Column,
            ..default()
        })
        .id();
    commands.entity(panel).add_child(right);
    let details = commands
        .spawn((
            Details,
            Text::new("Select a mission to inspect its intelligence file."),
            TextFont::from_font_size(18.),
            TextColor(Color::srgb(0.94, 0.96, 0.95)),
            Node {
                width: Val::Percent(100.),
                flex_grow: 1.,
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .id();
    commands.entity(right).add_child(details);
    let actions = commands
        .spawn(Node {
            flex_wrap: FlexWrap::Wrap,
            ..default()
        })
        .id();
    commands.entity(right).add_child(actions);
    button(
        &mut commands,
        actions,
        "Set waypoint",
        BrowserAction::Waypoint,
    );
    button(
        &mut commands,
        actions,
        "Clear waypoint",
        BrowserAction::ClearWaypoint,
    );
    button(
        &mut commands,
        actions,
        "Travel to giver",
        BrowserAction::Travel,
    );
    button(
        &mut commands,
        actions,
        "Start mission",
        BrowserAction::Start,
    );
    button(
        &mut commands,
        actions,
        "Cancel current mission",
        BrowserAction::Cancel,
    );
    let hint = commands
        .spawn((
            Text::new("Inspecting a mission does not start it.    M / Esc: close"),
            TextFont::from_font_size(14.),
            TextColor(Color::srgb(0.70, 0.79, 0.82)),
            Node {
                margin: UiRect::top(Val::Px(12.)),
                ..default()
            },
        ))
        .id();
    commands.entity(root).add_child(hint);
}
#[allow(clippy::too_many_arguments)]
fn refresh_ui(
    mut commands: Commands,
    map: Res<MapState>,
    mut browser: ResMut<MissionBrowser>,
    db: Res<MissionDb>,
    chaos: Option<Res<Chaos>>,
    done: Res<MissionsDone>,
    holds: Res<Strongholds>,
    active: Res<ActiveMission>,
    launches: Res<MissionLaunches>,
    mut roots: Query<&mut Visibility, With<BrowserRoot>>,
    mut panels: Query<&mut Node, With<BrowserPanel>>,
    mut details: Query<&mut Text, (With<Details>, Without<PageLabel>)>,
    mut page_labels: Query<&mut Text, (With<PageLabel>, Without<Details>)>,
    rows: Query<Entity, With<MissionRows>>,
    children: Query<&Children>,
    mut signature: ResMut<UiSignature>,
) {
    for mut root in &mut roots {
        *root = if map.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut panel in &mut panels {
        panel.display = if browser.view == BrowserView::Map {
            Display::None
        } else {
            Display::Flex
        };
    }
    if !map.open || browser.view == BrowserView::Map {
        return;
    }
    let entries = ordered_entries(&db);
    if browser.selected.is_none() {
        browser.selected = entries.first().map(|m| m.record.id.clone());
    }
    let chaos = chaos.as_ref().map_or(0, |c| c.total);
    let origin = browser
        .giver_origin
        .as_ref()
        .map_or(LaunchOrigin::Pda, |id| LaunchOrigin::Giver {
            employer: id.clone(),
        });
    let state = browser
        .selected
        .as_ref()
        .map(|id| state_for(&db, id, origin, chaos, &done, &holds, &active, &launches));
    let statuses: Vec<_> = entries
        .iter()
        .map(|m| {
            state_for(
                &db,
                &m.record.id,
                LaunchOrigin::Pda,
                chaos,
                &done,
                &holds,
                &active,
                &launches,
            )
        })
        .collect();
    let next = format!(
        "{:?}/{:?}/{}/{}/{:?}/{:?}/{:?}",
        browser.view,
        browser.selected,
        browser.page,
        browser.message,
        state,
        statuses,
        map.waypoint
    );
    if signature.0 == next {
        return;
    }
    signature.0 = next;
    if let Ok(mut text) = details.single_mut() {
        text.0 = if let Some(entry) = browser.selected.as_ref().and_then(|id| db.catalog.get(id)) {
            let faction = entry
                .record
                .faction()
                .map_or("Unknown faction", |f| f.name());
            let giver = entry
                .giver
                .as_ref()
                .map_or("No giver location available".into(), |g| {
                    format!("Giver location: {:.0}, {:.0}", g.position[0], g.position[2])
                });
            let support = if crate::missions::access::normal_launch_supported(&entry.record.id) {
                if browser.view == BrowserView::Giver {
                    "Limited mission preview: start without the briefing. Dialogue and scripted warnings are unavailable."
                } else {
                    "Limited mission preview. Dialogue, briefings and scripted warnings are unavailable. Start stays at your current position."
                }
            } else {
                "This mission can be inspected and marked on the map; it cannot be started in this build."
            };
            let status = state
                .as_ref()
                .map_or("Not available".into(), |s| label(&db, s));
            format!(
                "{}\n{} - {}\n\n{}\n\n{}\nChaos required: {}\n{}\n\n{}\n\n{}",
                entry.name,
                faction,
                entry.record.kind.name(),
                display_description(&entry.description),
                status,
                entry.record.chaos,
                giver,
                support,
                browser.message
            )
        } else {
            "Select a mission.".into()
        };
    }
    if let Ok(mut page) = page_labels.single_mut() {
        page.0 = format!("{} / {}", browser.page + 1, entries.len().div_ceil(8));
    }
    if let Ok(parent) = rows.single() {
        if let Ok(old) = children.get(parent) {
            for child in old.iter() {
                commands.entity(child).despawn();
            }
        }
        for (entry, status) in entries
            .iter()
            .zip(statuses.iter())
            .skip(browser.page * 8)
            .take(8)
        {
            let selected = browser.selected.as_deref() == Some(&entry.record.id);
            let title = format!(
                "{}{}\n{} - {}",
                if selected { "> " } else { "" },
                entry.name,
                entry.record.faction().map_or("Unknown", |f| f.name()),
                label(&db, status)
            );
            button(
                &mut commands,
                parent,
                &title,
                BrowserAction::Inspect(entry.record.id.clone()),
            );
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn action_visibility(
    browser: Res<MissionBrowser>,
    db: Res<MissionDb>,
    chaos: Option<Res<Chaos>>,
    done: Res<MissionsDone>,
    holds: Res<Strongholds>,
    active: Res<ActiveMission>,
    launches: Res<MissionLaunches>,
    mut buttons: Query<(&Action, &mut Node)>,
) {
    let entry = browser.selected.as_ref().and_then(|id| db.catalog.get(id));
    let origin = browser
        .giver_origin
        .as_ref()
        .map_or(LaunchOrigin::Pda, |id| LaunchOrigin::Giver {
            employer: id.clone(),
        });
    let available = entry.is_some_and(|e| {
        normal_start_available(
            &db,
            &e.record.id,
            origin,
            chaos.as_ref().map_or(0, |c| c.total),
            &done,
            &holds,
            &active,
            &launches,
        )
    });
    let busy = active.0.is_some() || launches.preparation().is_some();
    for (action, mut node) in &mut buttons {
        let visible = match action.0 {
            BrowserAction::Start => available,
            BrowserAction::Travel => !busy && entry.is_some_and(|e| e.giver.is_some()),
            BrowserAction::Waypoint => entry.is_some_and(|e| e.giver.is_some()),
            BrowserAction::Cancel => busy,
            _ => true,
        };
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
}
#[allow(clippy::too_many_arguments)]
fn normal_start_available(
    db: &MissionDb,
    id: &str,
    origin: LaunchOrigin,
    chaos: u32,
    done: &MissionsDone,
    holds: &Strongholds,
    active: &ActiveMission,
    launches: &MissionLaunches,
) -> bool {
    crate::missions::access::normal_launch_supported(id)
        && state_for(db, id, origin, chaos, done, holds, active, launches)
            == Availability::Available
}

fn ordered_entries(db: &MissionDb) -> Vec<&crate::missions::access::CatalogMission> {
    let mut entries: Vec<_> = db.catalog.0.iter().collect();
    entries.sort_by(|a, b| {
        a.record
            .faction()
            .map(|f| f.name())
            .cmp(&b.record.faction().map(|f| f.name()))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.record.id.cmp(&b.record.id))
    });
    entries
}
fn display_description(text: &str) -> String {
    text.replace("\\n", "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    fn fixture() -> Option<(std::sync::Arc<jc2_formats::GameFs>, MissionDb)> {
        let fs = std::sync::Arc::new(
            jc2_formats::GameFs::open(std::env::var("JC2_TEST_DIR").ok()?).ok()?,
        );
        let db = MissionDb::load(&fs);
        Some((fs, db))
    }
    fn definitions(fs: &jc2_formats::GameFs, db: &MissionDb) -> GiverAccess {
        let mut access = GiverAccess::default();
        for id in ["f2t10", "km03"] {
            let container = db
                .catalog
                .get(id)
                .unwrap()
                .giver
                .as_ref()
                .unwrap()
                .container
                .as_ref()
                .unwrap();
            let data = fs.read_inflated(container).unwrap();
            let file = jc2_formats::missions::first_property_member(&data, &[".blo"]).unwrap();
            let def = GiverDefinition::parse(db, id, container, &file)
                .expect("real giver smart object, trigger and action");
            access.definitions.insert(id.into(), def);
        }
        access
    }
    fn unlocked(db: &MissionDb) -> (MissionsDone, u32) {
        let record = db.record("f2t10").unwrap();
        let mut done = MissionsDone::default();
        if let Some(id) = record.prerequisite() {
            done.0.push(id.into());
        }
        (done, record.chaos)
    }
    #[test]
    fn original_givers_preserve_action_box_prompt_and_distinct_employer_identity() {
        let Some((fs, db)) = fixture() else { return };
        let access = definitions(&fs, &db);
        let f = &access.definitions["f2t10"];
        assert_eq!(f.employer, "f2t10Emp");
        let description = &db.catalog.get("f2t10").unwrap().description;
        assert!(description.contains("\\n"));
        assert!(!display_description(description).contains("\\n"));
        assert!(display_description(description).contains('\n'));
        assert_eq!(f.prompt, "Activate");
        assert_eq!(f.box_size, Vec3::splat(5.));
        assert!(f.action_events.iter().any(|e| e.ends_with("intro.play")));
        assert_eq!(f.completion_event.as_deref(), Some("msy.f2t10emp.complete"));
        assert!(f.contains(f.interaction.translation));
        assert!(!f.contains(
            f.interaction.translation + f.interaction.rotation * Vec3::new(2.51, 0., 0.)
        ));
        assert_eq!(f.routers.len(), 3);
        assert_eq!(
            f.routers.iter().map(|r| r.code).collect::<Vec<_>>(),
            vec![0, 3, 2]
        );
        let k = &access.definitions["km03"];
        assert_eq!(k.routers.len(), 4);
        assert_eq!(k.prompt, "Talk");
        assert_eq!(k.completion_event.as_deref(), Some("msy.km03emp.complete"));
        assert_ne!(k.employer, k.mission);
        assert!(!crate::missions::access::normal_launch_supported("km03"));
    }
    #[test]
    fn inspect_waypoint_and_travel_are_independent_of_normal_launch() {
        let Some((_, db)) = fixture() else { return };
        let (done, chaos) = unlocked(&db);
        let holds = Strongholds::default();
        let active = ActiveMission::default();
        let mut launches = MissionLaunches::default();
        let mut browser = MissionBrowser::default();
        let mut map = MapState::default();
        for action in [
            BrowserAction::Inspect("F2T10".into()),
            BrowserAction::Waypoint,
        ] {
            act(
                action,
                &mut browser,
                &mut map,
                &db,
                chaos,
                &done,
                &holds,
                &active,
                &mut launches,
            );
        }
        assert_eq!(browser.selected.as_deref(), Some("f2t10"));
        let giver = db.catalog.get("f2t10").unwrap().giver.as_ref().unwrap();
        assert_eq!(map.waypoint, Some(Vec3::from(giver.position)));
        assert!(map.travel.is_none());
        assert!(launches.preparation().is_none());
        assert!(launches.history.is_empty());
        act(
            BrowserAction::Travel,
            &mut browser,
            &mut map,
            &db,
            chaos,
            &done,
            &holds,
            &active,
            &mut launches,
        );
        assert_eq!(
            map.travel,
            Some(Vec2::new(giver.position[0], giver.position[2]))
        );
        assert!(browser.close_requested);
        assert!(launches.history.is_empty());
        assert!(launches.preparation().is_none());
    }
    fn launch_world(db: MissionDb) -> World {
        let mut world = World::new();
        world.insert_resource(db);
        world.init_resource::<Chaos>();
        world.init_resource::<MissionsDone>();
        world.init_resource::<Strongholds>();
        world.init_resource::<ActiveMission>();
        world.init_resource::<MissionLaunches>();
        world.init_resource::<EventBus>();
        world.init_resource::<crate::world::ExtraContainers>();
        world.init_resource::<crate::hud::Objective>();
        world.init_resource::<crate::hud::Notices>();
        world
    }
    #[test]
    fn locked_and_unsupported_start_never_enter_the_shared_launch_queue() {
        let Some((fs, db)) = fixture() else { return };
        let mut world = launch_world(MissionDb::load(&fs));
        let mut browser = MissionBrowser {
            selected: Some("f2t10".into()),
            ..default()
        };
        let mut map = MapState::default();
        let mut launches = MissionLaunches::default();
        act(
            BrowserAction::Start,
            &mut browser,
            &mut map,
            &db,
            0,
            &MissionsDone::default(),
            &Strongholds::default(),
            &ActiveMission::default(),
            &mut launches,
        );
        assert!(!browser.close_requested);
        let all_done = MissionsDone(
            db.catalog
                .0
                .iter()
                .filter(|m| m.record.id != "km03")
                .map(|m| m.record.id.clone())
                .collect(),
        );
        browser.selected = Some("km03".into());
        act(
            BrowserAction::Start,
            &mut browser,
            &mut map,
            &db,
            u32::MAX,
            &all_done,
            &Strongholds::default(),
            &ActiveMission::default(),
            &mut launches,
        );
        assert!(!browser.close_requested);
        world.insert_resource(Game { fs });
        world.insert_resource(launches);
        crate::missions::access::service_launches(&mut world);
        assert!(world.resource::<MissionLaunches>().history.is_empty());
    }
    #[test]
    fn giver_launch_waits_for_shared_receipt_can_cancel_and_retries_with_new_owner() {
        let Some((fs, db)) = fixture() else { return };
        let (done, chaos) = unlocked(&db);
        let mut world = launch_world(MissionDb::load(&fs));
        world.insert_resource(Game { fs });
        world.insert_resource(MissionsDone(done.0.clone()));
        world.resource_mut::<Chaos>().total = chaos;
        let mut browser = MissionBrowser {
            selected: Some("f2t10".into()),
            giver_origin: Some("f2t10Emp".into()),
            view: BrowserView::Giver,
            ..default()
        };
        let mut map = MapState::default();
        let mut launches = MissionLaunches::default();
        act(
            BrowserAction::Start,
            &mut browser,
            &mut map,
            &db,
            chaos,
            &done,
            &Strongholds::default(),
            &ActiveMission::default(),
            &mut launches,
        );
        world.insert_resource(launches);
        crate::missions::access::service_launches(&mut world);
        let owner = world
            .resource::<MissionLaunches>()
            .preparation()
            .unwrap()
            .owner;
        assert_eq!(
            world
                .resource::<MissionLaunches>()
                .preparation()
                .unwrap()
                .origin,
            LaunchOrigin::Giver {
                employer: "f2t10Emp".into()
            }
        );
        assert!(world.resource::<ActiveMission>().0.is_none());
        world.resource_mut::<MissionLaunches>().cancel(owner);
        crate::missions::access::service_launches(&mut world);
        assert!(world.resource::<MissionLaunches>().preparation().is_none());
        world
            .resource_mut::<MissionLaunches>()
            .request(MissionLaunchRequest {
                id: "f2t10".into(),
                origin: LaunchOrigin::Pda,
                debug_override: false,
            });
        crate::missions::access::service_launches(&mut world);
        assert_ne!(
            world
                .resource::<MissionLaunches>()
                .preparation()
                .unwrap()
                .owner,
            owner
        );
        assert!(
            !world
                .resource_mut::<MissionLaunches>()
                .acknowledge(owner, Ok(()))
        );
    }
    #[test]
    fn employer_intro_skip_event_is_once_after_start_and_does_not_complete_mission() {
        let Some((fs, db)) = fixture() else { return };
        let mut world = launch_world(MissionDb::load(&fs));
        world.insert_resource(definitions(&fs, &db));
        let status = LaunchStatus::Started {
            owner: MissionOwner(41),
            id: "f2t10".into(),
            origin: LaunchOrigin::Giver {
                employer: "f2t10Emp".into(),
            },
        };
        world
            .resource_mut::<MissionLaunches>()
            .history
            .push_back(status);
        world.run_system_once(sync_employers).unwrap();
        world.run_system_once(sync_employers).unwrap();
        assert_eq!(
            world.resource::<EventBus>().queue,
            vec!["msy.f2t10emp.complete"]
        );
        assert!(world.resource::<MissionsDone>().0.is_empty());
        assert!(world.resource::<GiverAccess>().employers["f2t10emp"].introduction_completed);
        assert_eq!(
            world.resource::<GiverAccess>().employers["f2t10emp"].phase,
            EmployerPhase::Available
        );
    }
    #[test]
    fn mission_modal_blocks_gameplay_and_map_click_travel_restores_time_on_close() {
        use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
        let Some((_, db)) = fixture() else { return };
        let Some((mut app, _, _)) = crate::fleet::testing::app(&[crate::map::MAP_TEXTURE], |app| {
            app.insert_resource(db)
                .init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<AccumulatedMouseMotion>()
                .init_resource::<AccumulatedMouseScroll>()
                .init_resource::<MissionsDone>()
                .init_resource::<Strongholds>()
                .init_resource::<ActiveMission>()
                .init_resource::<MissionLaunches>()
                .init_resource::<GiverAccess>()
                .init_resource::<MissionBrowser>()
                .add_plugins(crate::map::MapPlugin)
                .add_systems(PreUpdate, input.before(crate::map::MapInput));
        }) else {
            return;
        };
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyJ);
        app.update();
        assert!(app.world().resource::<MapState>().open);
        assert_eq!(
            app.world().resource::<MissionBrowser>().view,
            BrowserView::Missions
        );
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        {
            let world = app.world_mut();
            world
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyW);
            world
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            world.resource_mut::<AccumulatedMouseMotion>().delta = Vec2::ONE;
            world.resource_mut::<AccumulatedMouseScroll>().delta = Vec2::ONE;
        }
        app.update();
        assert!(app.world().resource::<MapState>().open);
        assert!(app.world().resource::<MapState>().travel.is_none());
        assert!(
            !app.world()
                .resource::<ButtonInput<KeyCode>>()
                .pressed(KeyCode::KeyW)
        );
        assert!(
            !app.world()
                .resource::<ButtonInput<MouseButton>>()
                .pressed(MouseButton::Left)
        );
        assert_eq!(
            app.world().resource::<AccumulatedMouseMotion>().delta,
            Vec2::ZERO
        );
        assert_eq!(
            app.world().resource::<AccumulatedMouseScroll>().delta,
            Vec2::ZERO
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();
        assert!(!app.world().resource::<MapState>().open);
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(
            app.world().resource::<MissionBrowser>().view,
            BrowserView::Map
        );
    }
    #[test]
    fn inspecting_another_mission_clears_the_previous_employer_context() {
        let Some((_, db)) = fixture() else { return };
        let mut browser = MissionBrowser {
            view: BrowserView::Giver,
            selected: Some("f2t10".into()),
            giver_origin: Some("f2t10Emp".into()),
            ..default()
        };
        act(
            BrowserAction::Inspect("km03".into()),
            &mut browser,
            &mut MapState::default(),
            &db,
            0,
            &MissionsDone::default(),
            &Strongholds::default(),
            &ActiveMission::default(),
            &mut MissionLaunches::default(),
        );
        assert_eq!(browser.selected.as_deref(), Some("km03"));
        assert_eq!(browser.view, BrowserView::Missions);
        assert!(browser.giver_origin.is_none());
    }
    #[test]
    fn unavailable_giver_releases_geometry_without_releasing_shared_mission_or_tile() {
        use crate::locations::{LocationOwner, LocationState, Locations};
        let Some(mut app) = crate::world::tests::tile_app(|app| {
            app.add_plugins(crate::locations::LocationsPlugin);
        }) else {
            return;
        };
        app.world_mut()
            .resource_mut::<Locations>()
            .registry
            .0
            .clear();
        let fs = app.world().resource::<Game>().fs.clone();
        let db = MissionDb::load(&fs);
        let mut access = definitions(&fs, &db);
        let container = access.definitions["f2t10"].container.clone();
        let mission = LocationOwner::Mission(MissionOwner(720));
        let tile = LocationOwner::Tile((31, 14));
        let generation = {
            let mut locations = app.world_mut().resource_mut::<Locations>();
            let generation = giver_ticket(
                &mut access,
                &mut locations,
                "f2t10",
                &container,
                Some((31, 14)),
            )
            .unwrap();
            assert_eq!(generation, locations.request(mission, &container, (31, 14)));
            assert_eq!(generation, locations.request(tile, &container, (31, 14)));
            generation
        };
        crate::fleet::testing::wait_until(&mut app, "giver geometry readiness", |app| {
            let state = app
                .world()
                .resource::<Locations>()
                .state(&container, generation);
            assert!(
                !matches!(state, Some(LocationState::Failed(_))),
                "giver: {state:?}"
            );
            state == Some(&LocationState::Ready)
        });
        let giver = access.tickets["f2t10"].owner;
        {
            let mut locations = app.world_mut().resource_mut::<Locations>();
            assert!(locations.has_owner(&container, giver));
            giver_ticket(&mut access, &mut locations, "f2t10", &container, None);
            assert!(!locations.has_owner(&container, giver));
            assert!(locations.has_owner(&container, mission));
            assert!(locations.has_owner(&container, tile));
        }
        app.update();
        assert_eq!(
            app.world()
                .resource::<Locations>()
                .state(&container, generation),
            Some(&LocationState::Ready)
        );
        app.world_mut().resource_mut::<Locations>().release(mission);
        app.update();
        assert_eq!(
            app.world()
                .resource::<Locations>()
                .state(&container, generation),
            Some(&LocationState::Ready)
        );
        app.world_mut().resource_mut::<Locations>().release(tile);
        crate::fleet::testing::wait_until(&mut app, "released giver geometry unload", |app| {
            app.world()
                .resource::<Locations>()
                .state(&container, generation)
                .is_none()
        });
        let new_generation = giver_ticket(
            &mut access,
            &mut app.world_mut().resource_mut::<Locations>(),
            "f2t10",
            &container,
            Some((31, 14)),
        )
        .unwrap();
        assert_ne!(new_generation, generation);
        assert_ne!(access.tickets["f2t10"].owner, giver);
    }
}
