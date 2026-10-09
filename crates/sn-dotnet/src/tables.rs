//! The metadata tables (`#~` stream, ECMA-335 § II.22 and § II.24.2.6):
//! row counts, the width of every column (heap indexes and table indexes
//! grow from 2 to 4 bytes with the heap or table size), and raw cell reads.

use crate::reader::Reader;
use crate::{Error, Result};

pub type TableId = u8;

/// Table numbers (ECMA-335 § II.22).
pub struct Table;

impl Table {
    pub const MODULE: TableId = 0x00;
    pub const TYPE_REF: TableId = 0x01;
    pub const TYPE_DEF: TableId = 0x02;
    pub const FIELD_PTR: TableId = 0x03;
    pub const FIELD: TableId = 0x04;
    pub const METHOD_PTR: TableId = 0x05;
    pub const METHOD_DEF: TableId = 0x06;
    pub const PARAM_PTR: TableId = 0x07;
    pub const PARAM: TableId = 0x08;
    pub const INTERFACE_IMPL: TableId = 0x09;
    pub const MEMBER_REF: TableId = 0x0A;
    pub const CONSTANT: TableId = 0x0B;
    pub const CUSTOM_ATTRIBUTE: TableId = 0x0C;
    pub const FIELD_MARSHAL: TableId = 0x0D;
    pub const DECL_SECURITY: TableId = 0x0E;
    pub const CLASS_LAYOUT: TableId = 0x0F;
    pub const FIELD_LAYOUT: TableId = 0x10;
    pub const STAND_ALONE_SIG: TableId = 0x11;
    pub const EVENT_MAP: TableId = 0x12;
    pub const EVENT_PTR: TableId = 0x13;
    pub const EVENT: TableId = 0x14;
    pub const PROPERTY_MAP: TableId = 0x15;
    pub const PROPERTY_PTR: TableId = 0x16;
    pub const PROPERTY: TableId = 0x17;
    pub const METHOD_SEMANTICS: TableId = 0x18;
    pub const METHOD_IMPL: TableId = 0x19;
    pub const MODULE_REF: TableId = 0x1A;
    pub const TYPE_SPEC: TableId = 0x1B;
    pub const IMPL_MAP: TableId = 0x1C;
    pub const FIELD_RVA: TableId = 0x1D;
    pub const ENC_LOG: TableId = 0x1E;
    pub const ENC_MAP: TableId = 0x1F;
    pub const ASSEMBLY: TableId = 0x20;
    pub const ASSEMBLY_PROCESSOR: TableId = 0x21;
    pub const ASSEMBLY_OS: TableId = 0x22;
    pub const ASSEMBLY_REF: TableId = 0x23;
    pub const ASSEMBLY_REF_PROCESSOR: TableId = 0x24;
    pub const ASSEMBLY_REF_OS: TableId = 0x25;
    pub const FILE: TableId = 0x26;
    pub const EXPORTED_TYPE: TableId = 0x27;
    pub const MANIFEST_RESOURCE: TableId = 0x28;
    pub const NESTED_CLASS: TableId = 0x29;
    pub const GENERIC_PARAM: TableId = 0x2A;
    pub const METHOD_SPEC: TableId = 0x2B;
    pub const GENERIC_PARAM_CONSTRAINT: TableId = 0x2C;
    /// Number of tables this reader knows.
    pub const COUNT: usize = 0x2D;
}

/// A table slot no coded index may point to.
const NONE: TableId = 0xFF;

/// A coded index (§ II.24.2.6): a tag in the low bits picks the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Coded {
    pub bits: u32,
    pub tables: &'static [TableId],
}

impl Coded {
    /// (table, 1-based row; 0 = null) of a coded value.
    pub fn decode(&self, value: u32) -> Result<(TableId, u32)> {
        let tag = (value & ((1 << self.bits) - 1)) as usize;
        match self.tables.get(tag) {
            Some(&t) if t != NONE => Ok((t, value >> self.bits)),
            _ => Err(Error::new(
                0,
                format!("coded index {value:#x}: bad tag {tag}"),
            )),
        }
    }
}

pub(crate) const TYPE_DEF_OR_REF: Coded = Coded {
    bits: 2,
    tables: &[Table::TYPE_DEF, Table::TYPE_REF, Table::TYPE_SPEC],
};
pub(crate) const HAS_CONSTANT: Coded = Coded {
    bits: 2,
    tables: &[Table::FIELD, Table::PARAM, Table::PROPERTY],
};
const HAS_CUSTOM_ATTRIBUTE: Coded = Coded {
    bits: 5,
    tables: &[
        Table::METHOD_DEF,
        Table::FIELD,
        Table::TYPE_REF,
        Table::TYPE_DEF,
        Table::PARAM,
        Table::INTERFACE_IMPL,
        Table::MEMBER_REF,
        Table::MODULE,
        Table::DECL_SECURITY,
        Table::PROPERTY,
        Table::EVENT,
        Table::STAND_ALONE_SIG,
        Table::MODULE_REF,
        Table::TYPE_SPEC,
        Table::ASSEMBLY,
        Table::ASSEMBLY_REF,
        Table::FILE,
        Table::EXPORTED_TYPE,
        Table::MANIFEST_RESOURCE,
        Table::GENERIC_PARAM,
        Table::GENERIC_PARAM_CONSTRAINT,
        Table::METHOD_SPEC,
    ],
};
const HAS_FIELD_MARSHAL: Coded = Coded {
    bits: 1,
    tables: &[Table::FIELD, Table::PARAM],
};
const HAS_DECL_SECURITY: Coded = Coded {
    bits: 2,
    tables: &[Table::TYPE_DEF, Table::METHOD_DEF, Table::ASSEMBLY],
};
pub(crate) const MEMBER_REF_PARENT: Coded = Coded {
    bits: 3,
    tables: &[
        Table::TYPE_DEF,
        Table::TYPE_REF,
        Table::MODULE_REF,
        Table::METHOD_DEF,
        Table::TYPE_SPEC,
    ],
};
const HAS_SEMANTICS: Coded = Coded {
    bits: 1,
    tables: &[Table::EVENT, Table::PROPERTY],
};
const METHOD_DEF_OR_REF: Coded = Coded {
    bits: 1,
    tables: &[Table::METHOD_DEF, Table::MEMBER_REF],
};
const MEMBER_FORWARDED: Coded = Coded {
    bits: 1,
    tables: &[Table::FIELD, Table::METHOD_DEF],
};
const IMPLEMENTATION: Coded = Coded {
    bits: 2,
    tables: &[Table::FILE, Table::ASSEMBLY_REF, Table::EXPORTED_TYPE],
};
const CUSTOM_ATTRIBUTE_TYPE: Coded = Coded {
    bits: 3,
    tables: &[NONE, NONE, Table::METHOD_DEF, Table::MEMBER_REF, NONE],
};
const RESOLUTION_SCOPE: Coded = Coded {
    bits: 2,
    tables: &[
        Table::MODULE,
        Table::MODULE_REF,
        Table::ASSEMBLY_REF,
        Table::TYPE_REF,
    ],
};
const TYPE_OR_METHOD_DEF: Coded = Coded {
    bits: 1,
    tables: &[Table::TYPE_DEF, Table::METHOD_DEF],
};

/// A column's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Col {
    U16,
    U32,
    Str,
    Guid,
    Blob,
    /// Index into one table.
    Idx(TableId),
    Coded(Coded),
}

use Col::*;

/// Columns of every table, in order (§ II.22). `Constant.Type` is a byte
/// followed by a padding byte: read as `U16`.
pub(crate) const SCHEMA: [&[Col]; Table::COUNT] = [
    /* Module */ &[U16, Str, Guid, Guid, Guid],
    /* TypeRef */ &[Coded(RESOLUTION_SCOPE), Str, Str],
    /* TypeDef */
    &[
        U32,
        Str,
        Str,
        Coded(TYPE_DEF_OR_REF),
        Idx(Table::FIELD),
        Idx(Table::METHOD_DEF),
    ],
    /* FieldPtr */ &[Idx(Table::FIELD)],
    /* Field */ &[U16, Str, Blob],
    /* MethodPtr */ &[Idx(Table::METHOD_DEF)],
    /* MethodDef */ &[U32, U16, U16, Str, Blob, Idx(Table::PARAM)],
    /* ParamPtr */ &[Idx(Table::PARAM)],
    /* Param */ &[U16, U16, Str],
    /* InterfaceImpl */ &[Idx(Table::TYPE_DEF), Coded(TYPE_DEF_OR_REF)],
    /* MemberRef */ &[Coded(MEMBER_REF_PARENT), Str, Blob],
    /* Constant */ &[U16, Coded(HAS_CONSTANT), Blob],
    /* CustomAttribute */
    &[
        Coded(HAS_CUSTOM_ATTRIBUTE),
        Coded(CUSTOM_ATTRIBUTE_TYPE),
        Blob,
    ],
    /* FieldMarshal */ &[Coded(HAS_FIELD_MARSHAL), Blob],
    /* DeclSecurity */ &[U16, Coded(HAS_DECL_SECURITY), Blob],
    /* ClassLayout */ &[U16, U32, Idx(Table::TYPE_DEF)],
    /* FieldLayout */ &[U32, Idx(Table::FIELD)],
    /* StandAloneSig */ &[Blob],
    /* EventMap */ &[Idx(Table::TYPE_DEF), Idx(Table::EVENT)],
    /* EventPtr */ &[Idx(Table::EVENT)],
    /* Event */ &[U16, Str, Coded(TYPE_DEF_OR_REF)],
    /* PropertyMap */ &[Idx(Table::TYPE_DEF), Idx(Table::PROPERTY)],
    /* PropertyPtr */ &[Idx(Table::PROPERTY)],
    /* Property */ &[U16, Str, Blob],
    /* MethodSemantics */ &[U16, Idx(Table::METHOD_DEF), Coded(HAS_SEMANTICS)],
    /* MethodImpl */
    &[
        Idx(Table::TYPE_DEF),
        Coded(METHOD_DEF_OR_REF),
        Coded(METHOD_DEF_OR_REF),
    ],
    /* ModuleRef */ &[Str],
    /* TypeSpec */ &[Blob],
    /* ImplMap */ &[U16, Coded(MEMBER_FORWARDED), Str, Idx(Table::MODULE_REF)],
    /* FieldRVA */ &[U32, Idx(Table::FIELD)],
    /* EncLog */ &[U32, U32],
    /* EncMap */ &[U32],
    /* Assembly */ &[U32, U16, U16, U16, U16, U32, Blob, Str, Str],
    /* AssemblyProcessor */ &[U32],
    /* AssemblyOS */ &[U32, U32, U32],
    /* AssemblyRef */ &[U16, U16, U16, U16, U32, Blob, Str, Str, Blob],
    /* AssemblyRefProcessor */ &[U32, Idx(Table::ASSEMBLY_REF)],
    /* AssemblyRefOS */ &[U32, U32, U32, Idx(Table::ASSEMBLY_REF)],
    /* File */ &[U32, Str, Blob],
    /* ExportedType */ &[U32, U32, Str, Str, Coded(IMPLEMENTATION)],
    /* ManifestResource */ &[U32, U32, Str, Coded(IMPLEMENTATION)],
    /* NestedClass */ &[Idx(Table::TYPE_DEF), Idx(Table::TYPE_DEF)],
    /* GenericParam */ &[U16, U16, Coded(TYPE_OR_METHOD_DEF), Str],
    /* MethodSpec */ &[Coded(METHOD_DEF_OR_REF), Blob],
    /* GenericParamConstraint */ &[Idx(Table::GENERIC_PARAM), Coded(TYPE_DEF_OR_REF)],
];

/// Heap-size flags of the `#~` header.
pub(crate) const HEAP_STRINGS_WIDE: u8 = 0x01;
pub(crate) const HEAP_GUID_WIDE: u8 = 0x02;
pub(crate) const HEAP_BLOB_WIDE: u8 = 0x04;

/// Byte width of a column, given the row counts and heap flags.
pub(crate) fn col_width(col: Col, rows: &[u32; Table::COUNT], heaps: u8) -> usize {
    let wide = |flag| if heaps & flag != 0 { 4 } else { 2 };
    match col {
        U16 => 2,
        U32 => 4,
        Str => wide(HEAP_STRINGS_WIDE),
        Guid => wide(HEAP_GUID_WIDE),
        Blob => wide(HEAP_BLOB_WIDE),
        Idx(t) => {
            if rows[t as usize] > 0xFFFF {
                4
            } else {
                2
            }
        }
        Coded(c) => {
            let max = c
                .tables
                .iter()
                .filter(|&&t| t != NONE)
                .map(|&t| rows[t as usize])
                .max()
                .unwrap_or(0);
            if max < (1 << (16 - c.bits)) { 2 } else { 4 }
        }
    }
}

/// Where each table's rows are in the `#~` stream.
#[derive(Clone, Debug)]
pub(crate) struct Tables<'a> {
    pub rows: [u32; Table::COUNT],
    /// The stream's bytes.
    data: &'a [u8],
    /// Offset of the stream in the file (for errors).
    base: usize,
    start: [usize; Table::COUNT],
    row_size: [usize; Table::COUNT],
    /// (offset in row, width) per column.
    cols: Vec<Vec<(usize, usize)>>,
}

impl<'a> Tables<'a> {
    pub fn parse(data: &'a [u8], base: usize) -> Result<Tables<'a>> {
        let mut r = Reader::at(data, base);
        r.u32()?; // reserved
        let major = r.u8()?;
        r.u8()?; // minor
        let heaps = r.u8()?;
        r.u8()?; // reserved
        let valid = r.u64()?;
        r.u64()?; // sorted
        if major != 2 {
            return Err(r.error(format!("table stream version {major}, expected 2")));
        }
        if valid >> Table::COUNT != 0 {
            return Err(r.error(format!(
                "tables present beyond {:#x} (valid mask {valid:#x})",
                Table::COUNT - 1
            )));
        }
        // Tables only uncompressed (`#-`) metadata or edit-and-continue use.
        for t in [
            Table::FIELD_PTR,
            Table::METHOD_PTR,
            Table::PARAM_PTR,
            Table::EVENT_PTR,
            Table::PROPERTY_PTR,
        ] {
            if valid & (1 << t) != 0 {
                return Err(r.error(format!("indirection table {t:#x} is not supported")));
            }
        }
        if heaps & 0x40 != 0 {
            r.u32()?; // extra data (seen in some obfuscated files)
        }
        let mut rows = [0u32; Table::COUNT];
        for (t, row) in rows.iter_mut().enumerate() {
            if valid & (1 << t) != 0 {
                *row = r.u32()?;
                if *row > 0x00FF_FFFF {
                    return Err(r.error(format!("table {t:#x}: {row} rows")));
                }
            }
        }
        let mut start = [0usize; Table::COUNT];
        let mut row_size = [0usize; Table::COUNT];
        let mut cols = Vec::with_capacity(Table::COUNT);
        let mut at = r.pos();
        for t in 0..Table::COUNT {
            let mut offset = 0;
            let mut c = Vec::with_capacity(SCHEMA[t].len());
            for &col in SCHEMA[t] {
                let w = col_width(col, &rows, heaps);
                c.push((offset, w));
                offset += w;
            }
            start[t] = at;
            row_size[t] = offset;
            cols.push(c);
            at = offset
                .checked_mul(rows[t] as usize)
                .and_then(|n| n.checked_add(at))
                .ok_or_else(|| r.error("table sizes overflow"))?;
        }
        if at > data.len() {
            return Err(Error::new(
                base + data.len(),
                format!("tables need {at} bytes, the stream has {}", data.len()),
            ));
        }
        Ok(Tables {
            rows,
            data,
            base,
            start,
            row_size,
            cols,
        })
    }

    /// Raw value of column `col` of 1-based `row` of `table`.
    pub fn cell(&self, table: TableId, row: u32, col: usize) -> Result<u32> {
        let t = table as usize;
        if t >= Table::COUNT || row == 0 || row > self.rows[t] {
            return Err(Error::new(
                self.base,
                format!(
                    "table {table:#x}: no row {row} ({} rows)",
                    self.rows.get(t).unwrap_or(&0)
                ),
            ));
        }
        let &(offset, width) = self.cols[t]
            .get(col)
            .ok_or_else(|| Error::new(self.base, format!("table {table:#x}: no column {col}")))?;
        let at = self.start[t] + (row as usize - 1) * self.row_size[t] + offset;
        let b = &self.data[at..at + width];
        Ok(match width {
            2 => u32::from(u16::from_le_bytes([b[0], b[1]])),
            _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        })
    }

    /// A coded column decoded to (table, row).
    pub fn coded(&self, table: TableId, row: u32, col: usize) -> Result<(TableId, u32)> {
        let value = self.cell(table, row, col)?;
        let Col::Coded(c) = SCHEMA[table as usize][col] else {
            return Err(Error::new(
                self.base,
                format!("table {table:#x} column {col} is not coded"),
            ));
        };
        c.decode(value).map_err(|e| {
            Error::new(
                self.base,
                format!("table {table:#x} row {row}: {}", e.message),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_grow_with_rows_and_heaps() {
        let mut rows = [0u32; Table::COUNT];
        assert_eq!(col_width(Str, &rows, 0), 2);
        assert_eq!(col_width(Str, &rows, HEAP_STRINGS_WIDE), 4);
        assert_eq!(col_width(Blob, &rows, HEAP_STRINGS_WIDE), 2);
        rows[Table::FIELD as usize] = 0x1_0000;
        assert_eq!(col_width(Idx(Table::FIELD), &rows, 0), 4);
        assert_eq!(col_width(Idx(Table::PARAM), &rows, 0), 2);
        // HasConstant has 2 tag bits: 2^14 rows make it wide.
        rows[Table::FIELD as usize] = (1 << 14) - 1;
        assert_eq!(col_width(Coded(HAS_CONSTANT), &rows, 0), 2);
        rows[Table::FIELD as usize] = 1 << 14;
        assert_eq!(col_width(Coded(HAS_CONSTANT), &rows, 0), 4);
    }

    #[test]
    fn coded_index_decodes() {
        assert_eq!(
            MEMBER_REF_PARENT.decode((7 << 3) | 1).unwrap(),
            (Table::TYPE_REF, 7)
        );
        assert!(MEMBER_REF_PARENT.decode(5).is_err());
        assert!(CUSTOM_ATTRIBUTE_TYPE.decode(0).is_err());
    }
}
