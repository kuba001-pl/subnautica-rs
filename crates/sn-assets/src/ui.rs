//! The game's UI as loaded objects (M9u, `docs/formats/ui.md`): the
//! hierarchies of `uGUI.prefab` and of the PDA's screen, every node with
//! its components. M9u0 counts and lists them (`sn-inspect ui`); later
//! steps read the components' contents.
//!
//! Where they come from (confirmed in M9u0 from the code and the data):
//! `uGUI.InitializeAsync` loads the Addressables key `uGUI.prefab` and
//! instantiates it; the main scene's `PDA` script holds the screen's
//! prefab in its first field, `prefabScreen`.

use std::collections::BTreeMap;

use sn_unity::{Catalog, GameObject, MonoBehaviourHeader, PPtr, TransformNode};

use crate::{Assets, ObjectRef, Result};

const GAME_OBJECT: i32 = 1;
const TRANSFORM: i32 = 4;
const MONO_BEHAVIOUR: i32 = 114;
const RECT_TRANSFORM: i32 = 224;
/// Deeper than any UI hierarchy of the game; stops a cycle in bad data.
const MAX_DEPTH: usize = 64;

/// The Addressables key `uGUI.InitializeAsync` loads.
pub const UGUI_PREFAB_KEY: &str = "uGUI.prefab";

/// A component of a UI node.
#[derive(Clone)]
pub struct UiComponent {
    pub class_id: i32,
    /// The script's class for a MonoBehaviour (`None` if it has no
    /// script or the script is not found).
    pub script: Option<String>,
    pub object: ObjectRef,
}

#[derive(Clone)]
pub struct UiNode {
    pub name: String,
    pub parent: Option<usize>,
    /// Names from the root, joined by `/`.
    pub path: String,
    pub active_self: bool,
    /// Active in the hierarchy (itself and every parent).
    pub active: bool,
    pub layer: u32,
    pub components: Vec<UiComponent>,
    /// The node's GameObject (M9u5c: scripts reference nodes by it);
    /// `None` for one made by code (`new GameObject`, M9u7c1).
    pub game_object: Option<ObjectRef>,
}

/// A UI hierarchy, nodes in depth-first order (a parent before its
/// children, children in their transform's order: uGUI's draw order).
#[derive(Clone)]
pub struct UiTree {
    pub key: String,
    pub nodes: Vec<UiNode>,
}

impl UiTree {
    /// Every component with its node index.
    pub fn components(&self) -> impl Iterator<Item = (usize, &UiComponent)> {
        self.nodes
            .iter()
            .enumerate()
            .flat_map(|(i, n)| n.components.iter().map(move |c| (i, c)))
    }

    /// The components that are MonoBehaviours of script `class`.
    pub fn scripts<'s>(&'s self, class: &'s str) -> impl Iterator<Item = (usize, &'s UiComponent)> {
        self.components()
            .filter(move |(_, c)| c.script.as_deref() == Some(class))
    }

    /// Components by class id (built-in classes).
    pub fn class_counts(&self) -> BTreeMap<i32, usize> {
        let mut out = BTreeMap::new();
        for (_, c) in self.components() {
            *out.entry(c.class_id).or_default() += 1;
        }
        out
    }

    /// MonoBehaviours by script class (`?` for a missing script).
    pub fn script_counts(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for (_, c) in self
            .components()
            .filter(|(_, c)| c.class_id == MONO_BEHAVIOUR)
        {
            *out.entry(c.script.clone().unwrap_or_else(|| "?".into()))
                .or_default() += 1;
        }
        out
    }
}

impl Assets<'_> {
    /// The hierarchy below the GameObject `root`, with every component.
    pub fn ui_tree(&self, key: &str, root: &ObjectRef) -> Result<UiTree> {
        let mut tree = UiTree {
            key: key.into(),
            nodes: Vec::new(),
        };
        self.add_ui_node(root, None, true, &mut tree, 0)?;
        Ok(tree)
    }

    fn add_ui_node(
        &self,
        game_object: &ObjectRef,
        parent: Option<usize>,
        parent_active: bool,
        tree: &mut UiTree,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(format!("{}: hierarchy too deep", tree.key));
        }
        let (info, data) = game_object.data()?;
        if info.class_id != GAME_OBJECT {
            return Err(format!(
                "{} object {}: class {} is not a GameObject",
                game_object.file.name, game_object.path_id, info.class_id
            ));
        }
        let big_endian = game_object.file.file().big_endian;
        let go = GameObject::parse(data, big_endian)
            .map_err(|e| format!("GameObject {}: {e}", game_object.path_id))?;
        let mut components = Vec::with_capacity(go.components.len());
        let mut transform = None;
        for pptr in &go.components {
            let Some(c) = self.resolve(&game_object.file, *pptr)? else {
                continue;
            };
            let (info, data) = c.data()?;
            let class_id = info.class_id;
            if class_id == TRANSFORM || class_id == RECT_TRANSFORM {
                transform = Some(
                    TransformNode::parse(data, c.file.file().big_endian)
                        .map_err(|e| format!("Transform {}: {e}", c.path_id))?,
                );
            }
            let script = (class_id == MONO_BEHAVIOUR)
                .then(|| self.script_class(&c))
                .flatten();
            components.push(UiComponent {
                class_id,
                script,
                object: c,
            });
        }
        let transform =
            transform.ok_or_else(|| format!("{}: GameObject without a Transform", go.name))?;
        let path = match parent {
            Some(p) => format!("{}/{}", tree.nodes[p].path, go.name),
            None => go.name.clone(),
        };
        let active = parent_active && go.active;
        let index = tree.nodes.len();
        tree.nodes.push(UiNode {
            name: go.name,
            parent,
            path,
            active_self: go.active,
            active,
            layer: go.layer,
            components,
            game_object: Some(game_object.clone()),
        });
        for child in &transform.children {
            let Some(t) = self.resolve(&game_object.file, *child)? else {
                continue;
            };
            let (info, data) = t.data()?;
            if info.class_id != TRANSFORM && info.class_id != RECT_TRANSFORM {
                return Err(format!(
                    "{} object {}: class {} is not a Transform",
                    t.file.name, t.path_id, info.class_id
                ));
            }
            let node = TransformNode::parse(data, t.file.file().big_endian)
                .map_err(|e| format!("Transform {}: {e}", t.path_id))?;
            let Some(child_go) = self.resolve(&t.file, node.game_object)? else {
                continue;
            };
            self.add_ui_node(&child_go, Some(index), active, tree, depth + 1)?;
        }
        Ok(())
    }

    /// `uGUI.prefab`'s root GameObject, as `uGUI.InitializeAsync` loads it.
    pub fn ugui_root(&self, catalog: &Catalog) -> Result<ObjectRef> {
        let location = catalog
            .locate(UGUI_PREFAB_KEY)
            .into_iter()
            .find(|l| l.resource_type == "UnityEngine.GameObject")
            .ok_or_else(|| format!("{UGUI_PREFAB_KEY}: not a GameObject in the catalog"))?;
        self.catalog_object(&location, GAME_OBJECT)?.ok_or_else(|| {
            format!(
                "{UGUI_PREFAB_KEY}: {} not in its bundle",
                location.internal_id
            )
        })
    }

    /// The main scene's `PDA.prefabScreen` and `PDA.screenAnchor`.
    pub fn pda_screen_refs(&self, scene: &crate::Scene) -> Result<(ObjectRef, Option<ObjectRef>)> {
        let pdas = scene.behaviours(self, "PDA");
        let [pda] = pdas.as_slice() else {
            return Err(format!("{} PDA scripts in the main scene", pdas.len()));
        };
        let (_, data) = pda.data()?;
        let big_endian = pda.file.file().big_endian;
        let (screen, anchor) =
            pda_fields(data, big_endian).map_err(|e| format!("PDA {}: {e}", pda.path_id))?;
        let screen = self
            .resolve(&pda.file, screen)?
            .ok_or("PDA.prefabScreen is null")?;
        Ok((screen, self.resolve(&pda.file, anchor)?))
    }
}

/// `PDA`'s serialized fields (`prefabScreen`, `screenAnchor`); every other
/// field is static, a constant or private (`sn-inspect code --fields PDA`).
pub(crate) fn pda_fields(
    data: &[u8],
    big_endian: bool,
) -> std::result::Result<(PPtr, PPtr), String> {
    let header = MonoBehaviourHeader::parse(data, big_endian).map_err(|e| e.to_string())?;
    let at = header.fields_offset;
    let field = |i: usize| -> std::result::Result<PPtr, String> {
        let start = at + 12 * i;
        let bytes = data
            .get(start..start + 12)
            .ok_or_else(|| format!("field {i} past the end at {start}"))?;
        let (file, path) = bytes.split_at(4);
        let file = file.try_into().map_err(|_| "bad PPtr")?;
        let path = path.try_into().map_err(|_| "bad PPtr")?;
        Ok(if big_endian {
            PPtr {
                file_id: i32::from_be_bytes(file),
                path_id: i64::from_be_bytes(path),
            }
        } else {
            PPtr {
                file_id: i32::from_le_bytes(file),
                path_id: i64::from_le_bytes(path),
            }
        })
    };
    let fields = (field(0)?, field(1)?);
    if data.len() != at + 24 {
        return Err(format!(
            "{} bytes where the two fields end at {}",
            data.len(),
            at + 24
        ));
    }
    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pda_bytes(extra: usize) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0i32.to_le_bytes());
        b.extend_from_slice(&5i64.to_le_bytes());
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&40i64.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // empty name
        for (f, p) in [(1i32, 77i64), (0, 9)] {
            b.extend_from_slice(&f.to_le_bytes());
            b.extend_from_slice(&p.to_le_bytes());
        }
        b.extend(std::iter::repeat_n(0, extra));
        b
    }

    #[test]
    fn reads_the_pda_fields() {
        let (screen, anchor) = pda_fields(&pda_bytes(0), false).unwrap();
        assert_eq!((screen.file_id, screen.path_id), (1, 77));
        assert_eq!((anchor.file_id, anchor.path_id), (0, 9));
    }

    #[test]
    fn bad_pda_fields_are_errors() {
        let b = pda_bytes(0);
        for len in [0, 20, 40, b.len() - 1] {
            assert!(pda_fields(&b[..len], false).is_err(), "{len}");
        }
        assert!(pda_fields(&pda_bytes(4), false).is_err());
    }
}
