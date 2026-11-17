use data_encoding::BASE64;
use std::path::PathBuf;

use super::state::MAX_CHUNK;
use super::*;

fn dir(name: &str) -> PathBuf {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("canoa-state-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("directory");
    dir
}

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    serde_json::from_slice(&control.handle(caller, &line).expect("answered")).expect("JSON")
}

fn open(home: &Path) -> Control {
    Control::open(
        Some(home),
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

fn put(control: &Control, caller: &Caller, name: &str, text: &str) -> Value {
    ask(
        control,
        caller,
        "state/put",
        json!({"name": name, "data": BASE64.encode(text.as_bytes())}),
    )
}

fn read(control: &Control, caller: &Caller, name: &str) -> String {
    let answer = ask(control, caller, "state/get", json!({"name": name}));
    String::from_utf8(
        BASE64
            .decode(answer["result"]["data"].as_str().expect("data").as_bytes())
            .expect("base64"),
    )
    .expect("text")
}

#[test]
fn a_document_survives_the_host_reopening() {
    let home = dir("reopen");
    let owner = Caller::Owner;
    let control = open(&home);
    assert!(put(&control, &owner, "layout.json", "{\"a\":1}")["result"].is_object());
    drop(control);
    let again = open(&home);
    assert_eq!(read(&again, &owner, "layout.json"), "{\"a\":1}");
    let missing = ask(&again, &owner, "state/get", json!({"name": "nothing"}));
    assert_eq!(missing["error"]["code"], NOT_FOUND);
}

#[test]
fn only_the_state_grant_reaches_it() {
    let home = dir("grant");
    let control = open(&home);
    let ledger = key(&[
        Grant::LedgerRead,
        Grant::LedgerWrite,
        Grant::Files,
        Grant::ConfigWrite,
    ]);
    for (method, params) in [
        ("state/get", json!({"name": "a"})),
        ("state/put", json!({"name": "a", "data": ""})),
        ("state/append", json!({"name": "a", "text": "x"})),
        ("state/list", json!({})),
        ("state/remove", json!({"name": "a"})),
    ] {
        assert_eq!(
            ask(&control, &ledger, method, params)["error"]["code"],
            DENIED,
            "{method}"
        );
    }
    let holder = key(&[Grant::State]);
    assert!(put(&control, &holder, "a", "x")["result"].is_object());
    assert_eq!(Grant::named("state"), Some(Grant::State));
    assert!(Grant::all().contains(&Grant::State));
}

#[test]
fn names_cannot_leave_the_state_folder() {
    let home = dir("names");
    let control = open(&home);
    let owner = Caller::Owner;
    for name in [
        "../x",
        "a/../b",
        "/abs",
        ".hidden",
        "a//b",
        "a\\b",
        "C:x",
        "",
        "a/b/c/d/e",
    ] {
        assert_eq!(
            put(&control, &owner, name, "x")["error"]["code"],
            BAD_PARAMS,
            "{name:?}"
        );
    }
    assert!(!home.join("x").exists());
}

#[test]
fn a_document_moves_in_chunks_and_a_cut_one_leaves_the_old() {
    let home = dir("chunks");
    let control = open(&home);
    let owner = Caller::Owner;
    put(&control, &owner, "drafts/ab/blob.bin", "old");
    let big: Vec<u8> = (0..MAX_CHUNK + 10).map(|i| (i % 251) as u8).collect();
    let first = ask(
        &control,
        &owner,
        "state/put",
        json!({"name": "drafts/ab/blob.bin", "data": BASE64.encode(&big[..MAX_CHUNK]), "done": false}),
    );
    assert!(first["result"].is_object());
    assert_eq!(read(&control, &owner, "drafts/ab/blob.bin"), "old");
    let wrong = ask(
        &control,
        &owner,
        "state/put",
        json!({"name": "drafts/ab/blob.bin", "data": "", "offset": 5}),
    );
    assert_eq!(wrong["error"]["code"], REFUSED);
    let last = ask(
        &control,
        &owner,
        "state/put",
        json!({"name": "drafts/ab/blob.bin", "data": BASE64.encode(&big[MAX_CHUNK..]), "offset": MAX_CHUNK}),
    );
    assert!(last["result"].is_object());
    let mut back = Vec::new();
    loop {
        let answer = ask(
            &control,
            &owner,
            "state/get",
            json!({"name": "drafts/ab/blob.bin", "offset": back.len()}),
        );
        back.extend(
            BASE64
                .decode(answer["result"]["data"].as_str().expect("data").as_bytes())
                .expect("base64"),
        );
        if answer["result"]["done"] == true {
            break;
        }
    }
    assert_eq!(back, big);
}

#[test]
fn append_list_and_remove() {
    let home = dir("lists");
    let control = open(&home);
    let owner = Caller::Owner;
    for line in ["one\n", "two\n"] {
        ask(
            &control,
            &owner,
            "state/append",
            json!({"name": "permissions-log.jsonl", "text": line}),
        );
    }
    assert_eq!(
        read(&control, &owner, "permissions-log.jsonl"),
        "one\ntwo\n"
    );
    put(&control, &owner, "drafts/aa/draft.json", "{}");
    put(&control, &owner, "drafts/aa/b.bin", "b");
    put(&control, &owner, "drafts/bb/draft.json", "{}");
    let all = ask(&control, &owner, "state/list", json!({"prefix": "drafts"}));
    assert_eq!(
        all["result"]["names"],
        json!([
            "drafts/aa/b.bin",
            "drafts/aa/draft.json",
            "drafts/bb/draft.json"
        ])
    );
    ask(
        &control,
        &owner,
        "state/remove",
        json!({"name": "drafts/aa"}),
    );
    let left = ask(&control, &owner, "state/list", json!({}));
    assert_eq!(
        left["result"]["names"],
        json!(["drafts/bb/draft.json", "permissions-log.jsonl"])
    );
    assert!(
        ask(
            &control,
            &owner,
            "state/remove",
            json!({"name": "drafts/aa"})
        )["result"]
            .is_object()
    );
}

#[test]
fn a_host_with_no_home_says_so() {
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens");
    let answer = ask(&control, &Caller::Owner, "state/list", json!({}));
    assert_eq!(answer["error"]["code"], REFUSED);
}
