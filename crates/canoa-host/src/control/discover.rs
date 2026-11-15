//! `discover/nearby`: the agents this computer can find for a client that cannot search itself, such as a browser. The host
//! reads this user's descriptors (agents listening on loopback) and browses the link for `_acp-agent._tcp`; what comes back is
//! each record's claim, and the client's proof at connect is what decides whether it holds.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a search names what it uses from std"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use coqui_agentic_peer::discovery::{self, DirectoryCheck};
use serde_json::{Value, json};

use super::{BAD_PARAMS, REFUSED};

/// The longest a search may be asked to take.
pub const MAX_SECONDS: u64 = 10;
/// How long a search takes when it is not told.
pub const DEFAULT_SECONDS: u64 = 3;

/// Whether a search is running in this process: each opens a multicast socket and waits, so a second one is turned away rather
/// than queued behind the first.
static SEARCHING: AtomicBool = AtomicBool::new(false);

struct Searching;

impl Searching {
    fn begin() -> Option<Self> {
        (!SEARCHING.swap(true, Ordering::AcqRel)).then(|| Self)
    }
}

impl Drop for Searching {
    fn drop(&mut self) {
        SEARCHING.store(false, Ordering::Release);
    }
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
/// `directory` is `"shared"` when the descriptor directory is writable by others, so none of its descriptors were read.
/// `networkError` says why the link could not be searched; the descriptors are still read.
///
/// # Errors
/// `-32602` for a `seconds` that is no whole number, and `-32002` while another search of this process runs.
pub(crate) fn nearby(params: &Value) -> Result<Value, (i32, String)> {
    let wait = wait_of(params)?;
    search(wait).ok_or((REFUSED, "a search is already running".to_owned()))
}

/// The search itself, for a caller that has already read its wait; `None` while another runs in this process.
#[must_use]
pub fn search(wait: Duration) -> Option<Value> {
    let _held = Searching::begin()?;
    let found = discovery::discover_report(wait);
    let mut reply = json!({
        "candidates": found
            .candidates
            .iter()
            .take(discovery::MAX_CANDIDATES)
            .map(discovery::Candidate::to_json)
            .collect::<Vec<_>>(),
    });
    if found.directory == Some(DirectoryCheck::SharedWithOthers) {
        reply["directory"] = "shared".into();
    }
    if let Some(error) = found.network_error {
        reply["networkError"] = error
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>()
            .into();
    }
    Some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_second_search_is_turned_away_while_one_runs() {
        let first = Searching::begin().expect("none runs");
        assert!(Searching::begin().is_none());
        assert_eq!(nearby(&json!({"seconds": 0})).unwrap_err().0, REFUSED);
        drop(first);
        assert!(Searching::begin().is_some());
    }
}
