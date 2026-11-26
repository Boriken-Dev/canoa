//! `[host.usage.limits.NAME]` and the `[usage] warn_permille` the host reads beside it.

use super::{Config, DEFAULT_WARN_PERMILLE, LimitFigure, LimitScope, LimitWindow};
use std::path::Path;

fn parse(text: &str) -> Result<Config, crate::ConfigError> {
    Config::from_str_in(text, Path::new("."))
}

fn refused(text: &str) -> String {
    parse(text).expect_err("is refused").to_string()
}

#[test]
fn a_file_with_no_limits_has_none_and_warns_at_the_default() {
    let none = parse("").expect("parses");
    assert!(none.usage.limits.is_empty());
    assert_eq!(none.usage.warn_permille, DEFAULT_WARN_PERMILLE);
    let host = parse("[host.usage]\nkeep_days = 30\n").expect("parses");
    assert!(host.usage.limits.is_empty());
    let set = parse("[usage]\nwarn_permille = 650\n").expect("parses");
    assert_eq!(set.usage.warn_permille, 650);
    assert!(parse("[usage]\nwarn_permille = 1001\n").is_err());
}

#[test]
fn every_figure_and_scope_is_read() {
    let config = parse(
        r#"
[usage]
warn_permille = 700

[host.usage.limits.work_week]
account = "work"
usd = 25.5
window = "week"
stop_permille = 1200

[host.usage.limits.team-day]
profile = "review"
tokens = 5000000
window = "day"
warn_permille = 500

[host.usage.limits.plan]
bucket = "seven_day"
account = "local:claude"

[host.usage.limits.any_bucket]
bucket = "any"
stop_permille = 1000

[host.usage.limits.whole]
usd = 3
window = "month"
"#,
    )
    .expect("parses");
    let limits = &config.usage.limits;
    assert_eq!(limits.len(), 5);
    let week = &limits["work_week"];
    assert_eq!(week.scope, LimitScope::Account("work".into()));
    assert_eq!(
        week.figure,
        LimitFigure::Usd {
            micro: 25_500_000,
            window: LimitWindow::Week
        }
    );
    assert_eq!((week.warn_permille, week.stop_permille), (700, Some(1200)));
    let day = &limits["team-day"];
    assert_eq!(day.scope, LimitScope::Profile("review".into()));
    assert_eq!(
        day.figure,
        LimitFigure::Tokens {
            count: 5_000_000,
            window: LimitWindow::Day
        }
    );
    assert_eq!((day.warn_permille, day.stop_permille), (500, None));
    assert_eq!(
        limits["plan"].scope,
        LimitScope::Account("local:claude".into())
    );
    assert_eq!(limits["any_bucket"].scope, LimitScope::Everyone);
    assert_eq!(
        limits["whole"].figure,
        LimitFigure::Usd {
            micro: 3_000_000,
            window: LimitWindow::Month
        }
    );
}

#[test]
fn a_limit_that_names_two_scopes_two_figures_or_a_stray_key_is_refused_naming_the_key() {
    let cases = [
        (
            "account = \"a\"\nprofile = \"b\"\ntokens = 1\nwindow = \"day\"",
            "two scopes",
        ),
        ("usd = 1.0\ntokens = 1\nwindow = \"day\"", "exactly one"),
        ("window = \"day\"", "exactly one"),
        ("usd = 1.0", "needs a window"),
        ("tokens = 5", "needs a window"),
        (
            "bucket = \"any\"\nwindow = \"day\"",
            "window goes with usd or tokens",
        ),
        ("bucket = \"any\"\nprofile = \"p\"", "belongs to an account"),
        ("tokens = 5\nwindow = \"hour\"", "day, week or month"),
        ("usd = 0.0\nwindow = \"day\"", "more than 0"),
        ("usd = -2.0\nwindow = \"day\"", "more than 0"),
        ("tokens = 0\nwindow = \"day\"", "more than 0"),
        (
            "tokens = 5\nwindow = \"day\"\nwarn_permille = 900\nstop_permille = 800",
            "at least warn_permille",
        ),
        (
            "tokens = 5\nwindow = \"day\"\nstop_permille = 700",
            "at least warn_permille",
        ),
        (
            "tokens = 5\nwindow = \"day\"\nsurprise = 1",
            "host.usage.limits.one",
        ),
    ];
    for (body, said) in cases {
        let why = refused(&format!("[host.usage.limits.one]\n{body}\n"));
        assert!(why.contains(said), "{body}: {why}");
    }
    for name in [
        "Upper",
        "has space",
        "",
        "a-name-of-more-than-thirty-two-letters",
    ] {
        let text = format!("[host.usage.limits.\"{name}\"]\ntokens = 5\nwindow = \"day\"\n");
        assert!(parse(&text).is_err(), "{name:?}");
    }
}

#[test]
fn a_stop_may_be_over_a_thousand_and_a_decimal_has_six_places() {
    let config =
        parse("[host.usage.limits.a]\nusd = 0.000001\nwindow = \"day\"\nstop_permille = 5000\n")
            .expect("parses");
    let limit = &config.usage.limits["a"];
    assert_eq!(limit.stop_permille, Some(5000));
    assert_eq!(
        limit.figure,
        LimitFigure::Usd {
            micro: 1,
            window: LimitWindow::Day
        }
    );
}

#[test]
fn windows_are_utc_days_weeks_from_monday_and_calendar_months() {
    // 2026-11-27 (a Friday) 13:00:00 UTC.
    let now = 1_795_784_400;
    assert_eq!(LimitWindow::Day.bounds(now), (1_795_737_600, 1_795_824_000));
    assert_eq!(
        LimitWindow::Week.bounds(now),
        (1_795_392_000, 1_795_996_800)
    );
    assert_eq!(
        LimitWindow::Month.bounds(now),
        (1_793_491_200, 1_796_083_200)
    );
    // The week starts on a Monday: the Sunday before belongs to the week before.
    let sunday = 1_795_392_000 - 1;
    assert_eq!(LimitWindow::Week.bounds(sunday).1, 1_795_392_000);
    // A December rolls over into January.
    let december = 1_796_083_200 + 86_400;
    assert_eq!(LimitWindow::Month.bounds(december).1, 1_798_761_600);
}
