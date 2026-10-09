//! The tables the game fills spawn slots from: the loot distribution
//! (`Resources/Balance/EntityDistributions`, JSON) and the world entity
//! infos (`Resources/WorldEntities/WorldEntityData`). See
//! `docs/formats/entities.md` § Spawn slots.

use std::collections::HashMap;

use sn_unity::json::{Json, parse_json};
use sn_world::{EntityInfo, LootDistribution, SlotKind};

use crate::water::resources;
use crate::{Assets, Result};

const MONO_BEHAVIOUR: i32 = 114;

/// The loot distribution with what the file tells about it.
#[derive(Clone, Debug, Default)]
pub struct LootTable {
    pub distribution: LootDistribution,
    /// Biome names from the comments the game's editor writes above each
    /// row (`// SafeShallows_SandFlat`); only for logs.
    pub biome_names: HashMap<u32, String>,
    /// Prefab entries (including the filler `None`).
    pub prefabs: usize,
    /// (prefab, biome) rows.
    pub rows: usize,
}

impl LootTable {
    /// `name (id)`, or the id alone.
    pub fn biome_label(&self, biome: u32) -> String {
        match self.biome_names.get(&biome) {
            Some(name) => format!("{name} ({biome})"),
            None => biome.to_string(),
        }
    }
}

fn number(row: &Json, key: &str, what: &str) -> Result<f64> {
    match row.get(key) {
        Some(Json::Number(v)) => Ok(*v),
        // LitJson leaves absent fields at their default.
        None => Ok(0.0),
        Some(other) => Err(format!("{what}: {key} is {other:?}, not a number")),
    }
}

/// Parses the distribution JSON: an object of ClassId → `{prefabPath,
/// distribution: [{biome, count, probability}, …]}`.
pub fn parse_loot_table(text: &[u8]) -> Result<LootTable> {
    let doc = parse_json(text).map_err(|e| format!("loot distribution: {e}"))?;
    let Json::Object(entries) = doc else {
        return Err("loot distribution: not a JSON object".into());
    };
    let mut table = LootTable {
        prefabs: entries.len(),
        ..LootTable::default()
    };
    let mut prefabs = Vec::with_capacity(entries.len());
    for (class_id, entry) in entries {
        let rows = match entry.get("distribution") {
            Some(Json::Array(rows)) => rows.as_slice(),
            None | Some(Json::Null) => &[],
            Some(other) => return Err(format!("{class_id}: distribution is {other:?}")),
        };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let biome = number(row, "biome", &class_id)?;
            if biome < 0.0 || biome > f64::from(u32::MAX) || biome.fract() != 0.0 {
                return Err(format!("{class_id}: biome {biome}"));
            }
            let count = number(row, "count", &class_id)? as i32;
            let probability = number(row, "probability", &class_id)? as f32;
            out.push((biome as u32, count, probability));
        }
        table.rows += out.len();
        prefabs.push((class_id, out));
    }
    table.distribution = LootDistribution::new(prefabs);
    table.biome_names = biome_names(&String::from_utf8_lossy(text));
    Ok(table)
}

/// `// Name` directly above a `"biome" : 102,` line.
fn biome_names(text: &str) -> HashMap<u32, String> {
    let mut names = HashMap::new();
    let mut comment: Option<&str> = None;
    for line in text.lines().map(str::trim) {
        if let Some(c) = line.strip_prefix("//") {
            comment = Some(c.trim());
            continue;
        }
        if let (Some(name), Some(rest)) = (comment, line.strip_prefix("\"biome\"")) {
            let value = rest.trim_start().trim_start_matches(':').trim();
            if let Ok(id) = value.trim_end_matches(',').trim().parse::<u32>() {
                names.entry(id).or_insert_with(|| name.to_string());
            }
        }
        if !line.is_empty() {
            comment = None;
        }
    }
    names
}

/// The game's loot distribution (`Balance/EntityDistributions`).
pub fn loot_table(assets: &Assets) -> Result<LootTable> {
    parse_loot_table(&crate::resource_bytes(
        assets,
        "balance/entitydistributions",
    )?)
}

/// ClassId → what the game knows about the prefab when filling slots
/// (`WorldEntities/WorldEntityData`).
pub fn entity_infos(assets: &Assets) -> Result<HashMap<String, EntityInfo>> {
    let path = "worldentities/worldentitydata";
    let (file, found) = resources(assets, path)?;
    for pptr in found {
        let Some(object) = assets.resolve(&file, pptr)? else {
            continue;
        };
        let (info, data) = object.data()?;
        if info.class_id != MONO_BEHAVIOUR {
            continue;
        }
        let infos = sn_unity::parse_world_entity_data(data, object.file.file().big_endian)
            .map_err(|e| format!("{path}: {e}"))?;
        return Ok(infos
            .into_iter()
            .map(|i| {
                let info = EntityInfo {
                    kind: SlotKind::from_index(i.slot_type),
                    prefab_z_up: i.prefab_z_up,
                    cell_level: i.cell_level,
                    local_scale: i.local_scale,
                };
                (i.class_id, info)
            })
            .collect());
    }
    Err(format!("no MonoBehaviour resource {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"// Use the editor instead of modifying this file directly

{
    "None" : {
        "prefabPath" : "None",
        "distribution" : [
            {
                // Desert_Sand
                "biome" : 900,
                "count" : 1,
                "probability" : 0.8
            }
        ]
    },
    "id-a" : {
        "prefabPath" : "WorldEntities/A",
        "distribution" : [
            {
                // Desert_Sand
                "biome" : 900,
                "count" : 2,
                "probability" : 0.25
            },
            {
                "biome" : 5,
                "count" : 1,
                "probability" : 1
            }
        ]
    },
    "id-b" : { "prefabPath" : "WorldEntities/B", "distribution" : null }
}
"#;

    #[test]
    fn reads_the_distribution_and_biome_names() {
        let t = parse_loot_table(SAMPLE.as_bytes()).unwrap();
        assert_eq!((t.prefabs, t.rows), (3, 3));
        let desert = t.distribution.biome(900).unwrap();
        assert_eq!(desert.len(), 2);
        assert_eq!(desert[0].class_id, "None");
        assert_eq!((desert[1].count, desert[1].probability), (2, 0.25));
        assert_eq!(t.distribution.biome(5).unwrap()[0].class_id, "id-a");
        assert_eq!(t.biome_label(900), "Desert_Sand (900)");
        // No comment above biome 5's row.
        assert_eq!(t.biome_label(5), "5");
    }

    #[test]
    fn bad_rows_are_errors() {
        let bad = SAMPLE.replace("\"biome\" : 5,", "\"biome\" : -5,");
        assert!(parse_loot_table(bad.as_bytes()).is_err());
        let bad = SAMPLE.replace("\"count\" : 2,", "\"count\" : \"2\",");
        assert!(parse_loot_table(bad.as_bytes()).is_err());
        assert!(parse_loot_table(b"[]").is_err());
    }
}
