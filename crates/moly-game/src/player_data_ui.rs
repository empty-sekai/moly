//! Player import controls inside the shared settings modal.

use crate::{
    game_settings::{SettingsBody, SettingsPanel},
    player_data::{ImportAction, PlayerDataImport},
};
use bevy::{
    input::{keyboard::KeyboardInput, ButtonState},
    prelude::*,
};

#[derive(Component)]
pub(crate) struct ImportBody;
#[derive(Component)]
pub(crate) enum Label {
    Uid,
    Region,
    Status,
}

pub(crate) fn spawn(commands: &mut Commands, parent: Entity, font: &Handle<Font>) {
    let body = commands
        .spawn((
            Node {
                display: Display::None,
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(14),
                ..default()
            },
            ImportBody,
        ))
        .id();
    commands.entity(parent).add_child(body);
    text(commands, body, font, "PLAYER DATA", 21., None);
    text(
        commands,
        body,
        font,
        "Import the home and room layouts available in your player data.",
        15.,
        None,
    );
    let account = row(commands, body);
    button(
        commands,
        account,
        font,
        "CN",
        ImportAction::Region,
        Some(Label::Region),
    );
    text(commands, account, font, "Player UID", 17., None);
    let input = commands
        .spawn((
            Node {
                flex_grow: 1.,
                min_width: px(165),
                padding: UiRect::all(px(10)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.035, 0.06, 0.10)),
        ))
        .id();
    commands.entity(account).add_child(input);
    text(
        commands,
        input,
        font,
        "Type UID here",
        18.,
        Some(Label::Uid),
    );
    let actions = row(commands, body);
    button(
        commands,
        actions,
        font,
        "Paste UID",
        ImportAction::Paste,
        None,
    );
    button(
        commands,
        actions,
        font,
        "Clear UID",
        ImportAction::ClearUid,
        None,
    );
    button(
        commands,
        actions,
        font,
        "Fetch player",
        ImportAction::Fetch,
        None,
    );
    button(
        commands,
        actions,
        font,
        "Choose JSON",
        ImportAction::File,
        None,
    );
    text(
        commands,
        body,
        font,
        "Choose the matching region. UID requests read player data only.",
        14.,
        None,
    );
    text(commands, body, font, "", 15., Some(Label::Status));
    let commit = row(commands, body);
    button(
        commands,
        commit,
        font,
        "Import preview",
        ImportAction::Apply,
        None,
    );
    button(
        commands,
        commit,
        font,
        "Restore backup",
        ImportAction::Restore,
        None,
    );
    button(commands, commit, font, "Cancel", ImportAction::Cancel, None);
    text(commands, body, font, "Import replaces housing layouts and keeps one backup. Graphics and audio settings are retained.", 14., None);
}

fn row(commands: &mut Commands, parent: Entity) -> Entity {
    let entity = commands
        .spawn(Node {
            width: percent(100),
            align_items: AlignItems::Center,
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(10),
            row_gap: px(8),
            ..default()
        })
        .id();
    commands.entity(parent).add_child(entity);
    entity
}

fn text(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    value: &str,
    size: f32,
    label: Option<Label>,
) {
    let mut entity = commands.spawn((
        Text::new(value),
        TextFont {
            font: font.clone(),
            font_size: size,
            ..default()
        },
        TextColor(Color::srgb(0.90, 0.93, 0.98)),
    ));
    if let Some(label) = label {
        entity.insert(label);
    }
    let id = entity.id();
    commands.entity(parent).add_child(id);
}

fn button(
    commands: &mut Commands,
    parent: Entity,
    font: &Handle<Font>,
    value: &str,
    action: ImportAction,
    label: Option<Label>,
) {
    let button = commands
        .spawn((
            Button,
            action,
            Node {
                padding: UiRect::axes(px(12), px(9)),
                border_radius: BorderRadius::all(px(7)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.18, 0.26, 0.38)),
        ))
        .id();
    commands.entity(parent).add_child(button);
    text(commands, button, font, value, 16., label);
}

fn enabled(action: ImportAction, state: &PlayerDataImport) -> bool {
    match action {
        ImportAction::Cancel => true,
        ImportAction::Apply => !state.busy && state.has_preview(),
        _ => !state.busy,
    }
}

pub(crate) fn input(
    panel: Res<SettingsPanel>,
    mut state: ResMut<PlayerDataImport>,
    actions: Query<(&Interaction, &ImportAction), Changed<Interaction>>,
    mut keyboard: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut select_all: Local<bool>,
) {
    let events: Vec<_> = keyboard.read().cloned().collect();
    if !panel.open || !panel.player_data {
        return;
    }
    for (interaction, action) in &actions {
        if *interaction == Interaction::Pressed && enabled(*action, &state) {
            state.action = Some(*action);
        }
    }
    let control = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    for event in events {
        if event.state != ButtonState::Pressed {
            continue;
        }
        if control && event.key_code == KeyCode::KeyV {
            if !state.busy {
                state.action = Some(ImportAction::Paste);
            }
        } else if control && event.key_code == KeyCode::KeyA {
            *select_all = true;
        } else if event.key_code == KeyCode::Enter {
            if !state.busy {
                state.action = Some(ImportAction::Fetch);
            }
        } else if matches!(event.key_code, KeyCode::Backspace | KeyCode::Delete) {
            state.invalidate_preview();
            if *select_all || control {
                state.uid.clear();
            } else {
                state.uid.pop();
            }
            *select_all = false;
        } else if !control {
            if let Some(value) = event.text.as_deref() {
                let digits: String = value.chars().filter(char::is_ascii_digit).collect();
                if !digits.is_empty() {
                    state.invalidate_preview();
                    if *select_all {
                        state.uid.clear();
                    }
                    let remaining = 20usize.saturating_sub(state.uid.len());
                    state.uid.extend(digits.chars().take(remaining));
                    *select_all = false;
                }
            }
        }
    }
}

pub(crate) fn refresh(
    panel: Res<SettingsPanel>,
    state: Res<PlayerDataImport>,
    mut roots: Query<&mut Node, (With<ImportBody>, Without<SettingsBody>)>,
    mut labels: Query<(&Label, &mut Text)>,
    mut buttons: Query<(&ImportAction, &Interaction, &mut BackgroundColor)>,
) {
    for mut root in &mut roots {
        root.display = if panel.player_data {
            Display::Flex
        } else {
            Display::None
        };
    }
    if !panel.open || !panel.player_data {
        return;
    }
    for (label, mut value) in &mut labels {
        let next = match label {
            Label::Uid if state.uid.is_empty() => "Type UID here".into(),
            Label::Uid => state.uid.clone(),
            Label::Region => state.region.to_uppercase(),
            Label::Status => state.status.clone(),
        };
        if **value != next {
            **value = next;
        }
    }
    for (action, interaction, mut color) in &mut buttons {
        color.0 = if !enabled(*action, &state) {
            Color::srgb(0.10, 0.13, 0.18)
        } else if *interaction == Interaction::Hovered {
            Color::srgb(0.26, 0.39, 0.53)
        } else {
            Color::srgb(0.18, 0.26, 0.38)
        };
    }
}
