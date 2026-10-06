//! A minimal reader for the protobuf wire format: varints, fixed 32/64-bit
//! values and length-delimited fields. Every read is checked; errors carry
//! the byte offset in the original buffer.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError {
    /// Byte offset in the buffer passed to the top-level parser.
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for WireError {}

pub type Result<T> = std::result::Result<T, WireError>;

/// A field value. Length-delimited values are returned as a reader over
/// their bytes.
#[derive(Clone, Debug)]
pub enum Value<'a> {
    Varint(u64),
    /// Skipped; nothing we read uses 64-bit fixed fields.
    Fixed64,
    Fixed32(u32),
    Bytes(Wire<'a>),
}

/// A cursor over protobuf bytes. `base` is the offset of `data[0]` in the
/// top-level buffer, for error messages.
#[derive(Clone, Debug)]
pub struct Wire<'a> {
    data: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> Wire<'a> {
    pub fn new(data: &'a [u8]) -> Wire<'a> {
        Wire {
            data,
            pos: 0,
            base: 0,
        }
    }

    /// Offset of the cursor in the top-level buffer.
    pub fn offset(&self) -> usize {
        self.base + self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// All bytes of this reader (not just the unread rest).
    pub fn bytes(&self) -> &'a [u8] {
        self.data
    }

    pub fn error<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(WireError {
            offset: self.offset(),
            message: message.into(),
        })
    }

    pub fn varint(&mut self) -> Result<u64> {
        let start = self.offset();
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let Some(&byte) = self.data.get(self.pos) else {
                return Err(WireError {
                    offset: start,
                    message: "varint runs past the end".into(),
                });
            };
            self.pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(WireError {
            offset: start,
            message: "varint longer than 10 bytes".into(),
        })
    }

    fn take(&mut self, len: usize) -> Result<Wire<'a>> {
        let end = self.pos.checked_add(len).filter(|&e| e <= self.data.len());
        let Some(end) = end else {
            return self.error(format!(
                "{len} bytes wanted, {} left",
                self.data.len() - self.pos
            ));
        };
        let sub = Wire {
            data: &self.data[self.pos..end],
            pos: 0,
            base: self.offset(),
        };
        self.pos = end;
        Ok(sub)
    }

    /// A message preceded by its length as a varint (no field tag).
    pub fn length_prefixed(&mut self) -> Result<Wire<'a>> {
        let len = self.varint()?;
        let len = usize::try_from(len).or_else(|_| self.error("length too big"))?;
        self.take(len)
    }

    /// `len` raw bytes.
    pub fn raw(&mut self, len: usize) -> Result<Wire<'a>> {
        self.take(len)
    }

    /// The next field, or `None` at the end.
    pub fn field(&mut self) -> Result<Option<(u32, Value<'a>)>> {
        if self.is_empty() {
            return Ok(None);
        }
        let at = self.offset();
        let key = self.varint()?;
        let number = u32::try_from(key >> 3).or_else(|_| self.error("field number too big"))?;
        let value = match key & 7 {
            0 => Value::Varint(self.varint()?),
            1 => {
                self.take(8)?;
                Value::Fixed64
            }
            2 => Value::Bytes(self.length_prefixed()?),
            5 => {
                let b = self.take(4)?.data;
                Value::Fixed32(u32::from_le_bytes(b.try_into().unwrap_or([0; 4])))
            }
            other => {
                return Err(WireError {
                    offset: at,
                    message: format!("unsupported wire type {other} (field {number})"),
                });
            }
        };
        Ok(Some((number, value)))
    }
}

impl Value<'_> {
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Varint(_) => "varint",
            Value::Fixed64 => "fixed64",
            Value::Fixed32(_) => "fixed32",
            Value::Bytes(_) => "bytes",
        }
    }
}

/// Test helpers: build protobuf bytes.
#[cfg(test)]
pub(crate) mod encode {
    pub fn varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    pub fn uint(out: &mut Vec<u8>, field: u32, v: u64) {
        varint(out, u64::from(field) << 3);
        varint(out, v);
    }

    pub fn bytes(out: &mut Vec<u8>, field: u32, b: &[u8]) {
        varint(out, (u64::from(field) << 3) | 2);
        varint(out, b.len() as u64);
        out.extend_from_slice(b);
    }

    pub fn float(out: &mut Vec<u8>, field: u32, v: f32) {
        varint(out, (u64::from(field) << 3) | 5);
        out.extend_from_slice(&v.to_le_bytes());
    }

    /// A message with a varint length in front.
    pub fn prefixed(out: &mut Vec<u8>, message: &[u8]) {
        varint(out, message.len() as u64);
        out.extend_from_slice(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_fields_of_every_supported_type() {
        let mut b = Vec::new();
        encode::uint(&mut b, 1, 300);
        encode::float(&mut b, 2, 1.5);
        encode::bytes(&mut b, 3, b"hi");
        b.extend([(4 << 3) | 1, 1, 0, 0, 0, 0, 0, 0, 0]);
        let mut w = Wire::new(&b);
        let mut seen = Vec::new();
        while let Some((n, v)) = w.field().unwrap() {
            seen.push(match v {
                Value::Varint(x) => (n, x),
                Value::Fixed32(x) => (n, u64::from(x)),
                Value::Fixed64 => (n, 1),
                Value::Bytes(s) => (n, s.bytes().len() as u64),
            });
        }
        assert_eq!(
            seen,
            vec![(1, 300), (2, 1.5f32.to_bits() as u64), (3, 2), (4, 1)]
        );
    }

    #[test]
    fn truncated_input_reports_the_offset() {
        let mut b = Vec::new();
        encode::bytes(&mut b, 1, b"hello");
        b.truncate(4);
        let err = Wire::new(&b).field().unwrap_err();
        assert_eq!(err.offset, 2);
        let err = Wire::new(&[0x80, 0x80]).varint().unwrap_err();
        assert_eq!(err.offset, 0);
    }

    #[test]
    fn groups_are_rejected() {
        let err = Wire::new(&[(1 << 3) | 3]).field().unwrap_err();
        assert!(err.message.contains("wire type 3"));
    }
}
