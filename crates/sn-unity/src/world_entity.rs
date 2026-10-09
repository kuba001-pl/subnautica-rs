//! `UWE.WorldEntityData`: the game's table of what it knows about each world
//! prefab when it fills spawn slots (`Resources/WorldEntities/WorldEntityData`).
//! See `docs/formats/entities.md` § Spawn slots.

use crate::Result;
use crate::objects::MonoBehaviourHeader;
use crate::reader::Reader;

/// One `WorldEntityInfo`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldEntityInfo {
    pub class_id: String,
    /// `TechType` value.
    pub tech_type: i32,
    /// `EntitySlot.Type` index (0 small, 1 medium, 2 large, 3 tall, 4 creature).
    pub slot_type: i32,
    pub prefab_z_up: bool,
    /// `LargeWorldEntity.CellLevel` value.
    pub cell_level: i32,
    pub local_scale: [f32; 3],
}

/// The `WorldEntityData` ScriptableObject: a MonoBehaviour header, then an
/// array of infos (string, 3 × i32 with a bool padded to 4 between them,
/// Vector3). 68 bytes per info with a 36-character ClassId.
pub fn parse_world_entity_data(data: &[u8], big_endian: bool) -> Result<Vec<WorldEntityInfo>> {
    let header = MonoBehaviourHeader::parse(data, big_endian)?;
    let mut r = Reader::new(data, big_endian);
    r.seek(header.fields_offset)?;
    let n = r.count(32)?;
    let mut infos = Vec::with_capacity(n);
    for _ in 0..n {
        let class_id = r.aligned_string()?;
        let tech_type = r.i32()?;
        let slot_type = r.i32()?;
        let prefab_z_up = r.u8()? != 0;
        r.align(4)?;
        let cell_level = r.i32()?;
        let local_scale = [r.f32()?, r.f32()?, r.f32()?];
        infos.push(WorldEntityInfo {
            class_id,
            tech_type,
            slot_type,
            prefab_z_up,
            cell_level,
            local_scale,
        });
    }
    Ok(infos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&0i64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&9i64.to_le_bytes());
        b.extend_from_slice(&4i32.to_le_bytes());
        b.extend_from_slice(b"Data");
        b.extend_from_slice(&2i32.to_le_bytes());
        for (id, kind, z_up, level) in [("abc", 1i32, true, 2i32), ("long-class-id", 4, false, 0)] {
            b.extend_from_slice(&(id.len() as i32).to_le_bytes());
            b.extend_from_slice(id.as_bytes());
            b.resize(b.len().next_multiple_of(4), 0);
            b.extend_from_slice(&7i32.to_le_bytes());
            b.extend_from_slice(&kind.to_le_bytes());
            b.extend_from_slice(&[u8::from(z_up), 0, 0, 0]);
            b.extend_from_slice(&level.to_le_bytes());
            for v in [1.0f32, 0.5, 2.0] {
                b.extend_from_slice(&v.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn reads_infos() {
        let infos = parse_world_entity_data(&sample(), false).unwrap();
        assert_eq!(infos.len(), 2);
        assert_eq!(
            infos[0],
            WorldEntityInfo {
                class_id: "abc".into(),
                tech_type: 7,
                slot_type: 1,
                prefab_z_up: true,
                cell_level: 2,
                local_scale: [1.0, 0.5, 2.0],
            }
        );
        assert_eq!(infos[1].class_id, "long-class-id");
        assert_eq!(infos[1].slot_type, 4);
        assert!(!infos[1].prefab_z_up);
    }

    #[test]
    fn truncated_data_is_an_error() {
        let data = sample();
        for i in 0..data.len() {
            assert!(parse_world_entity_data(&data[..i], false).is_err());
            let mut bad = data.clone();
            bad[i] = 0xff;
            let _ = parse_world_entity_data(&bad, false);
        }
    }
}
