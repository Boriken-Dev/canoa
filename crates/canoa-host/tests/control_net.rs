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
        "[host.control]\nmax_upload_bytes = 0\n".to_owned(),
        "[host.control]\nmax_uploads = 0\n".to_owned(),
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

#[test]
fn a_page_browses_uploads_edits_a_section_and_restarts_the_host_as_far_as_its_grants_say() {
    use base64_for_test::encode;

    let (writer, reader) = (key(), key());
    let config = format!(
        "[host.control]\nlisten = \"127.0.0.1:0\"\nmax_upload_bytes = 64\ngrants = {{ \"{}\" = [\"folders\", \"config.read\", \"config.write\", \"upload\", \"host.restart\"], \"{}\" = [\"folders\"] }}\n",
        writer.peer_id(),
        reader.peer_id()
    );
    let scratch = support::Scratch::new();
    let user = scratch.0.join("user");
    std::fs::create_dir_all(user.join("projects")).expect("a home");
    let mut host = Host::start_in_with(
        scratch,
        &config,
        1,
        &["--supervised"],
        &[
            ("HOME", user.clone().into_os_string()),
            ("USERPROFILE", user.clone().into_os_string()),
        ],
    );
    host.pair(&writer);
    host.pair(&reader);
    let url = host.url("@control").to_owned();
    let home = user.to_string_lossy().into_owned();

    // Folders, for a key that has only that.
    let listed = call(&url, &reader, &host, "fs/list", json!({"path": home})).expect("a link");
    assert_eq!(
        listed["result"]["entries"][0]["name"], "projects",
        "{listed}"
    );
    for (method, params) in [
        ("config/get", json!({"section": "sessions"})),
        ("upload/put", json!({"name": "a", "data": ""})),
        ("host/restart", json!({})),
    ] {
        let denied = call(&url, &reader, &host, method, params).expect("a link");
        assert_eq!(denied["error"]["code"], -32001, "{method}: {denied}");
    }
    let outside = host.scratch.0.to_string_lossy().into_owned();
    let refused = call(&url, &reader, &host, "fs/list", json!({"path": outside})).expect("a link");
    assert!(refused["error"].is_object(), "{refused}");

    // A section is written into the person's own canoa.cfg, and the file still serves.
    let set = call(
        &url,
        &writer,
        &host,
        "config/set",
        json!({"section": "sessions", "value": {"keep": 3}}),
    )
    .expect("a link");
    assert_eq!(set["result"]["section"], "sessions", "{set}");
    let text = std::fs::read_to_string(host.scratch.0.join("canoa.cfg")).expect("canoa.cfg");
    assert!(
        text.contains("[sessions]") && text.contains("keep = 3"),
        "{text}"
    );
    assert!(text.contains("max_upload_bytes"), "the rest stays: {text}");
    let host_table = call(
        &url,
        &writer,
        &host,
        "config/set",
        json!({"section": "host", "value": {}}),
    )
    .expect("a link");
    assert_eq!(host_table["error"]["code"], -32001, "{host_table}");

    // An upload is held to the setting.
    let small = call(
        &url,
        &writer,
        &host,
        "upload/put",
        json!({"name": "note.txt", "data": encode(b"hi")}),
    )
    .expect("a link");
    let kept = small["result"]["path"].as_str().expect("a path");
    assert_eq!(std::fs::read(kept).expect("kept"), b"hi");
    let large = call(
        &url,
        &writer,
        &host,
        "upload/put",
        json!({"name": "big", "data": encode(&[0_u8; 65])}),
    )
    .expect("a link");
    assert_eq!(large["error"]["code"], -32002, "{large}");

    // A restart ends the host with the status a supervisor starts it again on.
    let restart = call(&url, &writer, &host, "host/restart", json!({})).expect("a link");
    assert_eq!(restart["result"]["restarting"], true, "{restart}");
    let status = host
        .ended_within(std::time::Duration::from_secs(20))
        .expect("the host ends");
    assert_eq!(status.code(), Some(75), "{status:?}");
}

mod base64_for_test {
    pub fn encode(bytes: &[u8]) -> String {
        data_encoding::BASE64.encode(bytes)
    }
}
