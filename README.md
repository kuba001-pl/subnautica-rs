# subnautica-rs

An unofficial, from-scratch Rust reimplementation of the Subnautica engine for
desktop. It reads data from **your own installed copy** of Subnautica at
runtime. This repository contains no game files, and never will.

> Unofficial fan project, not affiliated with or endorsed by Unknown Worlds or
> Krafton. You need a legally owned copy of Subnautica. Developed with heavy
> use of AI coding agents; see `AGENTS.md` for the rules they follow.

Status: **early**. There is no game yet, only data tools. Architecture and
roadmap are in [docs/DESIGN.md](docs/DESIGN.md), and file format findings in
[docs/formats/](docs/formats/).

## What works

Tested on Windows 11 with game build 10 (Steam):

- Decoding all 5,416 terrain octree batch files (`.optoctrees`, ~1.1 GB):
  669,150 octrees, every one structurally validated, in under a second.
- Spatial layout of the octree data (child order, octree order, +Y up),
  confirmed from seam continuity.
- Reading the world index (`index.txt`): world, octree and batch dimensions.
- Turning terrain into meshes (surface nets) and exporting them as OBJ for
  Blender. Meshes of neighbouring batches join without holes.
- Reading Unity's containers: all 5,467 asset bundles and the player's
  `.assets` files (423,677 objects), matching UnityPy's listing exactly.
- Decoding the game's textures (matches UnityPy pixel for pixel on a 506-texture
  sample) and finding every terrain material (233 types, 211 textures).
- Reading where every world object is placed (`sn-inspect entities`): all
  4,581 placement files, 419,846 objects, each resolved to its prefab name
  (`prefabs.db`), in about a second.
- Loading prefabs through the Addressables catalog and decoding their meshes
  (`sn-inspect prefab`, exported as OBJ to `out/prefabs/`): all 1,369 placed
  prefabs and their 3,263 meshes, matching UnityPy on every vertex and
  triangle count.
- Reading the biome map and the water settings of all 145 biomes
  (`sn-inspect biomes`, `sn-inspect biomes --at X Y Z`).
- `sn-client`: a desktop window (Bevy) with a fly camera. Terrain streams in
  and out as you move, with four levels of detail out to 1.2 km, textured
  with the game's own terrain materials, blended between neighbouring
  materials the way the game does it (layers per chunk, soft ragged borders,
  cap/side transitions by slope), with the world's placed objects (coral,
  plants, rocks, wrecks; about 15,000 around the lifepod) streamed in with
  it, plus what the game's spawn slots fill with when a new world starts
  (resource outcrops, eggs, fragments; picked with a fixed seed,
  `--slot-seed`), seen through the game's own water fog model (colours and distances
  per biome) under the game's sun and sky light for a time of day
  (`--time`), with the game's water surface (simulated or baked waves, foam,
  reflection, refraction, sun glint; from above and below) under the
  game's sky (scattering, sun, clouds, planet, moon; day to night), lit
  the way the game lights surfaces (caustics, sunlight dimmed under water,
  the game's ambient; objects with their specular maps, glow maps with day
  and night strengths, and each biome's ambient settings; the point, spot and
directional lights placed objects carry, e.g. glowing coral, without their
shadows). Flying from the lifepod
  to the crater edge
  keeps memory under 1.6 GiB, at about 220 fps on average on an RTX 3080
  (about 100 fps in the dense start area).

## What doesn't work yet

The water clip map (no water inside the lifepod and bases,
shore foam); water settings per pixel (the
camera's are used everywhere); the Aurora, Lifepod 5, terrain grass; for
world objects: reflections of the biome
sky's cube map, cave/interior skies chosen by atmosphere volumes,
animation (waving plants), skinned meshes, creatures (including the
~100,000 the spawn slots would add); player, audio,
multiplayer.
Linux/macOS: not tested.

## Try it

You need Rust (stable) and, on Windows, Visual Studio with the C++ tools and
the Windows SDK.

```sh
# Point at the folder that contains Subnautica.exe
export SUBNAUTICA_DIR="D:/SteamLibrary/steamapps/common/Subnautica"   # PowerShell: $env:SUBNAUTICA_DIR = "..."

cargo run -p sn-inspect -- index            # world dimensions
cargo run -p sn-inspect -- octree 12 18 12  # statistics for one terrain batch
cargo run -p sn-inspect -- octree --all     # decode and validate every batch
cargo run -p sn-inspect -- orient 7 17 10   # show how octree data maps onto space
cargo run -p sn-inspect -- mesh 12 18 12 --radius 1   # Safe Shallows → out/*.obj
cargo run -p sn-inspect -- mesh 12 18 12 --radius 1 --lod 2   # coarser level of detail
cargo run -p sn-inspect -- voxel 0 -10 0    # what's at a world position, surfaces below
cargo run -p sn-inspect -- unity --types resources.assets   # what's inside a Unity file
cargo run -p sn-inspect -- unity --all      # parse every Unity file of the game
cargo run -p sn-inspect -- textures --census          # texture formats in the game
cargo run -p sn-inspect -- terrain-materials          # terrain type → material → textures

cargo run -p sn-client                      # fly around, starting at the lifepod
cargo run -p sn-client -- --view 2000       # see further
cargo run -p sn-client -- --flythrough 1700 -80 0   # automated test flight to the crater edge

cargo test --workspace                      # tests that don't need the game
cargo test -p sn-octree -- --ignored        # full-world test (needs SUBNAUTICA_DIR)
```

`sn-inspect` only reads the game folder; it never writes to it. Exports go to
`out/`, which is gitignored. They are derived from your game files: don't
share them.

## Layout

| Path | What |
|---|---|
| `crates/sn-octree` | Reader/writer for terrain octrees (pure, no dependencies) |
| `crates/sn-world` | World layout: `index.txt`, batch addressing |
| `crates/sn-mesh` | Surface nets mesher and mesh topology checks (pure, no dependencies) |
| `crates/sn-terrain` | Terrain batches → meshing fields → meshes (pure) |
| `crates/sn-unity` | Unity containers: UnityFS bundles, serialized files (pure) |
| `crates/sn-assets` | Finds Unity objects across bundles; terrain materials |
| `crates/sn-install` | Finds the game folder and reads its files (read-only) |
| `apps/sn-inspect` | Command-line inspector and validator |
| `apps/sn-client` | The desktop client (Bevy) |
| `docs/` | Design, roadmap, file format notes |

### Client controls

Hold the right mouse button (or press M) to look around. WASD moves, Q/E go
down/up, Shift is fast, and the mouse wheel changes speed.

## Licence

Our code is dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. Subnautica and its assets
belong to their owners and are not part of this project.

## Credits

Projects we learn from (reading, not copying): Nitrox,
Subnautica-TerrainPatcher, UnityPy, AssetStudio, OpenMW, hl2-rs, benilla,
gang-beasts-rust, and the AI Game Modding Guides.
