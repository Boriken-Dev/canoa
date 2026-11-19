use serde_json::{Value, json};

use super::{list, tail};
use crate::panel_tabs::TabCatalog;
use crate::panel_tabs::standin::with_extension;

fn ids(answer: &Value) -> Vec<String> {
    answer["tabs"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|each| each["id"].as_str().expect("text").to_owned())
        .collect()
}

fn row<'a>(answer: &'a Value, id: &str) -> &'a Value {
    answer["tabs"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["id"] == id)
        .unwrap_or_else(|| panic!("no tab {id}: {answer}"))
}

#[test]
fn the_hosts_terminal_and_log_are_providers_like_an_extensions_and_the_list_has_them_all() {
    let answer = list(&with_extension());
    assert_eq!(ids(&answer), ["terminal", "logs", "buildlog", "devshell"]);
    assert_eq!(row(&answer, "terminal")["origin"], "built-in");
    assert_eq!(row(&answer, "terminal")["shows"], "terminal");
    assert_eq!(row(&answer, "terminal")["shell"], Value::Null);
    assert_eq!(row(&answer, "logs")["shows"], "log");
    let dev = row(&answer, "devshell");
    assert_eq!(dev["origin"], "builds");
    assert_eq!(dev["shell"], "git-bash");
    assert_eq!(dev["icon"], "code");
}

#[test]
fn an_extension_taken_away_takes_its_kinds_from_the_list() {
    let without = list(&TabCatalog::from_registries(Vec::new()));
    assert_eq!(ids(&without), ["terminal", "logs"]);
}

#[test]
fn a_log_kind_answers_its_latest_lines_and_a_kind_that_shows_none_or_is_unknown_is_refused() {
    let catalog = with_extension();
    let two = tail(&catalog, &json!({"id": "buildlog", "lines": 2})).expect("answered");
    assert_eq!(two["text"], "step 2\nstep 3\n");
    crate::host_log::record("panel_tabs test line");
    let host = tail(&catalog, &json!({"id": "logs", "lines": 5})).expect("answered");
    assert!(
        host["text"]
            .as_str()
            .is_some_and(|text| text.contains("panel_tabs test line")),
        "the host's own log is the Logs kind's: {host}"
    );
    let terminal = tail(&catalog, &json!({"id": "terminal"})).expect_err("shows no log");
    assert!(terminal.1.contains("shows no log"), "{}", terminal.1);
    let missing = tail(&catalog, &json!({"id": "nothing"})).expect_err("unknown");
    assert!(missing.1.contains("no panel tab"), "{}", missing.1);
}

#[test]
fn listing_needs_the_grant_that_reads_the_configuration_and_the_log_the_one_that_hosts() {
    use crate::control::{
        Caller, Control, DEFAULT_MAX_RECORD_BYTES, DEFAULT_MAX_RECORDS, DENIED, Grant,
    };
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let ask = |grants: &[Grant], method: &str, params: Value| {
        let caller = Caller::Peer {
            id: "sha256:aa".to_owned(),
            grants: grants.iter().copied().collect(),
        };
        let line = serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        )
        .expect("encodes");
        serde_json::from_slice::<Value>(&control.handle(&caller, &line).expect("answered"))
            .expect("JSON")
    };
    let refused = ask(&[Grant::StoreRead], "panel_tabs/list", json!({}));
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let listed = ask(&[Grant::ConfigRead], "panel_tabs/list", json!({}));
    assert_eq!(ids(&listed["result"]), ["terminal", "logs"], "{listed}");
    let refused = ask(
        &[Grant::ConfigRead],
        "panel_tabs/tail",
        json!({"id": "logs"}),
    );
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let tailed = ask(&[Grant::Hosting], "panel_tabs/tail", json!({"id": "logs"}));
    assert!(tailed["result"]["text"].is_string(), "{tailed}");
}
