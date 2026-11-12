//! The control channel from a network: a paired key reaches it only when `[host.control]` lists it, and does only what its
//! grants say, on the records it owns.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod support;

use serde_json::{Value, json};
use support::{Host, key, with_link};

/// The answer to `method`, whole: `{"result"}` or `{"error"}`.
fn call(
    url: &str,
    who: &coqui_agentic_peer::auth::Identity,
    host: &Host,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    with_link(url, who, host.peer, |rpc| {
        match rpc.request(method, params) {
            Ok(result) => json!({"result": result}),
            Err(error) => json!({"error": error}),
        }
    })
}

#[test]
fn a_paired_key_reaches_the_ledger_only_as_far_as_its_grants_and_its_own_records() {
    let (reader, writer, stranger, unpaired) = (key(), key(), key(), key());
    let config = format!(
        "[host.control]\nlisten = \"127.0.0.1:0\"\ngrants = {{ \"{}\" = [\"ledger.read\"], \"{}\" = [\"ledger.read\", \"ledger.write\"] }}\n",
        reader.peer_id(),
        writer.peer_id()
    );
    let host = Host::start(&config, 1);
    for paired in [&reader, &writer, &stranger] {
        host.pair(paired);
    }
    let url = host.url("@control").to_owned();

    // A key with a read grant lists, and cannot write.
    let listed = call(&url, &reader, &host, "ledger/list", json!({})).expect("a link");
    assert_eq!(listed["result"]["records"], json!([]), "{listed}");
    let denied = call(
        &url,
        &reader,
        &host,
        "ledger/put",
        json!({"id": "x", "record": {}}),
    )
    .expect("a link");
    assert_eq!(denied["error"]["code"], -32001, "{denied}");

    // A key with both writes, and sees what it wrote and only that.
    let put = call(
        &url,
        &writer,
        &host,
        "ledger/put",
        json!({"id": "mine", "record": {"title": "T"}}),
    )
    .expect("a link");
    assert!(put["result"]["updated"].is_u64(), "{put}");
    let mine = call(&url, &writer, &host, "ledger/list", json!({})).expect("a link");
    assert_eq!(mine["result"]["records"][0]["id"], "mine");
    assert_eq!(
        mine["result"]["records"][0]["owner"],
        writer.peer_id().to_string()
    );
    // The key that may only read has none of its own: it does not see the writer's.
    let theirs = call(&url, &reader, &host, "ledger/list", json!({})).expect("a link");
    assert_eq!(theirs["result"]["records"], json!([]), "{theirs}");

    // A paired key the file does not name is turned away after its proof; an unpaired one never gets that far.
    for turned_away in [&stranger, &unpaired] {
        let refused = call(&url, turned_away, &host, "ledger/list", json!({}));
        assert!(
            !matches!(&refused, Ok(answer) if answer.get("result").is_some()),
            "{refused:?}"
        );
    }
}

#[test]
fn a_control_table_that_would_turn_everyone_away_or_names_what_is_unknown_is_refused() {
    let who = key();
    for config in [
        "[host.control]\nlisten = \"127.0.0.1:0\"\n".to_owned(),
        format!(
            "[host.control]\ngrants = {{ \"{}\" = [\"worktree.everything\"] }}\n",
            who.peer_id()
        ),
        "[host.control]\ngrants = { \"not a key\" = [] }\n".to_owned(),
        "[host.control]\nmax_records = 0\n".to_owned(),
        "[host.control]\nlisten = \"127.0.0.1:0\"\nsurprise = 1\n".to_owned(),
    ] {
        let parsed = canoa_host::Config::from_str_in(&config, std::path::Path::new("."));
        assert!(parsed.is_err(), "{config}");
    }
    let fine = canoa_host::Config::from_str_in(
        "[host.control]\nmax_records = 5\n",
        std::path::Path::new("."),
    )
    .expect("parses");
    assert_eq!(fine.control.map(|control| control.max_records), Some(5));
}
