//! Exact catalog admission and a ticketed observation ledger for the normal
//! NPC furniture owner. Navigation, fitting, Timeline, IK and disposal remain
//! in the same owner used by autonomous activities.
use super::*;
use bevy::ecs::system::{SystemParam, SystemState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewPhase {
    Approaching,
    Preparing,
    Playing,
    Exiting,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Resource, Debug, Clone)]
pub(crate) struct PreviewRecord {
    pub ticket: u64,
    pub key: ActivityKey,
    pub actor: Option<Entity>,
    pub phase: PreviewPhase,
    pub error: Option<String>,
}
impl PreviewRecord {
    pub(crate) fn active(&self) -> bool {
        matches!(
            self.phase,
            PreviewPhase::Approaching
                | PreviewPhase::Preparing
                | PreviewPhase::Playing
                | PreviewPhase::Exiting
        )
    }
    pub(crate) fn label(&self) -> &'static str {
        match self.phase {
            PreviewPhase::Approaching => "角色正在走向家具",
            PreviewPhase::Preparing => "正在准备角色动作",
            PreviewPhase::Playing => "角色互动中",
            PreviewPhase::Exiting => "角色正在离开家具",
            PreviewPhase::Completed => "角色互动已结束",
            PreviewPhase::Cancelled => "角色互动已停止",
            PreviewPhase::Failed => "角色互动未能完成",
        }
    }
}

#[derive(SystemParam)]
struct Admission<'w, 's> {
    commands: Commands<'w, 's>,
    factory: Factory<'w, 's>,
    placements: Res<'w, FixturePlacements>,
    walk: Res<'w, crate::walk_face::WalkFace>,
    face: Res<'w, ObjectiveFace>,
    epoch: Res<'w, GroundEpoch>,
    npcs: Query<
        'w,
        's,
        (
            Entity,
            &'static CharacterUnitId,
            &'static mut WalkState,
            &'static mut PathSlot,
            &'static mut RouteStops,
            &'static mut MotionPhase,
            &'static mut NpcActions,
            &'static mut RestLifecycle,
            &'static mut ObjectiveMind,
            &'static mut TalkSlot,
            &'static mut MemberRng,
        ),
        Without<crate::player::PlayerControlled>,
    >,
}

pub(crate) fn start(world: &mut World, key: ActivityKey, target: FixtureTarget, ticket: u64) {
    let result = admit(world, key, &target, ticket);
    let (actor, phase, error) = match result {
        Ok(actor) => (Some(actor), PreviewPhase::Approaching, None),
        Err(error) => {
            warn!("[npc-fixture-preview] ticket={ticket} key={key:?}: {error}");
            (None, PreviewPhase::Failed, Some(error))
        }
    };
    world.insert_resource(PreviewRecord {
        ticket,
        key,
        actor,
        phase,
        error,
    });
}

fn admit(
    world: &mut World,
    key: ActivityKey,
    target: &FixtureTarget,
    ticket: u64,
) -> Result<Entity, String> {
    if !world.contains_resource::<crate::walk_face::WalkFace>()
        || !world.contains_resource::<ObjectiveFace>()
    {
        return Err("当前场景的可行走区域尚未就绪".into());
    }
    let mut params = SystemState::<Admission>::new(world);
    let result = {
        let mut p = params.get_mut(world);
        let spec = p
            .factory
            .tables
            .as_deref()
            .zip(p.factory.scripts.as_deref())
            .and_then(|(tables, store)| tables.resolve_activity(key, store))
            .ok_or("原始角色互动关系已经不可用")?;
        let actors: Vec<_> = p
            .npcs
            .iter()
            .filter(|(_, unit, _, _, _, _, _, _, _, _, _)| unit.0 == spec.unit)
            .map(|row| row.0)
            .collect();
        let [actor] = actors.as_slice() else {
            return Err("所选角色没有唯一的场景实例".into());
        };
        let actor = *actor;
        if p.factory.owns_actor(actor) {
            return Err("所选角色正在进行另一项家具互动".into());
        }
        let other_targets: Vec<_> = p
            .npcs
            .iter()
            .map(|row| {
                (
                    row.0,
                    row.9.current.as_ref().and_then(|slot| slot.target_fixture),
                )
            })
            .collect();
        let (
            _,
            unit,
            mut walk,
            mut path,
            mut route,
            mut phase,
            mut actions,
            mut rest,
            mut mind,
            mut slot,
            mut rng,
        ) = p
            .npcs
            .get_mut(actor)
            .map_err(|_| "所选角色的活动状态尚未就绪")?;
        if !actions.ready() {
            return Err("所选角色还没有准备好".into());
        }
        let selection = p
            .factory
            .select_exact(
                key,
                target,
                actor,
                unit.0,
                &actions.site_type,
                p.epoch.0,
                walk.0.position,
                &p.placements,
                &other_targets,
                &p.face,
                &mut rng,
            )?
            .ok_or("家具旁没有可达且未占用的原始动作点")?;
        let landing = selection.position;
        let fit = crate::npc::FitCandidate {
            position: landing,
            rotation: selection.rotation,
        };
        let Some(next_phase) = crate::npc::depart(
            unit,
            &mut walk.0,
            &mut path.0,
            &mut route,
            &p.walk,
            &p.face,
            landing,
            Some(fit),
            &mut rng,
        ) else {
            return Err("角色无法沿场景路径走到家具动作点".into());
        };
        let data = selection.ai_data();
        if !p.factory.begin(selection, actions.enable_talk) {
            route.cancel();
            path.0 = NpcPathWalkSlot::from_corners(Vec::new());
            *phase = MotionPhase::Dwelling { remaining: None };
            return Err("家具动作点刚刚被其他互动占用".into());
        }
        p.factory
            .runtime
            .sessions
            .get_mut(&actor)
            .expect("just admitted actor")
            .preview_ticket = Some(ticket);
        slot.set_current(data);
        mind.current = Some(moly_law::objective::ObjectiveType::NoneTalk);
        mind.executing = true;
        *phase = next_phase;
        crate::npc::declare_navigation_action(&mut actions, &mut rest, &phase, &route);
        p.commands.entity(actor).remove::<TalkHold>();
        Ok(actor)
    };
    params.apply(world);
    result
}

pub(super) fn record_progress(world: &mut World, session: &mut Session) {
    let Some(ticket) = session.preview_ticket else {
        return;
    };
    let phase = match session.phase {
        Phase::Approaching => PreviewPhase::Approaching,
        Phase::Preparing => PreviewPhase::Preparing,
        Phase::Playing | Phase::TalkHeld => PreviewPhase::Playing,
        Phase::Exiting(_) => PreviewPhase::Exiting,
    };
    if let Some(mut record) = world.get_resource_mut::<PreviewRecord>() {
        if record.ticket == ticket {
            record.phase = phase;
            record.error = session.last_pending.clone();
        }
    }
    if !session.tweet_issued && matches!(session.phase, Phase::Playing) {
        let playing = session.token.is_some_and(|token| {
            matches!(
                world
                    .resource_mut::<FixtureActivityTimelines>()
                    .status(token),
                Some(TimelineStatus::Playing { .. })
            )
        });
        if playing {
            session.tweet_issued = true;
            if let Some(tweet) = &session.selection.tweet {
                crate::balloon::show_activity_balloon(
                    world,
                    session.owner,
                    session.selection.unit,
                    tweet,
                );
            }
        }
    }
}

pub(super) fn record_failure(world: &mut World, session: &Session, reason: String) {
    let Some(ticket) = session.preview_ticket else {
        return;
    };
    if let Some(mut record) = world.get_resource_mut::<PreviewRecord>() {
        if record.ticket == ticket {
            record.phase = PreviewPhase::Failed;
            record.error = Some(reason);
        }
    }
}
pub(super) fn record_disposed(world: &mut World, session: &Session, completed: bool) {
    let Some(ticket) = session.preview_ticket else {
        return;
    };
    if let Some(mut record) = world.get_resource_mut::<PreviewRecord>() {
        if record.ticket == ticket && record.phase != PreviewPhase::Failed {
            record.phase = if completed {
                PreviewPhase::Completed
            } else {
                PreviewPhase::Cancelled
            };
            record.error = None;
        }
    }
}

/// Generation-scoped cancellation. Never interrupts a natural activity merely
/// because it shares an actor, furniture master ID, or a recycled catalog row.
pub(crate) fn reap(world: &mut World, retained_ticket: Option<u64>) {
    let Some(mut runtime) = world.remove_resource::<NpcFixtureActivities>() else {
        return;
    };
    let retired: Vec<_> = runtime
        .sessions
        .iter()
        .filter_map(|(actor, session)| {
            session
                .preview_ticket
                .filter(|ticket| Some(*ticket) != retained_ticket)
                .map(|_| *actor)
        })
        .collect();
    for actor in retired {
        if let Some(session) = runtime.sessions.remove(&actor) {
            dispose(world, session, false, false);
        }
    }
    world.insert_resource(runtime);
}

pub(crate) fn active(world: &World) -> bool {
    world
        .get_resource::<NpcFixtureActivities>()
        .is_some_and(|runtime| {
            runtime
                .sessions
                .values()
                .any(|session| session.preview_ticket.is_some())
        })
}
