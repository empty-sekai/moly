//! The client's copy of the mission user data: the sheet the user is on and
//! the user's mission rows.
//!
//! `UserDataManager` keeps `UserMysekaiNormalMissionSheet` and
//! `UserMysekaiNormalMissions`; a response or a sync that carries one of them
//! replaces it whole (`UserDataManager.UpdateAll` -> `UpdateMysekaiMissionSheet`
//! and `UpdateMysekaiMission`). The copy is read as the screen's plain user
//! state ([`crate::mission::model::UserMissionState`]) by
//! `MysekaiMissionUtility.GetCurrentMissionSheetId` and
//! `MysekaiMissionUtility.GetMissionUserData`.
//!
//! The rows the server sends are carried both ways: as the sections of an
//! arrival ([`MissionSections`], applied by the one reader every other copy
//! is applied by), and inside a receive's reply (`updatedResources`,
//! [`ClientMission::apply_reply`], which runs only after the sections, so the
//! reply's own tables win). A row whose `mysekaiMissionStatus` word this
//! reader does not know reads as not achieved, as `TextUtility.StringToEnum`
//! does; the reply's row keeps its stored word so the copy can name it.

use bevy::prelude::*;
use moly_protocol::api::UserMysekaiNormalMissionRewardResponse;
use moly_protocol::rows::{
    UserMysekaiNormalMission, UserMysekaiNormalMissionSheet, SuiteUserSections,
};
use serde_json::Value;

use moly_masters::mission::MissionMasters;

use crate::mission::model::{MissionStatus, UserMission, UserMissionState};

/// `UserMysekaiNormalMissionSheet` and `UserMysekaiNormalMissions` as an
/// arrival carries them; `None` for the table it does not carry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MissionSections {
    pub sheet: Option<UserMysekaiNormalMissionSheet>,
    pub missions: Option<Vec<UserMysekaiNormalMission>>,
}

/// The one stored row of the copy: a mission, its progress, and the word the
/// server stored for its status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionRecord {
    pub mission_id: i32,
    pub progress: i32,
    /// `mysekaiMissionStatus` as stored; an empty word is a row the client
    /// never kept.
    pub status: String,
}

impl MissionRecord {
    /// `MysekaiMissionStatus` of the stored word; a word this reader does not
    /// know reads as not achieved (the screen's own rule).
    pub fn parsed_status(&self) -> MissionStatus {
        MissionStatus::parse(&self.status).unwrap_or(MissionStatus::NotAchieved)
    }
}

/// The client's copy of the mission user data (`UserDataManager`'s
/// `UserMysekaiNormalMissionSheet` and `UserMysekaiNormalMissions`), set by
/// responses and the receive reply, never by the client.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientMission {
    /// `userMysekaiNormalMissionSheet.mysekaiNormalMissionSheetId`; 0 while no
    /// response carried one.
    sheet_id: i32,
    rows: Vec<MissionRecord>,
    /// Applications of the sheet table (0: none yet).
    pub sheet_revision: u64,
    /// Applications of the mission rows.
    pub missions_revision: u64,
}

impl ClientMission {
    /// `GetCurrentMissionSheetId`.
    pub fn sheet_id(&self) -> i32 {
        self.sheet_id
    }

    /// `UserMysekaiNormalMissions` as stored, in the server's order.
    pub fn rows(&self) -> &[MissionRecord] {
        &self.rows
    }

    /// The row of a mission, when the user has one.
    pub fn row(&self, mission_id: i32) -> Option<&MissionRecord> {
        self.rows.iter().find(|row| row.mission_id == mission_id)
    }

    /// The user state the mission model reads ([`UserMissionState`]): the
    /// sheet and each row at its parsed status.
    pub fn state(&self) -> UserMissionState {
        UserMissionState {
            current_sheet: self.sheet_id,
            missions: self
                .rows
                .iter()
                .map(|row| UserMission {
                    mission_id: row.mission_id,
                    progress: row.progress,
                    status: row.parsed_status(),
                })
                .collect(),
        }
    }

    /// `UpdateMysekaiMissionSheet`: a carried sheet replaces the current one.
    pub fn apply_sheet(&mut self, sheet: UserMysekaiNormalMissionSheet) {
        self.sheet_id = sheet.mysekai_normal_mission_sheet_id;
        self.sheet_revision += 1;
    }

    /// `UpdateMysekaiMission`: the carried rows replace the list whole.
    pub fn apply_missions(&mut self, rows: Vec<UserMysekaiNormalMission>) {
        self.rows = rows
            .into_iter()
            .map(|row| MissionRecord {
                mission_id: row.mysekai_normal_mission_id,
                progress: row.progress,
                status: row.mysekai_mission_status,
            })
            .collect();
        self.missions_revision += 1;
    }

    /// Applies whichever of the two tables an arrival carried; returns how
    /// many it carried.
    pub fn apply(&mut self, sections: MissionSections) -> usize {
        let mut carried = 0;
        if let Some(sheet) = sections.sheet {
            self.apply_sheet(sheet);
            carried += 1;
        }
        if let Some(rows) = sections.missions {
            self.apply_missions(rows);
            carried += 1;
        }
        carried
    }

    /// The receive reply's `updatedResources` (`UserDataManager.UpdateAll`):
    /// the carry of its mission tables over the copy, after the reply's own
    /// sections. `false` when neither table is carried.
    pub fn apply_updated_resources(&mut self, updated: &SuiteUserSections) -> bool {
        let missions = updated.user_mysekai_normal_missions.clone();
        self.apply(MissionSections {
            sheet: updated.user_mysekai_normal_mission_sheet,
            missions,
        }) > 0
    }

    /// The receive reply applied once: its reply sections (the resources the
    /// receive produced) and its `updatedResources`, in that order, as
    /// `MysekaiMissionService` applies the answer.
    pub fn apply_reply(&mut self, reply: &UserMysekaiNormalMissionRewardResponse) -> bool {
        self.apply_updated_resources(&reply.updated_resources)
    }

    /// The copy as the control plane's `client` part shows it: the two
    /// document keys and the sheet's name.
    pub fn view(&self, masters: Option<&MissionMasters>) -> Value {
        serde_json::json!({
            "userMysekaiNormalMissionSheet": (self.sheet_id > 0).then(|| serde_json::json!({
                "mysekaiNormalMissionSheetId": self.sheet_id,
            })),
            "userMysekaiNormalMissionSheetName": masters
                .and_then(|masters| masters.sheet(self.sheet_id))
                .map(|sheet| sheet.name.clone()),
            "userMysekaiNormalMissions": self
                .rows
                .iter()
                .map(|row| serde_json::json!({
                    "mysekaiNormalMissionId": row.mission_id,
                    "progress": row.progress,
                    "mysekaiMissionStatus": row.status,
                }))
                .collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i32, progress: i32, status: &str) -> UserMysekaiNormalMission {
        UserMysekaiNormalMission {
            mysekai_normal_mission_id: id,
            progress,
            mysekai_mission_status: status.to_owned(),
        }
    }

    /// A carried table replaces the copy and leaves the other as it is; a
    /// carried list replaces the rows even when it is empty.
    #[test]
    fn a_carried_table_replaces_its_copy_and_the_other_stays() {
        let mut copy = ClientMission::default();
        let carried = copy.apply(MissionSections {
            sheet: Some(UserMysekaiNormalMissionSheet {
                mysekai_normal_mission_sheet_id: 2,
            }),
            missions: Some(vec![row(11, 5, "achieved"), row(12, 0, "not_achieved")]),
        });
        assert_eq!(carried, 2);
        assert_eq!(copy.sheet_id(), 2);
        assert_eq!(copy.rows().len(), 2);
        assert_eq!(copy.row(11).map(|row| row.parsed_status()), Some(MissionStatus::Achieved));
        assert_eq!(copy.apply(MissionSections { sheet: None, missions: Some(Vec::new()) }), 1);
        assert_eq!(copy.sheet_id(), 2);
        assert!(copy.rows().is_empty());
        assert_eq!(copy.sheet_revision, 1);
        assert_eq!(copy.missions_revision, 2);
        assert_eq!(copy.state().current_sheet, 2);
    }

    /// A stored status word this reader does not know reads as not achieved;
    /// the reply's `updatedResources` carries the sheet and the rows after
    /// the reply's own sections.
    #[test]
    fn an_unknown_status_word_reads_not_achieved_and_the_reply_wins() {
        let mut copy = ClientMission::default();
        copy.apply_missions(vec![row(3, 1, "side")]);
        assert_eq!(copy.rows()[0].parsed_status(), MissionStatus::NotAchieved);
        let reply = UserMysekaiNormalMissionRewardResponse {
            obtained_rewards: Vec::new(),
            updated_resources: SuiteUserSections {
                user_mysekai_normal_missions: Some(vec![row(3, 1, "received")]),
                user_mysekai_normal_mission_sheet: Some(UserMysekaiNormalMissionSheet {
                    mysekai_normal_mission_sheet_id: 3,
                }),
                ..SuiteUserSections::default()
            },
        };
        assert!(copy.apply_reply(&reply));
        assert_eq!(copy.sheet_id(), 3);
        assert_eq!(copy.rows()[0].parsed_status(), MissionStatus::Received);
        // A reply that carries neither table leaves the copy as it is.
        let bare = UserMysekaiNormalMissionRewardResponse::default();
        assert!(!copy.apply_reply(&bare));
        let view = copy.view(None);
        assert_eq!(view["userMysekaiNormalMissionSheet"]["mysekaiNormalMissionSheetId"], 3);
        assert_eq!(view["userMysekaiNormalMissions"][0]["progress"], 1);
        assert!(view["userMysekaiNormalMissionSheetName"].is_null());
    }
}
