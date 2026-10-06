//! Bounds-checked cursor over untrusted bytes.

use crate::{Error, ErrorKind, Result};

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    pub big_endian: bool,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], big_endian: bool) -> Self {
        Reader {
            data,
            pos: 0,
            big_endian,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn error(&self, kind: ErrorKind) -> Error {
        Error {
            offset: self.pos,
            kind,
        }
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(self.error(ErrorKind::UnexpectedEof {
                needed: pos - self.data.len(),
            }));
        }
        self.pos = pos;
        Ok(())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let remaining = self.data.len() - self.pos;
        if remaining < n {
            return Err(self.error(ErrorKind::UnexpectedEof {
                needed: n - remaining,
            }));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    /// Skips to the next multiple of `n` (absolute position in the data).
    pub fn align(&mut self, n: usize) -> Result<()> {
        let pad = (n - self.pos % n) % n;
        self.bytes(pad).map(|_| ())
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
}

macro_rules! number {
    ($($name:ident: $ty:ty),*) => {
        impl Reader<'_> {
            $(
                pub fn $name(&mut self) -> Result<$ty> {
                    let raw = self.array()?;
                    Ok(if self.big_endian { <$ty>::from_be_bytes(raw) } else { <$ty>::from_le_bytes(raw) })
                }
            )*
        }
    };
}

number!(u16: u16, i16: i16, u32: u32, i32: i32, u64: u64, i64: i64);

impl Reader<'_> {
    /// A null-terminated string (UTF-8; invalid bytes are replaced).
    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| self.error(ErrorKind::UnexpectedEof { needed: 1 }))?;
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(s)
    }

    /// A count that must be non-negative and plausible for the remaining
    /// data, given that each item takes at least `min_item_bytes`.
    pub fn count(&mut self, min_item_bytes: usize) -> Result<usize> {
        let at = self.pos;
        let n = self.i32()?;
        let remaining = self.data.len() - self.pos;
        if n < 0 || (n as usize).saturating_mul(min_item_bytes.max(1)) > remaining {
            return Err(Error {
                offset: at,
                kind: ErrorKind::Invalid(format!("implausible count {n}")),
            });
        }
        Ok(n as usize)
    }
}
