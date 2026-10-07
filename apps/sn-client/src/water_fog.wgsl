// Underwater fog: a full-screen pass over the lit (HDR) image, following the
// game's water fog model (docs/formats/water.md § The underwater fog; the
// model itself is in water_common.wgsl). Written for subnautica-rs.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_render::view::View
#import sn_client::water_common::{WaterFog, apply_water_fog}

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
    return vec4<f32>(apply_water_fog(fog, colour.rgb, camera, v, dist, sky), colour.a);
}
