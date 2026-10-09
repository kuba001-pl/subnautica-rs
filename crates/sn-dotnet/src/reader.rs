//! Bounds-checked little-endian cursor.

use crate::{Error, Result};

#[derive(Clone)]
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    /// Added to positions in errors (the slice's place in the file).
    base: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader::at(data, 0)
    }

    /// A reader over `data`, whose first byte is at `base` in the file.
    pub fn at(data: &'a [u8], base: usize) -> Reader<'a> {
        Reader { data, pos: 0, base }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn error(&self, message: impl Into<String>) -> Error {
        Error::new(self.base + self.pos, message)
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(self.error(format!("seek to {pos} past the end ({})", self.data.len())));
        }
        self.pos = pos;
        Ok(())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(self.error(format!(
                "unexpected end of data: needed {n} bytes, {} left",
                self.remaining()
            )));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64> {
        let mut a = [0; 8];
        a.copy_from_slice(self.bytes(8)?);
        Ok(u64::from_le_bytes(a))
    }

    /// An ECMA-335 compressed unsigned integer (1, 2 or 4 bytes).
    pub fn compressed(&mut self) -> Result<u32> {
        let b0 = u32::from(self.u8()?);
        if b0 & 0x80 == 0 {
            Ok(b0)
        } else if b0 & 0xC0 == 0x80 {
            Ok(((b0 & 0x3F) << 8) | u32::from(self.u8()?))
        } else if b0 & 0xE0 == 0xC0 {
            let rest = self.bytes(3)?;
            Ok(((b0 & 0x1F) << 24)
                | (u32::from(rest[0]) << 16)
                | (u32::from(rest[1]) << 8)
                | u32::from(rest[2]))
        } else {
            Err(self.error(format!("bad compressed integer lead byte {b0:#04x}")))
        }
    }

    /// A null-terminated ASCII name of at most `max` bytes (stream names).
    pub fn cstr(&mut self, max: usize) -> Result<&'a str> {
        let rest = &self.data[self.pos..];
        let len = rest
            .iter()
            .take(max)
            .position(|&b| b == 0)
            .ok_or_else(|| self.error("unterminated name"))?;
        let s = std::str::from_utf8(&rest[..len]).map_err(|_| self.error("name is not UTF-8"))?;
        self.pos += len + 1;
        Ok(s)
    }

    pub fn align(&mut self, n: usize) -> Result<()> {
        let pad = (n - self.pos % n) % n;
        self.bytes(pad).map(|_| ())
    }
}
