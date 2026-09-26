//! `PlayTotalRewardAnimation` and the honor reward state.
//!
//! A reply's total rewards (the delivery's or the gather's): with none the
//! method returns at once. Otherwise state GetReward (published); the
//! rewards split into the honor (`ResourceType` honor) and the rest. The
//! rest show in one `CommonRewardSubWindowDialog` (`DialogUtility.
//! ShowGetResourceDialogAsync(rest, MSG_RECEIVED_DELIVERY_TOTAL_REWARD,
//! OpenSE.MysekaiGetBlueprint)`: `ShowSubWindowDialog(null, onClose,
//! allowCloseExternal true, CommonRewardSubWindowDialog)`), awaited until its
//! onClose. With an honor: a 500 ms wait, player state 9
//! (`PlayerAvatarDeliveryHonorRewardState`), the intercept gate closed, a
//! wait of `DeliveryHonorRewardDialogShowTime` (FloatConfigs 173) truncated
//! to whole milliseconds, then a `ChainDialogPlayer` with one
//! `HonorRewardSubWindowDialog` per honor (`ChainSubWindowDialog(message,
//! null, allowCloseExternal true, HonorRewardSubWindowDialog)`, `Setup(
//! resource, OpenSE.MysekaiGetBlueprint)`), played until its onFinish; then
//! state Idle (published), the gate open and player state Idle. Without an
//! honor, state Idle (published) after the dialog.
//!
//! The dialogs open through the screen manager ([`crate::ui_layers`]):
//! `ShowDialog` (the Dialog layer, `SubWindowDialog`'s back key), `Open`,
//! the open animation's end, and on a close request `SubWindowDialog.
//! CloseProcess` -> `Close`: its onClose first (the flow goes on), then
//! `DialogBase.Close` and the destruction. A chained dialog's onClose opens
//! the next one; the last one's ends the chain. The close requests are the
//! manager's hardware back key, which the topmost dialog takes
//! (`OnHardwareBackKeyProcess` -> `CloseProcess`), and the dialog view's own
//! close taps.
//!
//! Named gaps: neither dialog has a view in this product. The root carries
//! no `CommonRewardSubWindowDialog` prefab; the `HonorRewardSubWindowDialog`
//! prefab is on the root, but its message key (`MSG_RECEIVED_BIRTHDAY_HONOR`
//! or `WORD_ACHIEVEMENT_GET` by the honor's rarity and level) and its honor
//! image need the honors master and the honor images, which the root does
//! not carry. Without a view the open and close animations pass in the frame
//! they start, and a dialog closes only on the back key.
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

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use super::server_mock::{Reward, HONOR};
use super::{publish, DeliveryActionState, DeliveryHold, DeliveryModel, DeliveryProgress};
use crate::player::PlayerControlled;
use crate::player_avatar::item_timeline::PlayerStepItem;
use crate::player_state::{PlayerActionState, PlayerAvatarStates};
use crate::site_move::timeline::Delay;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
};

/// `DeliveryHonorRewardDialogShowTime` (FloatConfigs 173).
pub(crate) const KEY_HONOR_DIALOG_SHOW_TIME: i32 = 173;
/// The honor state's step item.
const PETAL_TIMELINE: &str = "tl_site_prop_common_petal1";
/// `DialogType.CommonRewardSubWindowDialog` (the enum's value).
const COMMON_REWARD_DIALOG: DialogType = DialogType(250);
/// `DialogType.HonorRewardSubWindowDialog`.
const HONOR_REWARD_DIALOG: DialogType = DialogType(262);
/// `MSG_RECEIVED_DELIVERY_TOTAL_REWARD`, the reward dialog's main text.
const TOTAL_REWARD_MESSAGE: &str = "MSG_RECEIVED_DELIVERY_TOTAL_REWARD";

/// A dialog shown through the screen manager and awaited.
#[derive(Clone, Copy, Debug)]
struct Shown {
    id: DialogId,
    dialog: DialogType,
}

/// `ShowSubWindowDialog` / `ChainSubWindowDialog` then `Open`: the manager
/// shows it and, with no view, its open animation ends in the same frame.
/// None when the manager has no prefab for the type (the source's
/// `InstantiateDialog` returns null there): logged, and the run goes on as
/// if it closed.
fn show(
    screens: &mut ScreenManager,
    dialog: DialogType,
    caller: &str,
    what: &str,
) -> Option<Shown> {
    match screens.show_dialog(
        dialog,
        DisplayLayerType::LayerDialog,
        DialogBackKey::Close,
        caller,
    ) {
        Ok(id) => {
            screens.open_dialog(id);
            screens.dialog_open_finished(id);
            info!(
                "[delivery] {caller}: {dialog:?} shown and opened ({what}); no view in this product: the open animation passes at once; awaiting its onClose (the back key closes it)"
            );
            Some(Shown { id, dialog })
        }
        Err(reason) => {
            error!("[delivery] {reason}; {what} is not shown and the run goes on as if it closed");
            None
        }
    }
}

/// `SubWindowDialog.CloseProcess` -> `Close`: onClose (the caller goes on),
/// then `DialogBase.Close` and, with no close animation, the destruction.
fn close(screens: &mut ScreenManager, shown: Shown, why: &str) {
    info!(
        "[delivery] {:?}: {why}: CloseProcess -> Close: onClose",
        shown.dialog
    );
    screens.close_dialog(shown.id);
    screens.dialog_destroyed(shown.id);
}

/// Who awaits a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RewardOwner {
    Delivery,
    Gather,
}

enum RewardPhase {
    Start,
    OthersDialog(Option<Shown>),
    HonorDelay(Delay),
    HonorWait(Delay),
    /// The chain: the dialog shown now and the honors still queued.
    HonorDialog {
        shown: Option<Shown>,
        next: usize,
    },
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

    /// The site went: every run stops. Returns the dialogs they had shown
    /// (a site move cannot start while one is shown in the source, whose
    /// dialogs block the field input; the product closes them with the run).
    pub(crate) fn cancel(&mut self) -> Vec<DialogId> {
        let shown = self
            .runs
            .iter()
            .filter_map(|run| match run.phase {
                RewardPhase::OthersDialog(shown) | RewardPhase::HonorDialog { shown, .. } => {
                    shown.map(|shown| shown.id)
                }
                _ => None,
            })
            .collect();
        self.runs.clear();
        self.finished.clear();
        shown
    }
}

/// A dialog the flow awaits is shown (the instrument reads it).
#[derive(Resource, Default)]
pub(crate) struct DialogAwait {
    pub(crate) open: Option<DialogId>,
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
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut screens: ResMut<ScreenManager>,
    mut awaiting: ResMut<DialogAwait>,
    mut face: ResMut<super::flow::DeliveryFace>,
    site: Res<super::site::DeliverySite>,
    players: Query<(Entity, &Transform), With<PlayerControlled>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut step: Option<ResMut<PlayerStepItem>>,
) {
    let back: Vec<DialogBackKeyEvent> = back_keys.read().copied().collect();
    let closed_now = |shown: Option<Shown>| -> bool {
        shown.is_some_and(|shown| {
            back.iter()
                .any(|event| event.id == shown.id && event.back_key == DialogBackKey::Close)
        })
    };
    let rate = model.rate;
    let frame = u64::from(frames.0);
    let dt = time.delta_secs();
    let mut index = 0;
    let mut dialog_open = None;
    while index < runs.runs.len() {
        let run = &mut runs.runs[index];
        let done = match &mut run.phase {
            RewardPhase::Start => {
                if run.honor.is_empty() && run.others.is_empty() {
                    true
                } else {
                    model.state = DeliveryActionState::GetReward;
                    publish(
                        &mut progress,
                        DeliveryActionState::GetReward,
                        None,
                        0,
                        0.0,
                        rate,
                    );
                    info!(
                        "[delivery] PlayTotalRewardAnimation ({:?}): state GetReward; honor {:?}; others {}",
                        run.owner,
                        run.honor.iter().map(|r| (r.resource_id, r.level)).collect::<Vec<_>>(),
                        run.others.len()
                    );
                    if !run.others.is_empty() {
                        let what = format!(
                            "{TOTAL_REWARD_MESSAGE} with {} resources {:?}",
                            run.others.len(),
                            run.others
                                .iter()
                                .map(|r| format!(
                                    "{} {} x{}",
                                    r.resource_type, r.resource_id, r.quantity
                                ))
                                .collect::<Vec<_>>()
                        );
                        let shown = show(
                            &mut screens,
                            COMMON_REWARD_DIALOG,
                            "DialogUtility.ShowGetResourceDialogAsync",
                            &what,
                        );
                        dialog_open = shown.map(|s| s.id);
                        run.phase = RewardPhase::OthersDialog(shown);
                        false
                    } else {
                        start_honor(run, frame)
                    }
                }
            }
            RewardPhase::OthersDialog(shown) => {
                let closed = match *shown {
                    None => true,
                    Some(dialog) if closed_now(Some(dialog)) => {
                        close(&mut screens, dialog, "the back key");
                        true
                    }
                    Some(_) => false,
                };
                if closed && run.honor.is_empty() {
                    info!("[delivery] the reward dialog's onClose; no honor among the rewards: state Idle");
                    model.state = DeliveryActionState::Idle;
                    publish(&mut progress, DeliveryActionState::Idle, None, 0, 0.0, rate);
                    true
                } else if closed {
                    info!("[delivery] the reward dialog's onClose");
                    start_honor(run, frame)
                } else {
                    dialog_open = shown.map(|s| s.id);
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
                        "[delivery] honor wait {:.3} s done: ChainDialogPlayer with {} HonorRewardSubWindowDialog ({:?}), Play",
                        delay.target,
                        run.honor.len(),
                        run.honor.iter().map(|r| (r.resource_id, r.level)).collect::<Vec<_>>()
                    );
                    let shown = show_honor(&mut screens, &run.honor, 0);
                    dialog_open = shown.map(|s| s.id);
                    run.phase = RewardPhase::HonorDialog { shown, next: 1 };
                }
                false
            }
            RewardPhase::HonorDialog { shown, next } => {
                // A chained dialog's onClose opens the next one.
                while shown.is_none() || closed_now(*shown) {
                    if let Some(dialog) = shown.take() {
                        close(&mut screens, dialog, "the back key");
                    }
                    if *next >= run.honor.len() {
                        break;
                    }
                    *shown = show_honor(&mut screens, &run.honor, *next);
                    *next += 1;
                    if shown.is_some() {
                        break;
                    }
                }
                if shown.is_none() {
                    info!("[delivery] the honor chain's onFinish");
                    model.state = DeliveryActionState::Idle;
                    publish(&mut progress, DeliveryActionState::Idle, None, 0, 0.0, rate);
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
                    dialog_open = shown.map(|s| s.id);
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

/// The chain's dialog of honor `index` (its `Setup(resource,
/// OpenSE.MysekaiGetBlueprint)`).
fn show_honor(screens: &mut ScreenManager, honor: &[Reward], index: usize) -> Option<Shown> {
    let reward = &honor[index];
    show(
        screens,
        HONOR_REWARD_DIALOG,
        "ChainDialogPlayer.ChainSubWindowDialog",
        &format!(
            "honor {} of {}: honor id {} level {:?}; its message key needs the honor's master rarity",
            index + 1,
            honor.len(),
            reward.resource_id,
            reward.level
        ),
    )
}

/// After the reward dialog (or without one), with an honor: the
/// `UniTask.Delay(500)` before the honor state. The run goes on.
fn start_honor(run: &mut Run, frame: u64) -> bool {
    run.phase = RewardPhase::HonorDelay(Delay::new(
        (moly_law::delivery::HONOR_REWARD_DELAY_MS as f64 / 1000.0) as f32,
        frame,
    ));
    false
}
