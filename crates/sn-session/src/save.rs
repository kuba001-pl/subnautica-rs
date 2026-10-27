//! Our versioned server save. Pure bytes in/out; no game save compatibility.
use std::collections::{BTreeMap, BTreeSet};

use sn_protocol::codec::{Reader, Writer};
use sn_protocol::{Fnv64, PROTOCOL_VERSION, PlayerId, ServerMessage};

use super::{Server, ServerConfig};

pub const SAVE_VERSION: u16 = 1;
pub const MAX_SAVE_BYTES: usize = 64 * 1024 * 1024;
const MAGIC: u64 = u64::from_le_bytes(*b"SNRSsave");

impl Server {
    /// Captures both online and offline players. Passwords/connections omitted.
    pub fn save_bytes(&self, now_us: u64) -> Vec<u8> {
        let mut w = Writer::default();
        w.str(&self.config.build);
        w.u64_fixed(self.config.fingerprint);
        w.varint(self.config.tick_us);
        sn_protocol::write_settings(&mut w, &self.world.settings);
        w.varint(u64::from(self.clock(now_us).0.0));
        w.u64_fixed(self.random);
        w.varint(u64::from(self.next_id));
        let names: BTreeMap<_, _> = self.ids.iter().map(|(name, id)| (*id, name)).collect();
        w.varint(names.len() as u64);
        for (id, name) in names {
            w.varint(u64::from(id.0));
            w.str(name);
            let state = self
                .players
                .get(&id)
                .and_then(|p| p.state)
                .or_else(|| self.saved_states.get(&id).copied());
            w.bytes(
                &ServerMessage::Restore {
                    state,
                    vitals: self.vitals.get(&id).copied(),
                }
                .encode(),
            );
        }
        self.world.items.encode(&mut w);
        let mut envelope = Writer::default();
        envelope.u64_fixed(MAGIC);
        envelope.u16(SAVE_VERSION);
        envelope.u16(PROTOCOL_VERSION);
        envelope.u64_fixed(Fnv64::new().bytes(&w.out).finish());
        envelope.bytes(&w.out);
        envelope.out
    }

    /// Loads at a fresh process time, retaining the world's elapsed ticks.
    /// Every error is an absolute byte offset in the save; no partial mutation.
    pub fn from_save(config: ServerConfig, bytes: &[u8], now_us: u64) -> sn_protocol::Result<Self> {
        let mut r = Reader::new(bytes);
        if bytes.len() > MAX_SAVE_BYTES {
            return Err(r.error(0, "save exceeds 64 MiB"));
        }
        if r.u64_fixed("save magic")? != MAGIC {
            return Err(r.error(0, "not an sn-rs save"));
        }
        if r.u16("save version")? != SAVE_VERSION {
            return Err(r.error(8, "unsupported save version"));
        }
        if r.u16("protocol version")? != PROTOCOL_VERSION {
            return Err(r.error(10, "incompatible protocol version"));
        }
        let checksum = r.u64_fixed("checksum")?;
        let payload = r.bytes(MAX_SAVE_BYTES, "payload")?;
        let base = r.pos - payload.len();
        r.finish()?;
        if Fnv64::new().bytes(&payload).finish() != checksum {
            return Err(r.error(12, "save checksum mismatch"));
        }
        Self::load_payload(config, &payload, now_us).map_err(|mut e| {
            e.offset += base;
            e
        })
    }

    fn load_payload(
        mut config: ServerConfig,
        bytes: &[u8],
        now_us: u64,
    ) -> sn_protocol::Result<Self> {
        let mut r = Reader::new(bytes);
        let build = r.str(sn_protocol::MAX_BUILD, "build")?;
        if build != config.build {
            return Err(r.error(0, "save game build mismatch"));
        }
        let at = r.pos;
        if r.u64_fixed("rules fingerprint")? != config.fingerprint {
            return Err(r.error(at, "save rules mismatch"));
        }
        let at = r.pos;
        if r.varint("tick duration")? != config.tick_us {
            return Err(r.error(at, "save tick duration mismatch"));
        }
        config.settings = sn_protocol::read_settings(&mut r)?;
        let base_tick = r.varint_u32("elapsed ticks")?;
        let random = r.u64_fixed("random state")?;
        let next_id = r.varint_u16("next player id")?;
        let mut server = Self::new(config, now_us);
        server.base_tick = base_tick;
        server.random = random;
        server.next_id = next_id;
        let mut seen = BTreeSet::new();
        for _ in 0..r.count(u16::MAX as usize, "saved players")? {
            let at = r.pos;
            let id = PlayerId(r.varint_u16("saved player id")?);
            let name = r.str(sn_protocol::MAX_NAME, "saved player name")?;
            if id.0 >= next_id
                || !seen.insert(id)
                || name.trim().is_empty()
                || name.trim() != name
                || name.chars().any(char::is_control)
                || server.ids.insert(name, id).is_some()
            {
                return Err(r.error(at, "invalid or duplicate saved player"));
            }
            let message = r.bytes(1024, "saved pose/vitals")?;
            let start = r.pos - message.len();
            let decoded = ServerMessage::decode(&message).map_err(|mut e| {
                e.offset += start;
                e
            })?;
            let ServerMessage::Restore { state, vitals } = decoded else {
                return Err(r.error(start, "expected saved pose/vitals"));
            };
            if let Some(s) = state {
                if s.tick.0 > base_tick {
                    return Err(r.error(start, "player tick ahead of save"));
                }
                server.saved_states.insert(id, s);
            }
            if let Some(v) = vitals {
                server.vitals.insert(id, v);
            }
        }
        let at = r.pos;
        server.world.items = crate::ItemsState::decode(server.config.rules.clone(), &mut r)?;
        r.finish()?;
        if server
            .world
            .items
            .players
            .keys()
            .copied()
            .collect::<BTreeSet<_>>()
            != seen
        {
            return Err(r.error(at, "saved identities differ from item owners"));
        }
        // Check allocation and ownership invariants before accepting state.
        let items = &server.world.items;
        let rules = items.rules();
        if items.locker.size() != rules.locker_size
            || items
                .players
                .values()
                .any(|p| p.inventory.container.size() != rules.inventory_size)
        {
            return Err(r.error(at, "saved container size differs from rules"));
        }
        for c in std::iter::once(&items.locker)
            .chain(items.players.values().map(|p| &p.inventory.container))
        {
            for (tech, width, height, _) in c.to_parts().entries.into_iter().flatten() {
                if (width, height) != rules.techs.size(tech) || width <= 0 || height <= 0 {
                    return Err(r.error(at, "invalid saved item dimensions"));
                }
            }
        }
        let mut owned = BTreeSet::new();
        for i in items.locker.items().chain(
            items
                .players
                .values()
                .flat_map(|p| p.inventory.container.items()),
        ) {
            if !owned.insert(sn_protocol::ItemId(
                u32::try_from(i.id.0).map_err(|_| r.error(at, "item id overflow"))?,
            )) {
                return Err(r.error(at, "item owned twice"));
            }
        }
        for p in items.players.values() {
            for (_, item) in p.inventory.equipment.slots() {
                if let Some(i) = item
                    && !owned.insert(sn_protocol::ItemId(
                        u32::try_from(i.id.0).map_err(|_| r.error(at, "item id overflow"))?,
                    ))
                {
                    return Err(r.error(at, "equipment owned twice"));
                }
            }
        }
        if owned.iter().any(|id| {
            id.0 >= items.next_item
                || !items.classes.contains_key(id)
                || items.objects.contains_key(id)
        }) || items.classes.keys().any(|id| !owned.contains(id))
            || items.objects.keys().any(|id| id.0 >= items.next_item)
        {
            return Err(r.error(at, "invalid item allocation/class reference"));
        }
        server.hashes.clear();
        server.hashes.push_back((0, server.world.hash()));
        Ok(server)
    }
}
