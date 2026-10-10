//! Cinematics (M9g5c, `docs/DESIGN.md` § 4.3 "M9g5 plan"): the lifepod's
//! hatches move the player along a node the pod's animator moves, as
//! `PlayerCinematicController` does; `MainCameraControl.cinematicMode`
//! folds the look into the player's rotation and back; `Player.UpdateRotation`
//! eases what is left of a tilt. Our own code after reading the game's;
//! facts in `docs/formats/gameplay.md` § The player's body.
//!
//! The caller runs the animators (the pod's, which moves the node, and
//! the player's, which moves the camera's anchor) and gives their nodes'
//! world placements each frame; [`Cinematic`] says where the player and
//! the camera are and which animator parameters to set.

use crate::look::Look;
use crate::math::{Pose, Q, V3, lerp_angle};
use crate::player::{Hatches, Player, PlayerParams};

/// One controller's numbers (`PlayerCinematicController`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CinematicParams {
    /// `interpolationTime` / `interpolationTimeOut`, seconds.
    pub interpolation_in: f64,
    pub interpolation_out: f64,
    /// `endTransform` when the game uses it (`None`: no end point, or a
    /// VR-only one: the player stays where the animation leaves it).
    pub end: Option<Pose>,
}

/// What the caller does with the animators and the trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// The controller's `interpolateAnimParam` (`prepare_…`) on the pod's
    /// animator.
    Prepare(bool),
    /// `animState`: the pod's `animParam` and the player's
    /// `playerViewAnimationName`.
    Play(bool),
    /// The end event reached the trigger (`OnPlayerCinematicModeEnd` on
    /// the inform object): its `onCinematicEnd` calls run now.
    TriggerEnd,
    /// `EndCinematicMode`: controls back, the look taken back from the
    /// player's rotation ([`rotation_into_look`]).
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    In,
    Update,
    Out,
}

/// Where a frame leaves the player and the camera.
#[derive(Clone, Debug, PartialEq)]
pub struct CinematicFrame {
    /// The player's transform.
    pub player: Pose,
    /// `camRoot` in the world (`None`: where the rig puts it on the player,
    /// with no look of its own).
    pub camera_root: Option<Pose>,
    pub signals: Vec<Signal>,
}

/// A running cinematic.
#[derive(Clone, Debug, PartialEq)]
pub struct Cinematic {
    pub params: CinematicParams,
    pub phase: Phase,
    /// `timeStateChanged`.
    since: f64,
    player_from: Pose,
    /// `cameraPosition`: the camera in the anchor's space at the start.
    camera_offset: V3,
    camera_from: Q,
    /// Still running (false after `EndCinematicMode`).
    pub active: bool,
}

impl Cinematic {
    /// `StartCinematicMode` at `time`: the player's transform and
    /// `camRoot` as they are (the look already folded in,
    /// [`look_into_rotation`]), and the camera's anchor. Returns the
    /// cinematic and its first signals.
    pub fn start(
        params: CinematicParams,
        time: f64,
        player: Pose,
        camera_root: Pose,
        cam_anchor: Pose,
    ) -> (Cinematic, Vec<Signal>) {
        let c = Cinematic {
            params,
            phase: Phase::In,
            since: time,
            player_from: player,
            camera_offset: cam_anchor.inverse_transform_point(camera_root.position),
            camera_from: camera_root.rotation,
            active: true,
        };
        (c, vec![Signal::Prepare(true)])
    }

    /// `ManagedLateUpdate` at `time`: `animated` is the controller's
    /// `animatedTransform`, `cam_anchor` is `Player.camAnchor`, both in
    /// the world after this frame's animation. `player` is the player's
    /// transform before it.
    pub fn late_update(
        &mut self,
        time: f64,
        player: Pose,
        animated: Pose,
        cam_anchor: Pose,
    ) -> CinematicFrame {
        let mut frame = CinematicFrame {
            player,
            camera_root: None,
            signals: Vec::new(),
        };
        if !self.active {
            return frame;
        }
        let elapsed = time - self.since;
        match self.phase {
            Phase::In => {
                let t = fraction(elapsed, self.params.interpolation_in);
                frame.player = Pose::new(
                    V3::lerp(self.player_from.position, animated.position, t),
                    Q::slerp(self.player_from.rotation, animated.rotation, t),
                );
                frame.camera_root = Some(Pose::new(
                    cam_anchor.position + self.camera_offset * (1.0 - t),
                    Q::slerp(self.camera_from, animated.rotation, t),
                ));
                if t >= 1.0 {
                    self.phase = Phase::Update;
                    self.since = time;
                    frame.signals.push(Signal::Play(true));
                    frame.signals.push(Signal::Prepare(false));
                }
            }
            Phase::Update => {
                frame.player = animated;
                frame.camera_root = Some(Pose::new(cam_anchor.position, animated.rotation));
            }
            Phase::Out => {
                let t = fraction(elapsed, self.params.interpolation_out);
                if let Some(end) = self.params.end {
                    frame.player = Pose::new(
                        V3::lerp(self.player_from.position, end.position, t),
                        Q::slerp(self.player_from.rotation, end.rotation, t),
                    );
                }
                if t >= 1.0 {
                    self.active = false;
                    frame.signals.push(Signal::Ended);
                }
            }
        }
        frame
    }

    /// `OnPlayerCinematicModeEnd` at `time` (the pod clip's end event).
    /// Ignored when not running (or already moving out with no change:
    /// the game's second call in a frame re-puts the player on the node
    /// and restarts the move out; ported as is).
    pub fn end_event(&mut self, time: f64, animated: Pose, cam_anchor: Pose) -> CinematicFrame {
        let mut frame = CinematicFrame {
            player: animated,
            camera_root: Some(Pose::new(cam_anchor.position, animated.rotation)),
            signals: vec![Signal::Play(false)],
        };
        if !self.active {
            frame.signals.clear();
            frame.camera_root = None;
            return frame;
        }
        if self.params.end.is_some() {
            self.phase = Phase::Out;
            self.since = time;
            self.player_from = animated;
        } else {
            self.active = false;
            frame.signals.push(Signal::Ended);
        }
        // The inform object hears of it after the state change; `Ended`
        // (controls back) comes first in the no-end-point case, as
        // `EndCinematicMode` runs before the `SendMessage`.
        frame.signals.push(Signal::TriggerEnd);
        frame
    }
}

/// A lifepod hatch in use: its trigger and its cinematic.
#[derive(Clone, Debug, PartialEq)]
pub struct HatchRun {
    pub trigger: usize,
    pub cinematic: Cinematic,
}

impl Hatches {
    /// Using trigger `i` at `time` (`CinematicModeTriggerBase.StartCinematicMode`):
    /// nothing if it is inactive or a cinematic runs. The look goes into
    /// the player's rotation and the controller stops; a boarding trigger
    /// puts the player in the pod now (`CinematicEnter` at the start).
    /// `camera_root` and `cam_anchor`: `camRoot` and `Player.camAnchor` in
    /// the world now. Returns the run and its first signals.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &self,
        i: usize,
        time: f64,
        player: &mut Player,
        look: &mut Look,
        params: &PlayerParams,
        camera_root: Pose,
        cam_anchor: Pose,
    ) -> Option<(HatchRun, Vec<Signal>)> {
        let t = self.triggers.get(i)?;
        if !t.active || player.cinematic {
            return None;
        }
        player.rotation = look_into_rotation(look);
        *look = Look::default();
        let in_pod = t.enters.then_some(true);
        player.teleport(params, player.position, in_pod);
        player.cinematic = true;
        let from = Pose::new(player.position, player.rotation);
        let (cinematic, signals) =
            Cinematic::start(t.cinematic, time, from, camera_root, cam_anchor);
        Some((
            HatchRun {
                trigger: i,
                cinematic,
            },
            signals,
        ))
    }

    /// The trigger's `onCinematicEnd` ([`Signal::TriggerEnd`]): a leaving
    /// trigger takes the player out of the pod (`CinematicExit`); a
    /// first-use trigger hands over to its normal twin
    /// (`EscapePodFirstUseCinematicsController`). Returns the triggers
    /// switched on or off.
    pub fn finish(&mut self, i: usize, player: &mut Player) -> Vec<(usize, bool)> {
        if self.triggers.get(i).is_some_and(|t| t.exits) {
            player.in_pod = false;
        }
        let mut switched = Vec::new();
        if let Some(&(normal, first)) = self.first_use.iter().find(|p| p.1 == i) {
            for (k, on) in [(first, false), (normal, true)] {
                if let Some(tr) = self.triggers.get_mut(k) {
                    tr.active = on;
                    switched.push((k, on));
                }
            }
        }
        switched
    }
}

/// Puts a cinematic frame on the player: its transform; at
/// [`Signal::Ended`] the controller comes back (velocity 0, the motor's
/// height at once) and the look is taken back from the rotation
/// (`minimum_y` / `maximum_y`: the look's pitch limits).
pub fn apply(
    frame: &CinematicFrame,
    player: &mut Player,
    look: &mut Look,
    params: &PlayerParams,
    minimum_y: f64,
    maximum_y: f64,
) {
    player.position = frame.player.position;
    player.rotation = frame.player.rotation;
    if frame.signals.contains(&Signal::Ended) {
        let (back, rest) = rotation_into_look(player.rotation, minimum_y, maximum_y);
        *look = back;
        player.rotation = rest;
        player.cinematic = false;
        player.teleport(params, player.position, None);
        player.force_controller_size(params);
    }
}

/// `Mathf.Clamp01(elapsed / time)`, 1 for a zero time.
fn fraction(elapsed: f64, time: f64) -> f64 {
    if time > 0.0 {
        (elapsed / time).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// `MainCameraControl.cinematicMode = true`: the player's transform gets
/// the look (`Euler(−rotationY, rotationX, 0)`), the look is zeroed.
pub fn look_into_rotation(look: &Look) -> Q {
    Q::euler(-look.rotation_y, look.rotation_x, 0.0)
}

/// `MainCameraControl.cinematicMode = false`: the look taken back from the
/// player's rotation (yaw; pitch clamped to `minimum_y`…`maximum_y`); the
/// player keeps the rest of the pitch and the roll, yaw 0.
pub fn rotation_into_look(rotation: Q, minimum_y: f64, maximum_y: f64) -> (Look, Q) {
    let e = rotation.to_euler();
    let mut rotation_y = -e.x;
    if rotation_y > 180.0 {
        rotation_y -= 360.0;
    }
    if rotation_y < -180.0 {
        rotation_y += 360.0;
    }
    let rotation_y = rotation_y.clamp(minimum_y, maximum_y);
    let look = Look {
        rotation_x: e.y,
        rotation_y,
    };
    (look, Q::euler(e.x + rotation_y, 0.0, e.z))
}

/// `Player.UpdateRotation` (not in a cinematic): a pitch or roll left on
/// the player's transform eases back to level, `LerpEuler` at 10 × `dt`.
pub fn ease_tilt(rotation: Q, dt: f64) -> Q {
    let e = rotation.to_euler();
    if e.x == 0.0 && e.z == 0.0 {
        return rotation;
    }
    let k = dt * 10.0;
    Q::euler(lerp_angle(e.x, 0.0, k), e.y, lerp_angle(e.z, 0.0, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        (a - b).length() < 1e-9
    }

    fn pose(x: f64, y: f64, z: f64, yaw: f64) -> Pose {
        Pose::new(V3::new(x, y, z), Q::euler(0.0, yaw, 0.0))
    }

    fn params(end: Option<Pose>) -> CinematicParams {
        CinematicParams {
            interpolation_in: 0.25,
            interpolation_out: 0.25,
            end,
        }
    }

    #[test]
    fn moves_in_then_follows_the_node() {
        let start = pose(0.0, 0.0, 0.0, 0.0);
        let node = pose(2.0, 0.0, 0.0, 90.0);
        let anchor = pose(2.0, 1.5, 0.0, 90.0);
        let (mut c, s) = Cinematic::start(params(None), 10.0, start, start, anchor);
        assert_eq!(s, vec![Signal::Prepare(true)]);
        // The frame it starts: still where it was.
        let f = c.late_update(10.0, start, node, anchor);
        assert!(close(f.player.position, V3::ZERO) && f.signals.is_empty());
        // Halfway through the move in.
        let f = c.late_update(10.125, start, node, anchor);
        assert!(close(f.player.position, V3::new(1.0, 0.0, 0.0)));
        assert!((f.player.rotation.to_euler().y - 45.0).abs() < 1e-9);
        // In: the parameters switch when it arrives.
        let f = c.late_update(10.25, start, node, anchor);
        assert_eq!(f.signals, vec![Signal::Play(true), Signal::Prepare(false)]);
        assert_eq!(c.phase, Phase::Update);
        // Then on the node, the camera on the anchor, as the node moves.
        let moved = pose(3.0, 1.0, 0.0, 120.0);
        let f = c.late_update(11.0, f.player, moved, anchor);
        assert_eq!(f.player, moved);
        assert_eq!(
            f.camera_root,
            Some(Pose::new(anchor.position, moved.rotation))
        );
    }

    #[test]
    fn without_an_end_point_it_stops_on_the_node() {
        let start = pose(0.0, 0.0, 0.0, 0.0);
        let node = pose(0.0, 2.0, 0.0, 0.0);
        let (mut c, _) = Cinematic::start(params(None), 0.0, start, start, node);
        c.late_update(0.3, start, node, node);
        let last = pose(0.0, 5.0, 1.0, 30.0);
        let f = c.end_event(2.0, last, last);
        assert_eq!(f.player, last);
        assert_eq!(
            f.signals,
            vec![Signal::Play(false), Signal::Ended, Signal::TriggerEnd]
        );
        assert!(!c.active);
        // A second end event in the same frame (two layers): nothing.
        let f = c.end_event(2.0, last, last);
        assert!(f.signals.is_empty());
    }

    #[test]
    fn with_an_end_point_it_moves_out() {
        let start = pose(0.0, 0.0, 0.0, 0.0);
        let node = pose(0.0, 2.0, 0.0, 0.0);
        let end = pose(4.0, 2.0, 0.0, 0.0);
        let (mut c, _) = Cinematic::start(params(Some(end)), 0.0, start, start, node);
        c.late_update(0.3, start, node, node);
        let f = c.end_event(2.0, node, node);
        assert_eq!(f.signals, vec![Signal::Play(false), Signal::TriggerEnd]);
        assert_eq!(c.phase, Phase::Out);
        let f = c.late_update(2.125, node, node, node);
        assert!(close(f.player.position, V3::new(2.0, 2.0, 0.0)));
        assert!(f.camera_root.is_none());
        let f = c.late_update(2.25, f.player, node, node);
        assert!(close(f.player.position, end.position));
        assert_eq!(f.signals, vec![Signal::Ended]);
        assert!(!c.active);
    }

    #[test]
    fn a_hatch_runs_its_cinematic_and_hands_over() {
        use crate::player::HatchTrigger;
        let p = crate::player::tests::params();
        let node_start = pose(0.0, 2.0, 0.0, 180.0);
        let out = HatchTrigger {
            end: Some(V3::new(0.0, -1.0, 0.0)),
            cinematic: params(None),
            enters: false,
            exits: true,
            active: false,
        };
        let mut hatches = Hatches {
            triggers: vec![
                out,
                HatchTrigger {
                    active: true,
                    ..out
                },
            ],
            first_use: vec![(0, 1)],
        };
        let mut player = Player::new(&p, V3::new(0.5, 2.1, 0.0), true);
        let mut look = Look {
            rotation_x: 30.0,
            rotation_y: -10.0,
        };
        let cam = Pose::new(player.position, Q::IDENTITY);
        // The inactive normal twin cannot be used.
        assert!(
            hatches
                .begin(0, 0.0, &mut player, &mut look, &p, cam, cam)
                .is_none()
        );
        let (mut run, s) = hatches
            .begin(1, 0.0, &mut player, &mut look, &p, cam, cam)
            .unwrap();
        assert_eq!(s, vec![Signal::Prepare(true)]);
        assert!(player.cinematic && look == Look::default());
        assert!((player.rotation.to_euler().y - 30.0).abs() < 1e-9);
        // No second cinematic while one runs.
        assert!(
            hatches
                .begin(1, 0.1, &mut player, &mut look, &p, cam, cam)
                .is_none()
        );
        // The controller is off.
        let before = player.position;
        player.step(&p, &crate::collide::World::new(), &Default::default());
        assert_eq!(player.position, before);
        for k in 0..=20 {
            let f = run.cinematic.late_update(
                f64::from(k) * 0.02,
                Pose::new(player.position, player.rotation),
                node_start,
                node_start,
            );
            apply(&f, &mut player, &mut look, &p, -87.0, 87.0);
        }
        assert_eq!(player.position, node_start.position);
        // The end: out of the pod, the first use handed over, controls
        // and the look (the node's yaw) back.
        let last = pose(0.0, -1.2, 0.0, 200.0);
        let f = run.cinematic.end_event(2.0, last, last);
        assert!(f.signals.contains(&Signal::TriggerEnd));
        let switched = hatches.finish(run.trigger, &mut player);
        assert_eq!(switched, vec![(1, false), (0, true)]);
        assert!(!player.in_pod);
        apply(&f, &mut player, &mut look, &p, -87.0, 87.0);
        assert!(!player.cinematic);
        assert_eq!(player.position, last.position);
        assert!((look.rotation_x - 200.0).abs() < 1e-9 && look.rotation_y.abs() < 1e-9);
        assert!(player.rotation.dot(Q::IDENTITY).abs() > 1.0 - 1e-12);
    }

    #[test]
    fn the_look_goes_into_the_rotation_and_back() {
        let look = Look {
            rotation_x: 120.0,
            rotation_y: 30.0,
        };
        let q = look_into_rotation(&look);
        // Looking 30 degrees up: the forward vector climbs.
        assert!(q.rotate(V3::new(0.0, 0.0, 1.0)).y > 0.49);
        let (back, rest) = rotation_into_look(q, -87.0, 87.0);
        assert!((back.rotation_x - 120.0).abs() < 1e-9);
        assert!((back.rotation_y - 30.0).abs() < 1e-9);
        assert!(rest.dot(Q::IDENTITY).abs() > 1.0 - 1e-12);
        // A roll stays on the player and eases out.
        let rolled = Q::euler(0.0, 40.0, 20.0);
        let (back, rest) = rotation_into_look(rolled, -87.0, 87.0);
        assert!((back.rotation_x - 40.0).abs() < 1e-9 && back.rotation_y.abs() < 1e-9);
        assert!((rest.to_euler().z - 20.0).abs() < 1e-9);
        let mut r = rest;
        for _ in 0..100 {
            r = ease_tilt(r, 0.02);
        }
        let z = r.to_euler().z;
        assert!(z.min(360.0 - z) < 0.01, "{z}");
    }
}
