//! `ui --live` (M9u6a, `docs/formats/ui.md` § The live UI): `uGUI` read
//! once into a [`UiScene`] and drawn for many states, as the client does
//! each frame.
//!
//! - The stored state (unplaced, and placed at 1920×1080) and the start
//!   state: their hashes are printed, to compare with `ui --draw` and
//!   `ui --canvas` (equal before and after M9u6a's change, MODLOG).
//! - Each kind of change on a drawn node of the HUD: the draw list must
//!   change where expected, and drawing the start state again must give
//!   the start hash (nothing of a state stays behind).
//! - Frames: every HUD text, filled image, alpha and bar icon changed each
//!   frame; the draw time per frame against the target written before the
//!   first run (`docs/DESIGN.md` M9u6a).
//! - A fresh scene draws the start state with the same hash as the warmed
//!   one (the caches change nothing).

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, GlyphAtlases, LocalTransform, UiGraphicKind, UiScene, UiState, color32};
use sn_install::GameData;
use sn_ui::draw::DrawList;
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};
use sn_unity::Value;

use crate::Result;

/// The target written before the first run: the mean draw time of a frame
/// with every HUD text changed, uGUI at 1920×1080.
const TARGET_MS: f64 = 4.0;
const FRAMES: usize = 120;
const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const HUD: &str = "uGUI/ScreenCanvas/HUD/";

/// A change's expected effect on the draw list.
type Check = Box<dyn Fn(&DrawList) -> bool>;

/// The draw items of `node`.
fn items_of(d: &DrawList, node: usize) -> Vec<&sn_ui::draw::DrawItem> {
    d.items.iter().filter(|i| i.node == node).collect()
}

/// A quaternion turning `deg` degrees about y.
fn about_y(deg: f32) -> [f32; 4] {
    let h = deg.to_radians() / 2.0;
    [0.0, h.sin(), 0.0, h.cos()]
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let start = Instant::now();
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let start_state = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let t = Instant::now();
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    println!(
        "uGUI read into a scene in {:.3} s ({} nodes, {} placed canvases)",
        t.elapsed().as_secs_f64(),
        tree.nodes.len(),
        placements.len()
    );
    let mut failed = 0;
    // The stored state, each with fresh atlases as `ui --draw` and
    // `ui --canvas` make them (uGUI is their first tree).
    let none = HashMap::new();
    let mut fresh = GlyphAtlases::new(&fonts)?;
    let stored = scene.draw(
        &assets,
        &fonts,
        &set,
        &mut fresh,
        &none,
        &UiState::default(),
    )?;
    println!(
        "stored state, unplaced: {} draws, hash {:016x} (ui --draw's uGUI)",
        stored.list.items.len(),
        stored.list.hash()
    );
    let mut fresh = GlyphAtlases::new(&fonts)?;
    let placed = scene.draw(
        &assets,
        &fonts,
        &set,
        &mut fresh,
        &placements,
        &UiState::default(),
    )?;
    println!(
        "stored state, placed at {}×{}: {} draws, hash {:016x} (ui --canvas's)",
        SCREEN[0],
        SCREEN[1],
        placed.list.items.len(),
        placed.list.hash()
    );
    // The start state, with the atlases the frames go on with.
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let base = scene.draw(
        &assets,
        &fonts,
        &set,
        &mut atlases,
        &placements,
        &start_state,
    )?;
    let start_hash = base.list.hash();
    println!(
        "start state, placed: {} draws, hash {start_hash:016x}",
        base.list.items.len()
    );
    let again = |state: &UiState, scene: &mut UiScene, atlases: &mut GlyphAtlases| {
        scene.draw(&assets, &fonts, &set, atlases, &placements, state)
    };
    // Drawn HUD nodes, by kind.
    let graphics = assets.ui_graphics(&layouts, &tree)?;
    let hud = |n: usize| tree.nodes[n].path.starts_with(HUD);
    let drawn = |n: usize| !items_of(&base.list, n).is_empty();
    let texts: Vec<usize> = graphics
        .iter()
        .filter(|g| matches!(g.kind, UiGraphicKind::Text) && hud(g.node))
        .map(|g| g.node)
        .collect();
    let images: Vec<usize> = graphics
        .iter()
        .filter(|g| matches!(g.kind, UiGraphicKind::Image { .. }) && hud(g.node) && drawn(g.node))
        .map(|g| g.node)
        .collect();
    let filled: Vec<usize> = graphics
        .iter()
        .filter(|g| {
            matches!(&g.kind, UiGraphicKind::Image { params, .. } if params.kind == 3)
                && hud(g.node)
        })
        .map(|g| g.node)
        .collect();
    let drawn_filled = filled.iter().copied().find(|n| drawn(*n));
    println!(
        "HUD: {} texts ({} drawn), {} images drawn, {} filled images ({} drawn)",
        texts.len(),
        texts.iter().filter(|n| drawn(**n)).count(),
        images.len(),
        filled.len(),
        filled.iter().filter(|n| drawn(**n)).count()
    );
    // One change of each kind.
    let mut checks: Vec<(String, UiState, Check)> = Vec::new();
    let hud_root = tree
        .nodes
        .iter()
        .position(|n| n.path == HUD.trim_end_matches('/'))
        .ok_or("no HUD node")?;
    {
        let mut s = start_state.clone();
        s.active.insert(hud_root, false);
        let paths: Vec<bool> = tree.nodes.iter().map(|n| n.path.starts_with(HUD)).collect();
        checks.push((
            format!("SetActive(false) on {}", tree.nodes[hud_root].path),
            s,
            Box::new(move |d| !d.items.iter().any(|i| paths[i.node])),
        ));
    }
    if let Some(&text) = texts.iter().find(|n| drawn(**n)) {
        let mut s = start_state.clone();
        s.text.insert(text, "1234".into());
        let before: Vec<_> = items_of(&base.list, text)
            .iter()
            .map(|i| i.vertices.clone())
            .collect();
        checks.push((
            format!("text \"1234\" on {}", tree.nodes[text].path),
            s,
            Box::new(move |d| {
                let now: Vec<_> = items_of(d, text)
                    .iter()
                    .map(|i| i.vertices.clone())
                    .collect();
                // 4 glyphs: 16 vertices in the main mesh.
                now != before && now.first().is_some_and(|v| v.len() == 16)
            }),
        ));
    }
    if let Some(&img) = images.first() {
        let mut s = start_state.clone();
        s.enabled.insert(img, false);
        checks.push((
            format!("enabled false on {}", tree.nodes[img].path),
            s,
            Box::new(move |d| items_of(d, img).is_empty()),
        ));
        let mut s = start_state.clone();
        s.renderer_alpha.insert(img, 0.5);
        let a = items_of(&base.list, img)[0].alpha;
        checks.push((
            format!("CanvasRenderer.SetAlpha(0.5) on {}", tree.nodes[img].path),
            s,
            Box::new(move |d| items_of(d, img).first().is_some_and(|i| i.alpha == a * 0.5)),
        ));
        let mut s = start_state.clone();
        let c = [0.25, 0.5, 0.75, 1.0];
        s.color.insert(img, c);
        let want = color32(c);
        checks.push((
            format!("color {c:?} on {}", tree.nodes[img].path),
            s,
            Box::new(move |d| {
                items_of(d, img)
                    .first()
                    .is_some_and(|i| i.vertices.iter().all(|v| v.color[..3] == want[..3]))
            }),
        ));
        let mut s = start_state.clone();
        s.transform.insert(
            img,
            LocalTransform {
                scale: Some([2.0, 2.0, 1.0]),
                ..LocalTransform::default()
            },
        );
        let before = items_of(&base.list, img)[0].vertices.clone();
        let width = |v: &[sn_ui::mesh::Vertex]| {
            let xs = v.iter().map(|v| v.pos[0]);
            xs.clone().fold(f32::MIN, f32::max) - xs.fold(f32::MAX, f32::min)
        };
        let w0 = width(&before);
        checks.push((
            format!("localScale ×2 on {}", tree.nodes[img].path),
            s,
            Box::new(move |d| {
                items_of(d, img)
                    .first()
                    .is_some_and(|i| (width(&i.vertices) - 2.0 * w0).abs() < 1e-3 * w0.max(1.0))
            }),
        ));
    }
    if let Some(f) = drawn_filled {
        let mut s = start_state.clone();
        s.fill_amount.insert(f, 0.5);
        let before = items_of(&base.list, f)[0].vertices.clone();
        checks.push((
            format!("fillAmount 0.5 on {}", tree.nodes[f].path),
            s,
            Box::new(move |d| items_of(d, f).first().is_some_and(|i| i.vertices != before)),
        ));
    } else {
        println!(
            "  no drawn filled image in the HUD's start state: fillAmount checked by unit tests only"
        );
    }
    // A sprite a script assigns: one of the depth compass's half-moon
    // sprites other than the stored one.
    let mut sprite_check = None;
    for (node, c) in tree.scripts("uGUI_DepthCompass") {
        let (_, f) = assets.script_fields(&layouts, c)?;
        let pptr = |name: &str| f.get(name).and_then(Value::as_pptr);
        let Some(target) = pptr("halfMoon") else {
            continue;
        };
        let target = assets
            .resolve(&c.object.file, target)?
            .ok_or("halfMoon does not resolve")?;
        let key = target.key();
        let image = tree
            .nodes
            .iter()
            .position(|n| n.components.iter().any(|c| c.object.key() == key))
            .ok_or("halfMoon's node not found")?;
        // The Image's stored sprite.
        let (_, image_fields) = assets.script_fields(
            &layouts,
            &tree.nodes[image]
                .components
                .iter()
                .find(|c| c.object.key() == key)
                .ok_or("halfMoon's component")?
                .clone(),
        )?;
        let stored = match image_fields.get("m_Sprite").and_then(Value::as_pptr) {
            Some(p) => assets.resolve(&target.file, p)?.map(|o| o.key()),
            None => None,
        };
        for name in [
            "halfMoonNormal",
            "halfMoonDanger",
            "halfMoonCompassNormal",
            "halfMoonCompassDanger",
        ] {
            let Some(p) = pptr(name) else {
                continue;
            };
            let Some(sprite) = assets.resolve(&c.object.file, p)? else {
                continue;
            };
            let same = Some(sprite.key()) == stored;
            println!(
                "  {}: {name}{}",
                tree.nodes[node].path,
                if same {
                    " is the stored sprite of halfMoon"
                } else {
                    ""
                }
            );
            if !same && sprite_check.is_none() {
                sprite_check = Some((image, name, sprite));
            }
        }
    }
    if let Some((image, name, sprite)) = sprite_check {
        let mut s = start_state.clone();
        s.sprite.insert(image, Some(scene.sprite_id(&sprite)));
        let before = items_of(&base.list, image)
            .first()
            .map(|i| (i.texture, i.vertices.clone()));
        checks.push((
            format!("sprite {name} on {}", tree.nodes[image].path),
            s,
            Box::new(move |d| {
                let now = items_of(d, image)
                    .first()
                    .map(|i| (i.texture, i.vertices.clone()));
                now.is_some() && now != before
            }),
        ));
    } else {
        println!("  no other half-moon sprite: the sprite change is checked by unit tests only");
        failed += 1;
    }
    for (what, state, ok) in &checks {
        let d = again(state, &mut scene, &mut atlases)?;
        let changed = ok(&d.list);
        let back = again(&start_state, &mut scene, &mut atlases)?.list.hash();
        let fine = changed && back == start_hash;
        if !fine {
            failed += 1;
        }
        println!(
            "  {what}: {} draws, expected change {}; start state again {}{}",
            d.list.items.len(),
            if changed { "yes" } else { "NO" },
            if back == start_hash {
                "equal"
            } else {
                "DIFFERS"
            },
            if fine { "" } else { "  FAILED" }
        );
    }
    // Frames: everything the HUD's rules will change, changed each frame.
    let icons: Vec<usize> = (0..tree.nodes.len())
        .filter(|&n| {
            let p = &tree.nodes[n].path;
            p.starts_with(HUD) && p.contains("BarsPanel") && tree.nodes[n].name == "Icon"
        })
        .collect();
    let made_before = atlases.made;
    let mut times = Vec::with_capacity(FRAMES);
    let mut items = 0;
    for frame in 0..FRAMES {
        let mut s = start_state.clone();
        for (k, &n) in texts.iter().enumerate() {
            s.text.insert(n, ((frame * 37 + k * 11) % 1000).to_string());
        }
        let f = frame as f32 / FRAMES as f32;
        for &n in &filled {
            s.fill_amount.insert(n, f);
        }
        for &n in &images {
            s.renderer_alpha.insert(n, 1.0 - 0.5 * f);
        }
        for &n in &icons {
            s.transform.insert(
                n,
                LocalTransform {
                    rotation: Some(about_y(180.0 * f)),
                    ..LocalTransform::default()
                },
            );
        }
        let t = Instant::now();
        let d = again(&s, &mut scene, &mut atlases)?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        items += d.list.items.len();
        if frame == 0 {
            // The same state again: what the first frame's time was.
            let t = Instant::now();
            again(&s, &mut scene, &mut atlases)?;
            println!(
                "  frame 0: {:.3} ms, the same state again {:.3} ms; glyphs made {}",
                times[0],
                t.elapsed().as_secs_f64() * 1000.0,
                atlases.made - made_before
            );
        }
    }
    let back = again(&start_state, &mut scene, &mut atlases)?.list.hash();
    if back != start_hash {
        failed += 1;
    }
    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let p95 = sorted[(sorted.len() * 95 / 100).min(sorted.len() - 1)];
    let max = sorted[sorted.len() - 1];
    let max_at = times.iter().position(|t| *t == max).unwrap_or(0);
    if mean > TARGET_MS {
        failed += 1;
    }
    println!(
        "{FRAMES} frames ({} texts, {} filled images, {} alphas, {} icons turned each frame; {:.1} draws per frame): draw mean {mean:.3} ms, p95 {p95:.3} ms, max {max:.3} ms at frame {max_at} (target mean <= {TARGET_MS} ms){}; start state again {}",
        texts.len(),
        filled.len(),
        images.len(),
        icons.len(),
        items as f64 / FRAMES as f64,
        if mean > TARGET_MS { "  OVER" } else { "" },
        if back == start_hash {
            "equal"
        } else {
            "DIFFERS"
        }
    );
    println!(
        "  glyphs made during the frames: {}",
        atlases.made - made_before
    );
    // A fresh scene with fresh atlases: the same start hash.
    let mut fresh_scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut fresh = GlyphAtlases::new(&fonts)?;
    let h = fresh_scene
        .draw(&assets, &fonts, &set, &mut fresh, &placements, &start_state)?
        .list
        .hash();
    if h != start_hash {
        failed += 1;
    }
    println!(
        "fresh scene, start state: hash {h:016x}, {}",
        if h == start_hash {
            "equal to the warmed scene's"
        } else {
            "DIFFERS from the warmed scene's"
        }
    );
    println!(
        "glyphs made: {} in {:.3} s; failures {failed} ({:.1} s)",
        atlases.made,
        atlases.seconds,
        start.elapsed().as_secs_f64()
    );
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
