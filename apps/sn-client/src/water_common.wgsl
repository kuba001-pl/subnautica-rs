// The game's water fog model (docs/formats/water.md § The underwater fog),
// shared by the full-screen fog pass and the water surface. Written for
// subnautica-rs; works in world space with the water plane at
// y = water_level (the game works in view space; same result).

#define_import_path sn_client::water_common

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

// Henyey–Greenstein phase for a ray going along `v`.
fn fog_phase(fog: WaterFog, v: vec3<f32>) -> f32 {
    let cos_theta = dot(v, -fog.sun.xyz);
    return fog.phase.x * pow(fog.phase.y - fog.phase.z * cos_theta, -1.5);
}

// Light scattered towards the viewer over `t` metres of water along `v`,
// starting at height `start_height` relative to the water plane.
fn fog_in_scattering(fog: WaterFog, v: vec3<f32>, t: f32, start_height: f32) -> vec3<f32> {
    let sigma_t = max(fog.extinction.rgb, vec3<f32>(1e-6));
    let to_sun = fog.sun.xyz;
    let sun_up = max(to_sun.y, 1e-4);
    let sigma_sun = sigma_t * fog.sun.w;
    // Sunlight is dimmed along its way down from the surface: the exponent
    // changes along the ray at rate k, from s0 at the start.
    let k = -sigma_t + v.y * sigma_sun / sun_up;
    let s0 = start_height * sigma_sun / sun_up;
    let e = exp(k * t + s0);
    let integral = select((e - exp(s0)) / k, t * e, k == vec3<f32>(0.0));
    let light = fog.light.rgb * fog.scattering.w;
    return max(integral * fog_phase(fog, v) * fog.scattering.rgb * light, vec3<f32>(0.0));
}

// Light emitted by the water over `t` metres.
fn fog_emission(fog: WaterFog, t: f32) -> vec3<f32> {
    let sigma_t = max(fog.extinction.rgb, vec3<f32>(1e-6));
    return max(fog.emissive.rgb * (1.0 - exp(-sigma_t * t)) / sigma_t, vec3<f32>(0.0));
}

// Colour `c` seen at `dist` along the unit ray `v` from `camera`, through
// air fog and water fog. `sky`: the ray hits nothing (no air fog on it).
fn apply_water_fog(
    fog: WaterFog,
    c: vec3<f32>,
    camera: vec3<f32>,
    v: vec3<f32>,
    dist: f32,
    sky: bool,
) -> vec3<f32> {
    if fog.misc.z == 0.0 {
        return c;
    }
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
        let start_height = camera.y + v.y * start - water_level;
        let transmittance = exp(-sigma_t * t);
        result = base * transmittance + fog_in_scattering(fog, v, t, start_height)
            + fog_emission(fog, t);
    } else if !skip {
        result = base;
    }
    return result;
}

// The game's transmittance-only variant (used for the water surface's
// sub-surface foam): `c` dimmed by the water between the camera and `dist`.
fn water_transmittance(fog: WaterFog, c: vec3<f32>, camera: vec3<f32>, v: vec3<f32>, dist: f32) -> vec3<f32> {
    if fog.misc.z == 0.0 {
        return c;
    }
    let height = camera.y - fog.misc.y;
    var start = fog.extinction.w;
    var water_dist = dist;
    if height > 0.0 {
        if v.y >= 0.0 {
            return c;
        }
        start = height / -v.y + fog.misc.x;
    } else {
        let to_surface = -height / v.y;
        if to_surface > 0.0 {
            water_dist = min(dist, to_surface);
        }
    }
    if start < water_dist {
        return c * exp(-max(fog.extinction.rgb, vec3<f32>(1e-6)) * (water_dist - start));
    }
    return c;
}
