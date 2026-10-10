//! The `Animator`: a controller's state machines, layers and parameters
//! run over time, producing one value per animated property
//! (`docs/formats/animation.md` § Runtime). Unity's Mecanim is not
//! published; each rule below is from Unity's manual and our reading of
//! the data, marked **hypothesis** where the manual does not settle it.

use std::collections::HashMap;
use std::sync::Arc;

use sn_unity::{
    ATTR_EULER, ATTR_POSITION, ATTR_ROTATION, ATTR_SCALE, AnimationClip, AnimatorController,
    BIND_TRANSFORM, ConditionMode, Interruption, LayerBlending, ParamKind, SELECTOR_BASE,
    StateMachine, Transition,
};

use crate::blend::{self, Leaf};
use crate::math::{self, Quat};
use crate::sample::{length, sample};

/// What an animated property is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SlotKind {
    /// A Transform's local position (3 values).
    Position,
    /// A Transform's local rotation (quaternion, 4 values); Euler-angle
    /// curves write here too.
    Rotation,
    /// A Transform's local scale (3 values).
    Scale,
    /// Any other property (1 value): blend shape weight, light value, …
    /// named by the binding's class id and attribute hash.
    Float {
        type_id: i32,
        attribute: u32,
        custom_type: u8,
    },
}

impl SlotKind {
    pub fn width(self) -> usize {
        match self {
            SlotKind::Position | SlotKind::Scale => 3,
            SlotKind::Rotation => 4,
            SlotKind::Float { .. } => 1,
        }
    }

    pub fn is_transform(self) -> bool {
        !matches!(self, SlotKind::Float { .. })
    }
}

/// One animated property: the object (path hash below the animator) and
/// what of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub path: u32,
    pub kind: SlotKind,
    /// Index of its first value in [`Animator::pose`].
    pub offset: usize,
}

#[derive(Clone, Copy, Debug)]
enum Conv {
    Vec3,
    Quat,
    Euler,
    Float,
}

#[derive(Clone, Copy, Debug)]
struct Write {
    slot: usize,
    curve: usize,
    conv: Conv,
}

struct ClipProgram {
    clip: Arc<AnimationClip>,
    writes: Vec<Write>,
    length: f32,
    /// The clip's values at its start (the reference of additive layers,
    /// **hypothesis**: Unity's default "additive reference pose" is the
    /// clip's first frame), per pose value.
    reference: Vec<f32>,
}

struct LayerProgram {
    machine: usize,
    motion_set: usize,
    blending: LayerBlending,
    default_weight: f32,
    /// Per slot: how much this layer may change it (0: masked out).
    mask: Vec<f32>,
    /// Slots some state of this layer animates (with a mask above 0).
    slots: Vec<usize>,
}

/// A parameter's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Trigger(bool),
}

impl ParamValue {
    fn as_f32(self) -> f32 {
        match self {
            ParamValue::Float(v) => v,
            ParamValue::Int(v) => v as f32,
            ParamValue::Bool(v) | ParamValue::Trigger(v) => f32::from(u8::from(v)),
        }
    }

    fn as_bool(self) -> bool {
        match self {
            ParamValue::Bool(v) | ParamValue::Trigger(v) => v,
            ParamValue::Float(v) => v != 0.0,
            ParamValue::Int(v) => v != 0,
        }
    }
}

/// A controller compiled against its clips: shared by every animator that
/// uses it.
pub struct Program {
    pub controller: Arc<AnimatorController>,
    pub slots: Vec<Slot>,
    /// Values in a pose.
    pub width: usize,
    clips: Vec<Option<ClipProgram>>,
    layers: Vec<LayerProgram>,
    params: HashMap<u32, usize>,
    param_defaults: Vec<ParamValue>,
    /// Clips without a loaded clip (null reference or failed to load).
    pub missing_clips: usize,
}

fn slot_kind(type_id: i32, attribute: u32, custom_type: u8) -> (SlotKind, Conv) {
    if type_id == BIND_TRANSFORM {
        match attribute {
            ATTR_POSITION => return (SlotKind::Position, Conv::Vec3),
            ATTR_ROTATION => return (SlotKind::Rotation, Conv::Quat),
            ATTR_SCALE => return (SlotKind::Scale, Conv::Vec3),
            ATTR_EULER => return (SlotKind::Rotation, Conv::Euler),
            _ => {}
        }
    }
    (
        SlotKind::Float {
            type_id,
            attribute,
            custom_type,
        },
        Conv::Float,
    )
}

impl Program {
    /// Compiles `controller` with its clips (`clips[i]` is clip `i` of the
    /// controller).
    pub fn new(
        controller: Arc<AnimatorController>,
        clips: &[Option<Arc<AnimationClip>>],
    ) -> Program {
        let mut slots: Vec<Slot> = Vec::new();
        let mut by_key: HashMap<(u32, SlotKind), usize> = HashMap::new();
        let mut width = 0;
        let mut programs = Vec::with_capacity(controller.clips.len());
        let mut missing_clips = 0;
        for i in 0..controller.clips.len() {
            let Some(clip) = clips.get(i).cloned().flatten() else {
                missing_clips += 1;
                programs.push(None);
                continue;
            };
            let mut writes = Vec::new();
            let mut curve = 0;
            for b in &clip.bindings {
                let n = b.curve_count();
                if b.is_pptr_curve {
                    curve += n;
                    continue;
                }
                let (kind, conv) = slot_kind(b.type_id, b.attribute, b.custom_type);
                let slot = *by_key.entry((b.path, kind)).or_insert_with(|| {
                    slots.push(Slot {
                        path: b.path,
                        kind,
                        offset: width,
                    });
                    width += kind.width();
                    slots.len() - 1
                });
                writes.push(Write { slot, curve, conv });
                curve += n;
            }
            programs.push(Some(ClipProgram {
                length: length(&clip),
                writes,
                reference: Vec::new(),
                clip,
            }));
        }
        // Additive references, now that the pose's width is known.
        let mut buf = Vec::new();
        for p in programs.iter_mut().flatten() {
            let mut reference = vec![0.0; width];
            sample(&p.clip, p.clip.start_time, &mut buf);
            for w in &p.writes {
                write_value(&slots, w, &buf, &mut reference);
            }
            p.reference = reference;
        }
        let mut layers = Vec::new();
        for l in &controller.layers {
            let machine = l.state_machine as usize;
            let mask: Vec<f32> = if l.skeleton_mask.is_empty() {
                vec![1.0; slots.len()]
            } else {
                let by_path: HashMap<u32, f32> = l.skeleton_mask.iter().copied().collect();
                // A transform outside the mask is not this layer's
                // (**hypothesis**: other properties are never masked).
                slots
                    .iter()
                    .map(|s| {
                        if s.kind.is_transform() {
                            by_path.get(&s.path).copied().unwrap_or(0.0)
                        } else {
                            1.0
                        }
                    })
                    .collect()
            };
            let motion_set = l.synchronized_layer as usize;
            let mut used = vec![false; slots.len()];
            if let Some(sm) = controller.state_machines.get(machine) {
                for s in &sm.states {
                    let tree = s
                        .blend_tree_index
                        .get(motion_set)
                        .or_else(|| s.blend_tree_index.first())
                        .and_then(|&i| usize::try_from(i).ok())
                        .and_then(|i| s.blend_trees.get(i));
                    for n in tree.into_iter().flatten().filter(|n| n.is_leaf()) {
                        if let Some(Some(p)) = programs.get(n.clip as usize) {
                            for w in &p.writes {
                                used[w.slot] = true;
                            }
                        }
                    }
                }
            }
            let layer_slots = (0..slots.len())
                .filter(|&s| used[s] && mask[s] > 0.0)
                .collect();
            layers.push(LayerProgram {
                machine,
                motion_set,
                blending: l.blending,
                default_weight: l.default_weight,
                mask,
                slots: layer_slots,
            });
        }
        let mut params = HashMap::new();
        let mut param_defaults = Vec::new();
        for p in &controller.params {
            let i = p.index as usize;
            let d = &controller.defaults;
            let v = match p.kind {
                ParamKind::Float => ParamValue::Float(d.floats.get(i).copied().unwrap_or(0.0)),
                ParamKind::Int => ParamValue::Int(d.ints.get(i).copied().unwrap_or(0)),
                ParamKind::Bool => ParamValue::Bool(d.bools.get(i).copied().unwrap_or(false)),
                ParamKind::Trigger => ParamValue::Trigger(d.bools.get(i).copied().unwrap_or(false)),
                ParamKind::Other(_) => continue,
            };
            params.insert(p.id, param_defaults.len());
            param_defaults.push(v);
        }
        Program {
            controller,
            slots,
            width,
            clips: programs,
            layers,
            params,
            param_defaults,
            missing_clips,
        }
    }

    /// The slot of a property, if some clip animates it.
    pub fn slot(&self, path: u32, kind: SlotKind) -> Option<usize> {
        self.slots
            .iter()
            .position(|s| s.path == path && s.kind == kind)
    }
}

/// Binding `w`'s value from the sampled curves `curves` (the first
/// `width` values are used).
fn value_of(w: &Write, curves: &[f32]) -> [f32; 4] {
    let c = |i: usize| curves.get(w.curve + i).copied().unwrap_or(0.0);
    match w.conv {
        Conv::Vec3 => [c(0), c(1), c(2), 0.0],
        Conv::Quat => math::normalize([c(0), c(1), c(2), c(3)]),
        Conv::Euler => math::euler([c(0), c(1), c(2)]),
        Conv::Float => [c(0), 0.0, 0.0, 0.0],
    }
}

/// Writes binding `w`'s value into `out` (one pose).
fn write_value(slots: &[Slot], w: &Write, curves: &[f32], out: &mut [f32]) {
    let slot = slots[w.slot];
    let width = slot.kind.width();
    out[slot.offset..slot.offset + width].copy_from_slice(&value_of(w, curves)[..width]);
}

/// A state being played: its index and normalised time (in loops; 1.5 is
/// halfway through the second loop), before and after the last update.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Playing {
    state: usize,
    time: f32,
    prev: f32,
}

impl Playing {
    fn enter(state: usize, at: f32) -> Playing {
        // Just before `at`, so an exit time of `at` is crossed by the
        // first update.
        Playing {
            state,
            time: at,
            prev: at - 1e-6,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TransitionRef {
    State(usize, usize),
    Any(usize),
}

#[derive(Clone, Debug)]
struct Transit {
    next: Playing,
    elapsed: f32,
    duration: f32,
    via: TransitionRef,
}

#[derive(Clone, Debug)]
enum Current {
    State(Playing),
    /// The layer's values when a transition was interrupted: the new
    /// transition starts from this still pose (layer slots, full pose
    /// width).
    Frozen(Vec<f32>),
}

#[derive(Clone, Debug)]
struct LayerRun {
    current: Current,
    transit: Option<Transit>,
    weight: f32,
}

/// Where a layer is, for logs and scripts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerInfo {
    /// The current state's name hash (`None` while blending from an
    /// interrupted transition).
    pub state: Option<u32>,
    pub full_path: Option<u32>,
    pub normalized_time: f32,
    /// The state being blended to and the blend's progress (0..1).
    pub next: Option<(u32, f32)>,
}

/// One animator: parameters, each layer's state, the pose.
pub struct Animator {
    program: Arc<Program>,
    params: Vec<ParamValue>,
    layers: Vec<LayerRun>,
    defaults: Vec<f32>,
    pose: Vec<f32>,
    // Scratch.
    curves: Vec<f32>,
    below: Vec<f32>,
    acc: Vec<f32>,
    acc_w: Vec<f32>,
    state_w: Vec<f32>,
    /// Transitions started in the last update, per layer (for logs).
    pub started: Vec<(usize, u32, u32)>,
}

fn add_quat(acc: &mut [f32], q: Quat, w: f32, align: Quat) {
    let s = if math::dot(q, align) < 0.0 { -w } else { w };
    for i in 0..4 {
        acc[i] += q[i] * s;
    }
}

impl Animator {
    /// A new animator in each layer's default state, with the program's
    /// default parameters. `defaults` holds every property's value before
    /// animation (`Program::width` values; e.g. the Transforms' stored
    /// local placement): states with write defaults put these back.
    pub fn new(program: Arc<Program>, defaults: Vec<f32>) -> Animator {
        let mut defaults = defaults;
        defaults.resize(program.width, 0.0);
        let layers = program
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let state = program
                    .controller
                    .state_machines
                    .get(l.machine)
                    .map_or(0, |sm| sm.default_state as usize);
                LayerRun {
                    current: Current::State(Playing::enter(state, 0.0)),
                    transit: None,
                    // The first layer always weighs 1 (Unity ignores its
                    // weight).
                    weight: if i == 0 { 1.0 } else { l.default_weight },
                }
            })
            .collect();
        let width = program.width;
        Animator {
            params: program.param_defaults.clone(),
            layers,
            pose: defaults.clone(),
            defaults,
            curves: Vec::new(),
            below: vec![0.0; width],
            acc: vec![0.0; width],
            acc_w: vec![0.0; program.slots.len()],
            state_w: vec![0.0; program.slots.len()],
            started: Vec::new(),
            program,
        }
    }

    pub fn program(&self) -> &Arc<Program> {
        &self.program
    }

    /// Every property's current value (slot `s` at `slots[s].offset`).
    pub fn pose(&self) -> &[f32] {
        &self.pose
    }

    pub fn defaults(&self) -> &[f32] {
        &self.defaults
    }

    fn param(&self, id: u32) -> Option<ParamValue> {
        self.program.params.get(&id).map(|&i| self.params[i])
    }

    fn param_f32(&self, id: u32) -> f32 {
        self.param(id).map_or(0.0, ParamValue::as_f32)
    }

    fn set(&mut self, id: u32, f: impl FnOnce(ParamValue) -> ParamValue) -> bool {
        match self.program.params.get(&id) {
            Some(&i) => {
                self.params[i] = f(self.params[i]);
                true
            }
            None => false,
        }
    }

    /// `Animator.SetFloat`; false if there is no such parameter.
    pub fn set_float(&mut self, id: u32, v: f32) -> bool {
        self.set(id, |p| match p {
            ParamValue::Float(_) => ParamValue::Float(v),
            ParamValue::Int(_) => ParamValue::Int(v as i32),
            other => other,
        })
    }

    pub fn set_int(&mut self, id: u32, v: i32) -> bool {
        self.set(id, |p| match p {
            ParamValue::Int(_) => ParamValue::Int(v),
            ParamValue::Float(_) => ParamValue::Float(v as f32),
            other => other,
        })
    }

    pub fn set_bool(&mut self, id: u32, v: bool) -> bool {
        self.set(id, |p| match p {
            ParamValue::Bool(_) => ParamValue::Bool(v),
            ParamValue::Trigger(_) => ParamValue::Trigger(v),
            other => other,
        })
    }

    /// `Animator.SetTrigger`: stays set until a transition uses it.
    pub fn set_trigger(&mut self, id: u32) -> bool {
        self.set_bool(id, true)
    }

    pub fn get(&self, id: u32) -> Option<ParamValue> {
        self.param(id)
    }

    pub fn set_layer_weight(&mut self, layer: usize, weight: f32) {
        if layer > 0
            && let Some(l) = self.layers.get_mut(layer)
        {
            l.weight = weight;
        }
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// `Animator.Play(state, layer, normalizedTime)`: jumps to the state
    /// (by name hash or full path hash) without a blend. False if the
    /// layer has no such state.
    pub fn play(&mut self, layer: usize, state: u32, normalized_time: f32) -> bool {
        let Some(sm) = self.machine(layer) else {
            return false;
        };
        let Some(i) = sm
            .states
            .iter()
            .position(|s| s.name_id == state || s.full_path_id == state)
        else {
            return false;
        };
        let l = &mut self.layers[layer];
        l.current = Current::State(Playing::enter(i, normalized_time));
        l.transit = None;
        true
    }

    fn machine(&self, layer: usize) -> Option<&StateMachine> {
        let l = self.program.layers.get(layer)?;
        self.program.controller.state_machines.get(l.machine)
    }

    pub fn layer_info(&self, layer: usize) -> Option<LayerInfo> {
        let sm = self.machine(layer)?;
        let l = self.layers.get(layer)?;
        let (state, full_path, normalized_time) = match &l.current {
            Current::State(p) => {
                let s = sm.states.get(p.state)?;
                (Some(s.name_id), Some(s.full_path_id), p.time)
            }
            Current::Frozen(_) => (None, None, 0.0),
        };
        let next = l.transit.as_ref().and_then(|t| {
            let s = sm.states.get(t.next.state)?;
            Some((s.name_id, blend_fraction(t)))
        });
        Some(LayerInfo {
            state,
            full_path,
            normalized_time,
            next,
        })
    }

    /// The leaves a state plays now.
    fn leaves(&self, layer: usize, state: usize) -> Vec<Leaf> {
        let Some(sm) = self.machine(layer) else {
            return Vec::new();
        };
        let Some(s) = sm.states.get(state) else {
            return Vec::new();
        };
        let set = self.program.layers[layer].motion_set;
        let Some(tree) = s
            .blend_tree_index
            .get(set)
            .or_else(|| s.blend_tree_index.first())
            .and_then(|&i| usize::try_from(i).ok())
            .and_then(|i| s.blend_trees.get(i))
        else {
            return Vec::new();
        };
        let param = |id: u32| self.param_f32(id);
        blend::leaves(tree, &param)
    }

    /// A state's length in seconds for one loop: its leaves' lengths
    /// (times their duration factors) by weight; 1 s without clips.
    fn state_length(&self, layer: usize, state: usize) -> f32 {
        let total: f32 = self
            .leaves(layer, state)
            .iter()
            .filter_map(|l| {
                let p = self.program.clips.get(l.clip as usize)?.as_ref()?;
                Some(l.weight * p.length * l.duration.abs())
            })
            .sum();
        if total > 1e-6 { total } else { 1.0 }
    }

    fn state_speed(&self, layer: usize, state: usize) -> f32 {
        let Some(s) = self.machine(layer).and_then(|sm| sm.states.get(state)) else {
            return 1.0;
        };
        let mut speed = s.speed;
        if s.speed_param != 0 && self.param(s.speed_param).is_some() {
            speed *= self.param_f32(s.speed_param);
        }
        speed
    }

    fn advance(&self, layer: usize, p: &mut Playing, dt: f32) {
        p.prev = p.time;
        let s = self.machine(layer).and_then(|sm| sm.states.get(p.state));
        if let Some(s) = s
            && s.time_param != 0
            && self.param(s.time_param).is_some()
        {
            p.time = self.param_f32(s.time_param);
            return;
        }
        p.time += dt * self.state_speed(layer, p.state) / self.state_length(layer, p.state);
    }

    fn conditions_hold(&self, conditions: &[sn_unity::Condition]) -> bool {
        conditions.iter().all(|c| {
            let Some(v) = self.param(c.param) else {
                return false;
            };
            match c.mode {
                ConditionMode::If => v.as_bool(),
                ConditionMode::IfNot => !v.as_bool(),
                ConditionMode::Greater => v.as_f32() > c.threshold,
                ConditionMode::Less => v.as_f32() < c.threshold,
                ConditionMode::Equals => v.as_f32() == c.threshold,
                ConditionMode::NotEqual => v.as_f32() != c.threshold,
                ConditionMode::ExitTime | ConditionMode::Other(_) => true,
            }
        })
    }

    fn consume_triggers(&mut self, conditions: &[sn_unity::Condition]) {
        for c in conditions {
            if let Some(&i) = self.program.params.get(&c.param)
                && let ParamValue::Trigger(true) = self.params[i]
            {
                self.params[i] = ParamValue::Trigger(false);
            }
        }
    }

    /// Follows selectors (entry/exit nodes) to a state. `None`: no
    /// selector transition holds, or a selector leads nowhere.
    fn resolve(&mut self, layer: usize, destination: u32, consume: bool) -> Option<usize> {
        let mut d = destination;
        for _ in 0..16 {
            if d < SELECTOR_BASE {
                return Some(d as usize);
            }
            let sm = self.machine(layer)?;
            let sel = sm.selectors.get((d - SELECTOR_BASE) as usize)?;
            let found = sel
                .transitions
                .iter()
                .find(|t| self.conditions_hold(&t.conditions))
                .map(|t| (t.destination, t.conditions.clone()));
            let (next, conditions) = found?;
            if consume {
                self.consume_triggers(&conditions);
            }
            if next == u32::MAX {
                return None;
            }
            d = next;
        }
        None
    }

    /// Whether `t`'s exit time is reached by `p` in the last update
    /// (**hypothesis**: below 1 it is met each loop when the fraction
    /// crosses it; from 1 on, once the time is at or past it).
    fn exit_time_met(t: &Transition, p: &Playing) -> bool {
        if !t.has_exit_time {
            return true;
        }
        let et = t.exit_time;
        if et >= 1.0 {
            return p.time >= et;
        }
        // An integer k ≥ 0 with prev < k + et ≤ time: the smallest
        // integer above prev − et, at least 0, is at most time − et.
        let k = ((p.prev - et).floor() + 1.0).max(0.0);
        k <= p.time - et
    }

    /// The first transition among `candidates` that holds, as (where it
    /// is, its destination state).
    fn pick(
        &mut self,
        layer: usize,
        candidates: &[(TransitionRef, Playing)],
        skip_to: Option<usize>,
    ) -> Option<(TransitionRef, usize, Playing)> {
        for &(via, from) in candidates {
            let t = self.transition(layer, via)?.clone();
            if !self.conditions_hold(&t.conditions) || !Self::exit_time_met(&t, &from) {
                continue;
            }
            // Find the destination first; triggers are used only once the
            // transition is taken.
            let Some(dest) = self.resolve(layer, t.destination, false) else {
                continue;
            };
            if matches!(via, TransitionRef::Any(_))
                && !t.can_transition_to_self
                && (Some(dest) == skip_to || dest == from.state)
            {
                continue;
            }
            self.resolve(layer, t.destination, true);
            self.consume_triggers(&t.conditions);
            return Some((via, dest, from));
        }
        None
    }

    fn transition(&self, layer: usize, via: TransitionRef) -> Option<&Transition> {
        let sm = self.machine(layer)?;
        match via {
            TransitionRef::State(s, i) => sm.states.get(s)?.transitions.get(i),
            TransitionRef::Any(i) => sm.any_state_transitions.get(i),
        }
    }

    /// Advances every layer by `dt` seconds and computes the pose.
    pub fn update(&mut self, dt: f32) {
        self.started.clear();
        for layer in 0..self.layers.len() {
            self.update_layer(layer, dt);
        }
        self.evaluate();
    }

    fn update_layer(&mut self, layer: usize, dt: f32) {
        let mut run = self.layers[layer].clone();
        if let Current::State(p) = &mut run.current {
            self.advance(layer, p, dt);
        }
        if let Some(t) = &mut run.transit {
            self.advance(layer, &mut t.next, dt);
            t.elapsed += dt;
            if t.elapsed >= t.duration {
                run.current = Current::State(t.next);
                run.transit = None;
            }
        }
        let Some(sm) = self.machine(layer) else {
            self.layers[layer] = run;
            return;
        };
        let any = sm.any_state_transitions.len();
        // Candidates in priority order: any-state transitions, then the
        // ones the interruption rules allow (**hypothesis**: any-state
        // transitions are checked during transitions too).
        let mut candidates: Vec<(TransitionRef, Playing)> = Vec::new();
        let from = match (&run.current, &run.transit) {
            (_, Some(t)) => t.next,
            (Current::State(p), None) => *p,
            (Current::Frozen(_), None) => Playing::enter(0, 0.0),
        };
        for i in 0..any {
            candidates.push((TransitionRef::Any(i), from));
        }
        let skip_to = run.transit.as_ref().map(|t| t.next.state);
        match (&run.current, &run.transit) {
            (Current::State(p), None) => {
                let n = sm.states.get(p.state).map_or(0, |s| s.transitions.len());
                for i in 0..n {
                    candidates.push((TransitionRef::State(p.state, i), *p));
                }
            }
            (current, Some(t)) => {
                let Some(active) = self.transition(layer, t.via).cloned() else {
                    self.layers[layer] = run;
                    return;
                };
                let source = match (current, t.via) {
                    (Current::State(p), TransitionRef::State(s, i)) if p.state == s => {
                        let n = if active.ordered_interruption {
                            i
                        } else {
                            sm.states.get(s).map_or(0, |st| st.transitions.len())
                        };
                        (0..n)
                            .filter(|&j| j != i)
                            .map(|j| (TransitionRef::State(s, j), *p))
                            .collect()
                    }
                    (Current::State(p), _) => {
                        let n = sm.states.get(p.state).map_or(0, |s| s.transitions.len());
                        (0..n)
                            .map(|j| (TransitionRef::State(p.state, j), *p))
                            .collect()
                    }
                    _ => Vec::new(),
                };
                let n = sm
                    .states
                    .get(t.next.state)
                    .map_or(0, |s| s.transitions.len());
                let dest: Vec<_> = (0..n)
                    .map(|j| (TransitionRef::State(t.next.state, j), t.next))
                    .collect();
                match active.interruption {
                    Interruption::Source => candidates.extend(source),
                    Interruption::Destination => candidates.extend(dest),
                    Interruption::SourceThenDestination => {
                        candidates.extend(source);
                        candidates.extend(dest);
                    }
                    Interruption::DestinationThenSource => {
                        candidates.extend(dest);
                        candidates.extend(source);
                    }
                    Interruption::None | Interruption::Other(_) => {}
                }
            }
            (Current::Frozen(_), None) => {}
        }
        if let Some((via, dest, from)) = self.pick(layer, &candidates, skip_to) {
            let t = self.transition(layer, via).cloned();
            if let Some(t) = t {
                let duration = if t.has_fixed_duration {
                    t.duration
                } else {
                    t.duration * self.state_length(layer, from.state)
                };
                let next = Playing::enter(dest, t.offset);
                let names = self.machine(layer).map(|sm| {
                    (
                        sm.states.get(from.state).map_or(0, |s| s.name_id),
                        sm.states.get(dest).map_or(0, |s| s.name_id),
                    )
                });
                if let Some((a, b)) = names {
                    self.started.push((layer, a, b));
                }
                if run.transit.is_some() {
                    // Interrupted: blend on from the layer's values now.
                    run.current = Current::Frozen(self.layer_values(layer, &run));
                }
                if duration <= 0.0 {
                    run.current = Current::State(next);
                    run.transit = None;
                } else {
                    run.transit = Some(Transit {
                        next,
                        elapsed: 0.0,
                        duration,
                        via,
                    });
                }
            }
        }
        self.layers[layer] = run;
    }

    /// The layer's own values (over its slots; other values are the
    /// defaults) as it stands, for an interruption.
    fn layer_values(&mut self, layer: usize, run: &LayerRun) -> Vec<f32> {
        let saved = self.layers[layer].clone();
        self.layers[layer] = run.clone();
        let mut out = self.defaults.clone();
        let prog = self.program.clone();
        self.accumulate(
            layer,
            prog.layers[layer].blending == LayerBlending::Additive,
        );
        for &s in &prog.layers[layer].slots {
            let slot = prog.slots[s];
            let w = slot.kind.width();
            out[slot.offset..slot.offset + w]
                .copy_from_slice(&self.acc[slot.offset..slot.offset + w]);
        }
        self.layers[layer] = saved;
        out
    }

    /// Fills `acc` over the layer's slots with its blended values (or,
    /// for `additive`, deltas from each clip's start).
    fn accumulate(&mut self, layer: usize, additive: bool) {
        let prog = self.program.clone();
        let lp = &prog.layers[layer];
        for &s in &lp.slots {
            let slot = prog.slots[s];
            self.acc_w[s] = 0.0;
            for i in 0..slot.kind.width() {
                self.acc[slot.offset + i] = 0.0;
            }
        }
        let run = self.layers[layer].clone();
        let mut parts: Vec<(Option<Playing>, f32)> = Vec::new();
        let frozen = match &run.current {
            Current::Frozen(v) => Some(v.clone()),
            Current::State(_) => None,
        };
        match (&run.current, &run.transit) {
            (Current::State(p), None) => parts.push((Some(*p), 1.0)),
            (current, Some(t)) => {
                let a = blend_fraction(t);
                match current {
                    Current::State(p) => parts.push((Some(*p), 1.0 - a)),
                    Current::Frozen(_) => parts.push((None, 1.0 - a)),
                }
                parts.push((Some(t.next), a));
            }
            (Current::Frozen(_), None) => parts.push((None, 1.0)),
        }
        for (part, weight) in parts {
            if weight <= 0.0 {
                continue;
            }
            match part {
                None => {
                    if let Some(v) = &frozen {
                        for &s in &lp.slots {
                            let slot = prog.slots[s];
                            self.add(s, slot, &v[slot.offset..], weight);
                        }
                    }
                }
                Some(p) => self.add_state(layer, p, weight, additive),
            }
        }
        // What no part wrote passes the layers below through (an empty
        // state writes nothing: the game's player keeps its weight-1 arm
        // layers in empty states while the base layer animates the arms);
        // additive layers add nothing there. Then normalise; quaternions
        // to unit length.
        for &s in &lp.slots {
            let slot = prog.slots[s];
            let uncovered = 1.0 - self.acc_w[s];
            if uncovered > 1e-6 {
                let at = slot.offset;
                let width = slot.kind.width();
                let mut v = [0.0f32; 4];
                if additive {
                    if slot.kind == SlotKind::Rotation {
                        v = math::IDENTITY;
                    }
                } else {
                    v[..width].copy_from_slice(&self.below[at..at + width]);
                }
                self.add(s, slot, &v[..width], uncovered);
            }
            let w = self.acc_w[s];
            let at = slot.offset;
            if slot.kind == SlotKind::Rotation {
                let q = math::normalize([
                    self.acc[at],
                    self.acc[at + 1],
                    self.acc[at + 2],
                    self.acc[at + 3],
                ]);
                self.acc[at..at + 4].copy_from_slice(&q);
            } else if w > 1e-8 {
                for i in 0..slot.kind.width() {
                    self.acc[at + i] /= w;
                }
            }
        }
    }

    /// Adds `value` (a slot's values) with weight `w` to the accumulator.
    fn add(&mut self, s: usize, slot: Slot, value: &[f32], w: f32) {
        let at = slot.offset;
        if slot.kind == SlotKind::Rotation {
            let q = [value[0], value[1], value[2], value[3]];
            let align = if self.acc_w[s] > 0.0 {
                [
                    self.acc[at],
                    self.acc[at + 1],
                    self.acc[at + 2],
                    self.acc[at + 3],
                ]
            } else {
                q
            };
            add_quat(&mut self.acc[at..at + 4], q, w, align);
        } else {
            let width = slot.kind.width();
            for (a, v) in self.acc[at..at + width].iter_mut().zip(value) {
                *a += v * w;
            }
        }
        self.acc_w[s] += w;
    }

    fn add_state(&mut self, layer: usize, p: Playing, weight: f32, additive: bool) {
        let prog = self.program.clone();
        let lp = &prog.layers[layer];
        let Some(state) = self.machine(layer).and_then(|sm| sm.states.get(p.state)) else {
            return;
        };
        let write_defaults = state.write_default_values;
        let mut offset = state.cycle_offset;
        if state.cycle_offset_param != 0 {
            offset += self.param_f32(state.cycle_offset_param);
        }
        for &s in &lp.slots {
            self.state_w[s] = 0.0;
        }
        let mut wrote = false;
        for leaf in self.leaves(layer, p.state) {
            let Some(Some(clip)) = prog.clips.get(leaf.clip as usize) else {
                continue;
            };
            let w = weight * leaf.weight;
            if w <= 0.0 {
                continue;
            }
            let mut t = p.time + offset + leaf.cycle_offset + clip.clip.cycle_offset;
            t = if clip.clip.loop_time {
                t.rem_euclid(1.0)
            } else {
                t.clamp(0.0, 1.0)
            };
            if leaf.duration < 0.0 {
                t = 1.0 - t;
            }
            wrote = true;
            let mut curves = std::mem::take(&mut self.curves);
            sample(
                &clip.clip,
                clip.clip.start_time + t * clip.length,
                &mut curves,
            );
            for wr in &clip.writes {
                let s = wr.slot;
                if lp.mask[s] <= 0.0 {
                    continue;
                }
                let slot = prog.slots[s];
                let width = slot.kind.width();
                let mut value = value_of(wr, &curves);
                if additive {
                    let r = &clip.reference[slot.offset..slot.offset + width];
                    if slot.kind == SlotKind::Rotation {
                        let d = math::mul(
                            math::conjugate([r[0], r[1], r[2], r[3]]),
                            [value[0], value[1], value[2], value[3]],
                        );
                        value = d;
                    } else {
                        for i in 0..width {
                            value[i] -= r[i];
                        }
                    }
                }
                self.add(s, slot, &value[..width], w);
                self.state_w[s] += w;
            }
            self.curves = curves;
        }
        // What the state's motions leave out: the default (write
        // defaults on), else nothing (the layers below show through; on
        // the first layer, the value before this frame), or no change
        // (additive). A state without a motion writes nothing at all.
        if !wrote || additive || (!write_defaults && layer > 0) {
            return;
        }
        for &s in &lp.slots {
            let missing = weight - self.state_w[s];
            if missing <= 1e-6 {
                continue;
            }
            let slot = prog.slots[s];
            let width = slot.kind.width();
            let at = slot.offset;
            let mut v = [0.0f32; 4];
            if write_defaults {
                v[..width].copy_from_slice(&self.defaults[at..at + width]);
            } else {
                v[..width].copy_from_slice(&self.pose[at..at + width]);
            }
            self.add(s, slot, &v[..width], missing);
        }
    }

    /// Runs the layers over the defaults into the pose.
    fn evaluate(&mut self) {
        let prog = self.program.clone();
        self.below.copy_from_slice(&self.defaults);
        for layer in 0..self.layers.len() {
            let lp = &prog.layers[layer];
            let weight = self.layers[layer].weight;
            if weight <= 0.0 || lp.slots.is_empty() {
                continue;
            }
            let additive = lp.blending == LayerBlending::Additive;
            self.accumulate(layer, additive);
            for &s in &lp.slots {
                let slot = prog.slots[s];
                let at = slot.offset;
                let w = (weight * lp.mask[s]).clamp(0.0, 1.0);
                if slot.kind == SlotKind::Rotation {
                    let below = [
                        self.below[at],
                        self.below[at + 1],
                        self.below[at + 2],
                        self.below[at + 3],
                    ];
                    let v = [
                        self.acc[at],
                        self.acc[at + 1],
                        self.acc[at + 2],
                        self.acc[at + 3],
                    ];
                    let q = if additive {
                        math::mul(below, math::nlerp(math::IDENTITY, v, w))
                    } else {
                        math::nlerp(below, v, w)
                    };
                    self.below[at..at + 4].copy_from_slice(&q);
                } else {
                    for i in 0..slot.kind.width() {
                        let b = self.below[at + i];
                        let v = self.acc[at + i];
                        self.below[at + i] = if additive { b + v * w } else { b + (v - b) * w };
                    }
                }
            }
        }
        self.pose.copy_from_slice(&self.below);
    }
}

fn blend_fraction(t: &Transit) -> f32 {
    if t.duration > 0.0 {
        (t.elapsed / t.duration).clamp(0.0, 1.0)
    } else {
        1.0
    }
}
