//! Scoped scene preparation for the two product modes.
//!
//! Current-scene mode borrows only live UID-checked entities. Independent
//! mode changes to a different room, publishes a temporary layout through the
//! ordinary fixture pipeline, adds exact cast members through the ordinary NPC
//! pipeline, and returns through the ordinary site transition. No preview data
//! is written to the editor or settings store.
use super::*;
#[path = "camera_snapshot.rs"]
mod camera_snapshot;
use crate::{
    npc::WalkState, npc_objective::ObjectiveFace, player::PlayerControlled, site::GroundEpoch,
    site::SiteScenesReady, talk::TalkHold,
};
#[cfg(test)]
use moly_law::fixture::position::layout_type;
use moly_law::fixture::{Direction, GridPosition};

const SITE_TIMEOUT: f32 = 90.;
const ASSET_TIMEOUT: f32 = 120.;

#[derive(Clone)]
struct ActorPose {
    entity: Entity,
    before: Transform,
    after: Transform,
    npc: bool,
}

struct ParkedActor {
    entity: Entity,
    visibility: Visibility,
}

#[derive(Resource)]
pub(crate) struct ScenePreview {
    ticket: u64,
    pub(super) epoch: u64,
    actors: Vec<ActorPose>,
    camera: camera_snapshot::CameraSnapshot,
}
impl ScenePreview {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndependentPhase {
    SwitchingSite,
    LoadingContent,
    Staged,
    Experiencing,
    RequestingReturn,
    Returning,
}

#[derive(Resource)]
pub(crate) struct IndependentSession {
    pub(crate) ticket: u64,
    pub(crate) phase: IndependentPhase,
    original_site: String,
    destination: String,
    departure_epoch: u64,
    return_epoch: u64,
    elapsed: f32,
    required_units: Vec<u32>,
    required_fixtures: Vec<i32>,
    rows: Vec<crate::fixture::EditableFixture>,
    temporary_actors: Vec<Entity>,
    parked_actors: Vec<ParkedActor>,
    original_player: Option<(Entity, Transform)>,
    original_actors: Vec<(Entity, Transform)>,
    original_cameras: Vec<(Entity, Transform)>,
    original_camera_state: Option<camera_snapshot::CameraSnapshot>,
    original_appearance: crate::room_appearance::RoomAppearance,
    failure: Option<String>,
}
impl IndependentSession {
    pub(super) fn phase(&self) -> IndependentPhase {
        self.phase
    }
    pub(super) fn required_units(&self) -> &[u32] {
        &self.required_units
    }
    pub(super) fn required_fixtures(&self) -> &[i32] {
        &self.required_fixtures
    }
    pub(super) fn temporary_actors(&self) -> &[Entity] {
        &self.temporary_actors
    }
    pub(super) fn original_site(&self) -> &str {
        &self.original_site
    }
    pub(super) fn destination(&self) -> &str {
        &self.destination
    }
    pub(super) fn original_player_position(&self) -> Option<[f32; 3]> {
        self.original_player
            .map(|(_, pose)| pose.translation.to_array())
    }
    pub(super) fn original_actor_count(&self) -> usize {
        self.original_actors.len()
    }
}

fn owners_idle(world: &World) -> bool {
    !crate::npc_fixture_activity::preview::active(world)
        && !world.contains_resource::<crate::player_talk::PlayerTalkSession>()
        && !world.contains_resource::<crate::talk::ActiveTalk>()
        && !world.resource::<PlayerFixtureRuntime>().active()
        && !crate::fixture_gimmick::session::blocks_replacement(
            world.resource::<crate::fixture_gimmick::Gimmicks>(),
        )
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
                    .get_resource::<crate::talk::TalkStore>()
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
) -> Result<Vec<crate::fixture::EditableFixture>, String> {
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
        rows.push(crate::fixture::EditableFixture {
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
        .get_resource::<crate::fixture_edit::EditSessionActive>()
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
    let anchor = actors
        .first()
        .map(|actor| actor.after.translation)
        .or_else(|| bindings.first().map(|(_, _, _, pose)| pose.translation))
        .or_else(|| {
            if choice.mode == ExperienceMode::Independent {
                face.preview_center().map(Vec3::from)
            } else {
                None
            }
        })
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
            crate::npc::stop_for_external_activity(world, actor.entity);
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

fn independent_origin(world: &World) -> (u64, bool) {
    let current_epoch = world.get_resource::<GroundEpoch>().map(|epoch| epoch.0);
    let settled = current_epoch.is_some() && world.contains_resource::<SiteScenesReady>();
    (current_epoch.unwrap_or(0), settled)
}

fn independent_generation_ready(
    original_site: &str,
    destination: &str,
    departure_epoch: u64,
    current_epoch: Option<u64>,
) -> bool {
    current_epoch.is_some_and(|current| {
        if original_site == destination {
            current >= departure_epoch.max(1)
        } else {
            current > departure_epoch
        }
    })
}

fn start_independent(world: &mut World, choice: &PlaybackChoice) -> Result<(), String> {
    if world
        .get_resource::<crate::fixture_edit::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return Err("请先退出家具布局编辑，再开始独立体验".into());
    }
    let (units, fixtures, _) = requirements(world, choice)?;
    let rows = preview_rows(world.resource::<LibraryCatalog>(), &fixtures, choice.ticket)?;
    // Independent mode may be requested while the initial site is still
    // streaming. Do not spend a minute finishing a scene that will immediately
    // be discarded: epoch 0 means "no settled origin yet" and the destination's
    // first settled generation will advance to 1.
    let (epoch, origin_settled) = independent_origin(world);
    let original_site = world
        .resource::<crate::site::SiteSelection>()
        .site_type()
        .to_owned();
    // Furniture and character actions belong in a clean courtyard, not an
    // unrelated indoor room. A wallpaper/floor swatch requires an actual room
    // surface and is the only semantic exception, never a fabricated mesh.
    let surface = matches!(choice.key, EntryKey::Fixture(id)
        if world.resource::<LibraryCatalog>().fixture(id)
            .is_some_and(|row| matches!(row.presentation, FixturePresentation::Surface { .. })));
    let destination = if surface { "first_floor" } else { "home_site" }.to_owned();
    // A half-loaded origin has no authoritative poses yet. In that case the
    // normal site pipeline must reseed the player, actors and camera on return.
    let original_player = origin_settled
        .then(|| {
            world
                .query_filtered::<(Entity, &Transform), With<PlayerControlled>>()
                .iter(world)
                .next()
                .map(|(entity, pose)| (entity, *pose))
        })
        .flatten();
    let original_actors = if origin_settled {
        world
            .query_filtered::<(Entity, &Transform), (With<CharacterUnitId>, Without<PlayerControlled>)>()
            .iter(world)
            .map(|(entity, pose)| (entity, *pose))
            .collect()
    } else {
        Vec::new()
    };
    let original_camera_state =
        origin_settled.then(|| camera_snapshot::CameraSnapshot::capture(world));
    let original_cameras = if origin_settled {
        world
            .query_filtered::<(Entity, &Transform), With<Camera3d>>()
            .iter(world)
            .map(|(entity, pose)| (entity, *pose))
            .collect()
    } else {
        Vec::new()
    };
    camera_snapshot::CameraSnapshot::prepare_temporary(world);
    if original_site != destination {
        world.insert_resource(crate::site::TemporarySiteChangeRequest(destination.clone()));
    } else {
        info!(
            "[content-library] independent ticket={} reuses current site {} while it settles",
            choice.ticket, destination
        );
    }
    let original_appearance = world
        .resource::<crate::room_appearance::RoomAppearance>()
        .clone();
    let appearance = match choice.key {
        EntryKey::Fixture(id) => world
            .resource::<LibraryCatalog>()
            .fixture(id)
            .map(|row| row.presentation.clone()),
        _ => None,
    };
    if let Some(FixturePresentation::Surface { skin, wall, .. }) = appearance {
        let mut current = world.resource_mut::<crate::room_appearance::RoomAppearance>();
        if wall {
            current.wall = skin;
        } else {
            current.floor = skin;
        }
    }
    world.insert_resource(IndependentSession {
        ticket: choice.ticket,
        phase: IndependentPhase::SwitchingSite,
        original_site,
        destination,
        departure_epoch: epoch,
        return_epoch: 0,
        elapsed: 0.,
        required_units: units,
        required_fixtures: fixtures,
        rows,
        temporary_actors: Vec::new(),
        parked_actors: Vec::new(),
        original_player,
        original_actors,
        original_cameras,
        original_camera_state,
        original_appearance,
        failure: None,
    });
    let mut state = world.resource_mut::<ContentLibrary>();
    state.scene_owned = true;
    state.last_error = None;
    state.status = "正在前往独立空场景…".into();
    state.changed();
    Ok(())
}

fn park_background_actors(world: &mut World, required: &[u32]) -> Vec<ParkedActor> {
    let actors: Vec<_> = world
        .query_filtered::<(Entity, &CharacterUnitId, &Visibility), Without<PlayerControlled>>()
        .iter(world)
        .filter(|(_, unit, _)| !required.contains(&unit.0))
        .map(|(entity, _, visibility)| (entity, *visibility))
        .collect();
    for (entity, _) in &actors {
        crate::npc::stop_for_external_activity(world, *entity);
        world
            .entity_mut(*entity)
            .insert((TalkHold, Visibility::Hidden));
    }
    actors
        .into_iter()
        .map(|(entity, visibility)| ParkedActor { entity, visibility })
        .collect()
}

fn restore_background_actors(world: &mut World, parked: &[ParkedActor]) {
    for actor in parked {
        if let Ok(mut entity) = world.get_entity_mut(actor.entity) {
            entity.remove::<TalkHold>();
            entity.insert(actor.visibility);
        }
    }
}

fn restore_original_poses(world: &mut World, session: &IndependentSession) {
    if let Some(camera) = &session.original_camera_state {
        camera.restore(world);
    }
    if let Some((entity, pose)) = session.original_player {
        if let Ok(mut player) = world.get_entity_mut(entity) {
            player.insert((
                pose,
                GlobalTransform::from(pose),
                crate::player::PlayerInput::default(),
                crate::npc::MotionPhase::Dwelling { remaining: None },
            ));
        }
    }
    for (entity, pose) in &session.original_actors {
        if world.get_entity(*entity).is_err() {
            continue;
        }
        world.entity_mut(*entity).insert(*pose);
        if let Some(mut walk) = world.get_mut::<WalkState>(*entity) {
            walk.0.position = pose.translation.to_array();
            walk.0.forward = (pose.rotation * Vec3::Z).to_array();
        }
    }
    for (entity, pose) in &session.original_cameras {
        if let Ok(mut camera) = world.get_entity_mut(*entity) {
            camera.insert(*pose);
        }
    }
}

fn fail_independent(
    world: &mut World,
    session: &mut IndependentSession,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    session.failure = Some(playback::human_playback_failure(&reason));
    session.phase = IndependentPhase::RequestingReturn;
    session.elapsed = 0.;
    playback::fail(&mut world.resource_mut::<ContentLibrary>(), &reason);
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

fn actors_ready(world: &mut World, required: &[u32]) -> bool {
    let rows: Vec<_> = world
        .query::<(Entity, &CharacterUnitId, &NpcActions)>()
        .iter(world)
        .map(|(entity, unit, actions)| (entity, unit.0, actions.ready()))
        .collect();
    required.iter().all(|unit| {
        rows.iter().any(|(entity, id, ready)| {
            id == unit
                && *ready
                && world
                    .get::<crate::character::MotionDriver>(*entity)
                    .is_some()
        })
    })
}

fn reserve_required_actors(world: &mut World, required: &[u32]) {
    let actors: Vec<_> = world
        .query::<(Entity, &CharacterUnitId)>()
        .iter(world)
        .filter(|(_, unit)| required.contains(&unit.0))
        .map(|(entity, _)| entity)
        .collect();
    for entity in actors {
        crate::npc::stop_for_external_activity(world, entity);
    }
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

fn drive_independent(world: &mut World) {
    let Some(mut session) = world.remove_resource::<IndependentSession>() else {
        return;
    };
    session.elapsed += world.resource::<Time>().delta_secs();
    // Consecutive entries using the same fixture set need no site switch,
    // layout rebuild or character reload. Wait for the old native owners to
    // release, then transfer admission to the new generation in place.
    let replacement = world
        .resource::<ContentLibrary>()
        .pending
        .clone()
        .filter(|choice| {
            choice.mode == ExperienceMode::Independent && choice.ticket != session.ticket
        });
    if let Some(mut choice) = replacement.filter(|_| {
        matches!(
            session.phase,
            IndependentPhase::Staged
                | IndependentPhase::Experiencing
                | IndependentPhase::LoadingContent
        )
    }) {
        if let Ok((units, fixtures, _)) = requirements(world, &choice) {
            let compatible = fixtures.len() == session.required_fixtures.len()
                && fixtures
                    .iter()
                    .all(|id| session.required_fixtures.contains(id))
                && !matches!(choice.key, EntryKey::Fixture(id) if world.resource::<LibraryCatalog>().fixture(id)
                    .is_some_and(|row|matches!(row.presentation, FixturePresentation::Surface { .. })));
            if compatible {
                if !owners_idle(world) {
                    world.insert_resource(session);
                    return;
                }
                restore_preview(world);
                restore_background_actors(world, &session.parked_actors);
                session.parked_actors = park_background_actors(world, &units);
                match crate::npc::spawn_temporary_units(world, &units) {
                    Ok(actors) => session.temporary_actors.extend(actors),
                    Err(reason) => {
                        fail_independent(world, &mut session, reason);
                        world.insert_resource(session);
                        return;
                    }
                }
                choice.target = matching_fixture_targets(world, &fixtures)
                    .and_then(|targets| targets.first().cloned());
                session.ticket = choice.ticket;
                session.required_units = units;
                session.phase = IndependentPhase::LoadingContent;
                session.elapsed = 0.;
                session.failure = None;
                let mut state = world.resource_mut::<ContentLibrary>();
                state.active = None;
                state.pending = Some(choice);
                state.stopping = false;
                state.status = "已复用独立场景，正在切换内容…".into();
                state.changed();
                info!(
                    "[content-library] reused independent world ticket={} fixtures={:?}",
                    session.ticket, fixtures
                );
            }
        }
    }
    let state_ticket = {
        let state = world.resource::<ContentLibrary>();
        state
            .pending
            .as_ref()
            .or(state.active.as_ref().map(|active| &active.choice))
            .map(|choice| choice.ticket)
    };
    if !matches!(
        session.phase,
        IndependentPhase::RequestingReturn | IndependentPhase::Returning
    ) && state_ticket != Some(session.ticket)
    {
        session.phase = IndependentPhase::RequestingReturn;
        session.elapsed = 0.;
    }
    match session.phase {
        IndependentPhase::SwitchingSite => {
            let selection_ready = world
                .get_resource::<crate::site::SiteSelection>()
                .is_some_and(|selection| selection.site_type() == session.destination);
            let epoch = world.get_resource::<GroundEpoch>().map(|epoch| epoch.0);
            let site_ready = independent_generation_ready(
                &session.original_site,
                &session.destination,
                session.departure_epoch,
                epoch,
            ) && world.contains_resource::<SiteScenesReady>()
                && world.contains_resource::<crate::site_material::SiteMaterialsSwapped>()
                && world
                    .get_resource::<crate::fixture::FixturePlacements>()
                    .is_some_and(|layout| layout.site_type() == session.destination);
            if selection_ready && site_ready {
                if let Err(reason) = crate::fixture::install_temporary_layout(world, &session.rows)
                {
                    fail_independent(world, &mut session, reason);
                } else {
                    session.parked_actors = park_background_actors(world, &session.required_units);
                    session.phase = IndependentPhase::LoadingContent;
                    session.elapsed = 0.;
                    let mut state = world.resource_mut::<ContentLibrary>();
                    state.status = "空场景已就绪，正在载入所需角色与家具…".into();
                    state.changed();
                }
            } else if session.elapsed >= SITE_TIMEOUT {
                fail_independent(world, &mut session, "无法在限定时间内准备独立场景");
            }
        }
        IndependentPhase::LoadingContent => {
            if let Some(failure) = world.get_resource::<crate::fixture::FixtureLoadFailure>() {
                let reason = failure.0.clone();
                fail_independent(world, &mut session, reason);
                world.insert_resource(session);
                return;
            }
            let controller_ready = match crate::fixture_gimmick::catalog_stream::ready_for(
                world,
                &session.required_fixtures,
            ) {
                Ok(ready) => ready,
                Err(reason) => {
                    fail_independent(world, &mut session, reason);
                    world.insert_resource(session);
                    return;
                }
            };
            let epoch = world.get_resource::<GroundEpoch>().map(|epoch| epoch.0);
            let room_status = world.resource::<crate::room_appearance::RoomAppearanceState>();
            if world.resource::<crate::site::SiteSelection>().is_room() {
                if let Some(error) = &room_status.error {
                    let reason = error.clone();
                    fail_independent(world, &mut session, reason);
                    world.insert_resource(session);
                    return;
                }
            }
            let room_ready = !world.resource::<crate::site::SiteSelection>().is_room()
                || world
                    .resource::<crate::room_appearance::RoomAppearanceState>()
                    .ready;
            let scene_ready = room_ready
                && world.contains_resource::<crate::site_material::SiteMaterialsSwapped>()
                && epoch.is_some_and(|epoch| {
                    world.contains_resource::<crate::fixture::FixtureScenesReady>()
                        && world
                            .contains_resource::<crate::fixture_material::FixtureMaterialsSwapped>()
                        && world.get_resource::<ObjectiveFace>().is_some_and(|face| {
                            face.is_fresh(epoch)
                                && world
                                    .get_resource::<crate::walk_face::WalkFace>()
                                    .is_some_and(|navigation| {
                                        navigation.layout_revision()
                                            == world
                                                .resource::<crate::fixture::FixtureLayoutRevision>()
                                                .0
                                            && face.navigation_generation()
                                                == navigation.generation()
                                    })
                        })
                });
            if scene_ready {
                if session.temporary_actors.is_empty() {
                    match crate::npc::spawn_temporary_units(world, &session.required_units) {
                        Ok(actors) => session.temporary_actors = actors,
                        Err(reason) => fail_independent(world, &mut session, reason),
                    }
                }
                reserve_required_actors(world, &session.required_units);
                if session.phase == IndependentPhase::LoadingContent
                    && actors_ready(world, &session.required_units)
                    && controller_ready
                {
                    if let Some(targets) =
                        matching_fixture_targets(world, &session.required_fixtures)
                    {
                        let mut choice = world
                            .resource::<ContentLibrary>()
                            .pending
                            .clone()
                            .filter(|choice| choice.ticket == session.ticket);
                        if let Some(choice) = &mut choice {
                            choice.target = targets.first().cloned();
                            world.resource_mut::<ContentLibrary>().pending = Some(choice.clone());
                            match plan_current(world, choice) {
                                Ok(actors) => {
                                    apply_preview(world, choice, actors);
                                    session.phase = IndependentPhase::Staged;
                                    session.elapsed = 0.;
                                }
                                Err(reason) => fail_independent(world, &mut session, reason),
                            }
                        }
                    }
                }
            }
            if session.phase == IndependentPhase::LoadingContent && session.elapsed >= ASSET_TIMEOUT
            {
                fail_independent(
                    world,
                    &mut session,
                    "所需角色或家具没有在限定时间内准备完成",
                );
            }
        }
        IndependentPhase::Staged => {
            let state = world.resource::<ContentLibrary>();
            if state
                .pending
                .as_ref()
                .is_none_or(|choice| choice.ticket != session.ticket)
                && state
                    .active
                    .as_ref()
                    .is_none_or(|active| active.choice.ticket != session.ticket)
            {
                if let Some(reason) = state.last_error.clone() {
                    fail_independent(world, &mut session, reason);
                } else {
                    session.phase = IndependentPhase::RequestingReturn;
                    session.elapsed = 0.;
                }
                world.insert_resource(session);
                return;
            }
            if session.elapsed >= ASSET_TIMEOUT {
                let reason = world
                    .resource::<ContentLibrary>()
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "角色站位未能完成同步，已取消这次体验".into());
                fail_independent(world, &mut session, reason);
            }
            let state = world.resource::<ContentLibrary>();
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.choice.ticket == session.ticket)
            {
                session.phase = IndependentPhase::Experiencing;
                session.elapsed = 0.;
            }
        }
        IndependentPhase::Experiencing => {
            let state = world.resource::<ContentLibrary>();
            if state
                .pending
                .as_ref()
                .is_none_or(|choice| choice.ticket != session.ticket)
                && state
                    .active
                    .as_ref()
                    .is_none_or(|active| active.choice.ticket != session.ticket)
            {
                session.phase = IndependentPhase::RequestingReturn;
                session.elapsed = 0.;
            }
        }
        IndependentPhase::RequestingReturn => {
            if session.failure.is_none() {
                session.failure = world.resource::<ContentLibrary>().last_error.clone();
            }
            if owners_idle(world) {
                if world
                    .resource::<ContentLibrary>()
                    .active
                    .as_ref()
                    .is_some_and(|active| active.choice.ticket == session.ticket)
                {
                    world.resource_mut::<ContentLibrary>().active = None;
                }
                restore_preview(world);
                restore_background_actors(world, &session.parked_actors);
                session.parked_actors.clear();
                crate::npc::remove_temporary_units(world, &session.temporary_actors);
                session.temporary_actors.clear();
                // Withdraw an unconsumed preview before either rollback path.
                // Otherwise cancellation before read_switch could switch later.
                world.remove_resource::<crate::site::TemporarySiteChangeRequest>();
                world.insert_resource(session.original_appearance.clone());
                let never_left = world
                    .get_resource::<crate::site::SiteSelection>()
                    .is_some_and(|selection| selection.site_type() == session.original_site)
                    && world
                        .get_resource::<GroundEpoch>()
                        .is_some_and(|epoch| epoch.0 == session.departure_epoch)
                    && !world.contains_resource::<crate::site::TemporarySiteActive>()
                    && !world.contains_resource::<crate::fixture::TemporaryFixtureLayout>();
                if never_left {
                    restore_original_poses(world, &session);
                    let mut state = world.resource_mut::<ContentLibrary>();
                    state.stopping = false;
                    state.scene_owned = false;
                    state.status = session
                        .failure
                        .as_ref()
                        .map(|reason| format!("{reason}；原场景未被修改"))
                        .unwrap_or_else(|| "已留在原场景，未保存任何临时内容".into());
                    state.changed();
                    return;
                }
                session.return_epoch = world
                    .get_resource::<GroundEpoch>()
                    .map(|epoch| epoch.0)
                    .unwrap_or(0);
                world.remove_resource::<crate::site::TemporarySiteChangeRequest>();
                world.insert_resource(crate::site::SiteChangeRequest(
                    session.original_site.clone(),
                ));
                session.phase = IndependentPhase::Returning;
                session.elapsed = 0.;
                let mut state = world.resource_mut::<ContentLibrary>();
                state.status = "正在返回原场景并恢复位置…".into();
                state.changed();
            }
        }
        IndependentPhase::Returning => {
            let epoch = world.get_resource::<GroundEpoch>().map(|epoch| epoch.0);
            let restored = world
                .get_resource::<crate::site::SiteSelection>()
                .is_some_and(|selection| selection.site_type() == session.original_site)
                && epoch.is_some_and(|epoch| epoch > session.return_epoch)
                && world.contains_resource::<SiteScenesReady>()
                && world
                    .get_resource::<ObjectiveFace>()
                    .is_some_and(|face| face.is_fresh(epoch.unwrap()));
            if restored {
                restore_original_poses(world, &session);
                let mut state = world.resource_mut::<ContentLibrary>();
                state.stopping = false;
                state.scene_owned = false;
                state.status = session
                    .failure
                    .as_ref()
                    .map(|reason| format!("{reason}；已恢复原场景"))
                    .unwrap_or_else(|| "已返回原场景，位置与临时内容均已恢复".into());
                state.changed();
                info!(
                    "[content-library] independent ticket={} restored site={}",
                    session.ticket, session.original_site
                );
                return;
            }
            if session.elapsed >= SITE_TIMEOUT && session.failure.is_none() {
                session.failure = Some("返回原场景仍在等待资源；不会保存临时布局".into());
            }
        }
    }
    world.insert_resource(session);
}

pub(crate) fn prepare_pending(world: &mut World) {
    drive_independent(world);
    let state = world.resource::<ContentLibrary>();
    let Some(choice) = state.pending.clone().filter(|_| state.cleanup_frames == 0) else {
        return;
    };
    if !owners_idle(world) {
        return;
    }
    match choice.mode {
        ExperienceMode::Independent => {
            if !world.contains_resource::<IndependentSession>() {
                if let Err(reason) = start_independent(world, &choice) {
                    playback::fail(&mut world.resource_mut::<ContentLibrary>(), &reason);
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
                world.resource_mut::<ContentLibrary>().active = None;
            }
            match plan_current(world, &choice) {
                Ok(actors) => apply_preview(world, &choice, actors),
                Err(reason) => playback::fail(&mut world.resource_mut::<ContentLibrary>(), &reason),
            }
        }
    }
}

pub(crate) fn retire_scene(world: &mut World) {
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
    fn independent_origin_allows_pre_settle_switch_without_snapshotting_partial_state() {
        let mut world = World::new();
        assert_eq!(independent_origin(&world), (0, false));

        world.insert_resource(SiteScenesReady);
        assert_eq!(independent_origin(&world), (0, false));

        world.insert_resource(GroundEpoch(4));
        assert_eq!(independent_origin(&world), (4, true));

        world.remove_resource::<SiteScenesReady>();
        assert_eq!(independent_origin(&world), (4, false));
    }

    #[test]
    fn same_site_independent_preview_reuses_the_current_generation() {
        assert!(independent_generation_ready(
            "home_site",
            "home_site",
            4,
            Some(4)
        ));
        assert!(independent_generation_ready(
            "home_site",
            "home_site",
            0,
            Some(1)
        ));
        assert!(!independent_generation_ready(
            "home_site",
            "home_site",
            0,
            None
        ));
    }

    #[test]
    fn cross_site_independent_preview_requires_a_new_generation() {
        assert!(!independent_generation_ready(
            "grassland",
            "home_site",
            4,
            Some(4)
        ));
        assert!(independent_generation_ready(
            "grassland",
            "home_site",
            4,
            Some(5)
        ));
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
        world.insert_resource(SiteScenesReady);
        world.insert_resource(GroundEpoch(2));
        assert!(!world.contains_resource::<crate::site::SiteReady>());
        assert!(!world.contains_resource::<crate::inactive_nodes::SiteSettled>());
        assert!(world.contains_resource::<SiteScenesReady>());
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
