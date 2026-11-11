//! The way into a running host from its own process: a link served as a paired key's connection is, with no proof, ended by the
//! host's stop.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use std::time::{Duration, Instant};

use canoa_host::LocalError;
use linked::{Hosted, ask, entry, next, open_session, said, send};
use serde_json::json;
use support::{WAIT, cwd, running};

#[test]
fn a_link_from_inside_runs_a_turn_with_no_proof_and_no_paired_key() {
    let host = Hosted::start(&entry("fixture", ""));
    assert_eq!(host.local.entries(), ["fixture"]);
    let link = host.local.open("fixture").expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    let (answer, updates) = ask(
        &link,
        3,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": "hello"}]}),
    )
    .expect("the turn is answered");
    assert_eq!(answer["stopReason"], "end_turn");
    assert_eq!(said(&updates), "echo: hello");
    drop(link);
    host.stop();
    assert!(host.ended.join().expect("no panic").is_ok());
}

#[test]
fn an_entry_that_lists_other_keys_is_still_reached_from_inside() {
    let other = format!("sha256:{}", "0".repeat(64));
    let host = Hosted::start(&entry("fixture", &format!("peers = [\"{other}\"]")));
    let link = host.local.open("fixture").expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    assert!(!session.is_empty());
    drop(link);
    host.stop();
}

#[test]
fn an_entry_the_host_does_not_serve_is_no_entry() {
    let host = Hosted::start(&entry("fixture", ""));
    assert_eq!(host.local.open("other").err(), Some(LocalError::NoEntry));
    host.stop();
}

#[test]
fn closing_the_link_ends_its_agent() {
    let host = Hosted::start(&entry("fixture", ""));
    let link = host.local.open("fixture").expect("a link");
    open_session(&link, &cwd(&host.scratch));
    let (said, _) = ask(&link, 3, "_fixture/pid", json!({})).expect("it answers");
    let pid = u32::try_from(said["pid"].as_u64().expect("a pid")).expect("a pid");
    assert!(running(pid));
    link.close();
    let until = Instant::now() + WAIT;
    while running(pid) && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!running(pid), "the agent outlived its link");
    host.stop();
}

#[test]
fn stopping_the_host_ends_a_link_and_its_agent_and_later_links_are_refused() {
    let host = Hosted::start(&entry("fixture", ""));
    let link = host.local.open("fixture").expect("a link");
    open_session(&link, &cwd(&host.scratch));
    let (said, _) = ask(&link, 3, "_fixture/pid", json!({})).expect("it answers");
    let pid = u32::try_from(said["pid"].as_u64().expect("a pid")).expect("a pid");
    host.stop();
    assert!(
        !running(pid),
        "the agent was ended before the host returned"
    );
    assert!(next(&link).is_err(), "the link ended");
    assert_eq!(host.local.open("fixture").err(), Some(LocalError::Stopped));
    assert!(host.ended.join().expect("no panic").is_ok());
}

#[test]
fn entries_with_max_peers_take_no_more_links_than_they_allow() {
    let host = Hosted::start(&entry("fixture", "max_peers = 1"));
    let first = host.local.open("fixture").expect("a link");
    open_session(&first, &cwd(&host.scratch));
    let second = host
        .local
        .open("fixture")
        .expect("opens, then is turned away");
    send(
        &second,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}),
    );
    assert!(next(&second).is_err(), "the second is closed");
    drop(first);
    host.stop();
}
