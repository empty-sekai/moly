//! Shared prefab view: geometry, images and text use the same resolved nodes as hit testing.

mod filled_image;
mod tmp_font;
mod tmp_layout;
mod clip_render;
mod stencil_mask;
mod raycast;
pub(crate) use raycast::Pointer;
#[cfg(test)]
mod ui_source_compare;

pub(crate) fn install(app: &mut App) {
    clip_render::install(app);
}

use crate::balloon::{BalloonArt, BAKE_PPEM};
use bevy::asset::RenderAssetUsages;
use bevy::asset::io::AssetReaderError;
use bevy::asset::{AssetEvent, AssetId, AssetLoadError, AssetLoadFailedEvent, LoadState};
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use moly_assets::{
    json::JsonAsset,
    residency::load_image,
    ui_layout::{
        auto_layout::{compute_overrides, LayoutMetrics},
        RectTransform, UiComponent, UiPrefab, UiRect, UiRootManifest,
    },
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

const ROOT: &str = "moly://ui-layout-v2/";

/// The shared root's wording dictionary lives outside the shared UI root.
const SHARED_WORDINGS: &str = "moly://wordings.json";

/// Regions whose runtime reads every UI source file from its own
/// region-tagged root (`moly://ui-<region>/`) instead of the shared root.
/// That root's ui-manifest.json admits each file by a row of the file's own
/// kind, region, client version and sha256 (a layout also by its node count
/// and the serialized file it was read from). A file the product asks for
/// that the root does not carry fails the load; it never falls back to the
/// shared root.
const REGION_ROOTS: &[&str] = &["jp"];

/// The UI source files besides the layouts, each with the kind of its row
/// in a region root's manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiSourceFile {
    /// The runtime wording dictionary (on a region root: the built-in
    /// wording CSV with the master wordings applied over it).
    Wordings,
    /// Runtime texture aliases the hosts swap into Images.
    RuntimeTextures,
    /// Authored Sprite metrics of those runtime textures.
    RuntimeSprites,
    /// The TMP settings' line-breaking character sets.
    TextSettings,
    /// The host scene's root Canvas, its scaler, camera and layer Canvases.
    HostCanvas,
    /// The TMP font assets texts are laid out with (a region root only).
    TmpFontAssets,
    /// The colour palette palette-bound graphics read (a region root; the
    /// shared root when it carries one).
    Palette,
    /// The tween defaults tweens without their own settings run with (a
    /// region root; the shared root when it carries them).
    TweenSettings,
}

impl UiSourceFile {
    const ALL: [Self; 8] = [
        Self::Wordings, Self::RuntimeTextures, Self::RuntimeSprites, Self::TextSettings, Self::HostCanvas,
        Self::TmpFontAssets, Self::Palette, Self::TweenSettings,
    ];

    /// Files only a region root carries.
    fn region_only(self) -> bool {
        matches!(self, Self::TmpFontAssets)
    }

    /// Files the shared root may lack: absent, their readers keep their named
    /// fallback; present, each must be well formed and name the runtime's
    /// region in its source block.
    fn shared_optional(self) -> bool {
        matches!(self, Self::Palette | Self::TweenSettings)
    }

    fn file(self) -> &'static str {
        match self {
            Self::Wordings => "wordings.json",
            Self::RuntimeTextures => "textures.json",
            Self::RuntimeSprites => "runtime-sprites.json",
            Self::TextSettings => "text-settings.json",
            Self::HostCanvas => "host-canvas.json",
            Self::TmpFontAssets => "fonts/tmp-font-assets.json",
            Self::Palette => "palette.json",
            Self::TweenSettings => "tween-settings.json",
        }
    }

    fn kind(self) -> &'static str {
        match self {
            Self::Wordings => "wordings",
            Self::RuntimeTextures => "runtime-textures",
            Self::RuntimeSprites => "runtime-sprites",
            Self::TextSettings => "tmp-settings",
            Self::HostCanvas => "host-canvas",
            Self::TmpFontAssets => "tmp-font-assets",
            Self::Palette => "palette",
            Self::TweenSettings => "tween-settings",
        }
    }
}

/// The asset path of an image named inside a UI document: names of the shared
/// root are relative to it, names of a region root were rebased to full paths.
pub(crate) fn image_asset_path(name: &str) -> String {
    if name.contains("://") { name.to_owned() } else { format!("{ROOT}{name}") }
}

/// Whether a load failed because the root does not carry the file.
fn not_found(error: &AssetLoadError) -> bool {
    matches!(error, AssetLoadError::AssetReaderError(AssetReaderError::NotFound(_)))
}

/// The region and client version a settings document of the shared root names
/// in its source block; the region must be the runtime's, since the shared
/// root is read by every region without a root of its own.
fn shared_source(doc: &Value, runtime_region: &str) -> Result<(String, String), String> {
    let source = &doc["source"];
    let region = source["region"].as_str().ok_or("its source block names no region")?;
    let client = source["clientVersion"].as_str().ok_or("its source block names no client version")?;
    if region != runtime_region {
        return Err(format!("it is a {region} document, and the runtime's region is {runtime_region}"));
    }
    Ok((region.to_owned(), client.to_owned()))
}

/// The runtime's source region, from the snapshot identity document.
fn runtime_region(source: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(source).map_err(|e| format!("source.json: {e}"))?;
    if value["version"].as_u64() != Some(1) {
        return Err("source.json has an unsupported or missing identity version".into());
    }
    value["source"]["region"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "source.json has no source region".into())
}

/// Pointer-down ownership uses the same resolved geometry as the prefab view.
/// It only identifies source controls; field-wide decorative graphics are not
/// implicit input blockers. Modal/layer ownership is provided by their hosts.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct PointerUi<'w, 's> {
    layouts: Option<Res<'w, UiLayouts>>,
    dialogs: Res<'w, crate::menu_shell::ShellDialogState>,
    layers: Res<'w, crate::ui_layers::UiLayerStack>,
    editor: Option<Res<'w, crate::fixture_edit::EditView>>,
    views: Query<'w, 's, (Entity, &'static UiPrefabView, &'static GlobalTransform)>,
    hierarchy: Query<'w, 's, (Option<&'static Visibility>, Option<&'static ChildOf>)>,
    root: Option<Res<'w, crate::canvas::RootCanvas>>,
}

impl PointerUi<'_, '_> {
    pub(crate) fn captures(&self, position: Vec2, window: &Window) -> bool {
        if self.dialogs.blocks_field_input() { return true; }
        let editor_layer = self.layers.current() == crate::ui_layers::LayerId::MysekaiSiteEdit;
        if !self.layers.on_field() && !editor_layer { return true; }
        if editor_layer
            && self
                .editor
                .as_deref()
                .is_some_and(|edit| edit.exit_dialog || edit.clean_up_dialog)
        {
            return true;
        }
        let (Some(layouts), Some(root)) = (self.layouts.as_deref(), self.root.as_deref()) else { return false; };
        let canvas = root.size(window);
        let window_size = Vec2::new(window.width(), window.height());
        let screen = Vec3::new(position.x - window_size.x * 0.5, window_size.y * 0.5 - position.y, 0.);
        self.views.iter().any(|(entity, view, transform)| {
            if root_hidden(entity, &self.hierarchy) { return false; }
            let Some(doc) = layouts.document(view.key) else { return false; };
            let Some(rects) = view.resolved(layouts, canvas) else { return false; };
            let local = transform.affine().inverse().transform_point3(screen).truncate();
            if crate::fixture_edit_ui::captures_background(view, layouts, local, canvas) {
                return true;
            }
            // The field camera reset button: ownership by the source raycast
            // (padded hit graphic, raycast filters, highest depth wins) of the
            // pointer its press chain casts.
            if let Some(button) = crate::menu_shell::camera_reset_button(doc) {
                let pointer = crate::menu_shell::event_pointer(window, root, position);
                if view.press_target(layouts, pointer, canvas).is_some_and(|(_, id)| id == button) {
                    return true;
                }
            }
            doc.nodes.iter().zip(rects.iter()).any(|(node, rect)| {
                rect.active && rect.contains(local) && node.components.iter().any(|component| {
                    component.enabled && component.fields.get("m_Interactable").is_some()
                        && (component.class.ends_with("Button")
                            || component.class.ends_with("Toggle")
                            || component.class.ends_with("Slider"))
                })
            })
        })
    }
}

const DOCUMENTS: &[(&str, &str)] = &[
    ("Menu", "menu/MysekaiMenuDialog.json"),
    ("Option", "info/OptionDialog.json"),
    ("Info", "info/ScreenLayerMysekaiInfo.json"),
    ("GetResource", "menu/MysekaiGetResourceSubWindowDialog.json"),
    ("HUD", "hud/ScreenLayerMysekaiHUD.json"),
    ("Talk", "talk/ScreenLayerMysekaiTalk.json"),
    ("Common1", "menu/Common1ButtonDialog.json"),
    ("Common2", "menu/Common2ButtonDialog.json"),
    ("ShellHome", "shell/ScreenLayerMysekaiHome.json"),
    ("ShellMyRoom", "shell/ScreenLayerMysekaiMyRoom.json"),
    ("ShellHarvest", "shell/ScreenLayerMysekaiHarvest.json"),
    ("ShellDelivery", "shell/ScreenLayerMysekaiDelivery.json"),
    ("EditorSource", "editor/ScreenLayerSiteEditMode.json"),
    ("EditorExit", "editor/MysekaiEditSaveConfirmationDialog.json"),
    ("EditorTab", "editor/SiteEditListSelectorCell.json"),
    ("EditorOutdoor", "editor/SiteEditOutDoorContentList.json"),
    ("EditorFloor", "editor/SiteEditFloorContentList.json"),
    ("EditorWall", "editor/SiteEditWallContentList.json"),
    ("EditorEnvironment", "editor/SiteEnvironmentContentList.json"),
    ("EditorRoad", "editor/RoadFixtureContentList.json"),
    ("EditorCell", "editor/FixtureSelectCell.json"),
    ("EditorCategory", "editor/UIPartsLeftTabListMysekaiContentCell.json"),
    ("EditorHeader", "editor/ScreenLayerHeader.json"),
    ("LearnPhenomena", "menu/LearnPhenomenaSubWindowDialog.json"),
    ("Notice", "hud/ScreenLayerMysekaiNotice.json"),
    ("HarvestSummary", "hud/ScreenLayerMysekaiHarvestSummary.json"),
];

/// Documents a root may lack. A missing one (absent, or refused by the
/// region root's manifest) is named once and left out; the view that draws
/// it refuses by name. Every other document is required.
const OPTIONAL_DOCUMENTS: &[(&str, &str)] = &[
    ("CommonReward", "menu/CommonRewardSubWindowDialog.json"),
    ("HonorReward", "menu/HonorRewardSubWindowDialog.json"),
    ("RefreshBirthdayPlant", "menu/MysekaiRefreshBirthdayPlantSubWindowDialog.json"),
    ("HonorImage", "menu/UIPartsHonorImage.json"),
    ("BgmSelect", "bgmselect/ScreenLayerMysekaiBGMSelect.json"),
    ("BgmSelectListCell", "bgmselect/MysekaiBGMSelectListCell.json"),
    ("Inventory", "inventory/ScreenLayerMysekaiInventory.json"),
    ("InventoryFixtureList", "inventory/FixtureContentList.json"),
    ("InventoryMaterialList", "inventory/MaterialContentList.json"),
    ("InventoryItemList", "inventory/ItemContentList.json"),
    ("InventoryToolList", "inventory/ToolContentList.json"),
    ("InventoryItemCell", "inventory/UIPartsItemThumbnailListViewItem.json"),
    ("InventoryTabCell", "inventory/ContentListSelectorCell.json"),
    ("CraftScreen", "craft/ScreenLayerMysekaiCraft.json"),
    ("CraftFixtureList", "craft/CraftFixtureContentList.json"),
    ("CraftCell", "craft/CraftThumbnailViewItem.json"),
    ("CraftResult", "craft/CraftResultSubWindowDialog.json"),
];

#[derive(Resource, Default)]
pub(crate) struct UiLayouts {
    /// Active-theme colour of every colour palette entry, by `ColorEntry`
    /// value (a region root only).
    palette: Option<Vec<[f32; 4]>>,
    /// The tween defaults, when the root carries them (a region root only).
    tween_defaults: Option<TweenDefaults>,
    docs: HashMap<String, UiPrefab>,
    /// Optional documents the root lacks (named once when found missing).
    missing_documents: HashSet<String>,
    /// Wording keys the root lacks that a text has drawn (named once).
    missing_wordings: std::sync::Mutex<HashSet<String>>,
    document_revisions: HashMap<String, u64>,
    images: HashMap<String, Handle<Image>>,
    pub(crate) wordings: HashMap<String, String>,
    runtime_textures: HashMap<String, String>,
    runtime_sprites: Option<Value>,
    metadata_loaded: bool,
    sources_ready: bool,
    document_images: HashMap<String, Vec<String>>,
    image_documents: HashMap<AssetId<Image>, Vec<String>>,
    ready_documents: Mutex<HashSet<String>>,
    loaded_images: Mutex<HashSet<AssetId<Image>>>,
    failed_images: Mutex<HashMap<AssetId<Image>, String>>,
    resolved: Mutex<HashMap<String, ResolvedLayout>>,
    text_rules: Option<tmp_layout::TextRules>,
    measurement_revision: u64,
    canvas_reference_pixels_per_unit: Option<f32>,
    /// The host root Canvas' serialized pixel-perfect flag.
    canvas_pixel_perfect: Option<bool>,
    /// The host UI layer Canvas' sorting order (its override sorting is on).
    ui_layer_sorting_order: Option<i32>,
    runtime_sprite_layouts: HashMap<String, SpriteLayoutMetrics>,
    /// (document, component) pairs already reported as drawn by the plain
    /// sprite path instead of the Image mesh rules.
    image_fallbacks: Mutex<HashSet<(String, i64)>>,
    /// (document, Mask component) pairs already reported as refused by the
    /// stencil consumer.
    stencil_refusals: Mutex<HashSet<(String, i64)>>,
    /// Documents already reported as drawing sprites without an exported
    /// downscale multiplier.
    downscale_defaults: Mutex<HashSet<String>>,
}

struct ResolvedLayout {
    canvas: Vec2,
    document_revision: u64,
    measurement_revision: u64,
    font_metrics_revision: &'static str,
    rects: Arc<Vec<UiRect>>,
}

/// Authored Sprite metrics for an actual runtime replacement, not PNG crop size.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SpriteLayoutMetrics {
    pub(crate) rect_size: Vec2,
    /// left, bottom, right, top in Sprite pixels.
    pub(crate) border: [f32; 4],
    pub(crate) pixels_per_unit: f32,
}

#[derive(Resource)]
pub(crate) struct UiLayoutRequests {
    stage_only: bool,
    /// The runtime identity; the UI root is chosen once it is read.
    source: Option<Handle<JsonAsset>>,
    /// The requests of the chosen UI root.
    sources: Option<UiSources>,
}

/// Every UI source file requested from one root: the runtime region's own
/// root, admitted through its manifest, or the shared root.
struct UiSources {
    root: String,
    /// The runtime's region (the shared root's settings documents must name it).
    runtime_region: String,
    /// None for the shared root.
    region: Option<RegionRoot>,
    files: Vec<(UiSourceFile, Handle<JsonAsset>)>,
    docs: Vec<(&'static str, &'static str, Handle<JsonAsset>)>,
}

struct RegionRoot {
    region: String,
    manifest: Handle<JsonAsset>,
    parsed: Option<UiRootManifest>,
}

impl UiSources {
    /// The stage reads only the Talk layout and needs no editor sprites.
    fn request(server: &AssetServer, runtime_region: &str, region: Option<&str>, stage_only: bool) -> Self {
        let root = region.map_or_else(|| ROOT.to_owned(), |region| format!("moly://ui-{region}/"));
        let docs = DOCUMENTS
            .iter()
            .chain(OPTIONAL_DOCUMENTS)
            .filter(|(name, _)| !stage_only || *name == "Talk")
            .map(|(name, path)| (*name, *path, server.load(format!("{root}{path}"))))
            .collect();
        let files = UiSourceFile::ALL
            .into_iter()
            .filter(|file| !(stage_only && *file == UiSourceFile::RuntimeSprites))
            // The shared root carries no TMP font assets; its palette and tween
            // defaults are optional.
            .filter(|file| region.is_some() || !file.region_only())
            .map(|file| {
                let path = match (region, file) {
                    (None, UiSourceFile::Wordings) => SHARED_WORDINGS.to_owned(),
                    _ => format!("{root}{}", file.file()),
                };
                (file, server.load(path))
            })
            .collect();
        let region = region.map(|region| RegionRoot {
            region: region.to_owned(),
            manifest: server.load(format!("{root}ui-manifest.json")),
            parsed: None,
        });
        Self { root, runtime_region: runtime_region.to_owned(), region, files, docs }
    }

    fn file(&self, file: UiSourceFile) -> Option<&Handle<JsonAsset>> {
        self.files.iter().find(|(candidate, _)| *candidate == file).map(|(_, handle)| handle)
    }
}

pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>, stage: Option<Res<crate::browser_stage::BrowserStage>>) {
    commands.init_resource::<UiLayouts>();
    // The stage draws only the Talk layout, from the same root the runtime's
    // region selects.
    commands.insert_resource(UiLayoutRequests {
        stage_only: stage.is_some(),
        source: Some(server.load("moly://source.json")),
        sources: None,
    });
}

pub(crate) fn parse(
    mut commands: Commands,
    request: Option<ResMut<UiLayoutRequests>>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut layouts: ResMut<UiLayouts>,
) {
    let Some(mut request) = request else {
        return;
    };
    // The root is chosen by the runtime's region: its own root when it has
    // one, else the shared root.
    if request.sources.is_none() {
        let handle = request.source.clone().expect("UI requests carry the runtime identity");
        if let LoadState::Failed(e) = server.load_state(&handle) {
            panic!("UI runtime source identity failed: {e:?}");
        }
        let Some(asset) = json.get(&handle) else {
            return;
        };
        let region = runtime_region(&asset.0).unwrap_or_else(|e| panic!("UI: {e}"));
        let own_root = REGION_ROOTS.contains(&region.as_str());
        if !own_root {
            info!("UI sources for region {region}: the shared UI root {ROOT}");
        }
        let stage_only = request.stage_only;
        request.sources =
            Some(UiSources::request(&server, &region, own_root.then_some(region.as_str()), stage_only));
        request.source = None;
    }
    let stage_only = request.stage_only;
    let sources = request.sources.as_mut().expect("UI root chosen");
    if let Some(region) = sources.region.as_mut() {
        if region.parsed.is_none() {
            if let LoadState::Failed(e) = server.load_state(&region.manifest) {
                panic!("UI {} root manifest failed: {e:?}", region.region);
            }
            let Some(asset) = json.get(&region.manifest) else {
                return;
            };
            region.parsed = Some(
                UiRootManifest::parse(&asset.0, &region.region)
                    .unwrap_or_else(|e| panic!("UI {} root: {e}", region.region)),
            );
        }
    }
    if !layouts.metadata_loaded {
        for (file, handle) in &sources.files {
            if let LoadState::Failed(e) = server.load_state(handle) {
                if sources.region.is_none() && file.shared_optional() && not_found(&e) {
                    continue;
                }
                panic!("UI source {} failed in {}: {e:?}", file.file(), sources.root);
            }
            if json.get(handle).is_none() {
                return;
            }
        }
        let text = |file: UiSourceFile| -> &str {
            let handle = sources.file(file).unwrap_or_else(|| panic!("UI source {} not requested", file.file()));
            &json.get(handle).expect("UI source loaded").0
        };
        if let Some(region) = sources.region.as_ref() {
            let manifest = region.parsed.as_ref().expect("UI region manifest parsed");
            for (file, _) in &sources.files {
                let (_, identity) = manifest
                    .admit(file.file(), file.kind(), text(*file))
                    .unwrap_or_else(|e| panic!("UI {} root: {e}", region.region));
                info!(
                    "UI {} from the {} UI root {}{}; client {} unity {}; document sha256 {}; source sha256 {}",
                    file.kind(), identity.region, sources.root, file.file(), identity.client_version,
                    identity.unity_version, identity.document_sha256,
                    identity.source_sha256.as_deref().unwrap_or("none"),
                );
            }
        }
        let parse_json = |file: UiSourceFile| -> Value {
            serde_json::from_str(text(file)).unwrap_or_else(|e| panic!("UI source {}: {e}", file.file()))
        };
        let words = parse_json(UiSourceFile::Wordings);
        if let Some(region) = sources.region.as_ref() {
            let manifest = region.parsed.as_ref().expect("UI region manifest parsed");
            assert!(
                words["source"]["region"].as_str() == Some(manifest.region.as_str())
                    && words["source"]["clientVersion"].as_str() == Some(manifest.client_version.as_str()),
                "UI {} root: its wording dictionary is not {} {}",
                region.region, manifest.region, manifest.client_version
            );
        }
        let camera_fields = if sources.region.is_some() {
            crate::canvas::RootCameraFields::Required
        } else {
            crate::canvas::RootCameraFields::SharedRootMayLack
        };
        let root_canvas = layouts.apply_host_sources(
            &parse_json(UiSourceFile::HostCanvas),
            &parse_json(UiSourceFile::TextSettings),
            &words,
            camera_fields,
        );
        commands.insert_resource(root_canvas);
        match sources.region.as_ref() {
            Some(region) => {
                let manifest = region.parsed.as_ref().expect("UI region manifest parsed");
                let fonts = tmp_font::TmpFonts::parse(
                    &parse_json(UiSourceFile::TmpFontAssets),
                    &manifest.region,
                    &manifest.client_version,
                )
                .unwrap_or_else(|e| panic!("UI {} root: {e}", region.region));
                layouts.set_tmp_fonts(Some(fonts));
            }
            None => {
                warn!(
                    "UI sources of the shared root carry no TMP font assets: texts are laid out with the \
                     open font's advances and pairs, not the source glyph tables"
                );
                layouts.set_tmp_fonts(None);
            }
        }
        match sources.region.as_ref() {
            Some(region) => {
                layouts.palette = Some(
                    parse_palette(&parse_json(UiSourceFile::Palette))
                        .unwrap_or_else(|e| panic!("UI {} root palette: {e}", region.region)),
                );
                layouts.tween_defaults = Some(
                    TweenDefaults::parse(&parse_json(UiSourceFile::TweenSettings))
                        .unwrap_or_else(|e| panic!("UI {} root tween defaults: {e}", region.region)),
                );
            }
            None => {
                // The shared root's settings, when it carries them: each names
                // the runtime's region, or the root is refused.
                let shared = |file: UiSourceFile| -> Option<Value> {
                    json.get(sources.file(file)?)?;
                    let doc = parse_json(file);
                    let (region, client) = shared_source(&doc, &sources.runtime_region)
                        .unwrap_or_else(|e| panic!("UI shared root {}{}: {e}", sources.root, file.file()));
                    info!("UI {} from the shared UI root {}{}: region {region}, client {client}",
                        file.kind(), sources.root, file.file());
                    Some(doc)
                };
                layouts.palette = shared(UiSourceFile::Palette).map(|doc| {
                    parse_palette(&doc).unwrap_or_else(|e| panic!("UI shared root palette: {e}"))
                });
                layouts.tween_defaults = shared(UiSourceFile::TweenSettings).map(|doc| {
                    TweenDefaults::parse(&doc).unwrap_or_else(|e| panic!("UI shared root tween defaults: {e}"))
                });
                if let Some(palette) = layouts.palette.as_ref() {
                    info!("UI shared root palette: {} colour entries", palette.len());
                } else {
                    warn!(
                        "UI sources of the shared root carry no colour palette: palette-bound graphics keep \
                         their named fallback"
                    );
                }
                if layouts.tween_defaults.is_none() {
                    warn!(
                        "UI sources of the shared root carry no tween defaults: palette-bound button press \
                         effects are not drawn"
                    );
                }
            }
        }
        let textures: HashMap<String, String> = serde_json::from_str(text(UiSourceFile::RuntimeTextures))
            .expect("UI runtime texture inventory");
        // A region root names its images relative to itself; keys become the
        // full asset paths, like the images of its layouts.
        layouts.runtime_textures = textures
            .into_iter()
            .map(|(alias, path)| {
                let path = if sources.region.is_some() && !path.contains("://") {
                    format!("{}{path}", sources.root)
                } else {
                    path
                };
                (alias, path)
            })
            .collect();
        layouts.runtime_sprites = sources
            .file(UiSourceFile::RuntimeSprites)
            .map(|_| parse_json(UiSourceFile::RuntimeSprites));
        // The stage has no editor/menu: do not fetch thousands of unrelated
        // thumbnails and chrome textures. Talk document images are requested
        // by install_document, using the original source geometry.
        let paths: Vec<_> = if stage_only { Vec::new() } else { layouts.runtime_textures.values().cloned().collect() };
        for path in paths {
            layouts
                .images
                .entry(path.clone())
                .or_insert_with(|| load_image(&server, image_asset_path(&path)));
        }
        layouts.metadata_loaded = true;
    }
    let mut pending = false;
    for (name, path, handle) in &sources.docs {
        if layouts.docs.contains_key(*name) || layouts.missing_documents.contains(*name) {
            continue;
        }
        let optional = OPTIONAL_DOCUMENTS.iter().any(|(candidate, _)| candidate == name);
        if let LoadState::Failed(e) = server.load_state(handle) {
            if optional {
                error!(
                    "UI {name}: its document {path} is not in {} ({e}); the view that draws it refuses by name",
                    sources.root
                );
                layouts.missing_documents.insert((*name).to_owned());
                continue;
            }
            panic!("UI {name} source asset failed in {}: {e:?}", sources.root);
        }
        let Some(source) = json.get(handle) else {
            pending = true;
            continue;
        };
        let mut doc = match UiPrefab::parse(&source.0) {
            Ok(doc) => doc,
            Err(e) if optional => {
                error!("UI {name}: its document {path} is refused ({e}); the view that draws it refuses by name");
                layouts.missing_documents.insert((*name).to_owned());
                continue;
            }
            Err(e) => panic!("UI {name}: {e}"),
        };
        match sources.region.as_ref() {
            Some(region) => {
                let manifest = region.parsed.as_ref().expect("UI region manifest parsed");
                let identity = match manifest.admit_prefab(path, &source.0, &doc) {
                    Ok(identity) => identity,
                    Err(e) if optional => {
                        error!(
                            "UI {name}: the {} root refuses its document {path} ({e}); the view that draws it refuses by name",
                            region.region
                        );
                        layouts.missing_documents.insert((*name).to_owned());
                        continue;
                    }
                    Err(e) => panic!("UI {name}: {e}"),
                };
                doc.rebase_images(&sources.root);
                info!(
                    "UI prefab {name}: {} source nodes from the {} UI root {path}; client {} unity {}; \
                     document sha256 {}; serialized file sha256 {}",
                    doc.nodes.len(), identity.region, identity.client_version, identity.unity_version,
                    identity.document_sha256, identity.source_sha256.as_deref().unwrap_or("none"),
                );
            }
            None => {
                // A region-tagged document is admitted only through its region root.
                if let Some(region) = doc.source.region.as_deref() {
                    panic!("UI {name}: a {region} document in the shared UI root");
                }
                info!("UI prefab {name}: {} source nodes", doc.nodes.len());
            }
        }
        layouts.install_document(name, doc, &server);
    }
    if !pending {
        // Text glyphs are baked from all document wordings once. Publish the
        // source set atomically, even though each JSON was parsed only once.
        layouts.sources_ready = true;
        commands.remove_resource::<UiLayoutRequests>();
    }
}

/// `PaletteUtility.GetColor`'s table: the active theme's value of every
/// colour palette entry, indexed by `ColorEntry` (the generated enum lists
/// the entries in the palette's entry order).
fn parse_palette(doc: &Value) -> Result<Vec<[f32; 4]>, String> {
    if doc["version"].as_u64() != Some(1) {
        return Err("unsupported palette document version".into());
    }
    let active = doc["activeThemeId"].as_str().ok_or("no active theme id")?;
    let rows = doc["colorEntries"].as_array().ok_or("no colour entries")?;
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            if row["colorEntry"].as_u64() != Some(index as u64) {
                return Err(format!("colour entry {index} is out of order"));
            }
            let value = row["values"][active].as_array().filter(|v| v.len() == 4)
                .ok_or_else(|| format!("colour entry {index} has no active-theme colour"))?;
            let mut color = [0.0; 4];
            for (channel, number) in color.iter_mut().zip(value) {
                *channel = number.as_f64().ok_or_else(|| format!("colour entry {index} channel is not a number"))? as f32;
            }
            Ok(color)
        })
        .collect()
}

/// DOTween's settings asset, as a tween created without its own ease,
/// update type or auto-kill reads it. The tweens here implement the
/// OutQuad ease on scaled frame time with timeScale 1, killed on
/// completion; other defaults are refused, not approximated.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TweenDefaults;

impl TweenDefaults {
    fn parse(doc: &Value) -> Result<Self, String> {
        if doc["version"].as_u64() != Some(1) {
            return Err("unsupported tween settings document version".into());
        }
        let settings = &doc["settings"];
        let int = |name: &str| settings[name].as_i64().ok_or_else(|| format!("{name} is missing"));
        let flag = |name: &str| settings[name].as_bool().ok_or_else(|| format!("{name} is missing"));
        let float = |name: &str| settings[name].as_f64().ok_or_else(|| format!("{name} is missing"));
        // DG.Tweening.Ease 6 = OutQuad; UpdateType 0 = Normal.
        if int("defaultEaseType")? != 6 {
            return Err("defaultEaseType is not OutQuad".into());
        }
        if int("defaultUpdateType")? != 0 || flag("defaultTimeScaleIndependent")? {
            return Err("default updates are not scaled frame time".into());
        }
        if float("timeScale")? != 1.0 || flag("useSmoothDeltaTime")? {
            return Err("tween time is scaled or smoothed".into());
        }
        if !flag("defaultAutoKill")? {
            return Err("tweens are not killed on completion by default".into());
        }
        Ok(Self)
    }
}

impl UiLayouts {
    /// `PaletteUtility.GetColor(entry)`; None when the root carries no
    /// palette or the palette has no such entry.
    pub(crate) fn palette_color(&self, entry: usize) -> Option<[f32; 4]> {
        self.palette.as_ref()?.get(entry).copied()
    }

    /// The tween defaults, when the root carries them.
    pub(crate) fn tween_defaults(&self) -> Option<TweenDefaults> {
        self.tween_defaults
    }

    /// The host canvas, TMP settings and wording dictionary as the UI reads
    /// them: the root Canvas scaler and camera, the host reference pixels per
    /// unit, the root pixel-perfect flag, the UI layer Canvas' sorting order,
    /// the line-breaking rules and the wordings.
    pub(crate) fn apply_host_sources(
        &mut self,
        host_canvas: &Value,
        text_settings: &Value,
        words: &Value,
        camera_fields: crate::canvas::RootCameraFields,
    ) -> crate::canvas::RootCanvas {
        assert_eq!(host_canvas["version"].as_u64(), Some(1), "UI host canvas version");
        let root_canvas = crate::canvas::RootCanvas::from_host_canvas(host_canvas, camera_fields);
        let reference_ppu = host_canvas["referencePixelsPerUnit"]
            .as_f64()
            .expect("UI host referencePixelsPerUnit") as f32;
        // This is an explicit Mysekai CanvasRoot contract, not a claimed native
        // inheritance rule for every nested Canvas. Per-component data wins.
        self.set_canvas_reference_pixels_per_unit(reference_ppu);
        self.canvas_pixel_perfect = Some(
            host_canvas["rootCanvasFields"]["m_PixelPerfect"]
                .as_bool()
                .expect("UI host root Canvas m_PixelPerfect"),
        );
        self.ui_layer_sorting_order = Some(
            host_canvas["layers"]
                .as_array()
                .and_then(|layers| layers.iter().find(|l| l["name"].as_str() == Some("Layer_UI")))
                .and_then(|layer| layer["canvasFields"]["m_SortingOrder"].as_i64())
                .expect("UI host Layer_UI Canvas sorting order") as i32,
        );
        assert_eq!(text_settings["version"].as_u64(), Some(1), "UI text settings version");
        self.text_rules = Some(tmp_layout::TextRules {
            leading: text_settings["leadingCharacters"]
                .as_str()
                .expect("UI leading character rules")
                .chars()
                .collect(),
            following: text_settings["followingCharacters"]
                .as_str()
                .expect("UI following character rules")
                .chars()
                .collect(),
            modern_hangul: text_settings["useModernHangulLineBreakingRules"]
                .as_bool()
                .expect("UI modern Hangul rule flag"),
            fonts: None,
        });
        self.measurement_revision = self.measurement_revision.wrapping_add(1);
        self.wordings = words["entries"]
            .as_object()
            .expect("wordings entries")
            .iter()
            .filter_map(|(k, v)| v["value"].as_str().map(|v| (k.clone(), v.to_owned())))
            .collect();
        root_canvas
    }

    /// The TMP font assets the root's texts are laid out with (None: the
    /// open font's advances). Set after the host sources.
    fn set_tmp_fonts(&mut self, fonts: Option<tmp_font::TmpFonts>) {
        let rules = self.text_rules.as_mut().expect("UI TMP line rules are loaded before the font assets");
        rules.fonts = fonts.map(Arc::new);
        self.measurement_revision = self.measurement_revision.wrapping_add(1);
    }

    /// The runtime Sprite metrics document of the UI root, once loaded
    /// (none for the stage).
    /// Whether the root's runtime texture inventory (or a host) carries `alias`.
    pub(crate) fn has_runtime_texture(&self, alias: &str) -> bool {
        self.runtime_textures.contains_key(alias)
    }

    pub(crate) fn runtime_sprites(&self) -> Option<&Value> {
        self.runtime_sprites.as_ref()
    }

    /// Register an actual external provider before the first view binds it.
    /// Keys remain complete asset paths in images/runtime_textures. In
    /// particular moly://fixture-thumbnails/... is not relative to ROOT.
    /// Alias rebinding is rejected because existing view caches own the alias.
    pub(crate) fn register_runtime_texture(
        &mut self, alias: &str, path: &str, server: &AssetServer,
    ) -> Handle<Image> {
        let full_path = if path.contains("://") { path.to_owned() }
            else { format!("{ROOT}{path}") };
        if let Some(previous) = self.runtime_textures.get(alias) {
            assert_eq!(previous, &full_path, "runtime UI texture alias was rebound: {alias}");
        }
        let image = self.images.entry(full_path.clone())
            .or_insert_with(|| load_image(server, full_path.clone())).clone();
        self.runtime_textures.insert(alias.to_owned(), full_path);
        image
    }

    /// Replace one runtime-composed document, including its asset ownership
    /// and caches. Source templates must be requested before the initial text
    /// atlas bake: this invalidates glyph draw caches, not the atlas repertoire.
    /// Hosts clear/rebind their view overrides when instance identities change.
    pub(crate) fn replace_runtime_document(
        &mut self,
        key: &str,
        document: UiPrefab,
        server: &AssetServer,
    ) -> Result<(), String> {
        if !self.sources_ready {
            return Err("UI source documents are not ready for runtime composition".into());
        }
        self.install_document(key, document, server);
        Ok(())
    }

    fn document_revision(&self, key: &str) -> u64 {
        self.document_revisions.get(key).copied().unwrap_or(0)
    }

    fn install_document(&mut self, key: &str, document: UiPrefab, server: &AssetServer) {
        let mut paths = Vec::new();
        for node in &document.nodes {
            for component in &node.components {
                for source in [&component.sprite, &component.texture].into_iter().flatten() {
                    if let Some(path) = source["image"].as_str() {
                        paths.push(path.to_owned());
                    }
                }
            }
        }
        paths.sort_unstable();
        paths.dedup();
        // Drop the old reverse associations; retaining them would grow the
        // invalidation fan-out every time the same list is populated again.
        self.image_documents.retain(|_, documents| {
            documents.retain(|document| document != key);
            !documents.is_empty()
        });
        for path in &paths {
            let id = self.images.entry(path.clone())
                .or_insert_with(|| load_image(server, image_asset_path(path))).id();
            self.image_documents.entry(id).or_default().push(key.to_owned());
        }
        self.document_images.insert(key.to_owned(), paths);
        self.ready_documents.lock().expect("UI readiness cache poisoned").remove(key);
        self.resolved.lock().expect("UI layout cache poisoned").remove(key);
        let revision = self.document_revisions.entry(key.to_owned()).or_default();
        *revision = revision.wrapping_add(1);
        self.docs.insert(key.to_owned(), document);
    }

    /// The clicked source control's serialized sound fields (`se` and
    /// `otherSeName`), for `CustomSelectableDefine.PlaySE`. None when the
    /// control has no sound fields or is not interactable.
    pub(crate) fn button_se(
        &self, key: &str, path: &str,
    ) -> Option<(moly_law::ui::custom_button::SeType, String)> {
        let doc = self.document(key)?;
        let mut node = &doc.nodes[doc.find(path).expect("source button path")];
        if let Some(wrapper) = node.components.iter().find(|c| c.fields.get("_button").is_some()) {
            let pointer = wrapper.fields["_button"].as_array().expect("button reference");
            assert_eq!(pointer[0].as_i64(), Some(0), "button reference scope");
            let id = pointer[1].as_i64().expect("button reference identity");
            node = &doc.nodes[doc.find(&format!("@{id}")).expect("wrapped button")];
        }
        let control = node.components.iter().find(|c| c.enabled && c.fields.get("se").is_some())?;
        if !control.fields["m_Interactable"].as_bool().expect("button m_Interactable") { return None; }
        let se = control.fields["se"].as_i64()
            .and_then(moly_law::ui::custom_button::SeType::from_serialized)
            .expect("button SeType");
        let other = control.fields["otherSeName"].as_str().expect("button otherSeName").to_owned();
        Some((se, other))
    }

    /// Reports, once per document and component, an Image drawn as a plain
    /// stretched sprite instead of by the Image mesh rules.
    fn note_image_fallback(&self, doc: &UiPrefab, component: &UiComponent, reason: &str) {
        let first = self
            .image_fallbacks
            .lock()
            .expect("UI image fallback log poisoned")
            .insert((doc.prefab.clone(), component.path_id));
        if first {
            warn!(
                "UI {}: Image @{} is drawn as a plain stretched sprite, not by the Image mesh rules: {reason}",
                doc.prefab, component.path_id
            );
        }
    }

    fn note_stencil_refusal(&self, doc: &UiPrefab, mask: &UiComponent, reason: &str) {
        let first = self
            .stencil_refusals
            .lock()
            .expect("UI stencil refusal log poisoned")
            .insert((doc.prefab.clone(), mask.path_id));
        if first {
            error!(
                "UI {}: Mask @{} is not drawn as a stencil ({reason}); the Graphics under it are not clipped",
                doc.prefab, mask.path_id
            );
        }
    }

    /// The real host Canvas supplies this when it is outside the source prefab.
    pub(crate) fn set_canvas_reference_pixels_per_unit(&mut self, value: f32) {
        assert!(
            value.is_finite() && value > 0.0,
            "invalid UI Canvas reference PPU"
        );
        if self.canvas_reference_pixels_per_unit != Some(value) {
            self.canvas_reference_pixels_per_unit = Some(value);
            self.measurement_revision = self.measurement_revision.wrapping_add(1);
        }
    }

    pub(crate) fn set_runtime_sprite_layout(&mut self, name: &str, metrics: SpriteLayoutMetrics) {
        assert!(
            metrics.rect_size.is_finite()
                && metrics.border.iter().all(|n| n.is_finite())
                && metrics.pixels_per_unit.is_finite()
                && metrics.pixels_per_unit > 0.0,
            "invalid runtime Sprite layout metrics"
        );
        if self.runtime_sprite_layouts.get(name) != Some(&metrics) {
            self.runtime_sprite_layouts.insert(name.to_owned(), metrics);
            self.measurement_revision = self.measurement_revision.wrapping_add(1);
        }
    }

    fn resolve_layout(
        &self,
        doc: &UiPrefab,
        canvas: Vec2,
        visibility: &HashMap<usize, bool>,
        rect_overrides: &HashMap<usize, RectTransform>,
        changes: &HashMap<usize, &Override>,
    ) -> Arc<Vec<UiRect>> {
        let automatic = self.layout_overrides(doc, canvas, visibility, rect_overrides, changes);
        let mut rects = doc.resolve_with(canvas, visibility, &automatic);
        moly_assets::ui_layout::clipping::apply(doc, &mut rects)
            .unwrap_or_else(|error| panic!("UI clipping {}: {error}", doc.prefab));
        Arc::new(rects)
    }

    /// The RectTransforms the layout controllers (layout groups and content
    /// size fitters) drive, merged over the given overrides.
    fn layout_overrides(
        &self,
        doc: &UiPrefab,
        canvas: Vec2,
        visibility: &HashMap<usize, bool>,
        rect_overrides: &HashMap<usize, RectTransform>,
        changes: &HashMap<usize, &Override>,
    ) -> HashMap<usize, RectTransform> {
        let mut measurement_error = None;
        // Each resolution lays the document out as a freshly enabled
        // instance, so each text component's preferred-pass character array
        // starts empty here and lives for this resolution.
        let mut internal: HashMap<i64, Vec<char>> = HashMap::new();
        let automatic = compute_overrides(
            doc,
            canvas,
            visibility,
            rect_overrides,
            |index, axis, size| {
                if measurement_error.is_some() {
                    return None;
                }
                match self.measure_node(doc, index, axis, size, changes.get(&index).copied(), &mut internal) {
                    Ok(metrics) => metrics,
                    Err(error) => {
                        measurement_error = Some(format!("{}: {error}", doc.nodes[index].path));
                        None
                    }
                }
            },
        );
        if let Some(error) = measurement_error {
            panic!("UI content measurement failed: {error}");
        }
        automatic.unwrap_or_else(|error| panic!("UI layout {}: {error}", doc.prefab))
    }

    /// The nodes of a document whose RectTransform its layout controllers
    /// rewrite at this canvas size, with no host overrides.
    #[cfg(test)]
    fn layout_driven_nodes(&self, key: &str, canvas: Vec2) -> Option<Vec<usize>> {
        let doc = self.document(key)?;
        let mut driven: Vec<usize> = self
            .layout_overrides(doc, canvas, &HashMap::new(), &HashMap::new(), &HashMap::new())
            .into_keys()
            .collect();
        driven.sort_unstable();
        Some(driven)
    }

    /// Installs one source layout with no image requests and publishes the
    /// source set, so the research instrument drives the product's own
    /// layout and hit paths on an extracted document.
    #[cfg(test)]
    fn install_for_measurement(&mut self, key: &str, document: UiPrefab) {
        let revision = self.document_revisions.entry(key.to_owned()).or_default();
        *revision = revision.wrapping_add(1);
        self.docs.insert(key.to_owned(), document);
        self.metadata_loaded = true;
        self.sources_ready = true;
    }

    fn measure_node(
        &self,
        doc: &UiPrefab,
        index: usize,
        axis: usize,
        size: Vec2,
        change: Option<&Override>,
        internal: &mut HashMap<i64, Vec<char>>,
    ) -> Result<Option<LayoutMetrics>, String> {
        let mut preferred: Option<f32> = None;
        let preferred_field = if axis == 0 {
            "m_PreferredWidth"
        } else {
            "m_PreferredHeight"
        };
        // These built-in providers have priority zero and min=0/flexible=-1.
        // A positive-priority LayoutElement can make their preferred getter
        // irrelevant; do not demand an unused Sprite/font payload in that case.
        let preferred_overridden = doc.nodes[index].components.iter().any(|c| {
            c.enabled
                && c.class.rsplit('.').next() == Some("LayoutElement")
                && c.fields["m_LayoutPriority"].as_i64().is_some_and(|p| p > 0)
                && c.fields[preferred_field].as_f64().is_some_and(|v| v >= 0.0)
        });
        for comp in doc.nodes[index].components.iter().filter(|c| c.enabled) {
            let value = if comp.fields.get("m_fontSize").is_some() {
                if preferred_overridden {
                    preferred = Some(-1.0);
                    continue;
                }
                let (text, input_box) = match change.and_then(|v| v.text.as_deref()) {
                    Some(text) => (text.to_owned(), false),
                    None => self.text_source(comp),
                };
                let rules = self
                    .text_rules
                    .as_ref()
                    .ok_or("TMP line rules are not loaded")?;
                Some(tmp_layout::preferred_axis(
                    &text,
                    comp,
                    axis,
                    size,
                    rules,
                    input_box,
                    internal.entry(comp.path_id).or_default(),
                )?)
            } else if comp.fields.get("m_Type").is_some() {
                Some(if preferred_overridden {
                    -1.0
                } else {
                    self.image_preferred(comp, axis, change)?
                })
            } else {
                None
            };
            if let Some(value) = value {
                preferred = Some(preferred.map_or(value, |old| old.max(value)));
            }
        }
        // Image and TMP are both priority-zero providers, so their per-property
        // maxima can share one tuple. Serialized LayoutElements stay separate.
        Ok(preferred.map(|preferred| LayoutMetrics {
            min: 0.0,
            preferred,
            flexible: -1.0,
            priority: 0,
        }))
    }

    fn image_preferred(
        &self,
        comp: &UiComponent,
        axis: usize,
        change: Option<&Override>,
    ) -> Result<f32, String> {
        let kind = comp.fields["m_Type"]
            .as_i64()
            .ok_or("Image type is missing")?;
        if !(0..=3).contains(&kind) {
            return Err(format!("unsupported Image type {kind}"));
        }
        let (pixels, pixels_per_unit) =
            if let Some(name) = change.and_then(|v| v.texture.as_deref()) {
                let sprite = self.runtime_sprite_layouts.get(name).ok_or_else(|| {
                    format!("runtime Sprite {name} has no authored layout metrics")
                })?;
                let pixels = if kind == 1 || kind == 2 {
                    sprite.border[axis] + sprite.border[axis + 2]
                } else {
                    sprite.rect_size[axis]
                };
                (pixels, sprite.pixels_per_unit)
            } else {
                // Older extracted records omit `sprite` for the actual null PPtr.
                // Only [0, 0] proves that state; an unresolved external PPtr is not
                // a zero-size Sprite and must still report its missing metadata.
                if comp.fields["m_Sprite"].as_array().is_some_and(|ptr| {
                    ptr.len() == 2 && ptr.iter().all(|part| part.as_i64() == Some(0))
                }) {
                    return Ok(0.0);
                }
                let source = comp
                    .sprite
                    .as_ref()
                    .ok_or("Image Sprite identity is missing")?;
                if source["state"].as_str() == Some("none") {
                    return Ok(0.0);
                }
                if source["state"].as_str() != Some("ok") {
                    return Err("Image Sprite is unresolved".into());
                }
                let dimension = |key: &str, count: usize| -> Result<Vec<f32>, String> {
                    let array = source[key]
                        .as_array()
                        .filter(|a| a.len() == count)
                        .ok_or_else(|| format!("Sprite {key} is missing"))?;
                    array
                        .iter()
                        .map(|v| {
                            v.as_f64()
                                .map(|v| v as f32)
                                .filter(|v| v.is_finite())
                                .ok_or_else(|| format!("invalid Sprite {key}"))
                        })
                        .collect()
                };
                let pixels = if kind == 1 || kind == 2 {
                    let border = dimension("border", 4)?;
                    border[axis] + border[axis + 2]
                } else {
                    dimension("rectSize", 2)?[axis]
                };
                let ppu = source["pixelsPerUnit"]
                    .as_f64()
                    .map(|n| n as f32)
                    .filter(|n| n.is_finite() && *n > 0.0)
                    .ok_or("Sprite pixelsPerUnit is missing")?;
                (pixels, ppu)
            };
        let reference = comp
            .canvas_reference_pixels_per_unit
            .or(self.canvas_reference_pixels_per_unit)
            .filter(|n| n.is_finite() && *n > 0.0)
            .ok_or("Image Canvas referencePixelsPerUnit is not supplied")?;
        Ok(pixels / (pixels_per_unit / reference))
    }

    pub(crate) fn document(&self, key: &str) -> Option<&UiPrefab> {
        self.sources_ready.then(|| self.docs.get(key)).flatten()
    }
    pub(crate) fn ready(&self, key: &str, server: &AssetServer) -> bool {
        if self.document(key).is_none() {
            return false;
        }
        if self
            .ready_documents
            .lock()
            .expect("UI readiness cache poisoned")
            .contains(key)
        {
            return true;
        }
        let ready = self.document_images[key]
            .iter()
            .all(|path| self.image_ready(path, server));
        if ready {
            self.ready_documents
                .lock()
                .expect("UI readiness cache poisoned")
                .insert(key.to_owned());
        }
        ready
    }
    fn image_ready(&self, path: &str, server: &AssetServer) -> bool {
        let handle = self
            .images
            .get(path)
            .unwrap_or_else(|| panic!("UI image path missing: {path}"));
        if let Some(error) = self
            .failed_images
            .lock()
            .expect("UI image failure cache poisoned")
            .get(&handle.id())
        {
            panic!("UI image {path} failed: {error}");
        }
        if self
            .loaded_images
            .lock()
            .expect("UI image cache poisoned")
            .contains(&handle.id())
        {
            return true;
        }
        match server.load_state(handle) {
            LoadState::Loaded => {
                self.loaded_images
                    .lock()
                    .expect("UI image cache poisoned")
                    .insert(handle.id());
                true
            }
            LoadState::Failed(error) => panic!("UI image {path} failed: {error:?}"),
            _ => false,
        }
    }
    fn invalidate_image(&self, id: AssetId<Image>) {
        self.loaded_images
            .lock()
            .expect("UI image cache poisoned")
            .remove(&id);
        // Only assets belonging to these documents can invalidate their initial
        // readiness; overrides are checked by the view which actually uses them.
        if let Some(documents) = self.image_documents.get(&id) {
            let mut ready = self
                .ready_documents
                .lock()
                .expect("UI readiness cache poisoned");
            for key in documents {
                ready.remove(key);
            }
        }
    }
    fn resolved(&self, key: &str, canvas: Vec2) -> Option<Arc<Vec<UiRect>>> {
        // A suspended/minimized window has no drawable canvas. Keep the last
        // layout cache intact; the next usable viewport resolves normally.
        if !canvas.is_finite() || canvas.min_element() <= 0. {
            return None;
        }
        let doc = self.document(key)?;
        let mut cache = self.resolved.lock().expect("UI layout cache poisoned");
        if let Some(entry) = cache.get(key).filter(|entry| {
            entry.canvas == canvas
                && entry.document_revision == self.document_revision(key)
                && entry.measurement_revision == self.measurement_revision
                && entry.font_metrics_revision == tmp_layout::FONT_METRICS_REVISION
        }) {
            return Some(entry.rects.clone());
        }
        let rects = self.resolve_layout(
            doc,
            canvas,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        cache.insert(
            key.to_owned(),
            ResolvedLayout {
                canvas,
                document_revision: self.document_revision(key),
                measurement_revision: self.measurement_revision,
                font_metrics_revision: tmp_layout::FONT_METRICS_REVISION,
                rects: rects.clone(),
            },
        );
        Some(rects)
    }
    pub(crate) fn rect(&self, key: &str, path: &str, canvas: Vec2) -> Option<UiRect> {
        let doc = self.document(key)?;
        let index = doc.find(path).unwrap_or_else(|e| panic!("{e}"));
        Some(self.resolved(key, canvas)?[index].clone())
    }
    /// The text a text component shows before any code sets it. Only
    /// `CustomTextMesh` is drawn: its `Start` runs `UpdateWordingText`,
    /// which assigns the wording of `wordingKey` when `useWordingKey` is on
    /// and the key is not empty, and otherwise leaves the serialized TMP
    /// `m_text` (numeric fields keep it until their presenter supplies a
    /// value). All three fields are serialized by that class and its TMP
    /// base; a layout without one is refused, as is any other text class.
    /// The flag says whether the text is the component's serialized text:
    /// TMP parses the backslash escapes of that one only (its text input
    /// box source), not of a wording `Start` assigns through the text setter.
    /// `WordingManager.Get`: a key the dictionary lacks returns the key
    /// itself, so the client draws the key; each such key is named once.
    pub(crate) fn wording_or_key(&self, key: &str) -> String {
        if let Some(wording) = self.wordings.get(key) {
            return wording.clone();
        }
        let mut named = self.missing_wordings.lock().unwrap_or_else(|poison| poison.into_inner());
        if named.insert(key.to_owned()) {
            warn!("UI wording {key} is not in this root's wordings; its text draws the key, as the client does");
        }
        key.to_owned()
    }
    pub(crate) fn text_source(&self, component: &UiComponent) -> (String, bool) {
        assert_eq!(
            component.class, "Sekai.UI.CustomTextMesh",
            "UI text component {}: no text rule for this class", component.path_id
        );
        let field = |name: &str| {
            component.fields.get(name).unwrap_or_else(|| {
                panic!("UI CustomTextMesh {}: serialized {name} is missing", component.path_id)
            })
        };
        let use_key = field("useWordingKey").as_bool()
            .unwrap_or_else(|| panic!("UI CustomTextMesh {}: useWordingKey is not a bool", component.path_id));
        let key = field("wordingKey").as_str()
            .unwrap_or_else(|| panic!("UI CustomTextMesh {}: wordingKey is not a string", component.path_id));
        let serialized = field("m_text").as_str()
            .unwrap_or_else(|| panic!("UI CustomTextMesh {}: m_text is not a string", component.path_id));
        if use_key && !key.is_empty() {
            (self.wording_or_key(key), false)
        } else {
            (serialized.to_owned(), true)
        }
    }
    /// `CustomTextMesh.SetWordingText` on the CustomTextMesh at `path` of
    /// layout `doc_key`: it stores the key and the arguments (none for the
    /// one-parameter overload), then `UpdateWordingText` assigns
    /// `WordingManager.Get(key)`, passed through `String.Format` when there
    /// are arguments, through the TMP text setter (so without `SetText`'s
    /// no-break spaces), but only when the component's serialized
    /// `useWordingKey` is on and the key is not empty. Otherwise the text
    /// stays as it is (None). The source's lookup yields null for a key its
    /// dictionary lacks, so a missing key is refused here.
    pub(crate) fn set_wording_text(&self, doc_key: &str, path: &str, key: &str, args: Option<&[String]>) -> Option<String> {
        let doc = self.document(doc_key).unwrap_or_else(|| panic!("UI layout {doc_key} is not loaded"));
        let index = doc.find(path).unwrap_or_else(|error| panic!("{error}"));
        let component = doc.nodes[index].components.iter()
            .find(|c| c.class == "Sekai.UI.CustomTextMesh")
            .unwrap_or_else(|| panic!("UI {}: {path} has no CustomTextMesh", doc.prefab));
        let use_key = component.fields["useWordingKey"].as_bool()
            .unwrap_or_else(|| panic!("UI {}: {path} CustomTextMesh lacks useWordingKey", doc.prefab));
        if !use_key || key.is_empty() {
            return None;
        }
        let wording = self.wording_or_key(key);
        Some(match args {
            Some(args) => moly_law::text::custom_text_mesh::format_wording(&wording, args)
                .unwrap_or_else(|error| panic!("UI wording {key}: {error}")),
            None => wording,
        })
    }
    pub(crate) fn text_chars(&self) -> Vec<char> {
        let mut chars = Vec::new();
        for doc in self.docs.values() {
            for n in &doc.nodes {
                for c in &n.components {
                    if c.fields.get("m_fontSize").is_some() {
                        let (text, input_box) = self.text_source(c);
                        chars.extend(text.chars());
                        // An escape can draw a character the text does not spell.
                        if let Ok(processed) = tmp_layout::processing_characters(&text, c, input_box) {
                            chars.extend(processed);
                        }
                    }
                }
            }
        }
        chars.sort_unstable();
        chars.dedup();
        chars
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Override {
    visible: Option<bool>,
    text: Option<String>,
    text_alignment: Option<i64>,
    fill: Option<f32>,
    texture: Option<String>,
    slider: Option<f32>,
    alpha: Option<f32>,
    anchored_position: Option<Vec2>,
    size_delta: Option<Vec2>,
    /// `Graphic.color` set by code on the Graphic component with this id.
    graphic_color: Option<(i64, [f32; 4])>,
    /// `Behaviour.enabled` set by code on the component with this id (read by
    /// the stencil Mask).
    behaviour_enabled: Option<(i64, bool)>,
}

impl Override {
    fn same_paint(&self, other: Option<&Self>) -> bool {
        match other {
            Some(other) => {
                self.text == other.text
                    && self.text_alignment == other.text_alignment
                    && self.fill == other.fill
                    && self.texture == other.texture
                    && self.graphic_color == other.graphic_color
                    && self.behaviour_enabled == other.behaviour_enabled
            }
            None => self.text.is_none()
                && self.text_alignment.is_none()
                && self.fill.is_none()
                && self.texture.is_none()
                && self.graphic_color.is_none()
                && self.behaviour_enabled.is_none(),
        }
    }
}

#[derive(Component)]
pub(crate) struct UiPrefabView {
    pub(crate) key: &'static str,
    layer: usize,
    revision: u64,
    geometry_revision: u64,
    texture_revision: u64,
    overrides: BTreeMap<String, Override>,
    resolved: Mutex<Option<ViewLayout>>,
}

struct ViewLayout {
    key: &'static str,
    document_revision: u64,
    revision: u64,
    canvas: Vec2,
    measurement_revision: u64,
    font_metrics_revision: &'static str,
    rects: Arc<Vec<UiRect>>,
}

impl UiPrefabView {
    pub(crate) fn new(key: &'static str, layer: usize) -> Self {
        Self {
            key,
            layer,
            revision: 0,
            geometry_revision: 0,
            texture_revision: 0,
            overrides: BTreeMap::new(),
            resolved: Mutex::new(None),
        }
    }
    /// A host rebuilding a list must not retain selectors for removed cells.
    pub(crate) fn clear_overrides(&mut self) {
        if self.overrides.is_empty() {
            return;
        }
        self.overrides.clear();
        self.revision = self.revision.wrapping_add(1);
        self.geometry_revision = self.geometry_revision.wrapping_add(1);
        self.texture_revision = self.texture_revision.wrapping_add(1);
        *self.resolved.lock().expect("UI view layout cache poisoned") = None;
    }
    fn update(&mut self, path: &str, edit: impl FnOnce(&mut Override)) {
        let value = self.overrides.entry(path.to_owned()).or_default();
        let old = value.clone();
        edit(value);
        if *value != old {
            self.revision = self.revision.wrapping_add(1);
            if value.visible != old.visible
                || value.slider != old.slider
                || value.anchored_position != old.anchored_position
                || value.size_delta != old.size_delta
                || value.text != old.text
                || value.texture != old.texture
            {
                self.geometry_revision = self.geometry_revision.wrapping_add(1);
            }
            if value.texture != old.texture {
                self.texture_revision = self.texture_revision.wrapping_add(1);
            }
        }
    }
    pub(crate) fn set_visible(&mut self, path: &str, value: bool) {
        if self
            .overrides
            .get(path)
            .is_some_and(|v| v.visible == Some(value))
        {
            return;
        }
        self.update(path, |v| v.visible = Some(value));
    }
    pub(crate) fn set_text(&mut self, path: &str, value: String) {
        if self.overrides.get(path).and_then(|v| v.text.as_ref()) == Some(&value) {
            return;
        }
        self.update(path, |v| v.text = Some(value));
    }
    /// Runtime TMP.alignment on this text instance; do not alter the source
    /// RectTransform or its preferred size to simulate text alignment.
    pub(crate) fn set_text_alignment(&mut self, path: &str, value: i64) {
        if self.overrides.get(path).and_then(|v| v.text_alignment) == Some(value) {
            return;
        }
        self.update(path, |v| v.text_alignment = Some(value));
    }
    pub(crate) fn set_fill(&mut self, path: &str, value: f32) {
        let value = value.clamp(0., 1.);
        if self
            .overrides
            .get(path)
            .is_some_and(|v| v.fill == Some(value))
        {
            return;
        }
        self.update(path, |v| v.fill = Some(value));
    }
    pub(crate) fn set_texture(&mut self, path: &str, name: &str) {
        if self.overrides.get(path).and_then(|v| v.texture.as_deref()) == Some(name) {
            return;
        }
        self.update(path, |v| v.texture = Some(name.to_owned()));
    }
    pub(crate) fn set_slider(&mut self, path: &str, value: f32) {
        let value = value.clamp(0., 1.);
        if self
            .overrides
            .get(path)
            .is_some_and(|v| v.slider == Some(value))
        {
            return;
        }
        self.update(path, |v| v.slider = Some(value));
    }
    pub(crate) fn set_alpha(&mut self, path: &str, value: f32) {
        assert!(value.is_finite(), "non-finite UI alpha");
        let value = value.clamp(0., 1.);
        if self.overrides.get(path).and_then(|v| v.alpha) == Some(value) {
            return;
        }
        self.update(path, |v| v.alpha = Some(value));
    }
    pub(crate) fn set_anchored_position(&mut self, path: &str, value: Vec2) {
        assert!(value.is_finite(), "non-finite UI position");
        if self.overrides.get(path).and_then(|v| v.anchored_position) == Some(value) {
            return;
        }
        self.update(path, |v| v.anchored_position = Some(value));
    }
    /// `Graphic.color = value` on the Image component `graphic` (its node is
    /// the one `@graphic` names).
    pub(crate) fn set_graphic_color(&mut self, graphic: i64, value: [f32; 4]) {
        assert!(value.iter().all(|channel| channel.is_finite()), "non-finite UI graphic colour");
        let path = format!("@{graphic}");
        if self.overrides.get(&path).and_then(|v| v.graphic_color) == Some((graphic, value)) {
            return;
        }
        self.update(&path, |v| v.graphic_color = Some((graphic, value)));
    }
    /// `Behaviour.enabled = value` on the component `component` (its node is
    /// the one `@component` names).
    pub(crate) fn set_behaviour_enabled(&mut self, component: i64, value: bool) {
        let path = format!("@{component}");
        if self.overrides.get(&path).and_then(|v| v.behaviour_enabled) == Some((component, value)) {
            return;
        }
        self.update(&path, |v| v.behaviour_enabled = Some((component, value)));
    }
    pub(crate) fn set_size_delta(&mut self, path: &str, value: Vec2) {
        assert!(value.is_finite(), "non-finite UI size delta");
        if self.overrides.get(path).and_then(|v| v.size_delta) == Some(value) {
            return;
        }
        self.update(path, |v| v.size_delta = Some(value));
    }
    fn resolved(&self, layouts: &UiLayouts, canvas: Vec2) -> Option<Arc<Vec<UiRect>>> {
        if !canvas.is_finite() || canvas.min_element() <= 0. {
            return None;
        }
        let doc = layouts.document(self.key)?;
        let mut cache = self.resolved.lock().expect("UI view layout cache poisoned");
        if let Some(entry) = cache.as_ref().filter(|entry| {
            entry.key == self.key
                && entry.document_revision == layouts.document_revision(self.key)
                && entry.revision == self.geometry_revision
                && entry.canvas == canvas
                && entry.measurement_revision == layouts.measurement_revision
                && entry.font_metrics_revision == tmp_layout::FONT_METRICS_REVISION
        }) {
            return Some(entry.rects.clone());
        }
        let (visibility, rect_overrides, changes) = self.host_state(doc);
        let rects = if self.overrides.is_empty() {
            layouts.resolved(self.key, canvas)?
        } else {
            layouts.resolve_layout(doc, canvas, &visibility, &rect_overrides, &changes)
        };
        *cache = Some(ViewLayout {
            key: self.key,
            document_revision: layouts.document_revision(self.key),
            revision: self.geometry_revision,
            canvas,
            measurement_revision: layouts.measurement_revision,
            font_metrics_revision: tmp_layout::FONT_METRICS_REVISION,
            rects: rects.clone(),
        });
        Some(rects)
    }
    /// The host's overrides as the layout reads them: visibility and
    /// RectTransform overrides by node, and each overridden node's change.
    fn host_state<'a>(
        &'a self,
        doc: &UiPrefab,
    ) -> (HashMap<usize, bool>, HashMap<usize, RectTransform>, HashMap<usize, &'a Override>) {
        let mut visibility = HashMap::new();
        let mut rect_overrides = HashMap::new();
        for (path, value) in &self.overrides {
            let i = doc.find(path).unwrap_or_else(|e| panic!("{e}"));
            if let Some(v) = value.visible {
                visibility.insert(i, v);
            }
            if let Some(position) = value.anchored_position {
                rect_overrides
                    .entry(i)
                    .or_insert_with(|| doc.nodes[i].rect.clone())
                    .anchored_position = position.to_array();
            }
            if let Some(size) = value.size_delta {
                rect_overrides
                    .entry(i)
                    .or_insert_with(|| doc.nodes[i].rect.clone())
                    .size_delta = size.to_array();
            }
            if let Some(amount) = value.slider {
                let slider = doc.nodes[i]
                    .components
                    .iter()
                    .find(|c| c.fields.get("m_HandleRect").is_some())
                    .unwrap_or_else(|| panic!("UI slider component missing {path}"));
                let direction = slider.fields["m_Direction"]
                    .as_i64()
                    .expect("slider direction");
                let axis = if direction < 2 { 0 } else { 1 };
                let reverse = direction == 1 || direction == 3;
                for (field, handle) in [("m_FillRect", false), ("m_HandleRect", true)] {
                    let id = slider.fields[field]
                        .as_array()
                        .and_then(|p| p.get(1))
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    if id == 0 {
                        continue;
                    }
                    let n = doc
                        .find(&format!("@{id}"))
                        .unwrap_or_else(|e| panic!("{e}"));
                    let r = rect_overrides
                        .entry(n)
                        .or_insert_with(|| doc.nodes[n].rect.clone());
                    r.anchors_min = [0., 0.];
                    r.anchors_max = [1., 1.];
                    let v = if reverse { 1. - amount } else { amount };
                    if handle {
                        r.anchors_min[axis] = v;
                        r.anchors_max[axis] = v;
                    } else if reverse {
                        r.anchors_min[axis] = v;
                    } else {
                        r.anchors_max[axis] = v;
                    }
                }
            }
        }
        let changes: HashMap<usize, &Override> = self
            .overrides
            .iter()
            .map(|(path, value)| (doc.find(path).unwrap_or_else(|e| panic!("{e}")), value))
            .collect();
        (visibility, rect_overrides, changes)
    }

    /// The nodes whose RectTransform the layout controllers set in this
    /// view's drawn state: every node they drive that the host does not
    /// override, and host-overridden nodes whose value they change.
    #[cfg(test)]
    fn layout_driven(&self, layouts: &UiLayouts, canvas: Vec2) -> Option<Vec<usize>> {
        let doc = layouts.document(self.key)?;
        let (visibility, rect_overrides, changes) = self.host_state(doc);
        let automatic = layouts.layout_overrides(doc, canvas, &visibility, &rect_overrides, &changes);
        let same = |a: &RectTransform, b: &RectTransform| {
            a.anchors_min == b.anchors_min && a.anchors_max == b.anchors_max && a.pivot == b.pivot
                && a.anchored_position == b.anchored_position && a.size_delta == b.size_delta
                && a.local_scale == b.local_scale && a.local_rotation == b.local_rotation
        };
        let mut driven: Vec<usize> = automatic
            .iter()
            .filter(|(index, rect)| rect_overrides.get(index).is_none_or(|host| !same(host, rect)))
            .map(|(index, _)| *index)
            .collect();
        driven.sort_unstable();
        Some(driven)
    }

    pub(crate) fn rect(&self, layouts: &UiLayouts, path: &str, canvas: Vec2) -> Option<UiRect> {
        let doc = layouts.document(self.key)?;
        let index = doc.find(path).unwrap_or_else(|e| panic!("{e}"));
        self.resolved(layouts, canvas).map(|r| r[index].clone())
    }

    /// The drawn rects of this view (host overrides and layout controllers
    /// applied), one per document node.
    #[cfg(test)]
    fn drawn_rects(&self, layouts: &UiLayouts, canvas: Vec2) -> Option<Arc<Vec<UiRect>>> {
        self.resolved(layouts, canvas)
    }

    /// A check the solved view can fail: the nodes whose world corners are
    /// not all finite, and the nodes with a negative width or height (which
    /// the engine allows, so they are reported, not refused), by path.
    pub(crate) fn solve_check(&self, layouts: &UiLayouts, canvas: Vec2) -> Option<SolveCheck> {
        let doc = layouts.document(self.key)?;
        let rects = self.resolved(layouts, canvas)?;
        let mut check = SolveCheck { nodes: rects.len(), non_finite: Vec::new(), negative_size: Vec::new() };
        for (node, rect) in doc.nodes.iter().zip(rects.iter()) {
            let local = moly_law::ui::image::Rect::from_size_pivot(rect.size.to_array(), rect.pivot.to_array());
            let corners = [
                (local.x, local.y),
                (local.x, local.y + local.height),
                (local.x + local.width, local.y + local.height),
                (local.x + local.width, local.y),
            ];
            let finite = corners.iter().all(|&(x, y)| rect.world.transform_point3(Vec3::new(x, y, 0.)).is_finite());
            if !finite {
                check.non_finite.push(node.path.clone());
            }
            if rect.size.x < 0. || rect.size.y < 0. {
                check.negative_size.push(node.path.clone());
            }
        }
        Some(check)
    }

    /// The node of the graphic the source raycast puts first for `pointer`
    /// over this view: the pointer's enter target.
    pub(crate) fn raycast_winner(&self, layouts: &UiLayouts, pointer: Pointer, canvas: Vec2) -> Option<usize> {
        let doc = layouts.document(self.key)?;
        let rects = self.resolved(layouts, canvas)?;
        let order = layouts.ui_layer_sorting_order.expect("UI host layer sorting order is loaded");
        raycast::winner(doc, &rects, pointer, order)
    }

    /// The selectable (node, component identity) a press of `pointer` goes
    /// to by the source raycast over this view, if any.
    pub(crate) fn press_target(&self, layouts: &UiLayouts, pointer: Pointer, canvas: Vec2) -> Option<(usize, i64)> {
        let graphic = self.raycast_winner(layouts, pointer, canvas)?;
        raycast::selectable_handler(layouts.document(self.key)?, graphic)
    }

    /// Whether `node` is the enter target of `pointer` or one of its
    /// ancestors: the objects the event system's enter and exit reach.
    pub(crate) fn hovers(&self, layouts: &UiLayouts, pointer: Pointer, canvas: Vec2, node: usize) -> bool {
        let Some(doc) = layouts.document(self.key) else { return false; };
        let mut cursor = self.raycast_winner(layouts, pointer, canvas);
        while let Some(i) = cursor {
            if i == node {
                return true;
            }
            cursor = doc.parent(i);
        }
        false
    }

    /// `IsActive()` and `IsInteractable()` of the selectable component `id`
    /// on `node`; None only when the canvas has no area (nothing resolves or
    /// raycasts then). A missing layout, component or `m_Interactable` (a
    /// Selectable field) is refused.
    pub(crate) fn selectable_live(&self, layouts: &UiLayouts, canvas: Vec2, node: usize, id: i64) -> Option<(bool, bool)> {
        let doc = layouts.document(self.key)
            .unwrap_or_else(|| panic!("UI layout {} is not loaded", self.key));
        let component = doc.nodes[node].components.iter().find(|c| c.path_id == id)
            .unwrap_or_else(|| panic!("UI {}: node {node} has no component {id}", doc.prefab));
        let serialized = component.fields["m_Interactable"].as_bool()
            .unwrap_or_else(|| panic!("UI {}: selectable {id} has no m_Interactable", doc.prefab));
        let rects = self.resolved(layouts, canvas)?;
        let active = rects[node].active && component.enabled;
        let interactable = moly_law::ui::raycast::is_interactable(serialized, raycast::groups_allow_interaction(doc, node));
        Some((active, interactable))
    }
}

/// The outcome of `UiPrefabView::solve_check`.
#[derive(Debug, Clone)]
pub(crate) struct SolveCheck {
    pub(crate) nodes: usize,
    pub(crate) non_finite: Vec<String>,
    pub(crate) negative_size: Vec<String>,
}

#[derive(Component)]
pub(crate) struct UiPrefabContents;
#[derive(Default)]
pub(crate) struct ViewCache {
    revision: u64,
    document_revision: u64,
    canvas: Vec2,
    clip_pixel_size: Vec2,
    contents: Option<Entity>,
    key: Option<&'static str>,
    layer: usize,
    nodes: Vec<NodeCache>,
    image_users: HashMap<AssetId<Image>, Vec<usize>>,
    image_dirty: HashSet<usize>,
    glyph_image: Option<AssetId<Image>>,
    image_geometry_revision: u64,
    image_texture_revision: u64,
    measurement_revision: u64,
    font_metrics_revision: Option<&'static str>,
}

#[derive(Clone, PartialEq)]
struct NodeSnapshot {
    rect: UiRect,
    change: Override,
    stencil: Option<clip_render::Stencil>,
}

#[derive(Default)]
struct NodeCache {
    snapshot: Option<NodeSnapshot>,
    entity: Option<Entity>,
    meshes: Vec<Handle<Mesh>>,
    materials: Vec<Handle<ColorMaterial>>,
    clip_materials: Vec<Handle<clip_render::UiClipMaterial>>,
    clip_material_colors: Vec<Color>,
    hidden: bool,
    redraw: bool,
    sprites: Vec<(Entity, Color)>,
    material_colors: Vec<Color>,
    alpha: f32,
}

impl NodeCache {
    fn clear(
        &mut self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<ColorMaterial>,
        clip_materials: &mut Assets<clip_render::UiClipMaterial>,
    ) {
        if let Some(entity) = self.entity.take() {
            commands.entity(entity).try_despawn();
        }
        for handle in self.meshes.drain(..) {
            meshes.remove(handle.id());
        }
        for handle in self.materials.drain(..) {
            materials.remove(handle.id());
        }
        for handle in self.clip_materials.drain(..) {
            clip_materials.remove(handle.id());
        }
        self.clip_material_colors.clear();
        self.snapshot = None;
        self.hidden = false;
        self.redraw = false;
        self.sprites.clear();
        self.material_colors.clear();
        self.alpha = 1.;
    }
    fn apply_alpha(
        &mut self,
        alpha: f32,
        sprites: &mut Query<&mut Sprite>,
        materials: &mut Assets<ColorMaterial>,
        clip_materials: &mut Assets<clip_render::UiClipMaterial>,
    ) {
        if self.alpha == alpha {
            return;
        }
        for &(entity, color) in &self.sprites {
            if let Ok(mut sprite) = sprites.get_mut(entity) {
                sprite.color = color.with_alpha(color.alpha() * alpha);
            }
        }
        for (handle, &color) in self.materials.iter().zip(&self.material_colors) {
            if let Some(material) = materials.get_mut(handle) {
                material.color = color.with_alpha(color.alpha() * alpha);
            }
        }
        for (handle, &color) in self.clip_materials.iter().zip(&self.clip_material_colors) {
            if let Some(material) = clip_materials.get_mut(handle) {
                material.set_color(color.with_alpha(color.alpha() * alpha));
            }
        }
        self.alpha = alpha;
    }
}

fn group_alphas(doc: &UiPrefab, overrides: &HashMap<usize, &Override>) -> Vec<f32> {
    let mut alphas = Vec::with_capacity(doc.nodes.len());
    let mut indices = HashMap::new();
    for (index, node) in doc.nodes.iter().enumerate() {
        let mut alpha = indices
            .get(&node.parent_transform_id)
            .map(|&i: &usize| alphas[i])
            .unwrap_or(1.);
        let groups: Vec<_> = node
            .components
            .iter()
            .filter(|c| c.enabled && c.class == "UnityEngine.CanvasGroup")
            .collect();
        for group in &groups {
            if group.fields["m_IgnoreParentGroups"].as_bool() == Some(true) {
                alpha = 1.;
            }
            let local = overrides
                .get(&index)
                .and_then(|v| v.alpha)
                .unwrap_or_else(|| {
                    group.fields["m_Alpha"].as_f64().expect("CanvasGroup alpha") as f32
                });
            alpha *= local;
        }
        if groups.is_empty() {
            if let Some(value) = overrides.get(&index).and_then(|v| v.alpha) {
                alpha *= value;
            }
        }
        indices.insert(node.transform_id, index);
        alphas.push(alpha);
    }
    alphas
}

fn root_hidden(root: Entity, hierarchy: &Query<(Option<&Visibility>, Option<&ChildOf>)>) -> bool {
    let mut cursor = Some(root);
    while let Some(entity) = cursor {
        let Ok((visibility, parent)) = hierarchy.get(entity) else {
            break;
        };
        match visibility {
            Some(Visibility::Hidden) => return true,
            Some(Visibility::Visible) => return false,
            _ => cursor = parent.map(ChildOf::parent),
        }
    }
    false
}

fn image_path<'a>(
    layouts: &'a UiLayouts,
    comp: &'a moly_assets::ui_layout::UiComponent,
    change: Option<&'a Override>,
) -> Option<&'a str> {
    change
        .and_then(|value| value.texture.as_ref())
        .map(|name| {
            layouts
                .runtime_textures
                .get(name)
                .unwrap_or_else(|| panic!("UI runtime texture missing {name}"))
                .as_str()
        })
        .or_else(|| {
            comp.sprite
                .as_ref()
                .or(comp.texture.as_ref())
                .and_then(|source| source["image"].as_str())
        })
}

fn rebuild_image_users(
    previous: &mut ViewCache,
    doc: &UiPrefab,
    layouts: &UiLayouts,
    overrides: &HashMap<usize, &Override>,
    rects: &[UiRect],
    art: &BalloonArt,
) {
    previous.image_users.clear();
    for (index, (node, rect)) in doc.nodes.iter().zip(rects).enumerate() {
        if !rect.active || rect.size.min_element() <= 0. {
            continue;
        }
        for comp in node.components.iter().filter(|comp| comp.enabled) {
            if comp.class.ends_with("Image") && comp.fields.get("m_Color").is_some() {
                if let Some(path) = image_path(layouts, comp, overrides.get(&index).copied()) {
                    previous
                        .image_users
                        .entry(layouts.images[path].id())
                        .or_default()
                        .push(index);
                }
            }
            if comp.fields.get("m_fontSize").is_some() {
                let (text, input_box) = match overrides.get(&index).and_then(|v| v.text.clone()) {
                    Some(text) => (text, false),
                    None => layouts.text_source(comp),
                };
                let mut drawn: Vec<char> = text.chars().collect();
                if let Ok(processed) = tmp_layout::processing_characters(&text, comp, input_box) {
                    drawn.extend(processed);
                }
                let pages: HashSet<_> = drawn
                    .into_iter()
                    .filter(|ch| art.glyph_cell(*ch).is_some())
                    .map(|ch| art.glyph_image_for(ch).id())
                    .collect();
                for image in pages {
                    previous.image_users.entry(image).or_default().push(index);
                }
            }
        }
    }
}

pub(crate) fn render(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    art: Option<Res<BalloonArt>>,
    windows: Query<&Window>,
    views: Query<(Entity, &UiPrefabView)>,
    mut cache: Local<HashMap<Entity, ViewCache>>,
    hierarchy: Query<(Option<&Visibility>, Option<&ChildOf>)>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    mut image_failures: MessageReader<AssetLoadFailedEvent<Image>>,
    images: Res<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut clip_materials: ResMut<Assets<clip_render::UiClipMaterial>>,
    mut sprites: Query<&mut Sprite>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
) {
    cache.retain(|entity, previous| {
        if views.get(*entity).is_ok() {
            return true;
        }
        for node in &mut previous.nodes {
            node.clear(&mut commands, &mut meshes, &mut materials, &mut clip_materials);
        }
        if let Some(contents) = previous.contents.take() {
            commands.entity(contents).try_despawn();
        }
        false
    });
    for event in image_events.read() {
        let id = match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::LoadedWithDependencies { id } => *id,
            AssetEvent::Unused { .. } => continue,
        };
        layouts.invalidate_image(id);
        if !matches!(event, AssetEvent::Removed { .. }) {
            layouts
                .failed_images
                .lock()
                .expect("UI image failure cache poisoned")
                .remove(&id);
        }
        for previous in cache.values_mut() {
            if let Some(nodes) = previous.image_users.get(&id) {
                previous.image_dirty.extend(nodes.iter().copied());
                for &index in nodes {
                    if let Some(node) = previous.nodes.get_mut(index) {
                        node.redraw = true;
                    }
                }
            }
        }
    }
    for failure in image_failures.read() {
        layouts
            .failed_images
            .lock()
            .expect("UI image failure cache poisoned")
            .insert(failure.id, failure.error.to_string());
        layouts.invalidate_image(failure.id);
        for previous in cache.values_mut() {
            if let Some(nodes) = previous.image_users.get(&failure.id) {
                previous.image_dirty.extend(nodes.iter().copied());
                for &index in nodes {
                    if let Some(node) = previous.nodes.get_mut(index) {
                        node.redraw = true;
                    }
                }
            }
        }
    }
    let (Some(art), Some(root_canvas), Ok(window)) = (art, root_canvas, windows.single()) else {
        return;
    };
    let canvas = root_canvas.size(window);
    if !canvas.is_finite() || canvas.min_element() <= 0. { return; }
    // The current orthographic UI host maps this canvas to the full viewport.
    // Physical pixels (not CSS/logical pixels) provide the source half-pixel
    // projection term after conversion into this canvas's coordinate units.
    let clip_pixel_size = canvas / Vec2::new(window.physical_width() as f32,
        window.physical_height() as f32) * 0.5;
    for (root, view) in &views {
        if root_hidden(root, &hierarchy) {
            continue;
        }
        let Some(doc) = layouts.document(view.key) else {
            continue;
        };
        let previous = cache.entry(root).or_default();
        let clip_pixel_changed = previous.clip_pixel_size != clip_pixel_size;
        let document_revision = layouts.document_revision(view.key);
        let document_changed = previous.document_revision != document_revision;
        let glyph_image = Some(art.glyph_image().id());
        if previous.contents.is_some()
            && !clip_pixel_changed
            && !document_changed
            && previous.key == Some(view.key)
            && previous.layer == view.layer
            && previous.revision == view.revision
            && previous.canvas == canvas
            && previous.image_dirty.is_empty()
            && previous.glyph_image == glyph_image
            && previous.measurement_revision == layouts.measurement_revision
            && previous.font_metrics_revision == Some(tmp_layout::FONT_METRICS_REVISION)
        {
            continue;
        }
        if previous.key != Some(view.key) || previous.layer != view.layer || document_changed {
            for node in &mut previous.nodes {
                node.clear(&mut commands, &mut meshes, &mut materials, &mut clip_materials);
            }
            if let Some(old) = previous.contents.take() {
                commands.entity(old).try_despawn();
            }
            previous.nodes.clear();
            previous.image_users.clear();
            previous.image_dirty.clear();
        }
        let overrides: HashMap<usize, &Override> = view
            .overrides
            .iter()
            .map(|(path, value)| (doc.find(path).unwrap_or_else(|e| panic!("{e}")), value))
            .collect();
        let rects = view.resolved(&layouts, canvas).unwrap();
        let alphas = group_alphas(doc, &overrides);
        let stencils = stencil_mask::resolve(&layouts, doc, &rects, &overrides, &alphas);
        let text_layout_changed = previous.measurement_revision != layouts.measurement_revision
            || previous.font_metrics_revision != Some(tmp_layout::FONT_METRICS_REVISION);
        previous.measurement_revision = layouts.measurement_revision;
        previous.font_metrics_revision = Some(tmp_layout::FONT_METRICS_REVISION);
        let glyph_changed = previous.glyph_image != glyph_image;
        previous.glyph_image = glyph_image;
        if previous.key != Some(view.key)
            || previous.layer != view.layer
            || document_changed
            || previous.canvas != canvas
            || glyph_changed
            || previous.image_geometry_revision != view.geometry_revision
            || previous.image_texture_revision != view.texture_revision
        {
            rebuild_image_users(previous, doc, &layouts, &overrides, &rects, &art);
            previous.image_geometry_revision = view.geometry_revision;
            previous.image_texture_revision = view.texture_revision;
        }
        let contents = match previous.contents {
            Some(entity) => entity,
            None => {
                let entity = commands
                    .spawn((
                        UiPrefabContents,
                        Transform::default(),
                        Visibility::Inherited,
                        RenderLayers::layer(view.layer),
                    ))
                    .id();
                commands.entity(root).add_child(entity);
                previous.contents = Some(entity);
                entity
            }
        };
        previous
            .nodes
            .resize_with(doc.nodes.len(), NodeCache::default);
        for (index, (node, rect)) in doc.nodes.iter().zip(rects.iter()).enumerate() {
            let change = overrides.get(&index).copied();
            let rendered = &mut previous.nodes[index];
            rendered.apply_alpha(alphas[index], &mut sprites, &mut materials, &mut clip_materials);
            let image_dirty = previous.image_dirty.contains(&index)
                || (clip_pixel_changed && rect.clipping.rect.is_some())
                || ((glyph_changed || text_layout_changed)
                    && node
                        .components
                        .iter()
                        .any(|comp| comp.fields.get("m_fontSize").is_some()));
            if !rect.active || rect.size.min_element() <= 0. {
                if !rendered.hidden {
                    if let Some(entity) = rendered.entity {
                        commands.entity(entity).insert(Visibility::Hidden);
                    }
                    rendered.hidden = true;
                }
                rendered.redraw |= image_dirty;
                previous.image_dirty.remove(&index);
                // Keep the last rendered snapshot: changes while hidden are
                // applied when this node actually becomes visible again.
                continue;
            }
            let stencil = stencils.tested[index].as_ref();
            // A stencil-tested Graphic samples its masking graphic's texture.
            let stencil_ready = stencil.and_then(|s| s.texture.as_ref())
                .is_none_or(|texture| images.get(texture).is_some());
            if rendered.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.rect == *rect && snapshot.change.same_paint(change)
                    && snapshot.stencil.as_ref() == stencil
            }) && !image_dirty
                && !rendered.redraw
            {
                if rendered.hidden {
                    let ready = node
                        .components
                        .iter()
                        .filter(|comp| comp.enabled)
                        .filter(|comp| {
                            comp.class.ends_with("Image") && comp.fields.get("m_Color").is_some()
                        })
                        .filter_map(|comp| image_path(&layouts, comp, change))
                        .all(|path| {
                            layouts.image_ready(path, &server)
                                && images.get(&layouts.images[path]).is_some()
                        })
                        && stencil_ready;
                    if ready {
                        if let Some(entity) = rendered.entity {
                            commands.entity(entity).insert(Visibility::Inherited);
                        }
                        rendered.hidden = false;
                    } else {
                        previous.image_dirty.insert(index);
                    }
                }
                continue;
            }
            let snapshot = NodeSnapshot {
                rect: rect.clone(),
                change: change.cloned().unwrap_or_default(),
                stencil: stencil.cloned(),
            };
            let ready = node
                .components
                .iter()
                .filter(|comp| comp.enabled)
                .filter(|comp| {
                    comp.class.ends_with("Image") && comp.fields.get("m_Color").is_some()
                })
                .filter_map(|comp| image_path(&layouts, comp, change))
                .all(|path| {
                    layouts.image_ready(path, &server)
                        && images.get(&layouts.images[path]).is_some()
                })
                && stencil_ready;
            if !ready {
                if !rendered.hidden {
                    if let Some(entity) = rendered.entity {
                        commands.entity(entity).insert(Visibility::Hidden);
                    }
                    rendered.hidden = true;
                }
                previous.image_dirty.insert(index);
                continue;
            }
            rendered.clear(&mut commands, &mut meshes, &mut materials, &mut clip_materials);
            let has_drawing = node.components.iter().any(|comp| {
                comp.enabled
                    && ((comp.class.ends_with("Image") && comp.fields.get("m_Color").is_some())
                        || comp.fields.get("m_fontSize").is_some())
            });
            if !has_drawing {
                rendered.snapshot = Some(snapshot);
                previous.image_dirty.remove(&index);
                continue;
            }
            let node_contents = commands
                .spawn((
                    Transform::default(),
                    Visibility::Inherited,
                    RenderLayers::layer(view.layer),
                ))
                .id();
            commands.entity(contents).add_child(node_contents);
            rendered.entity = Some(node_contents);
            let (scale, rotation, _) = rect.world.to_scale_rotation_translation();
            let center = rect.center();
            let depth = 20. + index as f32 * 0.02;
            let base_transform = Transform {
                translation: Vec3::new(center.x, center.y, depth),
                rotation,
                scale,
            };
            for comp in &node.components {
                if !comp.enabled {
                    continue;
                }
                let f = &comp.fields;
                // A masking graphic that does not show writes no colour.
                if stencils.unshown[index] == Some(comp.path_id) {
                    continue;
                }
                if comp.class.ends_with("Image") && f.get("m_Color").is_some() {
                    let clip = clip_render::with_stencil(comp, rect, stencil);
                    let path = image_path(&layouts, comp, change);
                    let image = path.map(|p| layouts.images[p].clone()).unwrap_or_default();
                    // A colour set by code reaches the vertices as Color32.
                    let color = match change.and_then(|c| c.graphic_color).filter(|(id, _)| *id == comp.path_id) {
                        Some((_, value)) => {
                            let [r, g, b, a] = moly_law::ui::graphic_tap_effect::vertex_color(value);
                            Color::srgba(r, g, b, a)
                        }
                        None => serialized_rgba(f, "m_Color"),
                    };
                    let sprite = Sprite {
                        image,
                        color,
                        custom_size: Some(rect.size),
                        ..default()
                    };
                    if let Some(mesh) = path.and_then(|_| {
                        image_rule_mesh(&layouts, doc, index, comp, change, rect)
                    }) {
                        if let Some(clip) = clip {
                            clip_render::Draw { commands: &mut commands, images: &images,
                                meshes: &mut meshes, materials: &mut clip_materials,
                                pixel_size: clip_pixel_size, layer: view.layer
                            }.mesh(node_contents, rendered, mesh, base_transform,
                                sprite.color, Some(sprite.image.clone()), clip, alphas[index]);
                            continue;
                        }
                        let material = materials.add(ColorMaterial {
                            color: sprite.color.with_alpha(sprite.color.alpha() * alphas[index]),
                            texture: Some(sprite.image.clone()),
                            ..default()
                        });
                        let mesh = meshes.add(mesh);
                        rendered.meshes.push(mesh.clone());
                        rendered.materials.push(material.clone());
                        rendered.material_colors.push(sprite.color);
                        let entity = commands
                            .spawn((
                                Mesh2d(mesh),
                                MeshMaterial2d(material),
                                base_transform,
                                RenderLayers::layer(view.layer),
                            ))
                            .id();
                        commands.entity(node_contents).add_child(entity);
                        continue;
                    }
                    if f["m_Type"].as_i64() == Some(3) {
                        let fill = change
                            .and_then(|c| c.fill)
                            .unwrap_or_else(|| serialized_number(f, "m_FillAmount"));
                        let method = filled_image::FillMethod::from_serialized(
                            f["m_FillMethod"].as_i64().expect("Image fill method"),
                        );
                        let origin = f["m_FillOrigin"].as_u64().expect("Image fill origin") as usize;
                        let clockwise = f["m_FillClockwise"].as_bool().expect("Image fill direction");
                        let Some(mesh) = filled_image::mesh(rect.size, method, fill, origin, clockwise) else {
                            continue;
                        };
                        if let Some(clip) = clip {
                            clip_render::Draw { commands: &mut commands, images: &images,
                                meshes: &mut meshes, materials: &mut clip_materials,
                                pixel_size: clip_pixel_size, layer: view.layer
                            }.mesh(node_contents, rendered, mesh, base_transform,
                                sprite.color, path.map(|_| sprite.image.clone()), clip, alphas[index]);
                            continue;
                        }
                        let material = materials.add(ColorMaterial {
                            color: sprite.color.with_alpha(sprite.color.alpha() * alphas[index]),
                            texture: path.map(|_| sprite.image.clone()),
                            ..default()
                        });
                        let mesh = meshes.add(mesh);
                        rendered.meshes.push(mesh.clone());
                        rendered.materials.push(material.clone());
                        rendered.material_colors.push(sprite.color);
                        let entity = commands.spawn((
                            Mesh2d(mesh), MeshMaterial2d(material), base_transform,
                            RenderLayers::layer(view.layer),
                        )).id();
                        commands.entity(node_contents).add_child(entity);
                        continue;
                    }
                    if let Some(clip) = clip {
                        clip_render::Draw { commands: &mut commands, images: &images,
                            meshes: &mut meshes, materials: &mut clip_materials,
                            pixel_size: clip_pixel_size, layer: view.layer
                        }.sprite(node_contents, rendered, &sprite, base_transform, clip, alphas[index]);
                        continue;
                    }
                    let entity = commands
                        .spawn((
                            Sprite {
                                color: sprite
                                    .color
                                    .with_alpha(sprite.color.alpha() * alphas[index]),
                                ..sprite.clone()
                            },
                            base_transform,
                            RenderLayers::layer(view.layer),
                        ))
                        .id();
                    rendered.sprites.push((entity, sprite.color));
                    commands.entity(node_contents).add_child(entity);
                }
                if f.get("m_fontSize").is_some() {
                    let (text, input_box) = match change.and_then(|v| v.text.clone()) {
                        Some(text) => (text, false),
                        None => layouts.text_source(comp),
                    };
                    spawn_text(
                        &mut commands,
                        node_contents,
                        &art,
                        &text,
                        input_box,
                        comp,
                        change.and_then(|v| v.text_alignment),
                        rect,
                        layouts
                            .text_rules
                            .as_ref()
                            .expect("UI TMP line rules are loaded"),
                        depth + 0.005,
                        view.layer,
                        alphas[index],
                        rendered,
                        clip_render::with_stencil(comp, rect, stencil),
                        &images,
                        &mut meshes,
                        &mut clip_materials,
                        clip_pixel_size,
                    );
                }
            }
            rendered.snapshot = Some(snapshot);
            rendered.alpha = alphas[index];
            previous.image_dirty.remove(&index);
        }
        previous.key = Some(view.key);
        previous.document_revision = document_revision;
        previous.layer = view.layer;
        previous.revision = view.revision;
        previous.canvas = canvas;
        previous.clip_pixel_size = clip_pixel_size;
    }
}

/// The effective `Canvas.pixelPerfect` a Graphic at `index` reads: the
/// nearest enclosing Canvas that overrides pixel-perfect decides, otherwise
/// the host root Canvas does. (Nested canvases without the override inherit;
/// that inheritance is the engine's native getter and was not read from its
/// binary.)
fn effective_pixel_perfect(layouts: &UiLayouts, doc: &UiPrefab, index: usize) -> bool {
    let mut cursor = Some(index);
    while let Some(i) = cursor {
        if let Some(canvas) = doc.nodes[i].components.iter().find(|c| {
            c.enabled && c.class == "UnityEngine.Canvas"
                && c.fields["m_OverridePixelPerfect"].as_bool() == Some(true)
        }) {
            return canvas.fields["m_PixelPerfect"].as_bool().expect("Canvas m_PixelPerfect");
        }
        cursor = doc.parent(i);
    }
    layouts.canvas_pixel_perfect.expect("UI host root Canvas pixel-perfect flag is loaded")
}

/// `Image.OnPopulateMesh` for Simple and Sliced images as the renderer
/// uploads it; None when the component is drawn by another path: a raw image
/// (no Image type), a Filled image, or (reported once per document and
/// component) an Image the rules cannot draw, which the caller then draws as
/// a plain stretched sprite.
fn image_rule_mesh(
    layouts: &UiLayouts,
    doc: &UiPrefab,
    index: usize,
    comp: &UiComponent,
    change: Option<&Override>,
    rect: &UiRect,
) -> Option<Mesh> {
    match image_rule_mesh_data(layouts, doc, index, comp, change, rect) {
        Ok(data) => Some(data.into_mesh()),
        Err(ImageDraw::OtherPath) => None,
        Err(ImageDraw::Fallback(reason)) => {
            layouts.note_image_fallback(doc, comp, reason);
            None
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ImageDraw {
    /// Drawn by its own path (raw image, Filled).
    OtherPath,
    /// Not drawable by the Image rules; the plain sprite path draws it.
    Fallback(&'static str),
}

/// The vertex data of an Image drawn by the Image rules, exactly as the
/// renderer uploads it: positions relative to the node rect's centre (the
/// mesh entity sits at that centre), UVs in the exported top-down image,
/// and the triangle list. The rules are given `Graphic.color` white; the
/// renderer tints through the material colour instead, so `rule_colors`
/// (the rules' Color32 output) is not uploaded. `crop` (x, y, width, height
/// in texture texels) and `texture_size` map the UVs back to texture space.
#[derive(Debug, Clone)]
struct ImageMeshData {
    positions: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    // The rules' colours, the crop and the texture size are read by the
    // research instrument only.
    #[cfg_attr(not(test), allow(dead_code))]
    rule_colors: Vec<[u8; 4]>,
    #[cfg_attr(not(test), allow(dead_code))]
    crop: [f32; 4],
    #[cfg_attr(not(test), allow(dead_code))]
    texture_size: [f32; 2],
}

impl ImageMeshData {
    fn into_mesh(self) -> Mesh {
        let count = self.positions.len();
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; count])
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
            .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// The doc-to-mesh mapping of the Image rule path.
///
/// The exported image of an atlas sprite is its texture rect snapped to whole
/// texels (min corner rounded, size as exported), so atlas UVs map into it by
/// that crop. A runtime replacement with authored metrics is a whole-image
/// sprite. A replacement without authored Sprite metrics has no rect size,
/// border or pixels per unit to give the rules, so the rules do not draw it
/// (the same case the preferred-size query refuses).
#[allow(clippy::too_many_arguments)]
fn image_rule_mesh_data(
    layouts: &UiLayouts,
    doc: &UiPrefab,
    index: usize,
    comp: &UiComponent,
    change: Option<&Override>,
    rect: &UiRect,
) -> Result<ImageMeshData, ImageDraw> {
    use moly_law::ui::image as rule;
    let f = &comp.fields;
    // A raw image serializes no Image type; its texture is its own path.
    let Some(serialized_type) = f.get("m_Type") else { return Err(ImageDraw::OtherPath); };
    let image_type = serialized_type.as_i64().and_then(rule::ImageType::from_serialized)
        .unwrap_or_else(|| panic!("UI {}: Image @{} has an unknown m_Type {serialized_type}", doc.prefab, comp.path_id));
    match image_type {
        rule::ImageType::Simple | rule::ImageType::Sliced => {}
        rule::ImageType::Filled => return Err(ImageDraw::OtherPath),
        rule::ImageType::Tiled => return Err(ImageDraw::Fallback("Tiled images have no draw path")),
    }
    let (sprite, crop) = if let Some(name) = change.and_then(|v| v.texture.as_deref()) {
        let metrics = layouts.runtime_sprite_layouts.get(name)
            .ok_or(ImageDraw::Fallback("its runtime replacement has no authored Sprite metrics"))?;
        let size = metrics.rect_size;
        let sprite = rule::SpriteData {
            rect_size: size.to_array(),
            border: metrics.border,
            pixels_per_unit: metrics.pixels_per_unit,
            texture_rect: [0.0, 0.0, size.x, size.y],
            texture_rect_offset: [0.0, 0.0],
            downscale_multiplier: 1.0,
            texture_size: Some(size.to_array()),
        };
        (sprite, [0.0, 0.0, size.x, size.y])
    } else {
        let source = comp
            .sprite
            .as_ref()
            .filter(|v| v["state"].as_str() == Some("ok"))
            .ok_or(ImageDraw::Fallback("its sprite was not exported"))?;
        let array = |value: &Value, count: usize| -> Option<Vec<f32>> {
            let items = value.as_array().filter(|a| a.len() == count)?;
            items.iter().map(|v| v.as_f64().map(|n| n as f32)).collect()
        };
        let missing = ImageDraw::Fallback(
            "its sprite lacks the texture rect, offset, rect size, border, exported size or texture size",
        );
        let texture_rect = array(&source["textureRect"], 4).ok_or(missing)?;
        let offset = array(&source["textureRectOffset"], 2).ok_or(missing)?;
        let rect_size = array(&source["rectSize"], 2).ok_or(missing)?;
        let border = array(&source["border"], 4).ok_or(missing)?;
        let exported = array(&source["size"], 2).ok_or(missing)?;
        let texture_size = array(&source["texture"]["size"], 2).ok_or(missing)?;
        let downscale_multiplier = sprite_downscale_multiplier(layouts, doc, comp, source)?;
        let sprite = rule::SpriteData {
            rect_size: [rect_size[0], rect_size[1]],
            border: [border[0], border[1], border[2], border[3]],
            pixels_per_unit: source["pixelsPerUnit"]
                .as_f64()
                .ok_or(ImageDraw::Fallback("its sprite lacks pixels per unit"))? as f32,
            texture_rect: [texture_rect[0], texture_rect[1], texture_rect[2], texture_rect[3]],
            texture_rect_offset: [offset[0], offset[1]],
            downscale_multiplier,
            texture_size: Some([texture_size[0], texture_size[1]]),
        };
        let crop = [texture_rect[0].round(), texture_rect[1].round(), exported[0], exported[1]];
        (sprite, crop)
    };
    let local = rule::Rect::from_size_pivot(rect.size.to_array(), rect.pivot.to_array());
    let Some(pixel_adjusted) = rule::pixel_adjusted_rect(local, effective_pixel_perfect(layouts, doc, index)) else {
        error_once!("UI Image under a pixel-perfect Canvas: the native pixel-adjusted rect is not ported; drawn unsliced");
        return Err(ImageDraw::Fallback("the pixel-adjusted rect of a pixel-perfect canvas is not ported"));
    };
    let field = |name: &str| -> &Value {
        f.get(name).unwrap_or_else(|| panic!("UI {}: Image @{} lacks {name}", doc.prefab, comp.path_id))
    };
    let flag = |name: &str| -> bool {
        field(name).as_bool().unwrap_or_else(|| panic!("UI {}: Image @{} {name} is not a bool", doc.prefab, comp.path_id))
    };
    let input = rule::ImageInput {
        image_type,
        sprite: Some(&sprite),
        rect: local,
        pixel_adjusted_rect: pixel_adjusted,
        pivot: rect.pivot.to_array(),
        color: [1.0; 4],
        preserve_aspect: flag("m_PreserveAspect"),
        fill_center: flag("m_FillCenter"),
        use_sprite_mesh: flag("m_UseSpriteMesh"),
        pixels_per_unit_multiplier: field("m_PixelsPerUnitMultiplier").as_f64().unwrap_or_else(|| {
            panic!("UI {}: Image @{} m_PixelsPerUnitMultiplier is not a number", doc.prefab, comp.path_id)
        }) as f32,
        reference_pixels_per_unit: comp.canvas_reference_pixels_per_unit.or(layouts.canvas_reference_pixels_per_unit),
    };
    let rule::Populated::Mesh(stream) = rule::populate(&input) else {
        return Err(ImageDraw::Fallback("the Image rules do not port this configuration (sprite mesh)"));
    };
    let texture_size = sprite.texture_size.expect("rule sprite carries its texture size");
    // Positions relative to the rect centre; UVs from texture space into the
    // exported (top-down) image.
    let centre = (Vec2::splat(0.5) - rect.pivot) * rect.size;
    let mut positions = Vec::with_capacity(stream.vertices.len());
    let mut uvs = Vec::with_capacity(stream.vertices.len());
    let mut rule_colors = Vec::with_capacity(stream.vertices.len());
    for vertex in &stream.vertices {
        positions.push([vertex.position[0] - centre.x, vertex.position[1] - centre.y, 0.0]);
        let texel = [vertex.uv0[0] * texture_size[0], vertex.uv0[1] * texture_size[1]];
        uvs.push([(texel[0] - crop[0]) / crop[2], 1.0 - (texel[1] - crop[1]) / crop[3]]);
        rule_colors.push(vertex.color);
    }
    Ok(ImageMeshData { positions, uvs, indices: stream.indices, rule_colors, crop, texture_size })
}

/// The render data's downscale multiplier of an exported sprite. A layout
/// of a region root carries it (the atlas entry's for a packed sprite, else
/// the sprite's own render data); a shared-root layout was exported before
/// the field was and is drawn with 1.0, reported once per document. The
/// exported image is cropped from the texture at the texture rect, so the
/// UV-to-crop mapping above holds for 1.0 only; any other value is not drawn
/// by the rules.
fn sprite_downscale_multiplier(
    layouts: &UiLayouts,
    doc: &UiPrefab,
    comp: &UiComponent,
    source: &Value,
) -> Result<f32, ImageDraw> {
    let Some(value) = source.get("downscaleMultiplier") else {
        assert!(
            doc.source.region.is_none(),
            "UI {}: Image @{} sprite has no downscaleMultiplier in a region-tagged layout",
            doc.prefab, comp.path_id
        );
        let first = layouts
            .downscale_defaults
            .lock()
            .expect("UI downscale report poisoned")
            .insert(doc.prefab.clone());
        if first {
            warn!(
                "UI {}: its sprites carry no render-data downscale multiplier (exported before the field \
                 was); drawn with 1.0",
                doc.prefab
            );
        }
        return Ok(1.0);
    };
    let multiplier = value.as_f64().filter(|n| n.is_finite() && *n > 0.0).unwrap_or_else(|| {
        panic!("UI {}: Image @{} sprite downscaleMultiplier {value} is not a positive number", doc.prefab, comp.path_id)
    }) as f32;
    if multiplier != 1.0 {
        return Err(ImageDraw::Fallback(
            "its sprite's downscale multiplier is not 1, and the exported crop is mapped for 1 only",
        ));
    }
    Ok(multiplier)
}

/// A serialized float field of a component; a layout without it is refused.
fn serialized_number(fields: &Value, name: &str) -> f32 {
    fields[name].as_f64().unwrap_or_else(|| panic!("UI component {name} is missing or not a number")) as f32
}
/// A serialized Color field of a component (four channels); a layout
/// without it is refused.
fn serialized_rgba(fields: &Value, name: &str) -> Color {
    let channels = fields[name]
        .as_array()
        .filter(|v| v.len() == 4)
        .unwrap_or_else(|| panic!("UI component {name} is missing or not four channels"));
    let channel = |i: usize| {
        channels[i].as_f64().unwrap_or_else(|| panic!("UI component {name} channel {i} is not a number")) as f32
    };
    Color::srgba(channel(0), channel(1), channel(2), channel(3))
}

fn spawn_text(
    commands: &mut Commands,
    parent: Entity,
    art: &BalloonArt,
    text: &str,
    input_box: bool,
    component: &moly_assets::ui_layout::UiComponent,
    alignment: Option<i64>,
    rect: &UiRect,
    rules: &tmp_layout::TextRules,
    depth: f32,
    layer: usize,
    alpha: f32,
    rendered: &mut NodeCache,
    clip: Option<clip_render::Clipped>,
    images: &Assets<Image>,
    meshes: &mut Assets<Mesh>,
    clip_materials: &mut Assets<clip_render::UiClipMaterial>,
    clip_pixel_size: Vec2,
) {
    let fields = &component.fields;
    let layout = tmp_layout::layout(text, component, rect.size, rect.pivot, rules, alignment, input_box)
        .unwrap_or_else(|error| panic!("UI TMP layout failed: {error}"));
    let base_color = serialized_rgba(fields, "m_fontColor");
    let (cell, pen_x, base_top) = art.cell_geometry();
    let mut clipped_glyphs = Vec::new();
    for glyph in &layout.glyphs {
        let (cell_rect, _) = art.glyph_cell(glyph.ch).unwrap_or_else(|| {
            panic!(
                "UI glyph {:?} at source index {} is absent from the atlas",
                glyph.ch, glyph.source_index
            )
        });
        let factor = glyph.font_size * glyph.scale / BAKE_PPEM;
        let offset = glyph.pen
            + Vec2::new(
                (cell * 0.5 - pen_x) * factor * glyph.width_scale,
                -(cell * 0.5 - base_top) * factor,
            );
        // The pen is already in the rect's local space (origin at the pivot).
        let position = rect.world.transform_point3(offset.extend(0.));
        let (scale, rotation, _) = rect.world.to_scale_rotation_translation();
        let color = glyph
            .color
            .map(|c| Color::srgba(c[0], c[1], c[2], c[3]))
            .unwrap_or(base_color);
        let glyph_size = Vec2::new(cell * factor * glyph.width_scale, cell * factor);
        let glyph_transform = Transform {
            translation: Vec3::new(position.x, position.y, depth), rotation, scale,
        };
        if clip.is_some() {
            clipped_glyphs.push(clip_render::Glyph {
                image: art.glyph_image_for(glyph.ch).clone(), rect: cell_rect,
                size: glyph_size, transform: glyph_transform, color,
            });
            continue;
        }
        let entity = commands
            .spawn((
                Sprite {
                    image: art.glyph_image_for(glyph.ch).clone(),
                    rect: Some(cell_rect),
                    color: color.with_alpha(color.alpha() * alpha),
                    custom_size: Some(glyph_size),
                    ..default()
                },
                glyph_transform,
                RenderLayers::layer(layer),
            ))
            .id();
        commands.entity(parent).add_child(entity);
        rendered.sprites.push((entity, color));
    }
    if let Some(clip) = clip {
        clip_render::Draw { commands, images, meshes, materials: clip_materials,
            pixel_size: clip_pixel_size, layer
        }.glyphs(parent, rendered, clipped_glyphs, clip, alpha);
    }
}
