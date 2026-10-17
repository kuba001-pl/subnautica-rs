//! Named, fixed-camera benchmark evidence (M8g0a). All output stays in out/.
//! Animation and water time are recorded, not frozen by this mode.

use std::fmt::Write as _;
use std::path::PathBuf;

use bevy::diagnostic::DiagnosticsStore;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::aurora::GameClock;
use crate::objects::{LodSettings, LodStats, ObjectStreamer};
use crate::sky::SkyState;
use crate::terrain::StreamStats;
use crate::water::{WaterFog, WaterWorld};
use crate::water_surface::SurfaceClock;
use crate::{Args, Setup, SpawnedLights};

/// One path component, also used for the explicit revision label.
pub fn valid_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub fn positive_integer(s: Option<&String>, flag: &str) -> Result<usize, String> {
    s.and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .ok_or_else(|| format!("{flag} needs a whole number above 0"))
}

/// Require consecutive settled frames before measurement starts.
#[derive(Default)]
pub struct Warmup {
    pub target: usize,
    consecutive: usize,
}

impl Warmup {
    pub fn new(target: usize) -> Self {
        Self {
            target,
            consecutive: 0,
        }
    }

    pub fn ready(&mut self, settled: bool) -> bool {
        if !settled {
            self.consecutive = 0;
            return false;
        }
        self.consecutive = self.consecutive.saturating_add(1);
        self.consecutive >= self.target
    }
}

#[derive(Debug, PartialEq)]
pub struct FrameSummary {
    pub mean: f64,
    pub median: f64,
    pub p95: f64,
    pub p99: f64,
    pub worst: f64,
}

/// Nearest-rank tail percentiles; conventional median for an even count.
pub fn frame_summary(samples: &[f32]) -> Option<FrameSummary> {
    if samples.is_empty() || samples.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f32::total_cmp);
    let n = sorted.len();
    let rank = |percent: f64| -> f64 {
        f64::from(
            sorted[((n as f64 * percent).ceil() as usize)
                .saturating_sub(1)
                .min(n - 1)],
        )
    };
    let mid = n / 2;
    let median = if n % 2 == 0 {
        (f64::from(sorted[mid - 1]) + f64::from(sorted[mid])) * 0.5
    } else {
        f64::from(sorted[mid])
    };
    Some(FrameSummary {
        mean: sorted.iter().map(|&v| f64::from(v)).sum::<f64>() / n as f64,
        median,
        p95: rank(0.95),
        p99: rank(0.99),
        worst: f64::from(sorted[n - 1]),
    })
}

#[derive(Resource)]
pub struct Baseline {
    pub directory: PathBuf,
    config: String,
}

impl Baseline {
    pub fn create(name: &str, revision: &str) -> Result<Self, String> {
        if !valid_label(name) || !valid_label(revision) {
            return Err("baseline name/revision needs 1–96 letters, digits, - or _".into());
        }
        // Prefix avoids Windows device names such as CON and AUX.
        let root = PathBuf::from("out/baselines");
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        let directory = root.join(format!("run-{name}"));
        std::fs::create_dir(&directory).map_err(|e| {
            format!(
                "{}: {e}; choose a new baseline name (existing runs are preserved)",
                directory.display()
            )
        })?;
        Ok(Self {
            directory,
            config: format!("baseline_schema=1\nname={name}\nrevision_label={revision}\n"),
        })
    }

    pub fn configure(
        &mut self,
        args: &Args,
        build: &str,
        fingerprint: u64,
        world: &sn_protocol::WorldSettings,
    ) {
        // Explicit allowlist: never dump raw arguments, user names or environment.
        let _ = writeln!(
            self.config,
            "game_build={build:?}\nrules_fingerprint={fingerprint:016x}\n\
             package_version={}\nprofile={}\n\
             requested_physical_resolution={:?}\nrequested_frames={:?}\n\
             warmup_settled_frames={}\nview_metres={}\n\
             lifepod_seed={}\nslot_seed={:?}\nlifepod_position={:?}\n\
             lifepod_lighting_state={}\naurora_held={:?}\naurora_countdown_override={:?}\n\
             start_hours={}\ntime_scale={}\nwater_quality={:?}\ncolor_grading={:?}\n\
             fog_unit={}\ngpu_timings={}\n\
             debug_colours={}\nno_objects={}\nno_local_lights={}\n\
             no_slots={}\nno_placeholders={}\nno_grass={}\nno_animation={}\n\
             no_lod={}\nno_scenes={}\nno_water_fog={}\nno_water_surface={}\ngame_ui={}\n\
             visual_time_policy=live; animation, waves and foam history are NOT deterministic\n\
             reference_comparison=not tested\nframe_time_source=Time<Real> (unclamped)\n\
             gpu_scope=recent Bevy diagnostic history at measurement end; not the full sample interval\n\
             memory_scope=peak sampled process memory since loading began\n",
            env!("CARGO_PKG_VERSION"),
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            args.resolution,
            args.benchmark,
            args.warmup.unwrap_or(120),
            args.view,
            args.lifepod_seed,
            args.slot_seed,
            args.scenes.as_ref().and_then(|s| s.lifepod),
            args.lifepod_state,
            args.aurora_held,
            args.aurora_countdown,
            args.time,
            args.time_scale,
            args.water_quality,
            args.color_grading,
            args.fog_unit,
            args.gpu_timings,
            args.debug_colours,
            args.no_objects,
            args.no_local_lights,
            args.slot_seed.is_none(),
            args.no_placeholders,
            args.no_grass,
            args.no_animation,
            args.no_lod,
            args.scenes.is_none(),
            args.no_water_fog,
            args.no_water_surface,
            args.game_ui,
        );
        let _ = writeln!(
            self.config,
            "world_settings={world:?}\n\
            capture_time_policy=requested at measurement end; capture may complete later\n\
            object_lods_fields=levels,culled,groups"
        );
    }

    pub fn save(&self, samples: &[f32], report: &str) -> Result<(), String> {
        let mut csv = String::from("frame,real_frame_ms\n");
        for (i, ms) in samples.iter().enumerate() {
            let _ = writeln!(csv, "{},{ms:.6}", i + 1);
        }
        for (name, contents) in [
            ("frames.csv", csv),
            ("report.txt", format!("{}{report}", self.config)),
        ] {
            let path = self.directory.join(name);
            std::fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(())
    }
}

#[derive(SystemParam)]
pub struct Snapshot<'w, 's> {
    setup: Res<'w, Setup>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    fog: Query<'w, 's, &'static WaterFog, With<Camera3d>>,
    lights: SpawnedLights<'w, 's>,
    water: Option<Res<'w, WaterWorld>>,
    sky: Option<Res<'w, SkyState>>,
    clock: Res<'w, GameClock>,
    surface: Option<Res<'w, SurfaceClock>>,
    lods: Option<Res<'w, LodStats>>,
    lod_settings: Option<Res<'w, LodSettings>>,
    virtual_time: Res<'w, Time>,
}

impl Snapshot<'_, '_> {
    pub fn describe(
        &self,
        camera: &Transform,
        terrain: StreamStats,
        objects: Option<&ObjectStreamer>,
    ) -> String {
        let p = camera.translation;
        let f = camera.forward();
        let position = [p.x, p.y, -p.z];
        let mut out = format!(
            "camera_unity={position:?}\nforward_unity={:?}\nfov_vertical_deg={}\n\
             virtual_elapsed_seconds={:.9}\ngame_seconds={:.9}\nwater_surface_seconds={:?}\n\
             terrain={terrain:?}\nlod_settings={:?}\nobject_lods={:?}\n\
             point_lights={}\nspot_lights={}\nobject_directional_lights={}\n\
             camera_biome={:?}\nsky={:?}\n",
            [f.x, f.y, -f.z],
            self.setup.fov,
            self.virtual_time.elapsed_secs_f64(),
            self.clock.seconds,
            self.surface.as_ref().map(|s| s.0),
            self.lod_settings.as_deref(),
            self.lods.as_ref().map(|s| s.snapshot()),
            self.lights.point.iter().count(),
            self.lights.spot.iter().count(),
            self.lights.directional.iter().count(),
            self.water.as_ref().and_then(|w| w.biome_at(position)),
            self.sky.as_deref(),
        );
        if let Ok(w) = self.windows.single() {
            let _ = writeln!(
                out,
                "physical_resolution={}x{}\nscale_factor={}\npresent_mode={:?}",
                w.physical_width(),
                w.physical_height(),
                w.scale_factor(),
                w.present_mode
            );
        }
        if let Ok(fog) = self.fog.single() {
            let _ = writeln!(out, "water_fog={fog:?}");
        }
        if let Some(objects) = objects {
            let _ = writeln!(out, "objects={:?}", objects.stats());
        }
        out
    }
}

pub fn gpu_report(diagnostics: &DiagnosticsStore) -> String {
    let mut values: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.path().as_str().ends_with("elapsed_gpu"))
        .filter_map(|d| Some((d.path().as_str(), d.average()?)))
        .collect();
    values.sort_by(|a, b| a.0.cmp(b.0));
    let mut report = format!("gpu_diagnostic_count={}\n", values.len());
    for (path, ms) in values {
        let _ = writeln!(report, "gpu_recent_ms[{path}]={ms:.6}");
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_cannot_escape_output_directory() {
        for s in [
            "",
            "..",
            "../x",
            "a/b",
            "a\\b",
            "C:x",
            "a.txt",
            "a\nother=1",
            " name",
        ] {
            assert!(!valid_label(s), "{s:?}");
        }
        assert!(!valid_label(&"x".repeat(97)));
        assert!(valid_label("shallows-1_revision_dirty"));
    }

    #[test]
    fn integer_counts_reject_float_casts_and_zero() {
        for s in ["0", "-1", "1.5", "NaN", "inf", "18446744073709551616"] {
            assert!(positive_integer(Some(&s.to_owned()), "--benchmark").is_err());
        }
        assert_eq!(
            positive_integer(Some(&"300".to_owned()), "--benchmark"),
            Ok(300)
        );
        assert!(positive_integer(None, "--benchmark").is_err());
    }

    #[test]
    fn warmup_restarts_when_streaming_resumes() {
        let mut warmup = Warmup::new(3);
        assert!(!warmup.ready(true));
        assert!(!warmup.ready(true));
        assert!(!warmup.ready(false));
        assert!(!warmup.ready(true));
        assert!(!warmup.ready(true));
        assert!(warmup.ready(true));
        assert!(!Warmup::new(0).ready(false));
        assert!(Warmup::new(0).ready(true));
    }

    #[test]
    fn frame_statistics_preserve_stalls_and_use_nearest_rank() {
        let samples: Vec<f32> = (1..=100).rev().map(|n| n as f32).collect();
        assert_eq!(
            frame_summary(&samples),
            Some(FrameSummary {
                mean: 50.5,
                median: 50.5,
                p95: 95.0,
                p99: 99.0,
                worst: 100.0,
            })
        );
        assert_eq!(frame_summary(&[1000.0]).unwrap().worst, 1000.0);
        assert_eq!(frame_summary(&[1.0, 10.0, 2.0]).unwrap().median, 2.0);
        assert!(frame_summary(&[]).is_none());
        assert!(frame_summary(&[f32::NAN]).is_none());
    }
}
