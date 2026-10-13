//! Metadata M9u1 needs to know how Unity serializes a script's fields
//! (`docs/DESIGN.md` "M9u plan"): type flags, where a TypeRef points
//! (another assembly, an enclosing type), nested types, the assembly's
//! own name and the ones it references, custom attributes by type name.

use crate::assembly::Assembly;
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

impl<'a> Assembly<'a> {
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
