//! A clip's curves at a time (`docs/formats/animation.md`): streamed keys
//! hold a cubic up to the curve's next key, dense frames are joined by
//! straight lines, constants are constant.

use sn_unity::{AnimationClip, StreamedKey};

/// The clip's length in seconds (`stop − start`; at least 0).
pub fn length(clip: &AnimationClip) -> f32 {
    (clip.stop_time - clip.start_time).max(0.0)
}

fn streamed(keys: &[StreamedKey], t: f32) -> f32 {
    // The last key at or before `t` (keys are in time order).
    let i = keys.partition_point(|k| k.time <= t);
    let Some(k) = i.checked_sub(1).and_then(|i| keys.get(i)) else {
        return keys.first().map_or(0.0, |k| k.coeff[3]);
    };
    let [a, b, c, d] = k.coeff;
    if k.time < -1e30 {
        // The starting frame at −FLT_MAX: its value holds until the first key.
        return d;
    }
    let dt = t - k.time;
    ((a * dt + b) * dt + c) * dt + d
}

/// Every curve of `clip` at clip time `t` (seconds, from the clip's own
/// zero; not wrapped here) into `out` (resized to the curve count).
pub fn sample(clip: &AnimationClip, t: f32, out: &mut Vec<f32>) {
    out.clear();
    out.reserve(clip.curve_count());
    for keys in &clip.streamed {
        out.push(streamed(keys, t));
    }
    let dense = &clip.dense;
    let n = dense.curve_count as usize;
    if n > 0 {
        let frames = dense.frame_count.max(0) as usize;
        let f = ((t - dense.begin_time) * dense.sample_rate).max(0.0);
        let last = frames.saturating_sub(1);
        let i0 = (f.floor() as usize).min(last);
        let i1 = (i0 + 1).min(last);
        let w = (f - i0 as f32).clamp(0.0, 1.0);
        for c in 0..n {
            let a = dense.samples.get(i0 * n + c).copied().unwrap_or(0.0);
            let b = dense.samples.get(i1 * n + c).copied().unwrap_or(a);
            out.push(a + (b - a) * w);
        }
    }
    out.extend_from_slice(&clip.constant);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sn_unity::{DenseClip, GenericBinding};

    pub fn clip(name: &str, length: f32, looping: bool) -> AnimationClip {
        AnimationClip {
            name: name.into(),
            legacy: false,
            compressed: false,
            editor_curves: [0; 7],
            sample_rate: 30.0,
            wrap_mode: 0,
            bounds: ([0.0; 3], [0.0; 3]),
            streamed: vec![],
            dense: DenseClip::default(),
            constant: vec![],
            start_time: 0.0,
            stop_time: length,
            orientation_offset_y: 0.0,
            level: 0.0,
            cycle_offset: 0.0,
            average_angular_speed: 0.0,
            average_speed: [0.0; 3],
            index_array: vec![],
            value_array_delta: vec![],
            value_array_reference_pose: vec![],
            mirror: false,
            loop_time: looping,
            loop_blend: false,
            loop_blend_orientation: false,
            loop_blend_position_y: false,
            loop_blend_position_xz: false,
            start_at_origin: false,
            keep_original_orientation: false,
            keep_original_position_y: false,
            keep_original_position_xz: false,
            height_from_feet: false,
            bindings: Vec::<GenericBinding>::new(),
            pptr_curve_mapping: vec![],
            has_generic_root_transform: false,
            has_motion_float_curves: false,
            events: vec![],
        }
    }

    /// A streamed curve linear from `(t0, v0)` to `(t1, v1)`, constant
    /// outside.
    pub fn ramp(t0: f32, v0: f32, t1: f32, v1: f32) -> Vec<StreamedKey> {
        vec![
            StreamedKey {
                time: f32::MIN,
                coeff: [0.0, 0.0, 0.0, v0],
            },
            StreamedKey {
                time: t0,
                coeff: [0.0, 0.0, (v1 - v0) / (t1 - t0), v0],
            },
            StreamedKey {
                time: t1,
                coeff: [0.0, 0.0, 0.0, v1],
            },
            StreamedKey {
                time: f32::MAX,
                coeff: [0.0, 0.0, 0.0, v1],
            },
        ]
    }

    #[test]
    fn streamed_cubic_dense_lines_and_constants() {
        let mut c = clip("c", 1.0, false);
        // value = t³ − t + 2 from t = 0.
        c.streamed = vec![vec![
            StreamedKey {
                time: f32::MIN,
                coeff: [0.0, 0.0, 0.0, 2.0],
            },
            StreamedKey {
                time: 0.0,
                coeff: [1.0, 0.0, -1.0, 2.0],
            },
        ]];
        c.dense = DenseClip {
            frame_count: 3,
            curve_count: 2,
            sample_rate: 2.0,
            begin_time: 0.0,
            samples: vec![0.0, 10.0, 1.0, 20.0, 3.0, 40.0],
        };
        c.constant = vec![7.0];
        let mut out = Vec::new();
        sample(&c, 0.5, &mut out);
        assert_eq!(out.len(), 4);
        assert!((out[0] - (0.125 - 0.5 + 2.0)).abs() < 1e-6);
        assert_eq!(&out[1..], &[1.0, 20.0, 7.0]);
        sample(&c, 0.75, &mut out);
        assert!((out[1] - 2.0).abs() < 1e-6);
        assert!((out[2] - 30.0).abs() < 1e-5);
        // Before the first key: the starting value; past the last frame:
        // the last frame.
        sample(&c, -1.0, &mut out);
        assert_eq!(out[0], 2.0);
        assert_eq!(out[1], 0.0);
        sample(&c, 5.0, &mut out);
        assert_eq!(out[2], 40.0);
    }

    #[test]
    fn ramps_hold_their_ends() {
        let mut c = clip("r", 2.0, false);
        c.streamed = vec![ramp(0.5, 1.0, 1.5, 3.0)];
        let mut out = Vec::new();
        for (t, v) in [(0.0, 1.0), (0.5, 1.0), (1.0, 2.0), (1.5, 3.0), (9.0, 3.0)] {
            sample(&c, t, &mut out);
            assert!((out[0] - v).abs() < 1e-6, "{t}: {}", out[0]);
        }
    }
}
