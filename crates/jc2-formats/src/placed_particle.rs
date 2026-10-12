//! Placed world particle objects (`CParticleObject`): chimneys, damaged-building smoke, steam,
//! camp fires. Decoding only; the runtime lives in the game crate (`world_particles.rs`).
//! Research: `research/formats/smoke-effects.md` section 7 (loader 0x00649f10, constructor
//! 0x00796040, census 6916 objects in 1682 containers).
//!
//! Labels: **native** = read from the executable loader; **inferred** = said so.

use crate::placement::{Transform3, node_transform};
use crate::property::{Node, PropertyFile, Variant};

/// Particle leaf name (native `DAT_00eb95c8`).
const KEY_NAME: u32 = 0xbf03_69b0;
/// Byte at +0xa8, constructor default 1 (native); named "autostart" by inference.
const KEY_AUTOSTART: u32 = 0x122b_5b13;
/// Byte at +0xa9, 0 on every retail object (native); when set, SetMatrix keeps only the translation.
const KEY_TRANSLATION_ONLY: u32 = 0x75d3_45ca;
/// Event strings registered into the object's event slots +0x9c / +0xa0 / +0xa4 (native).
const KEY_START: u32 = 0xd2ab_39ea;
const KEY_STOP: u32 = 0x9042_73ab;
const KEY_THIRD: u32 = 0xd3e8_3257;

/// One placed `CParticleObject`.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedParticle {
    /// The leaf with directory and `.psm` stripped, lower case (native: stripped and hashed to a
    /// resource id by 0x00649f10).
    pub leaf: String,
    /// Byte +0xa8 (default 1).
    pub autostart: bool,
    /// Byte +0xa9 (default 0).
    pub translation_only: bool,
    pub start_event: Option<String>,
    pub stop_event: Option<String>,
    /// Role open (smoke-effects.md Gap 1); 345 retail objects set it.
    pub third_event: Option<String>,
    /// Absolute transform (engine axes), parents composed.
    pub world: Transform3,
}

impl PlacedParticle {
    /// The matrix the emitter follows: the translation alone when byte +0xa9 is set (native
    /// SetMatrix 0x008466e0).
    pub fn matrix(&self) -> Transform3 {
        if self.translation_only {
            let t = self.world.translation();
            Transform3::from_mat4x3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, t[0], t[1], t[2]])
        } else {
            self.world
        }
    }
}

/// The leaf name the native loader derives from the authored string: directory and extension
/// stripped (the resource id is the hash of what remains).
pub fn leaf_name(authored: &str) -> String {
    let base = authored.rsplit(['\\', '/']).next().unwrap_or(authored);
    let base = base.rsplit_once('.').map_or(base, |(stem, _)| stem);
    base.to_ascii_lowercase()
}

fn string(n: &Node, hash: u32) -> Option<String> {
    match n.value_hash(hash)? {
        Variant::Str(s) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn flag(n: &Node, hash: u32, default: bool) -> bool {
    match n.value_hash(hash) {
        Some(Variant::Int(i)) => *i != 0,
        _ => default,
    }
}

fn walk(n: &Node, parent: &Transform3, out: &mut Vec<PlacedParticle>) {
    let mut here = *parent;
    let class = n.str("_class");
    if class.is_some()
        && let Some(local) = node_transform(n)
    {
        here = local.then(parent);
    }
    if class == Some("CParticleObject")
        && let Some(name) = string(n, KEY_NAME)
    {
        out.push(PlacedParticle {
            leaf: leaf_name(&name),
            autostart: flag(n, KEY_AUTOSTART, true),
            translation_only: flag(n, KEY_TRANSLATION_ONLY, false),
            start_event: string(n, KEY_START),
            stop_event: string(n, KEY_STOP),
            third_event: string(n, KEY_THIRD),
            world: here,
        });
    }
    for (_, c) in &n.children {
        walk(c, &here, out);
    }
}

/// Every `CParticleObject` of a placement file (depth-first, transforms composed).
pub fn placed_particles(file: &PropertyFile) -> Vec<PlacedParticle> {
    let mut out = Vec::new();
    for root in &file.nodes {
        walk(root, &Transform3::IDENTITY, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_str;
    use crate::property::Key;

    fn node(pairs: Vec<(u32, Variant)>) -> Node {
        let mut n = Node::default();
        n.values.push((Key::Name("_class".into()), Variant::Str("CParticleObject".into())));
        n.values.push((
            Key::Name("world".into()),
            Variant::Mat4x3([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 5.0, 6.0, 7.0]),
        ));
        for (k, v) in pairs {
            n.values.push((Key::Hash(k), v));
        }
        n
    }

    #[test]
    fn key_hashes() {
        // the matrix key of the native loader (DAT_00eb95c4) is the ordinary `world`
        assert_eq!(hash_str("world"), 0x6ca6_d4b9);
    }

    #[test]
    fn leaf_names() {
        assert_eq!(leaf_name("fx\\Env\\FX_Env_Chimney_Large_01.psm"), "fx_env_chimney_large_01");
        assert_eq!(leaf_name("fx_km03_smoke_medium_01"), "fx_km03_smoke_medium_01");
    }

    #[test]
    fn defaults_and_events() {
        let n = node(vec![(KEY_NAME, Variant::Str("a\\B.psm".into()))]);
        let p = &placed_particles(&PropertyFile { nodes: vec![n] })[0];
        assert!(p.autostart && !p.translation_only);
        assert_eq!((p.leaf.as_str(), p.start_event.as_deref()), ("b", None));
        assert_eq!(p.world.translation(), [5.0, 6.0, 7.0]);
        let n = node(vec![
            (KEY_NAME, Variant::Str("x".into())),
            (KEY_AUTOSTART, Variant::Int(0)),
            (KEY_START, Variant::Str("t{a({G})}::destroyed".into())),
            (KEY_STOP, Variant::Str("stop".into())),
        ]);
        let p = &placed_particles(&PropertyFile { nodes: vec![n] })[0];
        assert!(!p.autostart);
        assert_eq!(p.start_event.as_deref(), Some("t{a({G})}::destroyed"));
        assert_eq!(p.stop_event.as_deref(), Some("stop"));
    }

    fn blo_files(container: &str) -> Option<Vec<PropertyFile>> {
        let dir = std::env::var("JC2_TEST_DIR").ok()?;
        let fs = crate::GameFs::open(dir).ok()?;
        let data = fs.read_inflated(container).ok()?;
        let sarc = crate::sarc::SmallArchive::parse(&data).ok()?;
        let mut out = Vec::new();
        for e in sarc.entries.iter().filter(|e| e.name.ends_with(".blo")) {
            let bytes = crate::compress::maybe_inflate(sarc.data(&data, e).ok()?).ok()?;
            out.push(PropertyFile::parse_any(&bytes).ok()?);
        }
        Some(out)
    }

    /// Needs JC2_TEST_DIR. The chimney village: autostart chimneys and event driven ones.
    #[test]
    fn retail_chimney_container() {
        let Some(files) = blo_files("f2m05.stffront.nlz") else {
            return;
        };
        let v: Vec<PlacedParticle> = files.iter().flat_map(placed_particles).collect();
        assert_eq!(v.len(), 31);
        let chimneys: Vec<_> = v
            .iter()
            .filter(|p| p.leaf.starts_with("fx_env_chimney"))
            .collect();
        assert!(chimneys.iter().any(|p| p.autostart && p.start_event.is_none()));
        assert!(
            chimneys
                .iter()
                .any(|p| !p.autostart && p.start_event.is_some())
        );
        assert!(
            v.iter()
                .all(|p| !p.translation_only && p.third_event.is_none())
        );
    }

    /// Needs JC2_TEST_DIR. The damage smoke of a building listens to the very string the
    /// building's damage control fires when it is destroyed.
    #[test]
    fn retail_damage_smoke_listens_to_the_unit_event() {
        let Some(files) = blo_files("22x19.nlz") else {
            return;
        };
        let smoke: Vec<PlacedParticle> = files
            .iter()
            .flat_map(placed_particles)
            .filter(|p| p.leaf == "fx_env_smoke_damage_large_01")
            .collect();
        assert_eq!(smoke.len(), 2);
        let fired: Vec<String> = files
            .iter()
            .flat_map(crate::destruction::destructibles)
            .flat_map(|d| d.fires)
            .flat_map(|f| {
                f.split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect();
        for p in &smoke {
            assert!(!p.autostart && p.stop_event.is_none());
            let start = p.start_event.as_ref().unwrap();
            assert!(fired.contains(start), "{start}");
        }
    }
}
