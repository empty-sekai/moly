//! The one server clock, and the refresh-window arithmetic the server's
//! refresh and daily schedule use.
//!
//! The windows are the master refresh time periods (`startHour`, `endHour`;
//! hours may pass 24), read in device local time, the frame the client's
//! schedule selection reads timestamps in. The browser and the native host
//! both read the device's offset from the platform's local time conversion.

use super::document::{Clock, ScheduleRow};

pub(crate) use moly_masters::clock::RefreshPeriod;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

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
pub(crate) fn device_utc_offset_ms(epoch_ms: i64) -> i64 {
    let seconds = epoch_ms.div_euclid(1000);
    let t = seconds as libc::time_t;
    // SAFETY: the out-parameter is a zeroed value owned here, and the input
    // pointer refers to a local that outlives the call.
    let local = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        #[cfg(windows)]
        assert_eq!(libc::localtime_s(&mut tm, &t), 0, "localtime_s failed");
        #[cfg(unix)]
        assert!(!libc::localtime_r(&t, &mut tm).is_null(), "localtime_r failed");
        tm
    };
    let local_seconds = days_from_civil(
        local.tm_year as i64 + 1900,
        local.tm_mon as i64 + 1,
        local.tm_mday as i64,
    ) * 86_400
        + local.tm_hour as i64 * 3600
        + local.tm_min as i64 * 60
        + local.tm_sec as i64;
    (local_seconds - seconds) * 1000
}

/// Days from 1970-01-01 to the proleptic Gregorian date `year-month-day`.
#[cfg(not(target_arch = "wasm32"))]
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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

/// The local midnight (an epoch millisecond) of the local day holding
/// `now_ms`: the `scheduleDate` of the schedule rows of that day.
pub(crate) fn local_today(now_ms: i64, offset: impl Fn(i64) -> i64) -> Option<i64> {
    Some(local_midnight(now_ms, &offset))
}
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
