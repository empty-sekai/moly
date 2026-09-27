//! The harvest screen's tool views (`ScreenLayerMysekaiHarvest`): the
//! selected-tool view and the tool selector's controller.
//!
//! - `SelectedToolView.UpdateContent(tool)`: the icon through its
//!   `UITextureLoader` (bundle `mysekai/thumbnail/tool`, file
//!   `<spriteName>_t`) and the count text
//!   `GetFormat("FORMAT_MYSEKAI_HARVEST_TOOL_QUANTITY", quantity)` written
//!   through the text setter; `Show` / `Hide` are its GameObject's active
//!   flag; `ShowArrowImage` / `HideArrowImage` set both arrows together.
//! - `UpdateToolView(tool, targetToolCount, fixtureType)` (in
//!   `ShowHarvestUI`): the selector's GameObject off; with a tool, a kind
//!   that needs one and a quantity above zero, `UpdateContent`, `Show`, and
//!   the arrows iff there are at least two target tools; otherwise `Hide`.
//! - `ChangeTool(tool)` (on a target change, and the selector's icon and
//!   cover clicks): the selector hidden; with a tool, `SetSelectTool`, then
//!   with a target the tool model, the selected-tool view active unless the
//!   target is harvested, `UpdateContent`, and
//!   `ToolSelectorViewController.UpdateToolName(master, isFadeHide: true)`.
//! - `OnOpenToolSelector` (the selected-tool button's click): fewer than two
//!   target tools return; otherwise the target tools, distinct by master id,
//!   and the selected one's index go to `ToolSelectorViewController.Show`,
//!   and the selected-tool view hides.
//!
//! The tool name view (`ToolNameView`, ComponentRoot/RightBottom/ToolName)
//! is at alpha 0 outside the selector: `SetUp` runs `HideAnimation()` (both
//! graphics to 0 at once) and `UpdateToolName(.., isHide: true)` runs
//! `ShowAndHideToolNameAnimation` (both fade to 0 over 0.2 s, then after
//! 1 s `HideAnimation()`; read in the game binary), so it is not drawn
//! here; only the selector's `Show` brings it to 1 (text) and 0.4 (frame).
//!
//! Named gaps: the selector is not drawn (its cell prefab,
//! `ToolScrollView._cellPrefab`, and the scroll view's serialized fields are
//! not extracted; release roadmap), so `Show` is refused by name and the
//! selected-tool view stays; the tool thumbnails are not on the roots yet
//! (named once per sprite; the icon and its loading circle stay hidden);
//! `ChangeTool`'s `ToolUseHistory` write to the local cache is not ported;
//! `SetInteractable` (`EnableMysekaiHarvestButton`) is not wired.
//! `SelectedToolView`'s serialized references are not decoded on the roots,
//! so its targets are read by structure: the one text under the view, the
//! two `Arrow` children (set together, so their left/right identity does not
//! matter), and the `ToolIconLoader`'s raw image and loading circle.

use std::collections::HashSet;

use bevy::prelude::*;

use super::action::HarvestPlayerModel;
use super::catalog::HarvestCatalog;
use super::law::is_tool_required_for_harvest;
use super::ui::HarvestButton;
use crate::ui_layout::{UiLayouts, UiPrefabView};
use moly_assets::ui_layout::UiPrefab;

const SELECTED: &str = "ComponentRoot/RightBottom/SelectedTool";
const TOOL_NAME: &str = "ComponentRoot/RightBottom/ToolName";
const SELECTOR: &str = "ComponentRoot/RightBottom/ToolSelector";
const COUNT_FORMAT: &str = "FORMAT_MYSEKAI_HARVEST_TOOL_QUANTITY";
/// `AssetBundleNames.GetMysekaiToolTransparentThumbnail`'s bundle.
const THUMBNAIL_BUNDLE: &str = "mysekai/thumbnail/tool";

/// The subtrees this module's views own inside the harvest screen.
pub(crate) const SUBTREES: [&str; 3] = [SELECTED, TOOL_NAME, SELECTOR];

/// `UpdateContent`'s tool.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SelectedContent {
    pub(crate) tool_id: i64,
    pub(crate) quantity: i32,
    pub(crate) sprite_name: String,
}

/// The tool views' state as the harvest presenter writes it.
#[derive(Default, Debug)]
pub(crate) struct ToolViewState {
    /// `SelectedToolView`'s GameObject active.
    pub(crate) selected_active: bool,
    /// The last `UpdateContent`.
    pub(crate) content: Option<SelectedContent>,
    /// `ShowArrowImage` (both arrows) or `HideArrowImage`.
    pub(crate) arrows: bool,
    /// The last `UpdateToolName`'s tool (the name view stays at alpha 0).
    pub(crate) tool_name: Option<i64>,
    /// Selected-tool button clicks raised this frame.
    pub(crate) open_presses: u32,
    /// Thumbnails already named missing.
    missing_named: HashSet<String>,
}

impl ToolViewState {
    /// `HideHarvestUI`'s part: the selector off, the selected-tool view
    /// hidden.
    pub(crate) fn hide(&mut self) {
        self.selected_active = false;
    }
}

/// The selected tool as `UpdateContent` reads it.
pub(crate) fn content_of(
    model: &HarvestPlayerModel,
    catalog: &HarvestCatalog,
    tool_id: i64,
) -> Option<SelectedContent> {
    let user = model.tools.iter().find(|tool| tool.tool_id == tool_id)?;
    let def = catalog.tool(tool_id)?;
    Some(SelectedContent {
        tool_id,
        quantity: user.quantity,
        sprite_name: def.sprite_name.clone(),
    })
}

/// `ScreenLayerMysekaiHarvest.UpdateToolView(tool, targetToolCount,
/// fixtureType)`.
pub(crate) fn update_tool_view(
    state: &mut ToolViewState,
    tool: Option<SelectedContent>,
    target_tool_count: usize,
    fixture_type: i32,
) {
    match tool.filter(|tool| is_tool_required_for_harvest(fixture_type) && tool.quantity > 0) {
        Some(tool) => {
            state.content = Some(tool);
            state.selected_active = true;
            state.arrows = target_tool_count >= 2;
        }
        None => state.selected_active = false,
    }
}

/// `ChangeTool(tool)`'s view part with a target (`harvested`: the target's
/// status is harvested).
pub(crate) fn change_tool(state: &mut ToolViewState, tool: SelectedContent, harvested: bool) {
    state.selected_active = !harvested;
    state.tool_name = Some(tool.tool_id);
    state.content = Some(tool);
}

/// The view's targets, as selectors.
#[derive(Debug)]
pub(crate) struct ToolViewMarks {
    pub(crate) selected: String,
    count_text: String,
    arrows: Vec<String>,
    icon: String,
    loading: String,
    tool_name: String,
    selector: String,
}

impl ToolViewMarks {
    pub(crate) fn from_document(doc: &UiPrefab) -> Self {
        let at = |path: &str| -> usize { doc.find(path).unwrap_or_else(|error| panic!("{error}")) };
        let id = |index: usize| format!("@{}", doc.nodes[index].game_object_id);
        let selected = at(SELECTED);
        let root = doc.nodes[selected].path.clone();
        let under =
            |node: &moly_assets::ui_layout::UiNode| node.path.starts_with(&format!("{root}/"));
        let texts: Vec<usize> = (0..doc.nodes.len())
            .filter(|&i| {
                under(&doc.nodes[i])
                    && doc.nodes[i]
                        .components
                        .iter()
                        .any(|c| c.class == "Sekai.UI.CustomTextMesh")
            })
            .collect();
        assert_eq!(
            texts.len(),
            1,
            "{}: SelectedToolView carries {} texts, expected the count text alone",
            doc.prefab,
            texts.len()
        );
        let arrows: Vec<String> = (0..doc.nodes.len())
            .filter(|&i| doc.nodes[i].path == format!("{root}/Arrow"))
            .map(id)
            .collect();
        assert_eq!(
            arrows.len(),
            2,
            "{}: SelectedToolView has {} Arrow children, expected 2",
            doc.prefab,
            arrows.len()
        );
        let loader = format!("{root}/IconFame/ToolIconLoader");
        let child = |name: &str| -> String {
            let path = format!("{loader}/{name}");
            let index = doc
                .nodes
                .iter()
                .position(|node| node.path == path)
                .unwrap_or_else(|| panic!("{}: {path} is missing", doc.prefab));
            id(index)
        };
        ToolViewMarks {
            selected: id(selected),
            count_text: id(texts[0]),
            arrows,
            icon: child("ToolIcon"),
            loading: child("UIPartsLoadingCircle"),
            tool_name: id(at(TOOL_NAME)),
            selector: id(at(SELECTOR)),
        }
    }
}

/// Dress the view from the state.
pub(crate) fn apply(
    state: &mut ToolViewState,
    marks: &ToolViewMarks,
    view: &mut UiPrefabView,
    layouts: &UiLayouts,
) {
    view.set_visible(&marks.selected, state.selected_active);
    // The selector is refused (see the module comment); the name view is at
    // alpha 0.
    view.set_visible(&marks.selector, false);
    view.set_visible(&marks.tool_name, false);
    for arrow in &marks.arrows {
        view.set_visible(arrow, state.arrows);
    }
    let Some(content) = state.content.as_ref() else {
        return;
    };
    let format = layouts.wording(COUNT_FORMAT);
    let text =
        moly_law::text::custom_text_mesh::format_wording(&format, &[content.quantity.to_string()])
            .unwrap_or_else(|error| panic!("UI wording {COUNT_FORMAT}: {error}"));
    view.set_text(&marks.count_text, text);
    view.set_visible(&marks.icon, false);
    view.set_visible(&marks.loading, false);
    if state.missing_named.insert(content.sprite_name.clone()) {
        warn!(
            "[harvest-ui] SelectedToolView.UpdateContent: tool {} icon {THUMBNAIL_BUNDLE}/{}_t is not on this root (the tool thumbnails are not exported yet); the icon and its loading circle stay hidden",
            content.tool_id, content.sprite_name
        );
    }
}

/// Update: the selected-tool button's clicks (`OnOpenToolSelector`).
pub(crate) fn open_tool_selector(
    mut button: ResMut<HarvestButton>,
    model: Option<Res<HarvestPlayerModel>>,
    catalog: Option<Res<HarvestCatalog>>,
) {
    let presses = std::mem::take(&mut button.tools.open_presses);
    let (Some(model), Some(catalog)) = (model, catalog) else {
        return;
    };
    for _ in 0..presses {
        let Some(targets) = model.target_tools(&catalog) else {
            error!("[harvest-ui] OnOpenToolSelector: no selected tool, so GetTargetTools is null (the source's Distinct throws here)");
            continue;
        };
        if targets.len() < 2 {
            info!(
                "[harvest-ui] OnOpenToolSelector: {} target tool(s); fewer than 2, nothing opens",
                targets.len()
            );
            continue;
        }
        let mut distinct: Vec<i64> = Vec::new();
        for id in targets {
            if !distinct.contains(&id) {
                distinct.push(id);
            }
        }
        let index = model
            .selected
            .and_then(|selected| distinct.iter().position(|id| *id == selected));
        warn!(
            "[harvest-ui] OnOpenToolSelector: tools {distinct:?}, selected index {index:?}: ToolSelectorViewController.Show is not drawn (the selector's cell prefab and scroll view fields are not extracted); the selected-tool view stays"
        );
    }
}
