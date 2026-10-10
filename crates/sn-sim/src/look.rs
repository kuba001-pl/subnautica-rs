//! The first-person mouse look (M9c): `MainCameraControl.OnUpdate` in
//! normal play, with the mouse delta scaled as `GameInputSystem.GetVector2`
//! does for `Look`. Our own code after reading the game's; the
//! sensitivity and the limits come from the install ([`LookParams`]).
//!
//! The game keeps two angles in degrees: `rotationX` (yaw, unbounded) and
//! `rotationY` (pitch, up positive, clamped). There is no smoothing of the
//! look itself (camera bob, tilt and impact bob are separate).

/// From the game's data (`sn-assets`' `look_params`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookParams {
    /// `MouseSensitivity` (the options slider; the default is
    /// `GameInputSystem.defaultMouseSensitivity`).
    pub mouse_sensitivity: f64,
    /// `InvertMouse`.
    pub invert: bool,
    /// `MainCameraControl.minimumY` / `maximumY`, degrees, up positive.
    pub minimum_y: f64,
    pub maximum_y: f64,
}

impl LookParams {
    /// Degrees per unit of mouse delta: the sensitivity × 1.5 × 0.5
    /// (`GameInputSystem.GetVector2`, the `Delta` case).
    pub fn degrees_per_count(&self) -> f64 {
        self.mouse_sensitivity * 1.5 * 0.5
    }
}

/// The look angles.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    /// `rotationX`, degrees.
    pub rotation_x: f64,
    /// `rotationY`, degrees, up positive.
    pub rotation_y: f64,
}

impl Look {
    /// Facing `yaw` radians (about +y, Unity's), level.
    pub fn facing(yaw: f64) -> Look {
        Look {
            rotation_x: yaw.to_degrees(),
            rotation_y: 0.0,
        }
    }

    /// One frame of mouse movement: `dx` right, `dy` up (Unity's mouse
    /// delta convention), in the mouse's own units.
    pub fn apply(&mut self, params: &LookParams, dx: f64, dy: f64) {
        let k = params.degrees_per_count();
        let dy = if params.invert { -dy } else { dy };
        self.rotation_x += dx * k;
        self.rotation_y = (self.rotation_y + dy * k).clamp(params.minimum_y, params.maximum_y);
    }

    /// Yaw about +y, radians (`sn_sim::player::Input::yaw`).
    pub fn yaw(&self) -> f64 {
        self.rotation_x.to_radians()
    }

    /// Pitch about +x, radians, positive looks down
    /// (`sn_sim::player::Input::pitch`): the camera's x angle is
    /// −`rotationY` (split between `cameraUPTransform` and the camera).
    pub fn pitch(&self) -> f64 {
        (-self.rotation_y).to_radians()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> LookParams {
        LookParams {
            mouse_sensitivity: 0.15,
            invert: false,
            minimum_y: -87.0,
            maximum_y: 87.0,
        }
    }

    #[test]
    fn scale_is_the_games() {
        let p = params();
        assert!((p.degrees_per_count() - 0.1125).abs() < 1e-12);
        let mut l = Look::default();
        l.apply(&p, 100.0, 0.0);
        assert!((l.rotation_x - 11.25).abs() < 1e-9);
        // Mouse up looks up: pitch (down positive) goes negative.
        l.apply(&p, 0.0, 40.0);
        assert!((l.rotation_y - 4.5).abs() < 1e-9);
        assert!((l.pitch() + 4.5f64.to_radians()).abs() < 1e-12);
    }

    #[test]
    fn pitch_stops_at_the_limits_and_yaw_does_not() {
        let p = params();
        let mut l = Look::default();
        for _ in 0..100 {
            l.apply(&p, 1000.0, 1000.0);
        }
        assert_eq!(l.rotation_y, 87.0);
        assert!((l.rotation_x - 100.0 * 112.5).abs() < 1e-6);
        l.apply(&p, 0.0, -1e9);
        assert_eq!(l.rotation_y, -87.0);
        // Coming back from the limit is immediate (no stored overshoot).
        l.apply(&p, 0.0, 10.0);
        assert!((l.rotation_y - (-87.0 + 1.125)).abs() < 1e-9);
    }

    #[test]
    fn invert_flips_pitch_only() {
        let p = LookParams {
            invert: true,
            ..params()
        };
        let mut l = Look::default();
        l.apply(&p, 10.0, 10.0);
        assert!(l.rotation_x > 0.0 && l.rotation_y < 0.0);
    }

    #[test]
    fn facing_keeps_the_yaw() {
        let l = Look::facing(1.25);
        assert!((l.yaw() - 1.25).abs() < 1e-12);
        assert_eq!(l.pitch(), 0.0);
    }
}
