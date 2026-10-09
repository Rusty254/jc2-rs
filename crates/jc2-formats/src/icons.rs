//! The map's icons. `settings\icon_types.bin` lists the icon kinds — `type` (a number),
//! `faction`, and `#3acab26e` the texture (`gui\MiniMapIcons\hud_icon_s_u_village_dif.dds`);
//! `settings\icons_global.bin` places 899 icons — `id`, `#01dfc541` the position (engine
//! axes), `#c9f36d94` a kind (collectables 8/9/10, challenges 17, …; 0 on the settlements,
//! whose kind is in the next file), `visible`, `enabled` and their events;
//! `settings\settlements.bin` names the settlements — `id` (the icon's: `31x14`,
//! `14x50base`, `dcs40`), `#4c4b2806` their icon kind (24 military base, 26/28
//! harbours, 27 airfield, 29 city, 30 village) and `text`, the key of their name in the
//! text database ([`crate::text::TextDb`]: `set_mil_txt_11` is "Lembah Genting Tinggi").
//!
//! The PDA's map (`pda_map_dif.dds`, 2048² DXT1) covers the whole world at 16 m a pixel,
//! north up: pixel `((x + 16384) / 16, (z + 16384) / 16)` for engine `(x, z)` — the
//! image's rows run with engine z, like the height maps (`research/formats/map.md`).

use crate::property::{Node, PropertyFile, Variant};

#[derive(Clone, Debug, PartialEq)]
pub struct IconType {
    pub id: String,
    pub kind: i32,
    pub faction: i32,
    /// The texture's file name, lower case (`hud_icon_s_u_village_dif.dds`).
    pub texture: String,
    /// `#fd7350ca`: the "completed" variant (`hud_icon_s_u_village_c_dif.dds`, shown once
    /// a settlement is liberated or a challenge done).
    pub completed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapIcon {
    pub id: String,
    pub position: [f32; 3],
    pub kind: i32,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settlement {
    pub id: String,
    pub kind: i32,
    pub text: String,
}

/// The kinds settlements use, with what they are. 32 are the factions' strongholds
/// (`f1s07_title_txt`: the mission title; the textures `hud_icon_sh_*`).
pub const SETTLEMENT_KINDS: [(i32, &str); 8] = [
    (24, "military base"),
    (25, "military airfield"),
    (26, "military harbour"),
    (27, "airfield"),
    (28, "harbour"),
    (29, "city"),
    (30, "village"),
    (32, "stronghold"),
];

const TEXTURE: u32 = 0x3aca_b26e;
const POSITION: u32 = 0x01df_c541;
const ICON_KIND: u32 = 0xc9f3_6d94;
const SETTLEMENT_KIND: u32 = 0x4c4b_2806;
const COMPLETED: u32 = 0xfd73_50ca;

/// Edge of the PDA map image in pixels, and the world it covers in metres.
pub const MAP_PIXELS: f32 = 2048.0;
pub const MAP_WORLD: f32 = 32768.0;
/// The world's smallest engine x and z (the map's top-left corner).
pub const MAP_ORIGIN: f32 = -16384.0;

fn int(n: &Node, h: u32) -> Option<i32> {
    match n.value_hash(h)? {
        Variant::Int(i) => Some(*i),
        Variant::Float(f) => Some(*f as i32),
        _ => None,
    }
}

fn each(data: &[u8], mut f: impl FnMut(&Node)) -> crate::Result<()> {
    let file = PropertyFile::parse_any(data)?;
    for root in &file.nodes {
        root.walk(&mut |n| f(n));
    }
    Ok(())
}

pub fn icon_types(data: &[u8]) -> crate::Result<Vec<IconType>> {
    let mut out = Vec::new();
    each(data, |n| {
        if let (Some(id), Some(Variant::Str(t))) = (n.str("id"), n.value_hash(TEXTURE)) {
            out.push(IconType {
                id: id.to_string(),
                kind: n.int("type").unwrap_or(-1),
                faction: n.int("faction").unwrap_or(0),
                texture: crate::placement::bare_name(t).to_ascii_lowercase(),
                completed: int(n, COMPLETED).is_some_and(|c| c != 0),
            });
        }
    })?;
    Ok(out)
}

pub fn map_icons(data: &[u8]) -> crate::Result<Vec<MapIcon>> {
    let mut out = Vec::new();
    each(data, |n| {
        if let (Some(id), Some(Variant::Vec3(p))) = (n.str("id"), n.value_hash(POSITION)) {
            out.push(MapIcon {
                id: id.to_string(),
                position: *p,
                kind: int(n, ICON_KIND).unwrap_or(0),
                visible: n.int("visible").is_none_or(|v| v != 0),
            });
        }
    })?;
    Ok(out)
}

pub fn settlements(data: &[u8]) -> crate::Result<Vec<Settlement>> {
    let mut out = Vec::new();
    each(data, |n| {
        if let (Some(id), Some(kind)) = (n.str("id"), int(n, SETTLEMENT_KIND)) {
            out.push(Settlement {
                id: id.to_string(),
                kind,
                text: n.str("text").unwrap_or("").to_string(),
            });
        }
    })?;
    Ok(out)
}

/// The map pixel (0..1 on each axis, north up) of an engine position.
pub fn map_uv(x: f32, z: f32) -> (f32, f32) {
    ((x - MAP_ORIGIN) / MAP_WORLD, (z - MAP_ORIGIN) / MAP_WORLD)
}

/// The engine `(x, z)` of a map position given as 0..1 on each axis (inverse of
/// [`map_uv`]).
pub fn map_uv_to_world(u: f32, v: f32) -> (f32, f32) {
    (MAP_ORIGIN + u * MAP_WORLD, MAP_ORIGIN + v * MAP_WORLD)
}

/// The pixel of `pda_map_dif.dds` (0..2048, row down) an engine position falls on.
pub fn map_pixel(x: f32, z: f32) -> (f32, f32) {
    let (u, v) = map_uv(x, z);
    (u * MAP_PIXELS, v * MAP_PIXELS)
}

/// What a settlement kind is (`SETTLEMENT_KINDS`).
pub fn kind_name(kind: i32) -> Option<&'static str> {
    SETTLEMENT_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, n)| *n)
}

/// The icon texture of a kind: the plain (not completed) variant when there is one, of
/// no faction when there is one (the generic stronghold rather than Ular's).
pub fn texture_for_kind(types: &[IconType], kind: i32) -> Option<&str> {
    let of_kind = || types.iter().filter(move |t| t.kind == kind);
    of_kind()
        .find(|t| !t.completed && t.faction == 0)
        .or_else(|| of_kind().find(|t| !t.completed))
        .or_else(|| of_kind().next())
        .map(|t| t.texture.as_str())
}

/// A settlement as the map draws it: `settlements.bin` joined with its icon in
/// `icons_global.bin` (by `id`) and its kind's texture in `icon_types.bin`.
#[derive(Clone, Debug, PartialEq)]
pub struct SettlementIcon {
    pub id: String,
    pub kind: i32,
    /// Engine axes.
    pub position: [f32; 3],
    /// The name's key in the text database (`set_mil_txt_67`).
    pub text: String,
    /// `None` when `icon_types.bin` has no texture for the kind.
    pub texture: Option<String>,
}

/// Join the three files: every settlement that has an icon, in `settlements` order.
pub fn settlement_icons(
    types: &[IconType],
    icons: &[MapIcon],
    settlements: &[Settlement],
) -> Vec<SettlementIcon> {
    let by_id: std::collections::HashMap<String, &MapIcon> = icons
        .iter()
        .map(|i| (i.id.to_ascii_lowercase(), i))
        .collect();
    settlements
        .iter()
        .filter_map(|s| {
            let icon = by_id.get(&s.id.to_ascii_lowercase())?;
            Some(SettlementIcon {
                id: s.id.clone(),
                kind: s.kind,
                position: icon.position,
                text: s.text.clone(),
                texture: texture_for_kind(types, s.kind).map(str::to_string),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with the three settings files.
    #[test]
    fn the_villages_are_on_the_map() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(t), Ok(i), Ok(s)) = (
            fs.read_inflated("icon_types.bin"),
            fs.read_inflated("icons_global.bin"),
            fs.read_inflated("settlements.bin"),
        ) else {
            return;
        };
        let types = icon_types(&t).unwrap();
        let village = types.iter().find(|t| t.kind == 30).unwrap();
        assert!(village.texture.starts_with("hud_icon_s_u_village"));
        let icons = map_icons(&i).unwrap();
        assert_eq!(icons.len(), 899);
        let set = settlements(&s).unwrap();
        assert!(set.len() > 300, "{}", set.len());
        // the start "village" is a military base, its icon on tile 31,14
        let ours = set.iter().find(|s| s.id == "31x14").unwrap();
        assert_eq!(ours.kind, 24);
        let icon = icons.iter().find(|i| i.id == "31x14").unwrap();
        let (u, v) = map_uv(icon.position[0], icon.position[2]);
        assert_eq!(((u * 64.0) as u32, (v * 64.0) as u32), (31, 14));
    }

    #[test]
    fn map_pixels_round_trip() {
        assert_eq!(map_pixel(-16384.0, -16384.0), (0.0, 0.0));
        assert_eq!(map_pixel(0.0, 0.0), (1024.0, 1024.0));
        // 16 m a pixel
        assert_eq!(map_pixel(16.0, -16.0), (1025.0, 1023.0));
        for (x, z) in [(-451.8, -8860.4), (12000.0, 3.5), (-16000.0, 16000.0)] {
            let (u, v) = map_uv(x, z);
            let (x2, z2) = map_uv_to_world(u, v);
            assert!((x2 - x).abs() < 1e-2 && (z2 - z).abs() < 1e-2);
        }
        assert_eq!(kind_name(24), Some("military base"));
        assert_eq!(kind_name(8), None);
    }

    /// The join the map and `jc2tool icons` use. Needs JC2_TEST_DIR.
    #[test]
    fn settlements_join_their_icons() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let (Ok(t), Ok(i), Ok(s)) = (
            fs.read_inflated("icon_types.bin"),
            fs.read_inflated("icons_global.bin"),
            fs.read_inflated("settlements.bin"),
        ) else {
            return;
        };
        let types = icon_types(&t).unwrap();
        let joined = settlement_icons(&types, &map_icons(&i).unwrap(), &settlements(&s).unwrap());
        assert!(joined.len() > 300, "{}", joined.len());
        assert!(
            joined.iter().all(|s| s.texture.is_some()),
            "a kind without a texture"
        );
        let base = joined.iter().find(|s| s.id == "31x14").unwrap();
        assert_eq!(base.kind, 24);
        assert_eq!(kind_name(base.kind), Some("military base"));
        assert_eq!(
            base.texture.as_deref(),
            Some("hud_icon_s_r_military_base_dif.dds")
        );
        let (px, pz) = map_pixel(base.position[0], base.position[2]);
        // 32 pixels a tile
        assert_eq!(((px / 32.0) as u32, (pz / 32.0) as u32), (31, 14));
        let village = texture_for_kind(&types, 30).unwrap();
        assert_eq!(village, "hud_icon_s_u_village_dif.dds");
        assert_eq!(texture_for_kind(&types, 32), Some("hud_icon_sh_dif.dds"));
        // every settlement's kind is one we can name
        assert!(joined.iter().all(|s| kind_name(s.kind).is_some()));
    }
}
