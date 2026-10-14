//! `BirthdayPartyPresenter.ExecuteAsync` after its cut-scene returns: the
//! party the player walks around in, and the close-down that gives the site
//! back.
//!
//! Call order once `PlayCutSceneAsync` returns:
//!
//! - `CacheBirthdayBgm`: the cut-scene's BGM stays (its `EndAsync` passes
//!   `restoreBGM false`, see [`crate::audio::cutscene`]).
//! - `ChangeUIForBirthday`: once the UI layer is idle, the Home and MyRoom
//!   screens' `UpdateUI`; with the context `InProgress` it shows the
//!   end-party button and hides the edit and site-map buttons.
//! - `ForceBirthdayPartyTalk(party, 1)`, `SetCharacterWaitTalkObjective(2)`,
//!   `RegisterBackKeyProcess` (`OnBackKeyForBirthday`), then
//!   `WaitEndBirthdayPartyAsync`.
//! - The end-party button and the back key both run `OnClickBackButton`: it
//!   runs only while the game state is below `Edit`, asks
//!   `ShowCloseBirthdayConfirmDialog`, and on OK sets the context to
//!   `AfterClose` (the end-party button hidden, the menu button disabled) and
//!   ends the wait.
//! - The close-down: the back key handler comes off (the key then does
//!   nothing in the scene until a field screen is set up again), the
//!   character cannot be talked to, `FadeInAsync(1)` to black, the player
//!   placed beside the fixture, 500 ms, `FadeOutAsync(1)`, the closing talk
//!   `ForceBirthdayPartyTalk(party, 3)`, the end motion and 2000 ms,
//!   `FadeInAsync(2)`, the character's action button removed, the characters
//!   disposed and the roster restored (`RestoreNpcAsync`, 30 s timeout), the
//!   site's BGM, `FadeOutAsync(1)`, `RemoveContext(Birthday)` and the normal
//!   UI, then the reward dialog when the API reply carried rewards.
//!
//! `FadeInAsync(t)` is `ScreenManager.FadeIn((0,0,0,1), 0, t)`, which covers
//! the screen, and `FadeOutAsync(t)` is `FadeOut((0,0,0,0), 0, t)`, which
//! uncovers it; each waits for the fader's `onFinish`.
//!
//! While the context is set the action buttons of [`DISABLED`] are disabled:
//! `ActionButtonRefresh` (event 45) hands the list to every button, whose
//! `GetButtonEnable` is `!list.Contains(type)` and whose `ChangeEnableView`
//! writes it to the button's `Behaviour.enabled`. Talk is not in the list, so
//! the character can still be spoken to.
//!
//! Named, and not run here:
//!
//! - The talks (`ForceBirthdayPartyTalk`, and the waiting objective's) play a
//!   lua through the talk engine (`MysekaiPlayerTalkAction.PlayAsync`); that
//!   entry is not built, so each talk logs its `birthdayPartyScenarios` row
//!   and takes no time. The character's waiting objective (type 20, talk type
//!   18) is not set.
//! - The party's character is the cut-scene's new avatar, which has no AI
//!   model and no collision object: `SetCharacterCantTalkState`,
//!   `EnableStopWhenIdleState` and `RemoveCharactersActionButton` find
//!   nothing to write.
//! - `ShowCharacterEndMotion` reads a character motion data list that is not
//!   extracted; the 2000 ms wait after it is run.
//! - `DisposeBirthdayBgm` unloads the cut-scene BGM's package; the stream
//!   table holds every cue already.
//! - The room's multiplay calls have no objects in this product.

use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_law::action_button::ButtonType;

use super::birthday::BirthdayContext;
use crate::common_dialog::Buttons;
use crate::server::client::talk_read::UserResource;
use crate::ui_layers::{BirthdayBackKey, DialogId, MysekaiBackKeyProcess};

/// `_disableActionButtonTypes`: the types the `Birthday` context disables
/// while it is set.
pub(crate) const DISABLED: &[ButtonType] = &[
    ButtonType::GimmickFixture,
    ButtonType::TimelineFixture,
    ButtonType::HouseEntry,
    ButtonType::OpenChest,
    ButtonType::GateEdit,
    ButtonType::GateInvitation,
    ButtonType::OpenCraftTool,
    ButtonType::OpenMysekaiInfo,
    ButtonType::OpenMysekaiBgmSelect,
    ButtonType::OpenMysekaiConvert,
    ButtonType::OpenOtherMysekai,
    ButtonType::OpenAvatarDressUp,
    ButtonType::OpenSecretShop,
    ButtonType::GoHomeSite,
    ButtonType::LeaveMysekai,
    ButtonType::Sketch,
    ButtonType::Delivery,
    ButtonType::BirthdayCutScene,
];

const CLOSE_CONFIRM: &str = "WORD_MYSEKAI_BIRTHDAY_PARTY_CLOSE_CONFIRM";
const RECEIVED_REWARD: &str = "MSG_RECEIVED_REWARD";
/// The wordings the party's screens draw.
pub(crate) const WORDINGS: &[&str] = &[
    CLOSE_CONFIRM,
    RECEIVED_REWARD,
    "WORD_END_BIRTHDAY_CUTSCENE",
];
/// `MOLY_BIRTHDAY_CLOSE_SECS=<seconds>`: the native instrument taps the close
/// confirm's OK that many seconds after it shows.
const CLOSE_INSTRUMENT: &str = "MOLY_BIRTHDAY_CLOSE_SECS";

/// The two fader targets.
const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const CLEAR: [f32; 4] = [0.0, 0.0, 0.0, 0.0];
/// The close-down's literals, in seconds.
const FIRST_FADE: f32 = 1.0;
const PLACE_DELAY: f32 = 0.5;
const END_MOTION_DELAY: f32 = 2.0;
const CLOSE_FADE: f32 = 2.0;
/// `RestoreCharacters`' `TimeoutController.Timeout(30 s)`.
const RESTORE_TIMEOUT: f32 = 30.0;

/// The party's character and fixture.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Party {
    pub(crate) id: i64,
    pub(crate) unit: u32,
    /// The placed fixture the party was started from.
    pub(crate) fixture: Entity,
    /// The cut-scene's avatar of the party's unit (`FindNPC(unit)`).
    pub(crate) avatar: Option<Entity>,
}

/// `MysekaiBirthdayPartyContext.BirthdayContextType`, as the screens' last
/// `UpdateUI` drew it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartyUi {
    /// 1: the end-party button shows; the edit and site-map buttons hide.
    InProgress,
    /// 2: the end-party button hides too and the menu button is disabled.
    AfterClose,
}

/// Where `ExecuteAsync` is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    /// `ChangeUIForBirthday`'s wait for the UI layer.
    ChangeUi,
    /// `WaitEndBirthdayPartyAsync`.
    Wait,
    /// `FadeInAsync(1)`.
    FirstFade,
    /// `UniTask.Delay(500)` after the player's placement.
    PlaceDelay { left: f32 },
    /// `FadeOutAsync(1)`.
    FirstUnveil,
    /// `UniTask.Delay(2000)` after the end motion.
    EndMotionDelay { left: f32 },
    /// `FadeInAsync(2)`.
    CloseFade,
    /// `RemoveCharactersActionButton`'s wait for the UI layer.
    RemoveButton,
    /// `RestoreCharacters` and its timeout.
    Restore { elapsed: f32 },
    /// `FadeOutAsync(1)`.
    LastUnveil,
    /// `UpdateUI`'s wait for the UI layer.
    UpdateUi,
}

/// `BirthdayPartyPresenter` from its cut-scene's return to the end of
/// `ExecuteAsync`.
#[derive(Resource)]
pub(crate) struct BirthdayParty {
    party: Party,
    stage: Stage,
    /// The context as the screens' last `UpdateUI` drew it; `None` before
    /// `ChangeUIForBirthday` and after the last `UpdateUI`.
    ui: Option<PartyUi>,
    /// `ShowCloseBirthdayConfirmDialog` while it is shown.
    close_confirm: Option<DialogId>,
    /// `obtainedResources` of the API reply; `None` when the fixture was
    /// already actioned.
    rewards: Option<Vec<UserResource>>,
}

impl BirthdayParty {
    /// The field menu's `ShowUI` branch for the party.
    pub(crate) fn ui(&self) -> Option<PartyUi> {
        self.ui
    }
}

/// A press on the end-party button (`SetOnClickEndBirthdayPartyButton`:
/// `OnClickBackButton().Forget()`).
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct EndBirthdayPartyClick;

/// One `birthdayPartyScenarios` row.
#[derive(Clone, Debug)]
struct Scenario {
    party: i64,
    seq: i64,
    lua: String,
}

const SCENARIOS: MasterTable<Vec<Scenario>> = MasterTable {
    table: "birthdayPartyScenarios",
    name: "birthdayPartyScenarios (the birthday party's talks)",
    parse: parse_scenarios,
};

fn parse_scenarios(text: &str) -> Result<Vec<Scenario>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(Scenario {
                party: master::int(row, "birthdayPartyId")?,
                seq: master::int(row, "seq")?,
                lua: master::text(row, "lua")?.to_owned(),
            })
        })
        .collect()
}

/// The scenario rows once they resolve.
#[derive(Resource, Default)]
struct Scenarios(Option<Result<Vec<Scenario>, String>>);

fn load(mut masters: ResMut<MasterData>) {
    masters.request(&SCENARIOS);
}

/// `MasterDataManager.GetMasterBirthdayScenario(party, seq)`: the row's lua,
/// `Ok(None)` when the table has no such row, `Err` when the table itself
/// failed.
fn scenario(world: &mut World, party: i64, seq: i64) -> Result<Option<String>, String> {
    if world.resource::<Scenarios>().0.is_none() {
        let key = SCENARIOS.key();
        let mut masters = world.resource_mut::<MasterData>();
        if !masters.is_resolved(key) {
            return Err("the birthdayPartyScenarios table is still loading".into());
        }
        let taken = masters
            .take(&SCENARIOS)
            .ok_or_else(|| "birthdayPartyScenarios was taken by another reader".to_owned())
            .and_then(|taken| taken.map_err(|error| error.to_string()));
        world.resource_mut::<Scenarios>().0 = Some(taken);
    }
    match world.resource::<Scenarios>().0.as_ref() {
        Some(Ok(rows)) => Ok(rows
            .iter()
            .find(|row| row.party == party && row.seq == seq)
            .map(|row| row.lua.clone())),
        Some(Err(reason)) => Err(reason.clone()),
        None => unreachable!("taken above"),
    }
}

/// `ForceBirthdayPartyTalk(party, seq)` up to the talk engine.
fn force_talk(world: &mut World, party: Party, seq: i64, caller: &str) {
    match scenario(world, party.id, seq) {
        Ok(Some(lua)) => info!("[birthday-party] {caller}: GetMasterBirthdayScenario({}, {seq}) = {lua}; FindNPC({}) = {:?}; PlayAsync(mysekai/talk/scenario/talk, {lua}, [the character]): the talk entry that plays a named lua is not built, so the talk does not play (named)", party.id, party.unit, party.avatar),
        Ok(None) => info!("[birthday-party] {caller}: GetMasterBirthdayScenario({}, {seq}) = null: the source returns silently", party.id),
        Err(reason) => warn!("[birthday-party] {caller}: GetMasterBirthdayScenario({}, {seq}): {reason}; the talk does not play (named)", party.id),
    }
}

/// `PlayCutSceneAsync` returned: the party goes on.
pub(crate) fn start(world: &mut World, party: Party, rewards: Option<Vec<UserResource>>) {
    info!("[birthday-party] ExecuteAsync(party {}, unit {}) goes on: CacheBirthdayBgm (the cut-scene's BGM plays on); ChangeUIForBirthday: WaitUntil(!ScreenManager.IsUILayerWorking)", party.id, party.unit);
    world.insert_resource(BirthdayParty {
        party,
        stage: Stage::ChangeUi,
        ui: None,
        close_confirm: None,
        rewards,
    });
}

fn delta(world: &World) -> f32 {
    world.get_resource::<Time>().map_or(0.0, Time::delta_secs)
}

fn ui_idle(world: &World) -> bool {
    world
        .get_resource::<crate::ui_layers::ScreenManager>()
        .is_none_or(|screens| !screens.is_ui_layer_working())
}

fn fade_done(world: &World) -> bool {
    world
        .get_resource::<crate::screen_fade::ScreenFader>()
        .is_none_or(crate::screen_fade::ScreenFader::finished)
}

fn set_back_key(world: &mut World, process: MysekaiBackKeyProcess, caller: &str) {
    if let Some(mut screens) = world.get_resource_mut::<crate::ui_layers::ScreenManager>() {
        screens.set_mysekai_back_key_process(process, caller);
    }
}

fn set_stage(world: &mut World, stage: Stage, note: &str) {
    if let Some(mut party) = world.get_resource_mut::<BirthdayParty>() {
        party.stage = stage;
    }
    info!("[birthday-party] -> {stage:?}: {note}");
}

/// Update (exclusive): the party's flow.
pub(crate) fn advance(world: &mut World) {
    let Some(stage) = world.get_resource::<BirthdayParty>().map(|party| party.stage) else {
        return;
    };
    if stage == Stage::Wait {
        let clicks = world
            .resource_mut::<Messages<EndBirthdayPartyClick>>()
            .drain()
            .count();
        let backs = world
            .resource_mut::<Messages<BirthdayBackKey>>()
            .drain()
            .count();
        if clicks > 0 {
            on_click_back_button(world, "the end-party button");
        }
        if backs > 0 {
            on_click_back_button(world, "the back key (OnBackKeyForBirthday)");
        }
        close_confirm_answer(world);
    }
    let Some(party) = world.get_resource::<BirthdayParty>().map(|party| party.party) else {
        return;
    };
    let dt = delta(world);
    match stage {
        Stage::ChangeUi => {
            if !ui_idle(world) {
                return;
            }
            world.resource_mut::<BirthdayParty>().ui = Some(PartyUi::InProgress);
            info!("[birthday-party] ChangeUIForBirthday: the UI layer is idle; ScreenLayerMysekaiHome/MyRoom.UpdateUI -> ShowUI -> SetUIForBirthday (InProgress): the end-party button shows, the edit and site-map buttons hide, VisibleMissionPanel(false); SetOnClickEndBirthdayPartyButton(OnClickBackButton)");
            force_talk(world, party, 1, "ForceBirthdayPartyTalk");
            force_talk(
                world,
                party,
                2,
                "SetCharacterWaitTalkObjective (the waiting objective, type 20 with talk type 18, is not set: named)",
            );
            // The waiting objective's body is `ChangeState(Idle)`: the
            // character's idle motion loops while the party waits. The
            // cut-scene's director released the animator, so the idle clip is
            // played again here.
            if let Some(avatar) = party.avatar {
                match crate::character::resume_idle_after_fixture(world, avatar) {
                    Ok(()) => info!("[birthday-party] NPCAvatarBirthdayPartyWaitObjective.ExecuteAsync: FindNPC({}).ChangeState(Idle): {avatar:?} plays its idle motion", party.unit),
                    Err(reason) => error!("[birthday-party] ChangeState(Idle) on {avatar:?}: {reason}"),
                }
            }
            set_back_key(
                world,
                MysekaiBackKeyProcess::Birthday,
                "BirthdayPartyPresenter.RegisterBackKeyProcess",
            );
            set_stage(
                world,
                Stage::Wait,
                "WaitEndBirthdayPartyAsync: WaitUntil(_birthdayEndFlag): the end-party button or the back key",
            );
        }
        Stage::Wait => {}
        Stage::FirstFade => {
            if !fade_done(world) {
                return;
            }
            info!("[birthday-party] FadeInAsync(1) finished; EnableStopWhenIdleState(false) (the party's character has no AI model: named); PutAndLookPlayerBesideFixture; UniTask.Delay({PLACE_DELAY} s)");
            super::birthday::put_and_look_player_beside_fixture(world, party.fixture);
            set_stage(world, Stage::PlaceDelay { left: PLACE_DELAY }, "the placement delay");
        }
        Stage::PlaceDelay { left } => {
            let left = left - dt;
            if left > 0.0 {
                world.resource_mut::<BirthdayParty>().stage = Stage::PlaceDelay { left };
                return;
            }
            crate::screen_fade::fade_out_color(
                world,
                CLEAR,
                0.0,
                FIRST_FADE,
                "BirthdayPartyPresenter.FadeOutAsync(1)",
            );
            set_stage(world, Stage::FirstUnveil, "FadeOutAsync(1)");
        }
        Stage::FirstUnveil => {
            if !fade_done(world) {
                return;
            }
            force_talk(world, party, 3, "ForceBirthdayPartyTalk");
            info!("[birthday-party] ShowCharacterEndMotion().Forget(): the character motion data list is not extracted, so no end motion plays (named); UniTask.Delay({END_MOTION_DELAY} s)");
            set_stage(
                world,
                Stage::EndMotionDelay {
                    left: END_MOTION_DELAY,
                },
                "the end-motion delay",
            );
        }
        Stage::EndMotionDelay { left } => {
            let left = left - dt;
            if left > 0.0 {
                world.resource_mut::<BirthdayParty>().stage = Stage::EndMotionDelay { left };
                return;
            }
            crate::screen_fade::fade_in_color(
                world,
                BLACK,
                0.0,
                CLOSE_FADE,
                "BirthdayPartyPresenter.FadeInAsync(2)",
            );
            set_stage(world, Stage::CloseFade, "FadeInAsync(2)");
        }
        Stage::CloseFade => {
            if !fade_done(world) {
                return;
            }
            info!("[birthday-party] FadeInAsync(2) finished; EnableStopWhenIdleState(true) (named, as above); RemoveCharactersActionButton: WaitUntil(!IsUILayerWorking)");
            set_stage(world, Stage::RemoveButton, "the UI-layer wait before the dispose");
        }
        Stage::RemoveButton => {
            if !ui_idle(world) {
                return;
            }
            let next = remove_and_restore(world, party);
            set_stage(world, next, "RemoveCharactersActionButton done");
        }
        Stage::Restore { elapsed } => {
            let elapsed = elapsed + dt;
            if crate::npc::restore::done(world) {
                info!("[birthday-party] RestoreNpcAsync finished after {elapsed:.3} s");
            } else if elapsed < RESTORE_TIMEOUT {
                world.resource_mut::<BirthdayParty>().stage = Stage::Restore { elapsed };
                return;
            } else {
                error!("[birthday-party] RestoreNpcAsync did not finish within {RESTORE_TIMEOUT} s: the timeout cancels it and ExecuteAsync throws; the close-down goes on here so the screen and the UI come back (named)");
            }
            crate::audio::cutscene::release_bgm(world, "MysekaiBGMManager.PlayBGMAsync(siteId, SiteEnvironmentUtility.GetSiteEnvironmentId(siteId)): the site's music");
            info!("[birthday-party] DisposeBirthdayBgm: the stream table holds the cue, nothing to unload (named)");
            crate::screen_fade::fade_out_color(
                world,
                CLEAR,
                0.0,
                FIRST_FADE,
                "BirthdayPartyPresenter.FadeOutAsync(1)",
            );
            set_stage(
                world,
                Stage::LastUnveil,
                "the site's BGM plays; FadeOutAsync(1)",
            );
        }
        Stage::LastUnveil => {
            if !fade_done(world) {
                return;
            }
            world.remove_resource::<BirthdayContext>();
            info!("[birthday-party] GameStateManager.RemoveContext(Birthday): MysekaiBirthdayPartyContext.OnExit (Finished; ActionButtonRefresh with an empty list: every action button is enabled); UpdateUI: WaitUntil(!IsUILayerWorking)");
            set_stage(world, Stage::UpdateUi, "the last UI update");
        }
        Stage::UpdateUi => {
            if !ui_idle(world) {
                return;
            }
            finish(world);
        }
    }
}

/// `OnClickBackButton`.
fn on_click_back_button(world: &mut World, from: &str) {
    if world
        .get_resource::<BirthdayParty>()
        .is_some_and(|party| party.close_confirm.is_some())
    {
        info!("[birthday-party] OnClickBackButton ({from}): the close confirm is shown and takes the input; nothing");
        return;
    }
    // IsEnableClickBackKey: GameStateManager.CurrentType < Edit.
    let (state, enabled) = world
        .get_resource::<crate::game_state::GameStateManager>()
        .map_or(("None", true), |manager| {
            (manager.current_type().name(), manager.allows_click_back_key())
        });
    if !enabled {
        info!("[birthday-party] OnClickBackButton ({from}): IsEnableClickBackKey is false (game state {state}); nothing");
        return;
    }
    let caller = "BirthdayPartyPresenter.ShowCloseBirthdayConfirmDialog";
    let body = crate::common_dialog::wording(world, caller, CLOSE_CONFIRM);
    let Some(dialog) = crate::common_dialog::open(
        world,
        caller,
        body,
        Buttons::Two {
            ok: "WORD_OK",
            cancel: "WORD_CANCEL",
        },
        Some(CLOSE_INSTRUMENT),
    ) else {
        error!("[birthday-party] OnClickBackButton ({from}): the close confirm cannot show; the party goes on");
        return;
    };
    info!("[birthday-party] OnClickBackButton ({from}): game state {state}; Show2ButtonDialog(Common2ButtonDialog, {CLOSE_CONFIRM}, WORD_OK, WORD_CANCEL, Layer_Dialog, Small, allowCloseExternal true) {dialog:?}");
    world.resource_mut::<BirthdayParty>().close_confirm = Some(dialog);
}

/// `ShowCloseBirthdayConfirmDialog`'s answer.
fn close_confirm_answer(world: &mut World) {
    let Some(dialog) = world
        .get_resource::<BirthdayParty>()
        .and_then(|party| party.close_confirm)
    else {
        return;
    };
    let Some(ok) = crate::common_dialog::answer(world, dialog) else {
        return;
    };
    world.resource_mut::<BirthdayParty>().close_confirm = None;
    if !ok {
        info!("[birthday-party] ShowCloseBirthdayConfirmDialog = false: the party goes on");
        return;
    }
    let party = world.resource::<BirthdayParty>().party;
    world.resource_mut::<BirthdayParty>().ui = Some(PartyUi::AfterClose);
    info!("[birthday-party] ShowCloseBirthdayConfirmDialog = true: SetAfterClosedStateType (AfterClose); _birthdayEndFlag = true; UpdateUI -> SetUIForBirthday (AfterClose): the end-party button hides, EnableMenuButton(false), the mission panel hidden");
    set_back_key(
        world,
        MysekaiBackKeyProcess::None,
        "BirthdayPartyPresenter.ExecuteAsync (-= OnBackKeyForBirthday: the handler is empty from here)",
    );
    info!("[birthday-party] SetCharacterCantTalkState: FindNPC({}) = {:?}, a cut-scene avatar with no talk flag (named); FadeInAsync(1)", party.unit, party.avatar);
    crate::screen_fade::fade_in_color(
        world,
        BLACK,
        0.0,
        FIRST_FADE,
        "BirthdayPartyPresenter.FadeInAsync(1)",
    );
    world.resource_mut::<BirthdayParty>().stage = Stage::FirstFade;
}

/// `RemoveCharactersActionButton`, `DeleteBirthdayCharacters` and the start
/// of `RestoreCharacters`.
fn remove_and_restore(world: &mut World, party: Party) -> Stage {
    info!("[birthday-party] RemoveCharactersActionButton: FindNPC({}) is the cut-scene avatar, which has no collision object: nothing to remove (named)", party.unit);
    let disposed = crate::npc::dispose::dispose_npc_all(
        world,
        "BirthdayPartyPresenter.DeleteBirthdayCharacters",
    );
    let avatar = party
        .avatar
        .filter(|&avatar| super::cast::dispose_avatar(world, avatar));
    info!("[birthday-party] DeleteBirthdayCharacters: AvatarDataStore.DisposeNPCAll: NPCs {disposed:?}, the party's character {avatar:?}");
    match crate::npc::restore::begin(world) {
        Ok(()) => {
            info!("[birthday-party] RestoreCharacters: SceneMysekai.RestoreNpcAsync -> SetupNpcVisitingAsync(visitingFromGate false, hasReturnedFromOtherRoom true), 30 s timeout");
            Stage::Restore { elapsed: 0.0 }
        }
        Err(reason) => {
            error!("[birthday-party] RestoreCharacters: {reason}; nothing is restored");
            Stage::Restore {
                elapsed: RESTORE_TIMEOUT,
            }
        }
    }
}

/// `UpdateUI` (normal) and the reward dialog: `ExecuteAsync` ends.
fn finish(world: &mut World) {
    let Some(party) = world.remove_resource::<BirthdayParty>() else {
        return;
    };
    info!("[birthday-party] UpdateUI -> SetUIForNormal(owner): the end-party button hides, the menu button is enabled, the edit and site-map buttons show; the room's multiplay calls have no objects here (named)");
    let rewards = party.rewards.unwrap_or_default();
    if rewards.is_empty() {
        info!("[birthday-party] ExecuteAsync ends: IsNullOrEmpty(rewards), no reward dialog");
        return;
    }
    let caller = "BirthdayPartyPresenter.ExecuteAsync: ShowSubWindowDialog<CommonRewardSubWindowDialog>";
    match crate::get_resource::show_common_reward(world, caller, RECEIVED_REWARD, rewards.len()) {
        Ok(dialog) => info!(
            "[birthday-party] {caller} {dialog:?} on Layer_Dialog; Setup(rewards {:?}, 0, OpenSE.Reward, {RECEIVED_REWARD}, false); not awaited: ExecuteAsync ends",
            rewards
                .iter()
                .map(|row| format!("{} {} x{}", row.resource_type, row.resource_id, row.quantity))
                .collect::<Vec<_>>()
        ),
        Err(reason) => error!("[birthday-party] {caller}: {reason}; ExecuteAsync ends"),
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Scenarios>()
        .add_message::<EndBirthdayPartyClick>()
        .add_systems(Startup, load)
        .add_systems(Update, advance.before(super::advance));
}
