//! Unity's `Light` component (class 108), Unity 2019.4 layout (confirmed
//! against UnityPy's class layout on the game's prefabs: 264 bytes, every
//! field equal). See `docs/formats/lighting.md` § Local lights.

use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;
use crate::water_surface::AnimationCurve;

/// `LightType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Spot,
    Directional,
    Point,
    /// Area lights are baked only.
    Area,
    Other(i32),
}

impl LightKind {
    fn from_i32(v: i32) -> LightKind {
        match v {
            0 => LightKind::Spot,
            1 => LightKind::Directional,
            2 => LightKind::Point,
            3 => LightKind::Area,
            other => LightKind::Other(other),
        }
    }
}

/// `LightShadows`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowKind {
    None,
    Hard,
    Soft,
    Other(i32),
}

/// `LightmapBakeType`: realtime lights (4) and mixed ones (1) light the
/// scene at run time; baked ones (2) only exist in lightmaps.
pub const BAKE_REALTIME: i32 = 4;
pub const BAKE_MIXED: i32 = 1;
pub const BAKE_BAKED: i32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub game_object: PPtr,
    pub enabled: bool,
    pub kind: LightKind,
    /// As stored (sRGB in a linear project).
    pub color: [f32; 4],
    pub intensity: f32,
    pub range: f32,
    /// Full cone angle, degrees.
    pub spot_angle: f32,
    pub inner_spot_angle: f32,
    pub cookie_size: f32,
    pub shadows: ShadowKind,
    /// −1: from the quality settings.
    pub shadow_resolution: i32,
    pub shadow_strength: f32,
    pub shadow_bias: f32,
    pub shadow_normal_bias: f32,
    pub shadow_near_plane: f32,
    pub cookie: PPtr,
    pub draw_halo: bool,
    /// `m_BakingOutput.lightmapBakeMode.lightmapBakeType`.
    pub bake_type: i32,
    pub flare: PPtr,
    /// 0 auto, 1 important, 2 not important (forward rendering only).
    pub render_mode: i32,
    /// Layers the light affects.
    pub culling_mask: u32,
}

impl Light {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Light> {
        let mut r = Reader::new(data, big_endian);
        let bool4 = |r: &mut Reader| -> Result<bool> {
            let b = r.u8()? != 0;
            r.align(4)?;
            Ok(b)
        };
        let game_object = PPtr::read(&mut r)?;
        let enabled = bool4(&mut r)?;
        let kind = LightKind::from_i32(r.i32()?);
        let _shape = r.i32()?;
        let color = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let intensity = r.f32()?;
        let range = r.f32()?;
        let spot_angle = r.f32()?;
        let inner_spot_angle = r.f32()?;
        let cookie_size = r.f32()?;
        let shadows = match r.i32()? {
            0 => ShadowKind::None,
            1 => ShadowKind::Hard,
            2 => ShadowKind::Soft,
            other => ShadowKind::Other(other),
        };
        let shadow_resolution = r.i32()?;
        let _custom_resolution = r.i32()?;
        let shadow_strength = r.f32()?;
        let shadow_bias = r.f32()?;
        let shadow_normal_bias = r.f32()?;
        let shadow_near_plane = r.f32()?;
        r.bytes(64)?; // m_CullingMatrixOverride
        bool4(&mut r)?; // m_UseCullingMatrixOverride
        let cookie = PPtr::read(&mut r)?;
        let draw_halo = bool4(&mut r)?;
        let _probe_occlusion_light_index = r.i32()?;
        let _occlusion_mask_channel = r.i32()?;
        let bake_type = r.i32()?;
        let _mixed_lighting_mode = r.i32()?;
        bool4(&mut r)?; // isBaked
        let flare = PPtr::read(&mut r)?;
        let render_mode = r.i32()?;
        let culling_mask = r.u32()?;
        Ok(Light {
            game_object,
            enabled,
            kind,
            color,
            intensity,
            range,
            spot_angle,
            inner_spot_angle,
            cookie_size,
            shadows,
            shadow_resolution,
            shadow_strength,
            shadow_bias,
            shadow_normal_bias,
            shadow_near_plane,
            cookie,
            draw_halo,
            bake_type,
            flare,
            render_mode,
            culling_mask,
        })
    }

    /// Whether the light lights the scene at run time.
    pub fn is_realtime(&self) -> bool {
        self.bake_type != BAKE_BAKED && self.kind != LightKind::Area
    }
}

/// The game's `DayNightLight` (a MonoBehaviour next to a `Light`): every
/// frame, colour = lerp((R, G, B)(d), replaceColor, sunFraction(d) ×
/// replaceFraction) and intensity = `IntensityToGamma(intensity(d) × fade)`,
/// `d` = `DayNightCycle.GetDayScalar()`.
#[derive(Clone, Debug, PartialEq)]
pub struct DayNightLight {
    pub color_r: AnimationCurve,
    pub color_g: AnimationCurve,
    pub color_b: AnimationCurve,
    pub intensity: AnimationCurve,
    pub sun_fraction: AnimationCurve,
    pub replace_color: [f32; 4],
    pub replace_fraction: f32,
    pub fade: f32,
}

impl DayNightLight {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<DayNightLight> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        Ok(DayNightLight {
            color_r: AnimationCurve::read(&mut r)?,
            color_g: AnimationCurve::read(&mut r)?,
            color_b: AnimationCurve::read(&mut r)?,
            intensity: AnimationCurve::read(&mut r)?,
            sun_fraction: AnimationCurve::read(&mut r)?,
            replace_color: [r.f32()?, r.f32()?, r.f32()?, r.f32()?],
            replace_fraction: r.f32()?,
            fade: r.f32()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A point light as Unity 2019.4 stores it (264 bytes).
    fn light_bytes(kind: i32, bake: i32) -> Vec<u8> {
        let mut b = Vec::new();
        let i = |b: &mut Vec<u8>, v: i32| b.extend_from_slice(&v.to_le_bytes());
        let f = |b: &mut Vec<u8>, v: f32| b.extend_from_slice(&v.to_le_bytes());
        let p = |b: &mut Vec<u8>, id: i64| {
            b.extend_from_slice(&0i32.to_le_bytes());
            b.extend_from_slice(&id.to_le_bytes());
        };
        p(&mut b, 42);
        b.extend_from_slice(&[1, 0, 0, 0]);
        i(&mut b, kind);
        i(&mut b, 0);
        for v in [0.5, 0.75, 1.0, 1.0, 1.5, 15.0, 30.0, 21.8, 10.0] {
            f(&mut b, v);
        }
        i(&mut b, 2);
        i(&mut b, -1);
        i(&mut b, -1);
        for v in [1.0, 0.05, 0.4, 0.2] {
            f(&mut b, v);
        }
        b.extend_from_slice(&[0; 64]);
        b.extend_from_slice(&[0; 4]);
        p(&mut b, 0);
        b.extend_from_slice(&[0; 4]);
        i(&mut b, -1);
        i(&mut b, -1);
        i(&mut b, bake);
        i(&mut b, 2);
        b.extend_from_slice(&[0; 4]);
        p(&mut b, 0);
        i(&mut b, 0);
        b.extend_from_slice(&u32::MAX.to_le_bytes());
        i(&mut b, 1);
        i(&mut b, 4);
        i(&mut b, 0);
        for v in [1.0, 1.0, 1.0, 6570.0] {
            f(&mut b, v);
        }
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&[0; 16]);
        b.extend_from_slice(&[0; 4]);
        assert_eq!(b.len(), 264);
        b
    }

    #[test]
    fn reads_a_point_light() {
        let l = Light::parse(&light_bytes(2, BAKE_REALTIME), false).unwrap();
        assert_eq!(l.game_object.path_id, 42);
        assert!(l.enabled && l.is_realtime());
        assert_eq!(l.kind, LightKind::Point);
        assert_eq!(l.color, [0.5, 0.75, 1.0, 1.0]);
        assert_eq!((l.intensity, l.range, l.spot_angle), (1.5, 15.0, 30.0));
        assert_eq!(l.shadows, ShadowKind::Soft);
        assert_eq!((l.shadow_bias, l.shadow_normal_bias), (0.05, 0.4));
        assert_eq!(
            (l.bake_type, l.render_mode, l.culling_mask),
            (4, 0, u32::MAX)
        );
    }

    #[test]
    fn baked_lights_are_not_realtime() {
        let l = Light::parse(&light_bytes(0, BAKE_BAKED), false).unwrap();
        assert_eq!(l.kind, LightKind::Spot);
        assert!(!l.is_realtime());
    }

    #[test]
    fn truncated_lights_are_errors() {
        let b = light_bytes(2, BAKE_REALTIME);
        for n in [0, 20, 150, 210] {
            assert!(Light::parse(&b[..n], false).is_err(), "{n}");
        }
    }
}
