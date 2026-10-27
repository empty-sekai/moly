//! The authored communication motion/face loop. This interpreter owns only
//! presentation while a shared NPC communication state is active.

use std::{collections::{HashMap, HashSet}, time::Duration};
use bevy::{animation::{graph::{AnimationGraph, AnimationNodeIndex}, transition::AnimationTransitions, AnimationPlayer}, asset::LoadState, gltf::Gltf, prelude::*};
use moly_assets::json::JsonAsset;
use moly_law::alone_action::{self as law, Step};
use crate::{alone_action_runtime::{self as shared, FacialTables, Lcg}, character::{MotionDriver, MotionLibrary}, character_material::{CharacterMaterial, ToonMaterials}, npc::CharacterUnitId};

#[derive(Component, Clone)]
pub struct CommunicationPlayback { pub epoch: u64, pub participants: Vec<u32> }

#[derive(Resource)]
struct Pending(Handle<JsonAsset>);

struct Branch { participant: Option<u32>, below: u32, steps: Vec<Step> }
struct Program { random: bool, branches: Vec<Branch>, tail: Vec<Step> }
#[derive(Resource, Default)]
struct Programs(HashMap<u32, Program>);

pub(crate) fn ready(world: &World) -> bool { world.get_resource::<Programs>().is_some_and(|p| !p.0.is_empty()) }

struct Running {
    epoch: u64,
    rng: Lcg,
    steps: Vec<Step>,
    cursor: usize,
    wait: Option<(f32, f32)>,
    nodes: HashMap<String, AnimationNodeIndex>,
    start: Option<(String, f32, f32)>,
}

#[derive(Resource, Default)]
struct Runtimes(HashMap<Entity, Running>);

pub fn install(app: &mut App) {
    app.init_resource::<Runtimes>()
        .add_systems(Startup, load)
        .add_systems(Update, parse)
        .add_systems(Update, advance.after(shared::advance).after(crate::player_talk::consume_trigger));
}

fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(Pending(server.load("moly://communication-actions.json")));
}

fn parse(mut commands: Commands, server: Res<AssetServer>, pending: Option<Res<Pending>>, assets: Res<Assets<JsonAsset>>) {
    let Some(pending) = pending else { return; };
    if let LoadState::Failed(error) = server.load_state(&pending.0) {
        warn!("[communication] scripts unavailable in this resource version: {error:?}");
        commands.insert_resource(Programs::default()); commands.remove_resource::<Pending>(); return;
    }
    let Some(asset) = assets.get(&pending.0) else { return; };
    let value: serde_json::Value = serde_json::from_str(&asset.0).expect("communication document must be JSON");
    assert_eq!(value["version"].as_u64(), Some(1), "unsupported communication program version");
    let units = value["units"].as_object().expect("communication document requires units");
    let mut programs = HashMap::new();
    for (key, row) in units {
        let unit: u32 = key.parse().expect("communication unit must be numeric");
        let program = &row["program"];
        assert_eq!(program["kind"].as_str(), Some("whileCommunication"));
        let random = !program["random"].is_null();
        if random { assert_eq!(program["random"]["min"].as_u64(), Some(0)); assert_eq!(program["random"]["max"].as_u64(), Some(99)); }
        let parse_steps = |value: &serde_json::Value| {
            let steps = value.as_array().expect("communication program requires steps");
            for step in steps {
                assert!(step.get("who").is_none_or(|v| v.as_u64() == Some(u64::from(unit))), "communication step targets another actor");
            }
            // The authored wrapper supplies 1 when the speed argument is
            // absent. The source field remains nullable in the extraction.
            let normalized: Vec<serde_json::Value> = steps.iter().cloned().map(|mut step| {
                if step["op"] == "animation" && step["speed"].is_null() { step["speed"] = 1.0.into(); }
                step
            }).collect();
            shared::parse_steps(&normalized, unit, "communication")
        };
        let branches = program["branches"].as_array().expect("communication program requires branches").iter().map(|branch| {
            let condition = &branch["condition"];
            Branch { participant: condition["participant"].as_u64().map(|v| v as u32),
                below: condition["randomBelow"].as_u64().expect("communication branch needs threshold") as u32,
                steps: parse_steps(&branch["steps"]) }
        }).collect();
        let tail = parse_steps(&program["steps"]);
        assert!(tail.iter().any(|s| matches!(s, Step::Wait { seconds, .. } if *seconds > 0.0)), "communication loop must yield");
        programs.insert(unit, Program { random, branches, tail });
    }
    commands.insert_resource(Programs(programs)); commands.remove_resource::<Pending>();
}

/// Source WaitTime compares a float accumulator directly, rather than using
/// the millisecond delay of the Rest interpreter. A wait yields before its
/// first delta; its overshoot is not carried into the next authored wait.
fn next_events(rt: &mut Running, program: &Program, participants: &[u32], dt: f32) -> Vec<Step> {
    let mut out = Vec::new();
    if let Some((duration, elapsed)) = rt.wait.as_mut() {
        *elapsed += dt;
        if *elapsed < *duration { return out; }
        rt.wait = None;
    }
    loop {
        if rt.cursor == rt.steps.len() {
            rt.steps.clear(); rt.cursor = 0;
            let drawn = program.random.then(|| rt.rng.percent());
            if let Some(branch) = program.branches.iter().find(|branch| branch.participant.is_none_or(|p| participants.contains(&p)) && drawn.is_none_or(|draw| draw < branch.below)) {
                rt.steps.extend(branch.steps.iter().cloned());
            }
            rt.steps.extend(program.tail.iter().cloned());
        }
        let step = rt.steps[rt.cursor].clone(); rt.cursor += 1;
        match step {
            Step::Wait { seconds, .. } if seconds > 0.0 => { rt.wait = Some((seconds, 0.0)); break; }
            Step::Wait { .. } => {}
            step => out.push(step),
        }
    }
    out
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn advance(
    clock: Res<crate::npc_clock::NpcClock>,
    programs: Option<Res<Programs>>, mut runtime: ResMut<Runtimes>,
    gate: Res<shared::AloneExecutionGate>, tables: Option<Res<FacialTables>>,
    library: Res<MotionLibrary>, libraries: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>, mut materials: ResMut<Assets<CharacterMaterial>>,
    mut emotes: ResMut<crate::emoticon::RestEmoteRequests>,
    mut actors: Query<(Entity, &CharacterUnitId, Option<&CommunicationPlayback>, Option<&crate::talk::TalkHold>, &ToonMaterials, &mut MotionDriver)>,
    mut players: Query<&mut AnimationPlayer>, mut transitions: Query<&mut AnimationTransitions>,
) {
    let (Some(programs), Some(tables), Some(lib)) = (programs, tables, libraries.get(&library.gltf)) else { return; };
    let mut retained = HashSet::new();
    for (entity, unit, playback, talk, toon, mut driver) in &mut actors {
        let playback = playback.filter(|_| talk.is_none());
        let Some(playback) = playback else {
            if runtime.0.remove(&entity).is_some() {
                emotes.hide(entity); driver.alone_holds = false;
                if talk.is_none() { shared::play_idle(&mut driver, &mut players, &mut transitions, unit.0); }
            }
            continue;
        };
        let Some(program) = programs.0.get(&unit.0) else { continue; };
        retained.insert(entity);
        let rt = runtime.0.entry(entity).or_insert_with(|| Running { epoch: playback.epoch, rng: Lcg::seeded(unit.0), steps: Vec::new(), cursor: 0, wait: None, nodes: HashMap::new(), start: None });
        if rt.epoch != playback.epoch { rt.epoch = playback.epoch; rt.steps.clear(); rt.cursor = 0; rt.wait = None; rt.start = None; }
        driver.alone_holds = true;
        let events = next_events(rt, program, &playback.participants, clock.delta());
        if gate.cutscene_active { continue; }
        for step in events {
            match step {
                Step::ChangeEye { pattern, .. } => if let Some(eye) = toon.slot_handle("eye") { shared::apply_eye_pattern(&mut materials, eye, tables.eye_pattern(&pattern)); },
                Step::ChangeMouth { pattern, .. } => if let Some(mouth) = toon.slot_handle("mouth") { shared::apply_mouth_pattern(&mut materials, mouth, tables.lip_pattern(&pattern).unwrap_or_default()); },
                Step::ChangeAnimation { motion, phase, speed, blend, play_end_motion, .. } => {
                    assert!(!play_end_motion, "communication end-segment chaining needs an explicit authored sequence");
                    let clip = format!("{motion}_{}", phase.map(shared::suffix_letter).unwrap_or("S"));
                    let node = shared::node_for(&mut rt.nodes, &mut graphs, lib, &driver.graph, &clip, unit.0);
                    let mut player = players.get_mut(driver.player).expect("communication actor player is installed");
                    let mut transition = transitions.get_mut(driver.player).expect("communication actor transitions are installed");
                    let speed = law::effective_speed(speed);
                    let playing = transition.play(&mut player, node, Duration::from_secs_f32(blend)); playing.set_speed(speed);
                    if phase == Some(law::SegmentSuffix::L) { playing.repeat(); }
                    rt.start = phase.is_none().then_some((motion, speed, blend)); driver.playing = None;
                }
                Step::ShowEmoticon { name, host_not_play_se, .. } => emotes.show(entity, name, host_not_play_se),
                Step::HideEmoticon { .. } => emotes.hide(entity),
                Step::Wait { .. } => unreachable!(),
            }
        }
        if let Some((motion, speed, blend)) = rt.start.clone() {
            let done = rt.nodes.get(&format!("{motion}_S")).is_some_and(|node| players.get(driver.player).is_ok_and(|player| player.playing_animations().any(|(id, state)| id == node && state.is_finished())));
            if done {
                let node = shared::node_for(&mut rt.nodes, &mut graphs, lib, &driver.graph, &format!("{motion}_L"), unit.0);
                let mut player = players.get_mut(driver.player).unwrap(); let mut transition = transitions.get_mut(driver.player).unwrap();
                transition.play(&mut player, node, Duration::from_secs_f32(blend)).set_speed(speed).repeat(); rt.start = None;
            }
        }
    }
    runtime.0.retain(|entity, _| retained.contains(entity));
}
