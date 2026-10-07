//! The game's sky values at a time of day (from `uSkyManager` and
//! `uSkyLight` in the main scene): sun direction and colour, the top ambient
//! colour, sky fog. Follows the game's classes; see `docs/formats/water.md`
//! § Sky. Eclipses (the planet in front of the sun) are ignored.

use bevy::prelude::*;
use sn_unity::{SkyLight, SkyManager};

/// Clock time of a new game: 09:36 (`DayNightCycle.dateOrigin`).
pub const NEW_GAME_HOURS: f32 = 9.6;

/// What the rest of the client needs from the sky, in Bevy coordinates and
/// linear colour (the game's units: 1 = a light of intensity 1).
#[derive(Resource, Clone, Copy, Debug)]
pub struct SkyState {
    /// The sky system's time (hours), derived from the clock.
    pub timeline: f32,
    pub to_sun: Vec3,
    /// Sun colour × intensity.
    pub sun: Vec3,
    pub top_ambient: Vec3,
    pub fog_color: Vec3,
    pub fog_density: f32,
    /// `uSkyManager.GetMeanSkyColor()`, linear.
    pub mean_sky: Vec3,
}

/// The game's day/night cycle maps clock time to the sky's timeline so that
/// the day (sunrise 0.125 → sunset 0.875 of the clock) fills 6 h → 18 h.
pub fn timeline_from_clock(hours: f32) -> f32 {
    let (rise, set) = (0.125f32, 0.875f32);
    let day = (hours / 24.0).rem_euclid(1.0);
    let span = set - rise;
    let t = if day > rise && day < set {
        (day - rise) / span * 0.5 + 0.25
    } else {
        let d = if day < set { day + 1.0 } else { day };
        let t = (d - set) / (1.0 - span) * 0.5 + 0.75;
        if t > 1.0 { t - 1.0 } else { t }
    };
    t * 24.0
}

/// Unity's `Color.linear` for one channel.
fn to_linear(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear(c: Vec3) -> Vec3 {
    Vec3::new(to_linear(c.x), to_linear(c.y), to_linear(c.z))
}

/// The direction the sun's light travels, in Unity coordinates.
fn light_direction(m: &SkyManager, timeline: f32) -> Vec3 {
    let t = timeline / 24.0;
    let angle = if timeline < 6.0 {
        let f = (t * 4.0).clamp(0.0, 1.0);
        -m.sun_max_angle * f
    } else if timeline > 18.0 {
        let f = ((t - 0.75) * 4.0).clamp(0.0, 1.0);
        m.sun_max_angle * (1.0 - f)
    } else {
        let f = (t * 2.0 - 0.5).clamp(0.0, 1.0);
        -m.sun_max_angle + 2.0 * m.sun_max_angle * f
    };
    // Unity's Quaternion.Euler(x, y, z) = Ry · Rx · Rz.
    let q = Quat::from_rotation_y(m.sun_direction.to_radians())
        * Quat::from_rotation_z(m.north_pole_offset.to_radians())
        * Quat::from_rotation_x((angle + 90.0).to_radians());
    q * Vec3::Z
}

/// `uSkyManager.BetaR` × 1000 (Rayleigh coefficients from the tinted
/// wavelengths).
fn beta_r(m: &SkyManager) -> Vec3 {
    let w: [f32; 3] = std::array::from_fn(|i| {
        let (a, b) = (m.wavelengths[i] + 150.0, m.wavelengths[i] - 150.0);
        (a + (b - a) * m.sky_tint[i]) * 1e-9
    });
    let num = 8.0 * std::f32::consts::PI.powi(3) * 0.000_600_218_8f32.powi(2) * 6.105;
    Vec3::from(w.map(|l| 1000.0 * num / (7.635e25 * l.powi(4) * 5.755))) * 1000.0
}

pub fn state(m: &SkyManager, l: &SkyLight, timeline: f32) -> SkyState {
    let forward = light_direction(m, timeline);
    let sun_dir = -forward;
    let mu = (sun_dir.y.max(-0.1975) * 5.35).atan() / 1.1 + 0.739;
    let day = mu.clamp(0.0, 1.0);
    let sunset = ((mu - 1.0) * (1.5 / m.rayleigh_scattering.powi(4))).clamp(0.0, 1.0);
    let night = (1.0 - day).max(0.0);
    let t01 = timeline / 24.0;

    let intensity = m.exposure * (l.sun_intensity * day + l.moon_intensity * night);
    let c = l.light_color.evaluate(t01);
    let sun = linear(Vec3::new(c[0], c[1], c[2]) * (day + night)) * intensity;

    // `uSkyLight.colorOffset` (sky colour, not ground).
    let beta = beta_r(m);
    let mut v = Vec3::new(beta.x / 5.81, beta.y / 13.57, beta.z / 33.13) * 0.5;
    let flipped = (Vec3::ONE - v).abs();
    v = flipped + (v - flipped) * sunset;
    v = Vec3::splat(0.5) + (v - Vec3::splat(0.5)) * day;
    let s = l.sky_color.evaluate(t01);
    let (offset, rayleigh_offset) = (0.15, 0.7);
    let base = Vec3::new(s[0], s[1], s[2]);
    let mut sky = base - Vec3::splat(offset) + (2.0 * offset) * v;
    let slider = m.rayleigh_scattering;
    if slider < 1.0 {
        sky *= slider;
    } else {
        let b = sky / beta * 4.0;
        sky += (b - sky) * ((slider - 1.0).max(0.0) / 4.0 * rayleigh_offset);
    }
    let sky = sky * m.exposure;
    let top_ambient = linear(sky) * l.ambient_light;

    let f = m.sky_fog_color.evaluate(t01);
    let mean = m.mean_sky_color.evaluate(t01);
    SkyState {
        timeline,
        to_sun: Vec3::new(sun_dir.x, sun_dir.y, -sun_dir.z),
        sun,
        top_ambient,
        fog_color: linear(Vec3::new(f[0], f[1], f[2])),
        fog_density: m.sky_fog_density,
        mean_sky: linear(Vec3::new(mean[0], mean[1], mean[2])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_to_timeline() {
        // Noon stays noon, sunrise (03:00 on the clock) is 06:00 on the sky.
        assert!((timeline_from_clock(12.0) - 12.0).abs() < 1e-4);
        assert!((timeline_from_clock(3.0) - 6.0).abs() < 1e-3);
        assert!((timeline_from_clock(21.0) - 18.0).abs() < 1e-3);
        // A new game: 09:36 → ≈ 10.4 h.
        assert!((timeline_from_clock(NEW_GAME_HOURS) - 10.4).abs() < 1e-3);
    }

    fn manager() -> SkyManager {
        let g = sn_unity::Gradient {
            keys: [[1.0; 4]; 8],
            color_times: [0; 8],
            alpha_times: [0; 8],
            mode: 0,
            color_keys: 1,
            alpha_keys: 1,
        };
        SkyManager {
            timeline: 12.0,
            use_time_of_day: true,
            sun_direction: -141.0,
            sun_max_angle: 65.0,
            north_pole_offset: 0.0,
            exposure: 1.0,
            rayleigh_scattering: 1.0,
            mie_scattering: 1.0,
            sun_anisotropy: 0.76,
            sun_size: 1.0,
            wavelengths: [680.0, 550.0, 440.0],
            sky_tint: [0.5, 0.5, 0.5, 1.0],
            ground_color: [0.0; 4],
            sky_fog_density: 0.0002,
            sky_fog_color: g.clone(),
            mean_sky_color: g,
        }
    }

    #[test]
    fn sun_path() {
        let m = manager();
        // Noon: straight down. 06:00: 25° above the horizon (90° − 65°).
        assert!((light_direction(&m, 12.0) - Vec3::NEG_Y).length() < 1e-5);
        let morning = light_direction(&m, 6.0);
        assert!(
            (morning.y + 25f32.to_radians().sin()).abs() < 1e-5,
            "{morning}"
        );
    }

    #[test]
    fn default_wavelengths_give_the_reference_beta() {
        // The class's fallback when there is no manager: (5.81, 13.57, 33.13).
        let b = beta_r(&manager());
        assert!(
            (b.x - 5.81).abs() < 0.05 && (b.y - 13.57).abs() < 0.1 && (b.z - 33.13).abs() < 0.2,
            "{b}"
        );
    }
}
