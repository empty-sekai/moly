//! `MysekaiMissionListDialog` (dialog type 340, Common1): the sheet list the
//! mission screen's `MissionListButton` opens.
//!
//! Source structure:
//! - `ScreenLayerMysekaiMission.OnClickMissionListButton`: `CreateMissionAllList`
//!   — every sheet in `seq` order, each with its completion mission's view
//!   data and its normal missions' view data — then
//!   `Show1ButtonDialog<MysekaiMissionListDialog>(340).Setup(list)`.
//! - `MysekaiMissionListDialog.Setup`: `_missionListView.OnCreateCell` set to
//!   `OnCreateCell`, `CreateViewItem(list.Count)`, then `SetupFirstView`.
//! - `OnCreateCell`: `MysekaiMissionSheetCell.Setup` of that row's view data.
//! - `Common1ButtonDialog.OnClickOK` closes; `OnCloseExternal` (the window's
//!   close button) and the hardware back key both call it.
//!
//! Host mapping: the dialog takes the dialog slot
//! (`ScreenManager::show_dialog` for dialog type 340, `DialogBase`'s close
//! rule) and draws the root's `MissionList` document through a runtime copy of
//! its own, so the published document stays as the root carries it. The
//! caller reads the request the screen's list button wrote and pushes the
//! slot; the dialog's own view reads the pushed slot the next frame and runs
//! `Setup` there ([`setup`]), as the source runs it on the shown dialog.
//!
//! Refused by name: the list's `MysekaiMissionSheetCell` is a prefab with no
//! document in this root — [`refuse_document`] names it once — so the list
//! draws no row. `SetupFirstView` therefore places no row: the source's pick
//! of the first index needs `IsAllCompleted`, which a host would take from a
//! session state this unit does not own, and a false pick would show a wrong
//! sheet.
//!
//! Named gaps: the `Tabs` object stays as the document has it (inactive);
//! the window scale tween of `DialogBase` is not drawn (the canvas scale the
//! view's entity carries stands); the footer's close label is the document's
//! serialized text (`useWordingKey` on with an empty key).

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::UiPrefab;

use super::cell::{refuse_document, MissionDialogRequest, SHEET_CELL};
use super::screen;
use crate::action_button::ActionTapConsumed;
use crate::canvas::RootCanvas;
use crate::gesture::{GestureEvent, GestureState};
use crate::ui_inputs::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::mission::ClientMission;

/// The root's document (`dialogs/MysekaiMissionListDialog.json`).
pub const KEY: &str = "MissionList";
/// The runtime copy the dialog draws (`UiPrefabView::new` takes this same
/// key).
pub const VIEW: &str = "MissionListView";
/// `DialogType.MysekaiMissionListDialog`.
pub const DIALOG: DialogType = DialogType(340);
/// The caller the dialog slot records (`OnClickMissionListButton`).
pub const CALLER: &str = "ScreenLayerMysekaiMission.OnClickMissionListButton";
const DIALOG_Z: f32 = 100.0;

/// The dialog's nodes, on the dialog's entity.
#[derive(Debug, Clone, Component)]
pub(crate) struct Bindings {
    /// `WindowRoot/Tabs` (inactive in the document).
    pub(crate) tabs: String,
    /// `WindowRoot/UIPartsCloseButton` (`DialogBase`'s close button,
    /// `OnCloseExternal`).
    pub(crate) close_button: String,
    /// `WindowRoot/FooterButtons/UIPartsCommonButton_close` (`OnClickOK`).
    pub(crate) footer_close: String,
    /// `ContentRoot/Content/ListView`: the sheet list `_missionListView`.
    pub(crate) list: String,
    /// `ListView/Viewport/Content`.
    pub(crate) content: String,
    /// `ListView/ScrollbarVertical`: the list's vertical scrollbar object.
    pub(crate) scrollbar: String,
}

impl Bindings {
    pub(crate) fn read(doc: &UiPrefab) -> Result<Self, String> {
        let root = doc
            .nodes
            .first()
            .map(|node| node.path.clone())
            .ok_or("the document has no root")?;
        let path = |suffix: &str| -> Result<String, String> {
            let full = format!("{root}/{suffix}");
            doc.find(&full).map(|_| full)
        };
        let footer_close = path("WindowRoot/FooterButtons/UIPartsCommonButton_close")?;
        let list = path("WindowRoot/ContentRoot/Content/ListView")?;
        Ok(Self {
            tabs: path("WindowRoot/Tabs")?,
            close_button: path("WindowRoot/UIPartsCloseButton")?,
            content: format!("{list}/Viewport/Content"),
            scrollbar: format!("{list}/ScrollbarVertical"),
            footer_close,
            list,
        })
    }
}

#[derive(Component)]
pub struct MissionListDialogRoot;

/// `MysekaiMissionListViewData`, as `OnClickMissionListButton` builds it: one
/// row per sheet in `seq` order, each with the sheet's completion mission and
/// its normal missions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetRow {
    /// The sheet's master id.
    pub sheet: i32,
    /// `GetParentMission(sheet.id)`'s master id.
    pub complete: Option<i32>,
    /// `GetChildMissions(sheet.id)`, in `seq` order.
    pub children: Vec<i32>,
}

/// `CreateMissionAllList`: every sheet in `seq` order.
pub fn all_rows(masters: &moly_masters::mission::MissionMasters) -> Vec<SheetRow> {
    masters
        .sheets_by_seq()
        .into_iter()
        .map(|sheet| SheetRow {
            sheet: sheet.id,
            complete: masters.parent_mission(sheet.id).map(|mission| mission.id),
            children: masters
                .child_missions(sheet.id)
                .into_iter()
                .map(|mission| mission.id)
                .collect(),
        })
        .collect()
}

/// The dialog's state.
#[derive(Resource, Default)]
pub struct MissionListDialog {
    pub(crate) shown: Option<DialogId>,
    /// `Setup`'s list: one row per sheet.
    pub(crate) rows: Vec<SheetRow>,
    /// `SetupFirstView`'s first index; `None` when no row is placed.
    pub(crate) first: Option<usize>,
    /// Whether `Setup` ran on the dialog's own view.
    pub(crate) set_up: bool,
}

impl MissionListDialog {
    /// The dialog is up.
    pub fn shown(&self) -> bool {
        self.shown.is_some()
    }

    /// The sheet rows `Setup` was given.
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// `SetupFirstView`'s first index, once `Setup` placed one.
    pub fn first(&self) -> Option<usize> {
        self.first
    }
}

/// Update: the dialog's runtime document and view, once the root's document is
/// in.
pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    roots: Query<(), With<MissionListDialogRoot>>,
) {
    if !roots.is_empty() {
        return;
    }
    let Some(doc) = layouts.document(KEY) else {
        return;
    };
    let bindings = match Bindings::read(doc) {
        Ok(bindings) => bindings,
        Err(reason) => {
            error!("mission: {KEY} is refused: {reason}");
            return;
        }
    };
    let document = doc.clone();
    if let Err(reason) = layouts.replace_runtime_document(VIEW, document, &server) {
        error!("mission: {KEY} is not installed: {reason}");
        return;
    }
    // The list's cell prefab is not in this root: no row is instantiated.
    let mut seen = std::collections::BTreeSet::new();
    refuse_document(&mut seen, SHEET_CELL);
    let mut view = UiPrefabView::new(VIEW, SITEMAP_LAYER);
    view.set_visible(&bindings.tabs, false);
    // `SetupFirstView` places no row (see the module notes).
    view.set_visible(&bindings.content, false);
    view.set_visible(&bindings.scrollbar, false);
    commands.spawn((
        MissionListDialogRoot,
        Visibility::Hidden,
        Transform::from_xyz(0., 0., DIALOG_Z),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
        bindings,
    ));
    info!("mission: MysekaiMissionListDialog's document ({KEY}) is installed as a runtime copy");
}

/// `Show1ButtonDialog<MysekaiMissionListDialog>(340)`: the slot, `Open`, and
/// the open animation's end (`DialogBase`'s tween is not drawn).
pub fn show(screens: &mut ScreenManager) -> Result<DialogId, String> {
    let id = screens.show_dialog(
        DIALOG,
        DisplayLayerType::LayerDialog,
        DialogBackKey::Close,
        CALLER,
    )?;
    screens.open_dialog(id);
    screens.dialog_open_finished(id);
    Ok(id)
}

/// Update: the list button's request. The pushed slot is read by [`setup`]
/// next frame, and [`place`] draws it while it is up.
pub fn open(
    mut requests: MessageReader<MissionDialogRequest>,
    mut dialog: ResMut<MissionListDialog>,
    mut screens: ResMut<ScreenManager>,
    tables: Option<Res<moly_masters::load::MasterTables>>,
    roots: Query<(), With<MissionListDialogRoot>>,
    layouts: Res<UiLayouts>,
) {
    let asked = requests
        .read()
        .any(|request| *request == MissionDialogRequest::List);
    if !asked {
        return;
    }
    let Some(masters) = tables
        .as_deref()
        .and_then(|tables| tables.mission.as_ref())
    else {
        error!(
            "mission: OnClickMissionListButton before the mission masters are in: nothing"
        );
        return;
    };
    if dialog.shown.is_some() {
        error!("mission: OnClickMissionListButton while the list dialog is shown: the shown one stays");
        return;
    }
    if roots.is_empty() || layouts.document(VIEW).is_none() {
        error!("mission: {KEY} is not installed: the mission list dialog does not open");
        return;
    }
    let id = match show(&mut screens) {
        Ok(id) => id,
        Err(reason) => {
            error!("mission: {reason}: the mission list dialog does not open");
            return;
        }
    };
    dialog.shown = Some(id);
    dialog.rows = all_rows(masters);
    dialog.first = None;
    dialog.set_up = false;
    info!(
        "mission: MysekaiMissionListDialog: Show1ButtonDialog<MysekaiMissionListDialog>(340) is open; CreateMissionAllList made {} sheet row(s)",
        dialog.rows.len()
    );
}

/// Update: `Setup` on the dialog's own view, once its slot is open. The rows
/// are `MysekaiMissionSheetCell`s, whose document the root does not carry, so
/// `CreateViewItem` has no cell to place and `SetupFirstView` places none.
pub fn setup(mut dialog: ResMut<MissionListDialog>, screens: Res<ScreenManager>) {
    let Some(id) = dialog.shown else {
        return;
    };
    if dialog.set_up || !screens.dialogs().any(|(open, _, _)| open == id) {
        return;
    }
    dialog.set_up = true;
    let rows = dialog.rows.len();
    let mut seen = std::collections::BTreeSet::new();
    refuse_document(&mut seen, SHEET_CELL);
    // `CreateViewItem(list.Count)` then `SetupFirstView`: the rows are
    // `MysekaiMissionSheetCell`s the root has no document for, so neither
    // places a view; the hidden list keeps the document as it is.
    info!(
        "mission: MysekaiMissionListDialog.Setup: CreateViewItem({rows}); the rows are {SHEET_CELL}s, whose document is not in the root, so CreateViewItem places no row and SetupFirstView places none"
    );
}

/// `Common1ButtonDialog.OnClickOK` (its reply is null here) then
/// `DialogBase.Close` and the destroy.
fn close(by: &str, dialog: &mut MissionListDialog, screens: &mut ScreenManager) {
    let Some(id) = dialog.shown.take() else { return };
    info!("mission: MysekaiMissionListDialog {id:?}: {by}; DialogBase.Close, the dialog is destroyed");
    screens.close_dialog(id);
    screens.dialog_destroyed(id);
    dialog.rows.clear();
    dialog.first = None;
    dialog.set_up = false;
}

/// The shown dialog's reads: its view, the canvas and the root's layouts.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DialogWorld<'w, 's> {
    pub layouts: Res<'w, UiLayouts>,
    pub windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    pub root_canvas: Option<Res<'w, RootCanvas>>,
    pub views: Query<'w, 's, (&'static Bindings, &'static UiPrefabView), With<MissionListDialogRoot>>,
}

/// A tap is the control of `path` of the dialog's view.
fn hit(
    layouts: &UiLayouts,
    view: &UiPrefabView,
    path: &str,
    point: Vec2,
    canvas: Vec2,
) -> bool {
    view.rect(layouts, path, canvas)
        .is_some_and(|rect| rect.active && rect.contains(point))
}

/// Update: `Show1ButtonDialog(340)`'s close rule — the footer's close button
/// (`OnClickOK`) closes, the window's close button (`OnCloseExternal` ->
/// `OnClickOK`) closes, and any other tap is the dialog's (its slot is
/// modal).
pub fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut dialog: ResMut<MissionListDialog>,
    mut screens: ResMut<ScreenManager>,
    mut consumed: ResMut<ActionTapConsumed>,
    world: DialogWorld,
    mut sounds: ResMut<crate::audio::SeRequests>,
) {
    if dialog.shown.is_none() {
        gestures.clear();
        return;
    }
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    if taps.is_empty() {
        return;
    }
    let (Ok(window), Some(canvas), Ok((bindings, view))) = (
        world.windows.single(),
        world.root_canvas.as_deref(),
        world.views.single(),
    ) else {
        return;
    };
    let size = canvas.size(window);
    let scale = canvas.scale(window);
    for position in taps {
        // The dialog slot is modal: every tap is the dialog's.
        consumed.0 = true;
        let point = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale;
        if hit(&world.layouts, view, &bindings.close_button, point, size) {
            world
                .layouts
                .queue_button_se(&mut sounds, VIEW, &bindings.close_button);
            close(
                "tap on the window's close button (OnCloseExternal -> OnClickOK -> DialogBase.Close)",
                &mut dialog,
                &mut screens,
            );
            return;
        }
        if hit(&world.layouts, view, &bindings.footer_close, point, size) {
            world
                .layouts
                .queue_button_se(&mut sounds, VIEW, &bindings.footer_close);
            close(
                "tap on the footer's close button (Common1ButtonDialog.OnClickOK -> DialogBase.Close)",
                &mut dialog,
                &mut screens,
            );
            return;
        }
    }
}

/// Update: `Common1ButtonDialog.OnHardwareBackKeyProcess` -> `OnClickOK`.
pub fn back_key(
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut dialog: ResMut<MissionListDialog>,
    mut screens: ResMut<ScreenManager>,
) {
    for event in back_keys.read() {
        if dialog.shown == Some(event.id) {
            close(
                "hardware back key (OnHardwareBackKeyProcess -> OnClickOK -> DialogBase.Close)",
                &mut dialog,
                &mut screens,
            );
        }
    }
}

/// Update: the dialog's entity visibility, canvas scale and drawn root while
/// it is up.
pub fn place(
    dialog: Res<MissionListDialog>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    mut roots: Query<
        (&Bindings, &mut Visibility, &mut Transform, &mut UiPrefabView),
        With<MissionListDialogRoot>,
    >,
) {
    let Ok((bindings, mut visibility, mut transform, mut view)) = roots.single_mut() else {
        return;
    };
    if let (Ok(window), Some(canvas)) = (windows.single(), root_canvas.as_deref()) {
        crate::canvas::set_scale(&mut transform, canvas.scale(window));
    }
    // The document root is the window's own node: an override hiding it
    // would hide the window, so the entity's visibility alone hides the
    // dialog.
    visibility.set_if_neq(if dialog.shown.is_none() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
    view.set_visible(&bindings.tabs, false);
}

/// `SetupFirstView`'s pick: `IsAllCompleted` puts the last row first,
/// otherwise the row of the sheet the user is on, otherwise the first row.
pub fn first_row(
    masters: &moly_masters::mission::MissionMasters,
    user: &ClientMission,
    sheets: usize,
) -> usize {
    let mut state = user.state();
    state.current_sheet = screen::current_sheet(masters, user);
    if super::model::is_all_completed(masters, &state) {
        return sheets.saturating_sub(1);
    }
    masters
        .sheets_by_seq()
        .iter()
        .position(|sheet| sheet.id == state.current_sheet)
        .unwrap_or(0)
}
