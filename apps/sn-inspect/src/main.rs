//! `sn-inspect`: headless command-line tool to inspect and validate the data in
//! a Subnautica install. Reads the game folder, never writes to it.

mod density;
mod game;
mod mesh;
mod octree;
mod orient;
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
