//! Instance-owned fixture controller events and the player's switch interval.
//!
//! Event-only motions are real clips: an empty pose curve set does not make
//! their AnimationEvents empty. Controller references select the motion; its
//! authored event selects the output, never a guess based on the clip name.
//! A bounded single-Transform Euler lane also preserves authored coefficients.
//! Non-interrupting source Trigger bits wait for the next eligible evaluation.
//! The current SD player's switch uses a named, simple source-motion adaptation.
//! General controllers and nonzero interruption sources remain separate gaps.

mod rotation;
pub(crate) mod catalog_stream;
pub(crate) mod session;

use std::{collections::HashMap, sync::Arc};

use bevy::{asset::LoadState, prelude::*};
use moly_assets::json::JsonAsset;
use serde_json::Value;

use crate::{
    audio::{SeClass, SeRequest, SeRequests},
    fixture::FixtureViewInstance,
    fixture_activity_data::FixtureActivityTables,
    fixture_activity_state::{FixtureActivityIdentity, FixtureActivityOwner, FixtureTarget},
    fixture_activity_timeline::SourceAssetId,
    fixture_emission::{FixtureEmission, FixtureEmissionOverride},
    npc::MotionPhase,
    player::{DashMode, PlayerControlled, PlayerInput},
    player_avatar::{AvatarDriver, switch_gesture},
    player_fixture_action::{PlayerFixtureControlOwner, PlayerFixtureHeld},
    player_state::{PlayerActionState, PlayerAvatarStates},
    site::GroundEpoch,
};

const SWITCH_SOUND_TIME: f32 = 0.5;
const SWITCH_END_TIME: f32 = 1.0;

#[derive(Resource)]
pub(crate) struct CatalogLoad(Handle<JsonAsset>);

#[derive(Clone, Debug)]
enum Effect {
    Emission(FixtureEmissionOverride),
    PlaySe(String),
    FinishOneShot,
    /// An audited source receiver with no serialized or static runtime supply.
    EmptyActivate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Trigger { On, Off }

impl Trigger {
    fn name(self) -> &'static str {
        match self { Self::On => "On", Self::Off => "Off" }
    }
}

#[derive(Default)]
struct PendingTriggers { on: bool, off: bool }

impl PendingTriggers {
    fn set(&mut self, trigger: Trigger) {
        match trigger { Trigger::On => self.on = true, Trigger::Off => self.off = true }
    }

    fn select(&self, order: [Trigger; 2]) -> Option<Trigger> {
        order.into_iter().find(|trigger| match trigger {
            Trigger::On => self.on,
            Trigger::Off => self.off,
        })
    }

    fn consume(&mut self, trigger: Trigger) {
        match trigger { Trigger::On => self.on = false, Trigger::Off => self.off = false }
    }
}

#[derive(Clone)]
struct ClipEvent {
    time: f64,
    function: String,
    effect: Effect,
}

#[derive(Clone)]
struct Program {
    clip: SourceAssetId,
    duration: f64,
    initial_time: f64,
    speed: f64,
    events: Vec<ClipEvent>,
    transition_seconds: f64,
    rotation: Option<rotation::Program>,
}

struct Definition {
    view_game_object: i64,
    source_file: String,
    view_component: i64,
    animator_component: i64,
    trigger_order: Option<[Trigger; 2]>,
    on: Arc<Program>,
    off: Arc<Program>,
}

#[derive(Resource, Default)]
pub(crate) struct Catalog(HashMap<String, Result<Arc<Definition>, String>>);

struct Playback {
    program: Arc<Program>,
    elapsed: f64,
    next_event: usize,
    outputs: Vec<Entity>,
    transition_elapsed: f64,
    previous: Option<PreviousPlayback>,
}

struct PreviousPlayback {
    program: Arc<Program>,
    elapsed: f64,
    next_event: usize,
}

impl Playback {
    fn new(program: Arc<Program>, outputs: Vec<Entity>, previous: Option<Playback>) -> Self {
        let elapsed = program.initial_time;
        let next_event = program.events.partition_point(|event| event.time < elapsed);
        let previous = if program.rotation.is_some() {
            previous.map(|previous| PreviousPlayback {
                program: previous.program,
                elapsed: previous.elapsed,
                next_event: previous.next_event,
            })
        } else {
            None
        };
        Self { program, elapsed, next_event, outputs, transition_elapsed: 0.0, previous }
    }

    fn transitioning(&self) -> bool {
        self.program.rotation.is_some()
            && self.transition_elapsed < self.program.transition_seconds
    }
}

struct Instance {
    on: bool,
    definition: Arc<Definition>,
    playback: Option<Playback>,
    rotation: Option<rotation::Binding>,
    pending: PendingTriggers,
    outputs: Vec<Entity>,
}

struct PlayerSwitch {
    owner: FixtureActivityOwner,
    gesture: switch_gesture::Lease,
    target: FixtureTarget,
    view: Entity,
    site_epoch: u64,
    elapsed: f32,
    sound_checked: bool,
    light: bool,
    before_dash: bool,
}

#[derive(Resource, Default)]
pub(crate) struct Gimmicks {
    instances: HashMap<FixtureTarget, Instance>,
    player: Option<PlayerSwitch>,
    leases: HashMap<FixtureTarget, session::Lease>,
}

pub(crate) fn load(
    mut commands: Commands, server: Res<AssetServer>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    if stage.is_some() {
        catalog_stream::load(&mut commands, &server);
        return;
    }
    commands.insert_resource(CatalogLoad(
        server.load("moly://fixture-gimmick/gimmicks.json"),
    ));
}

pub(crate) fn parse(
    mut commands: Commands,
    server: Res<AssetServer>,
    assets: Res<Assets<JsonAsset>>,
    pending: Option<Res<CatalogLoad>>,
) {
    let Some(pending) = pending else {
        return;
    };
    if let LoadState::Failed(error) = server.load_state(&pending.0) {
        warn!("[fixture-gimmick] controller metadata failed to load: {error}");
        commands.remove_resource::<CatalogLoad>();
        return;
    }
    let Some(asset) = assets.get(&pending.0) else {
        return;
    };
    let document: Value = serde_json::from_str(&asset.0).expect("fixture controller metadata JSON");
    assert_eq!(
        document["version"].as_u64(),
        Some(1),
        "fixture controller metadata version"
    );
    let mut catalog = Catalog::default();
    for package in document["packages"]
        .as_array()
        .expect("fixture controller packages")
    {
        let name = package["name"]
            .as_str()
            .expect("fixture controller package name");
        let definition = definition(package).map(Arc::new);
        if let Err(reason) = &definition {
            warn!("[fixture-gimmick] {name} preparation: {reason}");
        }
        assert!(
            catalog.0.insert(name.to_owned(), definition).is_none(),
            "duplicate fixture controller package"
        );
    }
    info!("[fixture-gimmick] source controller catalog loaded");
    commands.insert_resource(catalog);
    commands.remove_resource::<CatalogLoad>();
}

fn array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], String> {
    value[field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("missing array {field}"))
}

fn one<'a>(items: &'a [Value], label: &str) -> Result<&'a Value, String> {
    match items {
        [item] => Ok(item),
        _ => Err(format!("{label} is not a single source object")),
    }
}

fn identity(value: &Value) -> Result<SourceAssetId, String> {
    let file = value["file"]
        .as_str()
        .ok_or("source identity has no file")?;
    let id = value["pathId"]
        .as_str()
        .ok_or("source identity has no string pathId")?;
    id.parse::<i64>()
        .map_err(|_| "source identity pathId is not i64")?;
    Ok(SourceAssetId {
        file: file.to_owned(),
        path_id: id.to_owned(),
    })
}

fn referenced<'a>(items: &'a [Value], reference: &Value) -> Result<&'a Value, String> {
    let target = identity(reference)?;
    let found: Vec<_> = items
        .iter()
        .filter(|item| identity(&item["source"]).ok().as_ref() == Some(&target))
        .collect();
    match found.as_slice() {
        [item] => Ok(item),
        _ => Err("source reference is missing or ambiguous".into()),
    }
}

fn number(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| format!("missing finite {key}"))
}

fn definition(package: &Value) -> Result<Definition, String> {
    let view = one(array(package, "fixtureViews")?, "FixtureView")?;
    let view_id = identity(&view["gameObject"]["source"])?;
    let animator = referenced(array(package, "animators")?, &view["animator"]["source"])?;
    if identity(&animator["gameObject"])? != view_id {
        return Err("animation event receiver is not the FixtureView object".into());
    }
    let controller = referenced(
        array(package, "controllers")?,
        &animator["controller"]["source"],
    )?;
    if controller["motionMapping"]["status"].as_str() != Some("resolved")
        || controller["motionMapping"]["kind"].as_str() != Some("clip-binding-array-index")
    {
        return Err("source controller motion-index mapping is not resolved".into());
    }
    let layer = one(array(controller, "layers")?, "controller layer")?;
    let machine_index = layer["data"]["m_StateMachineIndex"]
        .as_u64()
        .ok_or("missing state machine index")?;
    let machines = array(controller, "stateMachines")?;
    let machine = machines
        .iter()
        .find(|machine| machine["index"].as_u64() == Some(machine_index))
        .ok_or("source state machine absent")?;
    let initial = machine["defaultState"]
        .as_u64()
        .ok_or("missing default state")?;
    let state = array(machine, "states")?
        .iter()
        .find(|state| state["index"].as_u64() == Some(initial))
        .ok_or("default state absent")?;
    if !array(state, "motions")?.is_empty() {
        return Err("authored initial motion needs a controller lifecycle consumer".into());
    }
    let animator_id = identity(&animator["source"])?;
    let view_component = identity(&view["source"])?;
    let empty_activate = audited_empty_activation(package, view);
    let on = Arc::new(program(package, machine, "On", &animator_id, empty_activate)?);
    let off = Arc::new(program(package, machine, "Off", &animator_id, empty_activate)?);
    if on.rotation.is_some()
        && (view_component.file != view_id.file || animator_id.file != view_id.file)
    {
        return Err("Euler receiver components cross serialized-file identity domains".into());
    }
    if on.rotation.as_ref().map(|rotation| &rotation.target)
        != off.rotation.as_ref().map(|rotation| &rotation.target)
    {
        return Err("On/Off Euler programs do not own the same source Transform".into());
    }
    let trigger_order = if on.rotation.is_some() {
        Some(euler_trigger_order(controller, machine)?)
    } else { None };
    Ok(Definition {
        view_game_object: view_id
            .path_id
            .parse()
            .map_err(|_| "invalid FixtureView GameObject")?,
        source_file: view_id.file,
        view_component: view_component.path_id.parse().map_err(|_| "invalid FixtureView component")?,
        animator_component: animator_id.path_id.parse().map_err(|_| "invalid Animator component")?,
        trigger_order,
        on,
        off,
    })
}

fn euler_trigger_order(controller: &Value, machine: &Value) -> Result<[Trigger; 2], String> {
    let [first, second] = array(machine, "anyStateTransitions")? else {
        return Err("Euler trigger lane requires exactly the authored On/Off AnyState pair".into());
    };
    let parameters = array(controller, "parameters")?;
    let defaults = array(&controller["defaultValues"], "m_BoolValues")?;
    let read = |transition: &Value| -> Result<(Trigger, usize), String> {
        let condition = one(array(transition, "conditions")?, "Euler trigger condition")?;
        let trigger = match condition["parameterName"].as_str() {
            Some("On") => Trigger::On,
            Some("Off") => Trigger::Off,
            _ => return Err("Euler AnyState condition is not an On/Off trigger".into()),
        };
        if condition["mode"].as_u64() != Some(1) {
            return Err("Euler AnyState condition is not Trigger mode If".into());
        }
        let id = condition["parameterId"].as_u64().ok_or("Euler trigger condition has no parameter id")?;
        let rows: Vec<_> = parameters.iter().filter(|parameter| parameter["id"].as_u64() == Some(id)).collect();
        let [parameter] = rows.as_slice() else {
            return Err("Euler trigger parameter is missing or ambiguous".into());
        };
        if parameter["type"].as_u64() != Some(9) || parameter["name"].as_str() != Some(trigger.name()) {
            return Err("Euler condition does not resolve to its source Trigger parameter".into());
        }
        let index = parameter["index"].as_u64().and_then(|index| usize::try_from(index).ok())
            .ok_or("Euler trigger has no source bool slot")?;
        if defaults.get(index).and_then(Value::as_bool) != Some(false) {
            return Err("Euler trigger lane requires authored false initial trigger slots".into());
        }
        Ok((trigger, index))
    };
    let a = read(first)?;
    let b = read(second)?;
    if a.0 == b.0 || a.1 == b.1 {
        return Err("Euler On/Off triggers do not have distinct source bool slots".into());
    }
    // Keep array order, not names, FIFO arrival order, or the latest request.
    Ok([a.0, b.0])
}

// This is deliberately not an "empty array => no-op" rule for all fixtures.
// CN 6.0.0's refrigerator receiver was traced through constructor, Initialize,
// Setup/SetupForHome and AttachComponents. Its package has no ParticleSystem;
// the only static field writer is SetFixtureActivateEffect, with no direct
// caller in any executable section of the game's native binary. Hot IFix overrides are excluded.
fn audited_empty_activation(package: &Value, view: &Value) -> bool {
    package["name"].as_str() == Some("mysekai__fixture__mdl_ext0002_fixture_fridge1")
        && view["source"]["file"].as_str() == Some("CAB-5cfb2fafb25d7e6d5edd3905b13caa87")
        && view["source"]["pathId"].as_str() == Some("-4218056244832114724")
        && view["fields"].get("_fixtureActivateEffect").is_some_and(|effects| {
            effects.is_null() || effects.as_array().is_some_and(Vec::is_empty)
        })
}

fn program(
    package: &Value, machine: &Value, trigger: &str,
    animator: &SourceAssetId, empty_activate: bool,
) -> Result<Program, String> {
    let transitions: Vec<_> = array(machine, "anyStateTransitions")?
        .iter()
        .filter(|transition| {
            transition["conditions"]
                .as_array()
                .is_some_and(|conditions| {
                    conditions.len() == 1
                        && conditions[0]["mode"].as_u64() == Some(1)
                        && conditions[0]["parameterName"].as_str() == Some(trigger)
                })
        })
        .collect();
    let [transition] = transitions.as_slice() else {
        return Err(format!("trigger {trigger} has no unique source transition"));
    };
    if transition["hasExitTime"].as_bool() != Some(false) {
        return Err("exit-time transition needs a general controller consumer".into());
    }
    let offset = number(transition, "offset")?;
    let destination = transition["destinationState"]
        .as_u64()
        .ok_or("missing destination state")?;
    let state = array(machine, "states")?
        .iter()
        .find(|state| state["index"].as_u64() == Some(destination))
        .ok_or("destination state absent")?;
    // With neither conditions nor exit time, a source transition is ineligible,
    // not an immediate jump to Exit. Other outgoing paths need the full runner.
    for outgoing in array(state, "transitions")? {
        if !array(outgoing, "conditions")?.is_empty()
            || outgoing["hasExitTime"].as_bool() != Some(false)
        {
            return Err("eligible outgoing state transition needs a controller consumer".into());
        }
    }
    let motion = one(array(state, "motions")?, "state motion")?;
    let clip = referenced(array(package, "clips")?, &motion["clip"]["source"])?;
    if !array(clip, "pptrCurves")?.is_empty() {
        return Err("this controller needs object-reference animation curves".into());
    }
    let rotation = rotation::program(clip, animator)?;
    let transition_seconds = if rotation.is_some() {
        if transition["raw"]["m_InterruptionSource"].as_u64() != Some(0)
            || transition["canTransitionToSelf"].as_bool() != Some(true)
        {
            return Err("Euler trigger lane requires source interruptionSource=0 and self transitions".into());
        }
        let seconds = number(transition, "duration")?;
        if transition["hasFixedDuration"].as_bool() != Some(true) || seconds < 0.0 {
            return Err("Euler playback requires a nonnegative fixed-duration transition".into());
        }
        if state["raw"]["m_CycleOffset"].as_f64() != Some(0.0)
            || state["raw"]["m_SpeedParamID"].as_u64() != Some(0)
            || state["raw"]["m_CycleOffsetParamID"].as_u64() != Some(0)
            || state["raw"]["m_TimeParamID"].as_u64() != Some(0)
            || state["raw"]["m_Mirror"].as_bool() != Some(false)
        {
            return Err("Euler state has unsupported dynamic time or mirror inputs".into());
        }
        seconds
    } else {
        0.0
    };
    if clip["loopTime"].as_bool() != Some(false) {
        return Err("event-only loop motion needs repeated-event scheduling".into());
    }
    let start = number(clip, "startTime")?;
    let duration = number(clip, "duration")?;
    let speed = number(state, "speed")?;
    if start != 0.0 || duration < 0.0 || speed <= 0.0 {
        return Err("unsupported event clip time domain".into());
    }
    if !(0.0..=1.0).contains(&offset) {
        return Err("event clip transition offset is outside its normalized time domain".into());
    }
    // The controller initializes normalized state time, then multiplies by
    // the clip length in single precision before passing clip seconds onward.
    // Incoming time advances during blending; the blend is not a pre-delay.
    let initial_time = f64::from((offset as f32) * (duration as f32));
    let mut events = Vec::new();
    for event in array(clip, "events")? {
        let function = event["functionName"]
            .as_str()
            .ok_or("missing animation event function")?;
        let effect = match function {
            "SetEnableMysekaiEmission" => Effect::Emission(FixtureEmissionOverride::Enabled),
            "SetDisableMysekaiEmission" => Effect::Emission(FixtureEmissionOverride::Disabled),
            "PlaySe" => Effect::PlaySe(
                event["data"]
                    .as_str()
                    .ok_or("animation PlaySe has no string parameter")?
                    .to_owned(),
            ),
            "OnFinishOneShotGimmick" => Effect::FinishOneShot,
            "PlayFixtureActivateEffect" if empty_activate => Effect::EmptyActivate,
            _ => return Err(format!("animation event {function} has no consumer")),
        };
        let time = number(event, "time")?;
        if time < 0.0 || time > duration {
            return Err("animation event is outside its source clip".into());
        }
        events.push(ClipEvent {
            time,
            function: function.to_owned(),
            effect,
        });
    }
    // An authored reset state may have a real, empty clip. Its absence of
    // outputs is not a missing motion and does not synthesize a completion.
    events.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(Program {
        clip: identity(&clip["source"])?,
        duration,
        initial_time,
        speed,
        events,
        transition_seconds,
        rotation,
    })
}

fn effect_outputs(world: &World, view: Entity) -> Vec<Entity> {
    let mut stack = vec![view];
    let mut outputs = Vec::new();
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
        if world
            .get::<FixtureEmission>(entity)
            .is_some_and(|emission| !emission.key.fence && emission.key.rug.is_none())
        {
            outputs.push(entity);
        }
    }
    outputs
}

struct Prepared {
    definition: Arc<Definition>,
    view: Entity,
    rotation: Option<rotation::Binding>,
    outputs: Vec<Entity>,
    light: bool,
    one_shot: bool,
}

/// Resolve one real source-backed controller without acquiring player input.
fn prepare_binding(world: &World, target: &FixtureTarget) -> Result<Prepared, String> {
    if world.get_resource::<crate::fixture_edit::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return Err("the layout editor owns fixture poses; interaction cannot start".into());
    }
    let identity = world
        .get::<FixtureActivityIdentity>(target.entity)
        .filter(|identity| target.matches(identity))
        .ok_or("fixture identity is stale")?
        .clone();
    let (light, one_shot) = {
        let master = world
            .get_resource::<FixtureActivityTables>()
            .and_then(|tables| tables.fixture_master(identity.master_id))
            .ok_or("fixture master is not loaded")?;
        match master.player_action_type.as_str() {
            "loop" => (master.handle_type == "light", false),
            "one_shot" => (master.handle_type == "light", true),
            _ => return Err("fixture does not have a source gimmick action".into()),
        }
    };
    let definition = world
        .get_resource::<Catalog>()
        .ok_or("fixture controller metadata is loading")?
        .0
        .get(&identity.model_package)
        .ok_or("fixture controller metadata is not supplied for this package")?
        .clone()?;
    let view = world
        .get::<FixtureViewInstance>(target.entity)
        .ok_or("actual FixtureView node is not bound")?
        .0;
    let extras: Value = serde_json::from_str(
        &world
            .get::<bevy::gltf::GltfExtras>(view)
            .ok_or("FixtureView source node identity missing")?
            .value,
    )
    .map_err(|_| "FixtureView source node identity invalid")?;
    if extras["gameObjectId"].as_i64() != Some(definition.view_game_object) {
        return Err("controller receiver differs from the actual FixtureView".into());
    }
    let rotation = rotation::bind(world, view, &definition)?;
    let outputs = effect_outputs(world, view);
    let needs_emission = definition.on.events.iter().chain(&definition.off.events)
        .any(|event| matches!(&event.effect, Effect::Emission(_)));
    if needs_emission && outputs.is_empty() {
        return Err("fixture's actual emission materials are not ready".into());
    }
    Ok(Prepared { definition, view, rotation, outputs, light, one_shot })
}

pub(crate) fn start(
    world: &mut World,
    target: &FixtureTarget,
    generation: u64,
) -> Result<(), String> {
    let Prepared { definition, view, rotation, outputs, light, one_shot } = prepare_binding(world, target)?;
    if world.contains_resource::<PlayerFixtureControlOwner>()
        || !world.resource::<PlayerAvatarStates>().can_intercept
    {
        return Err("player controls are occupied".into());
    }
    let actors: Vec<_> = world
        .query_filtered::<(Entity, &AvatarDriver, &DashMode), With<PlayerControlled>>()
        .iter(world)
        .filter(|(_, driver, _)| driver.locomotion_owns_animator())
        .map(|(entity, _, dash)| (entity, dash.0))
        .collect();
    let [(actor, before_dash)] = actors.as_slice() else {
        return Err("one free SD player is required".into());
    };
    if world.get::<crate::talk::TalkHold>(*actor).is_some() {
        return Err("player is in a conversation".into());
    }
    let owner = FixtureActivityOwner {
        actor: *actor,
        generation,
    };
    let site_epoch = world
        .get_resource::<GroundEpoch>()
        .ok_or("active site generation missing")?
        .0;
    world.resource_scope(|world, mut runtime: Mut<Gimmicks>| {
        if runtime.player.is_some() { return Err("player switch interval is occupied".into()); }
        let instance = runtime.instances.entry(target.clone()).or_insert_with(|| Instance {
            on: false,
            definition: definition.clone(),
            playback: None,
            rotation,
            pending: PendingTriggers::default(),
            outputs: outputs.clone(),
        });
        // Completion is the authored callback, not the clip's playback end.
        if one_shot && instance.on {
            return Err("fixture one-shot is still on".into());
        }
        // Resolve the exact adapted clip and take this body's existing
        // animator lease before the irreversible furniture model trigger.
        let gesture = switch_gesture::start(world, owner.actor)?;
        instance.on = !instance.on;
        let program = if instance.on { definition.on.clone() } else { definition.off.clone() };
        info!("[fixture-gimmick] {} trigger={} source-clip={:?} outputs={}", target.uid, if instance.on { "On" } else { "Off" }, program.clip, outputs.len());
        instance.outputs = outputs.clone();
        if instance.rotation.is_some() {
            // The source business model changed above, but the Animator may
            // still be blending another state. Set only the requested bool.
            instance.pending.set(if instance.on { Trigger::On } else { Trigger::Off });
        } else {
            // Event-only packages keep their already adopted scheduling.
            instance.playback = Some(Playback::new(program, outputs, None));
        }
        let states = &mut *world.resource_mut::<PlayerAvatarStates>();
        states.change_status(PlayerActionState::SwitchGimmick);
        states.can_intercept = false;
        world.entity_mut(owner.actor).insert((PlayerFixtureHeld(owner), MotionPhase::Dwelling { remaining: None }));
        world.insert_resource(PlayerFixtureControlOwner(owner));
        if let Some(mut input) = world.get_mut::<PlayerInput>(owner.actor) { input.active = false; input.direction = Vec3::ZERO; }
        runtime.player = Some(PlayerSwitch { owner, gesture, target: target.clone(), view, site_epoch, elapsed: 0.0, sound_checked: false, light, before_dash: *before_dash });
        Ok(())
    })
}

pub(crate) fn advance(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs();
    world.resource_scope(|world, mut runtime: Mut<Gimmicks>| {
        let runtime = &mut *runtime;
        runtime.instances.retain(|target, _| world.get::<FixtureActivityIdentity>(target.entity).is_some_and(|identity| target.matches(identity)));
        for (target, instance) in &mut runtime.instances {
            // First positive contribution includes the clip's start event;
            // a zero-length clip is not an empty event interval. A paused
            // update must not consume that first contribution.
            if delta <= 0.0 { continue; }
            let mut consumed = None;
            if let Some(order) = instance.definition.trigger_order {
                // Snapshot the scan gate before advancing either clip. A
                // transition that finishes below cannot rescan this update.
                if !instance.playback.as_ref().is_some_and(Playback::transitioning) {
                    if let Some(trigger) = instance.pending.select(order) {
                        let program = match trigger {
                            Trigger::On => instance.definition.on.clone(),
                            Trigger::Off => instance.definition.off.clone(),
                        };
                        info!("[fixture-gimmick] {} selected-trigger={} source-clip={:?}", target.uid, trigger.name(), program.clip);
                        let previous = instance.playback.take();
                        instance.playback = Some(Playback::new(program, instance.outputs.clone(), previous));
                        consumed = Some(trigger);
                    }
                }
            }
            let Some(mut playback) = instance.playback.take() else { continue; };
            let mut reset_triggered = false;
            playback.elapsed += f64::from(delta) * playback.program.speed;
            playback.transition_elapsed += f64::from(delta);
            let mut events = Vec::new();
            if let Some(previous) = &mut playback.previous {
                previous.elapsed += f64::from(delta) * previous.program.speed;
                if playback.transition_elapsed < playback.program.transition_seconds {
                    collect_events(&previous.program, previous.elapsed, &mut previous.next_event, &mut events);
                }
            }
            collect_events(&playback.program, playback.elapsed, &mut playback.next_event, &mut events);
            // Source controller consumption is separate from event callbacks.
            // Clear only the selected bit now; a callback's new Off survives.
            if let Some(trigger) = consumed { instance.pending.consume(trigger); }
            if let Some(binding) = &instance.rotation {
                if let Err(reason) = rotation::apply(world, binding, &playback) {
                    warn!("[fixture-gimmick] {} Euler playback stopped: {reason}", target.uid);
                    continue;
                }
            }
            for event in &events {
                match &event.effect {
                    Effect::Emission(mode) => {
                        for &entity in &playback.outputs {
                            if world.get::<FixtureEmission>(entity).is_some() { world.entity_mut(entity).insert(*mode); }
                        }
                        info!("[fixture-gimmick] {} event={} source-time={} applied-to={:?}", target.uid, event.function, event.time, playback.outputs);
                    }
                    Effect::PlaySe(bundle_or_cue) => {
                        // The event may name a bundle path. The shared sound
                        // bus resolves its leaf cue within that exact bank.
                        // Empty PlaySe parameters are no-ops.
                        if !bundle_or_cue.is_empty() {
                            world.resource_mut::<SeRequests>().0.push(SeRequest { owner: runtime.leases.get(target).map(|lease| lease.owner),
                                cue: bundle_or_cue.clone(),
                                class: SeClass::Ingame,
                                source: "fixture-animation-event",
                            });
                        }
                    }
                    Effect::FinishOneShot => {
                        instance.on = false;
                        if instance.rotation.is_some() {
                            // SetTrigger does not replace the active state.
                            // Preserve any pending On and select Off only in
                            // a later eligible controller evaluation.
                            instance.pending.set(Trigger::Off);
                        } else {
                            reset_triggered = true;
                        }
                        info!("[fixture-gimmick] {} event={} source-time={} trigger=Off", target.uid, event.function, event.time);
                    }
                    Effect::EmptyActivate => {
                        info!("[fixture-gimmick] {} event={} source-time={} audited CN receiver has no activation-particle supply", target.uid, event.function, event.time);
                    }
                }
            }
            if !playback.transitioning() { playback.previous = None; }
            if reset_triggered {
                // The callback sets the model Off immediately and sends the
                // authored Off trigger. Its new clip advances next update;
                // remaining events from this already sampled clip keep order.
                instance.playback = Some(Playback::new(
                    instance.definition.off.clone(), playback.outputs.clone(), Some(playback),
                ));
            } else if playback.elapsed < playback.program.duration || instance.rotation.is_some() {
                instance.playback = Some(playback);
            }
        }
        let Some(mut player) = runtime.player.take() else { return; };
        let alive = world.get_resource::<GroundEpoch>().is_some_and(|epoch| epoch.0 == player.site_epoch)
            && switch_gesture::is_alive(world, player.owner.actor, player.gesture)
            && world.get::<FixtureActivityIdentity>(player.target.entity).is_some_and(|identity| player.target.matches(identity))
            && world.get::<GlobalTransform>(player.view).is_some()
            && world.get::<PlayerFixtureHeld>(player.owner.actor).is_some_and(|held| held.0 == player.owner)
            && world.get_resource::<PlayerFixtureControlOwner>().is_some_and(|held| held.0 == player.owner);
        if !alive { finish(world, player); return; }
        player.elapsed += delta;
        if let Some(target_position) = world.get::<GlobalTransform>(player.view).map(GlobalTransform::translation) {
            if let Some(mut transform) = world.get_mut::<Transform>(player.owner.actor) {
                let direction = target_position - transform.translation;
                transform.rotation = Quat::from_rotation_y(direction.x.atan2(direction.z));
            }
        }
        if !player.sound_checked && player.elapsed >= SWITCH_SOUND_TIME {
            player.sound_checked = true;
            if player.light {
                world.resource_mut::<SeRequests>().0.push(SeRequest { owner: runtime.leases.get(&player.target).map(|lease| lease.owner), cue: "se_turn_on".into(), class: SeClass::Ingame, source: "fixture-switch" });
                info!("[fixture-gimmick] source switch SE queued; positional attenuation is not yet reproduced by the shared SE bus");
            }
        }
        if player.elapsed >= SWITCH_END_TIME { finish(world, player); } else { runtime.player = Some(player); }
    });
    session::advance(world);
}

fn collect_events(program: &Program, elapsed: f64, next: &mut usize, events: &mut Vec<ClipEvent>) {
    while let Some(event) = program.events.get(*next).filter(|event| event.time <= elapsed) {
        events.push(event.clone());
        *next += 1;
    }
}

fn finish(world: &mut World, player: PlayerSwitch) {
    let owner = player.owner;
    switch_gesture::release(world, owner.actor, player.gesture);
    let owns_controls = world
        .get_resource::<PlayerFixtureControlOwner>()
        .is_some_and(|held| held.0 == owner);
    if world
        .get::<PlayerFixtureHeld>(owner.actor)
        .is_some_and(|held| held.0 == owner)
    {
        world.entity_mut(owner.actor).remove::<PlayerFixtureHeld>();
        if owns_controls {
            world.entity_mut(owner.actor).insert((
                DashMode(player.before_dash),
                MotionPhase::Dwelling { remaining: None },
            ));
            if let Some(mut input) = world.get_mut::<PlayerInput>(owner.actor) {
                input.active = false;
                input.direction = Vec3::ZERO;
            }
        }
    }
    if owns_controls {
        world.remove_resource::<PlayerFixtureControlOwner>();
        let states = &mut *world.resource_mut::<PlayerAvatarStates>();
        if states.current == PlayerActionState::SwitchGimmick {
            states.can_intercept = true;
            states.change_status(PlayerActionState::Idle);
        }
    }
    info!(
        "[fixture-gimmick] {} player switch interval released",
        player.target.uid
    );
}

/// Host teardown protection: release only this generation's input lease.
/// This is lifecycle adaptation, not a claim that the source state has a
/// dedicated target-removal callback.
pub(crate) fn cancel_for_site_change(world: &mut World) {
    let owners: Vec<_> = world.resource::<Gimmicks>().leases.values().map(|lease| lease.owner).collect();
    for owner in owners { session::finish_owner(world, owner); }
    world.resource_scope(|world, mut runtime: Mut<Gimmicks>| {
        if let Some(player) = runtime.player.take() {
            finish(world, player);
        }
    });
}
