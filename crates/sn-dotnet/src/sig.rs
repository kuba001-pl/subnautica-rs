//! Type signatures (ECMA-335 § II.23.2): the types of fields. M9u reads
//! them to know how Unity serializes a script's fields
//! (`docs/DESIGN.md` "M9u plan"); M9u0 prints them (`sn-inspect code
//! --fields`).

use std::fmt;

use crate::assembly::Assembly;
use crate::reader::Reader;
use crate::tables::{Table, TableId};
use crate::{Error, Result};

/// Signatures nest (arrays of generic instances of …); deeper than this
/// is not a type the game declares.
const MAX_DEPTH: usize = 32;

/// `FieldAttributes` (§ II.23.1.5).
pub const FIELD_ACCESS_MASK: u16 = 0x0007;
pub const FIELD_PUBLIC: u16 = 0x0006;
pub const FIELD_STATIC: u16 = 0x0010;
pub const FIELD_INIT_ONLY: u16 = 0x0020;
pub const FIELD_LITERAL: u16 = 0x0040;
pub const FIELD_NOT_SERIALIZED: u16 = 0x0080;

const ELEMENT_PTR: u8 = 0x0F;
const ELEMENT_BYREF: u8 = 0x10;
const ELEMENT_VALUETYPE: u8 = 0x11;
const ELEMENT_CLASS: u8 = 0x12;
const ELEMENT_VAR: u8 = 0x13;
const ELEMENT_ARRAY: u8 = 0x14;
const ELEMENT_GENERICINST: u8 = 0x15;
const ELEMENT_I: u8 = 0x18;
const ELEMENT_U: u8 = 0x19;
const ELEMENT_OBJECT: u8 = 0x1C;
const ELEMENT_SZARRAY: u8 = 0x1D;
const ELEMENT_MVAR: u8 = 0x1E;
const ELEMENT_CMOD_REQD: u8 = 0x1F;
const ELEMENT_CMOD_OPT: u8 = 0x20;
const FIELD_SIG: u8 = 0x06;

/// A TypeDef, TypeRef or TypeSpec row of an assembly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeToken {
    pub table: TableId,
    pub row: u32,
}

/// A type named in a signature: where it is defined or referenced, and
/// its names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedType {
    pub token: TypeToken,
    pub namespace: String,
    pub name: String,
}

impl fmt::Display for NamedType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.namespace.is_empty() {
            f.write_str(&self.name)
        } else {
            write!(f, "{}.{}", self.namespace, self.name)
        }
    }
}

/// A type as a signature names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SigType {
    /// An element type without operands: `bool` (0x02) … `string`
    /// (0x0E), `IntPtr` (0x18), `UIntPtr` (0x19), `object` (0x1C).
    Primitive(u8),
    /// A reference type (nested types are named by their own name; the
    /// token tells them apart).
    Class(NamedType),
    /// A value type (struct or enum).
    ValueType(NamedType),
    /// `T[]`.
    SzArray(Box<SigType>),
    /// `T[,]…` with its rank.
    Array(Box<SigType>, u32),
    /// `List<T>` and the like: the generic type and its arguments.
    Generic(Box<SigType>, Vec<SigType>),
    /// The declaring type's generic parameter N.
    Var(u32),
    /// The method's generic parameter N.
    MVar(u32),
    /// Pointers and by-refs, which Unity does not serialize.
    Unsupported(u8),
}

impl fmt::Display for SigType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SigType::Primitive(e) => f.write_str(primitive_name(*e)),
            SigType::Class(t) | SigType::ValueType(t) => write!(f, "{t}"),
            SigType::SzArray(t) => write!(f, "{t}[]"),
            SigType::Array(t, rank) => {
                write!(f, "{t}[{}]", ",".repeat(rank.saturating_sub(1) as usize))
            }
            SigType::Generic(base, args) => {
                write!(f, "{base}<")?;
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{a}")?;
                }
                f.write_str(">")
            }
            SigType::Var(n) => write!(f, "!{n}"),
            SigType::MVar(n) => write!(f, "!!{n}"),
            SigType::Unsupported(e) => write!(f, "[element {e:#04x}]"),
        }
    }
}

fn primitive_name(e: u8) -> &'static str {
    match e {
        0x01 => "void",
        0x02 => "bool",
        0x03 => "char",
        0x04 => "sbyte",
        0x05 => "byte",
        0x06 => "short",
        0x07 => "ushort",
        0x08 => "int",
        0x09 => "uint",
        0x0A => "long",
        0x0B => "ulong",
        0x0C => "float",
        0x0D => "double",
        0x0E => "string",
        ELEMENT_I => "IntPtr",
        ELEMENT_U => "UIntPtr",
        ELEMENT_OBJECT => "object",
        _ => "?",
    }
}

impl<'a> Assembly<'a> {
    /// The declared type of a Field row.
    pub fn field_type(&self, row: u32) -> Result<SigType> {
        let sig = self.blob(self.tables().cell(Table::FIELD, row, 2)?)?;
        let mut r = Reader::new(sig);
        let kind = r.u8()?;
        if kind != FIELD_SIG {
            return Err(r.error(format!("field {row}: signature kind {kind:#04x}")));
        }
        self.sig_type(&mut r, 0)
    }

    fn sig_type(&self, r: &mut Reader, depth: usize) -> Result<SigType> {
        if depth > MAX_DEPTH {
            return Err(r.error("type signature nested too deep"));
        }
        let e = r.u8()?;
        Ok(match e {
            0x01..=0x0E | ELEMENT_I | ELEMENT_U | ELEMENT_OBJECT => SigType::Primitive(e),
            ELEMENT_CMOD_REQD | ELEMENT_CMOD_OPT => {
                r.compressed()?; // the modifier's type: no effect on layout
                return self.sig_type(r, depth + 1);
            }
            ELEMENT_CLASS | ELEMENT_VALUETYPE => {
                let t = self.type_def_or_ref_coded(r)?;
                if e == ELEMENT_CLASS {
                    SigType::Class(t)
                } else {
                    SigType::ValueType(t)
                }
            }
            ELEMENT_SZARRAY => SigType::SzArray(Box::new(self.sig_type(r, depth + 1)?)),
            ELEMENT_ARRAY => {
                let t = self.sig_type(r, depth + 1)?;
                let rank = r.compressed()?;
                for _ in 0..r.compressed()? {
                    r.compressed()?; // sizes
                }
                for _ in 0..r.compressed()? {
                    r.compressed()?; // lower bounds
                }
                SigType::Array(Box::new(t), rank)
            }
            ELEMENT_GENERICINST => {
                let base = self.sig_type(r, depth + 1)?;
                if !matches!(base, SigType::Class(..) | SigType::ValueType(..)) {
                    return Err(r.error("generic instance of a non-type"));
                }
                let n = r.compressed()?;
                if n as usize > r.remaining() {
                    return Err(r.error(format!("{n} generic arguments")));
                }
                let mut args = Vec::with_capacity(n as usize);
                for _ in 0..n {
                    args.push(self.sig_type(r, depth + 1)?);
                }
                SigType::Generic(Box::new(base), args)
            }
            ELEMENT_VAR => SigType::Var(r.compressed()?),
            ELEMENT_MVAR => SigType::MVar(r.compressed()?),
            ELEMENT_PTR | ELEMENT_BYREF => {
                self.sig_type(r, depth + 1)?;
                SigType::Unsupported(e)
            }
            other => return Err(r.error(format!("element type {other:#04x}"))),
        })
    }

    /// The type a TypeDef extends: a class (TypeDef or TypeRef) or a
    /// generic instance (TypeSpec); `None` for none (interfaces,
    /// `System.Object`).
    pub fn extends(&self, type_row: u32) -> Result<Option<SigType>> {
        let (table, row) = self.tables().coded(Table::TYPE_DEF, type_row, 3)?;
        if row == 0 {
            return Ok(None);
        }
        self.type_of_token(TypeToken { table, row }).map(Some)
    }

    /// The type a TypeDef extends, by name (`extends`, generic instances
    /// named by their generic type).
    pub fn base_type(&self, type_row: u32) -> Result<Option<(String, String)>> {
        let (table, row) = self.tables().coded(Table::TYPE_DEF, type_row, 3)?;
        if row == 0 {
            return Ok(None);
        }
        self.type_def_or_ref_name(table, row).map(Some)
    }

    /// A type token as a signature type: TypeDef and TypeRef rows as
    /// classes (whether one is a value type depends on what it extends),
    /// TypeSpec rows decoded.
    pub fn type_of_token(&self, token: TypeToken) -> Result<SigType> {
        match token.table {
            Table::TYPE_SPEC => {
                let sig = self.blob(self.tables().cell(Table::TYPE_SPEC, token.row, 0)?)?;
                self.sig_type(&mut Reader::new(sig), 0)
            }
            Table::TYPE_DEF | Table::TYPE_REF => {
                let (namespace, name) = self.type_def_or_ref_name(token.table, token.row)?;
                Ok(SigType::Class(NamedType {
                    token,
                    namespace,
                    name,
                }))
            }
            other => Err(Error::new(0, format!("type token in table {other:#x}"))),
        }
    }

    /// A `TypeDefOrRefOrSpecEncoded` token (§ II.23.2.8).
    fn type_def_or_ref_coded(&self, r: &mut Reader) -> Result<NamedType> {
        let coded = r.compressed()?;
        let table = match coded & 0x3 {
            0 => Table::TYPE_DEF,
            1 => Table::TYPE_REF,
            2 => Table::TYPE_SPEC,
            _ => return Err(Error::new(r.pos(), "bad TypeDefOrRef tag")),
        };
        let row = coded >> 2;
        let (namespace, name) = self.type_def_or_ref_name(table, row)?;
        Ok(NamedType {
            token: TypeToken { table, row },
            namespace,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::Builder;

    fn field_types(sigs: &[&[u8]], build: impl FnOnce(&mut Builder)) -> Vec<Result<SigType>> {
        let mut b = Builder::new();
        build(&mut b);
        b.type_def("", "Holder");
        let rows: Vec<u32> = sigs
            .iter()
            .enumerate()
            .map(|(i, s)| b.field_with_sig(FIELD_PUBLIC, &format!("f{i}"), s) & 0x00FF_FFFF)
            .collect();
        let bytes = b.build();
        let asm = Assembly::parse(&bytes).unwrap();
        rows.iter().map(|r| asm.field_type(*r)).collect()
    }

    #[test]
    fn primitives_classes_arrays_generics() {
        let mut list = 0;
        let mut vec2 = 0;
        let types = field_types(
            &[
                &[0x06, 0x08],
                &[0x06, 0x0E],
                &[0x06, 0x1D, 0x0C],
                // The type refs are rows 1 and 2: coded (row << 2) | 1.
                &[0x06, 0x12, 0x05],
                &[0x06, 0x15, 0x12, 0x05, 0x01, 0x11, 0x09],
                &[0x06, 0x20, 0x05, 0x02],
                &[0x06, 0x14, 0x08, 0x02, 0x00, 0x00],
            ],
            |b| {
                list = b.type_ref("System.Collections.Generic", "List`1");
                vec2 = b.type_ref("UnityEngine", "Vector2");
            },
        );
        assert_eq!((list, vec2), (1, 2));
        let shown: Vec<String> = types.into_iter().map(|t| t.unwrap().to_string()).collect();
        assert_eq!(
            shown,
            [
                "int",
                "string",
                "float[]",
                "System.Collections.Generic.List`1",
                "System.Collections.Generic.List`1<UnityEngine.Vector2>",
                "bool",
                "int[,]",
            ]
        );
    }

    #[test]
    fn bad_signatures_are_errors() {
        let types = field_types(
            &[
                &[0x07, 0x08],             // not a field signature
                &[0x06],                   // no type
                &[0x06, 0x12],             // no token
                &[0x06, 0x12, 0x03],       // tag 3
                &[0x06, 0x15, 0x08, 0x00], // generic instance of int
                &[0x06, 0x15, 0x12, 0x05, 0x7F],
                &[0x06, 0x42],
            ],
            |b| {
                b.type_ref("A", "B");
            },
        );
        for t in types {
            assert!(t.is_err(), "{t:?}");
        }
    }

    #[test]
    fn deep_nesting_is_an_error() {
        let mut sig = vec![0x06];
        sig.extend(std::iter::repeat_n(0x1D, 100));
        sig.push(0x08);
        let types = field_types(&[&sig], |_| {});
        assert!(types[0].is_err());
    }
}
