//! The birthday party's cut-scene: the party fixture's action
//! button, `MysekaiActionButtonsPresenter.OnClickBirthdayCutScene`, and
//! `BirthdayPartyPresenter.ExecuteAsync` up to the return of its cut-scene.
//!
//! The party fixture is a placed fixture whose `mysekaiSystemFixtures` row
//! has the type `birthday`; the row's `externalId` is the birthday party.
//!
//! - The button (`MysekaiFixtureUtility.CanShowBirthdayCutSceneButton`):
//!   not visiting, no `Birthday` game-state context, and the party's
//!   `startAt`/`closedAt` window holds the current time. It belongs to the
//!   action buttons; its click reaches [`BirthdayCutSceneClick`].
//! - The click (`OnClickBirthdayCutScene`): `IsExecutable` (the fixture's
//!   system fixture row and its party exist); `IsWithinBirthdayStartTime`
//!   (the party's `birthdayStartAt`/`closedAt` window holds the current
//!   time), else `ShowCommon1ButtonDialog(MSG_MYSEKAI_BIRTHDAY_PARTY_CLOSED,
//!   WORD_OK)` and an action button refresh; the fixture's cut-scene area
//!   must lie inside the grid and overlap no other fixture, else the overlap
//!   dialog; then `ExecuteAsync`.
//! - `ExecuteAsync` (call order): `Initialize` (party, unit, character:
//!   `gameCharacterUnits` then `gameCharacters`), `IsActionedSystemFixture`
//!   (the user's `userMysekaiSystemFixtureActions`),
//!   `ShowConfirmDialog(isActioned)`: a `Common2ButtonDialog` whose body is
//!   `WORD_MYSEKAI_BIRTHDAY_PARTY_CONFIRM` (the first time; OK labelled
//!   `WORD_START_BIRTHDAY_PARTY`) or `WORD_MYSEKAI_BIRTHDAY_PARTY_REMEMBER_CONFIRM`
//!   (after; OK labelled `WORD_OK`) formatted with the character's
//!   `givenName`, cancel `WORD_CANCEL`. Its cancel ends `ExecuteAsync`.
//!   Then `MysekaiSystemFixtureActionService.ExecuteAPI` (only when the
//!   fixture was not actioned), `MultiplayCore.IsConnected` (else the room's
//!   error dialog and the end), `AddContext(Birthday)`, the other players'
//!   hides, then
//!   `PlayCutSceneAsync`: `CutSceneExecutor.PlayAsync([party unit],
//!   GetMasterMysekaiBirthdayCutSceneByExternalId(party id).id, the fixture's
//!   view transform, useAlreadyExistCharacter true, hidePlayer false, showUI
//!   true, OnStartFadeInCallBack, OnEndFadeOutCallback, restoreBGM false)`.
//! - `OnStartFadeInCallBack`, awaited first in `SetupInternal`:
//!   `DownloadVoiceAsync`, `DestroyCharacters` (every NPC warps away and is
//!   destroyed), `CreateBirthdayCharacter` (a new NPC avatar of the party's
//!   unit, shown), which the presenter's `FindNPC` then takes over.
//! - `OnEndFadeOutCallback`, after the view is disposed: the character is
//!   shown, `PutCharacterBesideFixture`, the home screen's UI hidden,
//!   `PutAndLookPlayerBesideFixture`.
//! - After the cut-scene `ExecuteAsync` goes on with the party itself
//!   (`CacheBirthdayBgm`, `ChangeUIForBirthday`, `ForceBirthdayPartyTalk`,
//!   the wait for the party's end, the closing fades, `RestoreCharacters`,
//!   `RemoveContext`, the reward dialog): named, not run here.
//!
//! Server values, all the server model's: the current time (the server
//! clock), the realtime room (the own room, connected, with no other
//! players), the placed party fixture (the served home housing layout, whose
//! party cakes are its `policies.birthdayPartyFixtures`), the user's
//! `userMysekaiSystemFixtureActions` rows (the client's copy
//! [`ClientSystemFixtureActions`]) and the
//! `PostUserMysekaiSystemFixtureActionApi` reply
//! (`policies.systemFixtureActionReply`; see
//! [`crate::server::system_fixture_action`]). A refused reply is the source's
//! null reply: `ExecuteAsync` stops before its cut-scene.
//!
//! Named differences: the overlap dialog (`ShowCommonSubWindowDialog`) and
//! the home screen's `HideUI` are UI not drawn here; the cut-scene area
//! check is not modelled; the birthday
//! character is the cast's new avatar, created while the package loads and
//! hidden until the director's first frame (see [`super::cast`]); the voice
//! download is the root's stream table (the voice track plays from it).
//!
//! The native instrument `MOLY_BIRTHDAY_PARTY_ACTION=<seconds>[:<fixture>]`
//! presses the button as a player would, after that many real seconds on
//! the home site: it takes the first placed party fixture (or the named
//! master fixture) whose [`BirthdayButton`] shows, puts the player at the
//! spot `PutAndLookPlayerBesideFixture` uses (0.5 m behind the fixture's
//! forward, facing it), and once the action buttons' stack head is the
//! birthday cut-scene button taps its screen position through the window's
//! touch stream (the gesture layer, the button's hit and its dispatch, which
//! writes [`BirthdayCutSceneClick`]). The placement is the instrument's (a
//! player walks there); the game mode reads no environment variable.
//! `MOLY_BIRTHDAY_CONFIRM_SECS=<seconds>` taps the confirm dialog's OK that
//! many seconds after it shows.

use bevy::ecs::system::SystemState;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_law::action_button::ButtonType;
use serde_json::Value;

use std::collections::HashMap;

use super::{Cast, CastCaller, CastPlay};
use crate::common_dialog::Buttons;
use crate::fixture_activity_state::FixtureActivityIdentity;
use crate::ui_layers::DialogId;
use crate::server::client::system_fixture_action::ClientSystemFixtureActions;
use crate::server::client::talk_read::UserResource;
use crate::userdata::{Answered, ServerPending};
use moly_link::ServerLink;
use moly_protocol::api::SystemFixtureActionRequest;
use moly_protocol::{Call, Refusal, Reply};

/// `MysekaiCutSceneConditionType` of the birthday rows.
const CONDITION: &str = "birthday_party";
/// `MysekaiSystemFixtureType` of a party fixture.
const PARTY_FIXTURE_TYPE: &str = "birthday";
const INSTRUMENT: &str = "MOLY_BIRTHDAY_PARTY_ACTION";
const CONFIRM_INSTRUMENT: &str = "MOLY_BIRTHDAY_CONFIRM_SECS";
const CONFIRM_FIRST: &str = "WORD_MYSEKAI_BIRTHDAY_PARTY_CONFIRM";
const CONFIRM_AFTER: &str = "WORD_MYSEKAI_BIRTHDAY_PARTY_REMEMBER_CONFIRM";
const START: &str = "WORD_START_BIRTHDAY_PARTY";
const CLOSED: &str = "MSG_MYSEKAI_BIRTHDAY_PARTY_CLOSED";

/// The wordings the birthday dialogs and the site map's no-party dialog
/// draw (the shell charset takes them).
pub(crate) const WORDINGS: &[&str] = &[
    CONFIRM_FIRST,
    CONFIRM_AFTER,
    START,
    CLOSED,
    "MSG_NOTHING_BIRTHDAY_DIALOG_BODY",
];
/// `PutCharacterBesideFixture`: the angle added to the fixture's and the
/// distance from it; the unit `NIGO_MIKU_GAME_CHARACTER_UNIT_ID` has its own.
const CHARACTER_ANGLE: f32 = 30.0;
const CHARACTER_LENGTH: f32 = 0.75;
const NIGO_MIKU_UNIT: u32 = 31;
const NIGO_MIKU_ANGLE: f32 = 49.0;
/// The float 0.94 (bits 0x3f70a3d7).
const NIGO_MIKU_LENGTH: f32 = f32::from_bits(0x3f70_a3d7);
/// `PutAndLookPlayerBesideFixture`: the player's distance from the fixture.
const PLAYER_LENGTH: f32 = 0.5;

/// A click on the party fixture's birthday cut-scene button.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct BirthdayCutSceneClick {
    pub(crate) fixture: Entity,
}

/// A placed party fixture's button state, kept on the fixture every frame
/// for the action buttons:
/// - `within`: `IsWithinBirthdayTimeByFixtureId(fixtureId)` (the party's
///   `startAt`/`closedAt` window holds the server clock).
///   `MysekaiActionButtonsPresenter.GetPlayerActionButtonType` gives the
///   fixture the `BirthdayCutScene` button type when it holds, after the
///   gimmick and timeline checks and before the fixture's action type.
/// - `can_show`: `CanShowBirthdayCutSceneButton` (not visiting, no
///   `Birthday` context, the fixture exists, `within`), which
///   `IsActionButtonTypeAvailable(BirthdayCutScene)` answers. The product is
///   never visiting and the component sits on a placed fixture, so it is
///   `within` without the context.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BirthdayButton {
    pub(crate) within: bool,
    pub(crate) can_show: bool,
}

/// `GameStateManager.HasContext(MysekaiContextType.Birthday)`: set by
/// `ExecuteAsync`'s `AddContext`; while it is set the button does not show.
#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct BirthdayContext {
    pub(crate) party: i64,
}

/// The master tables, read from the region's master hosts.
const PARTIES: MasterTable<Vec<Party>> = MasterTable {
    table: "birthdayParties",
    name: "birthdayParties (the birthday cut-scene)",
    parse: parse_parties,
};
const SYSTEM_FIXTURES: MasterTable<Vec<SystemFixture>> = MasterTable {
    table: "mysekaiSystemFixtures",
    name: "mysekaiSystemFixtures (the birthday cut-scene)",
    parse: parse_system_fixtures,
};
const CUT_SCENES: MasterTable<Vec<(i64, i64, String)>> = MasterTable {
    table: "mysekaiCutScenes",
    name: "mysekaiCutScenes (the birthday cut-scene)",
    parse: parse_cut_scenes,
};
const CHARACTER_UNITS: MasterTable<HashMap<u32, i64>> = MasterTable {
    table: "gameCharacterUnits",
    name: "gameCharacterUnits (the birthday party's character)",
    parse: parse_character_units,
};
const CHARACTERS: MasterTable<HashMap<i64, String>> = MasterTable {
    table: "gameCharacters",
    name: "gameCharacters (the birthday confirm's given name)",
    parse: parse_characters,
};

/// The glyphs the birthday dialogs draw beyond their wordings: the party
/// characters' given names, once the tables are taken (empty when a table
/// failed, which the master layer names).
#[derive(Resource, Default)]
pub(crate) struct BirthdayCharset {
    pub(crate) chars: Option<Vec<char>>,
}

/// The tables: requested, then taken once all of them have resolved (a
/// missing or malformed table is named once by the master layer and kept
/// here as the error).
#[derive(Resource)]
enum BirthdayTables {
    Requested,
    Taken(Result<Tables, String>),
}

#[derive(Clone, Debug)]
struct Party {
    id: i64,
    unit: u32,
    start_at: i64,
    birthday_start_at: i64,
    closed_at: i64,
}

#[derive(Clone, Debug)]
struct SystemFixture {
    id: i64,
    fixture: i64,
    kind: String,
    external: i64,
}

/// `BirthdayPartyPresenter` from `ExecuteAsync` until its cut-scene returns.
#[derive(Resource)]
struct BirthdayPresenter {
    party: Party,
    fixture: Entity,
    rewards: Option<Vec<UserResource>>,
}

/// `ExecuteAsync` awaiting `ShowConfirmDialog` or its action's answer.
#[derive(Resource)]
pub(crate) struct BirthdayConfirm {
    dialog: DialogId,
    party: Party,
    system_fixture: i32,
    actioned: bool,
    fixture: Entity,
    /// The action's request, once the confirm's OK sent it.
    awaiting: Option<ServerPending>,
    /// The API reply's `obtainedResources`, kept for the party's reward
    /// dialog; `None` when the fixture was already actioned (the source
    /// nulls the field then, and the dialog is skipped).
    rewards: Option<Vec<UserResource>>,
}

impl BirthdayConfirm {
    /// The server call this confirm waits for, for the director.
    #[allow(dead_code)] // Read by the director's wait.
    pub(crate) fn server_pending(&self) -> Option<&ServerPending> {
        self.awaiting.as_ref()
    }
}

#[derive(Resource)]
struct InstrumentClick {
    after: f64,
    /// The party fixture's master fixture id, when the instrument names one.
    fixture: Option<i32>,
    stage: InstrumentStage,
}

#[derive(Clone, Copy, Debug)]
enum InstrumentStage {
    Waiting,
    /// The player stands beside the fixture since `at`; the button has been
    /// the stack head since `head_since`.
    Placed {
        master: i32,
        at: f64,
        head_since: Option<f64>,
    },
    Done,
}

/// How long the instrument waits for the button to reach the stack head.
const INSTRUMENT_HEAD_SECS: f64 = 10.0;
/// How long the button is the head before the tap (the button's view has
/// been placed and the shared input interval, 0.3 s, is over).
const INSTRUMENT_TAP_SECS: f64 = 0.5;
const INSTRUMENT_FINGER: u64 = 99021;

pub(super) fn load(mut commands: Commands, mut masters: ResMut<MasterData>) {
    masters.request(&PARTIES);
    masters.request(&SYSTEM_FIXTURES);
    masters.request(&CUT_SCENES);
    masters.request(&CHARACTER_UNITS);
    masters.request(&CHARACTERS);
    commands.insert_resource(BirthdayTables::Requested);
    if let Some(raw) = crate::server::instrument_env(INSTRUMENT) {
        let (seconds, fixture) = match raw.split_once(':') {
            Some((seconds, fixture)) => (seconds, Some(fixture)),
            None => (raw.as_str(), None),
        };
        let after: f64 = seconds.trim().parse().unwrap_or_else(|_| {
            panic!("{INSTRUMENT}={raw:?}: {seconds:?} is not a number of seconds")
        });
        let fixture = fixture.map(|fixture| {
            fixture.trim().parse::<i32>().unwrap_or_else(|_| {
                panic!("{INSTRUMENT}={raw:?}: {fixture:?} is not a master fixture id")
            })
        });
        info!("[birthday] native instrument {INSTRUMENT}: after {after} s on the home site, the player beside {} whose birthday cut-scene button shows, and one tap on that button", fixture.map_or("the first placed party fixture".to_owned(), |id| format!("party fixture {id}")));
        commands.insert_resource(InstrumentClick {
            after,
            fixture,
            stage: InstrumentStage::Waiting,
        });
    }
}

fn int(row: &Value, key: &str) -> Result<i64, String> {
    master::int(row, key)
}

/// `birthdayParties`.
fn parse_parties(text: &str) -> Result<Vec<Party>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(Party {
                id: int(row, "id")?,
                unit: u32::try_from(int(row, "gameCharacterUnitId")?)
                    .map_err(|_| "a party's gameCharacterUnitId is not a unit id")?,
                start_at: int(row, "startAt")?,
                birthday_start_at: int(row, "birthdayStartAt")?,
                closed_at: int(row, "closedAt")?,
            })
        })
        .collect()
}

/// `mysekaiSystemFixtures` (a row without `externalId` names none: 0).
fn parse_system_fixtures(text: &str) -> Result<Vec<SystemFixture>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(SystemFixture {
                id: int(row, "id")?,
                fixture: int(row, "mysekaiFixtureId")?,
                kind: row["mysekaiSystemFixtureType"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                external: row["externalId"].as_i64().unwrap_or(0),
            })
        })
        .collect()
}

/// The `mysekaiCutScenes` rows of the birthday condition: `id`,
/// `externalId`, `timelineAssetbundleName`.
fn parse_cut_scenes(text: &str) -> Result<Vec<(i64, i64, String)>, String> {
    master::rows(text)?
        .iter()
        .filter(|row| row["mysekaiCutSceneConditionType"].as_str() == Some(CONDITION))
        .map(|row| {
            Ok((
                int(row, "id")?,
                int(row, "externalId")?,
                master::text(row, "timelineAssetbundleName")?.to_owned(),
            ))
        })
        .collect()
}

/// `gameCharacterUnits`: unit id to `gameCharacterId`.
fn parse_character_units(text: &str) -> Result<HashMap<u32, i64>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            let unit = u32::try_from(int(row, "id")?).map_err(|_| "a unit id is negative")?;
            Ok((unit, int(row, "gameCharacterId")?))
        })
        .collect()
}

/// `gameCharacters`: id to `givenName`.
fn parse_characters(text: &str) -> Result<HashMap<i64, String>, String> {
    master::rows(text)?
        .iter()
        .map(|row| Ok((int(row, "id")?, master::text(row, "givenName")?.to_owned())))
        .collect()
}

struct TableRows {
    parties: Vec<Party>,
    system: Vec<SystemFixture>,
    cut_scenes: Vec<(i64, i64, String)>,
    units: HashMap<u32, i64>,
    characters: HashMap<i64, String>,
}

impl TableRows {
    /// `Initialize`'s character: the party unit's `gameCharacterId`, then
    /// that character's `givenName`.
    fn given_name(&self, unit: u32) -> Result<&str, String> {
        let character = self
            .units
            .get(&unit)
            .ok_or_else(|| format!("GetMasterGameCharacterUnit({unit}) = null"))?;
        self.characters
            .get(character)
            .map(String::as_str)
            .ok_or_else(|| format!("GetMasterGameCharacter({character}) = null (unit {unit})"))
    }
}

/// Shared, so that the readers of every frame take the tables without a copy.
type Tables = std::sync::Arc<TableRows>;

/// The master rows, once the master layer has resolved every table.
fn tables(world: &mut World) -> Result<Option<Tables>, String> {
    match world.get_resource::<BirthdayTables>() {
        None => return Err("the birthday tables were not requested".into()),
        Some(BirthdayTables::Taken(taken)) => return taken.clone().map(Some),
        Some(BirthdayTables::Requested) => {}
    }
    let mut masters = world.resource_mut::<MasterData>();
    let keys = [
        PARTIES.key(),
        SYSTEM_FIXTURES.key(),
        CUT_SCENES.key(),
        CHARACTER_UNITS.key(),
        CHARACTERS.key(),
    ];
    if keys.into_iter().any(|key| !masters.is_resolved(key)) {
        return Ok(None);
    }
    let (Some(parties), Some(system), Some(cut_scenes), Some(units), Some(characters)) = (
        masters.take(&PARTIES),
        masters.take(&SYSTEM_FIXTURES),
        masters.take(&CUT_SCENES),
        masters.take(&CHARACTER_UNITS),
        masters.take(&CHARACTERS),
    ) else {
        return Err("a birthday master table was taken by another reader".into());
    };
    let taken = (|| {
        Ok(std::sync::Arc::new(TableRows {
            parties: parties?,
            system: system?,
            cut_scenes: cut_scenes?,
            units: units?,
            characters: characters?,
        }))
    })()
    .map_err(|error: master::MasterError| error.to_string());
    let mut chars = Vec::new();
    if let Ok(tables) = &taken {
        info!(
            "[birthday] master tables: {} parties, {} system fixtures ({} of type {PARTY_FIXTURE_TYPE}), {} birthday cut-scenes, {} character units, {} characters",
            tables.parties.len(),
            tables.system.len(),
            tables.system.iter().filter(|row| row.kind == PARTY_FIXTURE_TYPE).count(),
            tables.cut_scenes.len(),
            tables.units.len(),
            tables.characters.len()
        );
        for party in &tables.parties {
            match tables.given_name(party.unit) {
                Ok(name) => chars.extend(name.chars()),
                Err(reason) => warn!("[birthday] party {}: {reason}; its confirm cannot show", party.id),
            }
        }
        chars.sort_unstable();
        chars.dedup();
    }
    world.resource_mut::<BirthdayCharset>().chars = Some(chars);
    world.insert_resource(BirthdayTables::Taken(taken.clone()));
    taken.map(Some)
}

/// Update (exclusive): the party fixtures' button state, the button's
/// clicks, the confirm's answer, and the instrument's press.
pub(super) fn advance(world: &mut World) {
    update_buttons(world);
    confirm_answer(world);
    let clicks: Vec<BirthdayCutSceneClick> = world
        .resource_mut::<Messages<BirthdayCutSceneClick>>()
        .drain()
        .collect();
    for click in clicks {
        on_click(world, click.fixture);
    }
    instrument_press(world);
}

/// Keeps [`BirthdayButton`] on each placed party fixture.
fn update_buttons(world: &mut World) {
    let Ok(Some(tables)) = tables(world) else {
        return;
    };
    let TableRows { parties, system, .. } = &*tables;
    let now = crate::birthday::now_ms();
    let context = world.contains_resource::<BirthdayContext>();
    let placed: Vec<(Entity, i64, Option<BirthdayButton>)> = world
        .query::<(Entity, &FixtureActivityIdentity, Option<&BirthdayButton>)>()
        .iter(world)
        .map(|(entity, identity, button)| (entity, i64::from(identity.master_id), button.copied()))
        .collect();
    for (entity, master, before) in placed {
        // IsWithinBirthdayTimeByFixtureId: the fixture's birthday system
        // fixture row, its party, the party's window.
        let within = system
            .iter()
            .find(|row| row.fixture == master && row.kind == PARTY_FIXTURE_TYPE)
            .and_then(|row| parties.iter().find(|party| party.id == row.external))
            .is_some_and(|party| {
                crate::birthday::is_within_time(now, party.start_at, party.closed_at)
            });
        if !within && before.is_none() {
            continue;
        }
        let button = BirthdayButton {
            within,
            can_show: within && !context,
        };
        if before != Some(button) {
            info!("[birthday] fixture {master} ({entity:?}) at {now}: IsWithinBirthdayTimeByFixtureId {within}, CanShowBirthdayCutSceneButton {} (Birthday context {context})", button.can_show);
            world.entity_mut(entity).insert(button);
        }
    }
}

/// The fixture's pose placed through `yaw_deg` and `length` in its own
/// frame, turned `facing_deg` (see [`end_fade_out_callback`]).
fn beside(pose: &Transform, yaw_deg: f32, length: f32, facing_deg: f32) -> (Vec3, Quat) {
    let yaw = yaw_deg.to_radians();
    let local = Vec3::new(length * yaw.sin(), 0.0, length * yaw.cos());
    let rotation =
        moly_assets::coordinates::source_rotation(Quat::from_rotation_y(facing_deg.to_radians()));
    (
        pose.translation + pose.rotation * moly_assets::coordinates::source_position(local),
        pose.rotation * rotation,
    )
}

fn player_entity(world: &mut World) -> Option<Entity> {
    world
        .query_filtered::<Entity, With<crate::player::PlayerControlled>>()
        .iter(world)
        .next()
}

fn set_stage(world: &mut World, stage: InstrumentStage) {
    world.resource_mut::<InstrumentClick>().stage = stage;
}

/// The instrument's press (see the module).
fn instrument_press(world: &mut World) {
    let Some((after, wanted, stage)) = world
        .get_resource::<InstrumentClick>()
        .map(|click| (click.after, click.fixture, click.stage))
    else {
        return;
    };
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    if matches!(stage, InstrumentStage::Done)
        || now < after
        || !world
            .get_resource::<crate::site::SiteActive>()
            .is_some_and(|site| site.site_type == "home_site")
    {
        return;
    }
    match stage {
        InstrumentStage::Waiting => instrument_place(world, wanted, now),
        InstrumentStage::Placed {
            master,
            at,
            head_since,
        } => instrument_tap(world, master, at, head_since, now),
        InstrumentStage::Done => {}
    }
}

/// The instrument's first step: the player beside a party fixture whose
/// button shows.
fn instrument_place(world: &mut World, wanted: Option<i32>, now: f64) {
    let tables = match tables(world) {
        Ok(Some(tables)) => tables,
        Ok(None) => return,
        Err(reason) => {
            error!("[birthday] {INSTRUMENT}: {reason}; no press");
            return set_stage(world, InstrumentStage::Done);
        }
    };
    let TableRows { parties, system, .. } = &*tables;
    let party_fixtures: Vec<i64> = system
        .iter()
        .filter(|row| row.kind == PARTY_FIXTURE_TYPE)
        .map(|row| row.fixture)
        .collect();
    let mut placed: Vec<(Entity, i32, Option<BirthdayButton>)> = world
        .query::<(Entity, &FixtureActivityIdentity, Option<&BirthdayButton>)>()
        .iter(world)
        .filter(|(_, identity, _)| {
            party_fixtures.contains(&i64::from(identity.master_id))
                && wanted.is_none_or(|id| id == identity.master_id)
        })
        .map(|(entity, identity, button)| (entity, identity.master_id, button.copied()))
        .collect();
    placed.sort_by_key(|(entity, _, _)| entity.index());
    if placed.is_empty() {
        error!("[birthday] {INSTRUMENT}: no party fixture{} (system fixture type {PARTY_FIXTURE_TYPE}: master fixtures {party_fixtures:?}) is placed on the home site; no press", wanted.map_or(String::new(), |id| format!(" {id}")));
        return set_stage(world, InstrumentStage::Done);
    }
    let clock = crate::birthday::now_ms();
    let Some(&(entity, master, _)) = placed
        .iter()
        .find(|(_, _, button)| button.is_some_and(|button| button.can_show))
    else {
        for (entity, master, button) in &placed {
            let party = system
                .iter()
                .find(|row| row.fixture == i64::from(*master))
                .and_then(|row| parties.iter().find(|party| party.id == row.external));
            error!("[birthday] {INSTRUMENT}: party fixture {master} ({entity:?}) shows no birthday cut-scene button at {clock}: {button:?}, party {:?}", party.map(|party| (party.id, party.start_at, party.closed_at)));
        }
        return set_stage(world, InstrumentStage::Done);
    };
    let Some(pose) = world
        .get::<GlobalTransform>(entity)
        .map(GlobalTransform::compute_transform)
    else {
        error!(
            "[birthday] {INSTRUMENT}: party fixture {master} ({entity:?}) has no pose; no press"
        );
        return set_stage(world, InstrumentStage::Done);
    };
    let (at, rotation) = beside(&pose, 180.0, PLAYER_LENGTH, 0.0);
    let Some(player) = player_entity(world) else {
        return;
    };
    if let Some(mut transform) = world.get_mut::<Transform>(player) {
        transform.translation = at;
        transform.rotation = rotation;
    }
    info!("[birthday] {INSTRUMENT}: party fixture {master} ({entity:?}) shows its button at {clock}; the player {player:?} is put {PLAYER_LENGTH} m behind the fixture's forward -> ({:.3},{:.3},{:.3}), facing it; waiting for the action buttons' stack head", at.x, at.y, at.z);
    set_stage(
        world,
        InstrumentStage::Placed {
            master,
            at: now,
            head_since: None,
        },
    );
}

/// The instrument's second step: the tap once the button is the stack head.
fn instrument_tap(world: &mut World, master: i32, at: f64, head_since: Option<f64>, now: f64) {
    let head = world
        .get_resource::<crate::action_button::ActionButtonState>()
        .and_then(|state| state.current());
    if !head.is_some_and(|(button, _)| button == ButtonType::BirthdayCutScene) {
        if now - at > INSTRUMENT_HEAD_SECS {
            error!("[birthday] {INSTRUMENT}: the birthday cut-scene button of party fixture {master} did not reach the action buttons' stack head in {INSTRUMENT_HEAD_SECS} s (head {head:?}); no press");
            return set_stage(world, InstrumentStage::Done);
        }
        if head_since.is_some() {
            set_stage(
                world,
                InstrumentStage::Placed {
                    master,
                    at,
                    head_since: None,
                },
            );
        }
        return;
    }
    let Some(since) = head_since else {
        info!(
            "[birthday] {INSTRUMENT}: the action buttons' stack head is {head:?} ({:.2} s after the placement)",
            now - at
        );
        return set_stage(
            world,
            InstrumentStage::Placed {
                master,
                at,
                head_since: Some(now),
            },
        );
    };
    if now - since < INSTRUMENT_TAP_SECS {
        return;
    }
    let Ok((window_entity, window)) = world
        .query_filtered::<(Entity, &Window), With<bevy::window::PrimaryWindow>>()
        .single(world)
        .map(|(entity, window)| (entity, window.clone()))
    else {
        error!("[birthday] {INSTRUMENT}: no primary window; no press");
        return set_stage(world, InstrumentStage::Done);
    };
    let mut screen: SystemState<crate::action_button::ActionButtonScreen> = SystemState::new(world);
    let position = screen
        .get(world)
        .button_position(&window, ButtonType::BirthdayCutScene);
    let Some(position) = position else {
        error!("[birthday] {INSTRUMENT}: the birthday cut-scene button has no screen position (its view is not laid out); no press");
        return set_stage(world, InstrumentStage::Done);
    };
    let mut events = world.resource_mut::<Messages<bevy::window::WindowEvent>>();
    for phase in [TouchPhase::Started, TouchPhase::Ended] {
        events.write(bevy::window::WindowEvent::TouchInput(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: INSTRUMENT_FINGER,
        }));
    }
    info!(
        "[birthday] {INSTRUMENT}: tap on the birthday cut-scene button at ({:.0},{:.0}) (stack head {head:?}) through the window's touch stream",
        position.x, position.y
    );
    set_stage(world, InstrumentStage::Done);
}

/// `OnClickBirthdayCutScene`.
fn on_click(world: &mut World, fixture: Entity) {
    if let Some(context) = world.get_resource::<BirthdayContext>() {
        info!("[birthday] OnClickBirthdayCutScene: the Birthday context is set (party {}): CanShowBirthdayCutSceneButton is false, so there is no button to click", context.party);
        return;
    }
    if world.contains_resource::<BirthdayConfirm>() {
        info!("[birthday] OnClickBirthdayCutScene: the confirm dialog is shown (it takes every tap); the click does nothing");
        return;
    }
    let tables = match tables(world) {
        Ok(Some(tables)) => tables,
        Ok(None) => {
            error!("[birthday] OnClickBirthdayCutScene: the birthday tables are still loading; the click does nothing");
            return;
        }
        Err(reason) => {
            error!("[birthday] OnClickBirthdayCutScene: {reason}; the click does nothing");
            return;
        }
    };
    let TableRows { parties, system, .. } = &*tables;
    // IsExecutable.
    let Some(master) = world
        .get::<FixtureActivityIdentity>(fixture)
        .map(|identity| i64::from(identity.master_id))
    else {
        error!("[birthday] IsExecutable: {fixture:?} is not a placed fixture");
        return;
    };
    let Some(row) = system.iter().find(|row| row.fixture == master).cloned() else {
        error!("[birthday] IsExecutable: fixture {master} has no system fixture row: the birthday cut-scene cannot play (logged by the source)");
        return;
    };
    let Some(party) = parties
        .iter()
        .find(|party| party.id == row.external)
        .cloned()
    else {
        error!("[birthday] IsExecutable: system fixture {} names birthday party {}, which has no row: the birthday cut-scene cannot play (logged by the source)", row.id, row.external);
        return;
    };
    // IsWithinBirthdayStartTime(party).
    let now = crate::birthday::now_ms();
    if !crate::birthday::is_within_time(now, party.birthday_start_at, party.closed_at) {
        let body = crate::common_dialog::wording(world, "DialogUtility.ShowCommon1ButtonDialog", CLOSED);
        let dialog = crate::common_dialog::open_detached(
            world,
            "DialogUtility.ShowCommon1ButtonDialog",
            body,
            Buttons::One { ok: "WORD_OK" },
        );
        info!("[birthday] OnClickBirthdayCutScene: party {} is not within its birthday start time at {now} ([{}, {})): ShowCommon1ButtonDialog({CLOSED}, WORD_OK) {dialog:?}; publish ActionButtonRefresh (the action buttons here are re-evaluated every frame)", party.id, party.birthday_start_at, party.closed_at);
        return;
    }
    info!("[birthday] OnClickBirthdayCutScene: party {} (unit {}, window [{}, {}), birthday start {}) at {now}: IsCutsceneAreaOverlap / IsCutsceneAreaWithinGrid are not modelled (taken as clear); ExecuteAsync", party.id, party.unit, party.start_at, party.closed_at, party.birthday_start_at);
    execute(world, party, row, fixture, &tables);
}

/// `ExecuteAsync` up to `ShowConfirmDialog`.
fn execute(world: &mut World, party: Party, row: SystemFixture, fixture: Entity, tables: &TableRows) {
    let Ok(system_fixture) = i32::try_from(row.id) else {
        error!("[birthday] ExecuteAsync: system fixture id {} does not fit the API's int; nothing plays", row.id);
        return;
    };
    let Some(actioned) = world
        .get_resource::<ClientSystemFixtureActions>()
        .map(|copy| copy.is_actioned(system_fixture))
    else {
        error!("[birthday] ExecuteAsync: the client has no copy of userMysekaiSystemFixtureActions (no server model); nothing plays");
        return;
    };
    let given_name = match tables.given_name(party.unit) {
        Ok(name) => name.to_owned(),
        Err(reason) => {
            error!("[birthday] Initialize(party {}): {reason}; ShowConfirmDialog dereferences the character and throws: nothing plays", party.id);
            return;
        }
    };
    // ShowConfirmDialog(isActioned): ShowFirstConfirmDialog or
    // ShowAfterSecondTimeConfirmDialog.
    let (caller, body_key, ok) = if actioned {
        ("BirthdayPartyPresenter.ShowAfterSecondTimeConfirmDialog", CONFIRM_AFTER, "WORD_OK")
    } else {
        ("BirthdayPartyPresenter.ShowFirstConfirmDialog", CONFIRM_FIRST, START)
    };
    let format = crate::common_dialog::wording(world, caller, body_key);
    let body = moly_law::text::custom_text_mesh::format_wording(&format, &[given_name.clone()])
        .unwrap_or_else(|reason| {
            error!("[birthday] {caller}: {reason}; the body is drawn unformatted");
            format.clone()
        });
    let Some(dialog) = crate::common_dialog::open(
        world,
        caller,
        body,
        Buttons::Two { ok, cancel: "WORD_CANCEL" },
        Some(CONFIRM_INSTRUMENT),
    ) else {
        error!("[birthday] ExecuteAsync(party {}): the confirm dialog cannot show; nothing plays", party.id);
        return;
    };
    info!("[birthday] ExecuteAsync(party {}, fixture {fixture:?}): Initialize (character {given_name}); IsActionedSystemFixture({system_fixture}) = {actioned}; {caller}: Show2ButtonDialog(Common2ButtonDialog, {ok}, WORD_CANCEL), SetWordingText({body_key}, {given_name}) {dialog:?}", party.id);
    world.insert_resource(BirthdayConfirm {
        dialog,
        party,
        system_fixture,
        actioned,
        fixture,
        awaiting: None,
        rewards: None,
    });
}

/// `ShowConfirmDialog`'s answer: its cancel ends `ExecuteAsync`; its OK
/// sends the action (or, when the fixture was already actioned, goes on).
fn confirm_answer(world: &mut World) {
    let Some(dialog) = world.get_resource::<BirthdayConfirm>().map(|confirm| confirm.dialog) else {
        return;
    };
    let Some(ok) = crate::common_dialog::answer(world, dialog) else {
        return;
    };
    if !ok {
        world.remove_resource::<BirthdayConfirm>();
        info!("[birthday] ShowConfirmDialog = false (cancel): ExecuteAsync ends, nothing plays");
        return;
    }
    let Some(confirm) = world.get_resource::<BirthdayConfirm>() else {
        return;
    };
    if confirm.awaiting.is_some() {
        return;
    }
    if confirm.actioned {
        info!("[birthday] ShowConfirmDialog = true; the fixture is actioned: no ExecuteAPI, no rewards (the party ends without a reward dialog)");
        let confirm = world.remove_resource::<BirthdayConfirm>().expect("pending confirm");
        begin_party(world, confirm);
        return;
    }
    let system_fixture = confirm.system_fixture;
    let call = Call::SystemFixtureAction(SystemFixtureActionRequest {
        mysekai_system_fixture_id: system_fixture,
    });
    let kind = call.kind();
    let seq = world.resource_mut::<ServerLink>().call(call);
    if let Some(mut confirm) = world.get_resource_mut::<BirthdayConfirm>() {
        confirm.awaiting = Some(ServerPending {
            seq: Some(seq),
            call: kind,
        });
    }
    info!("[birthday] ShowConfirmDialog = true; MysekaiSystemFixtureActionService.ExecuteAPI({system_fixture}) sent as seq {seq}; the party continues on its reply");
}

/// Reads the answer to the confirm's action.
fn read_action_answer(
    mut answers: MessageReader<Answered>,
    confirm: Option<Res<BirthdayConfirm>>,
    mut commands: Commands,
) {
    for answer in answers.read() {
        let Some(pending) = confirm
            .as_deref()
            .and_then(|confirm| confirm.awaiting.as_ref())
        else {
            continue;
        };
        if pending.seq != Some(answer.seq) {
            continue;
        }
        let seq = answer.seq;
        let outcome = answer.outcome.clone();
        commands.queue(move |world: &mut World| finish_action(world, seq, outcome));
    }
}

/// The action's answer: a success keeps the reward rows and goes on; a
/// refusal is the source's null reply and stops the cut-scene.
fn finish_action(world: &mut World, seq: u32, outcome: Result<Reply, Refusal>) {
    let Some(confirm) = world.get_resource::<BirthdayConfirm>() else {
        return;
    };
    if confirm.awaiting.as_ref().and_then(|pending| pending.seq) != Some(seq) {
        return;
    }
    let system_fixture = confirm.system_fixture;
    match outcome {
        Ok(Reply::SystemFixtureAction(reply)) if reply.success => {
            let mut confirm = world.remove_resource::<BirthdayConfirm>().expect("pending confirm");
            confirm.awaiting = None;
            confirm.rewards = Some(reply.obtained_resources);
            info!("[birthday] ShowConfirmDialog = true; MysekaiSystemFixtureActionService.ExecuteAPI({system_fixture}): success; updatedResources merged (IsActionedSystemFixture now {}); obtainedResources {} rows kept for the party's reward dialog", world.get_resource::<ClientSystemFixtureActions>().is_some_and(|copy| copy.is_actioned(system_fixture)), confirm.rewards.as_ref().map_or(0, Vec::len));
            begin_party(world, confirm);
        }
        Ok(Reply::SystemFixtureAction(reply)) => {
            error!("[birthday] MysekaiSystemFixtureActionService.ExecuteAPI({system_fixture}): the reply is null ({}): the API error (UI, not drawn); ExecuteAsync stops, nothing plays", reply.refusal.unwrap_or_default());
            world.remove_resource::<BirthdayConfirm>();
        }
        Ok(other) => {
            error!("[birthday] MysekaiSystemFixtureActionService.ExecuteAPI({system_fixture}): answered an unexpected {other:?}; ExecuteAsync stops, nothing plays");
            world.remove_resource::<BirthdayConfirm>();
        }
        Err(refusal) => {
            error!("[birthday] MysekaiSystemFixtureActionService.ExecuteAPI({system_fixture}): the reply is null ({}): the API error (UI, not drawn); ExecuteAsync stops, nothing plays", refusal.reason);
            world.remove_resource::<BirthdayConfirm>();
        }
    }
}

/// `ExecuteAsync` after the confirm, up to `PlayCutSceneAsync`.
fn begin_party(world: &mut World, confirm: BirthdayConfirm) {
    let BirthdayConfirm {
        rewards,
        party,
        fixture,
        ..
    } = confirm;
    let rows = match tables(world) {
        Ok(Some(tables)) => tables,
        Ok(None) => {
            error!("[birthday] ExecuteAsync: the birthday tables are not taken; nothing plays");
            return;
        }
        Err(reason) => {
            error!("[birthday] ExecuteAsync: {reason}; nothing plays");
            return;
        }
    };
    world.insert_resource(BirthdayContext { party: party.id });
    info!("[birthday] MultiplayCore.IsConnected (the server model's own room); GameStateManager.AddContext(Birthday) (MysekaiBirthdayPartyContext.OnEnter: InProgress; ActionButtonRefresh with its disabled types); no other players to hide");
    if let Some(mut screens) = world.get_resource_mut::<crate::ui_layers::ScreenManager>() {
        screens.set_mysekai_back_key_process(
            crate::ui_layers::MysekaiBackKeyProcess::None,
            "BirthdayPartyPresenter.ExecuteAsync",
        );
    }
    // PlayCutSceneAsync.
    let Some((cut_scene_id, _, timeline)) = rows
        .cut_scenes
        .iter()
        .find(|(_, external, _)| *external == party.id)
        .cloned()
    else {
        error!("[birthday] PlayCutSceneAsync: GetMasterMysekaiBirthdayCutSceneByExternalId({}) = null; the source dereferences it and throws: nothing plays", party.id);
        world.remove_resource::<BirthdayContext>();
        return;
    };
    let play = CastPlay {
        caller: CastCaller::Birthday,
        units: vec![party.unit],
        cut_scene_id,
        timeline,
        start: fixture,
        use_already_exist_character: true,
        hide_player: false,
        show_ui: true,
    };
    match super::play_async(world, play) {
        Ok(()) => {
            world.insert_resource(BirthdayPresenter {
                party,
                fixture,
                rewards,
            });
        }
        Err(reason) => {
            error!("[birthday] PlayCutSceneAsync: {reason}; nothing plays");
            world.remove_resource::<BirthdayContext>();
        }
    }
}

/// `OnStartFadeInCallBack`, at the start of `SetupInternal`.
pub(super) fn start_fade_in_callback(world: &mut World, cast: &Cast) {
    info!("[birthday] OnStartFadeInCallBack: DownloadVoiceAsync (bundle mysekai/talk/birthday/voice/{}: the voice track plays it from the root's stream table)", cast.play.timeline);
    // DestroyCharacters runs before CreateBirthdayCharacter, so it disposes
    // every NPC present then. The host created the party's character while
    // the package loaded (see `super::cast`), so that one is kept.
    let members = cast.members();
    let caller = "BirthdayPartyPresenter.DestroyCharacters";
    let mut disposed = Vec::new();
    for (unit, entity) in crate::npc::dispose::npc_list(world) {
        if members.iter().any(|(_, member)| *member == entity) {
            continue;
        }
        match crate::npc::dispose::dispose_npc(world, entity, caller) {
            Ok(unit) => disposed.push(unit),
            Err(reason) => warn!("[birthday] {caller}: unit {unit}: {reason}"),
        }
    }
    // DisposeNPC takes a unit off the roster; the party's character keeps
    // its own registration.
    if let Some(mut registry) = world.get_resource_mut::<crate::npc::Registry>() {
        for (unit, _) in &members {
            if !registry.character_unit_ids.contains(unit) {
                registry.character_unit_ids.push(*unit);
            }
        }
        registry.character_unit_ids.sort_unstable();
    }
    info!("[birthday] DestroyCharacters: every NPC warps away and is destroyed: units {disposed:?}; CreateBirthdayCharacter(unit {:?}): the new NPC {members:?}, whose AI runs as any NPC's", cast.play.units);
}

/// `OnEndFadeOutCallback`, after the view is disposed.
///
/// The source places both avatars in world space from the fixture's
/// position `p` and `GetFixtureAngle` `a` (the fixture's yaw: its direction
/// times 90 degrees): the character at `p + len (cos(a + o), 0,
/// -sin(a + o))` facing `-forward`, the player at `p + 0.5 (-sin a, 0,
/// -cos a)` looking at `p`. Since the fixture's forward is `(sin a, 0, cos
/// a)`, both are fixed poses in the fixture's own frame: the character at
/// yaw `90 + o` degrees and distance `len`, turned 180 degrees; the player
/// at yaw 180 degrees and distance 0.5, facing the fixture. They are placed
/// through the fixture's pose as the view's nodes are.
pub(super) fn end_fade_out_callback(world: &mut World, cast: &Cast) {
    let Some(presenter) = world.get_resource::<BirthdayPresenter>() else {
        error!("[birthday] OnEndFadeOutCallback: no birthday presenter");
        return;
    };
    let (fixture, unit) = (presenter.fixture, presenter.party.unit);
    let Some((_, character)) = cast.members().into_iter().find(|(u, _)| *u == unit) else {
        error!("[birthday] OnEndFadeOutCallback: FindNPC({unit}) found no character");
        return;
    };
    let Some(pose) = world
        .get::<GlobalTransform>(fixture)
        .map(GlobalTransform::compute_transform)
    else {
        error!("[birthday] OnEndFadeOutCallback: the party fixture {fixture:?} has no pose");
        return;
    };
    let place =
        |yaw_deg: f32, length: f32, facing_deg: f32| beside(&pose, yaw_deg, length, facing_deg);
    crate::npc::gate_entries::show(world, character);
    // PutCharacterBesideFixture.
    let (offset, length) = if unit == NIGO_MIKU_UNIT {
        (NIGO_MIKU_ANGLE, NIGO_MIKU_LENGTH)
    } else {
        (CHARACTER_ANGLE, CHARACTER_LENGTH)
    };
    let (at, rotation) = place(90.0 + offset, length, 180.0);
    crate::npc::force_set_position(world, character, at.to_array());
    if let Some(mut walk) = world.get_mut::<crate::npc::WalkState>(character) {
        walk.0.forward = (rotation * Vec3::Z).to_array();
    }
    if let Some(mut transform) = world.get_mut::<Transform>(character) {
        transform.translation = at;
        transform.rotation = rotation;
    }
    info!("[birthday] OnEndFadeOutCallback: FindNPC({unit}).Show(); PutCharacterBesideFixture: {length} m at {:.1} deg in the fixture's frame, facing away from its forward -> ({:.3},{:.3},{:.3}); home screen HideUI (UI, not drawn)", 90.0 + offset, at.x, at.y, at.z);
    put_and_look_player_beside_fixture(world, fixture);
}

/// `PutAndLookPlayerBesideFixture`: the player 0.5 m behind the fixture's
/// forward, facing it (see [`end_fade_out_callback`]).
pub(super) fn put_and_look_player_beside_fixture(world: &mut World, fixture: Entity) {
    let Some(pose) = world
        .get::<GlobalTransform>(fixture)
        .map(GlobalTransform::compute_transform)
    else {
        error!("[birthday] PutAndLookPlayerBesideFixture: the party fixture {fixture:?} has no pose");
        return;
    };
    let (at, rotation) = beside(&pose, 180.0, PLAYER_LENGTH, 0.0);
    let player = player_entity(world);
    if let Some(mut transform) = player.and_then(|player| world.get_mut::<Transform>(player)) {
        transform.translation = at;
        transform.rotation = rotation;
    }
    info!("[birthday] PutAndLookPlayerBesideFixture: player {player:?} ForceSetPosition {PLAYER_LENGTH} m behind the fixture's forward -> ({:.3},{:.3},{:.3}), LookAt(the fixture)", at.x, at.y, at.z);
}

/// `PlayCutSceneAsync` returned: `ExecuteAsync` goes on with the party (see
/// [`super::birthday_party`]).
pub(super) fn cut_scene_returned(world: &mut World, cast: &Cast) {
    let presenter = world.remove_resource::<BirthdayPresenter>();
    let outcome = cast.outcome.clone().unwrap_or(Ok(()));
    info!(
        "[birthday] PlayCutSceneAsync returned {outcome:?} (party {:?})",
        presenter.as_ref().map(|presenter| presenter.party.id)
    );
    match (outcome, presenter) {
        (Ok(()), Some(presenter)) => {
            let character = cast
                .members()
                .into_iter()
                .find(|(unit, _)| *unit == presenter.party.unit)
                .map(|(_, entity)| entity);
            info!("[birthday] the party's character: {character:?}");
            super::birthday_party::start(
                world,
                super::birthday_party::Party {
                    id: presenter.party.id,
                    unit: presenter.party.unit,
                    fixture: presenter.fixture,
                },
                presenter.rewards,
            );
        }
        (Ok(()), None) => {
            error!("[birthday] PlayCutSceneAsync returned with no presenter; the party cannot go on");
            world.remove_resource::<BirthdayContext>();
        }
        (Err(_), _) => {
            // The cut-scene did not play: nothing of the party is under way.
            world.remove_resource::<BirthdayContext>();
        }
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<BirthdayCharset>()
        .add_message::<BirthdayCutSceneClick>()
        .add_systems(Startup, load)
        .add_systems(Update, advance.before(super::advance))
        .add_systems(Update, read_action_answer.after(advance));
}
