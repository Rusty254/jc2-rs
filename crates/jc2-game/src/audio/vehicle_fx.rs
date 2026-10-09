//! Door events follow seat occupancy; the horn follows held input while driving.
use super::*;
use crate::vehicle::Enterable;

/// Effects presets have their own names, unrelated to the engine preset hashes.
/// The vehicle-to-preset assignment is inferred from body type (as engines are).
pub(super) fn effects_preset(name: &str) -> Option<(&'static str, &'static str)> {
    let (module, engine) = engine_preset(name);
    match module {
        "seve_engine" => None, // retail seve_effects has no presets
        "arve_engine" => Some((
            "arve_effects",
            if engine.starts_with("heli") || engine == "jet_heli" {
                "helicopter"
            } else {
                "jet_plane"
            },
        )),
        _ => Some((
            "lave_effects",
            match engine {
                "atv" | "tank" | "traktor" => return None, // no cabin door/horn assignment established
                "dirtbike" | "harley" | "kawasaki" | "new moped" => "motorcycle",
                "truck" | "monstertruck_dlc" => "truck",
                "bus" | "hummer" => "big_vehicle",
                "SmJeep" | "Corvette" | "ferrari" | "astonmartin" => "small_vehicle",
                _ => "medium_vehicle",
            },
        )),
    }
}

#[derive(Default)]
pub(super) struct SeatSounds {
    occupied: HashSet<Entity>,
}

/// Run after gameplay's deferred seat/hijack commands, including entering a second
/// car with the same model. Despawning/streaming a car is not a door interaction.
#[allow(clippy::type_complexity)]
pub(super) fn vehicle_effects(
    mode: Res<CameraMode>,
    keys: Res<ButtonInput<KeyCode>>,
    sounds: Res<Sounds>,
    mut playing: ResMut<Playing>,
    mut state: Local<SeatSounds>,
    vehicles: Query<
        (
            Entity,
            &Transform,
            Option<&Occupied>,
            Option<&Vehicle>,
            Option<&Heli>,
            Option<&Plane>,
        ),
        With<Enterable>,
    >,
) {
    let mut occupied = HashSet::new();
    let mut horn = None;
    for (entity, _, seat, car, heli, plane) in &vehicles {
        let name = car
            .map(|v| v.spec.name.as_str())
            .or_else(|| heli.map(|v| v.name.as_str()))
            .or_else(|| plane.map(|v| v.name.as_str()));
        let Some(name) = name else { continue };
        let seated = seat.is_some();
        if seated {
            occupied.insert(entity);
        }
        if seated != state.occupied.contains(&entity) {
            // Entry/exit are instantaneous today. Open now, close after the
            // original open sample (~0.6s), rather than overlaying both sounds.
            if let Some(open) = sounds.vehicle_effect(name, "door_open") {
                playing.fire(open, Place::On(entity), 1.0);
            }
            if let Some(close) = sounds.vehicle_effect(name, "door_close") {
                playing.fire_after(0.65, close, Place::On(entity), 1.0, &[]);
            }
        }
        if seated && car.is_some() && *mode == CameraMode::Drive && keys.pressed(KeyCode::KeyH) {
            horn = sounds.vehicle_effect(name, "horn");
        }
    }
    state.occupied = occupied;
    // Retail horns use velocity as a moving envelope parameter, not road speed:
    // .03 is within every sustained loop, before the acoustic release layer.
    playing.hold(
        "horn",
        horn.map(|key| (key, 1.0, &[("velocity", 0.03)][..])),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_follow_body_type_and_keep_boats_and_open_vehicles_silent() {
        assert_eq!(
            effects_preset("lave.v014_jeep.ee"),
            Some(("lave_effects", "big_vehicle"))
        );
        assert_eq!(
            effects_preset("lave.v047_jeep_small.ee"),
            Some(("lave_effects", "small_vehicle"))
        );
        assert_eq!(
            effects_preset("lave.v004_truck.ee"),
            Some(("lave_effects", "truck"))
        );
        assert_eq!(
            effects_preset("lave.v007_military.ee"),
            Some(("lave_effects", "motorcycle"))
        );
        assert_eq!(effects_preset("lave.v056_atv.ee"), None);
        assert_eq!(
            effects_preset("seve.v082_motorboat_standard_small.ee"),
            None
        );
        assert_eq!(
            effects_preset("arve.v009_military_helicopter.ee"),
            Some(("arve_effects", "helicopter"))
        );
    }

    fn car(name: &str) -> Vehicle {
        use crate::vehicle::VehicleSpec;
        Vehicle {
            spec: VehicleSpec {
                name: name.into(),
                mass: 1000.0,
                center_of_mass: Vec3::ZERO,
                wheels: vec![],
                max_steer: 0.0,
                steer_speed: 0.0,
                torque: 0.0,
                torque_rpm: 0.0,
                rpm_min: 0.0,
                rpm_max: 0.0,
                gears: vec![],
                reverse: 0.0,
                final_drive: 0.0,
                top_speed: 0.0,
                drag: 0.0,
            },
            wheels: vec![],
            throttle: 0.0,
            brake: 0.0,
            steer_input: 0.0,
            handbrake: false,
            gear: 0,
            rpm: 0.0,
            speed: 0.0,
            length: 4.0,
            collider_children: vec![],
        }
    }

    #[test]
    fn occupancy_emits_once_swaps_same_model_and_horn_stops_on_release_exit_and_free_camera() {
        let Some(fs) = super::super::tests::test_fs() else {
            return;
        };
        let sounds = Sounds::new(fs);
        let open = sounds
            .vehicle_effect("lave.v002_sedan.ee", "door_open")
            .unwrap();
        let close = sounds
            .vehicle_effect("lave.v002_sedan.ee", "door_close")
            .unwrap();
        let horn = sounds.vehicle_effect("lave.v002_sedan.ee", "horn").unwrap();
        let mut app = App::new();
        app.insert_resource(sounds)
            .init_resource::<Playing>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(CameraMode::Drive)
            .add_systems(PostUpdate, vehicle_effects);
        let a = app
            .world_mut()
            .spawn((
                car("lave.v002_sedan.ee"),
                Transform::default(),
                Enterable { reach: 2.0 },
                Occupied,
            ))
            .id();
        let b = app
            .world_mut()
            .spawn((
                car("lave.v002_sedan.ee"),
                Transform::default(),
                Enterable { reach: 2.0 },
            ))
            .id();
        app.update();
        let p = app.world().resource::<Playing>();
        assert!(
            matches!(&p.asks[0], Ask::Fire { key, at: Place::On(e), .. } if *key == open && *e == a)
        );
        assert!(
            matches!(&p.later[0], (delay, Ask::Fire { key, .. }) if *delay == 0.65 && *key == close)
        );
        app.world_mut().resource_mut::<Playing>().asks.clear();
        app.update();
        assert_eq!(app.world().resource::<Playing>().later.len(), 1);
        assert_eq!(app.world().resource::<Playing>().asks.len(), 1); // only the horn wish
        app.world_mut().resource_mut::<Playing>().asks.clear();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);
        app.update();
        assert!(
            matches!(app.world().resource::<Playing>().asks.last(), Some(Ask::Hold { slot: "horn", want: Some((key, _, _)) }) if *key == horn)
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyH);
        app.update();
        assert!(matches!(
            app.world().resource::<Playing>().asks.last(),
            Some(Ask::Hold {
                slot: "horn",
                want: None
            })
        ));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyH);
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Free;
        app.update();
        assert!(matches!(
            app.world().resource::<Playing>().asks.last(),
            Some(Ask::Hold { want: None, .. })
        ));
        *app.world_mut().resource_mut::<CameraMode>() = CameraMode::Drive;
        app.world_mut().entity_mut(a).remove::<Occupied>();
        app.world_mut().entity_mut(b).insert(Occupied);
        app.update();
        assert_eq!(app.world().resource::<Playing>().later.len(), 3); // a exit, b entry
        app.world_mut().entity_mut(b).remove::<Occupied>();
        app.update();
        assert!(matches!(
            app.world().resource::<Playing>().asks.last(),
            Some(Ask::Hold { want: None, .. })
        ));
        let n = app.world().resource::<Playing>().later.len();
        app.world_mut().despawn(b);
        app.update();
        assert_eq!(app.world().resource::<Playing>().later.len(), n);
    }
}
