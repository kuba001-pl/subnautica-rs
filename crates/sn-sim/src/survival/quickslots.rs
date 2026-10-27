//! Item-identity quick slots and held-tool transitions (M9s5).
//! Grid descriptions borrow identities, never own another inventory item.
use super::RuleError;
use crate::inventory::{ItemId, TechType};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToolTiming {
    pub draw: f32,
    pub holster: f32,
    pub drop: f32,
    pub bash: f32,
    pub animated: bool,
    pub named_animation: bool,
    pub first_use: bool,
}
impl ToolTiming {
    fn validate(self) -> Result<(), RuleError> {
        if [self.draw, self.holster, self.drop, self.bash]
            .iter()
            .all(|x| x.is_finite())
        {
            Ok(())
        } else {
            Err(RuleError("invalid tool timing"))
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuickRules {
    pub slots: usize,
    pub hand: i32,
    pub scrap: TechType,
    pub selectable: i32,
    pub selectable_chargeable: i32,
    pub defaults: ToolTiming,
    pub transition_max: f32,
}
impl QuickRules {
    pub fn validate(&self) -> Result<(), RuleError> {
        self.defaults.validate()?;
        // Structural allocation limit, our own; not a gameplay balance value.
        if self.slots == 0
            || self.slots > 1024
            || !self.transition_max.is_finite()
            || self.transition_max < 0.0
        {
            Err(RuleError("invalid quick-slot rules"))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuickItem {
    pub id: ItemId,
    pub tech: TechType,
    pub equipment_type: i32,
    pub slot_type: i32,
    pub tool: Option<ToolTiming>,
    pub eat: bool,
    pub use_action: bool,
    pub plantable: bool,
    pub live_mixin: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmsState {
    None,
    Draw,
    Hold,
    Holster,
    Drop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickEvent {
    Bound(usize, Option<ItemId>),
    Equip(ItemId),
    Unequip(ItemId),
    FirstUse(TechType),
    Animation(ItemId, bool),
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct Transition {
    t: f32,
    duration: f32,
    target: bool,
    active: bool,
}
impl Default for Transition {
    fn default() -> Self {
        Self {
            t: 0.0,
            duration: 0.0,
            target: false,
            active: false,
        }
    }
}
impl Transition {
    fn set(&mut self, duration: f32, target: bool) {
        *self = Self {
            t: if target { 0.0 } else { 1.0 },
            duration,
            target,
            active: true,
        };
    }
    fn update(&mut self, dt: f32) -> bool {
        if !self.active {
            return false;
        }
        let end = if self.target { 1.0 } else { 0.0 };
        if self.t == end {
            self.active = false;
            return true;
        }
        self.t = if self.duration == 0.0 {
            end
        } else {
            (self.t
                + if self.target {
                    dt / self.duration
                } else {
                    -dt / self.duration
                })
            .clamp(0.0, 1.0)
        };
        false
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct QuickUpdate {
    pub now: f64,
    pub dt: f32,
    pub tool_in_use: bool,
    pub time_bashed: Option<f64>,
    pub vr_animation: bool,
    pub debug_first_use: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QuickSlots {
    bindings: Vec<Option<ItemId>>,
    binding_types: Vec<Option<TechType>>,
    last_update: Option<f64>,
    desired: Option<usize>,
    held: Option<QuickItem>,
    state: ArmsState,
    sequence: Transition,
    refill: Option<(usize, TechType)>,
    used: BTreeSet<TechType>,
    suspended: bool,
    previous: Option<usize>,
}
impl QuickSlots {
    pub fn new(r: &QuickRules) -> Result<Self, RuleError> {
        r.validate()?;
        Ok(Self {
            bindings: vec![None; r.slots],
            binding_types: vec![None; r.slots],
            last_update: None,
            desired: None,
            held: None,
            state: ArmsState::None,
            sequence: Transition::default(),
            refill: None,
            used: BTreeSet::new(),
            suspended: false,
            previous: None,
        })
    }
    pub fn bindings(&self) -> &[Option<ItemId>] {
        &self.bindings
    }
    pub fn desired(&self) -> Option<usize> {
        self.desired
    }
    pub fn held(&self) -> Option<ItemId> {
        self.held.map(|i| i.id)
    }
    pub fn state(&self) -> ArmsState {
        self.state
    }
    pub fn used_tools(&self) -> &BTreeSet<TechType> {
        &self.used
    }
    pub fn suspended(&self) -> bool {
        self.suspended
    }
    fn slot_of(&self, id: ItemId) -> Option<usize> {
        self.bindings.iter().position(|x| *x == Some(id))
    }
    fn check(&self, r: &QuickRules, grid: &[QuickItem]) -> Result<(), RuleError> {
        r.validate()?;
        if r.slots != self.bindings.len() {
            return Err(RuleError("quick-slot rule count changed"));
        }
        let mut ids = BTreeSet::new();
        for item in grid {
            if !ids.insert(item.id) {
                return Err(RuleError("duplicate grid identity"));
            }
            if let Some(t) = item.tool {
                t.validate()?;
                if t.animated && !t.named_animation {
                    return Err(RuleError("animated tool missing animation identity"));
                }
            }
        }
        Ok(())
    }
    pub fn bind(
        &mut self,
        r: &QuickRules,
        grid: &[QuickItem],
        slot: usize,
        id: ItemId,
    ) -> Result<Vec<QuickEvent>, RuleError> {
        self.check(r, grid)?;
        let Some(item) = grid.iter().find(|i| i.id == id) else {
            return Err(RuleError("binding item absent from grid"));
        };
        if item.equipment_type != r.hand || slot >= self.bindings.len() {
            return Err(RuleError("item/slot not bindable"));
        }
        let mut events = Vec::new();
        if let Some(old) = self.slot_of(id) {
            self.bindings[old] = None;
            self.binding_types[old] = None;
            events.push(QuickEvent::Bound(old, None));
            if old == slot {
                return Ok(events);
            }
        }
        self.bindings[slot] = Some(id);
        self.binding_types[slot] = Some(item.tech);
        events.push(QuickEvent::Bound(slot, Some(id)));
        Ok(events)
    }
    pub fn on_add(
        &mut self,
        r: &QuickRules,
        grid: &[QuickItem],
        id: ItemId,
        avatar_enabled: bool,
    ) -> Result<Vec<QuickEvent>, RuleError> {
        self.check(r, grid)?;
        let Some(i) = grid.iter().find(|i| i.id == id) else {
            return Err(RuleError("added item absent"));
        };
        if i.equipment_type != r.hand
            || i.tech == r.scrap
            || grid.iter().filter(|x| x.tech == i.tech).count() > 1
            || (i.tool.is_none() && !i.use_action && !i.eat)
            || (i.eat && (i.plantable || !i.live_mixin))
        {
            return Ok(Vec::new());
        }
        let Some(slot) = self.bindings.iter().position(Option::is_none) else {
            return Ok(Vec::new());
        };
        let events = self.bind(r, grid, slot, id)?;
        if self.held.is_none() && avatar_enabled {
            self.select(slot);
        }
        Ok(events)
    }
    pub fn select(&mut self, slot: usize) {
        if slot < self.bindings.len() {
            self.desired = if self.desired == Some(slot) {
                None
            } else {
                Some(slot)
            };
        }
    }
    pub fn key(
        &mut self,
        r: &QuickRules,
        grid: &[QuickItem],
        slot: usize,
        hovered: Option<ItemId>,
        input_allowed: bool,
    ) -> Result<Vec<QuickEvent>, RuleError> {
        self.check(r, grid)?;
        if self.suspended {
            return match hovered {
                Some(id) => self.bind(r, grid, slot, id),
                None => Ok(Vec::new()),
            };
        }
        if input_allowed {
            self.select(slot);
        }
        Ok(Vec::new())
    }
    pub fn scroll(
        &mut self,
        r: &QuickRules,
        grid: &[QuickItem],
        forward: bool,
        input_allowed: bool,
    ) -> Result<(), RuleError> {
        self.check(r, grid)?;
        if !input_allowed || self.suspended {
            return Ok(());
        }
        let mut at = if forward {
            self.desired.map_or(-1, |s| s as isize)
        } else {
            self.desired
                .map_or(self.bindings.len() as isize, |s| s as isize)
        };
        loop {
            at += if forward { 1 } else { -1 };
            let Ok(slot) = usize::try_from(at) else {
                self.desired = None;
                break;
            };
            let Some(binding) = self.bindings.get(slot) else {
                self.desired = None;
                break;
            };
            if let Some(item) = binding.and_then(|id| grid.iter().find(|i| i.id == id))
                && (item.slot_type == r.selectable || item.slot_type == r.selectable_chargeable)
            {
                self.select(slot);
                break;
            }
        }
        Ok(())
    }
    fn release(&mut self, events: &mut Vec<QuickEvent>) {
        if let Some(item) = self.held.take() {
            events.push(QuickEvent::Unequip(item.id));
        }
    }
    /// Inventory.ReturnHeld refuses while a tool is in use. PDA.Open remembers
    /// the active binding then returns held immediately and suspends activation.
    pub fn open_pda(&mut self, tool_in_use: bool) -> Option<Vec<QuickEvent>> {
        if tool_in_use || self.suspended {
            return None;
        }
        self.previous = self.held.and_then(|i| self.slot_of(i.id));
        let events = self.deselect_immediate();
        self.suspended = true;
        Some(events)
    }
    pub fn close_pda(&mut self) {
        self.suspended = false;
        self.desired = self.previous.take();
    }
    pub fn deselect_immediate(&mut self) -> Vec<QuickEvent> {
        let mut events = Vec::new();
        if let Some(i) = self.held {
            events.push(QuickEvent::Animation(i.id, false));
        }
        self.release(&mut events);
        self.desired = None;
        self.state = ArmsState::None;
        self.sequence = Transition::default();
        events
    }
    /// Death/interaction teardown has no ownership transfer. Inventory removal
    /// is reconciled on update; first-use flags remain per player and tech type.
    pub fn death(&mut self) -> Vec<QuickEvent> {
        self.suspended = false;
        self.previous = None;
        self.deselect_immediate()
    }
    fn timing(&self, r: &QuickRules) -> ToolTiming {
        self.held.and_then(|i| i.tool).unwrap_or(r.defaults)
    }
    fn start_transition(&mut self, r: &QuickRules, state: ArmsState, events: &mut Vec<QuickEvent>) {
        let timing = self.timing(r);
        let duration = match state {
            ArmsState::Draw => timing.draw,
            ArmsState::Holster => timing.holster,
            ArmsState::Drop => timing.drop,
            _ => 0.0,
        }
        .clamp(0.0, r.transition_max);
        self.state = state;
        self.sequence.set(duration, state == ArmsState::Draw);
        if let Some(i) = self.held {
            events.push(QuickEvent::Animation(i.id, state == ArmsState::Draw));
        }
    }
    pub fn update(
        &mut self,
        r: &QuickRules,
        grid: &[QuickItem],
        input: QuickUpdate,
    ) -> Result<Vec<QuickEvent>, RuleError> {
        self.check(r, grid)?;
        if !input.now.is_finite()
            || !input.dt.is_finite()
            || input.dt < 0.0
            || input.time_bashed.is_some_and(|t| !t.is_finite())
        {
            return Err(RuleError("invalid quick-slot clock"));
        }
        let mut events = Vec::new();
        // Sequence callbacks precede UpdateState, with no leftover dt carried.
        let advance = self.last_update != Some(input.now);
        self.last_update = Some(input.now);
        if advance && self.sequence.update(input.dt) {
            match self.state {
                ArmsState::Draw => self.state = ArmsState::Hold,
                ArmsState::Holster => {
                    self.state = ArmsState::None;
                    self.release(&mut events);
                }
                ArmsState::Drop => self.state = ArmsState::None,
                _ => {}
            }
        }
        for slot in 0..self.bindings.len() {
            if let Some(id) = self.bindings[slot]
                && !grid.iter().any(|i| i.id == id)
            {
                if let Some(tech) = self.binding_types[slot].take() {
                    self.refill = Some((slot, tech));
                }
                if self.held() == Some(id) {
                    self.start_transition(r, ArmsState::Drop, &mut events);
                    self.release(&mut events);
                    self.desired = Some(slot);
                }
                self.bindings[slot] = None;
                events.push(QuickEvent::Bound(slot, None));
            }
        }
        if let Some((slot, tech)) = self.refill.take()
            && let Some(i) = grid.iter().find(|i| {
                i.tech == tech && self.slot_of(i.id).is_none() && i.equipment_type == r.hand
            })
        {
            self.bindings[slot] = Some(i.id);
            self.binding_types[slot] = Some(i.tech);
            events.push(QuickEvent::Bound(slot, Some(i.id)));
        }
        let active = self.held.and_then(|i| self.slot_of(i.id));
        match self.state {
            ArmsState::None if !self.suspended => {
                if let Some(slot) = self.desired {
                    let item = self
                        .bindings
                        .get(slot)
                        .copied()
                        .flatten()
                        .and_then(|id| grid.iter().find(|i| i.id == id))
                        .copied();
                    if let Some(i) = item {
                        self.held = Some(i);
                        events.push(QuickEvent::Equip(i.id));
                        if i.tool.is_some_and(|t| t.first_use)
                            && !input.vr_animation
                            && (input.debug_first_use || !self.used.contains(&i.tech))
                        {
                            self.used.insert(i.tech);
                            events.push(QuickEvent::FirstUse(i.tech));
                        }
                        if i.tool.is_some_and(|t| t.animated && t.named_animation) {
                            self.start_transition(r, ArmsState::Draw, &mut events);
                        } else {
                            self.state = ArmsState::Hold;
                        }
                    } else {
                        self.desired = None;
                    }
                }
            }
            ArmsState::Draw if active != self.desired => {
                self.start_transition(r, ArmsState::Holster, &mut events)
            }
            ArmsState::Hold if active != self.desired => {
                let bash = self.timing(r).bash;
                let bashing = input
                    .time_bashed
                    .is_some_and(|t| t != 0.0 && input.now < t + f64::from(bash));
                if !input.tool_in_use && !bashing {
                    self.start_transition(r, ArmsState::Holster, &mut events);
                }
            }
            ArmsState::Holster if active == self.desired && self.held.is_some() => {
                if self
                    .held
                    .is_some_and(|i| i.tool.is_some_and(|t| t.animated && t.named_animation))
                {
                    self.start_transition(r, ArmsState::Draw, &mut events);
                } else {
                    self.state = ArmsState::Hold;
                    self.sequence = Transition::default();
                }
            }
            _ => {}
        }
        // Rebinding can leave a held object unbound; mirror Update's teardown.
        if self.held.is_some() && self.held.is_none_or(|i| self.slot_of(i.id).is_none()) {
            events.extend(self.deselect_immediate());
        }
        Ok(events)
    }
    /// Call before a non-held item leaves the grid, so same-type refill knows
    /// its tech type. Held removal is reconciled by update using its snapshot.
    pub fn on_remove(&mut self, id: ItemId) {
        if let Some(slot) = self.slot_of(id)
            && self.held() != Some(id)
        {
            self.bindings[slot] = None;
            self.refill = self.binding_types[slot].take().map(|tech| (slot, tech));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rules() -> QuickRules {
        QuickRules {
            slots: 3,
            hand: 20,
            scrap: 90,
            selectable: 8,
            selectable_chargeable: 9,
            defaults: ToolTiming {
                draw: 0.4,
                holster: 0.2,
                drop: 0.6,
                bash: 0.8,
                animated: true,
                named_animation: true,
                first_use: true,
            },
            transition_max: 7.0,
        }
    }
    fn item(id: u64, tech: i32) -> QuickItem {
        QuickItem {
            id: ItemId(id),
            tech,
            equipment_type: 20,
            slot_type: 8,
            tool: Some(rules().defaults),
            eat: false,
            use_action: false,
            plantable: false,
            live_mixin: false,
        }
    }
    fn tick(q: &mut QuickSlots, grid: &[QuickItem], dt: f32) -> Vec<QuickEvent> {
        q.update(
            &rules(),
            grid,
            QuickUpdate {
                now: q.last_update.unwrap_or(10.0) + f64::from(dt),
                dt,
                ..Default::default()
            },
        )
        .unwrap()
    }
    #[test]
    fn manual_binding_toggle_move_auto_duplicates_and_full_toolbar() {
        let r = rules();
        let mut q = QuickSlots::new(&r).unwrap();
        let grid = [item(1, 1), item(2, 1), item(3, 2), item(4, 3), item(5, 4)];
        assert!(q.on_add(&r, &grid, ItemId(1), true).unwrap().is_empty());
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        assert_eq!(q.bindings()[0], None);
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        q.bind(&r, &grid, 1, ItemId(1)).unwrap();
        assert_eq!(q.bindings(), [None, Some(ItemId(1)), None]);
        q.on_add(&r, &grid, ItemId(3), false).unwrap();
        q.on_add(&r, &grid, ItemId(4), false).unwrap();
        assert!(q.on_add(&r, &grid, ItemId(5), false).unwrap().is_empty());
        let mut edible = item(6, 5);
        edible.tool = None;
        edible.eat = true;
        let mut other = QuickSlots::new(&r).unwrap();
        assert!(
            other
                .on_add(&r, &[edible], edible.id, false)
                .unwrap()
                .is_empty()
        );
        edible.live_mixin = true;
        assert_eq!(
            other.on_add(&r, &[edible], edible.id, true).unwrap().len(),
            1
        );
        assert_eq!(other.desired(), Some(0));
    }
    #[test]
    fn scrolling_deselects_at_end_and_skips_nonselectable() {
        let r = rules();
        let mut q = QuickSlots::new(&r).unwrap();
        let mut grid = [item(1, 1), item(2, 2)];
        grid[0].slot_type = 99;
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        q.bind(&r, &grid, 2, ItemId(2)).unwrap();
        q.scroll(&r, &grid, true, true).unwrap();
        assert_eq!(q.desired(), Some(2));
        q.scroll(&r, &grid, true, true).unwrap();
        assert_eq!(q.desired(), None);
        q.scroll(&r, &grid, false, true).unwrap();
        assert_eq!(q.desired(), Some(2));
        q.scroll(&r, &grid, false, true).unwrap();
        assert_eq!(q.desired(), None);
    }
    #[test]
    fn sequence_callback_is_one_update_late_and_switch_mid_draw_releases_once() {
        let r = rules();
        let mut q = QuickSlots::new(&r).unwrap();
        let grid = [item(1, 1), item(2, 2)];
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        q.bind(&r, &grid, 1, ItemId(2)).unwrap();
        q.select(0);
        let mut events = tick(&mut q, &grid, 0.1);
        assert_eq!(q.state(), ArmsState::Draw);
        events.extend(tick(&mut q, &grid, 0.4));
        assert_eq!(q.state(), ArmsState::Draw);
        q.select(1);
        events.extend(tick(&mut q, &grid, 0.1));
        assert_eq!(q.state(), ArmsState::Holster);
        events.extend(tick(&mut q, &grid, 0.2));
        events.extend(tick(&mut q, &grid, 0.1));
        assert_eq!(q.held(), Some(ItemId(2)));
        assert_eq!(
            events
                .iter()
                .filter(|e| **e == QuickEvent::Equip(ItemId(1)))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| **e == QuickEvent::Unequip(ItemId(1)))
                .count(),
            1
        );
        assert_eq!(q.used_tools().len(), 2);
    }
    #[test]
    fn remove_refill_drop_and_pda_death_keep_identity() {
        let r = rules();
        let mut q = QuickSlots::new(&r).unwrap();
        let grid = [item(1, 1), item(2, 1)];
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        q.select(0);
        tick(&mut q, &grid, 0.1);
        let events = tick(&mut q, &grid[1..], 0.1);
        assert_eq!(q.held(), None);
        assert_eq!(q.state(), ArmsState::Drop);
        assert_eq!(q.bindings()[0], Some(ItemId(2)));
        assert!(events.contains(&QuickEvent::Unequip(ItemId(1))));
        tick(&mut q, &grid[1..], 0.6);
        tick(&mut q, &grid[1..], 0.1);
        assert_eq!(q.held(), Some(ItemId(2)));
        assert!(q.open_pda(true).is_none());
        assert!(
            q.open_pda(false)
                .unwrap()
                .contains(&QuickEvent::Unequip(ItemId(2)))
        );
        tick(&mut q, &grid[1..], 1.0);
        assert_eq!(q.held(), None);
        q.close_pda();
        tick(&mut q, &grid[1..], 0.1);
        assert_eq!(q.held(), Some(ItemId(2)));
        assert_eq!(q.used_tools().len(), 1);
        q.death();
        assert_eq!(q.held(), None);
        assert_eq!(q.desired(), None);
        q.on_remove(ItemId(2));
        tick(&mut q, &[], 0.1);
        assert_eq!(q.bindings(), [None, None, None]);
    }
    #[test]
    fn nonheld_refill_and_same_clock_do_not_leave_dangling_bindings() {
        let r = rules();
        let mut q = QuickSlots::new(&r).unwrap();
        let grid = [item(1, 1), item(2, 1)];
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        tick(&mut q, &grid[1..], 0.1);
        assert_eq!(q.bindings()[0], Some(ItemId(2)));
        q.select(0);
        tick(&mut q, &grid[1..], 0.1);
        let now = q.last_update.unwrap();
        let before = q.sequence;
        q.update(
            &r,
            &grid[1..],
            QuickUpdate {
                now,
                dt: 1.0,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(q.sequence, before);
        // Holster reversal resets Sequence progress, as Set(current,target) does.
        q.select(0);
        tick(&mut q, &grid[1..], 0.1);
        assert_eq!(q.state(), ArmsState::Holster);
        q.select(0);
        tick(&mut q, &grid[1..], 0.1);
        assert_eq!(q.state(), ArmsState::Draw);
        assert_eq!(q.sequence.t, 0.0);
    }

    #[test]
    fn held_use_and_bash_delay_holster_and_invalid_input_is_atomic() {
        let r = rules();
        let mut q = QuickSlots::new(&r).unwrap();
        let grid = [item(1, 1)];
        q.bind(&r, &grid, 0, ItemId(1)).unwrap();
        q.select(0);
        tick(&mut q, &grid, 0.1);
        tick(&mut q, &grid, 0.4);
        tick(&mut q, &grid, 0.1);
        assert_eq!(q.state(), ArmsState::Hold);
        q.select(0);
        q.update(
            &r,
            &grid,
            QuickUpdate {
                now: 10.0,
                dt: 0.1,
                tool_in_use: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(q.state(), ArmsState::Hold);
        q.update(
            &r,
            &grid,
            QuickUpdate {
                now: 10.0,
                dt: 0.1,
                time_bashed: Some(9.5),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(q.state(), ArmsState::Hold);
        let before = q.clone();
        assert!(
            q.update(
                &r,
                &grid,
                QuickUpdate {
                    dt: f32::NAN,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(q, before);
        tick(&mut q, &grid, 0.1);
        assert_eq!(q.state(), ArmsState::Holster);
    }
}
