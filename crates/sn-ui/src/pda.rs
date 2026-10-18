//! `uGUI_PDA`'s screen states (M9u8a4a, `docs/DESIGN.md` "M9u8a4a",
//! `docs/formats/ui.md` § The PDA's screen): which of the screen's groups
//! and objects show as the PDA opens and closes, and `OpenTab`. Our own
//! code after reading the game's `uGUI_PDA`, `uGUI_PDATab` and its tabs,
//! and `CanvasGroup.SetVisible`. The toolbar's icons (made by code) come
//! with M9u8a4b.

use crate::state::UiState;

/// `PDATab` (the game's values, without `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tab {
    Intro,
    Inventory,
    Journal,
    Ping,
    Gallery,
    Log,
    Encyclopedia,
    TimeCapsule,
}

/// `uGUI_PDA.regularTabs`, in the toolbar's order.
pub const REGULAR_TABS: [Tab; 6] = [
    Tab::Inventory,
    Tab::Journal,
    Tab::Ping,
    Tab::Gallery,
    Tab::Log,
    Tab::Encyclopedia,
];

/// What a tab's `Open` and `Close` change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabShow {
    /// `content.SetVisible(…)`: this CanvasGroup's node.
    Content(usize),
    /// `gameObject.SetActive(…)`: the tab's own node (the intro and the
    /// time capsule tabs).
    Object(usize),
}

/// One tab of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabNodes {
    pub tab: Tab,
    pub show: TabShow,
}

/// The screen's nodes `uGUI_PDA` changes.
#[derive(Clone, Debug, PartialEq)]
pub struct PdaScreenConfig {
    /// The canvas's root (where `uGUI_PDA` and its canvas sit).
    pub root: usize,
    /// `canvasGroup`, `content`, `toolbarCanvasGroup`.
    pub canvas_group: usize,
    pub content: usize,
    pub toolbar_group: usize,
    /// `backButton`'s node.
    pub back_button: usize,
    /// `pdaBackground`'s node (`SetBackgroundAlpha`).
    pub background: usize,
    /// `uGUI_Toolbar.cycleLeftBtnText` / `cycleRightBtnText`: the gamepad's
    /// tab buttons, hidden while the keyboard is the primary device
    /// (`UpdateCycleBtnText` on `OnEnable`).
    pub cycle_texts: Vec<usize>,
    pub tabs: Vec<TabNodes>,
}

impl PdaScreenConfig {
    /// The same nodes `off` further on (the screen grafted into a scene).
    pub fn shifted(&self, off: usize) -> PdaScreenConfig {
        let show = |s: TabShow| match s {
            TabShow::Content(n) => TabShow::Content(n + off),
            TabShow::Object(n) => TabShow::Object(n + off),
        };
        PdaScreenConfig {
            root: self.root + off,
            canvas_group: self.canvas_group + off,
            content: self.content + off,
            toolbar_group: self.toolbar_group + off,
            back_button: self.back_button + off,
            background: self.background + off,
            cycle_texts: self.cycle_texts.iter().map(|n| n + off).collect(),
            tabs: self
                .tabs
                .iter()
                .map(|t| TabNodes {
                    tab: t.tab,
                    show: show(t.show),
                })
                .collect(),
        }
    }
}

/// `uGUI_PDA` and its tabs' visible state.
#[derive(Clone, Debug, PartialEq)]
pub struct PdaScreen {
    pub config: PdaScreenConfig,
    /// `tabOpen` (`None`: `PDATab.None`) and `tabPrev`.
    pub tab_open: Option<Tab>,
    pub tab_prev: Tab,
    /// `currentTabs` (the toolbar's tabs; `SetTabs(regularTabs)`).
    pub current_tabs: Vec<Tab>,
    /// The changes to the screen's stored state.
    pub state: UiState,
}

impl PdaScreen {
    /// After `Awake` (`content` visible, the canvas invisible) and
    /// `Initialize` (each tab's `Close`, the back button inactive,
    /// `SetTabs(regularTabs)`), as `PDA.ui` leaves it.
    pub fn new(config: PdaScreenConfig) -> PdaScreen {
        let mut s = PdaScreen {
            config,
            tab_open: None,
            tab_prev: REGULAR_TABS[0],
            current_tabs: REGULAR_TABS.to_vec(),
            state: UiState::default(),
        };
        s.set_visible(s.config.content, true);
        s.set_canvas_visible(false);
        for i in 0..s.config.tabs.len() {
            s.show_tab(i, false);
        }
        s.state.active.insert(s.config.back_button, false);
        // The toolbar's `OnEnable` with the keyboard (no gamepad ported).
        for n in s.config.cycle_texts.clone() {
            s.state.active.insert(n, false);
        }
        s
    }

    /// `CanvasGroup.SetVisible`: alpha 1 or 0 (`interactable` and
    /// `blocksRaycasts` follow; not drawn).
    fn set_visible(&mut self, group: usize, visible: bool) {
        self.state
            .group_alpha
            .insert(group, if visible { 1.0 } else { 0.0 });
    }

    /// `SetCanvasVisible`: the canvas group (the scaler's `active` and
    /// the masks' `enabled` follow; the pose is the client's).
    fn set_canvas_visible(&mut self, visible: bool) {
        self.set_visible(self.config.canvas_group, visible);
    }

    /// A tab's `Open` (`visible`) or `Close`. The intro's `Close` also
    /// sets the background's alpha to 1 (`SetBackgroundAlpha`).
    fn show_tab(&mut self, index: usize, visible: bool) {
        let t = self.config.tabs[index];
        match t.show {
            TabShow::Content(n) => self.set_visible(n, visible),
            TabShow::Object(n) => {
                self.state.active.insert(n, visible);
            }
        }
        if t.tab == Tab::Intro && !visible {
            let bg = self.config.background;
            let mut c = self.state.color.get(&bg).copied().unwrap_or([1.0; 4]);
            c[3] = 1.0;
            self.state.color.insert(bg, c);
        }
    }

    fn tab_index(&self, tab: Tab) -> Option<usize> {
        self.config.tabs.iter().position(|t| t.tab == tab)
    }

    /// `OnOpenPDA(tabId)` (`None`: `PDATab.None`, the previous tab; the
    /// pop-up's encyclopedia shortcut is not reached).
    pub fn on_open_pda(&mut self, tab: Option<Tab>) {
        self.set_canvas_visible(true);
        self.state.group_alpha.insert(self.config.content, 1.0);
        let tab = match tab {
            Some(t) => Some(t),
            None if self.tab_open.is_none() => Some(self.tab_prev),
            None => None,
        };
        if let Some(t) = tab {
            self.open_tab(t);
        }
    }

    /// `OnPDAOpened`: the content interactable (not drawn).
    pub fn on_pda_opened(&mut self) {}

    /// `OnClosePDA`: the open tab's `Close`, `SetTabs(regularTabs)`, the
    /// content hidden.
    pub fn on_close_pda(&mut self) {
        if let Some(t) = self.tab_open.take()
            && let Some(i) = self.tab_index(t)
        {
            self.show_tab(i, false);
        }
        self.current_tabs = REGULAR_TABS.to_vec();
        self.set_visible(self.config.content, false);
    }

    /// `OnPDAClosed`: the canvas invisible.
    pub fn on_pda_closed(&mut self) {
        self.set_canvas_visible(false);
    }

    /// `OpenTab(tabId)`: the open tab closed, this one opened, `tabPrev`
    /// kept for the regular tabs, the toolbar shown when the tab is on it.
    /// False when nothing changed.
    pub fn open_tab(&mut self, tab: Tab) -> bool {
        let Some(i) = self.tab_index(tab) else {
            return false;
        };
        if self.tab_open == Some(tab) {
            return false;
        }
        if let Some(open) = self.tab_open
            && let Some(j) = self.tab_index(open)
        {
            self.show_tab(j, false);
        }
        self.state.active.insert(self.config.back_button, false);
        self.tab_open = Some(tab);
        self.show_tab(i, true);
        if REGULAR_TABS.contains(&tab) {
            self.tab_prev = tab;
        }
        let on_toolbar = self.current_tabs.contains(&tab);
        self.set_visible(self.config.toolbar_group, on_toolbar);
        true
    }
}

#[cfg(test)]
mod tests;
