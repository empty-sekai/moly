//! 对话窗体：固定窗呈现侧（真源 `ScreenLayerMysekaiTalk` 的宿主消费）。
//!
//! 真源里窗体引擎服务**全部引擎播的对话**：玩家（SD）对话与配对剧情
//! （含多角色 fixture 对话）同走 `ShowTalkWindow`/`HideTalkWindow`/
//! `ShowTextAsync` 这扇窗；tweet 一类世界内文本不经窗体引擎、留在头顶
//! 气泡（[`crate::balloon`]）。两条链共用这扇窗与同一套状态机（开场/
//! 收场/淡变/打字机/点击闩）。本模块只做呈现与状态机半，步进/选取/
//! 驻留在消费律侧（玩家链 [`crate::player_talk`]、配对链 [`crate::talk`]）。
//!
//! **归属构造**：状态机的全部变迁方法（开场/收场/淡变/正文/名字栏/
//! 置闩/耗闩）都要一条 [`TalkSession`] 作保——构造即借用（枚举里握着
//! 会话的 `&mut`），编译器保证独占。`TalkSession` 是两条链的会话枚举
//! （见下），任一链在播都给得出所有权；两链互斥由各自入口守卫保证
//! （配对选取与双注入口都在玩家会话在场时让行，反之亦然），同一帧两
//! 链都在播是缺陷，输入面对
//! 此响亮 panic。置闩的输入面按会话在场门控：无会话的点击不进闩
//! （发起点击被发起消费；无对话期间的点击对窗体惰性）。
//!
//! **布局与样式**：窗体画在对话层的预制体上——几何、贴图、字号、颜色与
//! 字距/词距/行距/边距全部读自该预制体的布局文档（[`LAYOUT`]），本模块不存
//! 它们的值。每个节点的世界矩阵由布局解算给出（锚点、pivot、父链；canvas
//! 单位、原点在 canvas 中心、y 向上），窗体树的根只挂 canvas → 屏幕的缩放。
//! 消费的节点与字段：
//! * **面板**（talkBG 上的 CustomImage）：按 Image 规则（`moly_law::ui::image`）
//!   在解算矩形上铺片——图像类型、精灵的 rect/border/pixelsPerUnit、纹理矩形
//!   与颜色都取自组件，贴图是文档里该精灵的导出图。当前文档是 Sliced、四边
//!   border 恰为源图半边，规则因此只出四角与上下两条横带（横带的源宽为 0，
//!   整条取 border 线上那一列）。
//! * **名字栏**（LabelText）与**正文**（ContentText）：两个 CustomTextMesh 的
//!   矩形与 m_margin、m_fontColor、m_characterSpacing / m_wordSpacing（消费式：
//!   值 × 0.01 × 字号，UI 文本恒正交 ×1）、m_lineSpacing。字号取 TMP 渲染轮
//!   的起始字号（autosize 关即 m_fontSize，开即 m_fontSizeBase 夹进
//!   [m_fontSizeMin, m_fontSizeMax]）。两栏只实现水平居左、垂直居顶，文档给出
//!   别的对齐即响亮拒绝。行量法与摆位按排版律（`moly_law::text`），字距增量在
//!   摆位侧逐字加。
//! * **尾标**（EndSign 下的 AtlasImage `Image`）：同样按 Image 规则铺片
//!   （Simple）。打字机收尾拍才激活；激活期间组上的 Animator 循环播唯一状态
//!   `PageForward`（见 [`PAGE_FORWARD_LENGTH`] 一族）：它写标记的
//!   anchoredPosition (0, y(t)) 与 localScale (1, s(t), 1)，这两项作为该节点的
//!   RectTransform 覆写喂进布局解算，缩放于是绕标记自己的 pivot（底边中点）。
//!   同组的星标序列化为**未激活**，且真源里没有任何写者激活它（见
//!   [`spawn_tree`] 的注释）——不画。
//!
//! **时序**：
//! * **打字机**（`ShowTextAsync`）：第 0 字在 t=0 即现，第 k 字在 50k ms，
//!   末字后再过一拍进 DONE（尾标激活、在播清零）；空文本 t=0 即 DONE。
//!   跳过支把全文一次揭示并**清点击闩**——跳过与放行是两次点击
//!   （打字中点击只跳字，放行要等打字收尾再点）。
//! * **淡变**：窗体 CanvasGroup 序列化初值 α=1（读文档时核对）⇒ 对话开始的
//!   show 步被 `Approximately(α,1)` 短路（即时在场）；hide 步无条件 `DOFade`
//!   0.5s 到 0，被 hide 过再 show 走 0.3s 淡入；缓动取补间库缺省（OutQuad）。
//! * **收口**：层弹出路径不带淡出——对话收尾即时撤窗。
//!
//! * **点击无音效**：点跳与放行都不发 SE（真源点击链全程不调音频，
//!   论证见 [`tick_window`] 的注释）。
//!
//! **具名缺口**（真源有、此处不做）：
//! * 正文的字号搜索：正文 m_enableAutoSizing 开（当前文档 22–44、起始 40），
//!   TMP 渲染轮在全文放得下且字号低于上限时继续往上长、放不下时往下收（见
//!   自动字号律 `moly_law::text::auto_size`）；本模块的行量法不跑这场搜索，
//!   按起始字号定格，最宽行超盒时响亮告警。
//! * 字体资产：两栏各自的 TMP 字体资产（名字栏与正文是两种字重）不画；字形
//!   来自本仓自烘的开源字体图集，两栏同一字重。
//! * 段落距（m_paragraphSpacing——只作用于 U+2029 段落分隔，语料 0 个，恒不
//!   触发）；软换行（m_enableWordWrapping 开，但排版律无软换行且语料行宽全部
//!   在盒内，硬断行即全部形态）。
//! * `PageForward` 的曲线按真源裁件的值编进本文件：UI 布局提取器尚未导出
//!   Animator 控制器与剪辑（文档里 EndSign 只带控制器指针）——提取侧缺口，
//!   不是呈现侧缺口。
//! * 预制体的其余部分不画：右上的菜单键、隐藏 UI 键与截图键（真源在 MySekai
//!   场景里三者都在场）、Auto/Skip 菜单（开场即以 `Close` 状态收起）、自动播放标
//!   （随 auto 模式，开场为关）。
//!
//! 日志口径同对话域：只带 id/字数/步号/量法值——文本内容与名字栏文字
//! 不进日志（语言网关纪律）。

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::image::Image;
use bevy::math::Rect;
use bevy::prelude::*;
use moly_assets::ui_layout::{UiComponent, UiPrefab, UiRect};
use moly_law::text::{auto_size, layout_metrics, LayoutMetrics};
use moly_law::ui::image as image_rule;
use serde_json::Value;

use crate::balloon::{
    walk_glyphs, BalloonArt, GlyphSpot, ASCENT_RATIO, BAKE_PPEM, BALLOON_LAYER, FONT_FAMILY,
    TEXT_SCALE,
};
use crate::gesture::{GestureEvent, GestureState};
use crate::ui_layout::UiLayouts;

mod stage_layout;
pub(crate) use stage_layout::ResponsiveDialogueMetrics;

// ---------------------------------------------------------------------------
// 会话归属
// ---------------------------------------------------------------------------

/// 窗体归属的会话侧：玩家对话与配对剧情两条链共用这扇窗，步进所在的
/// 链不同。变迁方法只要一条 `&mut` 作保——借用即证明独占，同一帧只有
/// 一条链能驱动窗体（两链互斥由各自入口守卫保证，见模块注释；同帧
/// 双链在播是缺陷，输入面 panic）。
pub(crate) enum TalkSession<'a> {
    Player(&'a mut crate::player_talk::PlayerTalkSession),
    Pair(&'a mut crate::talk::ActiveTalk),
}

impl TalkSession<'_> {
    /// 链名（日志用）：归属枚举的读取点——字段构造即借用（独占的证
    /// 明），这里把「哪条链、哪一场」读出来落进日志。
    pub(crate) fn chain_word(&self) -> String {
        match self {
            TalkSession::Player(session) => format!("玩家链 talk {}", session.talk_id()),
            TalkSession::Pair(talk) => format!("配对链 talk {}", talk.talk_id()),
        }
    }
}

// ---------------------------------------------------------------------------
// 窗体在布局文档里的节点（路径后缀，各自在文档里唯一）
// ---------------------------------------------------------------------------

/// 对话层预制体的布局文档。
const LAYOUT: &str = "Talk";
/// 窗体组（层的 `_talkWindowCanvasGroup` 挂在这里：淡变写它的 α）。
const WINDOW_NODE: &str = "ComponentRoot/TalkWindow";
/// 面板。
const PANEL_NODE: &str = "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG";
/// 名字栏（层的 `_labelText`）。
const LABEL_NODE: &str = "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText";
/// 正文（层的 `_contentText`）。
const CONTENT_NODE: &str =
    "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText/ContentText";
/// 尾标组（层的 `_endIconObj`；Animator 挂在这里）。
const END_SIGN_NODE: &str =
    "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText/EndSign";
/// 尾标的可见标记（剪辑绑定的 `Image`）。
const END_MARK_NODE: &str =
    "ComponentRoot/TalkWindow/Window/ContentRoot/talkBG/Content/LabelText/EndSign/Image";

// ---------------------------------------------------------------------------
// 保留的常量（每一项写明为什么不从布局文档读）
// ---------------------------------------------------------------------------

/// 宿主根 Canvas 的参考 pixels-per-unit：Image 规则拿它换算切片边宽
/// （边宽 = border ÷ (精灵 ppu ÷ 参考 ppu)）。值在宿主画布文档里（1），UI 布局
/// 装载器读了它但存为私有、不给读口，这里照该文档抄录；组件自带的参考 ppu
/// 仍优先（见 [`ImageSource::read`]）。
const HOST_REFERENCE_PIXELS_PER_UNIT: f32 = 1.0;
/// 宿主根 Canvas 的 pixel-perfect 旗：Image 规则拿它决定像素对齐矩形。值在
/// 宿主画布文档里（根 Canvas 不 pixel-perfect），同上由装载器私有持有，这里
/// 照抄；窗体父链上覆写 pixel-perfect 的嵌套 Canvas 仍按文档读（见
/// [`pixel_perfect`]）。
const HOST_ROOT_PIXEL_PERFECT: bool = false;

/// `PageForward` 剪辑的值按真源裁件照录（UI 布局提取器尚未导出 EndSign 的
/// Animator 控制器与剪辑，布局文档里只有控制器指针）。
///
/// `PageForward` 剪辑长度（m_StopTime − m_StartTime，65 帧 @60fps），
/// 状态 loop、速度 1、Animator 普通更新模式（缩放时间）。
const PAGE_FORWARD_LENGTH: f32 = 1.0833334;

/// 剪辑的流式曲线段：(键时, [a, b, c, d])，段内 v(dt) = ((a·dt+b)·dt+c)·dt+d，
/// dt = t − 键时——引擎求值流式剪辑就是这个三次式，系数原样照录（f32）。
/// 标记的 localScale.y；x/z 两条曲线四键恒 1（剪辑里存为常值段）。
const PAGE_FORWARD_SCALE_Y: [(f32, [f32; 4]); 4] = [
    (0.0, [9.831591, -3.195267, 0.0, 1.0]),
    (0.21666667, [-29.494772, 9.585801, 0.0, 0.95]),
    (0.43333334, [0.7282659, -0.7100593, 0.0, 1.1]),
    (1.0833334, [0.0, 0.0, 0.0, 1.0]),
];

/// 标记的 m_AnchoredPosition.y（x 是常量曲线 0）。
const PAGE_FORWARD_POS_Y: [(f32, [f32; 4]); 30] = [
    (0.0, [140.62201, -8.922928, -4.668121, 0.0]),
    (0.21666667, [299.05084, -95.50585, 11.269543, 0.0]),
    (0.43333334, [-236.18683, 80.996284, 12.000001, 1.0]),
    (0.65, [0.0, 0.0, 11.6421585, 5.0]),
    (0.6666667, [0.0, 0.0, 8.325556, 5.1940365]),
    (0.68333334, [0.0, 0.0, 5.927788, 5.3327956]),
    (0.7, [0.0, 0.0, 4.008011, 5.431592]),
    (0.71666664, [0.0, 0.0, 2.3674617, 5.498392]),
    (0.73333335, [0.0, 0.0, 0.9002217, 5.53785]),
    (0.75, [0.0, 0.0, -0.45805022, 5.5528536]),
    (
        0.76666665,
        [-8.046564e-04, 1.3410975e-05, -1.7491105, 5.5452194],
    ),
    (0.78333336, [0.0, 0.0, -3.0034475, 5.5160675]),
    (0.8, [0.00321866, -5.3644282e-05, -4.244417, 5.46601]),
    (0.81666666, [0.0, 0.0, -5.49171, 5.39527]),
    (0.8333333, [0.0, 0.0, -6.763126, 5.3037415]),
    (0.85, [0.0, 0.0, -8.075531, 5.1910224]),
    (0.8666667, [0.0, 0.0, -9.446963, 5.0564303]),
    (0.8833333, [0.0, 0.0, -10.896674, 4.898981]),
    (0.9, [0.0, 0.0, -12.447134, 4.71737]),
    (0.9166667, [0.0, 0.0, -14.125027, 4.5099173]),
    (0.93333334, [0.0, 0.0, -15.963636, 4.2745004]),
    (0.95, [-0.01287464, 2.1457713e-04, -18.006252, 4.00844]),
    (0.96666664, [0.0, 0.0, -20.309591, 3.708336]),
    (
        0.98333335,
        [0.01287464, -2.1457713e-04, -22.95275, 3.369842],
    ),
    (1.0, [-0.01287464, 2.1457713e-04, -26.04931, 2.9872966]),
    (1.0166667, [0.0, 0.0, -29.770102, 2.5531418]),
    (1.0333333, [0.0, 0.0, -34.388844, 2.056974]),
    (1.05, [0.0, -4.2914815e-04, -40.3778, 1.4838271]),
    (1.0666667, [0.0, 4.2915426e-04, -48.651623, 0.8108596]),
    (1.0833334, [0.0, 0.0, 0.0, 0.0]),
];

/// 下面三项是对话层 `ScreenLayerMysekaiTalk` 的只读字段，由它的构造函数
/// 写入、不序列化（预制体与布局文档里都没有）：show 步淡入 0.3s、hide 步
/// 淡出 0.5s、打字机逐字间隔 `_showingTextIntervalTimeMs` 50ms。
const SHOW_FADE_SECONDS: f32 = 0.3;
const HIDE_FADE_SECONDS: f32 = 0.5;
const TYPE_INTERVAL: f32 = 0.05;

// ---------------------------------------------------------------------------
// 布局文档的读法（读不到或读到本模块不实现的取值即响亮拒绝——资产边界的
// 拒绝点）
// ---------------------------------------------------------------------------

fn field<'a>(fields: &'a Value, name: &str, node: &str) -> &'a Value {
    fields
        .get(name)
        .unwrap_or_else(|| panic!("对话窗 {node}：布局文档缺 {name}"))
}

fn number(fields: &Value, name: &str, node: &str) -> f32 {
    field(fields, name, node)
        .as_f64()
        .filter(|value| value.is_finite())
        .unwrap_or_else(|| panic!("对话窗 {node}：{name} 不是有限数")) as f32
}

fn boolean(fields: &Value, name: &str, node: &str) -> bool {
    field(fields, name, node)
        .as_bool()
        .unwrap_or_else(|| panic!("对话窗 {node}：{name} 不是布尔"))
}

fn integer(fields: &Value, name: &str, node: &str) -> i64 {
    field(fields, name, node)
        .as_i64()
        .unwrap_or_else(|| panic!("对话窗 {node}：{name} 不是整数"))
}

fn floats<const N: usize>(value: &Value, name: &str, node: &str) -> [f32; N] {
    let items = value
        .as_array()
        .filter(|items| items.len() == N)
        .unwrap_or_else(|| panic!("对话窗 {node}：{name} 不是 {N} 个数"));
    std::array::from_fn(|i| {
        items[i]
            .as_f64()
            .filter(|value| value.is_finite())
            .unwrap_or_else(|| panic!("对话窗 {node}：{name} 第 {i} 项不是有限数")) as f32
    })
}

/// 节点上唯一一个满足 `pick` 的组件。
fn component<'a>(
    doc: &'a UiPrefab,
    index: usize,
    node: &str,
    what: &str,
    pick: impl Fn(&UiComponent) -> bool,
) -> &'a UiComponent {
    let mut found = doc.nodes[index].components.iter().filter(|c| pick(*c));
    let first = found
        .next()
        .unwrap_or_else(|| panic!("对话窗 {node}：没有 {what}"));
    assert!(found.next().is_none(), "对话窗 {node}：{what} 不止一个");
    first
}

/// 节点自身与祖先上 CanvasGroup 的序列化 α 之积（不含窗体组——窗体组的 α
/// 由状态机写）；遇到 m_IgnoreParentGroups 开的组即止。
fn group_alpha(doc: &UiPrefab, index: usize, window: usize) -> f32 {
    let mut alpha = 1.0;
    let mut cursor = Some(index);
    while let Some(i) = cursor {
        let path = &doc.nodes[i].path;
        if let Some(group) = doc.nodes[i]
            .components
            .iter()
            .find(|c| c.enabled && c.class == "UnityEngine.CanvasGroup")
        {
            if i != window {
                alpha *= number(&group.fields, "m_Alpha", path);
            }
            if boolean(&group.fields, "m_IgnoreParentGroups", path) {
                break;
            }
        }
        cursor = doc.parent(i);
    }
    alpha
}

/// Graphic 读到的有效 `Canvas.pixelPerfect`：最近一个覆写 pixel-perfect 的
/// 外层 Canvas 决定，否则宿主根 Canvas 决定（UI 布局渲染器同一读法）。
fn pixel_perfect(doc: &UiPrefab, index: usize) -> bool {
    let mut cursor = Some(index);
    while let Some(i) = cursor {
        if let Some(canvas) = doc.nodes[i].components.iter().find(|c| {
            c.enabled
                && c.class == "UnityEngine.Canvas"
                && c.fields["m_OverridePixelPerfect"].as_bool() == Some(true)
        }) {
            return boolean(&canvas.fields, "m_PixelPerfect", &doc.nodes[i].path);
        }
        cursor = doc.parent(i);
    }
    HOST_ROOT_PIXEL_PERFECT
}

/// 一个 Image 组件的绘制输入：Image 规则读的全部字段，加导出图在纹理里的
/// 位置。
struct ImageSource {
    node: &'static str,
    /// Behaviour.enabled（关着的 Image 不画）。
    enabled: bool,
    /// 导出的精灵图（资产路径）。
    path: String,
    image_type: image_rule::ImageType,
    sprite: image_rule::SpriteData,
    /// 导出图占的纹素矩形 (x, y, w, h)：纹理矩形最小角取整、尺寸取导出尺寸。
    crop: [f32; 4],
    /// `Graphic.color`。
    color: [f32; 4],
    /// 外层 CanvasGroup 的序列化 α 之积（见 [`group_alpha`]）。
    group_alpha: f32,
    preserve_aspect: bool,
    fill_center: bool,
    use_sprite_mesh: bool,
    pixels_per_unit_multiplier: f32,
    reference_pixels_per_unit: f32,
    pixel_perfect: bool,
}

/// Image 规则铺出的一片：节点局部坐标（pivot 为原点）的最小/最大角，与它在
/// 导出图里的源矩形（像素，y 从顶量）。
struct Quad {
    min: Vec2,
    max: Vec2,
    source: Rect,
}

impl ImageSource {
    fn read(doc: &UiPrefab, index: usize, window: usize, node: &'static str) -> Self {
        let comp = component(doc, index, node, "Image 组件", |c| {
            c.fields.get("m_Type").is_some()
        });
        let f = &comp.fields;
        let sprite = comp
            .sprite
            .as_ref()
            .filter(|sprite| sprite["state"].as_str() == Some("ok"))
            .unwrap_or_else(|| panic!("对话窗 {node}：精灵未导出"));
        let texture_rect: [f32; 4] = floats(&sprite["textureRect"], "textureRect", node);
        let exported: [f32; 2] = floats(&sprite["size"], "size", node);
        // The exported image is the texture rect cropped at the render data's
        // scale; a layout exported before the multiplier was carries none (the
        // shared root only) and was cropped at 1.
        let downscale = match sprite.get("downscaleMultiplier") {
            Some(_) => number(sprite, "downscaleMultiplier", node),
            None => {
                assert!(
                    doc.source.region.is_none(),
                    "对话窗 {node}：地区根的布局文档缺精灵的 downscaleMultiplier"
                );
                1.0
            }
        };
        assert!(
            downscale == 1.0,
            "对话窗 {node}：精灵的 downscaleMultiplier {downscale} 不是 1，导出图的裁切只对应 1"
        );
        let image_type = image_rule::ImageType::from_serialized(integer(f, "m_Type", node))
            .filter(|kind| {
                matches!(
                    kind,
                    image_rule::ImageType::Simple | image_rule::ImageType::Sliced
                )
            })
            .unwrap_or_else(|| panic!("对话窗 {node}：只实现 Simple 与 Sliced 两种 Image"));
        let image = sprite["image"]
            .as_str()
            .unwrap_or_else(|| panic!("对话窗 {node}：精灵没有导出图"));
        Self {
            node,
            enabled: comp.enabled,
            path: crate::ui_layout::image_asset_path(image),
            image_type,
            sprite: image_rule::SpriteData {
                rect_size: floats(&sprite["rectSize"], "rectSize", node),
                border: floats(&sprite["border"], "border", node),
                pixels_per_unit: number(sprite, "pixelsPerUnit", node),
                texture_rect,
                texture_rect_offset: floats(
                    &sprite["textureRectOffset"],
                    "textureRectOffset",
                    node,
                ),
                downscale_multiplier: downscale,
                texture_size: Some(floats(&sprite["texture"]["size"], "texture size", node)),
            },
            crop: [
                texture_rect[0].round(),
                texture_rect[1].round(),
                exported[0],
                exported[1],
            ],
            color: floats(field(f, "m_Color", node), "m_Color", node),
            group_alpha: group_alpha(doc, index, window),
            preserve_aspect: boolean(f, "m_PreserveAspect", node),
            fill_center: boolean(f, "m_FillCenter", node),
            use_sprite_mesh: boolean(f, "m_UseSpriteMesh", node),
            pixels_per_unit_multiplier: number(f, "m_PixelsPerUnitMultiplier", node),
            reference_pixels_per_unit: comp
                .canvas_reference_pixels_per_unit
                .unwrap_or(HOST_REFERENCE_PIXELS_PER_UNIT),
            pixel_perfect: pixel_perfect(doc, index),
        }
    }

    /// 部件的基础色：Graphic 颜色乘外层组 α（窗体淡变 α 在写回时再乘）。
    fn base_color(&self) -> Color {
        let [r, g, b, a] = self.color;
        Color::srgba(r, g, b, a * self.group_alpha)
    }

    /// `Image.OnPopulateMesh` 在节点矩形上铺出的片（Simple 与 Sliced 都是轴对齐
    /// 的四边形：左下、左上、右上、右下四个顶点）。源矩形由纹理空间 UV 换进
    /// 导出图：纹素 = UV × 纹理尺寸，减去裁切角，行序翻成从顶量。
    fn quads(&self, rect: &UiRect) -> Vec<Quad> {
        let node = self.node;
        let local = image_rule::Rect::from_size_pivot(rect.size.to_array(), rect.pivot.to_array());
        let adjusted =
            image_rule::pixel_adjusted_rect(local, self.pixel_perfect).unwrap_or_else(|| {
                panic!("对话窗 {node}：pixel-perfect Canvas 下的像素对齐矩形未移植")
            });
        let input = image_rule::ImageInput {
            image_type: self.image_type,
            sprite: Some(&self.sprite),
            rect: local,
            pixel_adjusted_rect: adjusted,
            pivot: rect.pivot.to_array(),
            color: [1.0; 4],
            preserve_aspect: self.preserve_aspect,
            fill_center: self.fill_center,
            use_sprite_mesh: self.use_sprite_mesh,
            pixels_per_unit_multiplier: self.pixels_per_unit_multiplier,
            reference_pixels_per_unit: Some(self.reference_pixels_per_unit),
        };
        let stream = match image_rule::populate(&input) {
            image_rule::Populated::Mesh(stream) => stream,
            image_rule::Populated::Unported(branch) => {
                panic!("对话窗 {node}：Image 规则未移植 {branch}")
            }
        };
        assert_eq!(
            stream.vertices.len() % 4,
            0,
            "对话窗 {node}：Image 规则的顶点不成四边形"
        );
        let [texture_w, texture_h] = self.sprite.texture_size.expect("窗体精灵带纹理尺寸");
        let [crop_x, crop_y, _, crop_h] = self.crop;
        let texel =
            |uv: [f32; 2]| Vec2::new(uv[0] * texture_w - crop_x, uv[1] * texture_h - crop_y);
        stream
            .vertices
            .chunks_exact(4)
            .map(|quad| {
                let (low, high) = (quad[0], quad[2]);
                assert!(
                    quad[1].position == [low.position[0], high.position[1], 0.0]
                        && quad[3].position == [high.position[0], low.position[1], 0.0],
                    "对话窗 {node}：Image 规则的片不是轴对齐矩形"
                );
                let (a, b) = (texel(low.uv0), texel(high.uv0));
                Quad {
                    min: Vec2::new(low.position[0], low.position[1]),
                    max: Vec2::new(high.position[0], high.position[1]),
                    source: Rect {
                        min: Vec2::new(a.x, crop_h - b.y),
                        max: Vec2::new(b.x, crop_h - a.y),
                    },
                }
            })
            .collect()
    }
}

/// 一个 CustomTextMesh 的样式（摆位读的全部字段）。
struct TextStyle {
    node: &'static str,
    /// TMP 渲染轮的起始字号。
    font_size: f32,
    /// autosize 开时的 (m_fontSizeMin, m_fontSizeMax)。
    auto_size: Option<(f32, f32)>,
    /// m_fontColor。
    color: [f32; 4],
    /// 外层 CanvasGroup 的序列化 α 之积（见 [`group_alpha`]）。
    group_alpha: f32,
    /// 字距增量：m_characterSpacing × 0.01 × 字号。
    char_extra: f32,
    /// 词距增量：m_wordSpacing × 0.01 × 字号（空白字后补一笔）。
    word_extra: f32,
    /// m_lineSpacing（序列化原值，换算在行量律内部）。
    line_spacing: f32,
    /// m_margin：左、上、右、下。
    margin: [f32; 4],
}

impl TextStyle {
    fn read(doc: &UiPrefab, index: usize, window: usize, node: &'static str) -> Self {
        let comp = component(doc, index, node, "CustomTextMesh", |c| {
            c.class == "Sekai.UI.CustomTextMesh"
        });
        assert!(comp.enabled, "对话窗 {node}：CustomTextMesh 在预制体里关着");
        let f = &comp.fields;
        let auto = boolean(f, "m_enableAutoSizing", node);
        let (min, max) = (
            number(f, "m_fontSizeMin", node),
            number(f, "m_fontSizeMax", node),
        );
        let font_size = auto_size::render_start_size(
            auto,
            number(f, "m_fontSize", node),
            number(f, "m_fontSizeBase", node),
            min,
            max,
        );
        assert!(font_size > 0.0, "对话窗 {node}：字号 {font_size} 不为正");
        // TMP 的对齐以两个序列化字段为准（旧式合并字段 m_textAlignment 不读）。
        let horizontal = integer(f, "m_HorizontalAlignment", node);
        let vertical = integer(f, "m_VerticalAlignment", node);
        assert!(
            horizontal == 1 && vertical == 256,
            "对话窗 {node}：对齐 {horizontal}/{vertical} 未实现（只实现居左 1、居顶 256）"
        );
        let em = 0.01 * font_size;
        Self {
            node,
            font_size,
            auto_size: auto.then_some((min, max)),
            color: floats(field(f, "m_fontColor", node), "m_fontColor", node),
            group_alpha: group_alpha(doc, index, window),
            char_extra: number(f, "m_characterSpacing", node) * em,
            word_extra: number(f, "m_wordSpacing", node) * em,
            line_spacing: number(f, "m_lineSpacing", node),
            margin: floats(field(f, "m_margin", node), "m_margin", node),
        }
    }

    /// 文本区：节点矩形（节点局部坐标，pivot 为原点）按 m_margin 内缩。
    fn area(&self, rect: &UiRect) -> Rect {
        let min = -rect.pivot * rect.size;
        let max = min + rect.size;
        let [left, top, right, bottom] = self.margin;
        Rect {
            min: Vec2::new(min.x + left, min.y + bottom),
            max: Vec2::new(max.x - right, max.y - top),
        }
    }
}

/// 窗体从布局文档读出的全部绘制输入（节点下标与组件字段）。
struct WindowSource {
    panel: usize,
    label: usize,
    content: usize,
    end_sign: usize,
    end_mark: usize,
    panel_image: ImageSource,
    end_mark_image: ImageSource,
    label_text: TextStyle,
    content_text: TextStyle,
}

impl WindowSource {
    fn read(doc: &UiPrefab, canvas: Vec2) -> Self {
        let find = |path: &str| {
            doc.find(path)
                .unwrap_or_else(|error| panic!("对话窗：{error}"))
        };
        // 窗体的矩形不经自动布局解算：它们的父链上有布局控制器即拒绝。正文
        // 与尾标标记的父链覆盖了面板、名字栏与尾标组。
        for path in [CONTENT_NODE, END_MARK_NODE] {
            let rect = crate::browser_stage::dialogue_panel(doc, path, canvas)
                .unwrap_or_else(|error| panic!("对话窗：{error}"));
            assert!(rect.active, "对话窗 {path}：预制体里（或父链上）未激活");
        }
        let window = find(WINDOW_NODE);
        let group = component(doc, window, WINDOW_NODE, "CanvasGroup", |c| {
            c.class == "UnityEngine.CanvasGroup"
        });
        let rest = number(&group.fields, "m_Alpha", WINDOW_NODE);
        assert!(
            rest == 1.0,
            "对话窗：窗体组序列化 α {rest} 不是 1——状态机的静息 α 与 show 步的 Approximately(α,1) 短路都以它为前提"
        );
        let panel = find(PANEL_NODE);
        let label = find(LABEL_NODE);
        let content = find(CONTENT_NODE);
        let end_mark = find(END_MARK_NODE);
        Self {
            panel,
            label,
            content,
            end_sign: find(END_SIGN_NODE),
            end_mark,
            panel_image: ImageSource::read(doc, panel, window, PANEL_NODE),
            end_mark_image: ImageSource::read(doc, end_mark, window, END_MARK_NODE),
            label_text: TextStyle::read(doc, label, window, LABEL_NODE),
            content_text: TextStyle::read(doc, content, window, CONTENT_NODE),
        }
    }
}

/// 窗体各节点在当前 canvas 下的解算矩形（canvas 单位、原点在 canvas 中心）。
struct WindowRects {
    panel: UiRect,
    label: UiRect,
    content: UiRect,
    end_sign: UiRect,
    end_mark: UiRect,
}

/// 解算窗体节点：尾标标记带上 `PageForward` 在状态时间 `end_clock` 写的
/// anchoredPosition 与 localScale（剪辑的 x 位置与 x/z 缩放是常值 0 与 1）。
fn window_rects(
    doc: &UiPrefab,
    source: &WindowSource,
    canvas: Vec2,
    end_clock: f32,
) -> WindowRects {
    let (anchored, scale_y) = page_forward_pose(end_clock);
    let mut mark = doc.nodes[source.end_mark].rect.clone();
    mark.anchored_position = anchored.to_array();
    mark.local_scale = [1.0, scale_y, 1.0];
    let rects = doc.resolve_with(
        canvas,
        &HashMap::new(),
        &HashMap::from([(source.end_mark, mark)]),
    );
    WindowRects {
        panel: rects[source.panel].clone(),
        label: rects[source.label].clone(),
        content: rects[source.content].clone(),
        end_sign: rects[source.end_sign].clone(),
        end_mark: rects[source.end_mark].clone(),
    }
}

// ---------------------------------------------------------------------------
// 资源与组件
// ---------------------------------------------------------------------------

/// 窗体的绘制输入：从布局文档读出的值与两张导出图的句柄（常驻——句柄一丢
/// 装载即取消）。布局文档到了才读，读一次。
#[derive(Resource)]
pub(crate) struct WindowArt {
    source: WindowSource,
    panel: Handle<Image>,
    end_mark: Handle<Image>,
}

/// 窗体状态机半：真源层的可变字段（闩、打字机、α）+ 呈现侧版本号。
/// 常驻资源（对话不在播也在——真源层对象常驻，show/hide 只动可见性）。
#[derive(Resource)]
pub(crate) struct TalkWindowState {
    /// 窗体是否在场（对话开始置位、收口撤；层压栈/弹出的宿主侧对应物）。
    present: bool,
    /// CanvasGroup α 当前值。
    alpha: f32,
    /// 进行中的淡变 (起点, 目标, 时长, 已放秒)。
    fade: Option<(f32, f32, f32, f32)>,
    /// 名字栏文本（`ResetTexts` 清空、label 步覆写；常驻到下一次覆写）。
    label: String,
    /// 正文全文（`ShowTextAsync` 一次设入，打字机只动揭示游标）。
    text: String,
    /// 已揭示的字符数（真源下标 i 的「已过」侧；换行占原文位但不揭示）。
    shown: usize,
    /// 打字机时钟（秒）。
    typing_clock: f32,
    /// `_isPlayingTextAnimation`。
    playing: bool,
    /// `_isClicked`（点击闩：跳过支与放行门共写）。
    clicked: bool,
    /// `_endIconObj` 激活态。
    end_icon: bool,
    /// EndSign 上 Animator 的状态时间（秒，未取模）：激活沿归 0，激活
    /// 期间按帧时间推进；停用即归 0（`m_KeepAnimatorStateOnDisable` 关，
    /// 再激活从默认状态头起播）。
    end_clock: f32,
    /// 正文/名字栏的改版计数（呈现侧按版本重建字形实体）。
    text_version: u64,
    label_version: u64,
    built_text: u64,
    built_label: u64,
}

impl TalkWindowState {
    /// Read-only visible source transcript for accessibility/acceptance tools.
    pub(crate) fn transcript(&self) -> (&str, &str, bool, bool) {
        // HideTalkWindow fades alpha without unmounting the session. An
        // accessibility/QA reader must not mistake a retained window for a
        // visible dialogue prompt and accidentally skip a silent finale.
        let displaying = self.present
            && self
                .fade
                .as_ref()
                .map_or(self.alpha > 0.01, |(_, target, _, _)| *target > 0.01);
        (&self.label, &self.text, displaying, self.playing)
    }
}

impl Default for TalkWindowState {
    fn default() -> Self {
        Self {
            present: false,
            alpha: 1.0,
            fade: None,
            label: String::new(),
            text: String::new(),
            shown: 0,
            typing_clock: 0.0,
            playing: false,
            clicked: false,
            end_icon: false,
            end_clock: 0.0,
            text_version: 0,
            label_version: 0,
            built_text: 0,
            built_label: 0,
        }
    }
}

impl TalkWindowState {
    /// `Mathf.Approximately(α,1)` 的容差（浮点 ε×8 一档）。
    fn approximately_one(value: f32) -> bool {
        (value - 1.0).abs() <= 1e-6
    }

    /// 对话开始：层压栈即窗体在场（序列化初值 α=1，无淡入），`ResetTexts`
    /// 清两栏。首个 show 步因此被 Approximately 短路。要一条会话作保
    /// （归属构造，见模块注释——两条链的 [`TalkSession`] 任一）。
    pub(crate) fn open(&mut self, _owner: TalkSession<'_>) {
        self.reset_for_admission(true);
    }

    /// A fixture pre-action has acquired its cast but has not entered the
    /// source talk body yet. Clear the previous prompt without exposing an
    /// empty dialogue panel or admitting clicks during source asset loading.
    pub(crate) fn prepare(&mut self, _owner: TalkSession<'_>) {
        self.reset_for_admission(false);
    }

    fn reset_for_admission(&mut self, present: bool) {
        self.present = present;
        self.alpha = 1.0;
        self.fade = None;
        self.label.clear();
        self.text.clear();
        self.shown = 0;
        self.typing_clock = 0.0;
        self.playing = false;
        self.clicked = false;
        self.end_icon = false;
        self.end_clock = 0.0;
        self.text_version += 1;
        self.label_version += 1;
    }

    /// 对话收口：层弹出无淡出，即时撤（α 回序列化初值，下次开场即短路）。
    /// 同上要会话作保。
    pub(crate) fn close(&mut self, _owner: TalkSession<'_>) {
        self.present = false;
        self.alpha = 1.0;
        self.fade = None;
        self.label.clear();
        self.text.clear();
        self.shown = 0;
        self.typing_clock = 0.0;
        self.playing = false;
        self.clicked = false;
        self.end_icon = false;
        self.end_clock = 0.0;
        self.text_version += 1;
        self.label_version += 1;
    }

    /// show 步：α 已 ≈1 时短路（返回 `None`），否则 0.3s 淡入（返回
    /// 起始 α，日志用）。同上要会话作保。
    pub(crate) fn show(&mut self, _owner: TalkSession<'_>) -> Option<f32> {
        self.present = true;
        if Self::approximately_one(self.alpha) {
            return None;
        }
        let from = self.alpha;
        self.fade = Some((from, 1.0, SHOW_FADE_SECONDS, 0.0));
        Some(from)
    }

    /// hide 步：无条件 0.5s 淡出到 0（窗体仍在场，α=0）。同上要会话作保。
    pub(crate) fn hide(&mut self, _owner: TalkSession<'_>) {
        self.fade = Some((self.alpha, 0.0, HIDE_FADE_SECONDS, 0.0));
    }

    /// 正文进窗（`ShowTextAsync` 状态-0）：闩清、在播置位、尾标收、
    /// 游标 0；第 0 字在 t=0 即揭示。空文本直接 DONE。同上要会话作保。
    pub(crate) fn show_text(&mut self, _owner: TalkSession<'_>, text: &str) {
        self.clicked = false;
        self.text.clear();
        self.text.push_str(text);
        self.playing = true;
        self.end_icon = false;
        self.end_clock = 0.0;
        self.typing_clock = 0.0;
        self.shown = 0;
        let len = text.chars().count();
        if len == 0 {
            self.playing = false;
            self.end_icon = true;
        } else {
            self.shown = 1;
        }
        self.text_version += 1;
    }

    /// label 步：名字栏 `SetText`（常驻到下一次覆写或开场清空）。同上
    /// 要会话作保。
    pub(crate) fn set_label(&mut self, _owner: TalkSession<'_>, name: &str) {
        self.label.clear();
        self.label.push_str(name);
        self.label_version += 1;
    }

    /// 放行门（`IsWaitClick`）：闩置位且打字机已收尾。
    pub(crate) fn click_level(&self) -> bool {
        self.clicked && !self.playing
    }

    /// 置闩（真实点击或显式启用的冒烟输入）。要一条会话作保——输入
    /// 系统按会话在场门控后，置闩只可能来自某条链在播期间。
    pub(crate) fn latch(&mut self, _owner: TalkSession<'_>) {
        self.clicked = true;
    }

    /// 闩空闲？（会话首拍用来耗尽发起点击，不将它用于正文放行。）
    pub(crate) fn latch_idle(&self) -> bool {
        !self.clicked
    }

    /// 耗尽闩（`WaitClicked` 放行后的清闩——该击即耗尽）。放行不发音效
    /// （见 [`tick_window`]）。同上要会话作保。
    pub(crate) fn consume_click(&mut self, _owner: TalkSession<'_>) {
        self.clicked = false;
    }
}

/// 窗体根（挂 canvas → 屏幕的缩放；子件全在 canvas 单位系）。
#[derive(Component)]
pub(crate) struct TalkWindowRoot;

/// 铺树时各节点矩形的尺寸与 pivot：canvas 变化若改了它们（拉伸锚点），片与
/// 字形按新矩形重铺。
#[derive(Component, Clone, Copy, PartialEq)]
pub(crate) struct BuiltShape([Vec4; 4]);

impl BuiltShape {
    fn of(rects: &WindowRects) -> Self {
        let key = |rect: &UiRect| Vec4::new(rect.size.x, rect.size.y, rect.pivot.x, rect.pivot.y);
        Self([
            key(&rects.panel),
            key(&rects.label),
            key(&rects.content),
            key(&rects.end_mark),
        ])
    }
}

/// 跟随一个布局节点的容器：Transform = 该节点的解算世界矩阵（每帧写），片与
/// 字形作为子件画在节点局部坐标里（pivot 为原点）。
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WindowNode {
    Panel,
    Label,
    Content,
    /// 尾标标记（激活态随打字机 DONE 翻转）。
    EndMark,
}

impl WindowNode {
    fn transform(self, rects: &WindowRects) -> Transform {
        let rect = match self {
            WindowNode::Panel => &rects.panel,
            WindowNode::Label => &rects.label,
            WindowNode::Content => &rects.content,
            WindowNode::EndMark => &rects.end_mark,
        };
        Transform::from_matrix(rect.world)
    }
}

/// 窗体一个 sprite 部件：基础色（α 淡变写回的底）。
#[derive(Component)]
pub(crate) struct TalkWindowPart {
    base: Color,
}

/// 正文一个字形：`order` = 原文下标（打字机按它揭示）。
#[derive(Component)]
pub(crate) struct TalkGlyph {
    order: usize,
}

/// 名字栏一个字形（无打字机，常显）。
#[derive(Component)]
pub(crate) struct TalkLabelGlyph;

// ---------------------------------------------------------------------------
// 装载与输入
// ---------------------------------------------------------------------------

/// Startup：初始化窗体状态（常驻）。绘制输入等布局文档到了再读（见
/// [`tick_window`]）。
pub(crate) fn load(mut commands: Commands) {
    commands.init_resource::<TalkWindowState>();
}

/// Update（对话链内、步进前）：点跳输入最小集——Space 与 tap 族的
/// 收场沿。真源对话引擎的手势门：手势 ∈ {TAP, DOUBLE_TAP, LONG_TOUCH}
/// 且态为 End 即触发，**不问屏位**（手势层是全屏的，本来就没有位
/// 门槛）。相机拖拽在手势层就被折成 DRAG、不属 tap 族——拖完松手
/// 不会误触点跳（此前读裸左键按下沿，拖拽起手即误闩，那正是要由
/// 手势层接管的分工）。真源 `OnClick` 的门（玩家数据在场、UI 未隐藏）
/// 在宿主里的对应物是**任一对话会话在场**（归属构造，见模块注释）：
/// 无会话的点击不进闩——发起点按在会话插上前发生（发起消费该击），
/// 无对话期间的点击对窗体惰性。两条链的会话互斥由入口守卫保证，
/// 同帧双链在场是守卫失守，响亮 panic。
pub(crate) fn read_click_input(
    mut state: ResMut<TalkWindowState>,
    sessions: Option<ResMut<crate::player_talk::PlayerTalkSession>>,
    talks: Option<ResMut<crate::talk::ActiveTalk>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut gestures: MessageReader<GestureEvent>,
) {
    let tap_family_end = gestures
        .read()
        .any(|event| event.kind.is_tap_family() && event.state == GestureState::End);
    if !(tap_family_end || keys.just_pressed(KeyCode::Space)) {
        return;
    }
    match (sessions, talks) {
        (None, None) => {}
        (Some(mut session), None) => {
            let owner = TalkSession::Player(&mut session);
            let chain = owner.chain_word();
            state.latch(owner);
            info!("[talkwin] 点击闩置位（{chain}；tap 族收场/Space；打字中即跳字，收尾后放行）");
        }
        (None, Some(mut talk)) => {
            let owner = TalkSession::Pair(&mut talk);
            let chain = owner.chain_word();
            state.latch(owner);
            info!("[talkwin] 点击闩置位（{chain}；tap 族收场/Space；打字中即跳字，收尾后放行）");
        }
        (Some(_), Some(_)) => {
            panic!("玩家会话与配对会话同时在播：两链互斥的入口守卫失守，fail-closed");
        }
    }
}

/// 冒烟口（`MOLY_TALK_TAP_AT_SECS`，逗号分隔的时刻表；宿主侧仪表，与
/// 采集 autohit 同款）：到点即置闩——与真点击同一条路（打字中即
/// 点跳，收尾后放行）。无头窗口收不到真点击，点跳半边的 SE 验证靠它。
/// 同样按会话在场门控：时刻落在无会话的空档不消费（对窗体惰性，挂到
/// 下一场对话开场后即拍）。
pub(crate) fn smoke_tap(
    mut state: ResMut<TalkWindowState>,
    sessions: Option<ResMut<crate::player_talk::PlayerTalkSession>>,
    talks: Option<ResMut<crate::talk::ActiveTalk>>,
    time: Res<Time>,
    mut schedule: Local<Option<Vec<f32>>>,
    mut next: Local<usize>,
) {
    let times = schedule.get_or_insert_with(|| {
        std::env::var("MOLY_TALK_TAP_AT_SECS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .filter_map(|t| t.trim().parse::<f64>().ok().map(|v| v.max(0.0) as f32))
                    .collect()
            })
            .unwrap_or_default()
    });
    if times.is_empty() || *next >= times.len() {
        return;
    }
    if sessions.is_none() && talks.is_none() {
        return; // 空档：时刻不消费，等下一场对话
    }
    let now = time.elapsed_secs();
    if now < times[*next] {
        return;
    }
    *next += 1;
    match (sessions, talks) {
        (None, None) => {}
        (Some(mut session), None) => {
            let owner = TalkSession::Player(&mut session);
            let chain = owner.chain_word();
            state.latch(owner);
            info!(
                "[talkwin] 冒烟点击置闩（{chain}，时刻表第 {} 拍 @ {:.2}s；打字中即点跳，收尾后放行）",
                *next, now
            );
        }
        (None, Some(mut talk)) => {
            let owner = TalkSession::Pair(&mut talk);
            let chain = owner.chain_word();
            state.latch(owner);
            info!(
                "[talkwin] 冒烟点击置闩（{chain}，时刻表第 {} 拍 @ {:.2}s；打字中即点跳，收尾后放行）",
                *next, now
            );
        }
        (Some(_), Some(_)) => {
            panic!("玩家会话与配对会话同时在播：两链互斥的入口守卫失守，fail-closed");
        }
    }
}

// ---------------------------------------------------------------------------
// 帧推进
// ---------------------------------------------------------------------------

/// Update（对话链内、步进后）：状态机半（跳过/打字机/淡变/尾标动效）
/// + 呈现（树生死、字形重建、揭示与 α 写回、节点矩阵与 canvas 缩放）。布局
/// 文档到了才读绘制输入，纹源到齐后才动实体；装载失败响亮 panic（资产边界
/// 的拒绝点）。
///
/// **点击不发音效**（点跳与放行皆然）。真源的点击链是：手势事件 →
/// 对话引擎 `OnGesture`（TAP 族且态为 End）→ `OnClick` → 窗体 `OnClick`
/// 只置 `_isClicked`（UI 隐藏时连闩都不置）；打字机的点跳支只做
/// 「整段 SetText、清闩」，`WaitClicked` 只是 `WaitUntil(IsWaitClick)`，
/// Lua 侧 `wait_click` 只 yield 这个等待再冲延迟命令——整条链没有一次
/// 音频调用。表面反例：①对话剧本有 `se(label)` 这个 Lua 入口（转到
/// `PlaySe`），但日服全部 6919 个对话脚本里词表外调用 0 次、`se(` 0 次；
/// ②布局里的全屏点击区 `ClickArea` 是一个 CustomButton，但它序列化为
/// 未激活，只在场景为 OutGame 时（`SetupOutGame(true)`）才激活——
/// MySekai 场景里的对话不经过它。
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn tick_window(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    server: Res<AssetServer>,
    layouts: Option<Res<UiLayouts>>,
    art: Option<Res<BalloonArt>>,
    window_art: Option<Res<WindowArt>>,
    mut state: ResMut<TalkWindowState>,
    mut roots: Query<(Entity, &mut Transform, &BuiltShape), With<TalkWindowRoot>>,
    mut nodes: Query<
        (Entity, &WindowNode, &mut Transform, &mut Visibility),
        (Without<TalkWindowRoot>, Without<TalkGlyph>),
    >,
    mut glyphs: Query<(Entity, &TalkGlyph, &mut Visibility), Without<WindowNode>>,
    label_glyphs: Query<Entity, With<TalkLabelGlyph>>,
    mut parts: Query<(&TalkWindowPart, &mut Sprite)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let dt = time.delta_secs();
    let end_was_active = state.end_icon;

    // --- 状态机半：跳过支（先于推进——真源逐字循环每拍先查闩） ---
    if state.clicked && state.playing {
        let len = state.text.chars().count();
        state.shown = len;
        state.playing = false;
        state.end_icon = true;
        state.clicked = false;
        info!("[talkwin] 点跳：跳过打字机（{len} 字一次揭示），点击闩清零——放行须再点");
    }

    // --- 状态机半：打字机推进（第 k 字在 50k ms，DONE 在 50×len ms） ---
    if state.playing {
        state.typing_clock += dt;
        let len = state.text.chars().count();
        let due = ((state.typing_clock / TYPE_INTERVAL).floor() as usize + 1).min(len);
        state.shown = due;
        if state.typing_clock >= TYPE_INTERVAL * len as f32 {
            state.playing = false;
            state.end_icon = true;
            info!(
                "[talkwin] 打字机 DONE：{len} 字 {:.2}s（50ms/字，末字后再一拍），尾标激活，在播清零",
                state.typing_clock
            );
        }
    }

    // --- 状态机半：尾标 Animator 的状态时间。激活沿那一帧按 t=0 取姿态，
    // 此后每帧推进帧时间（普通更新模式）；停用即归 0。激活帧本身是否
    // 已推进一帧，未对引擎取证（差至多一帧）。 ---
    if state.end_icon && end_was_active {
        state.end_clock += dt;
    } else {
        state.end_clock = 0.0;
    }

    // --- 状态机半：淡变（OutQuad——补间库缺省） ---
    if let Some((from, target, duration, elapsed)) = state.fade {
        let elapsed = elapsed + dt;
        let t = (elapsed / duration).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        state.alpha = from + (target - from) * eased;
        if elapsed >= duration {
            state.alpha = target;
            state.fade = None;
            info!(
                "[talkwin] 淡变完成：α={:.2}（{:.1}s OutQuad）",
                target, duration
            );
        } else {
            state.fade = Some((from, target, duration, elapsed));
        }
    }

    // --- 呈现：树生死（根至多一棵——只有本系统铺） ---
    if !state.present {
        for (root, _, _) in &roots {
            commands.entity(root).despawn();
        }
        return;
    }
    let (Some(layouts), Some(root_canvas), Ok(window)) =
        (layouts.as_deref(), root_canvas.as_deref(), windows.single())
    else {
        return;
    };
    let Some(doc) = layouts.document(LAYOUT) else {
        return; // 布局文档未到：等
    };
    // Browser and native views consume the same authored tree: the viewport
    // changes the canvas the nodes resolve in, not their authored geometry.
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    if !scale.is_finite() || scale <= 0.0 || !canvas.is_finite() || canvas.min_element() <= 0.0 {
        return; // 挂起/最小化的窗口没有可画的 canvas
    }
    let Some(window_art) = window_art.as_deref() else {
        let source = WindowSource::read(doc, canvas);
        log_source(&source);
        let panel = moly_assets::residency::load_image(&server, source.panel_image.path.clone());
        let end_mark =
            moly_assets::residency::load_image(&server, source.end_mark_image.path.clone());
        commands.insert_resource(WindowArt {
            source,
            panel,
            end_mark,
        });
        return;
    };
    let source = &window_art.source;
    let rects = window_rects(doc, source, canvas, state.end_clock);
    let placement = Transform::from_scale(Vec3::splat(scale));
    if roots.is_empty() {
        // 无树：纹源齐了才铺；铺完把两栏版本记到当版（同帧已铺）。
        let Some(art) = art.as_deref() else {
            return; // 图集未烘（tweet 主表先到才烘——对话窗共用它的字形）
        };
        for (what, handle) in [("面板", &window_art.panel), ("尾标", &window_art.end_mark)] {
            match server.load_state(handle) {
                LoadState::Failed(err) => panic!("对话窗{what}贴图装载失败：{err:?}"),
                LoadState::Loaded => {}
                _ => return, // 未到：等（首次装载帧序）
            }
        }
        spawn_tree(&mut commands, art, window_art, &state, placement, &rects);
        state.built_label = state.label_version;
        state.built_text = state.text_version;
        log_rects(canvas, scale, &rects);
        return;
    }
    let (root, mut root_transform, built) = roots.single_mut().expect("TalkWindowRoot 至多一棵");
    if *built != BuiltShape::of(&rects) {
        // 节点尺寸或 pivot 随 canvas 变了：撤树，下一帧按新矩形重铺。
        commands.entity(root).despawn();
        return;
    }
    if *root_transform != placement || state.built_text != state.text_version {
        commands.entity(root).insert(stage_layout::source_metrics(
            scale,
            &rects.panel,
            source.content_text.font_size,
            &state.text,
        ));
    }
    root_transform.set_if_neq(placement);

    // --- 呈现：节点矩阵（尾标标记的矩阵带着剪辑姿态）与尾标激活态 ---
    let mut label_box = None;
    let mut content_box = None;
    for (entity, node, mut transform, mut visibility) in nodes.iter_mut() {
        transform.set_if_neq(node.transform(&rects));
        match node {
            WindowNode::Label => label_box = Some(entity),
            WindowNode::Content => content_box = Some(entity),
            WindowNode::EndMark => {
                visibility.set_if_neq(if state.end_icon {
                    Visibility::Visible
                } else {
                    Visibility::Hidden
                });
            }
            WindowNode::Panel => {}
        }
    }

    // --- 呈现：名字栏/正文字形重建（版本落后即重建） ---
    let Some(art) = art.as_deref() else {
        return;
    };
    let (Some(label_box), Some(content_box)) = (label_box, content_box) else {
        return; // 容器与树同一批命令铺下，下一帧才查得到
    };
    if state.built_label != state.label_version {
        for entity in &label_glyphs {
            commands.entity(entity).despawn();
        }
        spawn_label_glyphs(
            &mut commands,
            label_box,
            art,
            &source.label_text,
            &rects.label,
            &state,
        );
        state.built_label = state.label_version;
        info!(
            "[talkwin] 名字栏重建：{} 字（字号 {}，名字不进日志）",
            state.label.chars().count(),
            source.label_text.font_size
        );
    }
    if state.built_text != state.text_version {
        // Text replacement owns the previous glyph entities as well as the string.
        // Otherwise the new reveal cursor makes earlier sentences visible again.
        for (entity, _, _) in &glyphs {
            commands.entity(entity).despawn();
        }
        let style = &source.content_text;
        let (count, lines, widest) = spawn_content_glyphs(
            &mut commands,
            content_box,
            art,
            style,
            &rects.content,
            &state,
        );
        state.built_text = state.text_version;
        let box_width = style.area(&rects.content).width();
        if widest > box_width {
            match style.auto_size {
                Some((min, max)) => warn!(
                    "[talkwin] 正文最宽行 {widest:.0}px 超盒宽 {box_width:.0}：真源 autosize（{min}–{max}）会搜字号，本实现按起始字号 {} 定格",
                    style.font_size
                ),
                None => warn!(
                    "[talkwin] 正文最宽行 {widest:.0}px 超盒宽 {box_width:.0}（autosize 关，字号 {}）",
                    style.font_size
                ),
            }
        }
        info!(
            "[talkwin] 正文重建：{count} 字形 {lines} 行，最宽行 {widest:.0}px / 盒宽 {box_width:.0}（打字机 {} 字，揭示 {}）",
            state.text.chars().count(),
            state.shown
        );
    }

    update_visibility(&state, &mut glyphs, &mut parts);
}

/// 读到的绘制输入（一次，文档到了时）：贴图、字号、颜色与间距。
fn log_source(source: &WindowSource) {
    let image = |image: &ImageSource| {
        format!(
            "{}（{:?}，精灵 {:?}、border {:?}）",
            image.path, image.image_type, image.sprite.rect_size, image.sprite.border
        )
    };
    let text = |style: &TextStyle| {
        format!(
            "{}：字号 {}（autosize {:?}）、色 {:?}、字距 {:.4}/词距 {:.4}、行距 {}、边距 {:?}",
            style.node,
            style.font_size,
            style.auto_size,
            style.color,
            style.char_extra,
            style.word_extra,
            style.line_spacing,
            style.margin
        )
    };
    info!(
        "[talkwin] 绘制输入读自布局文档 {LAYOUT}：面板 {}；尾标 {}；{}；{}",
        image(&source.panel_image),
        image(&source.end_mark_image),
        text(&source.label_text),
        text(&source.content_text)
    );
}

/// 铺树时的解算矩形（canvas 单位：尺寸与中心）。
fn log_rects(canvas: Vec2, scale: f32, rects: &WindowRects) {
    let rect = |rect: &UiRect| {
        let center = rect.center();
        format!(
            "{:.1}x{:.1} @({:.2},{:.2})",
            rect.size.x, rect.size.y, center.x, center.y
        )
    };
    info!(
        "[talkwin] 窗体树上屏（canvas {:.1}x{:.1}、缩放 {scale:.4}）：面板 {}；名字栏 {}；正文 {}；尾标组 {}；尾标 {}",
        canvas.x,
        canvas.y,
        rect(&rects.panel),
        rect(&rects.label),
        rect(&rects.content),
        rect(&rects.end_sign),
        rect(&rects.end_mark)
    );
}

// ---------------------------------------------------------------------------
// 树与字形
// ---------------------------------------------------------------------------

fn update_visibility(
    state: &TalkWindowState,
    glyphs: &mut Query<(Entity, &TalkGlyph, &mut Visibility), Without<WindowNode>>,
    parts: &mut Query<(&TalkWindowPart, &mut Sprite)>,
) {
    // --- 呈现：揭示 ---
    for (_, glyph, mut visible) in glyphs.iter_mut() {
        visible.set_if_neq(if glyph.order < state.shown {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });
    }

    // --- 呈现：α 写回（淡变值乘进每个部件的基础色） ---
    let alpha = state.alpha;
    for (part, mut sprite) in parts.iter_mut() {
        sprite.color = part.base.with_alpha(part.base.alpha() * alpha);
    }
}

/// 引擎对流式剪辑曲线的求值：取键时 ≤ t 的最后一段，段内三次式
/// ((a·dt+b)·dt+c)·dt+d（f32，与引擎同精度）。末键之后落在末段（常值）。
fn streamed_curve(segments: &[(f32, [f32; 4])], t: f32) -> f32 {
    let index = segments
        .partition_point(|(key, _)| *key <= t)
        .saturating_sub(1);
    let (key, [a, b, c, d]) = segments[index];
    let dt = t - key;
    ((a * dt + b) * dt + c) * dt + d
}

/// `PageForward` 在状态时间 `clock` 的姿态：标记的 anchoredPosition 与
/// localScale.y（状态 loop：按剪辑长度取模）。
fn page_forward_pose(clock: f32) -> (Vec2, f32) {
    let t = clock.rem_euclid(PAGE_FORWARD_LENGTH);
    (
        Vec2::new(0.0, streamed_curve(&PAGE_FORWARD_POS_Y, t)),
        streamed_curve(&PAGE_FORWARD_SCALE_Y, t),
    )
}

/// 铺窗体树：根（canvas → 屏幕缩放）→ 四个节点容器（节点的解算矩阵）→
/// 面板片、尾标片与两栏字形（节点局部坐标）。
fn spawn_tree(
    commands: &mut Commands,
    art: &BalloonArt,
    window_art: &WindowArt,
    state: &TalkWindowState,
    placement: Transform,
    rects: &WindowRects,
) {
    let source = &window_art.source;
    let root = commands
        .spawn((
            TalkWindowRoot,
            BuiltShape::of(rects),
            stage_layout::source_metrics(
                placement.scale.x,
                &rects.panel,
                source.content_text.font_size,
                &state.text,
            ),
            placement,
            Visibility::default(),
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    // 面板之下**没有**全宽暗带，这是真源状态，不是缺件。表面反例：
    // 窗体节点 `Window`（1920×415、底边锚定）上挂着一张九宫 CustomImage，
    // 指向 ScenarioAtlas 里真实存在的 `bg_story_adv`，布局文档把它原样列
    // 出。但该组件序列化为 **enabled=false**（国服与日服两份预制体一致），
    // 而真源里能碰到它的写者一个都没有：对话层的序列化字段按顺序是名字栏、
    // 正文、自动播放标、尾标 GameObject、窗体 CanvasGroup、菜单/隐藏 UI/
    // 截图三组按钮与 CanvasGroup，外加四个 Vector2——没有一个指向这张
    // 图；层代码只动 CanvasGroup 的 α、两栏文本、自动播放标与尾标的激活。
    // ⇒ 引擎不画它，这里也不画。
    let panel = spawn_node(
        commands,
        root,
        WindowNode::Panel,
        rects,
        Visibility::Inherited,
    );
    spawn_image(
        commands,
        panel,
        &source.panel_image,
        &window_art.panel,
        &rects.panel,
        -0.1,
        state.alpha,
    );
    // 尾标：姿态由 `PageForward` 剪辑逐帧写进标记节点的矩阵（见
    // `window_rects`）；激活态随打字机。
    // 同组的星标 `star`（MenuAtlas `icon_pageForward_2`，带 TweenScale /
    // TweenPosition 两个循环补间）**不铺**：它序列化为未激活，Animator 剪辑
    // 不绑它（只绑 `Image`），层代码对尾标只做
    // `_endIconObj.SetActive(true/false)`——激活父级不会激活一个自身未激活
    // 的子级，于是它的两个补间永远不会起跑。
    let end_visibility = if state.end_icon {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    let end = spawn_node(commands, root, WindowNode::EndMark, rects, end_visibility);
    spawn_image(
        commands,
        end,
        &source.end_mark_image,
        &window_art.end_mark,
        &rects.end_mark,
        0.0,
        state.alpha,
    );
    let label = spawn_node(
        commands,
        root,
        WindowNode::Label,
        rects,
        Visibility::Inherited,
    );
    let content = spawn_node(
        commands,
        root,
        WindowNode::Content,
        rects,
        Visibility::Inherited,
    );
    spawn_label_glyphs(
        commands,
        label,
        art,
        &source.label_text,
        &rects.label,
        state,
    );
    spawn_content_glyphs(
        commands,
        content,
        art,
        &source.content_text,
        &rects.content,
        state,
    );
}

/// 一个节点容器（根的子件，Transform = 节点的解算矩阵）。
fn spawn_node(
    commands: &mut Commands,
    root: Entity,
    node: WindowNode,
    rects: &WindowRects,
    visibility: Visibility,
) -> Entity {
    let entity = commands
        .spawn((
            node,
            node.transform(rects),
            visibility,
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    commands.entity(root).add_child(entity);
    entity
}

/// 一个 Image 的片：每片一个 sprite（源矩形取自导出图，显示尺寸取片尺寸），
/// 画在节点局部坐标里。关着的 Image 不画。
fn spawn_image(
    commands: &mut Commands,
    parent: Entity,
    image: &ImageSource,
    handle: &Handle<Image>,
    rect: &UiRect,
    depth: f32,
    alpha: f32,
) {
    if !image.enabled {
        return;
    }
    let base = image.base_color();
    for quad in image.quads(rect) {
        let piece = commands
            .spawn((
                Sprite {
                    image: handle.clone(),
                    color: base.with_alpha(base.alpha() * alpha),
                    rect: Some(quad.source),
                    custom_size: Some(quad.max - quad.min),
                    ..default()
                },
                Transform::from_translation(((quad.min + quad.max) * 0.5).extend(depth)),
                TalkWindowPart { base },
                RenderLayers::layer(BALLOON_LAYER),
            ))
            .id();
        commands.entity(parent).add_child(piece);
    }
}

/// 铺名字栏字形（居左居顶）。行结构对账同气泡（摆位 vs 行量律）。
fn spawn_label_glyphs(
    commands: &mut Commands,
    container: Entity,
    art: &BalloonArt,
    style: &TextStyle,
    rect: &UiRect,
    state: &TalkWindowState,
) {
    let text = state.label.as_str();
    let metrics = layout_metrics(
        text,
        style.font_size,
        FONT_FAMILY,
        style.line_spacing,
        BAKE_PPEM,
        &|ch| art.advance(ch),
    );
    let (lines, missing) = walk_glyphs(
        text,
        art,
        style.font_size,
        style.char_extra,
        style.word_extra,
    );
    if missing > 0 {
        warn!("[talkwin] 名字栏缺字形 {missing} 个（label 名字的字符集已并进图集——缺即数据断点）");
    }
    if lines.len() != metrics.line_widths.len() {
        warn!(
            "[talkwin] 名字栏摆位行数 {} 与律行数 {} 不一致：摆位实现漂移",
            lines.len(),
            metrics.line_widths.len()
        );
    }
    spawn_text_glyphs(
        commands,
        container,
        art,
        state,
        &lines,
        &metrics,
        style,
        style.area(rect),
        GlyphSlot::Label,
    );
}

/// 铺正文字形（居左居顶 + 字距/词距增量）。返回 (字形数, 行数, 最宽行
/// 估算 px)——最宽行是超盒告警的量法（每字按其字号量宽，全角即精确）。
fn spawn_content_glyphs(
    commands: &mut Commands,
    container: Entity,
    art: &BalloonArt,
    style: &TextStyle,
    rect: &UiRect,
    state: &TalkWindowState,
) -> (usize, usize, f32) {
    let text = state.text.as_str();
    let metrics = layout_metrics(
        text,
        style.font_size,
        FONT_FAMILY,
        style.line_spacing,
        BAKE_PPEM,
        &|ch| art.advance(ch),
    );
    let (lines, missing) = walk_glyphs(
        text,
        art,
        style.font_size,
        style.char_extra,
        style.word_extra,
    );
    if lines.len() != metrics.line_widths.len() {
        warn!(
            "[talkwin] 正文摆位行数 {} 与律行数 {} 不一致：摆位实现漂移",
            lines.len(),
            metrics.line_widths.len()
        );
    }
    if missing > 0 {
        warn!("[talkwin] 正文缺字形 {missing} 个（候选预筛已把语料字符集并进图集——缺即数据断点）");
    }
    let mut widest = 0.0f32;
    for line in &lines {
        let line_w = line
            .iter()
            .map(|(spot, pen, _)| pen + spot.size * spot.visual)
            .fold(0.0f32, f32::max);
        widest = widest.max(line_w);
    }
    let count = spawn_text_glyphs(
        commands,
        container,
        art,
        state,
        &lines,
        &metrics,
        style,
        style.area(rect),
        GlyphSlot::Content,
    );
    (count, lines.len(), widest)
}

/// 栏位标记：正文字形带打字机序，名字栏字形常显。
enum GlyphSlot {
    Label,
    Content,
}

/// 两栏共用的字形摆位（节点局部坐标）：垂直居顶（首行基线 = 文本区顶 −
/// 首行字号×上行线比）、水平居左（起笔 = 文本区左沿）；行推进按行量律的
/// 行偏移（2x 单位折半）。格内笔点换算与气泡摆位同式。返回字形数。
#[allow(clippy::too_many_arguments)]
fn spawn_text_glyphs(
    commands: &mut Commands,
    container: Entity,
    art: &BalloonArt,
    state: &TalkWindowState,
    lines: &[Vec<(GlyphSpot, f32, usize)>],
    metrics: &LayoutMetrics,
    style: &TextStyle,
    area: Rect,
    slot: GlyphSlot,
) -> usize {
    let size_of_line = |i: usize| -> f32 {
        lines
            .get(i)
            .map(|line| {
                line.iter()
                    .map(|(spot, _, _)| spot.size)
                    .fold(0.0f32, f32::max)
            })
            .filter(|size| *size > 0.0)
            .unwrap_or(style.font_size)
    };
    let baseline_0 = area.max.y - size_of_line(0) * ASCENT_RATIO;
    let (cell, pen_x, baseline_from_top) = art.cell_geometry();
    let [font_r, font_g, font_b, font_a] = style.color;
    let mut count = 0usize;
    for (li, line) in lines.iter().enumerate() {
        let baseline =
            baseline_0 - metrics.line_offsets.get(li).copied().unwrap_or(0.0) / TEXT_SCALE;
        for (spot, pen, raw) in line {
            let Some((cell_rect, _)) = art.glyph_cell(spot.ch) else {
                continue;
            };
            let scale = spot.size * spot.visual / BAKE_PPEM;
            // 格内笔点在 (pen_x, baseline_from_top)；格中心到笔点的偏移按
            // 显示缩放折算，atlas 的 y 向下、canvas 坐标 y 向上取负。
            let dx = (cell / 2.0 - pen_x) * scale;
            let dy = (cell / 2.0 - baseline_from_top) * scale;
            let [r, g, b] = spot.color.unwrap_or([font_r, font_g, font_b]);
            let a = spot.alpha.unwrap_or(font_a) * style.group_alpha;
            let glyph_color = Color::srgba(r, g, b, a);
            let x = area.min.x + pen + dx;
            let y = baseline - dy;
            let revealed = match slot {
                GlyphSlot::Content => *raw < state.shown,
                GlyphSlot::Label => true,
            };
            let mut entity = commands.spawn((
                Sprite {
                    image: art.glyph_image_for(spot.ch).clone(),
                    color: glyph_color.with_alpha(a * state.alpha),
                    rect: Some(cell_rect),
                    custom_size: Some(Vec2::splat(cell * scale)),
                    ..default()
                },
                Transform::from_xyz(x, y, 0.0),
                TalkWindowPart { base: glyph_color },
                Visibility::Hidden,
                RenderLayers::layer(BALLOON_LAYER),
            ));
            match slot {
                GlyphSlot::Content => {
                    entity.insert(TalkGlyph { order: *raw });
                }
                GlyphSlot::Label => {
                    entity.insert(TalkLabelGlyph);
                }
            }
            if revealed {
                entity.insert(Visibility::Visible);
            }
            let id = entity.id();
            commands.entity(container).add_child(id);
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    /// crc32 of the Animator-relative path `Image` (the end mark under EndSign).
    const IMAGE_PATH_HASH: i64 = 83_635_035;
    /// crc32 of `m_AnchoredPosition.y` / `m_AnchoredPosition.x` (RectTransform bindings).
    const ANCHORED_Y: i64 = 538_195_251;
    const ANCHORED_X: i64 = 1_460_864_421;

    fn segments(curve: &serde_json::Value) -> Vec<(f32, [f32; 4])> {
        curve["raw"]
            .as_array()
            .expect("raw segments")
            .iter()
            .map(|row| {
                let c = &row[1];
                let f = |v: &serde_json::Value| v.as_f64().expect("number") as f32;
                (f(&row[0]), [f(&c[0]), f(&c[1]), f(&c[2]), f(&c[3])])
            })
            .collect()
    }

    /// Value check against the source clip. `MOLY_TALK_END_SIGN_CLIP` names
    /// the EndSign controller's `PageForward` clip decoded from the game's
    /// own serialized file (streamed-clip polynomials, bindings, and the
    /// clip evaluated at its 60 fps sample rate by the extractor's decoder).
    /// Red when the source clip's length, loop flag, bound properties,
    /// polynomial coefficients or sampled values differ from what this
    /// module renders.
    #[test]
    #[ignore = "needs the decoded source clip in MOLY_TALK_END_SIGN_CLIP"]
    fn page_forward_matches_the_source_clip() {
        let path = std::env::var("MOLY_TALK_END_SIGN_CLIP").expect("MOLY_TALK_END_SIGN_CLIP");
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("clip file")).expect("json");
        assert_eq!(doc["clip"], "PageForward");
        assert_eq!(doc["loopTime"], true, "the state loops");
        let length = (doc["stop"].as_f64().unwrap() - doc["start"].as_f64().unwrap()) as f32;
        assert_eq!(
            PAGE_FORWARD_LENGTH.to_bits(),
            length.to_bits(),
            "clip length"
        );

        let mut seen = [false; 5];
        for curve in doc["curves"].as_array().expect("curves") {
            let b = &curve["binding"];
            let key = (
                b[0].as_i64().unwrap(),
                b[1].as_i64().unwrap(),
                b[2].as_i64().unwrap(),
                b[3].as_i64().unwrap(),
            );
            assert_eq!(key.2, IMAGE_PATH_HASH, "every curve binds the Image child");
            let exact = |ours: &[(f32, [f32; 4])], name: &str| {
                let source = segments(curve);
                assert_eq!(source.len(), ours.len(), "{name}: segment count");
                for (i, ((st, sc), (ot, oc))) in source.iter().zip(ours).enumerate() {
                    assert_eq!(st.to_bits(), ot.to_bits(), "{name}: key time {i}");
                    for k in 0..4 {
                        assert_eq!(
                            sc[k].to_bits(),
                            oc[k].to_bits(),
                            "{name}: segment {i} coefficient {k}"
                        );
                    }
                }
            };
            match key {
                (4, 3, _, 1) => {
                    exact(&PAGE_FORWARD_SCALE_Y, "localScale.y");
                    seen[0] = true;
                }
                (4, 3, _, 0) | (4, 3, _, 2) => {
                    for (t, c) in segments(curve) {
                        assert_eq!(c, [0.0, 0.0, 0.0, 1.0], "localScale x/z constant 1 at {t}");
                    }
                    seen[1 + key.3 as usize / 2] = true;
                }
                (224, ANCHORED_Y, _, 0) => {
                    exact(&PAGE_FORWARD_POS_Y, "anchoredPosition.y");
                    seen[3] = true;
                }
                (224, ANCHORED_X, _, 0) => {
                    assert_eq!(curve["kind"], "const");
                    assert_eq!(curve["raw"][0][1].as_f64(), Some(0.0), "anchoredPosition.x");
                    seen[4] = true;
                }
                other => panic!("unexpected binding {other:?}"),
            }
        }
        assert_eq!(seen, [true; 5], "every bound property is accounted for");

        // Sampled pose, over two loops, against the decoder's own evaluation.
        let frames = doc["frames60"].as_array().expect("frames60");
        let rate = doc["sampleRate"].as_f64().unwrap() as f32;
        for lap in 0..2 {
            for row in frames.iter().take(frames.len() - 1) {
                let t = row[0].as_f64().unwrap() as f32;
                let clock = t + lap as f32 * PAGE_FORWARD_LENGTH;
                let (anchored, scale_y) = page_forward_pose(clock);
                let want_scale = row[2].as_f64().unwrap() as f32;
                let want_y = row[4].as_f64().unwrap() as f32;
                assert!(
                    (scale_y - want_scale).abs() < 2e-5,
                    "scale.y at {t} lap {lap}: {scale_y} vs {want_scale}"
                );
                assert!(
                    (anchored.y - want_y).abs() < 2e-4,
                    "pos.y at {t} lap {lap}: {} vs {want_y}",
                    anchored.y
                );
                assert_eq!(anchored.x, 0.0);
            }
        }
        // The loop wraps to the first frame at exactly one clip length.
        let (wrap, wrap_scale) = page_forward_pose(PAGE_FORWARD_LENGTH);
        assert_eq!(
            (wrap.y, wrap_scale),
            (frames[0][4].as_f64().unwrap() as f32, 1.0)
        );
        assert_eq!(
            frames.len() as f32 - 1.0,
            (length * rate).round(),
            "sample count covers the clip"
        );
    }
}
