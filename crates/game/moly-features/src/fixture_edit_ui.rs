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
    balloon::BALLOON_LAYER,
    fixture::EditableFixture,
    fixture_edit::{EditCommand, EditContentType, EditSelectionView, EditView, PutStatus},
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
pub use icons::{EditorIcons, load, parse};

/// Included with every pristine template before the existing glyph-atlas bake:
/// the cells' counts (`UIPartsThumbnail.SetQuantity`, format `×{0}`).
pub const FIXED_TEXTS: &[&str] = &["×0123456789"];

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChoiceOrigin {
    Placed(String),
    Inventory(String),
    /// A row of the owned fixture list (`EditView::catalog`).
    Owned(usize),
}

/// Identity/data-set signature only: moving a selected item must not clone its
/// whole list again or reset pointer/scroll ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ItemChoice {
    fixture_id: i32,
    texture_id: u32,
    /// The cell's count: an owned row's placeable count, 1 for a single
    /// placed or stored fixture.
    count: i32,
    origin: ChoiceOrigin,
    editable: bool,
}

impl ItemChoice {
    /// The registered thumbnail of this fixture colour.
    fn icon<'a>(&self, icons: &'a EditorIcons) -> Option<&'a str> {
        i32::try_from(self.texture_id)
            .ok()
            .and_then(|texture| icons.variant(self.fixture_id, texture))
    }

    fn is_placed(&self) -> bool {
        matches!(self.origin, ChoiceOrigin::Placed(_))
    }

    fn command(&self) -> EditCommand {
        match &self.origin {
            ChoiceOrigin::Placed(uid) => EditCommand::SelectPlaced { uid: uid.clone() },
            ChoiceOrigin::Inventory(uid) => EditCommand::SelectInventory { uid: uid.clone() },
            ChoiceOrigin::Owned(index) => EditCommand::SelectCatalog { index: *index },
        }
    }

    fn selected(&self, selected: Option<&EditSelectionView>) -> bool {
        let Some(selected) = selected else {
            return false;
        };
        match &self.origin {
            ChoiceOrigin::Placed(uid) | ChoiceOrigin::Inventory(uid) => selected.item.uid == *uid,
            ChoiceOrigin::Owned(_) => {
                (selected.is_new || selected.from_inventory)
                    && selected.item.fixture_id == self.fixture_id
                    && selected.item.texture_id == self.texture_id
            }
        }
    }
}

fn choices(edit: &EditView, icons: &EditorIcons) -> Vec<ItemChoice> {
    if edit.content == crate::fixture_edit::EditContentType::Environment {
        return Vec::new();
    }
    let mut result = Vec::new();
    // A stored fixture of a listed owned row is counted in that row.
    let owned_row = |row: &crate::fixture_edit::EditItemView| {
        edit.catalog
            .iter()
            .any(|owned| owned.fixture_id == row.fixture_id && owned.texture_id == row.texture_id)
    };
    for (rows, inventory) in [(&edit.placed_rows, false), (&edit.inventory, true)] {
        for row in rows {
            let is_road = row.layout == moly_law::fixture::position::layout_type::ROAD;
            if is_road != (edit.content == EditContentType::Road) {
                continue;
            }
            if inventory && owned_row(row) {
                continue;
            }
            let choice = ItemChoice {
                fixture_id: row.fixture_id,
                texture_id: row.texture_id,
                count: 1,
                origin: if inventory {
                    ChoiceOrigin::Inventory(row.uid.clone())
                } else {
                    ChoiceOrigin::Placed(row.uid.clone())
                },
                editable: row.editable || edit.content == EditContentType::Road,
            };
            // An unknown image is an explicit source gap, never a generated icon.
            if choice.icon(icons).is_some() {
                result.push(choice);
            }
        }
    }
    for row in &edit.catalog {
        let choice = ItemChoice {
            fixture_id: row.fixture_id,
            texture_id: row.texture_id,
            count: row.placeable,
            origin: ChoiceOrigin::Owned(row.index),
            editable: true,
        };
        if choice.icon(icons).is_some() {
            result.push(choice);
        }
    }
    result
}

#[derive(Component)]
pub struct EditorRoot {
    bindings: compose::Bindings,
    choices: Vec<ItemChoice>,
    environment_ready: bool,
    content: crate::fixture_edit::EditContentType,
    environment_choices: Vec<crate::fixture_edit::EditEnvironmentView>,
    site_id: u32,
    seen_revision: u64,
    seen_owned_revision: u64,
}

#[derive(Component)]
pub enum EditorAuxiliary {
    Header { back: String },
    Exit(auxiliary::ExitBindings),
    CleanUp(auxiliary::CleanUpBindings),
}

#[derive(Resource)]
pub struct EditorSpawned;

#[derive(Clone, Copy)]
enum DragTarget {
    Selector,
    List,
    Scrollbar { start: Vec2, scroll: f32 },
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
pub struct EditorUiState {
    hidden: bool,
    scroll: f32,
    drag: Option<DragTarget>,
    motion: SelectorMotion,
    was_active: bool,
    selected_uid: Option<String>,
    rectangle_open: bool,
    rectangle_elapsed: f32,
}

pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    icons: Res<EditorIcons>,
    edit: Res<EditView>,
    spawned: Option<Res<EditorSpawned>>,
) {
    if spawned.is_some()
        || !icons.is_ready()
        || [
            "EditorSource",
            "EditorFloor",
            "EditorOutdoor",
            "EditorRoad",
            "EditorCell",
            "EditorEnvironment",
            "EditorTab",
            auxiliary::HEADER,
            auxiliary::EXIT,
            auxiliary::CLEAN_UP,
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
            environment_ready: false,
            content: edit.content,
            environment_choices: edit.environment_choices.clone(),
            site_id: edit.site_id,
            seen_revision: u64::MAX,
            seen_owned_revision: u64::MAX,
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
    let mut clean_up = UiPrefabView::new(auxiliary::CLEAN_UP, SITEMAP_LAYER);
    let clean_up_bindings = auxiliary::clean_up(
        layouts.document(auxiliary::CLEAN_UP).unwrap(),
        &layouts,
        &mut clean_up,
    )
    .expect("original two-button clean-up confirmation");
    commands.spawn((
        EditorAuxiliary::CleanUp(clean_up_bindings),
        Visibility::Hidden,
        Transform::from_xyz(0., 0., 100.),
        RenderLayers::layer(SITEMAP_LAYER),
        clean_up,
    ));
    commands.insert_resource(EditorSpawned);
}

/// Shared PointerUi calls this in addition to actual source Button/Toggle/
/// Slider rectangles. Only the left panel and selected HUD's measured content
/// are opaque regions. The full-screen editor root is deliberately not one.
pub use crate::ui_inputs::captures_background;

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
pub fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    edit: Res<EditView>,
    stack: Res<UiLayerStack>,
    mut ui: ResMut<EditorUiState>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut actions: MessageWriter<EditCommand>,
    mut library: ResMut<crate::content_library::ContentLibrary>,
    mut sounds: ResMut<SeRequests>,
    roots: Query<(&EditorRoot, &UiPrefabView)>,
    auxiliaries: Query<(&EditorAuxiliary, &UiPrefabView)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let events: Vec<_> = gestures.read().copied().collect();
    let scroll_events: Vec<_> = wheel.read().copied().collect();
    // A save dialog (common dialog) takes the taps while it is shown.
    if !edit.active || stack.current() != LayerId::MysekaiSiteEdit || consumed.0 || edit.save_dialog
    {
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
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
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
                layouts.queue_button_se(&mut sounds, dialog_view.key, button);
                actions.write(command);
                break;
            }
        }
        return;
    }
    if edit.clean_up_dialog {
        ui.drag = None;
        for event in events
            .iter()
            .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        {
            consumed.0 = true;
            let point = canvas_position(event.position, size, scale);
            let Some((EditorAuxiliary::CleanUp(dialog), dialog_view)) = auxiliaries
                .iter()
                .find(|(auxiliary, _)| matches!(auxiliary, EditorAuxiliary::CleanUp(_)))
            else {
                continue;
            };
            let choice = if hit(dialog_view, &layouts, &dialog.positive, point, canvas) {
                Some((Some(&dialog.positive), EditCommand::CleanUpAll))
            } else if hit(dialog_view, &layouts, &dialog.negative, point, canvas) {
                Some((Some(&dialog.negative), EditCommand::CancelCleanUp))
            } else if hit(dialog_view, &layouts, &dialog.close, point, canvas) {
                Some((Some(&dialog.close), EditCommand::CancelCleanUp))
            } else if !hit(dialog_view, &layouts, &dialog.window, point, canvas) {
                // allowCloseExternal: a tap outside the window closes it.
                Some((None, EditCommand::CancelCleanUp))
            } else {
                None
            };
            if let Some((button, command)) = choice {
                if let Some(button) = button {
                    layouts.queue_button_se(&mut sounds, dialog_view.key, button);
                }
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
                    } else if !ui.hidden
                        && bindings
                            .scrollbar
                            .as_ref()
                            .is_some_and(|bar| hit(view, &layouts, &bar.handle, point, canvas))
                    {
                        Some(DragTarget::Scrollbar {
                            start: point,
                            scroll: ui.scroll,
                        })
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
                    Some(DragTarget::Scrollbar { start, scroll }) => {
                        if let Some(next) = compose::drag_scrollbar(
                            bindings, view, &layouts, canvas, start, point, scroll,
                        ) {
                            ui.scroll = next;
                        }
                    }
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
                layouts.queue_button_se(&mut sounds, view.key, &bindings.show_ui);
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
            layouts.queue_button_se(&mut sounds, header.key, back);
            actions.write(EditCommand::RequestExit);
            break;
        }
        if hit(view, &layouts, &bindings.hide_ui, point, canvas) {
            layouts.queue_button_se(&mut sounds, view.key, &bindings.hide_ui);
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
        if let Some(tab) = bindings.tabs.iter().find(|tab| {
            (tab.primary || tab.kind == 1 || (tab.kind == 19 && root.environment_ready))
                && hit(view, &layouts, &tab.button, point, canvas)
        }) {
            if let Some(content) = crate::fixture_edit::EditContentType::from_tab_kind(tab.kind) {
                layouts.queue_button_se(&mut sounds, view.key, &tab.button);
                actions.write(EditCommand::SelectContent(content));
                ui.scroll = 0.;
                break;
            }
        }
        if let Some(cell) = bindings
            .environment_cells
            .iter()
            .find(|cell| hit(view, &layouts, &cell.button, point, canvas))
        {
            layouts.queue_button_se(&mut sounds, view.key, &cell.button);
            actions.write(EditCommand::SelectEnvironment {
                phenomena_id: cell.phenomena_id,
            });
            break;
        }
        if edit.content == EditContentType::Road
            && hit(
                view,
                &layouts,
                &bindings.fixture_visibility_toggle,
                point,
                canvas,
            )
        {
            actions.write(EditCommand::RoadToggleFixtures);
            consumed.0 = true;
            continue;
        }
        if edit.content == EditContentType::Road
            && hit(view, &layouts, &bindings.road_switch, point, canvas)
        {
            actions.write(if edit.road.select_mode {
                EditCommand::RoadPlanningMode
            } else {
                EditCommand::RoadSelectMode
            });
            break;
        }
        if edit.content == EditContentType::Road && !edit.road.planned.is_empty() {
            let command = if edit.road.can_cancel
                && hit(view, &layouts, &bindings.cancel, point, canvas)
            {
                Some(EditCommand::Cancel)
            } else if edit.road.can_decide && hit(view, &layouts, &bindings.decide, point, canvas) {
                Some(EditCommand::Decide)
            } else if edit.road.moving && hit(view, &layouts, &bindings.delete, point, canvas) {
                Some(EditCommand::ReturnToInventory)
            } else {
                None
            };
            if let Some(command) = command {
                actions.write(command);
                break;
            }
        }
        if let Some(selected) = edit.selected.as_ref() {
            if hit(view, &layouts, &bindings.info, point, canvas)
                && library.inspect_fixture(selected.item.fixture_id)
            {
                layouts.queue_button_se(&mut sounds, view.key, &bindings.info);
                break;
            }
        }
        // The floor edit camera's two buttons answer with or without a
        // selection.
        let camera = if hit(view, &layouts, &bindings.camera_rotate, point, canvas) {
            Some((&bindings.camera_rotate, EditCommand::RotateCamera))
        } else if hit(view, &layouts, &bindings.change_look, point, canvas) {
            Some((&bindings.change_look, EditCommand::ChangeLookCamera))
        } else {
            None
        };
        if let Some((button, command)) = camera {
            layouts.queue_button_se(&mut sounds, view.key, button);
            actions.write(command);
            break;
        }
        if edit.content != crate::fixture_edit::EditContentType::Environment
            && hit(view, &layouts, &bindings.remove_all, point, canvas)
        {
            layouts.queue_button_se(&mut sounds, view.key, &bindings.remove_all);
            actions.write(EditCommand::RequestCleanUp);
            break;
        }
        let choice = if edit.can_save && hit(view, &layouts, &bindings.save, point, canvas) {
            // CN client: OnReceiveSaveLayout -> SaveLayoutWithValidation ->
            // GetSaveLayoutEventData(..., true). The successful save returns
            // from the editor; the reducer retains the draft/UI on failure.
            Some((&bindings.save, EditCommand::SaveAndExit))
        } else if let Some(selected) = edit.selected.as_ref() {
            if hit(view, &layouts, &bindings.cancel, point, canvas) {
                Some((&bindings.cancel, EditCommand::Cancel))
            } else if !selected.from_inventory
                && !selected.is_new
                && selected.can_clean_up
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
            layouts.queue_button_se(&mut sounds, view.key, button);
            actions.write(command);
            break;
        }
        if let Some(cell) = bindings
            .cells
            .iter()
            .find(|cell| cell.choice.editable && hit(view, &layouts, &cell.button, point, canvas))
        {
            layouts.queue_button_se(&mut sounds, view.key, &cell.button);
            actions.write(cell.choice.command());
            break;
        }
    }
}

fn source_world_position(selection: &EditSelectionView) -> Option<Vec3> {
    let item = &selection.item;
    let pose = EditableFixture {
        texture_id: item.texture_id,
        uid: item.uid.clone(),
        package: item.package.clone(),
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
pub fn refresh(
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    icons: Res<EditorIcons>,
    edit: Res<EditView>,
    stack: Res<UiLayerStack>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    origins: Query<&GlobalTransform, With<crate::fixture_scene_inputs::SiteCoordinateOrigin>>,
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
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. {
        return;
    }
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let active = edit.active && stack.current() == LayerId::MysekaiSiteEdit;
    let Ok((mut root, mut view, mut visibility, mut transform)) = roots.single_mut() else {
        return;
    };
    visibility.set_if_neq(if active {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    crate::canvas::set_scale(&mut transform, scale);
    if active {
        let environment_ready =
            icons.environment(None).is_some() && layouts.ready("EditorEnvironmentCell", &server);
        if root.environment_ready != environment_ready
            || root.seen_revision != edit.revision
            || root.seen_owned_revision != edit.owned_revision
            || root.environment_choices != edit.environment_choices
        {
            let next_choices = choices(&edit, &icons);
            if root.site_id != edit.site_id
                || root.choices != next_choices
                || root.content != edit.content
                || root.environment_choices != edit.environment_choices
                || root.environment_ready != environment_ready
            {
                let (document, bindings) = compose::compose(&layouts, &edit, next_choices.clone())
                    .unwrap_or_else(|error| panic!("source editor list update: {error}"));
                layouts
                    .replace_runtime_document(compose::RUNTIME, document, &server)
                    .expect("editor list source installation");
                view.clear_overrides();
                root.bindings = bindings;
                root.choices = next_choices;
                root.site_id = edit.site_id;
                root.environment_ready = environment_ready;
                root.content = edit.content;
                root.environment_choices = edit.environment_choices.clone();
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
                .map(|row| (row.fixture_id, row.texture_id))
                .chain(
                    edit.catalog
                        .iter()
                        .map(|row| (row.fixture_id, row.texture_id)),
                )
                .filter(|(fixture, texture)| {
                    i32::try_from(*texture)
                        .map_or(true, |texture| icons.variant(*fixture, texture).is_none())
                })
                .collect();
            if !missing.is_empty() {
                warn!(
                    "editor thumbnail provider lacks (fixture id, texture id) {missing:?}; not manufacturing list images"
                );
            }
            root.seen_revision = edit.revision;
            root.seen_owned_revision = edit.owned_revision;
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
            .map(|selected| selected.item.uid.clone())
            .or_else(|| (!edit.road.planned.is_empty()).then(|| "road-plan".to_owned()));
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
        let environment = edit.content == crate::fixture_edit::EditContentType::Environment;
        view.set_visible(&bindings.environment_content, environment);
        let name = edit
            .environment_choices
            .iter()
            .find(|row| row.phenomena_id == edit.environment)
            .map(|row| row.name.as_str())
            .unwrap_or("");
        view.set_text(&bindings.environment_text, name.to_owned());
        compose::enabled(&mut view, document, &bindings.remove_all, !environment);
        for tab in &bindings.tabs {
            if tab.kind == 19 {
                compose::enabled(&mut view, document, &tab.button, environment_ready);
                view.set_texture(
                    &tab.image,
                    if environment_ready {
                        "editor-tab-19-normal"
                    } else {
                        "editor-tab-19-disabled"
                    },
                );
            }
        }
        for cell in &bindings.environment_cells {
            let icon = icons.environment(cell.phenomena_id);
            if let Some(icon) = icon {
                view.set_texture(&cell.image, icon);
            }
            view.set_visible(&cell.image, icon.is_some());
            compose::enabled(&mut view, document, &cell.button, icon.is_some());
            view.set_visible(&cell.selected, cell.phenomena_id == edit.environment);
        }
        compose::enabled(&mut view, document, &bindings.save, edit.can_save);
        compose::enabled(&mut view, document, &bindings.info, edit.selected.is_some());
        view.set_visible(
            &bindings.hud,
            edit.selected.is_some() && !ui.hidden && !edit.exit_dialog,
        );
        for cell in &bindings.cells {
            let selected_road = if edit.content == EditContentType::Road {
                match &cell.choice.origin {
                    ChoiceOrigin::Placed(uid) => {
                        edit.road.planned.iter().any(|row| row.uid == *uid)
                    }
                    _ => edit.road.target.as_ref().is_some_and(|target| {
                        target.fixture_id == cell.choice.fixture_id
                            && target.texture_id == cell.choice.texture_id
                    }),
                }
            } else {
                false
            };
            view.set_visible(
                &cell.selected,
                selected_road || cell.choice.selected(edit.selected.as_ref()),
            );
        }
        if let Some(selected) = &edit.selected {
            view.set_visible(
                &bindings.delete,
                !selected.from_inventory && !selected.is_new && selected.can_clean_up,
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
        view.set_visible(
            &bindings.right_bottom,
            edit.content == EditContentType::Road && !ui.hidden,
        );
        view.set_visible(
            &bindings.fixture_visibility_toggle,
            edit.content == EditContentType::Road && !ui.hidden,
        );
        view.set_visible(&bindings.fixture_visibility_on, edit.road.show_fixtures);
        view.set_visible(&bindings.fixture_visibility_off, !edit.road.show_fixtures);
        let target = edit
            .road
            .target
            .as_ref()
            .filter(|_| edit.content == EditContentType::Road && !ui.hidden);
        view.set_visible(&bindings.target_icon, target.is_some());
        view.set_visible(&bindings.target_thumbnail, target.is_some());
        if let Some(target) = target {
            if let Ok(texture) = i32::try_from(target.texture_id) {
                if let Some(icon) = icons.variant(target.fixture_id, texture) {
                    view.set_texture(&bindings.target_image, icon);
                }
            }
            view.set_text(
                &bindings.target_quantity,
                format!("×{}", edit.road.remaining),
            );
        }
        let site_origin = origins
            .single()
            .map_or(Vec3::ZERO, GlobalTransform::translation);
        let markers = edit
            .road
            .markers
            .filter(|_| edit.content == EditContentType::Road && !ui.hidden);
        view.set_visible(&bindings.road_start_marker, markers.is_some());
        view.set_visible(
            &bindings.road_end_marker,
            markers.is_some() && !edit.road.dragging,
        );
        if let (Some(_), Ok((camera, camera_transform))) = (markers, cameras.single()) {
            for (row, selector) in [
                (edit.road.planned.first(), &bindings.road_start_marker),
                (edit.road.planned.last(), &bindings.road_end_marker),
            ] {
                let Some(row) = row else {
                    continue;
                };
                let item = EditableFixture {
                    uid: row.uid.clone(),
                    package: row.package.clone(),
                    texture_id: row.texture_id,
                    fixture_id: row.fixture_id,
                    center: row.center,
                    direction: row.direction,
                    grid_size: row.grid_size,
                    layout: row.layout,
                };
                if let Ok(pose) = item.pose() {
                    if let Ok(screen) =
                        camera.world_to_viewport(camera_transform, site_origin + pose.translation)
                    {
                        view.set_anchored_position(selector, canvas_position(screen, size, scale));
                    }
                }
            }
        }
        view.set_visible(&bindings.road_switch_on, edit.road.select_mode);
        view.set_visible(&bindings.road_switch_off, !edit.road.select_mode);
        let road_bounds = edit
            .road
            .selection_bounds
            .filter(|_| edit.content == EditContentType::Road && !ui.hidden);
        if road_bounds.is_some() && !ui.rectangle_open {
            ui.rectangle_elapsed = 0.;
        }
        ui.rectangle_open = road_bounds.is_some();
        ui.rectangle_elapsed = (ui.rectangle_elapsed + time.delta_secs()).min(0.25);
        view.set_visible(&bindings.road_rectangle, road_bounds.is_some());
        if let (Some((start, end)), Ok((camera, camera_transform))) =
            (road_bounds, cameras.single())
        {
            let min = site_origin
                + Vec3::new(
                    -f32::from(start.x.min(end.x)) * 0.25,
                    0.,
                    f32::from(start.z.min(end.z)) * 0.25,
                );
            let max = site_origin
                + Vec3::new(
                    -(f32::from(start.x.max(end.x)) + 1.) * 0.25,
                    0.,
                    (f32::from(start.z.max(end.z)) + 1.) * 0.25,
                );
            if let (Ok(a), Ok(b)) = (
                camera.world_to_viewport(camera_transform, min),
                camera.world_to_viewport(camera_transform, max),
            ) {
                let a = canvas_position(a, size, scale);
                let b = canvas_position(b, size, scale);
                view.set_anchored_position(&bindings.road_rectangle, (a + b) * 0.5);
                view.set_size_delta(
                    &bindings.road_rectangle,
                    (b - a).abs() * (ui.rectangle_elapsed / 0.25),
                );
            }
        }
        let road_visible =
            edit.content == EditContentType::Road && !edit.road.planned.is_empty() && !ui.hidden;
        if edit.content == EditContentType::Road {
            view.set_visible(&bindings.hud, road_visible && !edit.road.dragging);
            view.set_visible(&bindings.delete, edit.road.moving);
            view.set_visible(&bindings.rotate, false);
        }
        view.set_visible(&bindings.road_count, road_visible && edit.road.show_count);
        if edit.content == EditContentType::Road {
            compose::enabled(&mut view, document, &bindings.decide, edit.road.can_decide);
            compose::enabled(&mut view, document, &bindings.cancel, edit.road.can_cancel);
        }
        if road_visible {
            let count = if edit.road.remaining > 0 {
                moly_law::text::custom_text_mesh::format_wording(
                    &layouts.wording("MSG_MYSEKAI_LAYOUT_EDIT_PLANNING_COUNT"),
                    &[edit.road.remaining.to_string()],
                )
                .expect("source road planning count wording")
            } else {
                edit.road.remaining.to_string()
            };
            view.set_text(&bindings.road_count_text, count);
            if let Some(color) = layouts.palette_color(if edit.road.remaining > 0 { 0 } else { 17 })
            {
                let graphic = bindings
                    .road_count_text
                    .trim_start_matches('@')
                    .parse()
                    .expect("source road count graphic");
                view.set_graphic_color(graphic, color);
            }
            if let (Some(last), Ok((camera, camera_transform))) =
                (edit.road.planned.last(), cameras.single())
            {
                let item = EditableFixture {
                    uid: last.uid.clone(),
                    package: last.package.clone(),
                    texture_id: last.texture_id,
                    fixture_id: last.fixture_id,
                    center: last.center,
                    direction: last.direction,
                    grid_size: last.grid_size,
                    layout: last.layout,
                };
                if let Ok(pose) = item.pose() {
                    if let Ok(screen) =
                        camera.world_to_viewport(camera_transform, site_origin + pose.translation)
                    {
                        let position = canvas_position(screen, size, scale);
                        let positions: Vec<_> = edit
                            .road
                            .planned
                            .iter()
                            .filter_map(|row| {
                                EditableFixture {
                                    uid: row.uid.clone(),
                                    package: row.package.clone(),
                                    texture_id: row.texture_id,
                                    fixture_id: row.fixture_id,
                                    center: row.center,
                                    direction: row.direction,
                                    grid_size: row.grid_size,
                                    layout: row.layout,
                                }
                                .pose()
                                .ok()
                                .map(|pose| pose.translation)
                            })
                            .collect();
                        let average =
                            positions.iter().copied().sum::<Vec3>() / positions.len() as f32;
                        if let Ok(screen) =
                            camera.world_to_viewport(camera_transform, site_origin + average)
                        {
                            let point = canvas_position(screen, size, scale);
                            if let Some(content) =
                                view.rect(&layouts, &bindings.hud_content, canvas)
                            {
                                let limit = (canvas * (0.5 - bindings.screen_offset_scale)
                                    - content.size * 0.5)
                                    .max(Vec2::ZERO);
                                view.set_anchored_position(
                                    &bindings.hud,
                                    point.clamp(-limit, limit),
                                );
                            }
                        }
                        view.set_anchored_position(&bindings.road_count, position);
                    }
                }
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
            EditorAuxiliary::CleanUp(_) => active && edit.clean_up_dialog,
        };
        visibility.set_if_neq(if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        crate::canvas::set_scale(&mut transform, scale);
    }
}
