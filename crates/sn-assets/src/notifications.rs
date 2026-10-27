//! `uGUI.prefabNotificationLabel` (M9u8b5a; `docs/formats/ui.md`
//! § Notifications): the `NotificationLabel` root of `uGUI.prefab`'s file
//! read as a scene to graft, with its `uGUI_NotificationLabel`'s
//! `background` (an `Image`) and `text` (a `TextMeshProUGUI`) and the
//! text's stored colour (`SetAlpha` keeps a colour's rgb).

use sn_unity::Value;

use crate::{Assets, FontAssets, Layouts, Result, UiScene, UiTree};

/// The label's nodes in its own scene (the root is node 0).
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationLabelNodes {
    pub background: usize,
    pub text: usize,
    /// The background's and the text's stored colours.
    pub background_color: [f32; 4],
    pub text_color: [f32; 4],
}

impl Assets<'_> {
    /// `uGUI.prefabNotificationLabel` from `tree` (`uGUI`), as a scene
    /// (with `fonts` and the TagManager's `sorting_layers`) and its nodes.
    pub fn notification_label(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        fonts: &FontAssets,
        sorting_layers: &[(String, u32)],
    ) -> Result<(NotificationLabelNodes, UiScene)> {
        let (_, u) = tree.scripts("uGUI").next().ok_or("no uGUI")?;
        let (_, f) = self.script_fields(layouts, u)?;
        let prefab = f
            .get("prefabNotificationLabel")
            .and_then(Value::as_pptr)
            .filter(|p| !p.is_null())
            .ok_or("uGUI.prefabNotificationLabel: not set")?;
        let prefab = self
            .resolve(&u.object.file, prefab)?
            .ok_or("uGUI.prefabNotificationLabel: not found")?;
        let label = self.ui_tree("NotificationLabel", &prefab)?;
        let (_, script) = label
            .scripts("uGUI_NotificationLabel")
            .next()
            .ok_or("NotificationLabel: no uGUI_NotificationLabel")?;
        let (_, lf) = self.script_fields(layouts, script)?;
        // A field's component: the node holding it.
        let node_of = |name: &str| -> Result<usize> {
            let p = lf
                .get(name)
                .and_then(Value::as_pptr)
                .filter(|p| !p.is_null())
                .ok_or_else(|| format!("uGUI_NotificationLabel.{name}: not set"))?;
            let o = self
                .resolve(&script.object.file, p)?
                .ok_or_else(|| format!("uGUI_NotificationLabel.{name}: not found"))?;
            label
                .nodes
                .iter()
                .position(|n| n.components.iter().any(|c| c.object.key() == o.key()))
                .ok_or_else(|| format!("uGUI_NotificationLabel.{name}: not in the prefab"))
        };
        let background = node_of("background")?;
        let text = node_of("text")?;
        let color_of = |node: usize, script: &str, field: &str| -> Result<[f32; 4]> {
            let (_, c) = label
                .scripts(script)
                .find(|(n, _)| *n == node)
                .ok_or_else(|| format!("NotificationLabel: no {script} on node {node}"))?;
            let (_, cf) = self.script_fields(layouts, c)?;
            crate::messages::vector(&cf, field).map_err(|e| format!("NotificationLabel: {e}"))
        };
        let nodes = NotificationLabelNodes {
            background,
            text,
            background_color: color_of(background, "Image", "m_Color")?,
            text_color: color_of(text, "TextMeshProUGUI", "m_fontColor")?,
        };
        let scene = self.ui_scene(layouts, &label, fonts, sorting_layers)?;
        Ok((nodes, scene))
    }
}
