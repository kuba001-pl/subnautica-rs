//! `sn-inspect`: headless command-line tool to inspect and validate the data in
//! a Subnautica install. Reads the game folder, never writes to it.

mod density;
mod game;
mod mesh;
mod octree;
mod orient;

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
  mesh <X> <Y> <Z> [--radius <R>]
                         Mesh the terrain of a batch (or a cube of batches
                         R around it) and write an OBJ to out/
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
        ["mesh", x, y, z] => mesh::run(&game, parse_coord(x, y, z)?, 0),
        ["mesh", x, y, z, "--radius", r] => {
            let radius = r
                .parse()
                .map_err(|_| format!("radius {r:?} is not a whole number"))?;
            mesh::run(&game, parse_coord(x, y, z)?, radius)
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
