//! `discover/nearby`: the agents this computer can find for a client that cannot search itself, such as a browser. The host
//! reads this user's descriptors (agents listening on loopback) and browses the link for `_acp-agent._tcp`; what comes back is
//! each record's claim, and the client's proof at connect is what decides whether it holds.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a search names what it uses from std"
)]

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use agentic_peer::discovery::{self, DirectoryCheck};
use serde_json::{Value, json};

use super::BAD_PARAMS;

/// The longest a search may be asked to take.
pub const MAX_SECONDS: u64 = 10;
/// How long a search takes when it is not told.
pub const DEFAULT_SECONDS: u64 = 3;

/// The search this process is running, if any, and the answer the last one gave. Each search opens a multicast socket and waits, so
/// a caller that arrives while one runs takes that one's answer instead of starting another.
struct Slot {
    running: bool,
    finished: u64,
    answer: Value,
}

/// Shared single-flight discovery state so concurrent callers coalesce onto one mDNS socket.
static SLOT: Mutex<Slot> = Mutex::new(Slot {
    running: false,
    finished: 0,
    answer: Value::Null,
});
/// Notifies callers waiting on the active mDNS discovery run in `SLOT`.
static DONE: Condvar = Condvar::new();

/// Ends the running search when dropped, so a search that panics frees the callers waiting on it.
struct Running(Option<Value>);

impl Drop for Running {
    fn drop(&mut self) {
        let answer = self
            .0
            .take()
            .unwrap_or_else(|| json!({ "candidates": [], "networkError": "the search failed" }));
        let mut slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
        slot.running = false;
        slot.finished += 1;
        slot.answer = answer;
        DONE.notify_all();
    }
}

/// Runs `run` unless a search is running, in which case this waits for that search and answers what it found.
fn shared(run: impl FnOnce() -> Value) -> Value {
    let mut slot = SLOT.lock().unwrap_or_else(PoisonError::into_inner);
    if slot.running {
        let seen = slot.finished;
        while slot.finished == seen {
            slot = DONE.wait(slot).unwrap_or_else(PoisonError::into_inner);
        }
        return slot.answer.clone();
    }
    slot.running = true;
    drop(slot);
    let mut running = Running(None);
    let answer = run();
    running.0 = Some(answer.clone());
    drop(running);
    answer
}

/// How long `params` asks a search to take: `seconds`, cut to [`MAX_SECONDS`].
///
/// # Errors
/// When `seconds` is present and not a whole number.
pub(crate) fn wait_of(params: &Value) -> Result<Duration, (i32, String)> {
    match params.get("seconds") {
        None | Some(Value::Null) => Ok(Duration::from_secs(DEFAULT_SECONDS)),
        Some(value) => value
            .as_u64()
            .map(|seconds| Duration::from_secs(seconds.min(MAX_SECONDS)))
            .ok_or_else(|| (BAD_PARAMS, "seconds is a whole number".to_owned())),
    }
}

/// `discover/nearby {seconds?}` -> `{"candidates": [{label, url, peer, method, source, webtransport, webtransportHash,
/// acpVersions, offers}], "directory"?, "networkError"?}`.
///
/// `directory` is `"shared"` when another user can change the descriptor folder (none of its descriptors were read) and
/// `"unchecked"` when this platform could not say (they were read). `networkError` says why the link could not be searched; the
/// descriptors are still read. A call that arrives while a search runs waits for it and answers what it found.
///
/// # Errors
/// `-32602` for a `seconds` that is no whole number.
pub(crate) fn nearby(params: &Value) -> Result<Value, (i32, String)> {
    Ok(search(wait_of(params)?))
}

/// The search itself, for a caller that has already read its wait.
#[must_use]
pub fn search(wait: Duration) -> Value {
    shared(|| report(&discovery::discover_report(wait)))
}

/// What a descriptor folder's check tells a client, or `None` when the folder is as private as it should be.
fn directory_word(check: DirectoryCheck) -> Option<&'static str> {
    match check {
        DirectoryCheck::SharedWithOthers => Some("shared"),
        DirectoryCheck::Private => None,
        // Every other finding, `NotChecked` included, means the folder was not shown to be this user's alone.
        _ => Some("unchecked"),
    }
}

fn report(found: &discovery::Discovery) -> Value {
    let mut reply = json!({
        "candidates": found
            .candidates
            .iter()
            .take(discovery::MAX_CANDIDATES)
            .map(discovery::Candidate::to_json)
            .collect::<Vec<_>>(),
    });
    if let Some(word) = found.directory.and_then(directory_word) {
        reply["directory"] = word.into();
    }
    if let Some(error) = &found.network_error {
        reply["networkError"] = error
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>()
            .into();
    }
    reply
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The tests share the one slot: each runs alone.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    #[test]
    fn a_wait_is_a_whole_number_of_seconds_cut_to_the_most_a_search_takes() {
        assert_eq!(wait_of(&json!({})).unwrap(), Duration::from_secs(3));
        assert_eq!(
            wait_of(&json!({"seconds": null})).unwrap(),
            Duration::from_secs(3)
        );
        assert_eq!(
            wait_of(&json!({"seconds": 2})).unwrap(),
            Duration::from_secs(2)
        );
        assert_eq!(
            wait_of(&json!({"seconds": 3600})).unwrap(),
            Duration::from_secs(10)
        );
        for bad in [json!("soon"), json!(-1), json!(1.5), json!(true)] {
            let refused = wait_of(&json!({ "seconds": bad })).unwrap_err();
            assert_eq!(refused.0, BAD_PARAMS, "{bad}");
        }
    }

    #[test]
    fn a_caller_that_arrives_during_a_search_takes_its_answer_and_starts_none() {
        let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let runs = Arc::new(AtomicUsize::new(0));
        let (started, hold) = std::sync::mpsc::channel::<()>();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let leader = {
            let runs = Arc::clone(&runs);
            std::thread::spawn(move || {
                shared(|| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    started.send(()).unwrap();
                    wait.recv().unwrap();
                    json!({"candidates": ["leader"]})
                })
            })
        };
        hold.recv().unwrap();
        let follower = {
            let runs = Arc::clone(&runs);
            std::thread::spawn(move || {
                shared(|| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    json!({"candidates": ["follower"]})
                })
            })
        };
        std::thread::sleep(Duration::from_millis(200));
        release.send(()).unwrap();
        let led = leader.join().unwrap();
        let followed = follower.join().unwrap();
        assert_eq!(led, json!({"candidates": ["leader"]}));
        assert_eq!(
            followed, led,
            "the follower answers what the running search found"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_search_that_panics_frees_the_next_caller() {
        let _alone = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
        let failed = std::thread::spawn(|| shared(|| panic!("the search failed")));
        assert!(failed.join().is_err());
        let next = shared(|| json!({"candidates": []}));
        assert_eq!(next, json!({"candidates": []}));
    }

    #[test]
    fn a_folder_not_shown_to_be_private_is_said_so() {
        assert_eq!(directory_word(DirectoryCheck::Private), None);
        assert_eq!(
            directory_word(DirectoryCheck::SharedWithOthers),
            Some("shared")
        );
        assert_eq!(
            directory_word(DirectoryCheck::NotChecked),
            Some("unchecked")
        );
    }
}
