//! A body's own motion: the idle and walk segments of the shared motion
//! library, the ones the character plays as a cast member, played on the
//! body's own skeleton.
//!
//! The library's clips bind by the humanoid bone-name chain
//! ([``super::humanoid::motion_bones``]) and by the library's reference root
//! name, as the cast member's skeleton takes them; they are mapped the same
//! way onto the body's nodes ([``super::humanoid::own_bone``]), so a body
//! plays its unit's segments without retargeting: the same segment assets the
//! cast member of the unit plays, in the segment chain the cast member's
//! locomotion driver runs (see [`crate::character::drive`]).
//!
//! The body's animator sits on the body's animation root, the first named
//! node of its scene, with its own graph and transitions; every named node at
//! and below it takes the target of its chain of names, the root's own name
//! replaced by the library's reference root, as a cast member's skeleton
//! takes them ([`crate::character::animation_targets`]). While it owns the body (see
//! [``super::pose``]) the pose pass writes no driven bone and the body draws
//! its own pose; the avatar skeleton keeps playing in parallel, and the body
//! blends back to the retarget pose when the ownership ends.
//!
//! The source's segment change starts nothing for a family already in play:
//! the segment running (a lead-in or its loop) runs on, so a family's chain
//! only goes forward and a segment is never restarted.
//!
//! A one-shot ([`Family::Once`]) plays the segments of a named motion once,
//! in the order [`OnceSegment::Start`], [`OnceSegment::Loop`] and
//! [`OnceSegment::End`], each only when the library has it, and hands back to
//! the idle family once the last one played.
//!
//! The cast's character profiles (the segment families and the walking speed
//! of every unit) are read here once ([`load_profiles`]) and offered to each
//! body that is bound: a body whose unit or segments give it no own animator
//! keeps the avatar's motion, and the refusal names the unit and the segments
//! once.
//!
//! | the body's ask            | the body plays                          |
//! |---------------------------|-----------------------------------------|
//! | its own motion starts     | its family's lead-in, then its loop     |
//! | a one-shot is asked for   | its motion's start, loop, end, then idle |
//! | its own motion ends       | nothing; it blends back to the retarget |

use crate::character::SEGMENT_BLEND;
use crate::ui_inputs::UniTaskDelay;
use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::{AnimatedBy, AnimationClip, RepeatAnimation};
use bevy::asset::LoadState;
use bevy::ecs::system::SystemParam;
use bevy::gltf::{Gltf, GltfNode};
use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

/// An animator a body plays its own clips with, on the body's model entity.
#[derive(Component)]
pub struct OwnAnimator {
    /// The body's scene root, below the avatar skeleton's animator.
    pub root: Entity,
    /// The body's animation root, which carries its animator.
    pub animator: Entity,
    /// The library's reference root name.
    pub reference: String,
    idle: AnimationNodeIndex,
    walk: AnimationNodeIndex,
    /// The lead-ins; absent when the library has no such segment (the
    /// source's segment change then does nothing and goes to the loop).
    idle_s: Option<LeadIn>,
    walk_s: Option<LeadIn>,
    /// The one-shot's segments, when the family asked for is
    /// [`Family::Once`], and the motion they are of.
    once: Option<OnceSegments>,
    once_motion: Option<String>,
    /// The segment in play, and the lead-in's wait when one is.
    playing: Option<MotionKind>,
    /// The segment in play was started once and has not ended: the next
    /// update of the body goes to the segment that follows it.
    stepping: bool,
    lead_in_wait: Option<UniTaskDelay>,
    /// The body's own motion poses it: its clips play (they are paused
    /// while the body is drawn by the avatar's motion).
    pub playing_body: bool,
    /// The family the body is asked for.
    family: Option<Family>,
    /// The segment the next ask starts.
    asked: Option<MotionKind>,
}

/// The graph nodes of the segments a one-shot plays, each present only when
/// the library has it.
#[derive(Clone, Copy)]
struct OnceSegments {
    start: Option<AnimationNodeIndex>,
    loop_: Option<AnimationNodeIndex>,
    end: Option<AnimationNodeIndex>,
}

impl OnceSegments {
    /// The node of a segment that the library holds.
    fn get(self, segment: OnceSegment) -> Option<AnimationNodeIndex> {
        match segment {
            OnceSegment::Start => self.start,
            OnceSegment::Loop => self.loop_,
            OnceSegment::End => self.end,
        }
    }
}

/// A segment of a one-shot, in the order they play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnceSegment {
    Start,
    Loop,
    End,
}

/// A lead-in: its node and the seconds the source waits for it (its length,
/// rounded to the millisecond).
#[derive(Clone, Copy)]
struct LeadIn {
    node: AnimationNodeIndex,
    wait: f32,
}

/// A locomotion family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Idle,
    Walk,
    /// The named motion of [`OwnAnimator::set_once`], played once, then the
    /// idle.
    Once,
}

/// The phase of a family's segment chain: its lead-in, then its loop; or one
/// segment of a one-shot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionKind {
    IdleS,
    Idle,
    WalkS,
    Walk,
    Once(OnceSegment),
}

impl MotionKind {
    /// The loop segment a lead-in hands over to.
    fn loop_segment(self) -> Option<MotionKind> {
        match self {
            MotionKind::IdleS => Some(MotionKind::Idle),
            MotionKind::WalkS => Some(MotionKind::Walk),
            MotionKind::Idle | MotionKind::Walk | MotionKind::Once(_) => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            MotionKind::IdleS => "idle_s",
            MotionKind::Idle => "idle",
            MotionKind::WalkS => "walk_s",
            MotionKind::Walk => "walk",
            MotionKind::Once(OnceSegment::Start) => "once_s",
            MotionKind::Once(OnceSegment::Loop) => "once_l",
            MotionKind::Once(OnceSegment::End) => "once_e",
        }
    }
}

/// The reference root of a library: its one animation root node.
fn reference_root(library: &Gltf, nodes: &Assets<GltfNode>) -> Result<String, String> {
    let roots: Vec<&str> = library
        .nodes
        .iter()
        .filter_map(|handle| nodes.get(handle))
        .filter(|node| node.is_animation_root)
        .map(|node| node.name.as_str())
        .collect();
    match roots.as_slice() {
        [root] => Ok((*root).to_owned()),
        [] => Err("the motion library has no animation root".to_owned()),
        roots => Err(format!("the motion library has {} animation roots", roots.len())),
    }
}

/// The source's lead-in wait: the segment's length, rounded to the
/// millisecond.
fn lead_in_wait(clip_assets: &Assets<AnimationClip>, clip: &Handle<AnimationClip>) -> f32 {
    let seconds = clip_assets.get(clip).map_or(0.0, |clip| clip.duration());
    crate::tween_clock::delay_seconds(f64::from(seconds))
}

impl OwnAnimator {
    /// The animator of the body whose scene is spawned at `root` and the
    /// locomotion segments of the family names `idle` and `walk` (a segment
    /// is the family name plus `_L`, its lead-in the name plus `_S`). `None`
    /// while the motion library is loading; an error when the library, a
    /// segment or the body's animation root is missing.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        commands: &mut Commands,
        server: &AssetServer,
        libraries: &Assets<Gltf>,
        gltf_nodes: &Assets<GltfNode>,
        clip_assets: &Assets<AnimationClip>,
        graphs: &mut Assets<AnimationGraph>,
        names: &Query<&Name>,
        children: &Query<&Children>,
        root: Entity,
        idle: &str,
        walk: &str,
    ) -> Result<Option<Self>, String> {
        let library_handle = server.load::<Gltf>(moly_assets::motion_library());
        if let LoadState::Failed(error) = server.load_state(&library_handle) {
            return Err(format!("the motion library failed to load: {error}"));
        }
        let Some(library) = libraries.get(&library_handle) else {
            return Ok(None);
        };
        let reference = reference_root(library, gltf_nodes)?;
        let idle_clip = format!("{idle}_L");
        let walk_clip = format!("{walk}_L");
        let idle_l = library
            .named_animations
            .get(idle_clip.as_str())
            .ok_or_else(|| format!("the motion library has no segment {idle_clip}"))?;
        let walk_l = library
            .named_animations
            .get(walk_clip.as_str())
            .ok_or_else(|| format!("the motion library has no segment {walk_clip}"))?;
        let mut graph = AnimationGraph::new();
        let idle_node = graph.add_clip(idle_l.clone(), 1.0, graph.root);
        let walk_node = graph.add_clip(walk_l.clone(), 1.0, graph.root);
        let mut lead_in = |clip: &Handle<AnimationClip>| LeadIn {
            node: graph.add_clip(clip.clone(), 1.0, graph.root),
            wait: lead_in_wait(clip_assets, clip),
        };
        let idle_s = library
            .named_animations
            .get(format!("{idle}_S").as_str())
            .map(&mut lead_in);
        let walk_s = library
            .named_animations
            .get(format!("{walk}_S").as_str())
            .map(&mut lead_in);
        let animator = crate::character::animation_root(root, names, children)?;
        let graph = graphs.add(graph);
        // The animator on the animation root, and each named node's target
        // pointing back to it, as the cast member's skeleton is wired.
        for (node, target) in
            crate::character::animation_targets(animator, &Name::new(reference.clone()), names, children)
        {
            commands.entity(node).insert((target, AnimatedBy(animator)));
        }
        commands.entity(animator).insert((
            AnimationPlayer::default(),
            AnimationTransitions::new(),
            AnimationGraphHandle(graph),
        ));
        Ok(Some(Self {
            root,
            animator,
            reference,
            idle: idle_node,
            walk: walk_node,
            idle_s,
            walk_s,
            once: None,
            once_motion: None,
            playing: None,
            stepping: false,
            lead_in_wait: None,
            playing_body: false,
            family: None,
            asked: None,
        }))
    }

    /// The node of a phase.
    fn node(&self, kind: MotionKind) -> AnimationNodeIndex {
        match kind {
            MotionKind::Idle => self.idle,
            MotionKind::Walk => self.walk,
            MotionKind::IdleS => self.idle_s.expect("a lead-in phase has its lead-in").node,
            MotionKind::WalkS => self.walk_s.expect("a lead-in phase has its lead-in").node,
            MotionKind::Once(segment) => self
                .once
                .and_then(|segments| segments.get(segment))
                .expect("a one-shot phase has its segment"),
        }
    }

    /// The motion of the one-shot in play.
    pub fn once_motion(&self) -> Option<&str> {
        self.once_motion.as_deref()
    }

    /// Whether the family asked for is a one-shot: its row was taken and
    /// its segments are in play or have played.
    pub fn plays_once(&self) -> bool {
        self.family == Some(Family::Once)
    }

    /// Whether a one-shot asked for has played out: the body is back at its
    /// idle family after its segments, or has been asked for another family
    /// since (a walk's or the idle's).
    pub fn once_over(&self) -> bool {
        match self.family {
            Some(Family::Once) => {
                matches!(self.playing, Some(MotionKind::Idle | MotionKind::IdleS))
                    && self.asked.is_none()
            }
            Some(Family::Idle | Family::Walk) => !matches!(self.playing, Some(MotionKind::Once(_))),
            None => true,
        }
    }

    /// The one-shot `motion`: its segments added to the graph on their first
    /// use. A motion the library holds no segment of is a refusal.
    pub fn set_once(
        &mut self,
        graphs: &mut Assets<AnimationGraph>,
        library: &Gltf,
        graph: &Handle<AnimationGraph>,
        motion: &str,
    ) -> Result<(), String> {
        let Some(graph) = graphs.get_mut(graph) else {
            return Err(format!("{motion}: the body's animation graph is gone"));
        };
        let graph = graph.into_inner();
        let mut node = |name: String| -> Option<AnimationNodeIndex> {
            let clip = library.named_animations.get(name.as_str())?;
            let existing = graph.graph.node_indices().find(|node| {
                matches!(
                    &graph.graph[*node].node_type,
                    bevy::animation::graph::AnimationNodeType::Clip(handle) if handle == clip
                )
            });
            Some(existing.unwrap_or_else(|| graph.add_clip(clip.clone(), 1.0, graph.root)))
        };
        let segments = OnceSegments {
            start: node(format!("{motion}_S")),
            loop_: node(format!("{motion}_L")),
            end: node(format!("{motion}_E")),
        };
        if segments.start.is_none() && segments.loop_.is_none() && segments.end.is_none() {
            return Err(format!(
                "the motion library has no segment of {motion} ({motion}_S, {motion}_L or {motion}_E)"
            ));
        }
        self.once = Some(segments);
        self.once_motion = Some(motion.to_owned());
        Ok(())
    }

    /// The segment a one-shot plays first: its start, else its loop, else
    /// its end.
    fn once_first(&self) -> MotionKind {
        let segments = self.once.expect("a one-shot in play has its segments");
        let first = if segments.start.is_some() {
            OnceSegment::Start
        } else if segments.loop_.is_some() {
            OnceSegment::Loop
        } else {
            OnceSegment::End
        };
        MotionKind::Once(first)
    }

    /// The segment a one-shot plays after `segment`, if there is one.
    fn once_next(&self, segment: OnceSegment) -> Option<MotionKind> {
        let segments = self.once?;
        let next = match segment {
            OnceSegment::Start if segments.loop_.is_some() => Some(OnceSegment::Loop),
            OnceSegment::Start if segments.end.is_some() => Some(OnceSegment::End),
            OnceSegment::Loop if segments.end.is_some() => Some(OnceSegment::End),
            OnceSegment::Start | OnceSegment::Loop | OnceSegment::End => None,
        };
        next.map(MotionKind::Once)
    }

    fn lead_in(&self, kind: MotionKind) -> Option<LeadIn> {
        match kind {
            MotionKind::IdleS => self.idle_s,
            MotionKind::WalkS => self.walk_s,
            MotionKind::Idle | MotionKind::Walk | MotionKind::Once(_) => None,
        }
    }

    /// The first segment of a family's change: the family's lead-in when
    /// there is one, else its loop; the one-shot's first segment.
    fn start(&self, family: Family) -> MotionKind {
        match family {
            Family::Idle if self.idle_s.is_some() => MotionKind::IdleS,
            Family::Idle => MotionKind::Idle,
            Family::Walk if self.walk_s.is_some() => MotionKind::WalkS,
            Family::Walk => MotionKind::Walk,
            Family::Once => self.once_first(),
        }
    }

    /// Whether the clip of `kind` has played to its end.
    fn finished(player: &AnimationPlayer, node: AnimationNodeIndex) -> bool {
        player
            .playing_animations()
            .any(|(playing, animation)| *playing == node && animation.is_finished())
    }

    /// The segment that follows the one in play: a lead-in hands over to its
    /// loop once its wait is over, a one-shot's segment to the next one once
    /// its clip has played, and a one-shot's last segment to the idle.
    fn next_segment(&mut self, dt: f32, player: &AnimationPlayer) -> Option<MotionKind> {
        let playing = self.playing?;
        // The lead-in's wait: the frame it starts is not counted.
        if let Some(next) = playing.loop_segment() {
            let wait = self
                .lead_in_wait
                .as_mut()
                .expect("a lead-in in play has its wait");
            return wait.advance(dt).then_some(next);
        }
        let MotionKind::Once(segment) = playing else {
            return None;
        };
        if !self.stepping || !Self::finished(player, self.node(playing)) {
            return None;
        }
        Some(self.once_next(segment).unwrap_or(MotionKind::Idle))
    }

    /// Update: each asked segment one step forward, a lead-in handing over to
    /// its loop once its wait is over; every segment through the
    /// transitions. A lead-in plays once: the transition opens it on its
    /// first frame, and its time starts once the body's own motion has been
    /// calibrated and its clips play. The body's clips are paused while the
    /// body is not theirs, so a segment cut short waits on its own frame
    /// until the next ownership.
    ///
    /// A one-shot's segments play once each: the segment first played
    /// through the transitions from its start, and once its clip has ended
    /// the next one, so the following segment's transition starts from the
    /// end frame.
    pub(crate) fn drive(
        clock: Res<crate::npc_clock::NpcClock>,
        mut bodies: Query<&mut OwnAnimator>,
        mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
        held: Query<(), With<crate::fixture_activity_timeline::TimelinePlaybackOwner>>,
    ) {
        let dt = clock.delta();
        for mut body in &mut bodies {
            // A fixture action's timeline plays on the animator: the body's
            // segments wait for it to let go.
            if held.contains(body.animator) {
                continue;
            }
            if let Some(asked) = body.asked.take() {
                body.playing = Some(asked);
                body.stepping = matches!(asked, MotionKind::Once(_));
                if body.stepping {
                    // A one-shot opens on its first frame, its clip rewound
                    // to its start.
                    if let Ok((mut player, mut transitions)) = players.get_mut(body.animator) {
                        transitions
                            .play(&mut player, body.node(asked), SEGMENT_BLEND)
                            .set_repeat(RepeatAnimation::Never)
                            .rewind();
                    }
                    debug!("[oc] own motion {:?}: one-shot {}", body.root, asked.label());
                    continue;
                }
            }
            if body.playing.is_none() {
                continue;
            }
            let Ok((mut player, mut transitions)) = players.get_mut(body.animator) else {
                continue;
            };
            let kind = body.next_segment(dt, &player).unwrap_or_else(|| body.playing.unwrap());
            if body.playing == Some(kind) {
                // A one-shot placed while its body was drawn by another
                // motion waited for the first frame the body owns: it opens
                // now, from its start.
                if let Some(playing @ MotionKind::Once(_)) = body.playing {
                    if !body.stepping && body.playing_body {
                        transitions
                            .play(&mut player, body.node(playing), SEGMENT_BLEND)
                            .set_repeat(RepeatAnimation::Never)
                            .rewind();
                        body.stepping = true;
                    }
                }
                continue;
            }
            let node = body.node(kind);
            let once = matches!(kind, MotionKind::Once(_));
            match body.lead_in(kind) {
                Some(lead) => {
                    transitions
                        .play(&mut player, node, SEGMENT_BLEND)
                        .set_repeat(RepeatAnimation::Never)
                        .rewind();
                    body.lead_in_wait = Some(UniTaskDelay::new(lead.wait));
                }
                None => {
                    // A family's loop segment loops; a one-shot's segment
                    // plays once.
                    let repeat = if once {
                        RepeatAnimation::Never
                    } else {
                        RepeatAnimation::Forever
                    };
                    transitions
                        .play(&mut player, node, SEGMENT_BLEND)
                        .set_repeat(repeat);
                    body.lead_in_wait = None;
                }
            }
            body.stepping = once;
            debug!(
                "[oc] own motion {:?}: {} -> {}",
                body.root,
                body.playing.map_or("none", MotionKind::label),
                kind.label()
            );
            body.playing = Some(kind);
        }
    }
}

/// One unit's locomotion: the family names of its idle and walk segments.
#[derive(Clone, Debug)]
pub struct CharacterProfile {
    pub idle: String,
    pub walk: String,
}

/// The cast's character profiles, by unit (`moly://characters.json`): the
/// segments a character body plays as its own.
#[derive(Resource, Clone, Debug, Default)]
pub struct CharacterProfiles(HashMap<u32, CharacterProfile>);

impl CharacterProfiles {
    /// The profile of a unit.
    pub fn get(&self, unit: u32) -> Option<&CharacterProfile> {
        self.0.get(&unit)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The parts of a registry row this reads.
#[derive(Deserialize)]
struct Registry {
    characters: HashMap<String, RegistryRow>,
}

#[derive(Deserialize)]
struct RegistryRow {
    #[serde(rename = "unitId")]
    unit_id: u32,
    locomotion: RegistryLocomotion,
}

#[derive(Deserialize)]
struct RegistryLocomotion {
    #[serde(rename = "idleMotion")]
    idle_motion: String,
    #[serde(rename = "walkMotion")]
    walk_motion: String,
}

/// Update: the cast's character profiles, once the registry is in; the
/// first reading replaces the one in, and a registry that does not read is
/// reported and read no more this session.
pub fn load_profiles(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<moly_assets::json::JsonAsset>>,
    profiles: Option<Res<CharacterProfiles>>,
    mut handle: Local<Option<Handle<moly_assets::json::JsonAsset>>>,
    mut reported: Local<bool>,
) {
    if *reported || profiles.is_some_and(|profiles| !profiles.is_empty()) {
        return;
    }
    let handle = handle
        .get_or_insert_with(|| server.load(moly_assets::character_registry()))
        .clone();
    if let LoadState::Failed(error) = server.load_state(&handle) {
        error!(
            "[oc] the character registry failed to load: {error}; character bodies keep the avatar's idle and walk"
        );
        *reported = true;
        return;
    }
    if !server.is_loaded_with_dependencies(&handle) {
        return;
    }
    let Some(json) = jsons.get(&handle) else {
        return;
    };
    let registry: Registry = match serde_json::from_str(&json.0) {
        Ok(registry) => registry,
        Err(error) => {
            error!(
                "[oc] the character registry does not read: {error}; character bodies keep the avatar's idle and walk"
            );
            *reported = true;
            return;
        }
    };
    let by_unit: HashMap<u32, CharacterProfile> = registry
        .characters
        .into_values()
        .map(|row| {
            let profile = CharacterProfile {
                idle: row.locomotion.idle_motion,
                walk: row.locomotion.walk_motion,
            };
            (row.unit_id, profile)
        })
        .collect();
    info!(
        "[oc] character registry: {} motions for the character bodies",
        by_unit.len()
    );
    commands.insert_resource(CharacterProfiles(by_unit));
}

/// The offer of a body's own motion, made when the body is bound: its own
/// animator with the idle and walk segments of the unit it is worn for
/// ([`CharacterProfile`]). The body's owner offers its motion through
/// [`PlayOwn`], [`Retail`] and [`EndOwn`]; a body without its segments keeps
/// the avatar's motion, and the refusal names the unit and the segments
/// once.
#[derive(Component, Clone, Debug)]
pub struct OwnOffer {
    /// The unit whose character the body is.
    pub unit: Option<u32>,
    /// The idle and walk family names (`None`: the registry has no row for
    /// the unit).
    pub families: Option<(String, String)>,
}

/// The body's own motion starts, drawn by its own clips: its animator and
/// its segments are in ([`prepare`]).
#[derive(Message, Clone, Copy, Debug)]
pub struct PlayOwn(pub Entity);

/// The body's own motion ends: the body blends back to the avatar's motion
/// (see [``super::pose``]).
#[derive(Message, Clone, Copy, Debug)]
pub struct EndOwn(pub Entity);

/// `ChangeAnimationAsync(exit time, no end motion)` of a family on the
/// body's own animator: with the exit time the family's lead-in plays, then
/// its loop. For [`Family::Once`] the motion is the one the body's
/// [`OnceMotions`] rows hold ([`once`]), and its segments play once, then
/// the idle.
#[derive(Message, Clone, Copy, Debug)]
pub struct Retail {
    pub body: Entity,
    pub family: Family,
    pub exit_time: bool,
}

/// The one-shot motions asked for on each body ([`queue_once`]), by entity
/// and in ask order: a [`Retail`] of [`Family::Once`] takes the oldest row.
/// The motion of the one-shot in play is [`OwnAnimator::once_motion`].
#[derive(Resource, Default)]
pub struct OnceMotions(HashMap<Entity, Vec<String>>);

impl OnceMotions {
    /// Queues the one-shot `motion` of `body`.
    pub fn ask(&mut self, body: Entity, motion: &str) {
        self.0.entry(body).or_default().push(motion.to_owned());
    }

    /// The oldest row of `body`, kept.
    pub fn first(&self, body: Entity) -> Option<&String> {
        self.0.get(&body)?.first()
    }

    /// The bodies with a row waiting.
    pub fn bodies(&self) -> Vec<Entity> {
        self.0.keys().copied().collect()
    }

    /// Takes the oldest row of `body`.
    pub fn take(&mut self, body: Entity) -> Option<String> {
        let rows = self.0.get_mut(&body)?;
        if rows.is_empty() {
            self.0.remove(&body);
            return None;
        }
        let first = rows.remove(0);
        if rows.is_empty() {
            self.0.remove(&body);
        }
        Some(first)
    }
}

/// Queues the one-shot `motion` of the body `body` ([`OnceMotions`]), for
/// its next [`Retail`] of [`Family::Once`]:
/// `commands.run_system_cached_with(own::queue_once, (body, motion))`. The
/// segments are added to the body's graph when the offer of that [`Retail`]
/// takes the row.
pub fn queue_once(In((body, motion)): In<(Entity, String)>, mut rows: ResMut<OnceMotions>) {
    rows.ask(body, &motion);
}

fn unit_name(unit: Option<u32>) -> String {
    unit.map_or_else(|| "?".to_owned(), |unit| unit.to_string())
}

/// Update, after the binding: each offered body gets its own animator once
/// the motion library is in. A body whose unit or segments cannot give it
/// one is refused once, naming them, and keeps the avatar's motion.
#[allow(clippy::too_many_arguments)]
pub fn prepare(
    mut commands: Commands,
    server: Res<AssetServer>,
    libraries: Res<Assets<Gltf>>,
    gltf_nodes: Res<Assets<GltfNode>>,
    clip_assets: Res<Assets<AnimationClip>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    names: Query<&Name>,
    children: Query<&Children>,
    offers: Query<(Entity, &OwnOffer)>,
) {
    for (entity, offer) in &offers {
        let Some((idle, walk)) = offer.families.clone() else {
            error!(
                "[oc] unit {}: the character registry has no idle or walk motion; the body keeps the avatar's motion",
                unit_name(offer.unit)
            );
            commands.entity(entity).remove::<OwnOffer>();
            continue;
        };
        let spawned = OwnAnimator::spawn(
            &mut commands,
            &server,
            &libraries,
            &gltf_nodes,
            &clip_assets,
            &mut graphs,
            &names,
            &children,
            entity,
            &idle,
            &walk,
        );
        match spawned {
            Ok(Some(animator)) => {
                info!("[oc] unit {}: own motion of {idle} and {walk}", unit_name(offer.unit));
                commands.entity(entity).insert(animator).remove::<OwnOffer>();
            }
            // The library is still loading: the offer waits.
            Ok(None) => {}
            Err(reason) => {
                error!(
                    "[oc] unit {}: no own motion ({idle} / {walk}): {reason}; the body keeps the avatar's motion",
                    unit_name(offer.unit)
                );
                commands.entity(entity).remove::<OwnOffer>();
            }
        }
    }
}

/// The reads and the writes a body's own-motion offer works with.
#[derive(SystemParam)]
pub struct OfferBodies<'w, 's> {
    own: Query<'w, 's, &'static mut OwnAnimator>,
    graph_handles: Query<'w, 's, &'static AnimationGraphHandle>,
    owned: Query<'w, 's, Has<super::pose::OcOwn>>,
    started: Query<'w, 's, (), With<OwnAnimator>>,
}

/// Update, with the commands': the bodies' own motions start, ask, and end.
pub fn offer(
    mut commands: Commands,
    mut plays: MessageReader<PlayOwn>,
    mut ends: MessageReader<EndOwn>,
    mut retail: MessageReader<Retail>,
    mut once_motions: ResMut<OnceMotions>,
    server: Res<AssetServer>,
    libraries: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut bodies: OfferBodies,
) {
    let bodies = &mut bodies;
    let (starts, stops): (Vec<Entity>, Vec<Entity>) = (
        plays.read().map(|play| play.0).collect(),
        ends.read().map(|end| end.0).collect(),
    );
    // A body handed its own motion in this pass is owned for the asks read
    // below, though its marker lands with the commands.
    let mut handed = Vec::new();
    for body in starts {
        if bodies.started.get(body).is_err() {
            error!("[oc] {body:?}: the body's own motion cannot play: the body has no own animator");
            continue;
        }
        commands.entity(body).insert(super::pose::OcOwn);
        handed.push(body);
    }
    let owned = |bodies: &OfferBodies, body: Entity| {
        handed.contains(&body) || bodies.owned.get(body).unwrap_or(false)
    };
    for body in stops {
        // The ownership ends: the next start is a new one.
        if bodies.started.get(body).is_ok() {
            commands
                .entity(body)
                .remove::<super::pose::OcOwn>()
                .insert(super::pose::OcOwnEnd);
        }
    }
    let library = server.load::<Gltf>(moly_assets::motion_library());
    let library = libraries.get(&library);
    // Every ask of this frame, in the order they were written, read once.
    let written: Vec<Retail> = retail.read().copied().collect();
    // A waiting one-shot row is the ask of its body: it waits until the
    // library is in and the body plays its own motion, then its segments
    // join the body's graph and play from the first, whatever family was in
    // play. A row the library cannot give is dropped by name.
    for entity in once_motions.bodies() {
        let Some(library) = library else {
            continue;
        };
        if !owned(bodies, entity) {
            continue;
        }
        let Ok(mut body) = bodies.own.get_mut(entity) else {
            continue;
        };
        let Ok(graph) = bodies.graph_handles.get(body.animator) else {
            continue;
        };
        let Some(motion) = once_motions.take(entity) else {
            continue;
        };
        match body.set_once(&mut graphs, library, graph, &motion) {
            Ok(()) => {
                let next = body.once_first();
                body.family = Some(Family::Once);
                body.asked = Some(next);
            }
            Err(reason) => error!("[oc] {entity:?}: one-shot {motion} dropped: {reason}"),
        }
    }
    for ask in written.iter().filter(|ask| ask.family != Family::Once) {
        let (family, exit_time) = (ask.family, ask.exit_time);
        // A body drawn by the avatar's motion asks nothing: its own animator
        // keeps its last segment, whose first frame the next ownership
        // starts from.
        if !owned(bodies, ask.body) {
            continue;
        }
        let Ok(mut body) = bodies.own.get_mut(ask.body) else {
            continue;
        };
        // A family in play keeps its chain; a new one starts with its
        // lead-in, or with its loop; a one-shot plays its segments from the
        // first whatever family was in play.
        let next = match (body.family, family) {
            (Some(playing), _) if playing == family => continue,
            _ if exit_time => body.start(family),
            _ => match family {
                Family::Idle => MotionKind::Idle,
                Family::Walk => MotionKind::Walk,
                Family::Once => continue,
            },
        };
        body.family = Some(family);
        body.asked = Some(next);
    }
}
