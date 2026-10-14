// The game's UI shaders (M9u5c; `docs/formats/ui.md` § UI shaders and
// materials), ported from their compiled D3D11 programs:
// - `UI/Default` (no keywords, or `UNITY_UI_CLIP_RECT`): colour = vertex
//   colour × `_Color`; UV through `_MainTex_ST`; (`_MainTex` +
//   `_TextureSampleAdd`) × colour; with the keyword, alpha × the soft clip
//   factor of `RectMask2D` (`_ClipRect`, `_UIMaskSoftnessX/Y`).
// - `UI/Simple` (no keywords, or `ALPHA_PREMULTIPLY`): the same without UV
//   transform or soft clip; the hard clip `UnityGet2DClipping` always;
//   premultiplied, colour × the clipped alpha.
// - `uGUI/Compass` (M9u6c2, flag 16; its values in `st`): the UV divided by
//   `uv1.x` (the mesh stores UV / z and 1/z); p = 2 UV − 1; the texture at
//   p turned by `_Angle` about the centre; (`_MainTex` +
//   `_TextureSampleAdd`) × colour; alpha × the hard clip × sin(1 − |2t −
//   1|), t = saturate((the clockwise angle of p from +y as a fraction of a
//   turn − `_AlphaFrom`) ÷ (`_AlphaTo` − `_AlphaFrom`)). Its pass culls
//   back faces; the mesh faces the camera, so nothing is culled here.
// Vertex colours arrive as stored (sRGB bytes) and are made linear, as
// Unity does in a linear project (*hypothesis*, `docs/formats/ui.md`).

struct Item {
    // From the root canvas's space to clip space (the UI camera's).
    clip_from_canvas: mat4x4<f32>,
    // `_Color`, linear.
    color: vec4<f32>,
    // `_MainTex_ST`; `uGUI/Compass`: `_Angle`, `_AlphaFrom`, `_AlphaTo`.
    st: vec4<f32>,
    // `_TextureSampleAdd`.
    texture_add: vec4<f32>,
    // `_ClipRect`: x min, y min, x max, y max in the root canvas's space.
    clip_rect: vec4<f32>,
    // `_UIMaskSoftnessX`, `_UIMaskSoftnessY`, flags, unused. Flags: 1 the
    // soft clip (`UNITY_UI_CLIP_RECT`), 2 the hard clip (`UI/Simple`), 4
    // premultiply (`ALPHA_PREMULTIPLY`), 8 `_MainTex_ST` applies, 16
    // `uGUI/Compass`.
    softness: vec4<f32>,
    // `_ScreenParams.xy`, the projection's [0][0] and [1][1].
    pixel: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> items: array<Item>;
@group(1) @binding(0) var main_tex: texture_2d<f32>;
@group(1) @binding(1) var main_sampler: sampler;

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
    @location(2) canvas: vec2<f32>,
    @location(3) mask: vec4<f32>,
    @location(4) @interpolate(flat) instance: u32,
    // `uv1.x` (the compass's 1/z).
    @location(5) inv_z: f32,
}

const TAU: f32 = 6.283185;

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
    let clip = it.clip_from_canvas * vec4<f32>(in.position, 1.0);
    out.clip = clip;
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a) * it.color;
    if has(it.softness.z, 8u) {
        out.uv = in.uv * it.st.xy + it.st.zw;
    } else {
        out.uv = in.uv;
    }
    out.canvas = in.position.xy;
    // `UI/Default`'s mask: the offset from the clip rect's centre (× 2)
    // and the softness factor, the pixel size from the projection and the
    // screen.
    let pixel_size = clip.w / abs(vec2<f32>(it.pixel.z * it.pixel.x, it.pixel.w * it.pixel.y));
    let r = clamp(it.clip_rect, vec4<f32>(-2e10), vec4<f32>(2e10));
    out.mask = vec4<f32>(
        in.position.xy * 2.0 - r.xy - r.zw,
        0.25 / (0.25 * it.softness.xy + abs(pixel_size)),
    );
    out.instance = in.instance;
    out.inv_z = in.uv1.x;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let it = items[in.instance];
    // `uGUI/Compass`: the perspective-correct UV, the turned sample UV and
    // the angular window (computed for every item; the sample comes before
    // any branch).
    let compass = has(it.softness.z, 16u);
    let p = select(in.uv, in.uv / in.inv_z, compass) * 2.0 - 1.0;
    let turn = fract(atan2(p.x, p.y) / TAU);
    let t = saturate((turn - it.st.y) / (it.st.z - it.st.y));
    let window = sin(1.0 - abs(2.0 * t - 1.0));
    let sc = vec2<f32>(sin(it.st.x), cos(it.st.x));
    let turned = vec2<f32>(sc.y * p.x + sc.x * p.y, -sc.x * p.x + sc.y * p.y) * 0.5 + 0.5;
    let uv = select(in.uv, turned, compass);
    var c = (textureSample(main_tex, main_sampler, uv) + it.texture_add) * in.color;
    if compass {
        c.a = c.a * window;
    }
    if has(it.softness.z, 1u) {
        let m = saturate((it.clip_rect.zw - it.clip_rect.xy - abs(in.mask.xy)) * in.mask.zw);
        c.a = c.a * m.x * m.y;
    }
    if has(it.softness.z, 2u) {
        let inside = step(it.clip_rect.xy, in.canvas) * step(in.canvas, it.clip_rect.zw);
        c.a = c.a * inside.x * inside.y;
    }
    if has(it.softness.z, 4u) {
        c = vec4<f32>(c.rgb * c.a, c.a);
    }
    return c;
}
