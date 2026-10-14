//! Legacy animation clips as the HUD plays them (M9u6b2,
//! `docs/formats/ui.md` § The bars' animations): Unity's `AnimationCurve`
//! and a clip's editor curves sampled at a time, written into a
//! [`UiState`]. The bars' clips animate a node's scale and a
//! `CanvasGroup`'s alpha, the only two kinds handled; `sn-assets` refuses
//! a clip with any other.

use crate::state::UiState;

/// One key of an `AnimationCurve`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub time: f32,
    pub value: f32,
    pub in_slope: f32,
    pub out_slope: f32,
}

/// Unity's `AnimationCurve`, clamped before its first key and after its
/// last (the wrap mode of every curve the HUD uses), unweighted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    pub keys: Vec<Key>,
}

impl Curve {
    /// `Evaluate(t)`: Hermite between keys; a segment with an infinite
    /// slope at either end is a step holding its first key's value.
    pub fn evaluate(&self, t: f32) -> f32 {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else {
            return 0.0;
        };
        if t <= first.time {
            return first.value;
        }
        if t >= last.time {
            return last.value;
        }
        for pair in self.keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t > b.time {
                continue;
            }
            let dt = b.time - a.time;
            if dt <= 0.0 {
                return b.value;
            }
            if !a.out_slope.is_finite() || !b.in_slope.is_finite() {
                return a.value;
            }
            let s = (t - a.time) / dt;
            let (s2, s3) = (s * s, s * s * s);
            let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
            let h10 = s3 - 2.0 * s2 + s;
            let h01 = -2.0 * s3 + 3.0 * s2;
            let h11 = s3 - s2;
            return h00 * a.value + h10 * a.out_slope * dt + h01 * b.value + h11 * b.in_slope * dt;
        }
        last.value
    }
}

/// What a track animates.
#[derive(Clone, Debug, PartialEq)]
pub enum Channel {
    /// `Transform.localScale` (x, y, z).
    Scale([Curve; 3]),
    /// `CanvasGroup.m_Alpha`.
    GroupAlpha(Curve),
}

/// One animated property of one node.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub node: usize,
    pub channel: Channel,
}

/// A legacy clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Seconds (`stopTime − startTime`).
    pub length: f32,
    pub tracks: Vec<Track>,
}

impl Clip {
    /// The state's time for a `normalizedTime` under `WrapMode.Loop`
    /// (*hypothesis*: the time wraps at the clip's end, so 1 samples the
    /// start; the scripts clamp to 0…1 and reach 1 only at the end of a
    /// pulse, where every alpha the HUD uses is 0 at both ends).
    pub fn loop_time(&self, normalized: f32) -> f32 {
        let t = normalized * self.length;
        if self.length > 0.0 {
            t.rem_euclid(self.length)
        } else {
            0.0
        }
    }

    /// The clip at `time` (seconds) written into `out` (a single state of
    /// weight 1: the sampled values replace the node's).
    pub fn sample(&self, time: f32, out: &mut UiState) {
        for t in &self.tracks {
            match &t.channel {
                Channel::Scale(c) => {
                    let s = [
                        c[0].evaluate(time),
                        c[1].evaluate(time),
                        c[2].evaluate(time),
                    ];
                    out.transform.entry(t.node).or_default().scale = Some(s);
                }
                Channel::GroupAlpha(c) => {
                    out.group_alpha.insert(t.node, c.evaluate(time));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(time: f32, value: f32, in_slope: f32, out_slope: f32) -> Key {
        Key {
            time,
            value,
            in_slope,
            out_slope,
        }
    }

    #[test]
    fn a_curve_clamps_steps_and_interpolates() {
        let c = Curve {
            keys: vec![
                key(0.0, -1.0, f32::INFINITY, f32::INFINITY),
                key(0.5, 2.0, 0.0, 0.0),
                key(1.0, 4.0, 0.0, 0.0),
            ],
        };
        assert_eq!(c.evaluate(-1.0), -1.0);
        // A step: the first key's value until the second.
        assert_eq!(c.evaluate(0.25), -1.0);
        // Flat tangents: the smoothstep midpoint.
        assert!((c.evaluate(0.75) - 3.0).abs() < 1e-6);
        assert_eq!(c.evaluate(2.0), 4.0);
        // Slopes: from 0 with slope 2 to 1 with slope 2 is the line 2t.
        let line = Curve {
            keys: vec![key(0.0, 0.0, 2.0, 2.0), key(1.0, 2.0, 2.0, 2.0)],
        };
        assert!((line.evaluate(0.3) - 0.6).abs() < 1e-6);
        assert_eq!(Curve::default().evaluate(0.5), 0.0);
    }

    #[test]
    fn a_clip_writes_scale_and_group_alpha_and_loops() {
        let flat = |v: f32| Curve {
            keys: vec![key(0.0, v, 0.0, 0.0)],
        };
        let clip = Clip {
            name: "c".into(),
            length: 2.0,
            tracks: vec![
                Track {
                    node: 3,
                    channel: Channel::Scale([flat(1.5), flat(2.0), flat(1.0)]),
                },
                Track {
                    node: 4,
                    channel: Channel::GroupAlpha(Curve {
                        keys: vec![key(0.0, 0.0, 0.5, 0.5), key(2.0, 1.0, 0.5, 0.5)],
                    }),
                },
            ],
        };
        let mut s = UiState::default();
        clip.sample(clip.loop_time(0.25), &mut s);
        assert_eq!(s.transform[&3].scale, Some([1.5, 2.0, 1.0]));
        assert!((s.group_alpha[&4] - 0.25).abs() < 1e-6);
        assert_eq!(clip.loop_time(1.0), 0.0);
    }
}
