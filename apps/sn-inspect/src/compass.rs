//! `ui --compass` (M9u6c2, `docs/formats/ui.md` § The depth and compass):
//! the compass ring (`sn_ui::compass`, driven by `sn_ui::depth`) against
//! a replay of the decompiled `uGUI_Compass`, written here apart in `f64`
//! with explicit rotation matrices (y × x × z) instead of quaternions:
//! 1. the labels' stored `anchoredPosition`s are what the editor last
//!    computed at the stored `debugDirection`: ours there, N/N within
//!    0.001 px;
//! 2. 36 headings, each drawn as the client does: the ring's 4 vertices
//!    (canvas position, UV × z, 1/z) and its `_Angle`, `_AlphaFrom`,
//!    `_AlphaTo`; each label's place and its drawn alpha;
//! 3. a timeline with the compass equipped, unequipped, and the PDA open:
//!    the ring and its labels drawn iff equipped and shown.

use std::collections::HashMap;
use std::process::ExitCode;
use std::time::Instant;

use sn_assets::{Assets, GlyphAtlases, UiDraw, UiState};
use sn_install::GameData;
use sn_ui::compass::Compass;
use sn_ui::depth::DepthCompass;
use sn_ui::hud::{HudInputs, hud_mode};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};
use sn_unity::Value;

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;

type M3 = [[f64; 3]; 3];

fn mul3(a: &M3, b: &M3) -> M3 {
    let mut m = [[0.0; 3]; 3];
    for r in 0..3 {
        for c in 0..3 {
            m[r][c] = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    m
}

/// `Quaternion.Euler` as a matrix: Ry × Rx × Rz (z applied first).
fn euler(e: [f64; 3]) -> M3 {
    let (sx, cx) = e[0].to_radians().sin_cos();
    let (sy, cy) = e[1].to_radians().sin_cos();
    let (sz, cz) = e[2].to_radians().sin_cos();
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mul3(&mul3(&ry, &rx), &rz)
}

/// The replay of `uGUI_Compass`'s projection of a plane point at
/// `direction`: canvas x, y and z.
fn project(c: &Compass, p: [f64; 2], direction: f64) -> ([f64; 2], f64) {
    let r = c.rotation.map(f64::from);
    let m = euler([r[0], r[1], r[2] - direction * 360.0]);
    let s = c.scale.map(f64::from);
    let q = [p[0] * s[0], p[1] * s[1], 0.0];
    let v: Vec<f64> = (0..3)
        .map(|i| (0..3).map(|k| m[i][k] * q[k]).sum::<f64>() + f64::from(c.position[i]))
        .collect();
    // `Perspective(fov, 1, near, far)`, `MultiplyPoint3x4`.
    let cot = 1.0 / (f64::from(c.fov).to_radians() / 2.0).tan();
    let (n, f) = (f64::from(c.near), f64::from(c.far));
    let x = cot * v[0];
    let y = cot * v[1];
    let z = (f + n) / (n - f) * v[2] + 2.0 * n * f / (n - f);
    let k = f64::from(c.scale_2d);
    ([k * x / z, k * y / z], z)
}

/// The replay of a label at `direction`: its place and alpha.
fn label(c: &Compass, angle: f64, direction: f64) -> ([f64; 2], f64) {
    let rz = f64::from(c.rotation[2]);
    let a = (rz + angle).to_radians();
    let radius = f64::from(c.radius);
    let (pos, _) = project(c, [radius * a.cos(), radius * a.sin()], direction);
    let n = (rz + angle - 90.0 - direction * 360.0) / 360.0;
    let (from, to) = (f64::from(c.alpha_from), f64::from(c.alpha_to));
    let t = ((n - n.floor() - from) / (to - from)).clamp(0.0, 1.0);
    (pos, (1.0 - (2.0 * t - 1.0).abs()).sin())
}

fn items(d: &UiDraw, node: usize) -> Vec<&sn_ui::draw::DrawItem> {
    d.list.items.iter().filter(|i| i.node == node).collect()
}

fn inputs(t: f32, compass: bool, pda: bool, yaw: f32) -> HudInputs {
    HudInputs {
        active: true,
        mode: hud_mode(false, false, false),
        oxygen: 45.0,
        oxygen_capacity: 45.0,
        health: 100.0,
        max_health: 100.0,
        food: 50.5,
        water: 90.5,
        delta_time: 1.0 / 60.0,
        time: t,
        pda_open: pda,
        healed: None,
        ate: None,
        drank: None,
        depth: 10.0,
        compass,
        yaw,
    }
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
    let layout = assets.ui_layout(&layouts, &tree)?;
    let language = sn_assets::language(game, "English")?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let start_state = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let config = assets.depth_config(&layouts, &tree, &mut scene, &language)?;
    let ring = config.ring.clone();
    let node = config.compass;
    // The ring's stored fields and material.
    let (_, fields) = tree.nodes[node]
        .components
        .iter()
        .find(|c| c.script.as_deref() == Some("uGUI_Compass"))
        .map(|c| assets.script_fields(&layouts, c))
        .ok_or("no uGUI_Compass")??;
    let debug_direction = fields
        .get("debugDirection")
        .and_then(Value::as_f64)
        .ok_or("no debugDirection")?;
    println!(
        "uGUI_Compass {}: size {}, position {:?}, rotation {:?}, scale {:?}, scale2D {}, fov {}, near {}, far {}, radius {}, alpha {}…{}, {} labels; debugDirection {debug_direction}",
        tree.nodes[node].path,
        ring.size,
        ring.position,
        ring.rotation,
        ring.scale,
        ring.scale_2d,
        ring.fov,
        ring.near,
        ring.far,
        ring.radius,
        ring.alpha_from,
        ring.alpha_to,
        ring.labels.len()
    );
    let (mut checked, mut differ) = (0usize, 0usize);
    let mut check = |what: String, ok: bool| {
        checked += 1;
        if !ok {
            differ += 1;
            println!("  DIFFERS: {what}");
        }
    };

    // 1. The stored label places.
    let mut worst: f64 = 0.0;
    let mut stored_ok = 0;
    for l in &ring.labels {
        let stored = layout.tree.nodes[l.node].rect.anchored_position;
        let (ours, alpha) = ring.label(l.angle, debug_direction as f32);
        let (replay, _) = label(&ring, f64::from(l.angle), debug_direction);
        let e = ((f64::from(ours[0]) - f64::from(stored[0])).powi(2)
            + (f64::from(ours[1]) - f64::from(stored[1])).powi(2))
        .sqrt();
        let er = ((replay[0] - f64::from(stored[0])).powi(2)
            + (replay[1] - f64::from(stored[1])).powi(2))
        .sqrt();
        worst = worst.max(e);
        println!(
            "  label {} ({}°, {:?}): stored {stored:?}, ours {ours:?} ({e:.6} px), replay ({:.4}, {:.4}) ({er:.6} px), alpha {alpha:.4}",
            l.name,
            l.angle,
            config
                .labels
                .iter()
                .find(|(n, _)| *n == l.node)
                .map(|(_, t)| t.as_str()),
            replay[0],
            replay[1]
        );
        stored_ok += usize::from(e <= 0.001);
        check(format!("label {} stored place", l.name), e <= 0.001);
    }
    println!(
        "1. stored label places at debugDirection: {stored_ok} of {} within 0.001 px (worst {worst:.6} px)",
        ring.labels.len()
    );

    // 2. 36 headings.
    let mut depth = DepthCompass::new(config.clone());
    let mut times = Vec::new();
    let (mut vertex_worst, mut label_worst, mut alpha_worst) = (0.0f64, 0.0f64, 0u8);
    for k in 0..36 {
        let yaw = 10.0 * k as f32 + 3.7;
        let mut state = start_state.clone();
        state.apply(depth.update(&inputs(k as f32, true, false, yaw)));
        let begin = Instant::now();
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
        times.push(begin.elapsed().as_secs_f64() * 1000.0);
        let direction = f64::from(sn_ui::compass::wrap(yaw / 360.0));
        let drawn = items(&d, node);
        check(
            format!("yaw {yaw}: ring drawn once ({} items)", drawn.len()),
            drawn.len() == 1,
        );
        let Some(it) = drawn.first() else {
            continue;
        };
        // The ring's mesh (local → root canvas: the draw list's vertices
        // are in the root canvas's space; compare against the node's own
        // offset, the vertex at index 0's local position).
        let s = f64::from(ring.size);
        let corners = [[-s, -s], [-s, s], [s, s], [s, -s]];
        let uvs = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let local = ring.mesh();
        let offset = [
            f64::from(it.vertices[0].pos[0]) - f64::from(local.vertices[0].pos[0]),
            f64::from(it.vertices[0].pos[1]) - f64::from(local.vertices[0].pos[1]),
        ];
        let mut ok = it.vertices.len() == 4 && it.indices == [0, 1, 2, 2, 3, 0];
        for (i, v) in it.vertices.iter().enumerate().take(4) {
            let (p, z) = project(&ring, corners[i], 0.0);
            let e = [
                (f64::from(v.pos[0]) - offset[0] - p[0]).abs(),
                (f64::from(v.pos[1]) - offset[1] - p[1]).abs(),
                (f64::from(v.uv[0]) - uvs[i][0] / z).abs(),
                (f64::from(v.uv[1]) - uvs[i][1] / z).abs(),
                (f64::from(v.uv1[0]) - 1.0 / z).abs(),
            ];
            let m = e.iter().copied().fold(0.0, f64::max);
            vertex_worst = vertex_worst.max(m);
            ok &= m <= 1e-3;
        }
        check(format!("yaw {yaw}: ring vertices"), ok);
        let m = d.materials.get(it.material);
        let want_angle = (f64::from(ring.rotation[2]) - direction * 360.0).to_radians();
        let mat_ok = m.is_some_and(|m| {
            m.shader == "uGUI/Compass"
                && (f64::from(m.float("_Angle")) - want_angle).abs() <= 1e-5
                && m.float("_AlphaFrom") == ring.alpha_from
                && m.float("_AlphaTo") == ring.alpha_to
        });
        check(format!("yaw {yaw}: ring material"), mat_ok);
        let mut line = format!(
            "  yaw {yaw:>6.1}°: direction {direction:.4}, _Angle {:.4}",
            m.map_or(f32::NAN, |m| m.float("_Angle"))
        );
        for l in &ring.labels {
            let (want, alpha) = label(&ring, f64::from(l.angle), direction);
            let got = state
                .transform
                .get(&l.node)
                .and_then(|t| t.anchored_position)
                .unwrap_or([f32::NAN; 2]);
            let e = (f64::from(got[0]) - want[0])
                .abs()
                .max((f64::from(got[1]) - want[1]).abs());
            label_worst = label_worst.max(e);
            // Each drawn quad's alpha is the colour's.
            let want_a = sn_ui::state::color32([1.0, 1.0, 1.0, alpha as f32])[3];
            let quads = items(&d, l.node);
            let a_ok = !quads.is_empty()
                && quads.iter().flat_map(|q| &q.vertices).all(|v| {
                    let da = v.color[3].abs_diff(want_a);
                    alpha_worst = alpha_worst.max(da);
                    da <= 1
                });
            check(
                format!("yaw {yaw}: label {} place {got:?} alpha", l.name),
                e <= 1e-3 && a_ok,
            );
            if alpha > 0.0 {
                line += &format!(" {} {alpha:.2}", l.name);
            }
        }
        println!("{line}");
    }
    println!(
        "2. 36 headings: worst vertex error {vertex_worst:.2e}, label place {label_worst:.2e} px, label alpha {alpha_worst}/255"
    );

    // 3. Shown iff equipped: equip at 1 s, PDA 2–2.5 s, unequip at 3 s.
    let mut depth = DepthCompass::new(config.clone());
    let mut flips = 0;
    let mut last = None;
    for frame in 0..240 {
        let t = frame as f32 / 60.0;
        let equipped = (1.0..3.0).contains(&t);
        let pda = (2.0..2.5).contains(&t);
        let mut state = start_state.clone();
        state.apply(depth.update(&inputs(t, equipped, pda, 45.0)));
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
        let want = equipped && !pda;
        let ring_drawn = !items(&d, node).is_empty();
        let labels_drawn = ring
            .labels
            .iter()
            .filter(|l| !items(&d, l.node).is_empty())
            .count();
        let ok = ring_drawn == want && labels_drawn == if want { ring.labels.len() } else { 0 };
        check(
            format!("t {t:.3}: ring drawn {ring_drawn}, {labels_drawn} labels, want {want}"),
            ok,
        );
        if last != Some(ring_drawn) {
            flips += 1;
            println!("  t {t:.3} s: ring drawn {ring_drawn}, {labels_drawn} labels");
            last = Some(ring_drawn);
        }
    }
    println!("3. equip timeline: {flips} changes of the ring's drawing over 240 frames");
    let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
    println!(
        "{checked} compass checks, {differ} differ; draw mean {mean:.3} ms; ({:.1} s)",
        start.elapsed().as_secs_f64()
    );
    Ok(if differ == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
