//! Bounded ordinary foliage alpha state from the original instance/draw chain.
//! Distance/LOD scheduling and the alternate device-state branch are unresolved.

pub const INSTANCE_BYTE_SCALE: f32 = 1.0 / 255.0;
pub const REFERENCE_SCALE: f32 = 1.0 / 510.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoliageAlpha {
    /// Original block +0x3c word; bit5 disables ordinary shader discard.
    pub raw_flags: u32,
    /// Model header +0x0b, initialized255 by009236d0.
    pub instance_byte: u8,
}

impl FoliageAlpha {
    pub fn initialized(raw_flags: u32) -> Self {
        Self {
            raw_flags,
            instance_byte: 255,
        }
    }
    pub fn enabled(self) -> bool {
        self.raw_flags & (1 << 5) == 0
    }
    pub fn fraction(self) -> f32 {
        self.instance_byte as f32 * INSTANCE_BYTE_SCALE
    }
    pub fn reference_code(self) -> i32 {
        (self.fraction() * 128.0).round() as i32
    }
    pub fn reference(self) -> f32 {
        (self.reference_code() * 2 - 1) as f32 * REFERENCE_SCALE
    }
    /// The supported state has an unresolved distance factor, held at1.
    /// Preserve comparison7's strict inequality; zero-byte reference is negative.
    pub fn discards(self, texture_alpha: f32) -> bool {
        let scaled = (texture_alpha + texture_alpha) * self.fraction();
        self.enabled() && scaled < self.reference()
    }
}

/// 00a89b90 last-LOD/missing-next-LOD branch only. None means no submission;
/// it is different from submitting an arbitrary instance byte0.
pub fn last_lod_instance_byte(lod_fraction: u8) -> Option<u8> {
    if lod_fraction == 255 {
        return None;
    }
    let scaled = lod_fraction as f32 * (2.0 / 255.0);
    let fraction = 2.0 - scaled;
    Some(if fraction >= 1.0 {
        255
    } else {
        (fraction * 255.0) as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_last_lod_quantization_and_submission_are_separate_from_byte_zero() {
        for (input, byte, reference) in [
            (0, 255, 128),
            (127, 255, 128),
            (128, 253, 127),
            (192, 125, 63),
            (254, 1, 1),
        ] {
            assert_eq!(last_lod_instance_byte(input), Some(byte));
            let state = FoliageAlpha {
                raw_flags: 64,
                instance_byte: byte,
            };
            assert_eq!(state.reference_code(), reference);
        }
        assert_eq!(last_lod_instance_byte(255), None);
        let zero = FoliageAlpha {
            raw_flags: 64,
            instance_byte: 0,
        };
        assert!(zero.reference() < 0.0);
        assert!(!zero.discards(0.0));
    }
    #[test]
    fn strict_equality_inverse_flag_and_native_reference_keep_exact_boundaries() {
        let alpha = FoliageAlpha::initialized(64);
        assert_eq!(alpha.fraction(), 1.0);
        assert_eq!(alpha.reference(), 0.5);
        assert!(alpha.discards(f32::from_bits(0.25_f32.to_bits() - 1)));
        assert!(!alpha.discards(0.25));
        assert!(!alpha.discards(f32::from_bits(0.25_f32.to_bits() + 1)));
        assert!(!FoliageAlpha::initialized(64 | 32).discards(0.0));
        for byte in 0..=255 {
            let state = FoliageAlpha {
                raw_flags: 64,
                instance_byte: byte,
            };
            // Main doubled-alpha then fade and prez/shadow dp2 of alpha/fade
            // agree for representable normalized half-float texture samples.
            for alpha in [0.0, 0.125, 0.25, 0.5, 1.0] {
                let term = alpha * state.fraction();
                assert_eq!((alpha + alpha) * state.fraction(), term + term);
            }
        }
    }
}
