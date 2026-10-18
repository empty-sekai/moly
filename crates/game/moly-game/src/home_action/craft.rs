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
//!   (CraftResultSubWindowDialog, 379: `WORD_FIXTURE_CREATE_RESULT` from the
//!   fixture lists, else `WORD_TOOL_CREATE_RESULT`; awaited to its close,
//!   whose `OnCloseCreateResponseDialog` sets the preview up again), the
//!   fixture bonus dialog (407) when the bonus data exists, and
//!   `MysekaiRankUtility.OpenMysekaiRankDialogIfNeededAsync(before, after)`:
//!   nothing when the two experiences give one rank, else the rank-up dialog
//!   (336) and then the rank's release dialogs (the new harvest site 410
//!   first, then the player status update 343 for each release row). Each is
//!   awaited to its close. Then the sequence closes (gate, Idle, fades,
//!   camera). The follow-up views are [`follow_up`].
//! - Common1's back key and outside tap answer OK; Common2's answer cancel
//!   (their `OnHardwareBackKeyProcess` and `OnCloseExternal`).
//!
//! - The content lists (`CraftContentListSelector.Setup(ContentListTypes)`,
//!   the screen's static array Craft (20), Tool (9), LimitedCraft (21)): one
//!   `CraftContentList` per type, each holding the master blueprints
//!   `UserResourceFactory.GetUserBluePrints(type)` keeps
//!   (`CheckEnableBluePrint`: the list's target type, a tool for Tool and a
//!   fixture otherwise, or a canvas blueprint in any list but Tool; a term
//!   row only in LimitedCraft, whose term holds the current time; available
//!   without possession or owned; none while the user has no blueprint
//!   rows). The LimitedCraft tab cell is active only while some blueprint
//!   term holds the current time (`GetMysekaiBlueprintsTermInSession`). The
//!   screen opens on the Craft list (`selectedContent`) and `SelectFirstItem`
//!   selects its first row. A tab tap plays `se_mysekai_ui_select`
//!   (`SetTabClickSe`); a different tab sets `selectedContent` and selects
//!   that list's first row. A tap on a cell (`OnClickCell`) selects its row
//!   and sets the preview up for it. The view is [`view`].
//!
//! Named gaps (each logged where it is met):
//! - The LimitedCraft tab's badge and its seen state (`SaveSelectedTopTab`)
//!   are not kept; the badge stays off. The sort dropdown, the genre tabs,
//!   the search button and the hashtag balloon are not built: the rows keep
//!   master order.
//! - The count selector is not built: the count is 1 unless the instrument
//!   names one.
//! - The colour cells are the 3D preview's colours (`GetColors`), and the 3D
//!   preview is not built: the cells here are the fixture master's colour ids
//!   (texture 1 and its other colours' textures, ascending), cell 0 selected
//!   as `SelectCellImmediate(0)`. A tool has no cells (the source keeps the
//!   previous fixture's cells on a tool; the first selection has none).
//! - The result dialog's view is [`result`] (a root without its document
//!   shows it without a view, named and closed); the follow-up dialogs'
//!   views are [`follow_up`] (a root without a document shows the dialog
//!   without a view, named). The rank-up's title texture and particle
//!   animation, the character icon of the bonus dialog, and the thumbnail
//!   images of the status update and the new harvest site are not loaded:
//!   the dialogs draw their texts and the prefab's image areas.
//! - A first craft is refused by name while its bonus row
//!   (`craft_mysekai_fixture_first_bonus` of the rank experience master) is
//!   not carried. The fixture bonus's own rows are read from the masters;
//!   when a fixture has no bonus row the bonus dialog does not show (as
//!   `HasFixtureBonus` false).
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
//! (at its drawn rect, through [`crate::common_dialog`]) that many seconds
//! after the confirm shows;
//! `MOLY_CRAFT_STAGE_RESULT=<count>[,first]` stages the dialogs after the
//! hold two seconds after the screen opens, for the selected blueprint, with
//! no craft request and no sequence (the result dialog's view without a
//! reachable craft; loud in the log).

use bevy::prelude::*;
use moly_assets::json::master::{MasterData, MasterTable};
use moly_law::ui::mysekai_rank::{MasterMysekaiRank, get_mysekai_rank};

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::fixture_activity_state::FixtureTarget;
use crate::userdata::client::craft::{
    CRAFT_COUNT_MAX, CRAFT_COUNT_MIN, CraftMasters, CraftOwned, MasterBlueprint, MysekaiCraftType,
    UserMysekaiCraftRequest, is_create_count_limit, is_enough_material_to_craft, is_first_craft,
    is_reached_possession_limit_to_craft,
};
use crate::userdata::client::instrument_env;
use crate::userdata::client::inventory::{
    ClientMysekaiInventory, PossessionMasters, fixture_total_count,
};
use crate::common_dialog::Buttons;
use crate::ui_layers::{
    DialogBackKey, DialogId, DialogType, DisplayLayerType, LayerCommand, MenuScreenType,
    ScreenManager,
};
use crate::ui_layout::UiLayouts;
use crate::userdata::{Answered, ServerPending};
use moly_link::ServerLink;
use moly_protocol::{Call, Refusal, Reply};

pub(crate) mod follow_up;
pub(crate) mod result;
pub(crate) mod view;

/// `DialogType.CraftResultSubWindowDialog`.
const CRAFT_RESULT_DIALOG: DialogType = DialogType::CraftResultSubWindowDialog;
/// `CraftResultSubWindowDialog.Open`'s cue.
const SE_CRAFT_RESULT: &str = "se_craft_result";
/// The dialogs' views sit above the screens' (a screen view is at z 20 and
/// its nodes add their order).
const SE_ENTER_CRAFT_TOOL: &str = "se_enter_crafttool";
/// `SetupContentList`'s `SetTabClickSe`.
const SE_TAB_CLICK: &str = "se_mysekai_ui_select";
/// The wordings the craft screen and its two common dialogs draw (the shell
/// charset takes them).
pub(crate) const WORDINGS: &[&str] = &[
    "WORD_CREATE_CONFIRM",
    "MSG_CRAFT_TERM_LIMIT_DIALOG_BODY",
    // The tab cells.
    "WORD_FIXTURE",
    "WORD_TOOL",
    "WORD_LIMITED_FIXTURE",
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
    // The follow-up dialogs.
    "FORMAT_ADD_PARAM",
    "MSG_FIXTURE_BONUS_UP_DIALOG",
    "MSG_HOME_SITE_SIZE_EXPAND",
    "MSG_MYROOM_SITE_SIZE_EXPAND",
    "MSG_MYROOM_SITE_RELEASE_NEW_FLOOR",
    "MSG_MYSEKAI_FIXTURE_HOME_LAYOUT_COST_UP",
    "MSG_MYSEKAI_FIXTURE_MYROOM_LAYOUT_COST_UP",
    "MSG_MYSEKAI_FIXTURE_RELEASE_PUT_LIMIT",
    "MSG_MYSEKAI_OPEN_SITE_MESSAGE",
];

/// The craft screen's `ContentListType`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CraftListType {
    Craft = 20,
    Tool = 9,
    LimitedCraft = 21,
}

/// `ScreenLayerMysekaiCraft.ContentListTypes`, the tab order.
pub(crate) const CRAFT_LIST_TYPES: [CraftListType; 3] = [
    CraftListType::Craft,
    CraftListType::Tool,
    CraftListType::LimitedCraft,
];

impl CraftListType {
    /// `UserResourceFactory.GetCraftTargetType`.
    fn target(self) -> MysekaiCraftType {
        match self {
            Self::Tool => MysekaiCraftType::MysekaiTool,
            Self::Craft | Self::LimitedCraft => MysekaiCraftType::MysekaiFixture,
        }
    }

    /// `MysekaiUtility.GetContentListTabWordingKey`.
    pub(crate) fn tab_wording(self) -> &'static str {
        match self {
            Self::Craft => "WORD_FIXTURE",
            Self::Tool => "WORD_TOOL",
            Self::LimitedCraft => "WORD_LIMITED_FIXTURE",
        }
    }
}

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
    /// The first craft's fixture bonus, captured before the request.
    bonus: Option<follow_up::BonusStart>,
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
    Bonus {
        shown: Option<DialogId>,
    },
    Rank {
        shown: Option<DialogId>,
        before: i32,
        after: i32,
    },
    /// The rank's release dialogs, one at a time from `release_queue`.
    Release,
}

#[derive(Clone, Debug, Default)]
enum Stage {
    #[default]
    Idle,
    /// The confirm (Common2ButtonDialog) is shown.
    Confirm { dialog: DialogId },
    /// The request was sent and its reply has not arrived.
    Sending {
        pending: ServerPending,
        executed: Executed,
        fixture: FixtureTarget,
        /// The blueprint's `craftTargetId`, for the reply's log.
        target: i32,
    },
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
    /// A tab cell's `OnClickCell(index)` (`OnSelectContentType`).
    Tab(usize),
    /// `OnClickApplyButton` (the button is enabled).
    Apply,
}

/// The craft screen's state.
#[derive(Resource, Default)]
pub(crate) struct CraftScreen {
    open: bool,
    opened_at: f32,
    /// Each list's rows (`InitListView`, when the screen opens), in
    /// [`CRAFT_LIST_TYPES`] order, master order.
    lists: [Vec<MasterBlueprint>; 3],
    /// `selectedContent`, an index into [`CRAFT_LIST_TYPES`].
    list: usize,
    /// `isActiveLimitedTabCell`.
    limited_tab: bool,
    /// The selected list's rows.
    rows: Vec<MasterBlueprint>,
    /// Bumped whenever the rows are read again (the view recomposes).
    rows_revision: u64,
    /// The selected row (`_selectIndex`).
    selected: Option<usize>,
    selection: Option<Selection>,
    button: Option<ApplyButton>,
    requests: Vec<ViewRequest>,
    stage: Stage,
    /// The result dialog's close (its view's taps).
    answer: Option<bool>,
    /// The instruments already fired this opening.
    applied: bool,
    rank_reported: bool,
    /// The rank table (`None` until it resolves; `Err`: why the region has
    /// none).
    ranks: Option<Result<Vec<MasterMysekaiRank>, String>>,
    /// The shown result dialog's data (its view draws it).
    result: Option<result::ResultShown>,
    /// The shown follow-up dialog and its data (its view draws it).
    follow_up: Option<follow_up::FollowUp>,
    /// The follow-up dialog's id while one is shown.
    dialog: Option<DialogId>,
    /// The release dialogs still to show after the rank-up.
    release_queue: Vec<follow_up::FollowUp>,
    /// The dialogs were staged by the instrument (no sequence closes them).
    staged: bool,
    staged_fired: bool,
    /// Why the last confirmed craft was not sent or not done.
    refusal: Option<String>,
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
        screen.follow_up = None;
        screen.dialog = None;
        screen.release_queue.clear();
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

/// `UserResourceFactory.GetUserBluePrints(kind)`: the master blueprints
/// `CheckEnableBluePrint(master, kind)` keeps, master order: the list's
/// target type or a canvas blueprint (any list but Tool); a term row only in
/// LimitedCraft, whose term must hold the current time
/// (`CheckEnableLimitedBluePrint`); available without possession or owned.
/// None while the user has no blueprint rows (the source returns null).
fn craft_rows(world: &World, kind: CraftListType) -> Result<Vec<MasterBlueprint>, String> {
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
    let now = match kind {
        CraftListType::LimitedCraft => Some(current_timestamp(world)?),
        CraftListType::Craft | CraftListType::Tool => None,
    };
    Ok(blueprints
        .values()
        .filter(|blueprint| {
            let target = blueprint.craft_type == kind.target()
                || (blueprint.craft_type == MysekaiCraftType::MysekaiCanvas
                    && kind != CraftListType::Tool);
            let term = terms
                .iter()
                .find(|term| term.mysekai_blueprint_id == blueprint.id);
            let in_term = match (term, now) {
                (None, None) => true,
                (Some(term), Some(now)) => {
                    moly_masters::craft::is_within_time(now, term.start_at, term.end_at)
                }
                (None, Some(_)) | (Some(_), None) => false,
            };
            target
                && in_term
                && (blueprint.is_available_without_possession
                    || inventory.has_blueprint(blueprint.id))
        })
        .copied()
        .collect())
}

/// The craft lists as the craft screen would show them now, for the
/// observation: per list, each blueprint with its target, name, colours,
/// material costs, and whether the apply button would be on for one craft
/// of its first colour (else the balloon wording key that says why).
pub(crate) fn recipes(world: &World) -> serde_json::Value {
    use serde_json::json;
    let mut lists = serde_json::Map::new();
    for kind in CRAFT_LIST_TYPES {
        let key = match kind {
            CraftListType::Craft => "fixture",
            CraftListType::Tool => "tool",
            CraftListType::LimitedCraft => "limited",
        };
        let rows = match craft_rows(world, kind) {
            Ok(rows) => rows,
            Err(reason) => {
                lists.insert(key.into(), json!({ "error": reason }));
                continue;
            }
        };
        let masters = world.get_resource::<CraftMasters>();
        let inventory = world.get_resource::<ClientMysekaiInventory>();
        let possession = world.get_resource::<PossessionMasters>();
        let entries: Vec<serde_json::Value> = rows
            .iter()
            .map(|blueprint| {
                let costs: Vec<serde_json::Value> = masters
                    .and_then(|masters| masters.blueprint_costs(blueprint.id))
                    .unwrap_or_default()
                    .iter()
                    .map(|cost| json!({ "material_id": cost.mysekai_material_id, "quantity": cost.quantity }))
                    .collect();
                let selection = select(world, blueprint.id, 1);
                let state = match (&selection, masters, inventory) {
                    (Ok(selection), Some(masters), Some(inventory)) => {
                        match apply_state(selection, masters, &owned(inventory, possession)) {
                            Ok(None) => json!({ "can_craft": true }),
                            Ok(Some(balloon)) => json!({ "can_craft": false, "reason": balloon }),
                            Err(reason) => json!({ "can_craft": false, "reason": reason }),
                        }
                    }
                    (Err(reason), ..) => json!({ "can_craft": false, "reason": reason }),
                    _ => json!({ "can_craft": false, "reason": "the craft masters or the client copy of the user's tables are not installed" }),
                };
                json!({
                    "blueprint_id": blueprint.id,
                    "craft_type": blueprint.craft_type.name(),
                    "target_id": blueprint.craft_target_id,
                    "name": selection.as_ref().ok().map(|selection| selection.name.clone()),
                    "colors": selection.as_ref().ok().map(|selection| selection.colors.clone()),
                    "costs": costs,
                    "can_craft": state["can_craft"],
                    "reason": state.get("reason"),
                })
            })
            .collect();
        lists.insert(key.into(), json!(entries));
    }
    serde_json::Value::Object(lists)
}

/// The client's current timestamp (`ClientUserData`'s server date plus the
/// time since it was read).
// ---------------------------------------------------------------------------
// A script's hand on the screen
// ---------------------------------------------------------------------------

/// Where the craft screen stands, as a script waits on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CraftPhase {
    /// The screen is not open.
    Closed,
    /// Open and taking taps.
    Idle,
    /// The confirm dialog is shown.
    Confirm,
    /// The request waits for its reply.
    Sending,
    /// The term limit dialog is shown.
    TermLimit,
    /// The craft's world sequence plays.
    Executing,
    /// The result dialog is shown.
    Result,
    /// The dialogs after the result.
    Dialogs,
    /// The dialogs are done; the sequence closes.
    Done,
}

pub(crate) fn phase(world: &World) -> CraftPhase {
    let Some(screen) = world.get_resource::<CraftScreen>() else {
        return CraftPhase::Closed;
    };
    match &screen.stage {
        Stage::Idle if !screen.open => CraftPhase::Closed,
        Stage::Idle => CraftPhase::Idle,
        Stage::Confirm { .. } => CraftPhase::Confirm,
        Stage::Sending { .. } => CraftPhase::Sending,
        Stage::TermLimit { .. } => CraftPhase::TermLimit,
        Stage::Executing(_) => CraftPhase::Executing,
        Stage::Dialogs { next: DialogStep::Result { shown: Some(_) }, .. } => CraftPhase::Result,
        Stage::Dialogs { .. } => CraftPhase::Dialogs,
        Stage::Done => CraftPhase::Done,
    }
}

/// Chooses `blueprint` on the open screen with the taps a finger makes: the
/// tab of the list that holds it when another is shown, then its cell
/// (`OnSelectContentType`, `OnClickCell`), handed over as the view hands
/// them. Refused when the screen is not open and idle, or when no shown list
/// holds the blueprint.
pub(crate) fn choose(world: &mut World, blueprint: i32) -> Result<String, String> {
    let mut screen = world.resource_mut::<CraftScreen>();
    if !screen.open || !matches!(screen.stage, Stage::Idle) {
        return Err("the craft screen is not open and idle".into());
    }
    let found = (0..CRAFT_LIST_TYPES.len())
        .filter(|index| CRAFT_LIST_TYPES[*index] != CraftListType::LimitedCraft || screen.limited_tab)
        .find_map(|index| {
            screen.lists[index]
                .iter()
                .position(|row| row.id == blueprint)
                .map(|row| (index, row))
        });
    let Some((list, row)) = found else {
        return Err(format!("blueprint {blueprint} is not a row of a shown craft list"));
    };
    if list != screen.list {
        screen.requests.push(ViewRequest::Tab(list));
    }
    screen.requests.push(ViewRequest::Select(row));
    Ok(format!("{:?} list row {row}", CRAFT_LIST_TYPES[list]))
}

/// The selected blueprint and its apply button: enabled, or the wording key
/// of the balloon that says why it is off.
pub(crate) fn selected(world: &World) -> Option<(i32, Result<(), &'static str>)> {
    let screen = world.get_resource::<CraftScreen>()?;
    let selection = screen.selection.as_ref()?;
    let button = screen.button?;
    let state = if button.enabled { Ok(()) } else { Err(button.balloon.unwrap_or("the apply button is off")) };
    Some((selection.blueprint.id, state))
}

/// Why the last confirmed craft was not sent or not done.
pub(crate) fn refusal(world: &World) -> Option<String> {
    world.get_resource::<CraftScreen>()?.refusal.clone()
}

fn current_timestamp(world: &World) -> Result<i64, String> {
    let user = world
        .get_resource::<crate::userdata::client::ClientUserData>()
        .ok_or("the server has not answered yet (no current timestamp)")?;
    Ok(user.current_timestamp(world.resource::<Time<Real>>().elapsed_secs()))
}

/// `GetMysekaiBlueprintsTermInSession().Count`: the term rows holding the
/// current time.
fn terms_in_session(world: &World) -> Result<usize, String> {
    let terms = world
        .get_resource::<CraftMasters>()
        .ok_or("the craft masters are not installed")?
        .terms
        .as_ref()
        .ok_or("the blueprint term master (mysekai-blueprint-terms.json) is absent")?;
    let now = current_timestamp(world)?;
    Ok(terms
        .iter()
        .filter(|term| moly_masters::craft::is_within_time(now, term.start_at, term.end_at))
        .count())
}

/// Shows list `index`: its rows, no selection yet.
fn show_list(screen: &mut CraftScreen, index: usize) {
    screen.list = index;
    screen.rows = screen.lists[index].clone();
    screen.rows_revision += 1;
    screen.selected = None;
    screen.selection = None;
    screen.button = None;
}

/// `CraftContentListSelector.SelectFirstItem`: the shown list's first row,
/// or the empty preview.
fn select_first_item(world: &World, screen: &mut CraftScreen) {
    if screen.rows.is_empty() {
        info!(
            "[craft] SetupCraftPreview(null): SetupEmpty (the {:?} list has no rows)",
            CRAFT_LIST_TYPES[screen.list]
        );
    } else {
        select_row(world, screen, 0, 1);
    }
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
    let Some(user) = world.get_resource::<crate::userdata::client::ClientUserData>() else {
        error!(
            "[craft] the blueprint's term is read before the server's first response: taken as outside it"
        );
        return true;
    };
    let now = user.current_timestamp(world.resource::<Time<Real>>().elapsed_secs());
    !moly_masters::craft::is_within_time(now, term.start_at, term.end_at)
}

fn open_term_limit(world: &mut World, screen: &mut CraftScreen) {
    let body = match world.resource::<UiLayouts>().wordings.get("MSG_CRAFT_TERM_LIMIT_DIALOG_BODY") {
        Some(body) => body.clone(),
        None => {
            error!(
                "[craft] MSG_CRAFT_TERM_LIMIT_DIALOG_BODY is not on the root; the term dialog's body stays empty"
            );
            String::new()
        }
    };
    if let Some(dialog) = crate::common_dialog::open(
        world,
        "DialogUtility.ShowCommon1ButtonDialog",
        body,
        Buttons::One { ok: "WORD_OK" },
        None,
    ) {
        info!(
            "[craft] the blueprint's term is over: ShowCommon1ButtonDialog(MSG_CRAFT_TERM_LIMIT_DIALOG_BODY, WORD_OK) {dialog:?}"
        );
        screen.stage = Stage::TermLimit { dialog };
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
    if let Some(dialog) = crate::common_dialog::open(
        world,
        "CraftPreview.OpenCraftConfirmDialog",
        body,
        Buttons::Two { ok: "WORD_OK", cancel: "WORD_CANCEL" },
        Some("MOLY_CRAFT_CONFIRM_SECS"),
    ) {
        info!(
            "[craft] OpenCraftConfirmDialog: Show2ButtonDialog(Common2ButtonDialog, null, WORD_OK, WORD_CANCEL) {dialog:?}, body GetFormat(WORD_CREATE_CONFIRM, name, {})",
            selection.count
        );
        screen.stage = Stage::Confirm { dialog };
    }
}

/// `SendCraftApiAsync` (the confirm's OK).
fn send(world: &mut World, screen: &mut CraftScreen) {
    let Some(selection) = screen.selection.clone() else {
        error!("[craft] SendCraftApiAsync: no blueprint selected");
        return;
    };
    screen.refusal = None;
    let Some(workbench) = world.get_resource::<CraftWorkbench>().cloned() else {
        error!(
            "[craft] SendCraftApiAsync: no pressed workbench was handed over; the craft is refused"
        );
        screen.refusal = Some("no pressed workbench was handed over".into());
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
        screen.refusal = Some(format!(
            "blueprint {} is a first craft, and the first-craft bonus row is not carried",
            selection.blueprint.id
        ));
        screen.button = Some(setup_apply_button(world, &selection));
        return;
    }
    info!(
        "[craft] HeaderUtility.HideBackUIScreen, MysekaiMissionUtility.SetNoticeWait(true): no counterpart"
    );
    let gamedata = inventory.gamedata();
    let bonus = if first && selection.blueprint.craft_type == MysekaiCraftType::MysekaiFixture {
        match follow_up::bonus_start(world, selection.blueprint.craft_target_id) {
            Some(bonus) => Some(bonus),
            None => {
                warn!(
                    "[craft] MysekaiFixtureUtility.HasFixtureBonus({}): the fixture has no bonus row in the carried masters; the bonus dialog does not show",
                    selection.blueprint.craft_target_id
                );
                None
            }
        }
    } else {
        None
    };
    let request = UserMysekaiCraftRequest {
        blueprint_id: selection.blueprint.id,
        texture_id: selection.texture(),
        card_id: None,
        is_special_training: None,
        quantity: selection.count,
        mysekai_blueprint_term_id: None,
    };
    info!("[craft] MysekaiCraftService.Execute: PostUserMysekaiCraftApi {request:?}");
    let call = Call::Craft(request);
    let kind = call.kind();
    let seq = world.resource_mut::<ServerLink>().call(call);
    info!("[craft] PostUserMysekaiCraftApi sent as seq {seq}; the preview waits for its reply");
    screen.stage = Stage::Sending {
        pending: ServerPending {
            seq: Some(seq),
            call: kind,
        },
        executed: Executed {
            before_exp: gamedata.map(|g| g.total_exp),
            is_first_craft: first,
            bonus,
            craft_type: selection.blueprint.craft_type,
            count: selection.count,
            name: selection.name.clone(),
        },
        fixture: workbench.0.clone(),
        target: selection.blueprint.craft_target_id,
    };
}

/// Reads the answer to the craft request.
pub(super) fn read_answer(
    mut answers: MessageReader<Answered>,
    screen: Option<Res<CraftScreen>>,
    mut commands: Commands,
) {
    for answer in answers.read() {
        let Some(pending) = screen.as_deref().and_then(CraftScreen::server_pending) else {
            continue;
        };
        if pending.seq != Some(answer.seq) {
            continue;
        }
        let outcome = answer.outcome.clone();
        commands.queue(move |world: &mut World| finish_send(world, outcome));
    }
}

/// Continues the craft request: the success enters the world sequence, a
/// refusal sets the screen's error and sets the apply button up again.
fn finish_send(world: &mut World, outcome: Result<Reply, Refusal>) {
    let mut screen = std::mem::take(&mut *world.resource_mut::<CraftScreen>());
    let Stage::Sending {
        executed,
        fixture,
        target,
        ..
    } = std::mem::take(&mut screen.stage)
    else {
        *world.resource_mut::<CraftScreen>() = screen;
        return;
    };
    match outcome {
        Ok(Reply::SuiteUser(reply)) if reply.success => {
            let updated = &reply.updated;
            info!(
                "[craft] PostUserMysekaiCraftApi succeeded: materials {} rows, fixtures of {}: {:?}, gamedata {:?}",
                updated.user_mysekai_materials.as_ref().map_or(0, Vec::len),
                target,
                updated.user_mysekai_fixtures.as_ref().map(|rows| rows
                    .iter()
                    .filter(|row| row.mysekai_fixture_id == target)
                    .map(|row| (row.texture_id, row.quantity))
                    .collect::<Vec<_>>()),
                updated.user_mysekai_gamedata
            );
            screen.stage = Stage::Executing(executed);
            world.write_message(super::HomeActionRequest::Craft {
                fixture,
                posted: true,
            });
        }
        Ok(Reply::SuiteUser(reply)) => {
            let reason = reply.refusal.unwrap_or_else(|| "no reason".to_owned());
            error!(
                "[craft] PostUserMysekaiCraftApi refused ({reason}): the API error shows; the preview is set up again"
            );
            screen.refusal = Some(format!("the craft API refused: {reason}"));
            if let Some(selection) = screen.selection.clone() {
                screen.button = Some(setup_apply_button(world, &selection));
            }
        }
        Err(refusal) => {
            error!(
                "[craft] PostUserMysekaiCraftApi refused ({}): the API error shows; the preview is set up again",
                refusal.reason
            );
            screen.refusal = Some(format!("the craft API refused: {}", refusal.reason));
            if let Some(selection) = screen.selection.clone() {
                screen.button = Some(setup_apply_button(world, &selection));
            }
        }
        Ok(other) => {
            error!("[craft] PostUserMysekaiCraftApi answered with {other:?}; no sequence starts");
        }
    }
    *world.resource_mut::<CraftScreen>() = screen;
}

/// The server call this screen waits for, for the director.
impl CraftScreen {
    #[allow(dead_code)] // Read by the director's wait.
    pub(crate) fn server_pending(&self) -> Option<&ServerPending> {
        match &self.stage {
            Stage::Sending { pending, .. } => Some(pending),
            _ => None,
        }
    }
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
    follow_up::request_masters(&mut masters);
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
    Some(DialogStep::Bonus { shown: None })
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
            bonus: None,
            craft_type: selection.blueprint.craft_type,
            count,
            name: selection.name.clone(),
        },
        after_exp: gamedata.map(|g| g.total_exp),
        next: DialogStep::Result { shown: None },
    };
}

/// The shown follow-up dialog closed (its view's tap, the back key or an
/// outside close): close it on the manager and forget its data.
fn shown_closed(world: &mut World, screen: &mut CraftScreen, dialog: DialogId) -> bool {
    let gone = !world
        .resource::<ScreenManager>()
        .dialogs()
        .any(|(shown, ..)| shown == dialog);
    if screen.answer.take().is_some() || gone {
        if gone {
            warn!("[craft] follow-up dialog {dialog:?} was closed from outside");
        }
        info!("[craft] follow-up dialog {dialog:?}: Close");
        close(world, dialog);
        screen.dialog = None;
        screen.follow_up = None;
        true
    } else {
        false
    }
}

/// `OpenMysekaiRankDialogIfNeededAsync(before, after)`: show the rank-up
/// only when the two ranks differ, then its release dialogs.
fn start_rank(
    world: &mut World,
    screen: &mut CraftScreen,
    executed: &Executed,
    after_exp: Option<i32>,
) -> Option<DialogStep> {
    let ranks = screen.ranks.clone();
    let refusal = match (&ranks, executed.before_exp, after_exp) {
        (Some(Ok(rows)), Some(before_exp), Some(after_exp)) => {
            let before = get_mysekai_rank(rows, before_exp);
            let after = get_mysekai_rank(rows, after_exp);
            if before != after {
                let data = follow_up::FollowUp::RankUp { before, after };
                match follow_up::open(world, &data) {
                    Some(dialog) => {
                        info!(
                            "[craft] OpenMysekaiRankUpDialogAsync({before}, {after}): ShowSubWindowDialog(MysekaiRankUpDialog {dialog:?})"
                        );
                        screen.dialog = Some(dialog);
                        screen.follow_up = Some(data);
                        return Some(DialogStep::Rank {
                            shown: Some(dialog),
                            before,
                            after,
                        });
                    }
                    None => {
                        screen.release_queue = follow_up::release_dialogs(world, before, after);
                        return Some(DialogStep::Release);
                    }
                }
            } else {
                info!(
                    "[craft] OpenMysekaiRankDialogIfNeededAsync: one rank ({before}) for totalExp {before_exp} and {after_exp}; no dialog"
                );
                None
            }
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

/// Show the next release dialog, or end the chain when none is left.
fn next_release(world: &mut World, screen: &mut CraftScreen) -> Option<DialogStep> {
    while !screen.release_queue.is_empty() {
        let data = screen.release_queue.remove(0);
        match follow_up::open(world, &data) {
            Some(dialog) => {
                info!("[craft] rank release {data:?}: ShowSubWindowDialog({dialog:?})");
                screen.dialog = Some(dialog);
                screen.follow_up = Some(data);
                return Some(DialogStep::Release);
            }
            None => error!("[craft] rank release {data:?} did not open; the next one is tried"),
        }
    }
    None
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
        DialogStep::Bonus { shown: None } => {
            let data = executed
                .bonus
                .as_ref()
                .and_then(|start| follow_up::bonus_dialog(world, start));
            match data {
                Some(data) => match follow_up::open(world, &data) {
                    Some(dialog) => {
                        info!(
                            "[craft] ShowFixtureBonusDialog({data:?}): ShowSubWindowDialog(FixtureBonusSubWindowDialog {dialog:?})"
                        );
                        screen.dialog = Some(dialog);
                        screen.follow_up = Some(data);
                        Some(DialogStep::Bonus {
                            shown: Some(dialog),
                        })
                    }
                    None => Some(DialogStep::Rank {
                        shown: None,
                        before: 0,
                        after: 0,
                    }),
                },
                None => Some(DialogStep::Rank {
                    shown: None,
                    before: 0,
                    after: 0,
                }),
            }
        }
        DialogStep::Bonus {
            shown: Some(dialog),
        } => {
            if shown_closed(world, screen, dialog) {
                Some(DialogStep::Rank {
                    shown: None,
                    before: 0,
                    after: 0,
                })
            } else {
                Some(DialogStep::Bonus {
                    shown: Some(dialog),
                })
            }
        }
        DialogStep::Rank { shown: None, .. } => {
            start_rank(world, screen, &executed, after_exp)
        }
        DialogStep::Rank {
            shown: Some(dialog),
            before,
            after,
        } => {
            if shown_closed(world, screen, dialog) {
                screen.release_queue = follow_up::release_dialogs(world, before, after);
                Some(DialogStep::Release)
            } else {
                Some(DialogStep::Rank {
                    shown: Some(dialog),
                    before,
                    after,
                })
            }
        }
        DialogStep::Release => match screen.dialog {
            Some(dialog) => {
                if shown_closed(world, screen, dialog) {
                    next_release(world, screen)
                } else {
                    Some(DialogStep::Release)
                }
            }
            None => {
                screen.follow_up = None;
                next_release(world, screen)
            }
        },
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
            "[craft] ScreenLayerMysekaiCraft.OnInitComponent (workbench {}): SetupContentList (Craft, Tool, LimitedCraft), SelectFirstItem, SetupCraftPreview; PlaySEOneShot({SE_ENTER_CRAFT_TOOL})",
            workbench.as_deref().unwrap_or("not handed over")
        );
        world.resource_mut::<SeRequests>().0.push(SeRequest {
            owner: None,
            cue: SE_ENTER_CRAFT_TOOL.to_owned(),
            class: SeClass::Ui,
            source: "craft-screen",
        });
        for (index, kind) in CRAFT_LIST_TYPES.into_iter().enumerate() {
            screen.lists[index] = match craft_rows(world, kind) {
                Ok(rows) => rows,
                Err(reason) => {
                    error!("[craft] SetupContentList: {reason}; the {kind:?} list is empty");
                    Vec::new()
                }
            };
        }
        screen.limited_tab = match terms_in_session(world) {
            Ok(count) => {
                info!(
                    "[craft] CraftContentListSelector.Setup: {count} blueprint terms in session; the LimitedCraft tab cell is {}",
                    if count > 0 { "active" } else { "inactive" }
                );
                count > 0
            }
            Err(reason) => {
                error!(
                    "[craft] GetMysekaiBlueprintsTermInSession: {reason}; the LimitedCraft tab cell is inactive"
                );
                false
            }
        };
        show_list(&mut screen, 0);
        screen.requests.clear();
        info!(
            "[craft] the lists: Craft {} rows, Tool {} rows, LimitedCraft {} rows",
            screen.lists[0].len(),
            screen.lists[1].len(),
            screen.lists[2].len()
        );
        match select_instrument() {
            Some((blueprint, count)) => {
                let found = (0..CRAFT_LIST_TYPES.len())
                    .filter(|index| {
                        CRAFT_LIST_TYPES[*index] != CraftListType::LimitedCraft
                            || screen.limited_tab
                    })
                    .find_map(|index| {
                        screen.lists[index]
                            .iter()
                            .position(|row| row.id == blueprint)
                            .map(|row| (index, row))
                    });
                match found {
                    Some((index, row)) => {
                        if index != screen.list {
                            info!(
                                "[craft] instrument: blueprint {blueprint} is a {:?} list row; that tab is selected",
                                CRAFT_LIST_TYPES[index]
                            );
                            show_list(&mut screen, index);
                        }
                        select_row(world, &mut screen, row, count)
                    }
                    None => {
                        warn!(
                            "[craft] instrument: blueprint {blueprint} is not a row of a shown list; it is selected outside the lists"
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
            None => select_first_item(world, &mut screen),
        }
    } else if !open && screen.open {
        screen.open = false;
        info!("[craft] the craft screen closed");
        if let Stage::Confirm { dialog } | Stage::TermLimit { dialog } = screen.stage {
            crate::common_dialog::dismiss(world, dialog);
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
            // `<SetupContentList>b__10_0`: the same type does nothing.
            ViewRequest::Tab(index) if index == screen.list => {
                info!(
                    "[craft] OnSelectContentType({index}): {:?} is already selectedContent",
                    CRAFT_LIST_TYPES[index]
                );
            }
            ViewRequest::Tab(index) => {
                info!(
                    "[craft] OnSelectContentType({index}): selectedContent {:?}, SelectFirstItem ({} rows)",
                    CRAFT_LIST_TYPES[index],
                    screen.lists[index].len()
                );
                show_list(&mut screen, index);
                select_first_item(world, &mut screen);
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
        Stage::Confirm { dialog } => {
            if let Some(ok) = crate::common_dialog::answer(world, dialog) {
                info!(
                    "[craft] the confirm's {}: Close",
                    if ok { "OK" } else { "cancel" }
                );
                screen.stage = Stage::Idle;
                if ok {
                    send(world, &mut screen);
                }
            }
        }
        Stage::TermLimit { dialog } => {
            if crate::common_dialog::answer(world, dialog).is_some() {
                info!("[craft] the term dialog's OK: Close, BackUIScreen");
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
