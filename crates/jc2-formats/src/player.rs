//! The player's settings: `settings\player.bin` (Rico's model and skeleton) and
//! `settings\grapplingparams.bin` (the grappling hook). See `research/formats/character.md`.
//!
//! Most keys in these files are hashed; the ones named here were found by hashing
//! candidate names (`pull_force`, `reel_in_speed`, `hook_model`, `wire_width`,
//! `pull_force_character`); the rest are kept by hash with their observed values.

use crate::Result;
use crate::property::{Node, PropertyFile, Variant};

/// `settings\player.bin`, the `player` node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerSettings {
    /// `Characters\MainCharacters\Rico\mc01_lod1-rico` — the render model, without
    /// its `.rbm` extension.
    pub model: String,
    /// `animations\skeletons\syncreadthis\biped.bsk` (key `#26fa86fe`).
    pub skeleton: String,
}

impl PlayerSettings {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        Ok(Self::from_property(&file))
    }

    pub fn from_property(file: &PropertyFile) -> Self {
        let mut s = Self::default();
        for root in &file.nodes {
            root.walk(&mut |n| {
                if let Some(m) = n.str("model")
                    && s.model.is_empty()
                {
                    s.model = m.to_string();
                }
                if let Some(Variant::Str(sk)) = n.value_hash(0x26fa_86fe)
                    && s.skeleton.is_empty()
                {
                    s.skeleton = sk.to_string();
                }
            });
        }
        s
    }

    /// The model's file name as the archives key it: `mc01_lod1-rico.rbm`.
    pub fn model_file(&self) -> String {
        let bare = self.model.rsplit(['\\', '/']).next().unwrap_or(&self.model);
        if bare.to_ascii_lowercase().ends_with(".rbm") {
            bare.to_string()
        } else {
            format!("{bare}.rbm")
        }
    }
}

/// `settings\grapplingparams.bin`.
#[derive(Clone, Debug, PartialEq)]
pub struct GrapplingParams {
    /// Pull on a hooked object (N), `pull_force` = 500.
    pub pull_force: f32,
    /// Pull on the player when the hook holds (N), `pull_force_character` = 850.
    pub pull_force_character: f32,
    /// `reel_in_speed` = 40 (m/s): how fast the wire winds in.
    pub reel_in_speed: f32,
    /// `wire_width` = 0.006 m.
    pub wire_width: f32,
    /// `hook_model` = `Models/Weapons/GAE04/WEA04-Grapplinghook.lod` (the model lives
    /// in `general.blz`).
    pub hook_model: String,
    /// `#72104687` = 8100: fits "maximum range squared" (90 m) — assumed, not verified.
    pub range: f32,
    /// `character_impulse` = 200 (N·s), found by hashing: the kick the hook's impact gives
    /// a character (in the hook's impact block, `#c42c3b5c`, beside `hook_material` =
    /// "Metal" and `damage` = 76).
    pub character_impulse: f32,
    /// `object_impulse` = 200 (N·s): the same for a physics object.
    pub object_impulse: f32,
    /// `#59e07a13` = 120 (m/s): taken as how fast the hook flies out (assumed: the key
    /// has no name, and 120 is the fitting number among the unnamed ones).
    pub hook_speed: f32,
    /// `#76a95a64` = 0.91 (about cos 24.5°): kept for the detach angle (assumed, unnamed).
    pub detach_cos: f32,
}

impl Default for GrapplingParams {
    fn default() -> Self {
        Self {
            pull_force: 500.0,
            pull_force_character: 850.0,
            reel_in_speed: 40.0,
            wire_width: 0.006,
            hook_model: "Models/Weapons/GAE04/WEA04-Grapplinghook.lod".into(),
            range: 90.0,
            character_impulse: 200.0,
            object_impulse: 200.0,
            hook_speed: 120.0,
            detach_cos: 0.91,
        }
    }
}

impl GrapplingParams {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let file = PropertyFile::parse_any(data)?;
        Ok(Self::from_property(&file))
    }

    pub fn from_property(file: &PropertyFile) -> Self {
        let mut p = Self::default();
        let num = |n: &Node, name: &str| n.float(name);
        for root in &file.nodes {
            root.walk(&mut |n| {
                if let Some(v) = num(n, "pull_force") {
                    p.pull_force = v;
                }
                if let Some(v) = num(n, "pull_force_character") {
                    p.pull_force_character = v;
                }
                if let Some(v) = num(n, "reel_in_speed") {
                    p.reel_in_speed = v;
                }
                if let Some(v) = num(n, "wire_width") {
                    p.wire_width = v;
                }
                if let Some(v) = num(n, "character_impulse") {
                    p.character_impulse = v;
                }
                if let Some(v) = num(n, "object_impulse") {
                    p.object_impulse = v;
                }
                if let Some(Variant::Int(v)) = n.value_hash(0x59e0_7a13) {
                    p.hook_speed = *v as f32;
                }
                if let Some(Variant::Float(v)) = n.value_hash(0x59e0_7a13) {
                    p.hook_speed = *v;
                }
                if let Some(Variant::Float(v)) = n.value_hash(0x76a9_5a64) {
                    p.detach_cos = *v;
                }
                if let Some(m) = n.str("hook_model") {
                    p.hook_model = m.to_string();
                }
                if let Some(Variant::Int(sq)) = n.value_hash(0x7210_4687) {
                    p.range = (*sq as f32).max(0.0).sqrt();
                }
                if let Some(Variant::Float(sq)) = n.value_hash(0x7210_4687) {
                    p.range = sq.max(0.0).sqrt();
                }
            });
        }
        p
    }

    /// The hook's `.lod` file name, bare: `wea04-grapplinghook.lod`.
    pub fn hook_lod(&self) -> String {
        self.hook_model
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&self.hook_model)
            .to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs JC2_TEST_DIR with `player.bin` and `grapplingparams.bin` in the dropzone
    /// (`jc2tool cat <game> 'settings\player.bin' > dropzone/player.bin`, likewise for
    /// `settings\grapplingparams.bin`).
    #[test]
    fn player_and_grapple_settings() {
        let Ok(dir) = std::env::var("JC2_TEST_DIR") else {
            return;
        };
        let fs = crate::GameFs::open(dir).unwrap();
        if !fs.contains("player.bin") || !fs.contains("grapplingparams.bin") {
            return;
        }
        let p = PlayerSettings::parse(&fs.read_inflated("player.bin").unwrap()).unwrap();
        assert_eq!(p.model_file(), "mc01_lod1-rico.rbm");
        assert!(
            p.skeleton.to_ascii_lowercase().ends_with("biped.bsk"),
            "{p:?}"
        );
        let g = GrapplingParams::parse(&fs.read_inflated("grapplingparams.bin").unwrap()).unwrap();
        assert_eq!(g.pull_force, 500.0);
        assert_eq!(g.pull_force_character, 850.0);
        assert_eq!(g.reel_in_speed, 40.0);
        assert_eq!(g.hook_lod(), "wea04-grapplinghook.lod");
        assert!((g.range - 90.0).abs() < 1e-3, "{}", g.range);
        assert_eq!(g.character_impulse, 200.0);
        assert_eq!(g.object_impulse, 200.0);
        assert_eq!(g.hook_speed, 120.0);
        assert!((g.detach_cos - 0.91).abs() < 1e-6);
    }
}
