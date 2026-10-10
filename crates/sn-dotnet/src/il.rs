//! Decoding CIL instruction streams (ECMA-335 § III): every opcode's
//! operand size, so a method body can be split into instructions. What an
//! instruction does is left to the callers (`game.rs`), which accept only
//! the few patterns they expect.

use crate::reader::Reader;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    None,
    /// `ldarg.s`, `ldc.i4.s`, `unaligned.`, …: one byte (sign-extended
    /// for `ldc.i4.s`).
    I8(i8),
    U8(u8),
    U16(u16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    /// A metadata token (method, field, type, string).
    Token(u32),
    /// Branch target, relative to the next instruction.
    Branch(i32),
    Switch(Vec<i32>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instr {
    /// Byte offset in the method body.
    pub offset: usize,
    /// One-byte opcodes as is; two-byte ones as `0xFE00 | second byte`.
    pub opcode: u16,
    pub operand: Operand,
}

impl Instr {
    /// The constant pushed by an `ldc.i4` form.
    pub fn ldc_i4(&self) -> Option<i32> {
        match (self.opcode, &self.operand) {
            (0x15..=0x1E, _) => Some(i32::from(self.opcode as u8) - 0x16),
            (0x1F, Operand::I8(v)) => Some(i32::from(*v)),
            (0x20, Operand::I32(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn token(&self) -> Option<u32> {
        match self.operand {
            Operand::Token(t) => Some(t),
            _ => None,
        }
    }
}

pub const LDARG_0: u16 = 0x02;
pub const LDNULL: u16 = 0x14;
pub const LDC_R4: u16 = 0x22;
pub const DUP: u16 = 0x25;
pub const POP: u16 = 0x26;
pub const CALL: u16 = 0x28;
pub const RET: u16 = 0x2A;
pub const MUL: u16 = 0x5A;
pub const CALLVIRT: u16 = 0x6F;
pub const LDSTR: u16 = 0x72;
pub const NEWOBJ: u16 = 0x73;
pub const STFLD: u16 = 0x7D;
pub const LDSFLD: u16 = 0x7E;
pub const STSFLD: u16 = 0x80;
pub const NEWARR: u16 = 0x8D;
pub const STELEM_REF: u16 = 0xA2;

#[derive(Clone, Copy)]
enum Kind {
    None,
    I8,
    U8,
    U16,
    I32,
    I64,
    F32,
    F64,
    Token,
    BranchShort,
    Branch,
    Switch,
}

/// Operand kind of a one-byte opcode; `None` for unassigned bytes.
fn one_byte(op: u8) -> Option<Kind> {
    use Kind::*;
    Some(match op {
        0x00..=0x0D => None,        // nop … stloc.3
        0x0E..=0x13 => U8,          // ldarg.s … stloc.s
        0x14..=0x1E => None,        // ldnull, ldc.i4.m1 … ldc.i4.8
        0x1F => I8,                 // ldc.i4.s
        0x20 => I32,                // ldc.i4
        0x21 => I64,                // ldc.i8
        0x22 => F32,                // ldc.r4
        0x23 => F64,                // ldc.r8
        0x25 | 0x26 => None,        // dup, pop
        0x27..=0x29 => Token,       // jmp, call, calli
        0x2A => None,               // ret
        0x2B..=0x37 => BranchShort, // br.s … blt.un.s
        0x38..=0x44 => Branch,      // br … blt.un
        0x45 => Switch,
        0x46..=0x6E => None,  // ldind.*, stind.*, arithmetic, conv.*
        0x6F..=0x75 => Token, // callvirt, cpobj, ldobj, ldstr, newobj, castclass, isinst
        0x76 => None,         // conv.r.un
        0x79 => Token,        // unbox
        0x7A => None,         // throw
        0x7B..=0x81 => Token, // ldfld … stobj
        0x82..=0x8B => None,  // conv.ovf.*.un
        0x8C | 0x8D => Token, // box, newarr
        0x8E => None,         // ldlen
        0x8F => Token,        // ldelema
        0x90..=0xA2 => None,  // ldelem.*, stelem.*
        0xA3..=0xA5 => Token, // ldelem, stelem, unbox.any
        0xB3..=0xBA => None,  // conv.ovf.*
        0xC2 => Token,        // refanyval
        0xC3 => None,         // ckfinite
        0xC6 => Token,        // mkrefany
        0xD0 => Token,        // ldtoken
        0xD1..=0xDC => None,  // conv.u2 … endfinally
        0xDD => Branch,       // leave
        0xDE => BranchShort,  // leave.s
        0xDF | 0xE0 => None,  // stind.i, conv.u
        _ => return Option::None,
    })
}

/// Operand kind of `0xFE xx`.
fn two_byte(op: u8) -> Option<Kind> {
    use Kind::*;
    Some(match op {
        0x00..=0x05 => None,  // arglist, ceq, cgt, cgt.un, clt, clt.un
        0x06 | 0x07 => Token, // ldftn, ldvirtftn
        0x09..=0x0E => U16,   // ldarg … stloc
        0x0F => None,         // localloc
        0x11 => None,         // endfilter
        0x12 => U8,           // unaligned.
        0x13 | 0x14 => None,  // volatile., tail.
        0x15 | 0x16 => Token, // initobj, constrained.
        0x17 | 0x18 => None,  // cpblk, initblk
        0x19 => U8,           // no.
        0x1A => None,         // rethrow
        0x1C => Token,        // sizeof
        0x1D | 0x1E => None,  // refanytype, readonly.
        _ => return Option::None,
    })
}

/// Splits a method body into instructions. `base` is the body's file
/// offset, for errors.
pub fn decode(code: &[u8], base: usize) -> Result<Vec<Instr>> {
    let mut r = Reader::at(code, base);
    let mut out = Vec::new();
    while r.remaining() > 0 {
        let offset = r.pos();
        let first = r.u8()?;
        let (opcode, kind) = if first == 0xFE {
            let second = r.u8()?;
            let kind = two_byte(second).ok_or_else(|| {
                Error::new(base + offset, format!("unknown opcode fe {second:02x}"))
            })?;
            (0xFE00 | u16::from(second), kind)
        } else {
            let kind = one_byte(first)
                .ok_or_else(|| Error::new(base + offset, format!("unknown opcode {first:02x}")))?;
            (u16::from(first), kind)
        };
        let operand = match kind {
            Kind::None => Operand::None,
            Kind::I8 => Operand::I8(r.u8()? as i8),
            Kind::U8 => Operand::U8(r.u8()?),
            Kind::U16 => Operand::U16(r.u16()?),
            Kind::I32 => Operand::I32(r.u32()? as i32),
            Kind::I64 => Operand::I64(r.u64()? as i64),
            Kind::F32 => Operand::F32(f32::from_bits(r.u32()?)),
            Kind::F64 => Operand::F64(f64::from_bits(r.u64()?)),
            Kind::Token => Operand::Token(r.u32()?),
            Kind::BranchShort => Operand::Branch(i32::from(r.u8()? as i8)),
            Kind::Branch => Operand::Branch(r.u32()? as i32),
            Kind::Switch => {
                let n = r.u32()? as usize;
                if n > r.remaining() / 4 {
                    return Err(r.error(format!("switch with {n} targets")));
                }
                let mut targets = Vec::with_capacity(n);
                for _ in 0..n {
                    targets.push(r.u32()? as i32);
                }
                Operand::Switch(targets)
            }
        };
        out.push(Instr {
            offset,
            opcode,
            operand,
        });
    }
    Ok(out)
}

/// Mnemonic of the opcodes the readers name in errors; hex otherwise.
pub fn opcode_name(opcode: u16) -> String {
    let name = match opcode {
        LDNULL => "ldnull",
        0x15..=0x1E => "ldc.i4.<n>",
        0x1F => "ldc.i4.s",
        0x20 => "ldc.i4",
        LDC_R4 => "ldc.r4",
        DUP => "dup",
        POP => "pop",
        CALL => "call",
        RET => "ret",
        CALLVIRT => "callvirt",
        LDSTR => "ldstr",
        NEWOBJ => "newobj",
        0x7B => "ldfld",
        0x7C => "ldflda",
        LDSFLD => "ldsfld",
        0x7F => "ldsflda",
        STSFLD => "stsfld",
        NEWARR => "newarr",
        STELEM_REF => "stelem.ref",
        _ if opcode > 0xFF => return format!("fe {:02x}", opcode & 0xFF),
        _ => return format!("{opcode:02x}"),
    };
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_operands() {
        let code = [
            0x16, // ldc.i4.0
            0x1F, 0xFE, // ldc.i4.s -2
            0x20, 0x0E, 0x17, 0, 0, // ldc.i4 5902
            0x22, 0, 0, 0x80, 0x3F, // ldc.r4 1.0
            0x72, 1, 0, 0, 0x70, // ldstr
            0x2C, 0x02, // brfalse.s +2
            0x45, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, // switch
            0xFE, 0x0C, 3, 0, // ldloc 3
            0xFE, 0x16, 4, 0, 0, 0x02, // constrained.
            0x2A, // ret
        ];
        let instrs = decode(&code, 0).unwrap();
        let ops: Vec<u16> = instrs.iter().map(|i| i.opcode).collect();
        assert_eq!(
            ops,
            vec![
                0x16, 0x1F, 0x20, 0x22, 0x72, 0x2C, 0x45, 0xFE0C, 0xFE16, 0x2A
            ]
        );
        assert_eq!(instrs[0].ldc_i4(), Some(0));
        assert_eq!(instrs[1].ldc_i4(), Some(-2));
        assert_eq!(instrs[2].ldc_i4(), Some(5902));
        assert_eq!(instrs[3].operand, Operand::F32(1.0));
        assert_eq!(instrs[4].token(), Some(0x7000_0001));
        assert_eq!(instrs[5].operand, Operand::Branch(2));
        assert_eq!(instrs[6].operand, Operand::Switch(vec![1, 2]));
        assert_eq!(instrs[7].operand, Operand::U16(3));
        assert_eq!(instrs[8].token(), Some(0x0200_0004));
        assert_eq!(instrs[9].offset, code.len() - 1);
        assert_eq!(
            Instr {
                offset: 0,
                opcode: 0x15,
                operand: Operand::None
            }
            .ldc_i4(),
            Some(-1)
        );
        assert_eq!(
            Instr {
                offset: 0,
                opcode: 0x1E,
                operand: Operand::None
            }
            .ldc_i4(),
            Some(8)
        );
    }

    #[test]
    fn unknown_or_cut_instructions_are_errors() {
        assert!(decode(&[0x24], 0).is_err());
        assert!(decode(&[0xFE, 0x08], 0).is_err());
        assert!(decode(&[0x20, 1, 2], 0).is_err());
        assert!(decode(&[0x45, 0xFF, 0xFF, 0xFF, 0xFF], 0).is_err());
        // Every byte value decodes or errors, never panics.
        for a in 0..=255u8 {
            for b in [0u8, 0x0C, 0xFF] {
                let _ = decode(&[a, b, 0, 0], 0);
                let _ = decode(&[0xFE, a, b], 0);
            }
        }
    }
}
