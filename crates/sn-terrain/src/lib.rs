//! Turns terrain batches into meshing fields and meshes.
//!
//! Pure: the caller loads batches (e.g. with `sn-install`) and hands them in.
//! Each batch is meshed on its own with a 1-sample apron from its 26
//! neighbours, so meshes of neighbouring batches at the same level of detail
//! join without gaps (see the chunking rule in `sn-mesh`). Different levels
//! leave small cracks; hide them with `sn_mesh::add_skirts`.

use sn_mesh::{Field, Mesh, surface_nets};
use sn_octree::{Batch, OCTREE_SIZE, Voxel};
use sn_world::{BatchCoord, WorldIndex};

/// Coarsest supported level of detail: samples every 2^3 = 8 voxels. Batch
/// sizes (160, or 96 at the world edge) are multiples of 32, so every level
/// up to 5 divides them evenly.
pub const MAX_LOD: u32 = 3;

/// Size of a full batch in voxels along each axis.
pub fn batch_voxels(index: &WorldIndex) -> [i32; 3] {
    [0, 1, 2].map(|a| (index.batch_octrees[a] * index.octree_size) as i32)
}

/// A parsed batch and its size in octrees.
pub struct TerrainBatch {
    pub batch: Batch,
    pub octree_dims: [usize; 3],
}

impl TerrainBatch {
    /// Size in voxels.
    pub fn voxel_dims(&self) -> [usize; 3] {
        self.octree_dims.map(|n| n * OCTREE_SIZE)
    }

    /// The voxel at `pos` relative to the batch corner (`None` if outside or
    /// if the octree is malformed).
    pub fn sample(&self, pos: [usize; 3]) -> Option<Voxel> {
        self.batch.sample(self.octree_dims, pos)
    }
}

/// A batch and its 26 neighbours (`None` where a neighbour has no file).
pub struct Neighbourhood<'a> {
    pub coord: BatchCoord,
    /// Size of a full batch in voxels (see [`batch_voxels`]).
    pub batch_voxels: [i32; 3],
    /// Indexed `(dx + 1) + 3 (dy + 1) + 9 (dz + 1)`; the centre is 13.
    batches: [Option<&'a TerrainBatch>; 27],
}

impl<'a> Neighbourhood<'a> {
    pub fn new(
        coord: BatchCoord,
        batch_voxels: [i32; 3],
        mut lookup: impl FnMut(BatchCoord) -> Option<&'a TerrainBatch>,
    ) -> Self {
        let batches = std::array::from_fn(|i| {
            let d = [i % 3, (i / 3) % 3, i / 9].map(|n| n as i32 - 1);
            lookup(coord.offset(d[0], d[1], d[2]))
        });
        Neighbourhood {
            coord,
            batch_voxels,
            batches,
        }
    }

    pub fn center(&self) -> Option<&'a TerrainBatch> {
        self.batches[13]
    }

    fn start(&self) -> [i32; 3] {
        let c = [self.coord.x, self.coord.y, self.coord.z];
        [0, 1, 2].map(|a| c[a] * self.batch_voxels[a])
    }

    /// The voxel at a global voxel index near this batch. Voxels in
    /// neighbours that have no file are copied from the nearest voxel of the
    /// centre batch: missing batches are uniform, so this adds no surface.
    fn voxel(&self, center: &TerrainBatch, global: [i32; 3], clamped: &mut usize) -> Voxel {
        let size = self.batch_voxels;
        let c = [self.coord.x, self.coord.y, self.coord.z];
        let batch = [0, 1, 2].map(|a| global[a].div_euclid(size[a]));
        if (0..3).all(|a| (batch[a] - c[a]).abs() <= 1) {
            let d = [0, 1, 2].map(|a| (batch[a] - c[a] + 1) as usize);
            if let Some(neighbour) = self.batches[d[0] + 3 * d[1] + 9 * d[2]] {
                let local = [0, 1, 2].map(|a| (global[a] - batch[a] * size[a]) as usize);
                if let Some(voxel) = neighbour.sample(local) {
                    return voxel;
                }
            }
        }
        *clamped += 1;
        let start = self.start();
        let dims = center.voxel_dims();
        let local = [0, 1, 2].map(|a| (global[a] - start[a]).clamp(0, dims[a] as i32 - 1) as usize);
        center.sample(local).unwrap_or_default()
    }
}

/// A batch's meshing field plus facts about how its apron was filled.
pub struct BatchField {
    pub field: Field,
    /// Apron samples copied from the batch itself because the neighbour
    /// holding them has no file.
    pub clamped_samples: usize,
}

/// The field for the centre batch at level of detail `lod` (samples every
/// 2^lod voxels), plus a 1-sample apron. Field origin and mesh positions are
/// global voxel indices. `None` if the centre batch has no file.
///
/// # Panics
/// If `lod > MAX_LOD`.
pub fn batch_field(around: &Neighbourhood, lod: u32) -> Option<BatchField> {
    assert!(lod <= MAX_LOD, "level of detail {lod} > {MAX_LOD}");
    let center = around.center()?;
    let step = 1i32 << lod;
    let dims = center.voxel_dims().map(|n| n / step as usize + 2);
    let origin = around.start().map(|s| s - step);
    let mut field = Field::new(dims).with_origin(origin).with_step(step);
    let mut clamped_samples = 0;
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let local = [x, y, z];
                let global = [0, 1, 2].map(|a| origin[a] + local[a] as i32 * step);
                let voxel = around.voxel(center, global, &mut clamped_samples);
                field.set(local, voxel.signed_density(), voxel.ty);
            }
        }
    }
    Some(BatchField {
        field,
        clamped_samples,
    })
}

/// Meshes the centre batch at level of detail `lod`. Positions are global
/// voxel indices; convert with `sn_world::voxel_to_world`. `None` if the
/// centre batch has no file.
pub fn batch_mesh(around: &Neighbourhood, lod: u32) -> Option<Mesh> {
    batch_field(around, lod).map(|f| surface_nets(&f.field))
}

/// A stable false colour per terrain type id (sRGB, 0..1), until real terrain
/// materials are decoded (M6).
pub fn debug_colour(ty: u8) -> [f32; 3] {
    let hue = (f32::from(ty) * 0.618_034).fract() * 6.0;
    let (s, v) = (0.55, 0.85);
    let f = hue.fract();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match hue as u32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}
