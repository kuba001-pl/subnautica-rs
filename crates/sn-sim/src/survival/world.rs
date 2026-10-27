//! Headless knife/harvest, cabinet, crate and stored-loot rules (M9s6).
//! Damage resolution and scene-reference activation remain explicit caller
//! boundaries; this module never guesses a target's DamageSystem modifiers.
use super::{RuleError, nonnegative};
use crate::inventory::{
    AnalysisEntry, Container, HitKind, Inventory, Item, ItemId, Outcrop, TECH_NONE, TechType,
    Techs, pickup, spawn_supplies,
};

#[derive(Clone, Debug, PartialEq)]
pub struct WorldSurvivalRules {
    pub animation_guard: f64,
    pub cabinet_door_delay: f64,
    pub cabinet_room: (i32, i32),
    pub first_aid: TechType,
    pub cabinet_count: u32,
    pub damage_alive: i32,
    pub damage_dead: i32,
    pub harvest_count: u32,
}
impl WorldSurvivalRules {
    pub fn validate(&self) -> Result<(), RuleError> {
        if nonnegative(&[self.animation_guard, self.cabinet_door_delay])
            && self.cabinet_room.0 > 0
            && self.cabinet_room.1 > 0
            && self.first_aid != TECH_NONE
            && self.cabinet_count > 0
            && self.damage_alive != self.damage_dead
            && self.harvest_count > 0
        {
            Ok(())
        } else {
            Err(RuleError("invalid world survival rules"))
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolActionKind {
    Knife,
    Bash,
    Hand,
}
/// A move-only event created only by the gate; input itself grants no hit.
#[derive(Debug)]
pub struct ToolAction {
    kind: ToolActionKind,
    sequence: u64,
}
impl ToolAction {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
}
/// Monotone action ids and pending-event replacement are our server policy,
/// not the game's. Reach/occlusion/ownership validation is the caller's job.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolUseGate {
    last_sequence: Option<u64>,
    pending: Option<(u64, ToolActionKind)>,
    last_knife_event: Option<f64>,
}
impl ToolUseGate {
    pub fn begin(&mut self, sequence: u64, kind: ToolActionKind) -> bool {
        if self.last_sequence.is_some_and(|n| sequence <= n) {
            return false;
        }
        self.last_sequence = Some(sequence);
        self.pending = Some((sequence, kind));
        true
    }
    pub fn event(
        &mut self,
        r: &WorldSurvivalRules,
        sequence: u64,
        now: f64,
    ) -> Result<Option<ToolAction>, RuleError> {
        r.validate()?;
        if !now.is_finite() {
            return Err(RuleError("invalid tool event clock"));
        }
        let Some((id, kind)) = self.pending else {
            return Ok(None);
        };
        if sequence != id
            || (kind == ToolActionKind::Knife
                && self
                    .last_knife_event
                    .is_some_and(|t| now <= t + r.animation_guard))
        {
            return Ok(None);
        }
        self.pending = None;
        if kind == ToolActionKind::Knife {
            self.last_knife_event = Some(now);
        }
        Ok(Some(ToolAction { kind, sequence }))
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnifeInfo {
    pub damage: f64,
    pub reach: f64,
    pub damage_type: i32,
}
impl KnifeInfo {
    pub fn validate(&self) -> Result<(), RuleError> {
        if nonnegative(&[self.damage, self.reach]) {
            Ok(())
        } else {
            Err(RuleError("invalid knife fields"))
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct KnifeTarget {
    pub has_live_mixin: bool,
    pub was_alive: bool,
    pub knifeable: bool,
    pub weldable: bool,
    pub escape_pod: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HarvestInfo {
    pub kind: i32,
    pub output: TechType,
    pub final_bonus: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KnifeDamage {
    pub amount: f64,
    pub damage_type: i32,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KnifeStrike {
    pub valid: bool,
    pub damage: Option<KnifeDamage>,
    pub harvest: Option<(TechType, u32)>,
}
/// The resolver receives one raw prefab damage request and returns actual
/// post-damage liveness. It owns shields, modifiers and death lifecycle.
pub fn knife_strike(
    action: ToolAction,
    r: &WorldSurvivalRules,
    knife: KnifeInfo,
    target: KnifeTarget,
    harvest: HarvestInfo,
    apply_damage: impl FnOnce(KnifeDamage) -> bool,
) -> Result<KnifeStrike, RuleError> {
    r.validate()?;
    knife.validate()?;
    if action.kind != ToolActionKind::Knife {
        return Err(RuleError("not a knife event"));
    }
    let final_count = r
        .harvest_count
        .checked_add(harvest.final_bonus)
        .ok_or(RuleError("harvest count overflow"))?;
    if target.has_live_mixin && (target.weldable || !target.knifeable || target.escape_pod) {
        return Ok(KnifeStrike::default());
    }
    let mut strike = KnifeStrike {
        valid: true,
        ..Default::default()
    };
    if !target.has_live_mixin {
        return Ok(strike);
    }
    let damage = KnifeDamage {
        amount: knife.damage,
        damage_type: knife.damage_type,
    };
    let alive = apply_damage(damage);
    strike.damage = Some(damage);
    if harvest.output != TECH_NONE {
        if harvest.kind == r.damage_alive && target.was_alive {
            strike.harvest = Some((
                harvest.output,
                if alive { r.harvest_count } else { final_count },
            ));
        } else if harvest.kind == r.damage_dead && !alive {
            strike.harvest = Some((harvest.output, r.harvest_count));
        }
    }
    Ok(strike)
}
/// Failed output additions are returned for disposal, never spawned loose.
/// Item instantiation/unique id allocation belongs to the caller.
pub fn grant_harvest(container: &mut Container, techs: &Techs, items: Vec<Item>) -> Vec<Item> {
    spawn_supplies(container, techs, items)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcropClick {
    Refused,
    AwaitBash,
    HandHit(bool),
}
pub fn outcrop_click(
    gate: &mut ToolUseGate,
    r: &WorldSurvivalRules,
    outcrop: &mut Outcrop,
    sequence: u64,
    now: f64,
    has_bash: bool,
) -> Result<OutcropClick, RuleError> {
    r.validate()?;
    if !now.is_finite() {
        return Err(RuleError("invalid outcrop clock"));
    }
    if !gate.begin(
        sequence,
        if has_bash {
            ToolActionKind::Bash
        } else {
            ToolActionKind::Hand
        },
    ) {
        return Ok(OutcropClick::Refused);
    }
    if has_bash {
        return Ok(OutcropClick::AwaitBash);
    }
    let Some(_action) = gate.event(r, sequence, now)? else {
        return Ok(OutcropClick::Refused);
    };
    Ok(OutcropClick::HandHit(outcrop.hit(HitKind::Hand)))
}
pub fn bash_outcrop(action: ToolAction, outcrop: &mut Outcrop) -> Result<bool, RuleError> {
    if action.kind != ToolActionKind::Bash {
        return Err(RuleError("not a bash event"));
    }
    Ok(outcrop.hit(HitKind::Bash))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CabinetInfo {
    pub interval: f64,
    pub start_with_kit: bool,
}
impl CabinetInfo {
    pub fn validate(&self) -> Result<(), RuleError> {
        if nonnegative(&[self.interval]) {
            Ok(())
        } else {
            Err(RuleError("invalid cabinet interval"))
        }
    }
}
/// `MedicalCabinet.timeSpawnMedKit` is a float: the game adds the interval
/// to `(float)timePassed` in single precision, then compares it with the
/// double clock strictly. Kept here so late-game deadlines round as the game's.
fn f32_deadline(now: f64, delay: f64) -> f64 {
    f64::from(now as f32 + delay as f32)
}
/// `game` is `DayNightCycle.timePassed` (kit deadline); `real` is Unity's
/// scaled `Time.time`, which drives `Invoke("ToggleDoorState", 2f)`.
#[derive(Clone, Copy, Debug)]
pub struct CabinetClock {
    pub game: f64,
    pub real: f64,
}
impl CabinetClock {
    fn validate(self) -> Result<(), RuleError> {
        if self.game.is_finite() && self.real.is_finite() {
            Ok(())
        } else {
            Err(RuleError("invalid cabinet clock"))
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Cabinet {
    pub has_kit: bool,
    pub spawn_at: f64,
    pub door_open: bool,
    toggle_at: Vec<f64>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CabinetClick {
    Nothing,
    DoorToggled,
    TakeKit { tech: TechType, count: u32 },
}
impl Cabinet {
    pub fn new(info: CabinetInfo, now: f64) -> Result<Self, RuleError> {
        info.validate()?;
        let spawn_at = f32_deadline(
            now,
            if info.start_with_kit {
                0.0
            } else {
                info.interval
            },
        );
        if !now.is_finite() || !spawn_at.is_finite() {
            return Err(RuleError("invalid cabinet start"));
        }
        Ok(Self {
            has_kit: false,
            spawn_at,
            door_open: false,
            toggle_at: Vec::new(),
        })
    }
    /// Saved has_kit/deadline are authoritative. Runtime-only door timers start
    /// empty on restore; extending the save schema is M9s10, not this module.
    pub fn restored(has_kit: bool, spawn_at: f64) -> Result<Self, RuleError> {
        if !spawn_at.is_finite() {
            return Err(RuleError("invalid cabinet deadline"));
        }
        Ok(Self {
            has_kit,
            spawn_at,
            door_open: false,
            toggle_at: Vec::new(),
        })
    }
    pub fn advance(&mut self, clock: CabinetClock) -> Result<usize, RuleError> {
        clock.validate()?;
        if !self.spawn_at.is_finite() {
            return Err(RuleError("invalid cabinet deadline"));
        }
        if !self.has_kit && clock.game > self.spawn_at {
            self.has_kit = true;
        }
        let due = self.toggle_at.iter().filter(|t| clock.real >= **t).count();
        self.toggle_at.retain(|t| clock.real < *t);
        if due % 2 != 0 {
            self.door_open = !self.door_open;
        }
        Ok(due)
    }
    pub fn click(
        &mut self,
        r: &WorldSurvivalRules,
        info: CabinetInfo,
        clock: CabinetClock,
        constructed: bool,
        room_for_kit: bool,
    ) -> Result<CabinetClick, RuleError> {
        r.validate()?;
        info.validate()?;
        clock.validate()?;
        let next = f32_deadline(clock.game, info.interval);
        let toggle = clock.real + r.cabinet_door_delay;
        if !next.is_finite() || !toggle.is_finite() {
            return Err(RuleError("cabinet timer overflow"));
        }
        if !constructed || !self.has_kit {
            return Ok(CabinetClick::Nothing);
        }
        if self.door_open && room_for_kit {
            self.has_kit = false;
            self.spawn_at = next;
            self.toggle_at.push(toggle);
            Ok(CabinetClick::TakeKit {
                tech: r.first_aid,
                count: r.cabinet_count,
            })
        } else {
            self.door_open = !self.door_open;
            Ok(CabinetClick::DoorToggled)
        }
    }
    pub fn progress(&self, info: CabinetInfo, now: f64) -> Result<f64, RuleError> {
        info.validate()?;
        if !now.is_finite() || !self.spawn_at.is_finite() {
            return Err(RuleError("invalid cabinet progress"));
        }
        Ok(if info.interval == 0.0 {
            0.0
        } else {
            ((now - (self.spawn_at - info.interval)) / info.interval).clamp(0.0, 1.0)
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SupplyCrate {
    pub open: bool,
    pub sealed: bool,
    pub has_animation: bool,
    pub activate_child_on_open: bool,
    pub child_active: bool,
    pub child: Option<Item>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CrateClick {
    Sealed,
    NoAnimation,
    Opened,
    Empty,
    Full,
    Taken {
        item: ItemId,
        unlocked: Vec<TechType>,
    },
}
impl SupplyCrate {
    fn toggle(&mut self) -> bool {
        if !self.has_animation {
            return false;
        }
        self.open = !self.open;
        if self.open && self.activate_child_on_open {
            self.child_active = true;
        }
        true
    }
    pub fn sealed_opened(&mut self) -> bool {
        self.sealed = false;
        self.toggle()
    }
    pub fn click(
        &mut self,
        inv: &mut Inventory,
        techs: &Techs,
        analysis: &[AnalysisEntry],
        in_sub: bool,
    ) -> CrateClick {
        if self.sealed {
            return CrateClick::Sealed;
        }
        if !self.open {
            return if self.toggle() {
                CrateClick::Opened
            } else {
                CrateClick::NoAnimation
            };
        }
        if !self.child_active {
            return CrateClick::Empty;
        }
        let Some(item) = self.child.take() else {
            return CrateClick::Empty;
        };
        let id = item.id;
        match pickup(inv, techs, analysis, item, in_sub) {
            Ok((_, unlocked)) => CrateClick::Taken { item: id, unlocked },
            Err(item) => {
                self.child = Some(item);
                CrateClick::Full
            }
        }
    }
}

/// SplitMix64 is our deterministic seed policy, not Unity Random's sequence.
struct LootRng(u64);
impl LootRng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u32 << 24) as f32
    }
}
/// UWE.Utils.Sample is rejection sampling, not curve.Evaluate(random).
/// 4096 attempts is our termination limit, not the game's; failure is explicit.
pub fn draw_stored_loot(
    min: i32,
    max: i32,
    supply: &[TechType],
    seed: u64,
    mut density: impl FnMut(f32) -> f32,
) -> Result<Vec<TechType>, RuleError> {
    if min < 0 || max < min {
        return Err(RuleError("invalid stored-loot bounds"));
    }
    let mut rng = LootRng(seed);
    let mut sample = None;
    for _ in 0..4096 {
        let x = rng.unit();
        let y = rng.unit();
        let d = density(x);
        if !d.is_finite() {
            return Err(RuleError("invalid stored-loot density"));
        }
        if y < d {
            sample = Some(x);
            break;
        }
    }
    let x = sample.ok_or(RuleError("stored-loot rejection limit reached"))?;
    let count =
        ((min as f32 + x * (max - min) as f32).round_ties_even() as usize).min(supply.len());
    let mut indices: Vec<usize> = (0..supply.len()).collect();
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        let at = (rng.next() % (indices.len() as u64)) as usize;
        let i = indices.swap_remove(at);
        result.push(supply[i]);
    }
    Ok(result)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoredLoot {
    pub spawned: bool,
}
impl StoredLoot {
    /// Every selected item is attempted, including small items after a large
    /// one fails. Failed instances are returned for disposal; never loose loot.
    pub fn populate(
        &mut self,
        container: &mut Container,
        techs: &Techs,
        items: Vec<Item>,
    ) -> Vec<Item> {
        if self.spawned {
            return items;
        }
        let rejected = spawn_supplies(container, techs, items);
        self.spawned = true;
        rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rules() -> WorldSurvivalRules {
        WorldSurvivalRules {
            animation_guard: 0.5,
            cabinet_door_delay: 2.0,
            cabinet_room: (1, 1),
            first_aid: 80,
            cabinet_count: 1,
            damage_alive: 7,
            damage_dead: 8,
            harvest_count: 1,
        }
    }
    fn action(kind: ToolActionKind) -> ToolAction {
        let mut gate = ToolUseGate::default();
        gate.begin(1, kind);
        gate.event(&rules(), 1, 0.0).unwrap().unwrap()
    }
    fn target(alive: bool) -> KnifeTarget {
        KnifeTarget {
            has_live_mixin: true,
            was_alive: alive,
            knifeable: true,
            weldable: false,
            escape_pod: false,
        }
    }
    fn knife() -> KnifeInfo {
        KnifeInfo {
            damage: 6.0,
            reach: 1.7,
            damage_type: 20,
        }
    }
    #[test]
    fn harvest_uses_before_and_after_liveness_and_final_bonus() {
        for (kind, before, after, count) in [
            (7, true, true, Some(1)),
            (7, true, false, Some(4)),
            (7, false, false, None),
            (8, true, true, None),
            (8, true, false, Some(1)),
            (8, false, false, Some(1)),
            (0, true, false, None),
        ] {
            let mut calls = 0;
            let strike = knife_strike(
                action(ToolActionKind::Knife),
                &rules(),
                knife(),
                target(before),
                HarvestInfo {
                    kind,
                    output: 90,
                    final_bonus: 3,
                },
                |d| {
                    assert_eq!(d.amount, 6.0);
                    calls += 1;
                    after
                },
            )
            .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(strike.harvest, count.map(|n| (90, n)));
        }
        let strike = knife_strike(
            action(ToolActionKind::Knife),
            &rules(),
            knife(),
            target(true),
            HarvestInfo {
                kind: 7,
                output: TECH_NONE,
                final_bonus: 3,
            },
            |_| false,
        )
        .unwrap();
        assert_eq!(strike.harvest, None);
    }
    #[test]
    fn invalid_and_nonlive_targets_never_resolve_damage_or_harvest() {
        for t in [
            KnifeTarget {
                weldable: true,
                ..target(true)
            },
            KnifeTarget {
                knifeable: false,
                ..target(true)
            },
            KnifeTarget {
                escape_pod: true,
                ..target(true)
            },
            KnifeTarget {
                has_live_mixin: false,
                ..target(true)
            },
        ] {
            let strike = knife_strike(
                action(ToolActionKind::Knife),
                &rules(),
                knife(),
                t,
                HarvestInfo {
                    kind: 7,
                    output: 90,
                    final_bonus: 3,
                },
                |_| panic!("invalid target damage"),
            )
            .unwrap();
            assert_eq!(strike.damage, None);
            assert_eq!(strike.harvest, None);
        }
        let mut grid = Container::new(1, 1);
        grid.add(&Techs::default(), Item::new(1, 3)).unwrap();
        let rejected = grant_harvest(&mut grid, &Techs::default(), vec![Item::new(2, 90)]);
        assert_eq!(rejected.len(), 1);
        assert_eq!(grid.len(), 1);
    }
    #[test]
    fn input_and_animation_cannot_double_hit_and_guard_is_strict() {
        let r = rules();
        let mut gate = ToolUseGate::default();
        let mut outcrop = Outcrop::new(3);
        assert_eq!(
            outcrop_click(&mut gate, &r, &mut outcrop, 1, 0.0, true).unwrap(),
            OutcropClick::AwaitBash
        );
        assert!(!outcrop.broken);
        assert_eq!(outcrop.hits_left, 3);
        assert!(bash_outcrop(gate.event(&r, 1, 0.0).unwrap().unwrap(), &mut outcrop).unwrap());
        assert!(gate.event(&r, 1, 1.0).unwrap().is_none());
        assert_eq!(
            outcrop_click(&mut gate, &r, &mut outcrop, 1, 1.0, false).unwrap(),
            OutcropClick::Refused
        );
        assert!(gate.begin(2, ToolActionKind::Knife));
        assert!(gate.event(&r, 2, 1.0).unwrap().is_some());
        gate.begin(3, ToolActionKind::Knife);
        assert!(gate.event(&r, 3, 1.5).unwrap().is_none());
        assert!(gate.event(&r, 3, 1.50001).unwrap().is_some());
        assert!(gate.event(&r, 3, 2.1).unwrap().is_none());
    }
    #[test]
    fn cabinet_deadline_full_grid_and_two_distinct_clocks() {
        let r = rules();
        let info = CabinetInfo {
            interval: 10.0,
            start_with_kit: true,
        };
        let mut c = Cabinet::new(info, 100.0).unwrap();
        let clock = |game, real| CabinetClock { game, real };
        c.advance(clock(100.0, 1.0)).unwrap();
        assert!(!c.has_kit);
        c.advance(clock(100.01, 1.0)).unwrap();
        assert!(c.has_kit);
        assert_eq!(
            c.click(&r, info, clock(101.0, 1.0), false, true).unwrap(),
            CabinetClick::Nothing
        );
        assert_eq!(
            c.click(&r, info, clock(101.0, 1.0), true, true).unwrap(),
            CabinetClick::DoorToggled
        );
        assert_eq!(
            c.click(&r, info, clock(101.0, 1.0), true, false).unwrap(),
            CabinetClick::DoorToggled
        );
        assert!(c.has_kit);
        assert!(!c.door_open);
        c.click(&r, info, clock(101.0, 1.0), true, true).unwrap();
        assert_eq!(
            c.click(&r, info, clock(101.0, 1.0), true, true).unwrap(),
            CabinetClick::TakeKit { tech: 80, count: 1 }
        );
        assert!(!c.has_kit);
        assert_eq!(
            c.click(&r, info, clock(101.0, 1.0), true, true).unwrap(),
            CabinetClick::Nothing
        );
        c.advance(clock(111.0, 2.99)).unwrap();
        assert!(!c.has_kit);
        assert!(c.door_open);
        c.advance(clock(111.01, 3.0)).unwrap();
        assert!(c.has_kit);
        assert!(!c.door_open);
        assert_eq!(Cabinet::restored(true, 500.0).unwrap().spawn_at, 500.0);
        // Single-precision deadline: at 2^24 s, 600.3 rounds to an even float.
        let late = Cabinet::new(
            CabinetInfo {
                interval: 600.3,
                start_with_kit: false,
            },
            16_777_216.0,
        )
        .unwrap();
        assert_eq!(late.spawn_at, 16_777_816.0);
    }
    fn inventory() -> Inventory {
        let table = crate::inventory::SlotTable {
            slots: vec![],
            vehicle_module: 1,
            seamoth_module: 2,
            exosuit_module: 3,
        };
        Inventory::new(
            (1, 1),
            &table,
            &[] as &[&str],
            crate::inventory::KnownTech::default(),
        )
    }
    #[test]
    fn crate_sealed_closed_full_empty_and_absent_animation() {
        let mut inv = inventory();
        let t = Techs::default();
        let mut c = SupplyCrate {
            open: false,
            sealed: true,
            has_animation: false,
            activate_child_on_open: true,
            child_active: false,
            child: Some(Item::new(1, 4)),
        };
        assert_eq!(c.click(&mut inv, &t, &[], false), CrateClick::Sealed);
        assert!(!c.sealed_opened());
        assert!(!c.open);
        assert_eq!(c.click(&mut inv, &t, &[], false), CrateClick::NoAnimation);
        c.has_animation = true;
        assert_eq!(c.click(&mut inv, &t, &[], false), CrateClick::Opened);
        assert!(c.child_active);
        inv.container.add(&t, Item::new(2, 5)).unwrap();
        assert_eq!(c.click(&mut inv, &t, &[], false), CrateClick::Full);
        assert_eq!(c.child.as_ref().unwrap().id, ItemId(1));
        inv.container.remove(ItemId(2));
        assert!(matches!(
            c.click(&mut inv, &t, &[], false),
            CrateClick::Taken {
                item: ItemId(1),
                ..
            }
        ));
        assert_eq!(c.click(&mut inv, &t, &[], false), CrateClick::Empty);
        assert!(c.open);
    }
    #[test]
    fn seeded_rejection_sample_storage_indices_and_once_flag() {
        let supplies = [20, 21, 22, 23, 24];
        let a = draw_stored_loot(2, 4, &supplies, 123, |x| x).unwrap();
        assert_eq!(a, draw_stored_loot(2, 4, &supplies, 123, |x| x).unwrap());
        assert!((2..=4).contains(&a.len()));
        let mut unique = a.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), a.len());
        assert_eq!(
            draw_stored_loot(20, 20, &supplies, 123, |_| 1.0)
                .unwrap()
                .len(),
            supplies.len()
        );
        assert!(draw_stored_loot(1, 2, &supplies, 123, |_| -1.0).is_err());
        assert!(draw_stored_loot(1, 2, &supplies, 123, |_| f32::NAN).is_err());
        let mut storage = Container::new(2, 1);
        let mut t = Techs::default();
        t.sizes.insert(50, (3, 1));
        let mut state = StoredLoot::default();
        let rejected = state.populate(&mut storage, &t, vec![Item::new(1, 50), Item::new(2, 20)]);
        assert_eq!(rejected.len(), 1);
        assert_eq!(storage.len(), 1);
        assert!(state.spawned);
        assert_eq!(
            state
                .populate(&mut storage, &t, vec![Item::new(3, 21)])
                .len(),
            1
        );
        assert_eq!(storage.len(), 1);
    }
}
