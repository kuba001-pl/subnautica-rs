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
    /// `DayNightCycle.GetDayScalar()`: the clock as a fraction of the day
    /// (0 midnight, 0.5 noon). `DayNightLight` curves run on it.
    pub day_scalar: f32,
    /// Towards the directional light (the sun by day, a moon-like light at
    /// night; `uSkyLight`'s light, which always points down).
    pub to_sun: Vec3,
    /// Towards the sun itself: `uSkyManager.SunDir`, a plain hour angle that
    /// sets at night. The sky, its day/night factors, the water fog and the
    /// water surface use this one.
    pub to_sun_water: Vec3,
    /// Sun colour × intensity as the game's scripts see it
    /// (`uSkyLight.GetLightColor`: colour made linear, × intensity): the
    /// water fog and light shafts (`_UweFogLightColor`) use this.
    pub sun: Vec3,
    /// The sun as Unity's light passes see it (`_LightColor`): with
    /// `GraphicsSettings.m_LightsUseLinearIntensity` off (the game's
    /// setting), colour × intensity in gamma space, then made linear.
    pub sun_light: Vec3,
    pub top_ambient: Vec3,
    /// `_UweBottomAmbientColor`: the ground colour gradient, as the top.
    pub bottom_ambient: Vec3,
    /// Unity's own ambient (`RenderSettings.ambientSkyColor`, flat mode):
    /// `uSkyLight.CurrentSkyColor`, linear, without the ambient factor.
    pub unity_ambient: Vec3,
    pub fog_color: Vec3,
    pub fog_density: f32,
    /// `uSkyManager.GetMeanSkyColor()`, linear.
    pub mean_sky: Vec3,
    /// `_UweLocalLightScalar` (`DayNightCycle.UpdateAtmosphere`): how bright
    /// the light is, 0 (night) … 1, linear. Objects fade between their day
    /// and night glow with it.
    pub local_light: f32,
    /// For the sky dome (Unity coordinates, raw sRGB colours).
    pub dome: DomeInputs,
}

/// Intermediate values of the sky system the sky dome uses.
#[derive(Clone, Copy, Debug, Default)]
pub struct DomeInputs {
    /// `uSkyManager.SunDir`: towards the sun along the light's path, Unity
    /// coordinates.
    pub sun_dir: Vec3,
    pub day: f32,
    pub sunset: f32,
    pub night: f32,
    /// `uSkyLight.CurrentLightColor` (sun colour gradient), sRGB.
    pub light_colour: Vec3,
    /// `uSkyLight.CurrentSkyColor` (sky gradient with its colour offset and
    /// the exposure), sRGB.
    pub sky_colour: Vec3,
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

/// Unity's `Mathf.LinearToGammaSpace` (sRGB encoding).
pub fn to_gamma(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
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

pub fn linear(c: Vec3) -> Vec3 {
    Vec3::new(to_linear(c.x), to_linear(c.y), to_linear(c.z))
}

/// The directional light's rotation (uSkyManager.InitSunAndMoon), Unity
/// coordinates.
pub fn light_rotation(m: &SkyManager, timeline: f32) -> Quat {
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
    Quat::from_rotation_y(m.sun_direction.to_radians())
        * Quat::from_rotation_z(m.north_pole_offset.to_radians())
        * Quat::from_rotation_x((angle + 90.0).to_radians())
}

/// The direction the directional light travels, in Unity coordinates.
fn light_direction(m: &SkyManager, timeline: f32) -> Vec3 {
    light_rotation(m, timeline) * Vec3::Z
}

/// `uSkyManager.GetLightDirection()`: the direction of `sunEuler`, which
/// turns 15° per hour (`Timeline × 15° − 90°`) without the light's clamping,
/// in Unity coordinates. The water fog and surface use this one.
fn water_light_direction(m: &SkyManager, timeline: f32) -> Vec3 {
    let q = Quat::from_rotation_y(m.sun_direction.to_radians())
        * Quat::from_rotation_z(m.north_pole_offset.to_radians())
        * Quat::from_rotation_x((timeline * 15.0 - 90.0).to_radians());
    q * Vec3::Z
}

/// `uSkyManager.BetaR` × 1000 (Rayleigh coefficients from the tinted
/// wavelengths).
pub fn beta_r(m: &SkyManager) -> Vec3 {
    let w: [f32; 3] = std::array::from_fn(|i| {
        let (a, b) = (m.wavelengths[i] + 150.0, m.wavelengths[i] - 150.0);
        (a + (b - a) * m.sky_tint[i]) * 1e-9
    });
    let num = 8.0 * std::f32::consts::PI.powi(3) * 0.000_600_218_8f32.powi(2) * 6.105;
    Vec3::from(w.map(|l| 1000.0 * num / (7.635e25 * l.powi(4) * 5.755))) * 1000.0
}

/// `uSkyLight.colorOffset`: a gradient colour shifted by the Rayleigh
/// colour (towards its complement at sunset for the sky), the Rayleigh
/// slider, then × exposure. Raw (sRGB) values.
fn color_offset(
    m: &SkyManager,
    colour: [f32; 4],
    offset: f32,
    rayleigh_offset: f32,
    ground: bool,
    day: f32,
    sunset: f32,
) -> Vec3 {
    let beta = beta_r(m);
    let mut v = Vec3::new(beta.x / 5.81, beta.y / 13.57, beta.z / 33.13) * 0.5;
    if !ground {
        let flipped = (Vec3::ONE - v).abs();
        v = flipped + (v - flipped) * sunset;
    }
    v = Vec3::splat(0.5) + (v - Vec3::splat(0.5)) * day;
    let base = Vec3::new(colour[0], colour[1], colour[2]);
    let mut c = base - Vec3::splat(offset) + (2.0 * offset) * v;
    let slider = m.rayleigh_scattering;
    if slider < 1.0 {
        c *= slider;
    } else {
        let b = c / beta * 4.0;
        c += (b - c) * ((slider - 1.0).max(0.0) / 4.0 * rayleigh_offset);
    }
    c * m.exposure
}

/// `DayNightCycle.GetLocalLightScalar` of the light's intensity and colour
/// (as stored, sRGB), then `Mathf.GammaToLinearSpace`.
fn local_light_scalar(intensity: f32, colour: Vec3) -> f32 {
    let mean = (colour.x + colour.y + colour.z) / 3.0;
    to_linear((intensity * mean * 1.2 - 0.15).clamp(0.0, 1.0))
}

/// The sky at a clock time (hours; the game's clock, see
/// `timeline_from_clock`).
pub fn state(m: &SkyManager, l: &SkyLight, clock_hours: f32) -> SkyState {
    let timeline = timeline_from_clock(clock_hours);
    let forward = light_direction(m, timeline);
    let water = -water_light_direction(m, timeline);
    let light_dir = -forward;
    // `uSkyManager.uMuS` from `SunDir` (inside the manager, its own
    // `GetLightDirection`: the hour angle), not from the light.
    let sun_dir = water;
    let mu = (sun_dir.y.max(-0.1975) * 5.35).atan() / 1.1 + 0.739;
    let day = mu.clamp(0.0, 1.0);
    let sunset = ((mu - 1.0) * (1.5 / m.rayleigh_scattering.powi(4))).clamp(0.0, 1.0);
    let night = (1.0 - day).max(0.0);
    let t01 = timeline / 24.0;

    let intensity = m.exposure * (l.sun_intensity * day + l.moon_intensity * night);
    let c = l.light_color.evaluate(t01);
    let sun = linear(Vec3::new(c[0], c[1], c[2]) * (day + night)) * intensity;
    let sun_light = linear(Vec3::new(c[0], c[1], c[2]) * (day + night) * intensity);

    // `uSkyLight.CurrentSkyColor` / `CurrentGroundColor` (with exposure).
    let sky = color_offset(m, l.sky_color.evaluate(t01), 0.15, 0.7, false, day, sunset);
    let ground = color_offset(
        m,
        l.ground_color.evaluate(t01),
        0.25,
        0.85,
        true,
        day,
        sunset,
    );
    let top_ambient = linear(sky) * l.ambient_light;
    let bottom_ambient = linear(ground) * l.ambient_light;

    let f = m.sky_fog_color.evaluate(t01);
    let mean = m.mean_sky_color.evaluate(t01);
    SkyState {
        timeline,
        day_scalar: (clock_hours / 24.0).rem_euclid(1.0),
        to_sun: Vec3::new(light_dir.x, light_dir.y, -light_dir.z),
        to_sun_water: Vec3::new(water.x, water.y, -water.z),
        sun,
        sun_light,
        top_ambient,
        bottom_ambient,
        unity_ambient: linear(sky),
        fog_color: linear(Vec3::new(f[0], f[1], f[2])),
        fog_density: m.sky_fog_density,
        mean_sky: linear(Vec3::new(mean[0], mean[1], mean[2])),
        local_light: local_light_scalar(intensity, Vec3::new(c[0], c[1], c[2]) * (day + night)),
        dome: DomeInputs {
            sun_dir,
            day,
            sunset,
            night,
            light_colour: Vec3::new(c[0], c[1], c[2]),
            sky_colour: sky,
        },
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
            dome: Default::default(),
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
    fn water_sun_turns_with_the_hour() {
        let m = manager();
        // Noon: straight down; 06:00 on the horizon; midnight straight up.
        assert!((water_light_direction(&m, 12.0) - Vec3::NEG_Y).length() < 1e-5);
        assert!(water_light_direction(&m, 6.0).y.abs() < 1e-5);
        assert!((water_light_direction(&m, 0.0) - Vec3::Y).length() < 1e-5);
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
