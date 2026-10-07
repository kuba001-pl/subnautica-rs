// The water surface's per-frame textures, as the game makes them
// (docs/formats/water.md § Water surface): the displacement map from two
// baked frames, its normal map (plus mips) and the accumulated foam amount.
// Written for subnautica-rs from the behaviour of the game's shaders.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

// Must match `WaterSim` in water_surface.rs.
struct WaterSim {
    // frame a, frame b, fraction, unused
    frames: vec4<f32>,
    // patch length (cm), displacement texel (uv), foam decay, foam rate
    foam: vec4<f32>,
}

@group(0) @binding(0) var frames: texture_2d_array<f32>;
@group(0) @binding(1) var frames_sampler: sampler;
@group(0) @binding(2) var<uniform> sim: WaterSim;
@group(0) @binding(3) var source: texture_2d<f32>;
@group(0) @binding(4) var source_sampler: sampler;

// The game's largest displacement per axis (cm); frames store
// (value + max) / (2 max), y with 16 bits (G + A / 255).
const MAX_DISPLACEMENT = vec3<f32>(100.0, 300.0, 100.0);

fn decode(p: vec4<f32>) -> vec3<f32> {
    let v = vec3<f32>(p.x, p.y + p.w * (1.0 / 255.0), p.z);
    return v * (2.0 * MAX_DISPLACEMENT) - MAX_DISPLACEMENT;
}

// Displacement (cm): two frames blended linearly (the scene does not use
// the cubic variant).
@fragment
fn interpolate(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let a = decode(textureSampleLevel(frames, frames_sampler, in.uv, i32(sim.frames.x), 0.0));
    let b = decode(textureSampleLevel(frames, frames_sampler, in.uv, i32(sim.frames.y), 0.0));
    return vec4<f32>(mix(a, b, sim.frames.z), 0.0);
}

// Height differences across two texels, per texture axis (cm). The game
// samples at texel centres with offsets ±1 on a repeating texture.
@fragment
fn normals(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(source));
    let p = vec2<i32>(in.position.xy);
    let left = textureLoad(source, (p + vec2<i32>(-1, 0) + size) % size, 0).y;
    let right = textureLoad(source, (p + vec2<i32>(1, 0)) % size, 0).y;
    let up = textureLoad(source, (p + vec2<i32>(0, -1) + size) % size, 0).y;
    let down = textureLoad(source, (p + vec2<i32>(0, 1)) % size, 0).y;
    return vec4<f32>((left - right) * 0.5, (up - down) * 0.5, 0.0, 0.0);
}

// One mip level from the one above: the average of its 2 × 2 texels.
@fragment
fn downsample(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return textureSampleLevel(source, source_sampler, in.uv, 0.0);
}

// Foam where the waves squeeze the surface together. Blended onto the foam
// amount as `new + old × decay` (the game's "Blend One SrcAlpha").
@fragment
fn foam(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let texel = sim.foam.y;
    let spacing = sim.foam.x * texel * 0.01;
    let centre = textureSampleLevel(source, source_sampler, in.uv, 0.0);
    let next_u = textureSampleLevel(source, source_sampler, in.uv + vec2<f32>(texel, 0.0), 0.0);
    let next_v = textureSampleLevel(source, source_sampler, in.uv + vec2<f32>(0.0, texel), 0.0);
    // The game compares the heights (not the z displacement) along v.
    let du = centre.x * 0.01 - (next_u.x * 0.01 + spacing);
    let dv = centre.y * 0.01 - (next_v.y * 0.01 + spacing);
    let a = min(length(abs(vec2<f32>(du, dv))) * 11.0, 1.0) - 1.0;
    let f = a * a * (2.0 * a + 3.0);
    return vec4<f32>(pow(f, 10.0) * sim.foam.w, 0.0, 0.0, sim.foam.z);
}
