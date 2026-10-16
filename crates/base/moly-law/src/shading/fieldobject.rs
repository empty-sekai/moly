//! `Mysekai/Site/FieldObject` Base 片元程序的逐句翻译。
//!
//! 式子保持源程序的运算顺序，作为上屏 shader 移植的 oracle；
//! 材质值按名字取，与源程序声明一致，不猜值。

use crate::material::{texture_slot, FloatLookup, MaterialSlot};
use crate::shading::{TREASURE_SHADOW_EDGE, TREASURE_SHADOW_INV_RANGE, TREASURE_SHADOW_SCALE};

/// 该族在真源里的 shader 名。
pub const SHADER_NAME: &str = "Mysekai/Site/FieldObject";

/// 源片元里字面出现的常数。
pub const DITHER_SCALE: f32 = 0.0618750006;
pub const DITHER_BIAS: f32 = 0.00999999978;

/// 一帧的全局量，字段与源程序的全局块声明一一对应。
/// 现象与相机状态由调用方供给，这里不猜任何现象值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldObjectGlobals {
    pub light_vector: [f32; 3],
    pub phenomena_directional_light_color: [f32; 4],
    pub phenomena_shade_color: [f32; 4],
    pub edge_threshold: f32,
    pub edge_smoothness: f32,
    pub treasure_positions: [[f32; 3]; 2],
    pub treasure_shadow_intensity: [f32; 2],
    pub camera_position: [f32; 3],
    /// 源的 `unity_OrthoParams`；`.w == 0` 选透视视线。
    pub ortho_params: [f32; 4],
    /// Unity 列主序视图矩阵，按四列摊平。
    pub view_matrix: [f32; 16],
    pub screen_params: [f32; 4],
    pub fog_params: [f32; 4],
    pub fog_near_color: [f32; 4],
    pub fog_far_color: [f32; 4],
    pub projection_params: [f32; 4],
    pub mip_bias: [f32; 2],
    /// `.y` 驱动 overlay 的 UV 滚动。
    pub time: [f32; 4],
}

/// FieldObject Base 片元读的材质值。向量属性保持 Optional——
/// 特性关着时源不消费它；对应的源变体开着时 [`resolve`] 会要求它。
#[derive(Debug, Clone, PartialEq)]
pub struct FieldObjectMaterialParams {
    pub alpha_clip: f32,
    pub dither_alpha: f32,
    pub fresnel_power: f32,
    pub local_edge_smoothness: f32,
    pub local_edge_threshold: f32,
    pub local_shading_intensity: f32,
    pub override_shading_parameter: f32,
    pub texture_coord_overlay1st: i32,
    pub uv_scroll_overlay1st: [f32; 2],
    pub use_fresnel: f32,
    pub use_overlay_texture_vertex_alpha: f32,
    pub use_vertex_color_blend: f32,
    pub use_vertex_alpha_opacity: f32,
    /// Base 程序的输出 alpha 是被选 alpha 乘它，所以总被要求。
    pub base_opacity: f32,
    pub main_tex: Option<usize>,
    pub overlay_tex: Option<usize>,
    pub fresnel_color: Option<[f32; 4]>,
    pub overlay_st: Option<[f32; 4]>,
    /// `_USE_OVERLAY_TEXTURE` keyword 是否启用第一层 overlay。
    pub overlay_keyword: bool,
}

/// 解析所要求的标量 key 全集；向量属性在对应特性开着时另行要求。
pub const REQUIRED_SCALAR_KEYS: [&str; 15] = [
    "_AlphaClip",
    "_DitherAlpha",
    "_FresnelPower",
    "_LocalEdgeSmoothness",
    "_LocalEdgeThreshold",
    "_LocalShadingIntensity",
    "_OverrideShadingParameter",
    "_TextureCoord_Overlay1st",
    "_UVScrollX_Overlay1st",
    "_UVScrollY_Overlay1st",
    "_UseFresnel",
    "_UseOverlayTextureVertexAlpha",
    "_UseVertexColorBlend",
    "_UseVertexAlphaOpacity",
    "_BaseOpacity",
];

/// 从材质表解析 FieldObject 的标量与向量值。
/// 缺的值按名字报错，不发明 shader 默认值；也不以 shader 名做唯一门，
/// 字节一致的未来变体可以用同一解析器查。
pub fn resolve(material: &MaterialSlot) -> Result<FieldObjectMaterialParams, String> {
    let missing: Vec<&str> = REQUIRED_SCALAR_KEYS
        .into_iter()
        .filter(|key| material.get(key).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "material {:?}: missing FieldObject scalar properties: {}",
            material.name,
            missing.join(", ")
        ));
    }
    let get = |key: &str| -> Result<f32, String> {
        material
            .get(key)
            .ok_or_else(|| format!("material {:?}: missing float {key}", material.name))
    };
    let coord_raw = get("_TextureCoord_Overlay1st")?;
    let coord = float_to_i32(coord_raw).ok_or_else(|| {
        format!(
            "material {:?}: _TextureCoord_Overlay1st = {coord_raw} is not a whole-number int",
            material.name
        )
    })?;
    let use_fresnel = get("_UseFresnel")?;
    let use_vertex_color_blend = get("_UseVertexColorBlend")?;
    let use_overlay_texture_vertex_alpha = get("_UseOverlayTextureVertexAlpha")?;
    let overlay_keyword = material
        .keywords
        .iter()
        .any(|keyword| keyword == "_USE_OVERLAY_TEXTURE");
    let fresnel_color = material.colors.get("_FresnelColor").copied();
    if use_fresnel > 0.5 && fresnel_color.is_none() {
        return Err(format!(
            "material {:?}: _UseFresnel is enabled but _FresnelColor is absent",
            material.name
        ));
    }
    let overlay_st = material
        .texture_scale_offsets
        .get("_OverlayColorMap")
        .copied();
    let overlay_tex = texture_slot(material, "_OverlayColorMap");
    if overlay_keyword && overlay_tex.is_none() {
        return Err(format!(
            "material {:?}: _USE_OVERLAY_TEXTURE is enabled but _OverlayColorMap is absent",
            material.name
        ));
    }
    if overlay_keyword && overlay_st.is_none() {
        return Err(format!(
            "material {:?}: _USE_OVERLAY_TEXTURE is enabled but _OverlayColorMap ST is absent",
            material.name
        ));
    }
    Ok(FieldObjectMaterialParams {
        alpha_clip: get("_AlphaClip")?,
        dither_alpha: get("_DitherAlpha")?,
        fresnel_power: get("_FresnelPower")?,
        local_edge_smoothness: get("_LocalEdgeSmoothness")?,
        local_edge_threshold: get("_LocalEdgeThreshold")?,
        local_shading_intensity: get("_LocalShadingIntensity")?,
        override_shading_parameter: get("_OverrideShadingParameter")?,
        texture_coord_overlay1st: coord,
        uv_scroll_overlay1st: [get("_UVScrollX_Overlay1st")?, get("_UVScrollY_Overlay1st")?],
        use_fresnel,
        use_overlay_texture_vertex_alpha,
        use_vertex_color_blend,
        use_vertex_alpha_opacity: get("_UseVertexAlphaOpacity")?,
        base_opacity: get("_BaseOpacity")?,
        main_tex: texture_slot(material, "_MainTex"),
        overlay_tex,
        fresnel_color,
        overlay_st,
        overlay_keyword,
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

/// 顶点色混合是平方项，平方发生在片元程序里：
/// `base + use * (base * colour² - base)`。
#[must_use]
pub fn vertex_color_blend(base: [f32; 3], vertex_color: [f32; 3], use_blend: f32) -> [f32; 3] {
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = base[c] + use_blend * (base[c] * vertex_color[c] * vertex_color[c] - base[c]);
    }
    out
}

/// 第一层 overlay 混合，带可选的顶点 alpha 门：
/// `gate = (use_vertex_alpha * (COLOR0.a - 1) + 1) * overlay.a`。
#[must_use]
pub fn overlay_blend(
    base: [f32; 3],
    overlay: [f32; 4],
    vertex_alpha: f32,
    use_vertex_alpha: f32,
) -> [f32; 3] {
    let gate = (use_vertex_alpha * (vertex_alpha - 1.0) + 1.0) * overlay[3];
    [
        base[0] + gate * (overlay[0] - base[0]),
        base[1] + gate * (overlay[1] - base[1]),
        base[2] + gate * (overlay[2] - base[2]),
    ]
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

/// toon 阴影因子。分母保持源的 `(threshold-smoothness) - (threshold+smoothness)`，
/// 不加 epsilon——源在这一族没有小 smoothness 分支。
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
    let threshold = if use_local {
        local_threshold
    } else {
        global_threshold
    };
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

/// 一项径向宝藏阴影，对应源里重复出现的两个块。
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

/// Fresnel 附加项与开关，保持源的 `log2`/`exp2` 形。
#[must_use]
pub fn fresnel_color(
    base: [f32; 3],
    normal: [f32; 3],
    view_direction: [f32; 3],
    power: f32,
    color: [f32; 4],
    use_fresnel: f32,
) -> [f32; 3] {
    if use_fresnel <= 0.5 {
        return base;
    }
    let dot = normal[0] * view_direction[0]
        + normal[1] * view_direction[1]
        + normal[2] * view_direction[2];
    let one_minus = (1.0 - dot).clamp(0.0, 1.0);
    let factor = (one_minus.log2() * power).exp2();
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = base[c] + factor * color[c] * color[3];
    }
    out
}

/// 距离雾插值之后的高度雾。高度项是
/// `fog_colour.w * min(exp2(-y * z), 1.0)`：min 夹在 exp2 项上、乘的是
/// 插值后的雾色 alpha——两者都是源程序的原形，不是可交换的等价改写。
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

/// 源经 one-hot 点积还原出的 4×4 表。行是屏幕像素 y，列是 x——转置是有意的。
pub const BAYER: [[f32; 4]; 4] = [
    [0.0, 12.0, 3.0, 15.0],
    [8.0, 4.0, 11.0, 7.0],
    [2.0, 14.0, 1.0, 13.0],
    [10.0, 6.0, 9.0, 5.0],
];

/// 一个屏幕位置上的源 dither 阈值。
#[must_use]
pub fn bayer_threshold(screen_position: [f32; 4], screen_params: [f32; 4]) -> f32 {
    let qx = screen_position[0] / screen_position[3] * screen_params[0] * 0.25;
    let qy = screen_position[1] / screen_position[3] * screen_params[1] * 0.25;
    let ix = (qx.abs().fract() * 4.0) as usize;
    let iy = (qy.abs().fract() * 4.0) as usize;
    BAYER[iy.min(3)][ix.min(3)] * DITHER_SCALE + DITHER_BIAS
}

/// 源 tent 核里字面出现的两个权重常数。
pub const TENT_WEIGHT: f32 = 0.159999996;
pub const TENT_SECONDARY_WEIGHT: f32 = 0.0799999982;

/// 落影上色：`tint = lerp(colour, colour × shadow_colour.rgb, shadow_colour.a)`。
/// `shadow_colour` 是真源全局量 `_MysekaiDropShadowColor1` 形状的 RGBA；
/// 一次应用在源片元里是对它的两行直线算术。
#[must_use]
pub fn drop_shadow_tint(colour: [f32; 3], shadow_colour: [f32; 4]) -> [f32; 3] {
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = shadow_colour[3] * (colour[c] * shadow_colour[c] - colour[c]) + colour[c];
    }
    out
}

/// 落影按衰减插回：`lerp(tint(colour), colour, atten)`。本族在链上应用
/// 两次——toon 阴影之后、菲涅耳之后各一次，两次共用同一衰减、中间只隔
/// 宝藏阴影与菲涅耳，所以两次之间进色的部分也被再上一次色。
#[must_use]
pub fn apply_drop_shadow(colour: [f32; 3], atten: f32, shadow_colour: [f32; 4]) -> [f32; 3] {
    let tinted = drop_shadow_tint(colour, shadow_colour);
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = atten * (colour[c] - tinted[c]) + tinted[c];
    }
    out
}

/// 主光阴影衰减合成。源片元的直线顺序是三步：强度门
/// （`atten = sum·strength + (1−strength)`）→ 比较深度出 `[0, 1)` 整段
/// 免影（覆写前一步）→ 距离 fade 平方往无影插回；次序是源程序原形，
/// 不是可交换的等价改写。`params` 对应真源 `_MainLightShadowParams`：
/// `.x` 强度、`.z` 距离平方系数、`.w` fade 起点。`dist_sq` 是世界位置到
/// 相机位置的平方距离，采样值与比较深度由调用方供给。
#[must_use]
pub fn shadow_attenuate(
    shadow_sum: f32,
    dist_sq: f32,
    params: [f32; 4],
    compare_depth: f32,
) -> f32 {
    let mut atten = shadow_sum * params[0] + (1.0 - params[0]);
    if compare_depth < 0.0 || compare_depth >= 1.0 {
        atten = 1.0;
    }
    let fade = (dist_sq * params[2] + params[3]).clamp(0.0, 1.0);
    let fade = fade * fade;
    fade * (1.0 - atten) + atten
}

/// 单轴六份 tent 权重，按源 tent 核的直线算术逐字转录，次序
/// `[wa, wb, wc, wd, we, wf]`：`wa = W(1−f)`、`wb = W((1−f) − min(f,0)² + 1)`、
/// `wc = W((1+f) − max(f,0)² + 1)`、`wd = W(1+f)`、`we = W((f+0.5)²/2 − f)`、
/// `wf = S(f+0.5)²`，其中 `W`/`S` 即 [`TENT_WEIGHT`] / [`TENT_SECONDARY_WEIGHT`]。
/// `f` 是该轴落在基准 texel 内的带符号小数；六份之和恒为 1。
fn tent_weights(f: f32) -> [f32; 6] {
    let below = f.min(0.0);
    let above = f.max(0.0);
    let inward = 1.0 - f;
    let centre_sq = (f + 0.5) * (f + 0.5);
    [
        inward * TENT_WEIGHT,
        (inward - below * below + 1.0) * TENT_WEIGHT,
        ((f + 1.0) - above * above + 1.0) * TENT_WEIGHT,
        (f + 1.0) * TENT_WEIGHT,
        (centre_sq * 0.5 - f) * TENT_WEIGHT,
        centre_sq * TENT_SECONDARY_WEIGHT,
    ]
}

/// 九个采样点的 shadowmap UV，与源片元九次采样的程序顺序一致：右上、
/// 中上、左上、左中、左下、中心、右中、中下、右下。tent 偏移与权重
/// 同源——每个偏移由一份权重比给出，且两轴的中/尾配对不同（x 轴取
/// `wc`/`wf`，y 轴取 `wf`/`wc`），照抄不改写。比较深度不进 UV，由采样
/// 方与返回值组配。
///
/// `shadowmap_size` 是源全局量 `_MainLightShadowmapSize` 的四通道原形
/// `(1/宽, 1/高, 宽, 高)`：**格子与小数走 `.zw`（图边长，像素）**，
/// tap 偏移与基准 UV 走 `.xy`（texel 尺寸）——两条分量不可混用，格子
/// 乘 texel 会把格子行恒坍缩到 0、九个 tap 恒采图左上角。
#[must_use]
pub fn pcf9_tap_uvs(shadow_uv: [f32; 2], shadowmap_size: [f32; 4]) -> [[f32; 2]; 9] {
    let texel = [shadowmap_size[0], shadowmap_size[1]];
    let edge = [shadowmap_size[2], shadowmap_size[3]];
    let cell_x = (shadow_uv[0] * edge[0] + 0.5).floor();
    let cell_y = (shadow_uv[1] * edge[1] + 0.5).floor();
    let wx = tent_weights(shadow_uv[0] * edge[0] - cell_x);
    let wy = tent_weights(shadow_uv[1] * edge[1] - cell_y);
    // 列和（x 轴：左 wa+we、中 wc+wb、右 wf+wd）与行和
    // （y 轴：上 wa+we、中 wf+wd、下 wc+wb）。
    let col = [wx[0] + wx[4], wx[2] + wx[1], wx[5] + wx[3]];
    let row = [wy[0] + wy[4], wy[5] + wy[3], wy[2] + wy[1]];
    let dx = [
        (wx[0] / col[0] - 2.5) * texel[0],
        (wx[2] / col[1] - 0.5) * texel[0],
        (wx[5] / col[2] + 1.5) * texel[0],
    ];
    let dy = [
        (wy[0] / row[0] - 2.5) * texel[1],
        (wy[5] / row[1] - 0.5) * texel[1],
        (wy[2] / row[2] + 1.5) * texel[1],
    ];
    let base = [cell_x * texel[0], cell_y * texel[1]];
    [
        [base[0] + dx[2], base[1] + dy[0]],
        [base[0] + dx[1], base[1] + dy[0]],
        [base[0] + dx[0], base[1] + dy[0]],
        [base[0] + dx[0], base[1] + dy[1]],
        [base[0] + dx[0], base[1] + dy[2]],
        [base[0] + dx[1], base[1] + dy[1]],
        [base[0] + dx[2], base[1] + dy[1]],
        [base[0] + dx[1], base[1] + dy[2]],
        [base[0] + dx[2], base[1] + dy[2]],
    ]
}

/// 九次硬件深度比较结果的 tent 加权和；`samples` 的下标顺序与
/// [`pcf9_tap_uvs`] 一致。`frac` 是采样坐标在基准 texel 内的带符号小数
/// （`shadow_uv·图边长 − floor(shadow_uv·图边长 + 0.5)`，两轴各一——
/// 边长走 `_MainLightShadowmapSize.zw`，不是 texel 尺寸）。
/// 每份 tap 权重 = 行和×列和，累加次序保持源程序的乘加原形。
#[must_use]
pub fn pcf9(samples: [f32; 9], frac: [f32; 2]) -> f32 {
    let wx = tent_weights(frac[0]);
    let wy = tent_weights(frac[1]);
    let col = [wx[0] + wx[4], wx[2] + wx[1], wx[5] + wx[3]];
    let row = [wy[0] + wy[4], wy[5] + wy[3], wy[2] + wy[1]];
    // 源程序的乘加次序：中上、左上、右上、左中、中心、右中、左下、中下、右下。
    let mut sum = samples[1] * (row[0] * col[1]);
    sum += samples[2] * (row[0] * col[0]);
    sum += samples[0] * (row[0] * col[2]);
    sum += samples[3] * (row[1] * col[0]);
    sum += samples[5] * (row[1] * col[1]);
    sum += samples[6] * (row[1] * col[2]);
    sum += samples[4] * (row[2] * col[0]);
    sum += samples[7] * (row[2] * col[1]);
    sum += samples[8] * (row[2] * col[2]);
    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::close;

    #[test]
    fn vertex_color_blend_squares_the_vertex_colour() {
        // 源在片元程序里把 COLOR0.rgb 平方后再混合：每通道 base * colour²。
        let got = vertex_color_blend([0.2, 0.4, 0.8], [0.5, 1.5, 0.25], 1.0);
        close(got[0], 0.05);
        close(got[1], 0.9);
        close(got[2], 0.05);
        // 混合因子为零时底色原样通过。
        assert_eq!(
            vertex_color_blend([0.2, 0.4, 0.8], [0.5, 1.5, 0.25], 0.0),
            [0.2, 0.4, 0.8]
        );
    }

    #[test]
    fn overlay_blend_gates_on_vertex_alpha() {
        let base = [0.2, 0.4, 0.8];
        let overlay = [1.0, 0.0, 0.5, 0.75];
        // 门开：gate = (0.25 - 1.0 + 1.0) * 0.75 = 0.1875。
        let gated = overlay_blend(base, overlay, 0.25, 1.0);
        close(gated[0], 0.35);
        close(gated[1], 0.325);
        close(gated[2], 0.74375);
        // 门关：只有 overlay 自己的 alpha 起作用。
        let ungated = overlay_blend(base, overlay, 0.25, 0.0);
        close(ungated[0], 0.8);
        close(ungated[1], 0.1);
        close(ungated[2], 0.575);
    }

    #[test]
    fn toon_factor_ramps_between_shadow_bounds() {
        // half-Lambert 0.6 落在界 0.4 与 0.8 正中，局部强度 1.5。
        close(toon_factor(0.2, 1.0, 1.5, 0.6, 0.2, 0.0, 0.0), 0.75);
        // override 关时走全局阈值，局部值被忽略。
        close(toon_factor(0.2, 0.0, 99.0, 99.0, 99.0, 0.6, 0.2), 0.5);
        // 迎光面不受阴影、背光面全额阴影。
        close(toon_factor(1.0, 1.0, 1.5, 0.6, 0.2, 0.0, 0.0), 0.0);
        close(toon_factor(-1.0, 1.0, 1.5, 0.6, 0.2, 0.0, 0.0), 1.5);
    }

    #[test]
    fn treasure_shadow_darkens_at_the_centre_and_releases_beyond_the_falloff() {
        // 宝藏中心处衰减为 0：颜色只剩 1 - intensity * 0.5。
        let at_centre = treasure_shadow([1.0, 1.0, 1.0], [0.0, 0.0], [0.0, 0.0], 1.0);
        close(at_centre[0], 0.5);
        // 超出边缘（0.96）加衰减范围（1/25）之后阴影完全释放。
        let far = treasure_shadow([1.0, 1.0, 1.0], [2.0, 0.0], [0.0, 0.0], 1.0);
        close(far[0], 1.0);
    }

    #[test]
    fn bayer_threshold_samples_the_source_table() {
        let params = [4.0, 4.0, 0.0, 0.0];
        // 像素 (1,1) 的表值是 4。
        close(bayer_threshold([0.25, 0.25, 0.0, 1.0], params), 0.2575);
        // 像素 (0,0) 的表值是 0，只剩偏置。
        close(bayer_threshold([0.0, 0.0, 0.0, 1.0], params), 0.01);
        // (0,1) 的表值是 12；(2,1) 是 14；(3,3) 是 5。
        close(bayer_threshold([0.25, 0.0, 0.0, 1.0], params), 0.7525);
        close(bayer_threshold([0.25, 0.5, 0.0, 1.0], params), 0.87625);
        close(bayer_threshold([0.75, 0.75, 0.0, 1.0], params), 0.319375);
    }

    #[test]
    fn directional_light_blend_uses_the_colour_alpha_as_factor() {
        let got = directional_light_blend([0.2, 0.4, 0.8], [0.8, 0.6, 0.3, 0.65]);
        close(got[0], 0.174);
        close(got[1], 0.296);
        close(got[2], 0.436);
    }

    #[test]
    fn height_fog_scales_the_clamped_term_by_the_interpolated_alpha() {
        let base = [0.5, 0.5, 0.5];
        let fog_params = [0.0, 0.0, 0.5, 0.0];
        let near = [0.2, 0.3, 0.4, 0.8];
        let far = [0.0, 0.1, 0.2, 0.4];
        // fog 色 = clamp(lerp(far, near, 0.5)) = [0.1, 0.2, 0.3, 0.6]，
        // 其中 alpha 0.6 来自插值，不是 near 的 0.8。
        // y = 0：exp2(0) = 1，高度项 = 0.6 * 1 = 0.6。
        let at_zero = height_fog(base, 0.5, 0.0, fog_params, near, far);
        close(at_zero[0], 0.38);
        close(at_zero[1], 0.41);
        close(at_zero[2], 0.44);
        // y = 4、z = 0.5：exp2(-2) = 0.25，高度项 = 0.6 * 0.25 = 0.15。
        let decayed = height_fog(base, 0.5, 4.0, fog_params, near, far);
        close(decayed[0], 0.47);
        close(decayed[1], 0.4775);
        close(decayed[2], 0.485);
        // y = -8：exp2(4) = 16 被 min 夹回 1，高度项不放大。
        let clamped = height_fog(base, 0.5, -8.0, fog_params, near, far);
        close(clamped[0], 0.38);
        close(clamped[1], 0.41);
        close(clamped[2], 0.44);
    }

    #[test]
    fn fresnel_adds_one_minus_dot_raised_to_the_power() {
        // 掠射视线（dot = 0）：附加项是整份颜色乘它自己的 alpha。
        let grazing = fresnel_color(
            [0.2, 0.4, 0.8],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            2.0,
            [0.5, 0.25, 0.75, 0.5],
            1.0,
        );
        close(grazing[0], 0.45);
        close(grazing[1], 0.525);
        close(grazing[2], 1.175);
        // exp2(log2(x) * power) 就是 x^power：(1 - 0.70710677)² 乘单位色。
        let angled = fresnel_color(
            [0.0; 3],
            [0.0, 0.0, 1.0],
            [0.5, 0.5, 0.70710677],
            2.0,
            [1.0, 1.0, 1.0, 1.0],
            1.0,
        );
        close(angled[0], 0.0857864);
        close(angled[1], 0.0857864);
        close(angled[2], 0.0857864);
        // 开关低于 0.5 时底色原样通过。
        let off = fresnel_color(
            [0.2, 0.4, 0.8],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
            2.0,
            [0.5, 0.25, 0.75, 0.5],
            0.4,
        );
        assert_eq!(off, [0.2, 0.4, 0.8]);
    }

    #[test]
    fn drop_shadow_tint_blends_colour_toward_its_tinted_product() {
        let colour = [0.5, 1.0, 0.25];
        let shadow_colour = [0.8, 0.4, 0.2, 0.5];
        // 半透明落影：每通道 lerp(colour, colour·rgb, 0.5)。
        let tinted = drop_shadow_tint(colour, shadow_colour);
        close(tinted[0], 0.45);
        close(tinted[1], 0.7);
        close(tinted[2], 0.15);
        // alpha 为 0 是恒等，为 1 是整份乘 rgb。
        assert_eq!(drop_shadow_tint(colour, [0.8, 0.4, 0.2, 0.0]), colour);
        let full = drop_shadow_tint(colour, [0.8, 0.4, 0.2, 1.0]);
        close(full[0], 0.4);
        close(full[1], 0.4);
        close(full[2], 0.05);
    }

    #[test]
    fn apply_drop_shadow_interpolates_back_by_attenuation() {
        let colour = [0.5, 1.0, 0.25];
        let shadow_colour = [0.8, 0.4, 0.2, 0.5];
        // 衰减 0：整份上色；衰减 1：原色原样通过。
        let shadowed = apply_drop_shadow(colour, 0.0, shadow_colour);
        close(shadowed[0], 0.45);
        close(shadowed[1], 0.7);
        close(shadowed[2], 0.15);
        assert_eq!(apply_drop_shadow(colour, 1.0, shadow_colour), colour);
        // 半影：lerp(tint, colour, 0.25)。
        let partial = apply_drop_shadow(colour, 0.25, shadow_colour);
        close(partial[0], 0.4625);
        close(partial[1], 0.775);
        close(partial[2], 0.175);
    }

    #[test]
    fn shadow_attenuate_orders_strength_gate_then_distance_fade() {
        // 强度门：atten = sum·0.8 + 0.2；fade 为 0 不再改写。
        let params = [0.8, 0.0, 0.0, 0.0];
        close(shadow_attenuate(0.25, 100.0, params, 0.5), 0.4);
        // 比较深度出 [0, 1) 整段免影——含正好 1.0，不含 0.0。
        close(shadow_attenuate(0.0, 0.0, [1.0, 0.0, 0.0, 0.0], -0.1), 1.0);
        close(shadow_attenuate(0.0, 0.0, [1.0, 0.0, 0.0, 0.0], 1.0), 1.0);
        close(shadow_attenuate(0.25, 100.0, params, 0.0), 0.4);
        // 距离 fade 是平方项：d²·0.001 = 0.5 → fade² = 0.25。
        close(shadow_attenuate(0.0, 500.0, [1.0, 0.0, 0.001, 0.0], 0.5), 0.25);
        close(shadow_attenuate(1.0, 500.0, [1.0, 0.0, 0.001, 0.0], 0.5), 1.0);
        // 全序：0.2·0.5+0.5 = 0.6，fade² 0.25 拉回 0.25·0.4+0.6 = 0.7。
        close(shadow_attenuate(0.2, 250.0, [0.5, 0.0, 0.001, 0.25], 0.5), 0.7);
    }

    #[test]
    fn pcf9_weights_match_the_source_tent_at_a_signed_frac() {
        // frac = [0.3, −0.2]：x 轴列和 (0.1152, 0.6256, 0.2592)，
        // y 轴行和 (0.2312, 0.1352, 0.6336)，tap 权重 = 行×列，
        // 手算锚逐点核对。
        let frac = [0.3, -0.2];
        let mut taps = [0.0; 9];
        for (i, tap) in taps.iter_mut().enumerate() {
            let mut markers = [0.0; 9];
            markers[i] = 1.0;
            *tap = pcf9(markers, frac);
        }
        close(taps[2], 0.02663424); // 左上 = 行0·列0
        close(taps[1], 0.14463872); // 中上 = 行0·列1
        close(taps[0], 0.05992704); // 右上 = 行0·列2
        close(taps[5], 0.08458112); // 中心 = 行1·列1
        close(taps[4], 0.07299072); // 左下 = 行2·列0
        close(taps[8], 0.16422912); // 右下 = 行2·列2
        // 全亮全暗不增减能量。
        close(pcf9([1.0; 9], frac), 1.0);
        close(pcf9([0.0; 9], frac), 0.0);
    }

    #[test]
    fn pcf9_tap_uvs_reproduce_the_source_offsets() {
        // 4×4 图（size = (1/4, 1/4, 4, 4)）。整格中心（f = 0）：uv (0.5,0.5)
        // 落在像素 (2,2) 的中心——base = 2·texel = 0.5，偏移是 tent 核的
        // 既钉锚值（对称 ±0.40277778、中列 dx=0、y 轴中/下配对 -0.09722222
        // 与 0.5——kernel 值与本修无关，base 从坍缩的 0 回到正确格）。
        let zero = pcf9_tap_uvs([0.5, 0.5], [0.25, 0.25, 4.0, 4.0]);
        close(zero[2][0], 0.09722222);
        close(zero[2][1], 0.09722222);
        close(zero[0][0], 0.90277778);
        close(zero[5][0], 0.5);
        close(zero[5][1], 0.40277778);
        close(zero[4][1], 1.0);
        close(zero[8][0], 0.90277778);
        close(zero[8][1], 1.0);
        // 同一 f = 0.125 的手算锚（kernel 值沿用既钉），但 uv 落在
        // 像素 2.125：uv = 2.125/4 = 0.53125——格子与小数由图边长算出。
        let half = pcf9_tap_uvs([0.53125, 0.53125], [0.25, 0.25, 4.0, 4.0]);
        close(half[2][0], 0.10640496);
        close(half[2][1], 0.10640496);
        close(half[5][0], 0.50735294);
        close(half[5][1], 0.41198225);
        close(half[8][0], 0.91198225);
        close(half[8][1], 1.00735294);
    }

    #[test]
    fn pcf9_tap_uvs_cluster_around_the_visited_texel() {
        // 16×16 图，uv 落在像素 (10.4, 5.6)——格子 (10, 6)、base
        // (0.625, 0.375)。九个 tap 必须聚在该格附近（tent 跨约左 3 右 2
        // texel）；格子若乘错分量（texel 而非边长）base 坍缩到 0，本断言
        // 响亮红——旧测试只喂首 texel 内的位置，抓不到那个错。
        let taps = pcf9_tap_uvs([0.65, 0.35], [0.0625, 0.0625, 16.0, 16.0]);
        let texel = 0.0625;
        for tap in taps {
            assert!(
                tap[0] > 0.625 - 3.5 * texel && tap[0] < 0.625 + 2.5 * texel,
                "tap x {tap:?} 未聚在格子 10 附近"
            );
            assert!(
                tap[1] > 0.375 - 3.5 * texel && tap[1] < 0.375 + 2.5 * texel,
                "tap y {tap:?} 未聚在格子 6 附近"
            );
        }
        // 中心 tap 贴着格子中心（f_x = 0.4、f_y = -0.4 的小偏移内）。
        let centre = taps[5];
        assert!((centre[0] - 0.625).abs() < 2.0 * texel);
        assert!((centre[1] - 0.375).abs() < 2.0 * texel);
    }

    #[test]
    fn chain_pins_drop_shadow_twice_around_treasure_and_fresnel() {
        // 链序断言：底色 → 方向光 → toon 阴影 →【针一】→ 宝藏影×2 →
        // 菲涅耳 →【针二】→ 高度雾；两针共用同一衰减。
        let base = vertex_color_blend([0.8, 0.6, 0.4], [0.5, 1.0, 0.25], 1.0);
        let directed = directional_light_blend(base, [0.8, 0.6, 0.3, 0.65]);
        // toon 混合：因子 0.75，shade 色对半压暗。
        let factor = toon_factor(0.2, 1.0, 1.5, 0.6, 0.2, 0.0, 0.0);
        let shaded = [
            directed[0] + factor * (directed[0] * 0.5 - directed[0]),
            directed[1] + factor * (directed[1] * 0.5 - directed[1]),
            directed[2] + factor * (directed[2] * 0.5 - directed[2]),
        ];
        let atten = shadow_attenuate(0.25, 0.0, [0.8, 0.0, 0.0, 0.0], 0.5);
        let shadow_colour = [0.8, 0.4, 0.2, 0.5];
        let run = |pin1: bool, pin2: bool, atten2: f32| -> [f32; 3] {
            let mut c = shaded;
            if pin1 {
                c = apply_drop_shadow(c, atten, shadow_colour);
            }
            c = treasure_shadow(
                treasure_shadow(c, [1.0, 1.0], [1.0, 1.0], 1.0),
                [1.0, 1.0],
                [1.0, 1.0],
                1.0,
            );
            c = fresnel_color(
                c,
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0],
                2.0,
                [0.5, 0.25, 0.75, 0.5],
                1.0,
            );
            if pin2 {
                c = apply_drop_shadow(c, atten2, shadow_colour);
            }
            height_fog(
                c,
                0.25,
                0.0,
                [0.0, 0.0, 0.5, 0.0],
                [0.8, 0.8, 0.8, 0.5],
                [0.0, 0.0, 0.0, 0.5],
            )
        };
        // 全链手算终值：本落影色下每针把色乘 (0.94, 0.82, 0.76)；采样点
        // 取在两个宝藏位上（衰减 0 全额），宝藏影各减半；菲涅耳加
        // (0.25, 0.125, 0.375)；雾色 alpha 插值后仍 0.5，雾 = 0.625c + 0.075。
        let full = run(true, true, atten);
        close(full[0], 0.2368893);
        close(full[1], 0.16821734);
        close(full[2], 0.25389354);
        // 抽掉任一针、或两针衰减不同值，都不再是这条链的输出。
        assert_ne!(run(false, true, atten), full);
        assert_ne!(run(true, false, atten), full);
        assert_ne!(run(true, true, 0.5), full);
        // 衰减 1 的针是恒等——与抽掉该针等价。
        assert_eq!(run(true, false, atten), run(true, true, 1.0));
    }
}
