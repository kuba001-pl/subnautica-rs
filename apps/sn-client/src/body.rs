//! The player's body in the client (M9g4, `docs/DESIGN.md` § 4.3 "M9g
//! plan"): the main scene's `Player` hierarchy (spawned by `objects`), its
//! rig's base moved to the player's transform and the view model's turn
//! and bob every frame, and its animator given the parameters of the
//! game's empty-hand rules (`sn_sim::body`, run by `crate::player`). The
//! animator itself runs in `crate::animation` like every other rig.

use std::time::Instant;

use bevy::camera::visibility::{RenderLayers, ViewVisibility};
use bevy::prelude::*;
use sn_sim::body::AnimValue;
use sn_unity::name_hash;

use crate::animation::AnimatedRig;
use crate::objects::{PlayerBodyRig, ShadowsOnly};

/// Seconds between the body's status lines.
const LOG_EVERY: f32 = 10.0;

/// What `crate::player` hands the body each frame.
#[derive(Resource, Default)]
pub struct BodyDrive {
    /// The player exists (the body is hidden until then).
    pub shown: bool,
    /// The view model (`MainCameraControl.viewModel`) in the world: the
    /// player's transform, then the view model's local placement.
    pub view_model: Transform,
    /// This frame's parameters (taken when applied: triggers fire once).
    pub values: Vec<(&'static str, AnimValue)>,
    /// A cinematic's bools for the player's animator (M9g5e), by name hash.
    pub player_values: Vec<(u32, bool)>,
    /// A cinematic's bools for the drawn pod's animators (every rig of
    /// [`BodyDrive::pod_controller`]).
    pub pod_values: Vec<(u32, bool)>,
    /// The pod's controller name (`escape_pod_controller`).
    pub pod_controller: String,
    /// CPU time of this frame's rules (`sn_sim::body`), µs.
    pub rules_micros: f32,
}

/// Counts for the log.
#[derive(Default)]
pub struct BodyLog {
    since: f32,
    frames: u32,
    micros: f32,
    worst_micros: f32,
    /// Parameters the controller doesn't have.
    unknown: Vec<&'static str>,
}

/// Every frame, after `crate::player::update` and before
/// `crate::animation::animate`: place the rig and set its parameters.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    time: Res<Time>,
    mut drive: ResMut<BodyDrive>,
    mut rigs: Query<(&mut AnimatedRig, &mut Transform, &mut Visibility), With<PlayerBodyRig>>,
    mut others: Query<&mut AnimatedRig, Without<PlayerBodyRig>>,
    parts: Query<(&ViewVisibility, Option<&RenderLayers>, Has<ShadowsOnly>)>,
    globals: Query<&GlobalTransform>,
    locals: Query<&Transform, Without<PlayerBodyRig>>,
    mut log: Local<BodyLog>,
) {
    let start = Instant::now();
    let values = std::mem::take(&mut drive.values);
    let player_values = std::mem::take(&mut drive.player_values);
    let pod_values = std::mem::take(&mut drive.pod_values);
    if !pod_values.is_empty() {
        let mut pods = 0;
        for mut rig in &mut others {
            if rig.animator.program().controller.name != drive.pod_controller {
                continue;
            }
            pods += 1;
            for &(id, on) in &pod_values {
                rig.animator.set_bool(id, on);
            }
        }
        if pods == 0 {
            warn!(
                "body: no drawn {:?} animator for the hatch",
                drive.pod_controller
            );
        }
    }
    let mut rigs_seen = 0;
    let mut status = None;
    for (mut rig, mut transform, mut visibility) in &mut rigs {
        rigs_seen += 1;
        let want = if drive.shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != want {
            *visibility = want;
        }
        *transform = drive.view_model;
        for &(name, value) in &values {
            let id = name_hash(name);
            let known = match value {
                AnimValue::Float(x) => rig.animator.set_float(id, x as f32),
                AnimValue::Bool(b) => rig.animator.set_bool(id, b),
                AnimValue::Trigger => rig.animator.set_trigger(id),
            };
            if !known && !log.unknown.contains(&name) {
                log.unknown.push(name);
                warn!("body: parameter {name:?} is not in the player's controller");
            }
        }
        for &(id, on) in &player_values {
            rig.animator.set_bool(id, on);
        }
        if log.since + time.delta_secs() >= LOG_EVERY {
            // Drawn parts the camera sees, and shadows-only parts: those
            // seen by a view are seen by a light's shadow pass (the
            // camera's layer leaves them out).
            let (mut drawn, mut seen, mut only, mut only_seen, mut only_off_camera) =
                (0, 0, 0, 0, 0);
            for &e in &rig.parts {
                let Ok((v, layers, shadows_only)) = parts.get(e) else {
                    continue;
                };
                if shadows_only {
                    only += 1;
                    only_seen += usize::from(v.get());
                    only_off_camera += usize::from(
                        layers.is_some_and(|l| !l.intersects(&RenderLayers::default())),
                    );
                } else {
                    drawn += 1;
                    seen += usize::from(v.get());
                }
            }
            // Where the animated nodes are, in the view model's axes
            // (Bevy's: x right, y up, -z ahead).
            let base = GlobalTransform::from(*transform).affine().inverse();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for e in rig.slots.iter().flatten() {
                if let Ok(g) = globals.get(*e) {
                    let p = base.transform_point3(g.translation());
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
            // Rotations the animator moved more than 1° from the stored
            // pose, and the base layer's state.
            let (mut turned, mut rotations) = (0, 0);
            let program = rig.animator.program();
            for (k, slot) in program.slots.iter().enumerate() {
                if slot.kind != sn_anim::SlotKind::Rotation {
                    continue;
                }
                let (Some(Some(e)), Some(Some(n))) = (rig.slots.get(k), rig.desc.slot_nodes.get(k))
                else {
                    continue;
                };
                let (Ok(t), Some((_, stored))) =
                    (locals.get(*e), rig.desc.nodes.get(usize::from(*n)))
                else {
                    continue;
                };
                let [x, y, z, w] = stored.rotation;
                let q = Quat::from_xyzw(-x, -y, z, w).normalize();
                rotations += 1;
                turned += usize::from(t.rotation.angle_between(q) > 1f32.to_radians());
            }
            // Skin 0's joint matrices (global × inverse bind pose): all
            // equal means the mesh is drawn rigidly in its bind pose.
            let mut spread = 0.0f32;
            let mut with_slot = 0;
            if let Some(skin) = rig.desc.skins.first() {
                let mut first = None;
                for (k, &j) in skin.joints.iter().enumerate() {
                    // Joint entity: the slot entity whose node is `j`.
                    let e = rig
                        .desc
                        .slot_nodes
                        .iter()
                        .position(|n| *n == Some(j))
                        .and_then(|i| rig.slots.get(i).copied().flatten());
                    let (Some(e), Some(ib)) = (e, skin.inverse_bindposes.get(k)) else {
                        continue;
                    };
                    with_slot += 1;
                    let Ok(g) = globals.get(e) else { continue };
                    let m = g.to_matrix() * *ib;
                    let p = m.transform_point3(Vec3::new(0.0, 1.0, 0.0));
                    match first {
                        None => first = Some(p),
                        Some(f) => spread = spread.max((p - f).length()),
                    }
                }
            }
            let base_state = rig
                .animator
                .layer_info(0)
                .and_then(|i| i.state)
                .and_then(|h| program.controller.name_of(h).map(str::to_string))
                .unwrap_or_else(|| "?".into());
            status = Some(format!(
                "skin 0: {} joints with a slot, joint matrices spread {spread:.3} m; base layer {base_state:?}, {turned} of {rotations} animated rotations away from the stored pose; {drawn} parts drawn ({seen} in a view), {only} shadows only ({only_off_camera} off the camera's layer, {only_seen} in a light's shadow pass); animated nodes from ({:.2}, {:.2}, {:.2}) to ({:.2}, {:.2}, {:.2}) m of the view model",
                with_slot, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
            ));
        }
    }
    let micros = start.elapsed().as_secs_f32() * 1e6 + drive.rules_micros;
    if rigs_seen == 0 {
        return;
    }
    log.frames += 1;
    log.micros += micros;
    log.worst_micros = log.worst_micros.max(micros);
    log.since += time.delta_secs();
    if let Some(parts) = status {
        info!(
            "body: {parts}; rules and drive {:.1} µs per frame (worst {:.1}), shown {}",
            log.micros / log.frames.max(1) as f32,
            log.worst_micros,
            drive.shown
        );
        log.since = 0.0;
        log.frames = 0;
        log.micros = 0.0;
        log.worst_micros = 0.0;
    }
}
