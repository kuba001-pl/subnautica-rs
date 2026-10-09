//! Saved world objects: the game's placement caches and its ClassId table.
//! See `docs/formats/entities.md`.
//!
//! - `BatchObjectsCache/batch-objects-X-Y-Z.bin`: one object tree.
//! - `CellsCache/baked-batch-cells-X-Y-Z.bin`: a header, then per cell a
//!   header and (usually) an object tree.
//! - `prefabs.db`: ClassId → prefab path.
//!
//! An object tree is a sequence of length-prefixed protobuf messages: a
//! header, the object count, and per object the GameObject, its component
//! count and per component a type-name message plus the component's data.

use std::collections::HashMap;

use crate::wire::{Result, Value, Wire, WireError};

pub use crate::wire::WireError as EntityError;

/// First field of every object tree's header.
pub const TREE_MAGIC: u64 = 1_369_164_567;

/// Position, rotation (quaternion x, y, z, w) and scale, relative to the
/// parent; Unity coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }
    }
}

impl Transform {
    /// `child` placed in this transform's space (Unity's parent × local).
    pub fn then(&self, child: &Transform) -> Transform {
        let scaled = [0, 1, 2].map(|a| child.position[a] * self.scale[a]);
        let rotated = rotate(self.rotation, scaled);
        Transform {
            position: [0, 1, 2].map(|a| self.position[a] + rotated[a]),
            rotation: quat_mul(self.rotation, child.rotation),
            scale: [0, 1, 2].map(|a| self.scale[a] * child.scale[a]),
        }
    }

    /// A point in this transform's space → the parent's space (scale,
    /// rotate, translate).
    pub fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let r = self.rotate_vector([0, 1, 2].map(|a| p[a] * self.scale[a]));
        [0, 1, 2].map(|a| self.position[a] + r[a])
    }

    /// A direction rotated by this transform (no scale).
    pub fn rotate_vector(&self, v: [f32; 3]) -> [f32; 3] {
        rotate(self.rotation, v)
    }
}

pub(crate) fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let p = quat_mul(
        quat_mul(q, [v[0], v[1], v[2], 0.0]),
        [-q[0], -q[1], -q[2], q[3]],
    );
    [p[0], p[1], p[2]]
}

/// One saved component: its type name and its serialized data.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedComponent {
    pub type_name: String,
    pub data: Vec<u8>,
}

/// One saved GameObject.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedObject {
    pub id: String,
    /// The prefab's ClassId; empty for objects without a prefab (baked lights).
    pub class_id: String,
    /// The parent object's id; `None` for roots.
    pub parent: Option<String>,
    pub tag: String,
    /// Field 3: 0 or 21 (**hypothesis**: the Unity layer).
    pub layer: u32,
    /// Varint fields not understood yet (1, 2, 9, 10), as (field, value).
    pub unknown: Vec<(u32, u64)>,
    /// From the `UnityEngine.Transform` component (default if absent).
    pub transform: Transform,
    pub components: Vec<SavedComponent>,
}

/// A parsed object tree.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectTree {
    /// Header field 2 (4 in build 10).
    pub version: u64,
    pub objects: Vec<SavedObject>,
}

fn string(w: &Wire, value: Wire) -> Result<String> {
    String::from_utf8(value.bytes().to_vec()).or_else(|_| w.error("string is not UTF-8"))
}

pub(crate) fn bytes_field<'a>(w: &Wire, number: u32, value: Value<'a>) -> Result<Wire<'a>> {
    match value {
        Value::Bytes(b) => Ok(b),
        other => w.error(format!(
            "field {number}: {} where bytes were expected",
            other.kind()
        )),
    }
}

pub(crate) fn varint_field(w: &Wire, number: u32, value: Value) -> Result<u64> {
    match value {
        Value::Varint(v) => Ok(v),
        other => w.error(format!(
            "field {number}: {} where a varint was expected",
            other.kind()
        )),
    }
}

/// A message whose only field of interest is varint field 1 (counts).
fn count(w: &mut Wire) -> Result<u64> {
    let mut m = w.length_prefixed()?;
    let mut n = 0;
    while let Some((number, value)) = m.field()? {
        if number == 1 {
            n = varint_field(&m, number, value)?;
        }
    }
    Ok(n)
}

/// Floats in fields 1, 2, 3, … of a message; absent fields keep `out`'s value.
pub(crate) fn floats<const N: usize>(mut m: Wire, mut out: [f32; N]) -> Result<[f32; N]> {
    while let Some((number, value)) = m.field()? {
        let i = number as usize;
        match value {
            Value::Fixed32(bits) if (1..=N).contains(&i) => out[i - 1] = f32::from_bits(bits),
            Value::Fixed32(_) => {}
            other => return m.error(format!("field {number}: {} in a vector", other.kind())),
        }
    }
    Ok(out)
}

fn parse_transform(mut m: Wire) -> Result<Transform> {
    // Protobuf leaves out zero fields, so missing components are 0.
    let mut t = Transform {
        position: [0.0; 3],
        rotation: [0.0; 4],
        scale: [0.0; 3],
    };
    while let Some((number, value)) = m.field()? {
        let sub = bytes_field(&m, number, value)?;
        match number {
            1 => t.position = floats(sub, [0.0; 3])?,
            2 => t.rotation = floats(sub, [0.0; 4])?,
            3 => t.scale = floats(sub, [0.0; 3])?,
            _ => {}
        }
    }
    Ok(t)
}

fn parse_object(w: &mut Wire) -> Result<SavedObject> {
    let mut m = w.length_prefixed()?;
    let mut object = SavedObject {
        id: String::new(),
        class_id: String::new(),
        parent: None,
        tag: String::new(),
        layer: 0,
        unknown: Vec::new(),
        transform: Transform::default(),
        components: Vec::new(),
    };
    while let Some((number, value)) = m.field()? {
        match number {
            3 => {
                let v = varint_field(&m, number, value)?;
                object.layer = u32::try_from(v).or_else(|_| m.error("layer too big"))?;
            }
            4 => object.tag = string(&m, bytes_field(&m, number, value)?)?,
            6 => object.id = string(&m, bytes_field(&m, number, value)?)?,
            7 => object.class_id = string(&m, bytes_field(&m, number, value)?)?,
            8 => object.parent = Some(string(&m, bytes_field(&m, number, value)?)?),
            _ => match value {
                Value::Varint(v) => object.unknown.push((number, v)),
                other => {
                    return m.error(format!(
                        "GameObject field {number}: unexpected {}",
                        other.kind()
                    ));
                }
            },
        }
    }
    let components = count(w)?;
    for _ in 0..components {
        let mut header = w.length_prefixed()?;
        let mut type_name = None;
        while let Some((number, value)) = header.field()? {
            if number == 1 {
                type_name = Some(string(&header, bytes_field(&header, number, value)?)?);
            }
        }
        let Some(type_name) = type_name else {
            return header.error("component without a type name");
        };
        let data = w.length_prefixed()?;
        if type_name == "UnityEngine.Transform" {
            object.transform = parse_transform(data.clone())?;
        }
        object.components.push(SavedComponent {
            type_name,
            data: data.bytes().to_vec(),
        });
    }
    Ok(object)
}

fn parse_tree(w: &mut Wire) -> Result<ObjectTree> {
    let mut header = w.length_prefixed()?;
    let (mut magic, mut version) = (None, 0);
    while let Some((number, value)) = header.field()? {
        match number {
            1 => magic = Some(varint_field(&header, number, value)?),
            2 => version = varint_field(&header, number, value)?,
            _ => {}
        }
    }
    if magic != Some(TREE_MAGIC) {
        return header.error(format!(
            "object tree magic {magic:?}, expected {TREE_MAGIC}"
        ));
    }
    let n = count(w)?;
    // Each object takes at least a few bytes; don't trust huge counts.
    if n > w.bytes().len() as u64 {
        return w.error(format!("{n} objects can't fit"));
    }
    let mut objects = Vec::with_capacity(n as usize);
    for _ in 0..n {
        objects.push(parse_object(w)?);
    }
    Ok(ObjectTree { version, objects })
}

impl ObjectTree {
    /// Parses a whole `batch-objects-*.bin` file.
    pub fn parse(data: &[u8]) -> Result<ObjectTree> {
        let mut w = Wire::new(data);
        let tree = parse_tree(&mut w)?;
        if !w.is_empty() {
            return w.error("bytes left after the object tree");
        }
        Ok(tree)
    }

    /// Every object's transform in world space (parents applied), in object
    /// order, and the number of objects whose parent is not in the tree
    /// (those keep their own transform as if they were roots).
    pub fn world_transforms(&self) -> (Vec<Transform>, usize) {
        let index: HashMap<&str, usize> = self
            .objects
            .iter()
            .enumerate()
            .map(|(i, o)| (o.id.as_str(), i))
            .collect();
        let parent_of = |i: usize| {
            self.objects[i]
                .parent
                .as_deref()
                .and_then(|p| index.get(p).copied())
        };
        let orphans = self
            .objects
            .iter()
            .filter(|o| o.parent.as_deref().is_some_and(|p| !index.contains_key(p)))
            .count();
        let mut world: Vec<Option<Transform>> = vec![None; self.objects.len()];
        for start in 0..self.objects.len() {
            // Climb to a root or an already placed ancestor, then go back down.
            let mut chain = vec![start];
            let mut current = start;
            let mut base: Option<Transform> = None;
            while let Some(p) = parent_of(current) {
                if let Some(t) = world[p] {
                    base = Some(t);
                    break;
                }
                if chain.contains(&p) {
                    break; // a cycle: treat the top as a root
                }
                chain.push(p);
                current = p;
            }
            for &i in chain.iter().rev() {
                if world[i].is_some() {
                    base = world[i];
                    continue;
                }
                let own = &self.objects[i].transform;
                let t = base.map_or(*own, |b| b.then(own));
                world[i] = Some(t);
                base = Some(t);
            }
        }
        (
            world.into_iter().map(Option::unwrap_or_default).collect(),
            orphans,
        )
    }
}

/// One cell of a `baked-batch-cells` file.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedCell {
    /// Cell coordinates within the batch at its level.
    pub id: [i32; 3],
    pub level: u32,
    /// The cell's objects (header field 3); `None` if the length is 0.
    pub objects: Option<ObjectTree>,
    /// Header fields 4 and 5: lengths of further data blobs (never seen
    /// non-zero in build 10); the bytes are kept.
    pub legacy: Vec<u8>,
    pub waiters: Vec<u8>,
}

/// A parsed `baked-batch-cells-*.bin` file.
#[derive(Clone, Debug, PartialEq)]
pub struct BatchCells {
    /// Header field 1 (9 or 10 in build 10).
    pub version: u64,
    pub cells: Vec<BakedCell>,
}

fn int3(mut m: Wire) -> Result<[i32; 3]> {
    let mut out = [0i32; 3];
    while let Some((number, value)) = m.field()? {
        let v = varint_field(&m, number, value)?;
        if (1..=3).contains(&number) {
            // protobuf int32: negative values are sign-extended to 64 bits.
            out[number as usize - 1] = v as i64 as i32;
        }
    }
    Ok(out)
}

impl BatchCells {
    pub fn parse(data: &[u8]) -> Result<BatchCells> {
        let mut w = Wire::new(data);
        let mut header = w.length_prefixed()?;
        // Field 2 is the number of cells that follow.
        let (mut version, mut expected) = (0, 0);
        while let Some((number, value)) = header.field()? {
            match number {
                1 => version = varint_field(&header, number, value)?,
                2 => expected = varint_field(&header, number, value)?,
                _ => {}
            }
        }
        let mut cells = Vec::new();
        while !w.is_empty() {
            let mut h = w.length_prefixed()?;
            let mut id = [0; 3];
            let mut level = 0;
            let mut lengths = [0usize; 3];
            while let Some((number, value)) = h.field()? {
                match number {
                    1 => id = int3(bytes_field(&h, number, value)?)?,
                    2 => {
                        let v = varint_field(&h, number, value)?;
                        level = u32::try_from(v).or_else(|_| h.error("level too big"))?;
                    }
                    3..=5 => {
                        let v = varint_field(&h, number, value)?;
                        lengths[number as usize - 3] =
                            usize::try_from(v).or_else(|_| h.error("length too big"))?;
                    }
                    _ => {}
                }
            }
            let objects = if lengths[0] > 0 {
                let mut blob = w.raw(lengths[0])?;
                let tree = parse_tree(&mut blob)?;
                if !blob.is_empty() {
                    return blob.error("bytes left after the cell's object tree");
                }
                Some(tree)
            } else {
                None
            };
            let legacy = w.raw(lengths[1])?.bytes().to_vec();
            let waiters = w.raw(lengths[2])?.bytes().to_vec();
            cells.push(BakedCell {
                id,
                level,
                objects,
                legacy,
                waiters,
            });
        }
        if cells.len() as u64 != expected {
            return w.error(format!("{} cells, the header says {expected}", cells.len()));
        }
        Ok(BatchCells { version, cells })
    }
}

/// `prefabs.db`: an `i32` count, then that many (ClassId, prefab path) pairs
/// of .NET `BinaryWriter` strings (7-bit-encoded length, UTF-8).
pub fn parse_prefab_database(
    data: &[u8],
) -> std::result::Result<Vec<(String, String)>, EntityError> {
    let err = |offset: usize, message: &str| WireError {
        offset,
        message: message.into(),
    };
    let count = data
        .get(..4)
        .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| err(0, "no count"))?;
    let count = usize::try_from(count).map_err(|_| err(0, "negative count"))?;
    if count > data.len() {
        return Err(err(0, "count larger than the file"));
    }
    let mut pos = 4;
    let read = |pos: &mut usize| -> std::result::Result<String, EntityError> {
        let start = *pos;
        let mut len = 0usize;
        for shift in (0..35).step_by(7) {
            let byte = *data
                .get(*pos)
                .ok_or_else(|| err(start, "string length runs past the end"))?;
            *pos += 1;
            len |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                let end = pos.checked_add(len).filter(|&e| e <= data.len());
                let end = end.ok_or_else(|| err(start, "string runs past the end"))?;
                let s = std::str::from_utf8(&data[*pos..end])
                    .map_err(|_| err(*pos, "string is not UTF-8"))?;
                *pos = end;
                return Ok(s.to_string());
            }
        }
        Err(err(start, "string length too long"))
    };
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let class_id = read(&mut pos)?;
        let path = read(&mut pos)?;
        out.push((class_id, path));
    }
    if pos != data.len() {
        return Err(err(pos, "bytes left after the last entry"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::encode::*;

    fn vector(values: &[f32]) -> Vec<u8> {
        let mut m = Vec::new();
        for (i, &v) in values.iter().enumerate() {
            if v != 0.0 {
                float(&mut m, i as u32 + 1, v);
            }
        }
        m
    }

    fn transform(pos: [f32; 3], rot: [f32; 4], scale: [f32; 3]) -> Vec<u8> {
        let mut m = Vec::new();
        bytes(&mut m, 1, &vector(&pos));
        bytes(&mut m, 2, &vector(&rot));
        bytes(&mut m, 3, &vector(&scale));
        m
    }

    /// An object tree in the game's layout.
    fn tree(objects: &[(&str, &str, Option<&str>, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut header = Vec::new();
        uint(&mut header, 1, TREE_MAGIC);
        uint(&mut header, 2, 4);
        prefixed(&mut out, &header);
        let mut n = Vec::new();
        uint(&mut n, 1, objects.len() as u64);
        prefixed(&mut out, &n);
        for (id, class, parent, t) in objects {
            let mut go = Vec::new();
            uint(&mut go, 1, 0);
            uint(&mut go, 2, 1);
            uint(&mut go, 3, 21);
            bytes(&mut go, 4, b"Untagged");
            bytes(&mut go, 6, id.as_bytes());
            bytes(&mut go, 7, class.as_bytes());
            if let Some(p) = parent {
                bytes(&mut go, 8, p.as_bytes());
            }
            prefixed(&mut out, &go);
            let mut n = Vec::new();
            uint(&mut n, 1, 2);
            prefixed(&mut out, &n);
            let mut name = Vec::new();
            bytes(&mut name, 1, b"UnityEngine.Transform");
            uint(&mut name, 2, 1);
            prefixed(&mut out, &name);
            prefixed(&mut out, t);
            let mut name = Vec::new();
            bytes(&mut name, 1, b"LargeWorldEntity");
            uint(&mut name, 2, 1);
            prefixed(&mut out, &name);
            prefixed(&mut out, &[0x10, 0x02]);
        }
        out
    }

    fn sample() -> Vec<u8> {
        // Root at (10, 0, 0) turned 90° about Y; child 1 m along its local X.
        let s = std::f32::consts::FRAC_1_SQRT_2;
        tree(&[
            (
                "root",
                "class-a",
                None,
                transform([10.0, 0.0, 0.0], [0.0, s, 0.0, s], [2.0, 2.0, 2.0]),
            ),
            (
                "child",
                "class-b",
                Some("root"),
                transform([1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            ),
        ])
    }

    #[test]
    fn transform_point_scales_rotates_and_moves() {
        // 90° about y: +x goes to -z (Unity, left-handed).
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let t = Transform {
            position: [1.0, 2.0, 3.0],
            rotation: [0.0, h, 0.0, h],
            scale: [2.0, 1.0, 1.0],
        };
        let p = t.transform_point([1.0, 0.0, 0.0]);
        let want = [1.0, 2.0, 1.0];
        assert!((0..3).all(|a| (p[a] - want[a]).abs() < 1e-5), "{p:?}");
        // Same as placing a child there.
        let child = Transform {
            position: [1.0, 0.0, 0.0],
            ..Transform::default()
        };
        assert_eq!(t.then(&child).position, p);
    }

    #[test]
    fn parses_objects_components_and_transforms() {
        let t = ObjectTree::parse(&sample()).unwrap();
        assert_eq!(t.version, 4);
        assert_eq!(t.objects.len(), 2);
        let child = &t.objects[1];
        assert_eq!(child.id, "child");
        assert_eq!(child.class_id, "class-b");
        assert_eq!(child.parent.as_deref(), Some("root"));
        assert_eq!(child.tag, "Untagged");
        assert_eq!(child.layer, 21);
        assert_eq!(child.unknown, vec![(1, 0), (2, 1)]);
        let names: Vec<&str> = child
            .components
            .iter()
            .map(|c| c.type_name.as_str())
            .collect();
        assert_eq!(names, vec!["UnityEngine.Transform", "LargeWorldEntity"]);
        assert_eq!(child.transform.position, [1.0, 0.0, 0.0]);
        assert_eq!(t.objects[0].transform.scale, [2.0, 2.0, 2.0]);
    }

    #[test]
    fn children_are_placed_in_their_parents_space() {
        let t = ObjectTree::parse(&sample()).unwrap();
        let (world, orphans) = t.world_transforms();
        assert_eq!(orphans, 0);
        assert_eq!(world[0], t.objects[0].transform);
        // Local +X, scaled by 2, turned 90° about Y → world −Z.
        let p = world[1].position;
        assert!(
            (p[0] - 10.0).abs() < 1e-5 && p[1].abs() < 1e-5 && (p[2] + 2.0).abs() < 1e-5,
            "{p:?}"
        );
        assert_eq!(world[1].scale, [2.0, 2.0, 2.0]);
    }

    #[test]
    fn missing_parents_are_counted() {
        let data = tree(&[(
            "lost",
            "c",
            Some("nowhere"),
            transform([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
        )]);
        let t = ObjectTree::parse(&data).unwrap();
        let (world, orphans) = t.world_transforms();
        assert_eq!(orphans, 1);
        assert_eq!(world[0].position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn baked_cells_hold_object_trees() {
        let objects = sample();
        let mut out = Vec::new();
        let mut header = Vec::new();
        uint(&mut header, 1, 9);
        uint(&mut header, 2, 2);
        prefixed(&mut out, &header);
        for (id, blob) in [([0u64, 7, 8], objects.clone()), ([1, 2, 3], Vec::new())] {
            let mut cell = Vec::new();
            let mut c = Vec::new();
            for (i, v) in id.iter().enumerate() {
                uint(&mut c, i as u32 + 1, *v);
            }
            bytes(&mut cell, 1, &c);
            uint(&mut cell, 2, 1);
            uint(&mut cell, 3, blob.len() as u64);
            uint(&mut cell, 4, 0);
            uint(&mut cell, 5, 0);
            prefixed(&mut out, &cell);
            out.extend_from_slice(&blob);
        }
        let cells = BatchCells::parse(&out).unwrap();
        assert_eq!(cells.version, 9);
        assert_eq!(cells.cells.len(), 2);
        assert_eq!(cells.cells[0].id, [0, 7, 8]);
        assert_eq!(cells.cells[0].level, 1);
        assert_eq!(
            cells.cells[0].objects.as_ref().map(|t| t.objects.len()),
            Some(2)
        );
        assert!(cells.cells[1].objects.is_none());
        // A wrong cell count in the header is an error. Header bytes:
        // length, tag 1, 9, tag 2, count.
        assert_eq!(out[4], 2);
        out[4] = 3;
        let err = BatchCells::parse(&out).unwrap_err();
        assert!(err.message.contains("the header says 3"), "{err}");
    }

    #[test]
    fn prefab_database_round_trip() {
        let mut data = 2i32.to_le_bytes().to_vec();
        for s in ["id-1", "WorldEntities/a.prefab", "id-2", "b.prefab"] {
            data.push(s.len() as u8);
            data.extend_from_slice(s.as_bytes());
        }
        let db = parse_prefab_database(&data).unwrap();
        assert_eq!(db[1], ("id-2".to_string(), "b.prefab".to_string()));
        data.push(0);
        assert!(parse_prefab_database(&data).is_err());
    }

    #[test]
    fn corrupt_input_never_panics() {
        let data = sample();
        for i in 0..data.len() {
            let _ = ObjectTree::parse(&data[..i]);
            for flip in [0x00, 0xff, 0x80] {
                let mut bad = data.clone();
                bad[i] = flip;
                let _ = ObjectTree::parse(&bad);
                let _ = BatchCells::parse(&bad);
                let _ = parse_prefab_database(&bad);
            }
        }
        assert!(ObjectTree::parse(&data[..data.len() - 1]).is_err());
    }
}
