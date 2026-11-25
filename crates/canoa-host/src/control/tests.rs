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
    // The app's own list is its own records: what a key put is never taken for a session. It asks for all to see them.
    assert_eq!(
        ids(&ask(&control, &Caller::Owner, "ledger/list", json!({}))),
        ["mine"]
    );
    assert_eq!(
        ask(
            &control,
            &Caller::Owner,
            "ledger/list",
            json!({"all": true})
        )["result"]["records"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    // A key cannot ask for all.
    assert_eq!(
        ids(&ask(&control, &a, "ledger/list", json!({"all": true}))),
        ["from-a"]
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
    let listed = ask(
        &control,
        &Caller::Owner,
        "ledger/list",
        json!({"all": true}),
    );
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
    let listed = ask(&again, &Caller::Owner, "ledger/list", json!({"all": true}));
    let mut found = ids(&listed);
    found.sort_unstable();
    assert_eq!(found, ["kept", "theirs"]);
    // A file that is not a ledger does not stop the host: it is kept beside and a new ledger begins.
    fs::write(dir.join("ledger.json"), "[1, 2]").expect("written");
    let fresh = Control::open(Some(&dir), 1, 1, Vec::new()).expect("opens empty");
    assert!(
        ids(&ask(
            &fresh,
            &Caller::Owner,
            "ledger/list",
            json!({"all": true})
        ))
        .is_empty()
    );
    let aside = fs::read_dir(&dir)
        .expect("listed")
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains("json.bad-"));
    assert!(aside, "the unreadable file is kept");
    put(&fresh, &Caller::Owner, "after");
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
fn only_the_person_commits_pushes_or_opens_a_pull_request_through_the_control_link() {
    let key = "sha256:aa";
    let control = with_roots(vec![EntryRoots {
        peers: None,
        roots: vec![std::env::temp_dir()],
    }]);
    let keyed = peer(key, &[Grant::Worktree]);
    for (method, params) in [
        (
            "worktree/commit",
            json!({"path": missing_path(), "message": "m"}),
        ),
        ("worktree/push", json!({"path": missing_path()})),
        (
            "worktree/pull_request",
            json!({"path": missing_path(), "title": "t"}),
        ),
    ] {
        let denied = ask(&control, &keyed, method, params.clone());
        assert_eq!(denied["error"]["code"], DENIED, "{method}: {denied}");
        // The person's request reaches the repository checks: a folder that is not there is said so.
        let owner = ask(&control, &Caller::Owner, method, params);
        assert_eq!(owner["error"]["code"], BAD_PARAMS, "{method}: {owner}");
        assert_eq!(
            owner["error"]["message"], "that directory does not exist",
            "{method}: {owner}"
        );
    }
}

#[test]
fn a_key_installs_updates_rolls_back_or_removes_only_with_the_programs_grant() {
    let control = memory();
    let reader = peer("sha256:aa", &[Grant::StoreRead]);
    let installer = peer("sha256:cc", &[Grant::StoreRead, Grant::StorePrograms]);
    for method in ["store/rollback", "store/remove"] {
        let params = json!({"provider": "claude_store"});
        let answer = ask(&control, &reader, method, params.clone());
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
        // Past the gate; this control has no stores, so the host says that.
        let answer = ask(&control, &installer, method, params);
        assert_ne!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
    let probe = json!({"provider": "claude_store", "probe": true});
    let answer = ask(&control, &reader, "store/report", probe.clone());
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    let answer = ask(&control, &installer, "store/report", probe);
    assert_ne!(answer["error"]["code"], DENIED, "{answer}");
    let blind = peer("sha256:bb", &[Grant::LedgerRead]);
    let answer = ask(&control, &blind, "store/list", json!({}));
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    // The programs grant does not read the store, and reading it does not install.
    let only_installs = peer("sha256:dd", &[Grant::StorePrograms]);
    let answer = ask(&control, &only_installs, "store/list", json!({}));
    assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    assert!(
        Grant::named("store.write").is_none(),
        "not a grant a key can hold"
    );
    assert_eq!(Grant::named("store.read"), Some(Grant::StoreRead));
    assert_eq!(Grant::named("store.programs"), Some(Grant::StorePrograms));
    assert_eq!(Grant::named("accounts"), Some(Grant::Accounts));
}

#[test]
fn a_key_is_told_the_grants_it_holds_and_the_owner_all() {
    let control = memory();
    let key = peer("sha256:aa", &[Grant::LedgerRead, Grant::Accounts]);
    let answer = ask(&control, &key, "host/grants", json!({}));
    assert_eq!(
        answer["result"]["grants"],
        json!(["ledger.read", "accounts"])
    );
    let nothing = peer("sha256:bb", &[]);
    let answer = ask(&control, &nothing, "host/grants", json!({}));
    assert_eq!(answer["result"]["grants"], json!([]));
    let owner = ask(&control, &Caller::Owner, "host/grants", json!({}));
    assert_eq!(
        owner["result"]["grants"].as_array().map(Vec::len),
        Some(Grant::all().len())
    );
}

#[test]
fn shell_list_names_the_shells_found_the_choice_and_what_each_agent_is_told() {
    use crate::shells::{AgentShell, Plan};
    let own = if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_owned())
    } else {
        "/bin/sh".to_owned()
    };
    let plan = Plan::new(
        Some("canoa-no-such-shell-3117".to_owned()),
        vec![
            AgentShell {
                entry: "Claude".to_owned(),
                harness: "claude".to_owned(),
                own: Some(own),
            },
            AgentShell {
                entry: "Codex".to_owned(),
                harness: "codex".to_owned(),
                own: None,
            },
        ],
    );
    let control = memory().with_shells(plan);
    let answer = ask(&control, &Caller::Owner, "shell/list", json!({}));
    let report = &answer["result"];
    assert!(report["shells"].is_array(), "{answer}");
    assert_eq!(report["setting"], "canoa-no-such-shell-3117");
    assert!(
        report["problem"]
            .as_str()
            .is_some_and(|why| why.contains("canoa-no-such-shell-3117")),
        "a host choice that is not here is said: {answer}"
    );
    let agents = report["agents"].as_array().expect("agents");
    // The two entries come first; every other agent the host's providers describe has a row of its own, not hosted.
    let hosted: Vec<_> = agents.iter().filter(|row| row["hosted"] == true).collect();
    assert_eq!(hosted.len(), 2, "{agents:?}");
    let vendors = crate::vendors::all();
    assert!(vendors.len() > 2, "the bundled agents are described");
    for vendor in &vendors {
        assert!(
            agents.iter().any(|row| row["harness"] == vendor.agent),
            "{} has a row: {agents:?}",
            vendor.agent
        );
    }
    assert_eq!(
        agents.len(),
        2 + vendors
            .iter()
            .filter(|v| v.agent != "claude" && v.agent != "codex")
            .count()
    );
    assert!(
        agents.iter().all(|row| row["title"].is_string()),
        "{agents:?}"
    );
    assert_eq!(agents[0]["own"], true);
    assert_eq!(
        agents[0]["id"], "custom",
        "a program is a custom shell: {agents:?}"
    );
    assert!(agents[0]["notice"].is_string(), "{agents:?}");
    assert_eq!(agents[1]["own"], false);
    assert!(agents[1]["problem"].is_string(), "{agents:?}");
    // What a terminal could start is for a key that may open one or may read what is installed here.
    for (grant, allowed) in [
        (Some(Grant::Terminal), true),
        (Some(Grant::StoreRead), true),
        (Some(Grant::LedgerRead), false),
        (None, false),
    ] {
        let grants: Vec<Grant> = grant.into_iter().collect();
        let key = peer("sha256:aa", &grants);
        let answer = ask(&control, &key, "shell/list", json!({}));
        assert_eq!(
            answer.get("result").is_some(),
            allowed,
            "{grant:?}: {answer}"
        );
    }
}

/// Sends `requests` on a link served as `caller`, and the answer to each.
fn over_a_link(control: &Control, caller: &Caller, requests: &[(&str, Value)]) -> Vec<Value> {
    let stopped = alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false));
    let (client, host) = crate::local::pair(MAX_REQUEST, stopped);
    std::thread::scope(|scope| {
        scope.spawn(|| serve(control, caller, &host));
        let mut answers = Vec::new();
        for (id, (method, params)) in requests.iter().enumerate() {
            client
                .send(
                    &serde_json::to_vec(
                        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
                    )
                    .expect("encodes"),
                )
                .expect("sent");
            let answer = client
                .receive(Some(Duration::from_secs(5)))
                .expect("answered")
                .expect("a message");
            answers.push(serde_json::from_slice::<Value>(&answer).expect("JSON"));
        }
        client.close();
        answers
    })
}

#[test]
fn a_key_on_a_connection_is_refused_an_apply_without_the_programs_grant() {
    let control = memory();
    let apply = [(
        "store/apply",
        json!({"provider": "claude_store", "plan": {}}),
    )];
    let without = peer("sha256:aa", &[Grant::StoreRead, Grant::Accounts]);
    let refused = over_a_link(&control, &without, &apply);
    assert_eq!(refused[0]["error"]["code"], DENIED, "{refused:?}");
    let with = peer("sha256:bb", &[Grant::StorePrograms]);
    let allowed = over_a_link(&control, &with, &apply);
    assert_ne!(allowed[0]["error"]["code"], DENIED, "{allowed:?}");
}

#[test]
fn a_key_has_a_quota_of_its_own_and_cannot_fill_the_app_out() {
    let control = Control::open(None, 2, DEFAULT_MAX_RECORD_BYTES, Vec::new()).expect("opens");
    let key = peer("k", &[Grant::LedgerWrite, Grant::LedgerRead]);
    put(&control, &key, "1");
    put(&control, &key, "2");
    let full = put(&control, &key, "3");
    assert_eq!(full["error"]["code"], REFUSED, "{full}");
    // The key's records do not count against the app's.
    assert!(
        put(&control, &Caller::Owner, "mine-1")
            .get("result")
            .is_some()
    );
    assert!(
        put(&control, &Caller::Owner, "mine-2")
            .get("result")
            .is_some()
    );
    let more = put(&control, &Caller::Owner, "mine-3");
    assert_eq!(more["error"]["code"], REFUSED, "{more}");
}

#[test]
fn the_whole_ledger_has_a_byte_cap_with_room_kept_for_the_app() {
    let control = memory();
    let text = "x".repeat(200 * 1024);
    let mut taken = 0;
    // Owners are held to their own bytes, so several keys fill the part of the ledger a key may use.
    'keys: for n in 0..8 {
        let key = peer(&format!("k{n}"), &[Grant::LedgerWrite]);
        for i in 0..400 {
            let answer = ask(
                &control,
                &key,
                "ledger/put",
                json!({"id": format!("k{n}-r{i}"), "record": {"t": text}}),
            );
            if answer.get("error").is_some() {
                continue 'keys;
            }
            taken += 1;
        }
    }
    assert!(
        taken * 200 * 1024 <= TOTAL_BYTES - OWNER_RESERVE + 200 * 1024,
        "{taken}"
    );
    let mine = ask(
        &control,
        &Caller::Owner,
        "ledger/put",
        json!({"id": "mine", "record": {"t": text}}),
    );
    assert!(mine.get("result").is_some(), "{mine}");
}

#[test]
fn a_key_may_not_run_a_program_through_store_report() {
    let control = memory();
    let key = peer("k", &[Grant::StoreRead]);
    for params in [
        json!({"provider": "claude_store", "probe": true}),
        json!({"provider": "claude_store", "pick": "x"}),
    ] {
        let answer = ask(&control, &key, "store/report", params);
        assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    }
}

#[test]
fn a_ledger_the_host_cannot_read_at_all_is_moved_aside_too() {
    let dir = std::env::temp_dir().join(format!("canoa-control-dir-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("ledger.json")).expect("a directory where the file goes");
    let control = Control::open(Some(&dir), 10, 1024, Vec::new()).expect("opens empty");
    assert!(
        put(&control, &Caller::Owner, "after")
            .get("result")
            .is_some()
    );
    let aside = fs::read_dir(&dir)
        .expect("listed")
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains("json.bad-"));
    assert!(aside);
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn the_ledger_and_its_folder_are_made_private_whenever_the_host_opens_them() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = std::env::temp_dir().join(format!("canoa-control-mode-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("a folder");
    fs::write(dir.join("ledger.json"), r#"{"records":{}}"#).expect("a ledger");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("open");
    fs::set_permissions(dir.join("ledger.json"), fs::Permissions::from_mode(0o644)).expect("open");
    let _control = Control::open(Some(&dir), 10, 1024, Vec::new()).expect("opens");
    let mode = |path: &Path| fs::metadata(path).expect("there").permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join("ledger.json")), 0o600);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_recorded_session_is_the_owners_alone_whatever_a_key_is_granted() {
    let control = memory();
    let everything = peer(
        "k",
        &[
            Grant::LedgerRead,
            Grant::LedgerWrite,
            Grant::Worktree,
            Grant::StoreRead,
        ],
    );
    for method in ["history/export", "history/move", "history/portable"] {
        let answer = ask(
            &control,
            &everything,
            method,
            json!({"entry": "Claude", "session": "s", "to": "Codex"}),
        );
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
    }
}

#[test]
fn a_host_that_keeps_no_histories_refuses_the_owner_with_the_reason() {
    let control = memory();
    let answer = ask(
        &control,
        &Caller::Owner,
        "history/export",
        json!({"entry": "Claude", "session": "s"}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    assert!(
        answer["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("no session histories"))
    );
}

#[test]
fn no_grant_names_history() {
    assert!(Grant::named("history.read").is_none() && Grant::named("history.move").is_none());
}

#[test]
fn a_key_manages_accounts_only_with_the_accounts_grant() {
    let control = memory();
    let key = peer(
        "sha256:abc",
        &[
            Grant::LedgerRead,
            Grant::LedgerWrite,
            Grant::Worktree,
            Grant::StoreRead,
            Grant::StorePrograms,
        ],
    );
    let granted = peer("sha256:def", &[Grant::Accounts]);
    for (method, params) in [
        ("accounts/providers", json!({})),
        ("accounts/catalog", json!({"provider": "claude_accounts"})),
        ("accounts/list", json!({})),
        ("accounts/remove", json!({"name": "work"})),
        ("accounts/entries", json!({})),
        (
            "accounts/offer",
            json!({"entry": "Claude", "accounts": ["work"]}),
        ),
    ] {
        let answer = ask(&control, &key, method, params.clone());
        assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
        // Past the gate: the reason the owner of a host with no canoa.cfg gets.
        let answer = ask(&control, &granted, method, params);
        assert_eq!(answer["error"]["code"], REFUSED, "{method}: {answer}");
    }
    let owner = ask(&control, &Caller::Owner, "accounts/list", json!({}));
    assert_eq!(owner["error"]["code"], REFUSED, "{owner}");
}

#[test]
fn a_key_on_a_connection_is_refused_a_sign_in_without_the_accounts_grant() {
    let control = memory();
    let sign_ins = [
        (
            "accounts/add",
            json!({"name": "x", "harness": "claude", "method": "api-key"}),
        ),
        ("accounts/login", json!({"name": "x"})),
    ];
    let without = peer(
        "sha256:abc",
        &[Grant::LedgerRead, Grant::StoreRead, Grant::StorePrograms],
    );
    for answer in &over_a_link(&control, &without, &sign_ins) {
        assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    }
    let with = peer("sha256:def", &[Grant::Accounts]);
    for answer in &over_a_link(&control, &with, &sign_ins) {
        assert_ne!(answer["error"]["code"], DENIED, "{answer}");
    }
}

#[test]
fn usage_answers_a_key_holding_the_grant_and_the_owner_and_no_other() {
    let dir = std::env::temp_dir().join(format!("canoa-control-usage-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let log = Arc::new(crate::usage_log::UsageLog::open(dir.clone(), 400, 0));
    let mut spend = coqui_acp_api::Spend::new("s1");
    spend.input = 12;
    log.record(
        &crate::usage_log::Turn {
            account: "work",
            harness: "claude",
            session: "s1",
            parent: None,
            model: None,
        },
        &spend,
        crate::usage_splice::unix_now(),
    );
    let control = memory().with_usage(log);
    let granted = peer("sha256:aa", &[Grant::Usage]);
    let answer = ask(
        &control,
        &granted,
        "usage/summary",
        json!({"group_by": "account"}),
    );
    assert_eq!(answer["result"]["groups"][0]["key"], "work", "{answer}");
    assert_eq!(answer["result"]["groups"][0]["input"], 12);
    let answer = ask(&control, &granted, "usage/limits", json!({}));
    assert_eq!(answer["result"]["accounts"], json!([]), "{answer}");
    let owner = ask(&control, &Caller::Owner, "usage/summary", json!({}));
    assert_eq!(owner["result"]["total"]["input"], 12);
    for caller in [
        peer("sha256:bb", &[]),
        peer(
            "sha256:cc",
            &[Grant::Accounts, Grant::StoreRead, Grant::LedgerRead],
        ),
    ] {
        for method in ["usage/summary", "usage/limits"] {
            let answer = ask(&control, &caller, method, json!({}));
            assert_eq!(answer["error"]["code"], DENIED, "{method}: {answer}");
        }
    }
    let answer = ask(
        &control,
        &granted,
        "usage/summary",
        json!({"group_by": "nonsense"}),
    );
    assert_eq!(answer["error"]["code"], BAD_PARAMS, "{answer}");
    assert_eq!(Grant::named("usage"), Some(Grant::Usage));
    let told = ask(&control, &granted, "host/grants", json!({}));
    assert_eq!(told["result"]["grants"], json!(["usage"]));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_host_with_no_home_keeps_no_usage_and_says_so() {
    let control = memory();
    let answer = ask(&control, &Caller::Owner, "usage/summary", json!({}));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
}

#[test]
fn usage_read_needs_the_usage_grant_and_a_host_that_manages_accounts() {
    let control = memory();
    for caller in [
        peer("sha256:aa", &[]),
        peer("sha256:bb", &[Grant::Accounts, Grant::StoreRead]),
    ] {
        let answer = ask(&control, &caller, "usage/read", json!({"account": "work"}));
        assert_eq!(answer["error"]["code"], DENIED, "{answer}");
    }
    let granted = peer("sha256:cc", &[Grant::Usage]);
    let answer = ask(&control, &granted, "usage/read", json!({"account": "work"}));
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
    let answer = ask(
        &control,
        &granted,
        "options/choices",
        json!({"entry": "Claude"}),
    );
    assert_eq!(answer["error"]["code"], REFUSED, "{answer}");
}

#[test]
fn dispatch_table_grants_and_docs_stay_in_step() {
    let mut seen_methods = BTreeSet::new();
    for route in ROUTES {
        assert!(
            seen_methods.insert(route.method),
            "duplicate control route method: {}",
            route.method
        );
    }

    let mut seen_grants = BTreeSet::new();
    let mut seen_names = BTreeSet::new();
    for spec in GRANT_SPECS {
        assert!(
            seen_grants.insert(spec.grant),
            "duplicate grant: {:?}",
            spec.grant
        );
        assert!(
            seen_names.insert(spec.name),
            "duplicate grant name: {}",
            spec.name
        );
        assert_eq!(spec.grant.as_str(), spec.name);
        assert_eq!(Grant::named(spec.name), Some(spec.grant));
        let in_routes = ROUTES.iter().any(|route| route.access.mentions(spec.grant));
        assert_eq!(
            in_routes, spec.control,
            "grant {:?} ({}) control flag {} disagrees with ROUTES presence {}",
            spec.grant, spec.name, spec.control, in_routes
        );
    }
    assert_eq!(Grant::all().len(), GRANT_SPECS.len());

    let docs = include_str!("AGENTS.md");
    for spec in GRANT_SPECS {
        assert!(
            docs.contains(spec.name),
            "crates/canoa-host/src/control/AGENTS.md is missing grant {}",
            spec.name
        );
    }
    for route in ROUTES {
        assert!(
            docs.contains(route.method),
            "crates/canoa-host/AGENTS.md is missing control method {}",
            route.method
        );
    }
}

#[test]
fn conditional_and_link_only_routes_check_their_grants() {
    let control = memory();
    let none = peer("sha256:aa", &[]);
    let reader = peer("sha256:bb", &[Grant::ConfigRead]);
    let hosting = peer("sha256:cc", &[Grant::Hosting]);

    // `config/get` on `[host]` requires `hosting`, not `config.read`.
    assert_eq!(
        ask(&control, &reader, "config/get", json!({"section": "host"}))["error"]["code"],
        DENIED
    );
    assert_ne!(
        ask(&control, &hosting, "config/get", json!({"section": "host"}))["error"]["code"],
        DENIED
    );

    // Owner-only network and history routes reject peers and leave owners to run the handler.
    for method in ["network/fetch", "history/portable"] {
        assert_eq!(
            ask(&control, &hosting, method, json!({}))["error"]["code"],
            DENIED
        );
    }

    // Link-only methods enforce their grant before telling the caller a link is needed.
    for method in [
        "store/apply",
        "accounts/add",
        "accounts/login",
        "host/pair",
        "secrets/set",
        "terminal/open",
    ] {
        assert_eq!(
            ask(&control, &none, method, json!({}))["error"]["code"],
            DENIED
        );
        assert_eq!(
            ask(&control, &Caller::Owner, method, json!({}))["error"]["code"],
            REFUSED
        );
    }

    assert_eq!(
        ask(&control, &none, "no/such/method", json!({}))["error"]["code"],
        NO_METHOD
    );
}

#[test]
fn usage_limits_fills_canonical_provider_for_local_and_legacy_accounts() {
    let dir = std::env::temp_dir().join(format!("canoa-usage-limits-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let log = Arc::new(crate::usage_log::UsageLog::open(dir.clone(), 400, 0));
    let mut report = coqui_acp_api::UsageReport::new();
    report.observed_at = 1_776_000_000;
    report.buckets = vec![coqui_acp_api::Bucket::new("five_hour")];
    log.note_report("local:codex", None, &report, 1_776_000_000);
    log.note_report("codex:local", None, &report, 1_776_000_000);
    log.note_report("claude", None, &report, 1_776_000_000);
    log.note_report("id-other", None, &report, 1_776_000_000);
    let control = memory().with_usage(log);
    let answer = ask(
        &control,
        &peer("sha256:aa", &[Grant::Usage]),
        "usage/limits",
        json!({}),
    );
    let accounts = answer["result"]["accounts"].as_array().expect("accounts");
    let provider_of = |name: &str| {
        accounts
            .iter()
            .find(|row| row["account"] == name)
            .and_then(|row| row.get("provider"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    assert_eq!(provider_of("local:codex").as_deref(), Some("codex"));
    assert_eq!(provider_of("codex:local").as_deref(), Some("codex"));
    assert_eq!(provider_of("claude").as_deref(), Some("claude"));
    assert_eq!(provider_of("id-other"), None);
    let _ = fs::remove_dir_all(&dir);
}
