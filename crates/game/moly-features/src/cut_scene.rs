//! Starting a cut-scene with a cast, and hearing back from it.
//!
//! A caller fills a [`CutSceneRequest`] (the cast's units, the cut-scene row,
//! the object that places it and its own [`CutSceneCaller`]) and passes it to
//! [`start`]. The cut-scene player the app installed ([`CutScenePlayer`])
//! plays it, and the caller's callbacks run at three points of the play: at
//! the start of the set-up, after the view is disposed, and when the play
//! has returned. The caller's name, options and callbacks travel with the
//! request, so any code can be a caller without the player naming it.
//!
//! Also here: the home screen's start-animation message that starts the home
//! site's performance, the root the cut-scene views are placed under, and
//! the disposal of the avatars a cut-scene created.

use std::borrow::Cow;

use bevy::prelude::*;

pub use crate::fixture_activity_state::CutSceneCastLease;
pub use crate::markers::CutSceneAvatar;

/// Who asks for a cut-scene, how the play treats it, and what it does at
/// each point of the play.
#[derive(Clone, Debug)]
pub struct CutSceneCaller {
    /// The caller's name in the logs.
    pub name: Cow<'static, str>,
    /// The voice bundle directory the cut-scene's voice tracks read
    /// (`<voice_bank>/<timeline>`); none plays no voice.
    pub voice_bank: Option<Cow<'static, str>>,
    /// Whether the cut-scene screen fades in and out over its layer fade
    /// time; false fades at once.
    pub layer_fades: bool,
    /// Whether the site's BGM plays again when the cut-scene ends; false
    /// leaves the cut-scene's BGM playing, for the caller to hand back.
    pub restores_bgm: bool,
    /// Whether the play creates a new NPC for each unit of the cast, which
    /// stays on the site afterwards, instead of taking over or creating an
    /// avatar.
    pub creates_npcs: bool,
    /// Runs at the start of the set-up, before the game enters the
    /// cut-scene state.
    pub on_started: fn(&mut World, &CutSceneReport),
    /// Runs after the view is disposed, before the NPCs, the fixtures and
    /// the player hidden for the play are shown again.
    pub on_ended: fn(&mut World, &CutSceneReport),
    /// Runs when the play has returned: after the screen faded out, or at
    /// once when the load was refused.
    pub on_returned: fn(&mut World, &CutSceneReport),
}

/// A cut-scene with a cast, as a caller asks for it.
#[derive(Clone, Debug)]
pub struct CutSceneRequest {
    pub caller: CutSceneCaller,
    /// The cast's character units.
    pub units: Vec<u32>,
    /// The cut-scene master row.
    pub cut_scene_id: i64,
    /// The master row's timeline bundle name.
    pub timeline: String,
    /// The object whose transform places the cut-scene.
    pub start: Entity,
    /// Whether a unit's NPC present on the site plays its part; otherwise
    /// the play creates an avatar for the unit.
    pub take_over_present_npcs: bool,
    pub hide_player: bool,
    pub show_ui: bool,
}

/// What a caller's callbacks read about the play.
#[derive(Clone, Debug)]
pub struct CutSceneReport {
    pub request: CutSceneRequest,
    /// Every member's character by unit, taken-over NPCs included.
    pub members: Vec<(u32, Entity)>,
    /// The avatars the play created for its members, by unit; the caller
    /// disposes of them ([`dispose_avatar`]).
    pub avatars: Vec<(u32, Entity)>,
    /// The play's result once it is known; an error names why the cut-scene
    /// did not play or stopped.
    pub outcome: Option<Result<(), String>>,
}

/// The installed cut-scene player.
#[derive(Resource, Clone, Copy)]
pub struct CutScenePlayer {
    /// Starts the play of a request, or refuses it with the reason.
    pub start: fn(&mut World, CutSceneRequest) -> Result<(), String>,
}

/// Starts a cut-scene with a cast through the installed [`CutScenePlayer`].
/// Refuses, naming the reason, when no player is installed or the player
/// refuses the request; the caller's callbacks then do not run.
pub fn start(world: &mut World, request: CutSceneRequest) -> Result<(), String> {
    let Some(player) = world.get_resource::<CutScenePlayer>().copied() else {
        return Err(format!(
            "{}: no cut-scene player is installed",
            request.caller.name
        ));
    };
    (player.start)(world, request)
}

/// The systems that advance cut-scenes; a caller's systems that read a
/// play's progress on the same frame run after this set.
#[derive(bevy::ecs::schedule::SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CutSceneSystems;

/// `ScreenLayerMysekaiHome.OnFinishStartAnimation`: the home screen became
/// current (the entry's home setup, a door or cannon arrival at home).
#[derive(Message, Clone, Copy, Debug)]
pub struct HomeScreenStartAnimation {
    pub caller: &'static str,
}

/// The root the cut-scene views are placed under (`SceneMysekai`'s
/// `CutSceneRoot`): an unparented entity at the origin with the default
/// rotation and scale.
#[derive(Resource)]
struct SceneRoot(Entity);

/// The cut-scene root, created on first use and again when it was despawned.
pub fn scene_root(world: &mut World) -> Entity {
    if let Some(root) = world.get_resource::<SceneRoot>().map(|root| root.0) {
        if world.get_entity(root).is_ok() {
            return root;
        }
    }
    let entity = world
        .spawn((
            Name::new("CutSceneRoot"),
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
        ))
        .id();
    world.insert_resource(SceneRoot(entity));
    entity
}

/// Despawns `entity` when it is an avatar a cut-scene created; returns
/// whether it was one.
pub fn dispose_avatar(world: &mut World, entity: Entity) -> bool {
    if world.get::<CutSceneAvatar>(entity).is_none() {
        return false;
    }
    world.despawn(entity);
    true
}
