//! Spawn slots: the `EntitySlotsPlaceholder` components of the baked cells
//! and how the game fills them. See `docs/formats/entities.md` § Spawn slots.
//!
//! A placeholder holds a list of slots (biome, allowed slot types, density,
//! position and rotation relative to the placeholder). When its cell first
//! loads, the game picks one prefab per slot from the biome's loot table:
//! each candidate whose slot type the slot allows weighs `probability /
//! density`; if the weights add up to less than 1, the rest is "nothing".
//! The chosen entry's `count` copies are placed, the second and later ones
//! within 4 m of the slot.
//!
//! The game draws from Unity's global random generator, so its choice is
//! different in every save. We draw from our own generator keyed by (seed,
//! placeholder id, slot index): the same seed gives the same world in any
//! loading order.

use std::collections::HashMap;

use crate::entities::{Transform, bytes_field, floats, quat_mul, varint_field};
use crate::wire::{Result, Value, Wire};

/// Type name of the slot component in the object trees.
pub const SLOTS_COMPONENT: &str = "EntitySlotsPlaceholder";

/// Loot-table key the game skips (the "nothing" share is explicit there).
pub const FILLER_CLASS_ID: &str = "None";

/// Copies after the first are placed within this distance of the slot.
pub const EXTRA_COPY_RADIUS: f32 = 4.0;

/// A prefab's slot type (`EntitySlot.Type`, stored as its index in the
/// prefab table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SlotKind {
    Small,
    Medium,
    Large,
    Tall,
    Creature,
}

impl SlotKind {
    pub const ALL: [SlotKind; 5] = [
        SlotKind::Small,
        SlotKind::Medium,
        SlotKind::Large,
        SlotKind::Tall,
        SlotKind::Creature,
    ];

    pub fn from_index(i: i32) -> Option<SlotKind> {
        usize::try_from(i)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
    }

    /// The bit of this kind in a slot's allowed types.
    pub fn flag(self) -> u32 {
        1 << self as u32
    }
}

/// One slot of a placeholder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntitySlot {
    /// The game's `BiomeType` value (e.g. 102: Safe Shallows sand flat).
    pub biome: u32,
    /// Allowed slot kinds as flags ([`SlotKind::flag`]).
    pub allowed: u32,
    pub density: f32,
    /// Relative to the placeholder.
    pub position: [f32; 3],
    /// Relative to the placeholder (quaternion x, y, z, w).
    pub rotation: [f32; 4],
}

impl EntitySlot {
    pub fn allows(&self, kind: SlotKind) -> bool {
        self.allowed & kind.flag() != 0
    }

    pub fn is_creature_slot(&self) -> bool {
        self.allows(SlotKind::Creature)
    }
}

/// Fields the game reads as protobuf-net defaults when absent: the class's
/// own initial values (density 1), else zero.
fn parse_slot(mut m: Wire) -> Result<EntitySlot> {
    let mut slot = EntitySlot {
        biome: 0,
        allowed: 0,
        density: 1.0,
        position: [0.0; 3],
        rotation: [0.0; 4],
    };
    while let Some((number, value)) = m.field()? {
        match number {
            2 => {
                let v = varint_field(&m, number, value)?;
                slot.biome = u32::try_from(v).or_else(|_| m.error("biome too big"))?;
            }
            3 => {
                let v = varint_field(&m, number, value)?;
                slot.allowed = u32::try_from(v).or_else(|_| m.error("slot types too big"))?;
            }
            4 => match value {
                Value::Fixed32(bits) => slot.density = f32::from_bits(bits),
                other => return m.error(format!("density: {} in a slot", other.kind())),
            },
            5 => slot.position = floats(bytes_field(&m, number, value)?, [0.0; 3])?,
            6 => slot.rotation = floats(bytes_field(&m, number, value)?, [0.0; 4])?,
            _ => {}
        }
    }
    Ok(slot)
}

/// The slots of an `EntitySlotsPlaceholder` component (its saved data):
/// field 1 = version, field 2 (repeated) = one slot each.
pub fn parse_slots(data: &[u8]) -> Result<Vec<EntitySlot>> {
    let mut m = Wire::new(data);
    let mut slots = Vec::new();
    while let Some((number, value)) = m.field()? {
        if number == 2 {
            slots.push(parse_slot(bytes_field(&m, number, value)?)?);
        }
    }
    Ok(slots)
}

/// What the game knows about a prefab when it fills a slot
/// (`WorldEntityInfo`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityInfo {
    /// `None` for a type index outside the game's enum.
    pub kind: Option<SlotKind>,
    /// Turned −90° about x when placed.
    pub prefab_z_up: bool,
    /// `LargeWorldEntity.CellLevel`: 0–3 (near … very far), 10 (batch),
    /// 100 (global).
    pub cell_level: i32,
    /// Replaces the slot's scale.
    pub local_scale: [f32; 3],
}

/// One row of a biome's loot table.
#[derive(Clone, Debug, PartialEq)]
pub struct LootEntry {
    pub class_id: String,
    pub count: i32,
    pub probability: f32,
}

/// One prefab of the stored table: its ClassId and rows of (biome, count,
/// probability).
pub type LootRows = (String, Vec<(u32, i32, f32)>);

/// The loot tables per biome (`Balance/EntityDistributions`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LootDistribution {
    biomes: HashMap<u32, Vec<LootEntry>>,
}

impl LootDistribution {
    /// From the per-prefab table, as it is stored: per ClassId its rows of
    /// (biome, count, probability). Each biome's list keeps table order, as
    /// the game builds it (the order decides which entry a draw lands on).
    pub fn new(prefabs: Vec<LootRows>) -> LootDistribution {
        let mut biomes: HashMap<u32, Vec<LootEntry>> = HashMap::new();
        for (class_id, rows) in prefabs {
            for (biome, count, probability) in rows {
                biomes.entry(biome).or_default().push(LootEntry {
                    class_id: class_id.clone(),
                    count,
                    probability,
                });
            }
        }
        LootDistribution { biomes }
    }

    pub fn biome(&self, biome: u32) -> Option<&[LootEntry]> {
        self.biomes.get(&biome).map(Vec::as_slice)
    }

    pub fn biome_count(&self) -> usize {
        self.biomes.len()
    }

    /// Every biome's table (in no particular order).
    pub fn biomes(&self) -> impl Iterator<Item = (u32, &[LootEntry])> {
        self.biomes.iter().map(|(b, e)| (*b, e.as_slice()))
    }
}

/// What a slot is filled with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filler<'a> {
    pub class_id: &'a str,
    pub count: u32,
}

/// The game's choice for `slot` (`CSVEntitySpawner.GetPrefabForSlot`) with
/// the uniform draw `r` in [0, 1]. Not ported: the known-fragment filter
/// (nothing is known in a new game).
pub fn choose<'a>(
    slot: &EntitySlot,
    loot: &'a LootDistribution,
    infos: &HashMap<String, EntityInfo>,
    r: f32,
) -> Option<Filler<'a>> {
    let entries = loot.biome(slot.biome)?;
    let candidates = entries.iter().filter_map(|e| {
        if e.class_id == FILLER_CLASS_ID {
            return None;
        }
        let kind = infos.get(&e.class_id)?.kind?;
        if !slot.allows(kind) {
            return None;
        }
        let weight = e.probability / slot.density;
        (weight > 0.0).then_some((e, weight))
    });
    let total: f32 = candidates.clone().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return None;
    }
    let target = if total > 1.0 { r * total } else { r };
    let mut sum = 0.0;
    for (entry, weight) in candidates {
        sum += weight;
        if sum >= target {
            return u32::try_from(entry.count)
                .ok()
                .filter(|&c| c > 0)
                .map(|count| Filler {
                    class_id: &entry.class_id,
                    count,
                });
        }
    }
    None
}

/// Unity's `Quaternion.Euler(-90, 0, 0)`.
const Z_UP_TO_Y_UP: [f32; 4] = [
    -std::f32::consts::FRAC_1_SQRT_2,
    0.0,
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
];

/// Deterministic random numbers for one slot: SplitMix64 seeded from the
/// world seed, the placeholder's id and the slot's index.
#[derive(Clone, Debug)]
pub struct SlotRng(u64);

impl SlotRng {
    pub fn new(seed: u64, placeholder_id: &str, slot: usize) -> SlotRng {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in placeholder_id
            .bytes()
            .chain(seed.to_le_bytes())
            .chain((slot as u64).to_le_bytes())
        {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
        SlotRng(h)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn value(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform inside the unit sphere.
    pub fn inside_unit_sphere(&mut self) -> [f32; 3] {
        loop {
            let p = [0; 3].map(|_| self.value() * 2.0 - 1.0);
            if p.iter().map(|v| v * v).sum::<f32>() <= 1.0 {
                return p;
            }
        }
    }
}

/// One object a slot spawns.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotSpawn<'a> {
    /// Index of the slot in its placeholder.
    pub slot: usize,
    pub class_id: &'a str,
    /// Relative to the placeholder.
    pub transform: Transform,
    pub info: EntityInfo,
}

/// Fills every slot of a placeholder (`EntitySlotsPlaceholder.Spawn`).
pub fn fill_slots<'a>(
    seed: u64,
    placeholder_id: &str,
    slots: &[EntitySlot],
    loot: &'a LootDistribution,
    infos: &HashMap<String, EntityInfo>,
) -> Vec<SlotSpawn<'a>> {
    let mut out = Vec::new();
    for (i, slot) in slots.iter().enumerate() {
        let mut rng = SlotRng::new(seed, placeholder_id, i);
        let Some(filler) = choose(slot, loot, infos, rng.value()) else {
            continue;
        };
        let Some(info) = infos.get(filler.class_id) else {
            continue;
        };
        let rotation = if info.prefab_z_up {
            quat_mul(slot.rotation, Z_UP_TO_Y_UP)
        } else {
            slot.rotation
        };
        for copy in 0..filler.count {
            let mut position = slot.position;
            if copy > 0 {
                let d = rng.inside_unit_sphere();
                position = [0, 1, 2].map(|a| position[a] + d[a] * EXTRA_COPY_RADIUS);
            }
            out.push(SlotSpawn {
                slot: i,
                class_id: filler.class_id,
                transform: Transform {
                    position,
                    rotation,
                    scale: info.local_scale,
                },
                info: *info,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::encode::*;

    fn slot_bytes(biome: u64, allowed: u64, density: Option<f32>, pos: [f32; 3]) -> Vec<u8> {
        let mut s = Vec::new();
        uint(&mut s, 1, 1);
        uint(&mut s, 2, biome);
        uint(&mut s, 3, allowed);
        if let Some(d) = density {
            float(&mut s, 4, d);
        }
        let mut p = Vec::new();
        for (i, v) in pos.iter().enumerate() {
            float(&mut p, i as u32 + 1, *v);
        }
        bytes(&mut s, 5, &p);
        let mut r = Vec::new();
        float(&mut r, 2, 0.6);
        float(&mut r, 4, 0.8);
        bytes(&mut s, 6, &r);
        s
    }

    fn placeholder() -> Vec<u8> {
        let mut out = Vec::new();
        uint(&mut out, 1, 1);
        bytes(
            &mut out,
            2,
            &slot_bytes(102, 3, Some(4.0), [1.0, -2.0, 3.5]),
        );
        bytes(&mut out, 2, &slot_bytes(121, 16, None, [0.0, 5.0, 0.0]));
        out
    }

    #[test]
    fn slots_parse_with_the_game_defaults() {
        let slots = parse_slots(&placeholder()).unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!(
            slots[0],
            EntitySlot {
                biome: 102,
                allowed: 3,
                density: 4.0,
                position: [1.0, -2.0, 3.5],
                rotation: [0.0, 0.6, 0.0, 0.8],
            }
        );
        assert!(slots[0].allows(SlotKind::Small) && slots[0].allows(SlotKind::Medium));
        assert!(!slots[0].allows(SlotKind::Creature));
        // Absent density reads as the class's initial value.
        assert_eq!(slots[1].density, 1.0);
        assert!(slots[1].is_creature_slot());
    }

    #[test]
    fn corrupt_slots_never_panic() {
        let data = placeholder();
        for i in 0..data.len() {
            let _ = parse_slots(&data[..i]);
            for flip in [0x00, 0xff, 0x80] {
                let mut bad = data.clone();
                bad[i] = flip;
                let _ = parse_slots(&bad);
            }
        }
        assert!(parse_slots(&data[..data.len() - 1]).is_err());
    }

    fn info(kind: SlotKind, z_up: bool) -> EntityInfo {
        EntityInfo {
            kind: Some(kind),
            prefab_z_up: z_up,
            cell_level: 1,
            local_scale: [1.0, 2.0, 1.0],
        }
    }

    fn world() -> (LootDistribution, HashMap<String, EntityInfo>) {
        let loot = LootDistribution::new(vec![
            ("None".into(), vec![(102, 1, 0.5)]),
            ("plant".into(), vec![(102, 1, 0.8), (5, 1, 1.0)]),
            ("rock".into(), vec![(102, 3, 1.6)]),
            ("fish".into(), vec![(102, 1, 4.0)]),
            ("unknown".into(), vec![(102, 1, 4.0)]),
        ]);
        let infos = HashMap::from([
            ("plant".into(), info(SlotKind::Small, false)),
            ("rock".into(), info(SlotKind::Medium, true)),
            ("fish".into(), info(SlotKind::Creature, false)),
        ]);
        (loot, infos)
    }

    fn slot(density: f32) -> EntitySlot {
        EntitySlot {
            biome: 102,
            allowed: SlotKind::Small.flag() | SlotKind::Medium.flag(),
            density,
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn choice_follows_the_game_rule() {
        let (loot, infos) = world();
        // Density 4: plant 0.2, rock 0.4 (the fish is not allowed, "None"
        // and the prefab without info are skipped); total 0.6 < 1, so draws
        // above 0.6 are empty.
        let s = slot(4.0);
        let pick = |r| choose(&s, &loot, &infos, r).map(|f| (f.class_id, f.count));
        assert_eq!(pick(0.0), Some(("plant", 1)));
        assert_eq!(pick(0.2), Some(("plant", 1)));
        assert_eq!(pick(0.21), Some(("rock", 3)));
        assert_eq!(pick(0.6), Some(("rock", 3)));
        assert_eq!(pick(0.61), None);
        // Density 1: weights 0.8 + 1.6 = 2.4 > 1, the draw is scaled.
        let s = slot(1.0);
        let pick = |r| choose(&s, &loot, &infos, r).map(|f| f.class_id);
        assert_eq!(pick(0.33), Some("plant"));
        assert_eq!(pick(0.34), Some("rock"));
        assert_eq!(pick(1.0), Some("rock"));
        // A biome without a table.
        let mut other = slot(1.0);
        other.biome = 7;
        assert_eq!(choose(&other, &loot, &infos, 0.0), None);
    }

    #[test]
    fn copies_and_rotation_as_the_game_places_them() {
        let (loot, infos) = world();
        let mut slots = vec![slot(1.0); 64];
        for (i, s) in slots.iter_mut().enumerate() {
            s.position = [i as f32 * 100.0, 0.0, 0.0];
        }
        let spawns = fill_slots(7, "placeholder", &slots, &loot, &infos);
        let rocks: Vec<_> = spawns.iter().filter(|s| s.class_id == "rock").collect();
        assert!(!rocks.is_empty() && rocks.len() % 3 == 0, "{}", rocks.len());
        for group in rocks.chunks(3) {
            let at = slots[group[0].slot].position;
            assert_eq!(group[0].transform.position, at);
            for s in group {
                let d: f32 = (0..3)
                    .map(|a| (s.transform.position[a] - at[a]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                assert!(d <= EXTRA_COPY_RADIUS + 1e-4, "{d}");
                // Z-up prefab: turned −90° about x.
                let r = s.transform.rotation;
                assert!((r[0] + 0.70710677).abs() < 1e-6 && (r[3] - 0.70710677).abs() < 1e-6);
                assert_eq!(s.transform.scale, [1.0, 2.0, 1.0]);
            }
        }
        let plant = spawns.iter().find(|s| s.class_id == "plant").unwrap();
        assert_eq!(plant.transform.rotation, [0.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn the_same_seed_gives_the_same_world() {
        let (loot, infos) = world();
        let slots = vec![slot(1.5); 200];
        let a = fill_slots(1, "p", &slots, &loot, &infos);
        let b = fill_slots(1, "p", &slots, &loot, &infos);
        let c = fill_slots(2, "p", &slots, &loot, &infos);
        assert_eq!(a, b);
        assert_ne!(a, c);
        // Each slot draws on its own: filling a part gives the same objects.
        let part = fill_slots(1, "p", &slots[..100], &loot, &infos);
        assert_eq!(part[..], a[..part.len()]);
    }

    #[test]
    fn random_values_stay_in_range() {
        let mut rng = SlotRng::new(0, "x", 0);
        for _ in 0..10_000 {
            let v = rng.value();
            assert!((0.0..1.0).contains(&v));
            let p = rng.inside_unit_sphere();
            assert!(p.iter().map(|v| v * v).sum::<f32>() <= 1.0);
        }
    }
}
