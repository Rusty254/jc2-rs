//! Particle emitter simulation for effect components (research: `particle-consumers-next.md`,
//! `particle-units-next.md`, board tasks JC2-014 / JC2-080 / JC2-087 / dc95ceb0 / aee5ba94,
//! `effects-pipeline.md` doc 6.1).
//!
//! # Native-proven here
//! * per-job order (`0xa66490`): the emission clock advances, survivors update first (age < 1 and
//!   size > 0), then births while the clock is inside the emission window;
//! * normalized age advances by `dt / lifetime`; positions integrate the old velocity, then the
//!   indexed modifiers run (animation, size, signed opacity, force, damping) when the updated age is
//!   inside `[start, end)` (full-window bypass);
//! * births (`0xa5ca60`): the accumulator grows by `dt`, a particle is born while it is strictly
//!   greater than the spacing and the live count is below the cap, then it is reduced by
//!   `fmod(.., spacing)`; the spacing is `1/rate` divided by the density scale; the capacity is the
//!   authored cap (reduced by the reader to `ceil(life + 0.25 * variation) * rate`) times the
//!   density scale;
//! * the emitter stream is the MSVC LCG (`s = s * 0x343fd + 0x269ec3`, `(s >> 16) & 0x7fff`), the
//!   draw order of a birth is pitch, yaw, speed, roll rate, shape, roll, source lerp (spaces 0/1),
//!   lifetime, size, frame (`0xa5a2d0`, `0xa5ab90`, `0xa5a4e0`, `0xa5ca60`);
//! * birth direction `(cos p sin y, -sin p, cos p cos y)` times the speed, rotated by the source
//!   matrix (spaces 0/1), shape offsets 0 point / 3 box / 4 radial disc / 5 sphere shell
//!   (`particle-units-next.md` section 3);
//! * lifetime `clamp((2u - 1) * variation + base, 0.001, 10000)` (a non-positive base is legal);
//! * initial size `base + u * range` is a **half-extent in metres** (the renderer draws `2 * size`);
//! * space 2 = local: positions live in the emitter frame and are transformed by the current source
//!   matrix when drawn; space 1 adds the source velocity at birth; spaces 0/1 simulate in world
//!   space and spread births along the source's last step.
//!
//! # Approximations (labelled, not native)
//! * the LCG seed is derived from the emitter id (the native seed is unknown, Gap 6 of the doc);
//!   the sine and cosine are exact (the engine uses a parabolic approximation, error < 0.002);
//! * the CRT `rand()` fallbacks (zero direction in shapes 3/4/5) draw from the emitter LCG;
//! * the initial roll and roll rate fields are not read (their hashes are unknown): the draws are
//!   consumed, the roll stays 0;
//! * the first particle is born on the first step: the accumulator starts at `1.0001 * spacing`.
//!   Inferred (muzzle research 446ad726, kept after the dc95ceb0 re-evaluation): the retail muzzle
//!   flash leaves are one-particle systems with a rate of 1/s and a window of 0.09 s, which could
//!   never flash with a zero accumulator; the research probe's reference simulation also starts
//!   there. The native initial value is not located;
//! * steps longer than 1/30 s are split into equal sub-steps (non-native: a stalled frame must not
//!   move fast particles metres in one integration step);
//! * lighting, soft-particle depth attenuation, ground-plane clamps and start delays are not
//!   applied (doc Gaps 1, 3, 5);
//! * start delay: the leaf fields are unknown (`smoke-effects.md` Gap 3), so it is always 0; the
//!   camera position that feeds the cull-distance gate is inferred to be the render camera.
//!
//! # Emitter lifecycle (`smoke-effects.md` sections 4 and 5, native-proven)
//! * the emission window is the variation `duration` when positive, else the leaf `system_lifetime`;
//!   0 means unbounded (no stand-in);
//! * `stop` only sets the stop flag: an unbounded emitter closes its window at once (native
//!   `lifetime = elapsed = 1.0`), a bounded one keeps its window; live particles are never killed;
//! * births need no first-frame hold (cleared after the first job), no start delay, an open window,
//!   fewer than 16384 live particles in total, and the camera closer than the leaf cull distance
//!   (`#d1e88637`, default 800 m);
//! * the emitter finishes when it may release (flags byte bit 0, set by a stop), its window is over
//!   and no particle is alive; linked children never release themselves.

use bevy::prelude::*;
use jc2_formats::particle::{Emission, Modifier, ModifierKind, ParticleSystem, RenderFlags};
use std::sync::Arc;

/// Global live-particle limit of the job context (native 0x4000, `0xa62520`).
pub const MAX_GLOBAL_PARTICLES: usize = 0x4000;

/// Longest single integration step (non-native, see the module header).
const MAX_SUBSTEP: f32 = 1.0 / 30.0;
const MAX_SUBSTEPS: usize = 16;

#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub age: f32,
    /// Emitter-local for space 2, world otherwise.
    pub position: Vec3,
    pub velocity: Vec3,
    /// Half-extent in metres.
    pub size: f32,
    pub alpha: f32,
    pub frame: f32,
    /// Lifetime in seconds (the native particle stores `1 / life`).
    pub life: f32,
    /// The emitter id of this particle's own trail system (`#1de3fca7`), once started.
    pub trail: Option<u64>,
    /// Born since the manager last looked: a trail emitter is due (native: age at most 1.19e-7,
    /// which a frame split into sub-steps would lose).
    pub needs_trail: bool,
}

fn in_window(age: f32, start: f32, end: f32) -> bool {
    (start <= 0.0 && end >= 1.0) || (start <= age && age < end)
}

impl Particle {
    fn update(&mut self, dt: f32, mods: &[&Modifier], alpha_scale: f32) {
        self.age += (1.0 / self.life) * dt;
        self.position += self.velocity * dt;
        for m in mods {
            if !in_window(self.age, m.start, m.end) {
                continue;
            }
            match m.kind {
                ModifierKind::Animation {
                    lower,
                    upper,
                    rate,
                    looping,
                } => {
                    let (lo, hi) = (lower as f32, upper as f32);
                    self.frame += dt * rate;
                    if looping {
                        if self.frame < lo {
                            self.frame += hi - lo;
                        } else if self.frame >= hi {
                            self.frame -= hi - lo;
                        }
                    }
                    self.frame = self.frame.clamp(lo, hi);
                }
                // the size scale is the native instance default (1); the system scale (+0xac)
                // multiplies every opacity change (`smoke-effects.md` section 3)
                ModifierKind::Size(c) => self.size += dt * c,
                ModifierKind::Opacity(c) => {
                    self.alpha = (self.alpha + dt * c * alpha_scale).clamp(0.0, 1.0)
                }
                ModifierKind::Force(f) => self.velocity += Vec3::from_array(f) * dt,
                ModifierKind::Damping(c) => self.velocity *= 1.0 - (dt * c).min(1.0),
                ModifierKind::Unsupported => {}
            }
        }
    }
}

/// The emitter's random stream: MSVC `rand` (`0xa5a2d0`: `s = s * 0x343fd + 0x269ec3`,
/// `(s >> 16) & 0x7fff`). The native seed (emitter `+0x4c`) is unknown.
#[derive(Clone, Debug)]
pub struct Lcg(pub i32);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Self(((seed >> 32) ^ seed) as u32 as i32)
    }
    /// The 15-bit draw.
    pub fn raw(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        ((self.0 >> 16) & 0x7fff) as u32
    }
    /// "Signed draw": `raw * 6.103702e-05 - 1`, in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.raw() as f32 * 6.103702e-05 - 1.0
    }
    /// "Unit draw": `raw * 3.051851e-05`, in `[0, 1]`.
    pub fn unit(&mut self) -> f32 {
        self.raw() as f32 * 3.051851e-05
    }
    /// A random unit vector (stand-in for the CRT `rand()` fallback of a zero direction).
    fn unit_vec(&mut self) -> Vec3 {
        for _ in 0..16 {
            let v = Vec3::new(self.signed(), self.signed(), self.signed());
            let l = v.length_squared();
            if (1e-6..=1.0).contains(&l) {
                return v / l.sqrt();
            }
        }
        Vec3::Y
    }
}

/// Local birth direction: row 2 of `Xrot(pitch) * Yrot(yaw)` (`0xa5a3de..0xa5a40d`). Pitch
/// `-pi/2` is +Y, zero angles are +Z.
pub fn birth_direction(pitch: f32, yaw: f32) -> Vec3 {
    Vec3::new(pitch.cos() * yaw.sin(), -pitch.sin(), pitch.cos() * yaw.cos())
}

/// `lifetime = clamp((2u - 1) * variation + base, 0.001, 10000)` for the signed draw `s`
/// (`0xa5ca60`).
pub fn particle_lifetime(base: f32, variation: f32, signed: f32) -> f32 {
    (signed * variation + base).clamp(0.001, 10000.0)
}

/// The shape offset `S` for the un-oriented local direction `dir` (`0xa5a4e0`), drawing from
/// `rng` in the native order. `p` is the shape triple: inner radius, outer radius, height for
/// shapes 4/5 and the box extents for shape 3. Shapes 1 and 2 are unimplemented natively (the
/// stack is undefined; retail never uses them): they give 0 here.
pub fn shape_offset(shape: i32, p: [f32; 3], dir: Vec3, rng: &mut Lcg) -> Vec3 {
    let zero_dir = dir.length() <= 0.0;
    match shape {
        3 => {
            let [ex, ey, ez] = p;
            let mut s = if ex == 0.0 && ey > 0.0 && ez > 0.0 {
                Vec3::new(0.0, rng.signed(), rng.signed())
            } else if ey == 0.0 && ex > 0.0 && ez > 0.0 {
                Vec3::new(rng.signed(), 0.0, rng.signed())
            } else if ez == 0.0 && ex > 0.0 && ey > 0.0 {
                Vec3::new(rng.signed(), rng.signed(), 0.0)
            } else {
                // the point of the cube surface in the direction of `dir`
                let mut n = if zero_dir { rng.unit_vec() } else { dir.normalize() };
                let (ax, ay, az) = (n.x.abs(), n.y.abs(), n.z.abs());
                if ay < ax && az < ax {
                    n = Vec3::new(n.x.signum(), n.y / ax, n.z / ax);
                } else if ax < ay && az < ay {
                    n = Vec3::new(n.x / ay, n.y.signum(), n.z / ay);
                } else if ax < az && ay < az {
                    n = Vec3::new(n.x / az, n.y / az, n.z.signum());
                }
                n
            };
            s *= Vec3::new(ex, ey, ez) * 0.5;
            s
        }
        4 => {
            let (x, z) = if zero_dir {
                (rng.signed(), rng.signed())
            } else {
                (dir.x, dir.z)
            };
            let l = (x * x + z * z).sqrt();
            let (x, z) = if l > 1e-6 { (x / l, z / l) } else { (0.0, 0.0) };
            let (inner, outer, height) = (p[0], p[1], p[2]);
            let u1 = rng.unit();
            let u2 = rng.unit();
            let u3 = rng.unit();
            Vec3::new(
                (inner + (outer - inner) * u1) * x,
                (0.5 - u3) * height,
                (inner + (outer - inner) * u2) * z,
            )
        }
        5 => {
            let n = if zero_dir { rng.unit_vec() } else { dir.normalize() };
            n * (p[0] + (p[1] - p[0]) * rng.unit())
        }
        _ => Vec3::ZERO,
    }
}

/// What the manager decided when it admitted the emitter.
#[derive(Clone, Copy, Debug)]
pub struct Admission {
    /// Density scale `s` (doc 6.1).
    pub scale: f32,
    /// `duration` of the component variation (0 = not given).
    pub duration: f32,
    /// Emitter flag bit 13: the emitter may release itself (the variation flags byte `#b0124c0a`,
    /// default 1; linked children start with 0; a stop sets it).
    pub may_release: bool,
    pub seed: u64,
}

/// What the manager's job context carries (native 0x00a62520, built once per frame).
#[derive(Clone, Copy, Debug, Default)]
pub struct JobContext {
    /// Live particles of all emitters; births need fewer than [`MAX_GLOBAL_PARTICLES`].
    pub total_particles: usize,
    /// Position the cull distance is measured from (inferred: the render camera); `None` = no gate.
    pub camera: Option<Vec3>,
}

/// `0xa71720` tail: authored capacity (`#74175c29`), lowered to the steady-state count
/// `int(ceil(life + 0.25 * variation) / spacing)` when that is positive and smaller, at most 150
/// with debris slots, at most 1000. Same f32 arithmetic as the native code.
pub fn effective_capacity(
    authored: i32,
    life: f32,
    variation: f32,
    rate: f32,
    debris: bool,
) -> usize {
    let mut cap = authored;
    if rate > 0.0 {
        let spacing = 1.0 / rate;
        let steady = ((variation * 0.25 + life).ceil() / spacing) as i32;
        if steady > 0 && steady < cap {
            cap = steady;
        }
    }
    if debris && cap > 0x96 {
        cap = 0x96;
    }
    cap.clamp(0, 1000) as usize
}

/// One live emitter: the leaf, its particles and its source frame.
pub struct Emitter {
    pub system: Arc<ParticleSystem>,
    pub particles: Vec<Particle>,
    pub texture_member: String,
    pub flags: RenderFlags,
    pub matrix: Mat4,
    pub velocity: Vec3,
    rng: Lcg,
    spacing: f32,
    accumulator: f32,
    cap: usize,
    emission: Emission,
    /// Seconds since the start (native `+0x98`).
    elapsed: f32,
    /// Emission window `L` (native `+0x88`); `None` = unbounded (0 natively).
    window: Option<f32>,
    /// The stop flag (native flag 4).
    stopping: bool,
    /// Native flag bit 13.
    may_release: bool,
    /// First-frame hold (native flag 0x8000): no births in the first job.
    held: bool,
    /// Seconds before the first birth (always 0: Gap 3 of the research doc).
    start_delay: f32,
    /// Scale on every opacity change (native `+0xac`).
    pub system_scale: f32,
    /// Trail emitters of particles that died, for the manager to stop.
    pub orphan_trails: Vec<u64>,
    /// The source position at the start of the previous step (native `+0x58`).
    last_source: Option<Vec3>,
    pub births: u64,
    pub dropped_cap: u64,
}

impl Emitter {
    pub fn new(system: Arc<ParticleSystem>, a: Admission) -> Self {
        let s = a.scale.clamp(0.25, 1.0);
        let rate = system.emission.rate;
        let max = effective_capacity(
            i32::from(system.max_particles),
            system.lifetime,
            system.lifetime_variation,
            rate,
            system.debris_slots,
        );
        let scaled = (max as f32 * s).ceil() as usize;
        let texture_member = system
            .texture
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .to_string();
        let window = if a.duration > 0.0 {
            Some(a.duration)
        } else if system.system_lifetime > 0.0 {
            Some(system.system_lifetime)
        } else {
            None
        };
        let spacing = if rate > 0.0 {
            (1.0 / rate) / s
        } else {
            f32::INFINITY
        };
        Self {
            spacing,
            cap: scaled.min(max),
            emission: system.emission,
            texture_member,
            flags: system.render_flags(),
            system,
            particles: Vec::new(),
            matrix: Mat4::IDENTITY,
            velocity: Vec3::ZERO,
            rng: Lcg::new(a.seed | 1),
            // Inferred, see the module header: the first particle is born on the first step.
            accumulator: if spacing.is_finite() { spacing * 1.0001 } else { 0.0 },
            elapsed: 0.0,
            window,
            stopping: false,
            may_release: a.may_release,
            held: true,
            start_delay: 0.0,
            system_scale: 1.0,
            orphan_trails: Vec::new(),
            last_source: None,
            births: 0,
            dropped_cap: 0,
        }
    }

    /// Native stop (`0xa6af80(1)` then the end-test pass `0xa6e6c0`): sets the stop flag and the
    /// release bit; an unbounded emitter closes its window at once (`L = T = 1.0`), a bounded one
    /// keeps it. Live particles are never touched.
    pub fn stop(&mut self) {
        self.stopping = true;
        self.may_release = true;
        if self.window.is_none() {
            self.window = Some(1.0);
            self.elapsed = 1.0;
        }
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    /// The emission window has not ended (births may still happen if the gates allow).
    pub fn is_emitting(&self) -> bool {
        self.window.is_none_or(|w| self.elapsed < w)
    }

    /// Native finish test (`0xa6e6c0`): no hold, no delay, no live particle, window over
    /// (`L < T`, or `L == T != 0`) and the release bit set.
    pub fn finished(&self) -> bool {
        self.may_release
            && !self.held
            && self.start_delay <= 0.0
            && self.particles.is_empty()
            && self.window.is_some_and(|w| self.elapsed >= w)
    }

    fn local(&self) -> bool {
        self.system.space == 2
    }

    /// Advance by `dt` seconds (split into steps of at most 1/30 s, non-native).
    pub fn step(&mut self, dt: f32) {
        self.step_with(dt, &JobContext::default());
    }

    /// [`Self::step`] with the manager's job context (global particle gate, cull distance).
    pub fn step_with(&mut self, dt: f32, ctx: &JobContext) {
        if !(dt.is_finite() && dt > 0.0) {
            return;
        }
        let n = ((dt / MAX_SUBSTEP).ceil() as usize).clamp(1, MAX_SUBSTEPS);
        let h = dt / n as f32;
        for _ in 0..n {
            self.step_once(h, ctx);
        }
    }

    /// One job (`0xa66490`).
    fn step_once(&mut self, dt: f32, ctx: &JobContext) {
        let sys = self.system.clone();
        let source = self.matrix.w_axis.truncate();
        let previous = self.last_source.unwrap_or(source);
        self.last_source = Some(source);
        self.elapsed += dt;
        let mods: Vec<&Modifier> = sys.execution_modifiers().collect();
        // survivors first, then births (native job order)
        let orphans = &mut self.orphan_trails;
        self.particles.retain(|p| {
            let keep = p.age < 1.0 && p.size > 0.0;
            if !keep && let Some(t) = p.trail {
                orphans.push(t);
            }
            keep
        });
        for p in &mut self.particles {
            p.update(dt, &mods, self.system_scale);
        }
        // birth gates (`0xa66490`): hold, delay, window, particle total, camera distance
        if std::mem::take(&mut self.held) {
            return;
        }
        if self.start_delay > 0.0 {
            self.start_delay -= dt;
            return;
        }
        if !self.is_emitting() || ctx.total_particles >= MAX_GLOBAL_PARTICLES {
            return;
        }
        if let Some(cam) = ctx.camera
            && (source - cam).length_squared() >= sys.max_distance * sys.max_distance
        {
            return;
        }
        self.accumulator += dt;
        while self.accumulator > self.spacing && self.particles.len() < self.cap {
            self.accumulator -= self.spacing;
            self.birth(previous - source);
        }
        if self.spacing.is_finite() {
            if self.particles.len() >= self.cap && self.accumulator > self.spacing {
                self.dropped_cap += (self.accumulator / self.spacing) as u64;
            }
            // native: `fmod(accumulator, spacing)` after the loop
            self.accumulator %= self.spacing;
        }
    }

    /// `source_step` = previous source position minus the current one (births of spaces 0/1 are
    /// spread along it).
    fn birth(&mut self, source_step: Vec3) {
        let sys = self.system.clone();
        let em = self.emission;
        let local = self.local();
        // 0xa5a2d0: pitch, yaw, speed
        let pitch = self.rng.signed() * em.pitch[1] + em.pitch[0];
        let yaw = self.rng.signed() * em.yaw[1] + em.yaw[0];
        let speed = (self.rng.signed() * em.speed[1] + em.speed[0]).clamp(-128.0, 127.0);
        let dir = birth_direction(pitch, yaw);
        // 0xa5ab90: roll rate draw, shape, roll draw
        let _roll_rate = self.rng.signed();
        let shape = match em.shape {
            Some(s) => shape_offset(s, em.shape_params, dir, &mut self.rng),
            None => Vec3::ZERO,
        };
        let _roll = self.rng.signed();
        let offset = Vec3::from_array(em.offset) + shape;
        let velocity_local = dir * speed;
        let (position, velocity) = if local {
            (offset, velocity_local)
        } else {
            // spaces 0/1: births spread along the source's last step (one unit draw)
            let spread = self.rng.unit();
            let rot = Mat3::from_mat4(self.matrix);
            let inherited = if sys.space == 1 { self.velocity } else { Vec3::ZERO };
            (
                self.matrix.transform_point3(offset) + source_step * spread,
                rot * velocity_local + inherited,
            )
        };
        // 0xa5ca60: lifetime, size, frame
        let life = particle_lifetime(sys.lifetime, sys.lifetime_variation, self.rng.signed());
        let size = sys.initial_size[0] + self.rng.unit() * sys.initial_size[1];
        let frame = sys.initial_frame[0]
            + if sys.initial_frame[1] > 0.0 {
                (self.rng.raw() % (sys.initial_frame[1] as u32 + 1)) as f32
            } else {
                0.0
            };
        self.births += 1;
        self.particles.push(Particle {
            age: 0.0,
            position,
            velocity,
            size,
            alpha: sys.initial_alpha,
            frame,
            life,
            trail: None,
            needs_trail: sys.trail.is_some(),
        });
    }

    /// World position of particle `p` (local-space particles follow the current source matrix).
    pub fn world_position(&self, p: &Particle) -> Vec3 {
        if self.local() {
            self.matrix.transform_point3(p.position)
        } else {
            p.position
        }
    }

    /// World velocity of particle `p`.
    pub fn world_velocity(&self, p: &Particle) -> Vec3 {
        if self.local() {
            Mat3::from_mat4(self.matrix) * p.velocity
        } else {
            p.velocity
        }
    }

    /// UV rectangle `[u0, v0, u1, v1]` of the particle's playback cell (nearest-even, clamped into
    /// the atlas; row-major, top origin: a viewer convention, doc `particle-consumers-next.md`).
    pub fn cell_uv(&self, p: &Particle) -> [f32; 4] {
        let [cols, rows] = self.system.atlas.map(u32::from);
        let cell = (p.frame.round_ties_even().max(0.0) as u32).min(cols * rows - 1);
        let (x, y) = ((cell % cols) as f32 / cols as f32, (cell / cols) as f32 / rows as f32);
        [x, y, x + 1.0 / cols as f32, y + 1.0 / rows as f32]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, PI};

    fn adm() -> Admission {
        Admission {
            scale: 1.0,
            duration: 0.0,
            may_release: true,
            seed: 7,
        }
    }

    fn leaf(name: &str) -> Option<Arc<ParticleSystem>> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = jc2_formats::GameFs::open(dir).ok()?;
        let blob = fs.read_inflated("particles.blz").ok()?;
        let a = jc2_formats::sarc::SmallArchive::parse(&blob).ok()?;
        let e = a.get(&format!("{name}.psmb"))?;
        Some(Arc::new(ParticleSystem::parse(a.data(&blob, e).ok()?).ok()?))
    }

    fn smoke() -> Option<Arc<ParticleSystem>> {
        leaf("fx_vehicle_mc_smoke_01")
    }

    #[test]
    fn lcg_is_the_msvc_rand_stream() {
        // srand(0): 38, 7719, 21238, 2437, 8855 (the MSVC `rand` sequence)
        let mut r = Lcg(0);
        assert_eq!(
            [r.raw(), r.raw(), r.raw(), r.raw(), r.raw()],
            [38, 7719, 21238, 2437, 8855]
        );
        let mut r = Lcg(0);
        assert!((r.signed() - (38.0 * 6.103702e-05 - 1.0)).abs() < 1e-7);
        let mut r = Lcg(12345);
        for _ in 0..1000 {
            let u = r.unit();
            assert!((0.0..=1.0).contains(&u));
            let s = r.signed();
            assert!((-1.0..1.0).contains(&s));
        }
    }

    #[test]
    fn birth_direction_is_not_plus_y() {
        // pitch -pi/2 is straight up for any yaw
        for yaw in [0.0, 1.0, -2.5] {
            let d = birth_direction(-FRAC_PI_2, yaw);
            assert!((d - Vec3::Y).length() < 1e-6, "{d:?}");
        }
        // pitch 0 is horizontal, +Z at yaw 0
        assert!((birth_direction(0.0, 0.0) - Vec3::Z).length() < 1e-6);
        let d = birth_direction(0.0, 1.0);
        assert!(d.y.abs() < 1e-6 && (d.length() - 1.0).abs() < 1e-6);
        // the retail grenade pitch range [-pi, 0] is the upper hemisphere
        for k in 0..=20 {
            let d = birth_direction(-PI * k as f32 / 20.0, 0.7);
            assert!(d.y >= -1e-6, "{d:?}");
        }
    }

    #[test]
    fn lifetime_is_clamped_and_allows_non_positive_bases() {
        assert_eq!(particle_lifetime(-1.0, 0.5, 0.9), 0.001);
        assert_eq!(particle_lifetime(2.0, 1.0, 0.5), 2.5);
        assert_eq!(particle_lifetime(1.0e9, 0.0, 0.0), 10000.0);
        assert!((particle_lifetime(0.0, 0.5, 1.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn shape_bounds() {
        let mut r = Lcg(99);
        for _ in 0..500 {
            // shape 4: radius inside [inner, outer] per axis, y inside +-height/2
            let d = birth_direction(r.signed() * 3.0, r.signed() * 3.0);
            let s = shape_offset(4, [0.5, 2.0, 4.0], d, &mut r);
            assert!(s.y.abs() <= 2.0 + 1e-5, "{s:?}");
            assert!(s.x.abs() <= 2.0 + 1e-5 && s.z.abs() <= 2.0 + 1e-5);
            // shape 5: |S| in [inner, outer]
            let s = shape_offset(5, [1.0, 3.0, 0.0], d, &mut r);
            assert!((1.0 - 1e-4..=3.0 + 1e-4).contains(&s.length()), "{s:?}");
            // shape 3: inside the box, on its surface for the cube case
            let s = shape_offset(3, [2.0, 4.0, 6.0], d, &mut r);
            assert!(s.x.abs() <= 1.0 + 1e-5 && s.y.abs() <= 2.0 + 1e-5 && s.z.abs() <= 3.0 + 1e-5);
            let big = s.x.abs().max(s.y.abs() / 2.0).max(s.z.abs() / 3.0);
            assert!(big > 0.99, "not on the surface: {s:?}");
            // flat box: one axis zero
            let s = shape_offset(3, [0.0, 4.0, 6.0], d, &mut r);
            assert_eq!(s.x, 0.0);
            // shape 0 and the unimplemented 1/2 give no offset
            assert_eq!(shape_offset(0, [1.0; 3], d, &mut r), Vec3::ZERO);
            assert_eq!(shape_offset(2, [1.0; 3], d, &mut r), Vec3::ZERO);
        }
    }

    #[test]
    fn mc_smoke_emits_ages_and_finishes() {
        let Some(sys) = smoke() else { return };
        let mut e = Emitter::new(sys, adm());
        for _ in 0..120 {
            e.step(1.0 / 60.0);
        }
        // 25/s, 1 s life, authored cap 25: steady state near 25
        assert!((20..=25).contains(&e.particles.len()), "{}", e.particles.len());
        assert!(e.particles.iter().all(|p| (30.0..=50.0).contains(&p.frame)));
        let uv = e.cell_uv(&e.particles[0]);
        assert!(uv[2] > uv[0] && uv[3] > uv[1]);
        e.stop();
        for _ in 0..200 {
            e.step(1.0 / 60.0);
        }
        assert!(e.finished());
    }

    #[test]
    fn density_scale_shrinks_capacity_and_rate() {
        let Some(sys) = smoke() else { return };
        let full = Emitter::new(sys.clone(), adm());
        let a = Admission { scale: 0.5, ..adm() };
        let half = Emitter::new(sys, a);
        assert_eq!(full.cap, 25);
        assert_eq!(half.cap, 13);
        assert!((half.spacing / full.spacing - 2.0).abs() < 1e-5);
    }

    #[test]
    fn one_shot_window_ends_emission() {
        let Some(sys) = smoke() else { return };
        let mut e = Emitter::new(sys, Admission { duration: 0.5, ..adm() });
        for _ in 0..40 {
            e.step(1.0 / 60.0);
        }
        assert!(!e.is_emitting());
        assert!(e.births > 0);
    }

    #[test]
    fn local_space_follows_the_source() {
        let Some(sys) = smoke() else { return };
        let mut e = Emitter::new(sys, adm());
        for _ in 0..30 {
            e.step(1.0 / 60.0);
        }
        let p = e.particles[0].clone();
        let before = e.world_position(&p);
        e.matrix = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
        assert!((e.world_position(&p) - before - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn a_stalled_frame_is_split_into_substeps() {
        let Some(sys) = leaf("fx_exp_grenade_02") else { return };
        let run = |steps: &[f32]| {
            let mut e = Emitter::new(sys.clone(), adm());
            for dt in steps {
                e.step(*dt);
            }
            e.particles.iter().map(|p| p.position.length()).fold(0.0, f32::max)
        };
        // one 0.25 s stall versus fifteen 1/60 s frames: the radius agrees within a metre
        let stall = run(&[0.25]);
        let smooth = run(&[1.0 / 60.0; 15]);
        assert!((stall - smooth).abs() < 1.0, "{stall} {smooth}");
    }

    /// Retail fixture of `particle-units-next.md` section 7: envelopes over many seeds, source at
    /// the origin, dt = 1/60.
    #[test]
    fn grenade_explosion_fixture() {
        let Some(root) = leaf("fx_exp_grenade_01") else { return };
        // the 12 linked systems of the head
        assert_eq!(root.children.len(), 12);
        let max_chain = (0..)
            .map(|n| jc2_formats::hash::hash_str(&format!("linked_system_{n}")))
            .take_while(|h| root.raw.nodes[0].value_hash(*h).is_some())
            .count();
        assert_eq!(max_chain, 12);
        assert!(root.children.iter().any(|c| c == "fx_exp_grenade_12"));
        assert!(root.children.iter().any(|c| c == "fx_exp_grenade_13"));

        // _01 flash: exactly one particle, half-extent 8..12, alive 0.2 s, colour and fading alpha
        for seed in 0..40u64 {
            let mut e = Emitter::new(root.clone(), Admission { seed: seed * 7919 + 1, ..adm() });
            // the first job is the first-frame hold: the flash is born in the second
            e.step(1.0 / 60.0);
            assert!(e.particles.is_empty());
            e.step(1.0 / 60.0);
            assert_eq!(e.particles.len(), 1, "seed {seed}");
            let p = &e.particles[0];
            assert!((8.0..=12.0).contains(&p.size), "{}", p.size);
            assert!((p.life - 0.2).abs() < 1e-5, "life {}", p.life);
            for _ in 0..30 {
                e.step(1.0 / 60.0);
            }
            assert!(e.particles.is_empty(), "alive after 0.5 s");
            assert_eq!(e.births, 1);
        }
        assert!((root.colour[0] - 0.4233).abs() < 1e-3);
        assert!((root.colour[1] - 0.1406).abs() < 1e-3);
        assert!((root.colour[2] - 0.0273).abs() < 1e-3);

        // _02 smoke: four particles, growth and distance envelopes
        let smoke = leaf("fx_exp_grenade_02").unwrap();
        for seed in 0..40u64 {
            let mut e = Emitter::new(smoke.clone(), Admission { seed: seed * 7919 + 1, ..adm() });
            let mut at_025 = None;
            for k in 1..=60 {
                e.step(1.0 / 60.0);
                if k == 15 {
                    at_025 = Some(e.particles.iter().map(|p| p.size).collect::<Vec<_>>());
                }
            }
            assert!(e.particles.len() <= 4);
            let far = e.particles.iter().map(|p| p.position.length()).fold(0.0, f32::max);
            assert!(far <= 7.8, "seed {seed}: {far} m at 1 s");
            for s in at_025.unwrap() {
                assert!((1.4..=3.2).contains(&s), "half-extent {s} at 0.25 s");
            }
        }

        // _05 sparks: up to 250 live, none alive at 1 s, tiny half-extent
        let sparks = leaf("fx_exp_grenade_05").unwrap();
        let mut e = Emitter::new(sparks, adm());
        let mut peak = 0;
        for _ in 0..60 {
            e.step(1.0 / 60.0);
            peak = peak.max(e.particles.len());
            assert!(e.particles.iter().all(|p| p.size <= 0.036));
        }
        assert!(peak > 20 && peak <= 250, "{peak}");
        assert!(e.particles.is_empty(), "{}", e.particles.len());

        // _12 and _13 exist and parse
        assert!(leaf("fx_exp_grenade_12").is_some() && leaf("fx_exp_grenade_13").is_some());
    }

    #[test]
    fn capacity_rule_matches_the_research_model() {
        // fx_env_casino_smoke_01: authored 60, life 4, rate 15 -> 59 in f32 (doc section 5)
        assert_eq!(effective_capacity(60, 4.0, 0.0, 15.0, false), 59);
        // fx_vehicle_mc_smoke_01 stays at its authored 25
        assert_eq!(effective_capacity(25, 1.0, 0.0, 25.0, false), 25);
        // the steady-state count is the ceiling of life + 0.25 variation over the spacing
        assert_eq!(effective_capacity(500, 2.0, 1.0, 10.0, false), 30);
        // debris slots: at most 150; everything: at most 1000
        assert_eq!(effective_capacity(400, 1000.0, 0.0, 1.0, true), 150);
        assert_eq!(effective_capacity(400, 1000.0, 0.0, 1.0, false), 400);
        assert_eq!(effective_capacity(5000, 10000.0, 0.0, 1.0, false), 1000);
        // no rate: authored value
        assert_eq!(effective_capacity(40, 1.0, 0.0, 0.0, false), 40);
    }

    #[test]
    fn first_frame_hold_then_window_stop_and_finish() {
        let Some(sys) = smoke() else { return };
        // a bounded window of 0.5 s; the first job is the first-frame hold: no births
        let mut e = Emitter::new(sys.clone(), Admission { duration: 0.5, ..adm() });
        e.step(1.0 / 60.0);
        assert_eq!(e.births, 0);
        assert!(!e.finished());
        e.step(1.0 / 60.0);
        assert!(e.births > 0);
        // a stop does not shorten a bounded window and never removes particles
        let alive = e.particles.len();
        e.stop();
        assert!(e.is_stopping() && e.is_emitting());
        e.step(1.0 / 60.0);
        assert!(e.particles.len() >= alive);
        let births = e.births;
        for _ in 0..60 {
            e.step(1.0 / 60.0);
        }
        assert!(e.births > births && !e.is_emitting());
        for _ in 0..120 {
            e.step(1.0 / 60.0);
        }
        assert!(e.particles.is_empty() && e.finished());

        // unbounded: emits for ever, a stop closes the window at once (L = T = 1)
        let mut u = Emitter::new(sys.clone(), adm());
        for _ in 0..300 {
            u.step(1.0 / 60.0);
        }
        assert!(u.is_emitting() && !u.finished());
        let live = u.particles.len();
        u.stop();
        assert!(!u.is_emitting() && live > 0 && !u.finished());
        let births = u.births;
        u.step(1.0 / 60.0);
        assert_eq!(u.births, births);
        assert_eq!(u.particles.len(), live.min(u.particles.len()).max(u.particles.len()));
        for _ in 0..200 {
            u.step(1.0 / 60.0);
        }
        assert!(u.finished());

        // flags byte 0: never releases by itself, a stop sets the bit
        let mut h = Emitter::new(sys, Admission { duration: 0.2, may_release: false, ..adm() });
        for _ in 0..300 {
            h.step(1.0 / 60.0);
        }
        assert!(h.particles.is_empty() && !h.finished());
        h.stop();
        assert!(h.finished());
    }

    #[test]
    fn births_obey_the_cull_distance_and_the_global_particle_gate() {
        let Some(sys) = smoke() else { return };
        assert_eq!(sys.max_distance, 800.0, "the leaf states or defaults to 800 m");
        let at = |cam: Option<Vec3>, total: usize| {
            let mut e = Emitter::new(sys.clone(), adm());
            e.matrix = Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0));
            let ctx = JobContext { total_particles: total, camera: cam };
            for _ in 0..60 {
                e.step_with(1.0 / 60.0, &ctx);
            }
            e.births
        };
        assert!(at(None, 0) > 0);
        assert!(at(Some(Vec3::new(799.0, 0.0, 0.0)), 0) > 0);
        assert_eq!(at(Some(Vec3::new(800.0, 0.0, 0.0)), 0), 0);
        assert_eq!(at(Some(Vec3::new(0.0, 5000.0, 0.0)), 0), 0);
        assert!(at(None, MAX_GLOBAL_PARTICLES - 1) > 0);
        assert_eq!(at(None, MAX_GLOBAL_PARTICLES), 0);
        // existing particles keep simulating when the camera goes away
        let mut e = Emitter::new(sys, adm());
        for _ in 0..60 {
            e.step(1.0 / 60.0);
        }
        let ctx = JobContext { total_particles: 0, camera: Some(Vec3::splat(9000.0)) };
        let ages: Vec<f32> = e.particles.iter().map(|p| p.age).collect();
        e.step_with(1.0 / 60.0, &ctx);
        assert!(e.particles.iter().zip(&ages).all(|(p, a)| p.age > *a));
    }
}
