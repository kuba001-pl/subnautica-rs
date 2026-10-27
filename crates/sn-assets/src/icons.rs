//! What `uGUI_ItemIcon` and `uGUI_Icon` read (M9u7c1; `docs/formats/ui.md`
//! § Picked-up icons): `iconMaterial` (`new Material(ShaderManager.
//! preloadedShaders.UIIcon)`, the shader's own defaults), a sprite's mesh
//! as `SpriteManager.GetSpriteCache` takes it, and the nodes the notifier's
//! `GetInstance` makes by code.

use sn_ui::icon::{IconSprite, NEW_RECT_SIZE, sprite_uv};
use sn_ui::layout::RectTransform;
use sn_ui::material::UiMaterial;
use sn_ui::notifier::{IconAnimator, NotifierConfig};
use sn_unity::{Catalog, Fields, Shader, Sprite, Value};

use crate::hud::curve_field;
use crate::ui_scene::CodeNode;
use crate::{Assets, Layouts, ObjectRef, Result, UiGraphicKind, UiTree};

const NOTIFIER: &str = "uGUI_IconNotifier";

/// The Addressables key `ShaderManager.Init` loads.
pub const PRELOADED_SHADERS_KEY: &str = "Shaders/PreloadedShaders.asset";
const MONO_BEHAVIOUR: i32 = 114;
const SHADER: i32 = 48;
const SPRITE: i32 = 213;

/// `new Material(shader)`: every declared property at its default (colours
/// and vectors as stored; textures the default's, none of ours, with the
/// identity `_ST`).
pub fn new_material(name: &str, shader: &Shader) -> UiMaterial {
    let mut m = UiMaterial {
        name: name.into(),
        shader: shader.name.clone(),
        ..UiMaterial::default()
    };
    for p in &shader.properties {
        match p.kind {
            0 | 1 => {
                m.colors.insert(p.name.clone(), p.default);
            }
            2 | 3 => {
                m.floats.insert(p.name.clone(), p.default[0]);
            }
            4 => {
                m.textures.insert(p.name.clone(), 0);
                m.colors
                    .insert(format!("{}_ST", p.name), [1.0, 1.0, 0.0, 0.0]);
            }
            _ => {}
        }
    }
    m
}

/// `uGUI_ItemIcon.CreateForeground`'s copy of `iconMaterial`: `GRAYSCALE`
/// off and `_Chroma` 1 (`foregroundChroma`), `NOTIFICATION` off and
/// `_NotificationStrength` 0 (`godRaysIntensity`).
pub fn foreground_material(icon_material: &UiMaterial) -> UiMaterial {
    let mut m = icon_material.clone();
    m.keywords
        .retain(|k| k != "GRAYSCALE" && k != "NOTIFICATION");
    m.floats.insert("_Chroma".into(), 1.0);
    m.floats.insert("_NotificationStrength".into(), 0.0);
    m
}

fn f32_field(f: &Fields, what: &str, name: &str) -> Result<f32> {
    f.get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .ok_or_else(|| format!("{what}.{name}: not read"))
}

impl Assets<'_> {
    /// `uGUI_IconNotifier`'s stored fields in `tree` (`uGUI`): `canvas`
    /// (the node holding that `RectTransform`), `interval`, `iconSize` and
    /// the `animators` (clamped, unweighted curves).
    pub fn notifier_config(&self, layouts: &Layouts, tree: &UiTree) -> Result<NotifierConfig> {
        let (_, c) = tree
            .scripts(NOTIFIER)
            .next()
            .ok_or("no uGUI_IconNotifier")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let p = f
            .get("canvas")
            .and_then(Value::as_pptr)
            .filter(|p| !p.is_null())
            .ok_or("uGUI_IconNotifier.canvas: not set")?;
        let rect = self
            .resolve(&c.object.file, p)?
            .ok_or("uGUI_IconNotifier.canvas: not found")?;
        let canvas = tree
            .nodes
            .iter()
            .position(|n| n.components.iter().any(|c| c.object.key() == rect.key()))
            .ok_or("uGUI_IconNotifier.canvas: not in the tree")?;
        let mut animators = Vec::new();
        for (k, a) in f
            .get("animators")
            .and_then(Value::as_array)
            .ok_or("uGUI_IconNotifier.animators: not read")?
            .iter()
            .enumerate()
        {
            let a = a
                .as_struct()
                .ok_or_else(|| format!("uGUI_IconNotifier.animators[{k}]: not a struct"))?;
            let what = format!("{NOTIFIER}.animators[{k}]");
            let curve = |n: &str| curve_field(a, n).map_err(|e| format!("{what}: {e}"));
            animators.push(IconAnimator {
                duration: f32_field(a, &what, "duration")?,
                x: curve("x")?,
                y: curve("y")?,
                scale: curve("scale")?,
                alpha: curve("alpha")?,
                oscillate: a
                    .get("oscillate")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| format!("{what}.oscillate: not read"))?,
                osc_offset: f32_field(a, &what, "oscOffset")?,
                osc_reduction: f32_field(a, &what, "oscReduction")?,
                osc_frequency: f32_field(a, &what, "oscFrequency")?,
                osc_scale: f32_field(a, &what, "oscScale")?,
            });
        }
        Ok(NotifierConfig {
            canvas,
            interval: f32_field(&f, NOTIFIER, "interval")?,
            icon_size: f32_field(&f, NOTIFIER, "iconSize")?,
            animators,
        })
    }

    /// `ShaderManager.preloadedShaders.<field>` (`UIIcon`, …): the shader
    /// the `PreloadedShaders` asset's field points at.
    pub fn preloaded_shader(
        &self,
        catalog: &Catalog,
        layouts: &Layouts,
        field: &str,
    ) -> Result<Shader> {
        let location = catalog
            .locate(PRELOADED_SHADERS_KEY)
            .into_iter()
            .next()
            .ok_or_else(|| format!("{PRELOADED_SHADERS_KEY}: not in the catalog"))?;
        let o = self
            .catalog_object(&location, MONO_BEHAVIOUR)?
            .ok_or_else(|| format!("{PRELOADED_SHADERS_KEY}: not in its bundle"))?;
        let (_, f) = self.object_fields(layouts, &o)?;
        let p = f
            .get(field)
            .and_then(Value::as_pptr)
            .filter(|p| !p.is_null())
            .ok_or_else(|| format!("PreloadedShaders.{field}: not set"))?;
        let s = self
            .resolve(&o.file, p)?
            .ok_or_else(|| format!("PreloadedShaders.{field}: not found"))?;
        let (info, d) = s.data()?;
        if info.class_id != SHADER {
            return Err(format!("PreloadedShaders.{field}: class {}", info.class_id));
        }
        Shader::parse(d, s.file.file().big_endian).map_err(|e| format!("Shader: {e}"))
    }

    /// `uGUI_ItemIcon.iconMaterial`: a new material of `UIIcon`. Its
    /// `_NotificationOverlayTex` (`Resources.Load("Sprites/
    /// NotificationOverlay")`, [`Assets::resource_texture`]) is set by the
    /// caller that draws the `NOTIFICATION` variants (M9u8b5b).
    pub fn icon_material(&self, catalog: &Catalog, layouts: &Layouts) -> Result<UiMaterial> {
        let shader = self.preloaded_shader(catalog, layouts, "UIIcon")?;
        Ok(new_material("uGUI_ItemIcon.iconMaterial", &shader))
    }

    /// `SpriteManager.GetSpriteCache(sprite)` and the texture `uGUI_Icon`
    /// samples (`sprite.texture`: the atlas page for a sprite packed
    /// late): `vertices` and `triangles` from the sprite's mesh, `uv`
    /// from the render data it is drawn with ([`sprite_uv`]).
    pub fn icon_sprite(&self, sprite: &ObjectRef) -> Result<(IconSprite, Option<ObjectRef>)> {
        let (info, d) = sprite.data()?;
        if info.class_id != SPRITE {
            return Err(format!("class {} is not a Sprite", info.class_id));
        }
        let s =
            Sprite::parse(d, sprite.file.file().big_endian).map_err(|e| format!("Sprite: {e}"))?;
        let (vertices, triangles) = s.mesh.decode().map_err(|e| format!("{}: {e}", s.name))?;
        let (rd, page) = self.sprite_render_data(sprite, &s)?;
        let size = match &page {
            Some(p) => {
                let (_, td) = p.data()?;
                let t = sn_unity::Texture2D::parse(td, p.file.file().big_endian)
                    .map_err(|e| format!("Texture2D: {e}"))?;
                [t.width as f32, t.height as f32]
            }
            None => [1.0, 1.0],
        };
        let uv = vertices
            .iter()
            .map(|v| sprite_uv(*v, rd.uv_transform, size))
            .collect();
        Ok((
            IconSprite {
                size: [s.rect[2], s.rect[3]],
                pixels_per_unit: s.pixels_to_units,
                vertices,
                uv,
                triangles,
            },
            page,
        ))
    }
}

/// `uGUI_IconNotifier.GetInstance`'s icon, as made by code under the
/// node `parent` of a scene: `new GameObject("NotifierIcon")` with a
/// `uGUI_ItemIcon` (a `RectTransform` with a new one's values, then
/// `Init`'s `SetParams(anchor (0, 0), pivot (0.5, 0.5))`: local position
/// and rotation zero, scale one; `SetActive(false)`), and the child
/// `CreateForeground` makes (`"Foreground"`, a `uGUI_Icon` disabled,
/// anchors and pivot at the centre) with `material`. The game makes the
/// foreground on the first `SetForegroundSprite`, which every icon gets
/// before it is shown; made with the icon here, disabled, it draws the
/// same. Node 0 is the icon, node 1 the foreground.
pub fn notifier_icon_nodes(layer: u32, material: &UiMaterial) -> Vec<CodeNode> {
    let rect = |anchor: [f32; 2]| RectTransform {
        anchor_min: anchor,
        anchor_max: anchor,
        anchored_position: [0.0, 0.0],
        size_delta: NEW_RECT_SIZE,
        pivot: [0.5, 0.5],
        scale: [1.0, 1.0, 1.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        z: 0.0,
    };
    vec![
        CodeNode {
            name: "NotifierIcon".into(),
            parent: None,
            rect: rect([0.0, 0.0]),
            active_self: false,
            layer,
            graphic: None,
        },
        CodeNode {
            name: "Foreground".into(),
            parent: Some(0),
            rect: rect([0.5, 0.5]),
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
        },
    ]
}
