//! The PDA's notifications (M9u8b5a, `docs/formats/ui.md` § Notifications):
//! `NotificationManager` and what its targets show. Our own code after
//! reading the game's `NotificationManager`, `Inventory`, `uGUI_ItemIcon`,
//! `uGUI_EquipmentSlot`, `uGUI_Toolbar` and `uGUI_PDA`.
//!
//! - `Add(group, key, d)`: duration and time left `max(0.1, d)`; a new
//!   one tells its target (`Progress(timeLeft / duration)`).
//!   `Inventory.Pickup` adds `(Inventory, the item, 4)`; `OnRemoveItem`
//!   (the grid) and `OnUnequip` remove it.
//! - `LateUpdate` (the PDA's delta): for each target, a visible one counts
//!   its notification down (`Progress(timeLeft / duration)`, removed at
//!   ≤ 0); a hidden one is removed once more than 1 s of its duration has
//!   passed, else shows full (`Progress(1)`) and starts over.
//! - `RegisterTarget`: an older visible target of the key gets
//!   `Progress(0)`; the new one, if visible, `timeLeft / duration`.
//!   `UnregisterTarget`: each of its keys loses it; its notification is
//!   removed if more than 1 s has passed, else starts over; the target
//!   gets `Progress(0)`.
//! - `GetCount(group)`: the uGUI_PDA's toolbar shows the inventory's.
//! - `uGUI_ItemIcon.SetNotificationProgress(p)`: the label ("+") and the
//!   glow at sin(p·π/2) ([`label_alpha`]).

use std::collections::BTreeMap;

use crate::layout::V2;
use crate::state::UiState;

/// `NotificationManager.Group` (the ones ported).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Inventory,
}

/// `NotificationId`: the group and the key (the item's id for the
/// inventory: the game's `UniqueIdentifier.Id`).
pub type Key = (Group, u64);

/// `NotificationData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Data {
    pub duration: f32,
    pub time_left: f32,
}

/// `minDuration`.
pub const MIN_DURATION: f32 = 0.1;
/// `thresholdDuration`: seen longer than this, a hidden notification goes.
pub const THRESHOLD: f32 = 1.0;
/// `Inventory.Pickup`'s duration.
pub const PICKUP_DURATION: f32 = 4.0;

/// `uGUI_ItemIcon.SetNotificationProgress`: the label's alpha and the
/// glow's strength for `progress`.
pub fn label_alpha(progress: f32) -> f32 {
    (progress * std::f32::consts::FRAC_PI_2)
        .sin()
        .clamp(0.0, 1.0)
}

/// `uGUI_EquipmentSlot`'s label offset: the `iconRect`'s size ×
/// −(1 − 1/√2)/2, on the circle at 45°.
pub fn slot_label_offset(icon_rect: [f32; 2]) -> [f32; 2] {
    [icon_rect[0] * -0.146_446_6, icon_rect[1] * -0.146_446_6]
}

/// `RectTransformExtensions.GetAnchorPoint(UIAnchor)`.
pub fn anchor_point(anchor: i32) -> V2 {
    match anchor {
        0 => [0.0, 0.0],
        1 => [0.5, 0.0],
        2 => [1.0, 0.0],
        3 => [0.0, 0.5],
        4 => [0.5, 0.5],
        5 => [1.0, 0.5],
        6 => [0.0, 1.0],
        7 => [0.5, 1.0],
        8 => [1.0, 1.0],
        _ => [0.0, 0.0],
    }
}

/// `uGUI_ItemIcon`'s label: anchor upper right, offset (−10, −10).
pub const ICON_LABEL_ANCHOR: i32 = 8;
pub const ICON_LABEL_OFFSET: V2 = [-10.0, -10.0];
/// `SetNotificationProgress`'s text.
pub const SYMBOL: &str = "+";

/// A grafted `NotificationLabel`: its root, `background` and `text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelNodes {
    pub root: usize,
    pub background: usize,
    pub text: usize,
}

/// A label as `uGUI_ItemIcon` leaves it this frame: shown with `text` at
/// `alpha` (`SetAlpha`: the background's and the text's alpha; the text
/// keeps its stored rgb), at `offset` from its anchor. The frame it is
/// made, `SetBackgroundColor(notificationColor)` comes after `SetAlpha`,
/// so its background is opaque white that frame.
pub fn label_state(
    state: &mut UiState,
    label: LabelNodes,
    text: &str,
    alpha: f32,
    offset: V2,
    text_color: [f32; 4],
    made: bool,
) {
    let a = alpha.clamp(0.0, 1.0);
    state.active.insert(label.root, true);
    state.color.insert(
        label.background,
        [1.0, 1.0, 1.0, if made { 1.0 } else { a }],
    );
    state
        .color
        .insert(label.text, [text_color[0], text_color[1], text_color[2], a]);
    state.text.insert(label.text, text.to_string());
    state
        .transform
        .entry(label.root)
        .or_default()
        .anchored_position = Some(offset);
}

/// `NotificationManager` with targets `T` (our icons). Each call returns
/// the `Progress` calls it made, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notifications<T: Copy + PartialEq> {
    pub notifications: BTreeMap<Key, Data>,
    pub targets: BTreeMap<Key, T>,
}

impl<T: Copy + PartialEq> Notifications<T> {
    pub fn new() -> Self {
        Notifications {
            notifications: BTreeMap::new(),
            targets: BTreeMap::new(),
        }
    }

    /// `GetCount(group)`.
    pub fn count(&self, group: Group) -> usize {
        self.notifications.keys().filter(|k| k.0 == group).count()
    }

    /// `Add(group, key, duration)`.
    pub fn add(&mut self, key: Key, duration: f32) -> Vec<(T, f32)> {
        let d = duration.max(MIN_DURATION);
        let new = !self.notifications.contains_key(&key);
        self.notifications.insert(
            key,
            Data {
                duration: d,
                time_left: d,
            },
        );
        match (new, self.targets.get(&key)) {
            (true, Some(&t)) => vec![(t, 1.0)],
            _ => Vec::new(),
        }
    }

    /// `Remove(group, key)`: its target gets `Progress(0)`.
    pub fn remove(&mut self, key: Key) -> Vec<(T, f32)> {
        if self.notifications.remove(&key).is_none() {
            return Vec::new();
        }
        self.targets
            .get(&key)
            .map(|&t| (t, 0.0))
            .into_iter()
            .collect()
    }

    /// `RegisterTarget(key, target)`.
    pub fn register(&mut self, key: Key, target: T, visible: &dyn Fn(T) -> bool) -> Vec<(T, f32)> {
        let mut out = Vec::new();
        if let Some(&old) = self.targets.get(&key)
            && visible(old)
        {
            out.push((old, 0.0));
        }
        self.targets.insert(key, target);
        if let Some(d) = self.notifications.get(&key)
            && visible(target)
        {
            out.push((target, d.time_left / d.duration));
        }
        out
    }

    /// `UnregisterTarget(target)`.
    pub fn unregister(&mut self, target: T) -> Vec<(T, f32)> {
        let keys: Vec<Key> = self
            .targets
            .iter()
            .filter(|(_, t)| **t == target)
            .map(|(k, _)| *k)
            .collect();
        for k in keys {
            self.targets.remove(&k);
            if let Some(d) = self.notifications.get_mut(&k) {
                if d.duration - d.time_left > THRESHOLD {
                    self.notifications.remove(&k);
                } else {
                    d.time_left = d.duration;
                }
            }
        }
        vec![(target, 0.0)]
    }

    /// `LateUpdate` with the PDA's `dt`.
    pub fn late_update(&mut self, dt: f32, visible: &dyn Fn(T) -> bool) -> Vec<(T, f32)> {
        let mut out = Vec::new();
        let targets: Vec<(Key, T)> = self.targets.iter().map(|(k, t)| (*k, *t)).collect();
        for (key, target) in targets {
            let Some(d) = self.notifications.get_mut(&key) else {
                continue;
            };
            if visible(target) {
                d.time_left -= dt;
                if d.time_left > 0.0 {
                    out.push((target, d.time_left / d.duration));
                    continue;
                }
                self.notifications.remove(&key);
                out.push((target, 0.0));
            } else if d.duration - d.time_left > THRESHOLD {
                self.notifications.remove(&key);
                out.push((target, 0.0));
            } else {
                out.push((target, 1.0));
                d.time_left = d.duration;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const K: Key = (Group::Inventory, 7);

    #[test]
    fn a_pickup_counts_down_only_while_seen() {
        let mut n = Notifications::<u32>::new();
        assert!(n.add(K, PICKUP_DURATION).is_empty());
        assert_eq!(n.count(Group::Inventory), 1);
        // No target: nothing counts down.
        assert!(n.late_update(10.0, &|_| true).is_empty());
        assert_eq!(n.notifications[&K].time_left, 4.0);
        // A hidden target: full, starting over.
        assert!(n.register(K, 1, &|_| false).is_empty());
        assert_eq!(n.late_update(0.5, &|_| false), [(1, 1.0)]);
        assert_eq!(n.notifications[&K].time_left, 4.0);
        // Seen: 4 s down, then gone.
        assert_eq!(n.late_update(1.0, &|_| true), [(1, 0.75)]);
        assert_eq!(n.late_update(2.0, &|_| true), [(1, 0.25)]);
        assert_eq!(n.count(Group::Inventory), 1);
        assert_eq!(n.late_update(1.0, &|_| true), [(1, 0.0)]);
        assert_eq!(n.count(Group::Inventory), 0);
        assert!(n.late_update(1.0, &|_| true).is_empty());
    }

    #[test]
    fn hidden_after_a_second_seen_it_goes() {
        let mut n = Notifications::<u32>::new();
        n.add(K, 4.0);
        n.register(K, 1, &|_| true);
        n.late_update(0.5, &|_| true);
        // Half a second seen: hidden again, it starts over.
        assert_eq!(n.late_update(0.1, &|_| false), [(1, 1.0)]);
        n.late_update(1.5, &|_| true);
        assert_eq!(n.late_update(0.1, &|_| false), [(1, 0.0)]);
        assert_eq!(n.count(Group::Inventory), 0);
    }

    #[test]
    fn targets_and_removal() {
        let mut n = Notifications::<u32>::new();
        n.register(K, 1, &|_| true);
        // A new notification tells its target.
        assert_eq!(n.add(K, 4.0), [(1, 1.0)]);
        assert!(n.add(K, 0.0).is_empty());
        assert_eq!(n.notifications[&K].duration, MIN_DURATION);
        n.add(K, 4.0);
        n.late_update(0.5, &|_| true);
        // A new target: the old one cleared, the new one at 3.5 / 4.
        assert_eq!(n.register(K, 2, &|_| true), [(1, 0.0), (2, 0.875)]);
        // Unregistered within the first second: it starts over.
        assert_eq!(n.unregister(2), [(2, 0.0)]);
        assert_eq!(n.notifications[&K].time_left, 4.0);
        n.register(K, 3, &|_| true);
        n.late_update(1.5, &|_| true);
        n.unregister(3);
        assert_eq!(n.count(Group::Inventory), 0);
        n.add(K, 4.0);
        n.register(K, 4, &|_| true);
        assert_eq!(n.remove(K), [(4, 0.0)]);
        assert!(n.remove(K).is_empty());
    }

    #[test]
    fn label_alpha_and_slot_offset() {
        assert_eq!(label_alpha(0.0), 0.0);
        assert!((label_alpha(0.5) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((label_alpha(1.0) - 1.0).abs() < 1e-6);
        let o = slot_label_offset([124.0, 124.0]);
        assert!((o[0] + 18.159_38).abs() < 1e-3 && o[0] == o[1]);
        assert_eq!(anchor_point(8), [1.0, 1.0]);
        assert_eq!(anchor_point(1), [0.5, 0.0]);
    }

    #[test]
    fn a_new_label_has_an_opaque_background_for_a_frame() {
        let n = LabelNodes {
            root: 1,
            background: 2,
            text: 3,
        };
        let mut s = UiState::default();
        label_state(
            &mut s,
            n,
            "+",
            0.5,
            [-10.0, -10.0],
            [0.2, 0.3, 0.4, 1.0],
            true,
        );
        assert_eq!(s.color[&2], [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(s.color[&3], [0.2, 0.3, 0.4, 0.5]);
        label_state(
            &mut s,
            n,
            "+",
            0.5,
            [-10.0, -10.0],
            [0.2, 0.3, 0.4, 1.0],
            false,
        );
        assert_eq!(s.color[&2], [1.0, 1.0, 1.0, 0.5]);
        assert_eq!(s.text[&3], "+");
        assert_eq!(s.transform[&1].anchored_position, Some([-10.0, -10.0]));
    }
}
