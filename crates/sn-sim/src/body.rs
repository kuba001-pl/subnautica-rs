//! The player's body (M9g2, `docs/DESIGN.md` § 4.3 "M9g plan"): the values
//! the game gives the player's animator with empty hands
//! (`ArmsController.Update`, `SetPlayerSpeedParameters`, `UpdateDiving`,
//! the `on_surface` rule; `Player`'s falling clock, `vr_active` and death
//! triggers), and where the camera and the body's view model are
//! (`MainCameraControl.OnUpdate` in normal play: look split between
//! `camRoot` and `cameraUPTransform`, swim bob, landing bob, step amount,
//! strafe tilt). Our own code after reading the game's; numbers that are
//! data come in [`BodyParams`], the ones in method bodies are named after
//! their method. Facts in `docs/formats/gameplay.md` § The player's body.
//!
//! Angles are Unity's: degrees, left-handed, y up; Euler angles apply z,
//! then x, then y; a positive x angle looks down.

use crate::look::Look;
use crate::{Pose, Q, V3};

/// From the game's data (`sn-assets`' `body_params`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyParams {
    /// `ArmsController.smoothSpeedUnderWater` / `smoothSpeedAboveWater`.
    pub smooth_speed_under_water: f64,
    pub smooth_speed_above_water: f64,
    /// `ArmsController.turnAnimationDampTime`. Only 0 (the game's value)
    /// is ported: then `view_turn` is set as computed (**hypothesis** for
    /// Unity's damped `SetFloat` at damp time 0).
    pub turn_animation_damp_time: f64,
    /// `MainCameraControl.skin`: `camRoot` sits this far down.
    pub skin: f64,
    /// `MainCameraControl.stepAmount` as stored (it only decays).
    pub step_amount: f64,
    /// `cameraAngleMotion.y × cameraTiltMod`: a constant roll, degrees
    /// (`cameraAngleMotion` is the view model's stored Euler angles).
    pub view_model_roll: f64,
    /// `cameraUPTransform`'s local position (below `camRoot`).
    pub camera_up_position: V3,
    /// `cameraOffsetTransform`'s local position below `cameraUPTransform`
    /// (the main camera hangs here with identity locals, `AutoParent`).
    pub camera_offset_position: V3,
    /// `Ocean.GetOceanLevel`.
    pub ocean_level: f64,
}

/// `Player.GetPlayFallingAnimation`: the falling animation starts this
/// long after falling began.
pub const FALLING_ANIMATION_DELAY: f64 = 0.45;
/// `ArmsController.diveObstaclesScanInterval`.
pub const DIVE_SCAN_INTERVAL: f64 = 0.5;

/// A parameter value for the animator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnimValue {
    Float(f64),
    Bool(bool),
    /// `SetTrigger`.
    Trigger,
}

/// One frame of what the body reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyFrame {
    /// `Time.deltaTime` and `Time.time`.
    pub dt: f64,
    pub time: f64,
    /// The player's transform and `PlayerController.velocity`.
    pub position: V3,
    pub velocity: V3,
    /// `GroundMotor.IsGrounded`.
    pub grounded: bool,
    /// `Player.IsUnderwater`.
    pub underwater: bool,
    /// `Player.IsUnderwaterForSwimming`.
    pub swimming: bool,
    /// `Player.IsInside` (the lifepod; no subs or bases yet).
    pub inside: bool,
    /// The look angles after this frame's mouse movement.
    pub look: Look,
    /// `GameInput.GetMoveDirection().x`: strafe input, −1…1.
    pub strafe: f64,
    /// `PlayerController.inputEnabled` (off while dead).
    pub controls: bool,
    /// `MiscSettings.cameraBobbing` (the option; on by default).
    pub bobbing: bool,
}

/// The camera rig after a frame: `camRoot`'s local placement (the view
/// model takes its yaw and position), and `cameraUPTransform`'s pitch.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraPose {
    /// `camRoot.localPosition.y` (bobs; x and z are 0).
    pub root_y: f64,
    /// `camRoot.localEulerAngles`: x (looking down), y (yaw), z (roll).
    pub root_pitch: f64,
    pub yaw: f64,
    pub roll: f64,
    /// `cameraUPTransform.localEulerAngles.x` (looking up, ≤ 0).
    pub up_pitch: f64,
}

fn rot_x(deg: f64, v: V3) -> V3 {
    let (s, c) = deg.to_radians().sin_cos();
    V3::new(v.x, v.y * c - v.z * s, v.y * s + v.z * c)
}

fn rot_y(deg: f64, v: V3) -> V3 {
    let (s, c) = deg.to_radians().sin_cos();
    V3::new(v.x * c + v.z * s, v.y, -v.x * s + v.z * c)
}

fn rot_z(deg: f64, v: V3) -> V3 {
    let (s, c) = deg.to_radians().sin_cos();
    V3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z)
}

impl CameraPose {
    /// The camera's rotation applied to `v` (camera space → the player's
    /// space): `camRoot`'s `Euler(x, y, z)` then `cameraUPTransform`'s.
    pub fn rotate(&self, v: V3) -> V3 {
        rot_y(
            self.yaw,
            rot_x(self.root_pitch, rot_z(self.roll, rot_x(self.up_pitch, v))),
        )
    }

    /// The inverse of [`CameraPose::rotate`] (`InverseTransformDirection`).
    pub fn inverse_rotate(&self, v: V3) -> V3 {
        rot_x(
            -self.up_pitch,
            rot_z(-self.roll, rot_x(-self.root_pitch, rot_y(-self.yaw, v))),
        )
    }

    /// The camera's position relative to the player's transform
    /// (`camPivot` and the player's rotation are identity).
    pub fn eye(&self, params: &BodyParams) -> V3 {
        let below_up = rot_x(self.up_pitch, params.camera_offset_position);
        let root = |v: V3| rot_y(self.yaw, rot_x(self.root_pitch, rot_z(self.roll, v)));
        V3::new(0.0, self.root_y, 0.0) + root(params.camera_up_position + below_up)
    }

    /// `MainCameraControl.GetCameraPitch`: the three x angles summed,
    /// in −180…180, negated (up positive).
    pub fn camera_pitch(&self) -> f64 {
        let mut d = (self.root_pitch + self.up_pitch).rem_euclid(360.0);
        if d > 180.0 {
            d -= 360.0;
        }
        -d
    }
}

/// `UWE.Utils.Slerp(float, float, float)`: moves `from` towards `to` by
/// at most |`amount`|.
pub fn move_towards(from: f64, to: f64, amount: f64) -> f64 {
    let amount = amount.abs();
    if (to - from).abs() < amount {
        return to;
    }
    // `Mathf.Sign(0)` is 1.
    from + if to - from >= 0.0 { amount } else { -amount }
}

/// `Mathf.DeltaAngle`: the shortest difference `to − from`, degrees.
pub fn delta_angle(from: f64, to: f64) -> f64 {
    let mut d = (to - from).rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

/// `Vector3.Slerp(a, b, t)`, `t` clamped to 0…1: the direction turns by
/// the fraction `t` of the angle between them, the length is linear.
/// **Hypothesis** (Unity's is native code): with a zero vector it is a
/// linear blend; between opposite directions it turns about an axis
/// perpendicular to `a`.
pub fn vector_slerp(a: V3, b: V3, t: f64) -> V3 {
    let t = t.clamp(0.0, 1.0);
    let (la, lb) = (a.length(), b.length());
    let lerp = || a + (b - a) * t;
    let (Some(da), Some(db)) = (a.normalized(), b.normalized()) else {
        return lerp();
    };
    let dot = da.dot(db).clamp(-1.0, 1.0);
    let theta = dot.acos() * t;
    let rel = match (db - da * dot).normalized() {
        Some(r) => r,
        None if dot > 0.0 => return lerp(),
        // Opposite: any axis perpendicular to `a`.
        None => {
            let other = if da.x.abs() < 0.9 {
                V3::new(1.0, 0.0, 0.0)
            } else {
                V3::Y
            };
            match da.cross(other).cross(da).normalized() {
                Some(r) => r,
                None => return lerp(),
            }
        }
    };
    (da * theta.cos() + rel * theta.sin()) * (la + (lb - la) * t)
}

/// The body's state between frames.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// `ArmsController.smoothedVelocity`.
    pub smoothed_velocity: V3,
    /// `ArmsController.previousYAngle` (starts at 0, as the game's).
    previous_yaw: f64,
    /// `Player.wasFalling` / `timeFallingBegan` (`None` is −1).
    was_falling: bool,
    time_falling_began: Option<f64>,
    /// `ArmsController.obstaclesBelow` / `nextDiveObstaclesScanTime`.
    obstacles_below: bool,
    next_dive_scan: f64,
    /// `UnderWaterTracker.isUnderWater` (updated each physics step).
    tracker_underwater: bool,
    /// `MainCameraControl` state.
    swim_camera_animation: f64,
    smoothed_speed: f64,
    impact_bob: f64,
    impact_force: f64,
    step_amount: f64,
    strafe_tilt: f64,
    /// The camera after the last frame.
    pub pose: CameraPose,
    /// Death trigger waiting for the next frame's parameters.
    death_trigger: bool,
    /// `CameraToPlayerManager` (M9g5c): from the death to the respawn the
    /// camera copies the head camera bone and `MainCameraControl` stops
    /// (the rig keeps its last pose).
    pub head_camera: bool,
}

impl Body {
    pub fn new(params: &BodyParams) -> Body {
        Body {
            smoothed_velocity: V3::ZERO,
            previous_yaw: 0.0,
            was_falling: false,
            time_falling_began: None,
            obstacles_below: true,
            next_dive_scan: 0.0,
            tracker_underwater: false,
            swim_camera_animation: 0.0,
            smoothed_speed: 0.0,
            impact_bob: 0.0,
            impact_force: 0.0,
            step_amount: params.step_amount,
            strafe_tilt: 0.0,
            pose: CameraPose::default(),
            death_trigger: false,
            head_camera: false,
        }
    }

    /// One physics step: `Player.FixedUpdate`'s falling clock and
    /// `UnderWaterTracker.FixedUpdate`. `cinematic`: a cinematic plays.
    pub fn fixed_step(&mut self, time: f64, underwater: bool, grounded: bool, cinematic: bool) {
        let falling = !underwater && !grounded && !cinematic;
        if falling != self.was_falling {
            self.was_falling = falling;
            self.time_falling_began = falling.then_some(time);
        }
        self.tracker_underwater = underwater;
    }

    /// `Player.OnJump`: the falling animation plays at once.
    pub fn jumped(&mut self, time: f64) {
        self.time_falling_began = Some(time - FALLING_ANIMATION_DELAY);
        self.was_falling = true;
    }

    /// `MainCameraControl.OnLand` (from `Player.OnLand`, in or out of the
    /// water): `impact_y` is the vertical velocity before landing.
    pub fn landed(&mut self, impact_y: f64) {
        self.impact_force = (-impact_y).clamp(0.0, 15.0);
    }

    /// `Player.OnKill`: `player_death` on the next frame. Fire and
    /// explosions pick `player_death_fire` / `_explosion`; neither exists
    /// yet, so every death is the plain one.
    pub fn died(&mut self) {
        self.death_trigger = true;
        // `Player.OnKill`: `EnableHeadCameraController`.
        self.head_camera = true;
    }

    /// `Player.ResetPlayerOnDeath` when it moves the player to the
    /// respawn point: `DisableHeadCameraController`.
    pub fn respawned(&mut self) {
        self.head_camera = false;
    }

    /// One frame: `ArmsController.Update`, then
    /// `MainCameraControl.OnUpdate` (**hypothesis** for the order; the
    /// arms read the camera as the last frame left it). `obstacle(origin,
    /// direction, distance)` is the dive scan's ray (all layers but the
    /// player's, triggers ignored). Returns the animator's parameters.
    pub fn update(
        &mut self,
        params: &BodyParams,
        f: &BodyFrame,
        obstacle: &mut dyn FnMut(V3, V3, f64) -> bool,
    ) -> Vec<(&'static str, AnimValue)> {
        let mut out = Vec::with_capacity(24);
        // `SetPlayerSpeedParameters`.
        let relative = if f.underwater || !f.grounded {
            self.pose.inverse_rotate(f.velocity)
        } else {
            let flat = |v: V3| V3::new(v.x, 0.0, v.z).normalized().unwrap_or(V3::ZERO);
            let forward = flat(self.pose.rotate(V3::new(0.0, 0.0, 1.0)));
            let right = flat(self.pose.rotate(V3::new(1.0, 0.0, 0.0)));
            V3::new(right.dot(f.velocity), 0.0, forward.dot(f.velocity))
        };
        let k = if f.underwater {
            params.smooth_speed_under_water
        } else {
            params.smooth_speed_above_water
        };
        self.smoothed_velocity = vector_slerp(self.smoothed_velocity, relative, k * f.dt);
        let s = self.smoothed_velocity;
        out.push(("move_speed", AnimValue::Float(s.length())));
        out.push(("move_speed_x", AnimValue::Float(s.x)));
        out.push(("move_speed_y", AnimValue::Float(s.y)));
        out.push(("move_speed_z", AnimValue::Float(s.z)));
        out.push(("view_pitch", AnimValue::Float(self.pose.camera_pitch())));
        // The view model's world yaw (`eulerAngles.y`, 0…360).
        let yaw = self.pose.yaw.rem_euclid(360.0);
        if f.dt > 0.0 {
            let turn = delta_angle(self.previous_yaw, yaw) / f.dt;
            out.push(("view_turn", AnimValue::Float(turn)));
            self.previous_yaw = yaw;
        }
        // `ArmsController.Update`.
        out.push(("grab", AnimValue::Bool(false)));
        out.push(("bash", AnimValue::Bool(false)));
        out.push(("is_underwater", AnimValue::Bool(f.underwater)));
        out.push(("cinematics_enabled", AnimValue::Bool(true)));
        for name in [
            "using_tool",
            "using_tool_alt",
            "holding_tool",
            "in_seamoth",
            "in_exosuit",
            "cyclops_steering",
            "bleeder",
        ] {
            out.push((name, AnimValue::Bool(false)));
        }
        let falling_animation = self
            .time_falling_began
            .is_some_and(|t| t + FALLING_ANIMATION_DELAY <= f.time);
        out.push(("jump", AnimValue::Bool(falling_animation)));
        out.push(("verticalOffset", AnimValue::Float(self.impact_bob)));
        // `UpdateDiving`.
        let diving = !f.underwater && !f.grounded && f.velocity.y < 0.0;
        if diving && f.time >= self.next_dive_scan {
            self.next_dive_scan = f.time + DIVE_SCAN_INTERVAL;
            let down = V3::new(0.0, -1.0, 0.0);
            let dir = f.velocity.normalized().unwrap_or(V3::ZERO) + down;
            let dir = dir.normalized().unwrap_or(down);
            let distance = if f.position.y > 0.0 {
                (f.position.y + 1.5) / dir.dot(down)
            } else {
                5.0
            };
            self.obstacles_below = obstacle(f.position, dir, distance);
        } else {
            self.obstacles_below = self.obstacles_below || !diving;
        }
        out.push(("diving", AnimValue::Bool(diving && !self.obstacles_below)));
        out.push((
            "diving_land",
            AnimValue::Bool(diving && self.obstacles_below),
        ));
        // `InstallAnimationRules`.
        let level = params.ocean_level;
        let on_surface =
            f.position.y > level - 1.0 && f.position.y < level + 1.0 && !f.inside && f.swimming;
        out.push(("on_surface", AnimValue::Bool(on_surface)));
        out.push(("holding_welder", AnimValue::Bool(false)));
        out.push(("using_pda", AnimValue::Bool(false)));
        out.push(("using_builder", AnimValue::Bool(false)));
        // `Player.Start`, `Player.OnKill`.
        out.push(("vr_active", AnimValue::Bool(false)));
        if std::mem::take(&mut self.death_trigger) {
            out.push(("player_death", AnimValue::Trigger));
        }

        if !self.head_camera {
            self.update_camera(params, f);
        }
        out
    }

    /// The view model (`MainCameraControl.viewModel`) in the world: the
    /// player's transform, then `camRoot`'s position and its yaw only.
    pub fn view_model(&self, player: Pose) -> Pose {
        player.then(&Pose::new(
            V3::new(0.0, self.pose.root_y, 0.0),
            Q::euler(0.0, self.pose.yaw, 0.0),
        ))
    }

    /// `camRoot` (`MainCameraControl`'s transform) in the world where the
    /// rig puts it on the player's transform.
    pub fn camera_root(&self, player: Pose) -> Pose {
        let p = &self.pose;
        player.then(&Pose::new(
            V3::new(0.0, p.root_y, 0.0),
            Q::euler(p.root_pitch, p.yaw, p.roll),
        ))
    }

    /// The main camera in the world (M9g5c): `camRoot` where the rig puts
    /// it on the player's transform, or at `root` when something else
    /// places it (a cinematic, the death camera); then
    /// `cameraUPTransform` (its look-up pitch) and `cameraOffsetTransform`.
    pub fn camera(&self, params: &BodyParams, player: Pose, root: Option<Pose>) -> Pose {
        let p = &self.pose;
        let root = root.unwrap_or_else(|| self.camera_root(player));
        root.then(&Pose::new(
            params.camera_up_position,
            Q::euler(p.up_pitch, 0.0, 0.0),
        ))
        .then(&Pose::new(params.camera_offset_position, Q::IDENTITY))
    }

    /// `MainCameraControl.OnUpdate` in normal play (no PDA, vehicle,
    /// cinematic or look-around; no camera shake).
    fn update_camera(&mut self, params: &BodyParams, f: &BodyFrame) {
        let dt = f.dt;
        self.swim_camera_animation = (self.swim_camera_animation
            + if self.tracker_underwater { dt } else { -dt })
        .clamp(0.0, 1.0);
        let rotation_y = f.look.rotation_y;
        // `UpdateStrafeTilt`.
        let tilting = f.underwater && f.controls;
        let x = if tilting { f.strafe } else { 0.0 };
        self.strafe_tilt = (self.strafe_tilt - dt * x * 12.0).clamp(-10.0, 10.0);
        self.strafe_tilt = move_towards(self.strafe_tilt, 0.0, dt * 4.0);
        let mut y = -params.skin;
        if self.swim_camera_animation > 0.0 && f.bobbing {
            let to = (f.velocity.length() / 5.0).min(1.0);
            self.smoothed_speed = move_towards(self.smoothed_speed, to, dt);
            y += ((f.time * 6.0).sin() - 1.0)
                * (0.02 + self.smoothed_speed * 0.15)
                * self.swim_camera_animation;
        }
        if self.impact_force > 0.0 {
            self.impact_bob = (self.impact_bob + self.impact_force * dt).min(0.9);
            self.impact_force -= self.impact_force.max(1.0) * dt * 5.0;
        }
        y -= self.impact_bob;
        y -= self.step_amount;
        if self.impact_bob > 0.0 {
            self.impact_bob = (self.impact_bob - self.impact_bob.sqrt() * dt * 3.0).max(0.0);
        }
        self.step_amount +=
            (0.0 - self.step_amount) * (dt * self.step_amount.abs()).clamp(0.0, 1.0);
        self.pose = CameraPose {
            root_y: y,
            root_pitch: (-rotation_y).max(0.0),
            yaw: f.look.rotation_x,
            roll: params.view_model_roll + self.strafe_tilt,
            up_pitch: (-rotation_y).min(0.0),
        };
    }

    /// `swimCameraAnimation`, `impactBob` and `strafeTilt`, for logs.
    pub fn camera_state(&self) -> (f64, f64, f64) {
        (
            self.swim_camera_animation,
            self.impact_bob,
            self.strafe_tilt,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> BodyParams {
        BodyParams {
            smooth_speed_under_water: 10.0,
            smooth_speed_above_water: 15.0,
            turn_animation_damp_time: 0.0,
            skin: 0.0,
            step_amount: 0.0,
            view_model_roll: 0.0,
            camera_up_position: V3::new(0.0, 0.063, -0.15),
            camera_offset_position: V3::ZERO,
            ocean_level: 0.0,
        }
    }

    fn frame() -> BodyFrame {
        BodyFrame {
            dt: 0.02,
            time: 0.0,
            position: V3::new(0.0, -10.0, 0.0),
            velocity: V3::ZERO,
            grounded: false,
            underwater: true,
            swimming: true,
            inside: false,
            look: Look::default(),
            strafe: 0.0,
            controls: true,
            bobbing: true,
        }
    }

    fn value(out: &[(&str, AnimValue)], name: &str) -> AnimValue {
        out.iter()
            .find(|(n, _)| *n == name)
            .map(|p| p.1)
            .unwrap_or_else(|| panic!("{name} not set"))
    }

    fn float(out: &[(&str, AnimValue)], name: &str) -> f64 {
        match value(out, name) {
            AnimValue::Float(v) => v,
            v => panic!("{name}: {v:?}"),
        }
    }

    fn boolean(out: &[(&str, AnimValue)], name: &str) -> bool {
        match value(out, name) {
            AnimValue::Bool(v) => v,
            v => panic!("{name}: {v:?}"),
        }
    }

    fn run(
        body: &mut Body,
        p: &BodyParams,
        f: &mut BodyFrame,
        n: usize,
    ) -> Vec<(&'static str, AnimValue)> {
        let mut out = Vec::new();
        for _ in 0..n {
            body.fixed_step(f.time, f.underwater, f.grounded, false);
            out = body.update(p, f, &mut |_, _, _| false);
            f.time += f.dt;
        }
        out
    }

    #[test]
    fn helpers_match_the_game() {
        assert_eq!(move_towards(0.0, 1.0, 0.25), 0.25);
        assert_eq!(move_towards(1.0, 0.0, 2.0), 0.0);
        assert_eq!(move_towards(0.0, -1.0, -0.5), -0.5);
        assert_eq!(delta_angle(350.0, 10.0), 20.0);
        assert_eq!(delta_angle(10.0, 350.0), -20.0);
        // Slerp keeps turning along the arc and blends lengths.
        let s = vector_slerp(V3::new(2.0, 0.0, 0.0), V3::new(0.0, 0.0, 4.0), 0.5);
        assert!((s.length() - 3.0).abs() < 1e-12);
        assert!((s.x - s.z).abs() < 1e-12);
        assert_eq!(
            vector_slerp(V3::ZERO, V3::new(0.0, 0.0, 4.0), 0.25),
            V3::new(0.0, 0.0, 1.0)
        );
        let o = vector_slerp(V3::new(1.0, 0.0, 0.0), V3::new(-1.0, 0.0, 0.0), 0.5);
        assert!((o.length() - 1.0).abs() < 1e-12 && o.x.abs() < 1e-12);
    }

    #[test]
    fn speed_is_smoothed_in_the_view_frame() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        // Facing +x (yaw 90°), swimming +x at 7.6 m/s: forward speed.
        f.look = Look {
            rotation_x: 90.0,
            rotation_y: 0.0,
        };
        f.velocity = V3::new(7.6, 0.0, 0.0);
        let first = run(&mut b, &p, &mut f, 2);
        // After one frame with the new yaw, the second sees the turned
        // camera; smoothing 10 × 0.02 = 0.2 of the way per frame.
        assert!(float(&first, "move_speed") < 7.6);
        let out = run(&mut b, &p, &mut f, 100);
        assert!((float(&out, "move_speed_z") - 7.6).abs() < 1e-6);
        assert!(float(&out, "move_speed_x").abs() < 1e-6);
    }

    #[test]
    fn walking_ignores_pitch() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        f.underwater = false;
        f.swimming = false;
        f.grounded = true;
        f.position.y = 5.0;
        f.look = Look {
            rotation_x: 0.0,
            rotation_y: -60.0,
        };
        f.velocity = V3::new(0.0, 0.0, 3.5);
        let out = run(&mut b, &p, &mut f, 200);
        assert!((float(&out, "move_speed_z") - 3.5).abs() < 1e-6);
        assert!(float(&out, "move_speed_y").abs() < 1e-12);
        // Looking down is the camera's own pitch: view_pitch −60.
        assert!((float(&out, "view_pitch") + 60.0).abs() < 1e-9);
    }

    #[test]
    fn view_turn_is_the_yaw_rate() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        run(&mut b, &p, &mut f, 3);
        // 1° per frame at 50 frames per second; the arms see the yaw one
        // frame late.
        let mut out = Vec::new();
        for _ in 0..5 {
            f.look.rotation_x += 1.0;
            out = run(&mut b, &p, &mut f, 1);
        }
        assert!((float(&out, "view_turn") - 50.0).abs() < 1e-6);
        // Across 0°/360° (the arms see each yaw a frame late).
        f.look.rotation_x = -0.5;
        run(&mut b, &p, &mut f, 1);
        f.look.rotation_x = 0.5;
        run(&mut b, &p, &mut f, 1);
        f.look.rotation_x = 1.5;
        let out = run(&mut b, &p, &mut f, 1);
        assert!((float(&out, "view_turn") - 50.0).abs() < 1e-6, "{out:?}");
    }

    #[test]
    fn falling_animation_after_the_delay_or_at_once_after_a_jump() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        f.underwater = false;
        f.swimming = false;
        f.position.y = 5.0;
        // Off a ledge at t = 0: 0.44 s no, 0.46 s yes.
        let out = run(&mut b, &p, &mut f, 23);
        assert!(!boolean(&out, "jump"));
        let out = run(&mut b, &p, &mut f, 1);
        assert!(boolean(&out, "jump"), "t = {}", f.time);
        // Landing clears it; a jump sets it at once.
        f.grounded = true;
        assert!(!boolean(&run(&mut b, &p, &mut f, 1), "jump"));
        b.jumped(f.time);
        f.grounded = false;
        assert!(boolean(&run(&mut b, &p, &mut f, 1), "jump"));
        // Never under water.
        f.underwater = true;
        assert!(!boolean(&run(&mut b, &p, &mut f, 30), "jump"));
    }

    #[test]
    fn diving_scans_for_obstacles() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        f.underwater = false;
        f.position.y = 4.0;
        f.velocity = V3::new(0.0, -3.0, 4.0);
        let mut rays = Vec::new();
        let mut clear = |o: V3, d: V3, l: f64| {
            rays.push((o, d, l));
            false
        };
        b.fixed_step(f.time, false, false, false);
        let out = b.update(&p, &f, &mut clear);
        assert!(boolean(&out, "diving") && !boolean(&out, "diving_land"));
        // The ray: velocity direction + down, normalised; length to 1.5 m
        // below the water along it.
        let (o, d, l) = rays[0];
        assert_eq!(o, f.position);
        let want = V3::new(0.0, -0.6 - 1.0, 0.8).normalized().unwrap();
        assert!((d - want).length() < 1e-12);
        assert!((l - 5.5 / (-d.y)).abs() < 1e-12);
        // Scans every 0.5 s: a wall below shows at the next scan.
        let mut hit = |_: V3, _: V3, _: f64| true;
        f.time = 0.4;
        let out = b.update(&p, &f, &mut hit);
        assert!(boolean(&out, "diving"));
        f.time = 0.5;
        let out = b.update(&p, &f, &mut hit);
        assert!(boolean(&out, "diving_land") && !boolean(&out, "diving"));
        // Rising: neither.
        f.velocity.y = 1.0;
        let out = b.update(&p, &f, &mut hit);
        assert!(!boolean(&out, "diving") && !boolean(&out, "diving_land"));
    }

    #[test]
    fn on_surface_band() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        for (y, swimming, inside, want) in [
            (-0.5, true, false, true),
            (0.9, true, false, true),
            (-1.0, true, false, false),
            (1.0, true, false, false),
            (0.0, false, false, false),
            (0.0, true, true, false),
        ] {
            f.position.y = y;
            f.swimming = swimming;
            f.inside = inside;
            assert_eq!(
                boolean(&run(&mut b, &p, &mut f, 1), "on_surface"),
                want,
                "y {y}"
            );
        }
    }

    #[test]
    fn swim_bob_grows_with_speed() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        // At rest under water, after the 1 s ramp: −0.04…0.
        let mut lo: f64 = 0.0;
        for _ in 0..200 {
            run(&mut b, &p, &mut f, 1);
            lo = lo.min(b.pose.root_y);
            assert!(b.pose.root_y <= 1e-12);
        }
        assert!((lo + 0.04).abs() < 1e-3, "{lo}");
        // At 5 m/s or more, after the speed ramp: −0.34…0.
        f.velocity = V3::new(0.0, 0.0, 7.6);
        run(&mut b, &p, &mut f, 100);
        let mut lo: f64 = 0.0;
        for _ in 0..200 {
            run(&mut b, &p, &mut f, 1);
            lo = lo.min(b.pose.root_y);
        }
        assert!((lo + 0.34).abs() < 1e-3, "{lo}");
        // Out of the water the bob fades over 1 s and stops.
        f.underwater = false;
        run(&mut b, &p, &mut f, 60);
        assert_eq!(b.pose.root_y, 0.0);
    }

    #[test]
    fn landing_bob_rises_and_settles() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        f.underwater = false;
        f.grounded = true;
        b.landed(-12.0);
        let mut peak: f64 = 0.0;
        for _ in 0..300 {
            run(&mut b, &p, &mut f, 1);
            peak = peak.max(b.camera_state().1);
            assert!(b.camera_state().1 <= 0.9);
        }
        assert!(peak > 0.1, "{peak}");
        assert_eq!(b.camera_state().1, 0.0);
        // The arms see the same bob as `verticalOffset`.
        b.landed(-12.0);
        run(&mut b, &p, &mut f, 3);
        let out = run(&mut b, &p, &mut f, 1);
        assert!(float(&out, "verticalOffset") > 0.0);
    }

    #[test]
    fn strafe_tilt_is_bounded_and_returns() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        f.strafe = 1.0;
        run(&mut b, &p, &mut f, 500);
        // 12°/s in, then clamped to ±10°, then 4°/s back in the same
        // frame: it settles at −10 + 0.08.
        assert!((b.pose.roll + 9.92).abs() < 1e-9, "{}", b.pose.roll);
        f.strafe = 0.0;
        run(&mut b, &p, &mut f, 200);
        assert_eq!(b.pose.roll, 0.0);
        // No tilt out of the water.
        f.underwater = false;
        f.strafe = 1.0;
        run(&mut b, &p, &mut f, 50);
        assert_eq!(b.pose.roll, 0.0);
    }

    #[test]
    fn the_eye_pivots_as_the_game_does() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        f.underwater = false;
        f.grounded = true;
        f.position.y = 5.0;
        run(&mut b, &p, &mut f, 1);
        assert!((b.pose.eye(&p) - p.camera_up_position).length() < 1e-12);
        // Looking up turns about cameraUPTransform: the eye stays.
        f.look.rotation_y = 80.0;
        run(&mut b, &p, &mut f, 1);
        assert!((b.pose.eye(&p) - p.camera_up_position).length() < 1e-12);
        assert!((b.pose.camera_pitch() - 80.0).abs() < 1e-9);
        // Looking down turns camRoot: the eye swings about the origin.
        f.look.rotation_y = -90.0;
        run(&mut b, &p, &mut f, 1);
        let e = b.pose.eye(&p);
        assert!((e - V3::new(0.0, 0.15, 0.063)).length() < 1e-12, "{e:?}");
        // The camera looks along +z rotated by the look.
        let forward = b.pose.rotate(V3::new(0.0, 0.0, 1.0));
        assert!((forward - V3::new(0.0, -1.0, 0.0)).length() < 1e-12);
        assert!((b.pose.inverse_rotate(forward) - V3::new(0.0, 0.0, 1.0)).length() < 1e-12);
    }

    #[test]
    fn death_trigger_once() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        b.died();
        let out = run(&mut b, &p, &mut f, 1);
        assert_eq!(value(&out, "player_death"), AnimValue::Trigger);
        let out = run(&mut b, &p, &mut f, 1);
        assert!(out.iter().all(|(n, _)| *n != "player_death"));
    }

    #[test]
    fn the_world_camera_matches_the_rig() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        f.look = Look {
            rotation_x: 70.0,
            rotation_y: 25.0,
        };
        run(&mut b, &p, &mut f, 5);
        let at = V3::new(1.0, -10.0, 2.0);
        let player = Pose::new(at, Q::IDENTITY);
        // With the player not turned: the rig's own eye and rotation.
        let c = b.camera(&p, player, None);
        assert!((c.position - (at + b.pose.eye(&p))).length() < 1e-9);
        let v = V3::new(0.2, 0.3, 1.0);
        assert!((c.rotation.rotate(v) - b.pose.rotate(v)).length() < 1e-9);
        // A turned player turns the rig with it.
        let turned = Pose::new(at, Q::euler(0.0, 90.0, 0.0));
        let c = b.camera(&p, turned, None);
        let ahead = c.rotation.rotate(V3::new(0.0, 0.0, 1.0));
        let flat = Q::euler(0.0, 90.0, 0.0).rotate(b.pose.rotate(V3::new(0.0, 0.0, 1.0)));
        assert!((ahead - flat).length() < 1e-9);
        // Somewhere else placing camRoot: the eye hangs below it.
        let root = Pose::new(V3::new(5.0, 0.0, 0.0), Q::IDENTITY);
        let c = b.camera(&p, player, Some(root));
        let up = Q::euler(b.pose.up_pitch, 0.0, 0.0).rotate(p.camera_offset_position);
        assert!((c.position - (root.position + p.camera_up_position + up)).length() < 1e-9);
        // The view model takes the yaw only.
        let vm = b.view_model(player);
        assert!((vm.rotation.to_euler().y - 70.0).abs() < 1e-9);
    }

    #[test]
    fn the_head_camera_freezes_the_rig_until_respawn() {
        let p = params();
        let mut b = Body::new(&p);
        let mut f = frame();
        run(&mut b, &p, &mut f, 3);
        b.died();
        let frozen = b.pose;
        f.look.rotation_x = 45.0;
        run(&mut b, &p, &mut f, 3);
        assert_eq!(b.pose, frozen);
        b.respawned();
        run(&mut b, &p, &mut f, 1);
        assert_eq!(b.pose.yaw, 45.0);
    }
}
