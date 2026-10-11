//! Item data for M9d (`docs/DESIGN.md` § 4.3 "M9d plan", step M9d1):
//! what a prefab is as an item (by the game's `CraftData.GetTechType`
//! rule), what can be picked up, what outcrops break into, the player's
//! luck per tech type, the lifepod's locker and its starting supplies, the
//! rules that live only in the game's code, and the item text from the
//! language files. Facts and how they were checked:
//! `docs/formats/gameplay.md` § Pick-ups and outcrops, § Inventory.

use std::collections::HashMap;

use sn_dotnet::{Arg, Assembly};
use sn_install::GameData;
use sn_unity::json::{Json, parse_json};
use sn_unity::{
    BreakableResource, Catalog, EntTechEntry, FairRandomizer, LootSpawner, Pickupable,
    PrefabIdentifier, StorageContainer, parse_player_entropy, parse_prefab_list_spawner,
    parse_spawn_escape_pod_supplies, parse_tech_tag,
};

use crate::code::GAME_ASSEMBLY;
use crate::prefab::Prefab;
use crate::{Assets, ObjectRef, Result};

const MONO_BEHAVIOUR: i32 = 114;
const GAME_OBJECT: i32 = 1;

/// Item rules the game keeps only in its code.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemCode {
    /// `new ItemsContainer(width, height, …)` in `Inventory.Awake`.
    pub inventory_size: [i32; 2],
    /// `Equipment.slotMapping`: slot name → `EquipmentType` value, in
    /// source order.
    pub slot_types: Vec<(String, i32)>,
    /// `Inventory.UnlockDefaultEquipmentSlots`: the player's slots.
    pub player_slots: Vec<String>,
}

/// Reads [`ItemCode`] from the game's code assembly.
pub fn item_code(bytes: &[u8]) -> Result<ItemCode> {
    let e = |e: sn_dotnet::Error| format!("{GAME_ASSEMBLY}: {e}");
    let asm = Assembly::parse(bytes).map_err(e)?;
    let args = sn_dotnet::constructor_args(
        &asm,
        "",
        "Inventory",
        "Awake",
        "ItemsContainer",
        "_container",
    )
    .map_err(e)?;
    let inventory_size = match args[..] {
        [Arg::Int(w), Arg::Int(h), ..] => [w, h],
        _ => return Err(format!("Inventory.Awake: ItemsContainer size is {args:?}")),
    };
    Ok(ItemCode {
        inventory_size,
        slot_types: sn_dotnet::static_string_int_dictionary(&asm, "", "Equipment", "slotMapping")
            .map_err(e)?,
        player_slots: sn_dotnet::string_array(&asm, "", "Inventory", "UnlockDefaultEquipmentSlots")
            .map_err(e)?,
    })
}

/// Where an object's tech type came from (`CraftData.GetTechType`: the
/// nearest `TechTag` or `PrefabIdentifier` up from the object).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TechSource {
    TechTag,
    /// The `PrefabIdentifier`'s class id → its file in the prefab database
    /// → `EntTechData` by file name (case-insensitive).
    ClassId,
    /// Neither found: `TechType.None`.
    Nothing,
}

/// The game's prefab → tech type lookups.
pub struct TechLookup {
    /// `EntTechData`: prefab file name (lower case) → tech type.
    by_name: HashMap<String, i32>,
    /// The prefab database: class id → prefab path.
    class_paths: HashMap<String, String>,
}

impl TechLookup {
    pub fn new(ent_tech: &[EntTechEntry], class_paths: HashMap<String, String>) -> TechLookup {
        let mut by_name = HashMap::new();
        // Later entries win, as the game's dictionary assignment.
        for e in ent_tech {
            by_name.insert(e.prefab_name.to_lowercase(), e.tech_type);
        }
        TechLookup {
            by_name,
            class_paths,
        }
    }

    /// `entClassTechTable[classId]`: 0 (`None`) when unknown.
    pub fn by_class_id(&self, class_id: &str) -> i32 {
        let Some(path) = self.class_paths.get(class_id) else {
            return 0;
        };
        let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
        self.by_name.get(&stem.to_lowercase()).copied().unwrap_or(0)
    }
}

/// A prefab as an item: its tech type by the game's rule, and the item
/// scripts on it.
#[derive(Clone, Debug)]
pub struct ItemPrefab {
    pub tech_type: i32,
    pub source: TechSource,
    /// On the node of the `Pickupable` (else of the `BreakableResource`,
    /// else the root).
    pub pickupable: Option<Pickupable>,
    pub eatable: bool,
    pub breakable: Option<BreakableResource>,
    /// Number of `Pickupable`s in the hierarchy (the game finds the
    /// nearest one up from the hit collider; more than one is logged).
    pub pickupables: usize,
}

/// The item scripts of one node.
#[derive(Default)]
struct NodeScripts {
    tech_tag: Option<i32>,
    class_id: Option<String>,
    pickupable: Option<Pickupable>,
    eatable: bool,
    breakable: Option<BreakableResource>,
}

fn node_scripts(assets: &Assets, prefab: &Prefab, node: usize) -> Result<NodeScripts> {
    let mut s = NodeScripts::default();
    let Some(n) = prefab.nodes.get(node) else {
        return Ok(s);
    };
    for c in assets.node_components(n)? {
        let (info, data) = c.data()?;
        if info.class_id != MONO_BEHAVIOUR {
            continue;
        }
        let be = c.file.file().big_endian;
        let key = &prefab.key;
        match assets.script_class(&c).as_deref() {
            Some("TechTag") => {
                s.tech_tag =
                    Some(parse_tech_tag(data, be).map_err(|e| format!("{key} TechTag: {e}"))?);
            }
            Some("PrefabIdentifier") => {
                s.class_id = Some(
                    PrefabIdentifier::parse(data, be)
                        .map_err(|e| format!("{key} PrefabIdentifier: {e}"))?
                        .class_id,
                );
            }
            Some("Pickupable") => {
                s.pickupable = Some(
                    Pickupable::parse(data, be).map_err(|e| format!("{key} Pickupable: {e}"))?,
                );
            }
            Some("Eatable") => s.eatable = true,
            Some("BreakableResource") => {
                s.breakable = Some(
                    BreakableResource::parse(data, be)
                        .map_err(|e| format!("{key} BreakableResource: {e}"))?,
                );
            }
            _ => {}
        }
    }
    Ok(s)
}

/// Reads a prefab's item scripts and its tech type by the game's rule,
/// starting from the node of its `Pickupable` (else `BreakableResource`,
/// else the root).
pub fn item_prefab(assets: &Assets, prefab: &Prefab, lookup: &TechLookup) -> Result<ItemPrefab> {
    let mut scripts = Vec::with_capacity(prefab.nodes.len());
    for i in 0..prefab.nodes.len() {
        scripts.push(node_scripts(assets, prefab, i)?);
    }
    let pickupables = scripts.iter().filter(|s| s.pickupable.is_some()).count();
    let start = scripts
        .iter()
        .position(|s| s.pickupable.is_some())
        .or_else(|| scripts.iter().position(|s| s.breakable.is_some()))
        .unwrap_or(0);
    // Walk up until a TechTag or a PrefabIdentifier (bounded by the node
    // count, so a broken parent chain cannot loop).
    let (mut tech_type, mut source) = (0, TechSource::Nothing);
    let mut at = Some(start);
    for _ in 0..prefab.nodes.len() {
        let Some(i) = at else { break };
        let s = &scripts[i];
        if let Some(t) = s.tech_tag {
            (tech_type, source) = (t, TechSource::TechTag);
            break;
        }
        if let Some(id) = &s.class_id {
            (tech_type, source) = (lookup.by_class_id(id), TechSource::ClassId);
            break;
        }
        at = prefab.nodes[i].parent;
    }
    let own = &scripts[start];
    Ok(ItemPrefab {
        tech_type,
        source,
        pickupable: own.pickupable,
        eatable: own.eatable,
        breakable: scripts.iter().find_map(|s| s.breakable.clone()),
        pickupables,
    })
}

/// The prefab an Addressables key (a GUID of an `AssetReference`, or a
/// path) names, if it is a GameObject in the catalog.
pub fn addressable_prefab(assets: &Assets, catalog: &Catalog, key: &str) -> Result<Option<Prefab>> {
    let Some(location) = catalog
        .locate(key)
        .into_iter()
        .find(|l| l.resource_type == "UnityEngine.GameObject")
    else {
        return Ok(None);
    };
    let Some(root) = assets.catalog_object(&location, GAME_OBJECT)? else {
        return Ok(None);
    };
    let path = location
        .internal_id
        .strip_prefix("Assets/AddressableResources/")
        .unwrap_or(&location.internal_id)
        .to_string();
    Ok(Some(assets.hierarchy(&path, &root)?))
}

/// What the main scene, the essentials scene's spawned prefabs and the
/// lifepod hold about the player's items.
#[derive(Clone, Debug)]
pub struct PlayerItems {
    /// `PlayerEntropy.randomizers`: tech type and its `FairRandomizer`'s
    /// stored values.
    pub entropy: Vec<(i32, FairRandomizer)>,
    /// From the `LootSpawner` (on a prefab the essentials scene's
    /// `PrefabListSpawner` spawns), and that prefab's key.
    pub loot: LootSpawner,
    pub loot_prefab: String,
    /// The lifepod's locker (`StorageContainer` next to
    /// `SpawnEscapePodSupplies` in the escape pod scene).
    pub pod_locker: StorageContainer,
    /// The locker's node name.
    pub pod_locker_node: String,
}

fn only(found: Vec<ObjectRef>, what: &str) -> Result<ObjectRef> {
    match <[ObjectRef; 1]>::try_from(found) {
        Ok([o]) => Ok(o),
        Err(v) => Err(format!("{what}: {} found, expected 1", v.len())),
    }
}

/// Reads [`PlayerItems`].
pub fn player_items(assets: &Assets) -> Result<PlayerItems> {
    // The player's luck.
    let main = assets.scene("main")?;
    let entropy_ref = only(main.behaviours(assets, "PlayerEntropy"), "PlayerEntropy")?;
    let (_, data) = entropy_ref.data()?;
    let be = entropy_ref.file.file().big_endian;
    let mut entropy = Vec::new();
    for (tech, pptr) in parse_player_entropy(data, be).map_err(|e| format!("PlayerEntropy: {e}"))? {
        let o = assets
            .resolve(&entropy_ref.file, pptr)?
            .ok_or_else(|| format!("PlayerEntropy: tech type {tech} has no randomizer"))?;
        if assets.script_class(&o).as_deref() != Some("FairRandomizer") {
            return Err(format!(
                "PlayerEntropy: tech type {tech}: not a FairRandomizer"
            ));
        }
        let (_, d) = o.data()?;
        let f = FairRandomizer::parse(d, o.file.file().big_endian)
            .map_err(|e| format!("FairRandomizer: {e}"))?;
        entropy.push((tech, f));
    }

    // The loot spawner: on one of the prefabs Essentials spawns.
    let essentials = assets.scene("essentials")?;
    let spawner = only(
        essentials.behaviours(assets, "PrefabListSpawner"),
        "PrefabListSpawner",
    )?;
    let (_, data) = spawner.data()?;
    let mut loot = None;
    let prefabs = parse_prefab_list_spawner(data, spawner.file.file().big_endian)
        .map_err(|e| format!("PrefabListSpawner: {e}"))?;
    for pptr in prefabs {
        let Some(go) = assets.resolve(&spawner.file, pptr)? else {
            continue;
        };
        let prefab = assets.hierarchy("essentials spawn", &go)?;
        for node in &prefab.nodes {
            for c in assets.node_components(node)? {
                if assets.script_class(&c).as_deref() != Some("LootSpawner") {
                    continue;
                }
                if loot.is_some() {
                    return Err("more than one LootSpawner".into());
                }
                let (_, d) = c.data()?;
                let l = LootSpawner::parse(d, c.file.file().big_endian)
                    .map_err(|e| format!("LootSpawner: {e}"))?;
                loot = Some((l, prefab.nodes[0].name.clone()));
            }
        }
    }
    let (loot, loot_prefab) = loot.ok_or("no LootSpawner on the essentials' prefabs")?;

    // The lifepod's locker.
    let pod = assets.scene("escapepod")?;
    let supplies = only(
        pod.behaviours(assets, "SpawnEscapePodSupplies"),
        "SpawnEscapePodSupplies",
    )?;
    let (_, data) = supplies.data()?;
    let target = parse_spawn_escape_pod_supplies(data, supplies.file.file().big_endian)
        .map_err(|e| format!("SpawnEscapePodSupplies: {e}"))?;
    let locker = assets
        .resolve(&supplies.file, target)?
        .ok_or("SpawnEscapePodSupplies.storageContainer is null")?;
    if assets.script_class(&locker).as_deref() != Some("StorageContainer") {
        return Err("SpawnEscapePodSupplies.storageContainer is not a StorageContainer".into());
    }
    let (_, data) = locker.data()?;
    let pod_locker = StorageContainer::parse(data, locker.file.file().big_endian)
        .map_err(|e| format!("lifepod StorageContainer: {e}"))?;
    let (rr, nn) = pod
        .behaviour_node(assets, &locker)?
        .ok_or("lifepod StorageContainer: not in the scene")?;
    Ok(PlayerItems {
        entropy,
        loot,
        loot_prefab,
        pod_locker,
        pod_locker_node: pod.roots[rr].nodes[nn].name.clone(),
    })
}

/// The text the item UI shows (M9d5): every tech type's name and a
/// language file. Not rules: two players may read different languages.
#[derive(Clone, Debug, Default)]
pub struct ItemText {
    /// `TechType` value → its name (the enum's, from the game's code).
    pub tech_names: HashMap<i32, String>,
    pub strings: HashMap<String, String>,
}

impl ItemText {
    /// `Language.Get`: the text of `key`, or the key itself when the file
    /// has none (as the game shows it).
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.strings.get(key).map_or(key, String::as_str)
    }

    /// `Language.Get(TechType)`: the text keyed by the tech type's name.
    pub fn tech(&self, tech: i32) -> String {
        match self.tech_names.get(&tech) {
            Some(name) => self.get(name).to_string(),
            None => format!("#{tech}"),
        }
    }

    /// `Language.GetFormat(key, arg)`: `{0}` in the key's text replaced by
    /// `arg` (C#'s `string.Format`: `{{` and `}}` are literal braces;
    /// other placeholders are left as they are).
    pub fn format(&self, key: &str, arg: &str) -> String {
        let text = self.get(key);
        let mut out = String::with_capacity(text.len() + arg.len());
        let mut rest = text;
        while let Some(i) = rest.find(['{', '}']) {
            out.push_str(&rest[..i]);
            let tail = &rest[i..];
            if tail.starts_with("{{") || tail.starts_with("}}") {
                out.push_str(&tail[..1]);
                rest = &tail[2..];
            } else if let Some(after) = tail.strip_prefix("{0}") {
                out.push_str(arg);
                rest = after;
            } else {
                out.push_str(&tail[..1]);
                rest = &tail[1..];
            }
        }
        out.push_str(rest);
        out
    }
}

/// Reads [`ItemText`]: tech type names from the game's code (`dll`, as
/// `read_assembly` gives it) and the language file `language`.
pub fn item_text(game: &GameData, dll: &[u8], language: &str) -> Result<ItemText> {
    let e = |e: sn_dotnet::Error| format!("{GAME_ASSEMBLY}: {e}");
    let asm = Assembly::parse(dll).map_err(e)?;
    Ok(ItemText {
        tech_names: sn_dotnet::tech_type_names(&asm)
            .map_err(e)?
            .into_iter()
            .collect(),
        strings: self::language(game, language)?,
    })
}

/// A language file: key → text (`LanguageFiles/<name>.json`, read as
/// `Language.LoadLanguageFile` does; console platform suffixes are not
/// remapped on desktop).
pub fn language(game: &GameData, name: &str) -> Result<HashMap<String, String>> {
    let path = game
        .data_dir
        .join("StreamingAssets/SNUnmanagedData/LanguageFiles")
        .join(format!("{name}.json"));
    let bytes = game.read_file(&path)?;
    parse_language(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// Parses a language file: one JSON object of strings (comments allowed).
pub fn parse_language(bytes: &[u8]) -> Result<HashMap<String, String>> {
    let Json::Object(entries) = parse_json(bytes).map_err(|e| e.to_string())? else {
        return Err("not a JSON object".into());
    };
    let mut out = HashMap::with_capacity(entries.len());
    for (k, v) in entries {
        match v {
            Json::String(s) => {
                out.insert(k, s);
            }
            other => return Err(format!("{k}: {other:?} is not a string")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_ids_map_by_file_name_ignoring_case() {
        let ent = [
            EntTechEntry {
                prefab_name: "titanium".into(),
                tech_type: 5,
            },
            EntTechEntry {
                prefab_name: "Quartz".into(),
                tech_type: 7,
            },
        ];
        let paths = HashMap::from([
            (
                "a".to_string(),
                "WorldEntities/Natural/Titanium.prefab".to_string(),
            ),
            (
                "b".to_string(),
                "WorldEntities/Natural/quartz.prefab".to_string(),
            ),
            ("c".to_string(), "WorldEntities/Other.prefab".to_string()),
        ]);
        let l = TechLookup::new(&ent, paths);
        assert_eq!(l.by_class_id("a"), 5);
        assert_eq!(l.by_class_id("b"), 7);
        assert_eq!(l.by_class_id("c"), 0);
        assert_eq!(l.by_class_id("missing"), 0);
    }

    #[test]
    fn language_files() {
        let text = b"{\n\t// a comment\n\t\"Key\": \"Text {0}\",\n\t\"Other\": \"\"\n}";
        let l = parse_language(text).unwrap();
        assert_eq!(l.get("Key").map(String::as_str), Some("Text {0}"));
        assert_eq!(l.len(), 2);
        assert!(parse_language(b"[1]").is_err());
        assert!(parse_language(b"{\"a\": 1}").is_err());
        for n in 0..text.len() {
            let _ = parse_language(&text[..n]);
        }
    }

    #[test]
    fn item_text_formats_as_the_game() {
        let t = ItemText {
            tech_names: HashMap::from([(5, "Titanium".to_string()), (6, "Bare".to_string())]),
            strings: HashMap::from([
                ("Titanium".to_string(), "Ti".to_string()),
                ("PickUpFormat".to_string(), "Take {0} {{now}}".to_string()),
            ]),
        };
        assert_eq!(t.tech(5), "Ti");
        // No text: the key, as the game shows it.
        assert_eq!(t.tech(6), "Bare");
        assert_eq!(t.tech(7), "#7");
        assert_eq!(t.format("PickUpFormat", &t.tech(5)), "Take Ti {now}");
        assert_eq!(t.format("Missing", "x"), "Missing");
        assert_eq!(t.format("{1} {0}", "x"), "{1} x");
    }
}
