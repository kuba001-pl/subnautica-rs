//! Script fields by layout (M9u1, `docs/formats/ui.md` § Script fields):
//! a MonoBehaviour's fields read from a [`ScriptLayout`] that says, field
//! by field, how Unity 2019.4 stored them. The layouts come from the
//! game's assemblies (`sn-assets::script_layout`); this module only
//! reads bytes, so it is tested on synthetic layouts and data.
//!
//! Storage, as Unity writes it without type trees: numbers little-endian
//! at their own size; a field marked `align` is followed by padding to
//! 4 bytes (Unity marks fields smaller than 4 bytes and every string and
//! array); strings are `i32` length + UTF-8 + padding to 4 (also inside
//! arrays); arrays are `i32` count +
//! elements (elements are not padded one by one); object references are
//! PPtrs; nested classes and structs are their fields inline.

use std::sync::Arc;

use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;
use crate::{ErrorKind, Result};

/// How one field is stored.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    F32,
    F64,
    String,
    PPtr,
    /// `T[]` and `List<T>`.
    Array(Box<Kind>),
    /// A serializable class or struct, or one of Unity's built-in structs
    /// (`Vector3`, `Color`, `AnimationCurve`, …): its fields inline.
    Struct(Arc<StructLayout>),
}

impl Kind {
    /// The fewest bytes one value takes (bounds array counts).
    pub fn min_size(&self) -> usize {
        match self {
            Kind::Bool | Kind::I8 | Kind::U8 => 1,
            Kind::I16 | Kind::U16 => 2,
            Kind::I32 | Kind::U32 | Kind::F32 | Kind::String | Kind::Array(_) => 4,
            Kind::I64 | Kind::U64 | Kind::F64 => 8,
            Kind::PPtr => 12,
            Kind::Struct(s) => s.fields.iter().map(|f| f.kind.min_size()).sum(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub kind: Kind,
    /// Padding to 4 bytes after the value.
    pub align: bool,
}

impl Field {
    /// A script field: small numbers, strings and arrays are aligned.
    pub fn script(name: impl Into<String>, kind: Kind) -> Field {
        let align = matches!(
            kind,
            Kind::Bool
                | Kind::I8
                | Kind::U8
                | Kind::I16
                | Kind::U16
                | Kind::String
                | Kind::Array(_)
        );
        Field {
            name: name.into(),
            kind,
            align,
        }
    }

    /// A field stored without padding after it.
    pub fn packed(name: impl Into<String>, kind: Kind) -> Field {
        Field {
            name: name.into(),
            kind,
            align: false,
        }
    }
}

/// A class's or struct's stored fields, in order (base classes first).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StructLayout {
    /// For messages: `Namespace.Name`.
    pub type_name: String,
    pub fields: Vec<Field>,
}

/// The stored fields of a script (after the MonoBehaviour header).
pub type ScriptLayout = StructLayout;

/// A read value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    PPtr(PPtr),
    Array(Vec<Value>),
    /// A `byte[]`.
    Bytes(Vec<u8>),
    Struct(Fields),
}

/// Field names and values, in order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields(pub Vec<(String, Value)>);

impl Fields {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// A value by a dotted path (`m_Color.r`).
    pub fn path(&self, path: &str) -> Option<&Value> {
        let mut parts = path.split('.');
        let mut v = self.get(parts.next()?)?;
        for p in parts {
            match v {
                Value::Struct(f) => v = f.get(p)?,
                _ => return None,
            }
        }
        Some(v)
    }
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Float(f) => Some(f),
            Value::Int(i) => Some(i as f64),
            Value::UInt(u) => Some(u as f64),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Int(i) => Some(i),
            Value::UInt(u) => i64::try_from(u).ok(),
            Value::Bool(b) => Some(i64::from(b)),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Value::Bool(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_pptr(&self) -> Option<PPtr> {
        match *self {
            Value::PPtr(p) => Some(p),
            _ => None,
        }
    }

    pub fn as_struct(&self) -> Option<&Fields> {
        match self {
            Value::Struct(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// A MonoBehaviour read by its script's layout: the header and every
/// field. The data must end with the last field.
pub fn read_script(
    data: &[u8],
    big_endian: bool,
    layout: &ScriptLayout,
) -> Result<(MonoBehaviourHeader, Fields)> {
    let header = MonoBehaviourHeader::parse(data, big_endian)?;
    let mut r = Reader::new(data, big_endian);
    r.seek(header.fields_offset)?;
    let fields = read_fields(&mut r, &layout.fields)?;
    if r.pos() != data.len() {
        return Err(r.error(ErrorKind::Invalid(format!(
            "{}: {} bytes after the last field",
            layout.type_name,
            data.len() - r.pos()
        ))));
    }
    Ok((header, fields))
}

/// Where each field starts (offset in the object), read as far as the
/// data allows: for finding where a layout and the data part.
pub type Trace = Vec<(String, usize, Result<Value>)>;

pub fn trace_script(data: &[u8], big_endian: bool, layout: &ScriptLayout) -> Result<Trace> {
    let header = MonoBehaviourHeader::parse(data, big_endian)?;
    let mut r = Reader::new(data, big_endian);
    r.seek(header.fields_offset)?;
    let mut out = Vec::new();
    for f in &layout.fields {
        let at = r.pos();
        let v = read_value(&mut r, &f.kind).and_then(|v| {
            if f.align {
                r.align(4)?;
            }
            Ok(v)
        });
        let failed = v.is_err();
        out.push((f.name.clone(), at, v));
        if failed {
            break;
        }
    }
    Ok(out)
}

fn read_fields(r: &mut Reader, fields: &[Field]) -> Result<Fields> {
    let mut out = Vec::with_capacity(fields.len());
    for f in fields {
        let v = read_value(r, &f.kind)?;
        if f.align {
            r.align(4)?;
        }
        out.push((f.name.clone(), v));
    }
    Ok(Fields(out))
}

fn read_value(r: &mut Reader, kind: &Kind) -> Result<Value> {
    Ok(match kind {
        Kind::Bool => Value::Bool(r.u8()? != 0),
        Kind::I8 => Value::Int(i64::from(r.u8()? as i8)),
        Kind::U8 => Value::UInt(u64::from(r.u8()?)),
        Kind::I16 => Value::Int(i64::from(r.i16()?)),
        Kind::U16 => Value::UInt(u64::from(r.u16()?)),
        Kind::I32 => Value::Int(i64::from(r.i32()?)),
        Kind::U32 => Value::UInt(u64::from(r.u32()?)),
        Kind::I64 => Value::Int(r.i64()?),
        Kind::U64 => Value::UInt(r.u64()?),
        Kind::F32 => Value::Float(f64::from(r.f32()?)),
        Kind::F64 => Value::Float(r.f64()?),
        Kind::String => {
            // A string is an aligned array of bytes, also as an array's
            // element (`CompoundGoal.preconditions`, M9u1).
            let n = r.count(1)?;
            let s = String::from_utf8_lossy(r.bytes(n)?).into_owned();
            r.align(4)?;
            Value::Str(s)
        }
        Kind::PPtr => Value::PPtr(PPtr::read(r)?),
        Kind::Array(element) => {
            if **element == Kind::U8 {
                let n = r.count(1)?;
                Value::Bytes(r.bytes(n)?.to_vec())
            } else {
                let n = r.count(element.min_size().max(1))?;
                let mut items = Vec::with_capacity(n);
                for _ in 0..n {
                    items.push(read_value(r, element)?);
                }
                Value::Array(items)
            }
        }
        Kind::Struct(s) => Value::Struct(read_fields(r, &s.fields)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(b: &mut Vec<u8>) {
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&3i64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&40i64.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
    }

    fn vector2() -> Kind {
        Kind::Struct(Arc::new(StructLayout {
            type_name: "UnityEngine.Vector2".into(),
            fields: vec![Field::packed("x", Kind::F32), Field::packed("y", Kind::F32)],
        }))
    }

    fn layout() -> ScriptLayout {
        StructLayout {
            type_name: "Test".into(),
            fields: vec![
                Field::script("flag", Kind::Bool),
                Field::script("count", Kind::I32),
                Field::script("label", Kind::String),
                Field::script("size", vector2()),
                Field::script("points", Kind::Array(Box::new(vector2()))),
                Field::script("bytes", Kind::Array(Box::new(Kind::U8))),
                Field::script("target", Kind::PPtr),
                Field::script("small", Kind::I16),
                Field::packed("a", Kind::U8),
                Field::packed("b", Kind::U8),
                Field::script("tail", Kind::F64),
                Field::script("words", Kind::Array(Box::new(Kind::String))),
            ],
        }
    }

    fn data() -> Vec<u8> {
        let mut b = Vec::new();
        header(&mut b);
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&(-7i32).to_le_bytes());
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(b"hello\0\0\0");
        for v in [1.5f32, -2.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&2u32.to_le_bytes());
        for v in [0.0f32, 1.0, 2.0, 3.0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&[9, 8, 7, 0]);
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&77i64.to_le_bytes());
        b.extend_from_slice(&(-3i16).to_le_bytes());
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&[4, 5]);
        b.extend_from_slice(&0.25f64.to_le_bytes());
        // The array starts at 114: each string pads to the next multiple
        // of 4 in the object, not of its own length.
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&[b'a', 0]);
        b.extend_from_slice(&5u32.to_le_bytes());
        b.extend_from_slice(b"bcdef");
        b.extend_from_slice(&[0, 0, 0]);
        b
    }

    #[test]
    fn reads_every_kind() {
        let (h, f) = read_script(&data(), false, &layout()).unwrap();
        assert_eq!(h.game_object.path_id, 3);
        assert_eq!(f.get("flag"), Some(&Value::Bool(true)));
        assert_eq!(f.get("count").and_then(Value::as_i64), Some(-7));
        assert_eq!(f.get("label").and_then(Value::as_str), Some("hello"));
        assert_eq!(f.path("size.y").and_then(Value::as_f64), Some(-2.0));
        let points = f.get("points").and_then(Value::as_array).unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(
            points[1]
                .as_struct()
                .and_then(|p| p.get("x"))
                .and_then(Value::as_f64),
            Some(2.0)
        );
        assert_eq!(f.get("bytes"), Some(&Value::Bytes(vec![9, 8, 7])));
        let t = f.get("target").and_then(Value::as_pptr).unwrap();
        assert_eq!((t.file_id, t.path_id), (2, 77));
        assert_eq!(f.get("small").and_then(Value::as_i64), Some(-3));
        assert_eq!(f.get("b").and_then(Value::as_i64), Some(5));
        assert_eq!(f.get("tail").and_then(Value::as_f64), Some(0.25));
        let words: Vec<&str> = f
            .get("words")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(words, ["a", "bcdef"]);
        assert_eq!(f.path("size.z"), None);
        assert_eq!(f.path("count.x"), None);
    }

    #[test]
    fn short_long_and_corrupt_data_are_errors() {
        let d = data();
        let l = layout();
        for len in [0, 30, 40, 60, d.len() - 1] {
            assert!(read_script(&d[..len], false, &l).is_err(), "{len}");
        }
        let mut longer = d.clone();
        longer.extend_from_slice(&[0; 4]);
        let e = read_script(&longer, false, &l).unwrap_err();
        assert!(e.to_string().contains("4 bytes after"), "{e}");
        // A huge array count is an error, not an allocation.
        let mut huge = d.clone();
        let at = 32 + 4 + 4 + 12 + 8; // the header, flag, count, label, size
        huge[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read_script(&huge, false, &l).is_err());
    }
}
