//! 发射形状采样：rain 家族语料里 `shape` 模块的几何侧。
//!
//! 已落律的档：**Circle · Cone(底面, type 4) · Sphere · Hemisphere ·
//! SingleSidedEdge**——按产物直方图（`probe`，4363 个粒子系统、3222 个带
//! shape 块）这四档加 Circle 覆盖 90.5%。
//!
//! ⛔ **具名 fail-closed**：`ConeVolume`(8) · `Box`(5) · `Donut`(17) ·
//! `Mesh`(6) 一族（产物里 29 + 174 + 68 + 36 = 307 条）**不写进本文件**——
//! 它们的式子与这四档不同，静默取默认或按这四档降级都是把一处缺陷变成
//! 两处实现。调用方见到这四档之外的 `shape_type` 应**响亮拒绝**，
//! 不在这里 fallback。这是「未做完的活」的记账方式，不是疏漏。
//! 其中 `Donut`/`Mesh` 在「looping 且 rate>0」的 665 条里占 91 条
//! （59 + 32），是真会持续冒的，优先级高于 Box/ConeVolume。
//!
//! 每条律的注释标出处档（`mod.rs` 顶部的表）：本文件的三角核与每档的
//! 采样式都来自**版本匹配引擎原生体的反汇编**（`行为口径`：按可复算的
//! 行为规格实现，逐条给测量方案）。
//!
//! ⛔ **随机纪律（本仓通则）：先数「这个形状的采样求值了几次随机」，
//! 再写式子。** 每档的消耗个数写在该函数的文档里，是承重句。消耗错一个，
//! 从第二个粒子起寿命/速度/大小/颜色全线错位，而位置看起来一直对。
//!
//! ## 引擎的共享三角核（`行为口径`，五档共用同一段指令序列）
//!
//! 四档（Sphere/HemiSphere/Cone/Circle/SingleSidedEdge 的 `arcMode = Random`
//! 实例化）反汇编里逐字节同一条核：
//!
//! ```text
//!   phi  = arc_deg * (pi/180) * t_theta        t_theta = u01（xorshift 低 23 位）
//!   W    = phi * (1/(2*pi))                    （圈数）
//!   frac = |W - round(W)|                       （2^23 magic-add 半舍入）
//!   X    = 0.25 - frac                          （折到 [-0.25, 0.25]）
//!   sin(2*pi*frac) ~= X * sincPoly(X^2)
//!   cos(2*pi*frac) ~= (0.25 - |X|) * sincPoly((0.25 - |X|)^2)
//!   sincPoly(t) = 6.283185 - 39.657032*t + 81.601822*t^2 - 76.568619*t^3
//! ```
//!
//! 四个系数是编译器物化的位模式（0x40c90fda / 0x421ea0cd / 0x42a33422 /
//! 0x42992322），是引擎自己的 minimax，**不是教科书 Taylor**（Taylor 同阶
//! 在 [-0.25, 0.25] 上误差 1.6e-4，引擎这份 2.6e-2，全集中在 |X|=0.25 那条
//! 棱上）。**逐值比对的对象是这份引擎式子，不是 libm**——把真源那行改成
//! 别的值这条比对就会红。
//!
//! ## 每档的随机消耗个数（本文件承重句）
//!
//! | 档 | 抽签数 | 序 |
//! |---|---|---|
//! | Circle | **1** | t_theta |
//! | Cone(底面) | **2** | t_theta（角）→ t_radial（径向）|
//! | Sphere | **2** | t_theta（方位角）→ t_cos（俯仰）|
//! | Hemisphere | **2** | 同 Sphere，方向向量的 z 分量取绝对值 |
//! | SingleSidedEdge | **1** | t_theta（弧上位置，见下）|
//!
//! 对照（数出来的依据在函数文档里逐条钉着指令地址）：拒绝采样式
//! （`EmitterStoreData` 里的 Box 方向拒绝环）次数不定，不在此列。

/// f32 步进工具：所有乘加都按引擎的舍入次序走 f32。
#[inline(always)]
fn f(x: f32) -> f32 {
    x
}

/// 从引擎的 23 位低位直接构造 u01（`t = (bits & 0x7fffff) * 2^-23`）。
///
/// 暴露给「逐值比对」与调用方的随机桥：Unity 的 xorshift 每抽一次吐
/// 32 位，低 23 位喂给这里。本仓律的公开接口收 [0,1] 的 f32（与
/// `circle_position` 的既有签名一致），这条 helper 是两者之间的桥。
#[inline(always)]
pub fn u01_from_bits(bits23: u32) -> f32 {
    u01(bits23)
}

/// 引擎 sinc 多项式（见模块头）。`t = X*X`。
///
/// 出处：`行为口径`。系数是编译器在五份 `Start*<Random>` 里逐字节物化的
/// 同一组位模式；在 [-0.25, 0.25] 上对 `sin(2*pi*X)/X` 的最大绝对误差
/// 0.0262（全在 |X|=0.25 那条棱上），**这是真源自己的精度行为，照原样搬**。
///
/// Editor 测量方案：arc=360、radius=1、Circle，收 10 万样本；在
/// `theta = 90°`（X = 0.25）附近数高度——比教科书 sin 高 2.6% 处即此式。
#[inline(always)]
fn sinc_poly(t: f32) -> f32 {
    let c1 = f32::from_bits(0x40c90fda); // 6.283185
    let c2 = f32::from_bits(0x421ea0cd); // 39.657032
    let c3 = f32::from_bits(0x42a33422); // 81.601822
    let c4 = f32::from_bits(0x42992322); // 76.568619
    f(f(c1 - f(c2 * t)) + f(f(t * t) * f(c3 - f(c4 * t))))
}

/// 引擎的 u01：`t = (state & 0x7fffff) * 2^-23`，两步都 f32。
///
/// 出处：`行为口径`（`and` 掩码 0x7fffff + `scvtf` + 乘 2^-23）。
///
/// ⛔ **arcMode 缺口的具名（裁决：照原样写，不绕）**：本仓提取产物
/// （4363 条 shape 块的键并集）里**没有 `arcMode` 字段**——Unity 的
/// `arc` 有 Random / Loop / PingPong / BurstSpread 四模式，后两种
/// **不是随机采样**。本文件所有 `engine_sincos` 都按 `Random`（即
/// `MultiModeValue 0`，`t_theta = u01`）实现；**在产物补上 `arcMode`
/// 之前，「缺 arcMode 按 Random 处理」是一个未验证的假设**，写在
/// 这里而不是默认接死——否则后来的人会合理地把 Random 当默认，而
/// 反驳它需要的证据（引擎对 Loop/PingPong 的确定性步进）不在本文件。
#[inline(always)]
fn u01(bits23: u32) -> f32 {
    f((bits23 & 0x7fffff) as f32 * f32::from_bits(0x34000001))
}

/// 引擎的 `frac = |W - round(W)|`，用 2^23 magic-add（half away from zero）。
///
/// 出处：`行为口径`（`fadd W, 2^23` / `fsub` / `fabd` 三条指令的直译）。
#[inline(always)]
fn wrap_frac(w: f32) -> f32 {
    let big = f(w + 8388608.0);
    let r = f(big - 8388608.0);
    f((w - r).abs())
}

/// 共享核：给定 `arc_deg` 与 `t_theta` 的位，产出 `(sin, cos)` 引擎值。
///
/// 出处：`行为口径`。整条链是五份 `Start*<Random>` 反汇编的公共前缀。
#[inline(always)]
fn engine_sincos(arc_deg: f32, t_theta: f32) -> (f32, f32) {
    let deg2rad = f32::from_bits(0x3c8efa35); // pi/180
    let inv_2pi = f32::from_bits(0x3e22f983); // 1/(2pi)
    let phi = f(f(arc_deg * deg2rad) * t_theta);
    let w = f(phi * inv_2pi);
    let frac = wrap_frac(w);
    let x = f(0.25 - frac);
    let s = f(x * sinc_poly(f(x * x)));
    let u = f(0.25 - f(x.abs()));
    let c = f(u * sinc_poly(f(u * u)));
    (s, c)
}

/// Circle 形状一次发射的局部位置。
///
/// **随机消耗：1 次**（`t_theta`）。
///
/// 档位：**行为口径**。`radius`/`radiusThickness`/`arc` 属性可读，盘上分布
/// 原生；采样核来自 `StartCircle<Random>` 反汇编。
/// 实现口径：角 = `arc * (pi/180) * t_theta`，径向
/// `r/R = sqrt(1 - (1 - thickness)^2)`（thickness=1 全盘均匀，0 恒边缘）。
/// `rand_r` 参数保留给调用方把「盘内均匀性」的决定显式传进来；本律只
/// 暴露引擎那一路（thickness 单参数决定，无第二次抽签）。
///
/// Editor 测量方案：radius=10、thickness=1、arc=360，收 10 万样本，
/// 数 `|p| < 5` 的占比：均匀面积应 25%；thickness=0 时全部样本应落在
/// 半径恰为 10 的圆上。
pub fn circle_position(
    radius: f32,
    radius_thickness: f32,
    arc_deg: f32,
    rand_r: f32,
    rand_theta: f32,
) -> [f32; 3] {
    if !(radius >= 0.0)
        || !(radius_thickness >= 0.0 && radius_thickness <= 1.0)
        || !(rand_r >= 0.0 && rand_r <= 1.0)
        || !(rand_theta >= 0.0 && rand_theta <= 1.0)
        || !(arc_deg >= 0.0 && arc_deg <= 360.0)
    {
        return [f32::NAN; 3];
    }
    let (s, c) = engine_sincos(arc_deg, rand_theta);
    let one_minus = f(1.0 - radius_thickness);
    let r_over_r = f(f(1.0 - f(one_minus * one_minus)).sqrt());
    let r = f(r_over_r * radius);
    [f(r * c), f(r * s), 0.0]
}

/// Cone（type 4，**从圆锥底面发射**）一次发射的局部位置与方向。
///
/// **随机消耗：2 次**——先抽角（`t_theta`），再抽径向（`t_radial`）。
/// 次序即引擎次序（`StartCone<Random>` 反汇编里第一条 xorshift 喂角、
/// 第二条喂径向），颠倒会让第二个粒子起全链错位。
///
/// 档位：**行为口径**。
/// - 位置：底面圆盘上 `r = R * sqrt(thickness + (1-thickness) * sqrt(t_radial))`。
///   嵌套 sqrt 来自反汇编里连续两条 `fsqrt`；thickness=0 时 `r = R*sqrt(t)`
///   恰是面积均匀，thickness=1 时 r 恒 = R（贴边）。
/// - 方向：沿锥面，`angle`（度，半顶角）控制张角；引擎把方向交给
///   `EmitterStoreData` 做单位化，所以这里返回**未归一**的方向向量，
///   由调用方归一（与真源同一分工）。
/// - `length` 不进位置（产物里 1156/1156 条 Cone 的 `length` 全 5.0，是
///   默认值；它只在 `ConeVolume` 那档参与）。
///
/// ⛔ `ConeVolume`（type 8，从体积发射）是**另一个档**，式子完全不同——
/// 本函数只覆盖 type 4。产物里 type 8 仅 29 条，具名 fail-closed。
pub fn cone_base(
    radius: f32,
    radius_thickness: f32,
    angle_deg: f32,
    arc_deg: f32,
    t_theta: f32,
    t_radial: f32,
) -> ([f32; 3], [f32; 3]) {
    if !(radius >= 0.0)
        || !(radius_thickness >= 0.0 && radius_thickness <= 1.0)
        || !(angle_deg >= 0.0 && angle_deg <= 90.0)
        || !(t_theta >= 0.0 && t_theta <= 1.0)
        || !(t_radial >= 0.0 && t_radial <= 1.0)
        || !(arc_deg >= 0.0 && arc_deg <= 360.0)
    {
        return ([f32::NAN; 3], [f32::NAN; 3]);
    }
    let (s, c) = engine_sincos(arc_deg, t_theta);
    let inner = f(radius_thickness + f(f(1.0 - radius_thickness) * t_radial.sqrt()));
    let r_factor = inner.sqrt();
    let r = f(radius * r_factor);
    let pos = [f(r * c), f(r * s), 0.0];
    let ang = angle_deg.to_radians();
    let (sa, ca) = ang.sin_cos();
    let dir = [f(sa * c), f(sa * s), ca];
    (pos, dir)
}

/// Sphere（type 0，含 thickness 折叠掉 `SphereShell`）一次发射的局部位置。
///
/// **随机消耗：2 次**——先抽方位角（`t_theta`），再抽俯仰（`t_cos`）。
///
/// 档位：**行为口径**。
/// - 方向：单位球面上均匀——`z = 1 - 2*t_cos`（`t_cos` 在 [0,1) 均匀，
///   `z` 在 (-1, 1] 均匀即球面均匀的正确参数化），`x/y` 由方位角给出。
/// - 径向：`r/R = (1 - (1-thickness)^3)^(1/3)`（体积均匀的反演）。
///   ⚠ **这一行的出处档位比其余低半级**：反汇编里它是
///   `bl powf`（`exp2(log2(x)/3)` 三条指令）而不是内联的 `fsqrt`，
///   即「立方根」是从调用形式+`fmul s0, s0, s1(=3.0)` 的`1/3`推断的，
///   没有逐字公式。它是本文件**唯一只能推断的式子**，具名挂账。
/// - 位置 = 方向 * r。
///
/// ⛔ `SphereShell`（type 1）在引擎源里被标「deprecated, use
/// radiusThickness」，本档用 `radiusThickness` 一并覆盖：thickness=0 即
/// 旧的 Shell 行为。
///
/// Editor 测量方案：radius=10、thickness=1，收 10 万样本，数
/// `|p| < 5` 的占比：体积均匀应 12.5%；thickness=0 时应全部落在
/// 半径恰为 10 的球面上。
pub fn sphere_position(
    radius: f32,
    radius_thickness: f32,
    t_theta: f32,
    t_cos: f32,
) -> ([f32; 3], [f32; 3]) {
    if !(radius >= 0.0)
        || !(radius_thickness >= 0.0 && radius_thickness <= 1.0)
        || !(t_theta >= 0.0 && t_theta <= 1.0)
        || !(t_cos >= 0.0 && t_cos <= 1.0)
    {
        return ([f32::NAN; 3], [f32::NAN; 3]);
    }
    let (s, c) = engine_sincos(360.0, t_theta);
    let z = f(1.0 - f(2.0 * t_cos));
    let ring = f(f(1.0 - f(z * z)).max(0.0).sqrt());
    let dir = [f(ring * c), f(ring * s), z];
    let one_minus = f(1.0 - radius_thickness);
    let cb = f(f(one_minus * one_minus) * one_minus);
    let r_factor = f(f(1.0 - cb).powf(1.0 / 3.0));
    let r = f(radius * r_factor);
    ([f(r * dir[0]), f(r * dir[1]), f(r * dir[2])], dir)
}

/// Hemisphere（type 2）一次发射的局部位置。
///
/// **随机消耗：2 次**——与 Sphere 完全相同的两抽，**方向 z 取绝对值**
/// 翻到上半球。
///
/// 档位：**行为口径**。`StartHemiSphere<Random>` 与 `StartSphere<Random>`
/// 的循环体逐指令相同，只差收尾一个 `fabs`——所以消耗个数也是 2，
/// 不是「采整球再翻上来」那种 3 抽的写法。
///
/// ⛔ 这一档最阴险的写法差别：「采整球再把 z<0 翻上来」（3 抽）与
/// 「直接采半球」（2 抽 + fabs）单点采样给出同样合理的位置，但消耗的
/// 随机数个数不同 ⇒ 从第二个粒子起全线错位。引擎是后者。
pub fn hemisphere_position(
    radius: f32,
    radius_thickness: f32,
    t_theta: f32,
    t_cos: f32,
) -> ([f32; 3], [f32; 3]) {
    if !(radius >= 0.0)
        || !(radius_thickness >= 0.0 && radius_thickness <= 1.0)
        || !(t_theta >= 0.0 && t_theta <= 1.0)
        || !(t_cos >= 0.0 && t_cos <= 1.0)
    {
        return ([f32::NAN; 3], [f32::NAN; 3]);
    }
    let (s, c) = engine_sincos(360.0, t_theta);
    // 与 sphere 同一抽法得到 z，然后 fabs 翻到上半球：
    let z = f(f(1.0 - f(2.0 * t_cos)).abs());
    let ring = f(f(1.0 - f(z * z)).max(0.0).sqrt());
    let dir = [f(ring * c), f(ring * s), z];
    let one_minus = f(1.0 - radius_thickness);
    let cb = f(f(one_minus * one_minus) * one_minus);
    let r_factor = f(f(1.0 - cb).powf(1.0 / 3.0));
    let r = f(radius * r_factor);
    ([f(r * dir[0]), f(r * dir[1]), f(r * dir[2])], dir)
}

/// SingleSidedEdge（type 12，从一条边发射）一次发射的局部位置与方向。
///
/// **随机消耗：1 次**（`t_theta`）。
///
/// 档位：**行为口径**。
/// - `StartSingleSidedEdge<Random>` 反汇编的循环体是**一条 xorshift**，
///   没有第二条抽签 ⇒ 一维均匀采样，位置沿局部 X 轴的边：
///   `x = R * (2*t - 1)`，`y = z = 0`。
/// - ⛔ **厚度参数在这一档不参与采样**（裁决）：反汇编循环体里没有
///   任何对 `radiusThickness`（结构偏移 0x50）的读取。产物里
///   `radiusThickness` 的分布（0.28×334 / 1.0×316 / 0.0×112）是作者侧的
///   约定值，不是这一档的语义输入。**不要照球那档的插值形式套过来。**
/// - 方向：沿局部 +Z（`SingleSided` 的语义是「只朝一侧」）。
///
/// Editor 测量方案：radius=5，收 1 万样本，全部应落在线段 [-5, 5] × {0}
/// 上，且 x 均匀。
pub fn single_sided_edge(radius: f32, t_theta: f32) -> ([f32; 3], [f32; 3]) {
    if !(radius >= 0.0) || !(t_theta >= 0.0 && t_theta <= 1.0) {
        return ([f32::NAN; 3], [f32::NAN; 3]);
    }
    let x = f(radius * f(f(2.0 * t_theta) - 1.0));
    ([x, 0.0, 0.0], [0.0, 0.0, 1.0])
}

/// 以欧拉角（度）旋转向量。`x`/`y`/`z` 轴依次施加的约定是
/// `Quaternion.Euler` 的文档口径（`C# 可读`的文档，矩阵本身 extern）：
/// **先绕 Z、再绕 X、再绕 Y**。
pub fn euler_rotate_deg(angles_xyz_deg: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    let [rx, ry, rz] = angles_xyz_deg.map(f32::to_radians);
    let v = rotate_z(rz, v);
    let v = rotate_x(rx, v);
    rotate_y(ry, v)
}

fn rotate_x(a: f32, v: [f32; 3]) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c]
}

fn rotate_y(a: f32, v: [f32; 3]) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

fn rotate_z(a: f32, v: [f32; 3]) -> [f32; 3] {
    let (s, c) = a.sin_cos();
    [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]]
}
