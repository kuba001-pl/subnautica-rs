//! `ui` (M9u0, `docs/DESIGN.md` "M9u plan"): a census of the game's UI,
//! headless. Where `uGUI.prefab` and the PDA's screen come from and what
//! they hold, every canvas and canvas scaler, the hosts of the scripts
//! M9u ports, the sprite atlases and the fonts; each count checked
//! against the one the plan was written with.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, FileRef, ObjectRef, UiTree};
use sn_install::GameData;
use sn_unity::{
    Canvas, CanvasScaler, GameObject, MonoBehaviourHeader, RENDER_MODE_SCREEN_SPACE_CAMERA,
    RENDER_MODE_SCREEN_SPACE_OVERLAY, RENDER_MODE_WORLD_SPACE, SpriteAtlasHead, Texture2D,
    TransformNode, class_name, object_name,
};

use crate::Result;

const MONO_BEHAVIOUR: i32 = 114;
const TEXTURE_2D: i32 = 28;
const FONT: i32 = 128;
const SPRITE: i32 = 213;
const CANVAS: i32 = 223;
const RECT_TRANSFORM: i32 = 224;
const SPRITE_ATLAS: i32 = 687_078_895;

/// The scripts M9u ports (the plan's in-scope screens), listed with the
/// objects that hold them.
const IN_SCOPE: &[&str] = &[
    "uGUI",
    "uGUI_SceneHUD",
    "uGUI_CanvasScaler",
    "uGUI_OxygenBar",
    "uGUI_HealthBar",
    "uGUI_FoodBar",
    "uGUI_WaterBar",
    "uGUI_CircularBar",
    "uGUI_DepthCompass",
    "HandReticle",
    "uGUI_HandReticleIcon",
    "ErrorMessage",
    "uGUI_IconNotifier",
    "uGUI_PopupNotification",
    "uGUI_PopupNotificationDefault",
    "uGUI_NotificationLabel",
    "uGUI_QuickSlots",
    "uGUI_PlayerDeath",
    "uGUI_SceneRespawning",
    "uGUI_Fader",
    "uGUI_Overlays",
    "uGUI_Tooltip",
    "uGUI_PDA",
    "uGUI_InventoryTab",
    "uGUI_ItemsContainer",
    "uGUI_Equipment",
    "uGUI_ItemIcon",
];

/// Checks against the numbers the plan was written with.
struct Checks {
    failed: usize,
}

impl Checks {
    fn eq(&mut self, what: &str, got: usize, want: usize) {
        let ok = got == want;
        if !ok {
            self.failed += 1;
        }
        println!(
            "  check {what}: {got} (plan: {want}) {}",
            if ok { "ok" } else { "DIFFERS" }
        );
    }
}

fn mode_name(mode: i32) -> &'static str {
    match mode {
        RENDER_MODE_SCREEN_SPACE_OVERLAY => "screen-space overlay",
        RENDER_MODE_SCREEN_SPACE_CAMERA => "screen-space camera",
        RENDER_MODE_WORLD_SPACE => "world space",
        _ => "?",
    }
}

/// `uGUI_CanvasScaler.Mode` and `.ScaleMode` (`code --enum`).
fn scaler_mode(mode: i32) -> &'static str {
    match mode {
        0 => "Static",
        1 => "Inversed",
        2 => "Parented",
        3 => "World",
        _ => "?",
    }
}

fn scale_mode(mode: i32) -> &'static str {
    match mode {
        0 => "DontScale",
        1 => "Immediate",
        2 => "Postponed",
        _ => "?",
    }
}

fn data(object: &ObjectRef) -> Result<(&[u8], bool)> {
    let (_, d) = object.data()?;
    Ok((d, object.file.file().big_endian))
}

fn print_tree(tree: &UiTree) {
    let active = tree.nodes.iter().filter(|n| n.active).count();
    let mut layers: BTreeMap<u32, usize> = BTreeMap::new();
    for n in &tree.nodes {
        *layers.entry(n.layer).or_default() += 1;
    }
    println!(
        "  nodes {} ({active} active in the hierarchy), by layer {layers:?}",
        tree.nodes.len()
    );
    println!("  components by class:");
    for (class, n) in tree.class_counts() {
        println!("    {n:5} {class:>4} {}", class_name(class).unwrap_or("?"));
    }
    let mut scripts: Vec<(String, usize)> = tree.script_counts().into_iter().collect();
    scripts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    println!("  MonoBehaviours by script ({} scripts):", scripts.len());
    for (s, n) in &scripts {
        println!("    {n:5} {s}");
    }
}

/// Canvases and canvas scalers of a tree; returns (canvases, world-space
/// canvases, overlay canvases, scalers).
fn print_canvases(tree: &UiTree) -> Result<(usize, usize, usize, usize)> {
    let (mut canvases, mut world, mut overlay, mut scalers) = (0, 0, 0, 0);
    for (i, node) in tree.nodes.iter().enumerate() {
        for c in &node.components {
            if c.class_id == CANVAS {
                let (d, be) = data(&c.object)?;
                let cv = Canvas::parse(d, be).map_err(|e| format!("{}: Canvas: {e}", node.path))?;
                canvases += 1;
                match cv.render_mode {
                    RENDER_MODE_WORLD_SPACE => world += 1,
                    RENDER_MODE_SCREEN_SPACE_OVERLAY => overlay += 1,
                    _ => {}
                }
                println!(
                    "  canvas {:?}{}: {}, enabled {}, plane distance {}, sorting layer {} order {}{}, pixel perfect {}, shader channels {:#x}",
                    node.path,
                    if node.active { "" } else { " [inactive]" },
                    mode_name(cv.render_mode),
                    cv.enabled,
                    cv.plane_distance,
                    cv.sorting_layer_id,
                    cv.sorting_order,
                    if cv.override_sorting {
                        " (override)"
                    } else {
                        ""
                    },
                    cv.pixel_perfect,
                    cv.additional_shader_channels,
                );
            }
            if c.script.as_deref() == Some("uGUI_CanvasScaler") {
                let (d, be) = data(&c.object)?;
                let s = CanvasScaler::parse(d, be)
                    .map_err(|e| format!("{}: uGUI_CanvasScaler: {e}", node.path))?;
                scalers += 1;
                println!(
                    "  scaler {:?} (node {i}): enabled {}, mode {}, vr mode {}, distance {} m, reference {}×{}, scale {}",
                    node.path,
                    s.enabled,
                    scaler_mode(s.mode),
                    scaler_mode(s.vr_mode),
                    s.distance,
                    s.reference_resolution[0],
                    s.reference_resolution[1],
                    scale_mode(s.scale_mode),
                );
            }
        }
    }
    Ok((canvases, world, overlay, scalers))
}

fn print_hosts(tree: &UiTree) {
    for class in IN_SCOPE {
        for (i, _) in tree.scripts(class) {
            let n = &tree.nodes[i];
            println!(
                "  {class:<30} {:?}{}",
                n.path,
                if n.active { "" } else { " [inactive]" }
            );
        }
    }
}

/// The atlases bundle: every `SpriteAtlas`, and where its packed sprites
/// resolve to. Returns (atlases, the `Items` atlas's packed sprites, of
/// those resolved to a Sprite).
fn atlases(assets: &Assets) -> Result<(usize, usize, usize)> {
    let bundle = assets
        .bundle_named("atlases_")
        .ok_or("no atlases_ bundle")?
        .to_path_buf();
    println!(
        "atlases: {}",
        bundle.file_name().unwrap_or_default().to_string_lossy()
    );
    let loaded = assets.bundle(&bundle)?;
    let names: Vec<String> = loaded
        .file_names()
        .filter(|n| !n.ends_with(".resS"))
        .map(String::from)
        .collect();
    let (mut count, mut items, mut items_resolved) = (0, 0, 0);
    for name in &names {
        let file = assets.file(&bundle, name)?;
        for e in &file.file().externals {
            println!("  external {}", e.path);
        }
        let mut pages = Vec::new();
        for info in file.objects() {
            let Some(d) = file.file().object_data(file.bytes(), info) else {
                continue;
            };
            let be = file.file().big_endian;
            match info.class_id {
                SPRITE_ATLAS => {
                    let a =
                        SpriteAtlasHead::parse(d, be).map_err(|e| format!("SpriteAtlas: {e}"))?;
                    let (resolved, by_file) = resolve_sprites(assets, &file, &a)?;
                    count += 1;
                    if a.name == "Items" {
                        items = a.packed_sprites.len();
                        items_resolved = resolved;
                    }
                    println!(
                        "  atlas {:?}: {} packed sprites, {resolved} resolve to a Sprite, in files {by_file:?}",
                        a.name,
                        a.packed_sprites.len()
                    );
                }
                TEXTURE_2D => {
                    let t = Texture2D::parse(d, be).map_err(|e| format!("Texture2D: {e}"))?;
                    pages.push(format!(
                        "{:?} {}×{} format {}",
                        t.name, t.width, t.height, t.format
                    ));
                }
                _ => {}
            }
        }
        for p in pages {
            println!("  page {p}");
        }
    }
    Ok((count, items, items_resolved))
}

fn resolve_sprites(
    assets: &Assets,
    file: &FileRef,
    atlas: &SpriteAtlasHead,
) -> Result<(usize, BTreeMap<String, usize>)> {
    let mut resolved = 0;
    let mut by_file = BTreeMap::new();
    for p in &atlas.packed_sprites {
        if let Some(o) = assets.resolve(file, *p)?
            && o.data()?.0.class_id == SPRITE
        {
            resolved += 1;
            *by_file.entry(o.file.name.clone()).or_default() += 1;
        }
    }
    Ok((resolved, by_file))
}

/// The fonts bundle. Returns (fonts, TMP font assets, font atlases stored
/// with pixels).
fn fonts(assets: &Assets) -> Result<(usize, usize, usize)> {
    let bundle = assets
        .bundle_named("fonts_assets_all")
        .ok_or("no fonts_assets_all bundle")?
        .to_path_buf();
    println!(
        "fonts: {}",
        bundle.file_name().unwrap_or_default().to_string_lossy()
    );
    let loaded = assets.bundle(&bundle)?;
    let names: Vec<String> = loaded
        .file_names()
        .filter(|n| !n.ends_with(".resS"))
        .map(String::from)
        .collect();
    let (mut fonts, mut font_assets, mut stored) = (0, 0, 0);
    for name in &names {
        let file = assets.file(&bundle, name)?;
        let be = file.file().big_endian;
        for info in file.objects() {
            let Some(d) = file.file().object_data(file.bytes(), info) else {
                continue;
            };
            match info.class_id {
                FONT => {
                    fonts += 1;
                    let n = object_name(d, be).map_err(|e| format!("Font: {e}"))?;
                    println!("  font {n:?}: {} bytes", d.len());
                }
                MONO_BEHAVIOUR => {
                    let h = MonoBehaviourHeader::parse(d, be).map_err(|e| e.to_string())?;
                    let o = ObjectRef {
                        file: file.clone(),
                        path_id: info.path_id,
                    };
                    let script = assets.script_class(&o).unwrap_or_else(|| "?".into());
                    if script == "TMP_FontAsset" {
                        font_assets += 1;
                    }
                    println!("  {script} {:?}: {} bytes", h.name, d.len());
                }
                TEXTURE_2D => {
                    let t = Texture2D::parse(d, be).map_err(|e| format!("Texture2D: {e}"))?;
                    if t.name.ends_with(" Atlas") && t.width > 0 {
                        stored += 1;
                    }
                    if t.width > 0 {
                        println!(
                            "  texture {:?}: {}×{} format {}",
                            t.name, t.width, t.height, t.format
                        );
                    }
                }
                _ => {}
            }
        }
    }
    Ok((fonts, font_assets, stored))
}

/// GameObjects in `root`'s file whose transform has no parent, other
/// than `root`.
fn other_roots(assets: &Assets, root: &ObjectRef) -> Result<Vec<ObjectRef>> {
    let file = &root.file;
    let be = file.file().big_endian;
    let mut out = Vec::new();
    for info in file.objects().iter().filter(|o| o.class_id == 1) {
        if info.path_id == root.path_id {
            continue;
        }
        let Some(d) = file.file().object_data(file.bytes(), info) else {
            continue;
        };
        let go = GameObject::parse(d, be).map_err(|e| e.to_string())?;
        for c in &go.components {
            let Some(t) = assets.resolve(file, *c)? else {
                continue;
            };
            let (ti, td) = t.data()?;
            if ti.class_id == 4 || ti.class_id == RECT_TRANSFORM {
                let tn = TransformNode::parse(td, be).map_err(|e| e.to_string())?;
                if tn.father.is_null() {
                    out.push(ObjectRef {
                        file: file.clone(),
                        path_id: info.path_id,
                    });
                }
                break;
            }
        }
    }
    Ok(out)
}

fn node_name(assets: &Assets, transform: &ObjectRef) -> Result<String> {
    let (d, be) = data(transform)?;
    let t = TransformNode::parse(d, be).map_err(|e| e.to_string())?;
    let Some(go) = assets.resolve(&transform.file, t.game_object)? else {
        return Ok("?".into());
    };
    let (d, be) = data(&go)?;
    Ok(GameObject::parse(d, be).map_err(|e| e.to_string())?.name)
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let catalog = assets.catalog()?;
    let mut checks = Checks { failed: 0 };

    let root = assets.ugui_root(&catalog)?;
    println!(
        "{} (uGUI.InitializeAsync's key) → {} in {}",
        sn_assets::UGUI_PREFAB_KEY,
        root.path_id,
        root.file.name
    );
    let ugui = assets.ui_tree("uGUI.prefab", &root)?;
    print_tree(&ugui);
    let (canvases, world, overlay, scalers) = print_canvases(&ugui)?;
    let counts = ugui.class_counts();
    let scripts = ugui.script_counts();
    let script = |s: &str| scripts.get(s).copied().unwrap_or(0);
    checks.eq("uGUI.prefab nodes", ugui.nodes.len(), 361);
    checks.eq(
        "uGUI.prefab RectTransforms",
        counts.get(&RECT_TRANSFORM).copied().unwrap_or(0),
        360,
    );
    checks.eq("uGUI.prefab canvases", canvases, 12);
    checks.eq("world-space canvases", world, 11);
    checks.eq("screen-space overlay canvases", overlay, 1);
    checks.eq("uGUI_CanvasScaler", scalers, 4);
    checks.eq("Image", script("Image"), 143);
    checks.eq("TextMeshProUGUI", script("TextMeshProUGUI"), 101);
    checks.eq("uGUI_HandReticleIcon", script("uGUI_HandReticleIcon"), 10);
    println!("in-scope scripts in uGUI.prefab:");
    print_hosts(&ugui);

    // The bundle's other prefabs (root objects besides uGUI): the ones its
    // scripts instantiate.
    let others = other_roots(&assets, &root)?;
    println!(
        "other root objects in uGUI.prefab's file ({}):",
        others.len()
    );
    let (mut other_nodes, mut other_rects, mut other_canvases) = (0, 0, 0);
    let mut other_scripts: BTreeMap<String, usize> = BTreeMap::new();
    for o in &others {
        let t = assets.ui_tree("other", o)?;
        let c = t.class_counts();
        other_nodes += t.nodes.len();
        other_rects += c.get(&RECT_TRANSFORM).copied().unwrap_or(0);
        other_canvases += c.get(&CANVAS).copied().unwrap_or(0);
        let mut s: Vec<String> = t
            .script_counts()
            .into_iter()
            .map(|(k, v)| {
                *other_scripts.entry(k.clone()).or_default() += v;
                if v > 1 { format!("{k}×{v}") } else { k }
            })
            .collect();
        s.sort();
        println!(
            "  {:?}: {} nodes; {}",
            t.nodes[0].name,
            t.nodes.len(),
            s.join(", ")
        );
    }
    let both = |s: &str| script(s) + other_scripts.get(s).copied().unwrap_or(0);
    checks.eq(
        "nodes, uGUI and the other roots",
        ugui.nodes.len() + other_nodes,
        387,
    );
    checks.eq(
        "RectTransforms, uGUI and the other roots",
        counts.get(&RECT_TRANSFORM).copied().unwrap_or(0) + other_rects,
        385,
    );
    checks.eq(
        "canvases, uGUI and the other roots",
        canvases + other_canvases,
        13,
    );
    checks.eq("Image, uGUI and the other roots", both("Image"), 148);
    checks.eq(
        "TextMeshProUGUI, uGUI and the other roots",
        both("TextMeshProUGUI"),
        110,
    );

    let main = assets.scene("main")?;
    let (screen, anchor) = assets.pda_screen_refs(&main)?;
    let anchor_name = match &anchor {
        Some(a) => node_name(&assets, a)?,
        None => "null".into(),
    };
    println!(
        "PDA.prefabScreen → {} in {}; PDA.screenAnchor → {anchor_name:?}",
        screen.path_id, screen.file.name
    );
    let pda = assets.ui_tree("PDA screen", &screen)?;
    println!(
        "PDA screen {:?}:",
        pda.nodes.first().map(|n| n.name.as_str()).unwrap_or("")
    );
    print_tree(&pda);
    let (pda_canvases, ..) = print_canvases(&pda)?;
    println!("  ({pda_canvases} canvases)");
    let pda_scripts = pda.script_counts();
    let pda_script = |s: &str| pda_scripts.get(s).copied().unwrap_or(0);
    checks.eq("uGUI_PDA", pda_script("uGUI_PDA"), 1);
    checks.eq("uGUI_InventoryTab", pda_script("uGUI_InventoryTab"), 1);
    println!("in-scope scripts in the PDA screen:");
    print_hosts(&pda);

    let (atlas_count, items, items_resolved) = atlases(&assets)?;
    checks.eq("sprite atlases", atlas_count, 8);
    checks.eq("Items atlas packed sprites", items, 470);
    checks.eq("Items sprites that resolve", items_resolved, 470);

    let (font_count, font_assets, stored) = fonts(&assets)?;
    checks.eq("fonts", font_count, 9);
    checks.eq("TMP font assets", font_assets, 8);
    checks.eq("font atlases stored with pixels", stored, 1);

    println!(
        "{} checks differ ({:.2} s)",
        checks.failed,
        start.elapsed().as_secs_f64()
    );
    Ok(if checks.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
