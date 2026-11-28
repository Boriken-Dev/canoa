//! Hosting over the control channel: a key that holds `hosting` lists, pairs and removes keys, shows a PIN that another program
//! pairs with, and edits `[host]`, on a real host with a home of its own. One that does not is refused each of them.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use std::process::Command;

use serde_json::{Value, json};
use support::{Host, WAIT, host_command, key, with_link};

fn control_config(admin: &str, other: &str) -> String {
    format!(
        "[host.control]\nlisten = \"127.0.0.1:0\"\ngrants = {{ \"{admin}\" = [\"hosting\"], \"{other}\" = [\"ledger.read\"] }}\n"
    )
}

fn call(host: &Host, who: &cabuya::auth::Identity, method: &str, params: Value) -> Value {
    with_link(host.url("@control"), who, host.peer, |rpc| {
        match rpc.request(method, params) {
            Ok(result) => json!({"result": result}),
            Err(error) => json!({"error": error}),
        }
    })
    .expect("a link")
}

fn joiner_command(home: &std::path::Path) -> Command {
    let mut command = host_command();
    command
        .env("COQUI_AGENTIC_PEER_HOME", home)
        .env("COQUI_ACP_DESCRIPTORS", home.join("descriptors"))
        .env_remove("CANOA_CONFIG");
    command
}

#[test]
fn a_key_without_the_hosting_grant_is_refused_every_method_of_it() {
    let (admin, other) = (key(), key());
    let host = Host::start(
        &control_config(&admin.peer_id().to_string(), &other.peer_id().to_string()),
        1,
    );
    host.pair(&admin);
    host.pair(&other);
    for (method, params) in [
        ("host/keys", json!({})),
        ("host/accept", json!({"id": "sha256:aa"})),
        ("host/revoke", json!({"id": "sha256:aa"})),
        ("host/pair", json!({})),
        ("config/get", json!({"section": "host"})),
        ("config/set", json!({"section": "host", "value": {}})),
        ("config/check", json!({"section": "host", "value": {}})),
    ] {
        let answer = call(&host, &other, method, params);
        assert_eq!(answer["error"]["code"], -32001, "{method}: {answer}");
    }
}

#[test]
fn a_key_lists_pairs_and_removes_keys_and_sees_what_the_trust_store_holds() {
    let (admin, other, guest) = (key(), key(), key());
    let host = Host::start(
        &control_config(&admin.peer_id().to_string(), &other.peer_id().to_string()),
        1,
    );
    host.pair(&admin);
    // A key that proves itself and is not paired waits, which the person sees and decides.
    let _ = with_link(host.url("@control"), &guest, host.peer, |_| ());
    let guest_id = guest.peer_id().to_string();

    let listed = call(&host, &admin, "host/keys", json!({}));
    let keys = listed["result"]["keys"].as_array().expect("keys");
    let find = |id: &str| keys.iter().find(|key| key["id"] == id).cloned();
    assert_eq!(
        find(&admin.peer_id().to_string()).map(|key| key["state"].clone()),
        Some(json!("accepted")),
        "{listed}"
    );
    let waiting = find(&guest_id).expect("the guest waits");
    assert_eq!(waiting["state"], "waiting", "{listed}");

    // Pairing is the person's choice, by the whole id.
    let bad = call(&host, &admin, "host/accept", json!({"id": "nobody"}));
    assert_eq!(bad["error"]["code"], -32602, "{bad}");
    let unknown = call(
        &host,
        &admin,
        "host/accept",
        json!({"id": format!("sha256:{}", "ab".repeat(32))}),
    );
    assert_eq!(unknown["error"]["code"], -32002, "{unknown}");
    let accepted = call(&host, &admin, "host/accept", json!({"id": guest_id}));
    assert_eq!(accepted["result"]["accepted"], guest_id, "{accepted}");
    let again = call(&host, &admin, "host/accept", json!({"id": guest_id}));
    assert_eq!(again["error"]["code"], -32002, "{again}");

    let revoked = call(&host, &admin, "host/revoke", json!({"id": guest_id}));
    assert_eq!(revoked["result"]["removed"], true, "{revoked}");
    let gone = call(&host, &admin, "host/revoke", json!({"id": guest_id}));
    assert_eq!(gone["result"]["removed"], false, "{gone}");
    let after = call(&host, &admin, "host/keys", json!({}));
    assert!(
        after["result"]["keys"]
            .as_array()
            .expect("keys")
            .iter()
            .all(|key| key["id"] != guest_id),
        "{after}"
    );
}

#[test]
fn a_key_shows_a_pin_and_the_program_that_uses_it_is_paired() {
    let (admin, other) = (key(), key());
    let host = Host::start(
        &control_config(&admin.peer_id().to_string(), &other.peer_id().to_string()),
        1,
    );
    host.pair(&admin);
    let joiner = support::Scratch::new();
    let joiner_id = {
        let printed = joiner_command(&joiner.0)
            .arg("id")
            .output()
            .expect("canoa id");
        assert!(printed.status.success());
        String::from_utf8_lossy(&printed.stdout).trim().to_owned()
    };

    let outcome = with_link(host.url("@control"), &admin, host.peer, |rpc| {
        rpc.send(&json!({"jsonrpc": "2.0", "id": 100, "method": "host/pair", "params": {"timeout_seconds": 60}}))
            .expect("sent");
        let shown = rpc.next_message(WAIT).expect("the PIN");
        assert_eq!(shown["method"], "host/progress", "{shown}");
        assert_eq!(shown["params"]["id"], 100, "{shown}");
        let (pin, url) = (
            shown["params"]["pin"].as_str().expect("a PIN").to_owned(),
            shown["params"]["url"].as_str().expect("a url").to_owned(),
        );
        assert_eq!(shown["params"]["peer"], host.peer.to_string(), "{shown}");

        // The link serves other requests meanwhile, but not a second pairing on it.
        let meanwhile = rpc.request("host/keys", json!({}));
        assert!(meanwhile.is_ok(), "{meanwhile:?}");
        let second = rpc.request("host/pair", json!({}));
        assert_eq!(second.expect_err("refused")["code"], -32002);

        let file = joiner.0.join("pin.txt");
        std::fs::write(&file, format!("{pin}\n")).expect("the PIN file");
        let joined = joiner_command(&joiner.0)
            .args(["pair", "--join", &url, "--pin-file"])
            .arg(&file)
            .output()
            .expect("canoa pair --join");
        assert!(
            joined.status.success(),
            "{}",
            String::from_utf8_lossy(&joined.stderr)
        );
        loop {
            let message = rpc.next_message(WAIT).expect("the answer");
            if message["id"] == 100 {
                return message;
            }
        }
    })
    .expect("a link");
    assert_eq!(outcome["result"]["paired"], joiner_id, "{outcome}");
    let listed = call(&host, &admin, "host/keys", json!({}));
    assert!(
        listed["result"]["keys"]
            .as_array()
            .expect("keys")
            .iter()
            .any(|key| key["id"] == joiner_id && key["state"] == "accepted"),
        "{listed}"
    );
}

#[test]
fn a_pairing_ends_with_its_pin_unused_when_the_link_that_asked_for_it_closes() {
    let (admin, other) = (key(), key());
    let host = Host::start(
        &control_config(&admin.peer_id().to_string(), &other.peer_id().to_string()),
        1,
    );
    host.pair(&admin);
    let url = with_link(host.url("@control"), &admin, host.peer, |rpc| {
        rpc.send(&json!({"jsonrpc": "2.0", "id": 5, "method": "host/pair", "params": {}}))
            .expect("sent");
        let shown = rpc.next_message(WAIT).expect("the PIN");
        shown["params"]["url"].as_str().expect("a url").to_owned()
    })
    .expect("a link");
    // The pairing's listener is gone with it: nothing answers on its port any more.
    let address = url.trim_start_matches("ws://").to_owned();
    let until = std::time::Instant::now() + WAIT;
    loop {
        if std::net::TcpStream::connect(&address).is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "the listener outlived its link"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
fn a_key_with_hosting_edits_the_host_table_and_is_told_what_is_wrong_in_the_hosts_words() {
    let (admin, other) = (key(), key());
    let config = control_config(&admin.peer_id().to_string(), &other.peer_id().to_string());
    let host = Host::start(&config, 1);
    host.pair(&admin);
    let file = host.scratch.0.join("canoa.cfg");
    let before = std::fs::read_to_string(&file).expect("canoa.cfg");

    let read = call(&host, &admin, "config/get", json!({"section": "host"}));
    let mut table = read["result"]["value"].clone();
    assert!(table["control"]["grants"].is_object(), "{read}");
    assert_eq!(read["result"]["restart"], true, "{read}");

    table["origins"] = json!(["https://canoa.example"]);
    let checked = call(
        &host,
        &admin,
        "config/check",
        json!({"section": "host", "value": table}),
    );
    assert_eq!(checked["result"]["valid"], true, "{checked}");
    assert_eq!(std::fs::read_to_string(&file).expect("canoa.cfg"), before);

    let mut wrong = table.clone();
    wrong["origins"] = json!("not a list");
    let refused = call(
        &host,
        &admin,
        "config/set",
        json!({"section": "host", "value": wrong}),
    );
    assert_eq!(refused["error"]["code"], -32002, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|words| words.contains("origins")),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&file).expect("canoa.cfg"), before);

    let set = call(
        &host,
        &admin,
        "config/set",
        json!({"section": "host", "value": table}),
    );
    assert_eq!(set["result"]["restart"], true, "{set}");
    let text = std::fs::read_to_string(&file).expect("canoa.cfg");
    assert!(text.contains("https://canoa.example"), "{text}");
    assert!(
        text.contains(&admin.peer_id().to_string()),
        "the control table stays: {text}"
    );
}
