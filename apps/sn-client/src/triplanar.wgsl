#define_import_path sn_client::triplanar

// Normal maps: X = alpha × red (DXT5nm keeps X in alpha, red = 1), Y = green.
fn unpack_normal(t: vec4<f32>) -> vec3<f32> {
    let xy = vec2<f32>(t.a * t.r, t.g) * 2.0 - 1.0;
    return vec3<f32>(xy, sqrt(1.0 - min(dot(xy, xy), 1.0)));
}

// Tangent-space normal → world, one frame per projection (n: surface normal).
fn from_x(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let s = sign(n.x);
    return t.x * s * vec3<f32>(n.y, n.x, -n.z) + t.y * s * vec3<f32>(-n.z, n.y, n.x) + t.z * n;
}

fn from_y(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let s = sign(n.y);
    return t.x * s * vec3<f32>(n.y, -n.x, n.z) + t.y * s * vec3<f32>(n.x, -n.z, n.y) + t.z * n;
}

fn from_z(t: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let s = sign(n.z);
    return t.x * s * vec3<f32>(n.x, n.z, n.y) + t.y * s * vec3<f32>(n.z, n.y, n.x) + t.z * n;
}

// Weight + splotch → 0..1, the shape shared by the border alpha and the
// border tint.
fn border(weight: f32, splotch: f32, range: f32, offset: f32) -> f32 {
    let r = max(range, 1e-4);
    let threshold = (r + 1.0) / (1.01 - offset) * (1.0 - weight - offset) - r;
    return saturate((splotch - threshold) / r);
}

