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
    println!("  object above the player by: {} m", d.oxygen_above_player);
    println!("LiveMixin:");
    println!("  health:                     {}", d.live_mixin.health);
    println!(
        "  startHealthPercent:         {}",
        d.live_mixin.start_health_percent
    );
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
        ("GroundMotor", &d.ground_motor.motor),
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
    let cam = &d.camera;
    println!(
        "MainCameraControl (on {:?}, {:?} from the player):",
        d.camera_node.0, d.camera_node.1
    );
    println!(
        "  minimumY {} maximumY {} (pitch limits, degrees); minimumX {} maximumX {} (unused)",
        cam.minimum_y, cam.maximum_y, cam.minimum_x, cam.maximum_x
    );
    println!(
        "  mouseLookEnabled {} skin {} camPDAZOffset {} stepAmount {} cameraTiltMod {} maxViewModelRotation {} maxViewModelMovement {}",
        cam.mouse_look_enabled,
        cam.skin,
        cam.cam_pda_z_offset,
        cam.step_amount,
        cam.camera_tilt_mod,
        cam.max_view_model_rotation,
        cam.max_view_model_movement
    );
    let code = sn_assets::player_code(&sn_assets::read_assembly(game)?)?;
    println!("From the game's code ({}):", sn_assets::GAME_ASSEMBLY);
    println!(
        "  OxygenManager.oxygenUnitsPerSecondSurface: {} (initialiser)",
        code.oxygen_per_second_surface
    );
    println!(
        "  GameInputSystem.defaultMouseSensitivity:   {} (const)",
        code.default_mouse_sensitivity
    );
    let look = sn_assets::look_params(&d, &code);
    println!(
        "  mouse look: {} degrees per count (sensitivity × 1.5 × 0.5)",
        look.degrees_per_count()
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
    let g = &d.ground_motor;
    println!("GroundMotor (its own fields):");
    println!(
        "  movement: max speed forward {} sideways {} backwards {}; max fall speed {}; slope speed curve {:?}",
        g.max_forward_speed,
        g.max_sideways_speed,
        g.max_backwards_speed,
        g.max_fall_speed,
        g.slope_speed_multiplier
            .keys
            .iter()
            .map(|k| (k.time, k.value))
            .collect::<Vec<_>>()
    );
    println!(
        "  jumping: enabled {} base height {} extra height {} perp {} steep perp {}",
        g.jump_enabled,
        g.jump_base_height,
        g.jump_extra_height,
        g.jump_perp_amount,
        g.jump_steep_perp_amount
    );
    println!(
        "  sliding: enabled {} speed {} sideways control {} speed control {}",
        g.sliding_enabled, g.sliding_speed, g.sliding_sideways_control, g.sliding_speed_control
    );
    println!(
        "  controller: step offset {} slope limit {}°; moving platform {} (transfer {}); fly cheat {}",
        g.step_offset,
        g.slope_limit,
        g.moving_platform_enabled,
        g.movement_transfer,
        g.fly_cheat_enabled
    );
    println!(
        "player layer {}; ocean level {} m",
        d.player_layer, d.ocean_level
    );
    let rb = &d.rigidbody;
    println!(
        "player Rigidbody: mass {} drag {} angular drag {} useGravity {} isKinematic {} interpolate {} constraints {} collision detection {}",
        rb.mass,
        rb.drag,
        rb.angular_drag,
        rb.use_gravity,
        rb.is_kinematic,
        rb.interpolate,
        rb.constraints,
        rb.collision_detection
    );

    let ps = sn_assets::physics_settings(&assets)?;
    let named = |l: u32| {
        let n = ps.tags.layers.get(l as usize).map_or("", String::as_str);
        format!("{l} {n:?}")
    };
    println!(
        "TimeManager: fixed timestep {} s, max allowed {} s, time scale {}",
        ps.time.fixed_timestep, ps.time.maximum_allowed_timestep, ps.time.time_scale
    );
    let ph = &ps.physics;
    println!(
        "PhysicsManager: gravity {:?}; contact offset {}; solver iterations {} / {}; queries hit backfaces {}, triggers {}; friction type {}",
        ph.gravity,
        ph.default_contact_offset,
        ph.default_solver_iterations,
        ph.default_solver_velocity_iterations,
        ph.queries_hit_backfaces,
        ph.queries_hit_triggers,
        ph.friction_type
    );
    let layers: Vec<String> = (0..32)
        .filter(|&l| !ps.tags.layers[l as usize].is_empty())
        .map(named)
        .collect();
    println!("layers: {}", layers.join(", "));
    let hits: Vec<String> = (0..32)
        .filter(|&l| ph.collides(d.player_layer, l))
        .map(named)
        .collect();
    let misses: Vec<String> = (0..32)
        .filter(|&l| !ph.collides(d.player_layer, l) && !ps.tags.layers[l as usize].is_empty())
        .map(named)
        .collect();
    println!(
        "the player's layer ({}) collides with: {}",
        named(d.player_layer),
        hits.join(", ")
    );
    println!("  and not with (named layers): {}", misses.join(", "));
    let mut scene = assets.scene("escapepod")?;
    scene.spawn_lightmapped_prefab();
    scene.place_escape_pod(&assets, [0.0; 3])?;
    for h in scene.dive_hatches(&assets)? {
        println!(
            "dive hatch {:?} (escape pod {}, enter only {}): at {:?}; outside exit {:?}; inside spawn {:?} (lifepod placed at the origin)",
            scene.roots[h.node.0].nodes[h.node.1].name,
            h.hatch.is_for_escape_pod,
            h.hatch.enter_only,
            h.at.position,
            h.outside_exit.position,
            h.inside_spawn.position
        );
    }

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

/// Where a ray from `o` along `d` first crosses the triangle, either side
/// (Möller–Trumbore): the distance and whether it hit the front, the side
/// the PhysX normal `(b − a) × (c − a)` points to.
fn ray_triangle(o: [f64; 3], d: [f64; 3], t: [[f32; 3]; 3]) -> Option<(f64, bool)> {
    let sub = |p: [f64; 3], q: [f64; 3]| [p[0] - q[0], p[1] - q[1], p[2] - q[2]];
    let dot = |p: [f64; 3], q: [f64; 3]| p[0] * q[0] + p[1] * q[1] + p[2] * q[2];
    let cross = |p: [f64; 3], q: [f64; 3]| {
        [
            p[1] * q[2] - p[2] * q[1],
            p[2] * q[0] - p[0] * q[2],
            p[0] * q[1] - p[1] * q[0],
        ]
    };
    let [a, b, c] = t.map(|p| p.map(f64::from));
    let (e1, e2) = (sub(b, a), sub(c, a));
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-12 {
        return None;
    }
    let s = sub(o, a);
    let u = dot(s, p) / det;
    let q = cross(s, e1);
    let v = dot(d, q) / det;
    let dist = dot(e2, q) / det;
    if u < 0.0 || v < 0.0 || u + v > 1.0 || dist <= 0.0 {
        return None;
    }
    Some((dist, dot(cross(e1, e2), d) < 0.0))
}

/// First hits of rays along `d` from the points `origins`: (front, back).
fn first_hits(tris: &[[[f32; 3]; 3]], origins: &[[f64; 3]], d: [f64; 3]) -> (usize, usize) {
    let (mut front, mut back) = (0, 0);
    for &o in origins {
        let best = tris
            .iter()
            .filter_map(|&t| ray_triangle(o, d, t))
            .min_by(|x, y| x.0.total_cmp(&y.0));
        match best {
            Some((_, true)) => front += 1,
            Some((_, false)) => back += 1,
            None => {}
        }
    }
    (front, back)
}

fn bounds(tris: &[[[f32; 3]; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in tris.iter().flatten() {
        for a in 0..3 {
            lo[a] = lo[a].min(f64::from(p[a]));
            hi[a] = hi[a].max(f64::from(p[a]));
        }
    }
    (lo, hi)
}

/// `prefab --winding`: which side of our collision triangles faces the
/// open space, by the PhysX convention (front = `(b − a) × (c − a)`).
/// Terrain: rays straight down from above a few shallow batches, the
/// first hit should be a front face (seabed seen from the water). Mesh colliders of
/// the placed prefabs: rays from outside their bounds along the six axes.
pub fn winding(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let mut loader = sn_assets::CollisionLoader::new(game, None)?;
    println!("terrain (rays down on a 1 m grid, first hit):");
    let (mut tf, mut tb) = (0, 0);
    // Shallow batches only: the rays must start in open water. Above a
    // deep batch they start inside rock and see the surface from behind.
    let points = [
        [-128.0, 0.0, -50.0],
        [300.0, -100.0, 300.0],
        [-300.0, -80.0, -400.0],
    ];
    for p in points {
        for c in loader.batches_around(p, 1.0) {
            let Some(tris) = loader.terrain(c)? else {
                continue;
            };
            if tris.is_empty() {
                continue;
            }
            let (lo, hi) = bounds(&tris);
            let mut origins = Vec::new();
            let mut x = lo[0] + 0.5;
            while x < hi[0] {
                let mut z = lo[2] + 0.5;
                while z < hi[2] {
                    origins.push([x, hi[1] + 1.0, z]);
                    z += 1.0;
                }
                x += 1.0;
            }
            // A grid per batch over its triangles; keep it cheap.
            let step = (origins.len() / 4000).max(1);
            let origins: Vec<_> = origins.into_iter().step_by(step).collect();
            let (f, b) = first_hits(&tris, &origins, [0.0, -1.0, 0.0]);
            println!(
                "  batch {c:?}: {} triangles, {} rays: front {f}, back {b}",
                tris.len(),
                origins.len()
            );
            tf += f;
            tb += b;
        }
    }
    println!("  total: front {tf}, back {tb}");

    let placed = crate::prefab::placed_positions(game)?;
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut meshes = sn_assets::ColliderMeshes::new();
    println!("mesh colliders of placed prefabs (rays from outside, 6 axes × 12 × 12):");
    let (mut mf, mut mb, mut prefabs) = (0, 0, 0);
    for (key, at) in &placed {
        let Ok(prefab) = assets.prefab(&catalog, key) else {
            continue;
        };
        let (list, _) = assets.prefab_colliders(&prefab, &mut meshes)?;
        let mut tris = Vec::new();
        for c in list.iter().filter(|c| !c.trigger) {
            if let sn_assets::WorldCollider::Triangles(t) = c.world(&sn_world::Transform::default())
            {
                tris.extend(t);
            }
        }
        if tris.is_empty() {
            continue;
        }
        prefabs += 1;
        let (lo, hi) = bounds(&tris);
        let (mut f, mut b) = (0, 0);
        for axis in 0..3 {
            let (j, k) = ((axis + 1) % 3, (axis + 2) % 3);
            for sign in [1.0, -1.0] {
                let mut d = [0.0; 3];
                d[axis] = sign;
                let mut origins = Vec::new();
                for u in 0..12 {
                    for v in 0..12 {
                        let mut o = [0.0; 3];
                        o[j] = lo[j] + (hi[j] - lo[j]) * (f64::from(u) + 0.5) / 12.0;
                        o[k] = lo[k] + (hi[k] - lo[k]) * (f64::from(v) + 0.5) / 12.0;
                        o[axis] = if sign > 0.0 {
                            lo[axis] - 1.0
                        } else {
                            hi[axis] + 1.0
                        };
                        origins.push(o);
                    }
                }
                let (pf, pb) = first_hits(&tris, &origins, d);
                f += pf;
                b += pb;
            }
        }
        println!(
            "  {key}: {} triangles, {} placements: front {f}, back {b}",
            tris.len(),
            at.len()
        );
        mf += f;
        mb += b;
    }
    println!("  total over {prefabs} prefabs: front {mf}, back {mb}");
    println!("time: {:.1} s", start.elapsed().as_secs_f64());
    Ok(ExitCode::SUCCESS)
}
