//! Gameplay data read from the install (Phase E, P0): recipes and item data
//! (`Balance/TechData`, JSON), prefab ↔ tech type (`EntTechData`), and the
//! player's components in the main scene with the `PDAData` they point to.
//! See `docs/formats/gameplay.md`.

use std::collections::{BTreeMap, HashSet};

use sn_unity::json::{Json, parse_json};
use sn_unity::{
    EntTechEntry, GroundMotor, LiveMixin, LiveMixinData, MainCameraControl, Oxygen,
    PHYSICS_MANAGER, PdaData, PhysicsManager, PlayerController, PlayerFields, RIGIDBODY, Rigidbody,
    TAG_MANAGER, TIME_MANAGER, TagManager, TimeManager, UnderwaterMotor, parse_ent_tech_data,
};

use crate::prefab::PrefabNode;
use crate::water::resources;
use crate::{Assets, ObjectRef, Result};

const MONO_BEHAVIOUR: i32 = 114;
const GAME_OBJECT: i32 = 1;

/// One ingredient of a recipe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ingredient {
    pub tech_type: i32,
    pub amount: i32,
}

/// One entry of `Balance/TechData`. `None`: the key is absent and the game
/// uses its default (`TechData.defaults`, in the code; read in P1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TechEntry {
    pub tech_type: i32,
    /// Inventory cells (x, y).
    pub item_size: Option<[i32; 2]>,
    pub background_type: Option<i32>,
    pub equipment_type: Option<i32>,
    pub slot_type: Option<i32>,
    /// Seconds.
    pub craft_time: Option<f32>,
    pub craft_amount: Option<i32>,
    pub ingredients: Vec<Ingredient>,
    /// Extra items a craft gives.
    pub linked_items: Vec<i32>,
    pub processed: Option<i32>,
    pub buildable: Option<bool>,
    pub sound_pickup: Option<String>,
    pub sound_drop: Option<String>,
    pub sound_use: Option<String>,
    pub harvest_type: Option<i32>,
    pub harvest_output: Option<i32>,
    pub harvest_final_cut_bonus: Option<i32>,
    pub max_charge: Option<f32>,
    pub energy_cost: Option<f32>,
    pub powered_prefab: Option<String>,
}

/// `Balance/TechData`.
#[derive(Clone, Debug, Default)]
pub struct TechData {
    /// In file order.
    pub entries: Vec<TechEntry>,
    /// Entries without `techType` (the game skips them).
    pub without_tech_type: usize,
    /// Tech types listed more than once (the game's `Dictionary.Add` would
    /// throw on these).
    pub duplicates: Vec<i32>,
    /// Keys the game's reader does not know, with how often they occur.
    pub unknown_keys: BTreeMap<String, usize>,
}

impl TechData {
    pub fn get(&self, tech_type: i32) -> Option<&TechEntry> {
        self.entries.iter().find(|e| e.tech_type == tech_type)
    }

    /// Entries with a recipe (at least one ingredient or linked item).
    pub fn recipes(&self) -> impl Iterator<Item = &TechEntry> {
        self.entries
            .iter()
            .filter(|e| !e.ingredients.is_empty() || !e.linked_items.is_empty())
    }

    /// (recipe, ingredient) pairs whose ingredient has no entry of its
    /// own. Expected for items whose fields are all defaults: the game's
    /// editor trims those entries (`TechData.TrimDefaults`).
    pub fn ingredients_without_entry(&self) -> Vec<(i32, i32)> {
        let known: HashSet<i32> = self.entries.iter().map(|e| e.tech_type).collect();
        let mut out = Vec::new();
        for e in &self.entries {
            for i in &e.ingredients {
                if !known.contains(&i.tech_type) {
                    out.push((e.tech_type, i.tech_type));
                }
            }
        }
        out
    }
}

/// The keys `TechData` reads (`TechData.property*`).
const KNOWN_KEYS: &[&str] = &[
    "techType",
    "itemSize",
    "backgroundType",
    "equipmentType",
    "slotType",
    "craftTime",
    "craftAmount",
    "ingredients",
    "linkedItems",
    "processed",
    "buildable",
    "soundPickup",
    "soundDrop",
    "soundUse",
    "harvestType",
    "harvestOutput",
    "harvestFinalCutBonus",
    "maxCharge",
    "energyCost",
    "poweredPrefab",
];

fn int(v: &Json, what: &str) -> Result<i32> {
    match v {
        Json::Number(n)
            if n.fract() == 0.0 && *n >= f64::from(i32::MIN) && *n <= f64::from(i32::MAX) =>
        {
            Ok(*n as i32)
        }
        other => Err(format!("{what}: {other:?} is not an integer")),
    }
}

fn opt<T>(
    entry: &Json,
    key: &str,
    what: &str,
    read: impl Fn(&Json, &str) -> Result<T>,
) -> Result<Option<T>> {
    match entry.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => read(v, &format!("{what} {key}")).map(Some),
    }
}

fn float(v: &Json, what: &str) -> Result<f32> {
    match v {
        Json::Number(n) => Ok(*n as f32),
        other => Err(format!("{what}: {other:?} is not a number")),
    }
}

fn string(v: &Json, what: &str) -> Result<String> {
    v.as_str()
        .map(String::from)
        .ok_or_else(|| format!("{what}: {v:?} is not a string"))
}

fn boolean(v: &Json, what: &str) -> Result<bool> {
    match v {
        Json::Bool(b) => Ok(*b),
        other => Err(format!("{what}: {other:?} is not a bool")),
    }
}

fn array<'a>(entry: &'a Json, key: &str, what: &str) -> Result<&'a [Json]> {
    match entry.get(key) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::Array(items)) => Ok(items),
        Some(other) => Err(format!("{what} {key}: {other:?} is not an array")),
    }
}

/// Parses `Balance/TechData`: `{"entries": [{"techType": n, …}, …]}`.
pub fn parse_tech_data(text: &[u8]) -> Result<TechData> {
    let doc = parse_json(text).map_err(|e| format!("TechData: {e}"))?;
    let Some(Json::Array(items)) = doc.get("entries") else {
        return Err("TechData: no \"entries\" array".into());
    };
    let mut data = TechData::default();
    let mut seen = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        let Json::Object(keys) = item else {
            return Err(format!("TechData entry {index}: not an object"));
        };
        for (key, _) in keys {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                *data.unknown_keys.entry(key.clone()).or_insert(0) += 1;
            }
        }
        let what = format!("TechData entry {index}");
        let Some(tech_type) = opt(item, "techType", &what, int)? else {
            data.without_tech_type += 1;
            continue;
        };
        let what = format!("TechData {tech_type}");
        if !seen.insert(tech_type) {
            data.duplicates.push(tech_type);
        }
        let item_size = opt(item, "itemSize", &what, |v, w| {
            let x = v.get("x").map(|x| int(x, w)).transpose()?.unwrap_or(1);
            let y = v.get("y").map(|y| int(y, w)).transpose()?.unwrap_or(1);
            Ok([x, y])
        })?;
        let mut ingredients = Vec::new();
        for i in array(item, "ingredients", &what)? {
            let tech_type = i
                .get("techType")
                .ok_or_else(|| format!("{what}: ingredient without techType"))?;
            ingredients.push(Ingredient {
                tech_type: int(tech_type, &what)?,
                amount: match i.get("amount") {
                    Some(a) => int(a, &what)?,
                    None => 1,
                },
            });
        }
        let linked_items = array(item, "linkedItems", &what)?
            .iter()
            .map(|v| int(v, &what))
            .collect::<Result<_>>()?;
        data.entries.push(TechEntry {
            tech_type,
            item_size,
            background_type: opt(item, "backgroundType", &what, int)?,
            equipment_type: opt(item, "equipmentType", &what, int)?,
            slot_type: opt(item, "slotType", &what, int)?,
            craft_time: opt(item, "craftTime", &what, float)?,
            craft_amount: opt(item, "craftAmount", &what, int)?,
            ingredients,
            linked_items,
            processed: opt(item, "processed", &what, int)?,
            buildable: opt(item, "buildable", &what, boolean)?,
            sound_pickup: opt(item, "soundPickup", &what, string)?,
            sound_drop: opt(item, "soundDrop", &what, string)?,
            sound_use: opt(item, "soundUse", &what, string)?,
            harvest_type: opt(item, "harvestType", &what, int)?,
            harvest_output: opt(item, "harvestOutput", &what, int)?,
            harvest_final_cut_bonus: opt(item, "harvestFinalCutBonus", &what, int)?,
            max_charge: opt(item, "maxCharge", &what, float)?,
            energy_cost: opt(item, "energyCost", &what, float)?,
            powered_prefab: opt(item, "poweredPrefab", &what, string)?,
        });
    }
    Ok(data)
}

/// The game's `Balance/TechData`.
pub fn tech_data(assets: &Assets) -> Result<TechData> {
    parse_tech_data(&crate::resource_bytes(assets, "balance/techdata")?)
}

/// The game's `EntTechData` (prefab name → tech type).
pub fn ent_tech_data(assets: &Assets) -> Result<Vec<EntTechEntry>> {
    let path = "enttechdata";
    let (file, found) = resources(assets, path)?;
    for pptr in found {
        let Some(object) = assets.resolve(&file, pptr)? else {
            continue;
        };
        let (info, data) = object.data()?;
        if info.class_id == MONO_BEHAVIOUR {
            return parse_ent_tech_data(data, object.file.file().big_endian)
                .map_err(|e| format!("{path}: {e}"));
        }
    }
    Err(format!("no resource {path}"))
}

/// The player as the main scene holds it.
#[derive(Clone, Debug)]
pub struct PlayerData {
    pub player: PlayerFields,
    /// The player's own `Oxygen` (`isPlayer`).
    pub oxygen: Oxygen,
    pub live_mixin: LiveMixin,
    pub live_mixin_data: LiveMixinData,
    pub underwater_motor: UnderwaterMotor,
    pub ground_motor: GroundMotor,
    pub controller: PlayerController,
    pub pda: PdaData,
    /// The player's GameObject's physics layer.
    pub player_layer: u32,
    /// The player's `Rigidbody` (used by `UnderwaterMotor`).
    pub rigidbody: Rigidbody,
    /// `Ocean.GetOceanLevel`: the y of the `Ocean` object.
    pub ocean_level: f32,
    /// How far the player's `Oxygen` object is above the `Player` object
    /// (`OxygenManager` tests the source's height, M9c).
    pub oxygen_above_player: f32,
    /// The mouse look (M9c).
    pub camera: MainCameraControl,
    /// Where `MainCameraControl`'s object sits relative to the player's
    /// in the scene, and its node's name (M9c, logged).
    pub camera_node: (String, [f32; 3]),
}

fn one(assets: &Assets, scene: &crate::Scene, class: &str) -> Result<ObjectRef> {
    let mut found = scene.behaviours(assets, class);
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(format!("main scene: {n} {class} behaviours, expected 1")),
    }
}

/// Reads the player's components from the main scene and the `PDAData`
/// it points to.
pub fn player_data(assets: &Assets) -> Result<PlayerData> {
    let scene = assets.scene("main")?;
    let big_endian = scene.file.file().big_endian;
    let read = |o: &ObjectRef| -> Result<Vec<u8>> { Ok(o.data()?.1.to_vec()) };
    let player_ref = one(assets, &scene, "Player")?;
    let player =
        PlayerFields::parse(&read(&player_ref)?, big_endian).map_err(|e| format!("Player: {e}"))?;

    let mut oxygen = None;
    for o in scene.behaviours(assets, "Oxygen") {
        let ox = Oxygen::parse(&read(&o)?, big_endian).map_err(|e| format!("Oxygen: {e}"))?;
        if ox.is_player {
            oxygen = Some(ox);
        }
    }
    let oxygen = oxygen.ok_or("main scene: no Oxygen with isPlayer")?;

    let resolve = |pptr, what: &str| -> Result<ObjectRef> {
        assets
            .resolve(&player_ref.file, pptr)?
            .ok_or_else(|| format!("Player.{what} is null"))
    };
    let live = resolve(player.live_mixin, "liveMixin")?;
    let live_mixin =
        LiveMixin::parse(&read(&live)?, big_endian).map_err(|e| format!("LiveMixin: {e}"))?;
    let data_ref = assets
        .resolve(&live.file, live_mixin.data)?
        .ok_or("LiveMixin.data is null")?;
    if assets.script_class(&data_ref).as_deref() != Some("LiveMixinData") {
        return Err("LiveMixin.data is not a LiveMixinData".into());
    }
    let live_mixin_data = LiveMixinData::parse(&read(&data_ref)?, data_ref.file.file().big_endian)
        .map_err(|e| format!("LiveMixinData: {e}"))?;

    let underwater_motor =
        UnderwaterMotor::parse(&read(&one(assets, &scene, "UnderwaterMotor")?)?, big_endian)
            .map_err(|e| format!("UnderwaterMotor: {e}"))?;
    let ground = resolve(player.ground_motor, "groundMotor")?;
    if assets.script_class(&ground).as_deref() != Some("GroundMotor") {
        return Err("Player.groundMotor is not a GroundMotor".into());
    }
    let ground_motor =
        GroundMotor::parse(&read(&ground)?, big_endian).map_err(|e| format!("GroundMotor: {e}"))?;
    let (r, n) = scene
        .behaviour_node(assets, &player_ref)?
        .ok_or("Player: object not in the main scene")?;
    let player_layer = scene.roots[r].nodes[n].layer;
    let player_world = scene.roots[r].world(n).position;
    let mut rigidbody = None;
    for c in assets.node_components(&scene.roots[r].nodes[n])? {
        let (info, data) = c.data()?;
        if info.class_id == RIGIDBODY {
            rigidbody = Some(
                Rigidbody::parse(data, c.file.file().big_endian)
                    .map_err(|e| format!("player Rigidbody: {e}"))?,
            );
        }
    }
    let rigidbody = rigidbody.ok_or("the player has no Rigidbody")?;
    let ocean = one(assets, &scene, "Ocean")?;
    let (r, n) = scene
        .behaviour_node(assets, &ocean)?
        .ok_or("Ocean: object not in the main scene")?;
    let ocean_level = scene.roots[r].world(n).position[1];
    let controller = PlayerController::parse(
        &read(&one(assets, &scene, "PlayerController")?)?,
        big_endian,
    )
    .map_err(|e| format!("PlayerController: {e}"))?;

    let node_offset = |o: &ObjectRef, what: &str| -> Result<(String, [f32; 3])> {
        let (rr, nn) = scene
            .behaviour_node(assets, o)?
            .ok_or_else(|| format!("{what}: object not in the main scene"))?;
        let p = scene.roots[rr].world(nn).position;
        Ok((
            scene.roots[rr].nodes[nn].name.clone(),
            [
                p[0] - player_world[0],
                p[1] - player_world[1],
                p[2] - player_world[2],
            ],
        ))
    };
    let mut oxygen_above_player = None;
    for o in scene.behaviours(assets, "Oxygen") {
        if Oxygen::parse(&read(&o)?, big_endian).is_ok_and(|ox| ox.is_player) {
            oxygen_above_player = Some(node_offset(&o, "Oxygen")?.1[1]);
        }
    }
    let oxygen_above_player = oxygen_above_player.ok_or("main scene: no Oxygen with isPlayer")?;
    let camera_ref = one(assets, &scene, "MainCameraControl")?;
    let camera = MainCameraControl::parse(&read(&camera_ref)?, big_endian)
        .map_err(|e| format!("MainCameraControl: {e}"))?;
    let camera_node = node_offset(&camera_ref, "MainCameraControl")?;

    let pda_ref = resolve(player.pda_data, "pdaData")?;
    if assets.script_class(&pda_ref).as_deref() != Some("PDAData") {
        return Err("Player.pdaData is not a PDAData".into());
    }
    let pda = PdaData::parse(&read(&pda_ref)?, pda_ref.file.file().big_endian)
        .map_err(|e| format!("PDAData: {e}"))?;
    Ok(PlayerData {
        player,
        oxygen,
        live_mixin,
        live_mixin_data,
        underwater_motor,
        ground_motor,
        controller,
        player_layer,
        rigidbody,
        ocean_level,
        oxygen_above_player,
        camera,
        camera_node,
        pda,
    })
}

/// The project's physics, time and layer settings (`globalgamemanagers`).
#[derive(Clone, Debug)]
pub struct PhysicsSettings {
    pub physics: PhysicsManager,
    pub time: TimeManager,
    pub tags: TagManager,
}

pub fn physics_settings(assets: &Assets) -> Result<PhysicsSettings> {
    let ggm = assets.standalone("globalgamemanagers")?;
    let big_endian = ggm.file().big_endian;
    let data = |class: i32, name: &str| -> Result<Vec<u8>> {
        let info = ggm
            .objects()
            .iter()
            .find(|o| o.class_id == class)
            .ok_or_else(|| format!("globalgamemanagers: no {name}"))?;
        Ok(ggm
            .object(info.path_id)
            .ok_or_else(|| format!("globalgamemanagers: {name} unreadable"))?
            .1
            .to_vec())
    };
    Ok(PhysicsSettings {
        physics: PhysicsManager::parse(&data(PHYSICS_MANAGER, "PhysicsManager")?, big_endian)
            .map_err(|e| format!("PhysicsManager: {e}"))?,
        time: TimeManager::parse(&data(TIME_MANAGER, "TimeManager")?, big_endian)
            .map_err(|e| format!("TimeManager: {e}"))?,
        tags: TagManager::parse(&data(TAG_MANAGER, "TagManager")?, big_endian)
            .map_err(|e| format!("TagManager: {e}"))?,
    })
}

impl Assets<'_> {
    /// The components of a prefab node's GameObject.
    pub fn node_components(&self, node: &PrefabNode) -> Result<Vec<ObjectRef>> {
        let (bundle, name, path_id) = &node.object;
        let file = self.file(bundle, name)?;
        let go = ObjectRef {
            file,
            path_id: *path_id,
        };
        let (info, data) = go.data()?;
        if info.class_id != GAME_OBJECT {
            return Err(format!("{name}: object {path_id} is not a GameObject"));
        }
        let parsed = sn_unity::GameObject::parse(data, go.file.file().big_endian)
            .map_err(|e| format!("GameObject {path_id}: {e}"))?;
        let mut out = Vec::with_capacity(parsed.components.len());
        for c in parsed.components {
            if let Some(o) = self.resolve(&go.file, c)? {
                out.push(o);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
    "entries" : [
        // A recipe.
        {
            "techType" : 10,
            "itemSize" : { "x" : 2, "y" : 2 },
            "craftTime" : 3.5,
            "ingredients" : [
                { "techType" : 20, "amount" : 2 },
                { "techType" : 30 }
            ],
            "linkedItems" : [ 40, 40 ],
            "equipmentType" : 1,
            "buildable" : true,
            "soundPickup" : "event:/x"
        },
        { "techType" : 20, "harvestOutput" : 30, "maxCharge" : 100 },
        { "itemSize" : { "x" : 1 } },
        { "techType" : 20, "isExpanded" : true }
    ]
}"#;

    #[test]
    fn reads_entries_and_reports_odd_ones() {
        let d = parse_tech_data(SAMPLE.as_bytes()).unwrap();
        assert_eq!(d.entries.len(), 3);
        assert_eq!(d.without_tech_type, 1);
        assert_eq!(d.duplicates, vec![20]);
        assert_eq!(d.unknown_keys.get("isExpanded"), Some(&1));
        let e = d.get(10).unwrap();
        assert_eq!(e.item_size, Some([2, 2]));
        assert_eq!(e.craft_time, Some(3.5));
        assert_eq!(e.craft_amount, None);
        assert_eq!(
            e.ingredients,
            vec![
                Ingredient {
                    tech_type: 20,
                    amount: 2
                },
                Ingredient {
                    tech_type: 30,
                    amount: 1
                }
            ]
        );
        assert_eq!(e.linked_items, vec![40, 40]);
        assert_eq!(e.equipment_type, Some(1));
        assert_eq!(e.buildable, Some(true));
        assert_eq!(e.sound_pickup.as_deref(), Some("event:/x"));
        let h = d.get(20).unwrap();
        assert_eq!(h.harvest_output, Some(30));
        assert_eq!(h.max_charge, Some(100.0));
        assert_eq!(d.recipes().count(), 1);
        assert_eq!(d.ingredients_without_entry(), vec![(10, 30)]);
    }

    #[test]
    fn bad_values_are_errors() {
        for bad in [
            r#"{"entries": [{"techType": 1.5}]}"#,
            r#"{"entries": [{"techType": "x"}]}"#,
            r#"{"entries": [{"techType": 1, "ingredients": [{"amount": 1}]}]}"#,
            r#"{"entries": [{"techType": 1, "buildable": 1}]}"#,
            r#"{"entries": {}}"#,
            r#"[]"#,
            r#"{"entries": [{"techType": 1}"#,
        ] {
            assert!(parse_tech_data(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
