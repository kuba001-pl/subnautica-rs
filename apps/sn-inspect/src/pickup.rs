//! `sn-inspect pickup` (M9d6b, `docs/DESIGN.md` "M9d6 plan"): the items
//! headless, end to end, with the player's own rules and collision and an
//! in-process server and client (the same `sn-session` state machines the
//! client runs, over a direct in-memory link on a simulated clock). From
//! the pod: the hand on the locker, every supply taken out; out through
//! the hatch; the nearest pickup picked up; the nearest outcrop hit until
//! it breaks and its resource picked up; one item dropped and picked up
//! again at once; then a death and what it drops. Every request goes
//! through the hand's aim (`sn_assets::hand`) and the panel's rule
//! (`sn_sim::inventory::item_action`) as the client's do; the inventory,
//! the equipment and the cells are logged after each step, and each
//! expected count is checked (the exit code is non-zero if one fails).

use std::process::ExitCode;
use std::time::Instant;

use sn_assets::hand::{AimAction, aim};
use sn_assets::{ItemText, LOCKER_BODY, TargetBody, TargetKind};
use sn_install::GameData;
use sn_protocol::{
    AuroraStart, EntityKey, ItemId, MoveTarget, Request, Store, Tick, WorldSettings,
};
use sn_session::drops::{Draws, death_drops, drop_at};
use sn_session::{
    Action, Client, ClientConfig, ClientNotice, ConnId, ItemsState, Server, ServerConfig,
};
use sn_sim::V3;
use sn_sim::collide::Body;
use sn_sim::inventory::{self as inv, Button, DropPlace};
use sn_sim::player::{Input, hand_target};
use sn_sim::vitals::Vitals;

use crate::Result;
use crate::collision::Streamed;
use crate::coop;
use crate::walk::{self, Run, StepRecord, look_at};

/// The one connection of the direct link.
const LOCAL: ConnId = ConnId(0);

/// A server and a client joined over memory, on a simulated clock that
/// advances one tick per physics step sent.
struct Link {
    server: Server,
    client: Client,
    now: u64,
    tick_us: u64,
    /// Steps of the run sent as states.
    sent: usize,
    refused: Vec<String>,
}

impl Link {
    fn start(rules: &sn_assets::Rules, settings: WorldSettings) -> Result<Link> {
        let config = ServerConfig {
            build: rules.build.clone(),
            fingerprint: rules.fingerprint,
            password: None,
            max_players: 1,
            settings,
            tick_us: rules.tick_us,
            max_speed: rules.max_speed,
            rules: rules.items.clone(),
        };
        let client = Client::new(
            ClientConfig {
                name: "pickup".into(),
                build: rules.build.clone(),
                fingerprint: rules.fingerprint,
                password: None,
                tick_us: rules.tick_us,
                max_speed: rules.max_speed,
                rules: rules.items.clone(),
            },
            0,
        );
        let mut l = Link {
            server: Server::new(config, 0),
            client,
            now: 0,
            tick_us: rules.tick_us,
            sent: 0,
            refused: Vec::new(),
        };
        l.server.connected(LOCAL, 0);
        for _ in 0..500 {
            l.advance();
            if l.client.loaded() && l.client.server_us(l.now).is_some() {
                l.client.ready();
                l.advance();
                return Ok(l);
            }
        }
        Err("the in-process server did not welcome the client".into())
    }

    /// Moves every message across until none is left.
    fn pump(&mut self) {
        loop {
            let mut moved = false;
            for (_, m) in self.client.take_outgoing() {
                moved = true;
                self.server.received(LOCAL, &m.encode(), self.now);
            }
            for a in self.server.take_actions() {
                if let Action::Send { to, message, .. } = a
                    && to == LOCAL
                {
                    moved = true;
                    self.client.received(&message.encode(), self.now);
                }
            }
            if !moved {
                break;
            }
        }
        for n in self.client.take_notices() {
            match n {
                ClientNotice::Refused { req, reason } => {
                    println!("    request {req} REFUSED ({reason:?})");
                    self.refused.push(format!("request {req}: {reason:?}"));
                }
                ClientNotice::Apply(e) => println!("    WORLD EVENT DOES NOT APPLY: {e}"),
                _ => {}
            }
        }
        let _ = self.server.take_notices();
    }

    fn advance(&mut self) {
        self.now += self.tick_us;
        self.server.update(self.now);
        self.client.update(self.now);
        self.pump();
    }

    /// The run's steps not sent yet, one tick each, as the client's
    /// states (the server reads "in the pod" from them).
    fn flush(&mut self, rec: &[StepRecord]) {
        while self.sent < rec.len() {
            self.now += self.tick_us;
            self.server.update(self.now);
            self.client.update(self.now);
            let (state, _) = coop::to_state(&rec[self.sent]);
            for tick in self.client.steps(self.now) {
                let mut s = state;
                s.tick = Tick(tick);
                self.client.push_state(s);
            }
            self.pump();
            self.sent += 1;
        }
    }

    fn request(&mut self, r: Request) -> u32 {
        let what = format!("{r:?}");
        let req = self.client.request(r);
        println!("    request {req}: {}", short(&what));
        self.pump();
        req
    }

    fn items(&self) -> Option<&ItemsState> {
        self.client.world().map(|w| &w.items)
    }
}

fn short(s: &str) -> String {
    match s.char_indices().nth(200) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

fn records<'r>(r: &'r Run) -> &'r [StepRecord] {
    r.record.as_deref().unwrap_or(&[])
}

/// The client's view of the own items, in a log line, and the numbers
/// the checks use: (grid items, slot items, locker items, server-made
/// objects, gone keys).
fn snapshot(text: &ItemText, link: &Link) -> (String, [usize; 5]) {
    let Some(items) = link.items() else {
        return ("no items".into(), [0; 5]);
    };
    let Some(p) = link.client.player().and_then(|me| items.players.get(&me)) else {
        return ("no player".into(), [0; 5]);
    };
    let grid = |c: &inv::Container| -> Vec<String> {
        c.layout()
            .0
            .iter()
            .map(|q| {
                let name = c.get(q.id).map_or_else(String::new, |i| text.tech(i.tech));
                format!(
                    "{name} #{} ({}, {}) {}x{}",
                    q.id.0, q.x, q.y, q.width, q.height
                )
            })
            .collect()
    };
    let slots: Vec<String> = p
        .inventory
        .equipment
        .slots()
        .filter_map(|(s, i)| Some(format!("{s}: {}", text.tech(i?.tech))))
        .collect();
    let n = [
        p.inventory.container.len(),
        p.inventory.equipment.items().count(),
        items.locker.len(),
        items.objects.len(),
        items.gone.len(),
    ];
    let line = format!(
        "grid [{}]; slots [{}]; locker [{}]; {} server-made objects, {} keys gone",
        grid(&p.inventory.container).join(", "),
        slots.join(", "),
        grid(&items.locker).join(", "),
        n[3],
        n[4]
    );
    (line, n)
}

struct Checks(Vec<(String, bool)>);

impl Checks {
    fn check(&mut self, what: impl Into<String>, ok: bool) {
        let what = what.into();
        println!("  check: {what}: {}", if ok { "ok" } else { "FAILED" });
        self.0.push((what, ok));
    }
}

/// Steps the run idle (`steps` physics steps), sends them, and lets the
/// bodies follow the items.
fn idle(r: &mut Run, link: &mut Link, steps: usize) -> Result<()> {
    for _ in 0..steps {
        r.step(&Input::default())?;
    }
    link.flush(records(r));
    if let Some(items) = link.items() {
        let (removed, added) = r.s.follow_items(items);
        if removed + added > 0 {
            println!("    bodies follow the items: {removed} removed, {added} added");
        }
    }
    Ok(())
}

fn centre(b: &Body) -> Option<V3> {
    b.bounds().map(|(lo, hi)| (lo + hi) * 0.5)
}

/// Walks or swims towards `at`, the hand's ray aimed at it every step,
/// until the ray hits a body `want` accepts (that body and its
/// distance) or `seconds` pass. Blocked for half a second, it steps aside
/// for half a second (as `walk::go_to`).
fn reach(
    r: &mut Run,
    link: &mut Link,
    at: V3,
    want: &dyn Fn(u64) -> bool,
    seconds: f64,
) -> Result<Option<(f64, u64)>> {
    let end = r.steps + (seconds / r.params.fixed_dt) as usize;
    let half = (0.5 / r.params.fixed_dt) as usize;
    let (mut blocked, mut aside, mut side) = (0usize, 0usize, 1.0);
    while r.steps < end {
        let eye = r.eye();
        let (mut yaw, pitch) = look_at(eye, at);
        if let Some((d, hit)) = hand_target(&r.s.world, eye, yaw, pitch)
            && want(hit.body)
        {
            // The look as the hand aimed it.
            r.step(&Input {
                move_dir: V3::ZERO,
                yaw,
                pitch,
                jump: false,
            })?;
            link.flush(records(r));
            return Ok(Some((d, hit.body)));
        }
        if aside > 0 {
            aside -= 1;
            yaw += side * std::f64::consts::FRAC_PI_2;
        }
        let before = r.player.position;
        r.step(&Input {
            move_dir: V3::new(0.0, 0.0, 1.0),
            yaw,
            pitch,
            jump: false,
        })?;
        if (r.player.position - before).length() < 0.3 * r.params.walk_speed * r.params.fixed_dt {
            blocked += 1;
            if blocked >= half && aside == 0 {
                blocked = 0;
                aside = half;
                side = -side;
            }
        } else {
            blocked = 0;
        }
    }
    link.flush(records(r));
    Ok(None)
}

/// The hand on `body` as the client's hand sees it: its text and the
/// request a click sends.
fn hand(r: &Run, link: &Link, text: &ItemText, body: u64) -> Option<(String, AimAction)> {
    let items = link.items()?;
    let me = link.client.player()?;
    let own = items.players.get(&me)?;
    let a = aim(
        text,
        items.rules(),
        r.s.target(body),
        body & LOCKER_BODY != 0,
        |tech| {
            own.inventory
                .container
                .has_room_for(&items.rules().techs, tech)
        },
    )?;
    let shown = if a.subscript.is_empty() {
        a.text.clone()
    } else {
        format!("{} / {}", a.text, a.subscript)
    };
    Some((shown, a.action))
}

/// Swims to the target `t` and clicks it once the hand reaches it; the
/// request sent.
fn take(
    r: &mut Run,
    link: &mut Link,
    text: &ItemText,
    t: &TargetBody,
    body: u64,
    seconds: f64,
) -> Result<Option<Request>> {
    let at =
        r.s.world
            .get(body)
            .and_then(centre)
            .unwrap_or(V3::from_f32(t.at.position));
    let key = t.key.clone();
    // Any of the key's bodies.
    let all = r.s.targets_of(&key);
    let want = |b: u64| all.contains(&b);
    let Some((d, hit)) = reach(r, link, at, &want, seconds)? else {
        r.log(&format!("the hand did not reach {:?} {key:?}", t.kind));
        return Ok(None);
    };
    match hand(r, link, text, hit) {
        Some((shown, AimAction::Request(req))) => {
            r.log(&format!("the hand on {key:?} from {d:.2} m: {shown:?}"));
            link.request(req.clone());
            Ok(Some(req))
        }
        other => {
            r.log(&format!("the hand on {key:?}: {other:?}, nothing to ask"));
            Ok(None)
        }
    }
}

fn sim_id(id: ItemId) -> inv::ItemId {
    inv::ItemId(u64::from(id.0))
}

fn randomize(items: &ItemsState, id: ItemId) -> bool {
    items
        .classes
        .get(&id)
        .and_then(|c| items.rules().pickups.get(c))
        .is_some_and(|i| i.randomize_rotation)
}

pub fn run(game: &GameData, seed: u64) -> Result<ExitCode> {
    let started = Instant::now();
    let mut s = Streamed::new(game, seed)?;
    let rules = sn_assets::rules(game, s.loader.assets())?;
    let dll = sn_assets::read_assembly(game)?;
    let text = sn_assets::item_text(game, &dll, "English")?;
    let code = sn_assets::player_code(&dll)?;
    let data = sn_assets::player_data(s.loader.assets())?;
    let vp = sn_assets::vitals_params(&data, &code);
    s.loader.set_item_rules(rules.items.clone());
    println!(
        "rules: build {:?}, fingerprint {:016x}; {} pickup classes, {} outcrops, {} supplies; read in {:.1} s",
        rules.build,
        rules.fingerprint,
        rules.items.pickups.len(),
        rules.items.outcrops.len(),
        rules.items.supplies.len(),
        started.elapsed().as_secs_f64()
    );

    let mut r = walk::start(&mut s, seed)?;
    r.vitals = Some((vp.clone(), Vitals::new(&vp)));
    r.record = Some(Vec::new());
    let locker_at = r.pod.locker.as_ref().and_then(|l| {
        for (i, b) in l.bodies.iter().enumerate() {
            r.s.world.insert(LOCKER_BODY | i as u64, b.clone());
        }
        l.bodies.first().and_then(centre)
    });
    let mut link = Link::start(
        &rules,
        WorldSettings {
            slot_seed: Some(seed),
            lifepod: Some(r.pod_point.to_f32()),
            aurora: AuroraStart::Held { exploded: false },
            clock_start: 0.0,
            time_scale: 1.0,
        },
    )?;
    let mut checks = Checks(Vec::new());
    let mut draws = Draws(seed ^ 0x7069_636b);
    let (line, n0) = snapshot(&text, &link);
    println!("  start: {line}");
    checks.check(
        format!(
            "the locker holds the {} supplies",
            rules.items.supplies.len()
        ),
        n0[2] == rules.items.supplies.len() && n0[0] == 0,
    );

    // 1. The locker: the hand on it, then every item out (a left click,
    // `Switch`), as the panel does.
    println!("step 1: the locker");
    idle(&mut r, &mut link, 50)?;
    let opened = match locker_at {
        Some(at) => reach(&mut r, &mut link, at, &|b| b & LOCKER_BODY != 0, 15.0)?,
        None => None,
    };
    match opened.and_then(|(d, b)| Some((d, hand(&r, &link, &text, b)?))) {
        Some((d, (shown, AimAction::OpenLocker))) => {
            r.log(&format!("the hand on the locker from {d:.2} m: {shown:?}"));
        }
        other => r.log(&format!("the locker not reached: {other:?}")),
    }
    checks.check("the hand opens the locker", opened.is_some());
    // `EscapePod.StorageOpen`: the panel 0.25 s later.
    idle(&mut r, &mut link, 13)?;
    let place = DropPlace {
        in_escape_pod: r.player.in_pod,
        ..DropPlace::default()
    };
    let moves: Vec<Request> = link
        .items()
        .and_then(|items| {
            let p = items.players.get(&link.client.player()?)?;
            let list = items
                .locker
                .layout()
                .0
                .iter()
                .filter_map(|q| {
                    let id = ItemId(q.id.0 as u32);
                    let a = inv::item_actions(
                        &items.rules().techs,
                        &p.inventory,
                        Some(&items.locker),
                        q.id,
                        false,
                        place,
                    );
                    (inv::item_action(a, Button::Left) == inv::SWITCH).then_some(Request::Move {
                        item: id,
                        to: Store::Grid,
                        target: MoveTarget::Store,
                    })
                })
                .collect();
            Some(list)
        })
        .unwrap_or_default();
    for m in moves {
        link.request(m);
    }
    idle(&mut r, &mut link, 10)?;
    let (line, n1) = snapshot(&text, &link);
    println!("  after the locker: {line}");
    checks.check(
        "every supply moved to the grid",
        n1[0] == n0[2] && n1[2] == 0,
    );

    // 2. Out of the pod.
    println!("step 2: out of the pod");
    let out = walk::leave_pod(&mut r)?;
    idle(&mut r, &mut link, 50)?;
    checks.check("left the pod", out && !r.player.in_pod);

    // 3. The nearest pickup.
    println!("step 3: the nearest pickup");
    let near = |r: &Run, kind: TargetKind| -> Vec<(u64, TargetBody)> {
        let p = r.player.position;
        let mut v: Vec<(f64, u64, TargetBody)> =
            r.s.targets()
                .into_iter()
                .filter(|(_, t)| t.kind == kind && !matches!(t.key, EntityKey::Spawned(_)))
                .filter(|(_, t)| {
                    kind != TargetKind::Pickup
                        || rules
                            .items
                            .pickups
                            .get(&t.class)
                            .is_some_and(|i| i.pickupable)
                })
                .map(|(id, t)| ((V3::from_f32(t.at.position) - p).length(), id, t))
                .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        v.into_iter().map(|(_, id, t)| (id, t)).collect()
    };
    let mut picked = false;
    for (body, t) in near(&r, TargetKind::Pickup).into_iter().take(5) {
        let before = snapshot(&text, &link).1;
        println!(
            "  trying {} {:?} at {:.1} m",
            text.tech(rules.items.pickups[&t.class].tech),
            t.key,
            (V3::from_f32(t.at.position) - r.player.position).length()
        );
        if take(&mut r, &mut link, &text, &t, body, 30.0)?.is_some() {
            idle(&mut r, &mut link, 10)?;
            let (line, n) = snapshot(&text, &link);
            println!("  after the pick-up: {line}");
            let gone = link.items().is_some_and(|i| i.gone.contains(&t.key));
            checks.check(
                "the pickup is in the inventory (+1) and gone from the world",
                n[0] + n[1] == before[0] + before[1] + 1 && gone,
            );
            picked = true;
            break;
        }
    }
    checks.check("a pickup was reached", picked);

    // 4. The nearest outcrop: hit until it breaks, then its resource.
    println!("step 4: the nearest outcrop");
    let mut broke = false;
    for (body, t) in near(&r, TargetKind::Outcrop).into_iter().take(5) {
        let info = &rules.items.outcrops[&t.class];
        println!(
            "  trying {:?} ({} hits) {:?} at {:.1} m",
            text.get(&info.break_text),
            info.hits_to_break,
            t.key,
            (V3::from_f32(t.at.position) - r.player.position).length()
        );
        let before = snapshot(&text, &link).1;
        let mut hits = 0;
        for _ in 0..info.hits_to_break {
            if take(
                &mut r,
                &mut link,
                &text,
                &t,
                body,
                if hits == 0 { 30.0 } else { 5.0 },
            )?
            .is_none()
            {
                break;
            }
            hits += 1;
            // About a second between clicks.
            idle(&mut r, &mut link, 50)?;
        }
        let (line, n) = snapshot(&text, &link);
        println!("  after {hits} hits: {line}");
        if hits == 0 {
            continue;
        }
        let gone = link.items().is_some_and(|i| i.gone.contains(&t.key));
        let new_objects: Vec<(ItemId, String)> = link
            .items()
            .map(|i| {
                i.objects
                    .iter()
                    .map(|(id, o)| (*id, o.class.clone()))
                    .collect()
            })
            .unwrap_or_default();
        checks.check(
            format!(
                "{hits} hits broke the outcrop into {} resource(s)",
                n[3] - before[3]
            ),
            hits == info.hits_to_break as usize && gone && n[3] > before[3],
        );
        broke = true;
        // Its resource: the newest server-made object.
        let Some(&(id, ref class)) = new_objects.iter().max_by_key(|(id, _)| *id) else {
            break;
        };
        let key = EntityKey::Spawned(id);
        let found = r.s.targets().into_iter().find(|(_, x)| x.key == key);
        println!(
            "  its resource: object {} ({}), bodies {}",
            id.0,
            rules
                .items
                .pickups
                .get(class)
                .map_or_else(|| class.clone(), |i| text.tech(i.tech)),
            if found.is_some() { "yes" } else { "no" }
        );
        if let Some((b, x)) = found {
            let before = snapshot(&text, &link).1;
            let ok = take(&mut r, &mut link, &text, &x, b, 10.0)?.is_some();
            idle(&mut r, &mut link, 10)?;
            let (line, n) = snapshot(&text, &link);
            println!("  after picking the resource: {line}");
            checks.check(
                "the resource is in the inventory (+1), its object gone",
                ok && n[0] + n[1] == before[0] + before[1] + 1 && n[3] + 1 == before[3],
            );
        } else {
            checks.check("the resource has bodies", false);
        }
        break;
    }
    checks.check("an outcrop was broken", broke);

    // 5. Drop one item and pick it up again at once.
    println!("step 5: drop and pick up again");
    let first = link.items().and_then(|items| {
        let p = items.players.get(&link.client.player()?)?;
        let q = p.inventory.container.layout().0.first().map(|q| q.id)?;
        let id = ItemId(q.0 as u32);
        Some((id, randomize(items, id)))
    });
    if let Some((item, random)) = first {
        let before = snapshot(&text, &link).1;
        let forward =
            sn_sim::player::look_rotate(r.look.yaw(), r.look.pitch(), V3::new(0.0, 0.0, 1.0));
        let camera = r.eye();
        let d = drop_at(&r.s.world, camera, forward, random, &mut draws);
        let actions = link
            .items()
            .and_then(|items| {
                let p = items.players.get(&link.client.player()?)?;
                Some(inv::item_actions(
                    &items.rules().techs,
                    &p.inventory,
                    None,
                    sim_id(item),
                    false,
                    DropPlace::default(),
                ))
            })
            .unwrap_or(0);
        r.log(&format!(
            "right click on item {}: {}; ray to ({:.2}, {:.2}, {:.2}), lands {:.2} m from the camera",
            item.0,
            inv::action_name(inv::item_action(actions, Button::Right)),
            d.ray.x,
            d.ray.y,
            d.ray.z,
            (V3::from_f32(d.at.point) - camera).length()
        ));
        link.request(Request::Drop { item, at: d.at });
        idle(&mut r, &mut link, 2)?;
        let (line, n) = snapshot(&text, &link);
        println!("  after the drop: {line}");
        checks.check(
            "the drop left the inventory (−1) as a server-made object (+1)",
            n[0] + n[1] + 1 == before[0] + before[1] && n[3] == before[3] + 1,
        );
        let key = EntityKey::Spawned(item);
        let found = r.s.targets().into_iter().find(|(_, x)| x.key == key);
        let ok = match found {
            Some((b, x)) => take(&mut r, &mut link, &text, &x, b, 5.0)?.is_some(),
            None => false,
        };
        idle(&mut r, &mut link, 10)?;
        let (line, n) = snapshot(&text, &link);
        println!("  after picking it up again: {line}");
        checks.check(
            "picked up again at once: back to the count before the drop",
            ok && n[0] + n[1] == before[0] + before[1] && n[3] == before[3],
        );
    } else {
        checks.check("an item to drop", false);
    }

    // 6. A death: `LoseItems` at the respawn, before the move.
    println!("step 6: death");
    let before = snapshot(&text, &link).1;
    let lose: Vec<(ItemId, bool)> = link
        .items()
        .and_then(|items| {
            let me = link.client.player()?;
            let place = DropPlace {
                in_escape_pod: r.player.in_pod,
                ..DropPlace::default()
            };
            Some(
                items
                    .lose_list(me, place)
                    .into_iter()
                    .map(|id| (id, randomize(items, id)))
                    .collect(),
            )
        })
        .unwrap_or_default();
    if let Some((vp, v)) = r.vitals.as_mut() {
        let mut events = Vec::new();
        v.take_damage(vp.max_health, &mut events);
        r.body.died();
        r.log(&format!("killed: {events:?}"));
    }
    let limit = r.steps + (15.0 / r.params.fixed_dt) as usize;
    while r.death_camera.is_none() && r.steps < limit {
        r.step(&Input::default())?;
    }
    let mut lost = 0;
    if let Some((camera, forward)) = r.death_camera {
        // The states up to the death place, not the respawn's.
        let rec = records(&r);
        link.flush(&rec[..rec.len().saturating_sub(1)]);
        if !lose.is_empty() {
            let (origin, drops) = death_drops(&r.s.world, camera, forward, &lose, &mut draws);
            r.log(&format!(
                "LoseItems: {} of {} items from ({:.2}, {:.2}, {:.2})",
                drops.len(),
                before[0] + before[1],
                camera.x,
                camera.y,
                camera.z
            ));
            lost = drops.len();
            link.request(Request::Lose { origin, drops });
        }
    }
    idle(&mut r, &mut link, 60)?;
    let (line, n) = snapshot(&text, &link);
    println!("  after the death: {line}");
    checks.check(
        format!(
            "the death dropped {lost} items (all but one lost on death), kept {}",
            n[0] + n[1]
        ),
        r.death_camera.is_some()
            && lost > 0
            && n[0] + n[1] == before[0] + before[1] - lost
            && n[3] == before[3] + lost,
    );

    // The client and the server hold the same world.
    let (a, b) = (
        link.server.world().hash(),
        link.client.world().map(|w| w.hash()),
    );
    checks.check(
        format!("client and server world hashes equal ({a:016x})"),
        Some(a) == b,
    );
    checks.check(
        format!("no request refused ({:?})", link.refused),
        link.refused.is_empty(),
    );
    let failed = checks.0.iter().filter(|c| !c.1).count();
    println!(
        "seed {seed}: {} checks, {failed} failed; {} steps ({:.1} s simulated), {} ticks sent, in {:.1} s",
        checks.0.len(),
        r.steps,
        r.t(),
        link.sent,
        started.elapsed().as_secs_f64()
    );
    walk::report(&r);
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
