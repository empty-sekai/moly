//! 头顶 tweet 气泡：tweet 主表 → 选取律 → 排版律 → Sprite 呈现。
//!
//! 呈现参数按真源预制体取证的 SPEC 落值（此前的替身已全部换掉）：
//! * **文本**：字号 32（`m_fontSize`，min/max 同 32、autoSizing 关）、
//!   行距 -80（`lineSpacing`，经 [`moly_law::text`] 行量律的行距参数换算）、
//!   色 #555577FF、对齐 = 单行居中多行居左（垂直恒 Middle，靠 bg 上下
//!   对称 padding 成立）。硬断行即真源主形态（源实现无软换行，tweet 文本
//!   73.5% 自带 `\n`，且盒宽由内容首选尺寸驱动、折行条件不成立）。
//! * **盒几何**：真源是 Layout 链（ContentSizeFitter Preferred + bg VLG
//!   padding 左 40 右 40、上 32 下 32 + 行距 2.5）；此处取其近似式——
//!   文本宽 + 80 水平 / 文本高 + 64 垂直（= padding 之和），**注明近似**。
//! * **时序**：入场 DOScale 0→1 与收场 1→0 各 0.2s Linear，作用在
//!   animation 中间节点（真源 `_animationRoot`，非根）；显示 5.0s 是 NPC
//!   状态机时长的对应值（真源 HUD 无自定时器，生死由宿主事件驱动——
//!   事件驱动生命周期在此以驻留沿触发近似）。无点击跳过/关闭语义
//!   （真源 tweet 路径无打字机，点击只挂回调与音效）。
//! * **canvas 缩放**：SPEC 的盒几何/字号是 canvas 单位，根 UI canvas 以
//!   ScaleWithScreenSize（参考 1920×1080、MatchWidthOrHeight）缩放上屏；
//!   match 按屏幕纵横比定（H/W ≥ 1080/1920 → 高度匹配 H/1080，更宽 →
//!   宽度匹配 W/1920）。尺寸缩、锚点屏幕位不缩（乘在根上与顶边缩放相乘）。
//! * **顶边缩放与背面剔除**：屏幕 y>H/2 时整体缩（顶边收 0.5×）；
//!   目标在相机背面时 alpha=0（不销毁）。
//! * **两层绘制序**（气泡之间的覆盖语义）：跨气泡 = 真源 HUD 层的逐帧
//!   重排——`ScreenLayerMysekaiHUD.Update` 每帧调 `SetSiblingTweetHUD`，
//!   键 = 场地相机 transform 到各 HUD 目标（avatar 视图根；投影偏移
//!   (0,1,0) 不参与，排序键与投影锚是两个量）的欧氏距离，按距离**降序**
//!   逐个 `SetSiblingIndex(0..)`，uGUI 兄弟序后者在上 ⇒ **近者的气泡
//!   画在最上、盖住远者**。此处对应到部件 z：按名次分档步进（近者整套
//!   更高，任意两只的部件 z 区间恒不相交——同气泡整体成层，不与另一只
//!   交错）。同气泡内 = 预制体 `animation` 节点的子序（bg 子树在前——
//!   正文文本件是 bg 的**子节点**，箭头容器最后）⇒ **背板 < 正文 <
//!   箭头**（箭头最顶，盖在与背板下沿重叠的 9px 上）。跨 UI 档（对话
//!   窗体、摇杆）在本仓的覆盖相机上按 z 分层，气泡档恒在它们之下
//!   （根部 z 有天花板常量，见 [`BALLOON_LAYER_CEILING`]）。
//!
//! 字体按裁决保持**开源自烘图集**（仓内 OFL 字体；真源是商业 SDF 字体，
//! 资产永不进开源仓）——只还原字号/行距/色/对齐与字重档语义（tweet 正文
//! 只用常规档，粗体档属公告/哭喊变体不在本路径）。图集**两遍光栅化**：
//! 先量字符集实际墨迹的上下左右界再定格边；字号与格几何不随容量缩小。
//! 每页边长不超过当前 RenderDevice 的二维纹理上限，完整字符集按码点
//! 顺序装入多页；字形格记录所属页，排版仍只读原推进与页内矩形。
//! 同数据、同设备限制 ⇒ 同图集。
//!
//! 背景与箭头用 UI atlas 裁件的**真源纹源**（此前是程序化替身，已整块
//! 换掉）：背景 `btn_r30_wh`（CommonAtlas 裁件 80×80、四边 border 38、
//! 真源 Image 组件按 Sliced 九宫渲染）按九宫摆；箭头
//! `balloon_direction_triangle_wh`（同 atlas 裁件 40×20、border 0、
//! Simple 整图）。**选族按引用定，不按名字定**：真源 HUD 预制体里背景
//! 件与箭头件的序列化 sprite 引用（打包身份对齐的账本落名）恰好指向
//! 这两件，而 HUD 类的状态机从不改 sprite 名（按名取图的 setter 在此
//! 路径零调用 ⇒ 序列化引用即所渲染）。名字最像气泡的另一族
//! （思考/哭喊/公告那套 T/S × S/M/L 定尺图）只被共享区域地图的对话
//! 气泡组件引用，不属于本路径——其选档律见下方存档。两者颜色 =
//! 白 × α0.9019608（真源 `m_Color`，sprite 染色，纹理只供像素）。
//!
//! **名栏：真源头顶气泡无此构件**。取证三面：① HUD 预制体整树（根、
//! 动画节点、bg、bg 覆盖件、正文文本、按钮、按钮覆盖、箭头容器、
//! 箭头、箭头覆盖，共 10 节点）只有一件文本件（正文），HUD 类的字段
//! 表里也没有名字文本项；② `label` 步在真源只属于对话窗链——窗体
//! 引擎的名字栏方法是**裸文本 SetText、无背板图**，且窗体只被玩家
//! 点击链、档案回放页、多人房与教程处理器初始化（自主对话链构造上
//! 开不了窗）；③ 名字形候选纹源（`bg_base_mysekainame_h48` /
//! `balloon_mysekaitag_dbl` / `label_framebottom_r14_wh`）在代码调用点
//! 与字符串字面量两处都零引用（字面量检索空间经阳性对照确认存活），
//! 不是 label 步的背板。⇒ 气泡域的 label 步保持具名不呈现（对话模块
//! 的具名跳过日志），**不因盘上有名字形裁片就贴图**。
//!
//! **共享区域地图气泡的选档律（取证存档；真源属于区域地图的对话
//! 气泡组件，本仓当前无消费者，记此防重推导）**：
//! * 路由（表情类型枚举 None/Announce/AnnounceBig/Thinking/Cry/
//!   CryBig = 0..5）：None → 通用底图；Announce 与 AnnounceBig →
//!   公告幅；Thinking → 思考族；Cry 与 CryBig → 哭喊族；其余取值记
//!   错误日志。播放入口先落基础字号 32 与默认字体档，再按型覆盖。
//! * 思考/哭喊族名 = `"balloon_" + thinking|cry + "_" + 竖档 + "_" +
//!   横档`：竖档按**行数**恰 2 取 T 否则 S（枚举 T=0/S=1）；横档按
//!   **定行字符数**（不是像素宽）：<8 → S、8–10 → M、≥11 → L（枚举
//!   L=0/M=1/S=2）。定行字符数默认取**首行**；文本恰 2 字符且含断行
//!   时取前两行中的较长者。
//! * 公告幅整图 `balloon_announce`（该族唯一带 border 的件，四边
//!   40/42/41/40，九宫）；通用底图 `bg_base_r30_wh`。公告与哭喊的
//!   Big 型把字号抬到 40 并换 EB 字体档，否则 32。
//! * 幅面尺寸不走 Layout：组件构造时硬编码三张 `(型, 量) → 值` 查找
//!   表——宽（思考/哭喊按定行字符数 1..12 与 1..10）、高（按行数 1/2）、
//!   底图 Y 偏移（按行数 1/2），查表失败记错误日志。
//!
//! 数据/触发/定位：tweets.json 出主表行与摆设编辑池（join 后形），
//! tweet-tables.json 出问候链三表（WRT 归属 / 问候候选 / 候选条件）；
//! 问候选取只走 [`moly_law::tweet::pick_greeting`]——四条链里唯一带
//! 天气门的一条：条件型 `mysekai_phenomena_time_period` 在选取时比较
//! **当前现象 id** 与条件 value1（名字里的 time_period 是历史命名，比
//! 的不是时段枚举）。真源没有「档位变更即重抽」的监听——门只在选取时
//! 求值，变更只影响下一次选取，这里不做任何变更高压重选。访问计数以 0
//! 喂入（产品没有访问计数，0 是无数据的诚实值：真源三条访问条件
//! [1,5)/[5,∞)/[7,∞) 在 0 处全部不成立，门只剩现象一道——选取随档变化
//! 据此可从日志推导）。
//! 触发两条：问候走「移动相位进入驻留」为演示替身（真源挂在问候/
//! 站点入口两域链上）；摆设编辑链走真源域链——保存回执
//! （`crate::fixture_edit::LayoutSaved`）→ 池选取
//! （[`moly_law::tweet::pick_after_edit_tweet`]）→ 同一呈现链上屏 →
//! 5.0s 驻留（[`AFTER_EDIT_REACTION_DELAY_SECONDS`]）后收场。
//! 驻留在上屏**之后**（真源 objective 的序：选取 → 看向玩家 →
//! 上屏 → 延迟等待 → 收场；反应者抽签与逐角色 500–1000ms 错峰挂账，
//! 这里全员同帧替身），期间该成员被这条反应占用、问候触发沿让位
//! （objective 槽位互斥）；驻留内新回执取消旧反应、重抽重上屏。
//! 定位 = avatar 根位 + 世界偏移
//! (0,1,0) → 主相机投影到屏幕像素 → 盖到只画气泡层的 2D 相机（世界
//! 单位 = 逻辑像素），每帧跟随。日志只打 id、行数、量法、参数实际值——文本内容不进日志
//! （文本含非中英文字符）。

use crate::character::CharacterShell;
use crate::npc::{CharacterUnitId, MotionPhase};
use bevy::asset::{AssetPath, LoadState, RenderAssetUsages};
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::image::Image;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::renderer::RenderDevice;
use moly_assets::json::JsonAsset;
use moly_law::text::advance::{force_fallback_glyph, resolve_glyph_advance};
use moly_law::text::layout_metrics;
use moly_law::text::tags::{parse_rich_segments, transformed_glyphs, SizeSpec, TextSegment};
use moly_law::tweet::{
    condition_matches, greeting_tweet_id, pick_after_edit_tweet, pick_greeting, AfterEditRow,
    GreetingConditionRow, GreetingRow, TweetRow, UniformDraw, WithoutRelatedTalkRow,
    AFTER_EDIT_REACTION_DELAY_SECONDS, CONDITION_TYPE_TIME_PERIOD, CONDITION_TYPE_VISIT_COUNT,
    TWEET_DISPLAY_SECONDS,
};
use std::collections::HashMap;

/// 气泡层的渲染层号：主 3D 相机只看默认层 0，覆盖相机只看本层。
pub(crate) const BALLOON_LAYER: usize = 1;

/// 排版字号（排版像素）。真值：真源文本件 `m_fontSize` = 32（min/max
/// 同 32，autoSizing 关）。取与图集烘制字号同值（1:1 显示）。
const FONT_SIZE: f32 = 32.0;

/// 排版行距。真值：真源文本件 `lineSpacing` = -80（序列化原值，换算在
/// 行量律内部——每行偏移的增量项）。
const LINE_SPACING: f32 = -80.0;

/// 排版字族名：进推进律（只影响空格回退推进比）。真源是商业字体的
/// 常规档；开源自烘字体走默认档（`space_advance_ratio` 的 5/24 支）。
pub(crate) const FONT_FAMILY: &str = "Sans";

/// 图集的烘制字号（px/em）：与显示字号同值，1:1 上屏。
pub(crate) const BAKE_PPEM: f32 = 32.0;

/// 单元格内边距（px）：字形墨迹离格边的留白。
const CELL_PAD: f32 = 2.0;
/// 单页起始边长；实际边长受设备上限约束，放不下的字形继续入下一页。
const MIN_ATLAS_SIZE: usize = 2048;

/// 文字色（真值：`fontColor` #555577FF）。
const TEXT_COLOR: [f32; 3] = [85.0 / 255.0, 85.0 / 255.0, 119.0 / 255.0];

/// 背景与箭头的颜色：白 × α0.9019608（真值：两件 CustomImage 的
/// `m_Color`）。sprite 染色乘在纹理像素上（含纹理自己的透明圆角），
/// 与真源 ugui 的顶点色 × 纹理同式。
const SHAPE_ALPHA: f32 = 0.9019608;
const SHAPE_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, SHAPE_ALPHA);

/// 背景 9-slice 源图边长与四边 border（真值：`btn_r30_wh` 80×80、
/// border 38/38/38/38——UI atlas 裁件账本按打包身份对齐读出；真源
/// Image 组件按 Sliced 渲染）。border 是采样几何，不随换图变。
const BG_SPRITE: f32 = 80.0;
const BG_BORDER: f32 = 38.0;

/// bg 的 VLG padding（真值：左 40 右 40、上 32 下 32；盒宽近似式的 +80/+64 来源）。
const BG_PAD_LEFT: f32 = 40.0;
const BG_PAD_RIGHT: f32 = 40.0;
const BG_PAD_TOP: f32 = 32.0;
const BG_PAD_BOTTOM: f32 = 32.0;

/// animation 节点 VLG 的行距（bg 行与箭头行之间 2.5）。
const ROW_SPACING: f32 = 2.5;

/// 箭头（真值：`balloon_direction_triangle_wh` 40×20，pivot(0.5,1)，
/// anchor(0,1)，aPos.y=+11.5）。箭头行行高 0、越界画出：行顶 =
/// bg 下沿 − 行距 2.5，箭头顶 = 行顶 + 11.5 ⇒ 与 bg 重叠 9px。
const ARROW_W: f32 = 40.0;
const ARROW_H: f32 = 20.0;
const ARROW_POS_Y: f32 = 11.5;

/// 入场/收场 DOScale 时长（真值：各 0.2s，Linear）。
const SCALE_SECONDS: f32 = 0.2;

/// 同一气泡内的部件层 z（真值：HUD 预制体 `animation` 节点的子序——
/// bg 子树在前（正文文本件是 bg 的子节点），箭头容器最后；uGUI 兄弟序
/// 后者画在上 ⇒ 背板最底、正文其上、箭头最顶）。只承相对序；全跨度
/// （最大 [`PART_Z_ARROW`]）须小于跨气泡层的步进，两只气泡的部件
/// 区间才恒不相交。
const PART_Z_BG: f32 = 0.0;
const PART_Z_TEXT: f32 = 0.1;
const PART_Z_ARROW: f32 = 0.2;

/// 跨气泡层的根部 z 步进（名次差一档的间距）：须大于部件层全跨度
/// × 根缩放（0.2 × 顶边缩放(≤1) × canvas 缩放），任意两只气泡的部件
/// z 区间才恒不相交（同气泡整体成层，不与另一只交错）。4.0 覆盖到
/// canvas 缩放 20（7680px 级窗口的 4 倍）。
const BALLOON_LAYER_STRIDE: f32 = 4.0;

/// 气泡根部 z 的天花板：全部气泡（含最近名次）的部件 z 恒低于它 ⇒
/// 恒低于同层覆盖相机上的对话窗体档（z ∈ [-0.1, 0]）与摇杆档
/// （z ∈ [0.2, 0.3]）——跨 UI 的上下序不因名次接线漂移。名次多时
/// 向下延伸（远者更低；2D 覆盖相机正交近裁剪 -1000，250 只内不触）。
const BALLOON_LAYER_CEILING: f32 = -0.5;

/// 层序探针的近根距离（沿相机视线轴，米）：远根取过近锚点的同一
/// 视线射线两倍距离处 ⇒ 两只锚点投影到同一屏幕点（完全重叠）。
const ORDER_PROBE_NEAR: f32 = 4.0;

/// 锚点世界偏移（真值：HUD 基类的 (0, 1, 0)——投影**前**加在目标位置上，
/// 近大远小；锚的是 avatar 根位，不是头骨）。真源锚式 =
/// 单员对话取 avatar 根位 + 偏移；多员取成员根位均值 + 偏移（配置对
/// 话时另有目标 fixture 项参与均值，此处单员式）。
const ANCHOR_OFFSET: Vec3 = Vec3::new(0.0, 1.0, 0.0);

/// 根 UI canvas 的参考分辨率（真值：基准屏常量 (1920, 1080)，与根
/// canvas 缩放件的序列化参考分辨率一致——36 份缩放件全部
/// ScaleWithScreenSize + (1920,1080) + MatchWidthOrHeight）。
pub(crate) const CANVAS_REF_W: f32 = 1920.0;
pub(crate) const CANVAS_REF_H: f32 = 1080.0;

/// canvas 缩放（真值式）：SPEC 的盒几何/字号是 canvas 单位，上屏 = 单位
/// × 本系数。选择律（`ScreenManager.SetUpScreenResolution` 写 match，
/// `CanvasScaler` 的 MatchWidthOrHeight 模式消费它）：
/// 1080/1920 ≤ H/W（屏幕与 16:9 等高或更高）→ match=0 → W/1920；
/// 更宽 → match=1 → H/1080，即完整容纳参考画布。CanvasScaler 的 0 是宽，
/// 1 是高。本层相机 1 单位 = 1 逻辑像素，物理尺寸与窗口 DPI 因子约掉。
pub(crate) fn canvas_scale(width: f32, height: f32) -> f32 {
    if CANVAS_REF_H / CANVAS_REF_W <= height / width {
        width / CANVAS_REF_W
    } else {
        height / CANVAS_REF_H
    }
}

/// 真源 TextMeshPro 排版的坐标倍率（行量法的内部常数；摆位换算要
/// 与它同一坐标系）。
pub(crate) const TEXT_SCALE: f32 = 2.0;
/// 行量法的上/下行线比例（真源常数 66/75、9/75 两形）。
pub(crate) const ASCENT_RATIO: f32 = 66.0 / 75.0;
pub(crate) const DESCENT_RATIO: f32 = 9.0 / 75.0;

/// 仓内开源字体（OFL-1.1 授权文本与字体同目录进仓）。
const FONT_BYTES: &[u8] = include_bytes!("../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");

// ---------------------------------------------------------------------------
// 数据面：tweets.json（主表）+ tweet-tables.json（问候链三表）→ 行类型
// ---------------------------------------------------------------------------

/// tweet 主表的装载请求；解析成功后即撤。主表与问候链三表是两份文件，
/// 到齐一起解析。
#[derive(Resource)]
pub(crate) struct MasterHandle {
    tweets: Handle<JsonAsset>,
    tables: Handle<JsonAsset>,
}

/// 解析后的 tweet 主表：主表行 + 问候链三表（律的入参）+ 摆设编辑链
/// 的池（律的入参）。
///
/// 问候链是四条选取链里唯一带天气门的：候选行经 WRT 归属到角色，再经
/// 条件行过滤——条件型「当前现象时段相等」比的是**当前现象 id** 与
/// value1（见 [`crate::weather::CurrentPhenomenonId`]）。摆设编辑链的
/// 池 = AEH 引用的 WRT 行按角色过滤后的 tweet id 集，无第二道条件
/// （真源 Where 只比角色归属）。
#[derive(Resource)]
pub(crate) struct TweetMaster {
    /// 主表全行，按 id 升序（真源 MessagePack 表序即 id 序；提取产物是
    /// keyed dict，落地后自己排回数值序）。
    tweets: Vec<TweetRow>,
    /// 问候链的 WRT 行：把 tweet 归属给角色的连接脊（问候与站点入口
    /// 两张表都指到这里取角色与 tweet）。
    wrt: Vec<WithoutRelatedTalkRow>,
    /// 问候候选行。
    greetings: Vec<GreetingRow>,
    /// 候选条件行（访问次数区间 / 当前现象 id 相等两型）。
    conditions: Vec<GreetingConditionRow>,
    /// 摆设编辑链的 AEH 行。提取产物把 AEH→WRT→tweet 三表 join 成了
    /// 按角色的池（`afterEditPools`），这里按 WRT 表反查还原成律要的
    /// 行形状——盘上 WRT 的 (角色, tweet) 对零重复，反查恰一行，行 id
    /// 本身不参与选取（律只读 without_related_talk_id）。
    after_edit: Vec<AfterEditRow>,
}

/// tweet-tables.json 的资产路径。问候链三表在这份文件里（主表 tweets
/// 在 tweets.json，另一份）。
fn tweet_tables() -> AssetPath<'static> {
    AssetPath::from("moly://tweet-tables.json".to_owned())
}

impl TweetMaster {
    /// 只取结构键；报错只带字段名与 id——文本内容不进任何报错。
    /// 摆设编辑池（`afterEditPools`，键 = 角色单元 id，值 = tweet id 列）
    /// 与主表同文件，一并取出。
    fn parse_tweets(text: &str) -> (Vec<TweetRow>, Vec<(i32, Vec<i32>)>) {
        let value: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|err| panic!("tweet 主表不是合法 JSON：{err}"));
        let rows = value
            .get("tweets")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("tweet 主表缺 tweets 对象"));
        let mut tweets: Vec<TweetRow> = rows
            .iter()
            .map(|(key, row)| {
                let id: i32 = key
                    .parse()
                    .unwrap_or_else(|_| panic!("tweet 键不是数字 id：{key}"));
                let text = row
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("tweet {id} 缺 text"));
                let motion = match row.get("motion") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(v) => Some(
                        v.as_str()
                            .unwrap_or_else(|| panic!("tweet {id} 的 motion 不是字符串"))
                            .to_owned(),
                    ),
                };
                let eye = row
                    .get("eye")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("tweet {id} 缺 eye"))
                    .to_owned();
                let mouth = row
                    .get("mouth")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("tweet {id} 缺 mouth"))
                    .to_owned();
                // 表情件列：提取侧 v2 起已导出（216 条非空）；空值/缺列
                // 按 None 喂，与行类型口径一致。
                let emoticon = match row.get("emoticon") {
                    None | Some(serde_json::Value::Null) => None,
                    Some(v) => Some(
                        v.as_str()
                            .unwrap_or_else(|| panic!("tweet {id} 的 emoticon 不是字符串"))
                            .to_owned(),
                    ),
                };
                TweetRow {
                    id,
                    motion_name: motion,
                    emoticon_name: emoticon,
                    eye_name: eye,
                    mouth_name: mouth,
                    text: text.to_owned(),
                }
            })
            .collect();
        tweets.sort_by_key(|row| row.id);
        // 池的键与值都要求是整数（提取产物保证；解析失败即数据损伤，
        // 响亮失败在资产边界）。
        let pools = value
            .get("afterEditPools")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("tweet 主表缺 afterEditPools 对象"))
            .iter()
            .map(|(key, ids)| {
                let unit: i32 = key
                    .parse()
                    .unwrap_or_else(|_| panic!("after-edit 池键不是角色单元 id：{key}"));
                let ids = ids
                    .as_array()
                    .unwrap_or_else(|| panic!("after-edit 池 {unit} 不是 tweet id 数组"))
                    .iter()
                    .map(|id| {
                        id.as_i64()
                            .map(|id| id as i32)
                            .unwrap_or_else(|| panic!("after-edit 池 {unit} 里有非整数 tweet id"))
                    })
                    .collect();
                (unit, ids)
            })
            .collect();
        (tweets, pools)
    }

    /// 问候链三表（WRT / 问候 / 条件）。value2 的 null 落成 0——律的
    /// 口径里访问型条件 value2==0 即无上界，现象型不读 value2。
    fn parse_tables(
        text: &str,
    ) -> (
        Vec<WithoutRelatedTalkRow>,
        Vec<GreetingRow>,
        Vec<GreetingConditionRow>,
    ) {
        let value: serde_json::Value = serde_json::from_str(text)
            .unwrap_or_else(|err| panic!("tweet 问候链表不是合法 JSON：{err}"));
        let int_of = |row: &serde_json::Value, key: &str| -> i32 {
            row.get(key)
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| panic!("问候链表行缺 {key}")) as i32
        };
        let wrt: Vec<WithoutRelatedTalkRow> = value
            .get("withoutRelatedTalks")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("tweet 问候链表缺 withoutRelatedTalks 数组"))
            .iter()
            .map(|row| WithoutRelatedTalkRow {
                id: int_of(row, "id"),
                game_character_unit_id: int_of(row, "gameCharacterUnitId"),
                tweet_id: int_of(row, "tweetId"),
            })
            .collect();
        let greetings: Vec<GreetingRow> = value
            .get("greetings")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("tweet 问候链表缺 greetings 数组"))
            .iter()
            .map(|row| GreetingRow {
                id: int_of(row, "id"),
                without_related_talk_id: int_of(row, "withoutRelatedTalkId"),
                greeting_condition_id: int_of(row, "greetingConditionId"),
            })
            .collect();
        let conditions: Vec<GreetingConditionRow> = value
            .get("greetingConditions")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("tweet 问候链表缺 greetingConditions 数组"))
            .iter()
            .map(|row| {
                let condition_type = row
                    .get("conditionType")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("条件行缺 conditionType"))
                    .to_owned();
                // null（现象型条件的 value2）落成 0：访问型 0 = 无上界，
                // 现象型不读它。
                let value2 =
                    row.get("value2")
                        .and_then(|v| match v {
                            serde_json::Value::Null => Some(0),
                            v => v.as_i64(),
                        })
                        .unwrap_or_else(|| panic!("条件行缺 value2")) as i32;
                GreetingConditionRow {
                    id: int_of(row, "id"),
                    condition_type,
                    value1: int_of(row, "value1"),
                    value2,
                }
            })
            .collect();
        (wrt, greetings, conditions)
    }
}

/// 真源纹源两件的装载句柄（bg 九宫源图 + 箭头）。Startup 请求，
/// `bake_atlas` 在两件都到齐后才落 `BalloonArt`——资源在即是闩，
/// 装载失败响亮 panic，不回落任何替身（程序化替身已删）。
#[derive(Resource)]
pub(crate) struct SkinHandle {
    bg: Handle<Image>,
    arrow: Handle<Image>,
}

/// Startup：请求装载 tweet 主表、问候链三表与气泡两件真源纹源。
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(MasterHandle {
        tweets: server.load::<JsonAsset>(moly_assets::tweet_master()),
        tables: server.load::<JsonAsset>(tweet_tables()),
    });
    commands.insert_resource(SkinHandle {
        bg: server.load::<Image>(moly_assets::ui_atlas_sprite("CommonAtlas", "btn_r30_wh")),
        arrow: server.load::<Image>(moly_assets::ui_atlas_sprite(
            "CommonAtlas",
            "balloon_direction_triangle_wh",
        )),
    });
}

/// Update：两份表到齐即解析。装载失败响亮 panic（资产边界的唯一拒绝点）。
pub(crate) fn parse_master(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    handle: Option<Res<MasterHandle>>,
) {
    let Some(handle) = handle else {
        return;
    };
    for (name, handle) in [("主表", &handle.tweets), ("问候链表", &handle.tables)] {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("tweet {name}装载失败：{err:?}");
        }
    }
    let Some(tweets_json) = jsons.get(&handle.tweets) else {
        return;
    };
    let Some(tables_json) = jsons.get(&handle.tables) else {
        return;
    };
    let (tweets, pools) = TweetMaster::parse_tweets(&tweets_json.0);
    let (wrt, greetings, conditions) = TweetMaster::parse_tables(&tables_json.0);
    // 池的 join 形 → 律的 AEH 行形：每条池内 tweet 反查同角色同 tweet 的
    // WRT 行。提取产物以 join 形落盘（AEH 原行不在产物里），但链上引用
    // 是提取期 fail-loud 核过的——反查落空即数据损伤，响亮失败。
    let mut after_edit: Vec<AfterEditRow> = Vec::new();
    for (unit, tweet_ids) in &pools {
        for tweet_id in tweet_ids {
            let wrt_row = wrt
                .iter()
                .find(|w| w.game_character_unit_id == *unit && w.tweet_id == *tweet_id)
                .unwrap_or_else(|| {
                    panic!("after-edit 池 {unit} 的 tweet {tweet_id} 无同角色 WRT 行")
                });
            after_edit.push(AfterEditRow {
                id: after_edit.len() as i32 + 1,
                without_related_talk_id: wrt_row.id,
            });
        }
    }
    let (visit_rows, period_rows) = conditions.iter().fold((0usize, 0usize), |(v, p), c| {
        if c.condition_type == CONDITION_TYPE_VISIT_COUNT {
            (v + 1, p)
        } else if c.condition_type == CONDITION_TYPE_TIME_PERIOD {
            (v, p + 1)
        } else {
            (v, p)
        }
    });
    info!(
        "[tweet] 表就绪：主表 {} 条 + 问候链三表（WRT {} · 候选 {} · 条件 {}：访问型 {visit_rows} / 现象型 {period_rows}）+ after-edit 池 {} 单元 {} 行",
        tweets.len(),
        wrt.len(),
        greetings.len(),
        conditions.len(),
        pools.len(),
        after_edit.len(),
    );
    commands.insert_resource(TweetMaster {
        tweets,
        wrt,
        greetings,
        conditions,
        after_edit,
    });
    commands.remove_resource::<MasterHandle>();
}

// ---------------------------------------------------------------------------
// 字形图集与真源纹源（UI atlas 裁件）的装载
// ---------------------------------------------------------------------------

/// 一个字形在图集里的格：格矩形（图像像素坐标，min.y 从图像顶部量）与
/// 基准字号下的推进。
pub(crate) struct GlyphCell {
    rect: Rect,
    advance: f32,
    page: usize,
}

/// 气泡的全部纹源：字形图集 + 背景九宫源图 + 箭头源图。
#[derive(Resource)]
pub(crate) struct BalloonArt {
    /// 字形图集各页（白墨 + 覆盖度 alpha，染色靠 sprite）。至少有一页。
    images: Vec<Handle<Image>>,
    cells: HashMap<char, GlyphCell>,
    /// 格内基线行到格顶的距离与笔点列偏移（摆位换算要它们，与烘制同式）。
    baseline_from_top: f32,
    pen_x: f32,
    /// 格边长（两遍法按实际墨迹定的，见 `bake_atlas`）。
    cell: f32,
    /// 背景 9-slice 源图（真源纹源：UI atlas 裁件 `btn_r30_wh`，
    /// 80×80、border 38 四边；九宫采样几何用上面的常量）。
    bg: Handle<Image>,
    /// 箭头源图（真源纹源：UI atlas 裁件 `balloon_direction_triangle_wh`，
    /// 40×20、border 0）。
    arrow: Handle<Image>,
}

impl BalloonArt {
    /// 取一个字符的字形格（图集图像像素坐标的矩形 + 基准推进）。
    /// 没烘进的字符返回 `None`（缺字形在烘制时已具名计数）。
    pub(crate) fn glyph_cell(&self, ch: char) -> Option<(Rect, f32)> {
        self.cells.get(&ch).map(|c| (c.rect, c.advance))
    }

    /// 首页面身份供整组图集的缓存失效判断；绘制字符应取 glyph_image_for。
    pub(crate) fn glyph_image(&self) -> &Handle<Image> {
        &self.images[0]
    }

    /// 字符所在页的纹理句柄；须与同字符的 glyph_cell 页内矩形配对。
    /// 调用者先用 glyph_cell 处理缺字，不能用首页面替代已在后页的字形。
    pub(crate) fn glyph_image_for(&self, ch: char) -> &Handle<Image> {
        let cell = self
            .cells
            .get(&ch)
            .expect("glyph_image_for requires a baked glyph");
        &self.images[cell.page]
    }

    /// 格几何三件：格边长、格内笔点列偏移、基线到格顶距离。
    /// 摆位换算与 `tick` 同式（格中心到笔点的偏移那一套）。
    pub(crate) fn cell_geometry(&self) -> (f32, f32, f32) {
        (self.cell, self.pen_x, self.baseline_from_top)
    }
}

/// Update：主表就绪即烘一次图集（资源在即是闩）。字体在仓内
/// （include_bytes），光栅走 swash 的 `scale::Render`。烘制次序按码点
/// 升序——同数据同图。真源纹源两件（bg/箭头裁图）在同一次调用里
/// 定门：装载失败具名拒绝，未到齐等下一帧——BalloonArt 只在两者
/// 都在场后落，之后没有任何路径再生成替身图。
pub(crate) fn bake_atlas(
    mut commands: Commands,
    server: Res<AssetServer>,
    render_device: Res<RenderDevice>,
    mut images: ResMut<Assets<Image>>,
    master: Option<Res<TweetMaster>>,
    // 对话候选字符集：与 tweet 并集一次烘全（之后到的字符不上屏，
    // 候选集在对话模块的预筛侧已定格）。
    talk_charset: Option<Res<crate::talk::TalkCharset>>,
    // 玩家对话字符集：同一张图集的第三个并集成员（窗体正文/名字栏与
    // tweet 文案共用）。装载期定格、全 unit 收录——候选预筛在名册后
    // 才做，此处按超集并（字形只多不少）。
    player_talk_charset: Option<Res<crate::player_talk::PlayerTalkCharset>>,
    // 场地屏外壳字符集（固定文案 + 站名）：第四个并集成员。与上面三个
    // 同一定格语义：外壳文案全部装载期可枚举（站点主表是根级小表）。
    shell_charset: Option<Res<crate::menu_shell::ShellTextCharset>>,
    skin: Option<Res<SkinHandle>>,
    art: Option<Res<BalloonArt>>,
) {
    if art.is_some() {
        return;
    }
    let Some(skin) = skin else {
        return;
    };
    for (name, handle) in [
        ("bg btn_r30_wh", &skin.bg),
        ("arrow balloon_direction_triangle_wh", &skin.arrow),
    ] {
        if let LoadState::Failed(err) = server.load_state(handle) {
            panic!("[tweet] 气泡纹源 {name} 装载失败（fail-closed，不回落替身）：{err:?}");
        }
    }
    let loaded = |h: &Handle<Image>| matches!(server.load_state(h), LoadState::Loaded);
    if !(loaded(&skin.bg) && loaded(&skin.arrow)) {
        return;
    }
    // 尺寸核验：裁件尺寸与九宫/箭头采样常量脱节时，错采样是静默的——
    // 具名拒绝。两值都读自真源（sprite 账本按打包身份对齐）。
    let bg_image = images
        .get(&skin.bg)
        .unwrap_or_else(|| panic!("[tweet] 气泡纹源 bg 状态 Loaded 但图像缺席"));
    if bg_image.size().x != BG_SPRITE as u32 || bg_image.size().y != BG_SPRITE as u32 {
        panic!(
            "[tweet] 气泡纹源 btn_r30_wh 裁件 {}x{} 与九宫常量 {:.0}x{:.0} 不符：提取产物与 SPEC 脱节",
            bg_image.size().x,
            bg_image.size().y,
            BG_SPRITE,
            BG_SPRITE
        );
    }
    let arrow_image = images
        .get(&skin.arrow)
        .unwrap_or_else(|| panic!("[tweet] 气泡纹源 arrow 状态 Loaded 但图像缺席"));
    if arrow_image.size().x != ARROW_W as u32 || arrow_image.size().y != ARROW_H as u32 {
        panic!(
            "[tweet] 气泡纹源 balloon_direction_triangle_wh 裁件 {}x{} 与常量 {:.0}x{:.0} 不符：提取产物与 SPEC 脱节",
            arrow_image.size().x,
            arrow_image.size().y,
            ARROW_W,
            ARROW_H
        );
    }
    let (Some(master), Some(talk_charset), Some(player_talk_charset), Some(shell_charset)) =
        (master, talk_charset, player_talk_charset, shell_charset)
    else {
        return;
    };
    let font = swash::FontRef::from_index(FONT_BYTES, 0)
        .unwrap_or_else(|| panic!("仓内字体不是可读的 OpenType 字体"));
    let mut chars: Vec<char> = Vec::new();
    for row in &master.tweets {
        for ch in row.text.chars() {
            if ch != '\n' && !chars.contains(&ch) {
                chars.push(ch);
            }
        }
    }
    for ch in &talk_charset.chars {
        if *ch != '\n' && !chars.contains(ch) {
            chars.push(*ch);
        }
    }
    for ch in &player_talk_charset.chars {
        if *ch != '\n' && !chars.contains(ch) {
            chars.push(*ch);
        }
    }
    for ch in &shell_charset.chars {
        if *ch != '\n' && !chars.contains(ch) {
            chars.push(*ch);
        }
    }
    chars.sort_unstable();

    let mut context = swash::scale::ScaleContext::new();
    let mut scaler = context.builder(font).size(BAKE_PPEM).build();
    let render = swash::scale::Render::new(&[swash::scale::Source::Outline]);
    let glyph_metrics = font.glyph_metrics(&[]).scale(BAKE_PPEM);

    // 第一遍：全部光栅化，量**实际墨迹**的上/下/左/右界。格边仍按
    // 实际墨迹定而不是 hhea 上下界；分页只改变存储，不缩字或裁字符集。
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
            // 有 cmap 项、无墨迹的字形（空白类）：推进入表，无墨可烘。
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
    // 第二遍前的格几何：笔点钉在 (pen_x, baseline_from_top)，四边各留
    // CELL_PAD 与 1px 松弛。格取宽高较大者（方格）。
    let pen_x = CELL_PAD + (-ink_left).max(0.0);
    let baseline_from_top = CELL_PAD + ink_up.ceil() + 1.0;
    let cell = (baseline_from_top + ink_down.ceil() + 1.0 + CELL_PAD)
        .max(pen_x + ink_right.ceil() + 1.0 + CELL_PAD)
        .ceil();
    // Keep the measured glyph size. Grow a page only within this device's limit;
    // additional glyphs get another page with the same cell geometry.
    let required_cols = (rasters.len() as f64).sqrt().ceil() as usize;
    let max_edge = render_device.limits().max_texture_dimension_2d as usize;
    let atlas_size = MIN_ATLAS_SIZE
        .max(required_cols * cell as usize)
        .min(max_edge);
    let cols = atlas_size / cell as usize;
    assert!(
        cols > 0,
        "device texture limit {max_edge}px cannot hold one {cell}px glyph cell"
    );
    let page_capacity = cols * cols;
    let page_count = rasters.len().div_ceil(page_capacity).max(1);
    let capacity = page_capacity * page_count;
    let mut glyph_images = Vec::with_capacity(page_count);
    let mut data = vec![0u8; atlas_size * atlas_size * 4];
    let mut cells: HashMap<char, GlyphCell> = HashMap::new();
    for (index, r) in rasters.iter().enumerate() {
        let page = index / page_capacity;
        let page_index = index % page_capacity;
        if page_index == 0 && index != 0 {
            glyph_images.push(images.add(Image::new(
                Extent3d {
                    width: atlas_size as u32,
                    height: atlas_size as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                data,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            )));
            data = vec![0u8; atlas_size * atlas_size * 4];
        }
        let col = page_index % cols;
        let row = page_index / cols;
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
                    page,
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
                "字形 U+{:04X} 的墨迹 {}x{} 装不进 {:.0}px 格（墨迹上界 {:.1} 下界 {:.1} 左界 {:.1} 右界 {:.1}）：格几何与第一遍量得的界不符",
                r.ch as u32, r.width, r.height, cell, ink_up, ink_down, ink_left, ink_right
            );
        }
        for gy in 0..r.height {
            for gx in 0..r.width {
                let coverage = r.data[gy * r.width + gx];
                let px = origin_x as usize + gx;
                let py = origin_y as usize + gy;
                let at = (py * atlas_size + px) * 4;
                // 白色墨 + 覆盖度进 alpha；染色靠 sprite 的 color。
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
                page,
            },
        );
    }

    let missing_hex: Vec<String> = missing
        .iter()
        .map(|ch| format!("U+{:04X}", *ch as u32))
        .collect();
    info!(
        "字形图集烘成：{} 格（字符集 {}，{page_count} 页 {atlas_size}x{atlas_size}，每页容量 {page_capacity}，总容量 {capacity}，设备上限 {max_edge}），缺字形 {} 个 [{}]，烘制 {:.0}px 实际墨迹上界 {:.1} 下界 {:.1} ⇒ 格 {:.0}px 笔点=({pen_x:.0},{baseline_from_top:.0})",
        cells.len(),
        chars.len(),
        missing.len(),
        missing_hex.join(" "),
        BAKE_PPEM,
        ink_up,
        ink_down,
        cell
    );
    glyph_images.push(images.add(Image::new(
        Extent3d {
            width: atlas_size as u32,
            height: atlas_size as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )));
    let bg = skin.bg.clone();
    let arrow = skin.arrow.clone();
    info!(
        "[tweet] 气泡纹源就位：bg btn_r30_wh {:.0}x{:.0} border {:.0} 四边（九宫 Sliced）· arrow balloon_direction_triangle_wh {:.0}x{:.0}（整图 Simple）",
        BG_SPRITE, BG_SPRITE, BG_BORDER, ARROW_W, ARROW_H
    );
    commands.insert_resource(BalloonArt {
        images: glyph_images,
        cells,
        baseline_from_top,
        pen_x,
        cell,
        bg,
        arrow,
    });
    commands.remove_resource::<SkinHandle>();
}

// ---------------------------------------------------------------------------
// 触发与选取
// ---------------------------------------------------------------------------

/// 每名成员的触发状态：走姿相位时武装，进驻留的那一帧开火一次。
#[derive(Component)]
pub(crate) struct TweetArm {
    armed: bool,
    /// 第几次驻留（喂随机源种子，让每次驻留抽到不同位置）。
    dwell: u32,
}

/// 跨帧线性同余抽签：均匀落在 `[0, len)`。律只约束分布与「恰求值一次」，
/// 引擎序列不在律内；种子随成员与驻留次数走。
struct Lcg(u64);

impl UniformDraw for Lcg {
    fn draw(&mut self, len: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % len.max(1)
    }
}

/// 带计数器的抽签：量出「这条链这次真的只抽了一次」。
struct CountedDraw {
    inner: Lcg,
    calls: usize,
    last_len: usize,
}

impl CountedDraw {
    fn new(seed: u64) -> Self {
        Self {
            inner: Lcg(seed),
            calls: 0,
            last_len: 0,
        }
    }
}

impl UniformDraw for CountedDraw {
    fn draw(&mut self, len: usize) -> usize {
        self.calls += 1;
        self.last_len = len;
        self.inner.draw(len)
    }
}

/// 访问计数的替身值：产品没有访问计数（什么算一次「重访」归站点域，
/// 尚无对应物），0 是无数据的诚实值——真源三条访问条件 [1,5)/[5,∞)/
/// [7,∞) 在 0 处全部不成立，问候门只剩现象一道。真源选取不掷默认值，
/// 计数由调用方传入；本替身固定喂 0。
const VISIT_COUNT_STAND_IN: i32 = 0;

/// Update：驻留沿开火——走姿帧武装，进驻留帧对问候链跑一次选取（选取
/// 只走 [`pick_greeting`]；真源的问候触发挂在玩家入站的事件上，驻留沿
/// 是演示替身）。只对装配完成的成员跑（有外壳即装配毕）。
#[allow(clippy::type_complexity)]
pub(crate) fn trigger(
    mut commands: Commands,
    library: Res<crate::content_library::ContentLibrary>,
    master: Option<Res<TweetMaster>>,
    art: Option<Res<BalloonArt>>,
    // 选取时读当前现象 id——真源的门在选取时求值，不监听档位变更。
    phenomena: Res<crate::weather::CurrentPhenomenonId>,
    mut npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            &MotionPhase,
            Option<&mut TweetArm>,
        ),
        (
            With<CharacterShell>,
            Without<crate::player::PlayerControlled>,
            // 对话参演者（TalkHold）整名让位：驻留相位是对话的持留
            // 形态，不是 tweet 的触发沿，不在对话头上再叠一只 tweet 气泡。
            Without<crate::talk::TalkHold>,
            // after-edit 反应驻留中的成员同样让位：真源 objective 槽位
            // 互斥，摆设编辑反应占着槽位时问候 objective 进不来。组件经
            // 延迟命令插入，同帧碰撞拦不住，粒度一帧。
            Without<AfterEditHold>,
        ),
    >,
) {
    if library.owns_scene() {
        return;
    }
    let (Some(master), Some(art)) = (master, art) else {
        return;
    };
    for (npc, unit, phase, arm) in &mut npcs {
        let walking = matches!(phase, MotionPhase::Walking | MotionPhase::FitWalking { .. });
        let Some(mut arm) = arm else {
            // 首见：默认武装，让第一次驻留就能出 tweet。
            commands.entity(npc).insert(TweetArm {
                armed: true,
                dwell: 0,
            });
            continue;
        };
        if walking {
            arm.armed = true;
            continue;
        }
        if !arm.armed {
            continue;
        }
        arm.armed = false;
        arm.dwell += 1;
        let unit_id = unit.0 as i32;
        let seed = (unit_id as u64) << 32 | arm.dwell as u64;
        let mut draw = CountedDraw::new(seed);
        let picked = pick_greeting(
            &master.greetings,
            &master.wrt,
            &master.conditions,
            unit_id,
            VISIT_COUNT_STAND_IN,
            phenomena.0,
            &mut draw,
        );
        let Some(greeting) = picked else {
            let (by_character, matched) =
                pool_report(&master, unit_id, VISIT_COUNT_STAND_IN, phenomena.0);
            info!(
                "[tweet] unit={} 驻留 #{}：现象={} 访问={} → 候选 {} 条（角色命中 {} 条，条件挡下 {} 条），不抽签跳过",
                unit.0,
                arm.dwell,
                phenomena.0,
                VISIT_COUNT_STAND_IN,
                matched,
                by_character,
                by_character - matched
            );
            continue;
        };
        // WRT 一跳解析 tweet id；选取成功的行必然能解析（角色过滤已要求
        // WRT 在场），此处只是数据损伤时的响亮出口。
        let Some(tweet_id) = greeting_tweet_id(greeting, &master.wrt) else {
            warn!(
                "[tweet] unit={} 驻留 #{}：问候 {} 指向的 WRT 行不在表里，跳过",
                unit.0, arm.dwell, greeting.id
            );
            continue;
        };
        let Some(row) = master.tweets.iter().find(|t| t.id == tweet_id) else {
            warn!(
                "[tweet] unit={} 驻留 #{}：tweet {tweet_id} 不在主表里，跳过",
                unit.0, arm.dwell
            );
            continue;
        };
        info!(
            "[tweet] unit={} 驻留 #{}：现象={} 访问={} → tweet={} 入选（候选 {} 条，抽签恰 {} 次；显示 5.0s=NPC 状态机对应值）",
            unit.0,
            arm.dwell,
            phenomena.0,
            VISIT_COUNT_STAND_IN,
            row.id,
            draw.last_len,
            draw.calls
        );
        spawn_balloon(&mut commands, &art, npc, unit.0, row);
    }
}

/// after-edit 反应驻留：上屏后的保持段（真源 objective 的静态
/// `_waitTime` 5.0s，乘 1000 入延迟等待）。驻留内该成员的 objective
/// 槽位被这条反应占用：问候触发沿见它让位；新保存回执取消它重启
/// （真源 `TryCancelCurrentObjective`，本 objective 恒可取消）。
#[derive(Component)]
pub(crate) struct AfterEditHold {
    /// 剩余驻留（秒）。
    remaining: f32,
    /// 第几次保存触发（收场日志对账用）。
    sequence: u32,
}

/// Update：摆设编辑保存回执 → after-edit 池选取 → tweet 上屏 → 5.0s
/// 驻留收场（呈现复用 [`spawn_balloon`]，与问候链同一管线）。真源链 =
/// 保存回执进 `MysekaiAfterEditNPCUtility.OnEdit`（每张回执都进，不看
/// 载荷）→ 逐角色挂反应 objective（反应者抽签与 500–1000ms 错峰挂账，
/// 全员同帧替身）→ objective `ExecuteAsync`：**开始即选取**
/// （`RandomPick` 恰一次，律 [`pick_after_edit_tweet`] 逐条对齐）→
/// 看向玩家（挂账）→ 上屏（`SetTweetId`/`SetTweetType`/进 tweet 状态）
/// → 5.0s 驻留 → 收场（`ForceUpdateObjective`）。**驻留在上屏之后**
/// ——不是先等 5.0s 再显示。池空真源静默收场（不显示也不驻留，
/// objective 直接收场），这里同语义具名一行。驻留内新回执：真源取消
/// 旧 objective 重挂新链——这里同形，重抽、重上屏、驻留重起；上屏前
/// 该成员头上的在屏气泡先关（objective 槽位只有一个，新反应进场等于
/// 旧状态出场，跨链的问候气泡同一条让位门——问候触发与本链在
/// schedule 里链式定序加显式同步点，同帧铺出的问候气泡当帧可见、
/// 当帧让位，不定序并发时这一格是盲的）。成员集与问候触发一致
/// （非玩家、非对话参演者——对话持留头上不叠气泡是同一条呈现门；
/// 真源会对对话中成员取消对话强挂反应，跨域让位门是本侧替身，具名）。
#[allow(clippy::type_complexity)]
pub(crate) fn after_edit_reaction(
    mut commands: Commands,
    time: Res<Time>,
    mut saves: MessageReader<crate::fixture_edit::LayoutSaved>,
    master: Option<Res<TweetMaster>>,
    art: Option<Res<BalloonArt>>,
    mut sequence: Local<u32>,
    npcs: Query<
        (Entity, &CharacterUnitId),
        (
            With<CharacterShell>,
            Without<crate::player::PlayerControlled>,
            Without<crate::talk::TalkHold>,
        ),
    >,
    mut holds: Query<(Entity, &CharacterUnitId, &mut AfterEditHold)>,
    balloons: Query<(Entity, &BalloonAnchor)>,
) {
    // 驻留收段先走（真源延迟等待到点 → ForceUpdateObjective）：到点
    // 让位；同帧新回执的驻留从满值重起。
    for (npc, unit, mut hold) in &mut holds {
        hold.remaining -= time.delta_secs();
        if hold.remaining <= 0.0 {
            info!(
                "[tweet] after-edit unit={} 保存 #{}：驻留满 {:.1}s，反应收场（objective 让位）",
                unit.0, hold.sequence, AFTER_EDIT_REACTION_DELAY_SECONDS
            );
            commands.entity(npc).remove::<AfterEditHold>();
        }
    }
    let receipts = saves.read().count();
    if receipts == 0 {
        return;
    }
    *sequence += receipts as u32;
    let sequence = *sequence;
    let (Some(master), Some(art)) = (master, art) else {
        return;
    };
    info!(
        "[tweet] after-edit：保存回执 {} 张（第 {} 次）→ 逐成员反应（同帧替身）",
        receipts, sequence
    );
    let mut reacted = 0usize;
    for (npc, unit) in &npcs {
        let unit_id = unit.0 as i32;
        let mut draw = CountedDraw::new((unit_id as u64) << 32 | sequence as u64);
        let picked = pick_after_edit_tweet(
            &master.after_edit,
            &master.wrt,
            &master.tweets,
            unit_id,
            &mut draw,
        );
        let Some(row) = picked else {
            info!(
                "[tweet] after-edit unit={} 保存 #{sequence}：可解析池空（角色无池条目=数据缺席），不抽签跳过（真源 objective 直接收场同语义）",
                unit.0
            );
            continue;
        };
        reacted += 1;
        // 新反应进场：头上的在屏气泡先关（驻留内重抽、跨链的问候
        // 气泡，同一条让位门——槽位只有一个）。
        for (balloon_entity, anchor) in &balloons {
            if anchor.npc == npc {
                info!(
                    "[tweet] after-edit unit={} 保存 #{sequence}：在屏 tweet={} 让位（objective 槽位互斥，重抽）",
                    unit.0, anchor.tweet
                );
                commands.entity(balloon_entity).despawn();
            }
        }
        info!(
            "[tweet] after-edit unit={} 保存 #{sequence}：池 {} 条 → tweet={} 入选（抽签恰 {} 次）→ 上屏，驻留 {:.1}s 起",
            unit.0,
            draw.last_len,
            row.id,
            draw.calls,
            AFTER_EDIT_REACTION_DELAY_SECONDS
        );
        spawn_balloon(&mut commands, &art, npc, unit.0, row);
        commands.entity(npc).insert(AfterEditHold {
            remaining: AFTER_EDIT_REACTION_DELAY_SECONDS,
            sequence,
        });
    }
    info!(
        "[tweet] after-edit 保存 #{sequence}：在场 {} 名、入选 {reacted}",
        npcs.iter().count()
    );
}

/// 池组成的对账量法（只进日志，不参与选取——选取只走律）：角色过滤后
/// 与条件过滤后各几条。空池时读者从日志直接看出空在哪一道，不必回表
/// 推导。角色一跳与律同式（WRT 行在场且归属相同）；条件判定直接调律
/// 的 [`condition_matches`]，不另写一份。
fn pool_report(master: &TweetMaster, unit: i32, visit: i32, phenomena: i32) -> (usize, usize) {
    let mut by_character = 0;
    let mut matched = 0;
    for greeting in &master.greetings {
        let character_hit = master
            .wrt
            .iter()
            .find(|w| w.id == greeting.without_related_talk_id)
            .map(|w| w.game_character_unit_id == unit)
            .unwrap_or(false);
        if !character_hit {
            continue;
        }
        by_character += 1;
        let condition_hit = master
            .conditions
            .iter()
            .find(|c| c.id == greeting.greeting_condition_id)
            .is_some_and(|c| condition_matches(c, visit, phenomena));
        if condition_hit {
            matched += 1;
        }
    }
    (by_character, matched)
}

// ---------------------------------------------------------------------------
// 排版与铺装
// ---------------------------------------------------------------------------

/// 一只气泡：挂在哪名成员、对账量、相机背面剔除的当前态。
#[derive(Component)]
pub(crate) struct BalloonAnchor {
    npc: Entity,
    /// 对账 id——tweet 路径是 tweet id，对话路径是对话段 id。
    tweet: i32,
    /// 目标当前在相机背面（alpha=0 的门；进入/离开各报一次）。
    hidden: bool,
    /// 盒尺寸（canvas 单位，bg 的 W×H）——探针用：place 按窗口算完
    /// 缩放后报「最终显示像素」要拿它乘。
    box_size: Vec2,
    /// 探针根位（层序探针直供固定世界位，不走 npc 变换）；普通气泡
    /// 为 None（根位 = npc 的全局变换，每帧现读）。
    probe_root: Option<Vec3>,
    /// 层序探针气泡（判据脚本按它过滤掉同屏的普通气泡）。
    probe: bool,
}

/// DOScale 的作用节点（真源 `_animationRoot`：入场/收场缩放打在它上，
/// 不打根）。原点取 bg 中心——缩放向气泡中心收。
#[derive(Component)]
// The detached root's reversible editor Visibility must reach its Sprite
// descendants through this non-rendering intermediate node. Bevy stops
// propagation at a parent without Visibility/InheritedVisibility; Sprite then
// falls back to visible, even while the BalloonAnchor itself is Hidden.
#[require(Visibility)]
pub(crate) struct BalloonAnim;

/// 气泡的存活计时与时序相位（入场完成/收场开始各报一次）。
#[derive(Component)]
pub(crate) struct Elapsed {
    t: f32,
    entered: bool,
    exiting: bool,
}

/// 首次投影成功后的置位闩（投影值只报一次，避免刷屏）。
#[derive(Component)]
pub(crate) struct Placed;

/// 层序探针的稳态闩（每部件的最终排序键只报一次——须在入场完成后的
/// 稳态打：入场/收场 DOScale 会把部件层差缩没，t=0 时无从比）。
#[derive(Component)]
pub(crate) struct LayerLogged;

/// 气泡部件的层族（同一气泡内的绘制序）。层 z 值见 [`PART_Z_BG`] 一族
/// 的出处注释。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PartKind {
    Bg,
    Text,
    Arrow,
}

impl PartKind {
    /// 部件层 z（局部，乘在 animation 节点之下；真源子序的对应）。
    fn layer_z(self) -> f32 {
        match self {
            PartKind::Bg => PART_Z_BG,
            PartKind::Text => PART_Z_TEXT,
            PartKind::Arrow => PART_Z_ARROW,
        }
    }

    /// 日志名（ASCII——判据脚本按它解析，非 ASCII 不进日志）。
    fn tag(self) -> &'static str {
        match self {
            PartKind::Bg => "bg",
            PartKind::Text => "text",
            PartKind::Arrow => "arrow",
        }
    }
}

/// 气泡的一个 sprite 部件：基础色（相机背面剔除的 alpha 门写回）与
/// 层族（排序键日志用）。
#[derive(Component)]
pub(crate) struct BalloonPart {
    base: Color,
    kind: PartKind,
}

/// 一个字形的落位：行内笔点 + 段样式。
#[derive(Clone, Copy)]
pub(crate) struct GlyphSpot {
    pub(crate) ch: char,
    /// 段字号（进推进律与显示缩放）。
    pub(crate) size: f32,
    /// 视觉缩放倍数：段的 `<scale>` × 大小写变换的字倍率。
    pub(crate) visual: f32,
    pub(crate) color: Option<[f32; 3]>,
    pub(crate) alpha: Option<f32>,
}

/// 段字号解析：与行量法的私有解析同语义（None→层字号；绝对/增量/
/// 百分比/em）。
fn segment_size(seg: &TextSegment, font_size: f32) -> f32 {
    match &seg.size {
        None => font_size,
        Some(SizeSpec::Absolute(v)) => *v,
        Some(SizeSpec::Delta(v)) => font_size + *v,
        Some(SizeSpec::Percent(v)) => font_size * *v / 100.0,
        Some(SizeSpec::Em(v)) => font_size * *v,
    }
}

fn starts_with(left: &[char], prefix: &[char]) -> bool {
    left.len() >= prefix.len() && left.iter().zip(prefix.iter()).all(|(a, b)| a == b)
}

/// 逐字走一遍文本：行、段消费次序、笔点推进全按行量法的次序——
/// 推进逐式同源（`resolve_glyph_advance` × 段缩放；`cspace` 每字一笔、
/// 行尾再补一次；`<space=N>` 固定段无消费跟踪）。字形 quad 只对字形表
/// 命中的字符出（缺字形只占位并计数，推进与律同一回退公式）。
/// `char_extra`/`word_extra` 是真源字距的增量（`m_characterSpacing` 与
/// `m_wordSpacing` 的消费式：值 × 0.01 × 字号，正交字体乘 1；空白字
/// 后另补 word 一笔）——气泡两值皆 0，对话窗体带非 0 值进来。
/// 返回（每行的字形、笔点与**原文下标**，缺字形数）。原文下标供打字机
/// 按 `text[i]` 的 i 序揭示（换行占一拍：原文下标把 `\n` 计入）。
pub(crate) fn walk_glyphs(
    text: &str,
    art: &BalloonArt,
    font_size: f32,
    char_extra: f32,
    word_extra: f32,
) -> (Vec<Vec<(GlyphSpot, f32, usize)>>, usize) {
    let segments = parse_rich_segments(text);
    let mut clean: String = segments.iter().map(|seg| seg.text.as_str()).collect();
    if clean.ends_with('\n') {
        clean.pop();
    }
    let line_texts: Vec<String> = clean.split('\n').map(str::to_string).collect();
    let seg_cleans: Vec<Vec<char>> = segments
        .iter()
        .map(|seg| seg.text.chars().filter(|ch| *ch != '\n').collect())
        .collect();
    // 段内「净序 → 原文下标」表（`\n` 占原文下标）：打字机序要原文 i。
    // 前提是无标签富文本（段文本拼接即原文；语料已核 0 标签）。
    let seg_raw_of_clean: Vec<Vec<usize>> = segments
        .iter()
        .map(|seg| {
            let mut map = Vec::new();
            let mut raw = 0usize;
            for ch in seg.text.chars() {
                if ch != '\n' {
                    map.push(raw);
                }
                raw += 1;
            }
            map
        })
        .collect();
    let seg_raw_base: Vec<usize> = {
        let mut base = Vec::with_capacity(segments.len());
        let mut acc = 0usize;
        for seg in &segments {
            base.push(acc);
            acc += seg.text.chars().count();
        }
        base
    };

    let mut consumed = vec![0usize; segments.len()];
    let mut lines: Vec<Vec<(GlyphSpot, f32, usize)>> = vec![Vec::new(); line_texts.len()];
    let mut missing_used = 0usize;
    for (li, line_text) in line_texts.iter().enumerate() {
        let mut remaining: Vec<char> = line_text.chars().collect();
        let mut pen = 0.0f32;
        for (si, seg) in segments.iter().enumerate() {
            if remaining.is_empty() {
                break;
            }
            if let Some(fixed) = seg.fixed_advance {
                // 固定段：推进进笔点，无字形、无消费跟踪（行量法同款）。
                pen += fixed / TEXT_SCALE;
                continue;
            }
            let sc = &seg_cleans[si];
            if sc.is_empty() || consumed[si] >= sc.len() {
                continue;
            }
            // 本行的段内净序基准：取走前先记——取走动作会把 consumed 推到
            // 行尾，事后取它当基准必越界（首次 tweet 即撞）。
            let raw_base_in_seg = consumed[si];
            let rest = &sc[consumed[si]..];
            let part: Vec<char> = if starts_with(&remaining, rest) {
                let taken = rest.to_vec();
                remaining.drain(..rest.len());
                consumed[si] = sc.len();
                taken
            } else if starts_with(rest, &remaining) {
                let taken = remaining.clone();
                consumed[si] += remaining.len();
                remaining.clear();
                taken
            } else {
                continue;
            };
            let size = segment_size(seg, font_size);
            let seg_scale = seg.scale.unwrap_or(1.0);
            let cspace = seg.cspace.unwrap_or(0.0) / TEXT_SCALE;
            let measure = if seg.subscript || seg.superscript {
                size * 0.5
            } else {
                size
            };
            for (part_index, raw_ch) in part.iter().enumerate() {
                let raw_ch = *raw_ch;
                for (rendered_ch, char_scale) in transformed_glyphs(raw_ch, seg) {
                    let glyph = if force_fallback_glyph(rendered_ch) {
                        None
                    } else {
                        art.cells.get(&rendered_ch).map(|cell| cell.advance)
                    };
                    let step =
                        resolve_glyph_advance(glyph, rendered_ch, measure, BAKE_PPEM, FONT_FAMILY)
                            * char_scale
                            * seg_scale
                            + char_extra;
                    if let Some(_) = glyph {
                        lines[li].push((
                            GlyphSpot {
                                ch: rendered_ch,
                                size: measure,
                                visual: seg_scale * char_scale,
                                color: seg.color,
                                alpha: seg.alpha,
                            },
                            pen,
                            seg_raw_base[si] + seg_raw_of_clean[si][raw_base_in_seg + part_index],
                        ));
                    } else if !force_fallback_glyph(rendered_ch) {
                        // 律给回退推进、字形表没有：不出图，只占位并计数。
                        missing_used += 1;
                    }
                    pen += step;
                    pen += cspace;
                    // 空白字后补 word 一笔（真源：空白与零宽空格触发）。
                    if rendered_ch.is_whitespace() || rendered_ch == '\u{200b}' {
                        pen += word_extra;
                    }
                }
            }
            // 行尾补一次该段的 cspace（行量法同款怪癖）。
            pen += cspace;
        }
        // 无段认领的剩余字符按层字号走（行量法同款兜底）。原文下标按
        // 行基推（无标签前提：行文本切分即原文切分，`\n` 占位）。
        let line_raw_base: usize = line_texts
            .iter()
            .take(li)
            .map(|l| l.chars().count() + 1)
            .sum();
        for (fallback_pos, raw_ch) in remaining.into_iter().enumerate() {
            let glyph = if force_fallback_glyph(raw_ch) {
                None
            } else {
                art.cells.get(&raw_ch).map(|cell| cell.advance)
            };
            let step = resolve_glyph_advance(glyph, raw_ch, font_size, BAKE_PPEM, FONT_FAMILY)
                + char_extra;
            if glyph.is_some() {
                lines[li].push((
                    GlyphSpot {
                        ch: raw_ch,
                        size: font_size,
                        visual: 1.0,
                        color: None,
                        alpha: None,
                    },
                    pen,
                    line_raw_base + fallback_pos,
                ));
            } else if !force_fallback_glyph(raw_ch) {
                missing_used += 1;
            }
            pen += step;
            if raw_ch.is_whitespace() || raw_ch == '\u{200b}' {
                pen += word_extra;
            }
        }
    }
    (lines, missing_used)
}

/// 依律铺一只气泡：行量法出盒形，摆位 walk 出字形；结构 =
/// 根（定位 + 顶边缩放）→ animation 节点（DOScale 作用点，原点取 bg
/// 中心）→ 部件（九宫背景 + 字形 + 箭头）。
fn spawn_balloon(
    commands: &mut Commands,
    art: &BalloonArt,
    npc: Entity,
    unit: u32,
    row: &TweetRow,
) {
    spawn_balloon_text(commands, art, npc, unit, row.id, &row.text, None, false);
}

/// 铺一只气泡的核（对账 id 与日志由调用侧打）。
fn spawn_balloon_text(
    commands: &mut Commands,
    art: &BalloonArt,
    npc: Entity,
    unit: u32,
    id: i32,
    text: &str,
    probe_root: Option<Vec3>,
    probe: bool,
) -> Entity {
    // 对话路径没有 face 四键：tweet 对账日志读 face 的地方全部走
    // row.id/文本侧，face 缺省 None 不进对账。
    let row = TweetRow {
        id,
        text: text.to_string(),
        motion_name: None,
        emoticon_name: None,
        // 对话路径没有表情键：对账日志只读 id/文本侧，空串即「无值」
        // 的 tweet 同款语义（pick 侧从不消费对话路径的行）。
        eye_name: String::new(),
        mouth_name: String::new(),
    };
    let text = row.text.as_str();
    let metrics = layout_metrics(
        text,
        FONT_SIZE,
        FONT_FAMILY,
        LINE_SPACING,
        BAKE_PPEM,
        &|ch: char| art.cells.get(&ch).map(|cell| cell.advance),
    );
    let (lines, missing_used) = walk_glyphs(text, art, FONT_SIZE, 0.0, 0.0);
    let line_count = metrics.line_widths.len();

    // 行数对账：摆位的行结构必须与律一致（不一致响 warn 不 panic——
    // 画面照出，但实现漂移必须现形）。
    if lines.len() != line_count {
        warn!(
            "[tweet] tweet={} 摆位行数 {} 与律行数 {line_count} 不一致：摆位实现漂移",
            row.id,
            lines.len()
        );
    }

    // 盒几何：基线 y（相对盒中心，向上为正）= 锚基 − 行偏移；顶 =
    // 首行基线 + 首行字号×上行线，底 = 末行基线 − 末行字号×下行线
    // （66/75、9/75 是真源常数）。行字号取该行摆位字号的最大者，
    // 空行回退层字号（行量法同款）。
    // line_offsets 是 2x 单位（律侧累加不折），消费时除 TEXT_SCALE——
    // 与排版律参考实现的消费式一致（锚基原样、行偏移除 2）。
    let baseline_up = |i: usize| metrics.anchor_base - metrics.line_offsets[i] / TEXT_SCALE;
    let size_of_line = |i: usize| -> f32 {
        lines
            .get(i)
            .map(|line| {
                line.iter()
                    .map(|(spot, _, _)| spot.size)
                    .fold(0.0f32, f32::max)
            })
            .filter(|size| *size > 0.0)
            .unwrap_or(FONT_SIZE)
    };
    let box_top = baseline_up(0) + size_of_line(0) * ASCENT_RATIO;
    let box_bottom = baseline_up(line_count - 1) - size_of_line(line_count - 1) * DESCENT_RATIO;
    let text_h = (box_top - box_bottom).max(1.0);
    // 律的盒宽自带 32px 内边距常数（可见宽最大值 + 32），扣除得文本宽。
    let text_w = (metrics.box_w - 32.0).max(1.0);
    // 盒尺寸 = Layout preferred-size 的近似式：文本宽 + L40+R40、
    // 文本高 + 上 32 下 32（真源经 ContentSizeFitter/VLG 链驱动，近似注明）。
    let bg_w = text_w + BG_PAD_LEFT + BG_PAD_RIGHT;
    let bg_h = text_h + BG_PAD_TOP + BG_PAD_BOTTOM;
    // 对齐律（真值）：单行 Center、多行 Left；垂直 Middle 由上下对称
    // padding（32/32）成立。
    let single_line = line_count <= 1;

    let root = commands
        .spawn((
            BalloonAnchor {
                npc,
                tweet: row.id,
                hidden: false,
                // 盒尺寸（canvas 单位，bg 的 W×H）——探针用：place 按窗口
                // 算完缩放后报「最终显示像素」要拿它乘。
                box_size: Vec2::new(bg_w, bg_h),
                probe_root,
                probe,
            },
            Elapsed {
                t: 0.0,
                entered: false,
                exiting: false,
            },
            // 根不带 sprite，Transform 要自己带：定位系统按它写屏幕位
            // 与顶边缩放。
            Transform::default(),
            Visibility::default(),
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    // animation 节点：入场/收场 DOScale 的作用点（真源 _animationRoot）。
    // 初值 0（DOScale 从 0 起）；tick 每帧覆写。
    let anim = commands
        .spawn((
            BalloonAnim,
            Transform::from_xyz(0.0, bg_h / 2.0, 0.0).with_scale(Vec3::ZERO),
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    commands.entity(root).add_child(anim);

    // 背景：九宫（4 角 + 4 边 + 中心；角与边不缩放、中段拉伸）。
    for (rect, size, pos) in nine_slice_pieces(bg_w, bg_h) {
        let piece = commands
            .spawn((
                Sprite {
                    image: art.bg.clone(),
                    color: SHAPE_COLOR,
                    rect: Some(rect),
                    custom_size: Some(size),
                    ..default()
                },
                Transform::from_xyz(pos.x, pos.y, PART_Z_BG),
                BalloonPart {
                    base: SHAPE_COLOR,
                    kind: PartKind::Bg,
                },
                RenderLayers::layer(BALLOON_LAYER),
            ))
            .id();
        commands.entity(anim).add_child(piece);
    }

    // 字形：行量法的笔点摆位；单行居中/多行居左，垂直随 padding 居中。
    let mut first_glyph: Option<(char, Rect, f32, f32)> = None;
    for (li, line) in lines.iter().enumerate() {
        let origin = if single_line {
            // 居中按可见宽（rect_widths 不含行尾空白悬挂）。
            -metrics.rect_widths[li] / 2.0
        } else {
            // 居左：各行同一起笔边（文本块左沿）。
            -text_w / 2.0
        };
        // 基线离 bg 底的高度 = 基线中心系 y − 盒底 y + 下 padding，再折
        // 到 bg 中心系：文本盒 [下 padding, 下 padding+text_h] 居中于 bg。
        let baseline = baseline_up(li) - box_bottom + BG_PAD_BOTTOM - bg_h / 2.0;
        for (spot, pen, _) in line {
            let Some(cell) = art.cells.get(&spot.ch) else {
                continue;
            };
            let scale = spot.size * spot.visual / BAKE_PPEM;
            // 格内笔点在 (pen_x, baseline_from_top)；格中心到笔点的偏移
            // 按显示缩放折算，atlas 的 y 向下、盒坐标 y 向上取负。
            let dx = (art.cell / 2.0 - art.pen_x) * scale;
            let dy = (art.cell / 2.0 - art.baseline_from_top) * scale;
            let [r, g, b] = spot.color.unwrap_or(TEXT_COLOR);
            let alpha = spot.alpha.unwrap_or(1.0);
            let color = Color::srgba(r, g, b, alpha);
            let x = origin + pen + dx;
            let y = baseline - dy;
            if first_glyph.is_none() {
                first_glyph = Some((spot.ch, cell.rect, x, y));
            }
            let glyph = commands
                .spawn((
                    Sprite {
                        image: art.glyph_image_for(spot.ch).clone(),
                        color,
                        rect: Some(cell.rect),
                        custom_size: Some(Vec2::splat(art.cell * scale)),
                        ..default()
                    },
                    Transform::from_xyz(x, y, PART_Z_TEXT),
                    BalloonPart {
                        base: color,
                        kind: PartKind::Text,
                    },
                    RenderLayers::layer(BALLOON_LAYER),
                ))
                .id();
            commands.entity(anim).add_child(glyph);
        }
    }

    // 箭头：行高 0 越界画出——行顶 = bg 下沿 − 行距 2.5，箭头顶 =
    // 行顶 + 11.5 ⇒ 与 bg 重叠 9px（无缝衔接）。部件层序里箭头最顶
    // （真源预制体 `animation` 子序：箭头容器在 bg 子树之后，后者画
    // 在上——箭头盖在与 bg 重叠的 9px 上）。
    let arrow_top = -bg_h / 2.0 + (ARROW_POS_Y - ROW_SPACING);
    let arrow = commands
        .spawn((
            Sprite {
                image: art.arrow.clone(),
                color: SHAPE_COLOR,
                rect: Some(Rect {
                    min: Vec2::ZERO,
                    max: Vec2::new(ARROW_W, ARROW_H),
                }),
                custom_size: Some(Vec2::new(ARROW_W, ARROW_H)),
                ..default()
            },
            Transform::from_xyz(0.0, arrow_top - ARROW_H / 2.0, PART_Z_ARROW),
            BalloonPart {
                base: SHAPE_COLOR,
                kind: PartKind::Arrow,
            },
            RenderLayers::layer(BALLOON_LAYER),
        ))
        .id();
    commands.entity(anim).add_child(arrow);

    let face = row.face();
    let motion_word = face.motion.map(ascii_or).unwrap_or("-");
    info!(
        "[tweet] unit={unit} tweet={} 上屏 SPEC 值：字号 {FONT_SIZE} 行距 {LINE_SPACING} 对齐 {}（{} 行，垂直 Middle）文本 {text_w:.1}x{text_h:.1} 盒 {bg_w:.1}x{bg_h:.1}（文本+80/+64，Layout preferred 近似）bg btn_r30_wh border {BG_BORDER} α {SHAPE_ALPHA} 箭头 balloon_direction_triangle_wh {ARROW_W}x{ARROW_H} 重叠 {:.1} 时序 DOScale 0.2s/0.2s Linear 保持 5.0s",
        row.id,
        if single_line { "Center" } else { "Left" },
        lines.len(),
        ARROW_POS_Y - ROW_SPACING,
    );
    if line_count > 1 {
        // 行距实际值：首两行的基线节距（律换算后的落地值）。
        let pitch = metrics.line_offsets[1] / TEXT_SCALE;
        info!(
            "[tweet] tweet={} 行距节距（行距 -80 换算后）：{pitch:.2}",
            row.id
        );
    }
    info!(
        "[tweet] unit={unit} tweet={} 对账：行 {}（律 {line_count}）锚基 {:.2} 缺字形 {missing_used} 锚=avatar根+1.0 面={}/{} 动作 {}",
        row.id,
        lines.len(),
        metrics.anchor_base,
        ascii_or(face.eye),
        ascii_or(face.mouth),
        motion_word,
    );
    if let Some((ch, rect, x, y)) = first_glyph {
        info!(
            "[tweet] tweet={} 首字形 U+{:04X} rect=({:.0},{:.0},{:.0}x{:.0}) bg中心系局部=({:.1},{:.1})",
            row.id,
            ch as u32,
            rect.min.x,
            rect.min.y,
            rect.width(),
            rect.height(),
            x,
            y
        );
    }
    root
}

/// 九宫件清单：4 角 + 4 边 + 中心（`btn_r30_wh` 的 border 38 语义——
/// 角与边不缩放、中段拉伸；与真源 Image 组件的 Sliced 同式，中段近
/// 均匀白、拉伸无可见梯度）。返回
/// (源图矩形, 显示尺寸, 相对 bg 中心位置)。
fn nine_slice_pieces(bg_w: f32, bg_h: f32) -> Vec<(Rect, Vec2, Vec2)> {
    let b = BG_BORDER;
    let s = BG_SPRITE;
    let x0 = -bg_w / 2.0;
    let y0 = -bg_h / 2.0;
    let cw = (bg_w - 2.0 * b).max(1.0);
    let ch = (bg_h - 2.0 * b).max(1.0);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect {
        min: Vec2::new(x0, y0),
        max: Vec2::new(x1, y1),
    };
    vec![
        // 四角（源图坐标 y 从顶量；带上边缘的贴 bg 顶）。
        (
            rect(0.0, 0.0, b, b),
            Vec2::splat(b),
            Vec2::new(x0 + b / 2.0, y0 + bg_h - b / 2.0),
        ),
        (
            rect(s - b, 0.0, s, b),
            Vec2::splat(b),
            Vec2::new(x0 + bg_w - b / 2.0, y0 + bg_h - b / 2.0),
        ),
        (
            rect(0.0, s - b, b, s),
            Vec2::splat(b),
            Vec2::new(x0 + b / 2.0, y0 + b / 2.0),
        ),
        (
            rect(s - b, s - b, s, s),
            Vec2::splat(b),
            Vec2::new(x0 + bg_w - b / 2.0, y0 + b / 2.0),
        ),
        // 四边（中段）。
        (
            rect(b, 0.0, s - b, b),
            Vec2::new(cw, b),
            Vec2::new(0.0, y0 + bg_h - b / 2.0),
        ),
        (
            rect(b, s - b, s - b, s),
            Vec2::new(cw, b),
            Vec2::new(0.0, y0 + b / 2.0),
        ),
        (
            rect(0.0, b, b, s - b),
            Vec2::new(b, ch),
            Vec2::new(x0 + b / 2.0, 0.0),
        ),
        (
            rect(s - b, b, s, s - b),
            Vec2::new(b, ch),
            Vec2::new(x0 + bg_w - b / 2.0, 0.0),
        ),
        // 中心。
        (rect(b, b, s - b, s - b), Vec2::new(cw, ch), Vec2::ZERO),
    ]
}

/// 日志守卫：非 ASCII 内容换成占位符（语言网关令——名字应全 ASCII，
/// 万一不是也不进日志）。
pub(crate) fn ascii_or(value: &str) -> &str {
    if value.is_ascii() {
        value
    } else {
        "<non-ascii>"
    }
}

// ---------------------------------------------------------------------------
// 时序与定位
// ---------------------------------------------------------------------------

/// Update：时序 = 入场 DOScale 0→1（0.2s Linear）→ 保持到 5.0s（NPC
/// 状态机时长的对应值；真源 HUD 无自定时器、生死事件驱动，近似）→
/// 收场 DOScale 1→0（0.2s Linear）→ 缩出完成销毁。5.0 的语义按律是
/// 严格大于（恰 5.0 的那一拍仍满尺寸，之后才收）。缩放写 animation
/// 节点（非根）。
pub(crate) fn tick(
    mut commands: Commands,
    library: Res<crate::content_library::ContentLibrary>,
    time: Res<Time>,
    mut balloons: Query<(
        Entity,
        &BalloonAnchor,
        &mut Elapsed,
        &Children,
        Option<&ActivityBalloon>,
        Option<&TalkPreviewBalloon>,
    )>,
    mut anims: Query<&mut Transform, With<BalloonAnim>>,
) {
    let dt = time.delta_secs();
    for (entity, anchor, mut elapsed, kids, activity, preview) in &mut balloons {
        if let Some(preview) = preview {
            if !library.owns_talk_preview(preview.ticket) {
                commands.entity(entity).despawn();
                continue;
            }
            // The source HUD does not truncate text or own a duration timer.
            // A callable preview is held by its admitted interaction, until
            // engagement/cancel releases it; only the source enter tween runs.
            elapsed.t = (elapsed.t + dt).min(SCALE_SECONDS);
            for kid in kids.iter() {
                if let Ok(mut transform) = anims.get_mut(kid) {
                    transform.scale = Vec3::splat((elapsed.t / SCALE_SECONDS).clamp(0.,1.));
                }
            }
            continue;
        }
        if library.owns_scene() && activity.is_none() {
            commands.entity(entity).despawn();
            continue;
        }
        elapsed.t += dt;
        let end = TWEET_DISPLAY_SECONDS + SCALE_SECONDS;
        if elapsed.t > end {
            info!(
                "[tweet] tweet={} 销毁：收场 DOScale 1→0（0.2s Linear）完成，总时长 {:.2}s（5.0s 保持 + 0.2s 缩出）",
                anchor.tweet, elapsed.t
            );
            commands.entity(entity).despawn();
            continue;
        }
        if !elapsed.entered && elapsed.t >= SCALE_SECONDS {
            elapsed.entered = true;
            info!(
                "[tweet] tweet={} 入场完成：DOScale 0→1 实际 {:.2}s（0.2s Linear）",
                anchor.tweet, elapsed.t
            );
        }
        if !elapsed.exiting && elapsed.t > TWEET_DISPLAY_SECONDS {
            elapsed.exiting = true;
            info!(
                "[tweet] tweet={} 收场开始：elapsed {:.2} > 5.0（严格大于；5.0s=NPC 状态机时长对应值）",
                anchor.tweet, elapsed.t
            );
        }
        let scale = if elapsed.t < SCALE_SECONDS {
            elapsed.t / SCALE_SECONDS
        } else if elapsed.t <= TWEET_DISPLAY_SECONDS {
            1.0
        } else {
            ((end - elapsed.t) / SCALE_SECONDS).clamp(0.0, 1.0)
        };
        for kid in kids.iter() {
            if let Ok(mut transform) = anims.get_mut(kid) {
                transform.scale = Vec3::splat(scale);
            }
        }
    }
}

/// PostUpdate：avatar 根位 + 世界偏移 → 主相机屏幕像素 → 气泡层世界
/// 坐标（1 单位 = 1 逻辑像素，原点在窗心）。根位每帧现读，气泡跟随。
/// 每帧三件事（真源 Update 的两条 + canvas 缩放）：
/// * **相机背面剔除**：目标在相机背面（视线方向点积 ≤ 0）时 alpha=0
///   （不销毁，位置沿用上一帧）；回到前方恢复。
/// * **顶边缩放**：屏幕 y>H/2 时整体缩（顶边收 0.5×），y≤H/2 恒 1——
///   写在根上（与 DOScale 的 animation 节点分层，二者相乘）。
/// * **canvas 缩放**：见 [`canvas_scale`]。
/// 无屏幕边缘钳制（出界照画；投影失败帧沿用上一帧位置）。
///
/// world_to_viewport 返回的已经是逻辑像素（bevy 走 logical_viewport_rect），
/// 不能再除 scale_factor——那是对逻辑像素的二次换算，高 DPI 机器上会把
/// 气泡整体向窗心压缩（sf=1.5 实测每个气泡向左上偏约 1/1.5）。
#[allow(clippy::type_complexity)]
pub(crate) fn place(
    mut commands: Commands,
    mut balloons: Query<(
        Entity,
        &mut BalloonAnchor,
        &mut Transform,
        &Elapsed,
        Option<&Placed>,
        Option<&LayerLogged>,
    )>,
    transforms: Query<&GlobalTransform>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    windows: Query<&Window>,
    children_q: Query<&Children>,
    mut parts: Query<(&BalloonPart, &mut Sprite)>,
) {
    if balloons.is_empty() {
        return;
    }
    let Ok((camera, cam_transform, projection)) = cameras.single() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    // 探针的相机态：fov/near 从组件现读——手算投影要用的两把尺。
    let (fov_deg, near) = match projection {
        Projection::Perspective(p) => (p.fov.to_degrees(), p.near),
        Projection::Orthographic(o) => (0.0, o.near),
        Projection::Custom(_) => panic!("气泡投影探针不认识自定义投影：无从手算"),
    };
    let eye = cam_transform.translation();
    let fwd = cam_transform.forward();
    let sf = window.scale_factor();
    let (width, height) = (window.width(), window.height());
    let canvas = canvas_scale(width, height);
    // 跨气泡层序（真源 `SetSiblingTweetHUD` 每帧重排的对应）：键 = 相机
    // transform 到各气泡目标的欧氏距离（avatar 视图根；投影偏移不参与
    // ——真源排序键与投影锚是两个量），按距离降序 → 远的名次 0；
    // uGUI 兄弟序后者在上 ⇒ 近的最后画、盖住远的。bevy 2D 透明相位按
    // 全局 z 升序画（同向）⇒ 名次折进根部 z：近者高（天花板 − 档位 ×
    // 步进）。真源只在 ≥2 只时重排；单只时名次 0 = 最低档，与不排无差。
    // 反向臂旋钮把根部 z 全喂 0（= 接线前的恒 0），判据 ① 的负向臂。
    let no_layer = std::env::var("MOLY_BALLOON_ORDER_NO_LAYER").is_ok();
    let order_log = matches!(env_secs_opt("MOLY_BALLOON_ORDER_SECS"), Some(v) if v > 0.0);
    let mut ranked: Vec<(Entity, f32)> = Vec::new();
    for (entity, anchor, _transform, _elapsed, _placed, _layer) in balloons.iter() {
        let Some(root) = balloon_root(anchor, &transforms) else {
            continue;
        };
        ranked.push((entity, (root - eye).length()));
    }
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1)); // 降序：远者在前（名次小）
    let base_of = |entity: Entity| -> f32 {
        if no_layer {
            return 0.0;
        }
        match ranked.iter().position(|(e, _)| *e == entity) {
            Some(rank) => {
                BALLOON_LAYER_CEILING - (ranked.len() - rank) as f32 * BALLOON_LAYER_STRIDE
            }
            // 根位未就绪的气泡不进名次（当帧也不定位）；真到不了这里。
            None => BALLOON_LAYER_CEILING - ranked.len() as f32 * BALLOON_LAYER_STRIDE,
        }
    };
    for (entity, mut anchor, mut transform, elapsed, placed, layer) in &mut balloons {
        // 锚点（真值式）：avatar 根位 + 世界偏移 (0,1,0)——偏移在投影前加，
        // 近大远小；骨锚定的旧式已按真源换掉（真源取的是 avatar 根位）。
        let Some(root) = balloon_root(&anchor, &transforms) else {
            continue; // 变换还没传播（根位未就绪 = 恒等；探针根位直供）
        };
        let world = root + ANCHOR_OFFSET;
        // 相机背面（IsNeedShow 门之一）：alpha=0 不销毁。真源只此一门
        // （视线点积）；近平面**不剔除**——Unity 的 WorldToScreenPoint 对
        // 近平面前方的点照常出坐标（贴脸的目标会把气泡投影到画面外/
        // 极大，无屏幕钳制）。据此不用 `world_to_viewport`：它对
        // NDC z>1（近平面之前）返 `PastNearPlane` 错，会把真源会画的
        // 气泡静默丢掉（实测：跟随相机经过身侧的成员，头离相机 0.34m
        // < near 1.0，15/45 只气泡从未上屏）。改自折 NDC→视口像素。
        let view_z = (world - eye).dot(*fwd);
        let behind = view_z <= 0.0;
        if behind != anchor.hidden {
            anchor.hidden = behind;
            // Every part starts with its base color. Only a visibility edge
            // changes alpha; avoid dirtying every glyph Sprite each frame.
            set_alpha(
                &mut parts,
                &children_q,
                entity,
                if behind { 0.0 } else { 1.0 },
            );
            info!(
                "[tweet] tweet={} {}",
                anchor.tweet,
                if behind {
                    "相机背面：alpha=0（不销毁）"
                } else {
                    "回到相机前方：恢复显示"
                }
            );
        }
        if behind {
            continue;
        }
        let Some(ndc) = camera.world_to_ndc(cam_transform, world) else {
            continue; // 投影退化（NaN）：沿用上一帧位置
        };
        // NDC → 视口逻辑像素：y 翻转（NDC 向上、屏幕向下），原点在左下。
        let target = camera
            .logical_viewport_rect()
            .map(|r| r.size())
            .unwrap_or(Vec2::new(width, height));
        let px = Vec2::new(
            (ndc.x + 1.0) / 2.0 * target.x,
            (1.0 - ndc.y) / 2.0 * target.y,
        );
        let depth = view_z;
        // 根位 = 锚点屏幕位（盒中心过锚：世界偏移 (0,1,0) 已在投影前加，
        // 近大远小，把气泡中心钉在锚上方）。z = 跨气泡层序的名次档
        // （近者高、远者低；见函数头的真源出处）。
        let base = base_of(entity);
        transform.translation = Vec3::new(px.x - width / 2.0, height / 2.0 - px.y, base);
        // 顶边缩放（真值式）：y>H/2 时 scale = clamp((H−y)/(H/2))×0.5+0.5。
        let top_scale = if px.y > height / 2.0 {
            ((height - px.y) / (height / 2.0)).clamp(0.0, 1.0) * 0.5 + 0.5
        } else {
            1.0
        };
        // 根上的总缩放 = 顶边缩放 × canvas 缩放（真源里前者是 HUD 自己的
        // UpdateScale、后者是根 canvas 的 ScaleWithScreenSize，两层相乘；
        // 锚点屏幕位不参与——真源的 localPosition 已除过 canvas 缩放，
        // 乘回去净值为屏幕像素）。
        // Same authored canvas/HUD scale in browser and native rendering.
        transform.scale = Vec3::splat(top_scale * canvas);
        if placed.is_none() {
            commands.entity(entity).insert(Placed);
            // 矩阵 near：clip_from_view[3][2]。与组件 near 并排打——两者会
            // 不一致：struct 更新式 `..default()` 把 near_clip_plane 留在默认
            // 近距 0.1 上，斜裁剪调整因此生效，把矩阵 z 行改写成 (0,0,0,0.1)。
            // 有效近裁剪面是矩阵这份（0.1），组件声明的那份不进矩阵。
            let matrix_near = camera.clip_from_view().w_axis.z;
            info!(
                "[tweet] tweet={} 锚 world=({:.2},{:.2},{:.2}) 屏幕=({:.1},{:.1}) depth={:.2} 相机 eye=({:.2},{:.2},{:.2}) fwd=({:.4},{:.4},{:.4}) fov={:.1} near={:.2} 矩阵near={:.2} 窗口逻辑={:.0}x{:.0} 物理={:.0}x{:.0} sf={:.2} 气泡中心=({:.1},{:.1})",
                anchor.tweet,
                world.x,
                world.y,
                world.z,
                px.x,
                px.y,
                depth,
                eye.x,
                eye.y,
                eye.z,
                fwd.x,
                fwd.y,
                fwd.z,
                fov_deg,
                near,
                matrix_near,
                width,
                height,
                window.physical_width(),
                window.physical_height(),
                sf,
                transform.translation.x,
                transform.translation.y
            );
            // 尺寸探针：盒几何/字号按 canvas 单位算完，此处乘 canvas 缩放
            // 报「最终显示像素」（逻辑 px；物理 = ×sf）。
            info!(
                "[tweet] tweet={} 显示尺寸：盒 {:.1}x{:.1}px 字号 {:.1}px（canvas 单位 {:.1}x{:.1} · 字号 32，× 缩放 {:.3}；逻辑窗 {:.0}x{:.0}）",
                anchor.tweet,
                anchor.box_size.x * canvas,
                anchor.box_size.y * canvas,
                FONT_SIZE * canvas,
                anchor.box_size.x,
                anchor.box_size.y,
                canvas,
                width,
                height
            );
        }
        // 层序探针（MOLY_BALLOON_ORDER_SECS 才开）：稳态闩一次，逐部件报
        // 最终排序键。键与渲染器同式：根 z + 根缩放 × animation 节点缩放
        // × 部件层 z（animation 节点无 z 平移）——入场 DOScale=1 的保持段
        // 恰是传播值；t=0 缩放为 0 会把部件层差缩没，无从比，故闩在入场
        // 完成后。行全 ASCII（判据脚本按它解析；文本内容照例不进日志）。
        if order_log && elapsed.entered && !elapsed.exiting && layer.is_none() {
            commands.entity(entity).insert(LayerLogged);
            let k = top_scale * canvas;
            for anim in children_q.get(entity).iter().flat_map(|kids| kids.iter()) {
                for kid in children_q.get(anim).iter().flat_map(|kids| kids.iter()) {
                    if let Ok((part, _sprite)) = parts.get(kid) {
                        info!(
                            "[tweet-order] probe={} tweet={} part={} base={:.3} k={:.3} z={:.3}",
                            if anchor.probe { 1 } else { 0 },
                            anchor.tweet,
                            part.kind.tag(),
                            base,
                            k,
                            base + k * part.kind.layer_z()
                        );
                    }
                }
            }
        }
    }
}

/// 写全部部件的 alpha：`factor` 0 = 相机背面门（不销毁），1 = 基础色。
/// 部件在 animation 节点之下（根 → animation → 部件）。
fn set_alpha(
    parts: &mut Query<(&BalloonPart, &mut Sprite)>,
    children_q: &Query<&Children>,
    root: Entity,
    factor: f32,
) {
    for anim in children_q.get(root).iter().flat_map(|kids| kids.iter()) {
        for kid in children_q.get(anim).iter().flat_map(|kids| kids.iter()) {
            if let Ok((part, mut sprite)) = parts.get_mut(kid) {
                sprite.color = part.base.with_alpha(part.base.alpha() * factor);
            }
        }
    }
}

/// 气泡的排序根位（= avatar 视图根；探针气泡直供固定世界位）。None =
/// 变换还没传播或目标实体已不在——与旧定位门同一语义面（根位恒等即
/// 未就绪，含根位恰在原点的边角：那种成员当帧不定位、不进名次）。
fn balloon_root(anchor: &BalloonAnchor, transforms: &Query<&GlobalTransform>) -> Option<Vec3> {
    let root = match anchor.probe_root {
        Some(p) => p,
        None => transforms.get(anchor.npc).ok()?.translation(),
    };
    (root != Vec3::ZERO).then_some(root)
}

/// 环境变量秒数（未设 = None；非法值响亮 panic——验收旋钮，静默回默认
/// 会让「没跑成」伪装成「没开」）。0 或负 = 未启用，由调用侧判。
fn env_secs_opt(name: &str) -> Option<f32> {
    let raw = std::env::var(name).ok()?;
    let v: f64 = raw
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{name} 不是秒数：{raw:?}"));
    Some(v.max(0.0) as f32)
}

/// 层序探针的挂靠实体（成员字段的占位：让位门按真实成员比对，探针
/// 气泡不与任何成员相撞 ⇒ 探针在场期间不被让位）。
#[derive(Component)]
pub(crate) struct OrderProbe;

/// Update：层序探针（`MOLY_BALLOON_ORDER_SECS` 秒 > 0 启用）：就绪后造
/// 一对屏幕上完全重叠的探针气泡——近根取相机正前方 [`ORDER_PROBE_NEAR`]
/// 米（视线轴上），远锚取**过近锚点的同一视线射线**两倍距离处（两只
/// 锚点投影到同一屏幕点 = 用户目视报障的完全重叠场景），真实 tweet 行
/// 上屏（主表前两行——选取律不进这条链；字形必有，文本不进日志）。
/// 到秒数 `AppExit::Success` 退出（无头验收）；窗口耗尽仍未就绪（主表/
/// 图集/相机未齐）同样退出，判据侧从日志缺行现形（fail-closed：探针
/// 没造出来就一个键都没有）。
pub(crate) fn order_smoke(
    mut commands: Commands,
    master: Option<Res<TweetMaster>>,
    art: Option<Res<BalloonArt>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
    mut spawned: Local<bool>,
) {
    let Some(secs) = env_secs_opt("MOLY_BALLOON_ORDER_SECS") else {
        return;
    };
    if secs <= 0.0 {
        return;
    }
    if time.elapsed_secs() >= secs {
        // 已造过 → 窗口到点收工；没造过 → 就绪面缺席，同样收工（缺行
        // 就是判据侧的响亮失败）。
        info!(
            "[tweet-order] probe window {:.1}s reached, exiting (spawned={})",
            secs, *spawned as u8
        );
        exit.write(AppExit::Success);
        return;
    }
    if *spawned {
        return;
    }
    let (Some(master), Some(art)) = (master.as_deref(), art.as_deref()) else {
        return;
    };
    let Ok((_camera, cam_transform)) = cameras.single() else {
        return;
    };
    let (near_row, far_row) = match (master.tweets.first(), master.tweets.get(1)) {
        (Some(near), Some(far)) => (near, far),
        _ => panic!("layer-order probe needs 2+ master rows"),
    };
    *spawned = true;
    if let Ok(run_id) = std::env::var("MOLY_BALLOON_ORDER_RUN_ID") {
        info!("[tweet-order] run={} version={}", run_id, crate::VERSION);
    }
    let eye = cam_transform.translation();
    let fwd = cam_transform.forward();
    // 近锚 = 眼 + 视线 × 近距 + 投影偏移（与真实气泡同一偏移）；远锚 =
    // 过近锚点的视线射线两倍距离（同屏点）。根 = 锚 − 偏移（排序键与
    // 投影锚是两个量，见 place 的出处注释）。
    let near_root = eye + fwd * ORDER_PROBE_NEAR;
    let near_anchor = near_root + ANCHOR_OFFSET;
    let far_anchor = eye + (near_anchor - eye) * 2.0;
    let far_root = far_anchor - ANCHOR_OFFSET;
    let near_d = (near_root - eye).length();
    let far_d = (far_root - eye).length();
    let probe = commands.spawn(OrderProbe).id();
    spawn_balloon_text(
        &mut commands,
        art,
        probe,
        0,
        near_row.id,
        &near_row.text,
        Some(near_root),
        true,
    );
    spawn_balloon_text(
        &mut commands,
        art,
        probe,
        0,
        far_row.id,
        &far_row.text,
        Some(far_root),
        true,
    );
    info!(
        "[tweet-order] probe ready near_tweet={} near_d={:.3} far_tweet={} far_d={:.3} near_root=({:.2},{:.2},{:.2}) far_root=({:.2},{:.2},{:.2}) eye=({:.2},{:.2},{:.2})",
        near_row.id,
        near_d,
        far_row.id,
        far_d,
        near_root.x,
        near_root.y,
        near_root.z,
        far_root.x,
        far_root.y,
        far_root.z,
        eye.x,
        eye.y,
        eye.z
    );
}

/// Startup：气泡覆盖相机。只画 [`BALLOON_LAYER`]，不清屏——叠在 3D
/// 画面上；渲染次序在主相机（order 0）之后。
pub(crate) fn overlay_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        RenderLayers::layer(BALLOON_LAYER),
    ));
}

/// A selected furniture pre-action owns its own bubble. Background greetings
/// stay suppressed in an independent scene; this source-authored bubble does
/// not start a second dialogue or overwrite the Timeline's pose/face channels.
#[derive(Component, Clone, Copy)]
pub(crate) struct ActivityBalloon(pub(crate) crate::fixture_activity_state::FixtureActivityOwner);

pub(crate) fn show_activity_balloon(
    world: &mut World,
    owner: crate::fixture_activity_state::FixtureActivityOwner,
    unit: u32,
    tweet: &moly_law::talk::TweetRef,
) {
    if tweet.text.trim().is_empty() || world.get_entity(owner.actor).is_err() {
        return;
    }
    cancel_activity_balloon(world, owner);
    let mut state =
        bevy::ecs::system::SystemState::<(Commands, Option<Res<BalloonArt>>)>::new(world);
    let (mut commands, art) = state.get_mut(world);
    if let Some(art) = art {
        let root = spawn_balloon_text(
            &mut commands,
            &art,
            owner.actor,
            unit,
            tweet.id,
            &tweet.text,
            None,
            false,
        );
        commands.entity(root).insert(ActivityBalloon(owner));
    }
    state.apply(world);
}

pub(crate) fn cancel_activity_balloon(
    world: &mut World,
    owner: crate::fixture_activity_state::FixtureActivityOwner,
) {
    let roots: Vec<_> = world
        .query::<(Entity, &ActivityBalloon)>()
        .iter(world)
        .filter(|(_, bubble)| bubble.0 == owner)
        .map(|(entity, _)| entity)
        .collect();
    for entity in roots {
        if let Ok(root) = world.get_entity_mut(entity) {
            root.despawn();
        }
    }
}

#[cfg(test)]
mod embedded_font_coverage_tests {
    use super::FONT_BYTES;
    #[test]
    fn embedded_subset_contains_japanese_voicing_and_common_kanji() {
        let font=swash::FontRef::from_index(FONT_BYTES,0).expect("valid embedded OFL font");
        // Previously absent from the CN-only subset, creating silent holes in
        // Japanese dialogue despite Playing/scene-ready being true.
        for codepoint in [0x304C_u32,0x3054,0x3069,0x3060,0x3053,0x9055,0x6575] {
            assert_ne!(font.charmap().map(codepoint),0,"missing U+{codepoint:04X}");
        }
    }
}

/// Explicit source first-line preview, separate from a full dialogue session.
#[derive(Component)]
pub(crate) struct TalkPreviewBalloon { ticket: u64 }

pub(crate) fn show_talk_preview(world: &mut World, actor: Entity, unit: u32,
    tweet: &moly_law::talk::TweetRef, ticket: u64) {
    if tweet.id <= 0 || tweet.text.trim().is_empty() || world.get_entity(actor).is_err() { return; }
    let mut params = bevy::ecs::system::SystemState::<(Commands, Option<Res<BalloonArt>>)>::new(world);
    let (mut commands, art) = params.get_mut(world);
    if let Some(art) = art {
        let root = spawn_balloon_text(&mut commands, &art, actor, unit, tweet.id, &tweet.text, None, false);
        commands.entity(root).insert(TalkPreviewBalloon { ticket });
    }
    params.apply(world);
}
