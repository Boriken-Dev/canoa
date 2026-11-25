//! The Store's caches answer from what they keep, and a stand-in publisher counts how often it is asked.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;

/// A publisher that says `version` and counts the requests it gets.
fn publisher(version: &'static str, count: &Arc<AtomicUsize>) -> Fetch {
    let count = Arc::clone(count);
    Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"known": true, "version": version}))
    })
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-caches-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{what}");
}

#[test]
fn a_second_ask_is_answered_from_the_cache_with_no_request() {
    let cache = Arc::new(LatestCache::new(None, Duration::from_secs(3600)));
    let count = Arc::new(AtomicUsize::new(0));
    let first = cache
        .get("p", None, false, publisher("1.0.0", &count))
        .expect("asked");
    assert_eq!(first["version"], "1.0.0");
    let second = cache
        .get("p", None, false, publisher("2.0.0", &count))
        .expect("kept");
    assert_eq!(second["version"], "1.0.0", "{second}");
    assert_eq!(second["stale"], false);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    // Another channel is another entry.
    cache
        .get("p", Some("beta"), false, publisher("3.0.0", &count))
        .expect("asked");
    assert_eq!(count.load(Ordering::SeqCst), 2);
    // A refresh asks the publisher whatever is kept.
    let again = cache
        .get("p", None, true, publisher("2.0.0", &count))
        .expect("asked");
    assert_eq!(again["version"], "2.0.0");
    assert_eq!(count.load(Ordering::SeqCst), 3);
}

#[test]
fn a_stale_answer_is_served_at_once_and_refreshed_once() {
    let cache = Arc::new(LatestCache::new(None, Duration::from_millis(60)));
    let count = Arc::new(AtomicUsize::new(0));
    cache
        .get("p", None, false, publisher("1.0.0", &count))
        .expect("asked");
    std::thread::sleep(Duration::from_millis(120));
    // The publisher is slow now: the stale answer comes back before it says anything, however many ask meanwhile.
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let slow: Fetch = {
        let (count, gate) = (Arc::clone(&count), Arc::clone(&gate));
        Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            let mut open = lock(&gate.0);
            while !*open {
                open = gate.1.wait(open).unwrap_or_else(PoisonError::into_inner);
            }
            Ok(json!({"known": true, "version": "2.0.0"}))
        })
    };
    for _ in 0..3 {
        let stale = cache
            .get("p", None, false, Arc::clone(&slow))
            .expect("kept");
        assert_eq!(stale["version"], "1.0.0", "{stale}");
        assert_eq!(stale["stale"], false, "past its time is not a failure");
        assert_eq!(stale["refreshing"], true);
    }
    wait_for("the background request starts", || {
        count.load(Ordering::SeqCst) == 2
    });
    *lock(&gate.0) = true;
    gate.1.notify_all();
    wait_for("the new version is kept", || {
        cache
            .peek("p", None)
            .is_some_and(|kept| kept["version"] == "2.0.0")
    });
    assert_eq!(count.load(Ordering::SeqCst), 2, "one refresh, not three");
    let kept = cache.peek("p", None).expect("kept");
    assert_eq!(kept["stale"], false);
    assert_eq!(kept["refreshing"], false);
}

#[test]
fn what_the_publisher_did_not_say_is_not_kept_and_a_stale_version_survives_a_failure() {
    let cache = Arc::new(LatestCache::new(None, Duration::from_millis(60)));
    let count = Arc::new(AtomicUsize::new(0));
    let unknown: Fetch = {
        let count = Arc::clone(&count);
        Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"known": false, "unknown_because": "no answer"}))
        })
    };
    let said = cache
        .get("p", None, false, Arc::clone(&unknown))
        .expect("answered");
    assert_eq!(said["known"], false);
    assert!(cache.peek("p", None).is_none(), "nothing is kept");
    cache
        .get("p", None, false, publisher("1.0.0", &count))
        .expect("asked");
    std::thread::sleep(Duration::from_millis(120));
    let kept = cache.get("p", None, false, unknown).expect("kept");
    assert_eq!(kept["version"], "1.0.0");
    wait_for("the failed request is made", || {
        count.load(Ordering::SeqCst) == 3
    });
    wait_for("the request is over", || {
        lock(&cache.state).running.is_empty()
    });
    let held = cache.peek("p", None).expect("kept");
    assert_eq!(held["version"], "1.0.0");
    assert_eq!(held["stale"], true, "the ask failed: {held}");
    assert_eq!(held["problem"], "no answer");
    assert_eq!(held["refreshing"], false, "nothing is asking now");
    assert_eq!(held["checked"], kept["checked"], "when it last worked");

    // The next ask that works clears it.
    cache
        .get("p", None, true, publisher("2.0.0", &count))
        .expect("asked");
    let healed = cache.peek("p", None).expect("kept");
    assert_eq!(healed["stale"], false);
    assert_eq!(healed["version"], "2.0.0");
}

#[test]
fn the_cache_is_on_disk_and_another_run_answers_from_it() {
    let dir = folder("disk");
    let count = Arc::new(AtomicUsize::new(0));
    {
        let cache = Arc::new(LatestCache::new(
            Some(dir.clone()),
            Duration::from_secs(3600),
        ));
        cache
            .get("p", None, false, publisher("1.0.0", &count))
            .expect("asked");
    }
    let cache = Arc::new(LatestCache::new(
        Some(dir.clone()),
        Duration::from_secs(3600),
    ));
    let kept = cache
        .get("p", None, false, publisher("9.9.9", &count))
        .expect("kept");
    assert_eq!(kept["version"], "1.0.0");
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "no request for the second run"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_time_of_zero_asks_every_time() {
    let cache = Arc::new(LatestCache::new(None, Duration::ZERO));
    let count = Arc::new(AtomicUsize::new(0));
    for _ in 0..3 {
        cache
            .get("p", None, false, publisher("1.0.0", &count))
            .expect("asked");
    }
    assert_eq!(count.load(Ordering::SeqCst), 3);
}

#[test]
fn a_stale_entry_is_renewed_by_the_timer_and_a_fresh_one_is_left_alone() {
    let cache = Arc::new(LatestCache::new(None, Duration::from_millis(60)));
    let count = Arc::new(AtomicUsize::new(0));
    cache
        .get("p", None, false, publisher("1.0.0", &count))
        .expect("asked");
    assert!(cache.stale().is_empty());
    std::thread::sleep(Duration::from_millis(120));
    let stale = cache.stale();
    assert_eq!(stale, [("p".to_owned(), None)]);
    cache.renew("p", None, &publisher("2.0.0", &count));
    assert_eq!(cache.peek("p", None).expect("kept")["version"], "2.0.0");
    assert!(cache.stale().is_empty());
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn answers_kept_for_a_time_are_asked_again_after_it_a_change_or_a_refresh() {
    let cache: TtlCache<u32> = TtlCache::new(Duration::from_millis(80));
    let count = AtomicUsize::new(0);
    let ask = || {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(7)
    };
    assert_eq!(cache.get("a", false, ask), Ok(7));
    assert_eq!(cache.get("a", false, ask), Ok(7));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(cache.get("a", true, ask), Ok(7));
    assert_eq!(count.load(Ordering::SeqCst), 2);
    cache.clear();
    assert_eq!(cache.get("a", false, ask), Ok(7));
    assert_eq!(count.load(Ordering::SeqCst), 3);
    std::thread::sleep(Duration::from_millis(120));
    assert_eq!(cache.get("a", false, ask), Ok(7));
    assert_eq!(count.load(Ordering::SeqCst), 4);
    let failing: Result<u32, Failed> = cache.get("b", false, || Err((-32002, "no".to_owned())));
    assert!(failing.is_err());
    assert_eq!(cache.get("b", false, ask), Ok(7), "a failure is not kept");
}

#[test]
fn a_kept_answer_past_its_time_is_named_for_renewal_only_with_a_request_noted() {
    let cache: TtlCache<Value> = TtlCache::new(Duration::from_millis(40));
    cache
        .get("a", false, || Ok(json!({"n": 1})))
        .expect("asked");
    cache
        .get("b", false, || Ok(json!({"n": 2})))
        .expect("asked");
    cache.note("a", &json!({"type": "skills", "label": "main"}));
    assert!(cache.due().is_empty(), "within its time");
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(cache.expired(), ["a", "b"]);
    assert_eq!(cache.due(), [json!({"type": "skills", "label": "main"})]);
    cache.get("a", true, || Ok(json!({"n": 3}))).expect("asked");
    assert!(cache.due().is_empty(), "renewed");
}
