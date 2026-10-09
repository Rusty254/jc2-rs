use super::*;
use crate::stunts::tests::{hero, press, put_hero, release, rico_app, step};

type Setup = (App, f32, Vec3);

/// A headless app with Rico on the flat tile (and the stunts), standing on the ground.
fn setup() -> Option<Setup> {
    let (mut app, ground, spawn) = rico_app(&[], |_| {})?;
    crate::fleet::testing::wait_for_hero(&mut app);
    step(&mut app, 10);
    Some((app, ground, spawn))
}

fn rig(app: &mut App) -> GrappleRig {
    let mut q = app.world_mut().query::<&GrappleRig>();
    q.single(app.world()).unwrap().clone()
}

fn lock(app: &App) -> Option<LockTarget> {
    app.world().resource::<LockOn>().target
}

/// Put the camera behind Rico looking at `point`.
fn aim(app: &mut App, point: Vec3) {
    let (_, _, p, _) = hero(app);
    let mut q = app
        .world_mut()
        .query_filtered::<&mut Transform, With<FlyCamera>>();
    let mut cam = q.single_mut(app.world_mut()).unwrap();
    cam.translation = p + Vec3::new(0.0, 1.2, 3.5);
    cam.look_at(point, Vec3::Y);
}

fn tap_f(app: &mut App) {
    press(app, KeyCode::KeyF);
    step(app, 1);
    release(app, KeyCode::KeyF);
}

/// A big wall whose front face is `dist` ahead of (north of) `at`, `up` m tall from the ground.
fn wall(app: &mut App, at: Vec3, dist: f32, up: f32) {
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(120.0, up, 2.0),
        Transform::from_translation(Vec3::new(at.x, at.y - 0.9 + up / 2.0, at.z - dist - 1.0)),
        Name::new("wall"),
    ));
    step(app, 3);
}

fn face_z(at: Vec3, dist: f32) -> f32 {
    at.z - dist
}

fn tuning(app: &App) -> Tuning {
    Tuning::new(app.world().get_resource::<HeroParams>())
}

/// The hook flies at the data's speed (`#59e07a13`, 120 m/s) and attaches when it gets
/// there: `distance / speed` seconds after the shot.
#[test]
fn the_hook_flies_at_the_data_speed_and_attaches_on_arrival() {
    let Some((mut app, _ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    wall(&mut app, p, 60.0, 80.0);
    aim(&mut app, Vec3::new(p.x, p.y + 10.0, face_z(p, 60.0)));
    let speed = tuning(&app).hook_speed;
    assert!(speed > 50.0, "hook speed {speed}");
    tap_f(&mut app);
    let r = rig(&mut app);
    assert_eq!(r.phase, Phase::Flight, "the hook should be in the air");
    let f = r.flight.expect("a flight");
    assert!(f.hit.is_some(), "the wall is in range");
    assert!(
        f.total > 55.0 && f.total < 75.0,
        "flight length {}",
        f.total
    );
    assert!((f.speed - speed).abs() < 1e-3);
    assert!(hero(&mut app).1.grapple.is_none(), "not attached yet");
    let mut frames = 0;
    let mut last = r.hook;
    let mut moved = 0.0;
    while rig(&mut app).phase == Phase::Flight {
        step(&mut app, 1);
        frames += 1;
        let h = rig(&mut app).hook;
        moved = h.distance(last);
        last = h;
        assert!(frames < 200, "never arrived");
    }
    let expect = f.total / speed;
    let took = frames as f32 / 64.0;
    assert!(
        (took - expect).abs() < 3.0 / 64.0,
        "took {took:.3} s, distance/speed = {expect:.3} s"
    );
    assert!(moved > 0.0);
    let (_, h, _, _) = hero(&mut app);
    let g = h.grapple.expect("attached on arrival");
    assert!(
        (g.anchor.z - face_z(p, 60.0)).abs() < 0.5,
        "anchor {:?}",
        g.anchor
    );
    eprintln!(
        "hook: {:.1} m at {speed} m/s took {took:.3} s (expected {expect:.3} s)",
        f.total
    );
}

/// A shot past the range finds nothing to hold: the hook flies to the range and comes
/// home, and Rico is never on the wire.
#[test]
fn a_miss_beyond_the_range_retracts() {
    let Some((mut app, _ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    // a wall 150 m away: farther than the 90 m range
    wall(&mut app, p, 150.0, 80.0);
    aim(&mut app, Vec3::new(p.x, p.y + 10.0, face_z(p, 150.0)));
    let range = tuning(&app).range;
    tap_f(&mut app);
    let r = rig(&mut app);
    assert_eq!(r.phase, Phase::Flight);
    let f = r.flight.unwrap();
    assert!(f.hit.is_none(), "nothing within {range} m");
    assert!((f.total - range).abs() < 1.0, "flies {} m", f.total);
    let mut frames = 0;
    let mut saw_retract = false;
    let mut farthest = 0.0f32;
    while rig(&mut app).phase != Phase::Idle {
        step(&mut app, 1);
        frames += 1;
        let r = rig(&mut app);
        saw_retract |= r.phase == Phase::Retract;
        farthest = farthest.max(r.hook.distance(f.origin));
        let (_, h, _, _) = hero(&mut app);
        assert!(h.grapple.is_none(), "a miss must not hold him");
        assert!(frames < 64 * 5, "the hook never came home");
    }
    assert!(saw_retract);
    assert!(farthest > range - 3.0, "went out only {farthest} m");
    // out in range/speed, back in half that
    let out = range / f.speed;
    let secs = frames as f32 / 64.0;
    assert!(
        secs > out && secs < out * 2.0,
        "{secs:.2} s for a {out:.2} s flight out"
    );
}

/// Reeled to a wall he comes to rest on the goal (within 0.2 m) without overshoot or
/// oscillation: the distance falls all the way and he then holds still.
#[test]
fn reeling_arrives_at_a_wall_without_overshoot() {
    let Some((mut app, _ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    wall(&mut app, p, 40.0, 80.0);
    aim(&mut app, Vec3::new(p.x, p.y + 12.0, face_z(p, 40.0)));
    tap_f(&mut app);
    let mut series = Vec::new();
    let mut max_speed = 0.0f32;
    for _ in 0..64 * 8 {
        step(&mut app, 1);
        let r = rig(&mut app);
        if r.phase == Phase::Reel || r.phase == Phase::Hang {
            let (_, _, hp, v) = hero(&mut app);
            series.push(((hp - r.goal).length(), hp));
            max_speed = max_speed.max(v.length());
        }
        if r.phase == Phase::Hang && series.len() > 20 {
            break;
        }
    }
    let r = rig(&mut app);
    assert_eq!(r.phase, Phase::Hang, "never arrived: {:?}", series.last());
    let (_, h, _, _) = hero(&mut app);
    assert!(h.hanging && h.grapple.is_some());
    assert!(max_speed > 25.0, "reeled at only {max_speed} m/s");
    // 1.5 s of hanging: still at the goal
    let mut worst = 0.0f32;
    for _ in 0..96 {
        step(&mut app, 1);
        let (_, _, hp, v) = hero(&mut app);
        worst = worst.max((hp - r.goal).length());
        assert!(
            v.length() < 1.5,
            "moving at {} m/s while hanging",
            v.length()
        );
    }
    assert!(worst < 0.2, "hanging {worst:.3} m off the goal");
    // the distance to the goal never grows by more than a hair once he is near
    let mut near = false;
    let mut prev = f32::MAX;
    for (d, hp) in &series {
        if *d < 8.0 {
            near = true;
        }
        if near {
            assert!(*d <= prev + 0.02, "backed off: {prev:.3} -> {d:.3}");
        }
        prev = *d;
        // never into the wall: its face is at z = p.z - 40
        assert!(
            hp.z > face_z(p, 40.0) + RADIUS - 0.02,
            "inside the wall: {hp:?}"
        );
    }
    let last = series.last().unwrap();
    assert!(last.0 < 0.2, "arrived {:.3} m off", last.0);
    eprintln!(
        "reel: peak {max_speed:.1} m/s, arrived {:.3} m off the goal, held within {worst:.3} m",
        last.0
    );
}

/// `F` mid-reel lets go and he keeps flying at the speed he had.
#[test]
fn releasing_mid_reel_keeps_his_velocity() {
    let Some((mut app, _ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    wall(&mut app, p, 70.0, 100.0);
    aim(&mut app, Vec3::new(p.x, p.y + 25.0, face_z(p, 70.0)));
    tap_f(&mut app);
    for _ in 0..64 * 4 {
        step(&mut app, 1);
        if rig(&mut app).reel > 22.0 {
            break;
        }
    }
    let (_, h, _, before) = hero(&mut app);
    assert!(h.grapple.is_some() && before.length() > 20.0, "{before:?}");
    tap_f(&mut app);
    let (_, h, _, after) = hero(&mut app);
    assert!(h.grapple.is_none(), "still on the wire");
    assert!(
        after.length() > before.length() * 0.9,
        "lost speed: {before:?} -> {after:?}"
    );
    step(&mut app, 20);
    let (_, _, _, later) = hero(&mut app);
    assert!(later.length() > 17.0, "slowed to {later:?}");
    assert!(matches!(rig(&mut app).phase, Phase::Retract | Phase::Idle));
}

/// Opening the chute mid-reel keeps the speed: letting go afterwards he glides on at well
/// over the chute's own glide speed (11 m/s): the grapple-and-chute boost.
#[test]
fn opening_the_chute_mid_reel_gives_forward_speed() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    // 80 m up, a wall 70 m ahead
    put_hero(&mut app, Vec3::new(p.x, ground + 80.0, p.z));
    step(&mut app, 2);
    let (_, _, p, _) = hero(&mut app);
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(120.0, 100.0, 2.0),
        Transform::from_translation(Vec3::new(p.x, ground + 40.0, face_z(p, 70.0) - 1.0)),
    ));
    step(&mut app, 3);
    let (_, _, p, _) = hero(&mut app);
    aim(&mut app, Vec3::new(p.x, p.y + 4.0, face_z(p, 70.0)));
    tap_f(&mut app);
    for _ in 0..64 * 4 {
        step(&mut app, 1);
        if rig(&mut app).reel > 28.0 {
            break;
        }
    }
    assert!(rig(&mut app).reel > 28.0, "never got up to speed");
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    step(&mut app, 2);
    let (_, h, _, v) = hero(&mut app);
    assert!(h.parachute, "the chute did not open mid-reel: {h:?}");
    assert!(h.grapple.is_some(), "the wire still holds him");
    assert!(Vec3::new(v.x, 0.0, v.z).length() > 25.0, "slowed to {v:?}");
    // let go: the chute carries the speed on
    tap_f(&mut app);
    step(&mut app, 20);
    let (_, h, _, v) = hero(&mut app);
    let forward = Vec3::new(v.x, 0.0, v.z).length();
    assert!(h.parachute && h.grapple.is_none());
    assert!(forward > 19.0, "glides at {forward} m/s after the boost");
    eprintln!("chute mid-reel: {forward:.1} m/s forward after letting go");
}

/// Hooked on a ledge he hangs below it; `Ctrl` drops, `Space` climbs up and over.
#[test]
fn a_ledge_is_climbed_with_space_and_dropped_with_ctrl() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    // a building: roof 8 m up, its face 35 m ahead, 20 m deep
    let top = ground + 8.0;
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(40.0, 8.0, 20.0),
        Transform::from_translation(Vec3::new(p.x, ground + 4.0, face_z(p, 35.0) - 10.0)),
    ));
    step(&mut app, 3);
    aim(&mut app, Vec3::new(p.x, top - 1.5, face_z(p, 35.0)));
    step(&mut app, 1);
    let t = lock(&app).expect("a lock on the wall");
    assert_eq!(
        t.kind,
        TargetKind::Ledge,
        "the roof edge is in reach: {t:?}"
    );
    aim(&mut app, Vec3::new(p.x, top - 1.5, face_z(p, 35.0)));
    tap_f(&mut app);
    for _ in 0..64 * 5 {
        step(&mut app, 1);
        if rig(&mut app).phase == Phase::Hang {
            break;
        }
    }
    let r = rig(&mut app);
    assert_eq!(r.phase, Phase::Hang);
    assert!(
        r.ledge_top.is_some_and(|t| (t - top).abs() < 0.2),
        "{:?}",
        r.ledge_top
    );
    // drop first (Ctrl): he falls
    press(&mut app, KeyCode::ControlLeft);
    step(&mut app, 1);
    release(&mut app, KeyCode::ControlLeft);
    step(&mut app, 4);
    let (_, h, _, v) = hero(&mut app);
    assert!(h.grapple.is_none() && !h.hanging, "still hanging");
    assert!(v.y < 0.0, "not falling: {v:?}");
    // again, from the foot of the wall: hook it, hang, climb
    step(&mut app, 120);
    let (_, _, p2, _) = hero(&mut app);
    aim(&mut app, Vec3::new(p2.x, top - 1.5, face_z(p, 35.0)));
    tap_f(&mut app);
    for _ in 0..64 * 5 {
        step(&mut app, 1);
        if rig(&mut app).phase == Phase::Hang {
            break;
        }
    }
    assert_eq!(rig(&mut app).phase, Phase::Hang);
    step(&mut app, 30);
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    step(&mut app, 64 * 3);
    let (_, h, hp, _) = hero(&mut app);
    assert!(h.grounded, "not standing: {h:?} at {hp:?}");
    assert!(hp.y > top + 0.5, "not on the roof: {hp:?}, roof at {top}");
    assert!(hp.z < face_z(p, 35.0), "not over the edge: {hp:?}");
}

/// A plain wall: `Space` leaps off it away from the wall.
#[test]
fn space_leaps_off_a_wall() {
    let Some((mut app, _ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    wall(&mut app, p, 30.0, 80.0);
    aim(&mut app, Vec3::new(p.x, p.y + 12.0, face_z(p, 30.0)));
    tap_f(&mut app);
    for _ in 0..64 * 5 {
        step(&mut app, 1);
        if rig(&mut app).phase == Phase::Hang {
            break;
        }
    }
    assert_eq!(rig(&mut app).phase, Phase::Hang);
    step(&mut app, 20);
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    step(&mut app, 3);
    let (_, h, _, v) = hero(&mut app);
    assert!(h.grapple.is_none() && !h.parachute, "{h:?}");
    assert!(v.z > 3.0 && v.y > 3.0, "leapt with {v:?}");
}

/// Hooked on the ground ahead he is reeled to it and lands, standing, with the wire let go.
#[test]
fn reeling_to_the_ground_lands_him() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    // up in the air first, 20 m
    put_hero(&mut app, Vec3::new(p.x, ground + 20.0, p.z));
    step(&mut app, 2);
    let (_, _, p, _) = hero(&mut app);
    aim(&mut app, Vec3::new(p.x, ground, p.z - 25.0));
    tap_f(&mut app);
    for _ in 0..64 * 6 {
        step(&mut app, 1);
        let (_, h, _, _) = hero(&mut app);
        if h.grapple.is_none() && rig(&mut app).phase != Phase::Flight {
            break;
        }
    }
    step(&mut app, 64 * 2);
    let (_, h, hp, v) = hero(&mut app);
    assert!(h.grounded, "not landed: {h:?} at {hp:?}");
    assert!(h.grapple.is_none());
    assert!(v.length() < 1.0, "still moving at {v:?}");
    assert!((hp.z - (p.z - 25.0)).abs() < 3.0, "landed at {hp:?}");
}

/// A light prop is yanked towards Rico; he stays where he is.
#[test]
fn hooking_a_barrel_pulls_it_to_rico() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    let at = Vec3::new(p.x, ground + 0.4, p.z - 20.0);
    let barrel = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(0.6, 0.8, 0.6),
            Mass(40.0),
            Transform::from_translation(at),
        ))
        .id();
    step(&mut app, 30);
    let start = app.world().get::<Position>(barrel).unwrap().0;
    aim(&mut app, start);
    tap_f(&mut app);
    let mut pulling = false;
    for _ in 0..64 * 6 {
        step(&mut app, 1);
        pulling |= rig(&mut app).phase == Phase::Pull;
        let d = app.world().get::<Position>(barrel).unwrap().0.distance(p);
        if d < 4.0 {
            break;
        }
    }
    assert!(pulling, "the barrel was never pulled");
    let end = app.world().get::<Position>(barrel).unwrap().0;
    assert!(
        end.distance(p) < start.distance(p) - 10.0,
        "barrel {start:?} -> {end:?}, Rico at {p:?}"
    );
    let (_, h, hp, _) = hero(&mut app);
    assert!(h.grapple.is_none(), "a barrel does not reel Rico in");
    assert!(hp.distance(p) < 1.0, "Rico moved to {hp:?}");
    step(&mut app, 64 * 2);
    assert!(matches!(rig(&mut app).phase, Phase::Retract | Phase::Idle));
}

/// The authored category beats the mass: a 40 kg body marked heavy (2) is an anchor that
/// reels Rico in, and it keeps moving under its own velocity while he is reeled.
#[test]
fn an_authored_heavy_body_reels_rico_instead_of_being_pulled() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    let at = Vec3::new(p.x, ground + 0.4, p.z - 20.0);
    let body = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(0.6, 0.8, 0.6),
            Mass(40.0),
            GrappleWeight(2),
            Transform::from_translation(at),
        ))
        .id();
    step(&mut app, 30);
    let start = app.world().get::<Position>(body).unwrap().0;
    aim(&mut app, start);
    tap_f(&mut app);
    let mut pulled = false;
    let mut reeled = false;
    for _ in 0..64 * 2 {
        step(&mut app, 1);
        pulled |= rig(&mut app).phase == Phase::Pull;
        let (_, h, _, _) = hero(&mut app);
        reeled |= h.grapple.is_some();
    }
    assert!(!pulled, "a heavy body must not be pulled");
    assert!(reeled, "Rico was never reeled to the heavy body");
    let end = app.world().get::<Position>(body).unwrap().0;
    assert!(
        end.distance(start) < 3.0,
        "the anchor was dragged: {start:?} -> {end:?}"
    );
}

/// A soldier is pulled off his feet to Rico, lies there, then stands again.
#[test]
fn hooking_a_soldier_pulls_him_off_his_feet() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    let at = Vec3::new(p.x, ground + HEIGHT / 2.0, p.z - 18.0);
    let soldier = app
        .world_mut()
        .spawn((
            RigidBody::Kinematic,
            Collider::capsule(RADIUS, HEIGHT - 2.0 * RADIUS),
            Transform::from_translation(at),
            Npc::guard(at, 0.0),
        ))
        .id();
    step(&mut app, 10);
    aim(&mut app, at);
    step(&mut app, 1);
    assert_eq!(lock(&app).map(|t| t.kind), Some(TargetKind::Character));
    aim(&mut app, at);
    tap_f(&mut app);
    let mut lean = 0.0f32;
    for _ in 0..64 * 5 {
        step(&mut app, 1);
        if let Some(h) = app.world().get::<Hooked>(soldier) {
            lean = lean.max(h.lean);
        }
        let d = app.world().get::<Transform>(soldier).unwrap().translation;
        if Vec2::new(d.x - p.x, d.z - p.z).length() < 2.5 {
            break;
        }
    }
    let end = app.world().get::<Transform>(soldier).unwrap().translation;
    assert!(
        Vec2::new(end.x - p.x, end.z - p.z).length() < 2.5,
        "soldier at {end:?}, Rico at {p:?}"
    );
    assert!(lean > 0.5, "he stayed on his feet (lean {lean})");
    let (_, h, hp, _) = hero(&mut app);
    assert!(h.grapple.is_none() && hp.distance(p) < 1.0);
    // and up again
    step(&mut app, 64 * 4);
    assert!(app.world().get::<Hooked>(soldier).is_none(), "still down");
    let up = app
        .world()
        .get::<Transform>(soldier)
        .unwrap()
        .rotation
        .angle_between(Quat::IDENTITY);
    assert!(up < 0.05, "tilted {up}");
}

/// The lock-on: the crosshair on a prop or a soldier is a lock; a few degrees off still is
/// (magnetism); well off it is not.
#[test]
fn the_lock_on_picks_the_target_with_magnetism() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    let crate_at = Vec3::new(p.x, ground + 0.5, p.z - 25.0);
    let crate_e = app
        .world_mut()
        .spawn((
            RigidBody::Dynamic,
            Collider::cuboid(1.0, 1.0, 1.0),
            Mass(30.0),
            Transform::from_translation(crate_at),
        ))
        .id();
    let man_at = Vec3::new(p.x + 12.0, ground + HEIGHT / 2.0, p.z - 25.0);
    let man = app
        .world_mut()
        .spawn((
            RigidBody::Kinematic,
            Collider::capsule(RADIUS, HEIGHT - 2.0 * RADIUS),
            Transform::from_translation(man_at),
            Npc::guard(man_at, 0.0),
        ))
        .id();
    step(&mut app, 20);
    // dead centre
    aim(&mut app, crate_at);
    step(&mut app, 1);
    let t = lock(&app).expect("the crate under the crosshair");
    assert_eq!(
        (t.kind, t.body, t.magnet),
        (TargetKind::Object, Some(crate_e), false)
    );
    // 2.5 degrees to the side of its middle: off the crate itself, inside the cone
    let cam = p + Vec3::new(0.0, 1.2, 3.5);
    let to = crate_at - cam;
    let turned = Quat::from_rotation_y(-2.5f32.to_radians()) * to;
    aim(&mut app, cam + turned);
    step(&mut app, 1);
    let t = lock(&app).expect("magnetism should hold the crate");
    assert_eq!((t.kind, t.body), (TargetKind::Object, Some(crate_e)));
    assert!(
        t.magnet && t.angle <= MAGNET_DEG.to_radians() + 1e-4,
        "{t:?}"
    );
    // 12 degrees off: nothing special (the ground, maybe, but no lock)
    let turned = Quat::from_rotation_y(-12.0f32.to_radians()) * to;
    aim(&mut app, cam + turned);
    step(&mut app, 1);
    assert!(
        !app.world().resource::<LockOn>().locked(),
        "{:?}",
        lock(&app)
    );
    // the soldier, 3 degrees off his middle
    let to = man_at - cam;
    let turned = Quat::from_rotation_y(3.0f32.to_radians()) * to;
    aim(&mut app, cam + turned);
    step(&mut app, 1);
    let t = lock(&app).expect("the soldier");
    assert_eq!((t.kind, t.body), (TargetKind::Character, Some(man)));
    assert!(app.world().resource::<LockOn>().locked());
    assert_ne!(app.world().resource::<LockOn>().tint(), Color::WHITE);
}

/// The cable sags while slack and is a straight line when taut.
#[test]
fn a_cable_sags_slack_and_is_straight_taut() {
    let a = Vec3::new(0.0, 10.0, 0.0);
    let b = Vec3::new(30.0, 10.0, 0.0);
    let taut = cable_points(&Cable { a, b, slack: 0.0 });
    for p in &taut {
        assert!((p.y - 10.0).abs() < 1e-5);
    }
    let slack = cable_points(&Cable { a, b, slack: 1.0 });
    assert_eq!(slack[0], a);
    assert!(slack[CABLE_SEGMENTS].distance(b) < 1e-4);
    let mid = slack[CABLE_SEGMENTS / 2];
    assert!(
        (10.0 - mid.y - CABLE_SAG * 30.0).abs() < 0.05,
        "sag {}",
        10.0 - mid.y
    );
}
