//! Moly's content library: browsing is independent of runtime ownership.
//!
//! Entries retain authored backend/identity and full transcripts. Playback
//! re-enters the existing conversation and placed-fixture owners; the library
//! coordinates admission, cancellation and a small transport, not a new player.

use crate::{
    fixture_activity_state::{FixtureActivityIdentity, FixtureTarget},
    npc::{CharacterUnitId, NpcActions},
    npc_objective::{TalkBackend, TalkContent},
    player_fixture_action::{
        PlayerFixtureAvailability, PlayerFixtureCancelReason, PlayerFixtureRequest,
        PlayerFixtureRuntime,
    },
    player_talk::{PlayerTalkRequest, PlayerTalkStore, TalkCancelRequest},
    talk::TalkStore,
};
use bevy::{
    asset::{AssetPath, LoadState},
    camera::visibility::RenderLayers,
    input::{
        keyboard::KeyboardInput,
        mouse::{MouseScrollUnit, MouseWheel},
        ButtonState,
    },
    prelude::*,
    ui::{FocusPolicy, RelativeCursorPosition, UiTargetCamera},
    window::{Ime, PrimaryWindow},
};
use moly_assets::json::JsonAsset;
use moly_law::talk::{
    condition_type_discriminant, FixtureStep, TalkStep, CONDITION_AFTER_SET_FIXTURE,
    CONDITION_MYSEKAI_FIXTURE_ID, CONDITION_MYSEKAI_FIXTURE_TAG_ID,
};
use std::collections::{HashMap, HashSet};

pub(crate) mod bridge;
mod capabilities;
pub(crate) use capabilities::{load as load_capabilities, parse as parse_capabilities};
mod activities;
mod catalog;
pub(crate) use activities::build_activity_catalog;
mod context;
mod input;
mod playback;
mod qa;
pub use qa::library_diagnostics;
mod stage_framing;
mod staging;
mod view;
pub(crate) use catalog::{build_talk_catalog, parse_assets};
pub(crate) use context::refresh_context;
pub(crate) use input::input;
pub(crate) use playback::{dispatch, observe_start, reap_preview_owners};
pub(crate) use qa::qa_open;
pub(crate) use staging::{prepare_pending, retire_scene};
pub(crate) use view::{refresh, setup};

const FONT: &[u8] = include_bytes!("../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");
const CHARACTERS: &str = "moly://characters.json";
const FIXTURES: &str = "moly://mysekai-fixtures.json";
const THUMBNAILS: &str = "moly://fixture-thumbnails/fixture-thumbnails.json";
const FIXTURE_MODELS: &str = "moly://fixture-models/index.json";
const MAX_QUERY: usize = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LibraryTab {
    #[default]
    Conversations,
    Furniture,
    Performances,
    Activities,
}
impl LibraryTab {
    fn title(self) -> &'static str {
        match self {
            Self::Conversations => "对话",
            Self::Furniture => "家具",
            Self::Performances => "家具故事",
            Self::Activities => "角色互动",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scope {
    All,
    #[default]
    Here,
    Ready,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ExperienceMode {
    CurrentScene,
    #[default]
    Independent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryKey {
    Talk(TalkBackend, i32),
    Fixture(i32),
    Activity(crate::fixture_activity_data::ActivityKey),
}
#[derive(Clone, Debug)]
struct DialogueLine {
    speaker: String,
    text: String,
}
#[derive(Clone)]
struct LibraryTalk {
    content: TalkContent,
    preview_tweet: Option<moly_law::talk::TweetRef>,
    units: Vec<u32>,
    fixture_ids: Vec<i32>,
    title: String,
    preview: String,
    lines: Vec<DialogueLine>,
    search: String,
    furniture_related: bool,
    drives_fixture: bool,
}
impl LibraryTalk {
    fn key(&self) -> EntryKey {
        EntryKey::Talk(self.content.backend, self.content.master_id)
    }
    fn kind(&self) -> &'static str {
        if self.drives_fixture {
            "家具演出"
        } else if self.furniture_related {
            "家具故事"
        } else {
            "日常对话"
        }
    }
}
#[derive(Clone, Debug)]
struct FixtureSource {
    package: String,
    grid_size: moly_law::fixture::Vector3Int,
    exported: bool,
    layout: u8,
    center_y: i8,
}
#[derive(Clone, Debug, Default)]
enum FixturePresentation {
    #[default]
    Model,
    Surface {
        skin: String,
        wall: bool,
        available: bool,
    },
    Custom {
        ornament: String,
    },
}
#[derive(Clone)]
struct LibraryFixture {
    id: i32,
    name: String,
    description: String,
    action: String,
    search: String,
    thumbnail: Option<Handle<Image>>,
    source: Option<FixtureSource>,
    presentation: FixturePresentation,
}
impl LibraryFixture {
    fn available_for_preview(&self) -> bool {
        match &self.presentation {
            FixturePresentation::Surface { available, .. } => *available,
            _ => self.source.as_ref().is_some_and(|source| source.exported),
        }
    }
    fn action_label(&self) -> &'static str {
        if let FixturePresentation::Surface { wall, .. } = self.presentation {
            return if wall { "墙面外观" } else { "地板外观" };
        }
        if matches!(self.presentation, FixturePresentation::Custom { .. }) {
            return "自定义展示家具";
        }
        match self.action.as_str() {
            "timeline" => "角色互动",
            "loop" => "持续互动",
            "one_shot" => "单次互动",
            _ => "陈设家具",
        }
    }
    fn interactive(&self) -> bool {
        matches!(self.action.as_str(), "timeline" | "loop" | "one_shot")
    }
}
#[derive(Resource)]
pub(crate) struct LibraryAssets {
    characters: Handle<JsonAsset>,
    fixtures: Handle<JsonAsset>,
    thumbnails: Handle<JsonAsset>,
    models: Handle<JsonAsset>,
    reactions: Handle<JsonAsset>,
    portraits: Handle<JsonAsset>,
    processed: [bool; 6],
}
#[derive(Resource)]
pub(crate) struct LibraryFont(Handle<Font>);
#[derive(Resource, Default)]
pub(crate) struct LibraryCatalog {
    talks: Vec<LibraryTalk>,
    activities: Vec<activities::LibraryActivity>,
    activities_ready: bool,
    fixtures: Vec<LibraryFixture>,
    character_names: HashMap<u32, String>,
    character_colors: HashMap<u32, String>,
    character_groups: HashMap<u32, String>,
    character_unit_types: HashMap<u32, String>,
    character_portrait_paths: HashMap<u32, String>,
    character_portraits: HashMap<u32, Handle<Image>>,
    fixture_reactions: HashMap<i32, Vec<Vec<u32>>>,
    thumbnail_paths: HashMap<i32, String>,
    data_issues: Vec<String>,
    source_region: String,
    source_version: String,
    source_ready: bool,
    talks_ready: bool,
    revision: u64,
}
impl LibraryCatalog {
    fn character(&self, unit: u32) -> String {
        self.character_names
            .get(&unit)
            .cloned()
            .unwrap_or_else(|| format!("角色 {unit}"))
    }
    fn fixture(&self, id: i32) -> Option<&LibraryFixture> {
        self.fixtures.iter().find(|row| row.id == id)
    }
    fn fixture_name(&self, id: i32) -> String {
        self.fixture(id)
            .map(|row| row.name.clone())
            .unwrap_or_else(|| format!("家具 {id}"))
    }
    fn fixture_reactions(&self, id: i32) -> &[Vec<u32>] {
        self.fixture_reactions
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    fn portrait(&self, unit: u32) -> Option<&Handle<Image>> {
        self.character_portraits.get(&unit)
    }
    fn talk(&self, key: EntryKey) -> Option<&LibraryTalk> {
        self.talks.iter().find(|row| row.key() == key)
    }
    fn title(&self, key: EntryKey) -> String {
        match key {
            EntryKey::Fixture(id) => self.fixture_name(id),
            EntryKey::Activity(key) => self
                .activity(key)
                .map(|row| row.title.clone())
                .unwrap_or_else(|| "角色互动".into()),
            _ => self
                .talk(key)
                .map(|row| row.title.clone())
                .unwrap_or_else(|| "所选内容".into()),
        }
    }
}
#[derive(Clone)]
struct InstanceView {
    target: FixtureTarget,
    position: Vec3,
    ready: bool,
    can_stage: bool,
    reason: String,
}
#[derive(Resource, Default)]
pub(crate) struct LibraryContext {
    actors: HashMap<u32, bool>,
    instances: HashMap<i32, Vec<InstanceView>>,
    player: Option<Vec3>,
    revision: u64,
}
#[derive(Clone)]
struct PlaybackChoice {
    preview: bool,
    key: EntryKey,
    target: Option<FixtureTarget>,
    ticket: u64,
    mode: ExperienceMode,
}
#[derive(Clone)]
struct ActiveChoice {
    choice: PlaybackChoice,
    title: String,
    started: bool,
    // Finishing the authored action does not release the user's preview scene.
    completed: bool,
    elapsed: f32,
    effect_owner: Option<Entity>,
    static_view: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FixtureDialogTab {
    #[default]
    Details,
    Reactions,
}

#[derive(Resource)]
pub(crate) struct ContentLibrary {
    pub(crate) open: bool,
    // The web shell owns chrome while the same playback owner owns the scene.
    external_ui: bool,
    external_open: bool,
    external_input_capture: bool,
    watching: bool,
    // Scene ownership outlives closing the browser until rollback completes.
    scene_owned: bool,
    last_error: Option<String>,
    tab: LibraryTab,
    scope: Scope,
    mode: ExperienceMode,
    character: Option<u32>,
    special_only: bool,
    related_fixture: Option<i32>,
    inspection: Option<(u64, i32)>,
    search: String,
    search_cursor: usize,
    search_focus: bool,
    select_all: bool,
    ime_preedit: String,
    picker_open: bool,
    narrow_detail: bool,
    fixture_dialog: Option<i32>,
    fixture_dialog_tab: FixtureDialogTab,
    page_size: usize,
    offset: usize,
    selected: Option<EntryKey>,
    selected_uid: Option<String>,
    pending: Option<PlaybackChoice>,
    active: Option<ActiveChoice>,
    next_ticket: u64,
    cleanup_frames: u8,
    stopping: bool,
    status: String,
    release_guard: u8,
    revision: u64,
    filtered: Vec<EntryKey>,
    filter_stamp: (u64, u64, u64),
}
impl Default for ContentLibrary {
    fn default() -> Self {
        Self {
            open: false,
            external_ui: false,
            external_open: false,
            external_input_capture: false,
            watching: false,
            scene_owned: false,
            last_error: None,
            tab: LibraryTab::default(),
            scope: Scope::All,
            mode: ExperienceMode::default(),
            character: None,
            special_only: false,
            related_fixture: None,
            inspection: None,
            search: String::new(),
            search_cursor: 0,
            search_focus: false,
            select_all: false,
            ime_preedit: String::new(),
            picker_open: false,
            narrow_detail: false,
            fixture_dialog: None,
            fixture_dialog_tab: FixtureDialogTab::Details,
            page_size: 5,
            offset: 0,
            selected: None,
            selected_uid: None,
            pending: None,
            active: None,
            next_ticket: 0,
            cleanup_frames: 0,
            stopping: false,
            status: String::new(),
            release_guard: 0,
            revision: 1,
            filtered: Vec::new(),
            filter_stamp: (0, 0, 0),
        }
    }
}
impl ContentLibrary {
    pub(crate) fn browser_busy(&self) -> bool {
        self.active.is_some() || self.pending.is_some() || self.stopping || self.scene_owned
    }

    pub(crate) fn owns_talk_preview(&self, ticket: u64) -> bool {
        !self.stopping
            && self
                .active
                .as_ref()
                .is_some_and(|active| active.choice.preview && active.choice.ticket == ticket)
    }
    pub(crate) fn blocks_world_input(&self) -> bool {
        ((self.open || self.watching || self.fixture_dialog.is_some()) && !self.external_ui)
            || self.active.is_some()
            || self.pending.is_some()
            || self.scene_owned
            || self.external_input_capture
            || self.release_guard != 0
    }
    /// Viewing an NPC performance owns only its actors and furniture. The
    /// spectator remains a normal player; admission/rollback and every other
    /// interaction remain protected by the original world-input gate.
    pub(crate) fn blocks_exploration_input(&self) -> bool {
        if ((self.open || self.fixture_dialog.is_some()) && !self.external_ui)
            || self.external_input_capture
            || self.release_guard != 0
            || self.stopping
            || self.pending.is_some()
        {
            return true;
        }
        if self.active.as_ref().is_some_and(|active| {
            active.started
                && (active.completed
                    || matches!(active.choice.key, EntryKey::Activity(_))
                    || active.static_view)
        }) {
            return false;
        }
        self.blocks_world_input()
    }
    pub(crate) fn blocks_camera_input(&self) -> bool {
        if self.active.as_ref().is_some_and(|active| {
            active.started && matches!(active.choice.key, EntryKey::Talk(_, _))
        }) {
            return ((self.open || self.fixture_dialog.is_some()) && !self.external_ui)
                || self.external_input_capture
                || self.release_guard != 0
                || self.stopping
                || self.pending.is_some();
        }
        self.blocks_exploration_input()
    }
    pub(crate) fn blocks_talk_input(&self) -> bool {
        ((self.open || self.fixture_dialog.is_some()) && !self.external_ui)
            || self.external_input_capture
            || self.release_guard != 0
    }
    pub(crate) fn owns_scene(&self) -> bool {
        self.scene_owned
    }

    /// The original editor info button is a reading entry, not permission to
    /// discard a placement draft, start playback or move the player. Reuse the
    /// same catalogue and portrait links in native and browser presentation.
    pub(crate) fn inspect_fixture(&mut self, fixture_id: i32) -> bool {
        if fixture_id <= 0 || self.browser_busy() {
            return false;
        }
        self.inspection = Some((
            self.inspection.map_or(1, |(id, _)| id.wrapping_add(1)),
            fixture_id,
        ));
        if self.external_ui {
            // The web product owns its furniture drawer. Preserve the shared
            // selection/deep-link contract instead of creating native chrome.
            self.tab = LibraryTab::Furniture;
            self.scope = Scope::All;
            self.related_fixture = Some(fixture_id);
            self.character = None;
            self.special_only = false;
            self.search.clear();
            self.search_cursor = 0;
            self.ime_preedit.clear();
            self.search_focus = false;
            self.picker_open = false;
            self.reset_browse();
            self.selected = Some(EntryKey::Fixture(fixture_id));
            self.narrow_detail = true;
            self.external_open = true;
        } else {
            // The source editor's info button opens FixtureDescriptionDialog,
            // not the full content browser and not ScreenLayerMysekaiInfo.
            self.fixture_dialog = Some(fixture_id);
            self.fixture_dialog_tab = FixtureDialogTab::Details;
            self.search_focus = false;
            self.picker_open = false;
            self.release_guard = 2;
            self.changed();
        }
        true
    }

    fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    fn reset_browse(&mut self) {
        self.offset = 0;
        self.selected = None;
        self.selected_uid = None;
        self.narrow_detail = false;
        self.changed();
    }
    fn close(&mut self) {
        self.open = false;
        self.watching = false;
        self.picker_open = false;
        self.fixture_dialog = None;
        self.search_focus = false;
        self.ime_preedit.clear();
        self.pending = None;
        self.release_guard = 2;
        self.changed();
    }
    fn selected_index(&self) -> usize {
        self.selected
            .and_then(|key| self.filtered.iter().position(|item| *item == key))
            .unwrap_or(0)
    }
    fn select_index(&mut self, index: usize) {
        if let Some(key) = self.filtered.get(index).copied() {
            if self.selected != Some(key) {
                self.selected_uid = None;
            }
            self.selected = Some(key);
            self.offset = (index / self.page_size.max(1)) * self.page_size.max(1);
            self.changed();
        }
    }
    fn rebuild_results(&mut self, catalog: &LibraryCatalog, world: &LibraryContext) {
        let stamp = (self.revision, catalog.revision, world.revision);
        if self.filter_stamp == stamp {
            return;
        }
        self.filtered = catalog::filtered_keys(self, catalog, world);
        if self
            .selected
            .is_none_or(|key| !self.filtered.contains(&key))
        {
            self.selected = self.filtered.first().copied();
            self.selected_uid = None;
            self.offset = 0;
        }
        let max_page = self.filtered.len().saturating_sub(1) / self.page_size.max(1);
        self.offset = self.offset.min(max_page * self.page_size.max(1));
        self.filter_stamp = stamp;
    }
}
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryAction {
    Toggle,
    Return,
    Close,
    Back,
    Tab(LibraryTab),
    SetScope(Scope),
    SetMode(ExperienceMode),
    SpecialFilter,
    CharacterPicker,
    SetCharacter(Option<u32>),
    ClearRelated,
    Related(i32),
    RelatedActivities(i32),
    RelatedStory(i32),
    OpenFixtureDialog(i32),
    FixtureDialogTab(FixtureDialogTab),
    CloseFixtureDialog,
    FocusSearch,
    ClearSearch,
    Select(EntryKey),
    PreviousPage,
    NextPage,
    PreviousInstance,
    NextInstance,
    Play,
    Preview,
    Stop,
    RestoreScene,
    Continue,
    DismissPicker,
}
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Region {
    Overlay,
    Shell,
    HeaderTitle,
    HeaderSubtitle,
    Navigation,
    ModeDescription,
    SearchBox,
    Filters,
    Content,
    Browser,
    Detail,
    ListViewport,
    ListItems,
    DetailScroll,
    DetailBody,
    CharacterPicker,
    CharacterChoices,
    FixtureDialog,
    FixtureDialogBody,
    Transport,
    Launcher,
    Back,
    CharacterFilter,
    SpecialFilter,
    RelatedFilter,
    InstanceControls,
    ResumeControl,
    FooterHint,
}
#[derive(Component, Clone, Copy)]
pub(crate) enum UiLabel {
    Search,
    Count,
    Page,
    Status,
    ModeDescription,
    Source,
    ReadyScope,
    TransportTitle,
    TransportState,
    Character,
    Special,
    Related,
    Instance,
    Availability,
    Play,
}
#[derive(Component, Clone, Copy)]
pub(crate) enum ButtonKind {
    Primary,
    Secondary,
    Ghost,
    Chip,
    Row,
    Danger,
}
#[derive(Component)]
struct LibraryUiCamera;
#[derive(Component)]
struct DetailArtwork;

pub(crate) fn install(app: &mut App) {
    input::install(app);
    capabilities::install(app);
    app.init_resource::<ContentLibrary>()
        .init_resource::<LibraryCatalog>()
        .init_resource::<LibraryContext>()
        .add_message::<TalkCancelRequest>();
}
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(LibraryAssets {
        characters: server.load::<JsonAsset>(AssetPath::from(CHARACTERS.to_owned())),
        fixtures: server.load::<JsonAsset>(AssetPath::from(FIXTURES.to_owned())),
        thumbnails: server.load::<JsonAsset>(AssetPath::from(THUMBNAILS.to_owned())),
        models: server.load::<JsonAsset>(AssetPath::from(FIXTURE_MODELS.to_owned())),
        reactions: server.load::<JsonAsset>(moly_assets::fixture_reactions()),
        portraits: server.load::<JsonAsset>(moly_assets::character_portraits()),
        processed: [false; 6],
    });
}
fn excerpt(value: &str, limit: usize) -> String {
    let cleaned = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= limit {
        cleaned
    } else {
        cleaned
            .chars()
            .take(limit.saturating_sub(1))
            .collect::<String>()
            + "…"
    }
}
fn plain_text(value: &str) -> String {
    // Display source prose, not Unity/TMP formatting directives.
    let mut tag = false;
    value
        .chars()
        .filter(|ch| {
            if *ch == '<' {
                tag = true;
                return false;
            }
            if *ch == '>' && tag {
                tag = false;
                return false;
            }
            !tag && (*ch == '\n' || *ch == '\t' || !ch.is_control())
        })
        .collect()
}
fn human_preparation_error(
    error: &crate::player_fixture_action::PlayerFixturePreparationError,
) -> String {
    use crate::player_fixture_action::PlayerFixturePreparationError as Error;
    match error {
        Error::Missing(reason)
            if reason.contains("source SD action mapping")
                || reason.contains("NoTalk visual relation") =>
        {
            "当前玩家角色没有这件家具的互动动作，仍可欣赏相关故事".into()
        }
        Error::Missing(reason) if reason.contains("locator radius tile identities") => {
            "家具附近的路径或占用信息不完整，请调整摆放后重试".into()
        }
        Error::Missing(reason) if reason.contains("original") || reason.contains("EndLoc") => {
            "这件家具缺少可靠的互动位置，暂时无法体验".into()
        }
        Error::Missing(_) => "互动资源正在准备，请稍候".into(),
        Error::Invalid(_) => "这件家具的互动资源不完整".into(),
        Error::Rejected(reason) => human_reason(reason).into(),
        Error::Timeline(_) => "这件家具的演出资源暂不可用".into(),
    }
}
fn human_reason(reason: &str) -> &str {
    if reason.contains("session owns input") {
        "另一段家具互动正在进行"
    } else if reason.contains("reservation") || reason.contains("occupied") {
        "这个位置正被占用"
    } else if reason.contains("navigation") || reason.contains("path") {
        "暂时无法到达这个位置"
    } else if reason.contains("no_action") || reason.contains("unsupported") {
        "当前尚不支持这件家具的角色互动"
    } else {
        "当前场景条件不满足"
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_excerpt_is_safe() {
        assert_eq!(excerpt("你好，Moly 世界", 6), "你好，Mo…");
    }
    #[test]
    fn description_markup_is_not_a_ui_label() {
        assert_eq!(
            plain_text("第一行\n<color=#fff>第二行</color>"),
            "第一行\n第二行"
        );
    }
    #[test]
    fn selection_keys_preserve_backend_identity() {
        assert_ne!(
            EntryKey::Talk(TalkBackend::General, 12),
            EntryKey::Talk(TalkBackend::Fixture, 12)
        );
    }
    #[test]
    fn complete_catalog_defaults_to_independent_all_content() {
        let state = ContentLibrary::default();
        assert_eq!(state.mode, ExperienceMode::Independent);
        assert_eq!(state.scope, Scope::All);
    }
    #[test]
    fn native_editor_info_opens_source_fixture_dialog_without_opening_browser() {
        let mut state = ContentLibrary::default();
        assert!(state.inspect_fixture(534));
        assert_eq!(state.fixture_dialog, Some(534));
        assert_eq!(state.fixture_dialog_tab, FixtureDialogTab::Details);
        assert!(!state.open);
        assert!(state.blocks_world_input());
    }
    #[test]
    fn external_editor_info_keeps_browser_owned_detail_contract() {
        let mut state = ContentLibrary::default();
        state.external_ui = true;
        assert!(state.inspect_fixture(534));
        assert!(state.fixture_dialog.is_none());
        assert!(state.external_open);
        assert_eq!(state.selected, Some(EntryKey::Fixture(534)));
    }
}
