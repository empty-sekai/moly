//! Temporary viewing arrangements on the real field. Planning is read-only and
//! atomic; only existing actors and actual UID-checked fixtures participate.
//! The regular talk/player-fixture owners still perform all animation and cleanup.
use super::*;
use bevy::diagnostic::FrameCount;
use crate::{npc::WalkState, npc_objective::ObjectiveFace, player::PlayerControlled,
    site::GroundEpoch, talk::TalkHold};

#[derive(Clone)]
struct ActorPose { entity: Entity, before: Transform, after: Transform, npc: bool }
#[derive(Resource)]
pub(crate) struct ScenePreview {
    ticket: u64,
    epoch: u64,
    created_frame: u32,
    actors: Vec<ActorPose>,
}
impl ScenePreview {
    pub(super) fn ready(&self, choice: &PlaybackChoice, frame: u32) -> bool {
        self.ticket == choice.ticket && self.created_frame != frame
    }
}
fn owners_idle(world: &World) -> bool {
    !world.contains_resource::<crate::player_talk::PlayerTalkSession>()
        && !world.contains_resource::<crate::talk::ActiveTalk>()
        && !world.resource::<PlayerFixtureRuntime>().active()
        && !crate::fixture_gimmick::session::blocks_replacement(world.resource::<crate::fixture_gimmick::Gimmicks>())
}

/// A small, deterministic ring of candidates, accepted only by the current
/// navigation mesh and separated from the other real actors. Never return an
/// arbitrary point when all navigation samples fail.
fn viewing_position(
    center: Vec3, yaw: Quat, occupied: &[Vec3],
    mut sample: impl FnMut(Vec3) -> Option<Vec3>,
) -> Option<Vec3> {
    for radius in [0.95, 1.35, 1.8, 2.3, 2.9] {
        for turn in [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6] {
            let angle = turn as f32 * std::f32::consts::PI / 6.;
            let candidate = center + yaw * Vec3::new(angle.sin() * radius, 0., angle.cos() * radius);
            let Some(point) = sample(candidate) else { continue; };
            if point.is_finite() && point.distance(center) < 3.5
                && occupied.iter().all(|other| point.distance(*other) >= 0.62) {
                return Some(point);
            }
        }
    }
    None
}
fn facing(point: Vec3, target: Vec3, fallback: Quat) -> Quat {
    let direction = target - point;
    if direction.x * direction.x + direction.z * direction.z < 1e-6 { fallback }
    else { Quat::from_rotation_y(direction.x.atan2(direction.z)) }
}

fn plan(world: &mut World, choice: &PlaybackChoice) -> Result<Vec<ActorPose>, &'static str> {
    if world.get_resource::<crate::fixture_edit::EditSessionActive>().is_some_and(|editor| editor.is_active()) {
        return Err("请先退出家具布局编辑，再体验内容");
    }
    let epoch = world.get_resource::<GroundEpoch>().ok_or("场景还在准备，请稍候")?.0;
    let (units, required, pairs) = match choice.key {
        EntryKey::Talk(_, _) => {
            let row = world.resource::<LibraryCatalog>().talk(choice.key).ok_or("这段故事的数据暂不可用")?;
            let pairs = if let EntryKey::Talk(TalkBackend::Fixture, id) = choice.key {
                world.get_resource::<crate::talk::TalkStore>().and_then(|store| store.row(id))
                    .ok_or("这段演出的剧本还在准备")?.pairs.iter().map(|pair| (pair.unit_id as u32, pair.fixture_id)).collect::<Vec<_>>()
            } else { Vec::new() };
            (row.units.clone(), row.fixture_ids.clone(), pairs)
        }
        EntryKey::Fixture(id) => (Vec::new(), vec![id], Vec::new()),
    };
    let actual: Vec<_> = world.query::<(Entity, &FixtureActivityIdentity, &Transform)>().iter(world)
        .map(|(entity, identity, pose)| (entity, identity.clone(), *pose)).collect();
    let mut bindings = Vec::new();
    for id in &required {
        let mut candidates: Vec<_> = actual.iter().filter(|(_, identity, _)| identity.master_id == *id).collect();
        candidates.sort_by(|a,b| a.1.uid.cmp(&b.1.uid));
        let chosen = if let Some(target) = &choice.target {
            if actual.iter().any(|(entity, identity, _)| *entity == target.entity && target.matches(identity) && identity.master_id == *id) {
                candidates.iter().copied().find(|(entity, identity, _)| *entity == target.entity && target.matches(identity))
            } else { candidates.first().copied() }
        } else { candidates.first().copied() };
        let (entity, identity, pose) = chosen.ok_or("场景中缺少这段内容需要的家具，请先摆放")?;
        bindings.push((*id, *entity, identity.uid.clone(), *pose));
    }
    if let Some(target) = &choice.target {
        if !bindings.iter().any(|(_, entity, uid, _)| *entity == target.entity && uid == &target.uid) {
            return Err("所选家具已经移动或移除，请重新选择");
        }
    }
    let all_npcs: Vec<_> = world.query_filtered::<(Entity, &CharacterUnitId, &Transform, &NpcActions), Without<PlayerControlled>>()
        .iter(world).map(|(entity, unit, pose, actions)| (entity, unit.0, *pose, actions.ready())).collect();
    let players: Vec<_> = world.query_filtered::<(Entity, &Transform), With<PlayerControlled>>().iter(world)
        .map(|(entity, pose)| (entity, *pose)).collect();
    let [(player, player_pose)] = players.as_slice() else { return Err("玩家角色还没有准备好"); };
    let mut actors = Vec::new();
    for unit in &units {
        let Some((entity, _, pose, ready)) = all_npcs.iter().find(|(_, id, _, _)| id == unit) else { return Err("有登场角色不在当前场景"); };
        if !ready || world.get::<TalkHold>(*entity).is_some() { return Err("有登场角色正在进行其他互动，请稍候"); }
        actors.push(ActorPose { entity: *entity, before: *pose, after: *pose, npc: true });
    }
    if world.get::<TalkHold>(*player).is_some() { return Err("玩家正在另一段对话中，请稍候"); }
    let face = world.get_resource::<ObjectiveFace>().filter(|face| face.is_fresh(epoch)).ok_or("场景的行走区域还在准备，请稍候")?;
    let mut occupied: Vec<Vec3> = all_npcs.iter().filter(|(_, unit, _, _)| !units.contains(unit)).map(|(_, _, pose, _)| pose.translation).collect();
    let sample = |mut point: Vec3| { point.y = face.ref_y(); face.sample(point.to_array(), 0.15).map(Vec3::from) };
    // Conversations beside furniture borrow an arrangement only when needed.
    // A coherent nearby actor keeps its place; distant cast members are staged
    // at their own authored furniture pairing rather than at an invented ID.
    for (index, actor) in actors.iter_mut().enumerate() {
        let unit = units[index];
        let anchor = pairs.iter().find(|(member, _)| *member == unit)
            .and_then(|(_, id)| bindings.iter().find(|(fixture, _, _, _)| fixture == id));
        if let Some((_, _, _, fixture)) = anchor {
            let nearby = actor.before.translation.distance(fixture.translation) <= 1.8
                && occupied.iter().all(|other| actor.before.translation.distance(*other) >= 0.62)
                && sample(actor.before.translation).is_some();
            if !nearby {
                actor.after.translation = viewing_position(fixture.translation, fixture.rotation, &occupied, sample)
                    .ok_or("家具旁暂时没有足够的安全站位，请留出一些空间")?;
            }
            actor.after.rotation = facing(actor.after.translation, fixture.translation, actor.before.rotation);
        }
        occupied.push(actor.after.translation);
    }
    let anchor = actors.first().map(|actor| actor.after.translation)
        .or_else(|| bindings.first().map(|(_, _, _, pose)| pose.translation)).ok_or("没有可供体验的场景目标")?;
    let player_position = viewing_position(anchor, Quat::IDENTITY, &occupied, sample)
        .ok_or("目标附近暂时没有适合观看的位置，请留出一些空间")?;
    let mut after = *player_pose; after.translation = player_position; after.rotation = facing(player_position, anchor, after.rotation);
    actors.push(ActorPose { entity: *player, before: *player_pose, after, npc: false });
    Ok(actors)
}

pub(crate) fn prepare_pending(world: &mut World) {
    let state = world.resource::<ContentLibrary>();
    let Some(choice) = state.pending.clone().filter(|_| state.cleanup_frames == 0) else { return; };
    if !owners_idle(world) { return; }
    if let Some(preview) = world.get_resource::<ScenePreview>() {
        if preview.ticket == choice.ticket { return; }
        restore(world);
        world.resource_mut::<ContentLibrary>().active = None;
    }
    let actors = match plan(world, &choice) {
        Ok(actors) => actors,
        Err(reason) => { playback::fail(&mut world.resource_mut::<ContentLibrary>(), reason); return; }
    };
    let preview = ScenePreview { ticket: choice.ticket, epoch: world.resource::<GroundEpoch>().0,
        created_frame: world.resource::<FrameCount>().0, actors };
    for actor in &preview.actors {
        if actor.npc { crate::npc::stop_for_external_activity(world, actor.entity); }
        world.entity_mut(actor.entity).insert(actor.after);
        if let Some(mut walk) = world.get_mut::<WalkState>(actor.entity) {
            walk.0.position = actor.after.translation.to_array();
            walk.0.forward = (actor.after.rotation * Vec3::Z).to_array();
        }
        // This preparation hold lasts through transform propagation. Admission
        // replaces it with the existing talk owner's hold in the next update.
        if actor.npc { world.entity_mut(actor.entity).insert(TalkHold); }
    }
    info!("[content-library] staged ticket={} existing actors={} on scene={}", preview.ticket, preview.actors.len(), preview.epoch);
    world.insert_resource(preview);
}
fn restore(world: &mut World) {
    let Some(preview) = world.remove_resource::<ScenePreview>() else { return; };
    let same_site = world.get_resource::<GroundEpoch>().is_some_and(|epoch| epoch.0 == preview.epoch);
    for actor in preview.actors {
        if world.get_entity(actor.entity).is_err() { continue; }
        if actor.npc { world.entity_mut(actor.entity).remove::<TalkHold>(); }
        if !same_site { continue; }
        world.entity_mut(actor.entity).insert(actor.before);
        if actor.npc {
            if let Some(mut walk) = world.get_mut::<WalkState>(actor.entity) {
                walk.0.position = actor.before.translation.to_array();
                walk.0.forward = (actor.before.rotation * Vec3::Z).to_array();
            }
            world.entity_mut(actor.entity).remove::<TalkHold>();
        }
    }
    info!("[content-library] restored scene ticket={}", preview.ticket);
}
pub(crate) fn retire_scene(world: &mut World) {
    let Some(preview) = world.get_resource::<ScenePreview>() else { return; };
    let state = world.resource::<ContentLibrary>();
    let retained = state.pending.as_ref().is_some_and(|choice| choice.ticket == preview.ticket)
        || state.active.as_ref().is_some_and(|active| active.choice.ticket == preview.ticket);
    if !retained && owners_idle(world) { restore(world); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn blocked_navigation_never_fabricates_a_position() {
        assert!(viewing_position(Vec3::ZERO, Quat::IDENTITY, &[], |_| None).is_none());
    }
    #[test] fn stage_positions_are_separated_and_stay_near_the_real_anchor() {
        let mut occupied = Vec::new();
        for _ in 0..5 {
            let point = viewing_position(Vec3::ZERO, Quat::IDENTITY, &occupied, Some).unwrap();
            assert!(point.length() < 3.5);
            assert!(occupied.iter().all(|other| point.distance(*other) >= 0.62));
            occupied.push(point);
        }
    }
    #[test] fn restore_does_not_apply_old_coordinates_to_a_new_site() {
        let mut world = World::new(); world.insert_resource(GroundEpoch(2));
        let actor = world.spawn(Transform::from_xyz(20., 0., 20.)).id();
        world.insert_resource(ScenePreview { ticket: 1, epoch: 1, created_frame: 0,
            actors: vec![ActorPose { entity: actor, before: Transform::IDENTITY, after: Transform::IDENTITY, npc: false }] });
        restore(&mut world);
        assert_eq!(world.get::<Transform>(actor).unwrap().translation, Vec3::new(20.,0.,20.));
        assert!(!world.contains_resource::<ScenePreview>());
    }
}
