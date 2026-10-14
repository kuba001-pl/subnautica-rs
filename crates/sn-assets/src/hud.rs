//! The HUD's scripts' nodes and fields (M9u6b1; `docs/formats/ui.md`
//! § The HUD's bars), for `sn_ui::hud`'s rules: `uGUI_SceneHUD`'s elements
//! and the four bar scripts' `bar`, `text` and fields; their pulse curves,
//! legacy clips and icons (M9u6b2, § The bars' animations); and
//! `uGUI_DepthCompass`'s (M9u6c1, § The depth and compass). (`Survival`'s
//! start food and water are in its code: `crate::survival_start`.)

use std::collections::HashMap;

use sn_ui::clip::{Channel, Clip, Curve, Key, Track};
use sn_ui::depth::DepthConfig;
use sn_ui::hud::{BarConfig, BarKind, HudConfig, IconMotion, Overlays, Pulse};
use sn_unity::{AnimationCurve, EditorCurveKind, Fields, PPtr, Value};

use crate::{Assets, Layouts, ObjectRef, Result, SpriteId, UiGraphicKind, UiScene, UiTree};

/// `RectTransform`'s class.
const RECT_TRANSFORM: i32 = 224;
/// `CanvasGroup`'s class.
const CANVAS_GROUP: i32 = 225;
/// `AnimationCurve`'s clamp wrap mode (pre and post infinity).
const CLAMP: i64 = 2;

/// An `AnimationCurve` field: unweighted keys and clamped ends, the only
/// kind the HUD's rules evaluate.
fn curve_field(f: &Fields, name: &str) -> Result<Curve> {
    let c = f
        .get(name)
        .and_then(Value::as_struct)
        .ok_or_else(|| format!("no curve {name}"))?;
    for end in ["m_PreInfinity", "m_PostInfinity"] {
        if c.get(end).and_then(Value::as_i64) != Some(CLAMP) {
            return Err(format!("{name}: {end} is not clamp"));
        }
    }
    let mut keys = Vec::new();
    for k in c.get("m_Curve").and_then(Value::as_array).unwrap_or(&[]) {
        let k = k
            .as_struct()
            .ok_or_else(|| format!("{name}: a key is not a struct"))?;
        let n = |field: &str| {
            k.get(field)
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .ok_or_else(|| format!("{name}: a key without {field}"))
        };
        if k.get("weightedMode").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Err(format!("{name}: a weighted key"));
        }
        keys.push(Key {
            time: n("time")?,
            value: n("value")?,
            in_slope: n("inSlope")?,
            out_slope: n("outSlope")?,
        });
    }
    Ok(Curve { keys })
}

/// A clip's editor curve as ours, if clamped and unweighted.
fn curve_of(c: &AnimationCurve, what: &str) -> Result<Curve> {
    if c.pre_infinity != CLAMP as i32 || c.post_infinity != CLAMP as i32 {
        return Err(format!("{what}: not clamped"));
    }
    if c.keys.iter().any(|k| k.weighted_mode != 0) {
        return Err(format!("{what}: a weighted key"));
    }
    Ok(Curve {
        keys: c
            .keys
            .iter()
            .map(|k| Key {
                time: k.time,
                value: k.value,
                in_slope: k.in_slope,
                out_slope: k.out_slope,
            })
            .collect(),
    })
}

fn pptr(f: &Fields, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .filter(|p| !p.is_null())
        .ok_or_else(|| format!("no reference {name}"))
}

fn num(f: &Fields, name: &str) -> Result<f32> {
    f.get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .ok_or_else(|| format!("no number {name}"))
}

impl Assets<'_> {
    /// A node's stored `localScale` (its `RectTransform`).
    fn stored_scale(&self, tree: &UiTree, node: usize) -> Result<[f32; 3]> {
        let n = &tree.nodes[node];
        let c = n
            .components
            .iter()
            .find(|c| c.class_id == RECT_TRANSFORM)
            .ok_or_else(|| format!("{}: no RectTransform", n.path))?;
        let (_, d) = c.object.data()?;
        let r = sn_unity::RectTransform::parse(d, c.object.file.file().big_endian)
            .map_err(|e| format!("{}: RectTransform: {e}", n.path))?;
        Ok(r.scale)
    }

    /// The legacy `Animation` `anim` (on node `at`) as our clip: its
    /// only clip (the `Animation`'s state 0), each editor curve's path
    /// resolved below `at`. A scale or a `CanvasGroup`'s `m_Alpha` only.
    fn pulse_clip(&self, tree: &UiTree, anim: &ObjectRef, at: usize) -> Result<Clip> {
        let a = self.legacy_animation(anim)?;
        let mut clips = a.clips.iter().filter(|p| !p.is_null());
        let (Some(p), None) = (clips.next(), clips.next()) else {
            return Err(format!(
                "{}: Animation without exactly one clip",
                tree.nodes[at].path
            ));
        };
        let o = self
            .resolve(&anim.file, *p)?
            .ok_or("Animation clip not found")?;
        let clip = self.animation_clip(&o)?;
        if !clip.legacy {
            return Err(format!("{}: not a legacy clip", clip.name));
        }
        let base = &tree.nodes[at].path;
        let mut tracks = Vec::new();
        for e in &clip.editor {
            let path = if e.path.is_empty() {
                base.clone()
            } else {
                format!("{base}/{}", e.path)
            };
            let node = tree
                .nodes
                .iter()
                .position(|n| n.path == path)
                .ok_or_else(|| format!("{}: {path} not in the tree", clip.name))?;
            let what = format!("{} {path}", clip.name);
            let channel = match (e.kind, e.class_id, e.attribute.as_str()) {
                (EditorCurveKind::Scale, _, _) => Channel::Scale([
                    curve_of(&e.curves[0], &what)?,
                    curve_of(&e.curves[1], &what)?,
                    curve_of(&e.curves[2], &what)?,
                ]),
                (EditorCurveKind::Float, CANVAS_GROUP, "m_Alpha") => {
                    Channel::GroupAlpha(curve_of(&e.curves[0], &what)?)
                }
                (k, class, attr) => {
                    return Err(format!("{what}: {k:?} {attr} of class {class} not handled"));
                }
            };
            tracks.push(Track { node, channel });
        }
        Ok(Clip {
            name: clip.name.clone(),
            length: clip.stop_time - clip.start_time,
            tracks,
        })
    }

    /// `uGUI_SceneHUD` and its bars in `tree` (`uGUI`).
    pub fn hud_config(&self, layouts: &Layouts, tree: &UiTree) -> Result<HudConfig> {
        let (_, scene_hud) = tree
            .scripts("uGUI_SceneHUD")
            .next()
            .ok_or("no uGUI_SceneHUD")?;
        let (_, f) = self.script_fields(layouts, scene_hud)?;
        let from = &scene_hud.object;
        let node = |name: &str| -> Result<usize> {
            self.node_of(tree, from, pptr(&f, name)?)?
                .ok_or_else(|| format!("uGUI_SceneHUD.{name}: not in the tree"))
        };
        let graphics = self.ui_graphics(layouts, tree)?;
        let mut bars = Vec::new();
        for (kind, field, script) in [
            (BarKind::Oxygen, "barOxygen", "uGUI_OxygenBar"),
            (BarKind::Health, "barHealth", "uGUI_HealthBar"),
            (BarKind::Food, "barFood", "uGUI_FoodBar"),
            (BarKind::Water, "barWater", "uGUI_WaterBar"),
        ] {
            let root = node(field)?;
            let (_, c) = tree
                .scripts(script)
                .find(|(n, _)| *n == root)
                .ok_or_else(|| format!("{script} not on {}", tree.nodes[root].path))?;
            let (_, b) = self.script_fields(layouts, c)?;
            let of = |name: &str| -> Result<usize> {
                self.node_of(tree, &c.object, pptr(&b, name)?)?
                    .ok_or_else(|| format!("{script}.{name}: not in the tree"))
            };
            let bar = of("bar")?;
            let circular = graphics
                .iter()
                .find_map(|g| match &g.kind {
                    UiGraphicKind::CircularBar { bar: cb, .. } if g.node == bar => Some(*cb),
                    _ => None,
                })
                .ok_or_else(|| format!("{script}.bar is not a uGUI_CircularBar"))?;
            // The pulse: `animation` (oxygen, health) or `pulseAnimation`.
            let field = if b.get("animation").is_some() {
                "animation"
            } else {
                "pulseAnimation"
            };
            let anim = self
                .resolve(&c.object.file, pptr(&b, field)?)?
                .ok_or_else(|| format!("{script}.{field} not found"))?;
            let at = tree
                .nodes
                .iter()
                .position(|n| n.components.iter().any(|x| x.object.key() == anim.key()))
                .ok_or_else(|| format!("{script}.{field}: not in the tree"))?;
            let pulse = Pulse {
                reference_capacity: num(&b, "pulseReferenceCapacity")?,
                delay: curve_field(&b, "pulseDelayCurve")?,
                time: curve_field(&b, "pulseTimeCurve")?,
                clip: self.pulse_clip(tree, &anim, at)?,
            };
            let icon = if kind == BarKind::Oxygen {
                None
            } else {
                let icon = of("icon")?;
                Some(IconMotion {
                    icon,
                    scale: self.stored_scale(tree, icon)?,
                    spring_coef: num(&b, "rotationSpringCoef")?,
                    velocity_damp: num(&b, "rotationVelocityDamp")?,
                    velocity_max: num(&b, "rotationVelocityMax")?,
                    random_velocity: num(&b, "rotationRandomVelocity")?,
                })
            };
            let overlays = if kind == BarKind::Oxygen {
                Some(Overlays {
                    speed: [num(&b, "overlay1Speed")?, num(&b, "overlay2Speed")?],
                    alpha: [num(&b, "overlay1Alpha")?, num(&b, "overlay2Alpha")?],
                })
            } else {
                None
            };
            bars.push(BarConfig {
                kind,
                root,
                bar,
                circular,
                text: of("text")?,
                damp_speed: num(&b, "dampSpeed")?,
                overlays,
                pulse,
                icon,
            });
        }
        Ok(HudConfig {
            background_double: node("backgroundBarsDouble")?,
            background_quad: node("backgroundBarsQuad")?,
            bars,
        })
    }
}

impl Assets<'_> {
    /// `uGUI_DepthCompass` in `tree` (`uGUI`; M9u6c1, `docs/formats/ui.md`
    /// § The depth and compass): its nodes, its normal sprites as ids of
    /// `scene`, `textColorNormal`, and the strings `Language.Get` gives from
    /// `language` (a key the file lacks is shown as itself).
    pub fn depth_config(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        scene: &mut UiScene,
        language: &HashMap<String, String>,
    ) -> Result<DepthConfig> {
        let (_, c) = tree
            .scripts("uGUI_DepthCompass")
            .next()
            .ok_or("no uGUI_DepthCompass")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let from = &c.object;
        let node = |name: &str| -> Result<usize> {
            self.node_of(tree, from, pptr(&f, name)?)?
                .ok_or_else(|| format!("uGUI_DepthCompass.{name}: not in the tree"))
        };
        let mut sprite = |name: &str| -> Result<SpriteId> {
            let o = self
                .resolve(&from.file, pptr(&f, name)?)?
                .ok_or_else(|| format!("uGUI_DepthCompass.{name}: not found"))?;
            Ok(scene.sprite_id(&o))
        };
        let get = |key: &str| {
            language
                .get(key)
                .cloned()
                .unwrap_or_else(|| key.to_string())
        };
        let color = f
            .get("textColorNormal")
            .and_then(Value::as_struct)
            .ok_or("no textColorNormal")?;
        let channel = |k: &str| -> Result<u8> {
            color
                .get(k)
                .and_then(Value::as_i64)
                .and_then(|v| u8::try_from(v).ok())
                .ok_or_else(|| format!("textColorNormal.{k}"))
        };
        let text_color_normal = [channel("r")?, channel("g")?, channel("b")?, channel("a")?];
        // The compass's labels: `CompassDirection{name}` for each label with
        // a text (`Initialize`, `OnLanguageChanged`).
        let compass_ptr = pptr(&f, "compass")?;
        let compass = node("compass")?;
        let cf = self
            .fields_at(layouts, from, compass_ptr)?
            .ok_or("uGUI_DepthCompass.compass: not found")?;
        let mut labels = Vec::new();
        for l in cf.get("labels").and_then(Value::as_array).unwrap_or(&[]) {
            let Some(l) = l.as_struct() else {
                continue;
            };
            let Some(text) = l
                .get("text")
                .and_then(Value::as_pptr)
                .filter(|p| !p.is_null())
            else {
                continue;
            };
            let name = l.get("name").and_then(Value::as_str).unwrap_or("");
            let at = self
                .node_of(tree, from, text)?
                .ok_or_else(|| format!("compass label {name}: not in the tree"))?;
            labels.push((at, get(&format!("CompassDirection{name}"))));
        }
        Ok(DepthConfig {
            shadow: node("shadow")?,
            half_moon: node("halfMoon")?,
            depth_text: node("depthText")?,
            suffix_text: node("suffixText")?,
            submersible: [
                node("submersibleBackground")?,
                node("submersibleDepth")?,
                node("submersibleDepthSuffix")?,
                node("submersibleCrushDepth")?,
            ],
            compass,
            labels,
            shadow_normal: sprite("shadowNormal")?,
            half_moon_normal: sprite("halfMoonNormal")?,
            half_moon_compass_normal: sprite("halfMoonCompassNormal")?,
            text_color_normal,
            meter_suffix: get("MeterSuffix"),
        })
    }
}
