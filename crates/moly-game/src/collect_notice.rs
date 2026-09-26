//! The collect-item notice: `ScreenLayerMysekaiNotice.NoticeCollectItem`, the
//! entry point every collect notice goes through.
//!
//! The source: the openers (harvest pickups, delivery, crafting and talk
//! replies) call `MysekaiResourceUtility.NoticeCollectItem(resourceType,
//! resourceId, quantity)`, which resolves the item's name, its icon's bundle
//! and asset names and the limit, new and rare flags from the masters and the
//! user's data, then `MysekaiUtility.NoticeCollectItem` hands those seven
//! values to the notice layer. The layer's `NoticeCollectItem` takes a view
//! data object from its pool of 64, sets the seven values and a notice
//! duration of 2.5 s when the item is new or rare and 1.5 s otherwise, and
//! enqueues it on the collect-item slide notice controller, initialized with
//! 5 cells shown at once, a queue of 8, the layer's `collectItemCellPrefab`
//! under `collectItemRoot`, sliding in from the left.
//!
//! [`NoticeCollectItem`] is that layer call, with the same seven values. The
//! openers resolve them; this module owns the cell's drawing.
//!
//! Not drawn yet: the slide notice controller, the cell's `Refresh` (name,
//! count, new mark, rare effect, limit text, icon) and its open and close
//! animations are not built, so each call is logged with its duration and
//! refused by name instead of being drawn.

use bevy::ecs::message::MessageReader;
use bevy::prelude::*;

/// `ScreenLayerMysekaiNotice.NoticeCollectItem(itemName, assetBundleName,
/// resourceName, getCount, isLimit, isNew, isRare)`, one per collected item.
#[derive(Message, Clone, Debug, PartialEq)]
pub(crate) struct NoticeCollectItem {
    /// The item's display name.
    pub(crate) item_name: String,
    /// The bundle the icon is loaded from (for a material,
    /// `AssetBundleNames.GetMysekaiMaterialPreview(iconAssetbundleName)`).
    pub(crate) asset_bundle_name: String,
    /// The icon's asset name inside that bundle.
    pub(crate) resource_name: String,
    /// The quantity collected.
    pub(crate) get_count: i32,
    /// The inventory cannot take the item.
    pub(crate) is_limit: bool,
    /// The item is collected for the first time.
    pub(crate) is_new: bool,
    /// The item is rare. For a material the opener sets it from the master's
    /// `MysekaiMaterialRarityType` (rarity_1 = 0, rarity_2 = 1, rarity_3 = 2,
    /// rarity_4 = 3) with one unsigned compare, `rarity - 1 < 3`: rarity_2 to
    /// rarity_4 are rare, rarity_1 wraps and is not.
    pub(crate) is_rare: bool,
}

impl NoticeCollectItem {
    /// `NoticeViewDataBase.NoticeDuration` the layer sets: 2.5 s for a new or
    /// rare item, 1.5 s otherwise.
    pub(crate) fn notice_duration(&self) -> f32 {
        if self.is_new || self.is_rare {
            2.5
        } else {
            1.5
        }
    }
}

/// Each call, logged with the values the layer would enqueue.
fn receive(mut calls: MessageReader<NoticeCollectItem>) {
    for call in calls.read() {
        warn!(
            "[notice] NoticeCollectItem(bundle {}, resource {}, count {}, limit {}, new {}, rare {}): duration {} s; not drawn, the collect-item cell view is not built",
            crate::balloon::ascii_or(&call.asset_bundle_name),
            crate::balloon::ascii_or(&call.resource_name),
            call.get_count,
            call.is_limit,
            call.is_new,
            call.is_rare,
            call.notice_duration()
        );
    }
}

pub(crate) fn install(app: &mut App) {
    app.add_message::<NoticeCollectItem>()
        .add_systems(Update, receive);
}
