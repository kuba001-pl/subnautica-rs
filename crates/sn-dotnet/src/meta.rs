//! Metadata M9u1 needs to know how Unity serializes a script's fields
//! (`docs/DESIGN.md` "M9u plan"): type flags, where a TypeRef points
//! (another assembly, an enclosing type), nested types, the assembly's
//! own name and the ones it references, custom attributes by type name.

use crate::assembly::Assembly;
use crate::reader::Reader;
use crate::tables::Table;
use crate::{Error, Result, TypeToken};

/// `TypeAttributes` (§ II.23.1.15).
pub const TYPE_INTERFACE: u32 = 0x0000_0020;
pub const TYPE_ABSTRACT: u32 = 0x0000_0080;
pub const TYPE_SERIALIZABLE: u32 = 0x0000_2000;

/// Where a TypeRef's type is defined (`ResolutionScope`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefScope {
    /// In the assembly of this name.
    Assembly(String),
    /// Nested in the type of this TypeRef row.
    Outer(u32),
    /// In this module (or a module of this assembly).
    Module,
}

/// A custom attribute: what it is on, and its type's names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub parent: TypeToken,
    pub namespace: String,
    pub name: String,
}

/// A custom attribute's named argument value (§ II.23.3), of the kinds
/// [`Assembly::attribute_named_args`] reads.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    Bool(bool),
    Int(i64),
    F32(f32),
    F64(f64),
    Str(Option<String>),
    /// An array (`SZARRAY`); `None`: a null array.
    Array(Option<Vec<AttrValue>>),
    /// An enum value (its type's name; the underlying type assumed
    /// `int32`, as C# enums are by default).
    Enum(String, i32),
}

/// Largest array in an attribute blob accepted.
const MAX_ATTR_ARRAY: u32 = 4096;

/// `SerString`: a packed length (0xFF: null) and UTF-8.
fn ser_string(r: &mut Reader) -> Result<Option<String>> {
    let pos = r.pos();
    if r.bytes(1)? == [0xFF] {
        return Ok(None);
    }
    r.seek(pos)?;
    let n = r.compressed()? as usize;
    let b = r.bytes(n)?;
    String::from_utf8(b.to_vec())
        .map(Some)
        .map_err(|_| r.error("SerString is not UTF-8"))
}

/// One value of element type `t` (`0x55`: the enum's name was read).
fn attr_value(r: &mut Reader, t: u8, enum_name: Option<&str>, depth: u32) -> Result<AttrValue> {
    if depth > 4 {
        return Err(r.error("attribute value nested too deep"));
    }
    Ok(match t {
        0x02 => AttrValue::Bool(r.u8()? != 0),
        0x03 => AttrValue::Int(i64::from(r.u16()?)),
        0x04 => AttrValue::Int(i64::from(r.u8()? as i8)),
        0x05 => AttrValue::Int(i64::from(r.u8()?)),
        0x06 => AttrValue::Int(i64::from(r.u16()? as i16)),
        0x07 => AttrValue::Int(i64::from(r.u16()?)),
        0x08 => AttrValue::Int(i64::from(r.u32()? as i32)),
        0x09 => AttrValue::Int(i64::from(r.u32()?)),
        0x0A => AttrValue::Int(r.u64()? as i64),
        0x0B => AttrValue::Int(r.u64()? as i64),
        0x0C => AttrValue::F32(f32::from_bits(r.u32()?)),
        0x0D => AttrValue::F64(f64::from_bits(r.u64()?)),
        0x0E | 0x50 => AttrValue::Str(ser_string(r)?),
        0x55 => AttrValue::Enum(enum_name.unwrap_or("").to_string(), r.u32()? as i32),
        // A boxed `object`: its own type first.
        0x51 => {
            let (t, e) = field_or_prop_type(r)?;
            attr_value(r, t.0, e.as_deref(), depth + 1)?
        }
        _ => return Err(r.error(format!("attribute value type {t:#x} not read"))),
    })
}

/// `FieldOrPropType`: the element type, an array's element type, an
/// enum's name.
#[allow(clippy::type_complexity)]
fn field_or_prop_type(r: &mut Reader) -> Result<((u8, Option<u8>), Option<String>)> {
    let t = r.u8()?;
    match t {
        0x1D => {
            let e = r.u8()?;
            let name = if e == 0x55 { ser_string(r)? } else { None };
            Ok(((t, Some(e)), name))
        }
        0x55 => Ok(((t, None), ser_string(r)?)),
        _ => Ok(((t, None), None)),
    }
}

impl<'a> Assembly<'a> {
    /// The named arguments (fields and properties) of custom attribute
    /// `index` (0-based, in [`Assembly::attributes`]' order), for an
    /// attribute made by a constructor without parameters. Kinds other
    /// than [`AttrValue`]'s are errors.
    pub fn attribute_named_args(&self, index: u32) -> Result<Vec<(String, AttrValue)>> {
        let row = index + 1;
        let (ctor_table, ctor) = self.tables().coded(Table::CUSTOM_ATTRIBUTE, row, 1)?;
        let m = self.method_token((u32::from(ctor_table) << 24) | ctor)?;
        if m.params != 0 {
            return Err(Error::new(
                0,
                format!("attribute {}: constructor arguments not read", m.type_name),
            ));
        }
        let blob = self.blob(self.tables().cell(Table::CUSTOM_ATTRIBUTE, row, 2)?)?;
        let mut r = Reader::new(blob);
        if r.u16()? != 1 {
            return Err(r.error("attribute blob without its prolog"));
        }
        let n = r.u16()?;
        let mut out = Vec::with_capacity(usize::from(n.min(64)));
        for _ in 0..n {
            let kind = r.u8()?;
            if kind != 0x53 && kind != 0x54 {
                return Err(r.error(format!("named argument kind {kind:#x}")));
            }
            let ((t, elem), enum_name) = field_or_prop_type(&mut r)?;
            let name = ser_string(&mut r)?.unwrap_or_default();
            let value = match (t, elem) {
                (0x1D, Some(e)) => {
                    let count = r.u32()?;
                    if count == u32::MAX {
                        AttrValue::Array(None)
                    } else if count > MAX_ATTR_ARRAY {
                        return Err(r.error(format!("attribute array of {count}")));
                    } else {
                        let mut v = Vec::with_capacity(count as usize);
                        for _ in 0..count {
                            v.push(attr_value(&mut r, e, enum_name.as_deref(), 0)?);
                        }
                        AttrValue::Array(Some(v))
                    }
                }
                _ => attr_value(&mut r, t, enum_name.as_deref(), 0)?,
            };
            out.push((name, value));
        }
        Ok(out)
    }

    pub fn type_flags(&self, type_row: u32) -> Result<u32> {
        self.tables().cell(Table::TYPE_DEF, type_row, 0)
    }

    /// The type a TypeDef is nested in.
    pub fn outer_type(&self, type_row: u32) -> Result<Option<u32>> {
        for r in 1..=self.rows(Table::NESTED_CLASS) {
            if self.tables().cell(Table::NESTED_CLASS, r, 0)? == type_row {
                return Ok(Some(self.tables().cell(Table::NESTED_CLASS, r, 1)?));
            }
        }
        Ok(None)
    }

    /// Every (nested type, enclosing type) pair of TypeDef rows.
    pub fn nested_pairs(&self) -> Result<Vec<(u32, u32)>> {
        (1..=self.rows(Table::NESTED_CLASS))
            .map(|r| {
                Ok((
                    self.tables().cell(Table::NESTED_CLASS, r, 0)?,
                    self.tables().cell(Table::NESTED_CLASS, r, 1)?,
                ))
            })
            .collect()
    }

    /// A TypeRef's scope.
    pub fn type_ref_scope(&self, row: u32) -> Result<RefScope> {
        let (table, scope) = self.tables().coded(Table::TYPE_REF, row, 0)?;
        Ok(match table {
            Table::ASSEMBLY_REF => RefScope::Assembly(self.assembly_ref_name(scope)?.into()),
            Table::TYPE_REF => RefScope::Outer(scope),
            Table::MODULE | Table::MODULE_REF => RefScope::Module,
            other => {
                return Err(Error::new(
                    0,
                    format!("TypeRef {row}: scope in table {other:#x}"),
                ));
            }
        })
    }

    /// The name of an AssemblyRef row (e.g. `UnityEngine.CoreModule`).
    pub fn assembly_ref_name(&self, row: u32) -> Result<&'a str> {
        self.string(self.tables().cell(Table::ASSEMBLY_REF, row, 6)?)
    }

    /// Every assembly this one references, by name.
    pub fn assembly_refs(&self) -> Result<Vec<&'a str>> {
        (1..=self.rows(Table::ASSEMBLY_REF))
            .map(|r| self.assembly_ref_name(r))
            .collect()
    }

    /// The assembly's own name (`None` for a module without an Assembly
    /// row).
    pub fn assembly_name(&self) -> Result<Option<&'a str>> {
        if self.rows(Table::ASSEMBLY) == 0 {
            return Ok(None);
        }
        self.string(self.tables().cell(Table::ASSEMBLY, 1, 7)?)
            .map(Some)
    }

    /// The interfaces a TypeDef declares it implements (InterfaceImpl
    /// rows, not those of its base types), as (namespace, name); a generic
    /// interface by its generic type's name.
    pub fn interfaces(&self, type_row: u32) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for r in 1..=self.rows(Table::INTERFACE_IMPL) {
            if self.tables().cell(Table::INTERFACE_IMPL, r, 0)? != type_row {
                continue;
            }
            let (table, row) = self.tables().coded(Table::INTERFACE_IMPL, r, 1)?;
            out.push(self.type_def_or_ref_name(table, row)?);
        }
        Ok(out)
    }

    /// Every custom attribute, with the names of its type (the type that
    /// declares its constructor).
    pub fn attributes(&self) -> Result<Vec<Attribute>> {
        let n = self.rows(Table::CUSTOM_ATTRIBUTE);
        let mut out = Vec::with_capacity(n as usize);
        for r in 1..=n {
            let (table, row) = self.tables().coded(Table::CUSTOM_ATTRIBUTE, r, 0)?;
            let (ctor_table, ctor) = self.tables().coded(Table::CUSTOM_ATTRIBUTE, r, 1)?;
            let token = (u32::from(ctor_table) << 24) | ctor;
            let m = self.method_token(token)?;
            out.push(Attribute {
                parent: TypeToken { table, row },
                namespace: m.namespace,
                name: m.type_name,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::Builder;

    #[test]
    fn reads_attribute_named_arguments() {
        let mut b = Builder::new();
        let attr = b.type_ref("UnityEngine.InputSystem.Layouts", "InputControlAttribute");
        let ctor = b.member_ref((Table::TYPE_REF, attr), ".ctor", Some(0));
        b.type_def("", "MouseState");
        let field = b.field(0x0001, "buttons") & 0x00FF_FFFF;
        // name = "leftButton", bit = 0u, displayName = "Left", aliases =
        // { "a", null }, noisy = true, defaultState = (object)2.5f.
        let mut v = vec![1, 0, 6, 0];
        let s = |v: &mut Vec<u8>, t: &str| {
            v.push(t.len() as u8);
            v.extend_from_slice(t.as_bytes());
        };
        v.extend([0x53, 0x0E]);
        s(&mut v, "name");
        s(&mut v, "leftButton");
        v.extend([0x53, 0x09]);
        s(&mut v, "bit");
        v.extend(0u32.to_le_bytes());
        v.extend([0x54, 0x0E]);
        s(&mut v, "displayName");
        s(&mut v, "Left");
        v.extend([0x53, 0x1D, 0x0E]);
        s(&mut v, "aliases");
        v.extend(2u32.to_le_bytes());
        s(&mut v, "a");
        v.push(0xFF);
        v.extend([0x53, 0x02]);
        s(&mut v, "noisy");
        v.push(1);
        v.extend([0x53, 0x51]);
        s(&mut v, "defaultState");
        v.push(0x0C);
        v.extend(2.5f32.to_le_bytes());
        b.attribute_with(
            (Table::FIELD, field),
            (Table::MEMBER_REF, ctor & 0x00FF_FFFF),
            &v,
        );
        let bytes = b.build();
        let asm = Assembly::parse(&bytes).unwrap();
        assert_eq!(asm.attributes().unwrap()[0].name, "InputControlAttribute");
        assert_eq!(
            asm.attribute_named_args(0).unwrap(),
            [
                (
                    "name".to_string(),
                    AttrValue::Str(Some("leftButton".into()))
                ),
                ("bit".to_string(), AttrValue::Int(0)),
                (
                    "displayName".to_string(),
                    AttrValue::Str(Some("Left".into()))
                ),
                (
                    "aliases".to_string(),
                    AttrValue::Array(Some(vec![
                        AttrValue::Str(Some("a".into())),
                        AttrValue::Str(None)
                    ]))
                ),
                ("noisy".to_string(), AttrValue::Bool(true)),
                ("defaultState".to_string(), AttrValue::F32(2.5)),
            ]
        );
        // Cut short anywhere: an error, never a panic.
        for n in 0..v.len() {
            let mut b2 = Builder::new();
            let attr = b2.type_ref("", "A");
            let ctor = b2.member_ref((Table::TYPE_REF, attr), ".ctor", Some(0));
            b2.type_def("", "T");
            let f = b2.field(0x0001, "f") & 0x00FF_FFFF;
            b2.attribute_with(
                (Table::FIELD, f),
                (Table::MEMBER_REF, ctor & 0x00FF_FFFF),
                &v[..n],
            );
            let bytes = b2.build();
            let asm = Assembly::parse(&bytes).unwrap();
            assert!(asm.attribute_named_args(0).is_err(), "{n}");
        }
    }

    #[test]
    fn reads_declared_interfaces() {
        let mut b = Builder::new();
        let element = b.type_ref("UnityEngine.UI", "ILayoutElement");
        let a = b.type_def("G", "A");
        let own = b.type_def("G", "IOwn");
        let c = b.type_def("G", "C");
        b.implements(a, (Table::TYPE_REF, element));
        b.implements(a, (Table::TYPE_DEF, own));
        let bytes = b.build();
        let asm = Assembly::parse(&bytes).unwrap();
        assert_eq!(
            asm.interfaces(a).unwrap(),
            [
                ("UnityEngine.UI".to_string(), "ILayoutElement".to_string()),
                ("G".to_string(), "IOwn".to_string())
            ]
        );
        assert!(asm.interfaces(c).unwrap().is_empty());
    }

    #[test]
    fn reads_flags_scopes_nesting_and_attributes() {
        let mut b = Builder::new();
        b.assembly("Game");
        let core = b.assembly_ref("UnityEngine.CoreModule");
        let mono = b.type_ref_in((Table::ASSEMBLY_REF, core), "UnityEngine", "MonoBehaviour");
        let inner_ref = b.type_ref_in((Table::TYPE_REF, mono), "", "Inner");
        let attr = b.type_ref_in((Table::ASSEMBLY_REF, core), "UnityEngine", "SerializeField");
        let ctor = b.member_ref((Table::TYPE_REF, attr), ".ctor", Some(0));
        let outer = b.type_def_with(
            TYPE_SERIALIZABLE,
            "G",
            "Outer",
            Some((Table::TYPE_REF, mono)),
        );
        let field = b.field(0x0001, "hidden") & 0x00FF_FFFF;
        let inner = b.type_def_with(TYPE_ABSTRACT, "", "Nested", None);
        b.nested(inner, outer);
        b.attribute(
            (Table::FIELD, field),
            (Table::MEMBER_REF, ctor & 0x00FF_FFFF),
        );
        let bytes = b.build();
        let asm = Assembly::parse(&bytes).unwrap();

        assert_eq!(asm.assembly_name().unwrap(), Some("Game"));
        assert_eq!(asm.assembly_refs().unwrap(), ["UnityEngine.CoreModule"]);
        assert_eq!(
            asm.type_ref_scope(mono).unwrap(),
            RefScope::Assembly("UnityEngine.CoreModule".into())
        );
        assert_eq!(
            asm.type_ref_scope(inner_ref).unwrap(),
            RefScope::Outer(mono)
        );
        assert_eq!(asm.type_flags(outer).unwrap(), TYPE_SERIALIZABLE);
        assert_eq!(asm.type_flags(inner).unwrap(), TYPE_ABSTRACT);
        assert_eq!(asm.outer_type(inner).unwrap(), Some(outer));
        assert_eq!(asm.outer_type(outer).unwrap(), None);
        assert_eq!(asm.nested_pairs().unwrap(), [(inner, outer)]);
        let base = asm.extends(outer).unwrap().unwrap();
        assert_eq!(base.to_string(), "UnityEngine.MonoBehaviour");
        assert_eq!(asm.extends(inner).unwrap(), None);
        let attrs = asm.attributes().unwrap();
        assert_eq!(
            attrs,
            [Attribute {
                parent: TypeToken {
                    table: Table::FIELD,
                    row: field
                },
                namespace: "UnityEngine".into(),
                name: "SerializeField".into(),
            }]
        );
    }
}
