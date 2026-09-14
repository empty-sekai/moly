//! Shared prefab view: geometry, images and text use the same resolved nodes as hit testing.

mod filled_image;
mod tmp_layout;
mod clip_render;

pub(crate) fn install(app: &mut App) {
    clip_render::install(app);
}

use crate::balloon::{canvas_scale, BalloonArt, BAKE_PPEM};
use bevy::asset::RenderAssetUsages;
use bevy::asset::{AssetEvent, AssetId, AssetLoadFailedEvent, LoadState};
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::sprite::{BorderRect, SliceScaleMode, TextureSlicer};
use moly_assets::{
    json::JsonAsset,
    ui_layout::{
        auto_layout::{compute_overrides, LayoutMetrics},
        RectTransform, UiComponent, UiPrefab, UiRect,
    },
};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};

const ROOT: &str = "moly://ui-layout-v2/";

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
}

impl PointerUi<'_, '_> {
    pub(crate) fn captures(&self, position: Vec2, window_size: Vec2) -> bool {
        if self.dialogs.blocks_field_input() { return true; }
        let editor_layer = self.layers.current() == crate::ui_layers::LayerId::MysekaiSiteEdit;
        if !self.layers.on_field() && !editor_layer { return true; }
        if editor_layer && self.editor.as_deref().is_some_and(|edit| edit.exit_dialog) {
            return true;
        }
        let Some(layouts) = self.layouts.as_deref() else { return false; };
        let canvas = window_size / canvas_scale(window_size.x, window_size.y);
        let screen = Vec3::new(position.x - window_size.x * 0.5, window_size.y * 0.5 - position.y, 0.);
        self.views.iter().any(|(entity, view, transform)| {
            if root_hidden(entity, &self.hierarchy) { return false; }
            let Some(doc) = layouts.document(view.key) else { return false; };
            let Some(rects) = view.resolved(layouts, canvas) else { return false; };
            let local = transform.affine().inverse().transform_point3(screen).truncate();
            if crate::fixture_edit_ui::captures_background(view, layouts, local, canvas) {
                return true;
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
];

#[derive(Resource, Default)]
pub(crate) struct UiLayouts {
    docs: HashMap<String, UiPrefab>,
    document_revisions: HashMap<String, u64>,
    images: HashMap<String, Handle<Image>>,
    pub(crate) wordings: HashMap<String, String>,
    runtime_textures: HashMap<String, String>,
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
    runtime_sprite_layouts: HashMap<String, SpriteLayoutMetrics>,
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
    docs: Vec<(&'static str, Handle<JsonAsset>)>,
    wordings: Handle<JsonAsset>,
    textures: Handle<JsonAsset>,
    text_settings: Handle<JsonAsset>,
    host_canvas: Handle<JsonAsset>,
}

pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.init_resource::<UiLayouts>();
    commands.insert_resource(UiLayoutRequests {
        docs: DOCUMENTS
            .iter()
            .map(|(name, path)| (*name, server.load(format!("{ROOT}{path}"))))
            .collect(),
        wordings: server.load("moly://wordings.json"),
        textures: server.load(format!("{ROOT}textures.json")),
        text_settings: server.load(format!("{ROOT}text-settings.json")),
        host_canvas: server.load(format!("{ROOT}host-canvas.json")),
    });
}

pub(crate) fn parse(
    mut commands: Commands,
    request: Option<Res<UiLayoutRequests>>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut layouts: ResMut<UiLayouts>,
) {
    let Some(request) = request else {
        return;
    };
    for handle in [
        &request.wordings,
        &request.textures,
        &request.text_settings,
        &request.host_canvas,
    ] {
        if let LoadState::Failed(e) = server.load_state(handle) {
            panic!("UI source asset failed: {e:?}");
        }
        if json.get(handle).is_none() {
            return;
        }
    }
    if !layouts.metadata_loaded {
        let host_canvas: Value = serde_json::from_str(&json.get(&request.host_canvas).unwrap().0)
            .expect("UI host canvas JSON");
        assert_eq!(
            host_canvas["version"].as_u64(),
            Some(1),
            "UI host canvas version"
        );
        let reference_ppu = host_canvas["referencePixelsPerUnit"]
            .as_f64()
            .expect("UI host referencePixelsPerUnit") as f32;
        // This is an explicit Mysekai CanvasRoot contract, not a claimed native
        // inheritance rule for every nested Canvas. Per-component data wins.
        layouts.set_canvas_reference_pixels_per_unit(reference_ppu);
        let text_settings: Value =
            serde_json::from_str(&json.get(&request.text_settings).unwrap().0)
                .expect("UI text settings JSON");
        assert_eq!(
            text_settings["version"].as_u64(),
            Some(1),
            "UI text settings version"
        );
        layouts.text_rules = Some(tmp_layout::TextRules {
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
        });
        layouts.measurement_revision = layouts.measurement_revision.wrapping_add(1);
        let words: Value =
            serde_json::from_str(&json.get(&request.wordings).unwrap().0).expect("wordings JSON");
        layouts.wordings = words["entries"]
            .as_object()
            .expect("wordings entries")
            .iter()
            .filter_map(|(k, v)| v["value"].as_str().map(|v| (k.clone(), v.to_owned())))
            .collect();
        layouts.runtime_textures = serde_json::from_str(&json.get(&request.textures).unwrap().0)
            .expect("UI runtime texture inventory");
        let paths: Vec<_> = layouts.runtime_textures.values().cloned().collect();
        for path in paths {
            layouts
                .images
                .entry(path.clone())
                .or_insert_with(|| server.load(format!("{ROOT}{path}")));
        }
        layouts.metadata_loaded = true;
    }
    let mut pending = false;
    for (name, handle) in &request.docs {
        if layouts.docs.contains_key(*name) {
            continue;
        }
        if let LoadState::Failed(e) = server.load_state(handle) {
            panic!("UI {name} source asset failed: {e:?}");
        }
        let Some(source) = json.get(handle) else {
            pending = true;
            continue;
        };
        let doc = UiPrefab::parse(&source.0).unwrap_or_else(|e| panic!("UI {name}: {e}"));
        info!("UI prefab {name}: {} source nodes", doc.nodes.len());
        layouts.install_document(name, doc, &server);
    }
    if !pending {
        // Text glyphs are baked from all document wordings once. Publish the
        // source set atomically, even though each JSON was parsed only once.
        layouts.sources_ready = true;
        commands.remove_resource::<UiLayoutRequests>();
    }
}

impl UiLayouts {
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
            .or_insert_with(|| server.load(full_path.clone())).clone();
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
                .or_insert_with(|| server.load(format!("{ROOT}{path}"))).id();
            self.image_documents.entry(id).or_default().push(key.to_owned());
        }
        self.document_images.insert(key.to_owned(), paths);
        self.ready_documents.lock().expect("UI readiness cache poisoned").remove(key);
        self.resolved.lock().expect("UI layout cache poisoned").remove(key);
        let revision = self.document_revisions.entry(key.to_owned()).or_default();
        *revision = revision.wrapping_add(1);
        self.docs.insert(key.to_owned(), document);
    }

    /// CustomSelectableDefine's cue binding, using the clicked source control.
    pub(crate) fn button_sound(&self, key: &str, path: &str) -> Option<String> {
        let doc = self.document(key)?;
        let mut node = &doc.nodes[doc.find(path).expect("source button path")];
        if let Some(wrapper) = node.components.iter().find(|c| c.fields.get("_button").is_some()) {
            let pointer = wrapper.fields["_button"].as_array().expect("button reference");
            assert_eq!(pointer[0].as_i64(), Some(0), "button reference scope");
            let id = pointer[1].as_i64().expect("button reference identity");
            node = &doc.nodes[doc.find(&format!("@{id}")).expect("wrapped button")];
        }
        let control = node.components.iter().find(|c| c.enabled && c.fields.get("se").is_some())?;
        if control.fields["m_Interactable"].as_bool() == Some(false) { return None; }
        let cue = match control.fields["se"].as_i64().expect("button sound kind") {
            0 => return None,
            1 => "SE_DECIDE1",
            2 => "SE_CANCEL",
            3 => control.fields["otherSeName"].as_str().expect("custom button sound"),
            4 => "se_mysekai_ui_decision",
            5 => "se_mysekai_ui_select",
            6 => "se_mysekai_ui_cancel",
            other => panic!("unsupported button sound kind {other}"),
        };
        (!cue.is_empty()).then(|| cue.to_owned())
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
        let mut measurement_error = None;
        let automatic = compute_overrides(
            doc,
            canvas,
            visibility,
            rect_overrides,
            |index, axis, size| {
                if measurement_error.is_some() {
                    return None;
                }
                match self.measure_node(doc, index, axis, size, changes.get(&index).copied()) {
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
        let automatic =
            automatic.unwrap_or_else(|error| panic!("UI layout {}: {error}", doc.prefab));
        let mut rects = doc.resolve_with(canvas, visibility, &automatic);
        moly_assets::ui_layout::clipping::apply(doc, &mut rects)
            .unwrap_or_else(|error| panic!("UI clipping {}: {error}", doc.prefab));
        Arc::new(rects)
    }

    fn measure_node(
        &self,
        doc: &UiPrefab,
        index: usize,
        axis: usize,
        size: Vec2,
        change: Option<&Override>,
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
                let text = change
                    .and_then(|v| v.text.as_deref())
                    .map(str::to_owned)
                    .unwrap_or_else(|| self.text(&comp.fields));
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
    pub(crate) fn text(&self, fields: &Value) -> String {
        let key = fields["wordingKey"].as_str().unwrap_or("");
        // CustomTextMesh leaves its serialized text intact when the key is empty.
        // Numeric fields use this state until their presenter supplies a value.
        if fields["useWordingKey"].as_bool() == Some(true) && !key.is_empty() {
            self.wordings
                .get(key)
                .unwrap_or_else(|| panic!("UI wording missing: {key}"))
                .clone()
        } else {
            fields["m_text"].as_str().unwrap_or("").to_owned()
        }
    }
    pub(crate) fn text_chars(&self) -> Vec<char> {
        let mut chars = Vec::new();
        for doc in self.docs.values() {
            for n in &doc.nodes {
                for c in &n.components {
                    if c.fields.get("m_fontSize").is_some() {
                        chars.extend(self.text(&c.fields).chars());
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
}

impl Override {
    fn same_paint(&self, other: Option<&Self>) -> bool {
        match other {
            Some(other) => {
                self.text == other.text
                    && self.text_alignment == other.text_alignment
                    && self.fill == other.fill
                    && self.texture == other.texture
            }
            None => self.text.is_none()
                && self.text_alignment.is_none()
                && self.fill.is_none()
                && self.texture.is_none(),
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
    pub(crate) fn rect(&self, layouts: &UiLayouts, path: &str, canvas: Vec2) -> Option<UiRect> {
        let doc = layouts.document(self.key)?;
        let index = doc.find(path).unwrap_or_else(|e| panic!("{e}"));
        self.resolved(layouts, canvas).map(|r| r[index].clone())
    }
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
                let text = overrides
                    .get(&index)
                    .and_then(|v| v.text.clone())
                    .unwrap_or_else(|| layouts.text(&comp.fields));
                let pages: HashSet<_> = text
                    .chars()
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
    let (Some(art), Ok(window)) = (art, windows.single()) else {
        return;
    };
    let canvas =
        Vec2::new(window.width(), window.height()) / canvas_scale(window.width(), window.height());
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
            if rendered.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.rect == *rect && snapshot.change.same_paint(change)
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
                        });
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
                });
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
                if comp.class.ends_with("Image") && f.get("m_Color").is_some() {
                    let clip = clip_render::for_component(comp, rect);
                    let source = comp.sprite.as_ref().or(comp.texture.as_ref());
                    let path = image_path(&layouts, comp, change);
                    let image = path.map(|p| layouts.images[p].clone()).unwrap_or_default();
                    let mut sprite = Sprite {
                        image,
                        color: rgba(&f["m_Color"], Color::WHITE),
                        custom_size: Some(rect.size),
                        ..default()
                    };
                    if f["m_Type"].as_i64() == Some(1) {
                        if let Some(border) = source
                            .and_then(|v| v["border"].as_array())
                            .filter(|v| v.len() == 4)
                        {
                            let borders = [
                                num(&border[0]),
                                num(&border[1]),
                                num(&border[2]),
                                num(&border[3]),
                            ];
                            if let Some(size) = source
                                .and_then(|v| v["size"].as_array())
                                .filter(|v| v.len() == 2)
                            {
                                let image_size = Vec2::new(num(&size[0]), num(&size[1]));
                                // A zero-width UV centre is valid: it stretches one sampled line.
                                // TextureSlicer rejects this case and stretches the whole image.
                                if borders[0] + borders[2] >= image_size.x
                                    || borders[1] + borders[3] >= image_size.y
                                {
                                    let mesh = sliced_mesh(
                                        rect.size,
                                        image_size,
                                        borders,
                                        f["m_FillCenter"].as_bool().unwrap_or(true),
                                    );
                                    if let Some(clip) = clip {
                                        clip_render::Draw { commands: &mut commands, images: &images,
                                            meshes: &mut meshes, materials: &mut clip_materials,
                                            pixel_size: clip_pixel_size, layer: view.layer
                                        }.mesh(node_contents, rendered, mesh, base_transform,
                                            sprite.color, Some(sprite.image.clone()), clip, alphas[index]);
                                        continue;
                                    }
                                    let material = materials.add(ColorMaterial {
                                        color: sprite.color,
                                        texture: Some(sprite.image.clone()),
                                        ..default()
                                    });
                                    let mesh = meshes.add(mesh);
                                    rendered.meshes.push(mesh.clone());
                                    rendered.materials.push(material.clone());
                                    rendered.material_colors.push(sprite.color);
                                    if let Some(value) = materials.get_mut(&material) {
                                        value.color = sprite
                                            .color
                                            .with_alpha(sprite.color.alpha() * alphas[index]);
                                    }
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
                            }
                            sprite.image_mode = SpriteImageMode::Sliced(TextureSlicer {
                                border: BorderRect {
                                    min_inset: Vec2::new(num(&border[0]), num(&border[3])),
                                    max_inset: Vec2::new(num(&border[2]), num(&border[1])),
                                },
                                center_scale_mode: SliceScaleMode::Stretch,
                                sides_scale_mode: SliceScaleMode::Stretch,
                                max_corner_scale: 1.,
                            });
                        }
                    }
                    if f["m_Type"].as_i64() == Some(3) {
                        let fill = change
                            .and_then(|c| c.fill)
                            .unwrap_or_else(|| num(&f["m_FillAmount"]));
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
                    let text = change
                        .and_then(|v| v.text.clone())
                        .unwrap_or_else(|| layouts.text(f));
                    spawn_text(
                        &mut commands,
                        node_contents,
                        &art,
                        &text,
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
                        clip_render::for_component(comp, rect),
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

fn sliced_mesh(size: Vec2, image_size: Vec2, border: [f32; 4], fill_center: bool) -> Mesh {
    let mut adjusted = border;
    for axis in 0..2 {
        let combined = adjusted[axis] + adjusted[axis + 2];
        if combined > size[axis] {
            let ratio = size[axis] / combined;
            adjusted[axis] *= ratio;
            adjusted[axis + 2] *= ratio;
        }
    }
    let x = [0., adjusted[0], size.x - adjusted[2], size.x];
    let y = [0., adjusted[1], size.y - adjusted[3], size.y];
    let u = [
        0.,
        border[0] / image_size.x,
        1. - border[2] / image_size.x,
        1.,
    ];
    let v = [
        1.,
        1. - border[1] / image_size.y,
        border[3] / image_size.y,
        0.,
    ];
    let mut positions = Vec::with_capacity(16);
    let mut uv = Vec::with_capacity(16);
    let mut indices = Vec::with_capacity(54);
    for row in 0..4 {
        for col in 0..4 {
            positions.push([x[col] - size.x * 0.5, y[row] - size.y * 0.5, 0.]);
            uv.push([u[col], v[row]]);
        }
    }
    for row in 0..3 {
        for col in 0..3 {
            if (!fill_center && col == 1 && row == 1)
                || x[col + 1] <= x[col]
                || y[row + 1] <= y[row]
            {
                continue;
            }
            let lower_left = (row * 4 + col) as u32;
            indices.extend_from_slice(&[
                lower_left,
                lower_left + 1,
                lower_left + 5,
                lower_left,
                lower_left + 5,
                lower_left + 4,
            ]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0., 0., 1.]; 16])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
    .with_inserted_indices(Indices::U32(indices))
}

fn num(value: &Value) -> f32 {
    value.as_f64().unwrap_or(0.) as f32
}
fn rgba(value: &Value, default: Color) -> Color {
    value
        .as_array()
        .filter(|v| v.len() == 4)
        .map(|v| Color::srgba(num(&v[0]), num(&v[1]), num(&v[2]), num(&v[3])))
        .unwrap_or(default)
}

fn spawn_text(
    commands: &mut Commands,
    parent: Entity,
    art: &BalloonArt,
    text: &str,
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
    let layout = tmp_layout::layout(text, component, rect.size, rules, alignment)
        .unwrap_or_else(|error| panic!("UI TMP layout failed: {error}"));
    let base_color = rgba(
        &fields["m_fontColor"],
        Color::srgba(0.26666668, 0.26666668, 0.4, 1.),
    );
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
        let position = rect
            .world
            .transform_point3((offset + (Vec2::splat(0.5) - rect.pivot) * rect.size).extend(0.));
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
