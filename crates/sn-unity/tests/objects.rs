//! Object readers on bytes built in code (Unity 2019.4 layouts).

use sn_unity::{Material, MonoBehaviourHeader, PPtr, Texture2D, TextureFormat};

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
