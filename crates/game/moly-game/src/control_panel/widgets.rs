//! The panel's look: colours, and the few node builders every tab uses.
//! Buttons carry their [`PanelAction`]; [`style`] paints them from their
//! interaction and the [`Selected`] / [`Disabled`] markers a tab sets.

use super::PanelAction;
use bevy::prelude::*;

pub(super) const INK: Color = Color::srgb(0.92, 0.95, 0.98);
pub(super) const MUTED: Color = Color::srgb(0.60, 0.68, 0.77);
pub(super) const ACCENT: Color = Color::srgb(0.40, 0.84, 0.90);
pub(super) const GOOD: Color = Color::srgb(0.55, 0.88, 0.62);
pub(super) const BAD: Color = Color::srgb(0.96, 0.58, 0.58);
pub(super) const DRAWER: Color = Color::srgba(0.030, 0.045, 0.075, 0.95);
pub(super) const CARD: Color = Color::srgb(0.062, 0.085, 0.125);
pub(super) const EDGE: Color = Color::srgb(0.16, 0.23, 0.31);
const BUTTON: Color = Color::srgb(0.085, 0.115, 0.165);
const HOVER: Color = Color::srgb(0.13, 0.19, 0.27);
const PRESS: Color = Color::srgb(0.14, 0.36, 0.42);
const CHOSEN: Color = Color::srgb(0.10, 0.27, 0.33);
const PRIMARY: Color = Color::srgb(0.08, 0.38, 0.43);
const PRIMARY_HOVER: Color = Color::srgb(0.12, 0.48, 0.52);
const DANGER_EDGE: Color = Color::srgb(0.45, 0.25, 0.29);
const OFF: Color = Color::srgb(0.07, 0.09, 0.12);
const OFF_INK: Color = Color::srgb(0.38, 0.44, 0.51);

/// The selected one of a group (a tab, an enum value).
#[derive(Component)]
pub(super) struct Selected;

/// A control that does not apply now; pressing it does nothing.
#[derive(Component)]
pub(super) struct Disabled;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Plain,
    Primary,
    Danger,
    Chip,
}

pub(super) fn attach(c: &mut Commands, parent: Entity, node: Node) -> Entity {
    let id = c.spawn(node).id();
    c.entity(parent).add_child(id);
    id
}

pub(super) fn column(c: &mut Commands, parent: Entity, gap: f32) -> Entity {
    attach(
        c,
        parent,
        Node {
            width: percent(100),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(gap),
            flex_shrink: 0.,
            ..default()
        },
    )
}

/// A row that wraps onto more lines when it is too narrow.
pub(super) fn row(c: &mut Commands, parent: Entity, gap: f32) -> Entity {
    attach(
        c,
        parent,
        Node {
            width: percent(100),
            min_width: px(0),
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(gap),
            row_gap: px(gap),
            flex_shrink: 0.,
            ..default()
        },
    )
}

/// A framed group with an optional heading.
pub(super) fn card(c: &mut Commands, parent: Entity, font: &Handle<Font>, heading: Option<&str>) -> Entity {
    let id = attach(
        c,
        parent,
        Node {
            width: percent(100),
            min_width: px(0),
            flex_direction: FlexDirection::Column,
            row_gap: px(8),
            padding: UiRect::all(px(12)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            flex_shrink: 0.,
            ..default()
        },
    );
    c.entity(id).insert((BackgroundColor(CARD), BorderColor::all(EDGE)));
    if let Some(heading) = heading {
        text(c, id, font, heading, 13., ACCENT);
    }
    id
}

pub(super) fn text(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    value: impl Into<String>,
    size: f32,
    color: Color,
) -> Entity {
    let id = c
        .spawn((
            Text::new(value),
            TextFont {
                font: font.clone(),
                font_size: size,
                ..default()
            },
            TextColor(color),
            Node {
                min_width: px(0),
                flex_shrink: 0.,
                ..default()
            },
        ))
        .id();
    c.entity(parent).add_child(id);
    id
}

/// A label and its value on one line, the value taking the rest.
pub(super) fn pair(c: &mut Commands, parent: Entity, font: &Handle<Font>, label: &str, value: impl Into<String>) -> Entity {
    let line = attach(
        c,
        parent,
        Node {
            width: percent(100),
            min_width: px(0),
            column_gap: px(10),
            flex_shrink: 0.,
            ..default()
        },
    );
    let key = text(c, line, font, label, 12., MUTED);
    c.entity(key).insert(Node {
        width: px(96),
        min_width: px(96),
        flex_shrink: 0.,
        ..default()
    });
    let value = text(c, line, font, value, 12., INK);
    c.entity(value).insert(Node {
        flex_grow: 1.,
        flex_shrink: 1.,
        min_width: px(0),
        ..default()
    });
    line
}

pub(super) fn button(
    c: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    label: &str,
    action: PanelAction,
    kind: Kind,
) -> Entity {
    let (padding, height, size) = match kind {
        Kind::Chip => (UiRect::axes(px(10), px(5)), px(30), 13.),
        _ => (UiRect::axes(px(14), px(7)), px(36), 14.),
    };
    let id = c
        .spawn((
            Button,
            action,
            kind,
            Node {
                min_height: height,
                padding,
                flex_shrink: 0.,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(if kind == Kind::Chip { 15 } else { 9 })),
                ..default()
            },
            BackgroundColor(BUTTON),
            BorderColor::all(EDGE),
        ))
        .id();
    c.entity(parent).add_child(id);
    text(c, id, font, label, size, INK);
    id
}

/// Paints every panel button from its interaction and markers.
pub(super) fn style(
    mut buttons: Query<
        (
            &Kind,
            &Interaction,
            Has<Selected>,
            Has<Disabled>,
            &Children,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        With<PanelAction>,
    >,
    mut inks: Query<&mut TextColor>,
) {
    for (kind, interaction, selected, disabled, children, mut background, mut border) in &mut buttons {
        let fill = if disabled {
            OFF
        } else if *interaction == Interaction::Pressed {
            PRESS
        } else if *kind == Kind::Primary {
            if *interaction == Interaction::Hovered { PRIMARY_HOVER } else { PRIMARY }
        } else if selected {
            CHOSEN
        } else if *interaction == Interaction::Hovered {
            HOVER
        } else {
            BUTTON
        };
        background.set_if_neq(BackgroundColor(fill));
        border.set_if_neq(BorderColor::all(if selected && !disabled {
            ACCENT
        } else if *kind == Kind::Danger && !disabled {
            DANGER_EDGE
        } else {
            EDGE
        }));
        for child in children {
            if let Ok(mut ink) = inks.get_mut(*child) {
                ink.set_if_neq(TextColor(if disabled { OFF_INK } else { INK }));
            }
        }
    }
}
