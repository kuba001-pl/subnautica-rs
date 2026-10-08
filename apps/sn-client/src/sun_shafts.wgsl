// Light shafts under water ("god rays"), following the game's
// `WaterSunShaftsOnCamera` image effect (docs/formats/lighting.md § Light
// shafts): a ray march through the caustics texture projected along the
// sun, at half resolution, then added to the image. Written for
// subnautica-rs from the behaviour of the compiled shader. World space
// (Bevy); the game works in view space (same result).

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_pbr::mesh_view_bindings::{view, lights}
#import bevy_pbr::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
#import bevy_pbr::shadows::fetch_directional_shadow
#import sn_client::water_common::WaterFog

// Must match `SunShaftsUniform` in sun_shafts.rs.
struct SunShafts {
    // world → light space rows (Bevy world), w: offset
    light_x: vec4<f32>,
    light_y: vec4<f32>,
    light_z: vec4<f32>,
    // start distance, max distance, shafts scale, intensity
    params: vec4<f32>,
    // trace step, caustics amount (texture scale), caustics frame, on
    trace: vec4<f32>,
    // light colour (our units), unused
    light: vec4<f32>,
    // colour cast (distance, depth factor), unused, unused
    colour_cast: vec4<f32>,
}

// Group 0: Bevy's view bindings (view, lights, shadow maps).
@group(1) @binding(0) var<uniform> fog: WaterFog;
@group(1) @binding(1) var<uniform> shafts: SunShafts;
#ifdef MULTISAMPLED
@group(1) @binding(2) var depth: texture_depth_multisampled_2d;
#else
@group(1) @binding(2) var depth: texture_depth_2d;
#endif
@group(1) @binding(3) var caustics: texture_2d_array<f32>;
@group(1) @binding(4) var caustics_sampler: sampler;
@group(1) @binding(5) var shafts_texture: texture_2d<f32>;
@group(1) @binding(6) var shafts_sampler: sampler;

fn light_space(p: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(dot(shafts.light_x, p), dot(shafts.light_y, p), dot(shafts.light_z, p));
}

@fragment
fn trace(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    if shafts.trace.w == 0.0 || fog.misc.z == 0.0 {
        return vec4<f32>(0.0);
    }
    let full = vec2<i32>(view.viewport.zw);
    let pixel = clamp(vec2<i32>(in.uv * view.viewport.zw), vec2<i32>(0), full - 1);
    let d = textureLoad(depth, pixel, 0);
    let camera = view.world_position;
    let ndc = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, max(d, 1e-6), 1.0);
    let wp = view.world_from_clip * ndc;
    let towards = wp.xyz / wp.w - camera;
    let v = normalize(towards);
    let dist = select(length(towards), 1e6, d <= 0.0);

    let start = shafts.params.x;
    var t_end = min(dist, shafts.params.y);
    var t_start = start;
    let height = camera.y - fog.misc.y;
    if height > 0.0 {
        // Above the water: only rays going down into it.
        if v.y >= 0.0 {
            return vec4<f32>(0.0);
        }
        t_start = max(height / -v.y, start);
    } else {
        // Below: up to the surface.
        let to_surface = -height / v.y;
        if to_surface > 0.0 {
            t_end = min(to_surface, t_end);
        }
    }
    if t_end < t_start {
        return vec4<f32>(0.0);
    }

    let sigma_t = fog.extinction.rgb;
    let step = shafts.trace.x;
    let scale = shafts.params.z;
    let frame = i32(shafts.trace.z);
    let p_start = camera + v * t_start;
    var uv = light_space(vec4<f32>(p_start, 1.0)).xy * scale;
    let dir = light_space(vec4<f32>(v, 0.0));
    let duv = dir.xy * step * scale;
    var sum = vec3<f32>(0.0);
    // Each sample is lit only where the sun's shadow map sees it.
    let shadows = lights.n_directional_lights > 0u
        && (lights.directional_lights[0].flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u;
    let to_sun = normalize(fog.sun.xyz);
    for (var t = t_start; t < t_end; t += step) {
        var lit = 1.0;
        if shadows {
            let p = vec4<f32>(camera + v * t, 1.0);
            let view_z = (view.view_from_world * p).z;
            lit = fetch_directional_shadow(0u, p, to_sun, view_z, in.position.xy);
        }
        let c = textureSampleLevel(caustics, caustics_sampler, uv, frame, 0.0).rgb;
        sum += c * lit * exp(-sigma_t * t);
        uv += duv;
    }

    // Stronger looking towards the sun.
    let f0 = saturate((dir.z - 1.0) * -0.8);
    let facing = f0 * f0 * (3.0 - 2.0 * f0);
    var shaft = facing * shafts.light.rgb * step * fog.scattering.rgb * fog.scattering.w
        * shafts.trace.y * sum;
    // Sunlight dimmed from the surface to the start, with the colour cast.
    let path = (p_start.y - fog.misc.y) / -fog.sun.y;
    let sigma_sun = sigma_t * fog.sun.w;
    let grey = min(min(sigma_sun.x, sigma_sun.y), sigma_sun.z);
    let cast_factor = exp(-t_start * shafts.colour_cast.x - path * shafts.colour_cast.y);
    let att = exp(-path * (sigma_sun + cast_factor * (grey - sigma_sun)));
    return vec4<f32>(shaft * att * shafts.params.w, 1.0);
}

// The shafts added to the image (additive blending).
@fragment
fn combine(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(shafts_texture, shafts_sampler, in.uv, 0.0).rgb, 0.0);
}
