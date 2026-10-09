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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ParticleSystem {
    pub raw: PropertyFile,
    pub name: String,
    pub texture: String,
    pub lifetime: f32,
    pub system_lifetime: f32,
    pub emission_rate: f32,
    /// Shape/direction consumers are not yet implemented.
    pub emission_speed: [f32; 2],
    pub emitter: Node,
    /// Native loader +168/+16a, used as inverse UV dimensions by FUN_00a5b430.
    pub atlas: [u16; 2],
    pub child_link: Option<String>,
    pub initial_alpha: f32,
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
        if lifetime <= 0.0 || system_lifetime < 0.0 {
            return Err(Error::format("invalid particle lifetime"));
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
        let max_particles = integer(n, 0x74175c29)?;
        if !(0..=32767).contains(&max_particles) {
            return Err(Error::format(
                "particle capacity outside native signed-short range",
            ));
        }
        Ok(Self {
            name: string(n, 0xd31ab684)?,
            texture: string(n, 0xfbdb7d47)?,
            child_link: match n.value_hash(0x841408bf) {
                Some(Variant::Str(s)) => Some(s.clone()),
                _ => None,
            },
            atlas,
            lifetime,
            system_lifetime,
            emission_rate,
            emission_speed: [number(&emitter, 0xe4021a11)?, number(&emitter, 0x38d18085)?],
            emitter,
            modifiers,
            execution_order,
            // Native root+cc quantization, consumed as ushort/65535 at birth.
            initial_alpha: (initial_alpha * 65535.0) as u16 as f32 / 65535.0,
            initial_size: [number(n, 0xef05f293)?, number(n, 0xadeb7b1a)?],
            initial_frame: frame.map(|v| (v as i32) as i16 as f32),
            space: integer(n, 0x562663c7)?,
            lifetime_variation: number(n, 0x7535093b)?,
            max_particles: max_particles as i16,
            raw,
        })
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
}
