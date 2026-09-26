//! The gate's presentations after a server reply: the invite cut-scene and
//! the gate change with its visitors coming out of the gate (JP 6.8.1).
//!
//! Invite (`ScreenLayerMysekaiGateInvitationPresenter.ReserveProcessAsync`):
//! the pass check (`HasMysekaiColorfulPass`, else the expired-pass dialog and
//! no request), the reserve request with the selected unit as the unit
//! group, then, on the reply, `IsExistNPC(unit)`: a present unit only
//! refreshes the screen (UI); otherwise `MysekaiBootData.AddGateCharacters`
//! and `MysekaiTalkDataStore.UpdateTalkList` with the reply's talk list, the
//! view's fade out and the send dialog (UI), and `PlayInviteCutSceneAsync`:
//! the master cut-scene of the invite condition for the unit, played at the
//! gate's view transform with a new avatar and the player hidden; its end
//! callback (`OnEndCutScene`) disposes the cut-scene avatar, creates the
//! unit's NPC and whites the screen out at once (`WhiteOut(0, 0)`). Then
//! `WhiteIn(0, 0.2)` and a 1.0 s delay.
//!
//! Gate change (`MysekaiGateUtility.ExecuteChangeGateProcessAsync`, after the
//! change reply): `TryShowGoHomeCutSceneAsync` picks one present NPC
//! (`RandomPick` over the NPC list: `UnityEngine.Random.Range(0, count)`),
//! finds the leave cut-scene of its unit, disposes every NPC
//! (`DisposeNPCAll`) and plays it at the gate with a new avatar (its end
//! callback disposes every NPC again); then `FadeOutLayer`,
//! `ChangeGateAction` (`BackUIScreen`, `SiteLayoutUtility.ChangeGate`: the
//! gate shows the new gate's model and `se_gate_change` plays; a 1.0 s
//! delay), `StartCharacterAppearanceAsync` (not awaited:
//! `MysekaiTalkDataStore.SetTalkList` with the reply's talk list, the new
//! visitors created, then `PlayGateCharacterAppearTimeline`:
//! `ShowEffect(fx_fixture_home_gate_stay)`; each visitor's
//! `PlayGateCharacterAppearTimelineInternal` (`se_gate_chime`, 0.2 s, the
//! visitor's forced fixture timeline on the gate: `tl_visitgate_w_001` for
//! the first, `tl_visitgate_w_002` for the others, of the literal package
//! `mdl_non0006_gate_lon1`), 1000 ms apart; `HideEffect(...).Forget()`;
//! `PlayCloseGateAsync(tl_visitgate_w_003)` with no NPC), then `FadeInLayer`.
//!
//! `FixtureController.HideEffect` walks the active particle systems of the
//! view named so, one after another: `Stop()` (children included, emission
//! stopped, live particles finish), a wait on the Update loop while
//! `IsAlive(withChildren: false)` (the one system playing or holding a
//! particle), then `SetActive(false)`, which hides whatever the children
//! still hold.
//!
//! The model of the gate: `MysekaiGateModel.AssetBundleName` is the skin's
//! bundle (the skin row's type selects the unit or the common skin table)
//! when the gate has a skin (id > 0), else the gate row's own bundle; the
//! fixture factory loads `mysekai/fixture/<bundle>`.
//!
//! Server values come from the server model ([`crate::server`]): its gate
//! replies ([`crate::server::ServerGateReplies`]) are consumed here, whoever
//! made the request (the server panel's gate actions, or the native
//! instruments below). The talk lists of both replies go to the client's talk
//! store through [`crate::server::client_update_talk_list`]. Native request
//! instruments (the gate screens' requests; those screens are the UI
//! group's):
//! - `MOLY_GATE_INVITE=<unit>[,pass]`: the reserve request for the selected
//!   unit (the unit group of a single unit has the unit's id); `pass` first
//!   gives the user a colorful pass through the server panel's edit (the
//!   product default has none, and the reserve needs one);
//! - `MOLY_GATE_CHANGE=<mysekaiGateId>[/<mysekaiGateSkinId>]:<group>,...`:
//!   the gate change, with the server's answer stated (the new gate, its
//!   skin and the visiting unit groups are server values).
//! Each request is made once the home site stands with its gate and its
//! visitors placed and the client has joined, after a 2 s settle (a product
//! choice). A reply waits for the same readiness, without the settle.
//!
//! NPC-side calls go through the NPC runtime's entries (`npc::gate_entries`,
//! `npc::dispose`): the avatar store's NPC list; DisposeNPCAll; the invited
//! unit's CreateNPC at the gate's first action point end locator, its two
//! waits and its TryCancelCurrentObjective, ForceUpdateObjective and Show;
//! the go-home start callback's HideAndCancelObjective.
//!
//! Not routed, and named at their call sites with the source's arguments
//! (the NPC decision ladder has no route yet for the immediately-played
//! fixture timeline objective, 12): each visitor's
//! `ForceUpdateObjectiveImmediatelyTimeline` with the calls around it
//! (TryCancelCurrentObjective, SetImmediatelyExecuteNextObjective,
//! SetCanInterruptTalkData, the waits on the NPC's fixture timeline,
//! ReleaseUsingFixture, the second cancel) and `TryCancelIfFixtureActionGate`.
//! Meanwhile SetupNPC's CreateNPC without a pose is the NPC runtime's member
//! creation (`npc::spawn_temporary_units`, where new members are seated; the
//! entries have no CreateNPC without a pose), and each visitor timeline's
//! own gate, SE and Control tracks play on the gate with no NPC.

use std::collections::HashSet;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::objective::appearance::EngineRand;
use serde_json::Value;

use crate::cutscene::{Cast, CastCaller, CastPlay};
use crate::fixture_activity_provider::FixtureActivityProvider;
use crate::fixture_activity_state::{FixtureActivityIdentity, FixtureActivityOwner};
use crate::fixture_activity_timeline::{
    self as timeline, FixtureActivityTimelines, StartTimeline, TimelineBindings,
    TimelineDefinition, TimelineOwner, TimelineOwnerKind, TimelineStatus, TimelineTimeoutBudget,
    TimelineToken,
};
use crate::npc::CharacterUnitId;
use crate::server::{GateCharacter, GateReply, ReplyTalkList, TalkListUpdate};

const INVITE: &str = "MOLY_GATE_INVITE";
const CHANGE: &str = "MOLY_GATE_CHANGE";
const PLAYER_DATA: &str = "moly://fixture-models/player-data.json";
const FIXTURE_INDEX: &str = "moly://fixture-models/index.json";
const INVITE_CONDITION: &str = "mysekai_character_talk_character_invite_game_character_unit_id";
const LEAVE_CONDITION: &str = "mysekai_character_talk_character_leave_game_character_unit_id";
/// The literal fixture-timeline bundle of the visitor timelines
/// (`mysekai/fixture_timeline/mdl_non0006_gate_lon1`), whatever the skin.
const VISIT_PACKAGE: &str = "mysekai__fixture_timeline__mdl_non0006_gate_lon1";
/// The bundle name the gate controller passes with every visitor timeline
/// (a string literal, whichever gate model is shown).
const VISIT_BUNDLE: &str = "mysekai/fixture_timeline/mdl_non0006_gate_lon1";
const FIRST_VISIT: &str = "tl_visitgate_w_001";
const NEXT_VISIT: &str = "tl_visitgate_w_002";
const CLOSE_GATE: &str = "tl_visitgate_w_003";
const STAY_EFFECT: &str = "fx_fixture_home_gate_stay";
const SE_CHIME: &str = "se_gate_chime";
const SE_CHANGE: &str = "se_gate_change";
/// `ReserveProcessAsync`: `WhiteIn(0, 0.2)`, then `Delay(1.0 s)`.
const WHITE_IN: f32 = 0.2;
const AFTER_WHITE_IN: f64 = 1.0;
/// `ChangeGateAction`: `Delay(1.0 s)` after the change.
const AFTER_CHANGE: f64 = 1.0;
/// `PlayGateCharacterAppearTimeline`: `Delay(1000 ms)` after each visitor.
const VISITOR_INTERVAL: f64 = 1.0;
/// `PlayGateCharacterAppearTimelineInternal`: `Delay(0.2 s)` after the chime.
const CHIME_DELAY: f64 = 0.2;
/// Product settle before an instrument's reply is delivered (not a source
/// value).
const SETTLE: f64 = 2.0;
/// `PlayableDirector.Play` has no timeout.
const NO_TIMEOUT: f64 = f64::MAX;
const WAITS_FOR_EARLIER: &str = "waits for the earlier gate part";

/// The bundles a gate fixture may show (`MysekaiGateModel.AssetBundleName`
/// over the gate, unit skin and common skin tables), as fixture packages.
#[derive(Resource, Default, Debug)]
pub(crate) struct GateModelPackages(pub(crate) HashSet<String>);

/// The home gate's model as the server document states it
/// (`userMysekaiGates`: the gate set at the home site and its skin), which
/// the home site's layout shows on its gate rows. `package` is none when the
/// server document or the gate tables are not available, or when the
/// fixture index lacks the model; the layout's own row stands then.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub(crate) struct HomeGateModel {
    pub(crate) package: Option<String>,
    /// The fixture masters whose type is gate.
    pub(crate) masters: HashSet<i32>,
}

/// A request of the native instruments (the gate screens' requests).
#[derive(Clone, Debug)]
enum Request {
    /// `ExecutePostUserMysekaiGateReserveApi(SelectedGameCharacterUnitId)`.
    /// `grant_pass`: the server panel's edit first gives the user a colorful
    /// pass (a server value; the product default has none), delivered by
    /// its `sync` action.
    Reserve { unit: i32, grant_pass: bool },
    /// The gate change, with the server's answer stated.
    Change {
        gate: i32,
        skin: i32,
        groups: Vec<i32>,
    },
}

impl Request {
    /// The server model's `server.edit` command for the request.
    fn command(&self) -> serde_json::Map<String, Value> {
        let mut map = serde_json::Map::new();
        map.insert("type".into(), "server.edit".into());
        match self {
            Self::Reserve { unit, .. } => {
                map.insert("action".into(), "gate.reserve".into());
                map.insert("mysekaiGameCharacterUnitGroupId".into(), (*unit).into());
            }
            Self::Change { gate, skin, groups } => {
                map.insert("action".into(), "gate.change".into());
                map.insert("mysekaiGateId".into(), (*gate).into());
                map.insert("mysekaiGateSkinId".into(), (*skin).into());
                map.insert(
                    "mysekaiGameCharacterUnitGroupIds".into(),
                    groups.iter().copied().map(Value::from).collect(),
                );
            }
        }
        map
    }
}

struct Tables {
    /// (id, condition type, externalId, timelineAssetbundleName).
    cut_scenes: Vec<(i64, String, i64, String)>,
    /// Gate id -> bundle.
    gates: Vec<(i64, String)>,
    /// Skin id -> (type, type id).
    skins: Vec<(i64, String, i64)>,
    unit_skins: Vec<(i64, String)>,
    common_skins: Vec<(i64, String)>,
    /// The masters whose fixture type is gate.
    gate_masters: HashSet<i32>,
}

impl Tables {
    fn parse(document: &Value) -> Result<Self, String> {
        let tables = &document["tables"];
        let rows = |name: &str| -> Result<&Vec<Value>, String> {
            tables[name]
                .as_array()
                .ok_or_else(|| format!("the catalog has no {name} table"))
        };
        let text = |row: &Value, key: &str| row[key].as_str().unwrap_or_default().to_owned();
        let id = |row: &Value, key: &str| row[key].as_i64().unwrap_or(0);
        let pairs = |name: &str| -> Result<Vec<(i64, String)>, String> {
            Ok(rows(name)?
                .iter()
                .map(|row| (id(row, "id"), text(row, "assetbundleName")))
                .collect())
        };
        Ok(Self {
            cut_scenes: rows("mysekaiCutScenes")?
                .iter()
                .map(|row| {
                    (
                        id(row, "id"),
                        text(row, "mysekaiCutSceneConditionType"),
                        id(row, "externalId"),
                        text(row, "timelineAssetbundleName"),
                    )
                })
                .collect(),
            gates: pairs("mysekaiGates")?,
            skins: rows("mysekaiGateSkins")?
                .iter()
                .map(|row| {
                    (
                        id(row, "id"),
                        text(row, "mysekaiGateSkinType"),
                        id(row, "mysekaiGateSkinTypeId"),
                    )
                })
                .collect(),
            unit_skins: pairs("mysekaiGateUnitSkins")?,
            common_skins: pairs("mysekaiGateCommonSkins")?,
            gate_masters: rows("mysekaiFixtures")?
                .iter()
                .filter(|row| row["mysekaiFixtureType"].as_str() == Some("gate"))
                .map(|row| id(row, "id") as i32)
                .collect(),
        })
    }

    /// `GetMasterMysekaiCutScene(externalId, condition)`.
    fn cut_scene(&self, condition: &str, external: i64) -> Option<(i64, String)> {
        self.cut_scenes
            .iter()
            .find(|(_, kind, id, _)| kind == condition && *id == external)
            .map(|(id, _, _, timeline)| (*id, timeline.clone()))
    }

    /// `MysekaiGateModel.AssetBundleName`.
    fn gate_bundle(&self, gate: i64, skin: i64) -> Result<String, String> {
        if skin > 0 {
            let (_, kind, type_id) = self
                .skins
                .iter()
                .find(|(id, _, _)| *id == skin)
                .ok_or_else(|| format!("no mysekaiGateSkins row {skin}"))?;
            let table = match kind.as_str() {
                "unit" => &self.unit_skins,
                "common" => &self.common_skins,
                other => return Err(format!("gate skin {skin} has type {other}")),
            };
            return table
                .iter()
                .find(|(id, _)| id == type_id)
                .map(|(_, bundle)| bundle.clone())
                .ok_or_else(|| format!("no {kind} gate skin row {type_id}"));
        }
        self.gates
            .iter()
            .find(|(id, _)| *id == gate)
            .map(|(_, bundle)| bundle.clone())
            .ok_or_else(|| format!("no mysekaiGates row {gate}"))
    }

    fn model_packages(&self) -> HashSet<String> {
        self.gates
            .iter()
            .chain(&self.unit_skins)
            .chain(&self.common_skins)
            .map(|(_, bundle)| format!("mysekai__fixture__{bundle}"))
            .collect()
    }
}

/// One visitor's `PlayGateCharacterAppearTimelineInternal`.
struct Visitor {
    unit: u32,
    timeline: &'static str,
    started: f64,
    chimed: bool,
    request: Option<StartTimeline>,
    /// Tracks left out of the gate part so far, for the logs.
    left_out: Vec<String>,
    /// How far the gate part stepped down while another timeline holds the
    /// gate: 0 whole, 1 without the fixture track, 2 the SE tracks alone.
    fallback: u8,
    /// The request last handed to the runner, kept for a step-down.
    started_request: Option<StartTimeline>,
    token: Option<TimelineToken>,
    reported: Option<String>,
    done: bool,
}

enum Stage {
    /// Nothing in flight: a gate reply is taken once the field is ready.
    Idle,
    /// An instrument's request waits for the home site, its gate, its
    /// visitors and the client's join.
    Request {
        request: Request,
        ready_since: Option<f64>,
    },
    /// `PlayInviteCutSceneAsync` in flight.
    InviteCutScene,
    /// `WhiteIn(0, 0.2)`, then `Delay(1.0 s)` until `until`.
    InviteWhiteIn { until: f64 },
    /// `TryShowGoHomeCutSceneAsync` in flight.
    GoHome {
        gate: i64,
        skin: i64,
        change: Change,
    },
    /// `ChangeGateAction`'s delay after the change, until `until`.
    ChangeGate { change: Change, until: f64 },
    /// `StartCharacterAppearanceAsync` and the timelines out of the gate.
    Appearance(Box<Appearance>),
}

/// What the change reply hands on to `StartCharacterAppearanceAsync`.
struct Change {
    /// The reply's `userMysekaiGateCharacters`.
    rows: Vec<GateCharacter>,
    /// Their units after the client's group expansion (row order, no
    /// repeats).
    units: Vec<u32>,
    /// The reply's `mysekaiCharacterTalkWithReadHistories`.
    talks: ReplyTalkList,
}

struct Appearance {
    units: Vec<u32>,
    /// The loop's clock: the next visitor starts at this time.
    next_at: f64,
    visitors: Vec<Visitor>,
    stay: Option<crate::fixture_timeline_particles::ParticlePlayBinding>,
    stay_node: Option<Entity>,
    hidden: bool,
    close: Option<Visitor>,
    spawned: bool,
    generation: u64,
    /// The gate parts whose assets are loaded ahead of their start.
    warmed: Vec<&'static str>,
    /// The last reason the appearance waited, for the logs.
    waiting: Option<String>,
}

impl Appearance {
    fn wait(&mut self, t: f64, reason: String) {
        if self.waiting.as_deref() != Some(reason.as_str()) {
            info!("[gate] t={t:.3} PlayGateCharacterAppearTimeline waits: {reason}");
            self.waiting = Some(reason);
        }
    }
}

/// `HideEffect(name).Forget()` in flight on one stopped system: from the
/// frame after its `Stop()`, the wait on the Update loop while it is alive,
/// then `SetActive(false)`.
struct PendingHide {
    node: Entity,
    stopped_frame: u64,
    stopped_at: f64,
}

#[derive(Resource)]
pub(crate) struct GateFlow {
    stage: Stage,
    tables: Option<Handle<JsonAsset>>,
    parsed: Option<Tables>,
    /// The fixture package index, read once for the packages it lists.
    index: Option<Handle<JsonAsset>>,
    indexed: Option<HashSet<String>>,
    /// `OnEndCutScene`'s `CreateNPC` after its `Yield`: the units and the
    /// frame the cut-scene avatars were disposed on.
    create_after_yield: Option<(Vec<u32>, u64)>,
    /// `OnEndCutScene` after `CreateNPC`: the created NPCs whose
    /// `WaitUntil(IsExistNPCAll)` / `WaitWhile(CurrentObjectiveType == 0)`
    /// are running.
    invite_waits: Vec<(u32, Entity)>,
    /// The engine's scripting generator (`UnityEngine.Random`) as this flow
    /// draws from it.
    rand: EngineRand,
    hides: Vec<PendingHide>,
    /// The last reason a reply waited, for the logs.
    waiting: Option<&'static str>,
    /// The placed gate the server document's home gate was last checked
    /// against.
    restore_checked: Option<Entity>,
    /// The running cast's cloth space, while one of this flow's cut-scenes
    /// plays.
    cast_space: Option<CastSpace>,
}

/// The cloth space of the cast avatars of the running cut-scene. The cloth
/// runtime takes a member's own `Transform` as its world, so an avatar
/// under the view's character root is solved in that root's frame. That
/// is the world solve turned by the root's rotation when the root stands
/// still, turns about the vertical alone and has unit scale: gravity points
/// down in both frames and the anchors move alike up to that turn.
struct CastSpace {
    root: Entity,
    first: Transform,
    drift_translation: f32,
    drift_degrees: f32,
    cloth: Vec<u32>,
    frames: u64,
}

fn now(world: &World) -> f64 {
    world.resource::<Time>().elapsed_secs_f64()
}

fn parse_instruments() -> Option<Request> {
    if let Some(raw) = crate::server::instrument_env(INVITE) {
        let (unit, grant_pass) = match raw.trim().split_once(',') {
            Some((unit, "pass")) => (unit, true),
            Some(_) => ("", false),
            None => (raw.trim(), false),
        };
        return match unit.trim().parse::<i32>() {
            Ok(unit) if unit > 0 => Some(Request::Reserve { unit, grant_pass }),
            _ => {
                error!("[gate] {INVITE}={raw:?} is not <unit>[,pass]; no reserve request is made");
                None
            }
        };
    }
    if let Some(raw) = crate::server::instrument_env(CHANGE) {
        let parsed = raw.split_once(':').and_then(|(gate, groups)| {
            let (gate, skin) = match gate.split_once('/') {
                Some((gate, skin)) => (
                    gate.trim().parse::<i32>().ok()?,
                    skin.trim().parse::<i32>().ok()?,
                ),
                None => (gate.trim().parse::<i32>().ok()?, 0),
            };
            let groups = groups
                .split(',')
                .filter(|group| !group.trim().is_empty())
                .map(|group| group.trim().parse::<i32>().ok())
                .collect::<Option<Vec<_>>>()?;
            Some((gate, skin, groups))
        });
        return match parsed {
            Some((gate, skin, groups)) if gate > 0 && skin >= 0 => {
                Some(Request::Change { gate, skin, groups })
            }
            _ => {
                error!("[gate] {CHANGE}={raw:?} is not <gateId>[/<skinId>]:<group>,...; no gate change request is made");
                None
            }
        };
    }
    None
}

/// The engine generator's state at this flow's start: the source seeds it
/// at launch and every scripting draw of the process advances it, so the
/// state at a draw is not reproducible; this one is seeded from the
/// platform. (The NPC appearance holds its own; the source has one.)
fn engine_state() -> [u32; 4] {
    let a = crate::npc_objective::platform_seed();
    let b = crate::npc_objective::platform_seed();
    let state = [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32];
    if state == [0; 4] {
        [1, 0, 0, 0]
    } else {
        state
    }
}

/// Startup: the gate tables, the fixture index and the instruments.
fn start(mut commands: Commands, server: Res<AssetServer>) {
    let request = parse_instruments();
    let stage = match request {
        Some(request) => {
            info!("[gate] instrument request {request:?}: made once the home site, its gate and its visitors stand and the client has joined");
            Stage::Request {
                request,
                ready_since: None,
            }
        }
        None => Stage::Idle,
    };
    commands.insert_resource(GateFlow {
        stage,
        tables: Some(server.load::<JsonAsset>(PLAYER_DATA)),
        parsed: None,
        index: Some(server.load::<JsonAsset>(FIXTURE_INDEX)),
        indexed: None,
        create_after_yield: None,
        invite_waits: Vec::new(),
        rand: EngineRand::from_state(engine_state()),
        hides: Vec::new(),
        waiting: None,
        restore_checked: None,
        cast_space: None,
    });
}

/// The catalog's gate tables; also publishes the gate model packages.
fn tables(world: &mut World) -> Option<()> {
    if world.resource::<GateFlow>().parsed.is_some() {
        return Some(());
    }
    let handle = world.resource::<GateFlow>().tables.clone()?;
    let server = world.resource::<AssetServer>().clone();
    if let LoadState::Failed(error) = server.load_state(&handle) {
        error!("[gate] the fixture catalog failed to load: {error}; no gate table");
        world.resource_mut::<GateFlow>().tables = None;
        return None;
    }
    let text = world
        .resource::<Assets<JsonAsset>>()
        .get(&handle)
        .map(|json| json.0.clone())?;
    match serde_json::from_str::<Value>(&text)
        .map_err(|error| error.to_string())
        .and_then(|document| Tables::parse(&document))
    {
        Ok(parsed) => {
            let packages = parsed.model_packages();
            info!(
                "[gate] gate tables: {} cut-scene rows, gates {:?}, {} skins, gate masters {:?}; gate model packages {}",
                parsed.cut_scenes.len(),
                parsed.gates,
                parsed.skins.len(),
                parsed.gate_masters,
                packages.len()
            );
            world.insert_resource(GateModelPackages(packages));
            let mut flow = world.resource_mut::<GateFlow>();
            flow.parsed = Some(parsed);
            flow.tables = None;
            Some(())
        }
        Err(reason) => {
            error!("[gate] the fixture catalog's gate tables: {reason}; the gate flows do not run");
            world.resource_mut::<GateFlow>().tables = None;
            None
        }
    }
}

/// `FixtureManager.GetGateFixture`: the placed fixture of a gate master.
fn gate_fixture(world: &mut World) -> Option<Entity> {
    let masters = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()?
        .gate_masters
        .clone();
    world
        .query::<(Entity, &FixtureActivityIdentity)>()
        .iter(world)
        .find(|(_, identity)| masters.contains(&identity.master_id))
        .map(|(entity, _)| entity)
}

/// The avatar store's NPC list (not the player's avatar, not a cut-scene
/// avatar), in creation order.
fn present_npcs(world: &mut World) -> Vec<(u32, Entity)> {
    crate::npc::dispose::npc_list(world)
}

fn ready(world: &mut World) -> Result<Entity, &'static str> {
    if world
        .get_resource::<crate::site::SiteActive>()
        .is_none_or(|site| site.site_type != "home_site")
    {
        return Err("the home site is not loaded");
    }
    let gate = gate_fixture(world).ok_or("no gate is placed")?;
    if !world
        .get_resource::<crate::npc_gate::GateAppearance>()
        .is_some_and(|appearance| appearance.appeared())
    {
        return Err("the visitors are not placed yet");
    }
    let npcs = present_npcs(world);
    if npcs
        .iter()
        .any(|(_, npc)| world.get::<crate::character::MotionDriver>(*npc).is_none())
    {
        return Err("the visitors are being assembled");
    }
    if world.contains_resource::<crate::cutscene::HomePerform>()
        || world.contains_resource::<crate::game_state::UiHold>()
    {
        return Err("another performance holds the field");
    }
    Ok(gate)
}

fn se(world: &mut World, cue: &str, source: &'static str) {
    match world.get_resource_mut::<crate::audio::SeRequests>() {
        Some(mut requests) => requests.0.push(crate::audio::SeRequest {
            owner: None,
            cue: cue.to_owned(),
            class: crate::audio::SeClass::Ingame,
            source,
        }),
        None => error!("[gate] no SE request queue: {cue} not played"),
    }
}

fn joystick(world: &World) -> Option<bool> {
    world
        .get_resource::<crate::joystick::JoystickState>()
        .map(|joystick| joystick.enabled)
}

/// Publishes [`HomeGateModel`] from the server document once the gate
/// tables and the fixture index are read, and again when the document's
/// home gate changes.
fn publish_home_gate(world: &mut World) {
    let (masters, bundle) = {
        let flow = world.resource::<GateFlow>();
        match flow.parsed.as_ref() {
            Some(tables) => {
                // The server model is present but its document is still
                // loading.
                if world.contains_resource::<crate::server::ServerGateReplies>()
                    && !crate::server::installed()
                {
                    return;
                }
                let bundle = crate::server::home_gate().map(|(gate, skin)| {
                    (
                        (gate, skin),
                        tables.gate_bundle(i64::from(gate), i64::from(skin)),
                    )
                });
                (tables.gate_masters.clone(), bundle)
            }
            // The gate tables could not be read.
            None if flow.tables.is_none() => (HashSet::new(), None),
            None => return,
        }
    };
    let (package, note) = match bundle {
        None => (
            None,
            "no server document: the layout's gate row stands".to_owned(),
        ),
        Some((gate, Err(reason))) => (
            None,
            format!("userMysekaiGates {gate:?}: {reason}; the layout's gate row stands"),
        ),
        Some((gate, Ok(bundle))) => {
            let package = format!("mysekai__fixture__{bundle}");
            match indexed(world, &package) {
                Ok(true) => (Some(package), format!("userMysekaiGates {gate:?}")),
                Ok(false) => (
                    None,
                    format!("userMysekaiGates {gate:?}: the gate model mysekai/fixture/{bundle} is not in the fixture index (not extracted); the layout's gate row stands"),
                ),
                Err(_) => return, // the index is loading
            }
        }
    };
    let model = HomeGateModel { package, masters };
    if world.get_resource::<HomeGateModel>() != Some(&model) {
        info!(
            "[gate] home gate model {:?} for the gate masters {:?} ({note})",
            model.package, model.masters
        );
        world.insert_resource(model);
    }
}

/// The fixture index lists `package` (read once).
fn indexed(world: &mut World, package: &str) -> Result<bool, String> {
    if world.resource::<GateFlow>().indexed.is_none() {
        let handle = world
            .resource::<GateFlow>()
            .index
            .clone()
            .ok_or("the fixture index was not requested")?;
        let text = world
            .resource::<Assets<JsonAsset>>()
            .get(&handle)
            .map(|json| json.0.clone())
            .ok_or("the fixture index is not loaded")?;
        let document: Value = serde_json::from_str(&text)
            .map_err(|error| format!("the fixture index is not JSON: {error}"))?;
        let packages = document["packages"]
            .as_object()
            .ok_or("the fixture index has no packages object")?
            .keys()
            .cloned()
            .collect();
        let mut flow = world.resource_mut::<GateFlow>();
        flow.indexed = Some(packages);
        flow.index = None;
    }
    Ok(world
        .resource::<GateFlow>()
        .indexed
        .as_ref()
        .is_some_and(|packages| packages.contains(package)))
}

/// Update (exclusive), after the cut-scene presenter.
fn advance(world: &mut World) {
    if !world.contains_resource::<GateFlow>() {
        return;
    }
    create_after_yield(world);
    invite_waits(world);
    hide_effects(world);
    if matches!(
        world.resource::<GateFlow>().stage,
        Stage::InviteCutScene | Stage::GoHome { .. }
    ) {
        watch_cast_space(world);
    }
    publish_home_gate(world);
    if tables(world).is_none() {
        return;
    }
    let stage = std::mem::replace(&mut world.resource_mut::<GateFlow>().stage, Stage::Idle);
    let next = match stage {
        Stage::Idle => take_reply(world),
        Stage::Request {
            request,
            ready_since,
        } => request_step(world, request, ready_since),
        Stage::InviteWhiteIn { until } => {
            if now(world) >= until {
                info!(
                    "[gate] ReserveProcessAsync: Delay(1.0 s) done; the invitation ends (joystick enabled {:?})",
                    joystick(world)
                );
                Stage::Idle
            } else {
                Stage::InviteWhiteIn { until }
            }
        }
        Stage::ChangeGate { change, until } => {
            if now(world) >= until {
                info!("[gate] ChangeGateAction: Delay(1.0 s) done; StartCharacterAppearanceAsync(...).Forget(); FadeInLayer (the UI layer: not here)");
                start_character_appearance(world, change)
            } else {
                Stage::ChangeGate { change, until }
            }
        }
        Stage::Appearance(mut appearance) => {
            if appearance_step(world, &mut appearance) {
                Stage::Idle
            } else {
                Stage::Appearance(appearance)
            }
        }
        other => other,
    };
    world.resource_mut::<GateFlow>().stage = next;
}

/// Logs a wait once per reason.
fn note_wait(world: &mut World, what: &str, reason: &'static str) {
    let mut flow = world.resource_mut::<GateFlow>();
    if flow.waiting != Some(reason) {
        flow.waiting = Some(reason);
        info!("[gate] {what} waits: {reason}");
    }
}

/// A gate reply of the server model, taken once the field is ready.
fn take_reply(world: &mut World) -> Stage {
    let pending = world
        .get_resource::<crate::server::ServerGateReplies>()
        .is_some_and(|replies| !replies.0.is_empty());
    if !pending {
        restore_saved_gate(world);
        return Stage::Idle;
    }
    let readiness = ready(world).and_then(|gate| {
        world
            .contains_resource::<crate::fixture_activity_data::FixtureActivityTables>()
            .then_some(gate)
            .ok_or("the unit-group table is loading")
    });
    let gate = match readiness {
        Ok(gate) => gate,
        Err(reason) => {
            note_wait(world, "a gate reply", reason);
            return Stage::Idle;
        }
    };
    world.resource_mut::<GateFlow>().waiting = None;
    let Some(reply) = world
        .resource_mut::<crate::server::ServerGateReplies>()
        .0
        .pop_front()
    else {
        return Stage::Idle;
    };
    match reply {
        GateReply::Reserve { row, talks } => reserve_process(world, gate, row, talks),
        GateReply::Change {
            gate_id,
            skin_id,
            rows,
            talks,
        } => execute_change_gate_process(world, gate, gate_id, skin_id, rows, talks),
    }
}

/// An instrument's request: made once the field is ready, the client has
/// joined and the settle has passed.
fn request_step(world: &mut World, request: Request, ready_since: Option<f64>) -> Stage {
    let joined = world.contains_resource::<crate::server::ClientUserData>();
    let readiness = ready(world).and_then(|_| {
        joined
            .then_some(())
            .ok_or("the client has not joined the server yet")
    });
    if let Err(reason) = readiness {
        if ready_since.is_some() {
            info!("[gate] the request waits again: {reason}");
        }
        note_wait(world, "the instrument's request", reason);
        return Stage::Request {
            request,
            ready_since: None,
        };
    }
    let since = ready_since.unwrap_or_else(|| now(world));
    if now(world) - since < SETTLE {
        return Stage::Request {
            request,
            ready_since: Some(since),
        };
    }
    world.resource_mut::<GateFlow>().waiting = None;
    if let Request::Reserve {
        unit,
        grant_pass: true,
    } = request
    {
        grant_colorful_pass();
        return Stage::Request {
            request: Request::Reserve {
                unit,
                grant_pass: false,
            },
            ready_since: Some(since),
        };
    }
    if let Request::Reserve { unit, .. } = request {
        let realtime = world.resource::<Time<Real>>().elapsed_secs();
        let pass = world
            .resource::<crate::server::ClientUserData>()
            .has_mysekai_colorful_pass(realtime);
        info!("[gate] ScreenLayerMysekaiGateInvitationPresenter.ReserveProcessAsync(unit {unit}): HasMysekaiColorfulPass = {pass} (the client's copy of userMysekaiColorfulPass)");
        if !pass {
            info!("[gate] ReserveProcessAsync: MysekaiGateUtility.ShowNotInvitationExpiredPassDialog (UI); no request");
            return Stage::Idle;
        }
    }
    let command = request.command();
    let text = serde_json::Value::Object(command.clone()).to_string();
    match crate::server::with_model(|model| model.edit(&command)) {
        Some(Ok(receipt)) => info!("[gate] instrument request {request:?} -> server.edit {text}: {receipt}; the reply comes through the server's gate replies"),
        Some(Err(reason)) => error!("[gate] instrument request {request:?}: the server refused it: {reason}"),
        None => error!("[gate] instrument request {request:?}: no server model is installed; nothing is requested"),
    }
    Stage::Idle
}

/// The server panel's edits that give the user a colorful pass for a day of
/// the server clock (`userMysekaiColorfulPass`), then its `sync` action,
/// which delivers the pass to the client's copy as a response.
fn grant_colorful_pass() {
    let result = crate::server::with_model(|model| {
        let expired_at = model.now_ms().saturating_add(86_400_000);
        let mut edit = serde_json::Map::new();
        edit.insert("type".into(), "server.edit".into());
        edit.insert("path".into(), "userMysekaiColorfulPass".into());
        edit.insert(
            "value".into(),
            serde_json::json!({"mysekaiColorfulPassId": 1, "expiredAt": expired_at}),
        );
        let mut sync = serde_json::Map::new();
        sync.insert("type".into(), "server.edit".into());
        sync.insert("action".into(), "sync".into());
        (model.edit(&edit), model.edit(&sync))
    });
    match result {
        Some((Ok(pass), Ok(sync))) => info!("[gate] instrument: the server panel gives the user a colorful pass: {pass}; sync: {sync}"),
        Some((pass, sync)) => error!("[gate] instrument: the colorful pass edit {pass:?}, sync {sync:?}"),
        None => error!("[gate] instrument: no server model is installed; no colorful pass"),
    }
}

/// `HideEffect`'s wait and `SetActive(false)` for each stopped stay effect.
fn hide_effects(world: &mut World) {
    if world.resource::<GateFlow>().hides.is_empty() {
        return;
    }
    let (frame, t) = (frame(world), now(world));
    let hides = std::mem::take(&mut world.resource_mut::<GateFlow>().hides);
    let mut kept = Vec::new();
    for hide in hides {
        if world.get_entity(hide.node).is_err() {
            info!("[gate] HideEffect({STAY_EFFECT}): its node is gone (the gate was replaced); the wait ends");
            continue;
        }
        // `UniTask.WaitWhile` first tests the predicate on the Update loop
        // after the frame it was made on.
        if frame <= hide.stopped_frame || system_alive(world, hide.node) {
            kept.push(hide);
            continue;
        }
        moly_assets::scene_state::SetSourceActive {
            entity: hide.node,
            active: false,
        }
        .apply(world);
        info!(
            "[gate] t={t:.3} HideEffect({STAY_EFFECT}): IsAlive(withChildren: false) false {:.3} s after Stop(): SetActive(false)",
            t - hide.stopped_at
        );
    }
    world.resource_mut::<GateFlow>().hides.extend(kept);
}

/// `ParticleSystem.IsAlive(withChildren: false)` of the system on `node`:
/// playing, or holding a particle. A system the particle host simulates has
/// a draw anchored at the node. One it prepared no draw for (a system with
/// no Emission module, like the stay effect's root: it never holds a
/// particle) is not alive once stopped, as `Stop()` ends a play that holds
/// no particle.
fn system_alive(world: &mut World, node: Entity) -> bool {
    let draws: Vec<Entity> = world
        .query::<(Entity, &crate::uber_particle::FixtureParticleLive)>()
        .iter(world)
        .filter(|(_, live)| live.0.anchor == Some(node))
        .map(|(draw, _)| draw)
        .collect();
    draws.into_iter().any(|draw| {
        crate::weather_fx::fixture::system_playing(world, draw)
            || world
                .get::<crate::uber_particle::FixtureParticleLive>(draw)
                .is_some_and(|live| !live.0.pool.is_empty())
    })
}

fn frame(world: &World) -> u64 {
    world
        .get_resource::<bevy::diagnostic::FrameCount>()
        .map_or(0, |count| u64::from(count.0))
}

/// `OnEndCutScene` after its `Yield`: `CreateNPC` for the invited unit, then
/// `WhiteOut(0, 0)`.
fn create_after_yield(world: &mut World) {
    let due = world
        .resource::<GateFlow>()
        .create_after_yield
        .as_ref()
        .is_some_and(|(_, queued)| frame(world) > *queued);
    if !due {
        return;
    }
    let Some((units, _)) = world.resource_mut::<GateFlow>().create_after_yield.take() else {
        return;
    };
    for unit in units {
        // The gate's ActionPoints.First().EndLoc position, the literal
        // rotation of pi about the up axis.
        let created = gate_fixture(world)
            .ok_or_else(|| "no gate is placed (GetGateFixture is null)".to_owned())
            .and_then(|gate| crate::npc::gate_entries::gate_first_end_loc(world, gate))
            .and_then(|position| {
                crate::npc::gate_entries::create_npc(
                    world,
                    unit,
                    position,
                    crate::npc::gate_entries::invite_rotation(),
                )
            });
        match created {
            Ok(npc) => {
                info!("[gate] OnEndCutScene: CreateNPC(unit {unit}, 1, the gate's first EndLoc, Euler(0, pi, 0)): {npc:?}; WaitUntil(IsExistNPCAll); SendNpcSpawnFlagPayload (multiplayer: nothing here); WaitWhile(CurrentObjectiveType == 0)");
                world
                    .resource_mut::<GateFlow>()
                    .invite_waits
                    .push((unit, npc));
            }
            Err(reason) => error!("[gate] OnEndCutScene: CreateNPC(unit {unit}): {reason}"),
        }
    }
    // The source whites out after the two waits and the NPC calls below,
    // inside the end callback the cut-scene presenter awaits; this host's end
    // callback returns at once, so the white-out stays here, before the
    // presenter's return and its WhiteIn (named order difference: the NPC is
    // shown a few frames later, behind the white screen).
    crate::screen_fade::white_out(world, 0.0, 0.0, "OnEndCutScene: ScreenManager.WhiteOut");
    info!("[gate] OnEndCutScene: ScreenManager.WhiteOut(0, 0)");
}

/// `OnEndCutScene` after `CreateNPC`: once the NPC exists and has decided
/// its first objective, `TryCancelCurrentObjective`, `ForceUpdateObjective`
/// and `Show`.
fn invite_waits(world: &mut World) {
    let waits = std::mem::take(&mut world.resource_mut::<GateFlow>().invite_waits);
    let mut left = Vec::new();
    for (unit, npc) in waits {
        if world.get_entity(npc).is_err() {
            error!(
                "[gate] OnEndCutScene: unit {unit}'s created NPC is gone before its waits ended"
            );
            continue;
        }
        let (_, decided) = crate::npc::gate_entries::invite_waits(world, npc);
        if !decided {
            left.push((unit, npc));
            continue;
        }
        let (cancelled, updated) = crate::npc::gate_entries::invite_show(world, npc);
        info!("[gate] OnEndCutScene: unit {unit}: TryCancelCurrentObjective {cancelled}, ForceUpdateObjective {updated}, Show");
    }
    world.resource_mut::<GateFlow>().invite_waits = left;
}

/// `ReserveProcessAsync` after the reserve reply
/// (`UserMysekaiGateCharacterVisitResponse`). The selected unit is the reply
/// row's unit group (the request sends the selected unit id as the group).
fn reserve_process(
    world: &mut World,
    gate: Entity,
    row: GateCharacter,
    talks: ReplyTalkList,
) -> Stage {
    info!("[gate] ReserveProcessAsync: the reserve reply: userMysekaiGateCharacters row {row:?}; mysekaiCharacterTalkWithReadHistories {}", talks_text(&talks));
    let Ok(unit) = u32::try_from(row.unit_group_id) else {
        error!(
            "[gate] ReserveProcessAsync: unit group {} is not a unit id",
            row.unit_group_id
        );
        return Stage::Idle;
    };
    // The presenter's pass check comes before its request; a reply the
    // server panel made without the gate screen is checked here, as the
    // screen would have.
    let realtime = world.resource::<Time<Real>>().elapsed_secs();
    let pass = world
        .get_resource::<crate::server::ClientUserData>()
        .is_some_and(|client| client.has_mysekai_colorful_pass(realtime));
    if !pass {
        info!("[gate] ReserveProcessAsync: HasMysekaiColorfulPass false: ShowNotInvitationExpiredPassDialog (UI); the reply is not presented (the server holds the reservation)");
        return Stage::Idle;
    }
    let present = present_npcs(world);
    if present.iter().any(|(id, _)| *id == unit) {
        info!("[gate] ReserveProcessAsync: IsExistNPC({unit}) true: SetSelectedGameCharacterUnitId, Refresh and FadeInAsync (UI); no AddGateCharacters, no UpdateTalkList, no invite cut-scene");
        return Stage::Idle;
    }
    // `MysekaiBootData.AddGateCharacters`: the reply's row joins the gate
    // characters the server holds.
    let visitors = crate::server::with_model(|model| model.document().gate_characters.clone())
        .unwrap_or_else(|| vec![row]);
    crate::server::client_update_talk_list(
        world,
        TalkListUpdate {
            caller: "ReserveProcessAsync: MysekaiBootData.AddGateCharacters(the reply's rows); MysekaiTalkDataStore.UpdateTalkList([], the reply's talk list, [])",
            talks,
            visitors: Some(visitors),
        },
    );
    info!("[gate] ReserveProcessAsync: IsExistNPC({unit}) false: MysekaiBootData.AddGateCharacters and MysekaiTalkDataStore.UpdateTalkList handed to the client's talk store; the view's fade out and ShowSendInvitationDialog are UI");
    let row = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()
        .and_then(|tables| tables.cut_scene(INVITE_CONDITION, i64::from(unit)));
    let Some((id, timeline)) = row else {
        error!("[gate] PlayInviteCutSceneAsync: GetMasterMysekaiCutScene({unit}, {INVITE_CONDITION}) = null: logged, nothing plays");
        return white_in(world);
    };
    info!(
        "[gate] PlayInviteCutSceneAsync(unit {unit}): master cut-scene {id} {timeline}; NPCs present {:?}; the gate fixture {gate:?}",
        present.iter().map(|(unit, _)| *unit).collect::<Vec<_>>()
    );
    let play = CastPlay {
        caller: CastCaller::Invite,
        units: vec![unit],
        cut_scene_id: id,
        timeline,
        start: gate,
        use_already_exist_character: false,
        hide_player: true,
        show_ui: true,
    };
    match crate::cutscene::play_async(world, play) {
        Ok(()) => Stage::InviteCutScene,
        Err(reason) => {
            error!("[gate] PlayInviteCutSceneAsync: {reason}");
            white_in(world)
        }
    }
}

fn talks_text(talks: &ReplyTalkList) -> String {
    match talks {
        ReplyTalkList::Stated(rows) => format!(
            "stated, {} rows (talk ids {:?})",
            rows.len(),
            rows.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        ),
        ReplyTalkList::Policy => "the server's talk-list policy".to_owned(),
    }
}

fn white_in(world: &mut World) -> Stage {
    crate::screen_fade::white_in(
        world,
        0.0,
        WHITE_IN,
        "ReserveProcessAsync: ScreenManager.WhiteIn",
    );
    info!("[gate] ReserveProcessAsync: ScreenManager.WhiteIn(0, {WHITE_IN}); Delay(1.0 s)");
    Stage::InviteWhiteIn {
        until: now(world) + AFTER_WHITE_IN,
    }
}

/// `ExecuteChangeGateProcessAsync` after the change reply.
fn execute_change_gate_process(
    world: &mut World,
    gate: Entity,
    gate_id: i32,
    skin_id: i32,
    rows: Vec<GateCharacter>,
    talks: ReplyTalkList,
) -> Stage {
    let (id, skin) = (i64::from(gate_id), i64::from(skin_id));
    info!("[gate] MysekaiGateUtility.ExecuteChangeGateProcessAsync: the change reply: userMysekaiGates mysekaiGateId {id} mysekaiGateSkinId {skin} isSettingAtHomeSite true; userMysekaiGateCharacters {rows:?}; mysekaiCharacterTalkWithReadHistories {}; ChangeUIScreen(602) (UI)", talks_text(&talks));
    let mut units: Vec<u32> = Vec::new();
    {
        let Some(tables) =
            world.get_resource::<crate::fixture_activity_data::FixtureActivityTables>()
        else {
            error!("[gate] ExecuteChangeGateProcessAsync: the unit-group table is not loaded; the change process ends");
            return Stage::Idle;
        };
        for row in &rows {
            match tables.unit_ids_of_group(row.unit_group_id) {
                Ok(expanded) => {
                    for unit in expanded {
                        if !units.contains(&unit) {
                            units.push(unit);
                        }
                    }
                }
                Err(reason) => {
                    error!("[gate] ExecuteChangeGateProcessAsync: {reason} (the source's null dereference): the change process ends");
                    return Stage::Idle;
                }
            }
        }
    }
    let change = Change { rows, units, talks };
    let tutorial = match world.get_resource::<crate::server::ClientUserData>() {
        Some(client) => !client.gamedata.is_mysekai_tutorial_end,
        None => world
            .get_resource::<crate::server_panel::ServerPanel>()
            .is_some_and(|panel| panel.is_tutorial()),
    };
    if tutorial {
        info!("[gate] ExecuteChangeGateProcessAsync: in the tutorial: no go-home cut-scene and no appearance");
        return change_gate(
            world,
            id,
            skin,
            Change {
                units: Vec::new(),
                ..change
            },
        );
    }
    // TryShowGoHomeCutSceneAsync.
    let present = present_npcs(world);
    if present.is_empty() {
        info!("[gate] TryShowGoHomeCutSceneAsync: no NPC present: false");
        return change_gate(world, id, skin, change);
    }
    // `_npcList.RandomPick()`: the List overload, `Random.Range(0, _size)`.
    let (draw, before, after) = {
        let mut flow = world.resource_mut::<GateFlow>();
        let before = flow.rand.state;
        let draw = flow.rand.range_int(0, present.len() as i32);
        (draw, before, flow.rand.state)
    };
    let Some(&(unit, _)) = usize::try_from(draw)
        .ok()
        .and_then(|draw| present.get(draw))
    else {
        error!(
            "[gate] TryShowGoHomeCutSceneAsync: Random.Range(0, {}) = {draw} is outside the list",
            present.len()
        );
        return change_gate(world, id, skin, change);
    };
    info!(
        "[gate] TryShowGoHomeCutSceneAsync: NPCs present {:?}; RandomPick: UnityEngine.Random.Range(0, {}) = {draw} (engine state {before:08x?} -> {after:08x?}): unit {unit}",
        present.iter().map(|(unit, _)| *unit).collect::<Vec<_>>(),
        present.len()
    );
    let row = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()
        .and_then(|tables| tables.cut_scene(LEAVE_CONDITION, i64::from(unit)));
    let Some((cut_scene, timeline)) = row else {
        info!("[gate] TryShowGoHomeCutSceneAsync: no leave cut-scene for unit {unit}: false");
        return change_gate(world, id, skin, change);
    };
    info!("[gate] TryShowGoHomeCutSceneAsync: leave cut-scene {cut_scene} {timeline}; DestroySpawnedNetworkNPCObject (multiplayer: nothing here)");
    dispose_npc_all(world, "TryShowGoHomeCutSceneAsync");
    let play = CastPlay {
        caller: CastCaller::GoHome,
        units: vec![unit],
        cut_scene_id: cut_scene,
        timeline,
        start: gate,
        use_already_exist_character: false,
        hide_player: true,
        show_ui: true,
    };
    match crate::cutscene::play_async(world, play) {
        Ok(()) => Stage::GoHome {
            gate: id,
            skin,
            change,
        },
        Err(reason) => {
            error!("[gate] TryShowGoHomeCutSceneAsync: {reason}");
            change_gate(world, id, skin, change)
        }
    }
}

/// The home gate shows the model of the gate the server document sets at the
/// home site (`MysekaiGateModel.AssetBundleName`), checked once per placed
/// gate: a home load places a new one. The document persists the gate
/// change in the browser game.
fn restore_saved_gate(world: &mut World) {
    let Some((gate_id, skin_id)) = crate::server::home_gate() else {
        return;
    };
    if world
        .get_resource::<crate::site::SiteActive>()
        .is_none_or(|site| site.site_type != "home_site")
    {
        return;
    }
    let Some(gate) = gate_fixture(world) else {
        return;
    };
    if world.resource::<GateFlow>().restore_checked == Some(gate) {
        return;
    }
    world.resource_mut::<GateFlow>().restore_checked = Some(gate);
    let Some(bundle) = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()
        .map(|tables| tables.gate_bundle(i64::from(gate_id), i64::from(skin_id)))
    else {
        return;
    };
    let package = match bundle {
        Ok(bundle) => format!("mysekai__fixture__{bundle}"),
        Err(reason) => {
            error!("[gate] the home gate {gate_id} / skin {skin_id}: MysekaiGateModel.AssetBundleName: {reason}; the gate keeps its model");
            return;
        }
    };
    let current = world
        .get::<FixtureActivityIdentity>(gate)
        .map(|identity| identity.model_package.clone());
    if current.as_deref() == Some(package.as_str()) {
        return;
    }
    match indexed(world, &package) {
        Ok(true) => {}
        Ok(false) => {
            error!("[gate] the home gate {gate_id} / skin {skin_id}: the gate model {package} is not in the fixture index (not extracted); the gate keeps its model");
            return;
        }
        Err(reason) => {
            // The index is loading: check this gate again.
            debug!("[gate] the home gate check waits: {reason}");
            world.resource_mut::<GateFlow>().restore_checked = None;
            return;
        }
    }
    match swap_gate_model(world, &package) {
        Ok(old) => info!("[gate] the home gate {gate_id} / skin {skin_id} of the server document: the gate shows {package} (was {old})"),
        Err(reason) => error!(
            "[gate] the home gate {gate_id} / skin {skin_id}: {reason}; the gate keeps its model"
        ),
    }
}

/// `AvatarDataStore.DisposeNPCAll`: every NPC's avatar is disposed.
fn dispose_npc_all(world: &mut World, caller: &str) {
    let disposed = crate::npc::dispose::dispose_npc_all(world, caller);
    info!("[gate] {caller}: DisposeNPCAll: units {disposed:?} disposed");
}

/// `ChangeGateAction`: `BackUIScreen`, `SiteLayoutUtility.ChangeGate`, then
/// a 1.0 s delay.
fn change_gate(world: &mut World, id: i64, skin: i64, change: Change) -> Stage {
    info!("[gate] FadeOutLayer (the UI layer: not here); ChangeGateAction: BackUIScreen (UI)");
    let bundle = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()
        .map(|tables| tables.gate_bundle(id, skin));
    match bundle {
        Some(Ok(bundle)) => {
            let package = format!("mysekai__fixture__{bundle}");
            match indexed(world, &package) {
                Ok(true) => match swap_gate_model(world, &package) {
                    Ok(old) => {
                        se(world, SE_CHANGE, "gate change");
                        info!("[gate] SiteLayoutUtility.ChangeGate -> HomeSiteController.UpdateGateModelAsync: RemovePutData; SiteView.UpdateFixture(the gate, model {old} -> {package}): the gate's root alone is replaced; AddTileData (the footprint is unchanged); PlaySEOneShot({SE_CHANGE}); ObjectCollisionManager.ForceUpdate and the room update: not modelled here");
                    }
                    Err(reason) => error!(
                        "[gate] SiteLayoutUtility.ChangeGate: {reason}; the gate keeps its model"
                    ),
                },
                Ok(false) => error!("[gate] SiteLayoutUtility.ChangeGate: the gate model mysekai/fixture/{bundle} is not in the fixture index (not extracted); the gate keeps its model"),
                Err(reason) => error!(
                    "[gate] SiteLayoutUtility.ChangeGate: {reason}; the gate keeps its model"
                ),
            }
        }
        Some(Err(reason)) => {
            error!("[gate] MysekaiGateModel.AssetBundleName: {reason}; the gate keeps its model")
        }
        None => {}
    }
    Stage::ChangeGate {
        change,
        until: now(world) + AFTER_CHANGE,
    }
}

/// `SiteView.UpdateFixture` on the gate: its placement row shows the new
/// model and its root alone is replaced. Returns the old package.
fn swap_gate_model(world: &mut World, package: &str) -> Result<String, String> {
    let masters = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()
        .map(|tables| tables.gate_masters.clone())
        .unwrap_or_default();
    let uid = world
        .get_resource::<crate::fixture::FixturePlacements>()
        .ok_or("the layout is not installed")?
        .editor_rows()
        .into_iter()
        .find(|row| masters.contains(&row.fixture_id))
        .map(|row| row.uid)
        .ok_or("the layout has no gate row")?;
    crate::fixture::replace_instance_model(world, &uid, package)
}

/// `StartCharacterAppearanceAsync`: `MysekaiTalkDataStore.SetTalkList`, then
/// `SetupNpcVisitingAsync(visitingFromGate: true)` -> `ShowCharacterFromGate`.
fn start_character_appearance(world: &mut World, change: Change) -> Stage {
    let Change { rows, units, talks } = change;
    crate::server::client_update_talk_list(
        world,
        TalkListUpdate {
            caller: "StartCharacterAppearanceAsync: MysekaiTalkDataStore.SetTalkList(the change reply's talk list); the reply's gate characters visit",
            talks,
            visitors: Some(rows),
        },
    );
    info!("[gate] StartCharacterAppearanceAsync: MysekaiTalkDataStore.SetTalkList(the change reply's talk list) handed to the client's talk store; SetupNpcVisitingAsync(visitingFromGate true): ShowCharacterFromGate: SetVisitingCharacterFromGateStatusStart (event 65, no subscriber here); SetupNPC(units {units:?}), then PlayGateCharacterAppearTimeline (each visitor's forced gate timeline is named where it is called: not routed)");
    Stage::Appearance(Box::new(Appearance {
        units,
        next_at: 0.0,
        visitors: Vec::new(),
        stay: None,
        stay_node: None,
        hidden: false,
        close: None,
        spawned: false,
        generation: 0,
        warmed: Vec::new(),
        waiting: None,
    }))
}

/// The gate's stay effect node (`FixtureController.ShowEffect` finds every
/// ParticleSystem of the view named so, inactive ones included).
fn stay_effect(world: &World, gate: Entity) -> Option<Entity> {
    let mut stack = vec![gate];
    while let Some(entity) = stack.pop() {
        if world
            .get::<Name>(entity)
            .is_some_and(|name| name.as_str() == STAY_EFFECT)
        {
            return Some(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    None
}

fn appearance_step(world: &mut World, appearance: &mut Appearance) -> bool {
    let t = now(world);
    let Some(gate) = gate_fixture(world) else {
        appearance.wait(t, "no placed gate carries its identity yet".to_owned());
        return false;
    };
    // The gate shows its model (its scene is out and its materials are
    // swapped) before its effects and parts are looked up.
    if world
        .get::<crate::fixture::FixtureVisualReady>(gate)
        .is_none()
    {
        appearance.wait(t, format!("the gate {gate:?} does not show its model yet"));
        return false;
    }

    if !appearance.spawned {
        if appearance.units.is_empty() {
            info!("[gate] PlayGateCharacterAppearTimeline: no visitor picked by the server: logged, returns");
            return true;
        }
        match crate::npc::spawn_temporary_units(world, &appearance.units) {
            Ok(created) => {
                appearance.spawned = true;
                info!("[gate] SetupNPC: CreateNPC(unit, 1, visitCount) for {} visitors ({created:?}): the NPC runtime's member creation, where new members are seated (the entries have no CreateNPC without a pose)", created.len());
            }
            Err(reason) => {
                debug!("[gate] SetupNPC waits: {reason}");
                return false;
            }
        }
    }
    // The gate parts' assets load while the visitors are assembled (a
    // product loading step: the source's bundles are at hand).
    for name in [FIRST_VISIT, NEXT_VISIT, CLOSE_GATE] {
        if !appearance.warmed.contains(&name) && warm(world, gate, name) {
            appearance.warmed.push(name);
        }
    }
    let npcs = present_npcs(world);
    if appearance.visitors.is_empty() {
        // WaitUntil(IsExistNPCAll).
        let all = appearance.units.iter().all(|unit| {
            npcs.iter().any(|(id, npc)| {
                id == unit && world.get::<crate::character::MotionDriver>(*npc).is_some()
            })
        });
        let Some(node) = stay_effect(world, gate) else {
            if all && appearance.waiting.as_deref() != Some(STAY_EFFECT) {
                error!(
                    "[gate] the gate {gate:?} has no {STAY_EFFECT} node; ShowEffect finds nothing"
                );
                appearance.waiting = Some(STAY_EFFECT.to_owned());
            }
            return false;
        };
        if !all {
            appearance.wait(
                t,
                format!(
                    "WaitUntil(IsExistNPCAll): not every one of {:?} stands",
                    appearance.units
                ),
            );
            return false;
        }
        let package = world
            .get::<FixtureActivityIdentity>(gate)
            .map(|identity| identity.model_package.clone())
            .unwrap_or_default();
        // The gate fixture owns its model's particle document, as its
        // fixture timelines' Control clips read it.
        match crate::fixture_timeline_particles::prepare_played_object(world, gate, node, &package)
        {
            Ok(binding) => {
                moly_assets::scene_state::SetSourceActive {
                    entity: node,
                    active: true,
                }
                .apply(world);
                let played = crate::fixture_timeline_particles::play_object(world, &binding);
                info!("[gate] t={t:.3} PlayGateCharacterAppearTimeline: WaitUntil(IsExistNPCAll) done; gate.ShowEffect({STAY_EFFECT}): SetActive(true), Play(): {played:?} systems ({package})");
                appearance.stay = Some(binding);
                appearance.stay_node = Some(node);
            }
            Err(error) if error.retryable => {
                appearance.wait(
                    t,
                    format!("gate.ShowEffect({STAY_EFFECT}): {}", error.message),
                );
                return false;
            }
            Err(error) => {
                error!("[gate] gate.ShowEffect({STAY_EFFECT}): refused by the particle host: {}; the appearance goes on", error.message);
                appearance.stay_node = Some(node);
            }
        }
        appearance.next_at = t;
        for (index, unit) in appearance.units.iter().enumerate() {
            appearance.visitors.push(Visitor {
                unit: *unit,
                timeline: if index == 0 { FIRST_VISIT } else { NEXT_VISIT },
                started: f64::INFINITY,
                chimed: false,
                request: None,
                left_out: Vec::new(),
                fallback: 0,
                started_request: None,
                token: None,
                reported: None,
                done: false,
            });
        }
    }
    // The loop: each visitor starts 1000 ms after the previous one.
    let mut index = 0;
    while index < appearance.visitors.len() {
        let visitor = &mut appearance.visitors[index];
        if !visitor.started.is_finite() {
            if t + 1e-9 < appearance.next_at {
                break;
            }
            visitor.started = appearance.next_at;
            appearance.next_at += VISITOR_INTERVAL;
            if index > 0 {
                warn!("[gate] TryCancelIfFixtureActionGate() before unit {}: FirstOrDefault(the NPC fixture timeline list, its controller's FixtureUID == GetGateFixture().UId), then TryCancelCurrentObjective on its presenter and, on true, ForceUpdateObjective: not routed: the NPC decision ladder has no objective 12 route (next NPC lane), and no NPC-side read gives the NPC fixture timelines on the gate", visitor.unit);
            }
        }
        index += 1;
    }
    // A later gate part is handed to the runner only once the earlier one
    // has started (a product loading step: its preparation can finish
    // after the next one's, while the source's parts are ready at their
    // start, so the source's order holds).
    let mut earlier_started = true;
    for index in 0..appearance.visitors.len() {
        let mut visitor =
            std::mem::replace(&mut appearance.visitors[index], Visitor::placeholder());
        visitor_step(
            world,
            gate,
            &mut visitor,
            t,
            &mut appearance.generation,
            earlier_started,
        );
        earlier_started = visitor.token.is_some() || visitor.done;
        appearance.visitors[index] = visitor;
    }
    let all_started = appearance
        .visitors
        .iter()
        .all(|visitor| visitor.started.is_finite());
    if !all_started || t + 1e-9 < appearance.next_at {
        return false;
    }
    if !appearance.hidden {
        appearance.hidden = true;
        // HideEffect: the view's active systems of that name; the stay
        // effect has one, its root.
        let stopped = appearance
            .stay
            .as_ref()
            .map(|binding| crate::fixture_timeline_particles::stop_object(world, binding));
        if let Some(node) = appearance.stay_node {
            let frame = frame(world);
            world.resource_mut::<GateFlow>().hides.push(PendingHide {
                node,
                stopped_frame: frame,
                stopped_at: t,
            });
        }
        info!("[gate] t={t:.3} gate.HideEffect({STAY_EFFECT}).Forget(): Stop(withChildren, StopEmitting) on {stopped:?} systems; SetActive(false) once IsAlive(false) is false, checked on the Update loop from the next frame; PlayCloseGateAsync(mysekai/fixture_timeline/mdl_non0006_gate_lon1, {CLOSE_GATE}) at once");
        appearance.close = Some(Visitor {
            unit: 0,
            timeline: CLOSE_GATE,
            started: t,
            chimed: true,
            request: None,
            left_out: Vec::new(),
            fallback: 0,
            started_request: None,
            token: None,
            reported: None,
            done: false,
        });
    }
    if let Some(mut close) = appearance.close.take() {
        let visitors_started = appearance
            .visitors
            .iter()
            .all(|visitor| visitor.token.is_some() || visitor.done);
        visitor_step(
            world,
            gate,
            &mut close,
            t,
            &mut appearance.generation,
            visitors_started,
        );
        let done = close.done;
        appearance.close = Some(close);
        if done && appearance.visitors.iter().all(|visitor| visitor.done) {
            info!("[gate] t={t:.3} ShowCharacterFromGate: SetVisitingCharacterFromGateStatusEnd; the appearance ends");
            return true;
        }
    }
    false
}

impl Visitor {
    fn placeholder() -> Self {
        Self {
            unit: 0,
            timeline: "",
            started: f64::INFINITY,
            chimed: true,
            request: None,
            left_out: Vec::new(),
            fallback: 0,
            started_request: None,
            token: None,
            reported: None,
            done: true,
        }
    }
}

/// Loads one gate part's definition, clips, sounds and effects on the gate
/// without starting it. True once everything is prepared (or refused for
/// good: its start reports that).
fn warm(world: &mut World, gate: Entity, name: &'static str) -> bool {
    world.init_resource::<FixtureActivityProvider>();
    let definition =
        match world.resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
            provider.definition(world, VISIT_PACKAGE, name)
        }) {
            Ok(definition) => definition,
            Err(pending) => return !pending.retryable,
        };
    let (kept, _) = gate_part(&definition, name != CLOSE_GATE);
    let mut request = StartTimeline {
        owner: TimelineOwner {
            activity: FixtureActivityOwner {
                actor: gate,
                generation: 0,
            },
            kind: TimelineOwnerKind::FixtureOnly,
        },
        fixture: gate,
        definition: kept,
        bindings: TimelineBindings::default(),
        companions: Vec::new(),
        timeout_secs: NO_TIMEOUT,
        timeout_budget: TimelineTimeoutBudget::PlayerWall,
    };
    match world.resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
        provider.prepare_fixture_only_bindings(world, &mut request)
    }) {
        Ok(()) => true,
        Err(pending) => !pending.retryable,
    }
}

/// `PlayGateCharacterAppearTimelineInternal`'s NPC side after the chime and
/// its delay (JP 6.8.1 native): `character = FindNPC(unit)`, then
/// `character.ForceUpdateObjectiveImmediatelyTimeline(VISIT_BUNDLE,
/// timeline, gate.UId, character, [FindNPC(character's unit)],
/// [FixtureNpcActionLocateData(character's unit, index 0, slotId 0)])`;
/// `TryCancelCurrentObjective` (false: the method returns there);
/// `SetImmediatelyExecuteNextObjective(true)`;
/// `SetCanInterruptTalkData(true, ImmediatelyFixtureTimeline)`;
/// `WaitUntil`, then `WaitWhile`, the NPC fixture timeline list holds a
/// timeline acted by the character; `FixtureManager.ReleaseUsingFixture(
/// unit, gate)`; `TryCancelCurrentObjective`, and on true
/// `ForceUpdateObjective` and `SetImmediatelyExecuteNextObjective(true)`.
/// The model's call resets the AI and sets ImmediatelyFixture talk data; the
/// NPC decision ladder has no route from that data to the timeline
/// objective yet, so none of these calls is made and the NPC keeps its own
/// objective. This is the one place that routing is wired from.
fn force_update_objective_immediately_timeline(
    world: &mut World,
    gate: Entity,
    visitor: &Visitor,
    t: f64,
) {
    let uid = world
        .get::<FixtureActivityIdentity>(gate)
        .map(|identity| identity.uid.clone());
    let character = crate::npc::gate_entries::find_npc(world, visitor.unit);
    warn!("[gate] t={t:.3} NPCAvatarPresenter.ForceUpdateObjectiveImmediatelyTimeline(\"{VISIT_BUNDLE}\", \"{}\", gate uid {uid:?}, main character {character:?}, [{character:?}], [FixtureNpcActionLocateData {{ gameCharacterUnitId: {}, index: 0, slotId: 0 }}]) for unit {}: not routed: the NPC decision ladder has no objective 12 route (next NPC lane); TryCancelCurrentObjective, SetImmediatelyExecuteNextObjective(true), SetCanInterruptTalkData(true, 12), the waits on the NPC's fixture timeline, ReleaseUsingFixture({}, the gate) and the second cancel are not made", visitor.timeline, visitor.unit, visitor.unit, visitor.unit);
}

/// One frame of a visitor's `PlayGateCharacterAppearTimelineInternal` (or
/// of the close timeline, which has no NPC and no chime).
fn visitor_step(
    world: &mut World,
    gate: Entity,
    visitor: &mut Visitor,
    t: f64,
    generation: &mut u64,
    earlier_started: bool,
) {
    if visitor.done || !visitor.started.is_finite() || t + 1e-9 < visitor.started {
        return;
    }
    if !visitor.chimed {
        visitor.chimed = true;
        se(world, SE_CHIME, "gate appearance");
        info!("[gate] t={t:.3} PlayGateCharacterAppearTimelineInternal(unit {}, {}): PlaySEOneShot({SE_CHIME}); Delay(0.2 s)", visitor.unit, visitor.timeline);
    }
    let timeline_start = if visitor.unit == 0 {
        visitor.started
    } else {
        visitor.started + CHIME_DELAY
    };
    if t + 1e-9 < timeline_start {
        return;
    }
    if let Some(token) = visitor.token {
        let status = world
            .resource::<FixtureActivityTimelines>()
            .status(token)
            .cloned();
        let time = world
            .resource::<FixtureActivityTimelines>()
            .sampled_time(token);
        match status {
            Some(TimelineStatus::Playing { .. }) | Some(TimelineStatus::Preparing) => {}
            // Another gate part admitted on the same frame took the gate
            // first: this one steps down and starts again.
            Some(TimelineStatus::Failed(error))
                if error.message.contains("belongs to another") && visitor.fallback < 2 =>
            {
                timeline::cancel_and_release(world, token);
                visitor.token = None;
                if let Some(mut request) = visitor.started_request.take() {
                    step_down(visitor, &mut request, &error.message);
                }
            }
            other => {
                let word = match other {
                    Some(TimelineStatus::Completed) => "completed".to_owned(),
                    Some(TimelineStatus::Failed(error)) => format!("failed: {error}"),
                    _ => "cancelled".to_owned(),
                };
                info!(
                    "[gate] t={t:.3} {} (unit {}): the gate part {word} at director time {time:?}",
                    visitor.timeline, visitor.unit
                );
                timeline::cancel_and_release(world, token);
                if visitor.unit != 0 {
                    info!("[gate] PlayGateCharacterAppearTimelineInternal(unit {}): the gate part ends; the NPC side after the forced timeline (its waits, ReleaseUsingFixture and the second cancel) is not routed with it", visitor.unit);
                }
                visitor.done = true;
            }
        }
        return;
    }
    // The visitor's forced timeline: the NPC part is the NPC runtime's; the
    // timeline's own gate, SE and Control tracks play on the gate.
    world.init_resource::<FixtureActivityProvider>();
    let definition =
        match world.resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
            provider.definition(world, VISIT_PACKAGE, visitor.timeline)
        }) {
            Ok(definition) => definition,
            Err(pending) if pending.retryable => return,
            Err(pending) => {
                error!(
                    "[gate] {} (unit {}): {}: {}; nothing plays",
                    visitor.timeline, visitor.unit, pending.stage, pending.reason
                );
                visitor.done = true;
                return;
            }
        };
    let mut request = match visitor.request.take() {
        Some(request) => request,
        None => {
            *generation += 1;
            let (kept, left_out) = gate_part(&definition, visitor.unit != 0);
            if visitor.unit != 0 {
                force_update_objective_immediately_timeline(world, gate, visitor, t);
                info!("[gate] t={t:.3} {} (unit {}): the timeline's own gate, SE and Control tracks play on the gate; the NPC's tracks left out: {:?}", visitor.timeline, visitor.unit, left_out);
            }
            visitor.left_out = left_out;
            StartTimeline {
                owner: TimelineOwner {
                    activity: FixtureActivityOwner {
                        actor: gate,
                        generation: *generation,
                    },
                    kind: TimelineOwnerKind::FixtureOnly,
                },
                fixture: gate,
                definition: kept,
                bindings: TimelineBindings::default(),
                companions: Vec::new(),
                timeout_secs: NO_TIMEOUT,
                timeout_budget: TimelineTimeoutBudget::PlayerWall,
            }
        }
    };
    let prepared = world.resource_scope(|world, mut provider: Mut<FixtureActivityProvider>| {
        provider.prepare_fixture_only_bindings(world, &mut request)
    });
    match prepared {
        Ok(()) => {}
        Err(pending) if pending.retryable => {
            if visitor.reported.as_deref() != Some(pending.reason.as_str()) {
                info!(
                    "[gate] {} (unit {}) waits: {}: {}",
                    visitor.timeline, visitor.unit, pending.stage, pending.reason
                );
                visitor.reported = Some(pending.reason.clone());
            }
            visitor.request = Some(request);
            return;
        }
        Err(pending) => {
            let reason = format!("{}: {}", pending.stage, pending.reason);
            if !step_down(visitor, &mut request, &reason) {
                error!(
                    "[gate] {} (unit {}): {reason}; its gate part does not play",
                    visitor.timeline, visitor.unit
                );
                visitor.done = true;
            }
            return;
        }
    }
    if !earlier_started {
        if visitor.reported.as_deref() != Some(WAITS_FOR_EARLIER) {
            info!(
                "[gate] t={t:.3} {} (unit {}) is prepared and waits for the earlier gate part to start",
                visitor.timeline, visitor.unit
            );
            visitor.reported = Some(WAITS_FOR_EARLIER.to_owned());
        }
        visitor.request = Some(request);
        return;
    }
    if let Err(error) = timeline::validate_start(world, &request) {
        if error.retryable {
            visitor.request = Some(request);
            return;
        }
        if step_down(visitor, &mut request, &error.message) {
            return;
        }
        error!(
            "[gate] {} (unit {}): {error}; its gate part does not play",
            visitor.timeline, visitor.unit
        );
        visitor.done = true;
        return;
    }
    let token = world
        .resource_mut::<FixtureActivityTimelines>()
        .request_start(request.clone());
    info!(
        "[gate] t={t:.3} {} (unit {}) starts on the gate: director {} duration {:.4} s, {} tracks ({} Control clips driven, {} refused), SE {:?}",
        visitor.timeline,
        visitor.unit,
        request.definition.director.path_id,
        request.definition.duration,
        request.definition.tracks.len(),
        request.bindings.controls.len(),
        request.bindings.refused_controls.len(),
        request
            .definition
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .filter_map(|clip| match &clip.payload {
                timeline::TimelinePayload::Se { cue, .. } => Some(format!("{cue}@{:.3}", clip.start)),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    visitor.token = Some(token);
    visitor.started_request = Some(request);
}

/// The tracks of a visitor timeline that play on the gate with no NPC: the
/// fixture track, the SE and the Control tracks.
fn gate_part(
    definition: &TimelineDefinition,
    visitor: bool,
) -> (std::sync::Arc<TimelineDefinition>, Vec<String>) {
    if !visitor {
        return (std::sync::Arc::new(definition.clone()), Vec::new());
    }
    let mut kept = definition.clone();
    let mut left_out = Vec::new();
    kept.tracks.retain(|track| {
        let keep = matches!(track.class.as_str(), "SETrack" | "ControlTrack")
            || (track.class == "AnimationTrack" && track.name == "Fixture1");
        if !keep {
            left_out.push(format!("{} {}", track.class, track.name));
        }
        keep
    });
    (std::sync::Arc::new(kept), left_out)
}

/// Another timeline holds the gate here (one director per animator, and
/// one owner per particle object): the gate part steps down, first without
/// its fixture track, then to its SE tracks alone. The source lets several
/// directors write one Animator; this runner does not. Returns false once
/// nothing is left to drop.
fn step_down(visitor: &mut Visitor, request: &mut StartTimeline, reason: &str) -> bool {
    if visitor.fallback >= 2 {
        return false;
    }
    visitor.fallback += 1;
    let se_only = visitor.fallback == 2;
    let mut kept = (*request.definition).clone();
    kept.tracks.retain(|track| {
        let keep = if se_only {
            track.class == "SETrack"
        } else {
            track.class != "AnimationTrack"
        };
        if !keep {
            visitor
                .left_out
                .push(format!("{} {}", track.class, track.name));
        }
        keep
    });
    error!(
        "[gate] {} (unit {}): {reason}; the gate is held by another timeline here: {}",
        visitor.timeline,
        visitor.unit,
        if se_only {
            "its SE tracks play alone"
        } else {
            "it plays without its fixture track"
        }
    );
    request.definition = std::sync::Arc::new(kept);
    request.bindings = TimelineBindings::default();
    visitor.request = Some(request.clone());
    true
}

/// The cut-scene's start callback (`onStartFadeIn`), at `SetupInternal`.
pub(crate) fn cut_scene_started(world: &mut World, cast: &Cast) {
    match cast.play.caller {
        CastCaller::GoHome => {
            // OnStartCutSceneAsync(targetCharacterId): the NPCs of other
            // units (none remain after DisposeNPCAll).
            let target = cast.play.units.first().copied().unwrap_or(0);
            let others = crate::npc::gate_entries::on_start_cut_scene(world, target);
            info!(
                "[gate] OnStartCutSceneAsync(unit {target}): HideAndCancelObjective on {others:?}"
            );
        }
        CastCaller::Invite => {
            info!("[gate] PlayInviteCutSceneAsync: no start callback");
        }
    }
    info!(
        "[gate] joystick enabled before the cut-scene state: {:?}",
        joystick(world)
    );
}

/// The cut-scene's end callback, inside `EndAsync` after the view is disposed.
pub(crate) fn cut_scene_end_callback(world: &mut World, cast: &Cast) {
    match cast.play.caller {
        CastCaller::Invite => {
            let mut units = Vec::new();
            for (unit, avatar) in cast.avatars() {
                let disposed = crate::cutscene::dispose_avatar(world, avatar);
                info!("[gate] OnEndCutScene: DisposeNPC(the cut-scene avatar of unit {unit}): {disposed}; Yield");
                units.push(unit);
            }
            let frame = frame(world);
            if let Some(mut flow) = world.get_resource_mut::<GateFlow>() {
                flow.create_after_yield = Some((units, frame));
            }
        }
        CastCaller::GoHome => {
            for (_, avatar) in cast.avatars() {
                crate::cutscene::dispose_avatar(world, avatar);
            }
            dispose_npc_all(world, "OnEndGoHomeCutScene");
        }
    }
}

/// `CutSceneExecutor.PlayAsync` returned (after the presenter's fade out,
/// or at once when its load was refused).
/// Each frame of this flow's cut-scene: the cast avatars' parent, whether
/// it stays put, and which avatars carry a cloth runtime.
fn watch_cast_space(world: &mut World) {
    let members: Vec<(u32, Option<Entity>, bool)> = world
        .query_filtered::<(
            &CharacterUnitId,
            Option<&ChildOf>,
            Has<crate::cloth_runtime::ClothRuntime>,
        ), With<crate::cutscene::CutSceneAvatar>>()
        .iter(world)
        .map(|(unit, parent, cloth)| (unit.0, parent.map(ChildOf::parent), cloth))
        .collect();
    let Some(root) = members.iter().find_map(|(_, parent, _)| *parent) else {
        return;
    };
    let Some(pose) = world
        .get::<GlobalTransform>(root)
        .map(|global| global.compute_transform())
    else {
        return;
    };
    let mut space = world
        .resource_mut::<GateFlow>()
        .cast_space
        .take()
        .filter(|space| space.root == root)
        .unwrap_or_else(|| {
            let (yaw, pitch, roll) = pose.rotation.to_euler(bevy::math::EulerRot::YXZ);
            info!(
                "[gate] cast cloth space: avatars {:?} under the character root {root:?}: world translation {:.3?}, rotation yaw {:.3} pitch {:.4} roll {:.4} degrees, scale {:.4?}",
                members.iter().map(|(unit, parent, _)| (*unit, *parent)).collect::<Vec<_>>(),
                pose.translation.to_array(),
                yaw.to_degrees(),
                pitch.to_degrees(),
                roll.to_degrees(),
                pose.scale.to_array()
            );
            CastSpace {
                root,
                first: pose,
                drift_translation: 0.0,
                drift_degrees: 0.0,
                cloth: Vec::new(),
                frames: 0,
            }
        });
    space.frames += 1;
    space.drift_translation = space
        .drift_translation
        .max(pose.translation.distance(space.first.translation));
    space.drift_degrees = space.drift_degrees.max(
        pose.rotation
            .angle_between(space.first.rotation)
            .to_degrees(),
    );
    for (unit, parent, cloth) in &members {
        if *cloth && !space.cloth.contains(unit) {
            space.cloth.push(*unit);
            info!(
                "[gate] cast cloth space: unit {unit}'s avatar carries a cloth runtime at frame {} of the cut-scene, parent {parent:?}",
                space.frames
            );
        }
    }
    world.resource_mut::<GateFlow>().cast_space = Some(space);
}

pub(crate) fn cut_scene_returned(world: &mut World, cast: Cast) {
    if let Some(space) = world
        .get_resource_mut::<GateFlow>()
        .and_then(|mut flow| flow.cast_space.take())
    {
        info!(
            "[gate] cast cloth space over {} frames: the character root {:?} moved at most {:.6} m and turned at most {:.6} degrees; avatars with a cloth runtime {:?}",
            space.frames, space.root, space.drift_translation, space.drift_degrees, space.cloth
        );
    }
    let outcome = cast.outcome.clone().unwrap_or(Ok(()));

    for (_, avatar) in cast.avatars() {
        crate::cutscene::dispose_avatar(world, avatar);
    }
    info!(
        "[gate] {}: CutSceneExecutor.PlayAsync returned {outcome:?} (joystick enabled {:?})",
        cast.play.caller.name(),
        joystick(world)
    );
    let Some(mut flow) = world.get_resource_mut::<GateFlow>() else {
        return;
    };
    let stage = std::mem::replace(&mut flow.stage, Stage::Idle);
    let next = match (cast.play.caller, stage) {
        (CastCaller::Invite, Stage::InviteCutScene) => white_in(world),
        (CastCaller::GoHome, Stage::GoHome { gate, skin, change }) => {
            info!("[gate] TryShowGoHomeCutSceneAsync: true; ExecuteChangeGateProcessAsync: ChangeUIScreen(home) (UI)");
            change_gate(world, gate, skin, change)
        }
        (_, other) => other,
    };
    world.resource_mut::<GateFlow>().stage = next;
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<GateModelPackages>()
        .add_systems(Startup, start)
        .add_systems(Update, advance.after(crate::cutscene::advance));
}
