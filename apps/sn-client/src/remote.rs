//! Other players' bodies (M10h2, `docs/DESIGN.md` § M10 plan "Remote
//! bodies"): for each other player who is in the world (joined and
//! `Ready`), the player's own body prefab (`crate::objects`), its rig at
//! the drawn state's transform with the view model turned by the look's
//! yaw (as `crate::player` places the own one), its animator given the
//! values the owner's rules computed (`sn_sim::body` on the owner's
//! machine, sent in every state) and the owner's animator triggers. The
//! head is drawn (only the own first-person head casts shadows only).
//! Removed when the player leaves.
//!
//! Hatch cinematics (M10h3): the owner sends its cinematic's signals at
//! their steps (`CinematicPrepare` / `CinematicPlay`, after `HatchUsed`
//! and before `CinematicEnded`); when the drawn tick reaches them, the
//! drawn pod's animators get the hatch's `prepare` / `play` bools and the
//! remote body's animator the player's bool, so the doors open and the
//! body climbs as the owner's did. The own simulated pod animator (the one
//! that places the own player in a cinematic) does not get them.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use sn_protocol::anim::TRIGGERS;
use sn_protocol::{PlayerEventKind, PlayerId, WorldEventKind, quant};
use sn_sim::{Pose, Q, V3};
use sn_unity::name_hash;

use crate::animation::AnimatedRig;
use crate::body::BodyDrive;
use crate::objects::{ObjectStreamer, RemoteBodyRig, ShadowsOnly};
use crate::player::{PlayerSim, pose_to_bevy};
use crate::session::Session;

/// Seconds between the status lines.
const LOG_EVERY: f32 = 10.0;

#[derive(Default)]
pub struct RemoteLog {
    since: f32,
    /// Bodies whose head is drawn.
    heads: Vec<u16>,
    /// Where each rig was put last frame (Bevy), to check the drawn
    /// transform after propagation.
    placed: Vec<(u16, Vec3)>,
    /// Largest distance between the placed and the drawn rig base since
    /// the last line, and frames checked.
    worst: f32,
    checked: u32,
    triggers: u32,
    /// The hatch each remote player's cinematic uses, from the due
    /// events (the shared world state is ahead of the drawn tick).
    hatches: Vec<(u16, u16)>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn update(
    time: Res<Time>,
    mut commands: Commands,
    session: Res<Session>,
    mut streamer: ResMut<ObjectStreamer>,
    sim: Option<Res<PlayerSim>>,
    mut rigs: Query<(
        &RemoteBodyRig,
        &mut AnimatedRig,
        &mut Transform,
        &mut Visibility,
        &GlobalTransform,
    )>,
    shadows_only: Query<(), With<ShadowsOnly>>,
    mut drive: ResMut<BodyDrive>,
    mut log: Local<RemoteLog>,
) {
    // Who has a body: the other players in the world.
    let remotes = session.remotes();
    let wanted: Vec<u16> = remotes.iter().filter(|r| r.2).map(|r| r.0.0).collect();
    let have = streamer.remote_body_ids();
    for &id in &wanted {
        if !have.contains(&id) {
            streamer.want_remote_body(id);
        }
    }
    for id in have {
        if !wanted.contains(&id) {
            for e in streamer.remove_remote_body(id) {
                commands.entity(e).try_despawn();
            }
            log.heads.retain(|&h| h != id);
            log.placed.retain(|p| p.0 != id);
            info!("remote: player {id}'s body removed");
        }
    }

    // The owners' animator triggers and cinematic signals that came due.
    let mut triggers: Vec<(u16, u32)> = Vec::new();
    let mut body_bools: Vec<(u16, u32, bool)> = Vec::new();
    for e in &session.due {
        let WorldEventKind::Player { player, event } = &e.kind else {
            continue;
        };
        let id = player.0;
        match *event {
            PlayerEventKind::AnimatorTrigger { trigger } => {
                if let Some(name) = TRIGGERS.get(usize::from(trigger)) {
                    triggers.push((id, name_hash(name)));
                }
            }
            PlayerEventKind::HatchUsed { hatch } => {
                log.hatches.retain(|h| h.0 != id);
                log.hatches.push((id, hatch));
            }
            PlayerEventKind::CinematicEnded => log.hatches.retain(|h| h.0 != id),
            PlayerEventKind::CinematicPrepare { on } | PlayerEventKind::CinematicPlay { on } => {
                let play = matches!(event, PlayerEventKind::CinematicPlay { .. });
                let hatch = log.hatches.iter().find(|h| h.0 == id).map(|h| h.1);
                let names = hatch.and_then(|h| sim.as_ref()?.hatch_names(usize::from(h)));
                let Some(names) = names else {
                    warn!("remote: player {id}'s cinematic signal with no known hatch");
                    continue;
                };
                if play {
                    drive.pod_values.push((names.play, on));
                    if let Some(p) = names.player {
                        body_bools.push((id, p, on));
                    }
                } else if let Some(p) = names.prepare {
                    drive.pod_values.push((p, on));
                }
                info!(
                    "remote: player {id}'s hatch {} {} {on} at tick {}",
                    hatch.unwrap_or(0),
                    if play { "play" } else { "prepare" },
                    e.tick.0
                );
            }
            PlayerEventKind::Died { .. } | PlayerEventKind::Respawned => {}
        }
    }

    let skin = sim
        .as_ref()
        .and_then(|s| s.body_params())
        .map_or(0.0, |p| p.skin);
    let draw = session.draw_tick();
    let mut lines = Vec::new();
    let log = &mut *log;
    for (rig_id, mut rig, mut transform, mut visibility, global) in &mut rigs {
        let id = rig_id.0;
        // Removed this frame (despawned at the frame's end).
        if !wanted.contains(&id) {
            continue;
        }
        // Last frame's placement against where it was drawn.
        if let Some(&(_, at)) = log.placed.iter().find(|p| p.0 == id) {
            log.worst = log.worst.max(at.distance(global.translation()));
            log.checked += 1;
        }
        let sample = draw.and_then(|d| session.sample(PlayerId(id), d));
        let Some(s) = sample else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        if *visibility != Visibility::Inherited {
            *visibility = Visibility::Inherited;
        }
        // The player's transform, then the view model's (as
        // `sn_sim::body::Body::view_model`: `camRoot`'s height, the look's
        // yaw).
        let rotation = s.base.rotation.map_or(Q::IDENTITY, |r| {
            let c = |i: usize| quant::fixed_back(r[i], quant::QUATERNION_SCALE);
            Q {
                x: c(0),
                y: c(1),
                z: c(2),
                w: c(3),
            }
            .normalized()
        });
        let player = Pose::new(
            V3::new(s.position[0], s.position[1], s.position[2]),
            rotation,
        );
        let bob = s
            .base
            .anim
            .floats()
            .find(|(n, _)| *n == "verticalOffset")
            .map_or(0.0, |(_, v)| v);
        let view_model = player.then(&Pose::new(
            V3::new(0.0, -skin - bob, 0.0),
            Q::euler(0.0, s.yaw, 0.0),
        ));
        *transform = pose_to_bevy(&view_model);
        match log.placed.iter_mut().find(|p| p.0 == id) {
            Some(p) => p.1 = transform.translation,
            None => log.placed.push((id, transform.translation)),
        }
        for (name, v) in s.base.anim.floats() {
            rig.animator.set_float(name_hash(name), v as f32);
        }
        for (name, on) in s.base.anim.bools() {
            rig.animator.set_bool(name_hash(name), on);
        }
        for &(_, b, on) in body_bools.iter().filter(|b| b.0 == id) {
            rig.animator.set_bool(b, on);
        }
        for &(_, t) in triggers.iter().filter(|t| t.0 == id) {
            rig.animator.set_trigger(t);
            log.triggers += 1;
        }
        // The head: drawn for other players.
        if !log.heads.contains(&id) {
            let mut shown = 0;
            for &e in &rig.parts {
                if shadows_only.get(e).is_ok() {
                    commands.entity(e).remove::<(ShadowsOnly, RenderLayers)>();
                    shown += 1;
                }
            }
            log.heads.push(id);
            info!("remote: player {id}'s body placed; {shown} shadows-only parts (the head) drawn");
        }
        if log.since + time.delta_secs() >= LOG_EVERY {
            lines.push(format!(
                "player {id} at ({:.2}, {:.2}, {:.2}) {:?}, look ({:.1}, {:.1}), life {:?}, cinematic {}",
                s.position[0], s.position[1], s.position[2], s.kind, s.yaw, s.pitch, s.base.life, s.base.cinematic
            ));
        }
    }
    log.since += time.delta_secs();
    if log.since >= LOG_EVERY {
        if !lines.is_empty() || log.checked > 0 {
            info!(
                "remote: {} bodies; drawn rig base vs placed: max {:.6} m over {} checks; {} triggers; {}",
                log.placed.len(),
                log.worst,
                log.checked,
                log.triggers,
                lines.join("; ")
            );
        }
        log.since = 0.0;
        log.worst = 0.0;
        log.checked = 0;
    }
}
