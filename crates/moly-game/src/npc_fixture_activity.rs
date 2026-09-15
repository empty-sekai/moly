//! Autonomous single-NPC, no-dialogue furniture activities.
//!
//! The selected action row, actual placed Entity/UID, locator array index and
//! source timeline survive approach, loading, playback and disposal together.
//! This is not a NavMesh implementation: admission/approach still use the
//! existing, explicitly approximate WalkField/ObjectiveFace host. In particular
//! its .3m locator gate is NOT source IsPlayableActionPoints' complete .125m
//! post-sample geometry test. A real shared NavMesh producer remains separate.

mod areas;
pub(crate) mod preview;
pub(crate) use areas::NpcFixtureAreas;

use std::collections::HashMap;

use bevy::{ecs::system::SystemParam, prelude::*};
use moly_assets::scene_state::SourceInactive;
use moly_law::{
    fixture::position::layout_type,
    objective::TalkType,
    path::{
        angle_between, heading_yaw, make_positive_yaw, rotate_time, turn_angle, turn_motion,
        NpcPathWalkSlot, WaypointDraw,
    },
};

use crate::{
    character::MotionDriver,
    fixture::{FixturePlacements, FixtureRoot},
    fixture_activity_data::{
        ActivityKey, ActivityOrigin, ActivitySpec, ActivityTimeline, FixtureActivityTables, PutType,
    },
    fixture_activity_provider::{FixtureActivityProvider, SourceFixtureViewLocalY},
    fixture_activity_state::{
        FixtureActivityIdentity, FixtureActivityOwner, FixtureActivityReservations, FixtureTarget,
    },
    fixture_activity_timeline::{
        self, FixtureActivityTimelines, StartTimeline, TimelineBindings, TimelineOwner,
        TimelineOwnerKind, TimelineStatus, TimelineTimeoutBudget, TimelineToken,
    },
    fixture_attach::AttachPoints,
    fixture_scene_inputs::{FixtureScenePlacement, FixtureSceneSupply},
    fixture_tiles::OccupancyCoverage,
    npc::{
        CharacterUnitId, MotionPhase, NpcAction, NpcActions, PathSlot, PauseSeconds, RestLifecycle,
        RouteOutcome, RouteStops, WalkSpeed, WalkState,
    },
    npc_objective::{AiTalkData, MemberRng, ObjectiveFace, ObjectiveMind, TalkSlot},
    site::GroundEpoch,
    talk::TalkHold,
};

/// Movement ownership starts at arrival, not while an ordinary route is live.
/// It also protects loading/exit from npc::advance's provisional reentry warp.
#[derive(Component, Clone, Copy)]
pub(crate) struct NpcFixtureMotionOwner(pub FixtureActivityOwner);

/// Separate from Rest's alone_holds: the Rest script may dispose without
/// clearing the actual furniture timeline's animation lease.
#[derive(Component, Clone, Copy)]
pub(crate) struct NpcFixtureAnimationOwner(pub FixtureActivityOwner);

#[derive(Clone)]
pub(crate) struct Selection {
    actor: Entity,
    unit: u32,
    pub target: FixtureTarget,
    identity: FixtureActivityIdentity,
    site_epoch: u64,
    source: ActivityKey,
    tweet: Option<moly_law::talk::TweetRef>,
    timeline: ActivityTimeline,
    point: i32,
    locate_index: usize,
    slot_id: i32,
    put_type: PutType,
    pub position: [f32; 3],
    pub rotation: Quat,
}

impl Selection {
    pub(crate) fn ai_data(&self) -> AiTalkData {
        AiTalkData {
            kind: TalkType::NoneTalk,
            content: None, // A no-dialogue action has no fabricated talk master.
            target_fixture: Some(self.target.entity),
            target_position: self.position,
            main_character: self.unit,
            characters: vec![self.unit],
            pre_action: None,
            pending_factory: None,
        }
    }
}

enum Phase {
    Approaching,
    Preparing,
    Playing,
    Exiting(LocalMove),
}

struct Session {
    owner: FixtureActivityOwner,
    selection: Selection,
    phase: Phase,
    request: Option<StartTimeline>,
    token: Option<TimelineToken>,
    animator: Option<Entity>,
    previous_enable_talk: bool,
    last_pending: Option<String>,
    preview_ticket: Option<u64>,
    waiting_seconds: f32,
    tweet_issued: bool,
}

#[derive(Resource, Default)]
pub(crate) struct NpcFixtureActivities {
    next_generation: u64,
    sessions: HashMap<Entity, Session>,
    pending_factories: HashMap<Entity, String>,
}

#[derive(SystemParam)]
pub(crate) struct Factory<'w, 's> {
    editor: Res<'w, crate::fixture_edit::EditSessionActive>,
    tables: Option<Res<'w, FixtureActivityTables>>,
    scripts: Option<Res<'w, crate::talk::TalkStore>>,
    points: Option<Res<'w, AttachPoints>>,
    inputs: Option<Res<'w, FixtureSceneSupply>>,
    areas: Res<'w, NpcFixtureAreas>,
    reservations: ResMut<'w, FixtureActivityReservations>,
    runtime: ResMut<'w, NpcFixtureActivities>,
    fixtures: Query<
        'w,
        's,
        (
            Entity,
            &'static FixtureActivityIdentity,
            &'static FixtureScenePlacement,
            &'static GlobalTransform,
        ),
        (With<FixtureRoot>, Without<SourceInactive>),
    >,
}

impl Factory<'_, '_> {
    pub(crate) fn owns_actor(&self, actor: Entity) -> bool {
        self.runtime.sessions.contains_key(&actor)
    }

    pub(crate) fn report_pending(&mut self, actor: Entity, unit: u32, reason: String) {
        if self.runtime.pending_factories.get(&actor) != Some(&reason) {
            info!("[npc-fixture] unit={unit} factory pending: {reason}");
            self.runtime.pending_factories.insert(actor, reason);
        }
    }

    /// Source action-row permutation is independent of asset readiness.
    /// Empty eligible source pool is Ok(None); missing data is Err, not an
    /// invitation to select a different ready animation or a General story.
    pub(crate) fn select_none_talk(
        &self,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        face: &ObjectiveFace,
        rng: &mut MemberRng,
    ) -> Result<Option<Selection>, String> {
        self.select_internal(
            actor,
            unit,
            site_type,
            epoch,
            from,
            placements,
            other_targets,
            face,
            rng,
            None,
        )
    }

    pub(crate) fn select_exact(
        &self,
        key: ActivityKey,
        target: &FixtureTarget,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        face: &ObjectiveFace,
        rng: &mut MemberRng,
    ) -> Result<Option<Selection>, String> {
        let tables = self.tables.as_deref().ok_or("角色互动主表尚未载入")?;
        let scripts = self.scripts.as_deref().ok_or("家具前置演出主表尚未载入")?;
        let spec = tables
            .resolve_activity(key, scripts)
            .ok_or("所选互动的原始演员、家具或动作关系不可用")?;
        if spec.unit != unit {
            return Err("所选互动的演员身份已经变化".into());
        }
        self.select_internal(
            actor,
            unit,
            site_type,
            epoch,
            from,
            placements,
            other_targets,
            face,
            rng,
            Some((&spec, target)),
        )
    }

    fn select_internal(
        &self,
        actor: Entity,
        unit: u32,
        site_type: &str,
        epoch: u64,
        from: [f32; 3],
        placements: &FixturePlacements,
        other_targets: &[(Entity, Option<Entity>)],
        face: &ObjectiveFace,
        rng: &mut MemberRng,
        exact: Option<(&ActivitySpec, &FixtureTarget)>,
    ) -> Result<Option<Selection>, String> {
        if self.editor.is_active() {
            return Err(
                "the layout editor owns fixture mutation; no new activity is admitted".into(),
            );
        }
        let tables = self
            .tables
            .as_deref()
            .ok_or("no-talk source tables are still loading")?;
        let points = self
            .points
            .as_deref()
            .ok_or("source fixture locator arrays are still loading")?;
        let inputs = self
            .inputs
            .as_deref()
            .and_then(FixtureSceneSupply::current)
            .ok_or("current fixture scene inputs are not ready")?;
        if inputs.stamp.site_epoch != epoch
            || inputs.site_type != site_type
            || inputs.floor.coverage() != OccupancyCoverage::Complete
            || !inputs.gaps.is_empty()
        {
            return Err(
                "no-talk factory needs the current complete placement/floor snapshot".into(),
            );
        }
        let rows = placements.occupancy_rows();
        // GetAllFixture is insertion ordered. The offline placement owner is
        // that order in this host; query iteration and package-first lookup are
        // not. Resolve every selected row against exactly one live Entity/UID.
        let mut instances = Vec::with_capacity(rows.len());
        for row in &rows {
            let matches: Vec<_> = self
                .fixtures
                .iter()
                .filter(|(_, identity, placed, _)| {
                    identity.uid == row.uid && placed.0.uid == row.uid
                })
                .collect();
            let [(entity, identity, placed, world)] = matches.as_slice() else {
                return Err(format!("placement {} has no unique live instance", row.uid));
            };
            if &placed.0 != row || identity.model_package != row.package {
                return Err(format!(
                    "placement {} no longer agrees with its live owner",
                    row.uid
                ));
            }
            instances.push((*entity, *identity, *world, row));
        }
        // A projection, not a fabricated row in the NoTalk source table.
        // Exact pre-actions retain their own origin and authored tweet.
        struct Candidate {
            origin: ActivityOrigin,
            unit: u32,
            fixture_id: i32,
            group_id: i32,
            point: Option<i32>,
            timeline_id: Option<i32>,
            tweet: Option<moly_law::talk::TweetRef>,
        }
        let mut order: Vec<(u64, Candidate)> = if let Some((spec, _)) = exact {
            vec![(
                0,
                Candidate {
                    origin: spec.key.origin,
                    unit: spec.unit,
                    fixture_id: spec.fixture_id,
                    group_id: spec.timeline.group_id,
                    point: Some(spec.point),
                    timeline_id: Some(spec.timeline.id),
                    tweet: spec.tweet.clone(),
                },
            )]
        } else {
            // Preserve the natural source permutation and draw count.
            tables
                .no_talk_rows()
                .iter()
                .map(|row| {
                    (
                        rng.next(),
                        Candidate {
                            origin: ActivityOrigin::NoTalk(row.id),
                            unit: row.unit,
                            fixture_id: row.fixture_id,
                            group_id: row.timeline_group_id,
                            point: None,
                            timeline_id: None,
                            tweet: None,
                        },
                    )
                })
                .collect()
        };
        order.sort_by_key(|(key, _)| *key);
        for (_, action) in order {
            if action.unit != unit {
                continue;
            }
            // The source never resolves this action's group if no placed
            // fixture of its master can be considered at all.
            if !instances
                .iter()
                .any(|(_, identity, _, _)| identity.master_id == action.fixture_id)
            {
                continue;
            }
            let group: Vec<_> = tables.timeline_rows(action.group_id).collect();
            let Some(first) = group.first() else {
                return Err(format!(
                    "source action {} has no timeline group {}",
                    match action.origin {
                        ActivityOrigin::NoTalk(id) | ActivityOrigin::PreAction(id) => id,
                    },
                    action.group_id
                ));
            };
            // Source factory computes locators from FirstOrDefault(group),
            // then independently RandomPick(group) for the actual director.
            let point = match action.point {
                Some(point) => point,
                None => tables
                    .no_talk_point(unit, first)
                    .map_err(|error| format!("action {:?} locator: {error:?}", action.origin))?,
            };
            for (entity, identity, fixture_world, row) in &instances {
                if identity.master_id != action.fixture_id {
                    continue;
                }
                let target = FixtureTarget {
                    entity: *entity,
                    uid: identity.uid.clone(),
                };
                if exact.is_some_and(|(_, selected)| selected != &target) {
                    continue;
                }
                if self.reservations.npc_target_in_use(&target)
                    || other_targets
                        .iter()
                        .any(|(other, target)| *other != actor && *target == Some(*entity))
                {
                    continue;
                }
                // This slice covers placed ground-floor fixtures. Elevated
                // stack support is an explicit pending branch, not guessed
                // from render height or silently admitted as ground.
                if row.layout != layout_type::FLOOR || row.center_y != 0 {
                    return Err(format!(
                        "{} needs the elevated/non-floor source fixture branch",
                        identity.uid
                    ));
                }
                if fixture_world.translation().distance(Vec3::from(from)) > 1000.0 {
                    continue;
                }
                if self.areas.motion_overlaps(row, &rows, &inputs.floor)? {
                    continue;
                }
                let master = tables
                    .fixture_master(identity.master_id)
                    .ok_or_else(|| format!("missing fixture master {}", identity.master_id))?;
                if identity.model_package != format!("mysekai__fixture__{}", master.model_name) {
                    return Err("source action's instance model/master disagree".into());
                }
                let locate_index = points
                    .instance_index(&identity.model_package, point)
                    .ok_or_else(|| {
                        format!(
                            "{} locator {point} lacks a unique source array index",
                            identity.uid
                        )
                    })?;
                let slot_id = points
                    .instance_slot(&identity.model_package, point)
                    .ok_or_else(|| {
                        format!("{} locator {point} lacks a source slot", identity.uid)
                    })?;
                if self.reservations.action_slot_in_use(&target, slot_id) {
                    continue;
                }
                let poses = points
                    .instance_poses(&identity.model_package, point, fixture_world)
                    .ok_or("source selected locator has no instance pose")?;
                let end = poses.end.ok_or("source selected locator has no EndLoc")?;
                let position = [
                    poses.start.position[0],
                    inputs.floor.site_origin.y,
                    poses.start.position[2],
                ];
                // Retain the existing approximate host's navigation gate.
                // Do not call it the native CalculatePath/half-tile contract.
                let Some(hit) = face.sample(position, 0.3) else {
                    continue;
                };
                if !face.has_path(from, hit) {
                    continue;
                }
                let start_grid = inputs
                    .floor
                    .world_to_grid(Vec3::from(poses.start.position))
                    .ok_or("source StartLoc is outside the floor coordinate domain")?;
                if !inputs.floor.contains_grid(start_grid)
                    || inputs
                        .floor
                        .world_to_grid(Vec3::from(end.position))
                        .is_none_or(|grid| !inputs.floor.contains_grid(grid))
                {
                    continue;
                }
                let inside = row.min.x <= start_grid.x
                    && start_grid.x <= row.max.x
                    && row.min.z <= start_grid.z
                    && start_grid.z <= row.max.z;
                if !inside
                    && !matches!(
                        inputs.floor.tile_at_grid(start_grid),
                        crate::player_fixture_action::FixtureTileKnowledge::Empty
                    )
                {
                    continue;
                }
                let timeline = if let Some(id) = action.timeline_id {
                    (**group
                        .iter()
                        .find(|timeline| timeline.id == id)
                        .ok_or("所选动作已不在原始 Timeline 组中")?)
                    .clone()
                } else {
                    (*group[rng.index(group.len())]).clone()
                };
                return Ok(Some(Selection {
                    actor,
                    unit,
                    target,
                    identity: (*identity).clone(),
                    site_epoch: epoch,
                    source: ActivityKey {
                        origin: action.origin,
                        timeline_id: timeline.id,
                    },
                    tweet: action.tweet.clone(),
                    timeline,
                    point,
                    locate_index,
                    slot_id,
                    put_type: master.put_type,
                    position,
                    rotation: poses.start.rotation,
                }));
            }
        }
        Ok(None)
    }

    pub(crate) fn begin(&mut self, selection: Selection, enable_talk: bool) -> bool {
        if self.owns_actor(selection.actor) {
            return false;
        }
        self.runtime.next_generation = self
            .runtime
            .next_generation
            .checked_add(1)
            .expect("NPC fixture activity generation exhausted");
        let owner = FixtureActivityOwner {
            actor: selection.actor,
            generation: self.runtime.next_generation,
        };
        if !self
            .reservations
            .reserve_npc_target(&selection.target, owner)
        {
            return false;
        }
        self.runtime.pending_factories.remove(&selection.actor);
        info!(
            "[npc-fixture] unit={} selected action={} fixture={:?}/{} locator={}/slot={} timeline={} (WalkField approach remains approximate)",
            selection.unit,
            selection.source.source_id(),
            selection.target.entity,
            selection.target.uid,
            selection.locate_index,
            selection.slot_id,
            selection.timeline.id
        );
        self.runtime.sessions.insert(
            selection.actor,
            Session {
                owner,
                selection,
                phase: Phase::Approaching,
                request: None,
                token: None,
                animator: None,
                previous_enable_talk: enable_talk,
                last_pending: None,
                preview_ticket: None,
                waiting_seconds: 0.,
                tweet_issued: false,
            },
        );
        true
    }
}

/// Runs after the ordinary movement system and before the shared timeline.
/// An Arrived result transfers ownership; it is not objective completion.
pub(crate) fn advance(world: &mut World) {
    if world
        .get_resource::<crate::fixture_edit::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return;
    }
    let Some(mut runtime) = world.remove_resource::<NpcFixtureActivities>() else {
        return;
    };
    runtime
        .pending_factories
        .retain(|actor, _| world.entities().contains(*actor));
    let mut actors: Vec<_> = runtime
        .sessions
        .iter()
        .map(|(actor, session)| (*actor, session.owner.generation))
        .collect();
    actors.sort_by_key(|(_, generation)| *generation);
    let mut provider = world.remove_resource::<FixtureActivityProvider>();
    for (actor, _) in actors {
        let Some(mut session) = runtime.sessions.remove(&actor) else {
            continue;
        };
        let result = tick(world, provider.as_mut(), &mut session);
        match result {
            Ok(true) => dispose(world, session, false, true),
            Ok(false) => {
                preview::record_progress(world, &mut session);
                runtime.sessions.insert(actor, session);
            }
            Err(reason) => {
                warn!(
                    "[npc-fixture] unit={} action={} cancelled: {reason}",
                    session.selection.unit,
                    session.selection.source.source_id()
                );
                preview::record_failure(world, &session, reason);
                dispose(world, session, false, false);
            }
        }
    }
    if let Some(provider) = provider {
        world.insert_resource(provider);
    }
    world.insert_resource(runtime);
}

fn tick(
    world: &mut World,
    provider: Option<&mut FixtureActivityProvider>,
    session: &mut Session,
) -> Result<bool, String> {
    let actor = session.owner.actor;
    if matches!(session.phase, Phase::Approaching | Phase::Preparing) {
        session.waiting_seconds += world.resource::<Time>().delta_secs();
        if session.waiting_seconds > 120. {
            return Err(format!(
                "角色互动准备未完成：{}",
                session
                    .last_pending
                    .as_deref()
                    .unwrap_or("角色未能走到家具动作点")
            ));
        }
    }
    let selection = &session.selection;
    if world
        .get_resource::<GroundEpoch>()
        .is_none_or(|epoch| epoch.0 != selection.site_epoch)
    {
        return Err("site generation changed".into());
    }
    if world
        .get::<CharacterUnitId>(actor)
        .is_none_or(|unit| unit.0 != selection.unit)
        || world.get::<FixtureActivityIdentity>(selection.target.entity)
            != Some(&selection.identity)
        || world
            .get::<SourceInactive>(selection.target.entity)
            .is_some()
    {
        return Err("selected actor or fixture Entity/UID no longer exists".into());
    }
    if world
        .get::<TalkSlot>(actor)
        .and_then(|slot| slot.current.as_ref())
        .is_none_or(|data| {
            data.kind != TalkType::NoneTalk || data.target_fixture != Some(selection.target.entity)
        })
    {
        return Err("another objective replaced the selected action".into());
    }
    if world
        .get_resource::<FixtureActivityReservations>()
        .and_then(|reservations| reservations.npc_target_owner(&selection.target))
        != Some(session.owner)
    {
        return Err("future fixture reservation was replaced".into());
    }
    if world.get::<TalkHold>(actor).is_some() {
        // The present talk dispatcher has no registered-playing-fixture join.
        // An independently admitted conversation must not have its body stolen.
        return Err("another conversation owns the actor".into());
    }
    let dt = world.get_resource::<Time>().map_or(0.0, Time::delta_secs);
    if !dt.is_finite() || dt < 0.0 {
        return Err("invalid game delta".into());
    }

    if matches!(session.phase, Phase::Approaching) {
        match world
            .get::<RouteStops>(actor)
            .and_then(|route| route.outcome)
        {
            None => return Ok(false),
            Some(RouteOutcome::Stopped) => {
                return Err("ordinary approach stopped without arrival".into());
            }
            Some(RouteOutcome::Arrived) => {
                world
                    .entity_mut(actor)
                    .insert(NpcFixtureMotionOwner(session.owner));
                world
                    .get_mut::<RouteStops>(actor)
                    .ok_or("route owner was removed")?
                    .hold_for_fixture();
                world
                    .get_mut::<PathSlot>(actor)
                    .ok_or("path slot was removed")?
                    .0 = NpcPathWalkSlot::from_corners(Vec::new());
                world
                    .entity_mut(actor)
                    .insert(MotionPhase::Dwelling { remaining: None });
                change_action(world, actor, NpcAction::FixtureAction, false)?;
                session.phase = Phase::Preparing;
                info!(
                    "[npc-fixture] unit={} arrived at action {}; preparing its exact source timeline",
                    selection.unit, selection.source.source_id()
                );
            }
        }
    }
    if world
        .get::<NpcFixtureMotionOwner>(actor)
        .is_none_or(|held| held.0 != session.owner)
    {
        return Err("fixture movement lease was replaced".into());
    }
    if matches!(session.phase, Phase::Preparing) {
        let Some(provider) = provider else {
            return pending(session, "shared activity provider is not initialized");
        };
        if let Err(issue) = prepare(world, provider, session) {
            let reason = format!("{}: {}", issue.stage, issue.reason);
            if issue.retryable {
                return pending(session, &reason);
            }
            return Err(reason);
        }
        let request = session
            .request
            .take()
            .ok_or("prepared timeline request is missing")?;
        let target = session.selection.target.clone();
        let slot = session.selection.slot_id;
        let reserved = world
            .resource_mut::<FixtureActivityReservations>()
            .reserve_action_slot(&target, slot, session.owner);
        if !reserved {
            return Err("selected action slot became occupied before playback".into());
        }
        let poses = live_poses(world, &session.selection)?;
        set_pose(world, actor, poses.0)?;
        world
            .entity_mut(actor)
            .insert(NpcFixtureAnimationOwner(session.owner));
        if let Some(mut driver) = world.get_mut::<MotionDriver>(actor) {
            driver.playing = None;
        }
        let token = world
            .resource_mut::<FixtureActivityTimelines>()
            .request_start(request);
        session.token = Some(token);
        session.phase = Phase::Playing;
        session.last_pending = None;
        info!(
            "[npc-fixture] unit={} start action={} timeline={} target={:?}/{}",
            session.selection.unit,
            session.selection.source.source_id(),
            session.selection.timeline.id,
            target.entity,
            target.uid
        );
        return Ok(false);
    }
    if matches!(session.phase, Phase::Playing) {
        let token = session
            .token
            .ok_or("playing activity has no timeline token")?;
        if world
            .get::<MotionDriver>(actor)
            .is_none_or(|driver| Some(driver.player) != session.animator)
            || world
                .get::<NpcFixtureAnimationOwner>(actor)
                .is_none_or(|held| held.0 != session.owner)
        {
            return Err("selected NPC animator generation or lease changed".into());
        }
        let in_talk = world.contains_resource::<crate::player_talk::PlayerTalkSession>()
            || world
                .get_resource::<crate::talk::ActiveTalk>()
                .is_some_and(|talk| talk.includes_player());
        let status = {
            let mut timelines = world.resource_mut::<FixtureActivityTimelines>();
            timelines.set_timeout_budget(token, Some(!in_talk));
            timelines
                .status(token)
                .cloned()
                .ok_or("shared timeline removed the selected token")?
        };
        match status {
            TimelineStatus::Preparing => return Ok(false),
            TimelineStatus::Playing { enable_talk, .. } => {
                // Parenting semantics without recursive-despawn ownership:
                // recompose against this same instance's live locator.
                let poses = live_poses(world, &session.selection)?;
                set_pose(world, actor, poses.0)?;
                if let Some(mut actions) = world.get_mut::<NpcActions>(actor) {
                    actions.enable_talk = enable_talk;
                }
                return Ok(false);
            }
            TimelineStatus::Completed => {
                world
                    .resource_mut::<FixtureActivityTimelines>()
                    .release(token);
                session.token = None;
                release_animation_lease(world, session);
                change_action(world, actor, NpcAction::FixtureActionIdle, false)?;
                let (_, end) = live_poses(world, &session.selection)?;
                set_pose(world, actor, end)?;
                let speed = world
                    .get::<WalkSpeed>(actor)
                    .ok_or("source NPC walk speed is missing")?
                    .0;
                let sample = world
                    .get_resource::<ObjectiveFace>()
                    .and_then(|face| face.sample(end.translation.to_array(), 2.0));
                // Source fallback uses EndLoc itself, not a previous approach
                // checkpoint. The eventual real agent handles reattachment.
                let target = sample.map(Vec3::from).unwrap_or(end.translation);
                let distance = end.translation.distance(target);
                let speed = if distance >= 0.1 { speed } else { speed / 5.0 };
                let direction = (target - end.translation).normalize_or_zero();
                let rotation = if sample.is_none() || direction.dot(end.rotation * Vec3::Z) > 0.9 {
                    end.rotation
                } else if direction == Vec3::ZERO {
                    Quat::IDENTITY // Unity LookRotation(zero) yields identity.
                } else {
                    Quat::from_rotation_y(direction.x.atan2(direction.z))
                };
                session.phase = Phase::Exiting(LocalMove::new(end, target, rotation, speed)?);
                info!(
                    "[npc-fixture] unit={} source timeline ended; EndLoc exit begins",
                    session.selection.unit
                );
            }
            TimelineStatus::Cancelled => return Err("shared timeline was cancelled".into()),
            TimelineStatus::Failed(error) => {
                return Err(format!("source timeline failed: {error}"));
            }
        }
    }
    if let Phase::Exiting(movement) = &mut session.phase {
        let (pose, phase, done) = movement.step(dt);
        set_pose(world, actor, pose)?;
        world.entity_mut(actor).insert(phase);
        return Ok(done);
    }
    Ok(false)
}

fn pending(session: &mut Session, reason: &str) -> Result<bool, String> {
    if session.last_pending.as_deref() != Some(reason) {
        info!(
            "[npc-fixture] unit={} action={} preparation pending: {reason}",
            session.selection.unit,
            session.selection.source.source_id()
        );
        session.last_pending = Some(reason.into());
    }
    Ok(false)
}

fn prepare(
    world: &mut World,
    provider: &mut FixtureActivityProvider,
    session: &mut Session,
) -> Result<(), crate::fixture_activity_provider::ProviderPending> {
    let selection = &session.selection;
    let view_y = world
        .get::<SourceFixtureViewLocalY>(selection.target.entity)
        .ok_or("source FixtureView local Y is not installed")?
        .0;
    let (package, prefab) = crate::fixture_activity_data::timeline_asset(
        &selection.timeline.asset_name,
        selection.put_type,
        view_y,
    )
    .map_err(|error| format!("source timeline route: {error:?}"))?;
    let (animator, graph) = world
        .get::<MotionDriver>(session.owner.actor)
        .map(|driver| (driver.player, driver.graph.clone()))
        .ok_or("selected NPC body is not installed")?;
    if session.animator.is_some_and(|old| old != animator) {
        return Err("selected NPC body generation changed".into());
    }
    if session.request.is_none() {
        let definition = provider.definition(world, &package, &prefab)?;
        session.request = Some(StartTimeline {
            owner: TimelineOwner {
                activity: session.owner,
                kind: TimelineOwnerKind::Npc,
            },
            fixture: selection.target.entity,
            definition,
            bindings: TimelineBindings::default(),
            companions: Vec::new(),
            timeout_secs: 30.0,
            timeout_budget: TimelineTimeoutBudget::OwnerGated {
                advance: Some(true),
            },
        });
    }
    let request = session.request.as_mut().expect("installed request");
    provider.prepare_bindings(world, request, selection.unit, animator, graph)?;
    if !request
        .bindings
        .animations
        .values()
        .any(|binding| binding.animator == animator)
    {
        return Err("source-selected NPC timeline has no actual body binding".into());
    }
    fixture_activity_timeline::validate_start(world, request).map_err(|error| {
        crate::fixture_activity_provider::ProviderPending {
            stage: "live-binding-preflight",
            reason: error.to_string(),
            retryable: error.retryable,
        }
    })?;
    session.animator = Some(animator);
    Ok(())
}

fn live_poses(world: &World, selection: &Selection) -> Result<(Transform, Transform), String> {
    let fixture = world
        .get::<GlobalTransform>(selection.target.entity)
        .ok_or("fixture transform disappeared")?;
    let points = world
        .get_resource::<AttachPoints>()
        .ok_or("source locators disappeared")?;
    if points.instance_index(&selection.identity.model_package, selection.point)
        != Some(selection.locate_index)
    {
        return Err("source locator array identity changed".into());
    }
    let poses = points
        .instance_poses(&selection.identity.model_package, selection.point, fixture)
        .ok_or("source locator pose disappeared")?;
    let end = poses.end.ok_or("source EndLoc disappeared")?;
    let pose = |pose: crate::fixture_attach::AttachPose| {
        Transform::from_translation(Vec3::from(pose.position)).with_rotation(pose.rotation)
    };
    Ok((pose(poses.start), pose(end)))
}

fn set_pose(world: &mut World, actor: Entity, pose: Transform) -> Result<(), String> {
    if !pose.translation.is_finite() || !pose.rotation.is_finite() {
        return Err("nonfinite source actor pose".into());
    }
    // NPC logical roots currently have no transform parent. Do not replace a
    // parent adopted by another owner with a synthetic fixture-relative pose.
    if world.get::<ChildOf>(actor).is_some() {
        return Err("logical NPC has an unowned transform parent".into());
    }
    let mut walk = world
        .get_mut::<WalkState>(actor)
        .ok_or("NPC walk state disappeared")?;
    walk.0.position = pose.translation.to_array();
    walk.0.forward = (pose.rotation * Vec3::Z).to_array();
    walk.0.next_corner = 0;
    drop(walk);
    let scale = world
        .get::<Transform>(actor)
        .ok_or("NPC transform disappeared")?
        .scale;
    let pose = Transform { scale, ..pose };
    world
        .entity_mut(actor)
        .insert((pose, GlobalTransform::from(pose)));
    Ok(())
}

fn change_action(
    world: &mut World,
    actor: Entity,
    action: NpcAction,
    enable_talk: bool,
) -> Result<(), String> {
    let mut query = world.query::<(&mut NpcActions, &mut RestLifecycle)>();
    let (mut actions, mut rest) = query
        .get_mut(world, actor)
        .map_err(|_| "NPC action owner disappeared")?;
    actions.change(action, &mut rest);
    actions.enable_talk = enable_talk;
    Ok(())
}

fn release_animation_lease(world: &mut World, session: &Session) {
    let actor = session.owner.actor;
    if world
        .get::<NpcFixtureAnimationOwner>(actor)
        .is_some_and(|held| held.0 == session.owner)
    {
        world.entity_mut(actor).remove::<NpcFixtureAnimationOwner>();
        if let Some(mut driver) = world.get_mut::<MotionDriver>(actor) {
            if Some(driver.player) == session.animator {
                driver.playing = None;
            }
        }
    }
}

fn dispose(world: &mut World, session: Session, site_changed: bool, completed: bool) {
    preview::record_disposed(world, &session, completed);
    crate::balloon::cancel_activity_balloon(world, session.owner);
    let actor = session.owner.actor;
    if let Some(token) = session.token {
        if let Some(mut timelines) = world.get_resource_mut::<FixtureActivityTimelines>() {
            timelines.cancel(token);
            timelines.release(token);
        }
    }
    release_animation_lease(world, &session);
    let holds_motion = world
        .get::<NpcFixtureMotionOwner>(actor)
        .is_some_and(|held| held.0 == session.owner);
    if holds_motion {
        world.entity_mut(actor).remove::<NpcFixtureMotionOwner>();
    }
    if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() {
        reservations.release_owner(session.owner);
    }
    if site_changed || !world.entities().contains(actor) {
        return;
    }
    let mut query = world.query::<(
        &CharacterUnitId,
        &PauseSeconds,
        &mut ObjectiveMind,
        &mut TalkSlot,
        &mut NpcActions,
        &mut RestLifecycle,
        &mut RouteStops,
        &mut PathSlot,
        &mut MotionPhase,
    )>();
    if let Ok((
        unit,
        pause,
        mut mind,
        mut slot,
        mut actions,
        mut rest,
        mut route,
        mut path,
        mut phase,
    )) = query.get_mut(world, actor)
    {
        let still_ours = slot.current.as_ref().is_some_and(|data| {
            data.kind == TalkType::NoneTalk
                && data.target_fixture == Some(session.selection.target.entity)
        });
        if !still_ours {
            return;
        }
        // Never overwrite a separately admitted conversation's state/animation.
        if actions.current == NpcAction::Talk {
            if let Some(data) = slot.current.as_mut() {
                data.target_fixture = None;
            }
            mind.executing = false;
            return;
        }
        route.finish_fixture();
        path.0 = NpcPathWalkSlot::from_corners(Vec::new());
        *phase = MotionPhase::Dwelling { remaining: None };
        actions.enable_talk = session.previous_enable_talk;
        // Reuse the existing NoneTalk post-action refuel/Rest owner; it still
        // has its explicitly documented fixture-talk factory limitations.
        crate::npc_objective::finish_objective(
            unit.0,
            &mut mind,
            &mut slot,
            pause.0,
            !completed,
            &mut actions,
            &mut rest,
        );
        info!(
            "[npc-fixture] unit={} action={} disposed completed={completed}; instance/slot leases released",
            unit.0, session.selection.source.source_id()
        );
    }
}

pub(crate) fn cancel_for_site_change(world: &mut World) {
    cancel_runtime(world, true);
}

/// Unlike site destruction, the NPC remains alive after editor entry. Its
/// existing normal disposal branch clears only the matching no-talk target,
/// path and objective while respecting a separately acquired talk owner.
pub(crate) fn cancel_for_layout_edit(world: &mut World) {
    cancel_runtime(world, false);
}

fn cancel_runtime(world: &mut World, site_changed: bool) {
    let Some(mut runtime) = world.remove_resource::<NpcFixtureActivities>() else {
        return;
    };
    for (actor, session) in std::mem::take(&mut runtime.sessions) {
        if !site_changed
            && world
                .get::<NpcActions>(actor)
                .is_some_and(|actions| actions.current == NpcAction::Talk)
        {
            // Presenter.IsEnabledCancelCondition refuses Talk(4). Hiding the
            // actor does not authorize releasing another talk owner's leases.
            runtime.sessions.insert(actor, session);
            continue;
        }
        dispose(world, session, site_changed, false);
    }
    runtime.pending_factories.clear();
    world.insert_resource(runtime);
}

/// Existing host's local no-NavMesh interpolation, with the source exit speed
/// and phase order: move first, then turn to the exit orientation. It is not a
/// replacement for the native agent's on-mesh reattachment/collision behavior.
struct LocalMove {
    pose: Transform,
    leg: crate::npc::FitLeg,
    final_rotation: Quat,
    elapsed: f32,
    turn: Option<(Quat, f32, f32)>,
}

impl LocalMove {
    fn new(start: Transform, target: Vec3, rotation: Quat, speed: f32) -> Result<Self, String> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err("NPC exit speed must retain its positive source value".into());
        }
        let delta = target - start.translation;
        let move_rotation = if delta.x == 0.0 && delta.z == 0.0 {
            start.rotation
        } else {
            Quat::from_rotation_y(delta.x.atan2(delta.z))
        };
        Ok(Self {
            pose: start,
            leg: crate::npc::FitLeg {
                start: start.translation.to_array(),
                target: target.to_array(),
                travel: delta.length() / speed,
                move_quat: move_rotation,
            },
            final_rotation: rotation,
            elapsed: 0.0,
            turn: None,
        })
    }

    fn step(&mut self, dt: f32) -> (Transform, MotionPhase, bool) {
        let mut turn_delta = dt;
        if self.turn.is_none() {
            self.elapsed += dt;
            if self.leg.travel > 0.0 && self.elapsed < self.leg.travel {
                let position = self.leg.sample_position(self.elapsed);
                // CalcMoveLerp derives heading from this frame's proposed
                // position minus the live transform. The shared kernel owns
                // the 1.5 compounded quaternion lerp and yaw projection.
                let delta = position - self.pose.translation;
                if delta.length() > 0.00001 && (delta.x != 0.0 || delta.z != 0.0) {
                    self.leg.move_quat = Quat::from_rotation_y(delta.x.atan2(delta.z));
                    self.pose.rotation = self.leg.sample_rotation(self.pose.rotation, self.elapsed);
                }
                self.pose.translation = position;
                return (self.pose, MotionPhase::Walking, false);
            }
            // NoUseNavmeshMoveLerpAsync completes with SetPosition only.
            // Its per-frame CalcMoveLerp is not evaluated again at t=1.
            self.pose.translation = Vec3::from(self.leg.target);
            let forward = (self.pose.rotation * Vec3::Z).to_array();
            let mut duration = rotate_time(angle_between(
                forward,
                (Vec3::from(self.leg.target) - self.pose.translation).to_array(),
            ));
            if duration == 0.0 {
                // Source first passes the target POSITION to GetRotateTime;
                // after exact landing that is zero. DOLocalRotateAsync then
                // passes the Euler END VALUE as a world vector. Do not replace
                // this strange location-dependent fallback with yaw error/60.
                let (z, x, y) = self.final_rotation.to_euler(EulerRot::ZXY);
                let end_value = Vec3::new(
                    make_positive_yaw(x.to_degrees()),
                    make_positive_yaw(y.to_degrees()),
                    make_positive_yaw(z.to_degrees()),
                );
                duration = rotate_time(angle_between(
                    forward,
                    (end_value - self.pose.translation).to_array(),
                ));
            }
            self.turn = Some((self.pose.rotation, duration, 0.0));
            // Do not spend this frame's complete delta on both phases.
            turn_delta = 0.0;
        }
        let (from, duration, elapsed) = self.turn.as_mut().expect("installed exit turn");
        *elapsed += turn_delta;
        let t = if *duration <= 0.0 {
            1.0
        } else {
            (*elapsed / *duration).min(1.0)
        };
        self.pose.rotation = crate::npc::sample_turn_rotation(*from, self.final_rotation, t);
        let angle = turn_angle(
            heading_yaw((*from * Vec3::Z).to_array()),
            heading_yaw((self.final_rotation * Vec3::Z).to_array()),
        );
        let phase = if t >= 1.0 {
            MotionPhase::Dwelling { remaining: None }
        } else {
            MotionPhase::Turning {
                motion: turn_motion(angle),
                from: *from,
                to: self.final_rotation,
                duration: *duration,
                elapsed: *elapsed,
            }
        };
        (self.pose, phase, t >= 1.0)
    }
}
