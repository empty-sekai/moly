//! A fixture action that did not start, or stopped on a failure, says so over
//! the field, next to the log line the runner already writes.

use bevy::prelude::*;
use bevy::ui::{FocusPolicy, UiTargetCamera};

use crate::player_fixture_action::{
    PlayerFixtureCancelReason, PlayerFixtureOutcome, PlayerFixtureRequest, PlayerFixtureRuntime,
};

use super::widgets::BAD;

/// How long a failure stays on screen.
const SHOWN_SECS: f64 = 8.;

#[derive(Component)]
struct FaultToast;

#[derive(Resource, Default)]
struct Shown {
    /// The failure the toast was last armed for; cleared once the runner
    /// reports anything else or a new request arrives, so the same failure
    /// pressed again shows again.
    armed_for: Option<String>,
    until: f64,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Shown>()
        .add_systems(
            Startup,
            setup.after(crate::balloon::overlay_camera),
        )
        .add_systems(Update, update.after(crate::player_fixture_action::advance));
}

fn setup(
    mut c: Commands,
    ui_font: Res<crate::ui_font::UiFontAsset>,
    hud: Query<Entity, With<crate::balloon::HudCamera>>,
) {
    let Ok(camera) = hud.single() else {
        return;
    };
    c.spawn((
        Text::new(""),
        TextFont {
            font: ui_font.0.clone().into(),
            font_size: FontSize::Px(16.),
            ..default()
        },
        TextColor(BAD),
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            width: percent(100),
            bottom: px(96),
            display: Display::None,
            ..default()
        },
        FocusPolicy::Pass,
        FaultToast,
        UiTargetCamera(camera),
        GlobalZIndex(1290),
    ));
}

/// What a failed outcome says; `None` for the outcomes that are not failures.
fn failure(outcome: &PlayerFixtureOutcome) -> Option<String> {
    match outcome {
        PlayerFixtureOutcome::NotPrepared(error) => {
            Some(format!("fixture action refused: {error:?}"))
        }
        PlayerFixtureOutcome::LoadFailed { target, error } => {
            Some(format!("fixture action failed to load ({}): {error:?}", target.uid))
        }
        PlayerFixtureOutcome::GimmickNotPrepared { target, reason } => {
            Some(format!("fixture gimmick refused ({}): {reason}", target.uid))
        }
        PlayerFixtureOutcome::Cancelled(
            reason @ (PlayerFixtureCancelReason::NavigationFailed
            | PlayerFixtureCancelReason::AnimationFailed
            | PlayerFixtureCancelReason::OwnershipLost),
        ) => Some(format!("fixture action cancelled: {reason:?}")),
        PlayerFixtureOutcome::Completed
        | PlayerFixtureOutcome::Cancelled(_)
        | PlayerFixtureOutcome::GimmickStarted { .. } => None,
    }
}

fn update(
    runtime: Option<Res<PlayerFixtureRuntime>>,
    mut requests: MessageReader<PlayerFixtureRequest>,
    time: Res<Time>,
    mut shown: ResMut<Shown>,
    mut toast: Query<(&mut Text, &mut Node), With<FaultToast>>,
) {
    let Ok((mut text, mut node)) = toast.single_mut() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if requests
        .read()
        .any(|request| matches!(request, PlayerFixtureRequest::Timeline(_) | PlayerFixtureRequest::Gimmick(_)))
    {
        shown.armed_for = None;
        node.display = Display::None;
    }
    let current = runtime
        .as_deref()
        .and_then(|runtime| runtime.last_outcome.as_ref())
        .and_then(failure);
    match current {
        Some(message) if shown.armed_for.as_deref() != Some(message.as_str()) => {
            text.0 = message.clone();
            shown.armed_for = Some(message);
            shown.until = now + SHOWN_SECS;
            node.display = Display::Flex;
        }
        Some(_) => {}
        None => shown.armed_for = None,
    }
    if node.display != Display::None && now >= shown.until {
        node.display = Display::None;
    }
}
