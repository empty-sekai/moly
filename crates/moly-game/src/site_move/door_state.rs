//! The four house and room player states the door moves (and the entry's
//! house exit) change into, as one shared entry point: `ChangeState(state)`
//! runs `ChangeStatus`, whose intercept gate may drop the request, then the
//! state's `Initialize`: the gate closes and the state clip plays through
//! `PlayerAvatarPresenter.PlayAnimation`, crossfading 0.25 s at speed 1 (the
//! float each state passes there is an animation speed that
//! `AvatarBase.ChangeMotion` resets to 1). The timers of the states'
//! `UpdateState` are in `player_state::update_house_states`.
//!
//! The visible player is the SD body, which has none of the u000 house
//! clips; the SD stand-ins are chosen by root motion (the source clip's
//! forward travel at the moment the state or the move acts on it) and play
//! at 1x, the source waits unchanged:
//!
//! | state (source clip)                              | SD stand-in                     | source / SD travel      |
//! |--------------------------------------------------|---------------------------------|-------------------------|
//! | EnterMoveHouse `act_u000_hou_house_open_013_o`   | `mov_cw_all_house_out_inside_O` | 0.946 / 0.898 m at 2.0 s |
//! | ExitMoveHouse `act_u000_hou_house_open_011_o`    | `mov_cw_all_house_in_outside_O` | ends on the locator (1.56 / 0.64 m behind at 0) |
//! | EnterMoveMyRoom `c_000_mov_myroom_action_02`     | `mov_cw_all_house_in_outside_O` | ends on the locator (2.0 / 0.64 m behind at 0) |
//! | ExitMoveMyRoom `act_u000_hou_house_open_023_o`   | `mov_cw_all_house_out_inside_O` | 1.239 / 0.640 m at 1.45 s |

use std::time::Duration;

use bevy::ecs::system::SystemState;
use bevy::prelude::*;

use crate::player::PlayerControlled;
use crate::player_avatar::{
    AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, PlayerVisualClips,
};
use crate::player_state::{PlayerActionState, PlayerAvatarStates};

use super::door_law::STATE_CLIP_FADE;

/// One of the four states, with its source clip and SD stand-in.
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

    /// The SD stand-in (table above).
    pub(crate) fn sd_clip(self) -> &'static str {
        match self {
            Self::EnterMoveHouse | Self::ExitMoveMyRoom => "mov_cw_all_house_out_inside_O",
            Self::ExitMoveHouse | Self::EnterMoveMyRoom => "mov_cw_all_house_in_outside_O",
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
    play(
        world,
        state.sd_clip(),
        false,
        STATE_CLIP_FADE,
        token,
        state.source_clip(),
    )
}

/// `PlayerAvatarIdleState.Initialize`: the idle loop with the presenter's
/// 0.25 s fade, on the move's token.
pub(crate) fn play_idle_loop(
    world: &mut World,
    token: Option<PlayerActionToken>,
) -> Option<PlayerActionToken> {
    let clips = visual_clips(world)?;
    play(
        world,
        &format!("{}_L", clips.idle),
        true,
        STATE_CLIP_FADE,
        token,
        "c_000_mov_idle_00",
    )
}

/// `PlayerAvatarAutoMoveState.Initialize`: the run loop with the same fade.
pub(crate) fn play_run_loop(
    world: &mut World,
    token: Option<PlayerActionToken>,
) -> Option<PlayerActionToken> {
    let clips = visual_clips(world)?;
    play(
        world,
        &format!("{}_L", clips.run),
        true,
        STATE_CLIP_FADE,
        token,
        "auto-move run",
    )
}

fn visual_clips(world: &mut World) -> Option<PlayerVisualClips> {
    let mut players = world.query_filtered::<&PlayerVisualClips, With<PlayerControlled>>();
    let clips = players.iter(world).next().cloned();
    if clips.is_none() {
        error!("[site-move] the player has no SD clip set");
    }
    clips
}

fn play(
    world: &mut World,
    clip: &str,
    looping: bool,
    fade: f32,
    token: Option<PlayerActionToken>,
    source: &str,
) -> Option<PlayerActionToken> {
    let mut params = SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver, With<PlayerControlled>>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    let Some(mut driver) = drivers.iter_mut().next() else {
        error!("[site-move] no SD player driver for {clip}");
        return token;
    };
    let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
        error!("[site-move] SD player animator missing for {clip}");
        return token;
    };
    let motion = PlayerActionMotion {
        clip,
        looping,
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
            info!("[site-move] {source} plays {clip} (crossfade {fade}s, speed 1)");
            Some(token)
        }
        Err(error) => {
            error!("[site-move] {source}: SD clip {clip} refused: {error:?}");
            token
        }
    }
}

/// The move is over: the idle crossfade of `Finish` and the token returns
/// the animator to locomotion.
pub(crate) fn finish_idle(world: &mut World, token: Option<PlayerActionToken>, fade: f32) {
    let mut params = SystemState::<(
        Query<&mut AvatarDriver, With<PlayerControlled>>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut drivers, mut animators) = params.get_mut(world);
    if let Some(mut driver) = drivers.iter_mut().next() {
        if let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) {
            driver.play_idle(
                token,
                Duration::from_secs_f32(fade),
                &mut animator,
                &mut transitions,
            );
        }
    }
}
