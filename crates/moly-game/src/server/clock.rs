//! The one server clock, and the refresh-window arithmetic the server's
//! refresh and daily schedule use.
//!
//! The windows are the master refresh time periods (`startHour`, `endHour`;
//! hours may pass 24), read in device local time, the frame the client's
//! schedule selection reads timestamps in. The browser supplies its own
//! offset; the native host has no time-zone source and uses UTC, as the
//! server panel does.

use super::document::{Clock, ScheduleRow};

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

/// One master refresh window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RefreshPeriod {
    pub(crate) id: i32,
    pub(crate) start_hour: i64,
    pub(crate) end_hour: i64,
}

/// Device epoch milliseconds.
#[cfg(target_arch = "wasm32")]
pub(crate) fn device_now_ms() -> i64 {
    js_sys::Date::now() as i64
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn device_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Offset of device local time from UTC, in milliseconds, at an epoch
/// millisecond.
#[cfg(target_arch = "wasm32")]
pub(crate) fn device_utc_offset_ms(epoch_ms: i64) -> i64 {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(epoch_ms as f64));
    // getTimezoneOffset is UTC minus local, in minutes.
    -(date.get_timezone_offset() as i64) * 60_000
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn device_utc_offset_ms(_epoch_ms: i64) -> i64 {
    0
}

/// The server clock's epoch millisecond.
pub(crate) fn server_now_ms(clock: Clock) -> i64 {
    match clock {
        Clock::Device { offset_ms } => device_now_ms().saturating_add(offset_ms),
        Clock::Fixed { at_ms } => at_ms,
    }
}

/// Device-local midnight (as an epoch millisecond) of the local day holding
/// `epoch_ms`, under `offset`.
fn local_midnight(epoch_ms: i64, offset: &impl Fn(i64) -> i64) -> i64 {
    let local = epoch_ms + offset(epoch_ms);
    let midnight_local = local.div_euclid(DAY_MS) * DAY_MS;
    midnight_local - offset(midnight_local - offset(epoch_ms))
}

/// The start of the refresh window holding `now_ms`: the latest window
/// start at or before it among the windows of the local day before and the
/// day of `now_ms`. `None` when no window holds it.
pub(crate) fn window_start(
    now_ms: i64,
    periods: &[RefreshPeriod],
    offset: impl Fn(i64) -> i64,
) -> Option<i64> {
    let today = local_midnight(now_ms, &offset);
    let mut best: Option<i64> = None;
    for day in [today - DAY_MS, today] {
        for period in periods {
            let start = day + period.start_hour * HOUR_MS;
            let end = day + period.end_hour * HOUR_MS;
            if start <= now_ms && now_ms < end {
                best = Some(best.map_or(start, |known| known.max(start)));
            }
        }
    }
    best
}

/// The daily policy's rows: the local day before, the day of and the day
/// after `now_ms`, each master window in master order, the schedule date at
/// local midnight.
pub(crate) fn daily_rows(
    now_ms: i64,
    periods: &[RefreshPeriod],
    by_period: &std::collections::BTreeMap<i32, i32>,
    offset: impl Fn(i64) -> i64,
) -> Vec<ScheduleRow> {
    let today = local_midnight(now_ms, &offset);
    let mut rows = Vec::new();
    for day in [today - DAY_MS, today, today + DAY_MS] {
        // The midnight of a neighbouring day under that day's own offset.
        let midnight = local_midnight(day + HOUR_MS * 12, &offset);
        for period in periods {
            if let Some(phenomenon) = by_period.get(&period.id) {
                rows.push(ScheduleRow {
                    refresh_time_period_id: period.id,
                    schedule_date: midnight,
                    phenomena_id: *phenomenon,
                });
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIODS: [RefreshPeriod; 2] = [
        RefreshPeriod {
            id: 1,
            start_hour: 5,
            end_hour: 17,
        },
        RefreshPeriod {
            id: 2,
            start_hour: 17,
            end_hour: 29,
        },
    ];
    const JST: i64 = 9 * HOUR_MS;

    #[test]
    fn window_start_follows_the_local_day() {
        // 2026-09-24 12:00 JST is 03:00 UTC.
        let noon = 1_790_218_800_000;
        let start = window_start(noon, &PERIODS, |_| JST).unwrap();
        // 05:00 JST the same day.
        assert_eq!(start, noon - 7 * HOUR_MS);
        // 02:00 JST the next day is still in window 2 of the previous day.
        let night = noon + 14 * HOUR_MS;
        assert_eq!(
            window_start(night, &PERIODS, |_| JST).unwrap(),
            noon + 5 * HOUR_MS
        );
    }

    #[test]
    fn daily_rows_match_the_checked_in_dates() {
        let noon = 1_790_218_800_000;
        let map = [(1, 1), (2, 1)].into_iter().collect();
        let rows = daily_rows(noon, &PERIODS, &map, |_| JST);
        let dates: Vec<i64> = rows.iter().map(|row| row.schedule_date).collect();
        assert_eq!(
            dates,
            vec![
                1_790_089_200_000,
                1_790_089_200_000,
                1_790_175_600_000,
                1_790_175_600_000,
                1_790_262_000_000,
                1_790_262_000_000
            ]
        );
    }
}
