//! What `uGUI_PDA` reads (M9u8a4a; `docs/formats/ui.md` § The PDA's
//! screen): the screen prefab (`PDA.prefabScreen` in the main scene) and
//! the nodes its states change ([`sn_ui::pda::PdaScreenConfig`]).

use sn_ui::pda::{PdaScreenConfig, Tab, TabNodes, TabShow};
use sn_unity::{Fields, PPtr, Value};

use crate::{Assets, Layouts, ObjectRef, Result, UiTree};

const TYPE: &str = "uGUI_PDA";

/// `uGUI_PDA`'s tab fields, in `Initialize`'s dictionary order.
const TAB_FIELDS: [(&str, Tab); 8] = [
    ("tabIntro", Tab::Intro),
    ("tabInventory", Tab::Inventory),
    ("tabJournal", Tab::Journal),
    ("tabPing", Tab::Ping),
    ("tabGallery", Tab::Gallery),
    ("tabLog", Tab::Log),
    ("tabEncyclopedia", Tab::Encyclopedia),
    ("tabTimeCapsule", Tab::TimeCapsule),
];

fn pptr(f: &Fields, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .ok_or_else(|| format!("{name}: not read"))
}

/// The node holding the component `p` points at (from `from`'s file).
fn node_of(assets: &Assets, tree: &UiTree, from: &ObjectRef, p: PPtr, what: &str) -> Result<usize> {
    let o = assets
        .resolve(&from.file, p)?
        .ok_or_else(|| format!("{what}: not set"))?;
    tree.nodes
        .iter()
        .position(|n| n.components.iter().any(|c| c.object.key() == o.key()))
        .ok_or_else(|| format!("{what}: not in the tree"))
}

impl Assets<'_> {
    /// The PDA's screen prefab (`uGUI_PDAScreen`) as a UI tree, from the
    /// main scene's `PDA.prefabScreen`.
    pub fn pda_screen_tree(&self) -> Result<UiTree> {
        let main = self.scene("main")?;
        let (screen, _) = self.pda_screen_refs(&main)?;
        self.ui_tree("uGUI_PDAScreen", &screen)
    }

    /// `uGUI_PDA`'s nodes in `tree` (the screen prefab): its canvas group,
    /// content, toolbar group, back button, background, and each tab with
    /// what its `Open` / `Close` change (the regular tabs' `content`
    /// CanvasGroup; the intro's and the time capsule's own GameObject).
    pub fn pda_screen_config(&self, layouts: &Layouts, tree: &UiTree) -> Result<PdaScreenConfig> {
        let (root, c) = tree.scripts(TYPE).next().ok_or("no uGUI_PDA")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let e = |m: String| format!("{TYPE}.{m}");
        let node = |name: &str| -> Result<usize> {
            node_of(
                self,
                tree,
                &c.object,
                pptr(&f, name).map_err(e)?,
                &e(name.into()),
            )
        };
        let mut tabs = Vec::with_capacity(TAB_FIELDS.len());
        for (name, tab) in TAB_FIELDS {
            let p = pptr(&f, name).map_err(e)?;
            let at = node_of(self, tree, &c.object, p, &e(name.into()))?;
            let show = match tab {
                Tab::Intro | Tab::TimeCapsule => TabShow::Object(at),
                _ => {
                    let script = self
                        .resolve(&c.object.file, p)?
                        .ok_or_else(|| e(format!("{name}: not set")))?;
                    let (_, tf) = self.object_fields(layouts, &script)?;
                    let content = pptr(&tf, "content").map_err(|m| e(format!("{name}.{m}")))?;
                    TabShow::Content(node_of(
                        self,
                        tree,
                        &script,
                        content,
                        &e(format!("{name}.content")),
                    )?)
                }
            };
            tabs.push(TabNodes { tab, show });
        }
        // `uGUI_Toolbar`'s gamepad labels.
        let toolbar = self
            .resolve(&c.object.file, pptr(&f, "toolbar").map_err(e)?)?
            .ok_or_else(|| e("toolbar: not set".into()))?;
        let (_, tf) = self.object_fields(layouts, &toolbar)?;
        let mut cycle_texts = Vec::new();
        for name in ["cycleLeftBtnText", "cycleRightBtnText"] {
            let p = pptr(&tf, name).map_err(|m| e(format!("toolbar.{m}")))?;
            if !p.is_null() {
                cycle_texts.push(node_of(
                    self,
                    tree,
                    &toolbar,
                    p,
                    &e(format!("toolbar.{name}")),
                )?);
            }
        }
        Ok(PdaScreenConfig {
            root,
            cycle_texts,
            canvas_group: node("canvasGroup")?,
            content: node("content")?,
            toolbar_group: node("toolbarCanvasGroup")?,
            back_button: node("backButton")?,
            background: node("pdaBackground")?,
            tabs,
        })
    }
}
