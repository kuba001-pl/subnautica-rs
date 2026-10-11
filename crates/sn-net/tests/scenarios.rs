//! M10b's checks on the in-memory network, on simulated time, with the
//! plan's impairment presets; and one run over real UDP on 127.0.0.1.
//! Counters are printed (`cargo test -p sn-net -- --nocapture`).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use sn_net::sim::{Impairment, SimNetwork, SimSocket};
use sn_net::*;

const SERVER: u16 = 1;
const CLIENT: u16 = 2;

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from(([10, 0, 0, 1], port))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn chance(&mut self, p: f64) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
}

/// A reliable message: its number, then filler of a length and value
/// derived from it (so corruption or mix-ups are caught).
fn reliable_message(n: u32) -> Vec<u8> {
    let mut m = n.to_le_bytes().to_vec();
    m.extend(std::iter::repeat_n(n as u8, (n as usize * 7) % 300));
    m
}

fn check_reliable(m: &[u8]) -> u32 {
    let n = u32::from_le_bytes([m[0], m[1], m[2], m[3]]);
    assert_eq!(m, reliable_message(n), "message {n} corrupted");
    n
}

/// One side's traffic and what it saw of the other's.
#[derive(Default)]
struct Side {
    peer: Option<PeerId>,
    reliable_sent: u32,
    reliable_got: Vec<u32>,
    latest_sent: [u32; 2],
    latest_got: HashMap<u16, Vec<u32>>,
    unordered_sent: u32,
    unordered_got: Vec<u32>,
    disconnected: Option<(u64, DisconnectReason)>,
    /// The peer's last packet time, kept while it exists.
    last_heard: Option<u64>,
}

impl Side {
    fn events(&mut self, ep: &mut Endpoint<SimSocket>, now: u64) {
        while let Some(e) = ep.poll_event() {
            match e {
                // Only the first peer is the test's (garbage can make a
                // stray connection on the server).
                Event::Connected(id) => {
                    self.peer.get_or_insert(id);
                }
                Event::Disconnected(id, r) if Some(id) == self.peer => {
                    self.disconnected = Some((now, r));
                }
                Event::Disconnected(..) => {}
                Event::Reliable(_, m) => self.reliable_got.push(check_reliable(&m)),
                Event::Unordered(_, m) => self
                    .unordered_got
                    .push(u32::from_le_bytes([m[0], m[1], m[2], m[3]])),
                Event::Latest(_, key, m) => self
                    .latest_got
                    .entry(key)
                    .or_default()
                    .push(u32::from_le_bytes([m[0], m[1], m[2], m[3]])),
            }
        }
        if let Some(p) = self.peer {
            self.last_heard = ep.last_received_at(p).or(self.last_heard);
        }
    }

    /// The app's step: reliable messages at random (about 20 a second,
    /// plus a burst of 300 at 10 s), latest-only on two keys every 40 ms.
    fn traffic(&mut self, ep: &mut Endpoint<SimSocket>, now: u64, rng: &mut Rng) {
        let Some(p) = self.peer.filter(|_| self.disconnected.is_none()) else {
            return;
        };
        let burst = if now == 10_000_000 { 300 } else { 0 };
        let n = burst + u32::from(rng.chance(0.02));
        for _ in 0..n {
            ep.send_reliable(p, reliable_message(self.reliable_sent))
                .unwrap();
            self.reliable_sent += 1;
        }
        if now % 40_000 == 20_000 {
            ep.send_unordered(p, self.unordered_sent.to_le_bytes().to_vec())
                .unwrap();
            self.unordered_sent += 1;
        }
        if now % 40_000 == 0 {
            for key in 0..2u16 {
                let c = &mut self.latest_sent[usize::from(key)];
                ep.send_latest(p, key, c.to_le_bytes().to_vec()).unwrap();
                *c += 1;
            }
        }
    }
}

struct Run {
    net: SimNetwork,
    server: Endpoint<SimSocket>,
    client: Endpoint<SimSocket>,
    s: Side,
    c: Side,
    now: u64,
    rng: Rng,
}

impl Run {
    fn new(seed: u64, imp: Impairment) -> Run {
        let net = SimNetwork::new(seed, imp);
        let server = Endpoint::new(net.socket(addr(SERVER)), Config::server(8, seed + 1));
        let mut client = Endpoint::new(net.socket(addr(CLIENT)), Config::client(seed + 2));
        client.connect(addr(SERVER), 0);
        Run {
            net,
            server,
            client,
            s: Side::default(),
            c: Side::default(),
            now: 0,
            rng: Rng(seed + 3),
        }
    }

    /// Runs until `end`: the app steps every millisecond and every datagram
    /// is handled at its exact arrival time.
    fn until(&mut self, end: u64, traffic: bool) {
        let mut next_step = self.now.div_ceil(1_000) * 1_000;
        while self.now < end {
            let arrival = self.net.next_arrival().unwrap_or(u64::MAX);
            let t = next_step.min(arrival).min(end);
            self.now = t;
            self.net.set_time(t);
            if t == next_step {
                if traffic {
                    self.s.traffic(&mut self.server, t, &mut self.rng);
                    self.c.traffic(&mut self.client, t, &mut self.rng);
                }
                next_step += 1_000;
            }
            self.server.update(t);
            self.client.update(t);
            self.s.events(&mut self.server, t);
            self.c.events(&mut self.client, t);
        }
    }
}

fn increasing(v: &[u32]) -> bool {
    v.windows(2).all(|w| w[1] > w[0])
}

fn preset_run(name: &str, seed: u64) {
    let imp = Impairment::preset(name).unwrap();
    let mut r = Run::new(seed, imp);
    r.until(60_000_000, true);
    // No new messages: let resends finish.
    r.until(70_000_000, false);

    for (who, side, other) in [("client", &r.c, &r.s), ("server", &r.s, &r.c)] {
        assert!(side.peer.is_some(), "{name}: {who} never connected");
        assert_eq!(side.disconnected, None, "{name}: {who}");
        // Exactly once, in order.
        let want: Vec<u32> = (0..other.reliable_sent).collect();
        assert_eq!(side.reliable_got, want, "{name}: {who}'s reliable messages");
        for key in 0..2u16 {
            let got = side.latest_got.get(&key).map_or(&[][..], Vec::as_slice);
            assert!(
                increasing(got),
                "{name}: {who}'s latest key {key} went back"
            );
        }
        // Unordered: never duplicated, nothing invented; all on lan.
        let mut u = side.unordered_got.clone();
        let in_order = increasing(&u);
        u.sort_unstable();
        let n = u.len();
        u.dedup();
        assert_eq!(u.len(), n, "{name}: {who}'s unordered duplicated");
        assert!(u.iter().all(|&x| x < other.unordered_sent));
        println!(
            "{name} {who}: unordered {n} of {} arrived ({}in order)",
            other.unordered_sent,
            if in_order { "" } else { "not all " }
        );
        if name == "lan" {
            assert_eq!(n as u32, other.unordered_sent);
        }
    }
    let peer_c = r.c.peer.unwrap();
    let peer_s = r.s.peer.unwrap();
    let want_rtt = 2.0 * imp.latency_us as f64;
    for (who, ep, peer) in [("client", &r.client, peer_c), ("server", &r.server, peer_s)] {
        let rtt = ep.rtt_us(peer).unwrap();
        let st = ep.peer_stats(peer).unwrap();
        let other = if who == "client" { &r.s } else { &r.c };
        let latest_sent: u32 = other.latest_sent.iter().sum();
        println!(
            "{name} {who}: rtt {:.2} ms (configured {:.2} ms, error {:+.1} %); packets sent {} received {} (duplicates dropped {}), {} kB sent; reliable sent {} (resent {}) delivered {} (duplicates dropped {}); latest delivered {} of {} the peer sent (stale dropped {}); rtt samples {}",
            rtt / 1000.0,
            want_rtt / 1000.0,
            (rtt / want_rtt - 1.0) * 100.0,
            st.packets_sent,
            st.packets_received,
            st.duplicate_packets,
            st.bytes_sent / 1000,
            st.reliable_sent,
            st.reliable_resent,
            st.reliable_delivered,
            st.reliable_duplicates,
            st.latest_delivered,
            latest_sent,
            st.latest_stale,
            st.rtt_samples
        );
        assert!(
            (rtt - want_rtt).abs() <= 0.1 * want_rtt,
            "{name} {who}: rtt {rtt} µs, configured {want_rtt} µs"
        );
    }
    let n = r.net.stats();
    println!(
        "{name} network: {} datagrams sent, {} lost, {} duplicated, {} delivered; endpoint stats server {:?}, client {:?}",
        n.sent,
        n.lost,
        n.duplicated,
        n.delivered,
        r.server.stats(),
        r.client.stats()
    );
}

#[test]
fn lan_preset() {
    preset_run("lan", 10);
}

#[test]
fn internet_preset() {
    preset_run("internet", 20);
}

#[test]
fn bad_preset() {
    preset_run("bad", 30);
}

#[test]
fn bad_preset_other_seeds() {
    for seed in [31, 32, 33] {
        preset_run("bad", seed);
    }
}

#[test]
fn a_cut_link_times_out_after_five_seconds_on_both_sides() {
    let mut r = Run::new(40, Impairment::preset("internet").unwrap());
    r.until(5_000_000, true);
    r.net.set_impairment(Impairment {
        loss: 1.0,
        ..Impairment::NONE
    });
    r.until(15_000_000, true);
    for (who, side) in [("client", &r.c), ("server", &r.s)] {
        let (at, reason) = side.disconnected.unwrap();
        let last = side.last_heard.unwrap();
        let after = at - last;
        println!("cut link: {who} timed out {after} µs after its last packet");
        assert_eq!(reason, DisconnectReason::TimedOut);
        assert!(
            (TIMEOUT_US..=TIMEOUT_US + 1_000).contains(&after),
            "{who}: {after}"
        );
    }
}

#[test]
fn connecting_to_nobody_times_out() {
    let net = SimNetwork::new(1, Impairment::NONE);
    let mut client = Endpoint::new(net.socket(addr(CLIENT)), Config::client(1));
    let peer = client.connect(addr(99), 0);
    let mut t = 0;
    let mut got = None;
    while got.is_none() && t <= 6_000_000 {
        net.set_time(t);
        client.update(t);
        got = client.poll_event().map(|e| (t, e));
        t += 1_000;
    }
    assert_eq!(
        got,
        Some((
            TIMEOUT_US,
            Event::Disconnected(peer, DisconnectReason::TimedOut)
        ))
    );
    // A request every 100 ms, the first at 0.
    assert_eq!(net.stats().sent, TIMEOUT_US / CONNECT_RESEND_US);
}

#[test]
fn a_full_server_refuses_and_goodbyes_and_restarts_are_reported() {
    let net = SimNetwork::new(5, Impairment::preset("lan").unwrap());
    let mut server = Endpoint::new(net.socket(addr(SERVER)), Config::server(1, 1));
    let mut a = Endpoint::new(net.socket(addr(CLIENT)), Config::client(2));
    let mut b = Endpoint::new(net.socket(addr(3)), Config::client(3));
    a.connect(addr(SERVER), 0);
    let mut events: Vec<(char, Event)> = Vec::new();
    let mut step = |t: u64,
                    server: &mut Endpoint<SimSocket>,
                    a: &mut Endpoint<SimSocket>,
                    b: &mut Endpoint<SimSocket>| {
        net.set_time(t);
        server.update(t);
        a.update(t);
        b.update(t);
        for (who, ep) in [('s', server), ('a', a), ('b', b)] {
            while let Some(e) = ep.poll_event() {
                events.push((who, e));
            }
        }
    };
    for t in (0..50_000).step_by(500) {
        step(t, &mut server, &mut a, &mut b);
    }
    b.connect(addr(SERVER), 50_000);
    for t in (50_000..100_000).step_by(500) {
        step(t, &mut server, &mut a, &mut b);
    }
    // `a` restarts at the same address (a new endpoint, a new id).
    a = Endpoint::new(net.socket(addr(CLIENT)), Config::client(4));
    a.connect(addr(SERVER), 100_000);
    for t in (100_000..150_000).step_by(500) {
        step(t, &mut server, &mut a, &mut b);
    }
    // And says goodbye.
    let p = a.peers().next().unwrap();
    a.disconnect(p);
    for t in (150_000..200_000).step_by(500) {
        step(t, &mut server, &mut a, &mut b);
    }
    let s_events: Vec<&Event> = events.iter().filter(|e| e.0 == 's').map(|e| &e.1).collect();
    assert_eq!(
        s_events,
        [
            &Event::Connected(PeerId(0)),
            &Event::Disconnected(PeerId(0), DisconnectReason::Replaced),
            &Event::Connected(PeerId(1)),
            &Event::Disconnected(PeerId(1), DisconnectReason::Closed),
        ]
    );
    assert!(events.contains(&(
        'b',
        Event::Disconnected(PeerId(0), DisconnectReason::Refused)
    )));
    assert_eq!(server.stats().refused, 1);
}

#[test]
fn garbage_and_foreign_packets_leave_a_connection_alone() {
    let mut r = Run::new(50, Impairment::preset("lan").unwrap());
    let mut noise = r.net.socket(addr(77));
    let mut rng = Rng(9);
    let mut sent = 0;
    for ms in 0..20_000u64 {
        r.until((ms + 1) * 1_000, true);
        // Ten random datagrams a millisecond, some starting like ours.
        for _ in 0..10 {
            let n = (rng.next() % 100) as usize;
            let mut b: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            if rng.chance(0.5) && b.len() > 6 {
                b[..4].copy_from_slice(b"snrs");
                b[4] = 1;
            }
            noise.send(addr(SERVER), &b).unwrap();
            noise.send(addr(CLIENT), &b).unwrap();
            sent += 2;
        }
        let mut sink = [0; 2048];
        while noise.recv(&mut sink).unwrap().is_some() {}
    }
    r.until(25_000_000, false);
    assert_eq!(r.s.disconnected, None);
    assert_eq!(r.c.disconnected, None);
    assert_eq!(r.s.reliable_got, (0..r.c.reliable_sent).collect::<Vec<_>>());
    assert_eq!(r.c.reliable_got, (0..r.s.reliable_sent).collect::<Vec<_>>());
    let (s, c) = (r.server.stats(), r.client.stats());
    println!("garbage: {sent} datagrams; server {s:?}; client {c:?}");
    // A server accepts a random valid connect: those become peers that
    // time out. Everything else is counted, nothing panics.
    assert!(s.bad_packets + c.bad_packets > 0);
}

#[test]
fn udp_on_localhost() {
    let server_t = UdpTransport::bind("127.0.0.1:0").unwrap();
    let server_addr = server_t.local_addr().unwrap();
    let mut server = Endpoint::new(server_t, Config::server(8, random_seed()));
    let mut client = Endpoint::new(
        UdpTransport::bind("127.0.0.1:0").unwrap(),
        Config::client(random_seed()),
    );
    let start = Instant::now();
    let now = || start.elapsed().as_micros() as u64;
    let peer = client.connect(server_addr, now());
    let n = 500u32;
    for i in 0..n {
        client.send_reliable(peer, reliable_message(i)).unwrap();
    }
    let mut got = Vec::new();
    let mut latest = Vec::new();
    let mut server_peer = None;
    let mut sent_latest = 0u32;
    while (got.len() < n as usize || latest.is_empty()) && start.elapsed() < Duration::from_secs(5)
    {
        if let Some(p) = server_peer
            && sent_latest < 100
        {
            server
                .send_latest(p, 0, sent_latest.to_le_bytes().to_vec())
                .unwrap();
            sent_latest += 1;
        }
        server.update(now());
        client.update(now());
        while let Some(e) = server.poll_event() {
            match e {
                Event::Connected(p) => server_peer = Some(p),
                Event::Reliable(_, m) => got.push(check_reliable(&m)),
                other => panic!("server: {other:?}"),
            }
        }
        while let Some(e) = client.poll_event() {
            match e {
                Event::Connected(_) => {}
                Event::Latest(_, 0, m) => latest.push(u32::from_le_bytes([m[0], m[1], m[2], m[3]])),
                other => panic!("client: {other:?}"),
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(got, (0..n).collect::<Vec<_>>());
    assert!(!latest.is_empty() && increasing(&latest));
    let rtt = client.rtt_us(peer).unwrap_or(f64::NAN);
    println!(
        "udp 127.0.0.1: {n} reliable messages in {:.0} ms, {} latest received, rtt {:.3} ms; client {:?}",
        start.elapsed().as_secs_f64() * 1000.0,
        latest.len(),
        rtt / 1000.0,
        client.peer_stats(peer)
    );
}
