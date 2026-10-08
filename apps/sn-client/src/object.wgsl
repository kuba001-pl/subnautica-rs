// World-object shader: Bevy's standard material inputs (albedo, tint,
// alpha), the game's normal maps, which keep X in alpha and Y in green
// ("DXT5nm") and so can't use Bevy's own normal mapping, and a port of the
// game's object shader, MarmosetUBER (its deferred pass, read from the
// compiled shader; docs/formats/lighting.md § Objects): specular colour
// and gloss for the light pass, glow, and the Marmoset sky's ambient. Lit by
// the game's light pass (game_light.wgsl). Written for subnautica-rs.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_normal_mapping, calculate_tbn_mikktspace},
    pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT,
    mesh_view_bindings::view,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::forward_io::{VertexOutput, FragmentOutput}
#import sn_client::game_light::{GameSurface, game_lighting, local_light_scalar}
#endif

// Must match `ObjectParams` in object_look.rs.
struct ObjectParams {
    // Texture coordinates: uv * scale + offset (xy, zw).
    normal_st: vec4<f32>,
    spec_st: vec4<f32>,
    illum_st: vec4<f32>,
    // _SpecColor (linear), _SpecInt.
    spec_color: vec4<f32>,
    // _GlowColor (linear).
    glow_color: vec4<f32>,
    // _Shininess, _Fresnel, _IBLreductionAtNight, _EnableSimpleGlass.
    surface: vec4<f32>,
    // _GlowStrength, _GlowStrengthNight, _EmissionLM, _EmissionLMNight.
    glow: vec4<f32>,
    // The sky's _ExposureIBL: diffuse, specular, sky, camera exposure.
    exposure: vec4<f32>,
    // The sky's rotation (quaternion, Unity coordinates).
    sky_rotation: vec4<f32>,
    // _AffectedByDayNightCycle, _Outdoors, 1 for a MarmosetUBER material.
    sky_flags: vec4<f32>,
    // _SH0 … _SH8.
    sh: array<vec4<f32>, 9>,
    has_normal: u32,
    has_spec: u32,
    has_illum: u32,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> object: ObjectParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var normal_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var spec_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var spec_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var illum_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var illum_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(120) var light_params: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var caustics: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var caustics_sampler: sampler;

fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// Marmoset's 9-coefficient SH ambient at unit normal `n` (the sky's frame).
fn sky_sh(n: vec3<f32>) -> vec3<f32> {
    let s = object.sh;
    var c = s[0].xyz + s[1].xyz * n.y + s[2].xyz * n.z + s[3].xyz * n.x;
    c += s[4].xyz * (n.x * n.y) + s[5].xyz * (n.z * n.y) + s[7].xyz * (n.x * n.z);
    c += s[6].xyz * (3.0 * n.z * n.z - 1.0) + s[8].xyz * (n.x * n.x - n.y * n.y);
    return abs(c);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);

    var uv = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
    uv = in.uv;
#endif
#ifdef VERTEX_TANGENTS
#ifdef VERTEX_UVS_A
    if object.has_normal != 0u {
        let t = textureSample(normal_map, normal_sampler, uv * object.normal_st.xy + object.normal_st.zw);
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
    let base = pbr_input.material.base_color;
    let n = normalize(pbr_input.N);
    var surface: GameSurface;
    surface.world = in.world_position.xyz;
    surface.normal = n;
    surface.albedo = base.rgb;
    surface.specular = vec3<f32>(0.0);
    surface.gloss = 0.0;
    surface.unity_ambient = 1.0;
    var lit = true;
    var emission = vec3<f32>(0.0);

    if object.sky_flags.z != 0.0 {
        // MarmosetUBER's G-buffer.
        let cam_exposure = object.exposure.w;
        // Simple glass scales everything by the albedo's alpha.
        let glass = 1.0 + object.surface.w * (base.a - 1.0);
        let diffuse = base.rgb * cam_exposure * glass;

        // Specular: map × colour × intensity × fresnel; gloss (map alpha)
        // and shininess give a mip level, the light pass a power.
        var spec_tex = vec4<f32>(1.0);
        if object.has_spec != 0u {
            spec_tex = textureSample(spec_map, spec_sampler, uv * object.spec_st.xy + object.spec_st.zw);
        }
        let v = normalize(view.world_position - in.world_position.xyz);
        let f = saturate(1.25 - abs(dot(n, v)) * object.surface.y);
        let fresnel = f * f * f * f * f * object.spec_color.w;
        let spec = fresnel * spec_tex.rgb * object.spec_color.rgb * cam_exposure;
        let rough = (1.0 - spec_tex.a) * (1.0 - spec_tex.a);
        let lod = 8.0 - rough - object.surface.x * (1.0 - rough);
        let power = exp2(8.0 - lod);
        surface.albedo = diffuse;
        surface.specular = spec * (power * 0.159155 + 0.31831) * 0.125;
        surface.gloss = power * 0.015625;
        surface.unity_ambient = 0.0;

        // Skies not affected by the day/night cycle (caves, interiors) mark
        // the surface unlit for the light pass and add their own ambient.
        let affected = object.sky_flags.x;
        let outdoors = object.sky_flags.y;
        lit = ((1.0 - outdoors) + 2.0 * (1.0 - affected)) / 3.0 < 0.5;

        // Glow: day and night strengths by the light level.
        let night = (1.0 - local_light_scalar(light_params)) * affected;
        let strength = mix(object.glow.xz, object.glow.yw, night);
        let ibl_reduction = min(max(night, 0.0), object.surface.z);
        if object.has_illum != 0u {
            let illum = textureSample(illum_map, illum_sampler, uv * object.illum_st.xy + object.illum_st.zw) * glass;
            emission = illum.rgb * object.glow_color.rgb * strength.x * cam_exposure
                + diffuse * illum.a * strength.y;
        }
        if affected <= 0.0 {
            let ns = normalize(rotate(object.sky_rotation, vec3<f32>(n.x, n.y, -n.z)));
            emission += sky_sh(ns) * object.exposure.x * (1.0 - ibl_reduction) * diffuse;
        }
        // Not yet: the reflection of the sky's specular cube.
    }

    var colour = emission;
    if lit {
        colour += game_lighting(surface, in.position, light_params, caustics, caustics_sampler);
    }
    var out: FragmentOutput;
    out.color = vec4<f32>(colour, base.a);
#endif
    return out;
}
