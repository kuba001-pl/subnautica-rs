// The game's lighting of opaque surfaces (its deferred directional light
// pass, `Hidden/Internal-DeferredShadingCustom`, docs/formats/lighting.md):
// sun with caustics and underwater attenuation (and colour cast), top/bottom
// ambient, the water's emission as ambient light, Blinn specular. Written
// for subnautica-rs from the behaviour of the compiled shader.
//
// The per-frame values come from a small float texture (`params`, written
// on the GPU every frame by game_light.rs), so the many materials using it
// never need updating. Water settings are the ones at the camera (the game
// reads them per pixel from its volume around the camera).

#define_import_path sn_client::game_light

#import bevy_pbr::mesh_view_bindings::{view, lights, globals}
#import bevy_pbr::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
#import bevy_pbr::shadows::fetch_directional_shadow

// Texels of `params`; must match game_light.rs.
const P_LIGHT_DIR = 0;    // direction the light travels (Bevy world)
const P_LIGHT = 1;        // light colour × intensity (our units), light unit
const P_TOP = 2;          // top ambient (our units), ambient scale
const P_BOTTOM = 3;       // bottom ambient (our units), light scale
const P_EXTINCTION = 4;   // σt × extinction scale, sun attenuation
const P_EMISSIVE = 5;     // water emissive (our units), emission ambient scale
const P_MISC = 6;         // colour cast (distance, depth), fog enabled, water level
const P_CAUSTICS = 7;     // scale, amount.x, amount.y, frame
const P_LIGHT_X = 8;      // world → light matrix rows (Bevy world → light uv)
const P_LIGHT_Y = 9;
const P_UNITY_AMBIENT = 10; // Unity's flat ambient (our units), unused

fn param(params: texture_2d<f32>, i: i32) -> vec4<f32> {
    return textureLoad(params, vec2<i32>(i, 0), 0);
}

// What the game's G-buffer holds for a surface.
struct GameSurface {
    world: vec3<f32>,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    specular: vec3<f32>,
    gloss: f32,
}

// The lit colour (without emission) of a surface, for the first directional
// light (the sun; its shadows if enabled).
fn game_lighting(
    s: GameSurface,
    frag_coord: vec4<f32>,
    params: texture_2d<f32>,
    caustics: texture_2d_array<f32>,
    caustics_sampler: sampler,
) -> vec3<f32> {
    let light = param(params, P_LIGHT);
    let top = param(params, P_TOP);
    let bottom = param(params, P_BOTTOM);
    let ext = param(params, P_EXTINCTION);
    let emissive = param(params, P_EMISSIVE);
    let misc = param(params, P_MISC);
    let caustic = param(params, P_CAUSTICS);
    let light_dir = param(params, P_LIGHT_DIR).xyz;
    let fog_on = misc.z != 0.0;
    let height = s.world.y - misc.w;
    let underwater = height < 0.0 && fog_on;

    // Caustics: the sun's cookie, under water only.
    var cookie = vec3<f32>(1.0);
    if underwater {
        let uv = vec2<f32>(
            dot(param(params, P_LIGHT_X), vec4<f32>(s.world, 1.0)),
            dot(param(params, P_LIGHT_Y), vec4<f32>(s.world, 1.0)),
        ) * caustic.x;
        // The game samples with a mip bias of −8: the sharpest level.
        let c = textureSampleLevel(caustics, caustics_sampler, uv, i32(caustic.w), 0.0).rgb;
        cookie = max(c * caustic.z + (1.0 - caustic.y), vec3<f32>(0.0));
    }
    let light_scale = bottom.w;
    var direct = vec3<f32>(light_scale);
    var sun = cookie * light_scale;
    var ambient_extra = vec3<f32>(0.0);
    if fog_on {
        // Sunlight below the surface: dimmed along its `path` from the
        // surface, with a colour cast: near the camera and the surface the
        // attenuation is grey (its weakest channel), further away coloured.
        let to_camera = length(s.world - view.world_position);
        let path = select(0.0, height / light_dir.y, height <= 0.0 && -light_dir.y >= 0.0);
        let sigma = ext.xyz * ext.w;
        let grey = min(min(sigma.x, sigma.y), sigma.z);
        let cast_factor = exp(-to_camera * misc.x - path * misc.y);
        let att = exp(-(sigma + cast_factor * (grey - sigma)) * path);
        if path > 0.0 {
            direct = att * light_scale;
            sun = att * sun;
        }
        // The water's own glow as ambient light.
        ambient_extra = emissive.xyz * emissive.w / (ext.xyz + 0.0001);
    }

    // Shadows from Bevy's shadow map, if the sun casts them.
    var shadow = 1.0;
    if lights.n_directional_lights > 0u
        && (lights.directional_lights[0].flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
        let view_z = (view.view_from_world * vec4<f32>(s.world, 1.0)).z;
        shadow = fetch_directional_shadow(0u, vec4<f32>(s.world, 1.0), s.normal, view_z, frag_coord.xy);
    }
    sun *= shadow;

    let n = normalize(s.normal);
    let v = normalize(view.world_position - s.world);
    let h = normalize(v - light_dir);
    let power = max(s.gloss * 128.0, 0.1);
    let spec = pow(max(dot(h, n), 0.0), power) * saturate(sun.x);
    let ndl = max(dot(-light_dir, n), 0.0);
    let diffuse = sun * ndl * light.xyz;
    let lum = dot(light.xyz, vec3<f32>(0.039682, 0.458022, 0.006097));
    let spec_amount = clamp(spec * lum, 0.0, 100000.0);
    let hemi = n.y * 0.5 + 0.5;
    let ambient = mix(bottom.xyz, top.xyz, hemi) * top.w * direct + ambient_extra;
    // Unity's own ambient, added by the G-buffer pass (flat colour, the same
    // everywhere, also under water).
    let unity_ambient = param(params, P_UNITY_AMBIENT).xyz;
    return s.albedo * (ambient + diffuse + unity_ambient) + s.specular * diffuse * spec_amount;
}

// The caustics frame of the game's clock (25 frames per second).
fn caustics_frame(params: texture_2d<f32>) -> i32 {
    return i32(param(params, P_CAUSTICS).w);
}
