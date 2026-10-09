use super::*;
use crate::rig::RigData;
use std::{collections::HashMap, time::Duration};

fn original() -> Option<Ragdoll> {
    let dir = std::env::var("JC2_TEST_DIR").ok()?;
    let fs = jc2_formats::GameFs::open(dir).unwrap();
    Some(Ragdoll::parse(&fs.read_inflated("triplemapped_normal_ragdoll.brd").unwrap()).unwrap())
}
fn fixture(asset: Ragdoll, root: Transform) -> (App, Entity) {
    let fs = jc2_formats::GameFs::open(std::env::var("JC2_TEST_DIR").unwrap()).unwrap();
    let skeleton = Skeleton::parse(&fs.read_inflated("biped.bsk").unwrap()).unwrap();
    let mut app = crate::physics::tests::headless_app();
    app.insert_resource(RagdollAsset(Arc::new(asset)));
    app.insert_resource(Gravity(Vec3::NEG_Y * 9.81));
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        Duration::from_secs_f32(1.0 / 64.0),
    ));
    app.add_systems(PreUpdate, cleanup);
    app.add_systems(
        PostUpdate,
        reconstruct.before(bevy::transform::TransformSystems::Propagate),
    );
    app.add_systems(
        PostUpdate,
        activate.after(bevy::transform::TransformSystems::Propagate),
    );
    let mut character = Hero::default();
    character.downed = true;
    let hero = app
        .world_mut()
        .spawn((
            character,
            root,
            RigidBody::Dynamic,
            Collider::capsule(0.3, 1.2),
            GravityScale(0.0),
            CollisionLayers::default(),
        ))
        .id();
    let model = app
        .world_mut()
        .spawn((HeroModel, Transform::from_xyz(0., -0.9, 0.)))
        .id();
    app.world_mut().entity_mut(hero).add_child(model);
    let mut joints = Vec::new();
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let e = app
            .world_mut()
            .spawn((
                qs_to_bevy(&skeleton.reference_pose[i]),
                crate::rig::Joint(i),
            ))
            .id();
        let parent = bone.parent.map_or(model, |p| joints[p]);
        app.world_mut().entity_mut(parent).add_child(e);
        joints.push(e);
    }
    app.world_mut().entity_mut(hero).insert(Animator::new(
        Arc::new(RigData {
            file: "biped.bsk".into(),
            skeleton,
            clips: HashMap::new(),
        }),
        joints,
    ));
    app.update();
    (app, hero)
}
fn trigger(app: &mut App, hero: Entity, request: PendingRagdoll) {
    app.world_mut().entity_mut(hero).insert(request);
    app.update();
    assert!(
        app.world().get::<PhysicsPose>(hero).is_some(),
        "original rig must instantiate"
    );
}
fn owned_count(app: &mut App) -> usize {
    app.world_mut().query::<&Owned>().iter(app.world()).count()
}
#[test]
fn original_mapper_rest_and_posed_world_round_trip() {
    let Some(asset) = original() else { return };
    let root = Transform::from_xyz(110., 7., -40.).with_rotation(Quat::from_rotation_y(1.2));
    let (mut app, hero) = fixture(asset, root);
    // Change a driven shoulder and an unmapped descendant, then propagate before activation.
    let joints = app.world().get::<Animator>(hero).unwrap().joints.clone();
    let shoulder = app
        .world()
        .get::<Animator>(hero)
        .unwrap()
        .rig
        .skeleton
        .bone_index("LeftArm")
        .unwrap();
    app.world_mut()
        .get_mut::<Transform>(joints[shoulder])
        .unwrap()
        .rotation *= Quat::from_rotation_x(0.22);
    app.update();
    let before: Vec<_> = joints
        .iter()
        .map(|&e| app.world().get::<GlobalTransform>(e).unwrap().to_matrix())
        .collect();
    trigger(
        &mut app,
        hero,
        PendingRagdoll {
            origin: root.translation,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        },
    );
    assert_eq!(owned_count(&mut app), 27);
    reconstruct(app.world_mut());
    // Check local hierarchy directly before a physics step changes the captured pose.
    let an = app.world().get::<Animator>(hero).unwrap();
    let model = app.world().get::<Assembly>(hero).unwrap().model;
    let model_world = app.world().get::<Transform>(hero).unwrap().to_matrix()
        * app.world().get::<Transform>(model).unwrap().to_matrix();
    let mut worlds = Vec::new();
    for (i, bone) in an.rig.skeleton.bones.iter().enumerate() {
        let parent = bone.parent.map_or(model_world, |p| worlds[p]);
        let pose = parent * app.world().get::<Transform>(joints[i]).unwrap().to_matrix();
        assert!(
            pose.abs_diff_eq(before[i], 0.0002),
            "bone {i} snapped during posed activation"
        );
        worlds.push(pose);
    }
}
#[test]
fn moving_support_point_velocity_and_idempotent_activation() {
    let Some(asset) = original() else { return };
    let root = Transform::from_xyz(0., 10., 0.);
    let (mut app, hero) = fixture(asset, root);
    let request = PendingRagdoll {
        origin: Vec3::new(3., 10., 0.),
        linear: Vec3::new(5., 1., -2.),
        angular: Vec3::new(0., 0.7, 0.),
    };
    trigger(&mut app, hero, request);
    let assembly = app.world().get::<Assembly>(hero).unwrap();
    for &e in &assembly.bodies {
        let p = app.world().get::<Position>(e).unwrap().0;
        let r = app.world().get::<Rotation>(e).unwrap().0;
        let center = app.world().get::<CenterOfMass>(e).unwrap().0;
        let velocity = app.world().get::<LinearVelocity>(e).unwrap().0;
        assert!(velocity.abs_diff_eq(point_velocity(request, p + r * center), 0.0001));
        assert_eq!(
            app.world().get::<AngularVelocity>(e).unwrap().0,
            request.angular
        );
        assert!(app.world().get::<ChildOf>(e).is_none());
    }
    trigger(&mut app, hero, request);
    assert_eq!(owned_count(&mut app), 27);
}
#[test]
fn original_constraints_fall_settle_and_repeat_cleanup_in_physics_schedule() {
    let Some(asset) = original() else { return };
    let (mut app, hero) = fixture(asset, Transform::from_xyz(0., 6., 0.));
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(80., 2., 80.),
        Transform::from_xyz(0., -1., 0.),
    ));
    for cycle in 0..3 {
        app.world_mut().get_mut::<Hero>(hero).unwrap().downed = true;
        trigger(
            &mut app,
            hero,
            PendingRagdoll {
                origin: Vec3::ZERO,
                linear: Vec3::new(1., -4., 0.),
                angular: Vec3::ZERO,
            },
        );
        let original_rotations: Vec<_> = app
            .world()
            .get::<Assembly>(hero)
            .unwrap()
            .bodies
            .iter()
            .map(|&e| app.world().get::<Rotation>(e).unwrap().0)
            .collect();
        for _ in 0..384 {
            app.update();
        }
        let bodies = app.world().get::<Assembly>(hero).unwrap().bodies.clone();
        let hips_delta =
            app.world().get::<Rotation>(bodies[0]).unwrap().0 * original_rotations[0].inverse();
        let independently_moved = bodies
            .iter()
            .enumerate()
            .filter(|(i, e)| {
                let delta =
                    app.world().get::<Rotation>(**e).unwrap().0 * original_rotations[*i].inverse();
                delta.angle_between(hips_delta) > 0.1
            })
            .count();
        assert!(
            independently_moved >= 4,
            "assembly fell as a rigid visual substitute"
        );
        for e in bodies {
            let p = app.world().get::<Position>(e).unwrap().0;
            let velocity = app.world().get::<LinearVelocity>(e).unwrap().0;
            assert!(
                p.is_finite() && velocity.is_finite(),
                "cycle {cycle}: nonfinite body"
            );
            assert!(
                p.y > -0.5 && p.y < 3.,
                "cycle {cycle}: body failed to settle at {p:?}"
            );
            assert!(
                velocity.length() < 3.,
                "cycle {cycle}: unsettled {velocity:?}"
            );
        }
        let mut max_drift = 0.0f32;
        let mut pairs: Vec<_> = app
            .world_mut()
            .query::<&SphericalJoint>()
            .iter(app.world())
            .map(|j| {
                (
                    j.body1,
                    j.body2,
                    j.local_frame1().unwrap(),
                    j.local_frame2().unwrap(),
                )
            })
            .collect();
        pairs.extend(
            app.world_mut()
                .query::<&RevoluteJoint>()
                .iter(app.world())
                .map(|j| {
                    (
                        j.body1,
                        j.body2,
                        j.local_frame1().unwrap(),
                        j.local_frame2().unwrap(),
                    )
                }),
        );
        for (a, b, fa, fb) in pairs {
            let pa = app.world().get::<Position>(a).unwrap().0
                + app.world().get::<Rotation>(a).unwrap().0 * Vec3::from(fa.translation);
            let pb = app.world().get::<Position>(b).unwrap().0
                + app.world().get::<Rotation>(b).unwrap().0 * Vec3::from(fb.translation);
            max_drift = max_drift.max(pa.distance(pb));
        }
        eprintln!("ragdoll cycle {cycle} maximum joint pivot drift {max_drift:.6} m");
        assert!(max_drift < 0.15);
        app.world_mut().get_mut::<Hero>(hero).unwrap().downed = false;
        app.update();
        assert_eq!(owned_count(&mut app), 0);
        assert!(app.world().get::<PhysicsPose>(hero).is_none());
        assert_eq!(
            *app.world().get::<RigidBody>(hero).unwrap(),
            RigidBody::Dynamic
        );
        for _ in 0..4 {
            app.update();
        } // stale graph edges panic here rather than pass component-only checks.
    }
    app.world_mut().get_mut::<Hero>(hero).unwrap().downed = true;
    trigger(
        &mut app,
        hero,
        PendingRagdoll {
            origin: Vec3::ZERO,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        },
    );
    app.world_mut().despawn(hero);
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(owned_count(&mut app), 0);
}

#[test]
fn production_fatal_seated_support_captures_motion_and_respawns_with_no_owned_graph() {
    use crate::{
        combat::{CombatPlugin, Health},
        vehicle::{CameraMode, Enterable, Occupied},
    };
    let Some((mut app, _, spawn)) = crate::stunts::tests::rico_app(
        &[
            "triplemapped_normal_ragdoll.brd",
            "biped.bsk",
            "weapons.bin",
            "weapon_upgrades.bin",
            "ammunition.bin",
        ],
        |app| {
            app.add_plugins(CombatPlugin);
        },
    ) else {
        return;
    };
    crate::fleet::testing::wait_for_hero(&mut app);
    let hero = crate::stunts::tests::hero(&mut app).0;
    let velocity = Vec3::new(8., 0., -3.);
    let angular = Vec3::new(0., 0.6, 0.);
    let center = Vec3::new(0.2, 0.1, 0.);
    let support = app
        .world_mut()
        .spawn((
            RigidBody::Kinematic,
            Transform::from_translation(spawn + Vec3::Y * 4.),
            GravityScale(0.),
            LinearVelocity(velocity),
            AngularVelocity(angular),
            CenterOfMass(center),
            Mass(2.0),
            Enterable { reach: 1.5 },
            Occupied,
        ))
        .id();
    app.world_mut().spawn((
        Mass(2.0),
        CenterOfMass(Vec3::ZERO),
        Transform::from_xyz(2.0, 0.0, 0.0),
        ChildOf(support),
    ));
    app.world_mut().get_mut::<Hero>(hero).unwrap().in_vehicle = Some(support);
    *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Drive;
    crate::physics::stow_passenger(&mut app.world_mut().commands(), hero, support);
    app.world_mut().flush();
    app.update(); // propagate the live seated animation and vehicle pose first
    app.world_mut().get_mut::<Health>(hero).unwrap().hp = 0.;
    app.update(); // actual CombatPlugin hero_down producer, not injected PendingRagdoll
    assert!(
        app.world().get::<PhysicsPose>(hero).is_some(),
        "fatal producer must activate original rig"
    );
    let support_tf = *app.world().get::<Transform>(support).unwrap();
    let computed_center = app.world().get::<ComputedCenterOfMass>(support).unwrap().0;
    assert!(
        computed_center.distance(center) > 0.1,
        "contributing child must distinguish solver COM from authored COM"
    );
    let request = PendingRagdoll {
        origin: support_tf.transform_point(computed_center),
        linear: velocity,
        angular,
    };
    let bodies = app.world().get::<Assembly>(hero).unwrap().bodies.clone();
    for e in bodies {
        let p = app.world().get::<Position>(e).unwrap().0;
        let r = app.world().get::<Rotation>(e).unwrap().0;
        let c = app.world().get::<CenterOfMass>(e).unwrap().0;
        assert!(
            app.world()
                .get::<LinearVelocity>(e)
                .unwrap()
                .0
                .abs_diff_eq(point_velocity(request, p + r * c), 0.002)
        );
        assert_eq!(app.world().get::<AngularVelocity>(e).unwrap().0, angular);
    }
    assert_eq!(*app.world().resource::<CameraMode>(), CameraMode::Hero);
    assert!(app.world().get::<Occupied>(support).is_none());
    assert!(app.world().get::<crate::physics::Stowed>(hero).is_none());
    assert!(app.world().get::<Hero>(hero).unwrap().in_vehicle.is_none());
    for _ in 0..300 {
        app.update();
    }
    crate::fleet::testing::wait_for_hero(&mut app);
    assert_eq!(owned_count(&mut app), 0);
    assert!(app.world().get::<PhysicsPose>(hero).is_none());
    assert_eq!(
        *app.world().get::<RigidBody>(hero).unwrap(),
        RigidBody::Dynamic
    );
    assert_ne!(
        *app.world().get::<CollisionLayers>(hero).unwrap(),
        CollisionLayers::NONE
    );
    // A live wire stores world velocity at Rico's capsule centre, already including
    // pivot motion. Fatal release must preserve it without adding support v/omega twice.
    let wire_origin = spawn + Vec3::Y * 3.;
    let wire_velocity = Vec3::new(-2., 1., 7.);
    app.world_mut().entity_mut(hero).insert((
        Transform::from_translation(wire_origin),
        Position(wire_origin),
        LinearVelocity(wire_velocity),
        crate::stunts::WireSwing {
            pos: wire_origin,
            vel: wire_velocity,
        },
    ));
    app.world_mut().get_mut::<Hero>(hero).unwrap().stunt = Some(support);
    app.world_mut().get_mut::<Health>(hero).unwrap().hp = 0.;
    app.update();
    assert!(app.world().get::<PhysicsPose>(hero).is_some());
    let request = PendingRagdoll {
        origin: wire_origin,
        linear: wire_velocity,
        angular,
    };
    for &e in &app.world().get::<Assembly>(hero).unwrap().bodies {
        let p = app.world().get::<Position>(e).unwrap().0;
        let r = app.world().get::<Rotation>(e).unwrap().0;
        let c = app.world().get::<CenterOfMass>(e).unwrap().0;
        assert!(
            app.world()
                .get::<LinearVelocity>(e)
                .unwrap()
                .0
                .abs_diff_eq(point_velocity(request, p + r * c), 0.002),
            "wire world momentum lost or applied twice"
        );
    }
    assert!(app.world().get::<crate::stunts::WireSwing>(hero).is_none());
    assert!(app.world().get::<Hero>(hero).unwrap().stunt.is_none());
    for _ in 0..300 {
        app.update();
    }
    crate::fleet::testing::wait_for_hero(&mut app);
    assert_eq!(owned_count(&mut app), 0);
    // Explicit missing-asset branch still retains the visible rigid fallback.
    app.world_mut().remove_resource::<RagdollAsset>();
    app.world_mut().get_mut::<Health>(hero).unwrap().hp = 0.;
    for _ in 0..64 {
        app.update();
    }
    assert!(app.world().get::<Hero>(hero).unwrap().downed);
    assert!(app.world().get::<PhysicsPose>(hero).is_none());
    assert_eq!(app.world().get::<Animator>(hero).unwrap().rate, 0.);
    assert_eq!(owned_count(&mut app), 0);
}
