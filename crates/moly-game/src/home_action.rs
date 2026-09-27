//! Home actions: the world half of craft, canvas and sketch.
//!
//! After the craft screen's, the canvas screen's or the sketch screen's
//! request succeeds, the source plays a short sequence on the player in the
//! field (`CraftPreview.OnCraftExecuted`, `CanvasPreview.OnCraftExecuted`,
//! `ScreenLayerMysekaiSketchPresenter.OnSuccessApi`). The three have one shape,
//! parameterised here by [`HomeAction`]:
//!
//! | step | craft | canvas | sketch |
//! |---|---|---|---|
//! | hide UI | screens fade to 0 over 0.2 s, raycasts off | same | (the sketch view hides itself) |
//! | wait | 0.2 s | 0.3 s | 0.3 s |
//! | camera | remember the state, `ChangeState(ZoomPlayer)` | same | same |
//! | player | `ChangeState(Craft)`, gate closed | `ChangeState(Draw)`, gate closed | `ChangeState(Sketch)`, gate closed |
//! | skip button | on | on | on |
//! | SE | `PlaySE(se_craft)`, handle kept | `PlaySEOneShot(se_canvas)` | `se_sketch` from the state's Initialize |
//! | hold | first of 1.8 s or skip | 2.5 s or skip | 2.0 s or skip |
//! | end | skip off; fixture effect End; if skipped: gate open, Idle, ClearAvatarItem; StopSE | skip off; gate open; Idle; ClearAvatarItem | skip off; gate open; Idle |
//! | after | dialogs; gate open, Idle, ClearAvatarItem; screens fade to 1 over 0.15 s; camera back | dialog; fade back 0.15 s; camera back | dialog; camera back; wait 0.3 s; ClearAvatarItem |
//!
//! The states (`PlayerAvatarCraftMotionState` and its two siblings):
//! Initialize plays the state's clip through the presenter's
//! `PlayAnimation(clip, 0)` (the 0 is the speed, which the motion change
//! resets to 1; the crossfade is the presenter's 0.25 s), shows the avatar and,
//! for craft and canvas, turns it to `LookRotation(target - avatar)` when the
//! player's look-at object (the pressed fixture) is set; the sketch state
//! plays `se_sketch` first. Dispose changes to the idle motion at fade 0 and
//! clears the hand items.
//!
//! The clips' AnimationEvents (`MysekaiAnimationCommand`): the item events
//! change the hand items; `PublishCurrentFixtureEffectPlay/End` publish the
//! fixture effect event, which only the fixture whose UID is the player's
//! target acts on: on Play, if it is a craft tool, it emits the craft effect
//! (effect type 15) at its view and keeps it (then sets the copy 0.22 m above
//! the view); on End it stops the kept copy. Only these three clips of the
//! motion group carry such events.
//!
//! Named gaps (the screens are drawn elsewhere): the fades, raycast
//! switches, skip button, dialogs, `SetNoticeWait`, `UpdateInfo`,
//! `SetEnableSketchButton` and `ShowBackUIScreen` are hooks in
//! [`HomeActionPresentation`] and log lines, not drawn; the dialogs take no
//! time here, so craft's "after" runs in the frame its "end" runs. The
//! player's target (the source's action data) is the fixture the request
//! names. The SE volume class is the in-game one.
//!
//! Triggers: [`HomeActionRequest`] (what the screens send; each is first
//! posted to the server model through
//! [`crate::server::client::home_action::post`], and a refused reply starts
//! nothing), [`HomeActionSkip`] (the
//! skip button), [`SketchModeRequest`] (the menu's sketch button enters game
//! state 9 and player state 27; the sketch screen's exit leaves game state 9).
//! Stand-in keys: J craft and K canvas at the nearest craft tool, L enters
//! sketch mode and then sketches, U skips, I leaves sketch mode. Instrument:
//! `MOLY_HOME_ACTION_AUTOPLAY=craft|canvas|sketch` (off by default).

use std::collections::HashSet;
use std::time::Duration;

use bevy::asset::LoadState;
use bevy::ecs::system::SystemState;
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::camera::{CameraStateType, FieldCameraModel, FieldCameraState};
use crate::fixture::FixtureRoot;
use crate::fixture_activity_state::{FixtureActivityIdentity, FixtureTarget};
use crate::harvest::{EffectHook, HarvestEffectHooks};
use crate::player::PlayerControlled;
use crate::player_avatar::avatar_item::{AvatarItem, AvatarItemRequest, AvatarItemView, Hand};
use crate::player_avatar::{
    AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, PlayerMotionError,
    STATE_FADE,
};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::server::client::home_action::{post, HomeActionApi};

/// `EffectType.Craft`.
const CRAFT_EFFECT_TYPE: u16 = 15;
/// `FixtureController.EmitEffectAsset`: the copy's position is set to the
/// view's position raised by this much.
const CRAFT_EFFECT_LIFT: f32 = 0.22;
/// The screens' fade out and fade back durations (craft and canvas).
const UI_FADE_OUT_SECONDS: f32 = 0.2;
const UI_FADE_IN_SECONDS: f32 = 0.15;
/// The sketch sequence's wait between the camera's return and the last
/// `ClearAvatarItem`.
const SKETCH_CLEAR_DELAY: f32 = 0.3;
const SE_CRAFT: &str = "se_craft";
const SE_CANVAS: &str = "se_canvas";
const SE_SKETCH: &str = "se_sketch";
const SYSTEM_FIXTURES: &str = "moly://mysekai-system-fixtures.json";
const PROBE_INTERVAL: f32 = 0.25;

/// The home action systems' set (the harvest effect player runs after it).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct HomeActionSet;

/// Which of the three world sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HomeAction {
    Craft,
    Draw,
    Sketch,
}

impl HomeAction {
    fn state(self) -> PlayerActionState {
        match self {
            Self::Craft => PlayerActionState::Craft,
            Self::Draw => PlayerActionState::Draw,
            Self::Sketch => PlayerActionState::Sketch,
        }
    }

    /// The state's `GetAnimationName`.
    fn clip(self) -> &'static str {
        match self {
            Self::Craft => "mov_u000_site_craft01_o",
            Self::Draw => "mov_u000_site_canvas_o",
            Self::Sketch => "mov_u000_site_drawingboard01_o",
        }
    }

    /// The wait after the screens hide. The craft value is stored as a
    /// double widened from the float 0.2; as a delay it is 0.2 s.
    fn wait(self) -> f32 {
        match self {
            Self::Craft => 0.2,
            Self::Draw | Self::Sketch => 0.3,
        }
    }

    /// The hold before the sequence ends, unless skipped. The craft value is
    /// the double widened from the float 1.8.
    fn hold(self) -> f32 {
        match self {
            Self::Craft => 1.8,
            Self::Draw => 2.5,
            Self::Sketch => 2.0,
        }
    }

    fn faces_target(self) -> bool {
        matches!(self, Self::Craft | Self::Draw)
    }

    fn fades_screens(self) -> bool {
        matches!(self, Self::Craft | Self::Draw)
    }

    fn api(self) -> HomeActionApi {
        match self {
            Self::Craft => HomeActionApi::Craft,
            Self::Draw => HomeActionApi::Canvas,
            Self::Sketch => HomeActionApi::Sketch,
        }
    }

    fn is_motion_state(state: PlayerActionState) -> bool {
        matches!(
            state,
            PlayerActionState::Craft | PlayerActionState::Draw | PlayerActionState::Sketch
        )
    }
}

/// What the craft, canvas and sketch screens send when their request is
/// made; the reply comes from the server model ([`post`]) and a success starts
/// the world sequence.
#[derive(Message, Debug, Clone)]
pub(crate) enum HomeActionRequest {
    /// A craft at the workbench the player pressed.
    Craft { fixture: FixtureTarget },
    /// A canvas painting, started from the same workbench's craft screen.
    Draw { fixture: FixtureTarget },
    /// A sketch from the sketch screen (game state 9) of the target
    /// fixture's blueprint (`sketchTargetBpId`; None while the sketch screen,
    /// which picks it, is not built).
    Sketch { blueprint: Option<i32> },
}

/// The skip button.
#[derive(Message, Debug, Clone, Copy)]
pub(crate) struct HomeActionSkip;

/// Game state 9 (Sketch): entered by the menu's sketch button, left when the
/// sketch screen exits.
#[derive(Message, Debug, Clone, Copy)]
pub(crate) enum SketchModeRequest {
    Enter,
    Exit,
}

/// Present while the game state is 9 (Sketch).
#[derive(Resource, Debug)]
pub(crate) struct SketchGameState;

/// The screen-side hooks of the sequence, for the screens' owner to draw.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct HomeActionPresentation {
    /// The alpha the craft or canvas screen (and the background layer) fades
    /// to, and over how long.
    pub(crate) screen_alpha: f32,
    pub(crate) fade_seconds: f32,
    /// The screens' raycasts.
    pub(crate) raycasts: bool,
    /// The skip button.
    pub(crate) skip_button: bool,
}

impl Default for HomeActionPresentation {
    fn default() -> Self {
        Self {
            screen_alpha: 1.0,
            fade_seconds: 0.0,
            raycasts: true,
            skip_button: false,
        }
    }
}

/// `UniTask.Delay`: counted from the frame after it starts.
#[derive(Debug, Clone, Copy)]
struct Delay {
    seconds: f32,
    elapsed: f32,
}

impl Delay {
    fn new(seconds: f32) -> Self {
        Self {
            seconds,
            elapsed: 0.0,
        }
    }

    fn advance(&mut self, dt: f32) -> bool {
        self.elapsed += dt;
        self.elapsed >= self.seconds
    }
}

#[derive(Debug)]
enum Phase {
    /// After the screens hide.
    Wait(Delay),
    /// `WhenAny(Delay(hold), WaitUntil(skip))`.
    Hold(Delay),
    /// The sketch sequence's wait before its last `ClearAvatarItem`.
    ClearDelay(Delay),
}

struct Sequence {
    action: HomeAction,
    target: Option<FixtureTarget>,
    phase: Phase,
    /// The sequence started in this frame's call, before the phase check:
    /// its wait does not count this frame's delta. (A later phase is made
    /// after the check and counts from the next frame by itself.)
    fresh: bool,
    started: f32,
    epoch: u64,
    remembered: Option<CameraStateType>,
    token: Option<PlayerActionToken>,
    skipped: bool,
    /// The kept `se_craft` handle's scope.
    se_scope: Option<Entity>,
    /// The target fixture's kept craft effect (its ticket).
    effect: Option<u64>,
    /// The sketch's target blueprint (`sketchTargetBpId`).
    sketch_blueprint: Option<i32>,
    events: Vec<(f32, String, String)>,
    fired: usize,
    last_probe: f32,
}

#[derive(Resource, Default)]
pub(crate) struct HomeActions {
    sequence: Option<Sequence>,
    inbox: Vec<HomeActionRequest>,
    skips: usize,
    sketch: Vec<SketchModeRequest>,
    system_request: Option<Handle<JsonAsset>>,
    /// Master fixture ids whose system fixture type is `craft_tool`.
    craft_tools: Option<HashSet<i32>>,
    tables_absent: bool,
    next_ticket: u64,
    /// Sequences finished (for the instrument).
    pub(crate) finished: usize,
}

impl HomeActions {
    pub(crate) fn playing(&self) -> bool {
        self.sequence.is_some()
    }

    /// `MysekaiFixtureUtility.IsCraftTool`: the master fixture's system
    /// fixture row is a craft tool (no row: false).
    pub(crate) fn is_craft_tool(&self, master_id: i32) -> bool {
        self.craft_tools
            .as_ref()
            .is_some_and(|tools| tools.contains(&master_id))
    }
}

pub(crate) fn load(mut actions: ResMut<HomeActions>, server: Res<AssetServer>) {
    actions.system_request = Some(server.load(bevy::asset::AssetPath::from(
        SYSTEM_FIXTURES.to_owned(),
    )));
}

/// Update: the system fixture table (for `IsCraftTool`).
pub(crate) fn parse(
    mut actions: ResMut<HomeActions>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
) {
    if actions.craft_tools.is_some() || actions.tables_absent {
        return;
    }
    let Some(handle) = actions.system_request.clone() else {
        return;
    };
    match server.load_state(&handle) {
        LoadState::Failed(error) => {
            warn!("[home-action] input absent, no fixture is a craft tool (no craft effect): {SYSTEM_FIXTURES}: {error}");
            actions.tables_absent = true;
            return;
        }
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(asset) = json.get(&handle) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(&asset.0)
        .unwrap_or_else(|error| panic!("{SYSTEM_FIXTURES}: not JSON: {error}"));
    let entries = value["entries"]
        .as_object()
        .unwrap_or_else(|| panic!("{SYSTEM_FIXTURES}: no entries"));
    let mut tools = HashSet::new();
    for row in entries.values() {
        let kind = row["mysekaiSystemFixtureType"]
            .as_str()
            .unwrap_or_else(|| panic!("{SYSTEM_FIXTURES}: row without mysekaiSystemFixtureType"));
        if kind != "craft_tool" {
            continue;
        }
        let id = row["mysekaiFixtureId"]
            .as_i64()
            .unwrap_or_else(|| panic!("{SYSTEM_FIXTURES}: row without mysekaiFixtureId"));
        tools.insert(id as i32);
    }
    let mut ids: Vec<i32> = tools.iter().copied().collect();
    ids.sort_unstable();
    info!("[home-action] craft tools (system fixture type craft_tool): master fixtures {ids:?}");
    actions.craft_tools = Some(tools);
}

/// Update: collect the requests for the exclusive step.
pub(crate) fn collect(
    mut requests: MessageReader<HomeActionRequest>,
    mut skips: MessageReader<HomeActionSkip>,
    mut sketch: MessageReader<SketchModeRequest>,
    mut actions: ResMut<HomeActions>,
) {
    actions.inbox.extend(requests.read().cloned());
    actions.skips += skips.read().count();
    actions.sketch.extend(sketch.read().copied());
}

type BodyState<'w, 's> = SystemState<(
    Query<'w, 's, &'static mut AvatarDriver, With<PlayerControlled>>,
    Query<'w, 's, (&'static mut AnimationPlayer, &'static mut AnimationTransitions)>,
    ResMut<'w, Assets<AnimationGraph>>,
)>;

/// `PlayAnimation(clip, 0)` on the player's body for the home action.
fn play_clip(world: &mut World, clip: &str) -> Result<PlayerActionToken, PlayerMotionError> {
    let mut state: BodyState = SystemState::new(world);
    let (mut drivers, mut animators, mut graphs) = state.get_mut(world);
    let Ok(mut driver) = drivers.single_mut() else {
        return Err(PlayerMotionError::MissingGraph);
    };
    let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
        return Err(PlayerMotionError::MissingGraph);
    };
    driver.start_action(
        PlayerActionOwner::HomeAction,
        PlayerActionMotion {
            clip,
            speed: 1.0,
            blend: STATE_FADE,
            blocks_manual_movement: true,
        },
        &mut graphs,
        &mut animator,
        &mut transitions,
    )
}

/// `ChangeAnimation(AvatarConfig.MysekaiIdleMotion, 0)`: the idle clip at
/// fade 0; the animator goes back to locomotion.
fn play_idle(world: &mut World, token: Option<PlayerActionToken>) {
    let mut state: BodyState = SystemState::new(world);
    let (mut drivers, mut animators, mut graphs) = state.get_mut(world);
    let Ok(mut driver) = drivers.single_mut() else {
        return;
    };
    let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
        return;
    };
    driver.play_idle(
        token,
        Duration::ZERO,
        &mut graphs,
        &mut animator,
        &mut transitions,
    );
}

/// The seek time of the clip's node on the player's animator.
fn clip_time(world: &mut World, clip: &str) -> Option<f32> {
    let mut drivers = world.query_filtered::<&AvatarDriver, With<PlayerControlled>>();
    let driver = drivers.single(world).ok()?;
    let node = driver.node_of(clip)?;
    let animator = world.get::<AnimationPlayer>(driver.player)?;
    animator.animation(node).map(|active| active.seek_time())
}

fn clip_events(world: &mut World, clip: &str) -> Option<Vec<(f32, String, String)>> {
    let mut drivers = world.query_filtered::<&AvatarDriver, With<PlayerControlled>>();
    let driver = drivers.single(world).ok()?;
    let mut events = driver.clips().get(clip)?.events.clone();
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(events)
}

fn player_entity(world: &mut World) -> Option<Entity> {
    let mut players = world.query_filtered::<Entity, With<PlayerControlled>>();
    players.single(world).ok()
}

fn play_se(world: &mut World, cue: &str, owner: Option<Entity>, source: &'static str) {
    world.resource_mut::<SeRequests>().0.push(SeRequest {
        owner,
        cue: cue.to_owned(),
        class: SeClass::Ingame,
        source,
    });
}

fn items(world: &mut World, request: AvatarItemRequest) {
    world.write_message(request);
}

fn elapsed(world: &World, sequence: &Sequence) -> f32 {
    world.resource::<Time>().elapsed_secs() - sequence.started
}

/// The player state's Initialize for the sequence's state.
fn initialize(world: &mut World, sequence: &mut Sequence) {
    let action = sequence.action;
    let t = elapsed(world, sequence);
    if action == HomeAction::Sketch {
        play_se(world, SE_SKETCH, None, "sketch-state-initialize");
        info!("[home-action] t={t:.3} Sketch.Initialize: PlaySEOneShot({SE_SKETCH})");
    }
    let clip = action.clip();
    match play_clip(world, clip) {
        Ok(token) => {
            sequence.token = Some(token);
            info!("[home-action] t={t:.3} {:?}.Initialize: PlayAnimation({clip}, speed 0 reset to 1, fade 0.25s)", action.state());
        }
        Err(PlayerMotionError::UnknownClip(name)) => {
            error!("[home-action] {:?}.Initialize refused: clip {name} is not in the avatar motion group", action.state());
        }
        Err(error) => {
            error!("[home-action] {:?}.Initialize: PlayAnimation({clip}) refused: {error:?}", action.state());
        }
    }
    sequence.events = clip_events(world, clip).unwrap_or_default();
    sequence.fired = 0;
    let Some(player) = player_entity(world) else {
        return;
    };
    // PlayerAvatarView.SetVisible(true).
    world.entity_mut(player).insert(Visibility::Inherited);
    if !action.faces_target() {
        return;
    }
    let Some(target) = sequence.target.clone() else {
        return;
    };
    let Some(at) = world.get::<GlobalTransform>(target.entity).map(GlobalTransform::translation) else {
        warn!("[home-action] {:?}.Initialize: look-at fixture {} is gone; not turned", action.state(), target.uid);
        return;
    };
    let Some(mut transform) = world.get_mut::<Transform>(player) else {
        return;
    };
    // Quaternion.LookRotation(target - avatar) with up = Y (the avatar faces
    // its local +Z); a zero vector gives the identity.
    let forward = (at - transform.translation).normalize_or_zero();
    let rotation = if forward == Vec3::ZERO {
        Quat::IDENTITY
    } else {
        let right = Vec3::Y.cross(forward).normalize_or_zero();
        if right == Vec3::ZERO {
            Quat::IDENTITY
        } else {
            Quat::from_mat3(&Mat3::from_cols(right, forward.cross(right), forward))
        }
    };
    transform.rotation = rotation;
    let facing = rotation * Vec3::Z;
    info!(
        "[home-action] t={t:.3} {:?}.Initialize: SetVisible(true); turned to LookAtObject {} ({:?}) at ({:.2},{:.2},{:.2}): facing ({:.3},{:.3},{:.3})",
        action.state(),
        target.uid,
        target.entity,
        at.x,
        at.y,
        at.z,
        facing.x,
        facing.y,
        facing.z
    );
}

/// `ChangeStatus(status)` for the sequence, with the leaving motion state's
/// Dispose (idle at fade 0, ClearAvatarItem).
fn change_state(world: &mut World, sequence: &mut Sequence, status: PlayerActionState) -> bool {
    let before = world.resource::<PlayerAvatarStates>().current;
    world.resource_mut::<PlayerAvatarStates>().change_status(status);
    let after = world.resource::<PlayerAvatarStates>().current;
    if before == after {
        return after == status;
    }
    if HomeAction::is_motion_state(before) {
        let t = elapsed(world, sequence);
        play_idle(world, sequence.token.take());
        items(world, AvatarItemRequest::Clear);
        info!("[home-action] t={t:.3} {before:?}.Dispose: ChangeAnimation(c_000_mov_idle_00, fade 0); ClearAvatarItem");
    }
    after == status
}

fn set_gate(world: &mut World, open: bool) {
    world.resource_mut::<PlayerAvatarStates>().can_intercept = open;
}

/// `EventDispatcher.Publish(FixtureEffectEvent, Play/End)` as the target
/// fixture's controller receives it.
fn fixture_effect(world: &mut World, actions: &mut HomeActions, sequence: &mut Sequence, play: bool, source: &str) {
    let t = elapsed(world, sequence);
    let Some(target) = sequence.target.clone() else {
        info!("[home-action] t={t:.3} FixtureEffectEvent({}) from {source}: the player has no target fixture; no controller acts", if play { "Play" } else { "End" });
        return;
    };
    if !play {
        match sequence.effect.take() {
            Some(ticket) => {
                world.resource_mut::<HarvestEffectHooks>().stops.push(ticket);
                info!("[home-action] t={t:.3} FixtureEffectEvent(End) from {source} on {} ({:?}): the kept craft effect stops", target.uid, target.entity);
            }
            None => info!("[home-action] t={t:.3} FixtureEffectEvent(End) from {source} on {}: no effect is kept", target.uid),
        }
        return;
    }
    let master = world.get::<FixtureActivityIdentity>(target.entity).map(|identity| identity.master_id);
    let Some(master) = master else {
        warn!("[home-action] t={t:.3} FixtureEffectEvent(Play) from {source}: fixture {} has no identity; no effect", target.uid);
        return;
    };
    if !actions.is_craft_tool(master) {
        info!("[home-action] t={t:.3} FixtureEffectEvent(Play) from {source} on {} (master {master}): not a craft tool; no effect", target.uid);
        return;
    }
    let Some(view) = world.get::<GlobalTransform>(target.entity).map(GlobalTransform::translation) else {
        return;
    };
    let at = view + Vec3::Y * CRAFT_EFFECT_LIFT;
    actions.next_ticket += 1;
    let ticket = actions.next_ticket;
    world
        .resource_mut::<HarvestEffectHooks>()
        .kept
        .push((ticket, EffectHook::at(CRAFT_EFFECT_TYPE, at)));
    sequence.effect = Some(ticket);
    info!(
        "[home-action] t={t:.3} FixtureEffectEvent(Play) from {source} on {} ({:?}, master {master}, craft tool): Emit({CRAFT_EFFECT_TYPE}) at view ({:.2},{:.2},{:.2}) + {CRAFT_EFFECT_LIFT} up, kept",
        target.uid, target.entity, view.x, view.y, view.z
    );
}

/// The clip's AnimationEvents whose time the clip has reached.
fn dispatch_events(world: &mut World, actions: &mut HomeActions, sequence: &mut Sequence) {
    if world.resource::<PlayerAvatarStates>().current != sequence.action.state()
        || sequence.token.is_none()
    {
        return;
    }
    let clip = sequence.action.clip();
    let Some(now) = clip_time(world, clip) else {
        return;
    };
    while let Some((time, function, parameter)) = sequence.events.get(sequence.fired).cloned() {
        if time > now {
            break;
        }
        sequence.fired += 1;
        let t = elapsed(world, sequence);
        match function.as_str() {
            "PublishAvatarItemUseLeft" | "PublishAvatarItemUseRight" => {
                let hand = if function.ends_with("Left") { Hand::Left } else { Hand::Right };
                let item = AvatarItem::from_name(&parameter);
                items(world, AvatarItemRequest::Update { hand, item });
                info!("[home-action] t={t:.3} clip {clip} at {now:.3} (event {time:.3}): {function}({parameter}) -> UpdateAvatarItem({hand:?}, {item:?}, file {:?})", item.file_name());
            }
            "PublishCurrentFixtureEffectPlay" => {
                fixture_effect(world, actions, sequence, true, "the clip");
            }
            "PublishCurrentFixtureEffectEnd" => {
                fixture_effect(world, actions, sequence, false, "the clip");
            }
            other => {
                info!("[home-action] t={t:.3} clip {clip} event {other}({parameter}) at {time:.3}: not a home action event");
            }
        }
    }
}

/// One evidence line: the player state, the clip's time, the camera and the
/// hand items, read from their own state.
fn probe(world: &mut World, sequence: &Sequence, label: &str) {
    let t = elapsed(world, sequence);
    let state = world.resource::<PlayerAvatarStates>().current;
    let clip = clip_time(world, sequence.action.clip())
        .map_or_else(|| "not playing".to_owned(), |seek| format!("{seek:.3}"));
    let camera_state = world.resource::<FieldCameraState>().0;
    let camera = world.get_resource::<FieldCameraModel>().map(|model| {
        let pivot = model.look_at + model.offset;
        (model.distance, model.yaw, model.pitch, pivot)
    });
    let mut cameras = world.query_filtered::<&Transform, With<Camera3d>>();
    let eye = cameras.single(world).ok().map(|transform| transform.translation);
    let camera = match (camera, eye) {
        (Some((distance, yaw, pitch, pivot)), Some(eye)) => format!(
            "camera {camera_state:?} model distance {distance:.3} yaw {yaw:.2} pitch {pitch:.2}, eye-pivot {:.3}",
            eye.distance(pivot)
        ),
        _ => format!("camera {camera_state:?}"),
    };
    let hands = world
        .get_resource::<AvatarItemView>()
        .map_or_else(|| "no item view".to_owned(), AvatarItemView::describe);
    info!(
        "[home-action] probe {label} t={t:.3} {:?}: state {state:?}, clip {} t {clip}, {camera}; hands: {hands}; craft effect {}",
        sequence.action,
        sequence.action.clip(),
        if sequence.effect.is_some() { "kept" } else { "none" }
    );
}

fn hide_screens(world: &mut World, sequence: &Sequence) {
    if !sequence.action.fades_screens() {
        info!("[home-action] t=0.000 {:?}: the sketch view hides itself (no fade in this sequence)", sequence.action);
        return;
    }
    let mut presentation = world.resource_mut::<HomeActionPresentation>();
    presentation.screen_alpha = 0.0;
    presentation.fade_seconds = UI_FADE_OUT_SECONDS;
    presentation.raycasts = false;
    info!(
        "[home-action] t=0.000 {:?}: background layer and screen DOFade(0, {UI_FADE_OUT_SECONDS}), raycasts off (hook; the screens are not drawn here)",
        sequence.action
    );
}

fn start(world: &mut World, actions: &mut HomeActions, request: HomeActionRequest) {
    if let Some(sequence) = &actions.sequence {
        warn!("[home-action] {request:?} dropped: {:?} is still playing", sequence.action);
        return;
    }
    let mut sketch_blueprint = None;
    let (action, target) = match request {
        HomeActionRequest::Craft { fixture } => (HomeAction::Craft, Some(fixture)),
        HomeActionRequest::Draw { fixture } => (HomeAction::Draw, Some(fixture)),
        HomeActionRequest::Sketch { blueprint } => {
            sketch_blueprint = blueprint;
            (HomeAction::Sketch, None)
        }
    };
    if action == HomeAction::Sketch && world.get_resource::<SketchGameState>().is_none() {
        warn!("[home-action] Sketch requested outside game state 9; the sketch screen sends it from that state");
    }
    if let Some(target) = &target {
        if world.get_entity(target.entity).is_err() {
            warn!("[home-action] {action:?} dropped: fixture {} ({:?}) is gone", target.uid, target.entity);
            return;
        }
    }
    let reply = post(world, action.api(), target.as_ref().map(|target| target.uid.as_str()));
    if !reply.success {
        return;
    }
    let now = world.resource::<Time>().elapsed_secs();
    let epoch = world.get_resource::<crate::site::GroundEpoch>().map_or(0, |epoch| epoch.0);
    let sequence = Sequence {
        action,
        target,
        phase: Phase::Wait(Delay::new(action.wait())),
        fresh: true,
        started: now,
        epoch,
        remembered: None,
        token: None,
        skipped: false,
        se_scope: None,
        effect: None,
        sketch_blueprint,
        events: Vec::new(),
        fired: 0,
        last_probe: f32::NEG_INFINITY,
    };
    info!(
        "[home-action] {action:?} starts (target {}): wait {}s, hold {}s",
        sequence
            .target
            .as_ref()
            .map_or_else(|| "none".to_owned(), |target| format!("{} {:?}", target.uid, target.entity)),
        action.wait(),
        action.hold()
    );
    hide_screens(world, &sequence);
    actions.sequence = Some(sequence);
}

/// After the wait: camera, state, gate, skip button, SE.
fn begin_motion(world: &mut World, sequence: &mut Sequence) {
    let t = elapsed(world, sequence);
    match crate::zoom_player_camera::enter(world) {
        Ok(previous) => sequence.remembered = Some(previous),
        Err(reason) => error!("[home-action] t={t:.3} FieldCamera.ChangeState(ZoomPlayer) refused: {reason}"),
    }
    let state = sequence.action.state();
    if change_state(world, sequence, state) {
        initialize(world, sequence);
    } else {
        warn!(
            "[home-action] t={t:.3} ChangeState({state:?}) dropped: the intercept gate is closed (state {:?})",
            world.resource::<PlayerAvatarStates>().current
        );
    }
    set_gate(world, false);
    sequence.skipped = false;
    world.resource_mut::<HomeActionPresentation>().skip_button = true;
    info!("[home-action] t={t:.3} SetInterceptFlag(false); skip button on");
    match sequence.action {
        HomeAction::Craft => {
            let scope = world.spawn(Name::new("home action se_craft handle")).id();
            play_se(world, SE_CRAFT, Some(scope), "home-action-craft");
            sequence.se_scope = Some(scope);
            info!("[home-action] t={t:.3} PlaySE({SE_CRAFT}): handle kept ({scope:?})");
        }
        HomeAction::Draw => {
            play_se(world, SE_CANVAS, None, "home-action-canvas");
            info!("[home-action] t={t:.3} PlaySEOneShot({SE_CANVAS})");
        }
        HomeAction::Sketch => {}
    }
}

fn camera_back(world: &mut World, sequence: &mut Sequence) {
    if let Some(remembered) = sequence.remembered.take() {
        crate::zoom_player_camera::leave(world, remembered);
    }
}

fn show_screens(world: &mut World, sequence: &Sequence) {
    let t = elapsed(world, sequence);
    let mut presentation = world.resource_mut::<HomeActionPresentation>();
    presentation.screen_alpha = 1.0;
    presentation.fade_seconds = UI_FADE_IN_SECONDS;
    presentation.raycasts = true;
    info!("[home-action] t={t:.3} screens DOFade(1, {UI_FADE_IN_SECONDS}), raycasts on (hook)");
}

/// The hold ended (its delay or the skip). Returns true when the sequence
/// is done.
fn end(world: &mut World, actions: &mut HomeActions, sequence: &mut Sequence) -> bool {
    let t = elapsed(world, sequence);
    world.resource_mut::<HomeActionPresentation>().skip_button = false;
    info!(
        "[home-action] t={t:.3} hold ends ({}); skip button off",
        if sequence.skipped { "skipped" } else { "its delay" }
    );
    probe(world, sequence, "end");
    match sequence.action {
        HomeAction::Craft => {
            fixture_effect(world, actions, sequence, false, "the sequence");
            info!("[home-action] t={t:.3} MysekaiMissionUtility.SetNoticeWait(false): no counterpart");
            if sequence.skipped {
                set_gate(world, true);
                change_state(world, sequence, PlayerActionState::Idle);
                items(world, AvatarItemRequest::Clear);
                info!("[home-action] t={t:.3} skipped: SetInterceptFlag(true), ChangeState(Idle), ClearAvatarItem");
            }
            if let Some(scope) = sequence.se_scope.take() {
                crate::audio::dispose_scoped_se(world, scope);
                if let Ok(entity) = world.get_entity_mut(scope) {
                    entity.despawn();
                }
                info!("[home-action] t={t:.3} StopSE({SE_CRAFT} handle {scope:?})");
            }
            info!("[home-action] t={t:.3} OpenCreateResponseDialogAsync, the fixture bonus dialog and the rank dialog: the screens' owner draws them; no time passes here");
            set_gate(world, true);
            change_state(world, sequence, PlayerActionState::Idle);
            items(world, AvatarItemRequest::Clear);
            info!("[home-action] t={t:.3} SetInterceptFlag(true), ChangeState(Idle), ClearAvatarItem");
            show_screens(world, sequence);
            camera_back(world, sequence);
            info!("[home-action] t={t:.3} HeaderUtility.ShowBackUIScreen: the screens' owner");
            true
        }
        HomeAction::Draw => {
            set_gate(world, true);
            change_state(world, sequence, PlayerActionState::Idle);
            items(world, AvatarItemRequest::Clear);
            info!("[home-action] t={t:.3} SetInterceptFlag(true), ChangeState(Idle), ClearAvatarItem");
            info!("[home-action] t={t:.3} OpenCreateResponseDialog: the screens' owner draws it; no time passes here");
            show_screens(world, sequence);
            camera_back(world, sequence);
            info!("[home-action] t={t:.3} the card bonus dialog and ShowBackUIScreen: the screens' owner");
            true
        }
        HomeAction::Sketch => {
            info!("[home-action] t={t:.3} ScreenLayerMysekaiSketchView.SetEnableSketchButton: the screen's owner");
            set_gate(world, true);
            change_state(world, sequence, PlayerActionState::Idle);
            info!("[home-action] t={t:.3} SetInterceptFlag(true), ChangeState(Idle); SetNoticeWait: no counterpart");
            // `await SketchUtility.ShowSketchResultDialog(sketchTargetBpId,
            // onClose)` awaits the dialog's Setup, not its close (the onClose
            // re-enables the sketch screen's UI, the screen's owner's).
            match sequence.sketch_blueprint {
                Some(blueprint) => {
                    let ticket = world
                        .resource_mut::<crate::get_resource::GetResourceOpeners>()
                        .show_sketch_result(blueprint);
                    info!("[home-action] t={t:.3} SketchUtility.ShowSketchResultDialog({blueprint}) (ticket {ticket}); its Setup takes no time here; UpdateInfo: the screen's owner");
                }
                None => warn!("[home-action] t={t:.3} SketchUtility.ShowSketchResultDialog not called: the request names no target blueprint (the sketch screen that picks it is not built)"),
            }
            camera_back(world, sequence);
            // Created after this frame's check: counts from the next frame.
            sequence.phase = Phase::ClearDelay(Delay::new(SKETCH_CLEAR_DELAY));
            false
        }
    }
}

/// Leave the sequence when the site changes under it.
fn cancel(world: &mut World, actions: &mut HomeActions, mut sequence: Sequence, reason: &str) {
    warn!("[home-action] {:?} cancelled: {reason}", sequence.action);
    if let Some(ticket) = sequence.effect.take() {
        world.resource_mut::<HarvestEffectHooks>().stops.push(ticket);
    }
    if let Some(scope) = sequence.se_scope.take() {
        crate::audio::dispose_scoped_se(world, scope);
        if let Ok(entity) = world.get_entity_mut(scope) {
            entity.despawn();
        }
    }
    world.resource_mut::<HomeActionPresentation>().skip_button = false;
    set_gate(world, true);
    change_state(world, &mut sequence, PlayerActionState::Idle);
    items(world, AvatarItemRequest::Clear);
    actions.finished += 1;
}

fn sketch_mode(world: &mut World, request: SketchModeRequest) {
    match request {
        SketchModeRequest::Enter => {
            let before = world.resource::<PlayerAvatarStates>().current;
            world
                .resource_mut::<PlayerAvatarStates>()
                .change_status(PlayerActionState::SelectSketchItem);
            let after = world.resource::<PlayerAvatarStates>().current;
            world.insert_resource(SketchGameState);
            info!(
                "[home-action] menu sketch button: Player.ChangeState(SelectSketchItem) ({before:?} -> {after:?}; Initialize clears the next state), ChangeState(GameState Sketch = 9): ChangeGameState(9) published; PushUIScreen(MysekaiSketchUI) is the screens' owner's"
            );
        }
        SketchModeRequest::Exit => {
            if world.remove_resource::<SketchGameState>().is_some() {
                info!("[home-action] the sketch screen exits: game state 9 -> 1");
            }
        }
    }
}

/// Exclusive, Update: requests, the running sequence's phase and its clip
/// events.
pub(crate) fn advance(world: &mut World) {
    world.resource_scope(|world, mut actions: Mut<HomeActions>| {
        let actions = &mut *actions;
        for request in std::mem::take(&mut actions.sketch) {
            sketch_mode(world, request);
        }
        let skips = std::mem::take(&mut actions.skips);
        for request in std::mem::take(&mut actions.inbox) {
            start(world, actions, request);
        }
        let Some(mut sequence) = actions.sequence.take() else {
            return;
        };
        let epoch = world.get_resource::<crate::site::GroundEpoch>().map_or(0, |epoch| epoch.0);
        if epoch != sequence.epoch {
            cancel(world, actions, sequence, "the site changed");
            return;
        }
        let dt = world.resource::<Time>().delta_secs();
        let fresh = std::mem::replace(&mut sequence.fresh, false);
        let delay_done = match &mut sequence.phase {
            Phase::Wait(delay) | Phase::Hold(delay) | Phase::ClearDelay(delay) => {
                !fresh && delay.advance(dt)
            }
        };
        let mut done = false;
        match sequence.phase {
            Phase::Wait(_) => {
                if delay_done {
                    begin_motion(world, &mut sequence);
                    // Created in this frame after this frame's check: it
                    // counts from the next frame's delta.
                    sequence.phase = Phase::Hold(Delay::new(sequence.action.hold()));
                    // The clip's events at its start time fire in the frame
                    // it starts.
                    dispatch_events(world, actions, &mut sequence);
                }
            }
            Phase::Hold(_) => {
                if skips > 0 && !sequence.skipped {
                    sequence.skipped = true;
                    let t = elapsed(world, &sequence);
                    info!("[home-action] t={t:.3} skip button pressed");
                }
                dispatch_events(world, actions, &mut sequence);
                if delay_done || sequence.skipped {
                    done = end(world, actions, &mut sequence);
                }
            }
            Phase::ClearDelay(_) => {
                if delay_done {
                    items(world, AvatarItemRequest::Clear);
                    let t = elapsed(world, &sequence);
                    info!("[home-action] t={t:.3} ClearAvatarItem after {SKETCH_CLEAR_DELAY}s");
                    done = true;
                }
            }
        }
        let now = world.resource::<Time>().elapsed_secs();
        if now - sequence.last_probe >= PROBE_INTERVAL {
            sequence.last_probe = now;
            probe(world, &sequence, "tick");
        }
        if done {
            let t = elapsed(world, &sequence);
            info!("[home-action] {:?} done at t={t:.3}", sequence.action);
            actions.finished += 1;
        } else {
            actions.sequence = Some(sequence);
        }
    });
}

/// The sketch target blueprint the instruments name
/// (`MOLY_HOME_ACTION_SKETCH_BLUEPRINT`; game mode reads none).
fn sketch_blueprint_instrument() -> Option<i32> {
    crate::server::client::instrument_env("MOLY_HOME_ACTION_SKETCH_BLUEPRINT")
        .and_then(|raw| raw.trim().parse::<i32>().ok())
}

/// Update: the stand-in keys (J craft, K canvas, L sketch mode then sketch,
/// U skip, I leave sketch mode).
#[allow(clippy::too_many_arguments)]
pub(crate) fn stand_in(
    keys: Res<ButtonInput<KeyCode>>,
    actions: Res<HomeActions>,
    sketch_state: Option<Res<SketchGameState>>,
    players: Query<&Transform, With<PlayerControlled>>,
    fixtures: Query<(Entity, &FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>,
    mut requests: MessageWriter<HomeActionRequest>,
    mut skips: MessageWriter<HomeActionSkip>,
    mut sketch: MessageWriter<SketchModeRequest>,
) {
    if keys.just_pressed(KeyCode::KeyU) {
        skips.write(HomeActionSkip);
        info!("[home-action] stand-in U: skip");
    }
    if keys.just_pressed(KeyCode::KeyI) {
        sketch.write(SketchModeRequest::Exit);
        info!("[home-action] stand-in I: leave sketch mode");
    }
    if keys.just_pressed(KeyCode::KeyL) {
        if sketch_state.is_some() {
            requests.write(HomeActionRequest::Sketch {
                blueprint: sketch_blueprint_instrument(),
            });
            info!("[home-action] stand-in L: sketch");
        } else {
            sketch.write(SketchModeRequest::Enter);
            info!("[home-action] stand-in L: enter sketch mode");
        }
    }
    let craft = keys.just_pressed(KeyCode::KeyJ);
    let canvas = keys.just_pressed(KeyCode::KeyK);
    if !(craft || canvas) {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let Some(fixture) = nearest_craft_tool(&actions, player.translation, &fixtures) else {
        warn!("[home-action] stand-in: no craft tool on this site");
        return;
    };
    let request = if craft {
        HomeActionRequest::Craft { fixture }
    } else {
        HomeActionRequest::Draw { fixture }
    };
    info!("[home-action] stand-in: {request:?}");
    requests.write(request);
}

fn nearest_craft_tool(
    actions: &HomeActions,
    from: Vec3,
    fixtures: &Query<(Entity, &FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>,
) -> Option<FixtureTarget> {
    fixtures
        .iter()
        .filter(|(_, identity, _)| actions.is_craft_tool(identity.master_id))
        .min_by(|a, b| {
            a.2.translation()
                .distance_squared(from)
                .total_cmp(&b.2.translation().distance_squared(from))
        })
        .map(|(entity, identity, _)| FixtureTarget {
            entity,
            uid: identity.uid.clone(),
        })
}

#[derive(Default)]
pub(crate) struct Autoplay {
    warned: bool,
    since: Option<f32>,
    pressing: bool,
    arrived_at: Option<f32>,
    sent: bool,
    sent_at: Option<f32>,
    exit_sent: bool,
    skip_sent: bool,
    last_log: f32,
    progress: Option<(f32, f32)>,
}

/// Instrument (`MOLY_HOME_ACTION_AUTOPLAY=craft|canvas|sketch`, off by
/// default; `MOLY_HOME_ACTION_AUTOPLAY_AFTER` seconds after the joystick
/// opens, default 2; `MOLY_HOME_ACTION_AUTOPLAY_SKIP` seconds into the
/// sequence presses skip): walk to the nearest craft tool through the
/// joystick's touch stream and run one craft or canvas there, or enter sketch
/// mode and sketch. It stands in for the screens' request.
#[allow(clippy::too_many_arguments)]
pub(crate) fn autoplay(
    time: Res<Time>,
    actions: Res<HomeActions>,
    sketch_state: Option<Res<SketchGameState>>,
    joystick: Res<crate::joystick::JoystickState>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    players: Query<&Transform, With<PlayerControlled>>,
    fixtures: Query<(Entity, &FixtureActivityIdentity, &GlobalTransform), With<FixtureRoot>>,
    mut touches: MessageWriter<TouchInput>,
    mut requests: MessageWriter<HomeActionRequest>,
    mut skips: MessageWriter<HomeActionSkip>,
    mut sketch: MessageWriter<SketchModeRequest>,
    mut run: Local<Autoplay>,
) {
    let Ok(mode) = std::env::var("MOLY_HOME_ACTION_AUTOPLAY") else {
        return;
    };
    let mode = mode.trim().to_ascii_lowercase();
    if !run.warned {
        run.warned = true;
        warn!("[home-action] instrument MOLY_HOME_ACTION_AUTOPLAY={mode} is on: the player walks and the screens' request is sent by the instrument");
    }
    if !matches!(mode.as_str(), "craft" | "canvas" | "sketch") {
        return;
    }
    let now = time.elapsed_secs();
    const FINGER: u64 = 99021;
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    let base = Vec2::new(window.width() * 0.15, window.height() * 0.75);
    let mut touch = |phase: TouchPhase, position: Vec2| {
        touches.write(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: FINGER,
        });
    };
    // The sequence was sent: skip on request, then (sketch) leave the mode.
    if let Some(sent_at) = run.sent_at {
        let skip_at = std::env::var("MOLY_HOME_ACTION_AUTOPLAY_SKIP")
            .ok()
            .and_then(|raw| raw.trim().parse::<f32>().ok());
        if let Some(skip_at) = skip_at {
            if !run.skip_sent && actions.playing() && now - sent_at >= skip_at {
                run.skip_sent = true;
                skips.write(HomeActionSkip);
                info!("[home-action] instrument: skip at {:.3}s after the request", now - sent_at);
            }
        }
        if mode == "sketch" && !run.exit_sent && !actions.playing() && actions.finished > 0 && now - sent_at > 1.0 {
            run.exit_sent = true;
            sketch.write(SketchModeRequest::Exit);
            info!("[home-action] instrument: the sketch screen exits");
        }
        return;
    }
    if !joystick.enabled {
        return;
    }
    let after = std::env::var("MOLY_HOME_ACTION_AUTOPLAY_AFTER")
        .ok()
        .and_then(|raw| raw.trim().parse::<f32>().ok())
        .unwrap_or(2.0);
    let since = *run.since.get_or_insert(now);
    if now - since < after {
        return;
    }
    if mode == "sketch" {
        if sketch_state.is_none() && !run.sent {
            if run.arrived_at.is_none() {
                run.arrived_at = Some(now);
                sketch.write(SketchModeRequest::Enter);
                info!("[home-action] instrument: the menu's sketch button");
            }
            return;
        }
        if run.arrived_at.is_some_and(|at| now - at >= 1.0) {
            run.sent = true;
            run.sent_at = Some(now);
            requests.write(HomeActionRequest::Sketch {
                blueprint: sketch_blueprint_instrument(),
            });
            info!("[home-action] instrument: the sketch screen's request");
        }
        return;
    }
    let (Ok(player), Ok(camera)) = (players.single(), cameras.single()) else {
        return;
    };
    let Some(target) = nearest_craft_tool(&actions, player.translation, &fixtures) else {
        if now - run.last_log >= 2.0 {
            run.last_log = now;
            warn!("[home-action] instrument: no craft tool on this site yet");
        }
        return;
    };
    let Ok((_, _, at)) = fixtures.get(target.entity) else {
        return;
    };
    let delta = at.translation() - player.translation;
    let distance = Vec2::new(delta.x, delta.z).length();
    if let Some(arrived) = run.arrived_at {
        if run.pressing {
            touch(TouchPhase::Ended, base);
            run.pressing = false;
        }
        if now - arrived >= 0.5 {
            run.sent = true;
            run.sent_at = Some(now);
            let request = if mode == "craft" {
                HomeActionRequest::Craft { fixture: target }
            } else {
                HomeActionRequest::Draw { fixture: target }
            };
            info!("[home-action] instrument: the screen's request {request:?} at distance {distance:.2} m");
            requests.write(request);
        }
        return;
    }
    if now - run.last_log >= 1.0 {
        run.last_log = now;
        info!(
            "[home-action] instrument: walking to craft tool {} at ({:.2},{:.2},{:.2}); player ({:.2},{:.2},{:.2}), distance {distance:.2} m",
            target.uid,
            at.translation().x,
            at.translation().y,
            at.translation().z,
            player.translation.x,
            player.translation.y,
            player.translation.z
        );
    }
    // Stop within 1 m, or where the walk makes no progress for a second (the
    // fixture's box stops the player).
    let stalled = match run.progress {
        Some((best, _)) if distance < best - 0.02 => {
            run.progress = Some((distance, now));
            false
        }
        Some((_, at_time)) => now - at_time > 1.0,
        None => {
            run.progress = Some((distance, now));
            false
        }
    };
    if distance < 1.0 || stalled || now - since - after > 30.0 {
        run.arrived_at = Some(now);
        info!("[home-action] instrument: stopped at distance {distance:.2} m ({})", if distance < 1.0 { "within 1 m" } else if stalled { "no progress" } else { "30 s" });
        return;
    }
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let dir_world = Vec2::new(delta.x, delta.z).normalize_or(Vec2::X);
    let forward = camera.forward();
    let forward_flat = Vec2::new(forward.x, forward.z).normalize_or_zero();
    let right = camera.right();
    let joy = Vec2::new(
        dir_world.dot(Vec2::new(right.x, right.z)),
        dir_world.dot(forward_flat),
    );
    let scale = root_canvas.scale(window);
    let position = base + Vec2::new(joy.x, -joy.y) * crate::joystick::HANDLE_SIZE * scale;
    if !run.pressing {
        touch(TouchPhase::Started, base);
        run.pressing = true;
        return;
    }
    touch(TouchPhase::Moved, position);
}

/// Registration.
pub(crate) fn install(app: &mut App) {
    use crate::player_avatar::avatar_item;
    app.init_resource::<HomeActions>()
        .init_resource::<HomeActionPresentation>()
        .init_resource::<AvatarItemView>()
        .add_message::<HomeActionRequest>()
        .add_message::<HomeActionSkip>()
        .add_message::<SketchModeRequest>()
        .add_message::<AvatarItemRequest>()
        .add_systems(Startup, (load, avatar_item::load))
        .add_systems(
            Update,
            (
                parse,
                avatar_item::parse,
                crate::zoom_player_camera::capture_snapshot,
            ),
        )
        .add_systems(
            Update,
            (
                (
                    stand_in.run_if(crate::game_settings::scene_input_enabled),
                    autoplay,
                ),
                collect,
                advance,
                avatar_item::apply,
            )
                .chain()
                .in_set(HomeActionSet)
                .after(crate::player_state::drive_from_input)
                .after(crate::harvest::HarvestActionSet)
                .before(crate::player::advance)
                .before(crate::player_avatar::drive)
                .before(crate::audio::advance_se),
        );
}
