//! Source prefab composition and the source custom ListView placement law.

use super::{EditorIcons, ItemChoice};
use crate::fixture_edit::EditContentType;
use crate::{
    ui_inputs::{EDITOR_RUNTIME_KEY, EditView},
    ui_layout::{UiLayouts, UiPrefabView},
};
use bevy::prelude::*;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiPrefab};
use serde_json::Value;

pub(super) const RUNTIME: &str = EDITOR_RUNTIME_KEY;

#[derive(Clone)]
pub(super) struct Cell {
    pub choice: ItemChoice,
    pub root: String,
    pub button: String,
    pub image: String,
    pub quantity: String,
    pub selected: String,
    pub placed: String,
    pub hide: Vec<String>,
    /// The thumbnail's `frameMask`, which `SetupMysekaiFixture` disables.
    pub frame_mask: Option<i64>,
}

#[derive(Clone)]
pub(super) struct Tab {
    pub kind: i64,
    pub primary: bool,
    pub button: String,
    pub image: String,
    pub selected: String,
    pub hide: Vec<String>,
}

pub(super) struct Scrollbar {
    pub root: String,
    pub handle: String,
    pub container: String,
    pub direction: moly_law::ui::scrollbar::Direction,
    pub auto_hide: bool,
}

#[derive(Clone)]
pub(super) struct EnvironmentCell {
    pub phenomena_id: Option<i32>,
    pub root: String,
    pub button: String,
    pub image: String,
    pub selected: String,
    pub hide: Vec<String>,
}

pub(super) struct Bindings {
    pub environment_cells: Vec<EnvironmentCell>,
    pub environment_content: String,
    pub environment_text: String,
    pub selected_tab: i64,
    pub environment_ready: bool,
    pub list_rect: String,
    pub viewport: String,
    pub content: String,
    pub source_content_position: Vec2,
    pub padding: [f32; 4],
    pub cell_size: Vec2,
    pub spacing: Vec2,
    pub scroll_sensitivity: f32,
    pub scrollbar: Option<Scrollbar>,
    pub cells: Vec<Cell>,
    pub tabs: Vec<Tab>,
    pub hidden: Vec<String>,
    pub save: String,
    pub info: String,
    pub hide_ui: String,
    pub show_ui: String,
    pub hud: String,
    pub hud_content: String,
    pub delete: String,
    pub cancel: String,
    pub rotate: String,
    pub decide: String,
    /// SiteEditView's camera rotate button (`LayoutAction` 13).
    pub camera_rotate: String,
    /// SiteEditView's change-look button (`LayoutAction` 14).
    pub change_look: String,
    /// SiteEditView's remove-all button (`OnRemoveFixtureAll`).
    pub remove_all: String,
    /// The screen's `_groundEditTargetIcon` (an `EditTargetIcon`) and its
    /// `_fixtureThumbnail`.
    pub target_icon: String,
    pub target_thumbnail: String,
    pub target_image: String,
    pub target_quantity: String,
    pub target_hide: Vec<String>,
    pub target_frame_mask: Option<i64>,
    pub road_start_marker: String,
    pub road_end_marker: String,
    pub road_rectangle: String,
    pub road_count: String,
    pub road_count_text: String,
    pub road_switch: String,
    pub road_switch_on: String,
    pub road_switch_off: String,
    pub right_bottom: String,
    pub fixture_visibility_toggle: String,
    pub fixture_visibility_on: String,
    pub fixture_visibility_off: String,
    pub unsupported_buttons: Vec<String>,
    pub panel: String,
    pub panel_y: f32,
    pub panel_height_delta: f32,
    pub panel_width: f32,
    pub panel_max_width: f32,
    pub panel_hide: f32,
    pub panel_handle: String,
    pub screen_offset_scale: f32,
}

impl Bindings {
    pub(super) fn item_roots(&self) -> impl Iterator<Item = &str> {
        self.cells
            .iter()
            .map(|cell| cell.root.as_str())
            .chain(self.environment_cells.iter().map(|cell| cell.root.as_str()))
    }
}

pub(super) fn component<'a>(
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

pub(super) fn pointer(value: &Value) -> Result<i64, String> {
    let p = value
        .as_array()
        .filter(|p| p.len() == 2)
        .ok_or("expected decoded source PPtr")?;
    if p[0].as_i64() != Some(0) {
        return Err("source control PPtr is external".into());
    }
    p[1].as_i64()
        .ok_or_else(|| "invalid source control identity".into())
}

pub(super) fn field(fields: &Value, name: &str) -> Result<String, String> {
    let id = pointer(&fields[name])?;
    if id == 0 {
        return Err(format!("required source control {name} is null"));
    }
    Ok(format!("@{id}"))
}

fn cloned_field(instance: &UiInstance, fields: &Value, name: &str) -> Result<String, String> {
    instance.selector(pointer(&fields[name])?)
}

fn optional_cloned(
    instance: &UiInstance,
    fields: &Value,
    names: &[&str],
) -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    for name in names {
        let id = pointer(&fields[*name])?;
        if id != 0 {
            result.push(instance.selector(id)?);
        }
    }
    Ok(result)
}

/// Whether this layout's `SiteEditView` declares `_reportTipButton`, by the
/// layout's region tag. Layouts without a tag are the shared root's CN
/// extractions, whose class serializes it; the JP class of the region root
/// serializes nine references and the report-tip button is not one of them,
/// so on JP it is not built. A declared field the layout lacks is still an
/// error.
fn declares_report_tip_button(doc: &UiPrefab) -> Result<bool, String> {
    match doc.source.region.as_deref() {
        None => Ok(true),
        Some("jp") => Ok(false),
        Some(region) => Err(format!(
            "SiteEditView: no declared field set for region {region}"
        )),
    }
}

fn f(fields: &Value, name: &str) -> Result<f32, String> {
    fields[name]
        .as_f64()
        .filter(|v| v.is_finite())
        .map(|v| v as f32)
        .ok_or_else(|| format!("source UI float missing: {name}"))
}

pub(super) fn compose(
    layouts: &UiLayouts,
    edit: &EditView,
    choices: Vec<ItemChoice>,
) -> Result<(UiPrefab, Bindings), String> {
    let template = |key| {
        layouts
            .document(key)
            .ok_or_else(|| format!("source UI template not loaded: {key}"))
    };
    let mut doc = template("EditorSource")?.clone();
    let screen = component(&doc, &doc.prefab, ".ScreenLayerSiteEditMode")?
        .fields
        .clone();
    let action = component(&doc, &field(&screen, "_siteEditView")?, ".SiteEditView")?
        .fields
        .clone();
    let hud = component(
        &doc,
        &field(&screen, "_fixtureEditHeadUpDisplay")?,
        ".FixtureEditHeadUpDisplay",
    )?
    .fields
    .clone();
    let selector = component(
        &doc,
        &field(&screen, "_contentListSelector")?,
        ".ExpansionContentListSelector",
    )?
    .fields
    .clone();
    let room = matches!(edit.site_id, 2..=4);
    let environment = edit.content == EditContentType::Environment;
    let road = edit.content == EditContentType::Road;
    let list = template(if road {
        "EditorRoad"
    } else if environment {
        "EditorEnvironment"
    } else if room {
        "EditorFloor"
    } else {
        "EditorOutdoor"
    })?;
    let list_fields = component(
        list,
        &list.prefab,
        if road {
            ".FixtureContentList"
        } else if environment {
            ".SiteEnvironmentContentList"
        } else {
            ".SiteEditContentList"
        },
    )?
    .fields
    .clone();
    let fixture_fields = if environment || road {
        let mut fields = list_fields.clone();
        fields["_listView"] = fields["listView"].clone();
        fields
    } else {
        component(
            list,
            &field(&list_fields, "_fixtureContentListView")?,
            ".SiteEditFixtureContentListView",
        )?
        .fields
        .clone()
    };
    let list_view = component(list, &field(&fixture_fields, "_listView")?, ".ListView")?
        .fields
        .clone();
    let scroll = component(list, &field(&list_view, "scrollRect")?, ".CustomScrollRect")?
        .fields
        .clone();
    let list_instance = doc.instantiate_subtree(
        &field(&selector, "_contentListBaseRoot")?,
        list,
        &list.prefab,
        "FixtureList",
    )?;
    let list_rect = cloned_field(&list_instance, &fixture_fields, "_listView")?;
    let viewport = cloned_field(&list_instance, &scroll, "m_Viewport")?;
    let content = cloned_field(&list_instance, &scroll, "m_Content")?;
    let scrollbar = match pointer(&scroll["m_VerticalScrollbar"])? {
        0 => None,
        id => {
            let source = component(list, &format!("@{id}"), "Scrollbar")?;
            let handle_id = pointer(&source.fields["m_HandleRect"])?;
            let handle_node = list.find(&format!("@{handle_id}"))?;
            let container = list
                .parent(handle_node)
                .ok_or("source scrollbar handle lacks parent")?;
            let direction = source.fields["m_Direction"]
                .as_i64()
                .and_then(moly_law::ui::scrollbar::Direction::from_source)
                .ok_or("source scrollbar direction")?;
            let visibility = scroll["m_VerticalScrollbarVisibility"]
                .as_i64()
                .ok_or("source scrollbar visibility")?;
            // AutoHideAndExpandViewport also drives the viewport width; this
            // editor's source uses Permanent or AutoHide only.
            if !(0..=1).contains(&visibility) || direction.axis() != 1 {
                return Err(
                    "source editor requires a vertical Permanent/AutoHide scrollbar".into(),
                );
            }
            Some(Scrollbar {
                root: list_instance.selector(id)?,
                handle: list_instance.selector(handle_id)?,
                container: list_instance.selector(list.nodes[container].transform_id)?,
                direction,
                auto_hide: visibility == 1,
            })
        }
    };
    let source_content_position = Vec2::from_array(
        list.nodes[list.find(&field(&scroll, "m_Content")?)?]
            .rect
            .anchored_position,
    );
    let cells_source = template("EditorCell")?;
    let cell_fields = component(cells_source, &cells_source.prefab, ".FixtureSelectCell")?
        .fields
        .clone();
    let thumb_fields = component(
        cells_source,
        &field(&cell_fields, "thumbnail")?,
        ".UIPartsFixtureThumbnail",
    )?
    .fields
    .clone();
    let item_names: Vec<_> = (0..choices.len()).map(|i| format!("Item{i}")).collect();
    let instances = doc.instantiate_children(
        &content,
        cells_source,
        &cells_source.prefab,
        item_names.iter().map(String::as_str),
    )?;
    let cells = instances
        .iter()
        .zip(choices)
        .map(|(instance, choice)| {
            let mut hide = optional_cloned(instance, &cell_fields, &["_missionLabel"])?;
            hide.extend(optional_cloned(
                instance,
                &thumb_fields,
                &["disableCover", "labelImage", "_subThumbnailImage"],
            )?);
            // UIPartsItemThumbnail.SetupMysekaiFixture hides thumbnailBase and
            // ends with VisibleFrame = false: the frame image's GameObject off
            // and frameMask disabled, so a fixture thumbnail is not masked.
            hide.extend(optional_cloned(
                instance,
                &thumb_fields,
                &["thumbnailBase", "frameImage"],
            )?);
            let frame_mask = match pointer(&thumb_fields["frameMask"])? {
                0 => None,
                id => Some(instance.identity(id)?),
            };
            // Loading/failed-state graphics belong to the source texture loader;
            // the host hides them once the real catalog image is registered.
            let loader = component(
                cells_source,
                &field(&thumb_fields, "textureLoader")?,
                ".UITextureLoader",
            )?;
            hide.extend(optional_cloned(
                instance,
                &loader.fields,
                &["loadingObject", "notFoundObject"],
            )?);
            Ok(Cell {
                choice,
                root: instance.root_selector(),
                button: cloned_field(instance, &thumb_fields, "button")?,
                image: cloned_field(instance, &thumb_fields, "thumbnailImage")?,
                quantity: cloned_field(instance, &thumb_fields, "innerText")?,
                selected: cloned_field(instance, &cell_fields, "selected")?,
                placed: cloned_field(instance, &cell_fields, "_inPlacedLabel")?,
                hide,
                frame_mask,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let environment_cells = if environment {
        compose_environment_cells(
            &mut doc,
            template("EditorEnvironmentCell")?,
            &content,
            &edit.environment_choices,
        )?
    } else {
        Vec::new()
    };
    let cell_size = if environment {
        let source = template("EditorEnvironmentCell")?;
        let fields = &component(source, &source.prefab, ".SiteEnvironmentSelectCell")?.fields;
        Vec2::new(f(fields, "sizeX")?, f(fields, "sizeY")?)
    } else {
        Vec2::new(f(&cell_fields, "sizeX")?, f(&cell_fields, "sizeY")?)
    };
    let tab_source = template("EditorTab")?;
    let tab_fields = component(tab_source, &tab_source.prefab, ".ContentListSelectorCell")?
        .fields
        .clone();
    let kinds: &[i64] = if room { &[3, 4] } else { &[0, 1, 19] };
    let names: Vec<_> = kinds.iter().map(|id| format!("Tab{id}")).collect();
    let tab_instances = doc.instantiate_children(
        &field(&selector, "_tabCellRoot")?,
        tab_source,
        &tab_source.prefab,
        names.iter().map(String::as_str),
    )?;
    let tabs = tab_instances
        .iter()
        .zip(kinds)
        .enumerate()
        .map(|(index, (instance, kind))| {
            Ok(Tab {
                kind: *kind,
                primary: index == 0,
                button: cloned_field(instance, &tab_fields, "button")?,
                image: cloned_field(instance, &tab_fields, "_iconImage")?,
                selected: cloned_field(instance, &tab_fields, "selectedObj")?,
                hide: optional_cloned(
                    instance,
                    &tab_fields,
                    &[
                        "badgeObj",
                        "_missionIcon",
                        "selectBannerObj",
                        "selectBannerCover",
                        "lineObj",
                    ],
                )?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    // The screen's own two fixture thumbnails carry frame masks whose graphic
    // has alpha 0, so an enabled one writes no stencil and hides its
    // thumbnail. The source shows neither in the lists this host opens:
    // - the selector handle's FixtureThumbnail has no serialized reference, no
    //   code of the selector or the screen touches it, and it is inactive in
    //   the prefab, so it stays hidden (listed below);
    // - the ground target icon is shown only by SetupGroundEditTargetIcon when
    //   the open list is Road or Fence; it then activates the thumbnail and
    //   runs UIPartsFixtureThumbnail.Setup, i.e. SetupMysekaiFixture (base and
    //   frame hidden, frame mask disabled) like a list cell. For every other
    //   list, and on exit, EditTargetIcon.Hide turns off the icon and its
    //   thumbnail; see apply_static.
    let target_icon_fields = component(
        &doc,
        &field(&screen, "_groundEditTargetIcon")?,
        ".EditTargetIcon",
    )?
    .fields
    .clone();
    let target_icon = field(&screen, "_groundEditTargetIcon")?;
    let target_thumbnail = field(&target_icon_fields, "_fixtureThumbnail")?;
    let target_fields = component(&doc, &target_thumbnail, ".UIPartsFixtureThumbnail")?
        .fields
        .clone();
    let target_image = field(&target_fields, "thumbnailImage")?;
    let target_quantity = field(&target_fields, "innerText")?;
    let mut target_hide = Vec::new();
    for key in [
        "disableCover",
        "labelImage",
        "_subThumbnailImage",
        "thumbnailBase",
        "frameImage",
    ] {
        let id = pointer(&target_fields[key])?;
        if id != 0 {
            target_hide.push(format!("@{id}"));
        }
    }
    let loader = component(
        &doc,
        &field(&target_fields, "textureLoader")?,
        ".UITextureLoader",
    )?;
    for key in ["loadingObject", "notFoundObject"] {
        let id = pointer(&loader.fields[key])?;
        if id != 0 {
            target_hide.push(format!("@{id}"));
        }
    }
    let target_frame_mask = match pointer(&target_fields["frameMask"])? {
        0 => None,
        id => Some(id),
    };

    // Elements without a presenter here. The screen reaches these through its
    // serialized references; the two paths below have no serialized
    // reference in the layout (no other component points at them).
    let mut hidden = [
        "_rectangle",
        "_longTouchGauge",
        "_placedCountHeadUpDisplay",
        "_sequentialFixtureStartMarker",
        "_sequentialFixtureEndMarker",
        "_rightBottomRoot",
        "_fixtureLayoutCountInfo",
    ]
    .into_iter()
    .map(|name| field(&screen, name))
    .collect::<Result<Vec<_>, _>>()?;
    hidden.extend(
        [
            "ContentRoot/PlanningConfirmHeadUpDisplay",
            "ContentRoot/ExpansionFixtureSelecter/BaseContent/Background/Handle/FixtureThumbnail",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    // Full source hierarchy is retained, while unavailable data families have
    // no fabricated rows. Source's inactive outer prototype stays inactive.
    if !environment && !road {
        hidden.extend(optional_cloned(
            &list_instance,
            &list_fields,
            &[
                "listView",
                "_collectionContentList",
                "_penlightContentList",
                "_honorContentList",
                "_recordContentListView",
                "_photoContentListView",
                "_tabListController",
            ],
        )?);
        hidden.push(cloned_field(
            &list_instance,
            &fixture_fields,
            "_nothingText",
        )?);
        hidden.push(cloned_field(
            &list_instance,
            &fixture_fields,
            "_hashTagFilteredBalloon",
        )?);
    } else {
        hidden.extend(optional_cloned(
            &list_instance,
            &list_fields,
            &["_tabListController", "_sortDropdown", "_nothingText"],
        )?);
    }
    let mut unsupported = vec!["_presetSaveButton"];
    if declares_report_tip_button(&doc)? {
        unsupported.push("_reportTipButton");
    } else if action.get("_reportTipButton").is_some() {
        return Err(
            "this region's SiteEditView declares no _reportTipButton, but the layout carries one"
                .into(),
        );
    }
    let mut unsupported_buttons = unsupported
        .into_iter()
        .map(|name| field(&action, name))
        .collect::<Result<Vec<_>, _>>()?;
    if !environment {
        let filter = component(
            list,
            &field(&fixture_fields, "_searchButton")?,
            ".UIPartsFilterButton",
        )?;
        // SetCallback binds the local CustomButton and an optional serialized
        // extra button. Null in that extra field does not remove the local one.
        if let Ok(local_button) = component(
            list,
            &field(&fixture_fields, "_searchButton")?,
            ".CustomButton",
        ) {
            unsupported_buttons.push(list_instance.selector(local_button.path_id)?);
        }
        let extra_button = pointer(&filter.fields["_button"])?;
        if extra_button != 0 {
            unsupported_buttons.push(list_instance.selector(extra_button)?);
        }
        // Sorting/filter menus require their actual data model; do not leave an
        // enabled-looking dropdown which does nothing or show source placeholders.
        hidden.push(cloned_field(
            &list_instance,
            &fixture_fields,
            "_sortDropdown",
        )?);
    }
    let panel = field(&selector, "baseContentRectTransform")?;
    let panel_rect = doc.nodes[doc.find(&panel)?].rect.clone();
    let padding_values = list_view["padding"]
        .as_array()
        .filter(|p| p.len() == 4)
        .ok_or("ListView padding")?;
    let mut padding = [0.; 4];
    for (out, v) in padding.iter_mut().zip(padding_values) {
        *out = v.as_i64().ok_or("ListView padding integer")? as f32;
    }
    let road_count = field(&screen, "_placedCountHeadUpDisplay")?;
    let counter = component(&doc, &road_count, ".PlacedCountHeadUpDisplay")?;
    let road_count_text = field(&counter.fields, "_text")?;
    let road_switch = field(&screen, "_switchEditModeToggle")?;
    let switch = component(&doc, &road_switch, ".UIPartsToggle")?;
    let road_switch_on = field(&switch.fields, "onGroup")?;
    let road_switch_off = field(&switch.fields, "offGroup")?;
    let fixture_visibility_toggle = field(&screen, "_showFixtureToggle")?;
    let visibility = component(&doc, &fixture_visibility_toggle, ".UIPartsToggle")?;
    let fixture_visibility_on = field(&visibility.fields, "onGroup")?;
    let fixture_visibility_off = field(&visibility.fields, "offGroup")?;
    let bindings = Bindings {
        environment_cells,
        environment_content: field(&screen, "_siteEnvironmentSelectContent")?,
        environment_text: field(&screen, "_siteEnvironmentText")?,
        selected_tab: edit.content.tab_kind(),
        environment_ready: layouts.document("EditorEnvironmentCell").is_some(),
        list_rect,
        viewport,
        content,
        source_content_position,
        padding,
        cell_size,
        spacing: Vec2::new(
            f(&list_view, "horizontalSpacing")?,
            f(&list_view, "verticalSpacing")?,
        ),
        scroll_sensitivity: f(&scroll, "m_ScrollSensitivity")?,
        scrollbar,
        cells,
        tabs,
        hidden,
        save: field(&action, "_saveButton")?,
        info: field(&action, "_infoButton")?,
        hide_ui: field(&action, "_uiDisableButton")?,
        show_ui: field(&action, "_uiEnableButton")?,
        hud: field(&screen, "_fixtureEditHeadUpDisplay")?,
        hud_content: field(&hud, "_contentSizeFitter")?,
        delete: field(&hud, "deleteButton")?,
        cancel: field(&hud, "cancelButton")?,
        rotate: field(&hud, "rotateButton")?,
        decide: field(&hud, "decideButton")?,
        camera_rotate: field(&action, "_rotateButton")?,
        change_look: field(&action, "_changeLookButton")?,
        remove_all: field(&action, "_removeAllButton")?,
        target_icon,
        target_thumbnail,
        target_image,
        target_quantity,
        target_hide,
        target_frame_mask,
        road_start_marker: field(&screen, "_sequentialFixtureStartMarker")?,
        road_end_marker: field(&screen, "_sequentialFixtureEndMarker")?,
        road_rectangle: field(&screen, "_rectangle")?,
        road_count,
        road_count_text,
        road_switch,
        road_switch_on,
        road_switch_off,
        right_bottom: field(&screen, "_rightBottomRoot")?,
        fixture_visibility_toggle,
        fixture_visibility_on,
        fixture_visibility_off,
        unsupported_buttons,
        panel,
        panel_y: panel_rect.anchored_position[1],
        panel_height_delta: panel_rect.size_delta[1],
        panel_width: f(&selector, "viewBaseWidth")?,
        panel_hide: f(&selector, "viewBaseHide")?,
        panel_max_width: f(&selector, "viewMaxWidth")?,
        panel_handle: field(&selector, "eventTrigger")?,
        screen_offset_scale: f(&hud, "_screenOffsetScale")?,
    };
    Ok((doc, bindings))
}

pub(super) fn enabled(view: &mut UiPrefabView, doc: &UiPrefab, selector: &str, enabled: bool) {
    let Ok(button) = component(doc, selector, ".CustomButton") else {
        return;
    };
    if let Ok(id) = pointer(&button.fields["coverImage"]) {
        if id != 0 {
            view.set_visible(&format!("@{id}"), !enabled);
        }
    }
    if let Some(covers) = button.fields["optionalCoverImages"].as_array() {
        for cover in covers {
            if let Ok(id) = pointer(cover) {
                if id != 0 {
                    view.set_visible(&format!("@{id}"), !enabled);
                }
            }
        }
    }
}

pub(super) fn apply_static(
    view: &mut UiPrefabView,
    doc: &UiPrefab,
    bindings: &Bindings,
    icons: &EditorIcons,
) {
    for path in &bindings.hidden {
        view.set_visible(path, false);
    }
    // SetupGroundEditTargetIcon: ShouldShowGroundEditTargetIcon is true only
    // for the Road (1) and Fence (2) lists; otherwise EditTargetIcon.Hide.
    // This host opens its primary tab, OutDoor (0) or Floor (3), so the icon
    // and its thumbnail are hidden.
    view.set_visible(&bindings.target_icon, false);
    view.set_visible(&bindings.target_thumbnail, false);
    for path in &bindings.target_hide {
        view.set_visible(path, false);
    }
    if let Some(mask) = bindings.target_frame_mask {
        view.set_behaviour_enabled(mask, false);
    }
    for path in &bindings.unsupported_buttons {
        enabled(view, doc, path, false);
    }
    for tab in &bindings.tabs {
        let available = tab.primary
            || tab.kind == 1
            || (tab.kind == 19 && bindings.environment_ready && icons.environment(None).is_some());
        view.set_texture(
            &tab.image,
            &format!(
                "editor-tab-{}-{}",
                tab.kind,
                if available { "normal" } else { "disabled" }
            ),
        );
        view.set_visible(&tab.selected, tab.kind == bindings.selected_tab);
        for path in &tab.hide {
            view.set_visible(path, false);
        }
        enabled(view, doc, &tab.button, available);
    }
    for cell in &bindings.environment_cells {
        let icon = icons.environment(cell.phenomena_id);
        if let Some(icon) = icon {
            view.set_texture(&cell.image, icon);
        }
        view.set_visible(&cell.image, icon.is_some());
        for path in &cell.hide {
            view.set_visible(path, false);
        }
        enabled(view, doc, &cell.button, icon.is_some());
    }
    for cell in &bindings.cells {
        let icon = cell
            .choice
            .icon(icons)
            .expect("the editor lists only fixtures with a registered thumbnail");
        view.set_texture(&cell.image, icon);
        // UIPartsThumbnail.SetQuantity: "×{0}".
        view.set_text(&cell.quantity, format!("×{}", cell.choice.count));
        view.set_visible(&cell.placed, cell.choice.is_placed());
        for path in &cell.hide {
            view.set_visible(path, false);
        }
        if let Some(mask) = cell.frame_mask {
            view.set_behaviour_enabled(mask, false);
        }
        enabled(view, doc, &cell.button, cell.choice.editable);
    }
}

pub(super) fn layout_cells(
    bindings: &Bindings,
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
    canvas: Vec2,
    scroll: &mut f32,
) {
    let Some(list) = view.rect(layouts, &bindings.list_rect, canvas) else {
        return;
    };
    let mut used = bindings.cell_size.x + (bindings.padding[0] + bindings.padding[1]);
    let mut columns = 0usize;
    while used <= list.size.x {
        columns += 1;
        used = bindings.cell_size.x + (bindings.spacing.x + used);
    }
    let columns = columns.max(1);
    let step = bindings.cell_size + bindings.spacing;
    let rows = (bindings.item_roots().count() as f32 / columns as f32).ceil();
    let height = step.y * rows + bindings.padding[2] + bindings.padding[3]
        - if rows > 0. { bindings.spacing.y } else { 0. };
    view.set_size_delta(&bindings.content, Vec2::new(0., height));
    if let Some(viewport) = view.rect(layouts, &bindings.viewport, canvas) {
        let range = (height - viewport.size.y).max(0.);
        *scroll = scroll.clamp(0., range);
        if let Some(bar) = &bindings.scrollbar {
            view.set_visible(
                &bar.root,
                !bar.auto_hide
                    || moly_law::ui::scrollbar::scrolling_needed(height, viewport.size.y),
            );
            view.set_scrollbar(
                &bar.root,
                if range > 0. { 1. - *scroll / range } else { 1. },
                moly_law::ui::scrollbar::thumb_size(height, viewport.size.y, 0.),
            );
        }
    }
    view.set_anchored_position(
        &bindings.content,
        bindings.source_content_position + Vec2::Y * *scroll,
    );
    for (index, root) in bindings.item_roots().enumerate() {
        view.set_anchored_position(
            root,
            Vec2::new(
                step.x * (index % columns) as f32 + bindings.padding[0],
                -step.y * (index / columns) as f32 - bindings.padding[2],
            ),
        );
    }
}

/// A drag that started on the actual source thumb retains its initial grab
/// point while moving outside the track. Content dragging remains separate.
pub(super) fn drag_scrollbar(
    bindings: &Bindings,
    view: &UiPrefabView,
    layouts: &UiLayouts,
    canvas: Vec2,
    start: Vec2,
    point: Vec2,
    start_scroll: f32,
) -> Option<f32> {
    let bar = bindings.scrollbar.as_ref()?;
    let container = view.rect(layouts, &bar.container, canvas)?;
    let content = view.rect(layouts, &bindings.content, canvas)?;
    let viewport = view.rect(layouts, &bindings.viewport, canvas)?;
    if !container.active || container.world.determinant().abs() < f32::EPSILON {
        return None;
    }
    let range = (content.size.y - viewport.size.y).max(0.);
    if range <= 0. {
        return None;
    }
    let inverse = container.world.inverse();
    let from = inverse.transform_point3(start.extend(0.));
    let to = inverse.transform_point3(point.extend(0.));
    let axis = bar.direction.axis();
    let size = moly_law::ui::scrollbar::thumb_size(content.size.y, viewport.size.y, 0.);
    let remaining = container.size[axis] * (1. - size);
    let value = (1. - start_scroll / range).clamp(0., 1.);
    let corner = (if bar.direction.reversed() {
        1. - value
    } else {
        value
    }) * remaining
        + to[axis]
        - from[axis];
    moly_law::ui::scrollbar::drag_value(bar.direction, corner, remaining)
        .map(|value| (1. - value) * range)
}

fn compose_environment_cells(
    doc: &mut UiPrefab,
    source: &UiPrefab,
    content: &str,
    choices: &[crate::fixture_edit::EditEnvironmentView],
) -> Result<Vec<EnvironmentCell>, String> {
    let cell = &component(source, &source.prefab, ".SiteEnvironmentSelectCell")?.fields;
    let thumbnail = &component(
        source,
        &field(cell, "_thumbnail")?,
        ".UIPartsSiteEnvironmentThumbnail",
    )?
    .fields;
    let loader = &component(
        source,
        &field(thumbnail, "textureLoader")?,
        ".UITextureLoader",
    )?
    .fields;
    let names: Vec<_> = (0..choices.len())
        .map(|i| format!("Environment{i}"))
        .collect();
    let instances = doc.instantiate_children(
        content,
        source,
        &source.prefab,
        names.iter().map(String::as_str),
    )?;
    instances
        .iter()
        .zip(choices)
        .map(|(instance, choice)| {
            let mut hide = optional_cloned(
                instance,
                thumbnail,
                &["disableCover", "innerText", "labelImage"],
            )?;
            hide.extend(optional_cloned(
                instance,
                loader,
                &["loadingObject", "notFoundObject"],
            )?);
            Ok(EnvironmentCell {
                phenomena_id: choice.phenomena_id,
                root: instance.root_selector(),
                button: cloned_field(instance, thumbnail, "button")?,
                image: cloned_field(instance, thumbnail, "thumbnailImage")?,
                selected: cloned_field(instance, cell, "_selected")?,
                hide,
            })
        })
        .collect()
}
