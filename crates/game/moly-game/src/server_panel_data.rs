//! The server panel's document as the client holds it: `Sections.npcPanel` of
//! the last response, which the panel parses into its own copies.

use bevy::prelude::*;

pub(crate) use moly_protocol::gate::ReplyTalkList;
pub(crate) use moly_protocol::rows::{GateCharacter, ScheduleRow};

/// The server panel's document (schemaVersion 1) as the last response
/// carried it: `Sections.npcPanel`, the rows the join serves and the current
/// schedule. `revision` counts the responses that carried one.
#[derive(Resource, Default, Debug, Clone)]
pub(crate) struct ClientNpcPanel {
    pub(crate) text: Option<String>,
    pub(crate) revision: u64,
}

impl ClientNpcPanel {
    pub(crate) fn apply(&mut self, text: String) {
        self.text = Some(text);
        self.revision += 1;
    }
}

/// The schedule the phenomenon of the day is chosen from, as the client
/// holds it: the rows and its copy of `refreshedAt`. `revision` counts
/// changes; the server panel writes the schedule again on each.
#[derive(Resource, Default, Debug, Clone)]
pub(crate) struct LiveSchedule {
    pub(crate) revision: u64,
    pub(crate) schedules: Vec<ScheduleRow>,
    pub(crate) refreshed_at: i64,
}

/// `MysekaiTalkDataStore.UpdateTalkList` / `SetTalkList` calls of the gate
/// presenter, for the talk store's owner. `visitors` is the reply's gate
/// characters when the call also changes who visits.
#[derive(Clone, Debug)]
pub(crate) struct TalkListUpdate {
    pub(crate) caller: &'static str,
    pub(crate) talks: ReplyTalkList,
    pub(crate) visitors: Option<Vec<GateCharacter>>,
}

#[derive(Resource, Default, Debug)]
pub(crate) struct ClientTalkListUpdates(pub(crate) Vec<TalkListUpdate>);

/// The gate presenter's `UpdateTalkList` / `SetTalkList` call.
pub(crate) fn client_update_talk_list(world: &mut World, update: TalkListUpdate) {
    world
        .get_resource_or_insert_with(ClientTalkListUpdates::default)
        .0
        .push(update);
}
