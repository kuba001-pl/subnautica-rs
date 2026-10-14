// `Hidden/UI/CircularBar` (M9u6b1; `docs/formats/ui.md` § The HUD's bars),
// ported from its compiled D3D11 programs (8 vertex + 8 fragment: the
// keywords `BORDER`, `OVERLAY`, `ROUND_CORNERS`; the variants are the same
// terms switched off, here by flags).
//
// Vertex: colour = vertex colour × `_Color`; the two overlay UVs are the
// UV through `_Overlay1_ST` and `_Overlay2_ST`.
// Fragment, in the quad's UV mapped to −1…1 (p):
// - nothing at all when `_Value` ≤ 0;
// - the ring: v = 1 − |2 saturate((|p| + w − 1) ÷ w) − 1| (1 at the ring's
//   middle, 0 at its edges), w = `_Width`;
// - cut where the clockwise angle from the top, as a fraction of a turn,
//   is past `_Value`;
// - `ROUND_CORNERS`: v = max(v, 1 − |p − c| × 2 ÷ w) for the caps at the
//   start (0, 1 − w ÷ 2) and the end (sin a, cos a) × (1 − w ÷ 2), a =
//   `_Value` × 2π;
// - the hard clip on `_ClipRect`; discard where v × clip < 0.001;
// - colour = (`_MainTex` + `_TextureSampleAdd`) × vertex colour; alpha ×
//   smoothstep where 0 < v < `_EdgeWidth`;
// - `BORDER`: where 0 < v < `_BorderWidth`, rgb = lerp(`_BorderColor`, rgb,
//   smoothstep(v ÷ `_BorderWidth`));
// - `OVERLAY`: two samples of `_OverlayTex`, each at its overlay UV with x
//   shifted by −floor(y) × `_OverlayShift`, premultiplied, added × its
//   `_OverlayAlpha`.

struct BarItem {
    clip_from_canvas: mat4x4<f32>,
    // `_Color`, linear.
    color: vec4<f32>,
    overlay1_st: vec4<f32>,
    overlay2_st: vec4<f32>,
    // `_Width`, `_Value`, `_EdgeWidth`, `_BorderWidth`.
    bar: vec4<f32>,
    // `_BorderColor`, linear.
    border_color: vec4<f32>,
    // `_OverlayShift.xy`, `_OverlayAlpha.xy`.
    overlay: vec4<f32>,
    // `_TextureSampleAdd`.
    texture_add: vec4<f32>,
    // `_ClipRect`.
    clip_rect: vec4<f32>,
    // Flags (1 `BORDER`, 2 `OVERLAY`, 4 `ROUND_CORNERS`), unused.
    flags: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> items: array<BarItem>;
@group(1) @binding(0) var main_tex: texture_2d<f32>;
@group(1) @binding(1) var main_sampler: sampler;
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
    @location(3) overlay_uv: vec4<f32>,
    @location(4) @interpolate(flat) instance: u32,
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

// The programs' `atan2(x, y)` turned into a fraction of a turn: 0 at the
// top, growing clockwise, 0…1.
fn turn(p: vec2<f32>) -> f32 {
    if 0.0 < p.x {
        return atan2(p.x, p.y) * 0.159155;
    }
    return atan2(-p.x, -p.y) * 0.159155 + 0.5;
}

fn smooth01(x: f32) -> f32 {
    return x * x * (3.0 - 2.0 * x);
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    let it = items[in.instance];
    var out: VertexOut;
    out.clip = it.clip_from_canvas * vec4<f32>(in.position, 1.0);
    out.color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a) * it.color;
    out.uv = in.uv;
    out.canvas = in.position.xy;
    out.overlay_uv = vec4<f32>(
        in.uv * it.overlay1_st.xy + it.overlay1_st.zw,
        in.uv * it.overlay2_st.xy + it.overlay2_st.zw,
    );
    out.instance = in.instance;
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let it = items[in.instance];
    // Sampled first: WGSL needs uniform control flow for implicit
    // derivatives (the programs sample after their branches).
    let tex = textureSample(main_tex, main_sampler, in.uv) + it.texture_add;
    let shift = floor(in.overlay_uv.yw) * it.overlay.xy;
    let o1 = textureSample(overlay_tex, overlay_sampler, vec2<f32>(in.overlay_uv.x - shift.x, in.overlay_uv.y));
    let o2 = textureSample(overlay_tex, overlay_sampler, vec2<f32>(in.overlay_uv.z - shift.y, in.overlay_uv.w));
    let width = it.bar.x;
    let value = it.bar.y;
    if value <= 0.0 {
        return vec4<f32>(0.0);
    }
    let inv = 1.0 / width;
    let p = in.uv * 2.0 - 1.0;
    var v = saturate(inv * (length(p) + width - 1.0));
    v = 1.0 - abs(v * 2.0 - 1.0);
    if value - turn(p) < 0.0 {
        v = 0.0;
    }
    if has(it.flags.x, 4u) {
        let middle = 1.0 - width * 0.5;
        let a = value * TAU;
        let start = 1.0 - abs(length(vec2<f32>(0.0, middle) - p) * 2.0 * inv);
        let end = 1.0 - abs(length(vec2<f32>(sin(a), cos(a)) * middle - p) * 2.0 * inv);
        v = max(max(v, start), end);
    }
    let inside = step(it.clip_rect.xy, in.canvas) * step(in.canvas, it.clip_rect.zw);
    let shown = v * inside.x * inside.y;
    if shown - 0.001 < 0.0 {
        discard;
    }
    var c = tex * in.color;
    let edge = it.bar.z;
    if 0.0 < shown && shown < edge {
        c.a = smooth01(shown / edge) * c.a;
    }
    let border = it.bar.w;
    if has(it.flags.x, 1u) && 0.0 < shown && shown < border {
        c = vec4<f32>(mix(it.border_color.rgb, c.rgb, smooth01(shown / border)), c.a);
    }
    if has(it.flags.x, 2u) {
        c = vec4<f32>(c.rgb + o1.rgb * o1.a * it.overlay.z + o2.rgb * o2.a * it.overlay.w, c.a);
    }
    return c;
}
