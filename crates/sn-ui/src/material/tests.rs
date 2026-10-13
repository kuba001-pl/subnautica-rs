//! Hand-computed cases of TMP's material rules.

use super::*;

fn sdf(gradient: f32) -> UiMaterial {
    let mut m = UiMaterial {
        name: "test SDF".into(),
        shader: TMP_SDF.into(),
        ..UiMaterial::default()
    };
    for (k, v) in [
        ("_GradientScale", gradient),
        ("_FaceDilate", 0.0),
        ("_OutlineWidth", 0.0),
        ("_OutlineSoftness", 0.0),
        ("_WeightNormal", 0.0),
        ("_WeightBold", 0.5),
        ("_ScaleRatioA", 1.0),
        ("_UnderlayOffsetX", 0.0),
        ("_UnderlayOffsetY", 0.0),
        ("_UnderlayDilate", 0.0),
        ("_UnderlaySoftness", 0.0),
        ("_TextureWidth", 512.0),
        ("_TextureHeight", 512.0),
    ] {
        m.floats.insert(k.into(), v);
    }
    m.textures.insert("_MainTex".into(), 7);
    m
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn ratios_follow_the_widest_setting() {
    let mut m = sdf(10.0);
    update_shader_ratios(&mut m);
    // weight 0.5 / 4 = 0.125; range max(1, 0.125) = 1; (10 − 1) / 10.
    assert!(close(m.float("_ScaleRatioA"), 0.9));
    // Underlay: inner (0.125 + 0) · 9; range 1; (9 − 1.125) / 10.
    assert!(close(m.float("_ScaleRatioC"), 0.7875));
    assert!(!m.has("_ScaleRatioB"));
    m.keywords.push("RATIOS_OFF".into());
    update_shader_ratios(&mut m);
    assert_eq!(m.float("_ScaleRatioA"), 1.0);
    assert_eq!(m.float("_ScaleRatioC"), 1.0);
}

#[test]
fn padding_of_plain_outlined_extra_and_underlaid() {
    let mut m = sdf(10.0);
    assert!(close(tmp_padding(&mut m, false), 1.25));
    // Extra padding: min(0 + 4, 1) · 10 + 1.25.
    assert!(close(tmp_padding(&mut m, true), 11.25));
    let mut o = sdf(10.0);
    o.floats.insert("_OutlineWidth".into(), 0.2);
    // Ratio A stays 0.9 (range 1); 0.2 · 0.9 · 10 + 1.25.
    assert!(close(tmp_padding(&mut o, false), 3.05));
    let mut u = sdf(10.0);
    u.keywords.push("UNDERLAY_ON".into());
    u.floats.insert("_UnderlayOffsetX".into(), 0.5);
    u.floats.insert("_UnderlayOffsetY".into(), -0.5);
    u.floats.insert("_UnderlaySoftness".into(), 0.2);
    // Ratio C: inner 1.125, range max(1, 0.5 + 0.2) = 1, so 0.7875.
    // Reach 0.2 · 0.7875 = 0.1575, offsets ±0.39375: the widest side is
    // 0.55125, times 10, + 1.25.
    assert!(close(tmp_padding(&mut u, false), 6.7625));
    // Without the keyword the underlay adds nothing.
    u.keywords.clear();
    assert!(close(tmp_padding(&mut u, false), 1.25));
}

#[test]
fn padding_of_a_bitmap_material() {
    let mut m = UiMaterial::ui_default();
    assert_eq!(tmp_padding(&mut m, false), 1.0);
    m.floats.insert("_Padding".into(), 2.7);
    assert_eq!(tmp_padding(&mut m, true), 7.0);
}

#[test]
fn fallback_takes_the_fonts_atlas_and_metrics() {
    let mut source = sdf(10.0);
    source.floats.insert("_OutlineWidth".into(), 0.3);
    source.keywords.push("UNDERLAY_ON".into());
    let mut target = sdf(5.0);
    target.textures.insert("_MainTex".into(), 9);
    target.floats.insert("_TextureWidth".into(), 2048.0);
    target.floats.insert("_WeightBold".into(), 0.75);
    let f = fallback_material(&source, &target);
    assert_eq!(f.textures["_MainTex"], 9);
    assert_eq!(f.float("_GradientScale"), 5.0);
    assert_eq!(f.float("_TextureWidth"), 2048.0);
    assert_eq!(f.float("_WeightBold"), 0.75);
    // The text's own settings stay.
    assert_eq!(f.float("_OutlineWidth"), 0.3);
    assert_eq!(f.keywords, vec!["UNDERLAY_ON".to_string()]);
    // Not both SDF: the fallback's own material.
    assert_eq!(
        fallback_material(&UiMaterial::ui_default(), &target),
        target
    );
}

#[test]
fn uvs_pack_to_nine_bits_each() {
    assert_eq!(pack_uv(0.0, 0.0), 0.0);
    assert_eq!(pack_uv(1.0, 0.0), 511.0 * 4096.0);
    assert_eq!(pack_uv(0.5, 1.0), 255.0 * 4096.0 + 511.0);
}

#[test]
fn clip_keyword_joins_the_materials() {
    let mut m = UiMaterial::ui_default();
    m.keywords = vec!["UNITY_UI_ALPHACLIP".into()];
    assert_eq!(m.draw_keywords(false), vec!["UNITY_UI_ALPHACLIP"]);
    assert_eq!(
        m.draw_keywords(true),
        vec!["UNITY_UI_ALPHACLIP", "UNITY_UI_CLIP_RECT"]
    );
}
