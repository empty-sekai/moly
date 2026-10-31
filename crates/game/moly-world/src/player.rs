//! The player domain: the player entity's assembly and its input-driven
//! movement.
//!
//! The player is an entity separate from the roster (`AvatarDataStore.Player`
//! holds the single `PlayerAvatarPresenter`), and movement **does not use
//! pathfinding**: the `UpdateState` of `PlayerAvatarMoveState` and
//! `PlayerAvatarDashState` moves the agent directly every frame with
//! `NavMeshAgent.Move(input vector × speed × dt)`. The speed comes from three
//! ClientConfig panel keys (`MysekaiNormalMoveScale` /
//! `MysekaiHarvestMoveScale` / `MysekaiDashSpeedRate`, FloatConfigs keys
//! 77/78/95), **read every frame**: UpdateState calls the getters on each
//! frame, and this crate does the same.
//! Order of evaluation (the walk and dash states share it):
//! * On a harvesting site (grassland) the speed key is 78 and the animation
//!   multiplier 2.0; otherwise key 77 and 1.75
//!   (`SiteManager.CurrentSiteType == 4` selects the branch, and 4 is
//!   grassland, the default site of this crate).
//! * With the field camera in its FPS state
//!   (`FieldCamera.CurrentState == 16`) the speed is halved (only the
//!   displacement speed; the animation multiplier is unaffected).
//! * In the dash state the Move multiplier is rate × scale, so the halved
//!   scale is multiplied by the rate.
//!
//! Input forms:
//! * The main path is the virtual joystick (`OnTouchJoyStick`: the stick
//!   vector is projected onto the camera's forward/right axes, so it is
//!   camera-relative). The touch surface is [`crate::joystick`]; while it is
//!   active it takes precedence (`GetMoveDirection` reads only the stick).
//! * The keyboard is a named desktop adaptation: the product has only the
//!   touch joystick, and `GetKeyMoveDirection` has no call site. The
//!   adaptation keeps the stick's units: WASD gives a stick ratio
//!   (horizontal, vertical) ∈ {-1,0,1}², put through the joystick
//!   HandleInput clamp (a length above 1 is normalised), then projected onto
//!   the camera with the joystick's precomputed axes; a stick dragged to the
//!   same ratio yields the same movement vector, so a diagonal is no faster
//!   than a straight line.
//! * Dash is a **mode switch**, not a trigger: `PlayerAvatarStateMachine.MoveTo`
//!   dispatches to the Move/Dash state by the player's `IsDashMode`, and the
//!   HUD's dash button ([`crate::action_button`]'s sprint button) drives the
//!   switch.
//! * With no input the player returns to idle: `UpdateController` moves to
//!   Idle on reading empty input in the move state.
//!
//! The facing is **set directly**: `Quaternion.Euler(0, atan2(input.x, input.z), 0)`,
//! with no turn animation segment; turn blending belongs to the NPC patrol
//! domain. The animation segments follow [`MotionPhase`] through the player's
//! own driver chain ([`crate::player_avatar`]: walk while walking, the dash
//! clip in dash mode, the idle clip while dwelling); the playback speed uses
//! `clamp(√(input x² + input z²) × site multiplier, 0.55, 1.8)` (multiplier
//! 2.0 on harvesting sites, 1.75 elsewhere), and 1.0 while idle (the
//! `PlayerAvatarIdleState` start literal).
//!
//! Camera contract: the player entity holds [`AvatarRoot`] (inserted by
//! this module), and the camera domain reads
//! it every frame to frame the view; the camera follows the player from the
//! frame the player is assembled.
//!
//! The body is the player avatar's own model and action set
//! ([`crate::player_avatar::body`]: the spectator's model, its own skeleton,
//! its own 172 clip segments), not the appearance of a roster member. The
//! player belongs to [`crate::player_avatar`].
//! * Movement boundary: the constraint surface and the accept/refuse rules
//!   are in [`crate::walk_face`]; a step frame shares the erosion field with
//!   the NPCs, and the whole displacement is constrained.

use crate::character::AvatarRoot;
use crate::npc::MotionPhase;
use crate::player_fixture_action::PlayerFixtureHeld;
use crate::site::GroundMeshes;
use crate::walk_face;
use bevy::prelude::*;
use moly_law::path::heading_yaw;

/// Placement for the product's own starts only. In the source the player is
/// placed by the join (`JoinMysekaiActionState.MoveToEntrance`: the house's
/// outside door locator, ported in the entry), by the site moves (the cannon
/// landing, the house and room doors) and by the gate warp; nothing else
/// positions it. The starts the source never makes (the embedded web stage
/// without a house, a first site other than home, a site switch outside a
/// site move) have no source position, so they use this ring: the side of
/// the site centre opposite the roster's first member, seated on the walk
/// field. A named adaptation, not a source value.
const SEED_RADIUS: f32 = 3.0;

/// 地表高度采样半径：取「脚下的地面」，与名册同款。
const SURFACE_RADIUS: f32 = 2.0;

/// Walk animation speed multiplier, normal band: the `MoveState`/`DashState`
/// literal on sites other than a harvesting site.
const ANIM_SPEED_NORMAL: f32 = 1.75;

/// Walk animation speed multiplier, harvesting band: the
/// `MoveState`/`DashState` literal that replaces the normal band on a site
/// of type grassland. The camera's FPS band does not halve it; only the
/// displacement speed is halved.
const ANIM_SPEED_HARVEST: f32 = 2.0;

/// Lower bound of the animation speed: the floor of the run segment's
/// animation rate.
const MIN_ANIMATION_SPEED: f32 = 0.55;

/// Upper bound of the animation speed. The walk and dash states both apply
/// `fminf(v, 1.8)` and then `v < 0.55 ? 0.55 : <clamped value>`, so the upper
/// bound holds. The same function clamps its other values with plain
/// comparisons. The bound is kept: removing it would let the diagonal case
/// through. A diagonal input is not normalised (√2 × 2.0 ≈ 2.83), so the
/// animation speed exceeds the bound and is pressed back.
const MAX_ANIMATION_SPEED: f32 = 1.8;

pub use crate::markers::PlayerControlled;

/// Dash mode switch (`PlayerAvatarPresenter.IsDashMode`); the HUD's sprint
/// button toggles it.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct DashMode(pub bool);

/// 本帧输入：方向（世界系，相机相对合成）与是否活跃。输入系统写、推进
/// 系统读。
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerInput {
    pub direction: Vec3,
    pub active: bool,
}

/// 玩家的身体输入；spawn 消费后即撤。位移速度不在这儿：速度是
/// ClientConfig 面板三键的逐帧取值（见 [`move_speed`]），不随装配算定。
#[derive(Resource)]
pub struct PlayerSpecs {
    body: crate::player_avatar::body::PlayerBody,
}

/// 玩家出生时缓存的地表世界顶点（推进帧的脚下高度采样用）。
#[derive(Resource)]
pub struct PlayerGround(Vec<Vec3>);

/// spawn 一次性闩：玩家只铺一次。
#[derive(Resource)]
pub struct PlayerSpawned;

/// Startup：玩家的身体输入就位（avatar 模型与动作组；装载由
/// [`crate::player_avatar::body`] 在玩家铺下后发起）。
pub fn load(mut commands: Commands) {
    commands.insert_resource(PlayerSpecs {
        body: crate::player_avatar::body::PlayerBody::avatar(),
    });
}

/// The player's placement hold: the player does not appear until its three
/// inputs (the body spec, the client configs and the site ground) are in.
const SPAWN: crate::hold::HoldSite = crate::hold::HoldSite {
    tag: "player",
    what: "player spawns",
    effect: "the player does not appear",
    quiet: &[],
};

/// Update: once the spec, the configs and the site ground are ready, spawns
/// the player once. The entity carries the movement phase and the body spec
/// (avatar model and action set), plus the player-only components:
/// [`AvatarRoot`] (the camera follows the player from then on), the dash
/// mode and the input. The config gate is the roster's: the speed law reads
/// the configs, so no player is spawned while they are absent.
/// The birth phase is idle: `InitializeStatus` starts the player in Idle.
pub fn spawn_when_ready(
    mut commands: Commands,
    time: Res<Time>,
    meshes: Res<Assets<Mesh>>,
    specs: Option<Res<PlayerSpecs>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    ground: Option<Res<GroundMeshes>>,
    face: Option<Res<walk_face::WalkFace>>,
    spawned: Option<Res<PlayerSpawned>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
    mut hold: Local<crate::hold::HoldReport>,
) {
    if spawned.is_some() {
        return;
    }
    let now = time.elapsed_secs();
    let Some(specs) = specs else {
        return hold.hold(&SPAWN, "the player body spec is not loaded", now);
    };
    let Some(configs) = configs else {
        return hold.hold(&SPAWN, "the client configs are not loaded", now);
    };
    let Some(ground) = ground else {
        return hold.hold(&SPAWN, "the ground meshes are not loaded", now);
    };
    let verts = crate::npc::ground_verts(&meshes, &ground, &parts);
    if verts.is_empty() {
        return hold.hold(&SPAWN, "the site scene has no ground vertices yet", now);
    }
    hold.run(&SPAWN, now);
    let (center, center_y) = crate::npc::center_of(&verts);
    let scale = crate::npc::ring_scale(&verts);
    // The adaptation ring (see SEED_RADIUS): opposite the roster's first
    // member, the radius scaled to room-sized ground, seated on the walk
    // field. On the home start the entry moves the player to the house door
    // before the cover lifts, as the source's join does.
    let radius = SEED_RADIUS * scale;
    let (mut seed_x, mut seed_z) = (center.x - radius, center.y + radius);
    if let Some(face) = face.as_deref() {
        (seed_x, seed_z) = face.seat(seed_x, seed_z);
    }
    let surface = face
        .as_deref()
        .and_then(|face| face.surface_height([seed_x, seed_z]));
    let seed = [seed_x, navigation_y(&verts, seed_x, seed_z, center_y, surface), seed_z];
    commands.spawn((
        Transform::from_translation(Vec3::from(seed)),
        // 玩家是渲染层级的节点：模型子实体的可见性沿父链向上查到本实体。
        Visibility::default(),
        // Camera-follow contract: the player entity holds AvatarRoot (the
        // site camera follows the player avatar's view transform). The body
        // and its action set are attached by player_avatar::body.
        AvatarRoot,
        PlayerControlled,
        DashMode(false),
        PlayerInput::default(),
        MotionPhase::Dwelling { remaining: None },
        specs.body.clone(),
    ));
    commands.insert_resource(PlayerGround(verts));
    commands.insert_resource(PlayerSpawned);
    commands.remove_resource::<PlayerSpecs>();
    info!(
        "[player] 玩家就位：身体 {}，步速键 {}={:.3} · 采集档 {}={:.3} · 冲刺率 {}={:.3}（逐帧取值），AvatarRoot 移交相机",
        specs.body.model,
        crate::client_config::KEY_MYSEKAI_NORMAL_MOVE_SCALE,
        configs.float(crate::client_config::KEY_MYSEKAI_NORMAL_MOVE_SCALE),
        crate::client_config::KEY_MYSEKAI_HARVEST_MOVE_SCALE,
        configs.float(crate::client_config::KEY_MYSEKAI_HARVEST_MOVE_SCALE),
        crate::client_config::KEY_MYSEKAI_DASH_SPEED_RATE,
        configs.float(crate::client_config::KEY_MYSEKAI_DASH_SPEED_RATE),
    );
}

/// Update：换站后的玩家重播种。吃站点定案代数（见 `site::GroundEpoch`）：
/// 首代只记录不重排——玩家刚以同式铺位；代数翻新（换站）时实体原样保留
/// （AvatarRoot 与相机契约不动），位置落到新站出生点（缩放同 spawn），
/// 脚下地表顶点表换新站的——推进帧的高度采样从此读新地面。
pub fn reseed(
    epoch: Option<Res<crate::site::GroundEpoch>>,
    mut last: Local<u64>,
    meshes: Res<Assets<Mesh>>,
    ground: Option<Res<GroundMeshes>>,
    face: Option<Res<walk_face::WalkFace>>,
    parts: Query<(&Mesh3d, &GlobalTransform)>,
    mut players: Query<
        (
            &mut Transform,
            &mut MotionPhase,
            Option<&mut crate::player_avatar::AvatarDriver>,
        ),
        With<PlayerControlled>,
    >,
    mut animators: Query<&mut AnimationPlayer>,
    mut ground_cache: Option<ResMut<PlayerGround>>,
    site_move: Option<Res<crate::field_modes::SiteMoveActive>>,
) {
    let Some(epoch) = epoch else {
        return;
    };
    // Scene readiness advances GroundEpoch only after its roots are settled.
    // An unchanged generation cannot need a new world-space vertex snapshot.
    if epoch.0 == *last {
        return;
    }
    let Some(ground) = ground else {
        return;
    };
    let verts = crate::npc::ground_verts(&meshes, &ground, &parts);
    if verts.is_empty() {
        return;
    }
    let first = *last == 0;
    *last = epoch.0;
    if first {
        return;
    }
    if site_move.is_some() {
        // A cannon move carries the player onto the new site itself (its
        // flight and re-origin); only the ground snapshot is renewed.
        if let Some(cache) = &mut ground_cache {
            **cache = PlayerGround(verts);
        }
        info!("[player] ground snapshot renewed during a cannon move; the move places the player");
        return;
    }
    let (center, center_y) = crate::npc::center_of(&verts);
    let radius = SEED_RADIUS * crate::npc::ring_scale(&verts);
    let (mut seed_x, mut seed_z) = (center.x - radius, center.y + radius);
    if let Some(face) = face.as_deref() {
        (seed_x, seed_z) = face.seat(seed_x, seed_z);
    }
    let surface = face
        .as_deref()
        .and_then(|face| face.surface_height([seed_x, seed_z]));
    let seed = [seed_x, navigation_y(&verts, seed_x, seed_z, center_y, surface), seed_z];
    for (mut transform, mut phase, driver) in &mut players {
        if let Some(mut driver) = driver {
            if let Ok(mut animator) = animators.get_mut(driver.player) {
                driver.cancel_for_site_change(&mut animator);
            }
        }
        transform.translation = Vec3::from(seed);
        *phase = MotionPhase::Dwelling { remaining: None };
    }
    if let Some(cache) = &mut ground_cache {
        **cache = PlayerGround(verts);
    }
    info!(
        "[player] 换站重播种：落位 ({:.2},{:.2},{:.2})，脚下地表顶点 {} 个",
        seed[0],
        seed[1],
        seed[2],
        ground_cache.map(|cache| cache.0.len()).unwrap_or(0)
    );
}

/// The player domain's systems that the host schedule does not list: the
/// player avatar's talk-camera dither. Added by the avatar material plugin,
/// which the host registers. The host schedule lists the dash button's
/// systems with the action buttons.
pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            crate::avatar_material::player_dither
                .after(crate::talk_camera::advance)
                .after(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// 界缘裁决的跨帧态（沿触发日志用）：面内自由、刚被回吸、或被拒保持。
/// pub：schedule 在模块外登记本系统。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Boundary {
    #[default]
    Free,
    Snapped,
    Held,
}

/// The speed law (the evaluation order of the walk and dash states'
/// UpdateState, called every frame): a harvesting site (grassland) reads
/// speed key 78, any other site reads key 77; with the field camera in its
/// FPS state the speed is halved (only the displacement speed, not the
/// animation multiplier); the dash state multiplies by key 95 — the order is
/// `rate × scale`, so the halved scale is multiplied by the rate. Returns
/// this frame's displacement speed in metres per second.
fn move_speed(
    configs: &crate::client_config::ClientConfigs,
    site: &crate::site::SiteSelection,
    camera: crate::camera::CameraStateType,
    dash: bool,
) -> f32 {
    let mut scale = if site.is_grassland() {
        configs.float(crate::client_config::KEY_MYSEKAI_HARVEST_MOVE_SCALE)
    } else {
        configs.float(crate::client_config::KEY_MYSEKAI_NORMAL_MOVE_SCALE)
    };
    if camera == crate::camera::CameraStateType::Fps {
        scale *= 0.5;
    }
    if dash {
        scale *= configs.float(crate::client_config::KEY_MYSEKAI_DASH_SPEED_RATE);
    }
    scale
}

/// The walk animation speed multiplier (the branch beside the speed in the
/// same function): 2.0 on harvesting sites, 1.75 elsewhere; the camera's FPS
/// band does not halve it.
fn animation_speed_mul(site: &crate::site::SiteSelection) -> f32 {
    if site.is_grassland() {
        ANIM_SPEED_HARVEST
    } else {
        ANIM_SPEED_NORMAL
    }
}

/// The pace of a body walking on the Move state's law with the stick at
/// `magnitude` and the field camera in its normal state: its speed in metres
/// per second and its walk clip's animation speed. Both grow with the stick
/// until the animation speed meets its clamp, so a pace inside the clamp
/// keeps the stride on the ground.
pub fn move_pace(
    configs: &crate::client_config::ClientConfigs,
    site: &crate::site::SiteSelection,
    magnitude: f32,
) -> (f32, f32) {
    let speed = magnitude * move_speed(configs, site, crate::camera::CameraStateType::Normal, false);
    let animation =
        (magnitude * animation_speed_mul(site)).clamp(MIN_ANIMATION_SPEED, MAX_ANIMATION_SPEED);
    (speed, animation)
}

/// Update: advances the input of this frame into a new position and a new
/// facing. This is the player domain's displacement, applied directly each
/// frame rather than by pathfinding, like `NavMeshAgent.Move`; pathfinding
/// belongs to the NPC patrol domain. The speed is read from [`move_speed`]
/// every frame (the three panel keys, the site band, the camera band). Each
/// step is constrained by the walk face to the walkable part of the move
/// before its first obstruction, so a thin obstacle cannot be crossed. No
/// step is taken while the face is not ready; the navigation mesh exists
/// from scene load onward, so the player never moves on a frame without a
/// constraint surface.
///
/// With input: position += direction × speed(dash band) × dt, and the height
/// under the feet follows the sampled ground; the facing is set directly to
/// `atan2(direction.x, direction.z)` with no turn segment; the phase enters
/// Walking. Without input: the phase enters Dwelling (the idle clip). The
/// phase's readers are the animation driver chain and the camera.
/// This system writes the player's [`Transform`] (displacement and facing),
/// as does the dialogue domain's turn interpolation; while a dialogue plays
/// with the player taking part, a hold makes this system skip its work. A
/// furniture session owns the position from approach through seating to
/// leaving with its own movement flow; its animation lease covers only the
/// playing segment, so a player occupancy cannot stand in for holding the
/// whole displacement. Input is still collected by read_input so the session
/// can end when asked. The system takes private resources as parameters, so
/// it is pub.
pub fn advance(
    bulk: Option<Res<crate::field_modes::BulkHarvestActive>>,
    time: Res<Time>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    site: Option<Res<crate::site::SiteSelection>>,
    camera_state: Res<crate::camera::FieldCameraState>,
    ground: Option<Res<PlayerGround>>,
    face: Option<Res<walk_face::WalkFace>>,
    objective_face: Option<Res<crate::npc_objective::ObjectiveFace>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    // 对话持留让位：玩家参演对话期间位移推进不跑（相位在开场时已钉
    // 驻留；转身由对话域插值直写）。
    mut players: Query<
        (
            &mut Transform,
            &mut MotionPhase,
            &PlayerInput,
            &DashMode,
            Option<&crate::player_avatar::AvatarDriver>,
        ),
        (
            With<PlayerControlled>,
            Without<crate::talk::TalkHold>,
            Without<PlayerFixtureHeld>,
            Without<crate::field_modes::EntryHold>,
            Without<crate::npc_objective::events::tutorial::TutorialMovementHold>,
            Without<crate::field_modes::HarvestAutoMoveHeld>,
            Without<crate::field_modes::DeliveryHold>,
        ),
    >,
    mut boundary: Local<Boundary>,
    mut relocated: Local<Option<(Option<u64>, u64)>>,
) {
    if bulk.is_some() { return; }
    let dt = time.delta_secs();
    let (Some(configs), Some(site), Some(ground), Some(face)) = (configs, site, ground, face)
    else {
        return;
    };
    let navigation_y_at = |x: f32, z: f32, raw_fallback: f32| {
        objective_face
            .as_deref()
            .filter(|surface| {
                surface.navigation_generation() == face.generation()
                    && epoch.as_deref().is_some_and(|epoch| surface.is_fresh(epoch.0))
            })
            .and_then(|surface| surface.navigation_point_at([x, z]))
            .map(|point| point[1])
            .or_else(|| face.surface_height([x, z]))
            .unwrap_or_else(|| surface_y(&ground.0, x, z, raw_fallback))
    };
    for (mut transform, mut phase, input, dash, driver) in &mut players {
        if driver.is_some_and(|driver| driver.blocks_manual_movement()) {
            continue;
        }
        // The fallback for a point with no sampled ground keeps the height the
        // player already had; surface heights are absolute, so nothing is
        // added twice.
        let raw_y = transform.translation.y;
        // The source's crowd moves an agent only when its navigation polygon
        // is gone, that is once per build of the navigation cells (site and
        // generation): to the closest point of the nearest cell in its query
        // box. A player on a cell, edges included, stays.
        let build = (epoch.as_deref().map(|epoch| epoch.0), face.generation());
        if *relocated != Some(build) {
            *relocated = Some(build);
            let at = [transform.translation.x, transform.translation.z];
            let radius =
                crate::fixture_scene_inputs::PlayerFixtureAgentParameters::default().radius();
            if let Some([x, z]) = face.relocate(at, radius).filter(|point| *point != at) {
                transform.translation = Vec3::new(x, navigation_y_at(x, z, raw_y), z);
            }
        }
        // Rebuilds can remove a rug without invalidating x/z. A resting
        // navigation-owned player must settle onto that new floor as well.
        transform.translation.y = navigation_y_at(
            transform.translation.x, transform.translation.z, raw_y);
        if input.active {
            let speed = move_speed(&configs, &site, camera_state.0, dash.0);
            let mut position = transform.translation + input.direction * speed * dt;
            let requested = position;
            let start = [transform.translation.x, transform.translation.z];
            // The source's move state moves the agent by NavMeshAgent.Move,
            // the engine's corridor move on the navigation cells.
            let accepted =
                face.move_position(transform.translation.to_array(), position.to_array());
            position.x = accepted[0];
            position.z = accepted[1];
            let next_boundary = if accepted == [requested.x, requested.z] {
                Boundary::Free
            } else if accepted == start {
                Boundary::Held
            } else {
                Boundary::Snapped
            };
            if *boundary != next_boundary {
                debug!(
                    "[player] 导航线段裁决 {:?}：请求 ({:.2},{:.2}) → ({:.2},{:.2})，代数 {}",
                    next_boundary,
                    requested.x,
                    requested.z,
                    position.x,
                    position.z,
                    face.generation()
                );
                *boundary = next_boundary;
            }
            let was_walking = matches!(*phase, MotionPhase::Walking);
            if !was_walking {
                info!(
                    "[player] 输入开始：方向 ({:.2},{:.2}) 模式 {} 速度 {:.3} m/s",
                    input.direction.x,
                    input.direction.z,
                    if dash.0 { "dash" } else { "walk" },
                    speed
                );
            }
            // The facing is set directly: the move state writes
            // Euler(0, atan2(x, z), 0) every frame. A refused frame also turns
            // toward the push direction (the agent takes the Move request
            // every frame regardless).
            transform.rotation = Quat::from_rotation_y(input.direction.x.atan2(input.direction.z));
            position.y = navigation_y_at(position.x, position.z, raw_y);
            transform.translation = position;
            *phase = MotionPhase::Walking;
        } else if matches!(*phase, MotionPhase::Walking) {
            let p = transform.translation;
            info!(
                "[player] 输入结束：位置 ({:.2},{:.2},{:.2})，回待机",
                p.x, p.y, p.z
            );
            *boundary = Boundary::Free;
            *phase = MotionPhase::Dwelling { remaining: None };
        }
    }
}

/// Update: sets the animation playback speed with the rate law — in the move
/// state `clamp(√(input x² + input z²) × site multiplier, 0.55, 1.8)`
/// (multiplier 2.0 on harvesting sites, 1.75 elsewhere; the walk and dash
/// states share the formula, and dash adds no speed), and 1.0 while idle.
/// Runs after the player animation driver (the segment switch): a switch
/// starts the new segment at the default rate, and this system puts the rate
/// back to the law within the same frame.
pub fn tune_animation_speed(
    site: Option<Res<crate::site::SiteSelection>>,
    mut last_speed: Local<f32>,
    // The activity's approach/exit motion owns its speed even while the
    // locomotion driver still supplies the visible walk/run clip. Raw input
    // retained for end requests must not retime that movement's animation.
    players: Query<
        (
            &MotionPhase,
            &PlayerInput,
            &crate::player_avatar::AvatarDriver,
        ),
        (With<PlayerControlled>, Without<PlayerFixtureHeld>),
    >,
    mut animators: Query<&mut AnimationPlayer>,
) {
    let Some(site) = site else {
        return;
    };
    let anim_mul = animation_speed_mul(&site);
    for (phase, input, driver) in &players {
        if !driver.locomotion_owns_animator() {
            continue;
        }
        let speed = match phase {
            MotionPhase::Walking => {
                let magnitude = (input.direction.x * input.direction.x
                    + input.direction.z * input.direction.z)
                    .sqrt();
                (magnitude * anim_mul).clamp(MIN_ANIMATION_SPEED, MAX_ANIMATION_SPEED)
            }
            _ => 1.0,
        };
        let Ok(mut animator) = animators.get_mut(driver.player) else {
            continue; // 装配未完成，驱动系统尚未起播
        };
        for (_, animation) in animator.playing_animations_mut() {
            animation.set_speed(speed);
        }
        // 速率账（变更时一行，本系统单玩家）：速率律的现算观测面——
        // 站点档乘数与上夹是否生效从这行读（斜向输入的数是它给的）。
        if speed != *last_speed {
            debug!("[player] 动画速率 -> {:.3}（乘数 {:.2}）", speed, anim_mul);
            *last_speed = speed;
        }
    }
}

/// Update（定时）：玩家周期状态行。位置随帧变化是「真的在动」的现算
/// 证据，相邻行可推出位移与速度；dash 档与输入方向让推进行可对账。
/// `界=` 列逐行报可行走面裁决（内=在面内或被回吸按在面缘线上，外=面外
/// ——约束生效时不应出现，?=面未就绪），是移动边界冒烟的逐界验收列。
pub fn report(
    time: Res<Time>,
    face: Option<Res<walk_face::WalkFace>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    site: Option<Res<crate::site::SiteSelection>>,
    camera_state: Res<crate::camera::FieldCameraState>,
    players: Query<(&Transform, &MotionPhase, &DashMode, &PlayerInput), With<PlayerControlled>>,
) {
    let Some((configs, site)) = configs.as_deref().zip(site.as_deref()) else {
        return; // 面板或站点选择未立：速度律没有取值面，跳过本行
    };
    for (transform, phase, dash, input) in &players {
        let forward = transform.rotation * Vec3::Z;
        let yaw = heading_yaw([forward.x, forward.y, forward.z]);
        let phase_word = if matches!(phase, MotionPhase::Walking) {
            "walk"
        } else {
            "dwell"
        };
        let speed = move_speed(configs, site, camera_state.0, dash.0);
        let p = transform.translation;
        let boundary = match &face {
            None => "?",
            Some(face) if face.on_face(p.x, p.z) => "内",
            Some(_) => "外",
        };
        debug!(
            "[player] t={:.1} pos=({:.3},{:.3},{:.3}) yaw={:.1} phase={} dash={} 输入=({:.2},{:.2}) 速度档 {:.3} m/s 界={}",
            time.elapsed_secs(),
            p.x,
            p.y,
            p.z,
            yaw,
            phase_word,
            dash.0,
            input.direction.x,
            input.direction.z,
            speed,
            boundary
        );
    }
}

/// 采样点附近的地表高度：半径内地形顶点的最高者；没有顶点时取 `fallback`
/// （走出地表时保持原高度——移动边界挂账见模块注释）。
fn surface_y(verts: &[Vec3], x: f32, z: f32, fallback: f32) -> f32 {
    verts
        .iter()
        .filter(|v| {
            let dx = v.x - x;
            let dz = v.z - z;
            dx * dx + dz * dz <= SURFACE_RADIUS * SURFACE_RADIUS
        })
        .map(|v| v.y)
        .reduce(f32::max)
        .unwrap_or(fallback)
}

fn navigation_y(verts: &[Vec3], x: f32, z: f32, raw_fallback: f32, surface: Option<f32>) -> f32 {
    surface.unwrap_or_else(|| surface_y(verts, x, z, raw_fallback))
}

#[cfg(test)]
mod navigation_height_tests {
    use super::*;

    #[test]
    fn surface_height_wins_and_only_absent_ground_uses_the_fallback() {
        let ground = [Vec3::ZERO, Vec3::X];
        assert_eq!(navigation_y(&ground, 0.0, 0.0, 0.0, Some(0.05)), 0.05);
        // No surface at the point: the sampled ground vertex answers.
        assert_eq!(navigation_y(&ground, 1.0, 0.0, 0.05, None), 0.0);
        // No sampled ground either: the caller's prior height is kept.
        assert_eq!(navigation_y(&[], 0.0, 0.0, 0.05, None), 0.05);
    }
}
