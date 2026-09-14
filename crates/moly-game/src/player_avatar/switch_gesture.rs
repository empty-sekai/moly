//! User-authorized simple adaptation of the original player's switch motion.
//!
//! The small GLB contains rotation tracks for the current SD104 body, no mesh.
//! It is a product adaptation, not a source Humanoid map or a substitute gesture.
//! Its reference Scene is never spawned: the existing player's one animator owns
//! the action. The source one-second business interval still owns completion.

use super::{AvatarDriver, PlayerActionMotion, PlayerActionOwner, PlayerActionToken};
use crate::{character::CharacterPack, npc::CharacterUnitId, player::PlayerControlled};
use bevy::{
    animation::{AnimatedBy, AnimationClip, AnimationTargetId},
    asset::LoadState,
    ecs::system::SystemState,
    gltf::Gltf,
    prelude::*,
};
use std::time::Duration;

const ASSET: &str = "moly://avatar/motion/sd-switch-simple.glb";
const CLIP: &str = "product_sd104_switch_from_c_000_mov_fixture_action_01_01";
// The original stop-time float, also the adapted clip's final sample.
const SOURCE_DURATION: f32 = 1.000_000_1;
const TARGET_PATHS: [&str; 15] = [
    "mdl_sd_101_001/Root",
    "mdl_sd_101_001/Root/Hips",
    "mdl_sd_101_001/Root/Hips/LeftUpLeg",
    "mdl_sd_101_001/Root/Hips/LeftUpLeg/LeftLeg",
    "mdl_sd_101_001/Root/Hips/LeftUpLeg/LeftLeg/LeftFoot",
    "mdl_sd_101_001/Root/Hips/RightUpLeg",
    "mdl_sd_101_001/Root/Hips/RightUpLeg/RightLeg",
    "mdl_sd_101_001/Root/Hips/RightUpLeg/RightLeg/RightFoot",
    "mdl_sd_101_001/Root/Hips/Spine",
    "mdl_sd_101_001/Root/Hips/Spine/Spine1",
    "mdl_sd_101_001/Root/Hips/Spine/Spine1/Head",
    "mdl_sd_101_001/Root/Hips/Spine/Spine1/LeftArm",
    "mdl_sd_101_001/Root/Hips/Spine/Spine1/LeftArm/LeftForeArm",
    "mdl_sd_101_001/Root/Hips/Spine/Spine1/RightArm",
    "mdl_sd_101_001/Root/Hips/Spine/Spine1/RightArm/RightForeArm",
];

#[derive(Resource)]
struct MotionAsset(Handle<Gltf>);

#[derive(Clone, Copy)]
pub(crate) struct Lease {
    animator: Entity,
    token: PlayerActionToken,
}

pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(MotionAsset(server.load(ASSET)));
}

/// Check the loaded, exact clip and its actual player bindings before a
/// furniture trigger can change the model. This runs only on an action request.
fn resolve(world: &mut World, actor: Entity) -> Result<Handle<AnimationClip>, String> {
    if world.get::<PlayerControlled>(actor).is_none()
        || world.get::<CharacterUnitId>(actor).map(|unit| unit.0) != Some(4)
        || !world.get::<CharacterPack>(actor).is_some_and(|pack| pack.file == "sd_104.glb")
    {
        return Err("simple switch adaptation is authored only for the current SD104 player".into());
    }
    let (animator, graph) = world.get::<AvatarDriver>(actor)
        .map(|driver| (driver.player, driver.graph.clone()))
        .ok_or("SD switch animator is not installed")?;
    if world.get::<AnimationPlayer>(animator).is_none()
        || world.get::<AnimationTransitions>(animator).is_none()
        || world.resource::<Assets<AnimationGraph>>().get(&graph).is_none()
    {
        return Err("SD switch animator/transition graph is not ready".into());
    }
    let asset = world.get_resource::<MotionAsset>()
        .ok_or("simple SD switch motion was not requested")?.0.clone();
    if let LoadState::Failed(error) = world.resource::<AssetServer>().load_state(&asset) {
        return Err(format!("simple SD switch motion failed to load: {error}"));
    }
    let clip_handle = {
        let gltfs = world.resource::<Assets<Gltf>>();
        let gltf = gltfs.get(&asset).ok_or("simple SD switch motion is still loading")?;
        if !gltf.meshes.is_empty() || !gltf.skins.is_empty() || gltf.animations.len() != 1 {
            return Err("simple SD switch asset must contain one animation and no body".into());
        }
        gltf.named_animations.get(CLIP).cloned()
            .ok_or("exact named simple SD switch clip is absent")?
    };
    let target_ids: Vec<AnimationTargetId> = TARGET_PATHS.iter()
        .map(|path| AnimationTargetId::from_iter(path.split('/'))).collect();
    {
        let clips = world.resource::<Assets<AnimationClip>>();
        let clip = clips.get(&clip_handle).ok_or("simple SD switch clip is still loading")?;
        if clip.duration().to_bits() != SOURCE_DURATION.to_bits()
            || clip.curves().len() != target_ids.len()
            || target_ids.iter().any(|id| clip.curves_for_target(*id).is_none_or(|curves| curves.len() != 1))
        {
            return Err("simple SD switch clip duration or explicit bone targets differ".into());
        }
    }
    let mut counts = [0usize; TARGET_PATHS.len()];
    let mut targets = world.query::<(&AnimationTargetId, &AnimatedBy, &Transform)>();
    for (id, _, _) in targets.iter(world).filter(|(_, by, _)| by.0 == animator) {
        if let Some(index) = target_ids.iter().position(|expected| expected == id) {
            counts[index] += 1;
        }
    }
    if let Some(index) = counts.iter().position(|count| *count != 1) {
        return Err(format!("simple SD switch target is missing/ambiguous on this body: {}", TARGET_PATHS[index]));
    }
    Ok(clip_handle)
}

pub(crate) fn start(world: &mut World, actor: Entity) -> Result<Lease, String> {
    let clip = resolve(world, actor)?;
    let mut params = SystemState::<(
        ResMut<Assets<AnimationGraph>>,
        Query<&mut AvatarDriver>,
        Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
    )>::new(world);
    let (mut graphs, mut drivers, mut animators) = params.get_mut(world);
    let mut driver = drivers.get_mut(actor).map_err(|_| "SD switch driver disappeared")?;
    let animator = driver.player;
    let (mut player, mut transitions) = animators.get_mut(animator)
        .map_err(|_| "SD switch animator disappeared")?;
    if let Some(installed) = driver.sd_clips.get(CLIP) {
        if installed.id() != clip.id() {
            return Err("SD switch clip changed after its graph node was installed".into());
        }
    } else {
        driver.sd_clips.insert(CLIP.to_owned(), clip);
    }
    let token = driver.start_action(
        PlayerActionOwner::SwitchGimmick,
        PlayerActionMotion {
            clip: CLIP,
            looping: false,
            speed: 1.0,
            blend: Duration::from_millis(250),
            blocks_manual_movement: true,
        },
        &mut graphs,
        &mut player,
        &mut transitions,
    ).map_err(|error| format!("SD switch motion could not acquire the existing animator: {error:?}"))?;
    info!("[fixture-gimmick] simple source-switch adaptation started: actor={actor:?} animator={animator:?} clip={CLIP} token={token:?}");
    Ok(Lease { animator, token })
}

pub(crate) fn is_alive(world: &World, actor: Entity, lease: Lease) -> bool {
    world.get::<AvatarDriver>(actor).is_some_and(|driver| {
        driver.player == lease.animator
            && driver.action.as_ref().is_some_and(|action| action.token == lease.token)
    }) && world.get::<AnimationPlayer>(lease.animator).is_some()
        && world.get::<AnimationTransitions>(lease.animator).is_some()
}

pub(crate) fn release(world: &mut World, actor: Entity, lease: Lease) {
    let mut params = SystemState::<(
        Query<&mut AvatarDriver>,
        Query<&mut AnimationPlayer>,
    )>::new(world);
    let (mut drivers, mut animators) = params.get_mut(world);
    let Ok(mut driver) = drivers.get_mut(actor) else { return; };
    if driver.player != lease.animator { return; }
    if let Ok(mut player) = animators.get_mut(lease.animator) {
        // release_action checks the exact token before stopping any live node.
        driver.release_action(lease.token, &mut player);
    } else {
        // Same teardown convention as the existing fixture timeline lease:
        // clear only this token; this inert value creates no ECS body/player.
        driver.release_action(lease.token, &mut AnimationPlayer::default());
    }
}

