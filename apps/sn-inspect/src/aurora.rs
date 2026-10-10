//! `aurora` command (M7f4e): the Aurora's explosion clock, what the swap
//! switches, the volumes that may hide its exploded exterior, and the
//! renderers' shadow modes.

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{
    Assets, AuroraShow, Prefab, SHADOWS_OFF, SHADOWS_ON, SHADOWS_ONLY, SHADOWS_TWO_SIDED,
};
use sn_install::GameData;
use sn_sim::V3;
use sn_sim::aurora::{CullBox, ExplosionTimes};
use sn_world::SlotRng;

use crate::Result;

const MONO_BEHAVIOUR: i32 = 114;

/// Drawn renderers by `m_CastShadows`: off, on, two-sided, shadows only.
#[derive(Default)]
struct ShadowModes {
    renderers: [usize; 4],
    placements: [usize; 4],
    other: usize,
}

impl ShadowModes {
    fn add(&mut self, prefab: &Prefab, placements: usize) {
        for n in prefab.visible_nodes() {
            match usize::from(n.cast_shadows) {
                m @ 0..=3 => {
                    self.renderers[m] += 1;
                    self.placements[m] += placements;
                }
                _ => self.other += 1,
            }
        }
    }

    fn print(&self, what: &str) {
        let names = [
            (SHADOWS_OFF, "off"),
            (SHADOWS_ON, "on"),
            (SHADOWS_TWO_SIDED, "two-sided"),
            (SHADOWS_ONLY, "shadows only"),
        ];
        let line: Vec<String> = names
            .iter()
            .map(|&(m, name)| {
                let m = usize::from(m);
                format!(
                    "{name} {} ({} placed)",
                    self.renderers[m], self.placements[m]
                )
            })
            .collect();
        println!(
            "shadow modes of drawn renderers, {what}: {}; other values {}",
            line.join(", "),
            self.other
        );
    }
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;

    // The code's numbers.
    let code = sn_assets::exploder_code(&sn_assets::read_assembly(game)?)?;
    println!(
        "CrashedShipExploder (code): countdown Random.Range({}, {}) × {} s after the start; sound +{} s, explosion +{} s, swap +{} s",
        code.range.0,
        code.range.1,
        code.day_seconds,
        code.sound_delay,
        code.fx_delay,
        code.swap_delay
    );
    let times = ExplosionTimes {
        range: code.range,
        day_seconds: code.day_seconds,
        sound_delay: code.sound_delay,
        fx_delay: code.fx_delay,
        swap_delay: code.swap_delay,
    };
    let draws: Vec<String> = (1..=5)
        .map(|seed| {
            let d = times.delay(SlotRng::new(seed, "CrashedShipExploder", 0).value());
            format!("seed {seed}: {d:.0} s")
        })
        .collect();
    println!(
        "our seeded draws (countdown after the start): {}",
        draws.join(", ")
    );

    // The scene: what the swap switches, the groups, the manager.
    let mut scene = assets.scene("aurora")?;
    scene.spawn_lightmapped_prefab();
    let exploder = scene
        .exploder(&assets)?
        .ok_or("aurora: no CrashedShipExploder")?;
    let name = |pptr: sn_unity::PPtr| -> Result<String> {
        Ok(match assets.resolve(&scene.file, pptr)? {
            Some(o) => match scene.locate(&o) {
                Some((r, n)) => scene.roots[r].nodes[n].name.clone(),
                None => format!("(not in the scene: {})", pptr.path_id),
            },
            None => "(null)".into(),
        })
    };
    let names = |list: &[sn_unity::PPtr]| -> Result<Vec<String>> {
        list.iter().map(|p| name(*p)).collect()
    };
    println!(
        "disableOnExplosion {:?}; enableOnExplosion {:?}; explodedExterior {:?}",
        names(&exploder.disable_on_explosion)?,
        names(&exploder.enable_on_explosion)?,
        name(exploder.exploded_exterior)?
    );
    match scene.ship_exterior_cull_manager(&assets)? {
        Some(m) => println!(
            "ShipExteriorCullManager: runs, every {} frames",
            m.update_every_x_frames
        ),
        None => println!("ShipExteriorCullManager: none running"),
    }
    let groups = scene.aurora_groups(&assets)?;
    let mut by_show: BTreeMap<AuroraShow, (usize, usize)> = BTreeMap::new();
    let mut scene_shadows = ShadowModes::default();
    for g in &groups {
        let e = by_show.entry(g.show).or_default();
        e.0 += 1;
        e.1 += g.prefab.visible_nodes().count();
        scene_shadows.add(&g.prefab, 1);
    }
    for (show, (roots, drawn)) in &by_show {
        println!(
            "  shown intact {}, exploded {}, exploded exterior {}: {roots} parts, {drawn} drawn nodes",
            show.intact, show.exploded, show.exterior
        );
    }
    for exploded in [false, true] {
        let drawn: usize = groups
            .iter()
            .filter(|g| g.show.shown(exploded, false))
            .map(|g| g.prefab.visible_nodes().count())
            .sum();
        println!(
            "  drawn {}: {drawn} nodes",
            if exploded { "exploded" } else { "intact" }
        );
    }
    scene_shadows.print("the Aurora scene (both states)");

    // Every world prefab: cull volumes, DisableBeforeExplosion, shadows.
    let placed = crate::prefab::placed_transforms(game)?;
    let mut keys: BTreeSet<String> = game.read_prefab_database()?.into_values().collect();
    keys.extend(placed.keys().cloned());
    let mut unreadable = 0;
    let mut volumes: Vec<(String, CullBox)> = Vec::new();
    let (mut culls, mut registering, mut missing) = (0, 0, 0);
    let mut before_explosion: BTreeMap<String, usize> = BTreeMap::new();
    let mut shadows = ShadowModes::default();
    for key in &keys {
        let placements = placed.get(key).map_or(&[][..], Vec::as_slice);
        let Ok(prefab) = assets.prefab(&catalog, key) else {
            unreadable += 1;
            continue;
        };
        if !placements.is_empty() {
            shadows.add(&prefab, placements.len());
        }
        for cull in &prefab.exterior_culls {
            culls += 1;
            missing += cull.missing;
            if !cull.registers {
                continue;
            }
            registering += 1;
            for t in placements {
                for b in &cull.boxes {
                    let w = b.world(&prefab, t);
                    volumes.push((
                        key.clone(),
                        CullBox {
                            center: V3::from_f32(w.center),
                            axes: w.axes.map(V3::from_f32),
                            half: w.half.map(f64::from),
                        },
                    ));
                }
            }
        }
        for node in &prefab.nodes {
            for c in assets.node_components(node)? {
                let (info, _) = c.data()?;
                if info.class_id == MONO_BEHAVIOUR
                    && assets.script_class(&c).as_deref() == Some("DisableBeforeExplosion")
                {
                    *before_explosion.entry(key.clone()).or_default() += placements.len();
                }
            }
        }
    }
    println!(
        "ShipExteriorCull: {culls} scripts in the world prefabs, {registering} register; \
         {} volumes placed in the world; {missing} listed colliders not boxes of their prefab",
        volumes.len()
    );
    for (key, b) in &volumes {
        let c = b.center.to_f32().map(|v| (v * 100.0).round() / 100.0);
        let size = b.half.map(|h| (h * 200.0).round() / 100.0);
        println!(
            "  {key}: center {c:?}, size {size:?}, centre inside {}",
            b.contains(b.center)
        );
    }
    println!(
        "DisableBeforeExplosion: {} prefabs, {} placements",
        before_explosion.len(),
        before_explosion.values().sum::<usize>()
    );
    for (key, n) in &before_explosion {
        println!("  {key}: {n} placements");
    }
    shadows.print("placed prefabs");
    println!(
        "{} prefabs read, {unreadable} unreadable ({:.2} s)",
        keys.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(ExitCode::SUCCESS)
}
