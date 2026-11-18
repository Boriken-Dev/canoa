use serde_json::Value;

use super::of;
use crate::profile::items::ItemCatalog;
use crate::profile::items::standin::with_extension;

fn row<'a>(answer: &'a Value, id: &str) -> &'a Value {
    answer["items"]
        .as_array()
        .expect("a list")
        .iter()
        .find(|each| each["id"] == id)
        .unwrap_or_else(|| panic!("no item {id}: {answer}"))
}

#[test]
fn the_list_is_what_the_loaded_providers_say_of_their_items() {
    let answer = of(&with_extension());
    let ids: Vec<&str> = answer["items"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|each| each["id"].as_str().expect("text"))
        .collect();
    assert_eq!(
        ids,
        [
            "access", "context", "effort", "mode", "model", "identity", "proxy", "region"
        ]
    );
    let region = row(&answer, "region");
    assert_eq!(region["origin"], "regions");
    assert_eq!(region["stored"], "item");
    assert_eq!(region["harnesses"], serde_json::json!(["claude-code"]));
    let schema = &region["schema"];
    assert_eq!(schema["required"], serde_json::json!(["name"]));
    assert_eq!(
        schema["properties"]["tier"]["enum"],
        serde_json::json!(["standard", "fast"])
    );
    assert_eq!(schema["properties"]["tier"]["default"], "standard");
    assert_eq!(schema["properties"]["hosts"]["type"], "array");
    assert_eq!(region["form"]["sections"][0]["id"], "where");
    assert_eq!(region["form"]["fields"]["name"]["placeholder"], "eu-west");
    let token = &schema["properties"]["token"];
    assert!(
        token["x-sensitive"] == true || token["writeOnly"] == true,
        "a secret is marked in the schema the form is drawn from: {token}"
    );
    let access = row(&answer, "access");
    assert_eq!(access["stored"], "setting");
    assert_eq!(access["origin"], "built-in");
    assert_eq!(row(&answer, "proxy")["origin"], "bundled");
}

#[test]
fn an_extension_taken_away_takes_its_item_from_the_list() {
    let without = of(&ItemCatalog::built_in());
    assert!(
        without["items"]
            .as_array()
            .expect("a list")
            .iter()
            .all(|each| each["id"] != "region")
    );
    assert!(
        of(&with_extension())["items"]
            .as_array()
            .expect("a list")
            .iter()
            .any(|each| each["id"] == "region")
    );
}

#[test]
fn the_list_is_read_with_the_grant_that_reads_the_configuration() {
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
    let ask = |grants: &[Grant]| {
        let caller = Caller::Peer {
            id: "sha256:aa".to_owned(),
            grants: grants.iter().copied().collect(),
        };
        let line = serde_json::to_vec(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "profile_items/list", "params": {}}))
            .expect("encodes");
        serde_json::from_slice::<Value>(&control.handle(&caller, &line).expect("answered"))
            .expect("JSON")
    };
    let refused = ask(&[Grant::StoreRead]);
    assert_eq!(refused["error"]["code"], DENIED, "{refused}");
    let listed = ask(&[Grant::ConfigRead]);
    assert!(
        listed["result"]["items"]
            .as_array()
            .is_some_and(|items| items.iter().any(|each| each["id"] == "proxy")),
        "{listed}"
    );
}

#[test]
fn the_proxy_item_asks_for_each_pattern_of_hosts_with_the_certificate_to_present_as_a_pick_from_the_hosts_list()
 {
    let answer = of(&ItemCatalog::built_in());
    let proxy = row(&answer, "proxy");
    let hosts = &proxy["schema"]["properties"]["client_certificate_hosts"];
    assert_eq!(hosts["type"], "array", "{hosts}");
    let entry = &hosts["items"];
    let members = &entry["properties"];
    assert_eq!(members["pattern"]["type"], "string", "{entry}");
    assert_eq!(members["identity"]["default"], "system", "{entry}");
    assert_eq!(entry["required"], serde_json::json!(["pattern"]), "{entry}");
    // The form is the ordinary intake one: the list's entries have a form of their own, whose identity member says which widget a
    // renderer draws (a pick from `network/client_certificates`).
    let hints = &proxy["form"]["fields"]["client_certificate_hosts"]["form"]["fields"];
    assert_eq!(hints["identity"]["widget"], "client_certificate", "{proxy}");
}
