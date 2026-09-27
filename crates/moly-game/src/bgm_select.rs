//! The MySekai BGM select screen (`ScreenLayerMysekaiBGMSelect`, screen 626):
//! the record player's song picker.
//!
//! ## Entry
//!
//! The music-play fixture's action button (`OnOpenMysekaiBGMSelect`) calls
//! `MysekaiBGMSelectUtility.MoveScreenLayerMysekaiBGMSelect(site, fixture)`:
//! when `IsMusicPlay(fixtureId)` (the fixture's system fixture row is of type
//! music_play) it pushes screen 626 with the boot data (site id, fixture id,
//! the fixture's colour id). The header's back button pops it.
//!
//! ## The presenter (`BGMSelectPresenter`), in the source's order
//!
//! - `OnBoot` -> `BootProcess`: a new presenter, `Initialize(boot data)`:
//!   `LoadSettingFromSaveData` builds the record list
//!   (`CreateMusicRecordModelList`: the possessed records), the displayed
//!   list (`CreateDisplayMusicData`), the default selection
//!   (`GetDefaultSelectedData`: the site's setting record, else the first
//!   record; a setting found makes it the chosen record and a record plays),
//!   then `Resume(true)`: the highlighted record stays when displayed, else
//!   the first displayed one; `SetupBGMPlayerView`; `SetFadeTime(0.3)`.
//! - `OnInitComponent` -> `Setup`: `SetupBGMPlayerView`, and the highlighted
//!   record plays when a record plays.
//! - `OnFinishStartAnimation`: `PlaySEOneShot("se_enter_jukebox")`.
//! - A cell -> `OnSelectMusic`: it becomes the highlighted record, a record
//!   plays, `SetupBGMPlayerView`.
//! - The play status button -> `OnSwitchPlayStatus` (`SwitchBGMStatus`):
//!   the highlighted record becomes the chosen one, or, when it already is,
//!   the choice is cleared.
//! - `SetupBGMPlayerView`: the player view shows the highlighted record
//!   (`CreateBGMPlayerViewDataModel`: title, artist, vocal text; playing
//!   objects while it is the chosen record; the vocal button while the music
//!   has several vocals) and plays it (`PlaySelectBgm`). The record
//!   animation's pause (`PlayPauseAsync`, whose end would play the default)
//!   is cancelled by the resume the same call starts (`SafeCreate` cancels
//!   the running token), so the highlighted record keeps playing.
//! - `OnWillExit`: `ExecuteMysekaiMusicPlaySetApiIfNeeded` (set or eject, see
//!   [`model::Presenter::exit_request`]); the exit does not wait for it.
//! - `OnExited` -> `Dispose`: the chosen record plays (`PlaySettingBgm`), else
//!   the site's default (`PlayDefaultBGM`); `SetFadeTime(1.0)`.
//!
//! The BGM goes through [`crate::audio::RecordChoice`]; the requests go
//! through [`crate::server::client::music_play::post`].
//!
//! ## Named gaps (roadmap)
//!
//! - The category tabs, the sort dropdown, the free-word search, the filter
//!   button and its dialog (`BGMFilterDialog`), and the grid view mode are
//!   hidden: the list shows tab ALL in sort key Default.
//! - The change-vocal dialog (`MysekaiBGMSelectChangeVocalDialog`) is not
//!   built: its button is refused by name. The remembered vocal of each
//!   record (`MysekaiRecordSettingDataList`) lives for the session only.
//! - The list view is not `RepeatListVew`: no looping, snapping or centring,
//!   no fade materials; cells stack from the content's top, the highlighted
//!   one at its highlight height, and a tap selects a cell. A pool of
//!   [`CELL_POOL`] cells is re-bound as the list scrolls.
//! - The jackets (cells and player view), the blur jacket, the system fixture
//!   icon, the collaboration label, the record animation, the marquees'
//!   scrolling and the cells' spectrum analyser are not built.
//! - The tutorial gate before the push (`IsOpenMusicPlayer`) is out of scope.
//! - Each push builds a new presenter; `Resume` without `Initialize` (a
//!   screen above this one closing) does not occur in this product.

mod model;

use std::collections::{BTreeSet, HashMap};

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowEvent};
use moly_assets::json::master::MasterData;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiPrefab};
use serde_json::Value;

use crate::action_button::ActionTapConsumed;
use crate::fixture::FixturePlacement;
use crate::fixture_activity_state::FixtureTarget;
use crate::fixture_colors::FixtureColorChoice;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::server::client::music::ClientMusicPlaySettings;
use crate::server::client::music_play::{self, ClientMusicRecords};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{LayerCommand, LayerId, ScreenHook, ScreenLayerEvent, UiLayerStack};
use crate::ui_layout::{SpriteLayoutMetrics, UiLayouts, UiPrefabView};
use model::{BgmMasters, BootData, Pending, Presenter, VocalWordings};

pub(crate) const SCREEN: LayerId = LayerId::MysekaiBgmSelect;
const SCREEN_DOC: &str = "BgmSelect";
const CELL_DOC: &str = "BgmSelectListCell";
/// The shared header (`ScreenLayerHeader`).
const HEADER: &str = "EditorHeader";
const RUNTIME: &str = "BgmSelectRuntime";
/// The player view's images, by the root's alias prefix.
const TEXTURE_PREFIX: &str = "mysekai-bgm-select/";
/// Cells kept for the viewport and re-bound as the list scrolls.
pub(crate) const CELL_POOL: usize = 10;
/// `ScreenLayerMysekaiBGMSelect.OnFinishStartAnimation`'s one-shot cue.
const START_SE: &str = "se_enter_jukebox";
/// `MysekaiBGMManager.SetFadeTime` in `Resume` and in `Dispose`.
const RESUME_FADE_SECONDS: f32 = 0.3;
const DISPOSE_FADE_SECONDS: f32 = 1.0;

/// The wordings the screen writes.
pub(crate) const WORDINGS: &[&str] = &["MSG_MUSIC_VOCAL", "WORD_INSTRUMENTLE_VERSION"];

/// The action button's click on a music-play fixture: the head's fixture.
#[derive(Resource)]
pub(crate) struct BgmSelectOpenRequest(pub(crate) Option<FixtureTarget>);

/// The characters the screen can print (record titles, artists, vocal
/// texts, its wordings): a member of the shared glyph atlas.
#[derive(Resource, Default)]
pub(crate) struct BgmSelectCharset {
    pub(crate) chars: Vec<char>,
}

#[derive(Resource, Default)]
pub(crate) struct BgmSelect {
    pending: Pending,
    masters: Option<Result<BgmMasters, String>>,
    /// The boot data of the push in flight.
    boot: Option<BootData>,
    presenter: Option<Presenter>,
    /// `MysekaiRecordSettingDataList`: the vocal remembered per record.
    vocal_choice: HashMap<i32, i32>,
    scroll: f32,
    dragging: bool,
    was_open: bool,
    map_pending: bool,
    /// Why the screen cannot be drawn on this root.
    absent: Option<String>,
    refusal_popped: bool,
    /// Pool cell -> the record it shows this frame.
    bound: Vec<Option<i32>>,
    refused_textures: BTreeSet<String>,
}

// ---------------------------------------------------------------------------
// Start-up: masters and charset
// ---------------------------------------------------------------------------

pub(crate) fn load(
    mut commands: Commands,
    mut masters: ResMut<MasterData>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    if stage.is_some() {
        // A stage draws no screens.
        commands.insert_resource(BgmSelectCharset::default());
        return;
    }
    model::request(&mut masters);
}

fn words(layouts: &UiLayouts) -> VocalWordings {
    let get = |key: &str| layouts.wordings.get(key).cloned().unwrap_or_default();
    VocalWordings {
        vocal: get("MSG_MUSIC_VOCAL"),
        instrumental: get("WORD_INSTRUMENTLE_VERSION"),
    }
}

/// Update: take the masters as they resolve; once they have and the root's
/// wordings are in, the charset joins the shared atlas.
pub(crate) fn parse(
    mut commands: Commands,
    mut state: ResMut<BgmSelect>,
    mut data: ResMut<MasterData>,
    layouts: Res<UiLayouts>,
    charset: Option<Res<BgmSelectCharset>>,
) {
    if state.masters.is_none() {
        let Some(result) = state.pending.poll(&mut data) else {
            return;
        };
        match &result {
            Ok(masters) => {
                info!(
                    "[bgm-select] masters: {} records, {} musics, {} soundtracks, {} tagged all{}",
                    masters.records.len(),
                    masters.musics.len(),
                    masters.sound_tracks.len(),
                    masters.tags.get("all").map_or(0, |ids| ids.len()),
                    if masters.degraded.is_empty() {
                        String::new()
                    } else {
                        format!("; names left empty: {:?}", masters.degraded)
                    }
                );
            }
            Err(reason) => error!(
                "[bgm-select] the record masters are absent ({reason}): the screen lists no records"
            ),
        }
        state.masters = Some(result);
    }
    if charset.is_some() || layouts.document(HEADER).is_none() {
        return;
    }
    let words = words(&layouts);
    let mut chars: Vec<char> = Vec::new();
    for key in WORDINGS {
        if let Some(text) = layouts.wordings.get(*key) {
            chars.extend(text.chars());
        } else {
            warn!("[bgm-select] wording {key} missing from this root: the vocal texts that format it stay empty");
        }
    }
    if let Some(Ok(masters)) = &state.masters {
        for row in &masters.records {
            if row.soundtrack {
                if let Some(track) = masters.sound_tracks.get(&row.external_id) {
                    chars.extend(track.title.chars());
                    chars.extend(track.creator.chars());
                }
                continue;
            }
            let Some(music) = masters.musics.get(&row.external_id) else {
                continue;
            };
            chars.extend(music.title.chars());
            if let Some(artist) = masters.artists.get(&music.creator_artist_id) {
                chars.extend(artist.chars());
            }
            for vocal in masters.vocals.get(&row.external_id).into_iter().flatten() {
                chars.extend(model::vocal_text(vocal, masters, &words).chars());
            }
        }
    }
    chars.sort_unstable();
    chars.dedup();
    info!("[bgm-select] charset: {} characters", chars.len());
    commands.insert_resource(BgmSelectCharset { chars });
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

/// `MoveScreenLayerMysekaiBGMSelect(site, fixture)`.
pub(crate) fn open(
    mut commands: Commands,
    request: Option<Res<BgmSelectOpenRequest>>,
    fixtures: Query<(&FixturePlacement, &FixtureColorChoice)>,
    site: Option<Res<crate::site::SiteActive>>,
    table: Res<crate::system_fixture::SystemFixtures>,
    mut stack: ResMut<UiLayerStack>,
    mut state: ResMut<BgmSelect>,
) {
    let Some(request) = request else {
        return;
    };
    commands.remove_resource::<BgmSelectOpenRequest>();
    let Some(target) = request.0.as_ref() else {
        warn!("[bgm-select] MoveScreenLayerMysekaiBGMSelect: the button's target has no fixture identity; nothing pushed");
        return;
    };
    let (Ok((placement, color)), Some(site)) = (fixtures.get(target.entity), site) else {
        warn!(
            "[bgm-select] MoveScreenLayerMysekaiBGMSelect: fixture {} or the active site is gone; nothing pushed",
            target.uid
        );
        return;
    };
    let fixture_id = placement.fixture_id;
    if !table.is_music_play(fixture_id) {
        info!("[bgm-select] MoveScreenLayerMysekaiBGMSelect: fixture {fixture_id} is not a music-play fixture (IsMusicPlay false); nothing pushed");
        return;
    }
    let boot = BootData {
        site_id: site.site_id,
        fixture_id,
        color_id: i32::try_from(color.texture_id).unwrap_or(i32::MAX),
    };
    state.boot = Some(boot);
    stack.push_ui_screen(
        SCREEN,
        Some(format!(
            "ScreenLayerMysekaiBGMSelectBootData(site {}, fixture {}, colour {})",
            boot.site_id, boot.fixture_id, boot.color_id
        )),
        "MysekaiBGMSelectUtility.MoveScreenLayerMysekaiBGMSelect",
    );
}

// ---------------------------------------------------------------------------
// Composition from the source prefabs
// ---------------------------------------------------------------------------

fn component<'a>(
    doc: &'a UiPrefab,
    selector: &str,
    suffix: &str,
) -> Result<&'a UiComponent, String> {
    let node = &doc.nodes[doc.find(selector)?];
    node.components
        .iter()
        .find(|c| c.class.ends_with(suffix))
        .ok_or_else(|| format!("{} lacks source component {suffix}", node.path))
}

fn pointer(value: &Value) -> Result<i64, String> {
    let p = value
        .as_array()
        .filter(|p| p.len() == 2)
        .ok_or("expected a decoded PPtr")?;
    if p[0].as_i64() != Some(0) {
        return Err("the PPtr is external".into());
    }
    p[1].as_i64().ok_or_else(|| "invalid PPtr path id".into())
}

fn field(fields: &Value, name: &str) -> Result<String, String> {
    match pointer(&fields[name])? {
        0 => Err(format!("required source reference {name} is null")),
        id => Ok(format!("@{id}")),
    }
}

fn local_list(fields: &Value, name: &str) -> Result<Vec<String>, String> {
    let list = fields[name]
        .as_array()
        .ok_or_else(|| format!("source list {name} missing"))?;
    let mut out = Vec::new();
    for value in list {
        match pointer(value)? {
            0 => {}
            id => out.push(format!("@{id}")),
        }
    }
    Ok(out)
}

fn cloned(instance: &UiInstance, fields: &Value, name: &str) -> Result<String, String> {
    instance.selector(pointer(&fields[name])?)
}

fn optional_cloned(
    instance: &UiInstance,
    fields: &Value,
    names: &[&str],
) -> Result<Vec<String>, String> {
    if !fields.is_object() {
        return Err("the contents are not a decoded object".into());
    }
    let mut out = Vec::new();
    for name in names {
        // A field the region's class does not declare has no object to turn
        // off: that region's code never reaches it.
        let Some(value) = fields.get(*name) else {
            continue;
        };
        let id = pointer(value).map_err(|error| format!("{name}: {error}"))?;
        if id != 0 {
            out.push(instance.selector(id)?);
        }
    }
    Ok(out)
}

fn f32_field(fields: &Value, name: &str) -> Result<f32, String> {
    fields[name]
        .as_f64()
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
        .ok_or_else(|| format!("source float missing: {name}"))
}

fn template<'a>(layouts: &'a UiLayouts, key: &str) -> Result<&'a UiPrefab, String> {
    layouts
        .document(key)
        .ok_or_else(|| format!("source UI template not loaded: {key}"))
}

/// One pool cell: a `MusicSelectListCell` with its two contents.
struct CellBinding {
    root: String,
    button: String,
    highlight: String,
    neutral: String,
    highlight_title: String,
    highlight_artist: String,
    highlight_vocal: String,
    highlight_setting: String,
    neutral_title: String,
    neutral_setting: String,
}

pub(crate) struct Bindings {
    cells: Vec<CellBinding>,
    cell_size: Vec2,
    highlight_size: Vec2,
    viewport: String,
    content: String,
    source_content_position: Vec2,
    not_found: String,
    /// Nodes without a presenter here.
    hidden: Vec<String>,
    /// (image node, texture alias) of `MysekaiBGMPlayerView.SetupImages`.
    images: Vec<(String, String)>,
    title: String,
    artist: String,
    vocal: String,
    /// The marquee objects `MusicInfoContent.Setup` shows when their text is
    /// non-empty: (marquee, which text).
    marquees: [String; 3],
    playing: Vec<String>,
    stopped: Vec<String>,
    switch_button: String,
    change_vocal_button: String,
    jacket_shadow: String,
}

/// The contents fields a record cell turns off: the difficulty, clear
/// status, video-type and lock parts, the check box, the badge (the
/// Mysekai-record branch of `MusicSelectListCellContentsBase.Setup` turns
/// off the video-type objects), and the parts without a presenter here.
const HIGHLIGHT_OFF: &[&str] = &[
    "jacketThumbnail",
    "musicDifficultyPlayLevel",
    "uiPartsGroupMusicDifficultyClearStatus",
    "mv3dObj",
    "mv2dObj",
    "originalMvObj",
    "fullRangeMusic",
    "lockObject",
    "vocalLockObject",
    "_preliminaryTournamentBadgeObj",
    "checkBox",
    "spectrum",
    "transitionBalloon",
    "lockButton",
    "vocalLockButton",
];
const NEUTRAL_OFF: &[&str] = &[
    "jacketThumbnail",
    "musicDifficultyPlayLevel",
    "uiPartsGroupMusicDifficultyClearStatus",
    "mv3dObj",
    "mv2dObj",
    "originalMvObj",
    "fullRangeMusic",
    "lockObject",
    "vocalLockObject",
    "_preliminaryTournamentBadgeObj",
    "checkBox",
];

fn compose(layouts: &UiLayouts) -> Result<(UiPrefab, Bindings), String> {
    let mut doc = template(layouts, SCREEN_DOC)?.clone();
    let screen = component(&doc, &doc.prefab, ".ScreenLayerMysekaiBGMSelect")?
        .fields
        .clone();
    let view = component(&doc, &field(&screen, "_view")?, ".MysekaiBGMSelectView")?
        .fields
        .clone();
    let select = component(&doc, &field(&view, "musicSelect")?, ".MusicSelect")?
        .fields
        .clone();
    let list = component(&doc, &field(&select, "listView")?, ".MusicSelectListView")?
        .fields
        .clone();
    let scroll = component(&doc, &field(&list, "scrollRect")?, ".CustomScrollRect")?
        .fields
        .clone();
    let player = component(
        &doc,
        &field(&view, "bgmPlayerView")?,
        ".MysekaiBGMPlayerView",
    )?
    .fields
    .clone();
    let info = component(
        &doc,
        &field(&player, "_musicInfoContent")?,
        ".MusicInfoContent",
    )?
    .fields
    .clone();

    let mut hidden = Vec::new();
    for name in [
        "categoryTabList",
        "sortDropDown",
        "freeWordFilterContent",
        "filterButton",
    ] {
        hidden.push(field(&view, name)?);
    }
    for name in ["gridView", "_changeViewButton"] {
        hidden.push(field(&select, name)?);
    }
    for name in [
        "_jacketThumbnailImage",
        "_blurJacket",
        "_systemFixtureIcon",
        "_collaborationObj",
    ] {
        hidden.push(field(&player, name)?);
    }

    let content = field(&scroll, "m_Content")?;
    let viewport = field(&scroll, "m_Viewport")?;
    let source_content_position =
        Vec2::from_array(doc.nodes[doc.find(&content)?].rect.anchored_position);
    let cell_doc = template(layouts, CELL_DOC)?;
    let cell = component(cell_doc, &cell_doc.prefab, ".MusicSelectListCell")?
        .fields
        .clone();
    let highlight = component(
        cell_doc,
        &field(&cell, "highlightContents")?,
        ".MusicSelectListCellHighLightContents",
    )?
    .fields
    .clone();
    let neutral = component(
        cell_doc,
        &field(&cell, "neutralContents")?,
        ".MusicSelectListCellNeutralContents",
    )?
    .fields
    .clone();
    let names: Vec<String> = (0..CELL_POOL).map(|i| format!("Record{i}")).collect();
    let instances = doc.instantiate_children(
        &content,
        cell_doc,
        &cell_doc.prefab,
        names.iter().map(String::as_str),
    )?;
    let mut cells = Vec::new();
    for instance in &instances {
        hidden.extend(optional_cloned(instance, &highlight, HIGHLIGHT_OFF)?);
        hidden.extend(optional_cloned(instance, &neutral, NEUTRAL_OFF)?);
        for value in neutral["lockObjects"].as_array().into_iter().flatten() {
            match pointer(value)? {
                0 => {}
                id => hidden.push(instance.selector(id)?),
            }
        }
        cells.push(CellBinding {
            root: instance.root_selector(),
            button: cloned(instance, &cell, "selfButton")?,
            highlight: cloned(instance, &cell, "highlightContents")?,
            neutral: cloned(instance, &cell, "neutralContents")?,
            highlight_title: cloned(instance, &highlight, "title")?,
            highlight_artist: cloned(instance, &highlight, "artist")?,
            highlight_vocal: cloned(instance, &highlight, "vocal")?,
            highlight_setting: cloned(instance, &highlight, "mysekaiBGMSettingObj")?,
            neutral_title: cloned(instance, &neutral, "title")?,
            neutral_setting: cloned(instance, &neutral, "mysekaiBGMSettingObj")?,
        });
    }

    // SetupImages: the eight named assets of the player view's package.
    let mut images = Vec::new();
    for node in local_list(&player, "_recordBaseImageArray")? {
        images.push((node, "bg_base_circle_h240_wh"));
    }
    for (name, asset) in [
        ("_bgImage", "bg_mysekai_music_player"),
        ("_bgBottomImage", "bg_mysekai_music_player_bottom"),
        ("_setOnBgmButtonImage", "btn_bgm_set_on"),
        ("_setOffBgmButtonImage", "btn_bgm_set_off"),
        ("_changeVocalButtonImage", "btn_mysekai_vocal_change"),
        ("_playOffImage", "icon_mysekai_music_prayer_off"),
        ("_playOnImage", "icon_mysekai_music_prayer_on"),
    ] {
        images.push((field(&player, name)?, asset));
    }
    let images = images
        .into_iter()
        .map(|(node, asset)| (node, format!("{TEXTURE_PREFIX}{asset}")))
        .collect();

    let bindings = Bindings {
        cells,
        cell_size: Vec2::new(f32_field(&cell, "sizeX")?, f32_field(&cell, "sizeY")?),
        highlight_size: Vec2::new(
            f32_field(&cell, "highlightSizeX")?,
            f32_field(&cell, "highlightSizeY")?,
        ),
        viewport,
        content,
        source_content_position,
        not_found: field(&select, "notFilterResultObject")?,
        hidden,
        images,
        title: field(&info, "musicTitle")?,
        artist: field(&info, "_artistName")?,
        vocal: field(&info, "musicVocalName")?,
        marquees: [
            field(&info, "musicTitleMarquee")?,
            field(&info, "_artistNameMarquee")?,
            field(&info, "musicVocalNameMarquee")?,
        ],
        playing: local_list(&player, "_playingObjects")?,
        stopped: local_list(&player, "_stoppedObjects")?,
        switch_button: field(&player, "_switchPlayStatusButton")?,
        change_vocal_button: field(&player, "_changeVocalButton")?,
        jacket_shadow: field(&player, "_jacketShadow")?,
    };
    Ok((doc, bindings))
}

/// A CustomButton's cover graphics: shown while the button is disabled.
fn set_enabled(view: &mut UiPrefabView, doc: &UiPrefab, selector: &str, enabled: bool) {
    let Ok(button) = component(doc, selector, ".CustomButton") else {
        return;
    };
    let covers = std::iter::once(&button.fields["coverImage"]).chain(
        button.fields["optionalCoverImages"]
            .as_array()
            .into_iter()
            .flatten(),
    );
    for cover in covers {
        match pointer(cover) {
            Ok(0) | Err(_) => {}
            Ok(id) => view.set_visible(&format!("@{id}"), !enabled),
        }
    }
}

/// The shared header's back button (`OnClickBackUIScreen`); every other
/// header node is hidden.
fn bind_header(doc: &UiPrefab, view: &mut UiPrefabView) -> Result<String, String> {
    let mut buttons = doc
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .filter(|c| {
            c.class.ends_with(".CustomButton")
                && c.fields["m_OnClick"]["m_Calls"]
                    .as_array()
                    .is_some_and(|calls| {
                        calls.iter().any(|call| {
                            call["m_MethodName"].as_str() == Some("OnClickBackUIScreen")
                        })
                    })
        });
    let button = buttons
        .next()
        .ok_or("the header has no OnClickBackUIScreen button")?;
    if buttons.next().is_some() {
        return Err("the header's OnClickBackUIScreen button is ambiguous".into());
    }
    let selector = format!("@{}", button.path_id);
    let back = &doc.nodes[doc.find(&selector)?].path;
    for node in &doc.nodes {
        let is_ancestor = back == &node.path || back.starts_with(&format!("{}/", node.path));
        let is_descendant = node.path.starts_with(&format!("{back}/"));
        if !is_ancestor && !is_descendant {
            view.set_visible(&format!("@{}", node.transform_id), false);
        }
    }
    set_enabled(view, doc, &selector, true);
    Ok(selector)
}

/// The two sliced Sprites' authored metrics.
fn register_sprites(layouts: &mut UiLayouts) -> usize {
    let Some(sprites) = layouts.runtime_sprites().cloned() else {
        return 0;
    };
    let mut count = 0;
    for (name, sprite) in sprites.as_object().into_iter().flatten() {
        if !name.starts_with(TEXTURE_PREFIX) {
            continue;
        }
        let number = |field: &str, index: usize| {
            sprite[field][index]
                .as_f64()
                .filter(|v| v.is_finite())
                .expect("source Sprite metric") as f32
        };
        layouts.set_runtime_sprite_layout(
            name,
            SpriteLayoutMetrics {
                rect_size: Vec2::new(number("rectSize", 0), number("rectSize", 1)),
                border: [
                    number("border", 0),
                    number("border", 1),
                    number("border", 2),
                    number("border", 3),
                ],
                pixels_per_unit: sprite["pixelsPerUnit"]
                    .as_f64()
                    .expect("Sprite pixelsPerUnit") as f32,
            },
        );
        count += 1;
    }
    count
}

#[derive(Component)]
pub(crate) struct BgmSelectRoot {
    bindings: Bindings,
}

#[derive(Component)]
pub(crate) struct BgmSelectHeader {
    back: String,
}

#[derive(Resource)]
pub(crate) struct BgmSelectSpawned;

/// The documents this root lacks, once the shared header (a required
/// document) has loaded; `None` while they load.
fn absent_documents(layouts: &UiLayouts) -> Option<Vec<&'static str>> {
    layouts.document(HEADER)?;
    Some(
        [SCREEN_DOC, CELL_DOC]
            .into_iter()
            .filter(|key| layouts.document(key).is_none())
            .collect(),
    )
}

pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    mut state: ResMut<BgmSelect>,
    spawned: Option<Res<BgmSelectSpawned>>,
) {
    if spawned.is_some() || state.absent.is_some() {
        return;
    }
    match absent_documents(&layouts) {
        None => return,
        Some(absent) if !absent.is_empty() => {
            let reason = format!("this root lacks the BGM select documents {absent:?}");
            error!("[bgm-select] {reason}; the screen refuses by name when it is pushed");
            state.absent = Some(reason);
            return;
        }
        Some(_) => {}
    }
    if [SCREEN_DOC, CELL_DOC, HEADER]
        .into_iter()
        .any(|key| !layouts.ready(key, &server))
    {
        return;
    }
    let sprites = register_sprites(&mut layouts);
    let (document, bindings) = match compose(&layouts) {
        Ok(composed) => composed,
        Err(error) => {
            let reason = format!("its composition is refused: {error}");
            error!("[bgm-select] {reason}; the screen refuses by name when it is pushed");
            state.absent = Some(reason);
            return;
        }
    };
    layouts
        .replace_runtime_document(RUNTIME, document, &server)
        .expect("BGM select screen installation");
    let mut view = UiPrefabView::new(RUNTIME, SITEMAP_LAYER);
    for path in &bindings.hidden {
        view.set_visible(path, false);
    }
    let mut missing = Vec::new();
    for (node, alias) in &bindings.images {
        if layouts.has_runtime_texture(alias) {
            view.set_texture(node, alias);
        } else {
            view.set_visible(node, false);
            missing.push(alias.clone());
        }
    }
    if missing.is_empty() {
        info!(
            "[bgm-select] composed: {} pool cells, {} player images ({sprites} sliced Sprites), {} hidden parts",
            bindings.cells.len(),
            bindings.images.len(),
            bindings.hidden.len()
        );
    } else {
        warn!(
            "[bgm-select] player images absent from this root: {missing:?}; those images stay off"
        );
        state.refused_textures.extend(missing);
    }
    state.bound = vec![None; bindings.cells.len()];
    commands.spawn((
        BgmSelectRoot { bindings },
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(SITEMAP_LAYER),
        view,
    ));
    let mut header = UiPrefabView::new(HEADER, SITEMAP_LAYER);
    let back = bind_header(layouts.document(HEADER).unwrap(), &mut header)
        .unwrap_or_else(|error| panic!("BGM select header: {error}"));
    commands.spawn((
        BgmSelectHeader { back },
        Visibility::Hidden,
        Transform::from_xyz(0., 0., 20.),
        RenderLayers::layer(SITEMAP_LAYER),
        header,
    ));
    commands.insert_resource(BgmSelectSpawned);
}

// ---------------------------------------------------------------------------
// The presenter's lifecycle and the BGM
// ---------------------------------------------------------------------------

/// `PlaySelectBgm` / `PlaySettingBgm` -> `PlayBgm(model)`: nothing without a
/// model or without its resource; otherwise `PlayBGMAsync(site, model)`
/// (the choice skips a cue already playing).
fn play_record(
    presenter: &Presenter,
    id: Option<i32>,
    choice: &mut crate::audio::RecordChoice,
    why: &str,
) {
    let Some(model) = presenter.model(id) else {
        return;
    };
    if !model.has_resource {
        info!(
            "[bgm-select] {why}: record {} has no BGM resource; no BGM change",
            model.record_id
        );
        return;
    }
    choice.play_record(
        presenter.boot.site_id,
        model.record_id,
        model.vocal_id.unwrap_or(0),
    );
    info!(
        "[bgm-select] {why}: PlayBGMAsync(site {}, record {} vocal {:?}{})",
        presenter.boot.site_id,
        model.record_id,
        model.vocal_id,
        model
            .music_id
            .map_or_else(|| " soundtrack".to_owned(), |id| format!(" music {id}"))
    );
}

/// `SetupBGMPlayerView`'s BGM part: the highlighted record plays.
fn setup_player_view(presenter: &Presenter, choice: &mut crate::audio::RecordChoice) {
    play_record(
        presenter,
        presenter.select,
        choice,
        "SetupBGMPlayerView -> PlaySelectBgm",
    );
}

/// Update, after the screen manager: the layer's hooks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow_screen(
    mut commands: Commands,
    mut events: MessageReader<ScreenLayerEvent>,
    mut state: ResMut<BgmSelect>,
    layouts: Res<UiLayouts>,
    owned: Option<Res<ClientMusicRecords>>,
    settings: Option<Res<ClientMusicPlaySettings>>,
    user: Option<Res<crate::server::ClientUserData>>,
    real: Res<Time<Real>>,
    mut choice: ResMut<crate::audio::RecordChoice>,
    mut sounds: ResMut<crate::audio::SeRequests>,
) {
    for event in events.read() {
        if event.screen != SCREEN {
            continue;
        }
        match event.hook {
            ScreenHook::OnBoot => {
                state.scroll = 0.;
                state.map_pending = true;
                let Some(boot) = state.boot.take() else {
                    error!("[bgm-select] OnBoot without ScreenLayerMysekaiBGMSelectBootData (pushed without MoveScreenLayerMysekaiBGMSelect): no presenter");
                    continue;
                };
                let models = match &state.masters {
                    Some(Ok(masters)) => {
                        let now_ms = user
                            .as_deref()
                            .map_or(i64::MAX, |user| user.current_timestamp(real.elapsed_secs()));
                        let empty = ClientMusicRecords::default();
                        let (models, refused) = model::record_models(
                            masters,
                            owned.as_deref().unwrap_or(&empty),
                            &state.vocal_choice,
                            &words(&layouts),
                            now_ms,
                        );
                        for reason in refused {
                            warn!("[bgm-select] {reason}");
                        }
                        models
                    }
                    Some(Err(reason)) => {
                        error!("[bgm-select] BootProcess: the record masters are absent ({reason}); the list is empty");
                        Vec::new()
                    }
                    None => {
                        error!("[bgm-select] BootProcess: the record masters have not resolved yet; the list is empty");
                        Vec::new()
                    }
                };
                let site_row = settings.as_deref().and_then(|s| s.row(boot.site_id));
                let presenter = Presenter::initialize(
                    boot,
                    models,
                    site_row.map(|row| row.mysekai_music_record_id),
                );
                info!(
                    "[bgm-select] OnBoot -> BootProcess: Initialize(site {}, fixture {}, colour {}): {} owned records, {} displayed (tab ALL, sort Default); site setting {:?}; chosen {:?}, highlighted {:?}, playing record {}",
                    boot.site_id,
                    boot.fixture_id,
                    boot.color_id,
                    presenter.all.len(),
                    presenter.display.len(),
                    site_row.map(|row| (row.mysekai_music_record_id, row.music_vocal_id)),
                    presenter.setting,
                    presenter.select,
                    presenter.playing_record
                );
                setup_player_view(&presenter, &mut choice);
                // Resume: MysekaiBGMManager.SetFadeTime(0.3).
                choice.set_fade_time(RESUME_FADE_SECONDS);
                state.presenter = Some(presenter);
            }
            ScreenHook::OnInitComponent => {
                if let Some(presenter) = &state.presenter {
                    // Setup: SetupBGMPlayerView, then PlaySelectBgm while a
                    // record plays (the same cue: no second change).
                    setup_player_view(presenter, &mut choice);
                    if presenter.playing_record {
                        play_record(
                            presenter,
                            presenter.select,
                            &mut choice,
                            "Setup -> PlaySelectBgm",
                        );
                    }
                }
            }
            ScreenHook::OnFinishStartAnimation => {
                sounds.0.push(crate::audio::SeRequest {
                    owner: None,
                    cue: START_SE.to_owned(),
                    class: crate::audio::SeClass::Ui,
                    source: "bgm_select",
                });
            }
            ScreenHook::OnWillExit => {
                let Some(presenter) = &state.presenter else {
                    continue;
                };
                let site_row = settings
                    .as_deref()
                    .and_then(|s| s.row(presenter.boot.site_id))
                    .copied();
                match presenter.exit_request(site_row.as_ref()) {
                    None => info!(
                        "[bgm-select] OnWillExitAsync: ExecuteMysekaiMusicPlaySetApiIfNeeded: no request (chosen {:?}, site setting {:?})",
                        presenter.setting,
                        site_row.map(|row| (row.mysekai_music_record_id, row.music_vocal_id))
                    ),
                    Some(request) => {
                        info!("[bgm-select] OnWillExitAsync: ExecuteMysekaiMusicPlaySetApiIfNeeded -> {} {request:?}", request.api_name());
                        commands.queue(move |world: &mut World| {
                            let reply = music_play::post(world, request);
                            info!(
                                "[bgm-select] {} replied success {}",
                                request.api_name(),
                                reply.success
                            );
                        });
                    }
                }
            }
            ScreenHook::OnExited => {
                let Some(presenter) = state.presenter.take() else {
                    continue;
                };
                // Dispose.
                if presenter.setting.is_some() {
                    play_record(
                        &presenter,
                        presenter.setting,
                        &mut choice,
                        "Dispose -> PlaySettingBgm",
                    );
                } else {
                    choice.play_default(presenter.boot.site_id);
                    info!(
                        "[bgm-select] Dispose -> PlayDefaultBGM(site {})",
                        presenter.boot.site_id
                    );
                }
                choice.set_fade_time(DISPOSE_FADE_SECONDS);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// The per-frame view
// ---------------------------------------------------------------------------

/// The list's rows: (top, height) of each displayed record from the content's
/// top; the highlighted one at its highlight height.
fn row_layout(presenter: &Presenter, bindings: &Bindings) -> (Vec<(f32, f32)>, f32) {
    let mut top = 0.;
    let mut rows = Vec::with_capacity(presenter.display.len());
    for &index in &presenter.display {
        let height = if presenter.select == Some(presenter.all[index].record_id) {
            bindings.highlight_size.y
        } else {
            bindings.cell_size.y
        };
        rows.push((top, height));
        top += height;
    }
    (rows, top)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    stack: Res<UiLayerStack>,
    mut state: ResMut<BgmSelect>,
    layouts: Res<UiLayouts>,
    mut roots: Query<
        (
            &BgmSelectRoot,
            &mut UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<BgmSelectHeader>,
    >,
    mut headers: Query<
        (
            &BgmSelectHeader,
            &UiPrefabView,
            &mut Visibility,
            &mut Transform,
        ),
        Without<BgmSelectRoot>,
    >,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let open = stack.current() == SCREEN;
    if open != state.was_open {
        info!(
            "[bgm-select] screen {}",
            if open { "shown" } else { "closed" }
        );
        state.was_open = open;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let visibility_of = |open: bool| {
        if open {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        }
    };
    for (_, _, mut visibility, mut transform) in &mut headers {
        *visibility = visibility_of(open);
        transform.scale = Vec3::splat(scale);
    }
    let Ok((root, mut view, mut visibility, mut transform)) = roots.single_mut() else {
        return;
    };
    *visibility = visibility_of(open);
    transform.scale = Vec3::splat(scale);
    if !open {
        return;
    }
    let Some(doc) = layouts.document(RUNTIME) else {
        return;
    };
    let bindings = &root.bindings;
    let state = &mut *state;
    let Some(presenter) = state.presenter.as_ref() else {
        for cell in &bindings.cells {
            view.set_visible(&cell.root, false);
        }
        view.set_visible(&bindings.not_found, true);
        return;
    };

    // The player view (MysekaiBGMPlayerView.Setup / SetupEmpty).
    match presenter.model(presenter.select) {
        Some(model) => {
            view.set_text(&bindings.title, model.title.clone());
            view.set_text(&bindings.artist, model.artist.clone());
            view.set_text(&bindings.vocal, model.vocal_name.clone());
            for (marquee, text) in
                bindings
                    .marquees
                    .iter()
                    .zip([&model.title, &model.artist, &model.vocal_name])
            {
                view.set_visible(marquee, !text.is_empty());
            }
            let setting = presenter.is_setting_music();
            for node in &bindings.playing {
                view.set_visible(node, setting);
            }
            for node in &bindings.stopped {
                view.set_visible(node, !setting);
            }
            view.set_visible(&bindings.jacket_shadow, true);
            set_enabled(&mut view, doc, &bindings.switch_button, true);
            set_enabled(
                &mut view,
                doc,
                &bindings.change_vocal_button,
                model.multiple_vocals,
            );
        }
        None => {
            for text in [&bindings.title, &bindings.artist, &bindings.vocal] {
                view.set_text(text, String::new());
            }
            for marquee in &bindings.marquees {
                view.set_visible(marquee, false);
            }
            for node in &bindings.playing {
                view.set_visible(node, false);
            }
            for node in &bindings.stopped {
                view.set_visible(node, true);
            }
            view.set_visible(&bindings.jacket_shadow, false);
            set_enabled(&mut view, doc, &bindings.switch_button, false);
            set_enabled(&mut view, doc, &bindings.change_vocal_button, false);
        }
    }
    view.set_visible(&bindings.not_found, presenter.display.is_empty());

    // The list: content height, scroll clamp, the pool bound to the rows the
    // viewport shows.
    let (rows, height) = row_layout(presenter, bindings);
    view.set_size_delta(&bindings.content, Vec2::new(0., height));
    let viewport_height = view
        .rect(&layouts, &bindings.viewport, canvas)
        .map_or(0., |rect| rect.size.y);
    state.scroll = state.scroll.clamp(0., (height - viewport_height).max(0.));
    view.set_anchored_position(
        &bindings.content,
        bindings.source_content_position + Vec2::Y * state.scroll,
    );
    let first = rows
        .iter()
        .position(|(top, h)| top + h > state.scroll)
        .unwrap_or(rows.len());
    let mut bound = vec![None; bindings.cells.len()];
    for (slot, cell) in bindings.cells.iter().enumerate() {
        let row = first + slot;
        let Some(&(top, h)) = rows.get(row) else {
            view.set_visible(&cell.root, false);
            continue;
        };
        if top > state.scroll + viewport_height {
            view.set_visible(&cell.root, false);
            continue;
        }
        let model = &presenter.all[presenter.display[row]];
        bound[slot] = Some(model.record_id);
        view.set_visible(&cell.root, true);
        view.set_anchored_position(&cell.root, Vec2::new(0., -top));
        view.set_size_delta(&cell.root, Vec2::new(bindings.cell_size.x, h));
        let selected = presenter.select == Some(model.record_id);
        view.set_visible(&cell.highlight, selected);
        view.set_visible(&cell.neutral, !selected);
        view.set_text(&cell.highlight_title, model.title.clone());
        view.set_text(&cell.neutral_title, model.title.clone());
        view.set_text(&cell.highlight_artist, model.artist.clone());
        view.set_text(&cell.highlight_vocal, model.vocal_name.clone());
        // IsSettingBGM: the chosen record's setting mark.
        let is_setting = presenter.setting == Some(model.record_id);
        view.set_visible(&cell.highlight_setting, is_setting);
        view.set_visible(&cell.neutral_setting, is_setting);
    }
    if bound.iter().all(Option::is_some) && first + bound.len() < rows.len() {
        let last = first + bound.len() - 1;
        if rows[last].0 + rows[last].1 < state.scroll + viewport_height {
            warn!(
                "[bgm-select] the {CELL_POOL}-cell pool does not fill a {viewport_height:.0}-unit viewport"
            );
        }
    }
    state.bound = bound;

    if std::mem::take(&mut state.map_pending) {
        let size = Vec2::new(window.width(), window.height());
        let pixel = |view: &UiPrefabView, path: &str| {
            view.rect(&layouts, path, canvas).map(|rect| {
                let centre = rect.center() * scale;
                (
                    (centre.x + size.x * 0.5).round() as i32,
                    (size.y * 0.5 - centre.y).round() as i32,
                )
            })
        };
        let back = headers
            .single()
            .ok()
            .and_then(|(header, header_view, _, _)| pixel(header_view, &header.back));
        let cells: Vec<_> = bindings
            .cells
            .iter()
            .zip(&state.bound)
            .filter_map(|(cell, id)| id.map(|id| (id, pixel(&view, &cell.button))))
            .collect();
        info!(
            "[bgm-select] tap map (window px, window {}x{}): back {back:?}, play status {:?}, change vocal {:?}, cells (record, px) {cells:?}",
            size.x,
            size.y,
            pixel(&view, &bindings.switch_button),
            pixel(&view, &bindings.change_vocal_button),
        );
    }
}

fn canvas_position(position: Vec2, window_size: Vec2, scale: f32) -> Vec2 {
    Vec2::new(
        position.x - window_size.x * 0.5,
        window_size.y * 0.5 - position.y,
    ) / scale
}

fn hit(view: &UiPrefabView, layouts: &UiLayouts, path: &str, point: Vec2, canvas: Vec2) -> bool {
    view.rect(layouts, path, canvas)
        .is_some_and(|rect| rect.active && rect.contains(point))
}

/// Taps while the screen is open. The full-screen layer takes every tap.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    mut gestures: MessageReader<GestureEvent>,
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    stack: Res<UiLayerStack>,
    layouts: Res<UiLayouts>,
    mut state: ResMut<BgmSelect>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut layer_commands: MessageWriter<LayerCommand>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    mut choice: ResMut<crate::audio::RecordChoice>,
    roots: Query<(&BgmSelectRoot, &UiPrefabView)>,
    headers: Query<(&BgmSelectHeader, &UiPrefabView)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    let events: Vec<GestureEvent> = gestures.read().copied().collect();
    let scrolls: Vec<MouseWheel> = wheel.read().copied().collect();
    if stack.current() != SCREEN {
        state.dragging = false;
        state.refusal_popped = false;
        return;
    }
    if let Some(reason) = state.absent.clone() {
        consumed.0 |= !events.is_empty();
        if !state.refusal_popped && !stack.is_ui_layer_working() {
            error!("[bgm-select] PushUIScreen(MysekaiBGMSelect) refused: {reason}; the screen is popped");
            layer_commands.write(LayerCommand::Pop);
            state.refusal_popped = true;
        }
        return;
    }
    let (Ok((window_entity, window)), Some(root_canvas)) =
        (windows.single(), root_canvas.as_deref())
    else {
        return;
    };
    let (Ok((root, view)), Ok((header, header_view))) = (roots.single(), headers.single()) else {
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    if !size.is_finite() || size.min_element() <= 0. {
        return;
    }
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let bindings = &root.bindings;
    let state = &mut *state;
    if window.cursor_position().is_some_and(|p| {
        hit(
            view,
            &layouts,
            &bindings.viewport,
            canvas_position(p, size, scale),
            canvas,
        )
    }) {
        for event in scrolls.iter().filter(|event| event.window == window_entity) {
            let delta = match event.unit {
                MouseScrollUnit::Line => event.y * bindings.cell_size.y,
                MouseScrollUnit::Pixel => event.y / scale,
            };
            state.scroll -= delta;
        }
    }
    for event in events {
        consumed.0 = true;
        let point = canvas_position(event.position, size, scale);
        if event.kind == GestureKind::Drag {
            match event.state {
                GestureState::Began => {
                    state.dragging = hit(view, &layouts, &bindings.viewport, point, canvas)
                }
                GestureState::Moved if state.dragging => state.scroll -= event.delta.y / scale,
                GestureState::Moved => {}
                GestureState::End => state.dragging = false,
            }
            continue;
        }
        if !event.kind.is_tap_family() || event.state != GestureState::End {
            continue;
        }
        if hit(header_view, &layouts, &header.back, point, canvas) {
            sounds.source_button(&layouts, header_view.key, &header.back);
            info!("[bgm-select] header back button -> BackUIScreen");
            layer_commands.write(LayerCommand::Pop);
            continue;
        }
        let Some(presenter) = state.presenter.as_mut() else {
            continue;
        };
        let has_select = presenter.model(presenter.select).is_some();
        if has_select && hit(view, &layouts, &bindings.switch_button, point, canvas) {
            sounds.source_button(&layouts, view.key, &bindings.switch_button);
            presenter.switch_status();
            info!(
                "[bgm-select] play status button -> OnSwitchPlayStatus: chosen {:?}, playing record {}",
                presenter.setting, presenter.playing_record
            );
            setup_player_view(presenter, &mut choice);
            state.map_pending = true;
            continue;
        }
        if hit(view, &layouts, &bindings.change_vocal_button, point, canvas) {
            if presenter
                .model(presenter.select)
                .is_some_and(|m| m.multiple_vocals)
            {
                sounds.source_button(&layouts, view.key, &bindings.change_vocal_button);
                warn!("[bgm-select] change vocal button -> ShowChangeVocalDialog refused: the change-vocal dialog is not built");
            }
            continue;
        }
        if let Some(record) = bindings
            .cells
            .iter()
            .zip(&state.bound)
            .find(|(cell, id)| id.is_some() && hit(view, &layouts, &cell.button, point, canvas))
            .and_then(|(_, id)| *id)
        {
            let cell = bindings
                .cells
                .iter()
                .zip(&state.bound)
                .find(|(_, id)| **id == Some(record))
                .map(|(cell, _)| cell.button.clone())
                .unwrap_or_default();
            sounds.source_button(&layouts, view.key, &cell);
            presenter.on_select(record);
            info!("[bgm-select] cell record {record} -> OnSelectMusic");
            setup_player_view(presenter, &mut choice);
            state.map_pending = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Native instrument
// ---------------------------------------------------------------------------

/// `MOLY_BGM_SELECT_TAPS`: comma-separated `<seconds>=<target>` steps in time
/// order (seconds of real time since start-up); a target is `x:y` (window
/// pixels), `back`, `status` (the play status button), `vocal` or `cell:<n>`
/// (the n-th bound cell from the top). Each tap goes to the window's event
/// stream (down and up in one frame), through the gesture and click path.
/// A malformed step stops loudly.
#[derive(Default)]
pub(crate) struct TapScript {
    steps: Option<Vec<(f32, String)>>,
    next: usize,
}

fn parse_taps(raw: &str) -> Vec<(f32, String)> {
    raw.split(',')
        .filter(|step| !step.trim().is_empty())
        .map(|step| {
            let (at, target) = step.split_once('=').unwrap_or_else(|| {
                panic!("MOLY_BGM_SELECT_TAPS step {step:?} is not <seconds>=<target>")
            });
            let at: f32 = at
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("MOLY_BGM_SELECT_TAPS step {step:?}: bad seconds"));
            (at, target.trim().to_owned())
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn tap_instrument(
    mut script: Local<TapScript>,
    real: Res<Time<Real>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    state: Res<BgmSelect>,
    roots: Query<(&BgmSelectRoot, &UiPrefabView)>,
    headers: Query<(&BgmSelectHeader, &UiPrefabView)>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    mut window_events: MessageWriter<WindowEvent>,
) {
    let script = &mut *script;
    let steps = script.steps.get_or_insert_with(|| {
        std::env::var("MOLY_BGM_SELECT_TAPS")
            .map(|raw| parse_taps(&raw))
            .unwrap_or_default()
    });
    let Some((at, target)) = steps.get(script.next).cloned() else {
        return;
    };
    if real.elapsed_secs() < at {
        return;
    }
    script.next += 1;
    let (Ok((window_entity, window)), Some(root_canvas)) =
        (windows.single(), root_canvas.as_deref())
    else {
        warn!("[bgm-select-taps] {at}={target}: no window; skipped");
        return;
    };
    let size = Vec2::new(window.width(), window.height());
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let pixel = |view: &UiPrefabView, path: &str| {
        view.rect(&layouts, path, canvas)
            .filter(|rect| rect.active)
            .map(|rect| {
                let centre = rect.center() * scale;
                Vec2::new(centre.x + size.x * 0.5, size.y * 0.5 - centre.y)
            })
    };
    let root = roots.single().ok();
    let header = headers.single().ok();
    let position = if let Some((x, y)) = target.split_once(':').filter(|(x, _)| *x != "cell") {
        x.parse::<f32>()
            .ok()
            .zip(y.parse::<f32>().ok())
            .map(|(x, y)| Vec2::new(x, y))
    } else if target == "back" {
        header.and_then(|(header, view)| pixel(view, &header.back))
    } else if target == "status" {
        root.and_then(|(root, view)| pixel(view, &root.bindings.switch_button))
    } else if target == "vocal" {
        root.and_then(|(root, view)| pixel(view, &root.bindings.change_vocal_button))
    } else if let Some(n) = target
        .strip_prefix("cell:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        root.and_then(|(root, view)| {
            root.bindings
                .cells
                .iter()
                .zip(&state.bound)
                .filter(|(_, id)| id.is_some())
                .nth(n)
                .and_then(|(cell, _)| pixel(view, &cell.button))
        })
    } else {
        panic!(
            "MOLY_BGM_SELECT_TAPS target {target:?} is not x:y, back, status, vocal or cell:<n>"
        );
    };
    let Some(position) = position else {
        warn!("[bgm-select-taps] {at}={target}: the target is not on screen; skipped");
        return;
    };
    for phase in [TouchPhase::Started, TouchPhase::Ended] {
        window_events.write(WindowEvent::TouchInput(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: 99031,
        }));
    }
    info!(
        "[bgm-select-taps] {at}={target}: tap at ({:.0},{:.0})",
        position.x, position.y
    );
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<BgmSelect>()
        .add_systems(Startup, load)
        .add_systems(
            Update,
            (
                parse.before(crate::balloon::bake_atlas),
                spawn_when_ready,
                tap_instrument.before(crate::gesture::advance),
                open.after(crate::action_button::click)
                    .before(crate::ui_layers::advance),
                click
                    .run_if(crate::game_settings::scene_input_enabled)
                    .after(crate::menu_shell::click)
                    .before(crate::pick::pick)
                    .before(crate::ui_layers::advance),
                follow_screen.after(crate::ui_layers::advance),
                place
                    .after(crate::ui_layers::advance)
                    .after(follow_screen)
                    .after(click)
                    .before(crate::ui_layout::render),
            ),
        );
}
