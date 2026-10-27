//! Opt-in real-install restart proof. Only logs in out/; saves in system temp.
use std::fs::{self, File};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use sn_assets::{Assets, CollisionLoader, TargetKind};
use sn_install::GameData;
use sn_net::{Config, Endpoint, Event, PeerId, UdpTransport, random_seed};
use sn_protocol::*;
use sn_session::{Channel, Client, ClientConfig};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Bot {
    endpoint: Endpoint<UdpTransport>,
    peer: PeerId,
    client: Client,
    start: Instant,
    leaving: bool,
}
impl Bot {
    fn new(addr: SocketAddr, config: ClientConfig) -> Self {
        let mut endpoint = Endpoint::new(
            UdpTransport::bind("127.0.0.1:0").unwrap(),
            Config::client(random_seed()),
        );
        let peer = endpoint.connect(addr, 0);
        Self {
            endpoint,
            peer,
            client: Client::new(config, 0),
            start: Instant::now(),
            leaving: false,
        }
    }
    fn pump(&mut self) {
        let now = self.start.elapsed().as_micros() as u64;
        self.endpoint.update(now);
        while let Some(e) = self.endpoint.poll_event() {
            match e {
                Event::Reliable(_, b) | Event::Latest(_, _, b) | Event::Unordered(_, b) => {
                    self.client.received(&b, now)
                }
                _ => {}
            }
        }
        self.client.update(now);
        for (channel, message) in self.client.take_outgoing() {
            // After `Bye` the server closes the peer; late sends are moot.
            let sent = match channel {
                Channel::Reliable => self.endpoint.send_reliable(self.peer, message.encode()),
                Channel::Latest(k) => self.endpoint.send_latest(self.peer, k, message.encode()),
                Channel::Unordered => self.endpoint.send_unordered(self.peer, message.encode()),
            };
            if let Err(e) = sent {
                assert!(self.leaving, "send failed: {e:?}");
            }
        }
    }
    fn until(&mut self, predicate: impl Fn(&Client) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !predicate(&self.client) {
            assert!(Instant::now() < deadline, "scripted client timed out");
            self.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn bye(&mut self) {
        self.leaving = true;
        self.client.bye();
    }
    /// Pumps for 300 ms (lets late events and `Bye` get through).
    fn settle(&mut self) {
        let until = Instant::now() + Duration::from_millis(300);
        while Instant::now() < until {
            self.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn send(&mut self, message: ClientMessage) {
        self.endpoint
            .send_reliable(self.peer, message.encode())
            .unwrap();
    }
    fn request(&mut self, request: Request) {
        let before = self.client.world().unwrap().events;
        self.client.request(request);
        self.until(|c| c.world().is_some_and(|w| w.events > before));
    }
}

#[test]
#[ignore = "reads SUBNAUTICA_DIR; starts our dedicated server twice on loopback"]
fn real_server_restart_restores_items_and_player() {
    let Some(game_dir) = std::env::var_os("SUBNAUTICA_DIR") else {
        eprintln!("SUBNAUTICA_DIR unset; skipped");
        return;
    };
    let game = GameData::locate(Some(game_dir.clone().into())).unwrap();
    let assets = Assets::index(&game).unwrap();
    let rules = sn_assets::rules(&game, &assets).unwrap();
    let (pod, _) = sn_assets::lifepod_point(&assets, 1).unwrap();
    let mut loader = CollisionLoader::new(&game, Some(1)).unwrap();
    loader.set_item_rules(rules.items.clone());
    let mut targets = Vec::new();
    for batch in loader.batches_around(pod, 40.) {
        for t in loader.objects(batch).unwrap().targets {
            if matches!(t.kind, TargetKind::Pickup)
                && rules
                    .items
                    .pickups
                    .get(&t.class)
                    .is_some_and(|p| p.pickupable && rules.items.techs.size(p.tech) == (1, 1))
            {
                targets.push(t);
            }
        }
        if targets.len() >= 2 {
            break;
        }
    }
    assert!(targets.len() >= 2);
    let temp = Temp(std::env::temp_dir().join(format!("sn-rs-real-save-{}", random_seed())));
    fs::create_dir_all(&temp.0).unwrap();
    let logs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out/m9f");
    fs::create_dir_all(&logs).unwrap();
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let launch = |phase: &str| {
        let log = File::create(logs.join(format!("{phase}-server.log"))).unwrap();
        Process(
            Command::new(env!("CARGO_BIN_EXE_sn-server"))
                .args([
                    "--bind",
                    "127.0.0.1",
                    "--port",
                    &port.to_string(),
                    "--save-dir",
                    temp.0.to_str().unwrap(),
                    "--autosave",
                    "1",
                    "--run-for",
                    "8",
                    "--report",
                    "2",
                ])
                .env("SUBNAUTICA_DIR", &game_dir)
                .stdout(Stdio::from(log.try_clone().unwrap()))
                .stderr(Stdio::from(log))
                .spawn()
                .unwrap(),
        )
    };
    let client_config = || ClientConfig {
        name: "persistence-script".into(),
        build: rules.build.clone(),
        fingerprint: rules.fingerprint,
        password: None,
        tick_us: rules.tick_us,
        max_speed: rules.max_speed,
        rules: rules.items.clone(),
    };
    let join = |phase: &str| {
        // Server startup reads the install before binding. Wait for its log,
        // rather than connecting to a socket which does not exist yet.
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let ready = fs::read_to_string(logs.join(format!("{phase}-server.log")))
                .is_ok_and(|s| s.contains("listening on UDP"));
            if ready {
                break;
            }
            assert!(Instant::now() < deadline, "server did not listen");
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut bot = Bot::new(addr, client_config());
        bot.until(Client::loaded);
        bot
    };
    let mut process = launch("first");
    let mut bot = join("first");
    bot.client.ready();
    bot.pump();
    let player = bot.client.player().unwrap();
    let mut pose = PlayerState {
        tick: Tick(
            bot.client
                .server_tick(bot.start.elapsed().as_micros() as u64)
                .unwrap_or(0.) as u32,
        ),
        position: pod,
        velocity: [0; 3],
        yaw: 1234,
        pitch: -321,
        rotation: None,
        walking: true,
        swimming: false,
        grounded: true,
        walk_grounded: true,
        in_pod: true,
        cinematic: false,
        controls: true,
        life: Life::Alive,
        anim: AnimState::default(),
    };
    bot.send(ClientMessage::States(vec![pose]));
    let supply = bot
        .client
        .world()
        .unwrap()
        .items
        .locker
        .items()
        .next()
        .unwrap()
        .id
        .0 as u32;
    bot.request(Request::Move {
        item: ItemId(supply),
        to: Store::Grid,
        target: MoveTarget::Store,
    });
    pose.tick.0 += 1;
    pose.in_pod = false;
    pose.walking = false;
    pose.swimming = true;
    pose.position = targets[0].at.position;
    bot.send(ClientMessage::States(vec![pose]));
    let vitals = PlayerVitals {
        health: 78.25,
        oxygen: 37.5,
    };
    bot.send(ClientMessage::Vitals(vitals));
    let drop_id = bot.client.world().unwrap().items.next_item + 1;
    for t in targets.iter().take(2) {
        bot.request(Request::PickUp {
            key: t.key.clone(),
            class: t.class.clone(),
        });
    }
    bot.request(Request::Drop {
        item: ItemId(drop_id),
        at: DropAt {
            point: pose.position,
            rotation: [0, 0, 0, 32767],
        },
    });
    // Events can arrive after the request's own counter moved; wait for the
    // drop itself, then settle so the snapshot matches the server's.
    bot.until(|c| c.world().is_some_and(|w| w.items.objects.len() == 1));
    bot.settle();
    let expected = bot.client.world().unwrap().items.clone();
    let hash = Fnv64::new().bytes(&expected.to_bytes()).finish();
    println!(
        "before: player {}, inventory {}, locker {}, removed {}, dropped {}, item hash {hash:016x}, pose {:?}, health {}, oxygen {}",
        player.0,
        expected.players[&player].inventory.container.len(),
        expected.locker.len(),
        expected.gone.len(),
        expected.objects.len(),
        pose.position,
        vitals.health,
        vitals.oxygen
    );
    bot.bye();
    bot.settle();
    drop(bot);
    assert!(process.0.wait().unwrap().success());
    drop(process);
    let mut process = launch("restart");
    let mut bot = join("restart");
    assert_eq!(bot.client.player(), Some(player));
    assert_eq!(bot.client.world().unwrap().items, expected);
    assert_eq!(bot.client.take_restore(), Some((Some(pose), Some(vitals))));
    println!(
        "restart: same player/inventory/locker/removed/dropped/pose/vitals; item hash {hash:016x}; 0 differences"
    );
    bot.bye();
    bot.settle();
    drop(bot);
    assert!(process.0.wait().unwrap().success());
}
