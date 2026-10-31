//! Head tweet balloons: tweet master table → selection → layout → sprite
//! presentation.
//!
//! Presentation parameters (values from the HUD prefab):
//! * **Text**: font size 32 (`m_fontSize`, min/max both 32, autoSizing off),
//!   line spacing -80 (`lineSpacing`, converted by the line-spacing
//!   parameter of the [`moly_law::text`] line law), colour #555577FF,
//!   alignment = centered on one line, left-aligned on several (vertically
//!   always Middle, met by the bg's symmetric top and bottom padding). Hard
//!   line breaks are the only form: the text is not soft-wrapped, 73.5% of
//!   tweet texts already carry `\n`, and the box width follows the content's
//!   preferred size, so no wrap condition arises.
//! * **Box geometry**: a Layout chain (ContentSizeFitter Preferred + bg VLG
//!   padding left 40, right 40, top 32, bottom 32 + row spacing 2.5) is
//!   approximated here by text width + 80 horizontally / text height + 64
//!   vertically (the sum of the paddings); **an approximation**.
//! * **Timing**: entry DOScale 0→1 and exit →0 each take 0.2s Linear and act
//!   on the animation intermediate node (`_animationRoot`, not the root). An
//!   NPC head tweet's display time does not belong to the balloon: the HUD
//!   has no timer of its own, and the NPC state machine's HUD events
//!   (type 25, [`crate::npc_state::TweetHudEvent`]) drive it — entering a
//!   tweet / greeting state shows it, leaving the state closes it (a
//!   greeting closes on the first frame past 3.0s, while the state itself
//!   lives to 5.0s). The exit shrinks from the current scale (the tween
//!   starts at the value current at its start time). The layout after-edit
//!   reactions and the furniture pre-actions still hold for 5.0s (the tweet
//!   state's duration). No click-to-skip or close semantics (the tweet path
//!   has no typewriter; a click only attaches a callback and a sound).
//! * **canvas scaling**: the box geometry and font size are in canvas
//!   units; the root UI canvas scales them on screen with
//!   ScaleWithScreenSize (reference 1920×1080, MatchWidthOrHeight); the
//!   match follows the screen aspect ratio (H/W ≥ 1080/1920 → match height
//!   H/1080, wider → match width W/1920). The sizes scale, the anchor's
//!   screen position does not (it multiplies into the root and multiplies
//!   with the top-edge scale).
//! * **Top-edge scale and back-face culling**: when screen y>H/2 the
//!   balloon shrinks as a whole (the top edge contracts 0.5×); while the
//!   target is behind the camera alpha=0 (not despawned).
//! * **Two-level draw order** (how balloons cover each other): across
//!   balloons the HUD layer reorders its children every frame —
//!   `ScreenLayerMysekaiHUD.Update` calls `SetSiblingTweetHUD` each frame;
//!   the key is the Euclidean distance from the field camera transform to
//!   each HUD target (the avatar view root; the projection offset (0,1,0)
//!   takes no part, the sort key and the projection anchor are two different
//!   values), and the targets are given `SetSiblingIndex(0..)` in
//!   **descending** distance order, uGUI draws later siblings on top, so the
//!   **nearer balloon draws on top and covers the farther one**. Here this
//!   maps to part z: a stride per rank (the nearer one's whole set sits
//!   higher, and any two balloons' part z ranges never intersect — one
//!   balloon forms one layer and does not interleave with another). Within a
//!   balloon the order is the `animation` node's child order (the bg subtree
//!   first — the body text is a **child** of bg — the arrow container last),
//!   so **backdrop < body < arrow** (the arrow on top, covering the 9px it
//!   shares with the backdrop's bottom edge). The screen-UI bands (dialogue
//!   window, joystick) are layered by z on this crate's overlay camera, and
//!   the balloon band always stays below them (the root z has a ceiling
//!   constant, see [`BALLOON_LAYER_CEILING`]).
//!
//! The font stays an **open-source, locally baked atlas** (an OFL font in the
//! repository; the product uses a commercial SDF font whose assets never
//! enter this repository) — only the font size/line spacing/colour/alignment
//! and the weight-band semantics are reproduced (the tweet body uses the
//! regular band only; bold belongs to the announcement/cry variants, not to
//! this path). The atlas is rasterised in **two passes**: first the
//! character set's actual ink bounds are measured (top, bottom, left, right)
//! and the cell edge is fixed; the font size and the cell geometry do not
//! shrink with capacity. Each page's edge stays within the current
//! RenderDevice's 2D texture limit, and the full character set is packed
//! into pages in code-point order; a glyph cell records its page, and layout
//! reads only the advance and the in-page rectangle. Same data and same
//! device limits ⇒ same atlas.
//!
//! The background and the arrow use the UI atlas crops: the background
//! `btn_r30_wh` (a CommonAtlas crop of 80×80, border 38 on all four edges, rendered as a Sliced nine-slice by
//! its Image component) is placed as a nine-slice; the arrow
//! `balloon_direction_triangle_wh` (a crop of the same atlas, 40×20, border
//! 0, Simple whole image). **The family is chosen by reference, not by
//! name**: in the HUD prefab the background and arrow pieces' serialized
//! sprite references (names taken from the ledger aligned by pack identity)
//! point exactly at these two, and the HUD state machine never changes a
//! sprite name (no by-name sprite setter runs on this path, so the
//! serialized reference is what is drawn). The other family whose names
//! look most like balloons (the thinking/cry/announcement fixed-size set
//! T/S × S/M/L) is referenced only by the shared area map's dialogue
//! balloon component and is not on this path — its band selection is
//! recorded below. Both pieces' colour = white × α0.9019608 (`m_Color`, a
//! sprite tint; the texture only supplies pixels).
//!
//! **Name column: the head balloon has no such part.** Three checks: (a) the
//! whole HUD prefab tree (root, animation node, bg, bg overlay, body text,
//! button, button overlay, arrow container, arrow, arrow overlay, 10 nodes
//! in all) holds one text object (the body), and the HUD class's field table
//! has no name text entry either; (b) the `label` step belongs to the
//! dialogue window chain only — the window engine's name column method is a
//! **bare text SetText with no backdrop image**, and the window is only
//! initialised by the player click chain, the archive replay screen, the
//! multiplayer room and the tutorial handlers (the autonomous dialogue chain
//! cannot open it); (c) the name-shaped candidate textures
//! (`bg_base_mysekainame_h48` / `balloon_mysekaitag_dbl` /
//! `label_framebottom_r14_wh`) have zero references both at code call sites
//! and among string literals (the literal search space was checked with a
//! positive control), so none is the label step's backdrop. ⇒ The balloon
//! domain's label step stays named and is not drawn (a named skip log in the
//! dialogue module); **a name-shaped crop on disk is not enough to place
//! it**.
//!
//! **Band selection of the shared area map balloon (recorded reference; the
//! area map has its own dialogue balloon component, this crate has no
//! consumer yet, and this note is kept so the derivation is not redone)**:
//! * Routing (expression type enum None/Announce/AnnounceBig/Thinking/Cry/
//!   CryBig = 0..5): None → the generic backdrop; Announce and AnnounceBig →
//!   the announcement image; Thinking → the thinking family; Cry and CryBig
//!   → the cry family; any other value logs an error. Playback sets the base
//!   font size 32 and the default font band first, then overrides them by
//!   type.
//! * Thinking/cry family name =
//!   `"balloon_" + thinking|cry + "_" + <vertical> + "_" + <horizontal>`: the vertical band is T for exactly 2 **lines**
//!   and S otherwise (enum T=0/S=1); the horizontal band follows the
//!   **character count of the sizing line** (not the pixel width): <8 → S,
//!   8–10 → M, ≥11 → L (enum L=0/M=1/S=2). The sizing line is the
//!   **first line** by default; when the text is exactly 2 characters and
//!   contains a break, it is the longer of the first two lines.
//! * The announcement image `balloon_announce` (the only piece of that
//!   family with a border: 40/42/41/40 on the four edges, nine-slice); the
//!   generic backdrop `bg_base_r30_wh`. The Big types of announcement and cry
//!   raise the font size to 40 and switch to the EB font band, others stay
//!   at 32.
//! * The image sizes do not use Layout: the component hard-codes three
//!   `(type, quantity) → value` lookup tables at construction — width
//!   (thinking/cry by the sizing-line character count 1..12 and 1..10),
//!   height (by line count 1/2), backdrop Y offset (by line count 1/2); a
//!   failed lookup logs an error.
//!
//! Data / triggers / placement: the tweet master table and the three
//! greeting tables belong to the NPC side
//! ([`crate::npc_tweet::TweetTables`], two files together); the greeting
//! gate, selection and timing live in the NPC state machine (the greeting
//! gate in `npc_presenter`, the greeting state in `npc_state`), and the
//! balloon only consumes the HUD events it sends: Show shows, Hide closes,
//! Retire (a host site reset) takes down at once. Each member has only one
//! HUD balloon: a new Show replaces the one on screen (the entry starts from
//! 0; the shape of a HUD re-entry is not modelled).
//! The layout-edit chain follows the product's order — save receipt
//! (`crate::field_modes::LayoutSaved`) → reactor draw (the members on the
//! edited site sorted by distance to the player, the nearest
//! `Random.Range(1, ceil(n/2)+1)` of them taken) → release one by one (a
//! member is released only when the cancel check reports true, and after a
//! release the next waits `Random.Range(500, 1000)` milliseconds) → pool
//! pick ([`moly_law::tweet::pick_after_edit_tweet`]) → shown on the same
//! presentation chain → held for 5.0s ([`AFTER_EDIT_REACTION_DELAY_SECONDS`])
//! then closed. The hold comes **after** showing (the objective's order:
//! pick → look at the player → show → delay → close). The reaction objective
//! itself still runs outside the NPC objective loop (it is objective 2: it
//! cancels the current objective and sets the after-edit talk data), a named
//! gap, see [`after_edit_reaction`].
//! Placement = avatar root position + world offset (0,1,0) → projected by
//! the main camera to screen pixels → drawn by the 2D camera that renders
//! only the balloon layer (world unit = logical pixel), following every
//! frame. The log carries only ids, line counts, the measure and the actual
//! parameter values — text content never enters it (texts contain non-CJK
//! characters too).

use crate::character::CharacterShell;
use crate::npc::CharacterUnitId;
use bevy::asset::{LoadState, RenderAssetUsages};
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::image::Image;
use bevy::math::Rect;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::renderer::RenderDevice;
use moly_assets::residency::load_image;
use moly_law::text::advance::{force_fallback_glyph, resolve_glyph_advance};
use moly_law::text::layout_metrics;
use moly_law::text::tags::{parse_rich_segments, transformed_glyphs, SizeSpec, TextSegment};
use moly_law::tweet::{
    pick_after_edit_tweet, AfterEditRow, TweetRow, UniformDraw, AFTER_EDIT_REACTION_DELAY_SECONDS,
    TWEET_DISPLAY_SECONDS,
};
use serde_json::Value;
use std::collections::HashMap;

pub use crate::ui_inputs::atlas::{BALLOON_LAYER, BAKE_PPEM, SCREEN_UI_LAYER};

/// Layout font size (layout pixels). The text object's `m_fontSize` = 32
/// (min/max both 32, autoSizing off). Equal to the atlas bake size (drawn
/// 1:1).
const FONT_SIZE: f32 = 32.0;

/// Layout line spacing. The text object's `lineSpacing` = -80 (the
/// serialized value; converted inside the line law — the per-line offset
/// increment).
const LINE_SPACING: f32 = -80.0;

/// Layout font family name: feeds the advance law (it only affects the
/// fallback space advance ratio). The product uses the regular band of a
/// commercial font; the open-source baked font takes the default band (the
/// 5/24 branch of `space_advance_ratio`).
pub const FONT_FAMILY: &str = "Sans";

/// 单元格内边距（px）：字形墨迹离格边的留白。
const CELL_PAD: f32 = 2.0;
/// 单页起始边长；实际边长受设备上限约束，放不下的字形继续入下一页。
const MIN_ATLAS_SIZE: usize = 2048;

/// 文字色（真值：`fontColor` #555577FF）。
const TEXT_COLOR: [f32; 3] = [85.0 / 255.0, 85.0 / 255.0, 119.0 / 255.0];

/// Colour of the background and the arrow: white × α0.9019608 (the
/// `m_Color` of the two CustomImage pieces). The sprite tint multiplies the texture pixels (including
/// their own transparent corners), the same formula as uGUI's vertex colour
/// × texture.
const SHAPE_ALPHA: f32 = 0.9019608;
const SHAPE_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, SHAPE_ALPHA);

/// Side length and four-edge border of the background 9-slice source
/// (`btn_r30_wh` 80×80,
/// border 38/38/38/38 — read from the UI atlas crop ledger by pack identity;
/// the Image component renders it as Sliced). The border is sampling
/// geometry and does not change with the image.
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

/// Anchor world offset (the HUD base class's (0, 1, 0) — added to the target
/// position **before** projection, so it scales with distance; the anchor is
/// the avatar root position, not the skull). The anchor is the avatar root
/// position + offset for a one-member dialogue; for several members it is
/// the mean of the members' root positions + offset (a configured dialogue
/// also averages in a target fixture item; here the one-member form).
const ANCHOR_OFFSET: Vec3 = Vec3::new(0.0, 1.0, 0.0);

/// The coordinate scale of the TextMeshPro layout (an internal constant of
/// the line law; placement conversions use the same coordinate system).
pub const TEXT_SCALE: f32 = 2.0;
/// The ascent/descent line ratios of the line law (two forms, 66/75 and
/// 9/75).
pub const ASCENT_RATIO: f32 = 66.0 / 75.0;
pub const DESCENT_RATIO: f32 = 9.0 / 75.0;

/// 仓内开源字体（OFL-1.1 授权文本与字体同目录进仓）。
use crate::ui_font::UI_FONT as FONT_BYTES;

pub use crate::ui_inputs::atlas::{ascii_or, tmp_zero_metric};

// ---------------------------------------------------------------------------
// Balloon skin loading (the tweet master table and the three greeting tables
// belong to the NPC side: `crate::npc_tweet`)
// ---------------------------------------------------------------------------

/// Load handles for the two balloon skin images (bg nine-slice source +
/// arrow). Requested at Startup; `bake_atlas` inserts `BalloonArt` only once
/// both are present — the resource's presence is the latch, a failed load
/// panics loudly and falls back to no stand-in.
#[derive(Resource)]
pub struct SkinHandle {
    bg: Handle<Image>,
    arrow: Handle<Image>,
}

/// Startup: requests the load of the two balloon skin images.
pub fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(SkinHandle {
        bg: load_image(&server, moly_assets::ui_atlas_sprite("CommonAtlas", "btn_r30_wh")),
        arrow: load_image(
            &server,
            moly_assets::ui_atlas_sprite("CommonAtlas", "balloon_direction_triangle_wh"),
        ),
    });
}

// ---------------------------------------------------------------------------
// Loading of the glyph atlas and the balloon skin (UI atlas crops)
// ---------------------------------------------------------------------------

pub use crate::ui_inputs::atlas::{BalloonArt, GlyphCell};

/// A text laid out by [`walk_glyphs`], centred vertically: `top` is the
/// distance from its first line's baseline to the box's top edge (the
/// ascender share of the largest line size), `bottom` from its last line's
/// baseline to the bottom edge (the descender share), and `width` the widest
/// line's pen. The player view's marquees take these numbers from the
/// balloon atlas so that a scrolling title and a balloon lay their text out
/// with the same advances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextExtent {
    pub top: f32,
    pub bottom: f32,
    pub width: f32,
}

/// The extent of `text` at `font_size` in the balloon atlas's glyphs: every
/// character drawn by a baked glyph puts one more of its advances on the
/// pen, and the walk's pen is the extent (the law's line widths are its
/// arithmetic, not the walk's). The share of the font size a balloon's box
/// adds above and below the walk is kept, so a balloon and this measure
/// agree on where the text's first and last baselines sit.
pub fn text_extent(
    art: &BalloonArt,
    text: &str,
    font_size: f32,
    line_spacing: f32,
) -> TextExtent {
    let metrics = layout_metrics(
        text,
        font_size,
        FONT_FAMILY,
        line_spacing,
        BAKE_PPEM,
        &|ch: char| art.advance(ch),
    );
    // The law's widest visible line is the extent of a text written by one
    // style: its advances are the balloon walk's.
    let width = metrics
        .rect_widths
        .iter()
        .copied()
        .fold(0.0, f32::max);
    let line_count = metrics.rect_widths.len().max(1) as f32;
    let pitch = (line_count - 1.0) * (font_size - line_spacing) / TEXT_SCALE;
    TextExtent {
        top: font_size * ASCENT_RATIO,
        bottom: font_size * DESCENT_RATIO + pitch,
        width,
    }
}

/// One frame of a text's marquee (`UIPartsMarquee.Init` then
/// `UpdateMarquee`), as `set_text` writes its x into the view: `init_x` is
/// `GetInitPositionX` (half the text's width, its pivot 0.5), `restart_x` its
/// `GetRestartPositionX` (the text's right edge at the mask's left edge, plus
/// `lateral`), `end_x` its `GetEndPositionX` (the text's left edge at the
/// mask's right edge, less `lateral`). `needs` is `NeedsMarqueeMoveWidth`;
/// a text that fits is shown at its start and one that does not scrolls.
/// A new text (`fresh`) starts over, as `MusicInfoContent.Setup`'s `Init`.
pub fn step_marquee(
    marquee: &mut Marquee,
    fresh: bool,
    needs: bool,
    dt: f32,
    speed: f32,
    wait_seconds: f32,
    init_x: f32,
    restart_x: f32,
    end_x: f32,
) -> f32 {
    if fresh {
        marquee.start(needs, init_x, restart_x, wait_seconds);
    } else {
        marquee.advance(needs, dt, speed, wait_seconds, restart_x, end_x);
    }
    marquee.x
}

/// A text the player view's marquees scroll: the text object inside a
/// `UIPartsMarquee`, the mask the marquee clips it to and that mask's
/// centre column, and the text's own local centre column (the node
/// `marqueeTarget` is `UIPartsMarquee`'s, found by walking down from the
/// marquee to the first CustomTextMesh). `text` is a node of the view the
/// marquee draws; its x position is `UIPartsMarquee.UpdateMarquee`'s
/// `anchoredPosition.x`.
#[derive(Debug, Clone)]
pub struct MarqueeText {
    pub marquee: String,
    pub mask: String,
    pub text: String,
    pub mask_centre: f32,
    pub text_centre: f32,
    pub mask_half_width: f32,
    /// The text's pivot column (0 = its left edge, 1 its right edge).
    pub text_pivot: f32,
}

/// `UIPartsMarquee`'s geometry, which is not serialized: the marquee target
/// is the text in the marquee (the marquee object itself carries no graphic
/// to hold it), and the mask is `mask2D`, else `softMask`, which every marquee
/// of the player view has. A marquee with neither, or without a text under
/// it, places no text here and is named once.
pub fn marquee_text(
    layouts: &crate::ui_layout::UiLayouts,
    doc: &moly_assets::ui_layout::UiPrefab,
    view: &crate::ui_layout::UiPrefabView,
    marquee: &str,
    canvas: Vec2,
) -> Option<MarqueeText> {
    let index = doc.find(marquee).ok()?;
    let node = &doc.nodes[index];
    let component = |field: &str| {
        node.components
            .iter()
            .find(|component| component.class == "Sekai.UI.UIPartsMarquee")
            .and_then(|marquee| marquee.fields[field].as_array())
            .and_then(|pointer| pointer.get(1))
            .and_then(Value::as_i64)
            .filter(|id| *id != 0)
    };
    let referenced = |id: i64| doc.find(&format!("@{id}")).ok().map(|index| doc.nodes[index].path.as_str());
    let mask = component("mask2D")
        .or_else(|| component("softMask"))
        .and_then(referenced);
    let text = doc
        .nodes
        .iter()
        .enumerate()
        .find(|(other, node)| {
            *other != index
                && is_below(doc, *other, index)
                && node
                    .components
                    .iter()
                    .any(|component| component.class == "Sekai.UI.CustomTextMesh")
        })
        .map(|(_, node)| node.path.as_str());
    let (Some(mask), Some(text)) = (mask, text) else {
        warn!("[tweet] the marquee {marquee} of {} has no mask or no text target; it places no text", doc.prefab);
        return None;
    };
    let rects = view.rects(layouts, canvas)?;
    let mask_rect = rects.get(doc.find(mask).ok()?).cloned();
    let text_rect = rects.get(doc.find(text).ok()?).cloned();
    let (Some(mask_rect), Some(text_rect)) = (mask_rect, text_rect) else {
        warn!("[tweet] the marquee {marquee} of {} does not name its mask or its text in the view", doc.prefab);
        return None;
    };
    Some(MarqueeText {
        marquee: marquee.to_owned(),
        mask: mask.to_owned(),
        text: text.to_owned(),
        mask_centre: mask_rect.center().x,
        text_centre: text_rect.center().x,
        mask_half_width: mask_rect.size.x * 0.5,
        text_pivot: text_rect.pivot.x,
    })
}

/// Whether node `index` is a descendant of node `ancestor`.
fn is_below(doc: &moly_assets::ui_layout::UiPrefab, index: usize, ancestor: usize) -> bool {
    let mut cursor = doc.parent(index);
    while let Some(parent) = cursor {
        if parent == ancestor {
            return true;
        }
        cursor = doc.parent(parent);
    }
    false
}

/// A scrolling text's state (`UIPartsMarquee`'s `isMove`, `isWait`,
/// `wait_sec_cnt` and the text's x).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Marquee {
    moving: bool,
    waiting: bool,
    waited: f32,
    pub x: f32,
}

impl Marquee {
    /// Starts the scroll: `needed` is `UIPartsMarquee.NeedsMarqueeMoveWidth`
    /// (the text is wider than its mask). A text that fits goes back to the start at once; a wider one
    /// starts `UIPartsMarquee.GetInitPositionX` (the text's centre column
    /// times its pivot) to the right of the mask (`GetRestartPositionX`),
    /// waits `wait_sec`, then moves; `UIPartsMarquee.Init`.
    pub fn start(&mut self, needed: bool, init_x: f32, restart_x: f32, wait_seconds: f32) {
        self.waited = 0.0;
        if needed {
            self.moving = true;
            self.x = restart_x;
            self.waiting = wait_seconds > 0.0;
        } else {
            self.moving = false;
            self.waiting = false;
            self.x = init_x;
        }
    }

    /// One frame of `UIPartsMarquee.UpdateMarquee`: the wait runs down, then
    /// the text moves left at `speed`; at `end_x` it goes off and scrolls
    /// again from `restart_x` (the marquee's end restarts it).
    pub fn advance(
        &mut self,
        needed: bool,
        dt: f32,
        speed: f32,
        wait_seconds: f32,
        restart_x: f32,
        end_x: f32,
    ) {
        if !needed || !self.moving {
            return;
        }
        if self.waiting {
            self.waited += dt;
            if self.waited < wait_seconds {
                return;
            }
            self.waiting = false;
        }
        self.x -= speed * dt;
        if self.x <= end_x {
            self.x = end_x;
            self.start(true, restart_x, restart_x, wait_seconds);
        }
    }
}

/// The atlas bake's hold: no balloon is drawn until its skin images are in.
const ATLAS_BAKE: crate::hold::HoldSite = crate::hold::HoldSite {
    tag: "tweet",
    what: "balloon atlas bakes",
    effect: "no tweet balloon is drawn",
    quiet: &[],
};

/// Update: bakes the atlas once the master table is ready (the resource's
/// presence is the latch). The font is in the repository (include_bytes) and
/// is rasterised by swash's `scale::Render`. Glyphs are baked in ascending
/// code-point order; the two skin images, same image. The two skin images
/// (the bg and arrow crops) are gated in the same call: a failed load is
/// refused with a named message, and while they are incomplete the next frame
/// waits — BalloonArt is inserted only once both are present, and no path
/// generates a stand-in image afterwards.
pub fn bake_atlas(
    mut commands: Commands,
    server: Res<AssetServer>,
    time: Res<Time>,
    render_device: Res<RenderDevice>,
    mut images: ResMut<Assets<Image>>,
    tables: Option<Res<crate::npc_tweet::TweetTables>>,
    // 对话候选字符集：与 tweet 并集一次烘全（之后到的字符不上屏，
    // 候选集在对话模块的预筛侧已定格）。
    talk_charset: Option<Res<crate::talk::TalkCharset>>,
    // 玩家对话字符集：同一张图集的第三个并集成员（窗体正文/名字栏与
    // tweet 文案共用）。装载期定格、全 unit 收录——候选预筛在名册后
    // 才做，此处按超集并（字形只多不少）。
    player_talk_charset: Option<Res<crate::player_talk::PlayerTalkCharset>>,
    // 场地屏外壳字符集（固定文案 + 站名）：第四个并集成员。与上面三个
    // 同一定格语义：外壳文案全部装载期可枚举（站点主表是根级小表）。
    shell_charset: Option<Res<crate::ui_charsets::ShellTextCharset>>,
    // Collect-item notice characters (the masters' names and the count):
    // the fifth member, same once-baked semantics.
    collect_charset: Option<Res<crate::ui_charsets::CollectNoticeCharset>>,
    // The BGM select screen's record titles, artists, vocal texts and
    // wordings: the sixth member, same once-baked semantics.
    bgm_select_charset: Option<Res<crate::ui_charsets::BgmSelectCharset>>,
    skin: Option<Res<SkinHandle>>,
    art: Option<Res<BalloonArt>>,
    mut hold: Local<crate::hold::HoldReport>,
    mut text_revision: Local<(u64, u64)>,
) {
    if let Some(art) = art.as_deref() {
        let revision = crate::text_localization::revision();
        let font_revision = crate::ui_font::text_font_revision();
        if (revision, font_revision) == *text_revision { return; }
        let additional = crate::text_localization::characters();
        // An updated font can change advances and pixels for existing code
        // points. Reuse cells only when the selected font bytes also match.
        if font_revision == text_revision.1 && additional.iter().all(|ch| ch.is_control() || tmp_zero_metric(*ch) || art.cells.contains_key(ch)) {
            crate::text_localization::mark_glyphs_ready(revision);
            *text_revision = (revision, font_revision);
            return;
        }
    }
    let now = time.elapsed_secs();
    let retained_skin = art.as_deref().map(|art| SkinHandle { bg: art.bg.clone(), arrow: art.arrow.clone() });
    let skin = skin.as_deref().or(retained_skin.as_ref());
    let Some(skin) = skin else {
        return hold.hold(&ATLAS_BAKE, "the balloon skin handles are not installed", now);
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
        return hold.hold(&ATLAS_BAKE, "the balloon skin images are still loading", now);
    }
    // Size check: when a crop's size drifts from the nine-slice/arrow
    // sampling constants, sampling is wrong silently —
    // a named refusal. Both values are read from the sprite ledger aligned by
    // pack identity.
    if art.is_none() {
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
    }
    let missing = [
        (tables.is_none(), "the tweet tables are not installed"),
        (talk_charset.is_none(), "the talk charset is not installed"),
        (player_talk_charset.is_none(), "the player talk charset is not installed"),
        (shell_charset.is_none(), "the shell charset is not installed"),
        (collect_charset.is_none(), "the collect notice charset is not installed"),
        (bgm_select_charset.is_none(), "the bgm select charset is not installed"),
    ]
    .into_iter()
    .find_map(|(missing, reason)| missing.then_some(reason));
    if let Some(reason) = missing {
        return hold.hold(&ATLAS_BAKE, reason, now);
    }
    let (
        Some(tables),
        Some(talk_charset),
        Some(player_talk_charset),
        Some(shell_charset),
        Some(collect_charset),
        Some(bgm_select_charset),
    ) = (
        tables,
        talk_charset,
        player_talk_charset,
        shell_charset,
        collect_charset,
        bgm_select_charset,
    )
    else {
        unreachable!("every input was checked above");
    };
    hold.run(&ATLAS_BAKE, now);
    let font = swash::FontRef::from_index(&FONT_BYTES, 0)
        .unwrap_or_else(|| panic!("仓内字体不是可读的 OpenType 字体"));
    let mut chars: Vec<char> = tables
        .tweets()
        .iter()
        .flat_map(|row| row.text.chars())
        .chain(talk_charset.chars.iter().copied())
        .chain(player_talk_charset.chars.iter().copied())
        .chain(shell_charset.chars.iter().copied())
        .chain(collect_charset.chars.iter().copied())
        .chain(bgm_select_charset.chars.iter().copied())
        .chain(crate::text_localization::characters())
        // Rebuild from the source and selected display session. Old sprites
        // own their page handles; obsolete glyphs must not keep obsolete font
        // revisions necessary for every future atlas bake.
        .collect();
    chars.sort_unstable();
    chars.dedup();
    // TMP lays these out with its synthesized zero-metric glyph: nothing to
    // bake, and not a missing glyph.
    let zero_metric = chars.iter().filter(|ch| tmp_zero_metric(**ch)).count();
    chars.retain(|ch| !tmp_zero_metric(*ch));
    // The text normalisation point for the atlas: a control character
    // (general category Cc) never asks for a glyph. U+000A is the line break
    // every text walk splits on. U+000D reaches here from a UI prefab whose
    // serialized text ends a line with "\r\n"; the source's text generator
    // draws nothing for it and moves the pen back to the line's indent, which
    // the UI layout mirrors. The open font maps no control character, so
    // baking one could only report it missing.
    let controls = chars.iter().filter(|ch| ch.is_control()).count();
    chars.retain(|ch| !ch.is_control());

    let mut context = swash::scale::ScaleContext::new();
    let translation_fonts = crate::text_localization::fonts();
    let fallback_fonts: Vec<_> = translation_fonts.iter()
        .filter_map(|bytes| swash::FontRef::from_index(bytes, 0)).collect();
    let render = swash::scale::Render::new(&[swash::scale::Source::Outline]);

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
        let font = if font.charmap().map(ch as u32) != 0 { font } else {
            fallback_fonts.iter().copied().find(|font| font.charmap().map(ch as u32) != 0).unwrap_or(font)
        };
        let gid = font.charmap().map(ch as u32);
        let glyph_metrics = font.glyph_metrics(&[]).scale(BAKE_PPEM);
        let mut scaler = context.builder(font).size(BAKE_PPEM).build();
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
        "字形图集烘成：{} 格（字符集 {}，{page_count} 页 {atlas_size}x{atlas_size}，每页容量 {page_capacity}，总容量 {capacity}，设备上限 {max_edge}），TMP 零度量字符 {zero_metric} 个（不烘），控制字符 {controls} 个（不烘），缺字形 {} 个 [{}]，烘制 {:.0}px 实际墨迹上界 {:.1} 下界 {:.1} ⇒ 格 {:.0}px 笔点=({pen_x:.0},{baseline_from_top:.0})",
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
    let baked_revision = crate::text_localization::revision();
    let baked = BalloonArt {
        images: glyph_images,
        cells,
        baseline_from_top,
        pen_x,
        cell,
        bg,
        arrow,
    };
    commands.queue(move |world: &mut World| {
        world.insert_resource(baked);
        world.remove_resource::<SkinHandle>();
        crate::text_localization::mark_glyphs_ready(baked_revision);
    });
    *text_revision = (baked_revision, crate::ui_font::text_font_revision());
}

// ---------------------------------------------------------------------------
// 触发与选取
// ---------------------------------------------------------------------------

/// 跨帧线性同余抽签：均匀落在 `[0, len)`。律只约束分布与「恰求值一次」，
/// 引擎序列不在律内；种子随成员与保存次数走。
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

/// NPC 头顶 tweet（HUD 事件 25）的气泡：寿命归 NPC 状态机，气泡只记
/// 自己的缩放与收场起点。
#[derive(Component)]
pub struct HudBalloon {
    /// animation 节点的当前缩放（收场从它起缩）。
    scale: f32,
    /// 收场起点：(上屏计时, 起缩值)。
    exit: Option<(f32, f32)>,
}

/// Update：消费 NPC 状态机的 HUD 事件（类型 25）。Show 上屏（每名成员
/// 一只 HUD 气泡，新的替换在屏那只）；Hide 从当前缩放起收场 0.2s；
/// Retire（宿主换站重置、成员离站挂起）立即撤该成员头上的全部气泡
/// （含家具前置动作与摆设编辑反应那两条链的——头顶槽位只有一个，挂起
/// 的成员模型已隐藏）。独立场景（内容库占场）里 Show 不上屏，与背景
/// 气泡让位同一条门。图集未就绪时到达的事件读掉并具名。家具前置动作、
/// 摆设编辑反应与对话预览的气泡的上屏不归这里。
pub fn hud_event(
    mut commands: Commands,
    library: Res<crate::field_modes::LibraryMode>,
    art: Option<Res<BalloonArt>>,
    mut events: MessageReader<crate::npc_state::TweetHudEvent>,
    mut balloons: Query<(Entity, &BalloonAnchor, &Elapsed, &mut HudBalloon)>,
    others: Query<(Entity, &BalloonAnchor), Without<HudBalloon>>,
) {
    use crate::npc_state::TweetHudEvent;
    let Some(art) = art else {
        let dropped = events.read().count();
        if dropped > 0 {
            warn!("[tweet] HUD 事件 {dropped} 条到达时气泡图集未就绪，未上屏");
        }
        return;
    };
    // 本次调用铺出的根：命令延迟生效，同帧后续的 Hide / Retire 查询不到
    // 它们，这里自己记。它们还在缩放 0，收场即撤。
    let mut spawned: Vec<(Entity, Entity)> = Vec::new();
    fn drop_spawned(commands: &mut Commands, spawned: &mut Vec<(Entity, Entity)>, npc: Entity) {
        spawned.retain(|(owner, root)| {
            if *owner == npc {
                commands.entity(*root).despawn();
                false
            } else {
                true
            }
        });
    }
    for event in events.read() {
        match event {
            TweetHudEvent::Show {
                npc,
                unit,
                tweet_id,
                text,
                site_type,
            } => {
                if library.owns_scene() {
                    info!("[tweet] HUD show unit={unit} tweet={tweet_id}：独立场景占场，不上屏");
                    continue;
                }
                for (root, anchor, _, _) in &balloons {
                    if anchor.npc == *npc {
                        commands.entity(root).despawn();
                    }
                }
                drop_spawned(&mut commands, &mut spawned, *npc);
                let root =
                    spawn_balloon_text(&mut commands, &art, *npc, *unit, *tweet_id, text, None, false);
                commands.entity(root).insert(HudBalloon {
                    scale: 0.0,
                    exit: None,
                });
                spawned.push((*npc, root));
                info!(
                    "[tweet] HUD show unit={unit} tweet={tweet_id} site_type={site_type:?}：上屏，寿命归 NPC 状态机"
                );
            }
            TweetHudEvent::Hide {
                npc,
                unit,
                site_type,
            } => {
                drop_spawned(&mut commands, &mut spawned, *npc);
                for (_, anchor, elapsed, mut hud) in &mut balloons {
                    if anchor.npc == *npc && hud.exit.is_none() {
                        hud.exit = Some((elapsed.t, hud.scale));
                        info!(
                            "[tweet] HUD hide unit={unit} tweet={} site_type={site_type:?}：收场开始（起缩 {:.3}，0.2s Linear），上屏 {:.2}s",
                            anchor.tweet, hud.scale, elapsed.t
                        );
                    }
                }
            }
            TweetHudEvent::Retire { npc } => {
                drop_spawned(&mut commands, &mut spawned, *npc);
                let hud = balloons.iter().map(|(root, anchor, _, _)| (root, anchor));
                let mut removed = 0;
                for (root, anchor) in hud.chain(others.iter()) {
                    if anchor.npc == *npc {
                        commands.entity(root).despawn();
                        removed += 1;
                    }
                }
                if removed > 0 {
                    info!("[tweet] HUD retire npc={npc}：撤下头顶气泡 {removed} 只");
                }
            }
        }
    }
}

/// After-edit reaction hold: the keep segment after showing (the objective's
/// static `_waitTime` 5.0s, multiplied by 1000 into the delay wait).
#[derive(Component)]
pub struct AfterEditHold {
    /// 剩余驻留（秒）。
    remaining: f32,
    /// 第几次保存触发（收场日志对账用）。
    sequence: u32,
}

/// One save receipt's reaction pass (`MysekaiAfterEditNPCUtility.OnEdit` →
/// `ExecuteAfterEditNPCAction` → `ExecuteAfterEditLayout`). Each receipt runs
/// its own pass; passes do not cancel each other.
pub struct AfterEditPass {
    sequence: u32,
    /// The chosen reactors not yet reached, in the lottery's order.
    queue: std::collections::VecDeque<Entity>,
    /// The Delay after the last released reactor.
    delay: Option<ReactionDelay>,
}

/// `UniTask.Delay(ms, ignoreTimeScale: false, PlayerLoopTiming.Update)`:
/// created on `created`, it counts no time on that frame, then adds each
/// frame's scaled delta and completes on the first frame the sum reaches
/// `seconds`; the pass continues on that frame.
pub struct ReactionDelay {
    created: u32,
    elapsed: f32,
    seconds: f32,
}

/// The engine generator the reaction draws come from (`UnityEngine.Random`):
/// the reactor count and each stagger. Seeded from the platform, as the
/// other engine-draw sites of this host are; the engine's single shared
/// state across every caller is not modelled.
#[derive(Default)]
pub struct AfterEditRand(Option<moly_law::objective::appearance::EngineRand>);

impl AfterEditRand {
    fn get(&mut self) -> &mut moly_law::objective::appearance::EngineRand {
        self.0.get_or_insert_with(|| {
            let (a, b) = (
                crate::npc_objective::platform_seed(),
                crate::npc_objective::platform_seed(),
            );
            let state = [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32];
            moly_law::objective::appearance::EngineRand::from_state(if state == [0; 4] {
                [1, 0, 0, 0]
            } else {
                state
            })
        })
    }
}

/// `LotteryAfterEditReactionCharacters(siteType)`: the NPCs whose own site is
/// the edited one, ordered by distance to the player (a stable sort, as
/// `OrderBy` is), cut to `Random.Range(1, CeilToInt(n * 0.5) + 1)` of the
/// nearest. No NPC on the site: no draw and nobody.
fn lottery_after_edit_reactors(
    candidates: &[(Entity, Vec3)],
    player: Vec3,
    rand: &mut moly_law::objective::appearance::EngineRand,
) -> (Vec<Entity>, Option<i32>) {
    if candidates.is_empty() {
        return (Vec::new(), None);
    }
    let mut ordered: Vec<(Entity, f32)> = candidates
        .iter()
        .map(|(npc, position)| (*npc, position.distance(player)))
        .collect();
    ordered.sort_by(|a, b| a.1.total_cmp(&b.1));
    let half = ordered.len() as f32 * 0.5;
    let max_exclusive = if half.ceil() == f32::INFINITY {
        -2_147_483_647
    } else {
        half.ceil() as i32 + 1
    };
    let count = rand.range_int(1, max_exclusive);
    let take = usize::try_from(count).unwrap_or(0).min(ordered.len());
    (
        ordered[..take].iter().map(|(npc, _)| *npc).collect(),
        Some(count),
    )
}

/// Update: layout save receipt → reactor draw → staggered release one by one
/// → after-edit pool pick → tweet shown → 5.0s hold then close (the
/// presentation reuses [`spawn_balloon`], the same pipeline as the greeting
/// chain).
///
/// The product's pass, per receipt:
/// 1. `LotteryHighPriorityReactionTalk(updateFixtures)` first; a talk there
///    takes the high-priority path (unsupported: the updated-fixture list and
///    the condition-type-2 talk lookup are not carried here, named).
/// 2. Otherwise [`lottery_after_edit_reactors`] on the edited site; if any
///    chosen NPC is in the tutorial wait objective (19) the pass ends with no
///    reaction.
/// 3. `ExecuteAfterEditLayout`: for each chosen NPC in order, outside a
///    tutorial, `TryCancelCurrentObjective`; only when it reports true: the
///    after-edit talk data (type 12 at the NPC's position), the flag that
///    skips its next Rest, then `Random.Range(500, 1000)` milliseconds of
///    Delay before the next NPC. A refused cancel (talking, no live
///    objective) moves to the next NPC with no delay.
/// 4. After the pass, every NPC not chosen (outside a tutorial) gets
///    `ForceUpdateObjective`.
///
/// Here steps 2 and 3 run as the source orders and times them, with the
/// cancel answer read from the objective model ([`crate::npc_objective::ObjectiveMind::cancel_reports`]).
/// The released NPC's reaction objective (type 2: pick → look at the player →
/// tweet → 5.0 s → end) runs beside its objective loop, not inside it: the
/// objective is not cancelled, no after-edit talk data is set, step 4 is not
/// made, and the look at the player is not carried. Those need the objective
/// loop's after-edit route, named as a gap.
///
/// The objective body: **pick at the start** (`RandomPick` exactly once, the
/// law [`pick_after_edit_tweet`] matched row by row) → show → 5.0s hold →
/// close. The hold comes after showing. An empty pool closes silently with no
/// display and no hold; here the same semantics with one named line. Before
/// showing, the balloon on that member's head is closed first (there is one
/// objective slot — the HUD event consumer and this chain are ordered in
/// sequence in the schedule with an explicit sync point, so a HUD balloon
/// spawned in the same frame is visible and gives way in that frame).
pub fn after_edit_reaction(
    mut commands: Commands,
    time: Res<Time>,
    frames: Res<bevy::diagnostic::FrameCount>,
    clock: Option<Res<crate::npc_clock::NpcClock>>,
    site: Option<Res<crate::site::SiteActive>>,
    tutorial: Option<Res<crate::server_panel::ServerPanel>>,
    (hfsm_scope, reaction_receipts, hfsm_plans): (
        Option<Res<crate::npc_objective::controller::NpcHfsmScopeResource>>,
        Option<Res<crate::npc_objective::events::LayoutReactionFixtures>>,
        Res<crate::npc_objective::controller::PreparedPlans>,
    ),
    mut saves: MessageReader<crate::field_modes::LayoutSaved>,
    tables: Option<Res<crate::npc_tweet::TweetTables>>,
    art: Option<Res<BalloonArt>>,
    (mut sequence, mut after_edit, mut passes, mut rand): (
        Local<u32>,
        Local<Option<Vec<AfterEditRow>>>,
        Local<Vec<AfterEditPass>>,
        Local<AfterEditRand>,
    ),
    npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            &GlobalTransform,
            &crate::npc::NpcActions,
            Option<&crate::npc_objective::ObjectiveMind>,
            Has<crate::talk::TalkHold>,
        ),
        (
            With<CharacterShell>,
            Without<crate::player::PlayerControlled>,
        ),
    >,
    players: Query<&GlobalTransform, With<crate::player::PlayerControlled>>,
    mut holds: Query<(Entity, &CharacterUnitId, &mut AfterEditHold)>,
    hfsm_actors: Query<&crate::npc_objective::controller::NpcHfsmActor>,
    balloons: Query<(Entity, &BalloonAnchor)>,
) {
    // 池的 join 形 → 律的 AEH 行形，表一到就建一次（反查落空即数据
    // 损伤，响亮失败在装载期）。
    if after_edit.is_none() {
        if let Some(tables) = tables.as_deref() {
            *after_edit = Some(after_edit_rows(tables));
        }
    }
    // The engine's scaled frame delta (the Delay's clock), once the NPC clock
    // runs; before that the host's.
    let dt = clock
        .as_deref()
        .filter(|clock| clock.ready())
        .map_or_else(|| time.delta_secs(), |clock| clock.delta());
    let frame = frames.0;
    // The hold's closing segment runs first (the delay wait elapses →
    // ForceUpdateObjective): on expiry it gives way; a new reaction in the
    // same frame restarts the hold at the full value.
    for (npc, unit, mut hold) in &mut holds {
        hold.remaining -= dt;
        if hold.remaining <= 0.0 {
            info!(
                "[tweet] after-edit unit={} 保存 #{}：驻留满 {:.1}s，反应收场（objective 让位）",
                unit.0, hold.sequence, AFTER_EDIT_REACTION_DELAY_SECONDS
            );
            commands.entity(npc).remove::<AfterEditHold>();
        }
    }
    let in_tutorial = tutorial.as_deref().is_some_and(|panel| panel.is_tutorial());
    let receipts = saves.read().count();
    if receipts > 0 {
        let edited = site.as_deref().map(|site| site.site_type.clone());
        let player = players.iter().next().map(|g| g.translation());
        for _ in 0..receipts {
            *sequence += 1;
            let (Some(edited), Some(player)) = (edited.as_deref(), player) else {
                info!(
                    "[tweet] after-edit 保存 #{}：无在场站点或玩家，反应者抽签无输入，跳过",
                    *sequence
                );
                continue;
            };
            let candidates: Vec<(Entity, Vec3)> = npcs
                .iter()
                .filter(|(_, _, _, actions, _, _)| actions.site_type == edited)
                .map(|(npc, _, global, _, _, _)| (npc, global.translation()))
                .collect();
            let (chosen, count) = lottery_after_edit_reactors(&candidates, player, rand.get());
            let tutorial_wait = chosen.iter().any(|npc| {
                npcs.get(*npc).is_ok_and(|(_, _, _, _, mind, _)| {
                    mind.and_then(|mind| mind.current)
                        == Some(moly_law::objective::ObjectiveType::TutorialWait)
                })
            });
            info!(
                "[tweet] after-edit 保存 #{}：站点 {edited} 在站 {} 名 → Random.Range(1, ceil(n/2)+1) = {count:?} → 反应者 {:?}{}",
                *sequence,
                candidates.len(),
                chosen
                    .iter()
                    .filter_map(|npc| npcs.get(*npc).ok().map(|(_, unit, ..)| unit.0))
                    .collect::<Vec<_>>(),
                if tutorial_wait {
                    "；其中有教学等待目标，整次不反应"
                } else {
                    ""
                }
            );
            if tutorial_wait {
                continue;
            }
            passes.push(AfterEditPass {
                sequence: *sequence,
                queue: chosen.into(),
                delay: None,
            });
        }
    }
    let (Some(tables), Some(after_edit), Some(art)) = (tables, after_edit.as_ref(), art) else {
        return;
    };
    if reaction_receipts.as_deref().is_some_and(|r|!r.pending.is_empty()) || !hfsm_plans.reactions.is_empty() { return; }
    // Each pass walks its queue: past a finished Delay, release the next NPC
    // whose cancel reports true, then start that NPC's Delay.
    for pass in passes.iter_mut() {
        loop {
            if let Some(delay) = pass.delay.as_mut() {
                if delay.created == frame {
                    break;
                }
                delay.elapsed += dt;
                if delay.elapsed < delay.seconds {
                    break;
                }
                pass.delay = None;
            }
            let Some(npc) = pass.queue.pop_front() else {
                break;
            };
            // CanUpdateObjectiveInTutorial: always true outside a tutorial;
            // the tutorial branches are not carried (the host runs none).
            if in_tutorial {
                continue;
            }
            let Ok((_, unit, _, actions, mind, held)) = npcs.get(npc) else {
                continue;
            };
            if hfsm_plans.reaction_members.contains(&unit.0) { continue; }
            // TryCancelCurrentObjective: false while talking, without a
            // current objective, or on one that completed.
            // A dialogue hold is this host's talking presentation.
            let cancels = actions.current != crate::npc::NpcAction::Talk
                && !held
                && if hfsm_scope.as_deref().is_some_and(|scope|scope.0.enrols(unit.0)) {
                    hfsm_actors.get(npc).is_ok_and(|actor| actor.lower.is_none_or(|lower|lower.flags().can_interrupt))
                } else { mind.is_some_and(|mind| mind.cancel_reports()) };
            if !cancels {
                info!(
                    "[tweet] after-edit unit={} 保存 #{}：TryCancelCurrentObjective 为假（对话中或无在行目标），不反应、无错峰",
                    unit.0, pass.sequence
                );
                continue;
            }
            let ms = rand.get().range_int(500, 1000);
            pass.delay = Some(ReactionDelay {
                created: frame,
                elapsed: 0.0,
                seconds: ms as f32 / 1000.0,
            });
            info!(
                "[tweet] after-edit unit={} 保存 #{}：放行，下一名在 Random.Range(500, 1000) = {ms} ms 之后",
                unit.0, pass.sequence
            );
            let unit_id = unit.0 as i32;
            let mut draw = CountedDraw::new((unit_id as u64) << 32 | pass.sequence as u64);
            let picked =
                pick_after_edit_tweet(after_edit, &tables.wrt, tables.tweets(), unit_id, &mut draw);
            let Some(row) = picked else {
                info!(
                    "[tweet] after-edit unit={} save #{}: the resolvable pool is empty (no pool entries for this unit = no data); skipped without a draw (the objective closes with the same semantics)",
                    unit.0, pass.sequence
                );
                continue;
            };
            if hfsm_scope.as_deref().is_some_and(|scope| scope.0.enrols(unit.0)) {
                let tweet = row.id;
                commands.queue(move |world: &mut World| {
                    crate::npc_objective::events::begin_after_edit(world, npc, tweet);
                });
                continue;
            }
            // 新反应进场：头上的在屏气泡先关（驻留内重抽、跨链的问候
            // 气泡，同一条让位门——槽位只有一个）。
            for (balloon_entity, anchor) in &balloons {
                if anchor.npc == npc {
                    info!(
                        "[tweet] after-edit unit={} 保存 #{}：在屏 tweet={} 让位（objective 槽位互斥，重抽）",
                        unit.0, pass.sequence, anchor.tweet
                    );
                    commands.entity(balloon_entity).despawn();
                }
            }
            info!(
                "[tweet] after-edit unit={} 保存 #{}：池 {} 条 → tweet={} 入选（抽签恰 {} 次）→ 上屏，驻留 {:.1}s 起",
                unit.0,
                pass.sequence,
                draw.last_len,
                row.id,
                draw.calls,
                AFTER_EDIT_REACTION_DELAY_SECONDS
            );
            spawn_balloon(&mut commands, &art, npc, unit.0, row);
            commands.entity(npc).insert(AfterEditHold {
                remaining: AFTER_EDIT_REACTION_DELAY_SECONDS,
                sequence: pass.sequence,
            });
        }
    }
    passes.retain(|pass| pass.delay.is_some() || !pass.queue.is_empty());
}

/// 池的 join 形 → 律的 AEH 行形：每条池内 tweet 反查同角色同 tweet 的
/// WRT 行。提取产物以 join 形落盘（AEH 原行不在产物里），但链上引用
/// 是提取期 fail-loud 核过的——反查落空即数据损伤，响亮失败。
fn after_edit_rows(tables: &crate::npc_tweet::TweetTables) -> Vec<AfterEditRow> {
    let mut rows: Vec<AfterEditRow> = Vec::new();
    for (unit, tweet_ids) in &tables.after_edit_pools {
        for tweet_id in tweet_ids {
            let wrt_row = tables
                .wrt
                .iter()
                .find(|w| w.game_character_unit_id == *unit && w.tweet_id == *tweet_id)
                .unwrap_or_else(|| {
                    panic!("after-edit 池 {unit} 的 tweet {tweet_id} 无同角色 WRT 行")
                });
            rows.push(AfterEditRow {
                id: rows.len() as i32 + 1,
                without_related_talk_id: wrt_row.id,
            });
        }
    }
    info!(
        "[tweet] after-edit 池 {} 单元 {} 行",
        tables.after_edit_pools.len(),
        rows.len()
    );
    rows
}

// ---------------------------------------------------------------------------
// 排版与铺装
// ---------------------------------------------------------------------------

/// 一只气泡：挂在哪名成员、对账量、相机背面剔除的当前态。
#[derive(Component)]
pub struct BalloonAnchor {
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
    /// Layer-order probe balloon (the check script uses it to filter out the
    /// ordinary balloons on screen).
    probe: bool,
}

/// The node DOScale acts on (`_animationRoot`: the entry/exit scale is
/// applied to it, not to the root). Its origin is the bg centre, so the
/// balloon shrinks toward its centre.
#[derive(Component)]
// The detached root's reversible editor Visibility must reach its Sprite
// descendants through this non-rendering intermediate node. Bevy stops
// propagation at a parent without Visibility/InheritedVisibility; Sprite then
// falls back to visible, even while the BalloonAnchor itself is Hidden.
#[require(Visibility)]
pub struct BalloonAnim;

/// 气泡的存活计时与时序相位（入场完成/收场开始各报一次）。
#[derive(Component)]
pub struct Elapsed {
    t: f32,
    entered: bool,
    exiting: bool,
}

/// 首次投影成功后的置位闩（投影值只报一次，避免刷屏）。
#[derive(Component)]
pub struct Placed;

/// 层序探针的稳态闩（每部件的最终排序键只报一次——须在入场完成后的
/// 稳态打：入场/收场 DOScale 会把部件层差缩没，t=0 时无从比）。
#[derive(Component)]
pub struct LayerLogged;

/// 气泡部件的层族（同一气泡内的绘制序）。层 z 值见 [`PART_Z_BG`] 一族
/// 的出处注释。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Bg,
    Text,
    Arrow,
}

impl PartKind {
    /// Part layer z (local, multiplied under the animation node; mirrors the
    /// animation node's child order).
    fn layer_z(self) -> f32 {
        match self {
            PartKind::Bg => PART_Z_BG,
            PartKind::Text => PART_Z_TEXT,
            PartKind::Arrow => PART_Z_ARROW,
        }
    }

    /// Log name (ASCII — the check script parses it, non-ASCII never enters
    /// the log).
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
pub struct BalloonPart {
    base: Color,
    kind: PartKind,
}

/// 一个字形的落位：行内笔点 + 段样式。
#[derive(Clone, Copy)]
pub struct GlyphSpot {
    pub ch: char,
    /// 段字号（进推进律与显示缩放）。
    pub size: f32,
    /// 视觉缩放倍数：段的 `<scale>` × 大小写变换的字倍率。
    pub visual: f32,
    pub color: Option<[f32; 3]>,
    pub alpha: Option<f32>,
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

/// Walks the text glyph by glyph: the lines, the segment consumption order
/// and the pen advance all follow the line law's order — the advance uses the
/// same formula (`resolve_glyph_advance` × segment scale; `cspace` once per
/// character and again at the line end; a `<space=N>` fixed segment has no
/// consumption tracking). A glyph quad is emitted only for characters found
/// in the glyph table.
///
/// Characters the open font lacks follow the TMP lookup rather than a guess:
/// the code points TMP synthesizes into the static base asset
/// ([`crate::ui_inputs::atlas::TMP_ZERO_METRIC`]) advance zero and draw
/// nothing. Every other character is answered by the base asset's fallback
/// chain (the static on-demand asset, then the dynamic asset that adds glyphs
/// from the game's own font file), so TMP's missing-glyph replacement (U+25A1,
/// then U+0020) never fires on these texts; drawing a white square here
/// would be a replacement the product does not make. Such a character keeps the law's
/// fallback advance and is counted: its image is missing only because the
/// open font subset does not carry it.
/// `char_extra`/`word_extra` are the letter-spacing increments
/// (`m_characterSpacing` and `m_wordSpacing`, used as value × 0.01 × font
/// size, times 1 for the orthogonal font; a word increment is added after
/// each whitespace character) — both are 0 for balloons, and the dialogue
/// window passes non-zero values.
/// Returns (each line's glyphs, pen positions and **raw-text indices**, the
/// missing-glyph count). The raw-text index drives the typewriter's reveal
/// in the order of `text[i]` (a line break takes one step; the raw indices
/// count `\n`).
pub fn walk_glyphs(
    text: &str,
    art: &BalloonArt,
    font_size: f32,
    char_extra: f32,
    word_extra: f32,
) -> (Vec<Vec<(GlyphSpot, f32, usize)>>, usize) {
    let segments = parse_rich_segments(text);
    walk_segments(&segments, art, font_size, char_extra, word_extra)
}

fn walk_segments(
    segments: &[moly_law::text::tags::TextSegment],
    art: &BalloonArt,
    font_size: f32,
    char_extra: f32,
    word_extra: f32,
) -> (Vec<Vec<(GlyphSpot, f32, usize)>>, usize) {
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
        for seg in segments {
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
                    let zero = tmp_zero_metric(rendered_ch);
                    let glyph = if force_fallback_glyph(rendered_ch) {
                        None
                    } else if zero {
                        Some(0.0)
                    } else {
                        art.cells.get(&rendered_ch).map(|cell| cell.advance)
                    };
                    let step =
                        resolve_glyph_advance(glyph, rendered_ch, measure, BAKE_PPEM, FONT_FAMILY)
                            * char_scale
                            * seg_scale
                            + char_extra;
                    if zero {
                        // TMP's synthesized zero-metric glyph: no quad.
                    } else if glyph.is_some() {
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
                        // The source draws this glyph from its fallback chain;
                        // the open font subset lacks it: fallback advance, counted.
                        missing_used += 1;
                    }
                    pen += step;
                    pen += cspace;
                    // A word increment after each whitespace character
                    // (whitespace and zero-width space trigger it).
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
            let zero = tmp_zero_metric(raw_ch);
            let glyph = if force_fallback_glyph(raw_ch) {
                None
            } else if zero {
                Some(0.0)
            } else {
                art.cells.get(&raw_ch).map(|cell| cell.advance)
            };
            let step = resolve_glyph_advance(glyph, raw_ch, font_size, BAKE_PPEM, FONT_FAMILY)
                + char_extra;
            if zero {
                // TMP's synthesized zero-metric glyph: no quad.
            } else if glyph.is_some() {
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
) -> Entity {
    spawn_balloon_text(commands, art, npc, unit, row.id, &row.text, None, false)
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
    spawn_balloon_on(
        commands,
        art,
        npc,
        unit,
        id,
        text,
        probe_root,
        probe,
        BALLOON_LAYER,
    )
}

/// [`spawn_balloon_text`] on a given layer, for a host that draws its bubble
/// with its own views (the player view's greeting bubble).
pub fn spawn_balloon_on(
    commands: &mut Commands,
    art: &BalloonArt,
    npc: Entity,
    unit: u32,
    id: i32,
    text: &str,
    probe_root: Option<Vec3>,
    probe: bool,
    layer: usize,
) -> Entity {
    let session = crate::text_localization::snapshot();
    let translated = session.as_ref().map(|session| session.resolve(
        &format!("@tweet/{id}"), None, moly_text::TextKind::Tweet, text,
    ));
    let literal = translated.is_some_and(|display| display.origin != moly_text::TextOrigin::Original);
    let text = translated.map_or(text, |display| display.text);
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
    let segments = if literal {
        let mut segment = parse_rich_segments(" ").remove(0);
        segment.text = text.to_owned();
        vec![segment]
    } else { parse_rich_segments(text) };
    let metrics = moly_law::text::metrics::layout_segments(
        &segments,
        FONT_SIZE,
        FONT_FAMILY,
        LINE_SPACING,
        BAKE_PPEM,
        &|ch: char| art.advance(ch),
    );
    let (lines, missing_used) = walk_segments(&segments, art, FONT_SIZE, 0.0, 0.0);
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

    // Box geometry: baseline y (relative to the box centre, up positive) =
    // anchor base − line offset; top = first-line baseline + first-line font
    // size × ascent ratio, bottom = last-line baseline − last-line font size
    // × descent ratio (ratio constants 66/75 and 9/75). A line's font size is
    // the largest placement size on it; an empty line falls back to the
    // layer font size (as the line law does).
    // line_offsets are in 2x units (the law accumulates without folding), so
    // they are divided by TEXT_SCALE when used — the same use as in the
    // layout law (anchor base as is, line offsets divided by 2).
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
    // The law's box width includes a 32px padding constant (the widest
    // visible line + 32); subtracting it gives the text width.
    let text_w = (metrics.box_w - 32.0).max(1.0);
    // Box size = an approximation of the Layout preferred size: text width +
    // L40 + R40, text height + top 32 + bottom 32 (the product drives it
    // through the ContentSizeFitter/VLG chain; an approximation is noted).
    let bg_w = text_w + BG_PAD_LEFT + BG_PAD_RIGHT;
    let bg_h = text_h + BG_PAD_TOP + BG_PAD_BOTTOM;
    // Alignment: Center on one line, Left on several; vertical Middle is met
    // by the symmetric top and bottom padding (32/32).
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
            RenderLayers::layer(layer),
        ))
        .id();
    // animation node: the target of the entry/exit DOScale (_animationRoot).
    // Starts at 0 (DOScale starts from 0); tick overwrites it every frame.
    let anim = commands
        .spawn((
            BalloonAnim,
            Transform::from_xyz(0.0, bg_h / 2.0, 0.0).with_scale(anim_scale(0.0)),
            RenderLayers::layer(layer),
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
                RenderLayers::layer(layer),
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
                    RenderLayers::layer(layer),
                ))
                .id();
            commands.entity(anim).add_child(glyph);
        }
    }

    // Arrow: drawn outside its zero-height row — the row top = bg bottom
    // edge − row spacing 2.5, the arrow top = row top + 11.5 ⇒ it overlaps bg
    // by 9px (a seamless join). In the part layer order the arrow is on top
    // (the prefab `animation` child order: the arrow container comes after
    // the bg subtree, which therefore draws below — the arrow covers the 9px
    // it shares with bg).
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
            RenderLayers::layer(layer),
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

/// The nine-slice pieces: 4 corners + 4 edges + centre (the border 38 of
/// `btn_r30_wh` —
/// corners and edges are not scaled, the middle stretches; the same formula
/// as the Image component's Sliced mode, the middle is nearly uniform white
/// and stretched without a visible gradient). Returns
/// (source rectangle, display size, position relative to the bg centre).
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

// ---------------------------------------------------------------------------
// 时序与定位
// ---------------------------------------------------------------------------

/// Update：HUD 气泡 = 入场 DOScale 0→1（0.2s Linear）→ 保持到 Hide →
/// 从当前缩放收场（0.2s Linear）→ 缩出完成销毁；寿命归 NPC 状态机。
/// 其余气泡（摆设编辑反应、家具前置动作）= 入场 → 保持到 5.0s（tweet
/// 状态的时长）→ 收场 DOScale 1→0（0.2s Linear）→ 缩出完成销毁。5.0 的
/// 语义按律是严格大于（恰 5.0 的那一拍仍满尺寸，之后才收）。缩放写
/// animation 节点（非根）。
pub fn tick(
    mut commands: Commands,
    library: Res<crate::field_modes::LibraryMode>,
    time: Res<Time>,
    mut balloons: Query<(
        Entity,
        &BalloonAnchor,
        &mut Elapsed,
        &Children,
        Option<&ActivityBalloon>,
        Option<&TalkPreviewBalloon>,
        Option<&mut HudBalloon>,
    )>,
    mut anims: Query<&mut Transform, With<BalloonAnim>>,
    children_q: Query<&Children>,
    mut parts: Query<(&BalloonPart, &mut RenderLayers)>,
    layer: Option<Res<BubbleLayers>>,
    entities: &bevy::ecs::entity::Entities,
) {
    let dt = time.delta_secs();
    for (entity, anchor, mut elapsed, kids, activity, preview, hud) in &mut balloons {
        // A HUD balloon has no timer of its own; once its character is gone
        // nothing else would take it down.
        if hud.is_some() && !entities.contains(anchor.npc) {
            info!("[tweet] HUD balloon {entity:?}: its character {:?} is gone; taken down", anchor.npc);
            commands.entity(entity).despawn();
            continue;
        }
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
                    set_anim_scale(&mut transform, (elapsed.t / SCALE_SECONDS).clamp(0.,1.));
                }
            }
            continue;
        }
        if library.owns_scene() && activity.is_none() {
            commands.entity(entity).despawn();
            continue;
        }
        elapsed.t += dt;
        if let Some(mut hud) = hud {
            let scale = match hud.exit {
                None => (elapsed.t / SCALE_SECONDS).min(1.0),
                Some((since, from)) => {
                    let progress = (elapsed.t - since) / SCALE_SECONDS;
                    if progress >= 1.0 {
                        info!(
                            "[tweet] tweet={} 销毁：HUD 收场完成，上屏共 {:.2}s",
                            anchor.tweet, elapsed.t
                        );
                        commands.entity(entity).despawn();
                        continue;
                    }
                    from * (1.0 - progress)
                }
            };
            if !elapsed.entered && hud.exit.is_none() && elapsed.t >= SCALE_SECONDS {
                elapsed.entered = true;
                info!(
                    "[tweet] tweet={} 入场完成：DOScale 0→1 实际 {:.2}s（0.2s Linear）",
                    anchor.tweet, elapsed.t
                );
            }
            hud.scale = scale;
            for kid in kids.iter() {
                if let Ok(mut transform) = anims.get_mut(kid) {
                    set_anim_scale(&mut transform, scale);
                }
            }
            continue;
        }
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
                set_anim_scale(&mut transform, scale);
            }
        }
        if let Some(layer) = layer.as_deref() {
            apply_bubble_layer(layer, kids, &children_q, &mut parts);
        }
    }
}

/// Moves every part of a bubble to [`BubbleLayers`]' current layer (the
/// roster `raise_bubbles` moves the root's bubble to), so a bubble raised
/// over a screen shows on the HUD camera with all its parts: a shader draws
/// a sprite only on a layer it renders.
fn apply_bubble_layer(
    layer: &BubbleLayers,
    kids: &[Entity],
    children_q: &Query<&Children>,
    parts: &mut Query<(&BalloonPart, &mut RenderLayers)>,
) {
    let mut stack: Vec<Entity> = kids.to_vec();
    while let Some(entity) = stack.pop() {
        if let Ok((_, mut layers)) = parts.get_mut(entity) {
            let wanted = RenderLayers::layer(layer.0);
            if *layers != wanted {
                *layers = wanted;
            }
        }
        for children in children_q.get(entity) {
            stack.extend(children.iter().collect::<Vec<_>>());
        }
    }
}

/// PostUpdate: avatar root position + world offset → main camera screen
/// pixels → balloon layer world coordinates (1 unit = 1 logical pixel, origin
/// at the window centre). The root position is read every frame and the
/// balloon follows it.
/// Three things per frame (the two of the HUD's Update + canvas scaling):
/// * **Camera back-side culling**: while the target is behind the camera
///   (view-direction dot product ≤ 0) alpha=0 (not despawned; the position
///   stays at the previous frame's); restored when it returns to the front.
/// * **Top-edge scale**: when screen y>H/2 the balloon shrinks as a whole
///   (the top edge contracts 0.5×), y≤H/2 stays 1 — written on the root
///   (layered with the DOScale animation node, the two multiply).
/// * **canvas scaling**: see [`crate::canvas::RootCanvas::scale`].
///
/// No screen-edge clamp (drawn out of bounds; a frame with a failed
/// projection keeps the previous frame's position).
///
/// world_to_viewport already returns logical pixels (bevy uses
/// logical_viewport_rect), so dividing by scale_factor again would convert
/// them a second time; on a high-DPI machine that compresses every balloon
/// toward the window centre (measured at sf=1.5: each balloon shifts up-left
/// by about 1/1.5).
pub fn place(
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
    actors: Query<(&crate::npc::NpcActions, Has<crate::npc::gate_entries::ModelHidden>)>,
    site: Option<Res<crate::site::SiteActive>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    windows: Query<&Window>,
    children_q: Query<&Children>,
    mut parts: Query<(&BalloonPart, &mut Sprite)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    if balloons.is_empty() {
        return;
    }
    let Some(root_canvas) = root_canvas.as_deref() else {
        return;
    };
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
    let canvas = root_canvas.scale(window);
    // Cross-balloon layer order (the reorder `SetSiblingTweetHUD` makes
    // every frame): key = the Euclidean distance from the camera transform to
    // each balloon target (the avatar view root; the projection offset does
    // not take part — the sort key and the projection anchor are two
    // different values), sorted by descending distance → the far one is rank
    // 0; uGUI draws later siblings on top ⇒ the near one draws last and
    // covers the far one. bevy's 2D transparent phase draws in ascending
    // global z (same direction) ⇒ the rank folds into the root z: the nearer
    // is higher (ceiling − rank × stride). The HUD only reorders with ≥2
    // balloons; with one, rank 0 = the lowest band, the same as no reorder.
    // The reverse switch feeds 0 to every root z (the constant 0 before it
    // was wired), the negative arm of check ①.
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
        // Anchor: avatar root position + world offset (0,1,0) — the offset
        // is added before projection, so it scales with distance.
        let Some(root) = balloon_root(&anchor, &transforms) else {
            continue; // 变换还没传播（根位未就绪 = 恒等；探针根位直供）
        };
        let world = root + ANCHOR_OFFSET;
        // Camera back side (one IsNeedShow gate): alpha=0, not despawned. This
        // is the only gate (the view-direction dot product); the near plane
        // is **not** culled — Unity's WorldToScreenPoint still yields
        // coordinates for points in front of the near plane (a target close to
        // the face projects the balloon off screen or very large, with no
        // screen clamp). So `world_to_viewport` is not used: it returns the
        // `PastNearPlane` error for NDC z>1 (in front of the near plane) and
        // would silently drop balloons that are drawn (measured: following
        // the camera past a member at the side, the head 0.34m from the camera
        // < near 1.0, made 15 of 45 balloons never appear). Instead NDC is
        // folded to viewport pixels here.
        let view_z = (world - eye).dot(*fwd);
        let on_other_site = actors.get(anchor.npc).is_ok_and(|(actions, hidden)|
            hidden || site.as_deref().is_none_or(|site| site.site_type != actions.site_type));
        let behind = view_z <= 0.0 || on_other_site;
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
        // Root position = the anchor's screen position (the box centre lies
        // on the anchor: the world offset (0,1,0) is added before projection
        // and scales with distance, pinning the balloon centre above the
        // anchor). z = the cross-balloon rank band
        // (the nearer is higher, the farther lower; see the reorder described
        // in the function's header).
        let base = base_of(entity);
        transform.translation = Vec3::new(px.x - width / 2.0, height / 2.0 - px.y, base);
        // 顶边缩放（真值式）：y>H/2 时 scale = clamp((H−y)/(H/2))×0.5+0.5。
        let top_scale = if px.y > height / 2.0 {
            ((height - px.y) / (height / 2.0)).clamp(0.0, 1.0) * 0.5 + 0.5
        } else {
            1.0
        };
        // The root's total scale = top-edge scale × canvas scale (the first
        // is the HUD's own UpdateScale, the second the root canvas's
        // ScaleWithScreenSize, the two multiply; the anchor's screen position
        // takes no part — the localPosition is already divided by the canvas
        // scale, and multiplying it back nets screen pixels).
        // Same authored canvas/HUD scale in browser and native rendering.
        crate::canvas::set_scale(&mut transform, top_scale * canvas);
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
        // Layer-order probe (on only with MOLY_BALLOON_ORDER_SECS): latched
        // once in the steady state, reporting each part's final sort key. The
        // key uses the renderer's formula: root z + root scale × animation
        // node scale × part layer z (the animation node has no z
        // translation) — the keep segment with the entry DOScale=1 is exactly
        // the propagated value; at t=0 the scale is 0 and shrinks the part
        // layer differences away, so there is nothing to compare, and the
        // latch waits until the entry is
        // complete. The lines are all ASCII (the check script parses them;
        // text content does not enter the log).
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

/// The animation node's scale: the enter/exit tween grows the bubble in the
/// plane only. The node's z scale stays 1, so the parts keep their layer
/// offsets (background below text below arrow) from the first frame; with a
/// uniform scale the offsets are 0 while the tween starts.
fn anim_scale(scale: f32) -> Vec3 {
    Vec3::new(scale, scale, 1.0)
}

fn set_anim_scale(transform: &mut Mut<Transform>, scale: f32) {
    let scale = anim_scale(scale);
    if transform.scale != scale {
        transform.scale = scale;
    }
}

/// The depth a part was last queued for drawing at.
#[derive(Component)]
pub struct QueuedDepth(f32);

/// A part's depth moving farther than this re-queues it. It is above the
/// span of one bubble's layers and below the step between two bubbles.
const REQUEUE_DEPTH: f32 = 1.0;

/// PostUpdate, after transform propagation: re-queues the parts of a bubble
/// whose depth moved. The 2D transparent phase takes a mesh sprite's sort key
/// from its depth when the sprite is queued and keeps it until the sprite's
/// mesh or material changes. A bubble is placed after propagation, so its
/// parts are first queued at the spawn depth, and the stacking step
/// ([`BALLOON_LAYER_STRIDE`]) changes when another bubble appears or goes;
/// without a re-queue a part keeps drawing in the order of its first depth,
/// and a glyph can fall under the translucent background.
pub fn requeue_moved_parts(
    mut commands: Commands,
    mut parts: Query<(Entity, &GlobalTransform, &mut Mesh2d, Option<&mut QueuedDepth>), With<BalloonPart>>,
) {
    for (entity, transform, mut mesh, queued) in &mut parts {
        let depth = transform.translation().z;
        match queued {
            None => {
                commands.entity(entity).insert(QueuedDepth(depth));
            }
            Some(mut queued) if (depth - queued.0).abs() > REQUEUE_DEPTH => {
                queued.0 = depth;
                mesh.set_changed();
            }
            Some(_) => {}
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
pub struct OrderProbe;

/// Update: the layer-order probe (enabled when `MOLY_BALLOON_ORDER_SECS` is
/// greater than 0 seconds): once ready it spawns a pair of probe balloons
/// that overlap exactly on screen — the near root is [`ORDER_PROBE_NEAR`] metres straight
/// > ahead of the camera (on the view axis), the far anchor is twice that
/// > distance along **the same view ray through the near anchor** (both
/// > anchors project to the same screen point, the fully overlapping case
/// > seen in a user report), showing real tweet rows (the first two of the
/// > master table — the selection does not enter this chain; glyphs always
/// > exist and the text does not enter the log). On reaching the seconds it
/// > exits with `AppExit::Success` (headless acceptance); when the window runs
/// > out while still not ready (master table / atlas / camera not all ready)
/// > it also exits, and the check side sees it as missing log lines
/// > (fail-closed: with no probe there is no key at all).
pub fn order_smoke(
    mut commands: Commands,
    tables: Option<Res<crate::npc_tweet::TweetTables>>,
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
        // Spawned before → the window is reached, so stop; never spawned →
        // the readiness inputs were absent, so stop the same way (a missing
        // line is the loud failure on the check side).
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
    let (Some(tables), Some(art)) = (tables.as_deref(), art.as_deref()) else {
        return;
    };
    let Ok((_camera, cam_transform)) = cameras.single() else {
        return;
    };
    let (near_row, far_row) = match (tables.tweets().first(), tables.tweets().get(1)) {
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

/// The HUD overlay camera: the world's bubbles, the action buttons, the
/// joystick and the field HUD's own views.
#[derive(Component)]
pub struct HudCamera;

/// A bubble [`raise_bubbles`] moved to [`SCREEN_UI_LAYER`].
#[derive(Component)]
pub struct RaisedBubble;

/// The layer a bubble is drawn on: the HUD's while the field is the current
/// screen, the screen UI layer above it otherwise. Every part of a bubble
/// follows it ([`tick`]), so a bubble raised over a screen draws over it.
#[derive(Resource, Clone, Copy)]
pub struct BubbleLayers(pub usize);

impl Default for BubbleLayers {
    fn default() -> Self {
        Self(BALLOON_LAYER)
    }
}

/// Whether a bubble on this layer is currently drawn over a screen.
pub fn over_screen(layers: &BubbleLayers) -> bool {
    layers.0 == SCREEN_UI_LAYER
}

/// Startup：气泡覆盖相机。只画 [`BALLOON_LAYER`]，不清屏——叠在 3D
/// 画面上；渲染次序在主相机（order 0）之后。
pub fn overlay_camera(mut commands: Commands) {
    commands.spawn((
        HudCamera,
        Camera2d,
        crate::camera::MYSEKAI_CAMERA_MSAA,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        RenderLayers::layer(BALLOON_LAYER),
        crate::overlay_camera::IdleWhenEmpty,
    ));
}

/// PostUpdate: a world bubble (the HUD's own: a character's tweet, a talk
/// preview, a furniture pre-action's) is drawn on the HUD layer while the
/// field is the current screen, and on the screen UI layer otherwise, so a
/// pushed screen always draws over it. The bubble is only re-layered, as
/// `HudCamera` is the first overlay camera and draws it over the screen's
/// view; a bubble raised once stays raised until it is gone. Runs before the
/// render pass reads the layers.
pub fn raise_bubbles(
    mut commands: Commands,
    stack: Option<Res<crate::ui_layers::UiLayerStack>>,
    mut layer: ResMut<BubbleLayers>,
    bubbles: Query<
        (Entity, &RenderLayers, Option<&RaisedBubble>),
        Or<(
            With<BalloonAnchor>,
            With<ActivityBalloon>,
            With<TalkPreviewBalloon>,
        )>,
    >,
) {
    let Some(stack) = stack else {
        return;
    };
    let field = stack.on_field();
    layer.0 = if field { BALLOON_LAYER } else { SCREEN_UI_LAYER };
    for (entity, layers, raised) in &bubbles {
        if field {
            if raised.is_some() {
                commands
                    .entity(entity)
                    .insert(RenderLayers::layer(BALLOON_LAYER))
                    .remove::<RaisedBubble>();
            }
        } else if layers.intersects(&RenderLayers::layer(BALLOON_LAYER)) {
            commands
                .entity(entity)
                .insert((RenderLayers::layer(SCREEN_UI_LAYER), RaisedBubble));
        }
    }
}

/// A bubble a screen raises for one of its own nodes (the BGM select screen's
/// record greeting): it hangs below `owner`'s node `anchor` at that node's
/// place, with no world anchor, and its host takes it down.
/// `fx_mysekai_bgm_select_record`'s greeting event is not in the public asset
/// root, so the screen raises the HUD's balloon for it.
#[derive(Component)]
pub struct ScreenBubble {
    pub owner: Entity,
    pub anchor: String,
}

/// Raises a screen bubble below the owner's node `anchor`, drawn on `layer`,
/// and returns its root. The bubble grows with its own timer; the owner
/// places it every frame.
pub fn show_screen_bubble(
    commands: &mut Commands,
    owner: Entity,
    anchor: &str,
    art: &BalloonArt,
    layer: usize,
    text: &str,
) -> Entity {
    let root = spawn_balloon_on(commands, art, owner, 0, 0, text, None, false, layer);
    commands.entity(root).insert(ScreenBubble {
        owner,
        anchor: anchor.to_owned(),
    });
    root
}


/// A selected furniture pre-action owns its own bubble. Background greetings
/// stay suppressed in an independent scene; this source-authored bubble does
/// not start a second dialogue or overwrite the Timeline's pose/face channels.
#[derive(Component, Clone, Copy)]
pub struct ActivityBalloon(pub crate::fixture_activity_state::FixtureActivityOwner);

pub fn show_activity_balloon(
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
    let (mut commands, art) = state.get_mut(world).expect("system params");
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

pub fn cancel_activity_balloon(
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
        let font=swash::FontRef::from_index(&FONT_BYTES,0).expect("valid embedded OFL font");
        // Previously absent from the CN-only subset, creating silent holes in
        // Japanese dialogue despite Playing/scene-ready being true.
        for codepoint in [0x304C_u32,0x3054,0x3069,0x3060,0x3053,0x9055,0x6575] {
            assert_ne!(font.charmap().map(codepoint),0,"missing U+{codepoint:04X}");
        }
    }
}

/// Explicit source first-line preview, separate from a full dialogue session.
#[derive(Component)]
pub struct TalkPreviewBalloon { ticket: u64 }

pub fn show_talk_preview(world: &mut World, actor: Entity, unit: u32,
    tweet: &moly_law::talk::TweetRef, ticket: u64) {
    if tweet.id <= 0 || tweet.text.trim().is_empty() || world.get_entity(actor).is_err() { return; }
    let mut params = bevy::ecs::system::SystemState::<(Commands, Option<Res<BalloonArt>>)>::new(world);
    let (mut commands, art) = params.get_mut(world).expect("system params");
    if let Some(art) = art {
        let root = spawn_balloon_text(&mut commands, &art, actor, unit, tweet.id, &tweet.text, None, false);
        commands.entity(root).insert(TalkPreviewBalloon { ticket });
    }
    params.apply(world);
}
