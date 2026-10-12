//! Lossless effect graphs and the evidenced subset of compiled particle systems.
//! Unknown hashes, routing strings, modifier order/windows and child links remain
//! in the source tree. This decoder does not execute the effect graph.
use crate::property::{Node, PropertyFile, Variant};
use crate::{Error, GameFs, Result};
use serde::Serialize;

pub const FIXTURE_EFFECT: &str = "lave_mc_damage_smoke.bin";
pub const FIXTURE_LEAF: &str = "fx_vehicle_mc_smoke_01";

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EffectComponent {
    pub path: Vec<u32>,
    pub kind: i32,
    pub id: Option<i32>,
    pub particle_names: Vec<String>,
    pub raw: Node,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EffectGraph {
    pub raw: PropertyFile,
    pub components: Vec<EffectComponent>,
}
impl EffectGraph {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let raw = PropertyFile::parse_any(bytes)?;
        let mut components = Vec::new();
        fn visit(node: &Node, path: &mut Vec<u32>, out: &mut Vec<EffectComponent>) {
            if let Some(Variant::Int(kind)) = node.value_hash(0x2f6488d6) {
                fn names(n: &Node, out: &mut Vec<String>) {
                    if let Some(Variant::Str(s)) = n.value_hash(0xd31ab684) {
                        out.push(s.clone());
                    }
                    for (_, c) in &n.children {
                        names(c, out);
                    }
                }
                let mut particle_names = Vec::new();
                if *kind == 1 {
                    names(node, &mut particle_names);
                }
                out.push(EffectComponent {
                    path: path.clone(),
                    kind: *kind,
                    id: match node.value_hash(0x6a9dfd76) {
                        Some(Variant::Int(i)) => Some(*i),
                        _ => None,
                    },
                    particle_names,
                    raw: node.clone(),
                });
            }
            for (key, child) in &node.children {
                path.push(key.hash());
                visit(child, path, out);
                path.pop();
            }
        }
        for (i, n) in raw.nodes.iter().enumerate() {
            visit(n, &mut vec![i as u32], &mut components);
        }
        Ok(Self { raw, components })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Modifier {
    pub key: u32,
    pub class: String,
    /// Native comparisons use updated normalized particle age, with a bypass
    /// when start<=0 and end>=1 (FUN_00a62980 / class-specific consumers).
    pub start: f32,
    pub end: f32,
    pub kind: ModifierKind,
    pub raw: Node,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum ModifierKind {
    Animation {
        lower: u32,
        upper: u32,
        rate: f32,
        looping: bool,
    },
    Size(f32),
    Opacity(f32),
    Force([f32; 3]),
    Damping(f32),
    Unsupported,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RenderFlags {
    pub additive_hint: bool,
    pub bit6: bool,
    pub distortion: bool,
    /// Flag bit12 `#a1028df6`: the quad's long axis follows the screen velocity (doc
    /// `particle-units-next.md` section 6, native `0xa5bc30`).
    pub align_velocity: bool,
}
/// The emitter fields of a leaf (`0xa5a2d0` / `0xa5a4e0`, doc `particle-units-next.md` section 3).
/// A leaf without the shape field `#89c42bb5` never reads its emitter node, so it keeps the
/// constructor defaults (`0xa54830`): 30 births per second, no angles, speed 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Emission {
    /// Births per second.
    pub rate: f32,
    /// Emitter shape (`#89c42bb5`): 0 point, 3 box, 4 radial disc, 5 sphere shell.
    pub shape: Option<i32>,
    /// `[base, variation]` in radians: `ax = signed * variation + base`.
    pub pitch: [f32; 2],
    pub yaw: [f32; 2],
    /// `[base, variation]` in m/s, clamped to `[-128, 127]` after the draw.
    pub speed: [f32; 2],
    /// Birth offset (`#5c50f180`, `offset_y`, `#de1afd07`).
    pub offset: [f32; 3],
    /// Shape 3: box extents (`#11b73cc0`, `#d2cbd704`, `#2a2907f2`); shapes 4/5: inner radius,
    /// outer radius, height (`#c2826ce3`, `#7c01072c`, `#d2cbd704`). Defaults 0, 10, 10.
    pub shape_params: [f32; 3],
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParticleSystem {
    pub raw: PropertyFile,
    pub name: String,
    pub texture: String,
    pub lifetime: f32,
    pub system_lifetime: f32,
    pub emission_rate: f32,
    /// Raw `[velocity, variation]` of the emitter node (see [`Emission`] for the effective values).
    pub emission_speed: [f32; 2],
    pub emission: Emission,
    /// Initial particle colour `(#99030548, #b3c17b1d, #25e4e2ca)` (default 1).
    pub colour: [f32; 3],
    /// Velocity stretch `(#b8fd2b9e base, #b294c343 max, #4f2b9eee per m/s)`, defaults `(1, 1, 0)`.
    pub stretch: [f32; 3],
    /// Maximum draw / emit distance `#d1e88637` (default 800 m).
    pub max_distance: f32,
    /// Near fade `(#c9a96718 start, #5af3683a length)` in metres; both 0 = none.
    pub near_fade: [f32; 2],
    pub emitter: Node,
    /// Native loader +168/+16a, used as inverse UV dimensions by FUN_00a5b430.
    pub atlas: [u16; 2],
    pub child_link: Option<String>,
    /// Every named follow-on system of this leaf: `linked_system_N` for N = 0, 1, 2, ... until the
    /// first missing one (native `0xa71720`; 12 for `fx_exp_grenade_01`, up to 33 in retail). All
    /// of them start with the parent (`0xa6d290`).
    pub children: Vec<String>,
    /// Per-particle trail system `#1de3fca7` (native leaf+0xc, flag bit 14; 273 retail leaves,
    /// `0xa6e2f0`): every particle born gets its own emitter of this system that follows it. The
    /// key's spelling is unresolved (the hash is proven).
    pub trail: Option<String>,
    /// The leaf names `mesh_N` / `physics_N` debris slots (`0xa71720`): the capacity is then at most
    /// 150.
    pub debris_slots: bool,
    pub initial_alpha: f32,
    /// Initial emissive `#24683665` (native root `+0xce`, a clamped `ushort / 65535`, default 0: `0xa62710`
    /// zero-stores it; loader `0xa71720`), copied to the particle at birth
    /// (`0xa5a4e0`: `ushort(+0xbe) * 1.5259022e-05`) and uploaded as `cb1[3k+2].z`.
    pub initial_emissive: f32,
    /// Native birth size = base + uniform01 * range, not symmetric variation.
    pub initial_size: [f32; 2],
    pub initial_frame: [f32; 2],
    pub space: i32,
    pub lifetime_variation: f32,
    /// DAT_00f1217c -> root+76/worker+66, signed-short capacity at birth.
    pub max_particles: i16,
    /// Lossless diagnostic inventory (property-child order).
    pub modifiers: Vec<Modifier>,
    /// Native indexed effect_N order, or visual_effect_N then physical_effect_N
    /// only when effect_0 is absent. Raw unmatched children remain in `raw`.
    pub execution_order: Vec<u32>,
}
/// Key `#1de3fca7`: the per-particle trail system name (leaf+0xc).
pub const TRAIL_SYSTEM_HASH: u32 = 0x1de3fca7;
/// Key `#d1e88637`: the birth cull distance in metres (leaf+0x160, default 800).
pub const CULL_DISTANCE_HASH: u32 = 0xd1e88637;
/// Variation key `#b0124c0a` of an effect's particle component: the flags byte whose bit 0 lets
/// the emitter release itself (`0x005b9960`, default 1).
pub const RELEASE_FLAG_HASH: u32 = 0xb0124c0a;

/// Hash of the property `linked_system_N` (`0xa71720` builds the name with `sprintf`; N = 0 is
/// `#841408bf`, 1 is `#f47c95e6`, ...).
pub fn linked_system_hash(n: usize) -> u32 {
    crate::hash::hash_str(&format!("linked_system_{n}"))
}
fn number_or(n: &Node, hash: u32, default: f32) -> f32 {
    match n.value_hash(hash) {
        Some(Variant::Float(f)) if f.is_finite() => *f,
        _ => default,
    }
}
fn integer(n: &Node, hash: u32) -> Result<i32> {
    match n.value_hash(hash) {
        Some(Variant::Int(v)) => Ok(*v),
        _ => Err(Error::format(format!("particle #{hash:08x}: expected int"))),
    }
}
fn number(n: &Node, hash: u32) -> Result<f32> {
    match n.value_hash(hash) {
        Some(Variant::Float(f)) if f.is_finite() => Ok(*f),
        _ => Err(Error::format(format!(
            "particle #{hash:08x}: expected finite float"
        ))),
    }
}
fn string(n: &Node, hash: u32) -> Result<String> {
    match n.value_hash(hash) {
        Some(Variant::Str(s)) => Ok(s.clone()),
        _ => Err(Error::format(format!(
            "particle #{hash:08x}: expected string"
        ))),
    }
}
impl ParticleSystem {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let raw = PropertyFile::parse_any(bytes)?;
        if raw.nodes.len() != 1 {
            return Err(Error::format("particle requires one root"));
        }
        let n = &raw.nodes[0];
        let mut atlas = [0; 2];
        for (i, hash) in [0xb0a2484f, 0xe751ee6f].into_iter().enumerate() {
            atlas[i] = match n.value_hash(hash) {
                Some(Variant::Int(v)) if *v > 0 && *v <= 256 => *v as u16,
                _ => return Err(Error::format("particle atlas dimension outside 1..256")),
            };
        }
        let emitters: Vec<_> = n
            .children
            .iter()
            .filter(|(k, _)| k.hash() == 0xec21636e)
            .collect();
        if emitters.len() != 1 {
            return Err(Error::format("particle requires one emission node"));
        }
        let emitter = emitters[0].1.clone();
        let emission_rate = number(&emitter, 0x487f6fc3)?;
        if emission_rate < 0.0 {
            return Err(Error::format("negative particle emission rate"));
        }
        let lifetime = number(n, 0xbc4e9cf9)?;
        let system_lifetime = number(n, 0x79c213bb)?;
        // The base lifetime may be non-positive (30 retail leaves): the per-particle value is
        // clamped to [0.001, 10000] after the variation draw (`0xa5ca60`).
        if system_lifetime < 0.0 {
            return Err(Error::format("invalid particle system lifetime"));
        }
        let mut modifiers = Vec::new();
        for (key, child) in &n.children {
            if child.value_hash(0x1473b179).is_some() {
                let class = string(child, 0x1473b179)?;
                let kind = match class.as_str() {
                    "CParticleVisualAnimation" => ModifierKind::Animation {
                        lower: integer(child, 0xc871c24e)? as u32,
                        upper: integer(child, 0x9fba43a8)? as u32,
                        rate: number(child, 0xdb53dd7d)?,
                        looping: integer(child, 0x9ea55007)? != 0,
                    },
                    // DAT_00f12040 -> modifier+0; FUN_00a545e0 adds dt*coefficient.
                    "CParticleVisualSize" => ModifierKind::Size(number(child, 0xe3b6dc5d)?),
                    "CParticleVisualOpacity" => ModifierKind::Opacity(number(child, 0x9a4a5864)?),
                    "CParticlePhysicalForce" => ModifierKind::Force([
                        number(child, 0x0170c42f)?,
                        number(child, 0x6ba5c04f)?,
                        number(child, 0xfd539599)?,
                    ]),
                    "CParticlePhysicalDamping" => ModifierKind::Damping(number(child, 0xe14c7165)?),
                    _ => ModifierKind::Unsupported,
                };
                modifiers.push(Modifier {
                    key: key.hash(),
                    class,
                    start: number(child, 0xf40793e7)?,
                    end: number(child, 0x590f095f)?,
                    kind,
                    raw: child.clone(),
                });
            }
        }
        let mut execution_order = Vec::new();
        let mut family = |prefix: &str| -> Result<()> {
            for index in 0..32 {
                if execution_order.len() == 32 {
                    break;
                }
                let hash = crate::hash::hash_str(&format!("{prefix}{index}"));
                let matches: Vec<_> = n
                    .children
                    .iter()
                    .filter(|(key, _)| key.hash() == hash)
                    .collect();
                if matches.is_empty() {
                    break;
                }
                if matches.len() != 1 || !modifiers.iter().any(|m| m.key == hash) {
                    return Err(Error::format(
                        "ambiguous or missing indexed particle modifier class",
                    ));
                }
                execution_order.push(hash);
            }
            Ok(())
        };
        family("effect_")?;
        if n.child("effect_0").is_none() {
            family("visual_effect_")?;
            family("physical_effect_")?;
        }
        let frame = [number(n, 0xa5318e57)?, number(n, 0x97569fc5)?];
        if frame.iter().any(|v| !(-32768.0..=32767.0).contains(v)) {
            return Err(Error::format(
                "particle initial frame outside native signed-short range",
            ));
        }
        let initial_alpha = number(n, 0x4885935d)?.clamp(0.0, 1.0);
        let initial_emissive = number_or(n, 0x24683665, 0.0).clamp(0.0, 1.0);
        let max_particles = integer(n, 0x74175c29)?;
        if !(0..=32767).contains(&max_particles) {
            return Err(Error::format(
                "particle capacity outside native signed-short range",
            ));
        }
        let shape = match n.value_hash(0x89c42bb5) {
            Some(Variant::Int(v)) => Some(*v),
            _ => None,
        };
        let emission = if shape.is_some() {
            let box_shape = shape == Some(3);
            let p = |h| number_or(&emitter, h, 0.0);
            Emission {
                rate: emission_rate,
                shape,
                pitch: [p(0x8391b740), p(0xbedbc04d)],
                yaw: [p(0xdf6a7f80), p(0x815d0180)],
                speed: [p(0xe4021a11), p(0x38d18085)],
                offset: [p(0x5c50f180), p(0x90e04ae5), p(0xde1afd07)],
                shape_params: if box_shape {
                    [
                        number_or(&emitter, 0x11b73cc0, 0.0),
                        number_or(&emitter, 0xd2cbd704, 10.0),
                        number_or(&emitter, 0x2a2907f2, 10.0),
                    ]
                } else {
                    [
                        number_or(&emitter, 0xc2826ce3, 0.0),
                        number_or(&emitter, 0x7c01072c, 10.0),
                        number_or(&emitter, 0xd2cbd704, 10.0),
                    ]
                },
            }
        } else {
            Emission {
                rate: 30.0,
                shape: None,
                pitch: [0.0; 2],
                yaw: [0.0; 2],
                speed: [0.0; 2],
                offset: [0.0; 3],
                shape_params: [0.0, 10.0, 10.0],
            }
        };
        let mut children = Vec::new();
        for index in 0..256 {
            match n.value_hash(linked_system_hash(index)) {
                Some(Variant::Str(s)) => children.push(s.clone()),
                _ => break,
            }
        }
        let trail = match n.value_hash(TRAIL_SYSTEM_HASH) {
            Some(Variant::Str(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        // retail slots are numbered from 2 (`mesh_2` .. `mesh_8`, `physics_2` .. `physics_8`)
        let debris_slots = (0..40).any(|i| {
            ["mesh_", "physics_"]
                .iter()
                .any(|p| n.value_hash(crate::hash::hash_str(&format!("{p}{i}"))).is_some())
        });
        Ok(Self {
            trail,
            debris_slots,
            name: string(n, 0xd31ab684)?,
            texture: string(n, 0xfbdb7d47)?,
            child_link: match n.value_hash(0x841408bf) {
                Some(Variant::Str(s)) => Some(s.clone()),
                _ => None,
            },
            children,
            atlas,
            lifetime,
            system_lifetime,
            emission_rate,
            emission_speed: [number(&emitter, 0xe4021a11)?, number(&emitter, 0x38d18085)?],
            emission,
            colour: [
                number_or(n, 0x99030548, 1.0),
                number_or(n, 0xb3c17b1d, 1.0),
                number_or(n, 0x25e4e2ca, 1.0),
            ],
            stretch: [
                number_or(n, 0xb8fd2b9e, 1.0),
                number_or(n, 0xb294c343, 1.0),
                number_or(n, 0x4f2b9eee, 0.0),
            ],
            max_distance: number_or(n, CULL_DISTANCE_HASH, 800.0),
            near_fade: [number_or(n, 0xc9a96718, 0.0), number_or(n, 0x5af3683a, 0.0)],
            emitter,
            modifiers,
            execution_order,
            // Native root+cc quantization, consumed as ushort/65535 at birth.
            initial_alpha: (initial_alpha * 65535.0) as u16 as f32 / 65535.0,
            initial_emissive: (initial_emissive * 65535.0) as u16 as f32 * 1.525_902_2e-5,
            initial_size: [number(n, 0xef05f293)?, number(n, 0xadeb7b1a)?],
            initial_frame: frame.map(|v| (v as i32) as i16 as f32),
            space: integer(n, 0x562663c7)?,
            lifetime_variation: number(n, 0x7535093b)?,
            max_particles: max_particles as i16,
            raw,
        })
    }
    /// Root render flag ints. `distortion` (#71feec86, native render flags bit13) is set on all
    /// 129 leaves that use a normal-map texture (`distort_normal`, `spherify_*normal`) and on no
    /// other. `additive_hint` (#27230869, bit7) is **inferred**: it is set on every leaf of the
    /// glow textures (`flare`, `muzzleflashes`, `spark_dif`, `lightningbolt`, `gradient`) and on
    /// 80 of 117 fire leaves; the native blend routine is not traced for this bit.
    pub fn render_flags(&self) -> RenderFlags {
        let n = &self.raw.nodes[0];
        let g = |h| matches!(n.value_hash(h), Some(Variant::Int(v)) if *v != 0);
        RenderFlags {
            additive_hint: g(0x27230869),
            bit6: g(0x59d9aeb9),
            distortion: g(0x71feec86),
            align_velocity: g(0xa1028df6),
        }
    }
    pub fn execution_modifiers(&self) -> impl Iterator<Item = &Modifier> {
        self.execution_order
            .iter()
            .filter_map(|key| self.modifiers.iter().find(|m| m.key == *key))
    }
    pub fn playback_cell(&self, frame: f32) -> Result<u32> {
        let cell = frame.round_ties_even();
        if !cell.is_finite()
            || cell < 0.0
            || cell >= f32::from(self.atlas[0]) * f32::from(self.atlas[1])
        {
            return Err(Error::format("particle playback cell outside atlas"));
        }
        Ok(cell as u32)
    }
    /// Row-major, top-origin UV convention is a viewer policy. Playback and native
    /// frame orientation are not inferred from the atlas dimensions.
    pub fn preview_uv(&self, frame: u32) -> Result<[[f32; 2]; 4]> {
        let [cols, rows] = self.atlas.map(u32::from);
        if frame >= cols * rows {
            return Err(Error::format("particle atlas frame out of bounds"));
        }
        let x = (frame % cols) as f32 / cols as f32;
        let y = (frame / cols) as f32 / rows as f32;
        let dx = 1.0 / cols as f32;
        let dy = 1.0 / rows as f32;
        Ok([[x + dx, y], [x, y], [x, y + dy], [x + dx, y + dy]])
    }
}

#[derive(Clone, Debug)]
pub struct ParticleFixture {
    pub effect: EffectGraph,
    pub system: ParticleSystem,
    pub texture_member: String,
    pub texture_bytes: Vec<u8>,
}
impl ParticleFixture {
    /// Deliberately bounded to the researched MC graph/leaf. No graph routing,
    /// sounds, decals or recursive child emission is executed by this fixture.
    pub fn load(fs: &GameFs, effect_name: &str, leaf: &str) -> Result<Self> {
        if effect_name != FIXTURE_EFFECT || leaf != FIXTURE_LEAF {
            return Err(Error::format(
                "only the researched MC smoke fixture is supported",
            ));
        }
        let effect = EffectGraph::parse(&fs.read_inflated(effect_name)?)?;
        if effect
            .components
            .iter()
            .filter(|c| c.kind == 1)
            .flat_map(|c| &c.particle_names)
            .filter(|s| s.as_str() == leaf)
            .count()
            != 1
        {
            return Err(Error::format(
                "effect does not uniquely reference requested particle leaf",
            ));
        }
        let blob = fs.read_inflated("particles.blz")?;
        let archive = crate::sarc::SmallArchive::parse(&blob)?;
        let member = format!("{leaf}.psmb");
        let entry = archive
            .get(&member)
            .ok_or_else(|| Error::NotFound(member.clone()))?;
        let system = ParticleSystem::parse(archive.data(&blob, entry)?)?;
        if system.name != format!("{leaf}.psm") {
            return Err(Error::format("particle leaf name mismatch"));
        }
        let texture_member = system
            .texture
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .to_string();
        let entry = archive
            .get(&texture_member)
            .ok_or_else(|| Error::NotFound(texture_member.clone()))?;
        let texture_bytes = archive.data(&blob, entry)?.to_vec();
        let dds = crate::dds::DdsImage::parse(&texture_bytes)?;
        if !dds.width.is_multiple_of(u32::from(system.atlas[0]))
            || !dds.height.is_multiple_of(u32::from(system.atlas[1]))
        {
            return Err(Error::format("texture does not divide into particle atlas"));
        }
        Ok(Self {
            effect,
            system,
            texture_member,
            texture_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_mc_smoke_chain_and_lossless_fields() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP original particle fixture: JC2_TEST_DIR absent");
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let f = ParticleFixture::load(&fs, FIXTURE_EFFECT, FIXTURE_LEAF).unwrap();
        assert_eq!(
            f.effect
                .components
                .iter()
                .map(|c| (c.kind, c.id))
                .collect::<Vec<_>>(),
            [(1, Some(1)), (0, Some(0))]
        );
        assert_eq!(f.system.lifetime, 1.0);
        assert_eq!(f.system.system_lifetime, 0.0);
        assert_eq!(f.system.emission_rate, 25.0);
        assert_eq!(f.system.emission_speed, [0.4, 0.4]);
        assert_eq!(f.system.atlas, [8, 8]);
        assert_eq!(f.system.space, 2);
        assert_eq!(f.system.initial_alpha, 0.0);
        assert_eq!(f.system.initial_size, [0.08, 0.04]);
        assert_eq!(f.system.initial_frame, [30.0, 10.0]);
        assert_eq!(f.system.max_particles, 25);
        assert_eq!(
            f.system
                .execution_modifiers()
                .map(|m| m.kind)
                .collect::<Vec<_>>(),
            [
                ModifierKind::Animation {
                    lower: 30,
                    upper: 50,
                    rate: 30.0,
                    looping: false
                },
                ModifierKind::Size(0.2),
                ModifierKind::Opacity(4.0),
                ModifierKind::Opacity(-1.168),
                ModifierKind::Force([0.0, 1.0, 0.0]),
                ModifierKind::Damping(5.0),
            ]
        );
        assert_eq!(f.texture_member, "fumefx_bomb_01.dds");
        assert_eq!(
            f.system.child_link.as_deref(),
            Some("fx_vehicle_mc_smoke_02")
        );
        assert_eq!(
            f.system
                .modifiers
                .iter()
                .map(|m| m.class.as_str())
                .collect::<Vec<_>>(),
            [
                "CParticlePhysicalForce",
                "CParticleVisualAnimation",
                "CParticlePhysicalDamping",
                "CParticleVisualOpacity",
                "CParticleVisualOpacity",
                "CParticleVisualSize"
            ]
        );
        let opacity = &f.system.modifiers[3];
        assert!((opacity.start - 0.4579).abs() < 1e-5);
        assert_eq!(
            opacity.raw.value_hash(0x9a4a5864),
            Some(&Variant::Float(-1.168))
        );
        assert_eq!(
            PropertyFile::parse(&f.system.raw.to_bytes()).unwrap(),
            f.system.raw
        );
        assert_eq!(
            PropertyFile::parse(&f.effect.raw.to_bytes()).unwrap(),
            f.effect.raw
        );
        let dds = crate::dds::DdsImage::parse(&f.texture_bytes).unwrap();
        assert_eq!((dds.width, dds.height), (1024, 1024));
        assert_eq!(
            f.system.preview_uv(30).unwrap(),
            [[0.875, 0.375], [0.75, 0.375], [0.75, 0.5], [0.875, 0.5]]
        );
        assert!(f.system.preview_uv(64).is_err());
        assert!(ParticleFixture::load(&fs, "lave_damage_smoke.bin", FIXTURE_LEAF).is_err());
        let mut corrupt = f.system.raw.clone();
        let e = corrupt.nodes[0]
            .children
            .iter_mut()
            .find(|(k, _)| k.hash() == 0xec21636e)
            .unwrap();
        let rate =
            e.1.values
                .iter_mut()
                .find(|(k, _)| k.hash() == 0x487f6fc3)
                .unwrap();
        rate.1 = Variant::Float(f32::NAN);
        assert!(ParticleSystem::parse(&corrupt.to_bytes()).is_err());
    }
    #[test]
    fn linked_system_names_enumerate_until_the_first_gap() {
        // the first twelve hashes of doc particle-units-next.md section 5
        let known = [
            0x841408bf_u32, 0xf47c95e6, 0x02bcc1f0, 0x00e0c94d, 0x0f6944c8, 0x92010226,
            0x84401592, 0x5de2258b, 0xee3aea16, 0xe32a9c36, 0xb1334086, 0xb71ac6b5,
        ];
        for (n, h) in known.iter().enumerate() {
            assert_eq!(linked_system_hash(n), *h, "linked_system_{n}");
        }
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP linked systems: JC2_TEST_DIR absent");
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let blob = fs.read_inflated("particles.blz").unwrap();
        let a = crate::sarc::SmallArchive::parse(&blob).unwrap();
        let get = |name: &str| {
            let e = a.get(&format!("{name}.psmb")).unwrap();
            ParticleSystem::parse(a.data(&blob, e).unwrap()).unwrap()
        };
        let grenade = get("fx_exp_grenade_01");
        assert_eq!(grenade.children.len(), 12);
        assert_eq!(grenade.children[11], "fx_exp_grenade_13");
        assert_eq!(grenade.children[0], "fx_exp_grenade_02");
        let (mut most, mut shapeless) = (0, 0);
        for e in a.entries.iter().filter(|e| e.name.ends_with(".psmb")) {
            if let Ok(s) = ParticleSystem::parse(a.data(&blob, e).unwrap()) {
                most = most.max(s.children.len());
                shapeless += usize::from(s.emission.shape.is_none());
            }
        }
        eprintln!("most linked systems {most}, shapeless leaves {shapeless}");
        assert_eq!(most, 33);
        // a leaf with a non-positive base lifetime now parses
        let trail = get("fx_fire_car_trail_05");
        assert!(trail.lifetime <= 0.0);
    }
    #[test]
    fn every_retail_particle_system_parses() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            eprintln!("SKIP retail particle systems: JC2_TEST_DIR absent");
            return;
        };
        let fs = GameFs::open(dir).unwrap();
        let blob = fs.read_inflated("particles.blz").unwrap();
        let a = crate::sarc::SmallArchive::parse(&blob).unwrap();
        let (mut ok, mut bad) = (0, Vec::new());
        let mut distortion = 0;
        for e in a.entries.iter().filter(|e| e.name.ends_with(".psmb")) {
            match ParticleSystem::parse(a.data(&blob, e).unwrap()) {
                Ok(s) => {
                    ok += 1;
                    let normal_map = s.texture.contains("normal");
                    assert_eq!(s.render_flags().distortion, normal_map, "{}", e.name);
                    distortion += usize::from(normal_map);
                }
                Err(err) => bad.push(format!("{}: {err}", e.name)),
            }
        }
        eprintln!("particle systems parsed {ok}, failed {}", bad.len());
        for b in bad.iter().take(15) {
            eprintln!("  {b}");
        }
        // 9 retail leaves are not parseable (no texture string); the 30 leaves with a non-positive
        // base lifetime that have one parse since dc95ceb0 (the per-particle lifetime is clamped at
        // birth).
        assert_eq!((ok, bad.len()), (2591, 9));
        assert_eq!(distortion, 129);
    }
}
