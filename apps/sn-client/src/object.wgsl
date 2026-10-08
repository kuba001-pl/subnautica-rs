// World-object shader: Bevy's standard material inputs (albedo, tint,
// alpha), the game's normal maps, which keep X in alpha and Y in green
// ("DXT5nm") and so can't use Bevy's own normal mapping, and the game's
// lighting (game_light.wgsl; specular, gloss and emission maps not yet).
// Written for subnautica-rs.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_normal_mapping, calculate_tbn_mikktspace},
    pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::forward_io::{VertexOutput, FragmentOutput}
#import sn_client::game_light::{GameSurface, game_lighting}
#endif

// Must match `ObjectParams` in object_look.rs.
struct ObjectParams {
    // Normal-map texture coordinates: uv * scale + offset (xy, zw).
    normal_st: vec4<f32>,
    // 1 if a normal map is bound.
    has_normal: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> object: ObjectParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var normal_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(120) var light_params: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var caustics: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var caustics_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef VERTEX_TANGENTS
#ifdef VERTEX_UVS_A
    if object.has_normal != 0u {
        let t = textureSample(normal_map, normal_sampler, in.uv * object.normal_st.xy + object.normal_st.zw);
        let xy = vec2<f32>(t.a * t.r, t.g) * 2.0 - 1.0;
        let nt = vec3<f32>(xy, sqrt(1.0 - min(dot(xy, xy), 1.0)));
        let tbn = calculate_tbn_mikktspace(in.world_normal, in.world_tangent);
        let double_sided = (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
        pbr_input.N = apply_normal_mapping(pbr_input.material.flags, tbn, double_sided, is_front, nt);
    }
#endif
#endif

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var surface: GameSurface;
    surface.world = in.world_position.xyz;
    surface.normal = pbr_input.N;
    surface.albedo = pbr_input.material.base_color.rgb;
    surface.specular = vec3<f32>(0.0);
    surface.gloss = 0.0;
    var out: FragmentOutput;
    out.color = vec4<f32>(
        game_lighting(surface, in.position, light_params, caustics, caustics_sampler),
        pbr_input.material.base_color.a,
    );
#endif
    return out;
}
