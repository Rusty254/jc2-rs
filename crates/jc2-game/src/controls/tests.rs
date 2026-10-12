use super::settings::{ControlSettings, LoadStatus, SETTINGS_VERSION, SaveError, SettingsStore};
use super::*;
use std::time::Duration;

const KEY_Z: Input = Input::Key(KeyCode::KeyZ);
const SHIFT: Input = Input::Key(KeyCode::ShiftLeft);
const W: Input = Input::Key(KeyCode::KeyW);
const E: Input = Input::Key(KeyCode::KeyE);

const PULSE: ActionId = ActionId("test.pulse");
const SHIFT_Z: ActionId = ActionId("test.shift_z");
const MENU_E: ActionId = ActionId("test.menu_e");
const FOOT_E: ActionId = ActionId("test.foot_e");
const GLOBAL_E: ActionId = ActionId("test.global_e");
const VEH_W: ActionId = ActionId("test.vehicle_w");
const PLAIN_Z: ActionId = ActionId("test.plain_z");
const LOOK_X: ActionId = ActionId("test.look_x");

fn def(id: ActionId, ctx: Context, key: KeyCode) -> ActionDef {
    ActionDef {
        id,
        context: ctx,
        kind: ActionKind::Button,
        name: "test",
        defaults: [Some(Binding::key(key)), None],
    }
}

/// The defaults plus a few synthetic actions in other contexts.
fn controls() -> Controls {
    let mut c = Controls::default();
    c.register(def(PULSE, Context::Global, KeyCode::F5));
    c.register(ActionDef {
        id: SHIFT_Z,
        context: Context::OnFoot,
        kind: ActionKind::Button,
        name: "test",
        defaults: [
            Some(Binding::key(KeyCode::KeyZ).with(Modifiers::SHIFT)),
            None,
        ],
    });
    c.register(def(MENU_E, Context::Menu, KeyCode::KeyE));
    c.register(def(FOOT_E, Context::OnFoot, KeyCode::KeyH));
    c.register(def(VEH_W, Context::Vehicle, KeyCode::KeyW));
    c.register(ActionDef {
        id: LOOK_X,
        context: Context::Global,
        kind: ActionKind::Axis,
        name: "test",
        defaults: [
            Some(Binding::key(KeyCode::Digit1)),
            Some(Binding::key(KeyCode::Digit2)),
        ],
    });
    c
}

const FOOT: &[Context] = &[Context::Global, Context::OnFoot];

fn frame(buttons: &[Input]) -> InputFrame {
    InputFrame::held(buttons.iter().copied())
}

#[test]
fn defaults_are_the_projects_existing_keys_and_label_through_the_registry() {
    let c = Controls::default();
    assert_eq!(c.label(MOVE_FORWARD), "W");
    assert_eq!(c.labels(MOVE_FORWARD), "W / UP");
    assert_eq!(c.label(SPRINT), "LSHIFT");
    assert_eq!(c.label(ActionId("nope")), "UNKNOWN ACTION");
    assert!(c.all_conflicts().is_empty());
    let mut c = c;
    c.remap(MOVE_FORWARD, 0, None, ConflictPolicy::Reject)
        .unwrap();
    c.remap(MOVE_FORWARD, 1, None, ConflictPolicy::Reject)
        .unwrap();
    assert_eq!(c.label(MOVE_FORWARD), "UNBOUND");
}

#[test]
fn down_held_up_and_modifier_transitions_are_deterministic() {
    let mut c = controls();
    c.register(def(PLAIN_Z, Context::OnFoot, KeyCode::KeyZ));
    // Z alone: the plain binding, not the Shift+Z one
    c.evaluate(&frame(&[]), FOOT, None);
    c.evaluate(&frame(&[KEY_Z]), FOOT, None);
    assert!(!c.down(SHIFT_Z));
    assert!(c.down(PLAIN_Z) && c.just_pressed(PLAIN_Z));
    // Shift joins: the more specific gesture takes Z, the plain one ends
    c.evaluate(&frame(&[KEY_Z, SHIFT]), FOOT, None);
    assert!(c.down(SHIFT_Z) && c.just_pressed(SHIFT_Z));
    assert!(!c.down(PLAIN_Z) && c.just_released(PLAIN_Z));
    // next frame: still held, no second edge
    c.evaluate(&frame(&[KEY_Z, SHIFT]), FOOT, None);
    assert!(c.down(SHIFT_Z) && !c.just_pressed(SHIFT_Z));
    // Shift lets go while Z stays: Shift+Z is over and plain Z is a fresh press (the rule,
    // stated once: edges follow the state, so the same sequence always gives the same edges)
    c.evaluate(&frame(&[KEY_Z]), FOOT, None);
    assert!(!c.down(SHIFT_Z) && c.just_released(SHIFT_Z));
    assert!(c.down(PLAIN_Z) && c.just_pressed(PLAIN_Z));
    // Shift itself is Sprint, and does not stop movement keys from matching
    c.evaluate(&frame(&[W, SHIFT]), FOOT, None);
    assert!(c.down(SPRINT) && c.down(MOVE_FORWARD));
    // a binding with no modifiers still matches with extra ones held
    c.evaluate(
        &frame(&[KEY_Z, Input::Key(KeyCode::ControlLeft)]),
        FOOT,
        None,
    );
    assert!(c.down(PLAIN_Z) && !c.down(SHIFT_Z));
}

#[test]
fn a_press_is_delivered_once_across_zero_or_many_fixed_steps() {
    let mut c = controls();
    // zero fixed steps ran for three frames: the press is still waiting
    c.evaluate(&frame(&[]), FOOT, None);
    c.evaluate(&frame(&[Input::Key(KeyCode::F5)]), FOOT, None);
    assert!(c.just_pressed(PULSE));
    c.evaluate(&frame(&[Input::Key(KeyCode::F5)]), FOOT, None);
    assert!(!c.just_pressed(PULSE), "the frame edge lasts one dispatch");
    c.evaluate(&frame(&[Input::Key(KeyCode::F5)]), FOOT, None);
    // then several steps: exactly one consumer sees it
    assert!(c.take_pressed(PULSE));
    assert!(!c.take_pressed(PULSE));
    assert!(!c.take_pressed(PULSE));
    // a tap that is over before any fixed step still counts, once
    c.evaluate(&frame(&[]), FOOT, None);
    c.evaluate(&frame(&[Input::Key(KeyCode::F5)]), FOOT, None);
    c.evaluate(&frame(&[]), FOOT, None);
    assert!(!c.down(PULSE));
    assert!(c.take_pressed(PULSE));
    assert!(c.take_released(PULSE));
    assert!(!c.take_pressed(PULSE));
}

#[test]
fn modal_focus_death_and_cancel_release_everything_and_rearm_on_release() {
    for why in [Neutralize::Modal, Neutralize::Death, Neutralize::Cancel] {
        let mut c = controls();
        let mut f = frame(&[W]);
        f.authoritative_up = false; // a real session: Released messages exist
        c.evaluate(&f, FOOT, None);
        assert!(c.down(MOVE_FORWARD));
        c.evaluate(&f, FOOT, Some(why));
        assert!(!c.down(MOVE_FORWARD), "{why:?}");
        assert!(c.just_released(MOVE_FORWARD));
        assert_eq!(c.why_neutralized(), Some(why));
        // the modal reset the raw input; then key repeat presses W again on resume
        c.evaluate(&f, FOOT, None);
        assert!(!c.down(MOVE_FORWARD), "{why:?}: held through, no new press");
        assert!(!c.just_pressed(MOVE_FORWARD));
        assert!(!c.take_pressed(MOVE_FORWARD));
        // the player really lets go, and presses again
        let mut up = frame(&[]);
        up.authoritative_up = false;
        up.released = vec![W];
        c.evaluate(&up, FOOT, None);
        c.evaluate(&f, FOOT, None);
        assert!(
            c.down(MOVE_FORWARD) && c.just_pressed(MOVE_FORWARD),
            "{why:?}"
        );
    }
    // synthetic input without Released messages: seeing the button up is enough
    let mut c = controls();
    c.evaluate(&frame(&[W]), FOOT, None);
    c.evaluate(&frame(&[W]), FOOT, Some(Neutralize::Modal));
    c.evaluate(&frame(&[]), FOOT, None);
    c.evaluate(&frame(&[W]), FOOT, None);
    assert!(c.just_pressed(MOVE_FORWARD));
}

#[test]
fn losing_focus_forgets_buttons_released_while_away() {
    let mut c = controls();
    let mut f = frame(&[W]);
    f.authoritative_up = false;
    c.evaluate(&f, FOOT, None);
    c.evaluate(&frame(&[]), FOOT, Some(Neutralize::FocusLost));
    // W was let go while the window had no focus: no Released message ever arrives
    let mut again = frame(&[W]);
    again.authoritative_up = false;
    c.evaluate(&again, FOOT, None);
    assert!(
        c.just_pressed(MOVE_FORWARD),
        "not stuck behind a missed release"
    );
}

#[test]
fn neutralising_drops_pending_latched_edges() {
    let mut c = controls();
    c.evaluate(&frame(&[Input::Key(KeyCode::F5)]), FOOT, None);
    c.evaluate(&frame(&[]), FOOT, Some(Neutralize::Modal));
    assert!(
        !c.take_pressed(PULSE),
        "a press from before the modal is not delivered after"
    );
}

#[test]
fn overlapping_contexts_cannot_both_consume_one_button() {
    let mut c = controls();
    // E: the menu (priority) and, for the test, a Global action both want it
    c.register(def(GLOBAL_E, Context::Global, KeyCode::KeyE));
    c.evaluate(
        &frame(&[E]),
        &[Context::Global, Context::OnFoot, Context::Menu],
        None,
    );
    assert!(c.down(MENU_E));
    assert!(!c.down(GLOBAL_E), "the menu consumed the gesture");
    // menu closed: Global sees it
    c.evaluate(&frame(&[E]), FOOT, None);
    assert!(!c.down(MENU_E));
    assert!(c.down(GLOBAL_E));
}

#[test]
fn conflicts_are_per_simultaneously_active_context() {
    let mut c = controls();
    // OnFoot and Vehicle never overlap: W is a vehicle action and a movement action
    assert_eq!(c.def(VEH_W).unwrap().context, Context::Vehicle);
    assert!(c.all_conflicts().is_empty());
    // but a second on-foot action cannot take W from the movement
    assert!(
        c.remap(
            FOOT_E,
            0,
            Some(Binding::key(KeyCode::KeyW)),
            ConflictPolicy::Reject
        )
        .is_err()
    );
    // OnFoot vs Global does overlap, and the answer names the other action
    let r = c.remap(
        FOOT_E,
        0,
        Some(Binding::key(KeyCode::F5)),
        ConflictPolicy::Reject,
    );
    assert_eq!(
        r,
        Err(RemapError::Conflict {
            with: PULSE,
            slot: 0
        })
    );
    // refused: nothing changed
    assert_eq!(c.label(FOOT_E), "H");
    // Replace clears the other slot instead
    let cleared = c
        .remap(
            FOOT_E,
            0,
            Some(Binding::key(KeyCode::F5)),
            ConflictPolicy::Replace,
        )
        .unwrap();
    assert_eq!(cleared, vec![PULSE]);
    assert_eq!(c.label(PULSE), "UNBOUND");
    assert!(c.all_conflicts().is_empty());
    // a vehicle-only binding does not collide with an on-foot one
    c.remap(
        VEH_W,
        0,
        Some(Binding::key(KeyCode::KeyA)),
        ConflictPolicy::Reject,
    )
    .unwrap();
}

#[test]
fn remap_reset_and_unknown_actions_are_explicit() {
    let mut c = controls();
    assert_eq!(
        c.remap(ActionId("missing"), 0, None, ConflictPolicy::Reject),
        Err(RemapError::UnknownAction)
    );
    assert_eq!(
        c.remap(MOVE_FORWARD, 2, None, ConflictPolicy::Reject),
        Err(RemapError::BadSlot)
    );
    assert_eq!(
        c.remap(
            MOVE_FORWARD,
            0,
            Some(Binding::key(KeyCode::KeyP).with(Modifiers::NONE)),
            ConflictPolicy::Reject
        ),
        Ok(vec![])
    );
    assert_eq!(c.label(MOVE_FORWARD), "P");
    assert!(!c.down(ActionId("missing")) && !c.is_registered(ActionId("missing")));
    c.evaluate(&frame(&[Input::Key(KeyCode::KeyP)]), FOOT, None);
    assert!(c.down(MOVE_FORWARD), "the remapped key drives the action");
    assert!(!c.reset(ActionId("missing")));
    assert!(c.reset(MOVE_FORWARD));
    assert_eq!(c.label(MOVE_FORWARD), "W");
    c.remap(
        MOVE_BACK,
        0,
        Some(Binding::key(KeyCode::KeyP)),
        ConflictPolicy::Reject,
    )
    .unwrap();
    c.reset_all();
    assert_eq!(c.label(MOVE_BACK), "S");
}

#[test]
fn axes_combine_keys_with_pushed_values_and_clamp() {
    let mut c = controls();
    c.evaluate(&frame(&[Input::Key(KeyCode::Digit2)]), FOOT, None);
    assert_eq!(c.value(LOOK_X), 1.0);
    c.evaluate(
        &frame(&[Input::Key(KeyCode::Digit2), Input::Key(KeyCode::Digit1)]),
        FOOT,
        None,
    );
    assert_eq!(c.value(LOOK_X), 0.0, "both keys cancel");
    c.push_axis(LOOK_X, -0.5);
    c.evaluate(&frame(&[]), FOOT, None);
    assert_eq!(c.value(LOOK_X), -0.5);
    c.push_axis(LOOK_X, 9.0);
    c.evaluate(&frame(&[]), FOOT, None);
    assert_eq!(c.value(LOOK_X), 1.0);
    // pushing at a button is ignored
    c.push_axis(PULSE, 1.0);
    c.evaluate(&frame(&[]), FOOT, None);
    assert!(!c.down(PULSE));
}

#[test]
fn a_context_that_just_became_active_does_not_press_what_is_already_held() {
    let mut c = controls();
    // walking with W held into a vehicle and out again: back on foot the key is held, not new
    c.evaluate(&frame(&[]), &[Context::Global, Context::Vehicle], None);
    c.evaluate(&frame(&[W]), &[Context::Global, Context::Vehicle], None);
    assert!(c.down(VEH_W) && c.just_pressed(VEH_W));
    c.evaluate(&frame(&[W]), FOOT, None);
    assert!(!c.down(VEH_W) && c.just_released(VEH_W));
    assert!(c.down(MOVE_FORWARD), "still held on foot");
    assert!(!c.just_pressed(MOVE_FORWARD));
    assert!(!c.take_pressed(MOVE_FORWARD));
}

// -- settings ------------------------------------------------------------------------

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jc2-controls-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn settings_roundtrip_reset_and_sparse_storage() {
    let dir = scratch("roundtrip");
    let path = dir.join("controls.json");
    let mut c = controls();
    assert!(
        c.export_settings().entries.is_empty(),
        "defaults store nothing"
    );
    c.remap(
        MOVE_FORWARD,
        0,
        Some(Binding::key(KeyCode::KeyI)),
        ConflictPolicy::Reject,
    )
    .unwrap();
    c.remap(SPRINT, 1, None, ConflictPolicy::Reject).unwrap();
    c.remap(
        SHIFT_Z,
        0,
        Some(
            Binding::key(KeyCode::KeyX)
                .with(Modifiers::CTRL)
                .with(Modifiers::ALT),
        ),
        ConflictPolicy::Reject,
    )
    .unwrap();
    let saved = c.export_settings();
    assert_eq!(saved.entries.len(), 3);

    let (mut store, first) = SettingsStore::load(path.clone());
    assert_eq!(first.status, LoadStatus::Missing);
    store.save(&saved).unwrap();

    let (store2, loaded) = SettingsStore::load(path.clone());
    assert_eq!(loaded.status, LoadStatus::Loaded);
    assert_eq!(store2.status, LoadStatus::Loaded);
    assert!(loaded.warnings.is_empty());
    let mut fresh = controls();
    assert!(fresh.apply_settings(&loaded.settings).is_empty());
    for id in [MOVE_FORWARD, SPRINT, SHIFT_Z, MOVE_BACK] {
        assert_eq!(fresh.bindings(id), c.bindings(id), "{id:?}");
    }
    assert_eq!(fresh.label(SHIFT_Z), "CTRL+ALT+X");
    // reset back to the defaults and store that: the file holds no overrides
    fresh.reset_all();
    let mut store3 = store2;
    store3.save(&fresh.export_settings()).unwrap();
    let (_, again) = SettingsStore::load(path);
    assert!(again.settings.entries.is_empty());
    // saving a second time kept the first as .bak
    assert!(dir.join("controls.json.bak").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_invalid_and_oversized_files_leave_defaults_usable() {
    let dir = scratch("invalid");
    let path = dir.join("controls.json");
    for (text, label) in [
        ("{ not json", "garbage"),
        ("[]", "array"),
        (r#"{"bindings":{}}"#, "no version"),
        (r#"{"version":0,"bindings":{}}"#, "version zero"),
        (r#"{"version":"1","bindings":{}}"#, "version string"),
        (r#"{"version":1}"#, "no bindings"),
    ] {
        std::fs::write(&path, text).unwrap();
        let (store, out) = SettingsStore::load(path.clone());
        assert!(matches!(out.status, LoadStatus::Invalid(_)), "{label}");
        assert!(out.settings.entries.is_empty());
        let mut c = Controls::default();
        assert!(c.apply_settings(&out.settings).is_empty());
        assert_eq!(c.label(MOVE_FORWARD), "W", "{label}");
        drop(store);
    }
    // oversized
    std::fs::write(&path, " ".repeat(70 * 1024)).unwrap();
    let (_, out) = SettingsStore::load(path.clone());
    assert!(matches!(out.status, LoadStatus::Invalid(_)));
    // not UTF-8
    std::fs::write(&path, [0xffu8, 0xfe, 0x00]).unwrap();
    let (_, out) = SettingsStore::load(path.clone());
    assert!(matches!(out.status, LoadStatus::Invalid(_)));
    // saving over an invalid file keeps it as .invalid
    let (mut store, _) = SettingsStore::load(path.clone());
    store.save(&ControlSettings::default()).unwrap();
    assert!(dir.join("controls.json.invalid").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bad_entries_are_skipped_with_a_warning_and_the_rest_apply() {
    let text = r#"{"version":1,"bindings":{
        "onfoot.move_forward":["KeyI"],
        "onfoot.move_back":["Banana"],
        "onfoot.move_left":"KeyJ",
        "onfoot.move_right":[1],
        "onfoot.sprint":["Hyper+KeyQ"],
        "future.thing":["KeyQ",null]}}"#;
    let out = ControlSettings::parse(text);
    assert_eq!(out.status, LoadStatus::Loaded);
    assert_eq!(out.warnings.len(), 4, "{:?}", out.warnings);
    let mut c = Controls::default();
    let warnings = c.apply_settings(&out.settings);
    assert_eq!(c.label(MOVE_FORWARD), "I");
    assert_eq!(c.label(MOVE_BACK), "S");
    assert_eq!(c.label(MOVE_LEFT), "A");
    assert_eq!(c.label(SPRINT), "LSHIFT");
    assert!(warnings.iter().any(|w| w.contains("future.thing")));
    // the unknown entry is carried through a save, not dropped
    assert!(c.export_settings().entries.contains_key("future.thing"));
}

#[test]
fn stored_conflicts_are_dropped_but_a_valid_swap_applies() {
    let mut c = Controls::default();
    let swap = ControlSettings::parse(
        r#"{"version":1,"bindings":{"onfoot.move_forward":["KeyS","ArrowDown"],"onfoot.move_back":["KeyW","ArrowUp"]}}"#,
    );
    assert!(c.apply_settings(&swap.settings).is_empty());
    assert_eq!(c.label(MOVE_FORWARD), "S");
    assert_eq!(c.label(MOVE_BACK), "W");
    // a collision with a default that is not itself overridden
    let clash =
        ControlSettings::parse(r#"{"version":1,"bindings":{"onfoot.move_forward":["KeyD"]}}"#);
    let warnings = c.apply_settings(&clash.settings);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(
        c.label(MOVE_FORWARD),
        "W",
        "defaults restored for the dropped entry"
    );
    assert!(c.all_conflicts().is_empty());
}

#[test]
fn a_future_version_file_is_never_overwritten() {
    let dir = scratch("future");
    let path = dir.join("controls.json");
    let text = format!(
        r#"{{"version":{},"bindings":{{"onfoot.move_forward":["KeyI"]}},"shiny":true}}"#,
        SETTINGS_VERSION + 1
    );
    std::fs::write(&path, &text).unwrap();
    let (mut store, out) = SettingsStore::load(path.clone());
    assert_eq!(out.status, LoadStatus::FutureVersion(SETTINGS_VERSION + 1));
    let mut c = controls();
    c.apply_settings(&out.settings);
    assert_eq!(c.label(MOVE_FORWARD), "W", "defaults are usable");
    c.remap(
        MOVE_FORWARD,
        0,
        Some(Binding::key(KeyCode::KeyP)),
        ConflictPolicy::Reject,
    )
    .unwrap();
    let err = store.save(&c.export_settings()).unwrap_err();
    assert!(matches!(err, SaveError::FutureVersion(_)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert!(store.last_error.is_some());
    assert_eq!(
        c.label(MOVE_FORWARD),
        "P",
        "this session's remap still works"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_failed_write_keeps_the_old_file_and_the_current_settings() {
    let dir = scratch("failwrite");
    let path = dir.join("controls.json");
    let mut c = controls();
    c.remap(
        MOVE_FORWARD,
        0,
        Some(Binding::key(KeyCode::KeyI)),
        ConflictPolicy::Reject,
    )
    .unwrap();
    let (mut store, _) = SettingsStore::load(path.clone());
    store.save(&c.export_settings()).unwrap();
    let good = std::fs::read_to_string(&path).unwrap();

    // the temporary file cannot be created: a directory stands where it would go
    std::fs::create_dir(dir.join("controls.json.tmp")).unwrap();
    c.remap(
        MOVE_FORWARD,
        0,
        Some(Binding::key(KeyCode::KeyK)),
        ConflictPolicy::Reject,
    )
    .unwrap();
    let err = store.save(&c.export_settings());
    assert!(matches!(err, Err(SaveError::Io(_))));
    assert!(store.last_error.is_some());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        good,
        "old file intact"
    );
    assert_eq!(c.label(MOVE_FORWARD), "K", "in-memory settings intact");
    // and the unwritable location: the parent is a file
    std::fs::write(dir.join("afile"), "x").unwrap();
    let blocked = dir.join("afile").join("sub").join("c.json");
    let (mut store, _) = SettingsStore::load(blocked);
    assert!(store.save(&c.export_settings()).is_err());
    // an in-memory store reports it has nowhere to write
    let mut mem = SettingsStore::in_memory();
    assert!(matches!(
        mem.save(&c.export_settings()),
        Err(SaveError::NoPath)
    ));
    let _ = std::fs::remove_dir_all(dir);
}

// -- Bevy integration ------------------------------------------------------------------

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_plugins(ControlsPlugin::default());
    app.world_mut()
        .resource_mut::<Controls>()
        .register(def(PULSE, Context::Global, KeyCode::F5));
    app
}

fn tick(app: &mut App, dt: Duration) {
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(dt));
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .bypass_change_detection()
        .clear();
}

#[derive(Resource, Default)]
struct Seen {
    steps: u32,
    presses: u32,
}

fn fixed_consumer(mut c: ResMut<Controls>, mut seen: ResMut<Seen>) {
    seen.steps += 1;
    if c.take_pressed(PULSE) {
        seen.presses += 1;
    }
}

#[test]
fn dispatch_runs_before_the_fixed_step_and_a_press_reaches_it_once() {
    let mut app = app();
    app.init_resource::<Seen>()
        .add_systems(FixedUpdate, fixed_consumer);
    for _ in 0..3 {
        tick(&mut app, Duration::from_millis(1));
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::F5);
    // quarter-step frames: no fixed step runs yet, the press waits
    let before = app.world().resource::<Seen>().steps;
    tick(&mut app, Duration::from_millis(2));
    tick(&mut app, Duration::from_millis(2));
    assert_eq!(app.world().resource::<Seen>().steps, before);
    assert!(app.world().resource::<Controls>().down(PULSE));
    // one long frame: several fixed steps; exactly one sees the edge
    tick(&mut app, Duration::from_millis(80));
    let seen = app.world().resource::<Seen>();
    assert!(seen.steps - before >= 3, "steps {}", seen.steps - before);
    assert_eq!(seen.presses, 1);
    for _ in 0..5 {
        tick(&mut app, Duration::from_millis(40));
    }
    assert_eq!(app.world().resource::<Seen>().presses, 1, "held: no repeat");
}

#[test]
fn the_virtual_clock_the_modal_owner_stops_neutralises_the_controls() {
    let mut app = app();
    tick(&mut app, Duration::from_millis(16));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::F5);
    tick(&mut app, Duration::from_millis(16));
    assert!(app.world().resource::<Controls>().down(PULSE));
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    tick(&mut app, Duration::from_millis(16));
    let c = app.world().resource::<Controls>();
    assert!(!c.down(PULSE));
    assert_eq!(c.why_neutralized(), Some(Neutralize::Modal));
    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
    tick(&mut app, Duration::from_millis(16));
    let c = app.world().resource::<Controls>();
    assert!(c.why_neutralized().is_none());
    assert!(
        !c.down(PULSE),
        "F5 is still held from before: it must be re-pressed"
    );
}

#[test]
fn the_map_and_market_modals_and_a_cancel_message_neutralise() {
    let mut app = app();
    app.init_resource::<crate::map::MapState>();
    tick(&mut app, Duration::from_millis(16));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::F5);
    tick(&mut app, Duration::from_millis(16));
    assert!(app.world().resource::<Controls>().down(PULSE));
    app.world_mut().resource_mut::<crate::map::MapState>().open = true;
    tick(&mut app, Duration::from_millis(16));
    assert!(!app.world().resource::<Controls>().down(PULSE));
    app.world_mut().resource_mut::<crate::map::MapState>().open = false;
    tick(&mut app, Duration::from_millis(16));
    // device cancel
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::F5);
    tick(&mut app, Duration::from_millis(16));
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::F5);
    tick(&mut app, Duration::from_millis(16));
    assert!(app.world().resource::<Controls>().down(PULSE));
    app.world_mut().write_message(ControlsCancel);
    tick(&mut app, Duration::from_millis(16));
    assert_eq!(
        app.world().resource::<Controls>().why_neutralized(),
        Some(Neutralize::Cancel)
    );
    assert!(!app.world().resource::<Controls>().down(PULSE));
}

/// The migrated path end to end: a remapped movement action moves the hero input, the old
/// raw key no longer does, the prompt label follows the binding, and death neutralises.
#[test]
fn a_remapped_movement_action_reaches_the_character_input() {
    use crate::character::{Hero, HeroInput, OrbitCamera, hero_input};
    use crate::vehicle::CameraMode;
    use bevy::input::mouse::AccumulatedMouseMotion;

    let mut app = app();
    app.init_resource::<AccumulatedMouseMotion>()
        .init_resource::<HeroInput>()
        .init_resource::<OrbitCamera>()
        .insert_resource(CameraMode::Hero)
        .add_systems(Update, hero_input);
    let hero = app.world_mut().spawn(Hero::default()).id();
    {
        let mut c = app.world_mut().resource_mut::<Controls>();
        c.remap(
            MOVE_FORWARD,
            0,
            Some(Binding::key(KeyCode::KeyI)),
            ConflictPolicy::Reject,
        )
        .unwrap();
        c.remap(MOVE_FORWARD, 1, None, ConflictPolicy::Reject)
            .unwrap();
        assert_eq!(c.label(MOVE_FORWARD), "I");
    }
    tick(&mut app, Duration::from_millis(16));
    // W (the old key) and Up do nothing now
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.press(KeyCode::KeyW);
    keys.press(KeyCode::ArrowUp);
    tick(&mut app, Duration::from_millis(16));
    assert_eq!(app.world().resource::<HeroInput>().movement, Vec3::ZERO);
    // I walks forward (-z at yaw 0), and Shift sprints through the same service
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.release(KeyCode::KeyW);
    keys.release(KeyCode::ArrowUp);
    keys.press(KeyCode::KeyI);
    keys.press(KeyCode::ShiftLeft);
    tick(&mut app, Duration::from_millis(16));
    {
        let input = app.world().resource::<HeroInput>();
        assert!(input.movement.z < -0.9, "{:?}", input.movement);
        assert!(input.sprint);
        assert_eq!(
            input.chute_pull, 1.0,
            "the parachute pulls on the same action"
        );
    }
    // remap sprint away from Shift: Shift stops sprinting
    app.world_mut()
        .resource_mut::<Controls>()
        .remap(SPRINT, 0, None, ConflictPolicy::Reject)
        .unwrap();
    app.world_mut()
        .resource_mut::<Controls>()
        .remap(
            SPRINT,
            1,
            Some(Binding::key(KeyCode::KeyG)),
            ConflictPolicy::Reject,
        )
        .unwrap();
    tick(&mut app, Duration::from_millis(16));
    assert!(!app.world().resource::<HeroInput>().sprint);
    // he goes down: everything neutral, and I must be released and pressed again
    app.world_mut().get_mut::<Hero>(hero).unwrap().downed = true;
    tick(&mut app, Duration::from_millis(16));
    assert_eq!(app.world().resource::<HeroInput>().movement, Vec3::ZERO);
    assert_eq!(
        app.world().resource::<Controls>().why_neutralized(),
        Some(Neutralize::Death)
    );
    app.world_mut().get_mut::<Hero>(hero).unwrap().downed = false;
    tick(&mut app, Duration::from_millis(16));
    assert_eq!(
        app.world().resource::<HeroInput>().movement,
        Vec3::ZERO,
        "I is still held from before he went down"
    );
}
