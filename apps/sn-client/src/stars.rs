//! uSky's star catalogue (`StarsData`, a text asset in the game's
//! resources): 9110 stars of six floats each — position x, z, y, colour r,
//! g, b. The game keeps those with luminance (0.22, 0.707, 0.071 · colour)
//! between 0.0162 and 2.2 and places them on a sphere of radius 990
//! (`StarField.InitializeStarfield`; `docs/formats/sky.md` § Stars).

/// Radius of the star sphere and the size of a star's quad (990 / 100).
pub const RADIUS: f32 = 990.0;
pub const QUAD_SIZE: f32 = 9.9;
const STARS: usize = 9110;

/// One star for the GPU: position (Unity coordinates, on the sphere),
/// luminance; colour, unused. Must match `Star` in `stars.wgsl`.
pub type GpuStar = [f32; 8];

/// The stars the game draws, from the catalogue's bytes.
pub fn parse(bytes: &[u8]) -> Result<Vec<GpuStar>, String> {
    if bytes.len() < STARS * 24 {
        return Err(format!(
            "star catalogue: {} bytes, expected {}",
            bytes.len(),
            STARS * 24
        ));
    }
    let f = |i: usize| f32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let mut out = Vec::new();
    for s in 0..STARS {
        let o = s * 24;
        // Stored x, z, y; then scaled by (−1, 1, −1).
        let (x, z, y) = (f(o), f(o + 4), f(o + 8));
        let (r, g, b) = (f(o + 12), f(o + 16), f(o + 20));
        let lum = r * 0.22 + g * 0.707 + b * 0.071;
        if !(0.0162..=2.2).contains(&lum) {
            continue;
        }
        out.push([-x * RADIUS, y * RADIUS, -z * RADIUS, lum, r, g, b, 0.0]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_game_s_stars() {
        let mut bytes = vec![0u8; STARS * 24];
        let mut put = |star: usize, v: [f32; 6]| {
            for (k, x) in v.iter().enumerate() {
                bytes[star * 24 + k * 4..][..4].copy_from_slice(&x.to_le_bytes());
            }
        };
        // Star 0: x 0.1, z 0.2, y 0.3, white → kept, axes reordered.
        put(0, [0.1, 0.2, 0.3, 1.0, 1.0, 1.0]);
        // Star 1: too faint; star 2: too bright.
        put(1, [0.0, 1.0, 0.0, 0.01, 0.01, 0.01]);
        put(2, [0.0, 1.0, 0.0, 3.0, 3.0, 3.0]);
        let stars = parse(&bytes).unwrap();
        assert_eq!(stars.len(), 1);
        let s = stars[0];
        assert!((s[0] + 0.1 * RADIUS).abs() < 1e-3);
        assert!((s[1] - 0.3 * RADIUS).abs() < 1e-3);
        assert!((s[2] + 0.2 * RADIUS).abs() < 1e-3);
        assert!((s[3] - 0.998).abs() < 1e-4);
        assert!(parse(&bytes[..100]).is_err());
    }
}
