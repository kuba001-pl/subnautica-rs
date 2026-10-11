//! FNV-1a, 64 bits: the hash behind the rules fingerprint, the slot-fill
//! hash and the world-state hash. Both ends must compute it identically,
//! so it is defined here, byte by byte (little-endian for numbers). A
//! check against accidental differences, not a security measure.

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fnv64(u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Fnv64(OFFSET)
    }
}

impl Fnv64 {
    pub fn new() -> Fnv64 {
        Fnv64::default()
    }

    pub fn bytes(&mut self, data: &[u8]) -> &mut Self {
        for &b in data {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(PRIME);
        }
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.bytes(&v.to_le_bytes())
    }

    /// A float by its bits (so −0.0 and 0.0 differ, as they would on the
    /// wire).
    pub fn f64(&mut self, v: f64) -> &mut Self {
        self.u64(v.to_bits())
    }

    pub fn f32(&mut self, v: f32) -> &mut Self {
        self.bytes(&v.to_bits().to_le_bytes())
    }

    /// Text with its length first, so "ab" + "c" ≠ "a" + "bc".
    pub fn str(&mut self, s: &str) -> &mut Self {
        self.u64(s.len() as u64).bytes(s.as_bytes())
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_test_vectors() {
        // FNV-1a 64 reference values (the algorithm's published vectors).
        assert_eq!(Fnv64::new().finish(), 0xcbf2_9ce4_8422_2325);
        assert_eq!(Fnv64::new().bytes(b"a").finish(), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            Fnv64::new().bytes(b"foobar").finish(),
            0x8594_4171_f739_67e8
        );
    }

    #[test]
    fn text_is_length_prefixed() {
        let ab_c = Fnv64::new().str("ab").str("c").finish();
        let a_bc = Fnv64::new().str("a").str("bc").finish();
        assert_ne!(ab_c, a_bc);
    }
}
