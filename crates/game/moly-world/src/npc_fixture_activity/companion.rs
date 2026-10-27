//! A fixture action played by a body that is not a member of the NPC roster:
//! a companion. The action is one of the roster's own actions of the body's
//! unit on that fixture (a no-talk action, or the pre-action a talk of the
//! unit holds there), chosen, fitted and played by the same pieces as a
//! member's ([`Factory::select_exact`], the locator poses, the source
//! timeline on the body's animator); what the roster's objective loop does
//! around it (the approach, the talk window, the rest after it) is the
//! caller's.
//!
//! The steps, in order: [`select`] finds the action point and reserves the
//! fixture; the caller walks the body to [`CompanionAction::landing`];
//! [`CompanionAction::prepare`] loads the timeline and binds it to the body's
//! animator; [`CompanionAction::start`] reserves the slot, fits the body to
//! the StartLoc and starts the timeline; [`CompanionAction::poll`] keeps the
//! body on the StartLoc while it plays and puts it on the EndLoc when it has
//! ended. [`CompanionAction::release`] gives every lease back.

use super::*;
use moly_core::markers::GroundEpoch;
use bevy::ecs::system::SystemState;

/// One fixture action of a companion, from its selection to its end.
pub struct CompanionAction {
    /// The companion's logical root, which the locator poses move.
    root: Entity,
    /// The timeline's actor: the body, which carries the face materials.
    owner: FixtureActivityOwner,
    selection: Selection,
    request: Option<StartTimeline>,
    token: Option<TimelineToken>,
}

/// The action point of `unit`'s action at the placed fixture `uid`
/// for the companion at `root` whose body is `body`: the action named by
/// `activity` (a no-talk action's id or a pre-action's id) and `variant`
/// (the row's first timeline and the first action, no-talk actions before
/// pre-actions, when they are absent). The fixture is reserved for it.
#[allow(clippy::too_many_arguments)]
pub fn select(
    world: &mut World,
    root: Entity,
    body: Entity,
    unit: u32,
    uid: &str,
    activity: Option<i32>,
    variant: Option<u32>,
) -> Result<CompanionAction, String> {
    let (fixture, master) = world
        .query::<(Entity, &FixtureActivityIdentity)>()
        .iter(world)
        .find(|(_, identity)| identity.uid == uid)
        .map(|(entity, identity)| (entity, identity.master_id))
        .ok_or_else(|| format!("no placed fixture {uid} on this site"))?;
    let key = {
        let tables = world
            .get_resource::<FixtureActivityTables>()
            .ok_or("the fixture activity tables are not loaded")?;
        let store = world
            .get_resource::<crate::talk::TalkStore>()
            .ok_or("the talk store is not loaded")?;
        let mine: Vec<ActivitySpec> = tables
            .catalogue_activities(store)
            .into_iter()
            .filter(|spec| spec.unit == unit && spec.fixture_id == master)
            .collect();
        mine.iter()
            .filter(|spec| activity.is_none_or(|id| spec.key.source_id() == id))
            .find(|spec| variant.is_none_or(|variant| spec.variant == variant as usize))
            .map(|spec| spec.key)
            .ok_or_else(|| {
                format!(
                    "unit {unit} has no action {} variant {} at fixture {master} ({} actions there)",
                    activity.map_or("(any)".into(), |id| id.to_string()),
                    variant.map_or("(first)".into(), |variant| variant.to_string()),
                    mine.len()
                )
            })?
    };
    let site_type = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.site_type.clone())
        .ok_or("no site is active")?;
    let from = world
        .get::<WalkState>(root)
        .map(|walk| walk.0.position)
        .ok_or("the companion has no walk state")?;
    let other_targets: Vec<(Entity, Option<Entity>)> = world
        .query::<(Entity, &TalkSlot)>()
        .iter(world)
        .map(|(entity, slot)| {
            (entity, slot.current.as_ref().and_then(|data| data.target_fixture))
        })
        .collect();
    let target = FixtureTarget { entity: fixture, uid: uid.to_owned() };
    let mut rng = MemberRng::seeded(unit);
    let mut params = SystemState::<(
        Factory,
        Res<FixturePlacements>,
        Option<Res<ObjectiveFace>>,
        Option<Res<GroundEpoch>>,
    )>::new(world);
    let (selection, owner) = {
        let (mut factory, placements, face, epoch) = params
            .get_mut(world)
            .map_err(|error| format!("the fixture activity owner is not installed: {error}"))?;
        let face = face.ok_or("the site's navigation is not ready")?;
        let epoch = epoch.ok_or("the site's navigation is not ready")?.0;
        let selection = factory
            .select_exact(
                key,
                &target,
                body,
                unit,
                &site_type,
                epoch,
                from,
                &placements,
                &other_targets,
                &face,
                &mut rng,
            )?
            .ok_or("no reachable free action point beside the fixture")?;
        let owner = factory.runtime.next_owner(body);
        if !factory.reservations.reserve_npc_target(&target, owner) {
            return Err("another character is using the fixture".into());
        }
        (selection, owner)
    };
    params.apply(world);
    info!(
        "[companion-fixture] unit {unit}: action {} timeline {} at fixture {uid}; lands at {:?}",
        selection.source.source_id(),
        selection.timeline.id,
        selection.position
    );
    Ok(CompanionAction {
        root,
        owner,
        selection,
        request: None,
        token: None,
    })
}

impl CompanionAction {
    /// Where the walk to the action ends: the action point on the walk face.
    pub fn landing(&self) -> Vec3 {
        Vec3::from(self.selection.position)
    }

    /// The fixture the action is played on.
    pub fn fixture(&self) -> Entity {
        self.selection.target.entity
    }

    /// One step of the timeline's preparation on the body's animator
    /// `animator` and its graph: `Ok(true)` once the timeline is bound,
    /// `Ok(false)` while its assets load.
    pub fn prepare(
        &mut self,
        world: &mut World,
        animator: Entity,
        graph: Handle<AnimationGraph>,
    ) -> Result<bool, String> {
        let Some(mut provider) = world.remove_resource::<FixtureActivityProvider>() else {
            return Ok(false);
        };
        let result = self.bind(world, &mut provider, animator, graph);
        world.insert_resource(provider);
        match result {
            Ok(()) => Ok(true),
            Err(issue) if issue.retryable => Ok(false),
            Err(issue) => Err(format!("{}: {}", issue.stage, issue.reason)),
        }
    }

    fn bind(
        &mut self,
        world: &mut World,
        provider: &mut FixtureActivityProvider,
        animator: Entity,
        graph: Handle<AnimationGraph>,
    ) -> Result<(), crate::fixture_activity_provider::ProviderPending> {
        let selection = &self.selection;
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
        if self.request.is_none() {
            let definition = provider.definition(world, &package, &prefab)?;
            self.request = Some(StartTimeline {
                owner: TimelineOwner {
                    activity: self.owner,
                    kind: TimelineOwnerKind::Npc,
                },
                fixture: selection.target.entity,
                definition,
                bindings: TimelineBindings::default(),
                companions: Vec::new(),
                timeout_secs: 30.0,
                timeout_budget: TimelineTimeoutBudget::OwnerGated { advance: Some(true) },
            });
        }
        let request = self.request.as_mut().expect("installed request");
        provider.prepare_bindings(world, request, selection.unit, animator, graph)?;
        if !request
            .bindings
            .animations
            .values()
            .any(|binding| binding.animator == animator)
        {
            return Err("the timeline has no binding on the body".into());
        }
        fixture_activity_timeline::validate_start(world, request).map_err(|error| {
            crate::fixture_activity_provider::ProviderPending {
                stage: "live-binding-preflight",
                reason: error.to_string(),
                retryable: error.retryable,
            }
        })
    }

    /// The prepared timeline starts: the action slot is reserved and the
    /// companion stands on the StartLoc.
    pub fn start(&mut self, world: &mut World) -> Result<(), String> {
        let request = self.request.take().ok_or("the timeline is not prepared")?;
        let reserved = world
            .resource_mut::<FixtureActivityReservations>()
            .reserve_action_slot(&self.selection.target, self.selection.slot_id, self.owner);
        if !reserved {
            return Err("the action point was taken before the action began".into());
        }
        let (start, _) = live_poses(world, &self.selection)?;
        set_pose(world, self.root, start)?;
        let token = world.resource_mut::<FixtureActivityTimelines>().request_start(request);
        self.token = Some(token);
        info!(
            "[companion-fixture] unit {}: action {} timeline {} starts",
            self.selection.unit,
            self.selection.source.source_id(),
            self.selection.timeline.id
        );
        Ok(())
    }

    /// One frame of the playing action: `Ok(true)` once the timeline has
    /// ended and the companion stands on the EndLoc.
    pub fn poll(&mut self, world: &mut World) -> Result<bool, String> {
        let token = self.token.ok_or("the action has not started")?;
        let status = world
            .resource::<FixtureActivityTimelines>()
            .status(token)
            .cloned()
            .ok_or("the timeline is gone")?;
        match status {
            TimelineStatus::Preparing => Ok(false),
            TimelineStatus::Playing { .. } => {
                // The fixture is the body's parent while it plays.
                let (start, _) = live_poses(world, &self.selection)?;
                set_pose(world, self.root, start)?;
                Ok(false)
            }
            TimelineStatus::Completed => {
                let (_, end) = live_poses(world, &self.selection)?;
                world.resource_mut::<FixtureActivityTimelines>().release(token);
                self.token = None;
                set_pose(world, self.root, end)?;
                info!(
                    "[companion-fixture] unit {}: action {} ended on its EndLoc",
                    self.selection.unit,
                    self.selection.source.source_id()
                );
                Ok(true)
            }
            TimelineStatus::Cancelled => Err("the timeline was cancelled".into()),
            TimelineStatus::Failed(error) => Err(format!("the timeline failed: {error}")),
        }
    }

    /// Every lease of the action is given back; a timeline still running
    /// stops.
    pub fn release(self, world: &mut World) {
        if let Some(token) = self.token {
            if let Some(mut timelines) = world.get_resource_mut::<FixtureActivityTimelines>() {
                timelines.cancel(token);
                timelines.release(token);
            }
        }
        if let Some(mut reservations) = world.get_resource_mut::<FixtureActivityReservations>() {
            reservations.release_owner(self.owner);
        }
    }
}
