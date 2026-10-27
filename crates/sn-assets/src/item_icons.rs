//! What the inventory tab's icons read (M9u8b2a; `docs/formats/ui.md` §
//! The inventory tab's icons): the background sprites
//! `SpriteManager.GetBackground` loads from `Resources`, an item's size
//! and background type (`TechData`), and the icons' nodes made by code.

use sn_ui::icon::NEW_RECT_SIZE;
use sn_ui::layout::{RectTransform, V2};
use sn_ui::material::UiMaterial;
use sn_unity::Sprite;

use crate::code::TechDefaults;
use crate::icons::foreground_material;
use crate::ui_scene::CodeNode;
use crate::water::resources;
use crate::{Assets, ObjectRef, Result, TechData, UiGraphicKind};

const SPRITE: i32 = 213;
const TEXTURE_2D: i32 = 28;

/// `SpriteManager.GetBackground(BackgroundType)`'s Resources paths, by the
/// type's value (Normal, Blueprint, PlantWater, PlantWaterSeed, PlantAir,
/// PlantAirSeed, ExosuitArm); other values take `Normal`.
pub const BACKGROUND_PATHS: [&str; 7] = [
    "Sprites/Backgrounds/Normal",
    "Sprites/Backgrounds/Blueprint",
    "Sprites/Backgrounds/PlantWater",
    "Sprites/Backgrounds/PlantWater",
    "Sprites/Backgrounds/PlantAir",
    "Sprites/Backgrounds/PlantAir",
    "Sprites/Backgrounds/ExosuitArm",
];

/// A sprite `Resources.Load<Sprite>` found: the object, its name and
/// `rect` size.
#[derive(Clone, Debug)]
pub struct ResourceSprite {
    pub object: ObjectRef,
    pub name: String,
    pub size: V2,
}

/// `GetBackground`'s sprites, by background type (`None`: not found; the
/// game logs an error and the icon has no background).
#[derive(Clone, Debug)]
pub struct BackgroundSprites(pub Vec<Option<ResourceSprite>>);

impl BackgroundSprites {
    /// The sprite for background type `t`.
    pub fn get(&self, t: i32) -> Option<&ResourceSprite> {
        let i = usize::try_from(t)
            .ok()
            .filter(|&i| i < BACKGROUND_PATHS.len())
            .unwrap_or(0);
        self.0.get(i).and_then(Option::as_ref)
    }
}

/// `TechData.GetItemSize(tech)` and `GetBackgroundType(tech)`: the entry's
/// values, else the defaults.
pub fn tech_icon(data: &TechData, defaults: &TechDefaults, tech: i32) -> ((i32, i32), i32) {
    let e = data.get(tech);
    let size = e.and_then(|e| e.item_size).unwrap_or(defaults.item_size);
    let background = e
        .and_then(|e| e.background_type)
        .unwrap_or(defaults.background_type);
    ((size[0], size[1]), background)
}

impl Assets<'_> {
    /// `Resources.Load<Sprite>(path)`.
    pub fn resource_sprite(&self, path: &str) -> Result<Option<ResourceSprite>> {
        let (file, found) = resources(self, &path.to_lowercase())?;
        for p in found {
            let Some(o) = self.resolve(&file, p)? else {
                continue;
            };
            let (info, d) = o.data()?;
            if info.class_id != SPRITE {
                continue;
            }
            let s =
                Sprite::parse(d, o.file.file().big_endian).map_err(|e| format!("{path}: {e}"))?;
            return Ok(Some(ResourceSprite {
                object: o,
                name: s.name,
                size: [s.rect[2], s.rect[3]],
            }));
        }
        Ok(None)
    }

    /// `Resources.Load<Texture2D>(path)` (M9u8b5b: `uGUI_ItemIcon.
    /// iconMaterial`'s `Sprites/NotificationOverlay`); `None` if absent.
    pub fn resource_texture(&self, path: &str) -> Result<Option<ObjectRef>> {
        let (file, found) = resources(self, &path.to_lowercase())?;
        for p in found {
            let Some(o) = self.resolve(&file, p)? else {
                continue;
            };
            if o.data()?.0.class_id == TEXTURE_2D {
                return Ok(Some(o));
            }
        }
        Ok(None)
    }

    /// Every background `GetBackground` can give.
    pub fn background_sprites(&self) -> Result<BackgroundSprites> {
        let mut out = Vec::new();
        for p in BACKGROUND_PATHS {
            out.push(self.resource_sprite(p)?);
        }
        Ok(BackgroundSprites(out))
    }
}

/// An icon made by code (node 0 the `uGUI_ItemIcon`, 1 its `Background`,
/// 2 its `Foreground`; each `uGUI_Icon` disabled with its own copy of
/// `iconMaterial`): `uGUI_ItemsContainer.OnAddItem`'s `Item Icon` (anchor
/// and pivot (0, 1)) or, with `slot`, `uGUI_EquipmentSlot.icon`'s
/// `Equipment Slot Icon` (anchor and pivot at the centre).
pub fn item_icon_nodes(layer: u32, material: &UiMaterial, slot: bool) -> Vec<CodeNode> {
    let rect = |anchor: [f32; 2], pivot: [f32; 2]| RectTransform {
        anchor_min: anchor,
        anchor_max: anchor,
        anchored_position: [0.0, 0.0],
        size_delta: NEW_RECT_SIZE,
        pivot,
        scale: [1.0, 1.0, 1.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        z: 0.0,
    };
    let (name, anchor) = if slot {
        ("Equipment Slot Icon", [0.5, 0.5])
    } else {
        ("Item Icon", [0.0, 1.0])
    };
    let icon = |name: &str| CodeNode {
        name: name.into(),
        parent: Some(0),
        rect: rect([0.5, 0.5], [0.5, 0.5]),
        active_self: true,
        layer,
        graphic: Some((
            UiGraphicKind::Icon {
                sprite: None,
                color: [1.0; 4],
                material: foreground_material(material),
            },
            false,
        )),
    };
    vec![
        CodeNode {
            name: name.into(),
            parent: None,
            rect: rect(anchor, anchor),
            active_self: true,
            layer,
            graphic: None,
        },
        icon("Background"),
        icon("Foreground"),
    ]
}
