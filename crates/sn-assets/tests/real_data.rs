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
    assert_eq!(materials.texture_count, 183);
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
