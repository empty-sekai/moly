//! Source-qualified furniture activity resources for live scene instances.
//!
//! Nothing here walks the placed furniture. When the player asks for a
//! fixture, the player activity owner selects one seat, joins that seat's
//! player row with its SD visual row through master tables and locators
//! (`plan_player_row`, which touches no asset), and then polls
//! `prepare_player_timeline` from its loading phase until both timelines,
//! their bindings and their sounds are resolved. The prepared profile belongs
//! to that session and is released with it. A ready visual is neither an
//! eligible seat nor a playing session. Actor libraries and source timeline
//! parsing are delegated to their shared preparation layer, which the NPC and
//! fixture-talk owners use as well.

mod assets;

use std::sync::Arc;

use bevy::{animation::graph::AnimationGraph, prelude::*};

use crate::{
    fixture_activity_data::{self, ActivityTimeline, FixtureActivityTables, PlayerTimelineRow},
    fixture_activity_state::{FixtureActivityIdentity, FixtureActivityOwner, FixtureTarget},
    fixture_activity_timeline::{
        self as timeline, StartTimeline, TimelineBindings, TimelineDefinition,
    },
    fixture_attach::AttachPoints,
    player_avatar::AvatarDriver,
    player_fixture_action::PlayerFixtureVisualProfile,
};

/// The source ViewObject's local Y, supplied by its actual scene-node owner.
/// World Y and a placement's grid center are not substitutes for this value.
#[derive(Component, Clone, Copy, Debug)]
pub(crate) struct SourceFixtureViewLocalY(pub f32);

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
                "json-loading"
                    | "fixture-clip-loading"
                    | "fixture-instance"
                    | "fixture-animation-surface"
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

#[derive(Resource, Default)]
pub(crate) struct FixtureActivityProvider {
    assets: assets::ActivityAssets,
}

impl FixtureActivityProvider {
    pub(crate) fn cache_counts(&self) -> serde_json::Value {
        self.assets.cache_counts()
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

    /// The placed instance itself is live: typed identity, formal model GLTF
    /// route and a ready scene instance. A tapped source FixtureView is always
    /// set up, so the player owner checks this before it reserves a seat.
    pub(crate) fn require_live_fixture(
        &mut self,
        world: &World,
        target: &FixtureTarget,
        model_package: &str,
    ) -> Result<(), ProviderPending> {
        assets::require_live_fixture(&mut self.assets, world, target, model_package)
    }

    /// Common resource preparation, with no player/NPC activity admission.
    /// Caller retains the prepared request and supplies its own timeout budget.
    pub(crate) fn prepare_talk_cast_bindings(
        &mut self,
        world: &mut World,
        request: &mut StartTimeline,
        cast: &[(u32, Entity, Entity, Handle<AnimationGraph>)],
    ) -> Result<(), ProviderPending> {
        let identity = world
            .get::<FixtureActivityIdentity>(request.fixture)
            .ok_or_else(|| {
                ProviderPending::new("fixture-identity", "cast fixture has no typed identity")
            })?
            .clone();
        let target = FixtureTarget {
            entity: request.fixture,
            uid: identity.uid.clone(),
        };
        assets::require_live_fixture(&mut self.assets, world, &target, &identity.model_package)?;
        let common = request
            .definition
            .tracks
            .iter()
            .any(|track| track.name == "CharacterAnimator");
        if common && cast.len() != 1 {
            return Err(ProviderPending::new(
                "actor-cast",
                "common actor timeline requires exactly one admitted actor",
            ));
        }
        let mut pending = None;
        // Start independent actor loads in the same frame, not serially. The
        // retained request owns every completed binding while siblings load.
        for (unit, actor, animator, graph) in cast {
            let result = if common {
                timeline::prepare_actor_animation_bindings(
                    world,
                    request,
                    *unit,
                    *animator,
                    graph.clone(),
                )
            } else {
                timeline::prepare_cast_actor_bindings(
                    world,
                    request,
                    *unit,
                    *actor,
                    *animator,
                    graph.clone(),
                )
            };
            if let Err(error) = result {
                if !error.retryable {
                    return Err(ProviderPending::timeline("actor-clips", error));
                }
                pending = Some(ProviderPending::timeline("actor-clips", error));
            }
        }
        // Actor name ownership must already be visible when joining the
        // fixture tracks, even while an actor's own GLTF is still arriving.
        if pending.is_none() {
            self.assets.prepare_fixture_bindings(world, request)?;
        }
        if let Err(error) = timeline::prepare_source_sounds(world, request) {
            if !error.retryable {
                return Err(ProviderPending::timeline("audio", error));
            }
            pending = Some(ProviderPending::timeline("audio", error));
        }
        if let Some(pending) = pending {
            return Err(pending);
        }
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

    /// One poll of the resources for one planned player row; later polls
    /// observe what earlier ones requested. Every call requests both timeline
    /// definitions and the actor catalogue, so they load together. The rest
    /// is routed by what those contain: once both definitions are parsed, the
    /// source SE are requested and the actor clips resolve their library
    /// index and then the library file. The fixture clips come from the
    /// placed model's own file, which is already loaded. So a request whose
    /// inputs are not resident waits for about three load rounds (tables and
    /// catalogue; library index and SE; library file) before the player
    /// moves. An SE with no route, or whose audio fails to load, plays
    /// silent instead of failing the row (`silence_player_sounds`). `draft`
    /// keeps the handles of an unfinished attempt alive between polls, so an
    /// in-flight load is not cancelled, and holds the complete profile on
    /// success.
    pub(crate) fn prepare_player_timeline(
        &mut self,
        world: &mut World,
        plan: &PlayerTimelinePlan,
        draft: &mut Option<PlayerFixtureVisualProfile>,
    ) -> Result<(), ProviderPending> {
        assets::require_live_fixture(&mut self.assets, world, &plan.target, &plan.model_package)?;
        let owner = FixtureActivityOwner { actor: plan.actor, generation: 0 };
        if let Some(profile) = draft.as_ref().filter(|profile| profile_matches_plan(profile, plan)) {
            let request = profile.start_request(owner, &plan.target);
            if timeline::validate_prepared(world, &request).is_ok() {
                // A successful profile owns the exact required clips. Do not touch
                // its whole GLTF lookup again; unrelated animations can retire.
                return Ok(());
            }
        }
        // Poll both before reading either result, so the SD visual's tables
        // are not held back behind the player row's. The row's own timeline
        // still reports first, exactly as when they were polled in turn. The
        // SD body is always bound, so the actor catalogue is always needed.
        timeline::request_actor_catalog(world);
        let player_definition =
            self.definition(world, &plan.player_package, &plan.player_row.asset_name);
        let definition = self.definition(world, &plan.visual_package, &plan.visual_prefab);
        let player_definition = player_definition?;
        let definition = definition?;
        // Retain the previous attempt's handles until the next draft is installed.
        let previous = draft.take();
        let mut profile = PlayerFixtureVisualProfile {
            target: plan.target.clone(), actor: plan.actor, unit_id: plan.unit,
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
        let mut request = profile.start_request(owner, &plan.target);
        // An SE found silent stays silent for the session: it is neither
        // requested again nor reported twice. The previous attempt's SE handles
        // come along too, so a load that has failed since is seen as failed
        // here, before `prepare_source_sounds` would request it again (loading
        // a failed path restarts it).
        if let Some(previous) = previous.as_ref() {
            request.bindings.silent_sounds = previous.bindings.silent_sounds.clone();
            request.bindings.sounds = previous.bindings.sounds.clone();
        }
        silence_player_sounds(world, &plan.target, &mut request);
        // The SE routes need only the parsed tracks: request them now, so they
        // load alongside the actor clips. `prepare_bindings` requests the same
        // handles again at its end, and only its result counts.
        let _ = timeline::prepare_source_sounds(world, &mut request);
        let prepared = self.prepare_bindings(
            world,
            &mut request,
            plan.unit,
            plan.animator,
            plan.graph.clone(),
        );
        // An SE whose audio has failed to load plays silent as well.
        silence_player_sounds(world, &plan.target, &mut request);
        let has_actor_body = request
            .bindings
            .animations
            .values()
            .any(|binding| binding.animator == plan.animator);
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
            .start_request(owner, &plan.target);
        timeline::validate_prepared(world, &request)
            .map_err(|error| ProviderPending::timeline("live-binding-preflight", error))
    }
}

/// One selected seat's player row joined with the SD visual row that shows
/// it. The row's own timeline keeps the source route: its `mdl_` package and
/// its asset name, with no height variant. The SD visual row is a
/// product-layer substitution (the source plays this row on its player
/// avatar, while this player is an SD unit), so its prefab takes the NPC
/// factory's placement-height variant. Built from master tables, locators and
/// the live instance only; nothing is loaded.
#[derive(Clone)]
pub(crate) struct PlayerTimelinePlan {
    pub target: FixtureTarget,
    pub actor: Entity,
    pub unit: u32,
    pub animator: Entity,
    pub graph: Handle<AnimationGraph>,
    pub fixture_id: i32,
    pub model_package: String,
    pub player_row: PlayerTimelineRow,
    pub locate_index: usize,
    pub slot_id: i32,
    pub no_talk_row_id: i32,
    pub visual_row: ActivityTimeline,
    pub visual_point: i32,
    pub source_view_local_y: f32,
    pub player_package: String,
    pub visual_package: String,
    pub visual_prefab: String,
}

/// The data-level join for one player row of one placed fixture.
pub(crate) fn plan_player_row(
    world: &World,
    actor: Entity,
    unit: u32,
    target: &FixtureTarget,
    identity: &FixtureActivityIdentity,
    player_row: &PlayerTimelineRow,
) -> Result<PlayerTimelinePlan, ProviderPending> {
    let (animator, graph) = world
        .get::<AvatarDriver>(actor)
        .map(AvatarDriver::fixture_timeline_binding)
        .ok_or_else(|| {
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
    let tables = world
        .get_resource::<FixtureActivityTables>()
        .ok_or_else(|| {
            ProviderPending::new("source-tables", "fixture activity tables are not parsed")
        })?;
    let points = world.get_resource::<AttachPoints>().ok_or_else(|| {
        ProviderPending::new("source-locators", "attachment metadata is not parsed")
    })?;
    if !target.matches(identity) {
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
        .get::<GlobalTransform>(target.entity)
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
    let source_view_local_y = world
        .get::<SourceFixtureViewLocalY>(target.entity)
        .map(|y| y.0)
        .ok_or_else(|| {
            ProviderPending::new(
                "source-view-local-y",
                "source ViewObject.localPosition.y has not been supplied; world Y is not a substitute",
            )
        })?;
    let mut candidates = Vec::new();
    for relation in tables.sd_visual_rows(unit, identity.master_id) {
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
                unit, identity.master_id, player_row.action_point,
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
    Ok(PlayerTimelinePlan {
        target: target.clone(),
        actor,
        unit,
        animator,
        graph,
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

/// The player timeline's SE rule (`silence_unavailable_sounds`): an SE the
/// source would fail to load plays silent. Each is reported once, when it is
/// first found, because it then stays silent for the session.
fn silence_player_sounds(world: &World, target: &FixtureTarget, request: &mut StartTimeline) {
    for silenced in timeline::silence_unavailable_sounds(world, request) {
        warn!("[player-fixture] {} SE {silenced} plays silent", target.uid);
    }
}

fn profile_matches_plan(profile: &PlayerFixtureVisualProfile, plan: &PlayerTimelinePlan) -> bool {
    profile.target == plan.target
        && profile.actor == plan.actor
        && profile.unit_id == plan.unit
        && profile.fixture_id == plan.fixture_id
        && profile.player_timeline_row_id == plan.player_row.id
        && profile.action_point == plan.player_row.action_point
        && profile.visual_action_point == plan.visual_point
        && profile.slot_id == plan.slot_id
        && profile.no_talk_row_id == plan.no_talk_row_id
        && profile.timeline_group_id == plan.visual_row.group_id
        && profile.timeline_row_id == plan.visual_row.id
        && profile.source_view_local_y.to_bits() == plan.source_view_local_y.to_bits()
        && profile.definition.package == plan.visual_package
        && profile.definition.prefab == plan.visual_prefab
        && profile.player_definition.package == plan.player_package
        && profile.player_definition.prefab == plan.player_row.asset_name
        && profile.bindings.animations.values().any(|binding|
            binding.animator == plan.animator && binding.graph == plan.graph)
}

/// PostUpdate: all provider/NPC/talk preparation has finished for this frame.
/// Only redundant lookup references are dropped. Prepared profiles and active
/// timeline requests own their exact definition/clip/audio handles themselves.
pub(crate) fn retire_lookup_caches(world: &mut World) {
    if let Some(mut provider) = world.get_resource_mut::<FixtureActivityProvider>() {
        provider.assets.sweep_lookup_caches();
    }
    if let Some(mut loads) = world.get_resource_mut::<timeline::TimelineAssetLoads>() {
        loads.sweep_lookup_caches();
    }
}
