//! M9g3: the player's body in the scripted runs (`walk`, `dive`): the
//! game's empty-hand rules (`sn_sim::body`) set the player's animator
//! (`sn_anim`) each physics step; the states each layer passes through
//! are collected per phase of the script and checked against the ones
//! written in `docs/DESIGN.md` ("M9g3 expected states") before the run.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;

use sn_anim::{Animator, Program, SlotKind};
use sn_assets::{Assets, PosedNode};
use sn_sim::body::{AnimValue, Body, BodyFrame, BodyParams};
use sn_sim::look::Look;
use sn_sim::{Pose, Q, V3};
use sn_unity::{AnimatorController, name_hash};

use crate::Result;

/// The layers whose states are collected: "Base Modes", "Death" and
/// "Cinematics" (M9g5d).
const WATCHED: [&str; 3] = ["Base Modes", "Death", "Cinematics"];

pub(crate) struct BodyRun {
    pub(crate) params: BodyParams,
    /// `MainCameraControl.minimumY` / `maximumY`.
    look_limits: (f64, f64),
    body: Body,
    animator: Animator,
    program: Arc<Program>,
    controller: Arc<AnimatorController>,
    /// Watched layer index → its name.
    layers: Vec<(usize, String)>,
    /// The script's current phase.
    phase: String,
    /// Per phase (in order): per watched layer, the states it was in.
    visited: Vec<(String, BTreeMap<String, BTreeSet<String>>)>,
    /// Transitions started, with the time, for the log.
    pub(crate) transitions: Vec<(f64, String, String, String)>,
    nans: usize,
    worst_q: f32,
    us: Vec<f64>,
    /// Parameters set that the controller doesn't have.
    unknown: BTreeSet<&'static str>,
    /// `Player.camAnchor` and the death camera's bone (M9g5d).
    cam_anchor: PosedNode,
    head_camera: PosedNode,
}

impl BodyRun {
    pub(crate) fn new(assets: &Assets, ocean_level: f32) -> Result<BodyRun> {
        let body = assets.player_body()?;
        let params = body.body_params(ocean_level);
        let look_limits = (
            f64::from(body.camera.minimum_y),
            f64::from(body.camera.maximum_y),
        );
        let mut cache = HashMap::new();
        let set = assets.animation_set(&body.controller, &mut cache)?;
        if !set.errors.is_empty() {
            return Err(format!("player animation: {:?}", set.errors));
        }
        let controller = Arc::new(set.controller.clone());
        let program = Arc::new(Program::new(controller.clone(), &set.clips));
        let binding = body
            .prefab
            .bind_animator(body.animator_node, &program, &|_| Vec::new());
        if binding.missing > 0 {
            return Err(format!(
                "player animator: {} slots not in the hierarchy",
                binding.missing
            ));
        }
        let posed = |node: usize, what: &str| {
            body.prefab
                .posed_node(node, &program, &binding)
                .ok_or_else(|| format!("player: {what} not in the hierarchy"))
        };
        let cam_anchor = posed(body.cam_anchor_node, "camAnchor")?;
        let head_camera = posed(body.head_camera_node, "headCameraBone")?;
        // The posed chains start below the player's root and pass through
        // the view model, which must be stored at the root's origin (the
        // client and the rules put the view model there each frame).
        let vm = &body.prefab.nodes[body.view_model_node];
        if vm.parent != Some(0)
            || vm.local.position != [0.0; 3]
            || vm.local.rotation != [0.0, 0.0, 0.0, 1.0]
        {
            return Err("player: the view model is not at the player's origin".into());
        }
        let animator = Animator::new(program.clone(), binding.defaults);
        let layers = controller
            .layers
            .iter()
            .enumerate()
            .filter_map(|(i, l)| {
                let n = controller.name_of(l.binding)?.to_string();
                WATCHED.contains(&n.as_str()).then_some((i, n))
            })
            .collect::<Vec<_>>();
        if layers.len() != WATCHED.len() {
            return Err(format!("player controller: watched layers {layers:?}"));
        }
        println!(
            "body: {} slots, eye at rest ({:.3}, {:.3}, {:.3}) from the player's transform, smoothing {} / {}",
            program.slots.len(),
            params.camera_up_position.x + params.camera_offset_position.x,
            params.camera_up_position.y + params.camera_offset_position.y,
            params.camera_up_position.z + params.camera_offset_position.z,
            params.smooth_speed_under_water,
            params.smooth_speed_above_water
        );
        Ok(BodyRun {
            body: Body::new(&params),
            params,
            look_limits,
            animator,
            program,
            controller,
            layers,
            phase: "start".into(),
            visited: Vec::new(),
            transitions: Vec::new(),
            nans: 0,
            worst_q: 0.0,
            us: Vec::new(),
            unknown: BTreeSet::new(),
            cam_anchor,
            head_camera,
        })
    }

    fn name(&self, hash: u32) -> String {
        self.controller
            .name_of(hash)
            .map_or_else(|| format!("#{hash}"), str::to_string)
    }

    pub(crate) fn set_phase(&mut self, phase: &str) {
        self.phase = phase.to_string();
    }

    /// The main camera in the world (M9g5d): `camRoot` on the player's
    /// transform, or at `root` (a cinematic), or on the head camera bone
    /// after a death.
    pub(crate) fn camera(&self, player: Pose, root: Option<Pose>) -> Pose {
        let root = if self.body.head_camera {
            Some(self.node_world(&self.head_camera, player))
        } else {
            root
        };
        self.body.camera(&self.params, player, root)
    }

    /// `camRoot` where the rig puts it now.
    pub(crate) fn camera_root(&self, player: Pose) -> Pose {
        self.body.camera_root(player)
    }

    /// `Player.camAnchor` in the world now.
    pub(crate) fn cam_anchor(&self, player: Pose) -> Pose {
        self.node_world(&self.cam_anchor, player)
    }

    /// A node of the player's hierarchy in the world: the view model's
    /// place, then the node's chain for the animator's pose.
    fn node_world(&self, node: &PosedNode, player: Pose) -> Pose {
        let t = node.in_prefab(self.animator.pose());
        self.body.view_model(player).then(&Pose::new(
            V3::from_f32(t.position),
            Q::from_f32(t.rotation).normalized(),
        ))
    }

    /// Sets a bool of the player's animator (a cinematic's
    /// `playerViewAnimationName`); false if the controller lacks it.
    pub(crate) fn set_bool(&mut self, id: u32, v: bool) -> bool {
        self.animator.set_bool(id, v)
    }

    /// `MainCameraControl.minimumY` / `maximumY`.
    pub(crate) fn look_limits(&self) -> (f64, f64) {
        self.look_limits
    }

    pub(crate) fn respawned(&mut self) {
        self.body.respawned();
    }

    pub(crate) fn jumped(&mut self, time: f64) {
        self.body.jumped(time);
    }

    pub(crate) fn landed(&mut self, impact_y: f64) {
        self.body.landed(impact_y);
    }

    pub(crate) fn died(&mut self) {
        self.body.died();
    }

    /// One physics step of the body and its animator.
    pub(crate) fn step(
        &mut self,
        frame: &BodyFrame,
        cinematic: bool,
        obstacle: &mut dyn FnMut(V3, V3, f64) -> bool,
    ) {
        let t = Instant::now();
        self.body
            .fixed_step(frame.time, frame.underwater, frame.grounded, cinematic);
        let values = self.body.update(&self.params, frame, obstacle);
        for (name, v) in values {
            let id = name_hash(name);
            let known = match v {
                AnimValue::Float(x) => self.animator.set_float(id, x as f32),
                AnimValue::Bool(b) => self.animator.set_bool(id, b),
                AnimValue::Trigger => self.animator.set_trigger(id),
            };
            if !known {
                self.unknown.insert(name);
            }
        }
        self.animator.update(frame.dt as f32);
        self.us.push(t.elapsed().as_secs_f64() * 1e6);

        let pose = self.animator.pose();
        self.nans += pose.iter().filter(|v| !v.is_finite()).count();
        for s in self
            .program
            .slots
            .iter()
            .filter(|s| s.kind == SlotKind::Rotation)
        {
            let q = &pose[s.offset..s.offset + 4];
            let len = q.iter().map(|v| v * v).sum::<f32>().sqrt();
            self.worst_q = self.worst_q.max((len - 1.0).abs());
        }
        let started = self.animator.started.clone();
        for (layer, from, to) in started {
            if let Some((_, lname)) = self.layers.iter().find(|(i, _)| *i == layer) {
                self.transitions
                    .push((frame.time, lname.clone(), self.name(from), self.name(to)));
            }
        }
        if self.visited.last().is_none_or(|(p, _)| *p != self.phase) {
            self.visited.push((self.phase.clone(), BTreeMap::new()));
        }
        let mut now = Vec::new();
        for (i, lname) in &self.layers {
            if let Some(info) = self.animator.layer_info(*i) {
                for s in info.state.into_iter().chain(info.next.map(|n| n.0)) {
                    now.push((lname.clone(), self.name(s)));
                }
            }
        }
        if let Some((_, states)) = self.visited.last_mut() {
            for (l, s) in now {
                states.entry(l).or_default().insert(s);
            }
        }
    }

    /// The states layer `layer` was in during phases whose name starts
    /// with `phase`.
    pub(crate) fn states(&self, phase: &str, layer: &str) -> BTreeSet<String> {
        self.states_where(|p| p.starts_with(phase), layer)
    }

    /// The states layer `layer` was in during the phases named exactly
    /// `phase` (a phase name may come back, e.g. "in the pod" after the
    /// respawn).
    pub(crate) fn states_exact(&self, phase: &str, layer: &str) -> BTreeSet<String> {
        self.states_where(|p| p == phase, layer)
    }

    fn states_where(&self, keep: impl Fn(&str) -> bool, layer: &str) -> BTreeSet<String> {
        self.visited
            .iter()
            .filter(|(p, _)| keep(p))
            .filter_map(|(_, m)| m.get(layer))
            .flatten()
            .cloned()
            .collect()
    }

    /// Prints the transitions, the states per phase and the checks of the
    /// pose; false if the pose had NaNs or long quaternions or a set
    /// parameter is not in the controller.
    pub(crate) fn report(&self) -> bool {
        println!("body: transitions of the watched layers:");
        for (t, l, from, to) in &self.transitions {
            println!("  t {t:>6.2} s {l:?}: {from} → {to}");
        }
        println!("body: states per phase:");
        for (phase, layers) in &self.visited {
            let list: Vec<String> = layers
                .iter()
                .map(|(l, s)| format!("{l} {:?}", s.iter().collect::<Vec<_>>()))
                .collect();
            println!("  {phase}: {}", list.join("; "));
        }
        let mut us = self.us.clone();
        us.sort_by(f64::total_cmp);
        println!(
            "body: {} steps; NaNs {}; worst |q| − 1 {:.2e}; parameters not in the controller {:?}; cost per step mean {:.1} µs, p99 {:.1} µs",
            us.len(),
            self.nans,
            self.worst_q,
            self.unknown,
            us.iter().sum::<f64>() / us.len().max(1) as f64,
            crate::swim::percentile(&us, 0.99)
        );
        self.nans == 0 && self.worst_q < 1e-4 && self.unknown.is_empty()
    }
}

impl BodyRun {
    /// `Look` from the scripts' input angles (radians, pitch positive
    /// down), clamped to the game's limits.
    pub(crate) fn look(&self, yaw: f64, pitch: f64) -> Look {
        Look {
            rotation_x: yaw.to_degrees(),
            rotation_y: (-pitch.to_degrees()).clamp(self.look_limits.0, self.look_limits.1),
        }
    }
}
