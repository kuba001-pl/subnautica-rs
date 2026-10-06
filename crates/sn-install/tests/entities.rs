//! Against the player's own install. Opt-in:
//! `SUBNAUTICA_DIR=… cargo test -p sn-install -- --ignored`
//!
//! Expected numbers were measured on game build 10 (see MODLOG, M7a).

use std::path::PathBuf;

use sn_install::GameData;

#[test]
#[ignore = "needs SUBNAUTICA_DIR pointing at a Subnautica install"]
fn every_saved_object_parses_and_resolves() {
    let Some(dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR not set; skipping");
        return;
    };
    let game = GameData::locate(Some(PathBuf::from(dir))).unwrap();
    let prefabs = game.read_prefab_database().unwrap();
    assert_eq!(prefabs.len(), 3336);

    let (batches, unknown) = game.object_batches().unwrap();
    assert!(unknown.is_empty(), "{unknown:?}");
    let mut objects = 0;
    for &coord in &batches {
        let tree = game.read_batch_objects(coord).unwrap().unwrap();
        assert_eq!(tree.version, 4);
        for o in &tree.objects {
            assert!(prefabs.contains_key(&o.class_id), "{coord}: {}", o.class_id);
        }
        assert_eq!(tree.world_transforms().1, 0, "{coord}: missing parents");
        objects += tree.objects.len();
    }
    assert_eq!((batches.len(), objects), (2975, 5779));

    let (batches, unknown) = game.cell_batches().unwrap();
    assert!(unknown.is_empty(), "{unknown:?}");
    let (mut cells, mut objects, mut without_prefab) = (0, 0, 0);
    let mut versions = [0; 2];
    for &coord in &batches {
        let file = game.read_batch_cells(coord).unwrap().unwrap();
        match file.version {
            9 => versions[0] += 1,
            10 => versions[1] += 1,
            v => panic!("{coord}: version {v}"),
        }
        for cell in &file.cells {
            cells += 1;
            let Some(tree) = &cell.objects else { continue };
            assert_eq!(tree.world_transforms().1, 0, "{coord}: missing parents");
            for o in &tree.objects {
                if o.class_id.is_empty() {
                    without_prefab += 1;
                } else {
                    assert!(prefabs.contains_key(&o.class_id), "{coord}: {}", o.class_id);
                }
            }
            objects += tree.objects.len();
        }
    }
    assert_eq!(
        (batches.len(), cells, objects, without_prefab),
        (1606, 437_003, 414_067, 4681)
    );
    assert_eq!(versions, [1558, 48]);
}
