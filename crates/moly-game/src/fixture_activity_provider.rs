//! Source-qualified furniture activity resources for live scene instances.
//!
//! This provider prepares every source player row while the player is idle;
//! navigation/seat selection remains in the player activity owner. A ready
//! visual is neither an eligible seat nor a playing session. Actor libraries
//! and source timeline parsing are delegated to their shared preparation layer.

mod assets;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::{animation::graph::AnimationGraph, prelude::*};

use crate::{
    fixture::FixtureRoot,
    fixture_activity_data::{self, ActivityTimeline, FixtureActivityTables, PlayerTimelineRow},
    fixture_activity_state::{FixtureActivityIdentity, FixtureActivityOwner, FixtureTarget},
    fixture_activity_timeline::{
        self as timeline, StartTimeline, TimelineBindings, TimelineDefinition,
    },
    fixture_attach::AttachPoints,
    npc::CharacterUnitId,
    player::PlayerControlled,
    player_avatar::AvatarDriver,
    player_fixture_action::{PlayerFixtureVisualProfile, PlayerFixtureVisualProfiles},
};

/// The source ViewObject's local Y, supplied by its actual scene-node owner.
/// World Y and a placement's grid center are not substitutes for this value.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct SourceFixtureViewLocalY(pub f32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ProviderKey {
    pub actor: Entity,
    pub target: FixtureTarget,
    pub player_timeline_row_id: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderPending {
    pub stage: &'static str,
    pub reason: String,
    pub retryable: bool,
}

impl ProviderPending {
    fn new(stage: &'static str, reason: impl Into<String>) -> Self {
        Self {
            stage,
            reason: reason.into(),
            retryable: matches!(
                stage,
                "json-loading" | "fixture-clip-loading" | "fixture-instance"
            ),
        }
    }
    fn timeline(stage: &'static str, error: timeline::TimelineFailure) -> Self {
        Self {
            stage,
            reason: error.to_string(),
            retryable: error.retryable,
        }
    }
}
impl From<String> for ProviderPending {
    fn from(reason: String) -> Self {
        Self::new("live-preflight", reason)
    }
}
impl From<&str> for ProviderPending {
    fn from(reason: &str) -> Self {
        Self::new("live-preflight", reason)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderStatus {
    Pending(ProviderPending),
    VisualReady { sd_timeline_row_id: i32 },
}

struct Entry {
    status: ProviderStatus,
    // Drafts also retain sound handles while audio is still loading. Dropping
    // the only strong handle on each failed preflight would cancel that load.
    profile: Option<PlayerFixtureVisualProfile>,
}

#[derive(Resource, Default)]
pub(crate) struct FixtureActivityProvider {
    assets: assets::ActivityAssets,
    entries: HashMap<ProviderKey, Entry>,
    published: HashSet<ProviderKey>,
    pub global_pending: Option<ProviderPending>,
    /// Entity-only diagnostics for missing UID/master/host inputs. A missing
    /// identity cannot be represented by inventing a FixtureTarget UID.
    pub host_pending: HashMap<Entity, ProviderPending>,
    pub non_timeline_masters: HashSet<Entity>,
    pub no_player_locators: HashSet<Entity>,
}

impl FixtureActivityProvider {
    pub(crate) fn statuses(&self) -> impl Iterator<Item = (&ProviderKey, &ProviderStatus)> {
        self.entries.iter().map(|(key, entry)| (key, &entry.status))
    }

    /// Exact package/prefab resolution, also usable by the NPC owner after its
    /// source factory has selected a height-qualified PreparedTimeline.
    pub(crate) fn definition(
        &mut self,
        world: &World,
        package: &str,
        prefab: &str,
    ) -> Result<Arc<TimelineDefinition>, ProviderPending> {
        self.assets.definition(world, package, prefab)
    }

    /// Common resource preparation, with no player/NPC activity admission.
    /// Caller retains the prepared request and supplies its own timeout budget.
    pub(crate) fn prepare_talk_cast_bindings(
        &mut self,
        world: &mut World,
        request: &mut StartTimeline,
        cast: &[(u32, Entity, Entity, Handle<AnimationGraph>)],
    ) -> Result<(), ProviderPending> {
        let identity = world.get::<FixtureActivityIdentity>(request.fixture)
            .ok_or_else(|| ProviderPending::new("fixture-identity", "cast fixture has no typed identity"))?.clone();
        let target = FixtureTarget { entity: request.fixture, uid: identity.uid.clone() };
        assets::require_live_fixture(&mut self.assets, world, &target, &identity.model_package)?;
        let common = request.definition.tracks.iter().any(|track| track.name == "CharacterAnimator");
        if common && cast.len() != 1 {
            return Err(ProviderPending::new("actor-cast", "common actor timeline requires exactly one admitted actor"));
        }
        let mut pending = None;
        // Start independent actor loads in the same frame, not serially. The
        // retained request owns every completed binding while siblings load.
        for (unit, actor, animator, graph) in cast {
            let result = if common {
                timeline::prepare_actor_animation_bindings(world, request, *unit, *animator, graph.clone())
            } else {
                timeline::prepare_cast_actor_bindings(world, request, *unit, *actor, *animator, graph.clone())
            };
            if let Err(error) = result {
                if !error.retryable { return Err(ProviderPending::timeline("actor-clips", error)); }
                pending = Some(ProviderPending::timeline("actor-clips", error));
            }
        }
        // Actor name ownership must already be visible when joining the
        // fixture tracks, even while an actor's own GLTF is still arriving.
        if pending.is_none() {
            self.assets.prepare_fixture_bindings(world, request)?;
        }
        if let Err(error) = timeline::prepare_source_sounds(world, request) {
            if !error.retryable { return Err(ProviderPending::timeline("audio", error)); }
            pending = Some(ProviderPending::timeline("audio", error));
        }
        if let Some(pending) = pending { return Err(pending); }
        Ok(())
    }

    pub(crate) fn prepare_bindings(
        &mut self,
        world: &mut World,
        request: &mut StartTimeline,
        unit: u32,
        animator: Entity,
        graph: Handle<AnimationGraph>,
    ) -> Result<(), ProviderPending> {
        let identity = world
            .get::<FixtureActivityIdentity>(request.fixture)
            .ok_or_else(|| {
                ProviderPending::new(
                    "fixture-identity",
                    "binding request has no typed fixture identity",
                )
            })?
            .clone();
        let target = FixtureTarget {
            entity: request.fixture,
            uid: identity.uid.clone(),
        };
        assets::require_live_fixture(&mut self.assets, world, &target, &identity.model_package)?;
        timeline::prepare_actor_animation_bindings(world, request, unit, animator, graph).map_err(
            |error| {
                let mut issue = ProviderPending::timeline("actor-source-library", error);
                issue.reason = format!(
                    "unit {unit}, prefab {}: {}",
                    request.definition.prefab, issue.reason
                );
                issue
            },
        )?;
        self.assets.prepare_fixture_bindings(world, request)?;
        timeline::prepare_source_sounds(world, request)
            .map_err(|error| ProviderPending::timeline("source-sounds", error))?;
        Ok(())
    }
}

#[derive(Clone)]
struct Actor {
    entity: Entity,
    unit: u32,
    animator: Entity,
    graph: Handle<AnimationGraph>,
}

struct Plan {
    key: ProviderKey,
    actor: Actor,
    fixture_id: i32,
    model_package: String,
    player_row: PlayerTimelineRow,
    locate_index: usize,
    slot_id: i32,
    no_talk_row_id: i32,
    visual_row: ActivityTimeline,
    visual_point: i32,
    source_view_local_y: f32,
    player_package: String,
    visual_package: String,
    visual_prefab: String,
}

struct Discovery {
    plans: Vec<(ProviderKey, Result<Plan, ProviderPending>)>,
    host_pending: HashMap<Entity, ProviderPending>,
    non_timeline_masters: HashSet<Entity>,
    no_player_locators: HashSet<Entity>,
}

/// Register before the player's visual readiness/seat preparation, not after
/// the player has already opened a session. Only this provider's prior entries
/// are replaced; independently injected profiles are not silently overwritten.
pub(crate) fn advance(world: &mut World) {
    world.init_resource::<FixtureActivityProvider>();
    world.init_resource::<PlayerFixtureVisualProfiles>();
    let mut provider = world
        .remove_resource::<FixtureActivityProvider>()
        .expect("initialized provider");
    let discovered = discover(world);
    match discovered {
        Err(reason) => {
            provider.global_pending = Some(reason);
            provider.host_pending.clear();
            provider.non_timeline_masters.clear();
            provider.no_player_locators.clear();
        }
        Ok(discovered) => {
            provider.global_pending = None;
            provider.host_pending = discovered.host_pending;
            provider.non_timeline_masters = discovered.non_timeline_masters;
            provider.no_player_locators = discovered.no_player_locators;
            let plans = discovered.plans;
            let alive: HashSet<_> = plans.iter().map(|(key, _)| key.clone()).collect();
            provider.entries.retain(|key, _| alive.contains(key));
            for (key, result) in plans {
                let mut entry = provider.entries.remove(&key).unwrap_or(Entry {
                    status: ProviderStatus::Pending(ProviderPending::new("plan", "not prepared")),
                    profile: None,
                });
                entry.status = match result {
                    Err(reason) => ProviderStatus::Pending(reason),
                    Ok(plan) => match prepare_plan(world, &mut provider, &plan, &mut entry.profile)
                    {
                        Ok(()) => ProviderStatus::VisualReady {
                            sd_timeline_row_id: plan.visual_row.id,
                        },
                        Err(reason) => ProviderStatus::Pending(reason),
                    },
                };
                provider.entries.insert(key, entry);
            }
        }
    }
    publish(world, &mut provider);
    world.insert_resource(provider);
}

/// Report preparation changes without confusing a ready visual with a running
/// activity. In particular, missing source resources remain visible even when
/// the interaction request cannot yet pass navigation admission.
pub(crate) fn report(
    provider: Option<Res<FixtureActivityProvider>>,
    mut previous: Local<Vec<String>>,
) {
    let Some(provider) = provider else {
        return;
    };
    let mut current = Vec::new();
    if let Some(pending) = &provider.global_pending {
        current.push(format!("global {}: {}", pending.stage, pending.reason));
    }
    for (entity, pending) in &provider.host_pending {
        current.push(format!(
            "fixture {entity:?} {}: {}",
            pending.stage, pending.reason
        ));
    }
    for (key, status) in provider.statuses() {
        current.push(format!(
            "fixture {:?} player-row {}: {status:?}",
            key.target.entity, key.player_timeline_row_id,
        ));
    }
    current.sort();
    if *previous != current {
        for line in &current {
            info!("[fixture-activity-provider] {line}");
        }
        *previous = current;
    }
}

fn discover(world: &mut World) -> Result<Discovery, ProviderPending> {
    let players: Vec<_> = world.query_filtered::<(Entity, Option<&CharacterUnitId>, Option<&AvatarDriver>), With<PlayerControlled>>()
        .iter(world).map(|(entity, unit, driver)| {
            (entity, unit.map(|unit| unit.0), driver.map(AvatarDriver::fixture_timeline_binding))
        }).collect();
    let [(entity, unit, binding)] = players.as_slice() else {
        return Err(ProviderPending::new(
            "player-identity",
            "one actual PlayerControlled entity is required",
        ));
    };
    let unit = unit.ok_or_else(|| {
        ProviderPending::new("player-identity", "current SD unit is not installed")
    })?;
    let (animator, graph) = binding.clone().ok_or_else(|| {
        ProviderPending::new(
            "player-rig",
            "AvatarDriver is not installed on the actual player",
        )
    })?;
    if world.get::<AnimationPlayer>(animator).is_none() {
        return Err(ProviderPending::new(
            "player-rig",
            "AvatarDriver points at a missing AnimationPlayer",
        ));
    }
    let actor = Actor {
        entity: *entity,
        unit,
        animator,
        graph,
    };
    let fixtures: Vec<_> = world
        .query_filtered::<(
            Entity,
            Option<&FixtureActivityIdentity>,
            Option<&SourceFixtureViewLocalY>,
        ), With<FixtureRoot>>()
        .iter(world)
        .map(|(entity, identity, view_y)| (entity, identity.cloned(), view_y.map(|y| y.0)))
        .collect();
    let tables = world
        .get_resource::<FixtureActivityTables>()
        .ok_or_else(|| {
            ProviderPending::new("source-tables", "fixture activity tables are not parsed")
        })?;
    let points = world.get_resource::<AttachPoints>().ok_or_else(|| {
        ProviderPending::new("source-locators", "attachment metadata is not parsed")
    })?;
    let mut plans = Vec::new();
    let mut host_pending = HashMap::new();
    let mut non_timeline_masters = HashSet::new();
    let mut no_player_locators = HashSet::new();
    for (fixture, identity, view_y) in fixtures {
        let Some(identity) = identity else {
            host_pending.insert(
                fixture,
                ProviderPending::new(
                    "fixture-identity",
                    "actual FixtureRoot has no typed UID/master/model identity yet",
                ),
            );
            continue;
        };
        if identity.uid.is_empty() {
            host_pending.insert(
                fixture,
                ProviderPending::new("fixture-identity", "typed fixture UID is empty"),
            );
            continue;
        }
        let Some(master) = tables.fixture_master(identity.master_id) else {
            host_pending.insert(
                fixture,
                ProviderPending::new(
                    "fixture-master",
                    format!(
                        "typed master {} is absent; not a no-action classification",
                        identity.master_id
                    ),
                ),
            );
            continue;
        };
        if identity.model_package != format!("mysekai__fixture__{}", master.model_name) {
            host_pending.insert(
                fixture,
                ProviderPending::new(
                    "fixture-identity",
                    "typed model package disagrees with its source master",
                ),
            );
            continue;
        }
        match master.player_action_type.as_str() {
            "no_action" | "loop" | "one_shot" => {
                non_timeline_masters.insert(fixture);
                continue;
            }
            "timeline" => {}
            other => {
                host_pending.insert(
                    fixture,
                    ProviderPending::new(
                        "fixture-master",
                        format!("unknown source player action type {other}"),
                    ),
                );
                continue;
            }
        }
        let Some(source_points) = points.player_action_points(&identity.model_package) else {
            host_pending.insert(
                fixture,
                ProviderPending::new(
                    "source-locators",
                    "source attachment array/name metadata is not complete",
                ),
            );
            continue;
        };
        let source_rows: Vec<_> = tables.player_timelines(identity.master_id).collect();
        if source_rows.is_empty() {
            host_pending.insert(
                fixture,
                ProviderPending::new(
                    "source-player-rows",
                    "timeline master has no supplied player timeline rows",
                ),
            );
            continue;
        }
        let target = FixtureTarget {
            entity: fixture,
            uid: identity.uid.clone(),
        };
        let mut seen_rows = HashSet::new();
        // The source enumerates real view locators and applies FirstOrDefault
        // to the master table. A known-invalid unused locator cannot block a
        // different valid player point by requiring every master row to exist.
        for point in source_points {
            let Some(player_row) = source_rows
                .iter()
                .find(|row| row.action_point == point)
                .copied()
            else {
                continue;
            };
            if !seen_rows.insert(player_row.id) {
                continue;
            }
            let key = ProviderKey {
                actor: actor.entity,
                target: target.clone(),
                player_timeline_row_id: player_row.id,
            };
            let result = plan_row(
                world, tables, points, &actor, &key, &identity, view_y, player_row,
            );
            plans.push((key, result));
        }
        if seen_rows.is_empty() {
            no_player_locators.insert(fixture);
        }
    }
    Ok(Discovery {
        plans,
        host_pending,
        non_timeline_masters,
        no_player_locators,
    })
}

fn plan_row(
    world: &World,
    tables: &FixtureActivityTables,
    points: &AttachPoints,
    actor: &Actor,
    key: &ProviderKey,
    identity: &FixtureActivityIdentity,
    view_y: Option<f32>,
    player_row: &PlayerTimelineRow,
) -> Result<Plan, ProviderPending> {
    if !key.target.matches(identity) {
        return Err(ProviderPending::new(
            "fixture-identity",
            "fixture UID is absent or stale",
        ));
    }
    let master = tables
        .fixture_master(identity.master_id)
        .ok_or_else(|| ProviderPending::new("fixture-master", "instance master is absent"))?;
    if master.player_action_type != "timeline" {
        return Err(ProviderPending::new(
            "fixture-master",
            "player timeline row conflicts with its fixture action type",
        ));
    }
    if identity.model_package != format!("mysekai__fixture__{}", master.model_name) {
        return Err(ProviderPending::new(
            "fixture-identity",
            "instance model differs from its exact master",
        ));
    }
    let transform = world
        .get::<GlobalTransform>(key.target.entity)
        .ok_or_else(|| ProviderPending::new("fixture-instance", "instance transform is absent"))?;
    let locate_index = points
        .instance_index(&identity.model_package, player_row.action_point)
        .ok_or_else(|| {
            ProviderPending::new(
                "source-locator",
                "source attachment index is absent or ambiguous",
            )
        })?;
    let slot_id = points
        .instance_slot(&identity.model_package, player_row.action_point)
        .ok_or_else(|| {
            ProviderPending::new(
                "source-locator",
                "source slot identity is absent or ambiguous",
            )
        })?;
    if points
        .instance_poses(&identity.model_package, player_row.action_point, transform)
        .and_then(|pair| pair.end)
        .is_none()
    {
        return Err(ProviderPending::new(
            "source-locator",
            "instance StartLoc/EndLoc pair is incomplete",
        ));
    }
    let source_view_local_y = view_y.ok_or_else(|| {
        ProviderPending::new(
            "source-view-local-y",
            "source ViewObject.localPosition.y has not been supplied; world Y is not a substitute",
        )
    })?;
    let mut candidates = Vec::new();
    for relation in tables.sd_visual_rows(actor.unit, identity.master_id) {
        for visual in tables.timeline_rows(relation.timeline_group_id) {
            let point = tables
                .action_point_value(visual.action_point_definition, 0)
                .map_err(|error| {
                    ProviderPending::new("source-visual-point", format!("{error:?}"))
                })?;
            if point == Some(player_row.action_point) {
                candidates.push((relation.id, visual.clone(), point.expect("matching point")));
            }
        }
    }
    let [(no_talk_row_id, visual_row, visual_point)] = candidates.as_slice() else {
        return Err(ProviderPending::new(
            if candidates.is_empty() { "source-visual-missing" } else { "ambiguous-source-visual" },
            format!("unit {}, fixture {}, player point {} has source candidates {:?}; no first/ready fallback",
                actor.unit, identity.master_id, player_row.action_point,
                candidates.iter().map(|(relation, row, _)| (*relation, row.id)).collect::<Vec<_>>()),
        ));
    };
    let player_package = fixture_activity_data::timeline_package(&player_row.asset_name)
        .map_err(|error| ProviderPending::new("player-timeline-route", format!("{error:?}")))?;
    let (visual_package, visual_prefab) = fixture_activity_data::timeline_asset(
        &visual_row.asset_name,
        master.put_type,
        source_view_local_y,
    )
    .map_err(|error| ProviderPending::new("sd-timeline-variant", format!("{error:?}")))?;
    Ok(Plan {
        key: key.clone(),
        actor: actor.clone(),
        fixture_id: identity.master_id,
        model_package: identity.model_package.clone(),
        player_row: player_row.clone(),
        locate_index,
        slot_id,
        no_talk_row_id: *no_talk_row_id,
        visual_row: visual_row.clone(),
        visual_point: *visual_point,
        source_view_local_y,
        player_package,
        visual_package,
        visual_prefab,
    })
}

fn prepare_plan(
    world: &mut World,
    provider: &mut FixtureActivityProvider,
    plan: &Plan,
    draft: &mut Option<PlayerFixtureVisualProfile>,
) -> Result<(), ProviderPending> {
    assets::require_live_fixture(
        &mut provider.assets,
        world,
        &plan.key.target,
        &plan.model_package,
    )?;
    let player_definition =
        provider.definition(world, &plan.player_package, &plan.player_row.asset_name)?;
    let definition = provider.definition(world, &plan.visual_package, &plan.visual_prefab)?;
    // Retain the previous attempt's handles until the next draft is installed.
    let previous = draft.take();
    let mut profile = PlayerFixtureVisualProfile {
        target: plan.key.target.clone(), actor: plan.actor.entity, unit_id: plan.actor.unit,
        fixture_id: plan.fixture_id, player_timeline_row_id: plan.player_row.id,
        action_point: plan.player_row.action_point, visual_action_point: plan.visual_point,
        slot_id: plan.slot_id, no_talk_row_id: plan.no_talk_row_id,
        timeline_group_id: plan.visual_row.group_id, timeline_row_id: plan.visual_row.id,
        source_view_local_y: plan.source_view_local_y,
        definition, player_definition, bindings: TimelineBindings::default(),
        coverage_notes: vec![
            "Native SD visual timing; player source SE retains its absolute envelopes on the same clock".into(),
            "Sampled bone curves do not implement native FootIK, start offset, track matching or all mixer behavior".into(),
            format!("Source player row {}, locator array index {}, slot {}", plan.player_row.id, plan.locate_index, plan.slot_id),
        ],
    };
    let owner = FixtureActivityOwner {
        actor: plan.actor.entity,
        generation: 0,
    };
    let mut request = profile.start_request(owner, &plan.key.target);
    let prepared = provider.prepare_bindings(
        world,
        &mut request,
        plan.actor.unit,
        plan.actor.animator,
        plan.actor.graph.clone(),
    );
    let has_actor_body = request
        .bindings
        .animations
        .values()
        .any(|binding| binding.animator == plan.actor.animator);
    profile.bindings = request.bindings;
    *draft = Some(profile);
    drop(previous);
    prepared?;
    if !has_actor_body {
        return Err(ProviderPending::new(
            "actor-body-binding",
            "selected SD visual has no actual player-body animation binding",
        ));
    }
    // Reuse the business owner's exact companion/timeout construction rather
    // than duplicating its selection or manufacturing sound event times here.
    let request = draft
        .as_ref()
        .expect("stored draft")
        .start_request(owner, &plan.key.target);
    timeline::validate_start(world, &request)
        .map_err(|error| ProviderPending::timeline("live-binding-preflight", error))
}

fn profile_key(profile: &PlayerFixtureVisualProfile) -> ProviderKey {
    ProviderKey {
        actor: profile.actor,
        target: profile.target.clone(),
        player_timeline_row_id: profile.player_timeline_row_id,
    }
}

fn publish(world: &mut World, provider: &mut FixtureActivityProvider) {
    let mut profiles = world
        .remove_resource::<PlayerFixtureVisualProfiles>()
        .unwrap_or_default();
    profiles
        .profiles
        .retain(|profile| !provider.published.contains(&profile_key(profile)));
    provider.published.clear();
    for (key, entry) in &mut provider.entries {
        if !matches!(entry.status, ProviderStatus::VisualReady { .. }) {
            continue;
        }
        if profiles
            .profiles
            .iter()
            .any(|profile| profile_key(profile) == *key)
        {
            entry.status = ProviderStatus::Pending(ProviderPending::new(
                "profile-publication-owner",
                "another producer already owns this exact profile key",
            ));
            continue;
        }
        if let Some(profile) = &entry.profile {
            profiles.profiles.push(profile.clone());
            provider.published.insert(key.clone());
        }
    }
    world.insert_resource(profiles);
}
