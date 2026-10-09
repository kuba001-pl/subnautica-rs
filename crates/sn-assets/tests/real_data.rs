//! Against the player's own install. Opt-in:
//! `SUBNAUTICA_DIR=… cargo test -p sn-assets -- --ignored`
//!
//! Expected numbers were measured on game build 10 (see MODLOG, M6).

use std::path::PathBuf;

use sn_assets::{Assets, marmo_skies, terrain_materials};
use sn_install::GameData;

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
