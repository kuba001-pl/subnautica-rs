//! `WaterSurface` (a MonoBehaviour in the main scene): the ocean surface's
//! waves, look and foam, and Unity's `AnimationCurve`. Field layout
//! generated once from the game's assembly (UnityPy's type-tree generator,
//! dev machine only); see `docs/formats/water.md` § Water surface.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;

/// One key of an `AnimationCurve`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub time: f32,
    pub value: f32,
    pub in_slope: f32,
    pub out_slope: f32,
    /// 0 = not weighted (the only mode handled by `evaluate`).
    pub weighted_mode: i32,
    pub in_weight: f32,
    pub out_weight: f32,
}

/// Unity's `AnimationCurve`: Hermite segments between keys.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationCurve {
    pub keys: Vec<Keyframe>,
    /// Unity's internal wrap modes: 0 ping-pong, 1 repeat, 2 clamp.
    pub pre_infinity: i32,
    pub post_infinity: i32,
}

impl AnimationCurve {
    pub(crate) fn read(r: &mut Reader) -> Result<AnimationCurve> {
        let n = r.count(28)?;
        let mut keys = Vec::with_capacity(n);
        for _ in 0..n {
            keys.push(Keyframe {
                time: r.f32()?,
                value: r.f32()?,
                in_slope: r.f32()?,
                out_slope: r.f32()?,
                weighted_mode: r.i32()?,
                in_weight: r.f32()?,
                out_weight: r.f32()?,
            });
        }
        let pre_infinity = r.i32()?;
        let post_infinity = r.i32()?;
        let _rotation_order = r.i32()?;
        Ok(AnimationCurve {
            keys,
            pre_infinity,
            post_infinity,
        })
    }

    /// The curve's value at `t`, clamped outside its keys (the game's
    /// curves all clamp). Weighted keys are evaluated as unweighted.
    pub fn evaluate(&self, t: f32) -> f32 {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else {
            return 0.0;
        };
        if t <= first.time {
            return first.value;
        }
        if t >= last.time {
            return last.value;
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t > b.time {
                continue;
            }
            let dt = b.time - a.time;
            if dt <= 0.0 {
                return b.value;
            }
            if !a.out_slope.is_finite() || !b.in_slope.is_finite() {
                return a.value; // a step
            }
            let s = (t - a.time) / dt;
            let (s2, s3) = (s * s, s * s * s);
            let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
            let h10 = s3 - 2.0 * s2 + s;
            let h01 = -2.0 * s3 + 3.0 * s2;
            let h11 = s3 - s2;
            return h00 * a.value + h10 * a.out_slope * dt + h01 * b.value + h11 * b.in_slope * dt;
        }
        last.value
    }
}

/// `WaterDisplacementGenerator`'s wave settings (centimetres, seconds).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FftWaves {
    pub choppy_scale: f32,
    /// Waves shorter than this (cm) are damped.
    pub min_wave_size: f32,
    pub phillips_amplitude: f32,
    /// Degrees.
    pub wind_angle: f32,
    /// cm/s.
    pub wind_speed: f32,
    /// How much waves against the wind are kept (0 … 1).
    pub wind_dependency: f32,
}

/// The fields of `WaterSurface` that shape the surface's look. Colours are
/// as stored (sRGB); lengths in centimetres where the game uses them.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterSurface {
    pub surface_shader: PPtr,
    pub update_foam_shader: PPtr,
    pub update_normals_shader: PPtr,
    pub interpolate_shader: PPtr,
    /// Brightness of the sky seen from below, by the camera's depth (m).
    pub under_water_brightness_curve: AnimationCurve,
    pub use_under_water_brightness_curve: bool,
    /// Size of one wave tile (cm).
    pub patch_length: f32,
    /// The "High" quality waves (`WaterDisplacementGenerator`).
    pub waves: FftWaves,
    pub refraction_index: f32,
    pub under_water_refraction_index: f32,
    pub under_water_refraction_depth_scale: f32,
    pub water_offset: f32,
    /// Seconds for the 64 baked wave frames.
    pub sequence_length: f32,
    /// World size of one caustics tile (`WaterCausticsGenerator`).
    pub caustics_size: f32,
    pub num_caustics_frames: i32,
    pub caustics_frames_per_second: i32,
    pub cubic_interpolation: bool,
    pub reflection_color: [f32; 4],
    pub refraction_color: [f32; 4],
    pub back_light_tint: [f32; 4],
    pub sun_reflection_gloss: f32,
    pub sun_reflection_amount: f32,
    pub wave_height_thickness_scale: f32,
    pub foam_texture: PPtr,
    pub foam_mask_texture: PPtr,
    pub foam_smoothing: f32,
    pub foam_rate: f32,
    pub foam_scale: f32,
    pub foam_decay: f32,
    pub foam_distance: f32,
    pub sub_surface_foam_color: [f32; 4],
    pub sub_surface_foam_scale: f32,
    pub displacement_texture_foam_amount_multiplier: f32,
    pub enable_reflection: bool,
    pub screen_space_refraction_index: f32,
    pub time_scale: f32,
}

fn color(r: &mut Reader) -> Result<[f32; 4]> {
    Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?])
}

impl WaterSurface {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<WaterSurface> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let _frustum_dilation = r.f32()?;
        PPtr::read(&mut r)?; // settings (WaterscapeVolume)
        let surface_shader = PPtr::read(&mut r)?;
        let update_foam_shader = PPtr::read(&mut r)?;
        let update_normals_shader = PPtr::read(&mut r)?;
        PPtr::read(&mut r)?; // update spectrum (compute)
        let interpolate_shader = PPtr::read(&mut r)?;
        for _ in 0..3 {
            PPtr::read(&mut r)?; // pack displacement, resize, copy depth
        }
        let under_water_brightness_curve = AnimationCurve::read(&mut r)?;
        let use_under_water_brightness_curve = r.bool_aligned()?;
        // WaterDisplacementGenerator: two shaders, then patch length, choppy
        // scale, min wave size, Phillips amplitude, wind angle, speed,
        // dependency (used by the "High" quality FFT only, except the length).
        PPtr::read(&mut r)?;
        PPtr::read(&mut r)?;
        let patch_length = r.f32()?;
        let waves = FftWaves {
            choppy_scale: r.f32()?,
            min_wave_size: r.f32()?,
            phillips_amplitude: r.f32()?,
            wind_angle: r.f32()?,
            wind_speed: r.f32()?,
            wind_dependency: r.f32()?,
        };
        // WaterCausticsGenerator: two shaders, size, texture size, floor
        // depth, refraction index, colour dispersion, texture.
        PPtr::read(&mut r)?;
        PPtr::read(&mut r)?;
        let caustics_size = r.f32()?;
        r.bytes(4 * 4)?;
        PPtr::read(&mut r)?;
        let _rebuild = r.bool_aligned()?;
        let refraction_index = r.f32()?;
        let under_water_refraction_index = r.f32()?;
        let under_water_refraction_depth_scale = r.f32()?;
        let water_offset = r.f32()?;
        let sequence_length = r.f32()?;
        let _caustics_generation = r.i32()?;
        let _frame_texture_size = r.i32()?;
        let num_caustics_frames = r.i32()?;
        let caustics_frames_per_second = r.i32()?;
        let cubic_interpolation = r.bool_aligned()?;
        let reflection_color = color(&mut r)?;
        let refraction_color = color(&mut r)?;
        let back_light_tint = color(&mut r)?;
        let sun_reflection_gloss = r.f32()?;
        let sun_reflection_amount = r.f32()?;
        let wave_height_thickness_scale = r.f32()?;
        let foam_texture = PPtr::read(&mut r)?;
        let foam_mask_texture = PPtr::read(&mut r)?;
        let foam_smoothing = r.f32()?;
        let foam_rate = r.f32()?;
        let foam_scale = r.f32()?;
        PPtr::read(&mut r)?; // foam amount render texture
        let foam_decay = r.f32()?;
        let foam_distance = r.f32()?;
        let sub_surface_foam_color = color(&mut r)?;
        let sub_surface_foam_scale = r.f32()?;
        let displacement_texture_foam_amount_multiplier = r.f32()?;
        PPtr::read(&mut r)?; // sun light
        // Clip camera size, clip texture size, SSR downsample, iterations,
        // binary search iterations, pixel stride, stride z cut-off, pixel z
        // size offset, max ray distance.
        r.bytes(9 * 4)?;
        let enable_reflection = r.bool_aligned()?;
        // Screen edge fade start, eye fade start, eye fade end.
        r.bytes(3 * 4)?;
        let screen_space_refraction_index = r.f32()?;
        // Internal reflection flatness, SSR max distance and steps (PC and
        // console), triangles per patch, min patch size, error threshold.
        r.bytes(8 * 4)?;
        let time_scale = r.f32()?;
        let _visible = r.bool_aligned()?;
        PPtr::read(&mut r)?; // sky map renderer
        PPtr::read(&mut r)?; // sky manager
        if r.pos() != data.len() {
            return Err(r.error(crate::ErrorKind::Invalid(format!(
                "WaterSurface: {} bytes left after the last field",
                data.len() - r.pos()
            ))));
        }
        Ok(WaterSurface {
            surface_shader,
            update_foam_shader,
            update_normals_shader,
            interpolate_shader,
            under_water_brightness_curve,
            use_under_water_brightness_curve,
            patch_length,
            waves,
            refraction_index,
            under_water_refraction_index,
            under_water_refraction_depth_scale,
            water_offset,
            sequence_length,
            caustics_size,
            num_caustics_frames,
            caustics_frames_per_second,
            cubic_interpolation,
            reflection_color,
            refraction_color,
            back_light_tint,
            sun_reflection_gloss,
            sun_reflection_amount,
            wave_height_thickness_scale,
            foam_texture,
            foam_mask_texture,
            foam_smoothing,
            foam_rate,
            foam_scale,
            foam_decay,
            foam_distance,
            sub_surface_foam_color,
            sub_surface_foam_scale,
            displacement_texture_foam_amount_multiplier,
            enable_reflection,
            screen_space_refraction_index,
            time_scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, value: f32, in_slope: f32, out_slope: f32) -> Keyframe {
        Keyframe {
            time,
            value,
            in_slope,
            out_slope,
            weighted_mode: 0,
            in_weight: 1.0 / 3.0,
            out_weight: 1.0 / 3.0,
        }
    }

    #[test]
    fn curves_clamp_and_interpolate() {
        let c = AnimationCurve {
            keys: vec![key(0.0, 1.0, 0.0, 0.0), key(10.0, 3.0, 0.0, 0.0)],
            pre_infinity: 2,
            post_infinity: 2,
        };
        assert_eq!(c.evaluate(-5.0), 1.0);
        assert_eq!(c.evaluate(20.0), 3.0);
        // Flat tangents: smoothstep between the values.
        assert!((c.evaluate(5.0) - 2.0).abs() < 1e-6);
        assert!((c.evaluate(2.5) - (1.0 + 2.0 * 0.15625)).abs() < 1e-6);
        // A straight line when the slopes match it.
        let line = AnimationCurve {
            keys: vec![key(0.0, 0.0, 1.0, 1.0), key(4.0, 4.0, 1.0, 1.0)],
            pre_infinity: 2,
            post_infinity: 2,
        };
        assert!((line.evaluate(1.3) - 1.3).abs() < 1e-6);
    }

    #[test]
    fn truncated_data_is_an_error() {
        assert!(WaterSurface::parse(&[0u8; 40], false).is_err());
    }
}
