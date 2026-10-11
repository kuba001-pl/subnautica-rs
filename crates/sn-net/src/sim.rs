//! An in-memory network on simulated time with seeded impairments:
//! latency, jitter (which also reorders), loss and duplication. Tests and
//! the `--net-sim` options drive it; every run with the same seed and the
//! same calls is the same.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;

use crate::transport::Transport;

/// How the network treats every datagram (one way).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impairment {
    /// Mean one-way delay, microseconds.
    pub latency_us: u64,
    /// The delay varies uniformly by up to this much either way.
    pub jitter_us: u64,
    /// Chance that a datagram is lost.
    pub loss: f64,
    /// Chance that a datagram arrives twice (each copy with its own delay).
    pub duplicate: f64,
}

impl Impairment {
    pub const NONE: Impairment = Impairment {
        latency_us: 0,
        jitter_us: 0,
        loss: 0.0,
        duplicate: 0.0,
    };

    /// The M10 plan's presets.
    pub fn preset(name: &str) -> Option<Impairment> {
        Some(match name {
            "lan" => Impairment {
                latency_us: 1_000,
                ..Impairment::NONE
            },
            "internet" => Impairment {
                latency_us: 40_000,
                jitter_us: 10_000,
                loss: 0.01,
                duplicate: 0.0,
            },
            "bad" => Impairment {
                latency_us: 150_000,
                jitter_us: 50_000,
                loss: 0.05,
                duplicate: 0.01,
            },
            _ => return None,
        })
    }

    pub const PRESETS: [&'static str; 3] = ["lan", "internet", "bad"];
}

/// Counters of the whole network.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimStats {
    pub sent: u64,
    pub lost: u64,
    pub duplicated: u64,
    pub delivered: u64,
    /// Datagrams to an address nobody has.
    pub undeliverable: u64,
}

/// Datagrams waiting for one address: (arrival time, send order) →
/// (sender, bytes).
type Queue = BTreeMap<(u64, u64), (SocketAddr, Vec<u8>)>;

struct Inner {
    now: u64,
    rng: u64,
    impairment: Impairment,
    queues: HashMap<SocketAddr, Queue>,
    order: u64,
    stats: SimStats,
}

impl Inner {
    fn next(&mut self) -> u64 {
        // splitmix64
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn delay(&mut self) -> u64 {
        let i = self.impairment;
        let j = i.jitter_us as f64 * (self.unit() * 2.0 - 1.0);
        (i.latency_us as f64 + j).max(0.0).round() as u64
    }
}

/// The network; clone it to share it.
#[derive(Clone)]
pub struct SimNetwork {
    inner: Rc<RefCell<Inner>>,
}

impl SimNetwork {
    pub fn new(seed: u64, impairment: Impairment) -> SimNetwork {
        SimNetwork {
            inner: Rc::new(RefCell::new(Inner {
                now: 0,
                rng: seed,
                impairment,
                queues: HashMap::new(),
                order: 0,
                stats: SimStats::default(),
            })),
        }
    }

    /// A socket at `addr` (any address; nothing is bound for real).
    pub fn socket(&self, addr: SocketAddr) -> SimSocket {
        self.inner.borrow_mut().queues.entry(addr).or_default();
        SimSocket {
            net: self.clone(),
            addr,
        }
    }

    /// Moves the clock; datagrams due by then can be received.
    pub fn set_time(&self, now: u64) {
        let mut inner = self.inner.borrow_mut();
        inner.now = inner.now.max(now);
    }

    pub fn now(&self) -> u64 {
        self.inner.borrow().now
    }

    /// When the next datagram arrives anywhere, after the current time
    /// (one waiting already, unread, does not count: a socket nobody reads
    /// must not stop the clock).
    pub fn next_arrival(&self) -> Option<u64> {
        let inner = self.inner.borrow();
        let now = inner.now;
        inner
            .queues
            .values()
            .filter_map(|q| q.range((now + 1, 0)..).next().map(|(k, _)| k.0))
            .min()
    }

    pub fn set_impairment(&self, impairment: Impairment) {
        self.inner.borrow_mut().impairment = impairment;
    }

    pub fn stats(&self) -> SimStats {
        self.inner.borrow().stats
    }
}

/// One address on a [`SimNetwork`].
pub struct SimSocket {
    net: SimNetwork,
    addr: SocketAddr,
}

impl Transport for SimSocket {
    fn send(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()> {
        let mut inner = self.net.inner.borrow_mut();
        inner.stats.sent += 1;
        let loss = inner.impairment.loss;
        if inner.unit() < loss {
            inner.stats.lost += 1;
            return Ok(());
        }
        let duplicate = inner.impairment.duplicate;
        let copies = if inner.unit() < duplicate {
            inner.stats.duplicated += 1;
            2
        } else {
            1
        };
        if !inner.queues.contains_key(&to) {
            inner.stats.undeliverable += 1;
            return Ok(());
        }
        for _ in 0..copies {
            let at = inner.now + inner.delay();
            let order = inner.order;
            inner.order += 1;
            if let Some(q) = inner.queues.get_mut(&to) {
                q.insert((at, order), (self.addr, data.to_vec()));
            }
        }
        Ok(())
    }

    fn recv(&mut self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        let mut inner = self.net.inner.borrow_mut();
        let now = inner.now;
        let Some(q) = inner.queues.get_mut(&self.addr) else {
            return Ok(None);
        };
        let Some(&key) = q.keys().next().filter(|k| k.0 <= now) else {
            return Ok(None);
        };
        let Some((from, data)) = q.remove(&key) else {
            return Ok(None);
        };
        // As UDP: a datagram longer than the buffer is cut.
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        inner.stats.delivered += 1;
        Ok(Some((n, from)))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([10, 0, 0, 1], port))
    }

    #[test]
    fn delays_losses_and_duplicates_follow_the_settings() {
        let i = Impairment::preset("bad").unwrap();
        let net = SimNetwork::new(7, i);
        let mut a = net.socket(addr(1));
        let mut b = net.socket(addr(2));
        let n = 20_000;
        for k in 0..n {
            a.send(addr(2), &(k as u32).to_le_bytes()).unwrap();
        }
        net.set_time(i.latency_us - i.jitter_us - 1);
        let mut buf = [0; 8];
        assert!(b.recv(&mut buf).unwrap().is_none());
        net.set_time(i.latency_us + i.jitter_us);
        let mut got = Vec::new();
        while let Some((len, from)) = b.recv(&mut buf).unwrap() {
            assert_eq!((len, from), (4, addr(1)));
            got.push(u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]));
        }
        let s = net.stats();
        assert_eq!(got.len() as u64, s.delivered);
        assert_eq!(s.sent, n);
        assert_eq!(s.delivered, n - s.lost + s.duplicated);
        let loss = s.lost as f64 / n as f64;
        let dup = s.duplicated as f64 / n as f64;
        assert!((loss - 0.05).abs() < 0.01, "{loss}");
        assert!((dup - 0.01).abs() < 0.005, "{dup}");
        // Jitter reorders.
        assert!(got.windows(2).any(|w| w[1] < w[0]));
    }
}
