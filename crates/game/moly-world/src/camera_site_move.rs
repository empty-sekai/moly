//! The camera state the site move and the cut-scene share: the
//! `SiteMoveActionCameraState` private state, and Normal's exit snapshot and
//! entry that both callers run when they switch the camera state themselves.

use bevy::prelude::*;

use crate::camera::{normal_on_enter, normal_on_exit, FieldCameraState};

/// `SiteMoveActionCameraState` private state: `_groundedTweenTime` and the
/// avatar's `Step` (rig `Root`) and `Head` bones (`FindDeep` by name on the
/// avatar view).
#[derive(Resource, Debug)]
pub struct SiteMoveCamera {
    pub grounded: f32,
    pub step: Option<Entity>,
    pub head: Option<Entity>,
    pub missing_logged: bool,
}

/// `NormalCameraState.OnExit` at `site` ([`crate::camera::normal_on_exit`]),
/// for the callers that switch the camera state themselves. False without a
/// camera model.
pub fn record_normal_exit(world: &mut World, site: &str) -> bool {
    normal_on_exit(world, site)
}

/// `NormalGameState.OnEnter` -> `ChangeState(Normal)` from the current
/// camera state, whose own exit the caller has run:
/// `NormalCameraState.OnEnter` ([`crate::camera::normal_on_enter`]). A cannon
/// arrival takes the inherit branch (a harvest or delivery destination, or
/// home after an outdoor site); home after any other site takes
/// `TransferCameraSettings`, whose case 6 (from SiteMoveAction) tweens 0.6 s
/// to Normal's private model with its LookAt taken from the live one. The
/// site, category and previous site type the callers pass are the ones the
/// entry reads from the world.
pub fn enter_normal(world: &mut World, site: &str, _category: &str, _prev_site: &str) {
    world.remove_resource::<SiteMoveCamera>();
    let prev = world.resource::<FieldCameraState>().0;
    normal_on_enter(world, prev, "site-move");
    debug!("[site-move] camera -> Normal at {site} from {prev:?}");
}
