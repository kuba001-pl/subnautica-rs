//! Materials for world objects: Bevy's standard material with the game's
//! albedo, tint and alpha, plus its normal maps through `object.wgsl`.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

pub type ObjectMaterial = ExtendedMaterial<StandardMaterial, ObjectExtension>;

/// Must match `ObjectParams` in `object.wgsl`.
#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct ObjectParams {
    /// Normal-map texture coordinates: uv × (x, y) + (z, w).
    pub normal_st: Vec4,
    pub has_normal: u32,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct ObjectExtension {
    #[uniform(100)]
    pub params: ObjectParams,
    #[texture(101)]
    #[sampler(102)]
    pub normal_map: Handle<Image>,
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
