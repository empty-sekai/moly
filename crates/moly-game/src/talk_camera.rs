//! Player-owned conversation framing and return to the saved Normal view.
//!
//! The target list uses each participating NPC's live hip transform followed
//! by the player's view transform. ShowTalkCamera uses their planar angle,
//! clamps the yaw, sets the target height to the active site's coordinate
//! origin, and enters at distance 3.7 in 0.7 seconds with OutQuad easing.
//!
//! Normal's private model saves the live view on exit when its transfer is
//! complete. Its per-site transfer values are a separate unconditional write.
//! Returning uses the authored category/previous-site branch: 0.5 seconds for
//! inherited settings, otherwise 1 second to the saved private view. Following
//! resumes during that return; it does not compete with an active conversation.
//!
//! Talk owns the camera state, so its gestures cannot invoke Normal's pitch/
//! distance coupling or first-person transition. Its completed-zoom flag is
//! retained across entries, matching the lifetime of the source state.
//!
//! Actual bound furniture is part of the subject group for furniture stories.
//! Its combined footprint widens the existing camera, so a speaking furniture
//! character cannot remain outside an NPC-only close-up.

use crate::camera::{
    CameraSetting, CameraStateType, CameraTween, FieldCameraModel, FieldCameraState,
    NormalCameraMemory, PrevSiteType,
};
use crate::character::AvatarRoot;
use crate::character_material::ToonMaterials;
use crate::inactive_nodes::SiteSettled;
use crate::player_talk::PlayerTalkSession;
use crate::site::{GroundEpoch, SiteActive, SiteRoot};
use crate::talk::ActiveTalk;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// 对话机位距离（源 `TALK_CAMERA_DISTANCE`，入场缓动的目标距离）。
const TALK_CAMERA_DISTANCE: f32 = 3.7;
/// 机位角偏置（源 `CameraOffsetAngle`）。
const CAMERA_OFFSET_ANGLE: f32 = 37.0;
/// 背身角（源 `TalkBackCameraAngle`，大角度差时的回拉角）。
const TALK_BACK_CAMERA_ANGLE: f32 = 45.0;
/// 入场时长（源 `ShowTalkCamera` 的缓动实参）。
const ENGAGE_SECONDS: f32 = 0.7;
/// 还原时长（源回常态时的设定转移时长）。
const RESTORE_SECONDS: f32 = 1.0;

/// 一个机位：跟随模型的五个自由度（角度全部角度制，视场为度）。
#[derive(Debug, Clone, Copy)]
struct CamPoint {
    look_at: Vec3,
    fov_deg: f32,
    pitch: f32,
    yaw: f32,
    distance: f32,
}

/// Normal private model and per-site transfer values. Framing initializes the
/// construction values; leaving Normal updates the live return target.
#[derive(Resource)]
pub(crate) struct NormalSnapshot {
    point: CamPoint,
    /// Per-site transfer values are refreshed even when a Normal return tween
    /// has not completed; the private Normal model above keeps its old target.
    transfer: Option<CamPoint>,
    /// 抓取时的站点代数：站点变更后旧快照不得再当还原目标（入场也拒
    /// 绝引用它，等新站点取景）。
    epoch: u64,
}

/// 对话相机在播状态。插入即接管机位，撤除即交还。
#[derive(Resource)]
pub(crate) struct TalkCamera {
    phase: Phase,
    input_ready: bool,
}

impl TalkCamera {
    pub(crate) fn accepts_input(&self) -> bool {
        self.input_ready && !matches!(self.phase, Phase::Restore { .. })
    }
}

enum Phase {
    /// 入场缓动（0.7 秒，五量全写）。
    Engage {
        from: CamPoint,
        to: CamPoint,
        elapsed: f32,
    },
    /// 驻留：取景点与视场钉住，角度/距离留给输入。
    Hold { point: CamPoint },
    /// 还原缓动（1.0 秒；取景点不在此缓动名下——跟随律每帧牵引）。
    Restore {
        from: CamPoint,
        to: CamPoint,
        elapsed: f32,
        duration: f32,
        inherit: bool,
    },
}

/// 本帧相位推进的结果：要写的机位量（各相位写的字段集不同）。
enum FrameWrite {
    /// 入场：缓动写全部五量。
    Tween(CamPoint),
    /// 驻留：只钉取景点与视场。
    Pin { look_at: Vec3, fov_deg: f32 },
    /// 还原：写角度/距离/视场；取景点归跟随律（它排在本系统前面，
    /// 当帧值已在模型里）。
    Angles {
        fov_deg: f32,
        pitch: f32,
        yaw: f32,
        distance: f32,
    },
}

/// 相位迁移（写入之后应用）。
enum Transition {
    None,
    /// 入场缓动走完 → 驻留。
    ToHold(CamPoint),
    /// 还原缓动走完 → 撤资源。
    Done,
}

/// 俯仰/偏航角（角度制）转视线方向——相机模块同名私有函数的照式复
/// 制（源 `FieldCamera.GetPosition`：眼位 = 取景点 + 偏移 + 视线方向
/// × 距离；逐分量展开式钉在相机模块）。
fn view_dir(pitch_deg: f32, yaw_deg: f32) -> Vec3 {
    let pitch = pitch_deg.to_radians();
    let yaw = yaw_deg.to_radians();
    Vec3::new(
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        -(pitch.cos() * yaw.cos()),
    )
}

/// 源 `ConvertAngle180` 的照抄：角度差归到 [-180, 180]，缓动逐分量走
/// 最短有符号路径。
fn convert_angle180(x: f32) -> f32 {
    let r = x % 360.0;
    let r = if r <= 180.0 { r } else { r - 360.0 };
    if r < -180.0 {
        r + 360.0
    } else {
        r
    }
}

/// 源缓动档（SetEase(6) = OutQuad）。
fn out_quad(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

/// 机位角（源 `GetStartCameraAngle`，式子见模块注释）。a/b 都是 XZ 平
/// 面向量，这里用标量展开（az/bz 是 z 分量，不是别的轴）。
fn start_camera_angle(list: &[Vec3], cam_pos: Vec3, yaw: f32) -> f32 {
    // The live transforms are in the imported (-x, y, z) frame, while yaw
    // remains the source camera model's angle. Reflection preserves the dot
    // product but reverses the signed cross product: recover source-space
    // deltas before selecting the offset side and applying the yaw clamps.
    let (ax, az) = (list[0].x - list[1].x, list[1].z - list[0].z);
    let (bx, bz) = (list[0].x - cam_pos.x, cam_pos.z - list[0].z);
    let denom = ((ax * ax + az * az) * (bx * bx + bz * bz)).sqrt();
    let theta = if denom >= 1.0e-15 {
        let cosine = ((az * bz + ax * bx) / denom).clamp(-1.0, 1.0);
        cosine.acos() * 57.296
    } else {
        0.0
    };
    let signed = if az * bx - ax * bz >= 0.0 {
        theta
    } else {
        -theta
    };
    let offset = if signed <= 0.0 {
        -CAMERA_OFFSET_ANGLE
    } else {
        CAMERA_OFFSET_ANGLE
    };
    yaw - signed + offset
}

/// 入场目标偏航 = 机位角过两级钳（源 `ShowTalkCamera`：差只在钳前算
/// 一次；第二级比较读的是过了第一级钳之后的角）。
fn clamped_talk_yaw(list: &[Vec3], cam_pos: Vec3, yaw: f32) -> f32 {
    let mut angle = start_camera_angle(list, cam_pos, yaw);
    let diff = (angle - yaw).abs();
    if diff >= 90.0 {
        angle = yaw + if angle <= yaw { -90.0 } else { 90.0 };
    }
    if diff >= 110.0 {
        angle = yaw
            + if angle <= yaw {
                -TALK_BACK_CAMERA_ANGLE
            } else {
                TALK_BACK_CAMERA_ANGLE
            };
    }
    angle
}

/// 入场目标点（源 `ShowTalkCamera` 的取位段）：≥ 3 → 非玩家条目均值；
/// 恰 2 → 全表首条。Y 恒改写为当前坐标框架内的 SitePosition.y。
fn talk_target(list: &[Vec3], player_index: Option<usize>, site_position_y: f32) -> Vec3 {
    let mut target = if list.len() >= 3 {
        let others: Vec<Vec3> = list
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != player_index)
            .map(|(_, p)| *p)
            .collect();
        // 玩家恰一条且在末位：count ≥ 3 时非玩家条目 ≥ 2，分母恒正；
        // max(1) 只挡「全表都是玩家」这一不可达形。
        let count = others.len().max(1) as f32;
        others.iter().sum::<Vec3>() / count
    } else {
        list[0]
    };
    target.y = site_position_y;
    target
}

/// 视角锥当前视场（度）。单相机恒为透视锥——正交锥没有对话机位可
/// 言，响亮拒绝（fail-closed）。
fn projection_fov_deg(projection: &Projection) -> f32 {
    match projection {
        Projection::Perspective(perspective) => perspective.fov.to_degrees(),
        _ => panic!("相机投影不是透视锥——对话机位律只对透视锥成立"),
    }
}

/// 机位写入：视场（度→弧度）+ 眼位（源 `FieldCamera.GetPosition` 同
/// 式——取景点上抬偏移、眼位沿视线方向退距离、朝向取景点）。
fn write_camera(
    model: &FieldCameraModel,
    projection: &mut Projection,
    camera_transform: &mut Transform,
    fov_deg: f32,
) {
    match projection {
        Projection::Perspective(perspective) => perspective.fov = fov_deg.to_radians(),
        _ => panic!("相机投影不是透视锥——对话机位律只对透视锥成立"),
    }
    let pivot = model.look_at + model.offset;
    let eye = pivot + view_dir(model.pitch, model.yaw) * model.distance;
    *camera_transform = Transform::from_translation(eye).looking_at(pivot, Vec3::Y);
}

#[derive(Default)]
pub(crate) struct CaptureState {
    last_epoch: u64,
    awaiting: bool,
}

/// PostUpdate（取景之后、跟随之前）：抓常态构造机位快照。
///
/// 边沿条件 =「站点代数已变 ∧ 取景闩已撤 ∧ 模型在场」：取景完成那帧
/// 的模型还是构造值（跟随律排在本系统之后，还没碰取景点），取景点即
/// 站点中心 + 地表高。代数与取景闩同批插入、撤闩与插模型同批——本条
/// 件在任意同步点交错下都恰好抓到新模型一次（首站点同抓）。
pub(crate) fn capture_normal(
    mut local: Local<CaptureState>,
    epoch: Option<Res<GroundEpoch>>,
    settled: Option<Res<SiteSettled>>,
    model: Option<Res<FieldCameraModel>>,
    mut commands: Commands,
) {
    if let Some(epoch) = &epoch {
        if epoch.0 != local.last_epoch {
            local.last_epoch = epoch.0;
            local.awaiting = true;
        }
    }
    if !local.awaiting || settled.is_some() {
        return;
    }
    let Some(model) = &model else {
        return;
    };
    let point = CamPoint {
        look_at: model.look_at,
        fov_deg: model.fov,
        pitch: model.pitch,
        yaw: model.yaw,
        distance: model.distance,
    };
    commands.insert_resource(NormalSnapshot {
        point,
        transfer: None,
        epoch: local.last_epoch,
    });
    local.awaiting = false;
    info!(
        "[talk-cam] 常态机位快照：取景点 ({:.2},{:.2},{:.2}) 俯仰 {:+.1} 偏航 {:+.1} 距离 {:.2} 视场 {:.1}（站点代数 {}）",
        point.look_at.x,
        point.look_at.y,
        point.look_at.z,
        point.pitch,
        point.yaw,
        point.distance,
        point.fov_deg,
        local.last_epoch
    );
}

#[derive(Default)]
pub(crate) struct AdvanceState {
    epoch_seen: u64,
    /// 入场跳过只留名一次（条件恢复后重置），不逐帧刷屏。
    skip_logged: bool,
    /// Normal.OnExit does not overwrite its private model while its return
    /// tween is incomplete. A new conversation keeps that previous target.
    preserve_normal: bool,
    /// The source Talk state retains its completed-zoom flag across entries.
    zoom_completed: bool,
}

#[derive(SystemParam)]
pub(crate) struct TalkSceneContext<'w, 's> {
    epoch: Option<Res<'w, GroundEpoch>>,
    active: Option<Res<'w, SiteActive>>,
    previous_site: Res<'w, PrevSiteType>,
    setting: Option<Res<'w, CameraSetting>>,
    normal_tween: Option<Res<'w, CameraTween>>,
    normal_memory: Option<Res<'w, NormalCameraMemory>>,
    field_state: ResMut<'w, FieldCameraState>,
    transforms: Query<'w, 's, &'static GlobalTransform>,
    npc_bones: Query<'w, 's, &'static ToonMaterials>,
    player_avatar:
        Query<'w, 's, (&'static GlobalTransform, &'static mut Visibility), With<AvatarRoot>>,
    site_roots: Query<'w, 's, &'static GlobalTransform, With<SiteRoot>>,
}

impl TalkSceneContext<'_, '_> {
    fn npc_hip(&self, entity: Entity) -> Option<Vec3> {
        let skeleton = self.npc_bones.get(entity).ok()?;
        self.transforms
            .get(skeleton.hips_entity())
            .ok()
            .map(GlobalTransform::translation)
    }
}

/// PostUpdate（跟随之后、一切读机位的系统之前）：对话相机的相位推进。
///
/// 会话在场而无对话相机 → 尝试入场（条件不足的帧跳过留名，下一帧再
/// 试——fail-closed，相机留常态）；会话退场 → 起还原；还原中会话又
/// 起 → 杀在途缓动重新入场（源新入场先杀旧缓动）。
#[allow(clippy::type_complexity)]
pub(crate) fn advance(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Local<AdvanceState>,
    talk: Option<Res<ActiveTalk>>,
    session: Option<Res<PlayerTalkSession>>,
    mut model: Option<ResMut<FieldCameraModel>>,
    mut snapshot: Option<ResMut<NormalSnapshot>>,
    talk_cam: Option<ResMut<TalkCamera>>,
    mut scene: TalkSceneContext,
    mut cameras: Query<(&mut Projection, &mut Transform), With<Camera3d>>,
) {
    let present = talk.as_deref().is_some_and(ActiveTalk::includes_player) || session.is_some();
    let dt = time.delta_secs();

    // 站点代数守卫：对话中换站点——快照与机位全部失效。视场是本模块
    // 之外没有写手的量（取景只写模型，不改视角锥），先回写再撤；模型
    // 与机位由新站点取景重置，取景点归跟随律。
    let epoch_now = scene.epoch.as_deref().map_or(0, |e| e.0);
    let epoch_changed = epoch_now != state.epoch_seen;
    state.epoch_seen = epoch_now;
    if epoch_changed && talk_cam.is_some() {
        let fov_deg = snapshot.as_ref().map_or_else(
            || model.as_deref().map_or(0.0, |m| m.fov),
            |s| s.point.fov_deg,
        );
        let (mut projection, _) = cameras
            .single_mut()
            .expect("应恰有一台 3D 相机（站点守卫回写视场）");
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.fov = fov_deg.to_radians();
        }
        info!(
            "[talk-cam] 站点在对话中变更（代数 {epoch_now}）：视场回写 {fov_deg:.1} 度后撤下对话相机，新站点取景后可再入场"
        );
        commands.remove_resource::<TalkCamera>();
        scene.field_state.0 = CameraStateType::Normal;
        state.preserve_normal = false;
        state.skip_logged = false;
        return;
    }

    let Some(mut cam) = talk_cam else {
        if !present {
            state.skip_logged = false;
            state.preserve_normal = false;
            return;
        }
        // 入场尝试（每帧可重试）。取位表：参演 NPC 髋变换 + 玩家 avatar
        // 视变换（末位；玩家不在场则无末位，与源「空引用过滤恒过」同
        // 形）。
        let mut list = Vec::new();
        if let Some(talk) = &talk {
            for &(_, entity) in talk.participants() {
                if let Some(position) = scene.npc_hip(entity) {
                    list.push(position);
                }
            }
        } else if let Some(session) = &session {
            if let Some(position) = scene.npc_hip(session.npc_entity()) {
                list.push(position);
            }
        }
        let has_fixtures = talk.as_ref().is_some_and(|talk| !talk.fixture_instances().is_empty());
        if let Some(talk) = &talk {
            for &(_, entity) in talk.fixture_instances() {
                if let Ok(pose) = scene.transforms.get(entity) { list.push(pose.translation()); }
            }
        }
        let player_index = scene.player_avatar.single().ok().map(|(global, _)| {
            list.push(global.translation());
            list.len() - 1
        });
        if list.len() < 2 {
            if !state.skip_logged {
                state.skip_logged = true;
                info!(
                    "[talk-cam] 入场跳过：取位表不足两名（{} 名）——源里此处取位越界即异常，本侧相机留常态",
                    list.len()
                );
            }
            return;
        }
        let Some(snapshot) = &mut snapshot else {
            if !state.skip_logged {
                state.skip_logged = true;
                info!("[talk-cam] 入场跳过：常态机位快照未就绪——相机留常态");
            }
            return;
        };
        if snapshot.epoch != epoch_now {
            if !state.skip_logged {
                state.skip_logged = true;
                info!(
                    "[talk-cam] 入场跳过：快照代数 {} 落后于当前 {}——等新站点取景",
                    snapshot.epoch, epoch_now
                );
            }
            return;
        }
        let Some(model) = &mut model else {
            if !state.skip_logged {
                state.skip_logged = true;
                info!("[talk-cam] 入场跳过：相机模型未立——相机留常态");
            }
            return;
        };
        let Ok((projection, camera_transform)) = cameras.single_mut() else {
            if !state.skip_logged {
                state.skip_logged = true;
                info!("[talk-cam] 入场跳过：3D 相机不在——相机留常态");
            }
            return;
        };
        // SitePosition is the site's origin, not a point sampled from its mesh.
        // The active site's shell, modules and navigation share this frame.
        let Some(site_root) = scene.site_roots.iter().next() else {
            if !state.skip_logged {
                state.skip_logged = true;
                info!("[talk-cam] 入场跳过：活动站点坐标框架未就绪——相机留常态");
            }
            return;
        };
        // 入场（源 ShowTalkCamera → DoTweenCameraSetting）。
        let cam_pos = camera_transform.translation;
        let yaw = model.yaw;
        let target_yaw = clamped_talk_yaw(&list, cam_pos, yaw);
        let target = talk_target(&list, player_index, site_root.translation().y);
        let target_distance = if has_fixtures {
            let radius = list.iter().enumerate().filter(|(i, _)| Some(*i) != player_index)
                .map(|(_, point)| Vec2::new(point.x - target.x, point.z - target.z).length()).fold(0., f32::max);
            TALK_CAMERA_DISTANCE.max((radius + 0.8) * 2.4)
        } else { TALK_CAMERA_DISTANCE };
        let from = CamPoint {
            look_at: model.look_at,
            fov_deg: projection_fov_deg(&projection),
            pitch: model.pitch,
            yaw,
            distance: model.distance,
        };
        // Save the live Normal view, not the station's original framing view.
        // Normal's private FOV is a construction value, while its transfer
        // entry is written unconditionally on exit.
        if !state.preserve_normal && scene.field_state.0 == CameraStateType::Normal {
            let inherit = scene.active.as_deref().is_some_and(|active| {
                crate::camera::is_inherit_camera_setting(&active.category, &scene.previous_site.0)
            });
            if scene.normal_tween.is_none() || inherit {
                snapshot.point.look_at = from.look_at;
                snapshot.point.pitch = from.pitch;
                snapshot.point.yaw = from.yaw;
                snapshot.point.distance = from.distance;
            } else if let Some(memory) = scene.normal_memory.as_deref() {
                // An unfinished FPS-to-Normal return retains the same private
                // Normal target; the site-framing snapshot is not that target.
                snapshot.point.look_at = memory.look_at;
                snapshot.point.pitch = memory.pitch;
                snapshot.point.yaw = memory.yaw;
                snapshot.point.distance = memory.distance;
            }
        }
        if scene.field_state.0 == CameraStateType::Fps {
            // FPS.OnExit must run when Talk replaces it too: the avatar must
            // not remain hidden after the camera ceases to be first-person.
            if let Ok((_, mut visibility)) = scene.player_avatar.single_mut() {
                *visibility = Visibility::Visible;
            }
            if let Some(memory) = scene.normal_memory.as_deref() {
                snapshot.point.look_at = memory.look_at;
                snapshot.point.pitch = memory.pitch;
                snapshot.point.yaw = memory.yaw;
                snapshot.point.distance = memory.distance;
                snapshot.transfer = Some(CamPoint {
                    look_at: memory.look_at,
                    pitch: memory.pitch,
                    yaw: memory.yaw,
                    distance: memory.distance,
                    fov_deg: memory.fov,
                });
            }
        } else {
            snapshot.transfer = Some(CamPoint {
                fov_deg: model.fov,
                ..from
            });
        }
        if let Some(active) = scene
            .active
            .as_deref()
            .filter(|_| scene.field_state.0 != CameraStateType::Fps)
        {
            commands.insert_resource(NormalCameraMemory {
                site: active.site_type.clone(),
                look_at: model.look_at,
                distance: model.distance,
                yaw: model.yaw,
                pitch: model.pitch,
                fov: model.fov,
            });
        }
        commands.remove_resource::<CameraTween>();
        // Talk's copied model retains the authored distance limits. OnEnter
        // publishes these two fields, without replacing shared pitch bounds.
        if let Some(setting) = scene.setting.as_deref() {
            model.min_distance = setting.min_distance;
            model.max_distance = setting.max_distance;
        }
        scene.field_state.0 = CameraStateType::Talk;
        state.preserve_normal = false;
        let to = CamPoint {
            look_at: target,
            fov_deg: snapshot.point.fov_deg,
            // 源把当前俯仰原样传作缓动目标——俯仰不动。
            pitch: model.pitch,
            yaw: target_yaw,
            distance: target_distance,
        };
        info!(
            "[talk-cam] 对话相机入场：目标 ({:.2},{:.2},{:.2}) 偏航 {yaw:+.1}→{target_yaw:+.1} 俯仰 {:+.1} 保持 距离 {:.2}→{target_distance:.2} 视场 {:.1}→{:.1}，{ENGAGE_SECONDS}s OutQuad（参演 {} 名{}）",
            to.look_at.x,
            to.look_at.y,
            to.look_at.z,
            from.pitch,
            from.distance,
            from.fov_deg,
            to.fov_deg,
            list.len(),
            if player_index.is_some() {
                "，玩家在末位"
            } else {
                ""
            }
        );
        state.skip_logged = false;
        commands.insert_resource(TalkCamera {
            phase: Phase::Engage {
                from,
                to,
                elapsed: 0.0,
            },
            input_ready: state.zoom_completed,
        });
        return;
    };

    if present && matches!(cam.phase, Phase::Restore { .. }) {
        // 还原中会话又起：源新入场先杀在途缓动。撤除是延迟命令——本帧
        // 不再推进（缓动停在当前值一帧），下一帧走入场路径从当前值起。
        info!("[talk-cam] 还原中被新会话顶掉：杀在途缓动，从当前机位重新入场");
        commands.remove_resource::<TalkCamera>();
        state.preserve_normal = matches!(cam.phase, Phase::Restore { inherit: false, .. });
        state.skip_logged = false;
        return;
    }

    if !present && !matches!(cam.phase, Phase::Restore { .. }) {
        // 会话退场：起还原。
        let Some(snapshot) = &snapshot else {
            // 不可达：入场即需快照，快照只换不删。留名拒绝。
            info!("[talk-cam] 还原缺常态快照（不可达路径）：直接撤下对话相机");
            commands.remove_resource::<TalkCamera>();
            scene.field_state.0 = CameraStateType::Normal;
            return;
        };
        let Some(model) = &mut model else {
            info!("[talk-cam] 还原缺相机模型（不可达路径）：直接撤下对话相机");
            commands.remove_resource::<TalkCamera>();
            scene.field_state.0 = CameraStateType::Normal;
            return;
        };
        let Ok((projection, _)) = cameras.single_mut() else {
            commands.remove_resource::<TalkCamera>();
            scene.field_state.0 = CameraStateType::Normal;
            return;
        };
        let from = CamPoint {
            look_at: model.look_at,
            fov_deg: projection_fov_deg(&projection),
            pitch: model.pitch,
            yaw: model.yaw,
            distance: model.distance,
        };
        let inherit = scene.active.as_deref().is_some_and(|active| {
            crate::camera::is_inherit_camera_setting(&active.category, &scene.previous_site.0)
        });
        let to = if inherit {
            snapshot.transfer.unwrap_or(snapshot.point)
        } else {
            snapshot.point
        };
        let duration = if inherit { 0.5 } else { RESTORE_SECONDS };
        if let Some(setting) = scene.setting.as_deref() {
            model.min_distance = setting.min_distance;
            model.max_distance = setting.max_distance;
            model.min_pitch = setting.min_pitch;
            model.max_pitch = setting.max_pitch;
            model.offset = setting.offset;
        }
        scene.field_state.0 = CameraStateType::Normal;
        info!(
            "[talk-cam] 对话相机还原：回对话前机位（俯仰 {:+.1} 偏航 {:+.1} 距离 {:.2} 视场 {:.1}），{duration}s OutQuad；取景点归跟随律",
            to.pitch, to.yaw, to.distance, to.fov_deg
        );
        cam.phase = Phase::Restore {
            from,
            to,
            elapsed: 0.0,
            duration,
            inherit,
        };
    }

    // 相位推进（先走表，再落笔——借用分两段，避开相位自迁移的死锁）。
    if let Phase::Engage { elapsed, .. } | Phase::Restore { elapsed, .. } = &mut cam.phase {
        *elapsed += dt;
    }
    let (write, transition) = match &cam.phase {
        Phase::Engage { from, to, elapsed } => {
            let eased = out_quad((elapsed / ENGAGE_SECONDS).clamp(0.0, 1.0));
            let point = CamPoint {
                look_at: from.look_at.lerp(to.look_at, eased),
                fov_deg: from.fov_deg + (to.fov_deg - from.fov_deg) * eased,
                pitch: from.pitch + convert_angle180(to.pitch - from.pitch) * eased,
                yaw: from.yaw + convert_angle180(to.yaw - from.yaw) * eased,
                distance: from.distance + (to.distance - from.distance) * eased,
            };
            let done = *elapsed >= ENGAGE_SECONDS;
            (
                FrameWrite::Tween(point),
                if done {
                    Transition::ToHold(*to)
                } else {
                    Transition::None
                },
            )
        }
        Phase::Hold { point } => (
            FrameWrite::Pin {
                look_at: point.look_at,
                fov_deg: point.fov_deg,
            },
            Transition::None,
        ),
        Phase::Restore {
            from,
            to,
            elapsed,
            duration,
            ..
        } => {
            let eased = out_quad((elapsed / duration).clamp(0.0, 1.0));
            let angles = FrameWrite::Angles {
                fov_deg: from.fov_deg + (to.fov_deg - from.fov_deg) * eased,
                pitch: from.pitch + convert_angle180(to.pitch - from.pitch) * eased,
                yaw: from.yaw + convert_angle180(to.yaw - from.yaw) * eased,
                distance: from.distance + (to.distance - from.distance) * eased,
            };
            let done = *elapsed >= *duration;
            (
                angles,
                if done {
                    Transition::Done
                } else {
                    Transition::None
                },
            )
        }
    };

    // 落笔。
    let Some(model) = &mut model else {
        // 不可达：模型只在站点间被替换，从不缺席。留名撤除。
        info!("[talk-cam] 相位落笔缺相机模型（不可达路径）：撤下对话相机");
        commands.remove_resource::<TalkCamera>();
        scene.field_state.0 = CameraStateType::Normal;
        return;
    };
    let Ok((mut projection, mut camera_transform)) = cameras.single_mut() else {
        return;
    };
    match write {
        FrameWrite::Tween(point) => {
            model.look_at = point.look_at;
            model.pitch = point.pitch;
            model.yaw = point.yaw;
            model.distance = point.distance;
            write_camera(model, &mut projection, &mut camera_transform, point.fov_deg);
        }
        FrameWrite::Pin { look_at, fov_deg } => {
            model.look_at = look_at;
            write_camera(model, &mut projection, &mut camera_transform, fov_deg);
        }
        FrameWrite::Angles {
            fov_deg,
            pitch,
            yaw,
            distance,
        } => {
            model.pitch = pitch;
            model.yaw = yaw;
            model.distance = distance;
            write_camera(model, &mut projection, &mut camera_transform, fov_deg);
        }
    }

    match transition {
        Transition::None => {}
        Transition::ToHold(point) => {
            cam.phase = Phase::Hold { point };
            cam.input_ready = true;
            state.zoom_completed = true;
        }
        Transition::Done => {
            commands.remove_resource::<TalkCamera>();
            scene.field_state.0 = CameraStateType::Normal;
            state.preserve_normal = false;
            info!("[talk-cam] 还原完成：回到对话前机位，跟随律继续接管");
        }
    }
}
