//! Bounded regular CPlayer accuracy path. AI/modifier policies remain separate.
use bevy::prelude::*;

/// 005911b0 +007141e0, ordinary CPlayerAimControl with no assisted target.
/// Ordinary movement does not supply a guessed accuracy multiplier here.
pub(super) fn regular_player_maximum(weapon: &jc2_formats::weapons::Weapon) -> f32 {
    if weapon.aim_type == 1 {
        1.0
    } else {
        weapon.accuracy.clamp(0.0, 1.0)
    }
}

/// 004dc8a0 world-up look frame. An arc from-Z alone would retain the wrong roll.
pub(super) fn aim_frame(direction: Vec3) -> Quat {
    let right = direction.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(direction).normalize_or(Vec3::Y);
    Quat::from_mat3(&Mat3::from_cols(right, up, -direction))
}

#[derive(Default)]
pub(super) struct PlayerAccuracy {
    owner: Option<Entity>,
    pub value: f32,
}

impl PlayerAccuracy {
    // CCharacter starts at 0; accuracy belongs to the character, not the weapon.
    pub fn update(&mut self, owner: Entity, dt: f32, recovery: f32, maximum: f32) {
        if self.owner != Some(owner) {
            self.owner = Some(owner);
            self.value = 0.0;
        }
        self.value = (self.value + recovery * dt).clamp(0.0, maximum.clamp(0.0, 1.0));
    }

    pub fn fired(&mut self, loss: f32, maximum: f32) {
        self.value = (self.value - loss).clamp(0.0, maximum.clamp(0.0, 1.0));
    }
}

/// Sample policies match native ranges/discreteness; seed/global RNG parity is not claimed.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ShotRandom(u32);
impl Default for ShotRandom {
    fn default() -> Self {
        Self(0x9e37_79b9)
    }
}
impl ShotRandom {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    // 004ebaf0(2)-1: unsigned 32-bit source divided by UINT_MAX, endpoints included.
    fn aim(&mut self) -> f32 {
        (self.next() as f64 * (2.0 / u32::MAX as f64) - 1.0) as f32
    }
    // 0072aae0: CRT rand 0..32767 scaled 2/32767 then shifted by -1.
    fn pellet(&mut self) -> f32 {
        ((self.next() >> 17) as f32 * (2.0 / 32767.0)) - 1.0
    }

    pub fn aim_direction(&mut self, direction: Vec3, angle: f32) -> Vec3 {
        let yaw = self.aim() * angle;
        let pitch = self.aim() * angle;
        let right = direction.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(direction).normalize_or(Vec3::Y);
        // 0054c980 rotates about original aim-relative axes, yaw*pitch.
        (Quat::from_axis_angle(up, yaw) * Quat::from_axis_angle(right, pitch) * direction)
            .normalize()
    }

    pub fn pellet_direction(&mut self, aim_frame: Quat, spread: f32) -> Vec3 {
        // Native draws Z, X, Y; row-vector Rz*Rx*Ry becomes column-vector Ry*Rx*Rz.
        let z = self.pellet() * spread;
        let x = self.pellet() * spread;
        let y = self.pellet() * spread;
        aim_frame
            * (Quat::from_rotation_y(y) * Quat::from_rotation_x(x) * Quat::from_rotation_z(z))
            * Vec3::NEG_Z
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_clamps_and_remains_character_owned_across_switches() {
        let mut state = PlayerAccuracy::default();
        let owner = Entity::from_bits(1);
        state.update(owner, 0.1, 5.0, 1.0);
        assert_eq!(state.value, 0.5);
        state.fired(0.07, 1.0);
        assert!((state.value - 0.43).abs() < 1e-6);
        // Switching to a lower weapon bound clamps, without resetting character state.
        state.update(owner, 0.0, 5.0, 0.4);
        assert_eq!(state.value, 0.4);
        state.update(owner, 10.0, 5.0, 1.0);
        assert_eq!(state.value, 1.0);
        state.fired(20.0, 1.0);
        assert_eq!(state.value, 0.0);
        state.update(owner, 0.2, 5.0, 1.0);
        state.update(Entity::from_bits(2), 0.0, 5.0, 1.0);
        assert_eq!(state.value, 0.0);
        let mut split = PlayerAccuracy::default();
        for _ in 0..10 {
            split.update(owner, 0.01, 5.0, 1.0);
        }
        assert!((split.value - 0.5).abs() < 1e-6);
    }

    #[test]
    fn regular_player_bound_respects_authored_aim_type_bypass() {
        let mut weapon = super::super::fallback_weapon("fixture");
        weapon.accuracy = 0.6;
        assert_eq!(regular_player_maximum(&weapon), 0.6);
        weapon.aim_type = 1;
        assert_eq!(regular_player_maximum(&weapon), 1.0);
        weapon.aim_type = 0;
        weapon.accuracy = 2.0;
        assert_eq!(regular_player_maximum(&weapon), 1.0);
    }

    #[test]
    fn literal_oblique_native_axis_and_roll_fixture() {
        // Independent Rodrigues/native row-matrix reference, fixed samples/seed.
        let mut random = ShotRandom::default();
        let direction = Vec3::new(2.0, -1.0, -3.0).normalize();
        let aim = random.aim_direction(direction, 0.1);
        assert!(aim.distance(Vec3::new(0.5742126, -0.19398788, -0.7953921)) < 1e-6);
        let frame = aim_frame(aim);
        // The world-up frame's right axis has no vertical component.
        assert!((frame * Vec3::X).y.abs() < 1e-6);
        let pellet = random.pellet_direction(frame, 0.15);
        assert!(pellet.distance(Vec3::new(0.4512642, -0.3353713, -0.82697445)) < 1e-6);
    }

    #[test]
    fn two_stage_samples_are_repeatable_bounded_and_independent() {
        let mut rng = ShotRandom::default();
        let mut copy = rng.clone();
        let angle = 2.5_f32.to_radians();
        let mut mean = Vec2::ZERO;
        let mut moment = Vec2::ZERO;
        for _ in 0..10000 {
            let a = rng.aim();
            let b = rng.pellet();
            assert_eq!(a, copy.aim());
            assert_eq!(b, copy.pellet());
            assert!((-1.0..=1.0).contains(&a) && (-1.0..=1.0).contains(&b));
            let pair = Vec2::new(a, b);
            mean += pair;
            moment += pair * pair;
        }
        assert!(mean.abs().max_element() / 10000.0 < 0.02);
        assert!(
            (moment / 10000.0 - Vec2::splat(1.0 / 3.0))
                .abs()
                .max_element()
                < 0.02
        );
        let mut a = ShotRandom::default();
        assert_eq!(a.aim_direction(Vec3::NEG_Z, 0.0), Vec3::NEG_Z);
        assert_eq!(a.pellet_direction(Quat::IDENTITY, 0.0), Vec3::NEG_Z);
        for _ in 0..1000 {
            let aim = a.aim_direction(Vec3::NEG_Z, angle);
            assert!(aim.angle_between(Vec3::NEG_Z) <= angle * 2.0_f32.sqrt() + 1e-5);
            let frame = Quat::from_rotation_arc(Vec3::NEG_Z, aim);
            let pellet = a.pellet_direction(frame, angle);
            assert!(pellet.angle_between(aim) <= angle * 2.0_f32.sqrt() + 1e-5);
            assert!((pellet.length() - 1.0).abs() < 1e-5);
        }
    }

    /// Authored Beretta level-0 data (2.2 degrees, loss 0.07) with the retail player
    /// recovery 5.0 per second: shot-by-shot loss, recovery between shots, and the
    /// resulting aim-stage angle `degrees * (1 - accuracy)`.
    #[test]
    fn authored_pistol_loss_recovery_and_scatter_sequence() {
        let (degrees, loss, recovery) = (2.2_f32, 0.07_f32, 5.0_f32);
        let owner = Entity::from_bits(7);
        let mut state = PlayerAccuracy::default();
        let mut random = ShotRandom::default();
        let mut expected = 0.0_f32;
        for shot in 0..6 {
            // 0.12 s fire interval (authored): recover first, then sample, then lose.
            state.update(owner, 0.12, recovery, 1.0);
            expected = (expected + recovery * 0.12).clamp(0.0, 1.0);
            assert!((state.value - expected).abs() < 1e-6, "shot {shot}");
            let angle = degrees.to_radians() * (1.0 - state.value);
            let aim = random.aim_direction(Vec3::NEG_Z, angle);
            // yaw and pitch are each bounded by the angle; the cone by sqrt(2) * angle.
            assert!(aim.angle_between(Vec3::NEG_Z) <= angle * 2.0_f32.sqrt() + 1e-6);
            state.fired(loss, 1.0);
            expected = (expected - loss).clamp(0.0, 1.0);
            assert!((state.value - expected).abs() < 1e-6, "shot {shot}");
        }
        // 0.6 recovered per interval against 0.07 lost: the pistol saturates at 1 - 0.07.
        assert!((state.value - 0.93).abs() < 1e-6);
        // Fully accurate (clamped to 1) means exactly zero aim scatter.
        state.update(owner, 1.0, recovery, 1.0);
        assert_eq!(state.value, 1.0);
        let angle = degrees.to_radians() * (1.0 - state.value);
        assert_eq!(random.aim_direction(Vec3::NEG_Z, angle), Vec3::NEG_Z);
    }
}
