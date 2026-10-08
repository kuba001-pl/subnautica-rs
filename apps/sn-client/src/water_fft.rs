//! The "High" water quality's waves (M8c4): the game's initial wave
//! spectrum (`WaterDisplacementGenerator.ComputeInitialSpectrum`, a Phillips
//! spectrum), computed once on the CPU. The GPU then advances it in time and
//! turns it into displacement every frame (`water_fft.wgsl`). See
//! `docs/formats/water.md` § High quality waves.
//!
//! The game draws its random numbers from Unity's unseeded generator, so its
//! waves differ on every run; ours use a fixed seed (same statistics).

use sn_unity::FftWaves;

/// Size of the displacement map and the FFT.
pub const N: usize = 512;
/// Row length of the initial spectrum (the game's layout: N + 4).
pub const IN_WIDTH: usize = N + 4;
/// Gravity, cm/s².
const GRAVITY: f32 = 981.0;

/// A small deterministic random generator (xorshift64*), uniform in [0, 1).
struct Random(u64);

impl Random {
    fn value(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (x >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The game's `Gauss()`: Box–Muller.
    fn gauss(&mut self) -> f32 {
        let a = self.value().max(1e-6);
        let b = self.value();
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    }
}

/// The game's `Phillips(K, W, v, a, dirDepend, gravity)`.
fn phillips(w: &FftWaves, k: [f32; 2], wind: [f32; 2], amplitude: f32) -> f32 {
    let l = w.wind_speed * w.wind_speed / GRAVITY;
    let k2 = k[0] * k[0] + k[1] * k[1];
    let kw = k[0] * wind[0] + k[1] * wind[1];
    let mut p = amplitude * (-1.0 / (l * l * k2)).exp() * (kw * kw) / (k2 * k2 * k2);
    if kw < 0.0 {
        p *= 1.0 - w.wind_dependency;
    }
    p * (-k2 * w.min_wave_size * w.min_wave_size).exp()
}

/// The initial spectrum `h0` and the angular frequencies `ω`, laid out as
/// the game's buffers: rows of `IN_WIDTH`, `N + 1` rows. `patch_length` in
/// cm. (The game passes sequence length 0, so `ω` is not quantised.)
pub fn initial_spectrum(w: &FftWaves, patch_length: f32, seed: u64) -> (Vec<[f32; 2]>, Vec<f32>) {
    let count = IN_WIDTH * (N + 1);
    let mut h0 = vec![[0.0f32; 2]; count];
    let mut omega = vec![0.0f32; count];
    let mut random = Random(seed.max(1));
    let amplitude = w.phillips_amplitude * 1e-7;
    let angle = w.wind_angle.to_radians();
    let wind = [angle.cos(), angle.sin()];
    let step = std::f32::consts::TAU / patch_length;
    let half = std::f32::consts::FRAC_1_SQRT_2;
    for i in 0..=N {
        let ky = (i as f32 - N as f32 / 2.0) * step;
        for j in 0..=N {
            let kx = (j as f32 - N as f32 / 2.0) * step;
            // As the game: zero on both axes (not just at the origin).
            let amp = if kx != 0.0 && ky != 0.0 {
                phillips(w, [kx, ky], wind, amplitude).sqrt()
            } else {
                0.0
            };
            let index = i * IN_WIDTH + j;
            h0[index] = [amp * random.gauss() * half, amp * random.gauss() * half];
            omega[index] = (GRAVITY * (kx * kx + ky * ky).sqrt()).sqrt();
        }
    }
    (h0, omega)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waves() -> FftWaves {
        FftWaves {
            choppy_scale: 1.3,
            min_wave_size: 0.01,
            phillips_amplitude: 0.35,
            wind_angle: 45.0,
            wind_speed: 600.0,
            wind_dependency: 0.07,
        }
    }

    #[test]
    fn spectrum_follows_the_game() {
        let (h0, omega) = initial_spectrum(&waves(), 2000.0, 7);
        assert_eq!(h0.len(), IN_WIDTH * (N + 1));
        // The axes through k = 0 carry nothing.
        let centre = N / 2;
        for j in 0..=N {
            assert_eq!(h0[centre * IN_WIDTH + j], [0.0, 0.0]);
            assert_eq!(h0[j * IN_WIDTH + centre], [0.0, 0.0]);
        }
        // ω = √(g |k|).
        let step = std::f32::consts::TAU / 2000.0;
        let (i, j) = (centre + 3, centre + 4);
        let k = 5.0 * step;
        assert!((omega[i * IN_WIDTH + j] - (GRAVITY * k).sqrt()).abs() < 1e-4);
        // Waves against the wind keep 1 − dependency of their energy; none
        // across it.
        let w = waves();
        let wind = [std::f32::consts::FRAC_1_SQRT_2; 2];
        let along = phillips(&w, [0.01, 0.01], wind, 1.0);
        let against = phillips(&w, [-0.01, -0.01], wind, 1.0);
        assert!(along > 0.0 && ((against / along) - 0.93).abs() < 1e-5);
        assert!(phillips(&w, [0.01, -0.01], wind, 1.0).abs() < 1e-12);
    }

    #[test]
    fn gauss_is_standard_normal() {
        let mut r = Random(42);
        let n = 100_000;
        let (mut sum, mut sq) = (0.0f64, 0.0f64);
        for _ in 0..n {
            let g = f64::from(r.gauss());
            sum += g;
            sq += g * g;
        }
        let mean = sum / f64::from(n);
        let var = sq / f64::from(n) - mean * mean;
        assert!(
            mean.abs() < 0.02 && (var - 1.0).abs() < 0.03,
            "{mean} {var}"
        );
    }
}
