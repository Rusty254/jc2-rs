//! Native colour units for the consumers that have no native data yet (JC2-139).
//!
//! The world camera runs the native display chain (`post_effects.rs`, JC2-134/137): the scene
//! target is clamped to 0..1, the composite multiplies by `A * tint` and writes `sqrt`
//! (gamma 2.0). **Native colour unit 1.0 is display white** (brief `night-exposure-next.md`
//! section 4). So a colour `c` in native units is what a surface, a sprite or the fog
//! "is" on the scene target, and the display shows `sqrt(c * A * tint)`. The identities below
//! carry every consumer to those units through the one camera exposure `e0`
//! (`sky_light::camera_exposure`), exactly as the sun and ambient already are:
//!
//! | consumer | Bevy quantity | native-unit identity | here |
//! |---|---|---|---|
//! | sun / moon (directional) | lux | `pi / e0` per unit | `sky_light::lux_per_unit` (native, JC2-134) |
//! | ambient | `AmbientLight::brightness` | `1 / e0` per unit | `sky_light::ambient_per_unit` (native) |
//! | point light | none: the native window term in the shaders (JC2-221) | native colour units added to the irradiance, times `e0` | `point_lights.rs` / `point_lights.wgsl` |
//! | emissive of a **lit** material | `emissive` (times `e0` unless its alpha is 0) | `1 / e0` per unit | [`emissive_per_unit`] |
//! | unlit colour, clear colour | the value is written to the scene target as is | 1 | [`native_colour`] |
//! | fog colour | native since JC2-155: rows 2/3 of the zone gradient, mixed into the scene colour as is | 1 | `haze.rs` (no policy fog colour or distances remain) |
//!
//! `unlit` materials show `base_color` only: Bevy ignores `emissive` on them (the lit branch of
//! `pbr_fragment.wgsl` / `pbr.wgsl`), and the exposure never touches them. Fog is mixed into
//! the already exposed colour, so a fog colour is a scene colour as well (the native haze colour
//! is used unconverted).
//!
//! **What is native here and what is policy.** The identities are native (they follow from the
//! chain and Bevy 0.19.1's formulas, tested below against a CPU copy of Bevy's maths). The
//! *values* fed through them are native only where a consumer cites research; everything built
//! with [`display_native`] / [`native_colour`] below is a **project colour authored as a display
//! value** (what the player should see with `A = 1` and no tint) and converted to the native
//! unit by `c = d^2`, the inverse of the chain's `sqrt`. That is policy, labelled at each use.

use bevy::prelude::*;

/// `StandardMaterial::emissive` per unit of native colour for a **lit** material (the shader
/// multiplies it by `e0` when the emissive alpha is 1, the default of `LinearRgba::rgb`).
#[allow(dead_code)] // no lit emissive consumer yet (every current one is unlit); the identity is tested
pub fn emissive_per_unit() -> f32 {
    1.0 / crate::sky_light::camera_exposure()
}

/// Native scene colour of a project colour authored as the display value `d` (0..1 per
/// channel): `d^2`. The chain shows `sqrt(c * A * tint)`, so with `A = 1` and a neutral tint
/// `display(display_native(d)) = d`. **Policy conversion** (the display value is the project's).
pub fn display_native(d: Vec3) -> Vec3 {
    let d = d.max(Vec3::ZERO);
    d * d
}

/// The display value a native scene colour shows with `A = 1` and no tint (clamped).
#[allow(dead_code)] // the inverse of `display_native`, used by the tests
pub fn native_display(c: Vec3) -> Vec3 {
    c.clamp(Vec3::ZERO, Vec3::ONE).map(f32::sqrt)
}

/// A bevy [`Color`] holding the native scene colour of the display value `d` (see
/// [`display_native`]); `alpha` is plain opacity.
pub fn native_colour(d: Vec3, alpha: f32) -> Color {
    let c = display_native(d);
    Color::linear_rgba(c.x, c.y, c.z, alpha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post_effects::{PostPass, encode_roundtrip};

    /// Bevy's directional diffuse for albedo 1, `N.L` 1: `lux / pi * e0`.
    fn bevy_sun_diffuse(lux: f32) -> f32 {
        crate::sky_light::camera_exposure() * lux / std::f32::consts::PI
    }

    #[test]
    fn the_sun_ambient_and_emissive_scales_agree_on_one_unit() {
        // a native strength through a directional light
        let sun = bevy_sun_diffuse(crate::sky_light::lux_per_unit());
        assert!((sun - 1.0).abs() < 1e-4);
        // and ambient: brightness * e0 = c
        let amb = crate::sky_light::ambient_per_unit() * crate::sky_light::camera_exposure();
        assert!((amb - 1.0).abs() < 1e-5);
        assert!((emissive_per_unit() * crate::sky_light::camera_exposure() - 1.0).abs() < 1e-5);
    }

    /// A policy colour authored as a display value shows as that value through the neutral
    /// chain (`PostPass::default()`, `A = 1`), including the surface's sRGB encode.
    #[test]
    fn a_display_authored_colour_round_trips_through_the_native_chain() {
        let pass = PostPass::default();
        for d in [
            Vec3::new(0.58, 0.72, 0.90),
            Vec3::new(0.010, 0.016, 0.040),
            Vec3::new(1.0, 0.85, 0.4),
            Vec3::ZERO,
            Vec3::ONE,
        ] {
            let scene = display_native(d);
            let shown = pass.display(scene, 1.0);
            assert!((shown - d).length() < 1e-5, "{d:?} -> {shown:?}");
            assert!((native_display(scene) - d).length() < 1e-5);
            // the surface holds the gamma-2.0 value (the pass's sRGB trick)
            for v in shown.to_array() {
                assert!((encode_roundtrip(v) - v).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn native_colours_are_linear_and_keep_alpha() {
        let c = native_colour(Vec3::new(0.5, 1.0, 0.0), 0.25).to_linear();
        assert!((c.red - 0.25).abs() < 1e-6 && (c.green - 1.0).abs() < 1e-6 && c.blue == 0.0);
        assert!((c.alpha - 0.25).abs() < 1e-6);
        // negative or NaN-free input stays finite and non-negative
        assert_eq!(display_native(Vec3::splat(-1.0)), Vec3::ZERO);
    }
}
