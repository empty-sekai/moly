//! The server clock's refresh windows, the master time periods the refresh
//! policy and the daily schedule are read against.

use serde_json::Value;

/// One master refresh window (`startHour`, `endHour`; hours may pass 24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshPeriod {
    pub id: i32,
    pub start_hour: i64,
    pub end_hour: i64,
}

/// `refreshTimePeriods` of the phenomena index.
pub fn parse_periods(rows: &[Value]) -> Result<Vec<RefreshPeriod>, String> {
    rows.iter()
        .map(|row| {
            Ok(RefreshPeriod {
                id: crate::int32(row, "id")?,
                start_hour: row["startHour"].as_i64().ok_or("startHour is not an int")?,
                end_hour: row["endHour"].as_i64().ok_or("endHour is not an int")?,
            })
        })
        .collect()
}
