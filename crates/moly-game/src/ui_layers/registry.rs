//! The screen and dialog registries: which `MenuScreenType` and `DialogType`
//! the source's `ScreenManager` can open, and each screen's `ScreenLayerData`.
//!
//! Screens. `ScreenManager.Awake` fills its screen map from the entry list
//! `Screen/SceneEntry/EntryScreenLayers` in list order: a null entry is
//! skipped and the first entry of a screen type wins. A MySekai screen type is
//! registered exactly when the game's Resources carry its `ScreenLayerData`:
//! 39 of the 52 MySekai ids (600 to 658) do; the other 13 have no data asset
//! and no reference, so a request for one would log "Not Register
//! ScreenLayer" in the source. The table below lists all 52 with the data
//! asset name, so membership is read from the table and the data itself from
//! the region root's screen-layer-data documents, admitted by their manifest
//! rows. A registered screen whose data the root does not carry is a named
//! missing input: it is reported once, and each step that reads a data field
//! reports the field it could not read.
//!
//! Dialogs. `ScreenManager.InstantiateDialog<T>` loads `"Dialog/" +
//! dialogType.ToString()` from Resources; a missing prefab returns null, and
//! the show wrappers return null with it. The table lists the dialog types the
//! MySekai code references (87) and the shared generic dialogs it reuses
//! (20), with their enum values and whether the game's Resources carry the
//! prefab (101 of the 107 do).

use std::collections::{HashMap, HashSet};

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::UiRootManifest;
use serde_json::Value;

/// `Sekai.MenuScreenType`: a screen id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct MenuScreenType(pub(crate) u16);

impl std::fmt::Debug for MenuScreenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.name(), self.0)
    }
}

macro_rules! menu_screen_types {
    ($($name:ident = $id:literal, $asset:literal;)*) => {
        #[allow(non_upper_case_globals, dead_code)]
        impl MenuScreenType {
            $(pub(crate) const $name: MenuScreenType = MenuScreenType($id);)*
        }
        /// Every MySekai screen id with its `ScreenLayerData` asset name in
        /// the game's Resources (`screen/data/<name>`), empty when the game
        /// carries none.
        const MYSEKAI_SCREENS: &[(u16, &str, &str)] = &[$(($id, stringify!($name), $asset),)*];
    };
}

menu_screen_types! {
    MysekaiCommon = 600, "screenlayermysekaicommon";
    MysekaiHUD = 601, "screenlayermysekaihud";
    MysekaiHome = 602, "screenlayermysekaihome";
    CreateFixtureResultDialog = 604, "";
    SiteEditMode = 605, "screenlayersiteeditmode";
    MysekaiInventory = 607, "screenlayermysekaiinventory";
    MysekaiCanvasFurnitureEditMode = 608, "";
    MysekaiDeskCanvasFurnitureEditMode = 609, "";
    MysekaiHarvest = 610, "screenlayermysekaiharvest";
    MysekaiPanelCanvasFurnitureEditMode = 611, "";
    MysekaiJacketCanvasFurnitureEditMode = 612, "";
    MysekaiPenlightCanvasFurnitureEditMode = 613, "";
    MysekaiStandCanvasFurnitureEditMode = 614, "";
    MysekaiCraft = 615, "screenlayermysekaicraft";
    MyeskaiTutorial = 616, "";
    MysekaiScene = 618, "";
    MysekaiTalk = 619, "screenlayermysekaitalk";
    MysekaiPreset = 620, "screenlayermysekaipreset";
    MysekaiMission = 621, "screenlayermysekaimission";
    MysekaiInfo = 622, "screenlayermysekaiinfo";
    MysekaiFriend = 623, "";
    MysekaiOption = 624, "";
    MysekaiSecretShop = 625, "screenlayermysekaisecretshop";
    MysekaiBGMSelect = 626, "screenlayermysekaibgmselect";
    MysekaiConvertFurniture = 627, "screenlayermysekaiconvertfurniture";
    MysekaiGate = 628, "screenlayermysekaigate";
    MysekaiAvatarCostumeSetting = 629, "screenlayermysekaiavatarcostumesetting";
    MysekaiEnvironmentFurniture = 630, "";
    MysekaiBluePrint = 631, "";
    MysekaiPhotoShot = 632, "screenlayermysekaiphotoshot";
    MysekaiNotice = 633, "screenlayermysekainotice";
    MysekaiMyRoom = 634, "screenlayermysekaimyroom";
    MysekaiBlockList = 635, "screenlayermysekaiblocklist";
    MysekaiOtherProfile = 636, "screenlayermysekaiotherprofile";
    MysekaiHarvestSummary = 637, "screenlayermysekaiharvestsummary";
    MysekaiCustomFixtureEditor = 638, "screenlayermysekaicustomfixtureeditor";
    MysekaiCutScene = 639, "screenlayermysekaicutscene";
    MysekaiCanvas = 640, "screenlayermysekaicanvas";
    MysekaiSiteMap = 641, "screenlayermysekaisitemap";
    MysekaiPhotoAlbum = 642, "screenlayermysekaiphotoalbum";
    MysekaiPhotoAlbumPreview = 643, "screenlayermysekaiphotoalbumpreview";
    MysekaiPhotoSelect = 644, "screenlayermysekaiphotoselect";
    MysekaiGateInvitation = 645, "screenlayermysekaigateinvitation";
    MysekaiMultiplayUI = 650, "screenlayermysekaimultiplayui";
    MysekaiSketchUI = 651, "screenlayermysekaisketchui";
    MysekaiHousingCompetition = 652, "screenlayermysekaihousingcompetition";
    MysekaiSiteMove = 653, "screenlayermysekaisitemove";
    MysekaiEditTutorialNavigate = 654, "screenlayermysekaiedittutorialnavigate";
    MysekaiHousingCompetitionBackNumber = 655, "screenlayermysekaihousingcompetitionbacknumber";
    MysekaiDelivery = 656, "screenlayermysekaidelivery";
    MysekaiDeliveryInformation = 657, "screenlayermysekaideliveryinformation";
    MysekaiVisitTop = 658, "screenlayermysekaivisittop";
}

#[allow(non_upper_case_globals, dead_code)]
impl MenuScreenType {
    /// The shared header layer (3). Its `ScreenLayerData` is a shared screen,
    /// not in the MySekai block; the back key reads its back button.
    pub(crate) const Header: MenuScreenType = MenuScreenType(3);
    /// The field screen of the current site (the product's name for the site
    /// controllers' base screen): a change to it resolves by the current site
    /// category, see [`MenuScreenType::field_for_category`].
    pub(crate) const HomeField: MenuScreenType = MenuScreenType::MysekaiHome;
    /// Product names kept for the callers written against the earlier slot table.
    pub(crate) const MysekaiSiteEdit: MenuScreenType = MenuScreenType::SiteEditMode;
    pub(crate) const MysekaiConvert: MenuScreenType = MenuScreenType::MysekaiConvertFurniture;
    pub(crate) const MysekaiBgmSelect: MenuScreenType = MenuScreenType::MysekaiBGMSelect;
}

impl MenuScreenType {
    /// The source enum name.
    pub(crate) fn name(self) -> &'static str {
        if self == Self::Header {
            return "Header";
        }
        MYSEKAI_SCREENS
            .iter()
            .find(|(id, _, _)| *id == self.0)
            .map_or("unnamed", |(_, name, _)| name)
    }

    #[allow(dead_code)]
    pub(crate) fn label(self) -> &'static str {
        self.name()
    }

    /// The `ScreenLayerData` asset name when the game registers the screen.
    pub(crate) fn data_asset(self) -> Option<&'static str> {
        if self == Self::Header {
            return Some("screenlayerheader");
        }
        MYSEKAI_SCREENS
            .iter()
            .find(|(id, _, asset)| *id == self.0 && !asset.is_empty())
            .map(|(_, _, asset)| *asset)
    }

    /// Registered in the source's screen map.
    pub(crate) fn registered(self) -> bool {
        self.data_asset().is_some()
    }

    /// One of the four site controllers' base screens.
    pub(crate) fn is_field(self) -> bool {
        matches!(
            self,
            Self::MysekaiHome | Self::MysekaiMyRoom | Self::MysekaiHarvest | Self::MysekaiDelivery
        )
    }

    /// The base screen each site controller changes to on entering its site
    /// (`HomeSiteController`, `MyRoomSiteController`, `HarvestSiteController`,
    /// `DeliverySiteController`, each `ChangeUIScreen` of its screen).
    pub(crate) fn field_for_category(category: &str) -> Option<Self> {
        match category {
            "housing_home" => Some(Self::MysekaiHome),
            "housing_room" => Some(Self::MysekaiMyRoom),
            "harvest" => Some(Self::MysekaiHarvest),
            "delivery" => Some(Self::MysekaiDelivery),
            _ => None,
        }
    }
}

/// Counts of the screen table, for the registry's load line.
pub(crate) fn screen_counts() -> (usize, usize) {
    let live = MYSEKAI_SCREENS
        .iter()
        .filter(|(_, _, asset)| !asset.is_empty())
        .count();
    (MYSEKAI_SCREENS.len(), live)
}

/// `Sekai.HeaderDisplay`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderDisplay {
    Unrelated,
    Show,
    ShowWithAnimation,
    Hide,
    HideWithAnimation,
    SwapWithAnimation,
    ShowImmediate,
}

impl HeaderDisplay {
    fn from_value(value: i64) -> Result<Self, String> {
        Ok(match value {
            0 => Self::Unrelated,
            1 => Self::Show,
            2 => Self::ShowWithAnimation,
            3 => Self::Hide,
            4 => Self::HideWithAnimation,
            5 => Self::SwapWithAnimation,
            6 => Self::ShowImmediate,
            other => return Err(format!("HeaderDisplay {other} is not a value of the enum")),
        })
    }

    /// The display leaves the element shown.
    pub(crate) fn shows(self) -> bool {
        matches!(
            self,
            Self::Show | Self::ShowWithAnimation | Self::ShowImmediate | Self::SwapWithAnimation
        )
    }
}

/// The `ScreenLayerData` fields the manager reads.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct ScreenLayerData {
    pub(crate) display_layer: i64,
    pub(crate) start_animation: i64,
    pub(crate) exit_animation: i64,
    pub(crate) display_header: HeaderDisplay,
    pub(crate) display_back: HeaderDisplay,
    pub(crate) enable_back: bool,
    pub(crate) enable_tap_screen_animation: bool,
    pub(crate) include_child_canvas: bool,
    pub(crate) bgm_type: i64,
    pub(crate) background_type: i64,
    pub(crate) prefab: String,
    pub(crate) document: String,
}

impl ScreenLayerData {
    fn parse(doc: &Value, path: &str) -> Result<(MenuScreenType, Self), String> {
        let fields = &doc["fields"];
        let int = |name: &str| {
            fields[name]
                .as_i64()
                .ok_or_else(|| format!("{path}: field {name} is not an integer"))
        };
        let flag = |name: &str| {
            fields[name]
                .as_bool()
                .ok_or_else(|| format!("{path}: field {name} is not a bool"))
        };
        let id = int("ScreenType")?;
        let screen = MenuScreenType(
            u16::try_from(id).map_err(|_| format!("{path}: ScreenType {id} out of range"))?,
        );
        if doc["screenType"].as_i64() != Some(id) {
            return Err(format!("{path}: screenType and fields.ScreenType disagree"));
        }
        let data = Self {
            display_layer: int("DisplayLayer")?,
            start_animation: int("StartAnimationType")?,
            exit_animation: int("ExitAnimationType")?,
            display_header: HeaderDisplay::from_value(int("DisplayHeader")?)?,
            display_back: HeaderDisplay::from_value(int("DisplayBackUIScreen")?)?,
            enable_back: flag("EnableBackUIScreen")?,
            enable_tap_screen_animation: flag("EnableTapScreenAnimation")?,
            include_child_canvas: flag("IncludeChildCanvasForAlphaTransiton")?,
            bgm_type: int("bgmType")?,
            background_type: int("BackgroundType")?,
            prefab: doc["prefabLink"]["loadPath"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            document: path.to_owned(),
        };
        Ok((screen, data))
    }
}

/// How the root's screen data loads.
enum Load {
    Source(Handle<JsonAsset>),
    Manifest {
        region: String,
        handle: Handle<JsonAsset>,
    },
    Documents {
        manifest: UiRootManifest,
        pending: Vec<(String, Handle<JsonAsset>)>,
    },
    Done,
}

/// The screen registry: membership from the table, data from the root.
#[derive(Resource)]
pub(crate) struct ScreenRegistry {
    load: Load,
    data: HashMap<MenuScreenType, ScreenLayerData>,
    missing_reported: HashSet<MenuScreenType>,
}

impl ScreenRegistry {
    pub(crate) fn new(server: &AssetServer) -> Self {
        Self {
            load: Load::Source(server.load("moly://source.json")),
            data: HashMap::new(),
            missing_reported: HashSet::new(),
        }
    }

    /// The screen's data when the root carries it; a registered screen
    /// without data is reported once as a named missing input.
    pub(crate) fn data(&mut self, screen: MenuScreenType) -> Option<&ScreenLayerData> {
        if !self.data.contains_key(&screen) && self.missing_reported.insert(screen) {
            warn!(
                "[screen] ScreenLayerData of {screen:?} (screen/data/{}) is not in this root: named missing input; its data-driven steps are skipped",
                screen.data_asset().unwrap_or("-")
            );
        }
        self.data.get(&screen)
    }

    #[allow(dead_code)]
    pub(crate) fn loaded(&self) -> usize {
        self.data.len()
    }
}

/// Update: resolve the region, read the manifest, admit every
/// screen-layer-data row.
pub(crate) fn load(
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut registry: ResMut<ScreenRegistry>,
) {
    loop {
        let next = match &mut registry.load {
            Load::Done => return,
            Load::Source(handle) => {
                if let LoadState::Failed(error) = server.load_state(&*handle) {
                    let (total, live) = screen_counts();
                    warn!(
                        "[screen] registry: source.json did not load ({error}); {live} of {total} MySekai screen ids registered, no ScreenLayerData on this root (named missing input)"
                    );
                    Load::Done
                } else if let Some(asset) = jsons.get(&*handle) {
                    let value: Value = serde_json::from_str(&asset.0)
                        .unwrap_or_else(|e| panic!("[screen] source.json: {e}"));
                    match value["source"]["region"].as_str() {
                        Some(region) if REGION_ROOTS.contains(&region) => Load::Manifest {
                            region: region.to_owned(),
                            handle: server.load(format!("moly://ui-{region}/ui-manifest.json")),
                        },
                        region => {
                            let (total, live) = screen_counts();
                            warn!(
                                "[screen] registry: region {region:?} has no region UI root; {live} of {total} MySekai screen ids registered, their ScreenLayerData is not on this root (named missing input)"
                            );
                            Load::Done
                        }
                    }
                } else {
                    return;
                }
            }
            Load::Manifest { region, handle } => {
                if let LoadState::Failed(error) = server.load_state(&*handle) {
                    panic!(
                        "[screen] registry: the {region} UI root manifest did not load: {error}"
                    );
                }
                let Some(asset) = jsons.get(&*handle) else {
                    return;
                };
                let manifest = UiRootManifest::parse(&asset.0, region)
                    .unwrap_or_else(|e| panic!("[screen] registry: {e}"));
                let root = format!("moly://ui-{region}/");
                let pending: Vec<_> = manifest
                    .documents
                    .iter()
                    .filter(|row| row.kind == "screen-layer-data")
                    .map(|row| {
                        (
                            row.document.clone(),
                            server.load(format!("{root}{}", row.document)),
                        )
                    })
                    .collect();
                info!(
                    "[screen] registry: {} screen-layer-data rows in the {region} UI root manifest",
                    pending.len()
                );
                Load::Documents { manifest, pending }
            }
            Load::Documents { manifest, pending } => {
                let mut parsed = Vec::new();
                let mut waiting = false;
                for (path, handle) in pending.iter() {
                    if let LoadState::Failed(error) = server.load_state(handle) {
                        panic!("[screen] registry: {path} did not load: {error}");
                    }
                    let Some(asset) = jsons.get(handle) else {
                        waiting = true;
                        continue;
                    };
                    manifest
                        .admit(path, "screen-layer-data", &asset.0)
                        .unwrap_or_else(|e| panic!("[screen] registry: {e}"));
                    let doc: Value = serde_json::from_str(&asset.0)
                        .unwrap_or_else(|e| panic!("[screen] registry: {path}: {e}"));
                    parsed.push(
                        ScreenLayerData::parse(&doc, path)
                            .unwrap_or_else(|e| panic!("[screen] registry: {e}")),
                    );
                }
                if waiting {
                    return;
                }
                for (screen, data) in parsed {
                    // EntryScreenLayers: the first entry of a screen type wins.
                    registry.data.entry(screen).or_insert(data);
                }
                let (total, live) = screen_counts();
                let with_data = MYSEKAI_SCREENS
                    .iter()
                    .filter(|(id, _, asset)| {
                        !asset.is_empty() && registry.data.contains_key(&MenuScreenType(*id))
                    })
                    .count();
                let unregistered: Vec<_> = registry
                    .data
                    .keys()
                    .filter(|screen| !screen.registered())
                    .copied()
                    .collect();
                if !unregistered.is_empty() {
                    panic!(
                        "[screen] registry: the root carries ScreenLayerData for ids the table does not register: {unregistered:?}"
                    );
                }
                info!(
                    "[screen] registry: {live} of {total} MySekai screen ids registered; ScreenLayerData on this root for {with_data} of them (+ header: {})",
                    registry.data.contains_key(&MenuScreenType::Header)
                );
                Load::Done
            }
        };
        registry.load = next;
    }
}

/// Regions whose runtime reads its UI from a region root.
const REGION_ROOTS: &[&str] = &["jp"];

/// Whether the MySekai code or the shared generic dialogs own the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogScope {
    MySekai,
    Shared,
}

/// `Sekai.DialogType`: a dialog id (the enum's value).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DialogType(pub(crate) u16);

impl std::fmt::Debug for DialogType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.name(), self.0)
    }
}

#[allow(non_upper_case_globals, dead_code)]
impl DialogType {
    pub(crate) const Common1ButtonDialog: DialogType = DialogType(0);
    pub(crate) const Common2ButtonDialog: DialogType = DialogType(2);
    pub(crate) const MysekaiMenuDialog: DialogType = DialogType(312);
    pub(crate) const MysekaiGetResourceSubWindowDialog: DialogType = DialogType(332);
    pub(crate) const MysekaiEditSaveConfirmationDialog: DialogType = DialogType(310);
    pub(crate) const MysekaiWeatherDialog: DialogType = DialogType(369);
    pub(crate) const LearnPhenomenaSubWindowDialog: DialogType = DialogType(381);
    pub(crate) const MysekaiLeaveConfirmDialog: DialogType = DialogType(465);
}

impl DialogType {
    fn row(self) -> Option<&'static (u16, &'static str, DialogScope, bool)> {
        DIALOG_TYPES.iter().find(|row| row.0 == self.0)
    }

    pub(crate) fn name(self) -> &'static str {
        self.row().map_or("unnamed", |row| row.1)
    }

    /// `"Dialog/" + DialogType.ToString()`, when the game's Resources carry it.
    pub(crate) fn prefab(self) -> Option<String> {
        self.row()
            .filter(|row| row.3)
            .map(|row| format!("Dialog/{}", row.1))
    }

    #[allow(dead_code)]
    pub(crate) fn scope(self) -> Option<DialogScope> {
        self.row().map(|row| row.2)
    }
}

/// Counts of the dialog table: (rows, rows with a prefab) per scope.
pub(crate) fn dialog_counts() -> [(usize, usize); 2] {
    let count = |scope| {
        let rows = DIALOG_TYPES.iter().filter(|row| row.2 == scope);
        (rows.clone().count(), rows.filter(|row| row.3).count())
    };
    [count(DialogScope::MySekai), count(DialogScope::Shared)]
}

/// (enum value, name, owner, the Resources prefab `dialog/<lowercased name>`
/// exists in the game).
const DIALOG_TYPES: &[(u16, &str, DialogScope, bool)] = &[
    (0, "Common1ButtonDialog", DialogScope::Shared, true),
    (2, "Common2ButtonDialog", DialogScope::Shared, true),
    (3, "Common2ButtonMediumDialog", DialogScope::Shared, true),
    (5, "RankUpDialog", DialogScope::Shared, true),
    // `UIUtility.ShowRewardDialog` shows six or more rewards in this list
    // dialog (`ShowCommonRewardVerticalDialog`); the MySekai rank-up on the
    // way home (`MysekaiUtility.ShowPlayerRankUpDialogIfNeededAsync`) passes
    // the rank rewards to it with no unlock topics.
    (
        23,
        "CommonRewardVerticalListDialog",
        DialogScope::Shared,
        true,
    ),
    (
        58,
        "AdditionalDownloadConfirmDialog",
        DialogScope::Shared,
        true,
    ),
    (82, "OptionDialog", DialogScope::Shared, true),
    (87, "ValueShopBuyConfirmDialog", DialogScope::Shared, true),
    (99, "CommonWebviewDialog", DialogScope::Shared, true),
    (103, "ItemDetailDialog", DialogScope::Shared, true),
    (120, "HonorRaritySelectDialog", DialogScope::Shared, true),
    (126, "FindPlayerIdDialog", DialogScope::Shared, true),
    (
        176,
        "ScenarioAllSkipConfirmDialog",
        DialogScope::Shared,
        true,
    ),
    (242, "SubWindowDialog", DialogScope::Shared, true),
    (243, "ConnectingSubWindowDialog", DialogScope::Shared, true),
    (250, "CommonRewardSubWindowDialog", DialogScope::Shared, true),
    (262, "HonorRewardSubWindowDialog", DialogScope::Shared, true),
    (
        266,
        "CardTotalPowerChangeSubWindowDialog",
        DialogScope::Shared,
        true,
    ),
    (281, "PresetSubWindowDialog", DialogScope::MySekai, true),
    (
        282,
        "MysekaiAlertSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        283,
        "MysekaiConvertItemDetailDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        309,
        "CreateFixtureResultDialog",
        DialogScope::MySekai,
        false,
    ),
    (
        310,
        "MysekaiEditSaveConfirmationDialog",
        DialogScope::MySekai,
        true,
    ),
    (311, "MysekaiWorldMapDialog", DialogScope::MySekai, false),
    (312, "MysekaiMenuDialog", DialogScope::MySekai, true),
    (
        313,
        "MysekaiGateLevelUpConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        314,
        "MysekaiGateLevelUpResultDialog",
        DialogScope::MySekai,
        false,
    ),
    (
        315,
        "MysekaiGateSkinReleaseDialog",
        DialogScope::MySekai,
        true,
    ),
    (316, "MysekaiGateDetailDialog", DialogScope::MySekai, true),
    (
        317,
        "MysekaiGateInvitationSelectionSubWindowDialog",
        DialogScope::MySekai,
        false,
    ),
    (
        318,
        "MysekaiGateSendInvitationSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        319,
        "MysekaiGateSendInvitationCompletedDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        320,
        "MysekaiBGMSelectChangeVocalDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        321,
        "MysekaiRecoverBoostStaminaDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        322,
        "BoostStaminaRecoverConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        323,
        "MysekaiConvertStartConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        324,
        "MysekaiConvertFurnitureShortenDialog",
        DialogScope::MySekai,
        true,
    ),
    (325, "MysekaiKickConfirmDialog", DialogScope::MySekai, true),
    (
        326,
        "MysekaiConvertRandomBlueprintSelectSubWindowDialog",
        DialogScope::MySekai,
        false,
    ),
    (
        327,
        "MysekaiConvertRandomRecordSelectSubWindowDialog",
        DialogScope::MySekai,
        false,
    ),
    (
        328,
        "MysekaiInventoryScrapFixtureDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        329,
        "MysekaiInventoryRecycleMaterialDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        330,
        "MysekaiConvertBlueprintLotterySubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        331,
        "MysekaiConvertRecordLotterySubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        332,
        "MysekaiGetResourceSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        333,
        "MysekaiFixtureSearchDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        334,
        "MysekaiPresetConfirmationDialog",
        DialogScope::MySekai,
        true,
    ),
    (335, "MysekaiRankUpDialog", DialogScope::MySekai, true),
    (
        336,
        "MysekaiPhotoEffectSelectSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        337,
        "MysekaiCharaRewardSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        338,
        "MysekaiMissionDetailDialog",
        DialogScope::MySekai,
        true,
    ),
    (339, "MysekaiMissionListDialog", DialogScope::MySekai, true),
    (340, "MysekaiMissionInfoDialog", DialogScope::MySekai, true),
    (
        341,
        "MysekaiMissionCompleteDialog",
        DialogScope::MySekai,
        true,
    ),
    (342, "MysekaiMissionStartDialog", DialogScope::MySekai, true),
    (
        343,
        "MysekaiPlayerStatusUpdateDialog",
        DialogScope::MySekai,
        true,
    ),
    (344, "MysekaiVisitListDialog", DialogScope::MySekai, true),
    (345, "MysekaiRankListDialog", DialogScope::MySekai, true),
    (
        346,
        "MysekaiNotEnoughWhiteBluePrintDialog",
        DialogScope::MySekai,
        true,
    ),
    (347, "DebugFixtureListDialog", DialogScope::MySekai, true),
    (
        348,
        "DebugAddedFixtureListDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        349,
        "DebugModelRoomFixtureListDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        351,
        "DebugModelRoomGateSelectDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        352,
        "MysekaiSecretShopPurchaseConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        353,
        "MysekaiSecretShopHasBlueprintConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        354,
        "MysekaiSecretShopNotHavePermissionToLogInDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        355,
        "MysekaiSecretShopPurchaseLimitReachedDialog",
        DialogScope::MySekai,
        true,
    ),
    (356, "MaterialItemDetailDialog", DialogScope::MySekai, true),
    (
        357,
        "MysekaiExpireSiteHousingPresetExtendSlotConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        358,
        "MysekaiHousingCompetitionSubmittedInfoDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        359,
        "MysekaiHousingCompetitionScreenshotConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        360,
        "MysekaiHousingCompetitionSubmissionDiscardDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        361,
        "MysekaiHousingCompetitionSubmitConfirmDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        362,
        "MysekaiHousingCompetitionSearchSubmitDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        363,
        "MysekaiHousingCompetitionConfirmVisitSubmitDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        364,
        "MysekaiHousingCompetitionConfirmResubmitDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        365,
        "MysekaiHousingCompetitionSiteSelectionForSubmissionDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        366,
        "MysekaiHousingCompetitionSubmitCompletedDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        367,
        "MysekaiHousingCompetitionSubmitIdCopiedDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        368,
        "MysekaiHousingCompetitionEntryDetailDialog",
        DialogScope::MySekai,
        true,
    ),
    (369, "MysekaiWeatherDialog", DialogScope::MySekai, true),
    (
        370,
        "MysekaiBirthdayRewardListDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        377,
        "CraftResultSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (378, "FixtureDescriptionDialog", DialogScope::MySekai, true),
    (
        379,
        "MysekaiCanvasFurnitureEditModeDialog",
        DialogScope::MySekai,
        true,
    ),
    (380, "MysekaiSketchDialog", DialogScope::MySekai, true),
    (
        381,
        "LearnPhenomenaSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        392,
        "CanvasMemberListFilterDialog",
        DialogScope::MySekai,
        true,
    ),
    (395, "UnitFilterDialog", DialogScope::Shared, true),
    (396, "HonorGroupFilterDialog", DialogScope::Shared, true),
    (398, "BGMFilterDialog", DialogScope::MySekai, true),
    (
        405,
        "FixtureBonusSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        408,
        "MysekaiNewHarvestSiteOpenDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        409,
        "VisitModelRoomSelectDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        410,
        "CharacterArchiveMysekaiTalkFilterDialog",
        DialogScope::MySekai,
        true,
    ),
    (411, "FixtureListDialog", DialogScope::MySekai, true),
    (
        413,
        "CharacterArchiveMysekaiGeneralTalkReleaseConditionDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        414,
        "CharacterArchiveMysekaiFixtureTalkReleaseConditionDialog",
        DialogScope::MySekai,
        true,
    ),
    (415, "FixtureShortageDialog", DialogScope::MySekai, true),
    (
        418,
        "TakingScreenshotSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        419,
        "MysekaiHousingCommunityReportDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        420,
        "MysekaiHousingCompetitionCommunityReportDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        421,
        "InValidLayoutFixtureListDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        450,
        "MysekaiRefreshBirthdayPlantSubWindowDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        454,
        "MysekaiFixtureShopItemFilterDialog",
        DialogScope::MySekai,
        true,
    ),
    (
        455,
        "SelectMysekaiFixtureDialog",
        DialogScope::MySekai,
        true,
    ),
    (465, "MysekaiLeaveConfirmDialog", DialogScope::MySekai, true),
];
