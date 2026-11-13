#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use super::*;

fn forwarded(scope: &Scope, message: &Value) -> Value {
    match scope.check(&serde_json::to_vec(message).expect("JSON")) {
        Verdict::Forward(bytes) => serde_json::from_slice(&bytes).expect("JSON"),
        Verdict::Refuse(answer) => panic!("refused: {answer:?}"),
    }
}

fn refused(scope: &Scope, message: &[u8]) -> Option<Value> {
    match scope.check(message) {
        Verdict::Refuse(answer) => {
            answer.map(|bytes| serde_json::from_slice(&bytes).expect("JSON"))
        }
        Verdict::Forward(bytes) => panic!("forwarded: {}", String::from_utf8_lossy(&bytes)),
    }
}

fn new_session(servers: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "session/new", "params": {"cwd": "/work", "mcpServers": servers}})
}

#[test]
fn a_stdio_mcp_server_never_reaches_the_agent() {
    let scope = Scope::default();
    for server in [
        json!({"name": "x", "command": "/bin/sh", "args": ["-c", "id"], "env": []}),
        json!({"type": "stdio", "name": "x", "command": "calc.exe", "args": [], "env": []}),
        json!({"type": "http", "name": "x", "url": "https://a.example", "headers": [], "command": "sh"}),
        json!({"name": "x"}),
        json!({"type": "unknown", "name": "x"}),
        json!("sh"),
    ] {
        let message = serde_json::to_vec(&new_session(json!([server]))).expect("JSON");
        let answer = refused(&scope, &message).expect("a request is answered");
        assert_eq!(answer["id"], 7);
        assert_eq!(answer["error"]["code"], INVALID_PARAMS);
    }
}

#[test]
fn a_network_mcp_server_passes_only_over_https() {
    let scope = Scope::default();
    let https =
        json!([{"type": "http", "name": "x", "url": "https://a.example/mcp", "headers": []}]);
    forwarded(&scope, &new_session(https));
    forwarded(
        &scope,
        &new_session(
            json!([{"type": "sse", "name": "x", "url": "HTTPS://a.example", "headers": []}]),
        ),
    );
    forwarded(
        &scope,
        &new_session(json!([{"type": "acp", "name": "x", "id": "browser"}])),
    );
    let plain = serde_json::to_vec(&new_session(
        json!([{"type": "http", "name": "x", "url": "http://10.0.0.1", "headers": []}]),
    ))
    .expect("JSON");
    assert!(refused(&scope, &plain).is_some());
    let multibyte = serde_json::to_vec(&new_session(
        json!([{"type": "http", "name": "x", "url": "httpé://x", "headers": []}]),
    ))
    .expect("JSON");
    assert!(refused(&scope, &multibyte).is_some());
}

#[test]
fn mcp_servers_are_checked_on_every_method() {
    let scope = Scope::default();
    for method in [
        "session/load",
        "session/resume",
        "session/fork",
        "_vendor/anything",
    ] {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": {"sessionId": "s", "mcpServers": [{"name": "x", "command": "sh"}]}});
        assert!(
            refused(&scope, &serde_json::to_vec(&message).expect("JSON")).is_some(),
            "{method}"
        );
    }
    let notification = json!({"jsonrpc": "2.0", "method": "session/new", "params": {"mcpServers": [{"name": "x", "command": "sh"}]}});
    assert_eq!(
        refused(&scope, &serde_json::to_vec(&notification).expect("JSON")),
        None
    );
}

#[test]
fn positional_params_are_refused_since_a_struct_fills_its_fields_from_them() {
    let scope = Scope::new(vec![std::env::temp_dir().join("canoa-roots")]);
    let message = json!({"jsonrpc": "2.0", "id": 1, "method": "session/new", "params": [
        "/elsewhere", ["/elsewhere"], [{"name": "x", "command": "calc.exe", "args": [], "env": []}], {"claudeCode": {}},
    ]});
    let answer = refused(&scope, &serde_json::to_vec(&message).expect("JSON")).expect("answered");
    assert_eq!(answer["error"]["code"], INVALID_PARAMS);
    for params in [json!("text"), json!(7), json!(null), json!(true)] {
        let message = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params});
        assert!(
            refused(
                &Scope::default(),
                &serde_json::to_vec(&message).expect("JSON")
            )
            .is_some(),
            "{params}"
        );
    }
    let notification = json!({"jsonrpc": "2.0", "method": "session/cancel", "params": ["s"]});
    assert_eq!(
        refused(
            &Scope::default(),
            &serde_json::to_vec(&notification).expect("JSON")
        ),
        None
    );
}

#[test]
fn meta_is_removed_at_every_depth() {
    let scope = Scope::default();
    let message = json!({
        "jsonrpc": "2.0", "id": 1, "method": "session/new", "_meta": {"a": 1},
        "params": {"cwd": "/work", "mcpServers": [], "_meta": {"claudeCode": {"options": {"executable": "/tmp/x"}}},
                   "nested": [{"_meta": 2, "keep": true}]},
    });
    let passed = forwarded(&scope, &message);
    assert_eq!(
        passed,
        json!({"jsonrpc": "2.0", "id": 1, "method": "session/new",
               "params": {"cwd": "/work", "mcpServers": [], "nested": [{"keep": true}]}})
    );
}

#[test]
fn what_passes_is_the_hosts_own_serialization() {
    let scope = Scope::default();
    let duplicate = br#"{"jsonrpc":"2.0","id":1,"method":"session/new","params":{"mcpServers":[{"name":"x","command":"sh"}],"mcpServers":[]}}"#;
    let Verdict::Forward(bytes) = scope.check(duplicate) else {
        panic!("the last key is the one checked, and the one sent");
    };
    let sent: Value = serde_json::from_slice(&bytes).expect("JSON");
    assert_eq!(sent["params"]["mcpServers"], json!([]));
    assert_eq!(
        bytes
            .windows(b"mcpServers".len())
            .filter(|w| *w == b"mcpServers")
            .count(),
        1
    );
}

#[test]
fn what_is_not_one_json_object_is_refused() {
    let scope = Scope::default();
    let answer = refused(&scope, b"not json").expect("answered");
    assert_eq!(answer["error"]["code"], PARSE_ERROR);
    for message in [
        &b"[]"[..],
        b"[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}]",
        b"7",
    ] {
        let answer = refused(&scope, message).expect("answered");
        assert_eq!(answer["error"]["code"], INVALID_REQUEST);
    }
}

#[test]
fn ordinary_messages_pass_unchanged() {
    let scope = Scope::default();
    for message in [
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": 1}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "session/prompt", "params": {"sessionId": "s", "prompt": [{"type": "text", "text": "hi\nthere"}]}}),
        json!({"jsonrpc": "2.0", "id": 9, "result": {"outcome": {"outcome": "selected", "optionId": "allow"}}}),
        json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": "s"}}),
    ] {
        assert_eq!(forwarded(&scope, &message), message);
    }
}

#[test]
fn a_working_directory_outside_the_roots_is_refused() {
    let root = std::env::temp_dir().join("canoa-roots");
    let scope = Scope::new(vec![root.clone()]);
    let inside = root.join("project");
    let message = |cwd: &str| {
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": cwd, "mcpServers": []}}))
            .expect("JSON")
    };
    assert!(matches!(
        scope.check(&message(&inside.to_string_lossy())),
        Verdict::Forward(_)
    ));
    let outside = std::env::temp_dir().join("elsewhere");
    let climbing = root.join("..").join("elsewhere");
    let sibling = format!("{}-sibling", root.to_string_lossy());
    let dotted = format!("{}/./project", root.to_string_lossy());
    for cwd in [
        outside.to_string_lossy().as_ref(),
        climbing.to_string_lossy().as_ref(),
        &sibling,
        &dotted,
        "relative",
    ] {
        assert!(refused(&scope, &message(cwd)).is_some(), "{cwd}");
    }
    let more = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {"cwd": inside, "additionalDirectories": [outside], "mcpServers": []}});
    assert!(refused(&scope, &serde_json::to_vec(&more).expect("JSON")).is_some());
}

#[test]
fn any_bytes_give_a_verdict() {
    let scope = Scope::new(vec![std::env::temp_dir()]);
    const ALPHABET: &[u8] = b"{}[]\":,_metacwdmcpServers 0123\\u";
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..5000 {
        let bytes: Vec<u8> = (0..48)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                ALPHABET[(state % ALPHABET.len() as u64) as usize]
            })
            .collect();
        let _ = scope.check(&bytes);
    }
    let deep = "[".repeat(10_000);
    let _ = scope.check(deep.as_bytes());
}

fn initialize(version: u64, key: &str, meta: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": version, key: {"fs": {}, "_meta": meta}}})
}

#[test]
fn the_usage_opt_in_reaches_the_agent_with_the_hosts_account_and_not_the_clients() {
    let scope = Scope::default().account("work".to_owned());
    let asked =
        json!({USAGE_CAPABILITY: {"notify": true, "account": "mine"}, "claudeCode": {"x": 1}});
    let sent = forwarded(&scope, &initialize(1, "clientCapabilities", asked.clone()));
    assert_eq!(
        sent["params"]["clientCapabilities"]["_meta"],
        json!({USAGE_CAPABILITY: {"notify": true, "account": "work"}})
    );
    // Version 2 names the client's capabilities as `capabilities`.
    let sent = forwarded(&scope, &initialize(2, "capabilities", asked));
    assert_eq!(
        sent["params"]["capabilities"]["_meta"],
        json!({USAGE_CAPABILITY: {"notify": true, "account": "work"}})
    );
    // No notice is sent to a client that did not ask, and one that sent no capabilities at all is still given the account.
    let silent = forwarded(
        &scope,
        &initialize(1, "clientCapabilities", json!({"other": 1})),
    );
    assert_eq!(
        silent["params"]["clientCapabilities"]["_meta"],
        json!({USAGE_CAPABILITY: {"notify": false, "account": "work"}})
    );
    let bare = forwarded(
        &scope,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}),
    );
    assert_eq!(
        bare["params"]["clientCapabilities"]["_meta"][USAGE_CAPABILITY]["account"],
        "work"
    );
}

#[test]
fn a_scope_without_an_account_passes_only_the_clients_wish_for_notices() {
    let scope = Scope::default();
    let sent = forwarded(
        &scope,
        &initialize(
            1,
            "clientCapabilities",
            json!({USAGE_CAPABILITY: {"notify": true, "account": "mine"}}),
        ),
    );
    assert_eq!(
        sent["params"]["clientCapabilities"]["_meta"],
        json!({USAGE_CAPABILITY: {"notify": true}})
    );
    let none = forwarded(
        &scope,
        &initialize(1, "clientCapabilities", json!({"other": 1})),
    );
    assert!(
        none["params"]["clientCapabilities"].get("_meta").is_none(),
        "{none}"
    );
}

#[test]
fn the_usage_opt_in_is_read_only_from_initialize() {
    let scope = Scope::default().account("work".to_owned());
    let moved = json!({"jsonrpc": "2.0", "id": 2, "method": "session/new", "params": {
        "cwd": "/work", "mcpServers": [], "clientCapabilities": {"_meta": {USAGE_CAPABILITY: {"notify": true}}}}});
    let sent = forwarded(&scope, &moved);
    assert!(
        sent["params"]["clientCapabilities"].get("_meta").is_none(),
        "{sent}"
    );
}

#[test]
fn a_usage_report_leaves_under_the_hosts_account() {
    let scope = Scope::default().account("work".to_owned());
    let report = json!({"supported": true, "account": "the-agents", "revision": 1, "observed_at": 0, "buckets": []});
    let notice =
        json!({"jsonrpc": "2.0", "method": USAGE_NOTIFICATION, "params": {"report": report}});
    let answer = json!({"jsonrpc": "2.0", "id": 4, "result": report});
    for (message, path) in [
        (notice, "/params/report/account"),
        (answer, "/result/account"),
    ] {
        let bytes = serde_json::to_vec(&message).expect("JSON");
        let stamped = scope.stamp(&bytes);
        let stamped: Value = serde_json::from_slice(&stamped).expect("JSON");
        assert_eq!(stamped.pointer(path), Some(&json!("work")), "{stamped}");
    }
    // A report that names none is given the account too.
    let bare = json!({"jsonrpc": "2.0", "id": 5, "result": {"supported": false, "buckets": []}});
    let bytes = serde_json::to_vec(&bare).expect("JSON");
    let stamped = scope.stamp(&bytes);
    assert_eq!(
        serde_json::from_slice::<Value>(&stamped).expect("JSON")["result"]["account"],
        "work"
    );
}

#[test]
fn what_is_not_a_usage_report_passes_untouched() {
    let scope = Scope::default().account("work".to_owned());
    for message in [
        json!({"jsonrpc": "2.0", "id": 1, "result": {"supported": "yes", "buckets": []}}),
        json!({"jsonrpc": "2.0", "id": 1, "result": {"sessionId": "s", "note": "supported"}}),
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"text": "supported buckets"}}),
        json!({"jsonrpc": "2.0", "method": "x", "params": {"supported": true, "buckets": []}}),
    ] {
        let bytes = serde_json::to_vec(&message).expect("JSON");
        assert_eq!(scope.stamp(&bytes).as_ref(), bytes.as_slice(), "{message}");
    }
    let report = json!({"jsonrpc": "2.0", "id": 1, "result": {"supported": true, "buckets": []}});
    let bytes = serde_json::to_vec(&report).expect("JSON");
    assert_eq!(Scope::default().stamp(&bytes).as_ref(), bytes.as_slice());
}

#[test]
fn stamping_changes_the_account_of_every_vector_report_and_nothing_else() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../tests/vectors/usage.json")).expect("JSON");
    let scope = Scope::default().account("work".to_owned());
    let reports = vectors["roundtrip"].as_array().expect("a list");
    assert!(reports.len() >= 7);
    for vector in reports {
        let report = vector["report"].clone();
        let notice =
            json!({"jsonrpc": "2.0", "method": USAGE_NOTIFICATION, "params": {"report": report}});
        let bytes = serde_json::to_vec(&notice).expect("JSON");
        let stamped: Value = serde_json::from_slice(&scope.stamp(&bytes)).expect("JSON");
        let mut expected = report;
        expected["account"] = json!("work");
        assert_eq!(stamped["params"]["report"], expected, "{}", vector["name"]);
    }
}

#[test]
fn an_agents_request_in_the_hosts_own_namespace_is_never_passed_on() {
    for dropped in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "_canoa/profile/approve", "params": {}}),
        json!({"jsonrpc": "2.0", "method": "_canoa/anything"}),
        // The secret extension's methods are the host's to send: an agent forging a show would put a value on the card.
        json!({"jsonrpc": "2.0", "id": 1, "method": "_coqui_acp/secret_show_open", "params": {}}),
        json!({"jsonrpc": "2.0", "method": "_coqui_acp/secret_show_deliver", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "_coqui_acp/secret_request", "params": {}}),
    ] {
        assert!(is_host_only(dropped.to_string().as_bytes()), "{dropped}");
    }
    // JSON may spell the same method with escapes, which the bytes do not show.
    for escaped in [
        r#"{"jsonrpc":"2.0","id":1,"method":"_coqui_acp\/secret_request","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"_coqui_acp/secret_show_open"}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"_canoa\/profile\/approve","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"_canoa/anything"}"#,
    ] {
        assert!(is_host_only(escaped.as_bytes()), "{escaped}");
    }
    // A line this reader cannot read is dropped, since a more lenient reader in the client might take it for a request of the host's.
    let deep = format!(
        r#"{{"jsonrpc":"2.0","method":"_canoa/delegate/event","params":{}1{}}}"#,
        "[".repeat(200),
        "]".repeat(200)
    );
    for unreadable in [
        r#"{"jsonrpc":"2.0","method":"_canoa/delegate/event","params":{"pad":"\ud800"}}"#
            .to_owned(),
        deep,
        r#"{"jsonrpc":"2.0","method":"_canoa/delegate/event","params":{"n":1e400}}"#.to_owned(),
        r#"{"jsonrpc":"2.0","id":1,"method":"_canoa/profile/approve","params":{"n":1e400}}"#
            .to_owned(),
        "not json at all".to_owned(),
    ] {
        assert_eq!(
            dropped(unreadable.as_bytes()),
            Some(Dropped::Unreadable),
            "{unreadable}"
        );
    }
    assert_eq!(
        dropped(br#"{"jsonrpc":"2.0","method":"_canoa/delegate/event"}"#),
        Some(Dropped::HostOnly)
    );
    // An escape that spells another method, or sits in text, is not one.
    for passed in [
        r#"{"jsonrpc":"2.0","id":1,"method":"session\/update"}"#,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"text":"_canoa/x"}}"#,
    ] {
        assert!(!is_host_only(passed.as_bytes()), "{passed}");
    }
    for passed in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "session/request_permission"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "_coqui_acp/usage"}),
        // Text that merely mentions it, and an answer, are not requests of that name.
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"text": "_canoa/profile/approve"}}),
        json!({"jsonrpc": "2.0", "id": 1, "result": {"method": "_canoa/x"}}),
    ] {
        assert!(!is_host_only(passed.to_string().as_bytes()), "{passed}");
    }
}

#[test]
fn what_the_host_itself_sends_in_its_namespace_passes_and_only_while_it_is_sending() {
    let event =
        json!({"jsonrpc": "2.0", "method": "_canoa/delegate/event", "params": {}}).to_string();
    assert!(is_host_only(event.as_bytes()));
    assert!(!super::host_own(|| is_host_only(event.as_bytes())));
    assert!(
        is_host_only(event.as_bytes()),
        "the exemption ends with the send"
    );
    let unwound = std::panic::catch_unwind(|| super::host_own(|| panic!("a send that panics")));
    assert!(unwound.is_err());
    assert!(is_host_only(event.as_bytes()), "nor does a panic keep it");
}
