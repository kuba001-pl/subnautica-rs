// The game's sky (uSky's skybox and sky map shaders, docs/formats/sky.md):
// scattering, sun disc, the planet and its corona, night sky, moon, clouds.
// Written for subnautica-rs from the behaviour of the compiled shaders.
// Directions are in Unity coordinates (Bevy's with z flipped).

#define_import_path sn_client::sky_common

// Must match `SkyDomeUniform` in sky_dome.rs.
struct SkyDome {
    // towards the sun (light path), sun size (32 / SunSize)
    sun_dir: vec4<f32>,
    // colour correction (x, y), light unit, planet texture lod scale
    correction: vec4<f32>,
    beta_r: vec4<f32>,
    beta_m: vec4<f32>,
    mie_phase: vec4<f32>,
    mie_const: vec4<f32>,
    night_zenith: vec4<f32>,
    // sunset, day (with exposure), night
    sky_multiplier: vec4<f32>,
    ground: vec4<f32>,
    night_horizon: vec4<f32>,
    moon_inner: vec4<f32>,
    moon_outer: vec4<f32>,
    // rows of the world-to-moon matrix; moon_x.w: moon size
    moon_x: vec4<f32>,
    moon_y: vec4<f32>,
    moon_z: vec4<f32>,
    // planet position, radius
    planet_pos: vec4<f32>,
    planet_rim: vec4<f32>,
    // planet ambient light, light wrap
    planet_ambient: vec4<f32>,
    planet_inner: vec4<f32>,
    planet_outer: vec4<f32>,
    // eclipse, clouds attenuation, clouds alpha saturation, unused
    misc: vec4<f32>,
    // rows of the world-to-clouds matrix
    clouds_x: vec4<f32>,
    clouds_y: vec4<f32>,
    clouds_z: vec4<f32>,
    // shade colour from the sun (linear), sun colour multiplier
    shade_sun: vec4<f32>,
    // shade colour from the sky (linear), sky colour multiplier
    shade_sky: vec4<f32>,
    // clouds scattering exponent, multiplier, secondary light power, unused
    cloud_scatter: vec4<f32>,
    secondary_dir: vec4<f32>,
    secondary_colour: vec4<f32>,
}

const PI = 3.14159265;

// The sky's colour along the unit direction `d` (game units: 1 = one game
// light unit). `skymap`: the sky map variant (upper hemisphere, no cloud
// fade at the horizon). `pixel_angle`: radians per pixel, for the planet's
// texture detail.
fn sky_colour(
    sky: SkyDome,
    d_in: vec3<f32>,
    skymap: bool,
    pixel_angle: f32,
    planet_tex: texture_2d<f32>,
    burst_tex: texture_2d<f32>,
    moon_tex: texture_2d<f32>,
    clouds_tex: texture_2d<f32>,
    repeat_s: sampler,
    clamp_s: sampler,
) -> vec3<f32> {
    var d = d_in;
    if skymap {
        d.y = max(d.y, 0.0001);
    }
    let sun = sky.sun_dir.xyz;
    let mu = dot(d, sun);
    let up = max(d.y, 0.0);
    // Optical depth through the atmosphere (the skybox's vertex stage).
    let h = vec3<f32>(max(d.y + 0.06, 0.06)) + max(-d.y, 0.0) * sky.ground.rgb;

    let eclipse = sky.misc.x;
    let ecl = 1.0 - eclipse;
    let sr = 8.0 / h * ecl;
    let sm = 1.2 / h * ecl;
    let z = sky.night_zenith.rgb;
    let zenith = sr * z * (2.0 - z * sr);
    let ext = exp(-(sky.beta_r.rgb * sr + sky.beta_m.rgb * sm));
    var ray = zenith * ext + sky.sky_multiplier.x * ((1.0 - ext) - ext * zenith);
    let mie = sm * ray / ray.x * sky.mie_const.rgb;
    let phase = sky.mie_phase.x * pow(sky.mie_phase.y - sky.mie_phase.z * mu, -1.5);
    ray = ray * 0.75 + mie * phase;
    let rayleigh = (mu * mu + 1.0) * sky.sky_multiplier.y;

    // The planet (a sphere at `planet_pos`) or its corona.
    let p = sky.planet_pos.xyz;
    let radius = sky.planet_pos.w;
    let b = dot(d, p);
    let c = dot(p, p);
    let r2 = radius * radius;
    let outside = r2 < c;
    let disc = c - b * b;
    let hit = !(b < 0.0 && outside) && r2 >= disc;
    var planet: vec3<f32>;
    if hit {
        let s = sqrt(r2 - disc);
        let t = select(b + s, b - s, outside);
        let q = d * t - p;
        let nq = normalize(q);
        let lon = atan2(q.y, q.x);
        let lat = acos(clamp(q.z / radius, -1.0, 1.0));
        let ndl = dot(nq, sun);
        let wrap = sky.planet_ambient.w;
        let lit = saturate(ndl * (1.0 - wrap) + wrap) * 2.0 + sky.planet_ambient.rgb;
        let rim = 1.0 - saturate(dot(d, -nq));
        var x = (ndl * 0.5 + 0.5 + eclipse * 0.3) * rim;
        x = x * x * x;
        // Texture detail from the planet's size on screen.
        let lod = max(log2(pixel_angle * sky.correction.w * length(p) / radius), 0.0);
        let a = textureSampleLevel(planet_tex, repeat_s, nq.xy, lod).rgb;
        let e = textureSampleLevel(planet_tex, repeat_s, vec2<f32>(lon, lat) / PI, lod).rgb;
        let w = pow(abs(nq.z) * 0.5 + 0.5, 10.0);
        let tex = mix(e, a, w) * 5.0;
        planet = tex * (x * 10.0 + lit) + x * 10.0 * sky.planet_rim.rgb;
    } else {
        let a = dot(-d, normalize(p)) + 1.0;
        planet = sky.planet_inner.rgb / (a * sky.planet_inner.w + 1.05)
            + sky.planet_outer.rgb / (a * sky.planet_outer.w + 1.05);
    }
    var col = ray * rayleigh + ext * planet;
    if hit {
        col *= ecl * 0.5 + 0.5;
    }

    // The sun disc (and its burst, seen during an eclipse).
    if sun.y > -0.1 && !hit {
        let s = min(pow(saturate(1.0 - mu) * sky.sun_dir.w, -1.5), 1000.0);
        var lit = min(mie, vec3<f32>(up)) * s * ext + col;
        if eclipse > 0.0 {
            let n = normalize(sun);
            let r = n.yzx;
            let tl = length(vec2<f32>(r.x, r.y));
            let t = vec3<f32>(-r.x, 0.0, r.y) / max(tl, 1e-6);
            let bv = normalize(vec3<f32>(
                t.z * r.y - t.x * r.x,
                t.x * r.z,
                t.y * r.x - r.z * t.z,
            ));
            let o = 2.0 * (sun - d) / pow(eclipse, 0.2);
            let uv = vec2<f32>(dot(bv, o), t.z * o.y + t.x * o.z) * 0.5 + 0.5;
            let burst = textureSampleLevel(burst_tex, clamp_s, uv, 0.0).rgb;
            lit += burst * eclipse * 10.0;
        }
        col = lit;
    }

    // Night: horizon glow, moon and its corona.
    if sun.y < 0.25 {
        col += sky.night_horizon.rgb * zenith;
        if !hit {
            let size = sky.moon_x.w;
            let muv = vec2<f32>(dot(sky.moon_x.xyz, d), dot(sky.moon_y.xyz, d)) / size + 0.5;
            let moon = textureSampleLevel(moon_tex, clamp_s, muv, 0.0).rgb;
            var lit = moon * up * sky.sky_multiplier.z + col;
            let a = dot(d, sky.moon_z.xyz) + 1.0;
            lit += sky.moon_inner.rgb / (a * sky.moon_inner.w + 1.05);
            col = sky.moon_outer.rgb / (a * sky.moon_outer.w + 1.05) + lit;
        }
    }

    // The game's tone curve for an LDR sky in linear space.
    let sky_lin = pow((1.0 - exp(-col)) * sky.correction.x, vec3<f32>(sky.correction.y));

    // Clouds: a panorama around the zenith axis, rotating slowly.
    let cd = vec3<f32>(dot(sky.clouds_x.xyz, d), dot(sky.clouds_y.xyz, d), dot(sky.clouds_z.xyz, d));
    let elevation = asin(clamp(cd.y, -1.0, 1.0));
    let azimuth = atan2(cd.x, cd.z);
    let cuv = vec2<f32>(azimuth / (2.0 * PI), elevation / (0.5 * PI));
    let cloud = textureSampleLevel(clouds_tex, repeat_s, cuv, 0.0).x;
    var density = 0.0;
    for (var i = 0; i < 6; i++) {
        density += textureSampleLevel(clouds_tex, repeat_s, cuv + vec2<f32>(0.0, 0.004 * f32(i)), 0.0).x;
    }
    let transmission = exp2(-density * sky.misc.y);
    let mus = saturate(mu);
    let light = (pow(mus, sky.cloud_scatter.x) * sky.cloud_scatter.y + 1.0) * transmission;
    var cloud_col = mix(
        sky.shade_sky.rgb * sky.shade_sky.w,
        sky.shade_sun.rgb * sky.shade_sun.w,
        light,
    );
    cloud_col = mix(cloud_col, vec3<f32>(pow(mus, 50.0) * 0.25), eclipse);
    let sec = saturate(dot(d, sky.secondary_dir.xyz)) * sky.secondary_colour.rgb;
    cloud_col += pow(sec, vec3<f32>(max(sky.cloud_scatter.z, 0.1))) * 2.0;
    var alpha = pow(cloud, sky.misc.z);
    if !skymap {
        let f = saturate((d.y + 0.1) * 10.0);
        alpha *= f * f * (3.0 - 2.0 * f);
    }
    return mix(sky_lin, cloud_col, alpha);
}
