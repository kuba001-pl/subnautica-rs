//! The game's TextMesh Pro fonts (M9u3, `docs/formats/ui.md` § Fonts):
//! every `TMP_FontAsset` of the fonts bundle, read by its layout from
//! `Unity.TextMeshPro.dll` (M9u1), with the `Font` that holds its font
//! file. Every asset is dynamic: TMP makes its glyphs from the font file
//! while the game runs, so the file's bytes are what matters here.

use sn_ui::font::{FaceInfo, Glyph, GlyphMetrics, GlyphRect};
use sn_unity::{Fields, Font, MonoBehaviourHeader, Value};

use crate::{Assets, ObjectRef, Result, ScriptAssemblies};

const MONO_BEHAVIOUR: i32 = 114;
const FONT: i32 = 128;
/// The bundle that holds every font and font asset.
pub const FONTS_BUNDLE: &str = "fonts_assets_all";

/// A `TMP_FontAsset` with its fields and the objects it points at.
pub struct LoadedFontAsset {
    pub name: String,
    pub object: ObjectRef,
    pub fields: Fields,
    /// `m_SourceFontFile`.
    pub source: Option<ObjectRef>,
    /// `m_AtlasTextures`.
    pub atlas_textures: Vec<ObjectRef>,
    /// `m_FallbackFontAssetTable`.
    pub fallbacks: Vec<ObjectRef>,
}

/// A font asset's stored settings and tables, typed.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredFontAsset {
    pub face: FaceInfo,
    /// `m_AtlasPopulationMode` (1 = dynamic).
    pub population_mode: i64,
    /// `m_AtlasRenderMode` (4165 = SDFAA).
    pub render_mode: i64,
    pub padding: i32,
    pub atlas_width: i32,
    pub atlas_height: i32,
    /// `m_CreationSettings.pointSize`: the sampling size.
    pub point_size: i32,
    pub multi_atlas: bool,
    pub glyphs: Vec<Glyph>,
    /// (unicode, glyph index).
    pub characters: Vec<(u32, u32)>,
    pub used_rects: Vec<GlyphRect>,
    pub free_rects: Vec<GlyphRect>,
}

fn num(f: &Fields, path: &str) -> Result<f64> {
    f.path(path)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("no number {path}"))
}

fn rect(v: &Value) -> Result<GlyphRect> {
    let f = v.as_struct().ok_or("GlyphRect is not a struct")?;
    Ok(GlyphRect::new(
        num(f, "m_X")? as i32,
        num(f, "m_Y")? as i32,
        num(f, "m_Width")? as i32,
        num(f, "m_Height")? as i32,
    ))
}

fn items<'f>(f: &'f Fields, name: &str) -> &'f [Value] {
    f.get(name).and_then(Value::as_array).unwrap_or(&[])
}

impl LoadedFontAsset {
    /// The fields M9u3 uses, typed.
    pub fn stored(&self) -> Result<StoredFontAsset> {
        let f = &self.fields;
        let fi = f
            .get("m_FaceInfo")
            .and_then(Value::as_struct)
            .ok_or("no m_FaceInfo")?;
        let text = |n: &str| fi.get(n).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |p: &str| num(fi, p).map(|v| v as f32);
        let face = FaceInfo {
            face_index: num(fi, "m_FaceIndex")? as i32,
            family_name: text("m_FamilyName"),
            style_name: text("m_StyleName"),
            point_size: num(fi, "m_PointSize")? as i32,
            scale: n("m_Scale")?,
            line_height: n("m_LineHeight")?,
            ascent_line: n("m_AscentLine")?,
            cap_line: n("m_CapLine")?,
            mean_line: n("m_MeanLine")?,
            baseline: n("m_Baseline")?,
            descent_line: n("m_DescentLine")?,
            superscript_offset: n("m_SuperscriptOffset")?,
            superscript_size: n("m_SuperscriptSize")?,
            subscript_offset: n("m_SubscriptOffset")?,
            subscript_size: n("m_SubscriptSize")?,
            underline_offset: n("m_UnderlineOffset")?,
            underline_thickness: n("m_UnderlineThickness")?,
            strikethrough_offset: n("m_StrikethroughOffset")?,
            strikethrough_thickness: n("m_StrikethroughThickness")?,
            tab_width: n("m_TabWidth")?,
        };
        let mut glyphs = Vec::new();
        for g in items(f, "m_GlyphTable") {
            let g = g.as_struct().ok_or("Glyph is not a struct")?;
            let m = |p: &str| num(g, &format!("m_Metrics.{p}")).map(|v| v as f32);
            glyphs.push(Glyph {
                index: num(g, "m_Index")? as u32,
                metrics: GlyphMetrics {
                    width: m("m_Width")?,
                    height: m("m_Height")?,
                    bearing_x: m("m_HorizontalBearingX")?,
                    bearing_y: m("m_HorizontalBearingY")?,
                    advance: m("m_HorizontalAdvance")?,
                },
                rect: rect(g.get("m_GlyphRect").ok_or("no m_GlyphRect")?)?,
                scale: num(g, "m_Scale")? as f32,
                atlas_index: num(g, "m_AtlasIndex")? as i32,
            });
        }
        let mut characters = Vec::new();
        for c in items(f, "m_CharacterTable") {
            let c = c.as_struct().ok_or("TMP_Character is not a struct")?;
            characters.push((num(c, "m_Unicode")? as u32, num(c, "m_GlyphIndex")? as u32));
        }
        let rects =
            |name: &str| -> Result<Vec<GlyphRect>> { items(f, name).iter().map(rect).collect() };
        Ok(StoredFontAsset {
            face,
            population_mode: num(f, "m_AtlasPopulationMode")? as i64,
            render_mode: num(f, "m_AtlasRenderMode")? as i64,
            padding: num(f, "m_AtlasPadding")? as i32,
            atlas_width: num(f, "m_AtlasWidth")? as i32,
            atlas_height: num(f, "m_AtlasHeight")? as i32,
            point_size: num(f, "m_CreationSettings.pointSize")? as i32,
            multi_atlas: f
                .get("m_IsMultiAtlasTexturesEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            glyphs,
            characters,
            used_rects: rects("m_UsedGlyphRects")?,
            free_rects: rects("m_FreeGlyphRects")?,
        })
    }
}

/// A `Font` object.
pub struct LoadedFont {
    pub font: Font,
    pub object: ObjectRef,
}

/// The sprite asset `LanguageSDF.assetEntryPoints` names (its only one:
/// `…/Sprite Atlases/InputSymbols.asset`), which the game makes
/// `TMP_Settings.defaultSpriteAsset` at start (M9u6d1; decompiled code).
pub const RUNTIME_DEFAULT_SPRITE_ASSET: &str = "InputSymbols";

/// A `TMP_SpriteAsset`: the unicode values of its sprite characters
/// (0xFFFE: none), and (M9u6d1) its fields, sheet and materials.
pub struct LoadedSpriteAsset {
    pub name: String,
    pub object: ObjectRef,
    pub unicodes: Vec<u32>,
    /// `fallbackSpriteAssets`.
    pub fallbacks: Vec<ObjectRef>,
    pub fields: Fields,
    /// `spriteSheet`.
    pub sheet: Option<ObjectRef>,
    /// `TMP_Asset.material` and `m_materials` (`FindMatchingMaterial`).
    pub material: Option<ObjectRef>,
    pub materials: Vec<ObjectRef>,
}

impl LoadedSpriteAsset {
    /// The tables as the layout reads them; `sheet` is the sheet's size,
    /// fallbacks are left for the caller to resolve.
    pub fn tables(&self, sheet: [i32; 2]) -> Result<sn_ui::text::sprite::SpriteAsset> {
        use sn_ui::text::sprite::{SpriteCharacter, SpriteFace, SpriteGlyph};
        let f = &self.fields;
        let fi = f
            .get("m_FaceInfo")
            .and_then(Value::as_struct)
            .ok_or("no m_FaceInfo")?;
        let n = |p: &str| num(fi, p).map(|v| v as f32);
        let face = SpriteFace {
            point_size: num(fi, "m_PointSize")? as i32,
            scale: n("m_Scale")?,
            ascent_line: n("m_AscentLine")?,
            descent_line: n("m_DescentLine")?,
            baseline: n("m_Baseline")?,
        };
        let mut glyphs = Vec::new();
        for g in items(f, "m_SpriteGlyphTable") {
            let g = g.as_struct().ok_or("TMP_SpriteGlyph is not a struct")?;
            let m = |p: &str| num(g, &format!("m_Metrics.{p}")).map(|v| v as f32);
            glyphs.push(SpriteGlyph {
                index: num(g, "m_Index")? as u32,
                metrics: GlyphMetrics {
                    width: m("m_Width")?,
                    height: m("m_Height")?,
                    bearing_x: m("m_HorizontalBearingX")?,
                    bearing_y: m("m_HorizontalBearingY")?,
                    advance: m("m_HorizontalAdvance")?,
                },
                rect: rect(g.get("m_GlyphRect").ok_or("no m_GlyphRect")?)?,
                scale: num(g, "m_Scale")? as f32,
            });
        }
        let mut characters = Vec::new();
        for c in items(f, "m_SpriteCharacterTable") {
            let c = c.as_struct().ok_or("TMP_SpriteCharacter is not a struct")?;
            characters.push(SpriteCharacter {
                name: c
                    .get("m_Name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                hash: num(c, "m_HashCode")? as i32,
                unicode: num(c, "m_Unicode")? as u32,
                glyph: num(c, "m_GlyphIndex")? as u32,
                scale: num(c, "m_Scale")? as f32,
            });
        }
        Ok(sn_ui::text::sprite::SpriteAsset {
            name: self.name.clone(),
            face,
            characters,
            glyphs,
            sheet,
            fallbacks: Vec::new(),
        })
    }
}

/// `TMP_Settings` (the resource `TMP Settings`): what a text falls back to
/// when its own font chain lacks a character (`TMP_Text.GetTextElement`,
/// confirmed from its IL: the text's font and its fallbacks, the text's
/// sprite asset, these fallbacks, the default font, the default sprite
/// asset).
pub struct TmpSettings {
    pub object: ObjectRef,
    pub fields: Fields,
    pub default_font: Option<ObjectRef>,
    pub fallbacks: Vec<ObjectRef>,
    pub default_sprite_asset: Option<ObjectRef>,
    /// `m_missingGlyphCharacter` (0: TMP's default, U+25A1).
    pub missing_glyph: i64,
}

/// Every font and font asset of the fonts bundle, in file order, the
/// sprite assets, and the settings.
pub struct FontAssets {
    pub assets: Vec<LoadedFontAsset>,
    pub fonts: Vec<LoadedFont>,
    pub sprite_assets: Vec<LoadedSpriteAsset>,
    pub settings: Option<TmpSettings>,
}

impl FontAssets {
    pub fn asset(&self, name: &str) -> Option<&LoadedFontAsset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// The sprite asset an object reference points at.
    pub fn sprite_asset(&self, object: &ObjectRef) -> Option<&LoadedSpriteAsset> {
        self.sprite_assets
            .iter()
            .find(|a| a.object.key() == object.key())
    }

    /// The `Font` an object reference points at.
    pub fn font(&self, object: &ObjectRef) -> Option<&LoadedFont> {
        self.fonts.iter().find(|f| f.object.key() == object.key())
    }
}

/// The files `Resources.Load` reads from (`TMP Settings` is one of them).
const RESOURCE_FILES: [&str; 3] = [
    "resources.assets",
    "globalgamemanagers.assets",
    "sharedassets0.assets",
];

impl Assets<'_> {
    /// A TMP script object read by its layout: its script's class, name
    /// and fields; `None` if it is not a `TMPro` script.
    fn read_tmp(
        &self,
        layouts: &crate::Layouts,
        object: &ObjectRef,
    ) -> Result<Option<(String, String, Fields)>> {
        let Some(script) = self.mono_script(object) else {
            return Ok(None);
        };
        if script.namespace != "TMPro" {
            return Ok(None);
        }
        let layout = layouts.script(&script.assembly, &script.namespace, &script.class_name)?;
        let (_, d) = object.data()?;
        let (MonoBehaviourHeader { name, .. }, fields) =
            sn_unity::read_script(d, object.file.file().big_endian, &layout)
                .map_err(|e| format!("{} {}: {e}", script.class_name, object.path_id))?;
        Ok(Some((script.class_name, name, fields)))
    }

    fn field_ref(
        &self,
        from: &ObjectRef,
        fields: &Fields,
        name: &str,
    ) -> Result<Option<ObjectRef>> {
        match fields.get(name).and_then(Value::as_pptr) {
            Some(p) => self.resolve(&from.file, p),
            None => Ok(None),
        }
    }

    fn field_refs(&self, from: &ObjectRef, fields: &Fields, name: &str) -> Result<Vec<ObjectRef>> {
        let mut refs = Vec::new();
        for p in items(fields, name).iter().filter_map(Value::as_pptr) {
            if let Some(o) = self.resolve(&from.file, p)? {
                refs.push(o);
            }
        }
        Ok(refs)
    }

    fn read_sprite_asset(
        &self,
        layouts: &crate::Layouts,
        object: &ObjectRef,
    ) -> Result<Option<LoadedSpriteAsset>> {
        let Some((class, name, fields)) = self.read_tmp(layouts, object)? else {
            return Ok(None);
        };
        if class != "TMP_SpriteAsset" {
            return Ok(None);
        }
        let mut unicodes = Vec::new();
        for c in items(&fields, "m_SpriteCharacterTable") {
            let c = c.as_struct().ok_or("TMP_SpriteCharacter is not a struct")?;
            unicodes.push(num(c, "m_Unicode")? as u32);
        }
        let fallbacks = self.field_refs(object, &fields, "fallbackSpriteAssets")?;
        Ok(Some(LoadedSpriteAsset {
            name,
            object: object.clone(),
            unicodes,
            fallbacks,
            sheet: self.field_ref(object, &fields, "spriteSheet")?,
            material: self.field_ref(object, &fields, "material")?,
            materials: self.field_refs(object, &fields, "m_materials")?,
            fields,
        }))
    }

    /// `TMP Settings`, from the resource files.
    fn tmp_settings(&self, layouts: &crate::Layouts) -> Result<Option<TmpSettings>> {
        for name in RESOURCE_FILES {
            if !self.game().data_dir.join(name).is_file() {
                continue;
            }
            let file = self.standalone(name)?;
            for info in file
                .objects()
                .iter()
                .filter(|o| o.class_id == MONO_BEHAVIOUR)
            {
                let object = ObjectRef {
                    file: file.clone(),
                    path_id: info.path_id,
                };
                let Some((class, _, fields)) = self.read_tmp(layouts, &object)? else {
                    continue;
                };
                if class != "TMP_Settings" {
                    continue;
                }
                return Ok(Some(TmpSettings {
                    default_font: self.field_ref(&object, &fields, "m_defaultFontAsset")?,
                    fallbacks: self.field_refs(&object, &fields, "m_fallbackFontAssets")?,
                    default_sprite_asset: self.field_ref(
                        &object,
                        &fields,
                        "m_defaultSpriteAsset",
                    )?,
                    missing_glyph: fields
                        .get("m_missingGlyphCharacter")
                        .and_then(Value::as_i64)
                        .unwrap_or(0),
                    object,
                    fields,
                }));
            }
        }
        Ok(None)
    }

    /// Reads every `Font`, `TMP_FontAsset` and `TMP_SpriteAsset` of the
    /// fonts bundle, `TMP Settings`, and the default sprite asset it names.
    pub fn font_assets(&self, assemblies: &ScriptAssemblies) -> Result<FontAssets> {
        let layouts = assemblies.layouts()?;
        let bundle = self
            .bundle_named(FONTS_BUNDLE)
            .ok_or("no fonts_assets_all bundle")?
            .to_path_buf();
        let loaded = self.bundle(&bundle)?;
        let names: Vec<String> = loaded
            .file_names()
            .filter(|n| !n.ends_with(".resS") && !n.ends_with(".resource"))
            .map(String::from)
            .collect();
        let mut out = FontAssets {
            assets: Vec::new(),
            fonts: Vec::new(),
            sprite_assets: Vec::new(),
            settings: None,
        };
        for name in names {
            let file = self.file(&bundle, &name)?;
            let be = file.file().big_endian;
            for info in file.objects() {
                let object = ObjectRef {
                    file: file.clone(),
                    path_id: info.path_id,
                };
                match info.class_id {
                    FONT => {
                        let (_, d) = object.data()?;
                        let font = Font::parse(d, be)
                            .map_err(|e| format!("Font {}: {e}", info.path_id))?;
                        out.fonts.push(LoadedFont { font, object });
                    }
                    MONO_BEHAVIOUR => {
                        if let Some(s) = self.read_sprite_asset(&layouts, &object)? {
                            out.sprite_assets.push(s);
                            continue;
                        }
                        let Some((class, name, fields)) = self.read_tmp(&layouts, &object)? else {
                            continue;
                        };
                        if class != "TMP_FontAsset" {
                            continue;
                        }
                        out.assets.push(LoadedFontAsset {
                            name,
                            source: self.field_ref(&object, &fields, "m_SourceFontFile")?,
                            atlas_textures: self.field_refs(&object, &fields, "m_AtlasTextures")?,
                            fallbacks: self.field_refs(
                                &object,
                                &fields,
                                "m_FallbackFontAssetTable",
                            )?,
                            object,
                            fields,
                        });
                    }
                    _ => {}
                }
            }
        }
        out.settings = self.tmp_settings(&layouts)?;
        let default_sprites = out
            .settings
            .as_ref()
            .and_then(|s| s.default_sprite_asset.clone());
        if let Some(o) = default_sprites {
            if out.sprite_asset(&o).is_none() {
                if let Some(s) = self.read_sprite_asset(&layouts, &o)? {
                    out.sprite_assets.push(s);
                }
            }
        }
        Ok(out)
    }
}
