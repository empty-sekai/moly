//! The master rank table (`mysekaiRanks`) and the rank model the menu
//! dialog and the info screen build from it.
//!
//! The table is master data, read from the region's master mirror
//! (`mysekaiRanks`). The user's total experience is server-side user state
//! from the server document, one value both screens read ([`UserTotalExp`]).
//! A root whose table is absent or malformed does not stop the start: the
//! table is named once, and the first rank model asked of it panics with that
//! name, as the source throws when its table is absent.

use bevy::prelude::*;

use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_law::ui::mysekai_rank::{MasterMysekaiRank, MysekaiRankModel};

/// The rank table the rank gauges read.
const RANKS: MasterTable<Vec<MasterMysekaiRank>> = MasterTable {
    table: "mysekaiRanks",
    name: "mysekaiRanks (the rank gauges)",
    parse: parse_ranks,
};

/// The resolved table: its rows in master order, or why the root has none.
#[derive(Resource)]
pub(crate) struct MysekaiRanks(Result<Vec<MasterMysekaiRank>, String>);

/// `UserMysekaiGamedata.totalExp`, the server-side user state both rank
/// models read: the saved player data's value when it carries one, else the
/// server document's (its responses keep it current; the native overlay of
/// `MOLY_MENU_MOCK_TOTAL_EXP` lands in that document), else the document's
/// default before the document is installed.
#[derive(Resource)]
pub(crate) struct UserTotalExp(pub(crate) i32);

impl UserTotalExp {
    fn panel() -> i32 {
        crate::server::with_model(|model| model.document().gamedata.total_exp)
            .unwrap_or(crate::server::document::DEFAULT_TOTAL_EXP)
    }

    /// Imported player data's total experience, or the server document's.
    pub(crate) fn set(&mut self, imported: Option<i32>) {
        self.0 = imported.unwrap_or_else(Self::panel);
    }
}

impl Default for UserTotalExp {
    fn default() -> Self {
        UserTotalExp(crate::player_data::saved_total_exp().unwrap_or_else(Self::panel))
    }
}

/// Startup: request the table and seat the user's total experience.
pub(crate) fn load(mut commands: Commands, mut masters: ResMut<MasterData>) {
    commands.init_resource::<UserTotalExp>();
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

/// `mysekaiRanks`: the rows in master order.
pub(crate) fn parse_ranks(text: &str) -> Result<Vec<MasterMysekaiRank>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(MasterMysekaiRank {
                id: master::int32(row, "id")?,
                mysekai_rank: master::int32(row, "mysekaiRank")?,
                total_exp: master::int32(row, "totalExp")?,
            })
        })
        .collect()
}

impl MysekaiRanks {
    /// `new MysekaiRankModel()` with the given total experience. Panics where
    /// the source throws, and on a root without the table.
    pub(crate) fn model(&self, total_exp: i32) -> MysekaiRankModel {
        let rows = self.0.as_ref().unwrap_or_else(|reason| panic!("rank model: {reason}"));
        MysekaiRankModel::new(rows, total_exp).unwrap_or_else(|err| panic!("rank model: {err}"))
    }
}
