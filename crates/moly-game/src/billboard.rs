//! 公告板批次：粒子呈现层的**几何**半，材质无关。
//!
//! # 谁能调它
//!
//! 任何有「一批世界空间中心点 + 尺寸 + 颜色」要画成四边形的域——站点
//! 粒子、掉落特效、天气 effect 都是同一形状。本模块**不认识任何材质
//! 类型**，也不认识任何着色程序：它只把一批 [`Quad`] 写成一张 bevy
//! `Mesh` 的属性池。材质句柄、着色程序、混合档全归调用方。
//!
//! # 要传什么
//!
//! [`write_quads`] 吃：一张 `&mut Mesh`、一批 [`Quad`]、一个
//! [`Alignment`]、一个 [`CameraBasis`]（对齐与视口钳制要它）、一个
//! [`SizeClamp`]。回一份 [`BatchTally`]，里面是「这批里有多少颗被
//! 视口占比钳制过」一类的现算数——调用方把它印进自己的状态行。
//!
//! # 属性槽语义（着色程序必须照它读）
//!
//! | 槽 | 含义 |
//! |---|---|
//! | `POSITION` | **四边形那个角的世界坐标**（四角展开已在 CPU 完成） |
//! | `UV_0` | 纹理坐标，**Unity 粒子 UV 约定**：v 向上，(0,0) 在四边形左下角 |
//! | `COLOR` | 逐粒子颜色（四个顶点同值） |
//!
//! 索引：每颗 4 顶点、6 索引。双面由管线 CullOff 决定，不能额外写
//! 反向三角形；否则透明粒子的同一片面会被重复混合。
//!
//! # 为什么四角展开在 CPU
//!
//! 真源的顶点程序是一次朴素的 object→world→clip 变换（`ObjectToWorld`
//! 三乘一加、再 `MatrixVP`）——**四边形是引擎在 CPU 侧装配好再喂进去
//! 的**，顶点阶段不做任何展开。照这个分工，对齐档、轴心、旋转、视口
//! 占比钳制全部留在本模块（可复算、可印数），着色程序只剩变换与采样。

use bevy::asset::RenderAssetUsages;
use bevy::math::{Mat3, Vec2, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

/// 一颗要画的粒子。
///
/// `size` 是**全尺寸**（米），不是半宽：`0.5` 表示这颗占半米。
#[derive(Debug, Clone, Copy)]
pub struct Quad {
    /// 世界空间中心（轴心偏移之前）。
    pub centre: Vec3,
    /// X/Y 全尺寸（米）。
    pub size: Vec2,
    /// 绕四边形法线的自转（弧度，逆时针）。
    pub rotation: f32,
    /// 逐粒子颜色（线性域，直接进 `COLOR` 槽）。
    pub colour: [f32; 4],
}

/// `ParticleSystemRenderSpace` 的档位。
///
/// **数值锚引擎源的枚举体本身**（`View = 0` / `World = 1` / `Local = 2` /
/// `Facing = 3` / `Velocity = 4`；枚举体与它的脚本绑定两处互证，游戏侧
/// 的托管声明里这个枚举被剥掉了、锚不到那边）。⚠ **不要从任何一处
/// 「我方期望倒推」的注释里取这个映射**：倒推出来的映射会让门变成
/// 「不等于我实测到的那个值」，于是恒绿。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// 0：四边形平面平行于相机平面（法线 = 相机视线方向）。
    View,
    /// Align to the world XY plane.
    World,
    /// Face the eye position instead of sharing the camera plane.
    Facing,
}

impl Alignment {
    /// 从渲染器记录的 `alignment` 数值判档。**只认已实现的档**：别的
    /// 档回 None，由调用方具名拒绝并计数——静默按 View 画会给出一个
    /// 看起来合理但错向的东西。
    pub fn from_render_space(value: i64) -> Option<Self> {
        match value {
            0 => Some(Alignment::View),
            1 => Some(Alignment::World),
            3 => Some(Alignment::Facing),
            _ => None,
        }
    }

    /// 引擎源里该数值的档位名（拒绝行印它，读者不必回去查枚举）。
    pub fn render_space_name(value: i64) -> &'static str {
        match value {
            0 => "View",
            1 => "World",
            2 => "Local",
            3 => "Facing",
            4 => "Velocity",
            _ => "(不在引擎枚举体里)",
        }
    }
}

/// 相机的当帧姿态：对齐与视口占比钳制的输入。
///
/// 没有相机时**不要造一个替身**：调用方拿不到就跳过本帧的属性池重建
/// （上一帧的池还在，几何不会闪成错的）。
#[derive(Debug, Clone, Copy)]
pub struct CameraBasis {
    /// 世界空间机位。
    pub position: Vec3,
    /// 视线方向（单位向量，指向相机看的方向）。
    pub forward: Vec3,
    /// 相机右方向（单位向量）。
    pub right: Vec3,
    /// 相机上方向（单位向量）。
    pub up: Vec3,
    /// 纵向视场角（弧度）。透视投影才有；正交投影没有
    /// `tan(fov/2)` 语义。
    pub fov_y: f32,
    /// 视口宽高比。
    pub aspect: f32,
}

/// 视口占比钳制：渲染器记录的 min/maxParticleSize。
#[derive(Debug, Clone, Copy)]
pub struct SizeClamp {
    /// 视口占比上限（`maxParticleSize`）。
    pub max_screen_fraction: f32,
    /// 尺寸下限（米）：零缩放的四边形没有面积，画不出来。
    pub min_size: f32,
}

/// 一次属性池重建的现算数。调用方印进自己的状态行——**这几个数是
/// 「几何真的按档位算了」的唯一证据**（我方计数器只证明控制流，所以
/// 它们都是逐顶点写入时数的，不是意图数）。
#[derive(Debug, Clone, Copy, Default)]
pub struct BatchTally {
    /// 写进池的四边形数。
    pub quads: usize,
    /// 写进池的顶点数（= quads × 4）。
    pub vertices: usize,
    /// 被视口占比上限钳制过的颗数。
    pub screen_clamped: usize,
    /// 尺寸被下限抬起过的颗数。
    pub size_floored: usize,
}

/// 一张空的、拓扑为三角列表的网格：属性池逐帧重建，起手是空的。
pub fn empty_mesh() -> Mesh {
    // An empty batch still has the particle vertex contract. A mesh with no
    // attributes makes Bevy specialize Vertex without position/UV/colour, so
    // dormant emitters would compile an invalid shader before their first quad.
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new())
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, Vec::<[f32; 4]>::new())
        .with_inserted_indices(Indices::U32(Vec::new()))
}

/// 把一批四边形写进网格的属性池（整池替换，不增量）。
///
/// `pivot` 是轴心偏移，单位是**粒子尺寸的倍数**（渲染器记录的 `pivot`
/// 就是这个口径）：`(0,0,0)` 表示轴心在四边形中心。
pub fn write_quads(
    mesh: &mut Mesh,
    quads: &[Quad],
    alignment: Alignment,
    camera: CameraBasis,
    clamp: SizeClamp,
    pivot: [f32; 3],
) -> BatchTally {
    let mut tally = BatchTally::default();
    let n = quads.len();
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(n * 4);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(n * 4);
    let mut colours: Vec<[f32; 4]> = Vec::with_capacity(n * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(n * 6);

    for quad in quads {
        // 四边形所在平面的两个基向量。View 档：相机右/上——法线即视线
        // 方向，四边形平行于相机平面。
        let (basis_x, basis_y) = match alignment {
            Alignment::View => (camera.right, camera.up),
            Alignment::World => (Vec3::X, Vec3::Y),
            Alignment::Facing => {
                let normal = (camera.position - quad.centre).normalize_or_zero();
                let right = camera.up.cross(normal).normalize_or_zero();
                let right = if right.length_squared() > 0.0 { right } else { camera.right };
                (right, normal.cross(right).normalize_or_zero())
            },
        };

        let mut size = Vec2::new(quad.size.x.abs(), quad.size.y.abs());
        if size.x < clamp.min_size || size.y < clamp.min_size {
            size = size.max(Vec2::splat(clamp.min_size));
            tally.size_floored += 1;
        }
        // 视口占比钳制：深度 d 处的视口宽 W = 2·d·aspect·tan(fov/2)；
        // 两轴全尺寸的最大者占比超上限时两轴同乘比例（保长宽比）。
        let depth = camera.forward.dot(quad.centre - camera.position);
        if depth > 0.0 && camera.fov_y > 0.0 {
            let viewport_width = 2.0 * depth * camera.aspect * (camera.fov_y * 0.5).tan();
            let biggest = size.x.max(size.y);
            if biggest > 0.0 {
                let k = viewport_width * clamp.max_screen_fraction / biggest;
                if k < 1.0 {
                    size *= k;
                    tally.screen_clamped += 1;
                }
            }
        }

        // 轴心：偏移量是尺寸的倍数，沿四边形自己的两个基轴。
        let centre = quad.centre - basis_x * (pivot[0] * size.x) - basis_y * (pivot[1] * size.y);
        let (sin, cos) = quad.rotation.sin_cos();
        let base = (positions.len()) as u32;
        // 四角：Unity 粒子 UV 约定，(0,0) 在左下、v 向上。角点在四边形
        // 平面内先自转再乘尺寸。
        for corner in [[0.0f32, 0.0f32], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            let local = Vec2::new(corner[0] - 0.5, corner[1] - 0.5) * size;
            let spun = Vec2::new(
                local.x * cos - local.y * sin,
                local.x * sin + local.y * cos,
            );
            let world = centre + basis_x * spun.x + basis_y * spun.y;
            positions.push(world.to_array());
            uvs.push(corner);
            colours.push(quad.colour);
        }
        // One surface; render-state culling determines which side is visible.
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        tally.quads += 1;
        tally.vertices += 4;
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(indices));
    tally
}

/// 从相机的世界变换矩阵取三条基轴。bevy 的相机看向 `-Z`，所以视线
/// 方向是第三列的相反数。
pub fn basis_from_matrix(matrix: Mat3, position: Vec3, fov_y: f32, aspect: f32) -> CameraBasis {
    CameraBasis {
        position,
        forward: -matrix.z_axis.normalize_or_zero(),
        right: matrix.x_axis.normalize_or_zero(),
        up: matrix.y_axis.normalize_or_zero(),
        fov_y,
        aspect,
    }
}
