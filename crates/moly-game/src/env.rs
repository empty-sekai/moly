//! 站点全局量：一张表，一帧写一次，站点材质按名消费。
//!
//! 表的字段与真源站点 shader 的全局块一一对应；`FieldObjectGlobals` 是
//! 全集，Ground 族读的是它的子集，不另立第二张表。资源以中性初值落表
//! ——现象未到、相机未刷时，消费它的每个式子精确化简为无贡献，而不是
//! 静默无光。现象侧的量（光、雾、投影色）由天气系统每帧写入
//! （`weather.rs`）；晴天常量只在 Startup 写一次 toon 边界；相机态每帧
//! 刷新。

use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::{Buffer, BufferInitDescriptor, BufferUsages};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::transform::TransformSystems;
use moly_law::shading::fieldobject::FieldObjectGlobals;

use crate::camera;
use crate::light;

/// 晴天现象的全局 toon 阈值与平滑度，照抄源值。
const SUNNY_EDGE_THRESHOLD: f32 = 0.649_999_976_158_142_1;
const SUNNY_EDGE_SMOOTHNESS: f32 = 0.039_999_999_105_930_33;

/// 阴影 pass 推的全局投影色，照抄源值。站点族 Base 片元不读它；表带着，
/// 阴影消费方接进来时读到的就是源值。
const SUNNY_DROP_SHADOW_COLOR: [f32; 4] = [0.5106, 0.5549, 0.7075, 0.3333];

/// Ground/Water/Birthday 落影的 maskramp 边缘对（源全局量
/// `_ShadowMaskEdge1` / `_ShadowMaskEdge2`，图形配置常量，与天气无关）。
const SHADOW_MASK_EDGES: [f32; 4] = [0.0, 0.8, 0.0, 0.0];

/// DropItem 族的四个全局标量（源 `MysekaiGraphicsConfig`，与天气无关）：
/// (现象光强度, 阴影边缘平滑度, 阴影边缘阈值, 常规明暗强度)。
const DROPITEM_GLOBALS: [f32; 4] = [0.4, 0.04, 0.65, 1.0];

/// 高度淡出的天空底色（源全局量 `_MysekaiSkyBottomColor`，按天气由环境
/// 资产下发）。这里只是环境加载前的占位；天气解析后每帧由
/// ramp.skyBottomColor 覆写，并与 SetEnvironmentData 一同提交。
const SKY_BOTTOM_COLOR_NEUTRAL: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

/// 表的 GPU 布局：26 个 vec4 槽、416 字节。槽序是本文件与
/// `shaders/site_material.wgsl` / `shaders/fixture_material.wgsl` 里 `SiteEnv`
/// 结构体之间的契约，两边同改。
pub const ENV_SLOTS: usize = 26;
pub const ENV_BYTES: usize = ENV_SLOTS * 16;

/// 一帧的站点全局量。
#[derive(Debug, Clone, Resource, ExtractResource)]
pub struct SiteEnv {
    pub globals: FieldObjectGlobals,
    pub drop_shadow_color: [f32; 4],
    /// 现象自发光类型（全局 int `_MysekaiPhenomenaEmissionType`）：1 = 日系
    /// 现象、2 = 夜系现象。f32 存整数值，shader 侧取整消费。中性 0 = 两类
    /// 门全关（自发光贡献精确为零），由天气系统按档写入。
    pub emission_type: f32,
    /// Ground/Water/Birthday 落影 maskramp 的边缘对 (.x=E1, .y=E2)。
    pub shadow_mask_edges: [f32; 4],
    /// DropItem 族四个全局标量（见 [`DROPITEM_GLOBALS`]）。
    pub dropitem_globals: [f32; 4],
    /// 高度淡出混合目标，由当前已提交的现象 ramp 元数据驱动。
    pub sky_bottom_color: [f32; 4],
}

impl SiteEnv {
    /// 中性初值：每个消费式子精确化简为无贡献。
    ///
    /// - 现象光/阴面色 (1,1,1,1)：混合项与阴面色差恰好为零。
    /// - toon 边界取 (0.5, 0.5)：half-Lambert ∈ [0,1] 落在界内，斜坡因子
    ///   有限——边界取 (0,0) 会让分母为零、把 `t·0` 变成 `inf·0 = NaN`。
    /// - 雾三件全零：斜坡 0、雾色 0，距离项与高度项都乘零。
    /// - 投影参数取 (1, 1, 4000, 0.00025)：雾深式子用远近裁剪面做除法，
    ///   退化值（n == f）会产生 NaN；这组与站点相机一致。
    /// - 宝藏强度 0：径向衰减因子有限，贡献为零。
    pub fn neutral() -> Self {
        SiteEnv {
            globals: FieldObjectGlobals {
                light_vector: [0.0, 0.0, 1.0],
                phenomena_directional_light_color: [1.0, 1.0, 1.0, 1.0],
                phenomena_shade_color: [1.0, 1.0, 1.0, 1.0],
                edge_threshold: 0.5,
                edge_smoothness: 0.5,
                treasure_positions: [[0.0; 3]; 2],
                treasure_shadow_intensity: [0.0; 2],
                camera_position: [0.0, 0.0, 0.0],
                ortho_params: [0.0; 4],
                view_matrix: Mat4::IDENTITY.to_cols_array(),
                screen_params: [1.0, 1.0, 1.0, 1.0],
                fog_params: [0.0; 4],
                fog_near_color: [0.0; 4],
                fog_far_color: [0.0; 4],
                projection_params: [1.0, 1.0, 4000.0, 0.00025],
                mip_bias: [0.0; 2],
                time: [0.0; 4],
            },
            drop_shadow_color: [0.0; 4],
            emission_type: 0.0,
            shadow_mask_edges: SHADOW_MASK_EDGES,
            dropitem_globals: DROPITEM_GLOBALS,
            sky_bottom_color: SKY_BOTTOM_COLOR_NEUTRAL,
        }
    }

    /// 按上面的槽序摊平成字节。
    pub fn gpu_bytes(&self) -> Vec<u8> {
        let g = &self.globals;
        let mut bytes = Vec::with_capacity(ENV_BYTES);
        push(&mut bytes, [g.light_vector[0], g.light_vector[1], g.light_vector[2], 0.0]);
        push(&mut bytes, g.phenomena_directional_light_color);
        push(&mut bytes, g.phenomena_shade_color);
        push(&mut bytes, self.drop_shadow_color);
        push(&mut bytes, [g.edge_threshold, 0.0, 0.0, 0.0]);
        push(&mut bytes, [g.edge_smoothness, 0.0, 0.0, 0.0]);
        push(
            &mut bytes,
            [
                g.treasure_positions[0][0],
                g.treasure_positions[0][1],
                g.treasure_positions[0][2],
                0.0,
            ],
        );
        push(
            &mut bytes,
            [
                g.treasure_positions[1][0],
                g.treasure_positions[1][1],
                g.treasure_positions[1][2],
                0.0,
            ],
        );
        push(
            &mut bytes,
            [g.treasure_shadow_intensity[0], g.treasure_shadow_intensity[1], 0.0, 0.0],
        );
        push(
            &mut bytes,
            [g.camera_position[0], g.camera_position[1], g.camera_position[2], 0.0],
        );
        push(&mut bytes, g.ortho_params);
        push(&mut bytes, g.view_matrix[0..4].try_into().unwrap());
        push(&mut bytes, g.view_matrix[4..8].try_into().unwrap());
        push(&mut bytes, g.view_matrix[8..12].try_into().unwrap());
        push(&mut bytes, g.view_matrix[12..16].try_into().unwrap());
        push(&mut bytes, g.screen_params);
        push(&mut bytes, g.fog_params);
        push(&mut bytes, g.fog_near_color);
        push(&mut bytes, g.fog_far_color);
        push(&mut bytes, g.projection_params);
        push(&mut bytes, [g.mip_bias[0], g.mip_bias[1], 0.0, 0.0]);
        push(&mut bytes, g.time);
        push(&mut bytes, [self.emission_type, 0.0, 0.0, 0.0]);
        push(&mut bytes, self.shadow_mask_edges);
        push(&mut bytes, self.dropitem_globals);
        push(&mut bytes, self.sky_bottom_color);
        debug_assert_eq!(bytes.len(), ENV_BYTES);
        bytes
    }
}

fn push(bytes: &mut Vec<u8>, slot: [f32; 4]) {
    for component in slot {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
}

/// Startup：中性表落位。
pub fn insert_neutral(mut commands: Commands) {
    commands.insert_resource(SiteEnv::neutral());
}

/// Startup：晴天现象常量写入（光向、光色、阴面色、toon 边界、投影色）。
/// 现象档到齐前（天气系统在异步装载）这一份就是画面基线；天气系统每帧
/// 写入后，光与雾一侧由它接管——这里只留 toon 边界这类**非现象驱动**的
/// 晴天常量与装载期的基线值。
pub fn apply_sunny_phenomena(mut env: ResMut<SiteEnv>) {
    let toward = light::dir_toward_light(light::ANGLE_XZ, light::ANGLE_Y);
    env.globals.light_vector = [toward.x, toward.y, toward.z];
    // 晴天现象的平行光色，源值纯白、alpha 1。
    env.globals.phenomena_directional_light_color = [1.0, 1.0, 1.0, 1.0];
    env.globals.phenomena_shade_color = [
        light::SHADE_COLOR[0],
        light::SHADE_COLOR[1],
        light::SHADE_COLOR[2],
        1.0,
    ];
    env.globals.edge_threshold = SUNNY_EDGE_THRESHOLD;
    env.globals.edge_smoothness = SUNNY_EDGE_SMOOTHNESS;
    env.drop_shadow_color = SUNNY_DROP_SHADOW_COLOR;
    // 晴现象的自发光类型 = 1（日系）：门比 `_BrightPhenomenaEmission`。
    // 天气系统就绪后由它按档接管这一槽。
    env.emission_type = 1.0;
}

/// PostUpdate：相机态刷新——眼位、视图矩阵、投影参数、屏幕参数、时间。
/// 真源把这些量当作每帧的全局 uniform，这里同节奏。
pub fn write_frame_state(
    time: Res<Time>,
    mut env: ResMut<SiteEnv>,
    // 只认 3D 主相机：气泡层有一台 2D 覆盖相机共存，不过滤的话 single()
    // 失败早退——time 冻结在 0，站点材质的 uv 滚动与树摆全停（河流停流
    // 回归的根因）。
    cameras: Query<(&GlobalTransform, &Projection, &Camera), With<Camera3d>>,
) {
    let Ok((global, projection, camera)) = cameras.single() else {
        return;
    };
    let (near, far, ortho) = match projection {
        Projection::Perspective(p) => (p.near, p.far, [0.0, 0.0, 0.0, 0.0]),
        // 源程序只读 unity_OrthoParams.w（1 = 正交）；其余分量不读。
        Projection::Orthographic(o) => (o.near, o.far, [0.0, 0.0, 0.0, 1.0]),
        Projection::Custom(_) => {
            panic!("站点全局量不认识自定义投影：Unity 投影参数没有对应物")
        }
    };
    // 源的 _ProjectionParams = (翻转位, 近裁剪, 远裁剪, 1/远裁剪)；
    // 翻转位在 GL 约定（帧缓冲 y 自下而上，即本管线的约定）下恒 1。
    env.globals.projection_params = [1.0, near, far, 1.0 / far];
    env.globals.ortho_params = ortho;
    let translation = global.translation();
    env.globals.camera_position = [translation.x, translation.y, translation.z];
    // Unity 列主序视图矩阵：世界到相机的逆。四列按原布局摊平。
    env.globals.view_matrix = global.to_matrix().inverse().to_cols_array();
    // 源的 _ScreenParams = (宽, 高, 1+1/宽, 1+1/高)，物理像素。
    if let Some(size) = camera.physical_viewport_size() {
        let (w, h) = (size.x as f32, size.y as f32);
        env.globals.screen_params = [w, h, 1.0 + 1.0 / w, 1.0 + 1.0 / h];
    }
    // 源的 _Time = (t/20, t, 2t, 3t)。
    let t = time.elapsed_secs();
    env.globals.time = [t / 20.0, t, t * 2.0, t * 3.0];
}

/// 渲染侧的全局量 buffer；一帧一份，全部站点材质共用。
#[derive(Resource)]
pub struct SiteEnvGpuBuffer {
    pub buffer: Buffer,
}

fn create_env_buffer(mut commands: Commands, render_device: Res<RenderDevice>) {
    let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("site_env"),
        contents: &SiteEnv::neutral().gpu_bytes(),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(SiteEnvGpuBuffer { buffer });
}

fn write_env_buffer(
    env: Res<SiteEnv>,
    buffer: Res<SiteEnvGpuBuffer>,
    queue: Res<RenderQueue>,
) {
    let bytes = env.gpu_bytes();
    queue.write_buffer(&buffer.buffer, 0, &bytes);
}

/// 全局量桥：中性表 → 每帧相机态 → 抽取到渲染世界 → 单点写 buffer。
pub struct SiteEnvPlugin;

impl Plugin for SiteEnvPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractResourcePlugin::<SiteEnv>::default())
            .add_systems(Startup, (insert_neutral, apply_sunny_phenomena).chain())
            .add_systems(
                PostUpdate,
                write_frame_state
                    .after(TransformSystems::Propagate)
                    .after(camera::frame_site),
            );
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .add_systems(RenderStartup, create_env_buffer)
                // Prepare 链在 ExtractCommands 之后：抽取系统用 Commands 把主世界的
                // 全局量插进渲染世界，落在 ExtractCommands 才 apply；无序挂载会在
                // 插入落地前的那一帧读到不存在的资源。
                .add_systems(Render, write_env_buffer.in_set(RenderSystems::Prepare));
        }
    }
}
