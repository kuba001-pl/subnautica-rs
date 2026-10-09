//! Where Lifepod 5 lands in a new game: `RandomStart.GetRandomStartPoint`
//! draws x and z uniformly in ±2,048 m (y = 0) until the start map's pixel
//! there is green (> 0.5), at most 1,000 times, else the origin
//! (`docs/formats/unity.md` § Scenes). The game draws with Unity's unseeded
//! `Random`; we draw with our own seeded generator, so any valid point is
//! the game's behaviour and the same seed gives the same point.

use crate::slots::SlotRng;

/// Half the world's width covered by the map, metres.
pub const WORLD_EXTENTS: f32 = 2048.0;

/// Tries before the game gives up and uses the origin.
pub const MAX_TRIES: usize = 1000;

/// The `validStartPointTexture`'s green channel.
#[derive(Clone, Debug)]
pub struct StartMap {
    width: usize,
    height: usize,
    /// Row y = 0 first, as `Texture2D.GetPixel` counts.
    green: Vec<u8>,
    /// Wrap mode repeat (else clamp) for coordinates outside the map.
    repeat: bool,
}

impl StartMap {
    /// `green`: one byte per pixel, row y = 0 first. `None` if the sizes
    /// disagree or the map is empty.
    pub fn new(width: usize, height: usize, green: Vec<u8>, repeat: bool) -> Option<StartMap> {
        (width > 0 && height > 0 && green.len() == width.checked_mul(height)?).then_some(StartMap {
            width,
            height,
            green,
            repeat,
        })
    }

    fn pixel(&self, x: i64, y: i64) -> u8 {
        let wrap = |v: i64, n: usize| {
            let n = n as i64;
            if self.repeat {
                v.rem_euclid(n)
            } else {
                v.clamp(0, n - 1)
            }
        };
        let (x, y) = (wrap(x, self.width), wrap(y, self.height));
        self.green[y as usize * self.width + x as usize]
    }

    /// `RandomStart.IsStartPointValid`: the pixel under (x, z).
    pub fn is_valid(&self, x: f32, z: f32) -> bool {
        let u = ((x + WORLD_EXTENTS) / (2.0 * WORLD_EXTENTS)).clamp(0.0, 1.0);
        let v = ((z + WORLD_EXTENTS) / (2.0 * WORLD_EXTENTS)).clamp(0.0, 1.0);
        let px = (u * self.width as f32) as i64;
        let py = (v * self.height as f32) as i64;
        // Colour.g > 0.5 with g = byte / 255.
        f32::from(self.pixel(px, py)) / 255.0 > 0.5
    }

    /// Share of the map's pixels that are valid starts.
    pub fn valid_share(&self) -> f32 {
        let n = self
            .green
            .iter()
            .filter(|&&g| f32::from(g) / 255.0 > 0.5)
            .count();
        n as f32 / self.green.len() as f32
    }

    /// The start point for world `seed`, and how many draws it took
    /// (`MAX_TRIES` + 1 when none was valid: the origin).
    pub fn random_start(&self, seed: u64) -> ([f32; 3], usize) {
        let mut rng = SlotRng::new(seed, "RandomStart", 0);
        for i in 0..MAX_TRIES {
            let x = -WORLD_EXTENTS + rng.value() * 2.0 * WORLD_EXTENTS;
            let z = -WORLD_EXTENTS + rng.value() * 2.0 * WORLD_EXTENTS;
            if self.is_valid(x, z) {
                return ([x, 0.0, z], i + 1);
            }
        }
        ([0.0; 3], MAX_TRIES + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4×4 map, only pixel (x 3, y 0) valid: x in [1024, 2048), z in
    /// [−2048, −1024).
    fn map(repeat: bool) -> StartMap {
        let mut green = vec![0u8; 16];
        green[3] = 200;
        StartMap::new(4, 4, green, repeat).unwrap()
    }

    #[test]
    fn pixels_are_counted_from_the_bottom_left() {
        let m = map(false);
        assert!(m.is_valid(1500.0, -1500.0));
        assert!(!m.is_valid(-1500.0, -1500.0));
        assert!(!m.is_valid(1500.0, 1500.0));
        // 0.5 is not enough: 128/255 is.
        let half = StartMap::new(1, 1, vec![127], false).unwrap();
        assert!(!half.is_valid(0.0, 0.0));
        assert!(
            StartMap::new(1, 1, vec![128], false)
                .unwrap()
                .is_valid(0.0, 0.0)
        );
    }

    #[test]
    fn the_far_edge_wraps_or_clamps() {
        // x = 2048 gives pixel 4: clamped to 3 (valid) or repeated to 0.
        assert!(map(false).is_valid(2048.0, -2000.0));
        assert!(!map(true).is_valid(2048.0, -2000.0));
    }

    #[test]
    fn draws_land_on_valid_pixels_and_repeat_per_seed() {
        let m = map(false);
        assert_eq!(m.valid_share(), 1.0 / 16.0);
        for seed in 0..50 {
            let (p, tries) = m.random_start(seed);
            assert!(tries <= MAX_TRIES);
            assert!(m.is_valid(p[0], p[2]), "{p:?}");
            assert_eq!(p[1], 0.0);
            assert_eq!(m.random_start(seed), (p, tries));
        }
        assert_ne!(m.random_start(1).0, m.random_start(2).0);
        // Nothing valid: the origin after every try.
        let none = StartMap::new(2, 2, vec![0; 4], false).unwrap();
        assert_eq!(none.random_start(7), ([0.0; 3], MAX_TRIES + 1));
        assert!(StartMap::new(2, 2, vec![0; 3], false).is_none());
    }
}
