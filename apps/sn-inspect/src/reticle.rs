//! `ui --reticle` (M9u6d2, `docs/formats/ui.md` § The hand reticle): the
//! hand reticle's config as read, `GameInput.FormatButton(LeftHand)` step
//! by step, and its rules (`sn_ui::reticle`) through a scripted timeline
//! (nothing, a pickup, a full inventory, a pack-up item, the cursor freed,
//! an outcrop, nothing), checked every frame against a replay written
//! here apart from the rules:
//! - on a change of icon the new one's alpha rises and the old one's falls
//!   by `dt / 0.1` a frame (clamped to 0…1); to `None` every icon drops to
//!   0 at once; an icon's object is active iff its alpha is above 0;
//! - each text is the frame's (the hand text with the button's text in
//!   `HandReticleAddButtonFormat`) and is active iff not empty;
//! - the icon canvas's scale stays 1 (every hover asks for 1).
//!
//! A few frames are drawn as the client does: the hand text's sprite is a
//! quad of the sprite sheet, sized as `GenerateTextMesh` sizes it.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiState, texture_key};
use sn_install::GameData;
use sn_ui::reticle::{HAND, HAND_SUBSCRIPT, HandReticle, IconType, ReticleInputs, get_text};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 1.0 / 60.0;
const FRAMES: usize = 240;

/// What the hand is on, by frame: (icon set, hand text, button, subscript,
/// cursor free).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Aim {
    Nothing,
    Pickup,
    Full,
    PackUp,
    Free,
    Outcrop,
}

fn aim_at(frame: usize) -> Aim {
    match frame {
        0..30 => Aim::Nothing,
        30..60 => Aim::Pickup,
        60..80 => Aim::Full,
        80..100 => Aim::PackUp,
        100..120 => Aim::Free,
        120..150 => Aim::Outcrop,
        _ => Aim::Nothing,
    }
}

pub fn run(game: &GameData) -> Result<ExitCode> {
    let assets = Assets::index(game)?;
    let tags = sn_assets::physics_settings(&assets)?.tags;
    let assemblies = sn_assets::ScriptAssemblies::load(game)?;
    let layouts = assemblies.layouts()?;
    let fonts = assets.font_assets(&assemblies)?;
    let set = assets.tmp_font_set(&fonts)?;
    let catalog = assets.catalog()?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let language = sn_assets::language(game, "English")?;
    let config = assets.reticle_config(&layouts, &tree, &language)?;
    let (mut ok, mut all) = (0usize, 0usize);
    let mut check = |good: bool, what: &str| {
        all += 1;
        if good {
            ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    };

    println!(
        "HandReticle: icon canvas {} (scale {:?}), iconScaleSpeed {}, texts {:?}, progress {} / {}",
        tree.nodes[config.icon_canvas].path,
        config.icon_canvas_scale,
        config.icon_scale_speed,
        config.texts.map(|n| tree.nodes[n].name.clone()),
        tree.nodes[config.progress_image].name,
        tree.nodes[config.progress_text].name
    );
    println!(
        "  formats: add button {:?}, progress {:?}",
        config.add_button_format, config.progress_format
    );
    for i in &config.icons {
        println!(
            "  icon {:?}: {} with {} graphics",
            i.kind,
            tree.nodes[i.node].name,
            i.graphics.len()
        );
    }
    // `Awake`: one icon of each type but `None`.
    let mut kinds: Vec<i32> = config.icons.iter().map(|i| i.kind as i32).collect();
    kinds.sort_unstable();
    check(
        kinds == (1..=10).collect::<Vec<_>>(),
        "one icon of each type 1–10",
    );

    let button = sn_assets::format_button(game, &set, &language, "LeftHand")?;
    println!(
        "FormatButton(LeftHand = {}): binding {:?} → name {:?} → id {:?} → {:?}",
        button.button, button.binding, button.readable, button.id, button.text
    );
    let default = set.default_sprite.ok_or("no default sprite asset")?;
    let symbols = &set.sprites[default];
    let sprite_index = symbols.characters.iter().position(|c| c.name == button.id);
    check(
        sprite_index.is_some(),
        "the button's id is a sprite of the default asset",
    );
    check(
        button
            .text
            .starts_with(&format!("<sprite name=\"{}\" color=#", button.id)),
        "the button's text is a sprite tag",
    );

    // The timeline.
    let name = |key: &str| {
        language
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    };
    let format = |key: &str, arg: &str| name(key).replace("{0}", arg);
    let titanium = name("Titanium");
    let texts = |a: Aim| -> (IconType, String, String) {
        let b = Some(button.text.as_str());
        match a {
            Aim::Nothing | Aim::Free => (IconType::Default, String::new(), String::new()),
            Aim::Pickup => (
                IconType::Hand,
                get_text(&config, &format("PickUpFormat", &titanium), b),
                String::new(),
            ),
            Aim::Full => (IconType::Default, titanium.clone(), name("InventoryFull")),
            Aim::PackUp => (
                IconType::PackUp,
                get_text(&config, &format("PackUpFormat", &titanium), b),
                String::new(),
            ),
            Aim::Outcrop => (
                IconType::Hand,
                get_text(&config, &name("BreakLimestone"), b),
                String::new(),
            ),
        }
    };
    let mut reticle = HandReticle::new(config.clone());
    // The replay: each icon type's alpha, and the icon shown.
    let mut want: HashMap<IconType, f64> = config.icons.iter().map(|i| (i.kind, 0.0)).collect();
    want.insert(IconType::Default, 1.0);
    let mut shown = IconType::Default;
    let mut moving: HashMap<IconType, f64> = HashMap::new();
    let (mut fades, mut change_at, mut frame_checks, mut frame_fails) =
        (Vec::new(), None::<(usize, IconType)>, 0usize, 0usize);
    let start_state = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let sheet = fonts.sprite_assets[default]
        .sheet
        .as_ref()
        .map(texture_key)
        .ok_or("no sprite sheet")?;
    let mut drawn = 0;
    for frame in 0..FRAMES {
        let a = aim_at(frame);
        let (icon, hand, sub) = texts(a);
        let mut inputs = ReticleInputs {
            delta_time: DT,
            icon,
            hidden: a == Aim::Free,
            ..ReticleInputs::default()
        };
        inputs.texts[HAND] = hand.clone();
        inputs.texts[HAND_SUBSCRIPT] = sub.clone();
        let state = reticle.update(&inputs).clone();

        // Replay.
        let desired = if inputs.hidden { IconType::None } else { icon };
        if desired != shown {
            if desired == IconType::None {
                for v in want.values_mut() {
                    *v = 0.0;
                }
                moving.clear();
            } else {
                if shown != IconType::None {
                    moving.insert(shown, -1.0);
                }
                moving.insert(desired, 1.0);
                change_at = Some((frame, desired));
            }
            shown = desired;
        }
        for (k, dir) in &moving {
            let v = want.get_mut(k).ok_or("replay: unknown icon")?;
            *v = (*v + dir * f64::from(DT) / 0.1).clamp(0.0, 1.0);
        }
        if let Some((at, kind)) = change_at {
            if want[&kind] >= 1.0 {
                fades.push((frame + 1 - at) as f64 * f64::from(DT));
                change_at = None;
            }
        }
        let mut fail = |good: bool, what: String| {
            frame_checks += 1;
            if !good {
                frame_fails += 1;
                if frame_fails <= 10 {
                    println!("  frame {frame} ({a:?}): {what}");
                }
            }
        };
        for i in &config.icons {
            let w = want[&i.kind];
            for g in &i.graphics {
                // An icon never touched since the start keeps its start
                // alpha (the start state's).
                let got = state
                    .renderer_alpha
                    .get(g)
                    .copied()
                    .or_else(|| start_state.renderer_alpha.get(g).copied())
                    .map(f64::from);
                if let Some(got) = got {
                    fail(
                        (got - w).abs() < 1e-4,
                        format!("{:?} alpha {got} ≠ {w}", i.kind),
                    );
                }
            }
            let active = state
                .active
                .get(&i.node)
                .or_else(|| start_state.active.get(&i.node))
                .copied();
            if let Some(active) = active {
                fail(
                    active == (w > 0.0),
                    format!("{:?} active {active}, alpha {w}", i.kind),
                );
            }
        }
        for (k, text) in [(HAND, &hand), (HAND_SUBSCRIPT, &sub)] {
            let n = config.texts[k];
            fail(
                state.text.get(&n) == Some(text),
                format!("text {k} {:?}", state.text.get(&n)),
            );
            fail(
                state.active.get(&n) == Some(&!text.is_empty()),
                format!("text {k} active"),
            );
        }
        let scale = state
            .transform
            .get(&config.icon_canvas)
            .and_then(|t| t.scale);
        fail(
            scale == Some([1.0, 1.0, config.icon_canvas_scale[2]]),
            format!("scale {scale:?}"),
        );

        // Draw the last frame of each aim.
        if aim_at(frame + 1) != a || frame + 1 == FRAMES {
            let mut s = start_state.clone();
            s.apply(&state);
            let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &s)?;
            drawn += 1;
            let text_items: Vec<_> = d
                .list
                .items
                .iter()
                .filter(|i| i.node == config.texts[HAND])
                .collect();
            let sprites: Vec<_> = text_items.iter().filter(|i| i.texture == sheet).collect();
            let shown_icons: Vec<_> = config
                .icons
                .iter()
                .filter(|i| {
                    d.list
                        .items
                        .iter()
                        .any(|x| x.alpha > 0.0 && i.graphics.contains(&x.node))
                })
                .map(|i| i.kind)
                .collect();
            println!(
                "frame {frame} {a:?}: hand text {:?}; {} draws of it, {} of the sprite sheet; icons drawn {shown_icons:?}",
                hand,
                text_items.len(),
                sprites.len()
            );
            let with_button = matches!(a, Aim::Pickup | Aim::PackUp | Aim::Outcrop);
            check(
                sprites.len() == usize::from(with_button),
                &format!("{a:?}: sprite drawn iff the text has the button"),
            );
            if let Some(s) = sprites.first() {
                // A square quad (the glyph is square) whose height is the
                // glyph height × its scale for the text's size, in canvas
                // units of the root canvas.
                let xs: Vec<f32> = s.vertices.iter().map(|v| v.pos[0]).collect();
                let ys: Vec<f32> = s.vertices.iter().map(|v| v.pos[1]).collect();
                let w = xs.iter().cloned().fold(f32::MIN, f32::max)
                    - xs.iter().cloned().fold(f32::MAX, f32::min);
                let h = ys.iter().cloned().fold(f32::MIN, f32::max)
                    - ys.iter().cloned().fold(f32::MAX, f32::min);
                println!(
                    "  sprite quad {w:.3} × {h:.3}; colour {:?}",
                    s.vertices[0].color
                );
                check((w - h).abs() < 1e-3 && w > 1.0, "the sprite quad is square");
            }
            let want_icons: Vec<IconType> = config
                .icons
                .iter()
                .filter(|i| want[&i.kind] > 0.0)
                .map(|i| i.kind)
                .collect();
            check(
                shown_icons == want_icons,
                &format!("{a:?}: icons drawn {shown_icons:?}, replay {want_icons:?}"),
            );
        }
    }
    println!(
        "fades to full: {:?} s (0.1 s rule, {DT:.4} s frames)",
        fades.iter().map(|f| format!("{f:.4}")).collect::<Vec<_>>()
    );
    check(
        !fades.is_empty()
            && fades
                .iter()
                .all(|f| (f - 0.1).abs() <= f64::from(DT) + 1e-6),
        "every fade lasts 0.1 s within a frame",
    );
    check(frame_fails == 0, "every frame equals the replay");
    println!(
        "{frame_checks} frame checks over {FRAMES} frames, {frame_fails} differ; {drawn} frames drawn; checks {ok} of {all}"
    );
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
