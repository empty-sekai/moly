//! The item select page's detail panel (`MysekaiConvertFurnitureItemSelectDetailView`,
//! `MysekaiConvertFurnitureItemSelectDetailViewModel`).
//!
//! The view model's numbers are [`super::model::DetailModel`]; this module
//! binds the document's nodes to them and draws them.
//!
//! `DetailView.SetupView` order: `slotView.Setup`, `SetupObtainedResource`,
//! `SetupConsumedResource`, `SetupSelectConvertCount`,
//! `SetupConvertTimeString`, `SetupConvertStartButton`,
//! `SetupRemainingCount`, `SetupResetAt`, `SetupEndAt`, `SetupInfo`,
//! `SetupBalloon`. `SetupEmpty` (no model) turns the gauge, the count and the
//! start button off.
//!
//! The count selector is `UIPartsSelectCost`: its `ViewData(ConvertCount, 1,
//! GetOnceConvertibleCount(), OnChangedConvertCount)` bounds the count to one
//! convert at least and `GetOnceConvertibleCount` at most; `max <= 1` puts
//! the slider at its one-value maximum (`SetSliderPositionOneValueMax`). This
//! panel answers the minus and plus buttons, the number and the slider's
//! fill; the numeric input field and the slider's own drag are named, not
//! run.
//!
//! The consumption rows: the source instantiates
//! `MysekaiConvertFurnitureConsumedResourceCell` under a prefab the document
//! does not carry (no extracted document is named after that class, and its
//! fields are not on this root). The panel's required-material group
//! (`NeedsMaterialView`, a `UIPartsGroupVerticalListView` whose content is
//! empty) is the list the view fills, so this panel instantiates the craft
//! screen's `MaterialCostCell` (a thumbnail, a needed count and a held count)
//! under that content. A held count is white while the requirement is met and
//! `ColorUtility.FONT_COLOR_PINK2` otherwise; this layout has no text-colour
//! override, so the count keeps its authored colour (named once).

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use moly_assets::ui_layout::UiPrefab;
use moly_masters::convert::BoxDetail;
use serde_json::Value;

use super::model::{DetailModel, ItemContext, ItemModel};
use super::screen::note_once;
use super::slot::{format_wording, put_text, to_time_string};
use crate::ui_inputs::BalloonArt;
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// `WORD_CONVERT_ITEM_REMAINING_COUNT`.
pub const REMAINING_KEY: &str = "WORD_CONVERT_ITEM_REMAINING_COUNT";
/// `WORD_CONVERT_ITEM_REFRESH`.
pub const REFRESH_KEY: &str = "WORD_CONVERT_ITEM_REFRESH";
/// `WORD_CONVERT_ITEM_END_AT`.
pub const END_AT_KEY: &str = "WORD_CONVERT_ITEM_END_AT";
/// `FORMAT_CONVERT_SHORTEN_TIME_HAVE_JOINED_PASS`.
pub const SHORTEN_KEY: &str = "FORMAT_CONVERT_SHORTEN_TIME_HAVE_JOINED_PASS";
/// `MSG_CONVERT_NOT_ENOUGH_CONSUMED_RESOURCE`.
pub const NOT_ENOUGH_KEY: &str = "MSG_CONVERT_NOT_ENOUGH_CONSUMED_RESOURCE";
/// `WORD_CONVERT_FIXTURE_LEVEL_RELEASE`.
pub const LOCKED_KEY: &str = "WORD_CONVERT_FIXTURE_LEVEL_RELEASE";
/// `WORD_MYSEKAI_CONVERT_OUT_OF_STOCK_DETAIL`.
pub const SOLD_OUT_KEY: &str = "WORD_MYSEKAI_CONVERT_OUT_OF_STOCK_DETAIL";
/// `WORD_  REQUIRED_MATERIAL` (the source's key spells two spaces).
pub const REQUIRED_KEY: &str = "WORD_  REQUIRED_MATERIAL";
/// The document the cost cell template comes from (`MaterialCostCell`).
pub const COST_DOCUMENT: &str = "CraftScreen";
/// The cost cell's class in that document.
const COST_CLASS: &str = "MaterialCostCell";
/// The consumption rows: the most details a convert's consumption box holds.
pub const COST_ROWS: usize = 7;

/// The mysekai material and item master rows the panel's names and icons
/// come from.
#[derive(Debug, Clone, Copy, Default)]
pub struct Masters<'a> {
    tables: Option<&'a BTreeMap<&'static str, Vec<Value>>>,
}

impl<'a> Masters<'a> {
    /// The rows of the master tables, by table name.
    pub fn new(tables: &'a BTreeMap<&'static str, Vec<Value>>) -> Self {
        Self {
            tables: Some(tables),
        }
    }

    fn row(&self, table: &str, id: i32) -> Option<&Value> {
        self.tables?
            .get(table)?
            .iter()
            .find(|row| row.get("id").and_then(Value::as_i64) == Some(i64::from(id)))
    }

    /// The client load path and resource of a `UserResource`'s icon
    /// (`UIPartsItemThumbnail.Setup`): the mysekai material and item masters
    /// carry `iconAssetbundleName`; a type this reader has no master for has
    /// none, which the callers name.
    pub fn icon(&self, resource_type: &str, resource_id: i32) -> Option<(String, String)> {
        let (family, table) = match resource_type {
            "mysekai_material" => ("material", "mysekaiMaterials"),
            "mysekai_item" => ("item", "mysekaiItems"),
            _ => return None,
        };
        let icon = self
            .row(table, resource_id)?
            .get("iconAssetbundleName")?
            .as_str()?;
        Some((format!("mysekai/thumbnail/{family}/{icon}"), icon.to_owned()))
    }

    /// `GetResourceName`: the mysekai material and item masters' `name`.
    pub fn name(&self, resource_type: &str, resource_id: i32) -> Option<String> {
        let table = match resource_type {
            "mysekai_material" => "mysekaiMaterials",
            "mysekai_item" => "mysekaiItems",
            _ => return None,
        };
        self.row(table, resource_id)?
            .get("name")?
            .as_str()
            .map(str::to_owned)
    }
}

/// The detail panel's nodes.
#[derive(Debug, Clone, Default)]
pub struct DetailNodes {
    pub gauge: String,
    pub item_name: String,
    pub slot_number: String,
    pub possession: String,
    pub possession_value: String,
    pub remaining: String,
    pub remaining_text: String,
    pub reset_at: String,
    pub end_at: String,
    /// The gauge's item thumbnail (`UIPartsMysekaiConvertItem`'s image).
    pub item_image: String,
    /// `LockedObj` and its balloon (`ShowLockedBalloon`).
    pub locked: String,
    pub locked_balloon: String,
    pub sold_out: String,
    pub sold_out_text: String,
    pub not_enough: String,
    pub select_cost: String,
    pub count: String,
    pub decrement: String,
    pub increment: String,
    pub slider: String,
    pub needs_none: String,
    pub required_title: String,
    pub start: String,
    pub start_cover: String,
    pub convert_time: String,
    pub shorten_balloon: String,
    pub shorten_text: String,
    pub not_enough_balloon: String,
    pub not_enough_text: String,
    pub blueprints: String,
    pub records: String,
    pub costs: Vec<CostNodes>,
}

/// One consumption row's nodes.
#[derive(Debug, Clone, Default)]
pub struct CostNodes {
    pub root: String,
    pub image: String,
    pub need: String,
    pub have: String,
    /// The thumbnail's parts this panel does not drive (the count, the label,
    /// the zoom icon, the cover, the disabled overlay and the frame).
    pub hidden: Vec<String>,
}

fn sub(root: &str, below: &str) -> String {
    format!("{root}/{below}")
}

/// The detail panel's nodes and its consumption rows, below the page's
/// `ItemDetail`; a root without the cost cell document gets no rows (named).
pub fn compose(doc: &mut UiPrefab, layouts: &UiLayouts, view: &str) -> Result<DetailNodes, String> {
    let root = sub(view, "ItemDetail/ItemDetail");
    doc.find(&root).map_err(|error| error.to_string())?;
    let gauge = sub(&root, "Slot/Root/GaugeView");
    let possession = sub(&gauge, "PossessionCount");
    let remaining = sub(&gauge, "InfoContent/RemainingCountInfo");
    let locked = sub(&gauge, "LockedObj");
    let locked_balloon = sub(&locked, "Balloon");
    let sold_out = sub(&gauge, "SoldOutObj");
    let start = sub(&root, "ConvertStartButton");
    let select_cost = sub(&root, "UIPartsSelectCost");
    let needs = sub(&root, "NeedsMaterialView");
    let needs_content = sub(
        &needs,
        "Base/UIPartsGroupVerticalListView/ListView/Viewport/Content",
    );
    doc.find(&needs_content).map_err(|error| error.to_string())?;
    let item_image = sub(
        &gauge,
        "UIPartsConvertItem/UIPartsItemThumbnail (2)/mask/Icon",
    );
    let count = sub(&select_cost, "Num/Text");

    // The consumption rows, under the required-material group's content.
    let mut costs = Vec::new();
    let template = layouts.document(COST_DOCUMENT);
    let cell = template.and_then(|template| {
        template
            .nodes
            .iter()
            .find(|node| {
                node.components
                    .iter()
                    .any(|component| component.class.ends_with(&format!(".{COST_CLASS}")))
            })
            .map(|node| node.path.clone())
    });
    match (template, cell) {
        (Some(template), Some(cell)) => {
            let names: Vec<String> = (0..COST_ROWS).map(|index| format!("Cost{index}")).collect();
            let instances = doc.instantiate_children(
                &needs_content,
                template,
                &cell,
                names.iter().map(String::as_str),
            )?;
            for instance in &instances {
                let root = instance.root_selector();
                costs.push(CostNodes {
                    hidden: [
                        "UIPartsGeneralItem/Num",
                        "UIPartsGeneralItem/LabelImage",
                        "UIPartsGeneralItem/ZoomIcon",
                        "UIPartsGeneralItem/CoverEffect",
                        "UIPartsGeneralItem/Disabled",
                        "UIPartsGeneralItem/FrontContent",
                    ]
                    .into_iter()
                    .map(|name| sub(&root, name))
                    .collect(),
                    image: sub(&root, "UIPartsGeneralItem/mask/Icon"),
                    need: sub(&root, "NeedCountText"),
                    have: sub(&root, "HasCountText"),
                    root,
                });
            }
        }
        _ => {
            let mut seen = BTreeSet::new();
            note_once(
                &mut seen,
                format!("{COST_DOCUMENT} (the cost cell template) is not on this root: the detail panel draws no consumption rows"),
            );
        }
    }
    let not_enough = sub(&gauge, "NotEnoughConsumedResourceObj");
    Ok(DetailNodes {
        item_name: sub(&gauge, "ItemName/Text"),
        slot_number: sub(&gauge, "SlotNumberText"),
        possession_value: sub(&possession, "ValueText"),
        remaining_text: sub(&remaining, "CountText"),
        reset_at: sub(&gauge, "InfoContent/ResetAtText"),
        end_at: sub(&gauge, "InfoContent/EndAtText"),
        locked_balloon: sub(&locked_balloon, "Content/CustomText"),
        sold_out_text: sub(&sold_out, "Info/SettingText"),
        decrement: sub(&select_cost, "UIPartsDecrementButton"),
        increment: sub(&select_cost, "UIPartsIncrementButton"),
        slider: sub(&select_cost, "UIPartsSlider"),
        needs_none: sub(
            &needs,
            "Base/UIPartsGroupVerticalListView/ListView/NoneInfo",
        ),
        required_title: sub(&needs, "Title/CustomTextMesh"),
        start_cover: sub(&start, "Cover"),
        convert_time: sub(&start, "Content/ConvertTime/Text"),
        shorten_balloon: sub(&start, "ShortenByJoinedPassBalloon"),
        shorten_text: sub(&start, "ShortenByJoinedPassBalloon/Content/Text"),
        not_enough_balloon: sub(&start, "NotEnoughConsumedResourceBalloon"),
        not_enough_text: sub(&start, "NotEnoughConsumedResourceBalloon/Text"),
        blueprints: sub(&gauge, "MysekaiBlueprintRandomBoxCategoryGroup"),
        records: sub(&gauge, "MysekaiRecordRandomBoxCategoryGroup"),
        costs,
        item_image,
        start,
        gauge,
        possession,
        remaining,
        locked,
        sold_out,
        not_enough,
        select_cost,
        count,
    })
}

/// `MysekaiConvertFurnitureConsumedResourceCell.ViewData`: one consumption
/// detail at the count, with the held quantity and whether it is enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumedRow {
    pub quantity: i32,
    pub possession: i32,
    /// `IsEnoughResource`: the held quantity is at least the needed one.
    pub enough: bool,
}

/// `ConsumedResourceCellDataList` at `count` (`DetailModel.Setup(count)`).
pub fn consumed_rows(ctx: ItemContext<'_>, item: &ItemModel<'_>, count: i32) -> Vec<ConsumedRow> {
    let holdings = ctx.user.holdings.clone();
    ctx.machine()
        .consumption(item.master())
        .into_iter()
        .map(|detail| {
            let quantity = detail.quantity.saturating_mul(count.max(1));
            let possession = holdings.held(&detail.resource_type, detail.resource_id);
            ConsumedRow {
                quantity,
                possession,
                enough: possession >= quantity,
            }
        })
        .collect()
}

/// `_obtainUserResources.Length <= 1`'s single resource of a normal item.
pub fn single_obtain<'a>(ctx: ItemContext<'a>, item: &ItemModel<'a>) -> Option<&'a BoxDetail> {
    if !matches!(item, ItemModel::Normal { .. }) {
        return None;
    }
    match ctx.machine().obtain(item.master()).as_slice() {
        [detail] => Some(*detail),
        _ => None,
    }
}

/// `DetailViewModel` at the item, slot and count: the model the panel binds.
fn model_for(
    ctx: ItemContext<'_>,
    item: Option<i32>,
    slot_id: i32,
    count: i32,
) -> Option<DetailModel<'_>> {
    let item = ItemModel::all(ctx)
        .into_models()
        .into_iter()
        .find(|model| Some(model.id()) == item)?;
    Some(DetailModel::new(ctx, item, slot_id, count))
}

/// The panel's state between frames.
#[derive(Debug, Default)]
pub struct DetailPanel {
    /// `_viewModel.ConvertItemModel`'s item id.
    item: Option<i32>,
    /// `ConvertSlotId`.
    slot_id: i32,
    /// `ConvertCount`.
    count: i32,
    seen: BTreeSet<String>,
}

impl DetailPanel {
    /// `RefreshItemDetailView`: `DetailView.Setup(model)`, then
    /// `Setup(convertCount)` at one convert.
    pub fn setup(&mut self, item: i32, slot_id: i32) {
        self.item = Some(item);
        self.slot_id = slot_id;
        self.count = 1;
    }

    /// `SetupEmpty`.
    pub fn clear(&mut self) {
        self.item = None;
        self.count = 1;
    }

    pub fn count(&self) -> i32 {
        self.count
    }

    /// `GetOnceConvertibleCount()` at the slot: the count selector's bound.
    pub fn max(&self, ctx: ItemContext<'_>) -> i32 {
        match model_for(ctx, self.item, self.slot_id, self.count) {
            Some(model) => model.max_convert_count(ctx).max(1),
            None => 1,
        }
    }

    /// `OnChangedConvertCount(count)`: the count inside the selector's
    /// bounds (`UIPartsSelectCost`'s min 1 and `GetOnceConvertibleCount`).
    pub fn set_count(&mut self, ctx: ItemContext<'_>, count: i32) {
        if self.item.is_none() {
            return;
        }
        let max = self.max(ctx);
        self.count = count.clamp(1, max);
    }

    /// The panel's control at a node path, if any.
    pub fn hit(&self, nodes: &DetailNodes, path: &str) -> Option<Action> {
        if self.item.is_none() {
            return None;
        }
        let inside = |root: &str| path == root || path.starts_with(&format!("{root}/"));
        if inside(&nodes.decrement) {
            return Some(Action::Decrement);
        }
        if inside(&nodes.increment) {
            return Some(Action::Increment);
        }
        if inside(&nodes.start) {
            return Some(Action::Start);
        }
        if inside(&nodes.locked) {
            return Some(Action::LockedBalloon);
        }
        None
    }

    /// One frame of the panel: `SetupView`'s bindings, in its order. `name`
    /// is `GetItemName` of the item the panel shows.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        view: &mut UiPrefabView,
        layouts: &UiLayouts,
        art: Option<&BalloonArt>,
        nodes: &DetailNodes,
        ctx: ItemContext<'_>,
        masters: &Masters<'_>,
        name: Option<String>,
    ) {
        let Some(model) = model_for(ctx, self.item, self.slot_id, self.count) else {
            // `SetupEmpty`: the gauge, the count and the start button off.
            view.set_visible(&nodes.gauge, false);
            view.set_visible(&nodes.select_cost, false);
            view.set_visible(&nodes.start, false);
            return;
        };
        let item = model.item;
        view.set_visible(&nodes.gauge, true);
        view.set_visible(&nodes.select_cost, true);
        view.set_visible(&nodes.start, true);

        // `slotView.Setup(slotId, model)` and `SetupObtainedResource`.
        let slot_number = format_wording(
            layouts,
            &mut self.seen,
            super::slot::SLOT_NO_FORMAT,
            &[self.slot_id.to_string()],
        );
        put_text(
            view,
            &nodes.slot_number,
            Some(&slot_number),
            art,
            &mut self.seen,
        );
        let name = put_text(view, &nodes.item_name, name.as_ref(), art, &mut self.seen);
        view.set_visible(&nodes.item_name, name);
        view.set_visible(&nodes.blueprints, matches!(item, ItemModel::Blueprint { .. }));
        view.set_visible(&nodes.records, matches!(item, ItemModel::Record { .. }));
        let sold_out = layouts.wording(SOLD_OUT_KEY);
        let shows = put_text(view, &nodes.sold_out_text, Some(&sold_out), art, &mut self.seen);
        view.set_visible(&nodes.sold_out_text, shows);
        // `UIPartsMysekaiConvertItem.Setup(model)`: the obtained item.
        if let Some((path, _)) = super::item_select::icon_load(ctx, &item, masters) {
            if layouts.has_runtime_texture(&path) {
                view.set_texture(&nodes.item_image, &path);
            } else {
                note_once(
                    &mut self.seen,
                    format!("the detail thumbnail {path} is not on this root: the image keeps its serialized texture"),
                );
            }
        }

        // `PossessionCount`: a normal item with one obtain resource.
        let single = single_obtain(ctx, &item);
        let held = single
            .as_ref()
            .map(|detail| ctx.user.holdings.held(&detail.resource_type, detail.resource_id));
        put_text(
            view,
            &nodes.possession_value,
            held.map(|held| held.to_string()).as_ref(),
            art,
            &mut self.seen,
        );
        view.set_visible(&nodes.possession, held.is_some());

        // `SetupConsumedResource`. The cell's held count is white while the
        // requirement is met and `ColorUtility.FONT_COLOR_PINK2` otherwise;
        // this layout has no text-colour override (a text draws its serialized
        // `m_fontColor`), so the count keeps its authored colour (named once).
        let rows = consumed_rows(ctx, &item, self.count);
        let details = ctx.machine().consumption(item.master());
        view.set_visible(&nodes.needs_none, rows.is_empty());
        for (index, cell) in nodes.costs.iter().enumerate() {
            match (rows.get(index), details.get(index)) {
                (Some(row), Some(detail)) => {
                    view.set_visible(&cell.root, true);
                    for path in &cell.hidden {
                        view.set_visible(path, false);
                    }
                    view.set_text(&cell.need, row.quantity.to_string());
                    view.set_text(&cell.have, row.possession.to_string());
                    match masters.icon(&detail.resource_type, detail.resource_id) {
                        Some((path, _)) if layouts.has_runtime_texture(&path) => {
                            view.set_visible(&cell.image, true);
                            view.set_texture(&cell.image, &path);
                        }
                        _ => {
                            view.set_visible(&cell.image, false);
                            note_once(
                                &mut self.seen,
                                "a consumption row's resource is not a mysekai material or item (the two tables whose icons this reader has): the row draws its counts without a thumbnail".to_owned(),
                            );
                        }
                    }
                }
                _ => view.set_visible(&cell.root, false),
            }
        }
        if rows.iter().any(|row| !row.enough) {
            note_once(
                &mut self.seen,
                "a consumption row's held count keeps its serialized text colour: a short one is not turned to FONT_COLOR_PINK2".to_owned(),
            );
        }
        let line = layouts.wording(REQUIRED_KEY);
        let shows = put_text(view, &nodes.required_title, Some(&line), art, &mut self.seen);
        view.set_visible(&nodes.required_title, shows);

        // `SetupSelectConvertCount`, and
        // `UIPartsSelectCost.SetSliderPositionOneValueMax` when the bound is
        // one.
        let max = self.max(ctx);
        view.set_text(&nodes.count, self.count.to_string());
        let fill = if max <= 1 {
            1.0
        } else {
            (self.count - 1) as f32 / (max - 1) as f32
        };
        view.set_slider(&nodes.slider, fill);

        // `SetupConvertTimeString`.
        let time = to_time_string(model.convert_ms);
        put_text(view, &nodes.convert_time, Some(&time), art, &mut self.seen);

        // `SetupConvertStartButton`: `enabled = IsAllowedToConvertStart`.
        view.set_visible(&nodes.start_cover, !model.is_allowed_to_convert_start);

        // `SetupRemainingCount`, `SetupResetAt` and `SetupEndAt`.
        let limited = item.master().convert_limit > 0;
        let left = item.convertible_count(ctx);
        let shows_remaining = limited && item.has_convertible_count(ctx);
        view.set_visible(&nodes.remaining, shows_remaining);
        if shows_remaining {
            let line = format_wording(
                layouts,
                &mut self.seen,
                REMAINING_KEY,
                &[left.to_string()],
            );
            put_text(view, &nodes.remaining_text, Some(&line), art, &mut self.seen);
        }
        let reset_at = item.reset_at(ctx);
        view.set_visible(&nodes.reset_at, reset_at > 0);
        if reset_at > 0 {
            let line = format_wording(
                layouts,
                &mut self.seen,
                REFRESH_KEY,
                &[month_day_time(reset_at, ctx.utc_offset_ms())],
            );
            put_text(view, &nodes.reset_at, Some(&line), art, &mut self.seen);
        }
        let end_at = item.end_at();
        view.set_visible(&nodes.end_at, end_at > 0);
        if end_at > 0 {
            let line = format_wording(
                layouts,
                &mut self.seen,
                END_AT_KEY,
                &[month_day_time(end_at, ctx.utc_offset_ms())],
            );
            put_text(view, &nodes.end_at, Some(&line), art, &mut self.seen);
        }

        // `SetupInfo`: one badge of three.
        let released = item.is_released(ctx.machine().level(ctx.user));
        let convertible = item.has_convertible_count(ctx);
        let enough = item.affordable_converts(ctx) >= 1;
        view.set_visible(&nodes.locked, !released);
        view.set_visible(&nodes.sold_out, released && !convertible);
        view.set_visible(&nodes.not_enough, released && convertible && !enough);
        let locked = layouts.wording(LOCKED_KEY);
        let shows = put_text(view, &nodes.locked_balloon, Some(&locked), art, &mut self.seen);
        view.set_visible(&nodes.locked_balloon, shows);

        // `SetupBalloon`: the resource balloon first, the pass's otherwise.
        let short = model.shows_not_enough_resource_balloon(ctx) && art.is_some();
        view.set_visible(&nodes.not_enough_balloon, short);
        if short {
            let line = layouts.wording(NOT_ENOUGH_KEY);
            let shows = put_text(view, &nodes.not_enough_text, Some(&line), art, &mut self.seen);
            view.set_visible(&nodes.not_enough_text, shows);
        }
        let pass = !short && model.shows_shorten_by_joined_pass_balloon(ctx) && art.is_some();
        view.set_visible(&nodes.shorten_balloon, pass);
        if pass {
            let percent = model.shorten_by_joined_pass_percent(ctx).round() as i32;
            let line = format_wording(
                layouts,
                &mut self.seen,
                SHORTEN_KEY,
                &[percent.to_string()],
            );
            let shows = put_text(view, &nodes.shorten_text, Some(&line), art, &mut self.seen);
            view.set_visible(&nodes.shorten_text, shows);
        }
    }
}

/// A press on the detail panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `UIPartsDecrementButton`.
    Decrement,
    /// `UIPartsIncrementButton`.
    Increment,
    /// `ConvertStartButton` (`InvokeConvertStart`).
    Start,
    /// `LockedObj`'s press (`ShowLockedBalloon`).
    LockedBalloon,
}

/// `"MM/dd HH:mm"` at the local offset (`GetResetAtMessage`, `GetEndAtMessage`).
pub fn month_day_time(ms: i64, offset_ms: i64) -> String {
    let local = ms.saturating_add(offset_ms);
    let days = local.div_euclid(86_400_000);
    let rest = local.rem_euclid(86_400_000);
    let (_, month, day) = civil(days);
    let minutes = rest / 60_000;
    format!("{month:02}/{day:02} {:02}:{:02}", minutes / 60, minutes % 60)
}

/// The proleptic Gregorian date of a day count since the epoch.
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reset_line_reads_the_local_month_day_and_time() {
        // 2026-10-19 03:04 UTC is 12:04 at +09:00.
        let utc = 20_745 * 86_400_000 + 3 * 3_600_000 + 4 * 60_000;
        assert_eq!(month_day_time(utc, 9 * 3_600_000), "10/19 12:04");
        assert_eq!(month_day_time(utc, 0), "10/19 03:04");
        // 2026-01-30 22:00 UTC is the next day at +02:00.
        let january = 20_483 * 86_400_000 + 22 * 3_600_000;
        assert_eq!(month_day_time(january, 2 * 3_600_000), "01/31 00:00");
    }
}
