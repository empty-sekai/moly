//! The MySekai mission master tables (the mission sheets, their missions and
//! the missions' reward rows) and the pure rules the mission screen's model
//! reads them with.
//!
//! A sheet (`mysekaiNormalMissionSheets`) is shown in `seq` order. Its
//! missions (`mysekaiNormalMissions`) split into one sheet-completion mission
//! of category `complete` and the missions of category `normal`; the screen
//! shows the completion mission above the normal missions ordered by `seq`.
//! The completion mission's `requirement1` is the number of normal missions
//! the sheet expects, and each mission's reward rows
//! (`mysekaiNormalMissionRewards`) are ordered by `seq`.
//!
//! Every table is matched to the others by id, never by array position: the
//! sheets table does not follow the missions table's order.

use serde_json::Value;

/// `mysekaiNormalMissionCategory` of a mission row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MissionCategory {
    /// `normal`: one of the missions the screen lists.
    Normal,
    /// `complete`: the sheet's completion mission (`complete_sheet`).
    Complete,
}

impl MissionCategory {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "normal" => Some(Self::Normal),
            "complete" => Some(Self::Complete),
            _ => None,
        }
    }
}

/// `MysekaiNormalMissionRewardType`: what a mission's reward row gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RewardType {
    /// `resource_box`: `externalId` names a `resourceBoxes` row.
    ResourceBox,
    /// `mysekai_material_possession`: the material-possession capacity.
    MaterialPossession,
    /// `mysekai_fixture_possession`: the fixture-possession capacity.
    FixturePossession,
    /// `mysekai_convert_fixture_level`: the conversion machine's level.
    ConvertFixtureLevel,
    /// `mysekai_rank_obtained_exp`: MySekai rank experience; `externalId`
    /// names a `mysekaiNormalMissionRewardMysekaiRankObtainedExps` row.
    RankObtainedExp,
    /// `mysekai_release_element`: a released element.
    ReleaseElement,
}

impl RewardType {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "resource_box" => Some(Self::ResourceBox),
            "mysekai_material_possession" => Some(Self::MaterialPossession),
            "mysekai_fixture_possession" => Some(Self::FixturePossession),
            "mysekai_convert_fixture_level" => Some(Self::ConvertFixtureLevel),
            "mysekai_rank_obtained_exp" => Some(Self::RankObtainedExp),
            "mysekai_release_element" => Some(Self::ReleaseElement),
            _ => None,
        }
    }

    /// A box reward (`CreateMissionRewardTopThumbnail`: the box's own art, or
    /// its first detail's when the box expands).
    pub fn is_box(self) -> bool {
        self == Self::ResourceBox
    }

    /// `MysekaiResourceUtility.CreatePlayerStatusThumbnail` draws every other
    /// type as one status thumbnail.
    pub fn is_status(self) -> bool {
        !self.is_box()
    }

    /// `MysekaiResourceUtility.GetPlayerStatusWord`: the wording key of a
    /// status reward's label (`None` for `mysekai_release_element`, which the
    /// method has no case for).
    pub fn status_word_key(self) -> Option<&'static str> {
        match self {
            Self::ResourceBox => None,
            Self::MaterialPossession => Some("WORD_MATERIAL_POSSESSION"),
            Self::FixturePossession => Some("WORD_FIXTURE_POSSESSION"),
            Self::ConvertFixtureLevel => Some("WORD_TRADE_MACHINE_LEVEL_UP"),
            Self::RankObtainedExp => Some("WORD_SEKAI_RANK_EXP"),
            Self::ReleaseElement => None,
        }
    }
}

/// `MasterMysekaiNormalMissionSheet`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionSheet {
    pub id: i32,
    pub seq: i32,
    pub name: String,
}

/// The `MasterMysekaiNormalMission` fields the mission screen reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mission {
    pub id: i32,
    pub sheet_id: i32,
    pub seq: i32,
    pub sentence: String,
    pub category: MissionCategory,
    /// `requirement1`: the progress at which a mission is achieved, and the
    /// number of normal missions a completion mission expects.
    pub requirement1: i32,
}

/// `MasterMysekaiNormalMissionReward`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissionReward {
    pub id: i32,
    pub mission_id: i32,
    pub seq: i32,
    pub reward_type: RewardType,
    pub external_id: i32,
}

/// `MasterMysekaiNormalMissionRewardMysekaiRankObtainedExp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RankExpRow {
    pub id: i32,
    pub quantity: i32,
}

/// The mission masters: `mysekaiNormalMissionSheets`, `mysekaiNormalMissions`,
/// `mysekaiNormalMissionRewards` and
/// `mysekaiNormalMissionRewardMysekaiRankObtainedExps`, each in master order.
#[derive(bevy::prelude::Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct MissionMasters {
    pub sheets: Vec<MissionSheet>,
    pub missions: Vec<Mission>,
    pub rewards: Vec<MissionReward>,
    pub rank_exps: Vec<RankExpRow>,
}

impl MissionMasters {
    /// A table of zero rows as its readers apply them: no sheet, mission or
    /// reward.
    pub fn empty() -> Self {
        Self::default()
    }

    /// The sheets in `seq` order, ties in master order.
    pub fn sheets_by_seq(&self) -> Vec<&MissionSheet> {
        let mut sheets: Vec<&MissionSheet> = self.sheets.iter().collect();
        sheets.sort_by_key(|sheet| sheet.seq);
        sheets
    }

    /// `MasterDataManager.GetMasterMysekaiNormalMissionSheet(id)`.
    pub fn sheet(&self, id: i32) -> Option<&MissionSheet> {
        self.sheets.iter().find(|sheet| sheet.id == id)
    }

    /// `MysekaiMissionUtility.GetLastMissionSheet`: the sheet of the highest
    /// `seq`.
    pub fn last_sheet(&self) -> Option<&MissionSheet> {
        self.sheets_by_seq().into_iter().last()
    }

    /// The sheet of the lowest `seq`: the sheet a user without one starts on
    /// (`MysekaiMissionUtility.GetCurrentMissionSheetId` reads the user's
    /// sheet, which a new user has none of).
    pub fn first_sheet(&self) -> Option<&MissionSheet> {
        self.sheets_by_seq().into_iter().next()
    }

    /// The sheet after `id` in `seq` order, ties in master order: the sheet a
    /// receive advances to once the sheet it completes is done and is not the
    /// last one (`MysekaiMissionUtility.GetLastMissionSheet` bounds it).
    pub fn next_sheet(&self, id: i32) -> Option<&MissionSheet> {
        let sheets = self.sheets_by_seq();
        let at = sheets.iter().position(|sheet| sheet.id == id)?;
        sheets.get(at + 1).copied()
    }

    /// Every mission of a sheet, its completion mission and its normal
    /// missions, in `seq` order, ties in master order.
    pub fn sheet_missions(&self, sheet_id: i32) -> Vec<&Mission> {
        let mut missions: Vec<&Mission> = self
            .missions
            .iter()
            .filter(|mission| mission.sheet_id == sheet_id)
            .collect();
        missions.sort_by_key(|mission| mission.seq);
        missions
    }

    /// `MysekaiMissionUtility.GetParentMission`: the sheet's completion
    /// mission.
    pub fn parent_mission(&self, sheet_id: i32) -> Option<&Mission> {
        self.missions.iter().find(|mission| {
            mission.sheet_id == sheet_id && mission.category == MissionCategory::Complete
        })
    }

    /// `MysekaiMissionUtility.GetChildMissions`: the sheet's normal missions
    /// in `seq` order, ties in master order.
    pub fn child_missions(&self, sheet_id: i32) -> Vec<&Mission> {
        let mut children: Vec<&Mission> = self
            .missions
            .iter()
            .filter(|mission| {
                mission.sheet_id == sheet_id && mission.category == MissionCategory::Normal
            })
            .collect();
        children.sort_by_key(|mission| mission.seq);
        children
    }

    /// `MasterDataManager.GetMasterMysekaiNormalMission(id)`.
    pub fn mission(&self, id: i32) -> Option<&Mission> {
        self.missions.iter().find(|mission| mission.id == id)
    }

    /// `MysekaiMissionUtility.GetMissionRewards`: the mission's reward rows in
    /// `seq` order, ties in master order.
    pub fn rewards_of(&self, mission_id: i32) -> Vec<&MissionReward> {
        let mut rewards: Vec<&MissionReward> = self
            .rewards
            .iter()
            .filter(|reward| reward.mission_id == mission_id)
            .collect();
        rewards.sort_by_key(|reward| reward.seq);
        rewards
    }

    /// `MasterDataManager.GetMasterMysekaiNormalMissionRewardMysekaiRankObtainedExp(id)`.
    pub fn rank_exp(&self, id: i32) -> Option<RankExpRow> {
        self.rank_exps.iter().copied().find(|row| row.id == id)
    }
}

/// `mysekaiNormalMissionSheets`.
pub fn parse_sheets(rows: &[Value]) -> Result<Vec<MissionSheet>, String> {
    rows.iter()
        .map(|row| {
            Ok(MissionSheet {
                id: crate::int32(row, "id")?,
                seq: crate::int32(row, "seq")?,
                name: match row.get("name").and_then(Value::as_str) {
                    Some(name) => name.to_owned(),
                    None => return Err(format!("name of {row} is not a string")),
                },
            })
        })
        .collect()
}

/// `mysekaiNormalMissions`: a row whose category this reader does not know
/// drops the table.
pub fn parse_missions(rows: &[Value]) -> Result<Vec<Mission>, String> {
    rows.iter()
        .map(|row| {
            let category = row
                .get("mysekaiNormalMissionCategory")
                .and_then(Value::as_str)
                .and_then(MissionCategory::parse)
                .ok_or_else(|| {
                    format!(
                        "mysekaiNormalMissionCategory of {row} is {:?}, not a category this reader knows",
                        row.get("mysekaiNormalMissionCategory")
                    )
                })?;
            Ok(Mission {
                id: crate::int32(row, "id")?,
                sheet_id: crate::int32(row, "mysekaiNormalMissionSheetId")?,
                seq: crate::int32(row, "seq")?,
                sentence: row
                    .get("sentence")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                category,
                requirement1: crate::optional_int32(row, "requirement1")?,
            })
        })
        .collect()
}

/// `mysekaiNormalMissionRewards`: a row whose reward type this reader does
/// not know drops the table (a reward outside the six types would be drawn
/// as another type by mistake).
pub fn parse_rewards(rows: &[Value]) -> Result<Vec<MissionReward>, String> {
    rows.iter()
        .map(|row| {
            let reward_type = row
                .get("mysekaiNormalMissionRewardType")
                .and_then(Value::as_str)
                .and_then(RewardType::parse)
                .ok_or_else(|| {
                    format!(
                        "mysekaiNormalMissionRewardType of {row} is {:?}, not a type this reader knows",
                        row.get("mysekaiNormalMissionRewardType")
                    )
                })?;
            Ok(MissionReward {
                id: crate::int32(row, "id")?,
                mission_id: crate::int32(row, "mysekaiNormalMissionId")?,
                seq: crate::optional_int32(row, "seq")?,
                reward_type,
                external_id: crate::optional_int32(row, "externalId")?,
            })
        })
        .collect()
}

/// `mysekaiNormalMissionRewardMysekaiRankObtainedExps`.
pub fn parse_rank_exps(rows: &[Value]) -> Result<Vec<RankExpRow>, String> {
    rows.iter()
        .map(|row| {
            Ok(RankExpRow {
                id: crate::int32(row, "id")?,
                quantity: crate::int32(row, "quantity")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mission(id: i32, sheet: i32, seq: i32, category: &str) -> Value {
        json!({
            "id": id,
            "mysekaiNormalMissionSheetId": sheet,
            "seq": seq,
            "sentence": format!("mission {id}"),
            "mysekaiNormalMissionType": "create_any_mysekai_fixture",
            "mysekaiNormalMissionCategory": category,
            "requirement1": 3,
        })
    }

    fn reward(id: i32, mission: i32, seq: i32, kind: &str, external: i32) -> Value {
        json!({
            "id": id,
            "mysekaiNormalMissionId": mission,
            "seq": seq,
            "mysekaiNormalMissionRewardType": kind,
            "externalId": external,
        })
    }

    /// The tables are matched by id through their columns in the master
    /// shape, whatever the array order.
    #[test]
    fn the_tables_read_by_their_columns() {
        let sheets = parse_sheets(&[
            json!({ "id": 2, "seq": 2, "name": "second" }),
            json!({ "id": 1, "seq": 1, "name": "first" }),
        ])
        .unwrap();
        let missions = parse_missions(&[
            mission(11, 1, 1, "normal"),
            mission(10, 1, 0, "complete"),
            mission(21, 2, 1, "normal"),
        ])
        .unwrap();
        let rewards = parse_rewards(&[reward(1, 11, 2, "resource_box", 100)]).unwrap();
        let masters = MissionMasters {
            sheets,
            missions,
            rewards,
            rank_exps: Vec::new(),
        };
        let order: Vec<i32> = masters.sheets_by_seq().iter().map(|sheet| sheet.id).collect();
        assert_eq!(order, vec![1, 2]);
        assert_eq!(masters.last_sheet().unwrap().id, 2);
        assert_eq!(masters.parent_mission(1).unwrap().id, 10);
        let children: Vec<i32> = masters.child_missions(1).iter().map(|m| m.id).collect();
        assert_eq!(children, vec![11]);
        assert_eq!(masters.rewards_of(11).len(), 1);
    }

    /// The rewards of a mission come in `seq` order whatever their array
    /// order.
    #[test]
    fn reward_rows_come_in_seq_order() {
        let rewards = parse_rewards(&[
            reward(1, 100, 2, "mysekai_rank_obtained_exp", 1),
            reward(2, 100, 1, "resource_box", 100),
            reward(3, 101, 1, "resource_box", 1),
        ])
        .unwrap();
        let masters = MissionMasters {
            rewards,
            ..MissionMasters::empty()
        };
        let kinds: Vec<RewardType> = masters
            .rewards_of(100)
            .iter()
            .map(|reward| reward.reward_type)
            .collect();
        assert_eq!(kinds, vec![RewardType::ResourceBox, RewardType::RankObtainedExp]);
        assert_eq!(masters.rewards_of(100)[0].external_id, 100);
    }

    /// An unknown category or reward type is refused as a whole table
    /// rather than read as some other value.
    #[test]
    fn unknown_words_are_refused() {
        assert!(parse_missions(&[mission(1, 1, 1, "side")]).is_err());
        assert!(parse_rewards(&[reward(1, 1, 1, "mysekai_date_change", 0)]).is_err());
    }

    /// The six reward types and the status label each one carries.
    #[test]
    fn reward_types_read_and_name_their_words() {
        let words = [
            ("resource_box", RewardType::ResourceBox, None, true),
            (
                "mysekai_material_possession",
                RewardType::MaterialPossession,
                Some("WORD_MATERIAL_POSSESSION"),
                false,
            ),
            (
                "mysekai_fixture_possession",
                RewardType::FixturePossession,
                Some("WORD_FIXTURE_POSSESSION"),
                false,
            ),
            (
                "mysekai_convert_fixture_level",
                RewardType::ConvertFixtureLevel,
                Some("WORD_TRADE_MACHINE_LEVEL_UP"),
                false,
            ),
            (
                "mysekai_rank_obtained_exp",
                RewardType::RankObtainedExp,
                Some("WORD_SEKAI_RANK_EXP"),
                false,
            ),
            ("mysekai_release_element", RewardType::ReleaseElement, None, false),
        ];
        for (word, kind, key, is_box) in words {
            let parsed = parse_rewards(&[reward(1, 1, 1, word, 7)]).unwrap();
            assert_eq!(parsed[0].reward_type, kind);
            assert_eq!(kind.status_word_key(), key);
            assert_eq!(kind.is_box(), is_box);
            assert_eq!(kind.is_status(), !is_box);
        }
    }

    /// Rank experience rows read by id and quantity.
    #[test]
    fn rank_exp_rows_read_by_id() {
        let rows = parse_rank_exps(&[json!({ "id": 1, "quantity": 7000 })]).unwrap();
        let masters = MissionMasters {
            rank_exps: rows,
            ..MissionMasters::empty()
        };
        assert_eq!(masters.rank_exp(1), Some(RankExpRow { id: 1, quantity: 7000 }));
        assert_eq!(masters.rank_exp(2), None);
    }

    /// The tables alone are empty: no sheet, no mission, no last sheet.
    #[test]
    fn empty_tables_read_as_empty() {
        let masters = MissionMasters::empty();
        assert!(masters.last_sheet().is_none());
        assert!(masters.parent_mission(1).is_none());
        assert!(masters.child_missions(1).is_empty());
        assert!(masters.rewards_of(1).is_empty());
    }
}
