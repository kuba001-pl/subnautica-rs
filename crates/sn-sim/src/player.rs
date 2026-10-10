//! The player's movement (M9b, `docs/DESIGN.md` § 4.3 "M9b plan"): which
//! motor runs, swimming (`UnderwaterMotor`), walking (`GroundMotor` on a
//! character controller), going through the lifepod's hatches
//! ([`Player::teleport`]), what the hand points at ([`hand_target`]). Our own code after reading
//! the game's; the numbers come from the install ([`PlayerParams`]).
//!
//! One call to [`Player::step`] is one physics step of the game
//! (`Time.fixedDeltaTime`, 0.02 s in Subnautica). Not 1:1 yet: PhysX's
//! solver is our slide plus velocity clipping; the walking motor's moving
//! platforms, sprint, wind and slope-speed curve are left out; equipment
//! does not change speeds (no inventory yet).

use crate::V3;
use crate::collide::{Capsule, Hit, World};

/// Everything the rules need, from the game's data (see `sn-assets`'
/// `player_data` and `physics_settings`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerParams {
    /// `PlayerController.controllerRadius`.
    pub radius: f64,
    /// `standheight`, `swimheight`, `cameraOffset`.
    pub stand_height: f64,
    pub swim_height: f64,
    pub camera_offset: f64,
    /// `PlayerController.swim*MaxSpeed`.
    pub swim_forward: f64,
    pub swim_backward: f64,
    pub swim_strafe: f64,
    pub swim_vertical: f64,
    /// `swimWaterAcceleration`.
    pub water_acceleration: f64,
    /// `defaultSwimDrag`: the rigid body's drag while swimming.
    pub swim_drag: f64,
    /// `walkRunForwardMaxSpeed` (the walking motor uses it in every
    /// direction).
    pub walk_speed: f64,
    /// `PlayerMotor.groundAcceleration`, `airAcceleration`, `gravity`.
    pub ground_acceleration: f64,
    pub air_acceleration: f64,
    pub gravity: f64,
    /// `GroundMotor.movement.maxFallSpeed`.
    pub max_fall_speed: f64,
    /// `GroundMotor.jumping`.
    pub jump_enabled: bool,
    pub jump_base_height: f64,
    pub jump_perp_amount: f64,
    pub jump_steep_perp_amount: f64,
    /// `GroundMotor.sliding`.
    pub sliding_enabled: bool,
    pub sliding_speed: f64,
    pub sliding_sideways_control: f64,
    pub sliding_speed_control: f64,
    /// `GroundMotor.controllerSetup`: the character controller's.
    pub step_offset: f64,
    pub slope_limit_degrees: f64,
    /// `Ocean.GetOceanLevel()`.
    pub ocean_level: f64,
    /// `Time.fixedDeltaTime`.
    pub fixed_dt: f64,
}

/// What the player asks for in one step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    /// `GameInput.GetMoveDirection`: x right, y up (swimming only), z
    /// forward; each in −1..1.
    pub move_dir: V3,
    /// Camera yaw about +y and pitch about +x (positive looks down),
    /// radians, as Unity's Euler angles.
    pub yaw: f64,
    pub pitch: f64,
    /// The jump button is held.
    pub jump: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motor {
    Swim,
    Walk,
}

/// Something that happened in a step, for logs and sound later.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    MotorChanged(Motor),
    /// The walking motor landed; `impact_y` is the vertical velocity
    /// before the step (`GroundMotor.previousVelocity`, `OnLand`).
    Landed {
        impact_y: f64,
    },
    Jumped,
    /// Left the escape pod's 15 m radius without a hatch.
    LeftPodRadius,
    SteppedOutOfWater,
}

/// `Player.escapePodRadius`.
pub const ESCAPE_POD_RADIUS: f64 = 15.0;

/// `UnderwaterMotor.stepHeight` / `stepDistance`: climbing out of the
/// water onto land.
const SWIM_STEP_HEIGHT: f64 = 1.85;
const SWIM_STEP_DISTANCE: f64 = 0.3;

/// `GUIHand.kUseDistance`: how far the hand reaches, metres.
pub const USE_DISTANCE: f64 = 2.0;

/// `Targeting.standardRadiuses`: sphere casts tried when the ray misses.
pub const HAND_RADII: [f64; 2] = [0.15, 0.3];

/// What the hand points at (`Targeting.GetTarget`): a ray from the camera
/// along its forward axis, `USE_DISTANCE` long, against the bodies in the
/// [`HAND`](crate::collide::HAND) group; if it misses, sphere casts of
/// each radius in turn, each starting that radius ahead. The nearest hit
/// and its distance.
pub fn hand_target(world: &World, eye: V3, yaw: f64, pitch: f64) -> Option<(f64, Hit)> {
    let forward = look_rotate(yaw, pitch, V3::new(0.0, 0.0, 1.0));
    let group = crate::collide::HAND;
    if let Some(hit) = world.cast(group, eye, forward, 0.0, USE_DISTANCE) {
        return Some(hit);
    }
    HAND_RADII
        .iter()
        .find_map(|&r| world.cast(group, eye + forward * r, forward, r, USE_DISTANCE))
}

/// A hatch trigger of the lifepod (`CinematicModeTrigger` with its
/// `PlayerCinematicController`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HatchTrigger {
    /// The cinematic's end point (`None`: where its animation ends, not
    /// read).
    pub end: Option<V3>,
    /// Calls `EnterExitHelper.CinematicEnter` / `CinematicExit`.
    pub enters: bool,
    pub exits: bool,
    pub active: bool,
}

/// The lifepod's hatch triggers and which are first-use ones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hatches {
    pub triggers: Vec<HatchTrigger>,
    /// (normal, first use) pairs (`EscapePodFirstUseCinematicsController`).
    pub first_use: Vec<(usize, usize)>,
}

impl Hatches {
    /// Uses trigger `i`: the player goes to its end point at rest (the
    /// cinematic itself is not played), in or out of the pod by its call;
    /// a first-use trigger hands over to the normal one. Returns the
    /// triggers switched on or off, `None` if it cannot be used (inactive,
    /// or no end point).
    pub fn use_trigger(
        &mut self,
        i: usize,
        player: &mut Player,
        params: &PlayerParams,
    ) -> Option<Vec<(usize, bool)>> {
        let t = *self.triggers.get(i)?;
        if !t.active {
            return None;
        }
        let in_pod = if t.enters {
            Some(true)
        } else if t.exits {
            Some(false)
        } else {
            None
        };
        player.teleport(params, t.end?, in_pod);
        let mut switched = Vec::new();
        if let Some(&(normal, first)) = self.first_use.iter().find(|p| p.1 == i) {
            for (k, on) in [(first, false), (normal, true)] {
                if let Some(tr) = self.triggers.get_mut(k) {
                    tr.active = on;
                    switched.push((k, on));
                }
            }
        }
        Some(switched)
    }
}

/// `PlayerMotor.IsWalkable`.
fn walkable(normal: V3) -> bool {
    normal.y > 0.01
}

/// Unity's `Quaternion.Euler(pitch, yaw, 0) * v` (left-handed, y up).
pub fn look_rotate(yaw: f64, pitch: f64, v: V3) -> V3 {
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    // Pitch about x, then yaw about y.
    let p = V3::new(v.x, v.y * cp - v.z * sp, v.y * sp + v.z * cp);
    V3::new(p.x * cy + p.z * sy, p.y, -p.x * sy + p.z * cy)
}

fn slerp_up(n: V3, amount: f64) -> V3 {
    // Vector3.Slerp(up, n, amount) for unit vectors.
    let up = V3::Y;
    let d = up.dot(n).clamp(-1.0, 1.0);
    let theta = d.acos() * amount;
    let Some(rel) = (n - up * d).normalized() else {
        return up;
    };
    up * theta.cos() + rel * theta.sin()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    /// The player's transform (the camera sits here).
    pub position: V3,
    pub velocity: V3,
    pub motor: Motor,
    /// Inside the lifepod (`Player.escapePod`).
    pub in_pod: bool,
    /// Where the pod is, for the 15 m radius rule.
    pub pod_position: Option<V3>,
    /// `Player.isUnderwaterForSwimming`.
    pub swimming: bool,
    pub grounded: bool,
    /// `GroundMotor.IsGrounded()`: the walking motor's own flag. It starts
    /// true (`PlayerMotor.grounded`) and only the walking motor's steps
    /// change it, so it keeps its last value while swimming, dead or
    /// moved by a hatch or a respawn (M9g3: the body's falling clock reads
    /// it). `grounded` is cleared by those.
    pub walk_grounded: bool,
    /// The controller's current height (eases to the motor's).
    pub height: f64,
    ground_normal: V3,
    /// Seconds since the jump button went down (`lastButtonDownTime`).
    jump_held_for: Option<f64>,
    recently_collided: bool,
}

impl Player {
    /// A player standing at `position` (`EscapePod.playerSpawn` for a new
    /// game, in the pod).
    pub fn new(params: &PlayerParams, position: V3, in_pod: bool) -> Player {
        let mut p = Player {
            position,
            velocity: V3::ZERO,
            motor: Motor::Walk,
            in_pod,
            pod_position: None,
            swimming: false,
            grounded: false,
            walk_grounded: true,
            height: params.stand_height - params.camera_offset,
            ground_normal: V3::ZERO,
            jump_held_for: None,
            recently_collided: false,
        };
        p.update_swimming(params, None);
        p.motor = if p.swimming { Motor::Swim } else { Motor::Walk };
        p.height = p.desired_height(params);
        p
    }

    fn desired_height(&self, params: &PlayerParams) -> f64 {
        let h = if self.swimming {
            params.swim_height
        } else {
            params.stand_height
        };
        h - params.camera_offset
    }

    /// The collider at the current height: its centre is half its height
    /// below the transform, minus the camera offset (`SetControllerHeight`).
    pub fn capsule(&self, params: &PlayerParams) -> Capsule {
        Self::capsule_of(params, self.height)
    }

    fn capsule_of(params: &PlayerParams, height: f64) -> Capsule {
        let center = V3::new(0.0, -height * 0.5 - params.camera_offset, 0.0);
        Capsule::unity(params.radius, height, 1, center)
    }

    /// `Player.UpdateIsUnderwater` and `UpdateIsUnderwaterForSwimming`.
    fn update_swimming(&mut self, params: &PlayerParams, world: Option<&World>) {
        let y = self.position.y;
        let level = params.ocean_level;
        if self.in_pod {
            self.swimming = false;
            return;
        }
        if !self.swimming {
            self.swimming = y < level - 0.1;
            return;
        }
        let mut on_ground = false;
        if y > level + 0.1 {
            on_ground = self.grounded;
            if !on_ground && let Some(world) = world {
                // A ray down to just below the water (the game's, every
                // second; ours every step).
                let reach = y - level - 0.1 + params.stand_height * 0.5;
                on_ground = world
                    .cast(crate::collide::MOVE, self.position, -V3::Y, 0.0, reach)
                    .is_some();
            }
        }
        self.swimming = y < level + if on_ground { 0.1 } else { 0.8 };
    }

    /// Puts the player at `to`, at rest, and sets whether it is in the pod
    /// (`None`: unchanged). The end of a lifepod cinematic (the
    /// controller's end point; `EnterExitHelper.CinematicEnter` /
    /// `CinematicExit`), or `UseableDiveHatch`'s branch without one.
    pub fn teleport(&mut self, params: &PlayerParams, to: V3, in_pod: Option<bool>) {
        self.position = to;
        if let Some(inside) = in_pod {
            self.in_pod = inside;
        }
        self.velocity = V3::ZERO;
        self.grounded = false;
        self.update_swimming(params, None);
    }

    /// One physics step.
    pub fn step(&mut self, params: &PlayerParams, world: &World, input: &Input) -> Vec<Event> {
        let mut events = Vec::new();
        let dt = params.fixed_dt;
        // `Player.ValidateEscapePod`.
        if self.in_pod
            && let Some(pod) = self.pod_position
            && (pod - self.position).length() > ESCAPE_POD_RADIUS
        {
            self.in_pod = false;
            events.push(Event::LeftPodRadius);
        }
        // `PlayerController.HandleUnderWaterState`.
        self.update_swimming(params, Some(world));
        let motor = if self.swimming {
            Motor::Swim
        } else {
            Motor::Walk
        };
        if motor != self.motor {
            self.motor = motor;
            if motor == Motor::Swim {
                // `UnderwaterMotor.SetEnabled`.
                self.velocity.y *= 0.5;
            }
            events.push(Event::MotorChanged(motor));
        }
        // `PlayerController.UpdateController`: the height eases at 2 m/s,
        // growing only when there is room above.
        let desired = self.desired_height(params);
        let next = if desired > self.height {
            (self.height + 2.0 * dt).min(desired)
        } else {
            (self.height - 2.0 * dt).max(desired)
        };
        if next > self.height {
            let grow = next - self.height;
            let top = self.position + V3::new(0.0, -params.camera_offset, 0.0);
            let probe = Capsule {
                a: V3::ZERO,
                b: V3::ZERO,
                radius: params.radius + 0.01,
            };
            if world.sweep(&probe, top, V3::Y * grow).is_none() {
                if self.motor == Motor::Walk {
                    // `GroundMotor.SetControllerHeight` raises the player.
                    self.position.y += grow;
                }
                self.height = next;
            }
        } else {
            self.height = next;
        }
        match self.motor {
            Motor::Swim => self.swim(params, world, input, &mut events),
            Motor::Walk => {
                self.walk(params, world, input, &mut events);
                self.walk_grounded = self.grounded;
            }
        }
        events
    }

    /// `UnderwaterMotor.UpdateMove`, then the rigid body's step.
    fn swim(
        &mut self,
        params: &PlayerParams,
        world: &World,
        input: &Input,
        events: &mut Vec<Event>,
    ) {
        let dt = params.fixed_dt;
        let level = params.ocean_level;
        let mut dir = input.move_dir;
        let up = dir.y;
        let amount = dir.length().min(1.0);
        dir.y = 0.0;
        dir = dir.normalized().unwrap_or(V3::ZERO);
        let mut max = 0.0f64;
        if dir.z > 0.0 {
            max = params.swim_forward;
        } else if dir.z < 0.0 {
            max = params.swim_backward;
        }
        if dir.x != 0.0 {
            max = max.max(params.swim_strafe);
        }
        max = max.max(params.swim_vertical);
        // `AlterMaxSpeed`: no equipment yet; × 1.3 above the water.
        let base = max;
        if self.position.y > level {
            max *= 1.3;
        }
        let boost = if base > 0.0 { max / base } else { 1.0 };
        let cap = max.max(self.velocity.length());
        let look = look_rotate(input.yaw, input.pitch, dir);
        let mut wish = look;
        wish.y += up;
        let wish = wish.normalized().unwrap_or(V3::ZERO);
        let accel = if self.grounded {
            params.ground_acceleration
        } else {
            params.water_acceleration
        };
        let mut gain = amount * accel * dt;
        if boost > 1.0 {
            gain *= boost;
        }
        if gain > 0.0 {
            let mut v = self.velocity + wish * gain;
            if v.length() > cap {
                v = v.normalized().unwrap_or(V3::ZERO) * cap;
            }
            // Near the surface the upward speed fades out, unless diving
            // or looking down.
            let diving = up < 0.0;
            let looking_down = look.y < -0.3;
            let top = level + 0.8 - 0.28;
            let bottom = level - 0.5;
            if self.position.y >= bottom && !diving && !looking_down {
                let k = ((top - self.position.y) / (top - bottom)).clamp(0.0, 1.0);
                v.y *= k.powf(0.3);
            }
            // Climbing out onto land at the surface.
            let ahead = V3::new(wish.x, 0.0, wish.z) * SWIM_STEP_DISTANCE;
            let capsule = self.capsule(params);
            if self.position.y >= level
                && ahead.length() > 0.0
                && (self.recently_collided || world.sweep(&capsule, self.position, ahead).is_some())
                && world
                    .sweep(&capsule, self.position, V3::Y * SWIM_STEP_HEIGHT)
                    .is_none()
            {
                let high = self.position + V3::Y * SWIM_STEP_HEIGHT;
                if world.sweep(&capsule, high, ahead).is_none()
                    && let Some(h) = world.sweep(&capsule, high + ahead, -V3::Y * SWIM_STEP_HEIGHT)
                    && walkable(h.normal)
                {
                    let drop = (h.t * SWIM_STEP_HEIGHT - 0.1).max(0.0);
                    self.position = high + ahead - V3::Y * drop;
                    v.y = 0.2;
                    events.push(Event::SteppedOutOfWater);
                }
            }
            self.velocity = v;
        }
        // No gravity while swimming (`SetMotorMode`: underWaterGravity 0;
        // the rigid body has useGravity off). The body's drag
        // (*hypothesis*: PhysX damping v·(1 − drag·dt)).
        self.velocity = self.velocity * (1.0 - params.swim_drag * dt).max(0.0);
        self.grounded = false;
        let hits = self.integrate(params, world);
        self.recently_collided = !hits.is_empty();
    }

    /// Moves by the velocity through the world and drops the velocity's
    /// part into what was hit (a rigid body without bounce).
    fn integrate(&mut self, params: &PlayerParams, world: &World) -> Vec<Hit> {
        let capsule = self.capsule(params);
        let slide = world.move_and_slide(&capsule, self.position, self.velocity * params.fixed_dt);
        self.position = slide.position;
        for h in &slide.contacts {
            let into = self.velocity.dot(h.normal);
            if into < 0.0 {
                self.velocity -= h.normal * into;
            }
        }
        slide.contacts
    }

    /// `GroundMotor.UpdateFunction` (`ApplyInputVelocityChange`,
    /// `ApplyGravityAndJumping`, `CharacterController.Move`).
    fn walk(
        &mut self,
        params: &PlayerParams,
        world: &World,
        input: &Input,
        events: &mut Vec<Event>,
    ) {
        let dt = params.fixed_dt;
        let too_steep = self.ground_normal.y <= params.slope_limit_degrees.to_radians().cos();
        let before = self.velocity;
        let mut v = self.velocity;

        // Input → wanted velocity, yaw only.
        let amount = input.move_dir.length().min(1.0);
        let flat_input = V3::new(input.move_dir.x, 0.0, input.move_dir.z);
        let dir = look_rotate(input.yaw, 0.0, flat_input)
            .normalized()
            .unwrap_or(V3::ZERO);
        let mut wanted = if self.grounded && too_steep && params.sliding_enabled {
            let slide_dir = V3::new(self.ground_normal.x, 0.0, self.ground_normal.z)
                .normalized()
                .unwrap_or(V3::ZERO);
            // The game projects the raw (local) input here.
            let raw = input.move_dir;
            let along = slide_dir * raw.dot(slide_dir);
            (slide_dir
                + along * params.sliding_speed_control
                + (raw - along) * params.sliding_sideways_control)
                * params.sliding_speed
        } else {
            dir * (params.walk_speed * amount)
        };
        if self.grounded {
            // `AdjustGroundVelocityToNormal`.
            let len = wanted.length();
            wanted = V3::Y
                .cross(wanted)
                .cross(self.ground_normal)
                .normalized()
                .unwrap_or(V3::ZERO)
                * len;
        } else {
            v.y = 0.0;
        }
        let max_change = if self.grounded {
            params.ground_acceleration
        } else {
            params.air_acceleration
        } * dt;
        let mut change = wanted - v;
        if change.length() > max_change {
            change = change.normalized().unwrap_or(V3::ZERO) * max_change;
        }
        v += change;
        if self.grounded {
            v.y = v.y.min(0.0);
        }

        // Gravity and jumping.
        if input.jump {
            let held = self.jump_held_for.map_or(0.0, |t| t + dt);
            self.jump_held_for = Some(held);
        } else {
            self.jump_held_for = None;
        }
        if !self.grounded {
            v.y = (before.y - params.gravity * dt).max(-params.max_fall_speed);
        }
        if self.grounded && params.jump_enabled && self.jump_held_for.is_some_and(|t| t < 0.2) {
            self.grounded = false;
            // One jump per press (`lastButtonDownTime = -100`).
            self.jump_held_for = Some(f64::INFINITY);
            let perp = if too_steep {
                params.jump_steep_perp_amount
            } else {
                params.jump_perp_amount
            };
            let jump_dir = slerp_up(self.ground_normal.normalized().unwrap_or(V3::Y), perp);
            v.y = 0.0;
            v += jump_dir * (2.0 * params.jump_base_height * params.gravity).sqrt();
            events.push(Event::Jumped);
        }

        // Move: pushed down while grounded so it follows the floor.
        let start = self.position;
        let mut delta = v * dt;
        let step = V3::new(delta.x, 0.0, delta.z)
            .length()
            .max(params.step_offset);
        if self.grounded {
            delta.y -= step;
        }
        let moved = self.controller_move(params, world, delta);
        let mut new_v = (self.position - start) * (1.0 / dt);
        if new_v.dot(new_v) <= 0.2 {
            new_v = v;
        }
        if new_v.y > 0.0 || !moved.collided {
            new_v.y = v.y;
        }
        let h_wanted = V3::new(v.x, 0.0, v.z);
        let h_new = V3::new(new_v.x, 0.0, new_v.z);
        let mut out = if h_wanted.dot(h_wanted) == 0.0 {
            V3::new(0.0, new_v.y, 0.0)
        } else {
            let k = (h_new.dot(h_wanted) / h_wanted.dot(h_wanted)).clamp(0.0, 1.0);
            h_wanted * k + V3::Y * new_v.y
        };
        if out.y < v.y - 0.001 && out.y < 0.0 {
            out.y = v.y;
        }
        self.velocity = out;
        self.ground_normal = moved.ground_normal;
        let on_ground = walkable(self.ground_normal);
        if self.grounded && !on_ground {
            self.grounded = false;
            // Undo the push down: it found no floor.
            self.position.y += step;
        } else if !self.grounded && on_ground {
            self.grounded = true;
            events.push(Event::Landed { impact_y: before.y });
        }
    }

    /// Our character controller (PhysX's has an up pass by the step
    /// offset, a side pass treating too-steep slopes as walls, and a down
    /// pass; a step that ends on ground too steep for the slope limit is
    /// cancelled; this follows that outline). Returns the best floor
    /// normal touched while moving down, and whether anything was hit.
    fn controller_move(&mut self, params: &PlayerParams, world: &World, delta: V3) -> Moved {
        let side = V3::new(delta.x, 0.0, delta.z);
        let steep = params.slope_limit_degrees.to_radians().cos();
        let lift = if self.grounded && side.length() > 0.0 {
            params.step_offset
        } else {
            0.0
        };
        let start = self.position;
        let moved = self.controller_passes(params, world, delta, lift);
        if lift > 0.0 && moved.stepped && moved.ground_normal.y < steep {
            self.position = start;
            return self.controller_passes(params, world, delta, 0.0);
        }
        moved
    }

    fn controller_passes(
        &mut self,
        params: &PlayerParams,
        world: &World,
        delta: V3,
        lift: f64,
    ) -> Moved {
        let capsule = self.capsule(params);
        let steep = params.slope_limit_degrees.to_radians().cos();
        let mut ground = V3::ZERO;
        let mut collided = false;
        let side = V3::new(delta.x, 0.0, delta.z);
        let up = delta.y.max(0.0) + lift;
        // Up and down move straight and stop at the first contact (no
        // sliding, as PhysX's controller).
        let straight = |from: V3, delta: V3| match world.sweep(&capsule, from, delta) {
            Some(h) => (from + delta * h.t, Some(h)),
            None => (from + delta, None),
        };
        let start_y = self.position.y;
        // Up.
        let (to, hit) = straight(self.position, V3::Y * up);
        collided |= hit.is_some();
        let lifted = to.y - self.position.y;
        self.position = to;
        // Side: slopes too steep to climb act as walls.
        let s = world.move_and_slide_with(&capsule, self.position, side, |n| {
            if n.y < steep && n.y > -0.01 {
                V3::new(n.x, 0.0, n.z)
            } else {
                n
            }
        });
        collided |= !s.contacts.is_empty();
        self.position = s.position;
        // Down: the step back down and the move's own fall.
        let stepped_up = (lifted - delta.y.max(0.0)).max(0.0);
        let down = (-delta.y).max(0.0) + stepped_up;
        let (to, hit) = straight(self.position, -V3::Y * down);
        if let Some(h) = hit {
            if h.normal.y > 0.0 {
                ground = h.normal;
            }
            collided = true;
        }
        // Ended higher than it started: the step was used.
        let stepped = stepped_up > 0.0 && to.y > start_y + 1e-3;
        self.position = to;
        Moved {
            ground_normal: ground,
            collided,
            stepped,
        }
    }
}

struct Moved {
    ground_normal: V3,
    collided: bool,
    /// The up pass's lift was not all given back by the down pass.
    stepped: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::Body;

    /// The game's numbers as `sn-inspect player` prints them.
    fn params() -> PlayerParams {
        PlayerParams {
            radius: 0.3,
            stand_height: 1.5,
            swim_height: 0.5,
            camera_offset: -0.25,
            swim_forward: 7.6,
            swim_backward: 7.6,
            swim_strafe: 7.6,
            swim_vertical: 6.3,
            water_acceleration: 20.0,
            swim_drag: 2.5,
            walk_speed: 3.5,
            ground_acceleration: 45.0,
            air_acceleration: 5.0,
            gravity: 12.0,
            max_fall_speed: 50.0,
            jump_enabled: true,
            jump_base_height: 1.0,
            jump_perp_amount: 0.0,
            jump_steep_perp_amount: 0.5,
            sliding_enabled: true,
            sliding_speed: 7.0,
            sliding_sideways_control: 1.0,
            sliding_speed_control: 0.2,
            step_offset: 0.4,
            slope_limit_degrees: 60.0,
            ocean_level: 0.0,
            fixed_dt: 0.02,
        }
    }

    fn v(x: f64, y: f64, z: f64) -> V3 {
        V3::new(x, y, z)
    }

    fn quad(o: V3, u: V3, w: V3) -> Vec<[[f32; 3]; 3]> {
        let f = |p: V3| p.to_f32();
        vec![
            [f(o), f(o + u), f(o + u + w)],
            [f(o), f(o + u + w), f(o + w)],
        ]
    }

    /// The triangles turned to face the point `p` (triangles are one-sided).
    fn facing(tris: Vec<[[f32; 3]; 3]>, p: V3) -> Vec<[[f32; 3]; 3]> {
        tris.into_iter()
            .map(|t| {
                let [a, b, c] = t.map(V3::from_f32);
                if (b - a).cross(c - a).dot(p - a) < 0.0 {
                    [t[0], t[2], t[1]]
                } else {
                    t
                }
            })
            .collect()
    }

    fn floor_at(y: f64) -> Vec<[[f32; 3]; 3]> {
        facing(
            quad(v(-100.0, y, -100.0), v(200.0, 0.0, 0.0), v(0.0, 0.0, 200.0)),
            v(0.0, y + 1.0, 0.0),
        )
    }

    fn world(tris: Vec<[[f32; 3]; 3]>) -> World {
        let mut w = World::new();
        w.insert(1, Body::new(tris, vec![]));
        w
    }

    fn forward() -> Input {
        Input {
            move_dir: v(0.0, 0.0, 1.0),
            ..Input::default()
        }
    }

    #[test]
    fn look_rotation_matches_unity() {
        // Yaw 90°: forward (+z) turns to +x; pitch 90°: forward looks down.
        let r = look_rotate(std::f64::consts::FRAC_PI_2, 0.0, v(0.0, 0.0, 1.0));
        assert!((r - v(1.0, 0.0, 0.0)).length() < 1e-12);
        let r = look_rotate(0.0, std::f64::consts::FRAC_PI_2, v(0.0, 0.0, 1.0));
        assert!((r - v(0.0, -1.0, 0.0)).length() < 1e-12);
    }

    /// `GroundMotor.IsGrounded()` keeps its value while the walking motor
    /// doesn't run (swimming, a teleport); walking steps update it.
    #[test]
    fn walk_grounded_is_the_walking_motors_own_flag() {
        let p = params();
        let w = world(floor_at(0.0));
        let mut pl = Player::new(&p, v(0.0, 5.0, 0.0), false);
        assert!(pl.walk_grounded, "starts true, as PlayerMotor.grounded");
        pl.step(&p, &w, &Input::default());
        assert!(!pl.walk_grounded, "falling");
        for _ in 0..200 {
            pl.step(&p, &w, &Input::default());
        }
        assert!(pl.grounded && pl.walk_grounded);
        // A teleport clears `grounded`, not the walking motor's flag.
        pl.teleport(&p, v(0.0, 3.0, 0.0), None);
        assert!(!pl.grounded && pl.walk_grounded);
        // Swimming doesn't touch it either.
        let mut sw = Player::new(&p, v(0.0, -20.0, 0.0), false);
        for _ in 0..10 {
            sw.step(&p, &World::new(), &forward());
        }
        assert!(!sw.grounded && sw.walk_grounded);
    }

    #[test]
    fn swimming_settles_at_the_games_speed() {
        let p = params();
        let w = World::new();
        let mut pl = Player::new(&p, v(0.0, -20.0, 0.0), false);
        assert_eq!(pl.motor, Motor::Swim);
        let mut speeds = Vec::new();
        for _ in 0..300 {
            pl.step(&p, &w, &forward());
            speeds.push(pl.velocity.length());
        }
        // Capped at 7.6 before the drag, 7.6 × (1 − 2.5 × 0.02) after.
        let settled = *speeds.last().unwrap();
        assert!((settled - 7.6 * 0.95).abs() < 1e-9, "{settled}");
        assert!(speeds.iter().all(|&s| s <= 7.6 + 1e-9));
        // Let go: the drag stops the player.
        for _ in 0..300 {
            pl.step(&p, &w, &Input::default());
        }
        assert!(pl.velocity.length() < 0.01);
    }

    #[test]
    fn upward_speed_fades_at_the_surface() {
        let p = params();
        let w = World::new();
        let mut pl = Player::new(&p, v(0.0, -5.0, 0.0), false);
        let up = Input {
            move_dir: v(0.0, 1.0, 0.0),
            ..Input::default()
        };
        for _ in 0..500 {
            pl.step(&p, &w, &up);
        }
        // It cannot rise past level + 0.52, where the factor reaches 0.
        // (A step's worth of overshoot: the factor uses the position before
        // the move.)
        assert!(
            pl.position.y < 0.53 && pl.position.y > 0.3,
            "{}",
            pl.position.y
        );
        assert_eq!(pl.motor, Motor::Swim);
    }

    #[test]
    fn leaving_and_entering_water_switch_motors_at_the_games_levels() {
        let p = params();
        // A floor 1 m above the water: the swim step lifts the player out.
        let mut tris = floor_at(-5.0);
        tris.extend(facing(
            quad(v(2.0, -5.0, -50.0), v(0.0, 6.0, 0.0), v(0.0, 0.0, 100.0)),
            v(0.0, -2.0, 0.0),
        ));
        tris.extend(facing(
            quad(v(2.0, 1.0, -50.0), v(50.0, 0.0, 0.0), v(0.0, 0.0, 100.0)),
            v(10.0, 5.0, 0.0),
        ));
        let w = world(tris);
        // Facing +x (yaw 90°), forward and up, as a player climbing out.
        let east = Input {
            move_dir: v(0.0, 1.0, 1.0),
            yaw: std::f64::consts::FRAC_PI_2,
            ..Input::default()
        };
        let mut pl = Player::new(&p, v(0.0, -0.3, 0.0), false);
        assert_eq!(pl.motor, Motor::Swim);
        let mut changes = Vec::new();
        let mut stepped = false;
        for _ in 0..400 {
            for e in pl.step(&p, &w, &east) {
                match e {
                    Event::MotorChanged(m) => changes.push((m, pl.position.y)),
                    Event::SteppedOutOfWater => stepped = true,
                    _ => {}
                }
            }
        }
        assert!(stepped, "never climbed out at {:?}", pl.position);
        assert_eq!(pl.motor, Motor::Walk);
        assert!(pl.position.x > 3.0 && pl.grounded, "{:?}", pl.position);
        // Walking on land, the camera 1.5 m above the floor at y = 1 (the
        // 1.75 m capsule reaches 0.25 m above the camera).
        assert!(
            (pl.position.y - (1.0 + 1.5)).abs() < 0.05,
            "{}",
            pl.position.y
        );
        // Walk back off the edge into the water: swimming again below −0.1.
        let west = Input {
            move_dir: v(0.0, 0.0, 1.0),
            yaw: -std::f64::consts::FRAC_PI_2,
            ..east
        };
        for _ in 0..600 {
            for e in pl.step(&p, &w, &west) {
                if let Event::MotorChanged(m) = e {
                    changes.push((m, pl.position.y));
                }
            }
        }
        let (m, y) = *changes.last().unwrap();
        assert_eq!(m, Motor::Swim);
        assert!(y < -0.1, "switched at {y}");
    }

    #[test]
    fn walks_at_the_walk_speed_and_follows_the_floor() {
        let p = params();
        let w = world(floor_at(10.0));
        let mut pl = Player::new(&p, v(0.0, 12.0, 0.0), true);
        assert_eq!(pl.motor, Motor::Walk);
        for _ in 0..100 {
            pl.step(&p, &w, &Input::default());
        }
        assert!(pl.grounded);
        let feet = pl.position.y - 1.5;
        assert!((feet - 10.0).abs() < 0.03, "feet at {feet}");
        for _ in 0..100 {
            pl.step(&p, &w, &forward());
        }
        let h = V3::new(pl.velocity.x, 0.0, pl.velocity.z).length();
        assert!((h - 3.5).abs() < 1e-6, "{h}");
        assert!(pl.grounded);
    }

    #[test]
    fn steps_up_the_step_offset_but_not_more() {
        let p = params();
        for (rise, climbs) in [(0.3, true), (0.6, false)] {
            let mut tris = floor_at(0.0);
            tris.extend(facing(
                quad(v(-50.0, 0.0, 2.0), v(100.0, 0.0, 0.0), v(0.0, rise, 0.0)),
                v(0.0, 0.0, 0.0),
            ));
            tris.extend(facing(
                quad(v(-50.0, rise, 2.0), v(100.0, 0.0, 0.0), v(0.0, 0.0, 50.0)),
                v(0.0, rise + 1.0, 10.0),
            ));
            let w = world(tris);
            let mut pl = Player::new(&p, v(0.0, 1.8, 0.0), true);
            for _ in 0..200 {
                pl.step(&p, &w, &forward());
            }
            let on_top = pl.position.z > 2.5;
            assert_eq!(on_top, climbs, "rise {rise}: {:?}", pl.position);
        }
    }

    #[test]
    fn slides_down_a_slope_steeper_than_the_limit() {
        let p = params();
        // 70° slope rising towards +z.
        let a = 70f64.to_radians();
        let run = 20.0 * a.cos();
        let rise = 20.0 * a.sin();
        let mut tris = floor_at(0.0);
        tris.extend(facing(
            quad(v(-50.0, 0.0, 0.0), v(100.0, 0.0, 0.0), v(0.0, rise, run)),
            v(0.0, 10.0, 3.0),
        ));
        let w = world(tris);
        // Above the slope at z = 3 (its surface is at 3·tan 70° ≈ 8.2 m).
        let mut pl = Player::new(&p, v(0.0, 10.0, 3.0), true);
        let start_z = pl.position.z;
        for _ in 0..300 {
            pl.step(&p, &w, &Input::default());
        }
        assert!(pl.position.z < start_z - 1.0, "{:?}", pl.position);
        // Walking up it does not get the player up it.
        let mut pl = Player::new(&p, v(0.0, 1.8, -2.0), true);
        for _ in 0..300 {
            pl.step(&p, &w, &forward());
        }
        assert!(pl.position.y < 4.0, "{:?}", pl.position);
    }

    #[test]
    fn jumping_reaches_the_base_height() {
        let p = params();
        let w = world(floor_at(0.0));
        let mut pl = Player::new(&p, v(0.0, 1.8, 0.0), true);
        for _ in 0..50 {
            pl.step(&p, &w, &Input::default());
        }
        let ground = pl.position.y;
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        let mut top = ground;
        let mut jumped = 0;
        for _ in 0..100 {
            jumped += pl
                .step(&p, &w, &jump)
                .iter()
                .filter(|e| **e == Event::Jumped)
                .count();
            top = top.max(pl.position.y);
        }
        // One jump per press; v = √(2·1·12), peak ≈ 1 m (discrete steps).
        assert_eq!(jumped, 1);
        assert!((top - ground - 1.0).abs() < 0.1, "{}", top - ground);
    }

    #[test]
    fn the_hand_reaches_two_metres_and_the_nearest_thing_wins() {
        let mut w = World::new();
        // A hand-only target 1.5 m ahead, a solid wall in front of it.
        w.insert(
            7,
            Body::new(
                facing(
                    quad(v(-1.0, -1.0, 1.5), v(2.0, 0.0, 0.0), v(0.0, 2.0, 0.0)),
                    V3::ZERO,
                ),
                vec![],
            )
            .with_groups(crate::collide::HAND),
        );
        let (d, hit) = hand_target(&w, V3::ZERO, 0.0, 0.0).unwrap();
        assert_eq!(hit.body, 7);
        assert!((d - 1.5).abs() < 0.02);
        w.insert(
            8,
            Body::new(
                facing(
                    quad(v(-1.0, -1.0, 1.0), v(2.0, 0.0, 0.0), v(0.0, 2.0, 0.0)),
                    V3::ZERO,
                ),
                vec![],
            ),
        );
        assert_eq!(hand_target(&w, V3::ZERO, 0.0, 0.0).unwrap().1.body, 8);
        // Out of reach: the sphere casts start r ahead and go 2 m, so the
        // 0.3 m one reaches 2.6 m (as the game's).
        let mut far = World::new();
        far.insert(
            9,
            Body::new(
                facing(
                    quad(v(-1.0, -1.0, 3.0), v(2.0, 0.0, 0.0), v(0.0, 2.0, 0.0)),
                    V3::ZERO,
                ),
                vec![],
            ),
        );
        assert!(hand_target(&far, V3::ZERO, 0.0, 0.0).is_none());
        // Just beside the ray: only the sphere casts find it.
        let mut side = World::new();
        side.insert(
            10,
            Body::new(
                vec![],
                vec![crate::collide::Shape::Sphere {
                    center: v(0.25, 0.0, 1.0),
                    radius: 0.05,
                }],
            ),
        );
        assert_eq!(hand_target(&side, V3::ZERO, 0.0, 0.0).unwrap().1.body, 10);
    }

    #[test]
    fn going_through_a_hatch_sets_the_pod_flag_and_the_pod_radius_counts() {
        let p = params();
        let w = World::new();
        // The lifepod's bottom exit and boarding end points, pod at origin.
        let (outside, inside) = (v(-1.08, -1.25, -0.08), v(-0.13, 1.94, -0.01));
        let mut pl = Player::new(&p, v(0.0, 2.1, 0.0), true);
        pl.pod_position = Some(V3::ZERO);
        pl.teleport(&p, outside, Some(false));
        assert!(!pl.in_pod && pl.position == outside);
        pl.step(&p, &w, &Input::default());
        assert_eq!(pl.motor, Motor::Swim);
        pl.teleport(&p, inside, Some(true));
        assert!(pl.in_pod && pl.position == inside);
        pl.step(&p, &w, &Input::default());
        assert_eq!(pl.motor, Motor::Walk);
        // Teleported 20 m away while flagged inside: the radius clears it.
        pl.position = v(20.0, 2.1, 0.0);
        let e = pl.step(&p, &w, &Input::default());
        assert!(e.contains(&Event::LeftPodRadius) && !pl.in_pod);

        // Through the triggers: the first-use exit hands over to the normal one.
        pl.teleport(&p, inside, Some(true));
        let exit = HatchTrigger {
            end: Some(outside),
            enters: false,
            exits: true,
            active: true,
        };
        let mut hatches = Hatches {
            triggers: vec![
                HatchTrigger {
                    active: false,
                    ..exit
                },
                exit,
            ],
            first_use: vec![(0, 1)],
        };
        assert_eq!(hatches.use_trigger(0, &mut pl, &p), None);
        let switched = hatches.use_trigger(1, &mut pl, &p).unwrap();
        assert_eq!(switched, vec![(1, false), (0, true)]);
        assert!(!pl.in_pod && pl.position == outside);
        assert!(hatches.triggers[0].active && !hatches.triggers[1].active);
    }
}
