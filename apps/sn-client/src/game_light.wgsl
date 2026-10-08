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

#import bevy_pbr::mesh_view_bindings::{view, lights, globals, clustered_lights}
#import bevy_pbr::mesh_view_types::{
    DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT, POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE,
}
#import bevy_pbr::clustered_forward as clustering
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
const P_OBJECTS = 11;     // _UweLocalLightScalar (0 night … 1 day)
const P_DIRECTIONAL = 12; // count of directional lights of objects (≤ 8)
// then per light: 13 + 2i direction it travels, 14 + 2i colour (our units)

fn param(params: texture_2d<f32>, i: i32) -> vec4<f32> {
    return textureLoad(params, vec2<i32>(i, 0), 0);
}

// A directional light below the surface (the game's directional passes):
// dimmed along its `path` from the surface down to `world`, with a colour
// cast: near the camera and the surface the attenuation is grey (its
// weakest channel), further away coloured. 1 above water, without fog, or
// for lights pointing up.
fn underwater_path(world: vec3<f32>, light_dir: vec3<f32>, params: texture_2d<f32>) -> vec3<f32> {
    let ext = param(params, P_EXTINCTION);
    let misc = param(params, P_MISC);
    let height = world.y - misc.w;
    if misc.z == 0.0 {
        return vec3<f32>(1.0);
    }
    let path = select(0.0, height / light_dir.y, height <= 0.0 && -light_dir.y >= 0.0);
    if path <= 0.0 {
        return vec3<f32>(1.0);
    }
    let to_camera = length(world - view.world_position);
    let sigma = ext.xyz * ext.w;
    let grey = min(min(sigma.x, sigma.y), sigma.z);
    let cast_factor = exp(-to_camera * misc.x - path * misc.y);
    return exp(-(sigma + cast_factor * (grey - sigma)) * path);
}

// What the game's G-buffer holds for a surface.
struct GameSurface {
    world: vec3<f32>,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    specular: vec3<f32>,
    gloss: f32,
    // 1 where the G-buffer pass adds Unity's own ambient (terrain), 0 where
    // it doesn't (MarmosetUBER objects, which add their sky's instead).
    unity_ambient: f32,
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
        let att = underwater_path(s.world, light_dir, params);
        direct = att * light_scale;
        sun = att * sun;
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
    let unity_ambient = param(params, P_UNITY_AMBIENT).xyz * s.unity_ambient;
    return s.albedo * (ambient + diffuse + unity_ambient) + s.specular * diffuse * spec_amount;
}

// `_UweLocalLightScalar`: 0 at night … 1 by day.
fn local_light_scalar(params: texture_2d<f32>) -> f32 {
    return param(params, P_OBJECTS).x;
}

// The caustics frame of the game's clock (25 frames per second).
fn caustics_frame(params: texture_2d<f32>) -> i32 {
    return i32(param(params, P_CAUSTICS).w);
}

// Unity's light falloff texture `_LightTextureB0` at t = distance² / range².
// The engine makes this texture itself; its curve here is Unity's widely
// quoted built-in one (1 / (1 + 25 t), faded linearly to 0 from t = 0.64),
// a HYPOTHESIS until measured (docs/formats/lighting.md).
fn unity_light_falloff(t: f32) -> f32 {
    if t >= 1.0 {
        return 0.0;
    }
    var a = 1.0 / (1.0 + 25.0 * t);
    if t > 0.64 {
        a *= 1.0 - (t - 0.64) / 0.36;
    }
    return a;
}

// The game's point and spot lights (its deferred `POINT` / `SPOT` passes)
// on a surface: every light Bevy clustered at this pixel. Lights carry the
// game's `_LightColor` (see objects.rs, `spawn_light`). No water
// attenuation, no unlit check (as the game's passes).
fn game_local_lights(
    s: GameSurface,
    frag_coord: vec4<f32>,
    params: texture_2d<f32>,
    cookie: texture_2d<f32>,
    cookie_sampler: sampler,
) -> vec3<f32> {
    let unit = param(params, P_LIGHT).w;
    let view_z = (view.view_from_world * vec4<f32>(s.world, 1.0)).z;
    let is_orthographic = view.clip_from_view[3].w == 1.0;
    let cluster = clustering::view_fragment_cluster_index(frag_coord.xy, view_z, is_orthographic);
    let ranges = clustering::unpack_clusterable_object_index_ranges(cluster);
    let n = normalize(s.normal);
    let v = normalize(view.world_position - s.world);
    let power = max(s.gloss * 128.0, 0.1);
    var total = vec3<f32>(0.0);
    for (var i = ranges.first_point_light_index_offset;
         i < ranges.first_reflection_probe_index_offset;
         i = i + 1u) {
        let light = &clustered_lights.data[clustering::get_clusterable_object_id(i)];
        let to_light = (*light).position_radius.xyz - s.world;
        var a = unity_light_falloff(dot(to_light, to_light) * (*light).color_inverse_square_range.w);
        if i >= ranges.first_spot_light_index_offset {
            // The cone: Unity's default cookie projected over the spot
            // angle, only in front of the light. The cookie is radially
            // symmetric (checked), so the light's roll doesn't matter.
            let d = (*light).light_custom_data.xy;
            var y = sqrt(max(0.0, 1.0 - dot(d, d)));
            if ((*light).flags & POINT_LIGHT_FLAGS_SPOT_LIGHT_Y_NEGATIVE) != 0u {
                y = -y;
            }
            let forward = vec3<f32>(d.x, y, d.y);
            let along = dot(-to_light, forward);
            if along <= 0.0 {
                continue;
            }
            let across = length(-to_light - forward * along);
            let r = across / (along * (*light).spot_light_tan_angle);
            a *= textureSampleLevel(cookie, cookie_sampler, vec2<f32>(0.5 + 0.5 * r, 0.5), 0.0).a;
        }
        if a <= 0.0 {
            continue;
        }
        let colour = (*light).color_inverse_square_range.rgb * unit;
        let l = normalize(to_light);
        let h = normalize(v + l);
        let diffuse = a * max(dot(n, l), 0.0) * colour;
        let lum = dot(colour, vec3<f32>(0.039682, 0.458022, 0.006097));
        let spec = clamp(pow(max(dot(n, h), 0.0), power) * saturate(a) * lum, 0.0, 100000.0);
        total += s.albedo * diffuse + s.specular * diffuse * spec;
    }
    // Directional lights of objects (the game's plain `DIRECTIONAL` pass:
    // dimmed under water only when pointing down; no caustics, no
    // ambient, no unlit check).
    let count = i32(param(params, P_DIRECTIONAL).x);
    for (var i = 0; i < count; i = i + 1) {
        let dir = param(params, P_DIRECTIONAL + 1 + 2 * i).xyz;
        let colour = param(params, P_DIRECTIONAL + 2 + 2 * i).xyz;
        let att = underwater_path(s.world, dir, params);
        let l = -dir;
        let h = normalize(v + l);
        let diffuse = att * max(dot(n, l), 0.0) * colour;
        let lum = dot(colour, vec3<f32>(0.039682, 0.458022, 0.006097));
        let spec = clamp(pow(max(dot(n, h), 0.0), power) * min(att.x, 1.0) * lum, 0.0, 100000.0);
        total += s.albedo * diffuse + s.specular * diffuse * spec;
    }
    return total;
}
