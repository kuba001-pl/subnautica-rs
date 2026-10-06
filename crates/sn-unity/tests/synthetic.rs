//! Serialized files and bundles built in code. No game files needed.

use sn_unity::{Bundle, ErrorKind, SerializedFile, class_name, write_bundle};

/// (class id, path id, payload)
type Object<'a> = (i32, i64, &'a [u8]);

/// Writes a version-21 serialized file (little-endian, no type trees).
fn serialized_file(objects: &[Object], externals: &[&str]) -> Vec<u8> {
    let mut classes: Vec<i32> = objects.iter().map(|o| o.0).collect();
    classes.sort_unstable();
    classes.dedup();

    let mut meta = Vec::new();
    meta.extend(b"2019.4.36f1\0");
    meta.extend(19i32.to_le_bytes()); // StandaloneWindows64
    meta.push(0); // no type trees
    meta.extend((classes.len() as i32).to_le_bytes());
    for &class in &classes {
        meta.extend(class.to_le_bytes());
        meta.push(0); // not stripped
        meta.extend((-1i16).to_le_bytes());
        if class == 114 {
            meta.extend([7u8; 16]); // script id
        }
        meta.extend([0u8; 16]); // old type hash
    }
    // Object table; entries are aligned to 4 bytes in the *file*, which
    // starts with a 20-byte header.
    meta.extend((objects.len() as i32).to_le_bytes());
    let mut data = Vec::new();
    for (class, path_id, payload) in objects {
        while (20 + meta.len()) % 4 != 0 {
            meta.push(0);
        }
        meta.extend(path_id.to_le_bytes());
        meta.extend((data.len() as u32).to_le_bytes());
        meta.extend((payload.len() as u32).to_le_bytes());
        let type_index = classes.iter().position(|c| c == class).unwrap() as i32;
        meta.extend(type_index.to_le_bytes());
        data.extend_from_slice(payload);
        while data.len() % 8 != 0 {
            data.push(0);
        }
    }
    meta.extend(0i32.to_le_bytes()); // script types
    meta.extend((externals.len() as i32).to_le_bytes());
    for path in externals {
        meta.push(0);
        meta.extend([1u8; 16]);
        meta.extend(0i32.to_le_bytes());
        meta.extend(path.as_bytes());
        meta.push(0);
    }
    meta.extend(0i32.to_le_bytes()); // ref types
    meta.push(0); // user info

    let mut data_offset = 20 + meta.len();
    data_offset = data_offset.div_ceil(16) * 16;
    let file_size = data_offset + data.len();
    let mut out = Vec::new();
    out.extend((meta.len() as u32).to_be_bytes());
    out.extend((file_size as u32).to_be_bytes());
    out.extend(21u32.to_be_bytes());
    out.extend((data_offset as u32).to_be_bytes());
    out.extend([0, 0, 0, 0]); // little-endian + reserved
    out.extend(meta);
    out.resize(data_offset, 0);
    out.extend(data);
    out
}

fn sample_objects() -> Vec<Object<'static>> {
    vec![
        (1, 1, b"game object"),
        (4, 2, b"transform!"),
        (114, 3, b"script data"),
        (43, -9_000_000_000, b"mesh bytes go here"),
        (4, 5, b"t2"),
    ]
}

#[test]
fn serialized_file_round_trip() {
    let objects = sample_objects();
    let bytes = serialized_file(&objects, &["library/unity default resources", "CAB-abc"]);
    let file = SerializedFile::parse(&bytes).unwrap();
    assert_eq!(file.version, 21);
    assert_eq!(file.unity_version, "2019.4.36f1");
    assert_eq!(file.platform, 19);
    assert!(!file.big_endian && !file.type_tree_enabled);
    assert_eq!(file.types.len(), 4);
    assert_eq!(
        file.types
            .iter()
            .find(|t| t.class_id == 114)
            .unwrap()
            .script_id,
        Some([7; 16])
    );
    assert_eq!(file.externals.len(), 2);
    assert_eq!(file.externals[1].path, "CAB-abc");
    assert_eq!(file.objects.len(), objects.len());
    for (info, (class, path_id, payload)) in file.objects.iter().zip(&objects) {
        assert_eq!((info.class_id, info.path_id), (*class, *path_id));
        assert_eq!(file.object_data(&bytes, info), Some(*payload));
    }
}

#[test]
fn bundle_round_trip_with_and_without_lz4() {
    let inner = serialized_file(&sample_objects(), &[]);
    let resource = vec![42u8; 5000];
    for lz4 in [false, true] {
        let bytes = write_bundle(
            &[("CAB-test", 4, &inner), ("CAB-test.resS", 0, &resource)],
            "2019.4.36f1",
            lz4,
        );
        let bundle = Bundle::parse(&bytes).unwrap();
        assert_eq!(bundle.format, 7);
        assert_eq!(bundle.unity_version, "2019.4.36f1");
        assert_eq!(bundle.nodes.len(), 2);
        assert!(bundle.nodes[0].is_serialized_file() && !bundle.nodes[1].is_serialized_file());
        assert_eq!(bundle.node_data(&bundle.nodes[0]), inner.as_slice());
        assert_eq!(bundle.node_data(&bundle.nodes[1]), resource.as_slice());
        let file = SerializedFile::parse(bundle.node_data(&bundle.nodes[0])).unwrap();
        assert_eq!(file.objects.len(), 5);
    }
}

#[test]
fn rejects_things_that_are_not_bundles() {
    let err = Bundle::parse(b"UnityWeb\0garbage").unwrap_err();
    assert!(matches!(err.kind, ErrorKind::BadMagic(_)));
    let err = SerializedFile::parse(&[0u8; 8]).unwrap_err();
    assert!(matches!(err.kind, ErrorKind::UnexpectedEof { .. }));
}

#[test]
fn truncated_and_corrupted_input_never_panics() {
    let inner = serialized_file(&sample_objects(), &["x"]);
    let bundle = write_bundle(&[("CAB-test", 4, &inner)], "2019.4.36f1", true);
    for len in 0..inner.len() {
        let _ = SerializedFile::parse(&inner[..len]);
    }
    for len in 0..bundle.len() {
        let _ = Bundle::parse(&bundle[..len]);
    }
    // Flip every byte in turn.
    for i in 0..inner.len() {
        let mut bad = inner.clone();
        bad[i] ^= 0xA5;
        if let Ok(file) = SerializedFile::parse(&bad) {
            for o in &file.objects {
                let _ = file.object_data(&bad, o);
            }
        }
    }
    for i in 0..bundle.len() {
        let mut bad = bundle.clone();
        bad[i] ^= 0xA5;
        let _ = Bundle::parse(&bad);
    }
}

#[test]
fn class_names() {
    assert_eq!(class_name(1), Some("GameObject"));
    assert_eq!(class_name(43), Some("Mesh"));
    assert_eq!(class_name(-5), None);
}
