//! Vehicle enter capture (board 27da3660, JC2-259 follow-up): `jc2 <game> --world 40,40 --time 12:00
//! --day-speed 0 --no-npcs --no-civilians --no-traffic --heli --vehicle-capture DIR` (or `--boat`,
//! `--drive CONTAINER`, `--plane`) waits for the vehicle to be parked, saves `outside.png` from a
//! fixed camera with Rico standing outside, then teleports Rico next to it, presses `E` (the
//! normal `character::vehicle_entry` path) and saves `inside.png` from the SAME camera pose,
//! then quits. Next to the PNGs it writes `slots.txt`: for every vehicle part with a native LOD
//! plan the shown slot, distance to the camera and the camera zoom factor, outside and inside, so
//! a change of LOD on entering is visible without looking at the images. Debug harness only; no
//! original data.

use crate::character::Hero;
use crate::plant_lod::PlantLod;
use crate::vehicle::Enterable;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use std::fmt::Write as _;
use std::path::PathBuf;

/// Real seconds before the outside shot (start-up, streaming, settling).
const OUTSIDE_SECONDS: f32 = 35.0;
/// Real seconds after pressing `E` before the inside shot (the mode switch and the LOD swaps).
const INSIDE_DELAY: f32 = 5.0;
/// Fixed camera: behind and above the parked vehicle, looking at it (world axes).
const CAMERA_OFFSET: Vec3 = Vec3::new(0.0, 4.0, 14.0);

/// `OUTSIDE_SECONDS`, or the number of `JC2_VCAP_OUTSIDE` (a debugging aid for slow starts).
fn outside_seconds() -> f32 {
    std::env::var("JC2_VCAP_OUTSIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(OUTSIDE_SECONDS)
}

pub struct VehicleCapturePlugin(pub PathBuf);

#[derive(Resource)]
struct Capture {
    out: PathBuf,
    stage: u8,
    saved: bool,
    pose: Option<Transform>,
    t_enter: f32,
    report: String,
}

impl Plugin for VehicleCapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Capture {
            out: self.0.clone(),
            stage: 0,
            saved: false,
            pose: None,
            t_enter: 0.0,
            report: String::new(),
        })
        .add_systems(
            PreUpdate,
            press_enter.after(bevy::input::InputSystems),
        )
        .add_systems(Update, shoot)
        .add_systems(
            PostUpdate,
            fix_camera.before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// `E` for one frame when stage 2 asks for it.
fn press_enter(mut cap: ResMut<Capture>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    if cap.stage == 2 {
        keys.press(KeyCode::KeyE);
        cap.stage = 3;
    }
}

fn fix_camera(cap: Res<Capture>, mut cams: Query<&mut Transform, With<Camera3d>>) {
    if let Some(pose) = cap.pose {
        for mut tf in &mut cams {
            *tf = pose;
        }
    }
}

fn slot_report(
    label: &str,
    out: &mut String,
    cam: &GlobalTransform,
    zoom: f32,
    parts: &Query<(Entity, &PlantLod, &GlobalTransform, Option<&Name>)>,
    vehicle_parts: &[Entity],
) {
    let _ = writeln!(out, "[{label}] camera zoom factor F = {zoom:.3}");
    for (e, lod, tf, name) in parts.iter() {
        if !vehicle_parts.contains(&e) {
            continue;
        }
        let d = cam.translation().distance(tf.translation());
        let level = lod.vehicle.map_or(f32::NAN, |v| {
            jc2_formats::veglod::vehicle_level(jc2_formats::veglod::vehicle_scaled_d2(
                v.body_scale,
                d * d,
                zoom,
                v.unit,
            ))
        });
        let _ = writeln!(
            out,
            "  {:<34} model {}/{} multiplier {:.3} vehicle level {:.2} dist {:.1} m",
            name.map_or(format!("{e:?}"), |n| n.to_string()),
            lod.set.plan.models.get(lod.shown).map_or("?", |m| m.as_str()),
            lod.set.plan.models.len(),
            lod.set.plan.multiplier,
            level,
            d
        );
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn shoot(
    mut cap: ResMut<Capture>,
    real: Res<Time<Real>>,
    mut virt: ResMut<Time<Virtual>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    vehicles: Query<(Entity, &GlobalTransform, &Enterable)>,
    mut heroes: Query<
        (&mut Transform, &mut avian3d::prelude::LinearVelocity),
        (With<Hero>, Without<Enterable>),
    >,
    cams: Query<&GlobalTransform, With<Camera3d>>,
    parts: Query<(Entity, &PlantLod, &GlobalTransform, Option<&Name>)>,
    parents: Query<&ChildOf>,
    camera_fov: Option<Res<crate::gameplay_camera::CameraFov>>,
    projections: Query<&Projection, With<Camera3d>>,
) {
    let now = real.elapsed_secs();
    let take = |commands: &mut Commands, path: PathBuf, virt: &mut Time<Virtual>| {
        virt.pause();
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path))
            .observe(|_: On<ScreenshotCaptured>, mut c: ResMut<Capture>| c.saved = true);
    };
    // the parts of the first parked vehicle (parts are descendants of the vehicle entity)
    let Some((vehicle, vtf, enterable)) = vehicles.iter().next() else {
        return;
    };
    let vehicle_parts: Vec<Entity> = parts
        .iter()
        .map(|p| p.0)
        .filter(|&e| {
            let mut cur = e;
            while let Ok(p) = parents.get(cur) {
                cur = p.parent();
                if cur == vehicle {
                    return true;
                }
            }
            false
        })
        .collect();
    let zoom = crate::gameplay_camera::realised_fov(camera_fov.as_deref(), projections.iter().next())
        .map_or(1.0, jc2_formats::veglod::camera_zoom);
    match cap.stage {
        0 if now >= outside_seconds() => {
            std::fs::create_dir_all(&cap.out).ok();
            let target = vtf.translation();
            cap.pose =
                Some(Transform::from_translation(target + CAMERA_OFFSET).looking_at(target + Vec3::Y, Vec3::Y));
            cap.stage = 10;
        }
        // one frame later the fixed pose is in the camera's global transform
        10 => {
            if let Ok(cam) = cams.single() {
                let mut report = std::mem::take(&mut cap.report);
                slot_report("outside", &mut report, cam, zoom, &parts, &vehicle_parts);
                cap.report = report;
            }
            take(&mut commands, cap.out.join("outside.png"), &mut virt);
            cap.stage = 1;
        }
        1 if cap.saved => {
            cap.saved = false;
            virt.unpause();
            // Rico beside the vehicle (within `ENTER_RANGE` of its reach), then `E`
            if let Ok((mut tf, mut lin)) = heroes.single_mut() {
                tf.translation = vtf.translation() + Vec3::X * (enterable.reach + 1.0) + Vec3::Y;
                lin.0 = Vec3::ZERO;
            }
            cap.stage = 2;
            cap.t_enter = now;
        }
        3 if now - cap.t_enter >= INSIDE_DELAY => {
            if let Ok(cam) = cams.single() {
                let mut report = std::mem::take(&mut cap.report);
                slot_report("inside", &mut report, cam, zoom, &parts, &vehicle_parts);
                cap.report = report;
            }
            take(&mut commands, cap.out.join("inside.png"), &mut virt);
            cap.stage = 4;
        }
        4 if cap.saved => {
            std::fs::write(cap.out.join("slots.txt"), &cap.report).ok();
            info!("vehicle capture:\n{}", cap.report);
            exit.write(AppExit::Success);
            cap.stage = 5;
        }
        _ => {}
    }
}
