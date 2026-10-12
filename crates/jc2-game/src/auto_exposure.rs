//! Auto-exposure maths, the CPU reference of `auto_exposure.wgsl` (JC2-137, phase 2 of JC2-134;
//! research `research/formats/night-exposure-next.md` section 1, the `CalculateToneMapping`
//! pass `FUN_0093e660` of the exe and its probe `research/tools/night_exposure_probe.py`).
//!
//! Native chain (brief 1.2 / 1.3), per frame, on the pre-post (clamped, 8-bit sRGB) scene colour:
//!
//! 1. `SampleLumInitial`: a 64x64 map of the mean of four taps of `dot(rgb, LUMINANCE)`.
//! 2. `DownScale2x2` twice: 16x16, then 4x4 (mean of four taps).
//! 3. `SampleLumFinal`: four taps of the 4x4 map, then `avg = max(mean, 0.01)`,
//!    `T = 1 + y * (lerp(key / avg, max_i key / max(l_i, 0.01), x) - 1)`.
//! 4. `CalculateAdaptedLum`: `A' = prev + (T - prev) * (1 - exp(-1.74878 dt'))`, `dt'` clamped
//!    0.01..0.5 s, `A'` clamped 0..4 (NaN -> `T`, still NaN -> 0.5). The first frame is `A = T`.
//!
//! Status of the parts:
//!
//! * **Native (shader disassembly byte-pinned in the probe):** the luminance weights, the
//!   0.01 floors, the target formula, the adaptation rate 1.74878, the dt and A clamps, the NaN
//!   rules, key 0.32, `x`/`y` from the hour blocks (`PostEffectState`), first frame `A = T`.
//! * **Inferred (brief Gap 5, decoded pseudo-C of `FUN_00933700`, never run against a retail
//!   frame):** the tap geometry, i.e. [`TAP_OFFSETS`] (in units of source texels, added to the
//!   target pixel centre, bilinear sampler), the `0.5 uv + 0.25` mapping of the final stage,
//!   the map sizes 64 / 16 / 4 and that the first stage reads the full swap-chain-sized frame.
//! * **Native (JC2-135, `post-manager-defaults-next.md` section 2):** `dt` is the smoothed game
//!   clock of `FUN_004d3690(0)`, see [`ClockModel`]. **Inferred:** the elapsed milliseconds
//!   come from Bevy's real-time delta rounded to whole ms (native `timeGetTime`), and the time
//!   scale is 1.0 (slow motion is not ported).
//! * **Inferred (brief Gap 7):** one frame of latency (the pass runs after the composite, so
//!   the composite of frame N uses the value metered from frame N-1). The native ring texture
//!   may add a second frame of lag; that is not modelled.
//!
//! The same arithmetic is in `auto_exposure.wgsl`; the GPU path is
//! `post_effects_render.rs`.

// The GPU does this arithmetic in WGSL; the CPU functions are its reference and the tests' subject.
#![allow(dead_code)]

use bevy::prelude::{Vec2, Vec3};

/// `SampleLumInitial` weights (also the saturation term of the composite).
pub const LUMINANCE: Vec3 = Vec3::new(0.2125, 0.7154, 0.0721);
/// The floor of every luminance in the ratio (`max(l, 0.01)`).
pub const LUMINANCE_FLOOR: f32 = 0.01;
/// `CalculateAdaptedLum` rate: `1 - exp(-ADAPT_RATE dt)`.
pub const ADAPT_RATE: f32 = 1.74878;
/// `dt` clamp of `CalculateAdaptedLum`.
pub const DT_MIN: f32 = 0.01;
pub const DT_MAX: f32 = 0.5;
/// `A` clamp of `CalculateAdaptedLum`.
pub const A_MAX: f32 = 4.0;
/// The map sizes (`tone_mapping_downsampling` textures, R32_FLOAT).
pub const MAP_INITIAL: usize = 64;
pub const MAP_MID: usize = 16;
pub const MAP_FINAL: usize = 4;
/// **Inferred tap geometry** (brief 1.2, `FUN_00933700`): the four tap offsets in source texels
/// `(-0.5, -0.5), (-0.5, +1.5), (+1.5, -0.5), (+1.5, +1.5)` from the target pixel centre
/// (divide by the source size to get uv).
pub const TAP_OFFSETS: [Vec2; 4] = [
    Vec2::new(-0.5, -0.5),
    Vec2::new(-0.5, 1.5),
    Vec2::new(1.5, -0.5),
    Vec2::new(1.5, 1.5),
];

/// The luminance of a linear colour.
pub fn luminance(c: Vec3) -> f32 {
    c.dot(LUMINANCE)
}

/// `SampleLumFinal`: the target `T` from the four luminance taps and the block values `x`
/// (weight of the darkest tap, `S8`), `y` (strength, `S9`) and `key` (`S10`).
pub fn exposure_target(taps: [f32; 4], x: f32, y: f32, key: f32) -> f32 {
    let mean = taps.iter().sum::<f32>() * 0.25;
    let r_avg = key / mean.max(LUMINANCE_FLOOR);
    let r_max = taps
        .iter()
        .map(|l| (key / l.max(LUMINANCE_FLOOR)).max(0.0))
        .fold(f32::MIN, f32::max);
    1.0 + y * ((r_avg + x * (r_max - r_avg)) - 1.0)
}

/// Clamp of one elapsed sample in the game clock tick `FUN_004e1d90`, milliseconds.
pub const CLOCK_SAMPLE_MAX_MS: f32 = 67.0;
/// Smoothing divisor of the game clock (smoothing factor 10, divisor 11).
pub const CLOCK_SMOOTHING_DIVISOR: f32 = 11.0;
/// Value of `FUN_004d3690(0)` while the tutorial pause flag is set (the exposure pass clamps
/// it to [`DT_MIN`]).
pub const CLOCK_PAUSED_DT: f32 = 1e-6;

/// The game clock's dt (timer+0x14 of the `CClock` singleton, tick `FUN_004e1d90`), the value
/// the post pass receives. **Native** (JC2-135): constructor 1/60; each tick moves the stored
/// dt 1/11 of the way to `min(elapsed_ms, 67) * 0.001 * time_scale`, without a floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockModel {
    /// The stored smoothed dt in seconds.
    pub dt: f32,
}

impl Default for ClockModel {
    fn default() -> Self {
        Self { dt: 1.0 / 60.0 }
    }
}

impl ClockModel {
    /// One frame's tick; returns the new dt. `elapsed_ms` is 0 for the very first tick.
    pub fn tick(&mut self, elapsed_ms: f32, time_scale: f32) -> f32 {
        let sample = elapsed_ms.min(CLOCK_SAMPLE_MAX_MS) * 0.001 * time_scale;
        self.dt += (sample - self.dt) / CLOCK_SMOOTHING_DIVISOR;
        self.dt
    }

    /// What `FUN_004d3690(0)` returns: the stored dt, or 1e-6 while paused.
    pub fn post_dt(&self, paused: bool) -> f32 {
        if paused { CLOCK_PAUSED_DT } else { self.dt }
    }
}

/// `CalculateAdaptedLum`: one adaptation step from `prev` toward `target` over `dt` seconds.
pub fn adapt_exposure(prev: f32, target: f32, dt: f32) -> f32 {
    let k = 1.0 - (-ADAPT_RATE * dt.clamp(DT_MIN, DT_MAX)).exp();
    let mut v = prev + (target - prev) * k;
    if v.is_nan() {
        v = target;
    }
    if v.is_nan() {
        v = 0.5;
    }
    v.clamp(0.0, A_MAX)
}

/// The exposure state the GPU holds per view: the value the composite applies this frame and
/// the latest adapted value (see the module docs: one frame of latency).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExposureState {
    /// `A` the composite of the current frame multiplies by.
    pub applied: f32,
    /// The latest adapted `A` (becomes `applied` next frame).
    pub latest: f32,
    /// False until the first frame has been metered.
    pub inited: bool,
}

impl Default for ExposureState {
    fn default() -> Self {
        Self {
            applied: 1.0,
            latest: 1.0,
            inited: false,
        }
    }
}

impl ExposureState {
    /// One frame: meter target `target`, frame delta `dt`. The first frame is `A = T`
    /// (native: the previous texture is the target itself; flag `L+0x16c`); the choice that
    /// that frame's own composite already uses it is **non-native** (the native texture on
    /// frame 0 is unknown, brief Gap 7). Later frames apply last frame's value.
    pub fn step(&mut self, target: f32, dt: f32) {
        if !self.inited {
            self.inited = true;
            self.latest = target.clamp(0.0, A_MAX);
            self.applied = self.latest;
        } else {
            self.applied = self.latest;
            self.latest = adapt_exposure(self.latest, target, dt);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// CPU reference of the luminance chain (the unit tests; the GPU does the same in WGSL)

/// Bilinear fetch of a square `n x n` map at `uv` (clamp to edge, texel centres at +0.5), like
/// the hardware sampler the native pass uses (`MIN_MAG_LINEAR`).
pub fn bilerp(map: &[f32], n: usize, uv: Vec2) -> f32 {
    let p = uv * n as f32 - Vec2::splat(0.5);
    let f = p.floor();
    let t = p - f;
    let at = |dx: i32, dy: i32| {
        let x = (f.x as i32 + dx).clamp(0, n as i32 - 1) as usize;
        let y = (f.y as i32 + dy).clamp(0, n as i32 - 1) as usize;
        map[y * n + x]
    };
    let top = at(0, 0) * (1.0 - t.x) + at(1, 0) * t.x;
    let bottom = at(0, 1) * (1.0 - t.x) + at(1, 1) * t.x;
    top * (1.0 - t.y) + bottom * t.y
}

/// `DownScale2x2` from an `src_n` map to an `out_n` map (mean of four bilinear taps).
pub fn downscale(src: &[f32], src_n: usize, out_n: usize) -> Vec<f32> {
    let mut out = vec![0.0; out_n * out_n];
    for y in 0..out_n {
        for x in 0..out_n {
            let uv = (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / out_n as f32;
            out[y * out_n + x] = TAP_OFFSETS
                .iter()
                .map(|o| bilerp(src, src_n, uv + *o / src_n as f32))
                .sum::<f32>()
                * 0.25;
        }
    }
    out
}

/// The four taps of `SampleLumFinal` on the 4x4 map: `uv' = (0.5 + offset) * 0.5 + 0.25`.
pub fn final_taps(map4: &[f32]) -> [f32; 4] {
    TAP_OFFSETS.map(|o| {
        let uv = (Vec2::splat(0.5) + o / MAP_FINAL as f32) * 0.5 + Vec2::splat(0.25);
        bilerp(map4, MAP_FINAL, uv)
    })
}

/// The whole chain on a scene given as a function `uv -> linear colour` (the bilinear sampled
/// 8-bit frame of `size` pixels): the target `T` for the block values `x`, `y`, `key`.
pub fn meter_target(scene: impl Fn(Vec2) -> Vec3, size: Vec2, x: f32, y: f32, key: f32) -> f32 {
    let mut map64 = vec![0.0; MAP_INITIAL * MAP_INITIAL];
    for j in 0..MAP_INITIAL {
        for i in 0..MAP_INITIAL {
            let uv = (Vec2::new(i as f32, j as f32) + Vec2::splat(0.5)) / MAP_INITIAL as f32;
            map64[j * MAP_INITIAL + i] = TAP_OFFSETS
                .iter()
                .map(|o| luminance(scene(uv + *o / size)))
                .sum::<f32>()
                * 0.25;
        }
    }
    let map16 = downscale(&map64, MAP_INITIAL, MAP_MID);
    let map4 = downscale(&map16, MAP_MID, MAP_FINAL);
    exposure_target(final_taps(&map4), x, y, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: f32 = 0.32;
    const NIGHT: (f32, f32) = (0.0, 0.08);
    const DAY: (f32, f32) = (0.3, 0.10);

    fn uniform(l: f32) -> [f32; 4] {
        [l; 4]
    }

    /// The probe's steady-state table (brief 1.4): `A = T` for a uniform scene.
    #[test]
    fn the_clock_smooths_clamps_and_scales_like_the_tick() {
        let mut c = ClockModel::default();
        // the very first tick (elapsed 0) moves 1/60 to 0.0151515
        assert!((c.tick(0.0, 1.0) - 0.015_151_5).abs() < 1e-6);
        // steady 17 ms frames converge to 0.017
        let mut c = ClockModel::default();
        for _ in 0..300 {
            c.tick(17.0, 1.0);
        }
        assert!((c.dt - 0.017).abs() < 1e-5);
        // a 500 ms hitch moves dt by (0.067 - 1/60) / 11 only
        let mut c = ClockModel::default();
        let want = 1.0 / 60.0 + (0.067 - 1.0 / 60.0) / 11.0;
        assert!((c.tick(500.0, 1.0) - want).abs() < 1e-7);
        // time scale 0.5 halves the steady dt
        let mut c = ClockModel::default();
        for _ in 0..300 {
            c.tick(20.0, 0.5);
        }
        assert!((c.dt - 0.010).abs() < 1e-5);
        // paused: 1e-6, which the exposure step clamps to 0.01
        assert_eq!(c.post_dt(true), CLOCK_PAUSED_DT);
        assert_eq!(
            adapt_exposure(1.0, 2.0, c.post_dt(true)),
            adapt_exposure(1.0, 2.0, DT_MIN)
        );
    }

    #[test]
    fn the_steady_state_table_matches_the_probe() {
        let means = [0.02, 0.05, 0.1, 0.2, 0.32, 0.5, 0.8];
        let night = [2.200, 1.432, 1.176, 1.048, 1.000, 0.971, 0.952];
        let day = [2.500, 1.540, 1.220, 1.060, 1.000, 0.964, 0.940];
        for (i, m) in means.iter().enumerate() {
            let n = exposure_target(uniform(*m), NIGHT.0, NIGHT.1, KEY);
            let d = exposure_target(uniform(*m), DAY.0, DAY.1, KEY);
            assert!((n - night[i]).abs() < 5e-4, "night {m}: {n}");
            assert!((d - day[i]).abs() < 5e-4, "day {m}: {d}");
        }
        // exact probe checks
        assert!((exposure_target(uniform(0.05), 0.0, 0.08, KEY) - 1.432).abs() < 1e-6);
        assert!((exposure_target(uniform(0.8), 0.3, 0.1, KEY) - 0.94).abs() < 1e-6);
        // mean equal to the key: T = 1 for any x, y
        assert!((exposure_target(uniform(KEY), 0.3, 0.1, KEY) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_floor_and_the_clamp_follow_the_brief() {
        // black frame: floor 0.01 -> 1 + 0.08 (32 - 1) = 3.48 (night), never reaches 4
        let t = exposure_target(uniform(0.0), 0.0, 0.08, KEY);
        assert!((t - 3.48).abs() < 1e-5, "{t}");
        // only y >= 0.0968 would reach the 0..4 clamp at a black frame
        let t_hi = exposure_target(uniform(0.0), 0.0, 0.1, KEY);
        assert!(t_hi > A_MAX, "{t_hi}");
        assert_eq!(adapt_exposure(A_MAX, t_hi, 100.0), A_MAX);
        // the darkest tap pulls the target up with x: one black tap among 0.32 taps
        let taps = [0.0, 0.32, 0.32, 0.32];
        let avg = exposure_target(taps, 0.0, 0.1, KEY);
        let dark = exposure_target(taps, 0.3, 0.1, KEY);
        assert!(dark > avg);
        let r_avg = KEY / 0.24;
        let r_max = KEY / 0.01;
        assert!((dark - (1.0 + 0.1 * (r_avg + 0.3 * (r_max - r_avg) - 1.0))).abs() < 1e-4);
        // x = 1 uses only the darkest tap
        assert!((exposure_target(taps, 1.0, 1.0, KEY) - r_max).abs() < 1e-3);
    }

    #[test]
    fn the_adaptation_follows_the_probe() {
        // 60 frames of dt 1/60 toward 2.0 from 1.0 reach 1 + (1 - exp(-1.74878)) = 1.826
        let mut a = 1.0;
        for _ in 0..60 {
            a = adapt_exposure(a, 2.0, 1.0 / 60.0);
        }
        // 60 *discrete* steps of 1 - exp(-1.74878/60) are exactly the continuous exponential
        assert!(
            (a - (1.0 + (1.0 - (-ADAPT_RATE).exp()))).abs() < 1e-4,
            "{a}"
        );
        // dt clamps to 0.5 at most (factor 0.582), result clamps 0..4
        let k5 = 1.0 - (-ADAPT_RATE * 0.5).exp();
        assert!((adapt_exposure(1.0, 3.0, 100.0) - (1.0 + 2.0 * k5)).abs() < 1e-6);
        assert!((k5 - 0.582).abs() < 1e-3);
        assert_eq!(adapt_exposure(1.0, 9.0, 100.0), 4.0);
        assert_eq!(adapt_exposure(1.0, -9.0, 100.0), 0.0);
        // dt clamps to 0.01 at least
        let k1 = 1.0 - (-ADAPT_RATE * 0.01).exp();
        assert!((adapt_exposure(1.0, 2.0, 0.0) - (1.0 + k1)).abs() < 1e-6);
        // NaN: target is used, a NaN target gives 0.5
        assert_eq!(adapt_exposure(f32::NAN, 2.0, 0.1), 2.0);
        assert_eq!(adapt_exposure(f32::NAN, f32::NAN, 0.1), 0.5);
    }

    #[test]
    fn the_state_has_one_frame_of_latency_and_starts_at_the_target() {
        let mut s = ExposureState::default();
        s.step(1.5, 0.016);
        // frame 0: A = T (also applied by its own composite, non-native)
        assert_eq!((s.applied, s.latest), (1.5, 1.5));
        // the scene turns dark: target 3.0
        s.step(3.0, 0.1);
        assert_eq!(s.applied, 1.5, "the composite uses last frame's value");
        let k = 1.0 - (-ADAPT_RATE * 0.1).exp();
        assert!((s.latest - (1.5 + 1.5 * k)).abs() < 1e-6);
        s.step(3.0, 0.1);
        assert!((s.applied - (1.5 + 1.5 * k)).abs() < 1e-6);
    }

    /// The chain on a uniform frame is exactly the table (uniform luminance survives the
    /// bilinear maps), on a 1920x1080 frame.
    #[test]
    fn the_chain_on_uniform_frames_is_the_table() {
        let size = Vec2::new(1920.0, 1080.0);
        for (grey, want_night, want_day) in [(0.05, 1.432, 1.540), (0.8, 0.952, 0.940)] {
            // luminance weights sum to 1.0 so a grey colour has luminance `grey`
            let scene = |_: Vec2| Vec3::splat(grey);
            let n = meter_target(scene, size, NIGHT.0, NIGHT.1, KEY);
            let d = meter_target(scene, size, DAY.0, DAY.1, KEY);
            assert!((n - want_night).abs() < 1e-3, "night {n}");
            assert!((d - want_day).abs() < 1e-3, "day {d}");
        }
    }

    /// The per-axis weights of the final taps on the 4x4 map are (0, 0.375, 0.5, 0.125)
    /// (brief 1.3: a centre-weighted meter ignoring the first row and column).
    #[test]
    fn the_final_taps_are_centre_weighted() {
        let weights = [0.0_f32, 0.375, 0.5, 0.125];
        for ty in 0..4 {
            for tx in 0..4 {
                let mut map = [0.0; 16];
                map[ty * 4 + tx] = 1.0;
                let mean = final_taps(&map).iter().sum::<f32>() * 0.25;
                assert!(
                    (mean - weights[tx] * weights[ty]).abs() < 1e-6,
                    "texel ({tx},{ty}): {mean}"
                );
            }
        }
    }

    /// A bright patch in the left top corner texel row/column does not matter; the centre does.
    #[test]
    fn the_chain_ignores_the_outer_row_and_follows_the_centre() {
        let size = Vec2::new(1280.0, 720.0);
        // an extremely bright strip on the left 1/8 and top 1/8 of the frame is invisible
        let scene = |uv: Vec2| {
            if uv.x < 0.1 || uv.y < 0.1 {
                Vec3::splat(1.0)
            } else {
                Vec3::splat(0.05)
            }
        };
        let t = meter_target(scene, size, NIGHT.0, NIGHT.1, KEY);
        let uniform_t = exposure_target(uniform(0.05), NIGHT.0, NIGHT.1, KEY);
        assert!((t - uniform_t).abs() < 0.02, "{t} vs {uniform_t}");
        // a bright centre darkens the target (T < 1) while a dark centre brightens it
        let bright_c = |uv: Vec2| {
            if (uv - Vec2::splat(0.5)).abs().max_element() < 0.25 {
                Vec3::splat(0.9)
            } else {
                Vec3::splat(0.05)
            }
        };
        assert!(meter_target(bright_c, size, NIGHT.0, NIGHT.1, KEY) < 1.0);
    }
}
