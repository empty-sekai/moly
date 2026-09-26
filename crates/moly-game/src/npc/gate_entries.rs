//! The gate's and the cut-scene's calls into the NPC runtime (JP 6.8.1): the
//! presenter's objective calls (TryCancelCurrentObjective,
//! ForceUpdateObjective, Show, Hide, HideAndCancelObjective,
//! SetImmediatelyExecuteNextObjective, SetCanInterruptTalkData,
//! ForceUpdateCutSceneObjective, ChangeState), the invite's CreateNPC at the
//! gate and its show after the two waits, the cut-scene presenter's takeover
//! of a present NPC and its RestoreStates, and the go-home cut-scene's start
//! callback.
//!
//! The presenter's calls:
//! - TryCancelCurrentObjective: refused while the character talks (the
//!   cancel condition); otherwise the current objective's own cancel (see
//!   `moly_law::objective::presenter::try_cancel`) and its dispose. The
//!   product's per-objective cancel is `npc_fixture_talk::try_cancel_current`.
//! - ForceUpdateObjective: that cancel, and when it reported true, the
//!   model's reset and new talk data (owed to the character's next decision
//!   pass, see `npc_objective::owe_force_updates`).
//! - Hide sets the model's visibility off and hides the view; Show sets it on
//!   and shows the view. While the model is not visible the AI loop yields
//!   before its rest and its decision unless the character has no objective
//!   or runs the entry-site objective (see
//!   `moly_law::objective::presenter::can_running_ai`); the objective already
//!   running goes on. Show's teleport check (the view flag and the
//!   nearest-movable-position search) is not modelled.
//! - SetImmediatelyExecuteNextObjective(enable) writes the flag that skips
//!   the next rest; SetCanInterruptTalkData(enable, type) writes a new
//!   interrupt marker; the model's reset clears both.
//! - ForceUpdateCutSceneObjective makes no cancel: the model resets twice and
//!   takes a cut-scene talk data (type 17, no character, no fixture, no
//!   interrupt marker). The decision ladder has no row for that data: outside
//!   the tutorial it reaches the lottery on the next decision, and the
//!   cut-scene objective only continues a current one.
//!
//! The invite's end callback (`OnEndCutScene`): the cut-scene avatar is
//! disposed, one Yield, then CreateNPC at the gate's first action point's
//! end locator with a constant rotation of pi radians about the up axis
//! (the literal is not the locator's own rotation), the wait until the unit
//! exists, the wait while its current objective type is 0 (its first
//! decision takes the entry-site objective, which hides it), then
//! TryCancelCurrentObjective, ForceUpdateObjective and Show.
//!
//! CreateNPC places the new character where the network object is
//! instantiated; the agent then stands on the navigation surface nearest to
//! that point within its query box (the host's warp). The character's AI is
//! set up as every visitor's: entry-site talk data with the entry-site
//! interrupt marker and the flag that skips its first rest, on the home site.
//!
//! Every call writes one `[npc-entry]` line.

use bevy::prelude::*;
use moly_law::objective::presenter::{self as law, CancelFlags};
use moly_law::objective::{InterruptMarker, ObjectiveType, TalkType};

use crate::npc::{CharacterUnitId, NpcAction, NpcActions, RestLifecycle};
use crate::npc_objective::{AiTalkData, ObjectiveMind, TalkSlot};

/// The presenter model's visibility is off (Hide, HideAndCancelObjective).
/// Show removes it.
#[derive(Component)]
pub(crate) struct ModelHidden;

/// The invite's CreateNPC rotation: `Quaternion.Internal_FromEulerRad` of
/// (0, this, 0), the literal float pi (bits 0x40490fdb).
pub(crate) const INVITE_YAW_RADIANS: f32 = f32::from_bits(0x4049_0fdb);

fn frame(world: &World) -> u32 {
    world
        .get_resource::<bevy::diagnostic::FrameCount>()
        .map_or(0, |count| count.0)
}

fn unit_of(world: &World, entity: Entity) -> Option<u32> {
    world.get::<CharacterUnitId>(entity).map(|unit| unit.0)
}

/// The NPC entity of `unit` (the avatar store's FindNPC): not the player's
/// avatar and not a cut-scene avatar.
pub(crate) fn find_npc(world: &mut World, unit: u32) -> Option<Entity> {
    world
        .query_filtered::<(Entity, &CharacterUnitId), (
            With<ObjectiveMind>,
            Without<crate::player::PlayerControlled>,
            Without<crate::cutscene::CutSceneAvatar>,
        )>()
        .iter(world)
        .find(|(_, id)| id.0 == unit)
        .map(|(entity, _)| entity)
}

/// Whether the AI loop runs its rest and decision for this character now.
pub(crate) fn can_running_ai(hidden: bool, actions: &NpcActions, mind: &ObjectiveMind) -> bool {
    law::can_running_ai(false, actions.current as i32, !hidden, mind.current)
}

/// Presenter.TryCancelCurrentObjective. Returns the reported value.
pub(crate) fn try_cancel_current_objective(world: &mut World, entity: Entity) -> bool {
    let frame = frame(world);
    let unit = unit_of(world, entity).unwrap_or(0);
    let Some(mind) = world.get::<ObjectiveMind>(entity) else {
        info!("[npc-entry] unit={unit} frame={frame} TryCancelCurrentObjective: no AI (false)");
        return false;
    };
    let current = mind.current;
    // The cut-scene objective's constructor never sets CanCancel.
    if current.is_some_and(|kind| !law::objective_can_cancel(kind)) {
        let state = world
            .get::<NpcActions>(entity)
            .map_or(0, |actions| actions.current as i32);
        let reported = law::cancel_condition(state)
            && law::try_cancel(CancelFlags {
                can_cancel: false,
                canceled: false,
                disposed: false,
            })
            .reported;
        info!("[npc-entry] unit={unit} frame={frame} TryCancelCurrentObjective: current {current:?} cannot be cancelled ({reported})");
        return reported;
    }
    world.init_resource::<crate::npc_fixture_talk::FixtureTalkGroups>();
    let (reported, owed) = world.resource_scope(
        |world, mut groups: Mut<crate::npc_fixture_talk::FixtureTalkGroups>| {
            crate::npc_fixture_talk::try_cancel_current(world, &mut groups, entity, frame)
        },
    );
    if owed > 0 {
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
            crate::npc_objective::owe_force_updates(&mut mind, frame, owed);
        }
    }
    info!("[npc-entry] unit={unit} frame={frame} TryCancelCurrentObjective: current {current:?} -> {reported} (OnCancel owes {owed} ForceUpdateObjective)");
    reported
}

/// Presenter.ForceUpdateObjective: the cancel, then on true the model's
/// reset and new talk data, owed to the next decision pass. An objective
/// waiting in its body sees the cancel on its next poll (the next frame).
pub(crate) fn force_update_objective(world: &mut World, entity: Entity) -> bool {
    let frame = frame(world);
    let unit = unit_of(world, entity).unwrap_or(0);
    let waiting = world
        .get::<ObjectiveMind>(entity)
        .is_some_and(|mind| mind.body.is_some() || mind.cancelled);
    let cancelled = try_cancel_current_objective(world, entity);
    if cancelled {
        if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
            let end = if waiting {
                frame.wrapping_add(1)
            } else {
                frame
            };
            crate::npc_objective::owe_force_updates(&mut mind, end, 1);
        }
    }
    info!(
        "[npc-entry] unit={unit} frame={frame} ForceUpdateObjective: cancel {cancelled}{}",
        if cancelled {
            ", reset and new talk data owed to the next decision"
        } else {
            ", nothing updated"
        }
    );
    cancelled
}

/// Presenter.Hide: the model's visibility off, the view hidden.
pub(crate) fn hide(world: &mut World, entity: Entity) {
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return;
    };
    entity_mut.insert((ModelHidden, Visibility::Hidden));
    let unit = unit_of(world, entity).unwrap_or(0);
    info!("[npc-entry] unit={unit} frame={} Hide", frame(world));
}

/// Presenter.Show: the model's visibility on, the view shown (the teleport
/// check is not modelled).
pub(crate) fn show(world: &mut World, entity: Entity) {
    let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
        return;
    };
    entity_mut.remove::<ModelHidden>();
    entity_mut.insert(Visibility::Inherited);
    let unit = unit_of(world, entity).unwrap_or(0);
    info!("[npc-entry] unit={unit} frame={} Show", frame(world));
}

/// Presenter.HideAndCancelObjective: hidden, then the cancel (its value is
/// not used).
pub(crate) fn hide_and_cancel_objective(world: &mut World, entity: Entity) -> bool {
    hide(world, entity);
    try_cancel_current_objective(world, entity)
}

/// Presenter.SetImmediatelyExecuteNextObjective.
pub(crate) fn set_immediately_execute_next_objective(
    world: &mut World,
    entity: Entity,
    enable: bool,
) {
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        mind.skip_next_rest = enable;
    }
    let unit = unit_of(world, entity).unwrap_or(0);
    info!(
        "[npc-entry] unit={unit} frame={} SetImmediatelyExecuteNextObjective({enable})",
        frame(world)
    );
}

/// Presenter.SetCanInterruptTalkData(enable, type): a new marker.
pub(crate) fn set_can_interrupt_talk_data(
    world: &mut World,
    entity: Entity,
    enable: bool,
    objective: ObjectiveType,
) {
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
        slot.interrupt = Some(InterruptMarker {
            marker_type: objective as i32,
            can_interrupt: enable,
        });
    }
    let unit = unit_of(world, entity).unwrap_or(0);
    info!(
        "[npc-entry] unit={unit} frame={} SetCanInterruptTalkData({enable}, {})",
        frame(world),
        objective as i32
    );
}

/// The AI model's reset as far as this host holds it: the talk data moves to
/// the previous slot, the interrupt marker and the tweet id clear, and the
/// flag that skips the next rest clears.
fn model_reset(world: &mut World, entity: Entity) {
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
        slot.reset_ai_talk_data();
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        mind.skip_next_rest = false;
    }
    if let Some(mut actions) = world.get_mut::<NpcActions>(entity) {
        actions.tweet_id = 0;
    }
}

/// The cut-scene talk data: type 17 and nothing else.
fn cut_scene_talk_data() -> AiTalkData {
    AiTalkData {
        kind: TalkType::CutScene,
        content: None,
        target_fixture: None,
        target_position: [0.0; 3],
        main_character: 0,
        characters: Vec::new(),
        pre_action: None,
        locate: None,
        pending_factory: None,
    }
}

/// Presenter.ForceUpdateCutSceneObjective: two resets and the cut-scene talk
/// data, no cancel.
pub(crate) fn force_update_cut_scene_objective(world: &mut World, entity: Entity) {
    model_reset(world, entity);
    model_reset(world, entity);
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
        slot.set_current(cut_scene_talk_data());
    }
    let unit = unit_of(world, entity).unwrap_or(0);
    info!("[npc-entry] unit={unit} frame={} ForceUpdateCutSceneObjective: reset x2, cut-scene talk data (17)", frame(world));
}

/// Presenter.ChangeState.
pub(crate) fn change_state(world: &mut World, entity: Entity, state: NpcAction) {
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    if let Ok((mut actions, mut rest)) = query.get_mut(world, entity) {
        actions.change(state, &mut rest);
    }
    let unit = unit_of(world, entity).unwrap_or(0);
    info!(
        "[npc-entry] unit={unit} frame={} ChangeState({:?})",
        frame(world),
        state
    );
}

/// CutScenePresenter.Setup with useAlreadyExistCharacter, for a present NPC:
/// TryCancelCurrentObjective, ForceUpdateCutSceneObjective,
/// SetImmediatelyExecuteNextObjective(true). Returns the cancel's value.
pub(crate) fn take_over_for_cut_scene(world: &mut World, entity: Entity) -> bool {
    let cancelled = try_cancel_current_objective(world, entity);
    force_update_cut_scene_objective(world, entity);
    set_immediately_execute_next_objective(world, entity, true);
    cancelled
}

/// CutScenePresenter.RestoreStates for one bound unit: ChangeState(Idle).
pub(crate) fn restore_state(world: &mut World, entity: Entity) {
    change_state(world, entity, NpcAction::Idle);
}

/// MysekaiGateUtility.OnStartCutSceneAsync(target): HideAndCancelObjective
/// on every NPC whose unit is not the target. Returns (unit, cancel value).
pub(crate) fn on_start_cut_scene(world: &mut World, target_unit: u32) -> Vec<(u32, bool)> {
    let others: Vec<(u32, Entity)> = world
        .query_filtered::<(Entity, &CharacterUnitId), (
            With<ObjectiveMind>,
            Without<crate::player::PlayerControlled>,
            Without<crate::cutscene::CutSceneAvatar>,
        )>()
        .iter(world)
        .filter(|(_, unit)| unit.0 != target_unit)
        .map(|(entity, unit)| (unit.0, entity))
        .collect();
    let mut out = Vec::with_capacity(others.len());
    for (unit, entity) in others {
        let cancelled = hide_and_cancel_objective(world, entity);
        out.push((unit, cancelled));
    }
    info!("[npc-entry] frame={} OnStartCutSceneAsync({target_unit}): HideAndCancelObjective on {out:?}", frame(world));
    out
}

/// The invite's CreateNPC position: the gate's first action point's end
/// locator on the placed gate.
pub(crate) fn gate_first_end_loc(world: &World, gate: Entity) -> Result<[f32; 3], String> {
    let package = world
        .get::<crate::fixture_activity_state::FixtureActivityIdentity>(gate)
        .map(|identity| identity.model_package.clone())
        .ok_or("the gate has no fixture identity")?;
    let placed = world
        .get::<GlobalTransform>(gate)
        .copied()
        .ok_or("the gate has no transform")?;
    let points = world
        .get_resource::<crate::fixture_attach::AttachPoints>()
        .ok_or("the locator archive is not loaded")?;
    let array = points.source_array(&package, &placed)?;
    let (_, pair) = array
        .first()
        .ok_or_else(|| format!("{package} has no action point (First raises)"))?;
    let end = pair
        .end
        .ok_or_else(|| format!("{package}'s first action point has no end locator"))?;
    Ok(end.position)
}

/// The invite's CreateNPC rotation.
pub(crate) fn invite_rotation() -> Quat {
    Quat::from_rotation_y(INVITE_YAW_RADIANS)
}

/// MysekaiMultiplayNPCController.CreateNPC(unit, 1, position, rotation,
/// visit count) for a unit not present: a new character at the pose, its AI
/// set up as every visitor's (entry-site data, the entry-site interrupt, the
/// start flag), on the home site.
pub(crate) fn create_npc(
    world: &mut World,
    unit: u32,
    position: [f32; 3],
    rotation: Quat,
) -> Result<Entity, String> {
    if let Some(existing) = find_npc(world, unit) {
        return Err(format!("unit {unit} is present ({existing:?})"));
    }
    let created = crate::npc::spawn_temporary_units(world, &[unit])?;
    let entity = *created
        .first()
        .ok_or_else(|| format!("unit {unit} was not created"))?;
    let home = crate::npc::residency::SITE_TYPES[0].to_owned();
    // The agent stands on the navigation surface nearest the instantiation
    // point within its query box; a point that maps nowhere keeps the raw
    // position.
    let warped = world
        .get_resource::<crate::walk_face::WalkFace>()
        .and_then(|walk_face| {
            walk_face.sample(
                [position[0], position[2]],
                moly_law::carve::AGENT_QUERY_HALF_EXTENT,
            )
        })
        .and_then(|xz| {
            world
                .get_resource::<crate::npc_objective::ObjectiveFace>()
                .and_then(|face| face.navigation_point_at(xz))
        });
    let at = warped.unwrap_or(position);
    crate::npc::force_set_position(world, entity, at);
    let forward = (rotation * Vec3::Z).to_array();
    if let Some(mut walk) = world.get_mut::<crate::npc::WalkState>(entity) {
        walk.0.forward = forward;
    }
    if let Some(mut transform) = world.get_mut::<Transform>(entity) {
        transform.rotation = rotation;
    }
    if let Some(mut actions) = world.get_mut::<NpcActions>(entity) {
        actions.site_type = home;
    }
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
        *slot = TalkSlot::entry_site(unit);
    }
    info!(
        "[npc-entry] unit={unit} frame={} CreateNPC(unit {unit}, 1, position {position:?}, rotation {:?}): {entity:?} at {at:?}{} facing {forward:?}; AI set up with the entry-site data (marker 14) and the start flag",
        frame(world),
        rotation.to_array(),
        if warped.is_some() { "" } else { " (no navigation cell within the query box: raw position)" },
    );
    Ok(entity)
}

/// The invite's two waits after CreateNPC: IsExistNPCAll([unit]) (its model
/// is assembled), then while its current objective type is 0.
pub(crate) fn invite_waits(world: &World, entity: Entity) -> (bool, bool) {
    let exists = world
        .get::<crate::character::MotionDriver>(entity)
        .is_some();
    let decided = world
        .get::<ObjectiveMind>(entity)
        .is_some_and(|mind| mind.current.is_some());
    (exists, exists && decided)
}

/// The invite's calls after its waits: TryCancelCurrentObjective,
/// ForceUpdateObjective, Show.
pub(crate) fn invite_show(world: &mut World, entity: Entity) -> (bool, bool) {
    let cancelled = try_cancel_current_objective(world, entity);
    let updated = force_update_objective(world, entity);
    show(world, entity);
    (cancelled, updated)
}
