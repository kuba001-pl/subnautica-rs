// Underwater fog: a full-screen pass over the lit (HDR) image, following the
// game's water fog model (docs/formats/water.md § The underwater fog).
// Written for subnautica-rs; works in world space with the water plane at
// y = water_level (the game works in view space; same result).

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_render::view::View

// Must match `WaterFog` in water.rs. Coefficients are per metre and already
// include the above-water density scale; light values are in our units.
struct WaterFog {
    // σt (rgb), start distance
    extinction: vec4<f32>,
    // σs (rgb), light scale (sunlight scale × transmission)
    scattering: vec4<f32>,
    // emissive (rgb), unused
    emissive: vec4<f32>,
    // direction towards the sun (world), sun attenuation
    sun: vec4<f32>,
    // sun × amount + top ambient (rgb), unused
    light: vec4<f32>,
    // Henyey–Greenstein constants c0, c1, c2; unused
    phase: vec4<f32>,
    // sky fog colour (rgb), sky fog density
    sky: vec4<f32>,
    // above-water start distance, water level, enabled, unused
    misc: vec4<f32>,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
#ifdef MULTISAMPLED
@group(0) @binding(2) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(2) var depth: texture_depth_2d;
#endif
@group(0) @binding(3) var<uniform> view: View;
@group(0) @binding(4) var<uniform> fog: WaterFog;

fn world_at(uv: vec2<f32>, ndc_depth: f32) -> vec3<f32> {
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, ndc_depth, 1.0);
    let p = view.world_from_clip * ndc;
    return p.xyz / p.w;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let colour = textureSample(screen, screen_sampler, in.uv);
    if fog.misc.z == 0.0 {
        return colour;
    }
    let pixel = vec2<i32>(in.position.xy);
    // Sample 0 when multisampled, mip 0 otherwise: same call.
    let d = textureLoad(depth, pixel, 0);
    let camera = view.world_position;
    // Reverse-Z with an infinite far plane: 0 is the sky.
    let sky = d <= 0.0;
    let towards = world_at(in.uv, select(d, 1e-4, sky)) - camera;
    let v = normalize(towards);
    let dist = select(length(towards), 1e6, sky);

    let c = colour.rgb;
    let water_level = fog.misc.y;
    let height = camera.y - water_level;
    let sky_colour = fog.sky.rgb;
    let sky_density = fog.sky.w;

    var base = c;
    var water_dist = dist;
    var start = fog.extinction.w;
    var result: vec3<f32>;
    var skip = false;
    if height > 0.0 {
        // Above the water: air fog, and water fog only for rays going down
        // into the water before they hit anything.
        let down = v.y < -0.01;
        let air = mix(sky_colour, c, exp(-dist * sky_density));
        result = select(air, c, sky);
        if down {
            start = height / -v.y + fog.misc.x;
            skip = dist < start;
        } else {
            start = fog.misc.x;
            skip = true;
        }
    } else {
        // Below: rays that reach the surface see what is behind it through
        // air fog, and travel only up to the surface in water.
        let to_surface = -height / v.y;
        if to_surface > 0.0 {
            let air = mix(sky_colour, c, exp(-max(dist - to_surface, 0.0) * sky_density));
            base = select(air, c, sky);
            water_dist = min(dist, to_surface);
        }
        result = base;
    }

    if !skip && start < water_dist {
        let t = water_dist - start;
        let sigma_t = max(fog.extinction.rgb, vec3<f32>(1e-6));
        let sigma_s = fog.scattering.rgb;
        let to_sun = fog.sun.xyz;
        let cos_theta = dot(v, -to_sun);
        let phase = fog.phase.x * pow(fog.phase.y - fog.phase.z * cos_theta, -1.5);
        let sun_up = max(to_sun.y, 1e-4);
        let sigma_sun = sigma_t * fog.sun.w;
        // Sunlight is dimmed along its way down from the surface: the
        // exponent changes along the ray at rate k, from s0 at the start.
        let k = -sigma_t + v.y * sigma_sun / sun_up;
        let start_height = camera.y + v.y * start - water_level;
        let s0 = start_height * sigma_sun / sun_up;
        let e = exp(k * t + s0);
        let integral = select((e - exp(s0)) / k, t * e, k == vec3<f32>(0.0));
        let light = fog.light.rgb * fog.scattering.w;
        let in_scattered = max(integral * phase * sigma_s * light, vec3<f32>(0.0));
        let transmittance = exp(-sigma_t * t);
        let emitted = max(fog.emissive.rgb * (1.0 - transmittance) / sigma_t, vec3<f32>(0.0));
        result = base * transmittance + in_scattered + emitted;
    } else if !skip {
        result = base;
    }
    return vec4<f32>(result, colour.a);
}
