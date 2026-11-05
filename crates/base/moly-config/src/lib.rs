//! The program configuration file, `moly.toml`.
//!
//! One file holds what a user or a deployment sets before a program starts:
//! where resources and master tables come from, the server address, the
//! server's listen address and data directory, the launcher's choices,
//! display switches and developer switches. The launcher, the game, the
//! developer window and the server read the same file. State a program writes
//! while it runs (settings, server documents, saves) is kept in its own
//! documents and is not part of this file.
//!
//! Every key is declared once in [`KEYS`]: its path (`section.name`), the
//! environment variable that overrides it, the kind of value and what it
//! does. A value is taken from its environment variable when that is set,
//! else from the file, else the caller decides (a default, or a named refusal
//! through [`Config::require_text`]). A variable set to an empty value counts
//! as set for text keys (the caller refuses it by name) and as unset for
//! flags.
//!
//! The file is checked when it is loaded: a key that is not declared, a
//! top-level value outside a section, or a value of the wrong kind is an
//! error that names the key. [`set_values`] changes keys in place and keeps
//! the file's comments and order.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The kind of value a key holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A string.
    Text,
    /// A file or directory path.
    Path,
    /// An `http://` or `https://` address, or an address template.
    Url,
    /// `true` or `false` in the file; `1` or `0` in the environment.
    Flag,
    /// A whole number.
    Integer,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "a string",
            Self::Path => "a path",
            Self::Url => "an http:// or https:// address",
            Self::Flag => "true or false",
            Self::Integer => "a whole number",
        }
    }
}

/// One declared key.
#[derive(Debug, PartialEq, Eq)]
pub struct Key {
    /// `section.name` in `moly.toml`.
    pub path: &'static str,
    /// The environment variable that overrides the file, if any.
    pub env: Option<&'static str>,
    pub kind: Kind,
    /// Which programs read it.
    pub readers: &'static str,
    pub doc: &'static str,
}

/// The key as messages name it: `` `section.name` `` and, when it has one, its
/// environment variable in parentheses.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.env {
            Some(env) => write!(f, "`{}` ({env})", self.path),
            None => write!(f, "`{}`", self.path),
        }
    }
}

impl Key {
    fn section(&self) -> &'static str {
        self.path.split_once('.').map_or(self.path, |(section, _)| section)
    }

    fn name(&self) -> &'static str {
        self.path.split_once('.').map_or(self.path, |(_, name)| name)
    }
}

macro_rules! keys {
    ($($ident:ident = $path:literal, $env:expr, $kind:ident, $readers:literal, $doc:literal;)*) => {
        /// The declared keys, one constant each.
        pub mod keys {
            use super::{Key, Kind};
            $(
                #[doc = $doc]
                pub const $ident: Key = Key {
                    path: $path,
                    env: $env,
                    kind: Kind::$kind,
                    readers: $readers,
                    doc: $doc,
                };
            )*
        }
        /// Every declared key, in the order of the file's sections.
        pub const KEYS: &[&Key] = &[$(&keys::$ident),*];
    };
}

keys! {
    SETTINGS_FILE = "paths.settings", Some("MOLY_SETTINGS_FILE"), Path, "game, developer window",
        "The settings file. Default: `settings.json` in the data directory.";
    RESOURCE_ROOT = "resources.root", Some("MOLY_ASSET_ROOT"), Path, "game, developer window, stage",
        "The resource root directory. Default: the version selected in the launcher, else the newest installed one.";
    RESOURCE_STORE = "resources.store", Some("MOLY_ASSET_STORE_URL"), Url, "game",
        "A public asset store directory read over HTTPS instead of a local resource root. Setting both is refused.";
    RESOURCE_CATALOG = "resources.catalog", Some("MOLY_ASSET_CATALOG"), Text, "game",
        "The catalog release (a lowercase SHA-256) read from the resource root or the asset store.";
    RESOURCE_CACHE = "resources.cache", Some("MOLY_ASSET_CACHE"), Path, "game",
        "Where documents and files read from `resources.store` are kept. Created when missing.";
    RESOURCE_BASE = "resources.base", None, Url, "launcher",
        "The address resources are downloaded from, ending with `/`.";
    RESOURCE_DIR = "resources.dir", None, Path, "launcher, game",
        "Where the launcher installs resources. Default: the `resources` directory beside the data directory.";
    RESOURCE_SNAPSHOT = "resources.snapshot", None, Text, "launcher, game",
        "The installed resource version the game starts with. Default: the newest complete install.";
    STORAGE_BASE = "resources.storage", Some("MOLY_STORAGE_BASE"), Url, "game, developer window",
        "The address of the storage that remote resource files are read from.";
    STORAGE_MIRROR = "resources.storage_mirror", Some("MOLY_STORAGE_MIRROR_BASE"), Url, "game, developer window",
        "The storage tried when `resources.storage` does not answer.";
    MASTER_REPOSITORY = "masters.repository", Some("MOLY_MASTER_REPOSITORY"), Url, "game, developer window",
        "The first master table source: an address template with a `{repo}` path segment, which is replaced by the region's repository name.";
    MASTER_REPOSITORY_JP = "masters.repository_jp", Some("MOLY_MASTER_REPOSITORY_JP"), Text, "game, developer window",
        "The repository name of the `jp` region in `masters.repository`.";
    MASTER_REPOSITORY_CN = "masters.repository_cn", Some("MOLY_MASTER_REPOSITORY_CN"), Text, "game, developer window",
        "The repository name of the `cn` region in `masters.repository`.";
    MASTER_REPOSITORY_TW = "masters.repository_tw", Some("MOLY_MASTER_REPOSITORY_TW"), Text, "game, developer window",
        "The repository name of the `tw` region in `masters.repository`.";
    MASTER_REPOSITORY_KR = "masters.repository_kr", Some("MOLY_MASTER_REPOSITORY_KR"), Text, "game, developer window",
        "The repository name of the `kr` region in `masters.repository`.";
    MASTER_REPOSITORY_EN = "masters.repository_en", Some("MOLY_MASTER_REPOSITORY_EN"), Text, "game, developer window",
        "The repository name of the `en` region in `masters.repository`.";
    MASTER_BASE = "masters.base", Some("MOLY_MASTER_BASE"), Url, "game, developer window",
        "The second master table source.";
    MASTER_MIRROR = "masters.mirror", Some("MOLY_MASTER_MIRROR_BASE"), Url, "game, developer window",
        "The third master table source.";
    PLAYER_API = "player_data.api", Some("MOLY_PLAYER_API"), Url, "developer window",
        "The player data address template, with `{region}` and `{target_user_id}` placeholders.";
    PLAYER_REGION = "player_data.region", Some("MOLY_PLAYER_REGION"), Text, "developer window",
        "The region of the player data: `cn` or `jp`.";
    PLAYER_UID = "player_data.uid", Some("MOLY_PLAYER_UID"), Text, "developer window",
        "Fetches and previews the data of this user id at start.";
    PLAYER_DATA_FILE = "player_data.file", Some("MOLY_PLAYER_DATA_FILE"), Path, "developer window",
        "Imports this player data file at start. Set at most one of `uid` and `file`.";
    SERVER_URL = "server.url", Some("MOLY_SERVER_URL"), Url, "game, developer window",
        "The server the game connects to.";
    SERVER_LISTEN = "server.listen", Some("MOLY_SERVER_ADDR"), Text, "server",
        "The server's listen address (`host:port`).";
    SERVER_CONTROL_PORT = "server.control_port", Some("MOLY_SERVER_CONTROL_PORT"), Integer, "server",
        "The server's control port; 0 asks the system for a free port.";
    SERVER_DATA = "server.data", Some("MOLY_SERVER_DATA"), Path, "server",
        "The directory the server saves player documents in. Unset: documents are kept in memory only.";
    UPDATE_SOURCE = "update.source", None, Url, "launcher",
        "The release source the launcher checks for program updates.";
    LAUNCHER_JOBS = "launcher.jobs", None, Integer, "launcher",
        "Parallel transfers while downloading. Default: 16.";
    LAUNCHER_GAME = "launcher.game", None, Path, "launcher",
        "The game program. Default: the one beside the launcher.";
    HIDE_HUD = "display.hide_hud", Some("MOLY_HIDE_HUD"), Flag, "game, developer window, stage",
        "Hides every HUD, button and dialog view; the scene, the balloons and the talk window remain.";
    HIDE_PLAYER = "display.hide_player", Some("MOLY_HIDE_PLAYER"), Flag, "game, developer window, stage",
        "Hides the player's own body.";
    SHADOW_ENABLED = "graphics.shadow_enabled", Some("MOLY_SHADOW_ENABLED"), Flag, "game, stage",
        "Whether the main light casts a shadow (a depth map drawn before the scene and read by the site and room materials). `false` draws no depth map and leaves every receiver fully lit.";
    SHADOW_RESOLUTION = "graphics.shadow_resolution", Some("MOLY_SHADOW_RESOLUTION"), Integer, "game, stage",
        "The side of the main-light shadow depth maps, in texels: 512 to 8192, a power of two or a multiple of 512. Default: 2560 in the game, 2048 in the browser.";
    SHADOW_FIT = "graphics.shadow_fit", Some("MOLY_SHADOW_FIT"), Text, "game, stage",
        "What the shadow frame covers: `camera` (a sphere around the camera's view out to `graphics.shadow_distance`, dense near the camera) or `site` (one frame around every caster of the site). Default: `camera`.";
    SHADOW_DISTANCE = "graphics.shadow_distance", Some("MOLY_SHADOW_DISTANCE"), Text, "game, stage",
        "How far in front of the camera (`camera` fit) the shadow map reaches and where the shadow fades out, in metres; the fade starts at 90% of it. Default: 30.";
    SHADOW_FILTER = "graphics.shadow_filter", Some("MOLY_SHADOW_FILTER"), Text, "game, stage",
        "How a receiver reads the shadow map: `hard` (one bilinear comparison), `tent` (a nine-tap tent), `pcf` (a sixteen-tap disk of `graphics.shadow_pcf_radius`) or `pcss` (contact hardening from `graphics.shadow_light_angle`). Default: `pcss`.";
    SHADOW_PCF_RADIUS = "graphics.shadow_pcf_radius", Some("MOLY_SHADOW_PCF_RADIUS"), Text, "game, stage",
        "The radius of the `pcf` filter's disk, in metres (0 or more); capped at `graphics.shadow_max_radius_texels`. Default: 0.02.";
    SHADOW_LIGHT_ANGLE = "graphics.shadow_light_angle", Some("MOLY_SHADOW_LIGHT_ANGLE"), Text, "game, stage",
        "The sun's angular diameter for the `pcss` filter, in degrees from 0 up to 90: the penumbra is the blocker-to-receiver distance times its tangent. Default: 2.5.";
    SHADOW_MAX_RADIUS_TEXELS = "graphics.shadow_max_radius_texels", Some("MOLY_SHADOW_MAX_RADIUS_TEXELS"), Text, "game, stage",
        "The largest radius any shadow filter reaches, in map texels (1 or more). Default: 24.";
    SHADOW_STRENGTH = "graphics.shadow_strength", Some("MOLY_SHADOW_STRENGTH"), Text, "game, stage",
        "The shadow's opacity: the final attenuation is mixed from full light to the shadow by this value, 0 to 1. Default: 1.";
    SHADOW_DEPTH_BIAS = "graphics.shadow_depth_bias", Some("MOLY_SHADOW_DEPTH_BIAS"), Text, "game, stage",
        "A multiplier on the casters' depth bias, which keeps a lit surface from shadowing itself; 0 or more. Lower values keep contact shadows tight, higher values hide acne. Default: 1.";
    SHADOW_NORMAL_BIAS = "graphics.shadow_normal_bias", Some("MOLY_SHADOW_NORMAL_BIAS"), Text, "game, stage",
        "A multiplier on the casters' normal bias, which moves casters along their normals by map texels; 0 or more. Default: 1.";
    TEXT_SOURCE = "text.source", Some("MOLY_TEXT_SOURCE"), Path, "developer window",
        "The source text catalogue file the display translation reads. Unset: the original text is shown.";
    TEXT_OFFICIAL = "text.official", Some("MOLY_TEXT_OFFICIAL"), Path, "developer window",
        "An official translation overlay file applied over `text.source`.";
    TEXT_COMMUNITY = "text.community", Some("MOLY_TEXT_COMMUNITY"), Path, "developer window",
        "A community translation overlay file applied over `text.source`.";
    TEXT_LOCALE = "text.locale", Some("MOLY_TEXT_LOCALE"), Text, "developer window",
        "The locale of the display translation. Default: `zh-CN`.";
    DEV_WINDOW = "dev.window", Some("MOLY_DEV"), Flag, "game, server",
        "Starts the developer window instead of the game.";
    DEV_CHECK_SYSTEMS = "dev.check_systems", Some("MOLY_CHECK_SYSTEMS"), Flag, "game",
        "Reports every system parameter conflict and exits instead of starting the game.";
    DEV_PORTRAITS_OUT = "dev.portraits_out", Some("MOLY_PORTRAITS_OUT"), Path, "developer window",
        "Exports character portraits into this directory, with its own settings file there, instead of starting normally.";
    DEV_PORTRAIT_UNIT = "dev.portrait_unit", Some("MOLY_PORTRAIT_UNIT"), Text, "developer window",
        "Limits the portrait export to this source unit id.";
    DEV_ACTION_BUTTON_AUTOWALK_SECS = "dev.action_button_autowalk_secs", Some("MOLY_ACTION_BUTTON_AUTOWALK_SECS"), Text, "developer window",
        "A window (`<until>` or `<from>:<until>` seconds) in which the player is walked to each character and fixture and the action button is tapped; unset: the autowalk is off.";
    DEV_AUDIO_MOCK_MUSIC_RECORD = "dev.audio_mock_music_record", Some("MOLY_AUDIO_MOCK_MUSIC_RECORD"), Text, "server",
        "Sets the default document's `userMysekaiMusicPlayFixtureSettings` from `site:record:vocal` entries separated by commas; a malformed entry stops the start with a panic. Unset: the document's own settings.";
    DEV_AVATAR_MOCK_ACCESSORY = "dev.avatar_mock_accessory", Some("MOLY_AVATAR_MOCK_ACCESSORY"), Text, "server",
        "The accessory bundle name the developer avatar wears, turned into its master id once the masters resolve; a name no master row holds is logged. Unset: the document's own accessory.";
    DEV_AVATAR_MOCK_COORDINATE = "dev.avatar_mock_coordinate", Some("MOLY_AVATAR_MOCK_COORDINATE"), Text, "server",
        "The coordinate bundle name the developer avatar wears, turned into its master id once the masters resolve; a name no master row holds is logged. Unset: the document's own coordinate.";
    DEV_AVATAR_MOCK_COSTUME = "dev.avatar_mock_costume", Some("MOLY_AVATAR_MOCK_COSTUME"), Text, "server",
        "The costume bundle name the developer avatar wears, turned into its master id once the masters resolve; a name no master row holds is logged. Unset: the document's own costume.";
    DEV_AVATAR_MOCK_SKIN_COLOR = "dev.avatar_mock_skin_color", Some("MOLY_AVATAR_MOCK_SKIN_COLOR"), Text, "server",
        "The skin colour code the developer avatar wears, turned into its master id once the masters resolve; a code no master row holds is logged. Unset: the document's own skin colour.";
    DEV_BALLOON_ORDER_NO_LAYER = "dev.balloon_order_no_layer", Some("MOLY_BALLOON_ORDER_NO_LAYER"), Text, "developer window",
        "Feeds 0 as every balloon's root z, so balloons keep the constant layer instead of being ranked by distance from the camera; the negative arm of the layer-order check. Unset: balloons are ranked by distance.";
    DEV_BALLOON_ORDER_RUN_ID = "dev.balloon_order_run_id", Some("MOLY_BALLOON_ORDER_RUN_ID"), Text, "developer window",
        "A run id written into the layer-order probe's first log line together with the version. Unset: the line is not written.";
    DEV_BALLOON_ORDER_SECS = "dev.balloon_order_secs", Some("MOLY_BALLOON_ORDER_SECS"), Text, "developer window",
        "Spawns a pair of overlapping probe balloons that leave after this many seconds to check balloon layer order (headless acceptance). Read by moly_world::balloon; unset or 0: no probes.";
    DEV_BGM_SELECT_TAPS = "dev.bgm_select_taps", Some("MOLY_BGM_SELECT_TAPS"), Text, "developer window",
        "Comma-separated `<seconds>=<target>` taps on the BGM select screen (`x:y`, `back`, `status`, `vocal`, `cell:<n>`); unset: no taps.";
    DEV_BIRTHDAY_CLOSE_SECS = "dev.birthday_close_secs", Some("MOLY_BIRTHDAY_CLOSE_SECS"), Text, "developer window",
        "Seconds after the party's close confirm shows before its OK is tapped; unset: no tap.";
    DEV_BIRTHDAY_CONFIRM_SECS = "dev.birthday_confirm_secs", Some("MOLY_BIRTHDAY_CONFIRM_SECS"), Text, "developer window",
        "Seconds after the birthday confirm shows before its OK is tapped; unset: no tap.";
    DEV_BIRTHDAY_END_SECS = "dev.birthday_end_secs", Some("MOLY_BIRTHDAY_END_SECS"), Text, "developer window",
        "Seconds after start before the housing screen's end-party button is tapped once its UI shows it; unset: no tap.";
    DEV_BIRTHDAY_NOW_MS = "dev.birthday_now_ms", Some("MOLY_BIRTHDAY_NOW_MS"), Text, "server",
        "Fixes the server clock at this epoch millisecond, a positive integer, on the default document; any other value stops the start with a panic. Unset: the clock runs from the real time.";
    DEV_BIRTHDAY_PARTY_ACTION = "dev.birthday_party_action", Some("MOLY_BIRTHDAY_PARTY_ACTION"), Text, "developer window",
        "`<seconds>[:<fixture>]`: places the player beside a party fixture that many seconds after start and taps its birthday cut-scene button; unset: no press.";
    DEV_CAMERA_AUTOFPS_SECS = "dev.camera_autofps_secs", Some("MOLY_CAMERA_AUTOFPS_SECS"), Text, "developer window",
        "The FPS smoke hook's window in seconds: synthetic pinch and drag input is fed through the real camera dispatch (pinch in to FPS, drag inside FPS, pinch out) so the transitions can be read from the log. Unset or 0: no synthetic input.";
    DEV_CAMERA_AUTOGESTURE_SECS = "dev.camera_autogesture_secs", Some("MOLY_CAMERA_AUTOGESTURE_SECS"), Text, "developer window",
        "The gesture smoke script's window in seconds: in the Normal camera state four phases drive pitch and distance to their limits through the real camera rules; the script stays off while the FPS hook is on. Unset or 0: no script.";
    DEV_CAMERA_AUTORESET = "dev.camera_autoreset", Some("MOLY_CAMERA_AUTORESET"), Text, "developer window",
        "Instrument `<state>:<seconds>`: once per entry into that camera state, the camera reset runs after `seconds` as the reset button runs it. A value without both parts is ignored. Unset: no automatic reset.";
    DEV_CONTROL_PORT = "dev.control_port", Some("MOLY_CONTROL_PORT"), Text, "developer window",
        "Opens the control surface's HTTP endpoint on that port at 127.0.0.1; unset: no HTTP endpoint; anything but a port number stops the start.";
    DEV_CONVERT_MOCK_REQUEST = "dev.convert_mock_request", Some("MOLY_CONVERT_MOCK_REQUEST"), Text, "server",
        "Answers a native convert request: `start:slot:item:count[:choice]`, `receive:slot:count` or `shorten:slot:price`; a malformed value stops the start with a panic. Unset: no convert request is made.";
    DEV_CRAFT_APPLY_SECS = "dev.craft_apply_secs", Some("MOLY_CRAFT_APPLY_SECS"), Text, "developer window",
        "Seconds after the craft screen opens before the apply button is pressed; unset: no press.";
    DEV_CRAFT_CONFIRM_SECS = "dev.craft_confirm_secs", Some("MOLY_CRAFT_CONFIRM_SECS"), Text, "developer window",
        "Seconds after the craft confirm shows before its OK is tapped; unset: no tap.";
    DEV_CRAFT_MOCK_MATERIALS = "dev.craft_mock_materials", Some("MOLY_CRAFT_MOCK_MATERIALS"), Text, "server",
        "Sets `userMysekaiMaterials` rows from comma-separated `id:quantity` entries (an id of at least 1, a quantity of at least 0); a malformed entry stops the start with a panic. Unset: the document's own materials.";
    DEV_CRAFT_MOCK_REQUEST = "dev.craft_mock_request", Some("MOLY_CRAFT_MOCK_REQUEST"), Text, "server",
        "Makes the home action's craft request a craft of `blueprintId[:quantity[:textureId[:blueprintTermId]]]`; a malformed value stops the start with a panic. Unset: the home action's request is answered as sent.";
    DEV_CRAFT_RESULT_CLOSE_SECS = "dev.craft_result_close_secs", Some("MOLY_CRAFT_RESULT_CLOSE_SECS"), Text, "developer window",
        "Seconds after the craft result dialog shows before a tap on its close area; unset: no tap.";
    DEV_CRAFT_SELECT = "dev.craft_select", Some("MOLY_CRAFT_SELECT"), Text, "developer window",
        "`blueprint[xcount]`: the craft selection made when the craft screen opens; unset: no selection.";
    DEV_CRAFT_STAGE_RESULT = "dev.craft_stage_result", Some("MOLY_CRAFT_STAGE_RESULT"), Text, "developer window",
        "`<count>[,first]`: stages the result dialogs for the selected blueprint without a craft request; unset: no staging.";
    DEV_CRAFT_TAP_APPLY = "dev.craft_tap_apply", Some("MOLY_CRAFT_TAP_APPLY"), Text, "developer window",
        "Taps the craft screen's apply button three seconds after it opens when set; unset: no tap.";
    DEV_CRAFT_TAP_CELL = "dev.craft_tap_cell", Some("MOLY_CRAFT_TAP_CELL"), Text, "developer window",
        "The recycled cell slot tapped two seconds after the craft screen opens; unset: none.";
    DEV_CRAFT_TAP_TAB = "dev.craft_tap_tab", Some("MOLY_CRAFT_TAP_TAB"), Text, "developer window",
        "The tab cell index tapped at its drawn centre one second after the craft screen opens; unset: none.";
    DEV_DELIVERY_AUTOPLAY = "dev.delivery_autoplay", Some("MOLY_DELIVERY_AUTOPLAY"), Text, "developer window",
        "Instrument: `hold[,close]` in seconds walks the player to the delivery drops and into the place circle, holds the delivery button `hold` seconds and closes each awaited dialog after `close` seconds (default 1.5). Unset: no autoplay.";
    DEV_DIRECTOR_SCRIPT = "dev.director_script", Some("MOLY_DIRECTOR_SCRIPT"), Path, "developer window",
        "Queues this director script file when a native run starts; unset: no script is queued.";
    DEV_DOOR_WALK_AFTER = "dev.door_walk_after", Some("MOLY_DOOR_WALK_AFTER"), Text, "developer window",
        "The seconds of app time the door walk waits before it starts; unset: it starts at once.";
    DEV_DOOR_WALK_OFFSET = "dev.door_walk_offset", Some("MOLY_DOOR_WALK_OFFSET"), Text, "developer window",
        "`x,z` metres the door walk's goal is shifted by; unset: the goal is the door itself.";
    DEV_DOOR_WALK_SECS = "dev.door_walk_secs", Some("MOLY_DOOR_WALK_SECS"), Text, "developer window",
        "Walks the player to the site's door until that many seconds of app time and taps the door's button; unset or zero: no door walk.";
    DEV_EDIT_AUTOPLAY = "dev.edit_autoplay", Some("MOLY_EDIT_AUTOPLAY"), Text, "developer window",
        "The seconds after the joystick opens before the edit autoplay sends one layout edit (an empty value means 3); unset: no autoplay.";
    DEV_EDIT_AUTOPLAY_CATALOG = "dev.edit_autoplay_catalog", Some("MOLY_EDIT_AUTOPLAY_CATALOG"), Text, "developer window",
        "Comma-separated catalog indices the edit autoplay puts (a trailing `d` also decides the put); unset: `2,0`.";
    DEV_EDIT_AUTOPLAY_CLEAN_UP = "dev.edit_autoplay_clean_up", Some("MOLY_EDIT_AUTOPLAY_CLEAN_UP"), Text, "developer window",
        "Presses the remove-all button and its confirmation before saving; unset: nothing is removed.";
    DEV_EDIT_AUTOPLAY_CLEAN_UP_HOLD = "dev.edit_autoplay_clean_up_hold", Some("MOLY_EDIT_AUTOPLAY_CLEAN_UP_HOLD"), Text, "developer window",
        "Seconds the edit autoplay waits with the remove-all confirmation open before its clean-up button; unset: no hold.";
    DEV_EDIT_AUTOPLAY_HOLD = "dev.edit_autoplay_hold", Some("MOLY_EDIT_AUTOPLAY_HOLD"), Text, "developer window",
        "Seconds the edit autoplay idles after the camera buttons with nothing selected, so an outside drag reaches the edit camera; unset: no hold.";
    DEV_EDIT_AUTOPLAY_RETURN_BASE = "dev.edit_autoplay_return_base", Some("MOLY_EDIT_AUTOPLAY_RETURN_BASE"), Text, "developer window",
        "Picks the stack run's base fixture once more and sends it to storage before saving; unset: it is left in place.";
    DEV_EDIT_AUTOPLAY_STACK = "dev.edit_autoplay_stack", Some("MOLY_EDIT_AUTOPLAY_STACK"), Text, "developer window",
        "`<uid>,<uid>`: the fixture the stack run picks first and the fixture it is dragged onto; unset: the usual pick.";
    DEV_EDIT_AUTOPLAY_WAIT_TWEET = "dev.edit_autoplay_wait_tweet", Some("MOLY_EDIT_AUTOPLAY_WAIT_TWEET"), Text, "developer window",
        "The edit autoplay waits (up to 120 s) for a visible tweet balloon before the edit button; unset: it enters at once.";
    DEV_EFFECT_SAMPLE = "dev.effect_sample", Some("MOLY_EFFECT_SAMPLE"), Text, "developer window",
        "Also samples each system's live put-effect particles every 0.25 s after a put; unset: no samples.";
    DEV_EFFECT_TRACE = "dev.effect_trace", Some("MOLY_EFFECT_TRACE"), Text, "developer window",
        "Traces each effect instance's draws every frame; unset: no trace.";
    DEV_EMOTICON_ITEMS = "dev.emoticon_items", Some("MOLY_EMOTICON_ITEMS"), Text, "developer window",
        "A showcase carousel of emotes: comma-separated item names from the archive, refused with a panic when a name is not in the archive or the list is empty. Unset: no carousel.";
    DEV_EMOTICON_SHOW_SECS = "dev.emoticon_show_secs", Some("MOLY_EMOTICON_SHOW_SECS"), Text, "developer window",
        "The seconds each showcase emote is shown. Unset or not a number: 6.";
    DEV_FFMPEG = "dev.ffmpeg", Some("MOLY_FFMPEG"), Path, "developer window",
        "The ffmpeg program a recording encodes and mixes its video with. Unset: `ffmpeg`, found on `PATH`.";
    DEV_FIXTURE_CLOCK_NOW = "dev.fixture_clock_now", Some("MOLY_FIXTURE_CLOCK_NOW"), Text, "developer window",
        "Pins the fixture clock's local time to `HH:MM` on native builds so clock gimmicks can be verified at a known hour; a malformed value is refused with a panic. Unset: the device's local clock.";
    DEV_FIXTURE_DIRECTION = "dev.fixture_direction", Some("MOLY_FIXTURE_DIRECTION"), Text, "developer window",
        "Smoke knob that rewrites the facing of every placed fixture to 0=Front, 1=Left, 2=Back or 3=Right; a value outside 0..3 or not an integer is refused with a panic. Unset or empty: the table's own Front facing is kept.";
    DEV_FIXTURE_PREVIEW = "dev.fixture_preview", Some("MOLY_FIXTURE_PREVIEW"), Text, "developer window",
        "The developer model preview to add to the placement table, `surface-<n>` or `gimmick-<n>` in developer builds. Unset: no preview.";
    DEV_FIXTURE_WALK_AFTER = "dev.fixture_walk_after", Some("MOLY_FIXTURE_WALK_AFTER"), Text, "developer window",
        "The seconds of app time the fixture walk waits before it starts; unset: it starts at once.";
    DEV_FIXTURE_WALK_HOLD = "dev.fixture_walk_hold", Some("MOLY_FIXTURE_WALK_HOLD"), Text, "developer window",
        "The seconds the played fixture timeline runs before the walk pushes the joystick to end it; unset: 8 seconds.";
    DEV_FIXTURE_WALK_ID = "dev.fixture_walk_id", Some("MOLY_FIXTURE_WALK_ID"), Text, "developer window",
        "The id of the placed fixture the walk targets; unset: the nearest placed fixture with a player timeline row.";
    DEV_FIXTURE_WALK_SECS = "dev.fixture_walk_secs", Some("MOLY_FIXTURE_WALK_SECS"), Text, "developer window",
        "Arms the fixture walk until that many seconds of app time; unset or zero: the walk never starts.";
    DEV_FIXTURE_WALK_TAP = "dev.fixture_walk_tap", Some("MOLY_FIXTURE_WALK_TAP"), Text, "developer window",
        "The `x,y` window position of the timeline button the fixture walk taps; unset: the walk stops without tapping.";
    DEV_FIXTURE_WALK_VIA = "dev.fixture_walk_via", Some("MOLY_FIXTURE_WALK_VIA"), Text, "developer window",
        "`x,z;x,z` waypoints the fixture walk passes in order before the target; unset: it walks straight to the target.";
    DEV_GATE_CHANGE = "dev.gate_change", Some("MOLY_GATE_CHANGE"), Text, "developer window",
        "`<gateId>[/<skinId>]:<group>,...`: makes the gate change request with those unit groups; unset: no request from the instrument.";
    DEV_GATE_INVITE = "dev.gate_invite", Some("MOLY_GATE_INVITE"), Text, "developer window",
        "`<unit>[,pass]`: makes the gate's reserve request for that unit (optionally granting a pass first); unset: no request from the instrument.";
    DEV_GET_RESOURCE_AUTOCLOSE_SECS = "dev.get_resource_autoclose_secs", Some("MOLY_GET_RESOURCE_AUTOCLOSE_SECS"), Text, "developer window",
        "Seconds after a get-resource dialog shows before a tap on its close area; unset: no tap.";
    DEV_GET_RESOURCE_NOTICE = "dev.get_resource_notice", Some("MOLY_GET_RESOURCE_NOTICE"), Text, "developer window",
        "`type:id:qty[,...]`: calls NoticeCollectItem for each once the get-resource inputs are ready; unset: none.";
    DEV_GET_RESOURCE_NOTICE_SECS = "dev.get_resource_notice_secs", Some("MOLY_GET_RESOURCE_NOTICE_SECS"), Text, "developer window",
        "Seconds after the inputs are ready before the get-resource instrument fires; unset: 0.";
    DEV_GET_RESOURCE_REWARD_DIALOGS = "dev.get_resource_reward_dialogs", Some("MOLY_GET_RESOURCE_REWARD_DIALOGS"), Text, "developer window",
        "Comma-separated reward dialog ids (251, 263, 452) shown as their delivery caller shows them; unset: none.";
    DEV_GET_RESOURCE_SKETCH = "dev.get_resource_sketch", Some("MOLY_GET_RESOURCE_SKETCH"), Text, "developer window",
        "The blueprint id whose sketch result dialog is shown once the inputs are ready; unset: none.";
    DEV_HARVEST_AUTOPLAY = "dev.harvest_autoplay", Some("MOLY_HARVEST_AUTOPLAY"), Text, "developer window",
        "Instrument: a comma-separated plan of harvest kinds (`wood`, `mineral`, `plant`, `other`, `toolbox`, `driftage`, each optionally `kind:N` for N swings); it walks to each object, presses the action button and gathers the drops, writing the player's own inputs. Unset: no autoplay.";
    DEV_HARVEST_MOCK_MATERIAL_POSSESSION = "dev.harvest_mock_material_possession", Some("MOLY_HARVEST_MOCK_MATERIAL_POSSESSION"), Text, "developer window",
        "The material quantity the harvest possession mock seats until a reply carries one; unset: 0.";
    DEV_HARVEST_MOCK_PHENOMENA = "dev.harvest_mock_phenomena", Some("MOLY_HARVEST_MOCK_PHENOMENA"), Text, "server",
        "States the document's learned phenomena from comma-separated `mysekaiPhenomenaId:obtainedAt` entries; a malformed entry stops the start with a panic. Unset: the document's own rows.";
    DEV_HARVEST_MOCK_TOOLS = "dev.harvest_mock_tools", Some("MOLY_HARVEST_MOCK_TOOLS"), Text, "server",
        "States the document's tools from comma-separated `toolId:quantity:durability` entries; a malformed entry stops the start with a panic. Unset: the document's own tools.";
    DEV_HARVEST_SUMMARY_MOCK_PLAYER_TOTAL_EXP = "dev.harvest_summary_mock_player_total_exp", Some("MOLY_HARVEST_SUMMARY_MOCK_PLAYER_TOTAL_EXP"), Text, "developer window",
        "The mock player total experience the harvest summary panel starts from; a value that is not an int is logged and ignored. Unset: 100000.";
    DEV_HARVEST_SUMMARY_MOCK_TRIP_EXP = "dev.harvest_summary_mock_trip_exp", Some("MOLY_HARVEST_SUMMARY_MOCK_TRIP_EXP"), Text, "developer window",
        "The mock experience the server's replies add over one harvest trip; a value that is not an int is logged and ignored. Unset: 120.";
    DEV_HOME_ACTION_AUTOPLAY = "dev.home_action_autoplay", Some("MOLY_HOME_ACTION_AUTOPLAY"), Text, "developer window",
        "`craft|canvas|sketch|screen`: walks to the nearest craft tool and runs that home action; unset: no autoplay.";
    DEV_HOME_ACTION_AUTOPLAY_AFTER = "dev.home_action_autoplay_after", Some("MOLY_HOME_ACTION_AUTOPLAY_AFTER"), Text, "developer window",
        "Seconds after the joystick opens before the home action autoplay starts; unset: 2.";
    DEV_HOME_ACTION_AUTOPLAY_SKIP = "dev.home_action_autoplay_skip", Some("MOLY_HOME_ACTION_AUTOPLAY_SKIP"), Text, "developer window",
        "Seconds after the home action request before skip is pressed, when that long into the sequence; unset: no skip.";
    DEV_HOME_ACTION_SKETCH_BLUEPRINT = "dev.home_action_sketch_blueprint", Some("MOLY_HOME_ACTION_SKETCH_BLUEPRINT"), Text, "developer window",
        "The blueprint id the home action autoplay sketches; unset: no target.";
    DEV_INFO_MOCK_ACCESS_PERMISSION = "dev.info_mock_access_permission", Some("MOLY_INFO_MOCK_ACCESS_PERMISSION"), Text, "developer window",
        "The info panel's mock access permission (all/friend_only/reject/review); unset: all.";
    DEV_INFO_MOCK_FIXTURE_JOINT_PUT_LIMIT = "dev.info_mock_fixture_joint_put_limit", Some("MOLY_INFO_MOCK_FIXTURE_JOINT_PUT_LIMIT"), Text, "developer window",
        "The info panel's mock joint fixture put limit; unset: 10.";
    DEV_INFO_MOCK_FIXTURE_PUT_LIMIT = "dev.info_mock_fixture_put_limit", Some("MOLY_INFO_MOCK_FIXTURE_PUT_LIMIT"), Text, "developer window",
        "The info panel's mock fixture put limit; unset: 20.";
    DEV_INVENTORY_RECYCLE_CONFIRM_SECS = "dev.inventory_recycle_confirm_secs", Some("MOLY_INVENTORY_RECYCLE_CONFIRM_SECS"), Text, "developer window",
        "Seconds after the inventory recycle confirmation shows before its OK is tapped; unset: no tap.";
    DEV_JOYSTICK_AUTOSMOKE_SECS = "dev.joystick_autosmoke_secs", Some("MOLY_JOYSTICK_AUTOSMOKE_SECS"), Text, "developer window",
        "Injects the scripted joystick touch cycle until that many seconds of app time; unset or zero: no injection.";
    DEV_LEARN_PHENOMENA_AUTOTAP_SECS = "dev.learn_phenomena_autotap_secs", Some("MOLY_LEARN_PHENOMENA_AUTOTAP_SECS"), Text, "developer window",
        "Seconds after the learn-phenomena open animation before the thumbnail and then the close area are tapped; unset: no taps.";
    DEV_LIBRARY_OPEN = "dev.library_open", Some("MOLY_LIBRARY_OPEN"), Text, "developer window",
        "Opens the content library when its talks are ready; unset: it stays closed.";
    DEV_LIBRARY_QA_OUT = "dev.library_qa_out", Some("MOLY_LIBRARY_QA_OUT"), Path, "developer window",
        "Writes the content library's QA telemetry JSON to this file every 0.5 s; unset: no file is written (the browser export still works).";
    DEV_MENU_MOCK_CRYSTAL_SHOP_ALLOWED = "dev.menu_mock_crystal_shop_allowed", Some("MOLY_MENU_MOCK_CRYSTAL_SHOP_ALLOWED"), Text, "developer window",
        "Whether the menu's mock allows the crystal shop; unset: true.";
    DEV_MENU_MOCK_SKETCH_AVAILABLE = "dev.menu_mock_sketch_available", Some("MOLY_MENU_MOCK_SKETCH_AVAILABLE"), Text, "developer window",
        "Whether the menu's mock makes sketch available; unset: true.";
    DEV_MENU_MOCK_STAMINA_BOOST = "dev.menu_mock_stamina_boost", Some("MOLY_MENU_MOCK_STAMINA_BOOST"), Text, "server",
        "Overlays this boost stamina pool on the default document's `userMysekaiStamina`. Unset: the document's own value.";
    DEV_MENU_MOCK_STAMINA_ENHANCE = "dev.menu_mock_stamina_enhance", Some("MOLY_MENU_MOCK_STAMINA_ENHANCE"), Text, "server",
        "Overlays this enhance stamina pool on the default document's `userMysekaiStamina`. Unset: the document's own value.";
    DEV_MENU_MOCK_STAMINA_MAX = "dev.menu_mock_stamina_max", Some("MOLY_MENU_MOCK_STAMINA_MAX"), Text, "server",
        "Not applied: the stamina gauge maximum is the master `maxStamina`; when set, a warning says so. Unset: no warning.";
    DEV_MENU_MOCK_STAMINA_NORMAL = "dev.menu_mock_stamina_normal", Some("MOLY_MENU_MOCK_STAMINA_NORMAL"), Text, "server",
        "Overlays this normal stamina pool on the default document's `userMysekaiStamina`. Unset: the document's own value.";
    DEV_MENU_MOCK_TOTAL_EXP = "dev.menu_mock_total_exp", Some("MOLY_MENU_MOCK_TOTAL_EXP"), Text, "server",
        "Overlays this total experience on the default document's `userMysekaiGamedata.totalExp` and clears its rank, so the rank follows the experience; a value that is not an integer is logged and the document keeps its value. Unset: the document's own value.";
    DEV_MENU_MOCK_VISITING = "dev.menu_mock_visiting", Some("MOLY_MENU_MOCK_VISITING"), Text, "developer window",
        "The menu's mock visiting state (true/1 or false/0); unset: false.";
    DEV_MUSIC_MOCK_OWNED_RECORDS = "dev.music_mock_owned_records", Some("MOLY_MUSIC_MOCK_OWNED_RECORDS"), Text, "server",
        "The music records the mock user owns, comma-separated record ids obtained at the server clock; an id that is not an integer, or named twice, stops the start with a panic. Unset: the user owns no record.";
    DEV_NOTICE_COLLECT_AUTOPLAY = "dev.notice_collect_autoplay", Some("MOLY_NOTICE_COLLECT_AUTOPLAY"), Text, "developer window",
        "How many scripted NoticeCollectItem calls the collect notice sends two seconds after its cells are ready (a non-number sends all); unset: none.";
    DEV_NPC_ENTRY_PROBE = "dev.npc_entry_probe", Some("MOLY_NPC_ENTRY_PROBE"), Text, "developer window",
        "A `;`-separated script of gate and cut-scene entry steps (`SECONDS:OP[:UNIT[:ARG]]`) driven in developer builds, one log line per step. Unset: no steps run.";
    DEV_NPC_HARNESS_DIR = "dev.npc_harness_dir", Some("MOLY_NPC_HARNESS_DIR"), Path, "developer window",
        "The acceptance harness directory: cases are read from `<dir>/in/<name>.json` and results written to `<dir>/out/<name>.json`. The harness drivers stop with a panic naming the key when it is unset.";
    DEV_NPC_HFSM_SCOPE = "dev.npc_hfsm_scope", Some("MOLY_NPC_HFSM_SCOPE"), Text, "developer window",
        "The units whose NPC machine the controller drives (unit ids separated by commas or spaces, or `all`). Unset: the controller has no scope; on a jp source of version 7 or later it enables itself for every unit.";
    DEV_NPC_HFSM_TRACE = "dev.npc_hfsm_trace", Some("MOLY_NPC_HFSM_TRACE"), Text, "developer window",
        "Writes the NPC machine's per-decision, per-transition and per-recovery log lines; `1`, `true`, `yes` or `on` switch them on. Unset or any other value: only the constructor and report lines.";
    DEV_OC_MODEL = "dev.oc_model", Some("MOLY_OC_MODEL"), Path, "developer window",
        "A VRM or glTF model file the player wears from the start on native builds; an empty value is ignored. Unset: the player keeps the server's avatar body.";
    DEV_OC_UNIT = "dev.oc_unit", Some("MOLY_OC_UNIT"), Text, "developer window",
        "The unit id of the character body the player wears from the start on native builds; a value that is not a unit id is refused and logged. Unset: the server's avatar body.";
    DEV_OFFLINE_HOME_LEVEL = "dev.offline_home_level", Some("MOLY_OFFLINE_HOME_LEVEL"), Text, "developer window",
        "The home site's offline level a native start uses; unset: the site's saved level.";
    DEV_OPTION_MOCK_USER_CONFIG = "dev.option_mock_user_config", Some("MOLY_OPTION_MOCK_USER_CONFIG"), Text, "developer window",
        "`<true|false>,<all|id_search|reject>`: the mock user config the option screen reads; unset: the user config is null.";
    DEV_PARTVOICE_PROBE_SECS = "dev.partvoice_probe_secs", Some("MOLY_PARTVOICE_PROBE_SECS"), Text, "developer window",
        "After this many seconds of app time a one-shot composed part-voice segment is injected through the real talk chain to exercise the playback, missing-cue, fixture and gate-reject arms; a value that is not a number is warned about and turns the probe off. Unset: no injection.";
    DEV_PICK_NPC_TAP_SECS = "dev.pick_npc_tap_secs", Some("MOLY_PICK_NPC_TAP_SECS"), Text, "developer window",
        "Injects a tap on each present character's screen position in turn until that many seconds of app time; unset: no taps.";
    DEV_PLAYER_AUTOWALK_DIAGONAL = "dev.player_autowalk_diagonal", Some("MOLY_PLAYER_AUTOWALK_DIAGONAL"), Text, "developer window",
        "When positive, the autowalk direction is the normalized diagonal instead of +X; unset: +X.";
    DEV_PLAYER_AUTOWALK_SECS = "dev.player_autowalk_secs", Some("MOLY_PLAYER_AUTOWALK_SECS"), Text, "developer window",
        "Composes a fixed walk (then dash) input for that many seconds after startup; unset: no autowalk.";
    DEV_PLAYER_TALK_AUTOTAP_SECS = "dev.player_talk_autotap_secs", Some("MOLY_PLAYER_TALK_AUTOTAP_SECS"), Text, "developer window",
        "Instrument: for `<until>` or `<from>:<until>` seconds of app time, a synthetic tap on an on-screen NPC with a general-talk candidate is fed through the real pick and talk chain every few seconds. Unset or not a number: no taps.";
    DEV_PLAYER_TALK_STACK_TAP_SECS = "dev.player_talk_stack_tap_secs", Some("MOLY_PLAYER_TALK_STACK_TAP_SECS"), Text, "developer window",
        "A window (`<until>` or `<from>:<until>` seconds) in which the stack head's Talk button request is written for the character under it; unset: the probe is off.";
    DEV_PROCEDURAL_DEBUG = "dev.procedural_debug", Some("MOLY_PROCEDURAL_DEBUG"), Text, "developer window",
        "Prints the first particle whose position, velocity or axis differs from the native row; unset: no debug output.";
    DEV_RANK_CHANGE = "dev.rank_change", Some("MOLY_RANK_CHANGE"), Text, "developer window",
        "`<prev>,<current>`: delivers a rank change before the first site loads; unset: no rank change.";
    DEV_RECORD_AUDIO = "dev.record_audio", Some("MOLY_RECORD_AUDIO"), Flag, "developer window",
        "Starts without an audio card: the game's mix is rendered by the frame loop instead and recorded with the frames; read at start. Unset or false: the sound card plays the mix and recordings have no sound.";
    DEV_ROOM_LEVEL = "dev.room_level", Some("MOLY_ROOM_LEVEL"), Text, "developer window",
        "The room level a native start uses; unset: the site's saved level.";
    DEV_SCENE_CONTENT = "dev.scene_content", Some("MOLY_SCENE_CONTENT"), Text, "developer window",
        "`compact` or `full`: how much of a site's content is loaded at a native start; unset: compact.";
    DEV_SCREENSHOT_DIR = "dev.screenshot_dir", Some("MOLY_SCREENSHOT_DIR"), Path, "developer window",
        "The directory a requested frame capture is written to. Unset: captures go to `screenshots`.";
    DEV_SEED = "dev.seed", Some("MOLY_SEED"), Text, "developer window",
        "The world seed, a decimal u64: every random stream draws a fixed sequence from it, so a run can be repeated. Unset: each draw takes fresh platform random bytes.";
    DEV_SHADOW_PROBE_INTERVAL = "dev.shadow_probe_interval", Some("MOLY_SHADOW_PROBE_INTERVAL"), Text, "developer window",
        "The frames between shadow-map depth accounts on native (at least 1); unset: no native accounts run.";
    DEV_SILHOUETTE_PROBE_INTERVAL = "dev.silhouette_probe_interval", Some("MOLY_SILHOUETTE_PROBE_INTERVAL"), Text, "developer window",
        "The native interval, in replayed frames, between silhouette stencil read-backs; the first read-back comes on frame 10 and a value of 0 is ignored. Unset: no native read-back.";
    DEV_SITE = "dev.site", Some("MOLY_SITE"), Text, "developer window",
        "The site type a native start enters; unset: the saved preferred site, else the home site.";
    DEV_SITEMAP_AUTOCLICK_SECS = "dev.sitemap_autoclick_secs", Some("MOLY_SITEMAP_AUTOCLICK_SECS"), Text, "developer window",
        "Seconds before the site map instrument opens the map and taps a site's icon; unset: no auto click.";
    DEV_SITEMAP_AUTOCLICK_SITE = "dev.sitemap_autoclick_site", Some("MOLY_SITEMAP_AUTOCLICK_SITE"), Text, "developer window",
        "The site type the site map instrument taps; unset: no auto click.";
    DEV_SITEMAP_AUTO_UNLOCK_SECS = "dev.sitemap_auto_unlock_secs", Some("MOLY_SITEMAP_AUTO_UNLOCK_SECS"), Text, "developer window",
        "Seconds before the site map instrument runs the first locked icon's open animation once; unset: nothing.";
    DEV_SITE_EXTENSION_DEV = "dev.site_extension_dev", Some("MOLY_SITE_EXTENSION_DEV"), Text, "developer window",
        "Plays one site expansion clip from these `;`-separated `key=value` pairs through the developer hook; unset: the hook plays nothing.";
    DEV_SITE_MOVE_LANDING_DRAW = "dev.site_move_landing_draw", Some("MOLY_SITE_MOVE_LANDING_DRAW"), Text, "developer window",
        "Replaces the site move's landing draw with this value (the failed landing is one of the draws); unset: a random draw.";
    DEV_SITE_TOUR_ROUTE = "dev.site_tour_route", Some("MOLY_SITE_TOUR_ROUTE"), Text, "developer window",
        "The smoke tour's route: a comma-separated list of site types visited in that order instead of the master-row order; an unsupported site type stops the start with a panic. Unset: master-row order.";
    DEV_SITE_TOUR_SECS = "dev.site_tour_secs", Some("MOLY_SITE_TOUR_SECS"), Text, "developer window",
        "The smoke-tour instrument's dwell time in seconds: the game loads each site in turn unattended and exits cleanly after the last one; a value of 0 or not a number counts as off. Unset: no tour.";
    DEV_STEP_ITEM = "dev.step_item", Some("MOLY_STEP_ITEM"), Text, "developer window",
        "`<bundle>|<file>` of a step item: once the avatar is wired the step item sequence runs once to check the service alone; a value without `|` is logged as an error. Unset: no sequence.";
    DEV_TALK_AUTO_ADVANCE = "dev.talk_auto_advance", Some("MOLY_TALK_AUTO_ADVANCE"), Text, "developer window",
        "Set to `1`: a talk line advances as soon as its own voice has played out, without a tap, for unattended recordings. Any other value or unset: a tap advances the line.";
    DEV_TALK_TAP_AT_SECS = "dev.talk_tap_at_secs", Some("MOLY_TALK_TAP_AT_SECS"), Text, "developer window",
        "Smoke input: a comma-separated list of app times in seconds at which a tap on the talk window is set synthetically; parts that are not numbers are skipped. Unset or empty: no taps.";
    DEV_TALK_VOICE_PROBE_LINES = "dev.talk_voice_probe_lines", Some("MOLY_TALK_VOICE_PROBE_LINES"), Text, "developer window",
        "The number of voice lines in the synthetic talk segment; a value that is not a positive integer is warned about and the default is used. Unset: 3.";
    DEV_TALK_VOICE_PROBE_SECS = "dev.talk_voice_probe_secs", Some("MOLY_TALK_VOICE_PROBE_SECS"), Text, "developer window",
        "After this many seconds of app time a one-shot synthetic talk segment with voice lines is injected through the real talk chain, once no conversation plays; a value that is not a number is warned about and turns the probe off. Unset: no injection.";
    DEV_VISITOR_UNITS = "dev.visitor_units", Some("MOLY_VISITOR_UNITS"), Text, "developer window",
        "Replaces the roster of visitors the home site boots with: a comma-separated list of unit ids; parts that are not numbers are skipped, and an empty list leaves the gate rows. Unset: the roster comes from the gate rows.";
    DEV_WALK_BAKE_THREADS = "dev.walk_bake_threads", Some("MOLY_WALK_BAKE_THREADS"), Text, "developer window",
        "How many row bands the walk-field collider bake may spread over, clamped to 1..8 (0 and 1 keep it on one thread); a value that is not a number counts as unset. Unset: the machine's available parallelism, capped at 8.";
    DEV_WALK_PROBE = "dev.walk_probe", Some("MOLY_WALK_PROBE"), Text, "developer window",
        "QA probe: for every walk-field generation, logs the door locators' static path queries and the agent path for each `from>to` pair given here (x,z coordinates). Unset: no probe.";
    DEV_WINDOW_HIDDEN = "dev.window_hidden", Some("MOLY_WINDOW_HIDDEN"), Text, "developer window",
        "Starts a native run with the window hidden for headless batches; any value hides the window. Unset: the window is shown.";
}

/// Finds a declared key by its path.
pub fn key(path: &str) -> Option<&'static Key> {
    KEYS.iter().copied().find(|key| key.path == path)
}

/// Where a value came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Env(&'static str),
    File(Option<PathBuf>),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env(name) => write!(f, "environment variable {name}"),
            Self::File(Some(path)) => write!(f, "{}", path.display()),
            Self::File(None) => f.write_str("the configuration"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// The file exists but cannot be read or written.
    Io { path: PathBuf, message: String },
    /// The file is not valid TOML.
    Parse { path: Option<PathBuf>, message: String },
    /// A key, or a value outside a section, that is not declared.
    Unknown { path: Option<PathBuf>, key: String },
    /// A value of the wrong kind.
    Kind { key: &'static str, expected: Kind, source: Source },
    /// A required key with no value.
    Missing { key: &'static str, env: Option<&'static str> },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Parse { path: Some(path), message } => write!(f, "{}: {message}", path.display()),
            Self::Parse { path: None, message } => f.write_str(message),
            Self::Unknown { path, key } => {
                let path = path.as_deref().map_or("moly.toml".into(), |path| path.display().to_string());
                write!(f, "{path}: `{key}` is not a configuration key")
            }
            Self::Kind { key, expected, source } => {
                write!(f, "`{key}` in {source} must be {}", expected.name())
            }
            Self::Missing { key, env } => {
                let (section, name) = key.split_once('.').unwrap_or(("", key));
                write!(f, "`{key}` is not configured: set `{name}` in the [{section}] section of moly.toml")?;
                match env {
                    Some(env) => write!(f, " or the environment variable {env}"),
                    None => Ok(()),
                }
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// A loaded configuration file. An empty one (no file) still reads the
/// environment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    path: Option<PathBuf>,
    values: BTreeMap<&'static str, toml::Value>,
}

impl Config {
    /// No file: every value comes from the environment or the caller.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Reads `path`. A missing file is an empty configuration that remembers
    /// the path.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text, Some(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                path: Some(path.to_owned()),
                values: BTreeMap::new(),
            }),
            Err(error) => Err(ConfigError::Io { path: path.to_owned(), message: error.to_string() }),
        }
    }

    /// Parses and checks the text of a file.
    pub fn parse(text: &str, path: Option<&Path>) -> Result<Self, ConfigError> {
        let path = path.map(Path::to_owned);
        let table: toml::Table = text
            .parse()
            .map_err(|error: toml::de::Error| ConfigError::Parse { path: path.clone(), message: error.to_string() })?;
        let mut values = BTreeMap::new();
        for (section, entries) in table {
            let toml::Value::Table(entries) = entries else {
                return Err(ConfigError::Unknown { path, key: section });
            };
            for (name, value) in entries {
                let full = format!("{section}.{name}");
                let Some(key) = key(&full) else {
                    return Err(ConfigError::Unknown { path, key: full });
                };
                if !fits(key.kind, &value) {
                    return Err(ConfigError::Kind {
                        key: key.path,
                        expected: key.kind,
                        source: Source::File(path),
                    });
                }
                values.insert(key.path, value);
            }
        }
        Ok(Self { path, values })
    }

    /// The file this configuration was loaded from.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The key's value as text (`Text`, `Path` and `Url` keys), from the
    /// environment, else the file.
    pub fn text(&self, key: &'static Key) -> Option<String> {
        self.text_in(key, &process_env)
    }

    /// [`Self::text`], refusing with the key's name and where to set it when
    /// there is no value.
    pub fn require_text(&self, key: &'static Key) -> Result<String, ConfigError> {
        self.text(key).ok_or(ConfigError::Missing { key: key.path, env: key.env })
    }

    /// A `Flag` key: `1` or `0` in the environment, `true` or `false` in the
    /// file. Anything else is refused by name.
    pub fn flag(&self, key: &'static Key) -> Result<Option<bool>, ConfigError> {
        self.flag_in(key, &process_env)
    }

    /// An `Integer` key.
    pub fn integer(&self, key: &'static Key) -> Result<Option<i64>, ConfigError> {
        self.integer_in(key, &process_env)
    }

    /// Whether the file sets the key.
    pub fn has(&self, key: &Key) -> bool {
        self.values.contains_key(key.path)
    }

    /// The file's own value as text, ignoring the environment. For a program
    /// that edits the file: what it shows and saves back must be the file's.
    pub fn file_text(&self, key: &Key) -> Option<String> {
        match self.values.get(key.path)? {
            toml::Value::String(value) => Some(value.clone()),
            other => Some(other.to_string()),
        }
    }

    /// The file's own `Flag` value, ignoring the environment.
    pub fn file_flag(&self, key: &Key) -> Option<bool> {
        self.values.get(key.path).and_then(toml::Value::as_bool)
    }

    /// The file's own `Integer` value, ignoring the environment.
    pub fn file_integer(&self, key: &Key) -> Option<i64> {
        self.values.get(key.path).and_then(toml::Value::as_integer)
    }

    fn env_value(key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Option<(&'static str, String)> {
        let name = key.env?;
        env(name).map(|value| (name, value))
    }

    fn text_in(&self, key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        if let Some((_, value)) = Self::env_value(key, env) {
            return Some(value);
        }
        self.file_text(key)
    }

    fn flag_in(&self, key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Result<Option<bool>, ConfigError> {
        if let Some((name, value)) = Self::env_value(key, env).filter(|(_, value)| !value.is_empty()) {
            return match value.as_str() {
                "1" => Ok(Some(true)),
                "0" => Ok(Some(false)),
                _ => Err(ConfigError::Kind { key: key.path, expected: Kind::Flag, source: Source::Env(name) }),
            };
        }
        Ok(self.file_flag(key))
    }

    fn integer_in(&self, key: &'static Key, env: &dyn Fn(&str) -> Option<String>) -> Result<Option<i64>, ConfigError> {
        if let Some((name, value)) = Self::env_value(key, env) {
            return value
                .parse()
                .map(Some)
                .map_err(|_| ConfigError::Kind { key: key.path, expected: Kind::Integer, source: Source::Env(name) });
        }
        Ok(self.file_integer(key))
    }
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn fits(kind: Kind, value: &toml::Value) -> bool {
    match (kind, value) {
        (Kind::Text | Kind::Path, toml::Value::String(_)) => true,
        (Kind::Url, toml::Value::String(text)) => is_address(text),
        (Kind::Flag, toml::Value::Boolean(_)) => true,
        (Kind::Integer, toml::Value::Integer(_)) => true,
        _ => false,
    }
}

fn is_address(text: &str) -> bool {
    text.strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .is_some_and(|rest| !rest.is_empty())
}

static GLOBAL: OnceLock<Config> = OnceLock::new();

/// Makes `config` the program's configuration. Each program installs it once,
/// at start, before anything reads it; a second call is refused.
pub fn install(config: Config) -> Result<(), Config> {
    GLOBAL.set(config)
}

/// The program's configuration: the installed one, else an empty one that
/// reads the environment only (tests, and the browser build, which has no
/// file).
pub fn global() -> &'static Config {
    GLOBAL.get_or_init(Config::empty)
}

/// The value of `key` in the program's configuration: its environment
/// variable when set (even to an empty value), else the file's value as
/// text, else `None`. The same as `global().text(key)`.
pub fn text(key: &'static Key) -> Option<String> {
    global().text(key)
}

/// A value to write with [`set_values`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Flag(bool),
    Integer(i64),
}

/// Sets (`Some`) or removes (`None`) keys in the file at `path`, keeping its
/// comments and the order of what it already holds. The file is created when
/// it does not exist, and replaced in one step (written beside it, then
/// renamed). The result is checked like a loaded file before it is written.
pub fn set_values(path: &Path, values: &[(&'static Key, Option<Value>)]) -> Result<(), ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(ConfigError::Io { path: path.to_owned(), message: error.to_string() }),
    };
    let text = edit_text(&text, values, Some(path))?;
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| ConfigError::Io { path: parent.to_owned(), message: error.to_string() })?;
    }
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    std::fs::write(&part, text)
        .and_then(|()| std::fs::rename(&part, path))
        .map_err(|error| ConfigError::Io { path: path.to_owned(), message: error.to_string() })
}

fn edit_text(text: &str, values: &[(&'static Key, Option<Value>)], path: Option<&Path>) -> Result<String, ConfigError> {
    let mut document: toml_edit::DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| ConfigError::Parse { path: path.map(Path::to_owned), message: error.to_string() })?;
    for (key, value) in values {
        match value {
            Some(value) => {
                let section = document
                    .entry(key.section())
                    .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()));
                let Some(section) = section.as_table_like_mut() else {
                    return Err(ConfigError::Unknown { path: path.map(Path::to_owned), key: key.section().to_owned() });
                };
                let mut new = match value {
                    Value::Text(text) => toml_edit::Value::from(text.as_str()),
                    Value::Flag(flag) => toml_edit::Value::from(*flag),
                    Value::Integer(number) => toml_edit::Value::from(*number),
                };
                // A key already in the file keeps its comments and spacing;
                // only its value changes.
                if let Some(toml_edit::Item::Value(existing)) = section.get_mut(key.name()) {
                    *new.decor_mut() = existing.decor().clone();
                    *existing = new;
                } else {
                    section.insert(key.name(), toml_edit::Item::Value(new));
                }
            }
            None => {
                if let Some(section) = document.get_mut(key.section()).and_then(toml_edit::Item::as_table_like_mut) {
                    section.remove(key.name());
                }
            }
        }
    }
    let text = document.to_string();
    Config::parse(&text, path)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    #[test]
    fn every_key_has_a_section_and_a_unique_path_and_variable() {
        let mut paths = std::collections::BTreeSet::new();
        let mut envs = std::collections::BTreeSet::new();
        for key in KEYS {
            assert!(key.path.split_once('.').is_some_and(|(s, n)| !s.is_empty() && !n.is_empty()), "{}", key.path);
            assert!(paths.insert(key.path), "duplicate {}", key.path);
            if let Some(env) = key.env {
                assert!(env.starts_with("MOLY_"), "{env}");
                assert!(envs.insert(env), "duplicate {env}");
            }
        }
    }

    #[test]
    fn the_environment_wins_over_the_file_and_an_empty_flag_does_not_count() {
        let config = Config::parse("[server]\nurl = \"https://file.example\"\n", None).unwrap();
        let set = env_of(&[("MOLY_SERVER_URL", "https://env.example")]);
        assert_eq!(config.text_in(&keys::SERVER_URL, &set).as_deref(), Some("https://env.example"));
        let empty = env_of(&[("MOLY_SERVER_URL", "")]);
        assert_eq!(config.text_in(&keys::SERVER_URL, &empty).as_deref(), Some(""));
        let hud = Config::parse("[display]
hide_hud = true
", None).unwrap();
        assert_eq!(hud.flag_in(&keys::HIDE_HUD, &env_of(&[("MOLY_HIDE_HUD", "")])), Ok(Some(true)));
        assert_eq!(Config::empty().text_in(&keys::SERVER_URL, &env_of(&[])), None);
    }

    #[test]
    fn unknown_keys_and_wrong_kinds_are_refused_by_name() {
        let unknown = Config::parse("[server]\nadress = \"x\"\n", None).unwrap_err();
        assert_eq!(unknown, ConfigError::Unknown { path: None, key: "server.adress".into() });
        let loose = Config::parse("url = \"https://x.example\"\n", None).unwrap_err();
        assert_eq!(loose, ConfigError::Unknown { path: None, key: "url".into() });
        let kind = Config::parse("[display]\nhide_hud = \"yes\"\n", None).unwrap_err();
        assert!(matches!(kind, ConfigError::Kind { key: "display.hide_hud", .. }));
        let address = Config::parse("[server]\nurl = \"server.example\"\n", None).unwrap_err();
        assert!(matches!(address, ConfigError::Kind { key: "server.url", expected: Kind::Url, .. }));
    }

    #[test]
    fn flags_read_one_or_zero_from_the_environment_and_booleans_from_the_file() {
        let config = Config::parse("[display]\nhide_hud = true\n", None).unwrap();
        assert_eq!(config.flag_in(&keys::HIDE_HUD, &env_of(&[])), Ok(Some(true)));
        assert_eq!(config.flag_in(&keys::HIDE_HUD, &env_of(&[("MOLY_HIDE_HUD", "0")])), Ok(Some(false)));
        assert!(config.flag_in(&keys::HIDE_HUD, &env_of(&[("MOLY_HIDE_HUD", "yes")])).is_err());
    }

    #[test]
    fn a_missing_required_key_names_the_section_and_the_variable() {
        let error = Config::empty().require_text(&keys::PLAYER_API).unwrap_err().to_string();
        assert!(error.contains("[player_data]") && error.contains("MOLY_PLAYER_API"), "{error}");
        let error = Config::empty().require_text(&keys::RESOURCE_BASE).unwrap_err().to_string();
        assert!(error.contains("[resources]") && !error.contains("MOLY_"), "{error}");
    }

    #[test]
    fn editing_keeps_comments_and_removes_keys() {
        let text = "# the launcher's choices\n[launcher]\n# keep this\njobs = 4\n";
        let edited = edit_text(
            text,
            &[
                (&keys::LAUNCHER_JOBS, Some(Value::Integer(8))),
                (&keys::RESOURCE_BASE, Some(Value::Text("https://cdn.example/".into()))),
            ],
            None,
        )
        .unwrap();
        assert!(edited.contains("# the launcher's choices") && edited.contains("# keep this"), "{edited}");
        let config = Config::parse(&edited, None).unwrap();
        assert_eq!(config.integer_in(&keys::LAUNCHER_JOBS, &env_of(&[])), Ok(Some(8)));
        let removed = edit_text(&edited, &[(&keys::LAUNCHER_JOBS, None)], None).unwrap();
        assert_eq!(Config::parse(&removed, None).unwrap().integer_in(&keys::LAUNCHER_JOBS, &env_of(&[])), Ok(None));
        assert!(edit_text(text, &[(&keys::SERVER_URL, Some(Value::Text("not an address".into())))], None).is_err());
    }
}
