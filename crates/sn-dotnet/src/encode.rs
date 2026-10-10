//! Test-only writer of small .NET assemblies: a PE file with one section
//! holding the CLI header, method bodies and compressed metadata (`#~`,
//! `#Strings`, `#US`, `#Blob`). Lets the readers be tested on synthetic
//! bytes we encode ourselves, never on files from the game.

use crate::tables::{
    Coded, HAS_CONSTANT, HEAP_BLOB_WIDE, HEAP_GUID_WIDE, HEAP_STRINGS_WIDE, MEMBER_REF_PARENT,
    SCHEMA, TYPE_DEF_OR_REF, Table, TableId, col_width,
};

const SECTION_RVA: u32 = 0x2000;
const SECTION_FILE: usize = 0x200;
const CLI_HEADER: usize = 72;
const HEAPS: u8 = HEAP_STRINGS_WIDE | HEAP_GUID_WIDE | HEAP_BLOB_WIDE;

fn compressed(out: &mut Vec<u8>, v: u32) {
    if v < 0x80 {
        out.push(v as u8);
    } else if v < 0x4000 {
        out.extend_from_slice(&((v as u16) | 0x8000).to_be_bytes());
    } else {
        out.extend_from_slice(&(v | 0xC000_0000).to_be_bytes());
    }
}

fn coded(c: Coded, table: TableId, row: u32) -> u32 {
    let tag = c
        .tables
        .iter()
        .position(|&t| t == table)
        .expect("table not in coded index");
    (row << c.bits) | tag as u32
}

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

pub(crate) struct Builder {
    strings: Vec<u8>,
    user_strings: Vec<u8>,
    blobs: Vec<u8>,
    rows: Vec<Vec<Vec<u32>>>,
    il: Vec<u8>,
}

impl Builder {
    pub fn new() -> Builder {
        Builder {
            strings: vec![0],
            user_strings: vec![0],
            blobs: vec![0],
            rows: vec![Vec::new(); Table::COUNT],
            il: Vec::new(),
        }
    }

    fn rows(&self, t: TableId) -> u32 {
        self.rows[t as usize].len() as u32
    }

    fn row(&mut self, t: TableId, cells: Vec<u32>) -> u32 {
        assert_eq!(cells.len(), SCHEMA[t as usize].len());
        self.rows[t as usize].push(cells);
        self.rows(t)
    }

    fn string(&mut self, s: &str) -> u32 {
        let at = self.strings.len() as u32;
        self.strings.extend_from_slice(s.as_bytes());
        self.strings.push(0);
        at
    }

    fn blob(&mut self, b: &[u8]) -> u32 {
        let at = self.blobs.len() as u32;
        compressed(&mut self.blobs, b.len() as u32);
        self.blobs.extend_from_slice(b);
        at
    }

    /// A string literal; returns its `ldstr` token.
    pub fn user_string(&mut self, s: &str) -> u32 {
        let at = self.user_strings.len() as u32;
        let units: Vec<u16> = s.encode_utf16().collect();
        compressed(&mut self.user_strings, units.len() as u32 * 2 + 1);
        for u in units {
            self.user_strings.extend_from_slice(&u.to_le_bytes());
        }
        self.user_strings.push(0);
        0x7000_0000 | at
    }

    /// A TypeDef; the fields and methods added after it are its members.
    pub fn type_def(&mut self, namespace: &str, name: &str) -> u32 {
        let (n, ns) = (self.string(name), self.string(namespace));
        let (fields, methods) = (
            self.rows(Table::FIELD) + 1,
            self.rows(Table::METHOD_DEF) + 1,
        );
        self.row(Table::TYPE_DEF, vec![0, n, ns, 0, fields, methods])
    }

    pub fn type_ref(&mut self, namespace: &str, name: &str) -> u32 {
        let (n, ns) = (self.string(name), self.string(namespace));
        self.row(Table::TYPE_REF, vec![0, n, ns])
    }

    /// `GenericInst<class type>` over a TypeRef, with one `string` argument.
    pub fn generic_type_spec(&mut self, type_ref: u32) -> u32 {
        let mut sig = vec![0x15, 0x12];
        compressed(&mut sig, coded(TYPE_DEF_OR_REF, Table::TYPE_REF, type_ref));
        sig.extend_from_slice(&[1, 0x0E]);
        let b = self.blob(&sig);
        self.row(Table::TYPE_SPEC, vec![b])
    }

    /// A field of the last TypeDef; returns its token.
    pub fn field(&mut self, flags: u16, name: &str) -> u32 {
        let (n, sig) = (self.string(name), self.blob(&[0x06, 0x08]));
        0x0400_0000 | self.row(Table::FIELD, vec![u32::from(flags), n, sig])
    }

    /// An `int32` constant of a field (by token).
    pub fn constant_i4(&mut self, field: u32, value: i32) {
        let b = self.blob(&value.to_le_bytes());
        let parent = coded(HAS_CONSTANT, Table::FIELD, field & 0x00FF_FFFF);
        self.row(Table::CONSTANT, vec![0x08, parent, b]);
    }

    /// A `float32` constant of a field (by token).
    pub fn constant_r4(&mut self, field: u32, value: f32) {
        let b = self.blob(&value.to_le_bytes());
        let parent = coded(HAS_CONSTANT, Table::FIELD, field & 0x00FF_FFFF);
        self.row(Table::CONSTANT, vec![0x0C, parent, b]);
    }

    fn method_sig(&mut self, params: u8) -> u32 {
        let mut sig = vec![0x20, params, 0x01];
        sig.extend(std::iter::repeat_n(0x08, params as usize));
        self.blob(&sig)
    }

    /// A method of the last TypeDef, with a body if `code` is given;
    /// returns its token.
    pub fn method(&mut self, name: &str, params: u8, code: Option<&[u8]>) -> u32 {
        let rva = code.map_or(0, |c| self.body(c));
        let (n, sig) = (self.string(name), self.method_sig(params));
        0x0600_0000 | self.row(Table::METHOD_DEF, vec![rva, 0, 0, n, sig, 1])
    }

    /// A MemberRef on `(table, row)` (TypeRef, TypeDef or TypeSpec):
    /// a method with `params` parameters, or a field when `None`.
    pub fn member_ref(&mut self, parent: (TableId, u32), name: &str, params: Option<u8>) -> u32 {
        let n = self.string(name);
        let sig = match params {
            Some(p) => self.method_sig(p),
            None => self.blob(&[0x06, 0x08]),
        };
        let p = coded(MEMBER_REF_PARENT, parent.0, parent.1);
        0x0A00_0000 | self.row(Table::MEMBER_REF, vec![p, n, sig])
    }

    /// Stores a method body (tiny header up to 63 bytes, fat above);
    /// returns its RVA.
    fn body(&mut self, code: &[u8]) -> u32 {
        pad4(&mut self.il);
        let rva = SECTION_RVA + (CLI_HEADER + self.il.len()) as u32;
        if code.len() < 64 {
            self.il.push(((code.len() as u8) << 2) | 0x2);
        } else {
            self.il.extend_from_slice(&[0x03, 0x30]); // fat, 3 dwords
            self.il.extend_from_slice(&8u16.to_le_bytes()); // max stack
            self.il
                .extend_from_slice(&(code.len() as u32).to_le_bytes());
            self.il.extend_from_slice(&0u32.to_le_bytes()); // locals
        }
        self.il.extend_from_slice(code);
        rva
    }

    fn table_stream(&self) -> Vec<u8> {
        let mut rows = [0u32; Table::COUNT];
        let mut valid = 0u64;
        for (t, r) in rows.iter_mut().enumerate() {
            *r = self.rows(t as TableId);
            if *r > 0 {
                valid |= 1 << t;
            }
        }
        let mut out = vec![0, 0, 0, 0, 2, 0, HEAPS, 1];
        out.extend_from_slice(&valid.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        for r in rows.iter().filter(|&&r| r > 0) {
            out.extend_from_slice(&r.to_le_bytes());
        }
        for (t, table) in self.rows.iter().enumerate() {
            for row in table {
                for (&col, &v) in SCHEMA[t].iter().zip(row) {
                    match col_width(col, &rows, HEAPS) {
                        2 => out.extend_from_slice(&(v as u16).to_le_bytes()),
                        _ => out.extend_from_slice(&v.to_le_bytes()),
                    }
                }
            }
        }
        pad4(&mut out);
        out
    }

    fn metadata(&self) -> Vec<u8> {
        let mut heaps = [
            ("#~", self.table_stream()),
            ("#Strings", self.strings.clone()),
            ("#US", self.user_strings.clone()),
            ("#Blob", self.blobs.clone()),
        ];
        for (_, h) in &mut heaps {
            pad4(h);
        }
        let version = b"v4.0.30319\0\0";
        let mut header = Vec::new();
        header.extend_from_slice(&0x424A_5342u32.to_le_bytes());
        header.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0, 0]);
        header.extend_from_slice(&(version.len() as u32).to_le_bytes());
        header.extend_from_slice(version);
        header.extend_from_slice(&[0, 0]);
        header.extend_from_slice(&(heaps.len() as u16).to_le_bytes());
        let names_len: usize = heaps.iter().map(|(n, _)| 8 + (n.len() + 4) / 4 * 4).sum();
        let mut offset = header.len() + names_len;
        let mut body = Vec::new();
        for (name, h) in &heaps {
            header.extend_from_slice(&(offset as u32).to_le_bytes());
            header.extend_from_slice(&(h.len() as u32).to_le_bytes());
            header.extend_from_slice(name.as_bytes());
            header.push(0);
            pad4(&mut header);
            offset += h.len();
            body.extend_from_slice(h);
        }
        header.extend_from_slice(&body);
        header
    }

    pub fn build(&self) -> Vec<u8> {
        let mut il = self.il.clone();
        pad4(&mut il);
        let meta = self.metadata();
        let meta_rva = SECTION_RVA + (CLI_HEADER + il.len()) as u32;
        let mut section = Vec::new();
        section.extend_from_slice(&(CLI_HEADER as u32).to_le_bytes());
        section.extend_from_slice(&[2, 0, 5, 0]);
        section.extend_from_slice(&meta_rva.to_le_bytes());
        section.extend_from_slice(&(meta.len() as u32).to_le_bytes());
        section.resize(CLI_HEADER, 0);
        section.extend_from_slice(&il);
        section.extend_from_slice(&meta);

        let mut f = vec![0u8; SECTION_FILE];
        f[..2].copy_from_slice(b"MZ");
        f[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        let mut pe = Vec::new();
        pe.extend_from_slice(b"PE\0\0");
        pe.extend_from_slice(&0x14Cu16.to_le_bytes()); // i386
        pe.extend_from_slice(&1u16.to_le_bytes()); // sections
        pe.extend_from_slice(&[0; 12]);
        pe.extend_from_slice(&224u16.to_le_bytes()); // optional header size
        pe.extend_from_slice(&0x2102u16.to_le_bytes()); // DLL
        let mut opt = vec![0u8; 224];
        opt[..2].copy_from_slice(&0x10Bu16.to_le_bytes()); // PE32
        opt[92..96].copy_from_slice(&16u32.to_le_bytes()); // data directories
        let cli_dir = 96 + 14 * 8;
        opt[cli_dir..cli_dir + 4].copy_from_slice(&SECTION_RVA.to_le_bytes());
        opt[cli_dir + 4..cli_dir + 8].copy_from_slice(&(CLI_HEADER as u32).to_le_bytes());
        pe.extend_from_slice(&opt);
        pe.extend_from_slice(b".text\0\0\0");
        for v in [
            section.len() as u32,
            SECTION_RVA,
            section.len() as u32,
            SECTION_FILE as u32,
        ] {
            pe.extend_from_slice(&v.to_le_bytes());
        }
        pe.extend_from_slice(&[0; 16]);
        f[0x80..0x80 + pe.len()].copy_from_slice(&pe);
        f.extend_from_slice(&section);
        f
    }
}

/// A small IL assembler for the instructions the readers accept.
#[derive(Default)]
pub(crate) struct Il(pub Vec<u8>);

impl Il {
    fn op_token(mut self, op: u8, token: u32) -> Il {
        self.0.push(op);
        self.0.extend_from_slice(&token.to_le_bytes());
        self
    }
    pub fn ldc(mut self, v: i32) -> Il {
        match v {
            -1..=8 => self.0.push((0x16 + v) as u8),
            -128..=127 => self.0.extend_from_slice(&[0x1F, v as i8 as u8]),
            _ => {
                self.0.push(0x20);
                self.0.extend_from_slice(&v.to_le_bytes());
            }
        }
        self
    }
    pub fn ldc_r4(mut self, v: f32) -> Il {
        self.0.push(0x22);
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub fn op(mut self, op: u8) -> Il {
        self.0.push(op);
        self
    }
    pub fn ldstr(self, t: u32) -> Il {
        self.op_token(0x72, t)
    }
    pub fn call(self, t: u32) -> Il {
        self.op_token(0x28, t)
    }
    pub fn callvirt(self, t: u32) -> Il {
        self.op_token(0x6F, t)
    }
    pub fn newobj(self, t: u32) -> Il {
        self.op_token(0x73, t)
    }
    pub fn newarr(self, t: u32) -> Il {
        self.op_token(0x8D, t)
    }
    pub fn ldsfld(self, t: u32) -> Il {
        self.op_token(0x7E, t)
    }
    pub fn stsfld(self, t: u32) -> Il {
        self.op_token(0x80, t)
    }
    pub fn stfld(self, t: u32) -> Il {
        self.op_token(0x7D, t)
    }
    pub fn dup(self) -> Il {
        self.op(0x25)
    }
    pub fn pop(self) -> Il {
        self.op(0x26)
    }
    pub fn stelem_ref(self) -> Il {
        self.op(0xA2)
    }
    pub fn ret(self) -> Il {
        self.op(0x2A)
    }
}
