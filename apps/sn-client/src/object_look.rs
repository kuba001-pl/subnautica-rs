//! Materials for world objects: Bevy's standard material with the game's
//! albedo, tint and alpha, plus what `object.wgsl` needs to port the game's
//! object shader (MarmosetUBER): normal, specular and glow maps, the
//! material's values and the Marmoset sky it is lit with.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

pub type ObjectMaterial = ExtendedMaterial<StandardMaterial, ObjectExtension>;

/// Must match `ObjectParams` in `object.wgsl`.
#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct ObjectParams {
    /// Texture coordinates of the normal, specular and glow maps:
    /// uv × (x, y) + (z, w).
    pub normal_st: Vec4,
    pub spec_st: Vec4,
    pub illum_st: Vec4,
    /// `_SpecColor` (linear), `_SpecInt`.
    pub spec_color: Vec4,
    /// `_GlowColor` (linear), unused.
    pub glow_color: Vec4,
    /// `_Shininess`, `_Fresnel`, `_IBLreductionAtNight`, `_EnableSimpleGlass`.
    pub surface: Vec4,
    /// `_GlowStrength`, `_GlowStrengthNight`, `_EmissionLM`, `_EmissionLMNight`.
    pub glow: Vec4,
    /// The sky's `_ExposureIBL`: diffuse, specular, sky, camera exposure.
    pub exposure: Vec4,
    /// The sky's rotation (x, y, z, w; Unity coordinates).
    pub sky_rotation: Vec4,
    /// The sky's `_AffectedByDayNightCycle`, `_Outdoors`; 1 for a
    /// MarmosetUBER material (else the fields above are unused); unused.
    pub sky_flags: Vec4,
    /// The sky's `_SH0…_SH8` (RGB).
    pub sh: [Vec4; 9],
    pub has_normal: u32,
    pub has_spec: u32,
    pub has_illum: u32,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct ObjectExtension {
    #[uniform(100)]
    pub params: ObjectParams,
    #[texture(101)]
    #[sampler(102)]
    pub normal_map: Handle<Image>,
    #[texture(103)]
    #[sampler(104)]
    pub spec_map: Handle<Image>,
    #[texture(105)]
    #[sampler(106)]
    pub illum_map: Handle<Image>,
    /// The game's lighting values and caustics (`game_light.rs`).
    #[texture(120, sample_type = "float", filterable = false)]
    pub light_params: Handle<Image>,
    #[texture(121, dimension = "2d_array")]
    #[sampler(122)]
    pub caustics: Handle<Image>,
}

impl MaterialExtension for ObjectExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://sn_client/object.wgsl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "embedded://sn_client/object.wgsl".into()
    }
}

pub struct ObjectLookPlugin;

impl Plugin for ObjectLookPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "object.wgsl");
        app.add_plugins(MaterialPlugin::<ObjectMaterial>::default());
    }
}
