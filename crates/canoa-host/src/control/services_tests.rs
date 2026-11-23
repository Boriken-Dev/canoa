//! The control methods of a service's namespace: the grant that guards them, and what a host with no such service says.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, Grant};
use crate::services::Services;
use crate::services::standin::{TestHost, mail_over};

const DENIED: i64 = -32001;
const NO_METHOD: i64 = -32601;

fn control(services: Option<Services>) -> Control {
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    match services {
        Some(services) => control.with_services(Arc::new(services)),
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
fn only_a_key_holding_services_setup_reaches_a_services_methods() {
    let control = control(Some(mail_over(&[])));
    let reads = key(&[
        Grant::Services,
        Grant::Accounts,
        Grant::Hosting,
        Grant::ConfigWrite,
    ]);
    let setup = key(&[Grant::ServicesSetup]);
    for (method, params) in [
        ("mail/accounts", json!({})),
        ("mail/add", json!({"name": "work", "backend": "outlook"})),
        ("mail/test", json!({"name": "work"})),
        ("mail/remove", json!({"name": "work"})),
    ] {
        let answer = ask(&control, &reads, method, params.clone());
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
        let answer = ask(&control, &setup, method, params);
        assert_ne!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    assert_eq!(
        Grant::named("services.setup"),
        Some(Grant::ServicesSetup),
        "canoa.cfg can name it"
    );
    let answer = ask(&control, &setup, "mail/accounts", json!({}));
    assert_eq!(answer["result"]["accounts"], json!([]), "{answer}");
    let unknown = ask(&control, &setup, "mail/nothing", json!({}));
    assert_eq!(
        unknown["error"]["code"], -32003,
        "the service says what it has not: {unknown}"
    );
}

#[test]
fn a_host_without_the_service_has_no_such_method_whoever_asks() {
    let none = control(None);
    let bare = control(Some(Services::new(TestHost::without_libraries())));
    for control in [&none, &bare] {
        for caller in [Caller::Owner, key(&[Grant::ServicesSetup])] {
            let answer = ask(control, &caller, "mail/accounts", json!({}));
            assert_eq!(answer["error"]["code"], NO_METHOD, "{answer}");
        }
    }
    assert!(
        Services::new(TestHost::without_libraries())
            .extras()
            .is_empty()
    );
    assert!(!Services::new(TestHost::without_libraries()).offered_any());
}
