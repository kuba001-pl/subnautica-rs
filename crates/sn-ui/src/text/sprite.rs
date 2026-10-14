//! TMP's sprite assets (M9u6d1; `docs/formats/ui.md` § TMP inline sprites):
//! what a `<sprite>` tag draws, ported from the decompiled TMP and the IL
//! of `TMP_SpriteAsset` (read only; the code is ours).
//!
//! A sprite asset is a sheet of pictures, each a sprite character (its
//! name, its name's hash, a unicode, a scale) pointing at a sprite glyph
//! (metrics, the rect in the sheet, a scale). A tag finds its sprite by
//! the hash of its `name` value ([`simple_hash`], case-sensitive) in the
//! asset, then the asset's fallbacks depth first, then the default sprite
//! asset (`SearchForSpriteByHashCode`).

use std::collections::HashSet;

use crate::font::{GlyphMetrics, GlyphRect};

use super::input::{SpriteAssetValue, SpriteTag};
use super::{FontSet, TmpFont};

/// `TMP_TextUtilities.GetSimpleHashCode` (and `ValidateHtmlTag`'s value
/// hash): `h = ((h << 5) + h) ^ c` over the UTF-16 units, wrapping.
pub fn simple_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| {
        (h.wrapping_shl(5).wrapping_add(h)) ^ i32::from(c)
    })
}

/// The sprite asset's `m_FaceInfo`, the fields the layout reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpriteFace {
    pub point_size: i32,
    pub scale: f32,
    pub ascent_line: f32,
    pub descent_line: f32,
    pub baseline: f32,
}

/// `TMP_SpriteGlyph`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteGlyph {
    pub index: u32,
    pub metrics: GlyphMetrics,
    pub rect: GlyphRect,
    pub scale: f32,
}

/// `TMP_SpriteCharacter`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteCharacter {
    pub name: String,
    pub hash: i32,
    pub unicode: u32,
    pub glyph: u32,
    pub scale: f32,
}

/// A `TMP_SpriteAsset`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpriteAsset {
    pub name: String,
    pub face: SpriteFace,
    pub characters: Vec<SpriteCharacter>,
    pub glyphs: Vec<SpriteGlyph>,
    /// The `spriteSheet` texture's size.
    pub sheet: [i32; 2],
    /// `fallbackSpriteAssets` (with what `LanguageSDF` adds at start), as
    /// indices into the set.
    pub fallbacks: Vec<usize>,
}

impl SpriteAsset {
    /// The glyph of character `index` (`UpdateLookupTables` links them by
    /// glyph index).
    pub fn glyph_of(&self, index: usize) -> Option<&SpriteGlyph> {
        let c = self.characters.get(index)?;
        self.glyphs.iter().find(|g| g.index == c.glyph)
    }

    /// `GetSpriteIndexFromHashcode`.
    pub fn index_of_hash(&self, hash: i32) -> Option<usize> {
        self.characters.iter().position(|c| c.hash == hash)
    }
}

/// `TMP_SpriteAsset.SearchForSpriteByHashCode(asset, hash, includeFallbacks:
/// true)`: the asset and character index, searching `start`, its
/// fallbacks depth first (each asset once), then `default`.
pub fn search_by_hash(
    assets: &[SpriteAsset],
    start: usize,
    default: Option<usize>,
    hash: i32,
) -> Option<(usize, usize)> {
    fn internal(
        assets: &[SpriteAsset],
        at: usize,
        hash: i32,
        seen: &mut HashSet<usize>,
    ) -> Option<(usize, usize)> {
        let a = assets.get(at)?;
        if let Some(i) = a.index_of_hash(hash) {
            return Some((at, i));
        }
        for &f in &a.fallbacks {
            if seen.insert(f) {
                if let Some(r) = internal(assets, f, hash, seen) {
                    return Some(r);
                }
            }
        }
        None
    }
    let mut seen = HashSet::from([start]);
    internal(assets, start, hash, &mut seen).or_else(|| {
        let d = default?;
        if !seen.insert(d) {
            return None;
        }
        internal(assets, d, hash, &mut seen)
    })
}

/// The sprite a `<sprite>` tag draws (`ValidateHtmlTag`): its asset and
/// character index, or `None` when TMP rejects the tag (its characters
/// are then shown as text). The text's own sprite asset is not read (no
/// text in scope has one), so the first value's default is the default
/// sprite asset; `<sprite="Asset">` finds a loaded asset by name
/// (*hypothesis*: the game's `Resources` path holds none other). `anim`
/// is not ported: rejected.
pub fn resolve(
    sprites: &[SpriteAsset],
    default: Option<usize>,
    t: &SpriteTag,
) -> Option<(usize, usize)> {
    if t.anim {
        return None;
    }
    let mut asset = match &t.asset {
        SpriteAssetValue::Default | SpriteAssetValue::Index(_) => default?,
        SpriteAssetValue::Named(n) => sprites.iter().position(|a| &a.name == n)?,
    };
    let count = |a: usize| sprites.get(a).map_or(0, |s| s.characters.len()) as i64;
    let mut index: Option<usize> = None;
    if let SpriteAssetValue::Index(n) = t.asset {
        if n < 0 || i64::from(n) > count(asset) - 1 {
            return None;
        }
        index = Some(n as usize);
    }
    if let Some(name) = &t.name {
        let (a, i) = search_by_hash(sprites, asset, default, simple_hash(name))?;
        asset = a;
        index = Some(i);
    }
    if let Some(n) = t.index {
        if n < 0 || i64::from(n) > count(asset) - 1 {
            return None;
        }
        index = Some(n as usize);
    }
    Some((asset, index?))
}

/// The game's `TMP_SpriteAsset.FindMatchingMaterial(textMaterial)` (IL; a
/// Subnautica change to TMP): which material a sprite is drawn with, from
/// the names of the asset's `m_materials` (`None`: the asset's own
/// `material`). Each name's postfix is what follows its first space
/// (none: never matches); the longest postfix found in the text
/// material's name (ordinal, ignoring case) wins, the first on a tie.
pub fn find_matching_material(text_material: &str, materials: &[&str]) -> Option<usize> {
    let haystack = text_material.to_lowercase();
    let mut best: Option<(usize, usize)> = None;
    for (i, name) in materials.iter().enumerate() {
        let Some(at) = name.find(' ') else {
            continue;
        };
        let postfix = &name[at + 1..];
        if postfix.is_empty() || !haystack.contains(&postfix.to_lowercase()) {
            continue;
        }
        let len = postfix.encode_utf16().count();
        if best.is_none_or(|(_, l)| l < len) {
            best = Some((i, len));
        }
    }
    best.map(|(i, _)| i)
}

/// A sprite element's scale (`num2`), ascent and descent lines (scaled by
/// it when used) and baseline (`num17`) in `GenerateTextMesh` for font
/// size `size`, next to `font` (the text's current font asset). A sprite
/// asset with a point size scales by its own face; one without (the
/// game's `InputSymbols`) fits the glyph's height to the font's ascent
/// line.
pub fn sprite_metrics(
    fonts: &FontSet,
    font: &TmpFont,
    sprite: (usize, usize),
    size: f32,
) -> Option<(f32, f32, f32, f32)> {
    let a = fonts.sprites.get(sprite.0)?;
    let c = a.characters.get(sprite.1)?;
    let g = a.glyph_of(sprite.1)?;
    let font_scale = size / font.face.point_size as f32 * font.face.scale;
    if a.face.point_size > 0 {
        let s = size / a.face.point_size as f32 * a.face.scale;
        let scale = c.scale * g.scale * s;
        let baseline = a.face.baseline * font_scale * a.face.scale;
        Some((scale, a.face.ascent_line, a.face.descent_line, baseline))
    } else {
        let scale = font.face.ascent_line / g.metrics.height * c.scale * g.scale * font_scale;
        let k = font_scale / scale;
        let baseline = font.face.baseline * font_scale * font.face.scale;
        Some((
            scale,
            font.face.ascent_line * k,
            font.face.descent_line * k,
            baseline,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn asset(name: &str, sprites: &[&str], fallbacks: Vec<usize>) -> SpriteAsset {
        SpriteAsset {
            name: name.into(),
            face: SpriteFace::default(),
            characters: sprites
                .iter()
                .enumerate()
                .map(|(i, n)| SpriteCharacter {
                    name: (*n).into(),
                    hash: simple_hash(n),
                    unicode: 0xFFFE,
                    glyph: i as u32,
                    scale: 1.0,
                })
                .collect(),
            glyphs: Vec::new(),
            sheet: [256, 256],
            fallbacks,
        }
    }

    #[test]
    fn the_hash_is_tmps() {
        // h = ((h << 5) + h) ^ c: "A" → 65; "AB" → (65 × 33) ^ 66.
        assert_eq!(simple_hash(""), 0);
        assert_eq!(simple_hash("A"), 65);
        assert_eq!(simple_hash("AB"), (65 * 33) ^ 66);
        assert_ne!(simple_hash("ab"), simple_hash("AB"));
    }

    #[test]
    fn the_material_matches_the_longest_postfix() {
        let m = ["InputSymbols Outline", "InputSymbols Shadow", "X Out"];
        assert_eq!(find_matching_material("Aller_Rg SDF", &m), None);
        assert_eq!(find_matching_material("Aller_Rg SDF outline", &m), Some(0));
        assert_eq!(find_matching_material("Aller_Rg SDF Shadow", &m), Some(1));
        assert_eq!(find_matching_material("Out", &m), Some(2));
        assert_eq!(find_matching_material("anything", &["NoSpace"]), None);
    }

    #[test]
    fn the_search_goes_to_fallbacks_then_the_default() {
        let assets = vec![
            asset("a", &["x"], vec![1]),
            asset("b", &["y"], vec![0]),
            asset("d", &["z"], vec![]),
        ];
        assert_eq!(
            search_by_hash(&assets, 0, Some(2), simple_hash("x")),
            Some((0, 0))
        );
        assert_eq!(
            search_by_hash(&assets, 0, Some(2), simple_hash("y")),
            Some((1, 0))
        );
        assert_eq!(
            search_by_hash(&assets, 0, Some(2), simple_hash("z")),
            Some((2, 0))
        );
        assert_eq!(search_by_hash(&assets, 0, Some(2), simple_hash("w")), None);
    }
}
