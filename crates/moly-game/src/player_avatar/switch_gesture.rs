//! `PlayerAvatarSwitchGimmickState.Initialize`: the switch gesture,
//! `PlayAnimation("c_000_mov_fixture_action_01_01")` on the player's own
//! animator (fade 0.25, speed 1). The source's one-second business interval
//! still owns completion; this module only holds the animator lease.

use super::{AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken, STATE_FADE};
use crate::player::PlayerControlled;
use bevy::{ecs::system::SystemState, prelude::*};

/// The clip the switch state plays.
const CLIP: &str = "c_000_mov_fixture_action_01_01";

#[derive(Clone, Copy)]
pub(crate) struct Lease {
    animator: Entity,
    token: PlayerActionToken,
}

pub(crate) fn start(world: &mut World, actor: Entity) -> Result<Lease, String> {
    if world.get::<PlayerControlled>(actor).is_none() {
        return Err("the switch gesture is the player's".into());
    }
    let mut params = SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    let mut driver = drivers
        .get_mut(actor)
        .map_err(|_| "the player's driver is not installed")?;
    let animator = driver.player;
    let (mut player, mut transitions) = animators
        .get_mut(animator)
        .map_err(|_| "the player's animator is not installed")?;
    let token = driver
        .start_action(
            PlayerActionOwner::SwitchGimmick,
            PlayerActionMotion {
                clip: CLIP,
                speed: 1.0,
                blend: STATE_FADE,
                blocks_manual_movement: true,
            },
            &mut graphs,
            &mut player,
            &mut transitions,
        )
        .map_err(|error| format!("PlayAnimation({CLIP}) refused: {error:?}"))?;
    info!(
        "[fixture-gimmick] SwitchGimmick state: PlayAnimation({CLIP}, fade 0.25s) actor={actor:?} animator={animator:?} token={token:?}"
    );
    Ok(Lease { animator, token })
}

pub(crate) fn is_alive(world: &World, actor: Entity, lease: Lease) -> bool {
    world.get::<AvatarDriver>(actor).is_some_and(|driver| {
        driver.player == lease.animator
            && driver
                .action
                .as_ref()
                .is_some_and(|action| action.token == lease.token)
    }) && world.get::<AnimationPlayer>(lease.animator).is_some()
        && world.get::<AnimationTransitions>(lease.animator).is_some()
}

pub(crate) fn release(world: &mut World, actor: Entity, lease: Lease) {
    let mut params =
        SystemState::<(Query<&mut AvatarDriver>, Query<&mut AnimationPlayer>)>::new(world);
    let (mut drivers, mut animators) = params.get_mut(world);
    let Ok(mut driver) = drivers.get_mut(actor) else {
        return;
    };
    if driver.player != lease.animator {
        return;
    }
    if let Ok(mut player) = animators.get_mut(lease.animator) {
        // release_action checks the exact token before stopping any live node.
        driver.release_action(lease.token, &mut player);
    } else {
        // Same teardown convention as the fixture timeline lease: clear only
        // this token; this inert value creates no ECS body/player.
        driver.release_action(lease.token, &mut AnimationPlayer::default());
    }
}
