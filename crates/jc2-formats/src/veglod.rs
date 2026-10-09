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

// ---------------------------------------------------------------------------------------
// Vegetation fade distances (JC2-126 / JC2-140): `research/formats/tree-impostors-next.md`,
// reproduced by `research/tools/tree_impostors_probe.py`.
//
// The per-frame vegetation manager (`00a92ca0`) writes eight floats into the render engine;
// the pass setup of the tree impostor render blocks (`00a83740` Top, `00a843a0` Trunk) turns
// them and the camera FOV into the vertex-shader constants `cb1[1]`, and the foliage vertex
// shaders (VegetationFoliage / BillboardFoliage) reuse the Top pair for their distance fade.
// ---------------------------------------------------------------------------------------

/// The eight render-engine distances (`DAT_01191ab4` +4..+0x20, copied to the render context
/// +0xd1c..+0xd38 by `0095a190`). Field names are the context offsets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineTable {
    pub d1c: f32,
    pub d20: f32,
    pub d24: f32,
    pub d28: f32,
    pub d2c: f32,
    pub d30: f32,
    pub d34: f32,
    pub d38: f32,
}

/// The table for the graphics setting `/lodfactor` 0..=3 (`00a92ca0`; level `DAT_0108b924`
/// set by `00a7b730`, default 0). Values above 3 behave as 3 (the manager tests `< 2`, `< 3`).
pub fn engine_table(lodfactor: u32) -> EngineTable {
    let (d2c, d34) = match lodfactor {
        0 | 1 => (55.0, 60.0),
        2 => (119.0, 124.0),
        _ => (183.0, 188.0),
    };
    EngineTable {
        d1c: 450.0,
        d20: 350.0,
        d24: 480.0,
        d28: 120.0,
        d2c,
        d30: 55.0,
        d34,
        d38: 60.0,
    }
}

/// FOV factor `f = max(fov^2, 0.38)` (`00a83740` / `00a843a0`, `ctx+0xca8` = camera `+0x618`,
/// the FOV in radians, vertical axis, JC2-141). There is no upper cap on `f`.
pub fn fov_factor(fov_rad: f32) -> f32 {
    (fov_rad * fov_rad).max(0.38)
}

/// `cb1[1]` of TreeImpostorTop (`00a83740`): `(d2c/(d30 f), 1/d30, d1c/(d20 f), 1/d20)`.
pub fn top_constants(t: &EngineTable, f: f32) -> [f32; 4] {
    [
        t.d2c / (t.d30 * f),
        1.0 / t.d30,
        t.d1c / (t.d20 * f),
        1.0 / t.d20,
    ]
}

/// `cb1[1]` of TreeImpostorTrunk (`00a843a0`); with `a = d38 f`:
/// `((a * 0.33333334 + d34) / a, 3 / (2 d38), d24 / (d28 f), 1 / d28)`.
pub fn trunk_constants(t: &EngineTable, f: f32) -> [f32; 4] {
    let a = t.d38 * f;
    [
        (a * 0.333_333_34 + t.d34) / a,
        3.0 / (t.d38 * 2.0),
        t.d24 / (t.d28 * f),
        1.0 / t.d28,
    ]
}

/// `(x, y)` of the foliage shaders' `1 - sat(dist * y - x)`: the Top pair (`cb1[1].xy`).
pub fn foliage_constants(t: &EngineTable, f: f32) -> [f32; 2] {
    let c = top_constants(t, f);
    [c[0], c[1]]
}

/// Largest distance at which the foliage fade can still be non-zero for any FOV: `f` never
/// drops below 0.38, so the fade is gone at `d2c / 0.38 + d30` (199.7 m for
/// lodfactor 0) however the camera zooms. Used for a CPU-side cull that changes nothing
/// visible (a bound derived from the native constants, not a native distance).
pub fn foliage_fade_end_max(t: &EngineTable) -> f32 {
    t.d2c / 0.38 + t.d30
}

fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// Clip threshold of the impostor vertex shaders (`lt r1.x, r0.w, l(0.996078)`).
pub const CARD_CLIP: f32 = 0.996_078;

/// Alpha of a tree impostor card from the vertex shaders (TreeImpostorTop / Trunk):
/// `min(1 - sat(cheb * w - z), sat(euclid * y - x)) * 3 + 0.99`; the card is clipped when it is
/// below [`CARD_CLIP`]. Returns `(alpha, visible)`.
pub fn card_alpha(c: &[f32; 4], cheb: f32, euclid: f32) -> (f32, bool) {
    let far = 1.0 - sat(cheb * c[3] - c[2]);
    let near = sat(euclid * c[1] - c[0]);
    let a = far.min(near) * 3.0 + 0.99;
    (a, a >= CARD_CLIP)
}

/// Foliage model fade (VegetationFoliage / BillboardFoliage VS): `1 - sat(dist * y - x)`,
/// multiplied into the instance fade `cb2[16].x`; 0 clips the vertex.
pub fn foliage_fade(c: &[f32; 2], dist: f32) -> f32 {
    1.0 - sat(dist * c[1] - c[0])
}

/// Chebyshev distance of a card centre from the camera (both impostor shaders):
/// `max(|dy| / 16, |dx|, |dz|)`.
pub fn card_chebyshev(d: [f32; 3]) -> f32 {
    (d[1].abs() * 0.0625).max(d[0].abs()).max(d[2].abs())
}

/// Euclid distance of a Top card (`sqrt(dx^2 + dy^2 + dz^2)`).
pub fn top_euclid(d: [f32; 3]) -> f32 {
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Euclid distance of a Trunk card (`sqrt(dx^2 + dz^2 + (dy/4)^2)`).
pub fn trunk_euclid(d: [f32; 3]) -> f32 {
    let y = d[1] * 0.25;
    (d[0] * d[0] + d[2] * d[2] + y * y).sqrt()
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

    fn bisect(pred: impl Fn(f32) -> bool, mut lo: f32, mut hi: f32) -> f32 {
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if pred(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }

    /// Fixtures of `tree_impostors_probe.py`: (foliage full, foliage gone, Top in, Top gone,
    /// Trunk in, Trunk gone).
    fn ranges(lod: u32, f: f32) -> [f32; 6] {
        let t = engine_table(lod);
        let (tc, rc, fc) = (
            top_constants(&t, f),
            trunk_constants(&t, f),
            foliage_constants(&t, f),
        );
        [
            bisect(|d| foliage_fade(&fc, d) < 1.0, 0.0, 4000.0),
            bisect(|d| foliage_fade(&fc, d) <= 0.0, 0.0, 4000.0),
            bisect(|e| card_alpha(&tc, 0.0, e).1, 0.0, 4000.0),
            bisect(|c| !card_alpha(&tc, c, 4000.0).1, 0.0, 4000.0),
            bisect(|e| card_alpha(&rc, 0.0, e).1, 0.0, 4000.0),
            bisect(|c| !card_alpha(&rc, c, 4000.0).1, 0.0, 4000.0),
        ]
    }

    fn near(got: f32, want: f32, tol: f32) {
        assert!((got - want).abs() <= tol, "{got} vs {want}");
    }

    #[test]
    fn engine_table_by_lodfactor() {
        let t0 = engine_table(0);
        assert_eq!(
            [t0.d1c, t0.d20, t0.d24, t0.d28, t0.d2c, t0.d30, t0.d34, t0.d38],
            [450.0, 350.0, 480.0, 120.0, 55.0, 55.0, 60.0, 60.0]
        );
        assert_eq!(engine_table(1), t0);
        assert_eq!((engine_table(2).d2c, engine_table(2).d34), (119.0, 124.0));
        assert_eq!((engine_table(3).d2c, engine_table(3).d34), (183.0, 188.0));
    }

    #[test]
    fn fov_factor_is_squared_fov_floored() {
        assert_eq!(fov_factor(1.0), 1.0);
        assert_eq!(fov_factor(0.5), 0.38);
        near(fov_factor(1.570_796_4), 2.4674, 1e-3);
    }

    #[test]
    fn fade_ranges_lodfactor_zero_f_one() {
        let [fi, fo, ti, to, ri, ro] = ranges(0, 1.0);
        near(fi, 55.0, 0.01);
        near(fo, 110.0, 0.01);
        near(ti, 55.11, 0.05);
        near(to, 799.3, 0.5);
        near(ri, 53.4, 0.1);
        near(ro, 599.8, 0.5);
    }

    #[test]
    fn fade_ranges_other_lodfactors_and_narrow_fov() {
        let [fi, fo, ti, ..] = ranges(2, 1.0);
        near(fi, 119.0, 0.02);
        near(fo, 174.0, 0.02);
        near(ti, 119.1, 0.1);
        let [fi, fo, ti, _, ri, _] = ranges(3, 1.0);
        near(fi, 183.0, 0.02);
        near(fo, 238.0, 0.02);
        near(ti, 183.1, 0.1);
        near(ri, 138.8, 0.1);
        // f = 0.38: the start distances stretch by 1/f, the widths do not
        let [fi, fo, ti, to, ri, ro] = ranges(0, 0.38);
        near(fi, 55.0 / 0.38, 0.02);
        near(fo, 55.0 / 0.38 + 55.0, 0.02);
        near(ti, 144.9, 0.1);
        near(to, 1533.5, 0.5);
        near(ri, 118.7, 0.1);
        near(ro, 1382.9, 0.5);
    }

    /// Native on-foot camera (JC2-141): 40 degrees vertical, `f32(40 * 0.017453292)`; the FOV
    /// factor has no upper cap, so `f = 0.48738784` and the start distances stretch by 1/f.
    #[test]
    fn fade_ranges_at_the_native_on_foot_fov() {
        let fov = 40.0_f32 * 0.017_453_292;
        assert_eq!(fov, 0.698_131_7);
        let f = fov_factor(fov);
        near(f, 0.487_387_84, 1e-7);
        let [fi, fo, ti, to, ri, ro] = ranges(0, f);
        near(fi, 112.84, 0.05);
        near(fo, 167.84, 0.05);
        near(ti, 55.0 / f + 0.11, 0.05);
        near(to, 450.0 / f + 349.29, 0.1);
        near(ri, 95.5, 0.1);
        near(ro, 480.0 / f + 119.76, 0.1);
        // no cap: a 90 degree FOV shrinks the distances
        let f90 = fov_factor(std::f32::consts::FRAC_PI_2);
        near(f90, 2.467_401, 1e-5);
        near(ranges(0, f90)[0], 55.0 / f90, 0.02);
    }

    #[test]
    fn foliage_is_gone_by_the_maximum_for_every_fov() {
        let t = engine_table(0);
        let max = foliage_fade_end_max(&t);
        near(max, 199.7, 0.05);
        for fov in [0.05_f32, 0.3, 0.6164, 0.698_131_7, 1.0, 1.4, 2.0] {
            let fc = foliage_constants(&t, fov_factor(fov));
            assert_eq!(foliage_fade(&fc, max + 0.1), 0.0, "{fov}");
        }
    }

    #[test]
    fn card_distances_follow_the_shader() {
        // height counts 1/16 for the Chebyshev test, the trunk's Euclid divides height by 4
        assert_eq!(card_chebyshev([3.0, -160.0, 4.0]), 10.0);
        assert_eq!(card_chebyshev([30.0, -160.0, 4.0]), 30.0);
        assert_eq!(top_euclid([3.0, 4.0, 12.0]), 13.0);
        assert_eq!(trunk_euclid([3.0, 16.0, 0.0]), 5.0);
        // a Top card is clipped before its fade-in and solid in between
        let c = top_constants(&engine_table(0), 1.0);
        assert!(!card_alpha(&c, 0.0, 40.0).1);
        assert_eq!(card_alpha(&c, 0.0, 200.0).0, 3.99);
        assert!(!card_alpha(&c, 900.0, 900.0).1);
        // foliage: full inside 55 m, half at 82.5 m, gone at 110 m
        let fc = foliage_constants(&engine_table(0), 1.0);
        assert_eq!(foliage_fade(&fc, 50.0), 1.0);
        near(foliage_fade(&fc, 82.5), 0.5, 1e-4);
        assert_eq!(foliage_fade(&fc, 120.0), 0.0);
    }
}
