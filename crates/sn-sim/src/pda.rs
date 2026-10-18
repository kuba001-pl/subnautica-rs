//! The PDA (M9u8a1, `docs/DESIGN.md` "M9u8a"): `PDA`'s open and close
//! sequence, its state, the close by button and by distance, and
//! `PDA.time`. Our own code after reading the game's `PDA`, `Sequence`
//! and `uGUI_InputGroup`; facts in `docs/formats/gameplay.md` § The PDA.
//!
//! Times are `f32` as the game's: the sequence ends on the update where
//! `t` (summed in floats) is first exactly at its end.

use crate::V3;

/// `PDA.timeDraw`: the opening sequence.
pub const TIME_DRAW: f32 = 0.5;
/// `PDA.timeHolster`: the closing sequence.
pub const TIME_HOLSTER: f32 = 0.3;

/// `PDA.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Opened,
    Closed,
    Opening,
    Closing,
}

/// What the PDA did in an update or a call, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `Open` went through: the model is active, `using_pda` is set and
    /// the screen's `OnOpenPDA` runs.
    Open,
    /// `Close` went through (`using_pda` cleared, `OnClosePDA`, then the
    /// `onClose` callback, e.g. `EscapePod.StorageOnClosePDA`).
    Close(CloseReason),
    /// `Activated`: the opening sequence ended (the cursor freed, the
    /// screen selected, `OnPDAOpened`).
    Activated,
    /// `Deactivated`: the closing sequence ended (`OnPDAClosed`, the
    /// model inactive, `isInUse` false).
    Deactivated,
}

/// Why `ManagedUpdate` closed the PDA (or the caller did).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// `Button.PDA` while focused.
    Button,
    /// The target is gone or at least `activeSqrDistance` away.
    Distance,
    /// A call from elsewhere (`uGUI_PDATab`'s cancel button, death, …).
    Call,
}

/// The sequence's callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Callback {
    Activated,
    Deactivated,
}

/// UWE's `Sequence` in floats, with the PDA's two callbacks.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sequence {
    t: f32,
    time: f32,
    target: bool,
    dir: f32,
    active: bool,
    callback: Option<Callback>,
}

impl Sequence {
    /// `new Sequence(initialState)`: `Set(0, state, state)`.
    fn new(state: bool) -> Sequence {
        Sequence {
            t: if state { 1.0 } else { 0.0 },
            time: 0.0,
            target: state,
            dir: if state { 1.0 } else { -1.0 },
            active: true,
            callback: None,
        }
    }

    fn set(&mut self, time: f32, target: bool, callback: Callback) {
        self.time = time;
        self.target = target;
        self.dir = if target { 1.0 } else { -1.0 };
        self.callback = Some(callback);
        self.active = true;
    }

    /// `Update(dT)`: the callback when this call ends the sequence.
    fn update(&mut self, dt: f32) -> Option<Callback> {
        if !self.active {
            return None;
        }
        let end = if self.target { 1.0 } else { 0.0 };
        if self.t == end {
            self.active = false;
            return self.callback;
        }
        if self.time == 0.0 {
            self.t = end;
        } else {
            self.t = (self.t + self.dir * dt / self.time).clamp(0.0, 1.0);
        }
        None
    }
}

/// `(a − b).sqrMagnitude`.
fn sqr_distance(a: V3, b: V3) -> f64 {
    let d = a - b;
    d.dot(d)
}

/// The player's PDA.
#[derive(Clone, Debug, PartialEq)]
pub struct Pda {
    sequence: Sequence,
    /// `isInUse`: from `Open` to `Deactivated`.
    in_use: bool,
    /// The PDA's GameObject (`PlayerPDA`, the model in the hand) is
    /// active: from `Open` to `Deactivated`.
    active: bool,
    /// `ui.focused`: the screen selected (from `Activated`) until
    /// `OnClosePDA` deselects it.
    focused: bool,
    /// `ignorePDAInput` (`SetIgnorePDAInput`: death, inspecting).
    ignore_input: bool,
    /// `targetWasSet` and `activeSqrDistance` (the target's position is
    /// the caller's, each update).
    target_was_set: bool,
    active_sqr_distance: f64,
    /// `ArmsController.SetUsingPda`'s last value (`using_pda`).
    using_pda: bool,
    /// `PDA.time` and `PDA.deltaTime` (no `pdaPause`: the option is off
    /// by default, so `deltaTime` is `Time.deltaTime`).
    pub time: f32,
    pub delta_time: f32,
}

impl Default for Pda {
    fn default() -> Pda {
        Pda::new()
    }
}

impl Pda {
    /// As the main scene holds it: closed, its model inactive.
    pub fn new() -> Pda {
        Pda {
            sequence: Sequence::new(false),
            in_use: false,
            active: false,
            focused: false,
            ignore_input: false,
            target_was_set: false,
            active_sqr_distance: 0.0,
            using_pda: false,
            time: 0.0,
            delta_time: 0.0,
        }
    }

    /// `PDA.state`.
    pub fn state(&self) -> State {
        if self.sequence.target {
            if self.sequence.active {
                State::Opening
            } else {
                State::Opened
            }
        } else if !self.active || !self.sequence.active {
            State::Closed
        } else {
            State::Closing
        }
    }

    /// `isInUse`.
    pub fn in_use(&self) -> bool {
        self.in_use
    }

    /// The model is drawn (the PDA's GameObject is active).
    pub fn active(&self) -> bool {
        self.active
    }

    /// `isFocused`.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// The arms' `using_pda`.
    pub fn using_pda(&self) -> bool {
        self.using_pda
    }

    /// The sequence's `t` (0 closed … 1 open).
    pub fn t(&self) -> f32 {
        self.sequence.t
    }

    /// `SetIgnorePDAInput`.
    pub fn set_ignore_input(&mut self, ignore: bool) {
        self.ignore_input = ignore;
    }

    /// `Open(tab, target, onClose)`: refused while in use, with its input
    /// ignored, or in a cinematic (`Inventory.ReturnHeld` holds nothing
    /// to refuse with: no tools). `target` is the target's position and
    /// `player` the player's (`main.transform.position`).
    pub fn open(&mut self, target: Option<V3>, player: V3, cinematic: bool) -> bool {
        if self.in_use || self.ignore_input || cinematic {
            return false;
        }
        self.in_use = true;
        self.using_pda = true;
        self.active = true;
        self.sequence.set(TIME_DRAW, true, Callback::Activated);
        self.target_was_set = target.is_some();
        if let Some(t) = target {
            self.active_sqr_distance = sqr_distance(t, player) + 1.0;
        }
        true
    }

    /// `Close()`: nothing unless in use and not ignoring input. Returns
    /// whether it closed (then the caller runs its `onClose`).
    pub fn close(&mut self) -> bool {
        if !self.in_use || self.ignore_input {
            return false;
        }
        self.target_was_set = false;
        self.using_pda = false;
        // `ui.OnClosePDA` → `Deselect`.
        self.focused = false;
        self.sequence
            .set(TIME_HOLSTER, false, Callback::Deactivated);
        true
    }

    /// `PDA.PerformUpdate` for one frame of `dt` seconds: `UpdateTime`,
    /// and while the model is active `ManagedUpdate` (the sequence, then
    /// the close by `button` (`Button.PDA` pressed this frame) or by the
    /// target's distance). `target` is the target's position now
    /// (`None`: it is gone).
    pub fn update(&mut self, dt: f32, button: bool, target: Option<V3>, player: V3) -> Vec<Event> {
        self.delta_time = dt;
        self.time += dt;
        let mut events = Vec::new();
        if !self.active {
            return events;
        }
        match self.sequence.update(dt) {
            Some(Callback::Activated) => {
                // `ui.Select()`: focused (the input group's dummy is
                // pushed at once).
                self.focused = true;
                events.push(Event::Activated);
            }
            Some(Callback::Deactivated) => {
                self.active = false;
                self.in_use = false;
                events.push(Event::Deactivated);
            }
            None => {}
        }
        if self.in_use && self.focused && button {
            if self.close() {
                events.push(Event::Close(CloseReason::Button));
            }
        } else if self.target_was_set {
            let far = target.is_none_or(|t| sqr_distance(t, player) >= self.active_sqr_distance);
            if far && self.close() {
                events.push(Event::Close(CloseReason::Distance));
            }
        }
        events
    }
}

#[cfg(test)]
mod tests;
