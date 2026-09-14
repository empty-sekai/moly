//! 家具 Basic 族质感分支的两条加色律：菲涅尔（`fresnel_add`）与反射
//! （`reflection_add`）。两条是**并列的独立开关**，同一个材质可以只开
//! 一条、也可以两条同开；同开时源里那个 `1−N·V` 只算一次、两支各取
//! 一份（一支吃钳后的、一支吃未钳的，见 `reflection_add` 的说明）。
//!
//! 以下是菲涅尔那条。源程序（质感分支开启的着色变体）的片元尾部是
//! 「加色」而非「乘色」：fresnel 项 = exp2(log2(clamp(1−N·V, 0, 1))
//! × power) × color.rgb × color.a，逐通道加在宝藏阴影之后、雾之前的
//! rgb 上。三个承重点，缺一个就是另一个公式：
//!
//! - **先 clamp(1−N·V, 0, 1) 再取对数**——N·V 略超 1（法线/视线各自
//!   归一化后再点积，完全可能）时，不钳制会走 log2(负数) = NaN；
//! - **color 的 alpha 是因子**——项还要乘 color.a（bike1 实值
//!   0.15686…，量级直接砍到六分之一），不是只乘 rgb；
//! - **视线方向分两支**：透视相机用「相机位 − 片元位」归一化（无
//!   epsilon 钳制，与法线支不同）；正交相机直接取视图矩阵的第三行
//!   （列主序 [2]/[6]/[10]），由 unity_OrthoParams.w 是否非零选择。
//!
//! 期望位由源程序自身的运算顺序在 float32 上独立求值（numpy 逐算子
//! 转写），与本 crate 无关；断言开 ±1 ULP 窗吸收 exp2/log2 的 libm
//! 实现差，公式级错误挪动远超 1 ULP。

/// 视线方向：`ortho_params_w` 非零走正交支（视图矩阵第三行，列主序
/// 下标 [2]/[6]/[10]），否则透视支（相机位减片元位后按
/// inversesqrt(dot) 归一化——与法线支不同，源在这里没有 epsilon
/// 钳制，零向量会照原样产生 NaN，不替源修）。
#[must_use]
pub fn view_direction(
    camera_position: [f32; 3],
    world_position: [f32; 3],
    view_matrix: [f32; 16],
    ortho_params_w: f32,
) -> [f32; 3] {
    if ortho_params_w != 0.0 {
        [view_matrix[2], view_matrix[6], view_matrix[10]]
    } else {
        let to_camera = [
            camera_position[0] - world_position[0],
            camera_position[1] - world_position[1],
            camera_position[2] - world_position[2],
        ];
        let dot = to_camera[0] * to_camera[0]
            + to_camera[1] * to_camera[1]
            + to_camera[2] * to_camera[2];
        let inv = 1.0 / dot.sqrt();
        [
            to_camera[0] * inv,
            to_camera[1] * inv,
            to_camera[2] * inv,
        ]
    }
}

/// 菲涅尔项逐通道加回 rgb：按源的运算顺序
/// `(-ndv+1) → clamp(0,1) → log2 → ×power → exp2 → ×color.rgb →
/// ×color.a → +rgb`。mediump 标记是 GLES 翻译产物，源 HLSL 是
/// float32，本律按 float32 求值。
#[must_use]
pub fn fresnel_add(
    rgb: [f32; 3],
    n_dot_v: f32,
    fresnel_power: f32,
    fresnel_color: [f32; 4],
) -> [f32; 3] {
    let one_minus = (-n_dot_v + 1.0).clamp(0.0, 1.0);
    let fresnel = (one_minus.log2() * fresnel_power).exp2();
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = (fresnel * fresnel_color[c]) * fresnel_color[3] + rgb[c];
    }
    out
}

/// 一个**未绑定**的立方图采样器采出来的值，每通道恒定。
///
/// 引擎为每种纹理维度备了一张内建默认贴图，绘制期发现某个采样器没有
/// 绑定对象时按**维度**索引那张表顶上去（三条绘制期路径——纹理参数
/// 准备、属性表查不到时的兜底、共享材质数据取用——全部按维度索引，
/// 没有一条按声明里的默认名解析；那个名字只有编辑器侧工具读）。
/// 立方图那一格的内建贴图是**六个面各自整面清成同一个颜色**的，于是：
///
/// **采样值与采样方向、mip 级、mip 偏置、过滤模式全部无关**——六面同色的
/// 贴图，任何方向、任何 mip、任何插值权重下取出来的都是那一个颜色。
/// 这就是整条分支能折叠成常数的根据，它不依赖那张贴图有多大或有几级。
///
/// 值 = 每通道 128/255。两侧独立取证：编辑器二进制里那张表的立方图槽位
/// 与「全黑立方图」走**同一个构造函数**、只差填充实参（0x80808080 对 0）；
/// 游戏自带的引擎动态库里同两个站点也是**同一个构造函数**、同两个填充值
/// （逐指令解过，全黑那个站点当阳性对照）。填充色以逐通道 8 位的身份喂给
/// 清屏调用 ⇒ R=G=B=A=128。工程跑在 gamma 空间（现算，播放器设置里的
/// 色彩空间标记取 gamma 那一档）⇒ 引擎给默认贴图选的是无 sRGB 标记的
/// 8 位归一化格式 ⇒ **采样时不做 sRGB 解码，128/255 不打折**。
///
/// 字面量写死而不写成 `128.0 / 255.0`：那个除法在两种语言的常量求值里
/// 精度路径不同（一侧单精度一次舍入，另一侧先高精度再落回来，是两次
/// 舍入），写成十进制字面量则两侧解析出同一批位。这个十进制是该 f32 的
/// 最短往返形，别把它「化简」回除法。
pub const UNBOUND_CUBE_SAMPLE: f32 = 0.5019608;

/// 质感反射项逐通道加回 rgb，**按未绑定立方图折叠后的形态**。
///
/// 源序：`d = N·V` → `1−d` → `log2` → `×power` → `exp2` → `min(·, 1)` →
/// `×立方图采样` → `×intensity` → `+rgb`。四个承重点：
///
/// - **`log2` 的入参没有下钳**——与相邻的菲涅尔支路不同。两支在源里
///   **共用同一个 `1−d`**（同时开着的变体里那个减法只算一次）：菲涅尔支
///   吃的是 `clamp(·,0,1)` 之后的副本，反射支吃的是**未钳的原件**。
///   两支各钳一次、或者让反射支也吃钳后的值，都是另一个函数——
///   `N·V` 为负（法线背离视线）时未钳的 `1−d` 会大于 1，那正是下面那个
///   上钳存在的理由。
/// - **只有上钳 `min(·, 1)`**，写成显式比较是为了对齐被移植语言的 `min`
///   语义（只在 `1.0 < fresnel` 时取 1.0）⇒ `N·V` 因浮点误差略微超过 1 时
///   `log2` 收到负数、结果是 NaN，而这个 NaN **照原样穿过去**。源就是这个
///   形状，不替它补下钳。
/// - **乘法结合序是 `(采样 × fresnel) × intensity`**，两次乘法，采样常数
///   先乘。把两个常数预乘成一个是代数重结合，f32 乘法不满足结合律——
///   折叠只替换掉「取那个纹素」这一步，不动源写下的运算次序。
/// - **加色不是乘色**，且 alpha 不参与、不被改写。
///
/// `N·V` 恰为 1（正对视线）时 `log2(0)` 走 IEEE 的负无穷、`exp2` 回到 0
/// ⇒ 项归零，与逼近正对时的极限一致。
///
/// ⚠ **适用边界：只对立方图槽位为空的材质成立。** 真源里这条分支开着的
/// 材质分两族：蛋系那族的槽位是**空引用**（复核过，同一张纹理表里主贴图
/// 每条都非空，所以「空」不是读法坏了）⇒ 走本函数；载具那族**绑了一张
/// 真的立方图**（三个材质指向同一个对象，128×128、八级 mip 链完整在盘）
/// ⇒ **本函数对它们是错的**，必须真采样，而且 mip 偏置在那边会真的选 mip。
/// 载具那族当前一个都没摆放。
///
/// ⚠ **不要反过来「补一张灰色立方图」再去采样它。** 那是把一个已经证明
/// 为常数的取值重新展开成一次纹理读，多一张贴图、多一个采样器、多一条
/// 会漂的路径，换不到任何一位精度——六面同色贴图的采样结果与方向无关，
/// 这一点在上面那个常量的说明里是有出处的。
#[must_use]
pub fn reflection_add(
    rgb: [f32; 3],
    n_dot_v: f32,
    reflection_fresnel_power: f32,
    reflection_intensity: f32,
) -> [f32; 3] {
    let one_minus = -n_dot_v + 1.0;
    let fresnel = (one_minus.log2() * reflection_fresnel_power).exp2();
    let clamped = if 1.0 < fresnel { 1.0 } else { fresnel };
    let mut out = [0.0; 3];
    for c in 0..3 {
        out[c] = (UNBOUND_CUBE_SAMPLE * clamped) * reflection_intensity + rgb[c];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shading::within_one_ulp;

    /// bike1 材质的真实参数（资产 extras 里的 _FresnelPower / _FresnelColor）。
    const POWER: f32 = 3.0;
    const COLOR: [f32; 4] = [
        1.0,
        0.980_063_6,
        0.796_078_4,
        0.156_862_75,
    ];
    const RGB: [f32; 3] = [0.22, 0.47, 0.81];

    /// 期望位：源运算顺序在 float32 上独立求值（numpy 逐算子转写）。
    /// 每行挑了能钉住一个公式要素的 N·V：
    /// - 1.0000001：不先 clamp 就 log2(负数)=NaN ⇒ 钳制次序；
    /// - 0.0 与 −0.70710678 同位：负半轴饱和于 clamp ⇒ 钳制存在；
    /// - 0.0 的绝对值：项 = color.rgb×color.a+rgb ⇒ alpha 因子与幂；
    /// - 0.5/0.25：幂指数取值（power=2 或用 x² 都会挪出窗）。
    #[test]
    fn fresnel_add_matches_source_evaluated_bits() {
        let cases: [(f32, [u32; 3]); 7] = [
            (1.0, [0x3e61_47ae, 0x3ef0_a3d7, 0x3f4f_5c29]),
            (1.0000001, [0x3e61_47ae, 0x3ef0_a3d7, 0x3f4f_5c29]),
            (0.92387953, [0x3e61_59d1, 0x3ef0_acba, 0x3f4f_5fc5]),
            (0.70710678, [0x3e65_50e3, 0x3ef2_9e25, 0x3f50_29ca]),
            (0.5, [0x3e75_5bc2, 0x3efa_7aa4, 0x3f53_5b23]),
            (0.25, [0x3e92_85b9, 0x3f08_ec66, 0x3f5c_d8b5]),
            (0.0, [0x3ec0_f428, 0x3f1f_ad21, 0x3f6f_53f9]),
        ];
        for (n_dot_v, want_bits) in cases {
            let got = fresnel_add(RGB, n_dot_v, POWER, COLOR);
            for c in 0..3 {
                assert!(
                    within_one_ulp(got[c], f32::from_bits(want_bits[c])),
                    "ndv={n_dot_v} channel {c}: got {:08x}, expected {:08x}",
                    got[c].to_bits(),
                    want_bits[c]
                );
            }
        }
    }

    /// 负半轴 N·V 全部饱和在 clamp 上界——三种差异很大的输入同一位，
    /// 钉住「钳制发生在 log2 之前」；若先 pow 再钳，−1 的输入会给出
    /// exp2(3)=8 一类的爆炸值。
    #[test]
    fn negative_n_dot_v_saturates_at_the_same_bits() {
        for n_dot_v in [0.0, -0.70710678, -1.0, -3.0] {
            let got = fresnel_add(RGB, n_dot_v, POWER, COLOR);
            for c in 0..3 {
                assert_eq!(
                    got[c].to_bits(),
                    fresnel_add(RGB, 0.0, POWER, COLOR)[c].to_bits(),
                    "ndv={n_dot_v} channel {c} 未饱和于 clamp 上界"
                );
            }
        }
    }

    /// N·V=1（正对相机）时项恒为 0，输出逐位等于入参 rgb——加色而非
    /// 乘色的钉子：任何「乘以颜色」的写法在这里都会偏离。
    #[test]
    fn head_on_view_returns_rgb_unchanged() {
        let got = fresnel_add(RGB, 1.0, POWER, COLOR);
        for c in 0..3 {
            assert_eq!(got[c].to_bits(), RGB[c].to_bits());
        }
    }

    /// 透视支的期望位（cam=(3,5,10), pos=(1.5,−2,4) →
    /// (0.1605863, 0.7494028, 0.6423453)）。
    #[test]
    fn perspective_view_direction_matches_source_evaluated_bits() {
        let got = view_direction(
            [3.0, 5.0, 10.0],
            [1.5, -2.0, 4.0],
            [0.0; 16],
            0.0,
        );
        let want = [0x3e24_70be, 0x3f3f_d8dd, 0x3f24_70be];
        for c in 0..3 {
            assert!(
                within_one_ulp(got[c], f32::from_bits(want[c])),
                "channel {c}: got {:08x}, expected {:08x}",
                got[c].to_bits(),
                want[c]
            );
        }
    }

    /// 正交支取视图矩阵第三行（列主序 [2]/[6]/[10]）：喂一个 16 元素
    /// 互不相同的矩阵，读错任何下标（平移列 [3]/[7]/[11]、行主序）都
    /// 会给出别的值。
    #[test]
    fn ortho_view_direction_reads_the_third_row() {
        let matrix: [f32; 16] = core::array::from_fn(|i| (i as f32 + 1.0) / 64.0);
        let got = view_direction([9.0, 9.0, 9.0], [1.0, 2.0, 3.0], matrix, 1.0);
        let want = [matrix[2], matrix[6], matrix[10]];
        assert_eq!(got, want);
        // 选择器是「w 是否非零」：0.5 也走正交支，恰 0 才走透视支。
        let half = view_direction([9.0, 9.0, 9.0], [1.0, 2.0, 3.0], matrix, 0.5);
        assert_eq!(half, want);
        let persp = view_direction([3.0, 5.0, 10.0], [1.5, -2.0, 4.0], matrix, 0.0);
        assert_ne!(persp, want);
    }
}
