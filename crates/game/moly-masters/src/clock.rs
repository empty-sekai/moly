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

/// `configs.mysekai_refresh_hour`: the local hour the MySekai calendar turns.
pub const CONFIG_REFRESH_HOUR: &str = "mysekai_refresh_hour";
/// `configs.mysekai_weekly_reference`: the weekday the MySekai week turns on.
pub const CONFIG_WEEKLY_REFERENCE: &str = "mysekai_weekly_reference";

const DAY_MS: i64 = 86_400_000;
const HOUR_MS: i64 = 3_600_000;
const WEEKDAY_NAMES: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];

/// The MySekai weekly and monthly refresh: the local hour and the weekday
/// (`System.DayOfWeek`, Sunday = 0) the `configs` table names. Every rule
/// reads local time as UTC plus one offset (`TimeUtility.TimestampToDateTime`
/// adds a single base offset to every timestamp it converts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarRefresh {
    pub hour: i64,
    pub weekday: i64,
}

impl CalendarRefresh {
    /// Reads the two `configs` rows. The hour must be an hour of the day; the
    /// weekday follows `MysekaiUtility.GetWeeklyRefreshDayOfWeek`: a
    /// case-insensitive weekday name or an integer, and Sunday for anything
    /// else (an absent row included).
    pub fn from_configs(rows: &[Value]) -> Result<Self, String> {
        let value = |key: &str| {
            rows.iter()
                .find(|row| row.get("configKey").and_then(Value::as_str) == Some(key))
                .and_then(|row| row.get("value"))
                .and_then(Value::as_str)
        };
        Self::from_values(value(CONFIG_REFRESH_HOUR), value(CONFIG_WEEKLY_REFERENCE))
    }

    /// [`Self::from_configs`] on the two values themselves.
    pub fn from_values(hour: Option<&str>, weekly_reference: Option<&str>) -> Result<Self, String> {
        let text = hour.ok_or_else(|| format!("configs has no {CONFIG_REFRESH_HOUR} row"))?;
        let hour = text.trim().parse::<i64>().map_err(|_| {
            format!("configs {CONFIG_REFRESH_HOUR} value {text:?} is not an integer")
        })?;
        if !(0..24).contains(&hour) {
            return Err(format!(
                "configs {CONFIG_REFRESH_HOUR} value {hour} is not an hour of the day"
            ));
        }
        Ok(Self {
            hour,
            weekday: weekday_of(weekly_reference),
        })
    }

    /// `TimeUtility.HasCrossedWeeklyRefresh`: the target's local date at the
    /// refresh hour, moved to the reference weekday of its Sunday-first week,
    /// plus seven days; crossed once the clock reaches that instant.
    pub fn has_crossed_weekly(&self, target_at: i64, now: i64, offset_ms: i64) -> bool {
        let day = (target_at + offset_ms).div_euclid(DAY_MS);
        let anchor = (day + self.weekday - weekday(day) + 7) * DAY_MS + self.hour * HOUR_MS;
        anchor <= now + offset_ms
    }

    /// `TimeUtility.HasCrossedMonthlyRefresh`: the first day of the month
    /// after the target's local month, at midnight (the refresh hour does not
    /// enter the result); crossed once the clock reaches it.
    pub fn has_crossed_monthly(&self, target_at: i64, now: i64, offset_ms: i64) -> bool {
        next_month_first(target_at, offset_ms) <= now
    }

    /// `TimeUtility.GetNextWeekFirst(refreshDayOfWeek, refreshHour)` as
    /// `MysekaiUtility.GetWeeklyRefreshDateTime` calls it: the next reference
    /// weekday at the refresh hour; on the reference weekday itself, today
    /// before the refresh hour and a week later from it on. Epoch
    /// milliseconds.
    pub fn next_week_first(&self, now: i64, offset_ms: i64) -> i64 {
        let local = now + offset_ms;
        let day = local.div_euclid(DAY_MS);
        let hour = local.rem_euclid(DAY_MS) / HOUR_MS;
        let ahead = 7 + self.weekday - weekday(day);
        let whole = ahead / 7 * 7;
        let days = if ahead != whole {
            ahead - whole
        } else if hour >= self.hour {
            7
        } else {
            0
        };
        (day + days) * DAY_MS + self.hour * HOUR_MS - offset_ms
    }
}

/// `TimeUtility.GetNextMonthFirst`: the first day of the month after the
/// local month of `ms`, at local midnight. Epoch milliseconds.
pub fn next_month_first(ms: i64, offset_ms: i64) -> i64 {
    let (year, month) = civil_from_days((ms + offset_ms).div_euclid(DAY_MS));
    let (year, month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    days_from_civil(year, month, 1) * DAY_MS - offset_ms
}

/// `Enum.TryParse<DayOfWeek>(value, ignoreCase: true)`, Sunday on failure.
fn weekday_of(value: Option<&str>) -> i64 {
    let Some(value) = value.map(str::trim) else {
        return 0;
    };
    if let Ok(number) = value.parse::<i64>() {
        return number;
    }
    WEEKDAY_NAMES
        .iter()
        .position(|name| name.eq_ignore_ascii_case(value))
        .map_or(0, |index| index as i64)
}

/// The weekday (Sunday = 0) of a day count since the epoch, a Thursday.
fn weekday(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

/// The proleptic Gregorian year and month of a day count since the epoch.
fn civil_from_days(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month)
}

/// The day count since the epoch of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = HOUR_MS;
    /// Monday 2026-10-19 00:00 UTC (day 20745).
    const MONDAY: i64 = 20_745 * DAY_MS;
    const JST: i64 = 9 * HOUR;
    const RULE: CalendarRefresh = CalendarRefresh { hour: 5, weekday: 1 };

    #[test]
    fn the_weekday_reads_names_and_integers_and_falls_back_to_sunday() {
        assert_eq!(weekday(20_745), 1);
        assert_eq!(weekday_of(Some("MONDAY")), 1);
        assert_eq!(weekday_of(Some("saturday")), 6);
        assert_eq!(weekday_of(Some("3")), 3);
        assert_eq!(weekday_of(Some("someday")), 0);
        assert_eq!(weekday_of(None), 0);
    }

    #[test]
    fn a_week_turns_on_the_reference_day_after_the_targets_week() {
        // A start on Wednesday 10:00 local: Monday of that week at 05:00 is
        // two days back, plus seven is next Monday 05:00 local.
        let wednesday = MONDAY + 2 * DAY_MS + 10 * HOUR - JST;
        let next_monday = MONDAY + 7 * DAY_MS + 5 * HOUR - JST;
        assert!(!RULE.has_crossed_weekly(wednesday, next_monday - 1, JST));
        assert!(RULE.has_crossed_weekly(wednesday, next_monday, JST));
        // A Sunday start: the reference Monday is the next day, plus seven.
        let sunday = MONDAY - DAY_MS + 12 * HOUR - JST;
        let eight_days = MONDAY + 7 * DAY_MS + 5 * HOUR - JST;
        assert!(!RULE.has_crossed_weekly(sunday, eight_days - 1, JST));
        assert!(RULE.has_crossed_weekly(sunday, eight_days, JST));
    }

    #[test]
    fn a_month_turns_at_local_midnight_of_the_next_first() {
        let february = days_from_civil(2026, 2, 1) * DAY_MS - JST;
        assert_eq!(next_month_first(february - HOUR, JST), february);
        assert!(RULE.has_crossed_monthly(february - HOUR, february, JST));
        assert!(!RULE.has_crossed_monthly(february - HOUR, february - 1, JST));
        let december = days_from_civil(2026, 12, 5) * DAY_MS;
        assert_eq!(next_month_first(december, 0), days_from_civil(2027, 1, 1) * DAY_MS);
    }

    #[test]
    fn the_next_week_first_holds_today_before_the_hour_on_the_reference_day() {
        let week = 7 * DAY_MS;
        assert_eq!(RULE.next_week_first(MONDAY + 4 * HOUR, 0), MONDAY + 5 * HOUR);
        assert_eq!(RULE.next_week_first(MONDAY + 5 * HOUR, 0), MONDAY + week + 5 * HOUR);
        assert_eq!(RULE.next_week_first(MONDAY + 3 * DAY_MS, 0), MONDAY + week + 5 * HOUR);
        assert_eq!(RULE.next_week_first(MONDAY - DAY_MS, 0), MONDAY + 5 * HOUR);
    }
}
