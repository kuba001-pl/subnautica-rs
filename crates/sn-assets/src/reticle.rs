//! The hand reticle's nodes, fields and strings (M9u6d2;
//! `docs/formats/ui.md` § The hand reticle), for `sn_ui::reticle`'s
//! rules, and `GameInput.FormatButton` for the keyboard's default
//! bindings, every step read from the game's code at run time:
//! `GameInputSystem.bindingsKeyboard` (the button's binding path),
//! the control's `displayName` in `Unity.InputSystem.dll` (what
//! `InputControlPath.ToHumanReadableString` gives), `GameInputSystem.
//! displayKeyboard` (that name's display id), `GameInput.
//! defaultControlsColor`, then `AppendTranslationOrSprite`: a `<sprite>`
//! tag when the default sprite asset has the id, else the language's
//! text in a `<color>` tag.

use std::collections::HashMap;

use sn_dotnet::{Arg, Assembly, AttrValue};
use sn_install::GameData;
use sn_ui::reticle::{IconType, ReticleConfig, ReticleIcon};
use sn_ui::text::FontSet;
use sn_ui::text::sprite::simple_hash;
use sn_unity::{PPtr, Value};

use crate::{Assets, Layouts, Result, UiTree};

/// `RectTransform`'s class.
const RECT_TRANSFORM: i32 = 224;
/// The Input System's assembly.
const INPUT_ASSEMBLY: &str = "Managed/Unity.InputSystem.dll";

fn pptr(f: &sn_unity::Fields, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .filter(|p| !p.is_null())
        .ok_or_else(|| format!("HandReticle.{name}: not set"))
}

/// How a button's text was made (each step, for the logs).
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonText {
    /// `GameInput.Button`'s value.
    pub button: i32,
    /// The keyboard's default binding path (`<Mouse>/leftButton`).
    pub binding: String,
    /// The control's display name (`ToHumanReadableString`, device
    /// omitted).
    pub readable: String,
    /// `displayKeyboard`'s id for it (the name itself if absent).
    pub id: String,
    /// What `FormatButton` gives.
    pub text: String,
}

/// `GameInput.FormatButton(button)` for the keyboard and mouse with the
/// default bindings (the player's own bindings are not read). `button` is
/// a `GameInput.Button` name (`LeftHand`).
pub fn format_button(
    game: &GameData,
    set: &FontSet,
    language: &HashMap<String, String>,
    button: &str,
) -> Result<ButtonText> {
    let code = crate::read_assembly(game)?;
    let e = |e: sn_dotnet::Error| format!("{}: {e}", crate::GAME_ASSEMBLY);
    let asm = Assembly::parse(&code).map_err(e)?;
    let game_input = asm
        .find_type("", "GameInput")
        .map_err(e)?
        .ok_or("no GameInput")?;
    let buttons = asm
        .find_nested_type(game_input, "Button")
        .map_err(e)?
        .ok_or("no GameInput.Button")?;
    let value = sn_dotnet::enum_values_of(&asm, buttons, "GameInput.Button")
        .map_err(e)?
        .into_iter()
        .find(|(n, _)| n == button)
        .and_then(|(_, v)| i32::try_from(v).ok())
        .ok_or_else(|| format!("no GameInput.Button.{button}"))?;
    // The primary binding: the first path of the button's entry.
    let bindings =
        sn_dotnet::static_collection(&asm, "", "GameInputSystem", "bindingsKeyboard").map_err(e)?;
    let binding = bindings
        .iter()
        .find_map(|args| match args.as_slice() {
            [Arg::Int(b), Arg::Str(path), ..] if *b == value => Some(path.clone()),
            _ => None,
        })
        .ok_or_else(|| format!("no keyboard binding for {button}"))?;
    let readable = control_display_name(game, &binding)?;
    let display =
        sn_dotnet::static_collection(&asm, "", "GameInputSystem", "displayKeyboard").map_err(e)?;
    let id = display
        .iter()
        .find_map(|args| match args.as_slice() {
            [Arg::Str(k), Arg::Str(v)] if *k == readable => Some(v.clone()),
            _ => None,
        })
        .unwrap_or_else(|| readable.clone());
    let color =
        sn_dotnet::const_string(&asm, "", "GameInput", "defaultControlsColor").map_err(e)?;
    // `AppendTranslationOrSprite(id, sb, color)`.
    let has_sprite = set
        .default_sprite
        .and_then(|d| set.sprites.get(d))
        .is_some_and(|a| a.index_of_hash(simple_hash(&id)).is_some());
    let text = if has_sprite {
        format!("<sprite name=\"{id}\" color={color}>")
    } else {
        let t = language.get(&id).cloned().unwrap_or_else(|| id.clone());
        format!("<color={color}>{t}</color>")
    };
    Ok(ButtonText {
        button: value,
        binding,
        readable,
        id,
        text,
    })
}

/// The `displayName` of a binding path's control (`<Mouse>/leftButton`),
/// from the `[InputControl(name = …, displayName = …)]` attributes of the
/// layout's state struct (`MouseState`) in `Unity.InputSystem.dll`.
/// *Hypothesis*: `ToHumanReadableString` with the device omitted gives
/// exactly that name for a one-level control path.
fn control_display_name(game: &GameData, binding: &str) -> Result<String> {
    let (layout, control) = binding
        .strip_prefix('<')
        .and_then(|b| b.split_once(">/"))
        .ok_or_else(|| format!("binding {binding:?} not read"))?;
    if control.contains('/') {
        return Err(format!("binding {binding:?}: nested control not read"));
    }
    let bytes = game.read_file(&game.data_dir.join(INPUT_ASSEMBLY))?;
    let e = |e: sn_dotnet::Error| format!("{INPUT_ASSEMBLY}: {e}");
    let asm = Assembly::parse(&bytes).map_err(e)?;
    let state = asm
        .find_type(
            "UnityEngine.InputSystem.LowLevel",
            &format!("{layout}State"),
        )
        .map_err(e)?
        .ok_or_else(|| format!("no {layout}State"))?;
    let fields = asm.fields(state).map_err(e)?;
    for (k, a) in asm.attributes().map_err(e)?.iter().enumerate() {
        let on_state = (a.parent.table == sn_dotnet::Table::FIELD
            && fields.contains(&a.parent.row))
            || (a.parent.table == sn_dotnet::Table::TYPE_DEF && a.parent.row == state);
        if !on_state || a.name != "InputControlAttribute" {
            continue;
        }
        let args = asm.attribute_named_args(k as u32).map_err(e)?;
        let get = |n: &str| {
            args.iter().find_map(|(k, v)| match v {
                AttrValue::Str(Some(s)) if k == n => Some(s.clone()),
                _ => None,
            })
        };
        if get("name").as_deref() == Some(control) {
            return get("displayName")
                .ok_or_else(|| format!("{layout}State.{control}: no displayName"));
        }
    }
    Err(format!("{layout}State: no control {control}"))
}

impl Assets<'_> {
    /// `HandReticle` in `tree` (`uGUI`; M9u6d2): its nodes, icons and
    /// fields, and its strings from `language` (a key the file lacks is
    /// shown as itself).
    pub fn reticle_config(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        language: &HashMap<String, String>,
    ) -> Result<ReticleConfig> {
        let (_, c) = tree.scripts("HandReticle").next().ok_or("no HandReticle")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let from = &c.object;
        let node = |name: &str| -> Result<usize> {
            self.node_of(tree, from, pptr(&f, name)?)?
                .ok_or_else(|| format!("HandReticle.{name}: not in the tree"))
        };
        let mut icons = Vec::new();
        for p in f.get("icons").and_then(Value::as_array).unwrap_or(&[]) {
            let Some(p) = p.as_pptr().filter(|p| !p.is_null()) else {
                continue;
            };
            let o = self
                .resolve(&from.file, p)?
                .ok_or("HandReticle.icons: an icon not found")?;
            let at = self
                .node_of(tree, from, p)?
                .ok_or("HandReticle.icons: an icon not in the tree")?;
            let (_, fi) = self.object_fields(layouts, &o)?;
            let kind = fi
                .get("type")
                .and_then(Value::as_i64)
                .and_then(|v| i32::try_from(v).ok())
                .and_then(IconType::from_i32)
                .ok_or_else(|| format!("{}: icon type not read", tree.nodes[at].path))?;
            let mut graphics = Vec::new();
            for g in fi.get("graphic").and_then(Value::as_array).unwrap_or(&[]) {
                let Some(gp) = g.as_pptr().filter(|p| !p.is_null()) else {
                    continue;
                };
                graphics.push(self.node_of(tree, &o, gp)?.ok_or_else(|| {
                    format!("{}: a graphic not in the tree", tree.nodes[at].path)
                })?);
            }
            icons.push(ReticleIcon {
                kind,
                node: at,
                graphics,
            });
        }
        let icon_canvas = node("iconCanvas")?;
        let rect = tree.nodes[icon_canvas]
            .components
            .iter()
            .find(|c| c.class_id == RECT_TRANSFORM)
            .ok_or("HandReticle.iconCanvas: no RectTransform")?;
        let (_, d) = rect.object.data()?;
        let scale = sn_unity::RectTransform::parse(d, rect.object.file.file().big_endian)
            .map_err(|e| format!("iconCanvas: {e}"))?
            .scale;
        let speed = f
            .get("iconScaleSpeed")
            .and_then(Value::as_f64)
            .ok_or("HandReticle.iconScaleSpeed: not read")? as f32;
        let get = |key: &str| {
            language
                .get(key)
                .cloned()
                .unwrap_or_else(|| key.to_string())
        };
        Ok(ReticleConfig {
            icon_canvas,
            icon_canvas_scale: scale,
            icons,
            icon_scale_speed: speed,
            texts: [
                node("compTextHand")?,
                node("compTextHandSubscript")?,
                node("compTextUse")?,
                node("compTextUseSubscript")?,
            ],
            progress_image: node("progressImage")?,
            progress_text: node("progressText")?,
            add_button_format: get("HandReticleAddButtonFormat"),
            progress_format: get("HandReticleProgressPercentFormat"),
        })
    }
}
