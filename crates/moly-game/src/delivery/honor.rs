//! `PlayTotalRewardAnimation` and the honor reward state.
//!
//! A reply's total rewards (the delivery's; the gather's are always empty
//! from the panel): with none the method returns at once. Otherwise state
//! GetReward (published); the rewards split into the honor (`ResourceType`
//! honor) and the rest. The rest show in the get-resource dialog, awaited.
//! With an honor: a 500 ms wait, player state 9
//! (`PlayerAvatarDeliveryHonorRewardState`), the intercept gate closed, a
//! wait of `DeliveryHonorRewardDialogShowTime` (FloatConfigs 173) truncated to whole
//! milliseconds, the honor reward dialog chain, awaited; then state Idle
//! (published), the gate open and player state Idle. Without an honor,
//! state Idle (published) after the dialog.
//!
//! State 9: `Initialize` switches the camera to the honor reward state
//! (`delivery_camera`), turns the avatar toward the camera (DOLookAt of
//! the player position minus the camera's forward on x/z, 1.5 s OutCubic)
//! and plays the petal step item; `Dispose` stops and clears the step item
//! and switches the camera back to its previous state.
//!
//! The petal is the step item `tl_site_prop_common_petal1` of the delivery
//! site's bundle on the player's step item service: `Initialize` updates the
//! step item, sets it up with no stop callback and plays it; `Dispose`
//! stops its director and clears it.
//!
//! Named stand-ins: the two dialogs are the UI lane's; their close is a
//! [`DeliveryDialogClosed`] message (the Y key, or the autoplay).

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use super::server_mock::{Reward, HONOR};
use super::{
    publish, DeliveryActionState, DeliveryDialogClosed, DeliveryHold, DeliveryModel,
    DeliveryProgress,
};
use crate::player::PlayerControlled;
use crate::player_avatar::item_timeline::PlayerStepItem;
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::site_move::timeline::Delay;

/// `DeliveryHonorRewardDialogShowTime` (FloatConfigs 173).
pub(crate) const KEY_HONOR_DIALOG_SHOW_TIME: i32 = 173;
/// The honor state's step item.
const PETAL_TIMELINE: &str = "tl_site_prop_common_petal1";

/// Who awaits a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RewardOwner {
    Delivery,
    Gather,
}

enum RewardPhase {
    Start,
    OthersDialog,
    HonorDelay(Delay),
    HonorWait(Delay),
    HonorDialog,
}

struct Run {
    owner: RewardOwner,
    phase: RewardPhase,
    honor: Vec<Reward>,
    others: Vec<Reward>,
}

/// The `PlayTotalRewardAnimation` calls in flight.
#[derive(Resource, Default)]
pub(crate) struct RewardRuns {
    runs: Vec<Run>,
    finished: Vec<RewardOwner>,
}

impl RewardRuns {
    pub(crate) fn start(&mut self, owner: RewardOwner, rewards: Vec<Reward>) {
        let (honor, others) = rewards
            .into_iter()
            .partition(|reward| reward.resource_type == HONOR);
        self.runs.push(Run {
            owner,
            phase: RewardPhase::Start,
            honor,
            others,
        });
    }

    /// The owner's run has returned (consumed once).
    pub(crate) fn take_finished(&mut self, owner: RewardOwner) -> bool {
        match self.finished.iter().position(|o| *o == owner) {
            Some(index) => {
                self.finished.remove(index);
                true
            }
            None => false,
        }
    }

    pub(crate) fn cancel(&mut self) {
        self.runs.clear();
        self.finished.clear();
    }
}

/// A dialog the flow awaits is open (the instrument reads it).
#[derive(Resource, Default)]
pub(crate) struct DialogAwait {
    pub(crate) open: bool,
}

/// Update: step every run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    mut commands: Commands,
    frames: Res<FrameCount>,
    time: Res<Time>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    mut runs: ResMut<RewardRuns>,
    mut model: ResMut<DeliveryModel>,
    mut states: ResMut<PlayerAvatarStates>,
    mut progress: MessageWriter<DeliveryProgress>,
    mut closes: MessageReader<DeliveryDialogClosed>,
    mut awaiting: ResMut<DialogAwait>,
    mut face: ResMut<super::flow::DeliveryFace>,
    site: Res<super::site::DeliverySite>,
    players: Query<(Entity, &Transform), With<PlayerControlled>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut step: Option<ResMut<PlayerStepItem>>,
) {
    let closed = closes.read().count() > 0;
    let frame = u64::from(frames.0);
    let dt = time.delta_secs();
    let mut index = 0;
    let mut dialog_open = false;
    while index < runs.runs.len() {
        let run = &mut runs.runs[index];
        let done = match &mut run.phase {
            RewardPhase::Start => {
                if run.honor.is_empty() && run.others.is_empty() {
                    true
                } else {
                    model.state = DeliveryActionState::GetReward;
                    publish(&mut progress, DeliveryActionState::GetReward, None, 0, 0.0);
                    info!(
                        "[delivery] PlayTotalRewardAnimation ({:?}): state GetReward; honor {:?}; others {}",
                        run.owner,
                        run.honor.iter().map(|r| (r.resource_id, r.level)).collect::<Vec<_>>(),
                        run.others.len()
                    );
                    if !run.others.is_empty() {
                        info!(
                            "[delivery] ShowGetResourceDialogAsync (UI lane) with {} resources {:?}: awaiting its close",
                            run.others.len(),
                            run.others
                                .iter()
                                .map(|r| format!("{} {} x{}", r.resource_type, r.resource_id, r.quantity))
                                .collect::<Vec<_>>()
                        );
                        run.phase = RewardPhase::OthersDialog;
                        dialog_open = true;
                        false
                    } else {
                        start_honor(run, frame)
                    }
                }
            }
            RewardPhase::OthersDialog => {
                if closed && run.honor.is_empty() {
                    info!("[delivery] the get-resource dialog closed; no honor among the rewards: state Idle");
                    model.state = DeliveryActionState::Idle;
                    publish(&mut progress, DeliveryActionState::Idle, None, 0, 0.0);
                    true
                } else if closed {
                    info!("[delivery] the get-resource dialog closed");
                    start_honor(run, frame)
                } else {
                    dialog_open = true;
                    false
                }
            }
            RewardPhase::HonorDelay(delay) => {
                if delay.tick(frame, dt) {
                    // Player state 9: the gate is open (the end action opened it).
                    states.change_status(PlayerActionState::DeliveryHonorReward);
                    let entered = states.current == PlayerActionState::DeliveryHonorReward;
                    states.can_intercept = false;
                    let wait_ms = configs
                        .as_deref()
                        .map_or(0, |c| (c.float(KEY_HONOR_DIALOG_SHOW_TIME) * 1000.0) as i32);
                    if let (Ok((entity, transform)), Ok(camera)) =
                        (players.single(), cameras.single())
                    {
                        commands.entity(entity).insert(DeliveryHold);
                        let forward = camera.forward();
                        let p = transform.translation;
                        let target = Vec3::new(p.x - forward.x, p.y, p.z - forward.z);
                        let tween = super::flow::LookAtTween::new(
                            transform.rotation,
                            p,
                            target,
                            moly_law::delivery::HONOR_FACE_SECONDS,
                            super::flow::Ease::OutCubic,
                            "honor",
                        );
                        info!(
                            "[delivery] PlayerAvatarDeliveryHonorRewardState.Initialize (entered {entered}): camera ChangeState(DeliveryHonorReward); DoLookAt player minus camera forward ({:.3}, {:.3}, {:.3}) 1.5 s OutCubic (yaw -> {:.1} deg); intercept closed; wait {wait_ms} ms",
                            target.x,
                            target.y,
                            target.z,
                            tween.end_yaw()
                        );
                        face.0 = Some(tween);
                    }
                    commands.queue(crate::delivery_camera::enter);
                    match (site.objects.as_ref(), step.as_deref_mut()) {
                        (Some(objects), Some(step)) => {
                            let bundle = format!(
                                "{}{}",
                                super::flow::STEP_ITEM_BUNDLE_PREFIX,
                                objects.bundle
                            );
                            step.update_step_item_object(&bundle, PETAL_TIMELINE);
                            step.setup(None);
                            step.play();
                            info!(
                                "[delivery-timeline] honor state StartAnimation: UpdateStepItemObject({bundle}, {PETAL_TIMELINE}), Setup (no stop callback), Play"
                            );
                        }
                        _ => error!(
                            "[delivery-timeline] honor state StartAnimation: no delivery site objects or no step item service; {PETAL_TIMELINE} is not played"
                        ),
                    }
                    run.phase =
                        RewardPhase::HonorWait(Delay::new((wait_ms as f64 / 1000.0) as f32, frame));
                }
                false
            }
            RewardPhase::HonorWait(delay) => {
                if delay.tick(frame, dt) {
                    info!(
                        "[delivery] honor wait {:.3} s done: HonorRewardSubWindowDialog chain (UI lane) with {:?}: awaiting its close",
                        delay.target,
                        run.honor.iter().map(|r| (r.resource_id, r.level)).collect::<Vec<_>>()
                    );
                    run.phase = RewardPhase::HonorDialog;
                    dialog_open = true;
                }
                false
            }
            RewardPhase::HonorDialog => {
                if closed {
                    model.state = DeliveryActionState::Idle;
                    publish(&mut progress, DeliveryActionState::Idle, None, 0, 0.0);
                    states.can_intercept = true;
                    states.change_status(PlayerActionState::Idle);
                    if let Ok((entity, _)) = players.single() {
                        commands.entity(entity).remove::<DeliveryHold>();
                    }
                    commands.queue(crate::delivery_camera::exit);
                    let object = step.as_deref().and_then(PlayerStepItem::step_item_object);
                    if let Some(step) = step.as_deref_mut() {
                        step.stop();
                        step.clear_step_item_object();
                    }
                    info!(
                        "[delivery] honor dialog closed: state Idle; SetInterceptFlag(true), player state {:?}; the honor state's Dispose: {PETAL_TIMELINE} Stop, ClearStepItemObject (step object {object:?}), camera back to its previous state",
                        states.current
                    );
                    true
                } else {
                    dialog_open = true;
                    false
                }
            }
        };
        if done {
            let owner = runs.runs.remove(index).owner;
            runs.finished.push(owner);
        } else {
            index += 1;
        }
    }
    awaiting.open = dialog_open;
}

/// After the get-resource dialog (or without one), with an honor: the
/// `UniTask.Delay(500)` before the honor state. The run goes on.
fn start_honor(run: &mut Run, frame: u64) -> bool {
    run.phase = RewardPhase::HonorDelay(Delay::new(
        (moly_law::delivery::HONOR_REWARD_DELAY_MS as f64 / 1000.0) as f32,
        frame,
    ));
    false
}
