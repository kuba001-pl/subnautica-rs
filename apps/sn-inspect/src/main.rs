//! `sn-inspect`: headless command-line tool to inspect and validate the data in
//! a Subnautica install. Reads the game folder, never writes to it.

mod biomes;
mod density;
mod entities;
mod game;
mod grass;
mod materials;
mod mesh;
mod octree;
mod orient;
mod prefab;
mod scene;
mod skinned;
mod slots;
mod texture;
mod unity;
mod voxel;

use std::path::PathBuf;
use std::process::ExitCode;

use sn_world::BatchCoord;

use sn_install::GameData;

const USAGE: &str = "\
Usage: sn-inspect [--game-dir <PATH>] <COMMAND>

The game folder is the one containing Subnautica.exe. Instead of --game-dir
you can set the SUBNAUTICA_DIR environment variable.

Commands:
  index                  World dimensions and per-batch values from index.txt
  octree <X> <Y> <Z>     Decode one terrain batch and print statistics
  octree --all           Decode and validate every terrain batch
  density <X> <Y> <Z>    Relate density bytes to empty/solid and surface voxels
  mesh <X> <Y> <Z> [--radius <R>] [--lod <L>]
                         Mesh the terrain of a batch (or a cube of batches
                         R around it) at level of detail L (0 = full, 3 =
                         every 8th voxel) and write an OBJ to out/
  voxel <X> <Y> <Z>      What is at a Unity world position; terrain surfaces
                         in that column
  unity <FILE>...        List the contents of Unity files (bundles, *.assets);
                         paths relative to Subnautica_Data or absolute
  unity --types <FILE>   The same, with class names
  unity --all            Parse every Unity file of the game and print totals
  textures <FILE>...     List the Texture2D objects in Unity files
  textures --pixels <FILE>...
                         The same, plus a CRC-32 of each decoded image
  textures --census      Count texture formats over the whole game
  biomes                 Biome map, batch override biomes, and the water
                         settings of every biome
  biomes --at <X> <Y> <Z>
                         The biome and water settings at a Unity world position
  entities <X> <Y> <Z>   A batch's saved objects (batch objects, baked cells)
                         with prefab paths and world positions
  entities --all         Parse every object cache file; totals and checks
  grass <X> <Y> <Z> [--seed <N>]
                         The terrain grass of a batch at full resolution:
                         tufts, vertices, triangles per grass type
  slots [<X> <Y> <Z>] [--seed <N>]
                         Spawn slots of every batch (or one) and what they
                         fill with for world seed N (default 1): counts per
                         biome, spawned prefabs, a hash of the result
  prefab <KEY>           A prefab (e.g. WorldEntities/…/X.prefab): hierarchy,
                         meshes, materials; writes out/prefabs/<name>.obj
  prefab --placed [--oracle]
                         Load every prefab placed in the world and decode
                         its meshes (--oracle: out/mesh-check-rust.txt)
  prefab --skinned       Every skinned mesh of the placed prefabs and the
                         escape pod: skin checks, skinned LOD 0 bounds vs
                         the static LOD 1
  prefab --lights        The Light components of every placed prefab, and
                         how many lights the world's placements hold
  scene <NAME> [--tree <DEPTH> | --script <CLASS>]
                         A scene bundle (aurora, escapepod, main, …): object
                         counts, top-level objects (with their hierarchy to
                         DEPTH), what is drawn, scripts (with the bytes of
                         each MonoBehaviour of script CLASS)
  scene --lifepod [--seed <N>]
                         Lifepod 5 in a new game with world seed N (default
                         1): start point, player spawn, spawned modules
  scene --startup        The scenes the game loads at start and what the
                         spawned ones draw in a new game
  terrain-materials      Terrain block types → materials → textures, checked
                         against the type ids used by the octrees
  terrain-materials --props
                         Every property of every terrain material
  terrain-materials --region <X> <Y> <Z> <R>
                         Which types form the surface around a batch
  orient <X> <Y> <Z>     Score candidate child/octree orders using a batch and
                         its +X/+Y/+Z neighbours (default batch: 12 18 12)
";

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: Vec<String>) -> Result<ExitCode> {
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let mut game_dir = None;
    if let Some(i) = args.iter().position(|a| a == "--game-dir") {
        if i + 1 >= args.len() {
            return Err("--game-dir needs a path".into());
        }
        game_dir = Some(PathBuf::from(args.remove(i + 1)));
        args.remove(i);
    }
    let mut seed = 1;
    if let Some(i) = args.iter().position(|a| a == "--seed") {
        seed = args
            .get(i + 1)
            .and_then(|v| v.parse().ok())
            .ok_or("--seed needs a number")?;
        args.drain(i..i + 2);
    }
    let game = GameData::locate(game_dir)?;
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["index"] => game::print_index(&game),
        ["octree", "--all"] => octree::all(&game),
        ["octree", x, y, z] => octree::one(&game, parse_coord(x, y, z)?),
        ["density", x, y, z] => density::run(&game, parse_coord(x, y, z)?),
        ["mesh", x, y, z, options @ ..] => {
            let mut radius = 0;
            let mut lod = 0;
            let mut it = options.iter();
            while let Some(flag) = it.next() {
                let value = it.next().ok_or(format!("{flag} needs a value"))?;
                let value: i64 = value
                    .parse()
                    .map_err(|_| format!("{flag} value {value:?} is not a whole number"))?;
                match *flag {
                    "--radius" => radius = value as i32,
                    "--lod" => lod = value as u32,
                    _ => return Err(format!("unknown mesh option {flag}")),
                }
            }
            mesh::run(&game, parse_coord(x, y, z)?, radius, lod)
        }
        ["voxel", x, y, z] => {
            let parse = |s: &str| {
                s.parse::<f32>()
                    .map_err(|_| format!("{s:?} is not a number"))
            };
            voxel::run(&game, [parse(x)?, parse(y)?, parse(z)?])
        }
        ["unity", "--all"] => unity::all(&game),
        ["unity", "--types", path] => unity::types(&game, path),
        ["unity", paths @ ..] if !paths.is_empty() => unity::list(&game, paths),
        ["textures", "--census"] => texture::census(&game),
        ["textures", "--pixels", paths @ ..] if !paths.is_empty() => {
            texture::list(&game, paths, true)
        }
        ["textures", paths @ ..] if !paths.is_empty() => texture::list(&game, paths, false),
        ["biomes"] => biomes::run(&game, None),
        ["biomes", "--at", x, y, z] => {
            let n = |s: &str| {
                s.parse::<f32>()
                    .map_err(|_| format!("{s:?} is not a number"))
            };
            biomes::run(&game, Some([n(x)?, n(y)?, n(z)?]))
        }
        ["entities", "--all"] => entities::all(&game),
        ["entities", x, y, z] => entities::one(&game, parse_coord(x, y, z)?),
        ["slots"] => slots::run(&game, None, seed),
        ["grass", x, y, z] => grass::run(&game, parse_coord(x, y, z)?, seed),
        ["slots", x, y, z] => slots::run(&game, Some(parse_coord(x, y, z)?), seed),
        ["prefab", "--placed"] => prefab::placed(&game, false),
        ["prefab", "--lights"] => prefab::lights(&game),
        ["prefab", "--skinned"] => skinned::run(&game),
        ["prefab", "--placed", "--oracle"] => prefab::placed(&game, true),
        ["prefab", key] => prefab::one(&game, key),
        ["scene", "--startup"] => scene::startup(&game),
        ["scene", "--lifepod"] => scene::lifepod(&game, seed),
        ["scene", name] => scene::run(&game, name, None, None),
        ["scene", name, "--script", class] => scene::run(&game, name, None, Some(class)),
        ["scene", name, "--tree", depth] => {
            let depth = depth
                .parse()
                .map_err(|_| format!("depth {depth:?} is not a whole number"))?;
            scene::run(&game, name, Some(depth), None)
        }
        ["terrain-materials"] => materials::run(&game),
        ["terrain-materials", "--props"] => materials::props(&game),
        ["terrain-materials", "--region", x, y, z, r] => {
            let radius = r
                .parse()
                .map_err(|_| format!("radius {r:?} is not a whole number"))?;
            materials::region(&game, parse_coord(x, y, z)?, radius)
        }
        ["orient"] => orient::run(&game, BatchCoord::new(12, 18, 12)),
        ["orient", x, y, z] => orient::run(&game, parse_coord(x, y, z)?),
        _ => Err(format!(
            "unrecognised arguments: {}\n\n{USAGE}",
            args.join(" ")
        )),
    }
}

fn parse_coord(x: &str, y: &str, z: &str) -> Result<BatchCoord> {
    let parse = |s: &str| {
        s.parse::<i32>()
            .map_err(|_| format!("batch coordinate {s:?} is not a whole number"))
    };
    Ok(BatchCoord::new(parse(x)?, parse(y)?, parse(z)?))
}
