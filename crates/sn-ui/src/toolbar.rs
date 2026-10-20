//! `uGUI_Toolbar` (M9u8a4b, `docs/formats/ui.md` § The PDA's toolbar):
//! the PDA screen's tab bar, its icons made by code. Our own code after
//! reading the game's `uGUI_Toolbar`, `uGUI_ItemIcon` and
//! `RectTransformExtensions`.
//!
//! - `Initialize(content)` (from `uGUI_PDA.SetTabs`): one `uGUI_ItemIcon`
//!   per tab (`CreateIcon`: anchor (0, 0), pivot (0.5, 0.5); the tab's
//!   sprite as the foreground, `backgroundSpriteNormal` as the background;
//!   `SetActiveSize(buttonSize)`, `SetBackgroundSize(backgroundSize,
//!   keepAspect: true)`, `SetForegroundSize(foregroundSize)`, placed at
//!   x = ½·bg.x + i·(bg.x + step), y = ½·bg.y); the bar's width
//!   n·bg.x + (n − 1)·step; the selection marker last among its siblings
//!   and hidden; nothing selected.
//! - `Select(i)`: the old icon's background back to the normal sprite,
//!   the new one's to the selected sprite (both through
//!   `SetBackgroundSprite`, which sizes the background to `backgroundSize`
//!   *without* keeping the aspect), the marker at the icon's
//!   `localPosition` + `selectionMarkerOffset`, shown.
//! - `OnPointerClick` gives `uGUI_PDA.OnToolbarClick(index, button)`.
//!
//! The game destroys the old icons and makes new ones on each
//! `Initialize` (each `OnClosePDA`); we keep the nodes made the first time
//! and set them again, which leaves the same state (every field
//! `Initialize` sets is set again) in the same sibling order (the new
//! icons go last, then the marker is moved after them).
//!
//! Notification counts: `SetNotificationsAmount(i, 0)` makes nothing
//! (`SetNotificationAlpha(0)` with no label); the tabs' counts are
//! `NotificationManager`'s (M9u8b), 0 until then.

use crate::icon::{fill_rect, fit};
use crate::layout::{Rect, Tree, V2};
use crate::state::{SpriteId, UiState};

/// `uGUI_Toolbar`'s stored fields and the nodes it uses.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarConfig {
    /// The toolbar's own node (its `RectTransform`, the icons' parent).
    pub node: usize,
    /// `selectionMarker`'s node (a child of the toolbar).
    pub marker: Option<usize>,
    pub background_size: V2,
    pub foreground_size: V2,
    pub button_size: V2,
    pub button_step: f32,
    pub marker_offset: V2,
    /// The marker's anchor reference point in its parent's rect,
    /// `lerp(anchorMin, anchorMax, pivot)` (0–1), to turn the
    /// `localPosition` `Select` sets into an anchored position.
    pub marker_anchor: V2,
    /// The toolbar's rect height (laid out; `Initialize` sets the width
    /// only).
    pub height: f32,
    /// The toolbar's stored `sizeDelta`, and its parent's width × its
    /// anchors' span in x: `SetSizeWithCurrentAnchors` sets `sizeDelta.x`
    /// to the width less that.
    pub size_delta: V2,
    pub parent_span_x: f32,
}

impl ToolbarConfig {
    /// The fields read from the laid-out `tree` (before `Initialize`):
    /// the bar's height, stored `sizeDelta` and its parent's span in x,
    /// and the marker's anchor reference point.
    pub fn lay_out(&mut self, tree: &Tree) {
        if let Some(n) = tree.nodes.get(self.node) {
            let r = &n.rect;
            self.height = tree.rect(self.node).height;
            self.size_delta = r.size_delta;
            self.parent_span_x =
                tree.parent_size(self.node)[0] * (r.anchor_max[0] - r.anchor_min[0]);
        }
        if let Some(n) = self.marker.and_then(|m| tree.nodes.get(m)) {
            let r = &n.rect;
            self.marker_anchor =
                [0, 1].map(|a| r.anchor_min[a] + (r.anchor_max[a] - r.anchor_min[a]) * r.pivot[a]);
        }
    }
}

/// A sprite as the icons use it: the scene's id and `sprite.rect`'s size.
pub type IconSprite = (SpriteId, V2);

/// `backgroundSpriteNormal` and `backgroundSpriteSelected`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolbarSprites {
    pub normal: IconSprite,
    pub selected: IconSprite,
}

/// An icon's nodes: the `ToolbarIcon` (its `uGUI_ItemIcon`, the click
/// target), its `Background` and its `Foreground` (`uGUI_Icon`s, in that
/// sibling order after `RearrangeElements`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolbarIcon {
    pub root: usize,
    pub background: usize,
    pub foreground: usize,
}

/// What a toolbar call did, for the logs and the checks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolbarEvents {
    /// Nodes given `SetAsLastSibling`, in call order.
    pub last_sibling: Vec<usize>,
}

/// The toolbar's state.
#[derive(Clone, Debug, PartialEq)]
pub struct Toolbar {
    pub config: ToolbarConfig,
    pub sprites: ToolbarSprites,
    /// Every icon made, in order; the first `count` are the toolbar's.
    pub icons: Vec<ToolbarIcon>,
    pub count: usize,
    /// `selected` (`None`: −1).
    pub selected: Option<usize>,
    /// The width `Initialize` gave the bar.
    pub width: f32,
    pub events: ToolbarEvents,
}

impl Toolbar {
    pub fn new(config: ToolbarConfig, sprites: ToolbarSprites) -> Toolbar {
        Toolbar {
            config,
            sprites,
            icons: Vec::new(),
            count: 0,
            selected: None,
            width: 0.0,
            events: ToolbarEvents::default(),
        }
    }

    /// Icon `i`'s anchored position (`SetPosition`).
    pub fn icon_position(&self, i: usize) -> V2 {
        let (bg, step) = (self.config.background_size, self.config.button_step);
        [0.5 * bg[0] + i as f32 * (bg[0] + step), 0.5 * bg[1]]
    }

    /// Icon `i`'s rect in the toolbar's space, its pivot (the rect's
    /// centre) placed: the click target (`buttonSize`). The icon's anchor
    /// is the toolbar rect's lower left corner, `toolbar` the toolbar's
    /// laid-out rect.
    pub fn icon_rect(&self, i: usize, toolbar: Rect) -> Rect {
        let p = self.icon_position(i);
        let s = self.config.button_size;
        Rect {
            x: toolbar.x + p[0] - 0.5 * s[0],
            y: toolbar.y + p[1] - 0.5 * s[1],
            width: s[0],
            height: s[1],
        }
    }

    /// Makes icons (`make`: one icon's nodes, active, the graphics
    /// disabled) until there are `n`.
    pub fn make_icons(&mut self, n: usize, make: &mut dyn FnMut() -> ToolbarIcon) {
        while self.icons.len() < n {
            let icon = make();
            self.icons.push(icon);
        }
    }

    /// `Initialize(manager, content)`: `content` the tabs' sprites (`None`:
    /// `SpriteManager.Get` gave null, so no foreground), as many as there
    /// are icons made ([`Toolbar::make_icons`]).
    pub fn initialize(&mut self, content: &[Option<IconSprite>], state: &mut UiState) {
        self.events = ToolbarEvents::default();
        self.selected = None;
        let c = self.config.clone();
        let content = &content[..content.len().min(self.icons.len())];
        // The old icons destroyed (ours: the extra ones hidden).
        for icon in self.icons.iter().skip(content.len()) {
            state.active.insert(icon.root, false);
        }
        self.count = content.len();
        for (i, sprite) in content.iter().enumerate() {
            let icon = self.icons[i];
            state.active.insert(icon.root, true);
            // `SetForegroundSprite`: a null sprite makes no foreground.
            match sprite {
                Some((id, size)) => {
                    state.sprite.insert(icon.foreground, Some(*id));
                    state.enabled.insert(icon.foreground, true);
                    size_delta(state, icon.foreground, fit(c.foreground_size, *size, true));
                }
                None => {
                    state.sprite.insert(icon.foreground, None);
                    state.enabled.insert(icon.foreground, false);
                }
            }
            // `SetBackgroundSprite(normal)`, then `SetActiveSize`,
            // `SetBackgroundSize(…, keepAspect: true)`,
            // `SetForegroundSize`, `SetPosition`.
            self.set_background(state, icon, self.sprites.normal, true);
            size_delta(state, icon.root, c.button_size);
            let fill = fill_rect(c.background_size, c.foreground_size);
            for n in [icon.background, icon.foreground] {
                state
                    .material
                    .entry(n)
                    .or_default()
                    .vectors
                    .insert("_FillRect".into(), fill);
            }
            state
                .transform
                .entry(icon.root)
                .or_default()
                .anchored_position = Some(self.icon_position(i));
        }
        let n = content.len() as f32;
        self.width = (n * c.background_size[0] + (n - 1.0) * c.button_step).max(0.0);
        size_delta(
            state,
            c.node,
            [self.width - c.parent_span_x, c.size_delta[1]],
        );
        if let Some(m) = c.marker {
            self.events.last_sibling.push(m);
            state.active.insert(m, false);
        }
    }

    /// `SetBackgroundSprite(sprite)` with the background's desired size
    /// `backgroundSize`: the sprite, enabled, sized by `Fit` (`keep_aspect`:
    /// the later `SetBackgroundSize(…, true)` of `CreateIcon`; `false` in
    /// `SetBackgroundSprite`'s own call), `_Size`; `SLICE_9_GRID` off (the
    /// toolbar's sprites are not loaded through `SpriteManager.
    /// GetResource`, *hypothesis* logged by `ui --pda`).
    fn set_background(
        &self,
        state: &mut UiState,
        icon: ToolbarIcon,
        (id, size): IconSprite,
        keep_aspect: bool,
    ) {
        let bg = self.config.background_size;
        state.sprite.insert(icon.background, Some(id));
        state.enabled.insert(icon.background, true);
        size_delta(state, icon.background, fit(bg, size, keep_aspect));
        let m = state.material.entry(icon.background).or_default();
        m.vectors.insert("_Size".into(), [bg[0], bg[1], 0.0, 0.0]);
        m.keywords.insert("SLICE_9_GRID".into(), false);
    }

    /// `Select(index)`: false for an index out of range.
    pub fn select(&mut self, index: usize, state: &mut UiState) -> bool {
        if index >= self.count {
            return false;
        }
        if let Some(old) = self.selected {
            self.set_background(state, self.icons[old], self.sprites.normal, false);
        }
        self.selected = Some(index);
        self.set_background(state, self.icons[index], self.sprites.selected, false);
        if let Some(m) = self.config.marker {
            // `localPosition` = the icon's + the offset; as an anchored
            // position: less the marker's anchor reference point (the
            // toolbar's pivot cancels out).
            let p = self.icon_position(index);
            let (o, a) = (self.config.marker_offset, self.config.marker_anchor);
            let at = [
                p[0] + o[0] - self.width * a[0],
                p[1] + o[1] - self.config.height * a[1],
            ];
            state.transform.entry(m).or_default().anchored_position = Some(at);
            state.active.insert(m, true);
        }
        true
    }

    /// The icon under `point` (the toolbar's space, its rect `toolbar`),
    /// the last one first (drawn on top, so hit first).
    pub fn hit(&self, point: V2, toolbar: Rect) -> Option<usize> {
        (0..self.count).rev().find(|&i| {
            let r = self.icon_rect(i, toolbar);
            point[0] >= r.x
                && point[0] <= r.x + r.width
                && point[1] >= r.y
                && point[1] <= r.y + r.height
        })
    }
}

/// `RectTransformExtensions.SetSize` / `Fit` on a node whose anchors are
/// one point (the icons' are): `sizeDelta` = the size.
fn size_delta(state: &mut UiState, node: usize, size: V2) {
    state.transform.entry(node).or_default().size_delta = Some(size);
}

#[cfg(test)]
mod tests;
