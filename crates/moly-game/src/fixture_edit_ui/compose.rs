//! Source prefab composition and the source custom ListView placement law.

use super::{EditorIcons, ItemChoice};
use crate::{
    fixture_edit::EditView,
    ui_layout::{UiLayouts, UiPrefabView},
};
use bevy::prelude::*;
use moly_assets::ui_layout::{UiComponent, UiInstance, UiPrefab};
use serde_json::Value;

pub(super) const RUNTIME: &str = "EditorRuntime";

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

pub(super) struct Bindings {
    pub list_rect: String,
    pub viewport: String,
    pub content: String,
    pub source_content_position: Vec2,
    pub padding: [f32; 4],
    pub cell_size: Vec2,
    pub spacing: Vec2,
    pub scroll_sensitivity: f32,
    pub cells: Vec<Cell>,
    pub tabs: Vec<Tab>,
    pub hidden: Vec<String>,
    pub save: String,
    pub hide_ui: String,
    pub show_ui: String,
    pub hud: String,
    pub hud_content: String,
    pub delete: String,
    pub cancel: String,
    pub rotate: String,
    pub decide: String,
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
    let list = template(if room { "EditorFloor" } else { "EditorOutdoor" })?;
    let list_fields = component(list, &list.prefab, ".SiteEditContentList")?
        .fields
        .clone();
    let fixture_fields = component(
        list,
        &field(&list_fields, "_fixtureContentListView")?,
        ".SiteEditFixtureContentListView",
    )?
    .fields
    .clone();
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
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
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
    let mut hidden = [
        "ContentRoot/PlanningConfirmHeadUpDisplay",
        "ContentRoot/Rectangle",
        "ContentRoot/LongTapGauge",
        "ContentRoot/PlacedCountHeadUpDisplay",
        "ContentRoot/SequentialFixtureStartMarker",
        "ContentRoot/SequentialFixtureEndMarker",
        "ContentRoot/SiteEditView/RightBottom",
        "ContentRoot/ExpansionFixtureSelecter/BaseContent/Background/SiteEnvironment",
        "ContentRoot/ExpansionFixtureSelecter/BaseContent/Background/Handle/FixtureThumbnail",
        "ContentRoot/ExpansionFixtureSelecter/BaseContent/LayoutedCount",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    // Full source hierarchy is retained, while unavailable data families have
    // no fabricated rows. Source's inactive outer prototype stays inactive.
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
    let mut unsupported_buttons = [
        "_removeAllButton",
        "_presetSaveButton",
        "_infoButton",
        "_changeLookButton",
        "_rotateButton",
        "_reportTipButton",
    ]
    .into_iter()
    .map(|name| field(&action, name))
    .collect::<Result<Vec<_>, _>>()?;
    let filter = component(
        list,
        &field(&fixture_fields, "_searchButton")?,
        ".UIPartsFilterButton",
    )?;
    // SetCallback binds the local CustomButton and an optional serialized
    // extra button. Null in that extra field does not remove the local one.
    if let Ok(local_button) = component(list, &field(&fixture_fields, "_searchButton")?, ".CustomButton") {
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
    let bindings = Bindings {
        list_rect,
        viewport,
        content,
        source_content_position,
        padding,
        cell_size: Vec2::new(f(&cell_fields, "sizeX")?, f(&cell_fields, "sizeY")?),
        spacing: Vec2::new(
            f(&list_view, "horizontalSpacing")?,
            f(&list_view, "verticalSpacing")?,
        ),
        scroll_sensitivity: f(&scroll, "m_ScrollSensitivity")?,
        cells,
        tabs,
        hidden,
        save: field(&action, "_saveButton")?,
        hide_ui: field(&action, "_uiDisableButton")?,
        show_ui: field(&action, "_uiEnableButton")?,
        hud: field(&screen, "_fixtureEditHeadUpDisplay")?,
        hud_content: field(&hud, "_contentSizeFitter")?,
        delete: field(&hud, "deleteButton")?,
        cancel: field(&hud, "cancelButton")?,
        rotate: field(&hud, "rotateButton")?,
        decide: field(&hud, "decideButton")?,
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
    for path in &bindings.unsupported_buttons {
        enabled(view, doc, path, false);
    }
    for tab in &bindings.tabs {
        view.set_texture(
            &tab.image,
            &format!(
                "editor-tab-{}-{}",
                tab.kind,
                if tab.primary { "normal" } else { "disabled" }
            ),
        );
        view.set_visible(&tab.selected, tab.primary);
        for path in &tab.hide {
            view.set_visible(path, false);
        }
        enabled(view, doc, &tab.button, tab.primary);
    }
    for cell in &bindings.cells {
        view.set_texture(&cell.image, &icons.by_fixture[&cell.choice.fixture_id]);
        view.set_text(
            &cell.quantity,
            if cell.choice.unlimited() { "∞" } else { "1" }.to_owned(),
        );
        view.set_visible(&cell.placed, cell.choice.is_placed());
        for path in &cell.hide {
            view.set_visible(path, false);
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
    let rows = (bindings.cells.len() as f32 / columns as f32).ceil();
    let height = step.y * rows + bindings.padding[2] + bindings.padding[3]
        - if rows > 0. { bindings.spacing.y } else { 0. };
    view.set_size_delta(&bindings.content, Vec2::new(0., height));
    if let Some(viewport) = view.rect(layouts, &bindings.viewport, canvas) {
        *scroll = scroll.clamp(0., (height - viewport.size.y).max(0.));
    }
    view.set_anchored_position(
        &bindings.content,
        bindings.source_content_position + Vec2::Y * *scroll,
    );
    for (index, cell) in bindings.cells.iter().enumerate() {
        view.set_anchored_position(
            &cell.root,
            Vec2::new(
                step.x * (index % columns) as f32 + bindings.padding[0],
                -step.y * (index / columns) as f32 - bindings.padding[2],
            ),
        );
    }
}
