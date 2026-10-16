//! `Mysekai/Water` Base 片元程序（含 keyword 变体）的逐句翻译。
//!
//! 式子保持源程序的运算顺序，作为上屏 shader 移植的 oracle；
//! 材质值按名字取，与源程序声明一致，不猜值。
//!
//! 已知缺口：材质表带 `_WaterFoamColor` 颜色与 `_WaterDistortionTex` /
//! `_FoamTex` / `_WaterFoamMaskTex` 纹理槽，但已导出的 Water 记录
//! （含本程序所译的无 keyword Base 与 fresnel / overlay / alpha-clip /
//! skip-light 变体）没有任何一处消费 foam——泡沫辅助贴图在源导出里缺件，
//! 本律不含 foam 项，也不发明它的数据。
//!
//! 落影上色（真源全局量 `_MysekaiDropShadowColor1`）在本族收影变体里
//! 只应用一次（带该全局量的收影变体全体一致；FieldObject 族是两次）。
//! 落影的衰减与采样核不在本律内。

use crate::shading::{TREASURE_SHADOW_EDGE, TREASURE_SHADOW_INV_RANGE, TREASURE_SHADOW_SCALE};

/// 该族在真源里的 shader 名。
pub const SHADER_NAME: &str = "Mysekai/Water";

/// 一帧的全局量，字段与源程序的全局块声明一一对应。
/// 现象与相机状态由调用方供给，这里不猜任何现象值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterGlobals {
    /// 源在纹理调用上带 `_GlobalMipBias.x`；采样由宿主承担，值随块保留。
    pub mip_bias: f32,
    pub directional_light_vector: [f32; 3],
    pub phenomena_directional_light_color: [f32; 4],
    pub phenomena_shade_color: [f32; 4],
    pub edge_threshold: f32,
    pub edge_smoothness: f32,
    pub treasure_positions: [[f32; 3]; 2],
    pub treasure_shadow_intensity: [f32; 2],
    pub fog_params: [f32; 4],
    pub fog_near_color: [f32; 4],
    pub fog_far_color: [f32; 4],
}

/// Water Base 族读的材质标量。`_UseFresnel` 槽在 fresnel 记录里标为
/// unused——keyword 直接选片元体，不按这个标量分支——所以这里没有它。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterMaterial {
    pub base_opacity: f32,
    pub use_vertex_color_blend: f32,
    pub use_vertex_alpha_opacity: f32,
    pub override_shading_parameter: f32,
    pub local_shading_intensity: f32,
    pub local_edge_threshold: f32,
    pub local_edge_smoothness: f32,
    /// 只被 fresnel keyword 变体消费。
    pub fresnel_color: [f32; 4],
    pub fresnel_power: f32,
}

/// 逐片元输入：顶点程序产出 varying、采样器返回 `_MainTex` 之后的值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterFragmentInputs {
    pub world_position: [f32; 3],
    pub world_normal: [f32; 3],
    /// 源的 `vs_TEXCOORD4`：原样顶点色（这一族不平方，平方发生在片元）。
    pub vertex_color: [f32; 4],
    pub main_tex_rgba: [f32; 4],
    pub fog_factor: f32,
    /// 源的 `vs_TEXCOORD3`，只被 fresnel keyword 变体读。
    pub view_direction: [f32; 3],
}

/// 改变 Base 片元体的可选 keyword 特性。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaterFragmentFeatures {
    /// `_SKIP_PHENOMENA_LIGHT`：该变体直接输出纹理/顶点色，
    /// 不消费光照、宝藏与雾的全局量。
    pub skip_phenomena_lighting: bool,
    /// `_ENABLE_MODULE_FRESNEL`：在两个宝藏阴影之后附加 log2/exp2 项。
    pub enable_fresnel: bool,
}

impl Default for WaterFragmentFeatures {
    fn default() -> Self {
        Self {
            skip_phenomena_lighting: false,
            enable_fresnel: false,
        }
    }
}

/// 源的 `tex.wxyz * vs_COLOR0.wxyz`：逐分量乘。
/// `(tex.a*vc.a, tex.r*vc.r, tex.g*vc.g, tex.b*vc.b)`——rgb 各带顶点色的
/// 同通道分量，不是统一的顶点 alpha。
#[must_use]
pub fn vertex_alpha_products(tex: [f32; 4], vertex_color: [f32; 4]) -> [f32; 4] {
    [
        tex[3] * vertex_color[3],
        tex[0] * vertex_color[0],
        tex[1] * vertex_color[1],
        tex[2] * vertex_color[2],
    ]
}

/// 无 overlay 的底色：blend 开时 `products.yzw * vc.xyz`，即
/// `tex.rgb * vc.rgb²`——平方由这次再乘发生，与顶点 alpha 无关。
#[must_use]
pub fn unlit_base(tex: [f32; 4], vertex_color: [f32; 4], use_vertex_color_blend: f32) -> [f32; 3] {
    let products = vertex_alpha_products(tex, vertex_color);
    if use_vertex_color_blend > 0.5 {
        [
            products[1] * vertex_color[0],
            products[2] * vertex_color[1],
            products[3] * vertex_color[2],
        ]
    } else {
        [tex[0], tex[1], tex[2]]
    }
}

/// 被选 alpha：`_UseVertexAlphaOpacity` 开时取 products 的 `.x`
/// （= `tex.a * vc.a`），否则取纹理 alpha。
#[must_use]
pub fn source_alpha(tex: [f32; 4], vertex_color: [f32; 4], use_vertex_alpha_opacity: f32) -> f32 {
    if use_vertex_alpha_opacity > 0.5 {
        tex[3] * vertex_color[3]
    } else {
        tex[3]
    }
}

/// 第一/第二 overlay 的源合成：门是
/// `(use_vertex_alpha * (vc.a - 1) + 1) * overlay.a`，blend 开时再乘
/// `vc.rgb²`。返回的 RGB 直接进公共光照链；alpha 仍走 [`source_alpha`]。
#[must_use]
pub fn overlay_base_rgb(
    main_tex: [f32; 4],
    overlay_tex: [f32; 4],
    vertex_color: [f32; 4],
    overlay_vertex_alpha: f32,
    use_vertex_color_blend: f32,
) -> [f32; 3] {
    let mut gate = overlay_vertex_alpha;
    gate *= vertex_color[3] - 1.0;
    gate += 1.0;
    gate *= overlay_tex[3];

    let mut mixed = [0.0f32; 3];
    for c in 0..3 {
        let delta = overlay_tex[c] - main_tex[c];
        mixed[c] = gate * delta + main_tex[c];
    }
    if use_vertex_color_blend > 0.5 {
        [
            mixed[0] * vertex_color[0] * vertex_color[0],
            mixed[1] * vertex_color[1] * vertex_color[1],
            mixed[2] * vertex_color[2] * vertex_color[2],
        ]
    } else {
        mixed
    }
}

fn treasure_shadow_factor(world_position: [f32; 3], treasure_position: [f32; 3]) -> f32 {
    let dx = world_position[0] - treasure_position[0];
    let dz = world_position[2] - treasure_position[2];
    let mut distance_squared = dx * dx;
    distance_squared += dz * dz;
    let distance = distance_squared.sqrt();
    let mut edge = distance + -TREASURE_SHADOW_EDGE;
    edge *= TREASURE_SHADOW_INV_RANGE;
    edge.clamp(0.0, 1.0)
}

fn apply_treasure_shadow(mut color: [f32; 3], t: f32, intensity: f32) -> [f32; 3] {
    let scale = intensity * TREASURE_SHADOW_SCALE;
    for channel in &mut color {
        let delta = *channel * t + -*channel;
        *channel = scale * delta + *channel;
    }
    color
}

fn treasure_shadow_source(
    color: [f32; 3],
    world_position: [f32; 3],
    treasure_position: [f32; 3],
    intensity: f32,
) -> [f32; 3] {
    let t = treasure_shadow_factor(world_position, treasure_position);
    apply_treasure_shadow(color, t, intensity)
}

fn fog_source(color: [f32; 3], world_y: f32, fog_factor: f32, globals: WaterGlobals) -> [f32; 3] {
    let mut fog_color = [0.0f32; 4];
    for (c, value) in fog_color.iter_mut().enumerate() {
        let delta = globals.fog_near_color[c] + -globals.fog_far_color[c];
        *value = fog_factor * delta + globals.fog_far_color[c];
        *value = value.clamp(0.0, 1.0);
    }

    let mut distance_blended = [0.0f32; 3];
    for c in 0..3 {
        let delta = color[c] - fog_color[c];
        distance_blended[c] = fog_factor * delta + fog_color[c];
    }

    // 源的 exp2：min 夹在它上面、乘的是插值后的雾色 alpha。
    let mut height = -world_y * globals.fog_params[2];
    height = height.exp2();
    height = height.min(1.0);
    height *= fog_color[3];
    let mut out = color;
    for c in 0..3 {
        let delta = distance_blended[c] - out[c];
        out[c] = height * delta + out[c];
        out[c] = out[c].clamp(0.0, 1.0);
    }
    out
}

fn lit_color_source(
    input: WaterFragmentInputs,
    globals: WaterGlobals,
    material: WaterMaterial,
    base: [f32; 3],
    features: WaterFragmentFeatures,
) -> [f32; 3] {
    // fresnel 变体在光照链之前先算宝藏 0 的距离因子、在 toon 之后才用它。
    // 保持这个前置：挪动它就不再是编译后源程序的语句顺序。
    let fresnel_treasure0 = if features.enable_fresnel {
        Some(treasure_shadow_factor(
            input.world_position,
            globals.treasure_positions[0],
        ))
    } else {
        None
    };
    let normal = normalize_source(input.world_normal);
    let mut half_lambert = dot3(globals.directional_light_vector, normal);
    half_lambert = half_lambert * 0.5 + 0.5;

    let use_local = material.override_shading_parameter > 0.5;
    let threshold = if use_local {
        material.local_edge_threshold
    } else {
        globals.edge_threshold
    };
    let smoothness = if use_local {
        material.local_edge_smoothness
    } else {
        globals.edge_smoothness
    };
    let intensity = if use_local {
        material.local_shading_intensity
    } else {
        1.0
    };
    let upper = smoothness + threshold;
    let lower = -smoothness + threshold;
    let denominator = -upper + lower;
    let mut ramp = half_lambert + -upper;
    ramp /= denominator;
    ramp = ramp.clamp(0.0, 1.0);
    let ramp = intensity * ramp;

    let mut color = base;
    for c in 0..3 {
        let light_delta = color[c] * globals.phenomena_directional_light_color[c] + -color[c];
        color[c] = globals.phenomena_directional_light_color[3] * light_delta + color[c];
    }
    for c in 0..3 {
        let shade_delta = color[c] * globals.phenomena_shade_color[c] + -color[c];
        color[c] = ramp * shade_delta + color[c];
    }
    if let Some(t) = fresnel_treasure0 {
        color = apply_treasure_shadow(color, t, globals.treasure_shadow_intensity[0]);
    } else {
        color = treasure_shadow_source(
            color,
            input.world_position,
            globals.treasure_positions[0],
            globals.treasure_shadow_intensity[0],
        );
    }
    color = treasure_shadow_source(
        color,
        input.world_position,
        globals.treasure_positions[1],
        globals.treasure_shadow_intensity[1],
    );

    // `_ENABLE_MODULE_FRESNEL` keyword 选中这个体；材质的 `_UseFresnel`
    // 槽在该记录里是 unused，不参与分支。
    if features.enable_fresnel {
        let view = normalize_source(input.view_direction);
        let mut one_minus = 1.0 - dot3(normal, view);
        one_minus = one_minus.clamp(0.0, 1.0);
        one_minus = one_minus.log2();
        one_minus *= material.fresnel_power;
        one_minus = one_minus.exp2();
        for c in 0..3 {
            color[c] += one_minus * material.fresnel_color[c] * material.fresnel_color[3];
        }
    }
    fog_source(color, input.world_position[1], input.fog_factor, globals)
}

fn normalize_source(v: [f32; 3]) -> [f32; 3] {
    // 保持源的 dot → 求逆 → 乘的顺序；零向量与 shader 一样传播非有限值。
    let mut length_squared = v[0] * v[0];
    length_squared += v[1] * v[1];
    length_squared += v[2] * v[2];
    let inverse_length = 1.0 / length_squared.sqrt();
    [
        inverse_length * v[0],
        inverse_length * v[1],
        inverse_length * v[2],
    ]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let mut out = a[0] * b[0];
    out += a[1] * b[1];
    out += a[2] * b[2];
    out
}

fn transform_point_column_major(matrix: [f32; 16], position: [f32; 3]) -> [f32; 3] {
    [
        matrix[0] * position[0] + matrix[4] * position[1] + matrix[8] * position[2] + matrix[12],
        matrix[1] * position[0] + matrix[5] * position[1] + matrix[9] * position[2] + matrix[13],
        matrix[2] * position[0] + matrix[6] * position[1] + matrix[10] * position[2] + matrix[14],
    ]
}

fn transform_clip_column_major(matrix: [f32; 16], position: [f32; 3]) -> [f32; 4] {
    [
        matrix[0] * position[0] + matrix[4] * position[1] + matrix[8] * position[2] + matrix[12],
        matrix[1] * position[0] + matrix[5] * position[1] + matrix[9] * position[2] + matrix[13],
        matrix[2] * position[0] + matrix[6] * position[1] + matrix[10] * position[2] + matrix[14],
        matrix[3] * position[0] + matrix[7] * position[1] + matrix[11] * position[2] + matrix[15],
    ]
}

/// 顶点程序的逐句翻译：逆转置法线（带源的 epsilon 钳制）、透视/正交
/// 两种视线、UV 滚动、原样顶点色 varying、雾深式子。
#[must_use]
pub fn water_vertex(input: WaterVertexInputs) -> WaterVertexVaryings {
    let world_position = transform_point_column_major(input.object_to_world, input.position);
    let clip_position = transform_clip_column_major(input.view_projection, world_position);

    let mut transformed_normal = [0.0f32; 3];
    for row in 0..3 {
        transformed_normal[row] = input.normal[0] * input.world_to_object[row]
            + input.normal[1] * input.world_to_object[4 + row]
            + input.normal[2] * input.world_to_object[8 + row];
    }
    let mut normal_length_squared = dot3(transformed_normal, transformed_normal);
    normal_length_squared = normal_length_squared.max(1.17549435e-38);
    let normal_inverse_length = 1.0 / normal_length_squared.sqrt();
    let world_normal = [
        normal_inverse_length * transformed_normal[0],
        normal_inverse_length * transformed_normal[1],
        normal_inverse_length * transformed_normal[2],
    ];

    let mut to_camera = [
        input.camera_position[0] - world_position[0],
        input.camera_position[1] - world_position[1],
        input.camera_position[2] - world_position[2],
    ];
    let camera_length_inverse = 1.0 / dot3(to_camera, to_camera).sqrt();
    for value in &mut to_camera {
        *value *= camera_length_inverse;
    }
    let view_direction = if input.ortho_params_w == 0.0 {
        to_camera
    } else {
        [
            input.view_matrix[2],
            input.view_matrix[6],
            input.view_matrix[10],
        ]
    };

    let uv = scrolled_uv(input.uv, input.time_y, input.uv_scroll);
    let mut fog_depth = clip_position[2] + input.projection_params[1];
    let denominator = input.projection_params[1] + input.projection_params[2];
    fog_depth /= denominator;
    fog_depth *= input.projection_params[2];
    fog_depth = fog_depth.max(0.0);
    fog_depth = fog_depth * input.fog_params[0] + input.fog_params[1];
    let fog_factor = fog_depth.clamp(0.0, 1.0);

    WaterVertexVaryings {
        clip_position,
        world_position,
        world_normal,
        uv,
        view_direction,
        color: input.color,
        fog_factor,
    }
}

/// 顶点程序的输入。矩阵是 Unity 列主序（`column * 4 + row`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterVertexInputs {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub object_to_world: [f32; 16],
    pub world_to_object: [f32; 16],
    pub view_projection: [f32; 16],
    pub view_matrix: [f32; 16],
    pub camera_position: [f32; 3],
    /// 源的 `unity_OrthoParams.w`；零选透视支路。
    pub ortho_params_w: f32,
    pub projection_params: [f32; 4],
    pub fog_params: [f32; 4],
    pub time_y: f32,
    pub uv_scroll: [f32; 2],
}

/// [`water_vertex`] 产出的 varying。采样不属于顶点程序，交给宿主。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterVertexVaryings {
    pub clip_position: [f32; 4],
    pub world_position: [f32; 3],
    pub world_normal: [f32; 3],
    pub uv: [f32; 2],
    pub view_direction: [f32; 3],
    /// 源的 `vs_TEXCOORD4`，原样顶点色。
    pub color: [f32; 4],
    pub fog_factor: f32,
}

/// `_USE_ALPHA_CLIP` 变体的 discard 判据：被选 alpha（可选顶点 alpha 乘后）
/// 严格低于字面 `0.5` 才丢弃。与输出 alpha（被选 alpha × `_BaseOpacity`）
/// 是两个量，裁核的是前者。
#[must_use]
pub fn alpha_clip_keeps(main_alpha: f32, vertex_alpha: f32, use_vertex_alpha_opacity: f32) -> bool {
    let selected = if use_vertex_alpha_opacity > 0.5 {
        main_alpha * vertex_alpha
    } else {
        main_alpha
    };
    selected >= 0.5
}

/// 所有 Water 颜色变体共享的顶点 UV 滚动：
/// `(-_Time.yy) * vec2(_UVScrollX, _UVScrollY) + in_TEXCOORD0`。
#[must_use]
pub fn scrolled_uv(vertex_uv: [f32; 2], time_y: f32, uv_scroll: [f32; 2]) -> [f32; 2] {
    [
        -time_y * uv_scroll[0] + vertex_uv[0],
        -time_y * uv_scroll[1] + vertex_uv[1],
    ]
}

/// 第一/第二 overlay keyword 记录共享的 overlay UV：选择子 `1` 取原始
/// 次级 UV，其余（含默认臂）取已滚动的 UV；再过该层的 ST 与自滚动。
#[must_use]
pub fn overlay_uv(
    scrolled_main_uv: [f32; 2],
    secondary_uv: [f32; 2],
    selector: i32,
    st: [f32; 4],
    time_y: f32,
    layer_scroll: [f32; 2],
) -> [f32; 2] {
    let selected = if selector == 1 {
        secondary_uv
    } else {
        scrolled_main_uv
    };
    [
        selected[0] * st[0] + st[2] - time_y * layer_scroll[0],
        selected[1] * st[1] + st[3] - time_y * layer_scroll[1],
    ]
}

/// 源的无 keyword Base 片元。
#[must_use]
pub fn base_fragment_color(
    input: WaterFragmentInputs,
    globals: WaterGlobals,
    material: WaterMaterial,
) -> [f32; 4] {
    let base = unlit_base(
        input.main_tex_rgba,
        input.vertex_color,
        material.use_vertex_color_blend,
    );
    fragment_color(
        input,
        globals,
        material,
        base,
        WaterFragmentFeatures::default(),
    )
}

/// 求一个 keyword 变体的 `SV_Target0`，公共段次序原样保留。
#[must_use]
pub fn fragment_color(
    input: WaterFragmentInputs,
    globals: WaterGlobals,
    material: WaterMaterial,
    base: [f32; 3],
    features: WaterFragmentFeatures,
) -> [f32; 4] {
    if features.skip_phenomena_lighting {
        // skip-light 变体连 normal、宝藏、雾的全局量都不读；在求这些项
        // 之前返回，坏输入不能改变结果。
        let alpha = source_alpha(
            input.main_tex_rgba,
            input.vertex_color,
            material.use_vertex_alpha_opacity,
        ) * material.base_opacity;
        return [base[0], base[1], base[2], alpha];
    }
    let color = lit_color_source(input, globals, material, base, features);
    let alpha = source_alpha(
        input.main_tex_rgba,
        input.vertex_color,
        material.use_vertex_alpha_opacity,
    ) * material.base_opacity;
    [color[0], color[1], color[2], alpha]
}

/// `_ENABLE_MODULE_FRESNEL` 变体的便捷封装。
#[must_use]
pub fn fresnel_fragment_color(
    input: WaterFragmentInputs,
    globals: WaterGlobals,
    material: WaterMaterial,
) -> [f32; 4] {
    let base = unlit_base(
        input.main_tex_rgba,
        input.vertex_color,
        material.use_vertex_color_blend,
    );
    fragment_color(
        input,
        globals,
        material,
        base,
        WaterFragmentFeatures {
            skip_phenomena_lighting: false,
            enable_fresnel: true,
        },
    )
}

/// 从两张已采样纹理求 overlay keyword 变体。
#[must_use]
pub fn overlay_fragment_color(
    input: WaterFragmentInputs,
    globals: WaterGlobals,
    material: WaterMaterial,
    overlay_tex_rgba: [f32; 4],
    overlay_vertex_alpha: f32,
    features: WaterFragmentFeatures,
) -> [f32; 4] {
    let base = overlay_base_rgb(
        input.main_tex_rgba,
        overlay_tex_rgba,
        input.vertex_color,
        overlay_vertex_alpha,
        material.use_vertex_color_blend,
    );
    fragment_color(input, globals, material, base, features)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::within_one_ulp;

    fn globals() -> WaterGlobals {
        WaterGlobals {
            mip_bias: -0.125,
            directional_light_vector: [0.4, 0.7, -0.1],
            phenomena_directional_light_color: [0.8, 0.6, 0.3, 0.65],
            phenomena_shade_color: [0.25, 0.5, 0.75, 1.0],
            edge_threshold: 0.45,
            edge_smoothness: 0.18,
            treasure_positions: [[0.0, 0.0, 0.0], [2.0, 0.0, -1.0]],
            treasure_shadow_intensity: [0.4, 0.7],
            fog_params: [0.0, 0.0, 0.22, 0.0],
            fog_near_color: [0.2, 0.3, 0.4, 0.8],
            fog_far_color: [0.05, 0.08, 0.12, 0.2],
        }
    }

    fn material() -> WaterMaterial {
        WaterMaterial {
            base_opacity: 0.9,
            use_vertex_color_blend: 1.0,
            use_vertex_alpha_opacity: 1.0,
            override_shading_parameter: 1.0,
            local_shading_intensity: 0.75,
            local_edge_threshold: 0.52,
            local_edge_smoothness: 0.11,
            fresnel_color: [0.2, 0.4, 0.8, 0.6],
            fresnel_power: 2.3,
        }
    }

    fn input() -> WaterFragmentInputs {
        WaterFragmentInputs {
            world_position: [1.7, 2.3, -0.4],
            world_normal: [0.3, 0.8, -0.2],
            vertex_color: [0.7, 0.55, 0.9, 0.8],
            main_tex_rgba: [0.22, 0.47, 0.81, 0.63],
            fog_factor: 0.35,
            view_direction: [0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn vertex_color_blend_squares_the_vertex_colour_not_its_alpha() {
        // 源的 tex.wxyz * vc.wxyz 是逐分量乘：blend 开时底色是
        // tex.rgb * vc.rgb²，顶点 alpha 只进 alpha 路。
        let tex = [0.5, 0.5, 0.5, 0.5];
        let vc = [0.8, 0.4, 0.5, 0.25];
        // 期望是源两次乘的精确 f32 值（products 再乘 vc）。
        assert_eq!(unlit_base(tex, vc, 1.0), [0.32000002, 0.080000006, 0.125]);
        // blend 关：纹理 rgb 原样。
        assert_eq!(unlit_base(tex, vc, 0.0), [0.5, 0.5, 0.5]);
        // products：alpha 路 = tex.a * vc.a。
        assert_eq!(vertex_alpha_products(tex, vc), [0.125, 0.4, 0.2, 0.25]);
        assert_eq!(source_alpha(tex, vc, 1.0), 0.125);
        assert_eq!(source_alpha(tex, vc, 0.0), 0.5);
    }

    #[test]
    fn base_fragment_matches_independent_source_vector() {
        let actual = base_fragment_color(input(), globals(), material());
        // 期望位按源记录的语句顺序独立求值，与本 crate 无关。逐通道
        // ±1 ULP 开窗：exp2/log2 在平台 libm 的实现间、以及优化器对
        // powf↔exp2 的改写之间会挪 1 ULP——公式级错误挪动远超 1 ULP，
        // 窗只吸收这层实现噪声。
        let expected_bits = [0x3d8c_d00e, 0x3dae_1cac, 0x3e6b_3adb, 0x3ee8_3e42];
        for (c, (got, want)) in actual.iter().zip(expected_bits).enumerate() {
            assert!(
                within_one_ulp(*got, f32::from_bits(want)),
                "channel {c}: got {:08x}, expected {want:08x}",
                got.to_bits()
            );
        }
    }

    #[test]
    fn fresnel_keyword_adds_the_source_log2_exp2_term() {
        let out = fresnel_fragment_color(input(), globals(), material());
        let expected_bits = [0x3e2a_39fa, 0x3e8f_591e, 0x3f1e_a0a9, 0x3ee8_3e42];
        for (c, (got, want)) in out.iter().zip(expected_bits).enumerate() {
            assert!(
                within_one_ulp(*got, f32::from_bits(want)),
                "fresnel channel {c}: got {:08x}, expected {want:08x}",
                got.to_bits()
            );
        }
        assert!(out[0] > base_fragment_color(input(), globals(), material())[0]);
    }

    #[test]
    fn skip_light_variant_is_direct_source_output() {
        let mut m = material();
        m.use_vertex_color_blend = 0.0;
        m.use_vertex_alpha_opacity = 0.0;
        let i = input();
        let out = fragment_color(
            i,
            globals(),
            m,
            [i.main_tex_rgba[0], i.main_tex_rgba[1], i.main_tex_rgba[2]],
            WaterFragmentFeatures {
                skip_phenomena_lighting: true,
                enable_fresnel: false,
            },
        );
        assert_eq!(
            out,
            [
                i.main_tex_rgba[0],
                i.main_tex_rgba[1],
                i.main_tex_rgba[2],
                0.63 * 0.9
            ]
        );
    }

    #[test]
    fn vertex_translation_normal_view_uv_and_fog_match_source_order() {
        let mut object_to_world = [0.0f32; 16];
        let mut world_to_object = [0.0f32; 16];
        let mut view_projection = [0.0f32; 16];
        let mut view_matrix = [0.0f32; 16];
        for matrix in [
            &mut object_to_world,
            &mut world_to_object,
            &mut view_projection,
            &mut view_matrix,
        ] {
            matrix[0] = 1.0;
            matrix[5] = 1.0;
            matrix[10] = 1.0;
            matrix[15] = 1.0;
        }
        object_to_world[12] = 1.0;
        object_to_world[13] = -1.0;
        view_matrix[2] = 0.25;
        view_matrix[6] = 0.5;
        view_matrix[10] = 0.75;
        let vertex_input = WaterVertexInputs {
            position: [2.0, 4.0, 6.0],
            normal: [0.0, 2.0, 0.0],
            uv: [0.25, 0.75],
            color: [0.7, 0.55, 0.9, 0.8],
            object_to_world,
            world_to_object,
            view_projection,
            view_matrix,
            camera_position: [3.0, 3.0, 10.0],
            ortho_params_w: 0.0,
            projection_params: [1.0, 1.0, 1.0, 0.0],
            fog_params: [0.5, 0.1, 0.2, 0.0],
            time_y: 2.0,
            uv_scroll: [0.1, -0.2],
        };
        let v = water_vertex(vertex_input);
        assert_eq!(v.world_position, [3.0, 3.0, 6.0]);
        assert_eq!(v.clip_position, [3.0, 3.0, 6.0, 1.0]);
        assert_eq!(v.world_normal, [0.0, 1.0, 0.0]);
        assert_eq!(v.view_direction, [0.0, 0.0, 1.0]);
        // 顶点色 varying 原样：这一族的平方不在顶点程序里。
        assert_eq!(v.color, [0.7, 0.55, 0.9, 0.8]);
        assert_eq!(v.uv, [0.049999997, 1.15]);
        assert_eq!(v.fog_factor, 1.0);

        let ortho = water_vertex(WaterVertexInputs {
            ortho_params_w: 1.0,
            ..vertex_input
        });
        assert_eq!(ortho.view_direction, [0.25, 0.5, 0.75]);
    }

    #[test]
    fn overlay_and_alpha_clip_follow_source_swizzles_and_boundary() {
        let i = input();
        let out = overlay_base_rgb(
            i.main_tex_rgba,
            [0.9, 0.2, 0.1, 0.4],
            i.vertex_color,
            1.0,
            1.0,
        );
        let mixed = [
            0.22 + 0.8 * 0.4 * (0.9 - 0.22),
            0.47 + 0.8 * 0.4 * (0.2 - 0.47),
            0.81 + 0.8 * 0.4 * (0.1 - 0.81),
        ];
        let expected = [
            mixed[0] * 0.7 * 0.7,
            mixed[1] * 0.55 * 0.55,
            mixed[2] * 0.9 * 0.9,
        ];
        for c in 0..3 {
            assert!((out[c] - expected[c]).abs() < 1.0e-6, "overlay channel {c}");
        }
        assert!(alpha_clip_keeps(0.625, 0.8, 1.0));
        assert!(!alpha_clip_keeps(0.62499994, 0.8, 1.0));
        assert!(alpha_clip_keeps(0.5, 0.01, 0.0));
        assert_eq!(
            scrolled_uv([0.25, 0.75], 2.0, [0.1, -0.2]),
            [0.049999997, 1.15]
        );
        let overlay1 = overlay_uv(
            [0.05, 1.15],
            [0.8, 0.2],
            1,
            [2.0, 3.0, 0.1, -0.2],
            2.0,
            [0.25, -0.5],
        );
        assert!((overlay1[0] - 1.2).abs() < 1.0e-6);
        assert!((overlay1[1] - 1.4).abs() < 1.0e-6);
        let overlay_default = overlay_uv(
            [0.05, 1.15],
            [0.8, 0.2],
            7,
            [2.0, 3.0, 0.1, -0.2],
            2.0,
            [0.25, -0.5],
        );
        assert!((overlay_default[0] + 0.3).abs() < 1.0e-6);
        assert!((overlay_default[1] - 4.25).abs() < 1.0e-6);
    }
}
