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

/// The native wire: an 11-point parabola dipping 1 percent of the length at the middle, the
/// ends fixed, straight when it is not a completed wire (in flight, a pull).
#[test]
fn a_cable_sags_by_the_native_parabola_and_is_straight_otherwise() {
    let a = Vec3::new(0.0, 10.0, 0.0);
    let b = Vec3::new(100.0, 10.0, 0.0);
    let flat = cable_points(&Cable { a, b, sag: false, hooks: false });
    assert_eq!(flat, vec![a, b]);
    let sag = cable_points(&Cable { a, b, sag: true, hooks: false });
    assert_eq!(sag.len(), 11);
    assert!(sag[0].distance(a) < 1e-4 && sag[10].distance(b) < 1e-4);
    // ((2t-1)^2 - 1) * (0.02 * 100) * 0.5 = -1 m at t = 0.5, -0.64 m at t = 0.2
    assert!((10.0 - sag[5].y - 1.0).abs() < 1e-4, "mid sag {}", 10.0 - sag[5].y);
    assert!((10.0 - sag[2].y - 0.64).abs() < 1e-4);
    // a pull's tension factor of 0 draws it straight
    assert!(sag_polyline(a, b, 0.0).iter().all(|p| (p.y - 10.0).abs() < 1e-6));
}

/// The tube: eight sides on a circle of `wire_width * 1.4142`, rebuilt along any polyline.
#[test]
fn the_tube_is_an_octagon_of_the_native_radius() {
    let w = 0.006;
    let r = w * super::cable::TUBE_RADIUS_FACTOR;
    let o = super::cable::octagon(w);
    assert!((o[0] - Vec2::new(r, 0.0)).length() < 1e-6);
    assert!((o[1] - Vec2::new(w, w)).length() < 1e-4, "{:?}", o[1]);
    assert!((o[2] - Vec2::new(0.0, r)).length() < 1e-4);
    assert!(o.iter().all(|p| (p.length() - r).abs() < 1e-6));
    let pts = sag_polyline(Vec3::ZERO, Vec3::new(20.0, 5.0, 3.0), 1.0);
    let mesh = tube_mesh(&pts, w);
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .unwrap()
        .as_float3()
        .unwrap();
    assert_eq!(positions.len(), pts.len() * 9);
    // every ring vertex lies one radius from its polyline point
    for (i, p) in pts.iter().enumerate() {
        for v in &positions[i * 9..i * 9 + 8] {
            assert!((Vec3::from(*v).distance(*p) - r).abs() < 1e-4);
        }
    }
    assert_eq!(mesh.indices().unwrap().len(), (pts.len() - 1) * 8 * 6);
}

/// A cut leaves two pieces aimed at `T = A + normalize(B - A) * 0.5`, shrinking at 100 m/s
/// (full length) rising to 300 m/s, gone at 0.001 m.
#[test]
fn a_cut_wire_snaps_back_in_two_pieces() {
    let a = Vec3::new(1.0, 2.0, 3.0);
    let b = a + Vec3::new(10.0, 0.0, 0.0);
    let [pa, pb] = cut_pieces(a, b);
    assert!((pa.total - 0.5).abs() < 1e-5 && (pb.total - 9.5).abs() < 1e-5);
    assert!(pa.dir.distance(Vec3::X) < 1e-5 && pb.dir.distance(-Vec3::X) < 1e-5);
    let mut p = pb;
    let before = p.len;
    assert!(p.step(0.01));
    // full length: 100 m/s
    assert!((before - p.len - 1.0).abs() < 1e-3, "{}", before - p.len);
    let mut frames = 0;
    while p.step(1.0 / 60.0) {
        frames += 1;
        assert!(frames < 600);
    }
    assert!(frames > 1 && frames < 20, "{frames} frames");
}

// ---------------------------------------------------------------- animation traces
//
// Headless runs of the whole hook (physics, the graph of `anim.rs`, the clips Rico's
// animator ends up playing). Read them with `--nocapture`.

use crate::rig::{Animator, Clip};

fn record(app: &mut App) {
    let mut q = app.world_mut().query::<&mut GrappleRig>();
    q.single_mut(app.world_mut()).unwrap().anim.record = true;
}

/// The clips Rico's animator plays now: the base and the action layer (`None`: no
/// animator in this headless app).
fn playing(app: &mut App) -> Option<(Clip, Option<Clip>)> {
    let mut q = app.world_mut().query_filtered::<&Animator, With<Hero>>();
    q.single(app.world())
        .ok()
        .map(|a| (a.clip, a.layer(crate::rig::Slot::Action).clip))
}

/// Remember each new (base clip, action-layer clip) in order.
fn note_clips(app: &mut App, seen: &mut Vec<(Clip, Option<Clip>)>) {
    if let Some(c) = playing(app)
        && seen.last() != Some(&c)
    {
        seen.push(c);
    }
}

fn layer_states(app: &mut App, layer: u8) -> Vec<&'static str> {
    let r = rig(app);
    let mut out: Vec<&'static str> = Vec::new();
    for s in r.anim.steps.iter().filter(|s| s.layer == layer && s.accepted) {
        if out.last() != Some(&s.to) {
            out.push(s.to);
        }
    }
    out
}

fn body_states(app: &mut App) -> Vec<&'static str> {
    layer_states(app, 0)
}

fn arm_states(app: &mut App) -> Vec<&'static str> {
    layer_states(app, 2)
}

fn print_trace(name: &str, app: &mut App, clips: &[(Clip, Option<Clip>)]) {
    println!("--- {name}\n{}", rig(app).anim.trace());
    println!(
        "    animator: {}\n",
        clips
            .iter()
            .map(|(b, a)| match a {
                Some(a) => format!("{}+{}", b.file(), a.file()),
                None => b.file().to_string(),
            })
            .collect::<Vec<_>>()
            .join(" > ")
    );
}

/// From 20 m up, onto the ground ahead: reel flight (from the air), then the floor landing.
#[test]
fn trace_a_reel_to_the_ground() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    put_hero(&mut app, Vec3::new(p.x, ground + 20.0, p.z));
    step(&mut app, 2);
    let (_, _, p, _) = hero(&mut app);
    record(&mut app);
    aim(&mut app, Vec3::new(p.x, ground, p.z - 25.0));
    tap_f(&mut app);
    let mut clips = Vec::new();
    for _ in 0..64 * 8 {
        step(&mut app, 1);
        note_clips(&mut app, &mut clips);
    }
    print_trace("reel to the ground from the air", &mut app, &clips);
    let states = body_states(&mut app);
    assert_eq!(&states[..3], ["S_REEL_FLIGHT", "S_REEL_END_LAND", "(free)"]);
    let arm = arm_states(&mut app);
    assert_eq!(arm[0], "LA_S_RAISE_GRAPPLE");
    assert!(arm.contains(&"LA_S_GRAPPLE") && arm.contains(&"LA_S_GRAPPLE_TO_IDLE"));
    if !clips.is_empty() {
        let base: Vec<Clip> = clips.iter().map(|c| c.0).collect();
        assert!(base.contains(&Clip::Reel), "{base:?}");
        assert!(base.contains(&Clip::ReelEndLand), "{base:?}");
    }
}

/// From the ground: the take-off first.
#[test]
fn trace_a_reel_from_the_ground_to_a_wall() {
    let Some((mut app, _ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    wall(&mut app, p, 30.0, 80.0);
    record(&mut app);
    aim(&mut app, Vec3::new(p.x, p.y + 12.0, face_z(p, 30.0)));
    tap_f(&mut app);
    let mut clips = Vec::new();
    for _ in 0..64 * 5 {
        step(&mut app, 1);
        note_clips(&mut app, &mut clips);
        if rig(&mut app).phase == Phase::Hang {
            break;
        }
    }
    step(&mut app, 64);
    note_clips(&mut app, &mut clips);
    // Space: the bounce off the wall
    press(&mut app, KeyCode::Space);
    step(&mut app, 1);
    release(&mut app, KeyCode::Space);
    for _ in 0..64 * 2 {
        step(&mut app, 1);
        note_clips(&mut app, &mut clips);
    }
    print_trace("reel from the ground to a wall, then Space", &mut app, &clips);
    let states = body_states(&mut app);
    assert_eq!(
        &states[..5],
        [
            "S_REEL_START",
            "S_REEL_FLIGHT",
            "S_REEL_IMPACT_VERTICAL",
            "S_REELED_IN_IDLE",
            "S_REELED_IN_JUMP"
        ]
    );
    if !clips.is_empty() {
        let base: Vec<Clip> = clips.iter().map(|c| c.0).collect();
        for c in [
            Clip::ReelStart,
            Clip::Reel,
            Clip::ReelImpactVertical,
            Clip::ReeledInIdle,
            Clip::GrappleClimb,
        ] {
            assert!(base.contains(&c), "{c:?} missing in {base:?}");
        }
    }
}

/// A ceiling: the horizontal impact, the transition into the hang, the hang; Ctrl lets go.
#[test]
fn trace_a_reel_to_a_ceiling() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    // a slab 14 m up, wide, 4 m thick
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(60.0, 4.0, 60.0),
        Transform::from_translation(Vec3::new(p.x, ground + 16.0, p.z)),
    ));
    step(&mut app, 3);
    record(&mut app);
    // straight up and a little ahead: the underside faces down
    aim(&mut app, Vec3::new(p.x, ground + 14.0, p.z - 6.0));
    tap_f(&mut app);
    let mut clips = Vec::new();
    for _ in 0..64 * 5 {
        step(&mut app, 1);
        note_clips(&mut app, &mut clips);
        if rig(&mut app).phase == Phase::Hang {
            break;
        }
    }
    assert_eq!(rig(&mut app).phase, Phase::Hang, "never hung from the ceiling");
    assert_eq!(rig(&mut app).arrival, Arrival::Overhang);
    step(&mut app, 64 * 2);
    note_clips(&mut app, &mut clips);
    press(&mut app, KeyCode::ControlLeft);
    step(&mut app, 1);
    release(&mut app, KeyCode::ControlLeft);
    for _ in 0..64 * 2 {
        step(&mut app, 1);
        note_clips(&mut app, &mut clips);
    }
    print_trace("reel to a ceiling, hang, Ctrl", &mut app, &clips);
    let states = body_states(&mut app);
    assert_eq!(
        &states[..6],
        [
            "S_REEL_START",
            "S_REEL_FLIGHT",
            "S_REEL_IMPACT_HORIZONTAL",
            "S_REELED_IN_TO_HANG",
            "S_HANGSTUNT_IDLE",
            "S_HANG_TO_FALL"
        ]
    );
}

/// A barrel on the ground: Rico's left arm pulls, his body is left alone.
#[test]
fn trace_a_pull_on_the_ground() {
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
    record(&mut app);
    let start = app.world().get::<Position>(barrel).unwrap().0;
    aim(&mut app, start);
    tap_f(&mut app);
    let mut clips = Vec::new();
    for _ in 0..64 * 6 {
        step(&mut app, 1);
        note_clips(&mut app, &mut clips);
    }
    print_trace("pull a barrel on the ground", &mut app, &clips);
    assert!(body_states(&mut app).is_empty(), "the body must not be touched");
    let arm = arm_states(&mut app);
    assert!(arm.contains(&"LA_S_GRAPPLE_PULL"), "{arm:?}");
    if !clips.is_empty() {
        let acts: Vec<Clip> = clips.iter().filter_map(|c| c.1).collect();
        assert!(acts.contains(&Clip::GrapplePull), "{acts:?}");
    }
}

/// In the air: Rico flies at the barrel (`S_PULL_IN_AIR_FLIGHT`, the reel-flight clip) until
/// the wire is 6 m shorter, then falls.
#[test]
fn trace_a_pull_in_the_air() {
    let Some((mut app, ground, _)) = setup() else {
        return;
    };
    let (_, _, p, _) = hero(&mut app);
    let at = Vec3::new(p.x, ground + 0.4, p.z - 24.0);
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
    put_hero(&mut app, Vec3::new(p.x, ground + 30.0, p.z));
    step(&mut app, 2);
    record(&mut app);
    let (_, _, p0, _) = hero(&mut app);
    let start = app.world().get::<Position>(barrel).unwrap().0;
    aim(&mut app, start);
    tap_f(&mut app);
    let mut clips = Vec::new();
    for _ in 0..64 * 4 {
        step(&mut app, 1);
        note_clips(&mut app, &mut clips);
    }
    print_trace("pull in the air", &mut app, &clips);
    let states = body_states(&mut app);
    assert_eq!(&states[..2], ["S_PULL_IN_AIR_FLIGHT", "(free)"]);
    let (_, _, p1, _) = hero(&mut app);
    // he flew towards it (horizontally) before falling
    assert!(
        Vec2::new(p1.x - p0.x, p1.z - p0.z).length() > 2.0,
        "Rico was not pulled: {p0:?} -> {p1:?}"
    );
    if !clips.is_empty() {
        let base: Vec<Clip> = clips.iter().map(|c| c.0).collect();
        assert!(base.contains(&Clip::Reel), "{base:?}");
    }
}

/// A soldier: the arm pulls, his hook counter and `S_GRAPPLED` follow.
#[test]
fn trace_a_soldier_on_the_hook() {
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
    record(&mut app);
    aim(&mut app, at);
    tap_f(&mut app);
    let mut log: Vec<(&'static str, u32)> = Vec::new();
    for _ in 0..64 * 8 {
        step(&mut app, 1);
        if let Some(h) = app.world().get::<Hooked>(soldier) {
            let e = (h.victim.name(), h.grappled.0);
            if log.last() != Some(&e) {
                log.push(e);
            }
        }
    }
    println!(
        "--- a soldier on the hook\n{}\nvictim: {log:?}\n",
        rig(&mut app).anim.trace()
    );
    assert_eq!(
        log,
        [
            ("S_GRAPPLED", 1),
            ("S_REMOVE_GRAPPLINGHOOK", 0),
            ("S_AI_BASIC_NAVIGATION", 0)
        ]
    );
    let arm = arm_states(&mut app);
    assert!(arm.contains(&"LA_S_GRAPPLE_PULL"), "{arm:?}");
}

// ---------------------------------------------------------------- JC2-269: the device tick

#[test]
fn the_reel_speed_ramps_from_eleven_to_the_cap() {
    // `v = min((3.5 dt + 1) v, 40)` from 11 m/s: 23 frames at 60 Hz (JC2-246 section 1)
    let dt = 1.0 / 60.0;
    let mut v = REEL_START_SPEED;
    let mut frames = 0;
    while v < 40.0 {
        let (along, r) = reel_tick(&mut v, 1.0e6, dt, 40.0);
        assert_eq!(r, TickResult::Flying);
        assert_eq!(along, v);
        frames += 1;
    }
    assert_eq!(frames, 23);
    assert_eq!(v, 40.0);
    let (along, _) = reel_tick(&mut v, 1.0e6, dt, 40.0);
    assert_eq!(along, 40.0, "no braking, no further change");
}

#[test]
fn the_air_pull_starts_at_six_and_takes_34_frames() {
    let dt = 1.0 / 60.0;
    let mut v = PULL_START_SPEED;
    let mut frames = 0;
    while v < 40.0 {
        reel_tick(&mut v, 1.0e6, dt, 40.0);
        frames += 1;
    }
    assert_eq!(frames, 34);
}

#[test]
fn the_tick_overshoot_lands_exactly_on_the_hook() {
    let dt = 1.0 / 60.0;
    let mut v = 40.0;
    let (along, r) = reel_tick(&mut v, 0.5, dt, 40.0);
    assert_eq!(r, TickResult::Arrived);
    assert!((along * dt - 0.5).abs() < 1e-5);
    // exactly out of range is not an overshoot (remaining == 0 is not < 0)
    let mut v = 40.0;
    let (_, r) = reel_tick(&mut v, 40.0 * dt, dt, 40.0);
    assert_eq!(r, TickResult::Flying);
}

#[test]
fn a_paused_frame_changes_nothing() {
    let mut v = 11.0;
    let (along, r) = reel_tick(&mut v, 10.0, 0.0, 40.0);
    assert_eq!((along, r, v), (11.0, TickResult::Flying, 11.0));
}

#[test]
fn can_reel_in_state_follows_the_state_flags() {
    use anim::{ArmState, BodyState, GrappleAnim};
    let mut a = GrappleAnim::default();
    assert!(a.can_reel_in_state());
    for (s, ok) in [
        (BodyState::ReelFlight, true),
        (BodyState::ReelEndLand, true),
        (BodyState::ReelImpactHorizontal, false),
        (BodyState::ReeledInToHang, false),
        (BodyState::HangStuntIdle, false),
        (BodyState::HangToFall, false),
    ] {
        a.body = s;
        assert_eq!(a.can_reel_in_state(), ok, "{}", s.name());
    }
    a.body = BodyState::Free;
    a.arm = ArmState::GrapplePull;
    assert!(!a.can_reel_in_state());
    assert!(
        !a.reel_start(anim::BodyCtx::Air),
        "no new reel during the ground pull"
    );
}

// ---- effect calls (board task 7bd60e82)

mod fx {
    use super::*;
    use crate::effects::component::AdapterSet;
    use crate::effects::system::{EffectSystem, MemorySource, STATE_DEAD};
    use effects::{FxCall, GrappleFx, Slot};
    use jc2_formats::effect::{
        ComponentDesc, EffectDescription, EffectSettings, InteractionMatrix, PairMap, Variation,
    };
    use jc2_formats::property::Node;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn log(app: &App) -> Vec<FxCall> {
        app.world().resource::<GrappleFx>().log.clone()
    }

    fn count(log: &[FxCall], f: impl Fn(&FxCall) -> bool) -> usize {
        log.iter().filter(|c| f(c)).count()
    }

    fn plays(slot: Slot) -> impl Fn(&FxCall) -> bool {
        move |c| matches!(c, FxCall::PlaySlot { slot: s, .. } if *s == slot)
    }

    /// A looping effect with one component (a counted stub) for each id the grapple uses.
    fn spawner() -> EffectSystem {
        let mut descs: Vec<Option<Arc<EffectDescription>>> = vec![None; 420];
        for id in [11usize, 203, 204, 212, 213, 214, 215, 410] {
            let mut components = BTreeMap::new();
            components.insert(
                0,
                ComponentDesc {
                    uid: 0,
                    role: 100,
                    kind: 0,
                    variations: BTreeMap::from([(
                        1,
                        Variation {
                            uid: 1,
                            node: Node::default(),
                        },
                    )]),
                },
            );
            descs[id] = Some(Arc::new(EffectDescription {
                id: id as u32,
                settings: EffectSettings {
                    looping: true,
                    ..Default::default()
                },
                components,
                matrix: InteractionMatrix {
                    default: PairMap::from([(100, Some((0, 1)))]),
                    flagged: None,
                    materials: BTreeMap::new(),
                },
                inputs: BTreeMap::new(),
            }));
        }
        EffectSystem::new(255, Box::new(MemorySource(descs)), AdapterSet::stubs())
    }

    /// `PlaySlot` plays only a controller that is not playing, `StopSlot` stops only one that
    /// is (`0x00735700`, `0x005E9670`).
    #[test]
    fn play_slot_and_stop_slot_follow_the_native_rules() {
        let mut fx = spawner();
        let mut state = GrappleFx::default();
        let at = Vec3::ZERO;
        let slot = Slot::Reel;
        // stopping a slot that was never played does nothing
        state.push(FxCall::StopSlot(slot));
        state.flush(Some(&mut fx));
        assert_eq!(state.slot_state(&fx, slot), None);
        // play: the controller runs; a second play is ignored
        state.push(FxCall::PlaySlot { slot, at });
        state.flush(Some(&mut fx));
        fx.tick(0.1);
        assert_ne!(state.slot_state(&fx, slot), Some(STATE_DEAD));
        let live = fx.live();
        state.push(FxCall::PlaySlot { slot, at });
        state.flush(Some(&mut fx));
        assert_eq!(fx.live(), live);
        // slots 0 and 1 are separate controllers of the same effect 203
        state.push(FxCall::PlaySlot {
            slot: Slot::Attach,
            at,
        });
        state.flush(Some(&mut fx));
        assert_eq!(fx.live(), live + 1);
        // stop: graceful, and the slot can play again afterwards
        state.push(FxCall::StopSlot(slot));
        state.flush(Some(&mut fx));
        for _ in 0..5 {
            fx.tick(0.1);
        }
        assert_eq!(state.slot_state(&fx, slot), Some(STATE_DEAD));
        state.push(FxCall::PlaySlot { slot, at });
        state.flush(Some(&mut fx));
        fx.tick(0.1);
        assert_ne!(state.slot_state(&fx, slot), Some(STATE_DEAD));
        // the one-shots go through the interaction path
        state.push(FxCall::HookDetach {
            at,
            surface: Some(8),
        });
        state.push(FxCall::HookImpact {
            at,
            normal: Vec3::Y,
            surface: None,
            water: true,
        });
        state.push(FxCall::FlyingHook { at });
        let live = fx.live();
        state.flush(Some(&mut fx));
        fx.tick(0.1);
        assert_eq!(fx.live(), live + 3);
    }

    #[test]
    fn the_cancel_stops_slots_0_to_2_and_the_detach_plays_214_and_213() {
        let hook = Vec3::new(1.0, 2.0, 3.0);
        let mut rig = GrappleRig {
            phase: Phase::Flight,
            hook,
            ..default()
        };
        rig.retract();
        assert_eq!(
            rig.fx,
            vec![
                FxCall::StopSlot(Slot::Reel),
                FxCall::StopSlot(Slot::Attach),
                FxCall::StopSlot(Slot::Parachute),
                FxCall::PlaySlot {
                    slot: Slot::Miss,
                    at: hook
                },
                FxCall::HookDetach {
                    at: hook,
                    surface: None
                },
            ]
        );
        // already home: a second cancel makes no calls
        rig.fx.clear();
        rig.retract();
        assert!(rig.fx.is_empty());
    }

    #[test]
    fn the_cut_calls_are_213_twice_215_stop_410_once() {
        let calls = effects::cut_calls(Vec3::ZERO, Vec3::X, [Some(8), None]);
        assert_eq!(count(&calls, |c| matches!(c, FxCall::HookDetach { .. })), 2);
        assert_eq!(count(&calls, |c| *c == FxCall::StopSlot(Slot::Deployed)), 1);
        assert_eq!(count(&calls, plays(Slot::DetachDual)), 1);
        assert_eq!(
            calls[0],
            FxCall::HookDetach {
                at: Vec3::ZERO,
                surface: Some(8)
            }
        );
    }

    /// Fire, fly, attach, reel, land: `grapplefire` first, 212 and slot 1 at the hit, slot 0
    /// for the reel, its stop at the end, the detach after it.
    #[test]
    fn a_shot_at_a_wall_makes_the_native_calls_in_order() {
        let Some((mut app, _ground, _)) = setup() else {
            return;
        };
        let (_, _, p, _) = hero(&mut app);
        // far enough that the hook is still in the air at 0.19 s
        wall(&mut app, p, 50.0, 80.0);
        aim(&mut app, Vec3::new(p.x, p.y + 10.0, face_z(p, 50.0)));
        app.world_mut().resource_mut::<GrappleFx>().log.clear();
        tap_f(&mut app);
        let mut frames = 0;
        while rig(&mut app).phase != Phase::Idle && frames < 1200 {
            step(&mut app, 1);
            frames += 1;
            // a wall hang ends with a press of the jump key
            if rig(&mut app).phase == Phase::Hang && frames % 200 == 0 {
                press(&mut app, KeyCode::Space);
                step(&mut app, 3);
                release(&mut app, KeyCode::Space);
            }
        }
        let l = log(&app);
        let pos = |f: &dyn Fn(&FxCall) -> bool| l.iter().position(f);
        let fire = pos(&|c| matches!(c, FxCall::FlyingHook { .. })).expect("grapplefire");
        let impact = pos(&|c| matches!(c, FxCall::HookImpact { .. })).expect("hook_impact");
        let reel = pos(&plays(Slot::Reel)).expect("reel start");
        let stop = pos(&|c| *c == FxCall::StopSlot(Slot::Reel)).expect("reel end");
        assert_eq!(count(&l, |c| matches!(c, FxCall::FlyingHook { .. })), 1);
        assert_eq!(count(&l, |c| matches!(c, FxCall::HookImpact { .. })), 1);
        assert_eq!(count(&l, plays(Slot::Attach)), 1);
        assert!(fire < impact && impact <= reel && reel < stop, "{l:?}");
        assert!(
            l.iter().any(plays(Slot::Miss)),
            "device detach after the reel: {l:?}"
        );
    }

    #[test]
    fn grapplefire_comes_at_0_19_s_of_the_fire_clip() {
        let Some((mut app, _ground, _)) = setup() else {
            return;
        };
        let (_, _, p, _) = hero(&mut app);
        wall(&mut app, p, 60.0, 80.0);
        aim(&mut app, Vec3::new(p.x, p.y + 10.0, face_z(p, 60.0)));
        app.world_mut().resource_mut::<GrappleFx>().log.clear();
        tap_f(&mut app);
        let fired = |app: &App| count(&log(app), |c| matches!(c, FxCall::FlyingHook { .. }));
        let mut frames = 1; // tap_f steps one frame
        while fired(&app) == 0 && frames < 64 {
            step(&mut app, 1);
            frames += 1;
        }
        let t = frames as f32 / 64.0;
        assert!(
            (t - effects::GRAPPLEFIRE_AT).abs() < 3.0 / 64.0,
            "event at {t} s"
        );
    }
}
