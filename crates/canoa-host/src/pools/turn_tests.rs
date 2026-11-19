//! Which account of a pool takes a turn, with stand-in accounts and stand-in usage.

use std::time::Duration;

use coqui_acp_api::{Bucket, CacheLifetime, Coverage, Refill, Status, UsageReport};

use super::*;

const NOW: i64 = 1_800_000_000;

fn account(name: &str) -> Candidate {
    Candidate {
        name: name.to_owned(),
        ..Candidate::default()
    }
}

/// An account whose provider reports one bucket, in `status`, covering `models`, ending at `resets_at` when it is a window.
fn reporting(name: &str, status: Status, models: Coverage, resets_at: Option<i64>) -> Candidate {
    let mut bucket = Bucket::new("five_hour");
    bucket.status = status;
    bucket.models = models;
    bucket.refill = Refill::FixedWindow {
        length: None,
        resets_at,
    };
    let mut report = UsageReport::new();
    report.supported = true;
    report.buckets = vec![bucket];
    Candidate {
        usage: Some(report),
        ..account(name)
    }
}

fn exhausted(name: &str, resets_at: i64) -> Candidate {
    reporting(name, Status::Exhausted, Coverage::Unknown, Some(resets_at))
}

fn at<'a>(current: Option<&'a str>, idle: Option<u64>, lifetime: CacheLifetime) -> Turn<'a> {
    Turn {
        current,
        model: None,
        idle: idle.map(Duration::from_secs),
        lifetime,
        now: NOW,
    }
}

fn taken(pool: &str, cursor: &Cursor, accounts: &[Candidate], turn: &Turn<'_>) -> Chosen {
    decide(pool, cursor, accounts, turn).expect("an account takes it")
}

#[test]
fn sessions_that_start_take_the_accounts_in_turn() {
    let cursor = Cursor::default();
    let accounts = [account("a"), account("b"), account("c")];
    let start = at(None, None, CacheLifetime::Long);
    let order: Vec<String> = (0..4)
        .map(|_| taken("pool", &cursor, &accounts, &start).account)
        .collect();
    assert_eq!(order, ["a", "b", "c", "a"]);
    // Another pool has its own turn.
    assert_eq!(taken("other", &cursor, &accounts, &start).account, "a");
}

#[test]
fn a_warm_cache_keeps_its_account_and_a_cold_one_is_the_next_turns() {
    let cursor = Cursor::default();
    let accounts = [account("a"), account("b"), account("c")];
    // Within the long cache's hour the session stays, however often others start.
    let first = taken(
        "pool",
        &cursor,
        &accounts,
        &at(None, None, CacheLifetime::Long),
    );
    assert_eq!((first.account.as_str(), first.why), ("a", Why::Start));
    let warm = at(Some("a"), Some(59 * 60), CacheLifetime::Long);
    for _ in 0..3 {
        let kept = taken("pool", &cursor, &accounts, &warm);
        assert_eq!(
            (kept.account.as_str(), kept.moved, kept.why),
            ("a", false, Why::Kept)
        );
    }
    // Past it, the turn goes to the next account, which is a move.
    let cold = at(Some("a"), Some(61 * 60), CacheLifetime::Long);
    let next = taken("pool", &cursor, &accounts, &cold);
    assert_eq!(
        (next.account.as_str(), next.moved, next.why),
        ("b", true, Why::InTurn)
    );
    // The short cache and the agent's own are about five minutes, and a cache that is off is never warm.
    let short = at(Some("b"), Some(301), CacheLifetime::Short);
    assert_eq!(taken("pool", &cursor, &accounts, &short).account, "c");
    let default = at(Some("c"), Some(120), CacheLifetime::Default);
    assert_eq!(taken("pool", &cursor, &accounts, &default).account, "c");
    let off = at(Some("c"), Some(1), CacheLifetime::Off);
    let rotated = taken("pool", &cursor, &accounts, &off);
    assert_eq!((rotated.account.as_str(), rotated.moved), ("a", true));
}

#[test]
fn an_account_out_of_usage_is_left_and_the_session_moves_on_even_with_a_warm_cache() {
    let cursor = Cursor::default();
    let accounts = [exhausted("a", NOW + 600), account("b"), account("c")];
    // A new session does not start on it.
    let start = taken(
        "pool",
        &cursor,
        &accounts,
        &at(None, None, CacheLifetime::Long),
    );
    assert_eq!(start.account, "b");
    // A session on it, with a cache that is warm, moves and says what ran out.
    let warm = at(Some("a"), Some(5), CacheLifetime::Long);
    let moved = taken("pool", &cursor, &accounts, &warm);
    assert_eq!(
        (moved.account.as_str(), moved.moved, moved.why),
        ("c", true, Why::Exhausted)
    );
    assert_eq!(moved.left.as_deref(), Some("five_hour is used up"));
    // A turn a limit refused holds an account back as well, until the time it said, or until the client says it is over.
    let limited = Candidate {
        limited_until: Some(NOW + 30),
        ..account("b")
    };
    let accounts = [limited, account("c")];
    let on_b = at(Some("b"), Some(5), CacheLifetime::Long);
    assert_eq!(taken("limited", &cursor, &accounts, &on_b).account, "c");
    let over = Turn {
        now: NOW + 31,
        ..on_b
    };
    assert_eq!(taken("limited", &cursor, &accounts, &over).account, "b");
    let without_a_time = Candidate {
        limited_until: Some(i64::MAX),
        ..account("b")
    };
    let accounts = [without_a_time, account("c")];
    assert_eq!(taken("untimed", &cursor, &accounts, &on_b).account, "c");
}

#[test]
fn only_a_bucket_that_covers_the_model_holds_an_account_back() {
    let cursor = Cursor::default();
    let opus_only = reporting(
        "a",
        Status::Exhausted,
        Coverage::Only {
            models: vec!["opus".to_owned()],
        },
        Some(NOW + 600),
    );
    let accounts = [opus_only, account("b")];
    let turn = |model: Option<&'static str>| Turn {
        model,
        ..at(None, None, CacheLifetime::Long)
    };
    // The account is out for opus and in for the other model; with no model chosen nothing says it is out.
    assert_eq!(
        taken("opus", &cursor, &accounts, &turn(Some("opus"))).account,
        "b"
    );
    assert_eq!(
        taken("sonnet", &cursor, &accounts, &turn(Some("sonnet"))).account,
        "a"
    );
    assert_eq!(taken("none", &cursor, &accounts, &turn(None)).account, "a");
    // A bucket whose coverage is not said covers every model, and so does one that covers all of them.
    for coverage in [Coverage::Unknown, Coverage::All] {
        let out = reporting("a", Status::Exhausted, coverage, Some(NOW + 600));
        let accounts = [out, account("b")];
        assert_eq!(
            taken("any", &Cursor::default(), &accounts, &turn(Some("sonnet"))).account,
            "b"
        );
    }
    // A bucket that is warning, or ok, or that has refilled since the report, leaves the account in.
    for (status, resets_at) in [
        (Status::Warning, Some(NOW + 600)),
        (Status::Ok, None),
        (Status::Exhausted, Some(NOW - 1)),
    ] {
        let reported = reporting("a", status, Coverage::All, resets_at);
        let accounts = [reported, account("b")];
        assert_eq!(
            taken("in", &Cursor::default(), &accounts, &turn(None)).account,
            "a"
        );
    }
}

#[test]
fn an_account_that_does_not_list_the_model_or_is_held_is_never_chosen() {
    let cursor = Cursor::default();
    let unlisted = Candidate {
        unavailable: Some("it lists no such model".to_owned()),
        ..account("a")
    };
    let accounts = [unlisted, account("b")];
    for _ in 0..3 {
        let chosen = taken(
            "pool",
            &cursor,
            &accounts,
            &at(None, None, CacheLifetime::Long),
        );
        assert_eq!(chosen.account, "b");
    }
}

#[test]
fn with_every_account_out_the_refusal_names_when_the_soonest_allowance_refills() {
    let cursor = Cursor::default();
    // 2h10m after `NOW`, which is 2027-01-15 08:00 UTC, is 10:10; the other refills a day later.
    let accounts = [exhausted("a", NOW + 7800), exhausted("b", NOW + 86_400)];
    let why = decide(
        "pool",
        &cursor,
        &accounts,
        &at(None, None, CacheLifetime::Long),
    )
    .expect_err("none has usage left");
    assert!(
        why.contains("pool pool") && why.contains("a: five_hour is used up"),
        "{why}"
    );
    assert!(
        why.contains("in 2 h 10 min") && why.contains("2027-01-15 10:10 UTC"),
        "{why}"
    );
    // An allowance that says no time is said too.
    let mut untimed = exhausted("a", 0);
    untimed.usage.as_mut().expect("usage").buckets[0].refill = Refill::Unknown;
    let why = decide(
        "pool",
        &cursor,
        &[untimed],
        &at(None, None, CacheLifetime::Long),
    )
    .expect_err("none");
    assert!(why.contains("nothing says when"), "{why}");
    // When none lists the model the refusal says that, and not that usage is out.
    let none = Candidate {
        unavailable: Some("it lists no such model".to_owned()),
        ..account("a")
    };
    let why = decide(
        "pool",
        &cursor,
        &[none],
        &at(None, None, CacheLifetime::Long),
    )
    .expect_err("none");
    assert!(
        why.contains("a: it lists no such model") && !why.contains("refills"),
        "{why}"
    );
    let empty =
        decide("pool", &cursor, &[], &at(None, None, CacheLifetime::Long)).expect_err("none");
    assert!(empty.contains("holds no account"), "{empty}");
}

#[test]
fn times_are_said_in_utc() {
    assert_eq!(utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(utc(1_800_000_000), "2027-01-15 08:00 UTC");
    assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
    assert_eq!(after(0), "in 0 min");
    assert_eq!(after(61), "in 2 min");
    assert_eq!(after(3600), "in 1 h");
}
