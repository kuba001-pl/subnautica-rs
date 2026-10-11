//! What multiplayer reads from an install (M10e, `docs/DESIGN.md` § M10
//! plan): the game build, the **rules fingerprint** that two machines
//! must share to play together, and a new world's values decided once by
//! the host (Lifepod 5's point, the Aurora's countdown). The server and
//! the client both call these, so they compute the same numbers.

use std::path::Path;
use std::sync::Arc;

use sn_install::GameData;
use sn_protocol::Fnv64;
use sn_world::SlotRng;

use crate::{Assets, ExploderCode, Result};

/// `StreamingAssets/__buildnumber.txt`, trimmed.
pub fn build_number(game: &GameData) -> Result<String> {
    let bytes = game
        .read_file(Path::new("StreamingAssets/__buildnumber.txt"))
        .map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes).map_err(|_| "__buildnumber.txt is not UTF-8")?;
    Ok(text.trim().to_owned())
}

/// The rules two machines must share, and what multiplayer takes from
/// them.
#[derive(Clone, Debug)]
pub struct Rules {
    pub build: String,
    /// FNV-1a 64 over the parts' names and hashes, in order.
    pub fingerprint: u64,
    /// Each part's own hash, for finding what differs.
    pub parts: Vec<(&'static str, u64)>,
    /// One tick, microseconds (`Time.fixedDeltaTime`).
    pub tick_us: u64,
    /// Above this speed between two states a move is implausible (server)
    /// or a teleport (client): twice the fastest speed the movement rules
    /// allow (metres per second).
    pub max_speed: f64,
    pub exploder: ExploderCode,
    pub player: sn_sim::player::PlayerParams,
    /// The item rules (M9d3d), shared by the server and the client.
    pub items: Arc<sn_sim::inventory::ItemRules>,
}

/// Reads the rules (a few seconds: the main scene and the code assembly).
///
/// The parts: the player's movement numbers (they hold the physics step,
/// which must be equal for shared ticks), oxygen and health, the look,
/// the raw `Balance/TechData` text (recipes, for crafting later), the
/// Aurora's explosion numbers (every client times it from the shared
/// clock), and the item rules (every machine replays item events with
/// them; M9d3d). Each is hashed through its `Debug` text (Rust prints floats
/// exactly and the same everywhere; these structs hold no hash maps) or
/// its bytes. Not in it yet: the cinematic clips (when M10h plays them on
/// remote bodies); the body's animator rules (remote bodies use the
/// owner's values, so a difference cannot desync).
pub fn rules(game: &GameData, assets: &Assets) -> Result<Rules> {
    let build = build_number(game)?;
    let data = crate::player_data(assets)?;
    let settings = crate::physics_settings(assets)?;
    let player = crate::player_params(&data, &settings);
    let code_bytes = crate::read_assembly(game)?;
    let code = crate::player_code(&code_bytes)?;
    let vitals = crate::vitals_params(&data, &code);
    let look = crate::look_params(&data, &code);
    let exploder = crate::exploder_code(&code_bytes)?;
    let tech_data = crate::resource_bytes(assets, "balance/techdata")?;
    let (items, _) = crate::item_rules(game, assets, &code_bytes)?;
    let text = |s: String| Fnv64::new().str(&s).finish();
    let parts = vec![
        ("player", text(format!("{player:?}"))),
        ("vitals", text(format!("{vitals:?}"))),
        ("look", text(format!("{look:?}"))),
        ("techdata", Fnv64::new().bytes(&tech_data).finish()),
        ("exploder", text(format!("{exploder:?}"))),
        ("items", text(format!("{items:?}"))),
    ];
    let mut h = Fnv64::new();
    for (name, part) in &parts {
        h.str(name).u64(*part);
    }
    let tick_us = (player.fixed_dt * 1e6).round() as u64;
    if tick_us == 0 {
        return Err(format!("physics step {} s", player.fixed_dt));
    }
    let max_speed = 2.0
        * [
            player.swim_forward,
            player.swim_vertical,
            player.walk_speed,
            player.max_fall_speed,
        ]
        .into_iter()
        .fold(0.0, f64::max);
    Ok(Rules {
        build,
        fingerprint: h.finish(),
        parts,
        tick_us,
        max_speed,
        exploder,
        player,
        items: Arc::new(items),
    })
}

/// Lifepod 5's point for a new world with this seed (`RandomStart` with
/// our seeded draw, as the client did alone before M10), and the draws it
/// took.
pub fn lifepod_point(assets: &Assets, seed: u64) -> Result<([f32; 3], usize)> {
    Ok(assets.start_map()?.random_start(seed))
}

/// The Aurora's countdown in game seconds after a new game starts
/// (`CrashedShipExploder.SetExplodeTime`'s draw, our seeded value).
pub fn aurora_countdown(exploder: &ExploderCode, seed: u64) -> f32 {
    let times = sn_sim::aurora::ExplosionTimes {
        range: exploder.range,
        day_seconds: exploder.day_seconds,
        sound_delay: exploder.sound_delay,
        fx_delay: exploder.fx_delay,
        swap_delay: exploder.swap_delay,
    };
    times.delay(SlotRng::new(seed, "CrashedShipExploder", 0).value())
}
