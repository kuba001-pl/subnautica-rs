// The ocean surface, following the game's water surface shader
// (docs/formats/water.md § Water surface), drawn after the fog pass over
// the fogged image as the game draws it after its fog image effect. Written
// for subnautica-rs from the behaviour of the compiled shader.
//
// Coordinates: Bevy world space (Unity's with z flipped). The wave textures
// are laid out in Unity's x/z, so texture-space vectors are flipped on z.

#import bevy_render::view::View
#import sn_client::water_common::{
    WaterFog, apply_water_fog, fog_emission, fog_in_scattering
}

// Must match `WaterSurfaceUniform` in water_surface.rs.
struct WaterSurface {
    // grid centre x, z (snapped), water level, patch length (cm)
    grid: vec4<f32>,
    // 2 × normal texel length (cm), Fresnel R0, screen-space refraction
    // ratio (1 / index), under-water refraction index
    optics: vec4<f32>,
    // reflection colour (linear), unused
    reflection: vec4<f32>,
    // refraction colour (linear), unused
    refraction: vec4<f32>,
    // back-light tint × transmission (linear), sun reflection gloss
    back_light: vec4<f32>,
    // sun colour (our units), sun reflection amount
    sun: vec4<f32>,
    // top ambient (our units), foam smoothing (normal mip bias)
    ambient: vec4<f32>,
    // mean sky colour (our units), under-water sky brightness
    mean_sky: vec4<f32>,
    // foam scale, foam distance, sub-surface foam scale, foam multiplier
    foam: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> fog: WaterFog;
@group(0) @binding(2) var<uniform> surface: WaterSurface;
@group(0) @binding(3) var displacement: texture_2d<f32>;
@group(0) @binding(4) var normals: texture_2d<f32>;
@group(0) @binding(5) var foam_amount: texture_2d<f32>;
@group(0) @binding(6) var foam_texture: texture_2d<f32>;
@group(0) @binding(7) var foam_mask: texture_2d<f32>;
@group(0) @binding(8) var scene: texture_2d<f32>;
#ifdef MULTISAMPLED
@group(0) @binding(9) var scene_depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(9) var scene_depth: texture_depth_2d;
#endif
@group(0) @binding(10) var repeat_sampler: sampler;
@group(0) @binding(11) var clamp_sampler: sampler;

// Waves fade out with horizontal distance from the camera (m).
const WAVE_FADE_END = 200.0;
const WAVE_FADE_LENGTH = 192.0;

// The grid (see `water_surface.rs`): an inner square of fine quads, an
// outer square of coarser ones around it (out to where the waves have
// faded) and a flat ring out to the horizon.
const INNER_QUADS = 256u;
const INNER_STEP = 0.25;
const OUTER_QUADS = 416u;
const OUTER_STEP = 1.0;
const FAR = 40000.0;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

// Wave texture coordinates of a Bevy world position.
fn wave_uv(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(p.x, -p.y) * 100.0 / surface.grid.w;
}

// The game's displacement of the surface point above `p` (Bevy x, z), m.
fn displacement_at(p: vec2<f32>) -> vec3<f32> {
    let d = length(p - view.world_position.xz);
    let fade = saturate((WAVE_FADE_END - d) / WAVE_FADE_LENGTH);
    if fade <= 0.0 {
        return vec3<f32>(0.0);
    }
    let cm = textureSampleLevel(displacement, repeat_sampler, wave_uv(p), 0.0).xyz;
    return vec3<f32>(cm.x, cm.y, -cm.z) * 0.01 * fade;
}

const CORNERS = array<vec2<u32>, 6>(
    vec2<u32>(0u, 0u), vec2<u32>(0u, 1u), vec2<u32>(1u, 0u),
    vec2<u32>(1u, 0u), vec2<u32>(0u, 1u), vec2<u32>(1u, 1u),
);

@vertex
fn vertex(@builtin(vertex_index) index: u32, @builtin(instance_index) part: u32) -> VertexOutput {
    var out: VertexOutput;
    let centre = surface.grid.xy;
    var corners = CORNERS;
    let corner = corners[index % 6u];
    let quad = index / 6u;
    var local: vec2<f32>;
    var offset = vec3<f32>(0.0);
    if part == 0u {
        let g = vec2<u32>(quad % INNER_QUADS, quad / INNER_QUADS) + corner;
        let half = f32(INNER_QUADS) * INNER_STEP * 0.5;
        local = vec2<f32>(g) * INNER_STEP - half;
        let p = centre + local;
        let edge = g.x == 0u || g.y == 0u || g.x == INNER_QUADS || g.y == INNER_QUADS;
        let f = fract(local / OUTER_STEP);
        if edge && (f.x != 0.0 || f.y != 0.0) {
            // On the border with the coarse grid: follow its straight edge
            // so the two meet without cracks.
            let along = select(vec2<f32>(0.0, OUTER_STEP), vec2<f32>(OUTER_STEP, 0.0), f.x != 0.0);
            let t = max(f.x, f.y);
            let a = p - along * t;
            offset = mix(displacement_at(a), displacement_at(a + along), t);
        } else {
            offset = displacement_at(p);
        }
    } else if part == 1u {
        let g = vec2<u32>(quad % OUTER_QUADS, quad / OUTER_QUADS);
        let half = f32(OUTER_QUADS) * OUTER_STEP * 0.5;
        let inner = f32(INNER_QUADS) * INNER_STEP * 0.5;
        let q = vec2<f32>(g) * OUTER_STEP - half;
        if all(q >= vec2<f32>(-inner)) && all(q < vec2<f32>(inner)) {
            // Covered by the fine grid: a degenerate triangle.
            out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
            return out;
        }
        local = vec2<f32>(g + corner) * OUTER_STEP - half;
        offset = displacement_at(centre + local);
    } else {
        // Flat ring: 3 × 3 quads without the centre.
        let cell = vec2<u32>(quad % 3u, quad / 3u);
        if cell.x == 1u && cell.y == 1u {
            out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
            return out;
        }
        let half = f32(OUTER_QUADS) * OUTER_STEP * 0.5;
        var lines = array<f32, 4>(-FAR, -half, half, FAR);
        let c = cell + corner;
        local = vec2<f32>(lines[c.x], lines[c.y]);
    }
    let p = centre + local;
    let world = vec3<f32>(p.x, surface.grid.z, p.y) + offset;
    out.world = world;
    out.uv = wave_uv(p);
    out.position = view.clip_from_world * vec4<f32>(world, 1.0);
    return out;
}

// Linear depth (m) of what the scene shows at `uv`; the sky is far away.
fn scene_eye_depth(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(view.viewport.zw);
    let pixel = clamp(vec2<i32>(uv * view.viewport.zw), vec2<i32>(0), size - 1);
    let d = textureLoad(scene_depth, pixel, 0);
    if d <= 0.0 {
        return 1e7;
    }
    return view.clip_from_view[3][2] / d;
}

// The sky seen along `dir`. The game reads its sky map here; until the sky
// dome exists (M8c2) this is the game's mean sky colour, which the game
// itself uses for reflections further than a few tens of metres.
fn sky_map(dir: vec3<f32>) -> vec3<f32> {
    return surface.mean_sky.rgb;
}

// Texture-space normal (Unity x, y, z) → Bevy.
fn to_bevy(n: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(n.x, n.y, -n.z);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Texture lookups use explicit gradients: the code below branches per
    // pixel (and may discard).
    let du = dpdx(in.uv);
    let dv = dpdy(in.uv);
    let screen_uv = in.position.xy / view.viewport.zw;
    let eye = -(view.view_from_world * vec4<f32>(in.world, 1.0)).z;
    // Our depth test against the scene (the scene's depth buffer is read,
    // not attached).
    if scene_eye_depth(screen_uv) < eye {
        discard;
    }
    let camera = view.world_position;
    let to_camera = camera - in.world;
    let dist = length(to_camera);
    let v = to_camera / dist;
    let flat_dist = length(to_camera.xz);
    let fade = saturate((WAVE_FADE_END - flat_dist) / WAVE_FADE_LENGTH);
    let texel2 = surface.optics.x;
    let nt = textureSampleGrad(normals, repeat_sampler, in.uv, du, dv).xy;
    let n = to_bevy(normalize(vec3<f32>(nt.x * fade, texel2, nt.y * fade)));
    let to_sun = fog.sun.xyz;
    let sun = surface.sun.rgb;

    var colour: vec3<f32>;
    if front {
        // Above the water.
        let r = reflect(-v, n);
        let r_up = vec3<f32>(r.x, max(r.y, 0.0), r.z);
        let far = max(1.0 - 1.0 / (0.03 * flat_dist), 0.0);
        let sky = mix(sky_map(r_up), surface.mean_sky.rgb, far);

        // Refraction: the fogged scene, offset by the refracted ray's
        // direction in view space (less near the top of the screen).
        let t = refract(-v, n, surface.optics.z);
        let tv = (view.view_from_world * vec4<f32>(t, 0.0)).xy;
        let shifted = screen_uv + tv / eye * vec2<f32>(-0.1, 0.2);
        let top = saturate(1.0 - 4.0 * screen_uv.y);
        var y = mix(shifted.y, screen_uv.y, top);
        y = 1.0 - abs(abs(y) - 1.0);
        let refracted_uv = vec2<f32>(shifted.x, y);
        let refracted = textureSampleLevel(scene, clamp_sampler, refracted_uv, 0.0).rgb;
        let straight = textureSampleLevel(scene, clamp_sampler, screen_uv, 0.0).rgb;
        // Where the offset lands on something in front of the water, use
        // the colour straight behind instead.
        let in_front = saturate((eye - scene_eye_depth(refracted_uv)) * 10.0);
        let refraction = mix(refracted, straight, in_front) * surface.refraction.rgb;

        let r0 = surface.optics.y;
        let fresnel = r0 + (1.0 - r0) * pow(1.0 - saturate(dot(n, v)), 5.0);
        colour = mix(refraction, sky * surface.reflection.rgb, fresnel);

        // Light through the waves' thin crests ("back light").
        // The game's mip bias, as a gradient scale.
        let bias = exp2(surface.ambient.w);
        let blurred = textureSampleGrad(normals, repeat_sampler, in.uv, du * bias, dv * bias).xy;
        let up = texel2 / length(vec3<f32>(blurred.x * fade, texel2, blurred.y * fade));
        let slope = pow(1.0 - up, 1.2);
        var towards = dot(v, to_sun);
        towards = select(towards, towards * -0.55, towards < 0.0);
        let height = textureSampleLevel(displacement, repeat_sampler, in.uv, 0.0).y;
        let back = saturate(slope * towards * towards * towards) * 15.0
            + max(height * 0.0001, 0.0) * 15.0;
        colour += back * surface.back_light.rgb * sun * surface.refraction.rgb;

        // Sun glint.
        let glint = pow(saturate(dot(r_up, to_sun)), surface.back_light.w);
        colour += glint * sun * surface.sun.w;
    } else {
        // Below the water, looking up.
        let c = dot(n, v);
        let eta = surface.optics.w;
        let k = 1.0 - eta * eta * (1.0 - c * c);
        if k < 0.0 {
            // Total internal reflection: the deep water's own colour along
            // the reflected ray (1 km of in-scattering and emission).
            let r = reflect(-v, n);
            let down = vec3<f32>(r.x, -abs(r.y), r.z);
            colour = fog_in_scattering(fog, down, 1000.0, 0.0) + fog_emission(fog, 1000.0);
            colour = apply_water_fog(fog, colour, camera, -v, dist, false);
        } else {
            let t = refract(-v, -n, eta);
            let tv = (view.view_from_world * vec4<f32>(t, 0.0)).xy;
            let uv = screen_uv + tv / eye * vec2<f32>(-0.1, 0.2);
            let behind = scene_eye_depth(uv);
            if behind >= 1000.0 || behind < eye {
                colour = apply_water_fog(fog, sky_map(t), camera, -v, dist, false);
            } else {
                colour = textureSampleLevel(scene, clamp_sampler, uv, 0.0).rgb;
            }
            colour *= surface.mean_sky.w;
        }
    }

    // Foam where the waves squeeze together, fading with distance. (The
    // game's shore foam and sub-surface foam need its clip map, which only
    // bases and wrecks draw into: none of them here, so none of it.)
    let amount = exp(-0.02 * flat_dist)
        * saturate(textureSampleGrad(foam_amount, repeat_sampler, in.uv, du, dv).x) * surface.foam.w;
    let fs = surface.foam.x;
    let foam_tex = textureSampleGrad(foam_texture, repeat_sampler, in.uv * fs, du * fs, dv * fs);
    let ms = fs * 2.0;
    let mask = textureSampleGrad(foam_mask, repeat_sampler, in.uv * ms, du * ms, dv * ms).x;
    let lo = max(mask - 0.025, 0.0);
    let hi = min(mask + 0.025, 1.0);
    var q = saturate((amount - lo) / (hi - lo));
    q = q * q * (3.0 - 2.0 * q);
    let foam_alpha = foam_tex.x * q;
    let foam_lit = foam_tex.rgb * 2.0 * (sun + surface.ambient.rgb);
    let foam_fogged = apply_water_fog(fog, foam_lit, camera, -v, dist, false);
    colour = mix(colour, foam_fogged, foam_alpha * 0.5);

    if front {
        colour = mix(fog.sky.rgb, colour, exp(-dist * fog.sky.w));
    }
    return vec4<f32>(colour, 1.0);
}
