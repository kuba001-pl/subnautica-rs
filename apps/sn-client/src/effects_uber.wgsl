// The game's `UWE/Particles/UBER` in the variants we decoded
// (docs/formats/materials.md § `UWE/Particles/UBER` meshes): keywords
// FX_ADDFOG FX_SCROLL WBOIT, with or without FX_MULMAP (a second texture)
// and FX_FRESNELCLIP (the Precursor consoles' holograms), or with FX_DEFORM
// FX_MULMAP FX_SOFTEDGES and with or without FX_REFRACTMAP (the doors' force
// fields); accumulated into the WBOIT targets A and B like the fake light
// glows. Written for subnautica-rs from the decoded compiled programs.

#import bevy_render::view::View
#import sn_client::water_common::{WaterFog, apply_water_fog, water_fog_length}

// One drawn mesh; must match `ParticlesInstance` in effects.rs.
struct Instance {
    world_from_local: mat4x4<f32>,
    local_from_world: mat4x4<f32>,
    // _Color, linear
    color: vec4<f32>,
    // _ColorStrength, _ColorStrengthAtNight: Vector properties, as stored
    strength: vec4<f32>,
    strength_night: vec4<f32>,
    // _MainTex_ST, _MainTex2_ST, _DeformMap_ST, _RefractMap_ST
    main_st: vec4<f32>,
    main2_st: vec4<f32>,
    deform_st: vec4<f32>,
    refract_st: vec4<f32>,
    // _MainTex_Speed.xy, _MainTex2_Speed.xy
    speed: vec4<f32>,
    // _DeformMap_Speed.xy, _RefractMap_Speed.xy
    speed2: vec4<f32>,
    // _FresnelFade, _FresnelPow, _Cutoff, flags (1 FX_FRESNELCLIP,
    // 2 FX_MULMAP, 4 FX_SOFTEDGES, 8 FX_DEFORM, 16 FX_REFRACTMAP)
    params: vec4<f32>,
    // _InvFade, _DeformStrength, _RefractStrength, unused
    strengths: vec4<f32>,
}

struct Instances {
    items: array<Instance>,
}

// Must match `EffectGlobals` in effects.rs.
struct Globals {
    // depth weighting on (1) or off, its sharpness, our light unit, unused
    wboit: vec4<f32>,
    // `_Time.y` (s), `_UweLocalLightScalar`, unused, unused
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> fog: WaterFog;
@group(0) @binding(2) var<uniform> globals: Globals;
@group(0) @binding(3) var<storage, read> instances: Instances;
#ifdef MULTISAMPLED
@group(0) @binding(4) var scene_depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(4) var scene_depth: texture_depth_2d;
#endif
@group(1) @binding(0) var main_tex: texture_2d<f32>;
@group(1) @binding(1) var main_sampler: sampler;
@group(1) @binding(2) var main_tex2: texture_2d<f32>;
@group(1) @binding(3) var main_sampler2: sampler;
@group(1) @binding(4) var deform_map: texture_2d<f32>;
@group(1) @binding(5) var deform_sampler: sampler;
@group(1) @binding(6) var refract_map: texture_2d<f32>;
@group(1) @binding(7) var refract_sampler: sampler;

struct VertexInput {
    @builtin(instance_index) instance: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    // Eye depth (m).
    @location(1) eye: f32,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    // Object space, as stored (not normalised, as in the game).
    @location(4) normal: vec3<f32>,
    // Object space, towards the camera, normalised per vertex.
    @location(5) to_camera: vec3<f32>,
    @location(6) @interpolate(flat) instance: u32,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    let inst = instances.items[in.instance];
    let world = inst.world_from_local * vec4<f32>(in.position, 1.0);
    let camera = (inst.local_from_world * vec4<f32>(view.world_position, 1.0)).xyz;
    var out: VertexOutput;
    out.position = view.clip_from_world * world;
    out.world = world.xyz;
    out.eye = -(view.view_from_world * world).z;
    out.color = in.color;
    out.uv = in.uv;
    out.normal = in.normal;
    out.to_camera = normalize(camera - in.position);
    out.instance = in.instance;
    return out;
}

struct Targets {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
}

// Linear depth (m) of what the scene shows at `pixel`; the sky is far away.
fn scene_eye_depth(pixel: vec2<i32>) -> f32 {
    let size = vec2<i32>(view.viewport.zw);
    let d = textureLoad(scene_depth, clamp(pixel, vec2<i32>(0), size - 1), 0);
    if d <= 0.0 {
        return 1e7;
    }
    return view.clip_from_view[3][2] / d;
}

@fragment
fn fragment(in: VertexOutput) -> Targets {
    let inst = instances.items[in.instance];
    let fresnel_fade = inst.params.x;
    let fresnel_pow = inst.params.y;
    let cutoff = inst.params.z;
    let flags = u32(inst.params.w);

    // The game's depth test (ZTest Less, no ZWrite) against the scene.
    let scene = scene_eye_depth(vec2<i32>(in.position.xy));
    if scene <= in.eye {
        discard;
    }
    let t = globals.misc.x;
    var c = vec4<f32>(2.0 * inst.color.rgb, 2.0 * inst.color.a);
    if (flags & 4u) != 0u {
        // FX_SOFTEDGES: fades where the mesh meets the scene.
        c.a *= saturate((scene - in.eye) * inst.strengths.x);
    }
    if (flags & 1u) != 0u {
        // FX_FRESNELCLIP: a positive fade keeps what faces the camera, a
        // negative one the rim; `log`/`exp` as the game computes `pow`.
        let facing = dot(normalize(in.to_camera), in.normal);
        let rim = select(0.0, 1.0, fresnel_fade < 0.0);
        let f = exp2(log2(abs(rim - abs(facing))) * fresnel_pow) * abs(fresnel_fade);
        c.a *= min(f, 1.0);
    }
    c *= in.color;
    // FX_DEFORM: a scrolling map shifts the uv of every other texture by
    // (map.xy − 1) × _DeformStrength.
    let deform_uv = in.uv * inst.deform_st.xy + inst.deform_st.zw + fract(t * inst.speed2.xy);
    let deform = textureSample(deform_map, deform_sampler, deform_uv).xy;
    var base = in.uv;
    if (flags & 8u) != 0u {
        base += (deform - 1.0) * inst.strengths.y;
    }
    // FX_SCROLL: each texture's uv moves by frac(t × speed). All are
    // sampled here (texture lookups must stay in uniform control flow);
    // each counts only with its keyword, as in the game's variants.
    let uv = base * inst.main_st.xy + inst.main_st.zw + fract(t * inst.speed.xy);
    let uv2 = base * inst.main2_st.xy + inst.main2_st.zw + fract(t * inst.speed.zw);
    let refract_uv = base * inst.refract_st.xy + inst.refract_st.zw + fract(t * inst.speed2.zw);
    let refract = textureSample(refract_map, refract_sampler, refract_uv);
    let tex = textureSample(main_tex, main_sampler, uv);
    let tex2 = textureSample(main_tex2, main_sampler2, uv2);
    c *= tex;
    if (flags & 2u) != 0u {
        c *= tex2;
    }
    // Day and night strengths by the local light.
    let strength = mix(inst.strength, inst.strength_night, 1.0 - globals.misc.y);
    c *= strength;
    if c.a - cutoff < 0.0 {
        discard;
    }
    // Opacity follows brightness: dark texels are transparent.
    var alpha = saturate((c.r + c.g + c.b) / 3.0) * c.a;
    // FX_REFRACTMAP: a screen offset for the composite (B.yz), unpacked like
    // a normal map, × _RefractStrength × the unclamped alpha above.
    var offset = vec2<f32>(0.0);
    if (flags & 16u) != 0u {
        let n = vec2<f32>(refract.a * refract.r, refract.g) * 2.0 - 1.0;
        offset = n * inst.strengths.z * c.a;
    }
    // `_Color` and the textures are in game light units.
    var colour = c.rgb * globals.wboit.z;

    // FX_ADDFOG: fog at its own distance, faded out over its path through
    // fogged water, as the glows.
    if fog.misc.z != 0.0 {
        let camera = view.world_position;
        let towards = in.world - camera;
        let dist = length(towards);
        let v = towards / max(dist, 1e-6);
        colour = apply_water_fog(fog, colour, camera, v, dist, false);
        let water = water_fog_length(fog, camera, v, dist);
        alpha *= saturate(10.0 - 0.08 * water);
        // The offset fades with the blue extinction over that path.
        offset *= exp(-fog.extinction.b * water);
    }
    alpha = saturate(alpha);

    // The WBOIT weight: nearer meshes count more.
    let depth_weight = clamp(exp(-in.eye * globals.wboit.y), 0.01, 1.0);
    let w = globals.wboit.x * (alpha * depth_weight - 1.0) + 1.0;
    var out: Targets;
    out.a = vec4<f32>(w * alpha * colour, alpha);
    out.b = vec4<f32>(alpha * w, offset, 0.0);
    return out;
}
