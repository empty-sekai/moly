//! The option dialog (source `OptionDialog`, dialog type 82, a
//! `Common1ButtonDialog`), all five of its source pages.
//!
//! ## Entry and the first page
//!
//! The product settings panel opens this dialog; MySekai itself has no caller
//! of `OptionDialog.Setup`. The call this port follows is the out-game menu's
//! (the menu transition): `Setup(0, canAssetSetting: true,
//! canBlockListSetting: true, showCustomScoreTab: false,
//! showCommunicationTab: true)`. `Setup` disables the custom-score tab, enables
//! the communication tab and opens `(tabIndex | showCustomScoreTab) == 0 ? 1 :
//! tabIndex`, which is the Live page.
//!
//! ## Pages
//!
//! `Page`: CustomScore 0, Live 1, Volume 2, System 3, Communication 4 (the
//! tab group's toggle order). A tab change runs `DeactivateCurrentPage` (the
//! page's `UpdateLocalData`, except Communication, then `SetActive(false)`)
//! and `ActivateCurrentPage` (`SetActive(true)` and the page's `Setup` from the
//! dialog's data objects), then `UpdateTabLines`. So a page shown again starts
//! from the data objects: Live, Volume and System keep their edits through the
//! deactivation write; Communication does not.
//!
//! - **Live** (`OptionLiveSetting`, `OptionNoteSetting`): six numeric
//!   selectors and twelve toggle groups over `LiveSettingData`, which
//!   `LoadFromStorage` reads fresh at every open (no cache); the long-note and
//!   guide alphas write the data object on every change.
//! - **Volume** (`OptionVolumeSetting`): the six sliders, unchanged below.
//! - **System** (`OptionSystemSetting`): five toggle groups and five buttons.
//! - **Communication** (`OptionCommunicationSetting`): friend requests,
//!   story favorites, the block list.
//!
//! The pages bind through their classes' serialized references (the page
//! components, the numeric selectors and the toggle groups). A layout whose
//! page classes carry no decoded references shows that page's tab disabled,
//! with one warning naming the missing decode.
//!
//! ## Save and close
//!
//! The dialog shows through the screen manager (`DialogType.OptionDialog`,
//! 82); its back key is `Common1ButtonDialog.OnHardwareBackKeyProcess`, which
//! is `OnClickOK`, like `OnCloseExternal`.
//!
//! OK, a tap outside and the back key run `Save`: every set-up page's `UpdateLocalData`,
//! `UpdateServerData`, `LiveSettingData.SaveToStorage` and
//! `ApplicationLocalSettings.SaveToStorage`. The close button closes without
//! saving; the application settings object is the session's cached one, so a
//! deactivation's write stays in the session.
//!
//! ## Server values
//!
//! `UserConfig` (the online-status flag and the friend request status) is
//! server state: `MOLY_OPTION_MOCK_USER_CONFIG=<true|false>,<all|id_search|reject>`.
//! Without it the user config is null and the pages take the source's null
//! branches (online status off, friend request and story favorite toggles
//! left as `Awake` leaves them, all off) and `UpdateServerData` is not sent.
//!
//! ## Named gaps
//!
//! - The OptionDialog class itself is not decoded, so its tab group is
//!   reached by the prefab path `WindowRoot/Tabs`.
//! - Tab text colours (`UIPartsDialogTab.On/Off` writes the palette colours
//!   base_dbl / base_wh into the text): the view has no text colour override.
//! - Scroll views move by drag and clamp; `ScrollRect` elasticity, inertia and
//!   the scrollbar are not ported.
//! - The 120 fps confirmation (`Common2ButtonMediumDialog`), the timing tap
//!   dialog (`LiveNoteSettingDialog`), the note skin previews, the note SE
//!   names and test sounds (the live note bundles), the block list screen,
//!   the MV and music cache dialogs and the bulk downloads are other screens
//!   or live-game assets this product does not carry; their buttons log.
//! - `DisplayUtility.ActivateFixedScreenOrientation` has no host counterpart.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use serde_json::{json, Map, Value};

use crate::action_button::ActionTapConsumed;
use crate::audio::{
    apply_system_volume, stop_voice_all, LocalVolumeSettings, SeClass, SeRequest, SeRequests,
    VoiceChannel, VolumeBus, VolumeSettingData,
};
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::info::referenced_component;
use crate::menu_shell::ShellDialogState;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, UiLayerStack,
};

/// `Show1ButtonDialog(DialogType.OptionDialog)`.
const OPTION_DIALOG: DialogType = DialogType(82);

/// The dialog's layout document.
const KEY: &str = "Option";

/// Glyphs of the texts this dialog writes besides the source texts and
/// wordings: the numeric selectors' numbers.
pub(crate) const FIXED_TEXTS: &[&str] = &["0123456789.-+%"];

/// Wordings this dialog writes: the System buttons'
/// `UIPartsCommonButton.SetWordingKey` and the numeric selectors' button
/// texts (`SetupButton`). The atlas charset takes their glyphs from here.
pub(crate) const WORDINGS: &[&str] = &[
    "WORD_DOWNLOADED",
    "WORD_BULK_DOWNLOAD",
    "WORD_ADD_FORMAT",
    "WORD_SUBTRACT_FORMAT",
];

// ---------------------------------------------------------------------------
// Volume page identities (OptionVolumeSetting.SetupObject, read per slider)
// ---------------------------------------------------------------------------

/// 六滑杆之一（SetupObject 逐个对应，见模块头表）。序即草稿下标序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum VolumeSlider {
    LiveBgm,
    LiveSe,
    LiveVoice,
    SystemBgm,
    SystemSe,
    SystemVoice,
}

/// 六滑杆全集（序即草稿下标）。
const SLIDERS: [VolumeSlider; 6] = [
    VolumeSlider::LiveBgm,
    VolumeSlider::LiveSe,
    VolumeSlider::LiveVoice,
    VolumeSlider::SystemBgm,
    VolumeSlider::SystemSe,
    VolumeSlider::SystemVoice,
];

/// 组（本地档两组：LiveVolume / SystemVolume）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    Live,
    System,
}

impl Group {
    /// 组头文案（字段名族的自写标签，资产字符串未提取）。
    fn header(self) -> &'static str {
        match self {
            Group::Live => "Live",
            Group::System => "System",
        }
    }

    /// 组在本地档里的那一片。
    fn data<'a>(self, settings: &'a LocalVolumeSettings) -> &'a VolumeSettingData {
        match self {
            Group::Live => &settings.live,
            Group::System => &settings.system,
        }
    }
}

/// 滑杆的三类之一（BGM/SE/Voice——两组各三杆）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliderKind {
    Bgm,
    Se,
    Voice,
}

/// 预览声型（SetupObject 的型值，方法体直读）：0 BGM（cue 空，静默）·
/// 1 SE（PlaySEOneShot）· 2 Voice（PlayVoice）· 3 InGameSE（SamplePlaySE，
/// 音量 = 值×0.01）· 4 InGameVoice（PlayVoiceFixedVolume，音量 = 值×0.01）。
enum PreviewKind {
    Bgm,
    Se,
    Voice,
    IngameSe,
    IngameVoice,
}

impl PreviewKind {
    /// 账目行用的型名。
    fn label(self) -> &'static str {
        match self {
            PreviewKind::Bgm => "型 0 BGM",
            PreviewKind::Se => "型 1 SE",
            PreviewKind::Voice => "型 2 Voice",
            PreviewKind::IngameSe => "型 3 InGameSE",
            PreviewKind::IngameVoice => "型 4 InGameVoice",
        }
    }
}

impl VolumeSlider {
    fn group(self) -> Group {
        match self {
            VolumeSlider::LiveBgm | VolumeSlider::LiveSe | VolumeSlider::LiveVoice => Group::Live,
            VolumeSlider::SystemBgm | VolumeSlider::SystemSe | VolumeSlider::SystemVoice => {
                Group::System
            }
        }
    }

    fn kind(self) -> SliderKind {
        match self {
            VolumeSlider::LiveBgm | VolumeSlider::SystemBgm => SliderKind::Bgm,
            VolumeSlider::LiveSe | VolumeSlider::SystemSe => SliderKind::Se,
            VolumeSlider::LiveVoice | VolumeSlider::SystemVoice => SliderKind::Voice,
        }
    }

    /// 杆标签（组内三类名，自写）。
    fn kind_label(self) -> &'static str {
        match self.kind() {
            SliderKind::Bgm => "BGM",
            SliderKind::Se => "SE",
            SliderKind::Voice => "Voice",
        }
    }

    /// 草稿下标。
    fn index(self) -> usize {
        match self {
            VolumeSlider::LiveBgm => 0,
            VolumeSlider::LiveSe => 1,
            VolumeSlider::LiveVoice => 2,
            VolumeSlider::SystemBgm => 3,
            VolumeSlider::SystemSe => 4,
            VolumeSlider::SystemVoice => 5,
        }
    }

    /// 延时预览的 cue（SetupObject 逐个；BGM 两杆 cue 空 ⇒ 预览静默）。
    fn cue(self) -> Option<&'static str> {
        match self {
            VolumeSlider::LiveSe => Some("SE_VOLCHANGE_SE_INGAME"),
            VolumeSlider::LiveVoice => Some("SE_VOLCHANGE_VOX_INGAME"),
            VolumeSlider::SystemSe => Some("SE_VOLCHANGE_SE_UI"),
            VolumeSlider::SystemVoice => Some("SE_VOLCHANGE_VOX_SCENARIO"),
            VolumeSlider::LiveBgm | VolumeSlider::SystemBgm => None,
        }
    }

    fn preview_kind(self) -> PreviewKind {
        match self {
            VolumeSlider::LiveBgm | VolumeSlider::SystemBgm => PreviewKind::Bgm,
            VolumeSlider::LiveSe => PreviewKind::IngameSe,
            VolumeSlider::LiveVoice => PreviewKind::IngameVoice,
            VolumeSlider::SystemSe => PreviewKind::Se,
            VolumeSlider::SystemVoice => PreviewKind::Voice,
        }
    }

    /// 开框装值：档值 ×100 取整（真源 Setup：滑杆装 `(int)(值×100)` 的
    /// 整数位）。
    fn initial(self, settings: &LocalVolumeSettings) -> u8 {
        let value = match self.kind() {
            SliderKind::Bgm => self.group().data(settings).bgm,
            SliderKind::Se => self.group().data(settings).se,
            SliderKind::Voice => self.group().data(settings).voice,
        };
        (value * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

/// 草稿的一组 → 档值（OK 的 UpdateLoacalData：六值 ×0.01 写档对象）。
fn draft_group(draft: &[u8; 6], group: Group) -> VolumeSettingData {
    let (bgm, se, voice) = match group {
        Group::Live => (
            draft[VolumeSlider::LiveBgm.index()],
            draft[VolumeSlider::LiveSe.index()],
            draft[VolumeSlider::LiveVoice.index()],
        ),
        Group::System => (
            draft[VolumeSlider::SystemBgm.index()],
            draft[VolumeSlider::SystemSe.index()],
            draft[VolumeSlider::SystemVoice.index()],
        ),
    };
    VolumeSettingData {
        bgm: bgm as f32 * 0.01,
        se: se as f32 * 0.01,
        voice: voice as f32 * 0.01,
    }
}

/// 延时预览的延迟（真源 CP_DelayCall 的 0.15s 字面量）：每拍滑杆变更
/// 重排（b__0 先取消在途再重排）——停手 0.15s 后才响预览声。
const PREVIEW_DELAY_SECONDS: f32 = 0.15;

// ---------------------------------------------------------------------------
// Pages and the Setup call
// ---------------------------------------------------------------------------

/// `OptionDialog.Page` (None is `Option::None`); the tab group's toggle order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    CustomScore,
    Live,
    Volume,
    System,
    Communication,
}

const PAGES: [Page; 5] = [
    Page::CustomScore,
    Page::Live,
    Page::Volume,
    Page::System,
    Page::Communication,
];

impl Page {
    fn index(self) -> usize {
        self as usize
    }
}

/// `OptionDialog.Setup`'s arguments at the out-game menu's call site.
struct SetupArgs {
    tab_index: usize,
    can_asset_setting: bool,
    can_block_list_setting: bool,
    show_custom_score_tab: bool,
    show_communication_tab: bool,
}

const MENU_SETUP: SetupArgs = SetupArgs {
    tab_index: 0,
    can_asset_setting: true,
    can_block_list_setting: true,
    show_custom_score_tab: false,
    show_communication_tab: true,
};

impl SetupArgs {
    /// `((uint)tabIndex | showCustomScoreTab) == 0 ? 1 : tabIndex`.
    fn opening(&self) -> usize {
        if self.tab_index == 0 && !self.show_custom_score_tab {
            1
        } else {
            self.tab_index
        }
    }
}

// ---------------------------------------------------------------------------
// UIPartsNumericSelector
// ---------------------------------------------------------------------------

/// The selector's value in hundredths. The source keeps a `decimal`; every
/// setup value here is a `(decimal)float` (seven significant digits) or an
/// integer, and every step is a multiple of 0.01, so hundredths hold it except
/// a stored value with a third decimal, which rounds.
#[derive(Debug, Clone, PartialEq)]
struct Selector {
    value: i64,
    min: i64,
    max: i64,
    /// The variation values the buttons were set up with.
    steps: Steps,
    /// `_format`, appended after the number.
    suffix: &'static str,
    /// `_stringFormat` "F2"; the empty format prints the decimal as it is.
    two_decimals: bool,
}

/// `(decimal)value`: the float at seven significant digits, in hundredths.
fn decimal_hundredths(value: f32) -> i64 {
    let seven: f64 = format!("{:.6e}", value as f64).parse().expect("float text");
    (seven * 100.0).round() as i64
}

/// `SetupButton` calls: `Setup(float[])` sets button i up with value i (a
/// button past the array gets no listener); `Setup(float)` sets every button
/// up with the one value. Hundredths.
#[derive(Debug, Clone, PartialEq)]
enum Steps {
    PerButton(&'static [i64]),
    Every(i64),
}

impl Selector {
    fn new(
        value: i64,
        min: i64,
        max: i64,
        steps: Steps,
        suffix: &'static str,
        two_decimals: bool,
    ) -> Self {
        let mut selector = Selector {
            value,
            min: min * 100,
            max: max * 100,
            steps,
            suffix,
            two_decimals,
        };
        selector.update(value);
        selector
    }

    /// `UpdateValue`: clamp into [min, max].
    fn update(&mut self, value: i64) {
        self.value = value.min(self.max).max(self.min);
    }

    /// `decimal.ToString(format, InvariantCulture) + _format`.
    fn text(&self) -> String {
        let sign = if self.value < 0 { "-" } else { "" };
        let (whole, cents) = (self.value.abs() / 100, self.value.abs() % 100);
        let number = if self.two_decimals {
            format!("{sign}{whole}.{cents:02}")
        } else if cents == 0 {
            format!("{sign}{whole}")
        } else {
            format!(
                "{sign}{whole}.{}",
                format!("{cents:02}").trim_end_matches('0')
            )
        };
        format!("{number}{}", self.suffix)
    }

    /// `UpdateButtonEnable`: increments while `max - value > 0.0001`,
    /// decrements while `value - min > 0.0001`.
    fn can_increment(&self) -> bool {
        self.max - self.value > 0
    }

    fn can_decrement(&self) -> bool {
        self.value - self.min > 0
    }

    /// The variation value button `index` (either direction) adds or
    /// subtracts; None when that button has no listener.
    fn step(&self, index: usize) -> Option<i64> {
        match &self.steps {
            Steps::PerButton(steps) => steps.get(index).copied(),
            Steps::Every(step) => Some(*step),
        }
    }

    /// `Value` (`(float)_value`).
    fn value(&self) -> f32 {
        self.value as f32 / 100.0
    }
}

/// `LiveConfig.VariationValues[1]` = {1.0, 0.1}, in hundredths.
const VARIATION_VALUES_1: [i64; 2] = [100, 10];

/// `Mathf.Floor(value * 100)` of an alpha or brightness, as the integer the
/// percent selectors start from.
fn percent(value: f32) -> i64 {
    ((value * 100.0).floor() as i64) * 100
}

// ---------------------------------------------------------------------------
// Toggle groups
// ---------------------------------------------------------------------------

/// A `CustomIndexToggleGroup`'s selection: `None` while no toggle is on (the
/// state `Awake`'s `CollectToggles(false)` leaves), which `SelectedIndex`
/// reads as 0.
fn selected(state: Option<usize>) -> usize {
    state.unwrap_or(0)
}

/// `InitializeSelectIndex(!value ? 1 : 0)`.
fn from_bool(value: bool) -> Option<usize> {
    Some(if value { 0 } else { 1 })
}

/// The Live page's groups, in `OptionLiveSetting`'s field order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveToggle {
    CutIn,
    SkillAndPraise,
    NoteEffect,
    FeverEffect,
    SimultaneousPushingLine,
    Vibration,
    AllPerfectEffect,
    FastLateFlick,
    Mirror,
    Use120Fps,
    Mode,
    Quality,
}

const LIVE_TOGGLES: [(LiveToggle, &str); 12] = [
    (LiveToggle::CutIn, "_cutin"),
    (LiveToggle::SkillAndPraise, "_skillAndPraise"),
    (LiveToggle::NoteEffect, "_noteEffect"),
    (LiveToggle::FeverEffect, "_feverEffect"),
    (
        LiveToggle::SimultaneousPushingLine,
        "_simultaneousPushingLine",
    ),
    (LiveToggle::Vibration, "_vibration"),
    (LiveToggle::AllPerfectEffect, "_allPerfectEffect"),
    (LiveToggle::FastLateFlick, "_fastLateFlick"),
    (LiveToggle::Mirror, "_mirrorToggle"),
    (LiveToggle::Use120Fps, "_use120FpsToggle"),
    (LiveToggle::Mode, "_modeSetting"),
    (LiveToggle::Quality, "_qualitySetting"),
];

/// The Live page's selectors: `OptionLiveSetting` holds the first four,
/// `OptionNoteSetting` the two alphas.
const LIVE_SELECTORS: [&str; 6] = [
    "_noteSpeed",
    "_timingAdjust",
    "_brightness",
    "_laneAlpha",
    "_longNoteAlpha",
    "_guideAlpha",
];

/// The System page's groups, in `OptionSystemSetting`'s field order.
const SYSTEM_TOGGLES: [&str; 5] = [
    "defaultMusic",
    "screenFixed",
    "showNoteSpeed",
    "showLoginStatus",
    "notifyLiveBonusMax",
];
const SCREEN_FIXED: usize = 1;
const SHOW_LOGIN_STATUS: usize = 3;

/// The System page's buttons.
const SYSTEM_BUTTONS: [&str; 5] = [
    "voiceBulkDownload",
    "mvBulkDownload",
    "musicBulkDownload",
    "_mvSaveNumSettingButton",
    "_musicSaveNumSettingButton",
];

// ---------------------------------------------------------------------------
// Bindings: the source references, resolved once
// ---------------------------------------------------------------------------

/// A `CustomButton` or `CustomToggle` and the graphics its `HideCover`
/// (`OnEnable`) hides and its `ShowCover` (`OnDisable`, only when
/// `disableActionType` is Grayout) shows: the cover and the optional covers.
struct SelectableBinding {
    control: String,
    covers: Vec<String>,
    grayout: bool,
}

struct ToggleBinding {
    selectable: SelectableBinding,
    /// `Toggle.graphic` (the check mark), when set.
    graphic: Option<String>,
}

struct GroupBinding {
    toggles: Vec<ToggleBinding>,
}

struct SelectorBinding {
    text: String,
    decrement: Vec<SelectableBinding>,
    increment: Vec<SelectableBinding>,
    /// `_decrementButtonTexts` / `_incrementButtonTexts` (may be empty).
    decrement_texts: Vec<String>,
    increment_texts: Vec<String>,
}

struct ScrollBinding {
    content: String,
    viewport: String,
}

struct TabBinding {
    page: Page,
    selectable: SelectableBinding,
    background: String,
    line: Option<String>,
}

/// `UIPartsCommonButton`: its `CustomButton` and text.
struct CommonButtonBinding {
    button: SelectableBinding,
    text: String,
}

struct LiveBinding {
    selectors: Vec<SelectorBinding>,
    groups: Vec<GroupBinding>,
    vibration_root: String,
    timing_tap: SelectableBinding,
    skin: [SelectableBinding; 2],
    tap_se: [SelectableBinding; 2],
    tests: [SelectableBinding; 4],
}

struct SystemBinding {
    groups: Vec<GroupBinding>,
    buttons: Vec<CommonButtonBinding>,
}

struct CommunicationBinding {
    friend: GroupBinding,
    favorite: GroupBinding,
    block_list: SelectableBinding,
}

#[derive(Resource)]
pub(crate) struct OptionBindings {
    tabs: Vec<TabBinding>,
    /// Page nodes by page index.
    pages: Vec<Option<String>>,
    /// The page scroll views (`<page>/ScorollView`) by page index.
    scrolls: Vec<Option<ScrollBinding>>,
    live: Result<LiveBinding, String>,
    system: Result<SystemBinding, String>,
    communication: Result<CommunicationBinding, String>,
}

/// `@id` of a GameObject, RectTransform or component reference; None for a
/// null one.
fn object_ref(doc: &UiPrefab, reference: &Value) -> Option<String> {
    let reference = reference.as_array().expect("Option serialized reference");
    assert_eq!(
        reference[0].as_i64(),
        Some(0),
        "Option reference must be local"
    );
    let id = reference[1].as_i64().expect("Option reference identity");
    if id == 0 {
        return None;
    }
    let key = format!("@{id}");
    doc.find(&key)
        .unwrap_or_else(|e| panic!("Option reference: {e}"));
    Some(key)
}

/// The document's single component of `class`, when its serialized fields
/// were decoded (`probe` is one of them).
fn page_component<'a>(
    doc: &'a UiPrefab,
    class: &str,
    probe: &str,
) -> Result<&'a UiComponent, String> {
    let found: Vec<&UiComponent> = doc
        .nodes
        .iter()
        .flat_map(|n| n.components.iter())
        .filter(|c| c.class == class)
        .collect();
    let [component] = found[..] else {
        return Err(format!(
            "{}: {} {class} components, the dialog binds one",
            doc.prefab,
            found.len()
        ));
    };
    if component.fields.get(probe).is_none() {
        return Err(format!(
            "{}: {class} carries no decoded references (the layout predates its decoder)",
            doc.prefab
        ));
    }
    Ok(component)
}

fn selectable(doc: &UiPrefab, control: String, component: &UiComponent) -> SelectableBinding {
    let fields = &component.fields;
    let mut covers: Vec<String> = object_ref(doc, &fields["coverImage"]).into_iter().collect();
    if let Some(list) = fields["optionalCoverImages"].as_array() {
        covers.extend(
            list.iter()
                .filter_map(|reference| object_ref(doc, reference)),
        );
    }
    let grayout = fields["disableActionType"]
        .as_i64()
        .expect("Option selectable disableActionType")
        == 1;
    SelectableBinding {
        control,
        covers,
        grayout,
    }
}

fn toggle(doc: &UiPrefab, reference: &Value) -> ToggleBinding {
    let (control, toggle) = referenced_component(doc, reference, "Sekai.UI.CustomToggle");
    let graphic = object_ref(doc, &toggle.fields["graphic"]);
    ToggleBinding {
        selectable: selectable(doc, control, toggle),
        graphic,
    }
}

fn group(doc: &UiPrefab, reference: &Value) -> GroupBinding {
    let (_, group) = referenced_component(doc, reference, "Sekai.UI.CustomIndexToggleGroup");
    let toggles = group.fields["indexToggles"]
        .as_array()
        .expect("Option toggle references")
        .iter()
        .map(|reference| toggle(doc, reference))
        .collect();
    GroupBinding { toggles }
}

fn button(doc: &UiPrefab, reference: &Value) -> SelectableBinding {
    let (control, button) = referenced_component(doc, reference, "Sekai.UI.CustomButton");
    selectable(doc, control, button)
}

fn selector(doc: &UiPrefab, reference: &Value) -> SelectorBinding {
    let (_, s) = referenced_component(doc, reference, "Sekai.UIPartsNumericSelector");
    let buttons = |name: &str| -> Vec<SelectableBinding> {
        s.fields[name]
            .as_array()
            .unwrap_or_else(|| panic!("Option selector {name}"))
            .iter()
            .map(|reference| button(doc, reference))
            .collect()
    };
    let texts = |name: &str| -> Vec<String> {
        s.fields
            .get(name)
            .and_then(Value::as_array)
            .map_or_else(Vec::new, |list| {
                list.iter()
                    .map(|r| referenced_component(doc, r, "Sekai.UI.CustomTextMesh").0)
                    .collect()
            })
    };
    SelectorBinding {
        text: referenced_component(doc, &s.fields["_numText"], "Sekai.UI.CustomTextMesh").0,
        decrement: buttons("_decrementButtons"),
        increment: buttons("_incrementButtons"),
        decrement_texts: texts("_decrementButtonTexts"),
        increment_texts: texts("_incrementButtonTexts"),
    }
}

fn common_button(doc: &UiPrefab, reference: &Value) -> CommonButtonBinding {
    let (_, common) = referenced_component(doc, reference, "Sekai.UI.UIPartsCommonButton");
    CommonButtonBinding {
        button: button(doc, &common.fields["customButton"]),
        text: referenced_component(
            doc,
            &common.fields["customTextMesh"],
            "Sekai.UI.CustomTextMesh",
        )
        .0,
    }
}

/// The child of node `parent` named `name`.
fn child<'a>(
    doc: &'a UiPrefab,
    parent: usize,
    name: &str,
) -> Option<&'a moly_assets::ui_layout::UiNode> {
    let transform = doc.nodes[parent].transform_id;
    doc.nodes
        .iter()
        .find(|n| n.parent_transform_id == transform && n.name == name)
}

/// The tab group's toggles with their `UIPartsDialogTab`. A layout whose
/// `UIPartsDialogTab` is not decoded reaches the same two graphics through
/// the tab prefab's own children (`background`, `Line`).
fn tabs(doc: &UiPrefab) -> Vec<TabBinding> {
    let tabs_node = &doc.nodes[doc
        .find("WindowRoot/Tabs")
        .unwrap_or_else(|e| panic!("Option tabs: {e}"))];
    let tab_group = tabs_node
        .components
        .iter()
        .find(|c| c.class == "Sekai.UI.CustomIndexToggleGroup")
        .expect("Option tab group");
    let references = tab_group.fields["indexToggles"]
        .as_array()
        .expect("Option tab toggles");
    // The tab group holds one toggle per page; a client without the custom
    // score page has the four from Live.
    let pages: &[Page] = match references.len() {
        5 => &PAGES,
        4 => &PAGES[1..],
        n => panic!("Option tab group holds {n} toggles; the dialog knows four or five pages"),
    };
    references
        .iter()
        .zip(pages)
        .map(|(reference, page)| {
            let ToggleBinding { selectable, .. } = toggle(doc, reference);
            let node = doc.find(&selectable.control).expect("tab node");
            let tab = doc.nodes[node]
                .components
                .iter()
                .find(|c| c.class == "Sekai.UIPartsDialogTab")
                .expect("UIPartsDialogTab beside the tab toggle");
            let (background, line) = if tab.fields.get("backgroundImage").is_some() {
                (
                    referenced_component(
                        doc,
                        &tab.fields["backgroundImage"],
                        "Sekai.UI.CustomImage",
                    )
                    .0,
                    object_ref(doc, &tab.fields["lineObj"]),
                )
            } else {
                let background = child(doc, node, "background").expect("tab background");
                let image = background
                    .components
                    .iter()
                    .find(|c| c.class == "Sekai.UI.CustomImage")
                    .expect("tab background image");
                (
                    format!("@{}", image.path_id),
                    child(doc, node, "Line").map(|n| format!("@{}", n.game_object_id)),
                )
            };
            TabBinding {
                page: *page,
                selectable,
                background,
                line,
            }
        })
        .collect()
}

impl OptionBindings {
    fn from_prefab(doc: &UiPrefab) -> Self {
        let tabs = tabs(doc);
        let live = page_component(doc, "Sekai.OptionLiveSetting", "_noteSpeed").and_then(|live| {
            let note = page_component(doc, "Sekai.OptionNoteSetting", "_longNoteAlpha")?;
            let f = |name: &str| {
                if live.fields.get(name).is_some() {
                    &live.fields[name]
                } else {
                    &note.fields[name]
                }
            };
            Ok(LiveBinding {
                selectors: LIVE_SELECTORS
                    .iter()
                    .map(|name| selector(doc, f(*name)))
                    .collect(),
                groups: LIVE_TOGGLES
                    .iter()
                    .map(|(_, name)| group(doc, f(*name)))
                    .collect(),
                vibration_root: object_ref(doc, &live.fields["_vibrationRoot"])
                    .expect("vibration root"),
                timing_tap: button(doc, &live.fields["_timingAdjustTap"]),
                skin: [
                    button(doc, &note.fields["_skinChangeLeft"]),
                    button(doc, &note.fields["_skinChangeRight"]),
                ],
                tap_se: [
                    button(doc, &note.fields["_tapSeChangeLeft"]),
                    button(doc, &note.fields["_tapSeChangeRight"]),
                ],
                tests: [
                    "_testTapSeButton",
                    "_testFlickSeButton",
                    "_testLongSeButton",
                    "_testTraceSeButton",
                ]
                .map(|name| button(doc, &note.fields[name])),
            })
        });
        let system =
            page_component(doc, "Sekai.OptionSystemSetting", "defaultMusic").map(|system| {
                SystemBinding {
                    groups: SYSTEM_TOGGLES
                        .iter()
                        .map(|name| group(doc, &system.fields[*name]))
                        .collect(),
                    buttons: SYSTEM_BUTTONS
                        .iter()
                        .map(|name| common_button(doc, &system.fields[*name]))
                        .collect(),
                }
            });
        let communication = page_component(
            doc,
            "Sekai.OptionCommunicationSetting",
            "friendRequestToggle",
        )
        .map(|c| CommunicationBinding {
            friend: group(doc, &c.fields["friendRequestToggle"]),
            favorite: group(doc, &c.fields["displayStoryFavoriteToggle"]),
            block_list: button(doc, &c.fields["blockListButton"]),
        });
        let page_classes = [
            "Sekai.OptionCustomScoreSetting",
            "Sekai.OptionLiveSetting",
            "Sekai.OptionVolumeSetting",
            "Sekai.OptionSystemSetting",
            "Sekai.OptionCommunicationSetting",
        ];
        let page_nodes: Vec<Option<usize>> = page_classes
            .iter()
            .map(|class| {
                doc.nodes
                    .iter()
                    .position(|n| n.components.iter().any(|c| c.class == *class))
            })
            .collect();
        let pages = page_nodes
            .iter()
            .map(|node| node.map(|i| format!("@{}", doc.nodes[i].game_object_id)))
            .collect();
        let scrolls = page_nodes
            .iter()
            .map(|node| {
                let view = child(doc, (*node)?, "ScorollView")?;
                let rect = view
                    .components
                    .iter()
                    .find(|c| c.class == "Sekai.UI.CustomScrollRect")?;
                Some(ScrollBinding {
                    content: object_ref(doc, &rect.fields["m_Content"]).expect("scroll content"),
                    viewport: object_ref(doc, &rect.fields["m_Viewport"]).expect("scroll viewport"),
                })
            })
            .collect();
        for (name, missing) in [
            ("Live", live.as_ref().err()),
            ("System", system.as_ref().err()),
            ("Communication", communication.as_ref().err()),
        ] {
            if let Some(reason) = missing {
                warn!("[option] the {name} page is not built from this layout: {reason}; its tab shows disabled");
            }
        }
        OptionBindings {
            tabs,
            pages,
            scrolls,
            live,
            system,
            communication,
        }
    }

    /// Whether the page can be shown from this layout.
    fn built(&self, page: Page) -> bool {
        match page {
            Page::CustomScore => false,
            Page::Live => self.live.is_ok(),
            Page::Volume => self.pages[Page::Volume.index()].is_some(),
            Page::System => self.system.is_ok(),
            Page::Communication => self.communication.is_ok(),
        }
    }
}

// ---------------------------------------------------------------------------
// Data objects
// ---------------------------------------------------------------------------

/// `LiveSettingData`'s MessagePack keys with the constructor's values (read
/// natively: the reconstructed constructor drops the wide stores of the note
/// speed, brightness, lane transparency, note alpha and vibration).
fn live_setting_defaults() -> Map<String, Value> {
    let Value::Object(map) = json!({
        "NoteSpeed": 6.0, "TimingAdjustData": 0.0, "Brightness": 1.0, "LaneTransparent": 1.0,
        "UseCutIn": true, "HiddenSkillAndPraise": false, "UseSimultaneousPushingLine": true,
        "UseVibration": true, "UseAllPerfectEffect": true, "LiveMode": 1, "NoteAlpha": 1.0,
        "GuideAlpha": 0.6f32, "NoteSkinIndex": 0, "NoteSeIndex": 0, "IsMirror": false,
        "QualityType": 0, "IsFastLateFlick": false, "Use120FPS": false, "UsedVSync": null,
        "NoteEffect": 0, "_noteShowRate": 0.0, "FeverEffectTypeIndex": 0,
        "TotalPowerUpperLimit": null, "TotalPowerLowerLimit": null,
        "CustomRoomTotalPowerUpperLimit": null, "CustomRoomTotalPowerLowerLimit": null,
        "ShowsRoomId": true, "CustomRoomScoreSettingIndex": 0, "CustomRoomIsDisplayPlayerInfo": true,
        "CustomRoomSelectedDifficulties": null, "CustomRoomSelectedMusicType": 0, "ScoreSelectType": 0,
    }) else {
        unreachable!()
    };
    map
}

/// The stored key of `LiveSettingData` (its own persistent object in the
/// source, a section of the local settings document here).
const LIVE_SECTION: &str = "LiveSettingData";

/// `LiveSettingData`, as the stored object.
#[derive(Debug, Clone, PartialEq)]
struct LiveData(Map<String, Value>);

impl LiveData {
    /// `LoadFromStorage`: the stored object over the constructor's values; a
    /// stored member of another type than the constructor's keeps the latter.
    fn load(document: &Value) -> Self {
        let mut data = live_setting_defaults();
        match &document[LIVE_SECTION] {
            Value::Null => {}
            Value::Object(stored) => {
                for (key, value) in stored {
                    let same_kind = match data.get(key) {
                        Some(Value::Bool(_)) => value.is_boolean(),
                        Some(Value::Number(_)) => value.is_number(),
                        Some(Value::Null) | None => true,
                        Some(_) => false,
                    };
                    if same_kind {
                        data.insert(key.clone(), value.clone());
                    } else {
                        warn!("[option] {LIVE_SECTION}.{key}={value} is not the member's type; the constructor value stays");
                    }
                }
            }
            other => warn!(
                "[option] {LIVE_SECTION} is not an object ({other}); the constructor values stand"
            ),
        }
        LiveData(data)
    }

    fn f32(&self, key: &str) -> f32 {
        self.0[key]
            .as_f64()
            .unwrap_or_else(|| panic!("{LIVE_SECTION}.{key} is not a number")) as f32
    }

    fn int(&self, key: &str) -> i64 {
        let value = &self.0[key];
        value
            .as_i64()
            .or_else(|| value.as_f64().map(|v| v as i64))
            .unwrap_or_else(|| panic!("{LIVE_SECTION}.{key} is not an integer"))
    }

    fn bool(&self, key: &str) -> bool {
        self.0[key]
            .as_bool()
            .unwrap_or_else(|| panic!("{LIVE_SECTION}.{key} is not a bool"))
    }

    fn set(&mut self, key: &str, value: Value) {
        self.0.insert(key.to_owned(), value);
    }

    /// `GetNoteAlpha`: 0 reads as 1.
    fn note_alpha(&self) -> f32 {
        let alpha = self.f32("NoteAlpha");
        if alpha == 0.0 {
            1.0
        } else {
            alpha
        }
    }

    /// `GetGuideAlpha`: 0 reads as 0.6.
    fn guide_alpha(&self) -> f32 {
        let alpha = self.f32("GuideAlpha");
        if alpha == 0.0 {
            0.6
        } else {
            alpha
        }
    }
}

/// The `ApplicationLocalSettings` members the System and Communication pages
/// read and write, with the constructor's values (read natively: the
/// reconstructed constructor drops the stores of BatteryAlert,
/// NotifyLiveBonusMax and ShowNoteSpeedDialog, all true). The source object
/// is the session's cached one, so this resource is kept for the session.
#[derive(Resource, Debug, Clone, PartialEq)]
pub(crate) struct AppLocalOptions {
    default_music_ver: i64,
    screen_fixed: bool,
    show_note_speed_dialog: bool,
    notify_live_bonus_max: bool,
    hide_story_favorite_comment: bool,
    is_additional_voice: bool,
}

impl Default for AppLocalOptions {
    fn default() -> Self {
        AppLocalOptions {
            default_music_ver: 0,
            screen_fixed: false,
            show_note_speed_dialog: true,
            notify_live_bonus_max: true,
            hide_story_favorite_comment: false,
            is_additional_voice: false,
        }
    }
}

impl AppLocalOptions {
    fn load(document: &Value) -> Self {
        let mut options = AppLocalOptions::default();
        let flag = |key: &str, slot: &mut bool| {
            match &document[key] {
            Value::Null => {}
            Value::Bool(value) => *slot = *value,
            other => warn!("[option] ApplicationLocalSettings.{key}={other} is not a bool; the constructor value stays"),
        }
        };
        flag("ScreenFixed", &mut options.screen_fixed);
        flag("ShowNoteSpeedDialog", &mut options.show_note_speed_dialog);
        flag("NotifyLiveBonusMax", &mut options.notify_live_bonus_max);
        flag(
            "HideStoryFavoriteComment",
            &mut options.hide_story_favorite_comment,
        );
        flag("IsAdditionalVoice", &mut options.is_additional_voice);
        match &document["DefaultMusicVer"] {
            Value::Null => {}
            value => match value.as_i64() {
                Some(v) => options.default_music_ver = v,
                None => warn!("[option] ApplicationLocalSettings.DefaultMusicVer={value} is not an int; the constructor value stays"),
            },
        }
        options
    }

    /// The members the dialog writes, as root keys of the local settings
    /// document (beside `LiveVolume` and `SystemVolume`).
    fn sections(&self) -> Vec<(&'static str, Value)> {
        vec![
            ("DefaultMusicVer", json!(self.default_music_ver)),
            ("ScreenFixed", json!(self.screen_fixed)),
            ("ShowNoteSpeedDialog", json!(self.show_note_speed_dialog)),
            ("NotifyLiveBonusMax", json!(self.notify_live_bonus_max)),
            (
                "HideStoryFavoriteComment",
                json!(self.hide_story_favorite_comment),
            ),
        ]
    }
}

/// `UserDataManager.UserConfig`: server state. None is the null user config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UserConfig {
    display_login_status: bool,
    /// `FriendRequestStatus`: all 0, id_search 1, reject 2.
    friend_request_status: usize,
}

fn user_config_mock() -> Option<UserConfig> {
    const NAME: &str = "MOLY_OPTION_MOCK_USER_CONFIG";
    let raw = std::env::var(NAME).ok()?;
    let parsed = raw.split_once(',').and_then(|(login, friend)| {
        let display_login_status = match login.trim() {
            "true" => true,
            "false" => false,
            _ => return None,
        };
        let friend_request_status = ["all", "id_search", "reject"]
            .iter()
            .position(|name| *name == friend.trim())?;
        Some(UserConfig {
            display_login_status,
            friend_request_status,
        })
    });
    if parsed.is_none() {
        warn!("[option] mock panel: {NAME}={raw:?} is not <true|false>,<all|id_search|reject>; the user config stays null");
    }
    parsed
}

// ---------------------------------------------------------------------------
// Page state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LivePage {
    selectors: Vec<Selector>,
    toggles: [Option<usize>; 12],
    note_skin_index: i64,
    note_se_index: i64,
}

#[derive(Debug, Clone)]
struct SystemPage {
    toggles: [Option<usize>; 5],
}

#[derive(Debug, Clone)]
struct CommunicationPage {
    friend: Option<usize>,
    favorite: Option<usize>,
    friend_enabled: bool,
    block_list_enabled: bool,
}

/// The option dialog's state for one open.
#[derive(Resource, Default)]
pub(crate) struct OptionDialogState {
    /// The six volume sliders' integer draft (0..=100).
    draft: [u8; 6],
    /// The delayed preview in flight (b__0's reschedule): (due time, slider).
    pending: Option<(f32, VolumeSlider)>,
    /// The slider being dragged.
    dragging: Option<VolumeSlider>,
    /// `_currentPage`.
    current: Option<Page>,
    live_data: Option<LiveData>,
    live: Option<LivePage>,
    volume_set_up: bool,
    system: Option<SystemPage>,
    communication: Option<CommunicationPage>,
    user_config: Option<UserConfig>,
    /// Content offsets of the page scroll views, by page index.
    scroll: [f32; 5],
    /// A drag moving a page's content: (page, start y, start offset).
    scrolling: Option<(Page, f32, f32)>,
    /// The screen manager's handle of this dialog.
    dialog_id: Option<DialogId>,
}

// ---------------------------------------------------------------------------
// Page Setup / UpdateLocalData (OptionDialog.Activate/DeactivateCurrentPage)
// ---------------------------------------------------------------------------

/// `OptionLiveSetting.Setup` with `OptionNoteSetting.Setup`. The vibration
/// root is hidden and its group never set up, so its state carries over.
fn live_setup(data: &mut LiveData, previous: Option<&LivePage>) -> LivePage {
    let mut toggles = [None; 12];
    let set = |toggles: &mut [Option<usize>; 12], which: LiveToggle, value: Option<usize>| {
        let index = LIVE_TOGGLES
            .iter()
            .position(|(t, _)| *t == which)
            .expect("live toggle");
        toggles[index] = value;
    };
    set(
        &mut toggles,
        LiveToggle::CutIn,
        from_bool(data.bool("UseCutIn")),
    );
    set(
        &mut toggles,
        LiveToggle::NoteEffect,
        from_bool(data.int("NoteEffect") == 0),
    );
    set(
        &mut toggles,
        LiveToggle::FeverEffect,
        from_bool(data.int("FeverEffectTypeIndex") == 0),
    );
    set(
        &mut toggles,
        LiveToggle::SkillAndPraise,
        from_bool(!data.bool("HiddenSkillAndPraise")),
    );
    set(
        &mut toggles,
        LiveToggle::SimultaneousPushingLine,
        from_bool(data.bool("UseSimultaneousPushingLine")),
    );
    let vibration = previous.map_or(None, |p| p.toggles[LiveToggle::Vibration as usize]);
    set(&mut toggles, LiveToggle::Vibration, vibration);
    set(
        &mut toggles,
        LiveToggle::AllPerfectEffect,
        from_bool(data.bool("UseAllPerfectEffect")),
    );
    set(
        &mut toggles,
        LiveToggle::FastLateFlick,
        from_bool(data.bool("IsFastLateFlick")),
    );
    // The live mode switch (native jump table): High3D and Default3D 0,
    // Mode2D 1, OriginalMV 2, Low 3; another value leaves the group.
    let mode = match data.int("LiveMode") {
        0 | 1 => Some(0),
        2 => Some(1),
        3 => Some(3),
        4 => Some(2),
        _ => None,
    };
    set(&mut toggles, LiveToggle::Mode, mode);
    set(
        &mut toggles,
        LiveToggle::Quality,
        Some(if data.int("QualityType") == 0 { 1 } else { 0 }),
    );
    set(
        &mut toggles,
        LiveToggle::Mirror,
        from_bool(data.bool("IsMirror")),
    );
    set(
        &mut toggles,
        LiveToggle::Use120Fps,
        from_bool(data.bool("Use120FPS")),
    );
    let selectors = vec![
        Selector::new(
            decimal_hundredths(data.f32("NoteSpeed")),
            1,
            12,
            Steps::PerButton(&VARIATION_VALUES_1),
            "",
            true,
        ),
        Selector::new(
            decimal_hundredths(data.f32("TimingAdjustData")),
            -20,
            20,
            Steps::PerButton(&VARIATION_VALUES_1),
            "",
            true,
        ),
        Selector::new(
            percent(data.f32("Brightness")),
            50,
            100,
            Steps::Every(1000),
            "%",
            false,
        ),
        Selector::new(
            percent(data.f32("LaneTransparent")),
            0,
            100,
            Steps::Every(1000),
            "%",
            false,
        ),
        Selector::new(
            percent(data.note_alpha()),
            10,
            100,
            Steps::Every(500),
            "%",
            false,
        ),
        Selector::new(
            percent(data.guide_alpha()),
            10,
            100,
            Steps::Every(500),
            "%",
            false,
        ),
    ];
    let page = LivePage {
        selectors,
        toggles,
        note_skin_index: data.int("NoteSkinIndex"),
        note_se_index: data.int("NoteSeIndex"),
    };
    // The alpha selectors' update action runs inside Setup's UpdateValue.
    alpha_update(data, &page, 4);
    alpha_update(data, &page, 5);
    page
}

/// `UpdateSLongNoteAlphaSetting` / `UpdateGuideAlphaSetting`: the alpha
/// selectors write the data object on every change.
fn alpha_update(data: &mut LiveData, page: &LivePage, selector: usize) {
    let key = if selector == 4 {
        "NoteAlpha"
    } else {
        "GuideAlpha"
    };
    data.set(key, json!(page.selectors[selector].value() / 100.0));
}

/// `OptionLiveSetting.UpdateLocalData`.
fn live_update_local(data: &mut LiveData, page: &LivePage) {
    let s = &page.selectors;
    let at = |which: LiveToggle| selected(page.toggles[which as usize]);
    data.set("NoteSpeed", json!(s[0].value()));
    data.set("TimingAdjustData", json!(s[1].value()));
    data.set("Brightness", json!(s[2].value() / 100.0));
    data.set("LaneTransparent", json!(s[3].value() / 100.0));
    data.set("UseCutIn", json!(at(LiveToggle::CutIn) == 0));
    data.set(
        "NoteEffect",
        json!(i64::from(at(LiveToggle::NoteEffect) != 0)),
    );
    data.set(
        "FeverEffectTypeIndex",
        json!(i64::from(at(LiveToggle::FeverEffect) != 0)),
    );
    data.set(
        "HiddenSkillAndPraise",
        json!(at(LiveToggle::SkillAndPraise) != 0),
    );
    data.set(
        "UseSimultaneousPushingLine",
        json!(at(LiveToggle::SimultaneousPushingLine) == 0),
    );
    data.set("UseVibration", json!(at(LiveToggle::Vibration) == 0));
    data.set(
        "UseAllPerfectEffect",
        json!(at(LiveToggle::AllPerfectEffect) == 0),
    );
    data.set("IsFastLateFlick", json!(at(LiveToggle::FastLateFlick) == 0));
    // MVQualityType High 1, Default 0.
    let quality = if at(LiveToggle::Quality) == 0 { 1 } else { 0 };
    data.set("QualityType", json!(quality));
    // GetObjectLiveMode.
    let mode = match at(LiveToggle::Mode) {
        1 => 2,
        2 => 4,
        0 => {
            if quality == 1 {
                0
            } else {
                1
            }
        }
        _ => 3,
    };
    data.set("LiveMode", json!(mode));
    data.set("IsMirror", json!(at(LiveToggle::Mirror) == 0));
    data.set("Use120FPS", json!(at(LiveToggle::Use120Fps) == 0));
    data.set("NoteSkinIndex", json!(page.note_skin_index));
    data.set("NoteSeIndex", json!(page.note_se_index));
}

/// `OptionSystemSetting.Setup`. `InitializeSelectIndex` of an index outside
/// the group logs and changes nothing.
fn system_setup(
    app: &AppLocalOptions,
    user: Option<UserConfig>,
    previous: Option<&SystemPage>,
    groups: usize,
) -> SystemPage {
    let mut toggles = previous.map_or([None; 5], |p| p.toggles);
    match usize::try_from(app.default_music_ver)
        .ok()
        .filter(|v| *v < groups)
    {
        Some(index) => toggles[0] = Some(index),
        None => error!(
            "[option] DefaultMusicVer {}: unknown toggle index; the group is unchanged",
            app.default_music_ver
        ),
    }
    toggles[SCREEN_FIXED] = from_bool(app.screen_fixed);
    // OnSelectedScreenFixed runs from InitializeSelectIndex.
    info!(
        "[option] System: DisplayUtility.ActivateFixedScreenOrientation({}) has no host counterpart",
        app.screen_fixed
    );
    toggles[2] = from_bool(app.show_note_speed_dialog);
    toggles[4] = from_bool(app.notify_live_bonus_max);
    // A null user config reads as false.
    toggles[SHOW_LOGIN_STATUS] = from_bool(user.is_some_and(|u| u.display_login_status));
    SystemPage { toggles }
}

/// `OptionSystemSetting.UpdateLoacalData`.
fn system_update_local(app: &mut AppLocalOptions, page: &SystemPage) {
    app.default_music_ver = selected(page.toggles[0]) as i64;
    app.screen_fixed = selected(page.toggles[SCREEN_FIXED]) == 0;
    app.show_note_speed_dialog = selected(page.toggles[2]) == 0;
    app.notify_live_bonus_max = selected(page.toggles[4]) == 0;
}

/// `OptionCommunicationSetting.Setup`: with a user config both groups take
/// the stored values; without one both stay as `Awake` left them.
fn communication_setup(
    app: &AppLocalOptions,
    user: Option<UserConfig>,
    enabled_block_list: bool,
) -> CommunicationPage {
    let (friend, favorite) = match user {
        Some(user) => (
            Some(user.friend_request_status),
            Some(usize::from(app.hide_story_favorite_comment)),
        ),
        None => (None, None),
    };
    CommunicationPage {
        friend,
        favorite,
        friend_enabled: enabled_block_list,
        block_list_enabled: enabled_block_list,
    }
}

/// `OptionDialog.Save`'s report of the two server values, when the user config
/// exists and one of them changed (`UserInformationUtility.UpdateUserConfig`).
fn update_server_data(state: &OptionDialogState) {
    let Some(user) = state.user_config else {
        info!("[option] UpdateServerData: the user config is null (server state not in this product); nothing is sent");
        return;
    };
    let login = state
        .system
        .as_ref()
        .map_or(user.display_login_status, |p| {
            selected(p.toggles[SHOW_LOGIN_STATUS]) == 0
        });
    let friend = state
        .communication
        .as_ref()
        .map_or(user.friend_request_status, |p| selected(p.friend));
    if login == user.display_login_status && friend == user.friend_request_status {
        return;
    }
    info!("[option] UpdateServerData: UpdateUserConfig(isDisplayLoginStatus {login}, friendRequestStatus {friend}) goes to the server model");
}

impl OptionDialogState {
    /// `DeactivateCurrentPage`.
    fn deactivate(&mut self, settings: &mut LocalVolumeSettings, app: &mut AppLocalOptions) {
        match self.current {
            Some(Page::Live) => {
                if let (Some(data), Some(page)) = (self.live_data.as_mut(), self.live.as_ref()) {
                    live_update_local(data, page);
                }
            }
            Some(Page::Volume) => volume_update_local(settings, &self.draft),
            Some(Page::System) => {
                if let Some(page) = &self.system {
                    system_update_local(app, page);
                }
            }
            _ => {}
        }
    }

    /// `ActivateCurrentPage`: the page's Setup from the data objects; the
    /// scroll content goes back to the top.
    fn activate(
        &mut self,
        page: Page,
        settings: &LocalVolumeSettings,
        app: &AppLocalOptions,
        bindings: &OptionBindings,
    ) {
        self.scroll[page.index()] = 0.0;
        match page {
            Page::Live => {
                let data = self.live_data.as_mut().expect("live data loaded at open");
                self.live = Some(live_setup(data, self.live.as_ref()));
            }
            Page::Volume => {
                for which in SLIDERS {
                    self.draft[which.index()] = which.initial(settings);
                    info!(
                        "[option]   Volume {} {}: {} ({}, preview cue {})",
                        which.group().header(),
                        which.kind_label(),
                        self.draft[which.index()],
                        which.preview_kind().label(),
                        which.cue().unwrap_or("none"),
                    );
                }
                self.volume_set_up = true;
            }
            Page::System => {
                let groups = bindings
                    .system
                    .as_ref()
                    .map_or(0, |b| b.groups[0].toggles.len());
                self.system = Some(system_setup(
                    app,
                    self.user_config,
                    self.system.as_ref(),
                    groups,
                ));
            }
            Page::Communication => {
                self.communication = Some(communication_setup(
                    app,
                    self.user_config,
                    MENU_SETUP.can_block_list_setting,
                ));
            }
            Page::CustomScore => {}
        }
        info!("[option] ActivateCurrentPage: {page:?} Setup");
    }

    /// `OnSelectedTab`.
    fn select_tab(
        &mut self,
        page: Page,
        settings: &mut LocalVolumeSettings,
        app: &mut AppLocalOptions,
        bindings: &OptionBindings,
    ) {
        if self.current == Some(page) {
            return;
        }
        self.deactivate(settings, app);
        self.current = Some(page);
        self.activate(page, settings, app, bindings);
    }
}

/// `OptionVolumeSetting.UpdateLoacalData`: the draft into the session object.
fn volume_update_local(settings: &mut LocalVolumeSettings, draft: &[u8; 6]) {
    settings.live = draft_group(draft, Group::Live);
    settings.system = draft_group(draft, Group::System);
}

/// `OptionDialog.Save`: every set-up page's UpdateLocalData (each page's
/// method returns while its Setup has not run; read natively), UpdateServerData,
/// then both storage objects: the volumes through the audio module's own
/// write, the other members and `LiveSettingData` in one more write of the
/// local settings document.
fn save(
    state: &mut OptionDialogState,
    settings: &mut LocalVolumeSettings,
    app: &mut AppLocalOptions,
    cause: &str,
) {
    if let (Some(data), Some(page)) = (state.live_data.as_mut(), state.live.as_ref()) {
        live_update_local(data, page);
    }
    if state.volume_set_up {
        volume_update_local(settings, &state.draft);
    }
    if let Some(page) = &state.system {
        system_update_local(app, page);
    }
    if let Some(page) = &state.communication {
        app.hide_story_favorite_comment = selected(page.favorite) != 0;
    }
    update_server_data(state);
    crate::audio::save_volume_settings(settings);
    let mut sections: Vec<(&str, Value)> = app.sections();
    if let Some(data) = &state.live_data {
        sections.push((LIVE_SECTION, Value::Object(data.0.clone())));
    }
    match crate::settings_store::save_sections(&sections) {
        Ok(()) => {
            let readback = crate::settings_store::read_document()
                .map(|document| sections.iter().all(|(key, value)| &document[*key] == value));
            match readback {
                Ok(true) => info!(
                    "[option] {cause}: Save wrote {} sections (ApplicationLocalSettings members, {LIVE_SECTION}) to {}; read back equal",
                    sections.len(),
                    crate::settings_store::location()
                ),
                Ok(false) => warn!("[option] {cause}: Save read back different values"),
                Err(error) => warn!("[option] {cause}: Save wrote, the read back failed: {error}"),
            }
        }
        Err(error) => {
            warn!("[option] {cause}: Save failed: {error}; the session keeps the new values")
        }
    }
}

// ---------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------

/// `CustomButton` / `CustomToggle` enabled state: `OnEnable` hides the covers,
/// `OnDisable` shows them when the selectable greys out.
fn paint_selectable(
    view: &mut crate::ui_layout::UiPrefabView,
    selectable: &SelectableBinding,
    enabled: bool,
) {
    for cover in &selectable.covers {
        view.set_visible(cover, selectable.grayout && !enabled);
    }
}

/// A toggle group: the check mark of the one toggle on shows (`Toggle`'s
/// graphic alpha 1 or 0; its 0.1 s `CrossFadeAlpha` is not ported).
fn paint_group(
    view: &mut crate::ui_layout::UiPrefabView,
    group: &GroupBinding,
    state: Option<usize>,
    enabled: bool,
) {
    for (index, toggle) in group.toggles.iter().enumerate() {
        if let Some(graphic) = &toggle.graphic {
            view.set_alpha(graphic, if state == Some(index) { 1.0 } else { 0.0 });
        }
        paint_selectable(view, &toggle.selectable, enabled);
    }
}

/// `float.ToString()` of a variation value (hundredths): 1, 0.1, 10, 5.
fn variation_text(hundredths: i64) -> String {
    let (whole, cents) = (hundredths / 100, hundredths % 100);
    if cents == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{}", format!("{cents:02}").trim_end_matches('0'))
    }
}

fn paint_live(
    view: &mut crate::ui_layout::UiPrefabView,
    layouts: &crate::ui_layout::UiLayouts,
    binding: &LiveBinding,
    page: &LivePage,
) {
    for (selector, bound) in page.selectors.iter().zip(&binding.selectors) {
        view.set_text(&bound.text, selector.text());
        for button in &bound.decrement {
            paint_selectable(view, button, selector.can_decrement());
        }
        for button in &bound.increment {
            paint_selectable(view, button, selector.can_increment());
        }
        // SetupButton: WORD_ADD_FORMAT / WORD_SUBTRACT_FORMAT with the step.
        for (texts, key) in [
            (&bound.increment_texts, "WORD_ADD_FORMAT"),
            (&bound.decrement_texts, "WORD_SUBTRACT_FORMAT"),
        ] {
            for (index, text) in texts.iter().enumerate() {
                let Some(step) = selector.step(index) else {
                    continue;
                };
                let args = [variation_text(step)];
                if let Some(value) = layouts.set_wording_text(KEY, text, key, Some(&args[..])) {
                    view.set_text(text, value);
                }
            }
        }
    }
    for (group, state) in binding.groups.iter().zip(page.toggles) {
        paint_group(view, group, state, true);
    }
    // SetupVibration hides the vibration root.
    view.set_visible(&binding.vibration_root, false);
}

/// `OptionSystemSetting.Setup`'s five buttons: (enabled, wording key written).
/// Live rank matching is session state of the live game, off here; this
/// product carries no MV or music bundles, so both pending download counts
/// are 0.
fn system_buttons(app: &AppLocalOptions) -> [(bool, Option<&'static str>); 5] {
    let can_asset = MENU_SETUP.can_asset_setting;
    let rank_matching = false;
    let voice = if app.is_additional_voice {
        "WORD_DOWNLOADED"
    } else {
        "WORD_BULK_DOWNLOAD"
    };
    let pending_downloads = 0;
    let bulk = if pending_downloads == 0 {
        (false, Some("WORD_DOWNLOADED"))
    } else {
        (true, None)
    };
    [
        (
            can_asset && !rank_matching && !app.is_additional_voice,
            Some(voice),
        ),
        bulk,
        bulk,
        (can_asset, None),
        (can_asset, None),
    ]
}

fn paint_system(
    view: &mut crate::ui_layout::UiPrefabView,
    layouts: &crate::ui_layout::UiLayouts,
    binding: &SystemBinding,
    page: &SystemPage,
    app: &AppLocalOptions,
) {
    for (group, state) in binding.groups.iter().zip(page.toggles) {
        paint_group(view, group, state, true);
    }
    for (button, (enabled, wording)) in binding.buttons.iter().zip(system_buttons(app)) {
        paint_selectable(view, &button.button, enabled);
        if let Some(key) = wording {
            // UIPartsCommonButton.SetWordingKey turns the key on first.
            let text = layouts
                .wordings
                .get(key)
                .unwrap_or_else(|| panic!("UI wording missing: {key}"));
            view.set_text(&button.text, text.clone());
        }
    }
}

fn paint_communication(
    view: &mut crate::ui_layout::UiPrefabView,
    binding: &CommunicationBinding,
    page: &CommunicationPage,
) {
    paint_group(view, &binding.friend, page.friend, page.friend_enabled);
    paint_group(view, &binding.favorite, page.favorite, true);
    paint_selectable(view, &binding.block_list, page.block_list_enabled);
}

/// `UpdateTabLines` (read natively): over the tabs whose GameObject is
/// active, the line hides on the selected one (its `UIPartsDialogTab.isOn`),
/// on the one before it and on the last; the others show it.
fn tab_lines(active: &[usize], selected: Option<usize>) -> Vec<(usize, bool)> {
    let selected = selected
        .and_then(|tab| active.iter().position(|a| *a == tab))
        .map_or(-1, |k| k as i64);
    active
        .iter()
        .enumerate()
        .map(|(k, tab)| {
            let k = k as i64;
            let hidden = k == selected || k == selected - 1 || k == active.len() as i64 - 1;
            (*tab, !hidden)
        })
        .collect()
}

/// `Setup`'s `DisableToggle(0)` / `EnableToggle(4)`: whether a tab's
/// GameObject is active.
fn tab_shown(page: Page) -> bool {
    match page {
        Page::CustomScore => MENU_SETUP.show_custom_score_tab,
        Page::Communication => MENU_SETUP.show_communication_tab,
        _ => true,
    }
}

fn paint(
    view: &mut crate::ui_layout::UiPrefabView,
    layouts: &crate::ui_layout::UiLayouts,
    doc: &UiPrefab,
    bindings: &OptionBindings,
    state: &OptionDialogState,
    app: &AppLocalOptions,
) {
    let active: Vec<usize> = (0..bindings.tabs.len())
        .filter(|i| tab_shown(bindings.tabs[*i].page))
        .collect();
    let selected = bindings
        .tabs
        .iter()
        .position(|tab| Some(tab.page) == state.current);
    for (index, tab) in bindings.tabs.iter().enumerate() {
        view.set_visible(&tab.selectable.control, tab_shown(tab.page));
        // UIPartsDialogTab.On / Off: backgroundImage.enabled (the image is
        // its node's only graphic).
        view.set_visible(&tab.background, selected == Some(index));
        // A page this layout cannot build shows its tab as a disabled toggle.
        paint_selectable(view, &tab.selectable, bindings.built(tab.page));
    }
    for (tab, shown) in tab_lines(&active, selected) {
        if let Some(line) = &bindings.tabs[tab].line {
            view.set_visible(line, shown);
        }
    }
    for page in PAGES {
        if let Some(node) = &bindings.pages[page.index()] {
            view.set_visible(node, state.current == Some(page));
        }
        if let Some(scroll) = &bindings.scrolls[page.index()] {
            let base = doc.nodes[doc.find(&scroll.content).expect("scroll content node")]
                .rect
                .anchored_position;
            view.set_anchored_position(
                &scroll.content,
                Vec2::new(base[0], base[1] + state.scroll[page.index()]),
            );
        }
    }
    match state.current {
        Some(Page::Live) => {
            if let (Ok(binding), Some(page)) = (&bindings.live, &state.live) {
                paint_live(view, layouts, binding, page);
            }
        }
        Some(Page::Volume) => {
            for which in SLIDERS {
                let path = slider_root(which);
                let value = state.draft[which.index()];
                view.set_text(&format!("{path}/NumText"), value.to_string());
                view.set_slider(&format!("{path}/UIPartsSlider"), value as f32 / 100.0);
                view.set_visible(&format!("{path}/UIPartsDecrementButton/Cover"), value == 0);
                view.set_visible(
                    &format!("{path}/UIPartsIncrementButton/Cover"),
                    value == 100,
                );
            }
        }
        Some(Page::System) => {
            if let (Ok(binding), Some(page)) = (&bindings.system, &state.system) {
                paint_system(view, layouts, binding, page, app);
            }
        }
        Some(Page::Communication) => {
            if let (Ok(binding), Some(page)) = (&bindings.communication, &state.communication) {
                paint_communication(view, binding, page);
            }
        }
        Some(Page::CustomScore) | None => {}
    }
}

// ---------------------------------------------------------------------------
// Entities and systems
// ---------------------------------------------------------------------------

/// The dialog root (Dialog slot; open = the shell dialog state's option bit).
#[derive(Component)]
pub(crate) struct OptionDialogRoot;

pub(crate) fn init(mut commands: Commands) {
    commands.init_resource::<OptionDialogState>();
    let app = match crate::settings_store::read_document() {
        Ok(document) => AppLocalOptions::load(&document),
        Err(error) => {
            warn!("[option] the local settings document is unreadable ({error}); ApplicationLocalSettings starts from its constructor");
            AppLocalOptions::default()
        }
    };
    info!("[option] ApplicationLocalSettings members of the dialog: {app:?}");
    commands.insert_resource(app);
}

/// Resolve the bindings and build the source dialog once; the product
/// settings panel owns its entry.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<crate::ui_layout::UiLayouts>,
    server: Res<AssetServer>,
    bindings: Option<Res<OptionBindings>>,
) {
    if bindings.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    let doc = layouts.document(KEY).expect("ready layout");
    // The wordings the pages write resolve here or the write refuses later.
    let missing: Vec<&str> = WORDINGS
        .iter()
        .copied()
        .filter(|key| !layouts.wordings.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        warn!("[option] wordings missing from this root: {missing:?}; a page writing one of them refuses");
    }
    commands.insert_resource(OptionBindings::from_prefab(doc));
    commands.spawn((
        OptionDialogRoot,
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER),
        crate::ui_layout::UiPrefabView::new(KEY, SITEMAP_LAYER),
    ));
}

/// The open edge: `LiveSettingData.LoadFromStorage`, the user config, then
/// `Setup(MENU_SETUP)` and `InitializeSelectIndex(opening)`.
fn open(
    state: &mut OptionDialogState,
    settings: &LocalVolumeSettings,
    app: &AppLocalOptions,
    bindings: &OptionBindings,
) {
    let document = crate::settings_store::read_document().unwrap_or_else(|error| {
        warn!("[option] the local settings document is unreadable ({error}); {LIVE_SECTION} starts from its constructor");
        Value::Null
    });
    *state = OptionDialogState {
        live_data: Some(LiveData::load(&document)),
        user_config: user_config_mock(),
        ..OptionDialogState::default()
    };
    let opening = PAGES[MENU_SETUP.opening()];
    let first = if bindings.built(opening) {
        opening
    } else {
        let built = bindings
            .tabs
            .iter()
            .map(|tab| tab.page)
            .find(|page| tab_shown(*page) && bindings.built(*page))
            .expect("the Volume page is built");
        warn!("[option] Setup opens the {opening:?} page, which this layout cannot build; the dialog opens {built:?}");
        built
    };
    info!(
        "[option] open: Setup(tabIndex {}, canAssetSetting {}, canBlockListSetting {}, showCustomScoreTab {}, showCommunicationTab {}) -> {first:?}",
        MENU_SETUP.tab_index, MENU_SETUP.can_asset_setting, MENU_SETUP.can_block_list_setting,
        MENU_SETUP.show_custom_score_tab, MENU_SETUP.show_communication_tab,
    );
    // InitializeSelectIndex fires OnSelectedTab; no page was current.
    state.current = Some(first);
    state.activate(first, settings, app, bindings);
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn place(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    dialog: Res<ShellDialogState>,
    settings: Res<LocalVolumeSettings>,
    app: Res<AppLocalOptions>,
    mut state: ResMut<OptionDialogState>,
    mut voice: ResMut<VoiceChannel>,
    mut se_requests: ResMut<SeRequests>,
    mut roots: Query<
        (
            &mut Visibility,
            &mut Transform,
            &mut crate::ui_layout::UiPrefabView,
        ),
        With<OptionDialogRoot>,
    >,
    mut was_open: Local<bool>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    layouts: Res<crate::ui_layout::UiLayouts>,
    bindings: Option<Res<OptionBindings>>,
    mut stack: ResMut<UiLayerStack>,
) {
    let Some(bindings) = bindings else {
        return;
    };
    let open_now = dialog.option_open;
    match (*was_open, open_now) {
        (false, true) => {
            open(&mut state, &settings, &app, &bindings);
            // Shown without an open animation here. The dialog's back key is
            // Common1ButtonDialog's OnClickOK (Save, then close).
            match stack.show_dialog(OPTION_DIALOG, DisplayLayerType::LayerDialog, DialogBackKey::Close, "settings panel (OptionDialog.Setup)") {
                Ok(id) => {
                    stack.open_dialog(id);
                    stack.dialog_open_finished(id);
                    state.dialog_id = Some(id);
                }
                Err(error) => warn!("[option] {error}; the dialog shows outside the screen manager and the back key does not reach it"),
            }
        }
        (true, false) => {
            state.dragging = None;
            state.scrolling = None;
            state.current = None;
            if let Some(id) = state.dialog_id.take() {
                stack.close_dialog(id);
                stack.dialog_destroyed(id);
            }
        }
        _ => {}
    }
    *was_open = open_now;

    // The delayed preview (b__1 after 0.15 s).
    if let Some((at, which)) = state.pending {
        if time.elapsed_secs() >= at {
            state.pending = None;
            fire_preview(
                &mut commands,
                which,
                state.draft[which.index()],
                &mut voice,
                &mut se_requests,
            );
        }
    }

    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let Some(doc) = layouts.document(KEY) else {
        return;
    };
    let scale = root_canvas.scale(window);
    for (mut visible, mut transform, mut view) in &mut roots {
        *visible = if open_now {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        transform.scale = Vec3::splat(scale);
        if open_now {
            paint(&mut view, &layouts, doc, &bindings, &state, &app);
        }
    }
}
/// The delayed preview's call (b__1): an empty cue (the two BGM sliders)
/// returns; otherwise `StopVoiceAll`, then the slider's sound type through
/// `SoundManager`. The four cues are in the common menu bank.
fn fire_preview(
    commands: &mut Commands,
    which: VolumeSlider,
    value: u8,
    voice: &mut VoiceChannel,
    se_requests: &mut SeRequests,
) {
    let Some(cue) = which.cue() else {
        return;
    };
    stop_voice_all(voice, commands);
    let who = format!("{} {}", which.group().header(), which.kind_label());
    let volume = value as f32 * 0.01;
    match which.preview_kind() {
        PreviewKind::Se => {
            // Type 1, PlaySEOneShot: the UI SE player.
            se_requests.0.push(SeRequest {
                owner: None,
                cue: cue.into(),
                class: SeClass::Ui,
                source: "option_preview",
            });
            info!("[option] preview (0.15 s, b__1): {who} -> type 1 PlaySEOneShot({cue})");
        }
        PreviewKind::Voice => {
            info!("[option] preview (0.15 s, b__1): {who} -> type 2 PlayVoice({cue}, 1): the audio channel has no volume preview player; not played");
        }
        PreviewKind::IngameSe => {
            info!("[option] preview (0.15 s, b__1): {who} -> type 3 SamplePlaySE({cue}, {volume:.2}): the audio channel has no volume preview player; not played");
        }
        PreviewKind::IngameVoice => {
            info!("[option] preview (0.15 s, b__1): {who} -> type 4 PlayVoiceFixedVolume({cue}, {volume:.2}): the audio channel has no volume preview player; not played");
        }
        PreviewKind::Bgm => unreachable!("a slider with a cue is not type 0"),
    }
}

// ---------------------------------------------------------------------------
// Update：点按与拖动（模态先手）
// ---------------------------------------------------------------------------

/// 点按屏位（顶为原点）→ 参照画布坐标。
fn to_canvas(position: Vec2, width: f32, height: f32, scale: f32) -> Vec2 {
    Vec2::new(position.x - width / 2.0, height / 2.0 - position.y) / scale
}

fn slider_root(which: VolumeSlider) -> String {
    let group = if which.index() < 3 {
        "LiveVolume"
    } else {
        "SystemVolume"
    };
    let suffix = match which.index() % 3 {
        0 => "",
        1 => " (1)",
        _ => " (2)",
    };
    format!("Volume/ScorollView/Viewport/Content/{group}/UIPartsSliderLabelContent{suffix}/UIPartsSelectCost")
}
fn source_hit(
    canvas: Vec2,
    path: &str,
    layouts: &crate::ui_layout::UiLayouts,
    view: &crate::ui_layout::UiPrefabView,
    size: Vec2,
) -> bool {
    view.rect(layouts, path, size)
        .is_some_and(|r| r.active && r.contains(canvas))
}
fn track_slider_at(
    canvas: Vec2,
    layouts: &crate::ui_layout::UiLayouts,
    view: &crate::ui_layout::UiPrefabView,
    size: Vec2,
) -> Option<VolumeSlider> {
    SLIDERS.iter().copied().find(|which| {
        source_hit(
            canvas,
            &format!("{}/UIPartsSlider", slider_root(*which)),
            layouts,
            view,
            size,
        )
    })
}
fn value_from_x(
    which: VolumeSlider,
    x: f32,
    layouts: &crate::ui_layout::UiLayouts,
    view: &crate::ui_layout::UiPrefabView,
    size: Vec2,
) -> u8 {
    let Some(rect) = view.rect(
        layouts,
        &format!("{}/UIPartsSlider/HandleSlideArea", slider_root(which)),
        size,
    ) else {
        return 0;
    };
    let t = (x - (rect.center().x - rect.size.x * 0.5)) / rect.size.x;
    (t * 100.0).round().clamp(0.0, 100.0) as u8
}

/// 滑杆变更的整链（b__0）：写草稿 → 重排 0.15s 延时预览 → UpdateVolume
/// 立即（只读系统三滑杆——Live 杆也照走这条，总线值不变即无施加行）。
fn set_slider(
    state: &mut OptionDialogState,
    bus: &mut VolumeBus,
    now: f32,
    which: VolumeSlider,
    value: u8,
    cause: &str,
) {
    let old = state.draft[which.index()];
    if old == value {
        return; // 整数控件同值不回调（Unity Slider 同值不派发）
    }
    state.draft[which.index()] = value;
    state.pending = Some((now + PREVIEW_DELAY_SECONDS, which));
    let system = draft_group(&state.draft, Group::System);
    apply_system_volume(
        bus,
        &system,
        "选项页 UpdateVolume（b__0：重排延时预览后立即施加，只读系统三滑杆）",
    );
    if which.group() == Group::Live {
        info!(
            "[option] {}·{} 滑杆：{old} → {value}（{cause}）——LiveVolume 不进施加（UpdateVolume 只读\
             系统三滑杆；档值在保存关闭时随 Live 组落盘）",
            which.group().header(),
            which.kind_label()
        );
    } else {
        info!(
            "[option] {}·{} 滑杆：{old} → {value}（{cause}）",
            which.group().header(),
            which.kind_label()
        );
    }
}

// ---------------------------------------------------------------------------
// Taps and drags (modal)
// ---------------------------------------------------------------------------

/// What a tap on the dialog asks the host to do.
enum TapOutcome {
    /// Handled (or eaten) inside the dialog.
    Stay,
    /// Close without `Save` (the close button).
    Close(&'static str),
    /// `Save`, then close.
    SaveAndClose(&'static str),
}

/// The toggle of `group` under the tap, when it is shown and enabled.
fn tapped_toggle(hit: &dyn Fn(&str) -> bool, group: &GroupBinding) -> Option<usize> {
    group
        .toggles
        .iter()
        .position(|toggle| hit(&toggle.selectable.control))
}

/// A group's toggle turned on by a tap (`CustomIndexToggleGroup` with switch
/// off disallowed: tapping the toggle already on changes nothing). Returns
/// whether the selection changed.
fn select_toggle(slot: &mut Option<usize>, index: usize) -> bool {
    if *slot == Some(index) {
        return false;
    }
    *slot = Some(index);
    true
}

struct TapContext<'a> {
    hit: &'a dyn Fn(&str) -> bool,
    layouts: &'a crate::ui_layout::UiLayouts,
    sounds: &'a mut SeRequests,
    configs: Option<&'a crate::client_config::ClientConfigs>,
}

impl TapContext<'_> {
    fn sound(&mut self, control: &str) {
        self.sounds.source_button(self.layouts, KEY, control);
    }
}

fn tap_live(cx: &mut TapContext, binding: &LiveBinding, state: &mut OptionDialogState) -> bool {
    let (Some(page), Some(data)) = (state.live.as_mut(), state.live_data.as_mut()) else {
        return false;
    };
    for (index, (bound, selector)) in binding
        .selectors
        .iter()
        .zip(page.selectors.iter_mut())
        .enumerate()
    {
        for (direction, buttons) in [(-1, &bound.decrement), (1, &bound.increment)] {
            let Some(button) = buttons.iter().position(|b| (cx.hit)(&b.control)) else {
                continue;
            };
            let enabled = if direction < 0 {
                selector.can_decrement()
            } else {
                selector.can_increment()
            };
            if enabled {
                cx.sound(&buttons[button].control);
                let Some(step) = selector.step(button) else {
                    return true;
                };
                let before = selector.text();
                selector.update(selector.value + direction * step);
                info!(
                    "[option] Live {}: {before} -> {}",
                    LIVE_SELECTORS[index],
                    selector.text()
                );
                // The alpha selectors' update action.
                if index >= 4 {
                    let key = if index == 4 {
                        "NoteAlpha"
                    } else {
                        "GuideAlpha"
                    };
                    data.set(key, json!(selector.value() / 100.0));
                }
            }
            return true;
        }
    }
    for (index, group) in binding.groups.iter().enumerate() {
        let Some(toggle) = tapped_toggle(cx.hit, group) else {
            continue;
        };
        cx.sound(&group.toggles[toggle].selectable.control);
        let (which, name) = LIVE_TOGGLES[index];
        if which == LiveToggle::Use120Fps && toggle == 0 && page.toggles[index] != Some(0) {
            // OnSelectedUse120Fps: index 0 asks Common2ButtonMediumDialog
            // (MSG_USE_120FPS_DIALOG); cancel sets index 1 without notify.
            info!("[option] Live {name}: the 120 fps confirmation dialog is not in this product; the toggle stays as the cancel leaves it");
            page.toggles[index] = Some(1);
            return true;
        }
        if select_toggle(&mut page.toggles[index], toggle) {
            info!("[option] Live {name}: index {toggle}");
        }
        return true;
    }
    let counts = |key: i32| cx.configs.map(|configs| i64::from(configs.int(key)));
    for (pair, slot, key, name) in [
        (&binding.skin, 0, 79, "NoteSkinIndex"),
        (&binding.tap_se, 1, 80, "NoteSeIndex"),
    ] {
        for (direction, button) in [(-1i64, &pair[0]), (1, &pair[1])] {
            if !(cx.hit)(&button.control) {
                continue;
            }
            let Some(count) = counts(key) else {
                warn!("[option] Live {name}: ClientConfig.Live is not loaded; the index stays");
                return true;
            };
            cx.sound(&button.control);
            let value = if slot == 0 {
                &mut page.note_skin_index
            } else {
                &mut page.note_se_index
            };
            let next = *value + direction;
            *value = if next < 0 {
                count - 1
            } else if next >= count {
                0
            } else {
                next
            };
            info!("[option] Live {name}: {} of {count} (the note previews and names come from the live note bundles, not in this product)", *value);
            return true;
        }
    }
    if (cx.hit)(&binding.timing_tap.control) {
        cx.sound(&binding.timing_tap.control);
        info!("[option] Live timing tap: LiveNoteSettingDialog is not in this product");
        return true;
    }
    for (button, cue) in binding.tests.iter().zip([
        "SE_LIVE_PERFECT",
        "SE_LIVE_FLICK",
        "SE_LIVE_LONG",
        "SE_LIVE_TRACE",
    ]) {
        if (cx.hit)(&button.control) {
            cx.sound(&button.control);
            info!("[option] Live test sound {cue}: the live note SE banks are not in this product");
            return true;
        }
    }
    false
}

fn tap_system(
    cx: &mut TapContext,
    binding: &SystemBinding,
    state: &mut OptionDialogState,
    app: &AppLocalOptions,
) -> bool {
    let Some(page) = state.system.as_mut() else {
        return false;
    };
    for (index, group) in binding.groups.iter().enumerate() {
        let Some(toggle) = tapped_toggle(cx.hit, group) else {
            continue;
        };
        cx.sound(&group.toggles[toggle].selectable.control);
        if select_toggle(&mut page.toggles[index], toggle) {
            info!("[option] System {}: index {toggle}", SYSTEM_TOGGLES[index]);
            if index == SCREEN_FIXED {
                info!("[option] System: DisplayUtility.ActivateFixedScreenOrientation({}) has no host counterpart", toggle == 0);
            }
        }
        return true;
    }
    for ((button, (enabled, _)), name) in binding
        .buttons
        .iter()
        .zip(system_buttons(app))
        .zip(SYSTEM_BUTTONS)
    {
        if (cx.hit)(&button.button.control) {
            if enabled {
                cx.sound(&button.button.control);
                info!(
                    "[option] System {name}: its download or cache dialog is not in this product"
                );
            }
            return true;
        }
    }
    false
}

/// Returns the outcome when a control of the page took the tap.
fn tap_communication(
    cx: &mut TapContext,
    binding: &CommunicationBinding,
    state: &mut OptionDialogState,
) -> Option<TapOutcome> {
    let page = state.communication.as_mut()?;
    if let Some(toggle) = tapped_toggle(cx.hit, &binding.friend) {
        if page.friend_enabled {
            cx.sound(&binding.friend.toggles[toggle].selectable.control);
            select_toggle(&mut page.friend, toggle);
        }
        return Some(TapOutcome::Stay);
    }
    if let Some(toggle) = tapped_toggle(cx.hit, &binding.favorite) {
        cx.sound(&binding.favorite.toggles[toggle].selectable.control);
        select_toggle(&mut page.favorite, toggle);
        return Some(TapOutcome::Stay);
    }
    if (cx.hit)(&binding.block_list.control) {
        if !page.block_list_enabled {
            return Some(TapOutcome::Stay);
        }
        cx.sound(&binding.block_list.control);
        // PushUIScreen(BlockList) and OnPushedListManagementButton (Save,
        // OnApplyOption, Close).
        info!("[option] Communication block list: the BlockList screen is not in this product");
        return Some(TapOutcome::SaveAndClose("block list"));
    }
    None
}

fn tap_volume(
    canvas: Vec2,
    now: f32,
    layouts: &crate::ui_layout::UiLayouts,
    view: &crate::ui_layout::UiPrefabView,
    size: Vec2,
    state: &mut OptionDialogState,
    bus: &mut VolumeBus,
) -> bool {
    for which in SLIDERS {
        let value = state.draft[which.index()];
        if source_hit(
            canvas,
            &format!("{}/UIPartsDecrementButton", slider_root(which)),
            layouts,
            view,
            size,
        ) {
            if value > 0 {
                set_slider(state, bus, now, which, value - 1, "decrement");
            }
            return true;
        }
        if source_hit(
            canvas,
            &format!("{}/UIPartsIncrementButton", slider_root(which)),
            layouts,
            view,
            size,
        ) {
            if value < 100 {
                set_slider(state, bus, now, which, value + 1, "increment");
            }
            return true;
        }
    }
    if let Some(which) = track_slider_at(canvas, layouts, view, size) {
        set_slider(
            state,
            bus,
            now,
            which,
            value_from_x(which, canvas.x, layouts, view, size),
            "track tap",
        );
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn dispatch_tap(
    layouts: &crate::ui_layout::UiLayouts,
    view: &crate::ui_layout::UiPrefabView,
    size: Vec2,
    canvas: Vec2,
    now: f32,
    bindings: &OptionBindings,
    state: &mut OptionDialogState,
    settings: &mut LocalVolumeSettings,
    app: &mut AppLocalOptions,
    bus: &mut VolumeBus,
    sounds: &mut SeRequests,
    configs: Option<&crate::client_config::ClientConfigs>,
) -> TapOutcome {
    let hit = |path: &str| source_hit(canvas, path, layouts, view, size);
    const CLOSE: &str = "WindowRoot/UIPartsCloseButton";
    const OK: &str = "FooterButtons/UIPartsCommonButton";
    if hit(CLOSE) {
        sounds.source_button(layouts, KEY, CLOSE);
        return TapOutcome::Close("close button");
    }
    if hit(OK) {
        sounds.source_button(layouts, KEY, OK);
        return TapOutcome::SaveAndClose("OK");
    }
    if let Some(tab) = bindings
        .tabs
        .iter()
        .find(|tab| tab_shown(tab.page) && hit(&tab.selectable.control))
    {
        if bindings.built(tab.page) {
            sounds.source_button(layouts, KEY, &tab.selectable.control);
            state.select_tab(tab.page, settings, app, bindings);
            // UpdateTabLines runs with the paint.
        } else {
            info!(
                "[option] tab {:?}: this layout cannot build the page",
                tab.page
            );
        }
        return TapOutcome::Stay;
    }
    let mut cx = TapContext {
        hit: &hit,
        layouts,
        sounds,
        configs,
    };
    let taken = match state.current {
        Some(Page::Live) => bindings
            .live
            .as_ref()
            .is_ok_and(|binding| tap_live(&mut cx, binding, state)),
        Some(Page::Volume) => tap_volume(canvas, now, layouts, view, size, state, bus),
        Some(Page::System) => bindings
            .system
            .as_ref()
            .is_ok_and(|binding| tap_system(&mut cx, binding, state, app)),
        Some(Page::Communication) => match bindings
            .communication
            .as_ref()
            .ok()
            .and_then(|binding| tap_communication(&mut cx, binding, state))
        {
            Some(outcome) => return outcome,
            None => false,
        },
        Some(Page::CustomScore) | None => false,
    };
    if taken || hit("WindowRoot") {
        return TapOutcome::Stay;
    }
    // allowCloseExternal: a tap outside the window closes through Save.
    TapOutcome::SaveAndClose("tap outside")
}

/// The scroll offset range of the current page: 0 up to the content height
/// past the viewport.
fn scroll_limit(
    layouts: &crate::ui_layout::UiLayouts,
    view: &crate::ui_layout::UiPrefabView,
    size: Vec2,
    scroll: &ScrollBinding,
) -> f32 {
    let height = |path: &str| {
        view.rect(layouts, path, size)
            .map_or(0.0, |rect| rect.size.y)
    };
    (height(&scroll.content) - height(&scroll.viewport)).max(0.0)
}

/// Taps and drags while the dialog is open (modal: every tap-family end is
/// consumed). A drag starting on a volume slider track moves the slider; any
/// other drag starting in the current page's viewport scrolls its content.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    layouts: Res<crate::ui_layout::UiLayouts>,
    views: Query<&crate::ui_layout::UiPrefabView, With<OptionDialogRoot>>,
    mut gestures: MessageReader<GestureEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    mut dialog: ResMut<ShellDialogState>,
    mut settings: ResMut<LocalVolumeSettings>,
    mut app: ResMut<AppLocalOptions>,
    mut bus: ResMut<VolumeBus>,
    mut state: ResMut<OptionDialogState>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut sounds: ResMut<SeRequests>,
    bindings: Option<Res<OptionBindings>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
) {
    // Common1ButtonDialog.OnHardwareBackKeyProcess: OnClickOK.
    let back = back_keys
        .read()
        .any(|event| state.dialog_id == Some(event.id));
    if back && dialog.option_open {
        save(&mut state, &mut settings, &mut app, "back key");
        dialog.option_open = false;
        return;
    }
    let events: Vec<GestureEvent> = gestures.read().cloned().collect();
    if events.is_empty() || !dialog.option_open {
        return;
    }
    let (Ok(window), Some(root_canvas), Some(bindings), Ok(view)) = (
        windows.single(),
        root_canvas.as_deref(),
        bindings,
        views.single(),
    ) else {
        return;
    };
    let Some(current) = state.current else {
        return;
    };
    let (width, height) = (window.width(), window.height());
    let scale = root_canvas.scale(window);
    let size = root_canvas.size(window);
    let now = time.elapsed_secs();
    let mut outcome = TapOutcome::Stay;
    for event in &events {
        let canvas = to_canvas(event.position, width, height, scale);
        match event.kind {
            GestureKind::Drag => match event.state {
                GestureState::Began => {
                    if current == Page::Volume {
                        if let Some(which) = track_slider_at(canvas, &layouts, view, size) {
                            state.dragging = Some(which);
                            set_slider(
                                &mut state,
                                &mut bus,
                                now,
                                which,
                                value_from_x(which, canvas.x, &layouts, view, size),
                                "track press",
                            );
                            continue;
                        }
                    }
                    if let Some(scroll) = &bindings.scrolls[current.index()] {
                        if source_hit(canvas, &scroll.viewport, &layouts, view, size) {
                            state.scrolling =
                                Some((current, canvas.y, state.scroll[current.index()]));
                        }
                    }
                }
                GestureState::Moved | GestureState::End => {
                    if let Some(which) = state.dragging {
                        set_slider(
                            &mut state,
                            &mut bus,
                            now,
                            which,
                            value_from_x(which, canvas.x, &layouts, view, size),
                            "drag",
                        );
                    }
                    if let Some((page, start, offset)) = state.scrolling {
                        if let Some(scroll) = &bindings.scrolls[page.index()] {
                            let limit = scroll_limit(&layouts, view, size, scroll);
                            state.scroll[page.index()] =
                                (offset + canvas.y - start).clamp(0.0, limit);
                        }
                    }
                    if event.state == GestureState::End {
                        state.dragging = None;
                        state.scrolling = None;
                    }
                }
            },
            GestureKind::Tap | GestureKind::DoubleTap | GestureKind::LongTouch
                if event.state == GestureState::End =>
            {
                consumed.0 = true;
                if !matches!(outcome, TapOutcome::Stay) {
                    continue;
                }
                outcome = dispatch_tap(
                    &layouts,
                    view,
                    size,
                    canvas,
                    now,
                    &bindings,
                    &mut state,
                    &mut settings,
                    &mut app,
                    &mut bus,
                    &mut sounds,
                    configs.as_deref(),
                );
            }
            _ => {}
        }
    }
    match outcome {
        TapOutcome::Stay => {}
        TapOutcome::Close(cause) => {
            info!("[option] {cause}: close without Save (set-up pages' edits stay in the session objects)");
            dialog.option_open = false;
        }
        TapOutcome::SaveAndClose(cause) => {
            save(&mut state, &mut settings, &mut app, cause);
            dialog.option_open = false;
        }
    }
}
