// The game's `UWE/Particles/WBOIT-FakeVolumetricLight` (the glows on ion
// crystal pedestals and under Precursor lights), accumulated into the WBOIT
// targets A and B (docs/formats/materials.md § Fake volumetric lights).
// Written for subnautica-rs from the decoded compiled programs; the game's
// keywords (FX_ADDFOG FX_FRESNELCLIP FX_NEARCLIP FX_SOFTEDGES; FX_SCROLL
// and FX_REFRACT_OFF change nothing here) are compiled in.

#import bevy_render::view::View
#import sn_client::water_common::{WaterFog, apply_water_fog, water_fog_length}

// One drawn effect; must match `EffectInstance` in effects.rs.
struct Instance {
    world_from_local: mat4x4<f32>,
    local_from_world: mat4x4<f32>,
    // _Color, linear
    color: vec4<f32>,
    // _FresnelFade, _FresnelPow, _ClipOffset, _ClipFade
    fresnel: vec4<f32>,
    // _Intensity, _Offset, _Fallof, _InvFade
    falloff: vec4<f32>,
}

struct Instances {
    items: array<Instance>,
}

// Must match `EffectGlobals` in effects.rs.
struct Globals {
    // depth weighting on (1) or off, its sharpness, our light unit, unused
    wboit: vec4<f32>,
    // `_Time.y` (s), `_UweLocalLightScalar`, unused, unused (not used here)
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> fog: WaterFog;
@group(0) @binding(2) var<uniform> globals: Globals;
@group(0) @binding(3) var<storage, read> instances: Instances;
#ifdef MULTISAMPLED
@group(0) @binding(4) var scene_depth: texture_depth_multisampled_2d;
#else
@group(0) @binding(4) var scene_depth: texture_depth_2d;
#endif

struct VertexInput {
    @builtin(instance_index) instance: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    // Eye depth (m).
    @location(1) eye: f32,
    // The vertex colour's alpha.
    @location(2) alpha: f32,
    // Object space.
    @location(3) normal: vec3<f32>,
    // Object space, towards the camera, normalised per vertex.
    @location(4) to_camera: vec3<f32>,
    @location(5) @interpolate(flat) instance: u32,
}

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    let inst = instances.items[in.instance];
    let world = inst.world_from_local * vec4<f32>(in.position, 1.0);
    let camera = (inst.local_from_world * vec4<f32>(view.world_position, 1.0)).xyz;
    var out: VertexOutput;
    out.position = view.clip_from_world * world;
    out.world = world.xyz;
    out.eye = -(view.view_from_world * world).z;
    out.alpha = in.color.a;
    out.normal = in.normal;
    out.to_camera = normalize(camera - in.position);
    out.instance = in.instance;
    return out;
}

struct Targets {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
}

// Linear depth (m) of what the scene shows at `pixel`; the sky is far away.
fn scene_eye_depth(pixel: vec2<i32>) -> f32 {
    let size = vec2<i32>(view.viewport.zw);
    let d = textureLoad(scene_depth, clamp(pixel, vec2<i32>(0), size - 1), 0);
    if d <= 0.0 {
        return 1e7;
    }
    return view.clip_from_view[3][2] / d;
}

@fragment
fn fragment(in: VertexOutput) -> Targets {
    let inst = instances.items[in.instance];
    let fresnel_fade = inst.fresnel.x;
    let fresnel_pow = inst.fresnel.y;
    let clip_offset = inst.fresnel.z;
    let clip_fade = inst.fresnel.w;
    let intensity = inst.falloff.x;
    let offset = inst.falloff.y;
    let fallof = inst.falloff.z;
    let inv_fade = inst.falloff.w;

    // The game's depth test (ZTest LEqual, no ZWrite) against the scene.
    let scene = scene_eye_depth(vec2<i32>(in.position.xy));
    if scene < in.eye {
        discard;
    }
    // Soft edges where the glow meets the scene.
    let soft = saturate((scene - in.eye) * inv_fade);
    // Strongest facing the camera, gone at the silhouette; `log`/`exp` as
    // the game computes `pow` (0 stays 0).
    let facing = abs(dot(normalize(in.normal), in.to_camera));
    let fres = exp2(log2(facing) * fresnel_pow) * fresnel_fade;
    // Fades out near the camera (`near` is our near plane, Unity's
    // `_ProjectionParams.y`); not clamped below 0, as in the game.
    let near = view.clip_from_view[3][2];
    let clip = min((in.eye - clip_offset - near) / (clip_fade + clip_offset), 1.0);
    // Falloff along the mesh, painted into the vertex alpha.
    let t = saturate((1.0 - in.alpha - offset + 0.01) / (fallof + 0.01));
    let fall = t * t * (3.0 - 2.0 * t) * in.alpha;
    // Not clamped after `_Intensity`, as in the game.
    var alpha = saturate(fall * soft * fres * clip) * inst.color.a * intensity;
    // `_Color` is in game light units.
    var colour = inst.color.rgb * intensity * globals.wboit.z;

    // Fog at the glow's own distance; the glow fades out over its path
    // through fogged water (gone between 112.5 and 125 m).
    if fog.misc.z != 0.0 {
        let camera = view.world_position;
        let towards = in.world - camera;
        let dist = length(towards);
        let v = towards / max(dist, 1e-6);
        colour = apply_water_fog(fog, colour, camera, v, dist, false);
        alpha *= saturate(10.0 - 0.08 * water_fog_length(fog, camera, v, dist));
    }

    // The WBOIT weight: nearer glows count more.
    let depth_weight = clamp(exp(-in.eye * globals.wboit.y), 0.01, 1.0);
    let w = globals.wboit.x * (alpha * depth_weight - 1.0) + 1.0;
    var out: Targets;
    out.a = vec4<f32>(w * alpha * colour, alpha);
    out.b = vec4<f32>(alpha * w, 0.0, 0.0, 0.0);
    return out;
}
