//! The house controller's `PlayerOff` lane, bounded to what the entry plays.
//!
//! `HouseView` is a separate component on the house's FixtureView object: it
//! holds the two door locators and the house Animator. Its controller
//! (`HouseAnimationController`) has four AnyState trigger transitions on
//! layer 0 and an empty second layer (no states, weight 0), so only layer 0
//! can write a pose. The entry sets one trigger, `PlayerOff`; this lane plays
//! exactly that transition: 0.25 s fixed, from the default state `None`
//! (no motion, write defaults on, so the blend base is the bound pose), into
//! a four-component quaternion clip on the door joint. The clip's
//! `OnPlayHouseSE` events play the door sounds through `HouseView.OnPlayHouseSE`.
//! Any other shape of house controller is refused with its reason.

use std::{collections::HashMap, sync::Arc};

use bevy::{animation::AnimatedBy, prelude::*};
use moly_assets::source_navigation::SourceObjectIdentity;
use serde_json::Value;

use super::{array, identity, number, one, referenced, rotation};
use crate::{
    audio::{SeClass, SeRequest, SeRequests},
    fixture::{FixtureViewInstance, FixtureViewResolved},
    fixture_activity_state::FixtureActivityIdentity,
    fixture_activity_timeline::SourceAssetId,
    player_state::{PlayerActionState, PlayerAvatarStates},
    source_curve::Curve,
};

const TRIGGER: &str = "PlayerOff";
const SE_EVENT: &str = "OnPlayHouseSE";

#[derive(Clone, Debug, PartialEq, Eq)]
struct NodeId {
    game_object: i64,
    transform: i64,
}

impl NodeId {
    fn matches(&self, file: &str, id: &SourceObjectIdentity) -> bool {
        id.file == file
            && id.game_object == self.game_object
            && id.transform == self.transform
            && id.components.contains(&self.transform)
    }
}

struct DoorEvent {
    time: f64,
    se: String,
}

struct DoorProgram {
    clip: SourceAssetId,
    duration: f64,
    speed: f64,
    transition_seconds: f64,
    joint: NodeId,
    /// x, y, z, w of the source quaternion curve, source (Unity) frame.
    components: [Curve; 4],
    events: Vec<DoorEvent>,
}

impl DoorProgram {
    /// The clip's quaternion at `time`, converted once to the runtime frame.
    /// Mecanim blends raw samples and normalizes after accumulation, so the
    /// sample is not normalized here.
    fn sample(&self, time: f64) -> Quat {
        let t = time.min(self.duration) as f32;
        let raw = Quat::from_xyzw(
            self.components[0].sample(t),
            self.components[1].sample(t),
            self.components[2].sample(t),
            self.components[3].sample(t),
        );
        moly_assets::coordinates::source_rotation(raw)
    }
}

/// The house view record of one fixture package.
pub(crate) struct HouseDefinition {
    file: String,
    view_game_object: i64,
    house_view: i64,
    animator: i64,
    inside_door: Option<NodeId>,
    outside_door: Option<NodeId>,
    player_off: Arc<DoorProgram>,
}

/// Packages that carry a `HouseView`, keyed by package name. A package whose
/// controller is not the bounded shape keeps its refusal reason.
#[derive(Resource, Default)]
pub(crate) struct HouseCatalog(HashMap<String, Result<Arc<HouseDefinition>, String>>);

impl HouseCatalog {
    pub(crate) fn insert(&mut self, name: &str, package: &Value) {
        if package.get("houseViews").is_none() {
            return;
        }
        let definition = house_definition(package).map(Arc::new);
        if let Err(reason) = &definition {
            warn!("[house-door] {name} HouseView preparation: {reason}");
        }
        self.0.insert(name.to_owned(), definition);
    }

    fn get(&self, package: &str) -> Option<&Result<Arc<HouseDefinition>, String>> {
        self.0.get(package)
    }
}

fn i64_id(value: &Value, label: &str) -> Result<i64, String> {
    identity(value)?
        .path_id
        .parse()
        .map_err(|_| format!("{label} identity is not i64"))
}

fn locator(views: &Value, transforms: &[Value], field: &str) -> Result<Option<NodeId>, String> {
    let reference = &views[field];
    if reference.is_null() {
        return Ok(None);
    }
    let target = identity(reference)?;
    let rows: Vec<_> = transforms
        .iter()
        .filter(|row| identity(&row["asset"]).ok().as_ref() == Some(&target))
        .collect();
    let [row] = rows.as_slice() else {
        return Err(format!(
            "{field} Transform is missing or ambiguous in the record"
        ));
    };
    Ok(Some(NodeId {
        game_object: i64_id(&row["gameObject"], field)?,
        transform: target
            .path_id
            .parse()
            .map_err(|_| format!("{field} identity is not i64"))?,
    }))
}

fn house_definition(package: &Value) -> Result<HouseDefinition, String> {
    let house = &package["houseViews"];
    let view = one(array(house, "views")?, "HouseView")?;
    if view["enabled"].as_u64() != Some(1) {
        return Err("HouseView component is disabled".into());
    }
    let transforms = array(house, "transforms")?;
    let view_object = identity(&view["gameObject"])?;
    let file = view_object.file.clone();
    let animator_id = identity(&view["animator"])?;
    if identity(&view["asset"])?.file != file || animator_id.file != file {
        return Err("HouseView fields cross serialized-file identity domains".into());
    }
    let animator = referenced(array(package, "animators")?, &view["animator"])?;
    // Animation events call methods on the Animator's own GameObject; that
    // must be the HouseView object for OnPlayHouseSE to reach it.
    if identity(&animator["gameObject"])? != view_object {
        return Err("house Animator is not on the HouseView object".into());
    }
    let controller = referenced(
        array(package, "controllers")?,
        &animator["controller"]["source"],
    )?;
    if controller["motionMapping"]["status"].as_str() != Some("resolved")
        || controller["motionMapping"]["kind"].as_str() != Some("clip-binding-array-index")
    {
        return Err("house controller motion-index mapping is not resolved".into());
    }
    let machines = array(controller, "stateMachines")?;
    let machine_of = |layer: &Value| -> Result<&Value, String> {
        let index = layer["data"]["m_StateMachineIndex"]
            .as_u64()
            .ok_or("house layer has no state machine index")?;
        machines
            .iter()
            .find(|machine| machine["index"].as_u64() == Some(index))
            .ok_or_else(|| "house layer state machine absent".to_owned())
    };
    let [base, second] = array(controller, "layers")? else {
        return Err("house controller is not the two-layer HouseAnimationController shape".into());
    };
    let second = machine_of(second)?;
    if !array(second, "states")?.is_empty() || !array(second, "anyStateTransitions")?.is_empty() {
        return Err(
            "house controller's second layer has states; it would need a layer mixer".into(),
        );
    }
    let machine = machine_of(base)?;
    let initial = machine["defaultState"]
        .as_u64()
        .ok_or("house machine has no default state")?;
    let states = array(machine, "states")?;
    let default = states
        .iter()
        .find(|state| state["index"].as_u64() == Some(initial))
        .ok_or("house default state absent")?;
    if !array(default, "motions")?.is_empty()
        || default["writeDefaultValues"].as_bool() != Some(true)
    {
        return Err("house default state is not the motionless write-defaults state".into());
    }
    let parameters = array(controller, "parameters")?;
    let named: Vec<_> = parameters
        .iter()
        .filter(|parameter| parameter["name"].as_str() == Some(TRIGGER))
        .collect();
    let [parameter] = named.as_slice() else {
        return Err("house controller has no unique PlayerOff parameter".into());
    };
    if parameter["type"].as_u64() != Some(9) {
        return Err("PlayerOff is not a Trigger parameter".into());
    }
    let parameter_id = parameter["id"]
        .as_u64()
        .ok_or("PlayerOff parameter has no id")?;
    let transitions: Vec<_> = array(machine, "anyStateTransitions")?
        .iter()
        .filter(|transition| {
            transition["conditions"]
                .as_array()
                .is_some_and(|conditions| {
                    conditions.len() == 1
                        && conditions[0]["mode"].as_u64() == Some(1)
                        && conditions[0]["parameterId"].as_u64() == Some(parameter_id)
                })
        })
        .collect();
    let [transition] = transitions.as_slice() else {
        return Err("PlayerOff has no unique AnyState transition".into());
    };
    if transition["hasExitTime"].as_bool() != Some(false)
        || transition["hasFixedDuration"].as_bool() != Some(true)
        || transition["raw"]["m_InterruptionSource"].as_u64() != Some(0)
        || number(transition, "offset")? != 0.0
    {
        return Err(
            "PlayerOff transition is not a fixed, uninterruptible, zero-offset trigger".into(),
        );
    }
    let transition_seconds = number(transition, "duration")?;
    if transition_seconds < 0.0 {
        return Err("PlayerOff transition duration is negative".into());
    }
    let destination = transition["destinationState"]
        .as_u64()
        .ok_or("PlayerOff transition has no destination")?;
    let state = states
        .iter()
        .find(|state| state["index"].as_u64() == Some(destination))
        .ok_or("PlayerOff destination state absent")?;
    if !array(state, "transitions")?.is_empty() {
        return Err(
            "PlayerOff state has outgoing transitions; it would need a controller runner".into(),
        );
    }
    if state["raw"]["m_CycleOffset"].as_f64() != Some(0.0)
        || state["raw"]["m_SpeedParamID"].as_u64() != Some(0)
        || state["raw"]["m_CycleOffsetParamID"].as_u64() != Some(0)
        || state["raw"]["m_TimeParamID"].as_u64() != Some(0)
        || state["raw"]["m_Mirror"].as_bool() != Some(false)
    {
        return Err("PlayerOff state has dynamic time or mirror inputs".into());
    }
    let speed = number(state, "speed")?;
    let motion = one(array(state, "motions")?, "PlayerOff motion")?;
    let clip = referenced(array(package, "clips")?, &motion["clip"]["source"])?;
    if clip["loopTime"].as_bool() != Some(false) || number(clip, "startTime")? != 0.0 {
        return Err("PlayerOff clip is looping or does not start at zero".into());
    }
    if !array(clip, "pptrCurves")?.is_empty() {
        return Err("PlayerOff clip has object-reference curves".into());
    }
    let duration = number(clip, "duration")?;
    if duration < 0.0 || speed <= 0.0 {
        return Err("PlayerOff clip time domain is invalid".into());
    }
    let curves = array(clip, "curves")?;
    if curves.len() != 4 {
        return Err("PlayerOff clip is not one quaternion Transform binding".into());
    }
    let mut joint = None;
    let mut components: [Option<Curve>; 4] = std::array::from_fn(|_| None);
    for value in curves {
        let binding = &value["binding"];
        if binding["typeId"].as_u64() != Some(4) || binding["attribute"].as_u64() != Some(2) {
            return Err("PlayerOff clip writes something other than a Transform rotation".into());
        }
        let component = binding["component"]
            .as_u64()
            .filter(|component| *component < 4)
            .ok_or("invalid quaternion component index")? as usize;
        if components[component].is_some() {
            return Err("duplicate quaternion component binding".into());
        }
        let output = one(array(value, "targets")?, "quaternion curve target")?;
        if identity(&output["animator"])? != animator_id {
            return Err("quaternion curve belongs to another Animator".into());
        }
        let game_object = identity(&output["gameObject"])?;
        let transform = one(array(output, "components")?, "quaternion Transform")?;
        if transform["class"].as_str() != Some("Transform") {
            return Err("quaternion curve component is not a Transform".into());
        }
        let transform = identity(transform)?;
        if game_object.file != file || transform.file != file {
            return Err("quaternion curve crosses serialized-file identity domains".into());
        }
        let current = NodeId {
            game_object: game_object
                .path_id
                .parse()
                .map_err(|_| "invalid joint GameObject")?,
            transform: transform
                .path_id
                .parse()
                .map_err(|_| "invalid joint Transform")?,
        };
        if joint.as_ref().is_some_and(|joint| joint != &current) {
            return Err("quaternion curves do not share one Transform".into());
        }
        joint = Some(current);
        components[component] = Some(rotation::curve(value)?);
    }
    let [Some(x), Some(y), Some(z), Some(w)] = components else {
        return Err("quaternion component set is incomplete".into());
    };
    let mut events = Vec::new();
    for event in array(clip, "events")? {
        if event["functionName"].as_str() != Some(SE_EVENT) {
            return Err(format!(
                "PlayerOff event {:?} has no consumer",
                event["functionName"].as_str()
            ));
        }
        let se = event["data"]
            .as_str()
            .filter(|se| !se.is_empty())
            .ok_or("OnPlayHouseSE has no sound name")?;
        let time = number(event, "time")?;
        if !(0.0..=duration).contains(&time) {
            return Err("house event is outside its clip".into());
        }
        events.push(DoorEvent {
            time,
            se: se.to_owned(),
        });
    }
    events.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(HouseDefinition {
        file,
        view_game_object: view_object
            .path_id
            .parse()
            .map_err(|_| "invalid HouseView GameObject")?,
        house_view: i64_id(&view["asset"], "HouseView")?,
        animator: animator_id
            .path_id
            .parse()
            .map_err(|_| "invalid Animator identity")?,
        inside_door: locator(view, transforms, "insideDoorActionPoint")?,
        outside_door: locator(view, transforms, "outsideDoorActionPoint")?,
        player_off: Arc::new(DoorProgram {
            clip: identity(&clip["source"])?,
            duration,
            speed,
            transition_seconds,
            joint: joint.expect("four curves share one joint"),
            components: [x, y, z, w],
            events,
        }),
    })
}

/// One placed house resolved to its actual entities.
pub(crate) struct HouseBinding {
    pub(crate) root: Entity,
    pub(crate) uid: String,
    pub(crate) package: String,
    /// `HouseView.OutsideDoorActionPoint`; `None` when the record has none.
    pub(crate) outside_door: Option<Entity>,
    /// `HouseView.InsideDoorActionPoint`; `None` when the record has none.
    pub(crate) inside_door: Option<Entity>,
    joint: Entity,
    rest: Quat,
    definition: Arc<HouseDefinition>,
}

/// `FixtureManager.GetHouseView` over the placed layout.
pub(crate) enum HouseLookup {
    /// A placed house package is still binding its scene or view.
    Pending(String),
    /// No placed fixture carries a HouseView.
    Absent,
    Found(HouseBinding),
    /// A house is placed but cannot be driven; the reason names why.
    Failed(String),
}

pub(crate) fn find_house(world: &mut World) -> HouseLookup {
    if !world.contains_resource::<HouseCatalog>() {
        return HouseLookup::Pending("house controller catalogue is loading".into());
    }
    let mut roots = world.query::<(Entity, &FixtureActivityIdentity)>();
    let placed: Vec<_> = roots
        .iter(world)
        .map(|(entity, identity)| (entity, identity.clone()))
        .collect();
    let catalog = world.resource::<HouseCatalog>();
    let houses: Vec<_> = placed
        .into_iter()
        .filter_map(|(entity, identity)| {
            catalog
                .get(&identity.model_package)
                .map(|definition| (entity, identity, definition.clone()))
        })
        .collect();
    let [(root, identity, definition)] = houses.as_slice() else {
        return if houses.is_empty() {
            HouseLookup::Absent
        } else {
            HouseLookup::Failed(format!(
                "{} placed fixtures carry a HouseView",
                houses.len()
            ))
        };
    };
    let definition = match definition {
        Ok(definition) => definition.clone(),
        Err(reason) => return HouseLookup::Failed(format!("{}: {reason}", identity.model_package)),
    };
    let Some(view) = world.get::<FixtureViewInstance>(*root).map(|view| view.0) else {
        return if world.get::<FixtureViewResolved>(*root).is_some() {
            HouseLookup::Failed(format!(
                "{} FixtureView was not bound",
                identity.model_package
            ))
        } else {
            HouseLookup::Pending(format!("{} FixtureView is binding", identity.model_package))
        };
    };
    match bind(world, *root, identity, view, definition) {
        Ok(binding) => HouseLookup::Found(binding),
        Err(reason) => HouseLookup::Failed(format!("{}: {reason}", identity.model_package)),
    }
}

fn bind(
    world: &World,
    root: Entity,
    identity: &FixtureActivityIdentity,
    view: Entity,
    definition: Arc<HouseDefinition>,
) -> Result<HouseBinding, String> {
    let receiver = world
        .get::<SourceObjectIdentity>(view)
        .ok_or("HouseView object has no source identity; re-export this fixture")?;
    if receiver.file != definition.file
        || receiver.game_object != definition.view_game_object
        || !receiver.components.contains(&definition.house_view)
        || !receiver.components.contains(&definition.animator)
    {
        return Err("bound FixtureView is not the HouseView/Animator object".into());
    }
    let mut stack = vec![view];
    let (mut outside, mut inside, mut joints) = (Vec::new(), Vec::new(), Vec::new());
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
        let Some(id) = world.get::<SourceObjectIdentity>(entity) else {
            continue;
        };
        if definition
            .outside_door
            .as_ref()
            .is_some_and(|node| node.matches(&definition.file, id))
        {
            outside.push(entity);
        }
        if definition
            .inside_door
            .as_ref()
            .is_some_and(|node| node.matches(&definition.file, id))
        {
            inside.push(entity);
        }
        if definition.player_off.joint.matches(&definition.file, id) {
            joints.push(entity);
        }
    }
    let single = |found: &[Entity], label: &str| -> Result<Entity, String> {
        match found {
            [entity] => Ok(*entity),
            _ => Err(format!(
                "{label} has {} actual source-identity matches",
                found.len()
            )),
        }
    };
    let outside_door = definition
        .outside_door
        .as_ref()
        .map(|_| single(&outside, "outside door locator"))
        .transpose()?;
    let inside_door = definition
        .inside_door
        .as_ref()
        .map(|_| single(&inside, "inside door locator"))
        .transpose()?;
    let joint = single(&joints, "door joint")?;
    let rest = world
        .get::<Transform>(joint)
        .ok_or("door joint has no Transform")?
        .rotation;
    if let Some(by) = world.get::<AnimatedBy>(joint) {
        if world
            .get::<AnimationPlayer>(by.0)
            .is_some_and(|player| player.playing_animations().next().is_some())
        {
            return Err("door joint is already owned by an active animation player".into());
        }
    }
    Ok(HouseBinding {
        root,
        uid: identity.uid.clone(),
        package: identity.model_package.clone(),
        outside_door,
        inside_door,
        joint,
        rest,
        definition,
    })
}

struct DoorPlayback {
    root: Entity,
    uid: String,
    joint: Entity,
    file: String,
    rest: Quat,
    program: Arc<DoorProgram>,
    elapsed: f64,
    transition_elapsed: f64,
    next_event: usize,
}

#[derive(Resource, Default)]
pub(crate) struct HouseDoors(Vec<DoorPlayback>);

/// `HouseView.SetAnimationTrigger("PlayerOff")`: `Animator.SetTrigger`. The
/// controller takes the AnyState transition on its next evaluation, which is
/// this frame's animation update: the playback starts here and advances in
/// the same frame's [`advance`].
pub(crate) fn set_trigger_player_off(
    world: &mut World,
    binding: &HouseBinding,
) -> Result<(), String> {
    let mut doors = world.get_resource_or_insert_with(HouseDoors::default);
    if doors.0.iter().any(|door| door.root == binding.root) {
        return Err("a PlayerOff transition is already playing on this house".into());
    }
    let program = binding.definition.player_off.clone();
    info!(
        "[house-door] {} trigger={TRIGGER} source-clip={:?} transition={}s duration={}s joint={:?}",
        binding.uid, program.clip, program.transition_seconds, program.duration, binding.joint
    );
    doors.0.push(DoorPlayback {
        root: binding.root,
        uid: binding.uid.clone(),
        joint: binding.joint,
        file: binding.definition.file.clone(),
        rest: binding.rest,
        program,
        elapsed: 0.0,
        transition_elapsed: 0.0,
        next_event: 0,
    });
    Ok(())
}

/// Transition weight toward the clip: linear in fixed-duration seconds.
fn transition_weight(transition_elapsed: f64, seconds: f64) -> f32 {
    if seconds > 0.0 {
        (transition_elapsed / seconds).clamp(0.0, 1.0) as f32
    } else {
        1.0
    }
}

/// Mecanim's rotation accumulation: sign-corrected weighted sum, normalized.
/// The default state writes the bound pose with the remaining weight.
fn blend(rest: Quat, sampled: Quat, weight: f32) -> Quat {
    let sign = if rest.dot(sampled) < 0.0 { -1.0 } else { 1.0 };
    (rest * (1.0 - weight) + sampled * (weight * sign)).normalize()
}

/// `HouseView.CanPlayHouseDoorSe`: always on the home site, otherwise only
/// while the player is in ExitMoveHouse (13). The ignore set is written only
/// by the room-to-home action, which the entry does not run.
fn can_play_door_se(world: &World) -> bool {
    world
        .get_resource::<crate::site::SiteActive>()
        .is_some_and(|site| site.site_type == "home_site")
        || world
            .get_resource::<PlayerAvatarStates>()
            .is_some_and(|states| states.current == PlayerActionState::ExitMoveHouse)
}

pub(crate) fn advance(world: &mut World) {
    let delta = world.resource::<Time>().delta_secs();
    if delta <= 0.0 {
        return;
    }
    let Some(mut doors) = world.remove_resource::<HouseDoors>() else {
        return;
    };
    doors.0.retain_mut(|door| {
        let identity_ok = world
            .get::<SourceObjectIdentity>(door.joint)
            .is_some_and(|id| door.program.joint.matches(&door.file, id));
        if world.get_entity(door.root).is_err() || !identity_ok {
            warn!(
                "[house-door] {} playback stopped: the house or its door joint disappeared",
                door.uid
            );
            return false;
        }
        door.elapsed += f64::from(delta) * door.program.speed;
        door.transition_elapsed += f64::from(delta);
        let weight = transition_weight(door.transition_elapsed, door.program.transition_seconds);
        let pose = blend(door.rest, door.program.sample(door.elapsed), weight);
        if let Some(mut transform) = world.get_mut::<Transform>(door.joint) {
            if transform.rotation != pose {
                transform.rotation = pose;
            }
        }
        while let Some(event) = door
            .program
            .events
            .get(door.next_event)
            .filter(|event| event.time <= door.elapsed)
        {
            if can_play_door_se(world) {
                world.resource_mut::<SeRequests>().0.push(SeRequest {
                    owner: None,
                    cue: event.se.clone(),
                    class: SeClass::Ingame,
                    source: "house-animation-event",
                });
                info!(
                    "[house-door] {} event={SE_EVENT}({}) source-time={} clip-time={:.3}",
                    door.uid, event.se, event.time, door.elapsed
                );
            } else {
                info!(
                    "[house-door] {} event={SE_EVENT}({}) not played: CanPlayHouseDoorSe is false",
                    door.uid, event.se
                );
            }
            door.next_event += 1;
        }
        let finished = door.elapsed >= door.program.duration
            && door.transition_elapsed >= door.program.transition_seconds;
        if finished {
            info!(
                "[house-door] {} PlayerOff clip held at its last pose",
                door.uid
            );
        }
        !finished
    });
    world.insert_resource(doors);
}

#[cfg(test)]
mod tests {
    //! Research instrument: the mis0001 house PlayerOff clip (u000 011_O) as
    //! extracted, its first six cubic keys per component copied verbatim, and
    //! the pose Mecanim writes through the 0.25 s AnyState transition from the
    //! bound (identity) pose. Expected values were evaluated from the record
    //! in f32 (cubic `((a*dt+b)*dt+c)*dt+d`, linear transition weight,
    //! sign-corrected weighted sum, normalization, then the (x,-y,-z,w)
    //! frame map), outside this module.
    use super::*;

    fn keys(rows: &[(f32, [f32; 4])]) -> Curve {
        Curve::Cubic(rows.to_vec())
    }

    fn program() -> DoorProgram {
        let x = keys(&[
            (0.0, [0.0, 0.0, 0.0, 0.0]),
            (0.049999952, [0.0, 0.0, 0.0, 0.0]),
            (0.116666794, [0.0, 0.0, 0.0, 0.0]),
            (0.23333335, [0.0, 0.0, 0.0, 0.0]),
            (0.3333335, [-1.084552e-15, 8.742073e-17, 0.0, 0.0]),
            (
                0.4666667,
                [-2.4036805e-16, 1.5268923e-16, -3.4530486e-17, -1.016639e-18],
            ),
        ]);
        let y = keys(&[
            (0.0, [2.742409, 0.16487095, -0.03705511, 0.0010977769]),
            (0.049999952, [-72.123886, -4.707723, -1.1657647e-13, -0.0]),
            (0.116666794, [43.46655, -13.686095, -1.5893548, -0.04229353]),
            (0.23333335, [30.012777, 1.6096666, -3.0078936, -0.34497762]),
            (0.3333335, [-18.50556, 9.3509865, -1.7855737, -0.5996578]),
            (0.4666667, [-1.9792604, 1.249318, -0.27894107, -0.7153595]),
        ]);
        let z = keys(&[
            (0.0, [0.0, -0.0, 0.0, 0.0]),
            (0.049999952, [0.0, 0.0, 0.0, -0.0]),
            (0.116666794, [0.0, 0.0, 0.0, -0.0]),
            (0.23333335, [0.0, 0.0, 0.0, -0.0]),
            (0.3333335, [1.084552e-15, -8.742073e-17, 0.0, -0.0]),
            (
                0.4666667,
                [2.4036805e-16, -1.5268923e-16, 3.4530486e-17, 1.016639e-18],
            ),
        ]);
        let w = keys(&[
            (0.0, [0.034332577, 0.0032901622, -0.00058650976, 1.0000168]),
            (0.049999952, [-10.420761, 0.49340707, 0.0, 1.0]),
            (
                0.116666794,
                [-10.444449, -2.598919, -0.073156424, 0.9991053],
            ),
            (0.23333335, [33.10434, -6.08534, -1.1060511, 0.9386109]),
            (0.3333335, [-5.2015433, 4.959072, -1.3299878, 0.80025655]),
            (0.4666667, [-1.9462461, 1.2442869, -0.28498498, 0.6987566]),
        ]);
        DoorProgram {
            clip: SourceAssetId {
                file: "test".into(),
                path_id: "-714266417824402808".into(),
            },
            duration: 1.916_666_746_139_526_4,
            speed: 1.0,
            transition_seconds: 0.25,
            joint: NodeId {
                game_object: 1_986_495_244_691_942_711,
                transform: 1_099_000_171_873_291_942,
            },
            components: [x, y, z, w],
            events: Vec::new(),
        }
    }

    #[test]
    fn player_off_pose_matches_the_extracted_curves_through_the_transition() {
        let program = program();
        // (clip time, expected runtime-frame y, expected w); x and z stay 0.
        let expected: [(f64, f32, f32); 6] = [
            (0.05, 2.208_250_7e-15, 1.0),
            (0.10, 0.008_313_880_3, 0.999_965_5),
            (0.20, 0.196_768_97, 0.980_449_86),
            (0.25, 0.394_612_37, 0.918_847_7),
            (0.30, 0.529_771_5, 0.848_140_36),
            (0.40, 0.681_963_2, 0.731_386_5),
        ];
        for (time, y, w) in expected {
            let weight = transition_weight(time, program.transition_seconds);
            let pose = blend(Quat::IDENTITY, program.sample(time), weight);
            assert!(
                pose.x.abs() < 1.0e-12 && pose.z.abs() < 1.0e-12,
                "{time}: {pose:?}"
            );
            assert!((pose.y - y).abs() < 2.0e-6, "{time}: y {} vs {y}", pose.y);
            assert!((pose.w - w).abs() < 2.0e-6, "{time}: w {} vs {w}", pose.w);
        }
        // The weight is linear in fixed seconds and saturates at 0.25 s.
        assert_eq!(transition_weight(0.125, 0.25), 0.5);
        assert_eq!(transition_weight(0.5, 0.25), 1.0);
    }
}
