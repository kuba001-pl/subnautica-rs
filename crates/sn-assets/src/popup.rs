//! What `uGUI_PopupNotification` reads (M9u7d1; `docs/formats/ui.md`
//! § The unlock pop-up): its nodes and fields and its skins'
//! (`uGUI_PopupNotificationDefault`), `PDAData.analysisTech`'s message,
//! sound and picture per tech type, and `OnAnalyze`'s entry made from them.

use std::collections::HashMap;

use sn_ui::layout::Rect;
use sn_ui::popup::{Entry, PopupConfig, PopupLayout, PopupSkin, UNLOCK};
use sn_unity::{Fields, PPtr, Value};

use crate::{Assets, Layouts, ObjectRef, PlayerData, Result, UiScene, UiState, UiTree};

const TYPE: &str = "uGUI_PopupNotification";

/// One `KnownTech.AnalysisTech`, as the pop-up reads it.
#[derive(Clone, Debug)]
pub struct AnalysisPopup {
    pub tech_type: i32,
    /// A language key.
    pub unlock_message: String,
    /// `unlockSound`'s event path (`None`: not set).
    pub unlock_sound: Option<String>,
    /// `unlockPopup`, a Sprite (`None`: not set; the skin's
    /// `defaultSprite` is shown).
    pub unlock_popup: Option<ObjectRef>,
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

fn pptr(f: &Fields, name: &str) -> Result<PPtr> {
    f.get(name)
        .and_then(Value::as_pptr)
        .ok_or_else(|| format!("{name}: not read"))
}

fn num(f: &Fields, name: &str) -> Result<f32> {
    f.get(name)
        .and_then(Value::as_f64)
        .map(|v| v as f32)
        .ok_or_else(|| format!("{name}: not read"))
}

impl Assets<'_> {
    /// An `FMODAsset`'s event path (its `path` field; else its name).
    pub fn fmod_asset_name(
        &self,
        layouts: &Layouts,
        from: &ObjectRef,
        p: PPtr,
    ) -> Result<Option<String>> {
        let Some(o) = self.resolve(&from.file, p)? else {
            return Ok(None);
        };
        let (_, f) = self.object_fields(layouts, &o)?;
        let s = |n: &str| f.get(n).and_then(Value::as_str).map(str::to_string);
        Ok(s("path").filter(|p| !p.is_empty()).or_else(|| s("m_Name")))
    }

    /// `uGUI_PopupNotification` in `tree` (`uGUI`) and its skins; the
    /// skins' `defaultSprite`s get their ids from `scene`.
    pub fn popup_config(
        &self,
        layouts: &Layouts,
        tree: &UiTree,
        scene: &mut UiScene,
    ) -> Result<PopupConfig> {
        let (root, c) = tree
            .scripts(TYPE)
            .next()
            .ok_or("no uGUI_PopupNotification")?;
        let (_, f) = self.script_fields(layouts, c)?;
        let e = |m: String| format!("{TYPE}.{m}");
        let group = node_of(
            self,
            tree,
            &c.object,
            pptr(&f, "canvasGroup").map_err(e)?,
            "canvasGroup",
        )?;
        let skins = f
            .get("skins")
            .and_then(Value::as_array)
            .ok_or_else(|| e("skins: not read".into()))?;
        let mut out = Vec::new();
        for (k, s) in skins.iter().enumerate() {
            let p = s
                .as_pptr()
                .ok_or_else(|| e(format!("skins[{k}]: not a reference")))?;
            let skin = self
                .resolve(&c.object.file, p)?
                .ok_or_else(|| e(format!("skins[{k}]: not set")))?;
            let at = node_of(self, tree, &c.object, p, "skin")?;
            let (_, sf) = self.object_fields(layouts, &skin)?;
            let se = |m: String| format!("{TYPE}.skins[{k}].{m}");
            let node = |n: &str| -> Result<usize> {
                node_of(self, tree, &skin, pptr(&sf, n).map_err(se)?, &se(n.into()))
            };
            let image = match pptr(&sf, "image").map_err(se)? {
                p if p.is_null() => None,
                _ => Some(node("image")?),
            };
            let default_sprite = self
                .resolve(&skin.file, pptr(&sf, "defaultSprite").map_err(se)?)?
                .map(|o| scene.sprite_id(&o));
            let sound = |n: &str| -> Result<Option<String>> {
                self.fmod_asset_name(layouts, &skin, pptr(&sf, n).map_err(se)?)
            };
            out.push(PopupSkin {
                root: at,
                group: node("canvasGroup")?,
                ox: num(&sf, "ox").map_err(se)?,
                oy: num(&sf, "oy").map_err(se)?,
                title: node("title")?,
                text: node("text")?,
                controls: node("controls")?,
                image,
                default_sprite,
                sound_slide_in: sound("soundSlideIn")?,
                sound_slide_out: sound("soundSlideOut")?,
                default_sound: sound("defaultSound")?,
            });
        }
        Ok(PopupConfig {
            root,
            group,
            skins: out,
            default_duration: num(&f, "defaultDuration").map_err(e)?,
        })
    }

    /// `PDAData.analysisTech` (through `Player.pdaData`) as the pop-up
    /// reads it, by tech type. `KnownTech.Analyze` acts on the first entry
    /// of a tech type (`analyzedTech.Add` fails for the others), and
    /// `ValidateAnalysisTech` drops `TechType.None`'s.
    pub fn analysis_popups(
        &self,
        layouts: &Layouts,
        player: &PlayerData,
    ) -> Result<HashMap<i32, AnalysisPopup>> {
        let from = &player.pda_object;
        let mut out = HashMap::new();
        for a in &player.pda.analysis_tech {
            if a.tech_type == 0 || out.contains_key(&a.tech_type) {
                continue;
            }
            out.insert(
                a.tech_type,
                AnalysisPopup {
                    tech_type: a.tech_type,
                    unlock_message: a.unlock_message.clone(),
                    unlock_sound: self.fmod_asset_name(layouts, from, a.unlock_sound)?,
                    unlock_popup: self.resolve(&from.file, a.unlock_popup)?,
                },
            );
        }
        Ok(out)
    }
}

/// `OnAnalyze(analysis, verbose: true)`'s entry: the `Unlock` skin for
/// `defaultDuration`, the title `Language.Get(unlockMessage)`, the text
/// `Language.Get(techType.AsString())` (`tech_name`), the picture and the
/// sound. `Language.Get` gives the key itself when it has no text.
pub fn analysis_entry(
    config: &PopupConfig,
    analysis: &AnalysisPopup,
    tech_name: &str,
    language: &HashMap<String, String>,
    scene: &mut UiScene,
) -> Entry {
    let get = |k: &str| language.get(k).cloned().unwrap_or_else(|| k.to_string());
    Entry {
        duration: config.default_duration,
        skin: UNLOCK,
        title: get(&analysis.unlock_message),
        text: get(tech_name),
        controls: String::new(),
        sprite: analysis.unlock_popup.as_ref().map(|o| scene.sprite_id(o)),
        sound: analysis.unlock_sound.clone(),
    }
}

/// What `SetTransition` reads of `scene`'s layout for `state`: the
/// pop-up's rect and each skin's.
pub fn popup_layout(
    assets: &Assets,
    scene: &mut UiScene,
    set: &sn_ui::text::FontSet,
    placements: &HashMap<usize, sn_ui::scaler::Placement>,
    state: &UiState,
    config: &PopupConfig,
) -> Result<PopupLayout> {
    let tree = scene.layout_for(assets, set, placements, state)?;
    let skins: Vec<Rect> = config.skins.iter().map(|s| tree.rect(s.root)).collect();
    Ok(PopupLayout {
        parent: tree.rect(config.root),
        skins,
    })
}
