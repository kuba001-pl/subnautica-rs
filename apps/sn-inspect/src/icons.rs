//! `ui --icons` (M9u7c2, `docs/formats/ui.md` § Picked-up icons):
//! `uGUI_IconNotifier` read from the game (its canvas, `interval`,
//! `iconSize`, the two `IconAnimator`s), then its rules
//! (`sn_ui::notifier`) through a scripted queue of item sprites, checked
//! every frame against a replay written here apart from the rules, in
//! doubles:
//! - three pick-ups in one frame (the first shown at once, the others
//!   `interval` apart), one later on its own, then five more and one `To`
//!   (the other animator, its oscillation from `oscOffset`);
//! - each frame: which requests are shown, every shown icon's anchored
//!   position (x, y × the canvas rect), scale (the curve + `Oscillation`
//!   × `oscScale`) and alpha, the queue and the pool.
//!
//! `Random.value` is our generator, drawn in the same order by both.
//!
//! Every 5th frame the scene is drawn with the icons grafted: each shown
//! icon draws one item (its foreground's sprite mesh) at the rules' alpha
//! and scale about its anchored position, in the order shown (each
//! `SetAsLastSibling`); a pooled one draws nothing; the stored nodes draw
//! as before.

use std::collections::HashMap;
use std::process::ExitCode;

use sn_assets::{Assets, GlyphAtlases, UiScene, UiState};
use sn_install::GameData;
use sn_ui::clip::Curve;
use sn_ui::hud::Random;
use sn_ui::notifier::{FROM, Icon, IconAnimator, Notifier, TO};
use sn_ui::scaler::{DEFAULT_FIELD_OF_VIEW, DEFAULT_UI_SCALE, UiCamera, place};

use crate::Result;

const SCREEN: [i32; 2] = [1920, 1080];
const NEAR: f32 = 0.03;
const FAR: f32 = 1700.0;
const DT: f32 = 0.0173;
const FRAMES: usize = 400;
const SEED: u32 = 0x5eed_1c0f;

/// The script: (frame, tech type, animation).
const SCRIPT: &[(usize, &str, usize)] = &[
    (0, "Titanium", FROM),
    (0, "Copper", FROM),
    (0, "Quartz", FROM),
    (120, "Salt", FROM),
    (180, "Lead", FROM),
    (180, "Silver", FROM),
    (181, "Gold", FROM),
    (181, "Diamond", FROM),
    (182, "Lithium", FROM),
    (200, "Battery", TO),
];

/// `AnimationCurve.Evaluate` in doubles (Hermite between clamped keys).
fn eval(c: &Curve, t: f64) -> f64 {
    let (Some(first), Some(last)) = (c.keys.first(), c.keys.last()) else {
        return 0.0;
    };
    if t <= f64::from(first.time) {
        return f64::from(first.value);
    }
    if t >= f64::from(last.time) {
        return f64::from(last.value);
    }
    for w in c.keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (t0, t1) = (f64::from(a.time), f64::from(b.time));
        if t > t1 {
            continue;
        }
        let d = t1 - t0;
        let s = (t - t0) / d;
        let (s2, s3) = (s * s, s * s * s);
        return (2.0 * s3 - 3.0 * s2 + 1.0) * f64::from(a.value)
            + (s3 - 2.0 * s2 + s) * f64::from(a.out_slope) * d
            + (-2.0 * s3 + 3.0 * s2) * f64::from(b.value)
            + (s3 - s2) * f64::from(b.in_slope) * d;
    }
    f64::from(last.value)
}

/// One replayed request.
struct Req {
    /// Its index in the script.
    script: usize,
    animation: usize,
    /// The icon, by the order made.
    icon: Option<usize>,
    random: f64,
    t: f64,
}

/// `uGUI_IconNotifier` again, in doubles; icons by the order made.
struct Replay {
    pda: f64,
    time: f64,
    queue: std::collections::VecDeque<Req>,
    active: Vec<Req>,
    pool: Vec<usize>,
    made: usize,
    random: Random,
    /// Shown icons this frame: icon → (script index, position, scale,
    /// alpha).
    out: HashMap<usize, (usize, [f64; 2], [f64; 2], f64)>,
    shown_at: Vec<(usize, f64)>,
}

impl Replay {
    fn frame(
        &mut self,
        plays: &[(usize, usize)],
        a: &[IconAnimator],
        interval: f64,
        rect: [f64; 2],
    ) {
        for &(script, animation) in plays {
            self.queue.push_back(Req {
                script,
                animation,
                icon: None,
                random: 0.0,
                t: 0.0,
            });
        }
        self.pda += f64::from(DT);
        if !self.queue.is_empty() && self.pda - self.time >= interval {
            let mut r = self.queue.pop_front().expect("not empty");
            if self.pool.is_empty() {
                for _ in 0..4 {
                    self.pool.push(self.made);
                    self.made += 1;
                }
            }
            r.icon = self.pool.pop();
            r.random = f64::from(self.random.value());
            self.shown_at.push((r.script, self.pda));
            self.active.push(r);
            self.time = self.pda;
        }
        self.out.clear();
        let mut k = self.active.len();
        while k > 0 {
            k -= 1;
            let r = &mut self.active[k];
            let an = &a[r.animation];
            let t = r.t;
            let pos = [eval(&an.x, t) * rect[0], eval(&an.y, t) * rect[1]];
            let s = eval(&an.scale, t);
            let (mut o, mut o1) = (0.0, 0.0);
            let off = f64::from(an.osc_offset);
            if an.oscillate && t >= off {
                let tt = (t - off).clamp(0.0, 1.0) / (1.0 - off) * f64::from(an.duration);
                let amp = 0.5 * f64::from(an.osc_reduction).powf(-tt);
                let w = ((tt * f64::from(an.osc_frequency) + r.random.clamp(0.0, 1.0))
                    * 2.0
                    * std::f64::consts::PI)
                    .sin();
                (o, o1) = (amp * (1.0 + w), amp * (1.0 - w));
            }
            let os = f64::from(an.osc_scale);
            let icon = r.icon.expect("shown");
            if r.t == 1.0 {
                self.pool.push(icon);
                self.active.remove(k);
                continue;
            }
            self.out.insert(
                icon,
                (r.script, pos, [s + o * os, s + o1 * os], eval(&an.alpha, t)),
            );
            r.t = (r.t + f64::from(DT) / f64::from(an.duration)).clamp(0.0, 1.0);
        }
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
    let sprites = assets.sprites(&catalog)?;
    let root = assets.ugui_root(&catalog)?;
    let tree = assets.ui_tree("uGUI.prefab", &root)?;
    let start = UiState::from(&assets.ui_start_state(&layouts, &tree)?);
    let mut scene = assets.ui_scene(&layouts, &tree, &fonts, &tags.sorting_layers)?;
    let mut atlases = GlyphAtlases::new(&fonts)?;
    let camera = UiCamera::game(DEFAULT_FIELD_OF_VIEW, NEAR, FAR);
    let mut placements = HashMap::new();
    for (node, s) in assets.ui_scalers(&tree)? {
        if let Ok(p) = place(&s, &camera, DEFAULT_FIELD_OF_VIEW, SCREEN, DEFAULT_UI_SCALE) {
            placements.insert(node, p);
        }
    }
    let (mut ok, mut all) = (0usize, 0usize);
    let mut check = |good: bool, what: &str| {
        all += 1;
        if good {
            ok += 1;
        } else {
            println!("  FAIL: {what}");
        }
    };
    let before = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &start)?;
    let stored = scene.node_count();

    let config = assets.notifier_config(&layouts, &tree)?;
    let canvas = config.canvas;
    let rect = scene
        .layout_for(&assets, &set, &placements, &start)?
        .rect(canvas);
    println!(
        "uGUI_IconNotifier: canvas {} {rect:?}, interval {} s, iconSize {}",
        tree.nodes[canvas].path, config.interval, config.icon_size
    );
    for (k, a) in config.animators.iter().enumerate() {
        let at = |c: &Curve| [0.0, 0.25, 0.5, 0.75, 1.0].map(|t| c.evaluate(t));
        println!(
            "  animator {k}: {} s; x {:?}, y {:?}, scale {:?}, alpha {:?} at t = 0, ¼, ½, ¾, 1; oscillate {} from {} (reduction {}, frequency {}, × {})",
            a.duration,
            at(&a.x),
            at(&a.y),
            at(&a.scale),
            at(&a.alpha),
            a.oscillate,
            a.osc_offset,
            a.osc_reduction,
            a.osc_frequency,
            a.osc_scale
        );
    }
    check(
        config.animators.len() == 2 && config.interval > 0.0,
        "two animators (From, To) and an interval",
    );
    let material = assets.icon_material(&catalog, &layouts)?;
    let template = UiScene::from_code(
        "NotifierIcon",
        sn_assets::notifier_icon_nodes(tree.nodes[canvas].layer, &material),
    )?;
    let mut notifier = Notifier::new(config.clone(), SEED);
    let mut replay = Replay {
        pda: 0.0,
        time: -f64::from(config.interval),
        queue: Default::default(),
        active: Vec::new(),
        pool: Vec::new(),
        made: 0,
        random: Random::new(SEED),
        out: HashMap::new(),
        shown_at: Vec::new(),
    };
    // Each scripted request's sprite and its size (`sprite.rect`).
    let mut ids = Vec::new();
    for (_, tech, _) in SCRIPT {
        let s = sprites
            .item_icon(tech)
            .ok_or_else(|| format!("{tech}: no icon"))?;
        let mesh = assets.icon_sprite(&s.object)?.0;
        ids.push((
            scene.sprite_id(&s.object),
            [s.sprite.rect[2], s.sprite.rect[3]],
            mesh,
        ));
    }
    let wh = [f64::from(rect.width), f64::from(rect.height)];
    let (mut frame_bad, mut worst_pos, mut worst_scale, mut worst_alpha) =
        (0, 0.0f64, 0.0f64, 0.0f64);
    let (mut most_shown, mut draws, mut draw_bad) = (0usize, 0usize, 0usize);
    let mut shown_times = Vec::new();
    // The canvas's origin in its root canvas's space, from the first draw.
    let mut origin: Option<[f32; 2]> = None;
    let mut worst_draw = 0.0f32;
    for f in 0..FRAMES {
        let plays: Vec<(usize, usize)> = SCRIPT
            .iter()
            .enumerate()
            .filter(|(_, (at, _, _))| *at == f)
            .map(|(k, (_, _, a))| (k, *a))
            .collect();
        let real: Vec<_> = plays
            .iter()
            .map(|&(k, a)| (ids[k].0, ids[k].1, a))
            .collect();
        let mut failed = None;
        let mut make = || match scene.graft(&template, canvas) {
            Ok(n) => Icon {
                root: n,
                foreground: n + 1,
            },
            Err(e) => {
                failed = Some(e);
                Icon {
                    root: usize::MAX,
                    foreground: usize::MAX,
                }
            }
        };
        notifier.update(&real, DT, rect, &mut make);
        if let Some(e) = failed {
            return Err(format!("graft: {e}"));
        }
        for &n in &notifier.events.last_sibling {
            scene.set_as_last_sibling(n);
        }
        for &(_, at) in &notifier.events.shown {
            shown_times.push(at);
        }
        replay.frame(&plays, &config.animators, f64::from(config.interval), wh);
        // Compare: the same icons shown (by order made), each one's values.
        let s = &notifier.state;
        let mut bad = notifier.icons.len() != replay.made
            || notifier.pool.len() != replay.pool.len()
            || notifier.queue.len() != replay.queue.len();
        let mut shown_now = 0;
        for (i, icon) in notifier.icons.iter().enumerate() {
            let active = s.active.get(&icon.root).copied().unwrap_or(false);
            match replay.out.get(&i) {
                Some((_, p, sc, al)) => {
                    shown_now += 1;
                    let t = s.transform.get(&icon.root);
                    let (Some(pos), Some(scale)) =
                        (t.and_then(|t| t.anchored_position), t.and_then(|t| t.scale))
                    else {
                        bad = true;
                        continue;
                    };
                    let a = s
                        .renderer_alpha
                        .get(&icon.foreground)
                        .copied()
                        .unwrap_or(-1.0);
                    let ep = (f64::from(pos[0]) - p[0])
                        .abs()
                        .max((f64::from(pos[1]) - p[1]).abs());
                    let es = (f64::from(scale[0]) - sc[0])
                        .abs()
                        .max((f64::from(scale[1]) - sc[1]).abs());
                    let ea = (f64::from(a) - al).abs();
                    worst_pos = worst_pos.max(ep);
                    worst_scale = worst_scale.max(es);
                    worst_alpha = worst_alpha.max(ea);
                    bad |= !active || ep > 0.01 || es > 1e-4 || ea > 1e-4;
                }
                None => bad |= active,
            }
        }
        most_shown = most_shown.max(shown_now);
        if bad {
            frame_bad += 1;
            if frame_bad <= 5 {
                println!(
                    "  frame {f}: differs (made {} / {}, pool {} / {}, queue {} / {}, shown {shown_now} / {})",
                    notifier.icons.len(),
                    replay.made,
                    notifier.pool.len(),
                    replay.pool.len(),
                    notifier.queue.len(),
                    replay.queue.len(),
                    replay.out.len()
                );
            }
        }
        if f % 5 != 0 {
            continue;
        }
        // Draw: one item per shown icon, in the order shown.
        let mut state = start.clone();
        state.apply(&notifier.state);
        let d = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &state)?;
        draws += 1;
        let mut ok_draw = true;
        let mut order = Vec::new();
        for (i, icon) in notifier.icons.iter().enumerate() {
            let items: Vec<_> = d
                .list
                .items
                .iter()
                .filter(|it| it.node == icon.foreground)
                .collect();
            let root_items = d
                .list
                .items
                .iter()
                .filter(|it| it.node == icon.root)
                .count();
            let Some((script, p, sc, al)) = replay.out.get(&i) else {
                ok_draw &= items.is_empty() && root_items == 0;
                continue;
            };
            let Some(it) = items.first() else {
                ok_draw = false;
                continue;
            };
            ok_draw &= items.len() == 1 && root_items == 0;
            order.push((
                d.list.items.iter().position(|x| std::ptr::eq(x, *it)),
                *script,
            ));
            let mesh = &ids[*script].2;
            let k = config.icon_size / mesh.size[0] * mesh.pixels_per_unit;
            let want_alpha = (255.0 * *al).round() as u8;
            ok_draw &= it.vertices.len() == mesh.vertices.len()
                && it
                    .vertices
                    .iter()
                    .all(|v| v.color[3].abs_diff(want_alpha) <= 1);
            for (v, m) in it.vertices.iter().zip(&mesh.vertices) {
                // pos = origin + anchored position + scale × the sprite's.
                let local = [
                    origin.unwrap_or([0.0; 2])[0] + p[0] as f32 + sc[0] as f32 * m[0] * k,
                    origin.unwrap_or([0.0; 2])[1] + p[1] as f32 + sc[1] as f32 * m[1] * k,
                ];
                if origin.is_none() {
                    origin = Some([
                        v.pos[0] - p[0] as f32 - sc[0] as f32 * m[0] * k,
                        v.pos[1] - p[1] as f32 - sc[1] as f32 * m[1] * k,
                    ]);
                    continue;
                }
                worst_draw = worst_draw
                    .max((v.pos[0] - local[0]).abs())
                    .max((v.pos[1] - local[1]).abs());
            }
        }
        // Drawn in the order shown: later requests later.
        order.sort();
        let scripts: Vec<usize> = order.iter().map(|(_, s)| *s).collect();
        ok_draw &= scripts.windows(2).all(|w| w[0] < w[1]);
        let old = d.list.items.iter().filter(|i| i.node < stored).count();
        ok_draw &= old == before.list.items.len();
        if !ok_draw {
            draw_bad += 1;
            if draw_bad <= 5 {
                println!("  draw at frame {f}: differs (order {scripts:?}, stored {old})");
            }
        }
    }
    let gaps: Vec<f32> = shown_times.windows(2).map(|w| w[1] - w[0]).collect();
    println!(
        "{FRAMES} frames, {frame_bad} differing from the replay (position ≤ {worst_pos:.5} canvas units, scale ≤ {worst_scale:.6}, alpha ≤ {worst_alpha:.6}); {} shown at {:?}",
        shown_times.len(),
        shown_times
            .iter()
            .map(|t| format!("{t:.4}"))
            .collect::<Vec<_>>()
    );
    println!(
        "gaps between shown icons: {:?}; at most {most_shown} shown at once; {} icons made, pool {}",
        gaps.iter().map(|g| format!("{g:.4}")).collect::<Vec<_>>(),
        notifier.icons.len(),
        notifier.pool.len()
    );
    println!(
        "{draws} draws, {draw_bad} differing (vertices ≤ {worst_draw:.4} canvas units from origin + position + scale × the sprite's)"
    );
    check(frame_bad == 0, "every frame equals the replay");
    check(shown_times.len() == SCRIPT.len(), "every request is shown");
    check(
        gaps.iter().all(|g| *g >= config.interval - 1e-4),
        "requests are shown at least interval apart",
    );
    check(
        replay.shown_at.len() == shown_times.len()
            && replay
                .shown_at
                .iter()
                .zip(&shown_times)
                .all(|((_, a), b)| (a - f64::from(*b)).abs() < 1e-4),
        "shown at the replay's times",
    );
    check(
        most_shown == 3 && notifier.icons.len() == 4,
        "at most three shown at once (0.7 s ÷ 0.3 s); the first four icons suffice",
    );
    check(
        !notifier.busy() && notifier.pool.len() == 4,
        "all released at the end",
    );
    check(
        draw_bad == 0,
        "every draw: one item per shown icon, none for pooled ones, in order",
    );
    check(
        worst_draw < 0.01,
        "drawn at origin + position + scale × the sprite's vertices",
    );
    let after = scene.draw(&assets, &fonts, &set, &mut atlases, &placements, &start)?;
    check(
        after.list.items.len() == before.list.items.len()
            && after
                .list
                .items
                .iter()
                .zip(&before.list.items)
                .all(|(x, y)| x == y),
        "all hidden again: the stored nodes draw as before",
    );
    println!("checks {ok} of {all}");
    Ok(if ok == all {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
