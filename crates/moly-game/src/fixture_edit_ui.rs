//! Original furniture editor prefabs, driven by the single EditCommand/EditView owner.
//!
//! The host owns only view bindings, selector motion and scroll offset. Drafts,
//! UID ownership, validation, inventory and saving remain in fixture_edit.

mod auxiliary;
mod compose;
mod icons;

use crate::{
    action_button::ActionTapConsumed,
    audio::SeRequests,
    balloon::{BALLOON_LAYER, canvas_scale},
    fixture::EditableFixture,
    fixture_edit::{EditCommand, EditSelectionView, EditView, PutStatus},
    gesture::{GestureEvent, GestureKind, GestureState},
    sitemap::SITEMAP_LAYER,
    ui_layers::{LayerId, UiLayerStack},
    ui_layout::{UiLayouts, UiPrefabView},
};
use bevy::{
    camera::visibility::RenderLayers,
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    window::PrimaryWindow,
};
pub(crate) use icons::{EditorIcons, load, parse};

/// Included with every pristine template before the existing glyph-atlas bake.
/// Infinity identifies the four existing unlimited offline catalog mocks; it
/// is not a claim about real-account inventory.
pub(crate) const FIXED_TEXTS: &[&str] = &["0123456789∞"];

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChoiceOrigin {
    Placed(String),
    Inventory(String),
    OfflineCatalog(usize),
}

/// Identity/data-set signature only: moving a selected item must not clone its
/// whole list again or reset pointer/scroll ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ItemChoice {
    fixture_id: i32,
    origin: ChoiceOrigin,
    editable: bool,
}

impl ItemChoice {
    fn unlimited(&self) -> bool {
        matches!(self.origin, ChoiceOrigin::OfflineCatalog(_))
    }

    fn is_placed(&self) -> bool {
        matches!(self.origin, ChoiceOrigin::Placed(_))
    }

    fn command(&self) -> EditCommand {
        match &self.origin {
            ChoiceOrigin::Placed(uid) => EditCommand::SelectPlaced { uid: uid.clone() },
            ChoiceOrigin::Inventory(uid) => EditCommand::SelectInventory { uid: uid.clone() },
            ChoiceOrigin::OfflineCatalog(index) => EditCommand::SelectCatalog { index: *index },
        }
    }

    fn selected(&self, selected: Option<&EditSelectionView>) -> bool {
        let Some(selected) = selected else {
            return false;
        };
        match &self.origin {
            ChoiceOrigin::Placed(uid) | ChoiceOrigin::Inventory(uid) => selected.item.uid == *uid,
            ChoiceOrigin::OfflineCatalog(_) => {
                selected.is_new_mock && selected.item.fixture_id == self.fixture_id
            }
        }
    }
}

fn choices(edit: &EditView, icons: &EditorIcons) -> Vec<ItemChoice> {
    let mut result = Vec::new();
    for (rows, inventory) in [(&edit.placed_rows, false), (&edit.inventory, true)] {
        for row in rows {
            // The ordinary texture-1 provider covers the shipped editable set.
            // An unknown image is an explicit source gap, never a generated icon.
            if !icons.by_fixture.contains_key(&row.fixture_id) {
                continue;
            }
            result.push(ItemChoice {
                fixture_id: row.fixture_id,
                origin: if inventory {
                    ChoiceOrigin::Inventory(row.uid.clone())
                } else {
                    ChoiceOrigin::Placed(row.uid.clone())
                },
                editable: row.editable,
            });
        }
    }
    for row in &edit.catalog {
        if !row.unlimited_mock || !icons.by_fixture.contains_key(&row.fixture_id) {
            continue;
        }
        result.push(ItemChoice {
            fixture_id: row.fixture_id,
            origin: ChoiceOrigin::OfflineCatalog(row.index),
            editable: true,
        });
    }
    result
}

#[derive(Component)]
pub(crate) struct EditorRoot {
    bindings: compose::Bindings,
    choices: Vec<ItemChoice>,
    site_id: u32,
    seen_revision: u64,
}

#[derive(Component)]
pub(crate) enum EditorAuxiliary {
    Header { back: String },
    Exit(auxiliary::ExitBindings),
}

#[derive(Resource)]
pub(crate) struct EditorSpawned;

#[derive(Clone, Copy)]
enum DragTarget {
    Selector,
    List,
}

#[derive(Default)]
struct SelectorMotion {
    position: f32,
    from: f32,
    target: f32,
    elapsed: f32,
}

impl SelectorMotion {
    fn reset(&mut self, position: f32) {
        self.position = position;
        self.from = position;
        self.target = position;
        self.elapsed = 0.25;
    }

    fn target(&mut self, target: f32) {
        if self.target != target {
            self.from = self.position;
            self.target = target;
            self.elapsed = 0.;
        }
    }

    fn advance(&mut self, delta: f32) {
        // ExpansionContentListSelector.SetTweenTargetPosition: duration .25,
        // DOTween Ease 4 = InOutSine .
        self.elapsed = (self.elapsed + delta).min(0.25);
        let t = self.elapsed / 0.25;
        let eased = -((std::f32::consts::PI * t).cos() - 1.) * 0.5;
        self.position = self.from + (self.target - self.from) * eased;
    }
}

#[derive(Resource, Default)]
pub(crate) struct EditorUiState {
    hidden: bool,
    scroll: f32,
    drag: Option<DragTarget>,
    motion: SelectorMotion,
    was_active: bool,
    selected_uid: Option<String>,
}

pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    icons: Res<EditorIcons>,
    edit: Res<EditView>,
    spawned: Option<Res<EditorSpawned>>,
) {
    if spawned.is_some()
        || !icons.ready
        || [
            "EditorSource",
            "EditorFloor",
            "EditorOutdoor",
            "EditorCell",
            "EditorTab",
            auxiliary::HEADER,
            auxiliary::EXIT,
        ]
        .iter()
        .any(|key| !layouts.ready(key, &server))
    {
        return;
    }
    let choices = choices(&edit, &icons);
    let (document, bindings) = compose::compose(&layouts, &edit, choices.clone())
        .unwrap_or_else(|error| panic!("source furniture editor composition: {error}"));
    layouts
        .replace_runtime_document(compose::RUNTIME, document, &server)
        .expect("editor source document installation");
    let mut view = UiPrefabView::new(compose::RUNTIME, BALLOON_LAYER);
    compose::apply_static(
        &mut view,
        layouts.document(compose::RUNTIME).unwrap(),
        &bindings,
        &icons,
    );
    commands.spawn((
        EditorRoot {
            bindings,
            choices,
            site_id: edit.site_id,
            seen_revision: u64::MAX,
        },
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(BALLOON_LAYER),
        view,
    ));
    let mut header = UiPrefabView::new(auxiliary::HEADER, BALLOON_LAYER);
    let back = auxiliary::header(layouts.document(auxiliary::HEADER).unwrap(), &mut header)
        .expect("original shared BackUI binding");
    commands.spawn((
        EditorAuxiliary::Header { back },
        Visibility::Hidden,
        Transform::from_xyz(0., 0., 20.),
        RenderLayers::layer(BALLOON_LAYER),
        header,
    ));
    let mut exit = UiPrefabView::new(auxiliary::EXIT, SITEMAP_LAYER);
    let exit_bindings = auxiliary::exit(
        layouts.document(auxiliary::EXIT).unwrap(),
        &layouts,
        &mut exit,
    )
    .expect("original three-button exit dialog");
    commands.spawn((
        EditorAuxiliary::Exit(exit_bindings),
        Visibility::Hidden,
        Transform::from_xyz(0., 0., 100.),
        RenderLayers::layer(SITEMAP_LAYER),
        exit,
    ));
    commands.insert_resource(EditorSpawned);
}

/// Shared PointerUi calls this in addition to actual source Button/Toggle/
/// Slider rectangles. Only the left panel and selected HUD's measured content
/// are opaque regions. The full-screen editor root is deliberately not one.
pub(crate) fn captures_background(
    view: &UiPrefabView,
    layouts: &UiLayouts,
    point: Vec2,
    canvas: Vec2,
) -> bool {
    view.key == compose::RUNTIME
        && [
            "ContentRoot/ExpansionFixtureSelecter/BaseContent",
            "ContentRoot/FixtureEditHeadUpDisplay/Content",
        ]
        .iter()
        .any(|path| hit(view, layouts, path, point, canvas))
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, point: Vec2, canvas: Vec2) -> bool {
    view.rect(layouts, path, canvas)
        .is_some_and(|rect| rect.active && rect.contains(point))
}

fn canvas_position(position: Vec2, window_size: Vec2, scale: f32) -> Vec2 {
    Vec2::new(
        position.x - window_size.x * 0.5,
        window_size.y * 0.5 - position.y,
    ) / scale
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    edit: Res<EditView>,
    stack: Res<UiLayerStack>,
    mut ui: ResMut<EditorUiState>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut actions: MessageWriter<EditCommand>,
    mut sounds: ResMut<SeRequests>,
    roots: Query<(&EditorRoot, &UiPrefabView)>,
    auxiliaries: Query<(&EditorAuxiliary, &UiPrefabView)>,
) {
    let events: Vec<_> = gestures.read().copied().collect();
    let scroll_events: Vec<_> = wheel.read().copied().collect();
    if !edit.active || stack.current() != LayerId::MysekaiSiteEdit || consumed.0 {
        ui.drag = None;
        return;
    }
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. {
        return;
    }
    let scale = canvas_scale(size.x, size.y);
    let canvas = size / scale;
    let Ok((root, view)) = roots.single() else {
        return;
    };
    let bindings = &root.bindings;
    if edit.exit_dialog {
        ui.drag = None;
        for event in events
            .iter()
            .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        {
            consumed.0 = true;
            let point = canvas_position(event.position, size, scale);
            let Some((EditorAuxiliary::Exit(dialog), dialog_view)) = auxiliaries
                .iter()
                .find(|(auxiliary, _)| matches!(auxiliary, EditorAuxiliary::Exit(_)))
            else {
                continue;
            };
            let choice = if hit(dialog_view, &layouts, &dialog.cancel, point, canvas) {
                Some((&dialog.cancel, EditCommand::KeepEditing))
            } else if hit(dialog_view, &layouts, &dialog.close, point, canvas) {
                Some((&dialog.close, EditCommand::KeepEditing))
            } else if hit(dialog_view, &layouts, &dialog.discard, point, canvas) {
                Some((&dialog.discard, EditCommand::DiscardAndExit))
            } else if edit.can_save && hit(dialog_view, &layouts, &dialog.save, point, canvas) {
                Some((&dialog.save, EditCommand::SaveAndExit))
            } else {
                None
            };
            if let Some((button, command)) = choice {
                sounds.source_button(&layouts, dialog_view.key, button);
                actions.write(command);
                break;
            }
        }
        return;
    }
    if !ui.hidden
        && window.cursor_position().is_some_and(|position| {
            hit(
                view,
                &layouts,
                &bindings.viewport,
                canvas_position(position, size, scale),
                canvas,
            )
        })
    {
        for event in scroll_events
            .iter()
            .filter(|event| event.window == window_entity)
        {
            let delta = match event.unit {
                MouseScrollUnit::Line => event.y,
                MouseScrollUnit::Pixel => event.y / scale,
            };
            ui.scroll -= delta * bindings.scroll_sensitivity;
        }
    }
    for event in events {
        let point = canvas_position(event.position, size, scale);
        if !event.ui_owned {
            // Source OnTryShowSelector may reveal an unselected collapsed list
            // without claiming the scene tap. Scene selection still uses UID pick.
            if edit.selected.is_none()
                && event.kind.is_tap_family()
                && event.state == GestureState::End
            {
                ui.motion.target(bindings.panel_width);
            }
            continue;
        }
        consumed.0 = true;
        if event.state == GestureState::Began {
            if event.kind != GestureKind::Drag || ui.drag.is_none() {
                ui.drag =
                    if !ui.hidden && hit(view, &layouts, &bindings.panel_handle, point, canvas) {
                        Some(DragTarget::Selector)
                    } else if !ui.hidden && hit(view, &layouts, &bindings.viewport, point, canvas) {
                        Some(DragTarget::List)
                    } else {
                        None
                    };
            }
            continue;
        }
        if event.kind == GestureKind::Drag {
            if event.state == GestureState::Moved {
                match ui.drag {
                    Some(DragTarget::List) => ui.scroll -= event.delta.y / scale,
                    Some(DragTarget::Selector) => {
                        let target = (ui.motion.position + event.delta.x / scale)
                            .clamp(bindings.panel_hide, bindings.panel_max_width);
                        ui.motion.reset(target);
                    }
                    None => {}
                }
            } else if event.state == GestureState::End {
                ui.drag = None;
            }
            continue;
        }
        if !event.kind.is_tap_family() || event.state != GestureState::End {
            continue;
        }
        ui.drag = None;
        if ui.hidden {
            if hit(view, &layouts, &bindings.show_ui, point, canvas) {
                sounds.source_button(&layouts, view.key, &bindings.show_ui);
                ui.hidden = false;
                if edit.selected.is_none() {
                    ui.motion.target(bindings.panel_width);
                }
            }
            continue;
        }
        let back = auxiliaries
            .iter()
            .find_map(|(auxiliary, header)| match auxiliary {
                EditorAuxiliary::Header { back } if hit(header, &layouts, back, point, canvas) => {
                    Some((header, back))
                }
                _ => None,
            });
        if let Some((header, back)) = back {
            sounds.source_button(&layouts, header.key, back);
            actions.write(EditCommand::RequestExit);
            break;
        }
        if hit(view, &layouts, &bindings.hide_ui, point, canvas) {
            sounds.source_button(&layouts, view.key, &bindings.hide_ui);
            ui.hidden = true;
            continue;
        }
        if hit(view, &layouts, &bindings.panel_handle, point, canvas) {
            let target = if ui.motion.position <= bindings.panel_width - 1. {
                bindings.panel_width
            } else {
                bindings.panel_hide
            };
            ui.motion.target(target);
            continue;
        }
        let choice = if edit.can_save && hit(view, &layouts, &bindings.save, point, canvas) {
            // CN 6.0.0: OnReceiveSaveLayout -> SaveLayoutWithValidation ->
            // GetSaveLayoutEventData(..., true). The successful save returns
            // from the editor; the reducer retains the draft/UI on failure.
            Some((&bindings.save, EditCommand::SaveAndExit))
        } else if let Some(selected) = edit.selected.as_ref() {
            if hit(view, &layouts, &bindings.cancel, point, canvas) {
                Some((&bindings.cancel, EditCommand::Cancel))
            } else if !selected.from_inventory
                && !selected.is_new_mock
                && hit(view, &layouts, &bindings.delete, point, canvas)
            {
                Some((&bindings.delete, EditCommand::ReturnToInventory))
            } else if hit(view, &layouts, &bindings.rotate, point, canvas) {
                Some((&bindings.rotate, EditCommand::Rotate))
            } else if selected.put_status == PutStatus::Ok
                && hit(view, &layouts, &bindings.decide, point, canvas)
            {
                Some((&bindings.decide, EditCommand::Decide))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((button, command)) = choice {
            sounds.source_button(&layouts, view.key, button);
            actions.write(command);
            break;
        }
        if let Some(cell) = bindings
            .cells
            .iter()
            .find(|cell| cell.choice.editable && hit(view, &layouts, &cell.button, point, canvas))
        {
            sounds.source_button(&layouts, view.key, &cell.button);
            actions.write(cell.choice.command());
            break;
        }
    }
}

fn source_world_position(selection: &EditSelectionView) -> Option<Vec3> {
    let item = &selection.item;
    let pose = EditableFixture {
        uid: item.uid.clone(),
        package: item.package,
        fixture_id: item.fixture_id,
        center: item.center,
        grid_size: item.grid_size,
        layout: item.layout,
        direction: item.direction,
    }
    .pose()
    .ok()?;
    // UpdateEditActionSelectorEventData.CreateFixtureOffset.
    // This host implements floor/rug (SettableLayoutType != wall=0), using
    // the original unrotated GridSizeOrigin, not a guessed model AABB height.
    let grid = item.grid_size;
    let offset = (((grid.z as f32 * 0.9 + (grid.x as f32 * 0.9 + grid.y as f32 * 1.5)) / 3.)
        * 0.38)
        .clamp(0.75, 1.55);
    Some(pose.translation + Vec3::Y * offset)
}

fn place_hud(
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    bindings: &compose::Bindings,
    selection: &EditSelectionView,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    window_size: Vec2,
    scale: f32,
) {
    let Some(world_position) = source_world_position(selection) else {
        return;
    };
    let Ok(screen) = camera.world_to_viewport(camera_transform, world_position) else {
        view.set_visible(&bindings.hud, false);
        return;
    };
    let canvas = window_size / scale;
    let Some(content) = view.rect(layouts, &bindings.hud_content, canvas) else {
        return;
    };
    let point = canvas_position(screen, window_size, scale);
    // FixtureEditHeadUpDisplay.UpdatePosition: the measured original
    // content half-size plus screenOffsetScale defines the four screen limits.
    // Screen units are converted to this host's existing shared canvas units.
    let limit =
        (canvas * (0.5 - bindings.screen_offset_scale) - content.size * 0.5).max(Vec2::ZERO);
    view.set_anchored_position(&bindings.hud, point.clamp(-limit, limit));
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn refresh(
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    icons: Res<EditorIcons>,
    edit: Res<EditView>,
    stack: Res<UiLayerStack>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut ui: ResMut<EditorUiState>,
    mut roots: Query<
        (
            &mut EditorRoot,
            &mut UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<EditorAuxiliary>,
    >,
    mut auxiliaries: Query<
        (
            &EditorAuxiliary,
            &mut UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<EditorRoot>,
    >,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. {
        return;
    }
    let scale = canvas_scale(size.x, size.y);
    let canvas = size / scale;
    let active = edit.active && stack.current() == LayerId::MysekaiSiteEdit;
    let Ok((mut root, mut view, mut visibility, mut transform)) = roots.single_mut() else {
        return;
    };
    *visibility = if active {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    transform.scale = Vec3::splat(scale);
    if active {
        if root.seen_revision != edit.revision {
            let next_choices = choices(&edit, &icons);
            if root.site_id != edit.site_id || root.choices != next_choices {
                let (document, bindings) = compose::compose(&layouts, &edit, next_choices.clone())
                    .unwrap_or_else(|error| panic!("source editor list update: {error}"));
                layouts
                    .replace_runtime_document(compose::RUNTIME, document, &server)
                    .expect("editor list source installation");
                view.clear_overrides();
                root.bindings = bindings;
                root.choices = next_choices;
                root.site_id = edit.site_id;
                compose::apply_static(
                    &mut view,
                    layouts.document(compose::RUNTIME).unwrap(),
                    &root.bindings,
                    &icons,
                );
            }
            let missing: Vec<_> = edit
                .placed_rows
                .iter()
                .chain(&edit.inventory)
                .filter(|row| !icons.by_fixture.contains_key(&row.fixture_id))
                .map(|row| row.fixture_id)
                .collect();
            if !missing.is_empty() {
                warn!(
                    "editor thumbnail provider lacks fixture IDs {missing:?}; not manufacturing list images"
                );
            }
            root.seen_revision = edit.revision;
        }
        let bindings = &root.bindings;
        if !ui.was_active {
            ui.hidden = false;
            ui.scroll = 0.;
            ui.drag = None;
            ui.motion.reset(bindings.panel_width);
        }
        let selected_uid = edit
            .selected
            .as_ref()
            .map(|selected| selected.item.uid.clone());
        if ui.selected_uid != selected_uid {
            ui.motion.target(if selected_uid.is_some() {
                bindings.panel_hide
            } else {
                bindings.panel_width
            });
            ui.selected_uid = selected_uid;
        }
        ui.motion.advance(time.delta_secs());
        // Source SetPositionX. The right edge is the controller's
        // value; below base width the unchanged list translates offscreen.
        let position = ui.motion.position;
        let width = position
            .max(bindings.panel_width)
            .min(bindings.panel_max_width);
        let x = if position <= bindings.panel_width {
            position - bindings.panel_width
        } else {
            0.
        };
        view.set_size_delta(
            &bindings.panel,
            Vec2::new(width, bindings.panel_height_delta),
        );
        view.set_anchored_position(&bindings.panel, Vec2::new(x, bindings.panel_y));
        view.set_visible("ContentRoot", !ui.hidden);
        view.set_visible(&bindings.show_ui, ui.hidden);
        let document = layouts.document(compose::RUNTIME).unwrap();
        compose::enabled(&mut view, document, &bindings.save, edit.can_save);
        view.set_visible(
            &bindings.hud,
            edit.selected.is_some() && !ui.hidden && !edit.exit_dialog,
        );
        for cell in &bindings.cells {
            view.set_visible(&cell.selected, cell.choice.selected(edit.selected.as_ref()));
        }
        if let Some(selected) = &edit.selected {
            view.set_visible(
                &bindings.delete,
                !selected.from_inventory && !selected.is_new_mock,
            );
            compose::enabled(&mut view, document, &bindings.cancel, true);
            compose::enabled(
                &mut view,
                document,
                &bindings.rotate,
                selected.item.editable,
            );
            compose::enabled(
                &mut view,
                document,
                &bindings.decide,
                selected.put_status == PutStatus::Ok,
            );
            if let Ok((camera, camera_transform)) = cameras.single() {
                place_hud(
                    &mut view,
                    &layouts,
                    bindings,
                    selected,
                    camera,
                    camera_transform,
                    size,
                    scale,
                );
            }
        }
        compose::layout_cells(bindings, &mut view, &layouts, canvas, &mut ui.scroll);
    } else {
        ui.drag = None;
        ui.selected_uid = None;
    }
    ui.was_active = active;
    for (auxiliary, mut view, mut visibility, mut transform) in &mut auxiliaries {
        let visible = match auxiliary {
            EditorAuxiliary::Header { .. } => active && !ui.hidden && !edit.exit_dialog,
            EditorAuxiliary::Exit(dialog) => {
                if let Some(document) = layouts.document(auxiliary::EXIT) {
                    compose::enabled(&mut view, document, &dialog.save, edit.can_save);
                }
                active && edit.exit_dialog
            }
        };
        *visibility = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::splat(scale);
    }
}
