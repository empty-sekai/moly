//! The rank domain's world half: the user's total experience and the
//! `mysekaiRanks` row parser. The rank model the menu dialog and the info
//! screen build stays in `crate::mysekai_rank`; the NPC change-site cap and
//! the saved player data's seat read these two.

use bevy::prelude::*;

use moly_assets::json::master;
use moly_law::ui::mysekai_rank::MasterMysekaiRank;

/// `UserMysekaiGamedata.totalExp`, the server-side user state both rank
/// models read: the saved player data's value when it carries one, else 0
/// until the join's answer arrives. Its responses keep it current.
#[derive(Resource)]
pub(crate) struct UserTotalExp(pub(crate) i32);

impl UserTotalExp {
    /// Imported player data's total experience; without one the current
    /// value stays (the join's answer fills it).
    pub(crate) fn set(&mut self, imported: Option<i32>) {
        if let Some(imported) = imported {
            self.0 = imported;
        }
    }

    /// The value the world is seated with: the saved player data's total
    /// experience, else 0 until the join.
    pub(crate) fn seated(saved: Option<&moly_core::saved_total_exp::SavedTotalExp>) -> Self {
        UserTotalExp(saved.and_then(|saved| saved.0).unwrap_or(0))
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
