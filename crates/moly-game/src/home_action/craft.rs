//! The craft screen's logic: `ScreenLayerMysekaiCraft` (screen 615) and its
//! `CraftPreview`, from the workbench's button to the dialogs after the
//! world sequence.
//!
//! The source's order:
//! - The workbench's action button pushes 615; the pressed workbench is the
//!   craft's target (the source's action data), handed over as
//!   [`CraftWorkbench`]. `OnInitComponent` sets up the content list and the
//!   preview of the selected blueprint and plays `se_enter_crafttool`.
//! - `CraftPreview.SetupApplyButton`: the button reads
//!   `WORD_GOTO_SELECT_MEMBER` for a canvas blueprint, else
//!   `WORD_PRODUCTION`. It is disabled, with its balloon, by the first of: a
//!   blueprint that is not a tool while the fixture total is at the
//!   possession limit (`IsTotalFixtureCreateCountLimit`) ->
//!   `WORD_TOTAL_FURNITURE_IS_CREATED`; without colour cells,
//!   `IsCreateCountLimit(texture 1)` -> `WORD_THIS_FURNITURE_IS_CREATED`; with
//!   them, `IsCreateCountLimit(selected colour)` ->
//!   `WORD_THIS_FURNITURE_IS_CREATED` for one cell or every colour created
//!   (`IsAllColorCreated`: texture 1 and every other colour of the fixture at
//!   the limit), else `WORD_THIS_FURNITURE_IS_CREATED_COLOR`; then
//!   `IsReachedPossessionLimitToCraft` -> `MSG_CRAFT_REACHED_POSSESSION_LIMIT`;
//!   last `MaterialCostList.IsEnoughMaterialToCraft` ->
//!   `MSG_CRAFT_NOT_ENOUGH_MATERIAL`. The tutorial check is left out (the
//!   tutorial is out of scope).
//! - `OnClickApplyButton`: a blueprint with a term row outside the current
//!   time (`TimeUtility.IsWithinTime`) shows `Common1ButtonDialog(
//!   MSG_CRAFT_TERM_LIMIT_DIALOG_BODY, WORD_OK)`, whose OK goes back
//!   (`BackUIScreen`); a canvas blueprint goes to the member select (the
//!   canvas screen 640); any other opens the confirm,
//!   `Show2ButtonDialog(Common2ButtonDialog, null, WORD_OK, WORD_CANCEL)` with
//!   the body `GetFormat(WORD_CREATE_CONFIRM, target name, count)` (the
//!   fixture's or the tool's name; any other type throws in the source).
//! - The confirm's OK runs `SendCraftApiAsync`: `IsFirstCraft`, the apply
//!   button off, the term check again, `HideBackUIScreen`,
//!   `SetNoticeWait(true)`, the total experience before, the texture
//!   (`GetSelectedMysekaiColorId`: a tool none, a fixture without other
//!   colours 1, else the selected cell's colour), the fixture bonus data (the
//!   first craft of a fixture with a bonus), then `MysekaiCraftService.Execute(
//!   blueprint, count, texture, card none, special training none)`. Its
//!   success starts `OnCraftExecuted`: the home action's craft sequence
//!   ([`super::HomeActionRequest::Craft`] with `posted`, so the sequence sends
//!   no second request).
//! - After the sequence's hold: `OpenCreateResponseDialogAsync`
//!   (CraftResultSubWindowDialog, 377: `WORD_FIXTURE_CREATE_RESULT` from the
//!   fixture lists, else `WORD_TOOL_CREATE_RESULT`; awaited to its close,
//!   whose `OnCloseCreateResponseDialog` sets the preview up again), the
//!   fixture bonus dialog (405) when the bonus data exists, and
//!   `MysekaiRankUtility.OpenMysekaiRankDialogIfNeededAsync(before, after)`:
//!   nothing when the two experiences give one rank, else the rank-up dialog
//!   (335) and then the rank's release dialogs. Then the sequence closes
//!   (gate, Idle, fades, camera).
//! - Common1's back key and outside tap answer OK; Common2's answer cancel
//!   (their `OnHardwareBackKeyProcess` and `OnCloseExternal`).
//!
//! - The Craft list (`CraftContentList`, content type Craft): the master
//!   blueprints `UserResourceFactory.GetUserBluePrints(Craft)` keeps
//!   (`CheckEnableBluePrint`: a fixture or canvas blueprint without a term
//!   row, available without possession or owned; none while the user has no
//!   blueprint rows), and `SelectFirstItem` selects its first row when the
//!   screen opens. A tap on a cell (`OnClickCell`) selects its row and sets
//!   the preview up for it. The view is [`view`].
//!
//! Named gaps (each logged where it is met):
//! - The tab cells (their prefab is not exported): only the Craft list is
//!   listed; the Tool and LimitedCraft lists are not reachable. The sort
//!   dropdown, the genre tabs, the search button and the hashtag balloon are
//!   not built: the rows keep master order.
//! - The count selector is not built: the count is 1 unless the instrument
//!   names one.
//! - The colour cells are the 3D preview's colours (`GetColors`), and the 3D
//!   preview is not built: the cells here are the fixture master's colour ids
//!   (texture 1 and its other colours' textures, ascending), cell 0 selected
//!   as `SelectCellImmediate(0)`. A tool has no cells (the source keeps the
//!   previous fixture's cells on a tool; the first selection has none).
//! - The result dialog's view is [`result`] (a root without its document
//!   shows it without a view, named and closed); the fixture bonus and rank
//!   dialogs have no view in this build: each is shown through the screen
//!   manager, named and closed.
//! - A first craft is refused by name while its bonus row
//!   (`craft_mysekai_fixture_first_bonus` of the rank experience master) is
//!   not carried; the fixture bonus table
//!   (`MasterMysekaiFixtureGameCharacterGroupPerformanceBonus`) is not
//!   carried, so no bonus dialog shows.
//! - The rank dialog's ranks are `MysekaiRankModel(totalExp)`'s over the
//!   rank table (`mysekaiRanks`, the master layer) for the total experience
//!   before and after; a region whose table is not served names it and
//!   shows no rank dialog.
//! - The canvas screen (640) is not built: a canvas blueprint's member select
//!   is refused by name.
//!
//! Instruments (off by default; game mode reads none): `MOLY_CRAFT_SELECT`
//! = `blueprint[xcount]` selects when the screen opens;
//! `MOLY_CRAFT_APPLY_SECS` presses the apply button that many seconds after
//! the screen opens; `MOLY_CRAFT_CONFIRM_SECS` taps the confirm's OK button
//! (at its drawn rect) that many seconds after the confirm shows;
//! `MOLY_CRAFT_STAGE_RESULT=<count>[,first]` stages the dialogs after the
//! hold two seconds after the screen opens, for the selected blueprint, with
//! no craft request and no sequence (the result dialog's view without a
//! reachable craft; loud in the log).

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::master::{MasterData, MasterTable};
use moly_law::ui::mysekai_rank::{MasterMysekaiRank, get_mysekai_rank};

use crate::action_button::ActionTapConsumed;
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::fixture_activity_state::FixtureTarget;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::server::client::craft::{
    CRAFT_COUNT_MAX, CRAFT_COUNT_MIN, CraftMasters, CraftOwned, MasterBlueprint, MysekaiCraftType,
    UserMysekaiCraftRequest, is_create_count_limit, is_enough_material_to_craft, is_first_craft,
    is_reached_possession_limit_to_craft, post_craft,
};
use crate::server::client::instrument_env;
use crate::server::client::inventory::{
    ClientMysekaiInventory, PossessionMasters, fixture_total_count,
};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, LayerCommand,
    MenuScreenType, ScreenManager,
};
use crate::ui_layout::{UiLayouts, UiPrefabView};

pub(crate) mod result;
pub(crate) mod view;

/// `DialogType.CraftResultSubWindowDialog`.
const CRAFT_RESULT_DIALOG: DialogType = DialogType(377);
/// `CraftResultSubWindowDialog.Open`'s cue.
const SE_CRAFT_RESULT: &str = "se_craft_result";
/// The dialogs' views sit above the screens' (a screen view is at z 20 and
/// its nodes add their order).
const DIALOG_Z: f32 = 100.0;
/// `DialogType.FixtureBonusSubWindowDialog`.
const FIXTURE_BONUS_DIALOG: DialogType = DialogType(405);
/// `DialogType.MysekaiRankUpDialog`.
const RANK_UP_DIALOG: DialogType = DialogType(335);
const SE_ENTER_CRAFT_TOOL: &str = "se_enter_crafttool";
const COMMON1: &str = "Common1";
const COMMON2: &str = "Common2";
const FOOTER_BUTTON: &str = "WindowRoot/FooterButtons/UIPartsCommonButton";
const FOOTER_TEXT: &str = "WindowRoot/FooterButtons/UIPartsCommonButton/Text";
const MESSAGE_BODY: &str = "ContentRoot/Content/MessageBody";
const CLOSE_BUTTON: &str = "WindowRoot/UIPartsCloseButton";
const WINDOW_ROOT: &str = "WindowRoot";
const TABS: &str = "WindowRoot/Tabs";
/// The wordings the craft screen and its two common dialogs draw (the shell
/// charset takes them).
pub(crate) const WORDINGS: &[&str] = &[
    "WORD_CREATE_CONFIRM",
    "WORD_OK",
    "WORD_CANCEL",
    "MSG_CRAFT_TERM_LIMIT_DIALOG_BODY",
    // The screen's apply button, its balloon and the preview's texts.
    "WORD_PRODUCTION",
    "WORD_GOTO_SELECT_MEMBER",
    "WORD_TOTAL_FURNITURE_IS_CREATED",
    "WORD_THIS_FURNITURE_IS_CREATED",
    "WORD_THIS_FURNITURE_IS_CREATED_COLOR",
    "MSG_CRAFT_REACHED_POSSESSION_LIMIT",
    "MSG_CRAFT_NOT_ENOUGH_MATERIAL",
    "FORMAT_MYSEKAI_SITE_LAYOUT_SIZE",
    "WORD_THREE_HYPHEN",
    // The result dialog.
    "WORD_FIXTURE_CREATE_RESULT",
    "WORD_TOOL_CREATE_RESULT",
    "WORD_MULTIPLY_FORMAT",
    "WORD_FIRST_CRAFT_BONUS",
];

/// The workbench whose button opened the craft screen (the source's action
/// data); the action button sets it when it pushes 615.
#[derive(Resource, Clone, Debug)]
pub(crate) struct CraftWorkbench(pub(crate) FixtureTarget);

/// The selected blueprint and what the preview derived from it.
#[derive(Clone, Debug)]
struct Selection {
    blueprint: MasterBlueprint,
    count: i32,
    /// The target's name (`MasterMysekaiFixture.name` or
    /// `MasterMysekaiTool.name`).
    name: String,
    /// The colour cells (none for a tool) and the selected cell.
    colors: Vec<i32>,
    selected_color: usize,
}

impl Selection {
    /// `GetSelectedMysekaiColorId`: none for a tool; the selected cell's
    /// colour for a fixture with other colours; else 1.
    fn texture(&self) -> Option<i32> {
        match self.blueprint.craft_type {
            MysekaiCraftType::MysekaiTool => None,
            _ if self.colors.iter().any(|color| *color != 1) => {
                self.colors.get(self.selected_color).copied()
            }
            _ => Some(1),
        }
    }
}

/// What the craft carries into the sequence and its dialogs.
#[derive(Clone, Debug)]
struct Executed {
    before_exp: Option<i32>,
    is_first_craft: bool,
    /// The fixture whose bonus the bonus dialog shows.
    bonus_fixture: Option<i32>,
    craft_type: MysekaiCraftType,
    count: i32,
    name: String,
}

#[derive(Clone, Copy, Debug)]
enum DialogStep {
    /// The result dialog; `shown`: awaited to its close.
    Result {
        shown: Option<DialogId>,
    },
    Bonus,
    Rank,
}

#[derive(Clone, Debug, Default)]
enum Stage {
    #[default]
    Idle,
    /// The confirm (Common2ButtonDialog) is shown.
    Confirm { dialog: DialogId, body: String },
    /// The term limit dialog (Common1ButtonDialog) is shown.
    TermLimit { dialog: DialogId },
    /// The world sequence plays.
    Executing(Executed),
    /// The sequence's hold ended: the dialogs, in order.
    Dialogs {
        executed: Executed,
        after_exp: Option<i32>,
        next: DialogStep,
    },
    /// The dialogs are done; the sequence closes.
    Done,
}

/// The apply button's state (`SetupApplyButton`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ApplyButton {
    /// `WORD_GOTO_SELECT_MEMBER` or `WORD_PRODUCTION`.
    label: &'static str,
    /// `customButton.enabled`.
    enabled: bool,
    /// The cannot-craft balloon's wording (`SetCantCraftBalloon`); `None`:
    /// the balloon is off.
    balloon: Option<&'static str>,
}

/// A tap the view hands over.
#[derive(Clone, Copy, Debug)]
enum ViewRequest {
    /// `OnClickCell(index)`.
    Select(usize),
    /// `OnClickApplyButton` (the button is enabled).
    Apply,
}

/// The craft screen's state.
#[derive(Resource, Default)]
pub(crate) struct CraftScreen {
    open: bool,
    opened_at: f32,
    /// The Craft list's rows, master order.
    rows: Vec<MasterBlueprint>,
    /// Bumped whenever the rows are read again (the view recomposes).
    rows_revision: u64,
    /// The selected row (`_selectIndex`).
    selected: Option<usize>,
    selection: Option<Selection>,
    button: Option<ApplyButton>,
    requests: Vec<ViewRequest>,
    stage: Stage,
    shown_at: f32,
    /// The confirm's or the term dialog's answer (true: OK).
    answer: Option<bool>,
    /// The instruments already fired this opening.
    applied: bool,
    confirm_tapped: bool,
    rank_reported: bool,
    /// The rank table (`None` until it resolves; `Err`: why the region has
    /// none).
    ranks: Option<Result<Vec<MasterMysekaiRank>, String>>,
    /// The shown result dialog's data (its view draws it).
    result: Option<result::ResultShown>,
    /// The dialogs were staged by the instrument (no sequence closes them).
    staged: bool,
    staged_fired: bool,
}

/// The two common dialogs' views and their buttons. Common2's OK, cancel and
/// close buttons are [`crate::two_button_dialog::resolve`]'s (the footer
/// buttons found by their serialized labels); Common1's single footer button
/// is its OK (the dialogs' button fields are not decoded).
#[derive(Component)]
pub(crate) struct CommonDialogView {
    key: &'static str,
    /// Common2: `[cancel, OK]`; Common1: `[OK]`.
    buttons: Vec<String>,
    close: String,
}

// ---------------------------------------------------------------------------
// The home action sequence's side
// ---------------------------------------------------------------------------

/// The craft sequence's hold ended (`OnCraftExecuted` after its wait): read
/// the experience after and start the dialogs.
pub(super) fn sequence_end(world: &mut World) {
    let gamedata = world
        .get_resource::<ClientMysekaiInventory>()
        .and_then(ClientMysekaiInventory::gamedata);
    let mut screen = world.resource_mut::<CraftScreen>();
    match std::mem::take(&mut screen.stage) {
        Stage::Executing(executed) => {
            info!(
                "[craft] OnCraftExecuted: totalExp {:?} -> {:?}; OpenCreateResponseDialogAsync",
                executed.before_exp,
                gamedata.map(|g| g.total_exp)
            );
            screen.stage = Stage::Dialogs {
                executed,
                after_exp: gamedata.map(|g| g.total_exp),
                next: DialogStep::Result { shown: None },
            };
        }
        other => {
            warn!("[craft] the craft sequence ended outside a craft ({other:?}); no dialogs");
            screen.stage = Stage::Done;
        }
    }
}

/// The dialogs after the hold are done (the sequence may close).
pub(super) fn dialogs_done(world: &World) -> bool {
    world
        .get_resource::<CraftScreen>()
        .is_none_or(|screen| matches!(screen.stage, Stage::Done | Stage::Idle))
}

/// The sequence closed, or was cancelled: the screen is idle again.
pub(super) fn sequence_closed(world: &mut World, cancelled: bool) {
    let mut screen = world.resource_mut::<CraftScreen>();
    if matches!(
        screen.stage,
        Stage::Executing(_) | Stage::Dialogs { .. } | Stage::Done
    ) {
        if cancelled {
            warn!("[craft] the craft sequence was cancelled: its dialogs do not show");
        }
        screen.stage = Stage::Idle;
    }
}

// ---------------------------------------------------------------------------
// The preview's checks
// ---------------------------------------------------------------------------

fn owned<'a>(
    inventory: &'a ClientMysekaiInventory,
    possession: Option<&PossessionMasters>,
) -> CraftOwned<'a> {
    CraftOwned {
        materials: inventory.materials(),
        fixtures: inventory.fixtures(),
        canvases: inventory.canvases(),
        fixture_max_count: possession.and_then(|masters| inventory.fixture_max_count(masters)),
    }
}

/// `SetupApplyButton`: `None` when the button is enabled, else its balloon's
/// wording key. `Err` names a missing input.
fn apply_state(
    selection: &Selection,
    masters: &CraftMasters,
    owned: &CraftOwned,
) -> Result<Option<&'static str>, String> {
    let blueprint = &selection.blueprint;
    if blueprint.craft_type != MysekaiCraftType::MysekaiTool {
        // `IsTotalFixtureCreateCountLimit`: the limit less the total is
        // below 1.
        let max = owned.fixture_max_count.ok_or(
            "the fixture possession limit of the user's level is not in the possession master",
        )?;
        if max - fixture_total_count(owned.fixtures) < 1 {
            return Ok(Some("WORD_TOTAL_FURNITURE_IS_CREATED"));
        }
    }
    if selection.colors.is_empty() {
        if is_create_count_limit(blueprint, 1, owned)? {
            return Ok(Some("WORD_THIS_FURNITURE_IS_CREATED"));
        }
    } else {
        let texture = selection.colors[selection.selected_color];
        if is_create_count_limit(blueprint, texture, owned)? {
            let others: Vec<i32> = selection
                .colors
                .iter()
                .copied()
                .filter(|color| *color != 1)
                .collect();
            let mut all = is_create_count_limit(blueprint, 1, owned)? && !others.is_empty();
            for color in others {
                if !all {
                    break;
                }
                all = is_create_count_limit(blueprint, color, owned)?;
            }
            return Ok(Some(if selection.colors.len() == 1 || all {
                "WORD_THIS_FURNITURE_IS_CREATED"
            } else {
                "WORD_THIS_FURNITURE_IS_CREATED_COLOR"
            }));
        }
        if is_reached_possession_limit_to_craft(blueprint, texture, owned)? {
            return Ok(Some("MSG_CRAFT_REACHED_POSSESSION_LIMIT"));
        }
    }
    let costs = masters
        .blueprint_costs(blueprint.id)
        .ok_or_else(|| format!("blueprint {} has no material cost rows", blueprint.id))?;
    if !is_enough_material_to_craft(costs, selection.count, false, owned.materials) {
        return Ok(Some("MSG_CRAFT_NOT_ENOUGH_MATERIAL"));
    }
    Ok(None)
}

/// `SetupApplyButton`, logged.
fn setup_apply_button(world: &World, selection: &Selection) -> ApplyButton {
    let label = if selection.blueprint.craft_type == MysekaiCraftType::MysekaiCanvas {
        "WORD_GOTO_SELECT_MEMBER"
    } else {
        "WORD_PRODUCTION"
    };
    let (Some(masters), Some(inventory)) = (
        world.get_resource::<CraftMasters>(),
        world.get_resource::<ClientMysekaiInventory>(),
    ) else {
        error!(
            "[craft] SetupApplyButton: the craft masters or the client copy of the user's tables are not installed; the button stays off"
        );
        return ApplyButton {
            label,
            enabled: false,
            balloon: None,
        };
    };
    let possession = world.get_resource::<PossessionMasters>();
    match apply_state(selection, masters, &owned(inventory, possession)) {
        Ok(None) => {
            info!(
                "[craft] SetupApplyButton: blueprint {} ({} {} \"{}\") x{} texture {:?}: button {label} on",
                selection.blueprint.id,
                selection.blueprint.craft_type.name(),
                selection.blueprint.craft_target_id,
                crate::balloon::ascii_or(&selection.name),
                selection.count,
                selection.texture()
            );
            ApplyButton {
                label,
                enabled: true,
                balloon: None,
            }
        }
        Ok(Some(balloon)) => {
            info!(
                "[craft] SetupApplyButton: blueprint {} x{} texture {:?}: button {label} off, balloon {balloon}",
                selection.blueprint.id,
                selection.count,
                selection.texture()
            );
            ApplyButton {
                label,
                enabled: false,
                balloon: Some(balloon),
            }
        }
        Err(reason) => {
            error!(
                "[craft] SetupApplyButton: blueprint {}: {reason}; the button stays off",
                selection.blueprint.id
            );
            ApplyButton {
                label,
                enabled: false,
                balloon: None,
            }
        }
    }
}

/// `UserResourceFactory.GetUserBluePrints(Craft)`: the master blueprints
/// `CheckEnableBluePrint(master, Craft)` keeps, master order: a fixture
/// blueprint (the list's target type) or a canvas blueprint (any list but
/// Tool), without a term row (a term row belongs to the LimitedCraft list),
/// available without possession or owned. None while the user has no
/// blueprint rows (the source returns null).
fn craft_rows(world: &World) -> Result<Vec<MasterBlueprint>, String> {
    let masters = world
        .get_resource::<CraftMasters>()
        .ok_or("the craft masters are not installed")?;
    let inventory = world
        .get_resource::<ClientMysekaiInventory>()
        .ok_or("no client copy of the user's tables")?;
    if inventory.blueprints().is_empty() {
        return Ok(Vec::new());
    }
    let blueprints = masters
        .blueprints
        .as_ref()
        .ok_or("the blueprint master (mysekai-blueprints.json) is absent")?;
    let terms = masters
        .terms
        .as_ref()
        .ok_or("the blueprint term master (mysekai-blueprint-terms.json) is absent")?;
    Ok(blueprints
        .values()
        .filter(|blueprint| {
            matches!(
                blueprint.craft_type,
                MysekaiCraftType::MysekaiFixture | MysekaiCraftType::MysekaiCanvas
            ) && !terms
                .iter()
                .any(|term| term.mysekai_blueprint_id == blueprint.id)
                && (blueprint.is_available_without_possession
                    || inventory.has_blueprint(blueprint.id))
        })
        .copied()
        .collect())
}

/// `SelectContent` for a row: the selection, the preview and its button.
fn select_row(world: &World, screen: &mut CraftScreen, index: usize, count: i32) {
    let Some(blueprint) = screen.rows.get(index).copied() else {
        return;
    };
    screen.selected = Some(index);
    match select(world, blueprint.id, count) {
        Ok(selection) => {
            info!(
                "[craft] SelectContent: row {index}, SetupCraftPreview(blueprint {})",
                blueprint.id
            );
            screen.button = Some(setup_apply_button(world, &selection));
            screen.selection = Some(selection);
        }
        Err(reason) => {
            error!(
                "[craft] SetupCraftPreview({}): {reason}; the preview is empty",
                blueprint.id
            );
            screen.selection = None;
            screen.button = None;
        }
    }
}

/// The selection the instrument names (`MOLY_CRAFT_SELECT=blueprint[xcount]`).
fn select_instrument() -> Option<(i32, i32)> {
    let raw = instrument_env("MOLY_CRAFT_SELECT")?;
    let raw = raw.trim();
    match raw.split_once('x') {
        Some((blueprint, count)) => {
            Some((blueprint.trim().parse().ok()?, count.trim().parse().ok()?))
        }
        None => Some((raw.parse().ok()?, 1)),
    }
}

fn seconds_instrument(name: &str) -> Option<f32> {
    instrument_env(name).and_then(|raw| raw.trim().parse::<f32>().ok())
}

/// `SetupCraftPreview(blueprint)`: the preview's data for a selection.
fn select(world: &World, blueprint_id: i32, count: i32) -> Result<Selection, String> {
    let masters = world
        .get_resource::<CraftMasters>()
        .ok_or("the craft masters are not installed")?;
    let blueprint = *masters
        .blueprint(blueprint_id)
        .ok_or_else(|| format!("blueprint {blueprint_id} is not in the blueprint master"))?;
    let tool = match blueprint.craft_type {
        MysekaiCraftType::MysekaiTool => true,
        MysekaiCraftType::MysekaiFixture | MysekaiCraftType::MysekaiCanvas => false,
        MysekaiCraftType::Material => {
            return Err(format!(
                "blueprint {blueprint_id} crafts a material; the craft lists hold fixture, tool and canvas blueprints"
            ));
        }
    };
    let (name, colors) = world
        .get_resource::<crate::get_resource::GetResourceInputs>()
        .ok_or("the acquisition module's masters are not installed")?
        .craft_target(tool, blueprint.craft_target_id)?;
    Ok(Selection {
        blueprint,
        count: count.clamp(CRAFT_COUNT_MIN, CRAFT_COUNT_MAX),
        name,
        colors,
        selected_color: 0,
    })
}

// ---------------------------------------------------------------------------
// The dialogs
// ---------------------------------------------------------------------------

fn show(world: &mut World, dialog: DialogType, caller: &'static str) -> Option<DialogId> {
    let mut screens = world.resource_mut::<ScreenManager>();
    match screens.show_dialog(
        dialog,
        DisplayLayerType::LayerDialog,
        DialogBackKey::Close,
        caller,
    ) {
        Ok(id) => {
            screens.open_dialog(id);
            screens.dialog_open_finished(id);
            Some(id)
        }
        Err(reason) => {
            error!("[craft] {reason}");
            None
        }
    }
}

fn close(world: &mut World, id: DialogId) {
    let mut screens = world.resource_mut::<ScreenManager>();
    if screens.dialogs().any(|(shown, ..)| shown == id) {
        screens.close_dialog(id);
        screens.dialog_destroyed(id);
    }
}

/// A dialog without a view: shown, named with what it would show, closed.
fn show_viewless(world: &mut World, dialog: DialogType, caller: &'static str, detail: String) {
    if let Some(id) = show(world, dialog, caller) {
        error!("[craft] {dialog:?} {id:?} has no view in this build ({detail}): closed");
        close(world, id);
    }
}

/// `HasMysekaiBlueprintTerm` and not `IsWithinCurrentTime(startAt, endAt)`.
fn term_limited(world: &World, selection: &Selection) -> bool {
    let Some(term) = world
        .get_resource::<CraftMasters>()
        .and_then(|masters| masters.term(selection.blueprint.id).copied())
    else {
        return false;
    };
    let Some(user) = world.get_resource::<crate::server::ClientUserData>() else {
        error!(
            "[craft] the blueprint's term is read before the server's first response: taken as outside it"
        );
        return true;
    };
    let now = user.current_timestamp(world.resource::<Time<Real>>().elapsed_secs());
    !crate::server::craft::is_within_time(now, term.start_at, term.end_at)
}

fn open_term_limit(world: &mut World, screen: &mut CraftScreen) {
    if let Some(dialog) = show(
        world,
        DialogType::Common1ButtonDialog,
        "DialogUtility.ShowCommon1ButtonDialog",
    ) {
        info!(
            "[craft] the blueprint's term is over: ShowCommon1ButtonDialog(MSG_CRAFT_TERM_LIMIT_DIALOG_BODY, WORD_OK) {dialog:?}"
        );
        screen.stage = Stage::TermLimit { dialog };
        screen.shown_at = world.resource::<Time>().elapsed_secs();
        screen.answer = None;
    }
}

/// `OnClickApplyButton` (the button is on).
fn click_apply(world: &mut World, screen: &mut CraftScreen) {
    let Some(selection) = screen.selection.clone() else {
        warn!("[craft] OnClickApplyButton: no blueprint selected (the list view is not built)");
        return;
    };
    let button = setup_apply_button(world, &selection);
    screen.button = Some(button);
    if !button.enabled {
        info!("[craft] OnClickApplyButton: the button is off; the press does nothing");
        return;
    }
    if term_limited(world, &selection) {
        open_term_limit(world, screen);
        return;
    }
    if selection.blueprint.craft_type == MysekaiCraftType::MysekaiCanvas {
        error!(
            "[craft] OnClickMemberSelectButton: PushUIScreen(MysekaiCanvas 640) for blueprint {} is refused: the canvas screen is not built",
            selection.blueprint.id
        );
        return;
    }
    // `OpenCraftConfirmDialog`.
    let Some(format) = world
        .resource::<UiLayouts>()
        .wordings
        .get("WORD_CREATE_CONFIRM")
        .cloned()
    else {
        error!(
            "[craft] OpenCraftConfirmDialog: WORD_CREATE_CONFIRM is not on the root; the confirm is refused"
        );
        return;
    };
    let body = match moly_law::text::custom_text_mesh::format_wording(
        &format,
        &[selection.name.clone(), selection.count.to_string()],
    ) {
        Ok(body) => body,
        Err(reason) => {
            error!(
                "[craft] OpenCraftConfirmDialog: WORD_CREATE_CONFIRM: {reason}; the confirm is refused"
            );
            return;
        }
    };
    if let Some(dialog) = show(
        world,
        DialogType::Common2ButtonDialog,
        "CraftPreview.OpenCraftConfirmDialog",
    ) {
        info!(
            "[craft] OpenCraftConfirmDialog: Show2ButtonDialog(Common2ButtonDialog, null, WORD_OK, WORD_CANCEL) {dialog:?}, body GetFormat(WORD_CREATE_CONFIRM, name, {})",
            selection.count
        );
        screen.stage = Stage::Confirm { dialog, body };
        screen.shown_at = world.resource::<Time>().elapsed_secs();
        screen.answer = None;
        screen.confirm_tapped = false;
    }
}

/// `SendCraftApiAsync` (the confirm's OK).
fn send(world: &mut World, screen: &mut CraftScreen) {
    let Some(selection) = screen.selection.clone() else {
        error!("[craft] SendCraftApiAsync: no blueprint selected");
        return;
    };
    let Some(workbench) = world.get_resource::<CraftWorkbench>().cloned() else {
        error!(
            "[craft] SendCraftApiAsync: no pressed workbench was handed over; the craft is refused"
        );
        return;
    };
    let (Some(masters), Some(inventory)) = (
        world.get_resource::<CraftMasters>().cloned(),
        world.get_resource::<ClientMysekaiInventory>().cloned(),
    ) else {
        error!(
            "[craft] SendCraftApiAsync: the craft masters or the client copy of the user's tables are not installed"
        );
        return;
    };
    let possession = world.get_resource::<PossessionMasters>().cloned();
    let first = is_first_craft(
        &selection.blueprint,
        &owned(&inventory, possession.as_ref()),
    );
    info!("[craft] SendCraftApiAsync: IsFirstCraft {first}; the apply button off");
    if term_limited(world, &selection) {
        open_term_limit(world, screen);
        return;
    }
    if first && !matches!(masters.first_craft_bonus, Some(Some(_))) {
        error!(
            "[craft] SendCraftApiAsync: blueprint {} is a first craft, and its bonus row (craft_mysekai_fixture_first_bonus of the rank experience master) is not carried: refused",
            selection.blueprint.id
        );
        screen.button = Some(setup_apply_button(world, &selection));
        return;
    }
    info!(
        "[craft] HeaderUtility.HideBackUIScreen, MysekaiMissionUtility.SetNoticeWait(true): no counterpart"
    );
    let gamedata = inventory.gamedata();
    if first && selection.blueprint.craft_type == MysekaiCraftType::MysekaiFixture {
        error!(
            "[craft] MysekaiFixtureUtility.HasFixtureBonus({}): the fixture bonus master (MasterMysekaiFixtureGameCharacterGroupPerformanceBonus) is not carried; no bonus data",
            selection.blueprint.craft_target_id
        );
    }
    let request = UserMysekaiCraftRequest {
        blueprint_id: selection.blueprint.id,
        texture_id: selection.texture(),
        card_id: None,
        is_special_training: None,
        quantity: selection.count,
    };
    info!("[craft] MysekaiCraftService.Execute: PostUserMysekaiCraftApi {request:?}");
    let reply = post_craft(world, request);
    if !reply.success {
        error!(
            "[craft] PostUserMysekaiCraftApi refused ({}): the API error shows; the preview is set up again",
            reply.refusal.as_deref().unwrap_or("no reason")
        );
        screen.button = Some(setup_apply_button(world, &selection));
        return;
    }
    let updated = &reply.updated;
    info!(
        "[craft] PostUserMysekaiCraftApi succeeded: materials {} rows, fixtures of {}: {:?}, gamedata {:?}",
        updated.user_mysekai_materials.as_ref().map_or(0, Vec::len),
        selection.blueprint.craft_target_id,
        updated.user_mysekai_fixtures.as_ref().map(|rows| rows
            .iter()
            .filter(|row| row.mysekai_fixture_id == selection.blueprint.craft_target_id)
            .map(|row| (row.texture_id, row.quantity))
            .collect::<Vec<_>>()),
        updated.user_mysekai_gamedata
    );
    screen.stage = Stage::Executing(Executed {
        before_exp: gamedata.map(|g| g.total_exp),
        is_first_craft: first,
        bonus_fixture: None,
        craft_type: selection.blueprint.craft_type,
        count: selection.count,
        name: selection.name.clone(),
    });
    world.write_message(super::HomeActionRequest::Craft {
        fixture: workbench.0,
        posted: true,
    });
}

/// The rank table the rank dialog's ranks read.
const RANKS: MasterTable<Vec<MasterMysekaiRank>> = MasterTable {
    table: "mysekaiRanks",
    name: "mysekaiRanks (the craft's rank dialog)",
    parse: crate::mysekai_rank::parse_ranks,
};

/// Startup: request the rank table.
pub(crate) fn request_masters(mut masters: ResMut<MasterData>) {
    masters.request(&RANKS);
}

/// `OnCloseCreateResponseDialog`: the preview is set up again; the fixture
/// bonus dialog is next.
fn result_closed(world: &mut World, screen: &mut CraftScreen) -> Option<DialogStep> {
    info!(
        "[craft] OnCloseCreateResponseDialog: the preview is set up again; UpdateHarvestUser: no counterpart"
    );
    if let Some(selection) = screen.selection.clone() {
        screen.button = Some(setup_apply_button(world, &selection));
    }
    Some(DialogStep::Bonus)
}

/// The instrument's staged dialogs (`MOLY_CRAFT_STAGE_RESULT`): the dialogs
/// after the hold for the selected blueprint, without a craft.
fn stage_instrument(world: &mut World, screen: &mut CraftScreen, now: f32) {
    if screen.staged_fired || !screen.open || !matches!(screen.stage, Stage::Idle) {
        return;
    }
    let Some(raw) = instrument_env("MOLY_CRAFT_STAGE_RESULT") else {
        return;
    };
    if now - screen.opened_at < 2.0 {
        return;
    }
    screen.staged_fired = true;
    let mut parts = raw.trim().split(',');
    let count = parts
        .next()
        .and_then(|count| count.trim().parse::<i32>().ok())
        .unwrap_or(1);
    let first = parts.next().is_some_and(|flag| flag.trim() == "first");
    let Some(selection) = screen.selection.clone() else {
        error!(
            "[craft] instrument: MOLY_CRAFT_STAGE_RESULT with no blueprint selected: not staged"
        );
        return;
    };
    let gamedata = world
        .get_resource::<ClientMysekaiInventory>()
        .and_then(ClientMysekaiInventory::gamedata);
    warn!(
        "[craft] instrument: the dialogs after the hold are staged for blueprint {} x{count} (first craft {first}) with no craft request and no sequence",
        selection.blueprint.id
    );
    screen.staged = true;
    screen.stage = Stage::Dialogs {
        executed: Executed {
            before_exp: gamedata.map(|g| g.total_exp),
            is_first_craft: first,
            bonus_fixture: None,
            craft_type: selection.blueprint.craft_type,
            count,
            name: selection.name.clone(),
        },
        after_exp: gamedata.map(|g| g.total_exp),
        next: DialogStep::Result { shown: None },
    };
}

/// The dialogs after the hold, one step per frame.
fn advance_dialogs(world: &mut World, screen: &mut CraftScreen) {
    let Stage::Dialogs {
        executed,
        after_exp,
        next,
    } = screen.stage.clone()
    else {
        return;
    };
    let next = match next {
        DialogStep::Result { shown: None } => {
            let body = if executed.craft_type == MysekaiCraftType::MysekaiTool {
                "WORD_TOOL_CREATE_RESULT"
            } else {
                "WORD_FIXTURE_CREATE_RESULT"
            };
            let detail = format!(
                "{body}; name \"{}\", count {}, totalExp {:?} -> {:?}, first craft {}",
                crate::balloon::ascii_or(&executed.name),
                executed.count,
                executed.before_exp,
                after_exp,
                executed.is_first_craft
            );
            let caller = "CraftPreview.OpenCreateResponseDialogAsync";
            if !result::has_view(world) {
                show_viewless(world, CRAFT_RESULT_DIALOG, caller, detail);
                result_closed(world, screen)
            } else if let Some(dialog) = show(world, CRAFT_RESULT_DIALOG, caller) {
                // `IsObtainedExp`: after minus before above 0.
                let obtained_exp = matches!(
                    (executed.before_exp, after_exp),
                    (Some(before), Some(after)) if after - before > 0
                );
                let first_craft_bonus = world
                    .get_resource::<CraftMasters>()
                    .and_then(|masters| masters.first_craft_bonus.flatten())
                    .unwrap_or(0);
                info!(
                    "[craft] ShowSubWindowDialog(CraftResultSubWindowDialog {dialog:?}): {detail}; IsObtainedExp {obtained_exp}; Open: PlaySEOneShot({SE_CRAFT_RESULT})"
                );
                world.resource_mut::<SeRequests>().0.push(SeRequest {
                    owner: None,
                    cue: SE_CRAFT_RESULT.to_owned(),
                    class: SeClass::Ui,
                    source: "craft-result",
                });
                screen.result = Some(result::ResultShown {
                    body,
                    name: executed.name.clone(),
                    count: executed.count,
                    is_first_craft: executed.is_first_craft,
                    first_craft_bonus,
                    obtained_exp,
                    shown_at: world.resource::<Time>().elapsed_secs(),
                    balloon: false,
                    close_tapped: false,
                });
                screen.answer = None;
                Some(DialogStep::Result {
                    shown: Some(dialog),
                })
            } else {
                result_closed(world, screen)
            }
        }
        DialogStep::Result {
            shown: Some(dialog),
        } => {
            let gone = !world
                .resource::<ScreenManager>()
                .dialogs()
                .any(|(id, ..)| id == dialog);
            if screen.answer.take().is_some() || gone {
                if gone {
                    warn!("[craft] CraftResultSubWindowDialog {dialog:?} was closed from outside");
                }
                info!("[craft] CraftResultSubWindowDialog {dialog:?}: Close");
                close(world, dialog);
                screen.result = None;
                result_closed(world, screen)
            } else {
                Some(DialogStep::Result {
                    shown: Some(dialog),
                })
            }
        }
        DialogStep::Bonus => {
            if let Some(fixture) = executed.bonus_fixture {
                show_viewless(
                    world,
                    FIXTURE_BONUS_DIALOG,
                    "CraftPreview.ShowFixtureBonusDialog",
                    format!("fixture {fixture}"),
                );
            }
            Some(DialogStep::Rank)
        }
        DialogStep::Rank => {
            // `OpenMysekaiRankDialogIfNeededAsync(before, after)`: the two
            // `MysekaiRankModel` ranks.
            let refusal = match (&screen.ranks, executed.before_exp, after_exp) {
                (Some(Ok(rows)), Some(before_exp), Some(after_exp)) => {
                    let before = get_mysekai_rank(rows, before_exp);
                    let after = get_mysekai_rank(rows, after_exp);
                    if before != after {
                        show_viewless(
                            world,
                            RANK_UP_DIALOG,
                            "MysekaiRankUtility.OpenMysekaiRankUpDialogAsync",
                            format!(
                                "rank {before} -> {after} (totalExp {before_exp} -> {after_exp})"
                            ),
                        );
                        error!(
                            "[craft] OpenMysekaiRankReleaseDialogAsync({before}, {after}): the rank's release dialogs (the status update and the new harvest site) are not built"
                        );
                    } else {
                        info!(
                            "[craft] OpenMysekaiRankDialogIfNeededAsync: one rank ({before}) for totalExp {before_exp} and {after_exp}; no dialog"
                        );
                    }
                    None
                }
                (Some(Err(reason)), ..) => Some(format!("the rank table is not served ({reason})")),
                (None, ..) => Some("the rank table has not resolved".to_owned()),
                _ => Some("the total experience before or after is not seated".to_owned()),
            };
            if let Some(reason) = refusal {
                if !screen.rank_reported {
                    screen.rank_reported = true;
                    error!("[craft] OpenMysekaiRankDialogIfNeededAsync: {reason}; no rank dialog");
                }
            }
            None
        }
    };
    screen.stage = match next {
        Some(next) => Stage::Dialogs {
            executed,
            after_exp,
            next,
        },
        None => Stage::Done,
    };
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Exclusive, Update: the screen's opening, the apply button, the answers
/// and the request, and the dialogs after the sequence.
pub(crate) fn advance(world: &mut World) {
    let open =
        world.resource::<ScreenManager>().current_screen() == Some(MenuScreenType::MysekaiCraft);
    let now = world.resource::<Time>().elapsed_secs();
    let mut screen = std::mem::take(&mut *world.resource_mut::<CraftScreen>());
    if screen.ranks.is_none() {
        if let Some(result) = world
            .get_resource_mut::<MasterData>()
            .and_then(|mut masters| masters.take(&RANKS))
        {
            screen.ranks = Some(result.map_err(|error| error.to_string()));
        }
    }
    if open && !screen.open {
        screen.open = true;
        screen.opened_at = now;
        screen.applied = false;
        screen.staged_fired = false;
        let workbench = world
            .get_resource::<CraftWorkbench>()
            .map(|workbench| workbench.0.uid.clone());
        info!(
            "[craft] ScreenLayerMysekaiCraft.OnInitComponent (workbench {}): SetupContentList (the Craft list), SelectFirstItem, SetupCraftPreview; PlaySEOneShot({SE_ENTER_CRAFT_TOOL})",
            workbench.as_deref().unwrap_or("not handed over")
        );
        world.resource_mut::<SeRequests>().0.push(SeRequest {
            owner: None,
            cue: SE_ENTER_CRAFT_TOOL.to_owned(),
            class: SeClass::Ui,
            source: "craft-screen",
        });
        screen.rows = match craft_rows(world) {
            Ok(rows) => rows,
            Err(reason) => {
                error!("[craft] SetupContentList: {reason}; the Craft list is empty");
                Vec::new()
            }
        };
        screen.rows_revision += 1;
        screen.selected = None;
        screen.selection = None;
        screen.button = None;
        screen.requests.clear();
        info!("[craft] the Craft list: {} rows", screen.rows.len());
        match select_instrument() {
            Some((blueprint, count)) => {
                match screen.rows.iter().position(|row| row.id == blueprint) {
                    Some(index) => select_row(world, &mut screen, index, count),
                    None => {
                        warn!(
                            "[craft] instrument: blueprint {blueprint} is not a Craft list row; it is selected outside the list"
                        );
                        match select(world, blueprint, count) {
                            Ok(selection) => {
                                screen.button = Some(setup_apply_button(world, &selection));
                                screen.selection = Some(selection);
                            }
                            Err(reason) => {
                                error!("[craft] SetupCraftPreview({blueprint}): {reason}")
                            }
                        }
                    }
                }
            }
            None if screen.rows.is_empty() => {
                info!("[craft] SetupCraftPreview(null): SetupEmpty (the Craft list has no rows)")
            }
            None => select_row(world, &mut screen, 0, 1),
        }
    } else if !open && screen.open {
        screen.open = false;
        info!("[craft] the craft screen closed");
        if let Stage::Confirm { dialog, .. } | Stage::TermLimit { dialog } = screen.stage {
            close(world, dialog);
            screen.stage = Stage::Idle;
        }
    }
    // The view's taps (the list and the apply button are under the dialog
    // layer: a shown dialog takes the taps first).
    for request in std::mem::take(&mut screen.requests) {
        if !screen.open || !matches!(screen.stage, Stage::Idle) {
            continue;
        }
        match request {
            ViewRequest::Select(index) => {
                info!("[craft] OnClickCell({index})");
                select_row(world, &mut screen, index, 1);
            }
            ViewRequest::Apply => click_apply(world, &mut screen),
        }
    }
    if screen.open && !screen.applied && matches!(screen.stage, Stage::Idle) {
        if let Some(secs) = seconds_instrument("MOLY_CRAFT_APPLY_SECS") {
            if now - screen.opened_at >= secs {
                screen.applied = true;
                info!("[craft] instrument: the apply button pressed");
                click_apply(world, &mut screen);
            }
        }
    }
    stage_instrument(world, &mut screen, now);
    match screen.stage.clone() {
        Stage::Confirm { dialog, .. } => {
            if let Some(ok) = screen.answer.take() {
                info!(
                    "[craft] the confirm's {}: Close",
                    if ok { "OK" } else { "cancel" }
                );
                close(world, dialog);
                screen.stage = Stage::Idle;
                if ok {
                    send(world, &mut screen);
                }
            }
        }
        Stage::TermLimit { dialog } => {
            if screen.answer.take().is_some() {
                info!("[craft] the term dialog's OK: Close, BackUIScreen");
                close(world, dialog);
                screen.stage = Stage::Idle;
                world.write_message(LayerCommand::Pop);
            }
        }
        Stage::Dialogs { .. } => advance_dialogs(world, &mut screen),
        Stage::Done if screen.staged => {
            info!("[craft] instrument: the staged dialogs are done; the screen is idle again");
            screen.staged = false;
            screen.stage = Stage::Idle;
        }
        _ => {}
    }
    *world.resource_mut::<CraftScreen>() = screen;
}

/// Spawn the two common dialogs' views once their documents are ready.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut spawned: Local<bool>,
) {
    if *spawned
        || ![COMMON1, COMMON2]
            .iter()
            .all(|key| layouts.ready(key, &server))
    {
        return;
    }
    *spawned = true;
    for key in [COMMON1, COMMON2] {
        let doc = layouts.document(key).expect("ready document");
        let mut view = UiPrefabView::new(key, SITEMAP_LAYER);
        view.set_visible(TABS, false);
        let label = |view: &mut UiPrefabView, text: &str, wording: &str| match layouts
            .wordings
            .get(wording)
        {
            Some(value) => view.set_text(text, value.clone()),
            None => error!("[craft] {key}: {wording} is not on the root; its button text stays"),
        };
        let (buttons, close) = if key == COMMON2 {
            match crate::two_button_dialog::resolve(doc) {
                Ok(dialog) => {
                    label(&mut view, &dialog.negative_label, "WORD_CANCEL");
                    label(&mut view, &dialog.positive_label, "WORD_OK");
                    (vec![dialog.negative, dialog.positive], dialog.close)
                }
                Err(reason) => {
                    error!("[craft] {key}: {reason}; the confirm's buttons are not bound");
                    (Vec::new(), CLOSE_BUTTON.to_owned())
                }
            }
        } else {
            let ids = |suffix: &str| -> Vec<String> {
                doc.nodes
                    .iter()
                    .filter(|node| node.path.ends_with(suffix))
                    .map(|node| format!("@{}", node.game_object_id))
                    .collect()
            };
            let buttons = ids(FOOTER_BUTTON);
            let texts = ids(FOOTER_TEXT);
            if buttons.len() != 1 || texts.len() != 1 {
                error!(
                    "[craft] {key}: {} footer buttons and {} texts, not one; its button is not labelled",
                    buttons.len(),
                    texts.len()
                );
            } else {
                label(&mut view, &texts[0], "WORD_OK");
            }
            (buttons, CLOSE_BUTTON.to_owned())
        };
        info!("[craft] {key} view spawned: buttons {buttons:?} (the last is OK), close {close}");
        commands.spawn((
            CommonDialogView {
                key,
                buttons,
                close,
            },
            Visibility::Hidden,
            Transform::from_xyz(0., 0., DIALOG_Z),
            RenderLayers::layer(SITEMAP_LAYER),
            view,
        ));
    }
}

/// Draw the shown common dialog (the confirm or the term dialog); the
/// confirm instrument's tap.
pub(crate) fn place(
    mut screen: ResMut<CraftScreen>,
    layouts: Res<UiLayouts>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut views: Query<(
        &CommonDialogView,
        &mut Visibility,
        &mut Transform,
        &mut UiPrefabView,
    )>,
    mut injected: MessageWriter<GestureEvent>,
) {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    for (common, mut visible, mut transform, mut view) in &mut views {
        transform.scale = Vec3::splat(scale);
        let body = match (&screen.stage, common.key) {
            (Stage::Confirm { body, .. }, COMMON2) => Some(body.clone()),
            (Stage::TermLimit { .. }, COMMON1) => {
                match layouts.wordings.get("MSG_CRAFT_TERM_LIMIT_DIALOG_BODY") {
                    Some(body) => Some(body.clone()),
                    None => {
                        error!(
                            "[craft] MSG_CRAFT_TERM_LIMIT_DIALOG_BODY is not on the root; the term dialog's body stays empty"
                        );
                        Some(String::new())
                    }
                }
            }
            _ => None,
        };
        let Some(body) = body else {
            *visible = Visibility::Hidden;
            continue;
        };
        *visible = Visibility::Inherited;
        view.set_text(MESSAGE_BODY, body);
        // The instrument: a tap at the confirm's OK button's drawn centre.
        if common.key != COMMON2 || screen.confirm_tapped {
            continue;
        }
        let Some(secs) = seconds_instrument("MOLY_CRAFT_CONFIRM_SECS") else {
            continue;
        };
        if time.elapsed_secs() - screen.shown_at < secs {
            continue;
        }
        let Some(rect) = common
            .buttons
            .last()
            .and_then(|ok| view.rect(&layouts, ok, size))
        else {
            continue;
        };
        let centre = rect.center();
        let position = Vec2::new(
            centre.x * scale + window.width() / 2.0,
            window.height() / 2.0 - centre.y * scale,
        );
        screen.confirm_tapped = true;
        info!(
            "[craft] instrument: tap injected at ({:.0},{:.0}) on the confirm's OK",
            position.x, position.y
        );
        injected.write(GestureEvent {
            kind: GestureKind::Tap,
            state: GestureState::End,
            position,
            delta: Vec2::ZERO,
            ui_owned: false,
        });
    }
}

/// Taps and the back key on the shown common dialog (modal: every tap is
/// its). Common2: OK, or cancel by its cancel button, the close button, a
/// tap outside the window or the back key. Common1: every one of these is
/// OK.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut screen: ResMut<CraftScreen>,
    layouts: Res<UiLayouts>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    views: Query<(&CommonDialogView, &UiPrefabView)>,
    mut gestures: MessageReader<GestureEvent>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut consumed: ResMut<ActionTapConsumed>,
) {
    let (key, dialog) = match screen.stage {
        Stage::Confirm { dialog, .. } => (COMMON2, dialog),
        Stage::TermLimit { dialog } => (COMMON1, dialog),
        _ => {
            gestures.clear();
            back_keys.clear();
            return;
        }
    };
    if screen.answer.is_some() {
        return;
    }
    if back_keys.read().any(|event| event.id == dialog) {
        info!("[craft] {key}: back key (OnHardwareBackKeyProcess)");
        screen.answer = Some(key == COMMON1);
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
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let Some((common, view)) = views.iter().find(|(common, _)| common.key == key) else {
        return;
    };
    let size = root_canvas.size(window);
    let scale = root_canvas.scale(window);
    for position in taps {
        consumed.0 = true;
        let canvas = Vec2::new(
            position.x - window.width() / 2.0,
            window.height() / 2.0 - position.y,
        ) / scale;
        let hit = |path: &str| {
            view.rect(&layouts, path, size)
                .is_some_and(|rect| rect.active && rect.contains(canvas))
        };
        let ok = common
            .buttons
            .last()
            .is_some_and(|button| hit(button.as_str()));
        let cancel = key == COMMON2
            && common
                .buttons
                .first()
                .is_some_and(|button| hit(button.as_str()));
        let external = hit(common.close.as_str()) || !hit(WINDOW_ROOT);
        if ok || cancel || external {
            let answer = ok || (external && key == COMMON1);
            info!(
                "[craft] {key}: tap on {}",
                if ok {
                    "OK"
                } else if cancel {
                    "cancel"
                } else {
                    "the close button or outside (OnCloseExternal)"
                }
            );
            screen.answer = Some(answer);
            break;
        }
        info!("[craft] {key}: tap inside the window off its buttons: nothing");
    }
}
