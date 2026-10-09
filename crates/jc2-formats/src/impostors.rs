//! Tree impostors: what the engine draws for a tree beyond its model range
//! (`climate\common\treeimpostors.bin`, `CTreeImpostorTopRB` / `CTreeImpostorTrunkRB`;
//! see `research/formats/world.md`, "Tree impostors").
//!
//! Every plant type of the vegetation set with a `billboard` id (0..33; −1 = none) has up
//! to two impostors, both rectangles cut out of an atlas:
//!
//! * the **top**: the crown, from `climate\common\TopAtlas_DIF.dds` (2048 × 1024 DXT5,
//!   alpha), `size` metres wide and tall, its centre `offset.y` metres above the tree's
//!   foot. For the broadleaf trees it is a side view (a card that faces the camera); for
//!   the palms and pines (`#190d9496` = 0) it is a view from above, a card that lies flat;
//! * the **trunk**: the stem from the side, from `TrunkAtlas_DIF.dds` (1024 × 512 DXT5,
//!   with `TrunkAtlas_NRM.dds` next to it), likewise, always a standing card.
//!
//! Both atlases' alpha is a random 0.5..1 noise inside the shapes (for dithered fades);
//! a cutoff of 0.4 gives the solid silhouettes.
//!
//! Entries are nodes of two lists, `#567238eb` (the trunks) and `#f325bb6e` (the crowns): `id`, `#190d9496` (the flag above: 0 for the
//! palms and pines, 1 otherwise), `offset` (a vector, only `y` is non-zero),
//! `#755a3e9e` the name of the entry's `.tm` polygon (`impostor_top_LOD1-JT06.tm`; the
//! polygons themselves are `climate\common\impostor_{top,trunk}_lod1-*.bin`, a tight hull
//! of five or so points inside the cell that saves fill rate), `#b5b709ad` the atlas
//! rectangle `u0 v0 u1 v1` (v down) and `#b97abf28` the size in metres. A few entries are
//! empty placeholders (size 0).

use crate::property::{Node, PropertyFile, Variant};

const KEY_TOP_ATLAS: u32 = 0xf1af_b0d7;
const KEY_TRUNK_ATLAS: u32 = 0xc7ff_282e;
const KEY_TRUNK_NORMALS: u32 = 0x9409_5ab4;
/// The two lists: the trunks' and the crowns'.
const KEY_TRUNKS: u32 = 0x5672_38eb;
const KEY_TOPS: u32 = 0xf325_bb6e;
const KEY_FLAG: u32 = 0x190d_9496;
const KEY_UV: u32 = 0xb5b7_09ad;
const KEY_SIZE: u32 = 0xb97a_bf28;

/// One rectangle of an atlas, placed on a tree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpostorQuad {
    /// Height of the rectangle's centre above the tree's foot (m).
    pub offset_y: f32,
    /// Width and height in metres (for the top, its extent on the ground).
    pub size: [f32; 2],
    /// `[u0, v0, u1, v1]` in the atlas, v down.
    pub uv: [f32; 4],
    /// The picture is a view from above: the card lies on the horizontal (palm and pine
    /// crowns); false for a card that stands and faces the camera.
    pub flat: bool,
}

/// The impostors of one tree id.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TreeImpostor {
    pub top: Option<ImpostorQuad>,
    pub trunk: Option<ImpostorQuad>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeImpostors {
    /// Archive-relative atlas names as the file gives them.
    pub top_atlas: String,
    pub trunk_atlas: String,
    pub trunk_normals: String,
    /// Index = the plant type's `billboard` id.
    pub trees: Vec<TreeImpostor>,
}

fn floats(n: &Node, hash: u32) -> Option<Vec<f32>> {
    match n.value_hash(hash)? {
        Variant::Vec2([a, b]) => Some(vec![*a, *b]),
        Variant::Vec3(v) => Some(v.to_vec()),
        Variant::Vec4(v) => Some(v.to_vec()),
        Variant::Floats(v) => Some(v.clone()),
        _ => None,
    }
}

impl TreeImpostors {
    pub const FILE: &'static str = "climate\\common\\treeimpostors.bin";

    pub fn parse(file: &PropertyFile) -> Self {
        let mut out = Self::default();
        for root in &file.nodes {
            let text = |h| match root.value_hash(h) {
                Some(Variant::Str(s)) => s.clone(),
                _ => String::new(),
            };
            out.top_atlas = text(KEY_TOP_ATLAS);
            out.trunk_atlas = text(KEY_TRUNK_ATLAS);
            out.trunk_normals = text(KEY_TRUNK_NORMALS);
            for (key, is_top) in [(KEY_TRUNKS, false), (KEY_TOPS, true)] {
                let Some((_, list)) = root.children.iter().find(|(k, _)| k.hash() == key) else {
                    continue;
                };
                for (_, e) in &list.children {
                    let Some(id) = e.int("id").filter(|i| (0..4096).contains(i)) else {
                        continue;
                    };
                    let (Some(uv), Some(size)) = (floats(e, KEY_UV), floats(e, KEY_SIZE)) else {
                        continue;
                    };
                    if uv.len() < 4 || size.len() < 2 || size[0] <= 0.0 || size[1] <= 0.0 {
                        continue; // a placeholder
                    }
                    let offset_y = floats(e, crate::hash::hash_str("offset"))
                        .and_then(|o| o.get(1).copied())
                        .unwrap_or(0.0);
                    let quad = ImpostorQuad {
                        offset_y,
                        size: [size[0], size[1]],
                        uv: [uv[0], uv[1], uv[2], uv[3]],
                        flat: is_top && matches!(e.value_hash(KEY_FLAG), Some(Variant::Int(0))),
                    };
                    let id = id as usize;
                    if out.trees.len() <= id {
                        out.trees.resize(id + 1, TreeImpostor::default());
                    }
                    if is_top {
                        out.trees[id].top = Some(quad);
                    } else {
                        out.trees[id].trunk = Some(quad);
                    }
                }
            }
        }
        out
    }

    pub fn load(fs: &crate::GameFs) -> crate::Result<Self> {
        let bytes = fs.read_inflated(Self::FILE)?;
        Ok(Self::parse(&PropertyFile::parse_any(&bytes)?))
    }

    /// The impostor of a plant type's `billboard` id.
    pub fn get(&self, billboard: i32) -> Option<&TreeImpostor> {
        self.trees
            .get(usize::try_from(billboard).ok()?)
            .filter(|t| t.top.is_some() || t.trunk.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameFs;
    use crate::vegetation::Vegetation;

    #[test]
    fn impostors_read_and_match_the_vegetation_types() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let Ok(fs) = GameFs::open(dir) else { return };
        if !fs.contains(TreeImpostors::FILE) {
            return;
        }
        let ti = TreeImpostors::load(&fs).unwrap();
        assert!(
            ti.top_atlas
                .to_ascii_lowercase()
                .ends_with("topatlas_dif.dds")
        );
        assert!(
            ti.trunk_atlas
                .to_ascii_lowercase()
                .ends_with("trunkatlas_dif.dds")
        );
        assert_eq!(ti.trees.len(), 34);
        // the first jungle canopy: a 17 × 18 m crown 30 m up, a 4 × 14 m trunk 9 m up
        let t = ti.get(0).unwrap();
        let top = t.top.unwrap();
        assert_eq!(top.uv, [0.0, 0.5, 0.25, 1.0]);
        assert!((top.size[0] - 16.867058).abs() < 1e-4 && (top.offset_y - 30.0).abs() < 1e-6);
        assert!((t.trunk.unwrap().offset_y - 9.0).abs() < 1e-6);
        assert!(!top.flat, "a broadleaf crown is a side view");
        assert!(
            ti.get(6).unwrap().top.unwrap().flat,
            "a palm's crown lies flat"
        );
        assert!(!ti.get(6).unwrap().trunk.unwrap().flat);
        // the placeholders (ids 20..22 have no top, 30/31 no trunk)
        assert!(ti.get(20).unwrap().top.is_none() && ti.get(20).unwrap().trunk.is_some());
        assert!(ti.get(30).unwrap().trunk.is_none() && ti.get(30).unwrap().top.is_some());
        // every tree type of the vegetation names an id that exists
        let veg = Vegetation::load(&fs).unwrap();
        let mut with = 0;
        for z in veg.zones.iter().flatten() {
            for ty in &z.types {
                if ty.billboard >= 0 {
                    assert!(
                        ti.get(ty.billboard).is_some(),
                        "{} id {}",
                        ty.name,
                        ty.billboard
                    );
                    with += 1;
                }
            }
        }
        assert!(with >= 60, "{with} types with an impostor");
        // the jungle's big canopy tree is one of them
        let z0 = veg.zone(0).unwrap();
        let t01 = &z0.types[z0.type_index("Jungle_T01_CanopyM").unwrap() as usize];
        assert_eq!(t01.billboard, 0);
    }
}
