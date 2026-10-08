// Terrain shader: the surface the game's terrain shaders compute, lit by
// the game's lighting (game_light.wgsl). Written for subnautica-rs from the
// behaviour described in docs/formats/terrain-materials.md:
//
// - Textures are projected along the three world axes ("triplanar"), in
//   Unity's world space: X uses (y, z), Y uses (x, z), Z uses (y, x).
// - Cap/side materials use the cap texture on the Y projection only, and
//   switch to the side textures by slope, with a ragged edge from the cap
//   texture's alpha ("splotch").
// - Every mesh carries a per-vertex weight for its material (uv.x) and a
//   gloss (uv.y). Weight and splotch give the alpha for soft borders between
//   materials, and a tint towards the edge of a patch.

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
#import bevy_pbr::forward_io::{VertexOutput, FragmentOutput}
#import sn_client::game_light::{GameSurface, game_lighting, game_local_lights}
#endif

// Must match `TriplanarParams` in terrain_look.rs. Colours are linear.
struct TerrainParams {
    cap_tint: vec4<f32>,
    side_tint: vec4<f32>,
    border_tint: vec4<f32>,
    // Texture repeats per metre.
    cap_scale: f32,
    side_scale: f32,
    // Exponent of the projection weights (_TriplanarBlendRange).
    triplanar: f32,
    border_range: f32,
    border_offset: f32,
    inner_range: f32,
    inner_offset: f32,
    cap_range: f32,
    cap_offset: f32,
    cap_angle: f32,
    cap_emission: f32,
    side_emission: f32,
    flags: u32,
    // Specular colours (linear): _CapSpecColor, _SpecColor.
    cap_spec: vec4<f32>,
    side_spec: vec4<f32>,
}

const CAP_SIDE: u32 = 1u;
const CAP_NORMAL: u32 = 2u;
const SIDE_NORMAL: u32 = 4u;
const CAP_SIG: u32 = 8u;
const SIDE_SIG: u32 = 16u;

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> terrain: TerrainParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(120) var light_params: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var caustics: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var caustics_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(123) var spot_cookie: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(124) var spot_cookie_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var cap_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var cap_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var cap_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var cap_normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var cap_sig: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var cap_sig_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var side_albedo: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var side_albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var side_normal: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var side_normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var side_sig: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var side_sig_sampler: sampler;

// Normal maps: X = alpha × red (DXT5nm keeps X in alpha, red = 1), Y = green.
fn unpack_normal(t: vec4<f32>) -> vec3<f32> {
    let xy = vec2<f32>(t.a * t.r, t.g) * 2.0 - 1.0;
    return vec3<f32>(xy, sqrt(1.0 - min(dot(xy, xy), 1.0)));
}

// Tangent-space normal → world, one frame per projection (n: surface normal).
fn from_x(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let s = sign(n.x);
    return t.x * s * vec3<f32>(n.y, n.x, -n.z) + t.y * s * vec3<f32>(-n.z, n.y, n.x) + t.z * n;
}

fn from_y(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let s = sign(n.y);
    return t.x * s * vec3<f32>(n.y, -n.x, n.z) + t.y * s * vec3<f32>(n.x, -n.z, n.y) + t.z * n;
}

fn from_z(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let s = sign(n.z);
    return t.x * s * vec3<f32>(n.x, n.z, n.y) + t.y * s * vec3<f32>(n.z, n.y, n.x) + t.z * n;
}

// Weight + splotch → 0..1, the shape shared by the border alpha and the
// border tint.
fn border(weight: f32, splotch: f32, range: f32, offset: f32) -> f32 {
    let r = max(range, 1e-4);
    let threshold = (r + 1.0) / (1.01 - offset) * (1.0 - weight - offset) - r;
    return saturate((splotch - threshold) / r);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Unity's world space (z flipped).
    let flip = vec3<f32>(1.0, 1.0, -1.0);
    let n = normalize(in.world_normal) * flip;
    let p = in.world_position.xyz * flip;
    let weight = in.uv.x;
    let gloss = in.uv.y;

    var w = pow(max(1.96 * n * n, vec3<f32>(1e-8)), vec3<f32>(terrain.triplanar));
    w = w / (w.x + w.y + w.z);

    var colour: vec4<f32>;
    var normal: vec3<f32>;
    var sig = vec2<f32>(0.0);
    var spec = vec3<f32>(0.0);
    if (terrain.flags & CAP_SIDE) != 0u {
        let uv_cap = p.xz * terrain.cap_scale;
        let cap = textureSample(cap_albedo, cap_albedo_sampler, uv_cap);
        var cap_n = n;
        if (terrain.flags & CAP_NORMAL) != 0u {
            cap_n = normalize(from_y(unpack_normal(textureSample(cap_normal, cap_normal_sampler, uv_cap)), n));
        }
        // Specular: the SIG map's red, else the albedo's red.
        var cap_spec_r = cap.r;
        if (terrain.flags & CAP_SIG) != 0u {
            sig = textureSample(cap_sig, cap_sig_sampler, uv_cap).rg * vec2<f32>(1.0, terrain.cap_emission);
            cap_spec_r = sig.x;
        }
        // 0 = cap, 1 = side.
        let edge = clamp(1.0 - (n.y + terrain.cap_offset) * terrain.cap_angle - cap.a, -1.0, 1.0);
        let side = saturate((edge + terrain.cap_range) / max(2.0 * terrain.cap_range, 1e-4));

        let s = p * terrain.side_scale;
        let side_colour = textureSample(side_albedo, side_albedo_sampler, s.yz) * w.x
            + textureSample(side_albedo, side_albedo_sampler, s.xz) * w.y
            + textureSample(side_albedo, side_albedo_sampler, s.yx) * w.z;
        var side_n = n;
        if (terrain.flags & SIDE_NORMAL) != 0u {
            side_n = normalize(
                from_x(unpack_normal(textureSample(side_normal, side_normal_sampler, s.yz)), n) * w.x
                + from_y(unpack_normal(textureSample(side_normal, side_normal_sampler, s.xz)), n) * w.y
                + from_z(unpack_normal(textureSample(side_normal, side_normal_sampler, s.yx)), n) * w.z
            );
        }
        var side_sig_value = vec2<f32>(0.0);
        var side_spec_r = side_colour.r;
        if (terrain.flags & SIDE_SIG) != 0u {
            side_sig_value = (textureSample(side_sig, side_sig_sampler, s.yz).rg * w.x
                + textureSample(side_sig, side_sig_sampler, s.xz).rg * w.y
                + textureSample(side_sig, side_sig_sampler, s.yx).rg * w.z)
                * vec2<f32>(1.0, terrain.side_emission);
            side_spec_r = side_sig_value.x;
        }
        colour = mix(cap * terrain.cap_tint, side_colour * terrain.side_tint, side);
        normal = normalize(mix(cap_n, side_n, side));
        sig = mix(sig, side_sig_value, side);
        spec = mix(cap_spec_r * terrain.cap_spec.rgb, side_spec_r * terrain.side_spec.rgb, side);
    } else {
        let s = p * terrain.cap_scale;
        let albedo = textureSample(cap_albedo, cap_albedo_sampler, s.yz) * w.x
            + textureSample(cap_albedo, cap_albedo_sampler, s.xz) * w.y
            + textureSample(cap_albedo, cap_albedo_sampler, s.yx) * w.z;
        colour = vec4<f32>(albedo.rgb * terrain.cap_tint.rgb, albedo.a);
        normal = n;
        if (terrain.flags & CAP_NORMAL) != 0u {
            normal = normalize(
                from_x(unpack_normal(textureSample(cap_normal, cap_normal_sampler, s.yz)), n) * w.x
                + from_y(unpack_normal(textureSample(cap_normal, cap_normal_sampler, s.xz)), n) * w.y
                + from_z(unpack_normal(textureSample(cap_normal, cap_normal_sampler, s.yx)), n) * w.z
            );
        }
        var spec_r = albedo.r;
        if (terrain.flags & CAP_SIG) != 0u {
            sig = (textureSample(cap_sig, cap_sig_sampler, s.yz).rg * w.x
                + textureSample(cap_sig, cap_sig_sampler, s.xz).rg * w.y
                + textureSample(cap_sig, cap_sig_sampler, s.yx).rg * w.z)
                * vec2<f32>(1.0, terrain.cap_emission);
            spec_r = sig.x;
        }
        spec = spec_r * terrain.cap_spec.rgb;
    }

    // Towards the edge of a patch the colour turns to the border tint, and
    // the alpha falls off.
    let inner = border(weight, colour.a, terrain.inner_range, terrain.inner_offset);
    let rgb = mix(terrain.border_tint.rgb, colour.rgb, inner);
    let alpha = border(weight, colour.a, terrain.border_range, terrain.border_offset);

    pbr_input.material.base_color = vec4<f32>(rgb, alpha);
    pbr_input.material.perceptual_roughness = clamp(1.0 - gloss, 0.089, 1.0);
    // Emission in our units (the light unit is in the lighting parameters).
    let emission_unit = textureLoad(light_params, vec2<i32>(1, 0), 0).w;
    pbr_input.material.emissive = vec4<f32>(rgb * sig.y * emission_unit, 1.0);
    pbr_input.N = normal * flip;
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    let out = deferred_output(in, pbr_input);
#else
    // The game's lighting (game_light.wgsl) instead of Bevy's.
    var surface: GameSurface;
    surface.world = in.world_position.xyz;
    surface.normal = pbr_input.N;
    surface.albedo = pbr_input.material.base_color.rgb;
    surface.specular = spec;
    surface.gloss = gloss;
    surface.unity_ambient = 1.0;
    let lit = game_lighting(surface, in.position, light_params, caustics, caustics_sampler)
        + game_local_lights(surface, in.position, light_params, spot_cookie, spot_cookie_sampler)
        + pbr_input.material.emissive.rgb;
    var out: FragmentOutput;
    out.color = vec4<f32>(lit, pbr_input.material.base_color.a);
#endif
    return out;
}
