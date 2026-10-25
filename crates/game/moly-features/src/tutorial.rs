//! Tutorial chapter NPC processing. The chapter/phase and external service
//! acknowledgements are explicit inputs under `controls.tutorial.process`.
//! NPC creation, placements, talks, waits and the introductory cut-scene use
//! their real client owners. Navigation widgets and tutorial service calls
//! remain external producers; their boundaries are published, never completed
//! by a client timeout or a guessed server response.

use crate::npc::{NpcAction, NpcActions};
use crate::npc_objective::events::tutorial as lower;
use crate::ui_layers::{MenuScreenType, ScreenLayerState, ScreenManager};
use bevy::prelude::*;
use moly_assets::json::{
    JsonAsset,
    master::{MasterData, MasterTable},
};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProcessInput {
    id: String,
    episode: String,
    phase: String,
    recovery_point: bool,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pose {
    position: [f32; 3],
    rotation: [f32; 4],
}
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Placement {
    unit_character_id: u32,
    position: [f32; 3],
    rotation: [f32; 4],
}
#[derive(Clone, serde::Deserialize)]
struct PlacementDocument {
    schema: String,
    placements: HashMap<String, Vec<Placement>>,
}
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProcessorDocument {
    schema: String,
    coordinate_contract: String,
    initial_offstage: [f32; 3],
    initial_player: Pose,
    after_intro_npc: Pose,
    after_talk_player_rotation: [f32; 4],
}

#[derive(Resource)]
struct AssetsPending {
    placements: Handle<JsonAsset>,
    processor: Handle<JsonAsset>,
}
#[derive(Resource, Default)]
struct TutorialAssets {
    placements: Option<Result<PlacementDocument, String>>,
    processor: Option<Result<ProcessorDocument, String>>,
    cutscenes: Option<Result<HashMap<i64, String>, String>>,
}

fn parse_cutscenes(text: &str) -> Result<HashMap<i64, String>, String> {
    let rows: Vec<serde_json::Value> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|row| {
            let id = row["id"]
                .as_i64()
                .ok_or("tutorial cut-scene row has no id")?;
            let timeline = row["timelineAssetbundleName"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("tutorial cut-scene row has no timelineAssetbundleName")?
                .to_owned();
            Ok((id, timeline))
        })
        .collect()
}
const CUTSCENES: MasterTable<HashMap<i64, String>> = MasterTable {
    table: "mysekaiTutorialCutScenes",
    name: "mysekaiTutorialCutScenes (tutorial processor)",
    parse: parse_cutscenes,
};

/// Shared input ownership used by the joystick and the keyboard adaptation.
#[derive(Resource, Default)]
pub struct TutorialInput {
    pub movement_blocked: bool,
    pub taps_blocked: bool,
}

/// Observable client outcome. `boundary` names the next real producer needed;
/// it is not a successful server progression acknowledgement.
#[derive(Resource, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TutorialStatus {
    pub id: Option<String>,
    pub episode: Option<String>,
    pub phase: Option<String>,
    pub operation: String,
    pub state: String,
    pub boundary: Option<String>,
    pub error: Option<String>,
    pub events: Vec<serde_json::Value>,
}

#[derive(Clone)]
enum Op {
    Screen(MenuScreenType),
    Find(u32),
    Existing(u32),
    Placement(&'static str, bool),
    ForceTalk(u32, i32),
    WaitTalk(u32, i32, f32, i32, bool),
    Freewalk(u32, i32, bool),
    Show(u32),
    EnableJoystick(bool),
    WaitingOverwrite(u32,i32),
    MissionChildren,
    IntroPrepare,
    IntroCutscene,
    IntroAfterTalk,
    ChangeSite(u32, i32),
    HideNpc(u32, i32),
    AwaitTalks,
    Boundary(&'static str),
}

struct Run {
    input: ProcessInput,
    ops: VecDeque<Op>,
    forced: Option<(Entity, i32)>,
    hide: Option<HideRun>,
}
struct HideRun { actor: Entity, delay: crate::tween_clock::Delay, phase: u8 }

struct WaitingTalk {
    run: String,
    actor: Entity,
    unit: u32,
    id: i32,
    material: i32,
    seen_talk: bool,
    seen_not_talk: bool,
    sequential: bool,
    delay: Option<crate::tween_clock::Delay>,
    blocks: bool,
}
#[derive(Resource, Default)]
struct Processor {
    revision: Option<u64>,
    seen: HashSet<String>,
    queue: VecDeque<ProcessInput>,
    run: Option<Run>,
    waits: Vec<WaitingTalk>,
    returned_cutscene: Option<Result<(), String>>,
    cutscene_started: bool,
    input_lock_elapsed: Option<f32>,
}

const VBS: &[u32] = &[9, 10, 11, 12, 29, 39, 49];
const MMJ: &[u32] = &[5, 6, 7, 8, 28, 33];
const LEO: &[u32] = &[1, 2, 3, 4, 27, 42];
const WXS: &[u32] = &[13, 14, 15, 16, 30, 55];
const N25: &[u32] = &[17, 18, 19, 20, 31];

fn find_all(ops: &mut VecDeque<Op>, units: &[u32]) {
    ops.extend(units.iter().copied().map(Op::Find));
}
fn waiting(
    ops: &mut VecDeque<Op>,
    units: &[u32],
    first: i32,
    limits: &[f32],
    materials: &[i32],
    blocks: bool,
) {
    for (i, unit) in units.iter().enumerate() {
        ops.push_back(Op::WaitTalk(
            *unit,
            first + i as i32,
            limits.get(i).copied().unwrap_or(0.0),
            materials.get(i).copied().unwrap_or(0),
            blocks,
        ));
    }
}
fn freewalk(ops: &mut VecDeque<Op>, units: &[u32], first: i32, force_random: bool) {
    ops.extend(
        units
            .iter()
            .enumerate()
            .map(|(i, unit)| Op::Freewalk(*unit, first + i as i32, force_random)),
    );
}

/// The phase names are the chapter's actual call boundaries. Chapters whose
/// dialogue follows a cut-scene admit `afterCutscene` only from that producer;
/// a scene/recovery entry uses `start`. This makes partial chapter ownership
/// explicit without inventing a replacement tutorial UI.
fn plan(input: &ProcessInput) -> Result<VecDeque<Op>, String> {
    let mut ops = VecDeque::new();
    let phase = input.phase.as_str();
    match (input.episode.as_str(), phase) {
        ("1_1", "start") => {
            ops.extend([
                Op::Screen(MenuScreenType::MysekaiHome),
                Op::Find(21),
                Op::IntroPrepare,
                Op::IntroCutscene,
                Op::Show(21),
                Op::Screen(MenuScreenType::MysekaiHome),
                Op::ForceTalk(21, 1),
                Op::IntroAfterTalk,
                Op::WaitTalk(21, 2, 0.0, 0, false),
                Op::Screen(MenuScreenType::MysekaiSiteMove),
                Op::Boundary(
                    "SeqNext: navigation icon disposal and chapter manager acknowledgement",
                ),
            ]);
        }
        ("24_3", "waiting")=> {ops.extend([Op::Existing(21),Op::WaitingOverwrite(21,99),Op::Boundary("SeqNext then ExecuteProcess(24,4) callback")]);}
        (_, "enableJoyStick")=>{ops.push_back(Op::EnableJoystick(true));}
        ("1_2", "start") => {
            ops.extend([Op::Existing(21), Op::Show(21), Op::ChangeSite(21,1),
                Op::Placement("tutorial_1_2",false), Op::Screen(MenuScreenType::MysekaiMyRoom),
                Op::Boundary("SeqNext then ExecuteProcess(1,3)")]);
        }
        ("1_2", "afterChangeSite") => {
            ops.extend([
                Op::Existing(21),
                Op::Show(21),
                Op::Placement("tutorial_1_2", false),
                Op::Screen(MenuScreenType::MysekaiMyRoom),
                Op::Boundary("SeqNext then ExecuteProcess(1,3)"),
            ]);
        }
        ("1_3", "waiting") => {
            ops.extend([
                Op::Existing(21),
                Op::WaitTalk(21, 3, 0.0, 0, true),
                Op::Boundary("waiting-talk navigation callback"),
            ]);
        }
        ("1_4", "beforeForcedTalk") => {
            ops.extend([
                Op::Existing(21),
                Op::ForceTalk(21, 4),
                Op::WaitTalk(21, 5, 0.0, 0, false),
                Op::Boundary("remaining chapter screen/navigation producer"),
            ]);
        }
        ("4_1", "beforeForcedTalk") => {
            ops.extend([
                Op::Find(21),
                Op::ForceTalk(21, 8),
                Op::Boundary("chapter UI/mission operations before waiting talk 8"),
            ]);
        }
        ("4_1", "waiting") => {
            ops.extend([
                Op::Existing(21),
                Op::WaitTalk(21, 8, 0.0, 0, false),
                Op::Boundary("remaining chapter screen/navigation producer"),
            ]);
        }
        ("5_1", "beforeForcedTalk") => {
            ops.extend([
                Op::Find(21),
                Op::ForceTalk(21, 9),
                Op::Boundary("chapter operations before waiting talk 10"),
            ]);
        }
        ("5_1" | "5_2", "waiting") => {
            ops.extend([
                Op::Existing(21),
                Op::WaitTalk(21, 10, 0.0, 0, false),
                Op::Boundary("remaining chapter screen/navigation producer"),
            ]);
        }
        ("5_4", "beforeForcedTalk") => {
            ops.extend([
                Op::Existing(21),
                Op::ForceTalk(21, 11),
                Op::HideNpc(21,500),
                Op::EnableJoystick(true),
                Op::Boundary("homebutton configuration and SectionNext before home transition"),
            ]);
        }
        ("6_1", "waiting") => {
            ops.extend([
                Op::Existing(21),
                Op::WaitTalk(21, 12, 0.0, 0, true),
                Op::AwaitTalks,
                Op::HideNpc(21,500),
                Op::Boundary("tutorial cut-scene 1 producer"),
            ]);
        }
        ("6_2", "afterPlacement") => {
            find_all(&mut ops, WXS);
            waiting(&mut ops, WXS, 13, &[], &[47, 48, 49, 50, 55, 60], true);
            ops.push_back(Op::Boundary(
                "all collect callbacks then tutorial cut-scene 2",
            ));
        }
        ("7_1", "start") => {
            ops.push_back(Op::Screen(MenuScreenType::MysekaiHome));
            find_all(&mut ops, WXS);
            ops.push_back(Op::Placement("tutorial_7_1", true));
            waiting(&mut ops, WXS, 19, &[2., 1., 3., 3., 3., 3.], &[], false);
            ops.extend([
                Op::MissionChildren,
                Op::Boundary("ReceiveMission: PostUserMysekaiTutorialNormalMissionRewardApi"),
            ]);
        }
        ("8_1", "start") => {
            ops.push_back(Op::Screen(MenuScreenType::MysekaiHome));
            find_all(&mut ops, WXS);
            ops.push_back(Op::Placement("tutorial_7_1", true));
            freewalk(&mut ops, WXS, 25, true);
            ops.push_back(Op::Boundary(
                "SeqNext; gate navigation remains owned by chapter UI",
            ));
        }
        ("10_1", "afterCutscene") => {
            for unit in N25 {
                ops.push_back(Op::Existing(*unit));
            }
            waiting(&mut ops, N25, 31, &[], &[51, 52, 53, 54, 55], true);
            ops.push_back(Op::Boundary(
                "all collect callbacks and chapter continuation",
            ));
        }
        ("10_2", "afterCutscene") => {
            ops.extend([
                Op::Existing(19),
                Op::ForceTalk(19, 36),
                Op::WaitTalk(19, 36, 0.1, 0, false),
                Op::Boundary("NPC20 creation and ChangeSiteNPC(1) with chapter pose"),
            ]);
        }
        ("10_2", "npc20Waiting") => {
            ops.extend([
                Op::Existing(20),
                Op::WaitTalk(20, 37, 0.0, 0, true),
                Op::AwaitTalks,
                Op::WaitTalk(20, 37, 0.1, 0, false),
                Op::Boundary("chapter continuation"),
            ]);
        }
        ("11_1", "start") => {
            find_all(&mut ops, N25);
            ops.push_back(Op::Placement("tutorial_10_1", true));
            freewalk(&mut ops, N25, 38, false);
            ops.extend([
                Op::Screen(MenuScreenType::MysekaiHome),
                Op::MissionChildren,
                Op::Boundary("tutorial mission receive and chapter continuation"),
            ]);
        }
        ("12_1", "start") => {
            find_all(&mut ops, N25);
            ops.push_back(Op::Placement("tutorial_12_1", true));
            ops.push_back(Op::Screen(MenuScreenType::MysekaiHome));
            waiting(&mut ops, N25, 43, &[6., 8., 3., 2., 10.], &[], false);
            ops.push_back(Op::Boundary("chapter navigation and sequence continuation"));
        }
        ("14_1", "afterCutscene") => {
            for unit in MMJ {
                ops.push_back(Op::Existing(*unit));
            }
            waiting(&mut ops, MMJ, 48, &[], &[39, 40, 41, 42, 55, 56], true);
            ops.push_back(Op::Boundary(
                "all collect callbacks then tutorial cut-scene 8",
            ));
        }
        ("15_1", "start") => {
            find_all(&mut ops, MMJ);
            ops.push_back(Op::Placement("tutorial_14_1", true));
            waiting(&mut ops, MMJ, 54, &[6., 6., 2., 2., 5., 5.], &[], false);
            ops.push_back(Op::Boundary(
                "IsChildMissionAllAchieved branch and chapter UI continuation",
            ));
        }
        ("15_4", "start") => {
            find_all(&mut ops, MMJ);
            freewalk(&mut ops, MMJ, 54, false);
            ops.extend([
                Op::MissionChildren,
                Op::Boundary("tutorial mission receive and chapter continuation"),
            ]);
        }
        ("16_1", "start") => {
            find_all(&mut ops, MMJ);
            ops.push_back(Op::Placement("tutorial_14_1", true));
            freewalk(&mut ops, MMJ, 60, false);
            ops.push_back(Op::Boundary("gate navigation then SeqNext"));
        }
        ("18_1", "afterCutscene") => {
            for unit in VBS {
                ops.push_back(Op::Existing(*unit));
            }
            waiting(&mut ops, VBS, 66, &[], &[43, 44, 45, 46, 55, 57, 59], true);
            ops.push_back(Op::Boundary(
                "all collect callbacks then tutorial cut-scene 11",
            ));
        }
        ("19_1", "start") => {
            find_all(&mut ops, VBS);
            ops.push_back(Op::Placement("tutorial_18_1", true));
            waiting(&mut ops, VBS, 73, &[3., 3., 2., 2., 6., 6., 6.], &[], false);
            ops.extend([
                Op::Screen(MenuScreenType::MysekaiHome),
                Op::MissionChildren,
                Op::Boundary("tutorial mission receive then cut-scene 12"),
            ]);
        }
        ("20_1", "start") => {
            find_all(&mut ops, VBS);
            ops.push_back(Op::Placement("tutorial_20_1", true));
            freewalk(&mut ops, VBS, 80, false);
            ops.push_back(Op::Boundary("gate navigation then SeqNext"));
        }
        ("22_1", "afterPlacement") => {
            find_all(&mut ops, LEO);
            waiting(&mut ops, LEO, 87, &[], &[35, 36, 37, 38, 55, 58], true);
            ops.push_back(Op::Boundary(
                "all collect callbacks then tutorial cut-scene 15",
            ));
        }
        ("23_1", "start") => {
            find_all(&mut ops, LEO);
            ops.push_back(Op::Placement("tutorial_22_1", true));
            waiting(&mut ops, LEO, 93, &[6., 6., 6., 6., 3., 3.], &[], false);
            ops.extend([
                Op::MissionChildren,
                Op::Boundary("tutorial mission receive then cut-scene 16"),
            ]);
        }
        ("24_1", "start") => {
            find_all(&mut ops, LEO);
            ops.push_back(Op::Placement("tutorial_24_1", true));
            waiting(&mut ops, LEO, 99, &[5., 6., 7., 5., 3., 4.], &[], false);
            ops.push_back(Op::Boundary("gate navigation then SeqNext"));
        }
        _ => {
            return Err(format!(
                "tutorial {} phase {:?} has no admitted NPC call boundary",
                input.episode, input.phase
            ));
        }
    }
    Ok(ops)
}

fn load(mut commands: Commands, server: Res<AssetServer>, mut masters: ResMut<MasterData>) {
    commands.insert_resource(AssetsPending {
        placements: server.load("moly://tutorial/placements.json"),
        processor: server.load("moly://tutorial/processor.json"),
    });
    masters.request(&CUTSCENES);
}

fn read_assets(world: &mut World) {
    let Some(pending) = world.get_resource::<AssetsPending>() else {
        return;
    };
    let handles = (pending.placements.clone(), pending.processor.clone());
    if world.resource::<TutorialAssets>().placements.is_none() {
        if let Some(asset) = world.resource::<Assets<JsonAsset>>().get(&handles.0) {
            let parsed = serde_json::from_str::<PlacementDocument>(&asset.0)
                .map_err(|e| e.to_string())
                .and_then(|doc| {
                    if doc.schema == "moly-tutorial-placements/1" {
                        Ok(doc)
                    } else {
                        Err("tutorial placement schema mismatch".into())
                    }
                });
            world.resource_mut::<TutorialAssets>().placements = Some(parsed);
        }
    }
    if world.resource::<TutorialAssets>().processor.is_none() {
        if let Some(asset) = world.resource::<Assets<JsonAsset>>().get(&handles.1) {
            let parsed = serde_json::from_str::<ProcessorDocument>(&asset.0)
                .map_err(|e| e.to_string())
                .and_then(|doc| {
                    if doc.schema == "moly-tutorial-processor/1"
                        && doc.coordinate_contract == "moly-rh-y-up-reflect-x-v1"
                    {
                        Ok(doc)
                    } else {
                        Err("tutorial processor schema/coordinate mismatch".into())
                    }
                });
            world.resource_mut::<TutorialAssets>().processor = Some(parsed);
        }
    }
    if world.resource::<TutorialAssets>().cutscenes.is_none() {
        if !world.resource::<MasterData>().is_resolved(CUTSCENES.key()) {
            return;
        }
        if let Some(result) = world.resource_mut::<MasterData>().take(&CUTSCENES) {
            world.resource_mut::<TutorialAssets>().cutscenes =
                Some(result.map_err(|e| e.to_string()));
        }
    }
}

fn actor(world: &mut World, unit: u32) -> Result<Entity, String> {
    crate::npc::gate_entries::find_npc(world, unit)
        .ok_or_else(|| format!("tutorial unit {unit} is absent"))
}
fn ready(world: &World, entity: Entity) -> bool {
    world
        .get::<NpcActions>(entity)
        .is_some_and(NpcActions::ready)
        && world
            .get::<crate::character::MotionDriver>(entity)
            .is_some()
}
fn player(world: &mut World) -> Result<Entity, String> {
    world
        .query_filtered::<Entity, With<crate::player::PlayerControlled>>()
        .single(world)
        .map_err(|_| "tutorial needs exactly one real player".into())
}
fn apply_pose(world: &mut World, entity: Entity, pose: &Pose) -> Result<(), String> {
    if !crate::npc::force_set_position(world, entity, pose.position) {
        return Err("tutorial actor navigation is not ready".into());
    }
    let rotation = Quat::from_array(pose.rotation);
    world
        .get_mut::<Transform>(entity)
        .ok_or("tutorial actor has no pose")?
        .rotation = rotation;
    if let Some(mut walk) = world.get_mut::<crate::npc::WalkState>(entity) {
        walk.0.forward = (rotation * Vec3::Z).to_array();
    }
    Ok(())
}
fn change_site_npc(world: &mut World, entity: Entity, target: i32) -> Result<(), String> {
    let name = *crate::npc::residency::SITE_TYPES.get(target as usize)
        .ok_or("tutorial target site is outside home/floors")?;
    let position = if target == 0 {
        let house = match crate::fixture_gimmick::house_door::find_house(world) {
            crate::fixture_gimmick::house_door::HouseLookup::Found(house) => house,
            _ => return Err("tutorial home fixture has not been bound".into()),
        };
        let pose = world.get::<GlobalTransform>(house.root).ok_or("home fixture world pose missing")?;
        let points = world.get_resource::<crate::fixture_attach::AttachPoints>()
            .ok_or("home fixture attach points loading")?.source_array(&house.package, pose)?;
        points.first().map_or(pose.translation().to_array(), |(_, point)| point.start.position)
    } else {
        let loaded = world.get_resource::<crate::site::SiteActive>()
            .and_then(|site| crate::npc::residency::site_type_value(&site.site_type));
        if loaded == Some(target) {
            let point = crate::room_door_port::inside_point(world).ok_or("target room door action point loading")?;
            world.get::<GlobalTransform>(point).ok_or("target room door pose loading")?.translation().to_array()
        } else {
            world.get_resource::<crate::npc::residency::SiteNavigation>()
                .and_then(|sites| sites.get(name)).and_then(|site| site.door)
                .ok_or("target room has no retained door action point")?.0.to_array()
        }
    };
    world.get_mut::<NpcActions>(entity).ok_or("tutorial actor state missing")?.site_type = name.into();
    if !crate::npc::force_set_position(world, entity, position) {
        return Err("tutorial actor movement not ready".into());
    }
    Ok(())
}

fn hide_npc(world: &mut World, run: &mut Run, unit: u32, milliseconds: i32) -> Result<bool, String> {
    let frame = world.get_resource::<bevy::diagnostic::FrameCount>().map_or(0, |f| u64::from(f.0));
    let dt = world.resource::<Time>().delta_secs();
    if run.hide.is_none() {
        let actor = actor(world, unit)?;
        run.hide = Some(HideRun { actor, delay: crate::tween_clock::Delay::new(milliseconds as f32 / 1000.0, frame), phase: 0 });
    }
    let hide = run.hide.as_mut().unwrap();
    if hide.phase == 0 {
        if !hide.delay.tick(frame, dt) { return Ok(false); }
        let at = world.get::<Transform>(hide.actor).ok_or("tutorial hide actor pose missing")?.translation;
        if !world.contains_resource::<crate::site_move::effects::EffectPools>() { return Err("effect pools are not installed".into()); }
        world.resource_scope(|world, mut pools: Mut<crate::site_move::effects::EffectPools>| {
            pools.emit(world, 12, Transform::from_translation(at))
        })?;
        hide.delay = crate::tween_clock::Delay::new(0.2, frame);
        hide.phase = 1;
    }
    if !hide.delay.tick(frame, dt) { return Ok(false); }
    let at = processor_data(world)?.initial_offstage;
    crate::npc::gate_entries::hide(world, hide.actor);
    if !crate::npc::force_set_position(world, hide.actor, at) { return Err("tutorial hide actor movement missing".into()); }
    run.hide = None;
    Ok(true)
}

fn processor_data(world: &World) -> Result<ProcessorDocument, String> {
    world
        .resource::<TutorialAssets>()
        .processor
        .as_ref()
        .ok_or("tutorial processor document is loading")?
        .clone()
}
fn screen_ready(world: &World, screen: MenuScreenType) -> bool {
    world
        .get_resource::<ScreenManager>()
        .is_some_and(|manager| {
            manager.current_screen() == Some(screen)
                && manager.current_state() == ScreenLayerState::Playing
        })
}
fn event(world: &mut World, value: serde_json::Value) {
    info!("[tutorial-processor] {value}");
    world.resource_mut::<TutorialStatus>().events.push(value);
}

fn read_input(world: &mut World) {
    let incoming = world
        .get_resource::<crate::server_panel_data::ClientNpcPanel>()
        .and_then(|panel| {
            panel
                .text
                .as_ref()
                .map(|text| (panel.revision, text.clone()))
        });
    let Some((revision, text)) = incoming else {
        return;
    };
    if world.resource::<Processor>().revision == Some(revision) {
        return;
    }
    world.resource_mut::<Processor>().revision = Some(revision);
    let input = (|| -> Result<Option<ProcessInput>, String> {
        let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        value
            .pointer("/controls/tutorial/process")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
            .transpose()
    })();
    match input {
        Ok(Some(input)) if !input.id.is_empty() => {
            let mut p = world.resource_mut::<Processor>();
            if p.seen.insert(input.id.clone()) {
                p.queue.push_back(input);
            }
        }
        Ok(_) => {}
        Err(reason) => world.resource_mut::<TutorialStatus>().error = Some(reason),
    }
}

fn tick_waits(world: &mut World) {
    let frame = world
        .get_resource::<bevy::diagnostic::FrameCount>()
        .map_or(0, |f| u64::from(f.0));
    let dt = world.resource::<Time>().delta_secs();
    let waits = std::mem::take(&mut world.resource_mut::<Processor>().waits);
    for mut wait in waits {
        let Some(current) = world.get::<NpcActions>(wait.actor).map(|a| a.current) else {
            event(
                world,
                serde_json::json!({"id":wait.run,"unit":wait.unit,"error":"waiting actor disappeared"}),
            );
            continue;
        };
        if current == NpcAction::Talk {
            wait.seen_talk = true;
        }
        if current != NpcAction::Talk && (!wait.sequential || wait.seen_talk) {
            wait.seen_not_talk = true;
        }
        let complete = (wait.seen_talk && wait.seen_not_talk)
            || wait
                .delay
                .as_mut()
                .is_some_and(|delay| delay.tick(frame, dt));
        if !complete {
            world.resource_mut::<Processor>().waits.push(wait);
            continue;
        }
        if wait.delay.is_some() {
            if let Err(reason) = lower::begin_freewalk(world, wait.actor, wait.id) {
                world.resource_mut::<TutorialStatus>().error = Some(reason);
                world.resource_mut::<Processor>().waits.push(wait);
                continue;
            }
        } else {
            let same = world
                .get::<lower::TutorialTalkData>(wait.actor)
                .is_some_and(|data| data.master.id == wait.id);
            if same {
                if wait.material>0 {
                    let Some(mut openers)=world.get_resource_mut::<crate::get_resource::GetResourceOpeners>() else {
                        world.resource_mut::<TutorialStatus>().error=Some("tutorial material notice owner is not installed".into());
                        world.resource_mut::<Processor>().waits.push(wait);continue;
                    };
                    openers.notice_collect_item("mysekai_material",wait.material,1,"MysekaiTutorialProcessor.WaitToWaitingTalk");
                }
                event(
                    world,
                    serde_json::json!({"id":wait.run,"unit":wait.unit,"talk":wait.id,"event":"onTalk","noticeCollectItem":(wait.material>0).then_some(wait.material),"amount":(wait.material>0).then_some(1)}),
                );
            }
        }
        event(
            world,
            serde_json::json!({"id":wait.run,"unit":wait.unit,"talk":wait.id,"event":"waitingReturned"}),
        );
    }
}

fn step(world: &mut World, run: &mut Run, op: &Op) -> Result<bool, String> {
    match op {
        Op::Screen(screen) => Ok(screen_ready(world, *screen)),
        Op::Find(unit) => {
            let entity = crate::npc::find_or_create_tutorial(world, *unit)?;
            Ok(ready(world, entity))
        }
        Op::Existing(unit) => {
            let entity = actor(world, *unit)?;
            Ok(ready(world, entity))
        }
        Op::EnableJoystick(enabled)=> {
            let actor=player(world)?;
            if *enabled {world.entity_mut(actor).remove::<lower::TutorialMovementHold>();}
            else {world.entity_mut(actor).insert(lower::TutorialMovementHold);}
            Ok(true)
        }
        Op::WaitingOverwrite(unit,id)=> {
            let actor=actor(world,*unit)?;
            let run_id=run.input.id.clone();
            let callback:lower::OverwriteTalk=std::sync::Arc::new(move |world,actor| {
                crate::npc::gate_entries::set_talk_enabled(world,actor,false);
                event(world,serde_json::json!({"id":run_id,"event":"SeqNext","section":24,"nextSeq":4,"executeProcess":true}));
            });
            lower::begin_waiting(world,actor,*id,Some(callback),true)?;
            Ok(true)
        }
        Op::Show(unit) => {
            let entity = actor(world, *unit)?;
            crate::npc::gate_entries::show(world, entity);
            Ok(true)
        }
        Op::Placement(name, recovery_only) => {
            if *recovery_only && !run.input.recovery_point {
                return Ok(true);
            }
            let rows = world
                .resource::<TutorialAssets>()
                .placements
                .as_ref()
                .ok_or("tutorial placements are loading")?
                .as_ref()
                .map_err(Clone::clone)?
                .placements
                .get(*name)
                .ok_or_else(|| format!("tutorial placement {name} is absent"))?
                .clone();
            let mut actors = Vec::new();
            for row in &rows {
                let entity = actor(world, row.unit_character_id)?;
                if !ready(world, entity) {
                    return Ok(false);
                }
                actors.push(entity);
            }
            for (entity, row) in actors.into_iter().zip(rows) {
                apply_pose(
                    world,
                    entity,
                    &Pose {
                        position: row.position,
                        rotation: row.rotation,
                    },
                )?;
                crate::npc::gate_entries::show(world, entity);
                crate::npc_objective::events::reset_face(world, entity);
                world.entity_mut(entity).insert(Visibility::Inherited);
            }
            Ok(true)
        }
        Op::ForceTalk(unit, id) => {
            if let Some((entity, id)) = run.forced {
                return match crate::player_talk::forced_tutorial_result(world, entity, id) {
                    Some(true) => {
                        run.forced = None;
                        Ok(true)
                    }
                    Some(false) => Err(format!("forced tutorial talk {id} was cancelled")),
                    None => Ok(false),
                };
            }
            if world.contains_resource::<crate::player_talk::PlayerTalkSession>()
                || world.contains_resource::<crate::talk::ActiveTalk>()
            {
                return Ok(false);
            }
            let entity = actor(world, *unit)?;
            crate::player_talk::force_play_tutorial(world, entity, *id)?;
            run.forced = Some((entity, *id));
            Ok(false)
        }
        Op::WaitTalk(unit, id, limit, material, blocks) => {
            let entity = actor(world, *unit)?;
            lower::begin_waiting(world, entity, *id, None, true)?;
            let current = world.get::<NpcActions>(entity).map(|a| a.current);
            let frame = world
                .get_resource::<bevy::diagnostic::FrameCount>()
                .map_or(0, |f| u64::from(f.0));
            world.resource_mut::<Processor>().waits.push(WaitingTalk {
                run: run.input.id.clone(),
                actor: entity,
                unit: *unit,
                id: *id,
                material: *material,
                seen_talk: current == Some(NpcAction::Talk),
                seen_not_talk: *limit > 0.0 && current != Some(NpcAction::Talk),
                sequential: *limit <= 0.0,
                delay: (*limit > 0.0).then(|| crate::tween_clock::Delay::new(limit.trunc(), frame)),
                blocks: *blocks,
            });
            Ok(true)
        }
        Op::Freewalk(unit, id, force) => {
            let entity = actor(world, *unit)?;
            if *force {
                world
                    .entity_mut(entity)
                    .insert(lower::TutorialForceOccurRandomMove(true));
            }
            lower::begin_freewalk(world, entity, *id)?;
            Ok(true)
        }
        Op::MissionChildren => {
            let tables = world
                .get_resource::<moly_masters::load::MasterTables>()
                .and_then(|t| t.mission.as_ref())
                .ok_or("mission masters are loading")?;
            let user = world
                .get_resource::<crate::userdata::mission::ClientMission>()
                .ok_or("mission user data is absent")?;
            Ok(crate::mission::model::is_child_mission_all_achieved(
                tables,
                &user.state(),
            ))
        }
        Op::IntroPrepare => {
            let data = processor_data(world)?;
            let entity = actor(world, 21)?;
            if !crate::npc::force_set_position(world, entity, data.initial_offstage) {
                return Ok(false);
            }
            let player = player(world)?;
            let mut pose = world
                .get_mut::<Transform>(player)
                .ok_or("player pose is absent")?;
            pose.translation = Vec3::from_array(data.initial_player.position);
            pose.rotation = Quat::from_array(data.initial_player.rotation);
            drop(pose);
            world.entity_mut(player).insert((
                Visibility::Hidden,
                lower::TutorialMovementHold,
                crate::npc::MotionPhase::Dwelling { remaining: None },
            ));
            if let Some(mut input) = world.get_mut::<crate::player::PlayerInput>(player) {
                input.active = false;
                input.direction = Vec3::ZERO;
            }
            world.resource_mut::<TutorialInput>().movement_blocked = true;
            Ok(true)
        }
        Op::IntroCutscene => {
            if world.resource::<Processor>().cutscene_started {
                return match world.resource_mut::<Processor>().returned_cutscene.take() {
                    Some(Ok(())) => Ok(true),
                    Some(Err(e)) => Err(e),
                    None => Ok(false),
                };
            }
            let timeline = world
                .resource::<TutorialAssets>()
                .cutscenes
                .as_ref()
                .ok_or("tutorial cut-scene masters are loading")?
                .as_ref()
                .map_err(Clone::clone)?
                .get(&20)
                .ok_or("tutorial cut-scene 20 is absent")?
                .clone();
            let root = crate::cutscene::scene_root(world);
            crate::cutscene::play_async(
                world,
                crate::cutscene::CastPlay {
                    caller: crate::cutscene::CastCaller::Tutorial,
                    units: vec![21],
                    cut_scene_id: 20,
                    timeline,
                    start: root,
                    use_already_exist_character: true,
                    hide_player: false,
                    show_ui: true,
                },
            )?;
            world.resource_mut::<Processor>().cutscene_started = true;
            world.resource_mut::<TutorialInput>().taps_blocked = true;
            Ok(false)
        }
        Op::IntroAfterTalk => {
            let data = processor_data(world)?;
            let entity = player(world)?;
            world
                .get_mut::<Transform>(entity)
                .ok_or("player pose is absent")?
                .rotation = Quat::from_array(data.after_talk_player_rotation);
            event(
                world,
                serde_json::json!({"id":run.input.id,"event":"CreateFixtureNavigateIcon","fixtureId":2}),
            );
            Ok(true)
        }
        Op::ChangeSite(unit, target) => { let entity = actor(world, *unit)?; change_site_npc(world, entity, *target)?; Ok(true) },
        Op::HideNpc(unit, delay) => hide_npc(world, run, *unit, *delay),
        Op::AwaitTalks => Ok(!world
            .resource::<Processor>()
            .waits
            .iter()
            .any(|wait| wait.run == run.input.id && wait.blocks)),
        Op::Boundary(boundary) => {
            if world
                .resource::<Processor>()
                .waits
                .iter()
                .any(|wait| wait.run == run.input.id && wait.blocks)
            {
                return Ok(false);
            }
            world.resource_mut::<TutorialStatus>().boundary = Some((*boundary).into());
            world.resource_mut::<TutorialStatus>().state = "awaitingProducer".into();
            Ok(true)
        }
    }
}

pub(crate) fn start_fade_in_callback(world: &mut World, _cast: &crate::cutscene::Cast) {
    crate::screen_fade::fade_in(world, 0.0, 1.0, "Tutorial Execute1_1 start fade");
}
pub(crate) fn end_fade_out_callback(world: &mut World, _cast: &crate::cutscene::Cast) {
    let result: Result<(), String> = (|| {
        let data = processor_data(world)?;
        let entity = actor(world, 21)?;
        apply_pose(world, entity, &data.after_intro_npc)
    })();
    if let Err(reason) = result {
        world.resource_mut::<TutorialStatus>().error = Some(reason);
    }
    world.resource_mut::<Processor>().input_lock_elapsed = Some(0.0);
}
pub(crate) fn cut_scene_returned(world: &mut World, cast: &crate::cutscene::Cast) {
    let outcome = cast
        .outcome
        .clone()
        .unwrap_or_else(|| Err("tutorial cut-scene returned without an outcome".into()));
    world.resource_mut::<Processor>().returned_cutscene = Some(outcome);
}

fn update(world: &mut World) {
    read_assets(world);
    read_input(world);
    let active = world
        .get_resource::<crate::server_panel::ServerPanel>()
        .is_some_and(|p| p.is_tutorial());
    if !active {
        world.resource_mut::<Processor>().run = None;
        world.resource_mut::<Processor>().waits.clear();
        world.resource_mut::<Processor>().input_lock_elapsed = None;
        if world.resource::<TutorialInput>().movement_blocked {
            let held: Vec<Entity> = world
                .query_filtered::<Entity, With<lower::TutorialMovementHold>>()
                .iter(world)
                .collect();
            for entity in held {
                world
                    .entity_mut(entity)
                    .remove::<lower::TutorialMovementHold>();
            }
        }
        *world.resource_mut::<TutorialInput>() = TutorialInput::default();
        return;
    }
    if let Some(elapsed) = world.resource::<Processor>().input_lock_elapsed {
        let talk = world
            .get_resource::<crate::talk_window::TalkWindowState>()
            .is_some_and(|window| window.transcript().2);
        if elapsed >= 2.0 || talk {
            world.resource_mut::<Processor>().input_lock_elapsed = None;
            world.resource_mut::<TutorialInput>().taps_blocked = false;
        } else {
            world.resource_mut::<Processor>().input_lock_elapsed =
                Some(elapsed + world.resource::<Time>().delta_secs());
            world.resource_mut::<TutorialInput>().movement_blocked = true;
        }
    }
    tick_waits(world);
    if world.resource::<Processor>().run.is_none() {
        if let Some(input) = world.resource_mut::<Processor>().queue.pop_front() {
            let ops = plan(&input);
            let mut status = world.resource_mut::<TutorialStatus>();
            status.id = Some(input.id.clone());
            status.episode = Some(input.episode.clone());
            status.phase = Some(input.phase.clone());
            status.boundary = None;
            status.error = None;
            status.state = "running".into();
            drop(status);
            match ops {
                Ok(ops) => {
                    if input.episode == "1_1" {
                        crate::screen_fade::fade_out(world, 0.0, 0.0, "Tutorial Execute1_1");
                    }
                    world.resource_mut::<Processor>().cutscene_started = false;
                    world.resource_mut::<Processor>().run = Some(Run {
                        input,
                        ops,
                        forced: None,
                        hide: None,
                    });
                }
                Err(reason) => world.resource_mut::<TutorialStatus>().error = Some(reason),
            }
        }
    }
    let Some(mut run) = world.resource_mut::<Processor>().run.take() else {
        return;
    };
    // Several synchronous calls share the native async continuation. Stop at
    // the first real await; the queue itself bounds work, not an invented timer.
    while let Some(op) = run.ops.front().cloned() {
        let label = match &op {
            Op::Screen(_) => "WaitReadyScreenLayer",
            Op::Find(_) => "FindOrCreateNPC",
            Op::Existing(_) => "FindNPC",
            Op::Placement(..) => "AttachCharacterPlacement",
            Op::ForceTalk(..) => "ForcePlayTutorialTalk",
            Op::WaitTalk(..) => "WaitToWaitingTalk",
            Op::Freewalk(..) => "ForceUpdateFreeWalkTutorialObjective",
            Op::Show(_) => "Show",
            Op::EnableJoystick(_)=>"EnableJoyStick",
            Op::WaitingOverwrite(..)=>"WaitToWaitingOverwriteTalk",
            Op::MissionChildren => "IsChildMissionAllAchieved",
            Op::IntroPrepare => "intro poses",
            Op::IntroCutscene => "PlayTutorialCutSceneAsync",
            Op::IntroAfterTalk => "after introductory talk",
            Op::ChangeSite(..) => "ChangeSiteNPC",
            Op::HideNpc(..) => "HideNPC",
            Op::AwaitTalks => "await all waiting talks",
            Op::Boundary(_) => "producer boundary",
        };
        world.resource_mut::<TutorialStatus>().operation = label.into();
        match step(world, &mut run, &op) {
            Ok(true) => {
                run.ops.pop_front();
                world.resource_mut::<TutorialStatus>().error = None;
            }
            Ok(false) => break,
            Err(reason) => {
                let mut status = world.resource_mut::<TutorialStatus>();
                if status.error.as_ref() != Some(&reason) {
                    warn!("[tutorial-processor] {}: {reason}", run.input.id);
                }
                status.error = Some(reason);
                break;
            }
        }
    }
    if !run.ops.is_empty() {
        world.resource_mut::<Processor>().run = Some(run);
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<TutorialAssets>()
        .init_resource::<Processor>()
        .init_resource::<TutorialInput>()
        .init_resource::<TutorialStatus>()
        .add_systems(Startup, load)
        .add_systems(
            Update,
            update
                .after(crate::ui_layers::advance)
                .after(crate::cutscene::advance),
        );
}
