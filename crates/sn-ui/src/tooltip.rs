//! The tooltip (M9u8b3b, `docs/formats/ui.md` § The tooltip): our own code
//! after reading the game's `uGUI_Tooltip` and `TooltipFactory`.
//!
//! - [`TooltipConfig`]: `uGUI_Tooltip`'s nodes (on `uGUI/Tooltip`) and its
//!   `scaleFactor`.
//! - The texts (M9u8b3b1): [`inventory_item`] is
//!   `TooltipFactory.InventoryItem`: [`item_commons`] (the prefix: title,
//!   values, description) and [`item_actions`] (the postfix: the action
//!   hints with their buttons), with `WriteTitle`, `WriteDescription` and
//!   `WriteAction`'s rich-text tags. The language's formats go through
//!   [`cs_format`], the subset of C#'s `string.Format` they use.
//! - Layout and placement (M9u8b3b2): [`TooltipLayout`] is `OnLayout`
//!   with no icons (an inventory item has none), [`update_position`] is
//!   `UpdatePosition`: beside the pointer on the hovered graphic's canvas.

use std::fmt::Write;

use crate::layout::Rect;
use crate::state::{LocalTransform, UiState};

/// `uGUI_Tooltip`'s nodes and fields.
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipConfig {
    /// The script's own node (`uGUI/Tooltip`, a world-space canvas).
    pub root: usize,
    /// `canvas`'s node.
    pub canvas: usize,
    /// `rectTransform`: the node `UpdatePosition` places and `OnLayout`
    /// sizes.
    pub rect: usize,
    /// `canvasGroup`'s node: alpha 0 at `Awake`.
    pub canvas_group: usize,
    /// `background` (an `Image`).
    pub background: usize,
    /// `textPrefix`, `textPostfix` (`TextMeshProUGUI`).
    pub text_prefix: usize,
    pub text_postfix: usize,
    /// `iconCanvas` (a `FlexibleGridLayout`).
    pub icon_canvas: usize,
    /// `scaleFactor`.
    pub scale_factor: f32,
}

/// `ItemAction` (a flag set; the game's values).
pub const USE: u32 = 1;
pub const EAT: u32 = 2;
pub const EQUIP: u32 = 4;
pub const UNEQUIP: u32 = 8;
pub const ASSIGN: u32 = 16;
pub const SWITCH: u32 = 32;
pub const SWAP: u32 = 64;
pub const DROP: u32 = 128;

/// The language keys [`TooltipStrings`] reads: `RefreshActionStrings`'
/// (those `InventoryItem` uses) and `ItemCommons`' formats.
pub const LANGUAGE_KEYS: [&str; 19] = [
    "BatteryNotInserted",
    "Use",
    "Eat",
    "Equip",
    "Unequip",
    "AssignQuickSlot",
    "BindQuickSlot",
    "SwitchContainer",
    "SwapItems",
    "Drop",
    "KeyRange15",
    "DecomposingFormat",
    "Rotting",
    "Ripe",
    "Decomposing",
    "BatteryCharge",
    "HealthFormat",
    "FoodFormat",
    "WaterFormat",
];

/// `TooltipFactory`'s strings: the action words, the buttons' texts
/// (`GameInput.FormatButton(button0…3)`: `UISubmit`, `UICancel`,
/// `UIClear`, `UIAssign`) and the formats `ItemCommons` uses.
#[derive(Clone, Debug, PartialEq)]
pub struct TooltipStrings {
    /// `Language.Get` of each of [`LANGUAGE_KEYS`] (the key itself when
    /// the file has none).
    pub language: Vec<(String, String)>,
    pub buttons: [String; 4],
}

impl TooltipStrings {
    pub fn new(get: &dyn Fn(&str) -> String, buttons: [String; 4]) -> TooltipStrings {
        TooltipStrings {
            language: LANGUAGE_KEYS
                .iter()
                .map(|k| (k.to_string(), get(k)))
                .collect(),
            buttons,
        }
    }

    /// `Language.Get(key)` for one of [`LANGUAGE_KEYS`].
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.language
            .iter()
            .find(|(k, _)| k == key)
            .map_or(key, |(_, v)| v.as_str())
    }

    /// `GetUseActionString`: the word for one action (none: `None`).
    pub fn action(&self, action: u32) -> Option<&str> {
        let key = match action {
            USE => "Use",
            EAT => "Eat",
            EQUIP => "Equip",
            UNEQUIP => "Unequip",
            SWITCH => "SwitchContainer",
            SWAP => "SwapItems",
            DROP => "Drop",
            ASSIGN => "AssignQuickSlot",
            _ => return None,
        };
        Some(self.get(key))
    }
}

/// An argument of [`cs_format`].
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Int(i64),
    Float(f32),
    Str(String),
}

/// C#'s `string.Format(format, args)` for what the tooltip's language
/// entries use: `{n}`, `{n:custom}` with sections (`+#;-#;+0`: positive,
/// negative, zero; `#` and `0` digit places, other characters literal),
/// `{n:P0}` (a percentage without decimals, written `50%`: *hypothesis*,
/// the game's en-US culture under Mono), `{{` and `}}`. An index out of
/// range or a format not in this list is left as written.
pub fn cs_format(format: &str, args: &[Arg]) -> String {
    let mut out = String::with_capacity(format.len() + 8);
    let mut rest = format;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        if tail.starts_with('{')
            && let Some(end) = tail.find('}')
        {
            let item = &tail[1..end];
            let (index, spec) = match item.split_once(':') {
                Some((n, f)) => (n, Some(f)),
                None => (item, None),
            };
            let arg = index.trim().parse::<usize>().ok().and_then(|n| args.get(n));
            if let Some(text) = arg.and_then(|a| format_arg(a, spec)) {
                out.push_str(&text);
                rest = &tail[end + 1..];
                continue;
            }
        }
        out.push_str(&tail[..1]);
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

/// C#'s `MidpointRounding.AwayFromZero` to an integer (what custom
/// numeric formats do).
fn round_away(v: f32) -> i64 {
    let r = v.abs().round() as i64;
    if v < 0.0 { -r } else { r }
}

fn format_arg(a: &Arg, spec: Option<&str>) -> Option<String> {
    let Some(spec) = spec else {
        return Some(match a {
            Arg::Int(v) => v.to_string(),
            Arg::Float(v) => v.to_string(),
            Arg::Str(s) => s.clone(),
        });
    };
    let value = match a {
        Arg::Int(v) => *v as f32,
        Arg::Float(v) => *v,
        Arg::Str(s) => return Some(s.clone()),
    };
    if spec == "P0" {
        return Some(format!("{}%", round_away(value * 100.0)));
    }
    let int = match a {
        Arg::Int(v) => *v,
        _ => round_away(value),
    };
    custom_int(spec, int)
}

/// A custom numeric format of `#`, `0` and literal characters, with up to
/// three sections (`None` for anything else: `.`, `,`, `%`, quotes,
/// escapes).
fn custom_int(spec: &str, v: i64) -> Option<String> {
    if spec.contains(['.', ',', '%', '\'', '"', '\\']) {
        return None;
    }
    let sections: Vec<&str> = spec.split(';').collect();
    if sections.len() > 3 {
        return None;
    }
    let pick = |i: usize| sections.get(i).copied().filter(|s| !s.is_empty());
    // A negative value in its own section shows no minus of its own.
    let (section, minus) = if v < 0 {
        match pick(1) {
            Some(s) => (s, false),
            None => (sections[0], true),
        }
    } else if v == 0 {
        (pick(2).unwrap_or(sections[0]), false)
    } else {
        (sections[0], false)
    };
    let zeros = section.chars().filter(|&c| c == '0').count();
    let digits = v.unsigned_abs();
    let digits = if digits == 0 && zeros == 0 {
        String::new()
    } else {
        format!("{digits:0zeros$}")
    };
    let mut out = String::new();
    if minus {
        out.push('-');
    }
    let mut placed = false;
    for c in section.chars() {
        if c == '#' || c == '0' {
            if !placed {
                out.push_str(&digits);
                placed = true;
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// `WriteTitle`.
pub fn write_title(sb: &mut String, title: &str) {
    let _ = write!(sb, "<size=25><color=#ffffffff>{title}</color></size>");
}

/// `WriteDescription`: a new line unless first.
pub fn write_description(sb: &mut String, description: &str) {
    if !sb.is_empty() {
        sb.push('\n');
    }
    let _ = write!(sb, "<size=20><color=#DDDEDEFF>{description}</color></size>");
}

/// `WriteAction`: a new line unless first.
pub fn write_action(sb: &mut String, key: &str, action: &str) {
    if !sb.is_empty() {
        sb.push('\n');
    }
    let _ = write!(
        sb,
        "<size=20><color=#ffffffff>{key}</color> - <color=#00ffffff>{action}</color></size>"
    );
}

/// An `Eatable`'s values now (`GetFoodValue`, `GetWaterValue`) and its
/// stored ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eatable {
    pub food_value: f32,
    pub water_value: f32,
    pub decomposes: bool,
    /// `GetFoodValue()`, `GetWaterValue()`: the stored values less the
    /// decay (not ported: our items do not age, so these are the stored
    /// values).
    pub food_now: f32,
    pub water_now: f32,
}

impl Eatable {
    /// `GetSecondaryTooltip` (in survival): rotting, ripe or decomposing
    /// for an item that decomposes, else nothing.
    pub fn secondary<'a>(&self, s: &'a TooltipStrings) -> &'a str {
        if !self.decomposes {
            return "";
        }
        if self.food_now < 0.0 && self.water_now < 0.0 {
            s.get("Rotting")
        } else if self.food_now < 0.5 * self.food_value {
            s.get("Ripe")
        } else if self.food_now < 0.9 * self.food_value {
            s.get("Decomposing")
        } else {
            ""
        }
    }
}

/// What `ItemCommons` reads of an item: its texts from the language file
/// and the components on its `Pickupable`'s object that we port.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemFacts {
    /// `Language.Get(techType)`.
    pub title: String,
    /// `Language.Get("Tooltip_" + techType)`.
    pub description: String,
    pub eatable: Option<Eatable>,
    /// `techType == TechType.FirstAidKit`.
    pub first_aid_kit: bool,
    /// An `IBattery` (`Battery`): `charge`, `capacity`.
    pub battery: Option<(f32, f32)>,
}

/// `TooltipFactory.ItemCommons` (the prefix) for the parts we port, in
/// survival mode when `survival` (`GameModeUtils.RequiresSurvival`).
/// `WriteDebug` is off (`TooltipFactory.debug`).
pub fn item_commons(s: &TooltipStrings, item: &ItemFacts, survival: bool) -> String {
    let mut sb = String::new();
    let mut title = item.title.clone();
    if let Some(e) = item.eatable.filter(|_| survival) {
        let secondary = e.secondary(s);
        if !secondary.is_empty() {
            title = cs_format(
                s.get("DecomposingFormat"),
                &[Arg::Str(secondary.to_string()), Arg::Str(title)],
            );
        }
    }
    write_title(&mut sb, &title);
    let mut description = true;
    if let Some((charge, capacity)) = item.battery {
        // `Battery.GetChargeValueText`.
        let text = cs_format(
            s.get("BatteryCharge"),
            &[
                Arg::Float(charge / capacity),
                Arg::Int(round_to_int(charge)),
                Arg::Float(capacity),
            ],
        );
        write_description(&mut sb, &text);
        description = false;
    }
    if item.first_aid_kit {
        write_description(&mut sb, &cs_format(s.get("HealthFormat"), &[Arg::Float(50.0)]));
    }
    if let Some(e) = item.eatable.filter(|_| survival) {
        let food = e.food_now.ceil() as i64;
        if food != 0 {
            write_description(&mut sb, &cs_format(s.get("FoodFormat"), &[Arg::Int(food)]));
        }
        let water = e.water_now.ceil() as i64;
        if water != 0 {
            write_description(&mut sb, &cs_format(s.get("WaterFormat"), &[Arg::Int(water)]));
        }
    }
    if description {
        write_description(&mut sb, &item.description);
    }
    sb
}

/// `Mathf.RoundToInt`: halves to even.
fn round_to_int(v: f32) -> i64 {
    v.round_ties_even() as i64
}

/// `TooltipFactory.ItemActions` (the postfix) with the keyboard and mouse:
/// `actions` is `GetItemAction(item, button)` for buttons 0–3 (the game's
/// flags; 0 none); `bind` is `item == hoveredItem &&
/// GetCanBindItem(item)` (a `Hand` item in the player's own container).
/// Buttons 2 and 3 give no action with the keyboard, but are written as
/// the game writes them.
pub fn item_actions(s: &TooltipStrings, actions: [u32; 4], bind: bool) -> String {
    let mut sb = String::new();
    if !bind && actions.iter().all(|&a| a == 0) {
        return sb;
    }
    if bind {
        write_action(&mut sb, s.get("KeyRange15"), s.get("BindQuickSlot"));
    }
    // Button 3, 0, 2, then 1.
    for b in [3, 0, 2, 1] {
        if let Some(word) = s.action(actions[b]) {
            write_action(&mut sb, &s.buttons[b], word);
        }
    }
    sb
}

/// `TooltipFactory.InventoryItem`: (prefix, postfix).
pub fn inventory_item(
    s: &TooltipStrings,
    item: &ItemFacts,
    survival: bool,
    actions: [u32; 4],
    bind: bool,
) -> (String, String) {
    (
        item_commons(s, item, survival),
        item_actions(s, actions, bind),
    )
}

/// `uGUI_Tooltip`'s constants: `paddingX` (= `paddingY`), `spacing`,
/// `maxWidth` less both paddings (`OnLayout`'s 429), `margin`.
pub const PADDING: f32 = 20.0;
pub const SPACING: f32 = 10.0;
pub const MAX_TEXT_WIDTH: f32 = 429.0;
pub const MARGIN: [f32; 2] = [10.0, 20.0];

/// `OnLayout` with no icons: the icon grid's preferred size is 0 × 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipLayout {
    /// `a`: the texts' width.
    pub width: f32,
    /// `GetPreferredValues(text, a, 0).y` of the prefix and the postfix.
    pub prefix_height: f32,
    pub postfix_height: f32,
    /// The texts' and the icon grid's `anchoredPosition` (anchors and
    /// pivots at their top left).
    pub prefix_at: [f32; 2],
    pub icons_at: [f32; 2],
    pub postfix_at: [f32; 2],
    /// The root's (and so the background's) size.
    pub size: [f32; 2],
}

impl TooltipLayout {
    /// From the texts' unbounded preferred widths
    /// (`GetPreferredValues(text).x`) and their preferred heights at a
    /// width (`height_at(a)` → [prefix, postfix]).
    pub fn new(widths: [f32; 2], height_at: impl FnOnce(f32) -> [f32; 2]) -> TooltipLayout {
        let icons = [0.0f32, 0.0f32];
        let a = widths[0].max(widths[1]).max(icons[0]).min(MAX_TEXT_WIDTH);
        let [y, y2] = height_at(a);
        let mut num = PADDING;
        let prefix_at = [PADDING, -num];
        num += y;
        let icons_width = icons[0].min(a);
        let icons_at = [PADDING + 0.5 * (a - icons_width), -num];
        num += icons[1];
        if y2 > 0.0 {
            num += SPACING;
        }
        let postfix_at = [PADDING, -num];
        num += y2 + PADDING;
        TooltipLayout {
            width: a,
            prefix_height: y,
            postfix_height: y2,
            prefix_at,
            icons_at,
            postfix_at,
            size: [a + 2.0 * PADDING, num],
        }
    }

    /// The changes `OnUpdate` and `OnLayout` make to the tree: the texts,
    /// the children's sizes and places (the root's size goes through its
    /// placement, as a canvas's).
    pub fn state(&self, c: &TooltipConfig, prefix: &str, postfix: &str) -> UiState {
        let mut st = UiState::default();
        st.text.insert(c.text_prefix, prefix.to_string());
        st.text.insert(c.text_postfix, postfix.to_string());
        let mut place = |node: usize, at: [f32; 2], size: [f32; 2]| {
            st.transform.insert(
                node,
                LocalTransform {
                    anchored_position: Some(at),
                    size_delta: Some(size),
                    ..Default::default()
                },
            );
        };
        place(c.text_prefix, self.prefix_at, [self.width, self.prefix_height]);
        place(c.icon_canvas, self.icons_at, [0.0, 0.0]);
        place(c.text_postfix, self.postfix_at, [self.width, self.postfix_height]);
        st
    }
}

/// What `ExtractParams` reads of the hovered graphic's canvas
/// (`CursorManager.GetPointerInfo`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerOnCanvas {
    /// The canvas's `rect` in its own space.
    pub rect: Rect,
    /// The canvas's `lossyScale` (x, y).
    pub canvas_scale: [f32; 2],
    /// The pointer on the canvas's plane, in the canvas's space
    /// (`worldToLocalMatrix.MultiplyPoint3x4(position)`).
    pub pointer: [f32; 3],
    /// `Dot(aimingForward, position - aimingPosition)`: the pointer's
    /// depth along the camera's forward.
    pub depth: f32,
}

/// Where `UpdatePosition` puts the tooltip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipPose {
    /// `localScale` (all three axes); the rotation is the canvas's.
    pub scale: f32,
    /// The root's pivot (its top left) in the canvas's space.
    pub point: [f32; 3],
    /// The tooltip's size in the canvas's space (size × scale ÷ the
    /// canvas's lossy scale).
    pub size_on_canvas: [f32; 2],
}

/// `UpdatePosition` for a tooltip of `size` (the background's rect):
/// right of and below the pointer by [`MARGIN`]; on an axis where that
/// leaves the canvas's rect, on the pointer's other side; when both sides
/// would leave it, at the left (top) edge if the right (bottom) side
/// overflows more, else flush with the right (bottom) edge.
pub fn update_position(
    size: [f32; 2],
    p: &PointerOnCanvas,
    scale_factor: f32,
    ui_scale: f32,
) -> TooltipPose {
    let s = p.depth * scale_factor * ui_scale;
    let k = [s / p.canvas_scale[0], s / p.canvas_scale[1]];
    let w = [size[0] * k[0], size[1] * k[1]];
    let m = [MARGIN[0] * k[0], MARGIN[1] * k[1]];
    let v = p.pointer;
    let lo = [p.rect.x, p.rect.y];
    let hi = [p.rect.x + p.rect.width, p.rect.y + p.rect.height];
    let right = (v[0] + m[0] + w[0] - hi[0]).max(0.0);
    let left = -(v[0] - m[0] - w[0] - lo[0]).min(0.0);
    let top = (v[1] + m[1] + w[1] - hi[1]).max(0.0);
    let bottom = -(v[1] - m[1] - w[1] - lo[1]).min(0.0);
    let x = if right > 0.0 {
        if left > 0.0 {
            if right > left { lo[0] } else { hi[0] - w[0] }
        } else {
            v[0] - m[0] - w[0]
        }
    } else {
        v[0] + m[0]
    };
    let y = if bottom > 0.0 {
        if top > 0.0 {
            if bottom > top { hi[1] } else { lo[1] + w[1] }
        } else {
            v[1] + m[1] + w[1]
        }
    } else {
        v[1] - m[1]
    };
    TooltipPose {
        scale: s,
        point: [x, y, v[2]],
        size_on_canvas: w,
    }
}

#[cfg(test)]
mod tests;
