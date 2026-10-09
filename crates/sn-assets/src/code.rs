//! What the game keeps only in its code, read from the player's
//! `Assembly-CSharp.dll` with `sn-dotnet` (Phase E, P1): `TechType`
//! names, the crafting menus, `TechData`'s defaults. See
//! `docs/formats/dotnet.md`.

use std::collections::HashMap;

use sn_dotnet::{Assembly, CraftTree, DefaultValue};
use sn_install::GameData;

use crate::Result;

/// The game's code assembly, relative to `Subnautica_Data`.
pub const GAME_ASSEMBLY: &str = "Managed/Assembly-CSharp.dll";

/// `TechData`'s defaults: what an entry means when a key is absent.
#[derive(Clone, Debug, PartialEq)]
pub struct TechDefaults {
    pub item_size: [i32; 2],
    pub background_type: i32,
    pub equipment_type: i32,
    pub slot_type: i32,
    pub craft_time: f32,
    pub craft_amount: i32,
    pub processed: i32,
    pub buildable: bool,
    pub sound_pickup: String,
    pub sound_drop: String,
    pub sound_use: String,
    pub harvest_type: i32,
    pub harvest_output: i32,
    pub harvest_final_cut_bonus: i32,
    pub max_charge: f32,
    pub energy_cost: f32,
    pub powered_prefab: String,
}

impl TechDefaults {
    /// From `TechData`'s `default…` fields (`defaultCraftTime`, …).
    pub fn from_fields(fields: &[(String, DefaultValue)]) -> Result<TechDefaults> {
        let map: HashMap<&str, &DefaultValue> =
            fields.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let get = |name: &str| {
            map.get(name)
                .copied()
                .ok_or_else(|| format!("TechData: no {name}"))
        };
        let int = |name: &str| match get(name)? {
            DefaultValue::Int(v) => Ok(*v),
            other => Err(format!("TechData.{name} is {other:?}")),
        };
        let float = |name: &str| match get(name)? {
            DefaultValue::Float(v) => Ok(*v),
            other => Err(format!("TechData.{name} is {other:?}")),
        };
        let string = |name: &str| match get(name)? {
            DefaultValue::Str(v) => Ok(v.clone()),
            other => Err(format!("TechData.{name} is {other:?}")),
        };
        let item_size = match get("defaultItemSize")? {
            DefaultValue::Pair(x, y) => [*x, *y],
            other => return Err(format!("TechData.defaultItemSize is {other:?}")),
        };
        Ok(TechDefaults {
            item_size,
            background_type: int("defaultBackgroundType")?,
            equipment_type: int("defaultEquipmentType")?,
            slot_type: int("defaultSlotType")?,
            craft_time: float("defaultCraftTime")?,
            craft_amount: int("defaultCraftAmount")?,
            processed: int("defaultProcessed")?,
            buildable: int("defaultBuildable")? != 0,
            sound_pickup: string("defaultSoundPickup")?,
            sound_drop: string("defaultSoundDrop")?,
            sound_use: string("defaultSoundUse")?,
            harvest_type: int("defaultHarvestType")?,
            harvest_output: int("defaultHarvestOutput")?,
            harvest_final_cut_bonus: int("defaultHarvestFinalCutBonus")?,
            max_charge: float("defaultMaxCharge")?,
            energy_cost: float("defaultEnergyCost")?,
            powered_prefab: string("defaultPoweredPrefab")?,
        })
    }
}

/// The game data read from the game's code.
#[derive(Clone, Debug)]
pub struct GameCode {
    /// `TechType` value → name, in declaration order.
    pub tech_types: Vec<(i32, String)>,
    /// `TreeAction` names and values (`None`, `Expand`, `Craft`).
    pub tree_actions: Vec<(String, i64)>,
    pub craft_trees: Vec<CraftTree>,
    pub tech_defaults: TechDefaults,
}

impl GameCode {
    pub fn tech_name(&self, value: i32) -> Option<&str> {
        self.tech_types
            .iter()
            .find(|(v, _)| *v == value)
            .map(|(_, n)| n.as_str())
    }

    /// `TreeAction` value of a name.
    pub fn tree_action(&self, name: &str) -> Option<i32> {
        self.tree_actions
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| i32::try_from(*v).ok())
    }
}

/// Reads the game's code assembly.
pub fn read_assembly(game: &GameData) -> Result<Vec<u8>> {
    Ok(game.read_file(&game.data_dir.join(GAME_ASSEMBLY))?)
}

/// Everything P1 reads from the game's code.
pub fn game_code(bytes: &[u8]) -> Result<GameCode> {
    let e = |e: sn_dotnet::Error| format!("{GAME_ASSEMBLY}: {e}");
    let asm = Assembly::parse(bytes).map_err(e)?;
    Ok(GameCode {
        tech_types: sn_dotnet::tech_type_names(&asm).map_err(e)?,
        tree_actions: sn_dotnet::enum_values(&asm, "", "TreeAction").map_err(e)?,
        craft_trees: sn_dotnet::craft_trees(&asm).map_err(e)?,
        tech_defaults: TechDefaults::from_fields(&sn_dotnet::tech_data_defaults(&asm).map_err(e)?)?,
    })
}
