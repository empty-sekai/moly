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
//! - Tap on a released site (`SiteMapIcon.OnClickButtonFeedback` →
//!   `ScreenLayerMysekaiSiteMap.OnPushSiteMapIcon`): the current site backs
//!   the screen (`ScreenManager.BackUIScreen`, a [`LayerCommand::Pop`]); the
//!   festival garden first asks `GetMasterBirthdayPartiesInSession` (the
//!   party master filtered at the server clock, [`crate::birthday`]) and with
//!   none in session shows `ShowCommon1ButtonDialog` and stays; any other
//!   site goes to `MysekaiUtility.ChangeSiteProcessAsync`, which has no
//!   admission of its own (same site: nothing; else `ChangeSite`). The request
//!   goes to [`crate::site`] as it is: the site loader decides whether it can
//!   load the site, and says so by name when it cannot.
//! - Icon images (`SetImage`): `img_map_site_{id}` · `img_map_site_line`
//!   · `img_map_shadow` from the site-map texture package; sizes from the
//!   screen layer (button: home 842×664, others 522×420; line 466×420;
//!   shadow 352×208 @ (0,-162)). The home icon's `site_shadow` node is
//!   inactive in the prefab and `HomeSiteMapIcon` never activates it, so
//!   home draws no shadow.
//! - Icon hierarchy (the prefab's): icon root (placed at its position, the
//!   CanvasGroup the in-fade writes) → `ButtonGroup` (the in-slide) →
//!   `ButtonAndCloundRoot` (the floating) → the button (its own CanvasGroup,
//!   the open fade; the line and the current-location badge are its
//!   children, so they show with it only) and the two clouds, which
//!   `SetupHideCloud` and `OpenSiteAsync` instantiate there. The shadow, the
//!   name and the release balloon hang off the icon root: they fade in with
//!   the icon but neither slide nor float.
//! - `ResetIconState` puts the root's CanvasGroup at 0 and deactivates the
//!   button, the name, the line, the badge and the cloud; `ShowSiteIcon`
//!   then activates the button and the name (released) or the cloud
//!   (locked).
//! - Released or locked is client logic over a server value
//!   (`SiteMapInAnimation` + `SiteIconIn`): the rank releases of type
//!   `mysekai_site_level` with rank ≤ `UserMysekaiGamedata.mysekaiRank`, and a
//!   site is released when one of them names its first site level. The rows
//!   are the player-data catalog's; the rank is the server model's client
//!   copy. With no rank yet (the server model not installed) the map refuses
//!   to decide and says so; it does not assume released.
//! - A locked site shows its cloud (`SiteMapHideCloud`); tapping the cloud
//!   shows the release balloon (`UIPartsReleaseSiteBalloon.Setup(description)`:
//!   `string.Format(WORD_SITE_RELEASE, unlock rank)`, `Show` = alpha 0→1 in
//!   0.1 s), and a tap anywhere else hides it (alpha 1→0 in 0.1 s). A cloud tap
//!   never opens the site: the open animation (`OpenSiteAsync`) runs from
//!   `PlaySiteOpenDirection`, on the harvest-site release topic.
//! - Opening the map (`OnInitComponent`, on the manager's hook): every icon is
//!   reset and `SiteMapInAnimation` runs again: the home icon and the
//!   festival garden (at the secret position) at once, the others in a random
//!   order, each after `Delay(k × 0.04 s)` (k = 1, 2, …) following the
//!   previous one; then `se_open_map`. `SiteIconIn` shows the current-location
//!   badge on the icon of the current site only.
//! - `OpenSiteAsync` (JP 6.8.1): floating stops, the cloud goes, the button and
//!   line show, the button group fades 0→1 in 3.0 s (ease 3 = OutSine), the
//!   animator's `isOpen` trigger starts `clip_open_cloud` (32 curves, 2.2 s;
//!   its 0.7 s event `OnPlaySE("se_unlock_site")` reaches
//!   `CommandAnimator.OnPlaySE` → `PlaySEOneShot`), then after `Delay(3.0 s)`
//!   the name shows and fades 0→1 in 1.5 s (OutSine), `open_cloud` is
//!   deactivated and floating restarts.
//! - In-animation (`PlayInAnimation`): `ButtonGroup` from y+80 back to 0 in
//!   0.5 s with the serialized `_InEase` curve, joined by the icon's
//!   CanvasGroup 0→1 in 0.5 s with the tween default ease (OutQuad in the JP
//!   tween settings); then `PlayFloatingAnimation` (`ButtonAndCloundRoot` to
//!   y 30 in 3.0 s, infinite yoyo, serialized `_siteIconIdleEase`). The two
//!   curves are serialized on the icon behaviours, which no root carries:
//!   the slide takes OutQuad and the floating linear, named.
//! - Weather panel (`SiteMapPhenomenaView`): pinned top-centre (anchor
//!   (0.5,1), pivot the same, @ (0,8), 512×128); a phenomenon without a master
//!   row leaves it as it is, as in the source. The rows are
//!   [`crate::sitemap_phenomena::PHENOMENA_ROWS`]. The weather button
//!   (`_weatherButton`, the forecast dialog) is not drawn.
//!
//! Blur: none, as in the source. `ScreenManager.EnableBlur` returns unless the
//! current scene is in its `EnableBlurSceneList`, which its static
//! constructor fills with OutGame and MusicScoreMaker only; MySekai is not in
//! it, so no screen or dialog blurs in MySekai. The only blur object in the
//! site-map packages is an inactive `UIBlurLayerManager(Clone)` under the
//! `hide_cloud` prefab (listed among its inactive nodes), which never draws.
//!
//! Named gaps (not drawn here):
//! - the cloud particles (a `UIParticle` per cloud, 22 in the packages) wait
//!   for the UIParticle consumer, which waits on the particle host;
//! - the current-location badge (`badge_here_pk` in the shared UI atlas, with
//!   a `TweenRotation`), the balloon body (`balloon_taphint_*`) and the white
//!   base image of the background are serialized in the screen layer prefab:
//!   no root carries that layout yet, so the badge is decided and logged, the
//!   balloon draws its text only, and the background is not drawn;
//! - the background colours (`ChangeSiteMapColor`, the phenomenon's row of
//!   `mysekaiPhenomenaBackgroundColors`: base, gradient, ground highlight
//!   (`img_map_shadow` at 1760×1040 @ (0,668)), two corners, sphere) are in no
//!   root yet;
//! - the no-party dialog (`ShowCommon1ButtonDialog("MSG_NOTHING_BIRTHDAY_DIALOG_BODY")`)
//!   has no Common1 body view: the tap logs it and stays;
//! - `open_cloud`'s animator enters `clip_open_cloud` through a transition
//!   with exit time 0.75 and a 0.25 s blend; the curves here start at the
//!   trigger;
//! - the unlock direction's dark overlay (`open_direction_background`, 0→1 in
//!   0.4 s, 0.6 s after `OpenSiteAsync`) and the harvest-site release topic
//!   that starts it: the product raises no such topic, so the open animation
//!   runs from a verification hook only;
//! - the button press animation (`PlayButtonAnimationAsync`: scale 1.25,
//!   0.3 s, the serialized `_siteIconScaleEase`) is not played.

use bevy::asset::{AssetPath, LoadState, RenderAssetUsages};
use bevy::camera::visibility::RenderLayers;
use bevy::image::Image;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use moly_assets::json::JsonAsset;
use moly_law::ui::dotween::Ease;
use std::collections::HashMap;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::canvas::{CANVAS_REF_H, CANVAS_REF_W};
use crate::sitemap_phenomena::{DEFAULT_PHENOMENA_ID, PHENOMENA_ROWS};
use crate::source_curve::Curve;
use crate::ui_layers::{LayerCommand, MenuScreenType, ScreenHook, ScreenLayerEvent};
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
// 画布上（ScreenManager.SetUpScreenResolution 的匹配律，见
// [`crate::canvas`] 的根画布），底图/图标/面板全部按 canvas 单位直读直用。

/// `PlayInAnimation`: `ButtonGroup` starts at local y+80 and moves back in
/// 0.5 s; the icon's CanvasGroup fades 0→1 in the same 0.5 s.
const IN_SLIDE_PX: f32 = 80.0;
const IN_SECONDS: f32 = 0.5;
/// `SiteMapInAnimation`: the k-th of the shuffled icons waits
/// `Delay(k × 0.04 s)` after the previous icon's `SiteIconIn` (the step is
/// the single-precision literal the loop multiplies).
const IN_DELAY_STEP: f32 = 0.04;

/// `PlayFloatingAnimation`: `ButtonAndCloundRoot` to (0,30,0) in 3.0 s, loops
/// -1, yoyo. Its ease is the serialized `_siteIconIdleEase` curve, which no
/// root carries: the floating here is linear (named).
const FLOAT_AMPLITUDE_PX: f32 = 30.0;
const FLOAT_SECONDS: f32 = 3.0;

/// `OpenSiteAsync` (JP 6.8.1): the button group fades 0→1 in 3.0 s (ease 3 =
/// OutSine), then `Delay(3.0 s)`; after it the name fades 0→1 in 1.5 s
/// (OutSine), `open_cloud` is deactivated and floating restarts.
const OPEN_FADE_SECONDS: f32 = 3.0;
const OPEN_WAIT_SECONDS: f32 = 3.0;
const NAME_FADE_SECONDS: f32 = 1.5;
/// `clip_open_cloud`'s stop time.
const CLIP_SECONDS: f32 = 2.2;
/// The clip's AnimationEvent: `OnPlaySE("se_unlock_site")` at 0.7 s (checked
/// against the clip's events when the data is parsed).
const SE_EVENT_SECONDS: f32 = 0.7;
/// The cue that event plays, and the one `OnInitComponent` plays.
const SE_UNLOCK_SITE: &str = "se_unlock_site";
const SE_OPEN_MAP: &str = "se_open_map";
/// `UIPartsCommonBalloon.Show` / `Hide`: `DOFade(1 or 0, 0.1)`, default ease.
const BALLOON_FADE_SECONDS: f32 = 0.1;
/// `UIPartsCommonBalloon.mainText` font size and colour in the prefab
/// (28, white); the balloon's own position is the screen layer's
/// (`UIPartsReleaseSiteBalloon` @ (-2,234) under the icon root).
const BALLOON_TEXT_SIZE: f32 = 28.0;
const BALLOON_ANCHOR: Vec2 = Vec2::new(-2.0, 234.0);
/// `MysekaiRankReleaseType.mysekai_site_level` as the master names it.
const SITE_LEVEL_RELEASE: &str = "mysekai_site_level";
/// The balloon's wording key (`UIPartsReleaseSiteViewData`).
const WORD_SITE_RELEASE: &str = "WORD_SITE_RELEASE";
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
    /// The player-data catalog: its `mysekaiSiteLevels` and
    /// `mysekaiRankReleases` tables decide released or locked.
    catalog: Handle<JsonAsset>,
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
        screen: server.load::<JsonAsset>(json_path("site/sitemap/screen_layer/screen_layer.json")),
        sites: server.load::<JsonAsset>(json_path("site/sites.json")),
        catalog: server.load::<JsonAsset>(json_path("fixture-models/player-data.json")),
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
                    .unwrap_or_else(|| panic!("sitemap：{what} 的 {key}[{i}]（{part}）缺值"))
                    as f32
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

/// The two catalog tables `SiteMapInAnimation` and `SiteIconIn` read, in
/// master order: `mysekaiSiteLevels` (id, site id) and the rank releases of
/// type `mysekai_site_level` (rank, the site level id they release).
struct SiteReleases {
    levels: Vec<(i32, i32)>,
    releases: Vec<(i32, i32)>,
}

impl SiteReleases {
    fn parse(text: &str) -> SiteReleases {
        let document: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|err| panic!("sitemap: player-data.json is not JSON: {err}"));
        let rows = |name: &str| -> Vec<serde_json::Value> {
            document["tables"][name]
                .as_array()
                .unwrap_or_else(|| panic!("sitemap: player-data.json has no {name} table"))
                .clone()
        };
        let int = |row: &serde_json::Value, field: &str, table: &str| -> i32 {
            row[field]
                .as_i64()
                .unwrap_or_else(|| panic!("sitemap: a {table} row has no integer {field}"))
                as i32
        };
        let levels = rows("mysekaiSiteLevels")
            .iter()
            .map(|row| {
                (
                    int(row, "id", "mysekaiSiteLevels"),
                    int(row, "mysekaiSiteId", "mysekaiSiteLevels"),
                )
            })
            .collect();
        let releases = rows("mysekaiRankReleases")
            .iter()
            // The master column carries the source's own misspelling.
            .filter(|row| row["mysekaiRankRelaseType"].as_str() == Some(SITE_LEVEL_RELEASE))
            .map(|row| {
                (
                    int(row, "mysekaiRank", "mysekaiRankReleases"),
                    int(row, "externalId", "mysekaiRankReleases"),
                )
            })
            .collect();
        SiteReleases { levels, releases }
    }

    /// `GetMysekaiSiteLevels(siteId)[0].id`: the site's first level row in
    /// master order.
    fn first_level(&self, site_id: i32) -> Option<i32> {
        self.levels
            .iter()
            .find(|(_, site)| *site == site_id)
            .map(|(id, _)| *id)
    }

    /// `SiteIconIn`'s `isSiteRelease`: a site-level release with rank ≤ the
    /// user's rank (`SiteMapInAnimation`'s filter) names the site's first
    /// level.
    fn released(&self, site_id: i32, user_rank: i32) -> bool {
        let Some(level) = self.first_level(site_id) else {
            return false;
        };
        self.releases
            .iter()
            .any(|(rank, external)| *rank <= user_rank && *external == level)
    }

    /// `MysekaiUtility.GetMysekaiSiteUnlockLevel`: the rank of the release
    /// row of the site's first level (-1 when either row is missing, as the
    /// source returns after logging).
    fn unlock_rank(&self, site_id: i32) -> Option<i32> {
        let level = self.first_level(site_id)?;
        self.releases
            .iter()
            .find(|(_, external)| *external == level)
            .map(|(rank, _)| *rank)
    }
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
    releases: SiteReleases,
    screen: ScreenLayout,
    textures: HashMap<String, Handle<Image>>,
    /// 贴图装载是否已全部到位（铺装前的闩）。
    textures_pending: bool,
    /// The prefab document, kept for the UIParticle hosts under the clouds.
    prefab: Handle<JsonAsset>,
    /// The map root (the canvas-space root the UIParticle draws go under),
    /// once spawned.
    map_root: Option<Entity>,
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
        &request.catalog,
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
    let prefab_text = jsons
        .get(&request.prefab)
        .expect("prefab.json 已 Loaded")
        .0
        .clone();
    let animation_text = jsons
        .get(&request.animation)
        .expect("animation.json 已 Loaded")
        .0
        .clone();
    let texture_text = jsons
        .get(&request.texture)
        .expect("texture.json 已 Loaded")
        .0
        .clone();
    let screen_text = jsons
        .get(&request.screen)
        .expect("screen_layer.json 已 Loaded")
        .0
        .clone();
    let sites_text = jsons
        .get(&request.sites)
        .expect("sites.json 已 Loaded")
        .0
        .clone();
    let catalog_text = jsons
        .get(&request.catalog)
        .expect("player-data.json 已 Loaded")
        .0
        .clone();

    // ---- prefab.json：按根分片出布局与贴图 ----
    let prefab: serde_json::Value = serde_json::from_str(&prefab_text)
        .unwrap_or_else(|err| panic!("sitemap：prefab.json 不是合法 JSON：{err}"));
    let roots: Vec<(String, usize)> = prefab["roots"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：prefab.json 缺 roots 数组"))
        .iter()
        .map(|r| {
            (
                r["name"]
                    .as_str()
                    .unwrap_or_else(|| panic!("sitemap：roots 行缺 name"))
                    .to_owned(),
                r["nodes"]
                    .as_u64()
                    .unwrap_or_else(|| panic!("sitemap：roots 行缺 nodes 计数"))
                    as usize,
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
            at,
            rect_instances.len(),
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
    let animation: serde_json::Value = serde_json::from_str(&animation_text)
        .unwrap_or_else(|err| panic!("sitemap：animation.json 不是合法 JSON：{err}"));
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
            "m_AnchoredPosition.x"
            | "m_AnchoredPosition.y"
            | "m_Color.a"
            | "m_IsActive"
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
        curves.push(CurveBinding {
            node,
            attribute,
            curve,
        });
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
    assert!(
        se_ok,
        "sitemap：clip_open_cloud 的 SE 事件与 0.7s OnPlaySE(se_unlock_site) 不符"
    );

    // ---- texture.json：sRGB 变体表 ----
    let texture_manifest: serde_json::Value = serde_json::from_str(&texture_text)
        .unwrap_or_else(|err| panic!("sitemap：texture.json 不是合法 JSON：{err}"));
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
    let sites_value: serde_json::Value = serde_json::from_str(&sites_text)
        .unwrap_or_else(|err| panic!("sitemap：sites.json 不是合法 JSON：{err}"));
    let site_rows = sites_value["sites"]
        .as_array()
        .unwrap_or_else(|| panic!("sitemap：sites.json 缺 sites 数组"));
    let mut sites: Vec<SiteRow> = site_rows
        .iter()
        .map(|row| {
            let id = row["id"]
                .as_i64()
                .unwrap_or_else(|| panic!("sitemap：站点行缺 id")) as i32;
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
    let releases = SiteReleases::parse(&catalog_text);
    assert!(
        sites.iter().any(|s| s.id == DEFAULT_SITE_ID && s.on_map),
        "sitemap：站点表缺默认站点 {DEFAULT_SITE_ID}（HOME 必须有图标）"
    );

    // ---- screen_layer.json：屏幕层布局真值 ----
    // 图标与位置按下标配对（连接律），再按 siteType 名接主表行；锚点
    // 换算见 [Frame::place]——画布中心在原点，参照 1920×1080。
    let screen: serde_json::Value = serde_json::from_str(&screen_text)
        .unwrap_or_else(|err| panic!("sitemap：screen_layer.json 不是合法 JSON：{err}"));
    let canvas_ref = {
        let canvas = &screen["canvas"]["referenceResolution"];
        let axis = |i: usize| {
            canvas[i].as_f64().unwrap_or_else(|| {
                panic!("sitemap：screen_layer.json 的 canvas.referenceResolution[{i}] 缺失或不是数")
            }) as f32
        };
        (axis(0), axis(1))
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
                icon["siteType"]
                    .as_str()
                    .unwrap_or_else(|| panic!("sitemap：图标行缺 siteType"))
                    .to_owned(),
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
            position: position.place(Vec2::ZERO, Vec2::ZERO).center,
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
        Frame::parse(&prefab_outdoor[name], &format!("iconPrefab.outdoor.{name}"))
    };
    let name_outdoor = prefab_node("DefaultWorldNameBase")
        .place(Vec2::ZERO, Vec2::ZERO)
        .center;
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
    let missing: Vec<&String> = wanted
        .iter()
        .filter(|n| !srgb_names.contains_key(*n))
        .collect();
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
            moly_assets::residency::load_image(
                &server,
                AssetPath::from(format!("moly://site/sitemap/texture/{uri}")),
            ),
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
    info!(
        "sitemap: release table: {} site-level rank releases; first site level per map site [{}]",
        releases.releases.len(),
        sites
            .iter()
            .filter(|s| s.on_map)
            .map(|s| format!(
                "{}={:?}@rank {:?}",
                s.id,
                releases.first_level(s.id),
                releases.unlock_rank(s.id)
            ))
            .collect::<Vec<_>>()
            .join(" ")
    );
    commands.insert_resource(SitemapData {
        layout,
        raw_images,
        curves,
        curve_index,
        sites,
        releases,
        screen: screen_layout,
        textures,
        textures_pending: true,
        prefab: request.prefab.clone(),
        map_root: None,
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
    /// The root's `WORD_SITE_RELEASE` wording (the release balloon's format),
    /// baked into the atlas; None when the root's wordings lack it.
    release_format: Option<String>,
}

/// 烘字符集：可打印 ASCII + 现象名（JP+EN）+ 站点名 + the release
/// balloon's wording (its argument is a rank: ASCII digits). 缺字形按码点
/// 具名 panic（fail-closed：静默空格是看不见的错值）。
fn bake_text_atlas(
    images: &mut Assets<Image>,
    site_names: &[String],
    release_format: Option<String>,
) -> SitemapText {
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
    if let Some(format) = &release_format {
        push(format, &mut chars);
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
        let (w, h) = (
            glyph.placement.width as usize,
            glyph.placement.height as usize,
        );
        if w == 0 || h == 0 {
            rasters.push(Raster {
                ch,
                advance,
                left: 0,
                top: 0,
                width: 0,
                height: 0,
                data: Vec::new(),
            });
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
        let hex: Vec<String> = missing
            .iter()
            .map(|ch| format!("U+{:04X}", *ch as u32))
            .collect();
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
            cells.insert(
                r.ch,
                GlyphCell {
                    rect,
                    advance: r.advance,
                },
            );
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
        cells.insert(
            r.ch,
            GlyphCell {
                rect,
                advance: r.advance,
            },
        );
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
        release_format,
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

/// One site icon: the icon root (placed at its position) and the entities the
/// open-time state and the taps write. The three alphas are the prefab's
/// CanvasGroups: the icon root's (`_canvasGroup`, the in-fade), the button's
/// (image and line, the open fade) and the name's (the name fade).
#[derive(Component)]
pub(crate) struct MapIcon {
    /// Index into `sites`.
    site: usize,
    /// `SiteIconIn`'s `isSiteRelease` at the last open (false until the map
    /// has opened once).
    released: bool,
    /// `ShowSiteIcon` has run since the last reset: the button (released) or
    /// the cloud (locked) is active.
    active: bool,
    /// `SetHereIcon`: the icon of the current site.
    here: bool,
    /// The button image (`_mysekaiCustomButton`); the line is its child.
    image: Entity,
    /// `_siteNameObject` (the name row's parent; glyphs are its children).
    name: Entity,
    /// `ButtonGroup`: the in-slide moves it.
    button_group: Entity,
    /// `ButtonAndCloundRoot`: the floating moves it.
    button_and_cloud: Entity,
    /// The hide cloud (`SiteMapHideCloud`), instantiated under
    /// `ButtonAndCloundRoot`.
    cloud: Entity,
    /// `UIPartsReleaseSiteBalloon`.
    balloon: Entity,
    group_alpha: f32,
    button_alpha: f32,
    name_alpha: f32,
}

/// The in-animation of one icon after an open: `SiteIconIn` at `start`
/// seconds after the open, then `PlayInAnimation` for 0.5 s.
#[derive(Component)]
pub(crate) struct IconIn {
    start: f32,
    t: f32,
}

/// `PlayFloatingAnimation`'s tween on `ButtonAndCloundRoot`: from the local y
/// it starts at to 30, yoyo, forever.
#[derive(Component)]
pub(crate) struct IconFloat {
    t: f32,
    from: f32,
}

/// Which CanvasGroups a sprite of an icon sits under (besides the root's).
#[derive(Clone, Copy, PartialEq, Eq)]
enum PartLayer {
    Root,
    Button,
    Name,
}

/// A sprite under an icon's CanvasGroups: its alpha is `base_alpha` times the
/// groups'.
#[derive(Component)]
pub(crate) struct IconPart {
    site: usize,
    base_alpha: f32,
    layer: PartLayer,
}

/// `UIPartsReleaseSiteBalloon` of one icon: its CanvasGroup alpha and the
/// running fade (`Show` / `Hide`, 0.1 s each, from the current alpha).
#[derive(Component)]
pub(crate) struct ReleaseBalloon {
    site: usize,
    /// `blocksRaycasts`: true from `Show` until `Hide`'s fade has ended.
    shown: bool,
    alpha: f32,
    /// (from, to, elapsed).
    fade: Option<(f32, f32, f32)>,
    /// Half the size of the text row (canvas units, before the root scale):
    /// the balloon's rect for `CheckClose`.
    half: Vec2,
    glyphs: Vec<Entity>,
}

/// The open animation run (`OpenSiteAsync`); ends after the name fade.
#[derive(Component)]
pub(crate) struct UnlockRun {
    site: usize,
    icon: Entity,
    t: f32,
    se_played: bool,
    /// Time since the name fade began (after `Delay(3.0 s)`).
    name_t: Option<f32>,
    /// Cloud sprites (entity, PosX, PosY, ColorAlpha curve indices).
    clouds: Vec<(Entity, usize, usize, usize)>,
    /// The UIParticle hosts under the clouds (the container's CanvasGroup
    /// alpha is theirs).
    particles: Vec<Entity>,
    /// The cloud container's m_Alpha curve (multiplies every cloud).
    group_alpha: usize,
    flare_front: (Entity, usize),
    flare_back: (Entity, usize),
    site_mask: (Entity, usize),
    /// flare_front's m_IsActive curve.
    flare_active: usize,
    /// The `open_cloud` instance (deactivated after the delay).
    open_root: Entity,
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
        let Some(cell) = text.cells.get(&ch) else {
            continue;
        };
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
        crate::camera::MYSEKAI_CAMERA_MSAA,
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
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let canvas = root_canvas.scale(window);
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
    (layouts, root_canvas): (
        Option<Res<crate::ui_layout::UiLayouts>>,
        Option<Res<crate::canvas::RootCanvas>>,
    ),
) {
    if !roots.is_empty() {
        return;
    }
    let Some(mut data) = data else { return };
    // The wordings arrive with the host sources (the root canvas is set in
    // the same step): the balloon's wording is baked with the names.
    let (Some(layouts), Some(_)) = (layouts, root_canvas) else {
        return;
    };
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
    let release_format = layouts.wordings.get(WORD_SITE_RELEASE).cloned();
    if release_format.is_none() {
        warn!("sitemap: the root's wordings carry no {WORD_SITE_RELEASE}: the release balloon has no text");
    }
    let text = bake_text_atlas(&mut images, &site_names, release_format);

    // Hidden until the screen manager makes the site map the current screen
    // (the menu's site map button or the M key push it).
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
    data.map_root = Some(map_root);

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
    info!("sitemap: overlay spawned hidden; the screen manager shows it while MysekaiSiteMap is the current screen");
    info!(
        "sitemap：底图 bg_map_ground {:.0}x{:.0} @ ({:.1},{:.1})（屏幕层真值；窗口缩放=canvas_scale）",
        ground_at.size.x, ground_at.size.y, ground_at.center.x, ground_at.center.y
    );

    // The icon list in the screen layer's order (HOME first, asserted at
    // parse time). Each icon starts in `ResetIconState`'s state (the
    // CanvasGroup at 0, the button, name and line inactive); the open hook
    // decides released or locked and runs the in-animation.
    let mut site_index_by_type = HashMap::new();
    for (i, site) in data.sites.iter().enumerate() {
        site_index_by_type.insert(site.site_type.as_str(), i);
    }
    let mut home_id = -1;
    let count = data.screen.spots.len();
    for order in 0..count {
        let site_i = site_index_by_type[data.screen.spots[order].site_type.as_str()];
        if order == HOME_SITE_ICON_INDEX {
            home_id = data.sites[site_i].id;
        }
        spawn_icon(&mut commands, &data, &text, map_root, site_i, order);
    }
    assert_eq!(
        home_id, DEFAULT_SITE_ID,
        "sitemap：第 0 号图标必须是 HOME（真源 HOME_SITE_ICON_INDEX=0）"
    );

    spawn_weather_panel(&mut commands, &data, &text, ui_root);

    commands.insert_resource(text);
    info!(
        "sitemap: {count} icons spawned in ResetIconState (HOME index {HOME_SITE_ICON_INDEX} id={home_id}); released or locked is decided when the map opens"
    );
}

/// A child entity at `at` (local z orders siblings).
fn plain_node(commands: &mut Commands, parent: Entity, at: Vec2, z: f32) -> Entity {
    let e = commands
        .spawn((
            Transform::from_xyz(at.x, at.y, z),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(parent).add_child(e);
    e
}

/// One icon in the prefab's hierarchy: the icon root at its position (the
/// authored root scale; the runtime writes the position only) → shadow, name,
/// release balloon, and `ButtonGroup` → `ButtonAndCloundRoot` → button image
/// (→ line), hide cloud. Image names follow `SetImage` (`img_map_site_{id}`).
/// Everything starts as `ResetIconState` leaves it: the root's CanvasGroup at
/// 0, the button, the name and the cloud inactive.
fn spawn_icon(
    commands: &mut Commands,
    data: &SitemapData,
    text: &SitemapText,
    map_root: Entity,
    site_i: usize,
    order: usize,
) {
    let site = &data.sites[site_i];
    let spot = data.spot(site_i);
    let position = spot.position;
    let home = site.site_type == "home_site";
    let icon_root = commands
        .spawn((
            Transform::from_xyz(position.x, position.y, 0.0)
                .with_scale(Vec3::splat(spot.root_scale)),
            Visibility::default(),
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(map_root).add_child(icon_root);

    let icon_texture = format!("img_map_site_{}", site.id);
    let clear = Color::srgba(1.0, 1.0, 1.0, 0.0);

    // site_shadow (a direct child of the icon root, first sibling): 352×208
    // @ (0,-162), the prefab's tint. The home icon's shadow node is inactive.
    let shadow = (!home).then(|| {
        let shadow_at = data.screen.prefab.shadow;
        let tint = data.screen.prefab.shadow_tint;
        let shadow = spawn_image(
            commands,
            data,
            "img_map_shadow",
            shadow_at.size,
            shadow_at.center,
            0.1,
            Color::srgba(tint[0], tint[1], tint[2], 0.0),
        );
        commands.entity(icon_root).add_child(shadow);
        commands.entity(shadow).insert(IconPart {
            site: site_i,
            base_alpha: tint[3],
            layer: PartLayer::Root,
        });
        shadow
    });

    let button_group = plain_node(commands, icon_root, Vec2::ZERO, 0.2);
    let button_and_cloud = plain_node(commands, button_group, Vec2::ZERO, 0.0);

    // The button (`HomeIconButton`, 842×664 home, 522×420 others), inactive
    // after the reset; `site_line` (466×420) is its child, so it shows with
    // the button only.
    let image = spawn_image(
        commands,
        data,
        &icon_texture,
        spot.button,
        Vec2::ZERO,
        0.0,
        clear,
    );
    commands.entity(button_and_cloud).add_child(image);
    commands.entity(image).insert((
        IconPart {
            site: site_i,
            base_alpha: 1.0,
            layer: PartLayer::Button,
        },
        Visibility::Hidden,
    ));
    let line_at = data.screen.prefab.line;
    let line = spawn_image(
        commands,
        data,
        "img_map_site_line",
        line_at.size,
        line_at.center,
        0.1,
        clear,
    );
    commands.entity(image).add_child(line);
    commands.entity(line).insert(IconPart {
        site: site_i,
        base_alpha: 1.0,
        layer: PartLayer::Button,
    });

    let cloud = spawn_cloud_cluster(commands, data, button_and_cloud, site_i);

    // The name row (`DefaultWorldNameBase`): outdoor (0,-180.1), home (0,-196).
    let name_y = if home {
        data.screen.prefab.name_home.y
    } else {
        data.screen.prefab.name_outdoor.y
    };
    let name = plain_node(commands, icon_root, Vec2::new(0.0, name_y), 0.4);
    commands.entity(name).insert(Visibility::Hidden);
    let mut hidden = TEXT_COLOR;
    hidden.set_alpha(0.0);
    for glyph in spawn_text(
        commands,
        text,
        name,
        &site.name,
        SITE_NAME_SIZE,
        hidden,
        false,
    ) {
        commands.entity(glyph).insert(IconPart {
            site: site_i,
            base_alpha: TEXT_COLOR.alpha(),
            layer: PartLayer::Name,
        });
    }

    // The release balloon (@ (-2,234) under the icon root, hidden by
    // `Initialize`); its text is set when a cloud tap shows it.
    let balloon = plain_node(commands, icon_root, BALLOON_ANCHOR, 0.8);
    commands.entity(balloon).insert(ReleaseBalloon {
        site: site_i,
        shown: false,
        alpha: 0.0,
        fade: None,
        half: Vec2::ZERO,
        glyphs: Vec::new(),
    });

    commands.entity(icon_root).insert(MapIcon {
        site: site_i,
        released: false,
        active: false,
        here: false,
        image,
        name,
        button_group,
        button_and_cloud,
        cloud,
        balloon,
        group_alpha: 0.0,
        button_alpha: 1.0,
        name_alpha: 1.0,
    });
    info!(
        "sitemap: site icon id={} type={} tex={} button={:.0}x{:.0} scale={:.2} pos=({:.1},{:.1}) shadow={} order={}",
        site.id,
        site.site_type,
        icon_texture,
        spot.button.x,
        spot.button.y,
        spot.root_scale,
        position.x,
        position.y,
        shadow.is_some(),
        order
    );
}

/// The UIParticle under a cloud (`<cloud>/UI particle`, its systems' bake
/// drawn right after the cloud image and before the next cloud): a host at
/// the node's placement under the cloud sprite. Its own scale stays one (the
/// source drives it); its alpha is written by the screen's CanvasGroup pass.
fn spawn_cloud_particle(
    commands: &mut Commands,
    data: &SitemapData,
    root: &str,
    cloud_path: &str,
    cloud: Entity,
) -> Option<Entity> {
    let node = format!("{cloud_path}/UI particle");
    let Some(map_root) = data.map_root else {
        warn!("sitemap: {root}/{node}: the map root is not spawned; no UI particle");
        return None;
    };
    let Some(l) = data.layout.get(&(root.to_owned(), node.clone())) else {
        warn!("sitemap: prefab has no RectTransform for {root}/{node}; no UI particle");
        return None;
    };
    let host = commands
        .spawn((
            Transform {
                translation: Vec3::new(l.anchored.x, l.anchored.y, 0.0005),
                rotation: l.rotation,
                scale: Vec3::ONE,
            },
            Visibility::Inherited,
            crate::ui_particle::UiParticleHost {
                document: data.prefab.clone(),
                directory: "site/sitemap/prefab".to_owned(),
                root: root.to_owned(),
                node,
                canvas: map_root,
                layer: SITEMAP_LAYER,
                alpha: 0.0,
            },
        ))
        .id();
    commands.entity(cloud).add_child(host);
    Some(host)
}

/// A hide-cloud UIParticle host: its alpha is the icon root's CanvasGroup.
#[derive(Component)]
pub(crate) struct CloudParticle {
    site: usize,
}

/// 铺云簇（hide_cloud 根的构图，全部直读 prefab RectTransform）：根
/// 400×400 → cloud 容器 → 8 朵云（兄弟序即数字序）。
fn spawn_cloud_cluster(
    commands: &mut Commands,
    data: &SitemapData,
    parent: Entity,
    site_i: usize,
) -> Entity {
    let root_layout = data
        .layout
        .get(&("hide_cloud".to_owned(), String::new()))
        .expect("sitemap：prefab 缺 hide_cloud 根的 RectTransform");
    let container_layout = data
        .layout
        .get(&("hide_cloud".to_owned(), "cloud".to_owned()))
        .expect("sitemap：prefab 缺 hide_cloud/cloud 的 RectTransform");
    // `SetupHideCloud` instantiates the cloud under `ButtonAndCloundRoot` at
    // the button's local position (the button sits at the origin); the reset
    // deactivates it until `ShowSiteIcon(false)`.
    let cluster = commands
        .spawn((
            Transform::from_xyz(0.0, 0.0, 0.5),
            Visibility::Hidden,
            RenderLayers::layer(SITEMAP_LAYER),
        ))
        .id();
    commands.entity(parent).add_child(cluster);

    let container = commands
        .spawn((
            Transform::from_xyz(
                container_layout.anchored.x,
                container_layout.anchored.y,
                0.0,
            ),
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
                    color: Color::srgba(
                        image.color[0],
                        image.color[1],
                        image.color[2],
                        image.color[3],
                    ),
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
        commands.entity(cloud).insert(IconPart {
            site: site_i,
            base_alpha: image.color[3],
            layer: PartLayer::Root,
        });
        if let Some(host) = spawn_cloud_particle(commands, data, "hide_cloud", &path, cloud) {
            commands.entity(host).insert(CloudParticle { site: site_i });
        }
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
    cluster
}

/// 现象行的三件位（面板系）：图标中心、JP 文本左起点、EN 行中心。
/// JP 行 =「图标 + 文本」横排、组在行心居中（真源布局组件的组居中律；
/// 间距取具名近似值）。铺装与刷新共用，两路的组宽口径一致。
fn phenomena_slots(data: &SitemapData, text: &SitemapText, row: usize) -> (Vec2, Vec2, Vec2) {
    let layout = &data.screen.phenomena;
    let panel_at = layout.background.center;
    let jp_row_at = layout.jp_row - panel_at;
    let en_row_at = layout.en_row - panel_at;
    let jp_text_width = text_width(text, &PHENOMENA_ROWS[row].jp, PHENOMENA_JP_SIZE);
    let group_width = layout.icon_size.x + PHENOMENA_ICON_GAP_PX + jp_text_width;
    let icon_at =
        jp_row_at - Vec2::new(group_width / 2.0, 0.0) + Vec2::new(layout.icon_size.x / 2.0, 0.0);
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
        spawn_text(
            commands,
            text,
            parent,
            content,
            size,
            TEXT_COLOR,
            origin_left,
        );
    }
    rows
}

// ---------------------------------------------------------------------------
// 开云：点击 → open_cloud 实例 → 32 曲线逐帧
// ---------------------------------------------------------------------------

/// 开云实例的可驱动实体位（曲线下标由 curve_index 绑定，缺曲线即拒）。
struct OpenCloudParts {
    clouds: Vec<(Entity, usize, usize, usize)>,
    /// The UIParticle hosts under the clouds.
    particles: Vec<Entity>,
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
    let mut particles = Vec::new();
    for n in 1..=8usize {
        let node = format!("cloud ({n})");
        let path = format!("cloud/{node}");
        let e = textured(
            commands,
            container,
            "open_cloud",
            &path,
            0.001 * n as f32,
            1.0,
        );
        particles.extend(spawn_cloud_particle(commands, data, "open_cloud", &path, e));
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
        particles,
        group_alpha: curve("cloud", "m_Alpha"),
        flare_front: (flare_front_e, curve("flare_front", "m_Color.a")),
        flare_back: (flare_back_e, curve("flare_back", "m_Color.a")),
        site_mask: (site_mask_e, curve("site_mask", "m_Color.a")),
        flare_active: curve("flare_front", "m_IsActive"),
        root: cluster,
    }
}

/// `OpenSiteAsync` from its start: floating stops, the cloud goes, the button
/// (and its line) shows with its CanvasGroup at 0, `open_cloud` is
/// instantiated under `ButtonAndCloundRoot` and its clip starts.
fn begin_unlock(
    commands: &mut Commands,
    data: &SitemapData,
    icon_entity: Entity,
    icon: &mut MapIcon,
) {
    let site = &data.sites[icon.site];
    commands.entity(icon.button_and_cloud).remove::<IconFloat>();
    commands.entity(icon.cloud).insert(Visibility::Hidden);
    commands.entity(icon.image).insert(Visibility::Inherited);
    icon.released = true;
    icon.button_alpha = 0.0;
    icon.name_alpha = 0.0;

    let open = spawn_open_cloud(commands, data, icon.button_and_cloud);
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
    // Every curve at t = 0 (read from the keys, the start of the check).
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
        site: icon.site,
        icon: icon_entity,
        t: 0.0,
        se_played: false,
        name_t: None,
        clouds: open.clouds,
        particles: open.particles,
        group_alpha: open.group_alpha,
        flare_front: open.flare_front,
        flare_back: open.flare_back,
        site_mask: open.site_mask,
        flare_active: open.flare_active,
        open_root: open.root,
        last_sample: 0.0,
    });
}

/// The canvas offset of an icon's button: the in-slide on `ButtonGroup` plus
/// the floating on `ButtonAndCloundRoot`, both under the root scale.
fn button_offset(icon: &MapIcon, root_scale: f32, transforms: &Query<&Transform>) -> Vec2 {
    let y = |entity: Entity| {
        transforms
            .get(entity)
            .map(|t| t.translation.y)
            .unwrap_or(0.0)
    };
    Vec2::new(
        0.0,
        (y(icon.button_group) + y(icon.button_and_cloud)) * root_scale,
    )
}

/// Half the hide cloud's root rect (400×400 in the prefab): its button.
fn cloud_half(data: &SitemapData) -> Vec2 {
    data.layout
        .get(&("hide_cloud".to_owned(), String::new()))
        .expect("sitemap：prefab 缺 hide_cloud 根的 RectTransform")
        .size
        / 2.0
}

/// Starts a balloon fade from its current alpha (`DOFade`, 0.1 s, the
/// default ease).
fn fade_balloon(balloon: &mut ReleaseBalloon, to: f32) {
    balloon.fade = Some((balloon.alpha, to, 0.0));
    if to > 0.0 {
        balloon.shown = true;
    }
}

/// `UIPartsReleaseSiteBalloon.Setup` + `Show`: the text is
/// `string.Format(WORD_SITE_RELEASE, GetMysekaiSiteUnlockLevel(siteId))`.
fn show_balloon(
    commands: &mut Commands,
    data: &SitemapData,
    text: &SitemapText,
    balloon_entity: Entity,
    balloon: &mut ReleaseBalloon,
) {
    let site = &data.sites[balloon.site];
    let rank = data.releases.unlock_rank(site.id).unwrap_or_else(|| {
        warn!(
            "sitemap: site {} has no site-level rank release for its first level: GetMysekaiSiteUnlockLevel returns -1",
            site.id
        );
        -1
    });
    let Some(format) = text.release_format.as_deref() else {
        info!(
            "sitemap: release balloon site={} not shown: the root's wordings carry no {WORD_SITE_RELEASE}",
            site.id
        );
        return;
    };
    let description = moly_law::text::custom_text_mesh::format_wording(format, &[rank.to_string()])
        .unwrap_or_else(|err| panic!("sitemap: {WORD_SITE_RELEASE}: {err}"));
    for glyph in balloon.glyphs.drain(..) {
        commands.entity(glyph).despawn();
    }
    let mut colour = Color::WHITE;
    colour.set_alpha(balloon.alpha);
    balloon.glyphs = spawn_text(
        commands,
        text,
        balloon_entity,
        &description,
        BALLOON_TEXT_SIZE,
        colour,
        false,
    );
    balloon.half = Vec2::new(
        text_width(text, &description, BALLOON_TEXT_SIZE),
        BALLOON_TEXT_SIZE,
    ) / 2.0;
    fade_balloon(balloon, 1.0);
    info!(
        "sitemap: release balloon site={} Show \"{description}\" (unlock rank {rank})",
        site.id
    );
}

/// One tap on the map, the same path for a real tap and the smoke hook.
/// `canvas` is the tap in reference-canvas units. Returns whether a branch
/// took the tap.
///
/// - A shown balloon hides on any tap outside it (`CheckClose`); a tap inside
///   it stays with it.
/// - A released icon's button (`OnClickButtonFeedback` → `OnPushSiteMapIcon`):
///   the current site backs the screen; the festival garden first asks
///   `GetMasterBirthdayPartiesInSession`; any other site requests the change.
/// - A locked icon's cloud (`SiteMapHideCloud`'s button, the 400×400 root)
///   shows that icon's release balloon.
#[allow(clippy::too_many_arguments)]
fn tap(
    commands: &mut Commands,
    data: &SitemapData,
    text: &SitemapText,
    icons: &Query<(Entity, &MapIcon)>,
    transforms: &Query<&Transform>,
    balloons: &mut Query<(Entity, &mut ReleaseBalloon)>,
    active: Option<&crate::site::SiteActive>,
    birthday: Option<&crate::birthday::BirthdayParties>,
    layer_commands: &mut MessageWriter<LayerCommand>,
    canvas: Vec2,
) -> bool {
    for (_, mut balloon) in balloons.iter_mut() {
        if !balloon.shown {
            continue;
        }
        let spot = data.spot(balloon.site);
        let centre = spot.position + BALLOON_ANCHOR * spot.root_scale;
        let delta = (canvas - centre).abs();
        let half = balloon.half * spot.root_scale;
        if delta.x <= half.x && delta.y <= half.y {
            return true;
        }
        if balloon.fade.map(|(_, to, _)| to) != Some(0.0) {
            fade_balloon(&mut balloon, 0.0);
            info!(
                "sitemap: release balloon site={} Hide (a tap outside it)",
                data.sites[balloon.site].id
            );
        }
    }
    for (_, icon) in icons.iter() {
        if !icon.active || !icon.released {
            continue;
        }
        let spot = data.spot(icon.site);
        let site = &data.sites[icon.site];
        let at = spot.position + button_offset(icon, spot.root_scale, transforms);
        let delta = (canvas - at).abs();
        let half = spot.button * spot.root_scale / 2.0;
        if delta.x > half.x || delta.y > half.y {
            continue;
        }
        let current = active.map(|a| a.site_type.as_str());
        if current == Some(spot.site_type.as_str()) {
            layer_commands.write(LayerCommand::Pop);
            info!(
                "sitemap: tap released site={} type={} at=({:.1},{:.1}) → the current site → BackUIScreen",
                site.id, spot.site_type, canvas.x, canvas.y
            );
            return true;
        }
        if spot.festival {
            let Some(parties) = birthday else {
                info!("sitemap: tap festival_garden before the birthday party master has loaded: no branch");
                return false;
            };
            let now = crate::birthday::now_ms();
            let in_session = parties.labels_in_session(now);
            if in_session.is_empty() {
                info!(
                    "sitemap: tap festival_garden → no birthday party in session ({} rows, now={now}) → ShowCommon1ButtonDialog(MSG_NOTHING_BIRTHDAY_DIALOG_BODY): no Common1 body view, the map stays",
                    parties.len()
                );
                return true;
            }
            info!(
                "sitemap: tap festival_garden → {} birthday parties in session ({}; now={now}) → ChangeSiteProcessAsync",
                in_session.len(),
                in_session.join(", ")
            );
        }
        // `ChangeSiteProcessAsync` has no admission of its own: the site
        // loader decides, and says so by name when it cannot load the site.
        commands.insert_resource(crate::site::SiteChangeRequest(spot.site_type.clone()));
        info!(
            "sitemap: tap released site={} type={} → ChangeSiteProcessAsync → site change request {}",
            site.id, spot.site_type, spot.site_type
        );
        return true;
    }
    let cloud = cloud_half(data);
    for (_, icon) in icons.iter() {
        if !icon.active || icon.released {
            continue;
        }
        let spot = data.spot(icon.site);
        let at = spot.position + button_offset(icon, spot.root_scale, transforms);
        let delta = (canvas - at).abs();
        let half = cloud * spot.root_scale;
        if delta.x > half.x || delta.y > half.y {
            continue;
        }
        let Ok((balloon_entity, mut balloon)) = balloons.get_mut(icon.balloon) else {
            continue;
        };
        show_balloon(commands, data, text, balloon_entity, &mut balloon);
        return true;
    }
    false
}

/// Update: a tap on the site map (a left click while the map is the current
/// screen).
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<SitemapCamera>>,
    mut commands: Commands,
    (data, text): (Option<Res<SitemapData>>, Option<Res<SitemapText>>),
    roots: Query<&Visibility, With<SitemapRoot>>,
    icons: Query<(Entity, &MapIcon)>,
    transforms: Query<&Transform>,
    mut balloons: Query<(Entity, &mut ReleaseBalloon)>,
    runs: Query<(), With<UnlockRun>>,
    (active, birthday): (
        Option<Res<crate::site::SiteActive>>,
        Option<Res<crate::birthday::BirthdayParties>>,
    ),
    leave_dialog: Option<Res<crate::menu_shell::ShellDialogState>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut layer_commands: MessageWriter<LayerCommand>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // A dialog over the map takes the tap (the Dialog slot blocks the layers
    // under it).
    if leave_dialog
        .map(|d| d.blocks_field_input())
        .unwrap_or(false)
    {
        return;
    }
    let (Some(data), Some(text)) = (data, text) else {
        return;
    };
    if !runs.is_empty() {
        return;
    }
    if roots
        .single()
        .map(|v| *v == Visibility::Hidden)
        .unwrap_or(true)
    {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(camera_transform, cursor) else {
        return;
    };
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
    let canvas = world / root_canvas.scale(window);
    tap(
        &mut commands,
        &data,
        &text,
        &icons,
        &transforms,
        &mut balloons,
        active.as_deref(),
        birthday.as_deref(),
        &mut layer_commands,
        canvas,
    );
}

/// Update, verification only: with `MOLY_SITEMAP_AUTOCLICK_SECS` and
/// `MOLY_SITEMAP_AUTOCLICK_SITE` set, after that many seconds the hook opens
/// the map (a `PushUIScreen`, as the menu button does) and then taps the
/// site's icon at its position through [`tap`], the path a real tap takes.
/// It retries each frame until a branch takes the tap (the map not yet open,
/// the icon not yet shown, or an open animation running).
#[allow(clippy::too_many_arguments)]
pub(crate) fn smoke_autoclick(
    time: Res<Time>,
    mut commands: Commands,
    (data, text): (Option<Res<SitemapData>>, Option<Res<SitemapText>>),
    roots: Query<&Visibility, With<SitemapRoot>>,
    icons: Query<(Entity, &MapIcon)>,
    transforms: Query<&Transform>,
    mut balloons: Query<(Entity, &mut ReleaseBalloon)>,
    runs: Query<(), With<UnlockRun>>,
    (active, birthday): (
        Option<Res<crate::site::SiteActive>>,
        Option<Res<crate::birthday::BirthdayParties>>,
    ),
    mut layer_commands: MessageWriter<LayerCommand>,
    mut armed: Local<Option<Option<(f32, String, bool)>>>,
) {
    let (Some(data), Some(text)) = (data, text) else {
        return;
    };
    if armed.is_none() {
        let spec = std::env::var("MOLY_SITEMAP_AUTOCLICK_SECS").ok();
        let site = std::env::var("MOLY_SITEMAP_AUTOCLICK_SITE").ok();
        let parsed = spec
            .as_deref()
            .and_then(|raw| raw.trim().parse::<f32>().ok())
            .filter(|secs| *secs > 0.0)
            .zip(site.filter(|name| !name.trim().is_empty()))
            .map(|(secs, name)| (secs, name.trim().to_owned(), false));
        *armed = Some(parsed);
    }
    let Some(Some((secs, site_type, pushed))) = armed.as_mut().map(|s| s.as_mut()) else {
        return;
    };
    if time.elapsed_secs() < *secs || !runs.is_empty() {
        return;
    }
    let Some(spot) = data
        .screen
        .spots
        .iter()
        .find(|spot| spot.site_type == site_type.as_str())
    else {
        panic!("sitemap：冒烟口要点的站点 {site_type} 不在屏幕层图标表里");
    };
    let hidden = roots
        .single()
        .map(|v| *v == Visibility::Hidden)
        .unwrap_or(true);
    if hidden {
        if !*pushed {
            *pushed = true;
            layer_commands.write(LayerCommand::Push(MenuScreenType::MysekaiSiteMap));
            info!("sitemap: auto click opens the map (PushUIScreen MysekaiSiteMap)");
        }
        return;
    }
    let canvas = spot.position;
    let site_type = site_type.clone();
    let fired = tap(
        &mut commands,
        &data,
        &text,
        &icons,
        &transforms,
        &mut balloons,
        active.as_deref(),
        birthday.as_deref(),
        &mut layer_commands,
        canvas,
    );
    if fired {
        info!(
            "sitemap: auto click (MOLY_SITEMAP_AUTOCLICK) site={site_type} canvas=({:.1},{:.1})",
            canvas.x, canvas.y
        );
        *armed = Some(None);
    }
}

/// Update, verification only: with `MOLY_SITEMAP_AUTO_UNLOCK_SECS` set, after
/// that many seconds the first locked icon that has finished its
/// in-animation runs `OpenSiteAsync` once. In the source the open animation
/// runs from `PlaySiteOpenDirection` on the harvest-site release topic, which
/// the product does not raise; this hook is how a run without input shows the
/// animation. Without the variable it does nothing.
pub(crate) fn auto_unlock(
    time: Res<Time>,
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    mut icons: Query<(Entity, &mut MapIcon), Without<IconIn>>,
    runs: Query<(), With<UnlockRun>>,
    // Outer Some: read once; inner Some: seconds left, None: done or unset.
    mut auto: Local<Option<Option<f32>>>,
) {
    let Some(data) = data else { return };
    if auto.is_none() {
        *auto = Some(
            std::env::var("MOLY_SITEMAP_AUTO_UNLOCK_SECS")
                .ok()
                .and_then(|raw| raw.parse::<f32>().ok())
                .filter(|secs| *secs > 0.0),
        );
    }
    let Some(inner) = auto.as_mut() else { return };
    let Some(remaining) = inner.as_mut() else {
        return;
    };
    *remaining -= time.delta_secs();
    if *remaining > 0.0 || !runs.is_empty() {
        return;
    }
    let Some((entity, mut icon)) = icons
        .iter_mut()
        .find(|(_, icon)| icon.active && !icon.released)
    else {
        // No locked icon shown yet (the map not opened, or every site
        // released): stay at 0 and retry each frame.
        return;
    };
    let site_id = data.sites[icon.site].id;
    begin_unlock(&mut commands, &data, entity, &mut icon);
    info!("sitemap: auto unlock site={site_id} (MOLY_SITEMAP_AUTO_UNLOCK_SECS hook)");
    *inner = None;
}

/// A random order of `items` (`OrderBy(x => Guid.NewGuid())`: every order
/// equally likely).
fn shuffle<T>(items: &mut [T]) {
    for i in (1..items.len()).rev() {
        let mut word = [0u8; 8];
        getrandom::getrandom(&mut word).expect("sitemap: the icon order needs OS entropy");
        let j = (u64::from_le_bytes(word) % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
}

/// Update: the open hook and the in-animation.
///
/// On `OnInitComponent` of the site map: every icon is reset
/// (`ResetIconState`) and `SiteMapInAnimation` schedules `SiteIconIn`: the home
/// icon and the festival garden at once, the others in a random order, the
/// k-th after `Delay(k × 0.04 s)` following the previous one. `SiteIconIn`
/// decides released or locked from the rank releases and the user's
/// `mysekaiRank`, shows the button or the cloud (`ShowSiteIcon`), sets the
/// current-location badge, and plays `PlayInAnimation` (`ButtonGroup` from
/// y+80 to 0 and the root's CanvasGroup 0→1, 0.5 s), then floating.
/// `se_open_map` plays at the open. With no rank in the client user data the
/// open waits and says so once. Last, every icon sprite's alpha is written
/// from its CanvasGroups.
#[allow(clippy::too_many_arguments)]
pub(crate) fn tick_entries(
    time: Res<Time>,
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    mut hooks: MessageReader<ScreenLayerEvent>,
    (client, active): (
        Option<Res<crate::server::ClientUserData>>,
        Option<Res<crate::site::SiteActive>>,
    ),
    mut se: Option<ResMut<SeRequests>>,
    mut icons: Query<(Entity, &mut MapIcon, Option<&mut IconIn>)>,
    mut transforms: Query<&mut Transform>,
    mut parts: Query<(&mut Sprite, &IconPart)>,
    runs: Query<(Entity, &UnlockRun)>,
    mut cloud_particles: Query<(&mut crate::ui_particle::UiParticleHost, &CloudParticle)>,
    // Some(warned) while an open waits for the rank.
    mut pending: Local<Option<bool>>,
) {
    // Read every hook of the frame (a short-circuit would leave the rest for
    // the next frame).
    let opened = hooks.read().fold(false, |opened, e| {
        opened
            || (e.screen == MenuScreenType::MysekaiSiteMap && e.hook == ScreenHook::OnInitComponent)
    });
    if opened {
        *pending = Some(false);
        match se.as_deref_mut() {
            Some(se) => se.0.push(SeRequest {
                owner: None,
                cue: SE_OPEN_MAP.to_owned(),
                class: SeClass::Ui,
                source: "ScreenLayerMysekaiSiteMap.OnInitComponent",
            }),
            None => warn!("sitemap: no SE queue: {SE_OPEN_MAP} not played"),
        }
    }
    let Some(data) = data else { return };
    let mut just_opened = false;
    if let Some(warned) = pending.as_mut() {
        if !icons.is_empty() {
            match client.as_ref().and_then(|c| c.gamedata.mysekai_rank) {
                None => {
                    if !*warned {
                        *warned = true;
                        warn!(
                            "sitemap: the map opened with no mysekaiRank in the client user data: released or locked is not decided; the icons wait for the server model's rank"
                        );
                    }
                }
                Some(rank) => {
                    open_icons(
                        &mut commands,
                        &data,
                        rank,
                        active.as_deref(),
                        &mut icons,
                        &mut transforms,
                        &runs,
                    );
                    *pending = None;
                    just_opened = true;
                }
            }
        }
    }

    let dt = time.delta_secs();
    for (entity, mut icon, entry) in icons.iter_mut() {
        // The new schedule lands with this frame's commands.
        if just_opened {
            break;
        }
        let Some(mut entry) = entry else { continue };
        entry.t += dt;
        if entry.t < entry.start {
            continue;
        }
        if !icon.active {
            // `SiteIconIn`: ShowSiteIcon, SetHereIcon, PlayInAnimation.
            icon.active = true;
            let shown = if icon.released {
                icon.image
            } else {
                icon.cloud
            };
            commands.entity(shown).insert(Visibility::Inherited);
            if icon.released {
                commands.entity(icon.name).insert(Visibility::Inherited);
            }
        }
        let p = entry.t - entry.start;
        let factor = Ease::OutQuad.evaluate(p.min(IN_SECONDS), IN_SECONDS);
        icon.group_alpha = factor;
        if let Ok(mut transform) = transforms.get_mut(icon.button_group) {
            transform.translation.y = IN_SLIDE_PX * (1.0 - factor);
        }
        if p >= IN_SECONDS {
            icon.group_alpha = 1.0;
            commands.entity(entity).remove::<IconIn>();
            let from = transforms
                .get(icon.button_and_cloud)
                .map(|t| t.translation.y)
                .unwrap_or(0.0);
            commands
                .entity(icon.button_and_cloud)
                .insert(IconFloat { t: 0.0, from });
        }
    }

    // The CanvasGroups multiply: root × (button | name).
    let mut alphas: HashMap<usize, (f32, f32, f32)> = HashMap::new();
    for (_, icon, _) in icons.iter() {
        alphas.insert(
            icon.site,
            (icon.group_alpha, icon.button_alpha, icon.name_alpha),
        );
    }
    for (mut sprite, part) in parts.iter_mut() {
        let Some(&(group, button, name)) = alphas.get(&part.site) else {
            continue;
        };
        let layer = match part.layer {
            PartLayer::Root => 1.0,
            PartLayer::Button => button,
            PartLayer::Name => name,
        };
        sprite.color.set_alpha(part.base_alpha * group * layer);
    }
    for (mut host, particle) in cloud_particles.iter_mut() {
        if let Some(&(group, _, _)) = alphas.get(&particle.site) {
            host.alpha = group;
        }
    }
}

/// `ResetIconState` on every icon and the `SiteMapInAnimation` schedule.
fn open_icons(
    commands: &mut Commands,
    data: &SitemapData,
    rank: i32,
    active: Option<&crate::site::SiteActive>,
    icons: &mut Query<(Entity, &mut MapIcon, Option<&mut IconIn>)>,
    transforms: &mut Query<&mut Transform>,
    runs: &Query<(Entity, &UnlockRun)>,
) {
    // A running open animation is replaced by the reset.
    for (run_entity, run) in runs.iter() {
        commands.entity(run.open_root).despawn();
        commands.entity(run_entity).despawn();
    }
    let current = active.map(|a| a.site_type.clone());
    let mut starts: HashMap<Entity, f32> = HashMap::new();
    let mut others: Vec<Entity> = Vec::new();
    for (entity, icon, _) in icons.iter() {
        let spot = data.spot(icon.site);
        if spot.site_type == "home_site" || spot.festival {
            starts.insert(entity, 0.0);
        } else {
            others.push(entity);
        }
    }
    shuffle(&mut others);
    let mut at = 0.0f32;
    for (k, entity) in others.iter().enumerate() {
        at += (k + 1) as f32 * IN_DELAY_STEP;
        starts.insert(*entity, at);
    }
    let mut report = Vec::new();
    for (entity, mut icon, _) in icons.iter_mut() {
        let site = &data.sites[icon.site];
        icon.released = data.releases.released(site.id, rank);
        icon.here = current.as_deref() == Some(site.site_type.as_str());
        icon.active = false;
        icon.group_alpha = 0.0;
        icon.button_alpha = 1.0;
        icon.name_alpha = 1.0;
        for hidden in [icon.image, icon.name, icon.cloud] {
            commands.entity(hidden).insert(Visibility::Hidden);
        }
        if let Ok(mut transform) = transforms.get_mut(icon.button_group) {
            transform.translation.y = 0.0;
        }
        let start = starts[&entity];
        commands.entity(entity).insert(IconIn { start, t: 0.0 });
        report.push(format!(
            "{}:{}@{start:.2}{}",
            site.id,
            if icon.released { "released" } else { "locked" },
            if icon.here { " here" } else { "" }
        ));
    }
    info!(
        "sitemap: OnInitComponent: mysekaiRank {rank}; ResetIconState + SiteMapInAnimation [{}]; {SE_OPEN_MAP}; the current-location badge (badge_here_pk) is not drawn: no root carries the screen layer's layout",
        report.join(" ")
    );
}

/// Update: the floating yoyo (linear: the serialized `_siteIconIdleEase` is in
/// no root) and the balloon fades.
pub(crate) fn tick_floats(
    time: Res<Time>,
    mut floats: Query<(&mut IconFloat, &mut Transform)>,
    mut balloons: Query<&mut ReleaseBalloon>,
    mut glyphs: Query<&mut Sprite, Without<IconPart>>,
) {
    let dt = time.delta_secs();
    for (mut float, mut transform) in floats.iter_mut() {
        float.t += dt;
        let cycle = float.t % (2.0 * FLOAT_SECONDS);
        let phase = if cycle < FLOAT_SECONDS {
            cycle / FLOAT_SECONDS
        } else {
            1.0 - (cycle - FLOAT_SECONDS) / FLOAT_SECONDS
        };
        transform.translation.y = float.from + (FLOAT_AMPLITUDE_PX - float.from) * phase;
    }
    for mut balloon in balloons.iter_mut() {
        let Some((from, to, elapsed)) = balloon.fade else {
            continue;
        };
        let elapsed = elapsed + dt;
        let k = Ease::OutQuad.evaluate(elapsed.min(BALLOON_FADE_SECONDS), BALLOON_FADE_SECONDS);
        balloon.alpha = from + (to - from) * k;
        if elapsed >= BALLOON_FADE_SECONDS {
            balloon.alpha = to;
            balloon.fade = None;
            if to <= 0.0 {
                balloon.shown = false;
            }
        } else {
            balloon.fade = Some((from, to, elapsed));
        }
        let alpha = balloon.alpha;
        for glyph in &balloon.glyphs {
            if let Ok(mut sprite) = glyphs.get_mut(*glyph) {
                sprite.color.set_alpha(alpha);
            }
        }
    }
}

/// Update: the open animation (`OpenSiteAsync`): the 32 curves each frame
/// (positions, alphas, `m_IsActive`), the button fade (3.0 s, OutSine), the
/// clip's `OnPlaySE("se_unlock_site")` at 0.7 s, then after `Delay(3.0 s)` the
/// name (1.5 s, OutSine), `open_cloud` off and floating again.
pub(crate) fn tick_unlock(
    time: Res<Time>,
    mut commands: Commands,
    data: Option<Res<SitemapData>>,
    mut se: Option<ResMut<SeRequests>>,
    mut runs: Query<(Entity, &mut UnlockRun)>,
    mut icons: Query<&mut MapIcon>,
    mut sprites: Query<(&mut Transform, &mut Sprite), Without<IconPart>>,
    nodes: Query<&Transform, Without<Sprite>>,
    mut visibilities: Query<&mut Visibility>,
    mut particle_hosts: Query<&mut crate::ui_particle::UiParticleHost>,
) {
    let Some(data) = data else { return };
    let dt = time.delta_secs();
    for (run_entity, mut run) in runs.iter_mut() {
        run.t += dt;
        let t = run.t;
        let site_id = data.sites[run.site].id;
        let Ok(mut icon) = icons.get_mut(run.icon) else {
            continue;
        };

        // The clip's AnimationEvent → `CommandAnimator.OnPlaySE` →
        // `PlaySEOneShot`.
        if !run.se_played && t >= SE_EVENT_SECONDS {
            run.se_played = true;
            match se.as_deref_mut() {
                Some(se) => se.0.push(SeRequest {
                    owner: None,
                    cue: SE_UNLOCK_SITE.to_owned(),
                    class: SeClass::Ui,
                    source: "clip_open_cloud OnPlaySE",
                }),
                None => warn!("sitemap: no SE queue: {SE_UNLOCK_SITE} not played"),
            }
            info!(
                "sitemap: unlock site={site_id} OnPlaySE({SE_UNLOCK_SITE}) @{SE_EVENT_SECONDS:.3}"
            );
        }

        // Each cloud: its position curves in the container, its alpha curve
        // times the container's m_Alpha.
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
        for &host in &run.particles {
            if let Ok(mut host) = particle_hosts.get_mut(host) {
                host.alpha = group * icon.group_alpha;
            }
        }
        for (entity, curve) in [run.flare_back, run.site_mask, run.flare_front] {
            if let Ok((_, mut sprite)) = sprites.get_mut(entity) {
                sprite.color.set_alpha(data.curves[curve].curve.sample(ct));
            }
        }
        if let Ok(mut visible) = visibilities.get_mut(run.flare_front.0) {
            *visible = if data.curves[run.flare_active].curve.sample(ct) >= 0.5 {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }

        // The button's CanvasGroup: 0→1 in 3.0 s, ease 3 (OutSine).
        icon.button_alpha = ease_out_sine((t / OPEN_FADE_SECONDS).min(1.0));

        // After `Delay(3.0 s)`: the name shows and fades, `open_cloud` goes,
        // floating restarts from where it stopped.
        if run.name_t.is_none() && t >= OPEN_WAIT_SECONDS {
            run.name_t = Some(0.0);
            commands.entity(icon.name).insert(Visibility::Inherited);
            commands.entity(run.open_root).despawn();
            let from = nodes
                .get(icon.button_and_cloud)
                .map(|t| t.translation.y)
                .unwrap_or(0.0);
            commands
                .entity(icon.button_and_cloud)
                .insert(IconFloat { t: 0.0, from });
            info!("sitemap: unlock site={site_id} open_cloud off + name fade + floating @{t:.3}");
        }
        if let Some(nt) = run.name_t {
            icon.name_alpha = ease_out_sine((nt / NAME_FADE_SECONDS).min(1.0));
            run.name_t = Some(nt + dt);
        }

        if t >= OPEN_WAIT_SECONDS + NAME_FADE_SECONDS {
            icon.button_alpha = 1.0;
            icon.name_alpha = 1.0;
            commands.entity(run_entity).despawn();
            info!("sitemap: unlock site={site_id} done @{t:.3}");
            continue;
        }

        // A sample every 0.5 s (each cloud's curve values, checkable against
        // the keys).
        if t - run.last_sample >= UNLOCK_SAMPLE_PERIOD {
            run.last_sample = t;
            let clouds: Vec<String> = run
                .clouds
                .iter()
                .enumerate()
                .map(|(i, &(cloud, _, _, _))| match sprites.get(cloud) {
                    Ok((transform, sprite)) => format!(
                        "c{}=({:.1},{:.1},a{:.2})",
                        i + 1,
                        transform.translation.x,
                        transform.translation.y,
                        sprite.color.alpha()
                    ),
                    Err(_) => format!("c{}=?", i + 1),
                })
                .collect();
            let alpha = |entity: Entity| {
                sprites
                    .get(entity)
                    .map(|(_, s)| s.color.alpha())
                    .unwrap_or(0.0)
            };
            let flare_vis = visibilities
                .get(run.flare_front.0)
                .map(|v| *v != Visibility::Hidden)
                .unwrap_or(false);
            info!(
                "sitemap: unlock site={site_id} t={t:.3} group={group:.2} clouds [{}] flare_f=(a{:.2},v{flare_vis}) flare_b=a{:.2} mask=a{:.2} button=a{:.2}",
                clouds.join(" "),
                alpha(run.flare_front.0),
                alpha(run.flare_back.0),
                alpha(run.site_mask.0),
                icon.button_alpha
            );
        }
    }
}

/// Update: the M key opens the site map (`PushUIScreen`, as the menu's site
/// map button) or closes it (`BackUIScreen`); the screen manager shows the
/// view while the map is the current screen.
pub(crate) fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    manager: Option<Res<crate::ui_layers::ScreenManager>>,
    mut layer_commands: MessageWriter<LayerCommand>,
) {
    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    let Some(manager) = manager else { return };
    if manager.current_screen() == Some(MenuScreenType::MysekaiSiteMap) {
        layer_commands.write(LayerCommand::Pop);
        info!("sitemap: M → BackUIScreen");
    } else {
        layer_commands.write(LayerCommand::Push(MenuScreenType::MysekaiSiteMap));
        info!("sitemap: M → PushUIScreen MysekaiSiteMap");
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
    let (Some(data), Some(text)) = (data, text) else {
        return;
    };
    let Some(asset) = weather.map(|w| w.0.clone()) else {
        return;
    };
    let Ok((panel_entity, mut panel)) = panels.single_mut() else {
        return;
    };
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
            let rows = spawn_phenomena_text(
                &mut commands,
                &text,
                panel_entity,
                row,
                jp_text_at,
                en_row_at,
            );
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

/// Update: a periodic state line.
pub(crate) fn report(
    icons: Query<&MapIcon>,
    entries: Query<(), With<IconIn>>,
    floats: Query<(), With<IconFloat>>,
    runs: Query<&UnlockRun>,
    balloons: Query<&ReleaseBalloon>,
    panels: Query<&WeatherPanel>,
) {
    let total = icons.iter().count();
    let active = icons.iter().filter(|i| i.active).count();
    let released = icons.iter().filter(|i| i.released).count();
    let here: Vec<usize> = icons.iter().filter(|i| i.here).map(|i| i.site).collect();
    let phenomena = panels
        .single()
        .map(|p| PHENOMENA_ROWS[p.row].id)
        .unwrap_or(-1);
    info!(
        "sitemap: icons={total} shown={active} released={released} locked={} here={here:?} entries={} floats={} unlock_runs={} balloons_shown={} phenomena_id={phenomena}",
        total - released,
        entries.iter().count(),
        floats.iter().count(),
        runs.iter().count(),
        balloons.iter().filter(|b| b.shown).count()
    );
}
