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

use std::fmt::Write;

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

#[cfg(test)]
mod tests;
