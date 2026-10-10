//! A `Mesh`'s blend shapes (`m_Shapes`, Unity 2019.4 `BlendShapeData`) and
//! how a renderer's weights move the vertices (M7f4d). See
//! `docs/formats/unity.md` § Blend shapes.
//!
//! A *channel* is what a renderer weights (by index in
//! `m_BlendShapeWeights`, or in animations by the CRC-32 of its name,
//! [`BlendShapeChannel::name_hash`]). It
//! has one or more *frames*; each frame is a sparse list of vertex
//! offsets reached at its full weight.

use crate::mesh::MeshGeometry;
use crate::reader::Reader;
use crate::{Error, ErrorKind, Result};

/// One vertex's offsets in one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlendShapeVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub tangent: [f32; 3],
    /// Index into the mesh's vertices.
    pub index: u32,
}

/// One frame (Unity's `MeshBlendShape`): `vertices[first_vertex ..
/// first_vertex + vertex_count]` of [`BlendShapes::vertices`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlendShapeFrame {
    pub first_vertex: u32,
    pub vertex_count: u32,
    pub has_normals: bool,
    pub has_tangents: bool,
}

/// A channel: frames `frame_index .. frame_index + frame_count`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlendShapeChannel {
    pub name: String,
    /// CRC-32 of `name`.
    pub name_hash: u32,
    pub frame_index: u32,
    pub frame_count: u32,
}

/// `BlendShapeData`. Checked when read: every frame's vertices and every
/// channel's frames are in range, one full weight per frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlendShapes {
    pub vertices: Vec<BlendShapeVertex>,
    pub frames: Vec<BlendShapeFrame>,
    pub channels: Vec<BlendShapeChannel>,
    /// Per frame: the channel weight at which it is fully on (Unity's
    /// weights run 0–100).
    pub full_weights: Vec<f32>,
}

/// `PlayerSettings` (class 129, in `globalgamemanagers`).
pub const PLAYER_SETTINGS: i32 = 129;

/// `PlayerSettings.legacyClampBlendShapeWeights`: Unity clamps every blend
/// shape weight to 0–100 when set. In Unity 2019.4 it is the class's last
/// field, after three other bools (`cloudEnabled`,
/// `enableNativePlatformBackendsForNewInputSystem`,
/// `disableOldInputManagerSupport`), so it is read from the object's end
/// rather than through the class's ~140 fields. Each of the four bytes
/// must be 0 or 1. Confirmed against UnityPy's full read (M7f4d).
pub fn clamps_blend_shape_weights(player_settings: &[u8]) -> Result<bool> {
    let len = player_settings.len();
    match player_settings.get(len.saturating_sub(4)..) {
        Some(tail) if len >= 4 && tail.iter().all(|&b| b <= 1) => Ok(tail[3] == 1),
        _ => Err(Error {
            offset: len.saturating_sub(4),
            kind: ErrorKind::Invalid(
                "PlayerSettings does not end in four bools: not Unity 2019.4's layout".into(),
            ),
        }),
    }
}

/// How much of each of a channel's frames a channel weight takes, as
/// `(frame, factor)` (at most two; `frame` counts from the channel's
/// first), given the frames' full weights `full`. Unity's rule for
/// frames at full weights `w₀ < w₁ < …`: below `w₀` the first frame scaled by
/// `weight / w₀`; between two frames linear between them; past the
/// last, the last two extrapolated (with one frame: scaled by
/// `weight / w₀` everywhere). **Hypothesis** for the extrapolation
/// beyond `0..w_last`; Subnautica never reaches it, as its project
/// clamps weights to 0–100 ([`clamps_blend_shape_weights`]; the caller
/// clamps) and every frame's full weight is 100.
pub fn channel_frame_factors(full: &[f32], weight: f32) -> [(usize, f32); 2] {
    let Some(&w0) = full.first() else {
        return [(0, 0.0), (0, 0.0)];
    };
    let count = full.len();
    if count == 1 || weight <= w0 {
        let f = if w0 != 0.0 { weight / w0 } else { 0.0 };
        return [(0, f), (0, 0.0)];
    }
    // The segment holding the weight, else the last one.
    let k = (1..count).find(|&k| weight <= full[k]).unwrap_or(count - 1);
    let (a, b) = (full[k - 1], full[k]);
    let t = if b != a { (weight - a) / (b - a) } else { 1.0 };
    [(k - 1, 1.0 - t), (k, t)]
}

/// `to[..3] += f × d`.
fn add_scaled(to: &mut [f32], d: &[f32; 3], f: f32) {
    for (x, d) in to.iter_mut().zip(d) {
        *x += f * d;
    }
}

fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

impl BlendShapes {
    pub(crate) fn read(r: &mut Reader) -> Result<BlendShapes> {
        let n = r.count(40)?;
        let mut vertices = Vec::with_capacity(n);
        for _ in 0..n {
            vertices.push(BlendShapeVertex {
                position: vec3(r)?,
                normal: vec3(r)?,
                tangent: vec3(r)?,
                index: r.u32()?,
            });
        }
        r.align(4)?;
        let n = r.count(12)?;
        let mut frames = Vec::with_capacity(n);
        for _ in 0..n {
            frames.push(BlendShapeFrame {
                first_vertex: r.u32()?,
                vertex_count: r.u32()?,
                has_normals: r.u8()? != 0,
                has_tangents: r.u8()? != 0,
            });
            r.align(4)?;
        }
        r.align(4)?;
        let n = r.count(16)?;
        let mut channels = Vec::with_capacity(n);
        for _ in 0..n {
            let at = r.pos();
            let name = r.aligned_string()?;
            let name_hash = r.u32()?;
            let (frame_index, frame_count) = (r.i32()?, r.i32()?);
            let (Ok(frame_index), Ok(frame_count)) =
                (u32::try_from(frame_index), u32::try_from(frame_count))
            else {
                return Err(Error {
                    offset: at,
                    kind: ErrorKind::Invalid(format!(
                        "blend shape channel {name:?}: frames {frame_index} + {frame_count}"
                    )),
                });
            };
            channels.push(BlendShapeChannel {
                name,
                name_hash,
                frame_index,
                frame_count,
            });
        }
        r.align(4)?;
        let n = r.count(4)?;
        let mut full_weights = Vec::with_capacity(n);
        for _ in 0..n {
            full_weights.push(r.f32()?);
        }
        r.align(4)?;
        Ok(BlendShapes {
            vertices,
            frames,
            channels,
            full_weights,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Checks the ranges against the mesh's `vertex_count`; the problem
    /// found first, if any.
    pub fn check(&self, vertex_count: u32) -> std::result::Result<(), String> {
        if self.full_weights.len() != self.frames.len() {
            return Err(format!(
                "{} frames, {} full weights",
                self.frames.len(),
                self.full_weights.len()
            ));
        }
        for (i, f) in self.frames.iter().enumerate() {
            let end = u64::from(f.first_vertex) + u64::from(f.vertex_count);
            if end > self.vertices.len() as u64 {
                return Err(format!(
                    "frame {i}: vertices {}..{end} of {}",
                    f.first_vertex,
                    self.vertices.len()
                ));
            }
        }
        if let Some(v) = self.vertices.iter().find(|v| v.index >= vertex_count) {
            return Err(format!("vertex {} of {vertex_count}", v.index));
        }
        for c in &self.channels {
            let end = u64::from(c.frame_index) + u64::from(c.frame_count);
            if c.frame_count == 0 || end > self.frames.len() as u64 {
                return Err(format!(
                    "channel {:?}: frames {}..{end} of {}",
                    c.name,
                    c.frame_index,
                    self.frames.len()
                ));
            }
        }
        Ok(())
    }

    /// The frame offsets in `frame`'s vertices.
    pub fn frame_vertices(&self, frame: usize) -> &[BlendShapeVertex] {
        let Some(f) = self.frames.get(frame) else {
            return &[];
        };
        let start = f.first_vertex as usize;
        self.vertices
            .get(start..start + f.vertex_count as usize)
            .unwrap_or(&[])
    }

    /// [`channel_frame_factors`] for one of the channels: frame indices into
    /// [`BlendShapes::frames`].
    pub fn frame_factors(&self, channel: usize, weight: f32) -> [(usize, f32); 2] {
        let Some(c) = self.channels.get(channel) else {
            return [(0, 0.0), (0, 0.0)];
        };
        let first = c.frame_index as usize;
        let count = c.frame_count as usize;
        let Some(full) = self.full_weights.get(first..first + count) else {
            return [(0, 0.0), (0, 0.0)];
        };
        channel_frame_factors(full, weight).map(|(k, f)| (first + k, f))
    }

    /// Moves `geometry`'s vertices as the renderer's `weights` (one per
    /// channel; missing ones 0) put them: positions, and normals and
    /// tangents where the frame has them (normals renormalised, as
    /// Unity's skinning does). `geometry` is the mesh's own, decoded.
    pub fn apply(&self, geometry: &mut MeshGeometry, weights: &[f32]) {
        let n = geometry.positions.len();
        let normals = geometry.normals.len() == n;
        let tangents = geometry.tangents.len() == n;
        let mut moved = false;
        for (c, &w) in weights.iter().enumerate().take(self.channels.len()) {
            if w == 0.0 {
                continue;
            }
            for (frame, f) in self.frame_factors(c, w) {
                if f == 0.0 {
                    continue;
                }
                let info = self.frames[frame];
                for v in self.frame_vertices(frame) {
                    let i = v.index as usize;
                    let Some(p) = geometry.positions.get_mut(i) else {
                        continue;
                    };
                    add_scaled(p, &v.position, f);
                    if normals && info.has_normals {
                        add_scaled(&mut geometry.normals[i], &v.normal, f);
                    }
                    if tangents && info.has_tangents {
                        add_scaled(&mut geometry.tangents[i], &v.tangent, f);
                    }
                    moved = true;
                }
            }
        }
        if moved && normals {
            for v in &mut geometry.normals {
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if len > 0.0 {
                    *v = v.map(|c| c / len);
                }
            }
        }
    }

    /// `frame`'s offsets for every one of `vertex_count` vertices (zero for
    /// those it doesn't move): position, normal, tangent; normal and
    /// tangent zero when the frame has none.
    pub fn dense_frame(&self, frame: usize, vertex_count: usize) -> Vec<[[f32; 3]; 3]> {
        let mut out = vec![[[0.0; 3]; 3]; vertex_count];
        let Some(info) = self.frames.get(frame) else {
            return out;
        };
        for v in self.frame_vertices(frame) {
            if let Some(o) = out.get_mut(v.index as usize) {
                o[0] = v.position;
                if info.has_normals {
                    o[1] = v.normal;
                }
                if info.has_tangents {
                    o[2] = v.tangent;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(bytes);
    }

    fn string(out: &mut Vec<u8>, s: &str) {
        le(out, &(s.len() as u32).to_le_bytes());
        le(out, s.as_bytes());
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }

    /// Bytes of a `BlendShapeData`: one channel "open" with two frames at
    /// 50 and 100; frame 0 moves vertex 1 up by 1, frame 1 moves vertex 1
    /// up by 3 and vertex 2 sideways (with a normal offset).
    fn synthetic() -> Vec<u8> {
        let mut b = Vec::new();
        let verts: [([f32; 3], [f32; 3], u32); 3] = [
            ([0.0, 1.0, 0.0], [0.0; 3], 1),
            ([0.0, 3.0, 0.0], [0.0; 3], 1),
            ([2.0, 0.0, 0.0], [1.0, -1.0, 0.0], 2),
        ];
        le(&mut b, &3u32.to_le_bytes());
        for (p, n, i) in verts {
            for v in p.iter().chain(&n).chain(&[0.0f32; 3]) {
                le(&mut b, &v.to_le_bytes());
            }
            le(&mut b, &i.to_le_bytes());
        }
        le(&mut b, &2u32.to_le_bytes());
        for (first, count, normals) in [(0u32, 1u32, 0u8), (1, 2, 1)] {
            le(&mut b, &first.to_le_bytes());
            le(&mut b, &count.to_le_bytes());
            le(&mut b, &[normals, 0, 0, 0]);
        }
        le(&mut b, &1u32.to_le_bytes());
        string(&mut b, "open");
        le(&mut b, &0x1234u32.to_le_bytes());
        le(&mut b, &0i32.to_le_bytes());
        le(&mut b, &2i32.to_le_bytes());
        le(&mut b, &2u32.to_le_bytes());
        le(&mut b, &50f32.to_le_bytes());
        le(&mut b, &100f32.to_le_bytes());
        b
    }

    #[test]
    fn clamp_flag_is_the_last_bool() {
        let mut settings = vec![7u8; 20];
        settings.extend([0, 1, 0, 1]);
        assert_eq!(clamps_blend_shape_weights(&settings), Ok(true));
        let n = settings.len();
        settings[n - 1] = 0;
        assert_eq!(clamps_blend_shape_weights(&settings), Ok(false));
        settings[n - 2] = 9;
        assert!(clamps_blend_shape_weights(&settings).is_err());
        assert!(clamps_blend_shape_weights(&[1, 0]).is_err());
    }

    #[test]
    fn reads_to_the_last_byte() {
        let bytes = synthetic();
        let mut r = Reader::new(&bytes, false);
        let s = BlendShapes::read(&mut r).unwrap();
        assert_eq!(r.pos(), bytes.len());
        assert_eq!(s.vertices.len(), 3);
        assert_eq!(s.vertices[2].normal, [1.0, -1.0, 0.0]);
        assert_eq!(s.frames[1].first_vertex, 1);
        assert!(s.frames[1].has_normals && !s.frames[0].has_normals);
        assert_eq!(s.channels[0].name, "open");
        assert_eq!(s.channels[0].frame_count, 2);
        assert_eq!(s.full_weights, vec![50.0, 100.0]);
        assert_eq!(s.check(3), Ok(()));
        // Vertex 2 does not exist in a 2-vertex mesh.
        assert!(s.check(2).is_err());
    }

    #[test]
    fn truncated_bytes_are_an_error() {
        let bytes = synthetic();
        for cut in [3, 50, bytes.len() - 1] {
            let mut r = Reader::new(&bytes[..cut], false);
            assert!(BlendShapes::read(&mut r).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn bad_ranges_are_found() {
        let bytes = synthetic();
        let s = BlendShapes::read(&mut Reader::new(&bytes, false)).unwrap();
        let mut bad = s.clone();
        bad.frames[1].vertex_count = 3;
        assert!(bad.check(3).is_err());
        let mut bad = s.clone();
        bad.channels[0].frame_count = 3;
        assert!(bad.check(3).is_err());
        let mut bad = s;
        bad.full_weights.pop();
        assert!(bad.check(3).is_err());
    }

    #[test]
    fn frame_factors_follow_the_frames() {
        let s = BlendShapes::read(&mut Reader::new(&synthetic(), false)).unwrap();
        assert_eq!(s.frame_factors(0, 25.0), [(0, 0.5), (0, 0.0)]);
        assert_eq!(s.frame_factors(0, 50.0), [(0, 1.0), (0, 0.0)]);
        assert_eq!(s.frame_factors(0, 75.0), [(0, 0.5), (1, 0.5)]);
        assert_eq!(s.frame_factors(0, 100.0), [(0, 0.0), (1, 1.0)]);
        // Past the last frame: the last segment extrapolated.
        assert_eq!(s.frame_factors(0, 150.0), [(0, -1.0), (1, 2.0)]);
        // No such channel: nothing.
        assert_eq!(s.frame_factors(1, 50.0)[0].1, 0.0);
    }

    #[test]
    fn apply_moves_the_vertices() {
        let s = BlendShapes::read(&mut Reader::new(&synthetic(), false)).unwrap();
        let base = MeshGeometry {
            positions: vec![[0.0; 3]; 3],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            ..MeshGeometry::default()
        };
        let mut g = base.clone();
        s.apply(&mut g, &[75.0]);
        // Half of frame 0 (+0.5) and half of frame 1 (+1.5).
        assert_eq!(g.positions[1], [0.0, 2.0, 0.0]);
        assert_eq!(g.positions[2], [1.0, 0.0, 0.0]);
        // Normal (0, 1, 0) + 0.5 · (1, −1, 0) = (0.5, 0.5, 0), renormalised.
        let h = 0.5f32.sqrt();
        assert!((g.normals[2][0] - h).abs() < 1e-6 && (g.normals[2][1] - h).abs() < 1e-6);
        assert_eq!(g.positions[0], [0.0; 3]);
        // Zero weights leave the mesh as it is.
        let mut still = base.clone();
        s.apply(&mut still, &[0.0]);
        assert_eq!(still, base);
    }

    #[test]
    fn dense_frames_hold_every_vertex() {
        let s = BlendShapes::read(&mut Reader::new(&synthetic(), false)).unwrap();
        let d = s.dense_frame(1, 3);
        assert_eq!(d[0], [[0.0; 3]; 3]);
        assert_eq!(d[1][0], [0.0, 3.0, 0.0]);
        assert_eq!(d[2][1], [1.0, -1.0, 0.0]);
        // Frame 0 has no normals.
        assert_eq!(s.dense_frame(0, 3)[1][1], [0.0; 3]);
    }
}
