use super::*;

fn ask(control: &Control, caller: &Caller, method: &str, params: Value) -> Value {
    let line =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .expect("encodes");
    let answer = control.handle(caller, &line).expect("answered");
    serde_json::from_slice(&answer).expect("JSON")
}

fn memory() -> Control {
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
}

fn peer(id: &str, grants: &[Grant]) -> Caller {
    Caller::Peer {
        id: id.to_owned(),
        grants: grants.iter().copied().collect(),
    }
}

fn put(control: &Control, caller: &Caller, id: &str) -> Value {
    ask(
        control,
        caller,
        "ledger/put",
        json!({"id": id, "record": {"title": id}}),
    )
}

fn ids(answer: &Value) -> Vec<&str> {
    answer["result"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["id"].as_str().expect("an id"))
        .collect()
}

#[test]
fn a_record_put_is_listed_replaced_and_removed() {
    let control = memory();
    let put_it = put(&control, &Caller::Owner, "a");
    assert!(
        put_it["result"]["updated"]
            .as_u64()
            .is_some_and(|ms| ms > 0),
        "{put_it}"
    );
    let listed = ask(&control, &Caller::Owner, "ledger/list", json!({}));
    assert_eq!(ids(&listed), ["a"]);
    assert_eq!(listed["result"]["records"][0]["owner"], "local");
    assert_eq!(
        listed["result"]["records"][0]["record"],
        json!({"title": "a"})
    );
    ask(
        &control,
        &Caller::Owner,
        "ledger/put",
        json!({"id": "a", "record": {"title": "again"}}),
    );
    let again = ask(&control, &Caller::Owner, "ledger/list", json!({}));
    assert_eq!(
        again["result"]["records"][0]["record"],
        json!({"title": "again"})
    );
    let removed = ask(
        &control,
        &Caller::Owner,
        "ledger/remove",
        json!({"id": "a"}),
    );
    assert_eq!(removed["result"], json!({"removed": true}));
    let gone = ask(
        &control,
        &Caller::Owner,
        "ledger/remove",
        json!({"id": "a"}),
    );
    assert_eq!(gone["result"], json!({"removed": false}));
    assert!(ids(&ask(&control, &Caller::Owner, "ledger/list", json!({}))).is_empty());
}

#[test]
fn the_list_is_newest_first_in_pages_that_continue_where_the_last_ended() {
    let control = memory();
    for id in ["a", "b", "c", "d", "e"] {
        put(&control, &Caller::Owner, id);
        std::thread::sleep(Duration::from_millis(3));
    }
    let first = ask(&control, &Caller::Owner, "ledger/list", json!({"limit": 2}));
    assert_eq!(ids(&first), ["e", "d"]);
    assert_eq!(first["result"]["next"], "d");
    let second = ask(
        &control,
        &Caller::Owner,
        "ledger/list",
        json!({"limit": 2, "after": "d"}),
    );
    assert_eq!(ids(&second), ["c", "b"]);
    let last = ask(
        &control,
        &Caller::Owner,
        "ledger/list",
        json!({"limit": 2, "after": "b"}),
    );
    assert_eq!(ids(&last), ["a"]);
    assert!(last["result"].get("next").is_none(), "{last}");
}

#[test]
fn a_key_sees_and_changes_only_its_own_records_and_the_owner_all() {
    let control = memory();
    let (a, b) = (
        peer("a", &[Grant::LedgerRead, Grant::LedgerWrite]),
        peer("b", &[Grant::LedgerRead, Grant::LedgerWrite]),
    );
    put(&control, &Caller::Owner, "mine");
    put(&control, &a, "from-a");
    put(&control, &b, "from-b");
    assert_eq!(
        ids(&ask(&control, &a, "ledger/list", json!({}))),
        ["from-a"]
    );
    assert_eq!(
        ids(&ask(&control, &b, "ledger/list", json!({}))),
        ["from-b"]
    );
    assert_eq!(
        ask(&control, &Caller::Owner, "ledger/list", json!({}))["result"]["records"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    // Another key's id is denied for a write, and is not there to remove.
    let taken = put(&control, &b, "from-a");
    assert_eq!(taken["error"]["code"], DENIED, "{taken}");
    let none = ask(&control, &b, "ledger/remove", json!({"id": "from-a"}));
    assert_eq!(none["result"], json!({"removed": false}));
    assert_eq!(
        ids(&ask(&control, &a, "ledger/list", json!({}))),
        ["from-a"]
    );
    // A key owns what it puts, whatever the request says.
    let sly = ask(
        &control,
        &a,
        "ledger/put",
        json!({"id": "sly", "owner": "local", "record": {}}),
    );
    assert!(sly.get("result").is_some());
    let listed = ask(&control, &Caller::Owner, "ledger/list", json!({}));
    let owner_of_sly = listed["result"]["records"]
        .as_array()
        .and_then(|all| all.iter().find(|record| record["id"] == "sly"))
        .map(|record| record["owner"].clone());
    assert_eq!(owner_of_sly, Some(json!("a")));
    // clear takes the caller's own only.
    assert_eq!(
        ask(&control, &a, "ledger/clear", json!({}))["result"],
        json!({"removed": 2})
    );
    assert_eq!(
        ids(&ask(&control, &b, "ledger/list", json!({}))),
        ["from-b"]
    );
}

#[test]
fn a_key_does_only_what_its_grants_say() {
    let control = memory();
    let reader = peer("r", &[Grant::LedgerRead]);
    let writer = peer("w", &[Grant::LedgerWrite]);
    let nothing = peer("n", &[]);
    assert!(
        ask(&control, &reader, "ledger/list", json!({}))
            .get("result")
            .is_some()
    );
    assert_eq!(put(&control, &reader, "x")["error"]["code"], DENIED);
    assert!(put(&control, &writer, "x").get("result").is_some());
    assert_eq!(
        ask(&control, &writer, "ledger/list", json!({}))["error"]["code"],
        DENIED
    );
    for method in ["ledger/list", "ledger/put", "ledger/remove", "ledger/clear"] {
        assert_eq!(
            ask(&control, &nothing, method, json!({"id": "x", "record": {}}))["error"]["code"],
            DENIED,
            "{method}"
        );
    }
}

#[test]
fn what_is_not_asked_properly_is_refused_naming_why() {
    let control = memory();
    let owner = Caller::Owner;
    for params in [
        json!({"record": {}}),
        json!({"id": "", "record": {}}),
        json!({"id": "a\nb", "record": {}}),
        json!({"id": "x".repeat(MAX_ID + 1), "record": {}}),
        json!({"id": "a", "record": []}),
        json!({"id": 1, "record": {}}),
    ] {
        let answer = ask(&control, &owner, "ledger/put", params.clone());
        assert_eq!(answer["error"]["code"], BAD_PARAMS, "{params}: {answer}");
    }
    assert_eq!(
        ask(&control, &owner, "ledger/list", json!({"limit": 0}))["error"]["code"],
        BAD_PARAMS
    );
    assert_eq!(
        ask(
            &control,
            &owner,
            "ledger/list",
            json!({"limit": MAX_PAGE + 1})
        )["error"]["code"],
        BAD_PARAMS
    );
    assert_eq!(
        ask(&control, &owner, "no/such", json!({}))["error"]["code"],
        NO_METHOD
    );
    let not_json = control.handle(&owner, b"not json").expect("answered");
    assert_eq!(
        serde_json::from_slice::<Value>(&not_json).expect("JSON")["error"]["code"],
        NOT_JSON
    );
    assert!(
        control
            .handle(&owner, br#"{"jsonrpc":"2.0","method":"ledger/clear"}"#)
            .is_none(),
        "a notification is not answered"
    );
}

#[test]
fn the_limits_are_refused_at_not_dropped_to() {
    let control = Control::open(None, 2, 64, Vec::new()).expect("opens");
    let owner = Caller::Owner;
    put(&control, &owner, "a");
    put(&control, &owner, "b");
    let full = put(&control, &owner, "c");
    assert_eq!(full["error"]["code"], REFUSED, "{full}");
    assert!(
        put(&control, &owner, "a").get("result").is_some(),
        "a record that is there can still be replaced"
    );
    let big = ask(
        &control,
        &owner,
        "ledger/put",
        json!({"id": "a", "record": {"t": "x".repeat(100)}}),
    );
    assert_eq!(big["error"]["code"], REFUSED, "{big}");
    assert_eq!(
        ids(&ask(&control, &owner, "ledger/list", json!({}))).len(),
        2
    );
}

#[test]
fn the_ledger_is_kept_across_a_reopen_and_a_file_that_is_not_one_is_said() {
    let dir = std::env::temp_dir().join(format!("canoa-control-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    {
        let control = Control::open(
            Some(&dir),
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens");
        put(&control, &Caller::Owner, "kept");
        put(&control, &peer("k", &[Grant::LedgerWrite]), "theirs");
    }
    let again = Control::open(
        Some(&dir),
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens again");
    let listed = ask(&again, &Caller::Owner, "ledger/list", json!({}));
    let mut found = ids(&listed);
    found.sort_unstable();
    assert_eq!(found, ["kept", "theirs"]);
    fs::write(dir.join("ledger.json"), "[1, 2]").expect("written");
    let error = Control::open(Some(&dir), 1, 1, Vec::new()).expect_err("not a ledger");
    assert!(error.to_string().contains("ledger.json"), "{error}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_connection_is_answered_until_it_closes() {
    let control = memory();
    let stopped = alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false));
    let (client, host) = crate::local::pair(MAX_REQUEST, stopped);
    let serving = std::thread::scope(|scope| {
        scope.spawn(|| serve(&control, &Caller::Owner, &host));
        client
            .send(br#"{"jsonrpc":"2.0","id":7,"method":"ledger/list"}"#)
            .expect("sent");
        let answer = client
            .receive(Some(Duration::from_secs(5)))
            .expect("answered")
            .expect("a message");
        client.close();
        serde_json::from_slice::<Value>(&answer).expect("JSON")
    });
    assert_eq!(serving["id"], 7);
    assert_eq!(serving["result"]["records"], json!([]));
}

fn with_roots(entries: Vec<EntryRoots>) -> Control {
    Control::open(None, DEFAULT_MAX_RECORDS, DEFAULT_MAX_RECORD_BYTES, entries).expect("opens")
}

fn missing_path() -> String {
    std::env::temp_dir()
        .join("canoa-control-test-nothing-here")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn a_key_without_the_worktree_grant_is_denied() {
    let control = with_roots(Vec::new());
    let answer = ask(
        &control,
        &peer("sha256:aa", &[Grant::LedgerRead]),
        "worktree/status",
        json!({"path": missing_path()}),
    );
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
}

#[test]
fn the_owner_asks_about_any_path_on_the_computer() {
    let control = with_roots(Vec::new());
    let answer = ask(
        &control,
        &Caller::Owner,
        "worktree/status",
        json!({"path": missing_path()}),
    );
    assert_eq!(answer["result"]["exists"], false, "{answer}");
}

#[test]
fn a_key_reaches_only_the_roots_of_the_entries_it_may_use() {
    let key = "sha256:aa";
    let other = "sha256:bb";
    let control = with_roots(vec![
        EntryRoots {
            peers: Some(vec![key.to_owned()]),
            roots: vec![std::env::temp_dir()],
        },
        EntryRoots {
            peers: Some(vec![other.to_owned()]),
            roots: Vec::new(),
        },
    ]);
    let params = json!({"path": missing_path()});
    let granted = [Grant::Worktree];
    let inside = ask(
        &control,
        &peer(key, &granted),
        "worktree/status",
        params.clone(),
    );
    assert_eq!(inside["result"]["exists"], false, "{inside}");
    let outside = ask(&control, &peer(other, &granted), "worktree/status", params);
    assert_eq!(outside["error"]["code"], BAD_PARAMS, "{outside}");
}

#[test]
fn only_the_owner_installs_updates_rolls_back_or_removes() {
    let control = memory();
    let reader = peer("sha256:aa", &[Grant::StoreRead]);
    for method in ["store/rollback", "store/remove"] {
        let answer = ask(
            &control,
            &reader,
            method,
            json!({"provider": "claude_store"}),
        );
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let apply = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 1, "method": "store/apply",
        "params": {"provider": "claude_store", "plan": {}},
    }))
    .expect("encodes");
    let answer: Value =
        serde_json::from_slice(&control.handle(&reader, &apply).expect("answered")).expect("JSON");
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    let blind = peer("sha256:bb", &[Grant::LedgerRead]);
    let answer = ask(&control, &blind, "store/list", json!({}));
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    assert!(
        Grant::named("store.write").is_none(),
        "not a grant a key can hold"
    );
    assert_eq!(Grant::named("store.read"), Some(Grant::StoreRead));
}
