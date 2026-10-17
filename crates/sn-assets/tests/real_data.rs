//! Against the player's own install. Opt-in:
//! `SUBNAUTICA_DIR=… cargo test -p sn-assets -- --ignored`
//!
//! Expected numbers were measured on game build 10 (see MODLOG, M6).

use std::path::PathBuf;

use sn_assets::{Assets, marmo_skies, terrain_materials};
use sn_install::GameData;

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn atmosphere_all_placed_prefabs_and_colliders() {
    use sn_sim::{V3, atmosphere::contains, collide::Shape};
    use sn_world::{Transform, VOXEL_WORLD_OFFSET};
    use std::collections::BTreeMap;
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layout = assemblies
        .layouts()
        .unwrap()
        .script("Assembly-CSharp", "", "AtmosphereVolume")
        .unwrap();
    let paths = game.read_prefab_database().unwrap();
    let mut placements: BTreeMap<String, Vec<Transform>> = BTreeMap::new();
    let mut collect = |tree: &sn_world::ObjectTree| {
        let (world, missing) = tree.world_transforms();
        assert_eq!(missing, 0);
        for (o, at) in tree.objects.iter().zip(world) {
            if let Some(path) = paths.get(&o.class_id) {
                placements.entry(path.clone()).or_default().push(at);
            }
        }
    };
    for batch in game.object_batches().unwrap().0 {
        let Some(mut tree) = game.read_batch_objects(batch).unwrap() else {
            continue;
        };
        let corner = [batch.x, batch.y, batch.z];
        for root in tree.objects.iter_mut().filter(|o| o.parent.is_none()) {
            root.transform.position =
                [0, 1, 2].map(|i| corner[i] as f32 * 160.0 - VOXEL_WORLD_OFFSET[i]);
        }
        collect(&tree);
    }
    for batch in game.cell_batches().unwrap().0 {
        let Some(cells) = game.read_batch_cells(batch).unwrap() else {
            continue;
        };
        for cell in &cells.cells {
            if let Some(tree) = &cell.objects {
                collect(tree);
            }
        }
    }
    let mut families = 0;
    let mut counts = [0usize; 3];
    let mut supported = 0;
    let mut errors = BTreeMap::<String, usize>::new();
    for (path, instances) in &placements {
        let prefab = assets.prefab(&catalog, path).unwrap();
        let volumes = assets.prefab_atmospheres(&prefab, &layout).unwrap();
        if !volumes.is_empty() {
            families += 1;
        }
        for at in instances {
            for template in &volumes {
                match template.placed(at, 0) {
                    Ok(volume) => {
                        let (index, center) = match volume.shape {
                            Shape::Box { center, .. } => (0, center),
                            Shape::Sphere { center, .. } => (1, center),
                            Shape::Capsule { a, b, .. } => (2, (a + b) * 0.5),
                            Shape::Triangle(_) => panic!("not a volume primitive"),
                        };
                        assert!(contains(&volume.shape, center));
                        assert!(!contains(&volume.shape, V3::new(1e9, 1e9, 1e9)));
                        counts[index] += 1;
                        supported += 1;
                        if instances.len() < 4 {
                            println!(
                                "volume probe: center={:?}, priority={}, shape={index}, biome={}",
                                center.to_f32(),
                                volume.priority,
                                volume.biome
                            );
                        }
                    }
                    Err(e) => *errors.entry(e).or_default() += 1,
                }
            }
        }
        assets.trim_cache(512 << 20);
    }
    println!(
        "atmosphere census: {} placed prefab families checked, {families} volume families, {supported} supported placements, box/sphere/capsule={counts:?}, unsupported={errors:?}",
        placements.len()
    );
    assert!(families > 0 && supported > 0);
    assert!(
        errors.is_empty(),
        "installed volume transform gaps must be resolved or explicitly scoped"
    );
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn optimized_key_terminals_bind_skin_and_select_distinct_materials() {
    use sn_anim::{Animator, Program, SlotKind};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let atmosphere_layout = assemblies
        .layouts()
        .unwrap()
        .script("Assembly-CSharp", "", "AtmosphereVolume")
        .unwrap();
    let mut cache = HashMap::new();
    let mut materials = HashSet::new();
    for color in ["Purple", "Orange", "Blue"] {
        let path =
            format!("WorldEntities/Environment/Precursor/Precursor_{color}KeyTerminal.prefab");
        let mut prefab = assets.prefab(&catalog, &path).unwrap();
        let (root, animator) = prefab
            .nodes
            .iter()
            .enumerate()
            .find_map(|(i, n)| n.animator.as_ref().map(|a| (i, a)))
            .unwrap();
        let set = assets
            .animation_set(animator.controller.as_ref().unwrap(), &mut cache)
            .unwrap();
        let program = Arc::new(Program::new(Arc::new(set.controller), &set.clips));
        let binding = prefab.bind_animator(root, &program, &|_| Vec::new());
        // Reconstructed hidden bones have synthetic identities, not asset files.
        assets
            .prefab_atmospheres(&prefab, &atmosphere_layout)
            .unwrap();
        assert_eq!(binding.missing, 0, "{color}");
        let mut runtime = Animator::new(program.clone(), binding.defaults.clone());
        for _ in 0..60 {
            runtime.update(1.0 / 60.0);
        }
        assert!(runtime.pose().iter().all(|v| v.is_finite()));
        for (slot, node) in program.slots.iter().zip(&binding.nodes) {
            let Some(node) = *node else { continue };
            let mut local = prefab.nodes[node].local;
            match slot.kind {
                SlotKind::Position => local
                    .position
                    .copy_from_slice(&runtime.pose()[slot.offset..slot.offset + 3]),
                SlotKind::Rotation => local
                    .rotation
                    .copy_from_slice(&runtime.pose()[slot.offset..slot.offset + 4]),
                SlotKind::Scale => local
                    .scale
                    .copy_from_slice(&runtime.pose()[slot.offset..slot.offset + 3]),
                _ => continue,
            }
            prefab.set_local(node, local);
        }
        let (body, node) = prefab.visible().find(|(_, n)| n.skinned).unwrap();
        assert_eq!(node.bones.len(), 7);
        assert!(node.bones.iter().all(Option::is_some));
        let (mesh, geometry) = assets.mesh(node.mesh.as_ref().unwrap()).unwrap();
        assert_eq!(mesh.bone_name_hashes.len(), 7);
        let skin = prefab.skinned_geometry(body, &mesh, &geometry).unwrap();
        assert_eq!(skin.positions.len(), 1903);
        assert!(skin.positions.iter().flatten().all(|v| v.is_finite()));
        let (_, face) = prefab.visible().find(|(_, n)| !n.skinned).unwrap();
        materials.insert(face.materials[0].as_ref().unwrap().key());
        let centre = face.in_prefab.position;
        let distance = skin
            .positions
            .iter()
            .map(|p| {
                p.iter()
                    .zip(centre)
                    .map(|(p, c)| (p - c).powi(2))
                    .sum::<f32>()
                    .sqrt()
            })
            .fold(f32::INFINITY, f32::min);
        eprintln!(
            "{color}: bound slots {}, bones 7, glyph/body nearest-vertex distance {distance:.6} m",
            program.slots.len()
        );
        assert!(
            distance < 0.25,
            "glyph detached: {color} distance {distance}"
        );
    }
    assert_eq!(materials.len(), 3);
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn terrain_materials_resolve_and_decode() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let materials = terrain_materials(&assets).unwrap();
    assert!(materials.warnings.is_empty(), "{:?}", materials.warnings);
    assert_eq!(materials.scene_types, 56);
    assert_eq!(materials.prefab_types, 233);
    assert_eq!(materials.prefabs_without_id, 10);
    assert_eq!(materials.conflicts, vec![53, 54, 101, 102]);
    assert_eq!(materials.types.iter().flatten().count(), 233);
    // 183 colour and normal maps + 28 specular/illumination (SIG) maps of
    // the terrain, + the grass materials' own textures (below).
    let mut terrain_textures = std::collections::HashSet::new();
    for m in materials.types.iter().flatten() {
        for layer in [&m.cap, &m.side] {
            for t in [&layer.albedo, &layer.normal, &layer.sig]
                .into_iter()
                .flatten()
            {
                terrain_textures.insert(std::sync::Arc::as_ptr(t));
            }
        }
    }
    assert_eq!(terrain_textures.len(), 211);

    // Grass: 60 types, each with a mesh, a colour map and a cutoff.
    let grass: Vec<_> = materials
        .types
        .iter()
        .flatten()
        .filter_map(|m| m.grass.as_ref().map(|g| (m.type_id, g)))
        .collect();
    assert_eq!(grass.len(), 60);
    let mut grass_textures = std::collections::HashSet::new();
    for (id, g) in &grass {
        assert!(g.look.albedo.is_some(), "type {id}");
        assert!(
            !g.positions.is_empty() && g.indices.len() % 3 == 0,
            "type {id}"
        );
        // 0 for the few coral decos drawn with object materials (opaque).
        assert!((0.0..1.0).contains(&g.look.cutoff), "type {id}");
        let l = &g.look;
        for t in [&l.albedo, &l.normal, &l.sig, &l.mask, &l.spec, &l.illum]
            .into_iter()
            .flatten()
        {
            if !terrain_textures.contains(&std::sync::Arc::as_ptr(t)) {
                grass_textures.insert(std::sync::Arc::as_ptr(t));
            }
        }
    }
    assert_eq!(grass_textures.len(), 76);
    assert_eq!(materials.texture_count, 211 + 76);
    // Mesh sizes as UnityPy reads them.
    let size = |id: usize| {
        let g = materials.types[id]
            .as_ref()
            .unwrap()
            .grass
            .as_ref()
            .unwrap();
        (g.mesh_name.as_str(), g.positions.len(), g.indices.len())
    };
    assert_eq!(size(43), ("coral_reef_grass_03", 18, 18));
    assert_eq!(size(100), ("Coral_reef_small_deco_07", 125, 516));
    assert_eq!(materials.grass_types().iter().flatten().count(), 60);
    // Shaders, told apart by properties (names checked with UnityPy).
    use sn_assets::GrassShader;
    let of = |shader: GrassShader| {
        grass
            .iter()
            .filter(|(_, g)| g.look.shader == shader)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
    };
    assert_eq!(of(GrassShader::Marmoset), vec![52, 76, 83, 251]);
    assert_eq!(of(GrassShader::NoiseyWave), vec![243]);
    assert_eq!(of(GrassShader::Sig { sig: false }), vec![32]);
    assert_eq!(
        of(GrassShader::Sig { sig: true }),
        vec![7, 33, 50, 51, 100, 252]
    );
    assert_eq!(of(GrassShader::TerrainGrass).len(), 48);
    assert!(of(GrassShader::Unknown).is_empty());
    let cap_side = materials
        .types
        .iter()
        .flatten()
        .filter(|m| m.cap_side)
        .count();
    assert_eq!(cap_side, 119);
    assert!(materials.types[0].is_none(), "type 0 is empty space");

    // Every texture's pixel data is complete enough to decode its base level.
    for m in materials.types.iter().flatten() {
        for layer in [&m.cap, &m.side] {
            let t = layer
                .albedo
                .as_ref()
                .unwrap_or_else(|| panic!("{} lacks a texture", m.name));
            let rgba = t.texture.decode_rgba(&t.data).unwrap();
            assert_eq!(
                rgba.len(),
                (t.texture.width * t.texture.height * 4) as usize
            );
            assert!(
                layer.scale > 0.0 && layer.scale < 10.0,
                "{}: scale {}",
                m.name,
                layer.scale
            );
        }
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn prefabs_resolve_and_meshes_decode() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    assert_eq!((catalog.key_count(), catalog.entry_count()), (38483, 22406));

    // (prefab, nodes, visible meshes as (vertices, triangles)); counts match
    // UnityPy (see MODLOG, M7b). The hallway's mesh is a compressed one.
    type Case<'a> = (&'a str, usize, &'a [(usize, usize)]);
    let cases: [Case; 3] = [
        (
            "WorldEntities/Doodads/Coral_reef/Coral_reef_tree_mushrooms_connector_01.prefab",
            3,
            &[(213, 378)],
        ),
        (
            "WorldEntities/Doodads/Coral_reef/Coral_reef_purple_mushrooms_01_04.prefab",
            4,
            &[(395, 304)],
        ),
        (
            "WorldEntities/Doodads/Debris/Aurora/Rooms/CrashedShip_T_hallway.prefab",
            34,
            &[(43691, 49560)],
        ),
    ];
    for (key, nodes, expected) in cases {
        let prefab = assets.prefab(&catalog, key).unwrap();
        assert_eq!(prefab.nodes.len(), nodes, "{key}");
        let mut found = Vec::new();
        for node in prefab.visible_nodes() {
            let (_, g) = assets.mesh(node.mesh.as_ref().unwrap()).unwrap();
            let triangles: usize = g.sub_meshes.iter().map(|s| s.len() / 3).sum();
            found.push((g.positions.len(), triangles));
        }
        assert!(
            expected.iter().all(|e| found.contains(e)),
            "{key}: {found:?}"
        );
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn water_biomes_and_biome_map() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let water = sn_assets::water_biomes(&assets).unwrap();
    assert_eq!(water.biomes.len(), 145);
    assert_eq!((water.texture_size, water.upsampled_size), (8, 32));
    assert_eq!(water.region_bounds, 64.0);
    let volume = sn_assets::water_volume(&assets).unwrap();
    assert_eq!(
        (
            volume.water_transmission,
            volume.sun_attenuation,
            volume.sun_light_amount
        ),
        (0.7, 0.25, 100.0)
    );
    assert_eq!(volume.scattering_phase, -0.3);
    let shallows = water
        .biomes
        .iter()
        .find(|b| b.name == "safeShallows")
        .unwrap();
    assert_eq!(shallows.settings.absorption, [125.0, 20.0, 4.0]);

    let (map, names) = game.read_biome_map().unwrap();
    assert_eq!((map.width, map.height, names.len()), (1024, 1024, 19));
    // The lifepod (Unity 0, -10, 0) is in the Safe Shallows.
    let v = sn_world::world_to_voxel([0.0, -10.0, 0.0]);
    let i = map.index_at(v[0] as i32, v[2] as i32, 4160).unwrap();
    assert_eq!(names[usize::from(i)], "safeShallows");
    // Every named biome in the map except the open ocean has settings.
    for &c in &map.cells {
        let name = &names[usize::from(c)];
        assert!(
            name == "void"
                || water
                    .biomes
                    .iter()
                    .any(|b| b.name.eq_ignore_ascii_case(name)),
            "{name}"
        );
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn sky_system_values() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let (manager, light) = sn_assets::sky(&assets).unwrap();
    // Values as UnityPy reads them with layouts generated from the game's
    // assembly (see MODLOG, M8b).
    assert_eq!(
        (
            manager.timeline,
            manager.sun_direction,
            manager.sun_max_angle
        ),
        (8.2, -141.0, 65.0)
    );
    assert_eq!((manager.exposure, manager.sky_fog_density), (0.66, 0.0002));
    assert_eq!(manager.wavelengths, [680.0, 550.0, 440.0]);
    assert_eq!(manager.sky_fog_color.color_keys, 4);
    assert_eq!(
        manager.sky_fog_color.color_times[..4],
        [16769, 19661, 45875, 49151]
    );
    assert_eq!(
        (
            light.sun_intensity,
            light.moon_intensity,
            light.ambient_light
        ),
        (1.37, 0.5, 0.35)
    );
    assert_eq!(
        (light.light_color.mode, light.light_color.color_keys),
        (0, 7)
    );
    assert_eq!(
        light.light_color.color_times[..7],
        [15073, 17039, 20971, 32768, 44564, 48496, 50469]
    );
    assert!((light.light_color.keys[3][1] - 0.9686).abs() < 1e-4);
    // The mean sky colour (read past the planet and cloud fields).
    assert_eq!(
        manager.mean_sky_color.color_times,
        [14456, 17252, 19082, 20971, 44564, 47120, 48496, 51052]
    );
    assert_eq!(
        (
            manager.mean_sky_color.color_keys,
            manager.mean_sky_color.alpha_keys
        ),
        (8, 2)
    );
    assert!((manager.mean_sky_color.keys[0][2] - 0.0902).abs() < 1e-4);
    assert!((manager.mean_sky_color.keys[1][1] - 0.1608).abs() < 1e-4);
    // The sky dome's fields (read to the object's end).
    let d = &manager.dome;
    assert_eq!(
        (d.planet_radius, d.planet_zenith, d.planet_distance),
        (3500.0, 59.0, 10000.0)
    );
    assert_eq!((d.planet_orbit_speed, d.clouds_rotate_speed), (1000.0, 1.0));
    assert_eq!(
        (d.sun_color_multiplier, d.sky_color_multiplier),
        (3.72, 1.87)
    );
    assert_eq!(
        (d.clouds_attenuation, d.clouds_alpha_saturation),
        (1.05, 2.5)
    );
    assert_eq!(d.night_sky, 2);
    assert_eq!((d.moon_size, d.star_intensity), (0.2, 1.0));
    assert!(d.linear_space && !d.skybox_hdr);
    assert!(!d.clouds_texture.is_null() && !d.planet_texture.is_null());
    assert!(!d.sun_burst_texture.is_null() && !d.moon_texture.is_null());
    // uSky's star catalogue: 9110 stars × 6 floats.
    let stars = sn_assets::resource_bytes(&assets, "starsdata").unwrap();
    assert_eq!(stars.len(), 9110 * 6 * 4);
    let t = sn_assets::sky_textures(&assets, &manager).unwrap();
    for (what, tex) in [
        ("planet", &t.planet),
        ("sun burst", &t.sun_burst),
        ("moon", &t.moon),
        ("clouds", &t.clouds),
    ] {
        let x = &tex.texture;
        eprintln!(
            "{what}: {} {}x{} {:?} mips {} wrap {:?} colour space {}",
            x.name,
            x.width,
            x.height,
            x.texture_format(),
            x.mip_count,
            x.wrap,
            x.color_space
        );
        assert!(x.width > 0 && !tex.data.is_empty());
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn water_surface_values() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let w = sn_assets::water_surface(&assets).unwrap();
    let s = &w.surface;
    // Values as UnityPy reads them with a layout generated from the game's
    // assembly (see MODLOG, M8c1).
    assert_eq!((s.patch_length, s.sequence_length), (2000.0, 5.0));
    assert!(!s.cubic_interpolation && !s.enable_reflection);
    assert_eq!(
        (s.sun_reflection_gloss, s.sun_reflection_amount),
        (400.0, 1.0)
    );
    assert_eq!(
        (s.refraction_index, s.under_water_refraction_index),
        (1.33, 1.1)
    );
    assert_eq!(s.under_water_refraction_depth_scale, 0.01);
    assert_eq!((s.foam_rate, s.foam_decay, s.foam_scale), (3.0, 5.0, 6.0));
    assert_eq!(s.displacement_texture_foam_amount_multiplier, 5.0);
    assert_eq!(s.wave_height_thickness_scale, 0.16);
    assert_eq!(s.under_water_brightness_curve.keys.len(), 3);
    assert_eq!(s.under_water_brightness_curve.post_infinity, 2);
    assert!((s.under_water_brightness_curve.evaluate(46.932_28) - 3.979_84).abs() < 1e-4);
    assert_eq!(
        (s.num_caustics_frames, s.caustics_frames_per_second),
        (64, 25)
    );
    assert_eq!(s.caustics_size, 2.5);
    let waves = &s.waves;
    assert_eq!(
        (waves.wind_angle, waves.wind_speed, waves.wind_dependency),
        (45.0, 600.0, 0.07)
    );
    assert_eq!(
        (
            waves.choppy_scale,
            waves.phillips_amplitude,
            waves.min_wave_size
        ),
        (1.3, 0.35, 0.01)
    );
    // 64 frames, in order, 256² RGBA32, stored linear; foam textures sRGB.
    assert_eq!(w.frames.len(), 64);
    for (i, f) in w.frames.iter().enumerate() {
        assert_eq!(f.texture.name, format!("WaterFrame{i:02}"));
        assert_eq!((f.texture.width, f.texture.height), (256, 256));
        assert_eq!(f.texture.texture_format(), sn_unity::TextureFormat::Rgba32);
        assert_eq!(f.texture.color_space, 0);
    }
    assert_eq!(w.foam.texture.name, "WaterFoam");
    assert_eq!(w.foam_mask.texture.name, "FoamBubbles");
    let caustics = sn_assets::water_caustics(&assets, s.num_caustics_frames as usize).unwrap();
    let c = &caustics[0].texture;
    eprintln!(
        "caustics: {} frames, {} {}x{} {:?} mips {} wrap {:?} colour space {}",
        caustics.len(),
        c.name,
        c.width,
        c.height,
        c.texture_format(),
        c.mip_count,
        c.wrap,
        c.color_space
    );
    for (i, f) in caustics.iter().enumerate() {
        assert_eq!(f.texture.name, format!("WaterCaustics{i:02}"));
        assert_eq!((f.texture.width, f.texture.height), (c.width, c.height));
    }
    assert_eq!(
        (w.foam.texture.width, w.foam_mask.texture.width),
        (1024, 512)
    );
    assert_eq!(
        (w.foam.texture.color_space, w.foam_mask.texture.color_space),
        (1, 1)
    );
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn marmo_skies_and_sky_appliers() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let skies = marmo_skies(&assets).unwrap();
    eprintln!(
        "{} skies, {} biomes, global {:?}",
        skies.skies.len(),
        skies.biomes.len(),
        skies.global
    );
    for s in &skies.skies {
        eprintln!(
            "  {} {:?} exposure {:?}",
            s.name,
            s.rotation,
            s.sky.exposure()
        );
    }
    // Values read with UnityPy on the dev machine (see MODLOG, M8c7).
    assert_eq!((skies.skies.len(), skies.biomes.len()), (37, 145));
    let global = &skies.skies[skies.global.unwrap()];
    assert_eq!(global.name, "SkySafeShallows");
    assert_eq!(skies.for_biome(Some("SAFESHALLOWS")), skies.global);
    assert_eq!(skies.for_biome(None), skies.global);
    assert!(global.sky.affected_by_day_night && global.sky.outdoors);
    let e = global.sky.exposure();
    assert!(
        (e[1] - 0.65).abs() < 1e-6 && (e[3] - 1.0).abs() < 1e-6,
        "{e:?}"
    );
    let reef = &skies.skies[skies.for_biome(Some("grandReef")).unwrap()];
    assert_eq!(reef.sky.master_intensity, 3.0);
    let cave = &skies.skies[skies.for_biome(Some("safeShallows_Cave")).unwrap()];
    assert!(!cave.sky.affected_by_day_night);

    let catalog = assets.catalog().unwrap();
    let key = "WorldEntities/Doodads/Coral_reef/Coral_reef_purple_mushrooms_01_04.prefab";
    let prefab = assets.prefab(&catalog, key).unwrap();
    let anchors: Vec<Option<i32>> = prefab.visible_nodes().map(|n| n.sky_applier).collect();
    eprintln!("{key}: {anchors:?}");
    assert_eq!(anchors, vec![Some(0)]);
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn prefab_lights() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    // Values read with UnityPy's Light layout on the dev machine (M8e1).
    let key = "WorldEntities/Doodads/Coral_reef_Light/Coral_reef_Kelp_blood_03_Light.prefab";
    let prefab = assets.prefab(&catalog, key).unwrap();
    let lights: Vec<&sn_unity::Light> = prefab.nodes.iter().flat_map(|n| &n.lights).collect();
    assert_eq!(lights.len(), 4);
    for l in lights {
        assert!(l.enabled && l.is_realtime());
        assert_eq!(l.kind, sn_unity::LightKind::Point);
        assert_eq!((l.intensity, l.range), (1.5, 15.0));
        assert!((l.color[0] - 0.745_098_05).abs() < 1e-6 && l.color[2] == 1.0);
        assert_eq!(l.shadows, sn_unity::ShadowKind::None);
        assert_eq!(l.culling_mask, u32::MAX);
    }

    // The safe shallows' atmosphere volume: a directional "Bounce" light
    // driven by a DayNightLight (curve values read on the dev machine, M8e1).
    let key = "WorldEntities/Atmosphere/SafeShallows/Normal.prefab";
    let prefab = assets.prefab(&catalog, key).unwrap();
    let node = prefab.nodes.iter().find(|n| n.name == "Bounce").unwrap();
    assert_eq!(node.lights.len(), 1);
    assert_eq!(node.lights[0].kind, sn_unity::LightKind::Directional);
    let d = node.day_night_light.as_ref().unwrap();
    assert!((d.intensity.evaluate(0.5) - 0.11).abs() < 1e-3);
    assert!((d.intensity.evaluate(0.0) - 0.01).abs() < 1e-3);
    assert!((d.color_g.evaluate(0.5) - 0.96).abs() < 1e-3);
    assert_eq!((d.replace_fraction, d.fade), (0.0, 1.0));
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn volumetric_light_glow() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let key = "WorldEntities/Doodads/Precursor/Cache/IonCrystalPedestal_Cache 1.prefab";
    let prefab = assets.prefab(&catalog, key).unwrap();
    let glows: Vec<_> = prefab
        .nodes
        .iter()
        .filter_map(|n| n.volumetric_light.as_ref().map(|g| (n, g)))
        .collect();
    // Values read on the dev machine (2026-10-09): a green point light.
    assert_eq!(glows.len(), 1);
    let (node, glow) = glows[0];
    assert!(node.renderer_enabled && glow.sets_block && glow.updates);
    let s = &glow.script;
    assert_eq!(
        (s.intensity, s.start_offset, s.start_fallof),
        (0.35, 0.0, 0.0)
    );
    assert_eq!((s.near_clip, s.soft_edges, s.light_type), (1.0, 2.0, 2));
    let l = glow.light.as_ref().unwrap();
    assert!(l.enabled && l.intensity == 3.0);
    assert!((l.color[0] - 0.419_117_6).abs() < 1e-6 && l.color[1] == 1.0);
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn default_spot_cookie() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let soft = sn_assets::builtin_texture(&assets, "Soft").unwrap();
    let t = &soft.texture;
    let rgba = t.decode_rgba(&soft.data).unwrap();
    let alpha = |x: usize, y: usize| rgba[(y * t.width as usize + x) * 4 + 3];
    let row: Vec<u8> = (0..t.width as usize)
        .step_by(8)
        .map(|x| alpha(x, 64))
        .collect();
    eprintln!(
        "Soft: {}x{} format {} wrap {:?} filter {} alpha along the middle row {row:?}, corner {}",
        t.width,
        t.height,
        t.format,
        t.wrap,
        t.filter_mode,
        alpha(0, 0)
    );
    assert_eq!((t.width, t.height, t.format), (128, 128, 1));
    // Clamped, transparent outside the disc, opaque in the middle.
    assert_eq!(
        (t.wrap[0], t.wrap[1], alpha(0, 0), alpha(64, 64)),
        (1, 1, 0, 255)
    );
    assert_eq!(&row[..4], &[0, 76, 216, 255]);
    // Radially symmetric (our spot lights rely on it): the alpha at a
    // distance from the centre is the same in every direction.
    for r in [10, 30, 50, 60] {
        // The disc centre is at 63.5 (texel centres): 64 + r and 63 − r are
        // the same distance from it.
        let a = [
            alpha(64 + r, 64),
            alpha(63 - r, 64),
            alpha(64, 64 + r),
            alpha(64, 63 - r),
        ];
        let (lo, hi) = (a.iter().min().unwrap(), a.iter().max().unwrap());
        assert!(hi - lo <= 8, "r {r}: {a:?}");
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn spawn_slot_tables_and_fill() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let table = sn_assets::loot_table(&assets).unwrap();
    assert_eq!((table.prefabs, table.rows), (190, 1295));
    assert_eq!(table.distribution.biome_count(), 352);
    assert_eq!(table.biome_names.len(), 352);
    let infos = sn_assets::entity_infos(&assets).unwrap();
    assert_eq!(infos.len(), 3336);
    // Every prefab the distribution can pick is known to the info table.
    for (_, entries) in table.distribution.biomes() {
        for e in entries.iter().filter(|e| e.class_id != "None") {
            assert!(infos.contains_key(&e.class_id), "{}", e.class_id);
        }
    }

    // The lifepod's batch: its placeholders, slots and fillers per seed.
    let cells = game
        .read_batch_cells(sn_world::BatchCoord::new(12, 18, 12))
        .unwrap()
        .unwrap();
    let fill = |seed| {
        let (mut placeholders, mut slots, mut spawned) = (0, 0, Vec::new());
        for tree in cells.cells.iter().filter_map(|c| c.objects.as_ref()) {
            for o in &tree.objects {
                for c in o
                    .components
                    .iter()
                    .filter(|c| c.type_name == sn_world::SLOTS_COMPONENT)
                {
                    placeholders += 1;
                    let s = sn_world::parse_slots(&c.data).unwrap();
                    slots += s.len();
                    for sp in sn_world::fill_slots(seed, &o.id, &s, &table.distribution, &infos) {
                        spawned.push((sp.class_id.to_string(), sp.transform));
                    }
                }
            }
        }
        (placeholders, slots, spawned)
    };
    let (placeholders, slots, a) = fill(1);
    assert_eq!((placeholders, slots, a.len()), (30, 1719, 325));
    assert_eq!(fill(1).2, a, "the same seed gives the same world");
    let b = fill(2).2;
    assert_eq!(b.len(), 334);
    assert_ne!(b, a);
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn startup_scenes_and_aurora() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let (additional, autoload) = assets.startup_scenes().unwrap();
    assert_eq!(additional, ["Essentials"]);
    let names: Vec<(&str, bool)> = autoload
        .iter()
        .map(|a| (a.scene_name.as_str(), a.spawn_on_start))
        .collect();
    assert_eq!(
        names,
        [("Cyclops", false), ("EscapePod", true), ("Aurora", true)]
    );

    let mut aurora = assets.scene("aurora").unwrap();
    assert_eq!(aurora.class_counts[&1], 3189); // GameObjects
    assert_eq!(aurora.class_counts[&23], 437); // MeshRenderers
    assert_eq!(aurora.class_counts[&205], 291); // LODGroups
    assert_eq!(aurora.roots.len(), 6);
    assert!(aurora.spawn_lightmapped_prefab());
    let drawn =
        |s: &sn_assets::Scene| -> usize { s.roots.iter().map(|r| r.visible_nodes().count()).sum() };
    assert_eq!(aurora.swap_aurora_models(&assets, false).unwrap(), (2, 2));
    assert_eq!(drawn(&aurora), 330);
    assert_eq!(aurora.swap_aurora_models(&assets, true).unwrap(), (2, 2));
    assert_eq!(drawn(&aurora), 337);

    let mut pod = assets.scene("escapepod").unwrap();
    assert_eq!(pod.class_counts[&1], 248);
    assert_eq!(pod.class_counts[&137], 35); // SkinnedMeshRenderers
    assert!(pod.spawn_lightmapped_prefab());
}

/// M7f4e: the exploder's numbers from the code, the scene's cull manager,
/// the Aurora's parts by state, and the `ShipExteriorCull` volumes.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn aurora_clock_and_exterior_cull_volumes() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let code = sn_assets::exploder_code(&sn_assets::read_assembly(&game).unwrap()).unwrap();
    assert_eq!(code.range, (2.3, 4.0));
    assert_eq!(code.day_seconds, 1200.0);
    assert_eq!(
        (code.sound_delay, code.fx_delay, code.swap_delay),
        (24.0, 25.0, 27.0)
    );

    let assets = Assets::index(&game).unwrap();
    let mut aurora = assets.scene("aurora").unwrap();
    assert!(aurora.spawn_lightmapped_prefab());
    let manager = aurora.ship_exterior_cull_manager(&assets).unwrap().unwrap();
    assert_eq!(manager.update_every_x_frames, 10);
    let groups = aurora.aurora_groups(&assets).unwrap();
    let drawn = |exploded: bool| -> usize {
        groups
            .iter()
            .filter(|g| g.show.shown(exploded, false))
            .map(|g| g.prefab.visible_nodes().count())
            .sum()
    };
    // The same as the whole scene swapped (startup_scenes_and_aurora).
    assert_eq!((drawn(false), drawn(true)), (330, 337));
    let exterior: usize = groups
        .iter()
        .filter(|g| g.show.exterior)
        .map(|g| g.prefab.visible_nodes().count())
        .sum();
    assert_eq!(exterior, 18);

    let catalog = assets.catalog().unwrap();
    let mut boxes = 0;
    for (key, n) in [
        (
            "WorldEntities/Doodads/Debris/Aurora/Rooms/CrashedShip_cargo_room.prefab",
            4,
        ),
        (
            "WorldEntities/Doodads/Debris/Aurora/Rooms/CrashedShip_exo_room.prefab",
            3,
        ),
    ] {
        let prefab = assets.prefab(&catalog, key).unwrap();
        assert_eq!(prefab.exterior_culls.len(), 1, "{key}");
        let cull = &prefab.exterior_culls[0];
        assert!(cull.registers, "{key}");
        assert_eq!((cull.boxes.len(), cull.missing), (n, 0), "{key}");
        for b in &cull.boxes {
            assert!(b.size.iter().all(|&s| s > 0.0), "{key}: {b:?}");
        }
        boxes += cull.boxes.len();
    }
    assert_eq!(boxes, 7);
}

/// M7f4f: the quality levels' LOD settings, the camera's field of view
/// and a prefab's LOD groups.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn lod_settings_field_of_view_and_groups() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let fov = sn_assets::field_of_view_code(&sn_assets::read_assembly(&game).unwrap()).unwrap();
    assert_eq!(fov, 60.0);
    let assets = Assets::index(&game).unwrap();
    let q = sn_assets::quality_settings(&assets).unwrap();
    let names: Vec<(&str, f32, i32)> = q
        .levels
        .iter()
        .map(|l| (l.name.as_str(), l.lod_bias, l.maximum_lod_level))
        .collect();
    assert_eq!(
        names,
        [("Low", 0.66, 0), ("Medium", 1.0, 0), ("High", 10.0, 0)]
    );
    let high = q.level("High").unwrap();
    assert_eq!((high.shadow_distance, high.shadow_cascades), (50.0, 4));

    // A rock-like prefab with levels: every drawn node in a group is in
    // at least one level, and groups have falling heights.
    let catalog = assets.catalog().unwrap();
    let prefab = assets
        .prefab(
            &catalog,
            "WorldEntities/Environment/AbandonedBases/AbandonedBaseFloatingIsland1.prefab",
        )
        .unwrap();
    assert!(!prefab.lod_groups.is_empty());
    assert_eq!(prefab.lod_conflicts, 0);
    for g in &prefab.lod_groups {
        assert!(g.heights.windows(2).all(|w| w[0] > w[1]), "{g:?}");
        assert_eq!(g.fade_mode, 0);
    }
    for (_, n) in prefab.drawn() {
        if let Some((g, levels)) = n.lod_group {
            assert!(g < prefab.lod_groups.len());
            assert!(levels != 0);
            assert_eq!(n.lod, Some(levels.trailing_zeros() as usize));
        }
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn skinned_lod_matches_its_static_lod() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let prefab = assets
        .prefab(
            &catalog,
            "WorldEntities/Environment/AbandonedBases/AbandonedBaseFloatingIsland1.prefab",
        )
        .unwrap();
    let mut skinned_count = 0;
    let mut bounds = |lod: usize| {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for (i, n) in prefab.nodes.iter().enumerate() {
            if n.lod != Some(lod) || !n.active || !n.renderer_enabled || n.mesh.is_none() {
                continue;
            }
            let (m, g) = assets.mesh(n.mesh.as_ref().unwrap()).unwrap();
            for row in &m.bind_poses {
                assert_eq!([row[3], row[7], row[11], row[15]], [0.0, 0.0, 0.0, 1.0]);
            }
            // Bone-less skinned renderers are drawn as plain meshes.
            let points: Vec<[f32; 3]> = if let Some(s) = prefab.skinned_geometry(i, &m, &g) {
                skinned_count += 1;
                s.positions
            } else {
                g.positions
                    .iter()
                    .map(|&p| {
                        n.in_prefab
                            .then(&sn_world::Transform {
                                position: p,
                                ..Default::default()
                            })
                            .position
                    })
                    .collect()
            };
            for p in points {
                for a in 0..3 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
        }
        (lo, hi)
    };
    let (a, b) = (bounds(0), bounds(1));
    assert!(skinned_count > 0);
    for k in 0..3 {
        let size = b.1[k] - b.0[k];
        assert!(size > 0.5, "LOD 1 found");
        assert!(
            ((a.1[k] - a.0[k]) - size).abs() < 0.03 * size,
            "{a:?} vs {b:?}"
        );
        assert!(((a.1[k] + a.0[k]) - (b.1[k] + b.0[k])).abs() / 2.0 < 0.03 * size);
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn lifepod_start_and_modules() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let map = assets.start_map().unwrap();
    // 0.59 % of the 512 × 512 pixels.
    assert!(
        (map.valid_share() - 0.0059).abs() < 0.0005,
        "{}",
        map.valid_share()
    );
    let (point, _) = map.random_start(1);
    assert!(map.is_valid(point[0], point[2]));
    let mut pod = assets.scene("escapepod").unwrap();
    assert!(pod.spawn_lightmapped_prefab());
    let spawn = pod.place_escape_pod(&assets, point).unwrap();
    // The player spawn is inside the pod, about 2 m above the water line.
    let d: Vec<f32> = (0..3).map(|a| spawn.position[a] - point[a]).collect();
    assert!(
        d[0].hypot(d[2]) < 2.0 && (1.5..2.5).contains(&d[1]),
        "{d:?}"
    );
    assert_eq!(pod.follow_targets(&assets).unwrap(), 6);
    let spawns = pod.spawns(&assets).unwrap();
    assert_eq!(spawns.len(), 7);
    let catalog = assets.catalog().unwrap();
    let mut fabricators = 0;
    for s in &spawns {
        // Every module's parent is inside the pod (radius ~3 m).
        let p = s.parent.position;
        let r = (p[0] - point[0]).hypot(p[2] - point[2]);
        assert!(
            r < 4.0 && (-1.0..4.0).contains(&p[1]),
            "{} at {p:?}",
            s.name
        );
        if let sn_unity::SpawnPrefab::Address(guid) = &s.spawner.prefab {
            let prefab = assets.prefab(&catalog, guid).unwrap();
            fabricators += usize::from(prefab.nodes[0].name == "Fabricator");
        }
    }
    assert_eq!(fabricators, 1);
}

/// M7g1: the game's main camera leaves the `Occluder` layer (27) out, and
/// the occluder shells are on it (`docs/formats/materials.md`).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn main_camera_skips_the_occluder_layer() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let camera = assets.main_camera().unwrap();
    assert_eq!(camera.culling_mask, 0x65ff_ff17);
    assert_eq!(
        (camera.near, camera.far, camera.field_of_view),
        (0.03, 1700.0, 60.0)
    );
    assert!(!camera.draws_layer(27));

    let catalog = assets.catalog().unwrap();
    let prefab = assets
        .prefab(
            &catalog,
            "WorldEntities/Doodads/Precursor/LavaBase/Final_Rooms/Precursor_LavaBase_Hallway.prefab",
        )
        .unwrap();
    let hidden: Vec<&str> = prefab
        .visible_nodes()
        .filter(|n| !camera.draws_layer(n.layer))
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(hidden, ["Occluder_Precursor_LavaBase_Hallway_shell"]);
}

/// M7g2: shader names and render state from `Shader`'s parsed form; the
/// values were checked against UnityPy (`docs/formats/materials.md`).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn shaders_of_the_fake_volumetric_light() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let prefab = assets
        .prefab(
            &catalog,
            "WorldEntities/Doodads/Precursor/Gun/IonCrystalPedestal.prefab",
        )
        .unwrap();
    let mut names = Vec::new();
    for node in prefab.visible_nodes() {
        for material in node.materials.iter().flatten() {
            let (_, data) = material.data().unwrap();
            let m = sn_unity::Material::parse(data, material.file.file().big_endian).unwrap();
            let shader = assets.resolve(&material.file, m.shader).unwrap().unwrap();
            let (_, data) = shader.data().unwrap();
            let s = sn_unity::Shader::parse(data, shader.file.file().big_endian).unwrap();
            if node.name == "x_FakeVolumletricLight" {
                let pass = &s.passes()[0];
                let b = &pass.state.blend[0];
                assert_eq!((b.src.value, b.dst.value), (1.0, 1.0));
                assert_eq!(pass.state.z_write.value, 0.0);
                assert_eq!(pass.state.cull.value, 0.0);
            }
            names.push(s.name);
        }
    }
    assert!(
        names
            .iter()
            .any(|n| n == "UWE/Particles/WBOIT-FakeVolumetricLight")
    );
    assert!(names.iter().any(|n| n == "MarmosetUBER"));
}

/// M7h: the placeholders the game spawns into the Blood Kelp cache's
/// pedestals (an ion crystal) and its door root (the door and the key
/// terminal); every placeholder's layout read to its last byte.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn prefab_placeholders() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let paths = game.read_prefab_database().unwrap();
    let spawned = |key: &str| -> Vec<String> {
        let prefab = assets.prefab(&catalog, key).unwrap();
        assert_eq!(prefab.placeholder_groups.len(), 1, "{key}");
        let group = &prefab.placeholder_groups[0];
        assert!(group.enabled);
        assert_eq!(group.node, 0);
        group
            .placeholders
            .iter()
            .map(|&n| {
                let id = prefab.nodes[n].placeholder.as_deref().unwrap();
                paths.get(id).cloned().unwrap_or_else(|| format!("? {id}"))
            })
            .collect()
    };
    let pedestal = spawned("WorldEntities/Doodads/Precursor/Gun/IonCrystalPedestal.prefab");
    assert_eq!(
        pedestal,
        ["WorldEntities/Natural/PrecursorIonCrystal.prefab"]
    );
    let door = spawned(
        "WorldEntities/Environment/Precursor/Cache/Precursor_BloodKelpCache_DoorTerminalsRoot1.prefab",
    );
    assert_eq!(
        door,
        [
            "WorldEntities/Environment/Precursor/Gun/Precursor_Gun_Terminal2Door.prefab",
            "WorldEntities/Environment/Precursor/Precursor_PurpleKeyTerminal.prefab"
        ]
    );
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn anchor_pod_orientation() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();

    let cases = [
        ("Coral_reef_floating_stones_big_02", 9.69, 39.19),
        ("Coral_reef_floating_stones_mid_01", 16.35, 16.79),
        ("Coral_reef_floating_stones_mid_02", 26.72, 27.59),
        ("Coral_reef_floating_stones_small_01", 9.18, 9.20),
        ("Coral_reef_floating_stones_small_02", 14.27, 14.33),
    ];

    let expected_rot = [
        -std::f32::consts::FRAC_1_SQRT_2,
        0.0,
        0.0,
        std::f32::consts::FRAC_1_SQRT_2,
    ];

    for (name, expected_stone_y, expected_light_y) in cases {
        let key = format!("WorldEntities/Environment/{name}.prefab");
        let prefab = assets.prefab(&catalog, &key).unwrap();

        // The `model` node should have the -90 deg X rotation (Z-up to Y-up).
        let model_node = prefab.nodes.iter().find(|n| n.name == "model").unwrap();
        for (i, expected) in expected_rot.iter().enumerate() {
            assert!(
                (model_node.local.rotation[i] - expected).abs() < 1e-5,
                "{name}: rot mismatch at {i}: {:?}",
                model_node.local.rotation
            );
        }

        // The light node is along +Y at expected height.
        let light_node = prefab.nodes.iter().find(|n| n.name == "light").unwrap();
        let light_y = light_node.in_prefab.position[1];
        assert!(
            (light_y - expected_light_y).abs() < 0.1,
            "{name}: light Y mismatch: {light_y} vs {expected_light_y}"
        );

        // Stone mesh bounds should be elevated along +Y pointing upwards.
        let stone = prefab
            .visible_nodes()
            .find(|n| n.name.starts_with("stone_"))
            .unwrap();
        let (_, g) = assets.mesh(stone.mesh.as_ref().unwrap()).unwrap();
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for p in &g.positions {
            let world_p = stone.in_prefab.then(&sn_world::Transform {
                position: *p,
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            });
            min_y = min_y.min(world_p.position[1]);
            max_y = max_y.max(world_p.position[1]);
        }
        let center_y = (min_y + max_y) * 0.5;
        assert!(
            (center_y - expected_stone_y).abs() < 0.5,
            "{name}: stone center Y {center_y} should match expected {expected_stone_y}"
        );
        // Vines/roots anchor at the seafloor and the stone bulb is elevated above them.
        assert!(
            min_y > 5.0,
            "{name}: stone pod should be elevated above seafloor"
        );
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn tech_data_and_ent_tech_data() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let data = sn_assets::tech_data(&assets).unwrap();
    eprintln!("TechData entries: {}", data.entries.len());
    // 463 entries + 513 ingredients = the 976 `techType` keys in the file.
    assert_eq!(data.entries.len(), 463);
    assert_eq!(
        data.entries
            .iter()
            .map(|e| e.ingredients.len())
            .sum::<usize>(),
        513
    );
    assert_eq!(
        data.entries
            .iter()
            .filter(|e| e.craft_time.is_some())
            .count(),
        40
    );
    assert_eq!(data.without_tech_type, 0);
    assert!(data.duplicates.is_empty(), "{:?}", data.duplicates);
    assert!(data.unknown_keys.is_empty(), "{:?}", data.unknown_keys);
    assert_eq!(data.recipes().count(), 245);
    // Ingredients with no entry of their own (all their fields are
    // defaults, so the game's editor trimmed them): logged, not errors.
    let misses = data.ingredients_without_entry();
    let missing: std::collections::BTreeSet<i32> = misses.iter().map(|m| m.1).collect();
    eprintln!(
        "ingredients without an entry: {} rows, {} tech types",
        misses.len(),
        missing.len()
    );
    assert_eq!((misses.len(), missing.len()), (28, 12));

    let ent = sn_assets::ent_tech_data(&assets).unwrap();
    assert_eq!(ent.len(), 703);
    assert!(ent.iter().all(|e| e.tech_type != 0));
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn player_and_pda_data() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let d = sn_assets::player_data(&assets).unwrap();
    assert_eq!(d.player.equipment_models.len(), 3);
    assert_eq!(d.player.player_sphere_radius, 0.5);
    assert_eq!(
        (
            d.player.suffocation_time,
            d.player.suffocation_recovery_time
        ),
        (8.0, 4.0)
    );
    assert_eq!(d.oxygen.oxygen_capacity, 45.0);
    assert_eq!(
        (d.live_mixin.health, d.live_mixin_data.max_health),
        (100.0, 100.0)
    );
    let c = &d.controller;
    assert_eq!(c.swim_forward_max_speed, 7.6);
    assert_eq!(c.walk_run_forward_max_speed, 3.5);
    assert_eq!((c.stand_height, c.swim_height), (1.5, 0.5));
    assert_eq!(d.underwater_motor.motor.water_acceleration, 20.0);

    let pda = &d.pda;
    assert_eq!(
        (
            pda.log.len(),
            pda.encyclopedia.len(),
            pda.scanner.len(),
            pda.default_tech.len(),
            pda.analysis_tech.len(),
            pda.compound_tech.len()
        ),
        (179, 323, 268, 46, 126, 3)
    );
    // Every starting blueprint has TechData.
    let tech = sn_assets::tech_data(&assets).unwrap();
    for t in &pda.default_tech {
        assert!(tech.get(*t).is_some(), "default tech {t}");
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn game_code_names_menus_and_defaults() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let bytes = sn_assets::read_assembly(&game).unwrap();
    let code = sn_assets::game_code(&bytes).unwrap();
    eprintln!("TechType names: {}", code.tech_types.len());
    assert_eq!(code.tech_types.len(), 793);
    assert_eq!(code.tech_name(0), Some("None"));

    // Every TechData entry and ingredient has a TechType name.
    let assets = Assets::index(&game).unwrap();
    let tech = sn_assets::tech_data(&assets).unwrap();
    for e in &tech.entries {
        assert!(
            code.tech_name(e.tech_type).is_some(),
            "entry {}",
            e.tech_type
        );
        for i in &e.ingredients {
            assert!(
                code.tech_name(i.tech_type).is_some(),
                "ingredient {}",
                i.tech_type
            );
        }
    }

    let craft = code.tree_action("Craft").unwrap();
    let counts: Vec<(&str, usize)> = code
        .craft_trees
        .iter()
        .map(|t| (t.id.as_str(), t.root.walk().len()))
        .collect();
    eprintln!("craft trees (nodes): {counts:?}");
    assert_eq!(
        counts,
        [
            ("Fabricator", 100),
            ("Constructor", 7),
            ("Workbench", 13),
            ("SeamothUpgrades", 22),
            ("MapRoom", 5),
            ("Centrifuge", 3),
            ("CyclopsFabricator", 9),
        ]
    );
    // Every craft node's tech type has TechData.
    let mut crafts = 0;
    for tree in &code.craft_trees {
        for n in tree.root.walk() {
            if n.action == craft {
                crafts += 1;
                assert!(tech.get(n.tech_type).is_some(), "{}: {}", tree.id, n.id);
            }
        }
    }
    assert_eq!(crafts, 134);

    let d = &code.tech_defaults;
    assert_eq!(d.item_size, [1, 1]);
    assert_eq!((d.craft_time, d.craft_amount), (0.0, 1));
    assert_eq!(d.max_charge, -1.0);
    assert!(!d.buildable);
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn lifepod_colliders() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let mut scene = assets.scene("escapepod").unwrap();
    scene.spawn_lightmapped_prefab();
    scene.place_escape_pod(&assets, [10.0, 0.0, -20.0]).unwrap();
    let mut meshes = sn_assets::ColliderMeshes::new();
    let mut counts = sn_assets::ColliderCounts::default();
    let mut kinds = [0usize; 4];
    for root in &scene.roots {
        let (list, c) = assets.prefab_colliders(root, &mut meshes).unwrap();
        counts.add(&c);
        for col in list.iter().filter(|c| !c.trigger) {
            let w = col.world(&root.nodes[0].local);
            let (kind, finite) = match &w {
                sn_assets::WorldCollider::Box { center, half, .. } => {
                    (0, center.iter().chain(half).all(|v| v.is_finite()))
                }
                sn_assets::WorldCollider::Sphere { center, radius } => (
                    1,
                    center.iter().all(|v| v.is_finite()) && radius.is_finite(),
                ),
                sn_assets::WorldCollider::Capsule { a, b, radius } => (
                    2,
                    a.iter().chain(b).all(|v| v.is_finite()) && radius.is_finite(),
                ),
                sn_assets::WorldCollider::Triangles(t) => {
                    (3, t.iter().flatten().flatten().all(|v| v.is_finite()))
                }
            };
            assert!(finite, "{w:?}");
            kinds[kind] += 1;
        }
    }
    eprintln!("lifepod colliders: {counts:?}; box, sphere, capsule, mesh: {kinds:?}");
    // 54 colliders: 40 solid boxes; 5 enabled triggers on active nodes
    // (returned marked, not solid); 5 disabled and 4 on inactive nodes
    // left out (checked in that order).
    assert_eq!(kinds, [40, 0, 0, 0]);
    assert_eq!(
        (
            counts.kept,
            counts.triggers,
            counts.disabled,
            counts.inactive
        ),
        (40, 5, 5, 4)
    );
    assert_eq!(
        (counts.layout_errors, counts.mesh_errors, counts.null_mesh),
        (0, 0, 0)
    );
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn player_movement_data() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let d = sn_assets::player_data(&assets).unwrap();
    let g = &d.ground_motor;
    assert_eq!((g.step_offset, g.slope_limit), (0.4, 60.0));
    assert_eq!((g.max_fall_speed, g.sliding_speed), (50.0, 7.0));
    assert!(!d.rigidbody.use_gravity && d.rigidbody.drag == 2.5);
    assert_eq!((d.player_layer, d.ocean_level), (19, 0.0));
    let s = sn_assets::physics_settings(&assets).unwrap();
    assert_eq!(s.time.fixed_timestep, 0.02);
    assert!(!s.physics.queries_hit_backfaces);
    assert_eq!(s.tags.layer("Player"), Some(19));
    assert_eq!(s.tags.layer("OnlyVehicle"), Some(9));
    let missed: Vec<u32> = (0..32).filter(|&l| !s.physics.collides(19, l)).collect();
    assert_eq!(missed, vec![9]);

    // The lifepod's hand triggers: 8, each with an end point; in a new
    // game the two first-use ones are active and their normal ones not.
    let mut scene = assets.scene("escapepod").unwrap();
    scene.spawn_lightmapped_prefab();
    scene.place_escape_pod(&assets, [0.0; 3]).unwrap();
    let pairs = scene
        .init_lifepod_hatches(&assets, false, false)
        .unwrap()
        .unwrap();
    let triggers = scene.cinematic_triggers(&assets).unwrap();
    assert_eq!(triggers.len(), 8);
    assert!(triggers.iter().all(|t| t.hand && t.end.is_some()));
    assert_eq!(triggers.iter().filter(|t| t.enters).count(), 2);
    assert_eq!(triggers.iter().filter(|t| t.exits).count(), 6);
    assert_eq!(triggers.iter().filter(|t| t.end_only_in_vr).count(), 4);
    for (normal, first) in pairs {
        let active = |(r, n): (usize, usize)| scene.roots[r].nodes[n].active;
        assert!(!active(normal) && active(first));
    }
    // M9g5: every trigger's cinematic moves the player along one node of
    // the pod's animator, which one forwarder passes each end event from;
    // boarding calls `CinematicEnter` at the start, leaving `CinematicExit`
    // at the end.
    let name = |(r, n): (usize, usize)| scene.roots[r].nodes[n].name.clone();
    for t in &triggers {
        assert_eq!(t.animated_node.map(name).as_deref(), Some("cin_target"));
        assert_eq!(
            t.animator_node.map(name).as_deref(),
            Some("Life_Pod_damaged_03")
        );
        assert_eq!(
            (
                t.cinematic.interpolation_time,
                t.cinematic.interpolation_time_out
            ),
            (0.25, 0.25)
        );
        assert!(!t.cinematic.enforce_cinematic_mode_end);
        let start = t
            .trigger
            .on_cinematic_start
            .iter()
            .any(|c| c.method_name == "CinematicEnter");
        let end = t
            .trigger
            .on_cinematic_end
            .iter()
            .any(|c| c.method_name == "CinematicExit");
        assert!(if t.enters { start } else { end }, "{}", t.name);
    }
    let animated: std::collections::HashSet<_> =
        triggers.iter().filter_map(|t| t.animated_node).collect();
    assert_eq!(animated.len(), 1);
    let forwards = scene.cinematic_forwards(&assets).unwrap();
    assert_eq!(forwards.len(), 1);
    assert_eq!(name(forwards[0].node), "Life_Pod_damaged_03");
    assert!(
        triggers
            .iter()
            .all(|t| forwards[0].forward.contains(&t.cinematic_key))
    );
    // The hatch clips' end events: at their last frame (the intro's
    // `escapepod_full` clips are not hatch cinematics).
    let (r, n) = triggers[0].animator_node.unwrap();
    let controller = scene.roots[r].nodes[n]
        .animator
        .as_ref()
        .and_then(|a| a.controller.clone())
        .unwrap();
    let set = assets
        .animation_set(&controller, &mut std::collections::HashMap::new())
        .unwrap();
    let mut ends: Vec<(String, f32)> = set
        .clips
        .iter()
        .flatten()
        .filter(|c| !c.name.starts_with("escapepod_full"))
        .filter_map(|c| {
            let e = c
                .events
                .iter()
                .find(|e| e.function == "OnPlayerCinematicModeEnd")?;
            assert!(
                (e.time - (c.stop_time - c.start_time)).abs() < 0.03,
                "{}",
                c.name
            );
            Some((c.name.clone(), (e.time * 1000.0).round() / 1000.0))
        })
        .collect();
    ends.sort_by(|a, b| a.0.cmp(&b.0));
    ends.dedup();
    eprintln!("cinematic end events: {ends:?}");
    assert_eq!(
        ends,
        [
            ("bot_in", 1.667),
            ("bot_out", 0.667),
            ("escapepod_first_botout", 6.333),
            ("escapepod_first_topout", 8.167),
            ("side", 2.667),
            ("side2", 2.667),
            ("top_in", 1.167),
            ("top_out", 2.0),
        ]
        .map(|(n, t)| (n.to_string(), t))
    );
    // The dive hatch object of the pod is not in use (inactive).
    let hatches = scene.dive_hatches(&assets).unwrap();
    assert_eq!(hatches.len(), 1);
    let (r, n) = hatches[0].node;
    assert!(!scene.roots[r].nodes[n].active);
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn vitals_and_look_data() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let d = sn_assets::player_data(&assets).unwrap();
    let code = sn_assets::player_code(&sn_assets::read_assembly(&game).unwrap()).unwrap();
    let v = sn_assets::vitals_params(&d, &code);
    let l = sn_assets::look_params(&d, &code);
    eprintln!("vitals {v:?}; look {l:?}; camera on {:?}", d.camera_node);
    assert_eq!((v.oxygen_capacity, v.refill_per_second), (45.0, 30.0));
    assert_eq!((v.max_health, v.start_health_percent), (100.0, 1.0));
    assert_eq!(v.oxygen_above_player, 0.0);
    assert_eq!((l.minimum_y, l.maximum_y), (-87.0, 87.0));
    assert!((l.mouse_sensitivity - 0.15).abs() < 1e-6);
    assert_eq!((d.camera_node.1, d.camera.skin), ([0.0; 3], 0.0));
}

/// M7f4a/b: the player's and the lifepod's animators bind every property
/// to their hierarchy and run 60 s from their defaults without NaNs and
/// with unit quaternions (`docs/formats/animation.md`).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn player_and_lifepod_animators_run() {
    use std::collections::HashMap;
    use std::sync::Arc;

    use sn_anim::{Animator, Program, SlotKind};
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let mut cache = HashMap::new();
    // (scene, animator node name, slots, values, slots not in the hierarchy)
    let expected = [
        ("main", "player_view", 213, 710, 0),
        ("escapepod", "Life_Pod_damaged_03", 137, 458, 0),
    ];
    for (scene, node_name, slots, width, missing) in expected {
        let scene = assets.scene(scene).unwrap();
        let (prefab, node) = scene
            .roots
            .iter()
            .find_map(|p| {
                p.nodes
                    .iter()
                    .position(|n| n.name == node_name && n.animator.is_some())
                    .map(|i| (p, i))
            })
            .unwrap_or_else(|| panic!("{node_name}: no animator"));
        let a = prefab.nodes[node].animator.as_ref().unwrap();
        let set = assets
            .animation_set(a.controller.as_ref().unwrap(), &mut cache)
            .unwrap();
        assert!(set.errors.is_empty(), "{:?}", set.errors);
        let program = Arc::new(Program::new(Arc::new(set.controller.clone()), &set.clips));
        assert_eq!(program.missing_clips, 0, "{node_name}");
        let binding = prefab.bind_animator(node, &program, &|_| Vec::new());
        assert_eq!(
            (program.slots.len(), program.width, binding.missing),
            (slots, width, missing),
            "{node_name}"
        );
        let mut animator = Animator::new(program.clone(), binding.defaults);
        for _ in 0..60 * 60 {
            animator.update(1.0 / 60.0);
            let pose = animator.pose();
            assert!(pose.iter().all(|v| v.is_finite()), "{node_name}: NaN");
            for s in program
                .slots
                .iter()
                .filter(|s| s.kind == SlotKind::Rotation)
            {
                let q = &pose[s.offset..s.offset + 4];
                let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
                assert!((len - 1.0).abs() < 1e-4, "{node_name}: |q| = {len}");
            }
        }
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn blend_shape_clamp_and_brain_coral() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    // `PlayerSettings.legacyClampBlendShapeWeights` (M7f4d).
    assert!(sn_assets::blend_shape_clamp(&assets).unwrap());
    let catalog = assets.catalog().unwrap();
    let prefab = assets
        .prefab(&catalog, "WorldEntities/Environment/BrainCoral.prefab")
        .unwrap();
    let names = assets.blend_shape_names(&prefab);
    eprintln!("BrainCoral: blend shape channels by node {names:?}");
    // One skinned renderer (LOD 0) with three channels, all stored at 0.
    assert_eq!(names.len(), 1);
    let (&node, channels) = names.iter().next().unwrap();
    assert_eq!(channels.len(), 3);
    let n = &prefab.nodes[node];
    assert_eq!(n.lod, Some(0));
    assert!(n.blend_shape_weights.iter().all(|&w| w == 0.0));
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn lifepod_sky_and_lighting_controller() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let mut pod = assets.scene("escapepod").unwrap();
    assert!(pod.spawn_lightmapped_prefab());
    pod.place_escape_pod(&assets, [100.0, 0.0, 200.0]).unwrap();
    let l = pod.lifepod_lighting(&assets).unwrap().unwrap();
    let s = &l.sky.sky;
    eprintln!(
        "anchor sky {:?}: master {} diff {} spec {} sky {} cam {} affected {} outdoors {} rotation {:?}",
        l.sky.name,
        s.master_intensity,
        s.diff_intensity,
        s.spec_intensity,
        s.sky_intensity,
        s.cam_exposure,
        s.affected_by_day_night,
        s.outdoors,
        l.sky.rotation
    );
    let c = &l.controller;
    eprintln!(
        "controller: state {} fade {} emissive {:?}",
        c.state, c.fade_duration, c.emissive
    );
    for li in &l.lights {
        eprintln!(
            "light {:?} on node active_self {}: {:?} colour {:?} range {} angle {} at {:?}, per state {:?}",
            pod.roots[li.node.0].nodes[li.node.1].name,
            pod.roots[li.node.0].nodes[li.node.1].active_self,
            li.light.kind,
            li.light.color,
            li.light.range,
            li.light.spot_angle,
            li.world.position,
            li.intensities
        );
    }
    // The stored values (docs/formats/gameplay.md § The lifepod's light).
    // The sky stores the Operational state's intensities.
    assert_eq!(l.sky.name, "SkyEscapePod");
    assert_eq!(
        (s.master_intensity, s.diff_intensity, s.spec_intensity),
        (10.0, 2.0, 1.5)
    );
    assert!(!s.affected_by_day_night);
    assert_eq!((c.state, c.fade_duration), (0, 1.0));
    assert_eq!(l.controls_anchor, [true]);
    assert_eq!(c.skies[0].master, [10.0, 0.8, 2.5]);
    assert_eq!(c.skies[0].diffuse, [2.0, 0.5, 0.8]);
    assert_eq!(c.skies[0].specular, [1.5, 3.0, 1.0]);
    assert_eq!(c.emissive, [0.0, 1.0, 1.0]);
    assert_eq!((l.lights.len(), l.missing_lights), (3, 0));
    let per_state: Vec<_> = l.lights.iter().map(|x| x.intensities.clone()).collect();
    assert_eq!(
        per_state,
        [[0.0, 1.25, 0.0], [0.0, 1.25, 0.0], [0.0, 0.22, 0.0]]
    );
    // Their GameObjects start off (`MultiStatesLight` switches them on).
    assert!(
        l.lights
            .iter()
            .all(|x| !pod.roots[x.node.0].nodes[x.node.1].active_self)
    );
    // The lights are inside the pod (radius ~3 m).
    for x in &l.lights {
        let p = x.world.position;
        assert!(
            (p[0] - 100.0).hypot(p[2] - 200.0) < 3.0 && (-1.0..5.0).contains(&p[1]),
            "{p:?}"
        );
    }
    // Skipping the intro: the lights animator off, the hatch light off.
    let changed = pod.stop_pod_intro(&assets).unwrap();
    eprintln!("stop_pod_intro: {changed:?}");
    assert_eq!(changed.len(), 2);
}

/// M9g1: the player's body in the main scene: the equipment rule's slots
/// and what a new game draws, the head drawn only in shadows, the camera's
/// nodes, `ArmsController`, and every parameter the ported rules set is in
/// the player's controller (`docs/formats/gameplay.md` § The player's body).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn player_body() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let mut body = assets.player_body().unwrap();
    assert_eq!(body.prefab.nodes.len(), 134);
    let models: Vec<usize> = body.slots.iter().map(|s| s.models.len()).collect();
    assert_eq!(models, vec![4, 2, 3]);
    assert!(
        body.slots
            .iter()
            .flat_map(|s| &s.models)
            .all(|m| m.1.is_some())
    );
    // A new game: the defaults of two slots, nothing on the feet.
    let defaults: Vec<bool> = body
        .slots
        .iter()
        .map(|s| s.default_node.is_some())
        .collect();
    assert_eq!(defaults, vec![true, true, false]);
    let drawn: Vec<usize> = body.prefab.drawn().map(|(i, _)| i).collect();
    assert_eq!(drawn.len(), 3);
    assert!(drawn.contains(&body.head_node));
    assert!(drawn.iter().all(|&i| body.prefab.nodes[i].skinned));
    let head = &body.prefab.nodes[body.head_node];
    assert_eq!(head.cast_shadows, sn_assets::SHADOWS_ONLY);
    // Every drawn sub-mesh is MarmosetUBER.
    for &i in &drawn {
        for m in body.prefab.nodes[i].materials.iter().flatten() {
            assert_eq!(assets.shader_name(m).unwrap(), "MarmosetUBER");
        }
    }
    // The view model, the camera and the animator sit at the player's
    // origin; the look-up pivot does not.
    for n in [body.view_model_node, body.camera_node, body.animator_node] {
        assert_eq!(body.prefab.nodes[n].in_prefab.position, [0.0; 3]);
    }
    // M9g5: the camera's anchor in cinematics and its bone after death.
    assert_eq!(body.prefab.nodes[body.cam_anchor_node].name, "Cam");
    assert_eq!(
        body.prefab.nodes[body.head_camera_node].name,
        "cam_deathpos"
    );
    let up = body.prefab.nodes[body.camera_up_node].in_prefab.position;
    assert!(
        (up[1] - 0.063).abs() < 1e-3 && (up[2] + 0.15).abs() < 1e-3,
        "{up:?}"
    );
    let a = &body.arms;
    assert_eq!(
        (
            a.smooth_speed_under_water,
            a.smooth_speed_above_water,
            a.turn_animation_damp_time,
            a.ik_toggle_time
        ),
        (10.0, 15.0, 0.0, 0.0)
    );
    let controller = assets.animator_controller(&body.controller).unwrap();
    assert_eq!(controller.params.len(), 201);
    for name in sn_assets::RULE_PARAMETERS
        .iter()
        .chain(sn_assets::FIXED_PARAMETERS)
    {
        let id = sn_unity::name_hash(name);
        assert!(controller.params.iter().any(|p| p.id == id), "{name}");
    }
    // M9g2: the numbers `sn_sim::body` runs with.
    let bp = body.body_params(0.0);
    eprintln!("body params: {bp:?}");
    assert_eq!(bp.turn_animation_damp_time, 0.0);
    assert_eq!(
        (bp.skin, bp.step_amount, bp.view_model_roll),
        (0.0, 0.0, 0.0)
    );
    assert!((bp.camera_up_position.y - 0.063).abs() < 1e-3);
    assert!((bp.camera_up_position.z + 0.15).abs() < 1e-3);
    // A few micrometres of editor rounding.
    assert!(bp.camera_offset_position.length() < 1e-5);
    // The main camera hangs on cameraOffsetTransform (AutoParent).
    assert_eq!(body.main_camera_parent, body.camera_offset_node);
    assert_eq!(body.main_camera_in_object.position, [0.0; 3]);
    // Equipping the first body model hides the default and shows it.
    let (tech, node) = body.slots[0].models[0];
    let default = body.slots[0].default_node.unwrap();
    body.equip(|s| if s == "Body" { tech } else { 0 });
    assert!(body.prefab.nodes[node.unwrap()].active);
    assert!(!body.prefab.nodes[default].active);
}

/// M9g5b: each hatch cinematic on the pod's own animator, as the game
/// drives it (`prepare_…` for the 0.25 s move in, then the hatch
/// parameter): its end event fires in one frame, at about the clip's length, and
/// the animated node (`cin_target`) moves.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn lifepod_hatch_cinematic_events() {
    use std::collections::HashMap;
    use std::sync::Arc;

    use sn_anim::{Animator, Program};
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let mut scene = assets.scene("escapepod").unwrap();
    scene.spawn_lightmapped_prefab();
    scene.place_escape_pod(&assets, [0.0; 3]).unwrap();
    let triggers = scene.cinematic_triggers(&assets).unwrap();
    let (r, node) = triggers[0].animator_node.unwrap();
    let prefab = &scene.roots[r];
    let controller = prefab.nodes[node]
        .animator
        .as_ref()
        .and_then(|a| a.controller.clone())
        .unwrap();
    let set = assets
        .animation_set(&controller, &mut HashMap::new())
        .unwrap();
    let program = Arc::new(Program::new(Arc::new(set.controller.clone()), &set.clips));
    let binding = prefab.bind_animator(node, &program, &|_| Vec::new());
    let (ar, an) = triggers[0].animated_node.unwrap();
    assert_eq!(ar, r);
    let posed = prefab.posed_node(an, &program, &binding).unwrap();
    // The stored pose gives the stored placement.
    let stored = posed.in_prefab(&binding.defaults);
    let d = (0..3)
        .map(|a| (stored.position[a] - prefab.nodes[an].in_prefab.position[a]).abs())
        .fold(0.0f32, f32::max);
    assert!(d < 1e-4, "stored pose off by {d}");
    let dt = 1.0 / 60.0;
    for t in &triggers {
        let c = &t.cinematic;
        let mut a = Animator::new(program.clone(), binding.defaults.clone());
        for _ in 0..30 {
            a.update(dt);
        }
        a.set_bool(sn_unity::name_hash(&c.interpolate_anim_param), true);
        for _ in 0..15 {
            a.update(dt);
        }
        a.set_bool(sn_unity::name_hash(&c.interpolate_anim_param), false);
        assert!(a.set_bool(sn_unity::name_hash(&c.anim_param), true));
        let from = posed.in_prefab(a.pose());
        let mut fired = Vec::new();
        let mut travel = 0.0f32;
        let mut last = from.position;
        for i in 1..=60 * 12 {
            a.update(dt);
            let p = posed.in_prefab(a.pose()).position;
            travel += (0..3).map(|k| (p[k] - last[k]).powi(2)).sum::<f32>().sqrt();
            last = p;
            for e in &a.events {
                if e.function == "OnPlayerCinematicModeEnd" {
                    fired.push(i as f32 * dt);
                }
            }
        }
        eprintln!(
            "{}: {:?} → end event at {fired:?} s; cin_target from {:?} to {last:?}, travelled {travel:.2} m",
            t.name, c.anim_param, from.position
        );
        // Once per playing clip that carries it (two layers play the
        // first-use clips): all in one frame.
        assert!(!fired.is_empty(), "{}", t.name);
        assert!(fired.iter().all(|&f| f == fired[0]), "{}", t.name);
        assert!(travel > 0.1, "{}: cin_target did not move", t.name);
    }
}

/// M10e: the multiplayer rules read twice give the same fingerprint (two
/// players on the same install must match), the physics step is the
/// game's 0.02 s, and seed 1's world is the one the server logged.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn multiplayer_rules_are_stable() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let a = sn_assets::rules(&game, &Assets::index(&game).unwrap()).unwrap();
    let assets = Assets::index(&game).unwrap();
    let b = sn_assets::rules(&game, &assets).unwrap();
    println!(
        "build {:?}, fingerprint {:016x}, parts {:x?}, tick {} µs, max speed {} m/s",
        a.build, a.fingerprint, a.parts, a.tick_us, a.max_speed
    );
    assert_eq!(a.fingerprint, b.fingerprint);
    assert_eq!(a.parts, b.parts);
    assert_eq!(a.build, "10");
    assert_eq!(a.tick_us, 20_000);
    let (point, tries) = sn_assets::lifepod_point(&assets, 1).unwrap();
    println!(
        "seed 1: lifepod {point:?} after {tries} draws; aurora countdown {} s",
        sn_assets::aurora_countdown(&a.exploder, 1)
    );
    assert_eq!(tries, 72);
    // Within the game's 2.3–4 days of 1,200 s.
    let c = sn_assets::aurora_countdown(&a.exploder, 1);
    assert!((2.3 * 1200.0..=4.0 * 1200.0).contains(&c), "{c}");
}

/// M9d1: the item data. Prefabs checked here are the ones the spawn slots
/// can place (the placed ones are counted by `sn-inspect pickups`).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn item_data() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();

    // The rules in the code.
    let dll = sn_assets::read_assembly(&game).unwrap();
    let code = sn_assets::item_code(&dll).unwrap();
    assert_eq!(code.inventory_size, [6, 8]);
    assert_eq!(code.slot_types.len(), 38);
    assert_eq!(
        code.player_slots,
        ["Head", "Body", "Gloves", "Foots", "Chip1", "Chip2", "Tank"]
    );
    assert!(
        code.player_slots
            .iter()
            .all(|s| code.slot_types.iter().any(|(n, _)| n == s))
    );

    // Every slot prefab with a Pickupable has a tech type.
    let catalog = assets.catalog().unwrap();
    let class_paths = game.read_prefab_database().unwrap();
    let ent = sn_assets::ent_tech_data(&assets).unwrap();
    let lookup = sn_assets::TechLookup::new(&ent, class_paths.clone());
    let loot = sn_assets::loot_table(&assets).unwrap();
    let mut keys = std::collections::BTreeSet::new();
    for (_, entries) in loot.distribution.biomes() {
        for e in entries {
            if let Some(p) = class_paths.get(&e.class_id) {
                keys.insert(p.clone());
            }
        }
    }
    let (mut pickups, mut misses, mut outcrops) = (0, Vec::new(), 0);
    for key in &keys {
        let prefab = assets.prefab(&catalog, key).unwrap();
        let item = sn_assets::item_prefab(&assets, &prefab, &lookup).unwrap();
        if item.pickupables > 0 {
            pickups += 1;
            if item.tech_type == 0 {
                misses.push(key.clone());
            }
        }
        let Some(b) = item.breakable else { continue };
        outcrops += 1;
        let rolls = b
            .prefab_list
            .iter()
            .map(|r| (r.prefab.clone(), r.tech_type))
            .chain([(b.default_prefab.clone(), b.default_tech_type)]);
        for (guid, listed) in rolls {
            let p = sn_assets::addressable_prefab(&assets, &catalog, &guid)
                .unwrap()
                .unwrap_or_else(|| panic!("{key}: {guid} does not resolve"));
            let drop = sn_assets::item_prefab(&assets, &p, &lookup).unwrap();
            assert_eq!(drop.tech_type, listed, "{key}: {}", p.key);
        }
    }
    eprintln!(
        "slot prefabs {}, with a Pickupable {pickups}, outcrops {outcrops}, misses {misses:?}",
        keys.len()
    );
    assert!(misses.is_empty(), "{misses:?}");
    assert_eq!((keys.len(), pickups, outcrops), (189, 108, 3));

    // The player's luck, the lifepod's locker and its supplies.
    let p = sn_assets::player_items(&assets).unwrap();
    assert_eq!(p.entropy.len(), 8);
    assert!(
        p.entropy
            .iter()
            .all(|(_, f)| f.entropy == 0.0 && f.time_last_check == -1.0)
    );
    assert_eq!(p.loot.escape_pod.len(), 6);
    assert_eq!(p.loot.supply.len(), 6);
    assert_eq!((p.pod_locker.width, p.pod_locker.height), (4, 8));

    // Text.
    let english = sn_assets::language(&game, "English").unwrap();
    assert_eq!(english.len(), 3818);
    for key in ["PickUpFormat", "InventoryFull", "InventoryCantDropHere"] {
        assert!(english.contains_key(key), "{key}");
    }
}

/// M9d3d: the item rules from the install. Counts as M9d1 found them;
/// every outcrop resource and every supply is a pickup the rules know;
/// two reads give equal rules. Prints the time a read takes.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn item_rules_from_install() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let dll = sn_assets::read_assembly(&game).unwrap();
    let start = std::time::Instant::now();
    let (rules, report) = sn_assets::item_rules(&game, &assets, &dll).unwrap();
    let took = start.elapsed().as_secs_f64();
    let techs: std::collections::BTreeSet<i32> = rules
        .pickups
        .values()
        .map(|p| p.tech)
        .filter(|&t| t != 0)
        .collect();
    let outcrop_paths: std::collections::BTreeSet<(i32, usize, String)> = rules
        .outcrops
        .values()
        .map(|o| (o.hits_to_break, o.list.len(), o.default_class.clone()))
        .collect();
    println!(
        "item rules in {took:.1} s: {report:?}; pickup classes {}, tech types {}; outcrop classes {} ({} distinct); sizes {}, equipment types {}; slots {}, player's {:?}; grid {:?}; analysis {}; default tech {}; locker {:?}; supplies {:?}; luck {}; tech → prefab {}",
        rules.pickups.len(),
        techs.len(),
        rules.outcrops.len(),
        outcrop_paths.len(),
        rules.techs.sizes.len(),
        rules.techs.equipment.len(),
        rules.slots.slots.len(),
        rules.player_slots,
        rules.inventory_size,
        rules.analysis.len(),
        rules.default_tech.len(),
        rules.locker_size,
        rules.supplies,
        rules.luck.len(),
        rules.tech_class.len()
    );
    assert_eq!(rules.inventory_size, (6, 8));
    assert_eq!(rules.slots.slots.len(), 38);
    assert_eq!(rules.player_slots.len(), 7);
    assert_eq!(rules.analysis.len(), 126);
    assert_eq!(rules.locker_size, (4, 8));
    assert_eq!(rules.supplies.len(), 6);
    assert_eq!(rules.luck.len(), 8);
    assert_eq!(outcrop_paths.len(), 4);
    assert!(techs.len() >= 101, "{}", techs.len());
    // Every outcrop resource and every supply is a pickup of the listed
    // tech type.
    for o in rules.outcrops.values() {
        for d in &o.list {
            assert_eq!(rules.pickups[&d.class].tech, d.tech, "{}", d.class);
        }
        assert!(rules.pickups.contains_key(&o.default_class));
    }
    for t in &rules.supplies {
        let class = &rules.tech_class[t];
        assert_eq!(rules.pickups[class].tech, *t);
    }
    // The supplies fit the locker, as in a new game.
    let mut locker = sn_sim::inventory::Container::new(4, 8);
    let items: Vec<_> = rules
        .supplies
        .iter()
        .enumerate()
        .map(|(i, &t)| sn_sim::inventory::Item::new(i as u64, t))
        .collect();
    assert!(sn_sim::inventory::spawn_supplies(&mut locker, &rules.techs, items).is_empty());
    // Stable across reads.
    let (again, _) = sn_assets::item_rules(&game, &assets, &dll).unwrap();
    assert_eq!(again, rules);
}

/// M9d4a: hand targets around Lifepod 5 (lifepod and slot seed 1). Every
/// target's bodies carry its key; a cast at a target hits it; after its
/// key is removed the cast no longer does; a dropped item (a server-made
/// object of a pickup's class) is hit after it appears and not after it
/// goes; a gone key is left out when a batch loads. Counts are printed.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn hand_targets_follow_the_items() {
    use sn_assets::{BatchBodies, CollisionLoader, LayerRules, TargetKind};
    use sn_protocol::{EntityKey, ItemId};
    use sn_sim::V3;
    use sn_sim::collide::{HAND, World};
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let dll = sn_assets::read_assembly(&game).unwrap();
    let (rules, _) = sn_assets::item_rules(&game, &assets, &dll).unwrap();
    let rules = std::sync::Arc::new(rules);
    let (point, _) = sn_assets::lifepod_point(&assets, 1).unwrap();
    let mut loader = CollisionLoader::new(&game, Some(1)).unwrap();
    loader.set_item_rules(rules.clone());
    let data = sn_assets::player_data(loader.assets()).unwrap();
    let settings = sn_assets::physics_settings(loader.assets()).unwrap();
    let layers = LayerRules::new(&settings, data.player_layer);

    let mut batches = BatchBodies::new();
    let mut world = World::new();
    let changes = batches.update(&mut loader, &layers, point).unwrap();
    for (id, b) in changes.add {
        world.insert(id, b);
    }
    let [pickups, outcrops] = batches.target_counts();
    println!(
        "around the pod: {} batches; targets: {pickups} pickups, {outcrops} outcrops (loader: {:?} placed as targets)",
        batches.loaded(),
        loader.stats().targets
    );
    assert!(pickups > 0 && outcrops > 0);
    let pod = loader.lifepod(&layers, point).unwrap();
    let locker = pod.locker.as_ref().expect("the lifepod's locker");
    println!(
        "lifepod locker at {:?}: {} colliders, {} bodies",
        locker.at,
        locker.colliders,
        locker.bodies.len()
    );
    assert!(!locker.bodies.is_empty());

    // Cast down at each target from above its bodies; count those hit
    // first (others may sit under rock or another object).
    let ids: Vec<u64> = world.ids().collect();
    let cast_down = |world: &World, body: u64| -> Option<u64> {
        let (lo, hi) = world_bounds(world, body)?;
        let c = V3::lerp(lo, hi, 0.5);
        let from = V3::new(c.x, hi.y + 1.0, c.z);
        world
            .cast(HAND, from, V3::new(0.0, -1.0, 0.0), 0.0, hi.y - lo.y + 2.0)
            .map(|(_, h)| h.body)
    };
    let mut hit_self = Vec::new();
    for &id in &ids {
        let Some(t) = batches.target(id) else {
            continue;
        };
        if cast_down(&world, id) == Some(id) {
            hit_self.push((id, t.key.clone(), t.kind));
        }
    }
    let n_pick = hit_self
        .iter()
        .filter(|h| h.2 == TargetKind::Pickup)
        .count();
    println!(
        "targets hit by a cast from above: {} ({n_pick} pickups)",
        hit_self.len()
    );
    // An outcrop near the pod, for the client's hand check (M9d5).
    if let Some((ob, okey, _)) = hit_self.iter().find(|h| h.2 == TargetKind::Outcrop) {
        let t = batches.target(*ob).unwrap();
        println!("an outcrop {okey:?} ({}) at {:?}", t.class, t.at.position);
    }
    let (body, key, _) = hit_self
        .iter()
        .find(|h| h.2 == TargetKind::Pickup)
        .cloned()
        .expect("a pickup the hand can reach from above");
    let class = batches.target(body).unwrap().class.clone();
    println!(
        "chosen pickup {key:?} ({class}) at {:?}",
        batches.target(body).unwrap().at.position
    );
    // Removed: its bodies go, the cast no longer finds it.
    let before = world_bounds(&world, body).unwrap();
    let removed = batches.remove_key(&key);
    assert!(removed.contains(&body));
    for id in &removed {
        world.remove(*id);
    }
    let c = V3::lerp(before.0, before.1, 0.5);
    let again = world
        .cast(
            HAND,
            V3::new(c.x, before.1.y + 1.0, c.z),
            V3::new(0.0, -1.0, 0.0),
            0.0,
            before.1.y - before.0.y + 2.0,
        )
        .map(|(_, h)| h.body);
    assert!(again.is_none_or(|b| batches.target(b).is_none_or(|t| t.key != key)));
    assert!(batches.target(body).is_none());

    // Dropped (a server-made object of the same class, at the same
    // place): hit; then gone again.
    let at = before.0;
    let drop = ItemId(7);
    let c = batches.sync_spawned(
        &mut loader,
        &layers,
        [(drop, class.as_str(), at.to_f32(), [0, 0, 0, 32_767])],
    );
    assert!(!c.add.is_empty());
    for (id, b) in c.add {
        world.insert(id, b);
    }
    let spawned: Vec<u64> = world
        .ids()
        .filter(|id| {
            batches
                .target(*id)
                .is_some_and(|t| t.key == EntityKey::Spawned(drop))
        })
        .collect();
    assert!(!spawned.is_empty());
    assert!(spawned.iter().any(|&b| cast_down(&world, b) == Some(b)));
    let c = batches.sync_spawned(&mut loader, &layers, []);
    assert_eq!(c.remove.len(), spawned.len());
    for id in c.remove {
        world.remove(id);
    }

    // The cost: player-sized sweeps among the bodies, with targets and
    // without (a loader with no item rules), same batches.
    let capsule = sn_sim::collide::Capsule::unity(0.3, 1.8, 1, V3::ZERO);
    let time_sweeps = |world: &World| {
        let start = std::time::Instant::now();
        let mut hits = 0;
        for i in 0..20_000 {
            let a = i as f64 * 0.37;
            let from = V3::new(
                f64::from(point[0]) + 30.0 * a.cos(),
                -6.0,
                f64::from(point[2]) + 30.0 * a.sin(),
            );
            let d = V3::new(a.sin(), 0.1, -a.cos());
            hits += usize::from(world.sweep(&capsule, from, d).is_some());
        }
        (start.elapsed().as_secs_f64() * 1e6 / 20_000.0, hits)
    };
    let (with, hits_with) = time_sweeps(&world);
    let mut plain_loader = CollisionLoader::new(&game, Some(1)).unwrap();
    let mut plain = World::new();
    for (id, b) in BatchBodies::new()
        .update(&mut plain_loader, &layers, point)
        .unwrap()
        .add
    {
        plain.insert(id, b);
    }
    let (without, hits_without) = time_sweeps(&plain);
    println!(
        "sweeps: {with:.2} µs each with targets ({} bodies), {without:.2} µs without ({} bodies); hits {hits_with} / {hits_without}",
        world.len(),
        plain.len()
    );

    // A fresh load leaves the gone key out.
    let mut fresh = BatchBodies::new();
    let c = fresh
        .update_with(&mut loader, &layers, point, &|k| *k == key)
        .unwrap();
    assert_eq!(c.gone, 1);
    assert_eq!(fresh.target_counts(), [pickups - 1, outcrops]);
}

/// A body's bounds in the world.
fn world_bounds(world: &sn_sim::collide::World, id: u64) -> Option<(sn_sim::V3, sn_sim::V3)> {
    world.get(id)?.bounds()
}

/// M9u0: the UI hierarchies the game loads (`docs/formats/ui.md`).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn ui_trees_from_the_install() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ugui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    assert_eq!(ugui.nodes.len(), 361);
    assert_eq!(ugui.nodes[0].name, "uGUI");
    assert_eq!(ugui.class_counts().get(&223), Some(&12)); // Canvas
    assert_eq!(ugui.scripts("uGUI_HandReticleIcon").count(), 10);
    assert_eq!(ugui.scripts("uGUI_CanvasScaler").count(), 4);
    let (bar, _) = ugui.scripts("uGUI_OxygenBar").next().unwrap();
    assert_eq!(
        ugui.nodes[bar].path,
        "uGUI/ScreenCanvas/HUD/Content/BarsPanel/OxygenBar"
    );
    let main = assets.scene("main").unwrap();
    let (screen, anchor) = assets.pda_screen_refs(&main).unwrap();
    assert!(anchor.is_some());
    let pda = assets.ui_tree("PDA screen", &screen).unwrap();
    assert_eq!(pda.nodes.len(), 402);
    assert_eq!(pda.nodes[0].name, "uGUI_PDAScreen");
    assert_eq!(pda.scripts("uGUI_ItemsContainer").count(), 6);
    assert_eq!(pda.scripts("uGUI_Equipment").count(), 1);
}

/// M9u1: the layouts built from the game's assemblies read the same
/// values as the hand-written readers, on the objects those read.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn script_layouts_agree_with_hand_written_readers() {
    use sn_assets::{ObjectRef, ScriptAssemblies};
    use sn_unity::{
        CanvasScaler, Fields, LiveMixin, LiveMixinData, MainCameraControl, Oxygen, PdaData, Value,
    };

    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let generic = |o: &ObjectRef| -> Fields {
        let s = assets.mono_script(o).unwrap();
        let l = layouts
            .script(&s.assembly, &s.namespace, &s.class_name)
            .unwrap();
        let (_, d) = o.data().unwrap();
        sn_unity::read_script(d, false, &l).unwrap().1
    };
    let f32_of = |f: &Fields, n: &str| f.path(n).and_then(Value::as_f64).unwrap() as f32;
    let bool_of = |f: &Fields, n: &str| f.path(n).and_then(Value::as_bool).unwrap();
    let pptr_of = |f: &Fields, n: &str| f.path(n).and_then(Value::as_pptr).unwrap();

    let main = assets.scene("main").unwrap();
    let mut compared = 0;
    for o in main.behaviours(&assets, "Oxygen") {
        let hand = Oxygen::parse(o.data().unwrap().1, false).unwrap();
        let g = generic(&o);
        assert_eq!(f32_of(&g, "oxygenCapacity"), hand.oxygen_capacity);
        assert_eq!(bool_of(&g, "isPlayer"), hand.is_player);
        compared += 1;
    }
    for o in main.behaviours(&assets, "MainCameraControl") {
        let h = MainCameraControl::parse(o.data().unwrap().1, false).unwrap();
        let g = generic(&o);
        for (name, value) in [
            ("camPDAZOffset", h.cam_pda_z_offset),
            ("stepAmount", h.step_amount),
            ("minimumX", h.minimum_x),
            ("maximumX", h.maximum_x),
            ("minimumY", h.minimum_y),
            ("maximumY", h.maximum_y),
            ("skin", h.skin),
            ("maxViewModelRotation", h.max_view_model_rotation),
            ("maxViewModelMovement", h.max_view_model_movement),
            ("cameraTiltMod", h.camera_tilt_mod),
        ] {
            assert_eq!(f32_of(&g, name), value, "MainCameraControl.{name}");
        }
        assert_eq!(bool_of(&g, "mouseLookEnabled"), h.mouse_look_enabled);
        assert_eq!(pptr_of(&g, "viewModel"), h.view_model);
        assert_eq!(
            pptr_of(&g, "cameraOffsetTransform"),
            h.camera_offset_transform
        );
        assert_eq!(pptr_of(&g, "cameraUPTransform"), h.camera_up_transform);
        compared += 1;
    }
    for o in main.behaviours(&assets, "LiveMixin") {
        let h = LiveMixin::parse(o.data().unwrap().1, false).unwrap();
        let g = generic(&o);
        assert_eq!(pptr_of(&g, "data"), h.data);
        assert_eq!(f32_of(&g, "health"), h.health);
        compared += 1;
        let Some(data) = assets.resolve(&o.file, h.data).unwrap() else {
            continue;
        };
        let hd = LiveMixinData::parse(data.data().unwrap().1, false).unwrap();
        let gd = generic(&data);
        assert_eq!(f32_of(&gd, "maxHealth"), hd.max_health);
        assert_eq!(f32_of(&gd, "minDamageForSound"), hd.min_damage_for_sound);
        assert_eq!(
            f32_of(&gd, "loopEffectBelowPercent"),
            hd.loop_effect_below_percent
        );
        assert_eq!(bool_of(&gd, "destroyOnDeath"), hd.destroy_on_death);
        assert_eq!(bool_of(&gd, "weldable"), hd.weldable);
        assert_eq!(bool_of(&gd, "knifeable"), hd.knifeable);
        assert_eq!(bool_of(&gd, "canResurrect"), hd.can_resurrect);
        assert_eq!(
            bool_of(&gd, "invincibleInCreative"),
            hd.invincible_in_creative
        );
        compared += 1;
    }
    // PDAData, through the player's field.
    let player = &main.behaviours(&assets, "Player")[0];
    let pda_ref = pptr_of(&generic(player), "pdaData");
    let pda = assets.resolve(&player.file, pda_ref).unwrap().unwrap();
    let h = PdaData::parse(pda.data().unwrap().1, false).unwrap();
    let g = generic(&pda);
    let list = |n: &str| g.get(n).and_then(Value::as_array).unwrap();
    let default_tech: Vec<i32> = list("defaultTech")
        .iter()
        .map(|v| v.as_i64().unwrap() as i32)
        .collect();
    assert_eq!(default_tech, h.default_tech);
    assert_eq!(list("log").len(), h.log.len());
    assert_eq!(list("encyclopedia").len(), h.encyclopedia.len());
    assert_eq!(list("scanner").len(), h.scanner.len());
    assert_eq!(list("analysisTech").len(), h.analysis_tech.len());
    assert_eq!(list("compoundTech").len(), h.compound_tech.len());
    compared += 1;
    // The canvas scalers of the UI.
    let catalog = assets.catalog().unwrap();
    let ugui = assets
        .ui_tree("uGUI.prefab", &assets.ugui_root(&catalog).unwrap())
        .unwrap();
    for (_, c) in ugui.scripts("uGUI_CanvasScaler") {
        let h = CanvasScaler::parse(c.object.data().unwrap().1, false).unwrap();
        let g = generic(&c.object);
        assert_eq!(f32_of(&g, "distance"), h.distance);
        assert_eq!(
            f32_of(&g, "referenceResolution.x"),
            h.reference_resolution[0]
        );
        assert_eq!(
            f32_of(&g, "referenceResolution.y"),
            h.reference_resolution[1]
        );
        for (name, v) in [
            ("mode", h.mode),
            ("vrMode", h.vr_mode),
            ("scaleMode", h.scale_mode),
        ] {
            assert_eq!(
                g.get(name).and_then(Value::as_i64),
                Some(i64::from(v)),
                "{name}"
            );
        }
        compared += 1;
    }
    eprintln!("{compared} objects compared");
    assert!(compared >= 9, "{compared}");
}

/// M9u2: the sprite atlases `SpriteManager` loads and its lookup rule.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn sprites_by_the_games_rule() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let sprites = assets.sprites(&catalog).unwrap();
    assert_eq!(sprites.atlases.len(), 8);
    assert!(sprites.misses.is_empty(), "{:?}", sprites.misses);
    assert_eq!(sprites.atlases["Items"].sprites.len(), 470);
    let unknown = sprites.default_sprite().unwrap();
    assert_eq!(unknown.sprite.name, "unknown");
    // Its own icon (the lookup ignores case), and a fallback.
    let titanium = sprites.item_icon("Titanium").unwrap();
    assert_eq!(titanium.sprite.name, "titanium"); // the tech type is `Titanium`
    assert_eq!(titanium.sprite.rect, [0.0, 0.0, 128.0, 128.0]);
    let drillable = sprites.item_icon("DrillableTitanium").unwrap();
    assert_eq!(drillable.sprite.name, "unknown");
    let (w, h, px) = assets.sprite_pixels(titanium).unwrap();
    assert!(w > 0 && w <= 128 && h > 0 && h <= 128, "{w}×{h}");
    assert_eq!(px.len(), w * h * 4);
    assert!(px.chunks_exact(4).any(|p| p[3] > 0));
}

/// M9u3: TMP's font assets remade from the font files. Face metrics (but
/// the hinted cap and mean lines) equal the stored ones; `Aller_Rg SDF`'s
/// 2 stored glyphs equal ours in metrics and place; English needs no
/// glyph outside the fonts.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn fonts_remade_from_the_font_files() {
    use sn_ui::font::{Added, DynamicAtlas, FontFile};
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let fonts = assets.font_assets(&assemblies).unwrap();
    assert_eq!(fonts.fonts.len(), 9);
    assert_eq!(fonts.assets.len(), 8);
    assert!(fonts.settings.is_some());
    let hinted = ["m_CapLine", "m_MeanLine", "m_StrikethroughOffset"];
    for a in &fonts.assets {
        let s = a.stored().unwrap();
        assert_eq!((s.population_mode, s.render_mode), (1, 4165), "{}", a.name);
        let font = fonts.font(a.source.as_ref().unwrap()).unwrap();
        let file = FontFile::parse(&font.font.data).unwrap();
        let ours = file.face_info(s.point_size);
        assert_eq!(ours.family_name, s.face.family_name);
        assert_eq!(ours.style_name, s.face.style_name);
        for ((name, want), (_, got)) in s.face.numbers().iter().zip(ours.numbers()) {
            if !hinted.contains(name) {
                assert!(
                    (want - got).abs() <= 0.01,
                    "{} {name}: {got} vs {want}",
                    a.name
                );
            }
        }
    }
    let aller = fonts.asset("Aller_Rg SDF").unwrap();
    let s = aller.stored().unwrap();
    assert_eq!(s.glyphs.len(), 2);
    let font = fonts.font(aller.source.as_ref().unwrap()).unwrap();
    let file = FontFile::parse(&font.font.data).unwrap();
    let mut atlas = DynamicAtlas::new(s.atlas_width, s.atlas_height, s.padding, s.point_size);
    for &(unicode, index) in &s.characters {
        let Added::New(g) = atlas.add_character(&file, char::from_u32(unicode).unwrap()) else {
            panic!("U+{unicode:04X} not added");
        };
        let want = s.glyphs.iter().find(|g| g.index == index).unwrap();
        assert_eq!(g, *want, "U+{unicode:04X}");
    }
    assert_eq!(atlas.packer.used, s.used_rects);
    let mut free = atlas.packer.free.clone();
    let mut want = s.free_rects.clone();
    free.sort();
    want.sort();
    assert_eq!(free, want);
    let english = sn_assets::language(&game, "English").unwrap();
    let chain: Vec<FontFile> = ["Aller_Rg", "PTSans", "NotoSansCJKsc-Regular"]
        .iter()
        .map(|n| {
            let f = fonts.fonts.iter().find(|f| f.font.name == *n).unwrap();
            FontFile::parse(&f.font.data).unwrap()
        })
        .collect();
    let missing = english
        .values()
        .flat_map(|v| v.chars())
        .filter(|c| !c.is_control())
        .filter(|&c| chain.iter().all(|f| f.glyph_index(c).is_none()))
        .count();
    assert_eq!(missing, 0);
}

/// M9u4a: Unity saves driven RectTransform values as 0. Every value our
/// layout drives in `uGUI.prefab` and the PDA screen is stored as 0 (so
/// our driven sets are the game's), and a second run moves nothing.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn layout_drives_only_values_saved_as_zero() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let main = assets.scene("main").unwrap();
    let (screen, _) = assets.pda_screen_refs(&main).unwrap();
    let mut driven = 0;
    for (key, r) in [("uGUI", &root), ("PDA", &screen)] {
        let ui = assets.ui_tree(key, r).unwrap();
        let mut tree = assets.ui_layout(&layouts, &ui).unwrap().tree;
        let stored: Vec<_> = tree.nodes.iter().map(|n| n.rect).collect();
        let report = sn_ui::layout::layout(&mut tree);
        for (i, d) in report.driven.iter().enumerate() {
            let s = &stored[i];
            if d.anchors {
                driven += 4;
                assert_eq!(
                    (s.anchor_min, s.anchor_max),
                    ([0.0; 2], [0.0; 2]),
                    "{}",
                    ui.nodes[i].path
                );
            }
            for a in 0..2 {
                if d.position[a] {
                    driven += 1;
                    assert_eq!(s.anchored_position[a], 0.0, "{}", ui.nodes[i].path);
                }
                if d.size[a] {
                    driven += 1;
                    assert_eq!(s.size_delta[a], 0.0, "{}", ui.nodes[i].path);
                }
            }
        }
        let first: Vec<_> = tree.nodes.iter().map(|n| n.rect).collect();
        sn_ui::layout::layout(&mut tree);
        for (n, f) in tree.nodes.iter().zip(&first) {
            for (a, b) in n.rect.size_delta.iter().zip(&f.size_delta) {
                assert!(a.to_bits() == b.to_bits() || (a - b).abs() < 0.01);
            }
        }
    }
    // uGUI 198 + PDA 129 of the 370 (the other roots hold the rest).
    assert_eq!(driven, 327);
}

/// M9u4b: the meshes of `uGUI.prefab`'s and the PDA screen's images, on
/// their laid-out rects: every vertex inside its rect, indices valid.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn ui_image_meshes_stay_in_their_rects() {
    use sn_assets::UiGraphicKind;
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let main = assets.scene("main").unwrap();
    let (screen, _) = assets.pda_screen_refs(&main).unwrap();
    let mut built = 0;
    for (key, r) in [("uGUI", &root), ("PDA", &screen)] {
        let ui = assets.ui_tree(key, r).unwrap();
        let mut tree = assets.ui_layout(&layouts, &ui).unwrap().tree;
        sn_ui::layout::layout(&mut tree);
        for g in assets.ui_graphics(&layouts, &ui).unwrap() {
            let UiGraphicKind::Image { params, .. } = &g.kind else {
                continue;
            };
            let rect = tree.rect(g.node);
            if !g.enabled
                || !tree.active(g.node)
                || !rect.width.is_finite()
                || !rect.height.is_finite()
            {
                continue;
            }
            let pivot = tree.nodes[g.node].rect.pivot;
            let (_, m) = sn_ui::mesh::image_mesh(rect, pivot, params, 100.0);
            for v in &m.vertices {
                assert!(
                    v.pos[0] >= rect.x - 1e-3 && v.pos[0] <= rect.x + rect.width + 1e-3,
                    "{}",
                    ui.nodes[g.node].path
                );
                assert!(
                    v.pos[1] >= rect.y - 1e-3 && v.pos[1] <= rect.y + rect.height + 1e-3,
                    "{}",
                    ui.nodes[g.node].path
                );
            }
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
            built += 1;
        }
    }
    // Images of the two trees whose rects do not wait for text (the
    // other roots and RawImages are not counted here).
    assert!(built > 200, "{built}");
}

/// M9u4c: TMP's auto-size, ported, picks the size the editor stored for
/// every auto-sized text of `uGUI.prefab` and the PDA screen (laid out
/// in their rects, text sizes included).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn auto_sized_texts_match_the_stored_sizes() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let fonts = assets.font_assets(&assemblies).unwrap();
    let set = assets.tmp_font_set(&fonts).unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let main = assets.scene("main").unwrap();
    let (screen, _) = assets.pda_screen_refs(&main).unwrap();
    let mut compared = 0;
    for (key, r) in [("uGUI", &root), ("PDA", &screen)] {
        let ui = assets.ui_tree(key, r).unwrap();
        let mut tree = assets.ui_layout(&layouts, &ui).unwrap().tree;
        let texts = assets.ui_texts(&layouts, &ui, &fonts).unwrap();
        let sizer = sn_ui::text::Texts {
            fonts: &set,
            params: texts.iter().map(|t| (t.node, t.params.clone())).collect(),
        };
        sn_ui::layout::layout_with(&mut tree, &sizer);
        for t in texts.iter().filter(|t| t.params.auto_size) {
            let l = sn_ui::text::layout_text(&set, &t.params, tree.rect(t.node), 0.0);
            assert_eq!(l.font_size, t.stored_font_size, "{}", ui.nodes[t.node].path);
            compared += 1;
        }
    }
    assert_eq!(compared, 20);
}

/// M9u4d: the PDA screen's draw items, built twice, are the same, and
/// its rect masks clip some of them.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_pda_screen_draw_list_is_stable() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let tags = sn_assets::physics_settings(&assets).unwrap().tags;
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let main = assets.scene("main").unwrap();
    let (screen, _) = assets.pda_screen_refs(&main).unwrap();
    let ui = assets.ui_tree("PDA", &screen).unwrap();
    let build = || {
        let parts = assets
            .ui_draw_parts(&layouts, &ui, &tags.sorting_layers)
            .unwrap();
        let mut tree = assets.ui_layout(&layouts, &ui).unwrap().tree;
        sn_ui::layout::layout(&mut tree);
        let mut graphics = Vec::new();
        for g in assets.ui_graphics(&layouts, &ui).unwrap() {
            let rect = tree.rect(g.node);
            let sn_assets::UiGraphicKind::Image { params, .. } = &g.kind else {
                continue;
            };
            if !g.enabled || !rect.width.is_finite() || !rect.height.is_finite() {
                continue;
            }
            let (_, mesh) =
                sn_ui::mesh::image_mesh(rect, tree.nodes[g.node].rect.pivot, params, 100.0);
            graphics.push(sn_ui::draw::Graphic {
                node: g.node,
                mesh,
                texture: 0,
                material: 0,
                maskable: parts.maskable.get(&g.node).copied().unwrap_or(true),
                cull_transparent: false,
                alpha: 1.0,
                kind: "Image".into(),
            });
        }
        let input = sn_ui::draw::DrawInput {
            canvases: parts.canvases,
            groups: parts.groups,
            masks: parts.masks,
            graphics,
        };
        sn_ui::draw::draw_list(&tree, &input)
    };
    let a = build();
    let b = build();
    assert_eq!(a.hash(), b.hash());
    assert!(a.items.len() > 100, "{}", a.items.len());
    assert!(a.items.iter().any(|i| i.clip.is_some()));
}

/// M9u5a: the HUD's draw list with materials and text vertices: every
/// item has a material, every text quad's UVs lie in its font atlas, and
/// two builds from scratch are the same.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_hud_draws_with_materials_and_glyphs() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let build = || {
        let assets = Assets::index(&game).unwrap();
        let tags = sn_assets::physics_settings(&assets).unwrap().tags;
        let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
        let layouts = assemblies.layouts().unwrap();
        let fonts = assets.font_assets(&assemblies).unwrap();
        let set = assets.tmp_font_set(&fonts).unwrap();
        let mut atlases = sn_assets::GlyphAtlases::new(&fonts).unwrap();
        let catalog = assets.catalog().unwrap();
        let root = assets.ugui_root(&catalog).unwrap();
        let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
        let d = assets
            .ui_draw(
                &layouts,
                &ui,
                &fonts,
                &set,
                &mut atlases,
                &tags.sorting_layers,
            )
            .unwrap();
        (d, atlases.made, atlases.full)
    };
    let (a, made, full) = build();
    let (b, _, _) = build();
    assert_eq!(a.list.hash(), b.list.hash());
    assert!(made > 50, "{made}");
    assert_eq!(full, 0);
    assert!(a.list.items.iter().all(|i| i.material < a.materials.len()));
    let mut quads = 0;
    for it in a.list.items.iter().filter(|i| i.kind.starts_with("Text")) {
        assert_eq!(it.texture >> 40, 1, "{}", it.kind);
        for v in &it.vertices {
            assert!((0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]));
            assert!(v.uv1[1].is_finite() && v.uv1[1] != 0.0);
        }
        quads += it.vertices.len() / 4;
        let m = &a.materials[it.material];
        assert_eq!(m.shader, "TextMeshPro/Distance Field");
    }
    assert!(quads > 300, "{quads}");
}

/// M9u5b: `uGUI`'s four scalers are Static; placed on a 2560 × 1440 screen
/// with the game's UI camera, each canvas's corners land on the screen's.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_hud_canvases_fill_the_screen() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let scalers = assets.ui_scalers(&ui).unwrap();
    assert_eq!(scalers.len(), 4);
    let cam = sn_ui::scaler::UiCamera::game(60.0, 0.03, 1700.0);
    for (_, s) in &scalers {
        assert_eq!(s.mode, sn_ui::scaler::MODE_STATIC);
        let p = sn_ui::scaler::place(s, &cam, 60.0, [2560, 1440], 1.0).unwrap();
        let c = p.corners([0.5, 0.5]);
        let tr = cam.project(c[2], [2560, 1440]).unwrap();
        let bl = cam.project(c[0], [2560, 1440]).unwrap();
        assert!(
            (tr[0] - 2560.0).abs() < 0.5 && (tr[1] - 1440.0).abs() < 0.5,
            "{tr:?}"
        );
        assert!(bl[0].abs() < 0.5 && bl[1].abs() < 0.5, "{bl:?}");
    }
}

/// M9u6a: `uGUI` read once into a scene draws its start state with the
/// one-shot path's hash; a state with a changed text and a hidden node
/// changes the draw list, and the start state drawn again gives the start
/// hash back.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_live_hud_returns_to_its_start_state() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let tags = sn_assets::physics_settings(&assets).unwrap().tags;
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let fonts = assets.font_assets(&assemblies).unwrap();
    let set = assets.tmp_font_set(&fonts).unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let start = assets.ui_start_state(&layouts, &ui).unwrap();
    let none = std::collections::HashMap::new();
    let mut atlases = sn_assets::GlyphAtlases::new(&fonts).unwrap();
    let one_shot = assets
        .ui_draw_placed(
            &layouts,
            &ui,
            &fonts,
            &set,
            &mut atlases,
            &tags.sorting_layers,
            &none,
            &start,
        )
        .unwrap()
        .list
        .hash();
    let mut scene = assets
        .ui_scene(&layouts, &ui, &fonts, &tags.sorting_layers)
        .unwrap();
    let state = sn_assets::UiState::from(&start);
    let mut atlases = sn_assets::GlyphAtlases::new(&fonts).unwrap();
    let mut draw = |s: &sn_assets::UiState| {
        scene
            .draw(&assets, &fonts, &set, &mut atlases, &none, s)
            .unwrap()
            .list
    };
    let base = draw(&state);
    assert_eq!(base.hash(), one_shot);
    let node = |path: &str| ui.nodes.iter().position(|n| n.path == path).unwrap();
    let text = node("uGUI/ScreenCanvas/HUD/Content/BarsPanel/OxygenBar/Icon/Text");
    let quad = node("uGUI/ScreenCanvas/HUD/Content/BarsPanel/BackgroundQuad");
    let mut changed = state.clone();
    changed.text.insert(text, "1234".into());
    changed.active.insert(quad, false);
    let d = draw(&changed);
    assert_ne!(d.hash(), base.hash());
    assert!(d.items.iter().all(|i| i.node != quad));
    let t: Vec<_> = d.items.iter().filter(|i| i.node == text).collect();
    assert_eq!(t.first().map(|i| i.vertices.len()), Some(16));
    assert_eq!(draw(&state).hash(), base.hash());
}

/// M9u6b1: `uGUI_SceneHUD`'s four bars are read with their circular bars
/// and texts (only oxygen has overlays), and `Survival.ResetStats`' food
/// and water come from the code.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_hud_bars_are_read() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let c = assets.hud_config(&layouts, &ui).unwrap();
    assert_eq!(c.bars.len(), 4);
    for b in &c.bars {
        assert!(ui.nodes[b.bar].path.starts_with(&ui.nodes[b.root].path));
        assert!(ui.nodes[b.text].path.starts_with(&ui.nodes[b.root].path));
        assert_eq!(b.damp_speed, 0.1);
    }
    assert!(c.bars[0].overlays.is_some());
    assert!(c.bars[1..].iter().all(|b| b.overlays.is_none()));
    let code = sn_assets::read_assembly(&game).unwrap();
    assert_eq!(sn_assets::survival_start(&code).unwrap(), (50.5, 90.5));
}

/// M9u6c1: `uGUI_DepthCompass`'s nodes, sprites, colour and strings are
/// read: the player texts and images under `PlayerDepth`, three distinct
/// sprites, 8 compass labels, and a suffix from the language file; and
/// (M9u6c2) the compass ring's labels against their stored places.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_depth_compass_is_read() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let tags = sn_assets::physics_settings(&assets).unwrap().tags;
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let fonts = assets.font_assets(&assemblies).unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let mut scene = assets
        .ui_scene(&layouts, &ui, &fonts, &tags.sorting_layers)
        .unwrap();
    let language = sn_assets::language(&game, "English").unwrap();
    let c = assets
        .depth_config(&layouts, &ui, &mut scene, &language)
        .unwrap();
    for n in [c.shadow, c.half_moon, c.depth_text, c.suffix_text] {
        assert!(ui.nodes[n].path.contains("/DepthCompass/PlayerDepth/"));
    }
    let sprites = [
        c.shadow_normal,
        c.half_moon_normal,
        c.half_moon_compass_normal,
    ];
    assert!(sprites[0] != sprites[1] && sprites[1] != sprites[2]);
    assert_eq!(c.labels.len(), 8);
    assert_eq!(c.text_color_normal[3], 255);
    assert_eq!(Some(&c.meter_suffix), language.get("MeterSuffix"));
    // M9u6c2: the labels' stored places are what the editor last computed
    // at the stored `debugDirection` (0.646): ours within 0.001 px.
    assert_eq!(c.ring.labels.len(), 8);
    let layout = assets.ui_layout(&layouts, &ui).unwrap();
    for l in &c.ring.labels {
        let stored = layout.tree.nodes[l.node].rect.anchored_position;
        let (ours, _) = c.ring.label(l.angle, 0.646);
        let e = (ours[0] - stored[0]).hypot(ours[1] - stored[1]);
        assert!(e <= 0.001, "{}: {ours:?} vs {stored:?}", l.name);
    }
    assert_eq!(c.ring.mesh().vertices.len(), 4);
}

/// M9u6e1: the death, respawn and overlay screens are read: the vignette's
/// numbers from the IL, the respawn fader's fields, the out-of-oxygen
/// overlay's curve (0 at 0, 1 from 0.8 on) and `Loading` from the
/// language file.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_death_screens_are_read() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let language = sn_assets::language(&game, "English").unwrap();
    let code = sn_assets::read_assembly(&game).unwrap();
    let c = assets
        .death_config(&layouts, &ui, &code, &language)
        .unwrap();
    let v = c.vignette;
    assert_eq!(
        (v.begin_fade_delay, v.fade_out_delay, v.rate_in, v.rate_out),
        (0.0, 6.0, 5.0, 2.0)
    );
    assert_eq!((v.full, v.clear), (0.98, 0.02));
    assert!(
        ui.nodes[c.black_overlay]
            .path
            .contains("/PlayerDeathVignette/")
    );
    assert!(ui.nodes[c.loading_text].path.contains("/Respawning/"));
    let f = c.loading_background;
    assert_eq!((f.fade_in_time, f.fade_out_time), (1.0, 0.5));
    assert!(f.use_unscaled_time && !f.initial_state);
    assert_eq!(Some(&c.loading), language.get("Loading"));
    assert_eq!(c.overlays.len(), 1);
    let curve = &c.overlays[0].curve;
    assert!(
        ui.nodes[c.overlays[0].graphic]
            .path
            .ends_with("/OutOfOxygen")
    );
    assert_eq!(curve.evaluate(0.0), 0.0);
    assert!((curve.evaluate(0.8) - 1.0).abs() < 1e-6);
    assert_eq!(curve.evaluate(1.0), 1.0);
}

/// M9u7b: `ErrorMessage`'s canvas, stored fields, the times from its
/// constructor and `OnLateUpdate`'s constants, and its template (one
/// text node, white) with the preferred height of a one-line message.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_messages_are_read() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let tags = sn_assets::physics_settings(&assets).unwrap().tags;
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let fonts = assets.font_assets(&assemblies).unwrap();
    let set = assets.tmp_font_set(&fonts).unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let code = sn_assets::read_assembly(&game).unwrap();
    let (c, template) = assets
        .messages_config(&layouts, &ui, &code, &fonts, &tags.sorting_layers)
        .unwrap();
    assert!(ui.nodes[c.canvas].path.ends_with("/ErrorMessageCanvas"));
    assert_eq!((c.offset, c.y_spacing), ([140.0, 140.0], 10.0));
    let t = c.times;
    assert_eq!(
        (
            t.fly_in,
            t.delay,
            t.fade_out,
            t.invisible,
            t.slide_rate,
            t.snap
        ),
        (0.3, 5.0, 0.6, 0.1, 7.0, 1.0)
    );
    assert_eq!(template.node_count(), 1);
    assert_eq!(c.font_color, [1.0; 4]);
    let [w, h] = template
        .preferred_size(&set, 0, "Inventory full", 500.0)
        .unwrap();
    assert!(w > 0.0 && w < 500.0 && h > 20.0 && h < 40.0, "{w} {h}");
}

/// M9u7c: `uGUI_IconNotifier`'s fields (its canvas, 0.3 s, 64, two
/// animators of 0.7 s), `iconMaterial` (`UI/Icon`'s defaults, no keyword)
/// and an item sprite's mesh as `uGUI_Icon` draws it (its UVs inside the
/// packed rect, to a pixel).
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_icon_notifier_is_read() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let c = assets.notifier_config(&layouts, &ui).unwrap();
    assert!(ui.nodes[c.canvas].path.ends_with("/IconNotifications"));
    assert_eq!((c.interval, c.icon_size), (0.3, 64.0));
    assert_eq!(c.animators.len(), 2);
    for a in &c.animators {
        assert!((a.duration - 0.7).abs() < 1e-6);
        assert!(a.oscillate && a.osc_scale == 4.0);
        assert_eq!((a.x.evaluate(0.0), a.alpha.evaluate(0.0)), (0.5, 0.0));
    }
    let m = assets.icon_material(&catalog, &layouts).unwrap();
    assert_eq!(m.shader, sn_ui::icon::SHADER);
    assert!(m.keywords.is_empty());
    assert_eq!((m.float("_SrcFactor"), m.float("_DstFactor")), (5.0, 10.0));
    let sprites = assets.sprites(&catalog).unwrap();
    let s = sprites.item_icon("Titanium").unwrap();
    let (mesh, page) = assets.icon_sprite(&s.object).unwrap();
    assert_eq!((mesh.size, mesh.pixels_per_unit), ([128.0, 128.0], 100.0));
    assert!(mesh.vertices.len() >= 3 && mesh.triangles.len() % 3 == 0);
    assert_eq!(mesh.uv.len(), mesh.vertices.len());
    assert!(page.is_some());
    let r = s.packed.texture_rect;
    let (_, td) = page.as_ref().unwrap().data().unwrap();
    let t = sn_unity::Texture2D::parse(td, false).unwrap();
    for uv in &mesh.uv {
        let px = [uv[0] * t.width as f32, uv[1] * t.height as f32];
        assert!(
            px[0] > r[0] - 1.0 && px[0] < r[0] + r[2] + 1.0,
            "{px:?} {r:?}"
        );
        assert!(
            px[1] > r[1] - 1.0 && px[1] < r[1] + r[3] + 1.0,
            "{px:?} {r:?}"
        );
    }
}

/// M9u6s: the start rules hide the screens the game hides at start (the
/// feedback panel, the confirmation dialog, pop-ups, the vehicles' and
/// cameras' screens), and the HUD's bars, player depth and the reticle's
/// `Default` icon stay drawn.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_start_state_hides_what_the_game_hides() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let tags = sn_assets::physics_settings(&assets).unwrap().tags;
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let fonts = assets.font_assets(&assemblies).unwrap();
    let set = assets.tmp_font_set(&fonts).unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let start = assets.ui_start_state(&layouts, &ui).unwrap();
    let mut atlases = sn_assets::GlyphAtlases::new(&fonts).unwrap();
    let d = assets
        .ui_draw_placed(
            &layouts,
            &ui,
            &fonts,
            &set,
            &mut atlases,
            &tags.sorting_layers,
            &std::collections::HashMap::new(),
            &start,
        )
        .unwrap();
    let visible: Vec<&str> = d
        .list
        .items
        .iter()
        .filter(|i| i.alpha > 0.0 && i.vertices.iter().any(|v| v.color[3] > 0))
        .map(|i| ui.nodes[i.node].path.as_str())
        .collect();
    for hidden in [
        "uGUI/ScreenCanvas/FeedbackCollector/",
        "uGUI/ScreenCanvas/Confirmation/",
        "uGUI/ScreenCanvas/CriticalWarning/",
        "uGUI/ScreenCanvas/HUD/Content/PopupNotification/",
        "uGUI/ScreenCanvas/HUD/Content/CameraCyclops/",
        "uGUI/ScreenCanvas/HUD/Content/Seamoth/",
        "uGUI/ScreenCanvas/HUD/Content/HandReticle/IconCanvas/Hand",
        "uGUI/Tooltip/",
        "uGUI/OverlayCanvas/BuildWatermark",
    ] {
        assert!(
            !visible.iter().any(|p| p.starts_with(hidden)),
            "{hidden} drawn at start"
        );
    }
    for shown in [
        "uGUI/ScreenCanvas/HUD/Content/BarsPanel/OxygenBar/Bar",
        "uGUI/ScreenCanvas/HUD/Content/DepthCompass/PlayerDepth/HalfMoon",
        "uGUI/ScreenCanvas/HUD/Content/HandReticle/IconCanvas/Default",
    ] {
        assert!(visible.contains(&shown), "{shown} not drawn at start");
    }
}

/// M9u6b2: each bar's pulse is read: its clip (oxygen `OxygenBarPulse`:
/// the wave's scale and alpha, the halo's alpha; the others `IconPulse`:
/// the icon's scale and the halo's alpha), its curves, and the icons'
/// fields.
#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn the_hud_bars_pulses_are_read() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let assemblies = sn_assets::ScriptAssemblies::load(&game).unwrap();
    let layouts = assemblies.layouts().unwrap();
    let catalog = assets.catalog().unwrap();
    let root = assets.ugui_root(&catalog).unwrap();
    let ui = assets.ui_tree("uGUI.prefab", &root).unwrap();
    let config = assets.hud_config(&layouts, &ui).unwrap();
    let names: Vec<(&str, usize, bool)> = config
        .bars
        .iter()
        .map(|b| {
            (
                b.pulse.clip.name.as_str(),
                b.pulse.clip.tracks.len(),
                b.icon.is_some(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [
            ("OxygenBarPulse", 3, false),
            ("IconPulse", 2, true),
            ("IconPulse", 2, true),
            ("IconPulse", 2, true),
        ]
    );
    for b in &config.bars {
        assert_eq!(b.pulse.clip.length, 1.0);
        assert!(!b.pulse.delay.keys.is_empty() && !b.pulse.time.keys.is_empty());
        // No pulse with the bar full.
        assert!(b.pulse.delay.evaluate(0.0) + b.pulse.time.evaluate(0.0) <= 0.0);
    }
}

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn waterfall_caps_resolve_and_keep_their_height() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let assets = Assets::index(&game).unwrap();
    let catalog = assets.catalog().unwrap();
    let database = game.read_prefab_database().unwrap();
    let mut families = 0;
    for path in database.values() {
        if !path.to_lowercase().contains("waterfall") {
            continue;
        }
        let prefab = assets.prefab(&catalog, path).unwrap();
        let caps: Vec<_> = prefab
            .nodes
            .iter()
            .filter(|n| n.waterfall_cap_height.is_some())
            .collect();
        assert_eq!(caps.len(), 2, "{}", path);
        families += 1;
        for node in caps {
            let height = node.waterfall_cap_height.unwrap();
            let (_, geometry) = assets.mesh(node.mesh.as_ref().unwrap()).unwrap();
            let min = geometry
                .positions
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min);
            let max = geometry
                .positions
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max);
            for y in [0.25, 1.0, 12.0] {
                let root = sn_world::Transform {
                    scale: [1.0, y, 1.0],
                    ..Default::default()
                };
                let local = sn_assets::waterfall_local(node.in_prefab, Some(height), y);
                let world = root.then(&local);
                assert!((world.scale[1] - height).abs() < 1e-5);
                assert_eq!(world.position[1], node.in_prefab.position[1] * y);
                let thickness = (max - min) * world.scale[1];
                assert!((thickness - (max - min) * height).abs() < 1e-5);
            }
            eprintln!(
                "waterfall {} cap height={height}, mesh y extent={}, fixed thickness={}",
                path,
                max - min,
                (max - min) * height
            );
        }
    }
    assert!(families >= 3);
    eprintln!("waterfall families checked: {families}");
}
