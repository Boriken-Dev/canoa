//! The control methods of telemetry export are the person's own, and so is the write that turns it on or accepts the list.

use super::overlays_tests::ask;
use super::routes::guard_telemetry_switch;
use super::*;

fn memory() -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
}

fn key() -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: Grant::all().into_iter().collect(),
    }
}

#[test]
fn the_catalog_is_the_owners_to_read_and_a_key_with_every_grant_is_refused() {
    let control = memory();
    let owner = ask(&control, &Caller::Owner, "telemetry/catalog", json!({}));
    let digest = owner["result"]["digest"].as_str().expect("a digest");
    assert_eq!(digest, crate::telemetry_digest());
    assert!(
        owner["result"]["signals"]
            .as_array()
            .is_some_and(|signals| !signals.is_empty())
    );
    for method in ["telemetry/catalog", "telemetry/status", "telemetry/test"] {
        let refused = ask(&control, &key(), method, json!({"name": "local"}));
        assert!(refused.get("error").is_some(), "{method}: {refused}");
    }
}

#[test]
fn a_key_cannot_turn_export_on_or_accept_the_list_by_any_way_a_write_has() {
    let denied = |params: serde_json::Value| guard_telemetry_switch(&key(), &params).is_err();
    let allowed = |params: serde_json::Value| guard_telemetry_switch(&key(), &params).is_ok();
    assert!(denied(
        json!({"ops": [{"path": ["host", "telemetry", "enabled"], "value": true}]})
    ));
    assert!(denied(
        json!({"ops": [{"path": ["host", "telemetry", "acknowledged"], "value": "x"}]})
    ));
    assert!(denied(
        json!({"ops": [{"path": ["host", "telemetry"], "value": {"enabled": true}}]})
    ));
    assert!(denied(
        json!({"ops": [{"path": ["host"], "value": {"telemetry": {"enabled": true}}}]})
    ));
    assert!(denied(
        json!({"path": ["host", "telemetry"], "value": {"enabled": true}})
    ));
    // What does not turn it on: an exporter, the categories, another table.
    assert!(allowed(
        json!({"ops": [{"path": ["host", "telemetry", "record"], "value": ["logs"]}]})
    ));
    assert!(allowed(
        json!({"path": ["host", "telemetry", "exporters", "local"], "value": {"provider": "file"}})
    ));
    assert!(allowed(
        json!({"ops": [{"path": ["host", "web", "listen"], "value": "127.0.0.1:0"}]})
    ));
    assert!(allowed(
        json!({"ops": [{"path": ["host"], "value": {"shell": "x"}}]})
    ));
    // The control: the person's own link is let through.
    assert!(
        guard_telemetry_switch(
            &Caller::Owner,
            &json!({"ops": [{"path": ["host", "telemetry", "enabled"], "value": true}]})
        )
        .is_ok()
    );
}
