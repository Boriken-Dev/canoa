use serde_json::json;
use time::{Date, Month, PrimitiveDateTime, Time, UtcOffset};

use super::*;

/// Milliseconds since the Unix epoch of a wall-clock minute `offset` minutes from UTC.
fn at(day: (i32, u8, u8), hour: u8, minute: u8, offset: i32) -> u64 {
    let date = Date::from_calendar_date(day.0, Month::try_from(day.1).expect("a month"), day.2)
        .expect("a date");
    let time = Time::from_hms(hour, minute, 0).expect("a time");
    let offset = UtcOffset::from_whole_seconds(offset * 60).expect("an offset");
    let seconds = PrimitiveDateTime::new(date, time)
        .assume_offset(offset)
        .unix_timestamp();
    u64::try_from(seconds).expect("after 1970") * 1000
}

fn watch(params: serde_json::Value, now: u64, baseline: Option<TreeUse>) -> Watch {
    let config = Config::from_params(&params)
        .expect("a watch")
        .expect("enabled");
    Watch::new(config, "key".to_owned(), now, baseline).expect("armed")
}

fn quiet() -> Sample {
    Sample {
        quiet_ms: 60_000,
        ..Sample::default()
    }
}

fn used(processes: usize, cpu_ms: u64, memory_bytes: u64) -> Option<TreeUse> {
    Some(TreeUse {
        processes,
        cpu_ms: Some(cpu_ms),
        memory_bytes: Some(memory_bytes),
    })
}

#[test]
fn an_interval_watch_first_checks_one_interval_after_it_is_armed() {
    let mut watch = watch(json!({"everyMs": 500}), 1_000, None);
    assert!(!watch.due(1_000), "not at once");
    assert!(!watch.due(1_499));
    assert!(watch.due(1_500));
    assert_eq!(
        watch.check(1_500, &quiet()),
        Check::Prompt("Continue".to_owned())
    );
    assert!(!watch.due(1_999));
    assert!(watch.due(2_000));
    assert_eq!(watch.status()["nextCheckAt"], 2_000);
    assert_eq!(watch.status()["promptsSent"], 1);
}

#[test]
fn a_cron_line_names_its_next_minute_on_the_persons_clock() {
    let friday = (2026, 11, 20);
    let office = Cron::parse("*/15 9-17 * * 1-5", 0).expect("a line");
    assert_eq!(
        office.next_after(at(friday, 10, 30, 0)),
        Some(at(friday, 10, 45, 0)),
        "strictly after the minute it is asked at"
    );
    assert_eq!(
        office.next_after(at(friday, 17, 50, 0)),
        Some(at((2026, 11, 23), 9, 0, 0)),
        "the weekend is skipped"
    );
    let nine = Cron::parse("0 9 * * *", 60).expect("a line");
    assert_eq!(
        nine.next_after(at(friday, 0, 0, 0)),
        Some(at(friday, 9, 0, 60)),
        "nine on a clock an hour ahead of UTC"
    );
    assert_eq!(at(friday, 9, 0, 60), at(friday, 8, 0, 0));
    let sunday = Cron::parse("0 0 * * 7", 0).expect("a line");
    assert_eq!(
        sunday.next_after(at(friday, 0, 0, 0)),
        Some(at((2026, 11, 22), 0, 0, 0))
    );
}

#[test]
fn both_day_fields_restricted_take_a_day_either_one_names() {
    // The 10th, or any Friday: Tuesday the 10th, then Friday the 13th.
    let either = Cron::parse("0 0 10 * 5", 0).expect("a line");
    let saturday = at((2026, 11, 7), 0, 0, 0);
    let tenth = either.next_after(saturday).expect("a day");
    assert_eq!(tenth, at((2026, 11, 10), 0, 0, 0));
    assert_eq!(either.next_after(tenth), Some(at((2026, 11, 13), 0, 0, 0)));
    // One field restricted is that field alone.
    let tenths = Cron::parse("0 0 10 * *", 0).expect("a line");
    assert_eq!(tenths.next_after(tenth), Some(at((2026, 12, 10), 0, 0, 0)));
}

#[test]
fn a_cron_line_that_is_not_one_or_never_comes_is_refused() {
    for line in [
        "bad",
        "*/0 * * * *",
        "60 * * * *",
        "5-1 * * * *",
        "* * * *",
        "* * * * 8",
    ] {
        assert!(Cron::parse(line, 0).is_err(), "{line}");
    }
    let config = Config::from_params(&json!({"cron": "0 0 31 2 *"}))
        .expect("a line")
        .expect("enabled");
    let never = Watch::new(config, "key".to_owned(), at((2026, 11, 20), 0, 0, 0), None);
    assert!(never.is_err());
}

#[test]
fn params_that_are_not_a_watch_are_refused_and_enabled_false_takes_it_away() {
    assert_eq!(Config::from_params(&json!({"enabled": false})), Ok(None));
    for params in [
        json!({"cron": "* * * * *", "everyMs": 10}),
        json!({"everyMs": 0}),
        json!({"everyMs": -1}),
        json!({"whenIdle": "yes"}),
        json!({"prompt": "  "}),
        json!({"utcOffsetMinutes": 1440}),
    ] {
        assert!(Config::from_params(&params).is_err(), "{params}");
    }
    let defaults = Config::from_params(&json!({}))
        .expect("a watch")
        .expect("enabled");
    assert_eq!(defaults.schedule, Schedule::Every(DEFAULT_EVERY_MS));
    assert_eq!(defaults.max_prompts, DEFAULT_MAX_PROMPTS);
    assert!(defaults.conditions.when_idle);
}

#[test]
fn a_waiting_permission_a_running_turn_and_recent_activity_skip_a_check() {
    let mut watch = watch(json!({"everyMs": 10, "quietMs": 1_000}), 0, None);
    let asking = Sample {
        permissions_waiting: 1,
        ..quiet()
    };
    assert!(matches!(watch.check(10, &asking), Check::Skip(why) if why.contains("permission")));
    let running = Sample {
        turn_running: true,
        ..quiet()
    };
    assert!(matches!(watch.check(20, &running), Check::Skip(why) if why.contains("turn")));
    let recent = Sample {
        quiet_ms: 999,
        ..quiet()
    };
    assert!(matches!(watch.check(30, &recent), Check::Skip(why) if why.contains("active")));
    assert_eq!(watch.status()["lastCheck"]["prompted"], false);
    assert_eq!(
        watch.check(40, &quiet()),
        Check::Prompt("Continue".to_owned())
    );
    assert_eq!(watch.status()["lastCheck"]["prompted"], true);

    // Without whenIdle a running turn is no reason to wait.
    let mut busy = watch_any_turn();
    assert!(matches!(busy.check(10, &running), Check::Prompt(_)));
}

fn watch_any_turn() -> Watch {
    watch(json!({"everyMs": 10, "whenIdle": false}), 0, None)
}

#[test]
fn cpu_time_since_the_last_check_and_memory_decide_a_stall() {
    let params =
        json!({"everyMs": 10, "cpuBelowMs": 50, "memoryNotShrinking": true, "prompt": "Status"});
    let mut watch = watch(params, 0, used(1, 100, 1_000));
    let working = Sample {
        processes: used(1, 400, 1_000),
        ..quiet()
    };
    assert!(matches!(watch.check(10, &working), Check::Skip(why) if why.contains("300 ms")));
    let stalled = Sample {
        processes: used(1, 420, 1_000),
        ..quiet()
    };
    assert_eq!(
        watch.check(20, &stalled),
        Check::Prompt("Status".to_owned())
    );
    let freed = Sample {
        processes: used(1, 421, 800),
        ..quiet()
    };
    assert!(matches!(watch.check(30, &freed), Check::Skip(why) if why.contains("shrank")));
    let unread = Sample {
        processes: None,
        ..quiet()
    };
    assert!(
        matches!(watch.check(40, &unread), Check::Skip(why) if why.contains("could not be read"))
    );
}

#[test]
fn while_children_waits_for_a_process_of_the_agents_own() {
    let mut watch = watch(json!({"everyMs": 10, "whileChildren": true}), 0, None);
    let alone = Sample {
        processes: used(1, 0, 1),
        ..quiet()
    };
    assert!(matches!(watch.check(10, &alone), Check::Skip(why) if why.contains("nothing besides")));
    let building = Sample {
        processes: used(3, 0, 1),
        ..quiet()
    };
    assert!(matches!(watch.check(20, &building), Check::Prompt(_)));
}

#[test]
fn max_prompts_holds_off_until_the_person_writes_and_zero_never_does() {
    let mut watch = watch(json!({"everyMs": 10, "maxPrompts": 2}), 0, None);
    assert!(matches!(watch.check(10, &quiet()), Check::Prompt(_)));
    assert!(matches!(watch.check(20, &quiet()), Check::Prompt(_)));
    assert!(matches!(watch.check(30, &quiet()), Check::Skip(why) if why.contains("2 prompts")));
    assert!(watch.due(40), "it holds off, it does not stop");
    watch.person_wrote();
    assert!(matches!(watch.check(40, &quiet()), Check::Prompt(_)));

    let mut endless = Watch::new(
        Config::from_params(&json!({"everyMs": 10, "maxPrompts": 0}))
            .expect("a watch")
            .expect("enabled"),
        "key".to_owned(),
        0,
        None,
    )
    .expect("armed");
    for each in 1..=50 {
        assert!(matches!(
            endless.check(each * 10, &quiet()),
            Check::Prompt(_)
        ));
    }
}

#[test]
fn stop_on_input_ends_the_watch_at_the_persons_next_message() {
    let mut watch = watch(json!({"everyMs": 10, "stopOnInput": true}), 0, None);
    watch.person_wrote();
    assert!(!watch.due(1_000));
    let status = watch.status();
    assert_eq!(status["nextCheckAt"], serde_json::Value::Null);
    assert_eq!(status["stopped"], "you wrote in the session");
}

#[test]
fn the_persons_numbers_are_taken_as_they_are_with_no_floor_or_ceiling() {
    let mut watch = watch(
        json!({"everyMs": 1, "maxPrompts": 4_000_000_000_u64, "quietMs": 86_400_000_000_u64}),
        0,
        None,
    );
    assert!(!watch.due(0));
    assert!(watch.due(1));
    let status = watch.status();
    assert_eq!(status["schedule"]["everyMs"], 1);
    assert_eq!(status["maxPrompts"], 4_000_000_000_u64);
    assert_eq!(status["quietMs"], 86_400_000_000_u64);
    assert!(matches!(watch.check(1, &quiet()), Check::Skip(_)));
}

#[test]
fn a_watch_reads_the_processes_only_when_a_condition_needs_them() {
    for (params, reads) in [
        (json!({"everyMs": 10}), false),
        (
            json!({"everyMs": 10, "quietMs": 5, "whenIdle": false}),
            false,
        ),
        (json!({"everyMs": 10, "whileChildren": true}), true),
        (json!({"everyMs": 10, "cpuBelowMs": 5}), true),
        (json!({"everyMs": 10, "memoryNotShrinking": true}), true),
    ] {
        assert_eq!(
            watch(params.clone(), 0, None).measures_processes(),
            reads,
            "{params}"
        );
    }
    // Before its time a watch is not due, so nothing is read for it; after a check the next time is a whole interval on.
    let mut watch = watch(json!({"everyMs": 100, "whileChildren": true}), 0, None);
    assert!(!watch.due(99));
    assert!(watch.due(100));
    watch.check(100, &Sample::default());
    assert!(!watch.due(199));
}
