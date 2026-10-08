// The "High" water quality's waves (docs/formats/water.md § High quality
// waves): the game's spectrum update and displacement packing, with our own
// FFT (any correct one gives the same result: a 512² transform, sign −1, no
// scaling). Written for subnautica-rs from the behaviour of the game's
// shaders.

const N: u32 = 512u;
const IN_WIDTH: u32 = 516u;
const SLICE: u32 = 262144u; // N × N

// Must match `WaterSim` in water_surface.rs (only `fft` is used here).
struct WaterSim {
    frames: vec4<f32>,
    foam: vec4<f32>,
    // time (s), choppy scale, on, unused
    fft: vec4<f32>,
}

@group(0) @binding(0) var<storage, read> h0: array<vec2<f32>>;
@group(0) @binding(1) var<storage, read> omega: array<f32>;
@group(0) @binding(2) var<storage, read_write> ht: array<vec2<f32>>;
@group(0) @binding(3) var<storage, read_write> tmp: array<vec2<f32>>;
@group(0) @binding(4) var<uniform> params: WaterSim;
@group(0) @binding(5) var displacement: texture_storage_2d<rgba16float, write>;

// The spectrum at this time (the game's `UpdateSpectrumCS`): height and the
// two horizontal ("choppy") displacements, one slice each.
@compute @workgroup_size(16, 16)
fn update_spectrum(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= N || id.y >= N {
        return;
    }
    let h0k = h0[id.y * IN_WIDTH + id.x];
    let h0mk = h0[(N - id.y) * IN_WIDTH + (N - id.x)];
    let w = omega[id.y * IN_WIDTH + id.x] * params.fft.x;
    let c = cos(w);
    let s = sin(w);
    let sum = h0k + h0mk;
    let diff = h0k - h0mk;
    let h = vec2<f32>(sum.x * c - sum.y * s, diff.x * s + diff.y * c);
    let k = vec2<f32>(f32(id.x), f32(id.y)) - f32(N) * 0.5;
    let len = length(k);
    let kn = select(vec2<f32>(0.0), k / len, len > 0.0);
    let i = id.y * N + id.x;
    ht[i] = h;
    ht[SLICE + i] = vec2<f32>(kn.x * h.y, -kn.x * h.x);
    ht[2u * SLICE + i] = vec2<f32>(kn.y * h.y, -kn.y * h.x);
}

var<workgroup> line: array<vec2<f32>, 512>;

fn reverse9(i: u32) -> u32 {
    return reverseBits(i) >> 23u;
}

fn cmul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

// One 512-point FFT (sign −1) on `line`, 256 threads, bit-reversed input.
fn fft_line(t: u32) {
    for (var half = 1u; half < N; half = half * 2u) {
        workgroupBarrier();
        let j = t % half;
        let i0 = (t / half) * 2u * half + j;
        let i1 = i0 + half;
        let angle = -3.14159265358979 * f32(j) / f32(half);
        let wv = vec2<f32>(cos(angle), sin(angle));
        let a = line[i0];
        let b = cmul(line[i1], wv);
        line[i0] = a + b;
        line[i1] = a - b;
    }
    workgroupBarrier();
}

// Rows: `ht` → `tmp`. Workgroup (row, slice).
@compute @workgroup_size(256)
fn fft_rows(@builtin(local_invocation_index) t: u32, @builtin(workgroup_id) g: vec3<u32>) {
    let base = g.y * SLICE + g.x * N;
    line[reverse9(t)] = ht[base + t];
    line[reverse9(t + 256u)] = ht[base + t + 256u];
    fft_line(t);
    tmp[base + t] = line[t];
    tmp[base + t + 256u] = line[t + 256u];
}

// Columns: `tmp` → `ht`. Workgroup (column, slice).
@compute @workgroup_size(256)
fn fft_columns(@builtin(local_invocation_index) t: u32, @builtin(workgroup_id) g: vec3<u32>) {
    let base = g.y * SLICE + g.x;
    line[reverse9(t)] = tmp[base + t * N];
    line[reverse9(t + 256u)] = tmp[base + (t + 256u) * N];
    fft_line(t);
    ht[base + t * N] = line[t];
    ht[base + (t + 256u) * N] = line[t + 256u];
}

// The displacement map (cm): the real parts, with the checkerboard sign that
// centres the spectrum (the game's displacement shader).
@compute @workgroup_size(16, 16)
fn pack(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= N || id.y >= N {
        return;
    }
    let i = id.y * N + id.x;
    let sign = select(1.0, -1.0, ((id.x + id.y) & 1u) == 1u);
    let choppy = params.fft.y;
    let d = vec4<f32>(
        sign * ht[SLICE + i].x * choppy,
        sign * ht[i].x,
        sign * ht[2u * SLICE + i].x * choppy,
        0.0,
    );
    textureStore(displacement, vec2<i32>(id.xy), d);
}
