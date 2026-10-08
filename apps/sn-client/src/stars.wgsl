// Stars, following uSky's star field (`StarField`, shader `Hidden/uSky/Stars`;
// docs/formats/sky.md § Stars): one camera-centred billboard per star of the
// game's catalogue, twinkling, hidden behind the planet, the moon and the
// clouds; added to the image after the fog (the game draws them in the
// transparent queue). Written for subnautica-rs from the behaviour of the
// compiled shader. Star directions are in Unity coordinates.

#import bevy_render::view::View
#import sn_client::sky_common::SkyDome

struct Star {
    // position on the 990-unit sphere (Unity), luminance
    position: vec4<f32>,
    colour: vec4<f32>,
}

// Must match `StarsUniform` in sky_dome.rs.
struct Stars {
    // star brightness (intensity × night), time / 20, smoothed Δt, quad size
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> sky: SkyDome;
@group(0) @binding(1) var<uniform> view: View;
@group(0) @binding(2) var<uniform> stars: Stars;
@group(0) @binding(3) var<storage, read> catalogue: array<Star>;
@group(0) @binding(4) var moon_tex: texture_2d<f32>;
@group(0) @binding(5) var clouds_tex: texture_2d<f32>;
@group(0) @binding(6) var repeat_s: sampler;
@group(0) @binding(7) var clamp_s: sampler;
#ifdef MULTISAMPLED
@group(0) @binding(8) var depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(8) var depth: texture_depth_2d;
#endif

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) colour: vec4<f32>,
    @location(1) glow: vec2<f32>,
    @location(2) dir: vec3<f32>,
}

// The game's twinkle table.
const TWINKLE = array<vec2<f32>, 8>(
    vec2<f32>(0.897908, -0.347609), vec2<f32>(0.550299, 0.273587),
    vec2<f32>(0.823886, 0.098853), vec2<f32>(0.922739, -0.122109),
    vec2<f32>(0.800630, -0.088957), vec2<f32>(0.711673, 0.158864),
    vec2<f32>(0.870538, 0.085485), vec2<f32>(0.956022, -0.058115),
);

// The quad's corners and texture coordinates (the game's `createQuad`,
// triangles 0 2 1, 2 3 1).
const CORNERS = array<vec2<f32>, 4>(
    vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, -1.0),
);
const UVS = array<vec2<f32>, 4>(
    vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0),
);
const ORDER = array<u32, 6>(0u, 2u, 1u, 2u, 3u, 1u);

@vertex
fn vertex(@builtin(vertex_index) index: u32, @builtin(instance_index) instance: u32) -> VertexOutput {
    var out: VertexOutput;
    let star = catalogue[instance];
    var order = ORDER;
    var corners = CORNERS;
    var uvs = UVS;
    let k = order[index];
    // The billboard faces the origin (`BillboardMatrix`).
    let pos = star.position.xyz;
    let forward = normalize(pos);
    let right = normalize(cross(forward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(right, forward);
    let corner = corners[k] * stars.params.w;
    let local = pos + right * corner.x + up * corner.y;
    // Camera-centred, Unity → Bevy.
    let cam = view.world_position;
    let world_unity = local + vec3<f32>(cam.x, cam.y, -cam.z);
    out.position = view.clip_from_world * vec4<f32>(world_unity.x, world_unity.y, -world_unity.z, 1.0);

    // Twinkle, seeded from the vertex position (as the game).
    let seed = fract(local.xy * 256.0);
    let t = fract(fract((seed.y + 1.0) * (stars.params.y * 2.0 + stars.params.z) + seed.x)) * 8.0;
    let i = u32(t);
    var twinkle = TWINKLE;
    let tw = twinkle[i].x + fract(t) * 2.5 * twinkle[i].y;
    let brightness = exp2((3.94 * star.colour.w - 7.94) * 0.928771);
    let b = tw * brightness;
    // Only stars above the horizon (the game: saturate(world y)).
    let horizon = saturate(world_unity.y);
    out.colour = vec4<f32>(star.colour.rgb * b, b) * stars.params.x * horizon;
    out.glow = (uvs[k] - 0.5) * 5.0;
    out.dir = local;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Behind something in the scene (the game: depth test).
    let d = textureLoad(depth, vec2<i32>(in.position.xy), 0);
    if d > in.position.z {
        discard;
    }
    let dir = normalize(in.dir);
    // Behind the planet.
    let p = sky.planet_pos.xyz;
    let b = dot(dir, p);
    let c = dot(p, p);
    let r2 = sky.planet_pos.w * sky.planet_pos.w;
    let hit = !(b < 0.0 && r2 < c) && r2 >= c - b * b;
    if hit {
        discard;
    }
    // Behind the moon disc.
    let muv = vec2<f32>(dot(sky.moon_x.xyz, dir), dot(sky.moon_y.xyz, dir)) / sky.moon_x.w + 0.5;
    if textureSampleLevel(moon_tex, clamp_s, muv, 0.0).a != 0.0 {
        discard;
    }
    // Clouds in front.
    let cd = vec3<f32>(dot(sky.clouds_x.xyz, dir), dot(sky.clouds_y.xyz, dir), dot(sky.clouds_z.xyz, dir));
    let cuv = vec2<f32>(atan2(cd.x, cd.z) / 6.28318530718, asin(clamp(cd.y, -1.0, 1.0)) / 1.5707963268);
    let cloud = textureSampleLevel(clouds_tex, repeat_s, cuv, 0.0).x;
    let f0 = saturate((dir.y + 0.1) * 10.0);
    let fade = f0 * f0 * (3.0 - 2.0 * f0) * cloud;
    let r = dot(in.glow, in.glow);
    let glow = in.colour.rgb * exp(-r) + vec3<f32>(exp(-10.0 * r) * in.colour.a * 5.0);
    return vec4<f32>(glow * (1.0 - saturate(fade * 5.0)) * sky.correction.z, 0.0);
}
