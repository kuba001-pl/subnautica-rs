// The game's `Hidden/WBOIT Composite` without keywords (docs/formats/
// materials.md § Fake volumetric lights): the WBOIT targets A and B over the
// image so far. Written for subnautica-rs from the decoded compiled program.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var target_a: texture_2d<f32>;
@group(0) @binding(2) var target_b: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // B.yz moves the lookup (refraction; 0 for the glows).
    let uv = in.uv + textureSample(target_b, linear_sampler, in.uv).yz;
    let weight = clamp(textureSample(target_b, linear_sampler, uv).x, 1e-4, 5e4);
    let a = textureSample(target_a, linear_sampler, uv);
    let scene = textureSample(screen, linear_sampler, uv);
    // A.a is the product of the effects' (1 − α): how much scene is left.
    let keep = 1.0 - a.a;
    let out = keep * vec4<f32>(a.rgb / weight, keep) + a.a * scene;
    return max(out, vec4<f32>(0.0));
}
