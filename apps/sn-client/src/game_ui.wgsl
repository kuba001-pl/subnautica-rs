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
// - `UI/Icon` with `SLICE_9_GRID` (M9u8b2b, flag 32; `st` = `_Radius`,
//   `_Size`): the UV mapped round a rounded border onto a strip texture
//   (`sn_ui::icon::slice9_uv`, which this mirrors); then as `UI/Icon`.
// - `UI/Circle` (M9u8b2b, flag 64; `st` = `_TileX`, `_TileY`, `_Size`): the
//   UV mapped to angle and radius (`sn_ui::icon::circle_uv`); the hard clip.
// Both read `_MainTex_TexelSize.w` as the bound texture's height.
// - `UI/Icon` with `NOTIFICATION` (M9u8b5b, flag 128, with or without
//   `SLICE_9_GRID`; blobs 144 and 176; `notification` = strength,
//   `_PDATime`, `_Size`): rgb × (1 + 0.75 · strength · the overlay's red at
//   (|UV − ½| · `_Size` × 0.015152 − 3.18 · `_PDATime`, ½)); alpha as before
//   (`sn_ui::notifications::glow`, which this mirrors).
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
    // `uGUI/Compass`, 32 `SLICE_9_GRID`, 64 `UI/Circle`.
    softness: vec4<f32>,
    // `_ScreenParams.xy`, the projection's [0][0] and [1][1].
    pixel: vec4<f32>,
    // `_NotificationStrength`, `_PDATime`, `_Size` (flag 128).
    notification: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> items: array<Item>;
@group(1) @binding(0) var main_tex: texture_2d<f32>;
@group(1) @binding(1) var main_sampler: sampler;
// `_NotificationOverlayTex` (white when unused).
@group(2) @binding(0) var overlay_tex: texture_2d<f32>;
@group(2) @binding(1) var overlay_sampler: sampler;

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

// `sn_ui::icon::slice9_uv`.
fn slice9_uv(uv: vec2<f32>, radius: f32, size: vec2<f32>, h: f32) -> vec2<f32> {
    let r = min(min(size.y, size.x) * 0.5, radius);
    let b = r / size;
    let left = uv.x <= b.x;
    let not_right = uv.x <= 1.0 - b.x;
    let mid_u = (uv.x - b.x) / (1.0 - 2.0 * b.x);
    let t_left = 1.0 - uv.x / b.x;
    let t_right = (uv.x - (1.0 - b.x)) / b.x;
    var st: vec2<f32>;
    if uv.y <= b.y {
        let t_bottom = 1.0 - uv.y / b.y;
        st = select(vec2<f32>(0.5, length(vec2<f32>(t_right, t_bottom))), vec2<f32>(mid_u, t_bottom), not_right);
        st = select(st, vec2<f32>(0.5, length(vec2<f32>(t_left, t_bottom))), left);
    } else if uv.y <= 1.0 - b.y {
        let mid_v = (uv.y - b.y) / (1.0 - 2.0 * b.y);
        st = select(vec2<f32>(mid_v, t_right), vec2<f32>(0.5, 0.0), not_right);
        st = select(st, vec2<f32>(mid_v, t_left), left);
    } else {
        let t_top = (uv.y - (1.0 - b.y)) / b.y;
        st = select(vec2<f32>(0.5, length(vec2<f32>(t_top, t_right))), vec2<f32>(mid_u, t_top), not_right);
        st = select(st, vec2<f32>(0.5, length(vec2<f32>(t_top, t_left))), left);
    }
    let k = (r - h) / r;
    return vec2<f32>(st.x, (st.y - k) / (1.0 - k));
}

// `sn_ui::icon::circle_uv`.
fn circle_uv(uv: vec2<f32>, tile: vec2<f32>, size: f32, h: f32) -> vec2<f32> {
    let p = uv * 2.0 - 1.0;
    let fold = p.x < 0.0;
    let q = select(p, -p, fold);
    let half = select(0.0, 0.5, fold);
    let u = (atan2(q.y, q.x) * 0.159155 + half) * tile.x;
    let k = (size - 2.0 * h) / size;
    let v = saturate((length(p) - k) / (1.0 - k)) * tile.y;
    return vec2<f32>(u, v);
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
    let h = f32(textureDimensions(main_tex).y);
    var uv = select(in.uv, turned, compass);
    if has(it.softness.z, 32u) {
        uv = slice9_uv(in.uv, it.st.x, it.st.yz, h);
    }
    if has(it.softness.z, 64u) {
        uv = circle_uv(in.uv, it.st.xy, it.st.z, h);
    }
    var c = (textureSample(main_tex, main_sampler, uv) + it.texture_add) * in.color;
    // The glow's overlay (sampled for every item, used with flag 128).
    let n = it.notification;
    let ring = length((in.uv - 0.5) * n.zw) * 0.015152 - 3.18 * n.y;
    let glow = textureSample(overlay_tex, overlay_sampler, vec2<f32>(ring, 0.5)).r;
    if has(it.softness.z, 128u) {
        c = vec4<f32>(c.rgb * (1.0 + 0.75 * n.x * glow), c.a);
    }
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
