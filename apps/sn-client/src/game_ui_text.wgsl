// TextMesh Pro's `TextMeshPro/Distance Field` (M9u5d; `docs/formats/ui.md`
// § UI shaders and materials), ported from its compiled D3D11 programs
// (vertex 0–7, fragment 8–15; the keywords `UNDERLAY_ON` and
// `UNITY_UI_CLIP_RECT` are flags here; `UNITY_UI_ALPHACLIP` is in no
// in-scope material):
// - vertex: the SDF scale from the pixel size (the projection and the
//   screen, `_ScaleX/Y`), `|uv2.y|` (the character's scale × the text's
//   world scale), `_GradientScale`, `_Sharpness`, and `_PerspectiveFilter`
//   against the view direction and the normal; the weight (normal or
//   bold by the sign of `uv2.y`) with `_FaceDilate` and `_ScaleRatioA`;
//   the outline's alpha clip and the bias; the underlay's UV, scale and
//   bias (`_ScaleRatioC`, the atlas size); the face/outline UV unpacked
//   from `uv2.x`; the clip mask as `UI/Default`'s;
// - fragment: the face (vertex colour × `_FaceColor` × `_FaceTex`) and the
//   outline (`_OutlineColor` × `_OutlineTex`), both premultiplied, mixed
//   by the outline width; the softened edge from the distance; the
//   underlay behind; the soft clip; × the vertex alpha. Premultiplied
//   output (`One OneMinusSrcAlpha`).
// The face and outline textures are white (no in-scope material sets
// them; the client logs any that does).

struct TextItem {
    clip_from_canvas: mat4x4<f32>,
    world_from_canvas: mat4x4<f32>,
    // `_FaceColor`, `_OutlineColor`, `_UnderlayColor` (linear).
    face_color: vec4<f32>,
    outline_color: vec4<f32>,
    underlay_color: vec4<f32>,
    // `_FaceDilate`, `_OutlineWidth`, `_OutlineSoftness`, `_ScaleRatioA`.
    face: vec4<f32>,
    // `_WeightNormal`, `_WeightBold`, `_GradientScale`, `_Sharpness`.
    weight: vec4<f32>,
    // `_ScaleX`, `_ScaleY`, `_PerspectiveFilter`, the root canvas's
    // world scale (y) that `uv2.y` lacks.
    scale: vec4<f32>,
    // `_UnderlayOffsetX`, `_UnderlayOffsetY`, `_UnderlayDilate`,
    // `_UnderlaySoftness`.
    underlay: vec4<f32>,
    // `_ScaleRatioC`, `_TextureWidth`, `_TextureHeight`, flags (1 the
    // clip rect, 2 the underlay).
    atlas: vec4<f32>,
    // `_VertexOffsetX`, `_VertexOffsetY`, `_MaskSoftnessX`, `_MaskSoftnessY`.
    offset: vec4<f32>,
    // `_ClipRect` in the root canvas's space.
    clip_rect: vec4<f32>,
    // The text's normal in the world (normalized), unused.
    normal: vec4<f32>,
    // `_ScreenParams.xy`, the projection's [0][0] and [1][1].
    pixel: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> items: array<TextItem>;
@group(1) @binding(0) var atlas: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct VertexIn {
    @builtin(instance_index) instance: u32,
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv1: vec2<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    // Alpha clip, scale, bias, weight.
    @location(2) param: vec4<f32>,
    @location(3) mask: vec4<f32>,
    // Underlay UV, scale, bias.
    @location(4) underlay: vec4<f32>,
    @location(5) underlay_color: vec4<f32>,
    @location(6) @interpolate(flat) instance: u32,
}

fn has(flags: f32, bit: u32) -> bool {
    return (u32(flags) & bit) != 0u;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    let it = items[in.instance];
    var out: VertexOut;
    let vert = vec3<f32>(in.position.xy + it.offset.xy, in.position.z);
    let world = it.world_from_canvas * vec4<f32>(vert, 1.0);
    let clip = it.clip_from_canvas * vec4<f32>(vert, 1.0);
    out.clip = clip;
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
    out.uv = in.uv;

    // The pixel size and the SDF scale.
    let screen = abs(vec2<f32>(it.pixel.z * it.pixel.x, it.pixel.w * it.pixel.y)) * it.scale.xy;
    let pixel_size = clip.w / screen;
    var scale = inverseSqrt(dot(pixel_size, pixel_size));
    scale = scale * abs(in.uv1.y) * it.scale.w * it.weight.z * (it.weight.w + 1.0);
    // The UI camera is a perspective one: the perspective filter applies.
    // The view direction is from the vertex to the camera (at the origin).
    let view = normalize(-world.xyz);
    let n = it.normal.xyz;
    let filtered = abs(scale) * (1.0 - it.scale.z);
    scale = mix(filtered, scale, abs(dot(n, view)));

    let bold = select(0.0, 1.0, in.uv1.y <= 0.0);
    let ratio_a = it.face.w;
    let weight = (mix(it.weight.x, it.weight.y, bold) / 4.0 + it.face.x) * ratio_a * 0.5;
    let bias = (0.5 - weight) + 0.5 / scale;
    let alpha_clip = (1.0 - it.face.y * ratio_a - it.face.z * ratio_a) / 2.0 - 0.5 / scale - weight;
    out.param = vec4<f32>(alpha_clip, scale, bias, weight);

    let r = clamp(it.clip_rect, vec4<f32>(-2e10), vec4<f32>(2e10));
    out.mask = vec4<f32>(
        vert.xy * 2.0 - r.xy - r.zw,
        0.25 / (0.25 * it.offset.zw + pixel_size),
    );

    // The underlay.
    let ratio_c = it.atlas.x;
    let softness = it.underlay.w * ratio_c;
    let dilate = it.underlay.z * ratio_c;
    let layer_scale = scale / (1.0 + softness * scale);
    let layer_bias = (0.5 - weight) * layer_scale - 0.5 - dilate * layer_scale * 0.5;
    let shift = -vec2<f32>(it.underlay.x, it.underlay.y) * ratio_c * it.weight.z
        / vec2<f32>(it.atlas.y, it.atlas.z);
    out.underlay = vec4<f32>(in.uv + shift, layer_scale, layer_bias);
    out.underlay_color = vec4<f32>(it.underlay_color.rgb * it.underlay_color.a, it.underlay_color.a);
    out.instance = in.instance;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let it = items[in.instance];
    let scale = in.param.y;
    let ratio_a = it.face.w;
    // Face and outline, premultiplied (their textures white).
    var face = vec4<f32>(in.color.rgb * it.face_color.rgb, it.face_color.a);
    face = vec4<f32>(face.rgb * face.a, face.a);
    var outline = it.outline_color;
    outline = vec4<f32>(outline.rgb * outline.a, outline.a);
    let d = in.param.z - textureSample(atlas, atlas_sampler, in.uv).r;
    let width = it.face.y * ratio_a * scale;
    let mixed = sqrt(min(1.0, width)) * saturate(d * scale + width * 0.5);
    var c = mix(face, outline, mixed);
    let softness = it.face.z * ratio_a * scale;
    c = c * (1.0 - saturate((d * scale - width * 0.5 + softness * 0.5) / (1.0 + softness)));
    if has(it.atlas.w, 2u) {
        let u = textureSample(atlas, atlas_sampler, in.underlay.xy).r;
        let a = saturate(u * in.underlay.z - in.underlay.w);
        c = c + in.underlay_color * a * (1.0 - c.a);
    }
    if has(it.atlas.w, 1u) {
        let m = saturate((it.clip_rect.zw - it.clip_rect.xy - abs(in.mask.xy)) * in.mask.zw);
        c = c * m.x * m.y;
    }
    return c * in.color.a;
}
