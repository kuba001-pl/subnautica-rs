//! A minimal HUD of our own (M9c): oxygen and health bars with their
//! numbers, the depth, and the suffocation overlay. Not the game's UI
//! (its sprites and layout come later); the values are the rules' own
//! (`sn_sim::vitals`), written into [`Hud`] by the player each frame.

use bevy::prelude::*;

/// What the HUD shows; `player::update` fills it.
#[derive(Resource, Default)]
pub struct Hud {
    /// The player runs (false: the fly camera, nothing shown).
    pub shown: bool,
    pub oxygen: f64,
    pub capacity: f64,
    /// `Oxygen.GetSecondsLeft`.
    pub seconds_left: i64,
    pub health: f64,
    pub max_health: f64,
    pub depth: f64,
    /// The suffocation overlay's opacity (black).
    pub overlay: f64,
    /// Dead or respawning.
    pub dead: bool,
}

#[derive(Component)]
pub struct HudRoot;
#[derive(Component)]
pub struct OxygenFill;
#[derive(Component)]
pub struct HealthFill;
#[derive(Component)]
pub struct OxygenText;
#[derive(Component)]
pub struct HealthText;
#[derive(Component)]
pub struct DepthText;
#[derive(Component)]
pub struct Overlay;

const BAR_WIDTH: f32 = 220.0;
const BAR_HEIGHT: f32 = 14.0;

fn bar(
    parent: &mut ChildSpawnerCommands,
    fill: impl Component,
    colour: Color,
    text: impl Component,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Node {
                    width: Val::Px(BAR_WIDTH),
                    height: Val::Px(BAR_HEIGHT),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            ))
            .with_children(|b| {
                b.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(colour),
                    fill,
                ));
            });
            row.spawn((
                Text::new(""),
                TextFont::from_font_size(16.0),
                TextColor(Color::WHITE),
                text,
            ));
        });
}

pub fn setup(mut commands: Commands) {
    // Drawn first, so the bars stay on top of it.
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)),
        GlobalZIndex(0),
        Overlay,
    ));
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(20.0),
                bottom: Val::Px(20.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                ..default()
            },
            GlobalZIndex(1),
            Visibility::Hidden,
            HudRoot,
        ))
        .with_children(|p| {
            bar(p, OxygenFill, Color::srgb(0.25, 0.7, 1.0), OxygenText);
            bar(p, HealthFill, Color::srgb(0.85, 0.2, 0.2), HealthText);
            p.spawn((
                Text::new(""),
                TextFont::from_font_size(16.0),
                TextColor(Color::WHITE),
                DepthText,
            ));
        });
}

fn fraction(v: f64, of: f64) -> f32 {
    if of > 0.0 {
        (v / of).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

#[allow(clippy::type_complexity)]
pub fn update(
    hud: Res<Hud>,
    mut root: Query<&mut Visibility, With<HudRoot>>,
    mut fills: Query<(&mut Node, Has<OxygenFill>), Or<(With<OxygenFill>, With<HealthFill>)>>,
    mut texts: Query<
        (&mut Text, Has<OxygenText>, Has<HealthText>),
        Or<(With<OxygenText>, With<HealthText>, With<DepthText>)>,
    >,
    mut overlay: Query<&mut BackgroundColor, With<Overlay>>,
) {
    if !hud.is_changed() {
        return;
    }
    for mut v in &mut root {
        *v = if hud.shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (mut node, is_oxygen) in &mut fills {
        let f = if is_oxygen {
            fraction(hud.oxygen, hud.capacity)
        } else {
            fraction(hud.health, hud.max_health)
        };
        node.width = Val::Percent(f * 100.0);
    }
    for (mut text, is_oxygen, is_health) in &mut texts {
        text.0 = if is_oxygen {
            format!(
                "O2 {:.0} / {:.0}  ({} s)",
                hud.oxygen, hud.capacity, hud.seconds_left
            )
        } else if is_health {
            format!(
                "Health {:.0} / {:.0}{}",
                hud.health,
                hud.max_health,
                if hud.dead { "  (dead)" } else { "" }
            )
        } else {
            format!("Depth {:.0} m", hud.depth)
        };
    }
    for mut c in &mut overlay {
        let a = if hud.shown { hud.overlay as f32 } else { 0.0 };
        c.0 = Color::srgba(0.0, 0.0, 0.0, a);
    }
}
