//! `settings\blackmarket.bin`: what the Black Market sells.
//!
//! ```text
//! costs { 0 { level = 0, parts = 5 } … 4 { level = 4, parts = 25 } }   parts per upgrade
//! parameters { set_0 { parameter_set = 0, parameter_0 … 3 = "gui_wea_txt_01" … } … }
//! weapons  { weapon_N  { id, type = 0, entity = "heavydrops/mod.heavydrop.revolver.ee",
//!                        item_id = "WEA04", chaos = 40000, price = 40000,
//!                        name = "gui_txt_60", description = "gui_desc_txt_60",
//!                        parameter_set = 0, parameter_0 … 3 = [min, max],
//!                        #fba8025d { level_0 = "bm_revolver.dds" } } }
//! vehicles { vehicle_N { … type = 1, entity = "Vehicles/LAVE/LAVE.V056_ATV.ee", … } }
//! ```
//!
//! `chaos` is the chaos an item needs before it is on sale, `price` its dollars, `costs`
//! the weapon (or vehicle) parts each upgrade level asks for: the step from level `L`
//! to `L + 1` costs `costs[L].parts`. The `parameter_N` pairs are the stat bars (the
//! captions are the parameter set's text keys); `#fba8025d` the picture per upgrade
//! level (`bm_*.dds`, in `pc3`). Keys named by hashing (`research/formats/chaos.md`).

use crate::property::{Node, PropertyFile, Variant};

/// What kind of thing an item is (`parameter_set`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    /// 0: guns (`gui_wea_txt_01…04`: damage, rate of fire, accuracy, magazine).
    Weapon,
    /// 1: grenades and charges (`gui_wea_txt_06…08`).
    Explosive,
    /// 2: vehicles (`gui_v_txt_01…04`: top speed, acceleration, handling, armour).
    Vehicle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarketItem {
    pub id: i32,
    pub category: Category,
    /// `entity`: the heavy drop (`heavydrops/mod.heavydrop.revolver.ee`) or the vehicle.
    pub entity: String,
    /// `item_id`: `WEA04`, `v056`.
    pub item_id: String,
    /// `chaos` needed before it is for sale.
    pub chaos: u32,
    pub price: u32,
    /// Text keys.
    pub name: String,
    pub description: String,
    /// `parameter_0 … 3`: (min, max) of each stat bar, in the order of the set's captions.
    pub stats: Vec<Option<[f32; 2]>>,
    /// The picture for each upgrade level that has one (`bm_revolver.dds`).
    pub images: Vec<String>,
}

impl MarketItem {
    /// The container name of the entity (`mod.heavydrop.revolver.ee`, `lave.v056_atv.ee`).
    pub fn container(&self) -> String {
        crate::placement::bare_name(&self.entity.replace('/', "\\")).to_ascii_lowercase()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlackMarket {
    /// `costs`: parts for the step up from level `i`.
    pub costs: Vec<u32>,
    /// `parameters`: the stat captions (text keys) per category.
    pub captions: Vec<(Category, Vec<String>)>,
    /// Weapons, explosives and vehicles, each group in `id` order.
    pub items: Vec<MarketItem>,
}

const IMAGES: u32 = 0xfba8_025d;

fn category(n: i32) -> Category {
    match n {
        1 => Category::Explosive,
        2 => Category::Vehicle,
        _ => Category::Weapon,
    }
}

fn item(n: &Node) -> Option<MarketItem> {
    let stats = (0..4)
        .map(|i| match n.value(&format!("parameter_{i}")) {
            Some(Variant::Vec2(v)) => Some(*v),
            Some(Variant::Floats(v)) if v.len() >= 2 => Some([v[0], v[1]]),
            _ => None,
        })
        .collect();
    let mut images: Vec<(usize, String)> = n
        .children
        .iter()
        .find(|(k, _)| k.hash() == IMAGES)
        .map(|(_, c)| {
            (0..6)
                .filter_map(|l| Some((l, c.str(&format!("level_{l}"))?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    images.sort();
    Some(MarketItem {
        id: n.int("id")?,
        category: category(n.int("parameter_set").unwrap_or(0)),
        entity: n.str("entity")?.to_string(),
        item_id: n.str("item_id").unwrap_or("").to_string(),
        chaos: n.int("chaos").unwrap_or(0).max(0) as u32,
        price: n.int("price").unwrap_or(0).max(0) as u32,
        name: n.str("name").unwrap_or("").to_string(),
        description: n.str("description").unwrap_or("").to_string(),
        stats,
        images: images.into_iter().map(|i| i.1).collect(),
    })
}

impl BlackMarket {
    pub fn parse(data: &[u8]) -> crate::Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        let mut out = Self::default();
        for root in &file.nodes {
            if let Some(c) = root.child("costs") {
                let mut costs: Vec<(i32, u32)> = c
                    .children
                    .iter()
                    .filter_map(|(_, l)| Some((l.int("level")?, l.int("parts")?.max(0) as u32)))
                    .collect();
                costs.sort();
                out.costs = costs.into_iter().map(|c| c.1).collect();
            }
            if let Some(p) = root.child("parameters") {
                for (_, s) in &p.children {
                    let caps = (0..4)
                        .filter_map(|i| s.str(&format!("parameter_{i}")).map(str::to_string))
                        .collect();
                    out.captions
                        .push((category(s.int("parameter_set").unwrap_or(0)), caps));
                }
                out.captions.sort_by_key(|c| c.0);
            }
            for group in ["weapons", "vehicles"] {
                if let Some(g) = root.child(group) {
                    let mut items: Vec<MarketItem> =
                        g.children.iter().filter_map(|(_, n)| item(n)).collect();
                    items.sort_by_key(|i| (i.category, i.id));
                    out.items.extend(items);
                }
            }
        }
        out.items.sort_by_key(|i| (i.category, i.id));
        Ok(out)
    }

    /// Parts for the upgrade from `level` to the next; `None` at the top.
    pub fn upgrade_cost(&self, level: usize) -> Option<u32> {
        self.costs.get(level).copied()
    }

    /// The most upgrade levels an item can climb (5: levels 0 … 5).
    pub fn max_level(&self) -> usize {
        self.costs.len()
    }

    pub fn captions(&self, c: Category) -> &[String] {
        self.captions
            .iter()
            .find(|x| x.0 == c)
            .map_or(&[], |x| x.1.as_slice())
    }

    /// The smallest chaos above `chaos` that unlocks something (the "Next Black Market
    /// Item" bar).
    pub fn next_unlock(&self, chaos: u32) -> Option<u32> {
        self.items
            .iter()
            .map(|i| i.chaos)
            .filter(|c| *c > chaos)
            .min()
    }

    /// The highest unlock threshold at or below `chaos` (0 when none).
    pub fn last_unlock(&self, chaos: u32) -> u32 {
        self.items
            .iter()
            .map(|i| i.chaos)
            .filter(|c| *c <= chaos)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `blackmarket.bin`.
    #[test]
    fn the_black_market() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        let Ok(data) = fs.read_inflated("blackmarket.bin") else {
            return;
        };
        let m = BlackMarket::parse(&data).unwrap();
        assert_eq!(m.costs, vec![5, 10, 15, 20, 25]);
        assert_eq!(m.max_level(), 5);
        assert_eq!(m.upgrade_cost(0), Some(5));
        assert_eq!(m.upgrade_cost(5), None);
        assert_eq!(m.items.len(), 22);
        let weapons = m
            .items
            .iter()
            .filter(|i| i.category == Category::Weapon)
            .count();
        let explosives = m
            .items
            .iter()
            .filter(|i| i.category == Category::Explosive)
            .count();
        assert_eq!((weapons, explosives), (10, 2));
        let pistol = &m.items[0];
        assert_eq!(
            (pistol.item_id.as_str(), pistol.price, pistol.chaos),
            ("WEA02", 20000, 0)
        );
        assert_eq!(pistol.name, "gui_txt_59");
        assert_eq!(pistol.container(), "mod.heavydrop.beretta.ee");
        assert_eq!(pistol.images, vec!["bm_pistol.dds".to_string()]);
        assert_eq!(pistol.stats[0], Some([15.0, 35.0]));
        let rev = m.items.iter().find(|i| i.item_id == "WEA04").unwrap();
        assert_eq!((rev.price, rev.chaos), (40000, 40000));
        let atv = m.items.iter().find(|i| i.item_id == "v056").unwrap();
        assert_eq!(atv.category, Category::Vehicle);
        assert_eq!(atv.container(), "lave.v056_atv.ee");
        assert_eq!((atv.price, atv.chaos), (25000, 140000));
        assert_eq!(atv.images.len(), 6);
        assert_eq!(
            m.captions(Category::Weapon),
            [
                "gui_wea_txt_01",
                "gui_wea_txt_02",
                "gui_wea_txt_03",
                "gui_wea_txt_04"
            ]
        );
        assert_eq!(m.captions(Category::Vehicle)[3], "gui_v_txt_04");
        assert_eq!(m.next_unlock(0), Some(40000));
        assert_eq!(m.next_unlock(40000), Some(70000));
        assert_eq!(m.last_unlock(50000), 40000);
    }
}
