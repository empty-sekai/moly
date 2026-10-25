//! Tutorial waiting-talk operations and their explicit server-panel driver.
//!
//! `controls.tutorial.commands` is a named mock of tutorial chapter selection:
//! it supplies ordered `{id, unit, operation, tutorialTalkId}` commands. The
//! client executes the real waiting-talk operation and observes Talk entry and
//! exit. This is not an implementation of the tutorial's chapter/navigation UI.

use super::{EventLower, TalkSlot, controller};
use crate::npc::{NpcAction, NpcActions};
use bevy::prelude::*;
use moly_law::npc_hfsm::LowerState;
use moly_law::objective::TalkType;
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TutorialMaster {
    pub id: i32,
    pub assetbundle_name: String,
    pub lua: String,
    #[serde(default)]
    pub mysekai_character_talk_tweet_id: i32,
}

pub type OverwriteTalk = Arc<dyn Fn(&mut World, Entity) + Send + Sync>;

/// The tutorial-specific fields of the AI's current talk data. Kept separate
/// from ordinary character-talk masters, which have a different id domain.
#[derive(Component, Clone)]
pub struct TutorialTalkData {
    pub master: TutorialMaster,
    pub next_action: NpcAction,
    pub restore_rotation: bool,
    pub on_overwrite: Option<OverwriteTalk>,
}

/// The AI model retains this id across lower-state replacement, including a
/// waiting objective. The lower factory refreshes its free-walk data while
/// the tutorial is active; zero deliberately disables that refresh.
#[derive(Component, Clone, Copy, Default)]
pub struct DefaultTutorialTalkId(pub i32);

/// The old AI's random-move preference is still written by tutorial chapters.
/// HFSM ForceInterrupt always selects RandomWalk and does not consume it.
#[derive(Component, Clone, Copy, Default)]
pub struct TutorialForceOccurRandomMove(pub bool);

/// The tutorial processor's explicit EnableJoyStick(false) ownership.
#[derive(Component)]
pub struct TutorialMovementHold;

fn set_freewalk_data(
    world: &mut World,
    entity: Entity,
    master: TutorialMaster,
) -> Result<(), String> {
    let destination = crate::npc_objective::general_data_destination(world, entity)?;
    let mut data = super::talk_data(world, entity, TalkType::TutorialFreeWalking);
    data.target_position = destination;
    if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
        if slot
            .current
            .as_ref()
            .is_some_and(|current| current.content.is_some())
        {
            slot.previous = slot.current.clone();
        }
        slot.set_current(data);
    } else {
        return Err("tutorial actor has no talk-data slot".into());
    }
    world.entity_mut(entity).insert(TutorialTalkData {
        master,
        next_action: NpcAction::Idle,
        restore_rotation: false,
        on_overwrite: None,
    });
    Ok(())
}

/// The presenter's CreateLowerState prelude, before its lower-type switch.
pub fn refresh_default_freewalk(world: &mut World, entity: Entity) -> Result<(), String> {
    if !world
        .get_resource::<crate::server_panel::ServerPanel>()
        .is_some_and(|panel| panel.is_tutorial())
    {
        return Ok(());
    }
    let id = world
        .get::<DefaultTutorialTalkId>(entity)
        .map_or(0, |value| value.0);
    if id == 0 {
        return Ok(());
    }
    let master = crate::player_talk::tutorial_master(world, id)?;
    set_freewalk_data(world, entity, master)
}

/// ForceUpdateFreeWalkTutorialObjective installs the default data before
/// interrupting into RandomWalk; its data destination and the upper's walk
/// destination are separate draws. SetTweetId(null ordinary master) clears it.
pub fn begin_freewalk(world: &mut World, entity: Entity, id: i32) -> Result<(), String> {
    if !super::enrolled(world, entity) {
        return Err("tutorial actor is outside the state-machine scope".into());
    }
    let master = crate::player_talk::tutorial_master(world, id)?;
    world.entity_mut(entity).insert(DefaultTutorialTalkId(id));
    set_freewalk_data(world, entity, master)?;
    if let Some(mut actions) = world.get_mut::<NpcActions>(entity) {
        actions.tweet_id = 0;
    }
    if !controller::force_interrupt_in_world(
        world,
        entity,
        moly_law::npc_hfsm::UpperType::RandomWalk,
    ) {
        return Err("tutorial RandomWalk interrupt could not run".into());
    }
    Ok(())
}

/// ForceUpdateWaitingTutorialObjective: only Rest and Communication normalize
/// to Idle before the lower captures the next action. Zero creates an empty
/// waiting lower and leaves the current talk data unchanged.
fn begin_waiting_inner(
    update_default: bool,
    world: &mut World,
    entity: Entity,
    tutorial_talk_id: i32,
    on_overwrite: Option<OverwriteTalk>,
    restore_rotation: bool,
) -> Result<(), String> {
    if !super::enrolled(world, entity) {
        return Err("tutorial actor is outside the state-machine scope".into());
    }
    let master = if tutorial_talk_id == 0 {
        None
    } else {
        Some(crate::player_talk::tutorial_master(
            world,
            tutorial_talk_id,
        )?)
    };
    let current = world
        .get::<NpcActions>(entity)
        .ok_or("tutorial actor has no action state")?
        .current;
    let next_action = if matches!(current, NpcAction::Rest | NpcAction::Communication) {
        NpcAction::Idle
    } else {
        current
    };
    if update_default {
        world
            .entity_mut(entity)
            .insert(DefaultTutorialTalkId(tutorial_talk_id));
    }
    if !controller::force_event_lower(world, entity, LowerState::TutorialWait) {
        return Err("tutorial transition budget is exhausted".into());
    }
    if current != next_action {
        super::action(world, entity, next_action);
    }
    let at = super::frame(world);
    world.entity_mut(entity).insert((
        super::TutorialEntered,
        EventLower::new(LowerState::TutorialWait, at),
    ));
    super::mark_objective(world, entity, LowerState::TutorialWait);
    if let Some(master) = master {
        let data = super::talk_data(world, entity, TalkType::TutorialWaiting);
        if let Some(mut slot) = world.get_mut::<TalkSlot>(entity) {
            slot.set_current(data);
            slot.interrupt = None;
        }
        world.entity_mut(entity).insert(TutorialTalkData {
            master,
            next_action,
            restore_rotation,
            on_overwrite,
        });
    }
    Ok(())
}

/// AI-model forced waiting updates the retained default before transition.
pub fn begin_waiting(
    world: &mut World,
    entity: Entity,
    id: i32,
    callback: Option<OverwriteTalk>,
    restore: bool,
) -> Result<(), String> {
    begin_waiting_inner(true, world, entity, id, callback, restore)
}

/// The controller constructs an empty lower directly; this is not the
/// AI-model forced operation and must retain its default tutorial id.
pub(super) fn reenter_waiting(world: &mut World, entity: Entity) -> Result<(), String> {
    begin_waiting_inner(false, world, entity, 0, None, true)
}

/// OnClickPlayerTalkAction invokes the supplied override without starting its
/// default script. The callback owns whatever tutorial operation follows.
pub fn invoke_overwrite(world: &mut World, entity: Entity) -> bool {
    let callback = world
        .get::<TutorialTalkData>(entity)
        .and_then(|data| data.on_overwrite.clone());
    if let Some(callback) = callback {
        callback(world, entity);
        true
    } else {
        false
    }
}

/// The default tutorial callback reads the live data after the script finishes.
/// A waiting/free-walk operation may have replaced it while the window was open.
pub fn finish_default_talk(world: &mut World, entity: Entity, backup_rotation: Option<Quat>) {
    let Some(data) = world.get::<TutorialTalkData>(entity).cloned() else {
        warn!("[tutorial-talk] default talk finished without its current tutorial data");
        return;
    };
    if data.restore_rotation {
        if let Some(rotation) = backup_rotation {
            restore_rotation(world, entity, rotation);
        }
    }
    let mut query = world.query::<(&mut NpcActions, &mut crate::npc::RestLifecycle)>();
    if let Ok((mut actions, mut rest)) = query.get_mut(world, entity) {
        actions.change(data.next_action, &mut rest);
        actions.talk_owner = None;
    }
    if let Some(mut emotes) = world.get_resource_mut::<crate::emoticon::RestEmoteRequests>() {
        emotes.hide(entity);
    }
    super::reset_face(world, entity);
}

/// A local Euler tween for the native under-five-degree branch. That branch
/// deliberately plays no turn clip and does not use the locomotion turn driver.
#[derive(Component)]
pub(crate) struct SmallTutorialRotation {
    target: Quat,
    started: Option<(Vec3, Vec3)>,
    elapsed: f32,
    frame: u32,
}

/// Movement yields to the view's local rotation without exposing its tween state.
#[derive(bevy::ecs::system::SystemParam)]
pub struct TutorialRotationOwners<'w, 's> {
    actors: Query<'w, 's, (), With<SmallTutorialRotation>>,
}

impl TutorialRotationOwners<'_, '_> {
    pub fn owns(&self, entity: Entity) -> bool {
        self.actors.contains(entity)
    }
}

fn local_angles(rotation: Quat) -> Vec3 {
    let (y, x, z) = rotation.to_euler(EulerRot::YXZ);
    Vec3::new(x.to_degrees(), y.to_degrees(), z.to_degrees()).map(moly_law::path::make_positive_yaw)
}

fn advance_small_rotation(world: &mut World) {
    let dt = world.resource::<crate::npc_clock::NpcClock>().delta();
    let frame = super::frame(world);
    let entities: Vec<Entity> = world
        .query_filtered::<Entity, With<SmallTutorialRotation>>()
        .iter(world)
        .collect();
    for entity in entities {
        let Some(mut tween) = world.entity_mut(entity).take::<SmallTutorialRotation>() else {
            continue;
        };
        if tween.frame == frame {
            world.entity_mut(entity).insert(tween);
            continue;
        }
        let Some(rotation) = world.get::<Transform>(entity).map(|pose| pose.rotation) else {
            continue;
        };
        let target = tween.target;
        let (start, change) = *tween.started.get_or_insert_with(|| {
            let start = local_angles(rotation);
            let end = local_angles(target);
            (
                start,
                Vec3::new(
                    moly_law::path::look_at::fast_change(start.x, end.x),
                    moly_law::path::look_at::fast_change(start.y, end.y),
                    moly_law::path::look_at::fast_change(start.z, end.z),
                ),
            )
        });
        tween.elapsed = (tween.elapsed + dt).min(0.75);
        let ease = moly_law::ui::dotween::Ease::OutQuad.evaluate(tween.elapsed, 0.75);
        let angle = (start + change * ease).map(f32::to_radians);
        let rotation = Quat::from_euler(EulerRot::YXZ, angle.y, angle.x, angle.z);
        if let Some(mut pose) = world.get_mut::<Transform>(entity) {
            pose.rotation = rotation;
        }
        if let Some(mut walk) = world.get_mut::<crate::npc::WalkState>(entity) {
            walk.0.forward = (rotation * Vec3::Z).to_array();
        }
        if tween.elapsed < 0.75 {
            world.entity_mut(entity).insert(tween);
        } else if let Some(idle) = world
            .get::<crate::npc::MotionClips>(entity)
            .map(|clips| clips.idle.clone())
        {
            if let Err(reason) = super::begin_animation(world, entity, &idle, true, 0.5) {
                warn!("[tutorial-talk] restore idle: {reason}");
            }
        }
    }
}

/// Fire-and-forget DOLocalRotateAsync at the default tutorial talk's end.
pub fn restore_rotation(world: &mut World, entity: Entity, rotation: Quat) {
    let Some(from) = world.get::<Transform>(entity).map(|t| t.rotation) else {
        return;
    };
    let from_yaw = moly_law::path::heading_yaw((from * Vec3::Z).to_array());
    let to_yaw = moly_law::path::heading_yaw((rotation * Vec3::Z).to_array());
    let change = moly_law::path::look_at::fast_change(from_yaw, to_yaw);
    world.entity_mut(entity).remove::<SmallTutorialRotation>();
    if change.abs() < 5.0 {
        let frame = super::frame(world);
        world.entity_mut(entity).insert(SmallTutorialRotation {
            target: rotation,
            started: None,
            elapsed: 0.0,
            frame,
        });
        return;
    }
    use moly_law::path::TurnMotion;
    let motion = match (change >= 0.0, change.abs()) {
        (true, angle) if angle <= 60.0 => TurnMotion::Turn45R,
        (true, angle) if angle <= 120.0 => TurnMotion::Turn90R,
        (true, _) => TurnMotion::Turn180R,
        (false, angle) if angle <= 60.0 => TurnMotion::Turn45L,
        (false, angle) if angle <= 120.0 => TurnMotion::Turn90L,
        (false, _) => TurnMotion::Turn180L,
    };
    let clip_name = format!("{}_S", motion.base_name());
    let duration = world
        .get_resource::<crate::character::MotionLibrary>()
        .and_then(|library| {
            world
                .get_resource::<Assets<bevy::gltf::Gltf>>()?
                .get(&library.gltf)
        })
        .and_then(|gltf| gltf.named_animations.get(clip_name.as_str()))
        .and_then(|clip| world.get_resource::<Assets<AnimationClip>>()?.get(clip))
        .map_or(0.75, AnimationClip::duration);
    let mut to = rotation;
    if from.dot(to) < 0.0 {
        to = -to;
    }
    if let Some(mut phase) = world.get_mut::<crate::npc::MotionPhase>(entity) {
        *phase = crate::npc::MotionPhase::Turning {
            motion,
            from,
            to,
            duration,
            elapsed: 0.0,
        };
    }
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PanelCommand {
    id: String,
    unit: u32,
    operation: PanelOperation,
    #[serde(default)]
    tutorial_talk_id: i32,
    #[serde(default = "restore_default")]
    restore_rotate: bool,
}

fn restore_default() -> bool {
    true
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum PanelOperation {
    Wait,
    Complete,
}

struct WaitingTalk {
    command: PanelCommand,
    actor: Entity,
    entered_talk: bool,
}

#[derive(Resource, Default)]
struct PanelDriver {
    revision: Option<u64>,
    seen: HashSet<String>,
    pending: VecDeque<PanelCommand>,
    waiting: Option<WaitingTalk>,
    last_error: Option<String>,
}

fn read_commands(world: &mut World) {
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
    if world.resource::<PanelDriver>().revision == Some(revision) {
        return;
    }
    world.resource_mut::<PanelDriver>().revision = Some(revision);
    let commands = (|| -> Result<Vec<PanelCommand>, String> {
        let doc: serde_json::Value =
            serde_json::from_str(&text).map_err(|error| error.to_string())?;
        let Some(commands) = doc.pointer("/controls/tutorial/commands") else {
            return Ok(Vec::new());
        };
        serde_json::from_value(commands.clone())
            .map_err(|error| format!("tutorial panel commands: {error}"))
    })();
    match commands {
        Ok(commands) => {
            let mut driver = world.resource_mut::<PanelDriver>();
            for command in commands {
                if command.id.is_empty() {
                    warn!("[tutorial-panel] command requires a stable nonempty id");
                    continue;
                }
                if driver.seen.insert(command.id.clone()) {
                    driver.pending.push_back(command);
                }
            }
        }
        Err(reason) => warn!("[tutorial-panel] {reason}"),
    }
}

pub(super) fn update(world: &mut World) {
    advance_small_rotation(world);
    world.init_resource::<PanelDriver>();
    read_commands(world);
    let active = world
        .get_resource::<crate::server_panel::ServerPanel>()
        .is_some_and(|panel| panel.is_tutorial());
    if active {
        world.insert_resource(crate::audio_inputs::TutorialVoice);
    } else {
        world.remove_resource::<crate::audio_inputs::TutorialVoice>();
        return;
    }
    // WaitToWaitingTalk: await Talk, then await leaving Talk; a changed master
    // does not report that the requested tutorial line completed.
    let waiting = world.resource_mut::<PanelDriver>().waiting.take();
    if let Some(mut waiting) = waiting {
        let current = world
            .get::<NpcActions>(waiting.actor)
            .map(|actions| actions.current);
        if current == Some(NpcAction::Talk) {
            waiting.entered_talk = true;
        }
        if current.is_none() {
            warn!("[tutorial-panel] {} lost its actor", waiting.command.id);
        } else if waiting.entered_talk && current != Some(NpcAction::Talk) {
            let same = world
                .get::<TalkSlot>(waiting.actor)
                .and_then(TalkSlot::kind)
                == Some(TalkType::TutorialWaiting)
                && world
                    .get::<TutorialTalkData>(waiting.actor)
                    .is_some_and(|data| data.master.id == waiting.command.tutorial_talk_id);
            info!(
                "[tutorial-panel] {} waiting talk returned; requested master still current={same}",
                waiting.command.id
            );
        } else {
            world.resource_mut::<PanelDriver>().waiting = Some(waiting);
            return;
        }
    }
    let Some(command) = world.resource::<PanelDriver>().pending.front().cloned() else {
        return;
    };
    let Some(actor) = crate::npc::gate_entries::find_npc(world, command.unit) else {
        return;
    };
    if world
        .get::<NpcActions>(actor)
        .is_none_or(|actions| !actions.ready() || actions.current == NpcAction::Talk)
    {
        return;
    }
    match command.operation {
        PanelOperation::Wait => {
            if command.tutorial_talk_id == 0 {
                warn!("[tutorial-panel] wait requires a nonzero master id");
                world.resource_mut::<PanelDriver>().pending.pop_front();
                return;
            }
            if let Err(reason) = begin_waiting(
                world,
                actor,
                command.tutorial_talk_id,
                None,
                command.restore_rotate,
            ) {
                let mut driver = world.resource_mut::<PanelDriver>();
                if driver.last_error.as_ref() != Some(&reason) {
                    warn!("[tutorial-panel] {}: {reason}", command.id);
                    driver.last_error = Some(reason);
                }
                return;
            }
            info!(
                "[tutorial-panel] {} WaitToWaitingTalk unit {} tutorial {}",
                command.id, command.unit, command.tutorial_talk_id
            );
            world.resource_mut::<PanelDriver>().waiting = Some(WaitingTalk {
                command: command.clone(),
                actor,
                entered_talk: false,
            });
        }
        PanelOperation::Complete => super::notify_tutorial_complete(world, actor),
    }
    let mut driver = world.resource_mut::<PanelDriver>();
    driver.pending.pop_front();
    driver.last_error = None;
}
