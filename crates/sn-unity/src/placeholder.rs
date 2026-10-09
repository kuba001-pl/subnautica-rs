//! The game's `PrefabPlaceholder` and `PrefabPlaceholdersGroup` scripts:
//! prefabs the game spawns into a prefab when it first starts
//! (`docs/DESIGN.md` M7h). Serialized fields, from the classes: the
//! placeholder's `string prefabClassId, bool highPriority`, the group's
//! `PrefabPlaceholder[] prefabPlaceholders` (the rest is `[NonSerialized]`
//! or an `Action`). Both parsers require the data to end with the last
//! field, which checks the layout on every object read.

use crate::ErrorKind;
use crate::Result;
use crate::objects::{MonoBehaviourHeader, PPtr};
use crate::reader::Reader;

/// `PrefabPlaceholder`: where a prefab is spawned (the placeholder's own
/// GameObject gives the place).
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabPlaceholder {
    pub game_object: PPtr,
    /// The prefab to spawn (a world entity's ClassId).
    pub prefab_class_id: String,
    pub high_priority: bool,
}

impl PrefabPlaceholder {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PrefabPlaceholder> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let prefab_class_id = r.aligned_string()?;
        let high_priority = r.u8()? != 0;
        r.align(4)?;
        at_end(&r, data)?;
        Ok(PrefabPlaceholder {
            game_object: header.game_object,
            prefab_class_id,
            high_priority,
        })
    }
}

/// `PrefabPlaceholdersGroup`: the placeholders its `Start` spawns, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct PrefabPlaceholdersGroup {
    pub game_object: PPtr,
    /// `Start` runs only for an enabled component.
    pub enabled: bool,
    /// The `PrefabPlaceholder` components.
    pub placeholders: Vec<PPtr>,
}

impl PrefabPlaceholdersGroup {
    pub fn parse(data: &[u8], big_endian: bool) -> Result<PrefabPlaceholdersGroup> {
        let header = MonoBehaviourHeader::parse(data, big_endian)?;
        let mut r = Reader::new(data, big_endian);
        r.seek(header.fields_offset)?;
        let n = r.count(12)?;
        let mut placeholders = Vec::with_capacity(n);
        for _ in 0..n {
            placeholders.push(PPtr::read(&mut r)?);
        }
        r.align(4)?;
        at_end(&r, data)?;
        Ok(PrefabPlaceholdersGroup {
            game_object: header.game_object,
            enabled: header.enabled,
            placeholders,
        })
    }
}

fn at_end(r: &Reader, data: &[u8]) -> Result<()> {
    if r.pos() != data.len() {
        return Err(r.error(ErrorKind::Invalid(format!(
            "{} bytes after the last field",
            data.len() - r.pos()
        ))));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MonoBehaviour header (game object 5, enabled, script 9, no name).
    fn header(b: &mut Vec<u8>) {
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&5i64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&9i64.to_le_bytes());
        b.extend_from_slice(&0i32.to_le_bytes());
    }

    fn placeholder_bytes(class_id: &str) -> Vec<u8> {
        let mut b = Vec::new();
        header(&mut b);
        b.extend_from_slice(&(class_id.len() as i32).to_le_bytes());
        b.extend_from_slice(class_id.as_bytes());
        while b.len() % 4 != 0 {
            b.push(0);
        }
        b.extend_from_slice(&[1, 0, 0, 0]);
        b
    }

    #[test]
    fn reads_a_placeholder() {
        let p = PrefabPlaceholder::parse(&placeholder_bytes("abc-12"), false).unwrap();
        assert_eq!(p.game_object.path_id, 5);
        assert_eq!(p.prefab_class_id, "abc-12");
        assert!(p.high_priority);
    }

    #[test]
    fn reads_a_group() {
        let mut b = Vec::new();
        header(&mut b);
        b.extend_from_slice(&2i32.to_le_bytes());
        for id in [11i64, 12] {
            b.extend_from_slice(&0i32.to_le_bytes());
            b.extend_from_slice(&id.to_le_bytes());
        }
        let g = PrefabPlaceholdersGroup::parse(&b, false).unwrap();
        assert!(g.enabled);
        let ids: Vec<i64> = g.placeholders.iter().map(|p| p.path_id).collect();
        assert_eq!(ids, [11, 12]);
    }

    #[test]
    fn wrong_lengths_are_errors() {
        let b = placeholder_bytes("abc-12");
        for len in [0, 20, 32, b.len() - 1] {
            assert!(PrefabPlaceholder::parse(&b[..len], false).is_err(), "{len}");
        }
        // Trailing bytes mean another layout.
        let mut longer = b.clone();
        longer.extend_from_slice(&[0; 4]);
        assert!(PrefabPlaceholder::parse(&longer, false).is_err());
    }
}
