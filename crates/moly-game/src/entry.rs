//! The MySekai entry: from the opaque white cover to the player's control.
//!
//! Source order (own MySekai, not returning from another room, solo):
//! `SceneMysekai.SetupAsync` waits for the NPC spawn flags with a 3 s
//! timeout, then `JoinMysekaiActionExecutor.JoinMysekai` runs
//! `JoinMysekaiActionState`: `JoinMysekaiActionCore` (hide the player at the
//! house's outside door, camera HouseEntry, `StartMysekaiTransition` =
//! `LiveTransitioner.SafeFinish` + 0.5 s, weather banner, `PlayExitMyRoomAction`
//! = house `PlayerOff` + ExitMoveHouse + 1.5 s, `Finish`), then
//! `JoinMysekaiEndAction` (banner hide 1 s later, `OnFinishEnterAsync`: home
//! HUD and camera Normal with its 1 s transfer from HouseEntry). Afterwards
//! the back key is enabled. Every time below is relative to `SafeFinish`.
//!
//! Product adaptations, named: the embedded web stage has an empty layout
//! and no house, so it skips the join and lifts the cover at its own scene
//! readiness, with input as the stage has it; a native first site other than
//! `home_site` (an explicit choice the source never makes) does the same.
//! Named gaps: the start particle of the cover, the
//! `ScreenLayerMysekaiNotice` weather banner (INFO lines at its steps), the
//! `MysekaiTransitioner` (no product counterpart), and the stamina-refresh
//! branch (`isRefreshed` is a mock that is false). The cover's loading
//! indicator is drawn from the extracted prefab ([`indicator`]).

pub(crate) mod cover;
pub(crate) mod house;
pub(crate) mod indicator;
pub(crate) mod law;

use std::time::Duration;

use bevy::prelude::*;

use crate::{
    camera::{
        CameraSetting, CameraStateType, FieldCameraModel, FieldCameraState, NormalCameraMemory,
    },
    fixture_gimmick::house_door::{self, HouseBinding, HouseLookup},
    player::PlayerControlled,
    player_avatar::{AvatarDriver, PlayerActionToken},
    player_state::{PlayerActionState, PlayerAvatarStates},
};
use law::{ColorFade, FadeStage, NormalPrivate, UniTaskDelay};

/// Server flag `isRefreshed` of the join response (it adds the
/// StaminaRefresh topic): a named mock, not refreshed. The refresh branch
/// (`c_000_other_house1_003` and the 53-frame stamina view) is not ported.
const IS_REFRESHED: bool = false;

/// Movement hold of the join (`NavMeshAgent.enabled = false`,
/// `updatePosition = false` in `MoveToEntrance`, undone in `Finish`).
#[derive(Component)]
pub(crate) struct EntryHold;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    Join,
    /// Product adaptation: no join, cover lifted at readiness.
    CoverOnly(&'static str),
}

enum Phase {
    Loading,
    WaitCharacters {
        /// The linked timeout's delta-time timer: `Time.deltaTime` summed
        /// from the frame after the one it was created on.
        elapsed: f32,
        since_real: f64,
    },
    AwaitTransition {
        delay: UniTaskDelay,
        house: HouseBinding,
        player: Entity,
    },
    AwaitExit {
        delay: UniTaskDelay,
        player: Entity,
        token: Option<PlayerActionToken>,
    },
    Ended,
}

enum Cover {
    Opaque,
    Lifting {
        fade: ColorFade,
        destroy: UniTaskDelay,
    },
    Gone,
}

#[derive(Resource)]
pub(crate) struct EntrySequence {
    mode: Mode,
    phase: Phase,
    cover: Cover,
    frame: u64,
    started_real: f64,
    /// Frames and accumulated delta since the SafeFinish frame.
    since_finish: Option<(u64, f32)>,
    control_open: bool,
    hud_open: bool,
    site_input_open: bool,
    banner_hide: Option<UniTaskDelay>,
    end_source_t: Option<f32>,
    normal_private: Option<NormalPrivate>,
    hidden_visibility: Option<Visibility>,
    waiting_reported: bool,
}

impl EntrySequence {
    fn new(mode: Mode) -> Self {
        let open = mode != Mode::Join;
        Self {
            mode,
            phase: Phase::Loading,
            cover: Cover::Opaque,
            frame: 0,
            started_real: 0.0,
            since_finish: None,
            control_open: open,
            hud_open: open,
            site_input_open: open,
            banner_hide: None,
            end_source_t: None,
            normal_private: None,
            hidden_visibility: None,
            waiting_reported: false,
        }
    }

    /// The cover as the source's `LiveTransitioner` holds it, for host
    /// projections: lifting covers the fade delay, the fade and its hold.
    pub(crate) fn cover_state(&self) -> &'static str {
        match self.cover {
            Cover::Opaque => "opaque",
            Cover::Lifting { .. } => "fading",
            Cover::Gone => "gone",
        }
    }

    /// The await the entry is in, for host projections.
    pub(crate) fn phase_name(&self) -> &'static str {
        match (&self.mode, &self.phase) {
            (Mode::CoverOnly(_), Phase::Loading) => "cover-only",
            (Mode::Join, Phase::Loading) => "loading",
            (_, Phase::WaitCharacters { .. }) => "wait-character-spawned",
            (_, Phase::AwaitTransition { .. }) => "start-mysekai-transition",
            (_, Phase::AwaitExit { .. }) => "play-exit-my-room-action",
            (_, Phase::Ended) => "ended",
        }
    }
}

/// `MysekaiUtility.EnableJoyStick` / `EnableGestureLayer`: false until the
/// join's `Finish`. An app without the entry has the gate open.
pub(crate) fn control_open(entry: Option<&EntrySequence>) -> bool {
    entry.is_none_or(|entry| entry.control_open)
}

/// The home HUD (`ScreenLayerMysekaiHome`) exists only from
/// `HomeSiteController.OnFinishEnterAsync`.
pub(crate) fn hud_open(entry: Option<&EntrySequence>) -> bool {
    entry.is_none_or(|entry| entry.hud_open)
}

/// Site-change input: the back key is enabled after `JoinMysekai` returns.
pub(crate) fn site_input_open(entry: Option<Res<EntrySequence>>) -> bool {
    entry.as_deref().is_none_or(|entry| entry.site_input_open)
}

/// The embedded stage's `scene.ready`, mirrored by the page bridge.
#[derive(Resource, Default)]
pub(crate) struct StageSceneReady(pub(crate) bool);

/// Startup: decide the entry once for this product session.
pub(crate) fn init(
    mut commands: Commands,
    selection: Res<crate::site::SiteSelection>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    let mode = if stage.is_some() {
        Mode::CoverOnly("embedded stage: empty layout, cover lifted at scene.ready")
    } else if selection.site_type() == "home_site" {
        Mode::Join
    } else {
        Mode::CoverOnly("first site is not home_site: no house join, cover lifted at readiness")
    };
    info!(
        "[entry] mode {mode:?} (first site {})",
        selection.site_type()
    );
    commands.insert_resource(EntrySequence::new(mode));
    commands.init_resource::<StageSceneReady>();
}

fn step(seq: &EntrySequence, name: &str, source_t: Option<f32>) {
    match seq.since_finish {
        Some((frames, t)) => info!(
            "[entry] step={name} source_t={} measured_t={t:.3} frames_since_safe_finish={frames} app_frame={}",
            source_t.map_or("-".to_owned(), |t| format!("{t:.3}")),
            seq.frame
        ),
        None => info!("[entry] step={name} source_t=- measured_t=- app_frame={}", seq.frame),
    }
}

fn player_entity(world: &mut World) -> Option<Entity> {
    let mut players =
        world.query_filtered::<Entity, (With<PlayerControlled>, With<AvatarDriver>)>();
    let found: Vec<_> = players.iter(world).collect();
    match found.as_slice() {
        [player] => Some(*player),
        _ => None,
    }
}

/// What is not ready yet, or `None` when the join may begin.
fn join_blocker(world: &mut World) -> Option<String> {
    let site = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.site_type.clone());
    let Some(site) = site else {
        return Some("site not active".into());
    };
    if !world.contains_resource::<crate::site::SiteScenesReady>() {
        return Some(format!("{site} scenes are expanding"));
    }
    if !world.contains_resource::<crate::site_material::SiteMaterialsSwapped>() {
        return Some(format!("{site} materials are swapping"));
    }
    if !world.contains_resource::<FieldCameraModel>() || !world.contains_resource::<CameraSetting>()
    {
        return Some("field camera model is not set up".into());
    }
    if player_entity(world).is_none() {
        return Some("player body is not installed".into());
    }
    if !world.contains_resource::<crate::fixture::FixtureScenesReady>()
        || !crate::fixture::placements_resolved(world)
    {
        return Some("fixture layout is binding".into());
    }
    // A failed catalogue load is not waited on: the lookup names it.
    if !world.contains_resource::<house_door::HouseCatalog>()
        && world.contains_resource::<crate::fixture_gimmick::CatalogLoad>()
    {
        return Some("house controller catalogue is loading".into());
    }
    None
}

/// `WaitCharacterSpawnedAsync`: `npcList.Any()` and every NPC's spawn flag
/// (the flag is sent once each NPC exists; here: its body is installed).
fn characters_spawned(world: &mut World) -> bool {
    if !world.contains_resource::<crate::npc::Spawned>() {
        return false;
    }
    // The room's appeared flag of each NPC is set after the gate's
    // appearance has placed every visiting NPC.
    if !world
        .get_resource::<crate::npc_gate::GateAppearance>()
        .is_some_and(|gate| gate.appeared())
    {
        return false;
    }
    let Some(ids) = world
        .get_resource::<crate::npc::Registry>()
        .map(|registry| registry.character_unit_ids.clone())
    else {
        return false;
    };
    if ids.is_empty() {
        return false;
    }
    let mut members = world.query_filtered::<(
        &crate::npc::CharacterUnitId,
        Has<crate::character::MotionDriver>,
    ), Without<PlayerControlled>>();
    let members: Vec<_> = members
        .iter(world)
        .map(|(unit, driven)| (unit.0, driven))
        .collect();
    ids.iter()
        .all(|id| members.iter().any(|(unit, driven)| unit == id && *driven))
}

fn camera_fov_degrees(world: &mut World) -> f32 {
    let mut cameras = world.query_filtered::<&Projection, With<Camera3d>>();
    match cameras.single(world) {
        Ok(Projection::Perspective(perspective)) => perspective.fov.to_degrees(),
        _ => panic!("the field camera has a single perspective projection"),
    }
}

/// `LiveTransitioner.SafeFinish` -> `Finish`: indicator off, the fade
/// coroutine starts this frame, the object is destroyed 5 s later.
fn safe_finish(world: &mut World, seq: &mut EntrySequence, dt: f32) {
    if !matches!(seq.cover, Cover::Opaque) {
        return;
    }
    seq.since_finish = Some((0, 0.0));
    indicator::hide(world);
    let (fade, written) = ColorFade::start(
        law::WHITE_ALPHA_1,
        law::WHITE_ALPHA_0,
        law::COVER_FADE_DELAY,
        law::COVER_FADE_DURATION,
        dt,
    );
    if let Some(colour) = written {
        cover::write(world, colour);
    }
    seq.cover = Cover::Lifting {
        fade,
        destroy: UniTaskDelay::new(law::COVER_DESTROY_DELAY),
    };
    step(seq, "LiveTransitioner.SafeFinish", Some(0.0));
    info!("[entry] LiveTransitioner.Finish: loadingContent off, start particle resume (not drawn), ColorFader.Play(WHITE_ALPHA_0, delay 1.0, duration 1.0)");
}

fn advance_cover(world: &mut World, seq: &mut EntrySequence, dt: f32) {
    let (frames, t) = seq.since_finish.unwrap_or((0, 0.0));
    let Cover::Lifting { fade, destroy } = &mut seq.cover else {
        return;
    };
    if let Some(colour) = fade.resume(dt) {
        cover::write(world, colour);
        // The continuous source curve at this time: 1 s hold, 1 s linear.
        let nominal =
            1.0 - ((t - law::COVER_FADE_DELAY) / law::COVER_FADE_DURATION).clamp(0.0, 1.0);
        info!(
            "[entry-cover] frames_since_safe_finish={frames} measured_t={t:.3} alpha={:.4} source_curve_alpha={nominal:.4}{}",
            colour[3],
            if fade.stage() == FadeStage::Done { " (target written, fade done)" } else { "" }
        );
    }
    if destroy.advance(dt) {
        cover::destroy(world);
        seq.cover = Cover::Gone;
        step(
            seq,
            "LiveTransitioner destroyed",
            Some(law::COVER_DESTROY_DELAY),
        );
    }
}

/// The source's error branch of `JoinMysekaiActionCore` (no house or no
/// player): both transitioners finish at once and the join returns, so the
/// end action still runs; joystick and gesture layer stay disabled.
fn error_branch(world: &mut World, seq: &mut EntrySequence, reason: &str, dt: f32) {
    error!("[entry] JoinMysekaiActionCore: the room or the player does not exist ({reason}); both transitioners finish, joystick and gestures stay off");
    safe_finish(world, seq, dt);
    info!("[entry] MysekaiTransitioner.SafeFinish(null, 1.0): no product counterpart");
    end_action(world, seq, Some(0.0));
}

fn hold_player(world: &mut World, player: Entity) {
    world.entity_mut(player).insert((
        EntryHold,
        crate::npc::MotionPhase::Dwelling { remaining: None },
    ));
    if let Some(mut input) = world.get_mut::<crate::player::PlayerInput>(player) {
        input.active = false;
        input.direction = Vec3::ZERO;
    }
}

/// `JoinMysekaiActionCore` up to its first await.
fn join_core(world: &mut World, seq: &mut EntrySequence, dt: f32) {
    step(seq, "JoinMysekaiActionCore", None);
    let lookup = house_door::find_house(world);
    let player = player_entity(world);
    let house = match lookup {
        HouseLookup::Found(house) => house,
        HouseLookup::Absent => {
            return error_branch(world, seq, "no placed fixture is a home fixture", dt)
        }
        HouseLookup::Failed(reason) | HouseLookup::Pending(reason) => {
            return error_branch(world, seq, &reason, dt);
        }
    };
    let Some(player) = player else {
        return error_branch(world, seq, "no single installed player", dt);
    };
    // player.Hide()
    let before = world.get::<Visibility>(player).copied().unwrap_or_default();
    seq.hidden_visibility = Some(before);
    world.entity_mut(player).insert(Visibility::Hidden);
    step(seq, "player.Hide", None);
    if house.inside_door.is_none() {
        // The source logs an error and runs Finish(player) without animation;
        // StartMysekaiTransition is never reached, so the cover waits for its
        // own safety destroy.
        error!("[entry] {}: HouseView.InsideDoorActionPoint is null; Finish without the exit animation", house.package);
        hold_player(world, player);
        finish(world, seq, player, None, None);
        end_action(world, seq, None);
        return;
    }
    let Some(outside) = house.outside_door else {
        return error_branch(world, seq, "HouseView.OutsideDoorActionPoint is null", dt);
    };
    // MoveToEntrance: ForceSetPosition(locator), agent off, LookRotation(forward).
    let Some(locator) = world.get::<GlobalTransform>(outside).copied() else {
        return error_branch(
            world,
            seq,
            "outside door locator has no world transform",
            dt,
        );
    };
    let position = locator.translation();
    let forward = (locator.rotation() * Vec3::Z).normalize_or_zero();
    let right = Vec3::Y.cross(forward).normalize_or_zero();
    let rotation = Quat::from_mat3(&Mat3::from_cols(right, forward.cross(right), forward));
    if let Some(mut transform) = world.get_mut::<Transform>(player) {
        transform.translation = position;
        transform.rotation = rotation;
    }
    hold_player(world, player);
    info!(
        "[entry] MoveToEntrance {}: player at ({:.3}, {:.3}, {:.3}) facing ({:.3}, {:.3}, {:.3}); movement held",
        house.uid, position.x, position.y, position.z, forward.x, forward.y, forward.z
    );
    // FieldCamera.ChangeState(HouseEntry): Normal.OnExit then HouseEntry.OnEnter.
    let camera_fov = camera_fov_degrees(world);
    let setting = *world.resource::<CameraSetting>();
    let site = world
        .resource::<crate::site::SiteActive>()
        .site_type
        .clone();
    let tween = {
        let mut model = world.resource_mut::<FieldCameraModel>();
        seq.normal_private = Some(NormalPrivate {
            pitch: model.pitch,
            distance: model.distance,
            fov: setting.fov,
        });
        let memory = NormalCameraMemory {
            site,
            look_at: model.look_at,
            distance: model.distance,
            yaw: model.yaw,
            pitch: model.pitch,
            fov: model.fov,
        };
        let tween = law::enter_house_entry(&mut model, &setting, position, rotation, camera_fov);
        (memory, tween)
    };
    info!(
        "[entry] camera HouseEntry: tween {:.2}s pitch {:.1}->{:.1} yaw {:.1}->{:.1} distance {:.2}->{:.2} fov {:.1}->{:.1} look_at -> {:.3}",
        tween.1.duration, tween.1.pitch.0, tween.1.pitch.1, tween.1.yaw.0, tween.1.yaw.1,
        tween.1.distance.0, tween.1.distance.1, tween.1.fov.0, tween.1.fov.1, tween.1.look_at.1
    );
    world.insert_resource(tween.0);
    world.insert_resource(tween.1);
    world.resource_mut::<FieldCameraState>().0 = CameraStateType::HouseEntry;
    step(seq, "FieldCamera.ChangeState(HouseEntry)", None);
    // StartMysekaiTransition
    info!("[entry] MysekaiTransitioner.SafeFinish: no product counterpart");
    safe_finish(world, seq, dt);
    seq.phase = Phase::AwaitTransition {
        delay: UniTaskDelay::new(law::EXIT_HOME_ANIMATION_DELAY_TIME),
        house,
        player,
    };
}

/// `ShowSiteEnvironment` and `PlayExitMyRoomAction` up to its 1.5 s await.
fn exit_house(world: &mut World, seq: &mut EntrySequence, house: HouseBinding, player: Entity) {
    step(
        seq,
        "StartMysekaiTransition done (0.5 s)",
        Some(law::EXIT_HOME_ANIMATION_DELAY_TIME),
    );
    let phenomenon = world
        .get_resource::<crate::weather::CurrentPhenomenonId>()
        .map_or(0, |id| id.0);
    world.write_message(crate::notice_banner::SiteEnvironmentInfo::Show {
        phenomenon,
        delay: law::BANNER_SHOW_DELAY,
        duration: law::BANNER_SHOW_DURATION,
    });
    if let Err(reason) = house_door::set_trigger_player_off(world, &house) {
        error!("[entry] HouseView.SetAnimationTrigger(PlayerOff) refused: {reason}");
    }
    step(
        seq,
        "HouseView.SetAnimationTrigger(PlayerOff)",
        Some(law::EXIT_HOME_ANIMATION_DELAY_TIME),
    );
    // player.Show()
    let visibility = seq.hidden_visibility.take().unwrap_or_default();
    world.entity_mut(player).insert(visibility);
    step(
        seq,
        "player.Show",
        Some(law::EXIT_HOME_ANIMATION_DELAY_TIME),
    );
    // ChangeState(ExitMoveHouse): ChangeStatus, then the state's Initialize
    // closes the intercept gate and plays the exit clip with the presenter's
    // 0.25 s crossfade (the state and its clip are shared with the
    // room-to-home door move).
    let token = crate::site_move::door_state::change_state(
        world,
        crate::site_move::door_state::HouseState::ExitMoveHouse,
        None,
    );
    step(
        seq,
        "player.ChangeState(ExitMoveHouse)",
        Some(law::EXIT_HOME_ANIMATION_DELAY_TIME),
    );
    seq.phase = Phase::AwaitExit {
        delay: UniTaskDelay::new(law::EXIT_HOME_ANIMATION_TIME),
        player,
        token,
    };
}

/// `JoinMysekaiActionState.Finish(player)`.
fn finish(
    world: &mut World,
    seq: &mut EntrySequence,
    player: Entity,
    token: Option<PlayerActionToken>,
    source_t: Option<f32>,
) {
    world.entity_mut(player).remove::<EntryHold>();
    world.resource_mut::<PlayerAvatarStates>().can_intercept = true;
    let mut params = bevy::ecs::system::SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    if let Ok(mut driver) = drivers.get_mut(player) {
        if let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) {
            let blend = Duration::from_secs_f32(law::FINISH_IDLE_CROSSFADE);
            driver.play_idle(token, blend, &mut graphs, &mut animator, &mut transitions);
        }
    }
    world
        .resource_mut::<PlayerAvatarStates>()
        .change_status(PlayerActionState::Idle);
    seq.control_open = true;
    step(
        seq,
        "JoinMysekaiActionState.Finish: movement, idle 0.1 s, gestures and joystick on",
        source_t,
    );
}

/// `JoinMysekaiEndAction` (not refreshed) and the return of `JoinMysekai`.
/// `source_t` is this frame's source time: 2.0 after the exit, 0 in the
/// error branch (the end action follows SafeFinish in the same frame).
fn end_action(world: &mut World, seq: &mut EntrySequence, source_t: Option<f32>) {
    seq.end_source_t = source_t;
    if IS_REFRESHED {
        error!("[entry] JoinMysekaiEndAction: the stamina-refresh branch is not ported");
    }
    seq.banner_hide = Some(UniTaskDelay::new(law::BANNER_HIDE_CALL_DELAY));
    info!("[entry] JoinMysekaiEndAction: isRefreshed mock false; DelayCall({}, HideSiteEnvironment) scheduled", law::BANNER_HIDE_CALL_DELAY);
    // HomeSiteController.OnFinishEnterAsync: home HUD, then camera Normal.
    seq.hud_open = true;
    step(seq, "OnFinishEnterAsync: ScreenLayerMysekaiHome", source_t);
    world.write_message(crate::cutscene::HomeScreenStartAnimation {
        caller: "the entry's home setup",
    });
    let house_entry = world.resource::<FieldCameraState>().0 == CameraStateType::HouseEntry;
    if house_entry {
        let camera_fov = camera_fov_degrees(world);
        let setting = *world.resource::<CameraSetting>();
        let private = seq
            .normal_private
            .expect("HouseEntry was entered from Normal");
        let tween = {
            let mut model = world.resource_mut::<FieldCameraModel>();
            law::transfer_from_house_entry(&mut model, &setting, private, camera_fov)
        };
        info!(
            "[entry] camera Normal: TransferCameraSettings from HouseEntry {:.2}s pitch {:.1}->{:.1} yaw {:.1}->{:.1} distance {:.2}->{:.2} fov {:.1}->{:.1}",
            tween.duration, tween.pitch.0, tween.pitch.1, tween.yaw.0, tween.yaw.1,
            tween.distance.0, tween.distance.1, tween.fov.0, tween.fov.1
        );
        world.insert_resource(tween);
        world.resource_mut::<FieldCameraState>().0 = CameraStateType::Normal;
        step(seq, "FieldCamera.ChangeState(Normal)", source_t);
    }
    seq.site_input_open = true;
    seq.phase = Phase::Ended;
    step(seq, "JoinMysekai returned: back key enabled", source_t);
}

/// Update (exclusive): one step of the entry per frame.
pub(crate) fn advance(world: &mut World) {
    let Some(mut seq) = world.remove_resource::<EntrySequence>() else {
        return;
    };
    let dt = world.resource::<Time>().delta_secs();
    let real = world.resource::<Time<Real>>().elapsed_secs_f64();
    seq.frame += 1;
    if seq.frame == 1 {
        seq.started_real = real;
        step(&seq, "cover opaque (first frame)", None);
    }
    // The loading indicator runs while the cover is up (SafeFinish hides it).
    if matches!(seq.cover, Cover::Opaque) {
        indicator::advance(world, dt);
    }
    // Later frames of an already lifted cover, then this frame's step.
    if let Some((frames, t)) = seq.since_finish.as_mut() {
        if !matches!(seq.cover, Cover::Opaque) {
            *frames += 1;
            *t += dt;
        }
    }
    if seq.since_finish.is_some_and(|(frames, _)| frames > 0) {
        advance_cover(world, &mut seq, dt);
    }
    // LiveTransitioner.Play(timeout 300) starts DestroyMySelf: a cover that
    // no SafeFinish lifted is destroyed 300 s after it appeared. Counted
    // from the product's first frame; Play itself precedes the scene.
    if matches!(seq.cover, Cover::Opaque)
        && real - seq.started_real >= f64::from(law::COVER_SAFETY_TIMEOUT)
    {
        warn!(
            "[entry] LiveTransitioner destroyed by its {} s safety timeout without SafeFinish",
            law::COVER_SAFETY_TIMEOUT
        );
        indicator::hide(world);
        cover::destroy(world);
        seq.cover = Cover::Gone;
    }
    if let Some(delay) = seq.banner_hide.as_mut() {
        if delay.advance(dt) {
            seq.banner_hide = None;
            world.write_message(crate::notice_banner::SiteEnvironmentInfo::Hide {
                delay: 0.0,
                duration: law::BANNER_HIDE_DURATION,
            });
            let source_t = seq.end_source_t.map(|t| t + law::BANNER_HIDE_CALL_DELAY);
            step(&seq, "banner hide", source_t);
        }
    }
    match std::mem::replace(&mut seq.phase, Phase::Ended) {
        Phase::Loading => match seq.mode.clone() {
            Mode::Join => match join_blocker(world) {
                None => {
                    let site = world
                        .resource::<crate::site::SiteActive>()
                        .site_type
                        .clone();
                    if site != "home_site" {
                        seq.mode = Mode::CoverOnly("first loaded site is not home_site");
                        seq.control_open = true;
                        seq.hud_open = true;
                        seq.site_input_open = true;
                        info!("[entry] {site} loaded first: no house join (product adaptation), cover lifted now");
                        safe_finish(world, &mut seq, dt);
                    } else {
                        step(
                            &seq,
                            "site, fixtures and player ready: WaitCharacterSpawnedAsync",
                            None,
                        );
                        seq.phase = Phase::WaitCharacters {
                            elapsed: 0.0,
                            since_real: real,
                        };
                    }
                }
                Some(blocker) => {
                    if !seq.waiting_reported && real - seq.started_real > 30.0 {
                        seq.waiting_reported = true;
                        warn!("[entry] still under the cover after 30 s: {blocker}");
                    }
                    seq.phase = Phase::Loading;
                }
            },
            Mode::CoverOnly(reason) => {
                let ready = if world.contains_resource::<crate::browser_stage::BrowserStage>() {
                    world
                        .get_resource::<StageSceneReady>()
                        .is_some_and(|ready| ready.0)
                } else {
                    world.contains_resource::<crate::site::SiteScenesReady>()
                        && world.contains_resource::<crate::site_material::SiteMaterialsSwapped>()
                        && player_entity(world).is_some()
                };
                if ready {
                    info!("[entry] product adaptation ({reason}): cover Finish now");
                    safe_finish(world, &mut seq, dt);
                } else {
                    seq.phase = Phase::Loading;
                }
            }
        },
        Phase::WaitCharacters {
            elapsed,
            since_real,
        } => {
            // The timeout is a TimeoutController with the delta-time timer on
            // Update, registered before WaitUntil: each frame the timer runs
            // first, so the frame it reaches 3 s cancels the wait before the
            // predicate is read. The app's clock is the engine's deltaTime
            // (capped at maximumDeltaTime), not real time.
            let elapsed = elapsed + dt;
            let timed_out = elapsed >= law::CHARACTER_SPAWN_TIMEOUT;
            let spawned = !timed_out && characters_spawned(world);
            if spawned || timed_out {
                info!(
                    "[entry] WaitCharacterSpawnedAsync done after {elapsed:.3}s deltaTime ({:.3}s real): {}",
                    real - since_real,
                    if spawned {
                        "every NPC is spawned"
                    } else {
                        "3.0 s timeout (npcList empty or a body missing)"
                    }
                );
                join_core(world, &mut seq, dt);
            } else {
                seq.phase = Phase::WaitCharacters {
                    elapsed,
                    since_real,
                };
            }
        }
        Phase::AwaitTransition {
            mut delay,
            house,
            player,
        } => {
            if delay.advance(dt) {
                exit_house(world, &mut seq, house, player);
            } else {
                seq.phase = Phase::AwaitTransition {
                    delay,
                    house,
                    player,
                };
            }
        }
        Phase::AwaitExit {
            mut delay,
            player,
            token,
        } => {
            if delay.advance(dt) {
                let source_t =
                    Some(law::EXIT_HOME_ANIMATION_DELAY_TIME + law::EXIT_HOME_ANIMATION_TIME);
                finish(world, &mut seq, player, token, source_t);
                end_action(
                    world,
                    &mut seq,
                    Some(law::EXIT_HOME_ANIMATION_DELAY_TIME + law::EXIT_HOME_ANIMATION_TIME),
                );
            } else {
                seq.phase = Phase::AwaitExit {
                    delay,
                    player,
                    token,
                };
            }
        }
        Phase::Ended => {}
    }
    world.insert_resource(seq);
}
