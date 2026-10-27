//! DOLocalRotateAsync's view tween and presenter continuation have separate
//! lifetimes. A replacement kills only the old tween; token cancellation
//! completes that tween then skips idle/cloth. Look-at owns a different slot.

mod kernel;
mod source;

use crate::npc_objective::events::{self, AnimationLease, AnimationOutcome};
use bevy::prelude::*;
use source::ActorSource;
pub(crate) use source::ViewIdentity;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct RotationToken(u64);

#[derive(Component)]
pub(crate) struct ActiveLocalRotation {
    token: RotationToken,
    view: ViewIdentity,
    tween: kernel::LocalTween,
    created: u32,
}

/// Preserve npc::advance's method-based movement ownership boundary.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TutorialRotationOwners<'w, 's> {
    actors: Query<'w, 's, (), With<ActiveLocalRotation>>,
}
impl TutorialRotationOwners<'_, '_> {
    pub fn owns(&self, entity: Entity) -> bool {
        self.actors.contains(entity)
    }
}

#[derive(Clone, Copy)]
enum Stage {
    Preparing { target: Vec3, duration: f32 },
    Tween,
    Idle(AnimationLease),
}
struct Call {
    entity: Entity,
    view: ViewIdentity,
    source: Option<ActorSource>,
    stage: Stage,
}
#[derive(Resource, Default)]
struct Calls {
    next: u64,
    pending: BTreeMap<RotationToken, Call>,
}

fn current_view(world: &World, call: &Call) -> bool {
    source::identity(world, call.entity).as_ref() == Some(&call.view)
}

fn write_rotation(world: &mut World, entity: Entity, rotation: Quat) {
    if let Some(mut pose) = world.get_mut::<Transform>(entity) {
        pose.rotation = rotation;
    }
    if let Some(mut walk) = world.get_mut::<crate::npc::WalkState>(entity) {
        walk.0.forward = (rotation * Vec3::Z).to_array();
    }
}

fn take_call(world: &mut World, token: RotationToken) -> Option<Call> {
    world.get_resource_mut::<Calls>()?.pending.remove(&token)
}

fn put_call(world: &mut World, token: RotationToken, call: Call) {
    world.resource_mut::<Calls>().pending.insert(token, call);
}

fn retire(world: &mut World, token: RotationToken, call: Call) {
    if let Stage::Idle(lease) = call.stage {
        // Do not cancel an animation installed on a replacement view. Its
        // old outcome ticket is still safe to discard by generation.
        if current_view(world, &call) {
            events::cancel_animation_if_owned(world, call.entity, lease);
        }
        events::forget_animation_outcome(world, lease);
    }
    if world
        .get::<ActiveLocalRotation>(call.entity)
        .is_some_and(|t| t.token == token)
    {
        world
            .entity_mut(call.entity)
            .remove::<ActiveLocalRotation>();
    }
}

fn observe_idle(world: &mut World, token: RotationToken, call: Call) {
    let Stage::Idle(lease) = call.stage else {
        put_call(world, token, call);
        return;
    };
    match events::animation_outcome(world, call.entity, lease) {
        AnimationOutcome::Pending => put_call(world, token, call),
        outcome => {
            if outcome == AnimationOutcome::Complete && current_view(world, &call) {
                if let Some(source) = call.source.as_ref() {
                    events::release_source_idle(world, call.entity, lease, &source.idle);
                }
                if let Err(reason) = crate::cloth_runtime::request_reset(world, call.entity) {
                    warn!("[tutorial-talk] rotation cloth reset: {reason}");
                }
            }
            events::forget_animation_outcome(world, lease);
        }
    }
}

fn begin_idle(world: &mut World, token: RotationToken, mut call: Call) {
    if !current_view(world, &call) {
        retire(world, token, call);
        return;
    }
    let Some(source) = call.source.as_ref() else {
        warn!("[tutorial-talk] rotation continuation lost its source view");
        retire(world, token, call);
        return;
    };
    match events::begin_source_animation_owned(
        world,
        call.entity,
        &source.idle,
        true,
        true,
        0.5,
        source.clips.clone(),
        source.view.clone(),
    ) {
        Ok(lease) => {
            call.stage = Stage::Idle(lease);
            observe_idle(world, token, call);
        }
        Err(reason) => {
            warn!("[tutorial-talk] rotation restore idle: {reason}");
            retire(world, token, call);
        }
    }
}

fn prepare(world: &mut World, token: RotationToken, mut call: Call) {
    if !current_view(world, &call) {
        retire(world, token, call);
        return;
    }
    let Stage::Preparing { target, duration } = call.stage else {
        put_call(world, token, call);
        return;
    };
    let source = match source::resolve(world, call.entity) {
        Ok(Some(source)) => source,
        Ok(None) => {
            put_call(world, token, call);
            return;
        }
        Err(reason) => {
            warn!("[tutorial-talk] rotation source: {reason}");
            retire(world, token, call);
            return;
        }
    };
    let Some(from) = world.get::<Transform>(call.entity).map(|t| t.rotation) else {
        retire(world, token, call);
        return;
    };
    let signed = kernel::delta_angle(kernel::euler(kernel::reflect(from)).y, target.y);
    let plan = kernel::plan(signed, duration, source.uses_male, &source.clips);
    if let Some(family) = plan.family {
        match events::begin_source_animation_owned(
            world,
            call.entity,
            family,
            true,
            false,
            0.5,
            source.clips.clone(),
            source.view.clone(),
        ) {
            Ok(lease) => events::forget_animation_outcome(world, lease),
            Err(reason) => {
                warn!("[tutorial-talk] rotation turn motion: {reason}");
                retire(world, token, call);
                return;
            }
        }
    }
    // The presenter starts its turn animation before the view kills the old
    // local tween. Kill(false) resumes that old presenter's idle continuation.
    if let Some(old) = world.entity_mut(call.entity).take::<ActiveLocalRotation>() {
        if let Some(old_call) = take_call(world, old.token) {
            begin_idle(world, old.token, old_call);
        }
    }
    call.source = Some(source);
    call.stage = Stage::Tween;
    let created = super::super::frame(world);
    world.entity_mut(call.entity).insert(ActiveLocalRotation {
        token,
        view: call.view.clone(),
        tween: kernel::LocalTween::new(target, plan.duration),
        created,
    });
    put_call(world, token, call);
}

/// The tutorial supplies a backed-up canonical world quaternion and 0.75s.
/// Source saves world Euler then passes it to a local-Euler tween; NPC roots
/// currently have no parent. Parented roots are rejected rather than silently
/// treating a local quaternion as that world backup.
fn request_rotation(
    world: &mut World,
    entity: Entity,
    target: Quat,
    duration: f32,
) -> Result<RotationToken, String> {
    if !target.is_finite() || !duration.is_finite() || duration < 0.0 {
        return Err("invalid tutorial rotation input".into());
    }
    if world.get::<ChildOf>(entity).is_some() {
        return Err("tutorial source-world rotation requires an unparented NPC root".into());
    }
    let view = source::identity(world, entity).ok_or("tutorial actor has no initialized view")?;
    source::request_assets(world);
    world.init_resource::<Calls>();
    let token = {
        let mut calls = world.resource_mut::<Calls>();
        calls.next = calls
            .next
            .checked_add(1)
            .ok_or("tutorial rotation token exhausted")?;
        RotationToken(calls.next)
    };
    // A request still waiting for its source data has not created a native
    // tween. The newest such request supersedes it without an idle callback.
    let superseded: Vec<_> = world
        .resource::<Calls>()
        .pending
        .iter()
        .filter_map(|(token, c)| {
            (c.entity == entity && matches!(c.stage, Stage::Preparing { .. })).then_some(*token)
        })
        .collect();
    for old in superseded {
        if let Some(call) = take_call(world, old) {
            retire(world, old, call);
        }
    }
    let call = Call {
        entity,
        view,
        source: None,
        stage: Stage::Preparing {
            target: kernel::euler(kernel::reflect(target)),
            duration,
        },
    };
    prepare(world, token, call);
    Ok(token)
}

pub fn restore_rotation(world: &mut World, entity: Entity, rotation: Quat) {
    if let Err(reason) = request_rotation(world, entity, rotation, 0.75) {
        warn!("[tutorial-talk] restore rotation: {reason}");
    }
}

// This concrete caller uses CancellationToken.None. View-tween replacement
// follows Kill(false) above; a generic token-cancellation entrypoint is not
// installed without a real caller. It must not cancel the independent turn
// animation or idle continuation if another source operation later needs it.

pub(super) fn update(world: &mut World) {
    source::request_assets(world);
    world.init_resource::<Calls>();
    // Remove destroyed-view continuations before touching an animation or pose.
    let tokens: Vec<_> = world.resource::<Calls>().pending.keys().copied().collect();
    for token in tokens {
        let Some(call) = take_call(world, token) else {
            continue;
        };
        if !current_view(world, &call) {
            retire(world, token, call);
            continue;
        }
        match call.stage {
            Stage::Preparing { .. } => prepare(world, token, call),
            Stage::Idle(_) => observe_idle(world, token, call),
            Stage::Tween => put_call(world, token, call),
        }
    }
    let dt = world.resource::<crate::npc_clock::NpcClock>().delta();
    let frame = super::super::frame(world);
    let actors: Vec<_> = world
        .query_filtered::<Entity, With<ActiveLocalRotation>>()
        .iter(world)
        .collect();
    for entity in actors {
        let Some(mut tween) = world.entity_mut(entity).take::<ActiveLocalRotation>() else {
            continue;
        };
        if source::identity(world, entity).as_ref() != Some(&tween.view) {
            continue;
        }
        if tween.created == frame {
            world.entity_mut(entity).insert(tween);
            continue;
        }
        let Some(local) = world.get::<Transform>(entity).map(|t| t.rotation) else {
            continue;
        };
        let (rotation, complete) = tween.tween.step(kernel::reflect(local), dt);
        write_rotation(world, entity, kernel::reflect(rotation));
        if complete {
            if let Some(call) = take_call(world, tween.token) {
                begin_idle(world, tween.token, call);
            }
        } else {
            world.entity_mut(entity).insert(tween);
        }
    }
}
