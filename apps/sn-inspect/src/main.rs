//! `sn-inspect`: headless command-line tool to inspect and validate the data in
//! a Subnautica install. Reads the game folder, never writes to it.

mod anim;
mod aurora;
mod bars;
mod biomes;
mod body_run;
mod bots;
mod canvas;
mod code;
mod collision;
mod compass;
mod coop;
mod death;
mod density;
mod depth;
mod dive;
mod draw;
mod entities;
mod fonts;
mod game;
mod gameplay;
mod grass;
mod icon_mesh;
mod icons;
mod instantiate;
mod inventory_tab;
mod live;
mod lods;
mod materials;
mod mesh;
mod meshes;
mod messages;
mod octree;
mod orient;
mod notifications;
mod pda_tabs;
mod pickup;
mod pickups;
mod png;
mod popup;
mod prefab;
mod pulse;
mod rects;
mod reticle;
mod scene;
mod skinned;
mod slots;
mod sprites_tmp;
mod start;
mod swim;
mod texts;
mod tooltip;
mod texture;
mod ui;
mod unity;
mod voxel;
mod walk;
mod walk_pda;

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
  entities --find <TEXT> Every placed object whose prefab path contains TEXT
  grass <X> <Y> <Z> [--seed <N>]
                         The terrain grass of a batch at full resolution:
                         tufts, vertices, triangles per grass type
  slots [<X> <Y> <Z>] [--seed <N>]
                         Spawn slots of every batch (or one) and what they
                         fill with for world seed N (default 1): counts per
                         biome, spawned prefabs, a hash of the result
  prefab <KEY>           A prefab (e.g. WorldEntities/…/X.prefab): hierarchy,
                         meshes, materials; writes out/prefabs/<name>.obj
  prefab <KEY> --props   Every property of the prefab's drawn materials
  prefab --placed [--oracle]
                         Load every prefab placed in the world and decode
                         its meshes (--oracle: out/mesh-check-rust.txt)
  prefab --skinned       Every skinned mesh of the placed prefabs and the
                         escape pod: skin checks, skinned LOD 0 bounds vs
                         the static LOD 1
  prefab --shapes        M7f4d: blend shapes of the placed prefabs and the
                         escape pod: stored weights, which ones animators
                         drive, bounds with and without the stored weights
  prefab --lights        The Light components of every placed prefab, and
                         how many lights the world's placements hold
  prefab --placeholders  Placed prefabs with PrefabPlaceholders and whether
                         the world's saved objects already hold what they spawn
  prefab --colliders     Collider components (box, sphere, capsule, mesh) on
                         every placed prefab, and the Pickupable and
                         BreakableResource scripts
  prefab --winding       Which side of our collision triangles (terrain,
                         mesh colliders) faces open space, by the PhysX
                         convention
  prefab --materials     Every material drawn by the placed prefabs and the
                         startup scenes: uses, what our object shader takes
                         from it, its shader and the shader's first pass
  scene <NAME> [--tree <DEPTH> | --script <CLASS> | --cinematics]
                         A scene bundle (aurora, escapepod, main, …): object
                         counts, top-level objects (with their hierarchy to
                         DEPTH), what is drawn, scripts (with the bytes of
                         each MonoBehaviour of script CLASS)
  anim <KEY | scene:NAME> [--states | --play <S>]
                         M7f4: the animators of a prefab or of a scene's
                         top-level objects: controller layers, parameters
                         (--states: every state, its motion, transitions),
                         clips, and their bindings found in the hierarchy;
                         --play: run each from its defaults for S seconds
                         (transitions, NaNs, how far things move, cost)
  anim --placed          Animators of every placed prefab: placements,
                         controllers, culling modes, cost per update
  scene --lifepod [--seed <N>]
                         Lifepod 5 in a new game with world seed N (default
                         1): start point, player spawn, spawned modules
  swim [--seed <N>] [--seconds <S>]
                         M9a: a scripted swim from the lifepod (world seed
                         N, default 1) to the nearest Kelp Forest through
                         our collision, at most S simulated seconds
                         (default 600): contacts, penetrations, cost per step
  walk [--seed <N>]      M9b: a scripted run with the player's rules: spawn
                         in the lifepod, walk to the hatch, leave, swim
                         away 10 s and back, enter; positions, motor
                         changes, speeds next to the values read
  walk --pda [--seed <N>]
                         M9u8a2: the PDA opened, held and closed in the
                         lifepod and under water: its times, the arms'
                         states, the screen anchor's pose from the eye
  pickup [--seed <N>]    M9d6b: the items headless, with the player's rules,
                         an in-process server and client: the supplies
                         out of the locker, out of the pod, the nearest
                         pickup, the nearest outcrop broken and its
                         resource, a drop picked up again, a death's
                         drops; each step's counts checked
  dive [--seed <N>]      M9c: oxygen, suffocation and respawn: out of the
                         lifepod, to the seabed until the player dies,
                         respawn in the pod, a dive cut short by surfacing;
                         oxygen each second next to the values read
  aurora                 M7f4e: the Aurora's countdown rule (from the DLL),
                         what its swap switches, its parts by state, the
                         ShipExteriorCull volumes in the world, and the
                         drawn renderers' shadow modes
  lods                   M7f4f: quality levels (lodBias), the camera's field
                         of view, LOD groups of the placed prefabs and
                         scenes, and the level each placed group shows
                         at 5–800 m
  scene --startup        The scenes the game loads at start and what the
                         spawned ones draw in a new game
  terrain-materials      Terrain block types → materials → textures, checked
                         against the type ids used by the octrees
  terrain-materials --props
                         Every property of every terrain material
  terrain-materials --region <X> <Y> <Z> <R>
                         Which types form the surface around a batch
  code [--trees]         What the game keeps only in its code (its DLL):
                         every method body decoded, TechType names, the
                         crafting menus (--trees: every node), TechData's
                         defaults; checked against Balance/TechData
  code --il <TYPE> <METHOD> [<PARAMS>]
                         One method's IL with tokens resolved (TYPE as
                         ASSEMBLY:TYPE reads Managed/ASSEMBLY.dll; PARAMS:
                         the overload with that many parameters;
                         METHOD#N: the Nth overload)
  code --enum <TYPE>     An enum's names and values (Outer/Inner for a
                         nested one; also for --il)
  code --nested <TYPE>   The types nested in a type (coroutines, enums)
                         and their methods
  code --fields <TYPE>   A type's fields in declaration order: declared
                         type, public/static/readonly/const/[NonSerialized],
                         the type it extends, the interfaces it declares
                         and its methods (ASSEMBLY:TYPE as for --il)
  code --implements <INTERFACE> [<ASSEMBLY>]
                         The types that declare an interface (by name, any
                         namespace), with the type each extends
  code --extends <BASE> [<ASSEMBLY>]
                         The types whose base type is named BASE
  techdata               Recipes and item data (Balance/TechData) and the
                         prefab → tech type map (EntTechData), with checks
  player                 The player's numbers from the main scene (oxygen,
                         health, motors, speeds) and PDAData (unlocks)
  pickups                M9d1: item data: every placed and slot prefab that
                         can be picked up (tech type by the game's rule,
                         sizes, food), outcrops and what they break into,
                         the player's luck, the lifepod's locker and
                         supplies, the rules in the code, the item text
  player --body          M9g1: the player's body: suit models by the
                         equipment rule, drawn renderers and their shaders,
                         the head, the camera's nodes, ArmsController, the
                         animator's parameters by who sets them
  bots <ADDR:PORT> [--count <N>] [--seconds <S>]
                         M10e: N bot players (default 8) against a running
                         sn-server for S seconds (default 60): joins with
                         this install's build and rules, scripted paths,
                         every drawn tick compared with its owner's state
  coop <ADDR:PORT> [--count <N,...>] [--presets <P,...>] [--seconds <S>]
                         M10f: bots with the player's own rules and
                         collision (recorded first, then replayed) against
                         a running sn-server, each bot's link impaired by
                         the presets (default lan,internet,bad) for each
                         count (default 2,4); every drawn tick and every
                         event compared with its owner's; --seed N must be
                         the server's --lifepod-seed (default 1)
  ui                     M9u0: the game's UI, headless: where uGUI.prefab
                         and the PDA's screen come from, their nodes and
                         components, every canvas and canvas scaler, the
                         hosts of the scripts M9u ports, the sprite
                         atlases and the fonts; counts checked against
                         the plan's
  ui --scripts [--all]   M9u1: every MonoBehaviour of the UI's files and
                         the startup scenes (--all: of every bundle)
                         read by the layout built from the game's
                         assemblies; misses by script, skipped fields
  ui --sprites [--icons] M9u2: every Sprite and SpriteAtlas of the game
                         parsed; the atlases SpriteManager loads, how
                         their sprites are packed; every tech type's
                         icon by the game's rule (--icons: the items'
                         icons as PNGs in out/ui/icons/)
  ui --fonts [--atlas]   M9u3: every TMP font asset's face metrics from its
                         font file against the stored ones; the 2 stored
                         glyphs remade (metrics, places, distance texels);
                         the English text's characters in each text's
                         font chain, glyph cost (--atlas: the made atlases
                         as PNGs in out/ui/fonts/)
  ui --rects             M9u4a: the in-scope UI trees laid out by our port
                         of uGUI's layout system; every value it drives
                         compared with the stored one
  ui --meshes            M9u4b: the meshes of every Image and RawImage of
                         the in-scope trees on the laid-out rects; counts
                         per kind, vertices inside rects, UVs inside sprites
  ui --texts             M9u4c: the in-scope TMP texts' settings and the
                         rich-text tags of the stored texts and English
  ui --draw              M9u4d, M9u5a: every in-scope tree's draw list
                         (order, canvas space, alpha, clipping, materials,
                         text vertices), built twice from scratch; the
                         hashes must be equal
  ui --canvas            M9u5b: each canvas scaler's placement at several
                         screens, its corners projected through the UI
                         camera, and the trees laid out at those sizes
  ui --live              M9u6a: uGUI read once and drawn for many states:
                         each kind of script change, frames with every HUD
                         text changed (draw time), the start state again
  ui --bars              M9u6b1: the HUD's four bars driven by their rules
                         through a scripted dive and drawn each frame; each
                         second each bar's value and text against a replay
  ui --start             M9u6s: what uGUI draws at start, by screen
  ui --pulse             M9u6b2: the HUD bars' pulses, punches and turn
                         through a scripted timeline, against a replay
  ui --depth             M9u6c1: the depth readout through a scripted dive
                         (PDA, inactive HUD, compass flag), every frame
                         against a replay
  ui --compass           M9u6c2: the compass ring and labels: the labels'
                         stored places, 36 headings and an equip timeline
                         against a replay
  ui --death             M9u6e1: the death vignette, the respawn screen and
                         the out-of-oxygen overlay through a scripted
                         drowning, against a replay
  ui --instantiate       M9u7a: UI objects made at run time: a stored
                         subtree copied and moved, four message copies and
                         SetAsLastSibling, checked against the stored draws
  ui --messages          M9u7b: ErrorMessage, the HUD's text messages,
                         through a scripted timeline against a replay
  ui --icon-mesh         M9u7c1: item icons made by code (uGUI_ItemIcon,
                         uGUI_Icon, UI/Icon) drawn from their sprites' meshes
  ui --icons             M9u7c2: uGUI_IconNotifier, the picked-up icons,
                         through a scripted queue against a replay
  ui --popup             M9u7d1: uGUI_PopupNotification, the unlock pop-up,
                         through scripted OnAnalyze entries against a replay
  ui --pda               M9u8a4b: the PDA screen's tab bar (uGUI_Toolbar),
                         its icons, OpenTab and clicks against the rule
  ui --inventory         M9u8b1: the PDA's inventory tab without icons:
                         containers, the doll's slots, labels
  ui --notifications     M9u8b5a: NotificationLabel and NotificationManager on
                         the PDA screen (the toolbar's count, an icon's +).
  ui --tooltip           M9u8b3b: uGUI_Tooltip and TooltipFactory's item
                         texts
  ui --layout <CLASS> [<NAME>]
                         M9u1: a script's layout and, for its first object
                         (named NAME), where each field starts and its value
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
        ["coop", server, options @ ..] => {
            let (mut counts, mut presets, mut seconds) = (
                vec![2usize, 4],
                vec!["lan".to_string(), "internet".into(), "bad".into()],
                60.0f64,
            );
            for pair in options.chunks(2) {
                let [flag, value] = pair else {
                    return Err(format!("coop: {} needs a value", pair[0]));
                };
                match *flag {
                    "--count" => {
                        counts = value
                            .split(',')
                            .map(|v| v.parse().map_err(|_| "--count takes numbers"))
                            .collect::<std::result::Result<_, _>>()?
                    }
                    "--presets" => presets = value.split(',').map(str::to_string).collect(),
                    "--seconds" => {
                        seconds = value.parse().map_err(|_| "--seconds needs a number")?
                    }
                    _ => return Err(format!("unknown coop option {flag}")),
                }
            }
            coop::run(&game, server, &counts, &presets, seconds, seed)
        }
        ["bots", server, options @ ..] => {
            let (mut count, mut seconds) = (8usize, 60.0f64);
            for pair in options.chunks(2) {
                let [flag, value] = pair else {
                    return Err(format!("bots: {} needs a value", pair[0]));
                };
                match *flag {
                    "--count" => count = value.parse().map_err(|_| "--count needs a number")?,
                    "--seconds" => {
                        seconds = value.parse().map_err(|_| "--seconds needs a number")?
                    }
                    _ => return Err(format!("unknown bots option {flag}")),
                }
            }
            bots::run(&game, server, count, seconds).map(|()| ExitCode::SUCCESS)
        }
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
        ["entities", "--find", text] => entities::find(&game, text),
        ["entities", x, y, z] => entities::one(&game, parse_coord(x, y, z)?),
        ["slots"] => slots::run(&game, None, seed),
        ["grass", x, y, z] => grass::run(&game, parse_coord(x, y, z)?, seed),
        ["slots", x, y, z] => slots::run(&game, Some(parse_coord(x, y, z)?), seed),
        ["prefab", "--placed"] => prefab::placed(&game, false),
        ["prefab", "--lights"] => prefab::lights(&game),
        ["prefab", "--colliders"] => gameplay::colliders(&game),
        ["prefab", "--winding"] => gameplay::winding(&game),
        ["prefab", "--materials"] => prefab::materials(&game),
        ["prefab", "--placeholders"] => prefab::placeholders(&game),
        ["prefab", "--skinned"] => skinned::run(&game),
        ["prefab", "--shapes"] => skinned::shapes(&game),
        ["prefab", "--placed", "--oracle"] => prefab::placed(&game, true),
        ["prefab", key, "--props"] => prefab::props(&game, key),
        ["prefab", key] => prefab::one(&game, key),
        ["anim", "--placed"] => anim::placed(&game),
        ["anim", target] => anim::run(&game, target, false, None),
        ["anim", target, "--states"] => anim::run(&game, target, true, None),
        ["anim", target, "--play", seconds] => anim::run(
            &game,
            target,
            false,
            Some(
                seconds
                    .parse()
                    .map_err(|_| format!("{seconds:?} is not a number"))?,
            ),
        ),
        ["aurora"] => aurora::run(&game),
        ["lods"] => lods::run(&game),
        ["scene", "--startup"] => scene::startup(&game),
        ["scene", "--lifepod"] => scene::lifepod(&game, seed),
        ["walk"] => walk::run(&game, seed),
        ["walk", "--pda"] => walk_pda::run(&game, seed),
        ["dive"] => dive::run(&game, seed),
        ["pickup"] => pickup::run(&game, seed),
        ["swim"] => swim::run(&game, seed, 600.0),
        ["swim", "--seconds", s] => swim::run(
            &game,
            seed,
            s.parse().map_err(|_| format!("{s:?} is not a number"))?,
        ),
        ["scene", name] => scene::run(&game, name, None, None),
        ["scene", name, "--cinematics"] => scene::cinematics(&game, name),
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
        ["code"] => code::run(&game, false),
        ["code", "--trees"] => code::run(&game, true),
        ["code", "--il", ty, method] => code::il(&game, ty, method, None),
        ["code", "--il", ty, method, params] => {
            let n = params
                .parse()
                .map_err(|_| format!("parameter count {params:?} is not a number"))?;
            code::il(&game, ty, method, Some(n))
        }
        ["code", "--enum", ty] => code::enum_values(&game, ty),
        ["code", "--nested", ty] => code::nested(&game, ty),
        ["code", "--fields", ty] => code::fields(&game, ty),
        ["code", "--extends", name] => code::extends(&game, name, None),
        ["code", "--extends", name, asm] => code::extends(&game, name, Some(asm)),
        ["code", "--implements", name] => code::implements(&game, name, None),
        ["code", "--implements", name, asm] => code::implements(&game, name, Some(asm)),
        ["techdata"] => gameplay::techdata(&game),
        ["player"] => gameplay::player(&game),
        ["pickups"] => pickups::run(&game),
        ["ui"] => ui::run(&game),
        ["ui", "--scripts"] => ui::scripts(&game, false),
        ["ui", "--scripts", "--all"] => ui::scripts(&game, true),
        ["ui", "--rects"] => rects::run(&game),
        ["ui", "--meshes"] => meshes::run(&game),
        ["ui", "--texts"] => texts::run(&game),
        ["ui", "--draw"] => draw::run(&game),
        ["ui", "--canvas"] => canvas::run(&game),
        ["ui", "--live"] => live::run(&game),
        ["ui", "--bars"] => bars::run(&game),
        ["ui", "--start"] => start::run(&game),
        ["ui", "--pulse"] => pulse::run(&game),
        ["ui", "--depth"] => depth::run(&game),
        ["ui", "--compass"] => compass::run(&game),
        ["ui", "--tmp-sprites"] => sprites_tmp::run(&game),
        ["ui", "--reticle"] => reticle::run(&game),
        ["ui", "--death"] => death::run(&game),
        ["ui", "--instantiate"] => instantiate::run(&game),
        ["ui", "--messages"] => messages::run(&game),
        ["ui", "--icon-mesh"] => icon_mesh::run(&game),
        ["ui", "--icons"] => icons::run(&game),
        ["ui", "--popup"] => popup::run(&game),
        ["ui", "--pda"] => pda_tabs::run(&game),
        ["ui", "--notifications"] => notifications::run(&game),
        ["ui", "--inventory"] => inventory_tab::run(&game),
        ["ui", "--tooltip"] => tooltip::run(&game),
        ["ui", "--fonts"] => fonts::run(&game, false),
        ["ui", "--fonts", "--atlas"] => fonts::run(&game, true),
        ["ui", "--sprites"] => ui::sprites(&game, false),
        ["ui", "--sprites", "--icons"] => ui::sprites(&game, true),
        ["ui", "--layout", class] => ui::layout(&game, class, None),
        ["ui", "--layout", class, name] => ui::layout(&game, class, Some(name)),
        ["player", "--body"] => gameplay::player_body(&game),
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
