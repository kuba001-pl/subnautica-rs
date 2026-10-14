//! `ui --tmp-sprites` (M9u6d1, `docs/formats/ui.md` § TMP inline sprites):
//! the sprite assets as read, every `InputSymbols` name found by its tag,
//! a `<sprite name="MouseButtonLeft" color=#ADF8FFFF>` laid out and meshed
//! against the formula from the decompiled `GenerateTextMesh`, and the
//! material each text material gets (`FindMatchingMaterial`).

use std::process::ExitCode;

use sn_assets::{Assets, texture_key};
use sn_install::GameData;
use sn_ui::layout::Rect;
use sn_ui::text::sprite::{find_matching_material, resolve, simple_hash};
use sn_ui::text::{
    MeshOptions, SpriteAssetValue, SpriteTag, TextParams, layout_text, text_meshes_with,
};

use crate::Result;

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let (mut ok, mut all) = (0usize, 0usize);
    let mut check = |good: bool, what: &str| {
        all += 1;
        if good {
            ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    };
    for (s, a) in fonts.sprite_assets.iter().zip(&set.sprites) {
        let material = match &s.material {
            Some(m) => assets.ui_material(m, &fonts)?.name,
            None => "none".into(),
        };
        let sheet = match &s.sheet {
            Some(o) => {
                let t = assets.texture(o)?;
                format!(
                    "{}×{} format {} colour space {}",
                    t.texture.width, t.texture.height, t.texture.format, t.texture.color_space
                )
            }
            None => "none".into(),
        };
        println!(
            "sprite asset {}: {} characters, {} glyphs; face point size {}; sheet {sheet}; material {material}; {} m_materials; fallbacks {:?}",
            a.name,
            a.characters.len(),
            a.glyphs.len(),
            a.face.point_size,
            s.materials.len(),
            a.fallbacks
        );
        // Stored hashes are TMP's hash of the name; every character has
        // its glyph.
        for (i, c) in a.characters.iter().enumerate() {
            check(
                c.hash == simple_hash(&c.name),
                &format!("{} [{i}] hash", a.name),
            );
            check(a.glyph_of(i).is_some(), &format!("{} [{i}] glyph", a.name));
        }
    }
    let default = set
        .default_sprite
        .ok_or("no runtime default sprite asset")?;
    let symbols = &set.sprites[default];
    println!(
        "default sprite asset (as LanguageSDF sets it): {}",
        symbols.name
    );
    // Every name in the default asset, written as a tag, finds itself.
    let mut found = 0;
    for (i, c) in symbols.characters.iter().enumerate() {
        let t = SpriteTag {
            asset: SpriteAssetValue::Default,
            name: Some(c.name.clone()),
            index: None,
            tint: false,
            color: [255; 4],
            anim: false,
        };
        let r = resolve(&set.sprites, set.default_sprite, &t);
        let first = symbols.characters.iter().position(|x| x.name == c.name);
        if r == Some((default, i)) || (r.is_some() && r.map(|r| r.1) == first) {
            found += 1;
        }
    }
    check(found == symbols.characters.len(), "every name resolves");
    println!(
        "names resolved: {found} of {} in {}",
        symbols.characters.len(),
        symbols.name
    );

    // The sprite in a text, laid out and meshed.
    let font = set
        .fonts
        .iter()
        .position(|f| f.name == "Aller_Rg SDF")
        .ok_or("no Aller_Rg SDF")?;
    let size = 24.0;
    let p = TextParams::new(
        "<sprite name=\"MouseButtonLeft\" color=#ADF8FFFF>",
        font,
        size,
    );
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 500.0,
        height: 100.0,
    };
    let l = layout_text(&set, &p, rect, 0.0);
    let options = MeshOptions {
        horizontal_mapping: 0,
        vertical_mapping: 0,
        sdf_shader: true,
        lossy_scale_y: 1.0,
    };
    let (meshes, notes) = text_meshes_with(&l, &p, &options, &set.sprites, |_, _| None);
    check(
        l.unported.is_empty() && notes.is_empty(),
        "no unported notes",
    );
    let c = l
        .characters
        .iter()
        .find(|c| c.visible)
        .ok_or("the sprite is not visible")?;
    let index = symbols
        .characters
        .iter()
        .position(|c| c.name == "MouseButtonLeft")
        .ok_or("no MouseButtonLeft")?;
    check(
        c.sprite == Some((default, index)),
        "the character is the sprite",
    );
    let ch = &symbols.characters[index];
    let g = symbols.glyph_of(index).ok_or("no glyph")?;
    let f = &set.fonts[font].face;
    // `GenerateTextMesh`, a sprite asset without a point size, in f64.
    let font_scale = f64::from(size) / f64::from(f.point_size) * f64::from(f.scale);
    let scale = f64::from(f.ascent_line) / f64::from(g.metrics.height)
        * f64::from(ch.scale)
        * f64::from(g.scale)
        * font_scale;
    let w = f64::from(g.metrics.width) * scale;
    let h = f64::from(g.metrics.height) * scale;
    let got_w = f64::from(c.top_right[0] - c.top_left[0]);
    let got_h = f64::from(c.top_left[1] - c.bottom_left[1]);
    println!(
        "MouseButtonLeft at {size} px in {}: scale {:.6} (formula {scale:.6}); quad {got_w:.4} × {got_h:.4} (formula {w:.4} × {h:.4}); advance {:.4}",
        set.fonts[font].name,
        c.scale,
        f64::from(g.metrics.advance) * scale
    );
    check((f64::from(c.scale) - scale).abs() < 1e-5, "scale");
    check(
        (got_w - w).abs() < 1e-3 && (got_h - h).abs() < 1e-3,
        "quad size",
    );
    let sprite_mesh = meshes
        .iter()
        .find(|m| m.sprite == Some(default))
        .ok_or("no sprite sub-mesh")?;
    let v = &sprite_mesh.mesh.vertices;
    check(v.len() == 4, "4 vertices");
    let (sw, sh) = (symbols.sheet[0] as f32, symbols.sheet[1] as f32);
    let r = g.rect;
    let (u0, v0) = (r.x as f32 / sw, r.y as f32 / sh);
    let (u1, v1) = ((r.x + r.width) as f32 / sw, (r.y + r.height) as f32 / sh);
    let inside = v.iter().all(|x| {
        (x.uv[0] - u0).abs().min((x.uv[0] - u1).abs()) < 1e-6
            && (x.uv[1] - v0).abs().min((x.uv[1] - v1).abs()) < 1e-6
    });
    println!(
        "UVs {:?}; glyph rect ({}, {}, {}, {}) in {:?}; colours {:?}",
        v.iter().map(|x| x.uv).collect::<Vec<_>>(),
        r.x,
        r.y,
        r.width,
        r.height,
        symbols.sheet,
        v.iter().map(|x| x.color).collect::<Vec<_>>()
    );
    check(inside, "UVs at the glyph rect's corners");
    check(
        v.iter().all(|x| x.color == [0xAD, 0xF8, 0xFF, 0xFF]),
        "colour #ADF8FFFF",
    );

    // `FindMatchingMaterial` for the text materials the fonts have (and
    // their Outline / Shadow names); each candidate samples the sheet.
    let s = &fonts.sprite_assets[default];
    let mut names = Vec::new();
    for m in &s.materials {
        let u = assets.ui_material(m, &fonts)?;
        let sheet = s.sheet.as_ref().map(texture_key);
        check(
            u.textures.get("_MainTex").copied() == sheet,
            &format!("{} samples the sheet", u.name),
        );
        names.push(u.name);
    }
    if let Some(m) = &s.material {
        let u = assets.ui_material(m, &fonts)?;
        check(
            u.textures.get("_MainTex").copied() == s.sheet.as_ref().map(texture_key),
            &format!("{} samples the sheet", u.name),
        );
        println!(
            "material {}: shader {}, keywords {:?}",
            u.name, u.shader, u.keywords
        );
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    for text in [
        "Aller_Rg SDF Material",
        "Aller_Rg SDF Outline",
        "Aller_W_Bd SDF Shadow",
    ] {
        let chosen = find_matching_material(text, &refs).map_or("material", |i| refs[i]);
        println!("text material {text:?} → sprite material {chosen:?}");
    }
    println!("checks: {ok} of {all}");
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
