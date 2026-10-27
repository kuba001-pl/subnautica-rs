//! Dragging an item on the inventory tab (M9u8b4, `docs/formats/ui.md`
//! § Drag and drop): the pointer's press, drag and release, the slots'
//! feedback and the dragged icon's snap. Our own code after reading the
//! game's `FPSInputModule`, `ItemDragManager`, `uGUI_EquipmentSlot`,
//! `uGUI_ItemsContainer`, `uGUI_Equipment` and `MathExtensions`.
//!
//! - `FPSInputModule.ProcessMousePress`: a press remembers the pressed
//!   object (`pointerPress`, which is also `pointerDrag` for an item icon
//!   or a slot) and where; `ProcessDrag` starts the drag once the pointer
//!   is `pixelDragThreshold` pixels or more from there. On the release the
//!   click goes to the pressed object if the pointer is still over it
//!   (`eligibleForClick` stays true: `pointerPress == pointerDrag`), else
//!   the drop to what is under the pointer; then `OnEndDrag`
//!   (`ItemDragManager.DragStop`). A click during a drag does nothing
//!   (`uGUI_InventoryTab.OnPointerClick` checks `isDragging`).
//! - `uGUI_EquipmentSlot.UpdateFeedbackState`: selected (gamepad, not
//!   ported) else compatible: allowed while dragging and hovered, else
//!   highlighted; not compatible: denied while dragging and hovered, else
//!   default.
//! - `ItemDragManager.OnLateUpdate`: `dragSnapT` moves to 1 when a target
//!   asked for a snap this frame (`DragSnap`), else to 0, by
//!   `StableLerp(t, target, dragSnapSpeed = 15, PDA.deltaTime)`; it starts
//!   at 1 (snapped to the source).

use crate::inventory_tab::{TINT_DEFAULT, TINT_HIGHLIGHTED};
use crate::layout::V2;

/// `uGUI_EquipmentSlot.tintAllowed` (0, 255, 0, 255).
pub const TINT_ALLOWED: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
/// `tintDenied` (255, 0, 0, 255).
pub const TINT_DENIED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
/// `uGUI_ItemIcon.OnLock`'s alpha and `ItemDragManager.swapAlpha`.
pub const LOCK_ALPHA: f32 = 0.3;
/// `ItemDragManager.dragSnapSpeed`.
pub const SNAP_SPEED: f32 = 15.0;
/// `EventSystem.pixelDragThreshold`: Unity's default (*hypothesis*: the
/// game's `EventSystem` object was not found in its data).
pub const DRAG_THRESHOLD: f32 = 10.0;
/// The action hint's scale per metre of depth (`OnLateUpdate`).
pub const HINT_SCALE: f32 = 0.001_080_918;

/// `UpdateFeedbackState`'s tint for a slot (not selected).
pub fn slot_tint(compatible: bool, hovered: bool, dragging: bool) -> [f32; 4] {
    match (compatible, dragging && hovered) {
        (true, true) => TINT_ALLOWED,
        (true, false) => TINT_HIGHLIGHTED,
        (false, true) => TINT_DENIED,
        (false, false) => TINT_DEFAULT,
    }
}

/// `MathExtensions.StableLerp(current, target, coef, deltaTime)`.
pub fn stable_lerp(current: f32, target: f32, coef: f32, dt: f32) -> f32 {
    let n = 1.0 - coef / 60.0;
    if n <= 0.0 {
        return target;
    }
    let t = (1.0 - n.powf(60.0 * dt)).clamp(0.0, 1.0);
    current + (target - current) * t
}

/// One frame of `dragSnapT`.
pub fn snap_step(t: f32, snap: bool, dt: f32) -> f32 {
    stable_lerp(t, if snap { 1.0 } else { 0.0 }, SNAP_SPEED, dt)
}

/// Where the dragged icon goes when snapped: `DragSnap`'s position (in
/// the canvas's space), sizes and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub at: V2,
    pub size: V2,
    pub radius: f32,
}

impl Snap {
    /// `uGUI_EquipmentSlot`'s and an empty slot's (`Switch`): the
    /// `iconRect`'s centre and size, radius min(size) ÷ 2.
    pub fn slot(centre: V2, size: V2) -> Snap {
        Snap {
            at: centre,
            size,
            radius: size[0].min(size[1]) * 0.5,
        }
    }

    /// A swap target's (an icon or an occupied slot): down and right of
    /// its centre by half the dragged size, at the dragged size, radius 33.
    pub fn swap(centre: V2, dragged: V2) -> Snap {
        Snap {
            at: [centre[0] + dragged[0] * 0.5, centre[1] - dragged[1] * 0.5],
            size: dragged,
            radius: crate::item_icons::ICON_RADIUS,
        }
    }

    /// `UpdateIconTransform`: the cursor's place at `GetIconSize` and
    /// radius 33 blended to the snap by `t`.
    pub fn blend(&self, cursor: V2, dragged: V2, t: f32) -> Snap {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        Snap {
            at: [lerp(cursor[0], self.at[0]), lerp(cursor[1], self.at[1])],
            size: [
                lerp(dragged[0], self.size[0]),
                lerp(dragged[1], self.size[1]),
            ],
            radius: lerp(crate::item_icons::ICON_RADIUS, self.radius),
        }
    }
}

/// The swap hint's offset from the dragged icon's centre:
/// (−w·0.45, h·0.45) of the dragged size.
pub fn swap_hint_offset(dragged: V2) -> V2 {
    [-dragged[0] * 0.5 * 0.9, dragged[1] * 0.5 * 0.9]
}

/// What a release does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// A click on the pressed item (no drag started).
    Click(u64),
    /// The click on the dragged item itself: `OnPointerClick` ignores it
    /// while dragging; only the drag stops.
    ClickWhileDragging(u64),
    /// The dragged item dropped where the pointer is.
    Drop(u64),
}

/// The left button's press and drag on the tab's items.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointerDrag {
    /// The pressed item and the press's pixel.
    press: Option<(u64, V2)>,
    /// The dragged item, once the drag started.
    pub dragged: Option<u64>,
}

impl PointerDrag {
    /// A press on `id` at `pixel`.
    pub fn press(&mut self, id: u64, pixel: V2) {
        self.press = Some((id, pixel));
        self.dragged = None;
    }

    /// A press on `id` that drags at once (`--panel`'s scripted drag).
    pub fn start(&mut self, id: u64, pixel: V2) {
        self.press = Some((id, pixel));
        self.dragged = Some(id);
    }

    /// The pointer at `pixel`: true when the drag starts now.
    pub fn motion(&mut self, pixel: V2, threshold: f32) -> bool {
        let Some((id, p)) = self.press else {
            return false;
        };
        let d = [pixel[0] - p[0], pixel[1] - p[1]];
        if self.dragged.is_none() && d[0] * d[0] + d[1] * d[1] >= threshold * threshold {
            self.dragged = Some(id);
            return true;
        }
        false
    }

    /// The release with the pointer over `over`.
    pub fn release(&mut self, over: Option<u64>) -> Option<Release> {
        let (id, _) = self.press.take()?;
        let dragged = self.dragged.take().is_some();
        match (over == Some(id), dragged) {
            (true, false) => Some(Release::Click(id)),
            (true, true) => Some(Release::ClickWhileDragging(id)),
            (false, true) => Some(Release::Drop(id)),
            // `OnDrop` with nothing dragged returns.
            (false, false) => None,
        }
    }

    pub fn cancel(&mut self) {
        self.press = None;
        self.dragged = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_starts_at_the_threshold_and_drops_elsewhere() {
        let mut p = PointerDrag::default();
        p.press(7, [0.0, 0.0]);
        assert!(!p.motion([6.0, 7.9], 10.0));
        assert!(p.motion([6.0, 8.0], 10.0));
        assert!(!p.motion([20.0, 8.0], 10.0));
        assert_eq!(p.dragged, Some(7));
        assert_eq!(p.release(Some(9)), Some(Release::Drop(7)));
        assert_eq!(p.release(None), None);
    }

    #[test]
    fn a_release_on_the_pressed_item_clicks_unless_dragging() {
        let mut p = PointerDrag::default();
        p.press(8, [0.0; 2]);
        assert_eq!(p.release(Some(8)), Some(Release::Click(8)));
        p.press(8, [0.0; 2]);
        assert_eq!(p.release(Some(9)), None);
        p.press(8, [0.0; 2]);
        p.motion([30.0, 0.0], 10.0);
        assert_eq!(p.release(Some(8)), Some(Release::ClickWhileDragging(8)));
        p.start(5, [0.0; 2]);
        p.cancel();
        assert_eq!(p.release(None), None);
    }

    #[test]
    fn slot_feedback_follows_update_feedback_state() {
        assert_eq!(slot_tint(true, true, true), TINT_ALLOWED);
        assert_eq!(slot_tint(false, true, true), TINT_DENIED);
        assert_eq!(slot_tint(true, false, true), TINT_HIGHLIGHTED);
        assert_eq!(slot_tint(false, false, true), TINT_DEFAULT);
        assert_eq!(slot_tint(true, true, false), TINT_HIGHLIGHTED);
        assert_eq!(slot_tint(false, true, false), TINT_DEFAULT);
    }

    #[test]
    fn the_snap_is_stable_across_frame_rates() {
        // 1 − 15 / 60 = 0.75 left per 60th of a second.
        assert!((snap_step(0.0, true, 1.0 / 60.0) - 0.25).abs() < 1e-6);
        let two = snap_step(snap_step(0.0, true, 1.0 / 60.0), true, 1.0 / 60.0);
        assert!((two - snap_step(0.0, true, 1.0 / 30.0)).abs() < 1e-6);
        assert!((snap_step(1.0, false, 1.0 / 60.0) - 0.75).abs() < 1e-6);
        assert_eq!(snap_step(0.4, true, 0.0), 0.4);
        assert_eq!(stable_lerp(0.0, 1.0, 60.0, 0.01), 1.0);
    }

    #[test]
    fn snaps_and_blend() {
        let s = Snap::slot([10.0, 20.0], [80.0, 60.0]);
        assert_eq!(s.radius, 30.0);
        let w = Snap::swap([10.0, 20.0], [137.0, 66.0]);
        assert_eq!(w.at, [78.5, -13.0]);
        assert_eq!(w.radius, 33.0);
        let b = s.blend([0.0, 0.0], [66.0, 66.0], 0.5);
        assert_eq!(b.at, [5.0, 10.0]);
        assert_eq!(b.size, [73.0, 63.0]);
        assert_eq!(b.radius, 31.5);
        assert_eq!(s.blend([0.0; 2], [66.0; 2], 1.0), s);
        assert_eq!(swap_hint_offset([100.0, 50.0]), [-45.0, 22.5]);
    }
}
