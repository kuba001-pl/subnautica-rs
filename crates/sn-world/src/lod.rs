//! Which level of a `LODGroup` Unity draws (M7f4f): Unity's documented
//! rule (**hypothesis** for the exact form until compared with the game;
//! `docs/formats/unity.md` § LOD groups).

/// The group's height on screen as a fraction of the screen height, with
/// the quality level's bias: `world_size / 2 / (distance · tan(fov / 2)) ·
/// bias`. `fov_y` is the vertical field of view in degrees. At distance 0
/// the group fills any screen (infinity).
pub fn relative_height(world_size: f32, distance: f32, fov_y: f32, bias: f32) -> f32 {
    let half = (fov_y.to_radians() * 0.5).tan();
    if distance <= 0.0 || half <= 0.0 {
        return f32::INFINITY;
    }
    world_size * 0.5 / (distance * half) * bias
}

/// The level drawn for a relative height `h`: the first whose screen
/// height `h` reaches; `None` (nothing drawn) when `h` is below the last
/// one. Never more detailed than `max_level` (`maximumLODLevel`).
pub fn level(heights: &[f32], h: f32, max_level: usize) -> Option<usize> {
    let first = heights.iter().position(|&t| h >= t)?;
    Some(first.max(max_level).min(heights.len() - 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_falls_with_distance_and_rises_with_bias() {
        // 10 m tall, 60° view: at 8.66 m it is the screen's height.
        let d = 5.0 / (30f32.to_radians().tan());
        assert!((relative_height(10.0, d, 60.0, 1.0) - 1.0).abs() < 1e-5);
        assert!((relative_height(10.0, 2.0 * d, 60.0, 1.0) - 0.5).abs() < 1e-5);
        assert!((relative_height(10.0, 2.0 * d, 60.0, 10.0) - 5.0).abs() < 1e-4);
        assert_eq!(relative_height(10.0, 0.0, 60.0, 1.0), f32::INFINITY);
        // A narrower view makes things bigger.
        assert!(relative_height(10.0, d, 45.0, 1.0) > 1.0);
    }

    #[test]
    fn picks_the_first_level_reached_or_culls() {
        let heights = [0.5, 0.2, 0.05];
        assert_eq!(level(&heights, 2.0, 0), Some(0));
        assert_eq!(level(&heights, 0.5, 0), Some(0));
        assert_eq!(level(&heights, 0.3, 0), Some(1));
        assert_eq!(level(&heights, 0.05, 0), Some(2));
        assert_eq!(level(&heights, 0.01, 0), None);
        assert_eq!(level(&heights, f32::INFINITY, 0), Some(0));
        // maximumLODLevel: never more detailed than it.
        assert_eq!(level(&heights, 2.0, 1), Some(1));
        assert_eq!(level(&heights, 2.0, 9), Some(2));
        assert_eq!(level(&[], 2.0, 0), None);
    }
}
