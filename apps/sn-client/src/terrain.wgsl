// Terrain shader: Bevy's standard PBR material, with colour and normal maps
// projected from the three world axes ("triplanar"), since terrain meshes have
// no texture coordinates. Written for subnautica-rs.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

// "Cap" textures cover upward-facing surfaces (the Y projection), "side"
// textures slopes and cliffs (the X and Z projections). Plain materials use
// the same textures for both.
struct TerrainParams {
    cap_tint: vec4<f32>,
    side_tint: vec4<f32>,
    // Texture repeats per metre.
    cap_scale: f32,
    side_scale: f32,
    // 1 if the normal map is real (else a flat placeholder is bound).
    has_cap_normal: u32,
    has_side_normal: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> terrain: TerrainParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var cap_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var cap_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var cap_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var cap_normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var side_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var side_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var side_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var side_normal_sampler: sampler;

// Unity's DXT5nm normal maps keep X in alpha and Y in green.
fn unpack_normal(t: vec4<f32>) -> vec3<f32> {
    let xy = vec2<f32>(t.a, t.g) * 2.0 - 1.0;
    return vec3<f32>(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    // Smooth material transition at boundaries (Milestone 6b)
    let blend_w = in.uv.x;
    if blend_w < 0.999 {
        let bayer = array<f32, 16>(
             0.0 / 16.0,  8.0 / 16.0,  2.0 / 16.0, 10.0 / 16.0,
            12.0 / 16.0,  4.0 / 16.0, 14.0 / 16.0,  6.0 / 16.0,
             3.0 / 16.0, 11.0 / 16.0,  1.0 / 16.0,  9.0 / 16.0,
            15.0 / 16.0,  7.0 / 16.0, 13.0 / 16.0,  5.0 / 16.0,
        );
        let coord = vec2<u32>(in.position.xy) % 4u;
        let threshold = bayer[coord.y * 4u + coord.x];
        let blend = smoothstep(0.0, 1.0, blend_w);
        if blend < threshold {
            discard;
        }
    }

    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let n = normalize(in.world_normal);
    // Blend weights: mostly the axis the surface faces, sharpened.
    var w = pow(abs(n), vec3<f32>(4.0));
    w = w / (w.x + w.y + w.z);

    let world = in.world_position.xyz;
    let uv_x = world.zy * terrain.side_scale;
    let uv_y = world.xz * terrain.cap_scale;
    let uv_z = world.xy * terrain.side_scale;

    let colour = textureSample(side_albedo, side_albedo_sampler, uv_x) * terrain.side_tint * w.x
        + textureSample(cap_albedo, cap_albedo_sampler, uv_y) * terrain.cap_tint * w.y
        + textureSample(side_albedo, side_albedo_sampler, uv_z) * terrain.side_tint * w.z;
    pbr_input.material.base_color = pbr_input.material.base_color * vec4<f32>(colour.rgb, 1.0);

    // "Whiteout" blend: each projection's tangent-space normal is combined
    // with the surface normal in that projection's plane. A missing normal
    // map counts as flat.
    var tx = vec3<f32>(0.0, 0.0, 1.0);
    var ty = vec3<f32>(0.0, 0.0, 1.0);
    var tz = vec3<f32>(0.0, 0.0, 1.0);
    let sx = textureSample(side_normal, side_normal_sampler, uv_x);
    let sy = textureSample(cap_normal, cap_normal_sampler, uv_y);
    let sz = textureSample(side_normal, side_normal_sampler, uv_z);
    if terrain.has_side_normal != 0u {
        tx = unpack_normal(sx);
        tz = unpack_normal(sz);
    }
    if terrain.has_cap_normal != 0u {
        ty = unpack_normal(sy);
    }
    tx = vec3<f32>(tx.xy + n.zy, abs(tx.z) * n.x);
    ty = vec3<f32>(ty.xy + n.xz, abs(ty.z) * n.y);
    tz = vec3<f32>(tz.xy + n.xy, abs(tz.z) * n.z);
    pbr_input.N = normalize(tx.zyx * w.x + ty.xzy * w.y + tz.xyz * w.z);

    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
