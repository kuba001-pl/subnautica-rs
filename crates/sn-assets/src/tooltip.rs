//! The tooltip's nodes and fields (M9u8b3b; `docs/formats/ui.md` § The
//! tooltip), for `sn_ui::tooltip`'s rules.

use sn_ui::tooltip::{Eatable, ItemFacts, TooltipConfig};
use sn_unity::{PPtr, Value};

use crate::{Assets, Layouts, Result, UiTree};

fn pptr(f: &sn_unity::Fields, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .filter(|p| !p.is_null())
        .ok_or_else(|| format!("uGUI_Tooltip.{name}: not set"))
}

impl Assets<'_> {
    /// `uGUI_Tooltip` in `tree` (`uGUI`): its nodes and `scaleFactor`.
    pub fn tooltip_config(&self, layouts: &Layouts, tree: &UiTree) -> Result<TooltipConfig> {
        let (root, c) = tree
            .scripts("uGUI_Tooltip")
            .next()
            .ok_or("no uGUI_Tooltip")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let from = &c.object;
        let node = |name: &str| -> Result<usize> {
            self.node_of(tree, from, pptr(&f, name)?)?
                .ok_or_else(|| format!("uGUI_Tooltip.{name}: not in the tree"))
        };
        Ok(TooltipConfig {
            root,
            canvas: node("canvas")?,
            rect: node("rectTransform")?,
            canvas_group: node("canvasGroup")?,
            background: node("background")?,
            text_prefix: node("textPrefix")?,
            text_postfix: node("textPostfix")?,
            icon_canvas: node("iconCanvas")?,
            scale_factor: f
                .get("scaleFactor")
                .and_then(Value::as_f64)
                .ok_or("uGUI_Tooltip.scaleFactor: not read")? as f32,
        })
    }
}

/// The scripts `TooltipFactory.ItemCommons` reads that we do not port (the
/// tooltip leaves their lines out; logged when an item has one).
pub const UNPORTED_TOOLTIP_SCRIPTS: [&str; 6] = [
    "EnergyMixin",
    "Oxygen",
    "AirBladder",
    "Signal",
    "FireExtinguisher",
    "LiveMixin",
];

/// What `ItemCommons` reads of an item's prefab: the components on its
/// `Pickupable`'s object (`GetComponent` on `pickupable.gameObject`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemTooltipValues {
    /// `Eatable`: `foodValue`, `waterValue`, `decomposes`, `kDecayRate`.
    pub eatable: Option<(f32, f32, bool, f32)>,
    /// `Battery`: charge and `_capacity` of a new one (its charge is its
    /// capacity: `OnAfterDeserialize`).
    pub battery: Option<(f32, f32)>,
    /// Scripts of [`UNPORTED_TOOLTIP_SCRIPTS`] on that object.
    pub unported: Vec<String>,
}

impl Assets<'_> {
    /// [`ItemTooltipValues`] of `prefab` (its first `Pickupable`'s node,
    /// else its root).
    pub fn item_tooltip_values(
        &self,
        layouts: &Layouts,
        prefab: &crate::Prefab,
    ) -> Result<ItemTooltipValues> {
        let mut out = ItemTooltipValues::default();
        let scripts = |i: usize| -> Result<Vec<(String, crate::ObjectRef)>> {
            let Some(n) = prefab.nodes.get(i) else {
                return Ok(Vec::new());
            };
            Ok(self
                .node_components(n)?
                .into_iter()
                .filter_map(|c| self.script_class(&c).map(|s| (s, c)))
                .collect())
        };
        let mut node = 0;
        for i in 0..prefab.nodes.len() {
            if scripts(i)?.iter().any(|(s, _)| s == "Pickupable") {
                node = i;
                break;
            }
        }
        let float = |f: &sn_unity::Fields, k: &str, what: &str| -> Result<f32> {
            f.get(k)
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .ok_or_else(|| format!("{}: {what}.{k} not read", prefab.key))
        };
        for (class, c) in scripts(node)? {
            match class.as_str() {
                "Eatable" => {
                    let (_, f) = self.object_fields(layouts, &c)?;
                    let decomposes = f
                        .get("decomposes")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| format!("{}: Eatable.decomposes not read", prefab.key))?;
                    out.eatable = Some((
                        float(&f, "foodValue", "Eatable")?,
                        float(&f, "waterValue", "Eatable")?,
                        decomposes,
                        float(&f, "kDecayRate", "Eatable")?,
                    ));
                }
                "Battery" => {
                    // `_charge` is not stored: `OnAfterDeserialize` sets
                    // it to `_capacity`, so a new battery is full.
                    let (_, f) = self.object_fields(layouts, &c)?;
                    let capacity = float(&f, "_capacity", "Battery")?;
                    out.battery = Some((capacity, capacity));
                }
                s if UNPORTED_TOOLTIP_SCRIPTS.contains(&s) => out.unported.push(class.clone()),
                _ => {}
            }
        }
        Ok(out)
    }
}

impl ItemTooltipValues {
    /// The facts `ItemCommons` reads, with the item's `title`
    /// (`Language.Get(techType)`) and `description` (`Tooltip_<TechType>`).
    /// Food and water are the stored values: our items do not decay.
    pub fn facts(&self, title: String, description: String, first_aid_kit: bool) -> ItemFacts {
        ItemFacts {
            title,
            description,
            eatable: self.eatable.map(|(food, water, decomposes, _)| Eatable {
                food_value: food,
                water_value: water,
                decomposes,
                food_now: food,
                water_now: water,
            }),
            first_aid_kit,
            battery: self.battery,
        }
    }
}
