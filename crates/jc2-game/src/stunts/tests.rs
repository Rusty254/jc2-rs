use super::*;
use crate::character::{CharacterPlugin, Grapple};
use crate::fleet::Place;
use crate::vehicle::Vehicle;
use bevy::input::mouse::AccumulatedMouseMotion;

// ---- tethers: plain boxes, no game data needed

fn tether_app() -> App {
    // headless_app() with the plugin added before the app is finished
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::asset::AssetPlugin::default(),
        bevy::mesh::MeshPlugin,
        bevy::scene::ScenePlugin,
        PhysicsPlugins::default(),
    ))
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0 / 64.0),
    ))
    .insert_resource(Gravity(Vec3::NEG_Y * 9.81))
    .add_plugins(TetherPlugin);
    app.finish();
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(400.0, 2.0, 400.0),
        Transform::from_xyz(0.0, -1.0, 0.0),
        Name::new("ground"),
    ));
    app
}

fn crate_at(app: &mut App, p: Vec3) -> Entity {
    app.world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(1.0, 1.0, 1.0),
            Transform::from_translation(p),
            Name::new("crate"),
        ))
        .id()
}

fn shoot(app: &mut App, e: Entity, point: Vec3) {
    app.world_mut()
        .resource_mut::<TetherShots>()
        .0
        .push(TetherShot { entity: e, point });
}

fn shoot_body(app: &mut App, e: Entity) {
    let p = pos(app, e);
    shoot(app, e, p);
}

fn pos(app: &App, e: Entity) -> Vec3 {
    app.world().get::<Position>(e).unwrap().0
}

fn count<T: Component>(app: &mut App) -> usize {
    app.world_mut().query::<&T>().iter(app.world()).count()
}

fn run(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.update();
    }
}

/// Two boxes 20 m apart, tied at their centres: the cable winds in and they end up
/// together, inside the minimum length (plus the boxes' own size).
#[test]
fn tethered_boxes_are_pulled_together() {
    let mut app = tether_app();
    let a = crate_at(&mut app, Vec3::new(-10.0, 0.5, 0.0));
    let b = crate_at(&mut app, Vec3::new(10.0, 0.5, 0.0));
    run(&mut app, 5);
    shoot_body(&mut app, a);
    shoot_body(&mut app, b);
    run(&mut app, 2);
    assert_eq!(count::<Tether>(&mut app), 1);
    assert_eq!(count::<DistanceJoint>(&mut app), 1);
    run(&mut app, 64 * 6);
    let d = pos(&app, a).distance(pos(&app, b));
    assert!(d < TETHER_MIN + 0.3, "still {d:.2} m apart");
    assert!(d > 0.9, "inside each other: {d:.2}");
    // they met in the middle (equal masses)
    let mid = (pos(&app, a) + pos(&app, b)) / 2.0;
    assert!(mid.x.abs() < 2.0, "met at {mid:?}");
}

/// A body that goes takes its cable with it, and the fixed point made for a static end.
#[test]
fn despawned_bodies_leave_no_joint() {
    let mut app = tether_app();
    let a = crate_at(&mut app, Vec3::new(-6.0, 0.5, 0.0));
    let b = crate_at(&mut app, Vec3::new(6.0, 0.5, 0.0));
    let c = crate_at(&mut app, Vec3::new(0.0, 0.5, 8.0));
    run(&mut app, 5);
    shoot_body(&mut app, a);
    shoot_body(&mut app, b);
    // c to the ground (a static end: a fixed point is made)
    let floor = {
        let mut q = app
            .world_mut()
            .query_filtered::<(Entity, &RigidBody), With<Collider>>();
        q.iter(app.world())
            .find(|(_, rb)| **rb == RigidBody::Static)
            .map(|(e, _)| e)
            .unwrap()
    };
    shoot_body(&mut app, c);
    shoot(&mut app, floor, Vec3::new(0.0, 0.0, 12.0));
    run(&mut app, 30);
    assert_eq!(count::<DistanceJoint>(&mut app), 2);
    assert_eq!(count::<TetherAnchor>(&mut app), 1);
    // despawn one of the pair
    app.world_mut().entity_mut(a).despawn();
    run(&mut app, 3);
    assert_eq!(count::<DistanceJoint>(&mut app), 1, "joint to a dead body");
    assert_eq!(count::<Tether>(&mut app), 1);
    // despawn the box tied to the ground: the cable and its point go
    app.world_mut().entity_mut(c).despawn();
    run(&mut app, 3);
    assert_eq!(count::<DistanceJoint>(&mut app), 0);
    assert_eq!(count::<Tether>(&mut app), 0);
    assert_eq!(count::<TetherAnchor>(&mut app), 0, "dangling anchor");
    // the other crate is still there and the world still steps
    assert!(app.world().get_entity(b).is_ok());
    run(&mut app, 30);
    // a static thing that is despawned (a tile unloading) ends its cable too
    let d = crate_at(&mut app, Vec3::new(0.0, 0.5, -8.0));
    let wall = app
        .world_mut()
        .spawn((
            RigidBody::Static,
            Collider::cuboid(4.0, 4.0, 1.0),
            Transform::from_xyz(0.0, 2.0, -30.0),
        ))
        .id();
    run(&mut app, 3);
    shoot_body(&mut app, d);
    shoot(&mut app, wall, Vec3::new(0.0, 2.0, -29.5));
    run(&mut app, 10);
    assert_eq!(count::<Tether>(&mut app), 1);
    assert_eq!(count::<TetherAnchor>(&mut app), 1);
    app.world_mut().entity_mut(wall).despawn();
    run(&mut app, 3);
    assert_eq!(count::<Tether>(&mut app), 0);
    assert_eq!(count::<DistanceJoint>(&mut app), 0);
    assert_eq!(count::<TetherAnchor>(&mut app), 0, "dangling anchor");
}

/// Three at most (the oldest goes), `X` clears them, and a cable stretched too far snaps.
#[test]
fn tether_limits_and_clearing() {
    let mut app = tether_app();
    let boxes: Vec<Entity> = (0..5)
        .map(|i| crate_at(&mut app, Vec3::new(-8.0 + 4.0 * i as f32, 0.5, 0.0)))
        .collect();
    run(&mut app, 5);
    for pair in [(0, 1), (1, 2), (2, 3), (3, 4)] {
        shoot_body(&mut app, boxes[pair.0]);
        shoot_body(&mut app, boxes[pair.1]);
        run(&mut app, 2);
    }
    assert_eq!(count::<Tether>(&mut app), MAX_TETHERS);
    let serials: Vec<u64> = app
        .world_mut()
        .query::<&Tether>()
        .iter(app.world())
        .map(|t| t.serial)
        .collect();
    assert!(
        serials.iter().all(|s| *s >= 2),
        "oldest should be gone: {serials:?}"
    );
    // a lone first shot is only pending; X forgets it too
    shoot_body(&mut app, boxes[0]);
    run(&mut app, 2);
    assert!(app.world().resource::<TetherState>().pending.is_some());
    app.world_mut().resource_mut::<TetherState>().clear = true;
    run(&mut app, 3);
    assert_eq!(count::<Tether>(&mut app), 0);
    assert_eq!(count::<DistanceJoint>(&mut app), 0);
    assert!(app.world().resource::<TetherState>().pending.is_none());
    // snap: one end flung 40 m away
    let (a, b) = (boxes[0], boxes[4]);
    run(&mut app, 20);
    shoot_body(&mut app, a);
    shoot_body(&mut app, b);
    run(&mut app, 3);
    assert_eq!(count::<Tether>(&mut app), 1);
    let far = pos(&app, b) + Vec3::new(40.0, 0.0, 0.0);
    app.world_mut()
        .entity_mut(b)
        .insert((Position(far), Transform::from_translation(far)));
    run(&mut app, 2);
    assert_eq!(count::<Tether>(&mut app), 0, "should have snapped");
    assert_eq!(count::<DistanceJoint>(&mut app), 0);
}

// ---- the roof: needs Rico and the jeep

pub(crate) fn press(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
}
pub(crate) fn release(app: &mut App, key: KeyCode) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(key);
}
pub(crate) fn step(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.bypass_change_detection().clear();
    }
}
pub(crate) fn hero(app: &mut App) -> (Entity, Hero, Vec3, Vec3) {
    let mut q = app
        .world_mut()
        .query::<(Entity, &Hero, &Position, &LinearVelocity)>();
    let (e, h, p, v) = q.single(app.world()).unwrap();
    (e, h.clone(), p.0, v.0)
}
/// Step until the car has been at rest for a while (at most 20 s of game time).
pub(crate) fn settle_car(app: &mut App, car: Entity) {
    let mut still = 0;
    for _ in 0..64 * 20 {
        step(app, 1);
        let w = app.world();
        let calm = w.get::<LinearVelocity>(car).unwrap().length() < 0.25
            && w.get::<AngularVelocity>(car).unwrap().length() < 0.25;
        still = if calm { still + 1 } else { 0 };
        if still >= 30 {
            return;
        }
    }
    panic!("the car never came to rest");
}
pub(crate) fn car_pose(app: &App, car: Entity) -> (Vec3, Quat) {
    (
        app.world().get::<Position>(car).unwrap().0,
        app.world().get::<Rotation>(car).unwrap().0,
    )
}
pub(crate) fn put_hero(app: &mut App, at: Vec3) {
    let e = hero(app).0;
    app.world_mut().entity_mut(e).insert((
        Transform::from_translation(at),
        Position(at),
        LinearVelocity::ZERO,
    ));
}

/// Rico on the jeep's roof stays on it through 50 m of driving and turning, jumps off
/// with the car's velocity, and the hook lands him on it again.
#[test]
fn rico_rides_the_roof() {
    let Some((mut app, _, spawn)) =
        crate::fleet::testing::app(&["mc01_lod1-rico.rbm", "lave.v014_jeep.ee"], |app| {
            app.init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<AccumulatedMouseMotion>()
                .add_systems(FixedUpdate, crate::vehicle::vehicle_physics)
                .add_plugins((CharacterPlugin, StuntPlugin));
        })
    else {
        return;
    };
    app.world_mut().spawn((
        Transform::from_translation(spawn + Vec3::new(0.0, 4.0, -8.0)),
        FlyCamera {
            yaw: 0.0,
            pitch: 0.0,
            speed: 1.0,
        },
    ));
    let car = crate::fleet::testing::spawn(
        &mut app,
        "lave.v014_jeep.ee",
        Place::NearStart(Vec3::new(0.0, 0.0, 14.0)),
    );
    // the doll's stunt positions arrived with the car
    let spots = app
        .world()
        .get::<StuntSpots>(car)
        .expect("stunt spots")
        .0
        .clone();
    assert_eq!(spots.len(), 3, "{spots:?}");
    let roof = spots.iter().find(|s| s.name == "SO_Hardtop_RD").unwrap();
    assert!(
        (roof.local - to_bevy([0.005, 1.864, 0.284])).length() < 0.02,
        "{roof:?}"
    );
    let hood = spots.iter().find(|s| s.name == "SO_Hood_RD").unwrap();
    assert!(
        (hood.local.z - roof.local.z).abs() > 1.0,
        "hood and roof are apart"
    );
    // Rico's model loads on other threads, so he appears after some number of frames
    // that depends on the machine's load; wait for him by the wall clock, then let the
    // car come to rest before anything is measured, so the frame count is not part of
    // the test.
    crate::fleet::testing::wait_for_hero(&mut app);
    settle_car(&mut app, car);
    assert!(hero(&mut app).1.grounded, "rico never landed");
    // the roof sits near the top of the chassis shape
    {
        let aabb = *app.world().get::<ColliderAabb>(car).unwrap();
        let (p, r) = car_pose(&app, car);
        let top = (p + r * roof.local).y;
        eprintln!("roof point y {top:.2}, chassis aabb top {:.2}", aabb.max.y);
        assert!((top - aabb.max.y).abs() < 1.0);
    }
    app.world_mut()
        .resource_mut::<CameraMode>()
        .set_if_neq(CameraMode::Hero);
    // beside the jeep: Q gets him up
    let (p, r) = car_pose(&app, car);
    put_hero(&mut app, p + r * Vec3::new(2.5, 1.0, 0.0));
    step(&mut app, 30);
    press(&mut app, KeyCode::KeyQ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyQ);
    step(&mut app, 3);
    let (e, h, hp, _) = hero(&mut app);
    assert_eq!(h.stunt, Some(car), "not on the roof: {h:?} at {hp:?}");
    let at_roof = |app: &mut App| {
        let (e, _, hp, _) = hero(app);
        let (p, r) = car_pose(app, car);
        let on = *app.world().get::<OnStunt>(e).unwrap();
        (hp - (p + r * on.local)).length()
    };
    assert!(at_roof(&mut app) < 0.01);
    // he is inert: no collision
    assert_eq!(
        *app.world().get::<CollisionLayers>(e).unwrap(),
        CollisionLayers::NONE
    );
    // Drive 50 m with the car's own pedals and steering, turning a little. (The test
    // used to force the chassis' velocity every step with no suspension in the app, so
    // the bare chassis box scraped over the ground with a velocity it could not keep,
    // pitching and rolling at up to 3 rad/s: how it ended depended on the last bit of
    // its starting state, which the asset loading on other threads shifted, and now and
    // then it tumbled.)
    let start = car_pose(&app, car).0;
    let mut worst = 0.0f32;
    let mut moved = 0.0;
    for i in 0..1200 {
        {
            let mut v = app.world_mut().get_mut::<Vehicle>(car).unwrap();
            let throttle = if v.speed < 13.0 {
                (i as f32 / 32.0).min(1.0)
            } else {
                0.0
            };
            v.throttle = throttle;
            v.steer_input = if i > 120 { 0.12 } else { 0.0 };
        }
        step(&mut app, 1);
        worst = worst.max(at_roof(&mut app));
        // the drawn positions agree too
        let hero_tf = app.world().get::<Transform>(e).unwrap().translation;
        let car_tf = *app.world().get::<Transform>(car).unwrap();
        let on = *app.world().get::<OnStunt>(e).unwrap();
        worst = worst.max((hero_tf - (car_tf.translation + car_tf.rotation * on.local)).length());
        moved = (car_pose(&app, car).0 - start).length();
        if moved > 50.0 {
            break;
        }
    }
    assert!(moved > 50.0, "the car only moved {moved:.1} m");
    assert!(worst < 0.2, "drifted {worst:.3} m from the roof");
    let (_, h, _, v) = hero(&mut app);
    assert_eq!(h.stunt, Some(car));
    assert!(v.length() > 8.0, "should ride at the car's speed: {v:?}");
    // he crouches at speed
    if app.world().get_resource::<crate::rig::Rig>().is_some() {
        let mut q = app.world_mut().query::<&crate::rig::Animator>();
        let clip = q.single(app.world()).unwrap().clip;
        assert!(
            matches!(clip, crate::rig::Clip::Crouch | crate::rig::Clip::Idle),
            "{clip:?}"
        );
    }
    // jump off: he keeps the car's velocity
    // the roof's velocity: the car's plus its spin about the roof point (a bouncing car
    // rocks, so this differs from its centre's by a few m/s)
    let car_v = {
        let (p, _) = car_pose(&app, car);
        let at = hero(&mut app).2;
        let w = app.world();
        w.get::<LinearVelocity>(car).unwrap().0
            + w.get::<AngularVelocity>(car).unwrap().0.cross(at - p)
    };
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    step(&mut app, 1);
    let first = hero(&mut app).3;
    step(&mut app, 2);
    let (_, h, _, v) = hero(&mut app);
    eprintln!("jump-off velocity: first step {first:?}, then {v:?}; car {car_v:?}");
    assert_eq!(h.stunt, None, "still on the roof");
    assert!(app.world().get::<OnStunt>(e).is_none());
    let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
    assert!(
        (flat(first) - flat(car_v)).length() < 1.0,
        "jumped off at {first:?}, the car does {car_v:?}"
    );
    assert!(v.y > 3.0, "no jump: {v:?}");
    assert_ne!(
        *app.world().get::<CollisionLayers>(e).unwrap(),
        CollisionLayers::NONE,
        "collides again"
    );
    // let the car stop (brakes on) and him land
    for _ in 0..400 {
        app.world_mut().get_mut::<Vehicle>(car).unwrap().brake = 1.0;
        step(&mut app, 1);
    }
    settle_car(&mut app, car);
    // hook the car from a few metres: he is reeled up onto it
    let (p, r) = car_pose(&app, car);
    let target = p + r * roof.local;
    put_hero(&mut app, p + r * Vec3::new(5.0, 0.9, 0.0));
    step(&mut app, 5);
    // fired for real: the camera on the roof, `F`; the hook flies out, takes hold of the car
    // (a vehicle: the wire follows it) and reels him up
    {
        let w = app.world_mut();
        let hp = w.query::<(&Hero, &Position)>().single(w).unwrap().1.0;
        let mut q = w.query_filtered::<&mut Transform, With<FlyCamera>>();
        let mut cam = q.single_mut(w).unwrap();
        cam.translation = hp + Vec3::new(0.0, 1.5, 3.5) + r * Vec3::X * 0.0;
        cam.look_at(target, Vec3::Y);
    }
    press(&mut app, KeyCode::KeyF);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyF);
    let mut up = false;
    for _ in 0..300 {
        step(&mut app, 1);
        if hero(&mut app).1.stunt.is_some() {
            up = true;
            break;
        }
    }
    assert!(
        up,
        "the hook never landed him on the roof: {:?}",
        hero(&mut app)
    );
    step(&mut app, 5);
    assert!(at_roof(&mut app) < 0.01);
    // and E takes him into the seat
    press(&mut app, KeyCode::KeyE);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyE);
    step(&mut app, 3);
    let (_, h, _, _) = hero(&mut app);
    assert_eq!(h.stunt, None);
    assert_eq!(h.in_vehicle, Some(car), "E on the roof should seat him");
    eprintln!("rico: rode the roof 50 m, jumped off with the car's speed, hooked back on");
}

/// Rico killed on a roof or in the seat comes out of the car (collision and gravity back,
/// visible, the car empty, the camera his) and then gets up at the start: before, he
/// respawned still "in" the vehicle.
#[test]
fn rico_dying_in_a_car_or_on_its_roof_gets_out() {
    use crate::combat::{CombatPlugin, Health};
    use crate::physics::Stowed;
    use crate::vehicle::{CameraMode, Occupied};
    let Some((mut app, _, spawn)) = crate::fleet::testing::app(
        &[
            "mc01_lod1-rico.rbm",
            "lave.v014_jeep.ee",
            "weapons.bin",
            "weapon_upgrades.bin",
            "ammunition.bin",
        ],
        |app| {
            app.init_resource::<ButtonInput<MouseButton>>()
                .init_resource::<AccumulatedMouseMotion>()
                .add_plugins((CharacterPlugin, StuntPlugin, CombatPlugin));
        },
    ) else {
        return;
    };
    app.world_mut().spawn((
        Transform::from_translation(spawn + Vec3::new(0.0, 4.0, -8.0)),
        FlyCamera {
            yaw: 0.0,
            pitch: 0.0,
            speed: 1.0,
        },
    ));
    let car = crate::fleet::testing::spawn(
        &mut app,
        "lave.v014_jeep.ee",
        Place::NearStart(Vec3::new(0.0, 0.0, 14.0)),
    );
    crate::fleet::testing::wait_for_hero(&mut app);
    settle_car(&mut app, car);
    app.world_mut()
        .resource_mut::<CameraMode>()
        .set_if_neq(CameraMode::Hero);
    let kill = |app: &mut App| {
        let e = hero(app).0;
        app.world_mut().get_mut::<Health>(e).unwrap().hp = 0.0;
    };
    // the state of a man on his feet: collides, falls, nobody's passenger
    let free = |app: &mut App| {
        if hero(app).1.downed {
            step(app, 40); // the visible topple has finished, before respawn
        }
        let (e, h, ..) = hero(app);
        assert_eq!((h.in_vehicle, h.stunt), (None, None), "{h:?}");
        let w = app.world();
        assert!(w.get::<OnStunt>(e).is_none(), "still pinned to a roof");
        assert!(w.get::<Stowed>(e).is_none(), "still stowed");
        assert!(w.get::<WireSwing>(e).is_none(), "still swinging");
        assert!(
            w.get::<HookedVehicle>(e).is_none(),
            "still tracking a vehicle"
        );
        assert_eq!(w.get::<crate::combat::Dead>(e).is_some(), h.downed);
        let articulated = w.get::<crate::ragdoll::PhysicsPose>(e).is_some();
        if articulated {
            assert_eq!(
                *w.get::<CollisionLayers>(e).unwrap(),
                CollisionLayers::NONE,
                "logical capsule must not collide with its limbs"
            );
        } else {
            assert_ne!(
                *w.get::<CollisionLayers>(e).unwrap(),
                CollisionLayers::NONE,
                "no collision"
            );
        }
        assert_ne!(*w.get::<Visibility>(e).unwrap(), Visibility::Hidden);
        assert!(
            w.get::<Occupied>(car).is_none(),
            "the car is still occupied"
        );
        assert_eq!(*w.resource::<CameraMode>(), CameraMode::Hero);
        let mut query = app
            .world_mut()
            .query_filtered::<&Transform, With<crate::character::HeroModel>>();
        let model = query.single(app.world()).unwrap();
        let up = model.rotation * Vec3::Y;
        if h.downed && !articulated {
            assert!(
                up.y.abs() < 0.02,
                "vehicle death body must stay lying: {up:?}"
            );
        } else if !h.downed {
            assert!(up.y > 0.99, "respawn must restore upright model: {up:?}");
        }
    };
    // get up again at the start
    let respawn = |app: &mut App| {
        step(app, (crate::combat::RESPAWN_SECONDS * 64.0) as usize + 10);
        crate::fleet::testing::wait_for_hero(app);
        let (e, h, p, _) = hero(app);
        let w = app.world();
        assert_eq!(
            w.get::<Health>(e).unwrap().hp,
            w.get::<Health>(e).unwrap().max
        );
        assert!(p.distance(spawn) < 20.0, "respawned at {p:?}");
        assert_eq!((h.in_vehicle, h.stunt), (None, None));
        assert!(!h.downed && w.get::<crate::combat::Dead>(e).is_none());
    };

    // ---- on the roof
    let (p, r) = car_pose(&app, car);
    put_hero(&mut app, p + r * Vec3::new(2.5, 1.0, 0.0));
    step(&mut app, 30);
    press(&mut app, KeyCode::KeyQ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyQ);
    step(&mut app, 3);
    assert_eq!(hero(&mut app).1.stunt, Some(car), "not on the roof");
    let (entity, _, position, velocity) = hero(&mut app);
    app.world_mut().entity_mut(entity).insert(WireSwing {
        pos: position,
        vel: velocity,
    });
    kill(&mut app);
    step(&mut app, 3);
    free(&mut app);
    respawn(&mut app);
    free(&mut app);

    // ---- in the seat
    let (p, r) = car_pose(&app, car);
    put_hero(&mut app, p + r * Vec3::new(2.0, 0.5, 0.0));
    step(&mut app, 30);
    press(&mut app, KeyCode::KeyE);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyE);
    step(&mut app, 3);
    assert_eq!(hero(&mut app).1.in_vehicle, Some(car), "not in the car");
    assert!(app.world().get::<Occupied>(car).is_some());
    assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Drive);
    kill(&mut app);
    step(&mut app, 3);
    free(&mut app);
    let (_, _, at, _) = hero(&mut app);
    assert!(
        at.distance(car_pose(&app, car).0) < 8.0,
        "thrown out {at:?} from the car"
    );
    respawn(&mut app);
    free(&mut app);
    eprintln!("rico: died on the roof and in the seat, got out both times, and got up");
}

// ---- boats and helicopters

/// A headless app with Rico, the stunts and the free camera's entity (the hook and the
/// keys need it), plus whatever `setup` adds. `None` without the test data.
pub(crate) fn rico_app(needs: &[&str], setup: impl FnOnce(&mut App)) -> Option<(App, f32, Vec3)> {
    let mut all = vec!["mc01_lod1-rico.rbm"];
    all.extend_from_slice(needs);
    let (mut app, ground, spawn) = crate::fleet::testing::app(&all, |app| {
        app.init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .add_plugins((CharacterPlugin, StuntPlugin));
        setup(app);
    })?;
    app.world_mut().spawn((
        Transform::from_translation(spawn + Vec3::new(0.0, 4.0, -8.0)),
        FlyCamera {
            yaw: 0.0,
            pitch: 0.0,
            speed: 1.0,
        },
    ));
    Some((app, ground, spawn))
}

/// Where Rico should be on the spot he is pinned to, and how far he is from it.
pub(crate) fn off_spot(app: &mut App) -> f32 {
    let (e, _, hp, _) = hero(app);
    let on = *app.world().get::<OnStunt>(e).expect("on a vehicle");
    let (p, r) = car_pose(app, on.vehicle);
    if let Some(len) = on.wire {
        // on the wire he swings: he is `len` from the hook point, wherever that puts him
        return ((hp - (p + r * on.local)).length() - len).abs();
    }
    (hp - (p + r * on.local)).length()
}

/// Rico on the motorboat's bow deck (its `CRoofTopStuntController`) stays there within
/// 0.3 m while it runs at full throttle in a turn, pitching and rolling on the water, and
/// jumps off with its speed.
#[test]
fn rico_rides_a_boat_bow() {
    use crate::boat::{BoatPlugin, Watercraft, find_open_water};
    use jc2_formats::terrain::GlobalTerrain;
    let boat_name = "seve.v083_motorboat_small.ee";
    let Some((mut app, _, _)) = rico_app(&[boat_name, "terrain_win32.dat"], |app| {
        app.add_plugins(BoatPlugin);
    }) else {
        return;
    };
    let fs = app.world().resource::<crate::Game>().fs.clone();
    let map = GlobalTerrain::parse(&fs.read_inflated("terrain_win32.dat").unwrap()).unwrap();
    let (wx, wz) = find_open_water(&map, -15000.0, -15000.0, 500.0, 3.0).unwrap();
    let boat = crate::fleet::testing::spawn(
        &mut app,
        boat_name,
        Place::At(Transform::from_translation(crate::coords::bevy_xyz(
            wx,
            crate::world::SEA_LEVEL,
            wz,
        ))),
    );
    let spots = app
        .world()
        .get::<StuntSpots>(boat)
        .expect("spots")
        .0
        .clone();
    assert_eq!(spots.len(), 1, "{spots:?}");
    assert_eq!(spots[0].kind, StuntKind::Roof);
    // the bow: well ahead of the middle (the nose is −z)
    assert!(spots[0].local.z < -2.0, "{spots:?}");
    crate::fleet::testing::wait_for_hero(&mut app);
    // let the boat settle on the water
    step(&mut app, 64 * 4);
    app.world_mut()
        .resource_mut::<CameraMode>()
        .set_if_neq(CameraMode::Hero);
    // in the water beside the bow: Q gets him up
    let (p, r) = car_pose(&app, boat);
    put_hero(
        &mut app,
        p + r * (spots[0].local + Vec3::new(2.0, 0.5, 0.0)),
    );
    step(&mut app, 2);
    press(&mut app, KeyCode::KeyQ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyQ);
    step(&mut app, 3);
    let (e, h, _, _) = hero(&mut app);
    assert_eq!(h.stunt, Some(boat), "not on the bow: {h:?}");
    assert!(off_spot(&mut app) < 0.01);
    assert_eq!(
        *app.world().get::<CollisionLayers>(e).unwrap(),
        CollisionLayers::NONE
    );
    // full throttle, rudder over: 12 s
    app.world_mut()
        .entity_mut(boat)
        .insert(crate::vehicle::AiDriven);
    let mut worst = 0.0f32;
    let (mut pitch_lo, mut pitch_hi) = (f32::MAX, f32::MIN);
    let start = car_pose(&app, boat).0;
    for _ in 0..64 * 12 {
        {
            let mut b = app.world_mut().get_mut::<Watercraft>(boat).unwrap();
            b.throttle = 1.0;
            b.rudder = 0.4;
        }
        step(&mut app, 1);
        worst = worst.max(off_spot(&mut app));
        let r = car_pose(&app, boat).1;
        let nose = crate::coords::forward_of(r);
        let pitch = nose.y.asin();
        pitch_lo = pitch_lo.min(pitch);
        pitch_hi = pitch_hi.max(pitch);
        // the drawn positions agree too
        let on = *app.world().get::<OnStunt>(e).unwrap();
        let tf = app.world().get::<Transform>(e).unwrap().translation;
        let btf = *app.world().get::<Transform>(boat).unwrap();
        worst = worst.max((tf - (btf.translation + btf.rotation * on.local)).length());
    }
    let moved = (car_pose(&app, boat).0 - start).length();
    let speed = app.world().get::<LinearVelocity>(boat).unwrap().length();
    eprintln!(
        "boat bow: ran {moved:.0} m at up to {:.0} km/h, pitched {:.1}°..{:.1}°, Rico at most {worst:.3} m off the bow",
        speed * 3.6,
        pitch_lo.to_degrees(),
        pitch_hi.to_degrees()
    );
    assert!(moved > 60.0, "the boat only went {moved:.1} m");
    assert!(
        pitch_hi - pitch_lo > 0.5f32.to_radians(),
        "no pitching to speak of"
    );
    assert!(worst < 0.3, "drifted {worst:.3} m from the bow");
    let (_, h, _, v) = hero(&mut app);
    assert_eq!(h.stunt, Some(boat));
    assert!(v.length() > 5.0, "should ride at the boat's speed: {v:?}");
    // jump off: the boat's speed, up and away
    let bv = app.world().get::<LinearVelocity>(boat).unwrap().0;
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    step(&mut app, 1);
    let (_, h, _, v) = hero(&mut app);
    assert_eq!(h.stunt, None);
    let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
    assert!(
        (flat(v) - flat(bv)).length() < 2.0,
        "{v:?} vs the boat's {bv:?}"
    );
    assert!(v.y > 2.0, "no jump: {v:?}");
}

/// A helicopter with Rico hanging under its belly (`CBellyStuntController`): he stays
/// pinned within 0.2 m while it spins up, climbs 30 m and turns.
#[test]
fn rico_hangs_under_a_climbing_turning_helicopter() {
    use crate::heli::{Heli, HeliPlugin};
    let name = "arve.v062_chopper.ee";
    let Some((mut app, _, _)) = rico_app(&[name], |app| {
        app.add_plugins(HeliPlugin);
    }) else {
        return;
    };
    let heli =
        crate::fleet::testing::spawn(&mut app, name, Place::NearStart(Vec3::new(0.0, 0.0, 16.0)));
    assert!(app.world().get::<Occupied>(heli).is_none());
    let spots = app
        .world()
        .get::<StuntSpots>(heli)
        .expect("spots")
        .0
        .clone();
    let belly = spots
        .iter()
        .find(|s| s.kind == StuntKind::Belly)
        .unwrap_or_else(|| panic!("no belly spot: {spots:?}"))
        .clone();
    assert!(
        spots.iter().any(|s| s.kind == StuntKind::Cling),
        "{spots:?}"
    );
    crate::fleet::testing::wait_for_hero(&mut app);
    step(&mut app, 64 * 2);
    app.world_mut()
        .resource_mut::<CameraMode>()
        .set_if_neq(CameraMode::Hero);
    let (p, r) = car_pose(&app, heli);
    put_hero(&mut app, p + r * (belly.local + Vec3::new(2.5, 1.0, 0.0)));
    step(&mut app, 20);
    press(&mut app, KeyCode::KeyQ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyQ);
    step(&mut app, 3);
    let (e, h, hp, _) = hero(&mut app);
    assert_eq!(
        h.stunt,
        Some(heli),
        "not under the helicopter: {h:?} at {hp:?}"
    );
    assert_eq!(h.stunt_clip, Some(crate::rig::Clip::Hang));
    let on = *app.world().get::<OnStunt>(e).unwrap();
    assert_eq!(on.kind, StuntKind::Belly);
    // hanging: his middle well below the hold
    assert!((on.local - (belly.local - Vec3::Y * HANG_DROP)).length() < 1e-4);
    // a pilot's hands on the controls: spin up, climb, then turn
    app.world_mut()
        .entity_mut(heli)
        .insert(crate::vehicle::AiDriven);
    let y0 = car_pose(&app, heli).0.y;
    let heading0 = app.world().get::<Heli>(heli).unwrap().heading;
    let mut worst = 0.0f32;
    for i in 0..64 * 12 {
        {
            let mut hc = app.world_mut().get_mut::<Heli>(heli).unwrap();
            let t = i as f32 / 64.0;
            hc.collective = if t < 9.0 { 1.0 } else { 0.0 };
            hc.yaw = if (7.0..8.5).contains(&t) { 1.0 } else { 0.0 };
            hc.pitch = if t > 8.0 { 0.5 } else { 0.0 };
        }
        step(&mut app, 1);
        worst = worst.max(off_spot(&mut app));
    }
    let p1 = car_pose(&app, heli).0;
    let climbed = p1.y - y0;
    // (the pedals' heading, not wrapped round)
    let turned = app.world().get::<Heli>(heli).unwrap().heading - heading0;
    eprintln!(
        "heli belly: climbed {climbed:.0} m, turned {:.0}°, Rico at most {worst:.3} m off",
        turned.to_degrees()
    );
    assert!(climbed > 30.0, "climbed {climbed:.1} m");
    assert!(turned.abs() > 1.0, "turned {turned:.2} rad");
    assert!(worst < 0.2, "drifted {worst:.3} m");
    let (_, h, _, v) = hero(&mut app);
    assert_eq!(h.stunt, Some(heli));
    let hv = app.world().get::<LinearVelocity>(heli).unwrap().0;
    assert!(
        (v - hv).length() < 5.0,
        "his velocity {v:?}, the helicopter's {hv:?}"
    );
}

/// The hook fired at a flying helicopter from below: Rico is reeled up and stays on the
/// wire under it, pinned there while it flies; `Space` lets go, he falls, and `Space`
/// again opens the chute.
#[test]
fn rico_hangs_on_the_wire_then_lets_go() {
    use crate::heli::{Heli, HeliPlugin};
    let name = "arve.v062_chopper.ee";
    let Some((mut app, _, _)) = rico_app(&[name], |app| {
        app.add_plugins(HeliPlugin);
    }) else {
        return;
    };
    let heli =
        crate::fleet::testing::spawn(&mut app, name, Place::NearStart(Vec3::new(0.0, 0.0, 30.0)));
    crate::fleet::testing::wait_for_hero(&mut app);
    app.world_mut()
        .resource_mut::<CameraMode>()
        .set_if_neq(CameraMode::Hero);
    // up 60 m
    app.world_mut()
        .entity_mut(heli)
        .insert(crate::vehicle::AiDriven);
    let set = |app: &mut App, c: f32, y: f32, p: f32| {
        let mut hc = app.world_mut().get_mut::<Heli>(heli).unwrap();
        hc.collective = c;
        hc.yaw = y;
        hc.pitch = p;
    };
    let y0 = car_pose(&app, heli).0.y;
    let mut n = 0;
    while car_pose(&app, heli).0.y < y0 + 60.0 {
        set(&mut app, 1.0, 0.0, 0.0);
        step(&mut app, 1);
        n += 1;
        assert!(n < 64 * 30, "the helicopter never climbed");
    }
    for _ in 0..64 * 2 {
        set(&mut app, 0.0, 0.0, 0.0);
        step(&mut app, 1);
    }
    // Rico 15 m under the belly, in the air, firing the hook at it
    let spots = app.world().get::<StuntSpots>(heli).unwrap().0.clone();
    let belly = spots
        .iter()
        .find(|s| s.kind == StuntKind::Belly)
        .unwrap()
        .clone();
    let (p, r) = car_pose(&app, heli);
    let hook = p + r * belly.local;
    put_hero(&mut app, hook - Vec3::Y * 15.0 + Vec3::X * 2.0);
    {
        let w = app.world_mut();
        let mut q = w.query::<&mut Hero>();
        let mut h = q.single_mut(w).unwrap();
        h.grapple = Some(Grapple {
            anchor: hook,
            normal: Vec3::NEG_Y,
            age: 0.0,
        });
        h.grounded = false;
    }
    let mut caught = false;
    for _ in 0..200 {
        set(&mut app, 0.0, 0.0, 0.0);
        step(&mut app, 1);
        let (e, ..) = hero(&mut app);
        if app
            .world()
            .get::<OnStunt>(e)
            .is_some_and(|o| o.wire.is_some())
        {
            caught = true;
            break;
        }
    }
    assert!(caught, "never hung on the wire: {:?}", hero(&mut app));
    let (e, h, _, _) = hero(&mut app);
    let on = *app.world().get::<OnStunt>(e).unwrap();
    let len = on.wire.unwrap();
    assert!((WIRE_MIN..=WIRE_LENGTH).contains(&len), "wire {len}");
    assert_eq!(h.stunt, Some(heli));
    assert!(h.grapple.is_none());
    assert_eq!(h.stunt_clip, Some(crate::rig::Clip::Hang));
    // fly on for 4 s, turning and pitching forward: he stays on the wire's end
    let mut worst = 0.0f32;
    for i in 0..64 * 4 {
        set(&mut app, 0.0, 0.6, if i > 64 { 0.6 } else { 0.0 });
        step(&mut app, 1);
        worst = worst.max(off_spot(&mut app));
    }
    assert!(worst < 0.2, "drifted {worst:.3} m off the wire's end");
    let (_, h, hp, _) = hero(&mut app);
    assert_eq!(h.stunt, Some(heli));
    assert_eq!(
        *app.world().get::<CollisionLayers>(e).unwrap(),
        CollisionLayers::NONE
    );
    // let go
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    step(&mut app, 32);
    let (_, h, hp2, v) = hero(&mut app);
    assert_eq!(h.stunt, None, "still on the wire");
    assert!(app.world().get::<OnStunt>(e).is_none());
    assert!(!h.parachute);
    assert!(
        hp2.y < hp.y - 0.5 && v.y < -2.0,
        "not falling: {hp:?} → {hp2:?}, {v:?}"
    );
    assert_ne!(
        *app.world().get::<CollisionLayers>(e).unwrap(),
        CollisionLayers::NONE
    );
    // the chute
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    step(&mut app, 4);
    let (_, h, _, v) = hero(&mut app);
    assert!(h.parachute, "the chute did not open: {h:?}");
    eprintln!(
        "wire: hung {len:.1} m under, at most {worst:.3} m off while flying; let go, fell, chute open at {v:?}"
    );
}

/// The angle from the vertical (degrees) of the wire from `hook` to `at`.
fn wire_angle(hook: Vec3, at: Vec3) -> f32 {
    (hook - at).angle_between(Vec3::Y).to_degrees()
}

/// The pendulum under a helicopter that circles at 10 m/s on a 40 m radius for 12 s, slows
/// to a hover over 4 s and hangs still: the swing stays inside 45° and dies away to a
/// few degrees after.
#[test]
fn the_wire_swings_bounded_under_a_turning_helicopter_and_damps_out() {
    let dt = 1.0 / 64.0;
    let length = 5.0;
    let (radius, speed) = (40.0f32, 10.0f32);
    // the distance flown along the circle: steady, then braking at 2.5 m/s² to a stop
    let arc = |t: f32| {
        if t <= 12.0 {
            speed * t
        } else {
            let u = (t - 12.0).min(4.0);
            speed * 12.0 + speed * u - 1.25 * u * u
        }
    };
    let pivot = |t: f32| {
        let a = arc(t) / radius;
        Vec3::new(radius * a.sin(), 60.0, radius * (1.0 - a.cos()))
    };
    // he comes aboard moving with the helicopter
    let mut state = WireSwing {
        pos: pivot(0.0) - Vec3::Y * length,
        vel: (pivot(dt) - pivot(0.0)) / dt,
    };
    let mut worst = 0.0f32;
    let turning = 18.0;
    let mut t = 0.0;
    while t < turning {
        let hook = pivot(t);
        let vel = (pivot(t + dt) - hook) / dt;
        swing_step(&mut state, hook, vel, length, dt);
        let len = (state.pos - hook).length();
        assert!((len - length).abs() < 1e-3, "wire stretched to {len}");
        worst = worst.max(wire_angle(hook, state.pos));
        t += dt;
    }
    assert!(worst < 45.0, "swung out to {worst:.1}°");
    assert!(worst > 3.0, "the turn should swing him: {worst:.1}°");
    // the helicopter stops: the swing decays
    let hook = pivot(t);
    let mut peaks = Vec::new();
    let mut window = 0.0f32;
    for i in 0..64 * 40 {
        swing_step(&mut state, hook, Vec3::ZERO, length, dt);
        window = window.max(wire_angle(hook, state.pos));
        if (i + 1) % (64 * 10) == 0 {
            peaks.push(window);
            window = 0.0;
        }
    }
    assert!(
        peaks.windows(2).all(|w| w[1] < w[0]),
        "not decaying: {peaks:?}"
    );
    assert!(peaks[3] < 3.0, "still swinging {:.1}° after 40 s", peaks[3]);
    eprintln!("swing: worst {worst:.1}° while turning, then {peaks:?}");
}

mod transitions;
