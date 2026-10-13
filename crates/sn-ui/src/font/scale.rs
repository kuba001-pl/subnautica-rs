//! FreeType's fixed-point arithmetic, as its unhinted glyph loading uses
//! it: a 16.16 scale from `FT_DivFix(size_26_6, units_per_em)`, and each
//! outline coordinate and advance scaled with `FT_MulFix` into 26.6
//! pixels. Both round half away from zero on magnitudes. Written from the
//! documented behaviour (and checked against the game's 2 stored glyphs:
//! every metric equal), not from FreeType's source.

/// `a * b / 65536`, rounded half away from zero (saturating: bad font
/// data must not overflow).
pub fn mul_fix(a: i64, b: i64) -> i64 {
    let negative = (a < 0) != (b < 0);
    let c = (u128::from(a.unsigned_abs()) * u128::from(b.unsigned_abs()) + 0x8000) >> 16;
    let c = i64::try_from(c).unwrap_or(i64::MAX);
    if negative { -c } else { c }
}

/// `a * 65536 / b`, rounded half away from zero; `b == 0` gives the
/// largest value with `a`'s sign (FreeType's guard).
pub fn div_fix(a: i64, b: i64) -> i64 {
    if b == 0 {
        return if a < 0 { -0x7FFF_FFFF } else { 0x7FFF_FFFF };
    }
    let negative = (a < 0) != (b < 0);
    let (a, b) = (u128::from(a.unsigned_abs()), u128::from(b.unsigned_abs()));
    let q = ((a << 16) + (b >> 1)) / b;
    let q = i64::try_from(q).unwrap_or(i64::MAX);
    if negative { -q } else { q }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_rounds_like_freetype() {
        // 90 px on a 1000-unit em.
        let s = div_fix(90 * 64, 1000);
        assert_eq!(s, 377_487);
        // An advance of 733 units: 4222/64 px (4221.44 by float).
        assert_eq!(mul_fix(733, s), 4222);
        assert_eq!(mul_fix(-733, s), -4222);
        assert_eq!(mul_fix(733, -s), -4222);
        assert_eq!(mul_fix(0, s), 0);
        assert_eq!(div_fix(-5760, 1000), -377_487);
        assert_eq!(div_fix(1, 0), 0x7FFF_FFFF);
        assert_eq!(div_fix(-1, 0), -0x7FFF_FFFF);
        // No overflow on absurd input.
        assert_eq!(mul_fix(i64::MAX, i64::MAX), i64::MAX);
        assert_eq!(mul_fix(i64::MIN, 1 << 20), -i64::MAX);
        assert_eq!(div_fix(i64::MAX, 1), i64::MAX);
    }
}
