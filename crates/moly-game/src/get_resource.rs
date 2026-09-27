//! The acquisition dialogs: `MysekaiGetResourceSubWindowDialog` (DialogType
//! 332) with its openers, and the views of the delivery's reward dialogs
//! `CommonRewardSubWindowDialog` (250), `HonorRewardSubWindowDialog` (262)
//! and `MysekaiRefreshBirthdayPlantSubWindowDialog` (450). Every one of them
//! opens and closes through the screen manager ([`crate::ui_layers`]).
//!
//! ## The openers of 332 (client code, read in the source)
//!
//! `MysekaiResourceUtility.NoticeCollectItem(resourceType, resourceId,
//! quantity)` ([`GetResourceOpeners::notice_collect_item`]) switches on the
//! resource type:
//! - `mysekai_material`: the master row (none: nothing happens);
//!   `CanCollectMaterial`, `IsNewGetMaterial`, `GetMaterialCount` (its value
//!   is not used), `MarkEarnedMaterial` (which adds the id only when its set
//!   already holds it, so the set stays empty and `IsNewGetMaterial` is
//!   `IsNewMaterial`); `PlayDropItemSound` (a tone
//!   material, or a rarity of raw value 1 or more, plays
//!   `se_get_rare_material`, else `se_get_material`); then the notice layer's
//!   collect notice with the master name, the bundle
//!   `mysekai/item_preview/material/<icon>`, the icon name, the quantity,
//!   limit = not collectable, new, and rare = `(rarity - 1) < 3` unsigned
//!   (rarity_2 to rarity_4; rarity_1 is raw 0).
//! - `mysekai_blueprint`: the blueprint row, and for a fixture or tool
//!   blueprint its target row (either missing: nothing happens); a
//!   blueprint the player already has becomes the surplus blueprint item
//!   (`mysekai_item`, level 1); the entry is chained on the one shared chain
//!   player (`ChainSubWindowDialog(null, null, allowCloseExternal true, 332)`,
//!   `Setup(resource, null)`), which starts playing unless it already plays;
//!   its finish only drops the shared player.
//! - `mysekai_fixture`: the master name; a plant shows the preview
//!   `mysekai/item_preview/fixture/<bundle>_<id>`, any other fixture the
//!   thumbnail `mysekai/thumbnail/fixture/<bundle>`; `se_get_material`; the
//!   collect notice with limit, new and rare false.
//! - `mysekai_item`: the master name and `mysekai/item_preview/item/<icon>`;
//!   `se_get_material`; the notice with rare true.
//! - `mysekai_music_record`: the record row and its music (or soundtrack)
//!   row; a record the player already has becomes the surplus record item;
//!   then the chain as for a blueprint.
//! - `material`: the material master; `se_get_material`; the notice.
//! - any other type: the notice with an empty name and no icon, no sound.
//!
//! The harvest raises the same call from its own pickup
//! (`crate::harvest::notice`, which reads the possession limit, the new
//! flag and the rarity and plays the cue): this module reads its
//! `CollectNotice` messages and takes those flags as they are, playing no
//! cue for them, so a drop's cue plays once.
//!
//! `SketchUtility.ShowSketchResultDialog(blueprintId, onClose)`
//! ([`GetResourceOpeners::show_sketch_result`]): a single 332
//! (`ShowSubWindowDialog`, not chained) with `UserResource(blueprintId, 40,
//! level 0, quantity 1)`, whose close is awaited by the sketch sequence.
//!
//! `UIUtility.ShowRewardDialog` (the mission screen) and the secret shop are
//! not callers in this product: the mission screen is not built and the
//! shop is paid content.
//!
//! ## 332's Setup, by resource type
//!
//! - blueprint (40): the blueprint view instead of a thumbnail; body
//!   `Format(MSG_GET_BLUEPRINT, name)`, the name being
//!   `Format(FORMAT_MYSEKAI_BLUEPRINT_NAME, target name)` (a tool blueprint
//!   names the tool, a fixture or canvas blueprint the fixture);
//! - item (42): thumbnail; the surplus record item shows
//!   `MSG_GET_SURPLUS_RECORD`, the surplus blueprint item
//!   `Format(MSG_GET_SURPLUS_BLUEPRINT, item name)`, any other item the
//!   caller's key;
//! - music record (44): thumbnail; `MSG_GET_RECORD`;
//! - any other: thumbnail; the caller's key.
//!
//! The name balloon (a tap on the thumbnail) shows `GetResourceName`. Every
//! text comes from the root's wordings and masters.
//!
//! ## The views and their closes
//!
//! Each dialog draws its root's prefab document while the screen manager
//! shows it; the open animation passes at once (the fade is not built). A
//! `SubWindowDialog` closes on its close area (the full-screen area outside
//! the window, `allowCloseExternal`), on the hardware back key (the manager
//! sends its `DialogBackKeyEvent`) and never on a tap inside the window.
//! For 332 this module closes the dialog itself (`CloseProcess` -> `Close`)
//! and opens the next chained one. 250, 262 and 450 belong to the delivery
//! flow, which awaits their close request: their views answer a tap on the
//! close area with the same close request the back key sends, and the flow
//! closes them. Their callers hand the message and the Setup argument to
//! the view ([`GetResourceOpeners::set_reward_payload`]); without it the
//! body stays empty and says so.
//!
//! ## Missing inputs (named, never guessed)
//!
//! - A reward dialog whose prefab document the UI sources do not carry has
//!   no view: each time its caller shows it, it is refused by name and its
//!   close request goes to the caller at once (an undrawn modal dialog
//!   would hold the flow with nothing on screen).
//! - 250: one thumbnail per resource is not built.
//! - 262: the honor's rarity and its images come from the root's honor
//!   image index (`honor/index.json`: per honor its rarity, background
//!   bundle and frame bundle; per bundle its textures). A root without it
//!   (the CN roots carry no honor bundles) refuses the message key and the
//!   image by name. The image is `HonorUtility.CreateHonorImage(slot main,
//!   size 1)`: the `UIPartsHonorImage` prefab instantiated under `imageRoot`
//!   (composed once into the dialog's document), at the main slot's size
//!   and the size's scale; for a birthday honor its core image is
//!   `degree_main` of the background bundle, its frame
//!   `frame_degree_m_<1 + rarity>` of the frame bundle, the rank image is
//!   hidden (only an event honor shows one), the plain level row is hidden
//!   and the unique level row shows `level` pips of
//!   `frame_degree_level_<1 + rarity>`; the live master parts are hidden. A
//!   frame the bundle lacks is hidden and named; a root without the prefab
//!   names it and draws no image. The holder balloon is hidden.
//! - 450: the party's `icon_refresh.png` is not on the roots: the icon is
//!   hidden.
//! - The music master is not on the roots: a record's name balloon stays
//!   shut (its body is the fixed `MSG_GET_RECORD`).
//! - Server data: the user's blueprints and materials come from the client
//!   inventory copy (`ClientMysekaiInventory`); before the server model
//!   installs it, each use logs its stand-in (a first get, not new); the
//!   user's records likewise from `ClientMusicRecords`. A material call from
//!   outside the harvest (the instrument) takes `CanCollectMaterial` as
//!   collectable and logs the stand-in; the harvest's calls carry the
//!   harvest's own reading.
//! - The thumbnails, the blueprint's 3D preview, the open and close
//!   animations and the dialog sounds are not built.
//!
//! Instruments (off by default; game mode reads none):
//! `MOLY_GET_RESOURCE_NOTICE` = `type:id:qty[,...]` calls
//! `notice_collect_item` once the inputs are ready, after
//! `MOLY_GET_RESOURCE_NOTICE_SECS`; `MOLY_GET_RESOURCE_SKETCH` = a blueprint
//! id calls `show_sketch_result` then; `MOLY_GET_RESOURCE_REWARD_DIALOGS` =
//! `250,262,450` shows those reward dialogs as their delivery caller does
//! and closes them on their close request; `MOLY_GET_RESOURCE_AUTOCLOSE_SECS`
//! injects a tap at the top-left corner of the window (the close area) that
//! many seconds after any of these dialogs shows.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use serde_json::Value;

use crate::action_button::ActionTapConsumed;
use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::balloon::{BalloonArt, ascii_or};
use crate::collect_notice::NoticeCollectItem;
use crate::gesture::{GestureEvent, GestureKind, GestureState};
use crate::menu_shell::ShellDialogState;
use crate::server::client::instrument_env;
use crate::server::client::inventory::ClientMysekaiInventory;
use crate::server::client::music_play::ClientMusicRecords;
use crate::sitemap::SITEMAP_LAYER;
use crate::ui_layers::{
    DialogBackKey, DialogBackKeyEvent, DialogId, DialogState, DialogType, DisplayLayerType,
    ScreenManager,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// `DialogType.CommonRewardSubWindowDialog`.
const COMMON_REWARD_DIALOG: DialogType = DialogType(250);
/// `DialogType.HonorRewardSubWindowDialog`.
const HONOR_REWARD_DIALOG: DialogType = DialogType(262);
/// `DialogType.MysekaiRefreshBirthdayPlantSubWindowDialog`.
const REFRESH_DIALOG: DialogType = DialogType(450);

/// The layout keys of the three prefab documents.
const KEY_GET_RESOURCE: &str = "GetResource";
const KEY_COMMON_REWARD: &str = "CommonReward";
const KEY_HONOR: &str = "HonorReward";
const KEY_REFRESH: &str = "RefreshBirthdayPlant";
/// `UIPartsHonorImage`, the honor image prefab (an optional document).
const KEY_HONOR_IMAGE: &str = "HonorImage";
/// 262's document with the honor image composed under `imageRoot`.
const KEY_HONOR_COMPOSED: &str = "HonorRewardComposed";
/// Where `HonorRewardSubWindowDialog.Setup` has the image created.
const HONOR_IMAGE_PARENT: &str = "Content/CustomButton/imageRoot";
/// `HonorUtility.GetScaleFactorValue`: the reward dialog's
/// `CreateHonorImage` passes size 1, and the table's value there is 0.8
/// (sizes 0 to 3: 0.7, 0.8, 1.0, 1.2). `SetScale` sets the image's local
/// scale to it. The main slot's size (`GetSlotSize(main)`, 380 x 80) is the
/// prefab root's own size.
const HONOR_IMAGE_SCALE: f32 = 0.8;

const MSG_GET_BLUEPRINT: &str = "MSG_GET_BLUEPRINT";
const MSG_GET_SURPLUS_BLUEPRINT: &str = "MSG_GET_SURPLUS_BLUEPRINT";
const MSG_GET_SURPLUS_RECORD: &str = "MSG_GET_SURPLUS_RECORD";
const MSG_GET_RECORD: &str = "MSG_GET_RECORD";
const MSG_RECEIVED_REWARD: &str = "MSG_RECEIVED_REWARD";
const FORMAT_MYSEKAI_BLUEPRINT_NAME: &str = "FORMAT_MYSEKAI_BLUEPRINT_NAME";
const WORD_FORMAT_RECORD_NAME: &str = "WORD_FORMAT_RECORD_NAME";
/// The wordings every text of these dialogs is made of (the reward
/// dialogs' message keys are their callers').
const WORDINGS: &[&str] = &[
    "MSG_RECEIVED_DELIVERY_TOTAL_REWARD",
    "MSG_RECEIVED_BIRTHDAY_HONOR",
    "WORD_ACHIEVEMENT_GET",
    "MSG_MYSEKAI_DELIVERY_HARVEST_REFRESH",
    MSG_GET_BLUEPRINT,
    MSG_GET_SURPLUS_BLUEPRINT,
    MSG_GET_SURPLUS_RECORD,
    MSG_GET_RECORD,
    MSG_RECEIVED_REWARD,
    FORMAT_MYSEKAI_BLUEPRINT_NAME,
    WORD_FORMAT_RECORD_NAME,
];

/// The masters the openers read.
const MASTERS: [&str; 6] = [
    "mysekai-materials.json",
    "mysekai-blueprints.json",
    "mysekai-fixtures.json",
    "mysekai-tools.json",
    "mysekai-items.json",
    "mysekai-music-records.json",
];

const SE_GET_MATERIAL: &str = "se_get_material";
const SE_GET_RARE_MATERIAL: &str = "se_get_rare_material";

/// `ResourceType` names the openers switch on.
const RT_MYSEKAI_MATERIAL: &str = "mysekai_material";
const RT_MYSEKAI_BLUEPRINT: &str = "mysekai_blueprint";
const RT_MYSEKAI_FIXTURE: &str = "mysekai_fixture";
const RT_MYSEKAI_ITEM: &str = "mysekai_item";
const RT_MYSEKAI_MUSIC_RECORD: &str = "mysekai_music_record";
const RT_MATERIAL: &str = "material";

/// The resource type name of a `Sekai.Constants.ResourceType` raw value
/// (the program's constant table), as the openers switch on it; an empty
/// name for a type none of their arms names (the default arm).
#[allow(dead_code)] // Called by the harvest pickup's seam.
pub(crate) fn resource_type_name(value: i32) -> &'static str {
    match value {
        2 => RT_MATERIAL,
        39 => RT_MYSEKAI_FIXTURE,
        40 => RT_MYSEKAI_BLUEPRINT,
        41 => RT_MYSEKAI_MATERIAL,
        42 => RT_MYSEKAI_ITEM,
        44 => RT_MYSEKAI_MUSIC_RECORD,
        _ => "",
    }
}

/// Enum raw values (the program's constant table): `MysekaiMaterialType.tone`,
/// `MysekaiFixtureType.plant`, `MysekaiItemType.surplus_blueprint` and
/// `surplus_music_record`.
const MATERIAL_TYPE_TONE: i32 = 6;
const FIXTURE_TYPE_PLANT: i32 = 2;
const ITEM_TYPE_SURPLUS_BLUEPRINT: i32 = 1;
const ITEM_TYPE_SURPLUS_MUSIC_RECORD: i32 = 2;

// ---------------------------------------------------------------------------
// Masters
// ---------------------------------------------------------------------------

struct MaterialRow {
    name: String,
    icon: String,
    /// `MysekaiMaterialType` raw value.
    kind: i32,
    /// `MysekaiMaterialRarityType` raw value (rarity_1 = 0 ... rarity_4 = 3).
    rarity: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CraftType {
    Fixture,
    Tool,
    Canvas,
    Material,
}

struct BlueprintRow {
    craft_type: CraftType,
    target: i32,
}

struct FixtureRow {
    name: String,
    bundle: String,
    /// `MysekaiFixtureType` raw value.
    kind: i32,
}

struct ItemRow {
    name: String,
    icon: String,
    /// `MysekaiItemType` raw value.
    kind: i32,
}

struct Masters {
    materials: HashMap<i32, MaterialRow>,
    blueprints: HashMap<i32, BlueprintRow>,
    fixtures: HashMap<i32, FixtureRow>,
    tools: HashMap<i32, String>,
    items: HashMap<i32, ItemRow>,
    records: HashSet<i32>,
}

impl Masters {
    fn parse(docs: &[Value]) -> Result<Masters, String> {
        let entries = |index: usize| -> Result<&serde_json::Map<String, Value>, String> {
            docs[index]["entries"]
                .as_object()
                .ok_or_else(|| format!("{}: no entries", MASTERS[index]))
        };
        let int = |row: &Value, key: &str, file: &str| -> Result<i32, String> {
            row[key]
                .as_i64()
                .map(|v| v as i32)
                .ok_or_else(|| format!("{file}: a row without {key}"))
        };
        let text = |row: &Value, key: &str, file: &str| -> Result<String, String> {
            row[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{file}: a row without {key}"))
        };
        let mut materials = HashMap::new();
        for row in entries(0)?.values() {
            let file = MASTERS[0];
            let kind = match row["mysekaiMaterialType"].as_str() {
                Some("wood") => 0,
                Some("mineral") => 1,
                Some("plant") => 2,
                Some("junk") => 3,
                Some("game_character") => 4,
                Some("other") => 5,
                Some("tone") => MATERIAL_TYPE_TONE,
                Some("birthday_party") => 7,
                other => return Err(format!("{file}: material type {other:?}")),
            };
            let rarity = match row["mysekaiMaterialRarityType"].as_str() {
                Some("rarity_1") => 0,
                Some("rarity_2") => 1,
                Some("rarity_3") => 2,
                Some("rarity_4") => 3,
                other => return Err(format!("{file}: rarity {other:?}")),
            };
            materials.insert(
                int(row, "id", file)?,
                MaterialRow {
                    name: text(row, "name", file)?,
                    icon: text(row, "iconAssetbundleName", file)?,
                    kind,
                    rarity,
                },
            );
        }
        let mut blueprints = HashMap::new();
        for row in entries(1)?.values() {
            let file = MASTERS[1];
            let craft_type = match row["mysekaiCraftType"].as_str() {
                Some("mysekai_fixture") => CraftType::Fixture,
                Some("mysekai_tool") => CraftType::Tool,
                Some("mysekai_canvas") => CraftType::Canvas,
                Some("material") => CraftType::Material,
                other => return Err(format!("{file}: craft type {other:?}")),
            };
            blueprints.insert(
                int(row, "id", file)?,
                BlueprintRow {
                    craft_type,
                    target: int(row, "craftTargetId", file)?,
                },
            );
        }
        let mut fixtures = HashMap::new();
        let fixture_rows = docs[2]["fixtures"]
            .as_array()
            .ok_or_else(|| format!("{}: no fixtures", MASTERS[2]))?;
        for row in fixture_rows {
            let file = MASTERS[2];
            let kind = match row["fixtureType"].as_str() {
                Some("system") => 0,
                Some("custom") => 1,
                Some("plant") => FIXTURE_TYPE_PLANT,
                Some("house_plant") => 3,
                Some("surface_appearance") => 4,
                Some("gate") => 5,
                Some("normal") => 6,
                Some("canvas") => 7,
                other => return Err(format!("{file}: fixture type {other:?}")),
            };
            fixtures.insert(
                int(row, "id", file)?,
                FixtureRow {
                    name: text(row, "name", file)?,
                    bundle: text(row, "assetbundleName", file)?,
                    kind,
                },
            );
        }
        let mut tools = HashMap::new();
        for row in entries(3)?.values() {
            tools.insert(int(row, "id", MASTERS[3])?, text(row, "name", MASTERS[3])?);
        }
        let mut items = HashMap::new();
        for row in entries(4)?.values() {
            let file = MASTERS[4];
            let kind = match row["mysekaiItemType"].as_str() {
                Some("white_blueprint") => 0,
                Some("surplus_blueprint") => ITEM_TYPE_SURPLUS_BLUEPRINT,
                Some("surplus_music_record") => ITEM_TYPE_SURPLUS_MUSIC_RECORD,
                Some("mysekai_photo_film") => 3,
                Some("blueprint_fragment") => 4,
                other => return Err(format!("{file}: item type {other:?}")),
            };
            items.insert(
                int(row, "id", file)?,
                ItemRow {
                    name: text(row, "name", file)?,
                    icon: text(row, "iconAssetbundleName", file)?,
                    kind,
                },
            );
        }
        let mut records = HashSet::new();
        for row in entries(5)?.values() {
            records.insert(int(row, "id", MASTERS[5])?);
        }
        Ok(Masters {
            materials,
            blueprints,
            fixtures,
            tools,
            items,
            records,
        })
    }

    /// `GetMysekaiItem(itemType)`: the item row of that type (the lowest id
    /// when several share it).
    fn item_of_type(&self, kind: i32) -> Option<i32> {
        self.items
            .iter()
            .filter(|(_, row)| row.kind == kind)
            .map(|(id, _)| *id)
            .min()
    }

    /// `GetMysekaiBlueprintName`: the target's name in
    /// `FORMAT_MYSEKAI_BLUEPRINT_NAME`, empty when there is none.
    fn blueprint_name(&self, id: i32, wordings: &HashMap<String, String>) -> String {
        let Some(row) = self.blueprints.get(&id) else {
            return String::new();
        };
        let name = match row.craft_type {
            CraftType::Tool => self.tools.get(&row.target).cloned(),
            CraftType::Fixture | CraftType::Canvas => {
                self.fixtures.get(&row.target).map(|f| f.name.clone())
            }
            CraftType::Material => None,
        };
        match (name, wordings.get(FORMAT_MYSEKAI_BLUEPRINT_NAME)) {
            (Some(name), Some(format)) => format_one(format, &name),
            _ => String::new(),
        }
    }
}

/// `string.Format(format, arg)` for the one-argument wordings.
fn format_one(format: &str, arg: &str) -> String {
    moly_law::text::custom_text_mesh::format_wording(format, &[arg.to_owned()])
        .unwrap_or_else(|error| panic!("[get_resource] wording format {format:?}: {error}"))
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// `UserResource.resourceType`, by the arm of 332's Setup it takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceKind {
    Blueprint,
    Item,
    MusicRecord,
}

/// One 332 entry (a `UserResource` and its resolved texts).
#[derive(Debug, Clone)]
struct ResourceEntry {
    kind: ResourceKind,
    resource_id: i32,
    resource_level: i32,
    quantity: i32,
    /// The body text (`SetMessageBodyText`).
    body: String,
    /// `GetResourceName` for the name balloon; None when its master is not
    /// on the roots.
    name: Option<String>,
}

impl ResourceEntry {
    fn wire(&self) -> &'static str {
        match self.kind {
            ResourceKind::Blueprint => "mysekai_blueprint(40)",
            ResourceKind::Item => "mysekai_item(42)",
            ResourceKind::MusicRecord => "mysekai_music_record(44)",
        }
    }
}

/// A 332 entry's opener, for its close.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opener {
    /// `NoticeCollectItem`: on the shared chain player.
    Chain,
    /// `ShowSketchResultDialog`: alone, its close awaited by the ticket.
    Sketch(u64),
}

/// The 332 dialog on screen.
struct Shown {
    id: DialogId,
    entry: ResourceEntry,
    opener: Opener,
    /// The name balloon is open (a tap on the thumbnail toggles it; a new
    /// entry resets it).
    balloon: bool,
}

/// A call waiting for the inputs.
#[derive(Debug, Clone)]
enum Call {
    Notice {
        resource_type: String,
        resource_id: i32,
        quantity: i32,
        caller: &'static str,
        /// A harvest notice's flags (limit, new, rare); its cue has played.
        harvest: Option<(bool, bool, bool)>,
    },
    Sketch {
        blueprint_id: i32,
        ticket: u64,
    },
}

// ---------------------------------------------------------------------------
// The openers' API
// ---------------------------------------------------------------------------

/// What a delivery reward dialog's caller hands it: the message of
/// `Initialize(message)` and the argument of `Setup`.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Constructed by the delivery flow's seam.
pub(crate) enum RewardPayload {
    /// 250, `DialogUtility.ShowGetResourceDialogAsync(resources, message)`:
    /// the message's wording key and the number of resources (one
    /// thumbnail each).
    Common {
        message_key: String,
        resources: usize,
    },
    /// 262, `ChainSubWindowDialog(message)` + `Setup(resource)`: the honor
    /// and its level (the message key follows from the honor's rarity, read
    /// here from the root's honor image index: see [`honor_message_key`]).
    Honor { honor_id: i32, level: i32 },
    /// 450, `Setup(party)`: the birthday party.
    Refresh { party_id: i32 },
}

/// The client calls that open 332 or the collect notice, and the payloads of
/// the reward dialogs. Callers push; the module resolves the calls in order
/// once the masters and the wordings are loaded.
#[derive(Resource, Default)]
pub(crate) struct GetResourceOpeners {
    calls: VecDeque<Call>,
    next_ticket: u64,
    closed_sketches: HashSet<u64>,
    payloads: HashMap<DialogId, RewardPayload>,
}

impl GetResourceOpeners {
    /// `MysekaiResourceUtility.NoticeCollectItem(resourceType, resourceId,
    /// quantity)`. `caller` names the source caller for the log.
    #[allow(dead_code)] // Called by the harvest, delivery and talk seams.
    pub(crate) fn notice_collect_item(
        &mut self,
        resource_type: &str,
        resource_id: i32,
        quantity: i32,
        caller: &'static str,
    ) {
        self.calls.push_back(Call::Notice {
            resource_type: resource_type.to_owned(),
            resource_id,
            quantity,
            caller,
            harvest: None,
        });
    }

    /// `SketchUtility.ShowSketchResultDialog(blueprintId, onClose)`: returns
    /// the ticket whose close [`Self::take_sketch_closed`] reports.
    pub(crate) fn show_sketch_result(&mut self, blueprint_id: i32) -> u64 {
        self.next_ticket += 1;
        let ticket = self.next_ticket;
        self.calls.push_back(Call::Sketch {
            blueprint_id,
            ticket,
        });
        ticket
    }

    /// The sketch result dialog of `ticket` has closed (its onClose ran);
    /// reported once. Its onClose re-enables the sketch screen's UI.
    #[allow(dead_code)] // Read by the sketch screen once it is built.
    pub(crate) fn take_sketch_closed(&mut self, ticket: u64) -> bool {
        self.closed_sketches.remove(&ticket)
    }

    /// A reward dialog's caller hands its message and Setup argument to the
    /// view of the dialog `id` it showed.
    #[allow(dead_code)] // Called by the delivery flow's seam.
    pub(crate) fn set_reward_payload(&mut self, id: DialogId, payload: RewardPayload) {
        self.payloads.insert(id, payload);
    }
}

// ---------------------------------------------------------------------------
// The text charset
// ---------------------------------------------------------------------------

/// Every character these dialogs can draw: their wordings, every blueprint
/// name and every item name. It joins the shell's text charset, which the
/// one glyph atlas is baked from; it is present once the inputs are read
/// (empty when an input is missing, which is logged).
#[derive(Resource, Debug, Default)]
pub(crate) struct GetResourceCharset {
    pub(crate) chars: Vec<char>,
}

/// The fixed texts the shell charset takes from this module.
pub(crate) const FIXED_TEXTS: &[&str] = &["×0123456789"];

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// The requested masters and what was read from them.
#[derive(Resource, Default)]
pub(crate) struct GetResourceInputs {
    handles: Vec<Handle<JsonAsset>>,
    masters: Option<Masters>,
    absent: bool,
    /// The root's honor image index (`honor/index.json`), by honor id.
    /// Optional: a root without it refuses the honor dialog's message key
    /// and image by name.
    honors_handle: Option<Handle<JsonAsset>>,
    honors: Option<HashMap<i32, HonorImage>>,
}

/// One honor of the root's honor image index.
#[derive(Clone, Debug)]
struct HonorImage {
    /// The honor group's type is `birthday` (the only image path built here:
    /// `SetupLevelView`'s birthday branch).
    birthday: bool,
    /// `HonorRarity` raw value (the program's constant table: low 0,
    /// middle 1, high 2, highest 3).
    rarity: i32,
    /// The core image `degree_main` of the honor's background bundle.
    core: Option<String>,
    /// The frame `frame_degree_m_<1 + rarity>` of the honor's frame bundle
    /// (`m`: the reward dialog's slot 1), with its Sprite rect size.
    frame: Option<(String, Vec2)>,
    /// `HonorUtility.GetUniqueLevelTextureName(rarity)`: the level pip
    /// `frame_degree_level_<1 + rarity>` of the frame bundle.
    level_pip: Option<String>,
}

/// The honor image's nodes in 262's composed document.
#[derive(Clone, Debug)]
struct HonorBindings {
    body: String,
    frame: String,
    rank: String,
    levels: String,
    unique_levels: String,
    unique_icons: Vec<String>,
    live_master: String,
    /// The frame image's authored Sprite pixels per unit (the prefab's own
    /// frame Sprite; the index records rects only).
    frame_pixels_per_unit: f32,
}

/// The dialogs' runtime state.
#[derive(Resource, Default)]
pub(crate) struct GetResourcePlayer {
    /// The shared chain player's queue (`_chainDialogPlayer`); None when no
    /// chain plays.
    chain: Option<VecDeque<ResourceEntry>>,
    /// Sketch results waiting for the view (one 332 view at a time).
    sketches: VecDeque<(u64, ResourceEntry)>,
    shown: Option<Shown>,
    /// A close tap on 332 this frame.
    close_requested: bool,
    /// Sketch results closed or refused this frame, for their tickets.
    pending_sketch_closes: Vec<u64>,
    /// `_alreadyCollectMaterials`: nothing adds to it (`MarkEarnedMaterial`
    /// adds only an id it already holds), so it stays empty.
    earned_materials: HashSet<i32>,
    /// Stand-ins already reported, by (what, id).
    reported: HashSet<(&'static str, i32)>,
    instrument_fired: bool,
    /// Reward dialogs the instrument showed as their caller would.
    instrument_rewards: Vec<DialogId>,
    /// When the last shown dialog appeared (the autoclose instrument).
    shown_since: Option<(DialogId, f32)>,
    autoclosed: HashSet<DialogId>,
}

/// The 332 view's root.
#[derive(Component)]
pub(crate) struct GetResourceRoot;

/// The view root of a delivery reward dialog (262 or 450).
#[derive(Component)]
pub(crate) struct RewardDialogRoot {
    dialog: DialogType,
    /// The dialog instance drawn now.
    showing: Option<DialogId>,
}

/// The views are spawned (or their documents reported absent).
#[derive(Resource, Default)]
pub(crate) struct GetResourceSpawned {
    get_resource: bool,
    rewards: bool,
    /// Reward documents already named missing.
    reported: HashSet<&'static str>,
    /// 262's honor image nodes, once composed (None: no image prefab).
    honor: Option<HonorBindings>,
    honor_composed: bool,
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

pub(crate) fn init(mut commands: Commands, server: Res<AssetServer>) {
    commands.init_resource::<GetResourceOpeners>();
    commands.init_resource::<GetResourcePlayer>();
    commands.init_resource::<GetResourceSpawned>();
    commands.insert_resource(GetResourceInputs {
        handles: MASTERS
            .iter()
            .map(|path| server.load(bevy::asset::AssetPath::from(format!("moly://{path}"))))
            .collect(),
        masters: None,
        absent: false,
        honors_handle: Some(server.load(bevy::asset::AssetPath::from(HONOR_INDEX.to_owned()))),
        honors: None,
    });
}

// ---------------------------------------------------------------------------
// Update: spawn the views
// ---------------------------------------------------------------------------

/// Spawn each view once its document is ready. A document the UI sources
/// do not carry is reported once.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    mut layouts: ResMut<crate::ui_layout::UiLayouts>,
    server: Res<AssetServer>,
    mut spawned: ResMut<GetResourceSpawned>,
) {
    if !spawned.get_resource && layouts.ready(KEY_GET_RESOURCE, &server) {
        commands.spawn((
            GetResourceRoot,
            Visibility::Hidden,
            Transform::default(),
            RenderLayers::layer(SITEMAP_LAYER),
            crate::ui_layout::UiPrefabView::new(KEY_GET_RESOURCE, SITEMAP_LAYER),
        ));
        spawned.get_resource = true;
    }
    // The sources are loaded once the 332 document is there.
    if spawned.rewards || layouts.document(KEY_GET_RESOURCE).is_none() {
        return;
    }
    let mut waiting = false;
    // 262's image: the honor image prefab composed under `imageRoot` once.
    if !spawned.honor_composed && layouts.document(KEY_HONOR).is_some() {
        spawned.honor_composed = true;
        if layouts.document(KEY_HONOR_IMAGE).is_some() {
            match compose_honor(&mut layouts, &server) {
                Ok(bindings) => {
                    info!(
                        "[get_resource] UIPartsHonorImage composed under 262's {HONOR_IMAGE_PARENT} (scale {HONOR_IMAGE_SCALE})"
                    );
                    spawned.honor = Some(bindings);
                }
                Err(reason) => error!(
                    "[get_resource] UIPartsHonorImage could not be composed into 262 ({reason}); the honor image is not drawn"
                ),
            }
        } else {
            error!(
                "[get_resource] the UIPartsHonorImage prefab ({KEY_HONOR_IMAGE}) is not in the UI sources; 262 draws no honor image"
            );
        }
    }
    for (key, dialog) in [
        (KEY_COMMON_REWARD, COMMON_REWARD_DIALOG),
        (KEY_HONOR, HONOR_REWARD_DIALOG),
        (KEY_REFRESH, REFRESH_DIALOG),
    ] {
        if layouts.document(key).is_none() {
            if spawned.reported.insert(key) {
                error!(
                    "[get_resource] {dialog:?} has no view: the UI sources do not carry its prefab document ({key}); each show of it is refused"
                );
            }
            continue;
        }
        let view_key = if dialog == HONOR_REWARD_DIALOG && spawned.honor.is_some() {
            KEY_HONOR_COMPOSED
        } else {
            key
        };
        if !layouts.ready(view_key, &server) {
            waiting = true;
            continue;
        }
        commands.spawn((
            RewardDialogRoot {
                dialog,
                showing: None,
            },
            Visibility::Hidden,
            Transform::default(),
            RenderLayers::layer(SITEMAP_LAYER),
            crate::ui_layout::UiPrefabView::new(view_key, SITEMAP_LAYER),
        ));
    }
    if !waiting {
        spawned.rewards = true;
    }
}

/// `HonorUtility.CreateHonorImage` in 262: instantiate the honor image
/// prefab under `imageRoot` of a copy of 262's document, set its scale, and
/// install the copy as a runtime document.
fn compose_honor(
    layouts: &mut crate::ui_layout::UiLayouts,
    server: &AssetServer,
) -> Result<HonorBindings, String> {
    let mut doc = layouts
        .document(KEY_HONOR)
        .ok_or("262's document is not loaded")?
        .clone();
    let template = layouts
        .document(KEY_HONOR_IMAGE)
        .ok_or("the honor image document is not loaded")?
        .clone();
    let instance = doc.instantiate_subtree(
        HONOR_IMAGE_PARENT,
        &template,
        "UIPartsHonorImage",
        "UIPartsHonorImage",
    )?;
    let root = doc.find(&instance.root_selector())?;
    doc.nodes[root].rect.local_scale = [HONOR_IMAGE_SCALE; 3];
    let bind = |suffix: &str| -> Result<String, String> {
        let index = template.find(suffix)?;
        instance.selector(template.nodes[index].game_object_id)
    };
    let frame_index = template.find("UIPartsHonorImage/frame")?;
    let frame_pixels_per_unit = template.nodes[frame_index]
        .components
        .iter()
        .filter_map(|component| component.sprite.as_ref())
        .find_map(|sprite| sprite["pixelsPerUnit"].as_f64())
        .ok_or("the prefab's frame Sprite has no pixelsPerUnit")?
        as f32;
    let bindings = HonorBindings {
        body: bind("UIPartsHonorImage/body")?,
        frame: bind("UIPartsHonorImage/frame")?,
        rank: bind("UIPartsHonorImage/rank")?,
        levels: bind("UIPartsHonorImage/levels")?,
        unique_levels: bind("UIPartsHonorImage/uniqueLevels")?,
        unique_icons: (1..=5)
            .map(|index| bind(&format!("uniqueLevels/icon{index}")))
            .collect::<Result<_, _>>()?,
        live_master: bind("UIPartsHonorImage/liveMasterParts")?,
        frame_pixels_per_unit,
    };
    layouts.replace_runtime_document(KEY_HONOR_COMPOSED, doc, server)?;
    Ok(bindings)
}

// ---------------------------------------------------------------------------
// Resolving the calls
// ---------------------------------------------------------------------------

/// Read the masters once they are loaded.
fn read_inputs(inputs: &mut GetResourceInputs, server: &AssetServer, json: &Assets<JsonAsset>) {
    if let Some(handle) = inputs.honors_handle.clone() {
        if let LoadState::Failed(error) = server.load_state(&handle) {
            error!(
                "[get_resource] {HONOR_INDEX} is not on the root ({error}); the honor dialog's message key and image are refused"
            );
            inputs.honors_handle = None;
        } else if let Some(asset) = json.get(&handle) {
            inputs.honors_handle = None;
            match honor_index(&asset.0) {
                Ok(honors) => {
                    info!(
                        "[get_resource] {HONOR_INDEX}: {} honors, {} with a frame",
                        honors.len(),
                        honors
                            .values()
                            .filter(|honor| honor.frame.is_some())
                            .count()
                    );
                    inputs.honors = Some(honors);
                }
                Err(reason) => error!(
                    "[get_resource] {HONOR_INDEX}: {reason}; the honor dialog's message key and image are refused"
                ),
            }
        }
    }
    if inputs.masters.is_some() || inputs.absent {
        return;
    }
    let mut docs = Vec::new();
    for (path, handle) in MASTERS.iter().zip(&inputs.handles) {
        if let LoadState::Failed(error) = server.load_state(handle) {
            error!(
                "[get_resource] input absent: {path} ({error}); NoticeCollectItem and the sketch result stay off"
            );
            inputs.absent = true;
            return;
        }
        let Some(asset) = json.get(handle) else {
            return;
        };
        docs.push(
            serde_json::from_str::<Value>(&asset.0)
                .unwrap_or_else(|error| panic!("[get_resource] {path}: not JSON: {error}")),
        );
    }
    match Masters::parse(&docs) {
        Ok(masters) => {
            info!(
                "[get_resource] masters read: {} materials, {} blueprints, {} fixtures, {} tools, {} items, {} records",
                masters.materials.len(),
                masters.blueprints.len(),
                masters.fixtures.len(),
                masters.tools.len(),
                masters.items.len(),
                masters.records.len()
            );
            inputs.masters = Some(masters);
        }
        Err(error) => panic!("[get_resource] {error}"),
    }
}

/// The charset of every text these dialogs draw.
fn charset(masters: Option<&Masters>, wordings: &HashMap<String, String>) -> Vec<char> {
    let mut chars: Vec<char> = Vec::new();
    for key in WORDINGS {
        match wordings.get(*key) {
            Some(text) => chars.extend(text.chars()),
            None => error!(
                "[get_resource] wording {key} is not on the root; texts built from it are refused"
            ),
        }
    }
    if let Some(masters) = masters {
        for id in masters.blueprints.keys() {
            chars.extend(masters.blueprint_name(*id, wordings).chars());
        }
        for row in masters.items.values() {
            chars.extend(row.name.chars());
        }
    }
    for text in FIXED_TEXTS {
        chars.extend(text.chars());
    }
    chars.retain(|ch| *ch != '\n');
    chars.sort_unstable();
    chars.dedup();
    chars
}

/// A stand-in for server data, reported once per (what, id).
fn stand_in(player: &mut GetResourcePlayer, what: &'static str, id: i32, detail: &str) {
    if player.reported.insert((what, id)) {
        warn!("[get_resource] {what} for {id}: {detail}");
    }
}

fn play_se(se: &mut SeRequests, cue: &str) {
    se.0.push(SeRequest {
        owner: None,
        cue: cue.to_owned(),
        class: SeClass::Ingame,
        source: "notice-collect-item",
    });
}

/// 332's Setup texts for an entry.
fn setup_texts(
    kind: ResourceKind,
    resource_id: i32,
    masters: &Masters,
    wordings: &HashMap<String, String>,
) -> Result<(String, Option<String>), String> {
    let wording = |key: &str| -> Result<&String, String> {
        wordings
            .get(key)
            .ok_or_else(|| format!("wording {key} is not on the root"))
    };
    match kind {
        ResourceKind::Blueprint => {
            let name = masters.blueprint_name(resource_id, wordings);
            Ok((format_one(wording(MSG_GET_BLUEPRINT)?, &name), Some(name)))
        }
        ResourceKind::Item => {
            let row = masters
                .items
                .get(&resource_id)
                .ok_or_else(|| format!("item {resource_id} is not in the items master"))?;
            let body = match row.kind {
                ITEM_TYPE_SURPLUS_MUSIC_RECORD => wording(MSG_GET_SURPLUS_RECORD)?.clone(),
                ITEM_TYPE_SURPLUS_BLUEPRINT => {
                    format_one(wording(MSG_GET_SURPLUS_BLUEPRINT)?, &row.name)
                }
                // The caller's key: NoticeCollectItem passes null and never
                // reaches this arm; a sketch result is a blueprint.
                _ => wording(MSG_RECEIVED_REWARD)?.clone(),
            };
            Ok((body, Some(row.name.clone())))
        }
        // The record's name needs the music master, which is not on the roots.
        ResourceKind::MusicRecord => Ok((wording(MSG_GET_RECORD)?.clone(), None)),
    }
}

/// Resolve one `NoticeCollectItem` call: the collect notice, or a chained
/// 332 entry.
#[allow(clippy::too_many_arguments)]
fn notice(
    resource_type: &str,
    resource_id: i32,
    quantity: i32,
    caller: &'static str,
    masters: &Masters,
    wordings: &HashMap<String, String>,
    inventory: Option<&ClientMysekaiInventory>,
    records: Option<&ClientMusicRecords>,
    harvest: Option<(bool, bool, bool)>,
    player: &mut GetResourcePlayer,
    se: &mut SeRequests,
    notices: &mut MessageWriter<NoticeCollectItem>,
) {
    // A harvest notice's cue has played in the harvest's pickup.
    let mut quiet = SeRequests::default();
    let se = if harvest.is_some() { &mut quiet } else { se };
    let chained = |kind: ResourceKind, id: i32, level: i32| -> Option<ResourceEntry> {
        match setup_texts(kind, id, masters, wordings) {
            Ok((body, name)) => Some(ResourceEntry {
                kind,
                resource_id: id,
                resource_level: level,
                quantity,
                body,
                name,
            }),
            Err(reason) => {
                error!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): 332 refused: {reason}"
                );
                None
            }
        }
    };
    let hud = |notices: &mut MessageWriter<NoticeCollectItem>,
               item_name: String,
               bundle: String,
               resource: String,
               is_limit: bool,
               is_new: bool,
               is_rare: bool| {
        info!(
            "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}, {quantity}) -> MysekaiUtility.NoticeCollectItem(bundle {}, resource {}, count {quantity}, limit {is_limit}, new {is_new}, rare {is_rare})",
            ascii_or(&bundle),
            ascii_or(&resource)
        );
        notices.write(NoticeCollectItem {
            item_name,
            asset_bundle_name: bundle,
            resource_name: resource,
            get_count: quantity,
            is_limit,
            is_new,
            is_rare,
        });
    };
    match resource_type {
        RT_MYSEKAI_MATERIAL => {
            let Some(row) = masters.materials.get(&resource_id) else {
                info!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): no master row; nothing happens"
                );
                return;
            };
            // `IsNewGetMaterial`: `IsNewMaterial` (the user's material table
            // is absent or empty, or has no row of the id) and the id not in
            // the earned set. `MarkEarnedMaterial` adds the id only when the
            // set already holds it (the body tests `Contains`, then `Add`),
            // so nothing ever adds to it and it stays empty.
            let (is_limit, is_new) = match harvest {
                Some((is_limit, is_new, _)) => (is_limit, is_new),
                None => {
                    stand_in(
                        player,
                        "CanCollectMaterial",
                        resource_id,
                        "a call from outside the harvest reads no possession limit; collectable",
                    );
                    let is_new = match inventory {
                        Some(inventory) => {
                            !inventory
                                .materials()
                                .iter()
                                .any(|row| row.mysekai_material_id == resource_id)
                                && !player.earned_materials.contains(&resource_id)
                        }
                        None => {
                            stand_in(
                                player,
                                "IsNewMaterial",
                                resource_id,
                                "no client copy of the user's materials is installed; not new",
                            );
                            false
                        }
                    };
                    (false, is_new)
                }
            };
            let rare_sound = row.kind == MATERIAL_TYPE_TONE || row.rarity >= 1;
            play_se(
                se,
                if rare_sound {
                    SE_GET_RARE_MATERIAL
                } else {
                    SE_GET_MATERIAL
                },
            );
            let is_rare = match harvest {
                Some((_, _, is_rare)) => is_rare,
                None => (row.rarity.wrapping_sub(1) as u32) < 3,
            };
            hud(
                notices,
                row.name.clone(),
                format!("mysekai/item_preview/material/{}", row.icon),
                row.icon.clone(),
                is_limit,
                is_new,
                is_rare,
            );
        }
        RT_MYSEKAI_BLUEPRINT => {
            let Some(row) = masters.blueprints.get(&resource_id) else {
                info!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): no blueprint row; nothing happens"
                );
                return;
            };
            let target_present = match row.craft_type {
                CraftType::Fixture => masters.fixtures.contains_key(&row.target),
                CraftType::Tool => masters.tools.contains_key(&row.target),
                CraftType::Canvas | CraftType::Material => true,
            };
            if !target_present {
                info!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): no target row {}; nothing happens",
                    row.target
                );
                return;
            }
            let owned = match inventory {
                Some(inventory) => inventory.has_blueprint(resource_id),
                None => {
                    stand_in(
                        player,
                        "HasMysekaiBlueprint",
                        resource_id,
                        "no client copy of the user's blueprints is installed; a first get",
                    );
                    false
                }
            };
            // An owned blueprint becomes the surplus blueprint item, level 1.
            let entry = if owned {
                match masters.item_of_type(ITEM_TYPE_SURPLUS_BLUEPRINT) {
                    Some(item) => chained(ResourceKind::Item, item, 1),
                    None => {
                        error!(
                            "[get_resource] {caller}: the items master has no surplus blueprint row; 332 refused"
                        );
                        None
                    }
                }
            } else {
                chained(ResourceKind::Blueprint, resource_id, 1)
            };
            if let Some(entry) = entry {
                enqueue_chain(player, entry, caller);
            }
        }
        RT_MYSEKAI_MUSIC_RECORD => {
            if !masters.records.contains(&resource_id) {
                info!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): no record row; nothing happens"
                );
                return;
            }
            stand_in(
                player,
                "MusicMaster",
                resource_id,
                "the music and soundtrack masters the record's check reads are not on the roots; the record is taken as valid",
            );
            let owned = match records {
                Some(records) => records.possession(resource_id).is_some(),
                None => {
                    stand_in(
                        player,
                        "HasMysekaiMusicRecord",
                        resource_id,
                        "no client copy of the user's records is installed; a first get",
                    );
                    false
                }
            };
            // An owned record becomes the surplus record item, level 1.
            let entry = if owned {
                match masters.item_of_type(ITEM_TYPE_SURPLUS_MUSIC_RECORD) {
                    Some(item) => chained(ResourceKind::Item, item, 1),
                    None => {
                        error!(
                            "[get_resource] {caller}: the items master has no surplus record row; 332 refused"
                        );
                        None
                    }
                }
            } else {
                chained(ResourceKind::MusicRecord, resource_id, 1)
            };
            if let Some(entry) = entry {
                enqueue_chain(player, entry, caller);
            }
        }
        RT_MYSEKAI_FIXTURE => {
            let Some(row) = masters.fixtures.get(&resource_id) else {
                info!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): no master row; nothing happens"
                );
                return;
            };
            let (bundle, resource) = if row.kind == FIXTURE_TYPE_PLANT {
                (
                    format!("mysekai/item_preview/fixture/{}_{resource_id}", row.bundle),
                    format!("{}_{resource_id}", row.bundle),
                )
            } else {
                (
                    format!("mysekai/thumbnail/fixture/{}", row.bundle),
                    row.bundle.clone(),
                )
            };
            play_se(se, SE_GET_MATERIAL);
            hud(
                notices,
                row.name.clone(),
                bundle,
                resource,
                false,
                false,
                false,
            );
        }
        RT_MYSEKAI_ITEM => {
            let Some(row) = masters.items.get(&resource_id) else {
                info!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): no master row; nothing happens"
                );
                return;
            };
            play_se(se, SE_GET_MATERIAL);
            hud(
                notices,
                row.name.clone(),
                format!("mysekai/item_preview/item/{}", row.icon),
                row.icon.clone(),
                false,
                false,
                true,
            );
        }
        RT_MATERIAL => {
            error!(
                "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}): the material master is not on the roots; the notice is refused"
            );
        }
        _ => hud(
            notices,
            String::new(),
            String::new(),
            String::new(),
            false,
            false,
            false,
        ),
    }
}

/// `ChainSubWindowDialog` + `Setup` on the shared chain player, and `Play`
/// unless it plays already.
fn enqueue_chain(player: &mut GetResourcePlayer, entry: ResourceEntry, caller: &'static str) {
    let playing = player.chain.is_some();
    info!(
        "[get_resource] {caller}: NoticeCollectItem -> ChainSubWindowDialog(null, null, true, 332), Setup({} {} level {} x{}); {}",
        entry.wire(),
        entry.resource_id,
        entry.resource_level,
        entry.quantity,
        if playing {
            "the chain plays already: queued"
        } else {
            "Play(null, onFinish)"
        }
    );
    player
        .chain
        .get_or_insert_with(VecDeque::new)
        .push_back(entry);
}

// ---------------------------------------------------------------------------
// Update: the dialogs
// ---------------------------------------------------------------------------

#[derive(SystemParam)]
pub(crate) struct PlaceInputs<'w, 's> {
    server: Res<'w, AssetServer>,
    json: Res<'w, Assets<JsonAsset>>,
    layouts: ResMut<'w, crate::ui_layout::UiLayouts>,
    art: Option<Res<'w, BalloonArt>>,
    time: Res<'w, Time>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<'w, crate::canvas::RootCanvas>>,
    inventory: Option<Res<'w, ClientMysekaiInventory>>,
    records: Option<Res<'w, ClientMusicRecords>>,
    harvest_notices: MessageReader<'w, 's, crate::harvest::notice::CollectNotice>,
}

/// The characters of `text` the atlas lacks (rich-text tags and white space
/// are not drawn as glyphs).
fn undrawable(text: &str, art: &BalloonArt) -> Vec<char> {
    let mut missing = Vec::new();
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag || ch.is_whitespace() || ch.is_control() => {}
            _ if art.glyph_cell(ch).is_none() => missing.push(ch),
            _ => {}
        }
    }
    missing
}

/// Characters as `U+XXXX` code points, for the log.
fn code_points(chars: &[char]) -> String {
    let mut points: Vec<String> = chars
        .iter()
        .map(|ch| format!("U+{:04X}", *ch as u32))
        .collect();
    points.sort_unstable();
    points.dedup();
    points.join(" ")
}

/// Show the next 332 entry (a sketch result first, then the chain).
fn show_next(player: &mut GetResourcePlayer, screens: &mut ScreenManager, art: &BalloonArt) {
    loop {
        let (entry, opener) = if let Some((ticket, entry)) = player.sketches.pop_front() {
            (entry, Opener::Sketch(ticket))
        } else if let Some(entry) = player.chain.as_mut().and_then(VecDeque::pop_front) {
            (entry, Opener::Chain)
        } else {
            return;
        };
        let missing = undrawable(&entry.body, art);
        if !missing.is_empty() {
            error!(
                "[get_resource] 332 entry {} {} refused: its body's characters {} are not in the glyph atlas (absent from the charset or from the font)",
                entry.wire(),
                entry.resource_id,
                code_points(&missing)
            );
            finish(player, opener);
            continue;
        }
        let caller = match opener {
            Opener::Chain => "ChainDialogPlayer.Play",
            Opener::Sketch(_) => "SketchUtility.ShowSketchResultDialog",
        };
        match screens.show_dialog(
            DialogType::MysekaiGetResourceSubWindowDialog,
            DisplayLayerType::LayerDialog,
            DialogBackKey::Close,
            caller,
        ) {
            Ok(id) => {
                screens.open_dialog(id);
                screens.dialog_open_finished(id);
                info!(
                    "[get_resource] 332 shown ({caller}): Setup {} {} level {} x{}; OpenAsync: the open animation is not built and passes at once",
                    entry.wire(),
                    entry.resource_id,
                    entry.resource_level,
                    entry.quantity
                );
                player.shown = Some(Shown {
                    id,
                    entry,
                    opener,
                    balloon: false,
                });
                return;
            }
            Err(reason) => {
                error!("[get_resource] {reason}");
                finish(player, opener);
            }
        }
    }
}

/// An entry's onClose: a sketch result reports its ticket; the chain's last
/// entry runs its onFinish (the shared player is dropped).
fn finish(player: &mut GetResourcePlayer, opener: Opener) {
    match opener {
        Opener::Sketch(ticket) => player.pending_sketch_closes.push(ticket),
        Opener::Chain => {
            if player.chain.as_ref().is_some_and(VecDeque::is_empty) {
                player.chain = None;
                info!("[get_resource] the chain's onFinish: the shared chain player is dropped");
            }
        }
    }
}

/// Update: resolve the calls, drive 332 through the screen manager, and draw
/// the three views.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place(
    mut commands: Commands,
    inputs_param: PlaceInputs,
    mut inputs: ResMut<GetResourceInputs>,
    mut openers: ResMut<GetResourceOpeners>,
    mut player: ResMut<GetResourcePlayer>,
    mut screens: ResMut<ScreenManager>,
    mut dialog: ResMut<ShellDialogState>,
    mut se: ResMut<SeRequests>,
    mut notices: MessageWriter<NoticeCollectItem>,
    mut back_keys: MessageReader<DialogBackKeyEvent>,
    mut gestures: MessageWriter<GestureEvent>,
    charset_present: Option<Res<GetResourceCharset>>,
    spawned: Res<GetResourceSpawned>,
    mut roots: Query<
        (
            &mut Visibility,
            &mut Transform,
            &mut crate::ui_layout::UiPrefabView,
        ),
        (With<GetResourceRoot>, Without<RewardDialogRoot>),
    >,
    mut rewards: Query<
        (
            &mut RewardDialogRoot,
            &mut Visibility,
            &mut Transform,
            &mut crate::ui_layout::UiPrefabView,
        ),
        Without<GetResourceRoot>,
    >,
) {
    let PlaceInputs {
        server,
        json,
        mut layouts,
        art,
        time,
        windows,
        root_canvas,
        inventory,
        records,
        mut harvest_notices,
    } = inputs_param;
    read_inputs(&mut inputs, &server, &json);
    let sources_ready = layouts.document(KEY_GET_RESOURCE).is_some();
    let inputs_settled = inputs.masters.is_some() || inputs.absent;
    if charset_present.is_none() && sources_ready && inputs_settled {
        let chars = charset(inputs.masters.as_ref(), &layouts.wordings);
        info!("[get_resource] charset: {} characters", chars.len());
        commands.insert_resource(GetResourceCharset { chars });
    }
    let now = time.elapsed_secs();
    fire_instrument(
        &mut player,
        &mut openers,
        &mut screens,
        now,
        inputs_settled && sources_ready,
    );

    // The harvest's notices join the calls, in order.
    for harvest in harvest_notices.read() {
        use crate::harvest::notice::NoticeArm;
        let resource_type = match harvest.arm {
            NoticeArm::MysekaiMaterial => RT_MYSEKAI_MATERIAL,
            NoticeArm::MysekaiFixture => RT_MYSEKAI_FIXTURE,
            NoticeArm::MysekaiItem => RT_MYSEKAI_ITEM,
            NoticeArm::MysekaiBlueprint => RT_MYSEKAI_BLUEPRINT,
            NoticeArm::MysekaiMusicRecord => RT_MYSEKAI_MUSIC_RECORD,
            NoticeArm::Other => "other",
        };
        let Ok(resource_id) = i32::try_from(harvest.resource_id) else {
            error!(
                "[get_resource] harvest notice {resource_type} {}: the id does not fit a master id; refused",
                harvest.resource_id
            );
            continue;
        };
        openers.calls.push_back(Call::Notice {
            resource_type: resource_type.to_owned(),
            resource_id,
            quantity: harvest.get_count,
            caller: "HarvestUtility.OnCollectDropItem",
            harvest: Some((harvest.is_limit, harvest.is_new, harvest.is_rare)),
        });
    }

    // Resolve the calls.
    if let (Some(masters), true) = (inputs.masters.as_ref(), sources_ready) {
        while let Some(call) = openers.calls.pop_front() {
            match call {
                Call::Notice {
                    resource_type,
                    resource_id,
                    quantity,
                    caller,
                    harvest,
                } => notice(
                    &resource_type,
                    resource_id,
                    quantity,
                    caller,
                    masters,
                    &layouts.wordings,
                    inventory.as_deref(),
                    records.as_deref(),
                    harvest,
                    &mut player,
                    &mut se,
                    &mut notices,
                ),
                Call::Sketch {
                    blueprint_id,
                    ticket,
                } => match setup_texts(
                    ResourceKind::Blueprint,
                    blueprint_id,
                    masters,
                    &layouts.wordings,
                ) {
                    Ok((body, name)) => {
                        info!(
                            "[get_resource] ShowSketchResultDialog({blueprint_id}): ShowSubWindowDialog(null, onClose, true, 332), await Setup(UserResource({blueprint_id}, 40, 0, 1))"
                        );
                        player.sketches.push_back((
                            ticket,
                            ResourceEntry {
                                kind: ResourceKind::Blueprint,
                                resource_id: blueprint_id,
                                resource_level: 0,
                                quantity: 1,
                                body,
                                name,
                            },
                        ));
                    }
                    Err(reason) => {
                        error!(
                            "[get_resource] ShowSketchResultDialog({blueprint_id}) refused: {reason}"
                        );
                        openers.closed_sketches.insert(ticket);
                    }
                },
            }
        }
    } else if inputs.absent {
        let calls: Vec<Call> = openers.calls.drain(..).collect();
        for call in calls {
            match call {
                Call::Sketch { ticket, .. } => {
                    openers.closed_sketches.insert(ticket);
                }
                Call::Notice {
                    resource_type,
                    resource_id,
                    caller,
                    ..
                } => error!(
                    "[get_resource] {caller}: NoticeCollectItem({resource_type}, {resource_id}) refused: the masters are not on the root"
                ),
            }
        }
    }

    // Close 332 on its close tap or the back key.
    let back: Vec<DialogBackKeyEvent> = back_keys.read().copied().collect();
    // The instrument's reward dialogs close on their close request, as
    // their caller closes them.
    let instrument_rewards = std::mem::take(&mut player.instrument_rewards);
    for id in instrument_rewards {
        if back
            .iter()
            .any(|event| event.id == id && event.back_key == DialogBackKey::Close)
        {
            screens.close_dialog(id);
            screens.dialog_destroyed(id);
            info!(
                "[get_resource] instrument: {id:?} closed on its close request, as its caller closes it"
            );
        } else {
            player.instrument_rewards.push(id);
        }
    }
    if let Some(shown) = player.shown.as_ref() {
        let by_back = back
            .iter()
            .any(|event| event.id == shown.id && event.back_key == DialogBackKey::Close);
        if by_back || player.close_requested {
            let shown = player.shown.take().expect("shown");
            info!(
                "[get_resource] 332 closes ({}): SubWindowDialog.CloseProcess -> Close: onClose; DialogBase.Close; the close animation is not built: destroyed",
                if by_back {
                    "the back key"
                } else {
                    "the close area"
                }
            );
            screens.close_dialog(shown.id);
            screens.dialog_destroyed(shown.id);
            finish(&mut player, shown.opener);
        }
    }
    player.close_requested = false;
    for ticket in std::mem::take(&mut player.pending_sketch_closes) {
        openers.closed_sketches.insert(ticket);
    }
    if player.shown.is_none() {
        if let Some(art) = art.as_deref() {
            show_next(&mut player, &mut screens, art);
        }
    }
    for ticket in std::mem::take(&mut player.pending_sketch_closes) {
        openers.closed_sketches.insert(ticket);
    }

    // Draw.
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    for (mut visible, mut transform, mut view) in &mut roots {
        transform.scale = Vec3::splat(scale);
        match player.shown.as_ref() {
            Some(shown) => {
                *visible = Visibility::Inherited;
                let blueprint = shown.entry.kind == ResourceKind::Blueprint;
                view.set_text("Content/BodyText", shown.entry.body.clone());
                view.set_visible("Content/Object3DPreview", blueprint);
                view.set_visible("Content/ThunbmainRoot", !blueprint);
                view.set_visible("Object3DPreview/UIPartsLoadingCircleTexture", false);
                view.set_visible("Object3DPreview/UIPartsCommonBalloon (1)", shown.balloon);
                view.set_text(
                    "UIPartsCommonBalloon (1)/Content/CustomText",
                    shown.entry.name.clone().unwrap_or_default(),
                );
            }
            None => *visible = Visibility::Hidden,
        }
    }
    let mut reward_shown = false;
    for (mut root, mut visible, mut transform, mut view) in &mut rewards {
        transform.scale = Vec3::splat(scale);
        let top = screens
            .dialogs()
            .filter(|(_, dialog, state)| *dialog == root.dialog && *state == DialogState::Show)
            .map(|(id, _, _)| id)
            .last();
        if top != root.showing {
            if let Some(id) = top {
                let payload = openers.payloads.get(&id).cloned();
                let bindings = spawned
                    .honor
                    .as_ref()
                    .filter(|_| root.dialog == HONOR_REWARD_DIALOG);
                if let (Some(bindings), Some(RewardPayload::Honor { honor_id, .. })) =
                    (bindings, payload.as_ref())
                {
                    if let Some(image) = inputs
                        .honors
                        .as_ref()
                        .and_then(|honors| honors.get(honor_id))
                    {
                        register_honor_textures(
                            &mut layouts,
                            &server,
                            image,
                            bindings.frame_pixels_per_unit,
                        );
                    }
                }
                draw_reward(
                    &layouts,
                    &mut view,
                    root.dialog,
                    id,
                    payload.as_ref(),
                    art.as_deref(),
                    inputs.honors.as_ref(),
                    bindings,
                );
            } else {
                info!("[get_resource] {:?} view hidden", root.dialog);
            }
            if let Some(previous) = root.showing {
                openers.payloads.remove(&previous);
            }
            root.showing = top;
        }
        *visible = if top.is_some() {
            reward_shown = true;
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    dialog.get_resource_open = player.shown.is_some() || reward_shown;

    // The autoclose instrument.
    let top_id = player.shown.as_ref().map(|shown| shown.id).or_else(|| {
        rewards
            .iter()
            .filter_map(|(root, _, _, _)| root.showing)
            .last()
    });
    autoclose(&mut player, top_id, now, window, &mut gestures);
}

/// `UIPartsHonorImage.LoadAsync` for a birthday honor in the main slot:
/// the core image, the dedicated frame (`SetHonorFrame`), the rank image
/// off (only an event honor with a rank bundle shows it), and
/// `SetupLevelView`'s birthday branch: the plain level row off, the unique
/// level row on with the rarity's pip texture on every icon and icons
/// `0..level` active (`UIPartsUniqueHonorLevel.SetValue`). `Refresh`
/// initialises the live master parts, which hides them. Returns what the
/// bundles lack (drawn hidden).
fn draw_honor_image(
    view: &mut crate::ui_layout::UiPrefabView,
    bindings: &HonorBindings,
    image: &HonorImage,
    level: i32,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    match image.core.as_deref() {
        Some(path) => {
            view.set_visible(&bindings.body, true);
            view.set_texture(&bindings.body, path);
        }
        None => {
            view.set_visible(&bindings.body, false);
            missing.push("the core image degree_main");
        }
    }
    match image.frame.as_ref() {
        Some((path, _)) => {
            view.set_visible(&bindings.frame, true);
            view.set_texture(&bindings.frame, path);
        }
        None => {
            view.set_visible(&bindings.frame, false);
            missing.push("the main frame");
        }
    }
    view.set_visible(&bindings.rank, false);
    view.set_visible(&bindings.levels, false);
    view.set_visible(&bindings.live_master, false);
    match image.level_pip.as_deref() {
        Some(path) => {
            view.set_visible(&bindings.unique_levels, true);
            for (index, icon) in bindings.unique_icons.iter().enumerate() {
                view.set_texture(icon, path);
                view.set_visible(icon, (index as i32) < level);
            }
        }
        None => {
            view.set_visible(&bindings.unique_levels, false);
            missing.push("the level pip");
        }
    }
    missing
}

/// Load the textures a shown honor image draws, keyed by their paths; the
/// frame is a Sprite image, so its authored metrics are registered with it.
fn register_honor_textures(
    layouts: &mut crate::ui_layout::UiLayouts,
    server: &AssetServer,
    image: &HonorImage,
    frame_pixels_per_unit: f32,
) {
    for path in [image.core.as_deref(), image.level_pip.as_deref()]
        .into_iter()
        .flatten()
    {
        layouts.register_runtime_texture(path, path, server);
    }
    if let Some((path, size)) = image.frame.as_ref() {
        layouts.set_runtime_sprite_layout(
            path,
            crate::ui_layout::SpriteLayoutMetrics {
                rect_size: *size,
                border: [0.0; 4],
                pixels_per_unit: frame_pixels_per_unit,
            },
        );
        layouts.register_runtime_texture(path, path, server);
    }
}

/// The delivery's honor dialog message (`DeliverySiteController`, before
/// `ChainSubWindowDialog`): `WORD_ACHIEVEMENT_GET` for a low-rarity honor at
/// level 1, `MSG_RECEIVED_BIRTHDAY_HONOR` otherwise (also for an honor with
/// no rarity).
fn honor_message_key(rarity: Option<i32>, level: i32) -> &'static str {
    if rarity == Some(0) && level == 1 {
        "WORD_ACHIEVEMENT_GET"
    } else {
        "MSG_RECEIVED_BIRTHDAY_HONOR"
    }
}

/// A reward dialog's prefab document key.
fn reward_key(dialog: DialogType) -> Option<&'static str> {
    match dialog {
        COMMON_REWARD_DIALOG => Some(KEY_COMMON_REWARD),
        HONOR_REWARD_DIALOG => Some(KEY_HONOR),
        REFRESH_DIALOG => Some(KEY_REFRESH),
        _ => None,
    }
}

/// The root's honor image index.
const HONOR_INDEX: &str = "moly://honor/index.json";

/// Read the honor image index: per honor its rarity and the image files of
/// its background and frame bundles.
fn honor_index(text: &str) -> Result<HashMap<i32, HonorImage>, String> {
    let value: Value = serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))?;
    let bundles = value["bundles"].as_object().ok_or("no bundles")?;
    let image = |bundle: &str, texture: &str| -> Option<String> {
        bundles.get(bundle)?["textures"][texture]["image"]
            .as_str()
            .map(|path| format!("moly://{path}"))
    };
    let sprite_size = |bundle: &str, sprite: &str| -> Option<Vec2> {
        let rect = bundles.get(bundle)?["sprites"][sprite]["rect"].as_array()?;
        Some(Vec2::new(
            rect.get(2)?.as_f64()? as f32,
            rect.get(3)?.as_f64()? as f32,
        ))
    };
    let mut honors = HashMap::new();
    for row in value["honors"].as_array().ok_or("no honors")? {
        let id = row["honorId"].as_i64().ok_or("an honor without honorId")? as i32;
        let rarity = match row["honorRarity"].as_str() {
            Some("low") => 0,
            Some("middle") => 1,
            Some("high") => 2,
            Some("highest") => 3,
            other => return Err(format!("honor {id}: rarity {other:?}")),
        };
        let image_bundle = row["imageBundle"]
            .as_str()
            .ok_or_else(|| format!("honor {id}: no imageBundle"))?;
        let frame_bundle = row["frameBundle"]
            .as_str()
            .ok_or_else(|| format!("honor {id}: no frameBundle"))?;
        honors.insert(
            id,
            HonorImage {
                birthday: row["honorType"].as_str() == Some("birthday"),
                rarity,
                core: image(image_bundle, "degree_main"),
                frame: {
                    let name = format!("frame_degree_m_{}", 1 + rarity);
                    image(frame_bundle, &name).zip(sprite_size(frame_bundle, &name))
                },
                level_pip: image(frame_bundle, &format!("frame_degree_level_{}", 1 + rarity)),
            },
        );
    }
    Ok(honors)
}

/// Whether the view's document has exactly one node with this path suffix.
fn has_path(layouts: &crate::ui_layout::UiLayouts, key: &str, path: &str) -> bool {
    layouts
        .document(key)
        .is_some_and(|doc| doc.find(path).is_ok())
}

/// A reward dialog appears: its body from the caller's message, and what
/// its missing inputs leave out.
fn draw_reward(
    layouts: &crate::ui_layout::UiLayouts,
    view: &mut crate::ui_layout::UiPrefabView,
    dialog: DialogType,
    id: DialogId,
    payload: Option<&RewardPayload>,
    art: Option<&BalloonArt>,
    honors: Option<&HashMap<i32, HonorImage>>,
    bindings: Option<&HonorBindings>,
) {
    let key = view.key;
    let message_key: Option<&str> = match payload {
        Some(RewardPayload::Common { message_key, .. }) => Some(message_key.as_str()),
        Some(RewardPayload::Honor { honor_id, level }) => match honors {
            Some(honors) => {
                let rarity = honors.get(honor_id).map(|honor| honor.rarity);
                if rarity.is_none() {
                    error!(
                        "[get_resource] {dialog:?} {id:?}: honor {honor_id} is not in {HONOR_INDEX}; the message key takes it as without a rarity"
                    );
                }
                Some(honor_message_key(rarity, *level))
            }
            None => {
                error!(
                    "[get_resource] {dialog:?} {id:?}: {HONOR_INDEX} is not on the root; the honor's message key is refused"
                );
                None
            }
        },
        _ => None,
    };
    let body = match (payload, message_key) {
        (Some(RewardPayload::Refresh { .. }), _) => None,
        (Some(_), None) => Some(String::new()),
        (Some(_), Some(message_key)) => match layouts.wordings.get(message_key) {
            Some(text) => Some(text.clone()),
            None => {
                error!(
                    "[get_resource] {dialog:?} {id:?}: its message {message_key} is not on the root; the body stays empty"
                );
                Some(String::new())
            }
        },
        (None, _) => {
            if dialog != REFRESH_DIALOG {
                error!(
                    "[get_resource] {dialog:?} {id:?}: its caller handed no message; the body stays empty"
                );
                Some(String::new())
            } else {
                None
            }
        }
    };
    if let Some(body) = body {
        let missing = art.map(|art| undrawable(&body, art)).unwrap_or_default();
        let path = "BodyText";
        if !has_path(layouts, key, path) {
            error!(
                "[get_resource] {dialog:?}: the document {key} has no single {path} node; the body is not written"
            );
        } else if art.is_none() {
            error!(
                "[get_resource] {dialog:?} {id:?}: the glyph atlas is not baked; the body stays empty"
            );
            view.set_text(path, String::new());
        } else if !missing.is_empty() {
            error!(
                "[get_resource] {dialog:?} {id:?}: its body's characters {} are not in the glyph atlas (absent from the charset or from the font); the body stays empty",
                code_points(&missing)
            );
            view.set_text(path, String::new());
        } else {
            view.set_text(path, body);
        }
    }
    match dialog {
        COMMON_REWARD_DIALOG => {
            let count = match payload {
                Some(RewardPayload::Common { resources, .. }) => *resources,
                _ => 0,
            };
            error!(
                "[get_resource] {dialog:?} {id:?} drawn; its {count} resource thumbnails are not built"
            );
        }
        HONOR_REWARD_DIALOG => {
            if has_path(layouts, key, "CustomButton/UIPartsCommonBalloon") {
                view.set_visible("CustomButton/UIPartsCommonBalloon", false);
            }
            let (drawn, honor) = match payload {
                Some(RewardPayload::Honor { honor_id, level }) => {
                    let image = honors.map(|honors| honors.get(honor_id));
                    match (bindings, image) {
                        (_, None) => (
                            false,
                            format!(
                                "honor {honor_id} level {level}: {HONOR_INDEX} is not on the root"
                            ),
                        ),
                        (_, Some(None)) => (
                            false,
                            format!("honor {honor_id} level {level}: not in {HONOR_INDEX}"),
                        ),
                        (None, Some(Some(_))) => (
                            false,
                            format!(
                                "honor {honor_id} level {level}: the UIPartsHonorImage prefab is not composed (see its load line)"
                            ),
                        ),
                        (Some(_), Some(Some(image))) if !image.birthday => (
                            false,
                            format!(
                                "honor {honor_id} level {level}: not a birthday honor; only SetupLevelView's birthday branch is built"
                            ),
                        ),
                        (Some(bindings), Some(Some(image))) => {
                            let missing = draw_honor_image(view, bindings, image, *level);
                            (
                                true,
                                format!(
                                    "honor {honor_id} level {level}: core {}, frame {}, {level} level pips {}{}",
                                    image
                                        .core
                                        .as_deref()
                                        .unwrap_or("hidden (absent from its bundle)"),
                                    image
                                        .frame
                                        .as_ref()
                                        .map_or("hidden (absent from its bundle)", |(path, _)| {
                                            path.as_str()
                                        }),
                                    image
                                        .level_pip
                                        .as_deref()
                                        .unwrap_or("hidden (absent from its bundle)"),
                                    if missing.is_empty() {
                                        String::new()
                                    } else {
                                        format!("; missing: {}", missing.join(", "))
                                    }
                                ),
                            )
                        }
                    }
                }
                _ => (false, "no honor handed".to_owned()),
            };
            if has_path(layouts, key, HONOR_IMAGE_PARENT) {
                view.set_visible(HONOR_IMAGE_PARENT, drawn);
            }
            if drawn {
                info!(
                    "[get_resource] {dialog:?} {id:?} drawn; UIPartsHonorImage (slot main, scale {HONOR_IMAGE_SCALE}): {honor}; rank image and plain levels hidden (a birthday honor); the holder balloon is hidden"
                );
            } else {
                error!(
                    "[get_resource] {dialog:?} {id:?} drawn; the honor image is not drawn: {honor}; the holder balloon is hidden"
                );
            }
        }
        _ => {
            if has_path(layouts, key, "Base/IconLoader") {
                view.set_visible("Base/IconLoader", false);
            }
            let party = match payload {
                Some(RewardPayload::Refresh { party_id }) => format!("party {party_id}"),
                _ => "no party handed".to_owned(),
            };
            error!(
                "[get_resource] {dialog:?} {id:?} drawn ({party}); its icon (the party's icon_refresh.png) is not on the roots and is hidden"
            );
        }
    }
}

/// The notice and sketch instruments.
fn fire_instrument(
    player: &mut GetResourcePlayer,
    openers: &mut GetResourceOpeners,
    screens: &mut ScreenManager,
    now: f32,
    ready: bool,
) {
    if player.instrument_fired || !ready {
        return;
    }
    let notice = instrument_env("MOLY_GET_RESOURCE_NOTICE");
    let sketch = instrument_env("MOLY_GET_RESOURCE_SKETCH");
    let rewards = instrument_env("MOLY_GET_RESOURCE_REWARD_DIALOGS");
    if notice.is_none() && sketch.is_none() && rewards.is_none() {
        player.instrument_fired = true;
        return;
    }
    let at = instrument_env("MOLY_GET_RESOURCE_NOTICE_SECS")
        .and_then(|raw| raw.trim().parse::<f32>().ok())
        .unwrap_or(0.0);
    if now < at {
        return;
    }
    player.instrument_fired = true;
    if let Some(spec) = notice {
        for part in spec.split(',') {
            let fields: Vec<&str> = part.trim().split(':').collect();
            match (
                fields.as_slice(),
                fields.get(1).and_then(|v| v.parse::<i32>().ok()),
                fields.get(2).and_then(|v| v.parse::<i32>().ok()),
            ) {
                ([kind, _, _], Some(id), Some(qty)) => {
                    info!("[get_resource] instrument: NoticeCollectItem({kind}, {id}, {qty})");
                    openers.notice_collect_item(kind, id, qty, "instrument");
                }
                _ => warn!(
                    "[get_resource] instrument: {:?} is not type:id:qty; skipped",
                    ascii_or(part)
                ),
            }
        }
    }
    // `MOLY_GET_RESOURCE_REWARD_DIALOGS` = `250,262,450`: each shown as its
    // delivery caller shows it, with a payload of the caller's form; the
    // instrument closes it on its close request, as the caller does.
    if let Some(spec) = rewards {
        for part in spec.split(',') {
            let (dialog, layer, payload) = match part.trim() {
                "250" => (
                    COMMON_REWARD_DIALOG,
                    DisplayLayerType::LayerDialog,
                    RewardPayload::Common {
                        message_key: "MSG_RECEIVED_DELIVERY_TOTAL_REWARD".to_owned(),
                        resources: 2,
                    },
                ),
                "262" => (
                    HONOR_REWARD_DIALOG,
                    DisplayLayerType::LayerDialog,
                    RewardPayload::Honor {
                        honor_id: 6830,
                        level: 1,
                    },
                ),
                "450" => (
                    REFRESH_DIALOG,
                    DisplayLayerType::LayerOverlay,
                    RewardPayload::Refresh { party_id: 0 },
                ),
                other => {
                    warn!(
                        "[get_resource] instrument: reward dialog {:?} is not 250, 262 or 450",
                        ascii_or(other)
                    );
                    continue;
                }
            };
            match screens.show_dialog(dialog, layer, DialogBackKey::Close, "instrument") {
                Ok(id) => {
                    screens.open_dialog(id);
                    screens.dialog_open_finished(id);
                    openers.set_reward_payload(id, payload);
                    player.instrument_rewards.push(id);
                    info!(
                        "[get_resource] instrument: {dialog:?} {id:?} shown as its caller shows it"
                    );
                }
                Err(reason) => error!("[get_resource] instrument: {reason}"),
            }
        }
    }
    if let Some(raw) = sketch {
        match raw.trim().parse::<i32>() {
            Ok(id) => {
                let ticket = openers.show_sketch_result(id);
                info!("[get_resource] instrument: ShowSketchResultDialog({id}) ticket {ticket}");
            }
            Err(_) => {
                warn!("[get_resource] instrument: MOLY_GET_RESOURCE_SKETCH is not a blueprint id")
            }
        }
    }
}

/// `MOLY_GET_RESOURCE_AUTOCLOSE_SECS`: a tap at the window's top-left corner
/// (the close area) that many seconds after a dialog shows.
fn autoclose(
    player: &mut GetResourcePlayer,
    top: Option<DialogId>,
    now: f32,
    window: &Window,
    gestures: &mut MessageWriter<GestureEvent>,
) {
    let Some(secs) = instrument_env("MOLY_GET_RESOURCE_AUTOCLOSE_SECS")
        .and_then(|raw| raw.trim().parse::<f32>().ok())
    else {
        return;
    };
    let Some(top) = top else {
        player.shown_since = None;
        return;
    };
    let since = match player.shown_since {
        Some((id, at)) if id == top => at,
        _ => {
            player.shown_since = Some((top, now));
            now
        }
    };
    if now - since < secs || !player.autoclosed.insert(top) {
        return;
    }
    let position = Vec2::new(8.0, 8.0).min(Vec2::new(window.width(), window.height()));
    info!(
        "[get_resource] instrument: tap injected at ({:.0},{:.0}) on {top:?}",
        position.x, position.y
    );
    gestures.write(GestureEvent {
        kind: GestureKind::Tap,
        state: GestureState::End,
        position,
        delta: Vec2::ZERO,
        ui_owned: false,
    });
}

// ---------------------------------------------------------------------------
// Update: taps (modal first)
// ---------------------------------------------------------------------------

/// Whether the source raycast gives a tap at `position` to the close area.
fn on_close_area(
    view: &crate::ui_layout::UiPrefabView,
    layouts: &crate::ui_layout::UiLayouts,
    window: &Window,
    root: &crate::canvas::RootCanvas,
    position: Vec2,
) -> bool {
    let pointer = crate::menu_shell::event_pointer(window, root, position);
    let size = root.size(window);
    let Some(node) = view.raycast_winner(layouts, pointer, size) else {
        return false;
    };
    layouts
        .document(view.key)
        .and_then(|doc| doc.nodes.get(node))
        .is_some_and(|node| node.path.ends_with("CloseArea"))
}

/// Taps while any of these dialogs is drawn: every tap is theirs (the
/// dialog layer is modal). 332: a tap on the thumbnail toggles the name
/// balloon, a tap on the close area closes it. 250, 262 and 450: a tap on
/// the close area sends the close request the back key sends, which the
/// delivery flow answers. A reward dialog without a view is refused: its
/// close request goes out in the frame it shows.
#[allow(clippy::too_many_arguments)]
pub(crate) fn click(
    layouts: Res<crate::ui_layout::UiLayouts>,
    screens: Res<ScreenManager>,
    mut refused: Local<HashSet<DialogId>>,
    views: Query<&crate::ui_layout::UiPrefabView, With<GetResourceRoot>>,
    rewards: Query<(&RewardDialogRoot, &crate::ui_layout::UiPrefabView)>,
    mut gestures: MessageReader<GestureEvent>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut player: ResMut<GetResourcePlayer>,
    mut consumed: ResMut<ActionTapConsumed>,
    mut close_requests: MessageWriter<DialogBackKeyEvent>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    // The sources are read once the 332 document is there.
    if layouts.document(KEY_GET_RESOURCE).is_some() {
        refused.retain(|id| screens.dialogs().any(|(shown, ..)| shown == *id));
        for (id, dialog, state) in screens.dialogs() {
            let Some(key) = reward_key(dialog) else {
                continue;
            };
            if state == DialogState::Closed
                || layouts.document(key).is_some()
                || !refused.insert(id)
            {
                continue;
            }
            error!(
                "[get_resource] {dialog:?} {id:?} refused: its prefab document ({key}) is not in the UI sources; the close request goes to its caller"
            );
            close_requests.write(DialogBackKeyEvent {
                id,
                dialog,
                back_key: DialogBackKey::Close,
            });
        }
    }
    let taps: Vec<Vec2> = gestures
        .read()
        .filter(|event| event.kind.is_tap_family() && event.state == GestureState::End)
        .map(|event| event.position)
        .collect();
    let reward = rewards
        .iter()
        .filter_map(|(root, view)| root.showing.map(|id| (id, root.dialog, view)))
        .last();
    if taps.is_empty() || (player.shown.is_none() && reward.is_none()) {
        return;
    }
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    for position in taps {
        consumed.0 = true;
        if player.shown.is_some() {
            let Ok(view) = views.single() else {
                return;
            };
            let size = root_canvas.size(window);
            let canvas = Vec2::new(
                position.x - window.width() / 2.0,
                window.height() / 2.0 - position.y,
            ) / root_canvas.scale(window);
            let on_thumb = view
                .rect(&layouts, "Content/ThunbmainRoot", size)
                .is_some_and(|rect| rect.active && rect.contains(canvas));
            let shown = player.shown.as_mut().expect("shown");
            if on_thumb && shown.entry.kind != ResourceKind::Blueprint {
                if shown.entry.name.is_some() {
                    shown.balloon = !shown.balloon;
                    info!(
                        "[get_resource] thumbnail tap: name balloon {} (GetResourceName)",
                        if shown.balloon { "open" } else { "shut" }
                    );
                } else {
                    info!(
                        "[get_resource] thumbnail tap: the name balloon stays shut: the music master is not on the roots"
                    );
                }
            } else if on_close_area(view, &layouts, window, root_canvas, position) {
                info!("[get_resource] tap on the close area (allowCloseExternal): Close");
                player.close_requested = true;
            } else {
                info!(
                    "[get_resource] tap inside the window: nothing (the window closes only on its close area)"
                );
            }
        } else if let Some((id, dialog, view)) = reward {
            if on_close_area(view, &layouts, window, root_canvas, position) {
                info!(
                    "[get_resource] {dialog:?} {id:?}: tap on the close area (allowCloseExternal): the close request goes to its caller"
                );
                close_requests.write(DialogBackKeyEvent {
                    id,
                    dialog,
                    back_key: DialogBackKey::Close,
                });
            } else {
                info!("[get_resource] {dialog:?} {id:?}: tap inside the window: nothing");
            }
        }
    }
}
