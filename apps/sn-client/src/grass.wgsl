// Terrain grass: a port of the game's `UWE/SIG Terrain Grass` (its deferred
// pass, read from the compiled shader; docs/formats/terrain-materials.md
// § Grass shaders), also used for `UWE/SIG` (waves, mask and gradient off)
// and `UWE/SIG AlphaCutout + Noisey Wave` (its own sway).
// Lit by the game's light pass (game_light.wgsl). Written for subnautica-rs.
//
// Vertex colours (made by sn_terrain::build_grass): red = wave direction,
// blue = wave amount, alpha = height above the ground ÷ 5. The second uv set
// holds the position along `_ObjectUp` in the game's grass object (chunk).

#import bevy_pbr::{
    forward_io::{Vertex, VertexOutput, FragmentOutput},
    mesh_functions,
    mesh_view_bindings::globals,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{apply_normal_mapping, calculate_tbn_mikktspace},
    pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT,
    view_transformations::position_world_to_clip,
}
#import sn_client::game_light::{GameSurface, game_lighting, game_local_lights, local_light_scalar}

// Must match `GrassParams` in grass_look.rs.
struct GrassParams {
    // Linear colours: _Color, _Color2, _BotColor, _BotColor2.
    color: vec4<f32>,
    color2: vec4<f32>,
    bot_color: vec4<f32>,
    bot_color2: vec4<f32>,
    // _GradientParams (x: offset, y: scale).
    gradient: vec4<f32>,
    // _SIGstr: specular, glow by day, gloss, glow by night.
    sig_str: vec4<f32>,
    // _SpecColor (linear).
    spec_color: vec4<f32>,
    // _WaveAmount, _WaveSpeed, _TimeOffset, _ForceNormals.
    wave: vec4<f32>,
    // Noisey Wave: _WorldWaveDir (Unity axes).
    wave_dir: vec4<f32>,
    // _MaskScale, _MaskStr, _Cutoff, 1 with a mask texture.
    mask: vec4<f32>,
    // Texture coordinates: uv * scale + offset (xy, zw).
    main_st: vec4<f32>,
    bump_st: vec4<f32>,
    sig_st: vec4<f32>,
    mask_st: vec4<f32>,
    // 1: normal map; 2: SIG map; 4: SIG values from the albedo map
    // (`UWE/SIG` without its keyword); 8: Noisey Wave's sway.
    flags: u32,
}

const HAS_NORMAL: u32 = 1u;
const HAS_SIG: u32 = 2u;
const SIG_FROM_ALBEDO: u32 = 4u;
const NOISE_WAVE: u32 = 8u;

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> grass: GrassParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var albedo_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var normal_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var sig_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var sig_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var mask_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var mask_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(120) var light_params: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var caustics: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var caustics_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(123) var spot_cookie: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(124) var spot_cookie_sampler: sampler;

// The sway of a vertex, in Unity object (= world-aligned) space.
fn wave_offset(color: vec4<f32>) -> vec3<f32> {
    let height = color.a * 5.0;
    let period = 5.0 - 5.0 * grass.wave.y;
    let t = (globals.time + grass.wave.z * 0.1 - height) * 6.28318 / period;
    let sway = sin(t) * color.b * grass.wave.x;
    let angle = color.r * 0.783185 + 5.5;
    return vec3<f32>(cos(angle), 0.0, sin(angle)) * height * sway;
}

// Noisey Wave's 2D simplex noise of a Unity world (x, z), times 130, as
// its vertex program computes it.
fn mod289_2(x: vec2<f32>) -> vec2<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn mod289_3(x: vec3<f32>) -> vec3<f32> {
    return x - floor(x * (1.0 / 289.0)) * 289.0;
}

fn permute(x: vec3<f32>) -> vec3<f32> {
    return mod289_3((x * 34.0 + 1.0) * x);
}

fn simplex(v: vec2<f32>) -> f32 {
    let c = vec4<f32>(0.21132487, 0.36602542, -0.57735026, 0.024390243);
    var i = floor(v + dot(v, c.yy));
    let x0 = v - i + dot(i, c.xx);
    let i1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), x0.x > x0.y);
    let x1 = x0 + c.xx - i1;
    let x2 = x0 + c.zz;
    i = mod289_2(i);
    let p = permute(permute(i.y + vec3<f32>(0.0, i1.y, 1.0)) + i.x + vec3<f32>(0.0, i1.x, 1.0));
    var m = max(vec3<f32>(0.5) - vec3<f32>(dot(x0, x0), dot(x1, x1), dot(x2, x2)), vec3<f32>(0.0));
    m = m * m;
    m = m * m;
    let x = 2.0 * fract(p * c.www) - 1.0;
    let h = abs(x) - 0.5;
    let a0 = x - floor(x + 0.5);
    m *= 1.792843 - 0.853735 * (a0 * a0 + h * h);
    let g = vec3<f32>(a0.x * x0.x + h.x * x0.y, a0.y * x1.x + h.y * x1.y, a0.z * x2.x + h.z * x2.y);
    return 130.0 * dot(m, g);
}

// Noisey Wave's sway, a world offset in Unity axes: along _WorldWaveDir by
// _WaveAmount × the position along _ObjectUp in the chunk (`along`).
fn noise_wave_offset(xz: vec2<f32>, along: f32) -> vec3<f32> {
    let period = 5.0 - 5.0 * grass.wave.y;
    let t = (globals.time + grass.wave.z - simplex(xz)) * 6.28318 / period;
    return grass.wave_dir.xyz * (sin(t) * grass.wave.x * along);
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var local = vertex.position;
    var normal = vertex.normal;
#ifdef VERTEX_COLORS
    // Our meshes are in Bevy space: Unity's z is mirrored.
    if (grass.flags & NOISE_WAVE) == 0u {
        let w = wave_offset(vertex.color);
        local += vec3<f32>(w.x, 0.0, -w.z);
    }
    out.color = vertex.color;
#endif
#ifdef VERTEX_UVS_B
    if (grass.flags & NOISE_WAVE) != 0u {
        // The noise at the unmoved world position. Grass meshes sit at the
        // origin: local = world.
        let p = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0)).xyz;
        let w = noise_wave_offset(vec2<f32>(p.x, -p.z), vertex.uv_b.x);
        local += vec3<f32>(w.x, w.y, -w.z);
    }
#endif
    // _ForceNormals bends the normals towards up.
    normal = mix(normal, vec3<f32>(0.0, 1.0, 0.0), grass.wave.w);
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(local, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    out.world_normal = mesh_functions::mesh_normal_local_to_world(normal, vertex.instance_index);
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_TANGENTS
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex.instance_index);
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
    // The mask (sampled per vertex at the unswayed position, Unity x/z) is
    // passed in uv_b.x.
#ifdef VERTEX_UVS_B
    var mask = 0.0;
    if grass.mask.w != 0.0 {
        let p = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0)).xyz;
        let uv = vec2<f32>(p.x, -p.z) / grass.mask.x * grass.mask_st.xy + grass.mask_st.zw;
        mask = saturate(textureSampleLevel(mask_map, mask_sampler, uv, 0.0).r * grass.mask.y);
    }
    out.uv_b = vec2<f32>(mask, 0.0);
#endif
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    var uv = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
    var mask = 0.0;
#ifdef VERTEX_UVS_B
    mask = in.uv_b.x;
#endif
    let tex = textureSample(albedo_map, albedo_sampler, uv * grass.main_st.xy + grass.main_st.zw);
    if tex.a < grass.mask.z {
        discard;
    }
    // Tint: top and bottom colours by the mask, blended by height in the
    // texture (its v coordinate).
    let top = mix(grass.color.rgb, grass.color2.rgb, mask);
    let bottom = mix(grass.bot_color.rgb, grass.bot_color2.rgb, mask);
    let g = saturate((uv.y + grass.gradient.x) * grass.gradient.y);
    let albedo = tex.rgb * mix(bottom, top, g);

    var n = normalize(pbr_input.N);
#ifdef VERTEX_TANGENTS
#ifdef VERTEX_UVS_A
    if (grass.flags & HAS_NORMAL) != 0u {
        let t = textureSample(normal_map, normal_sampler, uv * grass.bump_st.xy + grass.bump_st.zw);
        let xy = vec2<f32>(t.a * t.r, t.g) * 2.0 - 1.0;
        let nt = vec3<f32>(xy, sqrt(1.0 - min(dot(xy, xy), 1.0)));
        let tbn = calculate_tbn_mikktspace(in.world_normal, in.world_tangent);
        let double_sided = (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
        n = normalize(apply_normal_mapping(pbr_input.material.flags, tbn, double_sided, is_front, nt));
    }
#endif
#endif

    // SIG: specular (R), glow (G), gloss (B).
    var sig = vec3<f32>(0.0);
    if (grass.flags & HAS_SIG) != 0u {
        sig = textureSample(sig_map, sig_sampler, uv * grass.sig_st.xy + grass.sig_st.zw).rgb;
    } else if (grass.flags & SIG_FROM_ALBEDO) != 0u {
        sig = tex.rgb;
    }
    let glow = sig.g * mix(grass.sig_str.y, grass.sig_str.w, 1.0 - local_light_scalar(light_params));
    // Emission in our units (the light unit is in the lighting parameters).
    let emission_unit = textureLoad(light_params, vec2<i32>(1, 0), 0).w;

    var surface: GameSurface;
    surface.world = in.world_position.xyz;
    surface.normal = n;
    surface.albedo = albedo;
    surface.specular = sig.r * grass.sig_str.x * grass.spec_color.rgb;
    surface.gloss = max(sig.b * grass.sig_str.z, 0.01);
    surface.unity_ambient = 1.0;
    let colour = albedo * glow * emission_unit
        + game_lighting(surface, in.position, light_params, caustics, caustics_sampler)
        + game_local_lights(surface, in.position, light_params, spot_cookie, spot_cookie_sampler);
    var out: FragmentOutput;
    out.color = vec4<f32>(colour, 1.0);
    return out;
}
