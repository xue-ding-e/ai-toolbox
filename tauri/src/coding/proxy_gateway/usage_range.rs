//! Resolve the persisted gateway-statistics time range into unix seconds.
//!
//! The tray usage block follows whatever range the statistics page has
//! selected, so the backend has to reproduce the frontend's
//! `resolveGatewayUsageRange` semantics: calendar presets are computed in the
//! **local** timezone, weeks start on Monday, and relative presets are
//! re-resolved from the current clock on every menu build (never frozen at the
//! moment the user picked them).

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};

use crate::settings::GatewayUsageRangeSelection;

const MILLIS_PER_SECOND: i64 = 1000;

/// Unix-second bounds `(start, end)` for a persisted range selection.
///
/// `custom` uses the stored absolute bounds; every other preset is relative to
/// `now_ms`. An unknown preset falls back to "today" so a corrupted or older
/// value can never produce an unbounded query.
pub fn resolve_usage_range(selection: &GatewayUsageRangeSelection, now_ms: i64) -> (i64, i64) {
    let Some(now) = Local.timestamp_millis_opt(now_ms).earliest() else {
        return (now_ms / MILLIS_PER_SECOND, now_ms / MILLIS_PER_SECOND);
    };
    let today = now.date_naive();
    let now_seconds = now_ms / MILLIS_PER_SECOND;

    match selection.preset.as_str() {
        "yesterday" => {
            let start = local_midnight(today - Duration::days(1));
            (start / MILLIS_PER_SECOND, local_midnight(today) / MILLIS_PER_SECOND - 1)
        }
        "thisWeek" => {
            let monday = start_of_week(today);
            (local_midnight(monday) / MILLIS_PER_SECOND, now_seconds)
        }
        "lastWeek" => {
            let this_monday = start_of_week(today);
            let previous_monday = this_monday - Duration::days(7);
            (
                local_midnight(previous_monday) / MILLIS_PER_SECOND,
                local_midnight(this_monday) / MILLIS_PER_SECOND - 1,
            )
        }
        "thisMonth" => (
            local_midnight(first_of_month(today)) / MILLIS_PER_SECOND,
            now_seconds,
        ),
        "lastMonth" => {
            let this_month = first_of_month(today);
            let previous_month = shift_month(this_month, -1);
            (
                local_midnight(previous_month) / MILLIS_PER_SECOND,
                local_midnight(this_month) / MILLIS_PER_SECOND - 1,
            )
        }
        "thisYear" => (
            local_midnight(NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today))
                / MILLIS_PER_SECOND,
            now_seconds,
        ),
        "custom" => match (selection.start_date, selection.end_date) {
            (Some(start), Some(end)) if start <= end => (start, end),
            (Some(start), Some(end)) => (end, start),
            // Mirrors the frontend's `custom` fallbacks exactly, so a
            // half-filled picker resolves to the same window in both places.
            (Some(start), None) => (start, now_seconds.max(start)),
            (None, Some(end)) => (end.saturating_sub(86_400), end),
            (None, None) => (now_seconds.saturating_sub(86_400), now_seconds),
        },
        _ => (
            local_midnight(today) / MILLIS_PER_SECOND,
            now_seconds,
        ),
    }
}

/// Monday of the week containing `date`.
fn start_of_week(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

fn first_of_month(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date)
}

/// Shift `first_of_month_date` by `months` months; the input is always the
/// first day, so the day-of-month can never overflow.
fn shift_month(first_day: NaiveDate, months: i32) -> NaiveDate {
    let zero_based = first_day.year() * 12 + first_day.month0() as i32 + months;
    let year = zero_based.div_euclid(12);
    let month = zero_based.rem_euclid(12) as u32 + 1;
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(first_day)
}

/// Local midnight of `date`, in unix milliseconds.
///
/// Some timezones skip midnight on a DST transition day. `earliest()` returns
/// `None` for a non-existent local time, so step forward until the wall clock
/// exists again; the first existing instant is that day's start.
fn local_midnight(date: NaiveDate) -> i64 {
    let mut naive = date
        .and_hms_opt(0, 0, 0)
        .unwrap_or_else(|| date.and_hms_opt(12, 0, 0).expect("valid time"));
    for _ in 0..4 {
        if let Some(midnight) = Local.from_local_datetime(&naive).earliest() {
            return midnight.timestamp_millis();
        }
        naive += Duration::hours(1);
    }
    // Unreachable in practice; keep the caller from panicking if a platform
    // reports a timezone with an unusually long skipped interval.
    date.and_hms_opt(0, 0, 0)
        .map(|naive| naive.and_utc().timestamp_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(preset: &str) -> GatewayUsageRangeSelection {
        GatewayUsageRangeSelection {
            preset: preset.to_string(),
            start_date: None,
            end_date: None,
        }
    }

    fn local_noon(year: i32, month: u32, day: u32) -> i64 {
        Local
            .with_ymd_and_hms(year, month, day, 12, 0, 0)
            .single()
            .expect("valid local noon")
            .timestamp_millis()
    }

    fn seconds(date: NaiveDate) -> i64 {
        local_midnight(date) / MILLIS_PER_SECOND
    }

    #[test]
    fn today_starts_at_local_midnight() {
        let now = local_noon(2026, 9, 12);
        let today = Local.timestamp_millis_opt(now).earliest().unwrap().date_naive();
        assert_eq!(
            resolve_usage_range(&selection("today"), now),
            (seconds(today), now / MILLIS_PER_SECOND)
        );
    }

    #[test]
    fn yesterday_is_the_previous_calendar_day() {
        let now = local_noon(2026, 9, 12);
        let today = Local.timestamp_millis_opt(now).earliest().unwrap().date_naive();
        let (start, end) = resolve_usage_range(&selection("yesterday"), now);
        assert_eq!(start, seconds(today - Duration::days(1)));
        assert_eq!(end, seconds(today) - 1);
    }

    #[test]
    fn weeks_start_on_monday() {
        let now = local_noon(2026, 9, 12); // Saturday
        let today = Local.timestamp_millis_opt(now).earliest().unwrap().date_naive();
        let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
        let (start, end) = resolve_usage_range(&selection("thisWeek"), now);
        assert_eq!(start, seconds(monday));
        assert_eq!(end, now / MILLIS_PER_SECOND);

        let (last_start, last_end) = resolve_usage_range(&selection("lastWeek"), now);
        assert_eq!(last_start, seconds(monday - Duration::days(7)));
        assert_eq!(last_end, seconds(monday) - 1);
    }

    #[test]
    fn last_month_steps_back_across_a_year_boundary() {
        let now = local_noon(2026, 1, 9);
        let (start, end) = resolve_usage_range(&selection("lastMonth"), now);
        assert_eq!(start, seconds(NaiveDate::from_ymd_opt(2025, 12, 1).unwrap()));
        assert_eq!(
            end,
            seconds(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()) - 1
        );
    }

    #[test]
    fn last_month_ends_before_this_month_starts() {
        let now = local_noon(2026, 3, 31);
        let (start, end) = resolve_usage_range(&selection("lastMonth"), now);
        assert_eq!(start, seconds(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()));
        assert_eq!(
            end,
            seconds(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()) - 1
        );
    }

    #[test]
    fn custom_uses_the_stored_absolute_bounds() {
        let selection = GatewayUsageRangeSelection {
            preset: "custom".to_string(),
            start_date: Some(1_700_000_000),
            end_date: Some(1_700_086_400),
        };
        assert_eq!(
            resolve_usage_range(&selection, local_noon(2026, 9, 12)),
            (1_700_000_000, 1_700_086_400)
        );
    }

    #[test]
    fn an_unknown_preset_falls_back_to_today() {
        let now = local_noon(2026, 9, 12);
        assert_eq!(
            resolve_usage_range(&selection("1d"), now),
            resolve_usage_range(&selection("today"), now)
        );
    }
}
