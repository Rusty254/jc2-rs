//! Rico model close-up capture (board 845e8e4c): `jc2 <game> --world 40,40 --time 12:00
//! --day-speed 0 --no-npcs --no-civilians --no-traffic --rico-capture DIR` stands Rico still,
//! puts the camera close in front of his face and chest from fixed angles, saves one PNG per
//! shot and quits. The same command on master and on the branch gives the before/after pairs.
//! Debug harness only; no original data and no gameplay effect.

use crate::character::HeroModel;
use crate::coords::facing_dir;
use crate::world::FlyCamera;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use std::path::PathBuf;

/// Real seconds before the first shot (start-up, streaming, settling).
const SETTLE_SECONDS: f32 = 30.0;
/// Real seconds between a camera move and its shot.
const HOLD_SECONDS: f32 = 1.5;

/// One shot: name, yaw offset from straight in front of him (degrees), distance, the height of
/// the look-at point above his feet and of the camera.
const SHOTS: [(&str, f32, f32, f32, f32); 6] = [
    ("face-front", -20.0, 0.75, 1.68, 1.68),
    ("face-side", 70.0, 0.75, 1.68, 1.68),
    ("chest", 0.0, 1.0, 1.3, 1.35),
    ("neck", 0.0, 0.6, 1.5, 1.55),
    ("bust", 35.0, 1.9, 1.35, 1.5),
    ("back", 180.0, 1.5, 1.7, 1.75),
];

pub struct RicoCapturePlugin(pub PathBuf);

#[derive(Resource)]
struct Capture {
    out: PathBuf,
    shot: usize,
    /// Real time at which the current shot's camera was placed.
    placed_at: Option<f32>,
    taken: bool,
    saved: bool,
}

impl Plugin for RicoCapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Capture {
            out: self.0.clone(),
            shot: 0,
            placed_at: None,
            taken: false,
            saved: false,
        })
        .add_systems(
            Update,
            drive.after(crate::character::hero_camera),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut cap: ResMut<Capture>,
    real: Res<Time<Real>>,
    models: Query<(Entity, &GlobalTransform, &Children), With<HeroModel>>,
    meshes: Query<(), With<Mesh3d>>,
    mut visibilities: Query<&mut Visibility>,
    mut cams: Query<&mut Transform, With<FlyCamera>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let now = real.elapsed_secs();
    if now < SETTLE_SECONDS {
        return;
    }
    if cap.shot >= SHOTS.len() {
        if cap.saved {
            exit.write(AppExit::Success);
        }
        return;
    }
    let (Ok((model_entity, model, children)), Ok(mut cam)) = (models.single(), cams.single_mut()) else {
        return;
    };
    // `JC2_RICO_HIDE=1,5`: hide those blocks (by their order in the model) to see which
    // block draws what
    if let Ok(list) = std::env::var("JC2_RICO_HIDE") {
        let _ = model_entity;
        let hide: Vec<usize> = list.split(',').filter_map(|n| n.trim().parse().ok()).collect();
        for (i, child) in children.iter().filter(|c| meshes.contains(*c)).enumerate() {
            if let Ok(mut v) = visibilities.get_mut(child) {
                *v = if hide.contains(&i) { Visibility::Hidden } else { Visibility::Inherited };
            }
        }
    }
    let (name, yaw_deg, distance, look_h, cam_h) = SHOTS[cap.shot];
    // the model's origin is at his feet, the model faces `facing_dir(0)` rotated by its own yaw
    let feet = model.translation();
    let ahead = Quat::from_rotation_y(yaw_deg.to_radians()) * (model.rotation() * facing_dir(0.0));
    let target = feet + Vec3::Y * look_h;
    cam.translation = feet + Vec3::Y * cam_h + ahead * distance;
    cam.look_at(target, Vec3::Y);
    let placed = *cap.placed_at.get_or_insert(now);
    if !cap.taken && now - placed >= HOLD_SECONDS {
        std::fs::create_dir_all(&cap.out).ok();
        cap.taken = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(cap.out.join(format!("{name}.png"))))
            .observe(|_: On<ScreenshotCaptured>, mut c: ResMut<Capture>| c.saved = true);
    } else if cap.taken && cap.saved {
        cap.shot += 1;
        cap.placed_at = None;
        cap.taken = false;
        if cap.shot < SHOTS.len() {
            cap.saved = false;
        }
    }
}
