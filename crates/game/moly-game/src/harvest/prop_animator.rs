//! The Animator of a harvest prop (the barrel, the toolbox, the treasure
//! boxes): its controller, run from the controller the package document
//! carries, and the view calls that drive it.
//!
//! Source calls: the driftage view's `OnPlayerActionStart(speed)` sets
//! `animator.speed = speed` and `SetBool("IsBreak", true)`; the toolbox's
//! sets the speed and `SetBool("IsOpen", true)`; the treasure box's sets the
//! speed, its `PlayDamageEffect` sets `SetBool("open", true)` and a box loaded
//! as harvested `SetBool("opened", true)`.
//!
//! The controller (`controllerData` of the prefab root's Animator in the
//! package document) is run as Mecanim runs a single-layer machine of bool
//! parameters: the default state plays from time 0; each frame the current
//! state's transitions are tested in their order (every condition true for
//! `If`, false for `IfNot`; with an exit time, the state's normalised time at
//! least the exit time), the first that holds starts; a fixed-duration
//! transition blends the two states' poses linearly over its seconds while
//! both states advance, then the destination is current. Time advances by the
//! frame's delta times the Animator speed times the state speed; a
//! non-looping state holds its last pose. A controller outside that shape
//! (another layer, AnyState transitions, non-bool parameters, blend trees, a
//! speed parameter, other condition modes, interruption) is refused by name
//! and the prop keeps its bound pose.
//!
//! Timing conventions (named): the frame that sets a parameter evaluates the
//! transition in the same frame and both states advance by that frame's
//! delta (Mecanim's sub-frame placement of a transition start was not read).
//! Poses are sampled on the glb clips the export wrote for the controller's
//! motions, through the prop's animation player.

use bevy::animation::graph::{AnimationGraph, AnimationGraphHandle, AnimationNodeIndex};
use bevy::gltf::Gltf;
use bevy::prelude::*;
use serde_json::Value;

use super::{HarvestDocs, HarvestGltfs, HarvestObject, HarvestRoot, HarvestViewNodes};

/// The views that own an Animator the flow drives.
pub(crate) fn has_prop_animator(class: &str) -> bool {
    matches!(
        class,
        "MysekaiAreadDriftageView" | "MysekaiAreaToolBoxView" | "MysekaiAreaTreasureBoxView"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    If,
    IfNot,
}

#[derive(Clone, Debug)]
struct TransitionDef {
    dest: usize,
    conditions: Vec<(Mode, usize)>,
    has_exit_time: bool,
    exit_time: f32,
    /// Seconds (fixed duration).
    duration: f32,
    /// Normalised start time of the destination.
    offset: f32,
}

#[derive(Clone, Debug)]
struct StateDef {
    name: String,
    /// Index into the glb's animations.
    animation: usize,
    length: f32,
    looping: bool,
    speed: f32,
    transitions: Vec<TransitionDef>,
}

/// A parsed controller.
#[derive(Clone, Debug)]
pub(crate) struct Controller {
    name: String,
    params: Vec<String>,
    defaults: Vec<bool>,
    states: Vec<StateDef>,
    default_state: usize,
}

fn f(value: &Value, key: &str) -> Result<f64, String> {
    value[key]
        .as_f64()
        .ok_or_else(|| format!("{key} is not a number"))
}

fn b(value: &Value, key: &str) -> Result<bool, String> {
    value[key]
        .as_bool()
        .ok_or_else(|| format!("{key} is not a bool"))
}

/// Read the prefab root's Animator controller from a package document.
pub(crate) fn parse_controller(document: &Value, scene: usize) -> Result<Controller, String> {
    let roots = document["roots"]
        .as_array()
        .ok_or("the document has no roots")?;
    let root = roots
        .iter()
        .find(|root| root["scene"].as_u64() == Some(scene as u64))
        .ok_or("no root for the prefab scene")?;
    let animators = root["animators"]
        .as_array()
        .ok_or("the prefab root has no animators")?;
    let [animator] = animators.as_slice() else {
        return Err(format!("{} animators on the prefab root", animators.len()));
    };
    let data = &animator["controllerData"];
    let name = animator["controller"]["source"]["name"]
        .as_str()
        .unwrap_or("?")
        .to_owned();
    let layers = data["layers"].as_array().ok_or("no layers")?;
    if layers.len() != 1 {
        return Err(format!("{} layers (one is run)", layers.len()));
    }
    let params_raw = data["parameters"].as_array().ok_or("no parameters")?;
    let mut params = Vec::new();
    let mut ids = Vec::new();
    for param in params_raw {
        if param["type"].as_i64() != Some(4) {
            return Err(format!("parameter {} is not a bool", param["name"]));
        }
        params.push(param["name"].as_str().ok_or("parameter name")?.to_owned());
        ids.push(param["id"].as_u64().ok_or("parameter id")?);
    }
    let defaults: Vec<bool> = data["defaultValues"]["m_BoolValues"]
        .as_array()
        .ok_or("no bool defaults")?
        .iter()
        .map(|v| v.as_bool().unwrap_or(false))
        .collect();
    if defaults.len() != params.len() {
        return Err("bool defaults do not match the parameters".into());
    }
    let clips = animator["clips"].as_array().ok_or("no animator clips")?;
    let clip_by_path = |path: &str| {
        clips
            .iter()
            .find(|clip| clip["source"]["source"]["pathId"].as_str() == Some(path))
    };
    let machines = data["stateMachines"]
        .as_array()
        .ok_or("no state machines")?;
    let [machine] = machines.as_slice() else {
        return Err(format!("{} state machines", machines.len()));
    };
    if !machine["anyStateTransitions"]
        .as_array()
        .is_some_and(|any| any.is_empty())
    {
        return Err("AnyState transitions are not run".into());
    }
    let raw_states = machine["states"].as_array().ok_or("no states")?;
    let mut states = Vec::new();
    for (position, state) in raw_states.iter().enumerate() {
        if state["index"].as_u64() != Some(position as u64) {
            return Err("state indices are not in order".into());
        }
        let raw = &state["raw"];
        if raw["m_SpeedParamID"].as_u64().unwrap_or(0) != 0 {
            return Err(format!("state {} has a speed parameter", state["name"]));
        }
        let motions = state["motions"].as_array().ok_or("no motions")?;
        let [motion] = motions.as_slice() else {
            return Err(format!(
                "state {} has {} motions",
                state["name"],
                motions.len()
            ));
        };
        if motion["raw"]["m_BlendType"].as_i64() != Some(0)
            || !motion["raw"]["m_ChildIndices"]
                .as_array()
                .is_some_and(|children| children.is_empty())
        {
            return Err(format!("state {} plays a blend tree", state["name"]));
        }
        let path = motion["clip"]["source"]["pathId"]
            .as_str()
            .ok_or("motion clip without path id")?;
        let clip = clip_by_path(path)
            .ok_or_else(|| format!("state {} motion names no exported clip", state["name"]))?;
        let animation = clip["animation"]
            .as_u64()
            .ok_or("exported clip without a glb animation index")? as usize;
        let length = f(clip, "stopTime")? as f32 - f(clip, "startTime").unwrap_or(0.0) as f32;
        let mut transitions = Vec::new();
        for transition in raw["m_TransitionConstantArray"]
            .as_array()
            .ok_or("no transitions")?
        {
            let t = &transition["data"];
            if t["m_InterruptionSource"].as_i64() != Some(0) {
                return Err("an interruptible transition is not run".into());
            }
            if !b(t, "m_HasFixedDuration")? {
                return Err("a normalised-duration transition is not run".into());
            }
            let dest = t["m_DestinationState"]
                .as_u64()
                .ok_or("transition destination")? as usize;
            if dest >= raw_states.len() {
                return Err(format!("transition destination {dest} is not a state"));
            }
            let mut conditions = Vec::new();
            for condition in t["m_ConditionConstantArray"]
                .as_array()
                .ok_or("no conditions")?
            {
                let c = &condition["data"];
                let mode = match c["m_ConditionMode"].as_i64() {
                    Some(1) => Mode::If,
                    Some(2) => Mode::IfNot,
                    other => return Err(format!("condition mode {other:?} is not run")),
                };
                let id = c["m_EventID"].as_u64().ok_or("condition parameter")?;
                let index = ids
                    .iter()
                    .position(|p| *p == id)
                    .ok_or("condition names no parameter")?;
                conditions.push((mode, index));
            }
            transitions.push(TransitionDef {
                dest,
                conditions,
                has_exit_time: b(t, "m_HasExitTime")?,
                exit_time: f(t, "m_ExitTime")? as f32,
                duration: f(t, "m_TransitionDuration")? as f32,
                offset: f(t, "m_TransitionOffset")? as f32,
            });
        }
        states.push(StateDef {
            name: state["name"].as_str().unwrap_or("?").to_owned(),
            animation,
            length,
            looping: state["loop"].as_bool().unwrap_or(false),
            speed: raw["m_Speed"].as_f64().unwrap_or(1.0) as f32,
            transitions,
        });
    }
    let default_state = machine["defaultState"].as_u64().ok_or("no default state")? as usize;
    if default_state >= states.len() {
        return Err("the default state is not a state".into());
    }
    Ok(Controller {
        name,
        params,
        defaults,
        states,
        default_state,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Blend {
    dest: usize,
    dest_time: f32,
    elapsed: f32,
    duration: f32,
}

/// The running machine (pure: the pose writer reads it).
#[derive(Clone, Debug)]
pub(crate) struct Machine {
    controller: Controller,
    params: Vec<bool>,
    pub(crate) speed: f32,
    current: usize,
    time: f32,
    blend: Option<Blend>,
    settled: bool,
}

/// What a frame changed (for the log).
#[derive(Debug, PartialEq)]
pub(crate) enum MachineEvent {
    Started {
        from: String,
        to: String,
        duration: f32,
    },
    Entered {
        state: String,
    },
}

impl Machine {
    pub(crate) fn new(controller: Controller) -> Self {
        Self {
            params: controller.defaults.clone(),
            current: controller.default_state,
            time: 0.0,
            blend: None,
            speed: 1.0,
            settled: false,
            controller,
        }
    }

    pub(crate) fn set_bool(&mut self, name: &str, value: bool) -> bool {
        match self.controller.params.iter().position(|p| p == name) {
            Some(index) => {
                self.params[index] = value;
                true
            }
            None => false,
        }
    }

    fn normalised(&self) -> f32 {
        let state = &self.controller.states[self.current];
        if state.length > 0.0 {
            self.time / state.length
        } else {
            1.0
        }
    }

    fn fires(&self, transition: &TransitionDef) -> bool {
        transition
            .conditions
            .iter()
            .all(|(mode, index)| match mode {
                Mode::If => self.params[*index],
                Mode::IfNot => !self.params[*index],
            })
            && (!transition.has_exit_time || self.normalised() >= transition.exit_time)
    }

    /// One frame.
    pub(crate) fn step(&mut self, dt: f32) -> Vec<MachineEvent> {
        let mut events = Vec::new();
        if self.blend.is_none() {
            let state = &self.controller.states[self.current];
            if let Some(transition) = state.transitions.iter().find(|t| self.fires(t)).cloned() {
                let dest = &self.controller.states[transition.dest];
                events.push(MachineEvent::Started {
                    from: state.name.clone(),
                    to: dest.name.clone(),
                    duration: transition.duration,
                });
                self.blend = Some(Blend {
                    dest: transition.dest,
                    dest_time: transition.offset * dest.length,
                    elapsed: 0.0,
                    duration: transition.duration,
                });
            }
        }
        let scale = dt * self.speed;
        self.time += scale * self.controller.states[self.current].speed;
        if let Some(blend) = self.blend.as_mut() {
            blend.dest_time += scale * self.controller.states[blend.dest].speed;
            blend.elapsed += scale;
            if blend.elapsed >= blend.duration {
                self.current = blend.dest;
                self.time = blend.dest_time;
                self.blend = None;
                events.push(MachineEvent::Entered {
                    state: self.controller.states[self.current].name.clone(),
                });
            }
        }
        events
    }

    /// The pose: (state, sample time, weight) for the one or two states
    /// that write it.
    pub(crate) fn pose(&self) -> Vec<(usize, f32, f32)> {
        let sample = |state: usize, time: f32| {
            let def = &self.controller.states[state];
            if def.looping && def.length > 0.0 {
                time.rem_euclid(def.length)
            } else {
                time.clamp(0.0, def.length)
            }
        };
        match self.blend {
            Some(blend) => {
                let w = if blend.duration > 0.0 {
                    (blend.elapsed / blend.duration).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                vec![
                    (self.current, sample(self.current, self.time), 1.0 - w),
                    (blend.dest, sample(blend.dest, blend.dest_time), w),
                ]
            }
            None => vec![(self.current, sample(self.current, self.time), 1.0)],
        }
    }

    pub(crate) fn state_name(&self) -> &str {
        &self.controller.states[self.current].name
    }

    fn state_label(&self, state: usize) -> &str {
        &self.controller.states[state].name
    }

    /// A non-looping, non-default state reaching its end on this frame's
    /// time (the one frame its last pose is first held), for the log.
    fn settling(&mut self) -> bool {
        let def = &self.controller.states[self.current];
        let at_end = self.blend.is_none()
            && self.current != self.controller.default_state
            && !def.looping
            && self.time >= def.length;
        let first = at_end && !self.settled;
        self.settled = at_end;
        first
    }
}

/// A prop's Animator: the machine and its pose writer.
#[derive(Component)]
pub(crate) struct PropAnimator {
    pub(crate) machine: Machine,
    player: Entity,
    nodes: Vec<AnimationNodeIndex>,
}

/// The controller of this prop was refused (named once).
#[derive(Component)]
pub(crate) struct PropAnimatorRefused;

/// A view call on a prop's Animator.
#[derive(Clone, Copy, Debug)]
pub(crate) enum PropCall {
    Speed(f32),
    SetBool(&'static str, bool),
}

/// Calls the flow made this frame, applied before the Animator evaluates.
#[derive(Resource, Default)]
pub(crate) struct PropAnimatorCalls(pub(crate) Vec<(Entity, PropCall)>);

/// Update: build each prop's Animator once its scene expanded.
#[allow(clippy::type_complexity)]
pub(crate) fn bind(
    mut commands: Commands,
    roots: Query<
        (Entity, &HarvestObject),
        (
            With<HarvestRoot>,
            With<HarvestViewNodes>,
            Without<PropAnimator>,
            Without<PropAnimatorRefused>,
        ),
    >,
    children: Query<&Children>,
    players: Query<(), With<AnimationPlayer>>,
    docs: Res<HarvestDocs>,
    json: Res<Assets<moly_assets::json::JsonAsset>>,
    glbs: Res<HarvestGltfs>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for (root, object) in &roots {
        if !has_prop_animator(object.class) {
            continue;
        }
        let (Some(doc), Some(gltf)) = (
            docs.0.get(&object.package).and_then(|h| json.get(h)),
            glbs.get(&object.package).and_then(|h| gltfs.get(h)),
        ) else {
            continue;
        };
        let refuse = |commands: &mut Commands, reason: String| {
            warn!(
                "[harvest-animator] {}#{} ({}): Animator refused, the prop keeps its bound pose: {reason}",
                object.leaf, object.fixture_id, object.class
            );
            commands.entity(root).insert(PropAnimatorRefused);
        };
        let value: Value = match serde_json::from_str(&doc.0) {
            Ok(value) => value,
            Err(error) => {
                refuse(&mut commands, format!("document is not JSON: {error}"));
                continue;
            }
        };
        let scene = super::prefab_scene_index(&doc.0, &object.leaf);
        let controller = match parse_controller(&value, scene) {
            Ok(controller) => controller,
            Err(reason) => {
                refuse(&mut commands, reason);
                continue;
            }
        };
        let mut stack = vec![root];
        let mut player = None;
        while let Some(entity) = stack.pop() {
            if players.get(entity).is_ok() {
                player = Some(entity);
                break;
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        let Some(player) = player else {
            refuse(
                &mut commands,
                "the scene has no animation player for the controller's clips".into(),
            );
            continue;
        };
        let mut graph = AnimationGraph::new();
        let mut nodes = Vec::new();
        let mut missing = None;
        for state in &controller.states {
            match gltf.animations.get(state.animation) {
                Some(clip) => nodes.push(graph.add_clip(clip.clone(), 1.0, graph.root)),
                None => {
                    missing = Some(state.name.clone());
                    break;
                }
            }
        }
        if let Some(state) = missing {
            refuse(
                &mut commands,
                format!("the glb has no animation for state {state}"),
            );
            continue;
        }
        let graph = graphs.add(graph);
        commands.entity(player).insert(AnimationGraphHandle(graph));
        info!(
            "[harvest-animator] {}#{} Animator bound: controller {} params {:?} (defaults {:?}), states {:?}, default {}",
            object.leaf,
            object.fixture_id,
            controller.name,
            controller.params,
            controller.defaults,
            controller
                .states
                .iter()
                .map(|s| format!(
                    "{} ({:.4} s, {} transitions)",
                    s.name,
                    s.length,
                    s.transitions.len()
                ))
                .collect::<Vec<_>>(),
            controller.states[controller.default_state].name
        );
        commands.entity(root).insert(PropAnimator {
            machine: Machine::new(controller),
            player,
            nodes,
        });
    }
}

/// Update, after the hits: apply this frame's calls, step every Animator and
/// write its pose.
#[allow(clippy::type_complexity)]
pub(crate) fn advance(
    time: Res<Time>,
    mut calls: ResMut<PropAnimatorCalls>,
    mut animators: Query<(Entity, &HarvestObject, &mut PropAnimator)>,
    binding: Query<
        (),
        (
            With<HarvestRoot>,
            Without<PropAnimator>,
            Without<PropAnimatorRefused>,
        ),
    >,
    mut players: Query<&mut AnimationPlayer>,
) {
    let dt = time.delta_secs();
    let mut later = Vec::new();
    for (entity, call) in std::mem::take(&mut calls.0) {
        let Ok((_, object, mut animator)) = animators.get_mut(entity) else {
            if binding.get(entity).is_ok() {
                // The scene has not expanded yet (a loaded harvested box
                // calls at placement): the call waits for the Animator.
                later.push((entity, call));
            } else {
                warn!("[harvest-animator] a view call names a prop without an Animator: {call:?}");
            }
            continue;
        };
        match call {
            PropCall::Speed(speed) => animator.machine.speed = speed,
            PropCall::SetBool(name, value) => {
                if !animator.machine.set_bool(name, value) {
                    warn!(
                        "[harvest-animator] {}#{}: SetBool({name}) names no parameter of the controller",
                        object.leaf, object.fixture_id
                    );
                }
            }
        }
        info!(
            "[harvest-animator] {}#{} {call:?} in state {}",
            object.leaf,
            object.fixture_id,
            animator.machine.state_name()
        );
    }
    calls.0 = later;
    for (_, object, mut animator) in &mut animators {
        let events = animator.machine.step(dt);
        let Ok(mut player) = players.get_mut(animator.player) else {
            continue;
        };
        let pose = animator.machine.pose();
        for (state, node) in animator.nodes.iter().enumerate() {
            match pose.iter().find(|(s, _, _)| *s == state) {
                Some(&(_, sample, weight)) => {
                    player
                        .play(*node)
                        .pause()
                        .set_weight(weight)
                        .seek_to(sample);
                }
                None => {
                    if player.animation(*node).is_some() {
                        player.stop(*node);
                    }
                }
            }
        }
        if !events.is_empty() || animator.machine.settling() {
            // What the animation player now holds (the pose's output state).
            let held: Vec<String> = animator
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(state, node)| {
                    player.animation(*node).map(|active| {
                        format!(
                            "{} t {:.4} w {:.3}",
                            animator.machine.state_label(state),
                            active.seek_time(),
                            active.weight()
                        )
                    })
                })
                .collect();
            for event in &events {
                info!(
                    "[harvest-animator] {}#{} {event:?} (speed {:.2}); player holds {held:?}",
                    object.leaf, object.fixture_id, animator.machine.speed
                );
            }
            if events.is_empty() {
                info!(
                    "[harvest-animator] {}#{} held pose: player holds {held:?}",
                    object.leaf, object.fixture_id
                );
            }
        }
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// The barrel controller as its package document states it: state 0
    /// (one frame) goes to state 1 (1.4167 s) on `IsBreak`, no exit time,
    /// 0.25 s fixed; a speed of 1.5 scales every clock.
    fn barrel() -> Controller {
        Controller {
            name: "barrel".into(),
            params: vec!["IsBreak".into()],
            defaults: vec![false],
            states: vec![
                StateDef {
                    name: "idle".into(),
                    animation: 0,
                    length: 1.0 / 60.0,
                    looping: false,
                    speed: 1.0,
                    transitions: vec![TransitionDef {
                        dest: 1,
                        conditions: vec![(Mode::If, 0)],
                        has_exit_time: false,
                        exit_time: 0.0,
                        duration: 0.25,
                        offset: 0.0,
                    }],
                },
                StateDef {
                    name: "break".into(),
                    animation: 1,
                    length: 1.4166667,
                    looping: false,
                    speed: 1.0,
                    transitions: vec![],
                },
            ],
            default_state: 0,
        }
    }

    #[test]
    fn barrel_break_blend_and_speed() {
        let mut machine = Machine::new(barrel());
        // Unset: nothing starts, the one-frame pose holds.
        assert!(machine.step(0.125).is_empty());
        assert_eq!(machine.pose(), vec![(0, 1.0 / 60.0, 1.0)]);
        machine.speed = 1.5;
        assert!(machine.set_bool("IsBreak", true));
        // The blend starts this frame; 0.125 s at speed 1.5 is 0.1875 of the
        // 0.25 s blend (weight 0.75).
        let events = machine.step(0.125);
        assert!(matches!(events[0], MachineEvent::Started { duration, .. } if duration == 0.25));
        assert_eq!(
            machine.pose(),
            vec![(0, 1.0 / 60.0, 0.25), (1, 0.1875, 0.75)]
        );
        // The next frame completes it: state 1 at 0.375.
        let events = machine.step(0.125);
        assert!(matches!(&events[0], MachineEvent::Entered { state } if state == "break"));
        assert_eq!(machine.pose(), vec![(1, 0.375, 1.0)]);
        // A non-looping state holds its last pose.
        machine.step(4.0);
        assert_eq!(machine.pose(), vec![(1, 1.4166667, 1.0)]);
    }

    /// The treasure controller's exit-time rule: `open` with exit time 1.0 on
    /// a one-frame state fires once that frame has played; state 1 then goes
    /// to state 2 by exit time alone.
    #[test]
    fn treasure_open_by_exit_time() {
        let state = |name: &str, length: f32, transitions| StateDef {
            name: name.into(),
            animation: 0,
            length,
            looping: false,
            speed: 1.0,
            transitions,
        };
        let t = |dest, conditions, exit_time, duration| TransitionDef {
            dest,
            conditions,
            has_exit_time: true,
            exit_time,
            duration,
            offset: 0.0,
        };
        let controller = Controller {
            name: "treasure".into(),
            params: vec!["open".into(), "opened".into()],
            defaults: vec![false, false],
            states: vec![
                state(
                    "closed",
                    1.0 / 60.0,
                    vec![
                        t(1, vec![(Mode::If, 0)], 1.0, 0.0),
                        t(2, vec![(Mode::If, 1)], 0.0, 0.25),
                    ],
                ),
                state("opening", 0.75, vec![t(2, vec![], 1.0, 0.0)]),
                state("open", 1.0 / 60.0, vec![]),
            ],
            default_state: 0,
        };
        let mut machine = Machine::new(controller);
        machine.step(0.125);
        machine.set_bool("open", true);
        let events = machine.step(0.125);
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(machine.state_name(), "opening");
        machine.step(0.5);
        machine.step(0.125);
        assert_eq!(machine.state_name(), "opening");
        let events = machine.step(0.125);
        assert!(matches!(&events[1], MachineEvent::Entered { state } if state == "open"));
    }
}
