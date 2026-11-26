//! `notify/*`: the grant that guards them, the catalog of events, the test notice and the link's own subscription.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::Arc;

use serde_json::{Value, json};

use super::notify::Link;
use super::{Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, Grant};
use crate::config::NotificationsConfig;
use crate::notify::{Directory, Hub};

const DENIED: i64 = -32001;
const REFUSED: i64 = -32002;
const NOT_FOUND: i64 = -32003;

fn hub() -> Arc<Hub> {
    Hub::fixed(Directory::from_registries(
        NotificationsConfig::default(),
        Vec::new(),
        None,
    ))
}

fn control(hub: Option<Arc<Hub>>) -> Control {
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    match hub {
        Some(hub) => control.with_notices(hub),
        None => control,
    }
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    let answer = control.handle(caller, &line).expect("answered");
    serde_json::from_slice(&answer).expect("JSON")
}

fn key(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

#[test]
fn only_a_caller_holding_notify_reaches_the_methods_and_the_owner_does() {
    let control = control(Some(hub()));
    for method in [
        "notify/notifiers",
        "notify/events",
        "notify/test",
        "notify/keep",
    ] {
        let answer = ask(
            &control,
            &key(&[Grant::ConfigRead]),
            method,
            json!({"name": "desktop"}),
        );
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let answer = ask(&control, &key(&[Grant::Notify]), "notify/events", json!({}));
    assert!(answer["error"].is_null(), "{answer}");
    let answer = ask(&control, &Caller::Owner, "notify/events", json!({}));
    assert!(answer["error"].is_null(), "{answer}");
    assert_eq!(Grant::named("notify"), Some(Grant::Notify));
}

#[test]
fn the_link_methods_need_a_link_when_asked_as_plain_requests() {
    let control = control(Some(hub()));
    for method in ["notify/subscribe", "notify/attended"] {
        let answer = ask(&control, &Caller::Owner, method, json!({}));
        assert_eq!(answer["error"]["code"], REFUSED, "{method}: {answer}");
    }
}

#[test]
fn the_catalog_lists_each_event_with_where_it_goes_and_the_host_events_start_off() {
    let control = control(Some(hub()));
    let answer = ask(&control, &Caller::Owner, "notify/events", json!({}));
    let events = answer["result"]["events"].as_array().expect("a list");
    let ids: Vec<&str> = events
        .iter()
        .filter_map(|each| each["id"].as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "permission",
            "question",
            "finished",
            "failed",
            "usage_warn",
            "usage_stop",
            "update_failed",
            "host_restarted",
            "host_updated"
        ]
    );
    let row = |id: &str| events.iter().find(|each| each["id"] == id).expect("a row");
    assert_eq!(row("permission")["notifiers"], json!(["desktop"]));
    assert_eq!(row("host_restarted")["notifiers"], json!([]));
    assert_eq!(row("host_updated")["set"], false);
}

#[test]
fn desktop_is_listed_first_with_no_removal_and_a_test_says_why_nobody_was_told() {
    let hub = hub();
    let control = control(Some(Arc::clone(&hub)));
    let answer = ask(&control, &Caller::Owner, "notify/notifiers", json!({}));
    let notifiers = answer["result"]["notifiers"].as_array().expect("a list");
    assert_eq!(notifiers[0]["name"], "desktop");
    assert_eq!(notifiers[0]["removable"], false);
    assert_eq!(notifiers[0]["state"], "no window is open");
    let answer = ask(
        &control,
        &Caller::Owner,
        "notify/test",
        json!({"name": "desktop"}),
    );
    assert_eq!(answer["result"]["ok"], false, "{answer}");
    assert!(
        answer["result"]["why"]
            .as_str()
            .is_some_and(|why| why.contains("no window"))
    );
    let window = hub.subscribe("en");
    let answer = ask(
        &control,
        &Caller::Owner,
        "notify/test",
        json!({"name": "desktop"}),
    );
    assert_eq!(answer["result"]["ok"], true, "{answer}");
    let lines = window.take();
    assert_eq!(lines.len(), 1);
    let line: Value = serde_json::from_slice(&lines[0]).expect("JSON");
    assert_eq!(line["params"]["body"], "Test");
    let answer = ask(
        &control,
        &Caller::Owner,
        "notify/test",
        json!({"name": "ghost"}),
    );
    assert_eq!(answer["error"]["code"], NOT_FOUND, "{answer}");
}

#[test]
fn a_host_that_tells_nobody_refuses_the_methods_in_words() {
    let control = control(None);
    let answer = ask(&control, &Caller::Owner, "notify/events", json!({}));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
}

#[test]
fn a_link_subscribes_once_reports_what_the_person_looks_at_and_ends_with_the_link() {
    let hub = hub();
    let link = Link::new(Some(Arc::clone(&hub)));
    let early = link.call("notify/attended", &json!({"session": "s1"}));
    assert!(early.is_err(), "ask for notices first");
    link.call("notify/subscribe", &json!({"language": "es"}))
        .expect("subscribed");
    link.call("notify/subscribe", &json!({"language": "es"}))
        .expect("again");
    assert!(hub.has_window());
    link.call("notify/attended", &json!({"session": "s1"}))
        .expect("attended");
    let done = hub.fire(
        "finished",
        crate::notify::Occurrence {
            session: Some(canoa::notifier::SessionRef::new("s1")),
            ..crate::notify::Occurrence::default()
        },
    );
    assert!(done.is_none(), "the person is looking at s1");
    link.call("notify/attended", &json!({"session": null}))
        .expect("looked away");
    let sent = hub
        .fire(
            "finished",
            crate::notify::Occurrence {
                session: Some(canoa::notifier::SessionRef::new("s1")),
                ..crate::notify::Occurrence::default()
            },
        )
        .expect("told")
        .join()
        .expect("ends");
    assert_eq!(sent[0].outcome, Ok(()));
    let lines = link.pending();
    assert_eq!(lines.len(), 1);
    let line: Value = serde_json::from_slice(&lines[0]).expect("JSON");
    assert_eq!(line["params"]["body"], "El agente terminó su turno");
    drop(link);
    assert!(!hub.has_window());
}
