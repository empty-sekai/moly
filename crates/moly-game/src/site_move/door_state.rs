//! The four house and room player states the door moves (and the entry's
//! house exit) change into, as one shared entry point: `ChangeState(state)`
//! runs `ChangeStatus`, whose intercept gate may drop the request, then the
//! state's `Initialize`: the gate closes and the state clip plays through
//! `PlayerAvatarPresenter.PlayAnimation`, crossfading 0.25 s at speed 1 (the
//! float each state passes there is an animation speed that
//! `AvatarBase.ChangeMotion` resets to 1). The timers of the states'
//! `UpdateState` are in `player_state::update_house_states`.
//!
//! | state                  | clip                              |
//! |------------------------|-----------------------------------|
//! | EnterMoveHouse (11)    | `act_u000_hou_house_open_013_o`   |
//! | ExitMoveHouse (13)     | `act_u000_hou_house_open_011_o`   |
//! | EnterMoveMyRoom (14)   | `c_000_mov_myroom_action_02`      |
//! | ExitMoveMyRoom (15)    | `act_u000_hou_house_open_023_o`   |

use std::time::Duration;

use bevy::ecs::system::SystemState;
use bevy::prelude::*;

use crate::player::PlayerControlled;
use crate::player_avatar::{
    AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, AUTO_MOVE_CLIP,
    IDLE_CLIP,
};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};

use super::door_law::STATE_CLIP_FADE;

/// One of the four states, with its source clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HouseState {
    EnterMoveHouse,
    ExitMoveHouse,
    EnterMoveMyRoom,
    ExitMoveMyRoom,
}

impl HouseState {
    pub(crate) fn state(self) -> PlayerActionState {
        match self {
            Self::EnterMoveHouse => PlayerActionState::EnterMoveHouse,
            Self::ExitMoveHouse => PlayerActionState::ExitMoveHouse,
            Self::EnterMoveMyRoom => PlayerActionState::EnterMoveMyRoom,
            Self::ExitMoveMyRoom => PlayerActionState::ExitMoveMyRoom,
        }
    }

    /// The literal clip name the state's `Initialize` passes.
    pub(crate) fn source_clip(self) -> &'static str {
        match self {
            Self::EnterMoveHouse => "act_u000_hou_house_open_013_o",
            Self::ExitMoveHouse => "act_u000_hou_house_open_011_o",
            Self::EnterMoveMyRoom => "c_000_mov_myroom_action_02",
            Self::ExitMoveMyRoom => "act_u000_hou_house_open_023_o",
        }
    }
}

/// `PlayerAvatarPresenter.ChangeState(state)`. Returns the action token the
/// clip plays under (the given one, or a new Door token), or `None` when the
/// gate dropped the change or the clip could not play (named in the log).
pub(crate) fn change_state(
    world: &mut World,
    state: HouseState,
    token: Option<PlayerActionToken>,
) -> Option<PlayerActionToken> {
    world
        .resource_mut::<PlayerAvatarStates>()
        .change_status(state.state());
    if world.resource::<PlayerAvatarStates>().current != state.state() {
        warn!("[player-state] ChangeState({state:?}) dropped by the closed intercept gate");
        return token;
    }
    play(world, state.source_clip(), STATE_CLIP_FADE, token)
}

/// `PlayerAvatarIdleState.Initialize`: `PlayAnimation("c_000_mov_idle_00")`
/// with the presenter's 0.25 s fade, on the move's token.
pub(crate) fn play_idle_loop(
    world: &mut World,
    token: Option<PlayerActionToken>,
) -> Option<PlayerActionToken> {
    play(world, IDLE_CLIP, STATE_CLIP_FADE, token)
}

/// `PlayerAvatarPresenter.PlayAnimation(clip, 1)` of a state outside the
/// four above (the entry's `PlayerAvatarRefreshState`), with the view's
/// 0.25 s fade, on the given token or a new Door one.
pub(crate) fn play_state_clip(
    world: &mut World,
    clip: &'static str,
    token: Option<PlayerActionToken>,
) -> Option<PlayerActionToken> {
    play(world, clip, STATE_CLIP_FADE, token)
}

/// `PlayerAvatarView.OnAnimationFinished` for a clip that does not loop:
/// whether `clip` has played to its end on the player's animator. None when
/// the clip is not on the animator (its `ChangeMotion` did not play it).
pub(crate) fn clip_finished(world: &mut World, clip: &str) -> Option<bool> {
    let mut drivers = world.query_filtered::<&AvatarDriver, With<PlayerControlled>>();
    let driver = drivers.iter(world).next()?;
    let (node, player) = (driver.node_of(clip)?, driver.player);
    let animator = world.get::<AnimationPlayer>(player)?;
    animator.animation(node).map(|active| active.is_finished())
}

/// `PlayerAvatarAutoMoveState.Initialize`: `ChangeAnimation(RunMotion)`
/// (`motion_avatar_run`) with the view's default 0.25 s fade.
pub(crate) fn play_run_loop(
    world: &mut World,
    token: Option<PlayerActionToken>,
) -> Option<PlayerActionToken> {
    play(world, AUTO_MOVE_CLIP, STATE_CLIP_FADE, token)
}

fn play(
    world: &mut World,
    clip: &str,
    fade: f32,
    token: Option<PlayerActionToken>,
) -> Option<PlayerActionToken> {
    let mut params = SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver, With<PlayerControlled>>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    let Some(mut driver) = drivers.iter_mut().next() else {
        error!("[site-move] no player driver for {clip}");
        return token;
    };
    let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
        error!("[site-move] player animator missing for {clip}");
        return token;
    };
    let motion = PlayerActionMotion {
        clip,
        speed: 1.0,
        blend: Duration::from_secs_f32(fade),
        blocks_manual_movement: true,
    };
    let result = match token {
        Some(token) => driver
            .play_owned_action(token, motion, &mut graphs, &mut animator, &mut transitions)
            .map(|()| token),
        None => driver.start_action(
            PlayerActionOwner::Door,
            motion,
            &mut graphs,
            &mut animator,
            &mut transitions,
        ),
    };
    match result {
        Ok(token) => {
            info!("[site-move] PlayAnimation({clip}, fade {fade}s, speed 1)");
            Some(token)
        }
        Err(error) => {
            error!("[site-move] PlayAnimation({clip}) refused: {error:?}");
            token
        }
    }
}

/// The move ends in the Idle state, whose clip is already playing: the
/// token returns the animator to locomotion without a new `PlayAnimation`.
pub(crate) fn hand_back(world: &mut World, token: Option<PlayerActionToken>) {
    let Some(token) = token else {
        return;
    };
    let mut drivers = world.query_filtered::<&mut AvatarDriver, With<PlayerControlled>>();
    if let Some(mut driver) = drivers.iter_mut(world).next() {
        if driver.hand_back(token) {
            info!("[site-move] the move ends in the Idle state: its clip keeps playing");
        }
    }
}

/// The move is over: the idle crossfade of `Finish` and the token returns
/// the animator to locomotion.
pub(crate) fn finish_idle(world: &mut World, token: Option<PlayerActionToken>, fade: f32) {
    let mut params = SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver, With<PlayerControlled>>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    if let Some(mut driver) = drivers.iter_mut().next() {
        if let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) {
            driver.play_idle(
                token,
                Duration::from_secs_f32(fade),
                &mut graphs,
                &mut animator,
                &mut transitions,
            );
        }
    }
}
