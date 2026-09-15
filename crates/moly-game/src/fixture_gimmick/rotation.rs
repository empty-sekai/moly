//! Source-bound single-Transform Euler sampling; no named-node fallback.
//!
//! Fixture GLBs retain authored Unity local TRS. Unlike SD/avatar exports,
//! these nodes must not receive an additional X-axis quaternion reflection.

use bevy::{animation::AnimatedBy, prelude::*};
use moly_assets::source_navigation::SourceObjectIdentity;
use serde_json::Value;

use crate::{fixture_activity_timeline::SourceAssetId, source_curve::Curve};
use super::{array, identity, one, Definition, Playback};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Target {
    file: String,
    game_object: i64,
    transform: i64,
}

impl Target {
    fn matches(&self, identity: &SourceObjectIdentity) -> bool {
        identity.file == self.file && identity.game_object == self.game_object
            && identity.transform == self.transform && identity.components.contains(&self.transform)
    }
}

#[derive(Clone)]
pub(super) struct Program {
    pub target: Target,
    axes: [Curve; 3],
}

impl Program {
    fn sample(&self, time: f64) -> Quat {
        // CN libunity: degrees -> f32 radians, Euler selector 4 -> qY*qX*qZ.
        // Preserve the cubic angle itself, including the >180-degree range.
        const DEG_TO_RAD: f32 = f32::from_bits(0x3c8efa35);
        let x = self.axes[0].sample(time as f32) * DEG_TO_RAD;
        let y = self.axes[1].sample(time as f32) * DEG_TO_RAD;
        let z = self.axes[2].sample(time as f32) * DEG_TO_RAD;
        Quat::from_rotation_y(y) * Quat::from_rotation_x(x) * Quat::from_rotation_z(z)
    }
}

pub(super) struct Binding {
    entity: Entity,
    target: Target,
    rest: Quat,
}

fn scalar(value: &Value) -> Result<f32, String> {
    value.as_f64().map(|value| value as f32).filter(|value| value.is_finite())
        .ok_or_else(|| "Euler curve contains a non-finite scalar".into())
}

fn curve(value: &Value) -> Result<Curve, String> {
    let points = array(value, "points")?;
    if points.is_empty() { return Err("Euler curve has no source points".into()); }
    match value["kind"].as_str() {
        Some("const") => {
            let point = one(points, "constant Euler key")?.as_array()
                .filter(|point| point.len() == 2).ok_or("invalid constant Euler point")?;
            if scalar(&point[0])? != 0.0 { return Err("constant Euler time is not zero".into()); }
            Ok(Curve::Const(scalar(&point[1])?))
        }
        Some("cubic") => {
            let mut keys = Vec::with_capacity(points.len());
            for point in points {
                let point = point.as_array().filter(|point| point.len() == 2)
                    .ok_or("invalid cubic Euler point")?;
                let time = scalar(&point[0])?;
                let coefficients = point[1].as_array().filter(|values| values.len() == 4)
                    .ok_or("Euler cubic has no four original coefficients")?;
                if keys.last().is_some_and(|(previous, _)| *previous >= time) {
                    return Err("Euler cubic key times are not strictly increasing".into());
                }
                keys.push((time, [scalar(&coefficients[0])?, scalar(&coefficients[1])?,
                                  scalar(&coefficients[2])?, scalar(&coefficients[3])?]));
            }
            Ok(Curve::Cubic(keys))
        }
        _ => Err("Euler curve kind has no coefficient-preserving consumer".into()),
    }
}

pub(super) fn program(clip: &Value, animator: &SourceAssetId) -> Result<Option<Program>, String> {
    let curves = array(clip, "curves")?;
    if curves.is_empty() { return Ok(None); }
    if curves.len() != 3 { return Err("Euler lane requires exactly one complete XYZ Transform binding".into()); }
    let mut target = None;
    let mut axes: [Option<Curve>; 3] = std::array::from_fn(|_| None);
    for value in curves {
        let binding = &value["binding"];
        if binding["typeId"].as_u64() != Some(4) || binding["attribute"].as_u64() != Some(4) {
            return Err("non-Euler Transform output needs another curve consumer".into());
        }
        let axis = binding["component"].as_u64().filter(|axis| *axis < 3)
            .ok_or("invalid Euler component index")? as usize;
        if axes[axis].is_some() { return Err("duplicate Euler component binding".into()); }
        let output = one(array(value, "targets")?, "Euler source target")?;
        if identity(&output["animator"])? != *animator {
            return Err("Euler target belongs to a different source Animator".into());
        }
        let game_object = identity(&output["gameObject"])?;
        let component = one(array(output, "components")?, "Euler Transform component")?;
        if component["class"].as_str() != Some("Transform") {
            return Err("Euler component is not a Transform".into());
        }
        let transform = identity(component)?;
        if game_object.file != transform.file || transform.file != animator.file {
            return Err("Euler target crosses serialized-file identity domains".into());
        }
        let current = Target {
            file: transform.file,
            game_object: game_object.path_id.parse().map_err(|_| "invalid Euler GameObject identity")?,
            transform: transform.path_id.parse().map_err(|_| "invalid Euler Transform identity")?,
        };
        if target.as_ref().is_some_and(|target| target != &current) {
            return Err("Euler XYZ curves do not resolve to one source Transform".into());
        }
        target = Some(current);
        axes[axis] = Some(curve(value)?);
    }
    let [Some(x), Some(y), Some(z)] = axes else { return Err("Euler XYZ set is incomplete".into()); };
    Ok(Some(Program { target: target.expect("three source curves"), axes: [x, y, z] }))
}

pub(super) fn bind(world: &World, view: Entity, definition: &Definition) -> Result<Option<Binding>, String> {
    let Some(program) = &definition.on.rotation else { return Ok(None); };
    let receiver = world.get::<SourceObjectIdentity>(view)
        .ok_or("Euler FixtureView requires lossless sourceObject extras; re-export this fixture")?;
    if receiver.file != definition.source_file || receiver.game_object != definition.view_game_object
        || !receiver.components.contains(&definition.view_component)
        || !receiver.components.contains(&definition.animator_component)
    {
        return Err("Euler receiver is not the exact source FixtureView/Animator pair".into());
    }
    let mut stack = vec![view];
    let mut found = Vec::new();
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) { stack.extend(children.iter()); }
        if world.get::<SourceObjectIdentity>(entity).is_some_and(|id| program.target.matches(id)) {
            found.push(entity);
        }
    }
    let [entity] = found.as_slice() else {
        return Err(format!("Euler target has {} actual source-identity matches under FixtureView", found.len()));
    };
    let rest = world.get::<Transform>(*entity).ok_or("Euler target has no actual Transform")?.rotation;
    if let Some(by) = world.get::<AnimatedBy>(*entity) {
        if world.get::<AnimationPlayer>(by.0).is_some_and(|player| player.playing_animations().next().is_some()) {
            return Err("Euler target is already owned by an active animation player".into());
        }
    }
    Ok(Some(Binding { entity: *entity, target: program.target.clone(), rest }))
}

pub(super) fn snapshot(world: &World, binding: &Binding) -> Option<(Entity, Quat)> {
    world.get::<Transform>(binding.entity).map(|transform| (binding.entity, transform.rotation))
}

pub(super) fn apply(world: &mut World, binding: &Binding, playback: &Playback) -> Result<(), String> {
    if !world.get::<SourceObjectIdentity>(binding.entity).is_some_and(|id| binding.target.matches(id)) {
        return Err("actual Euler target disappeared or changed source identity".into());
    }
    let current = playback.program.rotation.as_ref().ok_or("Euler program disappeared")?;
    let incoming = current.sample(playback.elapsed.min(playback.program.duration));
    let alpha = if playback.program.transition_seconds > 0.0 {
        (playback.transition_elapsed / playback.program.transition_seconds).clamp(0.0, 1.0) as f32
    } else { 1.0 };
    let outgoing = playback.previous.as_ref().and_then(|previous| {
        previous.program.rotation.as_ref().map(|program| {
            program.sample(previous.elapsed.min(previous.program.duration))
        })
    }).unwrap_or(binding.rest);
    // Source rotation accumulation uses sign-corrected weighted sums followed
    // by normalization. Bevy's default Quat animation mixer uses slerp instead.
    let sign = if outgoing.dot(incoming) < 0.0 { -1.0 } else { 1.0 };
    let sampled = (outgoing * (1.0 - alpha) + incoming * (alpha * sign)).normalize();
    let mut transform = world.get_mut::<Transform>(binding.entity)
        .ok_or("actual Euler Transform disappeared")?;
    // A clamped clip keeps its last pose. Avoid marking the whole hierarchy
    // changed every idle frame when that sampled pose is already installed.
    if transform.rotation != sampled { transform.rotation = sampled; }
    Ok(())
}
