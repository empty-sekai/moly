//! Modal input, UTF-8/IME editing and explicit intent. No runtime state is faked.
use super::*;
use std::sync::{Arc, Mutex};

#[derive(Resource, Default)]
pub(super) struct PasteInbox(Arc<Mutex<Option<Result<String, String>>>>);
pub(super) fn install(app: &mut App) {
    app.init_resource::<PasteInbox>();
}

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct LibraryInput<'w, 's> {
    keyboard: MessageReader<'w, 's, KeyboardInput>,
    ime: MessageReader<'w, 's, Ime>,
    wheels: MessageReader<'w, 's, MouseWheel>,
    actions: Query<'w, 's, (&'static Interaction, &'static LibraryAction), Changed<Interaction>>,
    scroll: Query<
        'w,
        's,
        (
            &'static Region,
            &'static RelativeCursorPosition,
            &'static ComputedNode,
            &'static mut ScrollPosition,
        ),
    >,
    windows: Query<'w, 's, &'static mut Window, With<PrimaryWindow>>,
    cancel_talk: MessageWriter<'w, TalkCancelRequest>,
    fixtures: MessageWriter<'w, PlayerFixtureRequest>,
    pub(super) settings: MessageWriter<'w, crate::game_settings::SettingsPanelRequest>,
    paste: Res<'w, PasteInbox>,
}

pub(crate) fn input(
    mut state: ResMut<ContentLibrary>,
    catalog: Res<LibraryCatalog>,
    world: Res<LibraryContext>,
    mut io: LibraryInput,
    mut modifiers: Local<Modifiers>,
) {
    state.release_guard = state.release_guard.saturating_sub(1);
    state.cleanup_frames = state.cleanup_frames.saturating_sub(1);
    if io
        .windows
        .iter()
        .next()
        .is_some_and(|window| !window.focused)
    {
        modifiers.0 = 0;
    }
    // A press and release may be delivered in the same frame. ButtonInput's
    // final snapshot loses the Ctrl modifier in that case. Preserve the order
    // of the actual OS keyboard events, including while the library is closed.
    let mut keyboard: Vec<_> = io
        .keyboard
        .read()
        .cloned()
        .map(|event| {
            modifiers.update(event.key_code, event.state);
            let command = modifiers.command();
            (event, command)
        })
        .collect();
    let ime: Vec<_> = io.ime.read().cloned().collect();
    let wheels: Vec<_> = io.wheels.read().cloned().collect();
    let mut actions: Vec<_> = io
        .actions
        .iter()
        .filter(|(interaction, _)| **interaction == Interaction::Pressed)
        .map(|(_, action)| *action)
        .collect();
    for (event, _) in &keyboard {
        if event.state != ButtonState::Pressed || event.repeat {
            continue;
        }
        if event.key_code == KeyCode::F9 {
            actions.push(LibraryAction::Toggle);
        }
        if state.watching && event.key_code == KeyCode::Escape {
            actions.push(LibraryAction::Stop);
        }
    }
    // Browser text entry and shortcuts are handled by real DOM controls.
    // Drain Bevy events without interpreting them as catalogue navigation.
    if state.external_ui {
        keyboard.clear();
        actions.clear();
    }
    let was_composing = !state.ime_preedit.is_empty();
    let mut committed = false;
    if state.open && state.search_focus {
        for event in ime {
            match event {
                Ime::Preedit { value, .. } => {
                    state.ime_preedit = value;
                    state.changed();
                }
                Ime::Commit { value, .. } => {
                    insert_text(&mut state, &value);
                    state.ime_preedit.clear();
                    committed = true;
                }
                Ime::Disabled { .. } => state.ime_preedit.clear(),
                _ => {}
            }
        }
        let pasted = io
            .paste
            .0
            .lock()
            .ok()
            .and_then(|mut mailbox| mailbox.take());
        if let Some(result) = pasted {
            match result {
                Ok(text) => insert_text(&mut state, &text),
                Err(reason) => state.status = reason,
            }
        }
    } else {
        let _ = io.paste.0.lock().map(|mut mailbox| mailbox.take());
    }
    if state.open {
        for (event, control) in keyboard {
            if event.state != ButtonState::Pressed {
                continue;
            }
            if (was_composing || committed || !state.ime_preedit.is_empty()) && !control {
                continue;
            }
            if control {
                match event.key_code {
                    KeyCode::KeyF => {
                        state.picker_open = false;
                        state.search_focus = true;
                        state.select_all = true;
                        state.changed();
                    }
                    KeyCode::KeyA if state.search_focus => {
                        state.select_all = true;
                        state.changed();
                    }
                    KeyCode::KeyV if state.search_focus => request_paste(&mut state, &io.paste),
                    KeyCode::Digit1 => actions.push(LibraryAction::Tab(LibraryTab::Conversations)),
                    KeyCode::Digit2 => actions.push(LibraryAction::Tab(LibraryTab::Furniture)),
                    KeyCode::Digit3 => actions.push(LibraryAction::Tab(LibraryTab::Performances)),
                    KeyCode::Digit4 => actions.push(LibraryAction::Tab(LibraryTab::Activities)),
                    KeyCode::Enter => actions.push(LibraryAction::Play),
                    KeyCode::Backspace if state.search_focus => {
                        state.search.clear();
                        state.search_cursor = 0;
                        state.reset_browse();
                    }
                    _ => {}
                }
                continue;
            }
            if state.picker_open {
                if event.key_code == KeyCode::Escape {
                    actions.push(LibraryAction::DismissPicker);
                }
                continue;
            }
            match event.key_code {
                KeyCode::Escape => actions.push(LibraryAction::Close),
                KeyCode::Tab => {
                    state.search_focus = !state.search_focus;
                    state.select_all = state.search_focus;
                    state.changed();
                }
                KeyCode::Enter if state.search_focus => {
                    state.search_focus = false;
                    state.select_all = false;
                    state.changed();
                }
                KeyCode::Enter => actions.push(LibraryAction::Play),
                KeyCode::ArrowUp | KeyCode::ArrowDown => {
                    state.search_focus = false;
                    let index = state.selected_index();
                    let next = if event.key_code == KeyCode::ArrowUp {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(state.filtered.len().saturating_sub(1))
                    };
                    state.select_index(next);
                }
                KeyCode::PageUp => actions.push(LibraryAction::PreviousPage),
                KeyCode::PageDown => actions.push(LibraryAction::NextPage),
                KeyCode::Home if !state.search_focus => state.select_index(0),
                KeyCode::End if !state.search_focus => {
                    let last = state.filtered.len().saturating_sub(1);
                    state.select_index(last);
                }
                KeyCode::ArrowLeft if state.search_focus => {
                    state.search_cursor = state.search_cursor.saturating_sub(1);
                    state.select_all = false;
                    state.changed();
                }
                KeyCode::ArrowRight if state.search_focus => {
                    state.search_cursor =
                        (state.search_cursor + 1).min(state.search.chars().count());
                    state.select_all = false;
                    state.changed();
                }
                KeyCode::Home if state.search_focus => {
                    state.search_cursor = 0;
                    state.select_all = false;
                    state.changed();
                }
                KeyCode::End if state.search_focus => {
                    state.search_cursor = state.search.chars().count();
                    state.select_all = false;
                    state.changed();
                }
                KeyCode::Backspace | KeyCode::Delete if state.search_focus => {
                    erase(&mut state, event.key_code == KeyCode::Backspace)
                }
                _ => {
                    if let Some(text) = event.text.as_deref() {
                        if !text.is_empty() && text.chars().any(|ch| !ch.is_control()) {
                            state.search_focus = true;
                            insert_text(&mut state, text);
                        }
                    }
                }
            }
        }
        for event in wheels {
            for (region, cursor, computed, mut position) in &mut io.scroll {
                if !cursor.cursor_over()
                    || (state.picker_open && *region != Region::CharacterChoices)
                {
                    continue;
                }
                if *region == Region::ListViewport && !state.picker_open {
                    let amount = if event.y > 0. {
                        -3isize
                    } else if event.y < 0. {
                        3
                    } else {
                        0
                    };
                    state.offset = state
                        .offset
                        .saturating_add_signed(amount)
                        .min(state.filtered.len().saturating_sub(state.page_size));
                    state.changed();
                } else if matches!(region, Region::DetailScroll | Region::CharacterChoices) {
                    let delta = -event.y
                        * if event.unit == MouseScrollUnit::Line {
                            36.
                        } else {
                            1.
                        };
                    let maximum = ((computed.content_size().y - computed.size().y)
                        * computed.inverse_scale_factor())
                    .max(0.);
                    position.y = (position.y + delta).clamp(0., maximum);
                }
            }
        }
    }
    state.rebuild_results(&catalog, &world);
    for action in actions {
        apply_action(action, &mut state, &catalog, &world, &mut io);
    }
    for command in bridge::drain_commands() {
        bridge::apply_command(command, &mut state, &catalog, &world, &mut io);
        state.rebuild_results(&catalog, &world);
    }
    state.rebuild_results(&catalog, &world);
    if let Ok(mut window) = io.windows.single_mut() {
        window.ime_enabled = state.open && state.search_focus;
        if window.ime_enabled {
            window.ime_position = Vec2::new(72., if window.width() < 860. { 166. } else { 152. });
        }
    }
}
pub(super) fn apply_action(
    action: LibraryAction,
    state: &mut ContentLibrary,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
    io: &mut LibraryInput,
) {
    match action {
        LibraryAction::Toggle => {
            if state.watching {
                state.open = true;
                state.watching = false;
                state.search_focus = false;
                state.changed();
            } else if state.open {
                close(state, io);
            } else {
                state.open = true;
                state.search_focus = false;
                state.release_guard = 2;
                state.changed();
            }
        }
        LibraryAction::Return => {
            state.open = true;
            state.watching = false;
            state.search_focus = false;
            state.changed();
        }
        LibraryAction::Continue => {
            state.open = false;
            state.watching = true;
            state.search_focus = false;
            state.release_guard = 2;
            state.changed();
        }
        LibraryAction::Close => close(state, io),
        LibraryAction::Stop | LibraryAction::RestoreScene => {
            cancel(io);
            state.pending = None;
            state.stopping = true;
            state.cleanup_frames = 2;
            state.watching = false;
            state.open = true;
            state.release_guard = 2;
            state.status = "正在结束播放并恢复场景…".into();
            state.changed();
        }
        LibraryAction::Play if state.open || state.watching || state.external_ui => {
            if let Some(reason) = context::selected_reason(state, catalog, world) {
                state.status = reason;
                state.changed();
                return;
            }
            let Some(key) = state.selected else {
                return;
            };
            let target = if state.mode == ExperienceMode::CurrentScene {
                context::selected_instance(state, catalog, world)
                    .map(|instance| instance.target.clone())
            } else {
                None
            };
            cancel(io);
            state.next_ticket = state.next_ticket.wrapping_add(1);
            state.pending = Some(PlaybackChoice {
                key,
                target,
                ticket: state.next_ticket,
                mode: state.mode,
            });
            state.cleanup_frames = 2;
            state.stopping = false;
            state.search_focus = false;
            state.status = "正在准备所选内容…".into();
            state.changed();
        }
        _ if !state.open && !state.external_ui => {}
        LibraryAction::Back => {
            state.narrow_detail = false;
            state.changed();
        }
        LibraryAction::Tab(tab) => {
            state.tab = tab;
            state.special_only = false;
            state.related_fixture = None;
            state.picker_open = false;
            state.search_focus = false;
            state.status.clear();
            state.reset_browse();
        }
        LibraryAction::SetScope(scope) => {
            if scope == Scope::Here && state.mode == ExperienceMode::Independent {
                state.mode = ExperienceMode::CurrentScene;
            }
            state.scope = scope;
            state.reset_browse();
        }
        LibraryAction::SetMode(mode) => {
            state.mode = mode;
            state.scope = if mode == ExperienceMode::Independent {
                Scope::All
            } else {
                Scope::Here
            };
            state.status.clear();
            state.reset_browse();
        }
        LibraryAction::SpecialFilter => {
            state.special_only = !state.special_only;
            state.reset_browse();
        }
        LibraryAction::CharacterPicker => {
            state.picker_open = !state.picker_open;
            state.search_focus = false;
            state.changed();
        }
        LibraryAction::SetCharacter(unit) => {
            state.character = unit;
            state.picker_open = false;
            state.reset_browse();
        }
        LibraryAction::DismissPicker => {
            state.picker_open = false;
            state.changed();
        }
        LibraryAction::ClearRelated => {
            state.related_fixture = None;
            state.reset_browse();
        }
        LibraryAction::RelatedActivities(id) => {
            state.tab = LibraryTab::Activities;
            state.related_fixture = Some(id);
            state.scope = Scope::All;
            state.special_only = false;
            state.character = None;
            state.search.clear();
            state.search_cursor = 0;
            state.reset_browse();
        }
        LibraryAction::RelatedStory(id) => {
            state.tab = LibraryTab::Performances;
            state.related_fixture = None;
            state.scope = Scope::All;
            state.special_only = false;
            state.character = None;
            state.search = format!("#{id}");
            state.search_cursor = state.search.chars().count();
            state.reset_browse();
        }
        LibraryAction::Related(id) => {
            state.tab = LibraryTab::Performances;
            state.related_fixture = Some(id);
            state.scope = Scope::All;
            state.special_only = false;
            state.character = None;
            state.search.clear();
            state.search_cursor = 0;
            state.reset_browse();
        }
        LibraryAction::FocusSearch => {
            state.search_focus = true;
            state.search_cursor = state.search.chars().count();
            state.select_all = false;
            state.changed();
        }
        LibraryAction::ClearSearch => {
            state.search.clear();
            state.search_cursor = 0;
            state.ime_preedit.clear();
            state.search_focus = true;
            state.reset_browse();
        }
        LibraryAction::Select(key) => {
            if state.filtered.contains(&key) {
                if state.selected != Some(key) {
                    state.selected_uid = None;
                }
                state.selected = Some(key);
                state.narrow_detail = true;
                state.search_focus = false;
                state.status.clear();
                state.changed();
            }
        }
        LibraryAction::PreviousPage => {
            let next = state.offset.saturating_sub(state.page_size);
            state.select_index(next);
        }
        LibraryAction::NextPage => {
            let next = (state.offset + state.page_size).min(state.filtered.len().saturating_sub(1));
            state.select_index(next);
        }
        LibraryAction::PreviousInstance | LibraryAction::NextInstance => {
            if let Some(key) = state.selected {
                let targets = context::instances_for(key, catalog, world);
                if !targets.is_empty() {
                    let index = targets
                        .iter()
                        .position(|row| {
                            Some(row.target.uid.as_str()) == state.selected_uid.as_deref()
                        })
                        .unwrap_or(0);
                    let next = if action == LibraryAction::PreviousInstance {
                        (index + targets.len() - 1) % targets.len()
                    } else {
                        (index + 1) % targets.len()
                    };
                    state.selected_uid = Some(targets[next].target.uid.clone());
                    state.changed();
                }
            }
        }
        _ => {}
    }
}
fn cancel(io: &mut LibraryInput) {
    io.cancel_talk.write(TalkCancelRequest);
    io.fixtures.write(PlayerFixtureRequest::Cancel(
        PlayerFixtureCancelReason::User,
    ));
}
fn close(state: &mut ContentLibrary, io: &mut LibraryInput) {
    if state.active.is_some() || state.pending.is_some() {
        cancel(io);
        state.stopping = true;
        state.cleanup_frames = 2;
    }
    state.close();
}
fn insert_text(state: &mut ContentLibrary, text: &str) {
    let mut characters: Vec<char> = if state.select_all {
        Vec::new()
    } else {
        state.search.chars().collect()
    };
    let cursor = if state.select_all {
        0
    } else {
        state.search_cursor.min(characters.len())
    };
    let inserted: Vec<char> = text
        .chars()
        .map(|ch| {
            if matches!(ch, '\r' | '\n' | '\t') {
                ' '
            } else {
                ch
            }
        })
        .filter(|ch| !ch.is_control())
        .take(MAX_QUERY.saturating_sub(characters.len()))
        .collect();
    state.search_cursor = cursor + inserted.len();
    characters.splice(cursor..cursor, inserted);
    state.search = characters.into_iter().collect();
    state.select_all = false;
    state.reset_browse();
}
fn erase(state: &mut ContentLibrary, backwards: bool) {
    if state.select_all {
        state.search.clear();
        state.search_cursor = 0;
    } else {
        let mut chars: Vec<_> = state.search.chars().collect();
        let cursor = state.search_cursor.min(chars.len());
        if backwards && cursor > 0 {
            chars.remove(cursor - 1);
            state.search_cursor = cursor - 1;
        } else if !backwards && cursor < chars.len() {
            chars.remove(cursor);
        }
        state.search = chars.into_iter().collect();
    }
    state.select_all = false;
    state.reset_browse();
}
#[cfg(not(target_arch = "wasm32"))]
fn request_paste(state: &mut ContentLibrary, _inbox: &PasteInbox) {
    match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
        Ok(text) => insert_text(state, &text),
        Err(_) => {
            state.status = "剪贴板暂时不可读，仍可直接输入关键词".into();
            state.changed();
        }
    }
}
#[cfg(target_arch = "wasm32")]
fn request_paste(state: &mut ContentLibrary, inbox: &PasteInbox) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let mailbox = inbox.0.clone();
    let promise = window.navigator().clipboard().read_text();
    state.status = "正在读取剪贴板…".into();
    wasm_bindgen_futures::spawn_local(async move {
        let result = wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .ok()
            .and_then(|value| value.as_string())
            .ok_or_else(|| "浏览器未允许读取剪贴板，仍可直接输入关键词".into());
        if let Ok(mut inbox) = mailbox.lock() {
            *inbox = Some(result);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_editing_preserves_complete_characters() {
        let mut state = ContentLibrary::default();
        insert_text(&mut state, "你好Moly");
        state.search_cursor = 1;
        insert_text(&mut state, "世界");
        assert_eq!(state.search, "你世界好Moly");
        erase(&mut state, true);
        assert_eq!(state.search, "你世好Moly");
    }
    #[test]
    fn select_all_and_paste_are_bounded() {
        let mut state = ContentLibrary::default();
        insert_text(&mut state, "旧查询");
        state.select_all = true;
        insert_text(&mut state, &"新".repeat(MAX_QUERY + 20));
        assert_eq!(state.search.chars().count(), MAX_QUERY);
    }
}

/// Per-event command-key state, not a per-frame final button snapshot.
#[derive(Default)]
pub(crate) struct Modifiers(u8);
impl Modifiers {
    fn update(&mut self, key: KeyCode, state: ButtonState) {
        let bit = match key {
            KeyCode::ControlLeft => 1,
            KeyCode::ControlRight => 2,
            KeyCode::SuperLeft => 4,
            KeyCode::SuperRight => 8,
            _ => return,
        };
        if state == ButtonState::Pressed {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
    fn command(&self) -> bool {
        self.0 != 0
    }
}
#[cfg(test)]
mod event_order_tests {
    use super::*;
    #[test]
    fn ctrl_a_does_not_insert_a_when_the_whole_chord_arrives_in_one_frame() {
        let mut modifiers = Modifiers::default();
        let mut command_at_a = false;
        for (key, state) in [
            (KeyCode::ControlLeft, ButtonState::Pressed),
            (KeyCode::KeyA, ButtonState::Pressed),
            (KeyCode::KeyA, ButtonState::Released),
            (KeyCode::ControlLeft, ButtonState::Released),
        ] {
            modifiers.update(key, state);
            if key == KeyCode::KeyA && state == ButtonState::Pressed {
                command_at_a = modifiers.command();
            }
        }
        assert!(command_at_a);
        assert!(!modifiers.command());
    }
    #[test]
    fn releasing_one_control_does_not_release_the_other() {
        let mut modifiers = Modifiers::default();
        modifiers.update(KeyCode::ControlLeft, ButtonState::Pressed);
        modifiers.update(KeyCode::ControlRight, ButtonState::Pressed);
        modifiers.update(KeyCode::ControlLeft, ButtonState::Released);
        assert!(modifiers.command());
        modifiers.update(KeyCode::ControlRight, ButtonState::Released);
        assert!(!modifiers.command());
    }
}
