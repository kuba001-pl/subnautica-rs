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
- `sn-client`: a desktop window (Bevy) with a fly camera. Terrain streams in
  and out as you move, with four levels of detail out to 1.2 km. Flying from
  the lifepod to the crater edge keeps memory under 0.9 GiB, at well over
  100 fps on an RTX 3080. Terrain uses false colours per material id.

## What doesn't work yet

Everything else: real terrain textures, water surface, models, world
objects, player, audio, multiplayer. Next up is M6: decoding textures and
real terrain materials.
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
