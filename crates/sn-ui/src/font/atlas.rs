//! A dynamic font asset's atlas: glyphs added on demand, packed as TMP
//! packs them (`FontEngine.TryAddGlyphToTexture` with
//! `GlyphPackingMode.BestShortSideFit`, a MaxRects packer).
//!
//! *Confirmed* against `Aller_Rg SDF`'s stored state (its 2 glyphs, used
//! and free rectangles; `sn-inspect ui --fonts`):
//! - the free list starts as one rectangle `(0, 0, width − 1, height − 1)`;
//! - a glyph of bitmap size `w × h` takes a used rectangle of
//!   `(w + 2·padding + 1) × (h + 2·padding + 1)`; the glyph's own rectangle
//!   is `padding + 1` in from its left and bottom, and its distance field
//!   (`(w + 2·padding) × (h + 2·padding)`) starts 1 texel in.
//! - the packer's choice and splits give the stored free rectangles.
//!
//! The packer is ours (MaxRects as published by Jukka Jylänki: best short
//! side fit, split every intersected free rectangle, drop contained ones).

use std::collections::HashMap;

use super::{FontFile, GlyphMetrics, sdf};

/// `UnityEngine.TextCore.GlyphRect`: texels, origin bottom left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl GlyphRect {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> GlyphRect {
        GlyphRect {
            x,
            y,
            width,
            height,
        }
    }

    fn right(&self) -> i32 {
        self.x + self.width
    }

    fn top(&self) -> i32 {
        self.y + self.height
    }

    fn contains(&self, o: &GlyphRect) -> bool {
        o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.top() <= self.top()
    }

    fn intersects(&self, o: &GlyphRect) -> bool {
        o.x < self.right() && o.right() > self.x && o.y < self.top() && o.top() > self.y
    }
}

/// MaxRects with best short side fit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Packer {
    pub free: Vec<GlyphRect>,
    pub used: Vec<GlyphRect>,
}

impl Packer {
    /// An empty atlas, as TMP sets one up.
    pub fn new(width: i32, height: i32) -> Packer {
        Packer {
            free: vec![GlyphRect::new(0, 0, width - 1, height - 1)],
            used: Vec::new(),
        }
    }

    /// Places a `width × height` rectangle, or `None` if it fits nowhere.
    pub fn insert(&mut self, width: i32, height: i32) -> Option<GlyphRect> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let mut best: Option<(i32, i32, GlyphRect)> = None;
        for f in &self.free {
            if width > f.width || height > f.height {
                continue;
            }
            let dw = f.width - width;
            let dh = f.height - height;
            let key = (dw.min(dh), dw.max(dh));
            if best.is_none_or(|(s, l, _)| key < (s, l)) {
                best = Some((key.0, key.1, GlyphRect::new(f.x, f.y, width, height)));
            }
        }
        let (_, _, placed) = best?;
        let mut next = Vec::with_capacity(self.free.len() + 4);
        for f in &self.free {
            if !f.intersects(&placed) {
                next.push(*f);
                continue;
            }
            if placed.x > f.x {
                next.push(GlyphRect::new(f.x, f.y, placed.x - f.x, f.height));
            }
            if placed.right() < f.right() {
                next.push(GlyphRect::new(
                    placed.right(),
                    f.y,
                    f.right() - placed.right(),
                    f.height,
                ));
            }
            if placed.y > f.y {
                next.push(GlyphRect::new(f.x, f.y, f.width, placed.y - f.y));
            }
            if placed.top() < f.top() {
                next.push(GlyphRect::new(
                    f.x,
                    placed.top(),
                    f.width,
                    f.top() - placed.top(),
                ));
            }
        }
        // Drop rectangles inside others (keep the first of equal ones).
        let mut kept: Vec<GlyphRect> = Vec::with_capacity(next.len());
        for (i, r) in next.iter().enumerate() {
            let inside = next
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.contains(r) && (o != r || j < i));
            if !inside {
                kept.push(*r);
            }
        }
        self.free = kept;
        self.used.push(placed);
        Some(placed)
    }
}

/// `UnityEngine.TextCore.Glyph`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub index: u32,
    pub metrics: GlyphMetrics,
    pub rect: GlyphRect,
    pub scale: f32,
    pub atlas_index: i32,
}

/// What happened to a character asked of the atlas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Added {
    /// Already there.
    Cached(Glyph),
    /// Rendered and packed now.
    New(Glyph),
    /// Not in the font (TMP then tries the fallbacks).
    Missing,
    /// No room left in the atlas.
    Full,
}

/// A dynamic font asset's atlas (one page) with its glyph and character
/// tables. Of the game's assets only `NotoSansCJKsc-Regular SDF` has
/// multi-atlas on (a new page when one is full); that is not ported
/// (`Added::Full` is returned and logged).
pub struct DynamicAtlas {
    pub width: i32,
    pub height: i32,
    pub padding: i32,
    pub point_size: i32,
    /// Alpha8, bottom row first.
    pub texels: Vec<u8>,
    pub packer: Packer,
    pub glyphs: HashMap<u32, Glyph>,
    /// Unicode → glyph index.
    pub characters: HashMap<u32, u32>,
}

impl DynamicAtlas {
    pub fn new(width: i32, height: i32, padding: i32, point_size: i32) -> DynamicAtlas {
        let (w, h) = (width.max(0) as usize, height.max(0) as usize);
        DynamicAtlas {
            width,
            height,
            padding,
            point_size,
            texels: vec![0; w * h],
            packer: Packer::new(width, height),
            glyphs: HashMap::new(),
            characters: HashMap::new(),
        }
    }

    /// `TMP_FontAsset.TryAddCharacterInternal`: the character's glyph,
    /// rendered and packed if it is new.
    pub fn add_character(&mut self, font: &FontFile, c: char) -> Added {
        if let Some(g) = self
            .characters
            .get(&u32::from(c))
            .and_then(|i| self.glyphs.get(i))
        {
            return Added::Cached(*g);
        }
        let Some(index) = font.glyph_index(c) else {
            return Added::Missing;
        };
        if let Some(g) = self.glyphs.get(&index).copied() {
            self.characters.insert(u32::from(c), index);
            return Added::Cached(g);
        }
        let Ok(scaled) = font.glyph(index, self.point_size) else {
            return Added::Missing;
        };
        let p = self.padding;
        let (w, h) = (scaled.width as i32, scaled.height as i32);
        // An empty glyph (space) takes no room.
        let rect = if w == 0 || h == 0 {
            GlyphRect::default()
        } else {
            let Some(used) = self.packer.insert(w + 2 * p + 1, h + 2 * p + 1) else {
                return Added::Full;
            };
            let field = sdf::glyph_field(&scaled, p.max(0) as u32);
            self.blit(&field, used.x + 1, used.y + 1);
            GlyphRect::new(used.x + p + 1, used.y + p + 1, w, h)
        };
        let glyph = Glyph {
            index,
            metrics: scaled.metrics,
            rect,
            scale: 1.0,
            atlas_index: 0,
        };
        self.glyphs.insert(index, glyph);
        self.characters.insert(u32::from(c), index);
        Added::New(glyph)
    }

    /// Copies a field to `(x, y)`; texels outside the atlas are dropped.
    fn blit(&mut self, field: &sdf::Field, x: i32, y: i32) {
        let (aw, ah) = (self.width.max(0) as i64, self.height.max(0) as i64);
        for j in 0..field.height {
            let ty = i64::from(y) + j as i64;
            if !(0..ah).contains(&ty) {
                continue;
            }
            for i in 0..field.width {
                let tx = i64::from(x) + i as i64;
                if !(0..aw).contains(&tx) {
                    continue;
                }
                self.texels[(ty * aw + tx) as usize] = field.texels[j * field.width + i];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_short_side_fit_places_and_splits() {
        let mut p = Packer::new(1024, 1024);
        assert_eq!(p.free, [GlyphRect::new(0, 0, 1023, 1023)]);
        assert_eq!(p.insert(75, 31), Some(GlyphRect::new(0, 0, 75, 31)));
        let mut free = p.free.clone();
        free.sort();
        assert_eq!(
            free,
            [
                GlyphRect::new(0, 31, 1023, 992),
                GlyphRect::new(75, 0, 948, 1023)
            ]
        );
        // The second goes beside the first (short side 883 < 958).
        assert_eq!(p.insert(65, 26), Some(GlyphRect::new(75, 0, 65, 26)));
        // Every free rectangle is clear of every used one and none holds
        // another.
        for f in &p.free {
            assert!(p.used.iter().all(|u| !f.intersects(u)), "{f:?}");
            assert!(p.free.iter().filter(|o| o.contains(f)).count() == 1);
        }
        assert_eq!(p.insert(2000, 1), None);
        assert_eq!(p.insert(0, 5), None);
    }

    #[test]
    fn a_field_off_the_atlas_is_clipped() {
        let mut a = DynamicAtlas::new(4, 4, 1, 10);
        let field = sdf::Field {
            width: 3,
            height: 3,
            texels: vec![9; 9],
        };
        a.blit(&field, -2, 2);
        a.blit(&field, 3, -1);
        let set: Vec<usize> = (0..16).filter(|&i| a.texels[i] == 9).collect();
        // (0, 2), (0, 3) from the first; (3, 0), (3, 1) from the second.
        assert_eq!(set, [3, 7, 8, 12]);
    }

    #[test]
    fn packing_fills_without_overlap() {
        let mut p = Packer::new(64, 64);
        let mut placed = Vec::new();
        for i in 0..200 {
            if let Some(r) = p.insert(3 + i % 7, 2 + i % 5) {
                placed.push(r);
            }
        }
        assert!(placed.len() > 50);
        for (i, a) in placed.iter().enumerate() {
            assert!(a.right() <= 63 && a.top() <= 63);
            for b in &placed[i + 1..] {
                assert!(!a.intersects(b), "{a:?} {b:?}");
            }
        }
    }
}
