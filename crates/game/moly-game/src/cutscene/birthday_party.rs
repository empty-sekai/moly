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
//! - `ForceBirthdayPartyTalk(party, 1)`, awaited: the first script plays on
//!   the party's character (`MysekaiPlayerTalkAction.PlayAsync`, see
//!   [`crate::player_talk::play_script`]).
//! - `SetCharacterWaitTalkObjective(2)`: the character's current objective
//!   cancelled, the second script set as its birthday lua, the birthday wait
//!   talk data and its interrupt (the next decision builds the birthday wait
//!   objective, type 20: Idle until cancelled), the next rest skipped. A
//!   player talking to the character then plays that script.
//! - `RegisterBackKeyProcess` (`OnBackKeyForBirthday`), then
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
//!   `ForceBirthdayPartyTalk(party, 3)`, the end motion (not awaited) and
//!   2000 ms, `FadeInAsync(2)`, the character's action button removed, the
//!   characters disposed and the roster restored (`RestoreNpcAsync`, 30 s
//!   timeout), the site's BGM, `FadeOutAsync(1)`, `RemoveContext(Birthday)`
//!   and the normal UI, then the reward dialog when the API reply carried
//!   rewards.
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
//! `ShowCharacterEndMotion` reads `BirthdayEndCharacterMotionDataList`: the
//! first row whose `CharacterId` is the party's character unit gives
//! `ChangeEyePattern(EyeName)`, `ChangeLipSyncPattern(MouthName)` and
//! `ChangeAnimation(MotionName, hasExitTime true, 0.5, 1.0, playEndAnimation
//! true)`.
//!
//! Named, and not run here:
//!
//! - The end motion's `ChangeAnimation`: the host plays a talk motion only
//!   inside a talk session, and the end motion runs after the closing talk.
//!   The eye and mouth patterns are applied.
//! - `EnableStopWhenIdleState(false/true)`: the flag is read when the
//!   character enters Idle while it has a destination; the waiting character
//!   stands without one.
//! - `RemoveCharactersActionButton`: the character's button leaves the stack
//!   when the character is disposed right after it.
//! - `DisposeBirthdayBgm` unloads the cut-scene BGM's package; the stream
//!   table holds every cue already.
//! - The room's multiplay calls have no objects in this product.
//!
//! The party's character is a new NPC whose AI starts at once. Its first
//! decision finds no talk data and runs the general lottery over the client
//! talk list, which the server panel's talk-list policy builds from the
//! visitors' talks only (see [`crate::server_panel`]); the party's character
//! is not a visitor, so the lottery keeps no talk, the source's index error
//! is raised, and that character's AI ends. The party's three talks do not
//! read the AI: the first and the closing are played by the presenter, and
//! the player's talk reads the birthday talk data the wait writes. The wait
//! objective itself (Idle until cancelled) is then never built, and the
//! character stands idle as the objective would hold it.

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::action_button::ButtonType;
use moly_law::talk::FaceSlot;

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
/// `mysekai/birthday/birthday_end_character_motion_data_list` as extracted.
const END_MOTIONS: &str = "moly://birthday-end-character-motions.json";

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

/// The party and the fixture it was started from. Its character is found by
/// unit each time (`FindNPC(party.gameCharacterUnitId)`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Party {
    pub(crate) id: i64,
    pub(crate) unit: u32,
    /// The placed fixture the party was started from.
    pub(crate) fixture: Entity,
}

/// `NPCAvatarPresenter.SetBirthdayLua`: the script the character's birthday
/// wait plays when the player talks to it, with its scenario row.
#[derive(Component, Clone, Debug)]
pub(crate) struct BirthdayLua {
    pub(crate) scenario: i64,
    pub(crate) lua: String,
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
    /// `ForceBirthdayPartyTalk(party, 1)` while its talk plays.
    FirstTalk,
    /// `WaitEndBirthdayPartyAsync`.
    Wait,
    /// `FadeInAsync(1)`.
    FirstFade,
    /// `UniTask.Delay(500)` after the player's placement.
    PlaceDelay { left: f32 },
    /// `FadeOutAsync(1)`.
    FirstUnveil,
    /// `ForceBirthdayPartyTalk(party, 3)` while its talk plays.
    ClosingTalk,
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
    /// The end motion list's load (`LoadOrDownloadAssetAsync`), requested at
    /// the start.
    end_motions: Handle<JsonAsset>,
    /// `ShowCharacterEndMotion().Forget()` has been called and waits for its
    /// list.
    end_motion_pending: bool,
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

/// `GetMasterBirthdayScenario(party, seq)` and `FindNPC(unit)`: the row and
/// the character, or `None` after the source's silent return or error log.
fn scenario_and_character(
    world: &mut World,
    party: Party,
    seq: i64,
    caller: &str,
) -> Option<(crate::player_talk::BirthdayScenario, Entity)> {
    let scenario = match crate::player_talk::birthday_scenario(world, party.id, seq) {
        Ok(Some(scenario)) => scenario,
        Ok(None) => {
            info!("[birthday-party] {caller}: GetMasterBirthdayScenario({}, {seq}) = null: the source returns silently", party.id);
            return None;
        }
        Err(reason) => {
            warn!("[birthday-party] {caller}: GetMasterBirthdayScenario({}, {seq}): {reason}; nothing plays (named)", party.id);
            return None;
        }
    };
    let Some(npc) = crate::npc::gate_entries::find_npc(world, party.unit) else {
        error!("[birthday-party] {caller}: GetMasterBirthdayScenario({}, {seq}) = row {} ({}); FindNPC({}) found no character: the source logs and returns", party.id, scenario.id, scenario.lua, party.unit);
        return None;
    };
    Some((scenario, npc))
}

/// `MysekaiPlayerTalkAction.ForceBirthdayPartyTalk(party, seq)`. Returns
/// whether a talk opened (the caller awaits its end).
fn force_talk(world: &mut World, party: Party, seq: i64, caller: &str) -> bool {
    let Some((scenario, npc)) = scenario_and_character(world, party, seq, caller) else {
        return false;
    };
    let talk = crate::player_talk::ScriptTalk {
        npc,
        scenario: scenario.id,
        lua: scenario.lua.clone(),
    };
    match crate::player_talk::play_script(world, talk) {
        Ok(talk) => {
            info!("[birthday-party] {caller}: GetMasterBirthdayScenario({}, {seq}) = row {}; PlayAsync(mysekai/talk/scenario/talk, {}, [{npc:?}]): talk {talk} plays", party.id, scenario.id, scenario.lua);
            true
        }
        Err(reason) => {
            error!("[birthday-party] {caller}: PlayAsync({}): {reason}; the talk does not play", scenario.lua);
            false
        }
    }
}

fn talk_playing(world: &World) -> bool {
    world.contains_resource::<crate::player_talk::PlayerTalkSession>()
}

/// `SetCharacterWaitTalkObjective(2)`.
fn set_character_wait_talk_objective(world: &mut World, party: Party) {
    let caller = "SetCharacterWaitTalkObjective";
    let Some((scenario, npc)) = scenario_and_character(world, party, 2, caller) else {
        return;
    };
    let cancelled = crate::npc::gate_entries::try_cancel_current_objective(world, npc);
    if !cancelled {
        error!("[birthday-party] {caller}: TryCancelCurrentObjective on unit {} = false: the source logs and goes on", party.unit);
    }
    crate::npc::gate_entries::drop_owed_force_updates(world, npc);
    world.entity_mut(npc).insert(BirthdayLua {
        scenario: scenario.id,
        lua: scenario.lua.clone(),
    });
    crate::npc::gate_entries::force_update_birthday_wait_talk(world, npc);
    crate::npc::gate_entries::set_immediately_execute_next_objective(world, npc, true);
    info!("[birthday-party] {caller}: SetBirthdayLua({}) (row {}) on {npc:?}; ForceUpdateBirthdayWaitTalk; SetImmediatelyExecuteNextObjective(true)", scenario.lua, scenario.id);
}

/// `ShowCharacterEndMotion` once its list is loaded: the eye and mouth
/// patterns of the character's row (the motion is named, see the module).
fn show_character_end_motion(world: &mut World, party: Party, list: &serde_json::Value) {
    let caller = "ShowCharacterEndMotion";
    let row = list
        .get("rows")
        .and_then(|rows| rows.as_array())
        .and_then(|rows| {
            rows.iter().find(|row| {
                row.get("CharacterId").and_then(|id| id.as_i64()) == Some(i64::from(party.unit))
            })
        });
    let Some(row) = row else {
        error!("[birthday-party] {caller}: BirthdayEndCharacterMotionDatas has no row of character {}: the source logs", party.unit);
        return;
    };
    let field = |key: &str| row.get(key).and_then(|value| value.as_str()).unwrap_or("").to_owned();
    let (eye, mouth, motion) = (field("EyeName"), field("MouthName"), field("MotionName"));
    let Some(npc) = crate::npc::gate_entries::find_npc(world, party.unit) else {
        error!("[birthday-party] {caller}: FindNPC({}) found no character: the source logs", party.unit);
        return;
    };
    let handles = world
        .get::<crate::character_material::ToonMaterials>(npc)
        .and_then(|toon| Some((toon.slot_handle("eye")?.clone(), toon.slot_handle("mouth")?.clone())));
    let Some((eye_handle, mouth_handle)) = handles else {
        error!("[birthday-party] {caller}: unit {} has no eye and mouth materials", party.unit);
        return;
    };
    if !world.contains_resource::<crate::alone_action_runtime::FacialTables>() {
        error!("[birthday-party] {caller}: the facial tables are not loaded");
        return;
    }
    world.resource_scope(
        |world, mut materials: Mut<Assets<crate::character_material::CharacterMaterial>>| {
            let tables = world.resource::<crate::alone_action_runtime::FacialTables>();
            for (slot, pattern) in [(FaceSlot::Eye, &eye), (FaceSlot::Mouth, &mouth)] {
                crate::delayed_faces::FaceCommand {
                    unit: i64::from(party.unit),
                    slot,
                    pattern: pattern.clone(),
                    source_talk: 0,
                    source_step: 0,
                }
                .apply(tables, &mut materials, &eye_handle, &mouth_handle);
            }
        },
    );
    info!("[birthday-party] {caller}: unit {}: ChangeEyePattern({eye}), ChangeLipSyncPattern({mouth}); ChangeAnimation({motion:?}, hasExitTime true, 0.5, 1.0, playEndAnimation true) is not played (named)", party.unit);
}

/// The end motion's list, once its load ends.
fn advance_end_motion(world: &mut World) {
    let Some((party, handle)) = world
        .get_resource::<BirthdayParty>()
        .filter(|party| party.end_motion_pending)
        .map(|party| (party.party, party.end_motions.clone()))
    else {
        return;
    };
    if let Some(LoadState::Failed(error)) = world.resource::<AssetServer>().get_load_state(&handle) {
        error!("[birthday-party] ShowCharacterEndMotion: LoadOrDownloadAssetAsync(BirthdayEndCharacterMotionDataList): {error}");
        world.resource_mut::<BirthdayParty>().end_motion_pending = false;
        return;
    }
    let Some(text) = world
        .resource::<Assets<JsonAsset>>()
        .get(&handle)
        .map(|json| json.0.clone())
    else {
        return;
    };
    world.resource_mut::<BirthdayParty>().end_motion_pending = false;
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(list) => show_character_end_motion(world, party, &list),
        Err(error) => error!("[birthday-party] ShowCharacterEndMotion: the list is not JSON: {error}"),
    }
}

/// `PlayCutSceneAsync` returned: the party goes on.
pub(crate) fn start(world: &mut World, party: Party, rewards: Option<Vec<UserResource>>) {
    info!("[birthday-party] ExecuteAsync(party {}, unit {}) goes on: CacheBirthdayBgm (the cut-scene's BGM plays on); ChangeUIForBirthday: WaitUntil(!ScreenManager.IsUILayerWorking)", party.id, party.unit);
    let end_motions = world.resource::<AssetServer>().load::<JsonAsset>(END_MOTIONS);
    world.insert_resource(BirthdayParty {
        party,
        stage: Stage::ChangeUi,
        ui: None,
        close_confirm: None,
        rewards,
        end_motions,
        end_motion_pending: false,
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

/// After the first talk: the waiting objective, the back key, the wait.
fn begin_wait(world: &mut World, party: Party) {
    set_character_wait_talk_objective(world, party);
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

/// After the closing talk: the end motion (not awaited) and the delay.
fn begin_end_motion(world: &mut World) {
    world.resource_mut::<BirthdayParty>().end_motion_pending = true;
    info!("[birthday-party] ShowCharacterEndMotion().Forget(); UniTask.Delay({END_MOTION_DELAY} s)");
    set_stage(
        world,
        Stage::EndMotionDelay {
            left: END_MOTION_DELAY,
        },
        "the end-motion delay",
    );
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
    advance_end_motion(world);
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
            if force_talk(world, party, 1, "ForceBirthdayPartyTalk") {
                set_stage(world, Stage::FirstTalk, "the first talk plays");
            } else {
                begin_wait(world, party);
            }
        }
        Stage::FirstTalk => {
            if talk_playing(world) {
                return;
            }
            info!("[birthday-party] ForceBirthdayPartyTalk(party {}, 1) returned", party.id);
            begin_wait(world, party);
        }
        Stage::Wait => {}
        Stage::FirstFade => {
            if !fade_done(world) {
                return;
            }
            info!("[birthday-party] FadeInAsync(1) finished; EnableStopWhenIdleState(false) (named); PutAndLookPlayerBesideFixture; UniTask.Delay({PLACE_DELAY} s)");
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
            if force_talk(world, party, 3, "ForceBirthdayPartyTalk") {
                set_stage(world, Stage::ClosingTalk, "the closing talk plays");
            } else {
                begin_end_motion(world);
            }
        }
        Stage::ClosingTalk => {
            if talk_playing(world) {
                return;
            }
            info!("[birthday-party] ForceBirthdayPartyTalk(party {}, 3) returned", party.id);
            begin_end_motion(world);
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
            info!("[birthday-party] FadeInAsync(2) finished; EnableStopWhenIdleState(true) (named); RemoveCharactersActionButton: WaitUntil(!IsUILayerWorking)");
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
    // SetCharacterCantTalkState: FindNPC(unit)?.SetTalkEnabled(false).
    let npc = crate::npc::gate_entries::find_npc(world, party.unit);
    if let Some(npc) = npc {
        crate::npc::gate_entries::set_talk_enabled(world, npc, false);
    }
    info!("[birthday-party] SetCharacterCantTalkState: FindNPC({}) = {npc:?}; FadeInAsync(1)", party.unit);
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
    info!("[birthday-party] RemoveCharactersActionButton: FindNPC({})'s button leaves the stack with the character, disposed next (named)", party.unit);
    let disposed = crate::npc::dispose::dispose_npc_all(
        world,
        "BirthdayPartyPresenter.DeleteBirthdayCharacters",
    );
    info!("[birthday-party] DeleteBirthdayCharacters: AvatarDataStore.DisposeNPCAll: NPCs {disposed:?}");
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
    app.add_message::<EndBirthdayPartyClick>()
        .add_systems(Update, advance.before(super::advance));
}
