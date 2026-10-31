//! The mission screen's model: the user's missions read against the mission
//! master tables, the step a receive request leads to, and the missions'
//! rewards.
//!
//! The rules, in the order the screen applies them:
//!
//! - The user is on one sheet (`UserMysekaiNormalMissionSheet`), replaced by
//!   each response that carries one ([`UserMissionState::apply_sheet`]).
//! - A mission's user row (`UserMysekaiNormalMission`) has a progress and a
//!   status. A mission of a sheet earlier than the current one is done: its
//!   status reads received, its progress the mission's `requirement1`,
//!   whether or not the user has a row for it (`GetMissionUserData`). A
//!   mission of the current sheet without a row reads not achieved with no
//!   progress; a word this reader does not know reads not achieved.
//! - The mission rows are replaced whole by each response that carries them
//!   ([`UserMissionState::apply_missions`]); a response without them leaves
//!   the rows as they are.
//! - The sheet's completion mission is complete when the number of received
//!   or achieved normal missions of the current sheet reaches its
//!   `requirement1` (`MysekaiMissionView.CreateMissionList`), never in the
//!   tutorial.
//! - The screen lists the completion mission, then every normal mission that
//!   is not received, in `seq` order. The completion row's gauge counts the
//!   normal missions received or achieved (`GetChildMissionProgressCount`).
//! - The bulk receive takes the completion mission when it is achieved, then
//!   every listed normal mission that is achieved (`GetReceivableMissionAll`);
//!   the bulk button is enabled while that list is not empty.
//! - The user has finished the sheets when the current sheet is the last one
//!   and the user has no mission row left (`IsAllCompleted`).
//! - The home mission panel shows the current sheet's completion mission
//!   while it is not received (`CreateRecommendViewData`).
//!
//! A cell's tap (`OnClickMysekaiMissionCell`) hands the completion mission's
//! completeness or a normal mission's achieved status to the screen; the
//! screen receives an achieved normal mission at once, sends the bulk
//! receive for a complete sheet outside the tutorial, and opens the
//! mission's detail otherwise.

use std::collections::BTreeMap;

use moly_masters::mission::{Mission, MissionMasters, MissionReward, RewardType};

/// `MysekaiMissionStatus`, in the order the source's enum declares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MissionStatus {
    #[default]
    NotAchieved,
    Achieved,
    Received,
}

impl MissionStatus {
    /// The spelling the user data's `mysekaiMissionStatus` column carries.
    pub fn name(self) -> &'static str {
        match self {
            Self::NotAchieved => "not_achieved",
            Self::Achieved => "achieved",
            Self::Received => "received",
        }
    }

    /// The status a stored word names; a word this reader does not know
    /// reads as not achieved (`TextUtility.StringToEnum`'s default).
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "not_achieved" => Some(Self::NotAchieved),
            "achieved" => Some(Self::Achieved),
            "received" => Some(Self::Received),
            _ => None,
        }
    }
}

/// `UserMysekaiNormalMission`: the user's row for one mission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserMission {
    pub mission_id: i32,
    pub progress: i32,
    pub status: MissionStatus,
}

/// The mission part of the user data: the sheet the user is on and the
/// mission rows the server sent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserMissionState {
    /// `UserMysekaiNormalMissionSheet.mysekaiNormalMissionSheetId`.
    pub current_sheet: i32,
    /// One row per mission the user has started or finished.
    pub missions: Vec<UserMission>,
}

impl UserMissionState {
    pub fn row(&self, mission_id: i32) -> Option<&UserMission> {
        self.missions.iter().find(|row| row.mission_id == mission_id)
    }

    /// `UpdateMysekaiMissionSheet`: a carried sheet replaces the current one.
    pub fn apply_sheet(&mut self, sheet_id: Option<i32>) {
        if let Some(sheet_id) = sheet_id {
            self.current_sheet = sheet_id;
        }
    }

    /// `UpdateMysekaiMission`: carried rows replace every row.
    pub fn apply_missions(&mut self, rows: Option<Vec<UserMission>>) {
        if let Some(rows) = rows {
            self.missions = rows;
        }
    }
}

/// A mission as the model reads it: its master row and the user's status and
/// progress for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissionView<'a> {
    pub master: &'a Mission,
    pub status: MissionStatus,
    pub progress: i32,
}

impl<'a> MissionView<'a> {
    /// `requirement1`: the progress the mission completes at.
    pub fn requirement(&self) -> i32 {
        self.master.requirement1
    }

    /// `IsComplete`: achieved or received.
    pub fn is_complete(&self) -> bool {
        matches!(self.status, MissionStatus::Achieved | MissionStatus::Received)
    }

    /// The mission of the current sheet: the user's row, or not achieved at
    /// 0.
    fn current(master: &'a Mission, row: Option<&UserMission>) -> Self {
        match row {
            Some(row) => Self {
                master,
                status: row.status,
                progress: row.progress,
            },
            None => Self {
                master,
                status: MissionStatus::NotAchieved,
                progress: 0,
            },
        }
    }

    /// The mission of an earlier sheet: received, at its requirement.
    fn earlier(master: &'a Mission) -> Self {
        Self {
            master,
            status: MissionStatus::Received,
            progress: master.requirement1,
        }
    }

    /// `MysekaiMissionDetailDialog.SetupProgress`: the gauge reads
    /// `min(current, max)` clamped at 0.
    pub fn gauge_fill(&self) -> f32 {
        gauge_fill(self.progress, self.requirement())
    }

    /// The progress text, `"{current}/{max}"` of the clamped value.
    pub fn progress_text(&self) -> String {
        progress_text(self.progress, self.requirement())
    }
}

/// `UIPartsGauge.Setup`'s fill of a progress (`min(current, max)`, clamped
/// at 0); a requirement of 0 reads empty.
pub fn gauge_fill(current: i32, max: i32) -> f32 {
    if max <= 0 {
        return 0.0;
    }
    current.clamp(0, max) as f32 / max as f32
}

/// The progress text, `"{current}/{max}"` of the clamped progress.
pub fn progress_text(current: i32, max: i32) -> String {
    let current = if max > 0 { current.clamp(0, max) } else { current.max(0) };
    format!("{current}/{max}")
}

/// `MysekaiMissionUtility.GetMissionUserData`: the mission's row as the user
/// sees it. A mission of a sheet earlier than the current one is received at
/// its requirement; one of the current sheet is the user's row, or not
/// achieved at 0. `None` when the mission's sheet is not in the masters.
pub fn mission_state<'a>(
    masters: &'a MissionMasters,
    user: &UserMissionState,
    master: &'a Mission,
) -> Option<MissionView<'a>> {
    let sheet = masters.sheet(master.sheet_id)?;
    match masters.sheet(user.current_sheet) {
        Some(current) if sheet.seq < current.seq => Some(MissionView::earlier(master)),
        _ => Some(MissionView::current(master, user.row(master.id))),
    }
}

/// One row of the mission screen's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MissionRow<'a> {
    /// The sheet's completion mission; `counted` is the number of normal
    /// missions received or achieved, which its gauge reads.
    Complete {
        mission: MissionView<'a>,
        counted: i32,
    },
    /// A normal mission that is not received.
    Mission(MissionView<'a>),
}

impl<'a> MissionRow<'a> {
    pub fn view(&self) -> MissionView<'a> {
        match self {
            Self::Complete { mission, .. } => *mission,
            Self::Mission(mission) => *mission,
        }
    }

    /// The progress the row's gauge reads: the completion row counts its
    /// normal missions, the others their own progress.
    pub fn gauge_progress(&self) -> i32 {
        match self {
            Self::Complete { counted, .. } => *counted,
            Self::Mission(mission) => mission.progress,
        }
    }

    /// The progress text the row shows.
    pub fn progress_text(&self) -> String {
        progress_text(self.gauge_progress(), self.view().requirement())
    }

    /// The row's gauge fill.
    pub fn gauge_fill(&self) -> f32 {
        gauge_fill(self.gauge_progress(), self.view().requirement())
    }
}

/// What a tap on a row does (`ScreenLayerMysekaiMission.OnClickMysekaiMissionCell`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellTap {
    /// Send the receive for this mission.
    Receive(i32),
    /// Send the bulk receive (the complete sheet's own tap).
    BulkReceive,
    /// Open the mission's detail.
    Detail,
}

/// The screen's data for the user's current sheet.
pub struct MissionScreen<'a> {
    /// The user is in the tutorial: the completion mission never reads
    /// complete there (`MysekaiTutorialManager.IsTutorial`).
    tutorial: bool,
    parent: Option<MissionView<'a>>,
    children: Vec<MissionView<'a>>,
}

impl<'a> MissionScreen<'a> {
    /// The current sheet's view; `None` while the sheet the user is on is
    /// not in the masters.
    pub fn new(
        masters: &'a MissionMasters,
        user: &UserMissionState,
        tutorial: bool,
    ) -> Option<Self> {
        masters.sheet(user.current_sheet)?;
        let read = |master: &'a Mission| mission_state(masters, user, master);
        let parent = masters.parent_mission(user.current_sheet).and_then(read);
        let children = masters
            .child_missions(user.current_sheet)
            .into_iter()
            .filter_map(read)
            .collect();
        Some(Self {
            tutorial,
            parent,
            children,
        })
    }

    pub fn parent(&self) -> Option<MissionView<'a>> {
        self.parent
    }

    pub fn children(&self) -> &[MissionView<'a>] {
        &self.children
    }

    /// `GetChildMissionProgressCount`: the normal missions of the current
    /// sheet that are received or achieved.
    pub fn child_progress(&self) -> i32 {
        self.children
            .iter()
            .filter(|child| child.is_complete())
            .count() as i32
    }

    /// The completion mission as the screen shows it: complete when the
    /// counted normal missions reach its requirement, never in the tutorial
    /// (`CreateMissionList`); otherwise the user's own status for it.
    pub fn parent_view(&self) -> Option<MissionView<'a>> {
        let parent = self.parent?;
        let status = if !self.tutorial && self.child_progress() >= parent.requirement() {
            MissionStatus::Achieved
        } else {
            parent.status
        };
        Some(MissionView {
            status,
            progress: self.child_progress(),
            ..parent
        })
    }

    /// The list the screen shows: the completion row (when the sheet has a
    /// completion mission), then every normal mission not received.
    pub fn rows(&self) -> Vec<MissionRow<'a>> {
        let mut rows = Vec::new();
        if let Some(complete) = self.parent_view() {
            rows.push(MissionRow::Complete {
                mission: complete,
                counted: self.child_progress(),
            });
        }
        rows.extend(
            self.children
                .iter()
                .filter(|child| child.status != MissionStatus::Received)
                .map(|child| MissionRow::Mission(*child)),
        );
        rows
    }

    /// `GetReceivableMissionAll`: the completion mission when it is
    /// achieved, then every listed normal mission that is achieved; empty in
    /// the tutorial.
    pub fn receivable(&self) -> Vec<i32> {
        if self.tutorial {
            return Vec::new();
        }
        let mut ids = Vec::new();
        if let Some(parent) = &self.parent {
            if parent.status == MissionStatus::Achieved {
                ids.push(parent.master.id);
            }
        }
        ids.extend(
            self.children
                .iter()
                .filter(|child| child.status == MissionStatus::Achieved)
                .map(|child| child.master.id),
        );
        ids
    }

    /// `UpdateCurrentPageStatus`: the bulk button is enabled while the
    /// receivable list is not empty.
    pub fn bulk_enabled(&self) -> bool {
        !self.receivable().is_empty()
    }

    /// The screen's tap on a row.
    pub fn tap(&self, row: &MissionRow<'_>) -> CellTap {
        match row {
            MissionRow::Mission(mission) if mission.status == MissionStatus::Achieved => {
                CellTap::Receive(mission.master.id)
            }
            MissionRow::Complete { .. } if self.parent_complete() && !self.tutorial => {
                CellTap::BulkReceive
            }
            _ => CellTap::Detail,
        }
    }

    /// The completion mission is complete: the user's completion row is
    /// achieved.
    pub fn parent_complete(&self) -> bool {
        self.parent_view().is_some_and(|parent| parent.is_complete())
    }
}

/// The completion mission of a sheet (`GetParentMission`).
pub fn parent_mission<'a>(masters: &'a MissionMasters, sheet_id: i32) -> Option<&'a Mission> {
    masters.parent_mission(sheet_id)
}

/// The normal missions of a sheet in `seq` order (`GetChildMissions`).
pub fn child_missions<'a>(masters: &'a MissionMasters, sheet_id: i32) -> Vec<&'a Mission> {
    masters.child_missions(sheet_id)
}

/// `MysekaiMissionUtility.IsAllCompleted`: the current sheet is the last one
/// and the user has no mission row left.
pub fn is_all_completed(masters: &MissionMasters, user: &UserMissionState) -> bool {
    match masters.last_sheet() {
        Some(last) => last.id == user.current_sheet && user.missions.is_empty(),
        None => false,
    }
}

/// `MysekaiMissionUtility.IsChildMissionAllAchieved`: no normal mission of
/// the current sheet is not achieved (an empty sheet counts).
pub fn is_child_mission_all_achieved(masters: &MissionMasters, user: &UserMissionState) -> bool {
    match MissionScreen::new(masters, user, false) {
        Some(screen) => screen
            .children()
            .iter()
            .all(|child| child.status != MissionStatus::NotAchieved),
        None => false,
    }
}

/// `MysekaiMissionUtility.IsChildMissionReceiveAvailable`: a normal mission
/// of the current sheet is achieved.
pub fn is_child_mission_receive_available(
    masters: &MissionMasters,
    user: &UserMissionState,
) -> bool {
    match MissionScreen::new(masters, user, false) {
        Some(screen) => screen
            .children()
            .iter()
            .any(|child| child.status == MissionStatus::Achieved),
        None => false,
    }
}

/// The home mission panel's content (`MysekaiMissionHomePanel.UpdateContent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelView<'a> {
    /// The current sheet's completion mission, at its counted progress.
    pub mission: MissionView<'a>,
    /// `IsChildMissionReceiveAvailable`: a normal mission is achieved.
    pub child_receive_available: bool,
}

impl PanelView<'_> {
    pub fn is_complete(&self) -> bool {
        self.mission.progress >= self.mission.requirement()
    }

    pub fn progress_text(&self) -> String {
        self.mission.progress_text()
    }

    pub fn gauge_fill(&self) -> f32 {
        self.mission.gauge_fill()
    }
}

/// `MysekaiMissionUtility.CreateRecommendViewData`: the current sheet's
/// completion mission while it is not received, at the counted progress;
/// `None` when the user's completion row reads received or the sheet has no
/// completion mission.
pub fn panel_view<'a>(
    masters: &'a MissionMasters,
    user: &UserMissionState,
) -> Option<PanelView<'a>> {
    let screen = MissionScreen::new(masters, user, false)?;
    let parent = screen.parent()?;
    if parent.status == MissionStatus::Received {
        return None;
    }
    Some(PanelView {
        mission: MissionView {
            progress: screen.child_progress(),
            ..parent
        },
        child_receive_available: screen
            .children()
            .iter()
            .any(|child| child.status == MissionStatus::Achieved),
    })
}

// ---------------------------------------------------------------------------
// The receive prompt
// ---------------------------------------------------------------------------

/// `MysekaiMissionService`: the mission ids the last request carried and the
/// sheet it named, kept while the request is out. `IsCompleteSheet` tests the
/// request against the completion mission of the sheet it names
/// ([`ReceiveRequest::is_complete_sheet`]); the screen reads it again after
/// the reply's resources are applied ([`after_receive`]), because the reply
/// may carry another sheet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReceiveRequest {
    /// `_missionSheetId`.
    pub sheet_id: i32,
    /// `UserMysekaiNormalMissionRewardRequest.mysekaiNormalMissionIds`.
    pub mission_ids: Vec<i32>,
}

/// What the screen does once the reply's resources are applied
/// (`OnFinishReceiveDialog`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterReceive {
    /// `BackUIScreen`.
    Back,
    /// `StartNextMission`: turn the page and show the next sheet's start.
    NextMission,
    /// Refresh the current page and receive the completion mission when it
    /// is achieved (`ShowMissionComplete`, then another receive).
    Stay,
}

impl ReceiveRequest {
    /// `IsCompleteSheet`: the request carries the completion mission of the
    /// sheet it names.
    pub fn is_complete_sheet(&self, masters: &MissionMasters) -> bool {
        masters
            .parent_mission(self.sheet_id)
            .is_some_and(|parent| self.mission_ids.contains(&parent.id))
    }
}

/// The screen's next step once the reply's resources are applied: only a
/// request that carried the sheet's completion mission moves the screen;
/// `return_pending` is `MYSEKAI_MISSION_RETURN` holding `FinishExpand`.
pub fn after_receive(
    masters: &MissionMasters,
    user: &UserMissionState,
    request: &ReceiveRequest,
    return_pending: bool,
) -> AfterReceive {
    if !request.is_complete_sheet(masters) {
        return AfterReceive::Stay;
    }
    if is_all_completed(masters, user) || return_pending {
        AfterReceive::Back
    } else {
        AfterReceive::NextMission
    }
}

// ---------------------------------------------------------------------------
// Rewards
// ---------------------------------------------------------------------------

/// A `resourceBoxes` row as a reward thumbnail reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceBox {
    pub id: i32,
    /// `resourceBoxType` (`expand`, `list`, or another).
    pub box_type: String,
    /// `details` in master order.
    pub details: Vec<BoxDetail>,
}

impl ResourceBox {
    /// `MasterResourceBox.IsExistDetails`.
    pub fn has_details(&self) -> bool {
        !self.details.is_empty()
    }

    pub fn is_expand(&self) -> bool {
        self.box_type == "expand"
    }

    pub fn is_list(&self) -> bool {
        self.box_type == "list"
    }
}

/// One `resourceBoxes.details` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxDetail {
    pub resource_type: String,
    pub resource_id: Option<i32>,
    pub quantity: i32,
    pub seq: i32,
}

/// The boxes a reward resolves against; a runtime reader seats it from the
/// `resourceBoxes` table.
pub type BoxIndex = BTreeMap<i32, ResourceBox>;

/// `MysekaiMissionRewardDetailData.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Channel {
    UserResource,
    ResourceBox,
    PlayerStatus,
}

/// One detail row of a mission's rewards (`MysekaiMissionRewardDetailData`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewardDetail<'a> {
    /// One resource of an expanding box, or a list box's first resource.
    Resource(&'a BoxDetail),
    /// A box with another type, drawn whole.
    Box(&'a ResourceBox),
    /// A player-status reward.
    PlayerStatus(&'a MissionReward),
}

impl<'a> RewardDetail<'a> {
    /// `MysekaiMissionRewardDetailData.RewardType`.
    pub fn kind(&self) -> Channel {
        match self {
            Self::Resource(_) => Channel::UserResource,
            Self::Box(_) => Channel::ResourceBox,
            Self::PlayerStatus(_) => Channel::PlayerStatus,
        }
    }

    /// The player-status reward's wording key (`GetPlayerStatusWord`).
    pub fn status_word_key(&self) -> Option<&'static str> {
        match self {
            Self::PlayerStatus(row) => row.reward_type.status_word_key(),
            _ => None,
        }
    }
}

/// A reward row of `resource_box` type names no box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissingBox(pub i32);

impl std::fmt::Display for MissingBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the reward names resource box {} absent from resourceBoxes", self.0)
    }
}

/// What one reward row draws as the cell's and the sheet list's single
/// thumbnail (`CreateMissionRewardTopThumbnail`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopThumbnail<'a> {
    /// A box: its first detail when it expands, the box itself otherwise;
    /// `None` when it has no details and does not expand.
    Box {
        box_row: &'a ResourceBox,
        expanded_first: Option<&'a BoxDetail>,
    },
    /// A player-status thumbnail; a rank-experience reward carries the
    /// quantity of the rank row its `externalId` names.
    Status {
        row: &'a MissionReward,
        quantity: Option<i32>,
    },
}

impl TopThumbnail<'_> {
    pub fn is_status(&self) -> bool {
        matches!(self, Self::Status { .. })
    }
}

/// `MysekaiMissionUtility.ConvertRewardDataToDetailData`: every reward row
/// in `seq` order; an expanding box yields its details in `seq` order, a
/// list box its first detail, another box itself, a status reward one status
/// row.
pub fn reward_details<'a>(
    boxes: &'a BoxIndex,
    rewards: &[&'a MissionReward],
) -> Result<Vec<RewardDetail<'a>>, MissingBox> {
    let mut out = Vec::new();
    for reward in rewards {
        if !reward.reward_type.is_box() {
            out.push(RewardDetail::PlayerStatus(reward));
            continue;
        }
        let box_row = boxes.get(&reward.external_id).ok_or(MissingBox(reward.external_id))?;
        if box_row.is_expand() {
            let mut details: Vec<&BoxDetail> = box_row.details.iter().collect();
            details.sort_by_key(|detail| detail.seq);
            out.extend(details.into_iter().map(RewardDetail::Resource));
        } else if box_row.is_list() {
            if let Some(first) = box_row.details.first() {
                out.push(RewardDetail::Resource(first));
            }
        } else {
            out.push(RewardDetail::Box(box_row));
        }
    }
    Ok(out)
}

/// `MysekaiMissionUtility.CreateMissionRewardTopThumbnail`.
pub fn top_thumbnail<'a>(
    masters: &'a MissionMasters,
    boxes: &'a BoxIndex,
    reward: &'a MissionReward,
) -> Result<TopThumbnail<'a>, MissingBox> {
    match reward.reward_type {
        RewardType::ResourceBox => {
            let box_row = boxes.get(&reward.external_id).ok_or(MissingBox(reward.external_id))?;
            let expanded_first = box_row
                .is_expand()
                .then(|| box_row.details.first())
                .flatten();
            Ok(TopThumbnail::Box {
                box_row,
                expanded_first,
            })
        }
        RewardType::RankObtainedExp => Ok(TopThumbnail::Status {
            row: reward,
            quantity: masters.rank_exp(reward.external_id).map(|row| row.quantity),
        }),
        _ => Ok(TopThumbnail::Status {
            row: reward,
            quantity: None,
        }),
    }
}

/// `WORD_OTHER_KIND_COUNT`: the "other kinds" count of a cell, shown when a
/// mission's detail rows number two or more.
pub fn other_kind_count(detail_rows: usize) -> Option<i32> {
    (detail_rows >= 2).then(|| detail_rows as i32 - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_masters::mission::{MissionCategory, MissionSheet, RankExpRow};

    /// A sheet whose `seq` is its id.
    fn sheet(id: i32) -> MissionSheet {
        MissionSheet {
            id,
            seq: id,
            name: format!("sheet {id}"),
        }
    }

    /// A mission; `seq` 0 marks the sheet's completion mission.
    fn mission(id: i32, sheet_id: i32, seq: i32, requirement: i32) -> Mission {
        Mission {
            id,
            sheet_id,
            seq,
            sentence: String::new(),
            category: if seq == 0 {
                MissionCategory::Complete
            } else {
                MissionCategory::Normal
            },
            requirement1: requirement,
        }
    }

    /// Sheet 1: completion 10 over 2, normal 11 (5) and 12 (1). Sheet 2:
    /// completion 20 over 1, normal 21 (3). Sheet 3: completion 30 over 1 and
    /// no normal mission.
    fn masters() -> MissionMasters {
        MissionMasters {
            sheets: vec![sheet(1), sheet(2), sheet(3)],
            missions: vec![
                mission(30, 3, 0, 1),
                mission(12, 1, 2, 1),
                mission(10, 1, 0, 2),
                mission(21, 2, 1, 3),
                mission(11, 1, 1, 5),
                mission(20, 2, 0, 1),
            ],
            ..MissionMasters::empty()
        }
    }

    fn row(mission_id: i32, progress: i32, status: MissionStatus) -> UserMission {
        UserMission {
            mission_id,
            progress,
            status,
        }
    }

    fn on_sheet(sheet: i32, rows: Vec<UserMission>) -> UserMissionState {
        UserMissionState {
            current_sheet: sheet,
            missions: rows,
        }
    }

    #[test]
    fn a_mission_of_an_earlier_sheet_reads_received_at_its_requirement() {
        let masters = masters();
        let user = on_sheet(2, Vec::new());
        let old = mission_state(&masters, &user, masters.mission(11).unwrap()).unwrap();
        assert_eq!(old.status, MissionStatus::Received);
        assert_eq!(old.progress, 5);
        assert!(old.is_complete());
        let current = mission_state(&masters, &user, masters.mission(21).unwrap()).unwrap();
        assert_eq!(current.status, MissionStatus::NotAchieved);
        assert_eq!(current.progress, 0);
    }

    #[test]
    fn the_sheets_are_read_by_their_sequence() {
        let masters = masters();
        assert_eq!(masters.last_sheet().unwrap().id, 3);
        let ids: Vec<i32> = masters.child_missions(1).iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![11, 12]);
        assert_eq!(parent_mission(&masters, 2).unwrap().id, 20);
    }

    #[test]
    fn the_completion_mission_completes_at_its_counted_normal_missions() {
        let masters = masters();
        let one = on_sheet(1, vec![row(11, 5, MissionStatus::Achieved)]);
        let screen = MissionScreen::new(&masters, &one, false).unwrap();
        assert_eq!(screen.child_progress(), 1);
        assert!(!screen.parent_complete());
        let both = on_sheet(
            1,
            vec![
                row(11, 5, MissionStatus::Received),
                row(12, 1, MissionStatus::Achieved),
            ],
        );
        let screen = MissionScreen::new(&masters, &both, false).unwrap();
        assert_eq!(screen.child_progress(), 2);
        assert!(screen.parent_complete());
        assert_eq!(screen.parent_view().unwrap().status, MissionStatus::Achieved);
    }

    #[test]
    fn the_tutorial_never_completes_the_sheet() {
        let masters = masters();
        let both = on_sheet(
            1,
            vec![
                row(11, 5, MissionStatus::Received),
                row(12, 1, MissionStatus::Received),
            ],
        );
        let screen = MissionScreen::new(&masters, &both, true).unwrap();
        assert_eq!(screen.child_progress(), 2);
        assert!(!screen.parent_complete());
        assert!(screen.receivable().is_empty());
        assert!(!screen.bulk_enabled());
    }

    #[test]
    fn the_list_is_the_completion_row_then_the_unreceived_missions() {
        let masters = masters();
        let user = on_sheet(
            1,
            vec![row(11, 5, MissionStatus::Achieved), row(12, 0, MissionStatus::NotAchieved)],
        );
        let screen = MissionScreen::new(&masters, &user, false).unwrap();
        let rows = screen.rows();
        assert_eq!(rows.len(), 3);
        match &rows[0] {
            MissionRow::Complete { mission, counted } => {
                assert_eq!(mission.master.id, 10);
                assert_eq!(*counted, 1);
                assert_eq!(rows[0].progress_text(), "1/2");
                assert_eq!(rows[0].gauge_fill(), 0.5);
            }
            other => panic!("the first row is the completion row, not {other:?}"),
        }
        assert_eq!(rows[1].view().master.id, 11);
        assert_eq!(rows[1].progress_text(), "5/5");
        assert_eq!(rows[2].view().master.id, 12);
        let after = on_sheet(
            1,
            vec![row(11, 5, MissionStatus::Received), row(12, 0, MissionStatus::NotAchieved)],
        );
        let screen = MissionScreen::new(&masters, &after, false).unwrap();
        let rows = screen.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].view().master.id, 12);
    }

    #[test]
    fn the_receivable_list_takes_the_completion_mission_then_achieved_rows() {
        let masters = masters();
        let user = on_sheet(
            1,
            vec![row(11, 5, MissionStatus::Achieved), row(12, 0, MissionStatus::NotAchieved)],
        );
        let screen = MissionScreen::new(&masters, &user, false).unwrap();
        assert_eq!(screen.receivable(), vec![11]);
        assert!(screen.bulk_enabled());
        let complete = on_sheet(
            1,
            vec![
                row(10, 0, MissionStatus::Achieved),
                row(11, 5, MissionStatus::Achieved),
                row(12, 1, MissionStatus::Achieved),
            ],
        );
        let screen = MissionScreen::new(&masters, &complete, false).unwrap();
        assert_eq!(screen.receivable(), vec![10, 11, 12]);
        let none = on_sheet(1, vec![row(11, 0, MissionStatus::NotAchieved)]);
        let screen = MissionScreen::new(&masters, &none, false).unwrap();
        assert!(screen.receivable().is_empty());
        assert!(!screen.bulk_enabled());
    }

    #[test]
    fn a_tap_receives_an_achieved_mission_and_opens_the_detail_otherwise() {
        let masters = masters();
        let user = on_sheet(
            1,
            vec![row(11, 5, MissionStatus::Achieved), row(12, 0, MissionStatus::NotAchieved)],
        );
        let screen = MissionScreen::new(&masters, &user, false).unwrap();
        let rows = screen.rows();
        assert_eq!(screen.tap(&rows[0]), CellTap::Detail);
        assert_eq!(screen.tap(&rows[1]), CellTap::Receive(11));
        assert_eq!(screen.tap(&rows[2]), CellTap::Detail);
        // The completion cell's second argument is the completion mission's
        // own status, not the counted normal missions: an achieved
        // completion mission sends the bulk receive, even with a normal
        // mission received and another one not achieved.
        let sheet_done = on_sheet(
            1,
            vec![
                row(10, 0, MissionStatus::Achieved),
                row(11, 5, MissionStatus::Received),
                row(12, 0, MissionStatus::NotAchieved),
            ],
        );
        let screen = MissionScreen::new(&masters, &sheet_done, false).unwrap();
        assert_eq!(screen.tap(&screen.rows()[0]), CellTap::BulkReceive);
        // A received completion mission is complete too, so its cell also
        // sends the bulk receive; only a completion mission that is neither
        // achieved nor received opens the detail.
        let sheet_received = on_sheet(
            1,
            vec![row(10, 0, MissionStatus::Received), row(12, 0, MissionStatus::NotAchieved)],
        );
        let screen = MissionScreen::new(&masters, &sheet_received, false).unwrap();
        assert_eq!(screen.tap(&screen.rows()[0]), CellTap::BulkReceive);
        let sheet_pending = on_sheet(
            1,
            vec![row(10, 0, MissionStatus::NotAchieved), row(12, 0, MissionStatus::NotAchieved)],
        );
        let screen = MissionScreen::new(&masters, &sheet_pending, false).unwrap();
        assert_eq!(screen.tap(&screen.rows()[0]), CellTap::Detail);
    }

    #[test]
    fn the_gauge_reads_the_clamped_progress() {
        assert_eq!(gauge_fill(5, 10), 0.5);
        assert_eq!(gauge_fill(12, 10), 1.0);
        assert_eq!(gauge_fill(-1, 10), 0.0);
        assert_eq!(gauge_fill(1, 0), 0.0);
        assert_eq!(progress_text(12, 10), "10/10");
        let masters = masters();
        let user = on_sheet(1, vec![row(11, 7, MissionStatus::NotAchieved)]);
        let view = mission_state(&masters, &user, masters.mission(11).unwrap()).unwrap();
        assert_eq!(view.progress_text(), "5/5");
        assert_eq!(view.gauge_fill(), 1.0);
    }

    #[test]
    fn all_completed_needs_the_last_sheet_and_no_rows_left() {
        let masters = masters();
        assert!(!is_all_completed(&masters, &on_sheet(2, Vec::new())));
        assert!(is_all_completed(&masters, &on_sheet(3, Vec::new())));
        assert!(!is_all_completed(
            &masters,
            &on_sheet(3, vec![row(30, 0, MissionStatus::Received)])
        ));
        assert!(!is_all_completed(&MissionMasters::empty(), &on_sheet(1, Vec::new())));
    }

    #[test]
    fn the_sheet_without_normal_missions_completes_at_zero_requirement() {
        let masters = MissionMasters {
            sheets: vec![sheet(1)],
            missions: vec![mission(10, 1, 0, 0)],
            ..MissionMasters::empty()
        };
        let screen = MissionScreen::new(&masters, &on_sheet(1, Vec::new()), false).unwrap();
        assert_eq!(screen.child_progress(), 0);
        assert!(screen.parent_complete());
        assert_eq!(screen.rows().len(), 1);
    }

    #[test]
    fn a_state_applies_the_carried_sheet_and_rows_whole() {
        let mut state = on_sheet(1, vec![row(11, 1, MissionStatus::NotAchieved)]);
        state.apply_sheet(Some(2));
        state.apply_sheet(None);
        assert_eq!(state.current_sheet, 2);
        state.apply_missions(Some(vec![row(21, 3, MissionStatus::Achieved)]));
        assert_eq!(state.missions, vec![row(21, 3, MissionStatus::Achieved)]);
        state.apply_missions(None);
        assert_eq!(state.missions.len(), 1);
    }

    #[test]
    fn the_return_prompt_reads_the_request_before_and_the_rows_after() {
        let masters = masters();
        let mut request = ReceiveRequest {
            sheet_id: 1,
            mission_ids: vec![10],
        };
        assert!(request.is_complete_sheet(&masters));
        let after = on_sheet(2, Vec::new());
        assert_eq!(after_receive(&masters, &after, &request, false), AfterReceive::NextMission);
        let last = on_sheet(3, Vec::new());
        assert_eq!(after_receive(&masters, &last, &request, false), AfterReceive::Back);
        assert_eq!(after_receive(&masters, &after, &request, true), AfterReceive::Back);
        request.mission_ids = vec![11];
        assert!(!request.is_complete_sheet(&masters));
        assert_eq!(after_receive(&masters, &after, &request, false), AfterReceive::Stay);
    }

    #[test]
    fn the_panel_shows_the_unreceived_completion_mission_at_its_counted_progress() {
        let masters = masters();
        let user = on_sheet(1, vec![row(11, 5, MissionStatus::Achieved)]);
        let panel = panel_view(&masters, &user).unwrap();
        assert_eq!(panel.mission.master.id, 10);
        assert_eq!(panel.progress_text(), "1/2");
        assert!(!panel.is_complete());
        assert!(panel.child_receive_available);
        let done = on_sheet(2, vec![row(20, 0, MissionStatus::Received)]);
        assert!(panel_view(&masters, &done).is_none());
        let next = on_sheet(2, Vec::new());
        assert_eq!(panel_view(&masters, &next).unwrap().mission.master.id, 20);
        assert!(is_child_mission_receive_available(&masters, &user));
        let never = on_sheet(1, Vec::new());
        assert!(!is_child_mission_receive_available(&masters, &never));
        assert!(!is_child_mission_all_achieved(&masters, &never));
        assert!(is_child_mission_all_achieved(
            &masters,
            &on_sheet(
                1,
                vec![row(11, 5, MissionStatus::Received), row(12, 1, MissionStatus::Achieved)]
            )
        ));
    }

    #[test]
    fn the_model_refuses_a_sheet_it_has_no_master_for() {
        assert!(MissionScreen::new(&MissionMasters::empty(), &on_sheet(1, Vec::new()), false).is_none());
        assert!(panel_view(&MissionMasters::empty(), &on_sheet(1, Vec::new())).is_none());
    }

    fn detail(kind: &str, quantity: i32, seq: i32) -> BoxDetail {
        BoxDetail {
            resource_type: kind.to_owned(),
            resource_id: None,
            quantity,
            seq,
        }
    }

    fn box_row(id: i32, box_type: &str, details: Vec<BoxDetail>) -> ResourceBox {
        ResourceBox {
            id,
            box_type: box_type.to_owned(),
            details,
        }
    }

    fn reward(id: i32, seq: i32, kind: RewardType, external: i32) -> MissionReward {
        MissionReward {
            id,
            mission_id: 100,
            seq,
            reward_type: kind,
            external_id: external,
        }
    }

    #[test]
    fn an_expanding_box_yields_every_detail_in_seq_order() {
        let boxes = BTreeMap::from([(
            100,
            box_row(
                100,
                "expand",
                vec![detail("jewel", 10, 2), detail("material", 3, 1)],
            ),
        )]);
        let boxed = reward(1, 1, RewardType::ResourceBox, 100);
        let details = reward_details(&boxes, &[&boxed]).unwrap();
        assert_eq!(details.len(), 2);
        assert!(details.iter().all(|row| row.kind() == Channel::UserResource));
        match details[0] {
            RewardDetail::Resource(first) => assert_eq!(first.resource_type, "material"),
            other => panic!("the first detail is a resource, not {other:?}"),
        }
    }

    #[test]
    fn a_list_box_yields_its_first_detail_and_another_box_stays_whole() {
        let boxes = BTreeMap::from([
            (
                100,
                box_row(
                    100,
                    "list",
                    vec![detail("jewel", 5000, 1), detail("gacha_ticket", 1, 2)],
                ),
            ),
            (200, box_row(200, "costume_3d", vec![detail("costume_3d", 1, 1)])),
        ]);
        let listed = reward(1, 1, RewardType::ResourceBox, 100);
        let whole = reward(2, 2, RewardType::ResourceBox, 200);
        let details = reward_details(&boxes, &[&listed, &whole]).unwrap();
        assert_eq!(details.len(), 2);
        match details[0] {
            RewardDetail::Resource(first) => assert_eq!(first.quantity, 5000),
            other => panic!("a list box yields its first detail, not {other:?}"),
        }
        match details[1] {
            RewardDetail::Box(row) => assert_eq!(row.id, 200),
            other => panic!("another box stays whole, not {other:?}"),
        }
    }

    #[test]
    fn a_status_reward_is_one_status_row_with_its_word() {
        let boxes = BTreeMap::new();
        let rank = reward(1, 1, RewardType::RankObtainedExp, 1);
        let possession = reward(2, 2, RewardType::MaterialPossession, 0);
        let details = reward_details(&boxes, &[&rank, &possession]).unwrap();
        assert_eq!(details.len(), 2);
        assert_eq!(details[0].kind(), Channel::PlayerStatus);
        assert_eq!(details[0].status_word_key(), Some("WORD_SEKAI_RANK_EXP"));
        assert_eq!(
            details[1].status_word_key(),
            Some("WORD_MATERIAL_POSSESSION")
        );
    }

    #[test]
    fn a_top_thumbnail_names_the_box_art_and_the_rank_quantity() {
        let boxes = BTreeMap::from([(
            100,
            box_row(100, "expand", vec![detail("jewel", 10, 1)]),
        )]);
        let masters = MissionMasters {
            rank_exps: vec![RankExpRow { id: 1, quantity: 7000 }],
            ..MissionMasters::empty()
        };
        let boxed = reward(1, 1, RewardType::ResourceBox, 100);
        match top_thumbnail(&masters, &boxes, &boxed).unwrap() {
            TopThumbnail::Box { expanded_first, .. } => {
                assert_eq!(expanded_first.unwrap().quantity, 10)
            }
            other => panic!("a box reward draws the box, not {other:?}"),
        }
        let rank = reward(2, 1, RewardType::RankObtainedExp, 1);
        match top_thumbnail(&masters, &boxes, &rank).unwrap() {
            TopThumbnail::Status { quantity, .. } => assert_eq!(quantity, Some(7000)),
            other => panic!("a rank reward draws the status, not {other:?}"),
        }
        let fixture = reward(3, 1, RewardType::FixturePossession, 0);
        match top_thumbnail(&masters, &boxes, &fixture).unwrap() {
            TopThumbnail::Status { quantity, .. } => assert_eq!(quantity, None),
            other => panic!("a possession reward draws the status, not {other:?}"),
        }
    }

    #[test]
    fn a_box_reward_needs_its_box_row() {
        let boxes = BTreeMap::new();
        let missing = reward(1, 1, RewardType::ResourceBox, 100);
        assert_eq!(reward_details(&boxes, &[&missing]), Err(MissingBox(100)));
        let masters = MissionMasters::empty();
        assert_eq!(top_thumbnail(&masters, &boxes, &missing), Err(MissingBox(100)));
    }

    #[test]
    fn the_other_kinds_count_all_but_the_first() {
        assert_eq!(other_kind_count(0), None);
        assert_eq!(other_kind_count(1), None);
        assert_eq!(other_kind_count(2), Some(1));
        assert_eq!(other_kind_count(4), Some(3));
    }

    #[test]
    fn a_status_word_and_a_stored_word_read_as_declared() {
        assert_eq!(MissionStatus::parse("received"), Some(MissionStatus::Received));
        assert_eq!(MissionStatus::parse("side"), None);
        assert_eq!(MissionStatus::NotAchieved.name(), "not_achieved");
        assert_eq!(MissionStatus::default(), MissionStatus::NotAchieved);
    }
}
