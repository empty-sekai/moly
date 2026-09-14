//! SD presentation on the existing logical player, without creating another NPC.
//!
//! Character model/rig/target installation is shared with `character`. Players
//! receive this driver, never NPC MotionDriver. Movement rules and AvatarRoot
//! remain on PlayerControlled; there is one visible body and one animator.
//! Special actions require an explicit SD clip mapping, never an idle fallback
//! or an invisible audience actor.

pub(crate) mod switch_gesture;

use crate::npc::MotionPhase;
use crate::player::{DashMode, PlayerControlled};
use bevy::animation::{AnimationClip, RepeatAnimation, graph::AnimationNodeIndex};
use bevy::prelude::*;
use std::{collections::HashMap, time::Duration};

const SEGMENT_BLEND: Duration = Duration::from_millis(250);

#[derive(Component, Clone, Debug)]
pub(crate) struct PlayerVisualClips {
    pub idle: String,
    pub walk: String,
    pub run: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AvatarMotion {
    Idle,
    Walk,
    Run,
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

pub(crate) struct PlayerActionMotion<'a> {
    /// Explicit playable SD clip; named product adaptations are not source aliases.
    pub clip: &'a str,
    pub looping: bool,
    pub speed: f32,
    pub blend: Duration,
    pub blocks_manual_movement: bool,
}

#[derive(Debug)]
pub(crate) enum PlayerMotionError {
    Owned(PlayerActionOwner),
    StaleOwner,
    UnmappedSdClip(String),
    MissingGraph,
    InvalidSpeed,
}

struct OwnedAction {
    token: PlayerActionToken,
    node: AnimationNodeIndex,
    looping: bool,
    blocks_manual_movement: bool,
}

/// One driver for the actual player model. The common SD rig wiring supplies
/// the existing animator, graph and model root; no hidden body is attached.
#[derive(Component)]
pub struct AvatarDriver {
    pub(crate) player: Entity,
    pub(crate) visual_root: Entity,
    graph: Handle<AnimationGraph>,
    locomotion: [AnimationNodeIndex; 3],
    clip_names: [String; 3],
    nodes: HashMap<String, AnimationNodeIndex>,
    sd_clips: HashMap<String, Handle<AnimationClip>>,
    playing: Option<AvatarMotion>,
    action: Option<OwnedAction>,
    next_generation: u64,
    installed_at: f32,
    probed: bool,
}

impl AvatarDriver {
    pub(crate) fn new_sd(
        player: Entity,
        visual_root: Entity,
        graph: Handle<AnimationGraph>,
        locomotion: [AnimationNodeIndex; 3],
        clip_names: [String; 3],
        sd_clips: HashMap<String, Handle<AnimationClip>>,
        installed_at: f32,
    ) -> Self {
        let nodes = clip_names.iter().cloned().zip(locomotion).collect();
        Self {
            player,
            visual_root,
            graph,
            locomotion,
            clip_names,
            nodes,
            sd_clips,
            playing: None,
            action: None,
            next_generation: 0,
            installed_at,
            probed: false,
        }
    }

    fn slot(motion: AvatarMotion) -> usize {
        match motion {
            AvatarMotion::Idle => 0,
            AvatarMotion::Walk => 1,
            AvatarMotion::Run => 2,
        }
    }

    pub(crate) fn locomotion_owns_animator(&self) -> bool {
        self.action.is_none()
    }

    pub(crate) fn blocks_manual_movement(&self) -> bool {
        self.action
            .as_ref()
            .is_some_and(|action| action.blocks_manual_movement)
    }

    /// Inspect the installed SD animator without acquiring it or playing a node.
    pub(crate) fn fixture_timeline_binding(&self) -> (Entity, Handle<AnimationGraph>) {
        (self.player, self.graph.clone())
    }

    /// The source activity owns approach/attachment; the timeline then owns
    /// sampling this same animator, rather than starting a second body clock.
    pub(crate) fn acquire_fixture_timeline(&mut self) -> Result<PlayerActionToken, PlayerMotionError> {
        if let Some(action) = &self.action {
            return Err(PlayerMotionError::Owned(action.token.owner));
        }
        self.next_generation = self.next_generation.checked_add(1)
            .expect("player action generation exhausted");
        let token = PlayerActionToken {
            owner: PlayerActionOwner::FixtureTimeline,
            generation: self.next_generation,
        };
        self.action = Some(OwnedAction {
            token,
            // Lease only: this method never starts the locomotion node.
            node: self.locomotion[0],
            looping: true,
            blocks_manual_movement: true,
        });
        self.playing = None;
        Ok(token)
    }

    pub(crate) fn owns_fixture_timeline(&self, token: PlayerActionToken) -> bool {
        token.owner == PlayerActionOwner::FixtureTimeline
            && self.action.as_ref().is_some_and(|action| action.token == token)
    }

    pub(crate) fn release_fixture_timeline(
        &mut self,
        token: PlayerActionToken,
        animator: &mut AnimationPlayer,
    ) -> bool {
        self.owns_fixture_timeline(token) && self.release_action(token, animator)
    }

    fn action_node(
        &mut self,
        clip: &str,
        graphs: &mut Assets<AnimationGraph>,
    ) -> Result<AnimationNodeIndex, PlayerMotionError> {
        let handle = self
            .sd_clips
            .get(clip)
            .ok_or_else(|| PlayerMotionError::UnmappedSdClip(clip.to_owned()))?;
        if let Some(node) = self.nodes.get(clip) {
            return Ok(*node);
        }
        let graph = graphs
            .get_mut(&self.graph)
            .ok_or(PlayerMotionError::MissingGraph)?;
        let node = graph.add_clip(handle.clone(), 1.0, graph.root);
        self.nodes.insert(clip.to_owned(), node);
        Ok(node)
    }

    /// Resolve first, then acquire. An unmapped action leaves locomotion intact.
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
        let node = self.action_node(motion.clip, graphs)?;
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("player action generation exhausted");
        let token = PlayerActionToken {
            owner,
            generation: self.next_generation,
        };
        self.play_action_node(token, node, motion, animator, transitions);
        Ok(token)
    }

    /// Explicit phase changes/replays restart even the same clip; locomotion's
    /// same-kind early return does not apply to a business action.
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
        let node = self.action_node(motion.clip, graphs)?;
        self.play_action_node(token, node, motion, animator, transitions);
        Ok(())
    }

    fn play_action_node(
        &mut self,
        token: PlayerActionToken,
        node: AnimationNodeIndex,
        motion: PlayerActionMotion<'_>,
        animator: &mut AnimationPlayer,
        transitions: &mut AnimationTransitions,
    ) {
        let animation = transitions.play(animator, node, motion.blend);
        animation.set_repeat(if motion.looping {
            RepeatAnimation::Forever
        } else {
            RepeatAnimation::Never
        });
        animation.set_speed(motion.speed);
        self.action = Some(OwnedAction {
            token,
            node,
            looping: motion.looping,
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

pub fn drive(
    mut players: Query<(&MotionPhase, &DashMode, &mut AvatarDriver), With<PlayerControlled>>,
    mut animators: Query<&mut AnimationPlayer>,
    mut transitions: Query<&mut AnimationTransitions>,
) {
    for (phase, dash, mut driver) in &mut players {
        if !driver.locomotion_owns_animator() {
            continue;
        }
        let motion = match phase {
            MotionPhase::Walking => {
                if dash.0 {
                    AvatarMotion::Run
                } else {
                    AvatarMotion::Walk
                }
            }
            MotionPhase::Dwelling { .. } => AvatarMotion::Idle,
            MotionPhase::Turning { .. }
            | MotionPhase::FitTurning { .. }
            | MotionPhase::FitWalking { .. } => {
                unreachable!("player locomotion does not use NPC navigation phases")
            }
        };
        if driver.playing == Some(motion) {
            continue;
        }
        let slot = AvatarDriver::slot(motion);
        let mut animator = animators
            .get_mut(driver.player)
            .expect("SD player animator must remain installed");
        let mut transition = transitions
            .get_mut(driver.player)
            .expect("SD player transitions must remain installed");
        transition
            .play(&mut animator, driver.locomotion[slot], SEGMENT_BLEND)
            .repeat();
        driver.playing = Some(motion);
        info!(
            "[player] SD motion {:?}: {}",
            motion, driver.clip_names[slot]
        );
    }
}

pub fn probe_playback(
    time: Res<Time>,
    mut players: Query<&mut AvatarDriver>,
    animators: Query<&AnimationPlayer>,
) {
    for mut driver in &mut players {
        if driver.probed || time.elapsed_secs() - driver.installed_at < 2.0 {
            continue;
        }
        let animator = animators
            .get(driver.player)
            .expect("SD player animator must remain installed");
        for (node, animation) in animator.playing_animations() {
            info!(
                "[player] SD animation: node={node:?} weight {:.2} elapsed {:.2}s",
                animation.weight(),
                animation.elapsed()
            );
        }
        driver.probed = true;
    }
}
