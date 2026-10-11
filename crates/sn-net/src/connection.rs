//! One connection's state machine, driven by an explicit clock
//! (microseconds), so tests run on simulated time: packet sequence
//! numbers and acks, the reliable-ordered channel, the latest-only
//! channel, the round-trip estimate and counters. No sockets here.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::packet::{Acks, DataWriter, Frame, MAX_MESSAGE};

/// A packet that acks a reliable frame goes out at most this long after
/// it arrived, even with nothing else to send.
pub const MAX_ACK_DELAY_US: u64 = 10_000;
/// With nothing to send for this long, an empty packet keeps the
/// connection (and the peer's timeout) alive.
pub const KEEPALIVE_US: u64 = 250_000;
/// Resend timeout before the first round-trip sample, and its bounds.
const INITIAL_RTO_US: u64 = 250_000;
const MIN_RTO_US: u64 = 50_000;
const MAX_RTO_US: u64 = 2_000_000;
/// Sent packets remembered for acks.
const SENT_HISTORY: usize = 1024;
/// Packets with reliable frames in flight (sent, not acked, younger than
/// the resend timeout) at most: an ack covers the newest packet and the 32
/// before it, so a longer burst could never be fully acknowledged and
/// would be resent for nothing.
const MAX_RELIABLE_IN_FLIGHT: usize = 32;
/// Unacknowledged reliable messages a connection holds before
/// `send_reliable` refuses more.
pub const MAX_PENDING_RELIABLE: usize = 4096;
/// Unordered messages waiting to be sent at most.
pub const MAX_PENDING_UNORDERED: usize = 256;
/// How far ahead of the next expected id a reliable message may be.
const RELIABLE_WINDOW: u32 = 8192;

/// Something a connection received, in arrival order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    Reliable(Vec<u8>),
    Latest {
        key: u16,
        data: Vec<u8>,
    },
    /// Unreliable, any order (duplicate datagrams are already dropped).
    Unordered(Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    UnknownPeer,
    TooLarge { len: usize },
    QueueFull,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::UnknownPeer => write!(f, "no such peer"),
            SendError::TooLarge { len } => {
                write!(f, "message of {len} bytes, more than {MAX_MESSAGE}")
            }
            SendError::QueueFull => write!(
                f,
                "{MAX_PENDING_RELIABLE} reliable messages are waiting for acks"
            ),
        }
    }
}

impl std::error::Error for SendError {}

/// Counters of one connection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    /// Packets received a second time (dropped whole).
    pub duplicate_packets: u64,
    /// Our packets the peer acknowledged.
    pub packets_acked: u64,
    pub reliable_sent: u64,
    /// Reliable frames sent again after their resend timeout.
    pub reliable_resent: u64,
    pub reliable_delivered: u64,
    /// Reliable frames received again (dropped).
    pub reliable_duplicates: u64,
    pub latest_sent: u64,
    pub latest_delivered: u64,
    /// Latest-only frames older than one already delivered (dropped).
    pub latest_stale: u64,
    pub unordered_sent: u64,
    pub unordered_delivered: u64,
    /// Unordered messages dropped before sending (queue full).
    pub unordered_dropped: u64,
    pub rtt_samples: u64,
}

struct SentPacket {
    seq: u32,
    at: u64,
    reliable: Vec<u32>,
    acked: bool,
}

struct Pending {
    data: Vec<u8>,
    last_sent: Option<u64>,
    sends: u32,
}

pub(crate) struct Connection {
    pub conn_id: u32,
    // Sending.
    next_seq: u32,
    sent: VecDeque<SentPacket>,
    last_send_at: Option<u64>,
    next_reliable_id: u32,
    pending: BTreeMap<u32, Pending>,
    latest_out: BTreeMap<u16, Vec<u8>>,
    unordered_out: VecDeque<Vec<u8>>,
    // Receiving.
    highest: Option<u32>,
    bits: u32,
    highest_at: u64,
    ack_due_since: Option<u64>,
    expected_id: u32,
    early: BTreeMap<u32, Vec<u8>>,
    latest_in: HashMap<u16, u32>,
    pub delivered: VecDeque<Delivery>,
    pub last_received_at: u64,
    // Round trip (RFC 6298's estimator), microseconds.
    srtt: Option<f64>,
    rttvar: f64,
    pub stats: Stats,
}

impl Connection {
    pub fn new(conn_id: u32, now: u64) -> Connection {
        Connection {
            conn_id,
            next_seq: 0,
            sent: VecDeque::new(),
            last_send_at: None,
            next_reliable_id: 0,
            pending: BTreeMap::new(),
            latest_out: BTreeMap::new(),
            unordered_out: VecDeque::new(),
            highest: None,
            bits: 0,
            highest_at: now,
            ack_due_since: None,
            expected_id: 0,
            early: BTreeMap::new(),
            latest_in: HashMap::new(),
            delivered: VecDeque::new(),
            last_received_at: now,
            srtt: None,
            rttvar: 0.0,
            stats: Stats::default(),
        }
    }

    /// The smoothed round trip, microseconds.
    pub fn rtt_us(&self) -> Option<f64> {
        self.srtt
    }

    fn rto(&self) -> u64 {
        match self.srtt {
            None => INITIAL_RTO_US,
            Some(srtt) => {
                let rto = srtt + (4.0 * self.rttvar).max(10_000.0) + MAX_ACK_DELAY_US as f64;
                (rto as u64).clamp(MIN_RTO_US, MAX_RTO_US)
            }
        }
    }

    pub fn send_reliable(&mut self, data: Vec<u8>) -> Result<(), SendError> {
        if data.len() > MAX_MESSAGE {
            return Err(SendError::TooLarge { len: data.len() });
        }
        if self.pending.len() >= MAX_PENDING_RELIABLE {
            return Err(SendError::QueueFull);
        }
        let id = self.next_reliable_id;
        self.next_reliable_id = id.wrapping_add(1);
        self.pending.insert(
            id,
            Pending {
                data,
                last_sent: None,
                sends: 0,
            },
        );
        Ok(())
    }

    /// Replaces whatever is waiting under `key`: only the newest goes.
    pub fn send_latest(&mut self, key: u16, data: Vec<u8>) -> Result<(), SendError> {
        if data.len() > MAX_MESSAGE {
            return Err(SendError::TooLarge { len: data.len() });
        }
        self.latest_out.insert(key, data);
        Ok(())
    }

    /// Sent once in the next packet; may be lost, and arrive in any order.
    /// If more than [`MAX_PENDING_UNORDERED`] wait, the oldest is dropped.
    pub fn send_unordered(&mut self, data: Vec<u8>) -> Result<(), SendError> {
        if data.len() > MAX_MESSAGE {
            return Err(SendError::TooLarge { len: data.len() });
        }
        if self.unordered_out.len() >= MAX_PENDING_UNORDERED {
            self.unordered_out.pop_front();
            self.stats.unordered_dropped += 1;
        }
        self.unordered_out.push_back(data);
        Ok(())
    }

    fn acks(&self, now: u64) -> Option<Acks> {
        self.highest.map(|ack| Acks {
            ack,
            bits: self.bits,
            delay_us: now.saturating_sub(self.highest_at).min(u64::from(u32::MAX)) as u32,
        })
    }

    /// The packets to send now: everything queued, reliable messages whose
    /// resend timeout passed, and an ack or keep-alive if one is due.
    pub fn poll(&mut self, now: u64) -> Vec<Vec<u8>> {
        let rto = self.rto();
        let due: Vec<u32> = self
            .pending
            .iter()
            .filter(|(_, p)| match p.last_sent {
                None => true,
                Some(t) => {
                    let backoff = rto
                        .saturating_mul(1 << (p.sends - 1).min(5))
                        .min(MAX_RTO_US);
                    now.saturating_sub(t) >= backoff
                }
            })
            .map(|(&id, _)| id)
            .collect();
        let mut room = MAX_RELIABLE_IN_FLIGHT.saturating_sub(
            self.sent
                .iter()
                .filter(|p| !p.acked && !p.reliable.is_empty() && now.saturating_sub(p.at) < rto)
                .count(),
        );
        let mut due = due.into_iter().peekable();
        let mut latest = std::mem::take(&mut self.latest_out).into_iter().peekable();
        let mut unordered = std::mem::take(&mut self.unordered_out)
            .into_iter()
            .peekable();
        let mut out = Vec::new();
        loop {
            let mut w = DataWriter::new(self.conn_id, self.next_seq, self.acks(now));
            let mut reliable = Vec::new();
            while let Some((key, data)) = latest.peek() {
                if !w.latest(*key, data) {
                    break;
                }
                latest.next();
                self.stats.latest_sent += 1;
            }
            while let Some(data) = unordered.peek() {
                if !w.unordered(data) {
                    break;
                }
                unordered.next();
                self.stats.unordered_sent += 1;
            }
            while let Some(&id) = due.peek().filter(|_| room > 0) {
                let Some(p) = self.pending.get_mut(&id) else {
                    due.next();
                    continue;
                };
                if !w.reliable(id, &p.data) {
                    break;
                }
                due.next();
                if p.last_sent.is_some() {
                    self.stats.reliable_resent += 1;
                }
                self.stats.reliable_sent += 1;
                p.last_sent = Some(now);
                p.sends += 1;
                reliable.push(id);
            }
            let ack_due = self
                .ack_due_since
                .is_some_and(|t| now.saturating_sub(t) >= MAX_ACK_DELAY_US);
            let idle = self
                .last_send_at
                .is_none_or(|t| now.saturating_sub(t) >= KEEPALIVE_US);
            if w.frames == 0 && !(out.is_empty() && (ack_due || idle)) {
                break;
            }
            if !reliable.is_empty() {
                room -= 1;
            }
            self.record(now, reliable, w.out.len());
            out.push(w.out);
            if latest.peek().is_none()
                && unordered.peek().is_none()
                && (room == 0 || due.peek().is_none())
            {
                break;
            }
        }
        // Anything that did not fit waits for the next poll.
        self.latest_out.extend(latest);
        self.unordered_out.extend(unordered);
        out
    }

    fn record(&mut self, now: u64, reliable: Vec<u32>, len: usize) {
        self.sent.push_back(SentPacket {
            seq: self.next_seq,
            at: now,
            reliable,
            acked: false,
        });
        if self.sent.len() > SENT_HISTORY {
            self.sent.pop_front();
        }
        self.next_seq = self.next_seq.wrapping_add(1);
        self.last_send_at = Some(now);
        self.ack_due_since = None;
        self.stats.packets_sent += 1;
        self.stats.bytes_sent += len as u64;
    }

    /// Whether `seq` was received already (only knowable within the last
    /// 33 packets).
    fn seen(&self, seq: u32) -> bool {
        match self.highest {
            None => false,
            Some(h) if seq == h => true,
            Some(h) if seq < h && h - seq <= 32 => self.bits & (1 << (h - seq - 1)) != 0,
            _ => false,
        }
    }

    fn mark_received(&mut self, seq: u32, now: u64) {
        match self.highest {
            Some(h) if seq < h => {
                let d = h - seq;
                if d <= 32 {
                    self.bits |= 1 << (d - 1);
                }
            }
            Some(h) => {
                let shift = seq - h;
                self.bits = if shift > 32 {
                    0
                } else {
                    ((u64::from(self.bits) << shift) | (1u64 << (shift - 1))) as u32
                };
                self.highest = Some(seq);
                self.highest_at = now;
            }
            None => {
                self.highest = Some(seq);
                self.highest_at = now;
            }
        }
    }

    fn on_ack(&mut self, seq: u32, now: u64, delay_us: Option<u32>) {
        let Some(front) = self.sent.front().map(|p| p.seq) else {
            return;
        };
        let Some(i) = seq.checked_sub(front).map(|i| i as usize) else {
            return;
        };
        let Some(p) = self.sent.get_mut(i) else {
            return;
        };
        if p.acked || p.seq != seq {
            return;
        }
        p.acked = true;
        self.stats.packets_acked += 1;
        for id in std::mem::take(&mut p.reliable) {
            self.pending.remove(&id);
        }
        if let Some(delay) = delay_us {
            let elapsed = now.saturating_sub(p.at);
            if u64::from(delay) <= elapsed {
                self.rtt_sample((elapsed - u64::from(delay)) as f64);
            }
        }
    }

    fn rtt_sample(&mut self, r: f64) {
        self.stats.rtt_samples += 1;
        match self.srtt {
            None => {
                self.srtt = Some(r);
                self.rttvar = r / 2.0;
            }
            Some(s) => {
                self.rttvar = 0.75 * self.rttvar + 0.25 * (s - r).abs();
                self.srtt = Some(0.875 * s + 0.125 * r);
            }
        }
    }

    /// A data packet of this connection arrived.
    pub fn receive(
        &mut self,
        now: u64,
        len: usize,
        seq: u32,
        acks: Option<Acks>,
        frames: &[Frame<'_>],
    ) {
        if self.seen(seq) {
            self.stats.duplicate_packets += 1;
            return;
        }
        self.mark_received(seq, now);
        self.last_received_at = now;
        self.stats.packets_received += 1;
        self.stats.bytes_received += len as u64;
        if let Some(a) = acks {
            self.on_ack(a.ack, now, Some(a.delay_us));
            for i in 0..32 {
                if a.bits & (1 << i) != 0
                    && let Some(s) = a.ack.checked_sub(i + 1)
                {
                    self.on_ack(s, now, None);
                }
            }
        }
        for frame in frames {
            match *frame {
                Frame::Reliable { id, data } => {
                    self.ack_due_since.get_or_insert(now);
                    let ahead = id.wrapping_sub(self.expected_id);
                    if ahead >= RELIABLE_WINDOW || self.early.contains_key(&id) {
                        // Behind the window: delivered already.
                        self.stats.reliable_duplicates += 1;
                        continue;
                    }
                    self.early.insert(id, data.to_vec());
                    while let Some(d) = self.early.remove(&self.expected_id) {
                        self.delivered.push_back(Delivery::Reliable(d));
                        self.stats.reliable_delivered += 1;
                        self.expected_id = self.expected_id.wrapping_add(1);
                    }
                }
                Frame::Latest { key, data } => {
                    let newer = self.latest_in.get(&key).is_none_or(|&s| seq > s);
                    if newer {
                        self.latest_in.insert(key, seq);
                        self.delivered.push_back(Delivery::Latest {
                            key,
                            data: data.to_vec(),
                        });
                        self.stats.latest_delivered += 1;
                    } else {
                        self.stats.latest_stale += 1;
                    }
                }
                Frame::Unordered { data } => {
                    self.delivered.push_back(Delivery::Unordered(data.to_vec()));
                    self.stats.unordered_delivered += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::{Packet, decode};

    fn feed(to: &mut Connection, now: u64, packets: &[Vec<u8>]) {
        for p in packets {
            if let Ok(Packet::Data {
                seq, acks, frames, ..
            }) = decode(p)
            {
                to.receive(now, p.len(), seq, acks, &frames);
            }
        }
    }

    #[test]
    fn ack_bits_track_the_last_33_packets() {
        let mut c = Connection::new(1, 0);
        for seq in [5, 3, 4, 40] {
            c.mark_received(seq, 0);
        }
        assert_eq!(c.highest, Some(40));
        // 40 − 35 = 5 → bit 34 is out of the window; nothing older kept.
        assert_eq!(c.bits, 0);
        c.mark_received(39, 0);
        c.mark_received(8, 0);
        assert_eq!(c.bits, 1 | (1 << 31));
        assert!(c.seen(40) && c.seen(39) && c.seen(8) && !c.seen(9));
    }

    #[test]
    fn reliable_messages_arrive_once_in_order_despite_loss_and_reordering() {
        let (mut a, mut b) = (Connection::new(1, 0), Connection::new(1, 0));
        for i in 0..5u8 {
            a.send_reliable(vec![i]).unwrap();
        }
        // First flight lost entirely.
        let lost = a.poll(0);
        assert_eq!(lost.len(), 1);
        // Before the resend timeout nothing is due, after it all is.
        assert!(a.poll(100_000).is_empty());
        let again = a.poll(INITIAL_RTO_US);
        assert_eq!(a.stats.reliable_resent, 5);
        // Delivered twice (duplicate datagram), then once more as a copy.
        feed(&mut b, INITIAL_RTO_US + 1_000, &again);
        feed(&mut b, INITIAL_RTO_US + 1_000, &again);
        let got: Vec<_> = b.delivered.drain(..).collect();
        assert_eq!(
            got,
            (0..5)
                .map(|i| Delivery::Reliable(vec![i]))
                .collect::<Vec<_>>()
        );
        assert_eq!(b.stats.duplicate_packets, 1);
        // B's packet 10 ms later acks them; A's queue empties and gets a
        // sample.
        let ack = b.poll(INITIAL_RTO_US + 1_000 + MAX_ACK_DELAY_US);
        assert_eq!(ack.len(), 1);
        feed(&mut a, INITIAL_RTO_US + 2_000 + MAX_ACK_DELAY_US, &ack);
        assert!(a.pending.is_empty());
        // Elapsed 12 ms minus B's 10 ms hold = 2 ms.
        assert_eq!(a.rtt_us(), Some(2_000.0));
    }

    #[test]
    fn latest_keeps_only_the_newest_per_key() {
        let (mut a, mut b) = (Connection::new(1, 0), Connection::new(1, 0));
        a.send_latest(1, vec![1]).unwrap();
        a.send_latest(1, vec![2]).unwrap();
        a.send_latest(2, vec![9]).unwrap();
        let first = a.poll(0);
        a.send_latest(1, vec![3]).unwrap();
        let second = a.poll(1);
        // The second packet overtakes the first.
        feed(&mut b, 5, &second);
        feed(&mut b, 6, &first);
        let got: Vec<_> = b.delivered.drain(..).collect();
        assert_eq!(
            got,
            [
                Delivery::Latest {
                    key: 1,
                    data: vec![3]
                },
                Delivery::Latest {
                    key: 2,
                    data: vec![9]
                },
            ]
        );
        assert_eq!(b.stats.latest_stale, 1);
    }

    #[test]
    fn big_queues_split_into_full_packets_and_limits_hold() {
        let mut a = Connection::new(1, 0);
        for _ in 0..10 {
            a.send_reliable(vec![0; 500]).unwrap();
        }
        let p = a.poll(0);
        assert_eq!(p.len(), 5);
        assert!(p.iter().all(|p| p.len() <= crate::MAX_PACKET));
        assert_eq!(
            a.send_reliable(vec![0; MAX_MESSAGE + 1]),
            Err(SendError::TooLarge {
                len: MAX_MESSAGE + 1
            })
        );
        while a.pending.len() < MAX_PENDING_RELIABLE {
            a.send_reliable(vec![]).unwrap();
        }
        assert_eq!(a.send_reliable(vec![]), Err(SendError::QueueFull));
    }

    #[test]
    fn idle_connections_send_keepalives() {
        let mut a = Connection::new(1, 0);
        assert_eq!(a.poll(0).len(), 1);
        assert!(a.poll(KEEPALIVE_US - 1).is_empty());
        assert_eq!(a.poll(KEEPALIVE_US).len(), 1);
    }
}
