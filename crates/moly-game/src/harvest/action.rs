//! The harvest action: target selection, the press, `PlayHarvestAction`
//! and the bookkeeping of each hit.
//!
//! Target (`HarvestPresenter.OnUpdate`): while collisions update, the
//! objects whose radius contains the player (3D distance) are in contact;
//! the target is the one with the lowest `|d|^2 * angle(forward, d)`. A new
//! target runs `OnChangeTargetHarvestObject`: the tool auto-choice, the tool
//! model, `UpdateHarvestUI`. The last contact leaving hides the harvest UI
//! and the tool model.
//!
//! Press (`OnHarvestAction`): during the cool-down it only queues a
//! continuation (`_isSustain`); otherwise `PlayHarvestAction` runs:
//! 1. no target / harvested target: logged, nothing happens;
//! 2. the selected tool;
//! 3. the stamina gate (`CanAttackRemainStamina`; failure: `se_emo_surprise`);
//! 4. tool state: Start for the two sustainable tools, Loop for other
//!    tools, None without a tool;
//! 5. GameState Harvest, player state 7, the intercept gate closed, the end
//!    motion's cancellation source re-created (which cancels a running End
//!    of an earlier press: that press ends there, its steps 10-11 never run);
//! 6. Start: the Start swing and its cool-down;
//! 7. speed (`GetAnimationSpeed`), `IsAutoChangedTool = false`;
//! 8. loop: facing (0.2 s, Linear), the swing clip (clip clock at 1.0 then
//!    set to the speed), `OnPlayerActionStart`, the boost effect, and the
//!    cool-down `Delay(GetHarvestActionTime / speed)`; then
//!    `CanContinueAction`;
//! 9. intercept open; with a tool, the End clip and
//!    `WhenAny(Delay(End length), WaitWhile(state == 7))`;
//! 10. the target re-read: alive -> `UpdateHarvestUI`; else contacts
//!     cleared, UI hidden;
//! 11. player Idle, GameState Normal, `_isSustain = false`, speed 1.0, the
//!     tool model hidden if `CheckNeedHideTool`.
//!
//! The hit clock is the source clip's own: its AnimationEvents at their
//! clip times, the clip time advancing by the frame time times the animator
//! speed (the frame that starts a clip counts: the product's tween
//! convention; the animator's own first-frame timing is not read). The SD
//! body plays the stand-in table (`stand_in`), which never drives the clock.
//!
//! Named gaps: treasure-box auto-move (types 3 and 4 are not placed by the
//! mock); the tone camera; the stamina gauge, HUD and AISAC; the tool
//! selector; `EnableMysekaiHarvestButton` (menu, mission, site map and
//! selected-tool views are not disabled during the action).

use std::collections::HashMap;
use std::time::Duration;

use bevy::animation::graph::AnimationGraph;
use bevy::diagnostic::FrameCount;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::catalog::{HarvestCatalog, ToolDef};
use super::clips::{ClipEvent, HarvestClips};
use super::law::{
    self, animation_speed, can_attack_remain_stamina, can_continue_action, harvest_action_time, harvest_clip_name,
    has_stamina_available_for_harvest, has_tool_required_for_harvest, inside_circle, is_sustainable_tool, target_priority,
    tool_type_for, ContinueInputs, PointTween, SegmentEase, Stamina, ToolClock, ToolState, ToolType,
};
use super::queue::{HarvestLogQueue, HarvestStack, Stack};
use super::server_mock::UserTool;
use super::stand_in::{stand_in, StandIn};
use super::tool_model::{ToolModelRequest, ToolModelRequests};
use super::ui::HarvestButton;
use super::{EffectHook, HarvestEffectHooks, HarvestHit, HarvestHitResults, HarvestHits, HarvestObject, STATUS_HARVESTED};
use crate::audio::SeRequests;
use crate::player::{PlayerControlled, PlayerInput};
use crate::player_avatar::{AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, PlayerVisualClips};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};

/// `PlayerAvatarView.PlayAnimation(name, 0.25, 1.0)` and the Idle state's
/// crossfade.
const CROSSFADE: Duration = Duration::from_millis(250);
/// `RotatePlayerForTargetObject`: `DORotate(..., 0.2, Linear)`.
const FACING_DURATION: f32 = 0.2;
/// `ShakeHarvestCamera`: range by tool, then duration 0.2, vibrato 3.
const SHAKE_DURATION: f32 = 0.2;
const SHAKE_VIBRATO: i32 = 3;
const SHAKE_RANDOMNESS: f32 = 90.0;

/// `HarvestPlayerModel`: stamina, the user tools and the tool choice.
#[derive(Resource, Debug)]
pub(crate) struct HarvestPlayerModel {
    pub(crate) stamina: Stamina,
    pub(crate) tools: Vec<UserTool>,
    /// `UsedTools`: the last tool used per tool type.
    used: HashMap<ToolType, i64>,
    /// The selected tool (`SelectToolDataModel`).
    pub(crate) selected: Option<i64>,
    /// `IsAutoChangedTool`.
    pub(crate) auto_changed: bool,
}

impl HarvestPlayerModel {
    pub(crate) fn from_login(stamina: Stamina, tools: &[UserTool]) -> Self {
        Self {
            stamina,
            tools: tools.to_vec(),
            used: HashMap::new(),
            selected: None,
            auto_changed: false,
        }
    }

    fn tool(&self, id: i64) -> Option<&UserTool> {
        self.tools.iter().find(|tool| tool.tool_id == id)
    }

    /// `SetAppropriateToolData(fixtureType)`: the last used tool of the
    /// type while its quantity is at least one, else the first tool of the
    /// type with a quantity above zero (user list order), else none; stored
    /// back as the used tool of the type.
    fn set_appropriate_tool(&mut self, catalog: &HarvestCatalog, fixture_type: i32) -> Option<i64> {
        let Some(tool_type) = tool_type_for(fixture_type) else {
            self.selected = None;
            return None;
        };
        let last = self
            .used
            .get(&tool_type)
            .copied()
            .filter(|id| self.tool(*id).is_some_and(|tool| tool.quantity >= 1));
        let choice = last.or_else(|| {
            self.tools
                .iter()
                .filter(|tool| tool.quantity > 0)
                .find(|tool| catalog.tool(tool.tool_id).is_some_and(|def| def.tool_type == tool_type))
                .map(|tool| tool.tool_id)
        });
        match choice {
            Some(id) => {
                self.used.insert(tool_type, id);
            }
            None => {
                self.used.remove(&tool_type);
            }
        }
        self.selected = choice;
        choice
    }

    /// Event 30: the tool list from the server; the choice survives while
    /// its tool does.
    pub(crate) fn refresh_tools(&mut self, tools: &[UserTool]) {
        self.tools = tools.to_vec();
        if self.selected.is_some_and(|id| self.tool(id).is_none()) {
            self.selected = None;
        }
    }
}

/// `_collisionObjects` and the target.
#[derive(Resource, Default)]
pub(crate) struct HarvestTargeting {
    contacts: Vec<Entity>,
    pub(crate) target: Option<Entity>,
}

enum Phase {
    /// Step 6: the Start swing's cool-down.
    Start { cooldown: crate::site_move::timeline::Delay },
    /// Step 8: one loop iteration's cool-down.
    Loop { cooldown: crate::site_move::timeline::Delay },
    /// Step 9: the End motion.
    End { wait: crate::site_move::timeline::Delay },
}

/// The source clip clock of the current `PlayHarvestMotion`.
struct Swing {
    clip: String,
    time: f32,
    length: f32,
    looping: bool,
    events: Vec<(f32, ClipEvent)>,
    fired: usize,
    started: f64,
}

struct Facing {
    from: f32,
    to: f32,
    clock: crate::site_move::timeline::TweenClock,
}

struct Action {
    fixture_type: i32,
    tool: Option<i64>,
    state: ToolState,
    speed: f32,
    phase: Phase,
    swing: Option<Swing>,
    facing: Option<Facing>,
    token: Option<PlayerActionToken>,
    pressed_at: f64,
    swings: u32,
}

/// One pending hit's timing, for the per-swing line.
#[derive(Clone, Copy)]
struct HitTiming {
    since_press: f64,
    event_time: f32,
    swing: u32,
}

/// The presenter's action state.
#[derive(Resource)]
pub(crate) struct HarvestAction {
    current: Option<Action>,
    /// `ScreenLayerMysekaiHarvest._isCoolDown`.
    cooling: bool,
    /// `_isSustain`.
    sustain: bool,
    /// The animator speed (`SetAnimationSpeed`; 1.0 after step 11).
    animator_speed: f32,
    hit_timing: Vec<HitTiming>,
    pub(crate) actions: u32,
}

impl Default for HarvestAction {
    fn default() -> Self {
        Self {
            current: None,
            cooling: false,
            sustain: false,
            animator_speed: 1.0,
            hit_timing: Vec::new(),
            actions: 0,
        }
    }
}

impl HarvestAction {
    pub(crate) fn in_progress(&self) -> bool {
        self.current.is_some()
    }
}

/// GameState Harvest (4) while an action runs.
#[derive(Resource)]
pub(crate) struct HarvestGameState;

/// Running camera shakes (`FieldCamera.ShakeCamera` on the camera offset).
#[derive(Resource, Default)]
pub(crate) struct HarvestCameraShakes {
    running: Vec<PointTween>,
    rng: u64,
}

fn tool_clock(def: &ToolDef) -> ToolClock {
    ToolClock {
        tool_type: def.tool_type,
        level: def.level,
        cool_time: def.cool_time,
    }
}

/// `UpdateHarvestUI(model)`.
fn update_harvest_ui(
    button: &mut HarvestButton,
    model: &HarvestPlayerModel,
    catalog: &HarvestCatalog,
    object: &HarvestObject,
) {
    if object.status == STATUS_HARVESTED {
        button.hide();
        return;
    }
    let tool = model.selected.and_then(|id| Some((catalog.tool(id)?, model.tool(id)?)));
    let has_tool = has_tool_required_for_harvest(
        tool.map(|(def, user)| (def.tool_type, user.durability)),
        object.fixture_type,
    );
    let has_stamina = has_stamina_available_for_harvest(
        model.stamina,
        object.fixture_type,
        object.hp,
        object.status == STATUS_HARVESTED,
        object.last_attack_stamina,
        tool.map(|(def, _)| def.attack_power),
    );
    button.shown = true;
    button.interactable = has_tool;
    button.cover = !has_tool || !has_stamina;
    button.cannot = !has_tool;
}

/// `ShowToolModel(tool, type)`: a tool with a quantity and a wood or
/// mineral target.
fn show_tool_model(
    requests: &mut ToolModelRequests,
    model: &HarvestPlayerModel,
    catalog: &HarvestCatalog,
    tool: Option<i64>,
    fixture_type: i32,
) {
    let Some(id) = tool else {
        return;
    };
    let quantity = model.tool(id).map_or(0, |tool| tool.quantity);
    if quantity == 0 || fixture_type >= 2 {
        return;
    }
    if let Some(def) = catalog.tool(id) {
        requests.0.push(ToolModelRequest::Show {
            tool_id: id,
            assetbundle: def.assetbundle.clone(),
        });
    }
}

#[derive(SystemParam)]
pub(crate) struct Body<'w, 's> {
    drivers: Query<'w, 's, (&'static mut AvatarDriver, &'static PlayerVisualClips), With<PlayerControlled>>,
    animators: Query<'w, 's, (&'static mut AnimationPlayer, &'static mut AnimationTransitions)>,
    graphs: ResMut<'w, Assets<AnimationGraph>>,
    clips: Res<'w, Assets<AnimationClip>>,
}

impl Body<'_, '_> {
    /// Play the stand-in of one source clip on the SD body, starting the
    /// harvest lease on the first call.
    fn play(&mut self, token: &mut Option<PlayerActionToken>, source: &str, state: ToolState, source_length: f32, speed: f32, blocks: bool) {
        let Ok((mut driver, visual)) = self.drivers.single_mut() else {
            return;
        };
        let family = visual.idle.get(4..6).unwrap_or("").to_owned();
        let clip = match stand_in(source, state, &family) {
            Ok(StandIn::Clip { name }) => name,
            Ok(StandIn::Idle) => format!("{}_L", visual.idle),
            Err(reason) => {
                warn!("[harvest] SD stand-in refused for {source}: {reason}; the SD body keeps its current motion");
                return;
            }
        };
        let looping = clip == format!("{}_L", visual.idle);
        let rate = match driver.sd_clip_duration(&clip, &self.clips) {
            Some(sd) if source_length > 0.0 && !looping => sd / source_length,
            _ => 1.0,
        };
        let Ok((mut animator, mut transitions)) = self.animators.get_mut(driver.player) else {
            return;
        };
        let motion = PlayerActionMotion {
            clip: &clip,
            looping,
            speed: rate * speed,
            blend: CROSSFADE,
            blocks_manual_movement: blocks,
        };
        let result = match *token {
            Some(live) => driver
                .play_owned_action(live, motion, &mut self.graphs, &mut animator, &mut transitions)
                .map(|_| live),
            None => driver.start_action(PlayerActionOwner::Harvest, motion, &mut self.graphs, &mut animator, &mut transitions),
        };
        match result {
            Ok(live) => {
                *token = Some(live);
                info!(
                    "[harvest] SD stand-in {clip} for {source} at {:.3} (sd/source length {:.3} x speed {:.2}){}",
                    rate * speed,
                    rate,
                    speed,
                    if blocks { "" } else { ", movement open" }
                );
            }
            Err(error) => warn!("[harvest] SD stand-in {clip} for {source} not played: {error:?}"),
        }
    }

    fn idle(&mut self, token: Option<PlayerActionToken>) {
        let Ok((mut driver, _)) = self.drivers.single_mut() else {
            return;
        };
        let Ok((mut animator, mut transitions)) = self.animators.get_mut(driver.player) else {
            return;
        };
        driver.play_idle(token, CROSSFADE, &mut animator, &mut transitions);
    }
}

#[derive(SystemParam)]
pub(crate) struct ActionWorld<'w, 's> {
    frames: Res<'w, FrameCount>,
    time: Res<'w, Time>,
    configs: Option<Res<'w, crate::client_config::ClientConfigs>>,
    catalog: Option<Res<'w, HarvestCatalog>>,
    clips: Option<Res<'w, HarvestClips>>,
    model: Option<ResMut<'w, HarvestPlayerModel>>,
    states: ResMut<'w, PlayerAvatarStates>,
    button: ResMut<'w, HarvestButton>,
    targeting: ResMut<'w, HarvestTargeting>,
    tool_models: ResMut<'w, ToolModelRequests>,
    hits: ResMut<'w, HarvestHits>,
    se: ResMut<'w, SeRequests>,
    effects: ResMut<'w, HarvestEffectHooks>,
    shakes: ResMut<'w, HarvestCameraShakes>,
}

/// Update: contacts and the target (`OnCollisionEnter/Exit`, `OnUpdate`,
/// `OnChangeTargetHarvestObject`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_targets(
    eligibility: crate::interaction::InteractionEligibility,
    players: Query<&Transform, (With<PlayerControlled>, Without<HarvestObject>)>,
    objects: Query<(Entity, &Transform, &HarvestObject)>,
    catalog: Option<Res<HarvestCatalog>>,
    mut model: Option<ResMut<HarvestPlayerModel>>,
    mut targeting: ResMut<HarvestTargeting>,
    mut button: ResMut<HarvestButton>,
    mut tool_models: ResMut<ToolModelRequests>,
) {
    let (Some(catalog), Some(model)) = (catalog, model.as_deref_mut()) else {
        return;
    };
    if !eligibility.collision_updates() {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let position = player.translation;
    let forward = player.rotation * Vec3::Z;
    let contacts: Vec<Entity> = objects
        .iter()
        .filter(|(_, transform, object)| object.collision && inside_circle(position, transform.translation, object.radius))
        .map(|(entity, _, _)| entity)
        .collect();
    let had = !targeting.contacts.is_empty();
    for entity in &contacts {
        if !targeting.contacts.contains(entity) {
            if let Ok((_, _, object)) = objects.get(*entity) {
                info!("[harvest] OnCollisionEnter {}#{} (radius {:.2})", object.leaf, object.fixture_id, object.radius);
            }
        }
    }
    targeting.contacts = contacts;
    if had && targeting.contacts.is_empty() {
        info!("[harvest] OnCollisionExit of the last object: harvest UI hidden, tool model hidden");
        button.hide();
        tool_models.0.push(ToolModelRequest::Hide);
    }
    if targeting.contacts.is_empty() {
        targeting.target = None;
        return;
    }
    let best = targeting
        .contacts
        .iter()
        .filter_map(|entity| objects.get(*entity).ok())
        .map(|(entity, transform, _)| (entity, target_priority(position, forward, transform.translation)))
        .min_by(|a, b| a.1.partial_cmp(&b.1).expect("priority is finite"))
        .map(|(entity, _)| entity);
    if best == targeting.target {
        return;
    }
    targeting.target = best;
    let Some((_, _, object)) = best.and_then(|entity| objects.get(entity).ok()) else {
        return;
    };
    let tool = model.set_appropriate_tool(&catalog, object.fixture_type);
    show_tool_model(&mut tool_models, model, &catalog, tool, object.fixture_type);
    update_harvest_ui(&mut button, model, &catalog, object);
    info!(
        "[harvest] OnChangeTargetHarvestObject {}#{} ({}) at ({}, {}): tool {:?}; button interactable {} cover {}",
        object.leaf, object.fixture_id, object.class, object.position_x, object.position_z, tool, button.interactable, button.cover
    );
}

fn yaw_of(rotation: Quat) -> f32 {
    rotation.to_euler(EulerRot::YXZ).0
}

/// Update: presses, the swing clock and the phases.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut action: ResMut<HarvestAction>,
    mut world: ActionWorld,
    mut body: Body,
    mut commands: Commands,
    mut players: Query<&mut Transform, (With<PlayerControlled>, Without<HarvestObject>)>,
    objects: Query<(&Transform, &HarvestObject)>,
) {
    let (Some(configs), Some(catalog), Some(clips), Some(mut model)) =
        (world.configs.take(), world.catalog.take(), world.clips.take(), world.model.take())
    else {
        world.button.presses = 0;
        return;
    };
    let frame = world.frames.0;
    let dt = world.time.delta_secs();
    let now = world.time.elapsed_secs_f64();
    let boost_speed = configs.float(crate::client_config::KEY_BOOST_STAMINA_ANIMATION_SPEED);

    // Presses (`OnHarvestAction`).
    for _ in 0..std::mem::take(&mut world.button.presses) {
        if action.cooling {
            action.sustain = true;
            info!("[harvest] press during the cool-down: queued as a continuation (_isSustain)");
            continue;
        }
        let Some(admitted) = admit(&mut world, &model, &catalog, &objects) else {
            continue;
        };
        // Step 5 re-creates the end motion's cancellation source: a running
        // End of an earlier press is cancelled there and that press ends
        // without its steps 10-11. The SD lease carries over.
        let carried = action.current.take().and_then(|previous| {
            info!("[harvest] press during the End motion: the running action is cancelled (its end steps do not run)");
            previous.token
        });
        start_action(
            &mut action, &mut world, &mut body, &mut model, &catalog, &clips, &mut commands, &mut players, &objects,
            boost_speed, frame, now, carried, admitted,
        );
    }

    let Some(mut current) = action.current.take() else {
        world.model = Some(model);
        return;
    };

    // Facing tween (fire and forget).
    if let Some(facing) = current.facing.as_mut() {
        let t = facing.clock.advance(dt);
        if let Ok(mut transform) = players.single_mut() {
            transform.rotation = Quat::from_rotation_y(law::fast_yaw(facing.from, facing.to, t));
        }
        if facing.clock.done() {
            current.facing = None;
        }
    }

    // The swing clip's AnimationEvents.
    if let Some(swing) = current.swing.as_mut() {
        swing.time += dt * action.animator_speed;
        let mut due: Vec<(f32, ClipEvent)> = Vec::new();
        loop {
            while let Some(&(time, kind)) = swing.events.get(swing.fired) {
                if time > swing.time {
                    break;
                }
                due.push((time, kind));
                swing.fired += 1;
            }
            if swing.looping && swing.length > 0.0 && swing.time >= swing.length {
                swing.time -= swing.length;
                swing.fired = 0;
                continue;
            }
            break;
        }
        let clip = swing.clip.clone();
        for (event_time, kind) in due {
            on_animation_event(&mut action, &mut world, &model, &catalog, &objects, &current, kind, event_time, &clip);
        }
    }

    // Phases.
    let mut finish = false;
    match &mut current.phase {
        Phase::Start { cooldown } => {
            if cooldown.tick(frame, dt) {
                action.cooling = false;
                current.state = ToolState::Loop;
                begin_loop(&mut action, &mut world, &mut body, &mut model, &catalog, &clips, &mut players, &objects, &mut current, boost_speed, frame, now);
            }
        }
        Phase::Loop { cooldown } => {
            if cooldown.tick(frame, dt) {
                action.cooling = false;
                let target = world.targeting.target.and_then(|entity| objects.get(entity).ok()).map(|(_, object)| object);
                let tool = current.tool.and_then(|id| catalog.tool(id));
                let inputs = ContinueInputs {
                    long_tap: world.button.is_press,
                    sustain: action.sustain,
                    has_target: target.is_some(),
                    target_harvested: target.is_some_and(|object| object.status == STATUS_HARVESTED),
                    auto_changed_tool: model.auto_changed,
                    tool_quantity: current.tool.map(|id| model.tool(id).map_or(0, |tool| tool.quantity)),
                    stamina_empty: model.stamina.is_empty(),
                    can_attack: target.is_some_and(|object| {
                        can_attack_remain_stamina(
                            model.stamina,
                            tool.map(|def| def.attack_power),
                            object.is_last_attack,
                            object.hp,
                            object.last_attack_stamina,
                        )
                    }),
                };
                let (again, sustain) = can_continue_action(inputs);
                action.sustain = sustain;
                info!(
                    "[harvest] cool-down done at {:.3} s since the press (swing {}): CanContinueAction {again} (held {} queued {} target {} harvested {} stamina {:?})",
                    now - current.pressed_at,
                    current.swings,
                    inputs.long_tap,
                    inputs.sustain,
                    inputs.has_target,
                    inputs.target_harvested,
                    model.stamina
                );
                if again {
                    begin_loop(&mut action, &mut world, &mut body, &mut model, &catalog, &clips, &mut players, &objects, &mut current, boost_speed, frame, now);
                } else {
                    // Step 9.
                    world.states.can_intercept = true;
                    if current.state != ToolState::None {
                        current.state = ToolState::End;
                        let tool = current.tool.and_then(|id| catalog.tool(id)).map(tool_clock);
                        let name = harvest_clip_name(current.fixture_type, tool, ToolState::End);
                        let length = name.as_deref().and_then(|name| clips.get(name)).map_or(0.0, |clip| clip.length);
                        let wait = harvest_action_time(tool, ToolState::End, length);
                        if let Some(name) = name.as_deref() {
                            start_swing(&mut current, &clips, name, now);
                            body.play(&mut current.token, name, ToolState::End, length, action.animator_speed, false);
                        }
                        world.states.change_status(PlayerActionState::Harvest);
                        info!("[harvest] End motion {:?}: wait {wait:.4} s (the End clip length), movement open", name);
                        current.phase = Phase::End {
                            wait: crate::site_move::timeline::Delay::new(law::delay_seconds(wait as f64), frame),
                        };
                    } else {
                        finish = true;
                    }
                }
            }
        }
        Phase::End { wait } => {
            let due = wait.tick(frame, dt);
            if world.states.current != PlayerActionState::Harvest {
                info!("[harvest] End motion left early: player state {:?} (WaitWhile state == 7)", world.states.current);
                finish = true;
            } else if due {
                finish = true;
            }
        }
    }
    if finish {
        finish_action(&mut action, &mut world, &mut body, &mut model, &catalog, &mut commands, &objects, current, now);
    } else {
        action.current = Some(current);
    }
    world.model = Some(model);
}

fn start_swing(current: &mut Action, clips: &HarvestClips, name: &str, now: f64) {
    let record = clips.get(name);
    if record.is_none() {
        warn!("[harvest] source clip {name} has no record in the motion manifest: no events, length 0");
    }
    current.swing = Some(Swing {
        clip: name.to_owned(),
        time: 0.0,
        length: record.map_or(0.0, |clip| clip.length),
        looping: record.is_some_and(|clip| clip.looping),
        events: record.map(|clip| clip.events.clone()).unwrap_or_default(),
        fired: 0,
        started: now,
    });
}

/// What steps 1 to 4 decided for an admitted press.
struct Admitted {
    target: Entity,
    tool: Option<i64>,
    state: ToolState,
}

/// Steps 1 to 4 of `PlayHarvestAction`: a press that ends here changes
/// nothing else (a running End goes on).
fn admit(
    world: &mut ActionWorld,
    model: &HarvestPlayerModel,
    catalog: &HarvestCatalog,
    objects: &Query<(&Transform, &HarvestObject)>,
) -> Option<Admitted> {
    // 1.
    let Some(target) = world.targeting.target else {
        info!("[harvest] PlayHarvestAction: target harvest object not obtainable");
        return None;
    };
    let Ok((_, object)) = objects.get(target) else {
        info!("[harvest] PlayHarvestAction: target model missing");
        return None;
    };
    if object.status == STATUS_HARVESTED {
        info!("[harvest] PlayHarvestAction: target model already destroyed");
        return None;
    }
    // 2.
    let tool = model.selected;
    let def = tool.and_then(|id| catalog.tool(id));
    // 3.
    let can_attack = can_attack_remain_stamina(
        model.stamina,
        def.map(|def| def.attack_power),
        object.is_last_attack,
        object.hp,
        object.last_attack_stamina,
    );
    if model.stamina.is_empty() || !can_attack {
        super::damage::push_se(&mut world.se, "se_emo_surprise", "harvest-stamina");
        info!(
            "[harvest] PlayHarvestAction: LackOfStamina (stamina {:?}, tool power {:?}, hp {}, last-attack stamina {}): se_emo_surprise",
            model.stamina,
            def.map(|def| def.attack_power),
            object.hp,
            object.last_attack_stamina
        );
        return None;
    }
    // 4.
    let state = match tool {
        Some(id) if is_sustainable_tool(id) => ToolState::Start,
        Some(_) => ToolState::Loop,
        None => ToolState::None,
    };
    Some(Admitted { target, tool, state })
}

/// Steps 5 to 8 (the first loop iteration).
#[allow(clippy::too_many_arguments)]
fn start_action(
    action: &mut HarvestAction,
    world: &mut ActionWorld,
    body: &mut Body,
    model: &mut HarvestPlayerModel,
    catalog: &HarvestCatalog,
    clips: &HarvestClips,
    commands: &mut Commands,
    players: &mut Query<&mut Transform, (With<PlayerControlled>, Without<HarvestObject>)>,
    objects: &Query<(&Transform, &HarvestObject)>,
    boost_speed: f32,
    frame: u64,
    now: f64,
    token: Option<PlayerActionToken>,
    admitted: Admitted,
) {
    let Admitted { target, tool, state } = admitted;
    let Ok((_, object)) = objects.get(target) else {
        return;
    };
    let def = tool.and_then(|id| catalog.tool(id));
    // 5.
    commands.insert_resource(HarvestGameState);
    world.states.change_status(PlayerActionState::Harvest);
    world.states.can_intercept = false;
    action.actions += 1;
    info!(
        "[harvest] PlayHarvestAction #{} on {}#{} (hp {}): tool {:?} state {:?}; GameState Harvest, player state {:?}, intercept closed",
        action.actions, object.leaf, object.fixture_id, object.hp, tool, state, world.states.current
    );
    let mut current = Action {
        fixture_type: object.fixture_type,
        tool,
        state,
        speed: 1.0,
        phase: Phase::Loop {
            cooldown: crate::site_move::timeline::Delay::new(f32::INFINITY, frame),
        },
        swing: None,
        facing: None,
        token,
        pressed_at: now,
        swings: 0,
    };
    if state == ToolState::Start {
        // 6: the Start swing at the animator speed PlayAnimation sets (1.0).
        let clock = def.map(tool_clock);
        let name = harvest_clip_name(object.fixture_type, clock, ToolState::Start);
        let length = name.as_deref().and_then(|name| clips.get(name)).map_or(0.0, |clip| clip.length);
        motion(action, world, body, players, objects, &mut current, clips, name.as_deref(), ToolState::Start, length, 1.0, now, target);
        let wait = harvest_action_time(clock, ToolState::Start, length) / current.speed;
        action.cooling = true;
        current.phase = Phase::Start {
            cooldown: crate::site_move::timeline::Delay::new(law::delay_seconds(wait as f64), frame),
        };
        action.current = Some(current);
        return;
    }
    begin_loop(action, world, body, model, catalog, clips, players, objects, &mut current, boost_speed, frame, now);
    action.current = Some(current);
}

/// `PlayHarvestMotion`: facing, the source clip, its SD stand-in.
#[allow(clippy::too_many_arguments)]
fn motion(
    action: &mut HarvestAction,
    world: &mut ActionWorld,
    body: &mut Body,
    players: &mut Query<&mut Transform, (With<PlayerControlled>, Without<HarvestObject>)>,
    objects: &Query<(&Transform, &HarvestObject)>,
    current: &mut Action,
    clips: &HarvestClips,
    name: Option<&str>,
    state: ToolState,
    length: f32,
    speed: f32,
    now: f64,
    target: Entity,
) {
    world.states.change_status(PlayerActionState::Harvest);
    // Facing: the kinds of mask 0x3A7 outside End (tone and boxes are not
    // placed by the mock).
    if state != ToolState::End && matches!(current.fixture_type, 0 | 1 | 2 | 5 | 7 | 8 | 9) {
        if let (Ok(transform), Ok((object_transform, _))) = (players.single(), objects.get(target)) {
            let d = object_transform.translation - transform.translation;
            current.facing = Some(Facing {
                from: yaw_of(transform.rotation),
                to: law::facing_yaw(Vec2::new(d.x, d.z)),
                clock: crate::site_move::timeline::TweenClock::new(FACING_DURATION),
            });
        }
    }
    let Some(name) = name else {
        warn!("[harvest] no source clip for fixture type {} and state {state:?}", current.fixture_type);
        return;
    };
    start_swing(current, clips, name, now);
    // PlayAnimation(name, 0.25, 1.0); a loop iteration sets the speed in the
    // same frame (`SetAnimationSpeed`), so the stand-in starts at it.
    action.animator_speed = speed;
    body.play(&mut current.token, name, state, length, speed, true);
}

/// One loop iteration (step 8).
#[allow(clippy::too_many_arguments)]
fn begin_loop(
    action: &mut HarvestAction,
    world: &mut ActionWorld,
    body: &mut Body,
    model: &mut HarvestPlayerModel,
    catalog: &HarvestCatalog,
    clips: &HarvestClips,
    players: &mut Query<&mut Transform, (With<PlayerControlled>, Without<HarvestObject>)>,
    objects: &Query<(&Transform, &HarvestObject)>,
    current: &mut Action,
    boost_speed: f32,
    frame: u64,
    now: f64,
) {
    if current.swings == 0 {
        // 7.
        current.speed = animation_speed(model.stamina, boost_speed);
        model.auto_changed = false;
    }
    let Some(target) = world.targeting.target else {
        info!("[harvest] loop: no target");
        return;
    };
    let clock = current.tool.and_then(|id| catalog.tool(id)).map(tool_clock);
    let name = harvest_clip_name(current.fixture_type, clock, current.state);
    let length = name.as_deref().and_then(|name| clips.get(name)).map_or(0.0, |clip| clip.length);
    let speed = current.speed;
    motion(action, world, body, players, objects, current, clips, name.as_deref(), current.state, length, speed, now, target);
    current.swings += 1;
    // OnPlayerActionStart(speed).
    if let Ok((_, object)) = objects.get(target) {
        if let Some(cue) = object.cues.swing_start {
            super::damage::push_se(&mut world.se, cue, "harvest-swing");
        }
        if matches!(object.class, "MysekaiAreadDriftageView" | "MysekaiAreaToolBoxView") {
            info!("[harvest] {}#{}: the prop's animator clip (break / open) is not played", object.leaf, object.fixture_id);
        }
    }
    if model.stamina.has_boost_or_enhance() {
        if let Ok(transform) = players.single() {
            world.effects.pending.push(EffectHook { kind: 142, position: transform.translation });
        }
    }
    // PlayAnimationHarvestUI -> ClickHarvestButton.
    let time = harvest_action_time(clock, current.state, length);
    let wait = law::delay_seconds((time / current.speed) as f64);
    action.cooling = true;
    current.phase = Phase::Loop {
        cooldown: crate::site_move::timeline::Delay::new(wait, frame),
    };
    info!(
        "[harvest] swing {} at {:.3} s since the press: clip {:?} (length {length:.4}), speed {:.2}, cool-down {wait:.3} s",
        current.swings,
        now - current.pressed_at,
        name,
        current.speed
    );
}

/// `HarvestPresenter.OnAnimationEvent`.
#[allow(clippy::too_many_arguments)]
fn on_animation_event(
    action: &mut HarvestAction,
    world: &mut ActionWorld,
    model: &HarvestPlayerModel,
    catalog: &HarvestCatalog,
    objects: &Query<(&Transform, &HarvestObject)>,
    current: &Action,
    kind: ClipEvent,
    event_time: f32,
    clip: &str,
) {
    let in_state = world.states.current == PlayerActionState::Harvest;
    let kind = match kind {
        ClipEvent::Hit => ClipEvent::Hit,
        ClipEvent::HitInHarvestState if in_state => ClipEvent::Hit,
        ClipEvent::EffectOnly if in_state => ClipEvent::EffectOnly,
        ClipEvent::PostStartAction => ClipEvent::PostStartAction,
        other => {
            info!("[harvest] {clip} event {other:?} at {event_time:.4} dropped: player state {:?}", world.states.current);
            return;
        }
    };
    let Some(target) = world.targeting.target else {
        world.states.can_intercept = true;
        info!("[harvest] {clip} event {kind:?}: no target; intercept opened");
        return;
    };
    let Ok((transform, object)) = objects.get(target) else {
        return;
    };
    if object.status == STATUS_HARVESTED {
        info!("[harvest] {clip} event {kind:?}: target already harvested, nothing");
        return;
    }
    let def = current.tool.and_then(|id| catalog.tool(id));
    let boost = model.stamina.has_boost_or_enhance();
    match kind {
        ClipEvent::Hit => {
            world.hits.0.push(HarvestHit {
                target,
                damage: def.map_or(1, |def| def.attack_power),
                tool_level: def.map_or(0, |def| def.level),
                is_boost: boost,
                tool: current.tool,
            });
            action.hit_timing.push(HitTiming {
                since_press: world.time.elapsed_secs_f64() - current.pressed_at,
                event_time,
                swing: current.swings,
            });
        }
        ClipEvent::EffectOnly => {
            info!(
                "[harvest] {clip} EffectOnly at {event_time:.4}: PlayDamageEffectForIndefinite on {}#{} (effect-pool step)",
                object.leaf, object.fixture_id
            );
            let _ = transform;
            if let Some(def) = def {
                push_shake(&mut world.shakes, def.id);
            }
        }
        ClipEvent::PostStartAction => {
            info!("[harvest] {clip} PostStartAction at {event_time:.4} on {}#{}", object.leaf, object.fixture_id);
        }
        ClipEvent::HitInHarvestState => unreachable!("mapped above"),
    }
}

/// `ShakeHarvestCamera(tool)`: range 0.015 for the sustainable tools, else
/// 0.03.
fn push_shake(shakes: &mut HarvestCameraShakes, tool_id: i64) {
    let range = if is_sustainable_tool(tool_id) { 0.015 } else { 0.03 };
    let mut rng = super::Rng(0x5348_414B_0000_0000 ^ shakes.rng);
    shakes.rng = shakes.rng.wrapping_add(1);
    let points = law::shake_points(SHAKE_DURATION / SHAKE_VIBRATO as f32, range, SHAKE_VIBRATO, SHAKE_RANDOMNESS, true, |min, max| {
        min + rng.next_f32() * (max - min)
    });
    shakes.running.push(PointTween::new(points, SegmentEase::Linear));
}

/// Steps 10 and 11.
#[allow(clippy::too_many_arguments)]
fn finish_action(
    action: &mut HarvestAction,
    world: &mut ActionWorld,
    body: &mut Body,
    model: &mut HarvestPlayerModel,
    catalog: &HarvestCatalog,
    commands: &mut Commands,
    objects: &Query<(&Transform, &HarvestObject)>,
    current: Action,
    now: f64,
) {
    // 10.
    let target = world.targeting.target.and_then(|entity| objects.get(entity).ok());
    match target {
        Some((_, object)) if object.status != STATUS_HARVESTED => {
            update_harvest_ui(&mut world.button, model, catalog, object);
        }
        _ => {
            world.targeting.contacts.clear();
            world.button.hide();
        }
    }
    // 11.
    world.states.can_intercept = true;
    world.states.change_status(PlayerActionState::Idle);
    commands.remove_resource::<HarvestGameState>();
    action.sustain = false;
    action.animator_speed = 1.0;
    action.cooling = false;
    let need_hide = current.tool.and_then(|id| catalog.tool(id)).is_some()
        && world
            .targeting
            .contacts
            .iter()
            .filter_map(|entity| objects.get(*entity).ok())
            .all(|(_, object)| tool_type_for(object.fixture_type).is_none());
    if need_hide {
        world.tool_models.0.push(ToolModelRequest::Hide);
    }
    body.idle(current.token);
    info!(
        "[harvest] action done at {:.3} s since the press after {} swings: player {:?}, GameState Normal, tool model {}",
        now - current.pressed_at,
        current.swings,
        world.states.current,
        if need_hide { "hidden" } else { "kept" }
    );
}

/// Update: `OnReceiveHarvestAnimationEvent` after `OnDamage`, and
/// `OnHarvestPlayerActionHit` (stamina, durability, the log stack, a broken
/// tool), `UpdateHarvestUI`, the camera shake.
#[allow(clippy::too_many_arguments)]
pub(crate) fn after_hits(
    mut action: ResMut<HarvestAction>,
    mut results: ResMut<HarvestHitResults>,
    catalog: Option<Res<HarvestCatalog>>,
    mut model: Option<ResMut<HarvestPlayerModel>>,
    objects: Query<&HarvestObject>,
    targeting: Res<HarvestTargeting>,
    mut button: ResMut<HarvestButton>,
    mut queue: ResMut<HarvestLogQueue>,
    mut shakes: ResMut<HarvestCameraShakes>,
    mut tool_models: ResMut<ToolModelRequests>,
) {
    let (Some(catalog), Some(model)) = (catalog, model.as_deref_mut()) else {
        results.0.clear();
        return;
    };
    let timings = std::mem::take(&mut action.hit_timing);
    for (index, result) in std::mem::take(&mut results.0).into_iter().enumerate() {
        let Ok(object) = objects.get(result.target) else {
            continue;
        };
        let before = model.stamina;
        let amount = if object.is_last_attack { object.last_attack_stamina } else { result.used };
        model.stamina.decrease(amount);
        let mut tool_log = None;
        if let Some(id) = result.tool {
            let max = catalog.tool(id).map_or(0, |def| def.max_durability);
            match model.tools.iter().position(|tool| tool.tool_id == id) {
                Some(position) => {
                    let tool = &mut model.tools[position];
                    if tool.durability < 1 {
                        error!("[harvest] DecreaseToolDurability: tool {id} durability {} is below one (the source throws)", tool.durability);
                    } else {
                        tool.durability -= 1;
                        let mut broke = false;
                        if tool.durability == 0 {
                            // DecreaseCountAndResetDurability.
                            tool.quantity -= 1;
                            tool.durability = max;
                            broke = true;
                        }
                        tool_log = Some((id, tool.durability, broke));
                        if tool.quantity < 1 {
                            model.tools.remove(position);
                            // TryChangeOtherTool.
                            let tool_type = catalog.tool(id).map(|def| def.tool_type);
                            let other = model
                                .tools
                                .iter()
                                .filter(|tool| tool.quantity > 0)
                                .find(|tool| catalog.tool(tool.tool_id).map(|def| def.tool_type) == tool_type)
                                .map(|tool| tool.tool_id);
                            model.selected = other;
                            if let (Some(other), Some(tool_type)) = (other, tool_type) {
                                model.used.insert(tool_type, other);
                                model.auto_changed = true;
                                show_tool_model(&mut tool_models, model, &catalog, Some(other), object.fixture_type);
                            }
                            info!("[harvest] tool {id} used up: TryChangeOtherTool -> {other:?}");
                        }
                    }
                }
                None => error!("[harvest] DecreaseToolDurability: tool {id} is not in the user tool list"),
            }
        }
        if !queue.destroyed.contains(&object.uid) {
            queue.stacks.push(Stack::Harvest(HarvestStack {
                site_id: object.site_id,
                position_x: object.position_x,
                position_z: object.position_z,
                fixture_id: object.fixture_id,
                hp: object.hp,
                is_last_attack: result.is_last_attack,
                tool: tool_log,
                stamina_after: model.stamina,
            }));
        }
        if result.is_last_attack {
            queue.destroyed.insert(object.uid);
        }
        if targeting.target == Some(result.target) {
            update_harvest_ui(&mut button, model, &catalog, object);
        }
        if let Some(id) = result.tool {
            push_shake(&mut shakes, id);
        }
        let timing = timings.get(index);
        info!(
            "[harvest-swing] {}#{} swing {}: {:.3} s since the press, clip event at {:.4}, damage {} used {} hp {} -> {} last {} stamina {:?} -> {:?} tool {:?}",
            object.leaf,
            object.fixture_id,
            timing.map_or(0, |t| t.swing),
            timing.map_or(f64::NAN, |t| t.since_press),
            timing.map_or(f32::NAN, |t| t.event_time),
            result.damage,
            result.used,
            object.prev_hp,
            object.hp,
            result.is_last_attack,
            before,
            model.stamina,
            tool_log,
        );
    }
}

/// PostUpdate, before the camera follows: the shakes write the camera
/// offset (each relative to the value it found, like DOTween's shake).
pub(crate) fn advance_camera_shake(
    time: Res<Time>,
    mut shakes: ResMut<HarvestCameraShakes>,
    model: Option<ResMut<crate::camera::FieldCameraModel>>,
) {
    let Some(mut model) = model else {
        shakes.running.clear();
        return;
    };
    if shakes.running.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let mut offset = model.offset;
    for tween in &mut shakes.running {
        offset = tween.advance(offset, dt);
    }
    model.offset = offset;
    shakes.running.retain(|tween| !tween.done());
}

/// Instrument (off by default): `MOLY_HARVEST_AUTOPLAY` names a plan of
/// kinds (`wood`, `mineral`, `plant`, `other`, `toolbox`, `driftage`), each
/// optionally `kind:N` for N swings. Per step it walks to the nearest
/// unharvested object of the kind, holds the action button until the object
/// is harvested (or N hits), then walks over the drops that landed within
/// 6 m. It writes the player's input and the button, the same inputs a
/// person gives.
#[derive(Resource, Default)]
pub(crate) struct HarvestAutoplay {
    plan: Option<Vec<(i32, Option<u32>)>>,
    step: usize,
    stage: AutoStage,
    target: Option<Entity>,
    started: f64,
    hits_at_start: usize,
    warned: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
enum AutoStage {
    #[default]
    Seek,
    Walk,
    Press,
    Hold,
    Collect,
    Done,
}

fn kind_type(word: &str) -> Option<i32> {
    Some(match word {
        "wood" => 0,
        "mineral" => 1,
        "plant" => 2,
        "other" => 5,
        "toolbox" => 7,
        "driftage" => 8,
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn autoplay_press(
    time: Res<Time>,
    mut auto: ResMut<HarvestAutoplay>,
    scenes: Option<Res<super::HarvestScenesReady>>,
    entry: Option<Res<crate::entry::EntrySequence>>,
    action: Res<HarvestAction>,
    stats: Res<super::HarvestStats>,
    mut button: ResMut<HarvestButton>,
    targeting: Res<HarvestTargeting>,
    mut players: Query<(&Transform, &mut PlayerInput), With<PlayerControlled>>,
    objects: Query<(Entity, &Transform, &HarvestObject)>,
    drops: Query<(&Transform, &super::HarvestDropItem)>,
) {
    if auto.plan.is_none() {
        let Ok(raw) = std::env::var("MOLY_HARVEST_AUTOPLAY") else {
            auto.plan = Some(Vec::new());
            return;
        };
        let plan: Vec<(i32, Option<u32>)> = raw
            .split(',')
            .filter(|entry| !entry.trim().is_empty())
            .map(|entry| {
                let (word, count) = entry.trim().split_once(':').map_or((entry.trim(), None), |(w, n)| (w, n.parse().ok()));
                (kind_type(word).unwrap_or_else(|| panic!("MOLY_HARVEST_AUTOPLAY: unknown kind {word}")), count)
            })
            .collect();
        warn!("[harvest-auto] MOLY_HARVEST_AUTOPLAY instrument on: plan {plan:?} (walks and presses for the run log; off by default)");
        auto.plan = Some(plan);
    }
    let plan = auto.plan.clone().unwrap_or_default();
    if plan.is_empty() || auto.stage == AutoStage::Done {
        return;
    }
    if scenes.is_none() || !crate::entry::control_open(entry.as_deref()) {
        return;
    }
    let Ok((player, mut input)) = players.single_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let Some(&(fixture_type, count)) = plan.get(auto.step) else {
        if !auto.warned {
            auto.warned = true;
            warn!("[harvest-auto] plan finished");
        }
        auto.stage = AutoStage::Done;
        input.active = false;
        input.direction = Vec3::ZERO;
        return;
    };
    let walk_to = |input: &mut PlayerInput, to: Vec3| {
        let d = Vec3::new(to.x - player.translation.x, 0.0, to.z - player.translation.z);
        if d.length_squared() > 1e-6 {
            input.direction = d.normalize();
            input.active = true;
        }
    };
    match auto.stage {
        AutoStage::Seek => {
            let nearest = objects
                .iter()
                .filter(|(_, _, object)| object.fixture_type == fixture_type && object.status != STATUS_HARVESTED && object.collision)
                .min_by(|a, b| {
                    a.1.translation
                        .distance_squared(player.translation)
                        .partial_cmp(&b.1.translation.distance_squared(player.translation))
                        .expect("finite")
                });
            match nearest {
                Some((entity, transform, object)) => {
                    warn!(
                        "[harvest-auto] step {}: walk to {}#{} at ({:.2}, {:.2}) from ({:.2}, {:.2})",
                        auto.step, object.leaf, object.fixture_id, transform.translation.x, transform.translation.z, player.translation.x, player.translation.z
                    );
                    auto.target = Some(entity);
                    auto.stage = AutoStage::Walk;
                    auto.started = now;
                }
                None => {
                    warn!("[harvest-auto] step {}: no unharvested object of fixture type {fixture_type}; skipped", auto.step);
                    auto.step += 1;
                }
            }
        }
        AutoStage::Walk => {
            let Some((_, transform, _)) = auto.target.and_then(|entity| objects.get(entity).ok()) else {
                auto.stage = AutoStage::Seek;
                return;
            };
            if targeting.target == auto.target && button.shown {
                input.active = false;
                input.direction = Vec3::ZERO;
                auto.stage = AutoStage::Press;
                warn!("[harvest-auto] step {}: in contact and targeted after {:.2} s", auto.step, now - auto.started);
            } else if now - auto.started > 30.0 {
                warn!("[harvest-auto] step {}: could not reach the object in 30 s; skipped", auto.step);
                input.active = false;
                auto.step += 1;
                auto.stage = AutoStage::Seek;
            } else {
                walk_to(&mut input, transform.translation);
            }
        }
        AutoStage::Press => {
            if action.in_progress() {
                return;
            }
            if button.interactable {
                button.presses += 1;
                button.is_press = true;
                auto.hits_at_start = stats.hits;
                auto.stage = AutoStage::Hold;
                auto.started = now;
                warn!("[harvest-auto] step {}: press and hold", auto.step);
            } else {
                warn!("[harvest-auto] step {}: the button is not interactable; skipped", auto.step);
                auto.step += 1;
                auto.stage = AutoStage::Seek;
            }
        }
        AutoStage::Hold => {
            let harvested = auto
                .target
                .and_then(|entity| objects.get(entity).ok())
                .is_none_or(|(_, _, object)| object.status == STATUS_HARVESTED);
            let enough = count.is_some_and(|n| (stats.hits - auto.hits_at_start) as u32 >= n);
            if harvested || enough {
                button.is_press = false;
                if !action.in_progress() {
                    warn!("[harvest-auto] step {}: released after {} hits; collecting drops", auto.step, stats.hits - auto.hits_at_start);
                    auto.stage = AutoStage::Collect;
                    auto.started = now;
                }
            } else {
                button.is_press = true;
                if !action.in_progress() && now - auto.started > 1.0 {
                    // A single-action kind ends each press; press again.
                    auto.stage = AutoStage::Press;
                }
            }
        }
        AutoStage::Collect => {
            let origin = auto.target.and_then(|entity| objects.get(entity).ok()).map(|(_, transform, _)| transform.translation);
            let nearest = drops
                .iter()
                .filter(|(transform, item)| item.radius > 0.0 && origin.is_none_or(|o| transform.translation.distance(o) < 6.0))
                .min_by(|a, b| {
                    a.0.translation
                        .distance_squared(player.translation)
                        .partial_cmp(&b.0.translation.distance_squared(player.translation))
                        .expect("finite")
                });
            let pending = drops.iter().any(|(_, item)| item.radius == 0.0);
            match nearest {
                Some((transform, _)) if now - auto.started < 20.0 => walk_to(&mut input, transform.translation),
                _ if pending && now - auto.started < 20.0 => {
                    input.active = false;
                }
                _ => {
                    input.active = false;
                    input.direction = Vec3::ZERO;
                    warn!("[harvest-auto] step {}: drops done after {:.2} s", auto.step, now - auto.started);
                    auto.step += 1;
                    auto.stage = AutoStage::Seek;
                }
            }
        }
        AutoStage::Done => {}
    }
}

/// Queued by the site change: the action, the target and the button go with
/// the site.
pub(crate) fn cancel_for_site_change(world: &mut World) {
    let token = world.resource_mut::<HarvestAction>().current.take().and_then(|current| current.token);
    {
        let mut action = world.resource_mut::<HarvestAction>();
        action.cooling = false;
        action.sustain = false;
        action.animator_speed = 1.0;
        action.hit_timing.clear();
    }
    if let Some(token) = token {
        let mut state = bevy::ecs::system::SystemState::<(
            Query<&mut AvatarDriver, With<PlayerControlled>>,
            Query<&mut AnimationPlayer>,
        )>::new(world);
        let (mut drivers, mut animators) = state.get_mut(world);
        if let Ok(mut driver) = drivers.single_mut() {
            let player = driver.player;
            if let Ok(mut animator) = animators.get_mut(player) {
                driver.release_action(token, &mut animator);
            }
        }
        let mut states = world.resource_mut::<PlayerAvatarStates>();
        states.can_intercept = true;
        if states.current == PlayerActionState::Harvest {
            states.change_status(PlayerActionState::Idle);
        }
        info!("[harvest] site change during an action: the action is dropped with the site");
    }
    world.remove_resource::<HarvestGameState>();
    let mut targeting = world.resource_mut::<HarvestTargeting>();
    targeting.contacts.clear();
    targeting.target = None;
    world.resource_mut::<HarvestButton>().hide();
    world.resource_mut::<HarvestLogQueue>().destroyed.clear();
    super::tool_model::hide_all(world);
}
