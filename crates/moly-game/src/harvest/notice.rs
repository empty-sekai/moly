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
//!   limit, new or rare;
//! - `mysekai_item` (a master row): `se_get_material`, rare;
//! - `mysekai_blueprint` and `mysekai_music_record`: no notice; a
//!   get-resource sub-window dialog is chained instead (a repeated blueprint
//!   or record becomes the surplus item's row);
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
//! Named gaps: the notice is a message here ([`CollectNotice`]) and nothing
//! draws it yet (the notice layer's view); the names and preview bundles are
//! the drawer's to resolve from the masters by id. The blueprint and record
//! dialogs are named in the log and not opened (the get-resource dialog's
//! collect opener has no entry from here).

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
    /// `material` (the game's general material master).
    Material,
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
        RT_MYSEKAI_BLUEPRINT | RT_MYSEKAI_MUSIC_RECORD => {
            let word = if resource_type == RT_MYSEKAI_BLUEPRINT {
                "blueprint"
            } else {
                "music record"
            };
            warn!(
                "[harvest-pickup] NoticeCollectItem ({reason}): {word} {resource_id} qty {quantity}: MysekaiGetResourceSubWindowDialog chained in the source; not opened (the dialog has no entry from the harvest path)"
            );
            return;
        }
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
        "[harvest-pickup] NoticeCollectItem ({reason}): {:?} {} x{} limit {} new {} rare {} (ScreenLayerMysekaiNotice: not drawn)",
        notice.arm, notice.resource_id, notice.get_count, notice.is_limit, notice.is_new, notice.is_rare
    );
    notices.push(notice);
}
