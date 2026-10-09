//! The terrain grass material: Bevy's standard material (for its pipeline
//! settings: alpha mask, culling) plus `grass.wgsl`, a port of the game's
//! `UWE/SIG Terrain Grass` (also drawing `UWE/SIG` and `UWE/SIG AlphaCutout +
//! Noisey Wave` materials).

use bevy::math::Affine2;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Face, ShaderType};
use bevy::shader::ShaderRef;
use sn_assets::{GrassLook, GrassShader};

use crate::game_light::GameLightImages;
use crate::textures::linear;

pub type GrassMaterial = ExtendedMaterial<StandardMaterial, GrassExtension>;

pub const HAS_NORMAL: u32 = 1;
pub const HAS_SIG: u32 = 2;
pub const SIG_FROM_ALBEDO: u32 = 4;
/// Noisey Wave's sway instead of the grass shader's.
pub const NOISE_WAVE: u32 = 8;

/// Must match `GrassParams` in `grass.wgsl`.
#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct GrassParams {
    pub color: Vec4,
    pub color2: Vec4,
    pub bot_color: Vec4,
    pub bot_color2: Vec4,
    pub gradient: Vec4,
    pub sig_str: Vec4,
    pub spec_color: Vec4,
    pub wave: Vec4,
    pub wave_dir: Vec4,
    pub mask: Vec4,
    pub main_st: Vec4,
    pub bump_st: Vec4,
    pub sig_st: Vec4,
    pub mask_st: Vec4,
    pub flags: u32,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct GrassExtension {
    #[uniform(100)]
    pub params: GrassParams,
    #[texture(101)]
    #[sampler(102)]
    pub albedo: Handle<Image>,
    #[texture(103)]
    #[sampler(104)]
    pub normal: Handle<Image>,
    #[texture(105)]
    #[sampler(106)]
    pub sig: Handle<Image>,
    #[texture(107)]
    #[sampler(108)]
    pub mask: Handle<Image>,
    #[texture(120, sample_type = "float", filterable = false)]
    pub light_params: Handle<Image>,
    #[texture(121, dimension = "2d_array")]
    #[sampler(122)]
    pub caustics: Handle<Image>,
    #[texture(123)]
    #[sampler(124)]
    pub spot_cookie: Handle<Image>,
}

impl MaterialExtension for GrassExtension {
    fn vertex_shader() -> ShaderRef {
        "embedded://sn_client/grass.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://sn_client/grass.wgsl".into()
    }
}

pub struct GrassLookPlugin;

impl Plugin for GrassLookPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "grass.wgsl");
        app.add_plugins(MaterialPlugin::<GrassMaterial>::default());
    }
}

/// Uploaded textures of a grass material (`None`: the material has none).
pub struct GrassImages {
    pub albedo: Option<Handle<Image>>,
    pub normal: Option<Handle<Image>>,
    pub sig: Option<Handle<Image>>,
    pub mask: Option<Handle<Image>>,
    /// Stand-ins for missing textures.
    pub white: Handle<Image>,
    pub flat: Handle<Image>,
}

/// The material values of `look` as the shader takes them. `UWE/SIG`
/// materials have one colour, no gradient, mask, waves or cutoff; their
/// glow is `_EmissionScale` by day and night.
pub fn grass_params(look: &GrassLook, images: &GrassImages) -> GrassParams {
    let c = |name: &str, default: [f32; 4]| linear(look.color(name, default));
    let f = |name: &str, default: f32| look.float(name, default);
    let st = |name: &str| Vec4::from(look.st(name));
    let mut flags = 0;
    if images.normal.is_some() {
        flags |= HAS_NORMAL;
    }
    let white = [1.0; 4];
    match look.shader {
        GrassShader::Sig { sig } => {
            let color = c("_Color", white);
            let emission = f("_EmissionScale", 1.0);
            if sig && images.sig.is_some() {
                flags |= HAS_SIG;
            } else if !sig {
                flags |= SIG_FROM_ALBEDO;
            }
            GrassParams {
                color,
                color2: color,
                bot_color: color,
                bot_color2: color,
                gradient: Vec4::new(0.0, 1.0, 0.0, 0.0),
                // Without the keyword the program has no glow.
                sig_str: if sig {
                    Vec4::new(1.0, emission, 1.0, emission)
                } else {
                    Vec4::new(1.0, 0.0, 1.0, 0.0)
                },
                spec_color: c("_SpecColor", white),
                wave: Vec4::ZERO,
                wave_dir: Vec4::ZERO,
                mask: Vec4::new(1.0, 0.0, 0.0, 0.0),
                main_st: st("_MainTex"),
                bump_st: st("_BumpMap"),
                sig_st: st("_SIGMap"),
                mask_st: st("_Mask"),
                flags,
            }
        }
        // One colour, no gradient or mask; a sway along `_WorldWaveDir` by
        // the position along `_ObjectUp` in the chunk, phased by noise.
        GrassShader::NoiseyWave => {
            flags |= NOISE_WAVE;
            if images.sig.is_some() {
                flags |= HAS_SIG;
            }
            let color = c("_Color", white);
            GrassParams {
                color,
                color2: color,
                bot_color: color,
                bot_color2: color,
                gradient: Vec4::new(0.0, 1.0, 0.0, 0.0),
                sig_str: Vec4::from(look.color("_SIGstr", [1.0, 1.0, 1.0, 0.0])),
                spec_color: c("_SpecColor", white),
                wave: Vec4::new(
                    f("_WaveAmount", 0.0),
                    f("_WaveSpeed", 0.0),
                    f("_TimeOffset", 0.0),
                    0.0,
                ),
                // As stored, Unity axes.
                wave_dir: Vec4::from(look.color("_WorldWaveDir", [0.0; 4])),
                mask: Vec4::new(1.0, 0.0, look.cutoff, 0.0),
                main_st: st("_MainTex"),
                bump_st: st("_BumpMap"),
                sig_st: st("_SIGMap"),
                mask_st: st("_Mask"),
                flags,
            }
        }
        _ => {
            if images.sig.is_some() {
                flags |= HAS_SIG;
            }
            GrassParams {
                color: c("_Color", white),
                color2: c("_Color2", white),
                bot_color: c("_BotColor", white),
                bot_color2: c("_BotColor2", white),
                // As stored (not colours).
                gradient: Vec4::from(look.color("_GradientParams", [0.0, 1.0, 0.0, 0.0])),
                sig_str: Vec4::from(look.color("_SIGstr", [1.0, 1.0, 1.0, 0.0])),
                spec_color: c("_SpecColor", white),
                wave: Vec4::new(
                    f("_WaveAmount", 0.0),
                    f("_WaveSpeed", 0.0),
                    f("_TimeOffset", 0.0),
                    f("_ForceNormals", 0.0),
                ),
                wave_dir: Vec4::ZERO,
                mask: Vec4::new(
                    f("_MaskScale", 1.0).max(1e-3),
                    f("_MaskStr", 0.0),
                    look.cutoff,
                    if images.mask.is_some() { 1.0 } else { 0.0 },
                ),
                main_st: st("_MainTex"),
                bump_st: st("_BumpMap"),
                sig_st: st("_SIGMap"),
                mask_st: st("_Mask"),
                flags,
            }
        }
    }
}

/// The material for a grass type drawn with `grass.wgsl`.
pub fn grass_material(
    look: &GrassLook,
    images: &GrassImages,
    light: &GameLightImages,
) -> GrassMaterial {
    let params = grass_params(look, images);
    let [sx, sy, ox, oy] = look.albedo_st;
    let cutout = params.mask.z > 0.0;
    GrassMaterial {
        base: StandardMaterial {
            // Only the pipeline settings matter: grass.wgsl does the rest.
            uv_transform: Affine2::from_scale_angle_translation(
                Vec2::new(sx, sy),
                0.0,
                Vec2::new(ox, oy),
            ),
            alpha_mode: if cutout {
                AlphaMode::Mask(params.mask.z)
            } else {
                AlphaMode::Opaque
            },
            double_sided: look.double_sided,
            cull_mode: if look.double_sided {
                None
            } else {
                Some(Face::Back)
            },
            ..default()
        },
        extension: GrassExtension {
            params,
            albedo: images
                .albedo
                .clone()
                .unwrap_or_else(|| images.white.clone()),
            normal: images.normal.clone().unwrap_or_else(|| images.flat.clone()),
            sig: images.sig.clone().unwrap_or_else(|| images.white.clone()),
            mask: images.mask.clone().unwrap_or_else(|| images.white.clone()),
            light_params: light.params.clone(),
            caustics: light.caustics.clone(),
            spot_cookie: light.spot_cookie.clone(),
        },
    }
}

/// The sway of a vertex (`grass.wgsl`'s `wave_offset`, kept in step by a
/// test): Unity object-space offset for vertex colour `color` at game time
/// `time`, with `wave` = (_WaveAmount, _WaveSpeed, _TimeOffset, _).
#[cfg(test)]
// The game's shader writes 6.28318, not τ.
#[allow(clippy::approx_constant)]
fn wave_offset(color: [f32; 4], wave: Vec4, time: f32) -> Vec3 {
    let height = color[3] * 5.0;
    let period = 5.0 - 5.0 * wave.y;
    let t = (time + wave.z * 0.1 - height) * 6.28318 / period;
    let sway = t.sin() * color[2] * wave.x;
    let angle = color[0] * 0.783185 + 5.5;
    Vec3::new(angle.cos(), 0.0, angle.sin()) * height * sway
}

/// Noisey Wave's 2D simplex noise (`grass.wgsl`'s `simplex`, read from the
/// game's vertex program), scaled by its 130.
#[cfg(test)]
fn simplex(v: Vec2) -> f32 {
    let c = Vec4::new(0.211_324_87, 0.366_025_42, -0.577_350_26, 0.024_390_243);
    let mod289 = |x: f32| x - (x * (1.0 / 289.0)).floor() * 289.0;
    let permute = |x: f32| mod289((x * 34.0 + 1.0) * x);
    let mut i = (v + Vec2::splat(v.dot(Vec2::splat(c.y)))).floor();
    let x0 = v - i + Vec2::splat(i.dot(Vec2::splat(c.x)));
    let i1 = if x0.x > x0.y {
        Vec2::new(1.0, 0.0)
    } else {
        Vec2::new(0.0, 1.0)
    };
    let x1 = x0 + Vec2::splat(c.x) - i1;
    let x2 = x0 + Vec2::splat(c.z);
    i = Vec2::new(mod289(i.x), mod289(i.y));
    let p = Vec3::new(
        permute(permute(i.y) + i.x),
        permute(permute(i.y + i1.y) + i.x + i1.x),
        permute(permute(i.y + 1.0) + i.x + 1.0),
    );
    let mut m = (Vec3::splat(0.5) - Vec3::new(x0.dot(x0), x1.dot(x1), x2.dot(x2))).max(Vec3::ZERO);
    m = m * m;
    m = m * m;
    let x = (p * c.w).fract() * 2.0 - Vec3::ONE;
    let h = x.abs() - Vec3::splat(0.5);
    let a0 = x - (x + Vec3::splat(0.5)).floor();
    m *= Vec3::splat(1.792_843) - 0.853_735 * (a0 * a0 + h * h);
    let g = Vec3::new(
        a0.x * x0.x + h.x * x0.y,
        a0.y * x1.x + h.y * x1.y,
        a0.z * x2.x + h.z * x2.y,
    );
    130.0 * m.dot(g)
}

/// Noisey Wave's sway (`grass.wgsl`'s `noise_wave_offset`): world offset
/// (Unity axes) of a vertex at Unity world `xz`, `along` `_ObjectUp` in its
/// chunk, with `wave` = (_WaveAmount, _WaveSpeed, _TimeOffset, _).
#[cfg(test)]
#[allow(clippy::approx_constant)] // the game's 6.28318
fn noise_wave_offset(xz: Vec2, along: f32, wave: Vec4, dir: Vec3, time: f32) -> Vec3 {
    let phase = (time + wave.z - simplex(xz)) * 6.28318 / (5.0 - 5.0 * wave.y);
    dir * (phase.sin() * wave.x * along)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_waves_follow_the_game() {
        // The noise (times 130): within ±1, smooth, not flat.
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for i in 0..2000 {
            let v = Vec2::new(i as f32 * 0.137 - 120.0, i as f32 * 0.071 + 1500.0);
            let n = simplex(v);
            lo = lo.min(n);
            hi = hi.max(n);
            let near = simplex(v + Vec2::splat(0.001));
            assert!((n - near).abs() < 0.01, "{v} {n} {near}");
        }
        assert!(lo > -1.0 && hi < 1.0 && hi - lo > 1.0, "{lo} {hi}");
        // Type 243's values: at most 0.005 × the height in the chunk, along
        // the wave direction only.
        let wave = Vec4::new(0.005, 0.5, 0.0, 0.0);
        let dir = Vec3::new(1.0, 0.0, 0.0);
        let mut max = 0.0f32;
        for i in 0..250 {
            let o = noise_wave_offset(Vec2::new(-700.0, -300.0), 12.0, wave, dir, i as f32 * 0.01);
            assert_eq!((o.y, o.z), (0.0, 0.0));
            max = max.max(o.x.abs());
        }
        assert!(max <= 0.06 + 1e-6 && max > 0.059, "{max}");
        // The shader holds the same constants.
        let wgsl = include_str!("grass.wgsl");
        for c in [
            "0.21132487",
            "0.36602542",
            "-0.57735026",
            "0.024390243",
            "289.0",
            "34.0",
            "1.792843",
            "0.853735",
            "130.0",
        ] {
            assert!(wgsl.contains(c), "{c}");
        }
    }

    #[test]
    fn noisey_wave_values() {
        let look = GrassLook {
            shader: GrassShader::NoiseyWave,
            cutoff: 0.2,
            floats: vec![("_WaveAmount".into(), 0.005), ("_WaveSpeed".into(), 0.5)],
            colors: vec![("_WorldWaveDir".into(), [1.0, 0.0, 0.0, 0.0])],
            ..GrassLook::default()
        };
        let p = grass_params(&look, &images());
        assert_eq!(p.flags, HAS_SIG | NOISE_WAVE);
        assert_eq!(p.wave, Vec4::new(0.005, 0.5, 0.0, 0.0));
        assert_eq!(p.wave_dir, Vec4::new(1.0, 0.0, 0.0, 0.0));
        assert_eq!(p.mask.z, 0.2);
    }

    #[test]
    fn waves_follow_the_game() {
        let wave = Vec4::new(0.3, 0.5, 0.0, 0.0);
        // The base (height 0) never moves.
        assert_eq!(wave_offset([0.5, 0.0, 1.0, 0.0], wave, 1.234), Vec3::ZERO);
        // A tip 1 m up (alpha 0.2) with full amount: at most 0.3 m, period
        // 2.5 s (speed 0.5), horizontal only.
        let mut max = 0.0f32;
        for i in 0..250 {
            let o = wave_offset([0.0, 0.0, 1.0, 0.2], wave, i as f32 * 0.01);
            assert_eq!(o.y, 0.0);
            max = max.max(o.length());
            let later = wave_offset([0.0, 0.0, 1.0, 0.2], wave, i as f32 * 0.01 + 2.5);
            assert!((o - later).length() < 1e-4);
        }
        assert!((max - 0.3).abs() < 1e-3, "{max}");
        // The shader holds the same constants.
        let wgsl = include_str!("grass.wgsl");
        for c in ["0.783185", "5.5", "6.28318", "* 0.1", "* 5.0"] {
            assert!(wgsl.contains(c), "{c}");
        }
    }

    fn images() -> GrassImages {
        GrassImages {
            albedo: None,
            normal: None,
            sig: Some(Handle::default()),
            mask: None,
            white: Handle::default(),
            flat: Handle::default(),
        }
    }

    #[test]
    fn terrain_grass_values() {
        let look = GrassLook {
            shader: GrassShader::TerrainGrass,
            cutoff: 0.3,
            floats: vec![("_WaveAmount".into(), 0.3), ("_WaveSpeed".into(), 0.5)],
            colors: vec![("_GradientParams".into(), [0.14, 2.2, 0.0, 0.0])],
            ..GrassLook::default()
        };
        let p = grass_params(&look, &images());
        assert_eq!(p.wave, Vec4::new(0.3, 0.5, 0.0, 0.0));
        assert_eq!(p.gradient, Vec4::new(0.14, 2.2, 0.0, 0.0));
        assert_eq!(p.mask.z, 0.3);
        assert_eq!(p.flags, HAS_SIG);
        // No mask texture: the mask stays 0 (top/bottom colours 1 and 2 not mixed).
        assert_eq!(p.mask.w, 0.0);
    }

    #[test]
    fn sig_materials_are_plain() {
        let look = GrassLook {
            shader: GrassShader::Sig { sig: false },
            floats: vec![("_EmissionScale".into(), 2.0)],
            ..GrassLook::default()
        };
        let p = grass_params(&look, &images());
        assert_eq!(p.flags, SIG_FROM_ALBEDO);
        assert_eq!(p.wave, Vec4::ZERO);
        assert_eq!(p.mask.z, 0.0, "no cutoff");
        assert_eq!(p.sig_str.y, 0.0, "no glow without the keyword");
        let look = GrassLook {
            shader: GrassShader::Sig { sig: true },
            ..look
        };
        let p = grass_params(&look, &images());
        assert_eq!(p.flags, HAS_SIG);
        assert_eq!(p.sig_str, Vec4::new(1.0, 2.0, 1.0, 2.0));
    }
}
