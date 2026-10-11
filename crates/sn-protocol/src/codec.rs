//! The byte-level codec: a writer and a strict reader. Every value has
//! exactly one encoding (minimal varints, bools 0 or 1, finite floats), so
//! a message that decodes re-encodes to the same bytes.

use crate::{Error, Result};

/// Appends values to a buffer.
#[derive(Default)]
pub struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    pub fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i16(&mut self, v: i16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64_fixed(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    /// LEB128: 7 bits per byte, low bits first.
    pub fn varint(&mut self, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.out.push(byte);
                return;
            }
            self.out.push(byte | 0x80);
        }
    }

    /// Zigzag, then LEB128.
    pub fn varint_signed(&mut self, v: i64) {
        self.varint(((v << 1) ^ (v >> 63)) as u64);
    }

    pub fn bool(&mut self, v: bool) {
        self.out.push(u8::from(v));
    }

    pub fn f32(&mut self, v: f32) {
        self.out.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    pub fn f64(&mut self, v: f64) {
        self.out.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    /// A varint length, then the bytes.
    pub fn bytes(&mut self, b: &[u8]) {
        self.varint(b.len() as u64);
        self.out.extend_from_slice(b);
    }
}

/// Reads values from bytes; every failure names its byte offset.
pub struct Reader<'a> {
    data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Reader<'a> {
        Reader { data, pos: 0 }
    }

    pub fn error(&self, at: usize, message: impl Into<String>) -> Error {
        Error::new(at, message)
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let s = &self.data[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            None => Err(self.error(
                self.pos,
                format!(
                    "{what}: needs {n} bytes, {} left",
                    self.data.len() - self.pos
                ),
            )),
        }
    }

    fn array<const N: usize>(&mut self, what: &str) -> Result<[u8; N]> {
        let mut a = [0; N];
        a.copy_from_slice(self.take(N, what)?);
        Ok(a)
    }

    pub fn u8(&mut self, what: &str) -> Result<u8> {
        Ok(self.array::<1>(what)?[0])
    }

    pub fn u16(&mut self, what: &str) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array(what)?))
    }

    pub fn i16(&mut self, what: &str) -> Result<i16> {
        Ok(i16::from_le_bytes(self.array(what)?))
    }

    pub fn u64_fixed(&mut self, what: &str) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array(what)?))
    }

    /// A minimal LEB128 of at most 10 bytes that fits in 64 bits.
    pub fn varint(&mut self, what: &str) -> Result<u64> {
        let start = self.pos;
        let mut v = 0u64;
        for i in 0..10 {
            let byte = self.u8(what)?;
            let bits = u64::from(byte & 0x7f);
            if i == 9 && byte > 1 {
                return Err(self.error(start, format!("{what}: varint overflows 64 bits")));
            }
            v |= bits << (7 * i);
            if byte & 0x80 == 0 {
                if byte == 0 && i > 0 {
                    return Err(self.error(start, format!("{what}: varint not minimal")));
                }
                return Ok(v);
            }
        }
        Err(self.error(start, format!("{what}: varint longer than 10 bytes")))
    }

    pub fn varint_u32(&mut self, what: &str) -> Result<u32> {
        let at = self.pos;
        let v = self.varint(what)?;
        u32::try_from(v).map_err(|_| self.error(at, format!("{what}: {v} is above 32 bits")))
    }

    pub fn varint_u16(&mut self, what: &str) -> Result<u16> {
        let at = self.pos;
        let v = self.varint(what)?;
        u16::try_from(v).map_err(|_| self.error(at, format!("{what}: {v} is above 16 bits")))
    }

    pub fn varint_i32(&mut self, what: &str) -> Result<i32> {
        let at = self.pos;
        let z = self.varint(what)?;
        let v = ((z >> 1) as i64) ^ -((z & 1) as i64);
        i32::try_from(v).map_err(|_| self.error(at, format!("{what}: {v} is outside 32 bits")))
    }

    pub fn bool(&mut self, what: &str) -> Result<bool> {
        let at = self.pos;
        match self.u8(what)? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(self.error(at, format!("{what}: bool byte {b}"))),
        }
    }

    pub fn f32(&mut self, what: &str) -> Result<f32> {
        let at = self.pos;
        let v = f32::from_bits(u32::from_le_bytes(self.array(what)?));
        if v.is_finite() {
            Ok(v)
        } else {
            Err(self.error(at, format!("{what}: not a finite number")))
        }
    }

    pub fn f64(&mut self, what: &str) -> Result<f64> {
        let at = self.pos;
        let v = f64::from_bits(u64::from_le_bytes(self.array(what)?));
        if v.is_finite() {
            Ok(v)
        } else {
            Err(self.error(at, format!("{what}: not a finite number")))
        }
    }

    /// UTF-8 text of at most `max` bytes.
    pub fn str(&mut self, max: usize, what: &str) -> Result<String> {
        let at = self.pos;
        let n = self.count(max, what)?;
        let bytes = self.take(n, what)?;
        match std::str::from_utf8(bytes) {
            Ok(s) => Ok(s.to_owned()),
            Err(_) => Err(self.error(at, format!("{what}: not UTF-8"))),
        }
    }

    /// At most `max` bytes, after their varint length.
    pub fn bytes(&mut self, max: usize, what: &str) -> Result<Vec<u8>> {
        let n = self.count(max, what)?;
        Ok(self.take(n, what)?.to_vec())
    }

    /// A count of at most `max`.
    pub fn count(&mut self, max: usize, what: &str) -> Result<usize> {
        let at = self.pos;
        let n = self.varint(what)?;
        match usize::try_from(n) {
            Ok(n) if n <= max => Ok(n),
            _ => Err(self.error(at, format!("{what}: {n} is more than {max}"))),
        }
    }

    /// Every byte must have been read.
    pub fn finish(&self) -> Result<()> {
        if self.pos == self.data.len() {
            Ok(())
        } else {
            Err(self.error(
                self.pos,
                format!("{} bytes after the message", self.data.len() - self.pos),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip_at_the_edges() {
        for v in [
            0,
            1,
            127,
            128,
            16_383,
            16_384,
            u64::from(u32::MAX),
            u64::MAX,
        ] {
            let mut w = Writer::default();
            w.varint(v);
            let mut r = Reader::new(&w.out);
            assert_eq!(r.varint("v").unwrap(), v);
            r.finish().unwrap();
        }
        for v in [0, -1, 1, i64::from(i32::MIN), i64::from(i32::MAX)] {
            let mut w = Writer::default();
            w.varint_signed(v);
            let mut r = Reader::new(&w.out);
            assert_eq!(i64::from(r.varint_i32("v").unwrap()), v);
        }
    }

    #[test]
    fn non_minimal_and_overlong_varints_are_errors() {
        // 0 written in two bytes.
        assert!(Reader::new(&[0x80, 0x00]).varint("v").is_err());
        // 11 bytes.
        assert!(Reader::new(&[0xff; 11]).varint("v").is_err());
        // Bit 64 set.
        let mut b = [0xff; 10];
        b[9] = 0x02;
        assert!(Reader::new(&b).varint("v").is_err());
        // Truncated.
        let e = Reader::new(&[0x80]).varint("v").unwrap_err();
        assert_eq!(e.offset, 1);
    }

    #[test]
    fn strict_bools_floats_and_text() {
        assert!(Reader::new(&[2]).bool("b").is_err());
        assert!(
            Reader::new(&f32::NAN.to_bits().to_le_bytes())
                .f32("f")
                .is_err()
        );
        assert!(
            Reader::new(&f64::INFINITY.to_bits().to_le_bytes())
                .f64("f")
                .is_err()
        );
        assert!(Reader::new(&[2, 0xc3, 0x28]).str(8, "s").is_err());
        assert!(Reader::new(&[9]).str(8, "s").is_err());
        assert_eq!(Reader::new(&[2, b'h', b'i']).str(8, "s").unwrap(), "hi");
    }
}
