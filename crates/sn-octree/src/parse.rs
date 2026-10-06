use std::fmt;

use crate::{Batch, FORMAT_VERSION, NODE_BYTES, Node, Octree};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset in the input where the problem was found.
    pub offset: usize,
    pub kind: ParseErrorKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The input ended while `needed` more bytes were expected.
    UnexpectedEof {
        needed: usize,
    },
    UnsupportedVersion(i32),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ParseErrorKind::UnexpectedEof { needed } => write!(
                f,
                "unexpected end of data at byte {}: needed {needed} more bytes",
                self.offset
            ),
            ParseErrorKind::UnsupportedVersion(v) => write!(
                f,
                "unsupported octree format version {v} (expected {FORMAT_VERSION})"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

impl Batch {
    /// Parses a whole `.optoctrees` file.
    ///
    /// Only the byte layout is checked here; use [`Octree::validate`] to check
    /// that each octree's node links form a proper tree.
    pub fn parse(bytes: &[u8]) -> Result<Batch, ParseError> {
        let mut r = Reader { bytes, pos: 0 };
        let version = i32::from_le_bytes(r.array()?);
        if version != FORMAT_VERSION {
            return Err(ParseError {
                offset: 0,
                kind: ParseErrorKind::UnsupportedVersion(version),
            });
        }

        let mut octrees = Vec::new();
        while r.pos < bytes.len() {
            let count = usize::from(u16::from_le_bytes(r.array()?));
            let raw = r.take(count * NODE_BYTES)?;
            let nodes = raw
                .chunks_exact(NODE_BYTES)
                .map(|c| Node {
                    ty: c[0],
                    density: c[1],
                    first_child: u16::from_le_bytes([c[2], c[3]]),
                })
                .collect();
            octrees.push(Octree { nodes });
        }
        Ok(Batch { version, octrees })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        let remaining = self.bytes.len() - self.pos;
        if remaining < n {
            return Err(ParseError {
                offset: self.pos,
                kind: ParseErrorKind::UnexpectedEof {
                    needed: n - remaining,
                },
            });
        }
        let out = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], ParseError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
}
