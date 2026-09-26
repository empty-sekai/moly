//! The edit game state's entry and exit (`EditGameState.OnEnter` and
//! `OnExit`) around the product's edit session.
//!
//! `OnEnter`, in order: `ChangeGameState(2)` is published; `ShowGrid` (event
//! 15, `ShowGridEventData(2)`); every player avatar presenter hides; every
//! NPC presenter hides and cancels its objective; `HideTweets` (event 27);
//! `FieldCamera.ChangeState(FloorEdit)`; the post process's fog is disabled.
//! `OnExit`: `HideGrid` (event 16); the local player shows; the other players
//! show unless they are this player, not in the room, on another site or in
//! state 11, 15 or 18; the NPCs show; `ShowTweets` (event 26);
//! `SiteObjectManager.ShowAll`; the fog is enabled again.
//!
//! Here:
//! - Players and NPCs: the edit actor overlay (`actors`) hides every
//!   character actor and cancels ordinary NPC objectives on entry, and its
//!   restore shows them; there are no remote players.
//! - Tweets: the HUD's subscriber hides (and later shows) every tweet head-up
//!   display. The same overlay hides the balloon anchors and emote draws and
//!   gives back their previous visibility. The instrument counts them.
//! - Camera: `floor_edit_camera`.
//! - Grid: the grid manager's `SetShowGrid(2)` draws the layout grid (line
//!   meshes, tile faces, floating faces and light poles built from its own
//!   prefabs and grid settings). It is not built here: the events are
//!   recorded and nothing is drawn (named gap).
//! - Fog: the weather host disables the fog while the edit session is active
//!   (it reads the session's active flag, which is set and cleared on the
//!   same frames as this entry and exit); the flag here is the record.
//! - `ShowAll`: the site object manager shows the site objects that other
//!   states hid; the product hides none, so there is nothing to show.

use bevy::prelude::*;

/// `ShowGridEventData`'s argument on entry.
const SHOW_GRID_MODE: u8 = 2;

/// The events and flags the edit game state wrote, as the grid and fog
/// readers would see them.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub(crate) struct EditGameState {
    /// `ShowGrid` mode while shown (event 15), `None` after `HideGrid` (16).
    pub grid: Option<u8>,
    /// `MysekaiRenderSettings.PostProcess.IsDisableFog`.
    pub fog_disabled: bool,
    /// Tweets hidden by event 27, shown again by event 26.
    pub tweets_hidden: bool,
}

/// `EditGameState.OnEnter`, after the edit session began (its actor overlay
/// is in place).
pub(super) fn enter(world: &mut World) {
    info!("[edit-state] EditGameState.OnEnter: ChangeGameState(2) published");
    set(world, |state| state.grid = Some(SHOW_GRID_MODE));
    info!("[edit-state] ShowGrid (event 15, ShowGridEventData({SHOW_GRID_MODE})): recorded; the layout grid is not drawn (grid manager not built)");
    info!("[edit-state] players Hide, NPCs HideAndCancelObjective: the edit actor overlay");
    set(world, |state| state.tweets_hidden = true);
    info!("[edit-state] HideTweets (event 27): tweet balloons and emotes under the edit actor overlay");
    match crate::floor_edit_camera::enter(world) {
        Ok(()) => {}
        Err(reason) => warn!("[edit-state] FieldCamera.ChangeState(FloorEdit) not taken: {reason}"),
    }
    set(world, |state| state.fog_disabled = true);
    info!("[edit-state] PostProcess.IsDisableFog = true: the weather host reads the active edit session");
}

/// `EditGameState.OnExit`, after the session ended (actors restored), and
/// then Normal's entry for the camera.
pub(super) fn exit(world: &mut World) {
    if !world
        .get_resource::<EditGameState>()
        .is_some_and(|state| state.grid.is_some() || state.tweets_hidden || state.fog_disabled)
    {
        return;
    }
    info!("[edit-state] EditGameState.OnExit");
    set(world, |state| state.grid = None);
    info!("[edit-state] HideGrid (event 16): recorded");
    info!("[edit-state] local player Show, NPCs Show: the edit actor overlay's restore");
    set(world, |state| state.tweets_hidden = false);
    info!("[edit-state] ShowTweets (event 26): the overlay gave back the balloons' visibility");
    info!("[edit-state] SiteObjectManager.ShowAll: no site object is hidden in the product; nothing to show");
    set(world, |state| state.fog_disabled = false);
    info!("[edit-state] PostProcess.IsDisableFog = false: the edit session is no longer active");
    crate::floor_edit_camera::leave(world);
}

/// The session was cleared by a site change: the same bookkeeping, and the
/// camera only returns to Normal (the new site frames it).
pub(super) fn exit_for_site_change(world: &mut World) {
    if world
        .get_resource::<EditGameState>()
        .is_some_and(|state| state.grid.is_some() || state.tweets_hidden || state.fog_disabled)
    {
        info!("[edit-state] EditGameState.OnExit for a site change");
        world.insert_resource(EditGameState::default());
    }
    crate::floor_edit_camera::leave_for_site_change(world);
}

fn set(world: &mut World, write: impl FnOnce(&mut EditGameState)) {
    let mut state = world
        .get_resource::<EditGameState>()
        .copied()
        .unwrap_or_default();
    write(&mut state);
    world.insert_resource(state);
}

/// Instrument (`MOLY_EDIT_AUTOPLAY`): after visibility propagation, how many
/// tweet balloons and emotes exist and how many are visible: every two
/// seconds, and on every change of the recorded state and each second for
/// three seconds after it.
#[derive(Default)]
pub(crate) struct TweetSampler {
    key: Option<(bool, Option<u8>, bool)>,
    changed_at: f32,
    last: Option<f32>,
}

pub(crate) fn sample_tweets(
    time: Res<Time>,
    state: Option<Res<EditGameState>>,
    balloons: Query<&InheritedVisibility, With<crate::balloon::BalloonAnchor>>,
    emotes: Query<&InheritedVisibility, With<crate::emoticon::EmoteDraw>>,
    mut run: Local<TweetSampler>,
) {
    if std::env::var("MOLY_EDIT_AUTOPLAY").is_err() {
        return;
    }
    let state = state.as_deref().copied().unwrap_or_default();
    let key = (state.tweets_hidden, state.grid, state.fog_disabled);
    let now = time.elapsed_secs();
    if run.key != Some(key) {
        run.key = Some(key);
        run.changed_at = now;
        run.last = None;
    }
    let period = if now - run.changed_at <= 3.0 {
        1.0
    } else {
        2.0
    };
    if run.last.is_some_and(|last| now - last < period) {
        return;
    }
    run.last = Some(now);
    let visible = |list: Vec<bool>| (list.len(), list.iter().filter(|v| **v).count());
    let (balloon_total, balloon_visible) = visible(balloons.iter().map(|v| v.get()).collect());
    let (emote_total, emote_visible) = visible(emotes.iter().map(|v| v.get()).collect());
    info!(
        "[edit-state] sample {:.1}s after the change: tweets hidden {} grid {:?} fog disabled {}; balloons {balloon_visible}/{balloon_total} visible, emotes {emote_visible}/{emote_total} visible",
        now - run.changed_at,
        state.tweets_hidden,
        state.grid,
        state.fog_disabled
    );
}
