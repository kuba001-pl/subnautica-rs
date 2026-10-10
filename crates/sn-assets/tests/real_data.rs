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
    // Equipping the first body model hides the default and shows it.
    let (tech, node) = body.slots[0].models[0];
    let default = body.slots[0].default_node.unwrap();
    body.equip(|s| if s == "Body" { tech } else { 0 });
    assert!(body.prefab.nodes[node.unwrap()].active);
    assert!(!body.prefab.nodes[default].active);
}
