//! The PE container (ECMA-335 § II.25), the CLI header, the metadata root
//! and its streams (§ II.24), and typed reads of the rows we use.

use std::ops::Range;

use crate::reader::Reader;
use crate::tables::{Table, TableId, Tables};
use crate::{Error, Result};

/// PE32 / PE32+ optional header magics.
const PE32: u16 = 0x10B;
const PE32_PLUS: u16 = 0x20B;
/// Data directory of the CLI header.
const CLI_DIRECTORY: usize = 14;
const METADATA_SIGNATURE: u32 = 0x424A_5342;

/// `FieldAttributes`.
pub(crate) const FIELD_STATIC: u16 = 0x0010;
pub(crate) const FIELD_LITERAL: u16 = 0x0040;

#[derive(Clone, Copy, Debug)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_pointer: u32,
    raw_size: u32,
}

struct Sections(Vec<Section>);

impl Sections {
    /// File offset of a relative virtual address.
    fn offset(&self, rva: u32) -> Result<usize> {
        for s in &self.0 {
            let size = s.virtual_size.max(s.raw_size);
            if rva >= s.virtual_address && rva - s.virtual_address < size {
                let delta = rva - s.virtual_address;
                if delta >= s.raw_size {
                    break;
                }
                return Ok(s.raw_pointer as usize + delta as usize);
            }
        }
        Err(Error::new(0, format!("RVA {rva:#x} is in no section")))
    }

    fn slice<'a>(&self, data: &'a [u8], rva: u32, len: usize) -> Result<&'a [u8]> {
        let start = self.offset(rva)?;
        start
            .checked_add(len)
            .and_then(|end| data.get(start..end))
            .ok_or_else(|| Error::new(start, format!("{len} bytes at RVA {rva:#x} past the end")))
    }
}

/// A method or field reference resolved to names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRefInfo {
    /// The declaring type's namespace and name (`""` namespace for none).
    pub namespace: String,
    pub type_name: String,
    pub name: String,
    /// From the method signature: parameter count (0 for fields).
    pub params: u32,
}

/// A parsed .NET assembly. Borrows the file's bytes.
pub struct Assembly<'a> {
    data: &'a [u8],
    sections: Sections,
    strings: &'a [u8],
    user_strings: &'a [u8],
    blobs: &'a [u8],
    tables: Tables<'a>,
    /// Runtime version string of the metadata root (e.g. `v4.0.30319`).
    pub runtime_version: String,
}

impl<'a> Assembly<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Assembly<'a>> {
        let mut r = Reader::new(data);
        if r.bytes(2)? != b"MZ" {
            return Err(Error::new(0, "not a PE file (no MZ signature)"));
        }
        r.seek(0x3C)?;
        let pe = r.u32()? as usize;
        r.seek(pe)?;
        if r.bytes(4)? != b"PE\0\0" {
            return Err(r.error("no PE signature"));
        }
        r.u16()?; // machine
        let section_count = r.u16()? as usize;
        r.bytes(12)?; // time stamp, symbol table, symbol count
        let optional_size = r.u16()? as usize;
        r.u16()?; // characteristics
        let optional = r.pos();
        let magic = r.u16()?;
        let (count_at, dirs_at) = match magic {
            PE32 => (92, 96),
            PE32_PLUS => (108, 112),
            other => return Err(r.error(format!("unknown optional header magic {other:#x}"))),
        };
        r.seek(optional + count_at)?;
        let dir_count = r.u32()? as usize;
        if dir_count <= CLI_DIRECTORY || dirs_at + dir_count * 8 > optional_size {
            return Err(r.error(format!("{dir_count} data directories: no CLI header")));
        }
        r.seek(optional + dirs_at + CLI_DIRECTORY * 8)?;
        let cli_rva = r.u32()?;
        let cli_size = r.u32()?;
        if cli_rva == 0 || cli_size < 16 {
            return Err(r.error("no CLI header (not a .NET assembly)"));
        }

        r.seek(optional + optional_size)?;
        let mut sections = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            r.bytes(8)?; // name
            let virtual_size = r.u32()?;
            let virtual_address = r.u32()?;
            let raw_size = r.u32()?;
            let raw_pointer = r.u32()?;
            r.bytes(16)?;
            sections.push(Section {
                virtual_address,
                virtual_size,
                raw_pointer,
                raw_size,
            });
        }
        let sections = Sections(sections);
        let cli = sections.slice(data, cli_rva, cli_size as usize)?;
        let mut c = Reader::at(cli, sections.offset(cli_rva)?);
        c.u32()?; // cb
        c.u32()?; // runtime version
        let meta_rva = c.u32()?;
        let meta_size = c.u32()? as usize;
        let meta_base = sections.offset(meta_rva)?;
        let meta = sections.slice(data, meta_rva, meta_size)?;

        let mut m = Reader::at(meta, meta_base);
        if m.u32()? != METADATA_SIGNATURE {
            return Err(m.error("bad metadata signature"));
        }
        m.u16()?;
        m.u16()?;
        m.u32()?;
        let version_len = m.u32()? as usize;
        let version = m.bytes(version_len)?;
        let end = version
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(version.len());
        let runtime_version = String::from_utf8_lossy(&version[..end]).into_owned();
        m.u16()?; // flags
        let stream_count = m.u16()?;
        let (mut tables, mut strings, mut user_strings, mut blobs) =
            (None, &[][..], &[][..], &[][..]);
        for _ in 0..stream_count {
            let offset = m.u32()? as usize;
            let size = m.u32()? as usize;
            let name = m.cstr(32)?;
            m.align(4)?;
            let bytes = offset
                .checked_add(size)
                .and_then(|end| meta.get(offset..end))
                .ok_or_else(|| m.error(format!("stream {name} outside the metadata")))?;
            match name {
                "#~" => tables = Some((bytes, meta_base + offset)),
                "#-" => return Err(m.error("uncompressed metadata (#-) is not supported")),
                "#Strings" => strings = bytes,
                "#US" => user_strings = bytes,
                "#Blob" => blobs = bytes,
                _ => {}
            }
        }
        let (bytes, base) = tables.ok_or_else(|| m.error("no #~ stream"))?;
        Ok(Assembly {
            data,
            sections,
            strings,
            user_strings,
            blobs,
            tables: Tables::parse(bytes, base)?,
            runtime_version,
        })
    }

    /// Bytes from an RVA to the end of the file, and their file offset.
    fn rva_rest(&self, rva: u32) -> Result<(&'a [u8], usize)> {
        let start = self.sections.offset(rva)?;
        let rest = self
            .data
            .get(start..)
            .ok_or_else(|| Error::new(start, "RVA past the end"))?;
        Ok((rest, start))
    }

    pub fn rows(&self, table: TableId) -> u32 {
        self.tables.rows.get(table as usize).copied().unwrap_or(0)
    }

    /// A string from the `#Strings` heap.
    pub fn string(&self, index: u32) -> Result<&'a str> {
        let rest = self
            .strings
            .get(index as usize..)
            .ok_or_else(|| Error::new(0, format!("string index {index} out of the heap")))?;
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::new(0, format!("string {index} is unterminated")))?;
        std::str::from_utf8(&rest[..len])
            .map_err(|_| Error::new(0, format!("string {index} is not UTF-8")))
    }

    /// A blob from the `#Blob` heap.
    pub fn blob(&self, index: u32) -> Result<&'a [u8]> {
        let rest = self
            .blobs
            .get(index as usize..)
            .ok_or_else(|| Error::new(0, format!("blob index {index} out of the heap")))?;
        let mut r = Reader::new(rest);
        let len = r.compressed()? as usize;
        r.bytes(len)
            .map_err(|e| Error::new(0, format!("blob {index}: {}", e.message)))
    }

    /// A string literal (`ldstr` token: table 0x70, `#US` offset).
    pub fn user_string(&self, token: u32) -> Result<String> {
        if token >> 24 != 0x70 {
            return Err(Error::new(0, format!("token {token:#x} is not a string")));
        }
        let index = (token & 0x00FF_FFFF) as usize;
        let rest = self
            .user_strings
            .get(index..)
            .ok_or_else(|| Error::new(0, format!("user string {index} out of the heap")))?;
        let mut r = Reader::new(rest);
        let len = r.compressed()? as usize;
        let bytes = r.bytes(len)?;
        // UTF-16LE, then one flag byte.
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units)
            .map_err(|_| Error::new(0, format!("user string {index} is not UTF-16")))
    }

    // --- TypeDef -------------------------------------------------------

    pub fn type_name(&self, row: u32) -> Result<(&'a str, &'a str)> {
        Ok((
            self.string(self.tables.cell(Table::TYPE_DEF, row, 2)?)?,
            self.string(self.tables.cell(Table::TYPE_DEF, row, 1)?)?,
        ))
    }

    /// Rows of `table` from column `col` of TypeDef `row` up to the next
    /// type's (the field and method lists).
    fn member_range(&self, row: u32, col: usize, table: TableId) -> Result<Range<u32>> {
        let start = self.tables.cell(Table::TYPE_DEF, row, col)?;
        let end = if row < self.rows(Table::TYPE_DEF) {
            self.tables.cell(Table::TYPE_DEF, row + 1, col)?
        } else {
            self.rows(table) + 1
        };
        if start == 0 || start > end || end > self.rows(table) + 1 {
            return Err(Error::new(
                0,
                format!("type {row}: member list {start}..{end} is out of order"),
            ));
        }
        Ok(start..end)
    }

    /// The Field rows of a type.
    pub fn fields(&self, type_row: u32) -> Result<Range<u32>> {
        self.member_range(type_row, 4, Table::FIELD)
    }

    /// The MethodDef rows of a type.
    pub fn methods(&self, type_row: u32) -> Result<Range<u32>> {
        self.member_range(type_row, 5, Table::METHOD_DEF)
    }

    /// TypeDef rows that are nested in another type.
    fn nested(&self) -> Result<Vec<u32>> {
        (1..=self.rows(Table::NESTED_CLASS))
            .map(|r| self.tables.cell(Table::NESTED_CLASS, r, 0))
            .collect()
    }

    /// A top-level (not nested) type by namespace and name.
    pub fn find_type(&self, namespace: &str, name: &str) -> Result<Option<u32>> {
        let nested = self.nested()?;
        for row in 1..=self.rows(Table::TYPE_DEF) {
            if self.type_name(row)? == (namespace, name) && !nested.contains(&row) {
                return Ok(Some(row));
            }
        }
        Ok(None)
    }

    /// The type whose member list (column `col`) holds `member`.
    fn owner(&self, member: u32, col: usize, table: TableId) -> Result<u32> {
        let n = self.rows(Table::TYPE_DEF);
        // Member lists are non-decreasing: binary search for the last type
        // whose list starts at or before `member`.
        let (mut lo, mut hi) = (1u32, n);
        let mut found = None;
        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            if self.tables.cell(Table::TYPE_DEF, mid, col)? <= member {
                found = Some(mid);
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        let row =
            found.ok_or_else(|| Error::new(0, format!("{table:#x} row {member} has no owner")))?;
        Ok(row)
    }

    // --- Field / MethodDef ---------------------------------------------

    pub fn field_flags(&self, row: u32) -> Result<u16> {
        Ok(self.tables.cell(Table::FIELD, row, 0)? as u16)
    }

    pub fn field_name(&self, row: u32) -> Result<&'a str> {
        self.string(self.tables.cell(Table::FIELD, row, 1)?)
    }

    pub fn method_name(&self, row: u32) -> Result<&'a str> {
        self.string(self.tables.cell(Table::METHOD_DEF, row, 3)?)
    }

    /// A method of a type by name (the first one).
    pub fn find_method(&self, type_row: u32, name: &str) -> Result<Option<u32>> {
        for m in self.methods(type_row)? {
            if self.method_name(m)? == name {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// Parameter count of a method signature blob.
    fn sig_params(&self, blob: u32) -> Result<u32> {
        let sig = self.blob(blob)?;
        let mut r = Reader::new(sig);
        let conv = r.u8()?;
        if conv & 0x10 != 0 {
            r.compressed()?; // generic parameter count
        }
        r.compressed()
    }

    /// The IL of a method (without its header and exception sections);
    /// `None` for methods without a body (abstract, extern).
    pub fn method_body(&self, row: u32) -> Result<Option<(&'a [u8], usize)>> {
        let rva = self.tables.cell(Table::METHOD_DEF, row, 0)?;
        if rva == 0 {
            return Ok(None);
        }
        let (rest, base) = self.rva_rest(rva)?;
        let mut r = Reader::at(rest, base);
        let first = r.u8()?;
        let (header, size) = match first & 0x3 {
            0x2 => (1, (first >> 2) as usize),
            0x3 => {
                let second = r.u8()?;
                let header = ((second >> 4) as usize) * 4;
                r.u16()?; // max stack
                let size = r.u32()? as usize;
                if header < 12 {
                    return Err(r.error(format!("fat method header of {header} bytes")));
                }
                (header, size)
            }
            _ => return Err(r.error(format!("method {row}: bad body header {first:#04x}"))),
        };
        r.seek(header)?;
        let code = r.bytes(size)?;
        Ok(Some((code, base + header)))
    }

    // --- Tokens --------------------------------------------------------

    fn type_def_or_ref_name(&self, table: TableId, row: u32) -> Result<(String, String)> {
        match table {
            Table::TYPE_DEF => {
                let (ns, n) = self.type_name(row)?;
                Ok((ns.into(), n.into()))
            }
            Table::TYPE_REF => Ok((
                self.string(self.tables.cell(Table::TYPE_REF, row, 2)?)?
                    .into(),
                self.string(self.tables.cell(Table::TYPE_REF, row, 1)?)?
                    .into(),
            )),
            Table::TYPE_SPEC => self.type_spec_name(row),
            other => Err(Error::new(
                0,
                format!("type in table {other:#x} is not supported"),
            )),
        }
    }

    /// A TypeSpec's name: a generic instance (`List<T>`) is named by its
    /// generic type (`List`1`); any other spec (arrays, pointers, …) gets
    /// `[spec N]`, which no type name equals.
    fn type_spec_name(&self, row: u32) -> Result<(String, String)> {
        const ELEMENT_GENERICINST: u8 = 0x15;
        const ELEMENT_CLASS: u8 = 0x12;
        const ELEMENT_VALUETYPE: u8 = 0x11;
        let sig = self.blob(self.tables.cell(Table::TYPE_SPEC, row, 0)?)?;
        let mut r = Reader::new(sig);
        if r.u8()? == ELEMENT_GENERICINST && matches!(r.u8()?, ELEMENT_CLASS | ELEMENT_VALUETYPE) {
            let coded = r.compressed()?;
            let table = match coded & 0x3 {
                0 => Table::TYPE_DEF,
                1 => Table::TYPE_REF,
                _ => return Err(r.error("generic instance of a TypeSpec")),
            };
            return self.type_def_or_ref_name(table, coded >> 2);
        }
        Ok((String::new(), format!("[spec {row}]")))
    }

    /// The type named by a token (`newarr`, …): TypeDef, TypeRef or TypeSpec.
    pub fn type_token(&self, token: u32) -> Result<(String, String)> {
        self.type_def_or_ref_name((token >> 24) as u8, token & 0x00FF_FFFF)
    }

    /// A method token (`call`, `newobj`): MethodDef or MemberRef.
    pub fn method_token(&self, token: u32) -> Result<MemberRefInfo> {
        let row = token & 0x00FF_FFFF;
        match (token >> 24) as u8 {
            Table::METHOD_DEF => {
                let owner = self.owner(row, 5, Table::METHOD_DEF)?;
                let (namespace, type_name) = self.type_name(owner)?;
                Ok(MemberRefInfo {
                    namespace: namespace.into(),
                    type_name: type_name.into(),
                    name: self.method_name(row)?.into(),
                    params: self.sig_params(self.tables.cell(Table::METHOD_DEF, row, 4)?)?,
                })
            }
            Table::MEMBER_REF => self.member_ref(row, true),
            other => Err(Error::new(
                0,
                format!("method token {token:#x} in table {other:#x} is not supported"),
            )),
        }
    }

    /// A field token (`ldsfld`, `stsfld`): Field or MemberRef.
    pub fn field_token(&self, token: u32) -> Result<MemberRefInfo> {
        let row = token & 0x00FF_FFFF;
        match (token >> 24) as u8 {
            Table::FIELD => {
                let owner = self.owner(row, 4, Table::FIELD)?;
                let (namespace, type_name) = self.type_name(owner)?;
                Ok(MemberRefInfo {
                    namespace: namespace.into(),
                    type_name: type_name.into(),
                    name: self.field_name(row)?.into(),
                    params: 0,
                })
            }
            Table::MEMBER_REF => self.member_ref(row, false),
            other => Err(Error::new(
                0,
                format!("field token {token:#x} in table {other:#x} is not supported"),
            )),
        }
    }

    fn member_ref(&self, row: u32, method: bool) -> Result<MemberRefInfo> {
        let (table, parent) = self.tables.coded(Table::MEMBER_REF, row, 0)?;
        let (namespace, type_name) = self.type_def_or_ref_name(table, parent)?;
        let sig = self.tables.cell(Table::MEMBER_REF, row, 2)?;
        Ok(MemberRefInfo {
            namespace,
            type_name,
            name: self
                .string(self.tables.cell(Table::MEMBER_REF, row, 1)?)?
                .into(),
            params: if method { self.sig_params(sig)? } else { 0 },
        })
    }

    // --- Constants -----------------------------------------------------

    /// Field row → (element type, value bytes) for every field constant.
    pub fn field_constants(&self) -> Result<std::collections::HashMap<u32, (u8, &'a [u8])>> {
        let mut out = std::collections::HashMap::new();
        for row in 1..=self.rows(Table::CONSTANT) {
            let ty = self.tables.cell(Table::CONSTANT, row, 0)? as u8;
            let (table, parent) = self.tables.coded(Table::CONSTANT, row, 1)?;
            if table == Table::FIELD {
                out.insert(
                    parent,
                    (ty, self.blob(self.tables.cell(Table::CONSTANT, row, 2)?)?),
                );
            }
        }
        Ok(out)
    }
}
