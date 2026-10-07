//! `Mesh` (class 43), Unity 2019.4 layout, and decoding of its geometry:
//! interleaved vertex streams, or Unity's compressed (bit-packed) form.
//! See `docs/formats/unity.md`.

use crate::objects::PPtr;
use crate::reader::Reader;
use crate::texture::StreamingInfo;
use crate::{Error, ErrorKind, Result};

/// A range of the index buffer drawn with one material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubMesh {
    /// Byte offset into the index buffer.
    pub first_byte: u32,
    pub index_count: u32,
    /// 0 = triangles (the only one seen in Subnautica).
    pub topology: i32,
    pub base_vertex: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
}

/// Where one vertex attribute lives in the vertex data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Channel {
    pub stream: u8,
    pub offset: u8,
    /// Unity `VertexFormat`: 0 float32, 1 float16, 2 unorm8, 3 snorm8,
    /// 4 unorm16, 5 snorm16, 6 uint8, 7 sint8, 8 uint16, 9 sint16,
    /// 10 uint32, 11 sint32.
    pub format: u8,
    /// Low 4 bits: component count (0 = channel absent).
    pub dimension: u8,
}

impl Channel {
    pub fn components(&self) -> usize {
        usize::from(self.dimension & 0xf)
    }

    fn format_size(&self) -> Option<usize> {
        Some(match self.format {
            0 | 10 | 11 => 4,
            1 | 4 | 5 | 8 | 9 => 2,
            2 | 3 | 6 | 7 => 1,
            _ => return None,
        })
    }
}

/// Channel indices (Unity 2019): position, normal, tangent, colour, then
/// texture coordinates 0–7, blend weights and blend indices.
pub mod channel {
    pub const POSITION: usize = 0;
    pub const NORMAL: usize = 1;
    pub const TANGENT: usize = 2;
    pub const COLOR: usize = 3;
    pub const UV0: usize = 4;
    pub const UV1: usize = 5;
}

/// A `PackedBitVector`: `count` values of `bit_size` bits each, packed
/// least significant bit first. Float vectors map 0..2^bits−1 linearly onto
/// `start`..`start + range`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackedBits {
    pub count: u32,
    pub range: f32,
    pub start: f32,
    pub data: Vec<u8>,
    pub bit_size: u8,
}

impl PackedBits {
    fn read(r: &mut Reader, with_range: bool) -> Result<PackedBits> {
        let count = r.u32()?;
        let (range, start) = if with_range {
            (r.f32()?, r.f32()?)
        } else {
            (0.0, 0.0)
        };
        let data = byte_vector(r)?;
        let bit_size = r.u8()?;
        r.align(4)?;
        Ok(PackedBits {
            count,
            range,
            start,
            data,
            bit_size,
        })
    }

    /// The values as integers; `None` if the data is too short.
    pub fn ints(&self) -> Option<Vec<u32>> {
        let bits = usize::from(self.bit_size);
        if bits > 32 {
            return None;
        }
        let count = self.count as usize;
        if count.checked_mul(bits)?.div_ceil(8) > self.data.len() {
            return None;
        }
        let mut out = Vec::with_capacity(count);
        let mut bit = 0usize;
        for _ in 0..count {
            let mut value = 0u64;
            let mut got = 0;
            while got < bits {
                let byte = u64::from(self.data[bit / 8]);
                let shift = bit % 8;
                let take = (bits - got).min(8 - shift);
                value |= ((byte >> shift) & ((1 << take) - 1)) << got;
                got += take;
                bit += take;
            }
            out.push(value as u32);
        }
        Some(out)
    }

    pub fn floats(&self) -> Option<Vec<f32>> {
        let max = ((1u64 << self.bit_size.min(32)) - 1).max(1) as f32;
        let scale = self.range / max;
        Some(
            self.ints()?
                .into_iter()
                .map(|v| self.start + v as f32 * scale)
                .collect(),
        )
    }
}

/// Unity's compressed mesh data (used when `compression` is not 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompressedMesh {
    pub vertices: PackedBits,
    pub uv: PackedBits,
    pub normals: PackedBits,
    pub tangents: PackedBits,
    pub weights: PackedBits,
    pub normal_signs: PackedBits,
    pub tangent_signs: PackedBits,
    pub float_colors: PackedBits,
    pub bone_indices: PackedBits,
    pub triangles: PackedBits,
    /// 4 bits per texture-coordinate set: bit 2 = present, bits 0–1 =
    /// dimension − 1. 0 = old layout (UV0, then maybe UV1, 2D).
    pub uv_info: u32,
}

impl CompressedMesh {
    fn read(r: &mut Reader) -> Result<CompressedMesh> {
        Ok(CompressedMesh {
            vertices: PackedBits::read(r, true)?,
            uv: PackedBits::read(r, true)?,
            normals: PackedBits::read(r, true)?,
            tangents: PackedBits::read(r, true)?,
            weights: PackedBits::read(r, false)?,
            normal_signs: PackedBits::read(r, false)?,
            tangent_signs: PackedBits::read(r, false)?,
            float_colors: PackedBits::read(r, true)?,
            bone_indices: PackedBits::read(r, false)?,
            triangles: PackedBits::read(r, false)?,
            uv_info: r.u32()?,
        })
    }
}

/// A `Mesh` object: everything needed to decode its geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub sub_meshes: Vec<SubMesh>,
    /// 0 = none; otherwise the geometry is in `compressed`.
    pub compression: u8,
    /// 0 = 16-bit indices, 1 = 32-bit.
    pub index_format: i32,
    pub index_buffer: Vec<u8>,
    pub vertex_count: u32,
    pub channels: Vec<Channel>,
    /// Vertex data stored in the object (empty if streamed).
    pub vertex_data: Vec<u8>,
    pub compressed: CompressedMesh,
    /// Vertex data stored in a resource file instead.
    pub stream: Option<StreamingInfo>,
    /// Bounding box centre and half-size.
    pub aabb: ([f32; 3], [f32; 3]),
}

fn skip_vector(r: &mut Reader, item_bytes: usize) -> Result<()> {
    let n = r.count(item_bytes)?;
    r.bytes(n * item_bytes)?;
    r.align(4)
}

fn byte_vector(r: &mut Reader) -> Result<Vec<u8>> {
    let n = r.count(1)?;
    let v = r.bytes(n)?.to_vec();
    r.align(4)?;
    Ok(v)
}

fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

impl Mesh {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Mesh> {
        let mut r = Reader::new(data, big_endian);
        let name = r.aligned_string()?;
        let n = r.count(48)?;
        let mut sub_meshes = Vec::with_capacity(n);
        for _ in 0..n {
            let s = SubMesh {
                first_byte: r.u32()?,
                index_count: r.u32()?,
                topology: r.i32()?,
                base_vertex: r.u32()?,
                first_vertex: r.u32()?,
                vertex_count: r.u32()?,
            };
            vec3(&mut r)?;
            vec3(&mut r)?;
            sub_meshes.push(s);
        }
        r.align(4)?;
        // Blend shapes: vertices, shapes, channels, full weights.
        skip_vector(&mut r, 40)?;
        skip_vector(&mut r, 12)?;
        let channels = r.count(16)?;
        for _ in 0..channels {
            r.aligned_string()?;
            r.u32()?;
            r.i32()?;
            r.i32()?;
        }
        r.align(4)?;
        skip_vector(&mut r, 4)?;
        skip_vector(&mut r, 64)?; // bind poses
        skip_vector(&mut r, 4)?; // bone name hashes
        r.u32()?; // root bone name hash
        skip_vector(&mut r, 24)?; // bone AABBs
        skip_vector(&mut r, 4)?; // variable bone count weights
        let compression = r.u8()?;
        let _readable = r.u8()?;
        let _keep_vertices = r.u8()?;
        let _keep_indices = r.u8()?;
        r.align(4)?;
        let index_format = r.i32()?;
        let index_buffer = byte_vector(&mut r)?;
        let vertex_count = r.u32()?;
        let n = r.count(4)?;
        let mut channels = Vec::with_capacity(n);
        for _ in 0..n {
            channels.push(Channel {
                stream: r.u8()?,
                offset: r.u8()?,
                format: r.u8()?,
                dimension: r.u8()?,
            });
        }
        r.align(4)?;
        let vertex_data = byte_vector(&mut r)?;
        let compressed = CompressedMesh::read(&mut r)?;
        let aabb = (vec3(&mut r)?, vec3(&mut r)?);
        let _usage_flags = r.i32()?;
        byte_vector(&mut r)?; // baked convex collision mesh
        byte_vector(&mut r)?; // baked triangle collision mesh
        r.f32()?;
        r.f32()?; // mesh metrics
        r.align(4)?;
        let offset = r.u32()?;
        let size = r.u32()?;
        let path = r.aligned_string()?;
        let stream = (!path.is_empty()).then_some(StreamingInfo {
            offset: u64::from(offset),
            size,
            path,
        });
        Ok(Mesh {
            name,
            sub_meshes,
            compression,
            index_format,
            index_buffer,
            vertex_count,
            channels,
            vertex_data,
            compressed,
            stream,
            aabb,
        })
    }

    fn invalid(&self, what: String) -> Error {
        Error {
            offset: 0,
            kind: ErrorKind::Invalid(format!("mesh {:?}: {what}", self.name)),
        }
    }

    /// Start and stride of each vertex stream, and the total size (streams
    /// follow one another, each starting on a 16-byte boundary).
    fn stream_layout(&self) -> Option<(Vec<usize>, Vec<usize>, usize)> {
        let streams = self
            .channels
            .iter()
            .filter(|c| c.components() > 0)
            .map(|c| usize::from(c.stream) + 1)
            .max()
            .unwrap_or(0);
        let mut strides = vec![0usize; streams];
        for c in self.channels.iter().filter(|c| c.components() > 0) {
            strides[usize::from(c.stream)] += c.format_size()? * c.components();
        }
        let mut starts = Vec::with_capacity(streams);
        let mut offset = 0usize;
        for stride in &strides {
            starts.push(offset);
            offset = offset.checked_add(stride.checked_mul(self.vertex_count as usize)?)?;
            offset = offset.checked_add(15)? & !15;
        }
        Some((starts, strides, offset))
    }

    /// Bytes of vertex data the channels need (for checking stream sizes).
    pub fn vertex_data_size(&self) -> Option<usize> {
        self.stream_layout().map(|(_, _, size)| size)
    }

    /// Decodes the geometry. `vertex_bytes` is [`Mesh::vertex_data`] or,
    /// for streamed meshes, the bytes [`Mesh::stream`] points to (unused for
    /// compressed meshes).
    pub fn decode(&self, vertex_bytes: &[u8]) -> Result<MeshGeometry> {
        let mut geometry = if self.compression == 0 {
            self.decode_streams(vertex_bytes)?
        } else {
            self.decode_compressed()?
        };
        let indices: Vec<u32> = if self.compression == 0 {
            if self.index_format == 0 {
                self.index_buffer
                    .chunks_exact(2)
                    .map(|c| u32::from(u16::from_le_bytes([c[0], c[1]])))
                    .collect()
            } else {
                self.index_buffer
                    .chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect()
            }
        } else {
            self.compressed
                .triangles
                .ints()
                .ok_or_else(|| self.invalid("packed triangles too short".into()))?
        };
        geometry.sub_meshes = self.split_indices(&indices, geometry.positions.len())?;
        Ok(geometry)
    }

    /// Per sub-mesh triangle lists, with the base vertex added and every
    /// index checked.
    fn split_indices(&self, indices: &[u32], vertices: usize) -> Result<Vec<Vec<u32>>> {
        let index_size = if self.index_format == 0 { 2 } else { 4 };
        let mut out = Vec::with_capacity(self.sub_meshes.len());
        for (i, s) in self.sub_meshes.iter().enumerate() {
            if s.topology != 0 {
                return Err(self.invalid(format!("sub-mesh {i}: topology {}", s.topology)));
            }
            let first = s.first_byte as usize / index_size;
            let list = first
                .checked_add(s.index_count as usize)
                .and_then(|end| indices.get(first..end))
                .ok_or_else(|| self.invalid(format!("sub-mesh {i}: indices out of bounds")))?;
            if list.len() % 3 != 0 {
                return Err(self.invalid(format!("sub-mesh {i}: {} indices", list.len())));
            }
            let mut absolute = Vec::with_capacity(list.len());
            for &raw in list {
                let index = raw
                    .checked_add(s.base_vertex)
                    .filter(|&x| (x as usize) < vertices)
                    .ok_or_else(|| {
                        self.invalid(format!(
                            "sub-mesh {i}: index {raw} + {} ≥ {vertices} vertices",
                            s.base_vertex
                        ))
                    })?;
                absolute.push(index);
            }
            out.push(absolute);
        }
        Ok(out)
    }

    fn decode_streams(&self, vertex_bytes: &[u8]) -> Result<MeshGeometry> {
        let (starts, strides, _) = self
            .stream_layout()
            .ok_or_else(|| self.invalid("unknown vertex format or size overflow".into()))?;
        let n = self.vertex_count as usize;
        let read = |index: usize| -> Result<Vec<[f32; 4]>> {
            let Some(c) = self.channels.get(index).filter(|c| c.components() > 0) else {
                return Ok(Vec::new());
            };
            let size = c.format_size().unwrap_or(4);
            let stream = usize::from(c.stream);
            let (start, stride) = (starts[stream], strides[stream]);
            let mut out = Vec::with_capacity(n);
            for v in 0..n {
                let at = start + v * stride + usize::from(c.offset);
                let bytes = vertex_bytes
                    .get(at..at + size * c.components())
                    .ok_or_else(|| {
                        self.invalid(format!("channel {index} runs past the vertex data"))
                    })?;
                let mut value = [0.0f32; 4];
                for (k, chunk) in bytes.chunks_exact(size).enumerate().take(4) {
                    value[k] = component(c.format, chunk);
                }
                out.push(value);
            }
            Ok(out)
        };
        let positions: Vec<[f32; 3]> = read(channel::POSITION)?
            .into_iter()
            .map(|v| [v[0], v[1], v[2]])
            .collect();
        if positions.len() != n {
            return Err(self.invalid("no positions".into()));
        }
        Ok(MeshGeometry {
            positions,
            normals: read(channel::NORMAL)?
                .into_iter()
                .map(|v| [v[0], v[1], v[2]])
                .collect(),
            tangents: read(channel::TANGENT)?,
            colors: read(channel::COLOR)?,
            uv0: read(channel::UV0)?
                .into_iter()
                .map(|v| [v[0], v[1]])
                .collect(),
            uv1: read(channel::UV1)?
                .into_iter()
                .map(|v| [v[0], v[1]])
                .collect(),
            sub_meshes: Vec::new(),
        })
    }

    fn decode_compressed(&self) -> Result<MeshGeometry> {
        let c = &self.compressed;
        let short = |what: &str| self.invalid(format!("packed {what} too short"));
        let vertices = c.vertices.floats().ok_or_else(|| short("vertices"))?;
        let n = vertices.len() / 3;
        let positions: Vec<[f32; 3]> = vertices
            .chunks_exact(3)
            .map(|v| [v[0], v[1], v[2]])
            .collect();

        // Normals and tangents keep x and y; z comes back from the length,
        // with its sign stored as a separate bit (tangents: also w's sign).
        let unpack = |values: &PackedBits, signs: &PackedBits, per: usize, what: &str| {
            if values.count == 0 {
                return Ok((Vec::new(), Vec::new()));
            }
            let v = values.floats().ok_or_else(|| short(what))?;
            let s = signs.ints().ok_or_else(|| short(what))?;
            if v.len() != 2 * n || s.len() != per * n {
                return Err(self.invalid(format!(
                    "{what}: {} values, {} signs for {n} vertices",
                    v.len(),
                    s.len()
                )));
            }
            Ok((v, s))
        };
        let z = |x: f32, y: f32, positive: bool| {
            let z = (1.0 - x * x - y * y).max(0.0).sqrt();
            if positive { z } else { -z }
        };
        let (nv, ns) = unpack(&c.normals, &c.normal_signs, 1, "normals")?;
        let normals = (0..nv.len() / 2)
            .map(|i| {
                let (x, y) = (nv[2 * i], nv[2 * i + 1]);
                [x, y, z(x, y, ns[i] != 0)]
            })
            .collect();
        let (tv, ts) = unpack(&c.tangents, &c.tangent_signs, 2, "tangents")?;
        let tangents = (0..tv.len() / 2)
            .map(|i| {
                let (x, y) = (tv[2 * i], tv[2 * i + 1]);
                let w = if ts[2 * i + 1] != 0 { 1.0 } else { -1.0 };
                [x, y, z(x, y, ts[2 * i] != 0), w]
            })
            .collect();

        let uv = if c.uv.count > 0 {
            c.uv.floats().ok_or_else(|| short("UVs"))?
        } else {
            Vec::new()
        };
        // (set, start, dimension) of each texture-coordinate set present.
        let mut sets = Vec::new();
        if c.uv_info == 0 {
            for set in 0..2 {
                if uv.len() >= (set + 1) * 2 * n {
                    sets.push((set, set * 2 * n, 2));
                }
            }
        } else {
            let mut at = 0;
            for set in 0..8 {
                let bits = (c.uv_info >> (set * 4)) & 0xf;
                if bits & 4 != 0 {
                    let dim = 1 + (bits & 3) as usize;
                    sets.push((set, at, dim));
                    at += dim * n;
                }
            }
        }
        let (mut uv0, mut uv1) = (Vec::new(), Vec::new());
        for (set, start, dim) in sets {
            let target = match set {
                0 => &mut uv0,
                1 => &mut uv1,
                _ => continue,
            };
            let values = uv.get(start..start + dim * n).ok_or_else(|| short("UVs"))?;
            *target = values
                .chunks_exact(dim)
                .map(|v| [v[0], v.get(1).copied().unwrap_or(0.0)])
                .collect();
        }
        let colors = if c.float_colors.count as usize == 4 * n && n > 0 {
            c.float_colors
                .floats()
                .ok_or_else(|| short("colours"))?
                .chunks_exact(4)
                .map(|v| [v[0], v[1], v[2], v[3]])
                .collect()
        } else {
            Vec::new()
        };
        Ok(MeshGeometry {
            positions,
            normals,
            tangents,
            colors,
            uv0,
            uv1,
            sub_meshes: Vec::new(),
        })
    }
}

fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let frac = f32::from(bits & 0x3ff);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 if frac == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// One component in `format` (little-endian), as a float; normalised formats
/// map to 0..1 or −1..1.
fn component(format: u8, b: &[u8]) -> f32 {
    match (format, b) {
        (0, [a, b, c, d]) => f32::from_le_bytes([*a, *b, *c, *d]),
        (1, [a, b]) => half(u16::from_le_bytes([*a, *b])),
        (2, [a]) => f32::from(*a) / 255.0,
        (3, [a]) => (f32::from(*a as i8) / 127.0).max(-1.0),
        (4, [a, b]) => f32::from(u16::from_le_bytes([*a, *b])) / 65535.0,
        (5, [a, b]) => (f32::from(i16::from_le_bytes([*a, *b])) / 32767.0).max(-1.0),
        (6, [a]) => f32::from(*a),
        (7, [a]) => f32::from(*a as i8),
        (8, [a, b]) => f32::from(u16::from_le_bytes([*a, *b])),
        (9, [a, b]) => f32::from(i16::from_le_bytes([*a, *b])),
        (10, [a, b, c, d]) => u32::from_le_bytes([*a, *b, *c, *d]) as f32,
        (11, [a, b, c, d]) => i32::from_le_bytes([*a, *b, *c, *d]) as f32,
        _ => 0.0,
    }
}

/// Decoded geometry in Unity's local space (left-handed). Optional
/// attributes are empty when the mesh has none.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshGeometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 4]>,
    pub colors: Vec<[f32; 4]>,
    pub uv0: Vec<[f32; 2]>,
    pub uv1: Vec<[f32; 2]>,
    /// Per sub-mesh: triangle-list indices into the vertex arrays.
    pub sub_meshes: Vec<Vec<u32>>,
}

/// `MeshFilter` (class 33).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshFilter {
    pub game_object: PPtr,
    pub mesh: PPtr,
}

impl MeshFilter {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<MeshFilter> {
        let mut r = Reader::new(data, big_endian);
        Ok(MeshFilter {
            game_object: PPtr::read(&mut r)?,
            mesh: PPtr::read(&mut r)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0xc000), -2.0);
        assert_eq!(half(0x3800), 0.5);
        assert_eq!(half(0x0001), 2f32.powi(-24));
        assert_eq!(half(0x7c00), f32::INFINITY);
    }

    #[test]
    fn normalised_formats() {
        assert_eq!(component(2, &[255]), 1.0);
        assert_eq!(component(3, &[0x81]), -1.0);
        assert_eq!(component(5, &[0xff, 0x7f]), 1.0);
    }

    #[test]
    fn packed_bits_unpack_lsb_first() {
        // 5, 3, 7, 1 in 3 bits each.
        let mut bits = 0u32;
        for (i, v) in [5u32, 3, 7, 1].iter().enumerate() {
            bits |= v << (3 * i);
        }
        let p = PackedBits {
            count: 4,
            range: 7.0,
            start: -1.0,
            data: bits.to_le_bytes()[..2].to_vec(),
            bit_size: 3,
        };
        assert_eq!(p.ints().unwrap(), vec![5, 3, 7, 1]);
        assert_eq!(p.floats().unwrap(), vec![4.0, 2.0, 6.0, 0.0]);
        let too_many = PackedBits { count: 6, ..p };
        assert!(too_many.ints().is_none());
    }

    /// A one-triangle mesh with interleaved float3 positions and half2 UVs,
    /// 16-bit indices, one sub-mesh with a base vertex.
    #[test]
    fn decodes_interleaved_streams() {
        let mut vertex = Vec::new();
        for (p, uv) in [
            ([0.0f32, 0.0, 0.0], [0x0000u16, 0x0000]),
            ([1.0, 0.0, 0.0], [0x3c00, 0x0000]),
            ([0.0, 1.0, 0.0], [0x0000, 0x3c00]),
            ([5.0, 5.0, 5.0], [0x0000, 0x0000]),
        ] {
            for v in p {
                vertex.extend(v.to_le_bytes());
            }
            for v in uv {
                vertex.extend(v.to_le_bytes());
            }
        }
        let mut channels = vec![Channel::default(); 14];
        channels[channel::POSITION] = Channel {
            stream: 0,
            offset: 0,
            format: 0,
            dimension: 3,
        };
        channels[channel::UV0] = Channel {
            stream: 0,
            offset: 12,
            format: 1,
            dimension: 2,
        };
        let mesh = Mesh {
            name: "t".into(),
            sub_meshes: vec![SubMesh {
                first_byte: 2,
                index_count: 3,
                topology: 0,
                base_vertex: 1,
                first_vertex: 0,
                vertex_count: 3,
            }],
            compression: 0,
            index_format: 0,
            index_buffer: [9u16, 0, 1, 2]
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect(),
            vertex_count: 4,
            channels,
            vertex_data: vertex.clone(),
            compressed: CompressedMesh::default(),
            stream: None,
            aabb: ([0.0; 3], [0.0; 3]),
        };
        // 4 vertices × (12 + 4) bytes, already a multiple of 16.
        assert_eq!(mesh.vertex_data_size(), Some(64));
        let g = mesh.decode(&vertex).unwrap();
        assert_eq!(g.positions[3], [5.0, 5.0, 5.0]);
        assert_eq!(g.uv0[1], [1.0, 0.0]);
        assert!(g.normals.is_empty());
        assert_eq!(g.sub_meshes, vec![vec![1, 2, 3]]);
        // An index past the vertices is an error, not a panic.
        let mut bad = mesh.clone();
        bad.sub_meshes[0].base_vertex = 2;
        assert!(bad.decode(&vertex).is_err());
        assert!(mesh.decode(&vertex[..40]).is_err());
    }
}
