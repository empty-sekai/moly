//! SD character on screen: character bundle/skeleton assembly for roster members
//! and the shared motion library. The player's body does not go through here (see
//! [`crate::player_avatar::body`]).
//!
//! The load gate has the same shape as sites: request → dependencies ready →
//! expand → assemble. Each roster member (an entity of the npc domain) carries its
//! own character bundle scene as a child entity — the member entity holds position
//! and facing and the model scene is its child, so the single write point for
//! position is preserved: the step rules remain the only writer of the npc entity
//! Transform, and this module does not touch member transforms.
//!
//! Animation binding basis: clips are looked up by the bone-name chain
//! ([`AnimationTargetId`]). Below the **reference root**, the shared motion
//! library's reference skeleton and every character bundle skeleton have
//! identical name chains (for every bone the library drives), but the reference
//! root's own name comes from the base model the library was built on, and other
//! bundles have their own root names. Assembly replaces the first link of the
//! chain with the library's reference root name, so clips drive each bundle
//! directly with no retargeting — the root name is read at runtime from the
//! library's [`GltfNode`] (`is_animation_root`, exactly one) and not hard-coded.
//! The glTF loader adds the target components only to scenes that contain an
//! animation; character bundles contain none, so this module adds them following
//! the loader's assembly convention: the player sits on the animation root node,
//! and every bone's target points back to the player.
//!
//! Segment choice: the move phase (the npc domain step rules' output) plays the
//! walk family while moving, the idle family while dwelling and the turn family
//! while turning. Family names: the walk family uses the roster's locomotion
//! columns; the turn family uses the base name chosen by the turn rules (always
//! the `mov_cw_normal` family — the segment constants are fixed that way and do
//! not depend on the character's own walk family).
//!
//! Segment switching follows the engine's switch call
//! (`ChangeAnimationAsync(family, hasExitTime, …, playEndMotion, 0.5)`) chain:
//! - Same family (equal after stripping the `_S`/`_L`/`_E`/`_O` suffixes) returns
//!   immediately — the playing segment is not restarted;
//! - otherwise it first plays the start segment `family_S`, waits
//!   `UniTask.Delay(segment length / speed)` (rounded to milliseconds, DeltaTime,
//!   at the Update timing: the creation frame does not count, afterwards frame
//!   deltas accumulate and the segment is joined on the frame the sum reaches the
//!   segment length), then plays the loop segment `family_L`; when the start
//!   segment is not in the controller neither step happens (no switch is sent and
//!   the wait returns immediately), going straight to `_L`;
//! - every step is `Animator.CrossFadeInFixedTime(segment name, 0.5)`: a
//!   fixed-duration transition, with the target segment starting from 0; a segment
//!   equal to the previous one is not sent.
//! - a new switch call cancels the previous segment chain: an old chain still
//!   waiting for its start segment no longer receives its `_L`.
//!
//! `hasExitTime` is an argument of this switch call, not a controller setting: when
//! it is false and `playEndMotion` is true, the previous family's end segment `_E`
//! plays first and the switch waits for it to finish; when true it is skipped. All
//! movement switch calls play no end segment — walk/idle switches have either
//! `hasExitTime` true or `playEndMotion` false, and the same holds for turns — so
//! there is no `_E` here. SD controllers have no asset-side transitions: each of
//! the 31 controllers has one layer and 1179 states, state transitions and AnyState
//! transitions are all 0, state speeds are all 1, and exit time, transition
//! duration, offset and interruption settings are neither readable nor usable —
//! the only transitions are the dynamic ones started by the code above.
//!
//! Transition weights: the engine's [`AnimationTransitions`] linearly lowers the
//! old segment's weight from 1 to 0 and the new segment takes the remainder, the
//! same shape as CrossFade's linear weights, and that is used here. Not supported:
//! the engine's interruption semantics for a second CrossFade in the middle of a
//! transition (engine behaviour): here, following [`AnimationTransitions`], segments
//! that are fading out are stacked layer by layer.

use crate::ui_inputs::UniTaskDelay;
use crate::npc::{CharacterUnitId, MotionClips, MotionPhase};
use bevy::animation::graph::AnimationNodeIndex;
use bevy::animation::{AnimatedBy, AnimationClip, AnimationTargetId};
use bevy::asset::{AssetPath, Assets, LoadState, RecursiveDependencyLoadState};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::observer::On;
use bevy::gltf::{Gltf, GltfNode};
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;
use bevy::world_serialization::{WorldInstanceReady, WorldAssetRoot};
use moly_assets::character as schema;
use moly_assets::json::JsonAsset;
use moly_law::path::TurnMotion;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// Segment transition duration: the transition argument the switch call passes as a
/// literal (0.5 s), shared by every switch — every change between idle, walk and
/// turn.
pub const SEGMENT_BLEND: Duration = Duration::from_millis(500);

/// 位移换段调用传的播放速度：走姿、待机、转身的调用点都传 1。起始段的
/// 等待是 `段长 / 速度`。
const LOCOMOTION_SPEED: f32 = 1.0;

/// 已请求装载的角色清单（代号 → 包文件）。常驻：名册成员随 spawn 到齐，
/// 计划系统逐名现读。
#[derive(Resource)]
pub struct ManifestAssets {
    json: Handle<JsonAsset>,
}

/// 已请求装载的共享动作库。常驻：装配与换段都从它取段。
#[derive(Resource)]
pub struct MotionLibrary {
    pub gltf: Handle<Gltf>,
}

pub use crate::markers::AvatarRoot;

pub use crate::markers::{CharacterModel, CharacterPack};

/// Imported local pose before any locomotion or content clip is evaluated.
/// Sparse source clips must not inherit transforms left by a previous clip.
#[derive(Component, Clone, Copy)]
pub struct CharacterRestTransform(pub Transform);

/// 角色包已挂载：装载门的一次性闩，防重复挂子场景。
#[derive(Component)]
pub struct ModelAttached;

/// 模型场景已展开；由 [`on_model_scene_ready`] 置位，装配消费后即撤。
#[derive(Component)]
pub struct ModelSceneReady;

/// 位移段种：静止帧待机、移动帧走姿、转体帧转身段，每族起始/循环两相。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    /// 待机起始段（`_S`）：播一遍，等满段长后接 [`Self::Idle`]。
    IdleS,
    /// 待机循环段（`_L`）。
    Idle,
    /// 走姿起始段（`_S`）：播一遍，等满段长后接 [`Self::Walk`]。
    WalkS,
    /// 走姿循环段（`_L`）。
    Walk,
    /// 转身起始段（`_S`）：播一遍，等满段长后接 [`Self::TurnL`]。
    TurnS(TurnMotion),
    /// 转身循环段（`_L`）。
    TurnL(TurnMotion),
}

/// 换段调用的族：段名去掉 `_S`/`_L`/`_E`/`_O` 后缀的那个原名。源的换段
/// 调用对同一原名直接返回，所以族相同即不换段。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MotionFamily {
    Idle,
    Walk,
    Turn(TurnMotion),
}

impl MotionKind {
    fn family(self) -> MotionFamily {
        match self {
            MotionKind::IdleS | MotionKind::Idle => MotionFamily::Idle,
            MotionKind::WalkS | MotionKind::Walk => MotionFamily::Walk,
            MotionKind::TurnS(motion) | MotionKind::TurnL(motion) => MotionFamily::Turn(motion),
        }
    }

    /// 起始段等满段长后接的循环段；循环段返回 `None`。
    fn loop_segment(self) -> Option<MotionKind> {
        match self {
            MotionKind::IdleS => Some(MotionKind::Idle),
            MotionKind::WalkS => Some(MotionKind::Walk),
            MotionKind::TurnS(motion) => Some(MotionKind::TurnL(motion)),
            MotionKind::Idle | MotionKind::Walk | MotionKind::TurnL(_) => None,
        }
    }
}

/// 一段起始段：图节点与源的等待秒数（`TimeSpan.FromSeconds(段长 / 速度)`
/// 的毫秒取整值）。
#[derive(Clone, Copy)]
struct LeadIn {
    node: AnimationNodeIndex,
    wait: f32,
}

/// 一名成员的动画驱动：播放器实体、图里各段的节点与段名、当前在播
/// 段。`playing` 装配时为 `None`——播放器经 commands 插上，装配当帧读
/// 不到，首段起播交给驱动系统。
///
/// While a state other than locomotion plays the animator it holds the
/// body's [`BodyToken`]: locomotion does not change segments until that
/// owner hands the token back. The graph handle and `playing`/`idle`/
/// `player` stay open to those owners (they add nodes by clip name, change
/// clips through the same transitions and return the body to idle).
#[derive(Component)]
pub struct MotionDriver {
    pub player: Entity,
    pub idle: AnimationNodeIndex,
    /// 待机起始段；库里没有该段时为 `None`（源：段不在控制器里，换段与
    /// 等待都不做，直接到循环段）。
    idle_s: Option<LeadIn>,
    walk: AnimationNodeIndex,
    /// 走姿起始段，同上。
    walk_s: Option<LeadIn>,
    /// 六个转身段基名的起始段，下标 = [`TurnMotion::index`]。
    turn_s: Vec<LeadIn>,
    /// 六个转身段基名的循环节点，下标 = [`TurnMotion::index`]。
    turn_l: Vec<AnimationNodeIndex>,
    pub playing: Option<MotionKind>,
    /// 在播起始段的等待（源 `UniTask.Delay`，DeltaTime 型）：起始段起播
    /// 时建、换段时换掉；只在 `playing` 是起始段时被读。
    lead_in_wait: Option<UniTaskDelay>,
    /// 该成员自己的动画图（图与播放器都是实体局部状态，不共享）。
    pub graph: Handle<AnimationGraph>,
    /// The state that plays the animator instead of locomotion, if any.
    held: Option<BodyToken>,
    next_generation: u64,
    /// Clip names already added to this character's graph, by the states that
    /// play it in place of locomotion.
    nodes: HashMap<String, AnimationNodeIndex>,
    idle_clip: String,
    walk_clip: String,
    installed_at: f32,
    probed: bool,
}

/// The states that play an NPC's animator in place of locomotion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyOwner {
    /// An alone action's script (Rest and the idle-action states).
    Alone,
    Communication,
    /// A talk script's motions.
    Talk,
    /// A player talk's motions on its NPC participants.
    PlayerTalk,
    /// An event's or a photo call's clips.
    Event,
    ChangeSite,
    /// A fixture action's director and the states around it.
    FixtureActivity,
    /// A fixture talk's pre-action director.
    FixtureTalk,
    /// A cut-scene's director on its cast.
    CutScene,
    /// A still pose held for a portrait capture.
    Portrait,
    /// A sequence player sampling its tracks at the played time.
    Sequence,
    /// A ragdoll: the body falls as rigid parts until it gets up, and its
    /// animation is handed back when it does.
    Ragdoll,
}

impl BodyOwner {
    /// The owner as a note names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Alone => "an alone action",
            Self::Communication => "a communication",
            Self::Talk => "a talk",
            Self::PlayerTalk => "a player talk",
            Self::Event => "an event",
            Self::ChangeSite => "a site change",
            Self::FixtureActivity => "a fixture activity",
            Self::FixtureTalk => "a fixture talk",
            Self::CutScene => "a cut-scene",
            Self::Portrait => "a portrait",
            Self::Sequence => "a sequence",
            Self::Ragdoll => "a ragdoll",
        }
    }
}

/// The character is falling: its bones' rotations are written by the ragdoll
/// each frame, and passes that solve poses of their own (a look-at) leave it
/// alone until it gets up.
#[derive(Component)]
pub struct Falling;

/// One motion a pose plays: the shared library's clip `clip`, sampled
/// `local` seconds into it at `weight`.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseLayer {
    pub clip: String,
    /// Seconds into the clip; a looping layer takes it modulo the clip's
    /// length, one that plays once holds its last frame past the end.
    pub local: f32,
    pub weight: f32,
    pub looping: bool,
}

/// One owner's hold on an NPC body: valid until the owner hands it back or a
/// later state takes the body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyToken {
    owner: BodyOwner,
    generation: u64,
}

impl BodyToken {
    pub fn owner(self) -> BodyOwner {
        self.owner
    }
}

impl MotionDriver {
    /// The state entering the body takes its animator. As with the source's
    /// `ChangeAnimation`, the newest caller wins: the token it replaces goes
    /// stale and can no longer hand the body back.
    pub fn take(&mut self, owner: BodyOwner) -> BodyToken {
        self.next_generation += 1;
        let token = BodyToken {
            owner,
            generation: self.next_generation,
        };
        self.held = Some(token);
        token
    }

    /// The token a state holds, for a state that keeps its hold in its own
    /// record and hands it back through [`Self::hand_back`].
    pub fn token(&self) -> Option<BodyToken> {
        self.held
    }

    /// [`Self::take`] for a state that keeps its token in `slot` while it
    /// plays several clips in a row. Called where the state enters the body
    /// (its first clip, or a new motion it is told to play), never to carry
    /// on: a state carrying on asks [`Self::keeps`].
    pub fn enter(&mut self, slot: &mut Option<BodyToken>, owner: BodyOwner) -> BodyToken {
        let token = self.take(owner);
        *slot = Some(token);
        token
    }

    /// Whether the token in `slot` still holds the body. A later state that
    /// took the body replaced this one: its run is over, so the slot is
    /// emptied and the caller writes nothing more to the animator (the new
    /// holder plays it, and plays idle when it lets go).
    pub fn keeps(&self, slot: &mut Option<BodyToken>) -> bool {
        match *slot {
            Some(token) if self.holds(token) => true,
            _ => {
                *slot = None;
                false
            }
        }
    }

    pub fn holds(&self, token: BodyToken) -> bool {
        self.held == Some(token)
    }

    /// The owner currently playing the animator instead of locomotion.
    pub fn held_by(&self) -> Option<BodyOwner> {
        self.held.map(BodyToken::owner)
    }

    /// Whether the body is held by the owner `owner`.
    pub fn is_held_by(&self, owner: BodyOwner) -> bool {
        self.held_by() == Some(owner)
    }

    pub fn locomotion_owns(&self) -> bool {
        self.held.is_none()
    }

    /// The owner of `token` lets go. Only the current holder can: a token a
    /// later state replaced returns false and changes nothing. Nothing is
    /// replayed here: the owner's clip keeps playing until locomotion's next
    /// segment change (or the idle the owner plays itself) crossfades from it.
    pub fn hand_back(&mut self, token: BodyToken) -> bool {
        if !self.holds(token) {
            return false;
        }
        self.held = None;
        true
    }

    /// [`Self::hand_back`] for a token kept in `slot`, which is emptied.
    pub fn hand_back_slot(&mut self, slot: &mut Option<BodyToken>) -> bool {
        slot.take().is_some_and(|token| self.hand_back(token))
    }

    /// The graph node of the shared library's clip `clip_name`, added to this
    /// character's graph on first use. A clip already in the graph (an
    /// earlier body of the same graph added it) is reused, not appended.
    pub fn clip_node(
        &mut self,
        graphs: &mut Assets<AnimationGraph>,
        library: &Gltf,
        clip_name: &str,
        unit: u32,
    ) -> AnimationNodeIndex {
        if let Some(node) = self.nodes.get(clip_name) {
            return *node;
        }
        let clip = library
            .named_animations
            .get(clip_name)
            .unwrap_or_else(|| panic!("the shared motion library has no clip {clip_name} (unit {unit})"));
        let graph = graphs
            .get_mut(&self.graph)
            .map(|asset| asset.into_inner())
            .unwrap_or_else(|| panic!("unit {unit}'s animation graph asset is missing"));
        let existing = graph.graph.node_indices().find(|node| {
            matches!(&graph.graph[*node].node_type, bevy::animation::graph::AnimationNodeType::Clip(handle) if handle == clip)
        });
        let node = existing.unwrap_or_else(|| graph.add_clip(clip.clone(), 1.0, graph.root));
        self.nodes.insert(clip_name.to_owned(), node);
        node
    }

    /// The node of `clip_name` if a state already added it to this graph.
    pub fn added_clip_node(&self, clip_name: &str) -> Option<AnimationNodeIndex> {
        self.nodes.get(clip_name).copied()
    }

    /// Poses the animator from the library's clips directly, without the
    /// transitions of locomotion: each layer is an animation of the graph
    /// started at its time and weight, and every animation of the player the
    /// layers do not list is stopped. Only the holder of `token` writes;
    /// anything else is refused with the owner that holds the body.
    ///
    /// The transitions of the player are cleared: their `advance_transitions`
    /// would write the departing animation's fading weight over every weight
    /// of the player each frame. The animations the transitions were fading
    /// stay in the player, paused and unweighted, so the body's own expiry
    /// (`expire_completed_transitions`) never stops a layer this call is about
    /// to write, and the next segment change of locomotion crossfades in from
    /// them. The locomotion ledger closes (`playing` empty), so that change is
    /// taken again.
    #[allow(clippy::too_many_arguments)]
    pub fn pose(
        &mut self,
        token: BodyToken,
        layers: &[PoseLayer],
        graphs: &mut Assets<AnimationGraph>,
        library: &Gltf,
        player: &mut AnimationPlayer,
        transitions: &mut AnimationTransitions,
        clips: &Assets<AnimationClip>,
        unit: u32,
    ) -> Result<(), String> {
        if !self.holds(token) {
            let owner = self.held_by().map_or("no state".to_owned(), |owner| owner.name().to_owned());
            return Err(format!("unit {unit}'s body is held by {owner}"));
        }
        let mut listed: HashSet<AnimationNodeIndex> = HashSet::new();
        for layer in layers {
            if !layer.local.is_finite() || !layer.weight.is_finite() || layer.weight < 0.0 {
                return Err(format!("the layer of {} is sampled at {:.3} s with weight {}", layer.clip, layer.local, layer.weight));
            }
            let node = self.clip_node(graphs, library, &layer.clip, unit);
            listed.insert(node);
            let length = library
                .named_animations
                .get(layer.clip.as_str())
                .and_then(|clip| clips.get(clip))
                .map(AnimationClip::duration)
                .ok_or_else(|| format!("the library clip {} has no length", layer.clip))?;
            let local = if layer.looping && length > 0.0 { layer.local.rem_euclid(length) } else { layer.local.clamp(0.0, length) };
            player.play(node).set_weight(layer.weight).seek_to(local).pause();
        }
        // The transitions cannot fade over a pose no state of theirs plays.
        if let Some(main) = transitions.get_main_animation().filter(|main| !listed.contains(main)) {
            player.stop(main);
        }
        let fading: Vec<AnimationNodeIndex> =
            player.playing_animations().map(|(node, _)| *node).filter(|node| !listed.contains(node)).collect();
        *transitions = AnimationTransitions::new();
        for node in fading {
            player.play(node).set_weight(0.0).pause();
        }
        self.playing = None;
        Ok(())
    }

    fn lead_in(&self, kind: MotionKind) -> Option<LeadIn> {
        match kind {
            MotionKind::IdleS => self.idle_s,
            MotionKind::WalkS => self.walk_s,
            MotionKind::TurnS(motion) => Some(self.turn_s[motion.index()]),
            MotionKind::Idle | MotionKind::Walk | MotionKind::TurnL(_) => None,
        }
    }

    fn node(&self, kind: MotionKind) -> AnimationNodeIndex {
        match kind {
            MotionKind::Idle => self.idle,
            MotionKind::Walk => self.walk,
            MotionKind::TurnL(motion) => self.turn_l[motion.index()],
            MotionKind::IdleS | MotionKind::WalkS | MotionKind::TurnS(_) => {
                self.lead_in(kind)
                    .expect("起始段种只在库里有该段时进入")
                    .node
            }
        }
    }

    /// 换段调用的第一段：族有起始段就从起始段起，否则直接循环段。
    fn start(&self, family: MotionFamily) -> MotionKind {
        match family {
            MotionFamily::Idle if self.idle_s.is_some() => MotionKind::IdleS,
            MotionFamily::Idle => MotionKind::Idle,
            MotionFamily::Walk if self.walk_s.is_some() => MotionKind::WalkS,
            MotionFamily::Walk => MotionKind::Walk,
            MotionFamily::Turn(motion) => MotionKind::TurnS(motion),
        }
    }

    fn clip(&self, kind: MotionKind) -> String {
        let stem = |loop_clip: &str| loop_clip.strip_suffix("_L").unwrap_or(loop_clip).to_owned();
        match kind {
            MotionKind::IdleS => format!("{}_S", stem(&self.idle_clip)),
            MotionKind::Idle => self.idle_clip.clone(),
            MotionKind::WalkS => format!("{}_S", stem(&self.walk_clip)),
            MotionKind::Walk => self.walk_clip.clone(),
            MotionKind::TurnS(motion) => format!("{}_S", motion.base_name()),
            MotionKind::TurnL(motion) => format!("{}_L", motion.base_name()),
        }
    }
}

/// The length in seconds of the shared library's clip `clip_name`, when the
/// library has exactly that clip and it is loaded.
pub fn motion_length(clip_name: &str, library: &Gltf, clips: &Assets<AnimationClip>) -> Option<f32> {
    let clip = library.named_animations.get(clip_name)?;
    clips.get(clip).map(AnimationClip::duration)
}

/// 源起始段的等待秒数：`TimeSpan.FromSeconds(段长 / 速度)` 取整到毫秒后
/// 由 `UniTask.Delay` 存成 float。
fn source_lead_in_seconds(clip_seconds: f32) -> f32 {
    crate::tween_clock::delay_seconds(f64::from(clip_seconds / LOCOMOTION_SPEED))
}

/// 角色网格外壳（米，装配帧的世界系快照）：相机取景偏移的现算量。
#[derive(Component)]
pub struct CharacterShell {
    pub height: f32,
    pub lowest: f32,
}

/// Startup：请求装载角色清单（包索引）与共享动作库。
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(ManifestAssets {
        json: server.load::<JsonAsset>(moly_assets::character_manifest()),
    });
    commands.insert_resource(MotionLibrary {
        gltf: server.load::<Gltf>(moly_assets::motion_library()),
    });
}

/// The character plan's hold: no body is planned until the character manifest
/// has loaded with its dependencies.
const PLAN_HOLD: crate::hold::HoldSite = crate::hold::HoldSite {
    tag: "npc",
    what: "character plans",
    effect: "the NPCs have no body and stay invisible",
    quiet: &[],
};

/// Update：名册成员到齐后逐名发角色包装载。清单的 `unit` 字段是代号
/// （成员 unitId 加 100——两份资产各持一半，清单持包文件、名册持身份），
/// 代码里把两侧折到同一坐标系再对。清单里没有某成员的包即响亮失败。
pub fn plan_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    manifest_files: Res<ManifestAssets>,
    time: Res<Time>,
    npcs: Query<(Entity, &CharacterUnitId), (With<MotionClips>, Without<CharacterPack>)>,
    mut hold: Local<crate::hold::HoldReport>,
) {
    if npcs.is_empty() {
        return; // 没有待装配的成员：名册未铺，或已全部装配
    }
    if let LoadState::Failed(err) = server.load_state(&manifest_files.json) {
        panic!("角色清单装载失败：{err:?}");
    }
    let now = time.elapsed_secs();
    if !server.is_loaded_with_dependencies(&manifest_files.json) {
        return hold.hold(&PLAN_HOLD, "the character manifest is still loading", now);
    }
    hold.run(&PLAN_HOLD, now);
    let manifest =
        schema::CharacterManifest::parse(&jsons.get(&manifest_files.json).expect("清单已到齐").0)
            .expect("角色清单解析失败");
    let rows: HashMap<u32, (&str, &str)> = manifest
        .units
        .iter()
        .map(|entry| {
            (
                entry.unit.parse::<u32>().expect("清单代号不是数字"),
                (entry.glb.as_str(), entry.rig.as_str()),
            )
        })
        .collect();
    let mut planned = 0usize;
    for (entity, unit) in &npcs {
        let code = unit
            .0
            .checked_add(100)
            .expect("成员 unitId 加 100 溢出：不是 unitId+100 形");
        let (glb, rig) = rows
            .get(&code)
            .copied()
            .unwrap_or_else(|| panic!("清单里没有成员 unit {} 的角色包（代号 {code}）", unit.0));
        commands.entity(entity).insert(CharacterPack {
            // 包内嵌贴图只在换装之前被画、只由 GPU 采样，没有读纹素的 CPU 读者。
            gltf: moly_assets::residency::load_gltf(
                &server,
                moly_assets::character_glb(glb),
                moly_assets::residency::GltfResidency::Character,
            ),
            file: glb.to_string(),
            rig: server.load::<JsonAsset>(AssetPath::from(format!("moly://{rig}"))),
            rig_file: rig.to_string(),
        });
        planned += 1;
    }
    info!(
        "角色装配计划：{planned} 名（清单分母 {}）；相机跟随契约归玩家域（AvatarRoot 由玩家实体持）",
        manifest.units.len()
    );
}

/// Update：某名成员的角色包到齐即挂载——包场景作为成员实体的子实体
/// 展开，模型的局部原点与成员重合，成员的 Transform 带它走。每名独立
/// 过门：一名没到齐不挡别人。装载失败响亮失败。
pub fn attach_when_ready(
    mut commands: Commands,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    npcs: Query<(Entity, &CharacterUnitId, &CharacterPack), Without<ModelAttached>>,
) {
    for (npc, unit, pack) in &npcs {
        if let LoadState::Failed(err) = server.load_state(&pack.gltf) {
            panic!("unit {} 的角色包 {} 装载失败：{err:?}", unit.0, pack.file);
        }
        if let RecursiveDependencyLoadState::Failed(err) =
            server.recursive_dependency_load_state(&pack.gltf)
        {
            panic!(
                "unit {} 的角色包 {} 依赖装载失败：{err:?}",
                unit.0, pack.file
            );
        }
        if !server.is_loaded_with_dependencies(&pack.gltf) {
            continue; // 这名等下一帧，不挡别人
        }
        let character = gltfs.get(&pack.gltf).expect("装载门已过");
        let Some(scene) = character.default_scene.clone() else {
            panic!("unit {} 的角色包 {} 没有默认 scene", unit.0, pack.file);
        };
        commands
            .entity(npc)
            .with_child((WorldAssetRoot(scene), CharacterModel));
        commands.entity(npc).insert(ModelAttached);
        info!("[npc unit={}] 角色包挂载 {}", unit.0, pack.file);
    }
}

/// 全局观察者：角色模型的场景展开完毕，在成员实体上置位
/// [`ModelSceneReady`]（站点同款闩，装配消费后即撤）。事件实体是挂
/// [`WorldAssetRoot`] 的子实体，沿父链一步回到成员。
pub fn on_model_scene_ready(
    trigger: On<WorldInstanceReady>,
    models: Query<&CharacterModel>,
    parents: Query<&ChildOf>,
    mut commands: Commands,
) {
    let model = trigger.event().entity;
    if models.get(model).is_err() {
        return; // 别人的场景（站点）
    }
    let npc = parents.get(model).expect("角色模型实体必有父").0;
    commands.entity(npc).insert(ModelSceneReady);
}

/// The animation root of a character scene spawned at `scene`: the first
/// named node down the scene's single chain of loader entities (the loader's
/// scene entities carry no name). The player goes on it, as the glTF loader
/// puts the player of a file with animations on its animation root.
pub fn animation_root(
    scene: Entity,
    names: &Query<&Name>,
    children: &Query<&Children>,
) -> Result<Entity, String> {
    let mut cursor = scene;
    loop {
        if cursor != scene && names.get(cursor).is_ok() {
            return Ok(cursor);
        }
        let kids = children
            .get(cursor)
            .map_err(|_| "the character scene has no named node".to_owned())?;
        if kids.len() != 1 {
            return Err(format!(
                "the character scene should be one chain of nodes down to its root, it has {} at a node",
                kids.len()
            ));
        }
        cursor = kids[0];
    }
}

/// The animation target of every named node at and below `root`: the node's
/// chain of names from `root`, with `root`'s own name replaced by
/// `reference`, the motion library's reference root (see the module
/// comment). Unnamed entities are the loader's helpers (a primitive's mesh),
/// not glTF nodes: they take no target and add no name to the chain.
pub fn animation_targets(
    root: Entity,
    reference: &Name,
    names: &Query<&Name>,
    children: &Query<&Children>,
) -> Vec<(Entity, AnimationTargetId)> {
    let mut targets = Vec::new();
    let mut chain: Vec<Name> = Vec::new();
    // (entity, the chain's length with the entity's own name)
    let mut stack: Vec<(Entity, usize)> = vec![(root, 1)];
    while let Some((entity, depth)) = stack.pop() {
        if let Ok(name) = names.get(entity) {
            chain.truncate(depth - 1);
            chain.push(if depth == 1 { reference.clone() } else { name.clone() });
            targets.push((entity, AnimationTargetId::from_names(chain.iter())));
        } else {
            chain.truncate(depth);
        }
        if let Ok(kids) = children.get(entity) {
            for kid in kids.iter() {
                stack.push((kid, depth + usize::from(names.get(kid).is_ok())));
            }
        }
    }
    targets
}

/// Update：模型场景展开后补动画目标、装播放器、把两段挂进动画图。
///
/// 逐名装配，每人一份动画图与播放器（图与播放器都是实体局部状态，
/// 不共享）。参考根见模块注释。包围盒扫网格实体的世界系顶点——外壳
/// 高与最低点是相机取景和「真的有模型吗」的现算量。
pub fn wire_when_ready(
    mut commands: Commands,
    time: Res<Time>,
    library: Res<MotionLibrary>,
    libraries: Res<Assets<Gltf>>,
    gltf_nodes: Res<Assets<GltfNode>>,
    clip_assets: Res<Assets<AnimationClip>>,
    mesh_assets: Res<Assets<Mesh>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    npcs: Query<(Entity, &CharacterUnitId, &MotionClips), With<ModelSceneReady>>,
    children: Query<&Children>,
    names: Query<&Name>,
    models: Query<&CharacterModel>,
    meshes_3d: Query<&Mesh3d>,
    skinned_meshes: Query<(), With<SkinnedMesh>>,
    globals: Query<&GlobalTransform>,
    locals: Query<&Transform>,
) {
    if npcs.is_empty() {
        return;
    }
    let Some(lib) = libraries.get(&library.gltf) else {
        return; // 动作库还没到齐；闩保持，下帧再试
    };
    // 库参考根：剪辑骨名链的链首名，运行时从库读出（恰一个）。
    let mut reference_root = None;
    let mut roots = 0usize;
    for handle in &lib.nodes {
        let Some(node) = gltf_nodes.get(handle) else {
            continue;
        };
        if node.is_animation_root {
            roots += 1;
            if reference_root.is_none() {
                reference_root = Some(node.name.clone());
            }
        }
    }
    let reference_root = match (reference_root, roots) {
        (Some(name), 1) => Name::new(name),
        (None, _) => panic!("共享动作库没有动画根节点"),
        (Some(_), count) => panic!("共享动作库有 {count} 个动画根节点：参考根必须唯一"),
    };

    for (npc, unit, clips) in &npcs {
        // 模型子实体：CharacterModel 所在的那个孩子。
        let kids = children.get(npc).expect("模型挂载后有子链");
        let mut model = None;
        for kid in kids.iter() {
            if models.get(kid).is_ok() {
                model = Some(kid);
                break;
            }
        }
        let model = model.expect("ModelSceneReady 的置位者必有 CharacterModel 子实体");
        // 动画根：装载器建的场景实体不带名字，沿链下探到第一个带名字的节点。
        let root = animation_root(model, &names, &children).unwrap_or_else(|reason| panic!("{reason}"));
        // 沿骨名链装配目标，链首替换成库参考根名——见模块注释。
        let mut targets = 0usize;
        for (entity, target) in animation_targets(root, &reference_root, &names, &children) {
            commands.entity(entity).insert((target, AnimatedBy(root)));
            if let Ok(pose) = locals.get(entity) {
                commands
                    .entity(entity)
                    .insert(CharacterRestTransform(*pose));
            }
            targets += 1;
        }
        let mut mesh_entities = 0usize;
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(-f32::MAX);
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if let Ok(mesh3d) = meshes_3d.get(entity) {
                mesh_entities += 1;
                // glTF stores a bind-pose AABB. Skinned face/hair meshes can
                // leave that box while an animation or fixture bridge moves
                // their joints, so let the shader decide visibility. Static
                // accessory meshes retain normal frustum culling.
                if skinned_meshes.get(entity).is_ok() {
                    commands.entity(entity).insert(NoFrustumCulling);
                }
                let mesh = mesh_assets
                    .get(&mesh3d.0)
                    .expect("网格实体引用的 Mesh 不在 Assets 里：装载门已过，不应发生");
                let positions = mesh
                    .attribute(Mesh::ATTRIBUTE_POSITION)
                    .and_then(|values| values.as_float3())
                    .expect("角色网格没有 float3 的 POSITION 属性");
                let global = globals.get(entity).expect("网格实体缺 GlobalTransform");
                for position in positions {
                    let world = global.transform_point(Vec3::from(*position));
                    min = min.min(world);
                    max = max.max(world);
                }
            }
            if let Ok(kids) = children.get(entity) {
                stack.extend(kids.iter());
            }
        }
        if mesh_entities == 0 {
            panic!("unit {} 的角色包里没有网格实体", unit.0);
        }
        let height = max.y - min.y;
        // 段并存一图，同一时刻只播一段（驱动系统负责换段过渡）。
        let idle_clip_name = format!("{}_L", clips.idle);
        let walk_clip_name = format!("{}_L", clips.walk);
        let idle = lib
            .named_animations
            .get(idle_clip_name.as_str())
            .unwrap_or_else(|| panic!("共享动作库没有段 {idle_clip_name}"));
        let walk = lib
            .named_animations
            .get(walk_clip_name.as_str())
            .unwrap_or_else(|| panic!("共享动作库没有段 {walk_clip_name}"));
        // 转身段：六个基名各 `_S`/`_L` 两节点。选段是运行时的（换腿帧才
        // 定），六个全进图；缺段即响亮失败——转身律选出的段必须全在图里，
        // 运行时选到不在图里的段就是断线。
        let mut turn_s = Vec::with_capacity(TurnMotion::ALL.len());
        let mut turn_l = Vec::with_capacity(TurnMotion::ALL.len());
        for motion in TurnMotion::ALL {
            let base = motion.base_name();
            let s_name = format!("{base}_S");
            let l_name = format!("{base}_L");
            let s = lib
                .named_animations
                .get(s_name.as_str())
                .unwrap_or_else(|| panic!("共享动作库没有转身段 {s_name}"));
            let l = lib
                .named_animations
                .get(l_name.as_str())
                .unwrap_or_else(|| panic!("共享动作库没有转身段 {l_name}"));
            turn_s.push(s.clone());
            turn_l.push(l.clone());
        }
        let mut graph = AnimationGraph::new();
        let idle_node = graph.add_clip(idle.clone(), 1.0, graph.root);
        let walk_node = graph.add_clip(walk.clone(), 1.0, graph.root);
        // 起始段进图，连同源等待的秒数（段长 / 速度，毫秒取整）。
        let clip_seconds = |clip: &Handle<AnimationClip>| -> f32 {
            clip_assets
                .get(clip)
                .expect("段句柄在库的 named_animations 里")
                .duration()
        };
        let mut lead_in = |clip: &Handle<AnimationClip>| LeadIn {
            node: graph.add_clip(clip.clone(), 1.0, graph.root),
            wait: source_lead_in_seconds(clip_seconds(clip)),
        };
        // 位移族的起始段可缺：源在段不在控制器里时换段与等待都不做。
        let idle_s = lib
            .named_animations
            .get(format!("{}_S", clips.idle).as_str())
            .map(&mut lead_in);
        let walk_s = lib
            .named_animations
            .get(format!("{}_S", clips.walk).as_str())
            .map(&mut lead_in);
        let turn_s_nodes: Vec<LeadIn> = turn_s.iter().map(&mut lead_in).collect();
        let turn_l_nodes: Vec<AnimationNodeIndex> = turn_l
            .iter()
            .map(|clip| graph.add_clip(clip.clone(), 1.0, graph.root))
            .collect();
        let graph_handle = graphs.add(graph);
        // 播放器按装载器约定落在动画根上；过渡组件与播放器同实体（引擎
        // 的换段过渡原语，驱动系统经它换段）。
        commands.entity(root).insert((
            AnimationPlayer::default(),
            AnimationTransitions::new(),
            AnimationGraphHandle(graph_handle.clone()),
        ));
        // 待机段一个循环的长度：装配账目行用（资产量，不发明默认值）。
        let idle_seconds = clip_assets
            .get(idle)
            .expect("段句柄在库的 named_animations 里")
            .duration();
        commands.entity(npc).insert((
            crate::markers::CharacterDriver,
            MotionDriver {
                player: root,
                idle: idle_node,
                idle_s,
                walk: walk_node,
                walk_s,
                turn_s: turn_s_nodes,
                turn_l: turn_l_nodes,
                playing: None,
                lead_in_wait: None,
                graph: graph_handle.clone(),
                held: None,
                next_generation: 0,
                nodes: HashMap::new(),
                idle_clip: idle_clip_name.clone(),
                walk_clip: walk_clip_name.clone(),
                installed_at: time.elapsed_secs(),
                probed: false,
            },
            CharacterShell {
                height,
                lowest: min.y,
            },
        ));
        commands.entity(npc).remove::<ModelSceneReady>();
        info!(
            "[npc unit={}] 装配完成：骨架目标 {targets} 网格实体 {mesh_entities} 外壳高 {height:.3} 待机段 {idle_clip_name}（{idle_seconds:.2}s）走姿段 {walk_clip_name}；起始段等待 待机 {:?} 走姿 {:?}",
            unit.0,
            idle_s.map(|lead| lead.wait),
            walk_s.map(|lead| lead.wait),
        );
    }
}

/// Transfer a completed source Director back to ordinary idle before a new
/// talk clip is admitted. The caller must release the Director first so its
/// sparse Root tracks cannot survive under a Hips-only shared animation.
///
/// Only the holder of `token` returns the body: when a later state (a talk
/// motion) took it meanwhile, that state's clip keeps playing.
pub fn resume_idle_after_fixture(
    world: &mut World,
    actor: Entity,
    token: BodyToken,
) -> Result<(), String> {
    let mut driver = world
        .get_mut::<MotionDriver>(actor)
        .ok_or("completed fixture actor animator is missing")?;
    if !driver.hand_back(token) {
        return Ok(());
    }
    let (animator, idle) = (driver.player, driver.idle);
    let mut query = world.query::<(&mut AnimationPlayer, &mut AnimationTransitions)>();
    let (mut player, mut transitions) = query
        .get_mut(world, animator)
        .map_err(|_| "completed fixture animation player is missing")?;
    transitions.play(&mut player, idle, SEGMENT_BLEND).repeat();
    world.get_mut::<MotionDriver>(actor).unwrap().playing = Some(MotionKind::Idle);
    Ok(())
}

/// Update：移动相位驱动动画段——在走播走姿族、驻留播待机族、转体播
/// 转身族。相位换族即发一条换段调用：起始段起播，等满段长接循环段，每
/// 步 0.5 秒过渡（段链语义见模块注释）；相位不换族不发。
///
/// 别的状态持有身体（[`MotionDriver::locomotion_owns`] 为假）时本系统整名让位：不换段、
/// 不做段族衔接——那段期间的播放器归演出侧，它交还时已把 `playing`
/// 簿记归位（idle），本系统读到「已在播」自然不重触发。
///
/// 换段一律经引擎过渡组件，且**只在段种变化时调用**：过渡原语每次调用
/// 都把段重置到头，同段重复换等于永不播放——`playing` 的簿记不是装饰，
/// 是调用的前置条件。
pub fn drive(
    clock: Res<crate::npc_clock::NpcClock>,
    mut npcs: Query<
        (
            &CharacterUnitId,
            &MotionPhase,
            &mut MotionDriver,
            Option<&crate::npc_look_at::NpcLookAt>,
        ),
    >,
    mut players: Query<&mut AnimationPlayer>,
    mut transitions: Query<&mut AnimationTransitions>,
) {
    let dt = clock.delta();
    for (unit, phase, mut driver, look_at) in &mut npcs {
        if !driver.locomotion_owns() {
            continue; // another state holds the body: locomotion waits
        }
        // A look-at in flight on a standing character: the presenter's turn
        // clip (started with the tween); its end hands back to the idle clip.
        let family = match (look_at, phase) {
            (Some(look), MotionPhase::Dwelling { .. }) => MotionFamily::Turn(look.clip()),
            (_, MotionPhase::Walking | MotionPhase::FitWalking { .. }) => MotionFamily::Walk,
            (_, MotionPhase::Dwelling { .. }) => MotionFamily::Idle,
            (_, MotionPhase::Turning { motion, .. } | MotionPhase::FitTurning { motion, .. }) => {
                MotionFamily::Turn(*motion)
            }
        };
        // 同族：源的换段调用对同一原名直接返回，在播的段（起始段或循环
        // 段）照播——族内段链单调，接上 `_L` 就停在 `_L`，不回推 `_S`。
        let mut kind = match driver.playing {
            Some(playing) if playing.family() == family => playing,
            _ => driver.start(family),
        };
        // 起始段接循环段：起始段起播的那一帧建等待，此后逐帧累加 NPC
        // 帧间隔，累计达到等待秒数的那一帧换到循环段（源 `UniTask.Delay`
        // 的 DeltaTime 型：创建帧不计）。相位没变：这是段链的衔接。
        if driver.playing == Some(kind) {
            if let Some(next) = kind.loop_segment() {
                let wait = driver
                    .lead_in_wait
                    .as_mut()
                    .expect("起始段在播时必有它起播时建的等待");
                if wait.advance(dt) {
                    kind = next;
                }
            }
        }
        if driver.playing == Some(kind) {
            continue;
        }
        let old = driver.playing;
        let mut player = players
            .get_mut(driver.player)
            .expect("装配系统应已插上播放器");
        let player = &mut *player;
        let transitions = transitions
            .get_mut(driver.player)
            .expect("装配系统应已插上过渡组件")
            .into_inner();
        let animation = transitions.play(player, driver.node(kind), SEGMENT_BLEND);
        match driver.lead_in(kind) {
            // 起始段播一遍（不循环），等待从这一帧起建。
            Some(lead) => driver.lead_in_wait = Some(UniTaskDelay::new(lead.wait)),
            None => {
                animation.repeat();
                driver.lead_in_wait = None;
            }
        }
        driver.playing = Some(kind);
        let word = |kind: MotionKind| match kind {
            MotionKind::IdleS => "idle_s".to_owned(),
            MotionKind::Idle => "idle".to_owned(),
            MotionKind::WalkS => "walk_s".to_owned(),
            MotionKind::Walk => "walk".to_owned(),
            MotionKind::TurnS(motion) => format!("turn_s({})", motion.label()),
            MotionKind::TurnL(motion) => format!("turn_l({})", motion.label()),
        };
        let from = old
            .map(word)
            .unwrap_or_else(|| "未起播".to_owned());
        debug!(
            "[npc unit={}] 动画段 {from} -> {}（{}）",
            unit.0,
            word(kind),
            driver.clip(kind)
        );
    }
}

/// Update：每名装配后两秒报一次播放器实况——「动画真的在播」的现算
/// 证据；没有一段在播即响亮失败。
pub fn probe_playback(
    time: Res<Time>,
    mut npcs: Query<(&CharacterUnitId, &mut MotionDriver)>,
    players: Query<&AnimationPlayer>,
) {
    for (unit, mut driver) in &mut npcs {
        if driver.probed || time.elapsed_secs() - driver.installed_at < 2.0 {
            continue;
        }
        let player = players.get(driver.player).expect("装配后播放器应仍在");
        if player.playing_animations().count() == 0 {
            panic!("unit {} 起播两秒后没有任何动画在播：动画图装配断了", unit.0);
        }
        for (node, animation) in player.playing_animations() {
            info!(
                "[npc unit={}] 动画在播：node={node:?} 权重 {:.2} 已播 {:.2}s 完成 {} 次",
                unit.0,
                animation.weight(),
                animation.elapsed(),
                animation.completions()
            );
        }
        driver.probed = true;
    }
}

#[cfg(test)]
mod visibility_regressions {
    use super::*;

    #[test]
    fn animated_face_is_not_rejected_by_its_offscreen_bind_pose() {
        use bevy::camera::{
            primitives::{Aabb, Frustum},
            visibility::{check_visibility_cpu_culling, VisibleEntities},
        };
        use bevy::shape::{HalfSpace, ViewFrustum};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, check_visibility_cpu_culling);
        // Screen edge at x=0: the bind-pose face is entirely outside, but its
        // animated joint has moved the face across the edge into the view.
        app.world_mut().spawn((
            Camera::default(),
            VisibleEntities::default(),
            Frustum(ViewFrustum {
                half_spaces: [HalfSpace::new(Vec4::new(1.0, 0.0, 0.0, 0.0)); 6],
            }),
        ));
        let mut spawn = || {
            app.world_mut()
                .spawn((
                    Aabb::from_min_max(Vec3::new(-1.1, -0.1, -0.1), Vec3::new(-0.9, 0.1, 0.1)),
                    GlobalTransform::IDENTITY,
                    InheritedVisibility::VISIBLE,
                    ViewVisibility::HIDDEN,
                ))
                .id()
        };
        let face = spawn();
        let static_prop = spawn();
        app.world_mut().entity_mut(face).insert(NoFrustumCulling);
        app.update();
        assert!(app.world().get::<ViewVisibility>(face).unwrap().get());
        assert!(!app
            .world()
            .get::<ViewVisibility>(static_prop)
            .unwrap()
            .get());
    }
}
