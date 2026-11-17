use super::*;

fn ask(control: &Control, caller: &Caller, method: &str) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {}}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn control() -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
}

fn key(grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

const TOKENS: [&str; 13] = [
    "page", "rail", "bar", "selected", "surface", "raised", "text", "muted", "accent", "onAccent",
    "success", "warning", "danger",
];

#[test]
fn looks_list_answers_both_vendor_documents_to_a_reader_of_the_config() {
    let answer = ask(&control(), &key(&[Grant::ConfigRead]), "looks/list");
    let looks = answer["result"]["looks"]
        .as_array()
        .unwrap_or_else(|| panic!("{answer}"));
    let ids: Vec<_> = looks.iter().map(|look| look["id"].as_str()).collect();
    assert_eq!(ids, [Some("claude"), Some("chatgpt")]);
    let vendors: Vec<_> = looks.iter().map(|look| look["vendor"].as_str()).collect();
    assert_eq!(vendors, [Some("claude"), Some("codex")]);
    for look in looks {
        for mode in ["light", "dark"] {
            for token in TOKENS {
                let value = look[mode][token].as_str().unwrap_or_default();
                assert!(
                    value.len() == 7 && value.starts_with('#'),
                    "{look}: {mode}.{token} is {value:?}"
                );
            }
            for number in ["radius", "controlRadius", "railWidth"] {
                assert!(look[mode][number].is_number(), "{mode}.{number}");
            }
            assert!(look[mode]["chat"]["user"].is_string(), "{mode}.chat.user");
        }
    }
}

#[test]
fn looks_list_is_refused_to_a_key_without_config_read() {
    let answer = ask(&control(), &key(&[Grant::LedgerRead]), "looks/list");
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
}
