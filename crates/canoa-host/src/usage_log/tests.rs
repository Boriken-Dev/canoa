#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use coqui_acp_api::{Bucket, ModelSpend};

use super::*;

/// 2026-11-16T12:00:00Z.
const NOW: i64 = 1_794_830_400;
const DAY: i64 = SECONDS_A_DAY;

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-usage-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn spend(session: &str, input: u64, output: u64, cost: Option<u64>) -> Spend {
    let mut spend = Spend::new(session);
    spend.input = input;
    spend.output = output;
    spend.cost_micro_usd = cost;
    spend
}

fn turn<'a>(account: &'a str, session: &'a str) -> Turn<'a> {
    Turn {
        account,
        harness: "claude",
        session,
        parent: None,
        model: Some("opus"),
    }
}

fn lines(dir: &Path, name: &str) -> Vec<Value> {
    fs::read_to_string(dir.join(name))
        .map(|text| {
            text.lines()
                .map(|line| serde_json::from_str(line).expect("a JSON line"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_cumulative_report_becomes_one_delta_a_turn() {
    let dir = folder("delta");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    log.record(&turn("work", "s1"), &spend("s1", 100, 20, Some(5_000)), NOW);
    log.record(
        &turn("work", "s1"),
        &spend("s1", 250, 70, Some(12_000)),
        NOW + 60,
    );
    let rows = lines(&dir, "2026-11.jsonl");
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["input"], 100);
    assert_eq!(rows[1]["input"], 150);
    assert_eq!(rows[1]["output"], 50);
    assert_eq!(rows[1]["cost_micro_usd"], 7_000);
    assert_eq!(rows[1]["account"], "work");
    assert_eq!(rows[1]["harness"], "claude");
    assert_eq!(rows[1]["model"], "opus");
    assert_eq!(rows[1]["session_id"], "s1");
    assert_eq!(rows[1]["time"], NOW + 60);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_report_seen_again_adds_nothing_even_after_the_host_restarts() {
    let dir = folder("again");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    log.record(&turn("work", "s1"), &spend("s1", 100, 20, None), NOW);
    log.record(&turn("work", "s1"), &spend("s1", 100, 20, None), NOW + 1);
    assert_eq!(lines(&dir, "2026-11.jsonl").len(), 1);
    drop(log);
    // A session loaded again after a restart reports its whole total.
    let log = UsageLog::open(dir.clone(), 400, NOW + 10);
    log.record(&turn("work", "s1"), &spend("s1", 100, 20, None), NOW + 10);
    assert_eq!(lines(&dir, "2026-11.jsonl").len(), 1);
    log.record(&turn("work", "s1"), &spend("s1", 130, 20, None), NOW + 20);
    let rows = lines(&dir, "2026-11.jsonl");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["input"], 30);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_counter_the_agent_never_moved_and_a_cost_it_never_gave_stay_absent() {
    let dir = folder("absent");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    log.record(&turn("work", "s1"), &spend("s1", 10, 4, None), NOW);
    let rows = lines(&dir, "2026-11.jsonl");
    let row = rows[0].as_object().expect("an object");
    for absent in [
        "cache_read",
        "cache_write",
        "cost_micro_usd",
        "parent_session_id",
    ] {
        assert!(!row.contains_key(absent), "{absent} in {row:?}");
    }
    assert_eq!(row["input"], 10);
    // A report of nothing at all is no line.
    log.record(&turn("work", "s2"), &spend("s2", 0, 0, None), NOW);
    assert_eq!(lines(&dir, "2026-11.jsonl").len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_total_that_falls_is_a_counter_that_started_over() {
    let dir = folder("reset");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    log.record(&turn("work", "s1"), &spend("s1", 500, 100, None), NOW);
    log.record(&turn("work", "s1"), &spend("s1", 40, 10, None), NOW + 1);
    let rows = lines(&dir, "2026-11.jsonl");
    assert_eq!(
        (rows[1]["input"].clone(), rows[1]["output"].clone()),
        (json!(40), json!(10))
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_report_that_names_its_models_gives_each_its_own_line() {
    let dir = folder("models");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    let mut report = spend("s1", 30, 6, Some(900));
    let mut opus = ModelSpend::new("opus");
    opus.input = 20;
    opus.output = 4;
    let mut haiku = ModelSpend::new("haiku");
    haiku.input = 10;
    haiku.output = 2;
    haiku.cost_micro_usd = Some(900);
    report.by_model = vec![opus, haiku];
    log.record(&turn("work", "s1"), &report, NOW);
    let rows = lines(&dir, "2026-11.jsonl");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["model"], "opus");
    assert!(rows[0].get("cost_micro_usd").is_none());
    assert_eq!(rows[1]["model"], "haiku");
    assert_eq!(rows[1]["cost_micro_usd"], 900);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_summary_groups_by_day_and_by_account_and_filters() {
    let dir = folder("summary");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    log.record(&turn("work", "s1"), &spend("s1", 100, 10, Some(1_000)), NOW);
    log.record(
        &turn("work", "s1"),
        &spend("s1", 150, 30, Some(1_500)),
        NOW + 60,
    );
    log.record(&turn("home", "s2"), &spend("s2", 7, 1, None), NOW + 70);
    log.record(&turn("work", "s3"), &spend("s3", 11, 2, None), NOW - DAY);
    let by_day = log
        .summary(&json!({"from": NOW - 2 * DAY, "to": NOW + DAY, "group_by": "day"}))
        .expect("answers");
    assert_eq!(by_day["timezone"], "UTC");
    let groups = by_day["groups"].as_array().expect("groups");
    assert_eq!(groups.len(), 2, "{by_day}");
    assert_eq!(groups[0]["key"], "2026-11-15");
    assert_eq!(groups[0]["input"], 11);
    assert_eq!(groups[1]["key"], "2026-11-16");
    assert_eq!(groups[1]["input"], 157);
    assert_eq!(groups[1]["sessions"], 2);
    assert_eq!(groups[1]["records"], 3);
    assert_eq!(by_day["total"]["input"], 168);
    let by_account = log
        .summary(&json!({"group_by": "account"}))
        .expect("answers");
    let groups = by_account["groups"].as_array().expect("groups");
    let home = groups
        .iter()
        .find(|group| group["key"] == "home")
        .expect("home");
    let work = groups
        .iter()
        .find(|group| group["key"] == "work")
        .expect("work");
    assert_eq!(home["input"], 7);
    assert!(
        home.get("cost_micro_usd").is_none(),
        "no cost was reported: {home}"
    );
    assert_eq!(work["input"], 161);
    assert_eq!(work["cost_micro_usd"], 1_500);
    let only_home = log
        .summary(&json!({"group_by": "session", "account": "home"}))
        .expect("answers");
    assert_eq!(only_home["groups"].as_array().map(Vec::len), Some(1));
    let none_of_it = log
        .summary(&json!({"provider": "codex", "host": "elsewhere"}))
        .expect("answers");
    assert_eq!(none_of_it["groups"], json!([]));
    // The end of the range is exclusive.
    let before = log
        .summary(&json!({"from": 0, "to": NOW, "group_by": "account"}))
        .expect("answers");
    assert_eq!(before["total"]["input"], 11);
    assert!(log.summary(&json!({"group_by": "host"})).is_err());
    assert!(log.summary(&json!({"from": "yesterday"})).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_summary_can_be_narrowed_to_one_model_or_one_session() {
    let dir = folder("narrowed");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    let mut report = spend("s1", 30, 6, None);
    let mut opus = ModelSpend::new("opus");
    opus.input = 20;
    let mut haiku = ModelSpend::new("haiku");
    haiku.input = 10;
    report.by_model = vec![opus, haiku];
    log.record(&turn("work", "s1"), &report, NOW);
    log.record(&turn("work", "s2"), &spend("s2", 5, 1, None), NOW + 5);
    let haiku_only = log
        .summary(&json!({"group_by": "session", "model": "haiku"}))
        .expect("answers");
    assert_eq!(haiku_only["total"]["input"], 10, "{haiku_only}");
    assert_eq!(haiku_only["total"]["sessions"], 1);
    let one = log
        .summary(&json!({"group_by": "model", "session": "s2"}))
        .expect("answers");
    assert_eq!(one["total"]["input"], 5, "{one}");
    assert_eq!(one["total"]["sessions"], 1);
    assert!(log.summary(&json!({"model": 3})).is_err());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn files_older_than_keep_days_are_removed_at_start_and_newer_ones_stay() {
    let dir = folder("retention");
    fs::create_dir_all(&dir).expect("made");
    for name in [
        "2024-01.jsonl",
        "2025-09.jsonl",
        "2026-10.jsonl",
        "2026-11.jsonl",
        "notes.txt",
    ] {
        fs::write(dir.join(name), "{}\n").expect("written");
    }
    // 400 days before 2026-11-16 is 2025-10-12: September 2025 ended before it, October 2026 did not.
    let _log = UsageLog::open(dir.clone(), 400, NOW);
    let mut left: Vec<String> = fs::read_dir(&dir)
        .expect("read")
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    left.sort();
    assert_eq!(left, ["2026-10.jsonl", "2026-11.jsonl", "notes.txt"]);
    let _ = UsageLog::open(dir.clone(), 10, NOW);
    assert!(
        !dir.join("2026-10.jsonl").exists(),
        "ended 2026-11-01, more than 10 days back"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_latest_allowances_are_kept_for_each_account_and_survive_a_restart() {
    let dir = folder("limits");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    let mut report = UsageReport::new();
    report.observed_at = NOW - 5;
    report.buckets = vec![Bucket::new("five_hour")];
    log.note_report("work", Some("claude"), &report, NOW);
    let bare = UsageReport::new();
    log.note_report("home", None, &bare, NOW);
    let kept = log.limits();
    assert_eq!(kept["accounts"].as_array().map(Vec::len), Some(1), "{kept}");
    assert_eq!(kept["accounts"][0]["account"], "work");
    assert_eq!(kept["accounts"][0]["provider"], "claude");
    assert_eq!(kept["accounts"][0]["updated_at"], NOW);
    assert_eq!(kept["accounts"][0]["observed_at"], NOW - 5);
    // The same reading again keeps when the host first read it.
    log.note_report("work", Some("claude"), &report, NOW + 30);
    assert_eq!(log.limits()["accounts"][0]["updated_at"], NOW);
    drop(log);
    let log = UsageLog::open(dir.clone(), 400, NOW + 100);
    assert_eq!(log.limits()["accounts"][0]["account"], "work");
    // Nothing of the allowances is appended to the history.
    assert!(lines(&dir, "2026-11.jsonl").is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_parent_and_its_two_children_sum_once_and_each_child_names_its_parent() {
    let dir = folder("children");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    let child = |session| Turn {
        parent: Some("parent"),
        ..turn("helper", session)
    };
    log.record(
        &turn("work", "parent"),
        &spend("parent", 1000, 100, None),
        NOW,
    );
    log.record(&child("c1"), &spend("c1", 120, 30, None), NOW + 1);
    log.record(&child("c2"), &spend("c2", 50, 10, None), NOW + 2);
    // A child's report seen again, as a reconnect would show it, adds nothing.
    log.record(&child("c1"), &spend("c1", 120, 30, None), NOW + 3);

    let rows = lines(&dir, "2026-11.jsonl");
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows[0].get("parent_session_id").is_none());
    assert_eq!(rows[1]["parent_session_id"], "parent");
    assert_eq!(rows[2]["parent_session_id"], "parent");
    let summary = log.summary(&json!({"from": NOW - 10})).expect("a summary");
    assert_eq!(summary["total"]["input"], 1170);
    assert_eq!(summary["total"]["output"], 140);
    assert_eq!(summary["total"]["sessions"], 3);
}

#[test]
fn a_sessions_own_figures_leave_out_the_ones_it_started_and_absent_counters_stay_absent() {
    let dir = folder("own");
    let log = UsageLog::open(dir, 400, NOW);
    log.record(
        &turn("work", "parent"),
        &spend("parent", 1000, 100, None),
        NOW,
    );
    log.record(
        &Turn {
            parent: Some("parent"),
            ..turn("helper", "child")
        },
        &spend("child", 7, 0, None),
        NOW + 1,
    );
    assert_eq!(
        log.session_usage("work", "parent"),
        Some(json!({"records": 1, "sessions": 1, "input": 1000, "output": 100}))
    );
    assert_eq!(
        log.session_usage("helper", "child"),
        Some(json!({"records": 1, "sessions": 1, "input": 7}))
    );
    // Another account's session of the same id, and a session that reported nothing, have no figures.
    assert_eq!(log.session_usage("work", "child"), None);
    assert_eq!(log.session_usage("work", "none"), None);
}

#[test]
fn recorded_buckets_count_a_keys_tokens_today_in_thirty_days_and_by_model() {
    let dir = folder("recorded");
    let log = UsageLog::open(dir.clone(), 400, NOW);
    assert!(log.recorded_buckets("key", NOW).is_empty());
    log.record(
        &turn("key", "s1"),
        &spend("s1", 100, 20, None),
        NOW - 3 * DAY,
    );
    log.record(&turn("key", "s2"), &spend("s2", 1_000, 200, None), NOW);
    log.record(&turn("other", "s3"), &spend("s3", 7, 7, None), NOW);
    let buckets = log.recorded_buckets("key", NOW);
    let used = |id: &str| {
        buckets
            .iter()
            .find(|bucket| bucket["id"] == id)
            .map(|bucket| bucket["used"].clone())
    };
    assert_eq!(used("recorded_today"), Some(json!(1_200)));
    assert_eq!(used("recorded_30_days"), Some(json!(1_320)));
    assert_eq!(used("recorded_model_opus"), Some(json!(1_320)));
    assert!(
        buckets
            .iter()
            .all(|bucket| bucket["source"] == "recorded" && bucket["unit"] == "tokens"),
        "{buckets:?}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_usage_file_is_replaced_whole_with_nothing_beside_it_and_keeps_its_mode() {
    use crate::replace::proof;
    let dir = proof::folder("usage-replace");
    let log = UsageLog::open(dir.clone(), 30, NOW);
    log.replace("totals.json", b"one");
    log.replace("totals.json", b"two");
    assert_eq!(fs::read(dir.join("totals.json")).expect("read"), b"two");
    assert_eq!(proof::names(&dir), ["totals.json"]);
    #[cfg(unix)]
    {
        proof::set_mode(&dir.join("totals.json"), 0o640);
        log.replace("totals.json", b"three");
        assert_eq!(proof::mode(&dir.join("totals.json")), 0o640);
    }
    let _ = fs::remove_dir_all(&dir);
}
