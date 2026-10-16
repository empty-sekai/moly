//! The game's MySekai rank model and the experience gauge it feeds.
//!
//! `MysekaiRankModel(totalExp)` reads the master rank table (rows with
//! `id`, `mysekaiRank`, `totalExp`, in master order) and the user's total
//! experience, and stores, in this order:
//! 1. the total-experience table, `mysekaiRank -> totalExp` (a dictionary
//!    built from every row; a repeated rank throws);
//! 2. the highest rank, the maximum `mysekaiRank` over the rows (an empty
//!    table throws);
//! 3. the total experience as given;
//! 4. the rank: the rows ordered by `mysekaiRank` descending (a stable sort),
//!    the first whose `totalExp` is at most the total experience, its
//!    `mysekaiRank`; 0 when no row qualifies;
//! 5. the total experience of the current rank: the first row in master order
//!    whose `mysekaiRank` equals the rank, its `totalExp` (no such row throws);
//! 6. the total experience of the next rank, looked up the same way for
//!    `Math.Min(rank + 1, highest rank)`, and the experience to the next rank,
//!    that value minus the total experience (C# `int` arithmetic, which wraps).
//!
//! The parameterless constructor the menu dialog and the info screen use
//! takes the total experience from the user's MySekai game data (0 when the
//! user has none); that value is server-side user state.
//!
//! `MysekaiRankGaugeViewDataModel` copies the rank, the highest rank, the total
//! experience and the two per-rank totals; `UIPartsMysekaiRankGauge.Setup`
//! writes the rank as decimal digits and calls `UIPartsGaugeExp.Setup(rank,
//! highest rank, total, current-rank total, next-rank total, "MSG_REST_VALUE")`.
//! That setup sets the gauge to `(0, 1)` and writes `WORD_MAX` into the rest
//! text when the rank equals the highest rank; otherwise it sets the gauge to
//! `(total - current total, next total - current total)` and writes the rest
//! key with one argument, `next total - total`. `UIPartsGauge.Setup(int, int)`
//! converts both to `float` and calls `Setup(float, float)`, which does
//! nothing when the maximum is 0 and otherwise sets the fill image's fill
//! amount to `now / max`; the Image setter clamps it to [0, 1].

/// One master rank row as the model reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasterMysekaiRank {
    pub id: i32,
    pub mysekai_rank: i32,
    pub total_exp: i32,
}

/// `MysekaiUtility.GetMysekaiRank(totalExp)`.
pub fn get_mysekai_rank(ranks: &[MasterMysekaiRank], total_exp: i32) -> i32 {
    let mut ordered: Vec<&MasterMysekaiRank> = ranks.iter().collect();
    // OrderByDescending is a stable sort: equal keys keep master order.
    ordered.sort_by(|a, b| b.mysekai_rank.cmp(&a.mysekai_rank));
    ordered
        .into_iter()
        .find(|row| row.total_exp <= total_exp)
        .map_or(0, |row| row.mysekai_rank)
}

/// `MasterDataManager.GetMasterMysekaiRank(mysekaiRank)`: the first row in
/// master order with that rank.
pub fn get_master_mysekai_rank(ranks: &[MasterMysekaiRank], mysekai_rank: i32) -> Option<&MasterMysekaiRank> {
    ranks.iter().find(|row| row.mysekai_rank == mysekai_rank)
}

/// The stored fields of `MysekaiRankModel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MysekaiRankModel {
    pub mysekai_rank: i32,
    pub max_mysekai_rank: i32,
    pub total_exp: i32,
    pub total_exp_to_current_rank: i32,
    pub total_exp_to_next_rank: i32,
    pub exp_to_next_rank: i32,
    /// `TotalExpTable`, in master order.
    pub total_exp_table: Vec<(i32, i32)>,
}

impl MysekaiRankModel {
    /// `MysekaiRankModel(int totalExp)`. The errors are the source's throws.
    pub fn new(ranks: &[MasterMysekaiRank], total_exp: i32) -> Result<Self, String> {
        let mut total_exp_table: Vec<(i32, i32)> = Vec::with_capacity(ranks.len());
        for row in ranks {
            if total_exp_table.iter().any(|(rank, _)| *rank == row.mysekai_rank) {
                return Err(format!("rank table repeats mysekaiRank {}", row.mysekai_rank));
            }
            total_exp_table.push((row.mysekai_rank, row.total_exp));
        }
        let max_mysekai_rank = ranks
            .iter()
            .map(|row| row.mysekai_rank)
            .max()
            .ok_or("rank table has no rows")?;
        let mysekai_rank = get_mysekai_rank(ranks, total_exp);
        let current = get_master_mysekai_rank(ranks, mysekai_rank)
            .ok_or_else(|| format!("rank table has no row for rank {mysekai_rank} (total experience {total_exp})"))?;
        let next_rank = mysekai_rank.wrapping_add(1).min(max_mysekai_rank);
        let next = get_master_mysekai_rank(ranks, next_rank)
            .ok_or_else(|| format!("rank table has no row for rank {next_rank}"))?;
        Ok(MysekaiRankModel {
            mysekai_rank,
            max_mysekai_rank,
            total_exp,
            total_exp_to_current_rank: current.total_exp,
            total_exp_to_next_rank: next.total_exp,
            exp_to_next_rank: next.total_exp.wrapping_sub(total_exp),
            total_exp_table,
        })
    }
}

/// The rest-text call of `UIPartsGaugeExp.Setup`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestText {
    /// `SetWordingText("WORD_MAX")`, no arguments.
    Max,
    /// `SetWordingText(restWordingKey, new object[] { value })`.
    Rest { key: String, value: i32 },
}

/// The calls `UIPartsGaugeExp.Setup(lvNow, lvMax, afterTotalExp,
/// expNeedNowLv, expNeedNextLv, restWordingKey)` makes: the gauge's
/// `Setup(int now, int max)` and the rest text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GaugeExpSetup {
    pub gauge_now: i32,
    pub gauge_max: i32,
    pub rest: RestText,
}

/// `UIPartsGaugeExp.Setup` with the rest key given.
pub fn gauge_exp_setup(
    lv_now: i32,
    lv_max: i32,
    after_total_exp: i32,
    exp_need_now_lv: i32,
    exp_need_next_lv: i32,
    rest_wording_key: &str,
) -> GaugeExpSetup {
    if lv_now == lv_max {
        GaugeExpSetup { gauge_now: 0, gauge_max: 1, rest: RestText::Max }
    } else {
        GaugeExpSetup {
            gauge_now: after_total_exp.wrapping_sub(exp_need_now_lv),
            gauge_max: exp_need_next_lv.wrapping_sub(exp_need_now_lv),
            rest: RestText::Rest {
                key: rest_wording_key.to_owned(),
                value: exp_need_next_lv.wrapping_sub(after_total_exp),
            },
        }
    }
}

/// `UIPartsGauge.Setup(int now, int max)` through `Setup(float, float)`:
/// the fill amount it assigns, or `None` when the maximum is 0 (nothing is
/// assigned). The value is the one handed to the Image setter, before its
/// clamp.
pub fn gauge_fill_amount(now: i32, max: i32) -> Option<f32> {
    let (now, max) = (now as f32, max as f32);
    (max != 0.0).then(|| now / max)
}

/// `UIPartsMysekaiRankGauge.Setup`'s rest key.
pub const RANK_GAUGE_REST_WORDING_KEY: &str = "MSG_REST_VALUE";

/// `UIPartsMysekaiRankGauge.Setup(MysekaiRankGaugeViewDataModel)` on the
/// model: the rank text and the experience gauge's calls.
pub fn rank_gauge_setup(model: &MysekaiRankModel) -> (String, GaugeExpSetup) {
    (
        model.mysekai_rank.to_string(),
        gauge_exp_setup(
            model.mysekai_rank,
            model.max_mysekai_rank,
            model.total_exp,
            model.total_exp_to_current_rank,
            model.total_exp_to_next_rank,
            RANK_GAUGE_REST_WORDING_KEY,
        ),
    )
}
