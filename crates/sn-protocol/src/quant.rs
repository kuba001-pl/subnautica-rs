//! Quantization: the sim's `f64` values ↔ the wire's small integers.
//! Lossy on purpose; the receiver only ever sees the quantized value, so
//! "the same state on both machines" means the same quantized value
//! (`docs/formats/protocol.md` § Quantization). Out-of-range values
//! saturate; NaN becomes 0.

/// Metres → `f32`: round to nearest. Below 4,096 m from the origin the
/// error is at most 2⁻¹³ m (0.12 mm: half of `f32`'s spacing there),
/// below 2,048 m at most 2⁻¹⁴ m.
pub fn position(v: f64) -> f32 {
    v as f32
}

/// Velocity, metres per second → centimetres per second (±327.67 m/s).
pub fn velocity(v: f64) -> i16 {
    fixed(v, 100.0)
}

pub fn velocity_back(v: i16) -> f64 {
    f64::from(v) / 100.0
}

/// An angle in degrees → 1/65,536 of a turn (about 0.0055°), wrapped.
pub fn turn(degrees: f64) -> u16 {
    let t = (degrees.rem_euclid(360.0) / 360.0 * 65_536.0).round();
    // 360° rounds to 65,536, which is 0 again; NaN becomes 0.
    (t as u32 & 0xffff) as u16
}

/// Degrees, 0 ≤ result < 360.
pub fn turn_back(v: u16) -> f64 {
    f64::from(v) * 360.0 / 65_536.0
}

/// A signed value at a fixed step (`scale` units per wire step is
/// `1/scale`): `round(v · scale)`, saturating at ±32,767 (−32,768 is
/// never produced, so the range is symmetric).
pub fn fixed(v: f64, scale: f64) -> i16 {
    ((v * scale).round() as i16).max(-i16::MAX)
}

pub fn fixed_back(v: i16, scale: f64) -> f64 {
    f64::from(v) / scale
}

/// Pitch, degrees → hundredths of a degree.
pub const PITCH_SCALE: f64 = 100.0;

/// A unit quaternion's component → ±32,767.
pub const QUATERNION_SCALE: f64 = 32_767.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_within_a_quarter_millimetre_out_to_4096_m() {
        // A seeded walk over the range, plus the edges.
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut worst = 0.0f64;
        for i in 0..200_000 {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let u = (s >> 11) as f64 / (1u64 << 53) as f64;
            let v = if i < 2 {
                [4095.999, -4095.999][i]
            } else {
                (u * 2.0 - 1.0) * 4095.999
            };
            worst = worst.max((f64::from(position(v)) - v).abs());
        }
        // The plan's target (0.25 mm) and the exact bound.
        assert!(worst <= 0.000_25, "worst {worst}");
        assert!(worst <= 2f64.powi(-13), "worst {worst}");
        assert!((f64::from(position(2_000.123_456_7)) - 2_000.123_456_7).abs() <= 2f64.powi(-14));
    }

    #[test]
    fn velocity_half_a_centimetre_and_saturates() {
        for v in [-10.0, -0.004_9, 0.0, 3.5, 7.6, 7.224_9] {
            assert!((velocity_back(velocity(v)) - v).abs() <= 0.005);
        }
        assert_eq!(velocity(1e9), i16::MAX);
        assert_eq!(velocity(-1e9), -i16::MAX);
        assert_eq!(velocity(f64::NAN), 0);
    }

    #[test]
    fn turns_wrap_and_stay_within_half_a_step() {
        let step = 360.0 / 65_536.0;
        for d in [
            0.0, 1.0, 89.999, 180.0, 359.999_9, 360.0, -90.0, 725.5, -1e6,
        ] {
            let back = turn_back(turn(d));
            let want = f64::rem_euclid(d, 360.0);
            let err = (back - want + 180.0).rem_euclid(360.0) - 180.0;
            assert!(err.abs() <= step / 2.0 + 1e-9, "{d}: {back}");
            assert!((0.0..360.0).contains(&back));
        }
        assert_eq!(turn(f64::NAN), 0);
    }

    #[test]
    fn fixed_steps_round_and_saturate_symmetrically() {
        assert_eq!(fixed(87.0, PITCH_SCALE), 8_700);
        assert_eq!(fixed(-87.004, PITCH_SCALE), -8_700);
        assert_eq!(fixed(1e9, 1.0), i16::MAX);
        assert_eq!(fixed(-1e9, 1.0), -i16::MAX);
        assert!((fixed_back(fixed(0.123_456, 1000.0), 1000.0) - 0.123).abs() < 1e-12);
    }
}
