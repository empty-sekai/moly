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
//!   animation's pause (`PlayPauseAsync`) is cancelled by the resume the
//!   same call starts (`SafeCreate` cancels the running token), so the
//!   highlighted record keeps playing.
//! - `OnWillExit`: `ExecuteMysekaiMusicPlaySetApiIfNeeded` (set or eject, see
//!   [`model::Presenter::exit_request`]); the exit does not wait for it.
//! - `OnExited` -> `Dispose`: the chosen record plays (`PlaySettingBgm`), else
//!   the site's default (`PlayDefaultBGM`); `SetFadeTime(1.0)`.
//!
//! The BGM goes through [`crate::audio::RecordChoice`]; the requests go
//! through the server link's `Call::MusicPlaySet` / `Call::MusicPlayEject`.
//!
//! ## The player view (`MysekaiBGMPlayerView`)
//!
//! - `Setup` loads the record's jacket (`LoadJacket`): the package
//!   `MusicUtility.GetJacketResourceInfo` names (see [`jacket`]) and the
//!   blurred background (`UIPartsBlurMusicJacket.LoadJacketAsync`). Until it
//!   arrives the jacket's image and the blurred background are off, as they
//!   are with no jacket at all; `SetupEmpty` clears the text and the jacket.
//! - `MusicInfoContent.Setup` writes the three texts and shows each marquee
//!   with its text; a marquee scrolls a text wider than its mask.
//! - `MysekaiBGMRecordPlayer` rolls the record in and turns it while it
//!   plays, and its `play` event starts the music ([`record`]).
//! - `SetupImages` binds the player view's package images at boot and the
//!   music-play fixture type's icon ([`fixture`]).
//! - The cells' jacket thumbnails (`UIPartsThumbnail`) are the same records'
//!   jackets, fitted to the thumbnail's rect: the loaded image's size is not
//!   keyed by the shared layout, so the aspect fit `UIPartsThumbnail.OnLoadSuccess`
//!   applies (which reads it) is not run and the image keeps its rect.
//!
//! ## Named gaps (roadmap)
//!
//! - The category tabs, the sort dropdown, the free-word search and the grid
//!   view are composed and visible as the source composes them in `Resume`
//!   (`CreateTabData`, `SetupFilterContent`, `SetSortOptions`,
//!   `SetFilterButton`, `MusicSelectChangeViewButton.Setup`); their cells and
//!   interactivity are not wired: the list stays on tab ALL, sort Default and
//!   list mode. The tab cell prefab is not in this UI root, so the strip shows
//!   its frame and arrows only. The filter button is shown and takes taps;
//!   [`dialogs`] loads `BGMFilterDialog` and the change-vocal dialog
//!   (`MysekaiBGMSelectChangeVocalDialog`).
//! - The filter's character checkboxes and its two music toggles are the
//!   prefab's own controls; OK saves `BGMFilterData` into the local settings
//!   document and the list re-filters. The grid view and the sort key stay at
//!   their defaults.
//! - `UserMysekaiMusicVocals` (`UserDataManager.UserMusicVocals`) is not
//!   carried: the change-vocal dialog cannot reproduce a vocal's release
//!   state (the lock and the release-condition balloon), and it omits
//!   `streaming_live` vocals rather than offering one whose ownership is
//!   unknown. The current special season is not carried either, so
//!   special-season vocals are omitted too. The remembered vocal of each
//!   record (`MysekaiRecordSettingDataList`) lives for the session only.
//! - The list view is not `RepeatListVew`: no looping, snapping or centring,
//!   no fade materials. Cells stack from the content's top, the highlighted
//!   one at its highlight height, and a tap selects a cell; the cell whose
//!   selection changes fades in once (the list's fade tween is its stand-in).
//!   A pool of [`CELL_POOL`] cells is re-bound as the list scrolls. The
//!   highlight's size tween is not run: the highlighted row is already laid
//!   out at the highlight size.
//! - The cells' spectrum analyser stays visible but is not animated. The
//!   record player's motion follows the source's states, events and log, but
//!   its durations and turn speed are the port's own values (the animation
//!   prefab's clips are not in the public asset root), the bubble its
//!   greeting raises is the HUD's balloon in place of the prefab's own
//!   greeting, and the blurred background shows the jacket's texture unblurred
//!   (the `UIPartsBlurMusicJacket` shader is not in the asset root, and the
//!   extracted UI carries no material parameters for it).
//! - The tutorial gate before the push (`IsOpenMusicPlayer`) is out of scope.
//! - Each push builds a new presenter; `Resume` without `Initialize` (a
//!   screen above this one closing) does not occur in this product.

mod dialogs;
mod fixture;
mod jacket;
mod model;
mod record;

use std::collections::{BTreeSet, HashMap};

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowEvent};
use moly_assets::json::master::MasterData;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiPrefab};
use serde_json::{json, Value};

use crate::action_button::ActionTapConsumed;
use crate::balloon::{BubbleLayers, Marquee, MarqueeText};
use crate::fixture::FixturePlacement;
use crate::fixture_activity_state::FixtureTarget;
use crate::fixture_colors::FixtureColorChoice;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{LayerCommand, LayerId, ScreenHook, ScreenLayerEvent, UiLayerStack};
use crate::ui_layout::{SpriteLayoutMetrics, UiLayouts, UiPrefabView};
use crate::audio_inputs::ClientMusicPlaySettings;
use crate::userdata::client::music_play::ClientMusicRecords;
use crate::userdata::{Answered, ServerPending};
use moly_link::ServerLink;
use moly_protocol::api::MusicPlayPost;
use moly_protocol::{Call, CallKind, Reply};
use model::{BgmMasters, BootData, Pending, Presenter, VocalWordings};

pub(crate) const SCREEN: LayerId = LayerId::MysekaiBgmSelect;
const SCREEN_DOC: &str = "BgmSelect";
const CELL_DOC: &str = "BgmSelectListCell";
/// The shared header (`ScreenLayerHeader`).
const HEADER: &str = "EditorHeader";
const RUNTIME: &str = "BgmSelectRuntime";
/// The player view's images and the records' jackets, by the root's alias
/// prefix.
const TEXTURE_PREFIX: &str = "mysekai-bgm-select/";
/// A music jacket's package on the region's storage host
/// (`AssetBundleNames.GetMusicJacketName` formats `music/jacket/{0}`); the
/// jacket is the package's file of the package's name.
const JACKET_DIRECTORY: &str = "music/jacket";
/// The record bases' texture (the serialized sprite name of that package).
const RECORD_BASE_ASSET: &str = "bg_base_circle_h240_wh";
/// How many jackets beyond the visible ones keep their load open: the
/// highlighted record's and the pool's are read every frame, so a load
/// started on a cell is not dropped when the list scrolls on.
const JACKET_CACHE_BEYOND_VIEW: usize = 4;
/// A soundtrack jacket's package (`AssetBundleNames` prefixes the soundtrack
/// category's `assetbundleName` with this directory).
const SOUNDTRACK_JACKET_PREFIX: &str = "mysekai/music_record_soundtrack/jacket/";
/// The player view's fixture icon: the `mysekaiSystemFixtureTypeIcons` row of
/// `MysekaiSystemFixtureType.music_play` (`SystemFixtureIcon` built with 6).
const MUSIC_PLAY_FIXTURE: &str = "music_play";
/// The marquees' scroll speed and wait (`UIPartsMarquee`'s serialized `spd`
/// and `wait_sec`, which the exported prefab does not carry: stand-ins,
/// named as such).
const MARQUEE_SPEED: f32 = 60.;
const MARQUEE_WAIT_SECONDS: f32 = 1.;
/// The side room a marquee's restart and end positions leave the mask
/// (`UIPartsMarquee.spd`'s lateral term).
const MARQUEE_LATERAL: f32 = 40.;
/// The player view's text size and line spacing (the values
/// `MusicInfoContent`'s texts carry).
const INFO_FONT_SIZE: f32 = 32.;
const INFO_LINE_SPACING: f32 = -80.;
/// Cell highlights: the list fades a cell's contents back in as it becomes
/// the highlighted one (`MusicSelectListView.SetActiveInFade`); the fade shows
/// its first frame's alpha here, a stand-in named as one.
const CELL_UNHIGHLIGHTED_ALPHA: f32 = 0.35;
/// Cells kept for the viewport and re-bound as the list scrolls.
pub(crate) const CELL_POOL: usize = 10;
/// `ScreenLayerMysekaiBGMSelect.OnFinishStartAnimation`'s one-shot cue.
const START_SE: &str = "se_enter_jukebox";
/// `MysekaiBGMManager.SetFadeTime` in `Resume` and in `Dispose`.
const RESUME_FADE_SECONDS: f32 = 0.3;
const DISPOSE_FADE_SECONDS: f32 = 1.0;

/// The wordings the screen writes or shows: the vocal texts, the empty-state
/// label, the sort caption and the two view-toggle labels, plus the search
/// placeholder.
pub(crate) const WORDINGS: &[&str] = &[
    "MSG_MUSIC_VOCAL",
    "WORD_INSTRUMENTLE_VERSION",
    "MSG_NOT_TARGET_MUSIC",
    "WORD_DEFAULT",
    "WORD_FIND_FROM_SONG_TITLE_CREATOR_NAME",
    "WORD_JACKET_MUSIC_SELECT",
    "WORD_LIST_MUSIC_SELECT",
];

/// The action button's click on a music-play fixture: the head's fixture.
#[derive(Resource)]
pub(crate) struct BgmSelectOpenRequest(pub(crate) Option<FixtureTarget>);

pub(crate) use crate::ui_charsets::BgmSelectCharset;

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
    /// The music-play call sent on exit, until its answer arrives.
    music_call: Option<ServerPending>,
    /// Why the screen cannot be drawn on this root.
    absent: Option<String>,
    refusal_popped: bool,
    /// Pool cell -> the record it shows this frame.
    bound: Vec<Option<i32>>,
    refused_textures: BTreeSet<String>,
    /// `BGMFilterData`: the filter dialog's saved data.
    filter: dialogs::BgmFilter,
    /// The dialog the last click asked for, taken by the dialog host.
    dialog_request: Option<dialogs::DialogRequest>,
    /// A dialog of this screen is open: the screen takes no taps behind it.
    dialog_open: bool,
    /// The player view's text (the marquees restart when it changes).
    player_text: Option<(String, String, String)>,
    /// The marquees' scroll, one per `MusicInfoContent` marquee.
    marquees: Vec<Marquee>,
    /// Records whose selection changed this frame: their cells fade in once.
    faded_in: BTreeSet<i32>,
    /// The records the last drawn frame showed (the highlighted one and the
    /// bound cells'): their jackets, and only theirs, stay asked for.
    wanted: BTreeSet<i32>,
    /// The record player (`MysekaiBGMRecordPlayer`).
    player: record::Player,
    /// The record player's greeting: its text and its bubble.
    greeting: Option<String>,
    greeting_bubble: Option<Entity>,
}

/// `MysekaiRecordSettingDataList`'s vocal of a record, asked before a jacket
/// package is resolved (the package follows the remembered vocal).
fn remembered_vocal(record: i32) -> Option<i32> {
    REMEMBERED.with(|vocals| vocals.borrow().get(&record).copied())
}

thread_local! {
    /// The screen's remembered vocals, filed by the screen and read where a
    /// record's jacket package is resolved.
    static REMEMBERED: std::cell::RefCell<HashMap<i32, i32>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Files the remembered vocals for the jacket packages.
fn file_remembered(state: Res<BgmSelect>) {
    REMEMBERED.with(|vocals| *vocals.borrow_mut() = state.vocal_choice.clone());
}

impl BgmSelect {
    /// The server call this screen waits for, for the director.
    #[allow(dead_code)] // Read by the director's wait.
    pub(crate) fn server_pending(&self) -> Option<&ServerPending> {
        self.music_call.as_ref()
    }

    /// `MysekaiBGMRecordPlayer.OnCallback`'s `"play"`: the roll-in ended and
    /// the record starts playing; the music starts with it (`PlaySelectBgm`).
    fn on_record_played(&mut self, choice: &mut crate::audio::RecordChoice) {
        let Some(presenter) = self.presenter.as_mut() else {
            return;
        };
        // The roll-in ends with `SetPlaying`: the player view's callback
        // (PlaySelectBgm) sets the record playing.
        presenter.playing_record = true;
        let Some(model) = presenter.model(presenter.select) else {
            return;
        };
        if !model.has_resource {
            info!(
                "[bgm-select] record player callback play: record {} has no BGM resource; no BGM change",
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
            "[bgm-select] record player callback play -> PlaySelectBgm: record {}",
            model.record_id
        );
    }

    /// `MysekaiBGMRecordPlayer.OnCallback`'s `"loop"`: every pass after the
    /// first.
    fn on_record_looped(&mut self, choice: &mut crate::audio::RecordChoice) {
        let Some(presenter) = self.presenter.as_mut() else {
            return;
        };
        presenter.playing_record = true;
        let Some(model) = presenter.model(presenter.select) else {
            return;
        };
        if !model.has_resource {
            info!(
                "[bgm-select] record player callback loop: record {} has no BGM resource; no BGM change",
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
            "[bgm-select] record player callback loop -> PlayBGMAsync(site {}, record {} vocal {:?})",
            presenter.boot.site_id,
            model.record_id,
            model.vocal_id
        );
    }
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
    jacket::request(&mut masters);
    fixture::request(&mut masters);
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
    match pointer(&fields[name]).map_err(|error| format!("{name}: {error}"))? {
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

/// A field the prefab names, when it has one and it is not null.
fn optional(instance: &UiInstance, fields: &Value, name: &str) -> Result<Option<String>, String> {
    match fields.get(name) {
        None => Ok(None),
        Some(value) => match pointer(value)? {
            0 => Ok(None),
            id => Ok(Some(instance.selector(id)?)),
        },
    }
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
    /// The `UIPartsThumbnail` of the highlighted and of the neutral contents
    /// (absent when the prefab has none).
    highlight_jacket: Option<String>,
    neutral_jacket: Option<String>,
    /// The `UIPartsThumbnail` image of each of them (its `thumbnailImage`,
    /// once the pool instance addresses it): the record's jacket is bound
    /// there while it is loaded, and the image stays off otherwise.
    jacket_images: [Option<String>; 2],
}

/// A `UIPartsThumbnail`'s image: the `thumbnailImage` its fields name (a
/// CustomRawImage). A thumbnail that names none, or one of another prefab,
/// shows no jacket and is named.
fn thumbnail_image(node: &moly_assets::ui_layout::UiNode, doc: &UiPrefab) -> Result<i64, String> {
    let thumbnail = node
        .components
        .iter()
        .find(|component| component.class.ends_with(".UIPartsThumbnail"))
        .ok_or_else(|| format!("{} is not a UIPartsThumbnail", node.path))?;
    let id = thumbnail
        .fields
        .get("thumbnailImage")
        .ok_or_else(|| format!("{}: UIPartsThumbnail has no thumbnailImage", node.path))
        .and_then(pointer)?;
    if id == 0 {
        return Err(format!("{}: UIPartsThumbnail names no image", node.path));
    }
    doc.find(&format!("@{id}"))?;
    Ok(id)
}

/// The selector of component `id` among the pool's instances, when it sits
/// in one of their subtrees.
fn instance_selector(instances: &[UiInstance], id: i64) -> Option<String> {
    instances.iter().find_map(|instance| instance.selector(id).ok())
}

pub(crate) struct Bindings {
    cells: Vec<CellBinding>,
    cell_size: Vec2,
    highlight_size: Vec2,
    viewport: String,
    content: String,
    source_content_position: Vec2,
    not_found: String,
    /// `MusicSelect.listView` (`MusicSelectListView`).
    list_view: String,
    /// `MusicSelect.gridView`: `SwitchListViewMode` only shows it in grid mode.
    grid_view: String,
    /// `MusicSelectChangeViewButton._gridModeContents` / `_listModeContents`:
    /// `SetupContents` shows one of the two per view mode.
    view_grid_contents: String,
    view_list_contents: String,
    /// `FreeWordFilterContent._clearTextButton`: shown only while the field has
    /// text.
    clear_text: String,
    /// The sort dropdown's caption; `SetSortOptions` writes the current option.
    sort_label: String,
    /// Nodes without a presenter here.
    hidden: Vec<String>,
    /// (image node, texture alias) of `MysekaiBGMPlayerView.SetupImages`.
    images: Vec<(String, String)>,
    title: String,
    artist: String,
    vocal: String,
    /// The marquee objects `MusicInfoContent.Setup` shows when their text is
    /// non-empty: title, artist, vocal.
    title_marquee: String,
    artist_marquee: String,
    vocal_marquee: String,
    /// The marquees' masks and texts, measured on the first placed frame.
    marquees: Vec<MarqueeText>,
    playing: Vec<String>,
    stopped: Vec<String>,
    switch_button: String,
    change_vocal_button: String,
    jacket_shadow: String,
    /// `_jacketThumbnailImage` (the jacket's image, a CustomRawImage) and
    /// `_blurJacket` (`UIPartsBlurMusicJacket`'s thumbnail image).
    jacket: String,
    blur_jacket: String,
    /// `_collaborationObj` and the text in it: shown with a record whose
    /// music belongs to a collaboration, the text being its label.
    collaboration: String,
    /// `_systemFixtureIcon`: shown once the type icon master names its
    /// texture.
    fixture_icon: String,
    /// `_recordAnimationTransform`: the record's roll-in and turns.
    record_animation: String,
    filter_button: String,
    filter_off: String,
    filter_on: String,
}

/// The contents fields a record cell turns off: the difficulty, clear
/// status, video-type and lock parts, the check box and the badge (the
/// Mysekai-record branch of `MusicSelectListCellContentsBase.Setup` turns
/// off the video-type objects). `MusicSelectListCellHighLightContents.LoadJacket`
/// calls `UIPartsSpectrumAnalyzer.Show` for a Mysekai record and both
/// contents load their jacket, so the spectrum analyser and the jacket
/// thumbnails stay visible.
const HIGHLIGHT_OFF: &[&str] = &[
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
    "transitionBalloon",
    "lockButton",
    "vocalLockButton",
];
const NEUTRAL_OFF: &[&str] = &[
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

/// The runtime draws only `Sekai.UI.CustomTextMesh`; the source's empty-state
/// label `NotFilterResult` is a legacy `Sekai.UI.CustomText` (uGUI Text), which
/// the layout leaves undrawn. Append a TMP text cloned from this prefab's own
/// text component, carrying the source label's wording key
/// (`MSG_NOT_TARGET_MUSIC`), font size and centre alignment, so the empty list
/// shows the source wording.
fn adopt_empty_text(doc: &mut UiPrefab) -> Result<(), String> {
    let target = doc
        .nodes
        .iter()
        .position(|node| node.path.ends_with("/NotFilterResult"))
        .ok_or("the BGM select prefab has no NotFilterResult")?;
    let template = doc
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .find(|component| component.class == "Sekai.UI.CustomTextMesh")
        .cloned()
        .ok_or("the BGM select prefab has no CustomTextMesh to clone")?;
    let source = doc.nodes[target]
        .components
        .iter()
        .find(|component| component.class == "Sekai.UI.CustomText")
        .ok_or("NotFilterResult is not a source CustomText")?;
    let size = source.fields["m_FontData"]["m_FontSize"]
        .as_f64()
        .unwrap_or(36.0);
    let color = source.fields["m_Color"].clone();
    let mut text = template;
    text.path_id = -1;
    text.enabled = true;
    text.fields["m_text"] = json!("");
    text.fields["useWordingKey"] = json!(true);
    text.fields["wordingKey"] = json!("MSG_NOT_TARGET_MUSIC");
    text.fields["m_fontSize"] = json!(size);
    text.fields["m_fontSizeBase"] = json!(size);
    text.fields["m_fontColor"] = color.clone();
    text.fields["m_Color"] = color;
    text.fields["m_HorizontalAlignment"] = json!(2);
    text.fields["m_VerticalAlignment"] = json!(512);
    text.fields["m_enableAutoSizing"] = json!(false);
    doc.nodes[target].components.push(text);
    Ok(())
}

fn compose(layouts: &UiLayouts) -> Result<(UiPrefab, Bindings), String> {
    let mut doc = template(layouts, SCREEN_DOC)?.clone();
    adopt_empty_text(&mut doc)?;
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

    // The tab list, sort dropdown, free-word field, filter button and view
    // toggle are composed by `Resume`: `CreateTabData` (CreateFilteringTabDataList
    // -> `MusicSelectUnitTabList.Initialize`), `SetupFilterContent`,
    // `SetSortOptions`, `SetFilterButton`, and `MusicSelectChangeViewButton.Setup`
    // in `MusicSelect.Setup`. None of them is ever deactivated, so they show at
    // boot with zero records too.
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
    let mut hidden = Vec::new();
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
            highlight_jacket: optional(instance, &highlight, "jacketThumbnail")?,
            neutral_jacket: optional(instance, &neutral, "jacketThumbnail")?,
            jacket_images: [None, None],
        });
    }
    // The jacket images are looked up in the cell prefab, which the
    // instances name by the selector of the same node below them.
    let mut named_gap = false;
    for cell in &mut cells {
        for (slot, path) in [&cell.highlight_jacket, &cell.neutral_jacket].into_iter().enumerate()
        {
            let image = path.as_deref().and_then(|path| {
                let node = &doc.nodes[doc.find(path).ok()?];
                let image = thumbnail_image(node, cell_doc).ok()?;
                instance_selector(&instances, image)
            });
            if image.is_none() && !named_gap {
                warn!("[bgm-select] the list cells show no jacket: the cell prefab's UIPartsThumbnail names no image in it");
                named_gap = true;
            }
            cell.jacket_images[slot] = image;
        }
    }

    // SetupImages: the record bases take the package's texture (a RawImage),
    // the rest are Images (`_recordBaseImageArray`, then the named ones).
    let mut images = Vec::new();
    for node in local_list(&player, "_recordBaseImageArray")? {
        images.push((node, RECORD_BASE_ASSET.to_owned()));
    }
    for name in [
        "_bgImage",
        "_bgBottomImage",
        "_setOnBgmButtonImage",
        "_setOffBgmButtonImage",
        "_changeVocalButtonImage",
        "_playOffImage",
        "_playOnImage",
    ] {
        let texture = doc.nodes[doc.find(&field(&player, name)?)?]
            .components
            .iter()
            .find_map(|component| component.sprite.as_ref()?.get("name")?.as_str())
            .unwrap_or_default()
            .to_owned();
        images.push((field(&player, name)?, texture));
    }
    let images = images
        .into_iter()
        .map(|(node, asset)| (node, format!("{TEXTURE_PREFIX}{asset}")))
        .collect();

    // `SetSortOptions` writes the current option into the dropdown's caption;
    // the serialized caption is empty. `MusicSelectChangeViewButton.Setup(0)`
    // shows `_listModeContents` and hides `_gridModeContents`.
    let dropdown = component(&doc, &field(&view, "sortDropDown")?, ".CustomDropdown")?
        .fields
        .clone();
    let view_button = component(
        &doc,
        &field(&select, "_changeViewButton")?,
        ".MusicSelectChangeViewButton",
    )?
    .fields
    .clone();
    let filter_content = component(
        &doc,
        &field(&view, "freeWordFilterContent")?,
        ".FreeWordFilterContent",
    )?
    .fields
    .clone();

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
        list_view: field(&select, "listView")?,
        grid_view: field(&select, "gridView")?,
        view_grid_contents: field(&view_button, "_gridModeContents")?,
        view_list_contents: field(&view_button, "_listModeContents")?,
        clear_text: field(&filter_content, "_clearTextButton")?,
        sort_label: field(&dropdown, "m_CaptionText")?,
        hidden,
        images,
        title: field(&info, "musicTitle")?,
        artist: field(&info, "_artistName")?,
        vocal: field(&info, "musicVocalName")?,
        title_marquee: field(&info, "musicTitleMarquee")?,
        artist_marquee: field(&info, "_artistNameMarquee")?,
        vocal_marquee: field(&info, "musicVocalNameMarquee")?,
        marquees: Vec::new(),
        playing: local_list(&player, "_playingObjects")?,
        stopped: local_list(&player, "_stoppedObjects")?,
        switch_button: field(&player, "_switchPlayStatusButton")?,
        change_vocal_button: field(&player, "_changeVocalButton")?,
        jacket_shadow: field(&player, "_jacketShadow")?,
        jacket: field(&player, "_jacketThumbnailImage")?,
        blur_jacket: field(&player, "_blurJacket")?,
        collaboration: field(&player, "_collaborationObj")?,
        fixture_icon: field(&player, "_systemFixtureIcon")?,
        record_animation: field(&player, "_recordAnimationTransform")?,
        filter_button: field(&view, "filterButton")?,
        filter_off: field(&view, "filterOffImage")?,
        filter_on: field(&view, "filterOnImage")?,
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

/// The player images' authored Sprite metrics: a replacement drawn by the
/// Image rules needs the Sprite's rect, border and pixels per unit, which an
/// extracted PNG does not carry.
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
    // `SystemFixtureIcon.Setup(MysekaiSystemFixtureType.music_play)`: the
    // type's icon, once the type icon master and the root's texture name it.
    match fixture::music_play_alias(&layouts) {
        Some(alias) => {
            view.set_visible(&bindings.fixture_icon, true);
            view.set_texture(&bindings.fixture_icon, &alias);
        }
        None => {
            view.set_visible(&bindings.fixture_icon, false);
            warn!("[bgm-select] the player view's fixture icon stays off: the fixture type icon master names no icon the UI root carries");
        }
    }
    state.bound = vec![None; bindings.cells.len()];
    state.marquees = vec![Marquee::default(); 3];
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

/// Reports a missing texture once, by its alias: the first frame would
/// otherwise name it every frame.
fn refuse_once(refused: &mut BTreeSet<String>, alias: &str, what: &str) {
    if refused.insert(alias.to_owned()) {
        warn!("[bgm-select] {what}: the UI root carries no runtime texture {alias}; it stays off");
    }
}

/// The runtime alias of a record's jacket (bound once its load has arrived).
fn jacket_alias(record: i32) -> String {
    format!("{TEXTURE_PREFIX}jacket/{record}")
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

/// Marks the cells whose selection the next frame changes (they fade in, see
/// [`CELL_UNHIGHLIGHTED_ALPHA`]).
fn fade_cells(faded_in: &mut BTreeSet<i32>, previous: Option<i32>, next: Option<i32>) {
    if let Some(previous) = previous {
        faded_in.insert(previous);
    }
    if let Some(next) = next {
        faded_in.insert(next);
    }
}

/// Update, after the screen manager: the layer's hooks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow_screen(
    mut events: MessageReader<ScreenLayerEvent>,
    mut state: ResMut<BgmSelect>,
    mut link: ResMut<ServerLink>,
    layouts: Res<UiLayouts>,
    owned: Option<Res<ClientMusicRecords>>,
    settings: Option<Res<ClientMusicPlaySettings>>,
    user: Option<Res<crate::userdata::client::ClientUserData>>,
    real: Res<Time<Real>>,
    mut choice: ResMut<crate::audio::RecordChoice>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    details: Option<Res<dialogs::Details>>,
) {
    for event in events.read() {
        if event.screen != SCREEN {
            continue;
        }
        match event.hook {
            ScreenHook::OnBoot => {
                state.scroll = 0.;
                state.map_pending = true;
                state.player = record::Player::default();
                state.player_text = None;
                state.greeting = None;
                state.greeting_bubble = None;
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
                let mut presenter = Presenter::initialize(
                    boot,
                    models,
                    site_row.map(|row| row.mysekai_music_record_id),
                );
                // Source 3 clears `isPlayingRecord`; `PlaySelectBgm` then makes
                // the player view call back, which sets it again. The record
                // that call plays is the highlighted one.
                if presenter.model(presenter.select).is_some() {
                    presenter.playing_record = true;
                }
                // A saved filter may hide the record the default selection
                // picked: Resume's rule keeps the highlighted record only
                // when it is displayed.
                let visible = dialogs::display(&presenter, &state.filter, details.as_deref());
                let selected = presenter.select.unwrap_or(0);
                if !visible
                    .iter()
                    .any(|&index| presenter.all[index].record_id == selected)
                {
                    presenter.select = visible.first().map(|&index| presenter.all[index].record_id);
                }
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
                        if state.music_call.is_some() {
                            info!("[bgm-select] OnWillExitAsync: {} is still waiting for its reply; no second request", request.api_name());
                            continue;
                        }
                        info!("[bgm-select] OnWillExitAsync: ExecuteMysekaiMusicPlaySetApiIfNeeded -> {} {request:?}", request.api_name());
                        let call = match request {
                            MusicPlayPost::Set(request) => Call::MusicPlaySet(request),
                            MusicPlayPost::Eject(request) => Call::MusicPlayEject(request),
                        };
                        let kind = call.kind();
                        let seq = link.call(call);
                        state.music_call = Some(ServerPending {
                            seq: Some(seq),
                            call: kind,
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

/// The music-play call's answer, logged once: the screen is already exiting.
fn read_music_answer(mut answers: MessageReader<Answered>, mut state: ResMut<BgmSelect>) {
    for answer in answers.read() {
        let Some(pending) = state.music_call.as_ref() else {
            continue;
        };
        if pending.seq != Some(answer.seq) {
            continue;
        }
        let api = match &pending.call {
            CallKind::MusicPlaySet => "PutUserMysekaiMusicPlaySetApi",
            CallKind::MusicPlayEject => "PutUserMysekaiMusicPlayEjectApi",
            _ => continue,
        };
        match &answer.outcome {
            Ok(Reply::MusicPlay(reply)) => {
                info!("[bgm-select] {api} replied success {}", reply.success);
            }
            Ok(other) => {
                warn!("[bgm-select] {api} replied an unexpected {other:?}");
            }
            Err(refusal) => {
                warn!("[bgm-select] {api} refused: {} ({})", refusal.reason, refusal.code);
            }
        }
        state.music_call = None;
    }
}

// ---------------------------------------------------------------------------
// The per-frame view
// ---------------------------------------------------------------------------

/// The list's rows: (top, height) of each displayed record from the content's
/// top; the highlighted one at its highlight height. `display` is the
/// filtered index list (`dialogs::display`).
fn row_layout(
    presenter: &Presenter,
    bindings: &Bindings,
    display: &[usize],
) -> (Vec<(f32, f32)>, f32) {
    let mut top = 0.;
    let mut rows = Vec::with_capacity(display.len());
    for &index in display {
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

/// The player view's marquees and the record's roll-in origin, read from the
/// laid-out document on the first frame the root is drawn.
fn measure(bindings: &mut Bindings, doc: &UiPrefab, layouts: &UiLayouts, view: &UiPrefabView, canvas: Vec2) {
    if bindings.marquees.is_empty() {
        let marquees = [
            bindings.title_marquee.clone(),
            bindings.artist_marquee.clone(),
            bindings.vocal_marquee.clone(),
        ];
        bindings.marquees = marquees
            .into_iter()
            .filter_map(|marquee| {
                crate::balloon::marquee_text(layouts, doc, view, &marquee, canvas)
            })
            .collect();
    }
}

/// One marquee frame: `UIPartsMarquee.Init` for a new text, else
/// `UpdateMarquee`. A text is wider than its mask when its width (measured
/// with the balloon atlas's advances, the only font metrics available to this
/// module) passes the mask's width; where the marquee puts the text is
/// returned for the host to write.
fn scroll_marquee(
    spec: &MarqueeText,
    marquee: &mut Marquee,
    art: Option<&crate::balloon::BalloonArt>,
    text: &str,
    fresh: bool,
    dt: f32,
) -> f32 {
    let width = art
        .map(|art| crate::balloon::text_extent(art, text, INFO_FONT_SIZE, INFO_LINE_SPACING).width)
        .unwrap_or(0.);
    let needed = width > spec.mask_half_width * 2.;
    // `GetInitPositionX` is the text's centre column times its pivot (it
    // keeps the text where the layout put it); `GetRestartPositionX` puts the
    // text's right edge `MARQUEE_LATERAL` past the mask's left edge;
    // `GetEndPositionX` puts its left edge `MARQUEE_LATERAL` inside the
    // mask's right edge.
    let init_x = spec.text_centre * spec.text_pivot;
    let restart_x = spec.mask_centre - spec.mask_half_width - MARQUEE_LATERAL + width * 0.5;
    let end_x = spec.mask_centre + spec.mask_half_width + MARQUEE_LATERAL - width * 0.5;
    crate::balloon::step_marquee(
        marquee,
        fresh,
        needed,
        dt,
        MARQUEE_SPEED,
        MARQUEE_WAIT_SECONDS,
        init_x,
        restart_x,
        end_x,
    )
}

/// Raises the record player's greeting: a balloon over the jacket, parented
/// below the jacket's image so it goes with the screen's view, drawn on the
/// layer the screen's views draw on. `fx_mysekai_bgm_select_record`'s
/// greeting event is not in the public asset root; the HUD's balloon is this
/// port's stand-in for it.
#[allow(clippy::too_many_arguments)]
fn raise_greeting(
    commands: &mut Commands,
    greeting: &mut Option<String>,
    greeting_bubble: &mut Option<Entity>,
    root: Entity,
    art: Option<&crate::balloon::BalloonArt>,
    jacket: &str,
    text: &str,
    layers: Option<&BubbleLayers>,
) {
    let Some(art) = art else {
        return;
    };
    // The bubble hangs below the jacket's image: its place in the player
    // view, and its life, are the screen view's.
    let layer = layers.map_or(SITEMAP_LAYER, |layers| layers.0);
    let bubble = crate::balloon::show_screen_bubble(commands, root, jacket, art, layer, text);
    *greeting = Some(text.to_owned());
    *greeting_bubble = Some(bubble);
}

/// Places the greeting at the jacket's image: the bubble's root sits above
/// the image's top edge, centred on it (the HUD balloon's arrow points down
/// at the image).
fn place_greeting(
    commands: &mut Commands,
    view: &UiPrefabView,
    layouts: &UiLayouts,
    jacket: &str,
    bubble: Entity,
    canvas: Vec2,
) {
    let (Some(doc), Some(rects)) = (layouts.document(view.key), view.rects(layouts, canvas)) else {
        return;
    };
    let Ok(index) = doc.find(jacket) else {
        return;
    };
    let rect = &rects[index];
    let top = rect.center().y + rect.size.y * (1. - rect.pivot.y);
    commands
        .entity(bubble)
        .insert(Transform::from_xyz(rect.center().x, top, 0.));
}

/// Takes the greeting down.
fn stop_greeting(commands: &mut Commands, greeting: &mut Option<String>, greeting_bubble: &mut Option<Entity>) {
    if let Some(bubble) = greeting_bubble.take() {
        commands.entity(bubble).despawn();
    }
    *greeting = None;
}

/// One cell's jacket images: a record whose jacket has arrived shows it (the
/// image keeps its rect), and the record without one shows nothing.
fn jacket_cells(
    cell: &CellBinding,
    record: i32,
    jackets: Option<&jacket::Jackets>,
    view: &mut UiPrefabView,
) {
    let jacket = jackets.and_then(|jackets| jackets.image(record));
    for image in cell.jacket_images.iter().flatten() {
        match jacket {
            Some(jacket) => {
                // `UIPartsThumbnailOnLoadSuccess`: the image is enabled and
                // shows the jacket. The fit to the jacket's aspect needs the
                // loaded image's size, which the UI layout does not key, so
                // the image keeps its rect.
                let alias = jacket_alias(record);
                let _ = jacket;
                view.set_visible(image, true);
                view.set_texture(image, &alias);
            }
            None => view.set_visible(image, false),
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn place(
    windows: Query<&Window, With<PrimaryWindow>>,
    stack: Res<UiLayerStack>,
    mut state: ResMut<BgmSelect>,
    layouts: Res<UiLayouts>,
    mut roots: Query<
        (
            Entity,
            &mut BgmSelectRoot,
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
    details: Option<Res<dialogs::Details>>,
    jackets: Option<Res<jacket::Jackets>>,
    art: Option<Res<crate::balloon::BalloonArt>>,
    bubble_layers: Option<Res<BubbleLayers>>,
    time: Res<Time>,
    mut commands: Commands,
    mut choice: ResMut<crate::audio::RecordChoice>,
) {
    let open = stack.current() == SCREEN;
    if open != state.was_open {
        info!("[bgm-select] screen {}", if open { "shown" } else { "closed" });
        state.was_open = open;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    for (_, _, mut visibility, mut transform) in &mut headers {
        visibility.set_if_neq(if open { Visibility::Inherited } else { Visibility::Hidden });
        crate::canvas::set_scale(&mut transform, scale);
    }
    let Ok((roots_entity, root, mut view, mut visibility, mut transform)) = roots.single_mut() else {
        return;
    };
    visibility.set_if_neq(if open { Visibility::Inherited } else { Visibility::Hidden });
    crate::canvas::set_scale(&mut transform, scale);
    if !open {
        return;
    }
    let Some(doc) = layouts.document(RUNTIME) else {
        return;
    };
    let dt = time.delta_secs();
    let (mut root, mut view, mut visibility, mut transform) = (root, view, visibility, transform);
    measure(&mut root.bindings, doc, &layouts, &view, canvas);
    let bindings = &root.bindings;
    let greeting_bubble = state.greeting_bubble;
    let state = &mut *state;
    view.set_visible(&bindings.grid_view, false);
    view.set_visible(&bindings.view_grid_contents, false);
    view.set_visible(&bindings.view_list_contents, true);
    view.set_visible(&bindings.clear_text, false);
    view.set_text(&bindings.sort_label, layouts.wording("WORD_DEFAULT"));
    state.wanted.clear();
    if let Some(record) = state.presenter.as_ref().and_then(|presenter| presenter.select) {
        state.wanted.insert(record);
    }
    let Some(presenter) = state.presenter.as_ref() else {
        for cell in &bindings.cells {
            view.set_visible(&cell.root, false);
        }
        view.set_visible(&bindings.list_view, false);
        view.set_visible(&bindings.not_found, true);
        return;
    };
    let display = dialogs::display(presenter, &state.filter, details.as_deref());
    // The record animation's play and loop callbacks; they change the
    // presenter, so they run once this frame's layout has read it.
    let mut record_event = (false, false);
    // The player view (MysekaiBGMPlayerView.Setup / SetupEmpty).
    match presenter.model(presenter.select) {
        Some(model) => {
            view.set_text(&bindings.title, model.title.clone());
            view.set_text(&bindings.artist, model.artist.clone());
            view.set_text(&bindings.vocal, model.vocal_name.clone());
            let text = (
                model.title.clone(),
                model.artist.clone(),
                model.vocal_name.clone(),
            );
            let fresh = state.player_text.as_ref() != Some(&text);
            if fresh {
                // `MusicInfoContent.Setup`: new text restarts every marquee.
                state.player_text = Some(text.clone());
            }
            for (marquee, empty) in [
                (&bindings.title_marquee, text.0.is_empty()),
                (&bindings.artist_marquee, text.1.is_empty()),
                (&bindings.vocal_marquee, text.2.is_empty()),
            ] {
                view.set_visible(marquee, !empty);
            }
            let texts = [text.0.as_str(), text.1.as_str(), text.2.as_str()];
            for (index, text) in texts.into_iter().enumerate() {
                let (Some(spec), Some(run)) =
                    (bindings.marquees.get(index), state.marquees.get_mut(index))
                else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                let x = scroll_marquee(spec, run, art.as_deref(), text, fresh, dt);
                view.set_anchored_position(&spec.text, Vec2::new(x, 0.));
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
            set_enabled(&mut view, doc, &bindings.change_vocal_button, model.multiple_vocals);
            // `LoadJacket`: the jacket's image and the blurred background;
            // until the jacket arrives both stay off, as with no jacket.
            let jacket = jackets
                .as_deref()
                .and_then(|jackets| jackets.image(model.record_id));
            view.set_visible(&bindings.jacket, jacket.is_some());
            view.set_visible(&bindings.blur_jacket, jacket.is_some());
            if jacket.is_some() {
                view.set_texture(&bindings.jacket, &jacket_alias(model.record_id));
                view.set_texture(&bindings.blur_jacket, &jacket_alias(model.record_id));
            }
            // `SetCollaborationText`: `model.MasterMusicCollaboration.Label`,
            // which the record model does not carry here (a named gap), so
            // the label and the greeting fall back to the record's title.
            let label: Option<String> = None;
            view.set_visible(&bindings.collaboration, label.is_some());
            if let Some(label) = label.clone() {
                view.set_text(&bindings.collaboration, label);
            }
            let events = record::animate(
                presenter,
                &mut state.player,
                &mut view,
                &bindings.record_animation,
                dt,
            );
            let (played, looped) = (events.played, events.looped);
            let greeting = label.unwrap_or_else(|| model.title.clone());
            let wanted = jacket.is_some() && presenter.is_setting_music();
            if !wanted && greeting_bubble.is_some() {
                stop_greeting(&mut commands, &mut state.greeting, &mut state.greeting_bubble);
            } else if wanted && (state.greeting.as_deref() != Some(greeting.as_str())) {
                stop_greeting(&mut commands, &mut state.greeting, &mut state.greeting_bubble);
                raise_greeting(
                    &mut commands,
                    &mut state.greeting,
                    &mut state.greeting_bubble,
                    roots_entity,
                    art.as_deref(),
                    &bindings.jacket,
                    &greeting,
                    bubble_layers.as_deref(),
                );
            }
            record_event = (played, looped);
            if let Some(bubble) = greeting_bubble {
                place_greeting(&mut commands, &view, &layouts, &bindings.jacket, bubble, canvas);
            }
        }
        None => {
            for text in [&bindings.title, &bindings.artist, &bindings.vocal] {
                view.set_text(text, String::new());
            }
            for marquee in [
                &bindings.title_marquee,
                &bindings.artist_marquee,
                &bindings.vocal_marquee,
            ] {
                view.set_visible(marquee, false);
            }
            for node in &bindings.playing {
                view.set_visible(node, false);
            }
            for node in &bindings.stopped {
                view.set_visible(node, true);
            }
            view.set_visible(&bindings.jacket_shadow, false);
            view.set_visible(&bindings.jacket, false);
            view.set_visible(&bindings.blur_jacket, false);
            view.set_visible(&bindings.collaboration, false);
            set_enabled(&mut view, doc, &bindings.switch_button, false);
            set_enabled(&mut view, doc, &bindings.change_vocal_button, false);
            record::animate(
                presenter,
                &mut state.player,
                &mut view,
                &bindings.record_animation,
                dt,
            );
            if greeting_bubble.is_some() {
                stop_greeting(&mut commands, &mut state.greeting, &mut state.greeting_bubble);
            }
        }
    }

    let (rows, height) = row_layout(presenter, bindings, &display);
    view.set_visible(&bindings.list_view, !display.is_empty());
    view.set_visible(&bindings.not_found, display.is_empty());
    view.set_visible(&bindings.filter_on, state.filter.is_filtered());
    view.set_visible(&bindings.filter_off, !state.filter.is_filtered());
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
        let model = &presenter.all[display[row]];
        bound[slot] = Some(model.record_id);
        state.wanted.insert(model.record_id);
        view.set_visible(&cell.root, true);
        view.set_anchored_position(&cell.root, Vec2::new(0., -top));
        view.set_size_delta(&cell.root, Vec2::new(bindings.cell_size.x, h));
        let selected = presenter.select == Some(model.record_id);
        view.set_visible(&cell.highlight, selected);
        view.set_visible(&cell.neutral, !selected);
        let fading = state.faded_in.remove(&model.record_id);
        view.set_alpha(&cell.highlight, if fading { CELL_UNHIGHLIGHTED_ALPHA } else { 1. });
        view.set_alpha(&cell.neutral, if fading { CELL_UNHIGHLIGHTED_ALPHA } else { 1. });
        view.set_text(&cell.highlight_title, model.title.clone());
        view.set_text(&cell.neutral_title, model.title.clone());
        view.set_text(&cell.highlight_artist, model.artist.clone());
        view.set_text(&cell.highlight_vocal, model.vocal_name.clone());
        let is_setting = presenter.setting == Some(model.record_id);
        view.set_visible(&cell.highlight_setting, is_setting);
        view.set_visible(&cell.neutral_setting, is_setting);
        jacket_cells(cell, model.record_id, jackets.as_deref(), &mut view);
    }
    if bound.iter().all(Option::is_some) && first + bound.len() < rows.len() {
        let last = first + bound.len() - 1;
        if rows[last].0 + rows[last].1 < state.scroll + viewport_height {
            warn!("[bgm-select] the {CELL_POOL}-cell pool does not fill a {viewport_height:.0}-unit viewport");
        }
    }
    state.bound = bound;
    match record_event {
        (true, _) => {
            info!("[bgm-select] {}", record::callback("play"));
            state.on_record_played(&mut choice);
        }
        (false, true) => {
            info!("[bgm-select] {}", record::callback("loop"));
            state.on_record_looped(&mut choice);
        }
        _ => {}
    }

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
    if state.dialog_open {
        // The open dialog owns the pointer; its host consumes the taps.
        state.dragging = false;
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
            layouts.queue_button_se(&mut sounds, header_view.key, &header.back);
            info!("[bgm-select] header back button -> BackUIScreen");
            layer_commands.write(LayerCommand::Pop);
            continue;
        }
        let Some(presenter) = state.presenter.as_mut() else {
            continue;
        };
        let has_select = presenter.model(presenter.select).is_some();
        if has_select && hit(view, &layouts, &bindings.switch_button, point, canvas) {
            layouts.queue_button_se(&mut sounds, view.key, &bindings.switch_button);
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
            if let Some(model) = presenter.model(presenter.select) {
                if model.multiple_vocals {
                    layouts.queue_button_se(&mut sounds, view.key, &bindings.change_vocal_button);
                    state.dialog_request = Some(dialogs::DialogRequest::ChangeVocal(model.record_id));
                    info!(
                        "[bgm-select] change vocal button -> ShowChangeVocalDialog(record {})",
                        model.record_id
                    );
                }
            }
            continue;
        }
        if hit(view, &layouts, &bindings.filter_button, point, canvas) {
            layouts.queue_button_se(&mut sounds, view.key, &bindings.filter_button);
            state.dialog_request = Some(dialogs::DialogRequest::Filter);
            info!("[bgm-select] filter button -> Show2ButtonDialog(BGMFilterDialog)");
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
            layouts.queue_button_se(&mut sounds, view.key, &cell);
            fade_cells(&mut state.faded_in, presenter.select, Some(record));
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
        .init_resource::<jacket::Jackets>()
        .add_systems(Startup, load)
        .add_systems(
            Update,
            (
                parse.before(crate::balloon::bake_atlas),
                fixture::keep,
                file_remembered,
                jacket::advance
                    .after(file_remembered)
                    .before(spawn_when_ready),
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
        )
        .add_systems(Update, read_music_answer.after(follow_screen));
    dialogs::install(app);
}
