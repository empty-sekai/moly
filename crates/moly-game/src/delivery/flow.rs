//! `OnStartDelivery` / `ExecuteDelivery`: the press, the approach, the start
//! wait, the hold loop, the release window, the end action and the delivery
//! API.
//!
//! - Press (`OnStartDelivery(id)`): a press while the loop is still running
//!   after a release (`!IsEnableDelivery` and state InDelivery) resumes it;
//!   otherwise, while no delivery runs, the party's site data must be able
//!   to deliver (some item left), and the delivery starts. A release
//!   (`OnEndDelivery`) clears `IsEnableDelivery`.
//! - `ExecuteDelivery`: `IsExecuteDelivery`, the rate starts at its minimum
//!   with no carry, then the pre-action: state InDelivery (published),
//!   GameState Delivery, the joystick's forced reset, and
//!   `PrePlayerHarvestMotion`: the AutoMove to the point
//!   `_deliveryAnimationRadius` from the place toward the player (threshold
//!   0.3, the intercept gate closed while it runs), one frame after it ends
//!   the gate as it was, then a 0.3 s linear look-at toward the place (not
//!   awaited). Then the avatar's delivery animation, player state 8, the
//!   gate closed, the drop annulus turned toward the player (the base angle
//!   from the place to the player), the delivery animation again; the start
//!   wait (`BirthdayDeliveryStartWaitTime`); one loop step; the next frame
//!   the per-frame loop.
//! - The loop, each frame: released and the time since the release no
//!   longer below 0.25 s (read before this frame's delta is added): state
//!   EndDeliveryLoop (published) and the end action. Otherwise the release
//!   time grows by the frame time (0 while held), one loop step, and while
//!   some item is left the rate accelerates; with nothing left the delivery
//!   ends (`EndDeliveryExecute`) and the end action runs.
//! - Loop step (`ExecuteDeliveryLoopAction`): the site model's count for
//!   this frame spent on the party, the progress published; when the reward
//!   loop count grew, the tree blooms (the place view's director, first time
//!   in the visit), the annulus is updated and one drop falls per new loop.
//! - End action (`ExecuteDeliveryEndAction`): the rate reads zero; the
//!   avatar's step item leaves its loop and runs out, then stops and is
//!   cleared; the delivery API (state Pending, published; the reply checked
//!   against the drops the client counted); the gate opens and the player
//!   goes Idle; the total reward animation (`honor`); state Idle, published,
//!   and `IsExecuteDelivery` clears.
//!
//! The avatar's delivery animation is the step item
//! `tl_site_prop_common_dewdrop01` of the site's bundle, played on the
//! player's step item service (`PlayerStepItem`): `PlayAvatarDeliveryAnimation`
//! updates the step item, sets it up with `MoveEndTime` as its stop callback
//! and plays it; the end action turns its loop flag off, waits while its
//! director plays, stops it and clears it. A step item the service refuses
//! leaves no object, and the end action then goes on at once, as the source
//! does when the avatar has no step object. The tree's bloom is the place
//! view's director (`bloom`).
//!
//! Named stand-ins and gaps: `BindSignalReceiver` binds the step item's
//! Signal track to the site's delivery signal receiver; no exporter reads a
//! Signal track's markers, so the two signals it sends (the tree effect's
//! play and stop) are not emitted. The AutoMove state's run clip plays as
//! the locomotion's dash gait (the harvest AutoMove's stand-in). The
//! joystick's forced reset and its GameState Delivery arm are the joystick's
//! (not wired). `ExecuteHarvestSiteRefresh` (a refreshed reply) is a dialog
//! of the UI lane's; the panel never replies refreshed.

use bevy::diagnostic::FrameCount;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use moly_law::delivery as law;

use super::honor::{RewardOwner, RewardRuns};
use super::site::{DeliveryObjects, DeliverySite};
use super::{
    publish, DeliveryActionState, DeliveryGameState, DeliveryHold, DeliveryModel, DeliveryProgress,
    DeliveryRequest,
};
use crate::player::PlayerControlled;
use crate::player_avatar::item_timeline::{PlayerStepItem, StepItemOnStop};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::site_move::timeline::{delay_seconds, Delay, TweenClock};

/// `MinBirthdayDeliveryCostCountPerSecond` (FloatConfigs 166).
pub(crate) const KEY_MIN_COST_PER_SECOND: i32 = 166;
/// `MaxBirthdayDeliveryCostCountPerSecond` (FloatConfigs 167).
pub(crate) const KEY_MAX_COST_PER_SECOND: i32 = 167;
/// `BirthdayDeliveryCostCountAcceleration` (FloatConfigs 168).
pub(crate) const KEY_COST_ACCELERATION: i32 = 168;
/// `BirthdayDeliveryStartWaitFrame` (IntConfigs 177).
pub(crate) const KEY_START_WAIT_FRAME: i32 = 177;

/// The avatar's delivery step item (`PlayAvatarDeliveryAnimation`).
pub(crate) const DEWDROP_TIMELINE: &str = "tl_site_prop_common_dewdrop01";
/// The step items' bundle: `"mysekai/site/field/" + ` the site's assetbundle name.
pub(crate) const STEP_ITEM_BUNDLE_PREFIX: &str = "mysekai/site/field/";

/// DOTween eases the flow uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ease {
    Linear,
    OutCubic,
}

/// `Transform.DOLookAt(target, duration, AxisConstraint.None)`: at the start
/// the end rotation is `LookRotation(target - position)` as Euler angles;
/// each axis turns the short way from the start's Euler angles (the
/// quaternion plugin's fast mode), eased, and the rotation is rebuilt from
/// the angles.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LookAtTween {
    start: Vec3,
    change: Vec3,
    clock: TweenClock,
    ease: Ease,
    pub(crate) label: &'static str,
}

impl LookAtTween {
    pub(crate) fn new(
        rotation: Quat,
        position: Vec3,
        target: Vec3,
        duration: f32,
        ease: Ease,
        label: &'static str,
    ) -> Self {
        let (y, x, z) = rotation.to_euler(EulerRot::YXZ);
        let start = Vec3::new(
            x.to_degrees().rem_euclid(360.0),
            y.to_degrees().rem_euclid(360.0),
            z.to_degrees().rem_euclid(360.0),
        );
        let towards = target - position;
        // LookRotation of the zero vector is the identity.
        let end = if towards.length_squared() > 0.0 {
            let f = towards.normalize();
            Vec3::new(
                (-f.y)
                    .clamp(-1.0, 1.0)
                    .asin()
                    .to_degrees()
                    .rem_euclid(360.0),
                f.x.atan2(f.z).to_degrees().rem_euclid(360.0),
                0.0,
            )
        } else {
            Vec3::ZERO
        };
        let change = Vec3::new(
            law::fast_change_deg(start.x, end.x),
            law::fast_change_deg(start.y, end.y),
            law::fast_change_deg(start.z, end.z),
        );
        Self {
            start,
            change,
            clock: TweenClock::new(duration),
            ease,
            label,
        }
    }

    /// One frame: the rotation to write, and whether the tween completed.
    pub(crate) fn step(&mut self, dt: f32) -> (Quat, bool) {
        let t = self.clock.advance(dt);
        let e = match self.ease {
            Ease::Linear => t,
            Ease::OutCubic => law::ease_out_cubic(t),
        };
        let euler = self.start + self.change * e;
        (
            Quat::from_euler(
                EulerRot::YXZ,
                euler.y.to_radians(),
                euler.x.to_radians(),
                euler.z.to_radians(),
            ),
            self.clock.done(),
        )
    }

    /// The end yaw in degrees (for the log).
    pub(crate) fn end_yaw(&self) -> f32 {
        self.start.y + self.change.y
    }
}

/// The player's look-at tween in flight (the approach's and the honor
/// state's).
#[derive(Resource, Default)]
pub(crate) struct DeliveryFace(pub(crate) Option<LookAtTween>);

struct Approach {
    target: Vec3,
    corners: Vec<Vec3>,
    cursor: usize,
    speed: f32,
    /// `CanIntercept` before the AutoMove, restored after it.
    recorded_gate: bool,
    /// The AutoMove state ended; the `WaitUntil` sees it the next frame.
    left: bool,
    held_logged: bool,
    unchanged: f32,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Approach(Approach),
    StartWait(Delay),
    Loop {
        release_elapsed: f32,
    },
    /// `WaitForTimelineEnd` on the step item, since this time.
    EndTimeline {
        since: f64,
    },
    Reward {
        is_refreshed: bool,
    },
}

#[derive(Resource, Default)]
pub(crate) struct DeliveryFlow {
    phase: Phase,
    party: Option<i32>,
    started_at: f64,
    /// The motion state before the AutoMove (the dash flag).
    dash_before: bool,
    /// The last half second the trace line was written for.
    last_trace: f64,
}

impl DeliveryFlow {
    pub(crate) fn cancel(&mut self) {
        *self = Self::default();
    }

    fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "idle",
            Phase::Approach(_) => "approach",
            Phase::StartWait(_) => "start wait",
            Phase::Loop { .. } => "loop",
            Phase::EndTimeline { .. } => "end action (step item run-out)",
            Phase::Reward { .. } => "reward",
        }
    }
}

#[derive(SystemParam)]
pub(crate) struct FlowWorld<'w, 's> {
    commands: Commands<'w, 's>,
    frames: Res<'w, FrameCount>,
    time: Res<'w, Time>,
    site: Res<'w, DeliverySite>,
    model: ResMut<'w, DeliveryModel>,
    states: ResMut<'w, PlayerAvatarStates>,
    progress: MessageWriter<'w, DeliveryProgress>,
    spawns: ResMut<'w, super::drops::DeliveryDropSpawns>,
    runs: ResMut<'w, RewardRuns>,
    face: ResMut<'w, DeliveryFace>,
    mock: Option<ResMut<'w, super::server_mock::DeliveryServerMock>>,
    tables: Option<Res<'w, super::server_mock::DeliveryTables>>,
    catalog: Option<Res<'w, crate::harvest::catalog::HarvestCatalog>>,
    navigation: Option<Res<'w, crate::player_fixture_action::PlayerFixtureNavigation>>,
    game_state: Option<Res<'w, DeliveryGameState>>,
    joystick_resets: MessageWriter<'w, crate::joystick::ForceResetJoystick>,
    step: Option<ResMut<'w, PlayerStepItem>>,
    timelines: Option<Res<'w, crate::fixture_activity_timeline::FixtureActivityTimelines>>,
    bloom: ResMut<'w, super::bloom::DeliveryBloom>,
}

type PlayerQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut Transform,
        &'static mut crate::npc::MotionPhase,
        &'static mut crate::player::DashMode,
        Option<&'static crate::player_avatar::AvatarDriver>,
    ),
    With<PlayerControlled>,
>;

/// Update: the requests and the flow.
pub(crate) fn advance(
    mut world: FlowWorld,
    mut flow: ResMut<DeliveryFlow>,
    mut requests: MessageReader<DeliveryRequest>,
    mut players: PlayerQuery,
    animators: Query<&AnimationPlayer>,
) {
    let frame = u64::from(world.frames.0);
    let now = world.time.elapsed_secs_f64();
    let dt = world.time.delta_secs();
    let Some(objects) = world.site.objects.clone() else {
        requests.clear();
        return;
    };
    for request in requests.read().copied().collect::<Vec<_>>() {
        on_request(&mut world, &mut flow, &mut players, &objects, request, now);
    }
    let Some(party_id) = flow.party else {
        return;
    };
    trace_player(&world, &mut flow, &players, &animators, now);
    match std::mem::take(&mut flow.phase) {
        Phase::Idle => {}
        Phase::Approach(mut approach) => {
            if approach.left {
                finish_pre_action(
                    &mut world,
                    &mut flow,
                    &mut players,
                    &objects,
                    party_id,
                    approach,
                    now,
                );
            } else {
                step_auto_move(
                    &mut approach,
                    &mut world,
                    &mut players,
                    dt,
                    now - flow.started_at,
                );
                flow.phase = Phase::Approach(approach);
            }
        }
        Phase::StartWait(mut delay) => {
            if delay.tick(frame, dt) {
                info!(
                    "[delivery] start wait {:.3} s done at {:.3} s since the press: first loop step",
                    delay.target,
                    now - flow.started_at
                );
                loop_step(&mut world, &objects, party_id, dt);
                flow.phase = Phase::Loop {
                    release_elapsed: 0.0,
                };
            } else {
                flow.phase = Phase::StartWait(delay);
            }
        }
        Phase::Loop {
            mut release_elapsed,
        } => {
            if !world.model.enabled && release_elapsed >= law::RELEASE_WINDOW {
                world.model.state = DeliveryActionState::EndDeliveryLoop;
                let party = world.model.party(party_id).cloned();
                let current = party.as_ref().map_or(0, |p| p.tally.current_points());
                publish(
                    &mut world.progress,
                    DeliveryActionState::EndDeliveryLoop,
                    party.as_ref(),
                    current,
                    0.0,
                );
                info!(
                    "[delivery] release window: {release_elapsed:.4} s since the release (not below {}) at {:.3} s since the press: EndDeliveryLoop",
                    law::RELEASE_WINDOW,
                    now - flow.started_at
                );
                start_end_action(&mut world, &mut flow, now);
                return;
            }
            release_elapsed += dt;
            if world.model.enabled {
                release_elapsed = 0.0;
            }
            loop_step(&mut world, &objects, party_id, dt);
            let can = world
                .model
                .party(party_id)
                .is_some_and(|p| p.tally.can_delivery());
            if can {
                world.model.rate.accelerate(dt);
                flow.phase = Phase::Loop { release_elapsed };
            } else {
                world.model.enabled = false;
                info!(
                    "[delivery] nothing left to deliver at {:.3} s since the press: EndDeliveryExecute",
                    now - flow.started_at
                );
                start_end_action(&mut world, &mut flow, now);
            }
        }
        Phase::EndTimeline { since } => {
            let playing = world
                .step
                .as_deref()
                .is_some_and(|step| step.is_playing_on(world.timelines.as_deref()));
            if playing {
                flow.phase = Phase::EndTimeline { since };
            } else {
                if let Some(step) = world.step.as_deref_mut() {
                    let end = step
                        .director_time_on(world.timelines.as_deref())
                        .map_or("no director".into(), |(time, duration)| {
                            format!("director time {time:.4} of {duration:.4} s")
                        });
                    info!(
                        "[delivery-timeline] {DEWDROP_TIMELINE}: WaitForTimelineEnd returned {:.3} s after ChangeLoopFlag(false) ({:.3} s since the press; step object {:?}, {end}); Stop, ClearStepItemObject",
                        now - since,
                        now - flow.started_at,
                        step.step_item_object()
                    );
                    step.stop();
                    step.clear_step_item_object();
                }
                delivery_api(&mut world, &mut flow, &mut players, party_id, now);
            }
        }
        Phase::Reward { is_refreshed } => {
            if !world.runs.take_finished(RewardOwner::Delivery) {
                flow.phase = Phase::Reward { is_refreshed };
                return;
            }
            if is_refreshed {
                info!(
                    "[delivery] ExecuteHarvestSiteRefresh: the refreshed reply's dialog (UI lane)"
                );
            }
            world.model.state = DeliveryActionState::Idle;
            publish(&mut world.progress, DeliveryActionState::Idle, None, 0, 0.0);
            world.model.executing = false;
            flow.party = None;
            info!(
                "[delivery] ExecuteDeliveryEndAction done at {:.3} s since the press: state Idle, IsExecuteDelivery false",
                now - flow.started_at
            );
        }
    }
}

/// Every half second of a delivery: the player's position, facing, state
/// and the clip on its animator with its time (evidence of the flow's
/// output, not of the control flow).
fn trace_player(
    world: &FlowWorld,
    flow: &mut DeliveryFlow,
    players: &PlayerQuery,
    animators: &Query<&AnimationPlayer>,
    now: f64,
) {
    let since = now - flow.started_at;
    if (since / 0.5).floor() == (flow.last_trace / 0.5).floor() && flow.last_trace >= 0.0 {
        return;
    }
    flow.last_trace = since;
    let Ok((_, transform, _, _, driver)) = players.single() else {
        return;
    };
    let clips: Vec<String> = driver
        .and_then(|driver| {
            animators.get(driver.player).ok().map(|animator| {
                animator
                    .playing_animations()
                    .map(|(node, animation)| {
                        format!(
                            "{} t {:.3} w {:.2}",
                            driver.literal_of(*node).unwrap_or("?"),
                            animation.seek_time(),
                            animation.weight()
                        )
                    })
                    .collect()
            })
        })
        .unwrap_or_default();
    let p = transform.translation;
    let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
    let rate = world.model.rate;
    let tally = world
        .model
        .party(flow.party.unwrap_or(0))
        .map(|party| party.tally);
    info!(
        "[delivery-trace] {since:.3} s since the press ({}): player ({:.3}, {:.3}, {:.3}) yaw {:.1} pitch {:.1}; state {:?} gate {}; clips [{}]; rate {:.3} carry {:.3}; tally {:?}; delivery state {:?} held {}",
        flow.phase_name(),
        p.x,
        p.y,
        p.z,
        yaw.to_degrees(),
        pitch.to_degrees(),
        world.states.current,
        world.states.can_intercept,
        clips.join(", "),
        rate.current,
        rate.carry,
        tally.map(|t| (t.remaining(), t.current_points(), t.total_drop_count())),
        world.model.state,
        world.model.enabled
    );
}

fn on_request(
    world: &mut FlowWorld,
    flow: &mut DeliveryFlow,
    players: &mut PlayerQuery,
    objects: &DeliveryObjects,
    request: DeliveryRequest,
    now: f64,
) {
    match request {
        DeliveryRequest::End => {
            if world.model.enabled {
                info!("[delivery] OnEndDelivery: IsEnableDelivery false");
            }
            world.model.enabled = false;
        }
        DeliveryRequest::Start(id) => {
            if !world.model.enabled && world.model.state == DeliveryActionState::InDelivery {
                world.model.enabled = true;
                info!("[delivery] OnStartDelivery during the release window: IsEnableDelivery true again");
                return;
            }
            if world.model.executing {
                return;
            }
            let id = id.or_else(|| world.model.parties.first().map(|p| p.id));
            let Some(party) = id.and_then(|id| world.model.party(id)) else {
                info!("[delivery] OnStartDelivery: no party site data (no party in session): nothing happens");
                return;
            };
            if !party.tally.can_delivery() {
                info!(
                    "[delivery] OnStartDelivery party {}: CanDelivery false (have {}, spent {}): nothing happens",
                    party.id, party.tally.synchronized_cost_quantity, party.tally.unsynchronized_cost
                );
                return;
            }
            let party_id = party.id;
            world.model.enabled = true;
            execute_delivery(world, flow, players, objects, party_id, now);
        }
    }
}

/// `ExecuteDelivery` up to the AutoMove of `PrePlayerHarvestMotion`.
fn execute_delivery(
    world: &mut FlowWorld,
    flow: &mut DeliveryFlow,
    players: &mut PlayerQuery,
    objects: &DeliveryObjects,
    party_id: i32,
    now: f64,
) {
    let Ok((entity, transform, mut phase, mut dash, _)) = players.single_mut() else {
        return;
    };
    world.model.executing = true;
    world.model.rate.start();
    // ExecuteDeliveryPreAction.
    world.model.state = DeliveryActionState::InDelivery;
    let party = world.model.party(party_id).cloned();
    publish(
        &mut world.progress,
        DeliveryActionState::InDelivery,
        party.as_ref(),
        0,
        0.0,
    );
    if world.game_state.is_none() {
        world.commands.insert_resource(DeliveryGameState);
        info!("[delivery] GameStateManager.ChangeState(Delivery): collision updates stop until the next site move");
    }
    // PrePlayerHarvestMotion.
    let recorded_gate = world.states.can_intercept;
    let player = transform.translation;
    let target = Vec3::from_array(law::approach_target(
        objects.place.to_array(),
        player.to_array(),
        objects.animation_radius,
    ));
    world.states.can_intercept = true;
    world.states.change_status(PlayerActionState::AutoMove);
    world.states.can_intercept = false;
    let (corners, speed, note) = match world.navigation.as_deref() {
        Some(navigation) => match navigation.world.path(player, target) {
            Ok(path) => (
                path.corners,
                navigation.agent_speed,
                format!("query succeeded {}", path.query_succeeded),
            ),
            Err(error) => (
                Vec::new(),
                navigation.agent_speed,
                format!("no path ({error:?}): the agent steers to where it stands"),
            ),
        },
        None => (
            Vec::new(),
            0.0,
            "no walk-field navigation installed: the agent does not move".to_owned(),
        ),
    };
    world.commands.entity(entity).insert(DeliveryHold);
    world.joystick_resets.write(crate::joystick::ForceResetJoystick {
        reason: "delivery pre-action",
    });
    flow.dash_before = dash.0;
    *phase = crate::npc::MotionPhase::Walking;
    dash.0 = true;
    flow.party = Some(party_id);
    flow.started_at = now;
    flow.last_trace = -1.0;
    info!(
        "[delivery] ExecuteDelivery party {party_id}: IsExecuteDelivery, rate {} (min) carry 0; pre-action: state InDelivery; ForceResetJoyStick; PrePlayerHarvestMotion: AutoMove from ({:.3}, {:.3}, {:.3}) to ({:.3}, {:.3}, {:.3}), {:.3} m from the place (animation radius {}), threshold {}; {} path corners ({note}), agent speed {speed:.3}; player state {:?}, intercept closed; the AutoMove run clip plays as the dash gait (stand-in)",
        world.model.rate.current,
        player.x,
        player.y,
        player.z,
        target.x,
        target.y,
        target.z,
        target.distance(objects.place),
        objects.animation_radius,
        law::APPROACH_THRESHOLD,
        corners.len(),
        world.states.current
    );
    flow.phase = Phase::Approach(Approach {
        target,
        corners,
        cursor: 0,
        speed,
        recorded_gate,
        left: false,
        held_logged: false,
        unchanged: 0.0,
    });
}

/// One frame of the AutoMove state (the harvest AutoMove's law).
fn step_auto_move(
    approach: &mut Approach,
    world: &mut FlowWorld,
    players: &mut PlayerQuery,
    dt: f32,
    since_press: f64,
) {
    let Ok((_, mut transform, _, _, _)) = players.single_mut() else {
        return;
    };
    let position = transform.translation;
    while approach
        .corners
        .get(approach.cursor)
        .is_some_and(|corner| position.distance(*corner) <= 0.00001)
    {
        approach.cursor += 1;
    }
    let steering = approach
        .corners
        .get(approach.cursor)
        .or(approach.corners.last())
        .copied()
        .unwrap_or(position);
    match crate::harvest::law::auto_move_step(
        position,
        steering,
        approach.target,
        approach.speed,
        law::APPROACH_THRESHOLD,
    ) {
        crate::harvest::law::AutoMoveStep::Arrived => {
            world.states.can_intercept = true;
            world.states.change_status(PlayerActionState::Idle);
            approach.left = true;
            info!(
                "[delivery] AutoMove arrived at {since_press:.3} s since the press: player ({:.3}, {:.3}, {:.3}), {:.3} m from the target (below {}); SetInterceptFlag(true), state {:?}",
                position.x,
                position.y,
                position.z,
                position.distance(approach.target),
                law::APPROACH_THRESHOLD,
                world.states.current
            );
        }
        crate::harvest::law::AutoMoveStep::Move(velocity) => {
            if dt <= 0.0 {
                return;
            }
            if velocity.x != 0.0 || velocity.z != 0.0 {
                transform.rotation = Quat::from_rotation_y(velocity.x.atan2(velocity.z));
            }
            let step = velocity * dt;
            let delta = steering - position;
            let requested = if step.length_squared() >= delta.length_squared() {
                steering
            } else {
                position + step
            };
            let accepted = world
                .navigation
                .as_deref()
                .and_then(|navigation| navigation.world.constrain_move(position, requested))
                .filter(|point| point.is_finite())
                .unwrap_or(position);
            transform.translation = accepted;
            if accepted.distance(position) < 0.001 {
                approach.unchanged += dt;
            } else {
                approach.unchanged = 0.0;
            }
            if approach.unchanged >= 0.5 && !approach.held_logged {
                approach.held_logged = true;
                warn!(
                    "[delivery] AutoMove held at ({:.3}, {:.3}, {:.3}) for 0.5 s, {:.3} m from the target (threshold {}); the source has no timeout, the delivery waits",
                    accepted.x,
                    accepted.y,
                    accepted.z,
                    accepted.distance(approach.target),
                    law::APPROACH_THRESHOLD
                );
            }
        }
    }
}

/// The rest of `PrePlayerHarvestMotion` and `ExecuteDeliveryPreAction`,
/// the frame after the AutoMove state ended.
#[allow(clippy::too_many_arguments)]
fn finish_pre_action(
    world: &mut FlowWorld,
    flow: &mut DeliveryFlow,
    players: &mut PlayerQuery,
    objects: &DeliveryObjects,
    party_id: i32,
    approach: Approach,
    now: f64,
) {
    let Ok((_, transform, mut phase, mut dash, _)) = players.single_mut() else {
        flow.phase = Phase::Approach(approach);
        return;
    };
    world.states.can_intercept = approach.recorded_gate;
    *phase = crate::npc::MotionPhase::Dwelling { remaining: None };
    dash.0 = flow.dash_before;
    let face = LookAtTween::new(
        transform.rotation,
        transform.translation,
        objects.place,
        law::APPROACH_FACE_SECONDS,
        Ease::Linear,
        "approach",
    );
    let player = transform.translation;
    world.face.0 = Some(face);
    play_avatar_delivery_animation(world, objects, "1st");
    world.states.change_status(PlayerActionState::Delivery);
    world.states.can_intercept = false;
    let base = law::base_angle([-player.x, player.z], [-objects.place.x, objects.place.z]);
    let range = if let Some(party) = world.model.party_mut(party_id) {
        party.change_range(base);
        Some(party.range)
    } else {
        None
    };
    play_avatar_delivery_animation(world, objects, "2nd, restarts it");
    flow.phase = Phase::StartWait(Delay::new(
        delay_seconds(world.model.start_wait as f64),
        u64::from(world.frames.0),
    ));
    info!(
        "[delivery] pre-action at {:.3} s since the press: gate restored {}; DOLookAt place 0.3 s Linear (yaw -> {:.1} deg); player state {:?}, intercept closed; base angle {base:.4} rad from the place to ({:.3}, {:.3}); ChangeDropItemRangeData -> {range:?}; start wait {:.3} s",
        now - flow.started_at,
        approach.recorded_gate,
        face.end_yaw(),
        world.states.current,
        player.x,
        player.z,
        world.model.start_wait
    );
}

/// `PlayAvatarDeliveryAnimation`: the avatar's step item is the dewdrop,
/// set up with `MoveEndTime` as its stop callback, its Signal track bound to
/// the site's delivery signal receiver, and played.
fn play_avatar_delivery_animation(world: &mut FlowWorld, objects: &DeliveryObjects, call: &str) {
    let Some(step) = world.step.as_deref_mut() else {
        error!("[delivery-timeline] PlayAvatarDeliveryAnimation ({call}): the player's step item service is not installed");
        return;
    };
    let bundle = format!("{STEP_ITEM_BUNDLE_PREFIX}{}", objects.bundle);
    step.update_step_item_object(&bundle, DEWDROP_TIMELINE);
    step.setup(Some(StepItemOnStop::MoveEndTime));
    match &objects.signal_receiver {
        Ok(receiver) => {
            for reaction in &receiver.reactions {
                info!(
                    "[delivery-timeline] PlayAvatarDeliveryAnimation ({call}): {} reacts to {} ({}/{}) with {}",
                    receiver.receiver,
                    reaction.signal_name,
                    reaction.signal.file,
                    reaction.signal.path_id,
                    reaction.calls.join(", ")
                );
            }
            step.bind_signal_receiver(receiver.clone());
        }
        Err(reason) => error!(
            "[delivery-timeline] PlayAvatarDeliveryAnimation ({call}): BindSignalReceiver is not made: {reason}; the step item's signals reach no receiver"
        ),
    }
    step.play();
}

/// `ExecuteDeliveryLoopAction`.
fn loop_step(world: &mut FlowWorld, objects: &DeliveryObjects, party_id: i32, dt: f32) {
    let rarity = world
        .catalog
        .as_deref()
        .zip(world.model.party(party_id))
        .and_then(|(catalog, party)| catalog.materials.get(&party.reward_material_id))
        .map_or(0, |material| material.rarity);
    let model: &mut DeliveryModel = &mut world.model;
    let state = model.state;
    let rate = &mut model.rate;
    let Some(party) = model.parties.iter_mut().find(|p| p.id == party_id) else {
        return;
    };
    let before_points = party.tally.current_points();
    let before_drops = party.tally.total_drop_count();
    let count = rate.step_count(party.tally.unsynchronized_cost, dt);
    let spent = party.tally.spend(count);
    publish(&mut world.progress, state, Some(party), before_points, dt);
    let after_drops = party.tally.total_drop_count();
    if spent > 0 {
        debug!(
            "[delivery] loop step dt {dt:.4}: rate {:.3} carry {:.4} count {count} spent {spent}; points {before_points} -> {}; remaining {}",
            rate.current,
            rate.carry,
            party.tally.current_points(),
            party.tally.remaining()
        );
    }
    if after_drops <= before_drops {
        return;
    }
    let range_before = party.range;
    party.update_range();
    let range_after = party.range;
    info!(
        "[delivery] reward loops {before_drops} -> {after_drops} (points {before_points} -> {}, requirement {}): UpdateDropItemRangeData {range_before:?} -> {range_after:?}",
        party.tally.current_points(),
        party.tally.reward_loop_requirement
    );
    if !model.flowered {
        model.flowered = true;
        info!(
            "[delivery-timeline] DeliveryPlaceObjectView.PlayAnimation: the first bloom of the visit, {} (not awaited)",
            objects.bloom_timeline.as_deref().unwrap_or("(no bound timeline found)")
        );
        world.bloom.play_animation();
    }
    for _ in before_drops..after_drops {
        super::drops::on_drop_item(
            model,
            &mut world.spawns,
            objects,
            party_id,
            rarity,
            true,
            false,
            "loop step",
        );
    }
}

/// The start of `ExecuteDeliveryEndAction`: the rate, the step item's
/// loop flag off, then `WaitForTimelineEnd`. With no step object the source
/// goes straight to `ClearStepItemObject`; the wait then ends at once.
fn start_end_action(world: &mut FlowWorld, flow: &mut DeliveryFlow, now: f64) {
    world.model.rate.stop();
    let object = world
        .step
        .as_deref()
        .and_then(PlayerStepItem::step_item_object);
    if let Some(step) = world.step.as_deref_mut() {
        step.change_loop_flag(false);
    }
    info!(
        "[delivery] ExecuteDeliveryEndAction at {:.3} s since the press: rate 0; {DEWDROP_TIMELINE}: ChangeLoopFlag(false) on step object {object:?}, WaitForTimelineEnd",
        now - flow.started_at
    );
    flow.phase = Phase::EndTimeline { since: now };
}

/// `DeliveryExecuteApiAsync`, the gate and Idle, then the reward animation.
fn delivery_api(
    world: &mut FlowWorld,
    flow: &mut DeliveryFlow,
    players: &mut PlayerQuery,
    party_id: i32,
    now: f64,
) {
    world.model.state = DeliveryActionState::Pending;
    let party = world.model.party(party_id).cloned();
    publish(
        &mut world.progress,
        DeliveryActionState::Pending,
        party.as_ref(),
        0,
        0.0,
    );
    let mut rewards = Vec::new();
    let mut is_refreshed = false;
    match (world.mock.as_deref_mut(), world.tables.as_deref(), party) {
        (Some(mock), Some(tables), Some(party)) => {
            let obtained_before = mock.obtained_count(party_id);
            let auto_count = party.auto_gathered.len() as i32;
            let unsynced = party.unsynced_drops.len() as i32;
            let cost = party.tally.unsynchronized_cost;
            let reply = mock.deliver(party_id, cost, tables);
            let mut regenerate = false;
            if let Some(reply) = reply.as_ref() {
                if reply.dropped_reward_count != unsynced {
                    error!(
                        "[delivery] the reply's dropped reward count {} is not the client's {unsynced} unsynchronized drops (party {party_id})",
                        reply.dropped_reward_count
                    );
                    regenerate = true;
                }
            }
            let (have, points) = mock.user_rows(party_id, party.item_material_id);
            if let Some(site_party) = world.model.party_mut(party_id) {
                site_party.update_synchronized(have, points);
            }
            let obtained_after = mock.obtained_count(party_id);
            if auto_count != obtained_after - obtained_before {
                error!(
                    "[delivery] party {party_id}: {auto_count} drops flew to the player, the reply's obtained count moved {obtained_before} -> {obtained_after}"
                );
            }
            match reply {
                Some(reply) => {
                    info!(
                        "[delivery] DeliveryExecuteApiAsync party {party_id}: sent {cost}; dropped {} (client {unsynced}), auto-gathered {auto_count} (obtained {obtained_before} -> {obtained_after}); synchronized: have {have}, points {points}; {} total rewards",
                        reply.dropped_reward_count,
                        reply.total_rewards.len()
                    );
                    rewards = reply.total_rewards;
                    is_refreshed = reply.is_refreshed;
                }
                None => error!("[delivery] the delivery API replied nothing: the error dialog (UI lane)"),
            }
            if regenerate {
                if let Some(objects) = world.site.objects.clone() {
                    super::drops::generate_unclaimed(&mut world.model, mock, &mut world.spawns, &objects, "reply mismatch");
                }
            }
        }
        _ => error!("[delivery] DeliveryExecuteApiAsync without the panel or the tables: the error dialog (UI lane)"),
    }
    world.states.can_intercept = true;
    world.states.change_status(PlayerActionState::Idle);
    if let Ok((entity, _, _, _, _)) = players.single_mut() {
        world.commands.entity(entity).remove::<DeliveryHold>();
    }
    info!(
        "[delivery] end action at {:.3} s since the press: SetInterceptFlag(true), player state {:?}; PlayTotalRewardAnimation with {} rewards",
        now - flow.started_at,
        world.states.current,
        rewards.len()
    );
    world.runs.start(RewardOwner::Delivery, rewards);
    flow.phase = Phase::Reward { is_refreshed };
}

/// Update, after the honor runner: the player's look-at tween.
pub(crate) fn apply_face(
    time: Res<Time>,
    mut face: ResMut<DeliveryFace>,
    mut players: Query<&mut Transform, With<PlayerControlled>>,
) {
    let Some(tween) = face.0.as_mut() else {
        return;
    };
    let Ok(mut transform) = players.single_mut() else {
        return;
    };
    let (rotation, done) = tween.step(time.delta_secs());
    transform.rotation = rotation;
    if done {
        let (yaw, pitch, _) = rotation.to_euler(EulerRot::YXZ);
        info!(
            "[delivery] DOLookAt ({}) complete: yaw {:.2} deg, pitch {:.2} deg",
            tween.label,
            yaw.to_degrees(),
            pitch.to_degrees()
        );
        face.0 = None;
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    /// DOLookAt on the yaw: from facing +z (yaw 0) toward +x (yaw 90) the
    /// linear tween is at 45 deg halfway; a target behind turns the short
    /// way (350 -> 10 is +20); the zero vector looks at the identity.
    #[test]
    fn look_at_turns_the_short_way() {
        let mut tween =
            LookAtTween::new(Quat::IDENTITY, Vec3::ZERO, Vec3::X, 1.0, Ease::Linear, "t");
        let (half, _) = tween.step(0.5);
        let (yaw, _, _) = half.to_euler(EulerRot::YXZ);
        assert!(
            (yaw.to_degrees() - 45.0).abs() < 1e-3,
            "{}",
            yaw.to_degrees()
        );
        let (end, done) = tween.step(0.5);
        assert!(done);
        assert!((end.to_euler(EulerRot::YXZ).0.to_degrees() - 90.0).abs() < 1e-3);
        let from = Quat::from_rotation_y(350f32.to_radians());
        let target = Vec3::new(10f32.to_radians().sin(), 0.0, 10f32.to_radians().cos());
        let tween = LookAtTween::new(from, Vec3::ZERO, target, 1.0, Ease::Linear, "t");
        assert!((tween.change.y - 20.0).abs() < 1e-3, "{}", tween.change.y);
        let zero = LookAtTween::new(from, Vec3::ZERO, Vec3::ZERO, 1.0, Ease::Linear, "t");
        assert!((zero.start.y + zero.change.y - 360.0).abs() < 1e-3);
    }
}
