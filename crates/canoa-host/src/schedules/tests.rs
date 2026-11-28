#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::Path;

use serde_json::json;
use time::{Date, Month, PrimitiveDateTime, Time};

use super::{Due, Last, MISSED_AFTER_MS, due, next_after, note_of};
use crate::config::{Config, ScheduleConfig};

/// The one task of a configuration made of `table`'s lines.
fn task(lines: &str) -> ScheduleConfig {
    // Read in UTC unless the lines name a zone, so what is expected does not depend on the computer.
    let zone = if lines.contains("time_zone") {
        ""
    } else {
        "time_zone = \"UTC\"\n"
    };
    let text = format!(
        "[schedules.t]\nentry = \"claude\"\nfolder = \"/work\"\nprompt = \"go\"\n{zone}{lines}\n"
    );
    Config::from_str_in(&text, Path::new(""))
        .expect("parses")
        .schedules
        .get("t")
        .expect("held")
        .clone()
}

fn refused(lines: &str) -> String {
    let text = format!(
        "[schedules.t]\nentry = \"claude\"\nfolder = \"/work\"\nprompt = \"go\"\n{lines}\n"
    );
    Config::from_str_in(&text, Path::new(""))
        .expect_err("refused")
        .to_string()
}

/// A UTC time as milliseconds since the Unix epoch.
fn utc(year: i32, month: Month, day: u8, hour: u8, minute: u8) -> u64 {
    let date = Date::from_calendar_date(year, month, day).expect("a date");
    let at = PrimitiveDateTime::new(date, Time::from_hms(hour, minute, 0).expect("a time"));
    u64::try_from(at.assume_utc().unix_timestamp() * 1000).expect("after the epoch")
}

fn last(at: u64) -> Last {
    Last {
        at,
        result: "completed".to_owned(),
        note: String::new(),
        session: None,
    }
}

#[test]
fn a_daily_task_comes_at_its_time_on_its_own_clock() {
    // 09:30 in Puerto Rico, four hours behind UTC all year, is 13:30 UTC.
    let task = task("at = \"09:30\"\ntime_zone = \"America/Puerto_Rico\"");
    let morning = utc(2026, Month::December, 1, 8, 0);
    assert_eq!(
        next_after(&task, None, morning),
        Some(utc(2026, Month::December, 1, 13, 30))
    );
    let after = utc(2026, Month::December, 1, 13, 30);
    assert_eq!(
        next_after(&task, None, after),
        Some(utc(2026, Month::December, 2, 13, 30))
    );
}

#[test]
fn a_task_stays_at_its_wall_clock_time_across_a_daylight_saving_change() {
    // New York is five hours behind UTC in winter and four in summer: the US changed on 2026-03-08 and changes back on 2026-11-01.
    let task = task("at = \"09:30\"\ntime_zone = \"America/New_York\"");
    let saturday = utc(2026, Month::March, 7, 14, 30);
    assert_eq!(
        next_after(&task, None, saturday),
        Some(utc(2026, Month::March, 8, 13, 30)),
        "the morning after the clocks went forward is an hour earlier in UTC"
    );
    let friday = utc(2026, Month::October, 31, 13, 30);
    assert_eq!(
        next_after(&task, None, friday),
        Some(utc(2026, Month::November, 1, 14, 30)),
        "and the morning after they went back an hour later"
    );
    // The time a change skips (02:30 on the day the clocks go forward) comes at the same wall time after it.
    let skipped = self::task("at = \"02:30\"\ntime_zone = \"America/New_York\"");
    let night = utc(2026, Month::March, 8, 6, 0);
    assert_eq!(
        next_after(&skipped, None, night),
        Some(utc(2026, Month::March, 8, 7, 30))
    );
}

#[test]
fn a_task_that_names_no_zone_is_read_in_the_computers_own() {
    let task = task("at = \"09:30\"");
    assert!(task.time_zone.is_some(), "the helper reads these in UTC");
    let text = "[schedules.t]\nentry = \"a\"\nfolder = \"/w\"\nprompt = \"p\"\nat = \"09:30\"\n";
    let own = Config::from_str_in(text, Path::new(""))
        .expect("parses")
        .schedules["t"]
        .clone();
    assert_eq!(own.time_zone, None);
    let now = utc(2026, Month::June, 1, 0, 0);
    let next = next_after(&own, None, now).expect("a next time");
    let zone = jiff::tz::TimeZone::system();
    let local = jiff::Timestamp::from_millisecond(i64::try_from(next).expect("fits"))
        .expect("a time")
        .to_zoned(zone);
    assert_eq!((local.hour(), local.minute()), (9, 30));
}

#[test]
fn a_weekly_task_comes_on_its_days_only() {
    // 2026-12-01 is a Tuesday.
    let task = task("repeat = \"weekly\"\nat = \"07:00\"\nweekdays = [\"mon\", \"fri\"]");
    let tuesday = utc(2026, Month::December, 1, 8, 0);
    assert_eq!(
        next_after(&task, None, tuesday),
        Some(utc(2026, Month::December, 4, 7, 0))
    );
}

#[test]
fn a_task_is_due_when_its_minute_passed_since_the_last_check() {
    let task = task("at = \"09:30\"");
    let before = utc(2026, Month::December, 1, 9, 29) + 40_000;
    let after = utc(2026, Month::December, 1, 9, 30) + 5_000;
    assert_eq!(
        due(&task, None, before, before + 10_000),
        Due::No,
        "still before the minute"
    );
    assert_eq!(due(&task, None, before, after), Due::Run);
    assert_eq!(
        due(&task, None, after, after + 15_000),
        Due::No,
        "the minute is not run twice"
    );
}

#[test]
fn a_one_time_task_runs_once_and_late_ones_are_missed_not_run() {
    let task = task("repeat = \"once\"\ndate = \"2026-12-01\"\nat = \"09:30\"");
    let at = utc(2026, Month::December, 1, 9, 30);
    assert_eq!(next_after(&task, None, at - 60_000), Some(at));
    assert_eq!(due(&task, None, 0, at - 1), Due::No);
    assert_eq!(due(&task, None, 0, at + 20_000), Due::Run);
    assert_eq!(due(&task, None, 0, at + MISSED_AFTER_MS + 1), Due::Missed);
    let ran = last(at + 20_000);
    assert_eq!(
        due(&task, Some(&ran), 0, at + 40_000),
        Due::No,
        "it has run"
    );
    assert_eq!(next_after(&task, Some(&ran), at - 60_000), None);
    let early = last(at - 3_600_000);
    assert_eq!(
        due(&task, Some(&early), 0, at + 20_000),
        Due::Run,
        "a run before its time is not its run"
    );
}

#[test]
fn what_a_task_says_is_checked_when_the_file_is_read() {
    assert!(refused("at = \"9\"").contains("at is a time of day"));
    assert!(refused("at = \"24:00\"").contains("at is a time of day"));
    assert!(refused("repeat = \"hourly\"\nat = \"09:00\"").contains("once, daily or weekly"));
    assert!(refused("repeat = \"once\"\nat = \"09:00\"").contains("names no date"));
    assert!(
        refused("repeat = \"once\"\ndate = \"2026-02-30\"\nat = \"09:00\"")
            .contains("date is a day")
    );
    assert!(refused("at = \"09:00\"\ndate = \"2026-12-01\"").contains("date goes with"));
    assert!(refused("repeat = \"weekly\"\nat = \"09:00\"").contains("names no weekdays"));
    assert!(
        refused("repeat = \"weekly\"\nat = \"09:00\"\nweekdays = [\"someday\"]")
            .contains("weekdays names days")
    );
    assert!(refused("at = \"09:00\"\nweekdays = [\"mon\"]").contains("weekdays goes with"));
    assert!(
        refused("at = \"09:00\"\ntime_zone = \"Nowhere/Land\"")
            .contains("time_zone is no IANA zone")
    );
    assert!(
        !refused(
            "at = \"09:00\"
utc_offset_minutes = 60"
        )
        .is_empty(),
        "the fixed offset is gone"
    );
    assert!(refused("at = \"09:00\"\ntimeout_minutes = 0").contains("timeout_minutes"));
    assert!(!refused("at = \"09:00\"\nsurprise = 1").is_empty());
    let text = "[schedules.Bad]\nentry = \"a\"\nfolder = \"/w\"\nprompt = \"p\"\nat = \"09:00\"\n";
    let why = Config::from_str_in(text, Path::new(""))
        .expect_err("refused")
        .to_string();
    assert!(why.contains("is not a task name"), "{why}");
}

#[test]
fn a_task_names_what_it_needs() {
    for key in ["entry", "folder", "prompt"] {
        let kept: Vec<String> = ["entry", "folder", "prompt"]
            .into_iter()
            .filter(|each| *each != key)
            .map(|each| format!("{each} = \"x\"\n"))
            .collect();
        let text = format!("[schedules.t]\nat = \"09:00\"\n{}", kept.concat());
        let why = Config::from_str_in(&text, Path::new(""))
            .expect_err("refused")
            .to_string();
        assert!(why.contains(&format!("names no {key}")), "{why}");
    }
}

#[test]
fn a_task_the_control_channel_sets_is_checked_by_the_reader() {
    let good = json!({"entry": "claude", "folder": "/w", "prompt": "p", "at": "09:00"});
    assert!(Config::check_schedule("nightly", &good).is_ok());
    let bad = json!({"entry": "claude", "folder": "/w", "prompt": "p", "at": "later"});
    assert!(Config::check_schedule("nightly", &bad).is_err());
    let unknown =
        json!({"entry": "claude", "folder": "/w", "prompt": "p", "at": "09:00", "when": "now"});
    assert!(Config::check_schedule("nightly", &unknown).is_err());
}

#[test]
fn a_note_is_one_line_cut_to_what_it_holds() {
    assert_eq!(note_of("  two\n\nlines  here "), "two lines here");
    let long = note_of(&"word ".repeat(200));
    assert!(long.ends_with('…'));
    assert_eq!(long.chars().count(), super::NOTE_CHARS + 1);
}

#[test]
fn a_home_with_no_configuration_file_has_no_tasks_and_nothing_to_complain_of() {
    let dir = std::env::temp_dir().join(format!("canoa-schedules-absent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let schedules = super::Schedules::new(crate::config::Layers::only(dir.join("canoa.cfg")), None);
    assert!(schedules.read_tasks().unwrap().is_none());

    std::fs::write(
        dir.join("canoa.cfg"),
        "[schedules.t
",
    )
    .unwrap();
    assert!(schedules.read_tasks().is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
