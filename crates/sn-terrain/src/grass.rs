//! Terrain grass: block types that scatter a small mesh (grass, seaweed,
//! little corals) over their faces, as the game's `VoxelandGrassBuilder`
//! does. See `docs/formats/terrain-materials.md` § Grass.
//!
//! Per face (one quad of the surface, one voxel in size) of a grass type:
//! the face normal must lie in the type's tilt range; the face is cut into 4
//! sub-quads (corner, edge midpoints, centre), and each gets a tuft when a
//! draw passes the density (or Perlin noise over x/z stays below it). The
//! tuft stands at the sub-quad's centre plus a little jitter, turned from up
//! to the face normal, spun about its own up, turned −90° about x for meshes
//! modelled Z-up, and scaled. Per chunk of 16³ voxels all types together get
//! at most `max_verts` vertices and `max_tris` triangles: a type that would
//! exceed its share has its reduction raised so it fits.
//!
//! Positions are in voxel-index space (y up, like Unity's axes). The game's
//! faces are Voxeland's own; ours are the quads of our surface nets, which
//! have the same size but not the same corners, and we draw from our own
//! random numbers, so tufts don't sit where the game puts them.

use std::collections::BTreeMap;

use sn_mesh::Mesh;

/// How a block type scatters grass (`VoxelandTypeBase`'s grass fields).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassRule {
    pub density: f32,
    pub z_up: bool,
    pub jitter: f32,
    pub min_scale: f32,
    pub max_scale: f32,
    /// Allowed slope of the face, degrees from up.
    pub min_tilt: f32,
    pub max_tilt: f32,
    pub random_spin: bool,
    pub perlin: bool,
    pub perlin_period: f32,
}

/// The grass mesh of a block type, in its own space (Unity coordinates).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GrassTemplate {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GrassType {
    pub rule: GrassRule,
    pub template: GrassTemplate,
}

/// Limits per chunk (`clipmaps-*.json`, `grassSettings` of a level).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassBudget {
    /// Share of candidate spots dropped up front (0 at the finest level).
    pub reduction: f64,
    pub max_verts: usize,
    pub max_tris: usize,
    /// Chunk size in mesh units (16 voxels).
    pub chunk: f32,
}

impl Default for GrassBudget {
    /// The game's finest level ("high" preset).
    fn default() -> Self {
        GrassBudget {
            reduction: 0.0,
            max_verts: 10_000,
            max_tris: 10_000,
            chunk: 16.0,
        }
    }
}

/// All tufts of one grass type in one batch, merged into one mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GrassMesh {
    pub ty: u8,
    pub tufts: usize,
    /// Voxel-index space.
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    /// Random RGB per vertex; alpha = height above the face ÷ 5, clamped.
    pub colors: Vec<[f32; 4]>,
    /// The position in the game's grass object: its chunk (16-voxel cell of
    /// the finest clipmap level) is the object, with its origin at the
    /// chunk's corner (`cellId × 16` in the game's voxel coordinates, which
    /// are ours + ½). Some grass shaders sway by it.
    pub chunk_local: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// Small deterministic generator (SplitMix64).
struct Rng(u64);

impl Rng {
    fn new(key: &[u64]) -> Rng {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for k in key {
            for b in k.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
        }
        Rng(h)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn value(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

type Quat = [f32; 4];

fn mul(a: Quat, b: Quat) -> Quat {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn rotate(q: Quat, v: [f32; 3]) -> [f32; 3] {
    let p = mul(mul(q, [v[0], v[1], v[2], 0.0]), [-q[0], -q[1], -q[2], q[3]]);
    [p[0], p[1], p[2]]
}

fn angle_axis(degrees: f32, axis: [f32; 3]) -> Quat {
    let (s, c) = (degrees.to_radians() * 0.5).sin_cos();
    [axis[0] * s, axis[1] * s, axis[2] * s, c]
}

/// The shortest rotation taking +y to `n` (unit).
fn from_up(n: [f32; 3]) -> Quat {
    let d = n[1];
    if d < -0.999_999 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    // cross(up, n) = (n.z, 0, −n.x)
    let q = [n[2], 0.0, -n[0], 1.0 + d];
    let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    q.map(|v| v / len)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    a.map(|v| v * s)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let len = dot(a, a).sqrt();
    if len > 0.0 { scale(a, 1.0 / len) } else { a }
}

/// Gradient noise over the plane, about 0..1 with mean 0.5, period 1 per
/// lattice cell; our stand-in for Unity's `Mathf.PerlinNoise` (**hypothesis**:
/// the same range; the pattern itself differs).
fn perlin(x: f32, z: f32) -> f32 {
    fn hash(ix: i32, iz: i32) -> u64 {
        Rng::new(&[ix as u64, iz as u64, 0x5eed]).next_u64()
    }
    fn grad(ix: i32, iz: i32, dx: f32, dz: f32) -> f32 {
        let a = (hash(ix, iz) >> 40) as f32 / (1u64 << 24) as f32 * std::f32::consts::TAU;
        a.cos() * dx + a.sin() * dz
    }
    fn fade(t: f32) -> f32 {
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }
    let (x0, z0) = (x.floor(), z.floor());
    let (ix, iz) = (x0 as i32, z0 as i32);
    let (fx, fz) = (x - x0, z - z0);
    let n00 = grad(ix, iz, fx, fz);
    let n10 = grad(ix + 1, iz, fx - 1.0, fz);
    let n01 = grad(ix, iz + 1, fx, fz - 1.0);
    let n11 = grad(ix + 1, iz + 1, fx - 1.0, fz - 1.0);
    let (u, v) = (fade(fx), fade(fz));
    let a = n00 + u * (n10 - n00);
    let b = n01 + u * (n11 - n01);
    // Gradient noise in 2D stays within ±√½; map that to 0..1.
    0.5 + (a + v * (b - a)) * std::f32::consts::FRAC_1_SQRT_2
}

/// One face of the surface: its corners and averaged normal.
struct Face {
    corners: [[f32; 3]; 4],
    normal: [f32; 3],
}

impl Face {
    /// Sub-quad `k`: corner `k`, the midpoint towards the next corner, the
    /// face centre, the midpoint towards the previous corner.
    fn sub_quad(&self, k: usize) -> [[f32; 3]; 4] {
        let c = &self.corners;
        let mid = |a: [f32; 3], b: [f32; 3]| scale(add(a, b), 0.5);
        let centre = scale(add(add(c[0], c[1]), add(c[2], c[3])), 0.25);
        [
            c[k],
            mid(c[k], c[(k + 1) % 4]),
            centre,
            mid(c[(k + 3) % 4], c[k]),
        ]
    }
}

/// The candidate spots of `faces` that pass the tilt, reduction and
/// density tests, in order (`EnumerateGrass`).
fn enumerate<'f>(
    faces: &'f [Face],
    rule: &GrassRule,
    reduction: f64,
    key: &[u64],
) -> impl Iterator<Item = (&'f Face, [[f32; 3]; 4])> {
    let min_y = (rule.max_tilt.to_radians()).cos();
    let max_y = (rule.min_tilt.to_radians()).cos();
    let mut reduce = Rng::new(&[key, &[1]].concat());
    let mut place = Rng::new(&[key, &[2]].concat());
    let rule = *rule;
    faces
        .iter()
        .filter(move |f| f.normal[1] >= min_y && f.normal[1] <= max_y)
        .flat_map(|f| (0..4).map(move |k| (f, f.sub_quad(k))))
        .filter(move |(_, q)| {
            if reduction > 0.0 && f64::from(reduce.value()) < reduction {
                return false;
            }
            if rule.perlin {
                let c = scale(add(add(q[0], q[1]), add(q[2], q[3])), 0.25);
                perlin(c[0] / rule.perlin_period, c[2] / rule.perlin_period) <= rule.density
            } else {
                place.value() <= rule.density
            }
        })
}

/// Places one tuft and appends its vertices (`GrassPos.ComputeTransform`
/// and the builder's vertex loop).
fn place_tuft(
    out: &mut GrassMesh,
    face: &Face,
    q: &[[f32; 3]; 4],
    ty: &GrassType,
    chunk_origin: [f32; 3],
    rng: &mut Rng,
) {
    let rule = &ty.rule;
    let centre = scale(add(add(q[0], q[1]), add(q[2], q[3])), 0.25);
    let mut quat = from_up(face.normal);
    if rule.random_spin {
        quat = mul(quat, angle_axis(rng.value() * 360.0, [0.0, 1.0, 0.0]));
    }
    if rule.z_up {
        quat = mul(quat, angle_axis(-90.0, [1.0, 0.0, 0.0]));
    }
    let a = normalize(sub(q[1], q[0]));
    let b = normalize(sub(q[2], q[0]));
    let j = rule.jitter.min(0.5);
    let offset = add(
        scale(a, j * (rng.value() - 0.5)),
        scale(b, j * (rng.value() - 0.5)),
    );
    let s = rule.min_scale + (rule.max_scale - rule.min_scale) * rng.value();
    let origin = add(centre, offset);
    let base = out.positions.len() as u32;
    let t = &ty.template;
    for (i, &p) in t.positions.iter().enumerate() {
        let pos = add(origin, rotate(quat, scale(p, s)));
        let height = dot(sub(pos, origin), face.normal);
        out.positions.push(pos);
        out.chunk_local.push(sub(add(pos, [0.5; 3]), chunk_origin));
        out.normals.push(rotate(
            quat,
            t.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
        ));
        let tangent = t.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]);
        let r = rotate(quat, [tangent[0], tangent[1], tangent[2]]);
        out.tangents.push([r[0], r[1], r[2], tangent[3]]);
        out.uvs.push(t.uvs.get(i).copied().unwrap_or_default());
        out.colors.push([
            rng.value(),
            rng.value(),
            rng.value(),
            (height / 5.0).clamp(0.0, 1.0),
        ]);
    }
    out.indices.extend(t.indices.iter().map(|&i| base + i));
    out.tufts += 1;
}

/// The quads of a surface-nets mesh (triangle pairs `[a, b, c]`, `[a, c,
/// d]`) with their type (the solid side's material).
fn faces_by_chunk(
    mesh: &Mesh,
    types: &[Option<GrassType>],
    chunk: f32,
) -> BTreeMap<([i32; 3], u8), Vec<Face>> {
    let mut out: BTreeMap<([i32; 3], u8), Vec<Face>> = BTreeMap::new();
    for (pair, materials) in mesh
        .triangles
        .chunks_exact(2)
        .zip(mesh.triangle_materials.chunks_exact(2))
    {
        let (t0, t1) = (pair[0], pair[1]);
        let ty = materials[0];
        if t1[0] != t0[0] || t1[1] != t0[2] || materials[1] != ty {
            continue;
        }
        if types.get(usize::from(ty)).is_none_or(Option::is_none) {
            continue;
        }
        let ids = [t0[0], t0[1], t0[2], t1[2]];
        let Some(corners) = ids
            .iter()
            .map(|&i| mesh.positions.get(i as usize).copied())
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let normal = normalize(
            ids.iter()
                .filter_map(|&i| mesh.normals.get(i as usize).copied())
                .fold([0.0; 3], add),
        );
        let corners = [corners[0], corners[1], corners[2], corners[3]];
        let centre = scale(
            add(add(corners[0], corners[1]), add(corners[2], corners[3])),
            0.25,
        );
        let key = centre.map(|v| (v / chunk).floor() as i32);
        out.entry((key, ty))
            .or_default()
            .push(Face { corners, normal });
    }
    out
}

/// Scatters the grass of every grass type over `mesh` (one batch at full
/// resolution), with the per-chunk budget; one merged mesh per type.
/// `types` is indexed by block type id. Deterministic for a `seed`.
pub fn build_grass(
    mesh: &Mesh,
    types: &[Option<GrassType>],
    budget: &GrassBudget,
    seed: u64,
) -> Vec<GrassMesh> {
    let mut merged: BTreeMap<u8, GrassMesh> = BTreeMap::new();
    let mut used: BTreeMap<[i32; 3], (usize, usize)> = BTreeMap::new();
    // Chunks in order, types in ascending order within a chunk.
    for ((chunk, ty), faces) in faces_by_chunk(mesh, types, budget.chunk) {
        let Some(Some(grass)) = types.get(usize::from(ty)) else {
            continue;
        };
        let (verts, idx) = (grass.template.positions.len(), grass.template.indices.len());
        if verts == 0 || idx == 0 {
            continue;
        }
        let key = [
            seed,
            chunk[0] as u64,
            chunk[1] as u64,
            chunk[2] as u64,
            u64::from(ty),
        ];
        let (used_verts, used_idx) = used.entry(chunk).or_default();
        let room_verts = budget.max_verts.saturating_sub(*used_verts).min(65_535) / verts;
        let room_tris = (budget.max_tris * 3).saturating_sub(*used_idx) / idx;
        let cap = (room_verts.min(room_tris) as f32 * 0.8) as usize;
        let mut count = enumerate(&faces, &grass.rule, budget.reduction, &key).count();
        if count == 0 {
            continue;
        }
        let mut reduction = budget.reduction;
        if count > cap {
            let t = 1.0 - cap as f64 / count as f64;
            reduction = budget.reduction + (1.0 - budget.reduction) * t;
            count = cap;
        }
        let out = merged.entry(ty).or_insert_with(|| GrassMesh {
            ty,
            ..GrassMesh::default()
        });
        let mut rng = Rng::new(&[&key[..], &[3]].concat());
        let origin = chunk.map(|c| c as f32 * budget.chunk);
        for (face, q) in enumerate(&faces, &grass.rule, reduction, &key).take(count) {
            place_tuft(out, face, &q, grass, origin, &mut rng);
        }
        *used_verts += verts * count;
        *used_idx += idx * count;
    }
    merged.into_values().filter(|m| m.tufts > 0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_mesh::{Field, surface_nets};

    /// Flat ground of type 7 with its top at y = 4.5, 40 × 40 voxels.
    fn ground() -> Mesh {
        let mut f = Field::new([40, 10, 40]);
        for x in 0..40 {
            for z in 0..40 {
                for y in 0..10 {
                    let v = 4.5 - y as f32;
                    f.set([x, y, z], v.clamp(-1.0, 1.0), 7);
                }
            }
        }
        surface_nets(&f)
    }

    /// A blade: a triangle 1 unit tall along +z (modelled Z-up).
    fn blade() -> GrassTemplate {
        GrassTemplate {
            positions: vec![[-0.1, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.0, 1.0]],
            normals: vec![[0.0, -1.0, 0.0]; 3],
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]],
            indices: vec![0, 1, 2],
        }
    }

    fn rule(density: f32) -> GrassRule {
        GrassRule {
            density,
            z_up: true,
            jitter: 0.5,
            min_scale: 1.0,
            max_scale: 2.0,
            min_tilt: 0.0,
            max_tilt: 45.0,
            random_spin: true,
            perlin: false,
            perlin_period: 10.0,
        }
    }

    fn types(rule: GrassRule) -> Vec<Option<GrassType>> {
        let mut t = vec![None; 256];
        t[7] = Some(GrassType {
            rule,
            template: blade(),
        });
        t
    }

    fn faces(mesh: &Mesh) -> usize {
        mesh.triangles.len() / 2
    }

    #[test]
    fn full_density_gives_four_tufts_per_face() {
        let mesh = ground();
        let budget = GrassBudget {
            max_verts: usize::MAX / 2,
            max_tris: usize::MAX / 8,
            ..GrassBudget::default()
        };
        let out = build_grass(&mesh, &types(rule(1.0)), &budget, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tufts, 4 * faces(&mesh));
        assert_eq!(out[0].positions.len(), 3 * out[0].tufts);
        assert_eq!(out[0].indices.len(), 3 * out[0].tufts);
        assert!(build_grass(&mesh, &types(rule(0.0)), &budget, 1).is_empty());
        // About half at density 0.5.
        let half = build_grass(&mesh, &types(rule(0.5)), &budget, 1)[0].tufts as f32
            / (4 * faces(&mesh)) as f32;
        assert!((half - 0.5).abs() < 0.05, "{half}");
    }

    #[test]
    fn tufts_stand_on_the_surface_pointing_up() {
        let mesh = ground();
        let out = &build_grass(&mesh, &types(rule(1.0)), &GrassBudget::default(), 3)[0];
        for tuft in out.positions.chunks(3) {
            // Base vertices on the ground (jitter moves along it), the tip up
            // by the scale (1..2): the Z-up blade was turned upright.
            assert!((tuft[0][1] - 4.5).abs() < 1e-3, "{:?}", tuft[0]);
            assert!(tuft[2][1] - 4.5 >= 1.0 - 1e-3 && tuft[2][1] - 4.5 <= 2.0 + 1e-3);
        }
        for c in out.colors.chunks(3) {
            assert_eq!(c[0][3], 0.0);
            assert!(c[2][3] > 0.19 && c[2][3] <= 0.4 + 1e-6, "{}", c[2][3]);
        }
        // Chunk-local positions: the world position + ½ minus a multiple of
        // 16; tuft bases (ground at y 4.5) inside their chunk (jitter and
        // blades reach a little over its edges).
        assert_eq!(out.chunk_local.len(), out.positions.len());
        for (p, l) in out.positions.iter().zip(&out.chunk_local) {
            for a in 0..3 {
                let corner = p[a] + 0.5 - l[a];
                assert!(
                    (corner / 16.0 - (corner / 16.0).round()).abs() < 1e-4,
                    "{p:?} {l:?}"
                );
                assert!(l[a] > -1.0 && l[a] < 17.0, "{p:?} {l:?}");
            }
        }
        for tuft in out.chunk_local.chunks(3) {
            assert!((tuft[0][1] - 5.0).abs() < 1e-3, "{:?}", tuft[0]);
        }
    }

    #[test]
    fn steep_faces_are_skipped() {
        let mut r = rule(1.0);
        r.min_tilt = 50.0;
        r.max_tilt = 90.0;
        assert!(build_grass(&ground(), &types(r), &GrassBudget::default(), 1).is_empty());
    }

    #[test]
    fn the_chunk_budget_holds() {
        let mesh = ground();
        let budget = GrassBudget {
            max_verts: 300,
            ..GrassBudget::default()
        };
        let out = &build_grass(&mesh, &types(rule(1.0)), &budget, 1)[0];
        // 40 × 40 voxels: 3 × 3 chunks of 16 (partly), each at most 300 ×
        // 0.8 vertices = 80 tufts; without the budget there would be ~6,000.
        assert!(out.tufts <= 9 * 80, "{}", out.tufts);
        assert!(out.tufts >= 9 * 60, "{}", out.tufts);
    }

    #[test]
    fn the_same_seed_gives_the_same_grass() {
        let mesh = ground();
        let t = types(rule(0.6));
        let b = GrassBudget::default();
        assert_eq!(build_grass(&mesh, &t, &b, 5), build_grass(&mesh, &t, &b, 5));
        assert_ne!(build_grass(&mesh, &t, &b, 5), build_grass(&mesh, &t, &b, 6));
    }

    #[test]
    fn perlin_noise_is_smooth_and_centred() {
        let mut sum = 0.0;
        let mut n = 0;
        for i in 0..200 {
            for j in 0..200 {
                let (x, z) = (i as f32 * 0.37, j as f32 * 0.29);
                let v = perlin(x, z);
                assert!((0.0..=1.0).contains(&v), "{v}");
                assert!((perlin(x + 0.01, z) - v).abs() < 0.05);
                sum += v;
                n += 1;
            }
        }
        let mean = sum / n as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
        let mut r = rule(0.5);
        r.perlin = true;
        let out = build_grass(&ground(), &types(r), &GrassBudget::default(), 1);
        assert!(!out.is_empty());
    }

    #[test]
    fn rotations_match_unity() {
        // From up to +x: up maps onto +x.
        let q = from_up([1.0, 0.0, 0.0]);
        let v = rotate(q, [0.0, 1.0, 0.0]);
        assert!((v[0] - 1.0).abs() < 1e-6 && v[1].abs() < 1e-6);
        // Unity's AngleAxis(−90, right) turns +z onto +y.
        let v = rotate(angle_axis(-90.0, [1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
        assert!((v[1] - 1.0).abs() < 1e-6 && v[2].abs() < 1e-6, "{v:?}");
        assert_eq!(from_up([0.0, -1.0, 0.0]), [1.0, 0.0, 0.0, 0.0]);
    }
}
