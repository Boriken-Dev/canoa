//! UTC calendar formatting and HTTP-date conversion backed by `time`.

use alloc::format;
use alloc::string::String;
use std::time::{SystemTime, UNIX_EPOCH};
use time::{Date, Month, OffsetDateTime};

fn unix_seconds(at: SystemTime) -> Option<i64> {
    let since = at.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since.as_secs()).ok()
}

fn utc_of(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// Formats `seconds` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub(crate) fn rfc3339_unix(seconds: i64) -> String {
    let dt = utc_of(seconds);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    )
}

/// Formats `at` as `YYYY-MM-DDTHH:MM:SSZ`, or `None` if before the Unix epoch or out of range.
pub(crate) fn rfc3339(at: SystemTime) -> Option<String> {
    let secs = unix_seconds(at)?;
    let dt = OffsetDateTime::from_unix_timestamp(secs).ok()?;
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    ))
}

/// Now as an ISO 8601 time in UTC, to the second (`YYYY-MM-DDTHH:MM:SSZ`).
pub(crate) fn iso_now() -> String {
    let secs = unix_seconds(SystemTime::now()).unwrap_or(0);
    rfc3339_unix(secs)
}

/// Current UTC `(YYYY-MM-DD, HHMMSS)`.
pub(crate) fn today_and_clock() -> (String, String) {
    let secs = unix_seconds(SystemTime::now()).unwrap_or(0);
    let dt = utc_of(secs);
    (
        format!(
            "{:04}-{:02}-{:02}",
            dt.year(),
            u8::from(dt.month()),
            dt.day()
        ),
        format!("{:02}{:02}{:02}", dt.hour(), dt.minute(), dt.second()),
    )
}

/// Formats Unix `seconds` as `YYYY-MM-DD HH:MM UTC`.
pub(crate) fn utc_minutes(seconds: i64) -> String {
    let dt = utc_of(seconds);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute(),
    )
}

/// Formats `at` as an HTTP-date (`Sun, 06 Nov 1994 08:49:37 GMT`).
pub(crate) fn http_date(at: SystemTime) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = unix_seconds(at).unwrap_or(0);
    let dt = utc_of(secs);
    let weekday = match dt.weekday() {
        time::Weekday::Monday => "Mon",
        time::Weekday::Tuesday => "Tue",
        time::Weekday::Wednesday => "Wed",
        time::Weekday::Thursday => "Thu",
        time::Weekday::Friday => "Fri",
        time::Weekday::Saturday => "Sat",
        time::Weekday::Sunday => "Sun",
    };
    let month = MONTHS[usize::from(u8::from(dt.month()).saturating_sub(1))];
    format!(
        "{weekday}, {:02} {month} {:04} {:02}:{:02}:{:02} GMT",
        dt.day(),
        dt.year(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    )
}

/// Converts a UTC `(year, month, day, hour, minute, second)` (year >= 1970) to Unix seconds.
pub(crate) fn unix_from_ymdhms(
    year: u64,
    month: u64,
    day: u64,
    hour: u64,
    minute: u64,
    second: u64,
) -> Option<u64> {
    if year < 1970 {
        return None;
    }
    let date = Date::from_calendar_date(
        i32::try_from(year).ok()?,
        Month::try_from(u8::try_from(month).ok()?).ok()?,
        u8::try_from(day).ok()?,
    )
    .ok()?;
    let second_clamped = if second == 60 { 59 } else { second };
    let dt = date
        .with_hms(
            u8::try_from(hour).ok()?,
            u8::try_from(minute).ok()?,
            u8::try_from(second_clamped).ok()?,
        )
        .ok()?
        .assume_utc();
    let ts = dt.unix_timestamp() + i64::from(second == 60);
    u64::try_from(ts).ok()
}
