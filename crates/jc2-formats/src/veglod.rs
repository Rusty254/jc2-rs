//! Native model-LOD selection of the original (`JustCause2.exe`), used for vegetation.
//!
//! Evidence and addresses: `research/formats/vegetation-lod-native.md`, reproduced by
//! `research/tools/vegetation_lod_probe.py`.
//!
//! * A `.lod` descriptor always has five slots (`TLodModel`, vtable +8 reports five once any
//!   slot is loaded, `00a47e60`). `-` is an empty slot. The optional sixth token is the
//!   model's **LOD scale**; without it the model default is 10.0 (`00a47ab0`).
//! * A model owner stores `1 / (s * F)^2` (`00a4f4b0`): `s` is that scale and `F` is
//!   `max(1, camera zoom factor)`. The scaled squared camera distance is `d^2` times it.
//! * `00a89b90` maps the scaled squared distance onto a continuous level through the
//!   breakpoints 100, 625, 2500, 10000 and 250000 (levels 1..5), selects slot
//!   `min(count-1, trunc(level))`, keeps `trunc(frac * 255)` as the transition fraction `k` and
//!   culls the model once the scaled distance passes the table entry for the slot count
//!   (`00f0ea0c`). Only the last (or last non-empty) slot fades, through the instance byte.
//!
//! Everything here is a pure function of those facts: no hysteresis, no cross-fade between
//! non-last slots (the original switches hard), and no policy distances.

/// Slots in a `.lod` descriptor.
pub const LOD_SLOTS: usize = 5;
/// `TLodModel` default scale when the descriptor has no sixth token (`00a47ab0`: 0x41200000).
pub const DEFAULT_LOD_SCALE: f32 = 10.0;

/// Breakpoints of the scaled squared distance (`00f0ea0c`, indexed by slot count): the level
/// reaches `i` at `BREAKS[i]`. Index 0 is the "nothing is ever visible" entry of a model
/// without slots.
const BREAKS: [f32; 6] = [0.0, 100.0, 625.0, 2500.0, 10000.0, 250000.0];
/// Interpolation constants at `0108a228..0108a238` (raw f32 bits).
const C_NEAR: f32 = f32::from_bits(1_008_981_770); // ~0.01
const C_1: f32 = f32::from_bits(989_440_308); // ~1/525
const C_2: f32 = f32::from_bits(973_852_517); // ~1/1875
const C_3: f32 = f32::from_bits(957_075_301); // ~1/7500
const C_4: f32 = f32::from_bits(915_132_261); // ~1/240000
/// `DAT_01089eb8` multiplies both factors of the owner scale (`00a4f4b0`).
const DISTANCE_UNIT: f32 = 1.0;
/// `2/255`, the last-slot fade slope printed by `00a89b90`.
const FADE_SLOPE: f32 = 0.007843138;

/// The owner's squared-distance multiplier (`owner+8`, written by `00a4f4b0`).
///
/// `model_scale` is the descriptor scale (`-1.0` means "none", as in the original),
/// `zoom` the camera factor at camera `+0x620` (1.0 at rest, clamped up to 1).
pub fn owner_scale(model_scale: f32, zoom: f32) -> f32 {
    let mut f = zoom;
    if 0.0 <= 1.0 - f {
        f = 1.0;
    }
    if model_scale != -1.0 {
        f *= model_scale;
    }
    1.0 / (((DISTANCE_UNIT * f) * DISTANCE_UNIT) * f)
}

/// Continuous LOD level of a scaled squared distance (`00a89b90`).
pub fn level(scaled_d2: f32) -> f32 {
    let x = scaled_d2;
    let mut l = if x - 100.0 < 0.0 {
        x * C_NEAR
    } else {
        (x - 100.0) * C_1 + 1.0
    };
    let mut l2 = (x - 625.0) * C_2 + 2.0;
    if 0.0 <= x - 2500.0 {
        l2 = (x - 2500.0) * C_3 + 3.0;
    }
    if 0.0 <= x - 625.0 {
        l = l2;
    }
    let l4 = (x - 10000.0) * C_4 + 4.0;
    if x - 10000.0 < 0.0 { l } else { l4 }
}

/// What the original decides for one model instance at one distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// Past the cull table entry: nothing is submitted.
    Culled,
    /// Slot `lod` (0-based) with transition fraction `k` (0..=255).
    Lod { lod: u8, k: u8 },
}

/// Selection for a model with `count` slots (5 for every `.lod`) and the owner multiplier of
/// [`owner_scale`], at squared camera distance `d2`. Both owner flags (cull, automatic LOD)
/// are at their initial values (`00a4f4b0`: set).
pub fn select(owner_scale: f32, d2: f32, count: u8) -> Selection {
    let scaled = owner_scale * d2;
    let count = count.min(5);
    if scaled > BREAKS[count as usize] {
        return Selection::Culled;
    }
    let mut l = level(scaled);
    if 0.0 <= l - count as f32 {
        l = count as f32 - 1.0;
    }
    if l < 0.0 {
        // unreachable for d2 >= 0; the original then leaves the stored slot unchanged
        return Selection::Lod { lod: 0, k: 0 };
    }
    let whole = l as i32;
    let lod = whole.min(count as i32 - 1).max(0) as u8;
    let k = (((l - whole as f32) * 255.0) as i32 & 0xff) as u8;
    Selection::Lod { lod, k }
}

/// First non-empty slot at or after `lod` (`00a47b40`), `None` when nothing is left to draw.
pub fn drawn_slot(filled: &[bool; LOD_SLOTS], lod: u8) -> Option<usize> {
    (lod as usize..LOD_SLOTS).find(|&i| filled[i])
}

/// Instance fade byte for a selection (`00a89b90` -> model owner `+0x1f`, consumed by the
/// foliage draw, JC2-024/025): 255 unless the slot is the last (slot 4, or the next slot is
/// empty), where it follows `k`. `None` = no submission (`k == 255` on the last slot).
pub fn instance_byte(filled: &[bool; LOD_SLOTS], lod: u8, k: u8) -> Option<u8> {
    let next_empty = (lod as usize + 1 >= LOD_SLOTS) || !filled[lod as usize + 1];
    if !next_empty {
        return Some(255);
    }
    if k > 0xfe {
        return None;
    }
    let f = 2.0 - k as f32 * FADE_SLOPE;
    Some(if f - 1.0 < 0.0 { (f * 255.0) as u8 } else { 255 })
}

/// Distance (m) at which the level reaches `level_index` (1..=5) for a model scale `s` at
/// zoom factor 1: `{10, 25, 50, 100, 500} * s`. Level 5 is where the model is culled.
pub fn distance_at_level(level_index: usize, model_scale: f32) -> f32 {
    BREAKS[level_index.clamp(1, 5)].sqrt() * model_scale
}

/// The five slots of one model resolved to distinct model names, with the owner multiplier:
/// what a runtime needs to pick the model to show at a camera distance.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotPlan {
    /// [`owner_scale`] at zoom factor 1.
    pub multiplier: f32,
    pub filled: [bool; LOD_SLOTS],
    /// Index into `models` per slot.
    pub slot_model: [Option<usize>; LOD_SLOTS],
    /// Distinct model names, in first-slot order.
    pub models: Vec<String>,
}

impl SlotPlan {
    pub fn new(model_scale: f32, slots: [Option<&str>; LOD_SLOTS]) -> Self {
        let mut models: Vec<String> = Vec::new();
        let mut slot_model = [None; LOD_SLOTS];
        for (i, name) in slots.iter().enumerate() {
            if let Some(name) = name {
                let at = models.iter().position(|m| m == name).unwrap_or_else(|| {
                    models.push((*name).to_string());
                    models.len() - 1
                });
                slot_model[i] = Some(at);
            }
        }
        Self {
            multiplier: owner_scale(model_scale, 1.0),
            filled: slot_model.map(|m| m.is_some()),
            slot_model,
            models,
        }
    }

    /// The model the original draws at squared camera distance `d2` (zoom factor 1):
    /// `None` when it culls the instance or the selected slot and all later ones are empty.
    pub fn model_at(&self, d2: f32) -> Option<usize> {
        match select(self.multiplier, d2, LOD_SLOTS as u8) {
            Selection::Culled => None,
            Selection::Lod { lod, .. } => self.slot_model[drawn_slot(&self.filled, lod)?],
        }
    }

    /// The model of the last filled slot (the one the original keeps the longest).
    pub fn far_model(&self) -> Option<usize> {
        self.slot_model.iter().rev().find_map(|m| *m)
    }

    /// More than one distinct model: only then is there anything to switch.
    pub fn switches(&self) -> bool {
        self.models.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_plan_picks_the_near_and_far_model() {
        let pine = SlotPlan::new(
            4.0,
            [
                Some("PineL_LOD1.rbm"),
                Some("PineL_LOD2.rbm"),
                Some("PineL_LOD2.rbm"),
                Some("PineL_LOD2.rbm"),
                Some("PineL_LOD2.rbm"),
            ],
        );
        assert_eq!(pine.models, ["PineL_LOD1.rbm", "PineL_LOD2.rbm"]);
        assert!(pine.switches());
        assert_eq!(pine.far_model(), Some(1));
        assert_eq!(pine.model_at(39.0 * 39.0), Some(0));
        assert_eq!(pine.model_at(41.0 * 41.0), Some(1));
        assert_eq!(pine.model_at(1999.0 * 1999.0), Some(1));
        assert_eq!(pine.model_at(2001.0 * 2001.0), None);
        // a bush with one model everywhere has nothing to switch
        let bush = SlotPlan::new(3.0, [Some("B_LOD1.rbm"); 5]);
        assert!(!bush.switches());
        assert_eq!(bush.model_at(10.0), Some(0));
        // rock with an empty tail: nothing is drawn once the selection passes slot 1
        let rock = SlotPlan::new(
            3.0,
            [Some("R_LOD1.rbm"), Some("R_LOD2.rbm"), None, None, None],
        );
        assert_eq!(rock.far_model(), Some(1));
        assert_eq!(rock.model_at(20.0 * 20.0), Some(0));
        assert_eq!(rock.model_at(50.0 * 50.0), Some(1));
        assert_eq!(rock.model_at(100.0 * 100.0), None);
        assert_eq!(rock.model_at(200.0 * 200.0), None);
    }

    fn at(scale: f32, metres: f32) -> Selection {
        select(owner_scale(scale, 1.0), metres * metres, 5)
    }

    #[test]
    fn default_scale_and_zoom() {
        assert_eq!(owner_scale(4.0, 1.0), 1.0 / 16.0);
        // zoom below one never shrinks the distances; above one it widens them
        assert_eq!(owner_scale(4.0, 0.5), owner_scale(4.0, 1.0));
        assert_eq!(owner_scale(4.0, 2.0), 1.0 / 64.0);
        // no descriptor scale (-1) leaves only the camera factor
        assert_eq!(owner_scale(-1.0, 3.0), 1.0 / 9.0);
    }

    #[test]
    fn level_hits_the_breakpoints() {
        for (x, want) in [
            (0.0, 0.0),
            (100.0, 1.0),
            (625.0, 2.0),
            (2500.0, 3.0),
            (10000.0, 4.0),
            (250000.0, 5.0),
        ] {
            assert!((level(x) - want).abs() < 1e-4, "{x}: {}", level(x));
        }
    }

    /// Arctic pine (`Arctic_T22_PineL-Whole.lod`): scale 4, slots LOD1, LOD2 x4.
    #[test]
    fn arctic_pine_thresholds() {
        let s = 4.0;
        assert_eq!(
            [1, 2, 3, 4, 5].map(|i| distance_at_level(i, s)),
            [40.0, 100.0, 200.0, 400.0, 2000.0]
        );
        assert_eq!(at(s, 0.0), Selection::Lod { lod: 0, k: 0 });
        assert!(matches!(at(s, 39.0), Selection::Lod { lod: 0, .. }));
        assert!(matches!(at(s, 41.0), Selection::Lod { lod: 1, .. }));
        assert!(matches!(at(s, 99.0), Selection::Lod { lod: 1, .. }));
        assert!(matches!(at(s, 101.0), Selection::Lod { lod: 2, .. }));
        assert!(matches!(at(s, 199.0), Selection::Lod { lod: 2, .. }));
        assert!(matches!(at(s, 201.0), Selection::Lod { lod: 3, .. }));
        assert!(matches!(at(s, 399.0), Selection::Lod { lod: 3, .. }));
        assert!(matches!(at(s, 401.0), Selection::Lod { lod: 4, .. }));
        assert!(matches!(at(s, 1999.0), Selection::Lod { lod: 4, .. }));
        assert_eq!(at(s, 2001.0), Selection::Culled);
        // the level is linear in the squared distance: 70 m is (306.25-100)/525 = 0.393 into
        // level 1, k = trunc(0.393 * 255) = 100
        let Selection::Lod { lod, k } = at(s, 70.0) else {
            panic!()
        };
        assert_eq!(lod, 1);
        assert_eq!(k, 100);
    }

    /// Slots filled like the pine descriptor: LOD1, LOD2, LOD2, LOD2, LOD2.
    #[test]
    fn pine_fades_only_in_the_last_slot() {
        let filled = [true; 5];
        assert_eq!(instance_byte(&filled, 0, 200), Some(255));
        assert_eq!(instance_byte(&filled, 3, 10), Some(255));
        assert_eq!(instance_byte(&filled, 4, 0), Some(255));
        // the JC2-025 fixtures: k 128 -> 253, 192 -> 125, 254 -> 1, 255 -> not submitted
        assert_eq!(instance_byte(&filled, 4, 128), Some(253));
        assert_eq!(instance_byte(&filled, 4, 192), Some(125));
        assert_eq!(instance_byte(&filled, 4, 254), Some(1));
        assert_eq!(instance_byte(&filled, 4, 255), None);
    }

    /// Rock (`Arctic_G03_RocksS-Whole.lod`): scale 3, slots LOD1, LOD2, -, -, -. Slot 1 is
    /// the last filled one, so it fades from where level 1 starts to where level 2 ends,
    /// and slots 2..4 find nothing to draw.
    #[test]
    fn rock_with_empty_tail_slots() {
        let s = 3.0;
        let filled = [true, true, false, false, false];
        assert_eq!(distance_at_level(5, s), 1500.0);
        let Selection::Lod { lod, k } = at(s, 20.0) else {
            panic!()
        };
        assert_eq!((lod, instance_byte(&filled, lod, k)), (0, Some(255)));
        let Selection::Lod { lod, k } = at(s, 60.0) else {
            panic!()
        };
        assert_eq!(lod, 1);
        assert!(instance_byte(&filled, lod, k).unwrap() < 255);
        let Selection::Lod { lod, .. } = at(s, 50.0) else {
            panic!()
        };
        assert_eq!(drawn_slot(&filled, lod), Some(1));
        let Selection::Lod { lod, .. } = at(s, 80.0) else {
            panic!()
        };
        assert_eq!((lod, drawn_slot(&filled, lod)), (2, None));
        let Selection::Lod { lod, .. } = at(s, 200.0) else {
            panic!()
        };
        assert_eq!(lod, 3);
        assert_eq!(drawn_slot(&filled, lod), None);
    }

    /// Bush (`Arctic_B11_bushtreeL-Whole.lod`): scale 3 and the same model in all slots.
    #[test]
    fn bush_scale_three() {
        assert_eq!(
            [1, 2, 3, 4, 5].map(|i| distance_at_level(i, 3.0)),
            [30.0, 75.0, 150.0, 300.0, 1500.0]
        );
        assert!(matches!(at(3.0, 29.0), Selection::Lod { lod: 0, .. }));
        assert!(matches!(at(3.0, 31.0), Selection::Lod { lod: 1, .. }));
        assert_eq!(at(3.0, 1501.0), Selection::Culled);
    }

    #[test]
    fn camera_zoom_widens_every_threshold() {
        let wide = owner_scale(4.0, 2.0);
        assert!(matches!(
            select(wide, 70.0 * 70.0, 5),
            Selection::Lod { lod: 0, .. }
        ));
        assert_eq!(select(wide, 4001.0 * 4001.0, 5), Selection::Culled);
    }

    #[test]
    fn short_slot_counts_cull_at_their_table_entry() {
        // count 2: culled where the level would reach 2 (scaled d^2 = 625), never clamped back
        let o = owner_scale(1.0, 1.0);
        let Selection::Lod { lod, k } = select(o, 24.0 * 24.0, 2) else {
            panic!()
        };
        assert_eq!(lod, 1);
        assert!((225..=235).contains(&k), "{k}");
        assert_eq!(select(o, 26.0 * 26.0, 2), Selection::Culled);
    }
}
