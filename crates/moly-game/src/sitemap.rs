//! 小地图：ScreenLayerMysekaiSiteMap 的上屏（独立 2D 相机层，与气泡层
//! 平行的覆盖层）。
//!
//! 数据面（提取产物，`moly://` 资产根下）：
//! - `site/sitemap/prefab/prefab.json`——六根的 RectTransform（92 实例）
//!   与 CustomRawImage（21 实例）；
//! - `site/sitemap/animation/animation.json`——clip_open_cloud 的 32 条
//!   曲线全键值（开云动画逐键驱动，键值直读不发明）；
//! - `site/sitemap/texture/texture.json`——贴图清单与色彩空间声明（UI
//!   一律取 sRGB=True 变体）；
//! - `site/sitemap/screen_layer/screen_layer.json`——屏幕层布局真值（APK
//!   player data 的 ScreenLayer prefab，`Resources.Load` 装载的那棵树）：
//!   图标×位置的下标配对、底图/面板的 RectTransform、图标 prefab 内件
//!   与面下色；
//! - `site/sites.json`——站点主表（id · name · isBase · sitePosition）。
//!
//! 真源律（反编译）与本模块的对应：
//! - 图标列：`_siteMapIcons[i]` 运行时放到 `_siteMapIconPositions[i]`——
//!   **下标配对是唯一的连接律**（按名字或 id 接都会错位）；实例根的
//!   authored 缩放是真值（运行时只写位置不写缩放）。HOME 恒列表第 0 位
//!   （`HOME_SITE_ICON_INDEX = 0`）。
//! - 点击已解锁站点（`SiteMapIcon.OnClickButtonFeedback` →
//!   `ScreenLayerMysekaiSiteMap.OnPushSiteMapIcon`）：点的就是当前站 →
//!   `ScreenManager.BackUIScreen`（关地图）；点别的站 →
//!   `MysekaiUtility.ChangeSite` 换站——接 [`crate::site`] 的换站请求
//!   通道；庆典庭院先问生日派对主表，无派对则弹一钮对话框——mock 无
//!   派对数据，具名日志占位、不放行。
//! - 图标三件装载（`SetImage`）：`img_map_site_{id}` · `img_map_site_line`
//!   · `img_map_shadow`，全部从 sitemap 贴图包装载；显示尺寸直读屏幕层
//!   真值（按钮件 home 842×664、其余 522×420；线 466×420；影 352×208
//!   @ (0,-162)，tint 深蓝灰）。
//! - 解锁判定（rank release 名单 + 服务端进度）是服务端域——mock 通则：
//!   支持集扩到主表全量后给满（地图上的站全部解锁，见
//!   [`site_unlocked`]）；锁定臂原样保留，mock 收紧即回用。
//! - 开云时间轴（`OpenSiteAsync`）：云簇收起 → 图标图 3.0s 淡入
//!   （ease 3 = OutSine）→ 动画触发（32 曲线，clip 2.2s）→ 3.5s 处
//!   名字淡入 1.5s（同 OutSine）且 open_cloud 立即关闭 → 起浮动。
//! - 入场（`PlayInAnimation`，逐图标串行）：y+80 滑回 0.5s + 整组淡入
//!   0.5s；真源 ease 是 serialized AnimationCurve（未提取），mock 取
//!   OutQuad。入场回调接浮动（`PlayFloatingAnimation`：y 0→30，3.0s，
//!   无限 yoyo）。
//! - 天气面板（`SiteMapPhenomenaView`）：屏幕层真值把面板钉在画布顶中
//!   （锚 (0.5,1)，pivot 同，@ (0,8)，512×128）；现象档名不在主表（如
//!   庆典园地档）时不更新——真源同形。主表本身是服务端下发数据，mock
//!   成 [`crate::sitemap_phenomena::PHENOMENA_ROWS`]。屏层里另有独立的
//!   天气按钮件（`_weatherButton`，开现象选择屏）——选择屏不在本单范
//!   围，不上屏（挂账）。
//!
//! 具名挂账（本模块不实现，报账里重列）：
//! - UIBlur 背景与 UIBlurLayerManager（BlurSamplingDistance 1.5）——
//!   后处理域；
//! - 每朵云一个 UIParticle 发射器（prefab 共 22 处）——粒子域；
//! - `OnPlaySE("se_unlock_site")`——只记日志，音频归音频域；
//! - 解锁话题气球（UIPartsReleaseSiteBalloon，355×96 @ (-2,234)）与
//!   「现在位置」标记（_hereIconImage，156×72——贴图不在 sitemap 贴图
//!   包，不发明装载源）；
//! - ScreenLayer 的地形高亮（img_map_shadow 的第二消费方，1760×1040 @
//!   (0,668)）与背景渐变族——rect 已在屏幕层产物里，绘制不在本层。

use bevy::asset::{AssetPath, LoadState, RenderAssetUsages};
use bevy::camera::visibility::RenderLayers;
use bevy::image::Image;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use moly_assets::json::JsonAsset;
use std::collections::HashMap;

use crate::balloon::{self, CANVAS_REF_H, CANVAS_REF_W};
use crate::sitemap_phenomena::{DEFAULT_PHENOMENA_ID, PHENOMENA_ROWS};
use crate::source_curve::Curve;
use crate::weather::CurrentPhenomenon;

// ---------------------------------------------------------------------------
// 常量（真源锚在注释里）
// ---------------------------------------------------------------------------

/// 真源常量：默认站点 id（主表类的 `DEFAULT_SITE_ID = 1`）——HOME。
const DEFAULT_SITE_ID: i32 = 1;

/// 真源常量：HOME 图标在屏幕层图标列表的序号（`HOME_SITE_ICON_INDEX = 0`）。
const HOME_SITE_ICON_INDEX: usize = 0;

/// 小地图独占渲染层：与气泡层（1）隔离——各自的相机互不采到对方，
/// 地图整体显隐只动本层。场地屏外壳的离开确认框借本层相机画（Dialog
/// 槽要盖过外壳与小地图）。
pub(crate) const SITEMAP_LAYER: usize = 2;

// 层内没有再往下的 fit/居中系数：屏幕层被运行时拉伸到 1920×1080 的参照
// 画布上（ScreenManager.SetUpScreenResolution 的匹配律 = 气泡层同款
// [`balloon::canvas_scale`]），底图/图标/面板全部按 canvas 单位直读直用。

/// 入场滑动（`PlayInAnimation`：localPosition y+80 起，0.5s 滑回）。
const IN_SLIDE_PX: f32 = 80.0;
const IN_SECONDS: f32 = 0.5;
/// 入场串行步距：真源逐图标 await 前一个入场完成再播下一个（异步状态
/// 机串行），等价于按入场时长步进。
const IN_STAGGER: f32 = IN_SECONDS;

/// 图标浮动（`PlayFloatingAnimation`：DOLocalMove (0,30,0) 3.0s，无限
/// yoyo）。ease 是 serialized AnimationCurve（未提取），mock 取线性
/// 往返。
const FLOAT_AMPLITUDE_PX: f32 = 30.0;
const FLOAT_SECONDS: f32 = 3.0;

/// 开云时间轴（`OpenSiteAsync`，真源值）：按钮（图标图）淡入 3.0s
/// （ease 3 = OutSine）；3.5s 处名字淡入 1.5s（同 OutSine）且
/// open_cloud 立即 SetActive(0)；序列尾重起浮动。
const OPEN_FADE_SECONDS: f32 = 3.0;
const OPEN_WAIT_SECONDS: f32 = 3.5;
const NAME_FADE_SECONDS: f32 = 1.5;
/// 开云 clip 的时间轴上限（stopTime）。
const CLIP_SECONDS: f32 = 2.2;
/// SE 事件（clip 的 AnimationEvent：0.7s OnPlaySE "se_unlock_site"）——
/// 只记日志，音频归音频域。
const SE_EVENT_SECONDS: f32 = 0.7;
/// 开云曲线采样日志的周期（验收要求每朵云的曲线采样值现算可推导）。
const UNLOCK_SAMPLE_PERIOD: f32 = 0.5;

/// 文本字号（canvas 单位）。屏幕层真值给出文本框（640×46 / 300×44 /
/// 256×32）与行位；CustomTextMesh 的字号不在产物里，取框高内可读值
/// （具名的近似）。
const SITE_NAME_SIZE: f32 = 36.0;
const PHENOMENA_JP_SIZE: f32 = 32.0;
const PHENOMENA_EN_SIZE: f32 = 24.0;
/// 现象 JP 行里图标与文本的间距（真源 HorizontalLayoutGroup 的 spacing
/// 是序列化值未提取；组居中律按「图标+文本」合成宽现算）。
const PHENOMENA_ICON_GAP_PX: f32 = 8.0;
/// 文本色（真源未提取，取深色可读色）。
const TEXT_COLOR: Color = Color::srgb(0.16, 0.13, 0.11);

/// 字形图集烘制参数（与气泡层同范式：仓内开源字体、两遍法按实际墨迹
/// 定格）。字符集只有百余字，1024 边长足够。
const BAKE_PPEM: f32 = 32.0;
const CELL_PAD: f32 = 2.0;
const ATLAS_SIZE: f32 = 1024.0;
/// 仓内开源字体（与气泡同款，授权文本同目录进仓）。
const FONT_BYTES: &[u8] = include_bytes!("../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");

// ---------------------------------------------------------------------------
// 数据面：装载请求 → 解析
// ---------------------------------------------------------------------------

/// 五份 JSON 的装载请求；解析成功后即撤。
#[derive(Resource)]
pub(crate) struct SitemapRequest {
    prefab: Handle<JsonAsset>,
    animation: Handle<JsonAsset>,
    texture: Handle<JsonAsset>,
    screen: Handle<JsonAsset>,
    sites: Handle<JsonAsset>,
}

fn json_path(rel: &str) -> AssetPath<'static> {
    AssetPath::from(format!("moly://{rel}"))
}

/// Startup：发装载请求。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(SitemapRequest {
        prefab: server.load::<JsonAsset>(json_path("site/sitemap/prefab/prefab.json")),
        animation: server.load::<JsonAsset>(json_path("site/sitemap/animation/animation.json")),
        texture: server.load::<JsonAsset>(json_path("site/sitemap/texture/texture.json")),
        screen: server
            .load::<JsonAsset>(json_path("site/sitemap/screen_layer/screen_layer.json")),
        sites: server.load::<JsonAsset>(json_path("site/sites.json")),
    });
}

/// 一个节点的布局（RectTransform 实例的字段子集；提取产物里锚点与
/// pivot 全部 (0.5,0.5)，中心对齐）。
#[derive(Clone, Debug)]
struct NodeLayout {
    /// m_AnchoredPosition（相对父，y 向上）。
    anchored: Vec2,
    /// m_SizeDelta（显示尺寸）。
    size: Vec2,
    /// m_LocalScale。
    scale: Vec3,
    /// m_LocalRotation（四元数；本数据里翻转全部经负 scale 承载）。
    rotation: Quat,
}

/// CustomRawImage 实例的字段子集（全部实例的 m_UVRect 都是 (0,0,1,1)，
/// 整图采样，无裁剪）。
#[derive(Clone, Debug)]
struct RawImage {
    texture: String,
    color: [f32; 4],
}

// ---------------------------------------------------------------------------
// 屏幕层布局真值（screen_layer.json 的消费面）
// ---------------------------------------------------------------------------

/// 一个 anchor/pivot 语义下的放置：中心在父系里的位置 + 显示尺寸。
/// 换算式 = Unity UI：锚点 = 父矩形按锚比例取的点，anchoredPosition 从
/// 锚点量到 pivot，中心 = 锚点 + anchoredPosition + (0.5−pivot)×size。
#[derive(Clone, Copy, Debug)]
struct Placed {
    center: Vec2,
    size: Vec2,
}

/// 产物里一条 rect 的字段子集。
#[derive(Clone, Copy)]
struct Frame {
    anchored: Vec2,
    size: Vec2,
    anchor: Vec2,
    pivot: Vec2,
    scale: f32,
}

impl Frame {
    fn parse(value: &serde_json::Value, what: &str) -> Frame {
        // 产物里向量是数组 [x, y(, z)]（提取侧的约定），这里按下标取。
        let v2 = |key: &str| -> Vec2 {
            let f = &value[key];
            let at = |i: usize, part: &str| -> f32 {
                f[i].as_f64()
                    .unwrap_or_else(|| panic!("sitemap：{what} 的 {key}[{i}]（{part}）缺值")) as f32
            };
            Vec2::new(at(0, "x"), at(1, "y"))
        };
        let scale = value["localScale"][0]
            .as_f64()
            .unwrap_or_else(|| panic!("sitemap：{what} 缺 localScale")) as f32;
        Frame {
            anchored: v2("anchoredPosition"),
            size: v2("sizeDelta"),
            anchor: v2("anchorsMin"),
            pivot: v2("pivot"),
            scale,
        }
    }

    /// 在父矩形（中心 `parent_center`、尺寸 `parent_size`）里落位。
    fn place(self, parent_center: Vec2, parent_size: Vec2) -> Placed {
        let anchor_point = parent_center + (self.anchor - Vec2::splat(0.5)) * parent_size;
        Placed {
            center: anchor_point + self.anchored + (Vec2::splat(0.5) - self.pivot) * self.size,
            size: self.size,
        }
    }
}

/// 一个站点图标的屏幕层真值（图标列表与位置列表按下标配对后的行）。
#[derive(Clone)]
struct IconSpot {
    /// 主表的 siteType 名（join 键，接名字不接 id——契约明令）。
    site_type: String,
    /// 位置件在 Content 系里的中心（图标根运行时被写到这里）。
    position: Vec2,
    /// 实例根 authored 缩放（运行时只改位置，缩放原样生效）。
    root_scale: f32,
    /// 按钮件显示尺寸（=点击命中域；home 842×664、其余 522×420）。
    button: Vec2,
    /// 图标根是否为庆典庭院（点击真源的生日派对门）。
    festival: bool,
}

/// 天气面板（SiteMapPhenomenaView）的真值位：面板系中心 + 行中心。
struct PhenomenaLayout {
    /// 面板背景（bg_site_info 装载件）= 面板系原点。
    background: Placed,
    /// 现象图标槽（44×44 真值；行内横排位置在铺装时按组宽现算）。
    icon_size: Vec2,
    /// 两行文本的中心（面板系；JP 行含图标横排，EN 行居中）。
    jp_row: Vec2,
    en_row: Vec2,
}

/// 图标 prefab 内件（outdoor 变体；home 只有按钮尺寸与名字行不同——
/// 这两件走 [`IconSpot::button`] 与按站点判别的名字行位）。
struct PrefabLayout {
    line: Placed,
    shadow: Placed,
    /// img_map_shadow 的装载 tint（真源 CustomImage 的 m_Color）。
    shadow_tint: [f32; 4],
    name_outdoor: Vec2,
    name_home: Vec2,
}

/// 屏幕层解析结果：地图件与两块 UI 板的最终画布位。
struct ScreenLayout {
    spots: Vec<IconSpot>,
    ground: Placed,
    phenomena: PhenomenaLayout,
    prefab: PrefabLayout,
}

/// 一条曲线绑定（节点路径 + 属性名；属性集在解析时校验，未知即拒）。
struct CurveBinding {
    node: String,
    attribute: String,
    curve: Curve,
}

/// 站点主表行（sites.json 的 sites 行）。
struct SiteRow {
    id: i32,
    name: String,
    /// 站点类型名（真源枚举名；与屏幕层图标行的 join 键）。
    site_type: String,
    /// 是否上小地图：以「图标贴图 img_map_site_{id} 在贴图清单里」为准
    /// （楼层站点 2/3/4 是 HOME 内部结构，图标族没有它们的贴图）。
    on_map: bool,
}

/// 解析后的全部数据 + 贴图句柄（按装载名索引；每个名字取 sRGB=True
/// 变体——清单里同名恰两份，色彩空间声明是提取侧从源纹理记下的事实）。
#[derive(Resource)]
pub(crate) struct SitemapData {
    /// (根名, 相对根的节点路径) → 布局。
    layout: HashMap<(String, String), NodeLayout>,
    /// (根名, 节点路径) → 贴图名与色。
    raw_images: HashMap<(String, String), RawImage>,
    curves: Vec<CurveBinding>,
    /// (节点, 属性字符串) → 曲线下标（铺装时绑定实体用）。
    curve_index: HashMap<(String, String), usize>,
    sites: Vec<SiteRow>,
    screen: ScreenLayout,
    textures: HashMap<String, Handle<Image>>,
    /// 贴图装载是否已全部到位（铺装前的闩）。
    textures_pending: bool,
}

impl SitemapData {
    /// `sites` 下标 → 图标真值行（缺位即拒：上地图的站必须有屏层行）。
    fn spot(&self, site_i: usize) -> &IconSpot {
        let site_type = &self.sites[site_i].site_type;
        self.screen
            .spots
            .iter()
            .find(|spot| spot.site_type == *site_type)
            .unwrap_or_else(|| panic!("sitemap：屏幕层没有站点 {site_type} 的图标行"))
    }
}

/// Update：五份 JSON 到齐即解析。缺键/缺根/未知曲线属性按路径具名
/// panic（fail-closed：本数据是提取产物，形状坏了该响亮失败）。
pub(crate) fn parse(
    mut commands: Commands,
    request: Option<ResMut<SitemapRequest>>,
    jsons: Res<Assets<JsonAsset>>,
    server: Res<AssetServer>,
) {
    let Some(request) = request else { return };
    let handles = [
        &request.prefab,
        &request.animation,
        &request.texture,
        &request.screen,
        &request.sites,
    ];
    for handle in handles {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("sitemap：五份数据之一装载失败（{handle:?}）：{err:?}");
        }
    }
    if !handles
        .iter()
        .all(|h| matches!(server.load_state(*h), LoadState::Loaded))
    {
        return;
    }
    let prefab_text = jsons.get(&request.prefab).expect("prefab.json 已 Loaded").0.clone();
    let animation_text = jsons.get(&request.animation).expect("animation.json 已 Loaded").0.clone();
    let texture_text = jsons.get(&request.texture).expect("texture.json 已 Loaded").0.clone();
    let screen_text = jsons.get(&request.screen).expect("screen_layer.json 已 Loaded").0.clone();
    let sites_text = jsons.get(&request.sites).expect("sites.json 已 Loaded").0.clone();

    // ---- prefab.json：按根分片出布局与贴图 ----
    let prefab: serde_json::Value =
        serde_json::from_str(&prefab_text).unwrap_or_else(|err| panic!("sitemap：prefab.json 不是合法 JSON：{err}"));
    let roots: Vec<(String, usize)> = prefab["roots"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：prefab.json 缺 roots 数组"))
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap_or_else(|| panic!("sitemap：roots 行缺 name")).to_owned(),
                r["nodes"].as_u64().unwrap_or_else(|| panic!("sitemap：roots 行缺 nodes 计数")) as usize,
            )
        })
        .collect();
    let node_layout = |fields: &serde_json::Value| NodeLayout {
        anchored: Vec2::new(
            fields["m_AnchoredPosition"]["x"].as_f64().unwrap_or(0.0) as f32,
            fields["m_AnchoredPosition"]["y"].as_f64().unwrap_or(0.0) as f32,
        ),
        size: Vec2::new(
            fields["m_SizeDelta"]["x"].as_f64().unwrap_or(0.0) as f32,
            fields["m_SizeDelta"]["y"].as_f64().unwrap_or(0.0) as f32,
        ),
        scale: Vec3::new(
            fields["m_LocalScale"]["x"].as_f64().unwrap_or(1.0) as f32,
            fields["m_LocalScale"]["y"].as_f64().unwrap_or(1.0) as f32,
            fields["m_LocalScale"]["z"].as_f64().unwrap_or(1.0) as f32,
        ),
        rotation: Quat::from_xyzw(
            fields["m_LocalRotation"]["x"].as_f64().unwrap_or(0.0) as f32,
            fields["m_LocalRotation"]["y"].as_f64().unwrap_or(0.0) as f32,
            fields["m_LocalRotation"]["z"].as_f64().unwrap_or(0.0) as f32,
            fields["m_LocalRotation"]["w"].as_f64().unwrap_or(1.0) as f32,
        ),
    };
    let component_instances = |type_name: &str| -> Vec<serde_json::Value> {
        prefab["components"][type_name]["instances"]
            .as_array()
            .unwrap_or_else(|| panic!("sitemap：prefab.json 缺 components.{type_name}.instances"))
            .clone()
    };
    // RectTransform 按根计数切片（提取侧的遍历序即根分组连续）；其余
    // 组件族（CustomRawImage）计数不同，用「节点路径属于哪个根」的单调
    // 指针分片——两组件族都在 hide_cloud/open_cloud 里出现同名节点
    // cloud/cloud (N)，靠分组连续性归属。
    let rect_instances = component_instances("RectTransform");
    let mut layout: HashMap<(String, String), NodeLayout> = HashMap::new();
    let mut root_paths: Vec<(String, std::collections::HashSet<String>)> = Vec::new();
    {
        let mut at = 0usize;
        for (name, count) in &roots {
            let mut paths = std::collections::HashSet::new();
            for inst in &rect_instances[at..at + count] {
                let node = inst["node"].as_str().unwrap_or("").to_owned();
                paths.insert(node.clone());
                layout.insert((name.clone(), node), node_layout(&inst["fields"]));
            }
            root_paths.push((name.clone(), paths));
            at += count;
        }
        assert_eq!(
            at, rect_instances.len(),
            "sitemap：RectTransform 实例数与 roots 计数不符"
        );
    }
    let mut raw_images: HashMap<(String, String), RawImage> = HashMap::new();
    {
        let mut root_at = 0usize;
        for inst in component_instances("CustomRawImage") {
            let node = inst["node"].as_str().unwrap_or("").to_owned();
            while root_at < roots.len() && !root_paths[root_at].1.contains(&node) {
                root_at += 1;
            }
            assert!(
                root_at < roots.len(),
                "sitemap：CustomRawImage 实例的节点 {node} 不属于任何根"
            );
            let root = root_paths[root_at].0.clone();
            let fields = &inst["fields"];
            let color = &fields["m_Color"];
            let texture = fields["m_Texture"]["name"]
                .as_str()
                .unwrap_or_else(|| panic!("sitemap：CustomRawImage {node} 缺贴图名"))
                .to_owned();
            raw_images.insert(
                (root, node),
                RawImage {
                    texture,
                    color: [
                        color["r"].as_f64().unwrap_or(1.0) as f32,
                        color["g"].as_f64().unwrap_or(1.0) as f32,
                        color["b"].as_f64().unwrap_or(1.0) as f32,
                        color["a"].as_f64().unwrap_or(1.0) as f32,
                    ],
                },
            );
        }
    }

    // ---- animation.json：clip_open_cloud 的 32 条曲线 ----
    let animation: serde_json::Value =
        serde_json::from_str(&animation_text).unwrap_or_else(|err| panic!("sitemap：animation.json 不是合法 JSON：{err}"));
    let clips = animation["animations"]["clips"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：animation.json 缺 animations.clips"));
    let clip = clips
        .iter()
        .find(|c| c["name"].as_str() == Some("clip_open_cloud"))
        .unwrap_or_else(|| panic!("sitemap：animation.json 缺 clip_open_cloud"));
    let mut curves: Vec<CurveBinding> = Vec::new();
    let mut curve_index: HashMap<(String, String), usize> = HashMap::new();
    for cv in clip["curves"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：clip_open_cloud 缺 curves"))
    {
        let node = cv["node"].as_str().unwrap_or("").to_owned();
        let attribute = cv["attribute"].as_str().unwrap_or("").to_owned();
        match attribute.as_str() {
            // 开云驱动的五个属性 + site_mask 的 rgb 三条常值曲线（显示色
            // 恒白，无需驱动）。
            "m_AnchoredPosition.x" | "m_AnchoredPosition.y" | "m_Color.a" | "m_IsActive"
            | "m_Alpha" => {}
            "m_Color.r" | "m_Color.g" | "m_Color.b" => continue,
            other => panic!("sitemap：clip_open_cloud 有未认识的曲线属性 {other}"),
        }
        let keys = cv["keys"]
            .as_array()
            .unwrap_or_else(|| panic!("sitemap：曲线 {node}.{attribute} 缺 keys"));
        let curve = match cv["kind"].as_str() {
            Some("cubic") => Curve::Cubic(
                keys.iter()
                    .map(|k| {
                        let c = k[1].as_array().unwrap_or_else(|| {
                            panic!("sitemap：cubic 曲线 {node}.{attribute} 的键缺系数组")
                        });
                        (
                            k[0].as_f64().unwrap_or_else(|| {
                                panic!("sitemap：曲线 {node}.{attribute} 的键缺时间")
                            }) as f32,
                            [
                                c[0].as_f64().unwrap_or(0.0) as f32,
                                c[1].as_f64().unwrap_or(0.0) as f32,
                                c[2].as_f64().unwrap_or(0.0) as f32,
                                c[3].as_f64().unwrap_or(0.0) as f32,
                            ],
                        )
                    })
                    .collect(),
            ),
            Some("const") => {
                let k = keys
                    .first()
                    .unwrap_or_else(|| panic!("sitemap：const 曲线 {node}.{attribute} 缺键"));
                Curve::Const(k[1].as_f64().unwrap_or(0.0) as f32)
            }
            other => panic!("sitemap：曲线 {node}.{attribute} 的 kind 未认识：{other:?}"),
        };
        curve_index.insert((node.clone(), attribute.clone()), curves.len());
        curves.push(CurveBinding { node, attribute, curve });
    }
    // SE 事件在解析期核验（铺装后按常量时刻触发日志）。
    let se_ok = clip["events"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：clip_open_cloud 缺 events"))
        .iter()
        .any(|e| {
            e["functionName"].as_str() == Some("OnPlaySE")
                && e["stringParameter"].as_str() == Some("se_unlock_site")
                && (e["time"].as_f64().unwrap_or(0.0) - SE_EVENT_SECONDS as f64).abs() < 1e-3
        });
    assert!(se_ok, "sitemap：clip_open_cloud 的 SE 事件与 0.7s OnPlaySE(se_unlock_site) 不符");

    // ---- texture.json：sRGB 变体表 ----
    let texture_manifest: serde_json::Value =
        serde_json::from_str(&texture_text).unwrap_or_else(|err| panic!("sitemap：texture.json 不是合法 JSON：{err}"));
    let colour_space: HashMap<String, bool> = texture_manifest["textureColourSpace"]
        .as_object()
        .unwrap_or_else(|| panic!("sitemap：texture.json 缺 textureColourSpace"))
        .iter()
        .map(|(k, v)| (k.clone(), v.as_bool().unwrap_or(true)))
        .collect();
    let strip_hash = |uri: &str| -> String {
        let file = uri.rsplit('/').next().unwrap_or(uri);
        match file.rfind('-') {
            Some(at) => file[..at].to_owned(),
            None => file.trim_end_matches(".png").to_owned(),
        }
    };
    let mut srgb_names: HashMap<String, String> = HashMap::new();
    for uri in texture_manifest["textures"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：texture.json 缺 textures 数组"))
    {
        let uri = uri.as_str().unwrap_or("");
        if colour_space.get(uri).copied().unwrap_or(false) {
            srgb_names.insert(strip_hash(uri), uri.to_owned());
        }
    }

    // ---- sites.json：站点行（on_map 以图标贴图存在性为准） ----
    let sites_value: serde_json::Value =
        serde_json::from_str(&sites_text).unwrap_or_else(|err| panic!("sitemap：sites.json 不是合法 JSON：{err}"));
    let site_rows = sites_value["sites"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：sites.json 缺 sites 数组"));
    let mut sites: Vec<SiteRow> = site_rows
        .iter()
        .map(|row| {
            let id = row["id"].as_i64().unwrap_or_else(|| panic!("sitemap：站点行缺 id")) as i32;
            let icon = format!("img_map_site_{id}");
            SiteRow {
                id,
                name: row["name"].as_str().unwrap_or("").to_owned(),
                site_type: row["siteType"]
                    .as_str()
                    .unwrap_or_else(|| panic!("sitemap：站点 {id} 行缺 siteType"))
                    .to_owned(),
                on_map: srgb_names.contains_key(&icon),
            }
        })
        .collect();
    sites.sort_by_key(|s| s.id);
    assert!(
        sites.iter().any(|s| s.id == DEFAULT_SITE_ID && s.on_map),
        "sitemap：站点表缺默认站点 {DEFAULT_SITE_ID}（HOME 必须有图标）"
    );

    // ---- screen_layer.json：屏幕层布局真值 ----
    // 图标与位置按下标配对（连接律），再按 siteType 名接主表行；锚点
    // 换算见 [Frame::place]——画布中心在原点，参照 1920×1080。
    let screen: serde_json::Value =
        serde_json::from_str(&screen_text).unwrap_or_else(|err| panic!("sitemap：screen_layer.json 不是合法 JSON：{err}"));
    let canvas_ref = {
        let canvas = &screen["canvas"]["referenceResolution"];
        (
            canvas[0].as_f64().unwrap_or(1920.0) as f32,
            canvas[1].as_f64().unwrap_or(1080.0) as f32,
        )
    };
    assert_eq!(
        (canvas_ref.0, canvas_ref.1),
        (CANVAS_REF_W, CANVAS_REF_H),
        "sitemap：屏幕层画布参照与气泡层的 1920×1080 律不符"
    );
    let canvas_center = Vec2::ZERO;
    let canvas_size = Vec2::new(CANVAS_REF_W, CANVAS_REF_H);
    let icon_rows = screen["icons"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：screen_layer.json 缺 icons"))
        .iter()
        .map(|icon| {
            (
                icon["siteType"].as_str().unwrap_or_else(|| panic!("sitemap：图标行缺 siteType")).to_owned(),
                Frame::parse(&icon["rootRect"], "图标根").scale,
                Frame::parse(&icon["buttonRect"], "按钮件"),
            )
        })
        .collect::<Vec<_>>();
    let position_rows = screen["iconPositions"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：screen_layer.json 缺 iconPositions"))
        .iter()
        .map(|entry| Frame::parse(&entry["rect"], "位置件"))
        .collect::<Vec<_>>();
    assert_eq!(
        icon_rows.len(),
        position_rows.len(),
        "sitemap：图标 {} 行与位置 {} 行不配对",
        icon_rows.len(),
        position_rows.len()
    );
    // 图标根与位置件同处 Content 系（Content 在画布原点）：位置件经
    // 一次 place 落进画布单位，就是图标根的运行时位置。
    let spots: Vec<IconSpot> = icon_rows
        .iter()
        .zip(position_rows.iter())
        .map(|((site_type, root_scale, button), position)| IconSpot {
            site_type: site_type.clone(),
            position: position
                .place(Vec2::ZERO, Vec2::ZERO)
                .center,
            root_scale: *root_scale,
            button: button.place(Vec2::ZERO, Vec2::ZERO).size,
            festival: site_type == "festival_garden",
        })
        .collect();
    assert_eq!(
        spots[HOME_SITE_ICON_INDEX].site_type, "home_site",
        "sitemap：屏幕层图标列表第 0 位必须是 home_site（真源 HOME_SITE_ICON_INDEX=0）"
    );
    let frame_of = |section: &str| -> Frame {
        let node = &screen["screen"][section];
        Frame::parse(&node["rect"], &format!("screen.{section}"))
    };
    let ground = frame_of("mapGround").place(Vec2::ZERO, Vec2::ZERO);
    // 面板：锚在画布顶中（Anchor 0.5,1）——父矩形=画布（中心原点、
    // y 向上；anchor=1 → 锚点在画布顶缘 +540）。
    let phenomena_view = frame_of("phenomenaView").place(canvas_center, canvas_size);
    // 面板内件的面下节点（背景与行），父矩形=面板矩形。
    let parse_node_rect = |path: &str| -> Frame {
        let node = &screen["nodes"][path];
        Frame::parse(node, &format!("nodes.{path}"))
    };
    let phenomena_root_path = screen["screen"]["phenomenaView"]["path"]
        .as_str()
        .unwrap_or_else(|| panic!("sitemap：screen.phenomenaView 缺 path"))
        .to_owned();
    let background = parse_node_rect(&format!("{phenomena_root_path}/PhenometaBackgroundIcon"))
        .place(phenomena_view.center, phenomena_view.size);
    let jp_row_node = parse_node_rect(&format!("{phenomena_root_path}/Text"));
    let jp_row = jp_row_node.place(phenomena_view.center, phenomena_view.size);
    let en_row = parse_node_rect(&format!("{phenomena_root_path}/PhenomenaENText"))
        .place(phenomena_view.center, phenomena_view.size);
    // 现象图标槽 44×44（行内横排的位置由真源布局组件在运行时定，产物
    // 里是布局前的作者位——消费侧按「图标+文本组居中」现算，见
    // spawn_phenomena_icon）。
    let icon_size = parse_node_rect(&format!("{phenomena_root_path}/Text/PhenometaIcon")).size;
    let phenomena = PhenomenaLayout {
        background,
        icon_size,
        jp_row: jp_row.center,
        en_row: en_row.center,
    };
    // 图标 prefab 内件（outdoor 变体；行位/影位/线位都在图标根的本地系）。
    let prefab_outdoor = &screen["iconPrefab"]["outdoor"]["nodes"];
    let prefab_node = |name: &str| -> Frame {
        Frame::parse(
            &prefab_outdoor[name],
            &format!("iconPrefab.outdoor.{name}"),
        )
    };
    let name_outdoor = prefab_node("DefaultWorldNameBase").place(Vec2::ZERO, Vec2::ZERO).center;
    let name_home = {
        let frame = Frame::parse(
            &screen["iconPrefab"]["home"]["nodes"]["DefaultWorldNameBase"],
            "iconPrefab.home.DefaultWorldNameBase",
        );
        frame.place(Vec2::ZERO, Vec2::ZERO).center
    };
    // 面下色：site_shadow 的 tint（六实例同值；产物按路径记账）。
    let shadow_tint = screen["colors"]
        .as_object()
        .and_then(|colors| {
            colors
                .iter()
                .find(|(path, _)| path.ends_with("/site_shadow"))
                .and_then(|(_, value)| value.as_array().map(|rgba| rgba.clone()))
        })
        .unwrap_or_else(|| panic!("sitemap：colors 缺 site_shadow 行"));
    let rgba = |value: &serde_json::Value, what: &str| -> f32 {
        value
            .as_f64()
            .unwrap_or_else(|| panic!("sitemap：site_shadow tint 缺 {what}")) as f32
    };
    let prefab = PrefabLayout {
        line: prefab_node("site_line").place(Vec2::ZERO, Vec2::ZERO),
        shadow: prefab_node("site_shadow").place(Vec2::ZERO, Vec2::ZERO),
        shadow_tint: [
            rgba(&shadow_tint[0], "r"),
            rgba(&shadow_tint[1], "g"),
            rgba(&shadow_tint[2], "b"),
            rgba(&shadow_tint[3], "a"),
        ],
        name_outdoor,
        name_home,
    };
    let screen_layout = ScreenLayout {
        spots,
        ground,
        phenomena,
        prefab,
    };
    // join 完整性：每个上地图的站都要有屏层行（缺位是错账不是空缺）。
    for site in sites.iter().filter(|s| s.on_map) {
        assert!(
            screen_layout
                .spots
                .iter()
                .any(|spot| spot.site_type == site.site_type),
            "sitemap：站点 {}（{}）在屏幕层没有图标行",
            site.id,
            site.site_type
        );
    }

    // ---- 装载本模块消费的贴图（sRGB 变体） ----
    let mut wanted: Vec<String> = vec![
        "bg_map_ground".into(),
        "img_map_site_line".into(),
        "img_map_site_mask".into(),
        "img_map_shadow".into(),
        "img_map_cloud_1".into(),
        "img_map_cloud_2".into(),
        "img_map_cloud_3".into(),
        "tex_sitemap_flare_front".into(),
        "tex_sitemap_flare_back".into(),
        "bg_site_info".into(),
    ];
    for site in &sites {
        if site.on_map {
            wanted.push(format!("img_map_site_{}", site.id));
        }
    }
    for row in PHENOMENA_ROWS {
        wanted.push(format!("icon_{}", row.icon));
    }
    let missing: Vec<&String> = wanted.iter().filter(|n| !srgb_names.contains_key(*n)).collect();
    assert!(
        missing.is_empty(),
        "sitemap：贴图清单缺 sRGB 变体：{:?}",
        missing
    );
    let mut textures: HashMap<String, Handle<Image>> = HashMap::new();
    for name in &wanted {
        let uri = &srgb_names[name];
        textures.insert(
            name.clone(),
            server.load::<Image>(AssetPath::from(format!("moly://site/sitemap/texture/{uri}"))),
        );
    }

    info!(
        "sitemap：数据面就绪——RectTransform {} · CustomRawImage {} · 曲线 {} · 站点 {}（地点型 {}）· 屏幕层图标 {} · 贴图 {}",
        layout.len(),
        raw_images.len(),
        curves.len(),
        sites.len(),
        sites.iter().filter(|s| s.on_map).count(),
        screen_layout.spots.len(),
        textures.len()
    );
    let spot_report: Vec<String> = screen_layout
        .spots
        .iter()
        .map(|spot| {
            format!(
                "{}=({:.1},{:.1})x{:.2}",
                spot.site_type, spot.position.x, spot.position.y, spot.root_scale
            )
        })
        .collect();
    info!(
        "sitemap: screen truth ground=({:.1},{:.1}) {:.0}x{:.0} phenomena=({:.1},{:.1}) {:.0}x{:.0} spots [{}]",
        screen_layout.ground.center.x,
        screen_layout.ground.center.y,
        screen_layout.ground.size.x,
        screen_layout.ground.size.y,
        screen_layout.phenomena.background.center.x,
        screen_layout.phenomena.background.center.y,
        screen_layout.phenomena.background.size.x,
        screen_layout.phenomena.background.size.y,
        spot_report.join(" ")
    );
    commands.insert_resource(SitemapData {
        layout,
        raw_images,
        curves,
        curve_index,
        sites,
        screen: screen_layout,
        textures,
        textures_pending: true,
    });
    commands.remove_resource::<SitemapRequest>();
}

// ---------------------------------------------------------------------------
// 字形图集（与气泡层同范式；字符集独立——气泡的字符集钉在 tweet/对话
// 候选上，现象名与站点名不在其中，两图集各自烘）
// ---------------------------------------------------------------------------

struct GlyphCell {
    rect: Rect,
    advance: f32,
}

#[derive(Resource)]
pub(crate) struct SitemapText {
    image: Handle<Image>,
    cells: HashMap<char, GlyphCell>,
    baseline_from_top: f32,
    pen_x: f32,
    cell: f32,
}

/// 烘字符集：可打印 ASCII + 现象名（JP+EN）+ 站点名。缺字形按码点
/// 具名 panic（fail-closed：静默空格是看不见的错值）。
fn bake_text_atlas(images: &mut Assets<Image>, site_names: &[String]) -> SitemapText {
    let mut chars: Vec<char> = (0x20u8..=0x7e).map(|b| b as char).collect();
    let push = |s: &str, chars: &mut Vec<char>| {
        for ch in s.chars() {
            if !chars.contains(&ch) {
                chars.push(ch);
            }
        }
    };
    for row in PHENOMENA_ROWS {
        push(row.jp, &mut chars);
        push(row.en, &mut chars);
    }
    for name in site_names {
        push(name, &mut chars);
    }
    chars.sort_unstable();
    chars.dedup();

    let font = swash::FontRef::from_index(FONT_BYTES, 0)
        .unwrap_or_else(|| panic!("sitemap：仓内字体不是可读的 OpenType 字体"));
    let mut context = swash::scale::ScaleContext::new();
    let mut scaler = context.builder(font).size(BAKE_PPEM).build();
    let render = swash::scale::Render::new(&[swash::scale::Source::Outline]);
    let glyph_metrics = font.glyph_metrics(&[]).scale(BAKE_PPEM);

    struct Raster {
        ch: char,
        advance: f32,
        left: i32,
        top: i32,
        width: usize,
        height: usize,
        data: Vec<u8>,
    }
    let mut rasters: Vec<Raster> = Vec::new();
    let mut missing: Vec<char> = Vec::new();
    for ch in chars.iter().copied() {
        let gid = font.charmap().map(ch as u32);
        let advance = glyph_metrics.advance_width(gid);
        if gid == 0 {
            missing.push(ch);
            continue;
        }
        let Some(glyph) = render.render(&mut scaler, gid) else {
            missing.push(ch);
            continue;
        };
        let (w, h) = (glyph.placement.width as usize, glyph.placement.height as usize);
        if w == 0 || h == 0 {
            rasters.push(Raster { ch, advance, left: 0, top: 0, width: 0, height: 0, data: Vec::new() });
            continue;
        }
        rasters.push(Raster {
            ch,
            advance,
            left: glyph.placement.left,
            top: glyph.placement.top,
            width: w,
            height: h,
            data: glyph.data.to_vec(),
        });
    }
    if !missing.is_empty() {
        let hex: Vec<String> = missing.iter().map(|ch| format!("U+{:04X}", *ch as u32)).collect();
        panic!(
            "sitemap：仓内字体缺字形 {} 个 [{}]——显示文本会静默缺字",
            missing.len(),
            hex.join(" ")
        );
    }
    // 第一遍：按实际墨迹量上下左右界（不按 hhea：见气泡层的同款注释）。
    let (mut ink_up, mut ink_down) = (0.0f32, 0.0f32);
    let (mut ink_left, mut ink_right) = (0.0f32, 0.0f32);
    for r in &rasters {
        if r.width == 0 {
            continue;
        }
        ink_up = ink_up.max(r.top as f32);
        ink_down = ink_down.max(r.height as f32 - r.top as f32);
        ink_left = ink_left.min(r.left as f32);
        ink_right = ink_right.max(r.left as f32 + r.width as f32);
    }
    let pen_x = CELL_PAD + (-ink_left).max(0.0);
    let baseline_from_top = CELL_PAD + ink_up.ceil() + 1.0;
    let cell = (baseline_from_top + ink_down.ceil() + 1.0 + CELL_PAD)
        .max(pen_x + ink_right.ceil() + 1.0 + CELL_PAD)
        .ceil();
    let cols = (ATLAS_SIZE / cell) as usize;
    let capacity = cols * cols;
    if rasters.len() > capacity {
        panic!(
            "sitemap：字符集 {count} 超出图集容量 {capacity}（格 {cell:.0}px）：加大图集边长",
            count = rasters.len()
        );
    }

    let mut data = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE) as usize * 4];
    let mut cells: HashMap<char, GlyphCell> = HashMap::new();
    for (index, r) in rasters.iter().enumerate() {
        let col = index % cols;
        let row = index / cols;
        let x0 = col as f32 * cell;
        let y0 = row as f32 * cell;
        let rect = Rect {
            min: Vec2::new(x0, y0),
            max: Vec2::new(x0 + cell, y0 + cell),
        };
        if r.width == 0 {
            cells.insert(r.ch, GlyphCell { rect, advance: r.advance });
            continue;
        }
        // swash 的 placement：以基线笔点为原点（y 向下为正），墨迹从
        // (left, -top) 起。
        let origin_x = x0 + pen_x + r.left as f32;
        let origin_y = y0 + baseline_from_top - r.top as f32;
        if origin_x < x0
            || origin_y < y0
            || origin_x + r.width as f32 > x0 + cell
            || origin_y + r.height as f32 > y0 + cell
        {
            panic!(
                "sitemap：字形 U+{:04X} 的墨迹 {}x{} 装不进 {:.0}px 格：格几何与第一遍量得的界不符",
                r.ch as u32, r.width, r.height, cell
            );
        }
        for gy in 0..r.height {
            for gx in 0..r.width {
                let coverage = r.data[gy * r.width + gx];
                let px = origin_x as usize + gx;
                let py = origin_y as usize + gy;
                let at = (py * ATLAS_SIZE as usize + px) * 4;
                // 白墨 + 覆盖度进 alpha；染色靠 sprite 的 color。
                data[at] = 255;
                data[at + 1] = 255;
                data[at + 2] = 255;
                data[at + 3] = data[at + 3].max(coverage);
            }
        }
        cells.insert(r.ch, GlyphCell { rect, advance: r.advance });
    }
    info!(
        "sitemap：字形图集烘成 {} 格（字符集 {}，容量 {capacity}），格 {:.0}px 笔点=({pen_x:.0},{baseline_from_top:.0})",
        cells.len(),
        chars.len(),
        cell
    );
    let image = images.add(Image::new(
        Extent3d {
            width: ATLAS_SIZE as u32,
            height: ATLAS_SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    ));
    SitemapText {
        image,
        cells,
        baseline_from_top,
        pen_x,
        cell,
    }
}

// ---------------------------------------------------------------------------
// 实体结构（全部 canvas 单位；两根分别乘 map_fit×canvas_scale 与
// canvas_scale——底图随地图根缩放，天气钮不随地图缩放）
// ---------------------------------------------------------------------------

/// 覆盖层总根（M 键切显隐的实体）。
#[derive(Component)]
pub(crate) struct SitemapRoot;

/// 地图根（底图 + 图标列；缩放 = map_fit × canvas_scale）。
#[derive(Component)]
pub(crate) struct MapRoot;

/// UI 根（天气钮；缩放 = canvas_scale）。
#[derive(Component)]
pub(crate) struct UiRoot;

/// 小地图覆盖相机（order 2；气泡层 order 1、主相机 order 0）。
#[derive(Component)]
pub(crate) struct SitemapCamera;

/// 一个站点图标（含锁定态云簇与解锁后的三件）。
#[derive(Component)]
pub(crate) struct MapIcon {
    /// `sites` 表下标。
    site: usize,
    unlocked: bool,
    /// 图标图 sprite（开云 3.0s 淡入的写点）。
    image: Entity,
    /// 图标阴影 sprite（与图标图同组淡入——真源里它在按钮组下）。
    shadow: Entity,
}

/// 图标入场运行态（有效进度 = t − order×IN_STAGGER）。
#[derive(Component)]
pub(crate) struct IconIn {
    order: usize,
    t: f32,
    base_y: f32,
}

/// 图标浮动（入场完成后插上；真源 PlayFloatingAnimation 的 yoyo 往返）。
#[derive(Component)]
pub(crate) struct IconFloat {
    t: f32,
    base_y: f32,
}

/// 图标部件 sprite（入场整组淡入的写点；base_alpha = 该部件的目标
/// 不透明度——锁定站点的图标图与阴影是 0）。
#[derive(Component)]
pub(crate) struct IconPart {
    site: usize,
    base_alpha: f32,
}

/// 云簇（锁定站点的 hide_cloud 实例；点击命中域 = 根 400×400）。
#[derive(Component)]
pub(crate) struct CloudCluster {
    site: usize,
    half: f32,
}

/// 名字字形（开云 3.5s 起的 1.5s 淡入写点）。
#[derive(Component)]
pub(crate) struct NameGlyph {
    base: Color,
}

/// 开云运行态（点击/自动钩子触发；t ≥ 3.5+1.5 后收口）。
#[derive(Component)]
pub(crate) struct UnlockRun {
    site: usize,
    t: f32,
    se_logged: bool,
    /// 名字淡入起算（t 过 3.5 后起计时）。
    name_t: Option<f32>,
    /// 云 sprite（实体, PosX, PosY, ColorAlpha 曲线下标）。
    clouds: Vec<(Entity, usize, usize, usize)>,
    /// cloud 容器的 m_Alpha 曲线下标（乘进全部子云）。
    group_alpha: usize,
    flare_front: (Entity, usize),
    flare_back: (Entity, usize),
    site_mask: (Entity, usize),
    /// flare_front 的 m_IsActive 曲线下标。
    flare_active: usize,
    icon: Entity,
    icon_image: Entity,
    icon_shadow: Entity,
    name_glyphs: Vec<Entity>,
    /// open_cloud 实例子树根（3.5s 时整体回收）。
    open_root: Entity,
    base_y: f32,
    last_sample: f32,
}

/// 天气面板（SiteMapPhenomenaView）。
#[derive(Component)]
pub(crate) struct WeatherPanel {
    icon: Entity,
    /// 两行文本的行父实体（字形是其子实体，撤行父即整行撤——刷新期
    /// 只撤一次，见 refresh_weather）。
    rows: Vec<Entity>,
    row: usize,
    /// 上次见到的现象档名（变更检测；档名不在主表时也记，避免每帧重报）。
    last_asset: String,
}

// ---------------------------------------------------------------------------
// 铺装辅助
// ---------------------------------------------------------------------------

/// 一个 sprite 的铺装（custom_size = prefab 的显示尺寸；z 决定同层遮挡，
/// 与真源的兄弟序一致）。
fn spawn_image(
    commands: &mut Commands,
    data: &SitemapData,
    texture: &str,
    size: Vec2,
    position: Vec2,
    z: f32,
    color: Color,
) -> Entity {
    let handle = data
        .textures
        .get(texture)
        .unwrap_or_else(|| panic!("sitemap：贴图 {texture} 不在装载表里"));
    commands
        .spawn((
            Sprite {
                image: handle.clone(),
                color,
                custom_size: Some(size),
                ..default()
            },
            Transform::from_xyz(position.x, position.y, z),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id()
}

/// 一段文本的铺装（字形 sprite 序列挂 parent 下；居中或左对齐起排）。
/// 一段文本在给定字号下的排版宽度（字形步进的和；与 [`spawn_text`] 同式）。
fn text_width(text: &SitemapText, content: &str, size: f32) -> f32 {
    let scale = size / BAKE_PPEM;
    content
        .chars()
        .filter_map(|ch| text.cells.get(&ch))
        .map(|c| c.advance)
        .sum::<f32>()
        * scale
}

fn spawn_text(
    commands: &mut Commands,
    text: &SitemapText,
    parent: Entity,
    content: &str,
    size: f32,
    color: Color,
    origin_left: bool,
) -> Vec<Entity> {
    let scale = size / BAKE_PPEM;
    let total = text_width(text, content, size);
    let mut pen = if origin_left { 0.0 } else { -total / 2.0 };
    let mut entities = Vec::new();
    for ch in content.chars() {
        let Some(cell) = text.cells.get(&ch) else { continue };
        let dx = (text.cell / 2.0 - text.pen_x) * scale;
        let dy = (text.cell / 2.0 - text.baseline_from_top) * scale;
        let x = pen + dx;
        let glyph = commands
            .spawn((
                Sprite {
                    image: text.image.clone(),
                    color,
                    rect: Some(cell.rect),
                    custom_size: Some(Vec2::splat(text.cell * scale)),
                    ..default()
                },
                Transform::from_xyz(x, -dy, 0.0),
                RenderLayers::layer(SITEMAP_LAYER),
            ))
            .id();
        commands.entity(parent).add_child(glyph);
        entities.push(glyph);
        pen += cell.advance * scale;
    }
    entities
}

/// DOTween ease 的 OutQuad（入场 ease 是 serialized 曲线，mock 取默认
/// OutQuad）。
fn ease_out_quad(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

/// DOTween ease 3（OutSine；真源 OpenSiteAsync 两处淡入的 SetEase 常量）。
fn ease_out_sine(t: f32) -> f32 {
    (t * std::f32::consts::FRAC_PI_2).sin()
}

// ---------------------------------------------------------------------------
// 系统
// ---------------------------------------------------------------------------

/// Startup：小地图覆盖相机。
pub(crate) fn overlay_camera(mut commands: Commands) {
    commands.spawn((
        SitemapCamera,
        Camera2d,
        Camera {
            order: 2,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        RenderLayers::layer(SITEMAP_LAYER),
    ));
}

/// Update：两根的缩放每帧对窗（resize 免重启）。地图整层与 UI 板同一
/// 个 canvas_scale——屏幕层的真值单位就是参照画布像素，没有第二层缩放。
pub(crate) fn fit_root(
    windows: Query<&Window>,
    mut map_roots: Query<&mut Transform, (With<MapRoot>, Without<UiRoot>)>,
    mut ui_roots: Query<&mut Transform, (With<UiRoot>, Without<MapRoot>)>,
) {
    let Ok(window) = windows.single() else { return };
    let canvas = balloon::canvas_scale(window.width(), window.height());
    for mut transform in &mut map_roots {
        transform.scale = Vec3::splat(canvas);
    }
    for mut transform in &mut ui_roots {
        transform.scale = Vec3::splat(canvas);
    }
}

/// Update：数据与贴图到齐铺一次（覆盖层总根存在即返回）。
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    data: Option<ResMut<SitemapData>>,
    roots: Query<(), With<SitemapRoot>>,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
) {
    if !roots.is_empty() {
        return;
    }
    let Some(mut data) = data else { return };
    if data.textures_pending {
        for handle in data.textures.values() {
            match server.load_state(handle) {
                LoadState::Loaded => {}
                LoadState::Failed(err) => {
                    panic!("sitemap：贴图装载失败（{handle:?}）：{err:?}");
                }
                _ => return,
            }
        }
        data.textures_pending = false;
    }
    let site_names: Vec<String> = data
        .sites
        .iter()
        .filter(|s| s.on_map)
        .map(|s| s.name.clone())
        .collect();
    let text = bake_text_atlas(&mut images, &site_names);

    // 默认收起——屏幕层生命周期的真值：该层引导即把全部子物体隐藏，
    // 只有被压入（菜单里的站点图按钮）才显示；宿主侧的展开/收起由 M 键
    // 承担（toggle 同一对状态翻转）。
    let overlay = commands
        .spawn((SitemapRoot, Visibility::Hidden, Transform::default()))
        .id();
    let map_root = commands
        .spawn((MapRoot, Visibility::default(), Transform::default()))
        .id();
    let ui_root = commands
        .spawn((UiRoot, Visibility::default(), Transform::default()))
        .id();
    commands.entity(overlay).add_child(map_root);
    commands.entity(overlay).add_child(ui_root);

    // 底图：世界全景图（真源装载给「サイトの球体画像」）。位置与尺寸
    // 都是屏幕层真值：2520×1140 的图在参照画布里锚中上（y+380）。
    let ground_at = data.screen.ground;
    let ground = spawn_image(
        &mut commands,
        &data,
        "bg_map_ground",
        ground_at.size,
        ground_at.center,
        0.0,
        Color::WHITE,
    );
    commands.entity(map_root).add_child(ground);
    info!("sitemap：覆盖层铺设，默认态=收起（屏幕层真值：引导即隐藏，压入才显示；M 键展开）");
    info!(
        "sitemap：底图 bg_map_ground {:.0}x{:.0} @ ({:.1},{:.1})（屏幕层真值；窗口缩放=canvas_scale）",
        ground_at.size.x, ground_at.size.y, ground_at.center.x, ground_at.center.y
    );

    // 图标列（屏幕层图标列表的顺序即真源入场序；HOME 恒第 0 位，解析
    // 期已断言）。位置=下标配对的位置件，缩放=实例根 authored 值。
    let mut site_index_by_type = HashMap::new();
    for (i, site) in data.sites.iter().enumerate() {
        site_index_by_type.insert(site.site_type.as_str(), i);
    }
    let mut locked = 0;
    let mut home_id = -1;
    let count = data.screen.spots.len();
    for order in 0..count {
        let site_i = site_index_by_type[data.screen.spots[order].site_type.as_str()];
        let unlocked = site_unlocked(&data.sites[site_i]);
        if !unlocked {
            locked += 1;
        }
        if order == HOME_SITE_ICON_INDEX {
            home_id = data.sites[site_i].id;
        }
        spawn_icon(&mut commands, &data, &text, map_root, site_i, order, unlocked);
    }
    assert_eq!(
        home_id, DEFAULT_SITE_ID,
        "sitemap：第 0 号图标必须是 HOME（真源 HOME_SITE_ICON_INDEX=0）"
    );

    spawn_weather_panel(&mut commands, &data, &text, ui_root);

    commands.insert_resource(text);
    info!(
        "sitemap：图标列铺成 {} 个（HOME 第 {HOME_SITE_ICON_INDEX} 号 id={home_id}；锁定 {} 个）",
        count, locked
    );
}

/// 解锁 mock（服务端进度域：真源=rank release 名单+服务端进度，未提
/// 取）。最初的 mock 是 isBase 即解锁——当时站点支持集只有 grassland 与
/// home；支持集扩到主表全量后 mock 给满：地图上的站全部解锁（九站本仓
/// 全可去，锁着不可去是矛盾状态）。锁定臂（云簇/开云链/自动开云钩子）
/// 原样保留，mock 收紧即回用。
fn site_unlocked(_site: &SiteRow) -> bool {
    true
}

/// 铺一个站点图标：图标根（入场/浮动写它的 y；缩放=实例根真值）+ 阴影
/// + 图标图 + 连线 + 名字 +（锁定）云簇。位置=屏幕层位置件，尺寸=屏幕
/// 层真值；图标贴图名按真源 `SetImage` 的装载式 img_map_site_{id}。
fn spawn_icon(
    commands: &mut Commands,
    data: &SitemapData,
    text: &SitemapText,
    map_root: Entity,
    site_i: usize,
    order: usize,
    unlocked: bool,
) {
    let site = &data.sites[site_i];
    let spot = data.spot(site_i);
    let position = spot.position;
    let icon_root = commands
        .spawn((
            IconIn { order, t: 0.0, base_y: position.y },
            Transform::from_xyz(position.x, position.y + IN_SLIDE_PX, 0.0)
                .with_scale(Vec3::splat(spot.root_scale)),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(map_root).add_child(icon_root);

    let icon_texture = format!("img_map_site_{}", site.id);

    // 阴影（_siteIconShadow 装 img_map_shadow）：屏幕层真值 352×208
    // @ (0,-162)，tint 深蓝灰（六图标实例同值）。
    let part_alpha = if unlocked { 1.0 } else { 0.0 };
    let shadow_at = data.screen.prefab.shadow;
    let tint = data.screen.prefab.shadow_tint;
    let shadow = spawn_image(
        commands,
        data,
        "img_map_shadow",
        shadow_at.size,
        shadow_at.center,
        0.1,
        Color::srgba(tint[0], tint[1], tint[2], part_alpha),
    );
    commands.entity(icon_root).add_child(shadow);
    commands
        .entity(shadow)
        .insert(IconPart { site: site_i, base_alpha: part_alpha });

    // 图标图（锁定态隐藏——真源 ResetIconState 关按钮组；开云 3.0s 淡入）。
    // 显示尺寸=屏幕层按钮件（home 842×664，其余 522×420）。
    let image = spawn_image(
        commands,
        data,
        &icon_texture,
        spot.button,
        Vec2::ZERO,
        0.2,
        Color::srgba(1.0, 1.0, 1.0, part_alpha),
    );
    commands.entity(icon_root).add_child(image);
    commands
        .entity(image)
        .insert(IconPart { site: site_i, base_alpha: part_alpha });

    // 连线（两种状态都在：真源 ShowSiteIcon 两臂都激活连线节点）。
    let line_at = data.screen.prefab.line;
    let line = spawn_image(
        commands,
        data,
        "img_map_site_line",
        line_at.size,
        line_at.center,
        0.3,
        Color::WHITE,
    );
    commands.entity(icon_root).add_child(line);
    commands.entity(line).insert(IconPart { site: site_i, base_alpha: 1.0 });

    // 名字（_siteName：解锁后随开云淡入；锁定不铺——开云时再铺）。行位
    // 是屏幕层真值：outdoor (0,-180.1)、home (0,-196)。
    let name_y = if site.site_type == "home_site" {
        data.screen.prefab.name_home.y
    } else {
        data.screen.prefab.name_outdoor.y
    };
    let name_parent = commands
        .spawn((
            Transform::from_xyz(0.0, name_y, 0.4),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(icon_root).add_child(name_parent);
    if unlocked {
        for glyph in spawn_text(commands, text, name_parent, &site.name, SITE_NAME_SIZE, TEXT_COLOR, false) {
            commands
                .entity(glyph)
                .insert(IconPart { site: site_i, base_alpha: 1.0 });
        }
    }

    if !unlocked {
        spawn_cloud_cluster(commands, data, icon_root, site_i);
    }

    commands.entity(icon_root).insert(MapIcon {
        site: site_i,
        unlocked,
        image,
        shadow,
    });
    let state = if unlocked { "unlocked" } else { "locked" };
    info!(
        "sitemap: site icon id={} type={} tex={} button={:.0}x{:.0} scale={:.2} pos=({:.1},{:.1}) truth=({:.1},{:.1}) state={} order={}",
        site.id,
        site.site_type,
        icon_texture,
        spot.button.x,
        spot.button.y,
        spot.root_scale,
        position.x,
        position.y,
        spot.position.x,
        spot.position.y,
        state,
        order
    );
}

/// 铺云簇（hide_cloud 根的构图，全部直读 prefab RectTransform）：根
/// 400×400 → cloud 容器 → 8 朵云（兄弟序即数字序）。
fn spawn_cloud_cluster(
    commands: &mut Commands,
    data: &SitemapData,
    icon_root: Entity,
    site_i: usize,
) {
    let root_layout = data
        .layout
        .get(&("hide_cloud".to_owned(), String::new()))
        .expect("sitemap：prefab 缺 hide_cloud 根的 RectTransform");
    let container_layout = data
        .layout
        .get(&("hide_cloud".to_owned(), "cloud".to_owned()))
        .expect("sitemap：prefab 缺 hide_cloud/cloud 的 RectTransform");
    // 根的运行时位置：真源 SetupHideCloud 把 hide_cloud 对齐到按钮位置
    // （serialized 值被覆盖），按钮在图标根的 (0,0)。
    let cluster = commands
        .spawn((
            CloudCluster { site: site_i, half: root_layout.size.x / 2.0 },
            Transform::from_xyz(0.0, 0.0, 0.5),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(icon_root).add_child(cluster);

    let container = commands
        .spawn((
            Transform::from_xyz(container_layout.anchored.x, container_layout.anchored.y, 0.0),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(cluster).add_child(container);

    let mut report = Vec::new();
    for n in 1..=8usize {
        let path = format!("cloud/cloud ({n})");
        let l = data
            .layout
            .get(&("hide_cloud".to_owned(), path.clone()))
            .unwrap_or_else(|| panic!("sitemap：prefab 缺 hide_cloud/{path}"));
        let image = data
            .raw_images
            .get(&("hide_cloud".to_owned(), path.clone()))
            .unwrap_or_else(|| panic!("sitemap：prefab 缺 hide_cloud/{path} 的贴图"));
        let cloud = commands
            .spawn((
                Sprite {
                    image: data
                        .textures
                        .get(&image.texture)
                        .unwrap_or_else(|| panic!("sitemap：云贴图 {} 不在装载表", image.texture))
                        .clone(),
                    color: Color::srgba(image.color[0], image.color[1], image.color[2], image.color[3]),
                    custom_size: Some(l.size),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(l.anchored.x, l.anchored.y, 0.001 * n as f32),
                    rotation: l.rotation,
                    scale: l.scale,
                },
                RenderLayers::layer(SITEMAP_LAYER),
            ))
            .id();
        commands.entity(container).add_child(cloud);
        commands
            .entity(cloud)
            .insert(IconPart { site: site_i, base_alpha: image.color[3] });
        report.push(format!(
            "{n}:{} ({:.1},{:.1}) {:.0}x{:.0} sx{:.2}",
            image.texture, l.anchored.x, l.anchored.y, l.size.x, l.size.y, l.scale.x
        ));
    }
    info!(
        "sitemap: cloud cluster site={} root {:.0}x{:.0} clouds [{}]",
        data.sites[site_i].id,
        root_layout.size.x,
        root_layout.size.y,
        report.join(", ")
    );
}

/// 现象行的三件位（面板系）：图标中心、JP 文本左起点、EN 行中心。
/// JP 行 =「图标 + 文本」横排、组在行心居中（真源布局组件的组居中律；
/// 间距取具名近似值）。铺装与刷新共用，两路的组宽口径一致。
fn phenomena_slots(
    data: &SitemapData,
    text: &SitemapText,
    row: usize,
) -> (Vec2, Vec2, Vec2) {
    let layout = &data.screen.phenomena;
    let panel_at = layout.background.center;
    let jp_row_at = layout.jp_row - panel_at;
    let en_row_at = layout.en_row - panel_at;
    let jp_text_width = text_width(text, &PHENOMENA_ROWS[row].jp, PHENOMENA_JP_SIZE);
    let group_width = layout.icon_size.x + PHENOMENA_ICON_GAP_PX + jp_text_width;
    let icon_at = jp_row_at - Vec2::new(group_width / 2.0, 0.0)
        + Vec2::new(layout.icon_size.x / 2.0, 0.0);
    let jp_text_at = jp_row_at - Vec2::new(group_width / 2.0, 0.0)
        + Vec2::new(layout.icon_size.x + PHENOMENA_ICON_GAP_PX, 0.0);
    (icon_at, jp_text_at, en_row_at)
}

/// 铺天气面板（SiteMapPhenomenaView：bg_site_info + icon + 两行文本，
/// 初始默认现象）。位置与内件排布都是屏幕层真值：面板钉在画布顶中
/// （锚 (0.5,1) @ (0,8)，512×128），现象图标在 JP 行内、EN 行在下。
/// 独立的天气按钮件（开现象选择屏）不在本单范围，不上屏（挂账）。
fn spawn_weather_panel(
    commands: &mut Commands,
    data: &SitemapData,
    text: &SitemapText,
    ui_root: Entity,
) {
    let layout = &data.screen.phenomena;
    let at = layout.background.center;
    let panel = commands
        .spawn((
            Transform::from_xyz(at.x, at.y, 1.0),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(ui_root).add_child(panel);

    let bg = spawn_image(
        commands,
        data,
        "bg_site_info",
        layout.background.size,
        Vec2::ZERO,
        0.0,
        Color::WHITE,
    );
    commands.entity(panel).add_child(bg);

    let row = PHENOMENA_ROWS
        .iter()
        .position(|r| r.id == DEFAULT_PHENOMENA_ID)
        .unwrap_or_else(|| panic!("sitemap：现象表缺默认行 {DEFAULT_PHENOMENA_ID}"));
    // JP 行 =「图标 + 文本」横排、组在行心居中；图标与文本的 x 都从
    // 组宽现算（与刷新路径共用 [`phenomena_slots`]）。
    let (icon_at, jp_text_at, en_row_at) = phenomena_slots(data, &text, row);
    let icon = spawn_phenomena_icon(commands, data, panel, row, icon_at);
    let rows = spawn_phenomena_text(commands, text, panel, row, jp_text_at, en_row_at);
    commands.entity(panel).insert(WeatherPanel {
        icon,
        rows,
        row,
        last_asset: PHENOMENA_ROWS[row].asset.to_owned(),
    });
    let p = &PHENOMENA_ROWS[row];
    info!(
        "sitemap: phenomena panel at ({:.1},{:.1}) bg {:.0}x{:.0}（屏幕层真值 顶中）; row id={} en={} jp_len={} icon=icon_{}",
        at.x, at.y, layout.background.size.x, layout.background.size.y,
        p.id, p.en, p.jp.chars().count(), p.icon
    );
}

fn spawn_phenomena_icon(
    commands: &mut Commands,
    data: &SitemapData,
    panel: Entity,
    row: usize,
    at: Vec2,
) -> Entity {
    let name = format!("icon_{}", PHENOMENA_ROWS[row].icon);
    // 图标槽 44×44（屏幕层真值）；行内 x 由调用方按组宽给定。
    let icon = spawn_image(
        commands,
        data,
        &name,
        data.screen.phenomena.icon_size,
        at,
        0.1,
        Color::WHITE,
    );
    commands.entity(panel).add_child(icon);
    icon
}

/// 两行文本（JP 行在图标右侧、EN 行在下面板中部；行中心是屏幕层真值）。
/// 返回**行父实体**——字形是行的子实体，随行父撤（撤两遍就是那批
/// despawn WARN：天气切换 × 面板重建的组合，见 refresh_weather）。
fn spawn_phenomena_text(
    commands: &mut Commands,
    text: &SitemapText,
    panel: Entity,
    row: usize,
    jp_text_at: Vec2,
    en_row_at: Vec2,
) -> Vec<Entity> {
    let row_data = &PHENOMENA_ROWS[row];
    let mut rows = Vec::new();
    for ((content, size), origin_left, at) in [
        ((&row_data.jp[..], PHENOMENA_JP_SIZE), true, jp_text_at),
        ((&row_data.en[..], PHENOMENA_EN_SIZE), false, en_row_at),
    ] {
        let parent = commands
            .spawn((
                Transform::from_xyz(at.x, at.y, 0.1),
                Visibility::default(),
                RenderLayers::layer(SITEMAP_LAYER),
            ))
            .id();
        commands.entity(panel).add_child(parent);
        rows.push(parent);
        spawn_text(commands, text, parent, content, size, TEXT_COLOR, origin_left);
    }
    rows
}

// ---------------------------------------------------------------------------
// 开云：点击 → open_cloud 实例 → 32 曲线逐帧
// ---------------------------------------------------------------------------

/// 开云实例的可驱动实体位（曲线下标由 curve_index 绑定，缺曲线即拒）。
struct OpenCloudParts {
    clouds: Vec<(Entity, usize, usize, usize)>,
    group_alpha: usize,
    flare_front: (Entity, usize),
    flare_back: (Entity, usize),
    site_mask: (Entity, usize),
    flare_active: usize,
    root: Entity,
}

/// 铺 open_cloud 实例（构图直读 prefab：flare_back → site_mask →
/// cloud 容器 → flare_front，兄弟序即遮挡序；初始态 alpha 0——
/// 曲线逐帧写）。
fn spawn_open_cloud(
    commands: &mut Commands,
    data: &SitemapData,
    icon_root: Entity,
) -> OpenCloudParts {
    let curve = |node: &str, attribute: &str| -> usize {
        *data
            .curve_index
            .get(&(node.to_owned(), attribute.to_owned()))
            .unwrap_or_else(|| panic!("sitemap：clip_open_cloud 缺曲线 {node}.{attribute}"))
    };
    // 真源把 open_cloud 摆在按钮位置（serialized 根位被运行时覆盖；根的
    // 尺寸也不消费——命中域在 hide_cloud 侧）。
    let cluster = commands
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.6),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(icon_root).add_child(cluster);

    let plain_child = |commands: &mut Commands, parent: Entity, at: Vec2, z: f32| -> Entity {
        let e = commands
            .spawn((
                Transform::from_xyz(at.x, at.y, z),
                Visibility::default(),
                RenderLayers::layer(SITEMAP_LAYER),
            ))
            .id();
        commands.entity(parent).add_child(e);
        e
    };
    let textured = |commands: &mut Commands,
                    parent: Entity,
                    root_name: &str,
                    node: &str,
                    z: f32,
                    alpha: f32|
     -> Entity {
        let l = data
            .layout
            .get(&(root_name.to_owned(), node.to_owned()))
            .unwrap_or_else(|| panic!("sitemap：prefab 缺 {root_name}/{node}"));
        let image = data
            .raw_images
            .get(&(root_name.to_owned(), node.to_owned()))
            .unwrap_or_else(|| panic!("sitemap：prefab 缺 {root_name}/{node} 的贴图"));
        let e = commands
            .spawn((
                Sprite {
                    image: data
                        .textures
                        .get(&image.texture)
                        .unwrap_or_else(|| panic!("sitemap：贴图 {} 不在装载表", image.texture))
                        .clone(),
                    color: Color::srgba(1.0, 1.0, 1.0, alpha),
                    custom_size: Some(l.size),
                    ..default()
                },
                Transform {
                    translation: Vec3::new(l.anchored.x, l.anchored.y, z),
                    rotation: l.rotation,
                    scale: l.scale,
                },
                RenderLayers::layer(SITEMAP_LAYER),
            ))
            .id();
        commands.entity(parent).add_child(e);
        e
    };

    // flare_back（最底）→ site_mask → cloud 容器 → 云（容器内数字序）
    // → flare_front（最顶）：z 单调递增，与真源兄弟序一致。
    let flare_back_e = textured(commands, cluster, "open_cloud", "flare_back", 0.0, 0.0);
    let site_mask_e = textured(commands, cluster, "open_cloud", "site_mask", 0.01, 0.0);
    let container_layout = data
        .layout
        .get(&("open_cloud".to_owned(), "cloud".to_owned()))
        .expect("sitemap：prefab 缺 open_cloud/cloud");
    let container = plain_child(
        commands,
        cluster,
        Vec2::new(container_layout.anchored.x, container_layout.anchored.y),
        0.02,
    );
    let mut clouds = Vec::new();
    for n in 1..=8usize {
        let node = format!("cloud ({n})");
        let path = format!("cloud/{node}");
        let e = textured(commands, container, "open_cloud", &path, 0.001 * n as f32, 1.0);
        clouds.push((
            e,
            curve(&path, "m_AnchoredPosition.x"),
            curve(&path, "m_AnchoredPosition.y"),
            curve(&path, "m_Color.a"),
        ));
    }
    let flare_front_e = textured(commands, cluster, "open_cloud", "flare_front", 0.1, 0.0);

    OpenCloudParts {
        clouds,
        group_alpha: curve("cloud", "m_Alpha"),
        flare_front: (flare_front_e, curve("flare_front", "m_Color.a")),
        flare_back: (flare_back_e, curve("flare_back", "m_Color.a")),
        site_mask: (site_mask_e, curve("site_mask", "m_Color.a")),
        flare_active: curve("flare_front", "m_IsActive"),
        root: cluster,
    }
}

/// 触发开云（真源 OpenSiteAsync 的起点：hide_cloud 关、按钮组淡入
/// 3.0s、open_cloud 实例化、动画起播）。
fn begin_unlock(
    commands: &mut Commands,
    data: &SitemapData,
    text: &SitemapText,
    icon_entity: Entity,
    icon: &MapIcon,
    cluster_entity: Entity,
    transforms: &mut Query<&mut Transform>,
    parts: &mut Query<(&mut Sprite, &IconPart)>,
) {
    let site_i = icon.site;
    let site = &data.sites[site_i];
    let spot = data.spot(site_i);
    let position = spot.position;
    // 入场未完的先收口（y 归位、部件满 alpha）——开云序列起跑即接管
    // 同一个图标根。
    if let Ok(mut transform) = transforms.get_mut(icon_entity) {
        transform.translation.y = position.y;
    }
    commands.entity(icon_entity).remove::<IconIn>();
    for (mut sprite, part) in parts.iter_mut() {
        if part.site == site_i {
            sprite.color.set_alpha(part.base_alpha);
        }
    }
    // 云簇收起（点击语义：hide_cloud 关、open_cloud 开）。
    commands.entity(cluster_entity).despawn();

    // 名字（alpha 0 铺，3.5s 起淡入）。行位=屏幕层真值（与 spawn_icon
    // 的名字行同位）。
    let name_y = if site.site_type == "home_site" {
        data.screen.prefab.name_home.y
    } else {
        data.screen.prefab.name_outdoor.y
    };
    let name_parent = commands
        .spawn((
            Transform::from_xyz(0.0, name_y, 0.4),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(icon_entity).add_child(name_parent);
    let mut hidden = TEXT_COLOR;
    hidden.set_alpha(0.0);
    let name_glyphs =
        spawn_text(commands, text, name_parent, &site.name, SITE_NAME_SIZE, hidden, false);
    for glyph in &name_glyphs {
        commands.entity(*glyph).insert(NameGlyph { base: TEXT_COLOR });
    }

    let open = spawn_open_cloud(commands, data, icon_entity);
    let bindings: Vec<String> = data
        .curves
        .iter()
        .map(|c| format!("{}.{}", c.node, c.attribute))
        .collect();
    info!(
        "sitemap: unlock site={} begin curves={} [{}]",
        site.id,
        data.curves.len(),
        bindings.join(" ")
    );
    // t=0 的全部曲线现值（键时值直读，逐键可推导的起点）。
    let at_zero: Vec<String> = data
        .curves
        .iter()
        .map(|c| format!("{:.3}", c.curve.sample(0.0)))
        .collect();
    info!(
        "sitemap: unlock site={} t=0.000 values [{}]",
        site.id,
        at_zero.join(",")
    );
    commands.spawn(UnlockRun {
        site: site_i,
        t: 0.0,
        se_logged: false,
        name_t: None,
        clouds: open.clouds,
        group_alpha: open.group_alpha,
        flare_front: open.flare_front,
        flare_back: open.flare_back,
        site_mask: open.site_mask,
        flare_active: open.flare_active,
        icon: icon_entity,
        icon_image: icon.image,
        icon_shadow: icon.shadow,
        name_glyphs,
        open_root: open.root,
        base_y: position.y,
        last_sample: 0.0,
    });
}

/// Update：点击分派。锁定站点的云簇 → 开云（SiteMapHideCloud 的按钮
/// 语义 + OpenSiteAsync 时间轴）；已解锁站点照真源 `OnPushSiteMapIcon`：
/// 点的就是当前站 → 关地图（`BackUIScreen`）；点别的站 → 换站
/// （`MysekaiUtility.ChangeSite`，接 [`crate::site`] 的请求通道，支持集
/// 内的站才放行——链外站点待装载链扩编）；庆典庭院先问生日派对主表
/// 按档期过滤，空 → 一钮对话框（不去），非空才放行。
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<SitemapCamera>>,
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    text: Option<Res<SitemapText>>,
    mut roots: Query<&mut Visibility, With<SitemapRoot>>,
    icons: Query<(Entity, &MapIcon)>,
    clusters: Query<(Entity, &CloudCluster)>,
    runs: Query<(), With<UnlockRun>>,
    mut transforms: Query<&mut Transform>,
    mut parts: Query<(&mut Sprite, &IconPart)>,
    active: Option<Res<crate::site::SiteActive>>,
    birthday: Option<Res<crate::birthday::BirthdayParties>>,
    leave_dialog: Option<Res<crate::menu_shell::ShellDialogState>>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // 离开确认框、菜单对话框或获得子窗开着 ⇒ 小地图点按全被挡（真源对
    // 话框不退场地屏、靠 Dialog 槽阻断下层输入——DisableTapScreen 的同形）。
    if leave_dialog
        .map(|d| d.blocks_field_input())
        .unwrap_or(false)
    {
        return;
    }
    let Some(data) = data else { return };
    if runs.iter().next().is_some() {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = cameras.single() else { return };
    let Ok(world) = camera.viewport_to_world_2d(camera_transform, cursor) else {
        return;
    };
    let scale = balloon::canvas_scale(window.width(), window.height());
    let canvas = world / scale;
    dispatch(
        &mut commands,
        &data,
        text.as_deref(),
        &mut roots,
        &icons,
        &clusters,
        &mut transforms,
        &mut parts,
        active.as_deref(),
        birthday.as_deref(),
        canvas,
    );
}

/// 点击分派的公共体（真实输入与冒烟口同一条路）：canvas 是点击点在
/// 参照画布系里的坐标。返回是否有命中的分支被走过。
#[allow(clippy::too_many_arguments)]
fn dispatch(
    commands: &mut Commands,
    data: &SitemapData,
    text: Option<&SitemapText>,
    roots: &mut Query<&mut Visibility, With<SitemapRoot>>,
    icons: &Query<(Entity, &MapIcon)>,
    clusters: &Query<(Entity, &CloudCluster)>,
    transforms: &mut Query<&mut Transform>,
    parts: &mut Query<(&mut Sprite, &IconPart)>,
    active: Option<&crate::site::SiteActive>,
    birthday: Option<&crate::birthday::BirthdayParties>,
    canvas: Vec2,
) -> bool {
    let Some(text) = text else { return false };
    // 地图收起时不分派（M 键或点当前站退屏后的层不可见）。
    let Ok(visible) = roots.single() else { return false };
    if *visible == Visibility::Hidden {
        return false;
    }
    // 锁定臂：云簇命中（hide_cloud 根 400×400，prefab 真值）。
    for (cluster_entity, cluster) in clusters.iter() {
        let Some((icon_entity, icon)) = icons.iter().find(|(_, i)| i.site == cluster.site) else {
            continue;
        };
        if icon.unlocked {
            continue;
        }
        let position = data.spot(icon.site).position;
        if canvas.distance(position) <= cluster.half {
            begin_unlock(
                commands,
                data,
                text,
                icon_entity,
                icon,
                cluster_entity,
                transforms,
                parts,
            );
            return true;
        }
    }
    // 解锁臂：按钮件命中域（home 842×664、其余 522×420，屏幕层真值），
    // 命中后续照 OnPushSiteMapIcon 的三岔。
    for (_, icon) in icons.iter() {
        if !icon.unlocked {
            continue;
        }
        let spot = data.spot(icon.site);
        let half = spot.button / 2.0;
        let delta = (canvas - spot.position).abs();
        if delta.x <= half.x && delta.y <= half.y {
            let site = &data.sites[icon.site];
            let current = active.map(|a| a.site_type.as_str());
            if current == Some(spot.site_type.as_str()) {
                // 点当前站：真源退屏。mock 的地图就是这层屏——收起。
                for mut visible in roots.iter_mut() {
                    *visible = Visibility::Hidden;
                }
                info!(
                    "sitemap: click unlocked site={} type={} at=({:.1},{:.1}) → current site → BackUIScreen（地图收起）",
                    site.id, spot.site_type, canvas.x, canvas.y
                );
                return true;
            }
            if spot.festival {
                // 庆典庭院：真源先问 GetMasterBirthdayPartiesInSession——
                // 生日派对主表按档期两臂律过滤「现在」，空则弹一钮对话
                // 框（不去），非空才走换站。主表未到齐时本臂不定，点击
                // 无分支（装载闩语义，与全仓一致）。
                let Some(parties) = birthday else {
                    return false;
                };
                let now = crate::birthday::now_ms();
                let in_session = parties.labels_in_session(now);
                if in_session.is_empty() {
                    info!(
                        "sitemap: click unlocked site={} type=festival_garden → no birthday party in session（主表 {} 行，now={}，档期内 0 行）→ ShowCommon1ButtonDialog（具名日志，不放行）",
                        site.id,
                        parties.len(),
                        now
                    );
                    return true;
                }
                // 非空：真源放行走换站。festival_garden 的站点装载链未
                // 扩到，落到下方挂账支（同一条换站分派的门口）。
                info!(
                    "sitemap: click unlocked site={} type=festival_garden → birthday party in session（{} 行：{}，now={}）→ ChangeSiteProcessAsync 放行",
                    site.id,
                    in_session.len(),
                    in_session.join("，"),
                    now
                );
            }
            let supported = matches!(
                spot.site_type.as_str(),
                "grassland" | "home_site" | "first_floor"
            );
            if supported {
                commands.insert_resource(crate::site::SiteChangeRequest(
                    spot.site_type.clone(),
                ));
                info!(
                    "sitemap: click unlocked site={} type={} → ChangeSiteProcessAsync → 换站请求 {}（下一帧由站点域消费）",
                    site.id, spot.site_type, spot.site_type
                );
            } else {
                info!(
                    "sitemap: click unlocked site={} type={} → ChangeSite 请求挂账：站点装载链未扩到该站",
                    site.id, spot.site_type
                );
            }
            return true;
        }
    }
    false
}

/// Update：**验证用**的自动点击：`MOLY_SITEMAP_AUTOCLICK_SECS` 给了秒数、
/// `MOLY_SITEMAP_AUTOCLICK_SITE` 给了站点类型，到时后把该图标真值位置
/// 换算成画布坐标、走 [`dispatch`] 与真实点击**同一条**分派路（真源
/// 事件面注入的同款冒烟法）。地图未铺成或有开云在跑则逐帧重试。
pub(crate) fn smoke_autoclick(
    time: Res<Time>,
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    text: Option<Res<SitemapText>>,
    mut roots: Query<&mut Visibility, With<SitemapRoot>>,
    icons: Query<(Entity, &MapIcon)>,
    clusters: Query<(Entity, &CloudCluster)>,
    runs: Query<(), With<UnlockRun>>,
    mut transforms: Query<&mut Transform>,
    mut parts: Query<(&mut Sprite, &IconPart)>,
    active: Option<Res<crate::site::SiteActive>>,
    birthday: Option<Res<crate::birthday::BirthdayParties>>,
    mut armed: Local<Option<Option<(f32, String)>>>,
) {
    let Some(data) = data else { return };
    if armed.is_none() {
        let spec = std::env::var("MOLY_SITEMAP_AUTOCLICK_SECS").ok();
        let site = std::env::var("MOLY_SITEMAP_AUTOCLICK_SITE").ok();
        let parsed = spec
            .as_deref()
            .and_then(|raw| raw.trim().parse::<f32>().ok())
            .filter(|secs| *secs > 0.0)
            .zip(site.filter(|name| !name.trim().is_empty()))
            .map(|(secs, name)| (secs, name.trim().to_owned()));
        *armed = Some(parsed);
    }
    let Some(Some((secs, site_type))) = armed.as_ref().map(|s| s.as_ref()) else {
        return;
    };
    if time.elapsed_secs() < *secs {
        return;
    }
    if runs.iter().next().is_some() {
        return; // 开云在跑：下一帧重试（单一分派语义与真实点击一致）
    }
    let Some(spot) = data
        .screen
        .spots
        .iter()
        .find(|spot| spot.site_type == site_type.as_str())
    else {
        panic!("sitemap：冒烟口要点的站点 {site_type} 不在屏幕层图标表里");
    };
    let canvas = spot.position;
    // 默认收起后，无人值守的冒烟口自己开图（真人路径按 M；这里的开图与
    // M 键同一对状态翻转，日志同形可对账）。分派有「收起不分派」的门，
    // 不先开图这一击会永远落空。
    if let Ok(mut visible) = roots.single_mut() {
        if *visible == Visibility::Hidden {
            *visible = Visibility::Inherited;
            info!("sitemap: map visible=true（auto click 开图：默认收起，与 M 键同款翻转）");
        }
    }
    let fired = dispatch(
        &mut commands,
        &data,
        text.as_deref(),
        &mut roots,
        &icons,
        &clusters,
        &mut transforms,
        &mut parts,
        active.as_deref(),
        birthday.as_deref(),
        canvas,
    );
    if fired {
        info!(
            "sitemap: auto click（MOLY_SITEMAP_AUTOCLICK 冒烟口）site={site_type} canvas=({:.1},{:.1})",
            canvas.x, canvas.y
        );
        // 单次语义：发完即撤仪表（与真实点击一次一手势同构）。
        *armed = Some(None);
    }
}

/// Update：**验证用**的自动开云：环境变量 `MOLY_SITEMAP_AUTO_UNLOCK_SECS`
/// 给了秒数就在到时后对第一个锁定站点触发一次开云（与点击同一条路径）
/// ——验收要在无人按键的跑法里从日志推导开云动画；变量不给时这条路径
/// 完全不生效。到时时地图未铺成则逐帧重试。
pub(crate) fn auto_unlock(
    time: Res<Time>,
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    text: Option<Res<SitemapText>>,
    icons: Query<(Entity, &MapIcon)>,
    clusters: Query<(Entity, &CloudCluster)>,
    runs: Query<(), With<UnlockRun>>,
    mut transforms: Query<&mut Transform>,
    mut parts: Query<(&mut Sprite, &IconPart)>,
    // 外层 Some = 首调已定型；内层 Some = 剩余秒数，None = 已触发或未给。
    mut auto: Local<Option<Option<f32>>>,
) {
    let (Some(data), Some(text)) = (data, text) else { return };
    if auto.is_none() {
        *auto = Some(
            std::env::var("MOLY_SITEMAP_AUTO_UNLOCK_SECS")
                .ok()
                .and_then(|raw| raw.parse::<f32>().ok())
                .filter(|secs| *secs > 0.0),
        );
    }
    let Some(inner) = auto.as_mut() else { return };
    let Some(remaining) = inner.as_mut() else { return };
    *remaining -= time.delta_secs();
    if *remaining > 0.0 {
        return;
    }
    // 与点击同规矩：已有开云在跑就不叠（一次只跑一个，跑完下一帧仍会
    // 触发——单一触发语义不变）。
    if !runs.is_empty() {
        return;
    }
    for (cluster_entity, cluster) in clusters.iter() {
        let Some((icon_entity, icon)) = icons.iter().find(|(_, i)| i.site == cluster.site) else {
            continue;
        };
        if icon.unlocked {
            continue;
        }
        begin_unlock(
            &mut commands,
            &data,
            &text,
            icon_entity,
            icon,
            cluster_entity,
            &mut transforms,
            &mut parts,
        );
        info!("sitemap: auto unlock（MOLY_SITEMAP_AUTO_UNLOCK_SECS 钩子）");
        *inner = None;
        return;
    }
    // 地图未铺成（或无锁定站点）：保持 0 逐帧重试——数据到齐后仍会触发。
}

/// Update：入场推进（有效进度 = t − order×IN_STAGGER；未到 0 保持
/// 起始态，到 0.5s 收口换浮动）。
pub(crate) fn tick_entries(
    time: Res<Time>,
    mut commands: Commands,
    mut icons: Query<(Entity, &MapIcon, &mut IconIn, &mut Transform)>,
    mut parts: Query<(&mut Sprite, &IconPart)>,
) {
    let dt = time.delta_secs();
    for (entity, icon, mut entry, mut transform) in icons.iter_mut() {
        entry.t += dt;
        let effective = entry.t - entry.order as f32 * IN_STAGGER;
        let factor = if effective <= 0.0 {
            0.0
        } else if effective < IN_SECONDS {
            ease_out_quad(effective / IN_SECONDS)
        } else {
            1.0
        };
        transform.translation.y = entry.base_y + IN_SLIDE_PX * (1.0 - factor);
        for (mut sprite, part) in parts.iter_mut() {
            if part.site == icon.site {
                sprite.color.set_alpha(part.base_alpha * factor);
            }
        }
        if effective >= IN_SECONDS {
            for (mut sprite, part) in parts.iter_mut() {
                if part.site == icon.site {
                    sprite.color.set_alpha(part.base_alpha);
                }
            }
            commands.entity(entity).remove::<IconIn>();
            commands
                .entity(entity)
                .insert(IconFloat { t: 0.0, base_y: entry.base_y });
        }
    }
}

/// Update：浮动推进（线性 yoyo 往返；ease 是 serialized 曲线，mock）。
pub(crate) fn tick_floats(time: Res<Time>, mut floats: Query<(&mut IconFloat, &mut Transform)>) {
    let dt = time.delta_secs();
    for (mut float, mut transform) in floats.iter_mut() {
        float.t += dt;
        let cycle = float.t % (2.0 * FLOAT_SECONDS);
        let phase = if cycle < FLOAT_SECONDS {
            cycle / FLOAT_SECONDS
        } else {
            1.0 - (cycle - FLOAT_SECONDS) / FLOAT_SECONDS
        };
        transform.translation.y = float.base_y + FLOAT_AMPLITUDE_PX * phase;
    }
}

/// Update：开云推进——32 曲线逐帧求值（位置写平移、alpha 写色、
/// m_IsActive 写显隐），按真源时刻触发 SE 日志/名字淡入/收口。
pub(crate) fn tick_unlock(
    time: Res<Time>,
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    mut runs: Query<(Entity, &mut UnlockRun)>,
    mut icons: Query<&mut MapIcon>,
    mut sprites: Query<(&mut Transform, &mut Sprite), Without<NameGlyph>>,
    mut name_glyphs: Query<(&mut Sprite, &NameGlyph)>,
    mut visibilities: Query<&mut Visibility>,
) {
    let Some(data) = data else { return };
    let dt = time.delta_secs();
    for (run_entity, mut run) in runs.iter_mut() {
        run.t += dt;
        let t = run.t;
        let site_id = data.sites[run.site].id;

        // SE 事件（只记日志；音频域）。
        if !run.se_logged && t >= SE_EVENT_SECONDS {
            run.se_logged = true;
            info!(
                "sitemap: unlock site={site_id} SE OnPlaySE(se_unlock_site) @{SE_EVENT_SECONDS:.3}"
            );
        }

        // 逐云：位置是容器系绝对值曲线；alpha = 自身曲线 × 容器组曲线。
        let ct = t.min(CLIP_SECONDS);
        let group = data.curves[run.group_alpha].curve.sample(ct);
        for &(cloud, px, py, pa) in &run.clouds {
            if let Ok((mut transform, mut sprite)) = sprites.get_mut(cloud) {
                transform.translation.x = data.curves[px].curve.sample(ct);
                transform.translation.y = data.curves[py].curve.sample(ct);
                sprite
                    .color
                    .set_alpha(data.curves[pa].curve.sample(ct) * group);
            }
        }
        // flare 双件与 site_mask（遮罩露出底下的站点图——prefab 里它在
        // 云之下、flare_back 之上）。
        if let Ok((_, mut sprite)) = sprites.get_mut(run.flare_back.0) {
            sprite
                .color
                .set_alpha(data.curves[run.flare_back.1].curve.sample(ct));
        }
        if let Ok((_, mut sprite)) = sprites.get_mut(run.site_mask.0) {
            sprite
                .color
                .set_alpha(data.curves[run.site_mask.1].curve.sample(ct));
        }
        if let Ok((_, mut sprite)) = sprites.get_mut(run.flare_front.0) {
            sprite
                .color
                .set_alpha(data.curves[run.flare_front.1].curve.sample(ct));
        }
        if let Ok(mut visible) = visibilities.get_mut(run.flare_front.0) {
            *visible = if data.curves[run.flare_active].curve.sample(ct) >= 0.5 {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }

        // 图标图与阴影：3.0s 淡入（ease 3 = OutSine；真源 CanvasGroup 的
        // 按钮组淡入）。
        let open_factor = ease_out_sine((t / OPEN_FADE_SECONDS).min(1.0));
        for e in [run.icon_image, run.icon_shadow] {
            if let Ok((_, mut sprite)) = sprites.get_mut(e) {
                sprite.color.set_alpha(open_factor);
            }
        }

        // 3.5s：open_cloud 关闭、名字淡入起算、浮动重起（真源序列尾的
        // PlayFloatingAnimation）。
        if run.name_t.is_none() && t >= OPEN_WAIT_SECONDS {
            run.name_t = Some(0.0);
            commands.entity(run.open_root).despawn();
            commands
                .entity(run.icon)
                .insert(IconFloat { t: 0.0, base_y: run.base_y });
            info!(
                "sitemap: unlock site={site_id} open_cloud closed + name fade begin + float restart @{t:.3}"
            );
        }
        // 名字淡入写点（1.5s OutSine）。
        if let Some(nt) = run.name_t {
            let factor = ease_out_sine((nt / NAME_FADE_SECONDS).min(1.0));
            for &glyph in &run.name_glyphs {
                if let Ok((mut sprite, marker)) = name_glyphs.get_mut(glyph) {
                    sprite.color = marker.base.with_alpha(marker.base.alpha() * factor);
                }
            }
            run.name_t = Some(nt + dt);
        }

        // 收口：图标转解锁态，运行实体撤。
        if t >= OPEN_WAIT_SECONDS + NAME_FADE_SECONDS {
            if let Ok(mut icon) = icons.get_mut(run.icon) {
                icon.unlocked = true;
            }
            if let Ok((_, mut sprite)) = sprites.get_mut(run.icon_image) {
                sprite.color.set_alpha(1.0);
            }
            commands.entity(run_entity).despawn();
            info!(
                "sitemap: unlock site={site_id} done @{t:.3} → unlocked"
            );
        }

        // 周期采样日志（每朵云的曲线采样值现算，可对着键值复算）。
        if t - run.last_sample >= UNLOCK_SAMPLE_PERIOD {
            run.last_sample = t;
            let clouds: Vec<String> = run
                .clouds
                .iter()
                .enumerate()
                .map(|(i, &(cloud, _, _, _))| {
                    match sprites.get(cloud) {
                        Ok((transform, sprite)) => format!(
                            "c{}=({:.1},{:.1},a{:.2})",
                            i + 1,
                            transform.translation.x,
                            transform.translation.y,
                            sprite.color.alpha()
                        ),
                        Err(_) => format!("c{}=?", i + 1),
                    }
                })
                .collect();
            let flare_a = sprites
                .get(run.flare_front.0)
                .map(|(_, s)| s.color.alpha())
                .unwrap_or(0.0);
            let flare_vis = visibilities
                .get(run.flare_front.0)
                .map(|v| *v != Visibility::Hidden)
                .unwrap_or(false);
            let flare_back = sprites
                .get(run.flare_back.0)
                .map(|(_, s)| s.color.alpha())
                .unwrap_or(0.0);
            let mask = sprites
                .get(run.site_mask.0)
                .map(|(_, s)| s.color.alpha())
                .unwrap_or(0.0);
            let icon_a = sprites
                .get(run.icon_image)
                .map(|(_, s)| s.color.alpha())
                .unwrap_or(0.0);
            info!(
                "sitemap: unlock site={site_id} t={t:.3} group={group:.2} clouds [{}] flare_f=(a{flare_a:.2},v{flare_vis}) flare_b=a{flare_back:.2} mask=a{mask:.2} icon=a{icon_a:.2}",
                clouds.join(" ")
            );
        }
    }
}

/// Update：M 键切换地图显隐。
pub(crate) fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut roots: Query<&mut Visibility, With<SitemapRoot>>,
) {
    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    for mut visible in &mut roots {
        *visible = if *visible == Visibility::Hidden {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        info!("sitemap: map visible={}", *visible != Visibility::Hidden);
    }
}

/// Update：天气钮跟随当前现象（天气域的只读出口；天气插件缺席时保持
/// 默认行——资源由 audio::install 兜底，Option 读法照旧）。档名不在主表
/// （如庆典园地档）时不更新——真源同形。
pub(crate) fn refresh_weather(
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    text: Option<Res<SitemapText>>,
    weather: Option<Res<CurrentPhenomenon>>,
    mut panels: Query<(Entity, &mut WeatherPanel)>,
) {
    let (Some(data), Some(text)) = (data, text) else { return };
    let Some(asset) = weather.map(|w| w.0.clone()) else { return };
    let Ok((panel_entity, mut panel)) = panels.single_mut() else { return };
    if panel.last_asset == asset {
        return;
    }
    let row = PHENOMENA_ROWS.iter().position(|r| r.asset == asset);
    match row {
        Some(row) if row != panel.row => {
            // 撤的是 icon 与两个行父：字形是行父的子实体，跟着一次撤。
            // 旧版把行父与字形混在一列各撤一遍，字形已被父级连带撤掉，
            // 第二次 despawn 落在死实体上——天气切换 × 面板重建每次固定
            // 刷出一批「despawn 落空」WARN（实测 native 130/wasm
            // 123，与本处字符数逐次吻合），即此处。
            commands.entity(panel.icon).despawn();
            for row_entity in &panel.rows {
                commands.entity(*row_entity).despawn();
            }
            let (icon_at, jp_text_at, en_row_at) = phenomena_slots(&data, &text, row);
            let icon = spawn_phenomena_icon(&mut commands, &data, panel_entity, row, icon_at);
            let rows = spawn_phenomena_text(&mut commands, &text, panel_entity, row, jp_text_at, en_row_at);
            *panel = WeatherPanel {
                icon,
                rows,
                row,
                last_asset: asset.clone(),
            };
            let p = &PHENOMENA_ROWS[row];
            info!(
                "sitemap: phenomena id={} en={} jp_len={} icon=icon_{}",
                p.id,
                p.en,
                p.jp.chars().count(),
                p.icon
            );
        }
        Some(_) => {
            panel.last_asset = asset;
        }
        None => {
            info!("sitemap: phenomena asset={asset} no master row — panel unchanged");
            panel.last_asset = asset;
        }
    }
}

/// Update：周期状态行（盘面可见性）。
pub(crate) fn report(
    icons: Query<&MapIcon>,
    entries: Query<(), With<IconIn>>,
    floats: Query<(), With<IconFloat>>,
    runs: Query<&UnlockRun>,
    panels: Query<&WeatherPanel>,
) {
    let total = icons.iter().count();
    let locked = icons.iter().filter(|i| !i.unlocked).count();
    let phenomena = panels
        .single()
        .map(|p| PHENOMENA_ROWS[p.row].id)
        .unwrap_or(-1);
    info!(
        "sitemap: icons={total} locked={locked} unlocked={} entries={} floats={} unlock_runs={} phenomena_id={phenomena}",
        total - locked,
        entries.iter().count(),
        floats.iter().count(),
        runs.iter().count()
    );
}
