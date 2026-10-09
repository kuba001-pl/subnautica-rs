//! Real terrain materials: the game's textures (uploaded to the GPU in their
//! compressed form) and a triplanar shader (`terrain.wgsl`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::math::Affine2;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, Face, ShaderType, TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use sn_assets::{GrassLook, GrassShader, TerrainMaterials, TerrainTexture};

use crate::game_light::GameLightImages;
use crate::grass_look::{GrassImages, GrassMaterial, grass_material};
use crate::object_look::{ObjectExtension, ObjectMaterial, ObjectParams};
use crate::objects::{Defaults, SkyLook, apply_sky, material_desc, object_material};
use crate::textures::{linear, to_image};

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TriplanarExtension>;

/// The sun's illuminance (lux). Emission 1.0 in the game is as bright as a
/// white surface lit by it.
pub const SUN_ILLUMINANCE: f32 = 8_000.0;

/// Bits of `TriplanarParams::flags`; must match `terrain.wgsl`.
const CAP_SIDE: u32 = 1;
const CAP_NORMAL: u32 = 2;
const SIDE_NORMAL: u32 = 4;
const CAP_SIG: u32 = 8;
const SIDE_SIG: u32 = 16;

/// Must match `TerrainParams` in `terrain.wgsl`. Colours are linear.
#[derive(Clone, Copy, Debug, Default, Reflect, ShaderType)]
pub struct TriplanarParams {
    pub cap_tint: Vec4,
    pub side_tint: Vec4,
    pub border_tint: Vec4,
    pub cap_scale: f32,
    pub side_scale: f32,
    pub triplanar: f32,
    pub border_range: f32,
    pub border_offset: f32,
    pub inner_range: f32,
    pub inner_offset: f32,
    pub cap_range: f32,
    pub cap_offset: f32,
    pub cap_angle: f32,
    pub cap_emission: f32,
    pub side_emission: f32,
    pub flags: u32,
    pub cap_spec: Vec4,
    pub side_spec: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TriplanarExtension {
    #[uniform(100)]
    pub params: TriplanarParams,
    #[texture(101)]
    #[sampler(102)]
    pub cap_albedo: Handle<Image>,
    #[texture(103)]
    #[sampler(104)]
    pub cap_normal: Handle<Image>,
    #[texture(105)]
    #[sampler(106)]
    pub cap_sig: Handle<Image>,
    #[texture(107)]
    #[sampler(108)]
    pub side_albedo: Handle<Image>,
    #[texture(109)]
    #[sampler(110)]
    pub side_normal: Handle<Image>,
    #[texture(111)]
    #[sampler(112)]
    pub side_sig: Handle<Image>,
    /// The game's lighting values and caustics (`game_light.rs`).
    #[texture(120, sample_type = "float", filterable = false)]
    pub light_params: Handle<Image>,
    #[texture(121, dimension = "2d_array")]
    #[sampler(122)]
    pub caustics: Handle<Image>,
    /// Unity's default spot-light cookie (`game_light.rs`).
    #[texture(123)]
    #[sampler(124)]
    pub spot_cookie: Handle<Image>,
}

impl MaterialExtension for TriplanarExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://sn_client/terrain.wgsl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "embedded://sn_client/terrain.wgsl".into()
    }
}

pub struct TerrainLookPlugin;

impl Plugin for TerrainLookPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "terrain.wgsl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default())
            .add_systems(Startup, build_look);
    }
}

/// The game's terrain materials, waiting to be turned into GPU resources.
#[derive(Resource)]
pub struct PendingTerrainLook(pub Option<TerrainMaterials>);

/// The game's terrain look: per octree type id, the material settings
/// (`None`: use the debug colour), and the GPU materials made from them.
#[derive(Resource, Default)]
pub struct TerrainLook {
    by_type: Vec<Option<TriplanarExtension>>,
    /// Per (type id, rank in the chunk's draw order).
    materials: HashMap<(u8, u8), Handle<TerrainMaterial>>,
    /// Per type id: the grass material, added to the assets on first use.
    grass: HashMap<u8, GrassLookMaterial>,
    grass_handles: HashMap<u8, GrassHandle>,
    /// Grass types drawn with our MarmosetUBER port: they take the global
    /// Marmoset sky once the objects' worker has read it.
    uber_grass: HashSet<u8>,
    /// Noisey Wave grass: `_ObjectUp` (Unity axes).
    grass_object_up: HashMap<u8, [f32; 3]>,
    pub textures: usize,
    pub texture_bytes: usize,
}

impl TerrainLook {
    /// The material for `type_id` drawn `rank`-th in its chunk: rank 0 is
    /// opaque, later ranks are alpha-blended over it, in rank order.
    pub fn material(
        &mut self,
        type_id: u8,
        rank: u8,
        assets: &mut Assets<TerrainMaterial>,
    ) -> Option<Handle<TerrainMaterial>> {
        let extension = self.by_type.get(usize::from(type_id))?.as_ref()?;
        let handle = self.materials.entry((type_id, rank)).or_insert_with(|| {
            assets.add(ExtendedMaterial {
                base: StandardMaterial {
                    alpha_mode: if rank == 0 {
                        AlphaMode::Opaque
                    } else {
                        AlphaMode::Blend
                    },
                    // Transparent meshes are drawn by distance plus this
                    // bias. All layers of a batch share one bounding box
                    // (see terrain.rs), so this keeps the game's order. It
                    // also nudges each layer towards the camera, so coplanar
                    // layers pass the depth test.
                    depth_bias: f32::from(rank),
                    ..default()
                },
                extension: extension.clone(),
            })
        });
        Some(handle.clone())
    }

    /// The grass material of `type_id`, if it has grass.
    pub fn grass_material(
        &mut self,
        type_id: u8,
        grass_assets: &mut Assets<GrassMaterial>,
        object_assets: &mut Assets<ObjectMaterial>,
    ) -> Option<GrassHandle> {
        if let Some(h) = self.grass_handles.get(&type_id) {
            return Some(h.clone());
        }
        let handle = match self.grass.get(&type_id)?.clone() {
            GrassLookMaterial::Grass(m) => GrassHandle::Grass(grass_assets.add(m)),
            GrassLookMaterial::Object(m) => GrassHandle::Object(object_assets.add(m)),
        };
        self.grass_handles.insert(type_id, handle.clone());
        Some(handle)
    }

    /// `_ObjectUp` of a Noisey Wave grass type; zero for the others (they
    /// don't sway by the position in the chunk).
    pub fn grass_object_up(&self, type_id: u8) -> [f32; 3] {
        self.grass_object_up
            .get(&type_id)
            .copied()
            .unwrap_or_default()
    }

    /// Lights the MarmosetUBER grass with the global sky. The game's grass
    /// pieces (`TerrainPoolManager.chunkGrassPrefab`) carry no `SkyApplier`,
    /// so they use the global sky, not their biome's.
    pub fn set_global_sky(&mut self, sky: Option<&SkyLook>, assets: &mut Assets<ObjectMaterial>) {
        for ty in &self.uber_grass {
            if let Some(GrassLookMaterial::Object(m)) = self.grass.get_mut(ty) {
                apply_sky(&mut m.extension.params, sky);
            }
            if let Some(GrassHandle::Object(h)) = self.grass_handles.get(ty)
                && let Some(mut m) = assets.get_mut(h)
            {
                apply_sky(&mut m.extension.params, sky);
            }
        }
        info!(
            "terrain look: {} MarmosetUBER grass materials lit with the global sky ({})",
            self.uber_grass.len(),
            if sky.is_some() { "found" } else { "missing" }
        );
    }
}

/// A grass type's material: the ported grass shader, or (for shaders not
/// ported yet) our object material.
#[derive(Clone)]
enum GrassLookMaterial {
    Grass(GrassMaterial),
    Object(ObjectMaterial),
}

#[derive(Clone)]
pub enum GrassHandle {
    Grass(Handle<GrassMaterial>),
    Object(Handle<ObjectMaterial>),
}

/// The first-pass look (M7e1) for grass shaders not ported yet: our object
/// material with the albedo, tint, cutoff and normal map.
fn first_pass_material(
    l: &GrassLook,
    albedo: Option<Handle<Image>>,
    normal: Option<Handle<Image>>,
    white: &Handle<Image>,
    flat: &Handle<Image>,
    light: &GameLightImages,
) -> ObjectMaterial {
    let c = linear(l.color);
    let [sx, sy, ox, oy] = l.albedo_st;
    let identity = Vec4::new(1.0, 1.0, 0.0, 0.0);
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::linear_rgba(c.x, c.y, c.z, c.w),
            base_color_texture: albedo,
            uv_transform: Affine2::from_scale_angle_translation(
                Vec2::new(sx, sy),
                0.0,
                Vec2::new(ox, oy),
            ),
            perceptual_roughness: 0.8,
            alpha_mode: AlphaMode::Mask(l.cutoff),
            double_sided: l.double_sided,
            cull_mode: if l.double_sided {
                None
            } else {
                Some(Face::Back)
            },
            ..default()
        },
        extension: ObjectExtension {
            params: ObjectParams {
                normal_st: Vec4::from(l.albedo_st),
                spec_st: identity,
                illum_st: identity,
                has_normal: u32::from(normal.is_some()),
                ..default()
            },
            normal_map: normal.unwrap_or_else(|| flat.clone()),
            spec_map: white.clone(),
            illum_map: white.clone(),
            light_params: light.params.clone(),
            caustics: light.caustics.clone(),
            spot_cookie: light.spot_cookie.clone(),
        },
    }
}

fn build_look(
    mut commands: Commands,
    mut pending: ResMut<PendingTerrainLook>,
    mut images: ResMut<Assets<Image>>,
    light: Res<GameLightImages>,
) {
    let Some(source) = pending.0.take() else {
        commands.insert_resource(TerrainLook::default());
        return;
    };
    let white = images.add(Image::new(
        Extent3d::default(),
        TextureDimension::D2,
        vec![255; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    // A flat normal in the game's packing (DXT5nm: x in alpha, y in green).
    let flat = images.add(Image::new(
        Extent3d::default(),
        TextureDimension::D2,
        vec![255, 128, 255, 128],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    ));
    let mut uploaded: Vec<(*const TerrainTexture, bool, Handle<Image>)> = Vec::new();
    let mut look = TerrainLook::default();
    let mut upload = |t: &Option<Arc<TerrainTexture>>, srgb: bool, look: &mut TerrainLook| {
        let t = t.as_ref()?;
        let key = Arc::as_ptr(t);
        if let Some((_, _, handle)) = uploaded.iter().find(|(k, s, _)| *k == key && *s == srgb) {
            return Some(handle.clone());
        }
        let image = to_image(t, srgb)?;
        look.textures += 1;
        look.texture_bytes += image.data.as_ref().map_or(0, Vec::len);
        let handle = images.add(image);
        uploaded.push((key, srgb, handle.clone()));
        Some(handle)
    };
    for slot in &source.types {
        let Some(m) = slot else {
            look.by_type.push(None);
            continue;
        };
        if let Some(g) = &m.grass {
            let l = &g.look;
            let albedo = upload(&l.albedo, true, &mut look);
            let normal = upload(&l.normal, false, &mut look);
            let material = match l.shader {
                GrassShader::TerrainGrass | GrassShader::Sig { .. } | GrassShader::NoiseyWave => {
                    if l.shader == GrassShader::NoiseyWave {
                        let [x, y, z, _] = l.color("_ObjectUp", [0.0, 1.0, 0.0, 0.0]);
                        look.grass_object_up.insert(m.type_id as u8, [x, y, z]);
                    }
                    let images = GrassImages {
                        albedo,
                        normal,
                        // Data, not colours.
                        sig: upload(&l.sig, false, &mut look),
                        mask: upload(&l.mask, false, &mut look),
                        white: white.clone(),
                        flat: flat.clone(),
                    };
                    GrassLookMaterial::Grass(grass_material(l, &images, &light))
                }
                // MarmosetUBER: our port of the object shader; the global
                // sky arrives later (`set_global_sky`).
                GrassShader::Marmoset if let Some(material) = &l.material => {
                    let mut images_by_id = HashMap::new();
                    let mut texture = |name: &str, srgb: Option<bool>| {
                        let t = match name {
                            "_MainTex" => &l.albedo,
                            "_BumpMap" => &l.normal,
                            "_SpecTex" => &l.spec,
                            "_Illum" => &l.illum,
                            _ => return None,
                        };
                        let own = t.as_ref()?.texture.color_space == 1;
                        let handle = upload(t, srgb.unwrap_or(own), &mut look)?;
                        let id = images_by_id.len() as u32;
                        images_by_id.insert(id, handle);
                        Some((id, l.st(name)))
                    };
                    let desc = material_desc(material, &mut texture);
                    if desc.uber.is_some() {
                        look.uber_grass.insert(m.type_id as u8);
                    }
                    let defaults = Defaults {
                        flat: flat.clone(),
                        white: white.clone(),
                    };
                    GrassLookMaterial::Object(object_material(
                        &desc,
                        None,
                        &images_by_id,
                        &defaults,
                        &light,
                    ))
                }
                GrassShader::Marmoset | GrassShader::Unknown => GrassLookMaterial::Object(
                    first_pass_material(l, albedo, normal, &white, &flat, &light),
                ),
            };
            look.grass.insert(m.type_id as u8, material);
        }
        let cap_albedo = upload(&m.cap.albedo, true, &mut look);
        let side_albedo = upload(&m.side.albedo, true, &mut look);
        let cap_normal = upload(&m.cap.normal, false, &mut look);
        let side_normal = upload(&m.side.normal, false, &mut look);
        // Specular/illumination maps hold data, not colours.
        let cap_sig = upload(&m.cap.sig, false, &mut look);
        let side_sig = upload(&m.side.sig, false, &mut look);
        let (Some(cap_albedo), Some(side_albedo)) = (cap_albedo, side_albedo) else {
            look.by_type.push(None);
            continue;
        };
        let flag = |on: bool, bit: u32| if on { bit } else { 0 };
        let b = &m.blend;
        look.by_type.push(Some(TriplanarExtension {
            params: TriplanarParams {
                cap_tint: linear(m.cap.tint),
                side_tint: linear(m.side.tint),
                border_tint: linear(b.border_tint),
                cap_scale: m.cap.scale,
                side_scale: m.side.scale,
                triplanar: b.triplanar,
                border_range: b.border_range,
                border_offset: b.border_offset,
                inner_range: b.inner_range,
                inner_offset: b.inner_offset,
                cap_range: b.cap_range,
                cap_offset: b.cap_offset,
                cap_angle: b.cap_angle,
                cap_emission: m.cap.emission,
                side_emission: m.side.emission,
                flags: flag(m.cap_side, CAP_SIDE)
                    | flag(cap_normal.is_some(), CAP_NORMAL)
                    | flag(side_normal.is_some(), SIDE_NORMAL)
                    | flag(cap_sig.is_some(), CAP_SIG)
                    | flag(side_sig.is_some(), SIDE_SIG),
                cap_spec: linear(m.cap.specular),
                side_spec: linear(m.side.specular),
            },
            cap_albedo,
            side_albedo,
            cap_normal: cap_normal.unwrap_or_else(|| white.clone()),
            side_normal: side_normal.unwrap_or_else(|| white.clone()),
            cap_sig: cap_sig.unwrap_or_else(|| white.clone()),
            side_sig: side_sig.unwrap_or_else(|| white.clone()),
            light_params: light.params.clone(),
            caustics: light.caustics.clone(),
            spot_cookie: light.spot_cookie.clone(),
        }));
    }
    info!(
        "terrain look: {} materials, {} grass materials ({} with the grass shader, {} with the object shader), {} textures ({:.0} MiB on the GPU)",
        look.by_type.iter().flatten().count(),
        look.grass.len(),
        look.grass
            .values()
            .filter(|m| matches!(m, GrassLookMaterial::Grass(_)))
            .count(),
        look.grass
            .values()
            .filter(|m| matches!(m, GrassLookMaterial::Object(_)))
            .count(),
        look.textures,
        look.texture_bytes as f64 / 1048576.0
    );
    commands.insert_resource(look);
}
