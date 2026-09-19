//! 天气律：一份现象的 `postprocess.json`，经采纳门折算成雾的三个全局
//! 量与六条 uber 后处理轴的本帧取值。
//!
//! 一份档案同时驱动七个量——只取雾的读者也得重解同一文件——所以
//! 解析一次、七个量一起给出：
//!
//! | 量 | 门（对采纳后的值） | 出处 |
//! |---|---|---|
//! | 雾 `fog_params` / `fog_near_color` / `fog_far_color` | `enabled` | `MysekaiFogPass.ExecuteFog` 每帧写的三个全局属性 |
//! | `_BLOOM_LQ` | `brightEnable && intensity > 0.001` | `MysekaiParticleBloomVolume.IsActive` |
//! | `_SKY_DIFFUSION` | `intensity > 0 && scatter > 0` | `MysekaiDiffusionVolume.IsActive` |
//! | `_SCREEN_FLAREPARA` | `isScreenFlareActive && screenFlareIntensity > 0.001` | `MysekaiFlareParaVolume.IsScreenFlareActive` |
//! | `_SUN_FLAREPARA` | `isSunFlareActive && sunFlareIntensity > 0.001` | `MysekaiFlareParaVolume.IsSunFlareActive` |
//! | `_GRAYSCALE` | 不由档案驱动：拍照/截图滤镜的相机捕获桥静态 | 恒关 |
//! | `_DISTORTION` | 天气路径上没有任何写入者 | 恒关 |
//!
//! # 第八个量：引擎原生调色，它不是一条 uber 轴
//!
//! 档案还携带 URP 原生的 `ColorAdjustments`，而它**不走** uber 那六条
//! 关键字。它走引擎自己的两段链——「烘一张调色 LUT」加「uber pass 查
//! 那张表」——而这两段在本管线的现象相机上是**上游**：Mysekai 的后处理
//! 特性挂在 `AfterRenderingPostProcessing`（600），引擎的调色 pass 挂在
//! `BeforeRenderingPostProcessing`（550）；且现象相机用的渲染器自己就
//! 入队了那两个 pass。⇒ 「不在六轴律的轴集里」是真的，「所以不接」是
//! 假的。见 [`ColorAdjustmentsParams`]。
//!
//! 同一族里另有三个组件在实证档案上**恒等**，不构成缺口：`WhiteBalance`
//! （只有一档带、且组件级 active 为假 ⇒ 落构造默认 ⇒ LMS 系数全 1）、
//! URP 原生 `Bloom`（只有一档带、active 亦为假）。`SplitToning` 是例外：
//! 有一档带且 active 为真 ⇒ 那一档的调色比本模块给出的多两次 soft light，
//! 未接，按未做完的活记账。
//!
//! # 采纳门 `overrideState` 是门，不是值
//!
//! 真源的 volume 栈只在参数 `overrideState` 为真时采纳序列化
//! `value`，为假时保留组件构造函数的默认值。实证后果：id 6 RAIN 与
//! id 14 SEKAI 序列化了 `isSunFlareActive value=1` 而 `overrideState=false`
//! ——照 value 读会给它们一条真游戏不显示的太阳光晕。所以本模块的
//! 每个字段都走「门开取序列化值、门关取构造默认」，下面的构造默认
//! 逐项转写自真源四个 volume 组件的构造函数，不是发明。
//!
//! 组件整体缺席不是错误：各档案携带的组件本就不同（014 另带
//! `SplitToning`/`WhiteBalance`，009 带原生 `Bloom`；四个 Mysekai 组件
//! 是闭集），而真源栈为每种组件类型预建了默认实例，行为恰是
//! 「组件缺席 → 构造默认」。组件在而参数缺才是 `Err`（点名它）：
//! 序列化资产总是携带整个组件，缺参数是数据损伤，不是游戏状态，
//! 静默兜底会重建本模块要替换的「接没接线分不清」。
//!
//! ⚠ 有一个字段在源侧**零读者**：`MysekaiParticleBloomVolume` 的
//! `useAreaOverride`。它的偏移在整份泛光 pass 里出现 0 次，而同一个类的
//! 其余十个字段偏移各出现 ≥1 次（10/11 阳性对照）。它序列化值恒 0，
//! 本模块照读进 [`BloomParams`] 但不装箱——「读得出来、源侧没人用」与
//! 「我方漏接」是两回事。
//!
//! # 形状纪律
//!
//! 无 GPU 对象、无文件读取：调用方拿字节来、决定读哪个现象的档案。
//! 找到文件是宿主传参的活，与真源把现象 id 一路传下去取资产名的
//! 形状一致——渲染侧没有从全局读「当前现象 id」的路径。

mod json;

pub mod sky;

#[cfg(test)]
mod corpus;

/// 一个序列化过的 volume 参数：采纳门，加只有门开着才活的值。
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileParameter {
    pub override_state: bool,
    pub value: ProfileValue,
}

/// `postprocess.json` 实际序列化的值形状：数值参数（含 `fixedBufferHeight`
/// 这类整数值）落 [`ProfileValue::Scalar`]，颜色/向量落
/// [`ProfileValue::Vector`]，贴图引用在全部实证档案里序列化为 null。
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileValue {
    Scalar(f32),
    Vector([f32; 4]),
    Null,
}

impl ProfileValue {
    fn parse(v: &json::Value, context: &str) -> Result<Self, String> {
        match v {
            json::Value::Null => Ok(ProfileValue::Null),
            json::Value::Number(n) => Ok(ProfileValue::Scalar(*n as f32)),
            json::Value::Array(items) => {
                if items.len() != 4 {
                    return Err(format!(
                        "{context} has {} components, expected 4",
                        items.len()
                    ));
                }
                let mut out = [0.0f32; 4];
                for (i, item) in items.iter().enumerate() {
                    out[i] = item
                        .as_f64()
                        .ok_or_else(|| format!("{context}[{i}] is not numeric"))?
                        as f32;
                }
                Ok(ProfileValue::Vector(out))
            }
            other => Err(format!("{context} is not a number, 4-vector or null: {other:?}")),
        }
    }
}

/// 档案里的一个 volume 组件，原样：`name`、volume 类名（四个 Mysekai
/// 组件或原生 URP 类）、组件级 `active` 标志、按文件序的参数表。
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeComponent {
    pub name: String,
    pub class: String,
    pub active: bool,
    pub parameters: Vec<(String, ProfileParameter)>,
}

impl VolumeComponent {
    fn parse(v: &json::Value, context: &str) -> Result<Self, String> {
        let name = field(v, "name", context)?
            .as_str()
            .ok_or_else(|| format!("{context}.name is not a string"))?
            .to_string();
        let class = field(v, "class", context)?
            .as_str()
            .ok_or_else(|| format!("{context}.class is not a string"))?
            .to_string();
        let active = field(v, "active", context)?
            .as_bool()
            .ok_or_else(|| format!("{context}.active is not a bool"))?;
        let params_v = field(v, "parameters", context)?;
        let params_obj = params_v
            .as_object()
            .ok_or_else(|| format!("{context}.parameters is not an object"))?;
        let mut parameters = Vec::with_capacity(params_obj.len());
        for (key, value) in params_obj {
            let param_context = format!("{context}.parameters.{key}");
            let override_state = field(value, "overrideState", &param_context)?
                .as_bool()
                .ok_or_else(|| format!("{param_context}.overrideState is not a bool"))?;
            let value = ProfileValue::parse(
                field(value, "value", &param_context)?,
                &format!("{param_context}.value"),
            )?;
            parameters.push((key.clone(), ProfileParameter { override_state, value }));
        }
        Ok(VolumeComponent { name, class, active, parameters })
    }

    fn parameter(&self, key: &str) -> Option<&ProfileParameter> {
        self.parameters
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| p)
    }
}

/// 一份解析完成的 `postprocess.json`：资产名与按文件序的组件表。
#[derive(Debug, Clone, PartialEq)]
pub struct PostProcessProfile {
    pub asset: String,
    pub components: Vec<VolumeComponent>,
}

/// 雾轴的采纳结果：每个字段都过了采纳门，就是真源栈本帧持有的值。
///
/// `enabled` 是采纳后的 `MysekaiFogVolume.enabled`，即 `IsActive` 门
/// 返回的那个值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogVolumeState {
    pub enabled: bool,
    pub density: f32,
    pub near_color: [f32; 4],
    pub near_density: f32,
    pub far_color: [f32; 4],
    pub far_density: f32,
    pub start: f32,
    pub end: f32,
    pub height: f32,
}

/// 真源雾 pass 每帧写出的三个全局量（`fog_params` / `fog_near_color` /
/// `fog_far_color`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogGlobals {
    pub fog_params: [f32; 4],
    pub fog_near_color: [f32; 4],
    pub fog_far_color: [f32; 4],
}

impl FogVolumeState {
    /// `MysekaiFogPass.ExecuteFog` 的逐项转写：
    ///
    /// ```text
    /// disabled = !enabled || IsDisableFog
    /// nearA = nearDensity * density  （disabled 时强写 0.0）
    /// farA  = farDensity  * density  （disabled 时强写 0.0）
    /// fog_params     = (-1/(end-start), end/(end-start), 1/fogHeight, 0)
    /// fog_near_color = (nearColor.rgb, nearA)   // rgb 直通
    /// fog_far_color  = (farColor.rgb,  farA)
    /// ```
    ///
    /// `fog_params.w` 是真源里的代码常量 `0.0`（`SetGlobalColor` 调用
    /// 的第四分量，从来不是 volume 值），这里写死 `0.0`；它有没有被
    /// 如实转写由配套的 w 配对测试看着。
    ///
    /// `disable_fog` 是宿主侧的总开关，对应真源 client-only 的静态门
    /// `IsDisable`/`IsDisableFog`（托管侧零调用点，产品恒开、宿主可达）。
    /// 如实照真源的 disabled 形状：只把两个 alpha 强制为 0.0，rgb 与
    /// params 照写——合成端靠 `height_factor = fog_color.w * falloff = 0`
    /// 精确化简回 `out = scene`。
    pub fn globals(&self, disable_fog: bool) -> FogGlobals {
        let disabled = !self.enabled || disable_fog;
        let near_a = if disabled { 0.0 } else { self.near_density * self.density };
        let far_a = if disabled { 0.0 } else { self.far_density * self.density };
        FogGlobals {
            fog_params: [
                -1.0 / (self.end - self.start),
                self.end / (self.end - self.start),
                1.0 / self.height,
                0.0,
            ],
            fog_near_color: [self.near_color[0], self.near_color[1], self.near_color[2], near_a],
            fog_far_color: [self.far_color[0], self.far_color[1], self.far_color[2], far_a],
        }
    }
}

/// `MysekaiParticleBloomVolume` 的采纳参数——[`AxisState::enabled`] 开着时
/// `_BLOOM_LQ` pass 读的就是它。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomParams {
    pub bright_enable: bool,
    pub threshold: f32,
    pub intensity: f32,
    pub scatter: f32,
    pub tint: [f32; 4],
    pub clamp: f32,
    /// `dirtTexture != null`。实证档案全部序列化 null，今天处处为 false；
    /// 真源 dirt 关键字变体（`_BLOOM_LQ_DIRT`）以它为键。
    pub dirt_texture_present: bool,
    pub dirt_intensity: f32,
    pub fixed_buffer_height: f32,
    pub use_area_override: bool,
    pub overlay_strength: f32,
}

/// `MysekaiDiffusionVolume` 的采纳参数——[`AxisState::enabled`] 开着时
/// `_SKY_DIFFUSION` pass 读的就是它。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffusionParams {
    pub intensity: f32,
    pub scatter: f32,
    pub contrast: f32,
    pub blend_mode: f32,
    pub max_iterations: f32,
    pub buffer_height: f32,
}

/// URP 原生 `ColorAdjustments` 的采纳参数。
///
/// 它**不是** Mysekai uber 那六条关键字轴之一——它走的是引擎自己的
/// 「烘 LUT + uber 查表」两段链，而那两段在本管线的现象相机上是**上游**：
/// Mysekai 的后处理特性挂在 `AfterRenderingPostProcessing` 这个事件上，
/// 引擎的调色 pass 挂在它之前。所以「不在六轴律的轴集里」为真、
/// 「所以不接」为假：档案里 15 档有 14 档的调色是非恒等的。
///
/// 单位照源参数的原始单位存（`contrast`/`saturation` 是百分数、
/// `hueShift` 是度、`postExposure` 是 EV），装箱一跳见
/// [`ColorAdjustmentsParams::pack`]。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorAdjustmentsParams {
    /// EV，`2^x` 是乘在场景色上的线性倍数。
    pub post_exposure: f32,
    /// 百分数，`x / 100 + 1` 是 LogC 域里绕 ACEScc 中灰的对比系数。
    pub contrast: f32,
    /// HDR 颜色参数（通道可 > 1），线性域的无钳制乘子。
    pub color_filter: [f32; 4],
    /// 度，`x / 360` 是 HSV 域的色相偏移。
    pub hue_shift: f32,
    /// 百分数，`x / 100 + 1` 是绕亮度的饱和度系数。
    pub saturation: f32,
}

/// 调色轴装箱后的三个量，逐项对应引擎两段链里的 uber 与 LUT 属性。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorGradingPack {
    /// uber pass 在查表**之前**乘上的曝光倍数。
    pub post_exposure_linear: f32,
    /// LUT 烘焙 pass 的 `_HueSatCon`：`(色相偏移, 饱和系数, 对比系数, 0)`。
    pub hue_sat_con: [f32; 4],
    /// LUT 烘焙 pass 的 `_ColorFilter.rgb`（线性域）。
    pub color_filter_linear: [f32; 3],
}

impl ColorAdjustmentsParams {
    /// 装箱一跳，逐项照引擎两段链自己的式子：
    ///
    /// * `post_exposure_linear` = `2^postExposure`，在 uber pass 里**先于**
    ///   LUT 查表乘上去（源注释写明这是为了「不影响 bloom / dof」）。
    /// * `hue_sat_con` = `(hueShift / 360, saturation / 100 + 1,
    ///   contrast / 100 + 1, 0)`——LUT 烘焙 pass 的打包实参。
    /// * `color_filter_linear` = colorFilter 经 `Mathf.GammaToLinearSpace`
    ///   （HDR 颜色参数的 `.linear`），alpha 不参与。
    #[must_use]
    pub fn pack(&self) -> ColorGradingPack {
        ColorGradingPack {
            post_exposure_linear: 2.0f32.powf(self.post_exposure),
            hue_sat_con: [
                self.hue_shift / 360.0,
                self.saturation / 100.0 + 1.0,
                self.contrast / 100.0 + 1.0,
                0.0,
            ],
            color_filter_linear: [
                gamma_to_linear(self.color_filter[0]),
                gamma_to_linear(self.color_filter[1]),
                gamma_to_linear(self.color_filter[2]),
            ],
        }
    }
}

/// `MysekaiFlareParaVolume` 屏幕光晕半边的采纳参数——
/// [`AxisState::enabled`] 开着时 `_SCREEN_FLAREPARA` pass 读的就是它。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenFlareParams {
    pub intensity: f32,
    pub direction: f32,
    pub color1: [f32; 4],
    pub color2: [f32; 4],
    pub offset1: f32,
    pub offset2: f32,
    pub exponent: f32,
}

/// `MysekaiFlareParaVolume` 太阳光晕半边的采纳参数。太阳的屏幕位置不是
/// volume 值（它是方向光向量经相机逆矩阵的绘制时变换），所以这里没有
/// 那个字段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunFlareParams {
    pub intensity: f32,
    pub color1: [f32; 4],
    pub color2: [f32; 4],
    pub offset1: f32,
    pub offset2: f32,
    pub exponent: f32,
}

/// 一条档案驱动的 uber 轴：本帧的关键字门，加 pass 要读的采纳值。
/// `enabled` 已把组件级 `active` 折进去：组件缺席或不活跃时，门在
/// 构造默认上取值——四个实证组件的默认门全是关。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisState<P> {
    pub enabled: bool,
    pub params: P,
}

/// 本档案不驱动的两条 uber 轴。类型化地立在解析输出里，六轴普查因此
/// 是看得见的而不是靠缺席暗示：
///
/// * `_GRAYSCALE` 由拍照/截图滤镜的相机捕获桥静态驱动，从不由天气
///   档案驱动——天气档案根本不带灰度组件。
/// * `_DISTORTION` 在天气路径上没有任何写入者（镜头畸变参数的唯一
///   写入方是原生 URP 的后处理 pass，而 uber 材质的关键字每帧清零）——恒关。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NotProfileDriven;

impl NotProfileDriven {
    /// 这两条轴在天气路径上的状态。
    pub const ENABLED: bool = false;
}

/// 一份档案驱动的七个量，全部过了采纳门。六轴就落在这里读：接线单
/// 落地时不需要重新解析，读的就是本轮雾接线读的这些结构。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhenomenonPostProcess {
    pub fog: AxisState<FogVolumeState>,
    pub bloom_lq: AxisState<BloomParams>,
    pub sky_diffusion: AxisState<DiffusionParams>,
    pub screen_flarepara: AxisState<ScreenFlareParams>,
    pub sun_flarepara: AxisState<SunFlareParams>,
    /// 引擎原生调色（`ColorAdjustments`）。`enabled` 是源组件
    /// `IsActive()` 的逐项转写：五项里任一项非恒等即开。
    pub color_grading: AxisState<ColorAdjustmentsParams>,
    pub grayscale: NotProfileDriven,
    pub distortion: NotProfileDriven,
}

/// 构造默认，逐项转写自真源四个 volume 组件的构造函数。
/// 「(默认门, 默认值) 成对」是过度建模：序列化资产的 `overrideState`
/// 是逐字段的，本模块唯一施加的默认是门关时的*值*。
mod ctor_defaults {
    use super::{ProfileValue, ScreenFlareParams, SunFlareParams};

    pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
    pub const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

    pub const FOG: super::FogVolumeState = super::FogVolumeState {
        enabled: false,
        density: 0.0,
        near_color: BLACK,
        near_density: 0.0,
        far_color: BLACK,
        far_density: 1.0,
        start: 1.0,
        end: 10.0,
        height: 1.0,
    };

    pub const BLOOM: super::BloomParams = super::BloomParams {
        bright_enable: false,
        threshold: 0.9,
        intensity: 0.0,
        scatter: 0.7,
        tint: WHITE,
        // 构造函数的字面量是 `0x477fc000` = 65472.0（不是 65520）。这一档
        // 在实证档案里可达：有两档把 `clamp` 的采纳门写成关，那两档真游戏
        // 取的就是这个构造默认。
        clamp: 65472.0,
        dirt_texture_present: false,
        dirt_intensity: 0.0,
        fixed_buffer_height: 540.0,
        use_area_override: false,
        overlay_strength: 0.0,
    };

    pub const DIFFUSION: super::DiffusionParams = super::DiffusionParams {
        intensity: 0.0,
        scatter: 0.7,
        contrast: 1.0,
        blend_mode: 6.0,
        max_iterations: 5.0,
        buffer_height: 540.0,
    };

    /// `ColorAdjustments` 的构造默认：全恒等（曝光 0 EV、对比 0%、
    /// 滤色白、色相 0°、饱和 0%）。
    pub const COLOR_ADJUSTMENTS: super::ColorAdjustmentsParams =
        super::ColorAdjustmentsParams {
            post_exposure: 0.0,
            contrast: 0.0,
            color_filter: WHITE,
            hue_shift: 0.0,
            saturation: 0.0,
        };

    pub const SCREEN_FLARE: ScreenFlareParams = ScreenFlareParams {
        intensity: 1.0,
        direction: 45.0,
        color1: WHITE,
        color2: BLACK,
        offset1: 0.0,
        offset2: 0.0,
        exponent: 1.0,
    };

    pub const SUN_FLARE: SunFlareParams = SunFlareParams {
        intensity: 1.0,
        color1: WHITE,
        color2: BLACK,
        offset1: 0.0,
        offset2: 0.0,
        exponent: 1.0,
    };

    /// 门开时参数序列化值应持有的标量形状。
    pub fn scalar_of(value: &ProfileValue, context: &str) -> Result<f32, String> {
        match value {
            ProfileValue::Scalar(s) => Ok(*s),
            other => Err(format!("{context} is not a scalar: {other:?}")),
        }
    }

    pub fn vector_of(value: &ProfileValue, context: &str) -> Result<[f32; 4], String> {
        match value {
            ProfileValue::Vector(v) => Ok(*v),
            other => Err(format!("{context} is not a 4-vector: {other:?}")),
        }
    }

    pub fn flag_of(value: &ProfileValue, context: &str) -> Result<bool, String> {
        match value {
            // 实证档案里序列化旗标是 JSON 数（`0`/`1`），不是 JSON 布尔。
            ProfileValue::Scalar(s) => Ok(*s != 0.0),
            other => Err(format!("{context} is not a 0/1 flag: {other:?}")),
        }
    }
}

use ctor_defaults as cd;

// ---------------------------------------------------------------------------
// 装箱一跳：解析结果 → uber pass 的程序选择与 uniform 块。
//
// 下列式子逐项转写自真源每帧填 uber 材质的 pass 代码，不是从着色器
// 的读法反推的：
//
// * 屏幕光晕：写 `_MysekaiScreenFlareColor`（screenFlareColor1）、
//   `_MysekaiScreenFlareColor2`、`_MysekaiScreenFlareParam1` =
//   (cos d, sin d, offset1, offset2)（d = screenFlareDirection *
//   0.017453292 喂 sincos）、`_MysekaiScreenFlareParam2` =
//   (screenFlareIntensity, screenFlareExponent, 0, 0)。
// * 太阳光晕：同样的四个形状。它的 param1.xy 与 param2.x 内的亮度因子
//   是光和相机的绘制时状态，不是 volume 值——这也是
//   [`SunFlareParams`] 没有方向字段的原因。本侧不解方向光变换，这两处
//   取真源的零方向与因子恒等：param1.xy = (0, 0)、param2.x =
//   sunFlareIntensity。档案携带的其余量（颜色、偏移、指数、强度）按
//   真源自己的式子装箱。
// * bloom：`_Bloom_Enable_States.x` = brightEnable ? 1 : 0；
//   `_Bloom_Params` = (intensity, tint')，tint' 是 tint 经
//   `GammaToLinearSpace` 进线性、再被 `ColorUtils.Luminance` 归一
//   （亮度为零回退白色）；`_Bloom_RGBM` = 0（本侧链路跑在 LDR 8bit
//   帧上，不编 RGBM）；`_BloomOverlayStrength` = overlayStrength。
//   dirt 关键字变体（dirtIntensity > 0 时武装）在天气数据里无对应：
//   实证档案 dirtIntensity 恒 0/null，天气数据只走普通 `_BLOOM_LQ` 分支。
// * 天空扩散：`_SkyDiffusionIntensity` = intensity、
//   `_DiffusionContrast` = contrast、`_DiffusionBlendMode` = blendMode，
//   并武装 `_SKY_DIFFUSION`。
//
// # 为什么 `_GRAYSCALE` 与 `_DISTORTION` 不在这里装箱
//
// 档案为它序列化的每个组件带字段，除 SplitToning/WhiteBalance 外读者
// 也能在某些原生 URP 组件里找到「看起来像灰度/畸变」的数据。两轴不接
// 是有据的裁决不是省略，证据见 [`NotProfileDriven`] 的 doc。没有驱动器
// 就别往 [`UberVariant`] 里添这两位。
// ---------------------------------------------------------------------------

/// 真源光晕装箱在 sincos 前乘的角度系数（`screenFlareDirection *
/// 0.017453292`）——照抄真源的截断字面量而不是算更精确的常数，装箱
/// 结果才能与源值逐值一致。
const DEG_TO_RAD: f32 = 0.017453292;

/// `Mathf.GammaToLinearSpace`：三段，不是两段。低段 `x / 12.92`、
/// 中段 `((x + 0.055) / 1.055)^2.4`、**`x >= 1` 段 `x^2.2`**。
///
/// 第三段不是装饰：调色的 colorFilter 是 HDR 颜色参数，实证档案里有一档
/// 三通道全 > 1.7，两段式会把它算高约 5.6%。bloom 的 threshold 落在
/// `[0, 1]`（钳制参数）、tint 恒白，所以补上第三段对既有两条泛光装箱
/// 逐位无影响——它只在调色这条新轴上出力。
fn gamma_to_linear(x: f32) -> f32 {
    if x <= 0.04045 {
        x / 12.92
    } else if x < 1.0 {
        ((x + 0.055) / 1.055).powf(2.4)
    } else {
        x.powf(2.2)
    }
}

/// `_Bloom_Params.yzw`：线性空间里的 tint，被 `ColorUtils.Luminance`
/// （`r * 0.2126729 + g * 0.7151522 + b * 0.072175`）归一，亮度为零时
/// 回退白色——真源自己的归一或白 guard。
fn normalized_linear_tint(tint: [f32; 4]) -> [f32; 3] {
    let linear = [
        gamma_to_linear(tint[0]),
        gamma_to_linear(tint[1]),
        gamma_to_linear(tint[2]),
    ];
    let luminance =
        linear[0] * 0.2126729 + linear[1] * 0.7151522 + linear[2] * 0.072175;
    if luminance <= 0.0 {
        [1.0, 1.0, 1.0]
    } else {
        [linear[0] / luminance, linear[1] / luminance, linear[2] / luminance]
    }
}

/// uber pass 的六条关键字轴，置位改变其编译结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UberVariant {
    /// `_DISTORTION`。
    pub distortion: bool,
    /// `_BLOOM_LQ`。
    pub bloom: bool,
    /// `_SKY_DIFFUSION`。
    pub sky_diffusion: bool,
    /// `_SCREEN_FLAREPARA`。
    pub screen_flare: bool,
    /// `_SUN_FLAREPARA`。
    pub sun_flare: bool,
    /// `_GRAYSCALE`。
    pub grayscale: bool,
}

/// uber pass 的 uniform 块：逐 lane 对应真源 uber 材质的具名属性，
/// 每个成员一个 16 字节向量（标量挤进具名向量的 lane，见字段 doc），
/// 接线侧的缓冲布局按这些 lane 名对齐。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UberPostParams {
    /// `_MysekaiScreenFlareColor`；`w` 是第一层的权重。
    pub screen_flare_color: [f32; 4],
    /// `_MysekaiScreenFlareColor2`；`w` 是第二层的权重。
    pub screen_flare_color2: [f32; 4],
    /// `_MysekaiScreenFlareParam1`：`xy` 投影轴，`zw` 衰减偏移。
    pub screen_flare_param1: [f32; 4],
    /// `_MysekaiScreenFlareParam2` 的 `xy`：整体强度、衰减指数。
    pub screen_flare_param2: [f32; 4],
    /// `_MysekaiSunFlareColor`。
    pub sun_flare_color: [f32; 4],
    /// `_MysekaiSunFlareColor2`。
    pub sun_flare_color2: [f32; 4],
    /// `_MysekaiSunFlareParam1`。
    pub sun_flare_param1: [f32; 4],
    /// `_MysekaiSunFlareParam2` 的 `xy`。
    pub sun_flare_param2: [f32; 4],
    /// `_GrayscaleColorBalance` 的 `xyz`。
    pub grayscale_color_balance: [f32; 4],
    /// `_Bloom_Params`：`x` 强度，`yzw` tint。
    pub bloom_params: [f32; 4],
    /// `_Bloom_Enable_States` 的 `xy`；本 pass 只读 `x`。
    pub bloom_enable_states: [f32; 4],
    /// `_Distortion_Params1`：`xy` 中心，`zw` 轴。
    pub distortion_params1: [f32; 4],
    /// `_Distortion_Params2`：`x` theta，`y` sigma，`z` scale，`w` 强度。
    pub distortion_params2: [f32; 4],
    /// `_BlitScaleBias`：顶点级的坐标变换，内容归宿主。
    pub blit_scale_bias: [f32; 4],
    /// `x` `_SkyDiffusionIntensity`，`y` `_DiffusionContrast`，
    /// `z` `_Bloom_RGBM`，`w` `_BloomOverlayStrength`。
    pub scalars: [f32; 4],
    /// `_GlobalMipBias` 的 `xy`；只读 `x`。
    pub mip_bias: [f32; 4],
    /// `x` `_DiffusionBlendMode`。真源按具名分支切换，未识别值是
    /// 有据的不动色，不是错误。
    pub modes: [i32; 4],
}

impl Default for UberPostParams {
    /// 全轴中性：未装箱的 lane 停在这些值上。
    fn default() -> Self {
        Self {
            screen_flare_color: [0.0; 4],
            screen_flare_color2: [0.0; 4],
            screen_flare_param1: [0.0; 4],
            screen_flare_param2: [0.0; 4],
            sun_flare_color: [0.0; 4],
            sun_flare_color2: [0.0; 4],
            sun_flare_param1: [0.0; 4],
            sun_flare_param2: [0.0; 4],
            grayscale_color_balance: [1.0, 1.0, 1.0, 0.0],
            bloom_params: [0.0, 1.0, 1.0, 1.0],
            bloom_enable_states: [0.0; 4],
            distortion_params1: [0.0, 0.0, 1.0, 1.0],
            distortion_params2: [0.0, 1.0, 1.0, 0.0],
            blit_scale_bias: [1.0, 1.0, 0.0, 0.0],
            scalars: [0.0; 4],
            mip_bias: [0.0; 4],
            modes: [0; 4],
        }
    }
}

impl PhenomenonPostProcess {
    /// 本帧四条档案驱动轴选出的 uber 程序：每条采纳门一位，
    /// `_GRAYSCALE`/`_DISTORTION` 恒关（证据见 [`NotProfileDriven`]，
    /// 别从档案的其他组件把它们「补回来」）。
    ///
    /// 这就是真源每帧清空 uber 材质关键字、再由各轴的 Render 自开
    /// 的那次逐帧重算：这里什么也不缓存，调用方每帧从当前解析状态
    /// 重新取，档案切换或宿主停用下一帧即生效，无需失效步骤。
    #[must_use]
    pub fn uber_variant(&self) -> UberVariant {
        UberVariant {
            distortion: false,
            bloom: self.bloom_lq.enabled,
            sky_diffusion: self.sky_diffusion.enabled,
            screen_flare: self.screen_flarepara.enabled,
            sun_flare: self.sun_flarepara.enabled,
            grayscale: false,
        }
    }

    /// 本帧的 uber uniform 块，按真源 pass 代码自己的式子从采纳值
    /// 装箱。档案不驱动的 lane 保持 [`UberPostParams::default`] 的中性值。
    #[must_use]
    pub fn uber_post_params(&self) -> UberPostParams {
        let mut p = UberPostParams::default();

        // Bloom：`_Bloom_Params`（x 强度，yzw 亮度归一的线性 tint）、
        // `_Bloom_Enable_States.x`（门折进去的 brightEnable 旗标）、
        // LDR 链上 `_Bloom_RGBM` 为 0、`_BloomOverlayStrength` 进标量 w lane。
        let bloom = &self.bloom_lq.params;
        let tint = normalized_linear_tint(bloom.tint);
        p.bloom_params = [bloom.intensity, tint[0], tint[1], tint[2]];
        p.bloom_enable_states = [f32::from(bloom.bright_enable), 0.0, 0.0, 0.0];
        p.scalars[2] = 0.0;
        p.scalars[3] = bloom.overlay_strength;

        // 天空扩散：强度/对比进标量 x/y lane，混合模式进整数 lane。
        let sky = &self.sky_diffusion.params;
        p.scalars[0] = sky.intensity;
        p.scalars[1] = sky.contrast;
        p.modes[0] = sky.blend_mode as i32;

        // 屏幕光晕：颜色原样（各色自己的 alpha 就是该层在着色器里的
        // 权重），param1 = (cos d, sin d, offset1, offset2)，
        // param2 = (intensity, exponent, 0, 0)。
        let screen = &self.screen_flarepara.params;
        let (sin_d, cos_d) = (screen.direction * DEG_TO_RAD).sin_cos();
        p.screen_flare_color = screen.color1;
        p.screen_flare_color2 = screen.color2;
        p.screen_flare_param1 = [cos_d, sin_d, screen.offset1, screen.offset2];
        p.screen_flare_param2 = [screen.intensity, screen.exponent, 0.0, 0.0];

        // 太阳光晕：同形；param1.xy 是档案之外的绘制时方向（零方向），
        // param2.x 直载采纳强度。
        let sun = &self.sun_flarepara.params;
        p.sun_flare_color = sun.color1;
        p.sun_flare_color2 = sun.color2;
        p.sun_flare_param1 = [0.0, 0.0, sun.offset1, sun.offset2];
        p.sun_flare_param2 = [sun.intensity, sun.exponent, 0.0, 0.0];

        p
    }

    /// bloom 预过滤 pass 的 `_Params` 向量（源 `ParticleBloom.Render` 的
    /// SetVector 实参）：`x` = scatter′ = scatter·0.9+0.05（scatter 为负
    /// 时地板 0.05），`y` = 膜前 clamp，`z` = threshold 的 gamma→linear
    /// （源属性 tooltip 明写 "Value is in gamma-space"），`w` = z·0.5
    /// （软膝宽——SetVector 的第 4 实参就是 `z * 0.5` 字面）。
    #[must_use]
    pub fn bloom_prefilter_params(&self) -> [f32; 4] {
        let bloom = &self.bloom_lq.params;
        let threshold = gamma_to_linear(bloom.threshold);
        [scatter_prime(bloom.scatter), bloom.clamp, threshold, threshold * 0.5]
    }

    /// 扩散上采样 pass 的 `_Scatter`：**不是档案原值**。源 pass 在建金字塔
    /// 之前把 scatter 折成 `scatter * 0.9 + 0.05`（scatter 为负时地板
    /// 0.05），再 `SetFloat` 到扩散材质——与泛光预滤波的 scatter′ 同一个
    /// 式子，两条轴各自算一次。
    ///
    /// 此前这里返回原值、doc 却写着「恰是扩散组件序列化的属性」，而消费侧
    /// 自己又折算了一遍 ⇒ 同一个折算有两份实现、其中一份是错的。折算归律，
    /// 消费侧读这里。
    #[must_use]
    pub fn diffusion_scatter(&self) -> [f32; 4] {
        [scatter_prime(self.sky_diffusion.params.scatter), 0.0, 0.0, 0.0]
    }
}

/// 散射权重的折算，泛光预滤波与扩散上采样共用同一个式子：
/// `scatter * 0.9 + 0.05`，负值取地板 0.05。0 会退化成纯高级别、1 会整层
/// 吞掉高级别，两头都不是现象语义——所以源侧把定义域压进 `[0.05, 0.95]`。
///
/// 公开是因为消费侧要在**交叉淡化之后**折算（淡化插的是档案原值），
/// 而折算只许有一份实现。上界不钳：源侧不钳，档案侧的钳制参数自己保证
/// `[0, 1]`，钳在这里会把数据损伤伪装成正常值。
#[must_use]
pub fn scatter_prime(scatter: f32) -> f32 {
    if scatter < 0.0 {
        0.05
    } else {
        scatter * 0.9 + 0.05
    }
}

fn field<'a>(v: &'a json::Value, key: &str, context: &str) -> Result<&'a json::Value, String> {
    v.get(key).ok_or_else(|| format!("{context} has no `{key}`"))
}

/// 采纳一个标量：门开 → 序列化值（不是标量则 `Err` 点名）；门关 →
/// 组件构造默认；*在场的*组件缺这个参数 → `Err`（序列化资产携带整个
/// 组件，缺参是数据损伤不是游戏状态，见模块 doc）。
fn adopt_scalar(
    component: &VolumeComponent,
    key: &str,
    default: f32,
    context: &str,
) -> Result<f32, String> {
    match component.parameter(key) {
        None => Err(format!("{context} has no `{key}`")),
        Some(param) if param.override_state => {
            cd::scalar_of(&param.value, &format!("{context}.{key}"))
        }
        Some(_) => Ok(default),
    }
}

fn adopt_vector(
    component: &VolumeComponent,
    key: &str,
    default: [f32; 4],
    context: &str,
) -> Result<[f32; 4], String> {
    match component.parameter(key) {
        None => Err(format!("{context} has no `{key}`")),
        Some(param) if param.override_state => {
            cd::vector_of(&param.value, &format!("{context}.{key}"))
        }
        Some(_) => Ok(default),
    }
}

fn adopt_flag(
    component: &VolumeComponent,
    key: &str,
    default: bool,
    context: &str,
) -> Result<bool, String> {
    match component.parameter(key) {
        None => Err(format!("{context} has no `{key}`")),
        Some(param) if param.override_state => {
            cd::flag_of(&param.value, &format!("{context}.{key}"))
        }
        Some(_) => Ok(default),
    }
}

impl PostProcessProfile {
    /// 从原始字节解析一份现象的 `postprocess.json`。
    ///
    /// 用 [`json`]，本 crate 手写的唯一 JSON 读取器，不引入第二个。
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let root = json::parse(bytes).map_err(|e| format!("postprocess.json: {e}"))?;
        let asset = field(&root, "asset", "postprocess.json")?
            .as_str()
            .ok_or_else(|| "postprocess.json.asset is not a string".to_string())?
            .to_string();
        let components_v = field(&root, "components", "postprocess.json")?
            .as_array()
            .ok_or_else(|| "postprocess.json.components is not an array".to_string())?;
        let mut components = Vec::with_capacity(components_v.len());
        for (i, item) in components_v.iter().enumerate() {
            components.push(VolumeComponent::parse(item, &format!("postprocess.json.components[{i}]"))?);
        }
        Ok(PostProcessProfile { asset, components })
    }

    /// 第一个给定类名的 volume 组件，档案不带则 `None`。各档案携带的
    /// 组件不同（模块 doc）；实证档案没有同类重复。
    fn component(&self, class: &str) -> Option<&VolumeComponent> {
        self.components.iter().find(|c| c.class == class)
    }

    /// 七个量全部过采纳门解析。`Err` 只为数据损伤（在场组件缺参数、
    /// 参数形状不对）——档案不携带的组件永远解析到构造默认。
    pub fn resolve(&self) -> Result<PhenomenonPostProcess, String> {
        let fog = self.resolve_fog()?;
        let bloom_lq = self.resolve_bloom()?;
        let sky_diffusion = self.resolve_diffusion()?;
        let (screen_flarepara, sun_flarepara) = self.resolve_flares()?;
        let color_grading = self.resolve_color_adjustments()?;
        Ok(PhenomenonPostProcess {
            fog,
            bloom_lq,
            sky_diffusion,
            screen_flarepara,
            sun_flarepara,
            color_grading,
            grayscale: NotProfileDriven,
            distortion: NotProfileDriven,
        })
    }

    /// 引擎原生调色的采纳。门是源组件 `IsActive()` 的逐项转写：
    /// `postExposure != 0 || contrast != 0 || colorFilter != white ||
    /// hueShift != 0 || saturation != 0`——**逐项相等比较，没有 epsilon**，
    /// 所以照抄成精确比较。
    ///
    /// `colorFilter != Color.white` 比的是四分量（含 alpha）；实证档案
    /// 的 alpha 恒 1，与白的 alpha 相等，所以这里也带上第四分量。
    pub fn resolve_split_toning(&self) -> Result<([f32; 4], [f32; 4]), String> {
        let mut shadows = [0.5, 0.5, 0.5, 0.0];
        let mut highlights = [0.5, 0.5, 0.5, 0.0];
        if let Some(c) = self.component("SplitToning").filter(|c| c.active) {
            shadows = adopt_vector(c, "shadows", [0.5, 0.5, 0.5, 1.0], "SplitToning")?;
            highlights = adopt_vector(c, "highlights", [0.5, 0.5, 0.5, 1.0], "SplitToning")?;
            shadows[3] = adopt_scalar(c, "balance", 0.0, "SplitToning")? / 100.0;
            highlights[3] = if shadows[..3] != [0.5; 3] || highlights[..3] != [0.5; 3] { 1.0 } else { 0.0 };
        }
        Ok((shadows, highlights))
    }

    fn resolve_color_adjustments(
        &self,
    ) -> Result<AxisState<ColorAdjustmentsParams>, String> {
        const CTX: &str = "postprocess.json.ColorAdjustments";
        let params = match self.component("ColorAdjustments") {
            None => cd::COLOR_ADJUSTMENTS,
            Some(c) if !c.active => cd::COLOR_ADJUSTMENTS,
            Some(c) => ColorAdjustmentsParams {
                post_exposure: adopt_scalar(
                    c,
                    "postExposure",
                    cd::COLOR_ADJUSTMENTS.post_exposure,
                    CTX,
                )?,
                contrast: adopt_scalar(c, "contrast", cd::COLOR_ADJUSTMENTS.contrast, CTX)?,
                color_filter: adopt_vector(
                    c,
                    "colorFilter",
                    cd::COLOR_ADJUSTMENTS.color_filter,
                    CTX,
                )?,
                hue_shift: adopt_scalar(c, "hueShift", cd::COLOR_ADJUSTMENTS.hue_shift, CTX)?,
                saturation: adopt_scalar(c, "saturation", cd::COLOR_ADJUSTMENTS.saturation, CTX)?,
            },
        };
        let enabled = params.post_exposure != 0.0
            || params.contrast != 0.0
            || params.color_filter != cd::WHITE
            || params.hue_shift != 0.0
            || params.saturation != 0.0;
        Ok(AxisState { enabled, params })
    }

    fn resolve_fog(&self) -> Result<AxisState<FogVolumeState>, String> {
        const CTX: &str = "postprocess.json.MysekaiFogVolume";
        let state = match self.component("MysekaiFogVolume") {
            None => cd::FOG,
            Some(c) if !c.active => cd::FOG,
            Some(c) => FogVolumeState {
                enabled: adopt_flag(c, "enabled", cd::FOG.enabled, CTX)?,
                density: adopt_scalar(c, "density", cd::FOG.density, CTX)?,
                near_color: adopt_vector(c, "nearColor", cd::FOG.near_color, CTX)?,
                near_density: adopt_scalar(c, "nearDensity", cd::FOG.near_density, CTX)?,
                far_color: adopt_vector(c, "farColor", cd::FOG.far_color, CTX)?,
                far_density: adopt_scalar(c, "farDensity", cd::FOG.far_density, CTX)?,
                start: adopt_scalar(c, "fogStartDistance", cd::FOG.start, CTX)?,
                end: adopt_scalar(c, "fogEndDistance", cd::FOG.end, CTX)?,
                height: adopt_scalar(c, "fogHeight", cd::FOG.height, CTX)?,
            },
        };
        Ok(AxisState { enabled: state.enabled, params: state })
    }

    fn resolve_bloom(&self) -> Result<AxisState<BloomParams>, String> {
        const CTX: &str = "postprocess.json.MysekaiParticleBloomVolume";
        let params = match self.component("MysekaiParticleBloomVolume") {
            None => cd::BLOOM,
            Some(c) if !c.active => cd::BLOOM,
            Some(c) => BloomParams {
                bright_enable: adopt_flag(c, "brightEnable", cd::BLOOM.bright_enable, CTX)?,
                threshold: adopt_scalar(c, "threshold", cd::BLOOM.threshold, CTX)?,
                intensity: adopt_scalar(c, "intensity", cd::BLOOM.intensity, CTX)?,
                scatter: adopt_scalar(c, "scatter", cd::BLOOM.scatter, CTX)?,
                tint: adopt_vector(c, "tint", cd::BLOOM.tint, CTX)?,
                clamp: adopt_scalar(c, "clamp", cd::BLOOM.clamp, CTX)?,
                dirt_texture_present: match c.parameter("dirtTexture") {
                    None => cd::BLOOM.dirt_texture_present,
                    Some(p) if p.override_state => p.value != ProfileValue::Null,
                    Some(_) => cd::BLOOM.dirt_texture_present,
                },
                dirt_intensity: adopt_scalar(c, "dirtIntensity", cd::BLOOM.dirt_intensity, CTX)?,
                fixed_buffer_height: adopt_scalar(
                    c,
                    "fixedBufferHeight",
                    cd::BLOOM.fixed_buffer_height,
                    CTX,
                )?,
                use_area_override: adopt_flag(
                    c,
                    "useAreaOverride",
                    cd::BLOOM.use_area_override,
                    CTX,
                )?,
                overlay_strength: adopt_scalar(
                    c,
                    "overlayStrength",
                    cd::BLOOM.overlay_strength,
                    CTX,
                )?,
            },
        };
        // IsActive 门：brightEnable && intensity > 0.001。
        let enabled = params.bright_enable && params.intensity > 0.001;
        Ok(AxisState { enabled, params })
    }

    fn resolve_diffusion(&self) -> Result<AxisState<DiffusionParams>, String> {
        const CTX: &str = "postprocess.json.MysekaiDiffusionVolume";
        let params = match self.component("MysekaiDiffusionVolume") {
            None => cd::DIFFUSION,
            Some(c) if !c.active => cd::DIFFUSION,
            Some(c) => DiffusionParams {
                intensity: adopt_scalar(c, "intensity", cd::DIFFUSION.intensity, CTX)?,
                scatter: adopt_scalar(c, "scatter", cd::DIFFUSION.scatter, CTX)?,
                contrast: adopt_scalar(c, "contrast", cd::DIFFUSION.contrast, CTX)?,
                blend_mode: adopt_scalar(c, "blendMode", cd::DIFFUSION.blend_mode, CTX)?,
                max_iterations: adopt_scalar(
                    c,
                    "maxIterations",
                    cd::DIFFUSION.max_iterations,
                    CTX,
                )?,
                buffer_height: adopt_scalar(c, "bufferHeight", cd::DIFFUSION.buffer_height, CTX)?,
            },
        };
        // IsActive 门：intensity > 0 && scatter > 0。
        let enabled = params.intensity > 0.0 && params.scatter > 0.0;
        Ok(AxisState { enabled, params })
    }

    fn resolve_flares(
        &self,
    ) -> Result<(AxisState<ScreenFlareParams>, AxisState<SunFlareParams>), String> {
        const CTX: &str = "postprocess.json.MysekaiFlareParaVolume";
        match self.component("MysekaiFlareParaVolume") {
            None => Ok((
                AxisState { enabled: false, params: cd::SCREEN_FLARE },
                AxisState { enabled: false, params: cd::SUN_FLARE },
            )),
            Some(c) if !c.active => Ok((
                AxisState { enabled: false, params: cd::SCREEN_FLARE },
                AxisState { enabled: false, params: cd::SUN_FLARE },
            )),
            Some(c) => {
                let screen = ScreenFlareParams {
                    intensity: adopt_scalar(c, "screenFlareIntensity", cd::SCREEN_FLARE.intensity, CTX)?,
                    direction: adopt_scalar(c, "screenFlareDirection", cd::SCREEN_FLARE.direction, CTX)?,
                    color1: adopt_vector(c, "screenFlareColor1", cd::SCREEN_FLARE.color1, CTX)?,
                    color2: adopt_vector(c, "screenFlareColor2", cd::SCREEN_FLARE.color2, CTX)?,
                    offset1: adopt_scalar(c, "screenFlareOffset1", cd::SCREEN_FLARE.offset1, CTX)?,
                    offset2: adopt_scalar(c, "screenFlareOffset2", cd::SCREEN_FLARE.offset2, CTX)?,
                    exponent: adopt_scalar(c, "screenFlareExponent", cd::SCREEN_FLARE.exponent, CTX)?,
                };
                let sun = SunFlareParams {
                    intensity: adopt_scalar(c, "sunFlareIntensity", cd::SUN_FLARE.intensity, CTX)?,
                    color1: adopt_vector(c, "sunFlareColor1", cd::SUN_FLARE.color1, CTX)?,
                    color2: adopt_vector(c, "sunFlareColor2", cd::SUN_FLARE.color2, CTX)?,
                    offset1: adopt_scalar(c, "sunFlareOffset1", cd::SUN_FLARE.offset1, CTX)?,
                    offset2: adopt_scalar(c, "sunFlareOffset2", cd::SUN_FLARE.offset2, CTX)?,
                    exponent: adopt_scalar(c, "sunFlareExponent", cd::SUN_FLARE.exponent, CTX)?,
                };
                // 两个门：采纳后的旗标 + 采纳后强度的 > 0.001 阈。旗标是
                // 采纳门的招牌案例：id 6/id 14 序列化 value=1 而
                // overrideState=false，这里必须不放行。
                let is_screen = adopt_flag(c, "isScreenFlareActive", false, CTX)?;
                let is_sun = adopt_flag(c, "isSunFlareActive", false, CTX)?;
                let screen_enabled = is_screen && screen.intensity > 0.001;
                let sun_enabled = is_sun && sun.intensity > 0.001;
                Ok((
                    AxisState { enabled: screen_enabled, params: screen },
                    AxisState { enabled: sun_enabled, params: sun },
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手搭的最小档案：只有雾组件，全部字段 overrideState=true，
    /// 载 014_sekai 的序列化雾值。
    fn sekai_profile_bytes() -> Vec<u8> {
        br#"{
            "asset": "postprocess_014_sekai.asset",
            "components": [
                {
                    "name": "MysekaiFogVolume",
                    "class": "MysekaiFogVolume",
                    "active": true,
                    "parameters": {
                        "enabled": {"overrideState": true, "value": 1},
                        "density": {"overrideState": true, "value": 1.0},
                        "nearColor": {"overrideState": true, "value": [0.641064465045929, 0.8585600256919861, 0.9245283007621765, 0.0]},
                        "nearDensity": {"overrideState": true, "value": 0.6499999761581421},
                        "farColor": {"overrideState": true, "value": [0.1745283007621765, 0.5407149791717529, 1.0, 0.0]},
                        "farDensity": {"overrideState": true, "value": 0.28999999165534973},
                        "fogStartDistance": {"overrideState": true, "value": 15.0},
                        "fogEndDistance": {"overrideState": true, "value": 30.0},
                        "fogHeight": {"overrideState": true, "value": 1.5}
                    }
                }
            ]
        }"#.to_vec()
    }

    /// 上面字节的手算期望 globals：直接从 ExecuteFog 的转写与写入
    /// 字节的序列化值展开——不经实现的任何管道。
    fn sekai_expected_globals() -> FogGlobals {
        let density = 1.0f32;
        let near_color = [0.64106447f32, 0.85856f32, 0.9245283f32, 0.0f32];
        let near_density = 0.65f32;
        let far_color = [0.1745283f32, 0.54071498f32, 1.0f32, 0.0f32];
        let far_density = 0.29f32;
        let (start, end, height) = (15.0f32, 30.0f32, 1.5f32);
        FogGlobals {
            fog_params: [
                -1.0 / (end - start),
                end / (end - start),
                1.0 / height,
                0.0,
            ],
            fog_near_color: [
                near_color[0],
                near_color[1],
                near_color[2],
                near_density * density,
            ],
            fog_far_color: [
                far_color[0],
                far_color[1],
                far_color[2],
                far_density * density,
            ],
        }
    }

    /// 解析出的雾状态逐字段回圈序列化值，`globals()` 与手算的
    /// ExecuteFog 输出位位相等（两边同序同 f32 运算，精确 `==` 才是
    /// 诚实的比较子）。
    #[test]
    fn fog_globals_match_hand_expansion_of_execute_fog() {
        let profile = PostProcessProfile::from_bytes(&sekai_profile_bytes())
            .expect("hand-built profile must parse");
        let resolved = profile.resolve().expect("hand-built profile must resolve");
        assert!(resolved.fog.enabled);
        let fog = &resolved.fog.params;
        assert_eq!(fog.density, 1.0);
        assert_eq!(fog.near_density, 0.65);
        assert_eq!(fog.far_density, 0.29);
        assert_eq!(fog.start, 15.0);
        assert_eq!(fog.end, 30.0);
        assert_eq!(fog.height, 1.5);

        let got = fog.globals(false);
        let expected = sekai_expected_globals();
        assert_eq!(got, expected, "globals() must equal the hand expansion");
    }

    /// `fog_params.w` 是代码常量 0.0 的配对判定：解析输出须与手算
    /// 展开精确相等，且 w 期望一旦换成任何非零常数比较子必须变红。
    /// 两个方向都断言，`w == 0.0` 的检查才有判别力。
    #[test]
    fn fog_params_w_zero_is_a_paired_judgment() {
        let profile = PostProcessProfile::from_bytes(&sekai_profile_bytes())
            .expect("hand-built profile must parse");
        let got = profile.resolve().unwrap().fog.params.globals(false).fog_params;

        let mut wrong_w = sekai_expected_globals().fog_params;
        wrong_w[3] = 0.5;
        assert_ne!(
            got, wrong_w,
            "the comparator must go red on a non-zero w -- if this fires, \
             the equality above proves nothing about w"
        );
        assert_eq!(got[3], 0.0, "fog_params.w is the code constant 0.0");
    }

    /// 采纳门是门不是值。同字节配对双臂：`isSunFlareActive` 的
    /// `overrideState=false`（id 6 / id 14 的形状——序列化值是 1！）
    /// 时太阳轴必须 OFF；只翻这一个 overrideState 位为 true 必须变 ON。
    /// 照 value 读的读者过不了第一臂；无视 value 的读者过不了第二臂。
    #[test]
    fn sun_flare_gate_reads_override_state_not_value() {
        let template = |override_state: bool| -> Vec<u8> {
            format!(
                r#"{{
                    "asset": "a",
                    "components": [
                        {{"name": "MysekaiFlareParaVolume", "class": "MysekaiFlareParaVolume", "active": true,
                         "parameters": {{
                            "isScreenFlareActive": {{"overrideState": true, "value": 1}},
                            "screenFlareIntensity": {{"overrideState": true, "value": 1.0}},
                            "screenFlareDirection": {{"overrideState": true, "value": 45.0}},
                            "screenFlareColor1": {{"overrideState": true, "value": [1,1,1,1]}},
                            "screenFlareColor2": {{"overrideState": true, "value": [0,0,0,1]}},
                            "screenFlareOffset1": {{"overrideState": true, "value": 0.0}},
                            "screenFlareOffset2": {{"overrideState": true, "value": 0.0}},
                            "screenFlareExponent": {{"overrideState": true, "value": 2.5}},
                            "isSunFlareActive": {{"overrideState": {override_state}, "value": 1}},
                            "sunFlareIntensity": {{"overrideState": true, "value": 2.0}},
                            "sunFlareColor1": {{"overrideState": true, "value": [1,1,1,1]}},
                            "sunFlareColor2": {{"overrideState": true, "value": [0,0,0,1]}},
                            "sunFlareOffset1": {{"overrideState": true, "value": 0.0}},
                            "sunFlareOffset2": {{"overrideState": true, "value": 0.0}},
                            "sunFlareExponent": {{"overrideState": true, "value": 1.5}}
                         }}}},
                        {{"name": "MysekaiFogVolume", "class": "MysekaiFogVolume", "active": true,
                         "parameters": {{
                            "enabled": {{"overrideState": true, "value": 1}},
                            "density": {{"overrideState": true, "value": 1.0}},
                            "nearColor": {{"overrideState": true, "value": [1,1,1,1]}},
                            "nearDensity": {{"overrideState": true, "value": 1.0}},
                            "farColor": {{"overrideState": true, "value": [0,0,0,1]}},
                            "farDensity": {{"overrideState": true, "value": 1.0}},
                            "fogStartDistance": {{"overrideState": true, "value": 1.0}},
                            "fogEndDistance": {{"overrideState": true, "value": 2.0}},
                            "fogHeight": {{"overrideState": true, "value": 1.0}}
                         }}}}
                    ]
                }}"#
            )
            .into_bytes()
        };

        let off = PostProcessProfile::from_bytes(&template(false))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(
            !off.sun_flarepara.enabled,
            "value=1 but overrideState=false: the stack keeps the ctor default false, so the axis is off"
        );
        assert_eq!(
            off.sun_flarepara.params.intensity, 2.0,
            "sunFlareIntensity IS adopted here (overrideState=true), so its serialized value is live"
        );

        let on = PostProcessProfile::from_bytes(&template(true))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(
            on.sun_flarepara.enabled,
            "same bytes with overrideState=true: the value 1 is adopted and the axis is on"
        );
    }

    /// 门关之下序列化的强度必须解析到构造默认而不是序列化数——与门
    /// 同一条采纳规则用在值字段上（sunFlareIntensity 构造默认 1.0）。
    #[test]
    fn closed_gate_value_fields_resolve_to_ctor_defaults() {
        let src = br#"{
            "asset": "a",
            "components": [
                {"name": "MysekaiFlareParaVolume", "class": "MysekaiFlareParaVolume", "active": true,
                 "parameters": {
                    "isScreenFlareActive": {"overrideState": false, "value": 0},
                    "screenFlareIntensity": {"overrideState": false, "value": 0.0},
                    "screenFlareDirection": {"overrideState": false, "value": 10.0},
                    "screenFlareColor1": {"overrideState": false, "value": [0.1, 0.1, 0.1, 0.1]},
                    "screenFlareColor2": {"overrideState": false, "value": [0.1, 0.1, 0.1, 0.1]},
                    "screenFlareOffset1": {"overrideState": false, "value": 1.0},
                    "screenFlareOffset2": {"overrideState": false, "value": 1.0},
                    "screenFlareExponent": {"overrideState": false, "value": 9.0},
                    "isSunFlareActive": {"overrideState": false, "value": 1},
                    "sunFlareIntensity": {"overrideState": false, "value": 2.0},
                    "sunFlareColor1": {"overrideState": false, "value": [0.1, 0.2, 0.3, 0.4]},
                    "sunFlareColor2": {"overrideState": false, "value": [0.1, 0.2, 0.3, 0.4]},
                    "sunFlareOffset1": {"overrideState": false, "value": 1.0},
                    "sunFlareOffset2": {"overrideState": false, "value": 1.0},
                    "sunFlareExponent": {"overrideState": false, "value": 9.0}
                 }}
            ]
        }"#;
        let resolved = PostProcessProfile::from_bytes(src)
            .unwrap()
            .resolve()
            .unwrap();
        assert!(!resolved.sun_flarepara.enabled);
        assert_eq!(
            resolved.sun_flarepara.params.intensity, 1.0,
            "overrideState=false: ctor default 1.0, not the serialized 2.0"
        );
        assert_eq!(
            resolved.sun_flarepara.params.color1, [1.0, 1.0, 1.0, 1.0],
            "overrideState=false: ctor default white, not the serialized color"
        );
    }

    /// 档案缺席的组件解析到构造默认（真源栈预建默认实例的语义），
    /// 四个实证组件的默认门全 OFF。在场组件*缺参数*是 `Err` 且点名它
    /// ——数据损伤，不是游戏状态。
    #[test]
    fn absent_component_resolves_to_ctor_defaults_but_absent_parameter_is_an_error() {
        let src = br#"{"asset": "a", "components": []}"#;
        let resolved = PostProcessProfile::from_bytes(src)
            .unwrap()
            .resolve()
            .unwrap();
        assert!(!resolved.fog.enabled);
        assert!(!resolved.bloom_lq.enabled);
        assert!(!resolved.sky_diffusion.enabled);
        assert!(!resolved.screen_flarepara.enabled);
        assert!(!resolved.sun_flarepara.enabled);
        // 构造默认雾数：s=1, e=10, h=1——转写式子由它得 (-1/9, 10/9, 1, 0)。
        assert_eq!(resolved.fog.params.start, 1.0);
        assert_eq!(resolved.fog.params.end, 10.0);
        assert_eq!(resolved.fog.params.height, 1.0);
        let g = resolved.fog.params.globals(false);
        assert_eq!(g.fog_params, [-1.0 / 9.0, 10.0 / 9.0, 1.0, 0.0]);
        assert_eq!(g.fog_near_color[3], 0.0, "ctor enabled=false => disabled => alpha 0");

        let damaged = br#"{
            "asset": "a",
            "components": [
                {"name": "MysekaiFogVolume", "class": "MysekaiFogVolume", "active": true,
                 "parameters": {
                    "enabled": {"overrideState": true, "value": 1}
                 }}
            ]
        }"#;
        let err = PostProcessProfile::from_bytes(damaged)
            .unwrap()
            .resolve()
            .expect_err("a present component missing a parameter must be an Err");
        assert!(err.contains("density"), "error must name the missing field, got: {err}");
    }

    /// 屏幕光晕门要旗标与强度阈值同时成立：id 1 SUNNY 的形状
    /// （旗标采纳 1、强度 0.0）必须 OFF，只把强度翻过 0.001 必须 ON。
    #[test]
    fn screen_flare_gate_is_flag_and_threshold() {
        let template = |intensity: &str| -> Vec<u8> {
            format!(
                r#"{{
                    "asset": "a",
                    "components": [
                        {{"name": "MysekaiFlareParaVolume", "class": "MysekaiFlareParaVolume", "active": true,
                         "parameters": {{
                            "isScreenFlareActive": {{"overrideState": true, "value": 1}},
                            "screenFlareIntensity": {{"overrideState": true, "value": {intensity}}},
                            "screenFlareDirection": {{"overrideState": true, "value": 45.0}},
                            "screenFlareColor1": {{"overrideState": true, "value": [1,1,1,1]}},
                            "screenFlareColor2": {{"overrideState": true, "value": [0,0,0,1]}},
                            "screenFlareOffset1": {{"overrideState": true, "value": 0.0}},
                            "screenFlareOffset2": {{"overrideState": true, "value": 0.0}},
                            "screenFlareExponent": {{"overrideState": true, "value": 2.5}},
                            "isSunFlareActive": {{"overrideState": false, "value": 0}},
                            "sunFlareIntensity": {{"overrideState": true, "value": 1.0}},
                            "sunFlareColor1": {{"overrideState": true, "value": [1,1,1,1]}},
                            "sunFlareColor2": {{"overrideState": true, "value": [0,0,0,1]}},
                            "sunFlareOffset1": {{"overrideState": true, "value": 0.0}},
                            "sunFlareOffset2": {{"overrideState": true, "value": 0.0}},
                            "sunFlareExponent": {{"overrideState": true, "value": 1.0}}
                         }}}}
                    ]
                }}"#
            )
            .into_bytes()
        };
        let off = PostProcessProfile::from_bytes(&template("0.0"))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(!off.screen_flarepara.enabled, "intensity 0.0 does not pass the 0.001 threshold");
        let on = PostProcessProfile::from_bytes(&template("1.0"))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(on.screen_flarepara.enabled);
    }

    /// 不活跃的组件即使序列化门全部满足也不得点亮轴（真源栈整个
    /// 无视不活跃组件，保持构造默认，默认门全关）。
    #[test]
    fn inactive_component_resolves_off() {
        let src = br#"{
            "asset": "a",
            "components": [
                {"name": "MysekaiFogVolume", "class": "MysekaiFogVolume", "active": false,
                 "parameters": {
                    "enabled": {"overrideState": true, "value": 1},
                    "density": {"overrideState": true, "value": 1.0},
                    "nearColor": {"overrideState": true, "value": [1,1,1,1]},
                    "nearDensity": {"overrideState": true, "value": 1.0},
                    "farColor": {"overrideState": true, "value": [0,0,0,1]},
                    "farDensity": {"overrideState": true, "value": 1.0},
                    "fogStartDistance": {"overrideState": true, "value": 1.0},
                    "fogEndDistance": {"overrideState": true, "value": 2.0},
                    "fogHeight": {"overrideState": true, "value": 1.0}
                 }}
            ]
        }"#;
        let resolved = PostProcessProfile::from_bytes(src)
            .unwrap()
            .resolve()
            .unwrap();
        assert!(!resolved.fog.enabled, "active=false: the stack holds ctor defaults => gate off");
        assert_eq!(
            resolved.fog.params.start, 1.0,
            "and the numbers are the ctor defaults, not the serialized ones"
        );
    }

    /// 两条非档案轴必须以类型化成员出现在解析输出里（六轴普查看得见），
    /// 状态是文档化的常量。
    #[test]
    fn non_profile_axes_resolve_typed_and_off() {
        let resolved = PostProcessProfile::from_bytes(&sekai_profile_bytes())
            .unwrap()
            .resolve()
            .unwrap();
        assert_eq!(resolved.grayscale, NotProfileDriven);
        assert_eq!(resolved.distortion, NotProfileDriven);
        assert!(!NotProfileDriven::ENABLED);
    }

    /// 宿主停用开关的端到端形状：`disable_fog=true` 只把两个 alpha
    /// 强制为 0（rgb 与 params 照写真值），两个 alpha=0 的全局在合成端
    /// 精确化简回恒等。
    #[test]
    fn disable_fog_forces_only_alphas_to_zero() {
        let profile = PostProcessProfile::from_bytes(&sekai_profile_bytes()).unwrap();
        let fog = profile.resolve().unwrap().fog.params;
        let live = fog.globals(false);
        let disabled = fog.globals(true);
        assert_eq!(disabled.fog_params, live.fog_params, "params are written either way");
        for c in 0..3 {
            assert_eq!(disabled.fog_near_color[c], live.fog_near_color[c]);
            assert_eq!(disabled.fog_far_color[c], live.fog_far_color[c]);
        }
        assert_eq!(live.fog_near_color[3], 0.65);
        assert_eq!(disabled.fog_near_color[3], 0.0);
        assert_eq!(disabled.fog_far_color[3], 0.0);
    }

    // -------------------------------------------------------------------
    // 装箱判定的取值半边：每条档案驱动轴的装箱 lane 必须等于源 pass
    // 装箱式子在序列化输入上的手算展开，此处写成字面量——绝不在别的
    // 数据上调实现自己的 helper 来生成期望，式子错了不可能靠构造与
    // 期望一致。两边同序同 f32 运算，精确 `==` 是诚实的比较子。每轴
    // 还带自己的采纳门负臂：同样的序列化值、轴驱动旗标 overrideState
    // 为假时，轴必须 OFF 且装箱落构造默认——id 6 / id 14 太阳光晕的
    // 形状，逐轴各来一遍。
    // -------------------------------------------------------------------

    /// `Mathf.GammaToLinearSpace`（sRGB 分段）的手算展开：此处拼写而
    /// 不导入，装箱测试的期望因此与实现不共 helper。
    fn hand_gamma_to_linear(x: f32) -> f32 {
        if x <= 0.04045f32 {
            x / 12.92f32
        } else {
            ((x + 0.055f32) / 1.055f32).powf(2.4f32)
        }
    }

    /// 真源 bloom tint 装箱的手算展开：进线性，再用 `ColorUtils.
    /// Luminance` 的精确常数亮度归一，亮度为零回白。
    fn hand_bloom_tint(tint: [f32; 4]) -> [f32; 3] {
        let l = [
            hand_gamma_to_linear(tint[0]),
            hand_gamma_to_linear(tint[1]),
            hand_gamma_to_linear(tint[2]),
        ];
        let lum = l[0] * 0.2126729f32 + l[1] * 0.7151522f32 + l[2] * 0.072175f32;
        if lum <= 0.0 {
            [1.0, 1.0, 1.0]
        } else {
            [l[0] / lum, l[1] / lum, l[2] / lum]
        }
    }

    /// bloom 组件全值字段序列化（全 overrideState=true），取无零无幺的
    /// 值：threshold 0.3（非构造 0.9）、intensity 1.5（非 0——门是
    /// brightEnable && intensity > 0.001，故 ON）、tint 非灰、clamp 0.8、
    /// overlay 0.5。
    fn bloom_profile(override_bright_enable: bool) -> Vec<u8> {
        format!(
            r#"{{
                "asset": "a",
                "components": [
                    {{"name": "MysekaiParticleBloomVolume", "class": "MysekaiParticleBloomVolume", "active": true,
                     "parameters": {{
                        "brightEnable": {{"overrideState": {override_bright_enable}, "value": 1}},
                        "threshold": {{"overrideState": true, "value": 0.3}},
                        "intensity": {{"overrideState": true, "value": 1.5}},
                        "scatter": {{"overrideState": true, "value": 0.4}},
                        "tint": {{"overrideState": true, "value": [0.3, 0.6, 1.0, 1.0]}},
                        "clamp": {{"overrideState": true, "value": 0.8}},
                        "dirtTexture": {{"overrideState": false, "value": null}},
                        "dirtIntensity": {{"overrideState": true, "value": 0.0}},
                        "fixedBufferHeight": {{"overrideState": true, "value": 540.0}},
                        "useAreaOverride": {{"overrideState": true, "value": 0}},
                        "overlayStrength": {{"overrideState": true, "value": 0.5}}
                     }}}}
                ]
            }}"#
        )
        .into_bytes()
    }

    /// bloom 装箱 lane 逐 lane 等于手算展开：`_Bloom_Params`（强度、
    /// 亮度归一线性 tint）、enable states、RGBM 0、overlay——位也开。
    #[test]
    fn bloom_packing_matches_hand_expansion_lane_for_lane() {
        let resolved = PostProcessProfile::from_bytes(&bloom_profile(true))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(resolved.bloom_lq.enabled, "brightEnable 1 and intensity 1.5: the gate is open");
        let variant = resolved.uber_variant();
        assert!(variant.bloom, "the open gate arms the bloom keyword");
        assert!(!variant.grayscale && !variant.distortion, "the two non-profile axes stay off");

        let p = resolved.uber_post_params();
        let tint = hand_bloom_tint([0.3, 0.6, 1.0, 1.0]);
        assert_eq!(
            p.bloom_params,
            [1.5f32, tint[0], tint[1], tint[2]],
            "_Bloom_Params = (intensity, luminance-normalized linear tint)"
        );
        // 手算 tint 须非平凡：线性化动每个通道、归一除以非幺亮度——
        // 跳过任一步的实现过不了上一条断言；这条断言防止比较子在
        // 全幺 tint 上空转。
        assert!(
            (tint[2] - 1.0).abs() > 0.5,
            "the blue channel must land far from its gamma-space value for the comparator to mean anything: {tint:?}"
        );
        assert_eq!(p.bloom_enable_states[0], 1.0, "brightEnable true packs 1.0");
        assert_eq!(p.scalars[2], 0.0, "_Bloom_RGBM: the LDR chain packs 0");
        assert_eq!(p.scalars[3], 0.5, "_BloomOverlayStrength");
        // 链路侧：预过滤 `_Params` 四 lane 逐 lane 手算（字面量不用实现
        // 的常数）。scatter 0.4 → x = 0.4·0.9+0.05 = 0.41；clamp → y 0.8；
        // threshold 0.3 是 gamma 值 → z = g2l(0.3) ≈ 0.0732；w = z·0.5
        // ≈ 0.0366（源 SetVector 第 4 实参 = z*0.5）。
        let prefilter = resolved.bloom_prefilter_params();
        assert_eq!(prefilter[0], 0.41, "scatter' = scatter*0.9+0.05 -> _Params.x");
        assert_eq!(prefilter[1], 0.8, "clamp -> _Params.y (pre-knee clamp)");
        assert!(
            (prefilter[2] - 0.0732).abs() < 1e-3,
            "gamma-space threshold 0.3 -> linear _Params.z: {}",
            prefilter[2]
        );
        assert!(
            (prefilter[3] - prefilter[2] * 0.5).abs() < 1e-6,
            "knee width = z*0.5 -> _Params.w: {}",
            prefilter[3]
        );
    }

    /// bloom 侧的采纳门：`brightEnable value=1` 而 `overrideState=false`
    /// 必须解析 OFF（构造默认 false）。门是逐字段的：自己的门开着的
    /// intensity 照样被采纳、照样装箱——真源的材质也会带着它——只是
    /// 位关时装箱块不被读，帧由位保持不变。
    #[test]
    fn bloom_gate_negative_arm_is_off_with_value_fields_still_adopted() {
        let resolved = PostProcessProfile::from_bytes(&bloom_profile(false))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(!resolved.bloom_lq.enabled, "the serialized brightEnable was never adopted");
        assert!(!resolved.uber_variant().bloom);
        let p = resolved.uber_post_params();
        assert_eq!(
            p.bloom_params[0], 1.5,
            "intensity's own gate is open, so the serialized 1.5 is live even though the axis is off"
        );
        assert_eq!(
            p.bloom_enable_states[0], 0.0,
            "brightEnable resolves to its ctor default false and packs 0.0"
        );
        assert_eq!(p.scalars[3], 0.5, "overlay strength likewise adopted");
        let prefilter = resolved.bloom_prefilter_params();
        assert!(
            (prefilter[2] - 0.0732).abs() < 1e-3,
            "threshold's own gate is open: live (gamma 0.3 -> linear {})",
            prefilter[2]
        );
    }

    /// 天空扩散：强度/对比/混合模式 lane 手核，混合模式取各数字段都
    /// 不同（18）的值，两条标量 lane 与整数 lane 的串位会显形。门负臂：
    /// intensity 门关序列化 → 构造 0.0 → OFF。
    #[test]
    fn sky_diffusion_packing_matches_hand_expansion_and_gate() {
        let on = br#"{
            "asset": "a",
            "components": [
                {"name": "MysekaiDiffusionVolume", "class": "MysekaiDiffusionVolume", "active": true,
                 "parameters": {
                    "intensity": {"overrideState": true, "value": 0.8},
                    "scatter": {"overrideState": true, "value": 0.7},
                    "contrast": {"overrideState": true, "value": 1.5},
                    "blendMode": {"overrideState": true, "value": 18},
                    "maxIterations": {"overrideState": true, "value": 5.0},
                    "bufferHeight": {"overrideState": true, "value": 540.0}
                 }}
            ]
        }"#;
        let resolved = PostProcessProfile::from_bytes(on).unwrap().resolve().unwrap();
        assert!(resolved.sky_diffusion.enabled, "intensity 0.8 > 0 and scatter 0.7 > 0");
        assert!(resolved.uber_variant().sky_diffusion);
        let p = resolved.uber_post_params();
        assert_eq!(p.scalars[0], 0.8, "_SkyDiffusionIntensity");
        assert_eq!(p.scalars[1], 1.5, "_DiffusionContrast");
        assert_eq!(p.modes[0], 18, "_DiffusionBlendMode");
        assert_eq!(p.scalars[2], 0.0, "the diffusion packing must not touch the bloom RGBM lane");
        // _Scatter 是**折算后**的值（scatter * 0.9 + 0.05），不是档案原值：
        // 真源在建金字塔之前 SetFloat 的是折算结果。原值与折算值同时钉住，
        // 改动其中任何一个都会红。
        assert_eq!(resolved.diffusion_scatter()[0], 0.7 * 0.9 + 0.05, "_Scatter = scatter*0.9+0.05");

        // 门臂：同一个强度数，未标记。门要 intensity > 0，未采纳的
        // 强度解析到构造 0.0，轴在 scatter 0.7 之下仍 OFF。
        let gated = br#"{
            "asset": "a",
            "components": [
                {"name": "MysekaiDiffusionVolume", "class": "MysekaiDiffusionVolume", "active": true,
                 "parameters": {
                    "intensity": {"overrideState": false, "value": 0.8},
                    "scatter": {"overrideState": true, "value": 0.7},
                    "contrast": {"overrideState": false, "value": 1.5},
                    "blendMode": {"overrideState": false, "value": 18},
                    "maxIterations": {"overrideState": false, "value": 5.0},
                    "bufferHeight": {"overrideState": false, "value": 540.0}
                 }}
            ]
        }"#;
        let resolved = PostProcessProfile::from_bytes(gated).unwrap().resolve().unwrap();
        assert!(!resolved.sky_diffusion.enabled);
        assert!(!resolved.uber_variant().sky_diffusion);
        assert_eq!(resolved.uber_post_params().scalars[0], 0.0, "ctor intensity, not 0.8");
        assert_eq!(resolved.uber_post_params().scalars[1], 1.0, "ctor contrast, not 1.5");
    }

    /// 屏幕光晕：方向的角度系数与 cos/sin lane、颜色原样（alpha 是层
    /// 权重）、偏移以异号进 param1.zw 防 swap、强度/指数进 param2。
    /// 门负臂：`isScreenFlareActive value=1` 而 `overrideState=false`
    /// ——轴 OFF，装箱 lane 落采纳值（各自门仍开）。
    #[test]
    fn screen_flare_packing_matches_hand_expansion_and_gate() {
        let template = |flare_flag: &str| -> Vec<u8> {
            format!(
                r#"{{
                    "asset": "a",
                    "components": [
                        {{"name": "MysekaiFlareParaVolume", "class": "MysekaiFlareParaVolume", "active": true,
                         "parameters": {{
                            "isScreenFlareActive": {{"overrideState": {flare_flag}, "value": 1}},
                            "screenFlareIntensity": {{"overrideState": true, "value": 1.2}},
                            "screenFlareDirection": {{"overrideState": true, "value": 60.0}},
                            "screenFlareColor1": {{"overrideState": true, "value": [0.9, 0.25, 0.6, 0.7]}},
                            "screenFlareColor2": {{"overrideState": true, "value": [0.2, 0.8, 0.45, 0.9]}},
                            "screenFlareOffset1": {{"overrideState": true, "value": -1.5}},
                            "screenFlareOffset2": {{"overrideState": true, "value": 0.5}},
                            "screenFlareExponent": {{"overrideState": true, "value": 1.8}},
                            "isSunFlareActive": {{"overrideState": false, "value": 0}},
                            "sunFlareIntensity": {{"overrideState": true, "value": 1.0}},
                            "sunFlareColor1": {{"overrideState": true, "value": [1,1,1,1]}},
                            "sunFlareColor2": {{"overrideState": true, "value": [0,0,0,1]}},
                            "sunFlareOffset1": {{"overrideState": true, "value": 0.0}},
                            "sunFlareOffset2": {{"overrideState": true, "value": 0.0}},
                            "sunFlareExponent": {{"overrideState": true, "value": 1.0}}
                         }}}}
                    ]
                }}"#
            )
            .into_bytes()
        };
        let resolved = PostProcessProfile::from_bytes(&template("true"))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(resolved.screen_flarepara.enabled);
        assert!(resolved.uber_variant().screen_flare);
        let p = resolved.uber_post_params();
        // 真源自己的截断角度系数，喂 sincos。
        let radians = 60.0f32 * 0.017_453_292f32;
        assert_eq!(p.screen_flare_param1[0], radians.cos(), "param1.x = cos(direction)");
        assert_eq!(p.screen_flare_param1[1], radians.sin(), "param1.y = sin(direction)");
        assert!(
            (p.screen_flare_param1[0] - p.screen_flare_param1[1]).abs() > 0.3,
            "60 degrees chosen so cos != sin: a swapped xy pair fails above"
        );
        assert_eq!(p.screen_flare_param1[2], -1.5, "offset1 into param1.z");
        assert_eq!(p.screen_flare_param1[3], 0.5, "offset2 into param1.w");
        assert_eq!(p.screen_flare_param2, [1.2f32, 1.8f32, 0.0, 0.0], "(intensity, exponent)");
        assert_eq!(p.screen_flare_color, [0.9f32, 0.25f32, 0.6f32, 0.7f32], "color1 verbatim");
        assert_eq!(p.screen_flare_color2, [0.2f32, 0.8f32, 0.45f32, 0.9f32], "color2 verbatim");

        // 门臂：序列化值 1、未标记——id 6 / id 14 的形状。轴 OFF，
        // 自己门开着的字段照旧采纳与装箱（逐字段采纳）：帧由 off 位
        // 保持，不靠清空 lane。
        let resolved = PostProcessProfile::from_bytes(&template("false"))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(!resolved.screen_flarepara.enabled);
        assert!(!resolved.uber_variant().screen_flare);
        let p = resolved.uber_post_params();
        assert_eq!(
            p.screen_flare_param2[0], 1.2,
            "screenFlareIntensity's own gate is open: the serialized 1.2 is live"
        );
        assert_eq!(p.screen_flare_param2[1], 1.8, "exponent likewise adopted");
        assert_eq!(p.screen_flare_param1[0], radians.cos(), "direction likewise adopted");
        assert_eq!(p.screen_flare_param1[2], -1.5, "offset1 likewise adopted");
        assert_eq!(
            p.screen_flare_color,
            [0.9f32, 0.25f32, 0.6f32, 0.7f32],
            "color1 likewise adopted"
        );
    }

    /// 太阳光晕：同四形；param1.xy 是零方向（光导投影轴不是档案值），
    /// 装箱 lane 来自档案。门负臂：两份真档案（id 6/id 14）序列化的
    /// `isSunFlareActive value=1` / `overrideState=false` 形状，逐轴再核。
    #[test]
    fn sun_flare_packing_matches_hand_expansion_and_gate() {
        let template = |sun_flag: &str| -> Vec<u8> {
            format!(
                r#"{{
                    "asset": "a",
                    "components": [
                        {{"name": "MysekaiFlareParaVolume", "class": "MysekaiFlareParaVolume", "active": true,
                         "parameters": {{
                            "isScreenFlareActive": {{"overrideState": false, "value": 0}},
                            "screenFlareIntensity": {{"overrideState": true, "value": 1.0}},
                            "screenFlareDirection": {{"overrideState": true, "value": 45.0}},
                            "screenFlareColor1": {{"overrideState": true, "value": [1,1,1,1]}},
                            "screenFlareColor2": {{"overrideState": true, "value": [0,0,0,1]}},
                            "screenFlareOffset1": {{"overrideState": true, "value": 0.0}},
                            "screenFlareOffset2": {{"overrideState": true, "value": 0.0}},
                            "screenFlareExponent": {{"overrideState": true, "value": 1.0}},
                            "isSunFlareActive": {{"overrideState": {sun_flag}, "value": 1}},
                            "sunFlareIntensity": {{"overrideState": true, "value": 1.4}},
                            "sunFlareColor1": {{"overrideState": true, "value": [0.85, 0.6, 0.2, 0.8]}},
                            "sunFlareColor2": {{"overrideState": true, "value": [0.5, 0.3, 0.1, 0.6]}},
                            "sunFlareOffset1": {{"overrideState": true, "value": -0.2}},
                            "sunFlareOffset2": {{"overrideState": true, "value": 0.35}},
                            "sunFlareExponent": {{"overrideState": true, "value": 1.6}}
                         }}}}
                    ]
                }}"#
            )
            .into_bytes()
        };
        let resolved = PostProcessProfile::from_bytes(&template("true"))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(resolved.sun_flarepara.enabled);
        assert!(resolved.uber_variant().sun_flare);
        let p = resolved.uber_post_params();
        assert_eq!(
            p.sun_flare_param1,
            [0.0f32, 0.0f32, -0.2f32, 0.35f32],
            "zero direction (not a profile value), profile offsets in zw"
        );
        assert_eq!(p.sun_flare_param2, [1.4f32, 1.6f32, 0.0, 0.0], "(intensity, exponent)");
        assert_eq!(p.sun_flare_color, [0.85f32, 0.6f32, 0.2f32, 0.8f32]);
        assert_eq!(p.sun_flare_color2, [0.5f32, 0.3f32, 0.1f32, 0.6f32]);
        // 屏幕半边不得被太阳装箱碰过。
        assert_eq!(p.screen_flare_param2[0], 1.0, "the screen half's own serialized 1.0");

        // 门臂：正是 id 6（RAIN）与 id 14（SEKAI）序列化的形状——值 1、
        // overrideState=false。太阳轴 OFF；自己门开着的字段照旧采纳并
        // 装箱，帧保护由 off 位承担。
        let resolved = PostProcessProfile::from_bytes(&template("false"))
            .unwrap()
            .resolve()
            .unwrap();
        assert!(!resolved.sun_flarepara.enabled);
        assert!(!resolved.uber_variant().sun_flare);
        let p = resolved.uber_post_params();
        assert_eq!(
            p.sun_flare_param2[0], 1.4,
            "sunFlareIntensity's own gate is open: the serialized 1.4 is live"
        );
        assert_eq!(p.sun_flare_param1[2], -0.2, "offset1 likewise adopted");
        assert_eq!(
            p.sun_flare_color,
            [0.85f32, 0.6f32, 0.2f32, 0.8f32],
            "color1 likewise adopted"
        );
    }

    /// 装箱判定的判别半边：门全开时装箱块须在每个轴自己的 lane 上
    /// 不同于全中性默认——什么都不写的装箱会把每帧留在接线前状态，
    /// 上面的帧级判定就成了空测。（位本身由各轴门断言覆盖。）
    #[test]
    fn fully_open_profile_packs_lanes_that_differ_from_neutral() {
        let src = br#"{
            "asset": "a",
            "components": [
                {"name": "MysekaiParticleBloomVolume", "class": "MysekaiParticleBloomVolume", "active": true,
                 "parameters": {
                    "brightEnable": {"overrideState": true, "value": 1},
                    "threshold": {"overrideState": true, "value": 0.3},
                    "intensity": {"overrideState": true, "value": 1.5},
                    "scatter": {"overrideState": true, "value": 0.4},
                    "tint": {"overrideState": true, "value": [0.3, 0.6, 1.0, 1.0]},
                    "clamp": {"overrideState": true, "value": 0.8},
                    "dirtTexture": {"overrideState": false, "value": null},
                    "dirtIntensity": {"overrideState": true, "value": 0.0},
                    "fixedBufferHeight": {"overrideState": true, "value": 540.0},
                    "useAreaOverride": {"overrideState": true, "value": 0},
                    "overlayStrength": {"overrideState": true, "value": 0.5}
                 }},
                {"name": "MysekaiDiffusionVolume", "class": "MysekaiDiffusionVolume", "active": true,
                 "parameters": {
                    "intensity": {"overrideState": true, "value": 0.8},
                    "scatter": {"overrideState": true, "value": 0.7},
                    "contrast": {"overrideState": true, "value": 1.5},
                    "blendMode": {"overrideState": true, "value": 18},
                    "maxIterations": {"overrideState": true, "value": 5.0},
                    "bufferHeight": {"overrideState": true, "value": 540.0}
                 }},
                {"name": "MysekaiFlareParaVolume", "class": "MysekaiFlareParaVolume", "active": true,
                 "parameters": {
                    "isScreenFlareActive": {"overrideState": true, "value": 1},
                    "screenFlareIntensity": {"overrideState": true, "value": 1.2},
                    "screenFlareDirection": {"overrideState": true, "value": 60.0},
                    "screenFlareColor1": {"overrideState": true, "value": [0.9, 0.25, 0.6, 0.7]},
                    "screenFlareColor2": {"overrideState": true, "value": [0.2, 0.8, 0.45, 0.9]},
                    "screenFlareOffset1": {"overrideState": true, "value": -1.5},
                    "screenFlareOffset2": {"overrideState": true, "value": 0.5},
                    "screenFlareExponent": {"overrideState": true, "value": 1.8},
                    "isSunFlareActive": {"overrideState": true, "value": 1},
                    "sunFlareIntensity": {"overrideState": true, "value": 1.4},
                    "sunFlareColor1": {"overrideState": true, "value": [0.85, 0.6, 0.2, 0.8]},
                    "sunFlareColor2": {"overrideState": true, "value": [0.5, 0.3, 0.1, 0.6]},
                    "sunFlareOffset1": {"overrideState": true, "value": -0.2},
                    "sunFlareOffset2": {"overrideState": true, "value": 0.35},
                    "sunFlareExponent": {"overrideState": true, "value": 1.6}
                 }}
            ]
        }"#;
        let resolved = PostProcessProfile::from_bytes(src).unwrap().resolve().unwrap();
        let p = resolved.uber_post_params();
        let d = UberPostParams::default();
        assert_ne!(p.bloom_params, d.bloom_params, "bloom lanes carry the profile");
        assert_ne!(p.scalars, d.scalars, "sky intensity/contrast + bloom overlay live here");
        assert_ne!(p.modes, d.modes, "blend mode lane carries the profile");
        assert_ne!(p.screen_flare_param1, d.screen_flare_param1);
        assert_ne!(p.screen_flare_param2, d.screen_flare_param2);
        assert_ne!(p.screen_flare_color, d.screen_flare_color);
        assert_ne!(p.sun_flare_param1, d.sun_flare_param1);
        assert_ne!(p.sun_flare_param2, d.sun_flare_param2);
        assert_ne!(p.sun_flare_color, d.sun_flare_color);
        // 未触及的 lane 停在中性值——装箱恰好写档案驱动的 lane，别无其他。
        assert_eq!(p.distortion_params1, d.distortion_params1);
        assert_eq!(p.distortion_params2, d.distortion_params2);
        assert_eq!(p.grayscale_color_balance, d.grayscale_color_balance);
        assert_eq!(p.blit_scale_bias, d.blit_scale_bias);
        assert_eq!(p.mip_bias, d.mip_bias);
    }
}

pub mod timeline;
