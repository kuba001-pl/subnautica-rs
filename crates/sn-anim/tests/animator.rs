//! The `Animator` on controllers built in code: one rule per test.

use std::sync::Arc;

use sn_anim::{Animator, ParamValue, Program, SlotKind};
use sn_unity::{
    ATTR_POSITION, ATTR_ROTATION, AnimationClip, AnimatorController, BIND_TRANSFORM, BlendNode,
    BlendType, Condition, ConditionMode, DefaultValues, DenseClip, GenericBinding, Interruption,
    Layer, LayerBlending, PPtr, Param, ParamKind, SELECTOR_BASE, SelectorState, SelectorTransition,
    State, StateMachine, StreamedKey, Transition, name_hash,
};

const ROOT: u32 = 0; // name_hash("")

fn key(time: f32, coeff: [f32; 4]) -> StreamedKey {
    StreamedKey { time, coeff }
}

/// A curve linear from v0 at t = 0 to v1 at t = len, constant outside.
fn ramp(len: f32, v0: f32, v1: f32) -> Vec<StreamedKey> {
    vec![
        key(f32::MIN, [0.0, 0.0, 0.0, v0]),
        key(0.0, [0.0, 0.0, (v1 - v0) / len, v0]),
        key(len, [0.0, 0.0, 0.0, v1]),
        key(f32::MAX, [0.0, 0.0, 0.0, v1]),
    ]
}

fn constant(v: f32) -> Vec<StreamedKey> {
    vec![key(f32::MIN, [0.0, 0.0, 0.0, v])]
}

fn binding(path: u32, attribute: u32) -> GenericBinding {
    GenericBinding {
        path,
        attribute,
        script: PPtr::default(),
        type_id: BIND_TRANSFORM,
        custom_type: 0,
        is_pptr_curve: false,
    }
}

/// A clip moving `path`'s position by the three given curves.
fn clip(
    name: &str,
    len: f32,
    looping: bool,
    path: u32,
    xyz: [Vec<StreamedKey>; 3],
) -> AnimationClip {
    let [x, y, z] = xyz;
    AnimationClip {
        name: name.into(),
        legacy: false,
        compressed: false,
        editor_curves: [0; 7],
        sample_rate: 30.0,
        wrap_mode: 0,
        bounds: ([0.0; 3], [0.0; 3]),
        streamed: vec![x, y, z],
        dense: DenseClip::default(),
        constant: vec![],
        start_time: 0.0,
        stop_time: len,
        orientation_offset_y: 0.0,
        level: 0.0,
        cycle_offset: 0.0,
        average_angular_speed: 0.0,
        average_speed: [0.0; 3],
        index_array: vec![],
        value_array_delta: vec![],
        value_array_reference_pose: vec![],
        mirror: false,
        loop_time: looping,
        loop_blend: false,
        loop_blend_orientation: false,
        loop_blend_position_y: false,
        loop_blend_position_xz: false,
        start_at_origin: false,
        keep_original_orientation: false,
        keep_original_position_y: false,
        keep_original_position_xz: false,
        height_from_feet: false,
        bindings: vec![binding(path, ATTR_POSITION)],
        pptr_curve_mapping: vec![],
        has_generic_root_transform: false,
        has_motion_float_curves: false,
        events: vec![],
    }
}

/// x goes 0 → 1 over 1 s (y, z stay 0).
fn ramp_x(name: &str, looping: bool) -> AnimationClip {
    clip(
        name,
        1.0,
        looping,
        ROOT,
        [ramp(1.0, 0.0, 1.0), constant(0.0), constant(0.0)],
    )
}

/// x held at `v`.
fn hold_x(name: &str, v: f32) -> AnimationClip {
    clip(
        name,
        1.0,
        false,
        ROOT,
        [constant(v), constant(0.0), constant(0.0)],
    )
}

fn leaf(clip: u32) -> BlendNode {
    BlendNode {
        kind: BlendType::Simple1D,
        param: 0,
        param_y: 0,
        children: vec![],
        thresholds: vec![],
        positions: vec![],
        magnitudes: vec![],
        pair_vectors: vec![],
        pair_avg_mag_inv: vec![],
        neighbors: vec![],
        direct_params: vec![],
        normalized_blend_values: false,
        clip,
        duration: 1.0,
        cycle_offset: 0.0,
        mirror: false,
    }
}

fn state(name: &str, tree: Vec<BlendNode>, transitions: Vec<Transition>) -> State {
    State {
        transitions,
        blend_tree_index: vec![0],
        blend_trees: vec![tree],
        name_id: name_hash(name),
        path_id: name_hash(name),
        full_path_id: name_hash(&format!("Base.{name}")),
        tag_id: 0,
        speed_param: 0,
        mirror_param: 0,
        cycle_offset_param: 0,
        time_param: 0,
        speed: 1.0,
        cycle_offset: 0.0,
        ik_on_feet: false,
        write_default_values: true,
        looping: false,
        mirror: false,
    }
}

fn to(destination: u32, conditions: Vec<Condition>) -> Transition {
    Transition {
        conditions,
        destination,
        full_path_id: 0,
        id: 0,
        user_id: 0,
        duration: 0.0,
        offset: 0.0,
        exit_time: 0.0,
        has_exit_time: false,
        has_fixed_duration: true,
        interruption: Interruption::None,
        ordered_interruption: true,
        can_transition_to_self: true,
    }
}

fn cond(mode: ConditionMode, param: &str, threshold: f32) -> Condition {
    Condition {
        mode,
        param: name_hash(param),
        threshold,
        exit_time: 0.0,
    }
}

fn machine(states: Vec<State>) -> StateMachine {
    StateMachine {
        states,
        any_state_transitions: vec![],
        selectors: vec![SelectorState {
            transitions: vec![SelectorTransition {
                destination: 0,
                conditions: vec![],
            }],
            full_path_id: name_hash("Base"),
            is_entry: true,
        }],
        default_state: 0,
        synchronized_layer_count: 1,
    }
}

fn layer(machine: u32) -> Layer {
    Layer {
        state_machine: machine,
        synchronized_layer: 0,
        body_mask: [u32::MAX; 3],
        skeleton_mask: vec![],
        binding: 0,
        blending: LayerBlending::Override,
        default_weight: 1.0,
        ik_pass: false,
        synced_layer_affects_timing: false,
    }
}

/// Parameters: (name, kind).
fn controller(
    layers: Vec<Layer>,
    machines: Vec<StateMachine>,
    params: &[(&str, ParamKind)],
    clips: usize,
) -> AnimatorController {
    let mut defaults = DefaultValues::default();
    let params = params
        .iter()
        .map(|&(n, kind)| {
            let index = match kind {
                ParamKind::Float => {
                    defaults.floats.push(0.0);
                    defaults.floats.len() - 1
                }
                ParamKind::Int => {
                    defaults.ints.push(0);
                    defaults.ints.len() - 1
                }
                _ => {
                    defaults.bools.push(false);
                    defaults.bools.len() - 1
                }
            };
            Param {
                id: name_hash(n),
                kind,
                index: index as u32,
            }
        })
        .collect();
    AnimatorController {
        name: "test".into(),
        layers,
        state_machines: machines,
        params,
        defaults,
        names: vec![],
        clips: vec![PPtr::default(); clips],
        state_machine_behaviours: vec![],
        multi_threaded: true,
    }
}

fn animator(c: AnimatorController, clips: Vec<AnimationClip>, defaults: Vec<f32>) -> Animator {
    let clips: Vec<_> = clips.into_iter().map(|c| Some(Arc::new(c))).collect();
    let program = Arc::new(Program::new(Arc::new(c), &clips));
    Animator::new(program, defaults)
}

fn x(a: &Animator) -> f32 {
    let p = a.program();
    let s = p.slot(ROOT, SlotKind::Position).expect("position slot");
    a.pose()[p.slots[s].offset]
}

fn state_is(a: &Animator, layer: usize, name: &str) -> bool {
    a.layer_info(layer).and_then(|i| i.state) == Some(name_hash(name))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn a_looping_clip_plays_and_wraps() {
    let c = controller(
        vec![layer(0)],
        vec![machine(vec![state("a", vec![leaf(0)], vec![])])],
        &[],
        1,
    );
    let mut a = animator(c, vec![ramp_x("a", true)], vec![]);
    a.update(0.25);
    assert!(close(x(&a), 0.25), "{}", x(&a));
    a.update(1.0);
    assert!(close(x(&a), 0.25), "{}", x(&a));
    assert!(close(a.layer_info(0).unwrap().normalized_time, 1.25));
}

#[test]
fn a_non_looping_clip_holds_its_end() {
    let c = controller(
        vec![layer(0)],
        vec![machine(vec![state("a", vec![leaf(0)], vec![])])],
        &[],
        1,
    );
    let mut a = animator(c, vec![ramp_x("a", false)], vec![]);
    a.update(3.0);
    assert!(close(x(&a), 1.0));
}

#[test]
fn a_bool_starts_a_fixed_blend() {
    let mut t = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    t.duration = 0.5;
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![t]),
        state("b", vec![leaf(1)], vec![]),
    ]);
    let c = controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2);
    let mut a = animator(c, vec![hold_x("a", 0.0), hold_x("b", 10.0)], vec![]);
    a.update(0.1);
    assert!(state_is(&a, 0, "a") && close(x(&a), 0.0));
    a.set_bool(name_hash("go"), true);
    a.update(0.0); // the transition starts
    a.update(0.25); // halfway
    assert!(close(x(&a), 5.0), "{}", x(&a));
    let info = a.layer_info(0).unwrap();
    assert_eq!(info.next.map(|n| n.0), Some(name_hash("b")));
    a.update(0.3);
    assert!(state_is(&a, 0, "b") && close(x(&a), 10.0));
}

#[test]
fn normalised_durations_scale_with_the_source() {
    let mut t = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    t.duration = 0.25;
    t.has_fixed_duration = false;
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![t]),
        state("b", vec![leaf(1)], vec![]),
    ]);
    let c = controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2);
    // Source 4 s long: 0.25 of it is 1 s.
    let long = clip(
        "a",
        4.0,
        true,
        ROOT,
        [constant(0.0), constant(0.0), constant(0.0)],
    );
    let mut a = animator(c, vec![long, hold_x("b", 10.0)], vec![]);
    a.set_bool(name_hash("go"), true);
    a.update(0.0);
    a.update(0.5);
    assert!(close(x(&a), 5.0), "{}", x(&a));
}

#[test]
fn exit_time_fires_when_crossed() {
    let mut t = to(1, vec![]);
    t.has_exit_time = true;
    t.exit_time = 0.75;
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![t]),
        state("b", vec![leaf(1)], vec![]),
    ]);
    let c = controller(vec![layer(0)], vec![sm], &[], 2);
    let mut a = animator(c, vec![ramp_x("a", true), hold_x("b", 10.0)], vec![]);
    a.update(0.5);
    assert!(state_is(&a, 0, "a"));
    a.update(0.3);
    assert!(state_is(&a, 0, "b"));
}

#[test]
fn exit_time_waits_for_the_next_loop_when_conditions_come_late() {
    let mut t = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    t.has_exit_time = true;
    t.exit_time = 0.5;
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![t]),
        state("b", vec![leaf(1)], vec![]),
    ]);
    let c = controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2);
    let mut a = animator(c, vec![ramp_x("a", true), hold_x("b", 10.0)], vec![]);
    a.update(0.6); // crossed 0.5 without "go"
    a.set_bool(name_hash("go"), true);
    a.update(0.3); // 0.9: not crossed again yet
    assert!(state_is(&a, 0, "a"));
    a.update(0.7); // 1.6: crossed 1.5
    assert!(state_is(&a, 0, "b"));
}

#[test]
fn triggers_are_used_up_by_the_transition() {
    let ab = to(1, vec![cond(ConditionMode::If, "hit", 0.0)]);
    let ba = to(0, vec![cond(ConditionMode::If, "hit", 0.0)]);
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![ab]),
        state("b", vec![leaf(1)], vec![ba]),
    ]);
    let c = controller(vec![layer(0)], vec![sm], &[("hit", ParamKind::Trigger)], 2);
    let mut a = animator(c, vec![hold_x("a", 0.0), hold_x("b", 1.0)], vec![]);
    a.set_trigger(name_hash("hit"));
    a.update(0.1);
    assert!(state_is(&a, 0, "b"));
    assert_eq!(a.get(name_hash("hit")), Some(ParamValue::Trigger(false)));
    a.update(0.1);
    assert!(state_is(&a, 0, "b"), "the trigger fired twice");
}

#[test]
fn conditions_compare_numbers() {
    let sm = machine(vec![
        state(
            "a",
            vec![leaf(0)],
            vec![
                to(
                    1,
                    vec![
                        cond(ConditionMode::Greater, "f", 0.5),
                        cond(ConditionMode::IfNot, "b", 0.0),
                    ],
                ),
                to(2, vec![cond(ConditionMode::Equals, "i", 3.0)]),
            ],
        ),
        state(
            "big",
            vec![leaf(0)],
            vec![to(0, vec![cond(ConditionMode::Less, "f", 0.5)])],
        ),
        state("three", vec![leaf(0)], vec![]),
    ]);
    let c = controller(
        vec![layer(0)],
        vec![sm],
        &[
            ("f", ParamKind::Float),
            ("b", ParamKind::Bool),
            ("i", ParamKind::Int),
        ],
        1,
    );
    let mut a = animator(c, vec![hold_x("a", 0.0)], vec![]);
    a.set_float(name_hash("f"), 0.7);
    a.set_bool(name_hash("b"), true);
    a.update(0.1);
    assert!(state_is(&a, 0, "a"), "IfNot b held while b is true");
    a.set_bool(name_hash("b"), false);
    a.update(0.1);
    assert!(state_is(&a, 0, "big"));
    a.set_float(name_hash("f"), 0.2);
    a.update(0.1);
    assert!(state_is(&a, 0, "a"));
    a.set_int(name_hash("i"), 3);
    a.update(0.1);
    assert!(state_is(&a, 0, "three"));
}

#[test]
fn the_offset_starts_the_destination_later() {
    let mut t = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    t.offset = 0.5;
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![t]),
        state("b", vec![leaf(1)], vec![]),
    ]);
    let c = controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2);
    let mut a = animator(c, vec![hold_x("a", 0.0), ramp_x("b", false)], vec![]);
    a.set_bool(name_hash("go"), true);
    a.update(0.0);
    assert!(state_is(&a, 0, "b"));
    assert!(close(x(&a), 0.5), "{}", x(&a));
}

#[test]
fn any_state_transitions_do_not_restart_their_own_state() {
    let mut sm = machine(vec![
        state("a", vec![leaf(0)], vec![]),
        state("b", vec![leaf(1)], vec![]),
    ]);
    let mut any = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    any.can_transition_to_self = false;
    sm.any_state_transitions = vec![any];
    let c = controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2);
    let mut a = animator(c, vec![hold_x("a", 0.0), ramp_x("b", true)], vec![]);
    a.set_bool(name_hash("go"), true);
    a.update(0.0);
    assert!(state_is(&a, 0, "b"));
    a.update(0.25);
    a.update(0.25);
    assert!(
        close(a.layer_info(0).unwrap().normalized_time, 0.5),
        "restarted"
    );
}

#[test]
fn the_destination_may_interrupt() {
    let mut ab = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    ab.duration = 1.0;
    ab.interruption = Interruption::Destination;
    let mut bc = to(2, vec![cond(ConditionMode::If, "more", 0.0)]);
    bc.duration = 1.0;
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![ab]),
        state("b", vec![leaf(1)], vec![bc]),
        state("c", vec![leaf(2)], vec![]),
    ]);
    let c = controller(
        vec![layer(0)],
        vec![sm],
        &[("go", ParamKind::Bool), ("more", ParamKind::Bool)],
        3,
    );
    let mut a = animator(
        c,
        vec![hold_x("a", 0.0), hold_x("b", 10.0), hold_x("c", 20.0)],
        vec![],
    );
    a.set_bool(name_hash("go"), true);
    a.update(0.0);
    a.update(0.5); // a→b halfway: x = 5
    assert!(close(x(&a), 5.0));
    a.set_bool(name_hash("more"), true);
    a.update(0.0); // interrupted: from the frozen 5 towards c
    let info = a.layer_info(0).unwrap();
    assert_eq!(info.state, None);
    assert_eq!(info.next.map(|n| n.0), Some(name_hash("c")));
    a.update(0.5); // from the frozen 5 halfway to 20
    assert!(close(x(&a), 12.5), "{}", x(&a));
    a.update(0.6);
    assert!(state_is(&a, 0, "c") && close(x(&a), 20.0), "{}", x(&a));
}

#[test]
fn without_interruption_the_blend_finishes_first() {
    let mut ab = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    ab.duration = 1.0;
    let bc = to(2, vec![cond(ConditionMode::If, "more", 0.0)]);
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![ab]),
        state("b", vec![leaf(1)], vec![bc]),
        state("c", vec![leaf(2)], vec![]),
    ]);
    let c = controller(
        vec![layer(0)],
        vec![sm],
        &[("go", ParamKind::Bool), ("more", ParamKind::Bool)],
        3,
    );
    let mut a = animator(
        c,
        vec![hold_x("a", 0.0), hold_x("b", 10.0), hold_x("c", 20.0)],
        vec![],
    );
    a.set_bool(name_hash("go"), true);
    a.set_bool(name_hash("more"), true);
    a.update(0.0);
    a.update(0.5);
    assert!(close(x(&a), 5.0));
    a.update(0.6); // a→b done; then b→c at once
    assert!(state_is(&a, 0, "c"));
}

#[test]
fn exit_nodes_lead_back_to_the_entry() {
    let mut sm = machine(vec![
        state(
            "a",
            vec![leaf(0)],
            vec![to(1, vec![cond(ConditionMode::If, "go", 0.0)])],
        ),
        state(
            "b",
            vec![leaf(1)],
            vec![to(
                SELECTOR_BASE + 1,
                vec![cond(ConditionMode::IfNot, "go", 0.0)],
            )],
        ),
    ]);
    sm.selectors.push(SelectorState {
        transitions: vec![SelectorTransition {
            destination: SELECTOR_BASE,
            conditions: vec![],
        }],
        full_path_id: name_hash("Base"),
        is_entry: false,
    });
    let c = controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2);
    let mut a = animator(c, vec![hold_x("a", 0.0), hold_x("b", 1.0)], vec![]);
    a.set_bool(name_hash("go"), true);
    a.update(0.1);
    assert!(state_is(&a, 0, "b"));
    a.set_bool(name_hash("go"), false);
    a.update(0.1);
    assert!(state_is(&a, 0, "a"));
}

#[test]
fn override_layers_blend_by_weight_and_mask() {
    let arm = name_hash("arm");
    // Base: root x = 1, arm x = 1. Upper: root x = 9, arm x = 9; its mask
    // keeps only the arm.
    let both = |name: &str, v: f32| {
        let mut c = clip(
            name,
            1.0,
            false,
            ROOT,
            [constant(v), constant(0.0), constant(0.0)],
        );
        c.streamed
            .extend([constant(v), constant(0.0), constant(0.0)]);
        c.bindings.push(binding(arm, ATTR_POSITION));
        c
    };
    let mut upper = layer(1);
    upper.default_weight = 0.5;
    upper.skeleton_mask = vec![(arm, 1.0)];
    let c = controller(
        vec![layer(0), upper],
        vec![
            machine(vec![state("base", vec![leaf(0)], vec![])]),
            machine(vec![state("up", vec![leaf(1)], vec![])]),
        ],
        &[],
        2,
    );
    let mut a = animator(c, vec![both("base", 1.0), both("up", 9.0)], vec![]);
    a.update(0.1);
    let p = a.program().clone();
    let arm_x = a.pose()[p.slots[p.slot(arm, SlotKind::Position).unwrap()].offset];
    assert!(close(x(&a), 1.0), "masked out: {}", x(&a));
    assert!(close(arm_x, 5.0), "half way: {arm_x}");
}

#[test]
fn additive_layers_add_the_change_from_the_clip_start() {
    let mut add = layer(1);
    add.blending = LayerBlending::Additive;
    let c = controller(
        vec![layer(0), add],
        vec![
            machine(vec![state("base", vec![leaf(0)], vec![])]),
            machine(vec![state("wave", vec![leaf(1)], vec![])]),
        ],
        &[],
        2,
    );
    // Additive clip goes 3 → 5: +2 at its end.
    let wave = clip(
        "wave",
        1.0,
        false,
        ROOT,
        [ramp(1.0, 3.0, 5.0), constant(0.0), constant(0.0)],
    );
    let mut a = animator(c, vec![hold_x("base", 10.0), wave], vec![]);
    a.update(0.5);
    assert!(close(x(&a), 11.0), "{}", x(&a));
    a.update(1.0);
    assert!(close(x(&a), 12.0), "{}", x(&a));
}

#[test]
fn write_defaults_put_back_what_a_state_does_not_animate() {
    // "a" animates x; "b" animates nothing of the root's position.
    let none = {
        let mut c = hold_x("none", 0.0);
        c.bindings[0].path = name_hash("elsewhere");
        c
    };
    let ab = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    let make = |write_defaults: bool| {
        let mut b = state("b", vec![leaf(1)], vec![]);
        b.write_default_values = write_defaults;
        let sm = machine(vec![state("a", vec![leaf(0)], vec![ab.clone()]), b]);
        controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2)
    };
    // Defaults: root position (7, 0, 0) — the slot order follows the clips.
    for (wd, expected) in [(true, 7.0), (false, 3.0)] {
        let mut a = animator(make(wd), vec![hold_x("a", 3.0), none.clone()], vec![]);
        let p = a.program().clone();
        let s = p.slot(ROOT, SlotKind::Position).unwrap();
        let mut defaults = vec![0.0; p.width];
        defaults[p.slots[s].offset] = 7.0;
        a = Animator::new(p, defaults);
        a.update(0.1);
        assert!(close(x(&a), 3.0));
        a.set_bool(name_hash("go"), true);
        a.update(0.1);
        assert!(state_is(&a, 0, "b"));
        assert!(close(x(&a), expected), "write defaults {wd}: {}", x(&a));
    }
}

#[test]
fn speed_and_one_d_trees() {
    let mut tree = leaf(u32::MAX);
    tree.children = vec![1, 2];
    tree.thresholds = vec![0.0, 1.0];
    tree.param = name_hash("blend");
    let mut s = state("mix", vec![tree, leaf(0), leaf(1)], vec![]);
    s.speed = 2.0;
    let c = controller(
        vec![layer(0)],
        vec![machine(vec![s])],
        &[("blend", ParamKind::Float)],
        2,
    );
    let mut a = animator(c, vec![hold_x("lo", 0.0), hold_x("hi", 10.0)], vec![]);
    a.set_float(name_hash("blend"), 0.3);
    a.update(0.25);
    assert!(close(x(&a), 3.0), "{}", x(&a));
    assert!(close(a.layer_info(0).unwrap().normalized_time, 0.5));
}

#[test]
fn rotations_blend_to_unit_quaternions() {
    let rot = |name: &str, deg: f32| {
        let (s, c) = (deg.to_radians() / 2.0).sin_cos();
        let mut cl = clip(
            name,
            1.0,
            false,
            ROOT,
            [constant(0.0), constant(s), constant(0.0)],
        );
        cl.streamed.push(constant(c));
        cl.bindings = vec![binding(ROOT, ATTR_ROTATION)];
        cl
    };
    let mut t = to(1, vec![cond(ConditionMode::If, "go", 0.0)]);
    t.duration = 1.0;
    let sm = machine(vec![
        state("a", vec![leaf(0)], vec![t]),
        state("b", vec![leaf(1)], vec![]),
    ]);
    let c = controller(vec![layer(0)], vec![sm], &[("go", ParamKind::Bool)], 2);
    let mut a = animator(c, vec![rot("a", 0.0), rot("b", 90.0)], vec![]);
    a.set_bool(name_hash("go"), true);
    a.update(0.0);
    a.update(0.5);
    let p = a.program().clone();
    let at = p.slots[p.slot(ROOT, SlotKind::Rotation).unwrap()].offset;
    let q = &a.pose()[at..at + 4];
    let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(close(len, 1.0));
    // Halfway between 0° and 90° about y: 45°.
    assert!(close(q[1], (22.5f32).to_radians().sin()), "{q:?}");
}

#[test]
fn an_empty_state_lets_the_layers_below_through() {
    // Upper layer at weight 1 in an empty state that can go to a state
    // animating x: until it does, the base layer shows.
    let mut upper_state = state(
        "nothing",
        vec![],
        vec![to(1, vec![cond(ConditionMode::If, "go", 0.0)])],
    );
    upper_state.blend_tree_index = vec![];
    let upper = machine(vec![upper_state, state("hold", vec![leaf(1)], vec![])]);
    let c = controller(
        vec![layer(0), layer(1)],
        vec![machine(vec![state("base", vec![leaf(0)], vec![])]), upper],
        &[("go", ParamKind::Bool)],
        2,
    );
    let mut a = animator(c, vec![ramp_x("base", true), hold_x("hold", 9.0)], vec![]);
    a.update(0.5);
    assert!(close(x(&a), 0.5), "{}", x(&a));
    a.set_bool(name_hash("go"), true);
    a.update(0.1);
    assert!(close(x(&a), 9.0), "{}", x(&a));
}
