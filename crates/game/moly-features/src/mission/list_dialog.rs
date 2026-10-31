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
//! Refused by name: a `MysekaiMissionSheetCell` document that is missing from
//! the root or fails to parse, carry the cell prefab, or lack one of the nodes
//! a row writes. A refusal leaves the list hidden, so every fail-closed path
//! names itself in the log once and draws no row.
//!
//! The sheet rows: the list's `listViewItemPrefab` is the
//! `MysekaiMissionSheetCell` prefab, which the UI root carries as a document
//! of its own. The document is not one of the layouts the loader requests, so
//! this unit reads it from the root the runtime draws from and admits it
//! through that root's manifest ([`load_cell`], shared with the detail
//! dialog); `Setup` waits for it and runs once it is in. Every `Setup`
//! installs a runtime copy of the dialog's document with one
//! `MysekaiMissionSheetCell` copy per sheet row, in sheet `seq` order, under
//! `ListView/Viewport/Content` (a replaced document drops the subtrees copied
//! into it, so the copies are made with it). The rows are written as
//! `MysekaiMissionSheetCell.Setup` writes a row:
//! - the sheet's name into `Title/TitleText`;
//! - the sheet's reward name text into `RewardName`;
//! - the other-count text into `Others`;
//! - `Get` is shown when the sheet's completion mission is complete.
//!
//! The copies' `ThumbnailRoot` thumbnails are not drawn: their art is not on
//! this root.
//!
//! The reward name and the other-count text are not in the list's view data
//! (`MysekaiMissionListViewData` carries the sheet, the completion mission and
//! the normal missions): the source builds them from the rewards
//! `MysekaiMissionUtility` converts, which this unit's data does not carry, so
//! [`SheetRow`] keeps no text for them and the rows leave those two nodes
//! empty. The sheet's completion is read from the mission masters' completion
//! mission, not from the user data (`IsAllCompleted`'s session state).
//!
//! `SetupFirstView` places no row: the source's pick of the first index needs
//! `IsAllCompleted`, which a host would take from a session state this unit
//! does not own, and a false pick would show a wrong sheet.
//!
//! Named gaps: the `Tabs` object stays as the document has it (inactive);
//! the window scale tween of `DialogBase` is not drawn (the canvas scale the
//! view's entity carries stands); the footer's close label is the document's
//! serialized text (`useWordingKey` on with an empty key); the list's wheel
//! and drag scroll is not drawn, so the rows are placed from the list's top
//! and the scrollbar object is hidden.

use std::collections::{BTreeSet, HashSet};

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use serde_json::Value;

use super::cell::{
    component_at, write_text, MissionDialogRequest, IMAGE_CLASS, SHEET_CELL, TEXT_CLASS,
};
use super::detail_dialog::{load_cell, CellSource};
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
/// The runtime key of the cell document, which joins the runtime documents so
/// the row texts' glyph set covers the cell's own texts.
const CELL_VIEW: &str = "MissionListSheetCell";
/// `MysekaiMissionSheetCell`'s root object.
const CELL_ROOT: &str = "MysekaiMissionSheetCell";
/// The cell's script (`Sekai.Mysekai.MysekaiMissionSheetCell`), whose fields
/// name the nodes `Setup` writes.
const CELL_CLASS: &str = "Sekai.Mysekai.MysekaiMissionSheetCell";
/// The cell's `_titleText`.
const TITLE_TEXT: &str = "Title/TitleText";
/// The cell's `_rewardText`.
const REWARD_TEXT: &str = "RewardName";
/// The cell's `_otherText`.
const OTHER_TEXT: &str = "Others";
/// The cell's `_getObject`, the object `Setup` shows once the mission is received.
const GET_OBJECT: &str = "Get";
/// The row copies' names in the dialog's document (`Mysekai0`, `Mysekai1`,
/// ...), as the detail dialog names its reward rows.
const ROW_NAME: &str = "Mysekai";

/// A cell node a row writes, as a path below a row's root and as the field of
/// the cell's script that names it.
struct CellNode {
    path: &'static str,
    field: &'static str,
}

const CELL_TEXT_NODES: [CellNode; 3] = [
    CellNode {
        path: TITLE_TEXT,
        field: "_titleText",
    },
    CellNode {
        path: REWARD_TEXT,
        field: "_rewardText",
    },
    CellNode {
        path: OTHER_TEXT,
        field: "_otherText",
    },
];

const CELL_IMAGE_NODES: [CellNode; 1] = [CellNode {
    path: GET_OBJECT,
    field: "_getObject",
}];

/// The cell's script component on its root object.
fn cell_script(cell: &UiPrefab) -> Option<&UiComponent> {
    let index = cell.find(CELL_ROOT).ok()?;
    cell.nodes[index]
        .components
        .iter()
        .find(|component| component.class == CELL_CLASS)
}

/// The object of the cell's script a node field names, as the field holds it.
fn field_object<'a>(script: &'a UiComponent, field: &str) -> Option<&'a Value> {
    script.fields.get(field)
}

/// One node of the cell a row writes, read from the cell document and checked
/// against the cell's script: the node exists, carries `class`, and the script
/// names that same object in `field`. `Err` names what the document lacks.
fn cell_node(
    cell: &UiPrefab,
    script: &UiComponent,
    node: &CellNode,
    class: &str,
) -> Result<String, String> {
    let full = format!("{CELL_ROOT}/{}", node.path);
    let index = cell
        .find(&full)
        .map_err(|_| format!("{SHEET_CELL} has no {}", node.path))?;
    let ui_node = &cell.nodes[index];
    let same = field_object(script, node.field)
        .and_then(Value::as_array)
        .and_then(|pair| pair.get(1))
        .and_then(Value::as_i64);
    let component = ui_node
        .components
        .iter()
        .find(|component| component.class == class);
    if component.is_none() {
        return Err(format!("{} has no {class}", node.path));
    }
    // A field holds either the object itself (`_getObject`) or one of its
    // components (the three text fields).
    let names_node = same == Some(ui_node.game_object_id)
        || component.is_some_and(|component| same == Some(component.path_id));
    if !names_node {
        return Err(format!(
            "{SHEET_CELL}'s {} does not name its {} object, {}",
            node.field, node.path, ui_node.name
        ));
    }
    Ok(format!("/{}", node.path))
}

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
        // The list's own component, so a document that carries no list view is
        // refused here rather than when a row is placed.
        list_spacing(doc, &list)?;
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

/// `Sekai.UI.ListView`: the list's own component.
const LIST_VIEW_CLASS: &str = "Sekai.UI.ListView";

/// The list's serialized padding (left, right, top, bottom) and
/// `verticalSpacing`; `Err` when the node carries no list view.
fn list_spacing(doc: &UiPrefab, list: &str) -> Result<([f32; 4], f32), String> {
    let node = &doc.nodes[doc.find(list)?];
    let list_view = node
        .components
        .iter()
        .find(|component| component.class == LIST_VIEW_CLASS)
        .ok_or_else(|| format!("{list} has no {LIST_VIEW_CLASS}"))?;
    let fields = &list_view.fields;
    let mut padding = [0.0; 4];
    for (slot, part) in padding
        .iter_mut()
        .zip(fields["padding"].as_array().into_iter().flatten())
    {
        *slot = part.as_f64().unwrap_or(0.0) as f32;
    }
    let spacing = fields["verticalSpacing"].as_f64().unwrap_or(0.0) as f32;
    Ok((padding, spacing))
}

/// The list, as `ListView` places its rows: the cell's authored height and
/// the list's padding and spacing, read from the two documents.
#[derive(Debug, Clone)]
struct SheetList {
    /// The list's serialized padding (left, right, top, bottom) and
    /// `verticalSpacing`.
    padding: [f32; 4],
    spacing: f32,
    /// The cell's authored height.
    cell_height: f32,
}

impl SheetList {
    /// `SetListViewItemVertical`: row `index`'s anchored position, the list's
    /// left padding and `-row * (cell height + spacing) - top padding`.
    fn row_position(&self, index: usize) -> Vec2 {
        Vec2::new(
            self.padding[0],
            -(index as f32) * (self.cell_height + self.spacing) - self.padding[2],
        )
    }

    /// `ListView.SetContentSize`: the content's height for `count` rows, the
    /// rows' height plus the padding, less the last spacing.
    fn content_height(&self, count: usize) -> f32 {
        if count == 0 {
            return 0.0;
        }
        count as f32 * (self.cell_height + self.spacing) + self.padding[2] + self.padding[3]
            - self.spacing
    }
}

/// The nodes a row writes and where rows go, read from the two documents:
/// the cell document checked against its own script, and the sheet list.
#[derive(Debug, Clone)]
struct SheetWrites {
    /// `Title/TitleText` (`_titleText`), below a row's root.
    title: String,
    /// `RewardName` (`_rewardText`).
    reward: String,
    /// `Others` (`_otherText`).
    others: String,
    /// `Get` (`_getObject`).
    get: String,
    list: SheetList,
}

impl SheetWrites {
    /// `Err` names what the cell document lacks or how it disagrees with the
    /// dialog's list.
    fn read(dialog: &UiPrefab, cell: &UiPrefab, list: &str) -> Result<Self, String> {
        let root = cell
            .nodes
            .first()
            .map(|node| node.path.clone())
            .ok_or("the cell document has no root")?;
        if root != CELL_ROOT {
            return Err(format!("the cell document's root is {root}, not {CELL_ROOT}"));
        }
        let script = cell_script(cell).ok_or_else(|| format!("{CELL_ROOT} has no {CELL_CLASS}"))?;
        let mut paths = [String::new(), String::new(), String::new()];
        for (node, path) in CELL_TEXT_NODES.iter().zip(paths.iter_mut()) {
            *path = cell_node(cell, script, node, TEXT_CLASS)?;
        }
        let get = cell_node(cell, script, &CELL_IMAGE_NODES[0], IMAGE_CLASS)?;
        let (padding, spacing) = list_spacing(dialog, list)?;
        let cell_height = cell
            .find(CELL_ROOT)
            .map_err(|_| format!("{SHEET_CELL} document has no {CELL_ROOT} root"))?;
        let [title, reward, others] = paths;
        Ok(Self {
            title,
            reward,
            others,
            get,
            list: SheetList {
                padding,
                spacing,
                cell_height: cell.nodes[cell_height].rect.size_delta[1],
            },
        })
    }
}

/// A cell-relative path as a node of a row's copy.
fn row_node(root: &str, suffix: &str) -> String {
    format!("{root}{suffix}")
}

/// A sheet row's copy below the list's content.
fn row_root(content: &str, index: usize) -> String {
    format!("{content}/{ROW_NAME}{index}")
}

/// One copy of the cell template per sheet row under the list's content,
/// named `Mysekai{row}`; the copies keep the cell's root position until
/// `Setup` places them. `Err` names what the documents lack.
fn compose_rows(
    document: &mut UiPrefab,
    cell: &UiPrefab,
    content: &str,
    count: usize,
) -> Result<(), String> {
    let names: Vec<String> = (0..count).map(|row| format!("{ROW_NAME}{row}")).collect();
    let instances =
        document.instantiate_children(content, cell, CELL_ROOT, names.iter().map(String::as_str))?;
    if instances.len() != count {
        return Err(format!("{count} cell copies were not made below {content}"));
    }
    Ok(())
}

/// The glyph set the texts are laid out with: the characters of every
/// installed document, the cell document's serialized texts among them. The
/// list's documents are installed before a row exists, so the row count never
/// changes the set.
fn glyphs_of(layouts: &UiLayouts) -> HashSet<char> {
    layouts.text_chars().into_iter().collect()
}

/// `MysekaiMissionSheetCell.Setup` on one copy: the sheet's name into
/// `Title/TitleText`, `Get` shown while the sheet is complete. The reward name
/// and the other-count text have no value in the list's view data and are
/// drawn empty.
fn draw_row(
    view: &mut UiPrefabView,
    cell: &UiPrefab,
    root: &str,
    writes: &SheetWrites,
    data: &SheetRow,
    glyphs: &HashSet<char>,
    seen: &mut BTreeSet<String>,
) {
    write_text(
        view,
        cell,
        &row_node(root, &writes.title),
        &data.name,
        glyphs,
        seen,
        "a sheet row's",
    );
    view.set_text(&row_node(root, &writes.reward), String::new());
    view.set_text(&row_node(root, &writes.others), String::new());
    view.set_visible(&row_node(root, &writes.get), data.received);
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
    /// The sheet's name (`MysekaiNormalMissionSheet.name`), the row's
    /// `Title/TitleText`.
    pub name: String,
    /// `GetParentMission(sheet.id)`'s master id.
    pub complete: Option<i32>,
    /// The completion mission's status reads received (`Get` is shown then:
    /// `MissionStatus - 2 == not_achieved`).
    pub received: bool,
    /// `GetChildMissions(sheet.id)`, in `seq` order.
    pub children: Vec<i32>,
}

/// `CreateMissionAllList`: every sheet in `seq` order.
pub fn all_rows(
    masters: &moly_masters::mission::MissionMasters,
    state: &super::model::UserMissionState,
) -> Vec<SheetRow> {
    masters
        .sheets_by_seq()
        .into_iter()
        .map(|sheet| SheetRow {
            sheet: sheet.id,
            name: sheet.name.clone(),
            complete: masters.parent_mission(sheet.id).map(|mission| mission.id),
            received: masters.parent_mission(sheet.id).is_some_and(|mission| {
                super::model::mission_state(masters, state, mission)
                    .is_some_and(|view| view.status == super::model::MissionStatus::Received)
            }),
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
    /// Whether the list refused its documents; the refusal is final.
    pub(crate) refused: bool,
    /// The two documents' row writes, once installed.
    pub(crate) writes: Option<SheetWrites>,
    pub(crate) seen: BTreeSet<String>,
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

/// Update: the dialog's runtime document and view, once the root's document
/// and the cell document are in.
#[allow(clippy::too_many_arguments)]
pub fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut cell_source: ResMut<CellSource>,
    mut dialog: ResMut<MissionListDialog>,
    roots: Query<(), With<MissionListDialogRoot>>,
) {
    if !roots.is_empty() || dialog.refused {
        return;
    }
    let Some(doc) = layouts.document(KEY) else {
        return;
    };
    let bindings = match Bindings::read(doc) {
        Ok(bindings) => bindings,
        Err(reason) => return refuse(&mut dialog, reason),
    };
    let cell = match load_cell(&layouts, &server, &json, SHEET_CELL, &mut cell_source) {
        Ok(Some(cell)) => cell,
        Ok(None) => return,
        Err(reason) => return refuse(&mut dialog, reason),
    };
    let writes = match SheetWrites::read(doc, &cell, &bindings.list) {
        Ok(writes) => writes,
        Err(reason) => return refuse(&mut dialog, reason),
    };
    let document = doc.clone();
    // The cell document's own nodes join the runtime documents, so the glyph
    // set read for the row texts covers them.
    if let Err(reason) = layouts.replace_runtime_document(CELL_VIEW, cell, &server) {
        error!("mission: {KEY} is not installed: {reason}");
        return;
    }
    if let Err(reason) = layouts.replace_runtime_document(VIEW, document, &server) {
        error!("mission: {KEY} is not installed: {reason}");
        return;
    }
    dialog.writes = Some(writes);
    let mut view = UiPrefabView::new(VIEW, SITEMAP_LAYER);
    view.set_visible(&bindings.tabs, false);
    // `SetupFirstView` places no row (see the module notes); the list's
    // content stands empty and its scrollbar hidden until `Setup` copies the
    // rows in.
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

/// The dialog's fail-closed refusal, named once: the list draws no row.
fn refuse(dialog: &mut MissionListDialog, reason: String) {
    if !dialog.refused {
        error!("mission: {KEY} is refused: {reason}; the mission list dialog draws no row");
    }
    dialog.refused = true;
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
#[allow(clippy::too_many_arguments)]
pub fn open(
    mut requests: MessageReader<MissionDialogRequest>,
    mut dialog: ResMut<MissionListDialog>,
    mut screens: ResMut<ScreenManager>,
    tables: Option<Res<moly_masters::load::MasterTables>>,
    roots: Query<(), With<MissionListDialogRoot>>,
    layouts: Res<UiLayouts>,
    user: Option<Res<ClientMission>>,
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
    let Some(user) = user else {
        error!("mission: OnClickMissionListButton before the mission user data is in: nothing");
        return;
    };
    let mut state = user.state();
    state.current_sheet = screen::current_sheet(masters, &user);
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
    dialog.rows = all_rows(masters, &state);
    dialog.first = None;
    dialog.set_up = false;
    info!(
        "mission: MysekaiMissionListDialog: Show1ButtonDialog<MysekaiMissionListDialog>(340) is open; CreateMissionAllList made {} sheet row(s)",
        dialog.rows.len()
    );
}

/// Update: `Setup` on the dialog's own view, once its slot is open: the
/// runtime copy is installed again with one `MysekaiMissionSheetCell` copy per
/// sheet row, and each copy runs the cell's `Setup` on its row.
#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut dialog: ResMut<MissionListDialog>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    screens: Res<ScreenManager>,
    json: Res<Assets<JsonAsset>>,
    mut cell_source: ResMut<CellSource>,
    mut views: Query<(&Bindings, &mut UiPrefabView), With<MissionListDialogRoot>>,
) {
    let Some(id) = dialog.shown else {
        return;
    };
    if !screens.dialogs().any(|(open, _, _)| open == id) {
        return;
    }
    // `Setup` runs once, on the first frame its documents are in.
    if dialog.set_up || dialog.refused {
        return;
    }
    let Some(template) = layouts.document(KEY).cloned() else {
        return;
    };
    let Some(writes) = dialog.writes.clone() else {
        return;
    };
    let cell = match load_cell(&layouts, &server, &json, SHEET_CELL, &mut cell_source) {
        Ok(Some(cell)) => cell,
        Ok(None) => {
            let waiting = format!("setup waits, {} row(s)", dialog.rows.len());
            if dialog.seen.insert(waiting) {
                info!("mission: MysekaiMissionListDialog.Setup waits for the {SHEET_CELL} document");
            }
            return;
        }
        Err(reason) => return refuse(&mut dialog, reason),
    };
    let Ok((bindings, mut view)) = views.single_mut() else {
        return;
    };
    let bindings = bindings.clone();
    // `CreateViewItem(list.Count)` then `SetupFirstView`: one copy per row.
    let mut document = template;
    if let Err(reason) = compose_rows(&mut document, &cell, &bindings.content, dialog.rows.len()) {
        return refuse(&mut dialog, reason);
    }
    if let Err(reason) = layouts.replace_runtime_document(VIEW, document, &server) {
        return refuse(&mut dialog, reason);
    }
    // The glyph set covers the documents before the copy, so it is read after
    // it went in.
    let glyphs = glyphs_of(&layouts);
    let mut seen = std::mem::take(&mut dialog.seen);
    for (index, data) in dialog.rows.iter().enumerate() {
        let root = row_root(&bindings.content, index);
        draw_row(&mut view, &cell, &root, &writes, data, &glyphs, &mut seen);
        view.set_anchored_position(&root, writes.list.row_position(index));
    }
    view.set_size_delta(
        &bindings.content,
        Vec2::new(0.0, writes.list.content_height(dialog.rows.len())),
    );
    view.set_visible(&bindings.content, true);
    // The list's scroll is not drawn: the rows are placed from the list's top
    // and the scrollbar object stays hidden.
    view.set_visible(&bindings.scrollbar, false);
    if !dialog.rows.is_empty() && seen.insert("thumbnail".to_owned()) {
        info!(
            "mission: the sheet rows' thumbnails ({SHEET_CELL}'s ThumbnailRoot) are not drawn: their art is not on this root"
        );
    }
    dialog.seen = seen;
    dialog.set_up = true;
    dialog.first = None;
    info!(
        "mission: MysekaiMissionListDialog.Setup: CreateViewItem({count}); {count} sheet row(s) copied from {SHEET_CELL}; SetupFirstView places none",
        count = dialog.rows.len()
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
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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
#[allow(clippy::too_many_arguments)]
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

