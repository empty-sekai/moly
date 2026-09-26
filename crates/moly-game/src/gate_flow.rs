//! The gate's presentations after a server reply: the invite cut-scene and
//! the gate change with its visitors coming out of the gate (JP 6.8.1).
//!
//! Invite (`ScreenLayerMysekaiGateInvitationPresenter.ReserveProcessAsync`,
//! after the reserve reply): the pass check (`HasMysekaiColorfulPass`, else
//! the expired-pass dialog), `MysekaiBootData.AddGateCharacters`,
//! `MysekaiTalkDataStore.UpdateTalkList`, the view's fade out and the send
//! dialog (UI), then, when the unit is not present (`IsExistNPC`),
//! `PlayInviteCutSceneAsync`: the master cut-scene of the invite condition
//! for the unit, played at the gate's view transform with a new avatar and
//! the player hidden; its end callback (`OnEndCutScene`) disposes the
//! cut-scene avatar, creates the unit's NPC and whites the screen out at
//! once (`WhiteOut(0, 0)`). Then `WhiteIn(0, 0.2)` and a 1.0 s delay.
//!
//! Gate change (`MysekaiGateUtility.ExecuteChangeGateProcessAsync`, after the
//! change reply): `TryShowGoHomeCutSceneAsync` picks one present NPC
//! (`RandomPick`: `UnityEngine.Random.Range(0, count)`), finds the leave
//! cut-scene of its unit, disposes every NPC (`DisposeNPCAll`) and plays it at
//! the gate with a new avatar (its end callback disposes every NPC again);
//! then `FadeOutLayer`, `ChangeGateAction` (`BackUIScreen`,
//! `SiteLayoutUtility.ChangeGate`: the gate shows the new gate's model and
//! `se_gate_change` plays; a 1.0 s delay), `StartCharacterAppearanceAsync`
//! (not awaited: the talk list, the new visitors created, then
//! `PlayGateCharacterAppearTimeline`: `ShowEffect(fx_fixture_home_gate_stay)`;
//! each visitor's `PlayGateCharacterAppearTimelineInternal` (`se_gate_chime`,
//! 0.2 s, the visitor's forced fixture timeline on the gate:
//! `tl_visitgate_w_001` for the first, `tl_visitgate_w_002` for the others,
//! of the literal package `mdl_non0006_gate_lon1`), 1000 ms apart;
//! `HideEffect`; `PlayCloseGateAsync(tl_visitgate_w_003)` with no NPC), then
//! `FadeInLayer`.
//!
//! The model of the gate: `MysekaiGateModel.AssetBundleName` is the skin's
//! bundle (the skin row's type selects the unit or the common skin table)
//! when the gate has a skin (id > 0), else the gate row's own bundle.
//!
//! Server values (instruments, named after the reply fields; no other
//! producer here):
//! - `MOLY_GATE_INVITE=<unit>[,pass=<0|1>]`: the reserve reply's
//!   `userMysekaiGateCharacters` row for the reserved unit, and the client's
//!   `HasMysekaiColorfulPass` (default 1);
//! - `MOLY_GATE_CHANGE=<mysekaiGateId>[/<mysekaiGateSkinId>]:<unit>,<unit>...`:
//!   the change reply's `userMysekaiGates` row set at the home site (gate id
//!   and skin id) and its `userMysekaiGateCharacters` (the new visitors'
//!   units).
//! Each is delivered once the home site stands with its gate and its
//! visitors placed, after a 2 s settle (a product choice), through the
//! same entries the gate screens call ([`reserve_process`],
//! [`execute_change_gate_process`]).
//!
//! NPC-side calls that are the NPC runtime's and are named, not made, here:
//! the created visitor's placement at the gate's first action point end
//! location and its objective restart (`TryCancelCurrentObjective`,
//! `ForceUpdateObjective`, `Show`), `HideAndCancelObjective`, and each
//! visitor's forced gate timeline (`ForceUpdateObjectiveImmediatelyTimeline`,
//! the wait for it, `ReleaseUsingFixture`). Meanwhile the visitors are
//! created where the NPC runtime places new members, and each visitor
//! timeline's own gate, SE and Control tracks play on the gate with no NPC.

use std::collections::HashSet;

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
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

const INVITE: &str = "MOLY_GATE_INVITE";
const CHANGE: &str = "MOLY_GATE_CHANGE";
const PLAYER_DATA: &str = "moly://fixture-models/player-data.json";
const INVITE_CONDITION: &str = "mysekai_character_talk_character_invite_game_character_unit_id";
const LEAVE_CONDITION: &str = "mysekai_character_talk_character_leave_game_character_unit_id";
/// The literal fixture-timeline bundle of the visitor timelines
/// (`mysekai/fixture_timeline/mdl_non0006_gate_lon1`), whatever the skin.
const VISIT_PACKAGE: &str = "mysekai__fixture_timeline__mdl_non0006_gate_lon1";
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

/// The bundles a gate fixture may show (`MysekaiGateModel.AssetBundleName`
/// over the gate, unit skin and common skin tables), as fixture packages.
#[derive(Resource, Default, Debug)]
pub(crate) struct GateModelPackages(pub(crate) HashSet<String>);

#[derive(Clone, Debug)]
enum Reply {
    Invite {
        unit: u32,
        pass: bool,
    },
    Change {
        gate: i64,
        skin: i64,
        units: Vec<u32>,
    },
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
    npc: Option<Entity>,
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
    /// The reply waits for the home site, its gate and its visitors.
    Waiting {
        reply: Reply,
        ready_since: Option<f64>,
    },
    /// `PlayInviteCutSceneAsync` in flight.
    InviteCutScene,
    /// `WhiteIn(0, 0.2)`, then `Delay(1.0 s)` until `until`.
    InviteWhiteIn {
        until: f64,
    },
    /// `TryShowGoHomeCutSceneAsync` in flight.
    GoHome {
        gate: i64,
        skin: i64,
        units: Vec<u32>,
    },
    /// `ChangeGateAction`'s delay after the change, until `until`.
    ChangeGate {
        units: Vec<u32>,
        until: f64,
    },
    /// `StartCharacterAppearanceAsync` and the timelines out of the gate.
    Appearance(Box<Appearance>),
    Done,
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
}

#[derive(Resource)]
pub(crate) struct GateFlow {
    stage: Stage,
    tables: Option<Handle<JsonAsset>>,
    parsed: Option<Tables>,
    /// `OnEndCutScene`'s `CreateNPC` after its `Yield`: the units and the
    /// frame the cut-scene avatars were disposed on.
    create_after_yield: Option<(Vec<u32>, u64)>,
}

fn now(world: &World) -> f64 {
    world.resource::<Time>().elapsed_secs_f64()
}

fn parse_instruments() -> Option<Reply> {
    if let Ok(raw) = std::env::var(INVITE) {
        let mut parts = raw.split(',');
        let unit = parts
            .next()
            .and_then(|unit| unit.trim().parse::<u32>().ok());
        let mut pass = true;
        let mut valid = unit.is_some();
        for part in parts {
            match part.trim().split_once('=') {
                Some(("pass", value)) if value == "0" || value == "1" => pass = value == "1",
                _ => valid = false,
            }
        }
        return match (unit, valid) {
            (Some(unit), true) => Some(Reply::Invite { unit, pass }),
            _ => {
                error!("[gate] {INVITE}={raw:?} is not <unit>[,pass=0|1]; no reserve reply is delivered");
                None
            }
        };
    }
    if let Ok(raw) = std::env::var(CHANGE) {
        let parsed = raw.split_once(':').and_then(|(gate, units)| {
            let (gate, skin) = match gate.split_once('/') {
                Some((gate, skin)) => (
                    gate.trim().parse::<i64>().ok()?,
                    skin.trim().parse::<i64>().ok()?,
                ),
                None => (gate.trim().parse::<i64>().ok()?, 0),
            };
            let units = units
                .split(',')
                .filter(|unit| !unit.trim().is_empty())
                .map(|unit| unit.trim().parse::<u32>().ok())
                .collect::<Option<Vec<_>>>()?;
            Some((gate, skin, units))
        });
        return match parsed {
            Some((gate, skin, units)) if gate > 0 && skin >= 0 => {
                Some(Reply::Change { gate, skin, units })
            }
            _ => {
                error!("[gate] {CHANGE}={raw:?} is not <gateId>[/<skinId>]:<unit>,...; no gate change reply is delivered");
                None
            }
        };
    }
    None
}

/// Startup: the instruments.
fn start(mut commands: Commands, server: Res<AssetServer>) {
    let reply = parse_instruments();
    let tables = reply
        .is_some()
        .then(|| server.load::<JsonAsset>(PLAYER_DATA));
    let stage = match reply {
        Some(reply) => {
            info!("[gate] instrument reply {reply:?}: delivered once the home site, its gate and its visitors stand");
            Stage::Waiting {
                reply,
                ready_since: None,
            }
        }
        None => Stage::Done,
    };
    commands.insert_resource(GateFlow {
        stage,
        tables,
        parsed: None,
        create_after_yield: None,
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

/// The NPCs present (not the player's avatar, not a cut-scene avatar).
fn present_npcs(world: &mut World) -> Vec<(u32, Entity)> {
    let mut npcs: Vec<(u32, Entity)> = world
        .query_filtered::<(Entity, &CharacterUnitId), (
            Without<crate::player::PlayerControlled>,
            Without<crate::cutscene::CutSceneAvatar>,
        )>()
        .iter(world)
        .map(|(entity, unit)| (unit.0, entity))
        .collect();
    npcs.sort_by_key(|(_, entity)| entity.index());
    npcs
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

/// Update (exclusive), after the cut-scene presenter.
fn advance(world: &mut World) {
    if !world.contains_resource::<GateFlow>() {
        return;
    }
    create_after_yield(world);
    if matches!(world.resource::<GateFlow>().stage, Stage::Done) {
        return;
    }
    if tables(world).is_none() {
        return;
    }
    let stage = std::mem::replace(&mut world.resource_mut::<GateFlow>().stage, Stage::Done);
    let next = match stage {
        Stage::Waiting { reply, ready_since } => match ready(world) {
            Err(reason) => {
                if ready_since.is_some() {
                    info!("[gate] the reply waits again: {reason}");
                }
                Stage::Waiting {
                    reply,
                    ready_since: None,
                }
            }
            Ok(gate) => {
                let since = ready_since.unwrap_or_else(|| now(world));
                if now(world) - since < SETTLE {
                    Stage::Waiting {
                        reply,
                        ready_since: Some(since),
                    }
                } else {
                    match reply {
                        Reply::Invite { unit, pass } => reserve_process(world, gate, unit, pass),
                        Reply::Change {
                            gate: id,
                            skin,
                            units,
                        } => execute_change_gate_process(world, gate, id, skin, units),
                    }
                }
            }
        },
        Stage::InviteWhiteIn { until } => {
            if now(world) >= until {
                info!(
                    "[gate] ReserveProcessAsync: Delay(1.0 s) done; the invitation ends (joystick enabled {:?})",
                    joystick(world)
                );
                Stage::Done
            } else {
                Stage::InviteWhiteIn { until }
            }
        }
        Stage::ChangeGate { units, until } => {
            if now(world) >= until {
                info!("[gate] ChangeGateAction: Delay(1.0 s) done; StartCharacterAppearanceAsync(...).Forget(); FadeInLayer (the UI layer: not here)");
                start_character_appearance(units)
            } else {
                Stage::ChangeGate { units, until }
            }
        }
        Stage::Appearance(mut appearance) => {
            if appearance_step(world, &mut appearance) {
                Stage::Done
            } else {
                Stage::Appearance(appearance)
            }
        }
        other => other,
    };
    world.resource_mut::<GateFlow>().stage = next;
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
        match crate::npc::spawn_temporary_units(world, &[unit]) {
            Ok(created) => info!("[gate] OnEndCutScene: CreateNPC(unit {unit}): {created:?}, where the NPC runtime places new members (the gate's ActionPoints[0].EndLoc placement is the NPC runtime's); WaitUntil(IsExistNPCAll); SendNpcSpawnFlagPayload (multiplayer: nothing here); TryCancelCurrentObjective, ForceUpdateObjective and Show are the NPC runtime's"),
            Err(reason) => error!("[gate] OnEndCutScene: CreateNPC(unit {unit}): {reason}"),
        }
    }
    crate::screen_fade::white_out(world, 0.0, 0.0, "OnEndCutScene: ScreenManager.WhiteOut");
    info!("[gate] OnEndCutScene: ScreenManager.WhiteOut(0, 0)");
}

/// `ReserveProcessAsync` after the reserve reply.
fn reserve_process(world: &mut World, gate: Entity, unit: u32, pass: bool) -> Stage {
    info!("[gate] ScreenLayerMysekaiGateInvitationPresenter.ReserveProcessAsync(unit {unit}): HasMysekaiColorfulPass = {pass} (instrument)");
    if !pass {
        info!("[gate] ReserveProcessAsync: no pass: the expired-pass dialog (UI) opens; nothing is reserved");
        return Stage::Done;
    }
    info!("[gate] ReserveProcessAsync: the reserve reply (instrument {INVITE}): userMysekaiGateCharacters row for unit {unit} (isReservation true); MysekaiBootData.AddGateCharacters and MysekaiTalkDataStore.UpdateTalkList(the reply's talk list) belong to the server model (not written here); the view's fade out and the send dialog are UI");
    let present = present_npcs(world);
    if present.iter().any(|(id, _)| *id == unit) {
        info!("[gate] ReserveProcessAsync: IsExistNPC({unit}) true: no invite cut-scene");
        return Stage::Done;
    }
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
    id: i64,
    skin: i64,
    units: Vec<u32>,
) -> Stage {
    info!("[gate] MysekaiGateUtility.ExecuteChangeGateProcessAsync: the change reply (instrument {CHANGE}): userMysekaiGates mysekaiGateId {id} mysekaiGateSkinId {skin} isSettingAtHomeSite true; userMysekaiGateCharacters units {units:?}; ChangeUIScreen(602) (UI)");
    let tutorial = world
        .get_resource::<crate::server_panel::ServerPanel>()
        .is_some_and(|panel| panel.is_tutorial());
    if tutorial {
        info!("[gate] ExecuteChangeGateProcessAsync: in the tutorial: no go-home cut-scene and no appearance");
        return change_gate(world, id, skin, Vec::new());
    }
    // TryShowGoHomeCutSceneAsync.
    let present = present_npcs(world);
    if present.is_empty() {
        info!("[gate] TryShowGoHomeCutSceneAsync: no NPC present: false");
        return change_gate(world, id, skin, units);
    }
    let draw = (crate::npc_objective::platform_seed() % present.len() as u64) as usize;
    let (unit, _) = present[draw];
    info!(
        "[gate] TryShowGoHomeCutSceneAsync: NPCs present {:?}; RandomPick: Random.Range(0, {}) = {draw} (a platform-seeded draw stands in for the engine generator): unit {unit}",
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
        return change_gate(world, id, skin, units);
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
            units,
        },
        Err(reason) => {
            error!("[gate] TryShowGoHomeCutSceneAsync: {reason}");
            change_gate(world, id, skin, units)
        }
    }
}

/// `AvatarDataStore.DisposeNPCAll`: every NPC's avatar is disposed.
fn dispose_npc_all(world: &mut World, caller: &str) {
    let npcs = present_npcs(world);
    let entities: Vec<Entity> = npcs.iter().map(|(_, entity)| *entity).collect();
    crate::npc::remove_temporary_units(world, &entities);
    info!(
        "[gate] {caller}: DisposeNPCAll: units {:?} disposed",
        npcs.iter().map(|(unit, _)| *unit).collect::<Vec<_>>()
    );
}

/// `ChangeGateAction`: `BackUIScreen`, `SiteLayoutUtility.ChangeGate`, then
/// a 1.0 s delay.
fn change_gate(world: &mut World, id: i64, skin: i64, units: Vec<u32>) -> Stage {
    info!("[gate] FadeOutLayer (the UI layer: not here); ChangeGateAction: BackUIScreen (UI)");
    let bundle = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()
        .map(|tables| tables.gate_bundle(id, skin));
    match bundle {
        Some(Ok(bundle)) => {
            let package = format!("mysekai__fixture__{bundle}");
            match swap_gate_model(world, &package) {
                Ok(old) => {
                    se(world, SE_CHANGE, "gate change");
                    info!("[gate] SiteLayoutUtility.ChangeGate -> HomeSiteController.UpdateGateModelAsync: RemovePutData; SiteView.UpdateFixture(the gate, model {old} -> {package}); AddTileData; PlaySEOneShot({SE_CHANGE}); ObjectCollisionManager.ForceUpdate (the layout reload rebuilds the collisions); the room update");
                }
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
        units,
        until: now(world) + AFTER_CHANGE,
    }
}

/// The gate's placement row shows the new model; the layout's instances are
/// reloaded (the product has no single-fixture model swap). Returns the old
/// package.
fn swap_gate_model(world: &mut World, package: &str) -> Result<String, String> {
    let masters = world
        .resource::<GateFlow>()
        .parsed
        .as_ref()
        .map(|tables| tables.gate_masters.clone())
        .unwrap_or_default();
    let placements = world
        .get_resource::<crate::fixture::FixturePlacements>()
        .cloned()
        .ok_or("the layout is not installed")?;
    let mut rows = placements.editor_rows();
    let mut old = None;
    for row in &mut rows {
        if masters.contains(&row.fixture_id) {
            old = Some(std::mem::replace(&mut row.package, package.to_owned()));
        }
    }
    let old = old.ok_or("the layout has no gate row")?;
    let next = placements.with_editor_rows(&rows, placements.next_edit_uid())?;
    world.insert_resource(next);
    crate::fixture::reload_after_save(world);
    Ok(old)
}

/// `StartCharacterAppearanceAsync`: the talk list, then
/// `SetupNpcVisitingAsync(visitingFromGate: true)` -> `ShowCharacterFromGate`.
fn start_character_appearance(units: Vec<u32>) -> Stage {
    info!("[gate] StartCharacterAppearanceAsync: MysekaiTalkDataStore.SetTalkList(the reply's talk list: the server model's, not written here); SetupNpcVisitingAsync(visitingFromGate true): ShowCharacterFromGate: SetVisitingCharacterFromGateStatusStart (event 65, no subscriber here); SetupNPC(units {units:?})");
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
        return false; // the layout reload is placing the fixtures
    };
    if !appearance.spawned {
        if appearance.units.is_empty() {
            info!("[gate] PlayGateCharacterAppearTimeline: no visitor picked by the server: logged, returns");
            return true;
        }
        match crate::npc::spawn_temporary_units(world, &appearance.units) {
            Ok(created) => {
                appearance.spawned = true;
                info!("[gate] SetupNPC: {} visitors created ({created:?}) where the NPC runtime places new members (the gate placement and the objective calls are the NPC runtime's)", created.len());
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
            if all {
                error!("[gate] the gate has no {STAY_EFFECT} node; ShowEffect finds nothing");
            }
            return false;
        };
        if !all {
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
            Err(error) if error.retryable => return false,
            Err(error) => {
                error!("[gate] gate.ShowEffect({STAY_EFFECT}): refused by the particle host: {}; the appearance goes on", error.message);
                appearance.stay_node = Some(node);
            }
        }
        appearance.next_at = t;
        for (index, unit) in appearance.units.iter().enumerate() {
            appearance.visitors.push(Visitor {
                unit: *unit,
                npc: npcs.iter().find(|(id, _)| id == unit).map(|(_, npc)| *npc),
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
                info!("[gate] TryCancelIfFixtureActionGate: the gate's fixture action for unit {} is the NPC runtime's (not here)", visitor.unit);
            }
        }
        index += 1;
    }
    for index in 0..appearance.visitors.len() {
        let mut visitor =
            std::mem::replace(&mut appearance.visitors[index], Visitor::placeholder());
        visitor_step(world, gate, &mut visitor, t, &mut appearance.generation);
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
        let stopped = appearance
            .stay
            .as_ref()
            .map(|binding| crate::fixture_timeline_particles::stop_object(world, binding));
        if let Some(node) = appearance.stay_node {
            moly_assets::scene_state::SetSourceActive {
                entity: node,
                active: false,
            }
            .apply(world);
        }
        info!("[gate] t={t:.3} gate.HideEffect({STAY_EFFECT}).Forget(): Stop() on {stopped:?} systems, SetActive(false) (the wait while its particles are alive is not modelled); PlayCloseGateAsync(mysekai/fixture_timeline/mdl_non0006_gate_lon1, {CLOSE_GATE})");
        appearance.close = Some(Visitor {
            unit: 0,
            npc: None,
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
        visitor_step(world, gate, &mut close, t, &mut appearance.generation);
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
            npc: None,
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

/// One frame of a visitor's `PlayGateCharacterAppearTimelineInternal` (or
/// of the close timeline, which has no NPC and no chime).
fn visitor_step(
    world: &mut World,
    gate: Entity,
    visitor: &mut Visitor,
    t: f64,
    generation: &mut u64,
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
                    info!("[gate] PlayGateCharacterAppearTimelineInternal(unit {}): the wait for the NPC's timeline, FixtureManager.ReleaseUsingFixture, TryCancelCurrentObjective and ForceUpdateObjective are the NPC runtime's (not here)", visitor.unit);
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
                info!("[gate] t={t:.3} NPCAvatarPresenter.ForceUpdateObjectiveImmediatelyTimeline(the gate, {}) for unit {} {:?}: the NPC's part is the NPC runtime's (not here); tracks left out: {:?}", visitor.timeline, visitor.unit, visitor.npc, left_out);
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
            let others = present_npcs(world);
            info!("[gate] OnStartCutSceneAsync(unit {:?}): HideAndCancelObjective on the other NPCs {:?} (the NPC runtime's; none remain after DisposeNPCAll)", cast.play.units, others.iter().map(|(unit, _)| *unit).collect::<Vec<_>>());
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
pub(crate) fn cut_scene_returned(world: &mut World, cast: Cast) {
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
    let stage = std::mem::replace(&mut flow.stage, Stage::Done);
    let next = match (cast.play.caller, stage) {
        (CastCaller::Invite, Stage::InviteCutScene) => white_in(world),
        (CastCaller::GoHome, Stage::GoHome { gate, skin, units }) => {
            info!("[gate] TryShowGoHomeCutSceneAsync: true; ExecuteChangeGateProcessAsync: ChangeUIScreen(home) (UI)");
            change_gate(world, gate, skin, units)
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
