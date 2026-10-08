//! Against the player's own install. Opt-in:
//! `SUBNAUTICA_DIR=… cargo test -p sn-assets -- --ignored`
//!
//! Expected numbers were measured on game build 10 (see MODLOG, M6).

use std::path::PathBuf;

use sn_assets::{Assets, terrain_materials};
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
    // 183 colour and normal maps + 28 specular/illumination (SIG) maps.
    assert_eq!(materials.texture_count, 211);
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
