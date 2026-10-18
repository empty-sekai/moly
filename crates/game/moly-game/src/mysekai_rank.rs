//! The master rank table (`mysekaiRanks`) and the rank model the menu
//! dialog and the info screen build from it.
//!
//! The table is master data, read from the region's master mirror
//! (`mysekaiRanks`). The user's total experience is server-side user state,
//! delivered with the join and every later response, one value both screens
//! read ([`UserTotalExp`]).
//! A root whose table is absent or malformed does not stop the start: the
//! table is named once, and a rank model asked of it is refused with that
//! name ([`MysekaiRanks::try_model`]), as the source throws when its table is
//! absent. The table refuses as a whole on a row it cannot read: its rows
//! are rank thresholds, and a dropped row would move every rank above it.

use bevy::prelude::*;

use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_law::ui::mysekai_rank::{MasterMysekaiRank, MysekaiRankModel};

pub(crate) use crate::rank_table::{parse_ranks, UserTotalExp};

/// The rank table the rank gauges read.
const RANKS: MasterTable<Vec<MasterMysekaiRank>> = MasterTable {
    table: "mysekaiRanks",
    name: "mysekaiRanks (the rank gauges)",
    parse: parse_ranks,
};

/// The resolved table: its rows in master order, or why the root has none.
#[derive(Resource)]
pub(crate) struct MysekaiRanks(Result<Vec<MasterMysekaiRank>, String>);

/// Startup: request the table and seat the user's total experience.
pub(crate) fn load(
    mut commands: Commands,
    saved: Option<Res<crate::player_data::SavedTotalExp>>,
    mut masters: ResMut<MasterData>,
) {
    commands.insert_resource(UserTotalExp::seated(saved.as_deref()));
    masters.request(&RANKS);
}

/// Update: seat the table once it resolves. An absent or malformed table is
/// named once by the master layer and kept as the reason the rank gauges
/// refuse.
pub(crate) fn parse(mut commands: Commands, mut masters: ResMut<MasterData>) {
    let Some(result) = masters.take(&RANKS) else {
        return;
    };
    match result {
        Ok(rows) => {
            info!(
                "[mysekai-rank] master rank table: {} rows, highest rank {}",
                rows.len(),
                rows.iter().map(|row| row.mysekai_rank).max().unwrap_or(0)
            );
            commands.insert_resource(MysekaiRanks(Ok(rows)));
        }
        Err(error) => {
            info!("[mysekai-rank] no rank table: the rank gauges refuse when a screen shows them");
            commands.insert_resource(MysekaiRanks(Err(error.to_string())));
        }
    }
}

impl MysekaiRanks {
    /// `new MysekaiRankModel()` with the given total experience. `Err` names
    /// where the source throws, and a root without the table.
    pub(crate) fn try_model(&self, total_exp: i32) -> Result<MysekaiRankModel, String> {
        let rows = self.0.as_ref().map_err(|reason| format!("rank model: {reason}"))?;
        MysekaiRankModel::new(rows, total_exp).map_err(|err| format!("rank model: {err}"))
    }

}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    #[test]
    fn a_root_without_the_table_refuses_the_model_by_name() {
        let ranks = MysekaiRanks(Err("mysekaiRanks (the rank gauges): malformed".into()));
        let error = ranks.try_model(0).unwrap_err();
        assert!(error.contains("mysekaiRanks (the rank gauges): malformed"), "{error}");
    }

    #[test]
    fn a_rank_row_the_reader_cannot_read_refuses_the_whole_table() {
        assert!(parse_ranks(r#"[{"id": 1, "mysekaiRank": 1, "totalExp": 0},
            {"id": 2, "mysekaiRank": "2", "totalExp": 10}]"#)
        .is_err());
        let rows = parse_ranks(r#"[{"id": 1, "mysekaiRank": 1, "totalExp": 0},
            {"id": 2, "mysekaiRank": 2, "totalExp": 10}]"#)
        .expect("well-formed rows");
        assert_eq!(MysekaiRanks(Ok(rows)).try_model(10).unwrap().mysekai_rank, 2);
    }
}
