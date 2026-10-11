//! The shared world state: what every machine must hold identically
//! (`docs/DESIGN.md` § M10 plan, promise 2). It is a function of the world
//! settings and the ordered world event log, so the server and every
//! client build it the same way: from a `Welcome` snapshot, then by
//! applying each event in `seq` order. Its hash is the check.
//!
//! In M10 it holds the players (id, name, ready, the cinematic they are
//! in); from M9d3b also every item ([`ItemsState`]). Positions are not
//! here: they are each owner's, and drawn from states (promise 3).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use sn_protocol::codec::Reader;
use sn_protocol::{
    AuroraStart, Fnv64, PlayerEventKind, PlayerId, PlayerInfo, RefuseReason, Tick, WorldEvent,
    WorldEventKind, WorldSettings,
};

use crate::items::{ItemRules, ItemsState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerRecord {
    pub name: String,
    pub ready: bool,
    /// The lifepod hatch whose cinematic the player is in, and its start.
    pub cinematic: Option<(u16, Tick)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorldState {
    pub settings: WorldSettings,
    /// World events applied: the `seq` the next one must have.
    pub events: u32,
    pub players: BTreeMap<PlayerId, PlayerRecord>,
    pub items: ItemsState,
}

/// An event that does not fit the state: a bug at the sender, or a lost
/// or reordered message (which the reliable channel rules out).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyError {
    /// The event's `seq` is not the next one.
    OutOfOrder {
        expected: u32,
        got: u32,
    },
    UnknownPlayer {
        seq: u32,
        player: PlayerId,
    },
    PlayerExists {
        seq: u32,
        player: PlayerId,
    },
    /// An item event the items do not allow (the server refuses the
    /// request for this reason; on a client it means the states differ).
    Item {
        seq: u32,
        reason: RefuseReason,
    },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::OutOfOrder { expected, got } => {
                write!(f, "world event {got} arrived, {expected} expected")
            }
            ApplyError::UnknownPlayer { seq, player } => {
                write!(f, "world event {seq} names unknown player {}", player.0)
            }
            ApplyError::PlayerExists { seq, player } => {
                write!(f, "world event {seq} adds player {} again", player.0)
            }
            ApplyError::Item { seq, reason } => {
                write!(f, "world event {seq}: item event refused ({reason:?})")
            }
        }
    }
}

impl std::error::Error for ApplyError {}

impl WorldState {
    /// A new world: with a lifepod, its locker gets the supplies.
    pub fn new(settings: WorldSettings, rules: Arc<ItemRules>) -> WorldState {
        WorldState {
            settings,
            events: 0,
            players: BTreeMap::new(),
            items: ItemsState::new(rules, settings.lifepod.is_some()),
        }
    }

    /// The state a `Welcome` describes (after its `next_seq` events).
    /// The items are each listed player's starting ones until
    /// [`WorldState::load_items`] puts the ones the `WorldChunk`s carry.
    pub fn from_welcome(
        settings: WorldSettings,
        next_seq: u32,
        players: &[PlayerInfo],
        rules: Arc<ItemRules>,
    ) -> Self {
        let mut items = ItemsState::new(rules, false);
        for p in players {
            items.join(p.id);
        }
        WorldState {
            items,
            settings,
            events: next_seq,
            players: players
                .iter()
                .map(|p| {
                    (
                        p.id,
                        PlayerRecord {
                            name: p.name.clone(),
                            ready: p.ready,
                            cinematic: p.cinematic,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Replaces the items with the ones `bytes` encode (the joined
    /// `WorldChunk`s).
    pub fn load_items(&mut self, bytes: &[u8]) -> sn_protocol::Result<()> {
        let mut r = Reader::new(bytes);
        let items = ItemsState::decode(self.items.rules().clone(), &mut r)?;
        r.finish()?;
        self.items = items;
        Ok(())
    }

    /// Applies the next event. On an error the state is unchanged.
    pub fn apply(&mut self, e: &WorldEvent) -> Result<(), ApplyError> {
        if e.seq != self.events {
            return Err(ApplyError::OutOfOrder {
                expected: self.events,
                got: e.seq,
            });
        }
        let seq = e.seq;
        let unknown = |player| ApplyError::UnknownPlayer { seq, player };
        match &e.kind {
            WorldEventKind::PlayerJoined { player, name } => {
                if self.players.contains_key(player) {
                    return Err(ApplyError::PlayerExists {
                        seq,
                        player: *player,
                    });
                }
                self.players.insert(
                    *player,
                    PlayerRecord {
                        name: name.clone(),
                        ready: false,
                        cinematic: None,
                    },
                );
                self.items.join(*player);
            }
            WorldEventKind::PlayerReady { player } => {
                self.players.get_mut(player).ok_or(unknown(*player))?.ready = true;
            }
            WorldEventKind::PlayerLeft { player, .. } => {
                self.players.remove(player).ok_or(unknown(*player))?;
            }
            WorldEventKind::Item { player, event, .. } => {
                if !self.players.contains_key(player) {
                    return Err(unknown(*player));
                }
                self.items
                    .apply(*player, event)
                    .map_err(|reason| ApplyError::Item { seq, reason })?;
            }
            WorldEventKind::Player { player, event } => {
                let p = self.players.get_mut(player).ok_or(unknown(*player))?;
                match event {
                    PlayerEventKind::HatchUsed { hatch } => p.cinematic = Some((*hatch, e.tick)),
                    PlayerEventKind::CinematicEnded => p.cinematic = None,
                    // Relayed for the bodies; the shared state keeps no
                    // more of them (life is in each state).
                    PlayerEventKind::Died { .. }
                    | PlayerEventKind::Respawned
                    | PlayerEventKind::AnimatorTrigger { .. }
                    | PlayerEventKind::CinematicPrepare { .. }
                    | PlayerEventKind::CinematicPlay { .. } => {}
                }
            }
        }
        self.events += 1;
        Ok(())
    }

    /// FNV-1a 64 of everything above, in a fixed order
    /// (`docs/formats/protocol.md` § World state).
    pub fn hash(&self) -> u64 {
        let mut h = Fnv64::new();
        let s = &self.settings;
        match s.slot_seed {
            Some(seed) => h.u64(1).u64(seed),
            None => h.u64(0),
        };
        match s.lifepod {
            Some(p) => h.u64(1).f32(p[0]).f32(p[1]).f32(p[2]),
            None => h.u64(0),
        };
        match s.aurora {
            AuroraStart::Countdown { seconds } => h.u64(0).f32(seconds),
            AuroraStart::Held { exploded } => h.u64(1).u64(u64::from(exploded)),
        };
        h.f64(s.clock_start).f32(s.time_scale);
        h.u64(u64::from(self.events));
        h.u64(self.players.len() as u64);
        for (id, p) in &self.players {
            h.u64(u64::from(id.0)).str(&p.name).u64(u64::from(p.ready));
            match p.cinematic {
                Some((hatch, start)) => h.u64(1).u64(u64::from(hatch)).u64(u64::from(start.0)),
                None => h.u64(0),
            };
        }
        self.items.hash_into(&mut h);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sn_protocol::LeaveReason;

    fn settings() -> WorldSettings {
        WorldSettings {
            slot_seed: Some(1),
            lifepod: Some([10.0, 0.5, -20.0]),
            aurora: AuroraStart::Countdown { seconds: 3000.0 },
            clock_start: 480.0,
            time_scale: 1.0,
        }
    }

    fn rules() -> Arc<ItemRules> {
        Arc::new(ItemRules::default())
    }

    fn ev(seq: u32, tick: u32, kind: WorldEventKind) -> WorldEvent {
        WorldEvent {
            seq,
            tick: Tick(tick),
            kind,
        }
    }

    #[test]
    fn events_build_the_state_and_a_welcome_reproduces_it() {
        let mut w = WorldState::new(settings(), rules());
        let a = PlayerId(0);
        let log = [
            ev(
                0,
                5,
                WorldEventKind::PlayerJoined {
                    player: a,
                    name: "a".into(),
                },
            ),
            ev(1, 9, WorldEventKind::PlayerReady { player: a }),
            ev(
                2,
                30,
                WorldEventKind::Player {
                    player: a,
                    event: PlayerEventKind::HatchUsed { hatch: 2 },
                },
            ),
        ];
        let empty = w.hash();
        for e in &log {
            w.apply(e).unwrap();
        }
        assert_ne!(w.hash(), empty);
        assert_eq!(w.players[&a].cinematic, Some((2, Tick(30))));
        let infos: Vec<PlayerInfo> = w
            .players
            .iter()
            .map(|(id, p)| PlayerInfo {
                id: *id,
                name: p.name.clone(),
                ready: p.ready,
                state: None,
                cinematic: p.cinematic,
            })
            .collect();
        let mut copy = WorldState::from_welcome(settings(), w.events, &infos, rules());
        copy.load_items(&w.items.to_bytes()).unwrap();
        assert_eq!(copy, w);
        assert_eq!(copy.hash(), w.hash());
    }

    #[test]
    fn bad_events_are_refused_and_change_nothing() {
        let mut w = WorldState::new(settings(), rules());
        let before = w.clone();
        assert_eq!(
            w.apply(&ev(
                1,
                0,
                WorldEventKind::PlayerReady {
                    player: PlayerId(0)
                }
            )),
            Err(ApplyError::OutOfOrder {
                expected: 0,
                got: 1
            })
        );
        assert!(
            w.apply(&ev(
                0,
                0,
                WorldEventKind::PlayerLeft {
                    player: PlayerId(3),
                    reason: LeaveReason::Left
                }
            ))
            .is_err()
        );
        assert_eq!(w, before);
    }

    #[test]
    fn every_field_moves_the_hash() {
        let base = WorldState::new(settings(), rules());
        let mut variants = Vec::new();
        let mut s = settings();
        s.slot_seed = Some(2);
        variants.push(WorldState::new(s, rules()));
        let mut s = settings();
        s.lifepod = Some([10.0, 0.5, -20.000_002]);
        variants.push(WorldState::new(s, rules()));
        let mut s = settings();
        s.aurora = AuroraStart::Held { exploded: false };
        variants.push(WorldState::new(s, rules()));
        let mut s = settings();
        s.clock_start = 480.5;
        variants.push(WorldState::new(s, rules()));
        let mut w = base.clone();
        w.events = 1;
        variants.push(w);
        let record = PlayerRecord {
            name: "a".into(),
            ready: false,
            cinematic: None,
        };
        let mut w = base.clone();
        w.players.insert(PlayerId(0), record.clone());
        variants.push(w.clone());
        let mut w2 = w.clone();
        w2.players.get_mut(&PlayerId(0)).unwrap().ready = true;
        variants.push(w2);
        let mut w3 = w.clone();
        w3.players.get_mut(&PlayerId(0)).unwrap().cinematic = Some((0, Tick(0)));
        variants.push(w3);
        let mut w4 = base.clone();
        w4.players.insert(PlayerId(1), record);
        variants.push(w4);
        let mut hashes: Vec<u64> = variants.iter().map(WorldState::hash).collect();
        hashes.push(base.hash());
        let n = hashes.len();
        hashes.sort_unstable();
        hashes.dedup();
        assert_eq!(hashes.len(), n);
    }
}
