//! Gameplay data (Phase E, P0): `techdata`, `player`, `prefab --colliders`.
//! Tech types are printed as numbers; their names come from the game's
//! DLL in P1.

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::Assets;
use sn_install::GameData;
use sn_unity::{BreakableResource, Collider, ColliderShape, class_name};

use crate::Result;

const MONO_BEHAVIOUR: i32 = 114;

/// `techdata`: `Balance/TechData` and `EntTechData`, with checks.
pub fn techdata(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let data = sn_assets::tech_data(&assets)?;
    println!("TechData entries: {}", data.entries.len());
    println!("  without techType (skipped): {}", data.without_tech_type);
    println!("  duplicate tech types: {:?}", data.duplicates);
    println!("  keys the game does not read: {:?}", data.unknown_keys);

    let has =
        |f: &dyn Fn(&sn_assets::TechEntry) -> bool| data.entries.iter().filter(|e| f(e)).count();
    println!("  with key:");
    for (name, n) in [
        ("itemSize", has(&|e| e.item_size.is_some())),
        ("backgroundType", has(&|e| e.background_type.is_some())),
        ("equipmentType", has(&|e| e.equipment_type.is_some())),
        ("slotType", has(&|e| e.slot_type.is_some())),
        ("craftTime", has(&|e| e.craft_time.is_some())),
        ("craftAmount", has(&|e| e.craft_amount.is_some())),
        ("ingredients", has(&|e| !e.ingredients.is_empty())),
        ("linkedItems", has(&|e| !e.linked_items.is_empty())),
        ("processed", has(&|e| e.processed.is_some())),
        ("buildable", has(&|e| e.buildable.is_some())),
        ("soundPickup", has(&|e| e.sound_pickup.is_some())),
        ("soundDrop", has(&|e| e.sound_drop.is_some())),
        ("soundUse", has(&|e| e.sound_use.is_some())),
        ("harvestType", has(&|e| e.harvest_type.is_some())),
        ("harvestOutput", has(&|e| e.harvest_output.is_some())),
        (
            "harvestFinalCutBonus",
            has(&|e| e.harvest_final_cut_bonus.is_some()),
        ),
        ("maxCharge", has(&|e| e.max_charge.is_some())),
        ("energyCost", has(&|e| e.energy_cost.is_some())),
        ("poweredPrefab", has(&|e| e.powered_prefab.is_some())),
    ] {
        println!("    {name:<22}{n}");
    }

    let recipes: Vec<_> = data.recipes().collect();
    let ingredients: usize = recipes.iter().map(|r| r.ingredients.len()).sum();
    println!(
        "recipes: {} ({ingredients} ingredient rows, {} linked item rows)",
        recipes.len(),
        recipes.iter().map(|r| r.linked_items.len()).sum::<usize>()
    );
    let mut sizes: BTreeMap<[i32; 2], usize> = BTreeMap::new();
    for e in &data.entries {
        if let Some(s) = e.item_size {
            *sizes.entry(s).or_default() += 1;
        }
    }
    println!("item sizes given: {sizes:?}");
    let times: Vec<f32> = data.entries.iter().filter_map(|e| e.craft_time).collect();
    if let (Some(min), Some(max)) = (
        times.iter().copied().reduce(f32::min),
        times.iter().copied().reduce(f32::max),
    ) {
        println!("craft times: {} given, {min} s to {max} s", times.len());
    }
    let misses = data.ingredients_without_entry();
    let missing: BTreeSet<i32> = misses.iter().map(|&(_, i)| i).collect();
    println!(
        "ingredients without an entry of their own: {} rows, {} tech types {:?}",
        misses.len(),
        missing.len(),
        missing
    );

    let ent = sn_assets::ent_tech_data(&assets)?;
    let names: BTreeSet<&str> = ent.iter().map(|e| e.prefab_name.as_str()).collect();
    let types: BTreeSet<i32> = ent.iter().map(|e| e.tech_type).collect();
    let in_tech_data = types.iter().filter(|t| data.get(**t).is_some()).count();
    println!(
        "EntTechData entries: {} ({} distinct prefab names, {} distinct tech types, {in_tech_data} of them have a TechData entry)",
        ent.len(),
        names.len(),
        types.len()
    );
    println!(
        "  entries with tech type 0 (None): {}",
        ent.iter().filter(|e| e.tech_type == 0).count()
    );
    println!(
        "  names not lower case: {}",
        ent.iter()
            .filter(|e| e.prefab_name != e.prefab_name.to_lowercase())
            .count()
    );
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}

/// `player`: the player's numbers from the main scene, and `PDAData`.
pub fn player(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let d = sn_assets::player_data(&assets)?;
    let p = &d.player;
    println!("Player:");
    println!("  equipment slots:            {:?}", p.equipment_slots);
    println!("  movementSpeed:              {}", p.movement_speed);
    println!("  depthLevel:                 {}", p.depth_level);
    println!("  playerSphereRadius:         {}", p.player_sphere_radius);
    println!("  crushDepth:                 {}", p.crush_depth);
    println!("  suffocationTime:            {} s", p.suffocation_time);
    println!(
        "  suffocationRecoveryTime:    {} s",
        p.suffocation_recovery_time
    );
    println!(
        "  diveGoal:                   {:?} (goal type {}, delay {})",
        p.dive_goal.key, p.dive_goal.goal_type, p.dive_goal.delay
    );
    println!("Oxygen (isPlayer):");
    println!("  oxygenCapacity:             {}", d.oxygen.oxygen_capacity);
    println!("LiveMixin:");
    println!("  health:                     {}", d.live_mixin.health);
    let l = &d.live_mixin_data;
    println!("  data.maxHealth:             {}", l.max_health);
    println!("  data.minDamageForSound:     {}", l.min_damage_for_sound);
    println!(
        "  data.loopEffectBelowPercent:{}",
        l.loop_effect_below_percent
    );
    println!(
        "  data flags: destroyOnDeath {} weldable {} knifeable {} canResurrect {} passDamageDataOnDeath {} broadcastKillOnDeath {} invincibleInCreative {}",
        l.destroy_on_death,
        l.weldable,
        l.knifeable,
        l.can_resurrect,
        l.pass_damage_data_on_death,
        l.broadcast_kill_on_death,
        l.invincible_in_creative
    );
    for (name, m) in [
        ("UnderwaterMotor", &d.underwater_motor.motor),
        ("GroundMotor", &d.ground_motor),
    ] {
        println!("{name} (PlayerMotor fields):");
        println!(
            "  max speed forward {} backward {} strafe {} vertical {}; climb {}",
            m.forward_max_speed,
            m.backward_max_speed,
            m.strafe_max_speed,
            m.vertical_max_speed,
            m.climb_speed
        );
        println!(
            "  gravity {} underWaterGravity {} usingGravity {} canSwim {} canJump {} jumpHeight {} canControl {}",
            m.gravity,
            m.underwater_gravity,
            m.using_gravity,
            m.can_swim,
            m.can_jump,
            m.jump_height,
            m.can_control
        );
        println!(
            "  sprint modifier forward {} strafe {}",
            m.forward_sprint_modifier, m.strafe_sprint_modifier
        );
        println!(
            "  drag swim {} ground {} air {} ladder {}",
            m.swim_drag, m.ground_drag, m.air_drag, m.ladder_drag
        );
        println!(
            "  acceleration water {} ground {} air {} ladder {}",
            m.water_acceleration, m.ground_acceleration, m.air_acceleration, m.ladder_acceleration
        );
    }
    println!(
        "UnderwaterMotor: fastSwimMode {} playerSpeedModifier {}",
        d.underwater_motor.fast_swim_mode, d.underwater_motor.player_speed_modifier
    );
    let c = &d.controller;
    println!("PlayerController:");
    println!(
        "  standheight {} swimheight {} cameraOffset {} controllerRadius {} defaultSwimDrag {}",
        c.stand_height, c.swim_height, c.camera_offset, c.controller_radius, c.default_swim_drag
    );
    println!(
        "  swim max speed forward {} backward {} strafe {} vertical {}; water acceleration {}",
        c.swim_forward_max_speed,
        c.swim_backward_max_speed,
        c.swim_strafe_max_speed,
        c.swim_vertical_max_speed,
        c.swim_water_acceleration
    );
    println!(
        "  seaglide max speed forward {} backward {} strafe {} vertical {}; water acceleration {}; swim drag {}",
        c.seaglide_forward_max_speed,
        c.seaglide_backward_max_speed,
        c.seaglide_strafe_max_speed,
        c.seaglide_vertical_max_speed,
        c.seaglide_water_acceleration,
        c.seaglide_swim_drag
    );
    println!(
        "  walk/run max speed forward {} backward {} strafe {}",
        c.walk_run_forward_max_speed, c.walk_run_backward_max_speed, c.walk_run_strafe_max_speed
    );
    println!(
        "  camera minimum y: default {} walk/run {}",
        c.default_camera_minimum_y, c.walk_run_camera_minimum_y
    );

    let pda = &d.pda;
    println!("PDAData:");
    println!("  log entries:          {}", pda.log.len());
    println!("  encyclopedia entries: {}", pda.encyclopedia.len());
    println!(
        "  scanner entries:      {} ({} fragments, {} locked)",
        pda.scanner.len(),
        pda.scanner.iter().filter(|s| s.is_fragment).count(),
        pda.scanner.iter().filter(|s| s.locked).count()
    );
    println!(
        "  defaultTech:          {} {:?}",
        pda.default_tech.len(),
        pda.default_tech
    );
    println!(
        "  analysisTech:         {} ({} unlocks, {} story goals)",
        pda.analysis_tech.len(),
        pda.analysis_tech
            .iter()
            .map(|a| a.unlock_tech_types.len())
            .sum::<usize>(),
        pda.analysis_tech
            .iter()
            .map(|a| a.story_goals.len())
            .sum::<usize>()
    );
    println!(
        "  compoundTech:         {} ({} dependencies)",
        pda.compound_tech.len(),
        pda.compound_tech
            .iter()
            .map(|c| c.dependencies.len())
            .sum::<usize>()
    );
    let tech = sn_assets::tech_data(&assets)?;
    let missing: Vec<i32> = pda
        .default_tech
        .iter()
        .copied()
        .filter(|t| tech.get(*t).is_none())
        .collect();
    println!(
        "  defaultTech without a TechData entry: {} {missing:?}",
        missing.len()
    );
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}

#[derive(Default)]
struct ColliderCount {
    components: usize,
    /// Weighted by the prefab's placements in the world.
    placed: usize,
    on_active: usize,
    enabled: usize,
    triggers: usize,
    /// Only for mesh colliders.
    convex: usize,
    null_mesh: usize,
    prefabs: BTreeSet<String>,
}

#[derive(Default)]
struct Scripts {
    pickupable: BTreeSet<String>,
    pickupable_placed: usize,
    breakable: BTreeSet<String>,
    breakable_placed: usize,
    breakable_errors: Vec<String>,
    hits: BTreeMap<i32, usize>,
    drops: BTreeSet<i32>,
}

impl Scripts {
    fn print(&self, what: &str) {
        println!(
            "{what}: Pickupable on {} prefabs ({} placements); BreakableResource on {} prefabs ({} placements), hitsToBreak {:?}, drop tech types {:?}, layout errors {}",
            self.pickupable.len(),
            self.pickupable_placed,
            self.breakable.len(),
            self.breakable_placed,
            self.hits,
            self.drops,
            self.breakable_errors.len()
        );
        for e in self.breakable_errors.iter().take(10) {
            println!("  {e}");
        }
    }
}

/// `prefab --colliders`: collider components on every placed prefab, and
/// the `Pickupable` and `BreakableResource` scripts on the placed prefabs
/// and on those the spawn slots can fill in (the loot table).
pub fn colliders(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let placed = crate::prefab::placed_positions(game)?;
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let class_paths = game.read_prefab_database()?;
    let loot = sn_assets::loot_table(&assets)?;
    let mut loot_keys: BTreeSet<String> = BTreeSet::new();
    for (_, entries) in loot.distribution.biomes() {
        for e in entries {
            if let Some(path) = class_paths.get(&e.class_id) {
                loot_keys.insert(path.clone());
            }
        }
    }

    let mut counts: BTreeMap<i32, ColliderCount> = BTreeMap::new();
    let mut layout_errors: Vec<String> = Vec::new();
    let mut unreadable = 0;
    let mut without_collider = 0;
    let (mut in_placed, mut in_loot) = (Scripts::default(), Scripts::default());
    let jobs = placed.iter().map(|(k, p)| (k, p.len(), true)).chain(
        loot_keys
            .iter()
            .filter(|k| !placed.contains_key(*k))
            .map(|k| (k, 0, false)),
    );
    for (key, placements, is_placed) in jobs {
        let prefab = match assets.prefab(&catalog, key) {
            Ok(p) => p,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        let scripts = if is_placed {
            &mut in_placed
        } else {
            &mut in_loot
        };
        let mut any_collider = false;
        let (mut seen_pickupable, mut seen_breakable) = (false, false);
        for node in &prefab.nodes {
            for c in assets.node_components(node)? {
                let (info, data) = c.data()?;
                let big_endian = c.file.file().big_endian;
                if is_placed && Collider::is_collider(info.class_id) {
                    any_collider = true;
                    let n = counts.entry(info.class_id).or_default();
                    n.components += 1;
                    n.placed += placements;
                    n.prefabs.insert(key.clone());
                    n.on_active += usize::from(node.active);
                    match Collider::parse(info.class_id, data, big_endian) {
                        Ok(col) => {
                            n.enabled += usize::from(col.enabled);
                            n.triggers += usize::from(col.is_trigger);
                            if let ColliderShape::Mesh { convex, mesh, .. } = col.shape {
                                n.convex += usize::from(convex);
                                n.null_mesh += usize::from(mesh.is_null());
                            }
                        }
                        Err(e) => layout_errors.push(format!(
                            "{key} {} ({} bytes): {e}",
                            class_name(info.class_id).unwrap_or("?"),
                            data.len()
                        )),
                    }
                } else if info.class_id == MONO_BEHAVIOUR {
                    match assets.script_class(&c).as_deref() {
                        Some("Pickupable") => seen_pickupable = true,
                        Some("BreakableResource") => {
                            seen_breakable = true;
                            match BreakableResource::parse(data, big_endian) {
                                Ok(b) => {
                                    *scripts.hits.entry(b.hits_to_break).or_default() += 1;
                                    scripts.drops.insert(b.default_tech_type);
                                    scripts
                                        .drops
                                        .extend(b.prefab_list.iter().map(|r| r.tech_type));
                                }
                                Err(e) => scripts
                                    .breakable_errors
                                    .push(format!("{key} ({} bytes): {e}", data.len())),
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        without_collider += usize::from(is_placed && !any_collider);
        if seen_pickupable {
            scripts.pickupable.insert(key.clone());
            scripts.pickupable_placed += placements;
        }
        if seen_breakable {
            scripts.breakable.insert(key.clone());
            scripts.breakable_placed += placements;
        }
    }
    println!(
        "placed prefabs: {}; loot table prefabs: {}; unreadable: {unreadable}; placed without any collider: {without_collider}",
        placed.len(),
        loot_keys.len()
    );
    println!(
        "colliders on placed prefabs (class: components, on active nodes, enabled, triggers, prefabs, placed in the world):"
    );
    for (class, n) in &counts {
        print!(
            "  {:<16} {class:>3}: {:>6} {:>6} {:>6} {:>6} {:>5} {:>8}",
            class_name(*class).unwrap_or("?"),
            n.components,
            n.on_active,
            n.enabled,
            n.triggers,
            n.prefabs.len(),
            n.placed
        );
        if *class == sn_unity::MESH_COLLIDER {
            print!("  (convex {}, null mesh {})", n.convex, n.null_mesh);
        }
        println!();
    }
    println!("collider layout errors: {}", layout_errors.len());
    for e in layout_errors.iter().take(10) {
        println!("  {e}");
    }
    in_placed.print("placed prefabs");
    in_loot.print("loot table prefabs not placed");
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}
