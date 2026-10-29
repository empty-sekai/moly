//! Source-bound animation lanes for fixture controllers; no named-node fallback.
//!
//! A lane is one authored curve set on one source object: a Transform's Euler
//! rotation, a Transform's local scale, or a behaviour's `m_Enabled` flag.
//! Enabling a behaviour runs that behaviour's own lifecycle (see [`tween`] and
//! [`dolly`]); the clip only decides when it is enabled.
//!
//! Fixture curves are authored in Unity's left-handed basis while the exported
//! fixture GLB uses `moly-rh-y-up-reflect-x-v1`. Convert a sampled quaternion
//! once at this raw-curve boundary; never infer the basis from a transform
//! determinant. Scale is unchanged by the reflection.

use std::{collections::HashMap, sync::Arc};

use bevy::{animation::AnimatedBy, math::Affine3A, prelude::*};
use moly_assets::source_navigation::SourceObjectIdentity;
use serde_json::Value;

use crate::{fixture_activity_timeline::SourceAssetId, source_curve::Curve};
use super::{array, dolly, identity, one, tween, Definition, Playback};

/// CRC-32 of the property name `m_Enabled`, the attribute id clips use for it.
const ENABLED_ATTRIBUTE: u64 = 3_305_885_265;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct Target {
    file: String,
    game_object: i64,
    component: i64,
}

impl Target {
    fn matches(&self, identity: &SourceObjectIdentity, transform: bool) -> bool {
        identity.file == self.file && identity.game_object == self.game_object
            && identity.components.contains(&self.component)
            && (!transform || identity.transform == self.component)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum Kind {
    Euler,
    Scale,
    Enabled,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct Key {
    target: Target,
    kind: Kind,
}

impl Key {
    fn matches(&self, identity: &SourceObjectIdentity) -> bool {
        self.target.matches(identity, self.kind != Kind::Enabled)
    }
}

#[derive(Clone)]
struct Lane {
    key: Key,
    curves: Vec<Curve>,
}

/// Unity's `Quaternion.Euler`: degrees to f32 radians, then qY*qX*qZ. The
/// result is already converted to the exported node basis.
pub(super) fn euler_rotation(degrees: Vec3) -> Quat {
    const DEG_TO_RAD: f32 = f32::from_bits(0x3c8efa35);
    let authored = Quat::from_rotation_y(degrees.y * DEG_TO_RAD)
        * Quat::from_rotation_x(degrees.x * DEG_TO_RAD)
        * Quat::from_rotation_z(degrees.z * DEG_TO_RAD);
    moly_assets::coordinates::source_rotation(authored)
}

impl Lane {
    /// Rotation lanes sample as a quaternion, scale lanes as XYZ, enable lanes
    /// as their single float. The unused components are zero.
    fn sample(&self, time: f64) -> Vec4 {
        let time = time as f32;
        let at = |index: usize| self.curves[index].sample(time);
        match self.key.kind {
            Kind::Euler => Vec4::from(euler_rotation(Vec3::new(at(0), at(1), at(2)))),
            Kind::Scale => Vec4::new(at(0), at(1), at(2), 0.0),
            Kind::Enabled => Vec4::new(at(0), 0.0, 0.0, 0.0),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Program {
    lanes: Vec<Lane>,
}

impl Program {
    pub(super) fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    fn sample(&self, key: &Key, time: f64) -> Option<Vec4> {
        self.lanes.iter().find(|lane| &lane.key == key).map(|lane| lane.sample(time))
    }
}

/// What a source clip's `m_Enabled` lane switches on.
#[derive(Clone)]
enum Behaviour {
    Tween(Arc<tween::Spec>),
    Cart { cart: dolly::Cart, path: Arc<dolly::Path>, path_target: Target },
}

/// The lanes of one controller's On and Off clips and the behaviours they drive.
pub(super) struct Plan {
    keys: Vec<Key>,
    behaviours: HashMap<Target, Behaviour>,
}

impl Plan {
    pub(super) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

fn scalar(value: &Value) -> Result<f32, String> {
    value.as_f64().map(|value| value as f32).filter(|value| value.is_finite())
        .ok_or_else(|| "source curve contains a non-finite scalar".into())
}

pub(super) fn curve(value: &Value) -> Result<Curve, String> {
    let points = array(value, "points")?;
    if points.is_empty() { return Err("source curve has no source points".into()); }
    match value["kind"].as_str() {
        Some("const") => {
            let point = one(points, "constant source key")?.as_array()
                .filter(|point| point.len() == 2).ok_or("invalid constant source point")?;
            if scalar(&point[0])? != 0.0 { return Err("constant source time is not zero".into()); }
            Ok(Curve::Const(scalar(&point[1])?))
        }
        Some("cubic") => {
            let mut keys = Vec::with_capacity(points.len());
            for point in points {
                let point = point.as_array().filter(|point| point.len() == 2)
                    .ok_or("invalid cubic source point")?;
                let time = scalar(&point[0])?;
                let coefficients = point[1].as_array().filter(|values| values.len() == 4)
                    .ok_or("source cubic has no four original coefficients")?;
                if keys.last().is_some_and(|(previous, _)| *previous >= time) {
                    return Err("source cubic key times are not strictly increasing".into());
                }
                keys.push((time, [scalar(&coefficients[0])?, scalar(&coefficients[1])?,
                                  scalar(&coefficients[2])?, scalar(&coefficients[3])?]));
            }
            Ok(Curve::Cubic(keys))
        }
        _ => Err("source curve kind has no coefficient-preserving consumer".into()),
    }
}

pub(super) fn program(clip: &Value, animator: &SourceAssetId) -> Result<Program, String> {
    let mut grouped: Vec<(Key, Vec<Option<Curve>>)> = Vec::new();
    for value in array(clip, "curves")? {
        let binding = &value["binding"];
        let (kind, class, width) = match (binding["typeId"].as_u64(), binding["attribute"].as_u64()) {
            (Some(4), Some(4)) => (Kind::Euler, "Transform", 3),
            (Some(4), Some(3)) => (Kind::Scale, "Transform", 3),
            (Some(114), Some(ENABLED_ATTRIBUTE)) => (Kind::Enabled, "MonoBehaviour", 1),
            _ => return Err("this animated output has no curve consumer".into()),
        };
        let component_index = binding["component"].as_u64()
            .filter(|index| *index < width as u64).ok_or("invalid curve component index")? as usize;
        let output = one(array(value, "targets")?, "curve source target")?;
        if identity(&output["animator"])? != *animator {
            return Err("curve target belongs to a different source Animator".into());
        }
        let game_object = identity(&output["gameObject"])?;
        let component = one(array(output, "components")?, "curve target component")?;
        if component["class"].as_str() != Some(class) {
            return Err(format!("curve target component is not a {class}"));
        }
        let component = identity(component)?;
        if game_object.file != component.file || component.file != animator.file {
            return Err("curve target crosses serialized-file identity domains".into());
        }
        let key = Key {
            target: Target {
                file: component.file,
                game_object: game_object.path_id.parse().map_err(|_| "invalid curve GameObject identity")?,
                component: component.path_id.parse().map_err(|_| "invalid curve component identity")?,
            },
            kind,
        };
        let position = match grouped.iter().position(|(existing, _)| existing == &key) {
            Some(position) => position,
            None => {
                grouped.push((key, vec![None; width]));
                grouped.len() - 1
            }
        };
        let slot = &mut grouped[position].1[component_index];
        if slot.is_some() { return Err("duplicate curve component binding".into()); }
        *slot = Some(curve(value)?);
    }
    let mut lanes = Vec::with_capacity(grouped.len());
    for (key, curves) in grouped {
        let curves = curves.into_iter().collect::<Option<Vec<_>>>()
            .ok_or("source curve component set is incomplete")?;
        lanes.push(Lane { key, curves });
    }
    Ok(Program { lanes })
}

fn identity_path(reference: &Value) -> Result<Option<i64>, String> {
    if reference["m_FileID"].as_i64() != Some(0) {
        return Err("serialized reference points into another file".into());
    }
    match &reference["m_PathID"] {
        Value::String(id) => id.parse::<i64>().map(|id| (id != 0).then_some(id))
            .map_err(|_| "serialized reference path id is not i64".into()),
        Value::Number(id) => Ok(id.as_i64().filter(|id| *id != 0)),
        _ => Err("serialized reference has no path id".into()),
    }
}

/// The behaviours of a source Animator's hierarchy that an `m_Enabled` lane
/// can switch.
fn behaviours(animator: &Value) -> Result<HashMap<Target, Behaviour>, String> {
    let nodes = array(animator, "nodes")?;
    let mut paths: HashMap<i64, (Target, Arc<dolly::Path>)> = HashMap::new();
    let mut found = Vec::new();
    for node in nodes {
        let game_object = identity(&node["gameObject"])?;
        let mut transform = None;
        let mut scripts = Vec::new();
        for component in array(node, "components")? {
            let source = identity(&component["source"])?;
            match component["source"]["class"].as_str() {
                Some("Transform") => transform = Some(source),
                Some("MonoBehaviour") => scripts.push((component, source)),
                _ => {}
            }
        }
        let game_object_id = game_object.path_id.parse::<i64>().map_err(|_| "invalid node GameObject")?;
        let target = |source: &SourceAssetId| -> Result<Target, String> {
            if source.file != game_object.file { return Err("node component crosses serialized-file identity domains".into()); }
            Ok(Target {
                file: source.file.clone(),
                game_object: game_object_id,
                component: source.path_id.parse().map_err(|_| "invalid node component identity")?,
            })
        };
        for (component, source) in scripts {
            match component["source"]["script"].as_str() {
                Some("CinemachinePath") => {
                    let transform = transform.as_ref().ok_or("path node has no Transform")?;
                    let path_id = source.path_id.parse::<i64>().map_err(|_| "invalid path identity")?;
                    paths.insert(path_id, (target(transform)?, Arc::new(dolly::Path::parse(&component["fields"])?)));
                }
                Some("CinemachineDollyCart") => {
                    found.push((target(&source)?, component["fields"].clone(), true));
                }
                Some("TweenRotation") => {
                    found.push((target(&source)?, component["fields"].clone(), false));
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    for (target, fields, cart) in found {
        let behaviour = if cart {
            let path_id = identity_path(&fields["m_Path"])?.ok_or("dolly cart has no path")?;
            let (path_target, path) = paths.get(&path_id).ok_or("dolly cart path is not in the Animator hierarchy")?;
            Behaviour::Cart { cart: dolly::Cart::parse(&fields)?, path: path.clone(), path_target: path_target.clone() }
        } else {
            Behaviour::Tween(Arc::new(tween::Spec::parse(&fields)?))
        };
        out.insert(target, behaviour);
    }
    Ok(out)
}

/// Every lane the controller's clips animate, with each `m_Enabled` lane
/// resolved to the behaviour it switches.
pub(super) fn plan(on: &Program, off: &Program, animator: &Value) -> Result<Plan, String> {
    let mut keys: Vec<Key> = Vec::new();
    for lane in on.lanes.iter().chain(&off.lanes) {
        if !keys.contains(&lane.key) { keys.push(lane.key.clone()); }
    }
    let behaviours = if keys.iter().any(|key| key.kind == Kind::Enabled) { behaviours(animator)? } else { HashMap::new() };
    for key in keys.iter().filter(|key| key.kind == Kind::Enabled) {
        if !behaviours.contains_key(&key.target) {
            return Err("m_Enabled lane target is not a behaviour with a source consumer".into());
        }
    }
    Ok(Plan { keys, behaviours })
}

struct Slot {
    key: Key,
    entity: Entity,
    rest: Vec4,
    behaviour: Option<usize>,
}

enum State {
    Tween(tween::State),
    Cart { state: dolly::State, path: Entity },
}

struct Runtime {
    entity: Entity,
    enabled: bool,
    state: State,
}

pub(super) struct Binding {
    view: Entity,
    slots: Vec<Slot>,
    behaviours: Vec<Runtime>,
}

fn find(world: &World, view: Entity, label: &str, matches: impl Fn(&SourceObjectIdentity) -> bool) -> Result<Entity, String> {
    let mut stack = vec![view];
    let mut found = Vec::new();
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) { stack.extend(children.iter()); }
        if world.get::<SourceObjectIdentity>(entity).is_some_and(&matches) { found.push(entity); }
    }
    match found.as_slice() {
        [entity] => Ok(*entity),
        _ => Err(format!("{label} has {} actual source-identity matches under FixtureView", found.len())),
    }
}

fn depth(world: &World, mut entity: Entity, view: Entity) -> usize {
    let mut depth = 0;
    while entity != view {
        let Some(parent) = world.get::<ChildOf>(entity) else { break };
        entity = parent.parent();
        depth += 1;
    }
    depth
}

pub(super) fn bind(world: &World, view: Entity, definition: &Definition) -> Result<Option<Binding>, String> {
    let plan = &definition.lanes;
    if plan.is_empty() { return Ok(None); }
    let receiver = world.get::<SourceObjectIdentity>(view)
        .ok_or("lane FixtureView requires lossless sourceObject extras; re-export this fixture")?;
    if receiver.file != definition.source_file || receiver.game_object != definition.view_game_object
        || !receiver.components.contains(&definition.view_component)
        || !receiver.components.contains(&definition.animator_component)
    {
        return Err("lane receiver is not the exact source FixtureView/Animator pair".into());
    }
    let mut resolved = Vec::with_capacity(plan.keys.len());
    for key in &plan.keys {
        let entity = find(world, view, "lane target", |id| key.matches(id))?;
        if let Some(by) = world.get::<AnimatedBy>(entity) {
            if world.get::<AnimationPlayer>(by.0).is_some_and(|player| player.playing_animations().next().is_some()) {
                return Err("lane target is already owned by an active animation player".into());
            }
        }
        resolved.push((key, entity));
    }
    // Carts write world poses: a parent is updated before its children.
    resolved.sort_by_key(|(_, entity)| depth(world, *entity, view));
    let mut slots = Vec::with_capacity(resolved.len());
    let mut behaviours = Vec::new();
    for (key, entity) in resolved {
        let transform = world.get::<Transform>(entity).ok_or("lane target has no actual Transform")?;
        let mut behaviour = None;
        let rest = match key.kind {
            Kind::Euler => Vec4::from(transform.rotation),
            Kind::Scale => transform.scale.extend(0.0),
            Kind::Enabled => {
                let (state, enabled) = match &plan.behaviours[&key.target] {
                    Behaviour::Tween(spec) => (State::Tween(tween::State::new(spec.clone())), spec.enabled()),
                    Behaviour::Cart { cart, path, path_target } => {
                        let path_entity = find(world, view, "dolly path", |id| path_target.matches(id, true))?;
                        (State::Cart { state: dolly::State::new(cart, path.clone()), path: path_entity }, cart.enabled())
                    }
                };
                if enabled {
                    return Err("a behaviour serialized as enabled needs its initial OnEnable lifecycle".into());
                }
                behaviour = Some(behaviours.len());
                behaviours.push(Runtime { entity, enabled, state });
                Vec4::ZERO
            }
        };
        slots.push(Slot { key: key.clone(), entity, rest, behaviour });
    }
    Ok(Some(Binding { view, slots, behaviours }))
}

/// The local poses a lease restores when the instance is released.
pub(super) fn snapshot(world: &World, binding: &Binding) -> Vec<(Entity, Transform)> {
    binding.slots.iter()
        .filter_map(|slot| world.get::<Transform>(slot.entity).map(|transform| (slot.entity, *transform)))
        .collect()
}

fn world_affine(world: &World, entity: Entity, view: Entity) -> Result<Affine3A, String> {
    let mut chain = Vec::new();
    let mut current = entity;
    while current != view {
        chain.push(current);
        current = world.get::<ChildOf>(current).ok_or("lane node is not under its FixtureView")?.parent();
    }
    let mut affine = world.get::<GlobalTransform>(view).ok_or("FixtureView has no world transform")?.affine();
    for node in chain.into_iter().rev() {
        affine = affine * world.get::<Transform>(node).ok_or("lane node has no Transform")?.compute_affine();
    }
    Ok(affine)
}

/// Runs the enabled behaviours for one update, before the Animator writes the
/// next `m_Enabled` values.
pub(super) fn step(world: &mut World, binding: &mut Binding, delta: f32) -> Result<(), String> {
    let view = binding.view;
    for runtime in &mut binding.behaviours {
        match &mut runtime.state {
            State::Tween(state) => {
                if let Some(rotation) = state.step(f64::from(delta)) {
                    world.get_mut::<Transform>(runtime.entity).ok_or("tween target has no Transform")?.rotation = rotation;
                }
            }
            State::Cart { state, path } => {
                if !runtime.enabled { continue; }
                let path_world = world_affine(world, *path, view)?;
                let (position, rotation) = state.step(delta, &path_world)?;
                let parent = world.get::<ChildOf>(runtime.entity).ok_or("dolly cart has no parent")?.parent();
                let parent_world = world_affine(world, parent, view)?;
                let (_, parent_rotation, _) = parent_world.to_scale_rotation_translation();
                let mut transform = world.get_mut::<Transform>(runtime.entity).ok_or("dolly cart has no Transform")?;
                transform.translation = parent_world.inverse().transform_point3(position);
                transform.rotation = (parent_rotation.inverse() * rotation).normalize();
            }
        }
    }
    Ok(())
}

fn mix(kind: Kind, outgoing: Vec4, incoming: Vec4, alpha: f32) -> Vec4 {
    match kind {
        // Source rotation accumulation uses sign-corrected weighted sums
        // followed by normalization. Bevy's default Quat animation mixer uses
        // slerp instead.
        Kind::Euler => {
            let sign = if outgoing.dot(incoming) < 0.0 { -1.0 } else { 1.0 };
            (outgoing * (1.0 - alpha) + incoming * (alpha * sign)).normalize()
        }
        Kind::Scale | Kind::Enabled => outgoing * (1.0 - alpha) + incoming * alpha,
    }
}

pub(super) fn apply(world: &mut World, binding: &mut Binding, playback: &Playback) -> Result<(), String> {
    let alpha = if playback.program.transition_seconds > 0.0 {
        (playback.transition_elapsed / playback.program.transition_seconds).clamp(0.0, 1.0) as f32
    } else { 1.0 };
    let incoming_time = playback.elapsed.min(playback.program.duration);
    for index in 0..binding.slots.len() {
        let slot = &binding.slots[index];
        if !world.get::<SourceObjectIdentity>(slot.entity).is_some_and(|id| slot.key.matches(id)) {
            return Err("actual lane target disappeared or changed source identity".into());
        }
        // A clip that does not animate a property leaves it at its rest value.
        let incoming = playback.program.lanes.sample(&slot.key, incoming_time).unwrap_or(slot.rest);
        let outgoing = playback.previous.as_ref().and_then(|previous| {
            previous.program.lanes.sample(&slot.key, previous.elapsed.min(previous.program.duration))
        }).unwrap_or(slot.rest);
        let sampled = mix(slot.key.kind, outgoing, incoming, alpha);
        match slot.key.kind {
            Kind::Euler => {
                let mut transform = world.get_mut::<Transform>(slot.entity).ok_or("actual lane Transform disappeared")?;
                let rotation = Quat::from_vec4(sampled);
                // A clamped clip keeps its last pose. Avoid marking the whole
                // hierarchy changed every idle frame when it is already installed.
                if transform.rotation != rotation { transform.rotation = rotation; }
            }
            Kind::Scale => {
                let mut transform = world.get_mut::<Transform>(slot.entity).ok_or("actual lane Transform disappeared")?;
                let scale = sampled.truncate();
                if transform.scale != scale { transform.scale = scale; }
            }
            Kind::Enabled => {
                let runtime = &mut binding.behaviours[slot.behaviour.expect("enable lane has a behaviour")];
                let enabled = sampled.x != 0.0;
                if enabled == runtime.enabled { continue; }
                runtime.enabled = enabled;
                match &mut runtime.state {
                    State::Tween(state) => {
                        if enabled {
                            let rotation = state.enable();
                            world.get_mut::<Transform>(runtime.entity).ok_or("tween target has no Transform")?.rotation = rotation;
                        } else {
                            state.disable();
                        }
                    }
                    State::Cart { .. } => {}
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_euler_crosses_the_source_boundary_once() {
        let lane = Lane {
            key: Key { target: Target { file: "test".into(), game_object: 1, component: 2 }, kind: Kind::Euler },
            curves: vec![Curve::Const(17.), Curve::Const(53.), Curve::Const(-31.)],
        };
        let radians = f32::from_bits(0x3c8efa35);
        let authored = Quat::from_rotation_y(53. * radians)
            * Quat::from_rotation_x(17. * radians)
            * Quat::from_rotation_z(-31. * radians);
        let canonical = Quat::from_vec4(lane.sample(0.37));
        let expected = moly_assets::coordinates::source_rotation(authored);
        assert!(canonical.dot(expected).abs() > 1.0 - 1.0e-5);
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let mapped = canonical * moly_assets::coordinates::source_position(axis);
            let wanted = moly_assets::coordinates::source_position(authored * axis);
            assert!(mapped.distance(wanted) < 1.0e-5);
        }
    }

    #[test]
    fn reflection_is_involution_for_euler_quaternion() {
        let q = Quat::from_euler(EulerRot::YXZ, 0.4, -0.2, 0.8);
        let reflected = moly_assets::coordinates::source_rotation(q);
        let restored = moly_assets::coordinates::source_rotation(reflected);
        assert!(q.dot(restored).abs() > 1.0 - 1.0e-5);
    }
}
