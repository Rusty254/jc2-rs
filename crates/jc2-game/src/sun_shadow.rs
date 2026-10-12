//! The Sun's cascade shadow set-up, from the native shadow manager (board 78ba941f).
//!
//! Native evidence (`JustCause2.exe`, all addresses are in the 32-bit image):
//!
//! * `0x007c6ba0` (per frame, calls the matrix builder `0x0096c020` last) fills three cascades
//!   from the camera position `c` (clamped to `max(ground, 200) + 1200` in height), the
//!   horizontal camera forward `f` (y = 0.0001, normalised), the player character's "height over
//!   ground" `v` (`0x00557050(1)`, the scalar `CHeightOverGroundCondition` reads) and the camera
//!   height over the ground `h` (clamped):
//!   * radius (half extent of the square orthographic box) `s0 = 10 + 20 (clamp(v, 5, 100) - 5) / 95`,
//!     `s1 = 50 (1 + (clamp(v, 7, 150) - 7) / 143)`, `s2 = 110 + 490 (clamp(h, 10, 300) - 10) / 290`
//!     (stored by `0x00974340` at shadow manager `+0x1bc`);
//!   * centre `c0 = c + 0.6 s0 f`, `c1 = c + 0.55 s1 f`, `c2 = c1 + 0.5 s2 f` (`0x00923160`, `+0x204`);
//!   * depth range (`0x00950eb0`, `+0x228`) 1250, 1250 and `1250 (1 + 3 (clamp(h, 200, 1000) - 200) / 800)`;
//!   * depth bias (`0x00952b00`, `+0x1f8`) `s0 * 0.0045 * 0.0008`, `s1 * 0.0045 * 0.0008`,
//!     `s2 * 0.0045 / range2`: with the box depth of about `range` metres that is `0.0045 s` metres;
//!   * the floor `+0x1f4 = 0.1` (`0x00e290bc`), and `+0x1f0 = 1.0` (the Sun-height clamp, inactive
//!     because `0x0105f7a6` is 1 in the image).
//! * `0x0096c020` builds, per cascade, a texel-snapped look-at from `centre - sun * range * 4/7`
//!   along the Sun, orthographic `2 s x 2 s`, near 10, far `range`; the depth bias goes in the
//!   atlas transform (`z - bias`), the three tiles are stacked in one atlas (`0x009315f0`).
//! * Resolution: the tile edge is `0x400`, or `0x800` when the shadow-resolution setting
//!   (`/shadowres`, default 0, settings index 0x2a) is non-zero (`0x00960000` range, the
//!   `ShadowManager` creation at `0x0096ffea`, and the settings apply in `text_00610000.c`);
//!   depth format code `0x37`. `/shadows` (default 1) enables the shadows and quality 2 selects
//!   the soft mode (12-tap PCF), so the default is the **hard** mode (4-tap PCF).
//!
//! Bevy's cascades are slices of the camera frustum, each fitted tightly, not the native nested
//! light-space boxes, so the box geometry cannot be reproduced; what is carried over is the
//! tile size (1024, one square map per cascade), the number of cascades (3), the distance each
//! cascade reaches along the view (centre distance plus the 78 % where the native edge fade
//! starts: pixel shader edge distance 3.9 of 5), the overlap (the fade zone, 22 %) and the
//! depth bias of cascade 0 (Bevy has one bias for the light). The per-cascade bias, the
//! 4-tap PCF kernel and the range are **not** reproduced (documented as gaps in
//! `research/formats/shadow-cascades-next.md`).

use bevy::light::{CascadeShadowConfig, DirectionalLightShadowMap};
use bevy::prelude::*;

use crate::character::Hero;
use crate::sky::Sun;
use crate::sky_light::ground_height;
use crate::world::Streamer;

/// Native tile edge in texels (`0x400`; `0x800` with the high shadow-resolution option).
pub const TILE_SIZE: usize = 1024;
/// Native depth bias in metres per metre of cascade radius (`0.0045`, see the module comment).
pub const BIAS_PER_RADIUS: f32 = 0.0045;
/// Native fade zone: the pixel shader fades a cascade over edge distance 3.9..5 of 5.
pub const EDGE_FADE_START: f32 = 0.78;

/// The three cascades of one frame (native values, metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeCascades {
    /// Half extent of each square box (`+0x1bc`).
    pub radius: [f32; 3],
    /// Distance of each box centre from the camera along the horizontal forward (`0.6 s0`,
    /// `0.55 s1`, `0.55 s1 + 0.5 s2`).
    pub centre_distance: [f32; 3],
    /// Depth range of each box (`+0x228`).
    pub range: [f32; 3],
}

/// `0x007c6ba0`'s dynamic branch. `hero_height` is the player's height over the ground
/// (`0x00557050(1)`, inferred as that measure; 0 on the ground), `camera_height` the camera's
/// height over the terrain under it (before the native cap, applied here).
pub fn native_cascades(hero_height: f32, camera_height: f32) -> NativeCascades {
    let v = hero_height;
    let h = camera_height;
    let s0 = (v.clamp(5.0, 100.0) - 5.0) * 0.010_526_316 * 20.0 + 10.0;
    let s1 = ((v.clamp(7.0, 150.0) - 7.0) * 0.006_993_007 + 1.0) * 50.0;
    let s2 = (h.clamp(10.0, 300.0) - 10.0) * 0.003_448_275_8 * 490.0 + 110.0;
    let r2 = ((h.clamp(200.0, 1000.0) - 200.0) * 0.001_25 * 3.0 + 1.0) * 1250.0;
    let d1 = 0.55 * s1;
    NativeCascades {
        radius: [s0, s1, s2],
        centre_distance: [0.6 * s0, d1, d1 + 0.5 * s2],
        range: [1250.0, 1250.0, r2],
    }
}

impl NativeCascades {
    /// Far bound of each cascade along the view for [`CascadeShadowConfig`]: where the native
    /// cascade's edge fade starts (centre distance plus 78 % of the radius), kept increasing.
    pub fn view_bounds(&self) -> [f32; 3] {
        let mut out = [0.0; 3];
        let mut prev = 0.0;
        for (i, o) in out.iter_mut().enumerate() {
            let b = self.centre_distance[i] + EDGE_FADE_START * self.radius[i];
            *o = b.max(prev + 1.0);
            prev = *o;
        }
        out
    }

    /// The native depth bias of cascade 0 in metres.
    pub fn depth_bias0(&self) -> f32 {
        BIAS_PER_RADIUS * self.radius[0]
    }
}

/// The camera cap of `0x007c6ba0`: `min(y, max(ground, 200) + 1200)` over the ground.
pub fn camera_height_over_ground(camera_y: f32, ground: f32) -> f32 {
    camera_y.min(ground.max(200.0) + 1200.0) - ground
}

/// `JC2_NO_NATIVE_CASCADES=1`: Bevy's own cascade split (4 cascades, 25 m, 800 m, 2048 texels).
pub fn disabled_by_env() -> bool {
    std::env::var_os("JC2_NO_NATIVE_CASCADES").is_some_and(|v| v != "0")
}

/// The native cascade set-up on the Sun, every frame.
pub fn update_cascades(
    streamer: Option<Res<Streamer>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    hero: Query<(&GlobalTransform, &Hero)>,
    mut sun: Query<(&mut CascadeShadowConfig, &mut DirectionalLight), With<Sun>>,
) {
    let Some(cam) = camera.iter().next() else {
        return;
    };
    let ground_at = |p: Vec3| {
        streamer.as_ref().map_or(0.0, |st| {
            ground_height(p.x, p.z, |k| st.loaded.get(&k).map(|t| t.heights.as_slice()))
        })
    };
    let p = cam.translation();
    let camera_height = camera_height_over_ground(p.y, ground_at(p));
    let hero_height = hero.iter().next().map_or(0.0, |(t, h)| {
        if h.grounded {
            0.0
        } else {
            let q = t.translation();
            (q.y - ground_at(q)).max(0.0)
        }
    });
    let c = native_cascades(hero_height, camera_height);
    let bounds = c.view_bounds();
    for (mut cfg, mut light) in &mut sun {
        if cfg.bounds.len() != 3 || cfg.bounds.iter().zip(bounds).any(|(a, b)| (a - b).abs() > 0.05) {
            cfg.bounds = bounds.to_vec();
        }
        cfg.overlap_proportion = 1.0 - EDGE_FADE_START;
        cfg.minimum_distance = 0.1;
        light.shadow_depth_bias = c.depth_bias0();
    }
}

/// Insert the native tile size. Called before the app runs.
pub fn insert_shadow_map(app: &mut App) {
    app.insert_resource(DirectionalLightShadowMap { size: TILE_SIZE });
}

/// `JC2_FRAME_STATS=1`: after a 10 s warm-up print the mean and 95th percentile frame time of
/// every 5 s window to stderr (`frame_stats mean=.. p95=.. n=..`). Measurement aid only.
#[derive(Resource, Default)]
pub struct FrameStats {
    elapsed: f32,
    window: Vec<f32>,
}

pub fn frame_stats(time: Res<Time<Real>>, mut stats: ResMut<FrameStats>) {
    let dt = time.delta_secs();
    stats.elapsed += dt;
    if stats.elapsed < 10.0 {
        return;
    }
    stats.window.push(dt * 1000.0);
    if stats.window.len() >= 300 {
        let mut w = std::mem::take(&mut stats.window);
        let mean = w.iter().sum::<f32>() / w.len() as f32;
        w.sort_by(|a, b| a.total_cmp(b));
        let p95 = w[w.len() * 95 / 100];
        eprintln!("frame_stats mean={mean:.2}ms p95={p95:.2}ms n={}", w.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_the_ground_the_cascades_are_the_minimum() {
        let c = native_cascades(0.0, 5.0);
        assert_eq!(c.radius, [10.0, 50.0, 110.0]);
        assert_eq!(c.range, [1250.0, 1250.0, 1250.0]);
        let d = c.centre_distance;
        assert!((d[0] - 6.0).abs() < 1e-4 && (d[1] - 27.5).abs() < 1e-4 && (d[2] - 82.5).abs() < 1e-4);
        let b = c.view_bounds();
        assert!(b[0] < b[1] && b[1] < b[2]);
        assert!((b[0] - 13.8).abs() < 1e-3);
        assert!((c.depth_bias0() - 0.045).abs() < 1e-6);
    }

    #[test]
    fn high_up_the_cascades_grow_to_their_maxima() {
        let c = native_cascades(200.0, 5000.0);
        assert!((c.radius[0] - 30.0).abs() < 1e-3);
        assert!((c.radius[1] - 100.0).abs() < 1e-3);
        assert!((c.radius[2] - 600.0).abs() < 1e-2);
        assert!((c.range[2] - 5000.0).abs() < 1e-1);
    }

    #[test]
    fn the_camera_height_is_capped_at_1200_over_the_higher_of_200_and_ground() {
        assert_eq!(camera_height_over_ground(100.0, 0.0), 100.0);
        assert_eq!(camera_height_over_ground(9000.0, 0.0), 1400.0);
        assert_eq!(camera_height_over_ground(9000.0, 500.0), 1200.0);
    }
}
