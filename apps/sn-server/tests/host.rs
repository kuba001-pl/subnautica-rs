//! M10e: the server's loop over real UDP on 127.0.0.1, with a made-up
//! configuration (no install needed): a client of another game build is
//! refused with its reason, a matching one is welcomed, garbage is
//! survived.

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use sn_net::{Config, Endpoint, Event, Transport, UdpTransport};
use sn_protocol::{AuroraStart, RejectReason, WorldSettings};
use sn_server::Host;
use sn_session::{Channel, Client, ClientConfig, ClientNotice, ServerConfig};

fn server_config() -> ServerConfig {
    ServerConfig {
        build: "10".into(),
        fingerprint: 0xabc,
        password: None,
        max_players: 8,
        settings: WorldSettings {
            slot_seed: Some(1),
            lifepod: Some([-127.7, 0.0, -49.7]),
            aurora: AuroraStart::Countdown { seconds: 4499.6 },
            clock_start: 480.0,
            time_scale: 1.0,
        },
        tick_us: 20_000,
        max_speed: 100.0,
    }
}

/// Connects a client with `build` and runs both sides until it is
/// welcomed or refused (at most 3 s).
fn join(build: &str) -> (Vec<ClientNotice>, Vec<String>) {
    let t = UdpTransport::bind("127.0.0.1:0").unwrap();
    let server_addr = t.local_addr().unwrap();
    let start = Instant::now();
    let now = || start.elapsed().as_micros() as u64;
    let mut host = Host::new(
        Endpoint::new(t, Config::server(8, 1)),
        server_config(),
        now(),
    );
    // Garbage first, from another socket.
    let junk = UdpSocket::bind("127.0.0.1:0").unwrap();
    for i in 0..200u8 {
        junk.send_to(&[i; 37], server_addr).unwrap();
    }
    let mut ep = Endpoint::new(
        UdpTransport::bind("127.0.0.1:0").unwrap(),
        Config::client(2),
    );
    let peer = ep.connect(server_addr, now());
    let mut client = Client::new(
        ClientConfig {
            name: "Ryley".into(),
            build: build.into(),
            fingerprint: 0xabc,
            password: None,
            tick_us: 20_000,
            max_speed: 100.0,
        },
        now(),
    );
    let mut notices = Vec::new();
    let mut log = Vec::new();
    while start.elapsed() < Duration::from_secs(3) && notices.is_empty() {
        log.extend(host.step(now()));
        for (ch, m) in client.take_outgoing() {
            let bytes = m.encode();
            match ch {
                Channel::Reliable => ep.send_reliable(peer, bytes).unwrap(),
                Channel::Latest(k) => ep.send_latest(peer, k, bytes).unwrap(),
                Channel::Unordered => ep.send_unordered(peer, bytes).unwrap(),
            }
        }
        ep.update(now());
        while let Some(e) = ep.poll_event() {
            if let Event::Reliable(_, b) | Event::Latest(_, _, b) | Event::Unordered(_, b) = e {
                client.received(&b, now());
            }
        }
        notices.extend(client.take_notices());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        host.endpoint.stats().bad_packets >= 200,
        "{:?}",
        host.endpoint.stats()
    );
    (notices, log)
}

#[test]
fn another_build_is_refused_with_its_reason() {
    let (notices, log) = join("9");
    println!("client: {notices:?}; server: {log:?}");
    assert_eq!(
        notices,
        [ClientNotice::Rejected(RejectReason::Build {
            server: "10".into()
        })]
    );
    assert!(
        log.iter()
            .any(|l| l.starts_with("rejected 127.0.0.1:") && l.contains("same game build"))
    );
}

#[test]
fn the_same_build_is_welcomed() {
    let (notices, log) = join("10");
    println!("client: {notices:?}; server: {log:?}");
    assert!(matches!(
        notices.as_slice(),
        [ClientNotice::Welcomed { .. }]
    ));
    assert!(
        log.iter()
            .any(|l| l.starts_with("joined: \"Ryley\" as player 0"))
    );
}
