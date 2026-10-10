//! Object readers on bytes built in code (Unity 2019.4 layouts).

use sn_unity::{
    AutoLoadScene, CrashedShipExploder, EscapePodCinematicControl, LightingController, LodGroup,
    Material, MonoBehaviourHeader, PPtr, ShipExteriorCullManager, Texture2D, TextureFormat,
    parse_additional_scenes, parse_autoload_scenes, parse_marmo_lifepod_sky,
    parse_ship_exterior_cull,
};

/// Little-endian writer with Unity's 4-byte alignment for strings and bools.
#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    fn align(&mut self) -> &mut Self {
        while self.0.len() % 4 != 0 {
            self.0.push(0);
        }
        self
    }
    fn u8a(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self.align()
    }
    fn str(&mut self, s: &str) -> &mut Self {
        self.i32(s.len() as i32);
        self.0.extend(s.as_bytes());
        self.align()
    }
    fn pptr(&mut self, file: i32, path: i64) -> &mut Self {
        self.i32(file);
        self.0.extend(path.to_le_bytes());
        self
    }
}

fn texture_bytes(
    name: &str,
    w: i32,
    h: i32,
    format: i32,
    pixels: &[u8],
    stream: Option<(u32, u32, &str)>,
) -> Vec<u8> {
    let mut b = W::default();
    b.str(name).i32(-1).u8a(0); // name, forced fallback format, downscale fallback
    b.i32(w).i32(h).i32(pixels.len() as i32).i32(format).i32(1);
    b.0.extend([0, 0, 0]); // readable, ignore master limit, streaming mipmaps
    b.align().i32(0).i32(1).i32(2); // streaming priority, image count, dimension
    b.i32(1).i32(1).f32(0.0).i32(0).i32(0).i32(0); // filter, aniso, mip bias, wrap u/v/w
    b.i32(0).i32(1); // lightmap format, colour space
    match stream {
        None => {
            b.i32(pixels.len() as i32);
            b.0.extend(pixels);
            b.align().u32(0).u32(0).str("");
        }
        Some((offset, size, path)) => {
            b.i32(0).u32(offset).u32(size).str(path);
        }
    }
    b.0
}

#[test]
fn texture_with_embedded_pixels_decodes_top_row_first() {
    // 2×2 RGBA32, stored bottom row first: bottom = red, green; top = blue, white.
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let bytes = texture_bytes("tiny", 2, 2, 4, &pixels, None);
    let t = Texture2D::parse(&bytes, false).unwrap();
    assert_eq!((t.name.as_str(), t.width, t.height), ("tiny", 2, 2));
    assert_eq!(t.texture_format(), TextureFormat::Rgba32);
    assert!(t.stream.is_none());
    let rgba = t.decode_rgba(&t.image_data).unwrap();
    assert_eq!(
        &rgba[..8],
        &[0, 0, 255, 255, 255, 255, 255, 255],
        "top row first"
    );
    assert_eq!(&rgba[8..], &[255, 0, 0, 255, 0, 255, 0, 255]);
}

#[test]
fn texture_streaming_info_and_alpha8() {
    let bytes = texture_bytes(
        "mask",
        4,
        4,
        1,
        &[],
        Some((4096, 16, "archive:/CAB-x/CAB-x.resS")),
    );
    let t = Texture2D::parse(&bytes, false).unwrap();
    let stream = t.stream.as_ref().unwrap();
    assert_eq!(
        (stream.offset, stream.size, stream.file_name()),
        (4096, 16, "CAB-x.resS")
    );
    let rgba = t.decode_rgba(&[7; 16]).unwrap();
    assert!(
        rgba.chunks(4).all(|p| p == [0, 0, 0, 7]),
        "alpha8 → (0,0,0,a)"
    );
    assert!(
        t.decode_rgba(&[7; 15]).is_err(),
        "short data is an error, not a panic"
    );
}

#[test]
fn dxt1_block_decodes() {
    // One 4×4 DXT1 block: both endpoint colours pure red (0xF800), all indices 0.
    let block = [0x00, 0xF8, 0x00, 0xF8, 0, 0, 0, 0];
    let bytes = texture_bytes("red", 4, 4, 10, &block, None);
    let t = Texture2D::parse(&bytes, false).unwrap();
    assert_eq!(t.mip0_size(), Some(8));
    let rgba = t.decode_rgba(&t.image_data).unwrap();
    assert!(
        rgba.chunks(4).all(|p| p == [255, 0, 0, 255]),
        "{:?}",
        &rgba[..4]
    );
}

#[test]
fn material_properties() {
    let mut real = W::default();
    real.str("Sand").pptr(3, 42).str("UWE_SIG").u32(0); // shader, keywords, lightmap flags
    real.0.extend([0, 0]); // two bools sharing one aligned slot
    real.align().i32(-1); // custom render queue
    real.i32(1).str("RenderType").str("Opaque"); // tag map
    real.i32(0); // disabled passes
    real.i32(2);
    real.str("_MainTex")
        .pptr(1, 900)
        .f32(1.0)
        .f32(2.0)
        .f32(0.0)
        .f32(0.5);
    real.str("_BumpMap")
        .pptr(0, 0)
        .f32(1.0)
        .f32(1.0)
        .f32(0.0)
        .f32(0.0);
    real.i32(1).str("_TriplanarScale").f32(0.14);
    real.i32(1)
        .str("_Color")
        .f32(1.0)
        .f32(0.5)
        .f32(0.25)
        .f32(1.0);
    let m = Material::parse(&real.0, false).unwrap();
    assert_eq!(m.name, "Sand");
    assert_eq!(
        m.shader,
        PPtr {
            file_id: 3,
            path_id: 42
        }
    );
    assert_eq!(m.keywords, "UWE_SIG");
    assert_eq!(m.tags, vec![("RenderType".into(), "Opaque".into())]);
    let main = m.texture("_MainTex").unwrap();
    assert_eq!(
        (main.texture.path_id, main.scale, main.offset),
        (900, [1.0, 2.0], [0.0, 0.5])
    );
    assert!(
        m.texture("_BumpMap").is_none(),
        "null texture slots are skipped"
    );
    assert_eq!(m.float("_TriplanarScale"), Some(0.14));
    assert_eq!(m.color("_Color"), Some([1.0, 0.5, 0.25, 1.0]));
    for len in 0..real.0.len() {
        let _ = Material::parse(&real.0[..len], false); // truncated: error, not panic
    }
}

#[test]
fn monobehaviour_header() {
    let mut b = W::default();
    b.pptr(0, 36).u8a(1).pptr(1, 1265).str("");
    b.i32(12345); // first script field
    let h = MonoBehaviourHeader::parse(&b.0, false).unwrap();
    assert_eq!(h.game_object.path_id, 36);
    assert!(h.enabled);
    assert_eq!(
        h.script,
        PPtr {
            file_id: 1,
            path_id: 1265
        }
    );
    assert_eq!(h.fields_offset, 32);
}

fn behaviour() -> W {
    let mut b = W::default();
    b.pptr(0, 3).u8a(1).pptr(1, 62).str("");
    b
}

#[test]
fn scene_lists() {
    let mut b = behaviour();
    b.i32(1).str("Essentials");
    assert_eq!(
        parse_additional_scenes(&b.0, false).unwrap(),
        ["Essentials"]
    );

    let mut b = behaviour();
    b.i32(2).str("Cyclops").u8a(0).str("Aurora").u8a(1);
    assert_eq!(
        parse_autoload_scenes(&b.0, false).unwrap(),
        [
            AutoLoadScene {
                scene_name: "Cyclops".into(),
                spawn_on_start: false
            },
            AutoLoadScene {
                scene_name: "Aurora".into(),
                spawn_on_start: true
            }
        ]
    );
    // Cut short: an error, not a panic.
    for len in 0..b.0.len() {
        assert!(parse_autoload_scenes(&b.0[..len], false).is_err());
    }
}

#[test]
fn crashed_ship_exploder() {
    let mut b = behaviour();
    b.pptr(1, 9);
    b.i32(2).pptr(0, 10).pptr(0, 11);
    b.i32(1).pptr(0, 12);
    b.pptr(0, 13);
    b.f32(1.0); // later fields are ignored
    let e = CrashedShipExploder::parse(&b.0, false).unwrap();
    let p = |path_id| PPtr {
        file_id: 0,
        path_id,
    };
    assert_eq!(e.crashed_ship_prefab.path_id, 9);
    assert_eq!(e.disable_on_explosion, [p(10), p(11)]);
    assert_eq!(e.enable_on_explosion, [p(12)]);
    assert_eq!(e.exploded_exterior, p(13));
    for len in 0..b.0.len() - 4 {
        assert!(CrashedShipExploder::parse(&b.0[..len], false).is_err());
    }
}

#[test]
fn lighting_controller_and_lifepod_sky() {
    let mut b = behaviour();
    b.i32(2).f32(1.5); // state, fade duration
    b.i32(1).pptr(0, 20); // one sky
    b.i32(3).f32(10.0).f32(0.8).f32(2.5);
    b.i32(3).f32(2.0).f32(0.5).f32(0.8);
    b.i32(2).f32(1.5).f32(3.0);
    b.i32(2).pptr(0, 21).i32(3).f32(0.0).f32(1.25).f32(0.0); // two lights
    b.pptr(0, 22).i32(0);
    b.i32(3).f32(0.0).f32(1.0).f32(1.0); // emissive
    let c = LightingController::parse(&b.0, false).unwrap();
    assert_eq!((c.state, c.fade_duration), (2, 1.5));
    assert_eq!(c.skies.len(), 1);
    assert_eq!(c.skies[0].sky.path_id, 20);
    assert_eq!(c.skies[0].master, [10.0, 0.8, 2.5]);
    assert_eq!(c.skies[0].diffuse, [2.0, 0.5, 0.8]);
    assert_eq!(c.skies[0].specular, [1.5, 3.0]);
    assert_eq!(c.lights.len(), 2);
    assert_eq!(c.lights[0].light.path_id, 21);
    assert_eq!(c.lights[0].intensities, [0.0, 1.25, 0.0]);
    assert!(c.lights[1].intensities.is_empty());
    assert_eq!(c.emissive, [0.0, 1.0, 1.0]);
    for len in 0..b.0.len() {
        assert!(LightingController::parse(&b.0[..len], false).is_err());
    }
    // An implausible count is an error, not an allocation.
    let mut b = behaviour();
    b.i32(0).f32(1.0).i32(1 << 30);
    assert!(LightingController::parse(&b.0, false).is_err());

    let mut b = behaviour();
    b.pptr(0, 1).pptr(0, 2).pptr(0, 3).pptr(0, 632);
    b.i32(2); // two keys: time, value, slopes, weighted mode, weights
    for (t, v) in [(0.0, 10.0), (10.5, 0.5)] {
        b.f32(t).f32(v).f32(0.0).f32(0.0).i32(0).f32(0.33).f32(0.33);
    }
    b.i32(2).i32(2).i32(0); // pre and post infinity, rotation order
    b.pptr(0, 565).pptr(0, 32).f32(0.0).u8a(0);
    let c = EscapePodCinematicControl::parse(&b.0, false).unwrap();
    assert_eq!(c.lighting_control.path_id, 3);
    assert_eq!(c.interior_sky.path_id, 632);
    assert_eq!(c.sky_intensity_curve.keys.len(), 2);
    assert_eq!(c.sky_intensity_curve.evaluate(20.0), 0.5);
    assert_eq!(
        (c.lights_animator.path_id, c.hatch_light.path_id),
        (565, 32)
    );
    for len in 0..b.0.len() - 8 {
        assert!(EscapePodCinematicControl::parse(&b.0[..len], false).is_err());
    }

    let mut b = behaviour();
    b.pptr(0, 632);
    assert_eq!(parse_marmo_lifepod_sky(&b.0, false).unwrap().path_id, 632);
    for len in 0..b.0.len() {
        assert!(parse_marmo_lifepod_sky(&b.0[..len], false).is_err());
    }
}

#[test]
fn ship_exterior_cull() {
    let mut b = behaviour();
    b.pptr(0, 21).i32(10);
    let m = ShipExteriorCullManager::parse(&b.0, false).unwrap();
    assert_eq!(m.crashed_ship_exploder.path_id, 21);
    assert_eq!(m.update_every_x_frames, 10);
    for len in 0..b.0.len() {
        assert!(ShipExteriorCullManager::parse(&b.0[..len], false).is_err());
    }

    let mut b = behaviour();
    b.i32(2).pptr(0, 30).pptr(0, 31);
    let boxes = parse_ship_exterior_cull(&b.0, false).unwrap();
    assert_eq!(
        boxes.iter().map(|p| p.path_id).collect::<Vec<_>>(),
        [30, 31]
    );
    for len in 0..b.0.len() {
        assert!(parse_ship_exterior_cull(&b.0[..len], false).is_err());
    }
    // A count larger than the bytes left: an error, not a huge allocation.
    let mut b = behaviour();
    b.i32(i32::MAX);
    assert!(parse_ship_exterior_cull(&b.0, false).is_err());
}

#[test]
fn lod_group() {
    let mut b = W::default();
    b.pptr(0, 7);
    b.f32(0.5).f32(1.0).f32(-2.0); // local reference point
    b.f32(12.0); // size
    b.i32(1); // fade mode: cross-fade
    b.0.extend([1, 0]); // animate cross-fading, last is billboard
    b.align();
    b.i32(2);
    b.f32(0.25).f32(0.1).i32(1).pptr(0, 20);
    b.f32(0.01).f32(0.0).i32(2).pptr(0, 21).pptr(0, 22);
    b.u8a(1); // enabled
    let g = LodGroup::parse(&b.0, false).unwrap();
    assert_eq!(g.game_object.path_id, 7);
    assert_eq!(g.local_reference_point, [0.5, 1.0, -2.0]);
    assert_eq!(g.size, 12.0);
    assert_eq!(g.fade_mode, 1);
    assert!(g.animate_cross_fading && !g.last_lod_is_billboard);
    assert_eq!(g.lods.len(), 2);
    assert_eq!(g.lods[0].screen_relative_height, 0.25);
    assert_eq!(g.lods[0].fade_transition_width, 0.1);
    assert_eq!(
        g.lods[1]
            .renderers
            .iter()
            .map(|p| p.path_id)
            .collect::<Vec<_>>(),
        [21, 22]
    );
    assert!(g.enabled);
    for len in 0..b.0.len() - 3 {
        assert!(LodGroup::parse(&b.0[..len], false).is_err());
    }
}

#[test]
fn skinned_mesh_renderer() {
    let mut b = W::default();
    // Renderer: game object, enabled, cast shadows, 6 flags, padding,
    // layer mask, priority, lightmap indices, tiling offsets, materials.
    b.pptr(0, 5);
    b.0.extend([1, 1, 1, 1, 0, 1, 1, 0]);
    b.u32(1).i32(0).i32(0xffff);
    for _ in 0..8 {
        b.f32(0.0);
    }
    b.i32(1).pptr(2, 77);
    // Static batch info, three PPtrs, sorting layer id, layer, order.
    b.i32(0).pptr(0, 0).pptr(0, 0).pptr(0, 0).i32(0).i32(0);
    // Quality, two flags, mesh, bones, blend shape weights, root bone, AABB.
    b.i32(0).u8a(0);
    b.pptr(3, 9);
    b.i32(2).pptr(0, 20).pptr(0, 21);
    b.i32(1).f32(0.5);
    b.pptr(0, 20);
    for v in [1.0, 2.0, 3.0, 0.5, 0.5, 0.5] {
        b.f32(v);
    }
    b.u8a(0);
    let r = sn_unity::SkinnedMeshRenderer::parse(&b.0, false).unwrap();
    assert!(r.renderer.enabled);
    assert_eq!(r.renderer.materials[0].path_id, 77);
    assert_eq!(r.mesh.path_id, 9);
    assert_eq!(
        r.bones.iter().map(|p| p.path_id).collect::<Vec<_>>(),
        [20, 21]
    );
    assert_eq!(r.blend_shape_weights, [0.5]);
    assert_eq!(r.root_bone.path_id, 20);
    assert_eq!(r.aabb, ([1.0, 2.0, 3.0], [0.5; 3]));
    for len in 0..b.0.len() - 4 {
        assert!(sn_unity::SkinnedMeshRenderer::parse(&b.0[..len], false).is_err());
    }
}

#[test]
fn escape_pod_and_spawners() {
    let mut b = behaviour();
    b.pptr(0, 40).pptr(0, 41).pptr(0, 99);
    let pod = sn_unity::EscapePod::parse(&b.0, false).unwrap();
    assert_eq!(pod.bottom_hatch_entrance.path_id, 40);
    assert_eq!(pod.player_spawn.path_id, 41);

    // PrefabSpawnBase fields as the game's escape pod stores them.
    let base = |b: &mut W, message: &str| {
        b.i32(sn_unity::SPAWN_ON_NEW_BORN).f32(0.0);
        b.u8a(1).u8a(0).u8a(0).u8a(1); // inherit layer, …, keep scale
        b.pptr(0, 376)
            .f32(1.0)
            .u8a(0)
            .pptr(0, 0)
            .str(message)
            .u8a(0);
    };
    let mut b = behaviour();
    base(&mut b, "ForceSpawnMedKit");
    b.str("d8c3e8dc5d573b94098088e33f096e28").str("").str("");
    let s = sn_unity::PrefabSpawner::parse(&b.0, false, true).unwrap();
    assert!(s.spawns_in_new_game() && s.keep_scale && !s.use_prefab_transform_as_local);
    assert_eq!(s.attach_to_parent.path_id, 376);
    assert_eq!(
        s.prefab,
        sn_unity::SpawnPrefab::Address("d8c3e8dc5d573b94098088e33f096e28".into())
    );
    for len in 0..b.0.len() - 8 {
        assert!(sn_unity::PrefabSpawner::parse(&b.0[..len], false, true).is_err());
    }

    let mut b = behaviour();
    base(&mut b, "");
    b.pptr(2, 157);
    let s = sn_unity::PrefabSpawner::parse(&b.0, false, false).unwrap();
    assert_eq!(
        s.prefab,
        sn_unity::SpawnPrefab::Object(PPtr {
            file_id: 2,
            path_id: 157
        })
    );
    let manual = sn_unity::PrefabSpawner {
        spawn_type: sn_unity::SPAWN_MANUAL,
        ..s
    };
    assert!(!manual.spawns_in_new_game());
}
