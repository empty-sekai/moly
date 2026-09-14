//! `Mysekai/Site/Ground` Base 程序（顶点 + 片元）的逐句翻译。
//!
//! 两个 keyword 变体——无 keyword 与 `_USE_ALPHA_CLIP`——的片元差异只有
//! discard 与 alpha 附近的语句次序，不改变任何值。式子保持源程序的
//! 运算顺序，作为上屏 shader 移植的 oracle。
//!
//! 落影上色（真源全局量 `_MysekaiDropShadowColor1`）在本族收影变体里
//! 只应用一次（带该全局量的收影变体全体一致；FieldObject 族是两次）。
//! 落影的衰减与采样核不在本律内。

use super::{TREASURE_SHADOW_EDGE, TREASURE_SHADOW_INV_RANGE, TREASURE_SHADOW_SCALE};

/// 该族在真源里的 shader 名。
pub const SHADER_NAME: &str = "Mysekai/Site/Ground";

/// 一帧的全局量，字段与源程序的全局块声明一一对应。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundGlobals {
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

/// Ground Base 片元读的材质标量。overlay 与 Fresnel 槽在源的
/// `UnityPerMaterial` 块里是 `Xhlslcc_UnusedX_*`——编译后的 Base 体不读
/// 它们——所以这里没有可发明的字段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundMaterial {
    pub base_opacity: f32,
    pub use_vertex_color_blend: f32,
    pub use_vertex_alpha_opacity: f32,
    /// `_USE_ALPHA_CLIP` 变体在光照前 discard。
    pub use_alpha_clip: bool,
    pub override_shading_parameter: f32,
    pub local_shading_intensity: f32,
    pub local_edge_threshold: f32,
    pub local_edge_smoothness: f32,
}

/// 逐片元输入：顶点程序产出 varying、采样器返回 `_MainTex` 之后的值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundFragmentInputs {
    pub world_position: [f32; 3],
    pub world_normal: [f32; 3],
    /// 顶点程序把 COLOR0.xyz 平方进 TEXCOORD4、w 原样；两个变体都从
    /// 平方后的 rgb 与原样的 alpha 混合。
    pub vertex_color: [f32; 4],
    pub main_tex_rgba: [f32; 4],
    pub fog_factor: f32,
}

/// 源顶点程序的输入。矩阵是 Unity 列主序（`column * 4 + row`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundVertexInputs {
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

/// [`ground_vertex`] 产出的 varying。`color_rgb_squared` 是源的 TEXCOORD4：
/// COLOR0.xyz 平方、COLOR0.w 原样。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundVertexVaryings {
    pub clip_position: [f32; 4],
    pub world_position: [f32; 3],
    pub world_normal: [f32; 3],
    pub uv: [f32; 2],
    pub color_rgb_squared: [f32; 3],
    pub color_alpha: f32,
    pub fog_factor: f32,
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

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    let mut out = a[0] * b[0];
    out += a[1] * b[1];
    out += a[2] * b[2];
    out
}

/// 顶点程序的逐句翻译：逆转置法线（带源的 epsilon 钳制）、透视/正交
/// 两种视线、UV 滚动、平方顶点色、雾深式子。两个 keyword 变体的顶点
/// 程序相同。
#[must_use]
pub fn ground_vertex(input: GroundVertexInputs) -> GroundVertexVaryings {
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
    // 源顶点程序无条件算这个视线 varying；Base 片元不读它。
    let _view_direction = if input.ortho_params_w == 0.0 {
        to_camera
    } else {
        [
            input.view_matrix[2],
            input.view_matrix[6],
            input.view_matrix[10],
        ]
    };

    let uv = [
        -input.time_y * input.uv_scroll[0] + input.uv[0],
        -input.time_y * input.uv_scroll[1] + input.uv[1],
    ];
    let mut fog_depth = clip_position[2] + input.projection_params[1];
    let denominator = input.projection_params[1] + input.projection_params[2];
    fog_depth /= denominator;
    fog_depth *= input.projection_params[2];
    fog_depth = fog_depth.max(0.0);
    fog_depth = fog_depth * input.fog_params[0] + input.fog_params[1];
    let fog_factor = fog_depth.clamp(0.0, 1.0);

    GroundVertexVaryings {
        clip_position,
        world_position,
        world_normal,
        uv,
        color_rgb_squared: [
            input.color[0] * input.color[0],
            input.color[1] * input.color[1],
            input.color[2] * input.color[2],
        ],
        color_alpha: input.color[3],
        fog_factor,
    }
}

fn normalize_source(v: [f32; 3]) -> [f32; 3] {
    // 保持源的 dot → 求逆 → 乘的顺序。
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

/// 被选 alpha：两个变体都输出它，`_USE_ALPHA_CLIP` 变体也拿它做
/// discard 判据——`_UseVertexAlphaOpacity` 开时是 `TEXCOORD4.w * tex.w`，
/// 否则是 `tex.w`。
#[must_use]
pub fn selected_alpha(
    main_tex_alpha: f32,
    vertex_alpha: f32,
    use_vertex_alpha_opacity: f32,
) -> f32 {
    if use_vertex_alpha_opacity > 0.5 {
        main_tex_alpha * vertex_alpha
    } else {
        main_tex_alpha
    }
}

/// 源的比较是 `selected - 0.5 < 0` 才丢弃：恰好 0.5 保留。
#[must_use]
pub fn alpha_clip_keeps(selected: f32) -> bool {
    selected >= 0.5
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

fn treasure_shadow_source(
    color: [f32; 3],
    world_position: [f32; 3],
    treasure_position: [f32; 3],
    intensity: f32,
) -> [f32; 3] {
    let t = treasure_shadow_factor(world_position, treasure_position);
    let scale = intensity * TREASURE_SHADOW_SCALE;
    let mut out = color;
    for channel in &mut out {
        let delta = *channel * t + -*channel;
        *channel = scale * delta + *channel;
    }
    out
}

fn fog_source(
    color: [f32; 3],
    world_y: f32,
    fog_factor: f32,
    globals: GroundGlobals,
) -> [f32; 3] {
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

    // 源的 exp2：min 夹在它上面、乘的是插值后的雾色 alpha。平台的
    // powf(2.0,·) 与 exp2 在个别输入上相差 1 ULP，优化器还会把前者
    // 改写成后者；比对按 ±1 ULP 开窗（见 golden 测试）。
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

/// 求 Ground Base 片元的 `SV_Target0`。discard 由
/// [`GroundMaterial::use_alpha_clip`] 承担：被丢弃的片元回报全零。
#[must_use]
pub fn ground_fragment_color(
    input: GroundFragmentInputs,
    globals: GroundGlobals,
    material: GroundMaterial,
) -> [f32; 4] {
    let selected = selected_alpha(
        input.main_tex_rgba[3],
        input.vertex_color[3],
        material.use_vertex_alpha_opacity,
    );
    let alpha = selected * material.base_opacity;
    if material.use_alpha_clip && !alpha_clip_keeps(selected) {
        return [0.0, 0.0, 0.0, 0.0];
    }

    // 底色 = _MainTex.rgb × 平方后的顶点 rgb；blend 不乘顶点 alpha——
    // alpha 走 selected_alpha 那条路。
    let vc2 = input.vertex_color;
    let base = if material.use_vertex_color_blend > 0.5 {
        [
            input.main_tex_rgba[0] * vc2[0],
            input.main_tex_rgba[1] * vc2[1],
            input.main_tex_rgba[2] * vc2[2],
        ]
    } else {
        [
            input.main_tex_rgba[0],
            input.main_tex_rgba[1],
            input.main_tex_rgba[2],
        ]
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
    color = treasure_shadow_source(
        color,
        input.world_position,
        globals.treasure_positions[0],
        globals.treasure_shadow_intensity[0],
    );
    color = treasure_shadow_source(
        color,
        input.world_position,
        globals.treasure_positions[1],
        globals.treasure_shadow_intensity[1],
    );
    let color = fog_source(color, input.world_position[1], input.fog_factor, globals);
    [color[0], color[1], color[2], alpha]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::within_one_ulp;

    fn globals() -> GroundGlobals {
        GroundGlobals {
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

    fn material() -> GroundMaterial {
        GroundMaterial {
            base_opacity: 0.9,
            use_vertex_color_blend: 1.0,
            use_vertex_alpha_opacity: 1.0,
            use_alpha_clip: false,
            override_shading_parameter: 1.0,
            local_shading_intensity: 0.75,
            local_edge_threshold: 0.52,
            local_edge_smoothness: 0.11,
        }
    }

    /// 源的 TEXCOORD4：COLOR0.xyz 在顶点程序里平方、w 原样。
    /// 这三个是精确的 f32 平方值。
    const TEXCOORD4: [f32; 4] = [0.48999998, 0.3025, 0.80999994, 0.8];

    fn input() -> GroundFragmentInputs {
        GroundFragmentInputs {
            world_position: [1.7, 2.3, -0.4],
            world_normal: [0.3, 0.8, -0.2],
            vertex_color: TEXCOORD4,
            main_tex_rgba: [0.22, 0.47, 0.81, 0.63],
            fog_factor: 0.35,
        }
    }

    #[test]
    fn base_fragment_matches_source_evaluated_bits() {
        let actual = ground_fragment_color(input(), globals(), material());
        // 期望位从源程序自身的运算顺序独立求值，与本 crate 无关。
        // 逐通道 ±1 ULP 开窗：雾的 exp2 在平台上 powf/exp2 两个 libm
        // 实现间相差 1 ULP，优化器还会在二者间改写——公式级错误挪动
        // 远超 1 ULP，窗只吸收这层实现噪声。
        let expected_bits = [0x3d8c_d00d, 0x3dae_1cac, 0x3e6b_3adb, 0x3ee8_3e42];
        for (c, (got, want)) in actual.iter().zip(expected_bits).enumerate() {
            assert!(
                within_one_ulp(*got, f32::from_bits(want)),
                "channel {c}: got {:08x}, expected {want:08x}",
                got.to_bits()
            );
        }
    }

    #[test]
    fn alpha_clip_tests_the_selected_alpha_at_one_half() {
        assert!(alpha_clip_keeps(0.5));
        assert!(!alpha_clip_keeps(0.49999997));
        let mut m = material();
        m.use_alpha_clip = true;
        let mut i = input();
        i.main_tex_rgba[3] = 0.4;
        // 被选 alpha 0.4 * 0.8 = 0.32 < 0.5：discard。
        let discarded = ground_fragment_color(i, globals(), m);
        assert_eq!(discarded, [0.0, 0.0, 0.0, 0.0]);
        // 同一材质去掉 keyword：片元保留，输出 alpha 是被选 alpha 乘
        // _BaseOpacity。
        m.use_alpha_clip = false;
        let kept = ground_fragment_color(i, globals(), m);
        assert_eq!(kept[3], 0.4 * 0.8 * 0.9);
        // 裁核的是被选 alpha，不是乘 _BaseOpacity 之后的输出 alpha：
        // 被选 0.75 * 0.8 = 0.6 保留，输出 alpha 0.6 * 0.5 = 0.3——
        // 若按输出 alpha 裁，这个片元会被丢掉。
        m.use_alpha_clip = true;
        m.base_opacity = 0.5;
        i.main_tex_rgba[3] = 0.75;
        let kept = ground_fragment_color(i, globals(), m);
        assert_eq!(kept[3], 0.75 * 0.8 * 0.5);
    }

    #[test]
    fn vertex_stage_squares_colour_scrolls_uv_and_derives_fog() {
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
        let vertex_input = GroundVertexInputs {
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
        let v = ground_vertex(vertex_input);
        assert_eq!(v.world_position, [3.0, 3.0, 6.0]);
        assert_eq!(v.world_normal, [0.0, 1.0, 0.0]);
        assert_eq!(v.uv, [0.049999997, 1.15]);
        // 平方发生在顶点程序里：COLOR0.rgb 的精确 f32 平方。
        assert_eq!(v.color_rgb_squared, [0.48999998, 0.3025, 0.80999994]);
        assert_eq!(v.color_alpha, 0.8);
        // clip z = 6 → 雾深 1.85 → clamp 到 1。
        assert_eq!(v.fog_factor, 1.0);
    }
}
