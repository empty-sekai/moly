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
//!   time), else the "party closed" dialog and an action button refresh;
//!   the fixture's cut-scene area must lie inside the grid and overlap no
//!   other fixture, else the overlap dialog; then `ExecuteAsync`.
//! - `ExecuteAsync` (call order): `Initialize` (party, unit,
//!   character), `IsActionedSystemFixture` (the user's
//!   `userMysekaiSystemFixtureActions`), `ShowConfirmDialog(isActioned)`,
//!   `MysekaiSystemFixtureActionService.ExecuteAPI`, `AddContext(Birthday)`,
//!   the multiplayer hides (single player: not connected), then
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
//! clock), the placed party fixture (the served home housing layout, whose
//! party cakes are its `policies.birthdayPartyFixtures`), the user's
//! `userMysekaiSystemFixtureActions` rows (the client's copy
//! [`ClientSystemFixtureActions`]) and the
//! `PostUserMysekaiSystemFixtureActionApi` reply
//! (`policies.systemFixtureActionReply`; see
//! [`crate::server::system_fixture_action`]). A refused reply is the source's
//! null reply: `ExecuteAsync` stops before its cut-scene.
//!
//! Named differences: the confirm dialog, the closed and overlap dialogs and
//! the home screen's `HideUI` are UI not drawn here (the confirm is taken as
//! pressed); the cut-scene area check is not modelled; the birthday
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

use bevy::ecs::system::SystemState;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_law::action_button::ButtonType;
use serde_json::Value;

use super::{Cast, CastCaller, CastPlay};
use crate::fixture_activity_state::FixtureActivityIdentity;
use crate::server::client::system_fixture_action::{self as action, ClientSystemFixtureActions};

/// `MysekaiCutSceneConditionType` of the birthday rows.
const CONDITION: &str = "birthday_party";
/// `MysekaiSystemFixtureType` of a party fixture.
const PARTY_FIXTURE_TYPE: &str = "birthday";
const INSTRUMENT: &str = "MOLY_BIRTHDAY_PARTY_ACTION";
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

/// The three master tables, read from the region's master hosts.
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

/// The tables: requested, then taken once all three have resolved (a
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

type Tables = (Vec<Party>, Vec<SystemFixture>, Vec<(i64, i64, String)>);

/// The party rows, the system fixture rows and the birthday cut-scene rows,
/// once the master layer has resolved all three.
fn tables(world: &mut World) -> Result<Option<Tables>, String> {
    match world.get_resource::<BirthdayTables>() {
        None => return Err("the birthday tables were not requested".into()),
        Some(BirthdayTables::Taken(taken)) => return taken.clone().map(Some),
        Some(BirthdayTables::Requested) => {}
    }
    let mut masters = world.resource_mut::<MasterData>();
    let keys = [PARTIES.key(), SYSTEM_FIXTURES.key(), CUT_SCENES.key()];
    if keys.into_iter().any(|key| !masters.is_resolved(key)) {
        return Ok(None);
    }
    let (Some(parties), Some(system), Some(cut_scenes)) = (
        masters.take(&PARTIES),
        masters.take(&SYSTEM_FIXTURES),
        masters.take(&CUT_SCENES),
    ) else {
        return Err("a birthday master table was taken by another reader".into());
    };
    let taken = (|| Ok((parties?, system?, cut_scenes?)))()
        .map_err(|error: master::MasterError| error.to_string());
    if let Ok((parties, system, cut_scenes)) = &taken {
        info!(
            "[birthday] master tables: {} parties, {} system fixtures ({} of type {PARTY_FIXTURE_TYPE}), {} birthday cut-scenes",
            parties.len(),
            system.len(),
            system.iter().filter(|row| row.kind == PARTY_FIXTURE_TYPE).count(),
            cut_scenes.len()
        );
    }
    world.insert_resource(BirthdayTables::Taken(taken.clone()));
    taken.map(Some)
}

/// Update (exclusive): the party fixtures' button state, the button's
/// clicks, and the instrument's press.
pub(super) fn advance(world: &mut World) {
    update_buttons(world);
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
    let Ok(Some((parties, system, _))) = tables(world) else {
        return;
    };
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
    let (parties, system, _) = match tables(world) {
        Ok(Some(tables)) => tables,
        Ok(None) => return,
        Err(reason) => {
            error!("[birthday] {INSTRUMENT}: {reason}; no press");
            return set_stage(world, InstrumentStage::Done);
        }
    };
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
    let (parties, system, cut_scenes) = tables;
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
        info!("[birthday] OnClickBirthdayCutScene: party {} is not within its birthday start time at {now} ([{}, {})): ShowCommon1ButtonDialog(MSG_MYSEKAI_BIRTHDAY_PARTY_CLOSED) (UI, not drawn); publish ActionButtonRefresh", party.id, party.birthday_start_at, party.closed_at);
        return;
    }
    info!("[birthday] OnClickBirthdayCutScene: party {} (unit {}, window [{}, {}), birthday start {}) at {now}: IsCutsceneAreaOverlap / IsCutsceneAreaWithinGrid are not modelled (taken as clear); ExecuteAsync", party.id, party.unit, party.start_at, party.closed_at, party.birthday_start_at);
    execute(world, party, row, fixture, &cut_scenes);
}

/// `ExecuteAsync` up to `PlayCutSceneAsync`.
fn execute(
    world: &mut World,
    party: Party,
    row: SystemFixture,
    fixture: Entity,
    cut_scenes: &[(i64, i64, String)],
) {
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
    info!("[birthday] ExecuteAsync(party {}, fixture {fixture:?}): Initialize; IsActionedSystemFixture({system_fixture}) = {actioned}; ShowConfirmDialog({actioned}): {} confirm dialog (UI, not drawn; taken as confirmed)", party.id, if actioned { "the after-second-time" } else { "the first" });
    let reply = action::post(world, system_fixture);
    if !reply.success {
        error!("[birthday] MysekaiSystemFixtureActionService.ExecuteAPI({system_fixture}): the reply is null ({}): the API error (UI, not drawn); ExecuteAsync stops, nothing plays", reply.refusal.unwrap_or_default());
        return;
    }
    info!("[birthday] MysekaiSystemFixtureActionService.ExecuteAPI({system_fixture}): success; updatedResources merged (IsActionedSystemFixture now {}); obtainedResources {} rows kept for the reward dialog (not run here)", world.get_resource::<ClientSystemFixtureActions>().is_some_and(|copy| copy.is_actioned(system_fixture)), reply.obtained_resources.len());
    world.insert_resource(BirthdayContext { party: party.id });
    info!("[birthday] GameStateManager.AddContext(Birthday); MultiplayCore not connected: no other players to hide; back key process cleared");
    // PlayCutSceneAsync.
    let Some((cut_scene_id, _, timeline)) = cut_scenes
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
            world.insert_resource(BirthdayPresenter { party, fixture });
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
    let disposed =
        crate::npc::dispose::dispose_npc_all(world, "BirthdayPartyPresenter.DestroyCharacters");
    let avatars = cast.avatars();
    info!("[birthday] DestroyCharacters: every NPC warps away and is destroyed: units {disposed:?}; CreateBirthdayCharacter(unit {:?}): the cast's new avatar {:?} stands for it (FindNPC takes it over)", cast.play.units, avatars);
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
    let Some((_, character)) = cast.avatars().into_iter().find(|(u, _)| *u == unit) else {
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
    world.entity_mut(character).insert(Visibility::Inherited);
    // PutCharacterBesideFixture.
    let (offset, length) = if unit == NIGO_MIKU_UNIT {
        (NIGO_MIKU_ANGLE, NIGO_MIKU_LENGTH)
    } else {
        (CHARACTER_ANGLE, CHARACTER_LENGTH)
    };
    let (at, rotation) = place(90.0 + offset, length, 180.0);
    if let Some(mut transform) = world.get_mut::<Transform>(character) {
        transform.translation = at;
        transform.rotation = rotation;
    }
    info!("[birthday] OnEndFadeOutCallback: FindNPC({unit}).Show(); PutCharacterBesideFixture: {length} m at {:.1} deg in the fixture's frame, facing away from its forward -> ({:.3},{:.3},{:.3}); home screen HideUI (UI, not drawn)", 90.0 + offset, at.x, at.y, at.z);
    // PutAndLookPlayerBesideFixture.
    let (at, rotation) = place(180.0, PLAYER_LENGTH, 0.0);
    let player = player_entity(world);
    if let Some(mut transform) = player.and_then(|player| world.get_mut::<Transform>(player)) {
        transform.translation = at;
        transform.rotation = rotation;
    }
    info!("[birthday] PutAndLookPlayerBesideFixture: player {player:?} ForceSetPosition {PLAYER_LENGTH} m behind the fixture's forward -> ({:.3},{:.3},{:.3}), LookAt(the fixture)", at.x, at.y, at.z);
}

/// `PlayCutSceneAsync` returned: `ExecuteAsync` goes on with the party.
pub(super) fn cut_scene_returned(world: &mut World, cast: &Cast) {
    let presenter = world.remove_resource::<BirthdayPresenter>();
    let outcome = cast.outcome.clone().unwrap_or(Ok(()));
    info!(
        "[birthday] PlayCutSceneAsync returned {outcome:?} (party {:?}); the party's own sequence (CacheBirthdayBgm, ChangeUIForBirthday, ForceBirthdayPartyTalk(1), SetCharacterWaitTalkObjective(2), WaitEndBirthdayPartyAsync, the closing fades, ForceBirthdayPartyTalk(3), DeleteBirthdayCharacters, RestoreCharacters, the site BGM, RemoveContext, the reward dialog) is not run here (named)",
        presenter.as_ref().map(|presenter| presenter.party.id)
    );
    if outcome.is_err() {
        // The cut-scene did not play: nothing of the party is under way.
        world.remove_resource::<BirthdayContext>();
    }
}

pub(super) fn install(app: &mut App) {
    app.add_message::<BirthdayCutSceneClick>()
        .add_systems(Startup, load)
        .add_systems(Update, advance.before(super::advance));
}
