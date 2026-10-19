//! The convert screen's item select page (`MysekaiConvertFurnitureItemSelectView`,
//! `MysekaiConvertFurnitureItemSelectState`) and its list.
//!
//! The source's structure:
//! - `ItemSelectState.Setup(slotId)` runs `ItemSelectView.Setup(slotId,
//!   onConvertStart)`: the tab cells come from
//!   `CreateItemSelectTabListCellDataList` (one per `mysekaiConvertItemTabs`
//!   row, in `seq` order, the first selected), the models from
//!   `CreateMysekaiConvertItemModels`. `OnEnter` then runs `Refresh()` and
//!   `Show()`; a tab list with no row hides the page.
//! - `Refresh()`: `GetValidConvertItemModels` (displayable and of the
//!   selected tab, in `CompareTo` order), the first selected,
//!   `RefreshListView(..., resetContentPosition: true)` and the detail view
//!   refreshed.
//! - `OnSelectTab(vd)`: the tab id, then `Refresh()`.
//! - `OnSelectedConvertItem(item)`: the item becomes the selected one, the
//!   list is refreshed with `resetContentPosition: false` (the scroll stays)
//!   and the detail view refreshed.
//! - A list cell (`MysekaiConvertFurnitureItemSelectListCell`): its thumbnail
//!   (`SetupObtainedItemThumbnail`), `SetupInfo`'s one-of-three badge (locked
//!   with its balloon, sold out, not enough consumed resource),
//!   `RefreshFrame`'s selected frame, and `SetupSelectedButton`'s press: a
//!   released or not-yet-selected cell selects the item, and a long press
//!   opens `MysekaiConvertItemDetailDialog`.
//!
//! The source's tab cell (`MysekaiConvertFixtureItemSelectTab`) and list cell
//! (`MysekaiConvertFurnitureItemSelectListCell`) prefabs are not part of the
//! public asset root: the screen's scroller names the list cell by an external
//! pointer, and no document is named after either class. This page
//! instantiates both from documents the root does carry: the tab cell from the
//! shared left tab list cell, the list cell from the shared item thumbnail
//! cell whose `UIPartsItemThumbnail` is the one the source's cell also holds,
//! and each cell's three badges from the detail panel's own badge subtrees
//! (the nodes the source's detail view uses), so every drawn image is the
//! document's.
//!
//! Named: the tab master's `name` and `assetbundleName` columns (the convert
//! master reader keeps only each tab's id and `seq`), so a tab draws its
//! selected and unselected backs without a label or icon; the list's inertia,
//! its bar handle and the cells' tap effects.

pub mod list;

use std::collections::BTreeSet;

use bevy::prelude::*;
use moly_assets::ui_layout::UiPrefab;

use super::detail::{DetailNodes, DetailPanel, Masters};
use super::model::{ItemContext, ItemModel};
use super::screen::note_once;
use super::slot::put_text;
use crate::ui_inputs::BalloonArt;
use crate::ui_layout::{UiLayouts, UiPrefabView};
use list::{Grid, Scroll};

/// The tab cell document (a shared left tab list cell).
pub const TAB_CELL: &str = "EditorCategory";
/// The list cell document (the shared item thumbnail cell).
pub const ITEM_CELL: &str = "InventoryItemCell";
/// The empty list's line (`MSG_CONVERT_ITEM_NONE`).
pub const NONE_KEY: &str = "MSG_CONVERT_ITEM_NONE";
/// The locked badge's balloon (`WORD_CONVERT_FIXTURE_LEVEL_RELEASE`).
pub const LOCKED_KEY: &str = "WORD_CONVERT_FIXTURE_LEVEL_RELEASE";
/// The sold-out badge's line (`WORD_MYSEKAI_CONVERT_OUT_OF_STOCK_DETAIL`).
pub const SOLD_OUT_KEY: &str = "WORD_MYSEKAI_CONVERT_OUT_OF_STOCK_DETAIL";

/// The page's class.
pub const PAGE_CLASS: &str = "MysekaiConvertFurnitureItemSelectView";
const LIST: &str = "ItemSelect/BaseContent/ListView";
const LIST_VIEWPORT: &str = "ItemSelect/BaseContent/ListView/Viewport";
const LIST_CONTENT: &str = "ItemSelect/BaseContent/ListView/Viewport/Content";
const NONE_INFO: &str = "ItemSelect/BaseContent/ListView/NoneInfo";
const TAB_CONTENT: &str =
    "ItemSelect/BaseContent/UIPartsLeftTabList/Content/Root/Viewport/Content";
/// The detail panel's badge subtrees this page copies into its cells.
const BADGES: &str = "ItemDetail/ItemDetail/Slot/Root/GaugeView";
/// The cell pool: more cells than the list's viewport shows at once.
const POOL: usize = 24;
/// The tab cell's step down the list: its 128 height and the scroller's
/// `layoutInfo.itemSpacing` of 16.
const TAB_STEP: f32 = 144.0;
/// `UIPartsLeftTabListCellBase`'s long press threshold.
const LONG_PRESS_SECONDS: f64 = 0.25;

fn sub(root: &str, below: &str) -> String {
    format!("{root}/{below}")
}

/// One tab cell's nodes.
#[derive(Debug, Clone, Default)]
pub struct TabNodes {
    pub root: String,
    pub icon: String,
    pub name: String,
    pub selected: String,
    pub unselected: String,
    /// The parts of the shared cell this page does not drive (the arrow, the
    /// lock cover and the mission target icon).
    pub hidden: Vec<String>,
}

/// One list cell's nodes.
#[derive(Debug, Clone, Default)]
pub struct CellNodes {
    pub root: String,
    pub image: String,
    pub frame: String,
    pub locked: String,
    pub balloon: String,
    pub balloon_text: String,
    pub sold_out: String,
    pub sold_out_text: String,
    pub not_enough: String,
    /// The thumbnail's parts this page does not drive (the count, the label,
    /// the zoom icon, the disabled cover and the tap effect).
    pub hidden: Vec<String>,
}

/// The page's nodes, the list's grid, its tab cells, its cell pool and the
/// detail panel's nodes.
#[derive(Debug, Clone, Default)]
pub struct PageBindings {
    pub view: String,
    /// The slot page in the same document: it is drawn by the layer's own
    /// view and stays hidden here.
    pub slot_page: String,
    pub viewport: String,
    pub content: String,
    pub none_info: String,
    pub detail: DetailNodes,
    pub grid: Option<Grid>,
    pub source_content: Vec2,
    /// The tab list's content, whose height the tab count sets.
    pub tab_content: String,
    pub tabs: Vec<TabNodes>,
    pub cells: Vec<CellNodes>,
}

/// Clones the screen document, adds the page's tab cells, its cell pool and
/// the detail panel's consumption rows, and answers it with the page's
/// bindings.
pub fn compose(layouts: &UiLayouts, tab_count: usize) -> Result<(UiPrefab, PageBindings), String> {
    let screen = layouts
        .document(super::screen::KEY)
        .ok_or("the screen document is not loaded")?;
    let mut doc = screen.clone();
    let view = doc
        .nodes
        .iter()
        .find(|node| {
            node.components
                .iter()
                .any(|component| component.class.ends_with(&format!(".{PAGE_CLASS}")))
        })
        .map(|node| node.path.clone())
        .ok_or_else(|| format!("no {PAGE_CLASS} in the screen"))?;
    let tab_content = sub(&view, TAB_CONTENT);
    let list = sub(&view, LIST);
    let viewport = sub(&view, LIST_VIEWPORT);
    let content = sub(&view, LIST_CONTENT);
    let none_info = sub(&view, NONE_INFO);
    for path in [&tab_content, &list, &viewport, &content, &none_info] {
        doc.find(path).map_err(|error| error.to_string())?;
    }
    let source_content = list::content_position(&doc, &content)?;

    // The tab cells.
    let tab_source = layouts
        .document(TAB_CELL)
        .ok_or_else(|| format!("the root lacks the tab cell document {TAB_CELL}"))?;
    let tab_names: Vec<String> = (0..tab_count).map(|index| format!("Tab{index}")).collect();
    let tab_instances = doc.instantiate_children(
        &tab_content,
        tab_source,
        &tab_source.prefab,
        tab_names.iter().map(String::as_str),
    )?;
    let mut tabs = Vec::new();
    for instance in &tab_instances {
        let root = instance.root_selector();
        tabs.push(TabNodes {
            hidden: ["Arrow", "Locked", "UIPartsMysekaiMissionTargetIcon"]
                .into_iter()
                .map(|name| sub(&root, name))
                .collect(),
            icon: sub(&root, "Icon"),
            name: sub(&root, "NameText"),
            selected: sub(&root, "SelectedBack"),
            unselected: sub(&root, "DeselectedBack"),
            root,
        });
    }

    // The list's cell pool and each cell's three badges.
    let cell_source = layouts
        .document(ITEM_CELL)
        .ok_or_else(|| format!("the root lacks the list cell document {ITEM_CELL}"))?;
    let cell_size = list::template_size(cell_source, &cell_source.prefab)?;
    let grid = Grid::read(&doc, &list, cell_size)?;
    let cell_names: Vec<String> = (0..POOL).map(|index| format!("Cell{index}")).collect();
    let instances = doc.instantiate_children(
        &content,
        cell_source,
        &cell_source.prefab,
        cell_names.iter().map(String::as_str),
    )?;
    let badge_source = doc.clone();
    let badge_root = sub(&view, BADGES);
    let mut cells = Vec::new();
    for instance in &instances {
        let root = instance.root_selector();
        let locked = doc
            .instantiate_subtree(
                &root,
                &badge_source,
                &sub(&badge_root, "LockedObj"),
                "LockedObj",
            )?
            .root_selector();
        let sold_out = doc
            .instantiate_subtree(
                &root,
                &badge_source,
                &sub(&badge_root, "SoldOutObj"),
                "SoldOutObj",
            )?
            .root_selector();
        let not_enough = doc
            .instantiate_subtree(
                &root,
                &badge_source,
                &sub(&badge_root, "NotEnoughConsumedResourceObj"),
                "NotEnoughConsumedResourceObj",
            )?
            .root_selector();
        let balloon = sub(&locked, "Balloon");
        cells.push(CellNodes {
            hidden: [
                "UIPartsItemThumbnail/Num",
                "UIPartsItemThumbnail/LabelImage",
                "UIPartsItemThumbnail/ZoomIcon",
                "UIPartsItemThumbnail/Disabled",
                "UIPartsItemThumbnail/CoverEffect",
            ]
            .into_iter()
            .map(|name| sub(&root, name))
            .collect(),
            image: sub(&root, "UIPartsItemThumbnail/mask/Icon"),
            frame: sub(&root, "Selected"),
            balloon_text: sub(&balloon, "Content/CustomText"),
            balloon,
            sold_out_text: sub(&sold_out, "Info/SettingText"),
            not_enough,
            sold_out,
            locked,
            root,
        });
    }
    let detail = super::detail::compose(&mut doc, layouts, &view)?;
    // The same composed document holds the slot page; the page hides it while
    // it shows, so its path is required like the nodes above.
    let slot_page = super::screen::class_path(&doc, "MysekaiConvertFurnitureSlotView")
        .ok_or_else(|| format!("no MysekaiConvertFurnitureSlotView in the composed screen"))?;
    Ok((
        doc,
        PageBindings {
            detail,
            grid: Some(grid),
            source_content,
            tabs,
            cells,
            tab_content,
            view,
            slot_page,
            viewport,
            content,
            none_info,
        },
    ))
}

/// `UIPartsMysekaiConvertItem.Setup(model)`'s thumbnail: a normal item's
/// single obtain resource, or the surplus item a blueprint or record yields.
pub fn icon_load(
    ctx: ItemContext<'_>,
    model: &ItemModel<'_>,
    masters: &Masters<'_>,
) -> Option<(String, String)> {
    match model {
        ItemModel::Blueprint { .. } => Some((
            "mysekai/thumbnail/item/item_surplus_blueprint".to_owned(),
            "item_surplus_blueprint".to_owned(),
        )),
        ItemModel::Record { .. } => Some((
            "mysekai/thumbnail/item/item_surplus_music_record".to_owned(),
            "item_surplus_music_record".to_owned(),
        )),
        ItemModel::Normal { .. } => {
            let details = ctx.machine().obtain(model.master());
            let [detail] = details.as_slice() else {
                return None;
            };
            masters.icon(&detail.resource_type, detail.resource_id)
        }
    }
}

/// `MysekaiConvertItemModelBase.GetItemName`: a normal item's single obtain
/// resource name (or, with several details, its obtain box's `name`), or a
/// blueprint's or record's wording. `None` where the master row it needs
/// lacks the text; `None` keeps the document's serialized text.
pub fn item_name(
    layouts: &UiLayouts,
    ctx: ItemContext<'_>,
    model: &ItemModel<'_>,
    masters: &Masters<'_>,
) -> Option<String> {
    match model {
        ItemModel::Blueprint { obtain, .. } => obtain.name_key().map(|key| layouts.wording(key)),
        ItemModel::Record { obtain, .. } => Some(layouts.wording(obtain.name_key())),
        ItemModel::Normal { .. } => match ctx.machine().obtain(model.master()).as_slice() {
            [detail] => masters.name(&detail.resource_type, detail.resource_id),
            _ => obtain_box(ctx, model).and_then(|box_| box_.name.clone()),
        },
    }
}

/// The `mysekai_convert_obtain` box a normal item draws
/// (`MasterDataManager.GetMasterResourceBox("mysekai_convert_obtain",
/// externalId)`).
pub fn obtain_box<'a>(ctx: ItemContext<'a>, model: &ItemModel<'a>) -> Option<&'a moly_masters::convert::ResourceBox> {
    if !matches!(model, ItemModel::Normal { .. }) {
        return None;
    }
    ctx.masters.box_of(moly_masters::convert::BOX_OBTAIN, model.master().external_id)
}

/// Every thumbnail the list and the detail panel can show, by its client
/// load path.
pub fn icon_loads(ctx: ItemContext<'_>, masters: &Masters<'_>) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for model in ItemModel::all(ctx) {
        if let Some((path, _)) = icon_load(ctx, &model, masters) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    // The consumption rows' resources (`SetupConsumedResource`).
    for item in ItemModel::all(ctx) {
        for detail in ctx.machine().consumption(item.master()) {
            if let Some((path, _)) = masters.icon(&detail.resource_type, detail.resource_id) {
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// A press on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Tab(usize),
    /// A pool slot (index into the cell pool).
    Cell(usize),
}

/// What a press on the page asks the screen for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemAction {
    /// `OnSelectTab(tab)` then `Refresh()`.
    SelectTab(i32),
    /// `OnSelectedConvertItem(item)`.
    SelectItem(i32),
    /// `ShowDetailDialog` (a later unit).
    Detail(i32),
}

#[derive(Debug, Clone, Copy)]
struct Press {
    /// The pool slot that was pressed.
    slot: usize,
    at: f64,
    long_fired: bool,
}

/// The page's state.
#[derive(Debug, Default)]
pub struct ItemSelectPage {
    /// `_convertSlotId`.
    slot_id: i32,
    /// `_selectedTabId`.
    selected_tab: i32,
    /// The tabs, in `seq` order.
    tabs: Vec<i32>,
    /// `GetValidConvertItemModels`' ids, in order.
    models: Vec<i32>,
    /// `_selectedConvertItemModel`.
    selected: Option<i32>,
    /// The detail panel's model and count.
    pub detail: DetailPanel,
    scroll: Scroll,
    /// The list's grid, once its page is drawn.
    grid: Option<Grid>,
    /// The list's viewport height, once its page is drawn.
    viewport_height: f32,
    press: Option<Press>,
    /// The list's locked items, as the last frame drew them
    /// (`SetupSelectedButton`'s `IsReleased`).
    locked: BTreeSet<i32>,
    /// The item whose lock balloon a press asked for (`ShowLockedBalloon`);
    /// it shows until the list changes.
    balloon_item: Option<i32>,
    shown: bool,
    seen: BTreeSet<String>,
}

impl ItemSelectPage {
    /// `ItemSelectState.Setup(slotId)`: the tab list and the slot; `OnEnter`
    /// then refreshes.
    pub fn setup(&mut self, slot_id: i32, tabs: &[(i32, i32)]) {
        self.slot_id = slot_id;
        self.selected_tab = tabs.first().map_or(0, |(id, _)| *id);
        self.tabs = tabs.iter().map(|(id, _)| *id).collect();
        self.selected = None;
        self.detail.clear();
        self.scroll = Scroll::default();
        self.press = None;
        self.locked.clear();
        self.balloon_item = None;
        self.shown = !self.tabs.is_empty();
        info!(
            "[convert] ItemSelectState.OnEnter: ItemSelectView.Setup(slot {}), {} tab(s), first tab {}; Refresh(), Show()",
            self.slot_id,
            self.tabs.len(),
            self.selected_tab
        );
        note_once(
            &mut self.seen,
            "MysekaiConvertFixtureItemSelectTab.RefreshName and RefreshIcon draw the tab master's name and assetbundleName, which the convert master reader does not keep: the tab cells draw their backs only".to_owned(),
        );
    }

    /// `_convertSlotId`: the slot the page converts in.
    pub fn slot_id(&self) -> i32 {
        self.slot_id
    }

    /// `_selectedConvertItemModel`: the item the start button hands over.
    pub fn item(&self) -> Option<i32> {
        self.selected
    }

    /// `ItemSelectState.OnExit`: `ItemSelectView.Hide()`.
    pub fn hide(&mut self) {
        self.shown = false;
        self.press = None;
        self.scroll = Scroll::default();
        self.locked.clear();
        self.balloon_item = None;
        self.detail.clear();
    }

    /// `Refresh()`: the tab's items, the first selected, the list back at its
    /// top and the detail panel on the first item.
    pub fn refresh(&mut self, ctx: ItemContext<'_>) {
        self.models = ItemModel::of_tab(ctx, self.selected_tab)
            .iter()
            .map(ItemModel::id)
            .collect();
        self.selected = self.models.first().copied();
        self.scroll.refresh(true);
        self.balloon_item = None;
        match self.selected {
            Some(item) => self.detail.setup(item, self.slot_id),
            None => self.detail.clear(),
        }
        info!(
            "[convert] ItemSelectView.Refresh(): tab {}, {} item(s), selected {:?}; RefreshListView(resetContentPosition true), RefreshItemDetailView()",
            self.selected_tab,
            self.models.len(),
            self.selected
        );
    }

    /// `OnSelectedConvertItem(item)`: the list is refreshed without moving.
    pub fn select_item(&mut self, item: i32) -> Option<ItemAction> {
        if !self.models.contains(&item) {
            return None;
        }
        self.selected = Some(item);
        self.scroll.refresh(false);
        self.balloon_item = None;
        self.detail.setup(item, self.slot_id);
        info!(
            "[convert] ItemSelectView.OnSelectedConvertItem({item}): RefreshListView(GetValidConvertItemModels(), resetContentPosition false), RefreshItemDetailView()"
        );
        Some(ItemAction::SelectItem(item))
    }

    /// The page's control at a node path, if any.
    pub fn hit(&self, bindings: &PageBindings, path: &str) -> Option<Hit> {
        let inside = |root: &str| path == root || path.starts_with(&format!("{root}/"));
        if let Some(index) = bindings.tabs.iter().position(|tab| inside(&tab.root)) {
            return Some(Hit::Tab(index));
        }
        bindings
            .cells
            .iter()
            .position(|cell| inside(&cell.root))
            .map(Hit::Cell)
    }

    /// A press: a pool cell starts a press, the viewport starts a scroll.
    pub fn pressed(&mut self, pointer: i32, point: Vec2, hit: Option<Hit>, in_viewport: bool, now: f64) {
        self.scroll.pressed(pointer, point, in_viewport);
        self.press = match hit {
            Some(Hit::Cell(slot)) => Some(Press {
                slot,
                at: now,
                long_fired: false,
            }),
            _ => None,
        };
    }

    /// `OnDrag`: a press that moves past the threshold scrolls the list; a
    /// press that leaves its cell is dropped.
    pub fn moved(&mut self, pointer: i32, point: Vec2, hit: Option<Hit>) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        if self.scroll.moved(pointer, point, &grid) {
            self.press = None;
            return true;
        }
        if let Some(press) = self.press {
            if hit != Some(Hit::Cell(press.slot)) {
                self.press = None;
            }
        }
        false
    }

    /// One frame of a held press: past the threshold it asks for the detail
    /// dialog (`ShowDetailDialog`).
    pub fn step(&mut self, now: f64) -> Option<ItemAction> {
        let press = self.press.as_mut()?;
        if press.long_fired || now - press.at < LONG_PRESS_SECONDS {
            return None;
        }
        press.long_fired = true;
        let data = self
            .grid
            .map(|grid| grid.visible(self.scroll.offset, self.viewport_height.max(1.0), self.models.len()))
            .and_then(|visible| visible.get(press.slot).copied())?;
        Some(ItemAction::Detail(self.models.get(data).copied()?))
    }

    /// A release: a tap that did not scroll clicks the tab or the cell.
    pub fn released(&mut self, pointer: i32, on: Option<Hit>) -> Option<ItemAction> {
        self.scroll.released(pointer);
        let press = self.press.take();
        if press.is_some_and(|press| press.long_fired) {
            return None;
        }
        match on? {
            Hit::Tab(index) => {
                let tab = *self.tabs.get(index)?;
                if tab == self.selected_tab {
                    return None;
                }
                self.selected_tab = tab;
                info!("[convert] ItemSelectView.OnSelectTab({tab}): Refresh()");
                Some(ItemAction::SelectTab(tab))
            }
            Hit::Cell(slot) => {
                let data = self.visible().get(slot).copied()?;
                let item = *self.models.get(data)?;
                if self.locked.contains(&item) && self.selected == Some(item) {
                    // `SetupSelectedButton`: a locked, already selected cell
                    // opens its balloon instead of selecting again.
                    self.balloon_item = Some(item);
                    info!("[convert] ItemSelectListCell.ShowLockedBalloon({item})");
                    None
                } else {
                    self.select_item(item)
                }
            }
        }
    }

    /// The data indices the viewport shows, pool order.
    fn visible(&self) -> Vec<usize> {
        match self.grid {
            Some(grid) => grid.visible(self.scroll.offset, self.viewport_height.max(1.0), self.models.len()),
            None => Vec::new(),
        }
    }

    /// The wheel over the list.
    pub fn wheel(&mut self, inside: bool, lines: f32) {
        if let Some(grid) = self.grid {
            self.scroll.wheel(inside, lines, &grid);
        }
    }

    /// One frame of the page: the tab cells' backs, the pool's cells, the
    /// selected frame, the list's scroll and the detail panel.
    pub fn draw(
        &mut self,
        view: &mut UiPrefabView,
        layouts: &UiLayouts,
        art: Option<&BalloonArt>,
        ctx: ItemContext<'_>,
        masters: &Masters<'_>,
        bindings: &PageBindings,
        canvas: Vec2,
    ) {
        view.set_visible(&bindings.view, self.shown);
        view.set_visible(&bindings.slot_page, false);
        if !self.shown {
            return;
        }
        for (index, tab) in bindings.tabs.iter().enumerate() {
            let selected = self.tabs.iter().position(|id| *id == self.selected_tab) == Some(index);
            view.set_visible(&tab.selected, selected);
            view.set_visible(&tab.unselected, !selected);
            for path in &tab.hidden {
                view.set_visible(path, false);
            }
            view.set_visible(&tab.icon, false);
            view.set_visible(&tab.name, false);
            // The scroller's own `layoutInfo` spacing (16) between 128-high
            // cells; the tab list does not scroll with five tabs.
            view.set_anchored_position(&tab.root, Vec2::new(0.0, -(index as f32) * TAB_STEP));
        }
        view.set_size_delta(
            &bindings.tab_content,
            Vec2::new(0.0, self.tabs.len() as f32 * TAB_STEP),
        );

        // The list: the viewport's height, the content's height and the pool.
        let height = view
            .rect(layouts, &bindings.viewport, canvas)
            .map_or(0.0, |rect| rect.size.y);
        if height > 0.0 {
            self.viewport_height = height;
        }
        let grid = bindings.grid.unwrap_or(Grid {
            cell: Vec2::new(152.0, 152.0),
            spacing: Vec2::new(28.0, 28.0),
            columns: 3,
            padding: [16.0, 0.0, 16.0, 0.0],
            sensitivity: 1.0,
            drag_threshold: list::DRAG_THRESHOLD,
        });
        let count = self.models.len();
        self.grid = bindings.grid;
        self.scroll
            .settle(grid.max_offset(count, self.viewport_height));
        view.set_size_delta(&bindings.content, Vec2::new(0.0, grid.content_height(count)));
        view.set_anchored_position(
            &bindings.content,
            bindings.source_content + Vec2::Y * self.scroll.offset,
        );
        let visible = grid.visible(self.scroll.offset, self.viewport_height.max(1.0), count);
        let all = ItemModel::all(ctx);
        let level = ctx.machine().level(ctx.user);
        self.locked = all
            .list()
            .iter()
            .filter(|model| !model.is_released(level))
            .map(ItemModel::id)
            .collect();
        for (slot, cell) in bindings.cells.iter().enumerate() {
            let Some(&data) = visible.get(slot) else {
                view.set_visible(&cell.root, false);
                continue;
            };
            let Some(item) = self
                .models
                .get(data)
                .and_then(|id| all.iter().find(|model| model.id() == *id))
            else {
                view.set_visible(&cell.root, false);
                continue;
            };
            view.set_visible(&cell.root, true);
            view.set_anchored_position(&cell.root, grid.slot(data));
            for path in &cell.hidden {
                view.set_visible(path, false);
            }
            let released = item.is_released(level);
            let convertible = item.has_convertible_count(ctx);
            let enough = item.affordable_converts(ctx) >= 1;
            view.set_visible(&cell.locked, !released);
            view.set_visible(&cell.sold_out, released && !convertible);
            view.set_visible(&cell.not_enough, released && convertible && !enough);
            if released && !convertible {
                let line = layouts.wording(SOLD_OUT_KEY);
                let shows = put_text(view, &cell.sold_out_text, Some(&line), art, &mut self.seen);
                view.set_visible(&cell.sold_out_text, shows);
            }
            // `RefreshFrame`: `VisibleFrame = IsSelected`.
            view.set_visible(&cell.frame, self.selected == Some(item.id()));
            // `SetupObtainedItemThumbnail`.
            match icon_load(ctx, item, masters) {
                Some((path, _)) if layouts.has_runtime_texture(&path) => {
                    view.set_visible(&cell.image, true);
                    view.set_texture(&cell.image, &path);
                }
                Some((path, _)) => {
                    view.set_visible(&cell.image, false);
                    note_once(
                        &mut self.seen,
                        format!("the cell thumbnail {path} is not on this root: the cell draws no image"),
                    );
                }
                None => view.set_visible(&cell.image, false),
            }
            // `SetupInfo`'s locked badge and its balloon: `ShowLockedBalloon`
            // shows the cell's own balloon, so one shows while the selected
            // locked cell asked for it.
            let balloon = !released && self.balloon_item == Some(item.id()) && art.is_some();
            view.set_visible(&cell.balloon, balloon);
            if balloon {
                let line = layouts.wording(LOCKED_KEY);
                let shows = put_text(view, &cell.balloon_text, Some(&line), art, &mut self.seen);
                view.set_visible(&cell.balloon_text, shows);
            }
        }
        let empty = count == 0;
        view.set_visible(&bindings.none_info, empty);
        if empty {
            let line = layouts.wording(NONE_KEY);
            let shows = put_text(view, &bindings.none_info, Some(&line), art, &mut self.seen);
            view.set_visible(&bindings.none_info, shows);
        }
        let name = self.selected.and_then(|id| {
            all.iter()
                .find(|model| model.id() == id)
                .and_then(|model| item_name(layouts, ctx, model, masters))
        });
        self.detail.draw(
            view,
            layouts,
            art,
            &bindings.detail,
            ctx,
            masters,
            name,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_switch_takes_the_new_tab_and_a_selection_keeps_the_scroll() {
        let mut page = ItemSelectPage {
            tabs: vec![1, 2],
            selected_tab: 1,
            models: vec![11, 12],
            selected: Some(11),
            grid: Some(Grid {
                cell: Vec2::new(152.0, 152.0),
                spacing: Vec2::new(28.0, 28.0),
                columns: 3,
                padding: [16.0, 0.0, 16.0, 0.0],
                sensitivity: 1.0,
                drag_threshold: list::DRAG_THRESHOLD,
            }),
            viewport_height: 300.0,
            ..ItemSelectPage::default()
        };
        assert_eq!(
            page.released(0, Some(Hit::Tab(1))),
            Some(ItemAction::SelectTab(2))
        );
        assert_eq!(page.selected_tab, 2);
        assert_eq!(page.released(0, Some(Hit::Tab(1))), None);
        page.scroll.offset = 120.0;
        page.scroll.refresh(false);
        assert_eq!(page.scroll.offset, 120.0);
        page.scroll.refresh(true);
        assert_eq!(page.scroll.offset, 0.0);
        assert_eq!(
            page.released(0, Some(Hit::Cell(1))),
            Some(ItemAction::SelectItem(12))
        );
        assert_eq!(page.selected, Some(12));
    }

    #[test]
    fn a_held_press_opens_the_detail_once() {
        let mut page = ItemSelectPage {
            models: vec![7, 8],
            selected: Some(7),
            grid: Some(Grid {
                cell: Vec2::new(152.0, 152.0),
                spacing: Vec2::new(28.0, 28.0),
                columns: 3,
                padding: [16.0, 0.0, 16.0, 0.0],
                sensitivity: 1.0,
                drag_threshold: list::DRAG_THRESHOLD,
            }),
            viewport_height: 300.0,
            ..ItemSelectPage::default()
        };
        page.pressed(0, Vec2::ZERO, Some(Hit::Cell(1)), true, 10.0);
        assert_eq!(page.step(10.2), None);
        assert_eq!(page.step(10.3), Some(ItemAction::Detail(8)));
        assert_eq!(page.step(10.4), None);
        // The release after the long press does not also select.
        assert_eq!(page.released(0, Some(Hit::Cell(1))), None);
    }
}
