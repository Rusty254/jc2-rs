//! Pure ocean inputs (JC2-164; research `research/formats/water-inputs-next.md`, JC2-102).
//!
//! Everything here is a CPU function of camera, sun and depth values, with the brief's probe
//! fixtures as tests (`research/tools/water_inputs_probe.py`):
//!
//! | function | native | source |
//! |---|---|---|
//! | [`sun_specular`] | PS `cb1[0]` | `FUN_00a80300`, section 2 |
//! | [`ocean_shader_variant`] | VS/PS pair per patch | `FUN_00a809c0`, section 3 |
//! | [`clipmap_level_at`] | which clip-map level draws a point | `FUN_00a67610`, section 3 |
//! | [`shore_alpha`] | LOD0 PS `o1.w` (the `SRC1_ALPHA` factor) | section 4 |
//! | [`water_absorption`] | `1 - 2^(-thickness * cb0[14].w)` | `water-consumers-next.md` section 5 |
//!
//! The water colour and density by camera height are `AtmosphereState::water_colour` /
//! `water_density` (`atmosphere.rs`).

use bevy::prelude::*;

/// Exponent of the sun highlight (`cb1[0].w`, written by `FUN_00a80300`).
pub const SPECULAR_EXPONENT: f32 = 256.0;

/// The static white `(Rs, Gs, Bs) = DAT_0108c5e4/e8/ec` (initialised data, never stored to).
const SPECULAR_BASE: Vec3 = Vec3::ONE;

/// The sun-specular constant `cb1[0] = (x, y, z, 256)` of the ocean pixel shaders.
///
/// `sun_y` is the y component of the unit vector towards the sun, **unflipped** (the moon flip
/// is not applied: at night `sun_y <= 0` selects the weak `m = 1` branch), `camera_y` the camera
/// height and `wet` the precipitation weight `P.+0x48` (`AtmosphereState::precip`, 0 in clear
/// weather).
pub fn sun_specular(sun_y: f32, camera_y: f32, wet: f32) -> Vec4 {
    let a = (sun_y * 32.0).abs().clamp(0.0, 1.0);
    let h = (1.0 - (camera_y - 200.0) * 0.003).clamp(0.6, 1.0);
    let c = 1.0 - wet;
    let f7 = (0.8 - sun_y.abs() * 1.5).max(0.0);
    let f8 = (1.0 - f7 * 2.0).max(0.0);
    let m = if sun_y > 0.0 { 8.0 } else { 1.0 };
    let k = 4.0 * a * h * 0.15 * c * m;
    Vec4::new(
        SPECULAR_BASE.x * k,
        (1.0 - f7) * SPECULAR_BASE.y * k,
        f8 * SPECULAR_BASE.z * k,
        SPECULAR_EXPONENT,
    )
}

/// A vertex-shader variant of the ocean patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OceanVs {
    /// `watershader_lod0` (339512): displaces the 5 x 5 vertices.
    Lod0,
    /// `watershader_lod1` (342468): flat.
    Lod1,
    /// `WaterBelow` (camera under the sea).
    Below,
}

/// A pixel-shader variant of the ocean patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OceanPs {
    /// 4 normal layers + foam (`watershader_lod0`).
    Lod0,
    /// The same with the stereo build's pass (`lod0_stereo`).
    Lod0Stereo,
    /// 4 normal samples, no foam.
    Lod1,
    /// 1 normal sample.
    Lod2,
    /// `WaterBelow`.
    Below,
}

/// `FUN_00a809c0`: the shader pair of an ocean patch of side `side` metres. Camera below 200.0
/// is `WaterBelow` for both; otherwise the vertex shader is lod1 only for `side > 128` and the
/// pixel shader is lod2 for `side > 128`, lod1 for `side > 32` or a camera 128 m or more above the
/// sea, else lod0 (`stereo` selects `lod0_stereo`, `FUN_0091bd00`).
pub fn ocean_shader_variant(side: f32, camera_y: f32, stereo: bool) -> (OceanVs, OceanPs) {
    if camera_y < 200.0 {
        return (OceanVs::Below, OceanPs::Below);
    }
    let vs = if side > 128.0 {
        OceanVs::Lod1
    } else {
        OceanVs::Lod0
    };
    let h = (camera_y - 200.0).max(0.0);
    let ps = if side > 32.0 || h >= 128.0 {
        if side > 128.0 {
            OceanPs::Lod2
        } else {
            OceanPs::Lod1
        }
    } else if stereo {
        OceanPs::Lod0Stereo
    } else {
        OceanPs::Lod0
    };
    (vs, ps)
}

/// `DAT_0108ab34` / `DAT_0108b948`: the clip-map grid origin on both axes.
pub const CLIPMAP_ORIGIN: f32 = -16384.0;
/// Number of clip-map levels (nibble 4 .. 14).
pub const CLIPMAP_LEVELS: usize = 11;
/// Half-width of a level's window in cells (7 x 7 patches `[X-3, X+3]`).
pub const CLIPMAP_HALF: i64 = 3;

/// The patch side (metres) of clip-map level index `i` (0 = the finest, 16 m ... 10 = 16384 m).
pub fn clipmap_side(i: usize) -> f32 {
    16.0 * (1u32 << i) as f32
}

/// The grid cell of `coord` at patch side `side`: `floor((coord - origin) / side)`.
fn cell(coord: f32, side: f32) -> i64 {
    ((coord - CLIPMAP_ORIGIN) / side).floor() as i64
}

/// The window `[X-3, X+3]` of cells a level activates around the camera coordinate `cam`
/// (`FUN_00a67610`).
pub fn clipmap_window(cam: f32, side: f32) -> (i64, i64) {
    let x = cell(cam, side);
    (x - CLIPMAP_HALF, x + CLIPMAP_HALF)
}

/// The finest clip-map level whose 7 x 7 window holds the cell of `point` (x, z) for a camera at
/// `camera` (x, z): `Some(level index)` (0 = 16 m), or `None` beyond the coarsest window.
/// A coarse patch is drawn only where finer active patches do not cover it, so the finest
/// containing window is the level actually drawn (the quadrant-cut logic itself is **inferred**,
/// water-inputs-next.md section 7). The region is square, not circular.
pub fn clipmap_level_at(camera: Vec2, point: Vec2) -> Option<usize> {
    (0..CLIPMAP_LEVELS).find(|&i| {
        let side = clipmap_side(i);
        let (x0, x1) = clipmap_window(camera.x, side);
        let (z0, z1) = clipmap_window(camera.y, side);
        let (cx, cz) = (cell(point.x, side), cell(point.y, side));
        (x0..=x1).contains(&cx) && (z0..=z1).contains(&cz)
    })
}

/// The LOD0 pixel shader's `o1.w` (the dual-source `SRC1_ALPHA` blend factor):
/// `sat(sat(4 (t_u - 0.5)) + 0.0005 * surface_eye)` with `t_u = eye_u - surface_eye` the
/// undistorted water thickness along the view depth. Transparent up to 0.5 m of water, opaque
/// from 0.75 m, plus a small distance term.
#[allow(dead_code)] // CPU mirror of sea.wgsl (tests, inspection)
pub fn shore_alpha(eye_u: f32, surface_eye: f32) -> f32 {
    let t_u = eye_u - surface_eye;
    ((4.0 * (t_u - 0.5)).clamp(0.0, 1.0) + 0.0005 * surface_eye).clamp(0.0, 1.0)
}

/// The deep-water mix `1 - 2^(-thickness * density)`, never negative (`max(0, ..)`; SM4 `exp`
/// is base 2). `thickness` is the (undistorted here) eye depth of the seabed minus that of the
/// surface; `density` is `AtmosphereState::water_density`.
#[allow(dead_code)] // CPU mirror of sea.wgsl (tests, inspection)
pub fn water_absorption(thickness: f32, density: f32) -> f32 {
    (1.0 - (-thickness * density).exp2()).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec4, b: [f32; 4]) -> bool {
        (a - Vec4::from(b)).abs().max_element() < 1e-4
    }

    #[test]
    fn sun_specular_matches_the_probe_fixtures() {
        // camera at y = 200, wet 0
        assert!(close(sun_specular(0.9, 200.0, 0.0), [4.8, 4.8, 4.8, 256.0]));
        assert!(close(
            sun_specular(0.3, 200.0, 0.0),
            [4.8, 3.12, 1.44, 256.0]
        ));
        assert!(close(
            sun_specular(0.1, 200.0, 0.0),
            [4.8, 1.68, 0.0, 256.0]
        ));
        assert!(close(
            sun_specular(0.02, 200.0, 0.0),
            [3.072, 0.70656, 0.0, 256.0]
        ));
        assert!(close(sun_specular(0.0, 200.0, 0.0), [0.0, 0.0, 0.0, 256.0]));
        assert!(close(
            sun_specular(-0.5, 200.0, 0.0),
            [0.6, 0.57, 0.54, 256.0]
        ));
        // h floors at 0.6 from 333 m up; wet 0.5 halves; the exponent is always 256
        assert!((sun_specular(0.9, 500.0, 0.0).x - 2.88).abs() < 1e-4);
        assert!((sun_specular(0.9, 200.0, 0.5).x - 2.4).abs() < 1e-4);
        assert_eq!(sun_specular(0.9, 200.0, 1.0).x, 0.0);
        assert_eq!(sun_specular(0.3, 900.0, 0.2).w, 256.0);
    }

    #[test]
    fn shader_variants_follow_the_side_and_height_rules() {
        use OceanPs as P;
        use OceanVs as V;
        let v = |s, y| ocean_shader_variant(s, y, false);
        assert_eq!(v(16.0, 230.0), (V::Lod0, P::Lod0));
        assert_eq!(v(32.0, 200.0), (V::Lod0, P::Lod0));
        assert_eq!(v(64.0, 230.0), (V::Lod0, P::Lod1));
        assert_eq!(v(128.0, 230.0), (V::Lod0, P::Lod1));
        assert_eq!(v(256.0, 230.0), (V::Lod1, P::Lod2));
        assert_eq!(v(16384.0, 230.0), (V::Lod1, P::Lod2));
        // height rule: 128 m above the sea forces PS lod1 for small patches only
        assert_eq!(v(16.0, 328.0), (V::Lod0, P::Lod1));
        assert_eq!(v(16.0, 327.99), (V::Lod0, P::Lod0));
        assert_eq!(v(256.0, 328.0), (V::Lod1, P::Lod2));
        // below the sea
        assert_eq!(v(16.0, 199.99), (V::Below, P::Below));
        assert_eq!(v(16.0, 200.0), (V::Lod0, P::Lod0));
        // stereo only swaps lod0
        assert_eq!(
            ocean_shader_variant(16.0, 230.0, true),
            (V::Lod0, P::Lod0Stereo)
        );
        assert_eq!(ocean_shader_variant(64.0, 230.0, true), (V::Lod0, P::Lod1));
    }

    #[test]
    fn clipmap_windows_and_levels_match_the_probe_fixtures() {
        let cam = Vec2::ZERO;
        let at = |x: f32, z: f32| clipmap_level_at(cam, Vec2::new(x, z));
        // camera on a 16 m cell edge: 16 m patches reach -48 .. +64 m
        assert_eq!(clipmap_window(0.0, 16.0), (1021, 1027));
        assert_eq!(at(63.9, 0.0), Some(0));
        assert_eq!(at(64.0, 0.0), Some(1));
        assert_eq!(at(-48.0, 0.0), Some(0));
        assert_eq!(at(-48.01, 0.0), Some(1));
        // 32 m: -96 .. +128; 64 m: to +256 / -192; 128 m: +512 / -384
        assert_eq!(at(127.9, 0.0), Some(1));
        assert_eq!(at(128.0, 0.0), Some(2));
        assert_eq!(at(-96.0, 0.0), Some(1));
        assert_eq!(at(-96.01, 0.0), Some(2));
        assert_eq!(at(511.9, 0.0), Some(3));
        assert_eq!(at(512.0, 0.0), Some(4));
        assert_eq!(at(-384.0, 0.0), Some(3));
        assert_eq!(at(-384.01, 0.0), Some(4));
        // the window is a square: the worst axis decides
        assert_eq!(at(60.0, 60.0), Some(0));
        assert_eq!(at(60.0, 64.0), Some(1));
        // the 8192 m level (index 9) window already spans the 32 km grid from the centre
        // (4 cells), so the 16384 m level is never the finest containing one there
        assert_eq!(clipmap_side(10), 16384.0);
        assert_eq!(at(-16000.0, 16000.0), Some(9));
        assert_eq!(
            at(20000.0, 0.0),
            Some(9),
            "past the grid edge: window only, no grid bound"
        );
        // the LOD0 pixel-shader levels (16/32 m) never reach past 128 m
        for x in [-200.0_f32, 130.0, 300.0] {
            assert!(at(x, 0.0).unwrap() >= 2, "{x}");
        }
    }

    #[test]
    fn shore_alpha_is_transparent_to_half_a_metre_and_opaque_from_three_quarters() {
        assert!((shore_alpha(100.2, 100.0) - 0.05).abs() < 1e-6);
        assert!((shore_alpha(100.5, 100.0) - 0.05).abs() < 1e-6);
        assert_eq!(shore_alpha(100.75, 100.0), 1.0);
        assert!((shore_alpha(100.625, 100.0) - 0.55).abs() < 1e-6);
        // negative thickness (seabed in front of the surface) is just the distance term
        assert!((shore_alpha(90.0, 100.0) - 0.05).abs() < 1e-6);
    }

    #[test]
    fn absorption_matches_the_optical_fixtures() {
        assert_eq!(water_absorption(0.0, 0.3), 0.0);
        assert!((water_absorption(1.0 / 0.3, 0.3) - 0.5).abs() < 1e-6);
        assert!((water_absorption(3.0, 0.3) - 0.464_113_3).abs() < 1e-6);
        assert!((water_absorption(10.0, 0.3) - 0.875).abs() < 1e-6);
        assert!((water_absorption(10.0, 0.15) - 0.646_446_6).abs() < 1e-6);
        assert_eq!(water_absorption(-5.0, 0.3), 0.0);
    }
}
