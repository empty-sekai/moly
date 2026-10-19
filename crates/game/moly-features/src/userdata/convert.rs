//! The client's copies of the convert screen's user data: the convert slots,
//! their item histories, and the convert items a response carries.
//!
//! `UserDataManager` keeps `UserMysekaiConvertSlots` and
//! `UserMysekaiConvertItemHistories`; each response that carries one of them
//! replaces it whole (`UpdateAll`). The convert items are the one table the
//! client receives without a `userMysekai` section name: every response and
//! the join carry `mysekaiConvertItems` (the master's rows with the
//! `convertLimit` and `onceConvertLimit` the document does not hold), and a
//! response that carries them replaces the client's list.
//!
//! The pure rules over these rows live in [`crate::convert::model`]; this
//! copy only holds and applies them.

use bevy::prelude::*;
use serde_json::Value;

use moly_protocol::json::{convert_histories_value, convert_items_value, convert_slots_value};
use moly_protocol::rows::{ConvertItemHistory, ConvertSlot, UserMysekaiConvertItem};

/// `UserDataManager.UserMysekaiConvertSlots`, `UserMysekaiConvertItemHistories`
/// and the `mysekaiConvertItems` list, set by responses only.
#[derive(Resource, Debug, Clone, Default)]
pub struct ClientConvertSlots {
    slots: Vec<ConvertSlot>,
    histories: Vec<ConvertItemHistory>,
    items: Vec<UserMysekaiConvertItem>,
    /// Applications that carried the slots (0: none yet).
    pub slots_revision: u64,
    /// Applications that carried the histories.
    pub histories_revision: u64,
    /// Applications that carried the items.
    pub items_revision: u64,
}

impl ClientConvertSlots {
    /// `UserMysekaiConvertSlots`: one row per machine slot.
    pub fn slots(&self) -> &[ConvertSlot] {
        &self.slots
    }

    /// `UserMysekaiConvertItemHistories`: the items whose converts are
    /// counted against a `convertLimit`.
    pub fn histories(&self) -> &[ConvertItemHistory] {
        &self.histories
    }

    /// `mysekaiConvertItems`: the item rows the screen builds its list from.
    pub fn items(&self) -> &[UserMysekaiConvertItem] {
        &self.items
    }

    /// The slot a `slotId` names (`GetMysekaiConvertSlot`).
    pub fn slot(&self, slot_id: i32) -> Option<&ConvertSlot> {
        self.slots.iter().find(|slot| slot.slot_id == slot_id)
    }

    /// The history row of an item, if the user has one.
    pub fn history(&self, item_id: i32) -> Option<&ConvertItemHistory> {
        self.histories
            .iter()
            .find(|row| row.mysekai_convert_item_id == item_id)
    }

    /// `UpdateUserMysekaiConvertSlots`: the carried rows replace the list.
    pub fn apply_slots(&mut self, rows: Vec<ConvertSlot>) {
        self.slots = rows;
        self.slots_revision += 1;
    }

    /// The histories: the carried rows replace the list.
    pub fn apply_histories(&mut self, rows: Vec<ConvertItemHistory>) {
        self.histories = rows;
        self.histories_revision += 1;
    }

    /// The convert items: the carried rows replace the list.
    pub fn apply_items(&mut self, rows: Vec<UserMysekaiConvertItem>) {
        self.items = rows;
        self.items_revision += 1;
    }

    /// Applies whichever of the three sections a response carried; returns
    /// how many it carried.
    pub fn apply(
        &mut self,
        slots: Option<Vec<ConvertSlot>>,
        histories: Option<Vec<ConvertItemHistory>>,
        items: Option<Vec<UserMysekaiConvertItem>>,
    ) -> usize {
        let mut carried = 0;
        if let Some(rows) = slots {
            self.apply_slots(rows);
            carried += 1;
        }
        if let Some(rows) = histories {
            self.apply_histories(rows);
            carried += 1;
        }
        if let Some(rows) = items {
            self.apply_items(rows);
            carried += 1;
        }
        carried
    }

    /// The copy as the control plane's `client` part shows it.
    pub fn view(&self) -> Value {
        serde_json::json!({
            "userMysekaiConvertSlots": convert_slots_value(&self.slots),
            "userMysekaiConvertItemHistories": convert_histories_value(&self.histories),
            "mysekaiConvertItems": convert_items_value(&self.items),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_protocol::rows::CONVERT_SLOT_COMPLETE;

    fn slot(slot_id: i32) -> ConvertSlot {
        ConvertSlot {
            slot_id,
            mysekai_convert_item_id: 3,
            choice_id: 0,
            convert_start_at: 0,
            once_convert_minutes: 0,
            convert_count: 0,
            received_progress: 0,
            mysekai_convert_slot_status: CONVERT_SLOT_COMPLETE.to_owned(),
            obtain_resources: Vec::new(),
        }
    }

    fn item(id: i32) -> UserMysekaiConvertItem {
        UserMysekaiConvertItem {
            mysekai_convert_item_id: id,
            obtain_type: "resource_box".to_owned(),
            external_id: 10003,
            required_convert_fixture_level: 1,
            convert_minutes: 12,
            convert_limit: 0,
            once_convert_limit: 100,
            refresh_cycle: "none".to_owned(),
            start_at: 0,
            end_at: 0,
        }
    }

    #[test]
    fn a_carried_section_replaces_its_copy_and_the_others_stay() {
        let mut copy = ClientConvertSlots::default();
        assert_eq!(copy.apply(Some(vec![slot(0), slot(1)]), None, Some(vec![item(3)])), 2);
        assert_eq!(copy.slots().len(), 2);
        assert_eq!(copy.slot(1).map(|row| row.slot_id), Some(1));
        assert!(copy.slot(2).is_none());
        assert_eq!(copy.items()[0].mysekai_convert_item_id, 3);
        // A response without the slots leaves them as they are; a carried
        // histories list replaces the copy, even with no row in it.
        assert_eq!(copy.apply(None, Some(Vec::new()), None), 1);
        assert_eq!(copy.slots().len(), 2);
        assert!(copy.histories().is_empty());
        assert!(copy.slots_revision == 1);
        assert_eq!(copy.histories_revision, 1);
        assert_eq!(copy.items_revision, 1);
        assert!(copy.view()["userMysekaiConvertSlots"].as_array().is_some_and(|rows| rows.len() == 2));
    }
}
