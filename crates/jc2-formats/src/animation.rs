//! Havok 5.5 animation: skeletons (`.bsk`) and spline-compressed clips (`.ban`).
//! See `research/formats/animation.md`.
//!
//! A `.bsk` is a packfile with an `hkaSkeleton` (bone names, parents, reference pose);
//! the shipped ones carry Havok's reflection data, so the layout below was read off
//! the file itself. A `.ban` holds one `hkaSplineSkeletalAnimation` — the format later
//! known as `hkaSplineCompressedAnimation`: per block, a 4-byte mask per track, then
//! for every track its position, rotation and scale as either nothing (identity), a
//! static value, or a NURBS of quantized control points over frame-indexed knots.

use crate::havok::Packfile;
use crate::{Error, Result};

/// Translation, rotation (x, y, z, w) and scale, engine axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QsTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl QsTransform {
    pub const IDENTITY: QsTransform = QsTransform {
        translation: [0.0; 3],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0; 3],
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
}

/// `hkaSkeleton`: offsets 0 name, 4/8 parentIndices (i16), 12/16 bones (pointers),
/// 20/24 referencePose (hkQsTransform, 48 bytes each).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub name: String,
    pub bones: Vec<Bone>,
    pub reference_pose: Vec<QsTransform>,
}

impl Skeleton {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let pf = Packfile::parse(bytes)?;
        let obj = pf
            .objects_of("hkaSkeleton")
            .next()
            .ok_or_else(|| Error::format("no hkaSkeleton in the packfile"))?;
        let o = obj.offset;
        let name = pf.pointer(o).map(|p| pf.cstr(p)).unwrap_or_default();
        let parents: Vec<i16> = match pf.array(o + 4) {
            Some((p, n)) => (0..n).map(|i| pf.u16(p + 2 * i) as i16).collect(),
            None => Vec::new(),
        };
        let mut bones = Vec::new();
        if let Some((p, n)) = pf.array(o + 12) {
            for i in 0..n {
                let bone = pf.pointer(p + 4 * i);
                let bname = bone
                    .and_then(|b| pf.pointer(b))
                    .map(|s| pf.cstr(s))
                    .unwrap_or_else(|| format!("bone{i}"));
                let parent = parents
                    .get(i as usize)
                    .copied()
                    .filter(|&p| p >= 0)
                    .map(|p| p as usize);
                bones.push(Bone {
                    name: bname,
                    parent,
                });
            }
        }
        let mut reference_pose = Vec::new();
        if let Some((p, n)) = pf.array(o + 20) {
            for i in 0..n {
                let t = p + 48 * i;
                reference_pose.push(QsTransform {
                    translation: pf.vec3(t),
                    rotation: pf.vec4(t + 16),
                    scale: pf.vec3(t + 32),
                });
            }
        }
        if bones.is_empty() {
            return Err(Error::format("skeleton without bones"));
        }
        if reference_pose.len() != bones.len() {
            return Err(Error::format(format!(
                "{} bones but {} reference transforms",
                bones.len(),
                reference_pose.len()
            )));
        }
        Ok(Self {
            name,
            bones,
            reference_pose,
        })
    }

    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones
            .iter()
            .position(|b| b.name.eq_ignore_ascii_case(name))
    }
}

/// One NURBS channel: `knots` empty means a constant (`points[0]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Channel<const N: usize> {
    pub degree: u8,
    pub knots: Vec<u8>,
    pub points: Vec<[f32; N]>,
}

impl<const N: usize> Channel<N> {
    fn constant(v: [f32; N]) -> Self {
        Self {
            degree: 0,
            knots: Vec::new(),
            points: vec![v],
        }
    }

    /// De Boor's algorithm at `u` (frames into the block).
    pub fn sample(&self, u: f32) -> [f32; N] {
        let n = self.points.len();
        if self.knots.is_empty() || n == 1 {
            return self.points[0];
        }
        let p = self.degree as usize;
        let knots = &self.knots;
        let last = knots[n] as f32; // knots has n + p + 1 entries; span ends at knots[n]
        let u = u.clamp(knots[p] as f32, last);
        // the span: largest k in p..n-1 with knots[k] <= u
        let mut k = p;
        while k + 1 < n && (knots[k + 1] as f32) <= u {
            k += 1;
        }
        let mut d: Vec<[f32; N]> = (0..=p).map(|j| self.points[k - p + j]).collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = k - p + j;
                let lo = knots[i] as f32;
                let hi = knots[i + p - r + 1] as f32;
                let alpha = if hi > lo { (u - lo) / (hi - lo) } else { 0.0 };
                let prev = d[j - 1];
                for (c, v) in d[j].iter_mut().enumerate() {
                    *v = (1.0 - alpha) * prev[c] + alpha * *v;
                }
            }
        }
        d[p]
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub position: Channel<3>,
    pub rotation: Channel<4>,
    pub scale: Channel<3>,
}

/// `hkaSplineSkeletalAnimation` (type 5): +12 duration, +16 transform tracks,
/// +36 numFrames, +40 numBlocks, +44 maxFramesPerBlock, +48 maskAndQuantizationSize,
/// +52 blockDuration, +56 its inverse, +60 frameDuration, +64 blockOffsets, +112 data.
#[derive(Clone, Debug, PartialEq)]
pub struct SplineAnimation {
    pub duration: f32,
    pub num_frames: usize,
    pub frame_duration: f32,
    pub block_duration: f32,
    pub max_frames_per_block: usize,
    /// Track names from the annotation tracks (`Hips_AnimationTrack` → `Hips`).
    pub track_names: Vec<String>,
    /// Per block, per track.
    pub blocks: Vec<Vec<Track>>,
    /// The extracted root motion (`hkaDefaultAnimatedReferenceFrame`): one sample per
    /// frame, `[x, y, z, yaw]`, engine axes. Empty when the clip stays in place.
    pub root_motion: Vec<[f32; 4]>,
}

struct Cursor<'a> {
    d: &'a [u8],
    o: usize,
}

impl Cursor<'_> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self
            .d
            .get(self.o)
            .ok_or_else(|| Error::format("animation data ends early"))?;
        self.o += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes([self.u8()?, self.u8()?]))
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes([
            self.u8()?,
            self.u8()?,
            self.u8()?,
            self.u8()?,
        ]))
    }
    fn bytes(&mut self, n: usize) -> Result<&[u8]> {
        let s = self
            .d
            .get(self.o..self.o + n)
            .ok_or_else(|| Error::format("animation data ends early"))?;
        self.o += n;
        Ok(s)
    }
    fn align(&mut self, n: usize) {
        self.o = self.o.div_ceil(n) * n;
    }
}

/// Rotation quantization (bits 2..5 of the mask's first byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RotQuant {
    Polar32,
    ThreeComp40,
    ThreeComp48,
    ThreeComp24,
    Straight16,
    Uncompressed,
}

impl RotQuant {
    fn from_bits(v: u8) -> Result<Self> {
        Ok(match v {
            0 => Self::Polar32,
            1 => Self::ThreeComp40,
            2 => Self::ThreeComp48,
            3 => Self::ThreeComp24,
            4 => Self::Straight16,
            5 => Self::Uncompressed,
            _ => return Err(Error::format(format!("rotation quantization {v}"))),
        })
    }
}

/// 40 bits: three 12-bit components scaled to ±1/√2, the index of the omitted
/// (largest) component in bits 36..37 and its sign in bit 38.
fn quat40(b: &[u8]) -> [f32; 4] {
    let mut v: u64 = 0;
    for (i, &x) in b.iter().enumerate().take(5) {
        v |= (x as u64) << (8 * i);
    }
    const FRACTAL: f32 = 0.000_345_436;
    let m = (1u64 << 12) - 1;
    let c = [
        (v & m) as f32 * FRACTAL - std::f32::consts::FRAC_1_SQRT_2,
        ((v >> 12) & m) as f32 * FRACTAL - std::f32::consts::FRAC_1_SQRT_2,
        ((v >> 24) & m) as f32 * FRACTAL - std::f32::consts::FRAC_1_SQRT_2,
    ];
    let shift = ((v >> 36) & 3) as usize;
    let neg = (v >> 38) & 1 == 1;
    let mut w = (1.0 - c[0] * c[0] - c[1] * c[1] - c[2] * c[2])
        .max(0.0)
        .sqrt();
    if neg {
        w = -w;
    }
    let mut q = [0.0f32; 4];
    let mut ci = 0;
    for (i, slot) in q.iter_mut().enumerate() {
        if i == shift {
            *slot = w;
        } else {
            *slot = c[ci];
            ci += 1;
        }
    }
    q
}

fn read_quat(r: &mut Cursor, q: RotQuant) -> Result<[f32; 4]> {
    match q {
        RotQuant::ThreeComp40 => Ok(quat40(r.bytes(5)?)),
        RotQuant::Uncompressed => Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        other => Err(Error::format(format!(
            "rotation quantization {other:?} not supported yet"
        ))),
    }
}

fn read_knots(r: &mut Cursor) -> Result<(usize, u8, Vec<u8>)> {
    let n = r.u16()? as usize;
    let p = r.u8()?;
    let knots = r.bytes(n + p as usize + 2)?.to_vec();
    Ok((n, p, knots))
}

/// A position or scale channel: `types` has static flags for x, y, z in bits 0..2 and
/// spline flags in bits 4..6; `sixteen` selects 16-bit over 8-bit control points.
fn read_vector(r: &mut Cursor, types: u8, sixteen: bool, default: f32) -> Result<Channel<3>> {
    let spline = [types & 16 != 0, types & 32 != 0, types & 64 != 0];
    let stat = [types & 1 != 0, types & 2 != 0, types & 4 != 0];
    let any_spline = spline.iter().any(|&s| s);
    let knots = if any_spline {
        let k = read_knots(r)?;
        r.align(4);
        Some(k)
    } else {
        None
    };
    let mut value = [default; 3];
    let mut ranges = [(0.0f32, 0.0f32); 3];
    for i in 0..3 {
        if spline[i] {
            ranges[i] = (r.f32()?, r.f32()?);
        } else if stat[i] {
            value[i] = r.f32()?;
        }
    }
    let Some((n, degree, knots)) = knots else {
        return Ok(Channel::constant(value));
    };
    let mut points = Vec::with_capacity(n + 1);
    for _ in 0..=n {
        let mut pt = value;
        for i in 0..3 {
            if spline[i] {
                let (lo, hi) = ranges[i];
                let q = if sixteen {
                    r.u16()? as f32 / 65535.0
                } else {
                    r.u8()? as f32 / 255.0
                };
                pt[i] = lo + (hi - lo) * q;
            }
        }
        points.push(pt);
    }
    r.align(4);
    Ok(Channel {
        degree,
        knots,
        points,
    })
}

fn read_rotation(r: &mut Cursor, types: u8, q: RotQuant) -> Result<Channel<4>> {
    if types & 0xf0 != 0 {
        let (n, degree, knots) = read_knots(r)?;
        if matches!(q, RotQuant::Polar32 | RotQuant::Uncompressed) {
            r.align(4);
        }
        let mut points = Vec::with_capacity(n + 1);
        for _ in 0..=n {
            points.push(read_quat(r, q)?);
        }
        r.align(4);
        Ok(Channel {
            degree,
            knots,
            points,
        })
    } else if types & 0x0f != 0 {
        let v = read_quat(r, q)?;
        r.align(4);
        Ok(Channel::constant(v))
    } else {
        Ok(Channel::constant([0.0, 0.0, 0.0, 1.0]))
    }
}

impl SplineAnimation {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let pf = Packfile::parse(bytes)?;
        let obj = pf
            .objects_of("hkaSplineSkeletalAnimation")
            .next()
            .ok_or_else(|| {
                let kinds: Vec<&str> = pf
                    .objects
                    .iter()
                    .map(|o| o.class.as_str())
                    .filter(|c| c.starts_with("hka") && c.contains("Animation"))
                    .collect();
                Error::format(format!(
                    "no hkaSplineSkeletalAnimation in the packfile (has {kinds:?})"
                ))
            })?;
        let o = obj.offset;
        let duration = pf.f32(o + 12);
        let num_tracks = pf.u32(o + 16) as usize;
        let num_frames = pf.u32(o + 36) as usize;
        let num_blocks = pf.u32(o + 40) as usize;
        let max_frames_per_block = pf.u32(o + 44) as usize;
        let block_duration = pf.f32(o + 52);
        let frame_duration = pf.f32(o + 60);
        let block_offsets: Vec<u32> = match pf.array(o + 64) {
            Some((p, n)) => (0..n).map(|i| pf.u32(p + 4 * i)).collect(),
            None => vec![0],
        };
        let (data_off, data_len) = pf
            .array(o + 112)
            .ok_or_else(|| Error::format("animation without data"))?;
        let data = pf
            .data()
            .get(data_off as usize..(data_off + data_len) as usize)
            .ok_or_else(|| Error::format("animation data out of range"))?;
        // track names from the annotation tracks
        let mut track_names = Vec::with_capacity(num_tracks);
        if let Some((p, n)) = pf.array(o + 28) {
            for i in 0..n {
                let name = pf
                    .pointer(p + 4 * i)
                    .and_then(|t| pf.pointer(t))
                    .map(|s| pf.cstr(s))
                    .unwrap_or_default();
                track_names.push(
                    name.strip_suffix("_AnimationTrack")
                        .unwrap_or(&name)
                        .to_string(),
                );
            }
        }
        track_names.resize(num_tracks, String::new());
        // root motion: +48 duration, +52/+56 referenceFrameSamples
        let mut root_motion = Vec::new();
        if let Some(rf) = pf.pointer(o + 24)
            && let Some((p, n)) = pf.array(rf + 52)
        {
            root_motion = (0..n).map(|i| pf.vec4(p + 16 * i)).collect();
            if root_motion.iter().all(|s| s.iter().all(|v| v.abs() < 1e-6)) {
                root_motion.clear();
            }
        }

        let mut blocks = Vec::with_capacity(num_blocks);
        for b in 0..num_blocks.max(1) {
            let start = block_offsets.get(b).copied().unwrap_or(0) as usize;
            let mut r = Cursor { d: data, o: start };
            let masks: Vec<[u8; 4]> = (0..num_tracks)
                .map(|_| {
                    let m = r.bytes(4)?;
                    Ok([m[0], m[1], m[2], m[3]])
                })
                .collect::<Result<_>>()?;
            r.align(4);
            let mut tracks = Vec::with_capacity(num_tracks);
            for m in &masks {
                let pos16 = m[0] & 3 == 1;
                let rq = RotQuant::from_bits((m[0] >> 2) & 15)?;
                let scale16 = (m[0] >> 6) & 3 == 1;
                let position = read_vector(&mut r, m[1], pos16, 0.0)?;
                let rotation = read_rotation(&mut r, m[2], rq)?;
                let scale = read_vector(&mut r, m[3], scale16, 1.0)?;
                tracks.push(Track {
                    position,
                    rotation,
                    scale,
                });
            }
            blocks.push(tracks);
        }
        Ok(Self {
            duration,
            num_frames,
            frame_duration,
            block_duration,
            max_frames_per_block,
            track_names,
            blocks,
            root_motion,
        })
    }

    /// Ground speed of the root motion (m/s), when the clip travels.
    pub fn root_speed(&self) -> Option<f32> {
        let (first, last) = (self.root_motion.first()?, self.root_motion.last()?);
        if self.duration <= 0.0 {
            return None;
        }
        let d = ((last[0] - first[0]).powi(2) + (last[2] - first[2]).powi(2)).sqrt();
        (d > 0.05).then_some(d / self.duration)
    }

    pub fn track_count(&self) -> usize {
        self.blocks.first().map(|b| b.len()).unwrap_or(0)
    }

    /// The pose at `time` seconds (clamped to the clip), one transform per track.
    pub fn sample(&self, time: f32, out: &mut Vec<QsTransform>) {
        out.clear();
        if self.blocks.is_empty() {
            return;
        }
        let t = time.clamp(0.0, self.duration.max(0.0));
        let block_dur = if self.block_duration > 0.0 {
            self.block_duration
        } else {
            self.duration.max(1e-6)
        };
        let mut b = (t / block_dur).floor() as usize;
        if b >= self.blocks.len() {
            b = self.blocks.len() - 1;
        }
        let frames_in_block = if b + 1 == self.blocks.len() {
            // the last block holds the remaining frames
            self.num_frames
                .saturating_sub(b * self.max_frames_per_block)
                .max(1)
        } else {
            self.max_frames_per_block
        };
        let u = ((t - b as f32 * block_dur) / self.frame_duration.max(1e-6))
            .clamp(0.0, (frames_in_block - 1) as f32);
        for track in &self.blocks[b] {
            let mut q = track.rotation.sample(u);
            let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            if len > 1e-6 {
                for c in &mut q {
                    *c /= len;
                }
            } else {
                q = [0.0, 0.0, 0.0, 1.0];
            }
            out.push(QsTransform {
                translation: track.position.sample(u),
                rotation: q,
                scale: track.scale.sample(u),
            });
        }
    }
}

/// `hkaInterleavedSkeletalAnimation` (type 1): +12 duration, +16 tracks, +28/+32
/// annotation tracks, +36/+40 `transforms` (48-byte hkQsTransforms, frame-major).
/// The canopy's `chute_*.ban` clips are this (its skeleton: `base`, `lft_1`…).
#[derive(Clone, Debug, PartialEq)]
pub struct InterleavedAnimation {
    pub duration: f32,
    pub track_names: Vec<String>,
    pub frames: Vec<Vec<QsTransform>>,
}

impl InterleavedAnimation {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let pf = Packfile::parse(bytes)?;
        let obj = pf
            .objects_of("hkaInterleavedSkeletalAnimation")
            .next()
            .ok_or_else(|| Error::format("no hkaInterleavedSkeletalAnimation"))?;
        let o = obj.offset;
        let duration = pf.f32(o + 12);
        let tracks = pf.u32(o + 16) as usize;
        if tracks == 0 {
            return Err(Error::format("animation without tracks"));
        }
        let mut track_names = Vec::with_capacity(tracks);
        if let Some((p, n)) = pf.array(o + 28) {
            for i in 0..n {
                let name = pf
                    .pointer(p + 4 * i)
                    .and_then(|t| pf.pointer(t))
                    .map(|s| pf.cstr(s))
                    .unwrap_or_default();
                track_names.push(
                    name.strip_suffix("_AnimationTrack")
                        .unwrap_or(&name)
                        .to_string(),
                );
            }
        }
        track_names.resize(tracks, String::new());
        let (p, n) = pf
            .array(o + 36)
            .ok_or_else(|| Error::format("animation without transforms"))?;
        let frames = (0..n as usize / tracks)
            .map(|f| {
                (0..tracks)
                    .map(|t| {
                        let a = p + 48 * (f * tracks + t) as u32;
                        QsTransform {
                            translation: pf.vec3(a),
                            rotation: pf.vec4(a + 16),
                            scale: pf.vec3(a + 32),
                        }
                    })
                    .collect()
            })
            .collect();
        Ok(Self {
            duration,
            track_names,
            frames,
        })
    }

    /// The pose at `time` (nearest frame, clamped).
    pub fn sample(&self, time: f32, out: &mut Vec<QsTransform>) {
        out.clear();
        let n = self.frames.len();
        if n == 0 {
            return;
        }
        let f = if self.duration > 0.0 {
            ((time / self.duration) * (n - 1) as f32).round() as usize
        } else {
            0
        };
        out.extend_from_slice(&self.frames[f.min(n - 1)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `biped.bsk`, `std_idle.ban`, `run_fwd.ban` and
    /// `grpl_hang.ban` in the dropzone (`jc2tool cat <game> biped.bsk`,
    /// `jc2tool cat <game> 'global\animations.blz/std_idle.ban'`, …).
    #[test]
    fn skeleton_and_clips() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        if !fs.contains_any("biped.bsk") || !fs.contains_any("std_idle.ban") {
            return;
        }
        let sk = Skeleton::parse(&fs.read_any("biped.bsk").unwrap()).unwrap();
        assert_eq!(sk.bones.len(), 84);
        assert_eq!(sk.bones[1].name, "Hips");
        assert_eq!(sk.bones[1].parent, Some(0));
        assert!((sk.reference_pose[1].translation[1] - 0.948).abs() < 0.01);
        assert_eq!(sk.bone_index("LeftFoot"), Some(29));

        let idle = SplineAnimation::parse(&fs.read_any("std_idle.ban").unwrap()).unwrap();
        assert_eq!(idle.track_count(), 79);
        assert_eq!(idle.num_frames, 161);
        assert!((idle.duration - 5.3333).abs() < 1e-3);
        assert_eq!(idle.track_names[1], "Hips");
        assert_eq!(idle.track_names[4], "Neck");
        // the static positions equal the reference pose of the named bone
        for (ti, name) in idle.track_names.iter().enumerate() {
            let Some(bi) = sk.bone_index(name) else {
                continue;
            };
            let p = idle.blocks[0][ti].position.points[0];
            let r = sk.reference_pose[bi].translation;
            let d = ((p[0] - r[0]).powi(2) + (p[1] - r[1]).powi(2) + (p[2] - r[2]).powi(2)).sqrt();
            assert!(d < 0.06, "track {ti} {name}: {p:?} vs reference {r:?}");
        }
        // sampled rotations are unit quaternions that move smoothly
        let mut a = Vec::new();
        let mut b = Vec::new();
        idle.sample(0.0, &mut a);
        for i in 1..=50 {
            idle.sample(i as f32 * 0.1, &mut b);
            for (ta, tb) in a.iter().zip(&b) {
                let dot: f32 = ta
                    .rotation
                    .iter()
                    .zip(&tb.rotation)
                    .map(|(x, y)| x * y)
                    .sum();
                assert!(
                    dot.abs() > 0.9,
                    "jump at {i}: {:?} -> {:?}",
                    ta.rotation,
                    tb.rotation
                );
            }
            std::mem::swap(&mut a, &mut b);
        }
        assert!(idle.root_speed().is_none());
        if fs.contains_any("chute_add_idle.ban") {
            let c =
                InterleavedAnimation::parse(&fs.read_any("chute_add_idle.ban").unwrap()).unwrap();
            assert_eq!(c.track_names[1], "base");
            assert_eq!(c.frames.len(), 41);
            assert_eq!(c.frames[0].len(), 16);
        }
        for name in ["run_fwd.ban", "grpl_hang.ban", "fall.ban"] {
            if !fs.contains_any(name) {
                continue;
            }
            let clip = SplineAnimation::parse(&fs.read_any(name).unwrap()).unwrap();
            if name == "run_fwd.ban" {
                let v = clip.root_speed().expect("the run travels");
                assert!((v - 3.333).abs() < 0.05, "run at {v} m/s");
            }
            // clips without the face carry fewer tracks
            assert!(clip.track_count() >= 60, "{name}: {}", clip.track_count());
            clip.sample(clip.duration / 2.0, &mut a);
            assert!(
                a[1].translation[1] > 0.3,
                "{name}: hips at {:?}",
                a[1].translation
            );
        }
    }
}
