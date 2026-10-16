//! `Mysekai/Character` 程序的逐句翻译。配件族的 shader 名不同、程序与
//! 本族逐字节相同，共用这里。
//!
//! 角色域语料全部记录收六个不同程序：全色 Base、Base 加雾、Base 加眉、
//! 加眉加雾、投影 caster、深度——变体之间只差对应的两段，下面按段给律。
//!
//! **face / body 两槽的着色差异（消下颌分界线先读这三条）**：
//! 1. 两槽不是两张 shader，是同一片元里 `_CharacterShaderUsage` 的
//!    switch：face 槽（0）把 mask 钉成 `(1, 0)`，body 槽（1）采样
//!    `_BodyMaskTex.xy`，其余值落默认 `(0, 0)`——见 [`mask_for_slot`]。
//! 2. 分界线机制在 [`select_shading`]：regime 边界不在 body/face 两个
//!    primitive 的交界，而在 `mask.x` 跨 0.5 的地方。body 网格上被遮罩
//!    涂到 `mask.x ≥ 0.5` 的皮肤（颈、颌一带）吃球面支，紧邻
//!    `mask.x < 0.5` 的像素吃 N·L toon 支；同一光源下两支是两个不同的
//!    函数，交界处因子跳变，即用户看到的分界线。shade 色对 `mask.x`
//!    连续（线性插值），不连续的只有因子式——消线要消在因子上。
//! 3. 两支的过渡不同源：球面支没有强度乘数、没有硬阈值分支，
//!    edge/smoothness 是引擎全局量；body 支的三元组全是材质局部量、
//!    smoothness 低于 [`TOON_HARD_SMOOTHNESS`] 还会切成硬阈值。
//!
//! 本程序不读顶点色：全部记录的顶点输入只有 POSITION/TEXCOORD/NORMAL。
//! `_AlphaClip` 在全色记录里声明而未消费；深度 pass 的 alpha 门槛是
//! 字面 [`DEPTH_ALPHA_CLIP`]，与它无关。头参考点在源里是材质块
//! uniform、只吃 `.xz`，值由引擎每帧写头骨世界位（提取产物里没有它，
//! 不进 [`resolve`]）——缺省留在原点时球面距离场中心滑到世界原点，
//! 球面界线整条错位。

use crate::material::{texture_slot, FloatLookup, MaterialSlot};
use crate::shading::SECOND_COLOR_TARGET;

/// 该族在真源里的 shader 名。
pub const SHADER_NAME: &str = "Mysekai/Character";
/// 配件族 shader 名；程序与本族逐字节相同。
pub const SHADER_NAME_ACCESSORY: &str = "Mysekai/Character-Accessory";

/// 源片元里字面出现的两个抖动常数。
pub const DITHER_SCALE: f32 = 0.0618750006;
pub const DITHER_BIAS: f32 = 0.00999999978;
/// 顶点程序对变换后法线的 `max(dot(v, v), ...)` 归一化地板；片元里球面
/// 距离场的 normalize 没有地板。
pub const NORMALIZE_FLOOR: f32 = 1.17549435e-38;
/// body toon 硬阈值/平滑两分支的门：smoothness 低于它走硬阈值。
pub const TOON_HARD_SMOOTHNESS: f32 = 0.00400000019;
/// 深度 pass 的 alpha 门槛，源里是字面 0.5。
pub const DEPTH_ALPHA_CLIP: f32 = 0.5;

/// 源经 one-hot 点积还原出的 4×4 抖动表，`[行 = 屏幕像素 y][列 = x]`。
/// 与站点族同表同向，按本族自己的记录保留一份。
pub const BAYER: [[f32; 4]; 4] = [
    [0.0, 12.0, 3.0, 15.0],
    [8.0, 4.0, 11.0, 7.0],
    [2.0, 14.0, 1.0, 13.0],
    [10.0, 6.0, 9.0, 5.0],
];

/// 一帧的全局量，字段与源片元的全局块声明一一对应。现象值由调用方
/// 供给，这里不猜值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterGlobals {
    pub light_vector: [f32; 3],
    pub light_color: [f32; 4],
    pub skin_shade_color: [f32; 4],
    pub body_shade_color: [f32; 4],
    /// 球面支的边缘与过渡带，引擎全局量、不随材质走。
    pub face_sphere_shadow_edge: f32,
    pub face_sphere_shadow_smoothness: f32,
    pub screen_params: [f32; 4],
    /// 源只吃 `.x`（采样 mip 偏置），`.y` 声明而未消费。
    pub mip_bias: [f32; 2],
    pub fog_params: [f32; 4],
    pub fog_near_color: [f32; 4],
    pub fog_far_color: [f32; 4],
}

/// 加眉变体的三件：眉纹理槽、discard 门槛、输出 alpha 权重。眉标量在
/// 三张槽的材质里都带，眉纹理只有 eye 槽带——变体按纹理槽在不在分，
/// 不按标量在不在分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyebrowParams {
    pub tex: usize,
    pub clip: f32,
    pub alpha: f32,
}

/// 角色槽的材质值。
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterMaterial {
    /// 0 = face 槽、1 = body 槽、其余 = 默认；源里是 int，装载边界转好。
    pub character_shader_usage: i32,
    pub brightness: f32,
    pub override_shading_parameter: f32,
    pub local_body_shading_intensity: f32,
    pub local_body_shading_edge_threshold: f32,
    pub local_body_shading_edge_smoothness: f32,
    pub use_dither: f32,
    pub dither_alpha: f32,
    pub main_tex_st: Option<[f32; 4]>,
    pub main_tex: Option<usize>,
    pub body_mask_tex: Option<usize>,
    pub eyebrow: Option<EyebrowParams>,
}

/// 解析角色槽的标量与纹理值。缺的值按名字报错，不发明 shader 默认值；
/// 眉纹理在场时才要求眉标量成对。
///
/// Accessory 变体的程序不声明 shading 组（31 员普查：93 个
/// `Mysekai/Character` 材质全带八标量，唯一 1 个 `Mysekai/Character-
/// Accessory` 带 4/8——缺的恰是 shading 四元组）。变体按分表只要求它
/// 声明过的四标量；四元组补 0.0：`_CharacterShaderUsage=0` 钉
/// [`mask_for_slot`] 到 `[1, 0]`，[`select_shading`] 恒走球面支，
/// [`body_toon_factor`] 的输出无人读——且即便 usage 取 1，override=0
/// 落的是全局缺省 `(1, 0, 0)` 支，不发明值（与眉纹理缺席退 ref 0 同款：
/// 前提具名，见 SHADER.md 的普查记载）。
pub fn resolve(material: &MaterialSlot) -> Result<CharacterMaterial, String> {
    const REQUIRED: [&str; 8] = [
        "_CharacterShaderUsage",
        "_Brightness",
        "_OverrideShadingParameter",
        "_LocalBodyShadingIntensity",
        "_LocalBodyShadingEdgeThreshold",
        "_LocalBodyShadingEdgeSmoothness",
        "_UseDither",
        "_DitherAlpha",
    ];
    const REQUIRED_ACCESSORY: [&str; 4] = [
        "_CharacterShaderUsage",
        "_Brightness",
        "_UseDither",
        "_DitherAlpha",
    ];
    let accessory = material.shader == SHADER_NAME_ACCESSORY;
    let required: &[&str] = if accessory {
        &REQUIRED_ACCESSORY
    } else {
        &REQUIRED
    };
    let missing: Vec<&str> = required
        .iter()
        .copied()
        .filter(|key| material.get(key).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "material {:?}: missing character scalar properties: {}",
            material.name,
            missing.join(", ")
        ));
    }
    let get = |key: &str| -> Result<f32, String> {
        material
            .get(key)
            .ok_or_else(|| format!("material {:?}: missing float {key}", material.name))
    };
    let usage_raw = get("_CharacterShaderUsage")?;
    let usage = float_to_i32(usage_raw).ok_or_else(|| {
        format!(
            "material {:?}: _CharacterShaderUsage = {usage_raw} is not a whole-number int",
            material.name
        )
    })?;
    let body_mask_tex = texture_slot(material, "_BodyMaskTex");
    if usage == 1 && body_mask_tex.is_none() {
        return Err(format!(
            "material {:?}: body slot requires _BodyMaskTex",
            material.name
        ));
    }
    let eyebrow_tex = texture_slot(material, "_EyebrowTex");
    let eyebrow = match eyebrow_tex {
        Some(tex) => Some(EyebrowParams {
            tex,
            clip: get("_EyebrowClip")?,
            alpha: get("_EyebrowAlpha")?,
        }),
        None => None,
    };
    Ok(CharacterMaterial {
        character_shader_usage: usage,
        brightness: get("_Brightness")?,
        // 变体的程序不声明这四项（见函数注释的普查与死支路论证）；
        // 0.0 在两种 usage 下都不发明渲染值。
        override_shading_parameter: if accessory {
            0.0
        } else {
            get("_OverrideShadingParameter")?
        },
        local_body_shading_intensity: if accessory {
            0.0
        } else {
            get("_LocalBodyShadingIntensity")?
        },
        local_body_shading_edge_threshold: if accessory {
            0.0
        } else {
            get("_LocalBodyShadingEdgeThreshold")?
        },
        local_body_shading_edge_smoothness: if accessory {
            0.0
        } else {
            get("_LocalBodyShadingEdgeSmoothness")?
        },
        use_dither: get("_UseDither")?,
        dither_alpha: get("_DitherAlpha")?,
        main_tex_st: material.texture_scale_offsets.get("_MainTex").copied(),
        main_tex: texture_slot(material, "_MainTex"),
        body_mask_tex,
        eyebrow,
    })
}

fn float_to_i32(value: f32) -> Option<i32> {
    if value.is_finite()
        && value.fract() == 0.0
        && value >= i32::MIN as f32
        && value <= i32::MAX as f32
    {
        Some(value as i32)
    } else {
        None
    }
}

/// 槽 switch 的原形：body 槽吃遮罩两通道，face 槽钉成 `(1, 0)`，
/// 其余值落默认 `(0, 0)`。默认槽跟着 body 支走但 shade 色落在 body 侧。
#[must_use]
pub fn mask_for_slot(usage: i32, body_mask: [f32; 2]) -> [f32; 2] {
    match usage {
        1 => body_mask,
        0 => [1.0, 0.0],
        _ => [0.0, 0.0],
    }
}

/// shade 色：`lerp(skin, body, 1 - mask.x)`，四通道一起插值。
/// 对 `mask.x` 连续；`.a` 在源里还要乘回因子（[`apply_shade`]），
/// alpha 不为 1 时两支的界会比因子本身更宽。
#[must_use]
pub fn shade_color(skin: [f32; 4], body: [f32; 4], mask_x: f32) -> [f32; 4] {
    let t = 1.0 - mask_x;
    let mut out = [0.0; 4];
    for c in 0..4 {
        out[c] = skin[c] + t * (body[c] - skin[c]);
    }
    out
}

/// body toon 因子：无 half-Lambert（站点族是 `dot*0.5+0.5`，本族直取
/// dot），先减 `mask.y` 再夹到 `[0, 1]`。override 门选材质局部三元组或
/// 全局缺省 `(1, 0, 0)`；smoothness 过门走硬阈值
/// `intensity × (threshold ≥ lambert ? 1 : 0)`，否则走平滑支的
/// `intensity × x²(3 − 2x)`，x 自 `(lambert − (thr+sm)) / ((thr−sm) − (thr+sm))`
/// 夹出来。分母保持源的两步相减，不合并成 `-2·smoothness`。
#[must_use]
pub fn body_toon_factor(
    lambert: f32,
    override_shading_parameter: f32,
    local_intensity: f32,
    local_threshold: f32,
    local_smoothness: f32,
) -> f32 {
    let use_local = override_shading_parameter > 0.5;
    let intensity = if use_local { local_intensity } else { 1.0 };
    let threshold = if use_local { local_threshold } else { 0.0 };
    let smoothness = if use_local {
        local_smoothness
    } else {
        0.0
    };
    let upper = threshold + smoothness;
    let hard = intensity * if threshold >= lambert { 1.0 } else { 0.0 };
    if smoothness < TOON_HARD_SMOOTHNESS {
        return hard;
    }
    let lower = threshold - smoothness;
    // 源先取分母倒数再相乘，不写除法。
    let inv_span = 1.0 / (lower - upper);
    let x = ((lambert - upper) * inv_span).clamp(0.0, 1.0);
    intensity * smoothstep_poly(x)
}

/// 球面支因子：脸吃头周 XZ 距离场而不是 N·L。方向 = normalize
/// （无地板，头正上方是源自带的 NaN 缺口）；`sdot` 是它与光源向量
/// `.xz` 的点积——光源 `.xz` 不归一化，sdot 的值域随光源倾斜缩短。
/// 之后与 body 平滑支同一条多项式：`(sdot − edge − smooth) / (−2·smooth)`
/// 夹出 x。没有硬阈值分支、没有强度乘数；edge/smoothness 是全局量。
#[must_use]
pub fn face_sphere_factor(
    world_position: [f32; 3],
    head_position: [f32; 3],
    light_vector: [f32; 3],
    edge: f32,
    smoothness: f32,
) -> f32 {
    let dx = world_position[0] - head_position[0];
    let dz = world_position[2] - head_position[2];
    let inv = (dx * dx + dz * dz).sqrt().recip();
    let sdot = dx * inv * light_vector[0] + dz * inv * light_vector[2];
    // 源先取分母倒数再相乘，不写除法。
    let inv_span = 1.0 / (smoothness * -2.0);
    let x = ((sdot - edge - smoothness) * inv_span).clamp(0.0, 1.0);
    smoothstep_poly(x)
}

/// regime 选择：`mask.x < 0.5 ? body : sphere`。这是分界线的落点——
/// body 槽自己的遮罩就能把像素切进球面 regime（颈颌皮肤的 mask.x 涂满
/// 时），face/body 两个 primitive 的交界反而是同侧同函数。因子在 0.5
/// 处无过渡带，两侧公式不同源。
#[must_use]
pub fn select_shading(mask_x: f32, body_factor: f32, sphere_factor: f32) -> f32 {
    if mask_x < 0.5 {
        body_factor
    } else {
        sphere_factor
    }
}

/// 平滑支共用的 `x²(3 − 2x)`，按源的两步乘加次序。
fn smoothstep_poly(x: f32) -> f32 {
    let t = x * -2.0 + 3.0;
    x * x * t
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

/// 着色应用：源先乘 shade 色的 alpha 再混合——
/// `color = (shade.a × factor) × (lit × shade.rgb − lit) + lit`。
#[must_use]
pub fn apply_shade(lit: [f32; 3], shade: [f32; 4], factor: f32) -> [f32; 3] {
    let k = shade[3] * factor;
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = k * (lit[c] * shade[c] - lit[c]) + lit[c];
    }
    out
}

/// 一个屏幕位置上的源抖动阈值（表元 × 缩放 + 偏置）。屏幕坐标非负，
/// 恒走源的带符号 fract 的正支路；负半平面取绝对值的 fract。
#[must_use]
pub fn bayer_threshold(screen_position: [f32; 4], screen_params: [f32; 4]) -> f32 {
    let qx = screen_position[0] / screen_position[3] * screen_params[0] * 0.25;
    let qy = screen_position[1] / screen_position[3] * screen_params[1] * 0.25;
    let ix = (qx.abs().fract() * 4.0) as usize;
    let iy = (qy.abs().fract() * 4.0) as usize;
    BAYER[iy.min(3)][ix.min(3)] * DITHER_SCALE + DITHER_BIAS
}

/// 源的 discard 判据：先 `_UseDither` 门，再 `_DitherAlpha` 减阈值低于
/// 零丢弃。
#[must_use]
pub fn dither_discards(
    use_dither: f32,
    dither_alpha: f32,
    screen_position: [f32; 4],
    screen_params: [f32; 4],
) -> bool {
    if use_dither <= 0.0 {
        return false;
    }
    dither_alpha - bayer_threshold(screen_position, screen_params) < 0.0
}

/// 眉变体的 discard：眉纹理 r 减门槛低于零丢弃；恰等于门槛不丢。
#[must_use]
pub fn eyebrow_discards(eyebrow_r: f32, clip: f32) -> bool {
    eyebrow_r - clip < 0.0
}

/// 眉变体的输出 alpha：`albedo.a × 眉纹理 r × _EyebrowAlpha`，再统一乘
/// brightness。底色 rgb 照走全色链不动。
#[must_use]
pub fn eyebrow_output_alpha(albedo_alpha: f32, eyebrow_r: f32, eyebrow_alpha: f32) -> f32 {
    albedo_alpha * eyebrow_r * eyebrow_alpha
}

/// 深度 pass 的 alpha 门槛：字面 0.5，低于丢弃。
#[must_use]
pub fn depth_alpha_discards(albedo_alpha: f32) -> bool {
    DEPTH_ALPHA_CLIP - albedo_alpha > 0.0
}

/// caster/深度顶点的两步偏移合成：世界位先加 `_LightDirection ×
/// _ShadowBias.x`，再加法线 × `(1 − clamp(dot(L, n), 0, 1)) × _ShadowBias.y`。
/// 法线是顶点程序自己归一化过的（[`vertex_world_normal`]）。
#[must_use]
pub fn caster_offset(
    light_direction: [f32; 3],
    world_normal: [f32; 3],
    shadow_bias: [f32; 2],
) -> [f32; 3] {
    let ndl = (light_direction[0] * world_normal[0]
        + light_direction[1] * world_normal[1]
        + light_direction[2] * world_normal[2])
        .clamp(0.0, 1.0);
    let slope = (1.0 - ndl) * shadow_bias[1];
    [
        light_direction[0] * shadow_bias[0] + world_normal[0] * slope,
        light_direction[1] * shadow_bias[0] + world_normal[1] * slope,
        light_direction[2] * shadow_bias[0] + world_normal[2] * slope,
    ]
}

/// caster/深度顶点的深度钳：`z = max(−w, z)`，xyw 原样。
#[must_use]
pub fn caster_clip_z(clip_z: f32, clip_w: f32) -> f32 {
    (-clip_w).max(clip_z)
}

/// 顶点程序的法线变换：`dot(N, WorldToObject 各行)` 后带地板归一化。
#[must_use]
pub fn vertex_world_normal(in_normal: [f32; 3], world_to_object: [[f32; 4]; 4]) -> [f32; 3] {
    let mut n = [0.0; 3];
    for r in 0..3 {
        n[r] = in_normal[0] * world_to_object[r][0]
            + in_normal[1] * world_to_object[r][1]
            + in_normal[2] * world_to_object[r][2];
    }
    let floor = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).max(NORMALIZE_FLOOR);
    let inv = floor.sqrt().recip();
    [n[0] * inv, n[1] * inv, n[2] * inv]
}

/// 顶点程序的 UV 变换：`uv × _MainTex_ST.xy + _MainTex_ST.zw`。
#[must_use]
pub fn vertex_uv(uv: [f32; 2], st: [f32; 4]) -> [f32; 2] {
    [
        uv[0] * st[0] + st[2],
        uv[1] * st[1] + st[3],
    ]
}

/// 顶点程序的屏幕位置：先按 `_ProjectionParams.x` 翻 y，xy 各取
/// `0.5 * (w + 分量)`，zw 原样直传。
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

/// 顶点程序的距离雾因子：`_ProjectionParams.yz`（near, far）造的线性深
/// 项——不除 clip w——max 0 在缩放偏置之前，再过 `_MysekaiFogParams.xy`
/// 与 clamp。只有加雾变体有这一段。
#[must_use]
pub fn vertex_fog_factor(clip_z: f32, projection_params: [f32; 4], fog_params: [f32; 4]) -> f32 {
    let mut f = clip_z + projection_params[1];
    f /= projection_params[1] + projection_params[2];
    f *= projection_params[2];
    f = f.max(0.0);
    f = f * fog_params[0] + fog_params[1];
    f.clamp(0.0, 1.0)
}

/// 距离雾插值之后的高度雾，按源程序的次序：雾色四通道 clamp 后，
/// 高度项是 `fog_colour.w * min(exp2(-y * z), 1.0)`——min 夹在 exp2 上、
/// 乘的是插值后的雾色 alpha，两者都不可交换改写。与站点族同式，
/// 按本族记录保留一份。
#[must_use]
pub fn height_fog(
    base: [f32; 3],
    fog_factor: f32,
    world_y: f32,
    fog_params: [f32; 4],
    fog_near: [f32; 4],
    fog_far: [f32; 4],
) -> [f32; 3] {
    let mut fog_color = [0.0; 4];
    for c in 0..4 {
        fog_color[c] = (fog_far[c] + fog_factor * (fog_near[c] - fog_far[c])).clamp(0.0, 1.0);
    }
    let height = fog_color[3] * (-(world_y) * fog_params[2]).exp2().min(1.0);
    let mut out = [0.0; 3];
    for c in 0..3 {
        let distance_blended = fog_color[c] + fog_factor * (base[c] - fog_color[c]);
        out[c] = (base[c] + height * (distance_blended - base[c])).clamp(0.0, 1.0);
    }
    out
}

/// 逐片元输入：顶点 varying 与采样器返回值。`fog_factor` 只有加雾变体
/// 才有；`head_position` 是引擎每帧写的头骨世界位（只吃 `.xz`）；
/// `eyebrow_r` 只有加眉变体被读。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CharacterFragmentInputs {
    pub main_tex_rgba: [f32; 4],
    pub body_mask_rg: [f32; 2],
    pub eyebrow_r: f32,
    pub world_normal: [f32; 3],
    pub world_position: [f32; 3],
    pub head_position: [f32; 3],
    pub fog_factor: Option<f32>,
    pub screen_position: [f32; 4],
}

/// 求全色片元的两个声明目标（Base 形；雾/眉按输入与材质里的开关进入）。
/// 抖动与眉的 discard 无法表达成值，是否丢弃由调用方另问
/// [`dither_discards`] / [`eyebrow_discards`]；被丢弃的片元两个目标全零。
/// 源的段序：albedo → mask switch → shade 色 → body 因子与球面因子 →
/// 选择 → 乘 shade alpha → 光混 → 着色混 → [雾] → alpha 覆写 → brightness。
#[must_use]
pub fn character_fragment(
    input: &CharacterFragmentInputs,
    globals: &CharacterGlobals,
    material: &CharacterMaterial,
) -> ([f32; 4], [f32; 4]) {
    let albedo = input.main_tex_rgba;
    let mask = mask_for_slot(material.character_shader_usage, input.body_mask_rg);
    let shade = shade_color(
        globals.skin_shade_color,
        globals.body_shade_color,
        mask[0],
    );
    let ndl = input.world_normal[0] * globals.light_vector[0]
        + input.world_normal[1] * globals.light_vector[1]
        + input.world_normal[2] * globals.light_vector[2];
    let lambert = (ndl - mask[1]).clamp(0.0, 1.0);
    let body = body_toon_factor(
        lambert,
        material.override_shading_parameter,
        material.local_body_shading_intensity,
        material.local_body_shading_edge_threshold,
        material.local_body_shading_edge_smoothness,
    );
    let sphere = face_sphere_factor(
        input.world_position,
        input.head_position,
        globals.light_vector,
        globals.face_sphere_shadow_edge,
        globals.face_sphere_shadow_smoothness,
    );
    let factor = select_shading(mask[0], body, sphere);
    let lit = directional_light_blend(
        [albedo[0], albedo[1], albedo[2]],
        globals.light_color,
    );
    let mut rgb = apply_shade(lit, shade, factor);
    if let Some(fog_factor) = input.fog_factor {
        rgb = height_fog(
            rgb,
            fog_factor,
            input.world_position[1],
            globals.fog_params,
            globals.fog_near_color,
            globals.fog_far_color,
        );
    }
    let alpha = match material.eyebrow {
        Some(eyebrow) => {
            eyebrow_output_alpha(albedo[3], input.eyebrow_r, eyebrow.alpha)
        }
        None => albedo[3],
    };
    let brightness = material.brightness;
    (
        [rgb[0] * brightness, rgb[1] * brightness, rgb[2] * brightness, alpha * brightness],
        SECOND_COLOR_TARGET,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::close;

    #[test]
    fn mask_for_slot_follows_the_usage_switch() {
        // body 槽透传遮罩两通道；face 槽钉成 (1, 0)；其余值落默认 (0, 0)。
        assert_eq!(mask_for_slot(1, [0.3, 0.9]), [0.3, 0.9]);
        assert_eq!(mask_for_slot(0, [0.3, 0.9]), [1.0, 0.0]);
        assert_eq!(mask_for_slot(7, [0.3, 0.9]), [0.0, 0.0]);
        assert_eq!(mask_for_slot(-1, [0.3, 0.9]), [0.0, 0.0]);
    }

    #[test]
    fn shade_color_interpolates_by_the_inverted_mask_x() {
        let skin = [0.9, 0.8, 0.7, 0.6];
        let body = [0.5, 0.4, 0.3, 0.2];
        // 权重是 1 - mask.x：满 1 落 skin、0 落 body、0.25 走 0.75。
        let at_skin = shade_color(skin, body, 1.0);
        for c in 0..4 {
            close(at_skin[c], skin[c]);
        }
        let at_body = shade_color(skin, body, 0.0);
        for c in 0..4 {
            close(at_body[c], body[c]);
        }
        let mixed = shade_color(skin, body, 0.25);
        close(mixed[0], 0.6);
        close(mixed[1], 0.5);
        close(mixed[2], 0.4);
        close(mixed[3], 0.3);
    }

    #[test]
    fn body_toon_factor_switches_hard_and_smooth_branches() {
        // 硬阈值支：smoothness 0.003 过门（< 0.004），lambert 越界即全变。
        close(body_toon_factor(0.02, 1.0, 1.0, 0.01, 0.003), 0.0);
        close(body_toon_factor(0.005, 1.0, 1.0, 0.01, 0.003), 1.0);
        // 强度乘数照乘。
        close(body_toon_factor(0.2, 1.0, 1.5, 0.5, 0.003), 1.5);
        // 平滑支：lambert = thr 时 x = 0.5，x²(3-2x) = 0.5。
        close(body_toon_factor(0.01, 1.0, 1.0, 0.01, 0.05), 0.5);
        // 上界（thr+sm）之上平滑支是 0。
        close(body_toon_factor(0.06, 1.0, 1.0, 0.01, 0.05), 0.0);
        close(body_toon_factor(0.9, 1.0, 1.0, 0.01, 0.05), 0.0);
        // override 关：走全局缺省 (1, 0, 0)，即硬阈值支、阈 0——恰在
        // 明暗界上（lambert = 0）全额变暗，lit 一点就是 0。
        close(body_toon_factor(0.0, 0.0, 9.0, 9.0, 9.0), 1.0);
        close(body_toon_factor(0.3, 0.0, 9.0, 9.0, 9.0), 0.0);
    }

    #[test]
    fn face_sphere_factor_ramps_over_the_xz_distance_field() {
        let world = [12.0, 0.0, 0.0];
        let head = [10.0, 0.0, 0.0];
        // d 归一化后 = (1, 0)；光源 .xz 不归一化。
        // sdot = 1：恰在 edge+smooth 上，因子 0（迎光）。
        close(face_sphere_factor(world, head, [1.0, 0.5, 0.0], 0.0, 1.0), 0.0);
        // sdot = 0（光在正侧方）：x = 0.5。
        close(face_sphere_factor(world, head, [0.0, 1.0, 0.0], 0.0, 1.0), 0.5);
        // sdot = -1（背光）：x = 1，全额因子。
        close(face_sphere_factor(world, head, [-2.0, 0.0, 0.0], 0.0, 1.0), 1.0);
        // sdot = 2（越过 edge+smooth 朝光）：夹回 0。
        close(face_sphere_factor(world, head, [4.0, 0.0, 0.0], 0.0, 1.0), 0.0);
        // 非 0 edge/smooth 手算锚：x = (0-0.3-0.5)/(-1) = 0.8 → 0.8²*1.4。
        close(face_sphere_factor(world, head, [0.0, 1.0, 0.0], 0.3, 0.5), 0.896);
    }

    #[test]
    fn select_shading_flips_regime_at_the_mask_midpoint() {
        // 0.49 走 body 支；恰 0.5 已是球面支——界上没有过渡带。
        close(select_shading(0.49, 0.25, 0.75), 0.25);
        close(select_shading(0.5, 0.25, 0.75), 0.75);
        close(select_shading(0.51, 0.25, 0.75), 0.75);
    }

    #[test]
    fn apply_shade_multiplies_the_factor_by_the_shade_alpha() {
        let lit = [1.0, 1.0, 1.0];
        let shade = [0.9, 0.8, 0.7, 0.6];
        let out = apply_shade(lit, shade, 0.5);
        // 有效因子 = 0.6 * 0.5 = 0.3。
        close(out[0], 0.97);
        close(out[1], 0.94);
        close(out[2], 0.91);
    }

    #[test]
    fn dither_discards_gate_on_use_dither_then_the_bayer_cell() {
        let params = [4.0, 4.0, 0.0, 0.0];
        let pos = [0.25, 0.25, 0.0, 1.0];
        // 像素 (1,1) 的表元是 4，阈值 0.2575。
        close(bayer_threshold(pos, params), 0.2575);
        assert!(dither_discards(1.0, 0.25, pos, params));
        assert!(!dither_discards(1.0, 0.26, pos, params));
        // 门关不丢。
        assert!(!dither_discards(0.0, 0.0, pos, params));
    }

    #[test]
    fn vertex_screen_pos_flips_y_by_the_projection_sign() {
        let pos = vertex_screen_pos([2.0, 4.0, 3.0, 8.0], [-1.0, 0.0, 0.0, 0.0]);
        close(pos[0], 5.0);
        close(pos[1], 2.0);
        close(pos[2], 3.0);
        close(pos[3], 8.0);
    }

    #[test]
    fn vertex_fog_factor_scales_the_linear_depth_term() {
        // clip_z=3、near=1、far=99：深项 (3+1)/100*99 = 3.96，
        // 过 (0.01, 0) 得 0.0396。
        close(vertex_fog_factor(3.0, [-1.0, 1.0, 99.0, 0.0], [0.01, 0.0, 0.0, 0.0]), 0.0396);
        // max 0 在缩放偏置之前：负深项不借偏置翻正。
        close(vertex_fog_factor(-5.0, [-1.0, 1.0, 99.0, 0.0], [0.01, 0.25, 0.0, 0.0]), 0.25);
    }

    #[test]
    fn height_fog_scales_the_clamped_term_by_the_interpolated_alpha() {
        let base = [0.5, 0.5, 0.5];
        let fog_params = [0.0, 0.0, 0.5, 0.0];
        let near = [0.2, 0.3, 0.4, 0.8];
        let far = [0.0, 0.1, 0.2, 0.4];
        // 雾色 = clamp(lerp(far, near, 0.5)) = [0.1, 0.2, 0.3, 0.6]，
        // alpha 0.6 来自插值。y=0：高度项 0.6。
        let at_zero = height_fog(base, 0.5, 0.0, fog_params, near, far);
        close(at_zero[0], 0.38);
        close(at_zero[1], 0.41);
        close(at_zero[2], 0.44);
        // y=4、z=0.5：exp2(-2)=0.25，高度项 0.15。
        let decayed = height_fog(base, 0.5, 4.0, fog_params, near, far);
        close(decayed[0], 0.47);
        close(decayed[1], 0.4775);
        close(decayed[2], 0.485);
    }

    #[test]
    fn eyebrow_variant_discards_below_the_clip_and_scales_alpha() {
        // 恰在门槛上不丢（减门槛为 0 不小于 0）。
        assert!(eyebrow_discards(0.3, 0.5));
        assert!(!eyebrow_discards(0.5, 0.5));
        // alpha = albedo.a × r × _EyebrowAlpha。
        close(eyebrow_output_alpha(0.8, 0.6, 0.4), 0.192);
    }

    #[test]
    fn depth_pass_clips_at_the_literal_half() {
        assert!(!depth_alpha_discards(0.5));
        assert!(depth_alpha_discards(0.4999));
    }

    #[test]
    fn caster_offset_and_depth_clamp_pin_the_bias_order() {
        let light = [0.0, 0.0, -1.0];
        // 侧向面：dot = 0，斜率项全乘。
        let side = caster_offset(light, [0.0, 1.0, 0.0], [0.5, 2.0]);
        close(side[0], 0.0);
        close(side[1], 2.0);
        close(side[2], -0.5);
        // 迎光面：clamp 后 dot = 1，斜率项归零，只剩深度偏移。
        let facing = caster_offset(light, [0.0, 0.0, -1.0], [0.5, 2.0]);
        close(facing[2], -0.5);
        // 深度钳：z 出 [-w, ∞) 之外被拉回 -w。
        close(caster_clip_z(-9.0, 2.0), -2.0);
        close(caster_clip_z(3.0, 2.0), 3.0);
    }

    #[test]
    fn vertex_world_normal_transforms_with_the_floor() {
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let n = vertex_world_normal([0.0, 2.0, 0.0], identity);
        close(n[0], 0.0);
        close(n[1], 1.0);
        close(n[2], 0.0);
    }

    #[test]
    fn vertex_uv_applies_the_st_transform() {
        let uv = vertex_uv([0.5, 0.5], [2.0, 2.0, 0.1, 0.2]);
        close(uv[0], 1.1);
        close(uv[1], 1.2);
    }

    #[test]
    fn character_fragment_pins_the_three_slots_on_one_input() {
        // 一份输入，三张槽：同一光源下 body 槽走 N·L toon、face 槽走
        // 球面支、默认槽因子为 0——正是分界线两侧公式不同源的锚。
        let input = CharacterFragmentInputs {
            main_tex_rgba: [0.5, 0.4, 0.3, 1.0],
            body_mask_rg: [0.8, 0.2],
            eyebrow_r: 1.0,
            world_normal: [0.0, 1.0, 0.0],
            world_position: [3.0, 5.0, 4.0],
            head_position: [0.0, 0.0, 0.0],
            fog_factor: None,
            screen_position: [0.0, 0.0, 0.0, 1.0],
        };
        let globals = CharacterGlobals {
            light_vector: [0.0, 1.0, 0.0],
            light_color: [0.8, 0.9, 1.0, 0.5],
            skin_shade_color: [0.95, 0.9, 0.85, 1.0],
            body_shade_color: [0.93, 0.85, 0.85, 1.0],
            face_sphere_shadow_edge: 0.0,
            face_sphere_shadow_smoothness: 1.0,
            screen_params: [4.0, 4.0, 0.0, 0.0],
            mip_bias: [0.0, 0.0],
            fog_params: [0.0, 0.0, 0.5, 0.0],
            fog_near_color: [0.0, 0.0, 0.0, 0.0],
            fog_far_color: [0.0, 0.0, 0.0, 0.0],
        };
        let base = |usage: i32| CharacterMaterial {
            character_shader_usage: usage,
            brightness: 0.9,
            override_shading_parameter: 1.0,
            local_body_shading_intensity: 1.0,
            local_body_shading_edge_threshold: 0.01,
            local_body_shading_edge_smoothness: 0.05,
            use_dither: 0.0,
            dither_alpha: 1.0,
            main_tex_st: None,
            main_tex: Some(0),
            body_mask_tex: Some(1),
            eyebrow: None,
        };
        // body 槽：mask=(0.8, 0.2)，lambert=0.8 过上界全亮（body 因子 0），
        // 但 0.8 ≥ 0.5 切进球面 regime：sdot=0 → 因子 0.5。
        // lit = (0.45, 0.38, 0.3)，shade = (0.946, 0.89, 0.85, 1)：
        // 每通道 lit + 0.5·(lit·shade − lit) 再 × 0.9。
        let (rgb, second) = character_fragment(&input, &globals, &base(1));
        close(rgb[0], 0.394065);
        close(rgb[1], 0.32319);
        close(rgb[2], 0.24975);
        close(rgb[3], 0.9);
        assert_eq!(second, SECOND_COLOR_TARGET);
        // face 槽：mask=(1, 0)，shade 落 skin、球面因子同为 0.5——
        // 与 body 槽只差 shade 色，几何相同。
        let (rgb, _) = character_fragment(&input, &globals, &base(0));
        close(rgb[0], 0.394875);
        close(rgb[1], 0.3249);
        close(rgb[2], 0.24975);
        // 默认槽：mask=(0, 0)，0 < 0.5 落 body 支、lambert=1 全亮 →
        // 因子 0，颜色只剩光混。
        let (rgb, _) = character_fragment(&input, &globals, &base(7));
        close(rgb[0], 0.405);
        close(rgb[1], 0.342);
        close(rgb[2], 0.27);
    }

    #[test]
    fn character_fragment_applies_fog_then_the_eyebrow_alpha() {
        // 加雾变体把雾接在着色混之后、brightness 之前；
        // 加眉变体的 alpha 覆写在雾之后（值域互不接触，逐通道核）。
        let input = CharacterFragmentInputs {
            main_tex_rgba: [0.5, 0.4, 0.3, 1.0],
            body_mask_rg: [0.0, 0.0],
            eyebrow_r: 0.6,
            world_normal: [0.0, 1.0, 0.0],
            world_position: [3.0, 0.0, 4.0],
            head_position: [0.0, 0.0, 0.0],
            fog_factor: Some(0.5),
            screen_position: [0.0, 0.0, 0.0, 1.0],
        };
        let globals = CharacterGlobals {
            light_vector: [0.0, 1.0, 0.0],
            light_color: [1.0, 1.0, 1.0, 0.0],
            skin_shade_color: [1.0, 1.0, 1.0, 0.0],
            body_shade_color: [1.0, 1.0, 1.0, 0.0],
            face_sphere_shadow_edge: 0.0,
            face_sphere_shadow_smoothness: 1.0,
            screen_params: [4.0, 4.0, 0.0, 0.0],
            mip_bias: [0.0, 0.0],
            fog_params: [0.0, 0.0, 0.0, 0.0],
            fog_near_color: [0.2, 0.3, 0.4, 1.0],
            fog_far_color: [0.0, 0.0, 0.0, 1.0],
        };
        let material = CharacterMaterial {
            character_shader_usage: 1,
            brightness: 0.5,
            override_shading_parameter: 0.0,
            local_body_shading_intensity: 1.0,
            local_body_shading_edge_threshold: 0.0,
            local_body_shading_edge_smoothness: 0.0,
            use_dither: 0.0,
            dither_alpha: 1.0,
            main_tex_st: None,
            main_tex: Some(0),
            body_mask_tex: Some(1),
            eyebrow: Some(EyebrowParams {
                tex: 2,
                clip: 0.5,
                alpha: 0.4,
            }),
        };
        let (rgb, _) = character_fragment(&input, &globals, &material);
        // 光混恒等；shade.a=0 把着色混变恒等，rgb 停在 albedo；
        // 雾色 alpha 插值后为 1、y=0 → 高度项 1，落点 = 雾色与底色的
        // 中点 (0.3, 0.275, 0.25)，再 × brightness 0.5。
        close(rgb[0], 0.15);
        close(rgb[1], 0.1375);
        close(rgb[2], 0.125);
        // alpha 覆写：1.0 × 0.6 × 0.4 = 0.24，再 × brightness 0.5。
        close(rgb[3], 0.12);
    }
}
