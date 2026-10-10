//! Lifepod 5's own light (M7f4g, `docs/DESIGN.md` § 4.4): the global
//! Marmoset sky is the pod's while the player is inside
//! (`MarmoLifepodSky`), the pod's `LightingController` sets that sky's
//! intensities and its lights per state (`sn_sim::lighting`), and the
//! modules spawned in the pod take its sky (`SkyApplier` below a
//! `MarmoLifepodSky`).

use std::time::Instant;

use bevy::prelude::*;
use sn_assets::{Assets as GameAssets, BiomeSky, Scene};
use sn_sim::lighting::{
    LightingController, SkyIntensities, StatesEmissive, StatesLight, StatesSky, state_name,
};
use sn_world::Transform as Placement;

use crate::object_look::ObjectMaterial;
use crate::objects::{LocalLight, ObjectStreamer, SkyLook, spawn_light, to_bevy};
use crate::player::PlayerSim;
use crate::terrain_look::TerrainLook;

/// `Player.escapePodRadius`: beyond it from the pod the player is out of
/// it (`Player.ValidateEscapePod`).
const POD_RADIUS: f32 = 15.0;

/// With the fly camera there is no player: whether the camera starts in
/// the pod (at its player spawn).
#[derive(Resource)]
pub struct FreeCamInPod(pub bool);

/// One of the controller's lights.
pub struct Lamp {
    name: String,
    spot: bool,
    /// As stored (sRGB).
    color: [f32; 3],
    range: f32,
    spot_angle: f32,
    world: Placement,
    /// The GameObject's parent is active (the light shows when its own
    /// GameObject is switched on too).
    parent_active: bool,
}

/// What the worker reads from the escapepod scene.
pub struct PodLightData {
    sky: BiomeSky,
    controller: LightingController,
    lamps: Vec<Lamp>,
    pod_position: [f32; 3],
    /// The controller's skies other than the anchor sky (not drawn).
    other_skies: usize,
    missing_lights: usize,
    /// What `EscapePodCinematicControl.StopAll` switched off.
    intro_stopped: Vec<String>,
    stored_state: usize,
}

impl PodLightData {
    /// Reads the pod's lighting from its scene (placed at `point`), as the
    /// intro leaves it (`StopAll`), and snaps the controller to `state`
    /// (the intro played or skipped ends in `Damaged`).
    pub fn read(
        assets: &GameAssets,
        scene: &mut Scene,
        point: [f32; 3],
        state: usize,
    ) -> Result<Option<PodLightData>, String> {
        let intro_stopped = scene.stop_pod_intro(assets)?;
        let Some(l) = scene.lifepod_lighting(assets)? else {
            return Ok(None);
        };
        let stored = SkyIntensities {
            master: l.sky.sky.master_intensity,
            diffuse: l.sky.sky.diff_intensity,
            specular: l.sky.sky.spec_intensity,
        };
        let mut skies = Vec::new();
        let mut other_skies = 0;
        for (s, &anchor) in l.controller.skies.iter().zip(&l.controls_anchor) {
            if anchor {
                skies.push(StatesSky::new(
                    s.master.clone(),
                    s.diffuse.clone(),
                    s.specular.clone(),
                    stored,
                ));
            } else {
                other_skies += 1;
            }
        }
        let mut lights = Vec::new();
        let mut lamps = Vec::new();
        for c in &l.lights {
            let root = &scene.roots[c.node.0];
            let node = &root.nodes[c.node.1];
            let parent_active = node.parent.is_none_or(|p| root.nodes[p].active);
            lights.push(StatesLight::new(
                c.intensities.clone(),
                c.light.intensity,
                node.active_self,
            ));
            lamps.push(Lamp {
                name: node.name.clone(),
                spot: c.light.kind == sn_unity::LightKind::Spot,
                color: [c.light.color[0], c.light.color[1], c.light.color[2]],
                range: c.light.range,
                spot_angle: c.light.spot_angle,
                world: c.world,
                parent_active,
            });
        }
        let stored_state = usize::try_from(l.controller.state).unwrap_or(0);
        let mut controller = LightingController::new(
            stored_state,
            l.controller.fade_duration,
            skies,
            lights,
            StatesEmissive::new(l.controller.emissive.clone()),
        );
        controller.snap_to_state(state);
        Ok(Some(PodLightData {
            sky: l.sky,
            controller,
            lamps,
            pod_position: point,
            other_skies,
            missing_lights: l.missing_lights,
            intro_stopped,
            stored_state,
        }))
    }

    /// The pod's sky with the controller's current intensities.
    pub fn look(&self) -> SkyLook {
        let mut sky = self.sky.clone();
        if let Some(s) = self.controller.skies.first() {
            sky.sky.master_intensity = s.current.master;
            sky.sky.diff_intensity = s.current.diffuse;
            sky.sky.spec_intensity = s.current.specular;
        }
        SkyLook::new(&sky)
    }
}

/// The pod's lighting while the game runs.
#[derive(Resource)]
pub struct PodLight {
    data: PodLightData,
    lamps: Vec<Option<Entity>>,
    /// The sky's intensities last applied to the materials.
    applied: Option<SkyIntensities>,
    /// The lamps' (on, intensity) last applied.
    lamp_state: Vec<(bool, f32)>,
    /// What was last logged (logged once a fade has settled).
    logged_sky: Option<SkyIntensities>,
    logged_lamps: Vec<Option<(bool, f32)>>,
    in_pod: Option<bool>,
    /// The last relight of the pod sky's materials: count, milliseconds.
    relit: (usize, f64),
    /// The fly camera went beyond the pod's radius.
    left: bool,
    logged: bool,
}

impl PodLight {
    pub fn new(data: PodLightData) -> PodLight {
        let n = data.lamps.len();
        PodLight {
            data,
            lamps: vec![None; n],
            applied: None,
            lamp_state: vec![(false, -1.0); n],
            logged_sky: None,
            logged_lamps: vec![None; n],
            in_pod: None,
            relit: (0, 0.0),
            left: false,
            logged: false,
        }
    }

    /// The values read, per state, for the log.
    fn log_read(&self) {
        let d = &self.data;
        let c = &d.controller;
        info!(
            "lifepod light: sky {:?} (stored master {}, diffuse {}, specular {}; affected by the day {}), controller stored state {} ({}), fade {} s, {} sky, {} lights ({} missing), {} other skies; intro stopped: {:?}",
            d.sky.name,
            d.sky.sky.master_intensity,
            d.sky.sky.diff_intensity,
            d.sky.sky.spec_intensity,
            d.sky.sky.affected_by_day_night,
            d.stored_state,
            state_name(d.stored_state),
            c.fade_duration,
            c.skies.len(),
            c.lights.len(),
            d.missing_lights,
            d.other_skies,
            d.intro_stopped
        );
        for state in 0..3 {
            let sky = c
                .skies
                .first()
                .map(|s| [&s.master, &s.diffuse, &s.specular].map(|v| v.get(state).copied()));
            let lights: Vec<Option<f32>> = c
                .lights
                .iter()
                .map(|l| l.intensities.get(state).copied())
                .collect();
            info!(
                "lifepod light: state {state} ({}): sky master/diffuse/specular {sky:?}, lights {lights:?}, emissive {:?}",
                state_name(state),
                c.emissive.intensities.get(state)
            );
        }
        for (lamp, l) in d.lamps.iter().zip(&c.lights) {
            info!(
                "lifepod light: lamp {:?} ({}, range {} m, colour {:?}) now {} at intensity {}",
                lamp.name,
                if lamp.spot { "spot" } else { "point" },
                lamp.range,
                lamp.color,
                if l.active { "on" } else { "off" },
                l.intensity
            );
        }
    }
}

/// Every frame: the controller, the global sky by where the player is, the
/// pod's sky and lights.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub fn update(
    mut commands: Commands,
    time: Res<Time>,
    pod: Option<ResMut<PodLight>>,
    mut streamer: ResMut<ObjectStreamer>,
    mut materials: ResMut<Assets<ObjectMaterial>>,
    mut terrain_look: Option<ResMut<TerrainLook>>,
    player: Option<Res<PlayerSim>>,
    free_cam: Res<FreeCamInPod>,
    camera: Query<&Transform, With<Camera3d>>,
    mut points: Query<&mut PointLight>,
    mut spots: Query<&mut SpotLight>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(mut pod) = pod else {
        return;
    };
    let pod = &mut *pod;
    if !pod.logged {
        pod.log_read();
        pod.logged = true;
    }
    pod.data.controller.update(time.delta_secs());

    // `Player.escapePod`.
    let in_pod = match player.as_deref() {
        Some(p) => p.in_pod(),
        None => {
            if let Ok(t) = camera.single() {
                let at = Vec3::new(t.translation.x, t.translation.y, -t.translation.z);
                if at.distance(Vec3::from(pod.data.pod_position)) > POD_RADIUS {
                    pod.left = true;
                }
            }
            Some(free_cam.0 && !pod.left)
        }
    };
    if let Some(inside) = in_pod
        && pod.in_pod != Some(inside)
    {
        let start = Instant::now();
        let n = streamer.set_in_pod(inside, &mut materials, terrain_look.as_deref_mut());
        info!(
            "lifepod light: player {} the pod: global sky {} ({n} materials relit in {:.2} ms)",
            if inside { "in" } else { "out of" },
            streamer.global_sky_name(&pod.data.sky.name),
            start.elapsed().as_secs_f64() * 1000.0
        );
        pod.in_pod = Some(inside);
    }

    // The pod's sky, as the controller sets it.
    let now = pod.data.controller.skies.first().map(|s| s.current);
    let fading = pod.data.controller.fading();
    if now.is_some() && now != pod.applied {
        let start = Instant::now();
        let n = streamer.set_pod_sky(pod.data.look(), &mut materials, terrain_look.as_deref_mut());
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        pod.relit = (n, ms);
        pod.applied = now;
    }
    if !fading
        && now != pod.logged_sky
        && let Some(s) = now
    {
        info!(
            "lifepod light: sky {:?} at master {}, diffuse {}, specular {} (state {}; last relight {} materials in {:.2} ms)",
            pod.data.sky.name,
            s.master,
            s.diffuse,
            s.specular,
            state_name(pod.data.controller.state),
            pod.relit.0,
            pod.relit.1
        );
        pod.logged_sky = now;
    }

    // The controller's lights.
    if !streamer.lights_enabled() {
        return;
    }
    for (i, lamp) in pod.data.lamps.iter().enumerate() {
        let Some(l) = pod.data.controller.lights.get(i) else {
            continue;
        };
        let on = l.active && lamp.parent_active;
        if !fading && pod.logged_lamps[i] != Some((on, l.intensity)) {
            info!(
                "lifepod light: lamp {:?} {} at intensity {}",
                lamp.name,
                if on { "on" } else { "off" },
                l.intensity
            );
            pod.logged_lamps[i] = Some((on, l.intensity));
        }
        if pod.lamp_state[i] == (on, l.intensity) {
            continue;
        }
        let gamma = Vec3::from(lamp.color) * l.intensity;
        let color = crate::sky::linear(gamma);
        let entity = *pod.lamps[i].get_or_insert_with(|| {
            let light = LocalLight {
                spot: lamp.spot,
                color: color.to_array(),
                range: lamp.range,
                spot_angle: lamp.spot_angle,
                local: lamp.world,
            };
            spawn_light(&mut commands, &light, to_bevy(&lamp.world))
        });
        let c = Color::linear_rgb(color.x, color.y, color.z);
        if let Ok(mut p) = points.get_mut(entity) {
            p.color = c;
        }
        if let Ok(mut s) = spots.get_mut(entity) {
            s.color = c;
        }
        let shown = if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        match visibility.get_mut(entity) {
            Ok(mut v) => *v = shown,
            Err(_) => {
                commands.entity(entity).insert(shown);
            }
        }
        pod.lamp_state[i] = (on, l.intensity);
    }
}
