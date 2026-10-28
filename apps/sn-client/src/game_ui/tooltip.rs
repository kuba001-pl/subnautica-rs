//! The inventory tooltip in the client; the rules live in sn-ui.
use super::*;
use sn_ui::draw::{apply, to_ancestor};
use sn_ui::tooltip::{PointerOnCanvas, TooltipConfig, TooltipLayout, TooltipStrings};

pub(super) struct Tooltips {
    config: TooltipConfig,
    strings: TooltipStrings,
    paths: HashMap<String, String>,
    values: HashMap<String, sn_assets::ItemTooltipValues>,
    names: HashMap<i32, String>,
    shown: Option<(u64, String, String)>,
}

impl Tooltips {
    pub(super) fn new(
        assets: &sn_assets::Assets,
        layouts: &sn_assets::Layouts,
        tree: &sn_assets::UiTree,
        game: &sn_install::GameData,
        set: &sn_ui::text::FontSet,
        language: &HashMap<String, String>,
    ) -> Result<Self, String> {
        let lang = |k: &str| language.get(k).cloned().unwrap_or_else(|| k.into());
        let mut buttons = [String::new(), String::new(), String::new(), String::new()];
        for (i, key) in ["UISubmit", "UICancel", "UIClear", "UIAssign"]
            .iter()
            .enumerate()
        {
            buttons[i] = sn_assets::format_button(game, set, language, key)?.text;
        }
        Ok(Self {
            config: assets.tooltip_config(layouts, tree)?,
            strings: TooltipStrings::new(&lang, buttons),
            paths: game
                .read_prefab_database_ordered()
                .map_err(|e| e.to_string())?
                .into_iter()
                .collect(),
            values: HashMap::new(),
            names: sn_assets::game_code(&sn_assets::read_assembly(game)?)?
                .tech_types
                .into_iter()
                .collect(),
            shown: None,
        })
    }

    pub(super) fn clear(&mut self, rules: &mut UiState, time: f32) {
        rules.group_alpha.insert(self.config.canvas_group, 0.0);
        rules.active.insert(self.config.root, false);
        if self.shown.take().is_some() {
            info!(
                "game UI tooltip at {time:.3} s: cleared, alpha 0, active false (off icon or PDA unfocused)"
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn update(
        &mut self,
        assets: &sn_assets::Assets,
        layouts: &sn_assets::Layouts,
        catalog: &sn_unity::Catalog,
        scene: &mut sn_assets::UiScene,
        set: &sn_ui::text::FontSet,
        language: &HashMap<String, String>,
        entry: Option<&TabEntry>,
        pointer: Option<[f32; 2]>,
        layout: &sn_ui::layout::Tree,
        root: usize,
        graphic: usize,
        placed: &Placement,
        camera: &UiCamera,
        state: &UiState,
        placements: &mut HashMap<usize, Placement>,
        rules: &mut UiState,
        time: f32,
    ) -> Result<Option<TooltipOnPda>, String> {
        let (Some(e), Some(point)) = (entry, pointer) else {
            return Ok(None);
        };
        let name = self
            .names
            .get(&e.tech)
            .cloned()
            .unwrap_or_else(|| e.tech.to_string());
        let lang = |k: &str| language.get(k).cloned().unwrap_or_else(|| k.into());
        let mut values = match &e.class {
            Some(class) => {
                if !self.values.contains_key(class) {
                    let path = self
                        .paths
                        .get(class)
                        .ok_or_else(|| format!("tooltip: no prefab for {class}"))?;
                    let prefab = assets.prefab(catalog, path)?;
                    let v = assets.item_tooltip_values(layouts, &prefab)?;
                    if !v.unported.is_empty() {
                        info!("game UI tooltip {name}: lines not ported: {:?}", v.unported);
                    }
                    self.values.insert(class.clone(), v);
                }
                self.values.get(class).cloned().unwrap_or_default()
            }
            None => sn_assets::ItemTooltipValues::default(),
        };
        if let (Some(charge), Some(battery)) = (e.charge, values.battery.as_mut()) {
            battery.0 = charge;
        }
        let facts = values.facts(
            lang(&name),
            lang(&format!("Tooltip_{name}")),
            e.tech == sn_sim::inventory::FIRST_AID_KIT,
        );
        let (prefix, postfix) = sn_ui::tooltip::inventory_item(
            &self.strings,
            &facts,
            true,
            [e.actions[0], e.actions[1], 0, 0],
            e.bind,
        );
        self.show(
            scene, set, &name, e.id, prefix, postfix, point, layout, root, graphic, placed, camera,
            state, placements, rules, time,
        )
    }

    pub(super) fn strings(&self) -> &TooltipStrings {
        &self.strings
    }

    /// `uGUI_Tooltip`'s layout and `UpdatePosition` for these texts beside
    /// `point` on `root`'s canvas (placed at `placed`), hovering `graphic`
    /// (M9u8b3b; since M9e4b also the crafting menu's). `id` and `name` are
    /// for the log.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn show(
        &mut self,
        scene: &mut sn_assets::UiScene,
        set: &sn_ui::text::FontSet,
        name: &str,
        id: u64,
        prefix: String,
        postfix: String,
        point: [f32; 2],
        layout: &sn_ui::layout::Tree,
        root: usize,
        graphic: usize,
        placed: &Placement,
        camera: &UiCamera,
        state: &UiState,
        placements: &mut HashMap<usize, Placement>,
        rules: &mut UiState,
        time: f32,
    ) -> Result<Option<TooltipOnPda>, String> {
        let c = &self.config;
        let pref = |node, text: &str, width| {
            scene
                .preferred_size(set, node, text, width)
                .ok_or_else(|| format!("tooltip: no text at {node}"))
        };
        let widths = [
            pref(c.text_prefix, &prefix, 0.0)?[0],
            pref(c.text_postfix, &postfix, 0.0)?[0],
        ];
        let width = widths[0].max(widths[1]).min(sn_ui::tooltip::MAX_TEXT_WIDTH);
        let heights = [
            pref(c.text_prefix, &prefix, width)?[1],
            pref(c.text_postfix, &postfix, width)?[1],
        ];
        let text_layout = TooltipLayout::new(widths, |_| heights);
        let world = apply(&placed.world_from_canvas(), [point[0], point[1], 0.0]);
        let depth = world
            .iter()
            .zip(camera.position)
            .zip(camera.forward())
            .map(|((&v, o), f)| (v - o) * f)
            .sum();
        let pose = sn_ui::tooltip::update_position(
            text_layout.size,
            &PointerOnCanvas {
                rect: layout.rect(root),
                canvas_scale: [placed.scale[0], placed.scale[1]],
                pointer: [point[0], point[1], 0.0],
                depth,
            },
            c.scale_factor,
            DEFAULT_UI_SCALE,
        );
        let k = pose.scale / placed.scale[0];
        let alpha = scene.inherited_alpha(graphic, state);
        rules.apply(&text_layout.state(c, &prefix, &postfix));
        rules.active.insert(c.root, true);
        rules.group_alpha.insert(c.canvas_group, alpha);
        let sorting = scene
            .canvas_sorting(root)
            .ok_or("tooltip: the hovered canvas has no canvas")?;
        scene.set_canvas_sorting(c.canvas, sorting.0, sorting.1.saturating_add(2));
        placements.insert(
            c.root,
            Placement {
                position: apply(&placed.world_from_canvas(), pose.point),
                rotation: placed.rotation,
                size: text_layout.size,
                scale: [pose.scale; 3],
                scale_factor: 1.0,
            },
        );
        let key = (id, prefix, postfix);
        if self.shown.as_ref() != Some(&key) {
            info!(
                "game UI tooltip at {time:.3} s: {name} #{}: prefix {:?}; postfix {:?}; pointer {point:?}, rect top-left {:?}, size {:?}, canvas size {:?}, scale {}, alpha {alpha}, sorting {:?}+2",
                id,
                key.1,
                key.2,
                pose.point,
                text_layout.size,
                pose.size_on_canvas,
                pose.scale,
                sorting
            );
        }
        self.shown = Some(key);
        Ok(Some(TooltipOnPda {
            root: c.root,
            point: pose.point,
            k,
        }))
    }
}

pub(super) fn item_node(tab: &InventoryTab, id: u64) -> Option<usize> {
    tab.inventory_icons
        .icons
        .get(&id)
        .or_else(|| tab.storage_icons.icons.get(&id))
        .map(|(i, _)| i.root)
        .or_else(|| {
            tab.config
                .slots
                .iter()
                .find(|s| {
                    tab.slot_icons
                        .get(&s.slot)
                        .is_some_and(|i| i.item == Some(id))
                })
                .map(|s| s.node)
        })
}

/// Scripted pointers go through the same projection and ray as the mouse.
pub(super) fn pointer(
    tab: &InventoryTab,
    l: &sn_ui::layout::Tree,
    root: usize,
    placed: &Placement,
    camera: &UiCamera,
    screen: [i32; 2],
    clicks: &PdaClicks,
) -> Option<[f32; 2]> {
    let pixel = match clicks.hover {
        Some(ScriptedHover::Item(id)) => {
            let node = item_node(tab, id)?;
            let r = l.rect(node);
            let p = apply(
                &to_ancestor(l, node, root),
                [r.x + r.width * 0.5, r.y + r.height * 0.5, 0.0],
            );
            camera.project(apply(&placed.world_from_canvas(), p), screen)
        }
        Some(ScriptedHover::Slot(slot)) => {
            let node = tab
                .config
                .slots
                .iter()
                .find(|s| s.slot == slot && l.active(s.node))?
                .node;
            let r = l.rect(node);
            let p = apply(
                &to_ancestor(l, node, root),
                [r.x + r.width * 0.5, r.y + r.height * 0.5, 0.0],
            );
            camera.project(apply(&placed.world_from_canvas(), p), screen)
        }
        Some(ScriptedHover::LastCell | ScriptedHover::LockerCell) => {
            let node = if clicks.hover == Some(ScriptedHover::LockerCell) {
                tab.config.storage.node
            } else {
                tab.config.inventory.node
            };
            if !l.active(node) {
                return None;
            }
            let r = l.rect(node);
            let p = apply(
                &to_ancestor(l, node, root),
                [r.x + r.width - 35.5, r.y + 35.5, 0.0],
            );
            camera.project(apply(&placed.world_from_canvas(), p), screen)
        }
        None => clicks.pointer,
    }?;
    placed.ray_point(camera.position, camera.screen_ray(pixel, screen))
}
