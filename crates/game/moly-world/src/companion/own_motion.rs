//! A companion's own motion: the motions of the shared motion library its
//! character plays as a cast member, played on the companion's SD body by the
//! body's own animator ([`crate::player_avatar::oc::own`]).
//!
//! The body is the character model the companion wears, the model whose
//! wearer is the companion; it has its own animator once its unit's
//! segments are in. The switch between the body's own motion and the
//! avatar's clips is [`super::walk::drive_clips`]'s.
//!
//! | companion state                      | motion on the body                          |
//! |--------------------------------------|---------------------------------------------|
//! | Idle, Done, Failed, Turning, Walking | the avatar's idle or walk clip, retargeted   |
//! | Acting                               | the motion asked for ([`super::act`]), on its own animator |
//! | Using                                | the fixture action's timeline ([`super::use_fixture`]), on its own animator |
//! | Building, its swing in flight        | the avatar's build swing                    |
//!
//! The walk is the avatar's walk clip at the player's Move pace: a companion
//! walks as fast as the player, and the clip's stride is authored for that
//! pace, where the character's own walk is authored for its own, slower one.

use crate::player_avatar::oc::own::{Family, OwnAnimator};
use crate::player_avatar::oc::OcModel;
use bevy::prelude::*;

/// The character body the companion `wearer` wears, once the body has its
/// own animator.
pub fn own_body(bodies: &Query<(Entity, &OcModel), With<OwnAnimator>>, wearer: Entity) -> Option<Entity> {
    bodies
        .iter()
        .find(|(_, model)| model.player == wearer)
        .map(|(body, _)| body)
}

/// The avatar clip family a companion's state asks for: the walk while its
/// walk is in flight, the idle otherwise.
pub fn family(state: &super::CompanionState) -> Family {
    if *state == super::CompanionState::Walking {
        Family::Walk
    } else {
        Family::Idle
    }
}
