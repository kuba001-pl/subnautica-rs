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
