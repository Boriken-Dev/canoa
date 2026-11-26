#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::path::Path;

use coqui_acp_api::usage::{Bucket, Coverage, Refill, Status, Unit};
use coqui_acp_api::{Spend, UsageReport};

use super::*;
use crate::config::Config;
use crate::usage_log::Turn;

/// 2026-11-25T12:00:00Z, a Wednesday.
const NOW: i64 = 1_795_608_000;
const DAY: i64 = 86_400;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-limits-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

const ACCOUNTS: &str = r#"
[host.accounts.work]
kind = "anthropic-api-key"
id = "id-work"
"#;

fn config(limits: &str) -> Config {
    Config::from_str_in(&format!("{ACCOUNTS}\n{limits}"), Path::new(".")).expect("parses")
}

fn limits_of(log: &Arc<UsageLog>, dir: &Path, text: &str) -> Arc<Limits> {
    let table = Table::of(&config(text));
    Limits::open(log, Some(dir.to_owned()), table, Box::new(|| None))
}

fn turn<'a>(account: &'a str, session: &'a str, profile: Option<&'a str>) -> Turn<'a> {
    Turn {
        account,
        harness: "claude",
        session,
        parent: None,
        model: Some("opus"),
        profile,
    }
}

fn spend(session: &str, tokens: u64, cost: Option<u64>) -> Spend {
    let mut spend = Spend::new(session);
    spend.input = tokens;
    spend.cost_micro_usd = cost;
    spend
}

fn bucket(id: &str, used: u64, limit: u64) -> Bucket {
    let mut bucket = Bucket::new(id);
    bucket.unit = Unit::Permille;
    bucket.used = Some(used);
    bucket.limit = Some(limit);
    bucket.status = Status::Ok;
    bucket.models = Coverage::All;
    bucket.refill = Refill::FixedWindow {
        length: None,
        resets_at: Some(NOW + DAY),
    };
    bucket.observed_at = NOW;
    bucket
}

fn report(buckets: Vec<Bucket>) -> UsageReport {
    let mut report = UsageReport::new();
    report.observed_at = NOW;
    report.buckets = buckets;
    report
}

fn read(limits: &Limits, log: &UsageLog, name: &str, context: &Context<'_>) -> Reading {
    let table = limits.table();
    let (found, limit) = table
        .limits
        .iter()
        .find(|(each, _)| each == name)
        .map(|(each, limit)| (each.as_str(), limit))
        .expect("the limit is configured");
    evaluate(log, &table, (found, limit), context, NOW)
}

fn read_plain(limits: &Limits, log: &UsageLog, name: &str) -> Reading {
    read(limits, log, name, &Context::default())
}

#[test]
fn recorded_tokens_and_money_count_in_the_window_for_the_scope() {
    let dir = folder("recorded");
    let log = Arc::new(UsageLog::open(dir.clone(), 400, NOW));
    let limits = limits_of(
        &log,
        &dir,
        r#"
[host.usage.limits.work_tokens]
account = "work"
tokens = 1000
window = "day"
[host.usage.limits.work_money]
account = "work"
usd = 10
window = "week"
stop_permille = 1000
[host.usage.limits.review_tokens]
profile = "review"
tokens = 1000
window = "day"
[host.usage.limits.everyone]
tokens = 4000
window = "month"
"#,
    );
    log.record(
        &turn("id-work", "s1", Some("review")),
        &spend("s1", 600, Some(4_000_000)),
        NOW - 60,
    );
    log.record(
        &turn("id-other", "s2", Some("default")),
        &spend("s2", 300, None),
        NOW - 30,
    );
    // Yesterday's spend is in the week and the month, not in the day.
    log.record(
        &turn("id-work", "s3", None),
        &spend("s3", 2000, Some(7_000_000)),
        NOW - DAY,
    );

    let tokens = read_plain(&limits, &log, "work_tokens");
    assert_eq!((tokens.used, tokens.permille), (Some(600), Some(600)));
    assert_eq!(tokens.state, State::Ok);
    let money = read_plain(&limits, &log, "work_money");
    assert_eq!(money.used, Some(11_000_000));
    assert_eq!(money.permille, Some(1100));
    assert_eq!(money.state, State::Stop);
    // A profile counts the lines that name it, and a line written with no profile is no profile's.
    let review = read_plain(&limits, &log, "review_tokens");
    assert_eq!(review.used, Some(600));
    let everyone = read_plain(&limits, &log, "everyone");
    assert_eq!(everyone.used, Some(2900));
    let (start, end) = everyone.window.expect("a window");
    assert_eq!(end - start, 30 * DAY);
}

#[test]
fn a_warn_threshold_of_a_tenth_below_the_stop_gives_the_three_states() {
    let dir = folder("states");
    let log = Arc::new(UsageLog::open(dir.clone(), 400, NOW));
    let limits = limits_of(
        &log,
        &dir,
        "[host.usage.limits.a]\naccount = \"work\"\ntokens = 1000\nwindow = \"day\"\nwarn_permille = 500\nstop_permille = 900\n",
    );
    assert_eq!(read_plain(&limits, &log, "a").state, State::Ok);
    log.record(
        &turn("id-work", "s", None),
        &spend("s", 500, None),
        NOW - 10,
    );
    assert_eq!(read_plain(&limits, &log, "a").state, State::Warn);
    log.record(&turn("id-work", "s", None), &spend("s", 900, None), NOW - 5);
    assert_eq!(read_plain(&limits, &log, "a").state, State::Stop);
}

#[test]
fn an_account_whose_agent_reports_no_cost_is_not_measured_and_one_that_spent_nothing_is_zero() {
    let dir = folder("nocost");
    let log = Arc::new(UsageLog::open(dir.clone(), 400, NOW));
    let limits = limits_of(
        &log,
        &dir,
        "[host.usage.limits.m]\naccount = \"work\"\nusd = 5\nwindow = \"day\"\nwarn_permille = 1\nstop_permille = 100\n",
    );
    let untouched = read_plain(&limits, &log, "m");
    assert_eq!((untouched.state, untouched.used), (State::Ok, Some(0)));
    log.record(
        &turn("id-work", "s", None),
        &spend("s", 500, None),
        NOW - 10,
    );
    let silent = read_plain(&limits, &log, "m");
    assert_eq!(silent.state, State::Unmeasured);
    assert_eq!(silent.permille, None);
    assert!(
        silent
            .why
            .as_deref()
            .is_some_and(|why| why.contains("no cost")),
        "{silent:?}"
    );
    // Once any line carries a cost, it is measured, and a day without one is 0.
    log.record(
        &turn("id-work", "t", None),
        &spend("t", 5, Some(1_000)),
        NOW - 3 * DAY,
    );
    assert_eq!(read_plain(&limits, &log, "m").state, State::Ok);
}

#[test]
fn a_bucket_is_counted_by_its_reported_fraction_and_exhausted_is_a_thousand() {
    let dir = folder("bucket");
    let log = Arc::new(UsageLog::open(dir.clone(), 400, NOW));
    let limits = limits_of(
        &log,
        &dir,
        r#"
[host.usage.limits.week]
account = "local:claude"
bucket = "seven_day"
stop_permille = 1000
[host.usage.limits.spare]
account = "local:claude"
bucket = "nothing_like_it"
"#,
    );
    log.note_report(
        "local:claude",
        Some("claude"),
        &report(vec![bucket("seven_day", 920, 1000)]),
        NOW,
    );
    let reading = read_plain(&limits, &log, "week");
    assert_eq!((reading.permille, reading.state), (Some(920), State::Warn));
    assert_eq!(reading.window.map(|(_, end)| end), Some(NOW + DAY));
    let mut exhausted = bucket("seven_day", 0, 0);
    exhausted.limit = None;
    exhausted.used = None;
    exhausted.status = Status::Exhausted;
    log.note_report(
        "local:claude",
        Some("claude"),
        &report(vec![exhausted]),
        NOW + 1,
    );
    let reading = read_plain(&limits, &log, "week");
    assert_eq!((reading.permille, reading.state), (Some(1000), State::Stop));
    // A bucket the account never reported is not measured.
    let missing = read_plain(&limits, &log, "spare");
    assert_eq!(missing.state, State::Unmeasured);
}

#[test]
fn any_takes_the_highest_bucket_that_covers_the_model_and_skips_the_disabled_and_the_reset() {
    let dir = folder("any");
    let log = Arc::new(UsageLog::open(dir.clone(), 400, NOW));
    let limits = limits_of(
        &log,
        &dir,
        "[host.usage.limits.all]\naccount = \"local:claude\"\nbucket = \"any\"\nstop_permille = 950\n",
    );
    let mut opus = bucket("opus_week", 700, 1000);
    opus.models = Coverage::Only {
        models: vec!["opus".to_owned()],
    };
    let mut sonnet = bucket("sonnet_week", 990, 1000);
    sonnet.models = Coverage::Only {
        models: vec!["sonnet".to_owned()],
    };
    let mut off = bucket("off", 1000, 1000);
    off.status = Status::Disabled;
    let mut past = bucket("past", 1000, 1000);
    past.refill = Refill::FixedWindow {
        length: None,
        resets_at: Some(NOW - 5),
    };
    log.note_report(
        "local:claude",
        Some("claude"),
        &report(vec![
            opus,
            sonnet,
            off,
            past,
            bucket("five_hour", 300, 1000),
        ]),
        NOW,
    );
    let for_opus = Context {
        model: Some("opus"),
        ..Context::default()
    };
    let reading = read(&limits, &log, "all", &for_opus);
    assert_eq!(reading.bucket.as_deref(), Some("opus_week"));
    assert_eq!(reading.state, State::Ok);
    // With no model known, every bucket that is not disabled or reset counts, so the sonnet one decides.
    let reading = read_plain(&limits, &log, "all");
    assert_eq!(reading.bucket.as_deref(), Some("sonnet_week"));
    assert_eq!(reading.state, State::Stop);
    // Only a reset bucket: nothing to count until it is reported again.
    log.note_report(
        "local:claude",
        Some("claude"),
        &report(vec![{
            let mut only = bucket("only", 1000, 1000);
            only.refill = Refill::FixedWindow {
                length: None,
                resets_at: Some(NOW - 1),
            };
            only
        }]),
        NOW + 1,
    );
    let reading = read_plain(&limits, &log, "all");
    assert_eq!(reading.state, State::Unmeasured);
    assert_eq!(reading.why.as_deref(), Some("reset, not yet reported"));
}

#[test]
fn a_limit_is_announced_once_in_its_window_and_not_again_after_a_restart() {
    struct Seen(Mutex<Vec<(EventKind, String)>>);
    impl Announce for Seen {
        fn raise(&self, event: &LimitEvent) {
            lock(&self.0).push((event.kind, event.text.clone()));
        }
    }
    let dir = folder("announce");
    let text = "[host.usage.limits.day]\naccount = \"work\"\ntokens = 1000\nwindow = \"day\"\nwarn_permille = 500\nstop_permille = 900\n";
    let log = Arc::new(UsageLog::open(dir.clone(), 400, NOW));
    let limits = limits_of(&log, &dir, text);
    let seen = Arc::new(Seen(Mutex::new(Vec::new())));
    limits.announce_to(Arc::clone(&seen) as Arc<dyn Announce>);
    // Nothing used: nothing to say.
    limits.state();
    assert!(lock(&seen.0).is_empty());
    let now = crate::usage_splice::unix_now();
    log.record(&turn("id-work", "s", None), &spend("s", 600, None), now);
    limits.state();
    limits.state();
    assert_eq!(
        lock(&seen.0).as_slice(),
        [(EventKind::Warn, "Limit day is at 60 percent".to_owned())]
    );
    log.record(&turn("id-work", "s", None), &spend("s", 950, None), now);
    limits.state();
    limits.state();
    let told = lock(&seen.0).clone();
    assert_eq!(told.len(), 2, "{told:?}");
    assert_eq!(told[1].0, EventKind::Stop);

    // A host that starts again over the same folder says nothing more in the same window.
    drop(limits);
    let again = limits_of(&log, &dir, text);
    let after = Arc::new(Seen(Mutex::new(Vec::new())));
    again.announce_to(Arc::clone(&after) as Arc<dyn Announce>);
    again.state();
    assert!(lock(&after.0).is_empty());
}

#[test]
fn a_stop_refuses_the_turn_with_the_reason_until_the_person_lets_one_through() {
    let dir = folder("gate");
    let now = crate::usage_splice::unix_now();
    let log = Arc::new(UsageLog::open(dir.clone(), 400, now));
    let limits = limits_of(
        &log,
        &dir,
        r#"
[host.usage.limits.cap]
account = "work"
tokens = 1000
window = "day"
stop_permille = 1000
[host.usage.limits.elsewhere]
profile = "other"
tokens = 1
window = "day"
warn_permille = 0
stop_permille = 1
"#,
    );
    let work = Asking {
        session: "s1",
        context: Context {
            account: Some("id-work"),
            profile: Some("review"),
            model: None,
        },
    };
    assert_eq!(limits.gate(&work), Gate::default());
    log.record(
        &turn("id-work", "s1", Some("review")),
        &spend("s1", 1500, None),
        now,
    );
    let gate = limits.gate(&work);
    let refused = gate.refused.expect("the turn is refused");
    assert_eq!(refused.limit, "cap");
    assert!(
        refused.text.contains("Limit cap stopped this turn"),
        "{}",
        refused.text
    );
    assert_eq!(refused.meta["limit"], "cap");
    assert_eq!(refused.meta["used_permille"], 1500);
    assert_eq!(refused.meta["scope"], "account");
    assert!(refused.meta["resets_at"].is_i64(), "{}", refused.meta);
    // A session of another account or profile is not in the scope of either limit.
    let other = Asking {
        session: "s2",
        context: Context {
            account: Some("id-other"),
            profile: Some("review"),
            model: None,
        },
    };
    assert_eq!(limits.gate(&other), Gate::default());
    // Letting one through is for that turn only: the next is refused again.
    limits.allow("cap", Some("s1")).expect("the limit exists");
    let through = limits.gate(&work);
    assert!(through.refused.is_none(), "{through:?}");
    assert_eq!(through.warnings.len(), 1);
    assert!(limits.gate(&work).refused.is_some());
    // An allowance for another session does not open this one.
    limits.allow("cap", Some("s9")).expect("the limit exists");
    assert!(limits.gate(&work).refused.is_some());
    assert!(limits.allow("nothing", None).is_err());
}

#[test]
fn a_warning_is_owed_to_a_session_in_scope_and_a_host_with_no_limits_changes_nothing() {
    let dir = folder("warn");
    let now = crate::usage_splice::unix_now();
    let log = Arc::new(UsageLog::open(dir.clone(), 400, now));
    let none = limits_of(&log, &dir, "");
    let asking = Asking {
        session: "s",
        context: Context {
            account: Some("id-work"),
            profile: None,
            model: None,
        },
    };
    log.record(
        &turn("id-work", "s", None),
        &spend("s", 5_000_000, None),
        now,
    );
    assert!(!none.any());
    assert_eq!(none.gate(&asking), Gate::default());
    assert!(none.state().is_empty());

    let dir = folder("warn2");
    let log = Arc::new(UsageLog::open(dir.clone(), 400, now));
    let limits = limits_of(
        &log,
        &dir,
        "[host.usage.limits.w]\nprofile = \"default\"\ntokens = 1000\nwindow = \"week\"\n",
    );
    log.record(
        &turn("id-work", "s", Some("default")),
        &spend("s", 920, None),
        now,
    );
    let gate = limits.gate(&asking);
    assert!(gate.refused.is_none());
    assert_eq!(gate.warnings.len(), 1);
    assert!(
        gate.warnings[0]
            .text
            .starts_with("Limit w: 92 percent used (920 of 1000 tokens), resets in"),
        "{}",
        gate.warnings[0].text
    );
}

#[test]
fn a_new_window_starts_over() {
    let dir = folder("window");
    let log = Arc::new(UsageLog::open(dir.clone(), 400, NOW));
    let limits = limits_of(
        &log,
        &dir,
        "[host.usage.limits.d]\naccount = \"work\"\ntokens = 100\nwindow = \"day\"\nwarn_permille = 1\nstop_permille = 100\n",
    );
    log.record(
        &turn("id-work", "s", None),
        &spend("s", 100, None),
        NOW - 10,
    );
    let today = read_plain(&limits, &log, "d");
    assert_eq!(today.state, State::Stop);
    let table = limits.table();
    let (name, limit) = &table.limits[0];
    let tomorrow = evaluate(&log, &table, (name, limit), &Context::default(), NOW + DAY);
    assert_eq!((tomorrow.state, tomorrow.used), (State::Ok, Some(0)));
    assert_ne!(today.window_key(), tomorrow.window_key());
}
