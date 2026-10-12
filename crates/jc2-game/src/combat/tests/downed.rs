use super::*;
use crate::character::{HeroInput, HeroModel};
use crate::grapple::{GrappleRig, Phase};
use crate::stunts::tests::{hero, press, release, rico_app, step};

fn downed_app() -> Option<(App, Vec3)> {
    let (mut app, _, spawn) = rico_app(
        &[
            "weapons.bin",
            "weapon_upgrades.bin",
            "ammunition.bin",
            "biped.bsk",
        ],
        |app| {
            app.add_plugins((CombatPlugin, crate::crew::CrewPlugin));
        },
    )?;
    crate::fleet::testing::wait_for_hero(&mut app);
    Some((app, spawn))
}

fn body(app: &mut App) -> Transform {
    *app.world_mut()
        .query_filtered::<&Transform, With<HeroModel>>()
        .single(app.world())
        .unwrap()
}

fn assert_down(app: &mut App) {
    let (entity, h, _, _) = hero(app);
    assert!(h.downed);
    assert!(app.world().get::<Dead>(entity).is_some());
    assert_eq!((h.in_vehicle, h.stunt, h.grapple), (None, None, None));
    assert!(!h.hanging && !h.parachute && !h.swimming);
    assert_ne!(
        *app.world().get::<Visibility>(entity).unwrap(),
        Visibility::Hidden
    );
    let physics = app
        .world()
        .get::<crate::ragdoll::PhysicsPose>(entity)
        .is_some();
    if physics {
        assert_eq!(
            *app.world().get::<RigidBody>(entity).unwrap(),
            RigidBody::Kinematic
        );
        assert_eq!(
            *app.world().get::<CollisionLayers>(entity).unwrap(),
            CollisionLayers::NONE
        );
    } else {
        assert!(
            (body(app).rotation * Vec3::Y).y.abs() < 0.02,
            "fallback body should lie flat"
        );
        let an = app.world().get::<Animator>(entity).unwrap();
        assert_eq!(an.rate, 0.0);
        assert_eq!(an.clip, Clip::Idle);
    }
    let an = app.world().get::<Animator>(entity).unwrap();
    assert!(an.layers.iter().all(|layer| layer.clip.is_none()));
    assert_eq!(an.aim.weight, 0.0);
    assert_eq!(
        app.world().get::<Rotation>(entity).unwrap().0,
        Quat::IDENTITY,
        "only the visual child topples"
    );
}

#[test]
fn downed_body_stays_visible_blocks_input_and_recovers_after_respawn() {
    let Some((mut app, spawn)) = downed_app() else {
        return;
    };
    let entity = hero(&mut app).0;
    app.world_mut().get_mut::<Health>(entity).unwrap().hp = 0.0;
    let before = {
        let a = app.world().resource::<Arsenal>();
        (a.shots, a.grenades, a.current, a.gun().ammo.magazine)
    };
    // Fresh edges every frame exercise direct E/Q/F/G handlers too, while W and
    // trigger remain held. A death must not consume ammo or reopen the chute.
    for _ in 0..64 {
        for key in [
            KeyCode::KeyW,
            KeyCode::Space,
            KeyCode::KeyE,
            KeyCode::KeyQ,
            KeyCode::KeyF,
            KeyCode::KeyG,
            KeyCode::KeyR,
            KeyCode::Digit2,
        ] {
            press(&mut app, key);
        }
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        step(&mut app, 1);
    }
    assert_down(&mut app);
    let a = app.world().resource::<Arsenal>();
    assert_eq!(
        (a.shots, a.grenades, a.current, a.gun().ammo.magazine),
        before
    );
    let input = app.world().resource::<HeroInput>();
    assert_eq!(input.movement, Vec3::ZERO);
    assert!(!input.jump && !input.chute_toggle);
    let fallen = hero(&mut app).2;
    for _ in 0..64 {
        press(&mut app, KeyCode::KeyW);
        step(&mut app, 1);
    }
    let still = hero(&mut app).2;
    if app
        .world()
        .get::<crate::ragdoll::PhysicsPose>(entity)
        .is_some()
    {
        assert!(still.is_finite());
        assert_eq!(app.world().resource::<HeroInput>().movement, Vec3::ZERO);
        assert_eq!(
            (
                app.world().resource::<Arsenal>().shots,
                app.world().resource::<Arsenal>().grenades
            ),
            (before.0, before.1)
        );
    } else {
        assert!(Vec2::new(still.x - fallen.x, still.z - fallen.z).length() < 0.05);
    }
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .reset_all();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .reset_all();
    step(&mut app, 64 * 3);
    crate::fleet::testing::wait_for_hero(&mut app);
    let (_, h, at, _) = hero(&mut app);
    assert!(!h.downed && h.grounded);
    assert!(app.world().get::<Dead>(entity).is_none());
    assert!(at.distance(spawn) < 10.0);
    assert!((body(&mut app).rotation * Vec3::Y).y > 0.99);
    assert!(app.world().get::<Animator>(entity).unwrap().rate > 0.0);
    for _ in 0..64 {
        press(&mut app, KeyCode::KeyW);
        step(&mut app, 1);
    }
    assert!(
        hero(&mut app).2.distance(at) > 2.0,
        "movement must return after respawn"
    );
    release(&mut app, KeyCode::KeyW);
}

#[test]
fn downed_cancels_grapple_ledge_pull_and_vehicle_tracking() {
    let Some((mut app, _)) = downed_app() else {
        return;
    };
    let (entity, _, at, _) = hero(&mut app);
    let target = app
        .world_mut()
        .spawn(crate::grapple::Hooked {
            speed: 0.0,
            lean: 0.0,
            down: 0.0,
            toward: Vec3::NEG_Z,
            pulled: true,
            grappled: Default::default(),
            victim: Default::default(),
        })
        .id();
    {
        let mut h = app.world_mut().get_mut::<Hero>(entity).unwrap();
        h.grapple = Some(crate::character::Grapple {
            anchor: at + Vec3::Y * 2.0,
            normal: Vec3::Z,
            age: 0.0,
        });
        h.hanging = true;
        h.parachute = true;
    }
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::stunts::HookedVehicle {
            vehicle: target,
            local: Vec3::ZERO,
        });
    {
        let mut g = app.world_mut().get_mut::<GrappleRig>(entity).unwrap();
        g.phase = Phase::Hang;
        g.ledge_top = Some(at.y + 2.0);
        g.climb = 0.4;
        g.track = Some((target, Vec3::ZERO));
        g.pull = Some(crate::grapple::PullInfo {
            body: target,
            local: Vec3::ZERO,
            soldier: true,
            age: 0.0,
            fresh: false,
        });
    }
    app.world_mut().get_mut::<Health>(entity).unwrap().hp = 0.0;
    step(&mut app, 64);
    assert_down(&mut app);
    let g = app.world().get::<GrappleRig>(entity).unwrap();
    assert_eq!(g.phase, Phase::Idle);
    assert!(g.flight.is_none() && g.pull.is_none() && g.track.is_none() && g.ledge_top.is_none());
    assert_eq!(g.climb, 0.0);
    assert!(
        app.world()
            .get::<crate::stunts::HookedVehicle>(entity)
            .is_none()
    );
    assert!(app.world().get::<crate::grapple::Hooked>(target).is_none());
}
