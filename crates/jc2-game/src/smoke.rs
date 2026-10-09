//! Flags for automated smoke tests: leave after a while, and leave a picture behind.
//!
//! `--exit-after <seconds>` ends the app cleanly (`AppExit::Success`) once that much
//! real time has passed since it started. `--screenshot <path.png>` saves the primary
//! window to that file first. The shot is requested at the exit time and the app then
//! waits (a few frames, at most [`CAPTURE_TIMEOUT`]) until the capture has been written
//! before it quits, so what is saved is a frame that has actually rendered. A screenshot
//! without `--exit-after` is taken after [`DEFAULT_SHOT_AFTER`] seconds.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use std::path::PathBuf;

/// When a `--screenshot` without `--exit-after` is taken.
pub const DEFAULT_SHOT_AFTER: f32 = 20.0;
/// How long to wait for the capture to land before quitting anyway.
const CAPTURE_TIMEOUT: f32 = 15.0;

pub struct SmokePlugin {
    pub exit_after: Option<f32>,
    pub screenshot: Option<PathBuf>,
}

#[derive(Resource)]
struct Smoke {
    at: f32,
    screenshot: Option<PathBuf>,
    /// The real time the shot was requested at.
    requested: Option<f32>,
    saved: bool,
    /// Frame times (ms) since the warm-up, for the summary line.
    frames: Vec<f32>,
}

/// Frames before this many seconds are start-up (loading, shader compiles) and are not
/// counted in the frame-time summary.
const WARM_UP: f32 = 12.0;

impl Plugin for SmokePlugin {
    fn build(&self, app: &mut App) {
        let at = self
            .exit_after
            .unwrap_or(if self.screenshot.is_some() {
                DEFAULT_SHOT_AFTER
            } else {
                f32::MAX
            })
            .max(0.0);
        app.insert_resource(Smoke {
            at,
            screenshot: self.screenshot.clone(),
            requested: None,
            saved: false,
            frames: Vec::new(),
        })
        .add_systems(Last, smoke);
    }
}

fn smoke(
    time: Res<Time<Real>>,
    mut state: ResMut<Smoke>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
    capture: crate::viewer::MaterialCapture,
) {
    let now = time.elapsed_secs();
    if now >= WARM_UP && state.requested.is_none() {
        state.frames.push(time.delta_secs() * 1000.0);
    }
    if now < state.at {
        return;
    }
    if state.requested.is_none() && !state.frames.is_empty() {
        let mut f = state.frames.clone();
        f.sort_by(f32::total_cmp);
        let avg = f.iter().sum::<f32>() / f.len() as f32;
        info!(
            "smoke: {} frames after warm-up, frame time avg {avg:.1} ms, median {:.1}, p95 {:.1}, worst {:.1}",
            f.len(),
            f[f.len() / 2],
            f[f.len() * 95 / 100],
            f[f.len() - 1]
        );
    }
    let Some(path) = state.screenshot.clone() else {
        info!("smoke: exiting after {now:.1} s");
        exit.write(AppExit::Success);
        return;
    };
    match state.requested {
        None => {
            if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
                let _ = std::fs::create_dir_all(dir);
            }
            info!("smoke: screenshot to {} at {now:.1} s", path.display());
            if let Some(receipt) = capture.snapshot() {
                let receipt_path = path.with_extension("camera.json");
                match std::fs::write(&receipt_path, receipt.to_string()) {
                    Ok(()) => info!(
                        "smoke: material capture state to {}",
                        receipt_path.display()
                    ),
                    Err(error) => error!("smoke: material capture receipt failed: {error}"),
                }
            }
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path))
                .observe(|_: On<ScreenshotCaptured>, mut state: ResMut<Smoke>| {
                    state.saved = true;
                });
            state.requested = Some(now);
        }
        Some(since) => {
            if state.saved {
                exit.write(AppExit::Success);
            } else if now - since > CAPTURE_TIMEOUT {
                error!("smoke: the screenshot never arrived, exiting without it");
                exit.write(AppExit::error());
            }
        }
    }
}
