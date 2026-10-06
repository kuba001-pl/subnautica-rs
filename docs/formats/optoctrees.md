# Terrain octrees: `.optoctrees`

Location: `Subnautica_Data/StreamingAssets/SNUnmanagedData/Build18/CompiledOctreesCache/`
Described build: game build `10` (2025-10-03), data folder `Build18`.
Reader: `crates/sn-octree`. Inspector: `sn-inspect octree`, `sn-inspect orient`.

Each fact is marked **confirmed** (with how) or **hypothesis**.

## World layout

From `index.txt` (see [world-index.md](world-index.md)) — **confirmed**:

| Unit | Size | Count in world |
|---|---|---|
| voxel | 1 unit (presumably 1 m) | 4096 × 3200 × 4096 |
| octree | 32³ voxels | 128 × 100 × 128 |
| batch (one file) | 5 × 5 × 5 octrees = 160³ voxels | 26 × 20 × 26 (13,520) |

- File name: `compiled-batch-X-Y-Z.optoctrees`, batch coordinates.
  5,416 of 13,520 possible files exist. Missing batches are **uniform**
  (entirely solid or entirely empty). Evidence: the faces of existing batches
  that touch a missing neighbour are almost always 0% or 100% solid (probe over
  all files, M2). Below the world (rows Y 0–6) they are solid. In the middle of
  the world they can be either. Coordinates span X 0–25, Y 7–19, Z 0–25.
- 128 = 25·5 + 3, so batches with X = 25 or Z = 25 are only 3 octrees wide on
  that axis and hold 75 octrees. **Confirmed**: exactly 157 files hold 75
  octrees and all are on those edges; the other 5,259 hold 125. No corner
  batch (X = 25 and Z = 25) exists.
- **+Y is up** — **confirmed**: the share of non-empty voxels per batch row
  falls from 100% (Y = 7) to 0.18% (Y = 19).
- Voxel → Unity world position: **plausible hypothesis** `world = voxel + 0.5 − (2048, 3040, 2048)`.
  It puts the top of batch row Y = 19 at world y = 160. Under it, the seabed at
  world (0, 0) (lifepod start, Safe Shallows) is at y = −17, and the highest
  terrain in the world is y = +156 at (x 345, z 907), plausibly the Mountain
  Island peak. Both fit the game. Mirroring of x/z versus the in-game map is
  not yet verified (M3, landmarks).

## File layout — confirmed

All integers little-endian.

```
i32  version                    // 4 in all 5,416 files
repeat until end of file:       // one entry per octree in the batch
  u16  node_count
  node_count × node:
    u8   type
    u8   density
    u16  first_child
```

Confirmed by parsing every file: each is consumed exactly to its last byte, and
the octree count always equals the batch's size in octrees.

## Node tree — confirmed

- Node 0 is the root and covers the whole 32³ octree.
- `first_child == 0` → leaf: every voxel in the node's cube has the node's
  `type` and `density`.
- Otherwise the node's 8 children are nodes `first_child .. first_child + 8`.
- Every one of the 669,150 octrees passes `Octree::validate`: all links in
  bounds, every non-root node has exactly one parent, no unreachable nodes, no
  children below single-voxel depth (depth 5). So leaves tile each 32³ cube
  exactly once (669,150 × 32,768 = 21,926,707,200 voxels covered).
- Largest octree: 37,449 nodes = 1 + 8 + 64 + 512 + 4,096 + 32,768, a complete tree.
- Children appear in breadth-first-like order (root's children are 1–8, then
  the first inner child's children, …) — **observed** in a hex dump; nothing
  depends on it.
- Inner (non-leaf) nodes also carry `type`/`density` values, probably a
  summary for coarser levels of detail — **hypothesis**, useful for LOD (M4).

## Spatial order — confirmed

Both orders list axes from fastest- to slowest-varying:

- **Child order ZYX**: bit 0 of a child index (0–7) moves +half along z,
  bit 1 along y, bit 2 along x. I.e. `child = x·4 + y·2 + z`.
- **Octree order ZYX**: octree `i` in the file sits at
  `z = i % dz`, `y = (i / dz) % dy`, `x = i / (dz·dy)` (in octrees within the batch).

How this was confirmed (`sn-inspect orient X Y Z`): rasterize a batch and its
+X/+Y/+Z neighbours under all 36 combinations of child order × octree order,
then compare how often empty/solid flips between face-adjacent voxels (a) inside
octrees, (b) across octree seams, (c) across batch seams. Only the right
combination makes seams look like the interior. On six detailed batches
(7-17-10, 9-16-5, 4-16-11, 6-17-9, 5-16-19, 22-17-11) and on 12-18-12, ZYX/ZYX
was best every time with seam/interior ratio 0.76–1.11; the runner-up was 1.4–10×
worse. Uniform batches (all rock or all water) carry no signal, and the tool
reports them as inconclusive.

Inside a single octree, all six axis orders score identically (an axis swap
just transposes the octree), so that check only shows that the tree is spatial
at all: a scrambled child order creates 2.6× as many empty/solid faces in batch
12-18-12.

Mirrored axes (child bit meaning −half instead of +half) were not tested; the
seam results leave no room for them to be needed.

## Values

### `density` — confirmed: quantised signed distance

- **Surface threshold:** every voxel with density 1–125 is empty and every
  voxel with density ≥ 126 is solid. The surface lies between 125 and 126.
  Checked on every leaf in the world (263 M): density side and type (0 = empty)
  never disagree (`sn-inspect octree --all`, "density vs type").
- **Scale:** near the surface (density 90–160), the gradient is about 13–15
  density units per voxel, so it is a signed distance at about 1/15-voxel
  precision. Further out, the gradient drops (about 3/voxel at 210–252,
  about 1/voxel around 40–90), so far values are compressed or clamped. The
  exact curve is unknown and not needed for meshing.
- **0 means "far from the surface"**: 68,769 leaves in batch 12-18-12, mostly
  large uniform ones. Which side they are on is given by `type`.
- Evidence: `sn-inspect density X Y Z` (empty/solid/on-surface counts per
  density range) and a gradient probe over batch 7-17-10.
- Meshing uses `signed = density − 125.5` (positive = solid), and ±126 for
  density 0 (`Voxel::signed_density`).

### `type`

- 0 = empty (water or air) — **confirmed** as far as possible: it agrees with
  the density sign on every leaf in the world.
- 210 distinct type ids occur. The type → material mapping is **unknown**
  (probably the Voxeland block-type table in the Unity assets; `meta.txt` says
  `39` / `BlockPrefabs`). M6.

### Meshing result (M2)

Surface nets over `signed density` gives clean terrain:
- batch 7-17-10: 220k triangles;
- 3×3×3 batches around 12-18-12: 2.0 M triangles, no holes along any batch
  seam, and 99.6% of triangles have empty space one voxel along their normal.
