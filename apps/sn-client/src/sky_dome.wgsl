// The sky dome (where the scene shows nothing) and the sky map the water
// reflects, both from the game's sky function (sky_common.wgsl). Written
// for subnautica-rs.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_render::view::View
#import sn_client::sky_common::{SkyDome, sky_colour}

@group(0) @binding(0) var<uniform> sky: SkyDome;
@group(0) @binding(1) var planet_tex: texture_2d<f32>;
@group(0) @binding(2) var burst_tex: texture_2d<f32>;
@group(0) @binding(3) var moon_tex: texture_2d<f32>;
@group(0) @binding(4) var clouds_tex: texture_2d<f32>;
@group(0) @binding(5) var repeat_s: sampler;
@group(0) @binding(6) var clamp_s: sampler;
@group(0) @binding(7) var<uniform> view: View;
#ifdef MULTISAMPLED
@group(0) @binding(8) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(8) var depth: texture_depth_2d;
#endif
@group(0) @binding(9) var source: texture_2d<f32>;

// The sky behind everything: only where the depth buffer is empty
// (reverse-Z: 0), as the game draws its skybox after opaque geometry.
@fragment
fn dome(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let d = textureLoad(depth, vec2<i32>(in.position.xy), 0);
    if d > 0.0 {
        discard;
    }
    let ndc = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, 1e-4, 1.0);
    let p = view.world_from_clip * ndc;
    let dir = normalize(p.xyz / p.w - view.world_position);
    let pixel_angle = 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let c = sky_colour(
        sky, vec3<f32>(dir.x, dir.y, -dir.z), false, pixel_angle,
        planet_tex, burst_tex, moon_tex, clouds_tex, repeat_s, clamp_s,
    );
    return vec4<f32>(c * sky.correction.z, 1.0);
}

// The sky map: the upper hemisphere on a disc, `p = (uv − 0.5) × 2.2` →
// direction `(2p.x, 1 − |p|², 2p.y) / (1 + |p|²)` (the game's mapping). In
// game units (stored 0…1, like the game's 8-bit map).
@fragment
fn skymap(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let q = (in.uv - 0.5) * 2.2;
    let l = dot(q, q);
    let dir = vec3<f32>(2.0 * q.x, 1.0 - l, 2.0 * q.y) / (l + 1.0);
    let c = sky_colour(
        sky, dir, true, 2.2 * 2.0 / 256.0,
        planet_tex, burst_tex, moon_tex, clouds_tex, repeat_s, clamp_s,
    );
    return vec4<f32>(saturate(c), 1.0);
}

// One mip level of the sky map from the one above.
@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return textureSampleLevel(source, clamp_s, in.uv, 0.0);
}
