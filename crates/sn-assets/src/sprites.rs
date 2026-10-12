//! The game's sprites by name, as `SpriteManager` finds them (M9u2,
//! `docs/formats/ui.md` § Sprite atlases). Confirmed from the code
//! (`sn-inspect code --il SpriteManager …`):
//! - `.cctor` loads every `SpriteAtlas` with the Addressables label
//!   `SpriteAtlases`, and maps the groups to atlas names: None →
//!   `Default`, Item → `Items`, Background → `Backgrounds`, Category →
//!   `Categories`, Log → `Log`, Tab → `Tabs`, Pings → `Pings`,
//!   ItemActions → `ItemActions`;
//! - `OnLoadedSpriteAtlases` keys each atlas's sprites by name (the clone's
//!   name without `(Clone)`), ignoring case;
//! - `Get(group, name, fallback)` looks the name up in the group's atlas,
//!   else gives the fallback; `Get(TechType)` uses the Item group, the
//!   tech type's name (`AsString(false)`) and `defaultSprite`, which is
//!   `Default`'s `Unknown`.

use std::collections::HashMap;

use sn_unity::{Catalog, Sprite, SpriteAtlas, SpriteRenderData};

use crate::{Assets, ObjectRef, Result};

const SPRITE: i32 = 213;
const SPRITE_ATLAS: i32 = 687_078_895;
/// The Addressables label `SpriteManager`'s `.cctor` loads.
pub const SPRITE_ATLASES_LABEL: &str = "SpriteAtlases";

/// `SpriteManager.Group`, by value.
pub const GROUP_ATLASES: [&str; 8] = [
    "Default",
    "Items",
    "Backgrounds",
    "Categories",
    "Log",
    "Tabs",
    "Pings",
    "ItemActions",
];
pub const GROUP_ITEM: usize = 1;

/// A packed sprite: its own data and where its atlas put it.
#[derive(Clone)]
pub struct AtlasSprite {
    pub sprite: Sprite,
    pub object: ObjectRef,
    /// The atlas's render data for it (page and rectangle).
    pub packed: SpriteRenderData,
    /// The page texture.
    pub page: ObjectRef,
}

/// One loaded atlas.
pub struct LoadedAtlas {
    pub atlas: SpriteAtlas,
    pub object: ObjectRef,
    /// Lower-case name → sprite.
    pub sprites: HashMap<String, AtlasSprite>,
}

/// Every atlas `SpriteManager` loads, by name.
pub struct Sprites {
    pub atlases: HashMap<String, LoadedAtlas>,
    /// Packed sprites that did not resolve (name, why).
    pub misses: Vec<(String, String)>,
}

impl Sprites {
    /// `SpriteManager.Get(group, name, fallback)` with `fallback`
    /// `defaultSprite`.
    pub fn get(&self, group: usize, name: &str) -> Option<&AtlasSprite> {
        self.get_no_default(group, name)
            .or_else(|| self.default_sprite())
    }

    /// The same without the fallback (`None` where the game falls back).
    pub fn get_no_default(&self, group: usize, name: &str) -> Option<&AtlasSprite> {
        let atlas = GROUP_ATLASES.get(group)?;
        self.atlases.get(*atlas)?.sprites.get(&name.to_lowercase())
    }

    /// `SpriteManager.defaultSprite`: `Default`'s `Unknown`.
    pub fn default_sprite(&self) -> Option<&AtlasSprite> {
        self.get_no_default(0, "Unknown")
    }

    /// `SpriteManager.Get(TechType)`, by the tech type's name.
    pub fn item_icon(&self, tech_name: &str) -> Option<&AtlasSprite> {
        self.get(GROUP_ITEM, tech_name)
    }
}

impl Assets<'_> {
    /// Loads the atlases with the `SpriteAtlases` label and their sprites.
    pub fn sprites(&self, catalog: &Catalog) -> Result<Sprites> {
        let mut atlases = HashMap::new();
        let mut misses = Vec::new();
        for location in catalog
            .locate(SPRITE_ATLASES_LABEL)
            .into_iter()
            .filter(|l| l.resource_type == "UnityEngine.U2D.SpriteAtlas")
        {
            let object = self
                .catalog_object(&location, SPRITE_ATLAS)?
                .ok_or_else(|| format!("{}: not in its bundle", location.internal_id))?;
            let (_, data) = object.data()?;
            let big_endian = object.file.file().big_endian;
            let atlas = SpriteAtlas::parse(data, big_endian)
                .map_err(|e| format!("{}: SpriteAtlas: {e}", location.internal_id))?;
            let mut sprites = HashMap::new();
            for (pptr, name) in atlas.packed_sprites.iter().zip(&atlas.packed_sprite_names) {
                let found = (|| -> Result<AtlasSprite> {
                    let o = self
                        .resolve(&object.file, *pptr)?
                        .ok_or("null sprite reference")?;
                    let (info, d) = o.data()?;
                    if info.class_id != SPRITE {
                        return Err(format!("class {} is not a Sprite", info.class_id));
                    }
                    let sprite = Sprite::parse(d, o.file.file().big_endian)
                        .map_err(|e| format!("Sprite: {e}"))?;
                    let packed = atlas
                        .render_data_of(&sprite.render_data_key)
                        .ok_or("no render data in the atlas")?
                        .clone();
                    let page = self
                        .resolve(&object.file, packed.texture)?
                        .ok_or("null atlas page")?;
                    Ok(AtlasSprite {
                        sprite,
                        object: o,
                        packed,
                        page,
                    })
                })();
                match found {
                    Ok(s) => {
                        sprites.insert(name.to_lowercase(), s);
                    }
                    Err(e) => misses.push((format!("{}/{name}", atlas.name), e)),
                }
            }
            atlases.insert(
                atlas.name.clone(),
                LoadedAtlas {
                    atlas,
                    object,
                    sprites,
                },
            );
        }
        Ok(Sprites { atlases, misses })
    }

    /// A packed sprite's pixels, cut from its atlas page: RGBA8, top row
    /// first, `(width, height, pixels)`.
    pub fn sprite_pixels(&self, s: &AtlasSprite) -> Result<(usize, usize, Vec<u8>)> {
        let page = self.texture(&s.page)?;
        let rgba = page
            .texture
            .decode_rgba(&page.data)
            .map_err(|e| format!("atlas page: {e}"))?;
        cut(
            &rgba,
            page.texture.width as usize,
            page.texture.height as usize,
            s.packed.texture_rect,
        )
    }
}

/// The pixels of `rect` (x, y from the bottom, width, height) of an RGBA8
/// image stored top row first.
pub fn cut(
    rgba: &[u8],
    width: usize,
    height: usize,
    rect: [f32; 4],
) -> Result<(usize, usize, Vec<u8>)> {
    let [x, y, w, h] = rect.map(|v| v.round());
    if x < 0.0 || y < 0.0 || w < 1.0 || h < 1.0 {
        return Err(format!("rectangle {rect:?} is empty or outside"));
    }
    let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
    if x + w > width || y + h > height || rgba.len() < width * height * 4 {
        return Err(format!("rectangle {rect:?} outside {width}×{height}"));
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let top = height - (y + h) + row;
        let start = (top * width + x) * 4;
        out.extend_from_slice(&rgba[start..start + w * 4]);
    }
    Ok((w, h, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_from_the_bottom() {
        // 2×3 image, top row first: rows 0 (top), 1, 2 (bottom).
        let mut img = Vec::new();
        for row in 0..3u8 {
            for col in 0..2u8 {
                img.extend([row, col, 0, 255]);
            }
        }
        // The bottom row, both columns.
        let (w, h, px) = cut(&img, 2, 3, [0.0, 0.0, 2.0, 1.0]).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(&px[..4], &[2, 0, 0, 255]);
        // The top-right pixel.
        let (_, _, px) = cut(&img, 2, 3, [1.0, 2.0, 1.0, 1.0]).unwrap();
        assert_eq!(px, [0, 1, 0, 255]);
        assert!(cut(&img, 2, 3, [1.0, 2.0, 2.0, 1.0]).is_err());
        assert!(cut(&img, 2, 3, [0.0, 0.0, 0.0, 1.0]).is_err());
        assert!(cut(&img, 2, 3, [-1.0, 0.0, 1.0, 1.0]).is_err());
        assert!(cut(&img[..8], 2, 3, [0.0, 0.0, 1.0, 1.0]).is_err());
    }
}
