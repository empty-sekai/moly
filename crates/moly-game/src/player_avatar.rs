//! The player's body and its motion group: the avatar model, drawn on its own
//! skeleton, playing its own clips by the names the source passes.
//!
//! The source's player (`PlayerAvatarView`, an `AvatarBase`) owns one
//! animator. Every player state starts a clip through
//! `PlayerAvatarView.PlayAnimation(name, fadeTime = 0.25, speed = 1)`:
//! `AnimationSpeed = speed`, then `AvatarBase.ChangeMotion(name, fadeTime)`,
//! which registers the clip on first use, sets the layer speed back to 1 and
//! crossfades to it over `fadeTime`, looping as the clip's own loop flag says
//! (`FollowAnimationClip`). The speed argument is therefore overwritten; the
//! states that want another speed set it after the call (`UpdateAnimationSpeed`
//! each frame in Move and Dash, `SetAnimationSpeed` in harvest).
//!
//! Clip names: the source passes the clip's asset name, the lower-case file
//! name of the `.anim` in the motion bundles; the model file carries each
//! clip under its serialized name, whose lower case is that file name for
//! every clip of the group. The driver resolves a source literal by it.
//!
//! The body is one input ([`body::PlayerBody`]): the model file and its
//! motion manifest. Nothing in the state machine names the model, so another
//! body only changes that input. Locomotion states and their clips:
//!
//! | state (source)  | clip                        | master avatar motion |
//! |-----------------|-----------------------------|----------------------|
//! | Idle            | `c_000_mov_idle_00`         | 42 MysekaiIdleMotion |
//! | Move            | `mov_u000_site_walk001_o`   | 44 MysekaiWalkMotion |
//! | Dash            | `mov_u000_site_run001_o`    | 43 MysekaiDashMotion |
//! | AutoMove        | `motion_avatar_run`         | 37 RunMotion         |
//!
//! Business actions (doors, the entry, the cannon, harvest, gimmicks) take the
//! animator with a token and play their own source clips on it.

pub(crate) mod body;
pub(crate) mod switch_gesture;

use crate::npc::MotionPhase;
use crate::player::{DashMode, PlayerControlled};
use bevy::animation::{graph::AnimationNodeIndex, AnimationClip, RepeatAnimation};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc, time::Duration};

/// `PlayerAvatarView.PlayAnimation`'s default `fadeTime` (and the literal the
/// presenter's `PlayAnimation(name, speed)` passes).
pub(crate) const STATE_FADE: Duration = Duration::from_millis(250);

/// Idle state clip (`PlayerAvatarIdleState.Initialize`).
pub(crate) const IDLE_CLIP: &str = "c_000_mov_idle_00";
/// Move state clip (`AvatarConfig.MysekaiWalkMotion`).
pub(crate) const WALK_CLIP: &str = "mov_u000_site_walk001_o";
/// Dash state clip (`AvatarConfig.MysekaiDashMotion`).
pub(crate) const DASH_CLIP: &str = "mov_u000_site_run001_o";
/// AutoMove state clip (`AvatarConfig.RunMotion`).
pub(crate) const AUTO_MOVE_CLIP: &str = "motion_avatar_run";

/// One clip of the body's motion group, as the model file and its manifest
/// carry it.
#[derive(Clone, Debug)]
pub(crate) struct BodyClip {
    /// The clip's serialized name (the model file's animation name).
    pub name: String,
    pub handle: Handle<AnimationClip>,
    /// Clip length in seconds (`AnimationClip.length`).
    pub length: f32,
    /// The clip's own loop flag (`FollowAnimationClip` follows it).
    pub looping: bool,
    /// Animation events: (time, function name, string parameter).
    pub events: Vec<(f32, String, String)>,
}

/// The body's motion group, keyed by the source literal (lower-case asset
/// name).
#[derive(Clone, Debug, Default)]
pub(crate) struct BodyClips(pub HashMap<String, BodyClip>);

impl BodyClips {
    pub(crate) fn get(&self, literal: &str) -> Option<&BodyClip> {
        self.0.get(&literal.to_lowercase())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Locomotion {
    Idle,
    Move,
    Dash,
}

impl Locomotion {
    fn clip(self) -> &'static str {
        match self {
            Self::Idle => IDLE_CLIP,
            Self::Move => WALK_CLIP,
            Self::Dash => DASH_CLIP,
        }
    }
}

/// Admission and game-state transitions stay in the business controller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlayerActionOwner {
    Conversation,
    Harvest,
    Door,
    Cannon,
    FixtureTimeline,
    SwitchGimmick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PlayerActionToken {
    owner: PlayerActionOwner,
    generation: u64,
}

/// One `PlayAnimation` call of a business action.
pub(crate) struct PlayerActionMotion<'a> {
    /// The source clip literal.
    pub clip: &'a str,
    /// The speed the clip plays at after the call (1 unless the caller sets
    /// another speed after `PlayAnimation`, as harvest does).
    pub speed: f32,
    pub blend: Duration,
    pub blocks_manual_movement: bool,
}

#[derive(Debug)]
pub(crate) enum PlayerMotionError {
    Owned(PlayerActionOwner),
    StaleOwner,
    /// The body's motion group has no clip of this name
    /// (`AvatarBase.ChangeMotion` returns false).
    UnknownClip(String),
    MissingGraph,
    InvalidSpeed,
}

struct OwnedAction {
    token: PlayerActionToken,
    node: AnimationNodeIndex,
    looping: bool,
    blocks_manual_movement: bool,
}

/// The one driver of the player's animator. The body installation supplies
/// the animator, its graph and the motion group; no second body exists.
#[derive(Component)]
pub struct AvatarDriver {
    /// The animator entity (the model root the loader gave the player).
    pub(crate) player: Entity,
    /// The body's scene root under the player entity.
    pub(crate) visual_root: Entity,
    graph: Handle<AnimationGraph>,
    clips: Arc<BodyClips>,
    /// Graph nodes by source literal.
    nodes: HashMap<String, AnimationNodeIndex>,
    /// Source literal of each graph node, for the probe.
    literals: HashMap<AnimationNodeIndex, String>,
    playing: Option<Locomotion>,
    action: Option<OwnedAction>,
    next_generation: u64,
}

impl AvatarDriver {
    pub(crate) fn new(
        player: Entity,
        visual_root: Entity,
        graph: Handle<AnimationGraph>,
        clips: Arc<BodyClips>,
    ) -> Self {
        Self {
            player,
            visual_root,
            graph,
            clips,
            nodes: HashMap::new(),
            literals: HashMap::new(),
            playing: None,
            action: None,
            next_generation: 0,
        }
    }

    pub(crate) fn clips(&self) -> &BodyClips {
        &self.clips
    }

    /// Length of one clip of the motion group (`GetAnimationTime`).
    pub(crate) fn clip_length(&self, literal: &str) -> Option<f32> {
        self.clips.get(literal).map(|clip| clip.length)
    }

    /// The source literal a graph node plays, if the driver added it.
    pub(crate) fn literal_of(&self, node: AnimationNodeIndex) -> Option<&str> {
        self.literals.get(&node).map(String::as_str)
    }

    pub(crate) fn node_of(&self, literal: &str) -> Option<AnimationNodeIndex> {
        self.nodes.get(&literal.to_lowercase()).copied()
    }

    pub(crate) fn locomotion_owns_animator(&self) -> bool {
        self.action.is_none()
    }

    /// The Move or Dash clip locomotion is playing (not Idle, not while a
    /// business action owns the animator): whether it is the Dash clip, its
    /// graph node and its source literal.
    pub(crate) fn gait_clip(&self) -> Option<(bool, AnimationNodeIndex, &'static str)> {
        if self.action.is_some() {
            return None;
        }
        let motion = self.playing?;
        let dash = match motion {
            Locomotion::Move => false,
            Locomotion::Dash => true,
            Locomotion::Idle => return None,
        };
        let node = self.node_of(motion.clip())?;
        Some((dash, node, motion.clip()))
    }

    pub(crate) fn blocks_manual_movement(&self) -> bool {
        self.action
            .as_ref()
            .is_some_and(|action| action.blocks_manual_movement)
    }

    /// Inspect the animator without acquiring it or playing a node.
    pub(crate) fn fixture_timeline_binding(&self) -> (Entity, Handle<AnimationGraph>) {
        (self.player, self.graph.clone())
    }

    /// The source activity owns approach/attachment; the timeline then owns
    /// sampling this same animator, rather than starting a second body clock.
    pub(crate) fn acquire_fixture_timeline(
        &mut self,
    ) -> Result<PlayerActionToken, PlayerMotionError> {
        if let Some(action) = &self.action {
            return Err(PlayerMotionError::Owned(action.token.owner));
        }
        let token = self.next_token(PlayerActionOwner::FixtureTimeline);
        let lease_node = self.nodes.get(IDLE_CLIP).copied().unwrap_or_default();
        self.action = Some(OwnedAction {
            token,
            // Lease only: this method never starts a node.
            node: lease_node,
            looping: true,
            blocks_manual_movement: true,
        });
        self.playing = None;
        Ok(token)
    }

    /// Take the animator for a business action without starting a clip:
    /// the state it enters replays nothing (`ChangeStatus` to the current
    /// state returns at once), so the clip on the animator keeps playing
    /// until the action plays its own.
    pub(crate) fn acquire(
        &mut self,
        owner: PlayerActionOwner,
    ) -> Result<PlayerActionToken, PlayerMotionError> {
        if let Some(action) = &self.action {
            return Err(PlayerMotionError::Owned(action.token.owner));
        }
        let token = self.next_token(owner);
        let lease_node = self.nodes.get(IDLE_CLIP).copied().unwrap_or_default();
        self.action = Some(OwnedAction {
            token,
            node: lease_node,
            looping: true,
            blocks_manual_movement: true,
        });
        self.playing = None;
        Ok(token)
    }

    pub(crate) fn owns_fixture_timeline(&self, token: PlayerActionToken) -> bool {
        token.owner == PlayerActionOwner::FixtureTimeline
            && self
                .action
                .as_ref()
                .is_some_and(|action| action.token == token)
    }

    pub(crate) fn release_fixture_timeline(
        &mut self,
        token: PlayerActionToken,
        animator: &mut AnimationPlayer,
    ) -> bool {
        self.owns_fixture_timeline(token) && self.release_action(token, animator)
    }

    fn next_token(&mut self, owner: PlayerActionOwner) -> PlayerActionToken {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("player action generation exhausted");
        PlayerActionToken {
            owner,
            generation: self.next_generation,
        }
    }

    /// The graph node of a clip, registering it on first use (the first
    /// `ChangeMotion` of a name registers its clip).
    fn node(
        &mut self,
        literal: &str,
        graphs: &mut Assets<AnimationGraph>,
    ) -> Result<(AnimationNodeIndex, bool), PlayerMotionError> {
        let key = literal.to_lowercase();
        let clip = self
            .clips
            .0
            .get(&key)
            .ok_or_else(|| PlayerMotionError::UnknownClip(literal.to_owned()))?;
        let looping = clip.looping;
        if let Some(node) = self.nodes.get(&key) {
            return Ok((*node, looping));
        }
        let handle = clip.handle.clone();
        let graph = graphs
            .get_mut(&self.graph)
            .ok_or(PlayerMotionError::MissingGraph)?;
        let node = graph.add_clip(handle, 1.0, graph.root);
        self.nodes.insert(key.clone(), node);
        self.literals.insert(node, key);
        Ok((node, looping))
    }

    /// Resolve first, then acquire. An unknown clip leaves locomotion intact.
    pub(crate) fn start_action(
        &mut self,
        owner: PlayerActionOwner,
        motion: PlayerActionMotion<'_>,
        graphs: &mut Assets<AnimationGraph>,
        animator: &mut AnimationPlayer,
        transitions: &mut AnimationTransitions,
    ) -> Result<PlayerActionToken, PlayerMotionError> {
        if let Some(action) = &self.action {
            return Err(PlayerMotionError::Owned(action.token.owner));
        }
        if !motion.speed.is_finite() || motion.speed <= 0.0 {
            return Err(PlayerMotionError::InvalidSpeed);
        }
        let (node, looping) = self.node(motion.clip, graphs)?;
        let token = self.next_token(owner);
        self.play_action_node(token, node, looping, motion, animator, transitions);
        Ok(token)
    }

    /// Explicit phase changes/replays restart even the same clip; locomotion's
    /// same-state early return does not apply to a business action.
    pub(crate) fn play_owned_action(
        &mut self,
        token: PlayerActionToken,
        motion: PlayerActionMotion<'_>,
        graphs: &mut Assets<AnimationGraph>,
        animator: &mut AnimationPlayer,
        transitions: &mut AnimationTransitions,
    ) -> Result<(), PlayerMotionError> {
        if !self
            .action
            .as_ref()
            .is_some_and(|action| action.token == token)
        {
            return Err(PlayerMotionError::StaleOwner);
        }
        if !motion.speed.is_finite() || motion.speed <= 0.0 {
            return Err(PlayerMotionError::InvalidSpeed);
        }
        let (node, looping) = self.node(motion.clip, graphs)?;
        self.play_action_node(token, node, looping, motion, animator, transitions);
        Ok(())
    }

    fn play_action_node(
        &mut self,
        token: PlayerActionToken,
        node: AnimationNodeIndex,
        looping: bool,
        motion: PlayerActionMotion<'_>,
        animator: &mut AnimationPlayer,
        transitions: &mut AnimationTransitions,
    ) {
        let animation = transitions.play(animator, node, motion.blend);
        animation.set_repeat(if looping {
            RepeatAnimation::Forever
        } else {
            RepeatAnimation::Never
        });
        animation.set_speed(motion.speed);
        self.action = Some(OwnedAction {
            token,
            node,
            looping,
            blocks_manual_movement: motion.blocks_manual_movement,
        });
        self.playing = None;
    }

    /// A finished clip does not itself release a session that may still await a
    /// door, camera, or another source-side completion.
    pub(crate) fn action_finished(
        &self,
        token: PlayerActionToken,
        animator: &AnimationPlayer,
    ) -> bool {
        self.action.as_ref().is_some_and(|action| {
            action.token == token
                && !action.looping
                && animator
                    .playing_animations()
                    .any(|(node, animation)| *node == action.node && animation.is_finished())
        })
    }

    /// A business action that ends without a clip of its own while the
    /// player's state machine is already Idle: `ChangeStatus(Idle)` on the
    /// Idle state returns at once, so nothing replays and the clip on the
    /// animator keeps playing (a finished one holds its last frame).
    /// Locomotion takes the animator back as the Idle state; its next state
    /// change crossfades from that clip.
    pub(crate) fn hand_back(&mut self, token: PlayerActionToken) -> bool {
        if !self
            .action
            .as_ref()
            .is_some_and(|action| action.token == token)
        {
            return false;
        }
        self.action = None;
        self.playing = Some(Locomotion::Idle);
        true
    }

    pub(crate) fn release_action(
        &mut self,
        token: PlayerActionToken,
        animator: &mut AnimationPlayer,
    ) -> bool {
        if !self
            .action
            .as_ref()
            .is_some_and(|action| action.token == token)
        {
            return false;
        }
        self.action.take().expect("matching owner exists");
        // Old phases may still be fading out on this same animator. Releasing
        // ownership must stop them as well; none may emit a later action event.
        animator.stop_all();
        self.playing = None;
        true
    }

    /// `PlayAnimation(idle, fade)` at the end of a business action: ownership
    /// returns to locomotion and its Idle clip takes over through the given
    /// crossfade instead of a hard stop. Without a live token only the idle
    /// crossfade is played.
    pub(crate) fn play_idle(
        &mut self,
        token: Option<PlayerActionToken>,
        blend: Duration,
        graphs: &mut Assets<AnimationGraph>,
        animator: &mut AnimationPlayer,
        transitions: &mut AnimationTransitions,
    ) {
        if let Some(token) = token {
            if self
                .action
                .as_ref()
                .is_some_and(|action| action.token == token)
            {
                self.action = None;
            }
        }
        if self.action.is_some() {
            return;
        }
        match self.node(IDLE_CLIP, graphs) {
            Ok((node, _)) => {
                transitions.play(animator, node, blend).repeat();
                self.playing = Some(Locomotion::Idle);
                info!(
                    "[player] PlayAnimation({IDLE_CLIP}, fade {:.2}s)",
                    blend.as_secs_f32()
                );
            }
            Err(error) => error!("[player] idle clip refused: {error:?}"),
        }
    }

    pub(crate) fn cancel_action(
        &mut self,
        token: PlayerActionToken,
        animator: &mut AnimationPlayer,
    ) -> bool {
        self.release_action(token, animator)
    }

    pub(crate) fn cancel_for_site_change(&mut self, animator: &mut AnimationPlayer) {
        if let Some(token) = self.action.as_ref().map(|action| action.token) {
            self.cancel_action(token, animator);
        }
    }
}

/// Update: locomotion's state clips. Idle plays `c_000_mov_idle_00`, Move the
/// walk clip, Dash the run clip, each through `PlayAnimation(name, 0.25)`
/// when the state is entered (a state is entered once; the same state does
/// not replay its clip).
pub fn drive(
    mut players: Query<(&MotionPhase, &DashMode, &mut AvatarDriver), With<PlayerControlled>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut animators: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    for (phase, dash, mut driver) in &mut players {
        if !driver.locomotion_owns_animator() {
            continue;
        }
        let motion = match phase {
            MotionPhase::Walking => {
                if dash.0 {
                    Locomotion::Dash
                } else {
                    Locomotion::Move
                }
            }
            MotionPhase::Dwelling { .. } => Locomotion::Idle,
            MotionPhase::Turning { .. }
            | MotionPhase::FitTurning { .. }
            | MotionPhase::FitWalking { .. } => {
                unreachable!("player locomotion does not use NPC navigation phases")
            }
        };
        if driver.playing == Some(motion) {
            continue;
        }
        let Ok((mut animator, mut transitions)) = animators.get_mut(driver.player) else {
            panic!("the player's animator must remain installed");
        };
        let (node, looping) = match driver.node(motion.clip(), &mut graphs) {
            Ok(found) => found,
            Err(error) => {
                error!(
                    "[player] {motion:?} clip {} refused: {error:?}",
                    motion.clip()
                );
                driver.playing = Some(motion);
                continue;
            }
        };
        let animation = transitions.play(&mut animator, node, STATE_FADE);
        animation.set_repeat(if looping {
            RepeatAnimation::Forever
        } else {
            RepeatAnimation::Never
        });
        driver.playing = Some(motion);
        info!(
            "[player] {motion:?} state: PlayAnimation({}, fade 0.25s)",
            motion.clip()
        );
    }
}

/// The player's current clips, read from the animator itself: every playing
/// node, the clip its graph node holds (named by the model file's animation
/// table), weight, seek time, speed and the clip length. One line whenever
/// the heaviest node changes, again 0.3 s later, and every 2 s.
pub fn probe_playback(
    time: Res<Time>,
    frames: Res<bevy::diagnostic::FrameCount>,
    states: Option<Res<crate::player_state::PlayerAvatarStates>>,
    players: Query<(&AvatarDriver, &body::BodyNames), With<PlayerControlled>>,
    animators: Query<(&AnimationPlayer, &AnimationGraphHandle)>,
    graphs: Res<Assets<AnimationGraph>>,
    clips: Res<Assets<AnimationClip>>,
    mut last: Local<(Option<AnimationNodeIndex>, f32, bool)>,
) {
    let Ok((driver, names)) = players.single() else {
        return;
    };
    let Ok((animator, graph_handle)) = animators.get(driver.player) else {
        return;
    };
    let Some(graph) = graphs.get(&graph_handle.0) else {
        return;
    };
    let mut rows: Vec<(AnimationNodeIndex, String, f32, f32, f32, f32, bool)> = Vec::new();
    for (node, active) in animator.playing_animations() {
        let Some(bevy::animation::graph::AnimationNodeType::Clip(handle)) =
            graph.get(*node).map(|n| &n.node_type)
        else {
            continue;
        };
        let name = names
            .0
            .get(&handle.id())
            .cloned()
            .unwrap_or_else(|| "<unnamed>".to_owned());
        let length = clips.get(handle).map_or(f32::NAN, AnimationClip::duration);
        rows.push((
            *node,
            name,
            active.weight(),
            active.seek_time(),
            active.speed(),
            length,
            active.is_finished(),
        ));
    }
    rows.sort_by(|a, b| b.2.total_cmp(&a.2));
    let main = rows.first().map(|row| row.0);
    let now = time.elapsed_secs();
    let changed = main != last.0;
    let settle = !changed && !last.2 && now - last.1 >= 0.3;
    let periodic = !changed && now - last.1 >= 2.0;
    if !(changed || settle || periodic) {
        return;
    }
    if changed {
        *last = (main, now, false);
    } else if settle {
        last.2 = true;
    } else {
        last.1 = now;
    }
    let state = states
        .as_deref()
        .map(|s| format!("{:?}", s.current))
        .unwrap_or_else(|| "-".to_owned());
    let listed: Vec<String> = rows
        .iter()
        .map(|(_, name, weight, seek, speed, length, finished)| {
            format!(
                "{name} w{weight:.2} t{seek:.3}/{length:.3} x{speed:.2}{}",
                if *finished { " finished" } else { "" }
            )
        })
        .collect();
    info!(
        "[avatar-probe] frame {} t {:.3} state {state} {}: {}",
        frames.0,
        now,
        if changed {
            "change"
        } else if settle {
            "settle"
        } else {
            "tick"
        },
        if listed.is_empty() {
            "no clip playing".to_owned()
        } else {
            listed.join(" | ")
        }
    );
}
