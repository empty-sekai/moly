//! `Mysekai/Site/Tree` Base 片元程序的逐句翻译。
//!
//! 这一族不是 FieldObject 族，差异正是要点：Tree 的顶点色混合是普通乘
//! （FieldObject 平方顶点色）；现象块在两个宝藏阴影之后把顶点色**再乘
//! 一次**（同一个门）；高度雾乘的是雾梯度自己的 alpha。跑 FieldObject
//! 尾部的 Tree 绘制能通过「变暗了没有」的检查，但律是错的。
//!
//! 落影上色（真源全局量 `_MysekaiDropShadowColor1`）在本族收影变体里
//! 应用两次，与 FieldObject 族同针法：toon 阴影之后一次、顶点色乘法之后
//! 一次，两次共用同一衰减——第二针之前是顶点色乘法而非菲涅耳。
//! 落影的衰减与采样核不在本律内。
//!
//! live 站点材质不走无 keyword 的基形：15 份树材质全部声明
//! `_DISABLE_DITHER` + `_RECEIVE_SHADOWS_OFF` + `_USE_ALPHA_CLIP`，其中
//! 7 份再加 `_USE_TREE_ANIMATION`。对本律的影响有三块——
//!
//! * 抖动 discard 被 `_DISABLE_DITHER` 编译期整块移除：live 片元没有
//!   bayer 路，[`bayer_threshold`]/[`dither_discards`] 保留作基形对照，
//!   live 不达；
//! * live 片元最前面多一条 alpha clip（[`alpha_clip_discards`]），其后
//!   的片元体与基形逐行一致（收影两次、alpha 输出常数 1.0 都同）；
//! * 动画变体的差异全在顶点段（[`wind_sway`]/[`leaf_rotation_uv`]）；
//!   两个 live 变体的片元体彼此逐行相同，差异只在顶点。

use crate::shading::{SECOND_COLOR_TARGET, TREASURE_SHADOW_EDGE, TREASURE_SHADOW_INV_RANGE, TREASURE_SHADOW_SCALE};

/// 该族在真源里的 shader 名。
pub const SHADER_NAME: &str = "Mysekai/Site/Tree";

/// 源记录字面出现的常数。
pub const DITHER_SCALE: f32 = 0.0618750006;
pub const DITHER_BIAS: f32 = 0.00999999978;
/// 顶点程序对变换后法线的 `max(dot(v, v), ...)` 归一化地板；
/// 片元自己的 normalize 没有地板。
pub const NORMALIZE_FLOOR: f32 = 1.17549435e-38;

/// live 变体片元首条 alpha clip 的编译期常量阈值。uniform 槽在源里被
/// 编译剔除；材质数据 `_AlphaClip` 恰是同值 0.5，但不是阈值的来源。
pub const ALPHA_CLIP_THRESHOLD: f32 = 0.5;
/// 树动画顶点段逐字转录的常数：波形二次项 `(w·0.1)·(w·0.1 − 0.1) + 0.1`
/// 的步长，与摆向量 x 分量的分摊（z 分量乘 1.0）。
pub const WIND_SHAPE_STEP: f32 = 0.100000001;
pub const WIND_DISP_X: f32 = 0.660000026;
/// 高度到摆幅的映射：`s = 0.011·(y·strength) + 1`。
pub const WIND_HEIGHT_SCALE: f32 = 0.0110000018;
/// 叶旋转的 π 与 π/360 字面量、uv 旋转中心、遮罩门（严格大于）。
pub const LEAF_ROTATION_PI: f32 = 3.14159274;
pub const LEAF_ROTATION_DEG_TO_RAD: f32 = 0.00872664712;
pub const LEAF_PIVOT: [f32; 2] = [0.75, 0.25];
pub const LEAF_MASK_GATE: f32 = 0.5;

/// 源经 one-hot 点积还原出的 4×4 抖动表，`[行 = 屏幕像素 y][列 = x]`。
/// 与 FieldObject 族同表同向，此处按本族自己的记录保留一份。
pub const BAYER: [[f32; 4]; 4] = [
    [0.0, 12.0, 3.0, 15.0],
    [8.0, 4.0, 11.0, 7.0],
    [2.0, 14.0, 1.0, 13.0],
    [10.0, 6.0, 9.0, 5.0],
];

/// 一帧的全局量，字段与源程序的全局块声明一一对应。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeGlobals {
    pub directional_light_vector: [f32; 3],
    pub phenomena_directional_light_color: [f32; 4],
    pub phenomena_shade_color: [f32; 4],
    pub edge_threshold: f32,
    pub edge_smoothness: f32,
    pub treasure_positions: [[f32; 3]; 2],
    pub treasure_shadow_intensity: [f32; 2],
    pub screen_params: [f32; 4],
    pub fog_params: [f32; 4],
    pub fog_near_color: [f32; 4],
    pub fog_far_color: [f32; 4],
}

/// Tree Base 片元读的材质标量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeMaterial {
    pub dither_alpha: f32,
    pub use_vertex_color_blend: f32,
    pub use_phenomena_lighting: f32,
    pub override_shading_parameter: f32,
    pub local_shading_intensity: f32,
    pub local_edge_threshold: f32,
    pub local_edge_smoothness: f32,
}

/// 逐片元输入：顶点程序产出 varying、采样器返回 `_MainTex` 之后的值
/// （源只读 `.xyz`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TreeFragmentInputs {
    pub main_tex_rgba: [f32; 4],
    pub vertex_color: [f32; 4],
    pub world_normal: [f32; 3],
    pub world_position: [f32; 3],
    pub fog_factor: f32,
    /// 顶点程序的屏幕位置 varying（[`vertex_screen_pos`] 的产物）。
    pub screen_position: [f32; 4],
}

/// 一个屏幕位置上的源抖动阈值：屏幕位置 / w，乘 `_MysekaiScreenParams.xy`，
/// 乘 1/8，取 fract，乘 4 下取整成 4×4 下标，缩放加偏置。
///
/// 屏幕坐标非负，恒走源的带符号 fract 的正支路；负半平面在源里下标
/// 越界（GLSL 未定义），这里取绝对值的 fract，镜像坐标读镜像表元。
/// live 站点变体不达（抖动块被编译期移除），保留作基形对照。
#[must_use]
pub fn bayer_threshold(screen_position: [f32; 4], screen_params: [f32; 4]) -> f32 {
    let qx = screen_position[0] / screen_position[3] * screen_params[0] * 0.125;
    let qy = screen_position[1] / screen_position[3] * screen_params[1] * 0.125;
    let ix = (qx.abs().fract() * 4.0) as usize;
    let iy = (qy.abs().fract() * 4.0) as usize;
    BAYER[iy.min(3)][ix.min(3)] * DITHER_SCALE + DITHER_BIAS
}

/// 源的 discard 判据：`_DitherAlpha` 减阈值低于零丢弃。
/// `_DitherAlpha` 为 1.0 时全表无一超过它，discard 不可能触发。
/// live 站点变体把整块抖动编译期移除（见模块注释），此路只在基形被走到。
#[must_use]
pub fn dither_discards(
    screen_position: [f32; 4],
    screen_params: [f32; 4],
    dither_alpha: f32,
) -> bool {
    dither_alpha - bayer_threshold(screen_position, screen_params) < 0.0
}

/// live 变体片元最前面的丢弃判据：采样 alpha 减常量阈值**严格**低于零
/// 丢弃，恰 `0.5` 存活。发生在整条着色链之前。
#[must_use]
pub fn alpha_clip_discards(sampled_alpha: f32) -> bool {
    sampled_alpha - ALPHA_CLIP_THRESHOLD < 0.0
}

/// 顶点色混合：`_UseVertexColorBlend >= 0.5` 时底色乘 `vs_COLOR0.xyz`
/// ——普通乘，没有平方；顶点色的 alpha 与纹理的 alpha 都不参与。
#[must_use]
pub fn vertex_color_blend(base: [f32; 3], vertex_color: [f32; 3], use_blend: f32) -> [f32; 3] {
    if use_blend < 0.5 {
        base
    } else {
        [
            base[0] * vertex_color[0],
            base[1] * vertex_color[1],
            base[2] * vertex_color[2],
        ]
    }
}

/// 片元对插值法线的 normalize：纯 `inversesqrt(dot)`，无地板
/// （地板只在顶点程序自己的 normalize 上）。
#[must_use]
pub fn normalized_normal(normal: [f32; 3]) -> [f32; 3] {
    let inv = 1.0 / (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    [normal[0] * inv, normal[1] * inv, normal[2] * inv]
}

/// toon 阴影因子：`_OverrideShadingParameter` 门选局部/全局阈值对，
/// half-Lambert 走源自己的分母 `(threshold - smoothness) - (threshold +
/// smoothness)`，乘着色强度。语句序列与 FieldObject 族一致。
#[must_use]
pub fn toon_factor(
    normal_dot_light: f32,
    override_shading_parameter: f32,
    local_intensity: f32,
    local_threshold: f32,
    local_smoothness: f32,
    global_threshold: f32,
    global_smoothness: f32,
) -> f32 {
    let use_local = override_shading_parameter > 0.5;
    let intensity = if use_local { local_intensity } else { 1.0 };
    let threshold = if use_local { local_threshold } else { global_threshold };
    let smoothness = if use_local {
        local_smoothness
    } else {
        global_smoothness
    };
    let half_lambert = normal_dot_light * 0.5 + 0.5;
    let upper = threshold + smoothness;
    let lower = threshold - smoothness;
    let ramp = ((half_lambert - upper) / (lower - upper)).clamp(0.0, 1.0);
    intensity * ramp
}

/// 现象方向光的混合；`color.w` 是混合因子，不是输出 alpha。
#[must_use]
pub fn directional_light_blend(base: [f32; 3], light_color: [f32; 4]) -> [f32; 3] {
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = base[c] + light_color[3] * (base[c] * light_color[c] - base[c]);
    }
    out
}

/// 一项径向宝藏阴影（两个重复块共用同一组常数）。
#[must_use]
pub fn treasure_shadow(
    color: [f32; 3],
    world_xz: [f32; 2],
    treasure_xz: [f32; 2],
    intensity: f32,
) -> [f32; 3] {
    let dx = world_xz[0] - treasure_xz[0];
    let dz = world_xz[1] - treasure_xz[1];
    let distance = (dx * dx + dz * dz).sqrt();
    let t = ((distance - TREASURE_SHADOW_EDGE) * TREASURE_SHADOW_INV_RANGE).clamp(0.0, 1.0);
    let scale = intensity * TREASURE_SHADOW_SCALE;
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = color[c] + scale * (color[c] * t - color[c]);
    }
    out
}

/// 现象块里的第二次顶点色乘，应答与第一次相同的
/// `_UseVertexColorBlend < 0.5` 门。
#[must_use]
pub fn phenomena_vertex_blend(lit: [f32; 3], vertex_color: [f32; 3], use_blend_below_half: bool) -> [f32; 3] {
    if use_blend_below_half {
        lit
    } else {
        [
            lit[0] * vertex_color[0],
            lit[1] * vertex_color[1],
            lit[2] * vertex_color[2],
        ]
    }
}

/// 距离雾插值之后的高度雾，按源程序的次序：雾梯度四通道 clamp 后，
/// 高度项是 `gradient.w * min(exp2(-y * z), 1.0)`——min 夹在 exp2 上、
/// 乘的是插值后的雾色 alpha，两者都不可交换改写。
#[must_use]
pub fn height_fog(
    base: [f32; 3],
    fog_factor: f32,
    world_y: f32,
    fog_params: [f32; 4],
    fog_near: [f32; 4],
    fog_far: [f32; 4],
) -> [f32; 3] {
    let mut gradient = [0.0; 4];
    for c in 0..4 {
        gradient[c] = (fog_far[c] + fog_factor * (fog_near[c] - fog_far[c])).clamp(0.0, 1.0);
    }
    let saturated = (-(world_y) * fog_params[2]).exp2().min(1.0);
    let height = gradient[3] * saturated;
    let mut out = [0.0; 3];
    for c in 0..3 {
        let distance_blended = gradient[c] + fog_factor * (base[c] - gradient[c]);
        out[c] = (base[c] + height * (distance_blended - base[c])).clamp(0.0, 1.0);
    }
    out
}

/// 顶点程序的距离雾因子：`_ProjectionParams.yz`（near, far）造的线性深
/// 项——不除 clip w——再过 `_MysekaiFogParams.xy` 的缩放偏置与 clamp。
#[must_use]
pub fn vertex_fog_factor(clip_z: f32, projection_params: [f32; 4], fog_params: [f32; 4]) -> f32 {
    let mut f = clip_z + projection_params[1];
    f /= projection_params[1] + projection_params[2];
    f *= projection_params[2];
    f = f.max(0.0);
    f = f * fog_params[0] + fog_params[1];
    f.clamp(0.0, 1.0)
}

/// 顶点程序的屏幕位置：先按 `_ProjectionParams.x` 翻 y，xy 各取
/// `0.5 * (w + 分量)`——Unity 的 ComputeScreenPos——zw 原样直传。
#[must_use]
pub fn vertex_screen_pos(clip: [f32; 4], projection_params: [f32; 4]) -> [f32; 4] {
    let y_flipped = clip[1] * projection_params[0];
    [
        0.5 * clip[3] + 0.5 * clip[0],
        0.5 * clip[3] + 0.5 * y_flipped,
        clip[2],
        clip[3],
    ]
}

/// 动画变体顶点段的风摆：时间、材质 `_turbulenceValue`/`_strengthValue`
/// 全显式入参。波形 `sin(time_s · turbulence)` 过二次形
/// `(w·0.1)·(w·0.1 − 0.1) + 0.1`，按 (0.66, 1.0) 分摊到 xz；幅度是
/// `s⁴ − s²`（`s = 0.011·(y·strength) + 1`，越高的顶点摆得越多）；最后
/// 球面规整——新方向乘**原**长度，树冠不越摆越大。之后才走正常的
/// 对象到世界变换。
/// 源的时间输入是引擎平滑过的秒数；本律不模拟平滑，调用方自选时钟。
#[must_use]
pub fn wind_sway(position: [f32; 3], time_s: f32, turbulence: f32, strength: f32) -> [f32; 3] {
    let wave = (time_s * turbulence).sin();
    let shaped = (wave * WIND_SHAPE_STEP) * (wave * WIND_SHAPE_STEP - WIND_SHAPE_STEP)
        + WIND_SHAPE_STEP;
    let s = (position[1] * strength) * WIND_HEIGHT_SCALE + 1.0;
    let s_squared = s * s;
    let amplitude = s_squared * s_squared - s_squared;
    let swayed = [
        position[0] + shaped * WIND_DISP_X * amplitude,
        position[1],
        position[2] + shaped * amplitude,
    ];
    // 源的两次乘分开落：先归一化方向，再乘原向量长度。
    let inv = 1.0
        / (swayed[0] * swayed[0] + swayed[1] * swayed[1] + swayed[2] * swayed[2]).sqrt();
    let direction = [swayed[0] * inv, swayed[1] * inv, swayed[2] * inv];
    let radius = (position[0] * position[0]
        + position[1] * position[1]
        + position[2] * position[2])
        .sqrt();
    [
        direction[0] * radius,
        direction[1] * radius,
        direction[2] * radius,
    ]
}

/// 动画变体顶点段的叶旋转：`sin(time_s · speed · π) · range · (π/360)`
/// 为角，绕 uv 中心 (0.75, 0.25) 转 2D。遮罩采样发生在顶点阶段
/// （`_LeafMaskTex`、lod 0）、读的是**原始** uv，`> 0.5` 才换用旋转后
/// 的 uv——采样值由调用方传入，两臂都落在本纯函数里。
/// live 动画材质 speed/range 全 0：角恒 0、旋转数学本身是恒等，但遮罩
/// 采样仍在。
#[must_use]
pub fn leaf_rotation_uv(
    uv: [f32; 2],
    time_s: f32,
    speed: f32,
    range: f32,
    leaf_mask: f32,
) -> [f32; 2] {
    let angle = ((time_s * speed * LEAF_ROTATION_PI).sin() * range) * LEAF_ROTATION_DEG_TO_RAD;
    let (sin_angle, cos_angle) = (angle.sin(), angle.cos());
    let delta_x = uv[0] - LEAF_PIVOT[0];
    let delta_y = uv[1] - LEAF_PIVOT[1];
    let rotated = [
        cos_angle * delta_x - sin_angle * delta_y + LEAF_PIVOT[0],
        cos_angle * delta_y + sin_angle * delta_x + LEAF_PIVOT[1],
    ];
    if leaf_mask > LEAF_MASK_GATE {
        rotated
    } else {
        uv
    }
}

/// 求 Tree Base 片元的两个声明目标。抖动 discard 无法表达成值，
/// 是否丢弃由调用方另问 [`dither_discards`]；被丢弃的片元两个目标全零。
#[must_use]
pub fn tree_fragment(
    input: &TreeFragmentInputs,
    globals: &TreeGlobals,
    material: &TreeMaterial,
) -> ([f32; 4], [f32; 4]) {
    let base = [input.main_tex_rgba[0], input.main_tex_rgba[1], input.main_tex_rgba[2]];
    let vc = [
        input.vertex_color[0],
        input.vertex_color[1],
        input.vertex_color[2],
    ];
    let blend_off = material.use_vertex_color_blend < 0.5;
    let mut color = vertex_color_blend(base, vc, material.use_vertex_color_blend);
    if material.use_phenomena_lighting > 0.5 {
        let normal = normalized_normal(input.world_normal);
        let lit = directional_light_blend(color, globals.phenomena_directional_light_color);
        let ndl = globals.directional_light_vector[0] * normal[0]
            + globals.directional_light_vector[1] * normal[1]
            + globals.directional_light_vector[2] * normal[2];
        let shade = toon_factor(
            ndl,
            material.override_shading_parameter,
            material.local_shading_intensity,
            material.local_edge_threshold,
            material.local_edge_smoothness,
            globals.edge_threshold,
            globals.edge_smoothness,
        );
        // 源把这一句写全：shade * (shadeColor * lit - lit) + lit。
        let mut shaded = [0.0; 3];
        for c in 0..3 {
            shaded[c] = shade * (globals.phenomena_shade_color[c] * lit[c] - lit[c]) + lit[c];
        }
        let world_xz = [input.world_position[0], input.world_position[2]];
        let after0 = treasure_shadow(
            shaded,
            world_xz,
            [
                globals.treasure_positions[0][0],
                globals.treasure_positions[0][2],
            ],
            globals.treasure_shadow_intensity[0],
        );
        let after1 = treasure_shadow(
            after0,
            world_xz,
            [
                globals.treasure_positions[1][0],
                globals.treasure_positions[1][2],
            ],
            globals.treasure_shadow_intensity[1],
        );
        let blended = phenomena_vertex_blend(after1, vc, blend_off);
        color = height_fog(
            blended,
            input.fog_factor,
            input.world_position[1],
            globals.fog_params,
            globals.fog_near_color,
            globals.fog_far_color,
        );
    }
    // alpha 是写入的常数 1.0；第二目标是写入的常数零（`SECOND_COLOR_TARGET`）。
    ([color[0], color[1], color[2], 1.0], SECOND_COLOR_TARGET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::close;

    #[test]
    fn bayer_threshold_matches_the_record_at_known_pixels() {
        let screen = [0.0, 0.0, 0.0, 1.0];
        let params = [64.0, 64.0, 1.0, 1.0];
        // q = sp.xy/w * 64 * 0.125 = sp.xy * 8。sp = (0,0) → q = 0 →
        // 下标 (0,0) → 表元 0，只剩偏置。
        close(bayer_threshold(screen, params), DITHER_BIAS);
        // sp = 0.15625 → q = 1.25 → fract 0.25 → *4 = 1 → 下标 (1,1)
        // → 表元 4。同时钉住 (1,1) 这个表元。
        let screen = [0.15625, 0.15625, 0.0, 1.0];
        close(bayer_threshold(screen, params), 4.0 * DITHER_SCALE + DITHER_BIAS);
        // sp = 0.3125 → q = 2.5 → fract 0.5 → *4 = 2 → 下标 (2,2) → 表元 1。
        let screen = [0.3125, 0.3125, 0.0, 1.0];
        close(bayer_threshold(screen, params), 1.0 * DITHER_SCALE + DITHER_BIAS);
        // 负半平面在源里未定义；这里钉自己的选择：镜像坐标读镜像表元。
        let screen = [-0.15625, -0.15625, 0.0, 1.0];
        close(bayer_threshold(screen, params), 4.0 * DITHER_SCALE + DITHER_BIAS);
    }

    #[test]
    fn dither_discard_has_both_polarities() {
        let params = [64.0, 64.0, 1.0, 1.0];
        // _DitherAlpha = 1.0 高于全表最大值（15 * scale + bias = 0.938）：
        // 全屏不丢。
        for k in 0..16u32 {
            let screen = [k as f32 / 32.0, k as f32 / 32.0, 0.0, 1.0];
            assert!(!dither_discards(screen, params, 1.0));
        }
        // _DitherAlpha = 0.02 落在表的唯一 0 表元与其余非零表元之间。
        // sp = k/32 → q = k/4 → 下标 (k%4, k%4)，扫过全部四个对角表元，
        // 恰好 0 表元存活。
        for k in 0..16u32 {
            let screen = [k as f32 / 32.0, k as f32 / 32.0, 0.0, 1.0];
            let survives = !dither_discards(screen, params, 0.02);
            assert_eq!(survives, k % 4 == 0, "k={k}");
        }
    }

    #[test]
    fn vertex_blend_is_a_plain_multiply_with_a_live_off_switch() {
        // 中性色 [1,1,1] 必须与「没有混合」可区分。
        let base = [0.5, 0.25, 1.0];
        let tint = [0.70, 0.85, 1.00];
        let got = vertex_color_blend(base, tint, 1.0);
        close(got[0], 0.35);
        close(got[1], 0.2125);
        close(got[2], 1.0);
        // 恰好 0.5 也混合（源的比较是 < 0.5）。
        assert_eq!(vertex_color_blend(base, tint, 0.5), got);
        // 门之下颜色整个忽略，不是乘 1。
        assert_eq!(vertex_color_blend(base, tint, 0.4), base);
    }

    #[test]
    fn phenomena_second_blend_shares_the_first_gate() {
        let lit = [0.8, 0.6, 0.4];
        let tint = [0.5, 1.0, 2.0];
        assert_eq!(
            phenomena_vertex_blend(lit, tint, false),
            [0.4, 0.6, 0.8],
            "blend on: the lit colour is multiplied AGAIN"
        );
        assert_eq!(phenomena_vertex_blend(lit, tint, true), lit);
    }

    #[test]
    fn fog_alpha_comes_from_the_gradient_not_from_fog_near() {
        // 梯度 alpha 被 far.a = 0 钉死、t = 0（整远）时高度项必须是零，
        // 哪怕 fogNear.a = 1——按 near alpha 缩放的尾部会在这里给出可见混合。
        let base = [0.4, 0.5, 0.6];
        let fog_params = [0.0, 0.0, 0.7, 0.0];
        let near = [0.9, 0.8, 0.7, 1.0];
        let far = [0.1, 0.2, 0.3, 0.0];
        let got = height_fog(base, 0.0, 0.0, fog_params, near, far);
        for c in 0..3 {
            close(got[c], base[c]);
        }
        // 梯度 alpha 为 1 时高度项咬合：整远像素就是远雾色。
        let near = [0.9, 0.8, 0.7, 1.0];
        let far = [0.1, 0.2, 0.3, 1.0];
        let got = height_fog(base, 0.0, 0.0, fog_params, near, far);
        for c in 0..3 {
            close(got[c], far[c]);
        }
    }

    #[test]
    fn toon_factor_ramp_hits_both_ends() {
        // 全局支路（override 关）：强度是记录自己的常数 1.0，不是局部值
        // ——局部值只在 `_OverrideShadingParameter > 0.5` 时被读。
        let t = 0.4;
        let s = 0.1;
        let upper = t + s;
        let lower = t - s;
        let got = toon_factor(upper * 2.0 - 1.0, 0.0, 0.8, 0.0, 0.0, t, s);
        close(got, 0.0);
        let got = toon_factor(lower * 2.0 - 1.0, 0.0, 0.8, 0.0, 0.0, t, s);
        close(got, 1.0);
        // 局部支路：同一 ndl 被覆盖——强度 0.8 与局部阈值对接管。
        let got = toon_factor(lower * 2.0 - 1.0, 1.0, 0.8, 0.6, 0.05, t, s);
        close(got, 0.8);
        let got = toon_factor((0.6 + 0.05) * 2.0 - 1.0, 1.0, 0.8, 0.6, 0.05, t, s);
        close(got, 0.0);
    }

    #[test]
    fn vertex_stage_derivations_match_the_record() {
        // 雾因子：clip z = 0.5、near 0.3 / far 20、雾参数 (0.02, 0.1)。
        let f = vertex_fog_factor(0.5, [1.0, 0.3, 20.0, 1.0 / 0.3], [0.02, 0.1, 0.5, 0.0]);
        let want = {
            let mut v: f32 = (0.5 + 0.3) / (0.3 + 20.0);
            v *= 20.0;
            v = v.max(0.0);
            v = v * 0.02 + 0.1;
            v.clamp(0.0, 1.0)
        };
        close(f, want);
        // 屏幕位置：单位 VP（clip = (x, y, z, 1)，proj.x = 1）。
        let sp = vertex_screen_pos([0.5, -0.25, 0.75, 1.0], [1.0, 0.3, 20.0, 1.0]);
        close(sp[0], 0.75);
        close(sp[1], 0.375);
        close(sp[2], 0.75);
        close(sp[3], 1.0);
    }

    #[test]
    fn fragment_end_to_end_both_polarities_of_every_switch() {
        // 全指定输入；合成结果对同一语句序列的独立内联转录逐值比对。
        let input = TreeFragmentInputs {
            main_tex_rgba: [0.72, 0.43, 0.91, 0.80],
            vertex_color: [0.70, 0.85, 1.00, 1.0],
            world_normal: [0.1, 0.6, 0.8],
            world_position: [0.5, -1.0, 2.0],
            fog_factor: 0.6,
            screen_position: [0.5, 0.5, 0.3, 1.0],
        };
        let globals = TreeGlobals {
            directional_light_vector: [0.3, 0.8, 0.2],
            phenomena_directional_light_color: [0.9, 0.7, 0.6, 0.55],
            phenomena_shade_color: [0.5, 0.6, 0.7, 0.0],
            edge_threshold: 0.4,
            edge_smoothness: 0.15,
            treasure_positions: [[2.0, 0.0, 2.0], [-2.0, 0.0, -2.0]],
            treasure_shadow_intensity: [0.6, 0.4],
            screen_params: [64.0, 64.0, 0.5, 0.5],
            fog_params: [0.02, 0.1, 0.5, 0.0],
            fog_near_color: [0.8, 0.6, 0.5, 0.9],
            fog_far_color: [0.1, 0.2, 0.4, 0.8],
        };
        let material = TreeMaterial {
            dither_alpha: 1.0,
            use_vertex_color_blend: 1.0,
            use_phenomena_lighting: 1.0,
            override_shading_parameter: 0.0,
            local_shading_intensity: 0.7,
            local_edge_threshold: 0.3,
            local_edge_smoothness: 0.2,
        };
        let (t0, t1) = tree_fragment(&input, &globals, &material);
        close(t1[0], 0.0);
        close(t0[3], 1.0);
        // L234..L314 的独立转录。
        let base = [
            input.main_tex_rgba[0],
            input.main_tex_rgba[1],
            input.main_tex_rgba[2],
        ];
        let vc = [
            input.vertex_color[0],
            input.vertex_color[1],
            input.vertex_color[2],
        ];
        let mut c = [
            base[0] * vc[0],
            base[1] * vc[1],
            base[2] * vc[2],
        ];
        if material.use_phenomena_lighting > 0.5 {
            let n2 = input.world_normal[0] * input.world_normal[0]
                + input.world_normal[1] * input.world_normal[1]
                + input.world_normal[2] * input.world_normal[2];
            let inv = 1.0 / n2.sqrt();
            let n = [
                input.world_normal[0] * inv,
                input.world_normal[1] * inv,
                input.world_normal[2] * inv,
            ];
            let light = globals.phenomena_directional_light_color;
            let lit = [
                c[0] + light[3] * (c[0] * light[0] - c[0]),
                c[1] + light[3] * (c[1] * light[1] - c[1]),
                c[2] + light[3] * (c[2] * light[2] - c[2]),
            ];
            let ndl = globals.directional_light_vector[0] * n[0]
                + globals.directional_light_vector[1] * n[1]
                + globals.directional_light_vector[2] * n[2];
            let hl = ndl * 0.5 + 0.5;
            let hi = globals.edge_smoothness + globals.edge_threshold;
            let lo = globals.edge_threshold - globals.edge_smoothness;
            let ramp = ((hl - hi) / (lo - hi)).clamp(0.0, 1.0);
            let mut shaded = [0.0; 3];
            for k in 0..3 {
                shaded[k] = ramp * (globals.phenomena_shade_color[k] * lit[k] - lit[k]) + lit[k];
            }
            let mut cur = shaded;
            for (tp, intensity) in globals.treasure_positions.iter().zip(globals.treasure_shadow_intensity) {
                let dx = input.world_position[0] - tp[0];
                let dz = input.world_position[2] - tp[2];
                let d = (dx * dx + dz * dz).sqrt();
                let t = ((d - TREASURE_SHADOW_EDGE) * TREASURE_SHADOW_INV_RANGE).clamp(0.0, 1.0);
                let s = intensity * TREASURE_SHADOW_SCALE;
                for k in 0..3 {
                    cur[k] = s * (cur[k] * t - cur[k]) + cur[k];
                }
            }
            for k in 0..3 {
                cur[k] *= vc[k];
            }
            let mut grad = [0.0; 4];
            for k in 0..4 {
                grad[k] = (globals.fog_far_color[k]
                    + input.fog_factor * (globals.fog_near_color[k] - globals.fog_far_color[k]))
                    .clamp(0.0, 1.0);
            }
            let h = grad[3] * (-(input.world_position[1]) * globals.fog_params[2]).exp2().min(1.0);
            for k in 0..3 {
                let dist = grad[k] + input.fog_factor * (cur[k] - grad[k]);
                c[k] = (cur[k] + h * (dist - cur[k])).clamp(0.0, 1.0);
            }
        }
        for k in 0..3 {
            close(t0[k], c[k]);
        }

        // 两个开关各自关掉都必须挪动像素：合成在两个维度上都是承重的。
        let mut off = material;
        off.use_phenomena_lighting = 0.0;
        let (t0_off, _) = tree_fragment(&input, &globals, &off);
        assert!(
            (t0_off[0] - t0[0]).abs() > 1e-4,
            "phenomena off must move the pixel"
        );
        let mut off = material;
        off.use_vertex_color_blend = 0.0;
        let (t0_novcb, _) = tree_fragment(&input, &globals, &off);
        assert!(
            (t0_novcb[0] - t0[0]).abs() > 1e-4,
            "vertex blend off must move the pixel"
        );
    }

    #[test]
    fn alpha_clip_threshold_is_a_constant_strictly_below() {
        // 阈值是编译期常量：恰 0.5 存活（严格小于），之下丢弃——
        // 材质数据里的同值 0.5 不参与判据。
        assert!(!alpha_clip_discards(0.5));
        assert!(!alpha_clip_discards(1.0));
        assert!(alpha_clip_discards(0.4999));
        assert!(alpha_clip_discards(0.0));
    }

    #[test]
    fn wind_sway_matches_the_independent_transcription() {
        // live 参数 turbulence=2 / strength=13。静止相位（time·turb = 0，
        // 波 0，形状项 0.1）与反相峰（time·turb = 3π/2，波 −1，形状项
        // 0.12）各一组；期望值是按源语句序独立转录的 f32 现算字面量。
        let rest = wind_sway([1.0, 2.0, 3.0], 0.0, 2.0, 13.0);
        close(rest[0], 1.04171383);
        close(rest[1], 1.94465292);
        close(rest[2], 3.02211165);
        let peak = wind_sway([1.0, 2.0, 3.0], 2.3561945, 2.0, 13.0);
        close(peak[0], 1.04973519);
        close(peak[1], 1.93386471);
        close(peak[2], 3.02625608);
        // 时间必须挪动顶点：两个相位的结果承重不同。
        assert!((peak[0] - rest[0]).abs() > 1e-4);
    }

    #[test]
    fn wind_sway_at_zero_height_returns_the_input_exactly() {
        // y = 0 ⇒ s = 1 ⇒ s⁴ − s² = 0 ⇒ 位移为零；球面规整把原长度还给
        // 原向量（本例的归一化与长度往返在 f32 里精确）。
        assert_eq!(wind_sway([0.0, 0.0, 5.0], 1.0, 2.0, 13.0), [0.0, 0.0, 5.0]);
    }

    #[test]
    fn leaf_rotation_gate_has_both_arms() {
        let uv = [1.0, 0.5];
        // 门开（> 0.5）：绕中心 45° 的独立转录期望。
        let open = leaf_rotation_uv(uv, 0.5, 1.0, 45.0, 1.0);
        close(open[0], 0.885299027);
        close(open[1], 0.576640725);
        // 门恰 0.5 不过（严格大于），之下更是：原 uv 原样返回。
        assert_eq!(leaf_rotation_uv(uv, 0.5, 1.0, 45.0, 0.5), [1.0, 0.5]);
        assert_eq!(leaf_rotation_uv(uv, 0.5, 1.0, 45.0, 0.0), [1.0, 0.5]);
        // 同一输入两臂必须分岔：门是承重的。
        assert_ne!(open, [1.0, 0.5]);
    }

    #[test]
    fn leaf_rotation_live_parameters_and_half_turn() {
        // live 动画材质 speed/range 全 0：角恒 0，即使门开旋转也是恒等
        // （选 f32 精确往返的 uv 钉死）。
        assert_eq!(
            leaf_rotation_uv([0.5, 0.25], 3.0, 0.0, 0.0, 1.0),
            [0.5, 0.25]
        );
        // 半圈锚：波峰 1、range = 360 时角恰是 360·(π/360) 字面量，绕中心
        // 的点反射落到 (0.5, 0)；f32 里 sin(角) ≈ −3.3e−7 不是 0，容差内核。
        let half = leaf_rotation_uv([1.0, 0.5], 0.25, 2.0, 360.0, 1.0);
        close(half[0], 0.5);
        close(half[1], 0.0);
    }
}
