//! The game's scripts about items (M9d1, `docs/DESIGN.md` § 4.3 "M9d
//! plan"): what can be picked up, what an object is, the player's luck per
//! tech type, the lifepod's locker and its starting supplies. Same rules
//! as `gameplay.rs`: fields in declaration order, Unity 2019.4
//! serialization, and every parser requires the data to end with its last
//! field. See `docs/formats/gameplay.md` § Pick-ups and outcrops.

use crate::Result;
use crate::gameplay::{at_end, fields, unity_event};
use crate::objects::PPtr;

/// `Pickupable`'s serialized fields (`HandTarget`, its base, has none;
/// the `[NonSerialized]` protobuf fields are not in the asset).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pickupable {
    pub is_pickupable: bool,
    pub destroy_on_death: bool,
    pub randomize_rotation_when_dropped: bool,
    pub activate_rigidbody_when_dropped: bool,
    pub use_pack_up_icon: bool,
}

impl Pickupable {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<Pickupable> {
        let mut r = fields(data, big_endian)?;
        let p = Pickupable {
            is_pickupable: r.bool4()?,
            destroy_on_death: r.bool4()?,
            randomize_rotation_when_dropped: r.bool4()?,
            activate_rigidbody_when_dropped: r.bool4()?,
            use_pack_up_icon: r.bool4()?,
        };
        at_end(&r, data)?;
        Ok(p)
    }
}

/// `TechTag.type`: the tech type an object says it is.
pub fn parse_tech_tag(data: &[u8], big_endian: bool) -> Result<i32> {
    let mut r = fields(data, big_endian)?;
    let t = r.i32()?;
    at_end(&r, data)?;
    Ok(t)
}

/// `PrefabIdentifier` (`UniqueIdentifier`'s serialized fields): the
/// prefab's class id. The object's own id is not serialized in the asset
/// (measured: the data ends after the class id).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrefabIdentifier {
    pub class_id: String,
}

impl PrefabIdentifier {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PrefabIdentifier> {
        let mut r = fields(data, big_endian)?;
        let p = PrefabIdentifier {
            class_id: r.aligned_string()?,
        };
        at_end(&r, data)?;
        Ok(p)
    }
}

/// `PlayerEntropy.randomizers`: (tech type, its `FairRandomizer`).
pub fn parse_player_entropy(data: &[u8], big_endian: bool) -> Result<Vec<(i32, PPtr)>> {
    let mut r = fields(data, big_endian)?;
    let n = r.count(16)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push((r.i32()?, PPtr::read(&mut r)?));
    }
    at_end(&r, data)?;
    Ok(out)
}

/// `FairRandomizer`'s serialized fields (`version` is `[NonSerialized]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FairRandomizer {
    pub time_last_check: f32,
    pub entropy: f32,
}

impl FairRandomizer {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<FairRandomizer> {
        let mut r = fields(data, big_endian)?;
        let f = FairRandomizer {
            time_last_check: r.f32()?,
            entropy: r.f32()?,
        };
        at_end(&r, data)?;
        Ok(f)
    }
}

/// `LootSpawner`: the lifepod locker's starting items and the supply
/// crates' pool, as tech types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LootSpawner {
    pub escape_pod: Vec<i32>,
    pub supply: Vec<i32>,
}

impl LootSpawner {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<LootSpawner> {
        let mut r = fields(data, big_endian)?;
        let l = LootSpawner {
            escape_pod: r.i32_list()?,
            supply: r.i32_list()?,
        };
        at_end(&r, data)?;
        Ok(l)
    }
}

/// `StorageContainer`: a locker's grid and texts.
#[derive(Clone, Debug, PartialEq)]
pub struct StorageContainer {
    pub prefab_root: PPtr,
    pub width: i32,
    pub height: i32,
    /// Language keys.
    pub hover_text: String,
    pub storage_label: String,
    /// `onUse`'s persistent listeners.
    pub on_use: usize,
    pub storage_root: PPtr,
    pub prevent_deconstruction_if_not_empty: bool,
}

impl StorageContainer {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<StorageContainer> {
        let mut r = fields(data, big_endian)?;
        let prefab_root = PPtr::read(&mut r)?;
        let width = r.i32()?;
        let height = r.i32()?;
        let hover_text = r.aligned_string()?;
        let storage_label = r.aligned_string()?;
        let on_use = unity_event(&mut r)?.len();
        PPtr::read(&mut r)?; // errorSound
        let s = StorageContainer {
            prefab_root,
            width,
            height,
            hover_text,
            storage_label,
            on_use,
            storage_root: PPtr::read(&mut r)?,
            prevent_deconstruction_if_not_empty: r.bool4()?,
        };
        at_end(&r, data)?;
        Ok(s)
    }
}

/// `SpawnEscapePodSupplies.storageContainer`: the locker a new game
/// fills.
pub fn parse_spawn_escape_pod_supplies(data: &[u8], big_endian: bool) -> Result<PPtr> {
    let mut r = fields(data, big_endian)?;
    let p = PPtr::read(&mut r)?;
    at_end(&r, data)?;
    Ok(p)
}

/// `PrefabListSpawner.prefabs`: the prefabs it instantiates on `Awake`.
pub fn parse_prefab_list_spawner(data: &[u8], big_endian: bool) -> Result<Vec<PPtr>> {
    let mut r = fields(data, big_endian)?;
    let n = r.count(12)?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(PPtr::read(&mut r)?);
    }
    at_end(&r, data)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Little-endian writer for synthetic objects.
    #[derive(Default)]
    struct W(Vec<u8>);

    impl W {
        /// A MonoBehaviour header with an empty name.
        fn behaviour() -> W {
            let mut w = W::default();
            w.pptr(0, 1).u8(1).align().pptr(1, 2).string("");
            w
        }
        fn u8(&mut self, v: u8) -> &mut W {
            self.0.push(v);
            self
        }
        fn align(&mut self) -> &mut W {
            self.0.resize(self.0.len().next_multiple_of(4), 0);
            self
        }
        fn bool4(&mut self, v: bool) -> &mut W {
            self.u8(u8::from(v)).align()
        }
        fn i32(&mut self, v: i32) -> &mut W {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn f32(&mut self, v: f32) -> &mut W {
            self.0.extend_from_slice(&v.to_le_bytes());
            self
        }
        fn pptr(&mut self, file: i32, path: i64) -> &mut W {
            self.i32(file);
            self.0.extend_from_slice(&path.to_le_bytes());
            self
        }
        fn string(&mut self, s: &str) -> &mut W {
            self.i32(s.len() as i32);
            self.0.extend_from_slice(s.as_bytes());
            self.align()
        }
    }

    /// Every prefix is an error, a trailing byte is an error, and no byte
    /// change panics.
    fn robust<T>(data: &[u8], parse: impl Fn(&[u8]) -> Result<T>) {
        for i in 0..data.len() {
            assert!(parse(&data[..i]).is_err(), "prefix {i} parsed");
            let mut bad = data.to_vec();
            bad[i] = 0xff;
            let _ = parse(&bad);
        }
        let mut long = data.to_vec();
        long.push(0);
        assert!(parse(&long).is_err(), "trailing byte parsed");
    }

    #[test]
    fn pickupable() {
        let mut w = W::behaviour();
        w.bool4(true)
            .bool4(false)
            .bool4(true)
            .bool4(true)
            .bool4(false);
        let p = Pickupable::parse(&w.0, false).unwrap();
        assert!(p.is_pickupable && !p.destroy_on_death && p.randomize_rotation_when_dropped);
        assert!(p.activate_rigidbody_when_dropped && !p.use_pack_up_icon);
        robust(&w.0, |d| Pickupable::parse(d, false));
    }

    #[test]
    fn tech_tag_and_identifier() {
        let mut w = W::behaviour();
        w.i32(1043);
        assert_eq!(parse_tech_tag(&w.0, false).unwrap(), 1043);
        robust(&w.0, |d| parse_tech_tag(d, false));

        let mut w = W::behaviour();
        w.string("class-id");
        let p = PrefabIdentifier::parse(&w.0, false).unwrap();
        assert_eq!(p.class_id, "class-id");
        robust(&w.0, |d| PrefabIdentifier::parse(d, false));
    }

    #[test]
    fn entropy() {
        let mut w = W::behaviour();
        w.i32(2).i32(10).pptr(0, 5).i32(11).pptr(0, 6);
        let e = parse_player_entropy(&w.0, false).unwrap();
        assert_eq!(e.len(), 2);
        assert_eq!((e[1].0, e[1].1.path_id), (11, 6));
        robust(&w.0, |d| parse_player_entropy(d, false));

        let mut w = W::behaviour();
        w.f32(-1.0).f32(0.25);
        let f = FairRandomizer::parse(&w.0, false).unwrap();
        assert_eq!((f.time_last_check, f.entropy), (-1.0, 0.25));
        robust(&w.0, |d| FairRandomizer::parse(d, false));
    }

    #[test]
    fn loot_spawner() {
        let mut w = W::behaviour();
        w.i32(2).i32(1).i32(2).i32(1).i32(3);
        let l = LootSpawner::parse(&w.0, false).unwrap();
        assert_eq!((l.escape_pod, l.supply), (vec![1, 2], vec![3]));
        robust(&w.0, |d| LootSpawner::parse(d, false));
    }

    #[test]
    fn storage_container() {
        let mut w = W::behaviour();
        w.pptr(0, 3)
            .i32(4)
            .i32(5)
            .string("OpenStorage")
            .string("Label");
        // onUse: one listener.
        w.i32(1)
            .pptr(0, 9)
            .string("Open")
            .i32(1)
            .pptr(0, 0)
            .string("")
            .i32(0)
            .f32(0.0)
            .string("")
            .bool4(false)
            .i32(2);
        w.pptr(0, 0).pptr(0, 7).bool4(true);
        let s = StorageContainer::parse(&w.0, false).unwrap();
        assert_eq!((s.width, s.height, s.on_use), (4, 5, 1));
        assert_eq!(s.hover_text, "OpenStorage");
        assert_eq!(s.storage_root.path_id, 7);
        assert!(s.prevent_deconstruction_if_not_empty);
        robust(&w.0, |d| StorageContainer::parse(d, false));
    }

    #[test]
    fn prefab_list_spawner() {
        let mut w = W::behaviour();
        w.i32(2).pptr(1, 0x95).pptr(1, 0x165);
        let l = parse_prefab_list_spawner(&w.0, false).unwrap();
        assert_eq!(
            l.iter().map(|p| p.path_id).collect::<Vec<_>>(),
            [0x95, 0x165]
        );
        robust(&w.0, |d| parse_prefab_list_spawner(d, false));

        let mut w = W::behaviour();
        w.pptr(0, 12);
        assert_eq!(
            parse_spawn_escape_pod_supplies(&w.0, false)
                .unwrap()
                .path_id,
            12
        );
        robust(&w.0, |d| parse_spawn_escape_pod_supplies(d, false));
    }
}
