//! The UI camera and where `uGUI_CanvasScaler` puts a canvas (M9u5b,
//! `docs/formats/ui.md` § Canvas placement), ported from the decompiled
//! game code (read only; the code is ours):
//! - the UI camera (`MainCamera (UI)`) is unparented at the world origin
//!   with no rotation (`SNCameraRoot.Start`), and takes the main camera's
//!   field of view, near and far planes (`SNCameraRoot.Awake`,
//!   `SyncFieldOfView`);
//! - `UpdateTransform` (Static mode): the canvas at the UI camera's
//!   position + forward × `distance`, with its rotation;
//! - `UpdateFrustum`: the frustum's size at `distance` from the **main**
//!   camera's field of view and the screen's aspect; the scale factor
//!   `min(screen ÷ reference)` (× the settings' UI scale unless
//!   `DontScale`); the canvas's size the screen ÷ that factor; its local
//!   scale such that the canvas exactly fills the frustum;
//! - `SetScaleFactor`: `Canvas.scaleFactor` = that factor.
//!
//! Unity's conventions: left handed, forward +z, up +y; screen pixels from
//! the bottom left. VR (`XRSettings.enabled`) is not ported.

use crate::draw::{Affine, apply, invert, trs};
use crate::layout::Tree;

/// `uGUI_CanvasScaler.Mode`.
pub const MODE_STATIC: i32 = 0;
pub const MODE_INVERSED: i32 = 1;
pub const MODE_PARENTED: i32 = 2;
pub const MODE_WORLD: i32 = 3;
/// `uGUI_CanvasScaler.ScaleMode`.
pub const SCALE_DONT: i32 = 0;

/// `MiscSettings.fieldOfView`'s default (the main camera's).
pub const DEFAULT_FIELD_OF_VIEW: f32 = 60.0;
/// `MiscSettings.uiScale`'s default on desktop.
pub const DEFAULT_UI_SCALE: f32 = 1.0;

/// A canvas's `uGUI_CanvasScaler` fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScalerSettings {
    pub reference_resolution: [f32; 2],
    pub mode: i32,
    pub distance: f32,
    pub scale_mode: i32,
}

/// A perspective camera in Unity's conventions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiCamera {
    pub position: [f32; 3],
    /// x, y, z, w.
    pub rotation: [f32; 4],
    /// Vertical, degrees.
    pub field_of_view: f32,
    pub near: f32,
    pub far: f32,
}

impl UiCamera {
    /// The game's UI camera with the main camera's field of view and
    /// planes (`SNCameraRoot`).
    pub fn game(field_of_view: f32, near: f32, far: f32) -> UiCamera {
        UiCamera {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            field_of_view,
            near,
            far,
        }
    }

    /// `transform.forward`.
    pub fn forward(&self) -> [f32; 3] {
        let m = trs([0.0; 3], self.rotation, [1.0; 3]);
        apply(&m, [0.0, 0.0, 1.0])
    }

    /// Where a world point lands on a `screen`-sized view, in pixels from
    /// the bottom left (`Camera.WorldToScreenPoint`); `None` behind the
    /// camera.
    pub fn project(&self, p: [f32; 3], screen: [i32; 2]) -> Option<[f32; 2]> {
        // Into the camera's space: inverse rotation of the offset.
        let [x, y, z, w] = self.rotation;
        let inverse = trs([0.0; 3], [-x, -y, -z, w], [1.0; 3]);
        let d = [
            p[0] - self.position[0],
            p[1] - self.position[1],
            p[2] - self.position[2],
        ];
        let c = apply(&inverse, d);
        if c[2] <= 0.0 {
            return None;
        }
        let tan = (self.field_of_view.to_radians() * 0.5).tan();
        let aspect = screen[0] as f32 / screen[1] as f32;
        let ndc = [c[0] / (c[2] * tan * aspect), c[1] / (c[2] * tan)];
        Some([
            (ndc[0] + 1.0) * 0.5 * screen[0] as f32,
            (ndc[1] + 1.0) * 0.5 * screen[1] as f32,
        ])
    }

    /// The direction (world, not normalized) of the ray from the camera
    /// through `pixel` (from the bottom left; `Camera.ScreenPointToRay`'s
    /// direction, M9u8a4b): [`UiCamera::project`]'s inverse.
    pub fn screen_ray(&self, pixel: [f32; 2], screen: [i32; 2]) -> [f32; 3] {
        let tan = (self.field_of_view.to_radians() * 0.5).tan();
        let aspect = screen[0] as f32 / screen[1] as f32;
        let ndc = [
            2.0 * pixel[0] / screen[0] as f32 - 1.0,
            2.0 * pixel[1] / screen[1] as f32 - 1.0,
        ];
        let c = [ndc[0] * tan * aspect, ndc[1] * tan, 1.0];
        apply(&trs([0.0; 3], self.rotation, [1.0; 3]), c)
    }
}

/// Where the scaler puts a canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    /// The canvas's rect size, in canvas units (`SetSizeWithCurrentAnchors`).
    pub size: [f32; 2],
    /// `localScale`: (x, y, x).
    pub scale: [f32; 3],
    /// `Canvas.scaleFactor`.
    pub scale_factor: f32,
}

impl Placement {
    /// From the canvas's space (its pivot at the origin) to the world.
    pub fn world_from_canvas(&self) -> Affine {
        trs(self.position, self.rotation, self.scale)
    }

    /// Where the ray from `origin` along `dir` meets the canvas's plane,
    /// in the canvas's space (its pivot at the origin); `None` parallel to
    /// it or behind the origin (M9u8a4b: a click on a world-space canvas,
    /// as `GraphicRaycaster` finds it).
    pub fn ray_point(&self, origin: [f32; 3], dir: [f32; 3]) -> Option<[f32; 2]> {
        let m = self.world_from_canvas();
        let to_canvas = invert(&m)?;
        let o = apply(&to_canvas, origin);
        // A direction: the linear part only.
        let at = apply(
            &to_canvas,
            [origin[0] + dir[0], origin[1] + dir[1], origin[2] + dir[2]],
        );
        let d = [at[0] - o[0], at[1] - o[1], at[2] - o[2]];
        if d[2].abs() < 1e-12 {
            return None;
        }
        let s = -o[2] / d[2];
        (s >= 0.0).then(|| [o[0] + s * d[0], o[1] + s * d[1]])
    }

    /// The canvas rect's corners in the world (bottom left, top left, top
    /// right, bottom right), for a pivot (0–1).
    pub fn corners(&self, pivot: [f32; 2]) -> [[f32; 3]; 4] {
        let m = self.world_from_canvas();
        let x0 = -pivot[0] * self.size[0];
        let y0 = -pivot[1] * self.size[1];
        let (x1, y1) = (x0 + self.size[0], y0 + self.size[1]);
        [
            apply(&m, [x0, y0, 0.0]),
            apply(&m, [x0, y1, 0.0]),
            apply(&m, [x1, y1, 0.0]),
            apply(&m, [x1, y0, 0.0]),
        ]
    }
}

/// `UpdateTransform` then `UpdateFrustum` for a Static-mode canvas: its
/// pose in front of `ui_camera`, sized from the main camera's
/// `main_field_of_view` on a `screen`. Other modes follow an anchor (the
/// PDA screen, M9u8a) and are an error here.
pub fn place(
    s: &ScalerSettings,
    ui_camera: &UiCamera,
    main_field_of_view: f32,
    screen: [i32; 2],
    ui_scale: f32,
) -> Result<Placement, String> {
    if s.mode != MODE_STATIC {
        return Err(format!("scaler mode {} is not ported", s.mode));
    }
    if screen[0] <= 0 || screen[1] <= 0 {
        return Err(format!("screen {}×{}", screen[0], screen[1]));
    }
    if s.reference_resolution[0] <= 0.0 || s.reference_resolution[1] <= 0.0 {
        return Err("reference resolution not positive".into());
    }
    let f = ui_camera.forward();
    let position = [
        ui_camera.position[0] + f[0] * s.distance,
        ui_camera.position[1] + f[1] * s.distance,
        ui_camera.position[2] + f[2] * s.distance,
    ];
    let (sw, sh) = (screen[0] as f32, screen[1] as f32);
    let aspect = sw / sh;
    let half = s.distance * (main_field_of_view * 0.5 * (std::f32::consts::PI / 180.0)).tan();
    let frustum_h = half * 2.0;
    let frustum_w = half * aspect * 2.0;
    let per_pixel = [frustum_w / sw, frustum_h / sh];
    let mut factor = (sw / s.reference_resolution[0]).min(sh / s.reference_resolution[1]);
    if s.scale_mode > SCALE_DONT {
        factor *= ui_scale;
    }
    let inverse = 1.0 / factor;
    let scale_x = per_pixel[0] * factor;
    let scale_y = per_pixel[1] * factor;
    Ok(Placement {
        position,
        rotation: ui_camera.rotation,
        size: [sw * inverse, sh * inverse],
        scale: [scale_x, scale_y, scale_x],
        scale_factor: factor,
    })
}

/// `RectTransform.SetSizeWithCurrentAnchors` on both axes, as
/// `UpdateFrustum` sizes the canvas: `sizeDelta = size − parent size ×
/// (anchorMax − anchorMin)`; and its `localScale`. The tree must be laid
/// out again after.
pub fn apply_to_tree(tree: &mut Tree, node: usize, p: &Placement) {
    let parent = match tree.nodes.get(node).and_then(|n| n.parent) {
        Some(q) => {
            let r = tree.rect(q);
            [r.width, r.height]
        }
        None => [0.0, 0.0],
    };
    let Some(n) = tree.nodes.get_mut(node) else {
        return;
    };
    for (axis, parent_size) in parent.into_iter().enumerate() {
        let span = n.rect.anchor_max[axis] - n.rect.anchor_min[axis];
        n.rect.size_delta[axis] = p.size[axis] - parent_size * span;
    }
    n.rect.scale = p.scale;
}

#[cfg(test)]
mod tests;
