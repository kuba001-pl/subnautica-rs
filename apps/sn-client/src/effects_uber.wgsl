// The game's `UWE/Particles/UBER` in the keyword sets we decoded
// (docs/formats/materials.md § `UWE/Particles/UBER` meshes): one program
// whose terms the keywords switch on (FX_FRESNELCLIP, FX_MULMAP,
// FX_SOFTEDGES, FX_DEFORM, FX_REFRACTMAP, FX_TRIPLANAR, FX_NEARCLIP,
// FX_ADDFOG, FX_ALPHAPOW), accumulated into the WBOIT targets A and B like
// the fake light glows. Written for subnautica-rs from the decoded compiled
// programs.

#import bevy_render::view::View
#import sn_client::water_common::{WaterFog, apply_water_fog, water_fog_length}

// One drawn mesh; must match `ParticlesInstance` in effects.rs.
struct ParticleLight {
    position: vec4<f32>,
    color: vec4<f32>,
    spot: vec4<f32>,
    attenuation: vec4<f32>,
}
struct Instance {
    world_from_local: mat4x4<f32>,
    local_from_world: mat4x4<f32>,
    // _Color, linear
    color: vec4<f32>,
    // _ColorStrength, _ColorStrengthAtNight: Vector properties, as stored
    strength: vec4<f32>,
    strength_night: vec4<f32>,
    // _MainTex_ST, _MainTex2_ST, _DeformMap_ST, _RefractMap_ST;
    // brine uses the first three slots for _TriplanarUV xyz scales.
    main_st: vec4<f32>,
    main2_st: vec4<f32>,
    deform_st: vec4<f32>,
    refract_st: vec4<f32>,
    // _MainTex_Speed.xy, _MainTex2_Speed.xy
    speed: vec4<f32>,
    // _DeformMap_Speed.xy, _RefractMap_Speed.xy
    speed2: vec4<f32>,
    // _FresnelFade, _FresnelPow, _Cutoff, flags (1 FX_FRESNELCLIP,
    // 2 FX_MULMAP, 4 FX_SOFTEDGES, 8 FX_DEFORM, 16 FX_REFRACTMAP,
    // 32 FX_TRIPLANAR, 64 FX_NEARCLIP, 128 FX_ADDFOG, 256 FX_ALPHAPOW,
    // 512 back culling, 1024 pixel lighting)
    params: vec4<f32>,
    // _InvFade, _DeformStrength, _RefractStrength, deform Z speed
    strengths: vec4<f32>,
    // _ClipOffset, _ClipFade, main/main2 Z scroll speeds
    clip: vec4<f32>,
    // _AlphaPow, unused ×3
    extra: vec4<f32>,
    lights: array<ParticleLight, 4>,
    sh: array<vec4<f32>, 9>,
    exposure: vec4<f32>,
    ambient_top: vec4<f32>,
    ambient_bottom: vec4<f32>,
    water_lighting: vec4<f32>,
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
    @location(7) world_normal: vec3<f32>,
#ifdef PIXEL_LIGHTING
    @location(8) light0: vec4<f32>,
    @location(9) light1: vec4<f32>,
    @location(10) light2: vec4<f32>,
    @location(11) light3: vec4<f32>,
#endif
}

#ifdef PIXEL_LIGHTING
// Unity's range lookup, the same approximation used by the opaque lights.
fn particle_attenuation(t: f32) -> f32 {
    if t >= 1.0 { return 0.0; }
    var a = 1.0 / (1.0 + 25.0 * t);
    if t > 0.64 { a *= 1.0 - (t - 0.64) / 0.36; }
    return a;
}

fn vertex_light(light: ParticleLight, world: vec3<f32>) -> vec4<f32> {
    if light.color.w == 0.0 { return vec4<f32>(0.0); }
    let delta = light.position.xyz - world * light.position.w;
    let d2 = dot(delta, delta);
    let direction = delta / max(sqrt(d2), 1e-6);
    var attenuation = 1.0;
    if light.position.w != 0.0 {
        attenuation = particle_attenuation(d2 / max(light.attenuation.z, 1e-6));
        if light.spot.w != 0.0 {
            attenuation *= saturate((saturate(dot(direction, light.spot.xyz)) - light.attenuation.x) * light.attenuation.y);
        }
    }
    return vec4<f32>((view.view_from_world * vec4<f32>(direction, 0.0)).xyz, attenuation);
}
#endif

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
    if (u32(inst.params.w) & 64u) != 0u {
        // FX_NEARCLIP: the vertex program fades alpha before interpolation.
        let near = view.clip_from_view[3][2];
        out.color.a *= saturate((out.eye - inst.clip.x - near) / (inst.clip.x + inst.clip.y));
    }
    out.uv = in.uv;
    out.normal = in.normal;
    out.to_camera = normalize(camera - in.position);
    out.instance = in.instance;
    out.world_normal = normalize((transpose(inst.local_from_world) * vec4<f32>(in.normal, 0.0)).xyz);
#ifdef PIXEL_LIGHTING
    out.light0 = vertex_light(inst.lights[0], world.xyz);
    out.light1 = vertex_light(inst.lights[1], world.xyz);
    out.light2 = vertex_light(inst.lights[2], world.xyz);
    out.light3 = vertex_light(inst.lights[3], world.xyz);
#endif
    return out;
}

#ifdef PIXEL_LIGHTING
fn lit_particle_colour(inst: Instance, in: VertexOutput, albedo: vec3<f32>) -> vec3<f32> {
    // The field has no deformation: that uniform slot holds the lighting
    // coefficients (affected, desaturation, normal sharpness, self illumination).
    let p = inst.deform_st;
    let object_normal = normalize(in.normal * vec3<f32>(1.0, 1.0, -1.0));
    let normal = normalize((view.view_from_world * vec4<f32>(in.world_normal, 0.0)).xyz);
    let biased = mix(vec3<f32>(0.0, 0.0, 1.0), normal, p.z);
    let height = in.world.y - fog.misc.y;
    var transmission = vec3<f32>(1.0);
    var emission = vec3<f32>(0.0);
    var water_transmission = 1.0;
    if fog.misc.z != 0.0 && height <= 0.0 && fog.sun.y > 0.0 {
        let path = -height / fog.sun.y;
        let sigma = fog.extinction.rgb * fog.sun.w;
        let grey = min(min(sigma.x, sigma.y), sigma.z);
        let cast_factor = exp(-length(in.world - view.world_position) * inst.water_lighting.x - path * inst.water_lighting.y);
        transmission = exp(-(sigma + cast_factor * (grey - sigma)) * path);
        emission = (fog.emissive.rgb / max(globals.wboit.z, 1e-6)) * inst.water_lighting.w / max(fog.extinction.rgb, vec3<f32>(1e-6));
        water_transmission = mix(1.0, inst.water_lighting.z, saturate(-height));
    }
    var sum = vec3<f32>(0.0);
    let directions = array<vec4<f32>, 4>(in.light0, in.light1, in.light2, in.light3);
    for (var i = 0u; i < 4u; i += 1u) {
        let light = inst.lights[i];
        var color = light.color.rgb;
        if light.position.w == 0.0 {
            if p.x == 0.0 { continue; }
            let ambient = mix(inst.ambient_bottom.rgb, inst.ambient_top.rgb, object_normal.y * 0.5 + 0.5);
            color = (color * transmission + emission) * water_transmission + ambient * transmission;
            let grey_color = dot(color, vec3<f32>(0.3, 0.59, 0.11));
            color = mix(color, vec3<f32>(grey_color), p.y);
        }
        let diffuse = saturate(dot(directions[i].xyz, biased) * inst.clip.z + 1.0 - inst.clip.z);
        sum += color * diffuse * directions[i].w;
    }
    var color = albedo * mix(sum, vec3<f32>(1.0), p.w);
    if p.x < 1.0 {
        let n = object_normal;
        let sh = inst.sh[0].rgb + inst.sh[1].rgb * n.y + inst.sh[2].rgb * n.z + inst.sh[3].rgb * n.x
            + inst.sh[4].rgb * n.x * n.y + inst.sh[5].rgb * n.y * n.z
            + inst.sh[6].rgb * (3.0 * n.z * n.z - 1.0) + inst.sh[7].rgb * n.x * n.z
            + inst.sh[8].rgb * (n.x * n.x - n.y * n.y);
        let ambient = abs(sh) * dot(inst.exposure.rgb, vec3<f32>(1.0 / 3.0));
        let grey = dot(ambient, vec3<f32>(0.3, 0.59, 0.11));
        color += mix(ambient, vec3<f32>(grey), p.y) * (1.0 - p.x);
    }
    return color;
}
#endif

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

// Particle projection differs from SIG: yz, xz, yx, with fourth-power
// normal weights. Explicit gradients allow sampling only the selected path.
fn projected(tex: texture_2d<f32>, smp: sampler, p: vec3<f32>,
             dx: vec3<f32>, dy: vec3<f32>, weights: vec3<f32>) -> vec4<f32> {
    return weights.x * textureSampleGrad(tex, smp, p.yz, dx.yz, dy.yz)
         + weights.y * textureSampleGrad(tex, smp, p.xz, dx.xz, dy.xz)
         + weights.z * textureSampleGrad(tex, smp, p.yx, dx.yx, dy.yx);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> Targets {
    let inst = instances.items[in.instance];
    let fresnel_fade = inst.params.x;
    let fresnel_pow = inst.params.y;
    let cutoff = inst.params.z;
    let flags = u32(inst.params.w);
    let triplanar = (flags & 32u) != 0u;
    let add_fog = (flags & 128u) != 0u;
    // The custom effect buffer keeps its original indices. A reflection
    // reverses raster facing, so apply back culling in model-space facing.
    let reflected = determinant(mat3x3<f32>(inst.world_from_local[0].xyz,
        inst.world_from_local[1].xyz, inst.world_from_local[2].xyz)) < 0.0;
    if (flags & 512u) != 0u && front == reflected {
        discard;
    }

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
    if (flags & 256u) != 0u {
        // FX_ALPHAPOW: on the starting alpha, before any texture.
        c.a = exp2(log2(c.a) * inst.extra.x);
    }
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
    // Restore Unity world coordinates after the mesh/placement Z reflection.
    let world = in.world * vec3<f32>(1.0, 1.0, -1.0);
    let n2 = in.world_normal * in.world_normal;
    let n4 = n2 * n2;
    let weights = n4 / max(dot(n4, vec3<f32>(1.0)), 1e-20);
    let deform_p = world * inst.deform_st.xyz + fract(t * vec3<f32>(inst.speed2.xy, inst.strengths.w));
    let deform_dx = dpdx(deform_p);
    let deform_dy = dpdy(deform_p);
    var displaced = world;
    if triplanar && (flags & 8u) != 0u {
        let d = projected(deform_map, deform_sampler, deform_p, deform_dx, deform_dy, weights);
        displaced += (d.xyz - 1.0) * inst.strengths.y;
    }
    let main_p = displaced * inst.main_st.xyz + fract(t * vec3<f32>(inst.speed.xy, inst.clip.z));
    let main2_p = displaced * inst.main2_st.xyz + fract(t * vec3<f32>(inst.speed.zw, inst.clip.w));
    let main_dx = dpdx(main_p);
    let main_dy = dpdy(main_p);
    let main2_dx = dpdx(main2_p);
    let main2_dy = dpdy(main2_p);
    if triplanar {
        c *= projected(main_tex, main_sampler, main_p, main_dx, main_dy, weights);
        c *= projected(main_tex2, main_sampler2, main2_p, main2_dx, main2_dy, weights);
    } else {
        c *= tex;
        if (flags & 2u) != 0u {
            c *= tex2;
        }
    }
    // Day and night strengths by the local light.
    let strength = mix(inst.strength, inst.strength_night, 1.0 - globals.misc.y);
    c *= strength;
    if c.a - cutoff < 0.0 {
        discard;
    }
    // FX_ADDFOG: opacity follows brightness (dark texels are transparent);
    // without it, ordinary alpha.
    var alpha = c.a;
    if add_fog {
        alpha = saturate((c.r + c.g + c.b) / 3.0) * c.a;
    }
    // FX_REFRACTMAP: a screen offset for the composite (B.yz), unpacked like
    // a normal map, × _RefractStrength × the unclamped alpha above.
    var offset = vec2<f32>(0.0);
    if (flags & 16u) != 0u {
        let n = vec2<f32>(refract.a * refract.r, refract.g) * 2.0 - 1.0;
        offset = n * inst.strengths.z * c.a;
    }
    // `_Color` and the textures are in game light units.
    var lit_colour = c.rgb;
#ifdef PIXEL_LIGHTING
    if (flags & 1024u) != 0u {
        lit_colour = lit_particle_colour(inst, in, lit_colour);
    }
#endif
    var colour = lit_colour * globals.wboit.z;

    // Every decoded program receives fog, with or without FX_ADDFOG.
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
    if !add_fog {
        // Without FX_ADDFOG colour saturates in game light units after fog.
        colour = clamp(colour, vec3<f32>(0.0), vec3<f32>(globals.wboit.z));
    }

    // The WBOIT weight: nearer meshes count more.
    let depth_weight = clamp(exp(-in.eye * globals.wboit.y), 0.01, 1.0);
    let w = globals.wboit.x * (alpha * depth_weight - 1.0) + 1.0;
    var out: Targets;
    out.a = vec4<f32>(w * alpha * colour, alpha);
    out.b = vec4<f32>(alpha * w, offset, 0.0);
    return out;
}
