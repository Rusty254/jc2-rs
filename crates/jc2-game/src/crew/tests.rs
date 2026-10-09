use super::*;
use crate::fleet::Place;
use crate::fleet::testing::{spawn_crewed, wait_for_hero, wait_until};
use crate::stunts::StuntSpots;
use crate::stunts::tests::{car_pose, hero, press, put_hero, release, rico_app, settle_car, step};
use jc2_formats::vehicle::StuntKind;

fn skipper() -> BoatPilot {
    BoatPilot {
        throttle: 0.4,
        rudder: 0.7,
        avoidance: 0.0,
        probe_in: 0.0,
    }
}

#[test]
fn boat_probes_exclude_nested_own_colliders_but_keep_obstacles() {
    #[derive(Resource)]
    struct ProbeEntities {
        boat: Entity,
        own: Entity,
        obstacle: Entity,
    }
    #[derive(Resource, Default)]
    struct ProbeResults {
        own_unfiltered: Option<Entity>,
        own_filtered: Option<Entity>,
        obstacle: Option<Entity>,
    }
    fn probe(
        entities: Res<ProbeEntities>,
        children: Query<&Children>,
        spatial: SpatialQuery,
        mut results: ResMut<ProbeResults>,
    ) {
        let cast = |z, filter: &SpatialQueryFilter| {
            spatial
                .cast_ray(Vec3::new(0.0, 20.0, z), Dir3::NEG_Y, 40.0, true, filter)
                .map(|hit| hit.entity)
        };
        let filter = boat_probe_filter(entities.boat, &children);
        results.own_unfiltered = cast(-40.0, &SpatialQueryFilter::default());
        results.own_filtered = cast(-40.0, &filter);
        results.obstacle = cast(-60.0, &filter);
    }
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::asset::AssetPlugin::default(),
        bevy::mesh::MeshPlugin,
        PhysicsPlugins::default(),
    ))
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0 / 64.0),
    ))
    .init_resource::<ProbeResults>()
    .add_systems(Update, probe);
    let boat = app
        .world_mut()
        .spawn((
            RigidBody::Static,
            Collider::cuboid(3.0, 3.0, 3.0),
            Transform::IDENTITY,
        ))
        .id();
    let child = app
        .world_mut()
        .spawn((Transform::from_xyz(0.0, 0.0, -20.0), ChildOf(boat)))
        .id();
    let own = app
        .world_mut()
        .spawn((
            Collider::cuboid(3.0, 3.0, 3.0),
            Transform::from_xyz(0.0, 0.0, -20.0),
            ChildOf(child),
        ))
        .id();
    let obstacle = app
        .world_mut()
        .spawn((
            RigidBody::Static,
            Collider::cuboid(3.0, 3.0, 3.0),
            Transform::from_xyz(0.0, 0.0, -60.0),
        ))
        .id();
    app.insert_resource(ProbeEntities {
        boat,
        own,
        obstacle,
    });
    app.finish();
    for _ in 0..5 {
        app.update();
    }
    let entities = app.world().resource::<ProbeEntities>();
    let results = app.world().resource::<ProbeResults>();
    assert_eq!(
        results.own_unfiltered,
        Some(entities.own),
        "probe must actually see the nested own collider without filtering"
    );
    assert_eq!(
        results.own_filtered, None,
        "own child collider is not a shore"
    );
    assert_eq!(
        results.obstacle,
        Some(entities.obstacle),
        "independent obstacle must remain visible"
    );
}

#[test]
fn boat_navigation_keeps_cruise_in_open_water() {
    let mut pilot = skipper();
    pilot.avoidance = -1.0;
    assert_eq!(
        navigate_boat(&mut pilot, Vec3::ZERO, Vec3::NEG_Z, 10.0, 8.0, |_| true),
        (0.4, 0.7)
    );
    assert_eq!(pilot.avoidance, 0.0);
}

#[test]
fn boat_navigation_turns_early_and_holds_its_escape_side() {
    let mut pilot = skipper();
    let coast = |point: Vec3| point.z > -45.0;
    let (throttle, rudder) = navigate_boat(&mut pilot, Vec3::ZERO, Vec3::NEG_Z, 10.0, 6.0, coast);
    assert!(throttle > 0.0 && throttle < pilot.throttle);
    assert_eq!(rudder, 1.0);
    // A tie used to be free to choose either side each update. Keep a previous right
    // escape even when the next probe's two alternatives have equal clearance.
    pilot.avoidance = -1.0;
    let (_, rudder) = navigate_boat(&mut pilot, Vec3::ZERO, Vec3::NEG_Z, 10.0, 6.0, coast);
    assert_eq!(rudder, -1.0);
}

#[test]
fn boat_navigation_checks_hull_width_and_reverses_out_of_a_corner() {
    let narrow = |point: Vec3| point.x.abs() < 2.0;
    assert_eq!(
        boat_clearance(Vec3::ZERO, Vec3::NEG_Z, 12.0, 50.0, &narrow),
        0.0
    );
    let mut pilot = skipper();
    let corner = |point: Vec3| point.z > -9.0 && point.x < 9.0;
    let (throttle, rudder) = navigate_boat(&mut pilot, Vec3::ZERO, Vec3::NEG_Z, 10.0, 5.0, corner);
    assert!(throttle < 0.0, "must back away from the blocked bow");
    assert_eq!(
        rudder, -pilot.avoidance,
        "reverse thrust needs reverse rudder"
    );
    let (throttle, _) = navigate_boat(&mut pilot, Vec3::ZERO, Vec3::NEG_Z, 10.0, 5.0, |_| false);
    assert_eq!(
        throttle, 0.0,
        "do not power into land when also blocked astern"
    );
}

/// Real boat physics and loaded collision: a detailed shore that is absent from the
/// coarse global map must still be avoided, without grabbing player controls.
#[test]
fn crewed_motorboat_avoids_a_loaded_shore_and_leaves_occupied_controls_alone() {
    use crate::boat::{BoatPlugin, find_open_water};
    use crate::fleet::testing;
    let Some((mut app, _, _)) = testing::app(
        &["seve.v083_motorboat_small.ee", "terrain_win32.dat"],
        |app| {
            app.add_plugins(BoatPlugin).add_systems(Update, boat_pilots);
        },
    ) else {
        return;
    };
    let fs = app.world().resource::<Game>().fs.clone();
    let map = GlobalTerrain::parse(&fs.read_inflated("terrain_win32.dat").unwrap()).unwrap();
    let (x, z) = find_open_water(&map, -15000.0, -15000.0, 500.0, 3.0).unwrap();
    let shore = z - 80.0;
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(1000.0, 100.0, 400.0),
        Transform::from_xyz(x, SEA_LEVEL - 40.0, shore - 200.0),
    ));
    let id = testing::spawn(
        &mut app,
        "seve.v083_motorboat_small.ee",
        Place::At(Transform::from_xyz(x, SEA_LEVEL, z)),
    );
    app.world_mut().entity_mut(id).insert((
        AiDriven,
        BoatPilot {
            rudder: 0.0,
            ..skipper()
        },
    ));
    // The fleet test helper spawns a player-driven boat by default.
    app.world_mut().entity_mut(id).remove::<Occupied>();
    let mut minimum = f32::INFINITY;
    let mut escaped = false;
    let mut turned = 0.0_f32;
    for _ in 0..64 * 30 {
        app.update();
        let position = app.world().get::<Position>(id).unwrap().0;
        minimum = minimum.min(position.z - shore);
        turned += app.world().get::<AngularVelocity>(id).unwrap().y / 64.0;
        escaped |= position.x < x - 15.0;
    }
    eprintln!(
        "boat shore: minimum centre clearance {minimum:.1} m; turned {turned:.2} rad; escaped {escaped}"
    );
    let length = app.world().get::<Watercraft>(id).unwrap().length;
    assert!(
        minimum > length * 0.5 + 2.0,
        "boat reached the shore: {minimum:.1} m"
    );
    assert!(escaped && turned > 0.3, "skipper never steered away");
    app.world_mut().entity_mut(id).insert(Occupied);
    let boat = &mut *app.world_mut().get_mut::<Watercraft>(id).unwrap();
    boat.throttle = 0.0;
    boat.rudder = 0.0;
    app.update();
    let boat = app.world().get::<Watercraft>(id).unwrap();
    assert_eq!((boat.throttle, boat.rudder), (0.0, 0.0));
}

/// Run until the vehicle's driver sits in it (his outfit loads on other threads).
fn wait_for_driver(app: &mut App, vehicle: Entity) -> Entity {
    wait_until(app, "the driver", |app| {
        app.world()
            .get::<Crew>(vehicle)
            .is_some_and(|c| c.driver.is_some())
    });
    app.world().get::<Crew>(vehicle).unwrap().driver.unwrap()
}

/// The jeep with a villager at the wheel: `E` beside it throws him out to the left and
/// seats Rico, who then drives it; the hijack counts.
#[test]
fn hijacking_a_crewed_car() {
    let car_name = "lave.v014_jeep.ee";
    let Some((mut app, ground, _)) = rico_app(&[car_name, CIVILIAN_DOLLS[0]], |app| {
        app.add_plugins((crate::vehicle::VehiclePlugin, CrewPlugin));
    }) else {
        return;
    };
    let car = spawn_crewed(
        &mut app,
        car_name,
        Place::NearStart(Vec3::new(0.0, 0.0, 14.0)),
        Some(CrewKind::Civilian),
    );
    let driver = wait_for_driver(&mut app, car);
    // he sits in it: a child of the car, at the driver's seat (left of the middle)
    assert_eq!(
        app.world().get::<ChildOf>(driver).map(|c| c.parent()),
        Some(car)
    );
    let seat = app.world().get::<Seated>(driver).unwrap().local;
    assert!(seat.x < 0.0, "the driver sits on the left: {seat:?}");
    assert!(app.world().get::<Animator>(driver).is_some());
    wait_for_hero(&mut app);
    settle_car(&mut app, car);
    app.world_mut()
        .resource_mut::<CameraMode>()
        .set_if_neq(CameraMode::Hero);
    let (p, r) = car_pose(&app, car);
    put_hero(&mut app, p + r * Vec3::new(-2.3, 0.6, 0.0));
    step(&mut app, 20);
    press(&mut app, KeyCode::KeyE);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyE);
    step(&mut app, 3);
    let (_, h, _, _) = hero(&mut app);
    assert_eq!(h.in_vehicle, Some(car), "not in the car: {h:?}");
    let w = app.world();
    assert!(w.get::<Occupied>(car).is_some());
    assert!(w.get::<Crew>(car).is_none(), "still crewed");
    assert_eq!(*w.resource::<CameraMode>(), CameraMode::Drive);
    assert_eq!(w.resource::<Hijackings>().count, 1);
    assert_eq!(w.resource::<Hijackings>().last, Some(car));
    assert!(
        w.get::<ChildOf>(driver).is_none(),
        "the driver is still in the car"
    );
    assert!(w.get::<Thrown>(driver).is_some());
    // he comes down outside, on the ground
    for _ in 0..64 * 3 {
        step(&mut app, 1);
    }
    let t = *app.world().get::<Thrown>(driver).unwrap();
    assert!(t.landed.is_some(), "still falling: {t:?}");
    let at = app.world().get::<Transform>(driver).unwrap().translation;
    let (p, r) = car_pose(&app, car);
    let local = r.inverse() * (at - p);
    eprintln!(
        "hijack: the driver lies at {local:?} in the car's frame, {:.2} m over the ground",
        at.y - ground
    );
    assert!(local.x < -1.5, "not thrown out to the left: {local:?}");
    assert!(
        (at.y - ground).abs() < 1.0,
        "not on the ground: {at:?} vs {ground}"
    );
    // Rico drives it
    press(&mut app, KeyCode::KeyW);
    for _ in 0..64 * 3 {
        app.update();
    }
    release(&mut app, KeyCode::KeyW);
    let moved = (car_pose(&app, car).0 - p).length();
    assert!(moved > 4.0, "the car only moved {moved:.1} m");
    // the thrown driver goes after a while
    for _ in 0..(LIE_SECONDS * 64.0) as usize + 64 {
        step(&mut app, 1);
    }
    assert!(
        app.world().get_entity(driver).is_err(),
        "the driver is still lying there"
    );
}

/// A crewed chopper: `E` from the ground does nothing (a pilot is pulled out only from a
/// stunt position); Rico hangs under its belly (`Q`) while its pilot takes off and
/// circles; `E` there throws the pilot out and seats Rico; the hijack counts.
#[test]
fn hijacking_a_crewed_helicopter_from_its_stunt_position() {
    let name = "arve.v062_chopper.ee";
    let Some((mut app, ground, _)) = rico_app(&[name, SOLDIER_DOLL], |app| {
        app.add_plugins((crate::heli::HeliPlugin, CrewPlugin));
    }) else {
        return;
    };
    let heli = spawn_crewed(
        &mut app,
        name,
        Place::NearStart(Vec3::new(0.0, 0.0, 16.0)),
        Some(CrewKind::Soldier),
    );
    // the pilot takes the controls at once; hold it on the ground for now
    step(&mut app, 2);
    assert!(app.world().get::<HeliPilot>(heli).is_some());
    app.world_mut().entity_mut(heli).remove::<HeliPilot>();
    let pilot = wait_for_driver(&mut app, heli);
    wait_for_hero(&mut app);
    step(&mut app, 64);
    app.world_mut()
        .resource_mut::<CameraMode>()
        .set_if_neq(CameraMode::Hero);
    // beside the door: E does nothing
    let (p, r) = car_pose(&app, heli);
    put_hero(&mut app, p + r * Vec3::new(-3.0, 0.0, -3.0));
    step(&mut app, 20);
    press(&mut app, KeyCode::KeyE);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyE);
    step(&mut app, 3);
    let (_, h, _, _) = hero(&mut app);
    assert_eq!(
        h.in_vehicle, None,
        "got into a crewed helicopter from the ground"
    );
    assert!(app.world().get::<Crew>(heli).is_some());
    assert_eq!(app.world().resource::<Hijackings>().count, 0);
    // Q: under the belly
    let spots = app.world().get::<StuntSpots>(heli).unwrap().0.clone();
    let belly = spots
        .iter()
        .find(|s| s.kind == StuntKind::Belly)
        .unwrap()
        .clone();
    let (p, r) = car_pose(&app, heli);
    put_hero(&mut app, p + r * (belly.local + Vec3::new(2.5, 1.0, 0.0)));
    step(&mut app, 20);
    press(&mut app, KeyCode::KeyQ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyQ);
    step(&mut app, 3);
    assert_eq!(
        hero(&mut app).1.stunt,
        Some(heli),
        "not under the helicopter"
    );
    // the pilot flies off with him for 12 s
    let (p0, _) = car_pose(&app, heli);
    app.world_mut()
        .entity_mut(heli)
        .insert(HeliPilot::circling(p0, 60.0, 35.0));
    for _ in 0..64 * 12 {
        step(&mut app, 1);
    }
    let (p1, _) = car_pose(&app, heli);
    eprintln!("the pilot flew to {:?} from {p0:?}", p1 - p0);
    assert!(p1.y > p0.y + 15.0, "the pilot did not take off: {p1:?}");
    assert_eq!(hero(&mut app).1.stunt, Some(heli));
    // E: the pilot goes out of the door, Rico takes the stick
    press(&mut app, KeyCode::KeyE);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyE);
    step(&mut app, 3);
    let (e, h, _, _) = hero(&mut app);
    assert_eq!(h.in_vehicle, Some(heli), "not in the helicopter: {h:?}");
    assert_eq!(h.stunt, None);
    let w = app.world();
    assert!(w.get::<crate::stunts::OnStunt>(e).is_none());
    assert!(w.get::<Occupied>(heli).is_some());
    assert!(w.get::<Crew>(heli).is_none());
    assert!(w.get::<HeliPilot>(heli).is_none());
    assert!(w.get::<AiDriven>(heli).is_none());
    assert_eq!(*w.resource::<CameraMode>(), CameraMode::Drive);
    assert_eq!(w.resource::<Hijackings>().count, 1);
    assert!(w.get::<Thrown>(pilot).is_some());
    // the pilot falls to the ground
    let mut landed = false;
    for _ in 0..64 * 8 {
        step(&mut app, 1);
        if app
            .world()
            .get::<Thrown>(pilot)
            .is_some_and(|t| t.landed.is_some())
        {
            landed = true;
            break;
        }
    }
    assert!(landed, "the pilot never came down");
    let at = app.world().get::<Transform>(pilot).unwrap().translation;
    assert!(
        (at.y - ground).abs() < 1.0,
        "landed at {at:?} over ground {ground}"
    );
    // Rico flies it: Space climbs
    let y = car_pose(&app, heli).0.y;
    press(&mut app, KeyCode::Space);
    for _ in 0..64 * 2 {
        app.update();
    }
    release(&mut app, KeyCode::Space);
    assert!(car_pose(&app, heli).0.y > y + 5.0, "Rico can't fly it");
}
