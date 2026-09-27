//! The collection notice (`MysekaiResourceUtility.NoticeCollectItem`).
//!
//! Raised by `HarvestUtility.OnCollectDropItem` for each collected drop
//! (before its gather stack is queued) and by the player's collision owner
//! when it touches a drop the inventory cannot take. By resource type:
//! - `mysekai_material` (a master row): `isLimit` = not `CanCollectMaterial`
//!   (read before this drop's stack is queued), `isNew` =
//!   `IsNewGetMaterial`, the drop cue (`PlayDropItemSound`: a tone material,
//!   or a rarity of rarity_2 and above, plays `se_get_rare_material`, else
//!   `se_get_material`), `isRare` = rarity_2 to rarity_4;
//! - `mysekai_fixture` and `material` (a master row): `se_get_material`, not
//!   limit, new or rare (the general `material` master is not an input here,
//!   so that arm is refused with an error line);
//! - `mysekai_item` (a master row): `se_get_material`, rare;
//! - `mysekai_blueprint` and `mysekai_music_record`: no notice and no cue;
//!   a get-resource sub-window dialog is chained instead (a repeated
//!   blueprint or record becomes the surplus item's row). The message
//!   carries the arm and the acquisition module ([`crate::get_resource`])
//!   chains the dialog;
//! - any other type: a notice with an empty name, no cue.
//!
//! A type whose master row is absent raises nothing. The notice then goes
//! to `MysekaiUtility.NoticeCollectItem(itemName, assetBundleName,
//! resourceName, getCount, isLimit, isNew, isRare)`, the notice layer's
//! (`ScreenLayerMysekaiNotice`).
//!
//! `IsNewGetMaterial(id)`: the user has no material row of the id
//! (`IsNewMaterial`) and the id is not in `_alreadyCollectMaterials`. That
//! set is private and only `MarkEarnedMaterial` adds to it, and only when it
//! already holds the id (both builds read that way), so it stays empty: a
//! material is new at every notice until the user holds a row of it.
//!
//! The notice is a message here ([`CollectNotice`]). The acquisition module
//! ([`crate::get_resource`]) reads it: it resolves the name, the preview
//! bundle and the resource name from the masters by id and hands the notice
//! layer's call (`crate::collect_notice::NoticeCollectItem`) the seven
//! values, or chains the get-resource dialog for a blueprint or a record.
//! The cues play here, once; that module plays none for these messages.

use bevy::prelude::*;

use super::catalog::{HarvestCatalog, HarvestUserData};
use super::possession::Capacity;
use super::{
    RT_MATERIAL, RT_MYSEKAI_BLUEPRINT, RT_MYSEKAI_FIXTURE, RT_MYSEKAI_ITEM, RT_MYSEKAI_MATERIAL,
    RT_MYSEKAI_MUSIC_RECORD,
};
use crate::audio::SeRequests;

/// `MysekaiMaterialType.tone`.
const MATERIAL_TYPE_TONE: i32 = 6;

/// The resource type arm of the notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoticeArm {
    MysekaiMaterial,
    MysekaiFixture,
    MysekaiItem,
    /// `mysekai_blueprint`: the get-resource dialog, no notice.
    MysekaiBlueprint,
    /// `mysekai_music_record`: the get-resource dialog, no notice.
    MysekaiMusicRecord,
    /// A type outside the switch (empty name).
    Other,
}

/// `MysekaiUtility.NoticeCollectItem`'s arguments, by id (the drawer
/// resolves the name, the preview bundle and the resource name).
#[derive(Message, Clone, Debug)]
pub(crate) struct CollectNotice {
    pub(crate) arm: NoticeArm,
    pub(crate) resource_id: i64,
    pub(crate) get_count: i32,
    pub(crate) is_limit: bool,
    pub(crate) is_new: bool,
    pub(crate) is_rare: bool,
}

/// `MysekaiResourceUtility.NoticeCollectItem(resourceType, resourceId,
/// quantity)`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn notice_collect_item(
    resource_type: i32,
    resource_id: i64,
    quantity: i32,
    capacity: &Capacity,
    catalog: &HarvestCatalog,
    user: &HarvestUserData,
    se: &mut SeRequests,
    notices: &mut Vec<CollectNotice>,
    reason: &str,
) {
    let cue = |se: &mut SeRequests, name: &'static str| {
        super::damage::push_se(se, name, "harvest-pickup");
    };
    let notice = match resource_type {
        RT_MYSEKAI_MATERIAL => {
            let Some(row) = catalog.materials.get(&resource_id) else {
                info!("[harvest-pickup] NoticeCollectItem ({reason}): mysekai_material {resource_id} has no master row: nothing");
                return;
            };
            let is_limit = !capacity.can_collect_material(resource_id, quantity);
            let is_new = !user.materials.contains_key(&resource_id);
            let rare_cue = row.material_type == MATERIAL_TYPE_TONE || row.rarity >= 1;
            cue(
                se,
                if rare_cue {
                    "se_get_rare_material"
                } else {
                    "se_get_material"
                },
            );
            CollectNotice {
                arm: NoticeArm::MysekaiMaterial,
                resource_id,
                get_count: quantity,
                is_limit,
                is_new,
                is_rare: (1..=3).contains(&row.rarity),
            }
        }
        RT_MYSEKAI_FIXTURE | RT_MATERIAL | RT_MYSEKAI_ITEM => {
            let (arm, known) = match resource_type {
                RT_MYSEKAI_FIXTURE => (
                    NoticeArm::MysekaiFixture,
                    catalog.fixture_ids.contains(&resource_id),
                ),
                RT_MYSEKAI_ITEM => (
                    NoticeArm::MysekaiItem,
                    catalog.item_ids.contains(&resource_id),
                ),
                _ => {
                    // The general material master is not an input here.
                    error!("[harvest-pickup] NoticeCollectItem ({reason}): material {resource_id}: the material master is not loaded; nothing raised");
                    return;
                }
            };
            if !known {
                info!("[harvest-pickup] NoticeCollectItem ({reason}): {arm:?} {resource_id} has no master row: nothing");
                return;
            }
            cue(se, "se_get_material");
            CollectNotice {
                arm,
                resource_id,
                get_count: quantity,
                is_limit: false,
                is_new: false,
                is_rare: arm == NoticeArm::MysekaiItem,
            }
        }
        // No cue: the get-resource dialog is chained by the acquisition
        // module, which checks the master rows and the user's copies.
        RT_MYSEKAI_BLUEPRINT | RT_MYSEKAI_MUSIC_RECORD => CollectNotice {
            arm: if resource_type == RT_MYSEKAI_BLUEPRINT {
                NoticeArm::MysekaiBlueprint
            } else {
                NoticeArm::MysekaiMusicRecord
            },
            resource_id,
            get_count: quantity,
            is_limit: false,
            is_new: false,
            is_rare: false,
        },
        _ => CollectNotice {
            arm: NoticeArm::Other,
            resource_id,
            get_count: quantity,
            is_limit: false,
            is_new: false,
            is_rare: false,
        },
    };
    info!(
        "[harvest-pickup] NoticeCollectItem ({reason}): {:?} {} x{} limit {} new {} rare {} (to the acquisition module, which draws it in the collect-item notice cells)",
        notice.arm, notice.resource_id, notice.get_count, notice.is_limit, notice.is_new, notice.is_rare
    );
    notices.push(notice);
}
