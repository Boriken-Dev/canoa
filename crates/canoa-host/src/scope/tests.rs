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
    for cwd in [
        outside.to_string_lossy().as_ref(),
        climbing.to_string_lossy().as_ref(),
        &sibling,
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
