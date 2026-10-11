//! A real transport with the simulated network's impairments, on the
//! wall clock (M10f): every datagram sent waits its delay (or is lost, or
//! sent twice) before it leaves, and every datagram received waits its
//! delay before it is handed on. So one end impairs both legs of its own
//! connection, as [`crate::sim::Impairment`] describes one leg. For bots
//! and `--net-sim`; the in-memory [`crate::sim::SimNetwork`] stays the
//! tool for exact, repeatable tests.

use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::time::Instant;

use crate::sim::Impairment;
use crate::transport::Transport;

type Queue = BTreeMap<(u64, u64), (SocketAddr, Vec<u8>)>;

pub struct ImpairedTransport<T: Transport> {
    inner: T,
    impairment: Impairment,
    rng: u64,
    clock: Instant,
    order: u64,
    outgoing: Queue,
    incoming: Queue,
    buf: Vec<u8>,
}

impl<T: Transport> ImpairedTransport<T> {
    pub fn new(inner: T, impairment: Impairment, seed: u64) -> ImpairedTransport<T> {
        ImpairedTransport {
            inner,
            impairment,
            rng: seed,
            clock: Instant::now(),
            order: 0,
            outgoing: BTreeMap::new(),
            incoming: BTreeMap::new(),
            buf: vec![0; 2048],
        }
    }

    fn unit(&mut self) -> f64 {
        // splitmix64
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn now(&self) -> u64 {
        self.clock.elapsed().as_micros() as u64
    }

    /// Queues `data` with this end's impairment: lost, once or twice,
    /// each copy with its own delay.
    fn schedule(&mut self, queue_out: bool, addr: SocketAddr, data: &[u8]) {
        let i = self.impairment;
        if self.unit() < i.loss {
            return;
        }
        let copies = if self.unit() < i.duplicate { 2 } else { 1 };
        let now = self.now();
        for _ in 0..copies {
            let j = i.jitter_us as f64 * (self.unit() * 2.0 - 1.0);
            let at = now + (i.latency_us as f64 + j).max(0.0).round() as u64;
            let key = (at, self.order);
            self.order += 1;
            let q = if queue_out {
                &mut self.outgoing
            } else {
                &mut self.incoming
            };
            q.insert(key, (addr, data.to_vec()));
        }
    }

    /// Sends what is due.
    fn flush(&mut self) -> io::Result<()> {
        let now = self.now();
        while let Some((&key, _)) = self.outgoing.first_key_value() {
            if key.0 > now {
                break;
            }
            if let Some((to, data)) = self.outgoing.remove(&key) {
                self.inner.send(to, &data)?;
            }
        }
        Ok(())
    }
}

impl<T: Transport> Transport for ImpairedTransport<T> {
    fn send(&mut self, to: SocketAddr, data: &[u8]) -> io::Result<()> {
        self.schedule(true, to, data);
        self.flush()
    }

    fn recv(&mut self, buf: &mut [u8]) -> io::Result<Option<(usize, SocketAddr)>> {
        self.flush()?;
        // Everything that arrived goes through the delay line.
        let mut inner_buf = std::mem::take(&mut self.buf);
        while let Some((n, from)) = self.inner.recv(&mut inner_buf)? {
            self.schedule(false, from, &inner_buf[..n]);
        }
        self.buf = inner_buf;
        let now = self.now();
        let Some((&key, _)) = self.incoming.first_key_value() else {
            return Ok(None);
        };
        if key.0 > now {
            return Ok(None);
        }
        let Some((from, data)) = self.incoming.remove(&key) else {
            return Ok(None);
        };
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(Some((n, from)))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}
