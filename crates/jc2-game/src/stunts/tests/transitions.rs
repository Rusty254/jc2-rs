use super::*;
use bevy::ecs::system::RunSystemOnce;

fn original_spots(container: &str) -> Option<StuntSpots> {
    let dir = std::env::var("JC2_TEST_DIR").ok()?;
    let game = Game {
        fs: std::sync::Arc::new(jc2_formats::GameFs::open(dir).unwrap()),
    };
    let spots = StuntSpots(read_spots(Some(&game), container));
    assert!(
        !spots.0.is_empty(),
        "missing required original fixture {container}"
    );
    Some(spots)
}

#[test]
fn authored_actions_keep_identifiers_states_and_exact_destinations() {
    use jc2_formats::property::Variant;
    let Some(mut jeep) = original_spots("lave.v014_jeep.ee") else {
        return;
    };
    let roof = jeep
        .0
        .iter()
        .find(|s| s.name == "SO_Hardtop_RD")
        .unwrap()
        .clone();
    assert_eq!(roof.authored.object_id, Some(365841728));
    let action = roof
        .authored
        .actions
        .iter()
        .find(|a| a.name() == "RoofToFrontClingAction")
        .unwrap();
    assert_eq!(action.destination_id(), Some(-1607847811));
    assert_eq!(
        action.raw.value_hash(0x3eff86a9),
        Some(&Variant::Str("vhl.frontcling.enter".into()))
    );
    assert_eq!(
        action.raw.value_hash(0x5d655631),
        Some(&Variant::Str("S_PERFORMING_STUNT".into()))
    );
    assert_eq!(
        action.raw.value_hash(0xd0003e19),
        Some(&Variant::Str("S_CLING_STUNTPOSITION_IDLE".into()))
    );
    assert!(action.raw.value("world").is_some() && action.raw.value("local").is_some());
    assert!(action.raw.value_hash(0x44dcb6a3).is_some()); // authored UUID
    let front = jeep
        .0
        .iter()
        .find(|s| s.kind == StuntKind::Cling)
        .unwrap()
        .clone();
    assert_eq!(transfer_options(&front, &jeep).len(), 2);
    let on = OnStunt {
        vehicle: Entity::PLACEHOLDER,
        local: front.centre(),
        kind: front.kind,
        wire: None,
    };
    assert_eq!(
        transfer_targets(&on, &CurrentStuntSpot(front.authored.object_id), &jeep),
        (Some("hood"), Some("roof"))
    );
    assert_eq!(
        transfer_targets(
            &OnStunt {
                wire: Some(6.0),
                ..on
            },
            &CurrentStuntSpot(front.authored.object_id),
            &jeep
        ),
        (None, None)
    );

    jeep.0.reverse(); // decoded child order cannot change the associations
    let options = transfer_options(&front, &jeep);
    assert_eq!(options[0].0.name(), "FrontToHood");
    assert_eq!(options[0].1.authored.object_id, Some(238058710));
    assert_eq!(options[1].1.authored.object_id, Some(365841728));
    let duplicate = options[0].1.clone();
    jeep.0.push(duplicate);
    assert_eq!(
        transfer_options(&front, &jeep).len(),
        1,
        "ambiguous ID accepted"
    );
    jeep.0.retain(|s| s.kind != StuntKind::Roof);
    assert!(
        transfer_options(&front, &jeep).is_empty(),
        "dangling link guessed"
    );
    for name in ["lave.v046_buscoach.ee", "arve.v060_attackheli.ee"] {
        let spots = original_spots(name).unwrap();
        let actions: Vec<_> = spots.0.iter().flat_map(|s| &s.authored.actions).collect();
        assert!(actions.iter().any(|a| a.name() == "RoofToDriverAction"));
        assert!(actions.iter().any(|a| a.destination_id().is_some()));
        assert!(
            spots
                .0
                .iter()
                .all(|s| transfer_options(s, &spots).is_empty()),
            "unknown action enabled"
        );
        assert!(actions.iter().all(|a| a.raw.str("_class").is_some()));
    }
}

fn moving_app(spots: StuntSpots) -> (App, Entity, Entity) {
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
    .insert_resource(Gravity(Vec3::ZERO))
    .insert_resource(CameraMode::Hero)
    .init_resource::<ButtonInput<KeyCode>>()
    .add_observer(stunt_removed)
    .add_systems(Update, mount_input)
    .add_systems(FixedPostUpdate, follow_vehicle.in_set(PhysicsSystems::Last));
    app.finish();
    let vehicle = app
        .world_mut()
        .spawn((
            RigidBody::Kinematic,
            Collider::cuboid(2., 1., 4.),
            Transform::from_xyz(0., 20., 0.),
            LinearVelocity(Vec3::new(4., 1., -3.)),
            AngularVelocity(Vec3::new(0.1, 0.6, 0.2)),
            CenterOfMass(Vec3::new(0.2, -0.5, 0.3)),
            spots,
        ))
        .id();
    let hero = app
        .world_mut()
        .spawn((
            Hero::default(),
            RigidBody::Dynamic,
            Collider::capsule(0.3, 1.2),
            Transform::from_xyz(10., 20., 0.),
        ))
        .id();
    app.update();
    (app, vehicle, hero)
}

fn mount(app: &mut App, vehicle: Entity, entity: Entity, spot: StuntSpot) {
    let (pos, rot) = car_pose(app, vehicle);
    app.world_mut()
        .run_system_once(
            move |mut commands: Commands, mut heroes: Query<&mut Hero>| {
                attach(
                    &mut commands,
                    entity,
                    &mut heroes.get_mut(entity).unwrap(),
                    vehicle,
                    pos,
                    rot,
                    Vec3::ZERO,
                    &spot,
                );
            },
        )
        .unwrap();
}

#[test]
fn original_transfers_follow_rotating_frame_and_cancel_with_point_velocity() {
    let Some(spots) = original_spots("lave.v014_jeep.ee") else {
        return;
    };
    let roof = spots
        .0
        .iter()
        .find(|s| s.name == "SO_Hardtop_RD")
        .unwrap()
        .clone();
    let (mut app, vehicle, entity) = moving_app(spots);
    mount(&mut app, vehicle, entity, roof.clone());
    step(&mut app, 2);
    press(&mut app, KeyCode::KeyZ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyZ);
    assert!(app.world().get::<StuntTransfer>(entity).is_some());
    for _ in 0..48 {
        step(&mut app, 1);
        let on = app.world().get::<OnStunt>(entity).unwrap();
        let (p, r) = car_pose(&app, vehicle);
        assert!(pos(&app, entity).distance(p + r * on.local) < 0.001);
        assert!(
            app.world()
                .get::<LinearVelocity>(entity)
                .unwrap()
                .is_finite()
        );
    }
    assert_eq!(
        app.world().get::<CurrentStuntSpot>(entity).unwrap().0,
        Some(-1607847811)
    );
    assert!(app.world().get::<StuntTransfer>(entity).is_none());
    assert_eq!(
        app.world().get::<OnStunt>(entity).unwrap().kind,
        StuntKind::Cling
    );
    let (p, r) = car_pose(&app, vehicle);
    let center = app.world().get::<ComputedCenterOfMass>(vehicle).unwrap().0;
    let linear = app.world().get::<LinearVelocity>(vehicle).unwrap().0;
    let angular = app.world().get::<AngularVelocity>(vehicle).unwrap().0;
    let expected_point = linear + angular.cross(pos(&app, entity) - p - r * center);
    assert!(
        app.world()
            .get::<LinearVelocity>(entity)
            .unwrap()
            .0
            .distance(expected_point)
            < 0.001,
        "point velocity used transform origin instead of support COM"
    );

    // Shift+Z chooses the other explicitly linked action (FrontToRoof).
    press(&mut app, KeyCode::ShiftLeft);
    press(&mut app, KeyCode::KeyZ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyZ);
    release(&mut app, KeyCode::ShiftLeft);
    step(&mut app, 48);
    assert_eq!(
        app.world().get::<CurrentStuntSpot>(entity).unwrap().0,
        roof.authored.object_id
    );
    press(&mut app, KeyCode::KeyZ);
    step(&mut app, 1);
    release(&mut app, KeyCode::KeyZ);
    step(&mut app, 12);
    let expected = app.world().get::<LinearVelocity>(entity).unwrap().0;
    press(&mut app, KeyCode::KeyQ);
    app.world_mut().run_system_once(mount_input).unwrap();
    release(&mut app, KeyCode::KeyQ);
    assert!(app.world().get::<OnStunt>(entity).is_none());
    assert!(app.world().get::<StuntTransfer>(entity).is_none());
    assert!(app.world().get::<CurrentStuntSpot>(entity).is_none());
    assert!(
        app.world()
            .get::<LinearVelocity>(entity)
            .unwrap()
            .0
            .distance(expected)
            < 0.001
    );
    assert_ne!(
        *app.world().get::<CollisionLayers>(entity).unwrap(),
        CollisionLayers::NONE
    );
    assert!(app.world().get::<crate::physics::Stowed>(entity).is_none());
    // Support removal releases the same capsule without disabling physics graph nodes.
    mount(&mut app, vehicle, entity, roof);
    step(&mut app, 2);
    app.world_mut().entity_mut(vehicle).despawn();
    step(&mut app, 2);
    assert!(app.world().get::<OnStunt>(entity).is_none());
    assert!(app.world().get::<crate::physics::Stowed>(entity).is_none());
}

#[test]
fn wire_tilt_has_vertical_rest_dt_smoothing_and_support_cleanup() {
    let mut q = Quat::IDENTITY;
    for _ in 0..128 {
        q = smooth_wire_tilt(q, Vec3::Y, 1. / 64.);
    }
    assert!(q.angle_between(Quat::IDENTITY) < 1e-6);
    let up = Vec3::new(0.4, 1., -0.2).normalize();
    let whole = smooth_wire_tilt(q, up, 0.1);
    let split = smooth_wire_tilt(smooth_wire_tilt(q, up, 0.05), up, 0.05);
    assert!(whole.angle_between(split) < 0.001);
    for _ in 0..128 {
        q = smooth_wire_tilt(q, up, 1. / 64.);
    }
    assert!((q * Vec3::Y).distance(up) < 0.001);
    assert_eq!(smooth_wire_tilt(q, Vec3::ZERO, 0.1), q);
    let Some(spots) = original_spots("arve.v060_attackheli.ee") else {
        return;
    };
    let (mut app, vehicle, entity) = moving_app(spots);
    let (p, r) = car_pose(&app, vehicle);
    app.world_mut()
        .run_system_once(
            move |mut commands: Commands, mut heroes: Query<&mut Hero>| {
                attach_wire(
                    &mut commands,
                    entity,
                    &mut heroes.get_mut(entity).unwrap(),
                    vehicle,
                    Vec3::ZERO,
                    6.,
                    p,
                    r,
                    Vec3::new(3., 0., 1.),
                );
            },
        )
        .unwrap();
    step(&mut app, 20);
    assert!(app.world().get::<WireTilt>(entity).unwrap().0.is_finite());
    app.world_mut().entity_mut(entity).remove::<OnStunt>(); // crew/fatal support removal route
    assert!(app.world().get::<WireSwing>(entity).is_none());
    assert!(app.world().get::<WireTilt>(entity).is_none());
}

#[test]
fn hang_wire_uses_final_launcher_frame_and_ragdoll_owns_model_pose() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_systems(Update, crate::character::hero_visuals)
        .add_systems(
            PostUpdate,
            (hang_wire_visual, crate::grapple::draw_cables)
                .chain()
                .after(bevy::transform::TransformSystems::Propagate),
        );
    let vehicle = app.world_mut().spawn(Transform::from_xyz(3., 15., 2.)).id();
    let mut hero = Hero::default();
    hero.stunt = Some(vehicle);
    let entity = app
        .world_mut()
        .spawn((
            hero,
            Transform::from_xyz(3., 9., 2.),
            OnStunt {
                vehicle,
                local: Vec3::ZERO,
                kind: StuntKind::Roof,
                wire: Some(6.),
            },
            WireTilt(Quat::from_rotation_z(0.3)),
        ))
        .id();
    let model = app
        .world_mut()
        .spawn((
            crate::character::HeroModel,
            Transform::default(),
            ChildOf(entity),
        ))
        .id();
    let launcher = app
        .world_mut()
        .spawn((
            crate::grapple::GrappleLauncher,
            Transform::from_xyz(-0.3, 1.6, -0.1),
            ChildOf(model),
        ))
        .id();
    app.update();
    app.update();
    for _ in 0..3 {
        let launcher_position = app
            .world()
            .get::<GlobalTransform>(launcher)
            .unwrap()
            .translation();
        let mut wires = app
            .world_mut()
            .query_filtered::<(&Cable, &Children), With<HangWire>>();
        let (c, children) = wires.single(app.world()).unwrap();
        assert!(c.a.distance(launcher_position) < 1e-6);
        let first = children.iter().next().unwrap();
        let global = *app.world().get::<GlobalTransform>(first).unwrap();
        let mesh = app.world().get::<Mesh3d>(first).unwrap().0.clone();
        let ring = app
            .world()
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()[..8]
            .to_vec();
        // the tube's first ring is centred on the launcher this frame
        let centre = ring
            .iter()
            .map(|p| global.transform_point(Vec3::from(*p)))
            .sum::<Vec3>()
            / 8.0;
        assert!(
            centre.distance(c.a) < 1e-4,
            "rendered tube trails launcher: {centre:?} != {:?}",
            c.a
        );
        app.world_mut()
            .get_mut::<Transform>(launcher)
            .unwrap()
            .translation
            .x += 0.2;
        app.update();
    }
    let pose = *app.world().get::<Transform>(model).unwrap();
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::ragdoll::PendingRagdoll {
            origin: Vec3::ZERO,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        });
    app.world_mut().get_mut::<WireTilt>(entity).unwrap().0 = Quat::from_rotation_x(0.8);
    app.update();
    assert_eq!(*app.world().get::<Transform>(model).unwrap(), pose);
    assert_eq!(count::<HangWire>(&mut app), 0);
    app.world_mut()
        .entity_mut(entity)
        .remove::<crate::ragdoll::PendingRagdoll>();
    app.world_mut()
        .entity_mut(entity)
        .insert(crate::ragdoll::PhysicsPose);
    app.update();
    assert_eq!(*app.world().get::<Transform>(model).unwrap(), pose);
    assert_eq!(count::<HangWire>(&mut app), 0);
}
