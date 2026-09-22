//! Opt-in, read-only acceptance telemetry. It never creates actors, fixtures,
//! requests or synthetic assets. Ordinary runs do not touch the filesystem.
use super::*;
#[path = "qa_memory.rs"]
mod memory;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};

static BROWSER_DIAGNOSTICS: AtomicBool = AtomicBool::new(false);
static DIAGNOSTICS: OnceLock<Mutex<String>> = OnceLock::new();

/// Opt-in read-only telemetry from the same native QA projection. No command
/// is accepted and no simulation data, actor or fixture is created or changed.
/// Normal embeds never call this export; they retain the compact UI snapshot.
pub fn library_diagnostics() -> String {
    BROWSER_DIAGNOSTICS.store(true, Ordering::Relaxed);
    DIAGNOSTICS
        .get_or_init(|| Mutex::new(r#"{"schemaVersion":1,"ready":false}"#.into()))
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| r#"{"schemaVersion":1,"ready":false}"#.into())
}
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct QaDiagnostics<'w, 's> {
    weather_transition: Option<Res<'w, crate::weather_transition::WeatherTransition>>,
    weather_particles: Option<Res<'w, crate::weather_fx::WeatherFxState>>,
    particle_meshes: Res<'w, Assets<Mesh>>,
    weather_timeline: Option<Res<'w, crate::weather::WeatherTimelineState>>,
    weather_environment: Option<Res<'w, crate::env::SiteEnv>>,
    character_environment: Option<Res<'w, crate::character_material::CharacterEnv>>,
    window: Res<'w, crate::talk_window::TalkWindowState>,
    dialogue_layout: Query<'w, 's, &'static crate::talk_window::ResponsiveDialogueMetrics>,
    camera_model: Option<Res<'w, crate::camera::FieldCameraModel>>,
    camera_state: Option<Res<'w, crate::camera::FieldCameraState>>,
    camera_views: Query<'w, 's, (&'static Transform, &'static Projection), With<Camera3d>>,
    appearance_values: Res<'w, crate::room_appearance::RoomAppearance>,
    gimmicks: Res<'w, crate::fixture_gimmick::Gimmicks>,
    stage: Option<Res<'w, staging::ScenePreview>>,
    site_scenes: Option<Res<'w, crate::site::SiteScenesReady>>,
    appearance: Res<'w, crate::room_appearance::RoomAppearanceState>,
    navigation: Option<Res<'w, crate::walk_face::WalkFace>>,
    collision: Option<Res<'w, crate::fixture_collision::CollisionBakeStatus>>,
    objective: Option<Res<'w, crate::npc_objective::ObjectiveFace>>,
    layout_revision: Res<'w, crate::fixture::FixtureLayoutRevision>,
    fixture_scenes: Option<Res<'w, crate::fixture::FixtureScenesReady>>,
    fixture_materials: Option<Res<'w, crate::fixture_material::FixtureMaterialsSwapped>>,
    independent: Option<Res<'w, staging::IndependentSession>>,
    temporary_layout: Option<Res<'w, crate::fixture::TemporaryFixtureLayout>>,
    player: Query<
        'w,
        's,
        (&'static Transform, &'static crate::player::PlayerInput),
        With<crate::player::PlayerControlled>,
    >,
    voices: Query<'w, 's, Entity, With<crate::audio::VoicePlaybackIdentity>>,
    sounds: Query<'w, 's, Entity, With<crate::audio::ScopedSe>>,
    control: Option<Res<'w, crate::player_fixture_action::PlayerFixtureControlOwner>>,
    activity: Option<Res<'w, crate::npc_fixture_activity::preview::PreviewRecord>>,
    activity_bubbles: Query<'w, 's, Entity, With<crate::balloon::ActivityBalloon>>,
    harvests: Query<'w, 's, Entity, With<crate::harvest::HarvestObject>>,
    fixture_talk_action: Option<Res<'w, crate::talk::fixture_action::FixtureTalkAction>>,
    cast_timelines: Option<Res<'w, crate::fixture_activity_timeline::FixtureActivityTimelines>>,
    provider: Option<Res<'w, crate::fixture_activity_provider::FixtureActivityProvider>>,
    timeline_loads: Option<Res<'w, crate::fixture_activity_timeline::TimelineAssetLoads>>,
    gltfs: Res<'w, Assets<bevy::gltf::Gltf>>,
    images: Res<'w, Assets<Image>>,
    clips: Res<'w, Assets<AnimationClip>>,
    graphs: Res<'w, Assets<AnimationGraph>>,
    asset_server: Res<'w, AssetServer>,
    fixture_loads: Option<Res<'w, crate::fixture::FixtureGltfAssets>>,
    fixture_sources: Query<'w, 's, &'static crate::fixture::FixtureSource, With<crate::fixture::FixtureRoot>>,
    bones: Query<'w, 's, (&'static Name, &'static bevy::animation::AnimatedBy, &'static Transform)>,
}
#[derive(Default)]
pub(crate) struct QaState {
    initialized: bool,
    auto_open: bool,
    opened: bool,
    output: Option<String>,
    elapsed: f32,
}
pub(crate) fn qa_open(
    mut state: ResMut<ContentLibrary>,
    catalog: Res<LibraryCatalog>,
    context: Res<LibraryContext>,
    time: Res<Time>,
    player_session: Option<Res<crate::player_talk::PlayerTalkSession>>,
    pair_session: Option<Res<crate::talk::ActiveTalk>>,
    runtime: Res<PlayerFixtureRuntime>,
    holds: Query<Entity, With<crate::talk::TalkHold>>,
    actors: Query<(&CharacterUnitId, &Transform, Option<&crate::character::MotionDriver>, Option<&crate::npc::MotionPhase>)>,
    all: Query<Entity>,
    extra: QaDiagnostics,
    mut qa: Local<QaState>,
    selection: Res<crate::site::SiteSelection>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
) {
    if !qa.initialized {
        qa.initialized = true;
        qa.auto_open = std::env::var("MOLY_LIBRARY_OPEN")
            .ok()
            .is_some_and(|value| value == "1");
        qa.output = std::env::var("MOLY_LIBRARY_QA_OUT").ok();
    }
    if qa.auto_open && !qa.opened && catalog.talks_ready && context.revision > 0 {
        state.open = true;
        state.changed();
        qa.opened = true;
    }
    if qa.output.is_none() && !BROWSER_DIAGNOSTICS.load(Ordering::Relaxed) {
        return;
    }
    qa.elapsed += time.delta_secs();
    if qa.elapsed < 0.5 {
        return;
    }
    qa.elapsed = 0.;
    {
        let independent = extra.independent.as_ref().map(|session| serde_json::json!({
            "ticket":session.ticket, "phase":format!("{:?}",session.phase()),
            "original_site":session.original_site(), "destination":session.destination(),
            "required_units":session.required_units(), "required_fixtures":session.required_fixtures(),
            "temporary_actors":session.temporary_actors().iter().map(|entity|format!("{entity:?}")).collect::<Vec<_>>(),
            "original_player_position":session.original_player_position(),
            "temporary_layout":extra.temporary_layout.is_some(),
            "original_actor_count":session.original_actor_count(),
        }));
        let source = serde_json::json!({"region":catalog.source_region,"version":catalog.source_version,
            "viewable_furniture":catalog.fixtures.iter().filter(|row|row.source.as_ref().is_some_and(|source|source.exported)).count()});
        let transcript = serde_json::json!({"speaker":extra.window.transcript().0,
            "text":extra.window.transcript().1,"visible":extra.window.transcript().2,
            "typing":extra.window.transcript().3});
        let player = extra.player.iter().next().map(|(pose,input)|serde_json::json!({
            "position":pose.translation.to_array(),"rotation":pose.rotation.to_array(),
            "scale":pose.scale.to_array(),"moving":input.active,"direction":input.direction.to_array()}));
        let actor_rows = actors
            .iter()
            .map(|(unit, transform, driver, phase)| {
                let basis = driver.map(|driver| extra.bones.iter()
                    .filter(|(name, by, _)| by.0 == driver.player && matches!(name.as_str(), "Root" | "Hips"))
                    .map(|(name, _, pose)| serde_json::json!({
                        "name": name.as_str(), "position": pose.translation.to_array(),
                        "rotation": pose.rotation.to_array(), "forward": (pose.rotation * Vec3::Z).to_array(),
                    })).collect::<Vec<_>>());
                serde_json::json!({
            "unit":unit.0,"position":transform.translation.to_array(),"rotation":transform.rotation.to_array(),
            "phase":phase.map(|phase| match phase {
                crate::npc::MotionPhase::Walking => "walking",
                crate::npc::MotionPhase::Turning { .. } => "turning",
                crate::npc::MotionPhase::FitWalking { .. } => "fit-walking",
                crate::npc::MotionPhase::FitTurning { .. } => "fit-turning",
                crate::npc::MotionPhase::Dwelling { .. } => "idle",
            }),"boneBasis":basis})
            })
            .collect::<Vec<_>>();
        let fixture_rows = context.instances.iter().map(|(id, instances)| {
            let rows = instances.iter().map(|i| serde_json::json!({"uid":i.target.uid,
                "ready":i.ready,"reason":i.reason,"runtime":format!("{:?}",runtime.availability(&i.target)),
                "position":i.position.to_array()})).collect::<Vec<_>>();
            serde_json::json!({"id":id,"instances":rows})
        }).collect::<Vec<_>>();
        let mut value = serde_json::json!({
            "schemaVersion":1,"ready":true,
            "open": state.open, "watching": state.watching, "search": state.search,
            "mode":format!("{:?}",state.mode), "site":selection.site_type(), "ground_epoch":epoch.map(|epoch|epoch.0),
            "independent":independent,
            "scene_input_owned":state.scene_owned,
            "last_error":state.last_error,
            "room_appearance":{"ready":extra.appearance.ready,"phase":extra.appearance.phase,"error":extra.appearance.error,
                "wall":extra.appearance_values.wall,"floor":extra.appearance_values.floor},
            "camera": {
                "state":extra.camera_state.as_ref().map(|state|format!("{:?}",state.0)),
                "model":extra.camera_model.as_ref().map(|model|serde_json::json!({
                    "lookAt":model.look_at.to_array(),"offset":model.offset.to_array(),
                    "yaw":model.yaw,"pitch":model.pitch,"distance":model.distance,
                    "gesturedDistance":model.gestured_distance,"fov":model.fov,
                    "minDistance":model.min_distance,"maxDistance":model.max_distance,
                    "minPitch":model.min_pitch,"maxPitch":model.max_pitch
                })),
                "views":extra.camera_views.iter().map(|(pose,projection)|serde_json::json!({
                    "position":pose.translation.to_array(),"rotation":pose.rotation.to_array(),
                    "fov":match projection { Projection::Perspective(value)=>Some(value.fov), _=>None }
                })).collect::<Vec<_>>()
            },
            "readiness": {"site_scenes":extra.site_scenes.is_some(),
                "fixture_scenes":extra.fixture_scenes.is_some(), "fixture_materials":extra.fixture_materials.is_some(),
                "layout_revision":extra.layout_revision.0,
                "navigation_revision":extra.navigation.as_ref().map(|nav|nav.layout_revision()),
                "navigation_generation":extra.navigation.as_ref().map(|nav|nav.generation()),
                "objective_generation":extra.objective.as_ref().map(|face|face.navigation_generation())},
            "transcript":transcript,

            "gimmick_owners": crate::fixture_gimmick::session::lease_count(&extra.gimmicks),
            "scene_preview": extra.stage.is_some(), "player_control_owned":extra.control.is_some(),
            "voice_players":extra.voices.iter().count(), "scoped_sounds":extra.sounds.iter().count(),
            "fixture_outcome":format!("{:?}",runtime.last_outcome),
            "player":player,
            "selected": state.selected.map(|key| format!("{key:?}")),
            "active": state.active.as_ref().map(|active| format!("{:?}", active.choice.key)),
            "started": state.active.as_ref().is_some_and(|active| active.started),
            "pending": state.pending.as_ref().map(|pending| format!("{:?}", pending.key)),
            "status": state.status, "results": state.filtered.len(), "offset": state.offset, "page_size": state.page_size,
            "talks": catalog.talks.len(), "furniture": catalog.fixtures.len(), "issues": catalog.data_issues,
            "source":source,
            "general_owner": player_session.as_ref().map(|session| session.talk_id()),
            "fixture_owner": pair_session.as_ref().map(|session| session.talk_id()),
            "player_fixture_active": runtime.active(), "held_actors": holds.iter().count(), "entities": all.iter().count(),
            "actors":actor_rows, "fixtures":fixture_rows
        });
        value["completed"] =
            serde_json::json!(state.active.as_ref().is_some_and(|active| active.completed));
        value["navigation_geometry"] = extra.collision.as_ref().map_or(serde_json::Value::Null,
            |status| serde_json::json!({
                "ready": status.ready, "reason": status.reason,
                "colliders": status.colliders, "polygons": status.polygons,
                "mode": "canonical-collider-xz-projection"
            }));
        value["resource_counts"] = serde_json::json!({
            "gltf":extra.gltfs.len(),"meshes":extra.particle_meshes.len(),"images":extra.images.len(),
            "animationClips":extra.clips.len(),"animationGraphs":extra.graphs.len(),
            "graphNodes":extra.graphs.iter().map(|(_,graph)|graph.graph.node_count()).sum::<usize>(),
            "provider":extra.provider.as_ref().map(|p|p.cache_counts()),
            "timeline":extra.timeline_loads.as_ref().map(|p|p.cache_counts())
        });
        value["resource_residency"] = memory::diagnostics(&extra.images, &extra.particle_meshes, &extra.asset_server);
        value["resource_residency"]["fixtures"] = serde_json::json!({
            "liveInstances": extra.fixture_sources.iter().count(),
            "liveUniqueGltfs": extra.fixture_sources.iter().map(|source| source.0.id())
                .collect::<std::collections::HashSet<_>>().len(),
            "loader": extra.fixture_loads.as_ref().map(|loads| loads.residency()),
            "policy": "live instances own assets; loader releases each path after its last spawn",
        });
        value["fixture_talk_action"] = extra
            .fixture_talk_action
            .as_ref()
            .map_or(serde_json::Value::Null, |action| {
                action.diagnostics(extra.cast_timelines.as_deref())
            });
        value.as_object_mut().expect("QA object").extend(serde_json::json!({
            "weather_transition":extra.weather_transition.as_deref(),
            "weather_particles":extra.weather_particles.as_ref().map(|state|state.diagnostics(&extra.particle_meshes)),
            "weather_timeline":extra.weather_timeline.as_ref().map(|state|serde_json::json!({
                "elapsed":state.elapsed,"localTime":state.local_time,"duration":state.duration,
                "skyColor":state.values.sky_color,"lightColor":state.values.light_color,
                "skyIntensity":state.values.sky_intensity,"lightIntensity":state.values.light_intensity,
            })),
            "weather_light":extra.weather_environment.as_ref().map(|environment|environment.globals.phenomena_directional_light_color),
            "character_light":extra.character_environment.as_ref().map(|environment|environment.globals.light_color),
            "dialogue_layout":extra.dialogue_layout.iter().next().map(|layout|serde_json::json!({
                "font_px":layout.font_px,"lines":layout.line_count,
                "min":layout.bounds.min.to_array(),"max":layout.bounds.max.to_array()
            })),
            "activities":catalog.activities.len(),
            "activity":extra.activity.as_ref().map(|record|serde_json::json!({
                "ticket":record.ticket,"key":format!("{:?}",record.key),
                "actor":record.actor.map(|entity|format!("{entity:?}")),
                "phase":format!("{:?}",record.phase),"active":record.active(),"error":record.error})),
            "activity_bubbles":extra.activity_bubbles.iter().count(),
            "harvest_nodes":extra.harvests.iter().count()
        }).as_object().expect("activity QA object").clone());
        if BROWSER_DIAGNOSTICS.load(Ordering::Relaxed) {
            if let Ok(mut slot) = DIAGNOSTICS.get_or_init(|| Mutex::new(String::new())).lock() {
                *slot = value.to_string();
            }
        }
        // Native QA retains its original opt-in file sink. The browser never
        // receives a filename or gains a filesystem-write capability.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = &qa.output {
            let temp = format!("{path}.tmp");
            if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
                if std::fs::write(&temp, bytes).is_ok() {
                    let _ = std::fs::rename(&temp, path);
                }
            }
        }
    }
}
