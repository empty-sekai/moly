//! `ScreenLayerMysekaiMission` (screen 621): the mission screen's view, its
//! two pages and the page turn.
//!
//! Source structure:
//! - `OnBoot`: the multiplay UI is hidden (this tree has no multiplay UI),
//!   then `ScreenBootDone`.
//! - `OnInitComponent`: `InitializePage` (both pages set up, page 1 current
//!   at x 0, page 2 next at x 2580, both stamp roots inactive, page 1's
//!   mission view initialized with the user's current sheet), `SetupEvent`,
//!   `RefreshScreenView`, `UpdateCurrentPageStatus`, `SetupTutorial`, then
//!   `ContinueNextMission` when the temporary return cache holds
//!   `FinishExpand`, and the cache entry is always removed.
//! - `OnWillExit`: the multiplay UI again, then `ScreenWillExitDone`.
//! - `OnExited`: each page disposed (its stamp unloaded), the bundle
//!   unloaded.
//!
//! The screen shows the model's list ([`crate::mission::model`]). Its
//! buttons:
//! - the page's own cell is the sheet's completion mission
//!   (`_completeCell`): a complete sheet sends the bulk receive
//!   (`OnClickMysekaiMissionCell` -> `OnClickBulkReceiveButton`), otherwise
//!   the cell opens the detail dialog;
//! - `MissionListButton` opens the sheet list
//!   (`Show1ButtonDialog<MysekaiMissionListDialog>(340)`);
//! - `BulkReceiveButton` sends the bulk receive (`OnClickBulkReceiveButton`)
//!   when the receivable list is not empty (`UpdateCurrentPageStatus`).
//!
//! The list's normal mission rows are [`crate::mission::cell`]'s: it copies
//! the document's own `MysekaiMissionCell` subtree into each page's list and
//! reports which row a press reached. A row tap is
//! `OnClickMysekaiMissionCell` too: an achieved mission sends its receive, any
//! other row opens the detail dialog.
//!
//! The page turn (`TurnPage`) runs when a receive completes the sheet
//! (`StartNextMission`): the next page's view is set to the user's current
//! sheet and refreshed, the next root squashes to 0.9 and the current root
//! follows (`DOScale(0.9, 0.125)`), 125 ms later the next root is restored
//! (`DOScale(1, 0.125)`), the current page's stamp hides, the pages swap
//! (`index = ~index & 1`), the next page waits at 2580, and both roots then
//! slide (the next from 2580 to 0, the current from 0 to -2580) over 0.2 s
//! with `Ease.OutCubic`, snapping at the end. The scale steps are not drawn:
//! this view has no scale override (named). The start dialog and the rule
//! slides of `StartNextMission` are later units (named). The reward chain,
//! which decides that the sheet is complete, is a later unit: it calls
//! [`turn_page`], and until then no turn starts by itself.
//!
//! The stamp (`ShowMissionComplete`, the FX instantiated under the page's
//! `CompleteStampRoot`) and the tap cover (`_tapCoverCollider`, on while the
//! reward chain plays) are drawn from the screen state. The cover's
//! `TapCoverEffect` and `Cover` are drawn as the document has them.
//!
//! Product mapping, named:
//! - The completion cell's nodes are bound and drawn from
//!   [`PageContent::complete`], the sheet's completion mission. The list's
//!   normal-mission rows are [`crate::mission::cell`]'s: it copies the
//!   document's own cell subtree into each page's list and reports which row a
//!   press reached, and the list's texts the atlas cannot draw are drawn empty
//!   there rather than left as the copied text.
//! - The document's `ListView/NoneInfo` text is bound with `useWordingKey`
//!   on and an empty key, so the serialized text stands exactly as the
//!   client shows it. It is shown from the cell unit, which owns the list.
//! - This tree has no tutorial domain (`SetupTutorial`'s balloon) and no
//!   site-return cache (`MYSEKAI_MISSION_RETURN`): both are refused by name.
//! - A text the shared glyph atlas cannot draw is skipped and its node keeps
//!   the document's text (as the convert screen does).

use std::collections::BTreeSet;
use std::sync::Arc;

use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::UiPrefab;
use moly_law::ui::custom_button as button_rule;
use moly_law::ui::dotween::Ease;
use moly_masters::mission::MissionMasters;
use serde_json::Value;

use super::model::{self, CellTap, MissionScreen as MissionModel};
use super::service::{MissionReceiveFailed, MissionReceived, MysekaiMissionService};
use crate::balloon::BALLOON_LAYER;
use crate::canvas::RootCanvas;
use crate::gesture::{UiPointerEvent, UiPointerPhase};
use crate::menu_shell::ShellDialogState;
use crate::tween_clock::TweenClock;
use crate::ui_inputs::BalloonArt;
use crate::ui_layers::{
    LayerCommand, MenuScreenType, ScreenHook, ScreenLayerEvent, ScreenLayerState, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use crate::userdata::mission::ClientMission;

/// The screen layer's document (`screens/ScreenLayerMysekaiMission.json`).
pub const KEY: &str = "MissionScreen";
/// The screen (`MenuScreenType.MysekaiMission`).
pub const SCREEN: MenuScreenType = MenuScreenType::MysekaiMission;
const CONFIG_ROOT: &str = "ScreenLayerMysekaiMission/ComponentRoot";
const VIEW_CLASS: &str = "Sekai.Mysekai.MysekaiMissionView";
const CELL_CLASS: &str = "Sekai.Mysekai.MysekaiMissionCell";
const GAUGE_CLASS: &str = "Sekai.UIPartsGauge";
const BUTTON_CLASS: &str = "Sekai.UI.CustomButton";
const TEXT_CLASS: &str = "Sekai.UI.CustomTextMesh";
const IMAGE_CLASS: &str = "Sekai.UI.CustomImage";
/// `OnClickBulkReceiveButton`'s button text (`WORD_OBTAIN_ALL`).
pub const OBTAIN_ALL: &str = "WORD_OBTAIN_ALL";
/// `SetupOtherCount`'s "other kinds" label (`WORD_OTHER_KIND_COUNT`).
pub const OTHER_KIND_COUNT: &str = "WORD_OTHER_KIND_COUNT";
/// The list header (`WORD_SEKAI_MISSION`, read by the document).
pub const HEADER: &str = "WORD_SEKAI_MISSION";
/// `MysekaiMissionPage.SetX`: the page turn distance.
pub const PAGE_TURN_X: f32 = 2580.0;
/// `TurnPage`'s scale steps (`DOScale(0.9, 0.125)` / `DOScale(1, 0.125)`).
pub const TURN_SCALE_SECONDS: f32 = 0.125;
pub const TURN_SCALE: f32 = 0.9;
/// The page slide (`DOAnchorPosX` over 0.2 s, `Ease.OutCubic`, snapping).
pub const TURN_SLIDE_SECONDS: f32 = 0.2;

// ---------------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------------

/// The completion cell's nodes, as `MysekaiMissionCell`'s script writes
/// them.
#[derive(Debug, Clone)]
pub struct CellBinding {
    /// `MissionCellView/Root/Mission` (`SetMissionContent`).
    pub mission: String,
    /// `MissionCellView/Root/Progress` (the `"{current}/{max}"` text).
    pub progress: String,
    /// `MissionCellView/Root/AchieveLabel` (`SetCompleteLabel`).
    pub achieve: String,
    /// `MysekaiMissionCell/Others` (`SetupOtherCount`).
    pub others: String,
    /// The `UIPartsGauge`'s `fillImage`.
    pub fill: String,
    /// `MissionCellView`'s CustomButton: its node path and component identity.
    /// The path, not the node index: the cell unit inserts list rows into the
    /// document, which moves every later node.
    pub button: (String, i64),
}

/// One page's nodes, read from the document.
#[derive(Debug, Clone)]
pub struct PageBinding {
    /// `ComponentRoot/Page{n}`: the node whose x the turn writes.
    pub root: String,
    /// The page root's authored y, which the turn keeps.
    pub root_y: f32,
    /// `NormalMissionRoot` (`MysekaiMissionView`).
    pub view: String,
    /// `NormalMissionRoot/UIPartsGroupListView/ListView`.
    pub list: String,
    /// `ListView/NoneInfo`.
    pub none_info: String,
    /// The page's completion cell.
    pub cell: CellBinding,
    /// `MissionListButton` (path and component identity).
    pub list_button: (String, i64),
    /// `BulkReceiveButton` (path and component identity).
    pub bulk_button: (String, i64),
    /// `CompleteStampRoot`.
    pub stamp: String,
}

impl PageBinding {
    fn read(doc: &UiPrefab, root: &str) -> Result<Self, String> {
        let index = doc
            .find(root)
            .map_err(|_| format!("the document has no {root}"))?;
        let root_y = doc.nodes[index].rect.anchored_position[1];
        let view = under(doc, root, VIEW_CLASS)
            .ok_or_else(|| format!("{root} has no {VIEW_CLASS}"))?;
        let cell = under(doc, &view, CELL_CLASS)
            .ok_or_else(|| format!("{view} has no {CELL_CLASS}"))?;
        let list = format!("{view}/UIPartsGroupListView/ListView");
        let cell_view = format!("{cell}/MissionCellView");
        let cell_root = format!("{cell_view}/Root");
        let gauge = format!("{cell_root}/UIPartsGauge");
        let button = |path: &str| {
            let index = doc.find(path)?;
            let component = doc.nodes[index]
                .components
                .iter()
                .find(|component| component.class == BUTTON_CLASS)
                .ok_or_else(|| format!("{path} has no {BUTTON_CLASS}"))?;
            Ok::<_, String>((doc.nodes[index].path.clone(), component.path_id))
        };
        let fill_id = component_reference(doc, &gauge, GAUGE_CLASS, "fillImage")
            .ok_or_else(|| format!("{gauge} has no fillImage"))?;
        let fill = image_path(doc, fill_id)
            .ok_or_else(|| format!("fillImage {fill_id} is not a CustomImage node"))?;
        Ok(Self {
            root: root.to_owned(),
            root_y,
            view: view.clone(),
            none_info: format!("{list}/NoneInfo"),
            list,
            cell: CellBinding {
                mission: format!("{cell_root}/Mission"),
                progress: format!("{cell_root}/Progress"),
                achieve: format!("{cell_root}/AchieveLabel"),
                others: format!("{cell}/Others"),
                fill,
                button: button(&cell_view)?,
            },
            list_button: button(&format!("{root}/MissionListButton"))?,
            bulk_button: button(&format!("{root}/BulkReceiveButton"))?,
            stamp: format!("{root}/CompleteStampRoot"),
        })
    }
}

/// The screen's bindings.
#[derive(Debug, Clone)]
pub struct ScreenBindings {
    pub pages: [PageBinding; 2],
    /// `ComponentRoot/FillCollider` (`_tapCoverCollider`).
    pub tap_cover: String,
    /// `ComponentRoot/TutorialImage/image` (`_tutorialNaviImage`).
    pub tutorial_image: String,
}

impl ScreenBindings {
    /// Which button (0 the page's own cell, 1 the list, 2 the bulk receive) of
    /// `page` a pressed node path and component identity name.
    pub fn button_role(&self, page: usize, path: &str, id: i64) -> Option<usize> {
        let page = &self.pages[page & 1];
        let buttons = [&page.cell.button, &page.list_button, &page.bulk_button];
        buttons
            .into_iter()
            .position(|button| button.0 == path && button.1 == id)
    }
}

/// The node path of the first node under `prefix` carrying `class`.
fn under(doc: &UiPrefab, prefix: &str, class: &str) -> Option<String> {
    doc.nodes
        .iter()
        .filter(|node| node.path.starts_with(&format!("{prefix}/")))
        .find(|node| node.components.iter().any(|component| component.class == class))
        .map(|node| node.path.clone())
}

/// A serialized reference of the component at `path` whose class is
/// `class`; `None` for a null reference or one outside the document.
fn component_reference(doc: &UiPrefab, path: &str, class: &str, field: &str) -> Option<i64> {
    let index = doc.find(path).ok()?;
    let component = doc.nodes[index]
        .components
        .iter()
        .find(|component| component.class == class)?;
    let pair = component.fields.get(field)?.as_array()?;
    if pair.first().and_then(Value::as_i64) != Some(0) {
        return None;
    }
    pair.get(1).and_then(Value::as_i64).filter(|id| *id != 0)
}

/// The node path of the `CustomImage` component `id` (a gauge fill).
fn image_path(doc: &UiPrefab, id: i64) -> Option<String> {
    let index = doc.find(&format!("@{id}")).ok()?;
    doc.nodes[index]
        .components
        .iter()
        .any(|component| component.class == IMAGE_CLASS)
        .then(|| doc.nodes[index].path.clone())
}

/// The serialized text of the `CustomTextMesh` at `path`.
pub fn serialized_text<'a>(doc: &'a UiPrefab, path: &str) -> Option<&'a str> {
    let index = doc.find(path).ok()?;
    let component = doc.nodes[index]
        .components
        .iter()
        .find(|component| component.class == TEXT_CLASS)?;
    component.fields["m_text"].as_str()
}

/// Reads the screen's two pages; `Err` names what the document lacks.
pub fn bind(doc: &UiPrefab) -> Result<ScreenBindings, String> {
    Ok(ScreenBindings {
        pages: [
            PageBinding::read(doc, &format!("{CONFIG_ROOT}/Page1"))?,
            PageBinding::read(doc, &format!("{CONFIG_ROOT}/Page2"))?,
        ],
        tap_cover: format!("{CONFIG_ROOT}/FillCollider"),
        tutorial_image: format!("{CONFIG_ROOT}/TutorialImage/image"),
    })
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// One list cell's content, as `MysekaiMissionCell.Setup` writes it
/// (`CompleteCell` is its name on the page's own cell).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompleteCell {
    /// `SetMissionContent(master.sentence)`.
    pub mission: String,
    /// The `"{current}/{max}"` text.
    pub progress: String,
    /// `UIPartsGauge.Setup`'s fill.
    pub fill: f32,
    /// `SetCompleteLabel(IsComplete)`.
    pub achieved: bool,
    /// `SetupOtherCount`: the argument of `WORD_OTHER_KIND_COUNT`, shown for
    /// two or more detail rows.
    pub others: Option<i32>,
}

impl CompleteCell {
    /// `MysekaiMissionCell.Setup(row)` of one list row.
    pub(crate) fn of(row: &model::MissionRow<'_>) -> Self {
        Self {
            mission: row.view().master.sentence.clone(),
            progress: row.progress_text(),
            fill: row.gauge_fill(),
            achieved: row.view().is_complete(),
            others: None,
        }
    }
}

/// One page, as `RefreshScreenView` last wrote it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PageContent {
    /// The completion cell; `None` until the masters read, so the document's
    /// texts stand.
    pub complete: Option<CompleteCell>,
    /// The normal missions, in the order [`crate::mission::cell`]'s list
    /// copies its cells for.
    pub children: Vec<CompleteCell>,
    /// `CreateMissionList` made no normal mission cell (the list's
    /// "NoneInfo" text is the cell unit's to show).
    pub none: bool,
    /// The stamp shows (`ShowMissionComplete`).
    pub stamp: bool,
}

impl PageContent {
    /// `MysekaiMissionView.Refresh` / `CreateMissionList`: the page's rows.
    pub fn from_model(model: &MissionModel<'_>) -> Self {
        let mut content = Self::default();
        for row in &model.rows() {
            match row {
                model::MissionRow::Complete { .. } => {
                    content.complete = Some(CompleteCell::of(row));
                }
                model::MissionRow::Mission(_) => content.children.push(CompleteCell::of(row)),
            }
        }
        content.none = content.children.is_empty();
        content
    }
}

/// The page turn's steps (`TurnPage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    /// `DOScale(0.9, 0.125)` on the next page.
    Squash,
    /// The 125 ms wait, then `DOScale(1, 0.125)` on the next page.
    Restore,
    /// The pages swapped: the next page waits at 2580 and both slide.
    Slide,
}

impl Turn {
    /// The step after this one; `None` after the slide.
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Squash => Some(Self::Restore),
            Self::Restore => Some(Self::Slide),
            Self::Slide => None,
        }
    }

    /// The step's length.
    pub fn seconds(self) -> f32 {
        match self {
            Self::Squash | Self::Restore => TURN_SCALE_SECONDS,
            Self::Slide => TURN_SLIDE_SECONDS,
        }
    }
}

/// A page turn in flight: its step and the step's clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Turning {
    pub step: Turn,
    pub clock: TweenClock,
}

/// The slide's x: the next page from 2580 to 0 and the current page from 0
/// to -2580, `Ease.OutCubic`; past the duration it is at the end (the
/// source's snapping).
pub fn slide_x(start: f32, elapsed: f32) -> f32 {
    let ease = Ease::OutCubic.evaluate(elapsed.clamp(0.0, TURN_SLIDE_SECONDS), TURN_SLIDE_SECONDS);
    start * (1.0 - ease)
}

/// `ScreenLayerMysekaiMission`'s state.
#[derive(Resource, Default)]
pub struct MissionScreen {
    bindings: Option<Arc<ScreenBindings>>,
    absent: Option<String>,
    spawned: bool,
    booted: bool,
    set_up: bool,
    /// `_currentPageIndex`: 0 or 1.
    pub current: usize,
    /// The sheet the current page shows (`GetCurrentMissionSheetId` at the
    /// boot, or the user's sheet at the refresh `OnFinishReceiveDialog`
    /// runs).
    sheet: i32,
    /// The receive is out: the tap cover shows (`_tapCoverCollider`).
    covering: bool,
    /// The page turn in flight.
    turn: Option<Turning>,
    /// Each page's data.
    pages: [PageContent; 2],
    /// Each page's x: the current page at 0, the next at 2580.
    offsets: [f32; 2],
    /// The button the current press went to: its node path (a path, not an
    /// index, because the cell unit inserts list rows and rebuilds the node
    /// indices) and its component identity.
    pressed: Option<(String, i64)>,
    refusal_popped: bool,
    seen: BTreeSet<String>,
}

impl MissionScreen {
    /// The page turn runs (taps disabled).
    pub fn turning(&self) -> bool {
        self.turn.is_some()
    }

    /// The tap cover shows (`_tapCoverCollider`).
    pub fn covered(&self) -> bool {
        self.covering
    }

    /// Why the screen cannot be drawn on this root.
    #[allow(dead_code)] // Read by a host's panel.
    pub fn absent(&self) -> Option<&str> {
        self.absent.as_deref()
    }

    /// A page's content.
    pub fn page(&self, index: usize) -> &PageContent {
        &self.pages[index & 1]
    }

    /// The sheet the current page shows.
    pub fn sheet(&self) -> i32 {
        self.sheet
    }

    /// The page a turn is about to bring in.
    pub fn next_page(&self) -> usize {
        1 - (self.current & 1)
    }
}

/// Logs one line once per distinct message (`mission: <message>`).
pub fn note_once(seen: &mut BTreeSet<String>, message: String) {
    if seen.insert(message.clone()) {
        info!("mission: {message}");
    }
}

/// The one line a unit leaves where a later unit's call stands:
/// `"mission: <what> is not built yet"`, and nothing else.
pub fn not_built(seen: &mut BTreeSet<String>, what: &str) {
    note_once(seen, format!("{what} is not built yet"));
}

#[derive(Component)]
pub struct MissionScreenRoot;

// ---------------------------------------------------------------------------
// The model's inputs
// ---------------------------------------------------------------------------

/// The screen's model at a sheet: the user's missions read against the
/// masters, outside the tutorial. `None` while the sheet is not in the
/// masters.
pub fn read_model<'a>(
    masters: &'a MissionMasters,
    user: &'a ClientMission,
    sheet: i32,
) -> Option<MissionModel<'a>> {
    let mut state = user.state();
    state.current_sheet = sheet;
    MissionModel::new(masters, &state, false)
}

/// `GetCurrentMissionSheetId`: the user's sheet; the sheet of the lowest
/// `seq` when the copy holds none (or one the masters lack).
pub fn current_sheet(masters: &MissionMasters, user: &ClientMission) -> i32 {
    if masters.sheet(user.sheet_id()).is_some() {
        return user.sheet_id();
    }
    masters.first_sheet().map_or(0, |sheet| sheet.id)
}

/// `RefreshScreenView(page)`, with `UpdateCurrentPageStatus`'s log: the page
/// reads the model at the screen's sheet.
pub fn refresh_page(
    screen: &mut MissionScreen,
    masters: &MissionMasters,
    user: &ClientMission,
    index: usize,
) {
    let index = index & 1;
    let Some(model) = read_model(masters, user, screen.sheet) else {
        screen.pages[index] = PageContent::default();
        return;
    };
    let stamp = screen.pages[index].stamp;
    screen.pages[index] = PageContent::from_model(&model);
    screen.pages[index].stamp = stamp;
    let receivable = model.receivable();
    info!(
        "[mission] RefreshScreenView(page {}): sheet {}, {} normal mission(s) listed; UpdateCurrentPageStatus: {} receivable mission(s) {:?}; the bulk button {}",
        index + 1,
        screen.sheet,
        screen.pages[index].children.len(),
        receivable.len(),
        receivable,
        if model.bulk_enabled() { "enabled" } else { "disabled" }
    );
}

// ---------------------------------------------------------------------------
// The turn
// ---------------------------------------------------------------------------

/// `TurnPage`: the next page set to the user's current sheet and refreshed,
/// the current page's stamp hidden, then the turn's steps.
pub fn turn_page(screen: &mut MissionScreen, masters: &MissionMasters, user: &ClientMission) {
    if screen.turn.is_some() {
        info!("[mission] TurnPage is called while a turn runs; the turn continues");
        return;
    }
    let next = screen.next_page();
    screen.sheet = current_sheet(masters, user);
    refresh_page(screen, masters, user, next);
    screen.pages[screen.current].stamp = false;
    screen.offsets[next] = PAGE_TURN_X;
    screen.turn = Some(Turning {
        step: Turn::Squash,
        clock: TweenClock::new(Turn::Squash.seconds()),
    });
    info!(
        "[mission] TurnPage: page {} set to sheet {}, its root DOScale({TURN_SCALE}, {TURN_SCALE_SECONDS}), the next root DOScale(1, {TURN_SCALE_SECONDS}) after the step; then the pages swap and slide over {TURN_SLIDE_SECONDS} s (OutCubic); the current page's stamp hides",
        next + 1,
        screen.sheet
    );
    not_built(
        &mut screen.seen,
        "the next sheet's start dialog (ShowMissionStart, MysekaiMissionStartDialog) and CheckAndShowRuleSlide",
    );
}

/// One frame of the turn: whether it ended. A step ends when its clock
/// completes; the end of the restore swaps the pages and starts the slide,
/// and the end of the slide snaps both pages to rest.
pub fn step_turn(turn: &mut Turning, offsets: &mut [f32; 2], current: &mut usize, dt: f32) -> bool {
    match turn.step {
        Turn::Slide => {
            turn.clock.advance(dt);
            offsets[*current] = slide_x(PAGE_TURN_X, turn.clock.elapsed);
            offsets[1 - *current] = slide_x(0.0, turn.clock.elapsed);
            if turn.clock.done() {
                offsets[*current] = 0.0;
                offsets[1 - *current] = PAGE_TURN_X;
                true
            } else {
                false
            }
        }
        step => {
            turn.clock.advance(dt);
            if !turn.clock.done() {
                return false;
            }
            let Some(next) = step.next() else {
                return true;
            };
            turn.step = next;
            turn.clock = TweenClock::new(next.seconds());
            if next == Turn::Slide {
                // `index = ~index & 1`: the pages swap, the next page waits
                // at 2580, and the page that was current slides out.
                *current = 1 - (*current & 1);
                offsets[*current] = PAGE_TURN_X;
                offsets[1 - *current] = 0.0;
            }
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Update: the view once the document is in and both pages bind. A root
/// that carries no document, or one this view cannot read, refuses the
/// screen by name, and its push is popped ([`follow_screen`]).
pub fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut screen: ResMut<MissionScreen>,
) {
    if screen.spawned {
        return;
    }
    let Some(doc) = layouts.document(KEY) else {
        // The root's documents are all in when the first of them (the menu)
        // is: a document still absent then is one the root does not carry,
        // and the screen refuses by name (its push is then popped).
        let settled = layouts.document("Menu").is_some();
        if settled && screen.absent.is_none() {
            error!("[mission] this root carries no {KEY} document; the screen refuses to open by name");
            screen.absent = Some(format!("this root carries no {KEY} document"));
        }
        return;
    };
    let bindings = match bind(doc) {
        Ok(bindings) => bindings,
        Err(reason) => {
            error!("[mission] {KEY} is refused: {reason}");
            screen.absent = Some(reason);
            return;
        }
    };
    if !layouts.ready(KEY, &server) {
        return;
    }
    let mut view = UiPrefabView::new(KEY, BALLOON_LAYER);
    // `InitializePage`: both stamp roots inactive. `SetupTutorial` shows the
    // balloon only in the tutorial; the tap cover starts inactive.
    for page in &bindings.pages {
        view.set_visible(&page.stamp, false);
    }
    view.set_visible(&bindings.tutorial_image, false);
    view.set_visible(&bindings.tap_cover, false);
    let page = &bindings.pages[0];
    info!(
        "[mission] {KEY}: pages {} and {}; the completion cell's mission text {}, progress {}, achievement label {}, others {}, gauge fill {}",
        bindings.pages[0].root,
        bindings.pages[1].root,
        page.cell.mission,
        page.cell.progress,
        page.cell.achieve,
        page.cell.others,
        page.cell.fill
    );
    screen.bindings = Some(Arc::new(bindings));
    screen.spawned = true;
    commands.spawn((
        MissionScreenRoot,
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(BALLOON_LAYER),
        view,
    ));
}

/// Update, after the screen manager: the layer's hooks and the refused
/// push's pop.
#[allow(clippy::too_many_arguments)]
pub fn follow_screen(
    mut events: MessageReader<ScreenLayerEvent>,
    tables: Option<Res<moly_masters::load::MasterTables>>,
    user: Option<Res<ClientMission>>,
    mut screens: ResMut<ScreenManager>,
    mut layer_commands: MessageWriter<LayerCommand>,
    mut screen: ResMut<MissionScreen>,
) {
    let hooks: Vec<ScreenLayerEvent> = events
        .read()
        .filter(|event| event.screen == SCREEN)
        .copied()
        .collect();
    for event in hooks {
        match event.hook {
            ScreenHook::OnBoot => {
                if let Some(reason) = screen.absent.clone() {
                    error!(
                        "[mission] OnBoot on a root that refuses the screen ({reason}): ScreenBootDone at once, and the push is popped"
                    );
                    screen.booted = true;
                    continue;
                }
                let Some(masters) = tables.as_deref().and_then(|tables| tables.mission.as_ref())
                else {
                    note_once(
                        &mut screen.seen,
                        "OnBoot: the mission masters are not in; the boot waits".to_owned(),
                    );
                    continue;
                };
                let Some(user) = user.as_deref() else {
                    note_once(
                        &mut screen.seen,
                        "OnBoot: the user's mission data is not in; the boot waits".to_owned(),
                    );
                    continue;
                };
                // `InitializePage`: page 1 current at x 0, page 2 next at
                // 2580, page 1's view at the user's current sheet.
                screen.current = 0;
                screen.offsets = [0.0, PAGE_TURN_X];
                screen.sheet = current_sheet(masters, user);
                screen.pages = [PageContent::default(), PageContent::default()];
                refresh_page(&mut screen, masters, user, 0);
                screen.covering = false;
                screen.turn = None;
                screen.pressed = None;
                screen.booted = true;
                info!(
                    "[mission] OnBoot -> ScreenBootDone (the multiplay UI is not in this tree); InitializePage: page 1 at x 0, page 2 at x {PAGE_TURN_X}, sheet {}",
                    screen.sheet
                );
            }
            ScreenHook::OnInitComponent => {
                if !screen.booted || !screen.spawned {
                    continue;
                }
                let (Some(masters), Some(user)) = (
                    tables.as_deref().and_then(|tables| tables.mission.as_ref()),
                    user.as_deref(),
                ) else {
                    continue;
                };
                screen.set_up = true;
                let page = screen.current;
                refresh_page(&mut screen, masters, user, page);
                note_once(
                    &mut screen.seen,
                    "SetupTutorial: the tutorial balloon (balloon_tutorial_mission) needs a tutorial domain this tree has none of; the image stays inactive".to_owned(),
                );
                not_built(
                    &mut screen.seen,
                    "ContinueNextMission (the MYSEKAI_MISSION_RETURN cache the site controllers write)",
                );
                info!("[mission] OnInitComponent: InitializePage, SetupEvent, RefreshScreenView, UpdateCurrentPageStatus, SetupTutorial; the return cache entry is removed");
            }
            ScreenHook::OnWillExit => {
                info!("[mission] OnWillExit: the multiplay UI again (not in this tree); ScreenWillExitDone");
            }
            ScreenHook::OnExited => {
                screen.booted = false;
                screen.set_up = false;
                screen.covering = false;
                screen.turn = None;
                screen.pressed = None;
                info!(
                    "[mission] OnExited: each page disposed (its stamp's FX and the bundle mysekai/mysekai_mission are not loaded here; named)"
                );
            }
            _ => {}
        }
    }
    let current = screens.current_screen() == Some(SCREEN);
    if current && screen.absent.is_some() {
        if !screen.refusal_popped && !screens.is_ui_layer_working() {
            error!(
                "[mission] PushUIScreen(MenuScreenType::MysekaiMission) refused: {}; the screen is popped",
                screen.absent.as_deref().unwrap_or_default()
            );
            layer_commands.write(LayerCommand::Pop);
            screen.refusal_popped = true;
        }
    } else if !current {
        screen.refusal_popped = false;
    }
}

/// Update: the receive's answers, as the screen's callbacks see them.
pub fn follow_receive(
    mut received: MessageReader<MissionReceived>,
    mut failed: MessageReader<MissionReceiveFailed>,
    tables: Option<Res<moly_masters::load::MasterTables>>,
    mut screen: ResMut<MissionScreen>,
) {
    for event in received.read() {
        screen.covering = false;
        let complete = tables
            .as_deref()
            .and_then(|tables| tables.mission.as_ref())
            .is_some_and(|masters| {
                masters
                    .parent_mission(event.sheet_id)
                    .is_some_and(|parent| event.mission_ids.contains(&parent.id))
            });
        info!(
            "[mission] OnFinishReceive(success): {} obtained reward(s); IsCompleteSheet {complete}; the tap cover hides",
            event.obtained_rewards.len()
        );
        not_built(
            &mut screen.seen,
            "ShowRewards (the reward popup, the level-up and rank-up dialogs, OnFinishReceiveDialog)",
        );
    }
    for event in failed.read() {
        screen.covering = false;
        error!(
            "[mission] OnFinishReceive(failure) for missions {:?}: {:?}",
            event.mission_ids, event.refusal
        );
        not_built(&mut screen.seen, "APIExecutor's RetryCancel dialog");
    }
}

/// Update: the turn's steps, the page roots' x and visibility, the current
/// page's completion cell, its stamp and the tap cover.
#[allow(clippy::too_many_arguments)]
pub fn advance(
    time: Res<Time>,
    screens: Res<ScreenManager>,
    layouts: Res<UiLayouts>,
    art: Option<Res<BalloonArt>>,
    mut screen: ResMut<MissionScreen>,
    mut roots: Query<(&mut Visibility, &mut UiPrefabView), With<MissionScreenRoot>>,
) {
    let Ok((mut visibility, mut view)) = roots.single_mut() else {
        return;
    };
    let current = screens.current_screen() == Some(SCREEN);
    let playing = screens.current_state() == ScreenLayerState::Playing;
    let shown = current && screen.set_up && playing;
    visibility.set_if_neq(if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    if !shown || screen.absent.is_some() {
        return;
    }
    let Some(bindings) = screen.bindings.clone() else {
        return;
    };
    // The page's completion cell, which `RefreshScreenView` wrote from the
    // masters and the user's rows (the model the cell and dialog units read).
    let page_fill = page_gauge(&layouts, &bindings.pages[screen.current]);
    let screen = &mut *screen;
    if let Some(mut turn) = screen.turn {
        let done = {
            let MissionScreen {
                offsets, current, ..
            } = &mut *screen;
            step_turn(&mut turn, offsets, current, time.delta_secs())
        };
        screen.turn = (!done).then_some(turn);
        if done {
            info!("[mission] TurnPage: the slide ended; taps enabled");
        }
    }
    for (index, page) in bindings.pages.iter().enumerate() {
        view.set_visible(&page.root, index == screen.current);
        view.set_anchored_position(&page.root, Vec2::new(screen.offsets[index], page.root_y));
    }
    let page = &bindings.pages[screen.current];
    let content = &screen.pages[screen.current];
    if let Some(complete) = &content.complete {
        put_text(&mut view, &page.cell.mission, &complete.mission, art.as_deref());
        put_text(&mut view, &page.cell.progress, &complete.progress, art.as_deref());
        super::cell::set_gauge(&mut view, page_fill.clone(), complete.fill);
        view.set_visible(&page.cell.achieve, complete.achieved);
        match complete.others {
            Some(count) => {
                let text = count.to_string();
                if put_text(&mut view, &page.cell.others, &text, art.as_deref()) {
                    view.set_visible(&page.cell.others, true);
                }
            }
            None => view.set_visible(&page.cell.others, false),
        }
    }
    view.set_visible(&page.stamp, content.stamp);
    view.set_visible(&bindings.tap_cover, screen.covering);
    view.set_visible(&bindings.tutorial_image, false);
}

/// The gauge fill of a page's own cell: the `GaugeFill` below its
/// `UIPartsGauge`, by the Mask above it.
fn page_gauge(layouts: &UiLayouts, page: &PageBinding) -> Option<super::cell::GaugeFill> {
    super::cell::GaugeFill::of_image(layouts.document(KEY)?, &page.cell.fill)
}

/// `put_text` as the convert screen's: a text with a character the shared
/// glyph atlas lacks is skipped, and its node keeps the document's text.
fn put_text(view: &mut UiPrefabView, path: &str, text: &str, art: Option<&BalloonArt>) -> bool {
    if text.is_empty() {
        view.set_text(path, String::new());
        return true;
    }
    let Some(art) = art else {
        return false;
    };
    if let Some(missing) = text
        .chars()
        .find(|ch| !ch.is_whitespace() && art.glyph_cell(*ch).is_none())
    {
        warn!(
            "mission: the text {text:?} at {path} has the character {missing:?}, which the shared glyph atlas lacks; the node keeps the document's text"
        );
        return false;
    }
    view.set_text(path, text.to_owned());
    true
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// The screen's reads of the root's resources, the layer stack and the
/// master tables: one parameter for the input step.
#[derive(SystemParam)]
pub struct InputWorld<'w, 's> {
    pub layouts: Res<'w, UiLayouts>,
    pub screens: Res<'w, ScreenManager>,
    pub dialog: Res<'w, ShellDialogState>,
    pub tables: Option<Res<'w, moly_masters::load::MasterTables>>,
    pub user: Option<Res<'w, ClientMission>>,
    pub cells: Res<'w, super::cell::MissionCells>,
    /// The screen's root view, hit-tested by the source raycast.
    pub roots: Query<'w, 's, &'static UiPrefabView, With<MissionScreenRoot>>,
}

/// The screen's writes: the receive service, the screen's own state, the
/// button sounds, the gesture consumption and the dialog requests. The link
/// the receive is sent on is the service's own write.
#[derive(SystemParam)]
pub struct InputWrites<'w> {
    pub service: ResMut<'w, MysekaiMissionService>,
    pub screen: ResMut<'w, MissionScreen>,
    pub sounds: ResMut<'w, crate::audio::SeRequests>,
    pub consumed: ResMut<'w, crate::action_button::ActionTapConsumed>,
    pub dialogs: MessageWriter<'w, super::cell::MissionDialogRequest>,
    pub link: ResMut<'w, moly_link::ServerLink>,
}

/// The screen's buttons through the source raycast over the screen's view.
/// Every pointer event is the screen's while it is current; its buttons
/// take a press only while it plays, turns no page and shows no tap cover.
pub fn input(
    mut pointers: MessageReader<UiPointerEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut world: InputWorld,
    mut writes: InputWrites,
    root_canvas: Option<Res<RootCanvas>>,
) {
    let InputWrites {
        service,
        screen,
        sounds,
        consumed,
        dialogs,
        link,
    } = &mut writes;
    let screens = &world.screens;
    let dialog = &world.dialog;
    let layouts = &world.layouts;
    let cells = &world.cells;
    let events: Vec<UiPointerEvent> = pointers.read().copied().collect();
    if events.is_empty() || screen.absent.is_some() {
        return;
    }
    let (Ok(window), Some(root_canvas), Ok(view)) =
        (windows.single(), root_canvas.as_deref(), world.roots.single())
    else {
        return;
    };
    let Some(doc) = layouts.document(KEY) else {
        return;
    };
    let Some(bindings) = screen.bindings.clone() else {
        return;
    };
    let canvas = root_canvas.size(window);
    let current_screen = screens.current_screen() == Some(SCREEN);
    let blocked = dialog.blocks_field_input()
        || screens.exists_dialog()
        || screens.current_state() != ScreenLayerState::Playing
        || !screen.set_up
        || screen.turn.is_some()
        || screen.covering;
    let mut clicks: Vec<usize> = Vec::new();
    let mut rows: Vec<usize> = Vec::new();
    for event in events {
        // The screen layer owns every pointer event while it is current.
        if current_screen {
            consumed.0 = true;
        }
        let pointer = crate::ui_inputs::event_pointer(window, root_canvas, event.position);
        let target = view.press_target(layouts, pointer, canvas);
        match event.phase {
            UiPointerPhase::Down => {
                screen.pressed = None;
                if !current_screen || blocked {
                    continue;
                }
                let Some((node, id)) = target else {
                    continue;
                };
                let path = doc.nodes[node].path.clone();
                if bindings.button_role(screen.current, &path, id).is_some()
                    || cells.row_at(screen.current, id).is_some()
                {
                    screen.pressed = Some((path, id));
                }
            }
            UiPointerPhase::Move => {}
            UiPointerPhase::Up | UiPointerPhase::Cancel => {
                let pressed = screen.pressed.take();
                let Some(pressed) = pressed else {
                    continue;
                };
                if event.phase == UiPointerPhase::Cancel {
                    continue;
                }
                let same = match target {
                    Some((node, id)) => doc.nodes[node].path == pressed.0 && id == pressed.1,
                    None => false,
                };
                if blocked || !same {
                    continue;
                }
                layouts.queue_button_se(sounds, KEY, &pressed.0);
                if let Some(role) = bindings.button_role(screen.current, &pressed.0, pressed.1) {
                    clicks.push(role);
                } else if let Some(index) = cells.row_at(screen.current, pressed.1) {
                    rows.push(index);
                }
            }
        }
    }
    let tables = world.tables.as_deref();
    let user = world.user.as_deref();
    for role in clicks {
        on_click(screen, service, link, dialogs, tables, user, role);
    }
    for row in rows {
        on_row_click(screen, service, link, dialogs, tables, user, row);
    }
}

/// A list row's tap: `MysekaiMissionView.OnClickMissionCell` ->
/// `ScreenLayerMysekaiMission.OnClickMysekaiMissionCell`. An achieved normal
/// mission sends its receive; any other row opens the detail dialog.
pub fn on_row_click(
    screen: &mut MissionScreen,
    service: &mut MysekaiMissionService,
    link: &mut moly_link::ServerLink,
    dialogs: &mut MessageWriter<super::cell::MissionDialogRequest>,
    tables: Option<&moly_masters::load::MasterTables>,
    user: Option<&ClientMission>,
    index: usize,
) {
    let (Some(masters), Some(user)) = (
        tables.and_then(|tables| tables.mission.as_ref()),
        user,
    ) else {
        note_once(
            &mut screen.seen,
            "a tap before the mission masters and the user's missions are in: nothing".to_owned(),
        );
        return;
    };
    let Some(model) = read_model(masters, user, screen.sheet) else {
        return;
    };
    let rows = model.rows();
    let Some(row) = rows
        .iter()
        .filter(|row| matches!(row, model::MissionRow::Mission(_)))
        .nth(index)
    else {
        note_once(
            &mut screen.seen,
            "a tap on a list row the page does not hold: nothing".to_owned(),
        );
        return;
    };
    match model.tap(row) {
        CellTap::Receive(id) => send(
            screen,
            service,
            link,
            vec![id],
            "ScreenLayerMysekaiMission.OnClickMysekaiMissionCell",
        ),
        // A normal mission's row is never the sheet's completion: an
        // achieved row receives, every other row opens the detail.
        CellTap::BulkReceive | CellTap::Detail => {
            let id = row.view().master.id;
            dialogs.write(super::cell::MissionDialogRequest::Detail(id));
            info!(
                "[mission] OnClickMysekaiMissionCell: mission {id} opens the detail dialog (Show2ButtonDialog(339))"
            );
        }
    }
}

/// One page-button tap of the current page: `OnClickMysekaiMissionCell` on the
/// page's own (completion) cell, `OnClickMissionListButton` and
/// `OnClickBulkReceiveButton`.
pub fn on_click(
    screen: &mut MissionScreen,
    service: &mut MysekaiMissionService,
    link: &mut moly_link::ServerLink,
    dialogs: &mut MessageWriter<super::cell::MissionDialogRequest>,
    tables: Option<&moly_masters::load::MasterTables>,
    user: Option<&ClientMission>,
    role: usize,
) {
    let (Some(masters), Some(user)) = (
        tables.and_then(|tables| tables.mission.as_ref()),
        user,
    ) else {
        note_once(
            &mut screen.seen,
            "a tap before the mission masters and the user's missions are in: nothing".to_owned(),
        );
        return;
    };
    let Some(model) = read_model(masters, user, screen.sheet) else {
        return;
    };
    match role {
        1 => {
            dialogs.write(super::cell::MissionDialogRequest::List);
            info!(
                "[mission] OnClickMissionListButton: the sheet list dialog (Show1ButtonDialog(340)) opens"
            );
        }
        2 => {
            let ids = model.receivable();
            send(screen, service, link, ids, "ScreenLayerMysekaiMission.OnClickBulkReceiveButton");
        }
        _ => {
            // The page's own cell is the sheet's completion mission
            // (`_completeCell`): its tap is `OnClickMysekaiMissionCell`.
            let rows = model.rows();
            let row = rows
                .iter()
                .find(|row| matches!(row, model::MissionRow::Complete { .. }));
            let Some(row) = row else {
                note_once(
                    &mut screen.seen,
                    "a tap on a page with no completion row: nothing".to_owned(),
                );
                return;
            };
            match model.tap(row) {
                CellTap::BulkReceive => {
                    let ids = model.receivable();
                    send(
                        screen,
                        service,
                        link,
                        ids,
                        "ScreenLayerMysekaiMission.OnClickMysekaiMissionCell (complete sheet)",
                    );
                }
                CellTap::Receive(id) => send(
                    screen,
                    service,
                    link,
                    vec![id],
                    "ScreenLayerMysekaiMission.OnClickMysekaiMissionCell",
                ),
                CellTap::Detail => {
                    let id = row.view().master.id;
                    dialogs.write(super::cell::MissionDialogRequest::Detail(id));
                    info!(
                        "[mission] OnClickMysekaiMissionCell: mission {id} opens the detail dialog (Show2ButtonDialog(339))"
                    );
                }
            }
        }
    }
}

/// `_service.Receive(GetCurrentMissionSheetId(), ids, OnFinishReceive)` and
/// the tap cover on until the answer.
fn send(
    screen: &mut MissionScreen,
    service: &mut MysekaiMissionService,
    link: &mut moly_link::ServerLink,
    ids: Vec<i32>,
    caller: &str,
) {
    if ids.is_empty() {
        info!("[mission] {caller}: the receivable list is empty; nothing is sent");
        return;
    }
    match service.send(link, screen.sheet, ids, false, caller) {
        Ok(_) => {
            screen.covering = true;
            info!("[mission] {caller}: the tap cover shows until the answer");
        }
        Err(reason) => error!("[mission] {reason}"),
    }
}

/// The CustomButton gate's configuration of a control's serialized fields.
#[allow(dead_code)] // Read by the press gate's animation in the cell unit.
pub fn button_config(fields: &Value) -> Option<button_rule::CustomButtonConfig> {
    Some(button_rule::CustomButtonConfig {
        se: button_rule::SeType::from_serialized(fields["se"].as_i64()?)?,
        other_se_name: fields["otherSeName"].as_str()?.to_owned(),
        interval: button_rule::IntervalUseType::from_serialized(fields["interval"].as_i64()?)?,
        absolutely_press: fields["absolutelyPress"].as_bool()?,
        enable_long_press: fields["enableLongPress"].as_bool()?,
        // This root's class serializes the field; a class that does not has
        // no hold repeat.
        enable_hold_repeat: fields
            .get("enableHoldRepeat")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

#[cfg(test)]
mod value_checks {
    use super::*;

    fn turning(step: Turn) -> Turning {
        Turning {
            step,
            clock: TweenClock::new(step.seconds()),
        }
    }

    /// `TurnPage` with 0.1 s frames: the squash and the restore each end on
    /// the second frame (0.2 s >= 0.125 s), the swap runs at the restore's
    /// end (pages: page 2 current at 2580, page 1 at 0), and the slide ends
    /// on the second frame (0.2 s), leaving page 2 at 0 and page 1 at 2580.
    #[test]
    fn the_turn_squashes_restores_swaps_and_slides() {
        let mut turn = turning(Turn::Squash);
        let mut offsets = [0.0, PAGE_TURN_X];
        let mut current = 0;
        assert!(!step_turn(&mut turn, &mut offsets, &mut current, 0.1));
        assert_eq!(turn.step, Turn::Squash);
        assert!(!step_turn(&mut turn, &mut offsets, &mut current, 0.1));
        assert_eq!(turn.step, Turn::Restore);
        assert!(!step_turn(&mut turn, &mut offsets, &mut current, 0.1));
        assert!(!step_turn(&mut turn, &mut offsets, &mut current, 0.1));
        assert_eq!(turn.step, Turn::Slide);
        assert_eq!(current, 1);
        assert_eq!(offsets, [0.0, PAGE_TURN_X]);
        // 0.1 s of 0.2 s OutCubic: Ease 9 with t = -0.5 evaluates to 0.875,
        // so the next page is at 2580 * 0.125 = 322.5.
        assert!(!step_turn(&mut turn, &mut offsets, &mut current, 0.1));
        assert!((offsets[1] - 322.5).abs() < 1e-3, "{}", offsets[1]);
        assert_eq!(offsets[0], 0.0);
        assert!(step_turn(&mut turn, &mut offsets, &mut current, 0.1));
        assert_eq!(offsets, [PAGE_TURN_X, 0.0]);
        assert_eq!(current, 1);
    }

    /// The slide's ends: 2580 at the start, 0 at its end, snapping past it.
    #[test]
    fn the_slide_snaps_to_its_ends() {
        assert_eq!(slide_x(PAGE_TURN_X, 0.0), PAGE_TURN_X);
        assert_eq!(slide_x(PAGE_TURN_X, TURN_SLIDE_SECONDS), 0.0);
        assert_eq!(slide_x(PAGE_TURN_X, 5.0), 0.0);
        assert_eq!(slide_x(0.0, 0.1), 0.0);
    }
}
