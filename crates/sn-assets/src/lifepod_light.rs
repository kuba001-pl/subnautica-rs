//! Lifepod 5's own light (M7f4g): `MarmoLifepodSky` (the pod's sky, global
//! while the player is inside), its `LightingController` (skies and lights
//! per state) and what the intro's end does to them
//! (`EscapePodCinematicControl.StopAll`). See `docs/formats/gameplay.md`
//! § The lifepod's light.

use sn_unity::{
    EscapePodCinematicControl, LightingController, MarmoSky, MonoBehaviourHeader, PPtr,
    parse_marmo_lifepod_sky,
};
use sn_world::Transform;

use crate::marmo::BiomeSky;
use crate::scene::Scene;
use crate::{Assets, Result};

const ANIMATOR: i32 = 95;
const LIGHT: i32 = 108;

/// One of the controller's lights, found in the scene.
#[derive(Clone, Debug)]
pub struct ControlledLight {
    /// (root, node) of its GameObject.
    pub node: (usize, usize),
    pub light: sn_unity::Light,
    /// World placement (after the pod was placed).
    pub world: Transform,
    /// Its intensity per state (`MultiStatesLight.intensities`).
    pub intensities: Vec<f32>,
}

/// The pod's lighting as stored in the scene.
#[derive(Clone, Debug)]
pub struct LifepodLighting {
    /// `MarmoLifepodSky.anchorSky`, with its world rotation.
    pub sky: BiomeSky,
    pub controller: LightingController,
    /// For each of the controller's skies: whether it is the anchor sky
    /// (the only sky we draw with; others are counted, not applied).
    pub controls_anchor: Vec<bool>,
    /// The controller's lights that resolve to a `Light` in the scene.
    pub lights: Vec<ControlledLight>,
    /// Controller lights that don't resolve.
    pub missing_lights: usize,
}

impl Scene {
    /// The pod's `MarmoLifepodSky` and `LightingController`, if the scene
    /// has both (call after [`Scene::place_escape_pod`]).
    pub fn lifepod_lighting(&self, assets: &Assets) -> Result<Option<LifepodLighting>> {
        let big_endian = self.file.file().big_endian;
        let Some(marmo) = self
            .behaviours(assets, "MarmoLifepodSky")
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let Some(control) = self
            .behaviours(assets, "LightingController")
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let (_, data) = marmo.data()?;
        let anchor = parse_marmo_lifepod_sky(data, big_endian)
            .map_err(|e| format!("MarmoLifepodSky: {e}"))?;
        let sky_ref = assets
            .resolve(&self.file, anchor)?
            .ok_or("MarmoLifepodSky: anchorSky is null")?;
        let (_, data) = sky_ref.data()?;
        let header = MonoBehaviourHeader::parse(data, big_endian).map_err(|e| e.to_string())?;
        let sky = MarmoSky::parse(data, big_endian).map_err(|e| format!("anchor Sky: {e}"))?;
        let (name, rotation) = match assets
            .resolve(&self.file, header.game_object)?
            .and_then(|go| self.locate(&go))
        {
            Some((r, n)) => (
                self.roots[r].nodes[n].name.clone(),
                self.roots[r].world(n).rotation,
            ),
            None => ("?".to_string(), [0.0, 0.0, 0.0, 1.0]),
        };

        let (_, data) = control.data()?;
        let controller = LightingController::parse(data, big_endian)
            .map_err(|e| format!("LightingController: {e}"))?;
        let controls_anchor = controller
            .skies
            .iter()
            .map(|s| same(s.sky, anchor))
            .collect();
        let mut lights = Vec::new();
        let mut missing_lights = 0;
        for l in &controller.lights {
            match self.light(assets, l.light)? {
                Some((node, light)) => lights.push(ControlledLight {
                    node,
                    light,
                    world: self.roots[node.0].world(node.1),
                    intensities: l.intensities.clone(),
                }),
                None => missing_lights += 1,
            }
        }
        Ok(Some(LifepodLighting {
            sky: BiomeSky {
                name,
                sky,
                rotation,
            },
            controller,
            controls_anchor,
            lights,
            missing_lights,
        }))
    }

    /// A `Light` component of the scene and the node it sits on.
    fn light(
        &self,
        assets: &Assets,
        pptr: PPtr,
    ) -> Result<Option<((usize, usize), sn_unity::Light)>> {
        let Some(obj) = assets.resolve(&self.file, pptr)? else {
            return Ok(None);
        };
        let (info, data) = obj.data()?;
        if info.class_id != LIGHT {
            return Ok(None);
        }
        let light = sn_unity::Light::parse(data, obj.file.file().big_endian)
            .map_err(|e| format!("Light {}: {e}", obj.path_id))?;
        let Some(node) = assets
            .resolve(&obj.file, light.game_object)?
            .and_then(|go| self.locate(&go))
        else {
            return Ok(None);
        };
        Ok(Some((node, light)))
    }

    /// `EscapePodCinematicControl.StopAll` (the intro ends or is skipped):
    /// the lights animator is disabled and the hatch light deactivated
    /// (its effects and sky curve stop with it; we play neither). Returns
    /// the names of the animator's and the hatch light's nodes changed.
    pub fn stop_pod_intro(&mut self, assets: &Assets) -> Result<Vec<String>> {
        let big_endian = self.file.file().big_endian;
        let Some(b) = self
            .behaviours(assets, "EscapePodCinematicControl")
            .into_iter()
            .next()
        else {
            return Ok(Vec::new());
        };
        let (_, data) = b.data()?;
        let c = EscapePodCinematicControl::parse(data, big_endian)
            .map_err(|e| format!("EscapePodCinematicControl: {e}"))?;
        let mut changed = Vec::new();
        if let Some(animator) = assets.resolve(&self.file, c.lights_animator)? {
            let (info, data) = animator.data()?;
            if info.class_id == ANIMATOR {
                let a = sn_unity::Animator::parse(data, big_endian)
                    .map_err(|e| format!("lights Animator: {e}"))?;
                if let Some((r, n)) = assets
                    .resolve(&self.file, a.game_object)?
                    .and_then(|go| self.locate(&go))
                {
                    let node = &mut self.roots[r].nodes[n];
                    if let Some(na) = node.animator.as_mut() {
                        na.component.enabled = false;
                        changed.push(node.name.clone());
                    }
                }
            }
        }
        if let Some(go) = assets.resolve(&self.file, c.hatch_light)?
            && let Some((r, n)) = self.locate(&go)
        {
            self.roots[r].set_active(n, false);
            changed.push(self.roots[r].nodes[n].name.clone());
        }
        Ok(changed)
    }
}

/// Two references from the same file to the same object.
fn same(a: PPtr, b: PPtr) -> bool {
    a.file_id == b.file_id && a.path_id == b.path_id
}
