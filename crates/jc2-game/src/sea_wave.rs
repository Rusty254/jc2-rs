//! The CPU wave height of the sea: what `CPfxSea` slot 2 (`FUN_0059ff30`) answers to boats,
//! splashes and swimmers (JC2-189, water-sea-consumers.md), i.e. `FUN_00a8bba0 = 200 +
//! FUN_00a88ea0` (JC2-165/JC2-228, water-cpu-wave.md).
//!
//! * The gradient table (`FUN_00a80da0`) is rebuilt from the engine clock: sixteen directions
//!   from an MSVC LCG, each turned by `angle1 = (clock & 0x3fffff) / 350` about the body y axis.
//!   Only the first row of the final matrix is stored, and the x rotation by `angle2` never
//!   reaches it (a row of `RotX * N` is the first row of `N`), so `angle2` is not used here.
//! * The noise (`FUN_00a81190`) is a 16-lattice gradient noise with a smoothstep fade and a
//!   running-sum permutation (`z mod 16 + x mod 16 + 1`), evaluated at `7.34 * p`.
//! * A vertex is `0.6 * noise * clamp(1 - 0.015 d, 0, 1)` (`d` from the camera) and only exists
//!   for `|y - 200| <= 5` and `d <= 66.67 m` (`FUN_00a88d60`); the height at a point is the plane
//!   through the three vertices of its triangle in the cell grid of the terrain patch under it
//!   (side / 4, `FUN_00a88ea0`), the patch being the finest clip-map level that contains the point
//!   (`FUN_00a5e680`, [`crate::ocean_inputs::clipmap_level_at`]).
//!
//! The `y` the native getter tests is the **camera's** height (slot 2 builds its point from the
//! camera's y), so the wave only exists while the camera is within 5 m of the sea level.
//!
//! **Inferred / not native:** the clock origin and unit (the engine clock `FUN_004e7220(0)` is
//! taken as milliseconds since start; here: fixed-step time), the matrix call order of the
//! table builder (the `this` pointers of the matrix helpers are not in the decompilation), the
//! finest-level rule for the patch under a point, and the alternative path of `FUN_00a88d60`
//! (`DAT_013657fb && DAT_0108ab05 && FUN_0091d150()` returns `FUN_00a78eb0` instead; unread).

use crate::ocean_inputs::{CLIPMAP_ORIGIN, clipmap_level_at, clipmap_side};
use crate::post_effects::PostPass;
use crate::world::SEA_LEVEL;
use bevy::prelude::*;

/// `FUN_00a88d60` / `FUN_00a88ea0`: only within this squared distance of the camera
/// (`4444.4453`, i.e. 66.67 m).
const REACH_SQ: f32 = 4444.4453;
/// `FUN_00a88d60`: the vertex height scale, the distance fade slope and the noise frequency.
const AMPLITUDE: f32 = 0.6;
const FADE_PER_M: f32 = 0.015;
const FREQUENCY: f32 = 7.34;
/// `FUN_00a80da0`: the table clock mask and the angle scale (`1 / 350`).
const CLOCK_MASK: u32 = 0x3f_ffff;
const ANGLE1_PER_TICK: f32 = 0.002_857_142_8;
/// The table: 16 gradients, mirrored once, plus two copies of the first (34 entries).
const TABLE: usize = 34;

// the constants of the engine's rotation helpers (`DAT_01081d9c`, `DAT_01081da0`, `DAT_01081da4`)
const TWO_PI: f32 = f32::from_bits(0x40c9_0fdb);
const HALF_PI: f32 = f32::from_bits(0x3fc9_0fdb);
const INV_PI: f32 = f32::from_bits(0x3ea2_f983);

#[derive(Resource, Clone, Debug)]
pub struct SeaWaves {
    /// The x and y of each table entry (entries 16..32 repeat 0..16, 32 and 33 repeat 0 and 1).
    grad: [[f32; 2]; TABLE],
    /// The camera (engine axes) the wave is referenced to; `None` without one.
    pub camera: Option<Vec3>,
    /// The clock the table was built for, in engine ticks (milliseconds, inferred).
    pub clock: u32,
}

impl Default for SeaWaves {
    fn default() -> Self {
        Self::at_clock(0)
    }
}

/// The engine's sine / cosine of an angle (`FUN_004eaad0`): a parabola refined once with the
/// constant 0.225. Returns `(sin, cos)`.
fn sin_cos(angle: f32) -> (f32, f32) {
    let mut a = angle - ((INV_PI * angle * 0.5) as i32 as f32) * TWO_PI;
    if std::f32::consts::PI - a < 0.0 {
        a -= TWO_PI;
    }
    if a + std::f32::consts::PI < 0.0 {
        a += TWO_PI;
    }
    let sign = if a < 0.0 { -1.0 } else { 1.0 };
    let s = (1.0 - INV_PI * sign * a) * a * INV_PI * 4.0;
    let mut b = HALF_PI + a;
    let s = (s * s * sign - s) * 0.225 + s;
    if std::f32::consts::PI - b < 0.0 {
        b -= TWO_PI;
    }
    let sign = if b < 0.0 { -1.0 } else { 1.0 };
    let c = (1.0 - INV_PI * sign * b) * b * INV_PI * 4.0;
    let c = (c * c * sign - c) * 0.225 + c;
    (s, c)
}

fn floor_f(v: f32) -> f32 {
    v.floor()
}

impl SeaWaves {
    /// The table `FUN_00a80da0` builds for engine clock `clock`.
    pub fn at_clock(clock: u32) -> Self {
        let angle1 = (clock & CLOCK_MASK) as f32 * ANGLE1_PER_TICK;
        let (sin, cos) = sin_cos(angle1);
        let mut grad = [[0.0f32; 2]; TABLE];
        let mut state: i32 = 0;
        for k in 0..16 {
            let next = |s: i32| s.wrapping_mul(0x343fd).wrapping_add(0x26_9ec3);
            let (r1, r2, r3) = {
                let r1 = next(state);
                let r2 = next(r1);
                (r1, r2, next(r2))
            };
            state = r3;
            let comp = |r: i32| ((r >> 16) & 0x7fff) as f32 * 6.103_702e-5 - 1.0;
            let (mut x, mut y, mut z) = (comp(r3), comp(r2), comp(r1));
            let len2 = x * x + y * y + z * z;
            let scale = if len2 - 1e-6 < 0.0 { 1.0 } else { 1.0 / len2.sqrt() };
            (x, y, z) = (x * scale, y * scale, z * scale);
            // the first row of the "look along the axis" matrix with up = +y (`FUN_004e0f60`):
            // up x (eye - at), normalised; identity when the axis is near-parallel to up
            let row0 = if y.abs() > 0.9999 {
                [1.0f32, 0.0, 0.0]
            } else {
                let len2 = x * x + y * y + z * z;
                let s = if len2 - 1e-6 < 0.0 { 1.0 } else { 1.0 / len2.sqrt() };
                let (fx, _fy, fz) = (-x * s, -y * s, -z * s);
                let raw = [fz, 0.0, -fx];
                let l2 = raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2];
                let n = if l2 - 1e-6 < 0.0 { 1.0 } else { 1.0 / l2.sqrt() };
                [raw[0] * n, raw[1] * n, raw[2] * n]
            };
            // row0 * RotY(angle1): the component the noise reads are x and y
            let g = [row0[0] * cos + row0[2] * sin, row0[1]];
            grad[k] = g;
            grad[k + 16] = g;
        }
        grad[32] = grad[0];
        grad[33] = grad[1];
        Self {
            grad,
            camera: None,
            clock,
        }
    }

    /// `FUN_00a81190` at an (already scaled) point `(px, pz)`.
    fn noise(&self, px: f32, pz: f32) -> f32 {
        let (ax, az) = (px.abs(), pz.abs());
        let tz = az * 0.0625;
        let zmod = (tz - floor_f(tz)) * 16.0;
        let tx = ax * 0.0625;
        let i = ((tx - floor_f(tx)) * 16.0) as i32;
        let fx = ax - floor_f(ax);
        let fz = az - floor_f(az);
        // the table's fourth float is the running index 1..16 (mirrored entries repeat it)
        let w = |entry: i32| ((entry % 16) + 1) as f32;
        let ya = zmod + w(i);
        let yb = zmod + w(i + 1);
        let g = |idx: f32| self.grad[(idx as i32).clamp(0, TABLE as i32 - 1) as usize];
        let (ga, gb) = (g(ya), g(yb));
        let (ga1, gb1) = (g(ya + 1.0), g(yb + 1.0));
        let c00 = ga[0] * fx + ga[1] * fz;
        let c10 = fz * gb[1] + (fx - 1.0) * gb[0];
        let c01 = (fz - 1.0) * ga1[1] + fx * ga1[0];
        let c11 = (fz - 1.0) * gb1[1] + (fx - 1.0) * gb1[0];
        let v = fz * fz * (3.0 - fz * 2.0);
        let u = fx * fx * (3.0 - fx * 2.0);
        let n0 = (c01 - c00) * v + c00;
        let n1 = (c11 - c10) * v + c10;
        (n1 - n0) * u + n0
    }

    /// `FUN_00a88d60`: the vertex height offset at `(x, z)` for a point of height `gate_y`
    /// against the reference (camera) position `r`.
    fn vertex(&self, x: f32, z: f32, gate_y: f32, r: Vec3) -> f32 {
        if (gate_y - SEA_LEVEL).abs() > 5.0 {
            return 0.0;
        }
        let d2 = (r.z - z) * (r.z - z) + (r.x - x) * (r.x - x);
        if d2 > REACH_SQ {
            return 0.0;
        }
        let fade = (1.0 - d2.sqrt() * FADE_PER_M).clamp(0.0, 1.0);
        self.noise(x * FREQUENCY, z * FREQUENCY) * fade * AMPLITUDE
    }

    /// `FUN_00a88ea0`: the wave offset (metres above 200) at `(x, z)`; `gate_y` is the height the
    /// native getter tests (the camera's), `r` the camera position.
    pub fn offset(&self, x: f32, z: f32, gate_y: f32, r: Vec3) -> f32 {
        if (gate_y - SEA_LEVEL).abs() > 5.0 {
            return 0.0;
        }
        if (r.z - z) * (r.z - z) + (r.x - x) * (r.x - x) > REACH_SQ {
            return 0.0;
        }
        let Some(level) = clipmap_level_at(Vec2::new(r.x, r.z), Vec2::new(x, z)) else {
            return 0.0;
        };
        let side = clipmap_side(level);
        let cell = side * 0.25;
        // the patch origin, then the cell of the point inside it
        let ox = floor_f((x - CLIPMAP_ORIGIN) / side) * side + CLIPMAP_ORIGIN;
        let oz = floor_f((z - CLIPMAP_ORIGIN) / side) * side + CLIPMAP_ORIGIN;
        let x0 = floor_f((x - ox) / cell) * cell + ox;
        let z0 = floor_f((z - oz) / cell) * cell + oz;
        let (x1, z1) = (x0 + cell, z0 + cell);
        // the diagonal: below it the corner (x0, z0) closes the triangle, above it (x1, z1)
        let first = if (z - z0) + (x - x0) < cell {
            (x0, z0)
        } else {
            (x1, z1)
        };
        let v = [first, (x1, z0), (x0, z1)];
        let h = v.map(|(vx, vz)| self.vertex(vx, vz, gate_y, r));
        // the plane through the three vertices, evaluated at the point
        let (ax, az, ah) = (v[0].0, v[0].1, h[0]);
        let (e1x, e1z, e1h) = (v[1].0 - ax, v[1].1 - az, h[1] - ah);
        let (e2x, e2z, e2h) = (v[2].0 - ax, v[2].1 - az, h[2] - ah);
        let det = e1x * e2z - e2x * e1z;
        if det.abs() < 1e-9 {
            return ah;
        }
        let (px, pz) = (x - ax, z - az);
        let s = (px * e2z - e2x * pz) / det;
        let t = (e1x * pz - px * e1z) / det;
        ah + s * e1h + t * e2h
    }

    /// `FUN_00a8bba0`: the wave height (absolute y) at `(x, z)`. With no camera the reference is
    /// `fallback` (non-native stand-in: the querying body itself).
    pub fn height(&self, x: f32, z: f32, fallback: Vec3) -> f32 {
        let r = self.camera.unwrap_or(fallback);
        SEA_LEVEL + self.offset(x, z, r.y, r)
    }
}

/// Rebuild the table from the clock and follow the world camera.
pub fn sea_waves_step(
    time: Res<Time>,
    cams: Query<&GlobalTransform, (With<Camera3d>, With<PostPass>)>,
    mut waves: ResMut<SeaWaves>,
) {
    let clock = (time.elapsed_secs_f64() * 1000.0) as u32;
    if waves.clock != clock {
        let camera = waves.camera;
        *waves = SeaWaves::at_clock(clock);
        waves.camera = camera;
    }
    // without a camera the reference stays as set (tests pin it)
    if let Some(t) = cams.iter().next() {
        waves.camera = Some(t.translation());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_sine_is_close_to_sine() {
        for i in -40..=40 {
            let a = i as f32 * 0.37;
            let (s, c) = sin_cos(a);
            assert!((s - a.sin()).abs() < 0.002, "sin({a}) = {s}");
            assert!((c - a.cos()).abs() < 0.002, "cos({a}) = {c}");
        }
    }

    #[test]
    fn table_is_mirrored_and_bounded() {
        let w = SeaWaves::at_clock(123_456);
        for k in 0..16 {
            assert_eq!(w.grad[k], w.grad[k + 16]);
            assert!(w.grad[k][0].abs() <= 1.001, "unit row");
            // the second component is the look-at matrix's y, which is zero for an up vector of +y
            assert_eq!(w.grad[k][1], 0.0);
        }
        assert_eq!(w.grad[32], w.grad[0]);
        assert_eq!(w.grad[33], w.grad[1]);
        // same clock, same table (the LCG restarts at 0 every call)
        assert_eq!(SeaWaves::at_clock(123_456).grad, w.grad);
        assert_ne!(SeaWaves::at_clock(1000).grad, w.grad);
    }

    #[test]
    fn noise_is_continuous_and_zero_at_lattice_points() {
        let w = SeaWaves::at_clock(5000);
        // gradient noise is zero at its lattice points
        for (x, z) in [(3.0, 5.0), (10.0, 2.0), (0.0, 0.0)] {
            assert!(w.noise(x, z).abs() < 1e-5, "noise({x},{z})");
        }
        let mut last = w.noise(1.0, 1.0);
        for i in 1..=2000 {
            let x = 1.0 + i as f32 * 0.002;
            let n = w.noise(x, 1.37);
            assert!((n - last).abs() < 0.05, "jump at {x}");
            last = n;
        }
    }

    #[test]
    fn wave_is_bounded_gated_and_fades_out() {
        let w = SeaWaves::at_clock(77_777);
        let cam = Vec3::new(1000.0, 202.0, -2000.0);
        let mut seen = 0.0f32;
        for i in 0..400 {
            let x = cam.x + (i % 20) as f32 * 2.5 - 25.0;
            let z = cam.z + (i / 20) as f32 * 2.5 - 25.0;
            let h = w.offset(x, z, cam.y, cam);
            assert!(h.abs() <= 0.6, "wave {h} above the 0.6 m amplitude");
            seen = seen.max(h.abs());
        }
        assert!(seen > 0.05, "some wave near the camera: {seen}");
        // beyond 66.67 m and with the camera higher than 5 m above the sea: flat
        assert_eq!(w.offset(cam.x + 70.0, cam.z, cam.y, cam), 0.0);
        let high = Vec3::new(cam.x, 220.0, cam.z);
        assert_eq!(w.offset(cam.x + 3.0, cam.z + 3.0, high.y, high), 0.0);
        // the height is 200 + offset
        let h = w.height(cam.x + 5.0, cam.z + 7.0, cam);
        assert!((h - 200.0 - w.offset(cam.x + 5.0, cam.z + 7.0, cam.y, cam)).abs() < 1e-4);
    }

    #[test]
    fn height_follows_the_vertices_of_its_cell() {
        // on a vertex of the 4 m cell grid the height is that vertex's own value
        let w = SeaWaves::at_clock(31_337);
        let cam = Vec3::new(8.0, 200.0, 8.0);
        let (x, z) = (12.0, 4.0);
        let v = w.vertex(x, z, cam.y, cam);
        assert!((w.offset(x, z, cam.y, cam) - v).abs() < 1e-5);
    }
}
