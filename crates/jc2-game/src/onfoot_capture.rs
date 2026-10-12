//! On-foot camera capture (board c0c2758e): `jc2 <game> --world 40,40 --time 12:00 --day-speed 0
//! --no-npcs --no-civilians --no-traffic --onfoot-capture DIR` saves `standing.png` once Rico has
//! stood still for [`STAND_SECONDS`], then walks him forward (the orbit camera at its default
//! yaw, so straight ahead) and saves `running.png` [`RUN_SECONDS`] later, then quits. The same
//! command on master and on the branch gives the before/after pair. Debug harness only; no
//! original data.

use crate::character::HeroInput;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use std::path::PathBuf;

/// Real seconds before the standing shot (start-up, streaming, settling).
const STAND_SECONDS: f32 = 30.0;
/// Fixed-step seconds of running before the running shot.
const RUN_SECONDS: f32 = 5.0;

pub struct OnfootCapturePlugin(pub PathBuf);

#[derive(Resource)]
struct Capture {
    out: PathBuf,
    stage: u8,
    run_ticks: u32,
    saved: bool,
}

impl Plugin for OnfootCapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Capture {
            out: self.0.clone(),
            stage: 0,
            run_ticks: 0,
            saved: false,
        })
        .add_systems(FixedUpdate, drive.before(crate::character::hero_physics))
        .add_systems(Update, shoot);
    }
}

fn drive(
    mut cap: ResMut<Capture>,
    mut input: ResMut<HeroInput>,
    mut orbit: ResMut<crate::character::OrbitCamera>,
) {
    // the start faces a wall: turn the camera round (the open grass is behind) and run that way
    orbit.yaw = std::f32::consts::PI;
    if cap.stage == 2 {
        cap.run_ticks += 1;
        input.movement = Vec3::Z;
    }
}

fn shoot(
    mut cap: ResMut<Capture>,
    real: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    mut virt: ResMut<Time<Virtual>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let now = real.elapsed_secs();
    let take = |commands: &mut Commands, path: PathBuf, virt: &mut Time<Virtual>| {
        virt.pause();
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path))
            .observe(|_: On<ScreenshotCaptured>, mut c: ResMut<Capture>| c.saved = true);
    };
    match cap.stage {
        0 if now >= STAND_SECONDS => {
            std::fs::create_dir_all(&cap.out).ok();
            take(&mut commands, cap.out.join("standing.png"), &mut virt);
            cap.stage = 1;
        }
        1 if cap.saved => {
            cap.saved = false;
            virt.unpause();
            cap.stage = 2;
        }
        2 if cap.run_ticks as f32 * fixed.timestep().as_secs_f32() >= RUN_SECONDS => {
            take(&mut commands, cap.out.join("running.png"), &mut virt);
            cap.stage = 3;
        }
        3 if cap.saved => {
            exit.write(AppExit::Success);
            cap.stage = 4;
        }
        _ => {}
    }
}
