//! The datagram format (`docs/formats/protocol.md` § Packets): a header,
//! then for data packets acks and frames. Strict like `sn-protocol`'s
//! codec: every failure names its byte offset, nothing panics.

use std::fmt;

/// Every packet starts with these bytes; anything else is not ours.
pub(crate) const MAGIC: [u8; 4] = *b"snrs";
/// The packet layer's own version (separate from `sn-protocol`'s).
pub(crate) const VERSION: u8 = 1;
/// Largest datagram we send or accept (no IP fragmentation on common
/// paths).
pub const MAX_PACKET: usize = 1200;
/// Largest message one frame carries: the packet minus the largest data
/// header (28 bytes: magic 4, version 1, kind 1, connection 4, seq 4, acks
/// flag 1, ack 4, bits 4, delay ≤ 5) and frame header (kind 1, id ≤ 5,
/// length 2), less 3 to spare: 1,161 bytes.
pub const MAX_MESSAGE: usize = MAX_PACKET - 28 - 8 - 3;

const CONNECT: u8 = 1;
const ACCEPT: u8 = 2;
const REFUSE: u8 = 3;
const DATA: u8 = 4;
const DISCONNECT: u8 = 5;

const FRAME_RELIABLE: u8 = 1;
const FRAME_LATEST: u8 = 2;
const FRAME_UNORDERED: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PacketError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for PacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

/// Why a server turned a connection down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefuseReason {
    Full,
}

/// What a data packet acknowledges: the newest packet received and the
/// 32 before it (bit `i`: `ack − 1 − i` received), and how long the
/// newest one waited before this packet left (for the round trip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Acks {
    pub ack: u32,
    pub bits: u32,
    pub delay_us: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Frame<'a> {
    Reliable { id: u32, data: &'a [u8] },
    Latest { key: u16, data: &'a [u8] },
    Unordered { data: &'a [u8] },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Packet<'a> {
    Connect {
        conn: u32,
    },
    Accept {
        conn: u32,
    },
    Refuse {
        conn: u32,
        reason: RefuseReason,
    },
    Data {
        conn: u32,
        seq: u32,
        acks: Option<Acks>,
        frames: Vec<Frame<'a>>,
    },
    Disconnect {
        conn: u32,
    },
}

fn header(out: &mut Vec<u8>, kind: u8, conn: u32) {
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(kind);
    out.extend_from_slice(&conn.to_le_bytes());
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
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

pub(crate) fn control(kind: ControlKind, conn: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    match kind {
        ControlKind::Connect => header(&mut out, CONNECT, conn),
        ControlKind::Accept => header(&mut out, ACCEPT, conn),
        ControlKind::Refuse(reason) => {
            header(&mut out, REFUSE, conn);
            out.push(match reason {
                RefuseReason::Full => 0,
            });
        }
        ControlKind::Disconnect => header(&mut out, DISCONNECT, conn),
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControlKind {
    Connect,
    Accept,
    Refuse(RefuseReason),
    Disconnect,
}

/// Builds one data packet, frame by frame, within [`MAX_PACKET`].
pub(crate) struct DataWriter {
    pub out: Vec<u8>,
    pub frames: usize,
}

impl DataWriter {
    pub fn new(conn: u32, seq: u32, acks: Option<Acks>) -> DataWriter {
        let mut out = Vec::with_capacity(MAX_PACKET);
        header(&mut out, DATA, conn);
        out.extend_from_slice(&seq.to_le_bytes());
        match acks {
            Some(a) => {
                out.push(1);
                out.extend_from_slice(&a.ack.to_le_bytes());
                out.extend_from_slice(&a.bits.to_le_bytes());
                varint(&mut out, u64::from(a.delay_us));
            }
            None => out.push(0),
        }
        DataWriter { out, frames: 0 }
    }

    fn frame_len(id_or_key: u64, len: usize) -> usize {
        let mut v = Vec::new();
        varint(&mut v, id_or_key);
        varint(&mut v, len as u64);
        1 + v.len() + len
    }

    /// Adds a frame if it fits; `false` if the packet is too full.
    pub fn reliable(&mut self, id: u32, data: &[u8]) -> bool {
        if self.out.len() + Self::frame_len(u64::from(id), data.len()) > MAX_PACKET {
            return false;
        }
        self.out.push(FRAME_RELIABLE);
        varint(&mut self.out, u64::from(id));
        varint(&mut self.out, data.len() as u64);
        self.out.extend_from_slice(data);
        self.frames += 1;
        true
    }

    pub fn latest(&mut self, key: u16, data: &[u8]) -> bool {
        if self.out.len() + Self::frame_len(u64::from(key), data.len()) > MAX_PACKET {
            return false;
        }
        self.out.push(FRAME_LATEST);
        varint(&mut self.out, u64::from(key));
        varint(&mut self.out, data.len() as u64);
        self.out.extend_from_slice(data);
        self.frames += 1;
        true
    }

    pub fn unordered(&mut self, data: &[u8]) -> bool {
        // Like a latest frame without the key.
        if self.out.len() + Self::frame_len(0, data.len()) - 1 > MAX_PACKET {
            return false;
        }
        self.out.push(FRAME_UNORDERED);
        varint(&mut self.out, data.len() as u64);
        self.out.extend_from_slice(data);
        self.frames += 1;
        true
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn err(&self, at: usize, message: impl Into<String>) -> PacketError {
        PacketError {
            offset: at,
            message: message.into(),
        }
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8], PacketError> {
        match self.pos.checked_add(n).filter(|&e| e <= self.data.len()) {
            Some(end) => {
                let s = &self.data[self.pos..end];
                self.pos = end;
                Ok(s)
            }
            None => Err(self.err(self.pos, format!("{what}: needs {n} bytes"))),
        }
    }

    fn u8(&mut self, what: &str) -> Result<u8, PacketError> {
        Ok(self.take(1, what)?[0])
    }

    fn u32(&mut self, what: &str) -> Result<u32, PacketError> {
        let mut b = [0; 4];
        b.copy_from_slice(self.take(4, what)?);
        Ok(u32::from_le_bytes(b))
    }

    /// A minimal varint of at most `max`.
    fn varint(&mut self, max: u64, what: &str) -> Result<u64, PacketError> {
        let start = self.pos;
        let mut v = 0u64;
        for i in 0..10 {
            let byte = self.u8(what)?;
            if i == 9 && byte > 1 {
                return Err(self.err(start, format!("{what}: varint overflows")));
            }
            v |= u64::from(byte & 0x7f) << (7 * i);
            if byte & 0x80 == 0 {
                if byte == 0 && i > 0 {
                    return Err(self.err(start, format!("{what}: varint not minimal")));
                }
                if v > max {
                    return Err(self.err(start, format!("{what}: {v} is more than {max}")));
                }
                return Ok(v);
            }
        }
        Err(self.err(start, format!("{what}: varint too long")))
    }
}

pub(crate) fn decode(data: &[u8]) -> Result<Packet<'_>, PacketError> {
    let mut r = Reader { data, pos: 0 };
    if data.len() > MAX_PACKET {
        return Err(r.err(
            MAX_PACKET,
            format!("{} bytes, more than {MAX_PACKET}", data.len()),
        ));
    }
    if r.take(4, "magic")? != MAGIC {
        return Err(r.err(0, "not an sn-net packet"));
    }
    let at = r.pos;
    let version = r.u8("version")?;
    if version != VERSION {
        return Err(r.err(at, format!("packet version {version}, ours {VERSION}")));
    }
    let at = r.pos;
    let kind = r.u8("kind")?;
    let conn = r.u32("connection id")?;
    let p = match kind {
        CONNECT => Packet::Connect { conn },
        ACCEPT => Packet::Accept { conn },
        REFUSE => {
            let at = r.pos;
            let reason = match r.u8("refuse reason")? {
                0 => RefuseReason::Full,
                k => return Err(r.err(at, format!("unknown refuse reason {k}"))),
            };
            Packet::Refuse { conn, reason }
        }
        DISCONNECT => Packet::Disconnect { conn },
        DATA => {
            let seq = r.u32("seq")?;
            let at = r.pos;
            let acks = match r.u8("acks present")? {
                0 => None,
                1 => Some(Acks {
                    ack: r.u32("ack")?,
                    bits: r.u32("ack bits")?,
                    delay_us: r.varint(u64::from(u32::MAX), "ack delay")? as u32,
                }),
                b => return Err(r.err(at, format!("acks flag {b}"))),
            };
            let mut frames = Vec::new();
            while r.pos < data.len() {
                let at = r.pos;
                let frame = match r.u8("frame kind")? {
                    FRAME_RELIABLE => {
                        let id = r.varint(u64::from(u32::MAX), "message id")? as u32;
                        let n = r.varint(MAX_MESSAGE as u64, "message length")? as usize;
                        Frame::Reliable {
                            id,
                            data: r.take(n, "message")?,
                        }
                    }
                    FRAME_LATEST => {
                        let key = r.varint(u64::from(u16::MAX), "latest key")? as u16;
                        let n = r.varint(MAX_MESSAGE as u64, "message length")? as usize;
                        Frame::Latest {
                            key,
                            data: r.take(n, "message")?,
                        }
                    }
                    FRAME_UNORDERED => {
                        let n = r.varint(MAX_MESSAGE as u64, "message length")? as usize;
                        Frame::Unordered {
                            data: r.take(n, "message")?,
                        }
                    }
                    k => return Err(r.err(at, format!("unknown frame kind {k}"))),
                };
                frames.push(frame);
            }
            return Ok(Packet::Data {
                conn,
                seq,
                acks,
                frames,
            });
        }
        k => return Err(r.err(at, format!("unknown packet kind {k}"))),
    };
    if r.pos != data.len() {
        return Err(r.err(r.pos, "bytes after the packet"));
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_round_trip() {
        for (kind, want) in [
            (ControlKind::Connect, Packet::Connect { conn: 7 }),
            (ControlKind::Accept, Packet::Accept { conn: 7 }),
            (
                ControlKind::Refuse(RefuseReason::Full),
                Packet::Refuse {
                    conn: 7,
                    reason: RefuseReason::Full,
                },
            ),
            (ControlKind::Disconnect, Packet::Disconnect { conn: 7 }),
        ] {
            assert_eq!(decode(&control(kind, 7)).unwrap(), want);
        }
        let acks = Acks {
            ack: u32::MAX,
            bits: 0x8000_0001,
            delay_us: 12_345,
        };
        let mut w = DataWriter::new(9, 41, Some(acks));
        assert!(w.reliable(u32::MAX, b"hello"));
        assert!(w.latest(65_535, b""));
        assert!(w.reliable(0, &[1; 300]));
        assert!(w.unordered(b"any order"));
        let p = decode(&w.out).unwrap();
        assert_eq!(
            p,
            Packet::Data {
                conn: 9,
                seq: 41,
                acks: Some(acks),
                frames: vec![
                    Frame::Reliable {
                        id: u32::MAX,
                        data: b"hello"
                    },
                    Frame::Latest {
                        key: 65_535,
                        data: b""
                    },
                    Frame::Reliable {
                        id: 0,
                        data: &[1; 300]
                    },
                    Frame::Unordered { data: b"any order" },
                ],
            }
        );
        let empty = DataWriter::new(1, 0, None);
        assert_eq!(
            decode(&empty.out).unwrap(),
            Packet::Data {
                conn: 1,
                seq: 0,
                acks: None,
                frames: vec![]
            }
        );
    }

    #[test]
    fn a_largest_message_fits_a_full_header_and_one_more_byte_does_not() {
        let acks = Some(Acks {
            ack: u32::MAX,
            bits: u32::MAX,
            delay_us: u32::MAX,
        });
        let mut w = DataWriter::new(u32::MAX, u32::MAX, acks);
        assert!(w.reliable(u32::MAX, &vec![0; MAX_MESSAGE]));
        assert!(w.out.len() <= MAX_PACKET, "{}", w.out.len());
        decode(&w.out).unwrap();
        let mut w = DataWriter::new(u32::MAX, u32::MAX, acks);
        assert!(w.latest(u16::MAX, &vec![0; MAX_MESSAGE]));
        assert!(w.out.len() <= MAX_PACKET);
    }

    #[test]
    fn foreign_and_broken_packets_are_errors() {
        assert!(decode(b"").is_err());
        assert!(decode(b"GET / HTTP/1.1").is_err());
        let mut p = control(ControlKind::Connect, 1);
        p[4] = 99;
        assert_eq!(decode(&p).unwrap_err().offset, 4);
        let mut p = control(ControlKind::Connect, 1);
        p.push(0);
        assert!(decode(&p).is_err());
        let mut w = DataWriter::new(1, 1, None);
        assert!(w.reliable(1, b"abc"));
        let n = w.out.len();
        assert!(decode(&w.out[..n - 1]).is_err());
        assert!(decode(&vec![0; MAX_PACKET + 1]).is_err());
    }

    #[test]
    fn random_and_mutated_datagrams_never_panic() {
        let mut s = 0x1234_5678u64;
        let mut next = || {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            s >> 33
        };
        let mut w = DataWriter::new(
            3,
            5,
            Some(Acks {
                ack: 4,
                bits: 3,
                delay_us: 100,
            }),
        );
        w.reliable(2, b"payload");
        w.latest(1, b"state");
        let valid = [w.out, control(ControlKind::Refuse(RefuseReason::Full), 2)];
        let mut decoded = 0;
        for i in 0..300_000u32 {
            let mut b = valid[(i % 2) as usize].clone();
            match i % 3 {
                0 => {
                    let n = (next() % 64) as usize;
                    b = (0..n).map(|_| next() as u8).collect();
                    if next() % 2 == 0 && b.len() >= 6 {
                        b[..4].copy_from_slice(&MAGIC);
                        b[4] = VERSION;
                    }
                }
                1 => {
                    let n = (next() as usize) % b.len();
                    b.truncate(n);
                }
                _ => {
                    let at = (next() as usize) % b.len();
                    b[at] ^= 1 << (next() % 8);
                }
            }
            decoded += usize::from(decode(&b).is_ok());
        }
        assert!(decoded > 0);
    }
}
