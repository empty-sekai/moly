//! Scoped scene preparation for the two product modes.
//!
//! Current-scene mode borrows only live UID-checked entities. Independent
//! mode arranges a stage set in the hub (its room, temporary layout through
//! the ordinary fixture pipeline, exact cast through the ordinary NPC
//! pipeline, and way back), waits for the furniture and characters the choice
//! needs, then stages them. No preview data is written to the editor or
//! settings store.
use super::*;
#[path = "camera_snapshot.rs"]
mod camera_snapshot;
use moly_features::{
    lease::{Holder, Rank},
    npc::WalkState,
    npc_objective::ObjectiveFace,
    player::PlayerControlled,
    room_appearance::RoomAppearance,
    site::GroundEpoch,
    stage_set::{self, SetPhase, SetSite},
    talk::TalkHold,
};
#[cfg(test)]
use moly_law::fixture::position::layout_type;
use moly_law::fixture::{Direction, GridPosition};

#[derive(Clone)]
struct ActorPose {
    entity: Entity,
    before: Transform,
    after: Transform,
    npc: bool,
}

#[derive(Resource)]
pub struct ScenePreview {
    ticket: u64,
    pub(super) epoch: u64,
    actors: Vec<ActorPose>,
    camera: camera_snapshot::CameraSnapshot,
}
impl ScenePreview {
    /// Whether every staged actor stands at its staged pose: natively while
    /// the choice's content stages, in the bridge before its scene starts.
    pub(super) fn ready(
        &self,
        choice: &PlaybackChoice,
        actors: &Query<(&Transform, &GlobalTransform)>,
    ) -> bool {
        self.ticket == choice.ticket
            && self.actors.iter().all(|actor| {
                actors.get(actor.entity).is_ok_and(|(pose, global)| {
                    pose == &actor.after
                        && global.translation().distance(actor.after.translation) < 0.01
                })
            })
    }
}

/// The library's side of an independent choice: the hub's stage set it is
/// arranged on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndependentPhase {
    SwitchingSite,
    LoadingContent,
    Staged,
    Experiencing,
    RequestingReturn,
    Returning,
    Returned,
    Failed,
}

#[derive(Resource, Clone)]
pub struct IndependentSession {
    /// The library choice the set is staged for.
    pub ticket: u64,
    /// The hub's set this session asked for, numbered by the set itself.
    set: u64,
    pub phase: IndependentPhase,
    /// The hub's set answered its furniture and cast ready and took the
    /// library's room surface.
    set_ready: bool,
}

impl IndependentSession {
    pub(super) fn phase(&self) -> IndependentPhase {
        self.phase
    }
}

/// Whether the shared owners a staging pass displaces are idle. The
/// library's own staged preview is not one of them: it is what the library
/// replaces or takes off itself.
fn owners_idle(world: &World) -> bool {
    stage_set::owners_idle(world, false)
}

fn viewing_position(
    center: Vec3,
    yaw: Quat,
    occupied: &[Vec3],
    mut sample: impl FnMut(Vec3) -> Option<Vec3>,
) -> Option<Vec3> {
    for radius in [0.95, 1.35, 1.8, 2.3, 2.9] {
        for turn in [0, 1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6] {
            let angle = turn as f32 * std::f32::consts::PI / 6.;
            let candidate =
                center + yaw * Vec3::new(angle.sin() * radius, 0., angle.cos() * radius);
            let Some(point) = sample(candidate) else {
                continue;
            };
            if point.is_finite()
                && point.distance(center) < 3.5
                && occupied.iter().all(|other| point.distance(*other) >= 0.62)
            {
                return Some(point);
            }
        }
    }
    None
}
fn observer_viewing_yaw(mode: ExperienceMode) -> Quat {
    if mode == ExperienceMode::Independent {
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_3)
    } else {
        Quat::IDENTITY
    }
}

fn observer_anchor(
    fixture: Option<Vec3>,
    actor: Option<Vec3>,
    independent_center: Option<Vec3>,
) -> Option<Vec3> {
    // A fixture-bound actor still has to approach its authored StartLoc after
    // this staging pass. Its temporary nearby pose is not the content center.
    fixture.or(actor).or(independent_center)
}

fn completed_observer_pose(
    final_anchor: Vec3,
    current: Transform,
    occupied: &[Vec3],
    sample: impl FnMut(Vec3) -> Option<Vec3>,
) -> Option<Transform> {
    if !final_anchor.is_finite() {
        return None;
    }
    let point = viewing_position(
        final_anchor,
        observer_viewing_yaw(ExperienceMode::Independent),
        occupied,
        sample,
    )?;
    Some(Transform {
        translation: point,
        rotation: facing(point, final_anchor, current.rotation),
        ..current
    })
}

fn replace_preview_observer(
    world: &mut World,
    ticket: u64,
    epoch: u64,
    player: Entity,
    after: Transform,
) -> bool {
    if world.get::<PlayerControlled>(player).is_none()
        || world.get::<Transform>(player).is_none()
        || world.get_resource::<GroundEpoch>().map(|value| value.0) != Some(epoch)
    {
        return false;
    }
    let Some(mut preview) = world.get_resource_mut::<ScenePreview>() else {
        return false;
    };
    if preview.ticket != ticket || preview.epoch != epoch {
        return false;
    }
    let Some(snapshot) = preview
        .actors
        .iter_mut()
        .find(|actor| actor.entity == player && !actor.npc)
    else {
        return false;
    };
    // The first staging snapshot is the sole restoration source. Handoff
    // updates only the expected staged pose, never the user's original pose.
    snapshot.after = after;
    world.entity_mut(player).insert(after);
    true
}

/// Stage once before the visible approach, using the retained plan's future
/// talk locator. Never move the observer at the dialogue/animation boundary.
/// Talk-enabled live loops retain their fixture-centered framing instead.
pub fn stage_fixture_talk_observer(
    world: &mut World,
    talk_id: i32,
    player: Entity,
    final_anchor: Vec3,
    planned_cast_positions: &[Vec3],
) -> Result<bool, String> {
    let Some(ticket) = world
        .get_resource::<ContentLibrary>()
        .and_then(|library| library.active.as_ref())
        .filter(|active| {
            active.choice.mode == ExperienceMode::Independent
                && active.choice.key == EntryKey::Talk(TalkBackend::Fixture, talk_id)
                && !active.choice.preview
                && !active.completed
        })
        .map(|active| active.choice.ticket)
    else {
        return Ok(false);
    };
    let Some(epoch) = world.get_resource::<ScenePreview>().and_then(|preview| {
        (preview.ticket == ticket
            && preview
                .actors
                .iter()
                .any(|actor| actor.entity == player && !actor.npc))
        .then_some(preview.epoch)
    }) else {
        return Ok(false);
    };
    if world.get_resource::<GroundEpoch>().map(|value| value.0) != Some(epoch)
        || world.get::<PlayerControlled>(player).is_none()
    {
        return Ok(false);
    }
    let current = world
        .get::<Transform>(player)
        .copied()
        .ok_or("独立体验的玩家站位已不可用")?;
    let mut occupied: Vec<_> = world
        .query_filtered::<(Entity, &Transform), With<CharacterUnitId>>()
        .iter(world)
        .filter(|(entity, _)| *entity != player)
        .map(|(_, pose)| pose.translation)
        .collect();
    occupied.extend_from_slice(planned_cast_positions);
    let after = {
        let face = world
            .get_resource::<ObjectiveFace>()
            .filter(|face| face.is_fresh(epoch))
            .ok_or("演出站位的行走区域尚未准备好")?;
        completed_observer_pose(final_anchor, current, &occupied, |mut point| {
            point.y = face.ref_y();
            let point = face.sample(point.to_array(), 0.15)?;
            face.has_path(current.translation.to_array(), point)
                .then_some(Vec3::from(point))
        })
        .ok_or("动作结束位置附近没有可安全到达的观察站位")?
    };
    Ok(replace_preview_observer(
        world, ticket, epoch, player, after,
    ))
}

fn facing(point: Vec3, target: Vec3, fallback: Quat) -> Quat {
    let direction = target - point;
    if direction.x * direction.x + direction.z * direction.z < 1e-6 {
        fallback
    } else {
        Quat::from_rotation_y(direction.x.atan2(direction.z))
    }
}

fn requirements(
    world: &World,
    choice: &PlaybackChoice,
) -> Result<(Vec<u32>, Vec<i32>, Vec<(u32, i32)>), String> {
    match choice.key {
        EntryKey::Activity(id) => {
            let row = world
                .resource::<LibraryCatalog>()
                .activity(id)
                .ok_or("所选角色互动的原始关系不可用")?;
            Ok((
                vec![row.spec.unit],
                vec![row.spec.fixture_id],
                vec![(row.spec.unit, row.spec.fixture_id)],
            ))
        }
        EntryKey::Talk(_, _) => {
            let row = world
                .resource::<LibraryCatalog>()
                .talk(choice.key)
                .ok_or_else(|| "这段故事的数据暂不可用".to_owned())?;
            let pairs = if let EntryKey::Talk(TalkBackend::Fixture, id) = choice.key {
                world
                    .get_resource::<moly_features::talk::TalkStore>()
                    .and_then(|store| store.row(id))
                    .ok_or_else(|| "这段演出的剧本还在准备".to_owned())?
                    .pairs
                    .iter()
                    .map(|pair| (pair.unit_id as u32, pair.fixture_id))
                    .collect()
            } else {
                Vec::new()
            };
            Ok((row.units.clone(), row.fixture_ids.clone(), pairs))
        }
        EntryKey::Fixture(id) => {
            let surface = world
                .resource::<LibraryCatalog>()
                .fixture(id)
                .is_some_and(|row| matches!(row.presentation, FixturePresentation::Surface { .. }));
            Ok((
                Vec::new(),
                if surface { Vec::new() } else { vec![id] },
                Vec::new(),
            ))
        }
    }
}

fn preview_rows(
    catalog: &LibraryCatalog,
    ids: &[i32],
    ticket: u64,
) -> Result<Vec<moly_features::fixture::EditableFixture>, String> {
    let mut rows = Vec::new();
    for (index, id) in ids.iter().copied().enumerate() {
        let fixture = catalog
            .fixture(id)
            .ok_or_else(|| format!("家具 {id} 已不在当前来源中"))?;
        let source = fixture
            .source
            .as_ref()
            .filter(|source| source.exported)
            .ok_or_else(|| format!("{} 的原始模型尚未完整导出", fixture.name))?;
        let x = i8::try_from(index.saturating_mul(12))
            .map_err(|_| "这段内容需要的家具过多，无法安全布置".to_owned())?;
        let z = if source.layout & moly_law::fixture::position::WALL_LAYOUT_MASK != 0 {
            -12
        } else {
            0
        };
        rows.push(moly_features::fixture::EditableFixture {
            uid: format!("library-{ticket}-{id}-{index}"),
            package: source.package.clone(),
            texture_id: 1,
            fixture_id: id,
            center: GridPosition::new(x, source.center_y, z),
            grid_size: source.grid_size,
            layout: source.layout,
            direction: Direction::Front,
        });
    }
    Ok(rows)
}

fn plan_current(world: &mut World, choice: &PlaybackChoice) -> Result<Vec<ActorPose>, String> {
    if world
        .get_resource::<moly_features::field_modes::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return Err("请先退出家具布局编辑，再体验内容".into());
    }
    let epoch = world
        .get_resource::<GroundEpoch>()
        .ok_or("场景还在准备，请稍候")?
        .0;
    let (units, required, pairs) = requirements(world, choice)?;
    let actual: Vec<_> = world
        .query::<(Entity, &FixtureActivityIdentity, &Transform)>()
        .iter(world)
        .map(|(entity, identity, pose)| (entity, identity.clone(), *pose))
        .collect();
    let mut bindings = Vec::new();
    for id in &required {
        if world
            .resource::<LibraryCatalog>()
            .fixture(*id)
            .is_some_and(|row| matches!(row.presentation, FixturePresentation::Surface { .. }))
        {
            continue;
        }
        let mut candidates: Vec<_> = actual
            .iter()
            .filter(|(_, identity, _)| identity.master_id == *id)
            .collect();
        candidates.sort_by(|a, b| a.1.uid.cmp(&b.1.uid));
        let chosen = if let Some(target) = &choice.target {
            if actual.iter().any(|(entity, identity, _)| {
                *entity == target.entity && target.matches(identity) && identity.master_id == *id
            }) {
                candidates.iter().copied().find(|(entity, identity, _)| {
                    *entity == target.entity && target.matches(identity)
                })
            } else {
                candidates.first().copied()
            }
        } else {
            candidates.first().copied()
        };
        let (entity, identity, pose) =
            chosen.ok_or_else(|| "场景中缺少这段内容需要的家具".to_owned())?;
        bindings.push((*id, *entity, identity.uid.clone(), *pose));
    }
    if let Some(target) = &choice.target {
        if !bindings
            .iter()
            .any(|(_, entity, uid, _)| *entity == target.entity && uid == &target.uid)
        {
            return Err("所选家具已经移动或移除，请重新选择".into());
        }
    }
    let all_npcs: Vec<_> = world.query_filtered::<(Entity, &CharacterUnitId, &Transform, &NpcActions), Without<PlayerControlled>>()
        .iter(world).map(|(entity, unit, pose, actions)| (entity, unit.0, *pose, actions.ready())).collect();
    let players: Vec<_> = world
        .query_filtered::<(Entity, &Transform), With<PlayerControlled>>()
        .iter(world)
        .map(|(entity, pose)| (entity, *pose))
        .collect();
    let [(player, player_pose)] = players.as_slice() else {
        return Err("玩家角色还没有准备好".into());
    };
    let mut actors = Vec::new();
    for unit in &units {
        let Some((entity, _, pose, ready)) = all_npcs.iter().find(|(_, id, _, _)| id == unit)
        else {
            return Err("有登场角色尚未进入场景".into());
        };
        if !ready || world.get::<TalkHold>(*entity).is_some() {
            return Err("有登场角色正在进行其他互动，请稍候".into());
        }
        actors.push(ActorPose {
            entity: *entity,
            before: *pose,
            after: *pose,
            npc: true,
        });
    }
    if world.get::<TalkHold>(*player).is_some() {
        return Err("玩家正在另一段对话中，请稍候".into());
    }
    let face = world
        .get_resource::<ObjectiveFace>()
        .filter(|face| face.is_fresh(epoch))
        .ok_or("场景的行走区域还在准备，请稍候")?;
    let mut occupied: Vec<Vec3> = all_npcs
        .iter()
        .filter(|(_, unit, _, _)| !units.contains(unit))
        .map(|(_, _, pose, _)| pose.translation)
        .collect();
    let sample = |mut point: Vec3| {
        point.y = face.ref_y();
        face.sample(point.to_array(), 0.15).map(Vec3::from)
    };
    let mut independent_group_anchor = None;
    let independent_center = (choice.mode == ExperienceMode::Independent)
        .then(|| {
            bindings
                .first()
                .map(|(_, _, _, pose)| pose.translation)
                .or_else(|| face.preview_center().map(Vec3::from))
        })
        .flatten();
    for (index, actor) in actors.iter_mut().enumerate() {
        let unit = units[index];
        let anchor = pairs
            .iter()
            .find(|(member, _)| *member == unit)
            .and_then(|(_, id)| bindings.iter().find(|(fixture, _, _, _)| fixture == id));
        if let Some((_, _, _, fixture)) = anchor {
            let nearby = actor.before.translation.distance(fixture.translation) <= 1.8
                && occupied
                    .iter()
                    .all(|other| actor.before.translation.distance(*other) >= 0.62)
                && sample(actor.before.translation).is_some();
            if !nearby {
                actor.after.translation =
                    viewing_position(fixture.translation, fixture.rotation, &occupied, sample)
                        .ok_or("家具旁暂时没有足够的安全站位")?;
            }
            actor.after.rotation = facing(
                actor.after.translation,
                fixture.translation,
                actor.before.rotation,
            );
            independent_group_anchor.get_or_insert(actor.after.translation);
        } else if choice.mode == ExperienceMode::Independent {
            if index == 0 {
                actor.after.translation = independent_center.ok_or("独立空间中央没有安全站位")?;
                actor.after.rotation = Quat::IDENTITY;
            }
            if let Some(group_anchor) = independent_group_anchor {
                let nearby = actor.before.translation.distance(group_anchor) <= 1.8
                    && occupied
                        .iter()
                        .all(|other| actor.before.translation.distance(*other) >= 0.62)
                    && sample(actor.before.translation).is_some();
                if !nearby {
                    actor.after.translation =
                        viewing_position(group_anchor, Quat::IDENTITY, &occupied, sample)
                            .ok_or("登场角色附近暂时没有足够的安全站位")?;
                }
                actor.after.rotation =
                    facing(actor.after.translation, group_anchor, actor.before.rotation);
            } else {
                independent_group_anchor = Some(actor.after.translation);
            }
        }
        occupied.push(actor.after.translation);
    }
    let fixture_anchor = choice
        .target
        .as_ref()
        .and_then(|target| {
            bindings
                .iter()
                .find(|(_, entity, uid, _)| *entity == target.entity && uid == &target.uid)
        })
        .or_else(|| bindings.first())
        .map(|(_, _, _, pose)| pose.translation);
    let anchor = observer_anchor(
        fixture_anchor,
        actors.first().map(|actor| actor.after.translation),
        independent_center,
    )
    .ok_or("没有可供体验的场景目标")?;
    // The independent observer must not stand directly between the normal
    // follow camera and a small table/lamp. Preserve the real safe-radius and
    // navigation sampler, but prefer an oblique side of the content. This is
    // staging composition only; the original scene and authored actor paths
    // retain their owners and are restored from the same snapshot.
    let observer_yaw = observer_viewing_yaw(choice.mode);
    let player_position = viewing_position(anchor, observer_yaw, &occupied, sample)
        .ok_or("目标附近暂时没有适合观看的位置")?;
    let mut after = *player_pose;
    after.translation = player_position;
    after.rotation = facing(player_position, anchor, after.rotation);
    actors.push(ActorPose {
        entity: *player,
        before: *player_pose,
        after,
        npc: false,
    });
    Ok(actors)
}

fn apply_preview(world: &mut World, choice: &PlaybackChoice, actors: Vec<ActorPose>) {
    let preview = ScenePreview {
        ticket: choice.ticket,
        epoch: world.resource::<GroundEpoch>().0,
        actors,
        camera: camera_snapshot::CameraSnapshot::capture(world),
    };
    for actor in &preview.actors {
        if actor.npc {
            moly_features::npc::stop_for_external_activity(world, actor.entity);
        }
        world.entity_mut(actor.entity).insert(actor.after);
        if let Some(mut walk) = world.get_mut::<WalkState>(actor.entity) {
            walk.0.position = actor.after.translation.to_array();
            walk.0.forward = (actor.after.rotation * Vec3::Z).to_array();
        }
        if actor.npc {
            world.entity_mut(actor.entity).insert(TalkHold);
        }
    }
    if let Some(player) = preview.actors.iter().find(|actor| !actor.npc) {
        super::stage_framing::prepare(world, choice, player.after);
    }
    info!(
        "[content-library] staged ticket={} actors={} epoch={}",
        preview.ticket,
        preview.actors.len(),
        preview.epoch
    );
    world.insert_resource(preview);
}

/// The holder the library arranges its sets as.
fn set_requester() -> Holder {
    Holder::new(CONTENT_LIBRARY, Rank::Human)
}

/// Furniture and character actions belong in a clean courtyard, not an
/// unrelated indoor room. A wallpaper or floor swatch needs an actual room
/// surface and is the only semantic exception, never a fabricated mesh.
fn set_site(world: &World, key: EntryKey) -> SetSite {
    let surface = matches!(key, EntryKey::Fixture(id)
        if world.resource::<LibraryCatalog>().fixture(id)
            .is_some_and(|row| matches!(row.presentation, FixturePresentation::Surface { .. })));
    if surface {
        SetSite::Room
    } else {
        SetSite::Courtyard
    }
}

/// Arranges the hub's set for `choice`.
fn start_independent(world: &mut World, choice: &PlaybackChoice) -> Result<(), String> {
    let (units, fixtures, _) = requirements(world, choice)?;
    let rows = preview_rows(world.resource::<LibraryCatalog>(), &fixtures, choice.ticket)?;
    let request = stage_set::SetRequest {
        requester: set_requester(),
        site: set_site(world, choice.key),
        units,
        fixtures: rows,
    };
    let set = stage_set::StageSet::request(world, request)?;
    world.insert_resource(IndependentSession {
        ticket: choice.ticket,
        set,
        phase: IndependentPhase::SwitchingSite,
        set_ready: false,
    });
    library_mut(world, |state| {
        state.scene_owned = true;
        state.last_error = None;
        state.status = "正在前往独立空场景…".into();
        state.changed();
    });
    Ok(())
}

/// The room surface a wallpaper or floor swatch stages over the room's own.
fn surface_appearance(world: &World, key: EntryKey) -> Option<RoomAppearance> {
    let EntryKey::Fixture(id) = key else {
        return None;
    };
    let presentation = world
        .resource::<LibraryCatalog>()
        .fixture(id)
        .map(|row| row.presentation.clone());
    let Some(FixturePresentation::Surface { skin, wall, .. }) = presentation else {
        return None;
    };
    let mut current = world.resource::<RoomAppearance>().clone();
    if wall {
        current.wall = skin;
    } else {
        current.floor = skin;
    }
    Some(current)
}

/// The library's choice a set's ticket names, pending or active.
fn choice_of(world: &World, ticket: u64) -> Option<PlaybackChoice> {
    let state = world.resource::<ContentLibrary>();
    state
        .pending
        .clone()
        .or_else(|| state.active.as_ref().map(|active| active.choice.clone()))
        .filter(|choice| choice.ticket == ticket)
}

fn matching_fixture_targets(world: &mut World, required: &[i32]) -> Option<Vec<FixtureTarget>> {
    let mut found = Vec::new();
    for id in required {
        if world
            .resource::<LibraryCatalog>()
            .fixture(*id)
            .is_some_and(|row| matches!(row.presentation, FixturePresentation::Surface { .. }))
        {
            continue;
        }
        let mut rows: Vec<_> = world
            .query::<(Entity, &FixtureActivityIdentity)>()
            .iter(world)
            .filter(|(_, identity)| identity.master_id == *id)
            .map(|(entity, identity)| FixtureTarget {
                entity,
                uid: identity.uid.clone(),
            })
            .collect();
        rows.sort_by(|a, b| a.uid.cmp(&b.uid));
        found.push(rows.into_iter().next()?);
    }
    Some(found)
}

/// Stages the choice a set's ticket names on the set's published furniture:
/// its target, then the planned cast and observer poses. `None` while the
/// placed furniture is not there yet.
fn stage_choice(world: &mut World, ticket: u64, set: u64) -> Option<bool> {
    // A staged preview is placed once per set.
    if world.contains_resource::<ScenePreview>() {
        return None;
    }
    let mut choice = choice_of(world, ticket)?;
    let (_, fixtures, _) = match requirements(world, &choice) {
        Ok(required) => required,
        Err(reason) => {
            lose_independent(world, set, &reason);
            return Some(false);
        }
    };
    let targets = matching_fixture_targets(world, &fixtures)?;
    choice.target = targets.first().cloned();
    library_mut(world, |state| state.pending = Some(choice.clone()));
    // Waiting for the content's own resources reports why it cannot go on.
    let failed = world.resource::<ContentLibrary>().last_error.clone();
    if let Some(reason) = failed {
        lose_independent(world, set, &reason);
        return Some(false);
    }
    match plan_choice(world, &choice) {
        Ok(actors) => {
            apply_preview(world, &choice, actors);
            Some(true)
        }
        Err(reason) => {
            lose_independent(world, set, &reason);
            Some(false)
        }
    }
}

/// Plans the choice's cast and observer poses on the set; a failure is
/// reported to the caller by name, never failed here.
fn plan_choice(world: &mut World, choice: &PlaybackChoice) -> Result<Vec<ActorPose>, String> {
    plan_current(world, choice)
}

/// Ends an independent choice by name: the library fails it and the set
/// returns and restores the original scene.
fn lose_independent(world: &mut World, set: u64, reason: &str) {
    library_mut(world, |state| playback::fail(state, reason));
    stage_set::StageSet::fail(
        world,
        set,
        playback::human_playback_failure(reason),
    );
}

/// Ends the independent session once its set has ended and the library's
/// staged poses come off. The status stays as published: before the way back,
/// or the failure that ended the set.
fn finish_independent(world: &mut World) {
    world.remove_resource::<ScenePreview>();
}

/// Whether the library still holds a choice of its own on the set: the
/// choice is waiting, running, or finished and kept for a replay or the next
/// choice. A stop takes it off once its owners are idle.
fn plays_choice(world: &World, ticket: u64) -> bool {
    let state = world.resource::<ContentLibrary>();
    state.pending.as_ref().is_some_and(|choice| choice.ticket == ticket)
        || state
            .active
            .as_ref()
            .is_some_and(|active| active.choice.ticket == ticket)
}

fn restore_preview(world: &mut World) {
    let Some(preview) = world.remove_resource::<ScenePreview>() else {
        return;
    };
    let same_site = world
        .get_resource::<GroundEpoch>()
        .is_some_and(|epoch| epoch.0 == preview.epoch);
    for actor in preview.actors {
        if world.get_entity(actor.entity).is_err() {
            continue;
        }
        if actor.npc {
            world.entity_mut(actor.entity).remove::<TalkHold>();
        }
        if !same_site {
            continue;
        }
        world.entity_mut(actor.entity).insert(actor.before);
        if actor.npc {
            if let Some(mut walk) = world.get_mut::<WalkState>(actor.entity) {
                walk.0.position = actor.before.translation.to_array();
                walk.0.forward = (actor.before.rotation * Vec3::Z).to_array();
            }
        }
    }
    if same_site {
        preview.camera.restore(world);
    }
    info!(
        "[content-library] restored staged poses ticket={}",
        preview.ticket
    );
}

/// The replacement path: a pending independent choice with a ticket of its
/// own, while this set already stands, reuses it in place when it needs the
/// same furniture and is not a surface. `true` hands the frame to the new
/// choice, the session already put back.
fn reuse_set(
    world: &mut World,
    session: &IndependentSession,
    phase: &Option<SetPhase>,
) -> bool {
    let Some(plan) = replacement(world, session, phase) else {
        return false;
    };
    if !owners_idle(world) {
        world.insert_resource(session.clone());
        return true;
    }
    restore_preview(world);
    // The cast is swapped before anything is staged, so a spawn that fails
    // leaves the set to be restored rather than staged with the old cast.
    if let Err(reason) = stage_set::StageSet::recast(world, session.set, plan.units) {
        lose_independent(world, session.set, &reason);
        // The session stays to drive the set back.
        world.insert_resource(session.clone());
        return true;
    }
    let mut next = plan.choice;
    next.target = matching_fixture_targets(world, &plan.fixtures)
        .and_then(|targets| targets.first().cloned());
    library_mut(world, |state| {
        state.active = None;
        state.pending = Some(next.clone());
        state.stopping = false;
        state.status = "已复用独立场景，正在切换内容…".into();
        state.changed();
    });
    let mut reused = session.clone();
    reused.ticket = next.ticket;
    // The re-cast set loads its new cast again before it stands.
    reused.set_ready = false;
    reused.phase = IndependentPhase::LoadingContent;
    info!(
        "[content-library] reused independent world ticket={} fixtures={:?}",
        reused.ticket, plan.fixtures
    );
    world.insert_resource(reused);
    true
}

/// What the replacement path swaps in, and the choice it stages.
struct Replacement {
    choice: PlaybackChoice,
    units: Vec<u32>,
    fixtures: Vec<i32>,
}

/// The pending independent choice this set can take in place: the same
/// furniture, no surface, and a set standing, loading or playing. Its target
/// is left to the caller, which is the first to see the new furniture.
fn replacement(
    world: &World,
    session: &IndependentSession,
    phase: &Option<SetPhase>,
) -> Option<Replacement> {
    if !matches!(
        session.phase,
        IndependentPhase::Staged
            | IndependentPhase::Experiencing
            | IndependentPhase::LoadingContent
    ) {
        return None;
    }
    if !phase
        .as_ref()
        .is_some_and(|phase| phase.is_running())
    {
        return None;
    }
    let choice = world
        .resource::<ContentLibrary>()
        .pending
        .clone()
        .filter(|choice| {
            choice.mode == ExperienceMode::Independent && choice.ticket != session.ticket
        })?;
    let set = stage_set::StageSet::session(world)?;
    if set.ticket() != session.set {
        return None;
    }
    let (units, fixtures, _) = requirements(world, &choice).ok()?;
    let surface = matches!(choice.key, EntryKey::Fixture(id)
        if world.resource::<LibraryCatalog>().fixture(id)
            .is_some_and(|row| matches!(row.presentation, FixturePresentation::Surface { .. })));
    let compatible = !surface
        && fixtures.len() == set.rows().len()
        && fixtures
            .iter()
            .all(|id| set.rows().iter().any(|row| row.fixture_id == *id));
    if !compatible {
        return None;
    }
    Some(Replacement {
        choice,
        units,
        fixtures,
    })
}

fn set_restore(world: &mut World, set: u64) {
    stage_set::StageSet::restore(world, set);
}

fn set_drive(world: &mut World, set: u64) {
    stage_set::StageSet::drive(
        world,
        set,
        Some(stage_set::Content {
            appearance: None,
            held: world.contains_resource::<ScenePreview>(),
        }),
    );
}

/// The library's steps for an independent choice through the hub's set, run
/// every frame whatever the pending choice is. The library waits for the
/// furniture and characters the choice needs and places it; while the choice
/// plays, the set keeps it. A new pending choice with the same furniture
/// reuses the set in place. When the choice ends, or another program takes
/// the set over, the library takes its staged poses off and hands the scene
/// over; the set returns and restores the original scene.
fn drive_independent(world: &mut World) {
    let Some(mut session) = world.remove_resource::<IndependentSession>() else {
        return;
    };
    let ticket = session.ticket;
    let set = session.set;
    let phase = stage_set::StageSet::phase(world, set).cloned();
    if let Some(SetPhase::Failed(reason)) = &phase {
        // A set that lost itself fails the choice by name, unless the
        // library failed it already.
        if session.phase != IndependentPhase::RequestingReturn {
            lose_independent(world, set, reason);
        }
        let reason = reason.clone();
        library_mut(world, |state| {
            state.stopping = false;
            state.scene_owned = false;
            state.status = format!("{reason}；已恢复原场景");
            state.changed();
        });
        stage_set::StageSet::discard(world, set);
        finish_independent(world);
        return;
    }
    if phase == Some(SetPhase::Returned) {
        library_mut(world, |state| {
            state.stopping = false;
            state.scene_owned = false;
            state.status = "已返回原场景，位置与临时内容均已恢复".into();
            state.changed();
        });
        info!("[content-library] independent ticket={ticket} returned with set {set}");
        stage_set::StageSet::discard(world, set);
        finish_independent(world);
        return;
    }
    if reuse_set(world, &session, &phase) {
        return;
    }
    if phase.is_some()
        && !matches!(
            session.phase,
            IndependentPhase::RequestingReturn | IndependentPhase::Returning
        )
        && !plays_choice(world, ticket)
    {
        session.phase = IndependentPhase::RequestingReturn;
        restore_preview(world);
        library_mut(world, |state| {
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.choice.ticket == ticket)
            {
                state.active = None;
            }
            state.stopping = false;
            // The scene stays the library's until the set is back: the
            // way back to the same site reloads it only while it is owned.
            state.status = "独立场景已结束，位置与临时内容均已恢复".into();
            state.changed();
        });
    }
    // Nothing below starts the way back: the returning phase only advances
    // the set's own steps, without which it would never come back.
    match session.phase {
        IndependentPhase::SwitchingSite | IndependentPhase::LoadingContent => {
            if !session.set_ready {
                let content = stage_set::Content {
                    appearance: choice_of(world, ticket)
                        .and_then(|choice| surface_appearance(world, choice.key)),
                    held: world.contains_resource::<ScenePreview>(),
                };
                session.set_ready = stage_set::StageSet::finish(world, set, content);
            }
            if session.set_ready {
                match stage_choice(world, ticket, set) {
                    Some(true) => session.phase = IndependentPhase::Staged,
                    Some(false) => session.phase = IndependentPhase::RequestingReturn,
                    None => {
                        session.phase = IndependentPhase::LoadingContent;
                        library_mut(world, |state| {
                            state.status = "空场景已就绪，正在载入所需角色与家具…".into();
                            state.changed();
                        });
                    }
                }
            } else if session.phase == IndependentPhase::SwitchingSite
                && phase != Some(SetPhase::SwitchingSite)
            {
                session.phase = IndependentPhase::LoadingContent;
                library_mut(world, |state| {
                    state.status = "空场景已就绪，正在载入所需角色与家具…".into();
                    state.changed();
                });
            }
        }
        IndependentPhase::Staged => {
            if world
                .resource::<ContentLibrary>()
                .active
                .as_ref()
                .is_some_and(|active| active.choice.ticket == ticket && active.started)
            {
                session.phase = IndependentPhase::Experiencing;
            }
        }
        IndependentPhase::Experiencing => {}
        IndependentPhase::RequestingReturn => {
            if owners_idle(world) {
                let withdrawn = !world.resource::<ContentLibrary>().status.ends_with("已恢复原场景");
                if withdrawn {
                    library_mut(world, |state| {
                        if state
                            .active
                            .as_ref()
                            .is_some_and(|active| active.choice.ticket == ticket)
                        {
                            state.active = None;
                        }
                        state.status = "正在返回原场景并恢复位置…".into();
                        state.changed();
                    });
                }
                set_restore(world, set);
                // The library's own content is done: the set's owners were
                // idle as it was asked to go back.
                session.phase = IndependentPhase::Returning;
            }
        }
        IndependentPhase::Returning => {
            // The way back runs on the set's own steps, whatever the owners
            // report: a set that stops being driven never ends.
            set_drive(world, set);
        }
        IndependentPhase::Returned | IndependentPhase::Failed => {}
    }
    if stage_set::StageSet::phase(world, set).is_some() {
        world.insert_resource(session);
    } else {
        finish_independent(world);
    }
}

/// The library's steps for an independent set, run every frame whatever the
/// pending choice is: another program may finish with a set while the library
/// owns it.
pub fn push_independent(world: &mut World) {
    drive_independent(world);
}

pub fn prepare_pending(world: &mut World) {
    let state = world.resource::<ContentLibrary>();
    let Some(choice) = state.pending.clone().filter(|_| state.cleanup_frames == 0) else {
        return;
    };
    if !owners_idle(world) {
        return;
    }
    match choice.mode {
        ExperienceMode::Independent => {
            // A set still held for another ticket hands the choice over in
            // its own steps; a new set is arranged only when none is held.
            let held = world
                .get_resource::<stage_set::SetState>()
                .is_some_and(|set| set.session().is_some());
            if !held {
                if let Err(reason) = start_independent(world, &choice) {
                    library_mut(world, |state| playback::fail(state, &reason));
                }
            }
        }
        ExperienceMode::CurrentScene => {
            if world.contains_resource::<IndependentSession>() {
                return;
            }
            if let Some(preview) = world.get_resource::<ScenePreview>() {
                if preview.ticket == choice.ticket {
                    return;
                }
                restore_preview(world);
                library_mut(world, |state| state.active = None);
            }
            match plan_current(world, &choice) {
                Ok(actors) => apply_preview(world, &choice, actors),
                Err(reason) => library_mut(world, |state| playback::fail(state, &reason)),
            }
        }
    }
}

pub fn retire_scene(world: &mut World) {
    // An independent session keeps its own preview until its set ends.
    if world.contains_resource::<IndependentSession>() {
        return;
    }
    let Some(preview) = world.get_resource::<ScenePreview>() else {
        return;
    };
    let state = world.resource::<ContentLibrary>();
    let retained = state
        .pending
        .as_ref()
        .is_some_and(|choice| choice.ticket == preview.ticket)
        || state
            .active
            .as_ref()
            .is_some_and(|active| active.choice.ticket == preview.ticket);
    if !retained && owners_idle(world) {
        restore_preview(world);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_observer_does_not_follow_a_temporary_actor_approach_pose() {
        let fixture = Vec3::new(0.125, 0., 0.125);
        let temporary_actor = fixture + Vec3::Z * 0.95;
        let source_start = Vec3::new(0.625, 0., -0.775);
        let yaw = observer_viewing_yaw(ExperienceMode::Independent);
        let before = observer_anchor(Some(fixture), Some(temporary_actor), None).unwrap();
        let after = observer_anchor(Some(fixture), Some(source_start), None).unwrap();
        assert_eq!(before, fixture);
        assert_eq!(after, fixture);
        let point = viewing_position(before, yaw, &[temporary_actor], Some).unwrap();
        assert!((point.z - 0.6).abs() < 1e-5);
        assert!((point.distance(fixture) - 0.95).abs() < 1e-5);
        assert!(point.distance(temporary_actor) >= 0.62);
        assert!(point.distance(source_start) >= 0.62);
        assert_eq!(
            point,
            viewing_position(after, yaw, &[temporary_actor], Some).unwrap()
        );
    }

    #[test]
    fn observer_anchor_preserves_nonfixture_talk_and_independent_fallbacks() {
        let actor = Vec3::new(2., 0., 3.);
        let center = Vec3::new(4., 0., 5.);
        assert_eq!(
            observer_anchor(None, Some(actor), Some(center)),
            Some(actor)
        );
        assert_eq!(observer_anchor(None, None, Some(center)), Some(center));
        assert_eq!(observer_anchor(None, None, None), None);
    }

    #[test]
    fn fixture_observer_still_requires_navigation_and_actor_clearance() {
        let center = Vec3::new(0.125, 0., 0.125);
        let yaw = observer_viewing_yaw(ExperienceMode::Independent);
        let occupied = center + yaw * Vec3::Z * 0.95;
        let point = viewing_position(center, yaw, &[occupied], Some).unwrap();
        assert!(point.distance(occupied) >= 0.62);
        assert!(point.distance(center) < 3.5);
        assert!(viewing_position(center, yaw, &[occupied], |_| None).is_none());
    }

    #[test]
    fn completed_observer_uses_final_action_anchor_not_the_previous_staging_pose() {
        let initial = Transform::from_xyz(0.947724, 0., 1.55);
        let final_anchor = Vec3::new(0.625, 0., -0.775);
        let pose = completed_observer_pose(final_anchor, initial, &[], Some).unwrap();
        assert!((pose.translation.x - 1.447724).abs() < 1e-5);
        assert!((pose.translation.z + 0.3).abs() < 1e-5);
        assert!(pose.translation.distance(final_anchor) < 1.0);
        assert!(pose.rotation.is_normalized());
        assert_ne!(pose.translation, initial.translation);
    }

    #[test]
    fn completed_observer_handoff_does_not_fabricate_a_blocked_route() {
        let initial = Transform::from_xyz(0.947724, 0., 1.55);
        assert!(
            completed_observer_pose(Vec3::new(0.625, 0., -0.775), initial, &[], |_| None,)
                .is_none()
        );
    }

    #[test]
    fn completed_handoff_preserves_the_original_player_restore_snapshot() {
        let mut world = World::new();
        world.insert_resource(GroundEpoch(7));
        let before = Transform::from_xyz(4., 0., 6.);
        let staged = Transform::from_xyz(0.947724, 0., 0.6);
        let completed = Transform::from_xyz(1.447724, 0., -0.3);
        let player = world
            .spawn((PlayerControlled, staged, GlobalTransform::default()))
            .id();
        let camera = camera_snapshot::CameraSnapshot::capture(&mut world);
        world.insert_resource(ScenePreview {
            ticket: 12,
            epoch: 7,
            actors: vec![ActorPose {
                entity: player,
                before,
                after: staged,
                npc: false,
            }],
            camera,
        });
        assert!(!replace_preview_observer(
            &mut world, 11, 7, player, completed
        ));
        assert!(!replace_preview_observer(
            &mut world, 12, 6, player, completed
        ));
        assert_eq!(*world.get::<Transform>(player).unwrap(), staged);
        assert!(replace_preview_observer(
            &mut world, 12, 7, player, completed
        ));
        let snapshot = &world.resource::<ScenePreview>().actors[0];
        assert_eq!(snapshot.before, before);
        assert_eq!(snapshot.after, completed);
        assert_eq!(*world.get::<Transform>(player).unwrap(), completed);
        restore_preview(&mut world);
        assert_eq!(*world.get::<Transform>(player).unwrap(), before);
    }

    #[test]
    fn independent_observer_is_not_collinear_with_the_normal_camera_and_content() {
        let yaw = observer_viewing_yaw(ExperienceMode::Independent);
        let point = viewing_position(Vec3::ZERO, yaw, &[], Some).unwrap();
        assert!(point.x.abs() > 0.7);
        assert!((point.length() - 0.95).abs() < 1e-5);
        let current = viewing_position(
            Vec3::ZERO,
            observer_viewing_yaw(ExperienceMode::CurrentScene),
            &[],
            Some,
        )
        .unwrap();
        assert!(current.x.abs() < 1e-5);
        assert!(viewing_position(Vec3::ZERO, yaw, &[], |_| None).is_none());
    }
    #[test]
    fn blocked_navigation_never_fabricates_a_position() {
        assert!(viewing_position(Vec3::ZERO, Quat::IDENTITY, &[], |_| None).is_none());
    }
    #[test]
    fn stage_positions_are_separated_and_stay_near_the_real_anchor() {
        let mut occupied = Vec::new();
        for _ in 0..5 {
            let point = viewing_position(Vec3::ZERO, Quat::IDENTITY, &occupied, Some).unwrap();
            assert!(point.length() < 3.5);
            assert!(occupied.iter().all(|other| point.distance(*other) >= 0.62));
            occupied.push(point);
        }
    }
    #[test]
    fn restore_does_not_apply_old_coordinates_to_a_new_site() {
        let mut world = World::new();
        world.insert_resource(GroundEpoch(2));
        let actor = world
            .spawn((
                Transform::from_xyz(20., 0., 20.),
                GlobalTransform::default(),
            ))
            .id();
        let camera_snapshot = camera_snapshot::CameraSnapshot::capture(&mut world);
        world.insert_resource(ScenePreview {
            ticket: 1,
            epoch: 1,
            camera: camera_snapshot,
            actors: vec![ActorPose {
                entity: actor,
                before: Transform::IDENTITY,
                after: Transform::IDENTITY,
                npc: false,
            }],
        });
        restore_preview(&mut world);
        assert_eq!(
            world.get::<Transform>(actor).unwrap().translation,
            Vec3::new(20., 0., 20.)
        );
        assert!(!world.contains_resource::<ScenePreview>());
    }

    #[test]
    fn static_and_interactive_furniture_use_the_same_exact_source_placement_path() {
        let mut catalog = LibraryCatalog::default();
        for (id, action, package) in [
            (7, "no_action", "mysekai__fixture__static"),
            (8, "timeline", "mysekai__fixture__chair"),
        ] {
            catalog.fixtures.push(LibraryFixture {
                id,
                name: format!("家具{id}"),
                description: String::new(),
                action: action.into(),
                search: String::new(),
                thumbnail: None,
                presentation: FixturePresentation::Model,
                source: Some(FixtureSource {
                    package: package.into(),
                    grid_size: moly_law::fixture::Vector3Int::new(2, 2, 2),
                    exported: true,
                    layout: layout_type::FLOOR,
                    center_y: 0,
                }),
            });
        }
        let rows = preview_rows(&catalog, &[7, 8], 42).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].fixture_id, 7);
        assert_eq!(rows[0].package, "mysekai__fixture__static");
        assert_eq!(rows[1].package, "mysekai__fixture__chair");
        assert!(rows.iter().all(|row| row.texture_id == 1));
    }
}

#[cfg(test)]
mod lifecycle_regressions {
    use super::*;
    #[test]
    fn scene_readiness_is_not_the_consumed_camera_or_scene_event_latch() {
        let mut world = World::new();
        world.insert_resource(moly_features::site::SiteScenesReady);
        world.insert_resource(GroundEpoch(2));
        assert!(!world.contains_resource::<moly_features::site::SiteReady>());
        assert!(!world.contains_resource::<moly_features::inactive_nodes::SiteSettled>());
        assert!(world.contains_resource::<moly_features::site::SiteScenesReady>());
        assert_eq!(world.resource::<GroundEpoch>().0, 2);
    }
    #[test]
    fn closing_ui_keeps_game_input_blocked_until_owned_scene_is_restored() {
        let mut state = ContentLibrary::default();
        state.scene_owned = true;
        state.close();
        state.release_guard = 0;
        assert!(state.blocks_world_input());
        state.scene_owned = false;
        assert!(!state.blocks_world_input());
    }
}
