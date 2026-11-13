//! The connection layer's bookkeeping, with no process: what a session is offered, what a bearer opens, how the parent's mode is
//! followed, and the settings.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{BridgeCommand, ConnInfo, Engine, Host, META, Principal, SERVER_NAME, Source, child};
use crate::config::{Config, DelegateConfig};
use crate::local::Link;
use crate::serve::LocalError;

struct Fake;

impl Host for Fake {
    fn sources(&self) -> Vec<Source> {
        vec![Source {
            name: "a".to_owned(),
            title: None,
            harness: None,
        }]
    }

    fn may_use(&self, _who: &Principal, _source: &str) -> bool {
        true
    }

    fn kind(&self, _source: &str) -> Option<String> {
        Some("fake".to_owned())
    }

    fn open(&self, _source: &str, _context: super::ChildContext) -> Result<Link, LocalError> {
        Err(LocalError::NoEntry)
    }
}

fn engine(config: DelegateConfig) -> Arc<Engine> {
    let engine = Engine::new(config);
    let _ = engine.host.set(Arc::new(Fake));
    let _ = engine.url.set("http://127.0.0.1:1/mcp".to_owned());
    let _ = engine.bridge.set(BridgeCommand {
        program: "/opt/canoa/canoa".to_owned(),
        args: vec!["mcp-bridge".to_owned()],
    });
    engine
}

fn info(child: bool, depth: usize) -> ConnInfo {
    ConnInfo {
        principal: Principal::Owner,
        entry: "e".to_owned(),
        roots: Vec::new(),
        depth,
        child,
    }
}

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("JSON")
}

fn initialize(meta: bool) -> Vec<u8> {
    let mut params = json!({"protocolVersion": 1});
    if meta {
        params["_meta"] = json!({ META: true });
    }
    bytes(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": params}))
}

fn initialized(http: bool) -> Vec<u8> {
    bytes(
        &json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 1,
        "agentCapabilities": {"mcpCapabilities": {"http": http}}}}),
    )
}

/// An agent's answer to `initialize` in protocol version 2, which says what a session takes under `capabilities`.
fn initialized_v2(http: bool, stdio: bool) -> Vec<u8> {
    bytes(
        &json!({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": 2,
        "capabilities": {"session": {"mcp": {"http": http, "stdio": stdio}}}}}),
    )
}

fn new_session(id: u64) -> Vec<u8> {
    bytes(
        &json!({"jsonrpc": "2.0", "id": id, "method": "session/new", "params": {
        "cwd": "/work", "mcpServers": [{"type": "http", "name": SERVER_NAME, "url": "https://example.com/mcp", "headers": []}]}}),
    )
}

fn opened(id: u64, session: &str, mode: &str) -> Vec<u8> {
    bytes(
        &json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": session, "configOptions": [
        {"id": "mode", "category": "mode", "currentValue": mode, "options": [{"value": "manual"}, {"value": "bypass"}]}]}}),
    )
}

/// What `session/new` came out as, after the layer: the host's entries in `mcpServers`.
fn servers(message: &[u8]) -> Vec<Value> {
    serde_json::from_slice::<Value>(message).expect("JSON")["params"]["mcpServers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn token_of(entry: &Value) -> String {
    entry["headers"][0]["value"]
        .as_str()
        .and_then(|value| value.strip_prefix("Bearer "))
        .expect("a bearer")
        .to_owned()
}

fn opt_in(conn: &Arc<super::Conn>, meta: bool, http: bool) {
    conn.peek(&initialize(meta));
    assert!(conn.client_said(initialize(meta)).is_some());
    conn.agent_said(&initialized(http));
}

#[test]
fn a_session_is_given_the_entry_only_when_the_client_asked_and_over_http_when_the_agent_takes_it() {
    for (meta, http, offered) in [
        (true, true, Some("http")),
        (false, true, None),
        (true, false, Some("stdio")),
        (false, false, None),
    ] {
        let conn = engine(DelegateConfig::default()).connect(info(false, 0));
        opt_in(&conn, meta, http);
        let sent = conn.client_said(new_session(2)).expect("it passes");
        let listed = servers(&sent);
        match offered {
            Some("http") => {
                assert_eq!(
                    listed.len(),
                    1,
                    "the client's entry of that name is replaced"
                );
                assert_eq!(listed[0]["name"], SERVER_NAME);
                assert_eq!(listed[0]["type"], "http");
                assert_eq!(listed[0]["url"], "http://127.0.0.1:1/mcp");
                assert_eq!(token_of(&listed[0]).len(), 64);
            }
            Some(_) => {
                assert_eq!(listed.len(), 1);
                assert_eq!(listed[0]["name"], SERVER_NAME);
                // Version 1 writes a stdio server with no type; the token is in the environment, not the arguments.
                assert!(listed[0].get("type").is_none(), "{}", listed[0]);
                assert_eq!(listed[0]["command"], "/opt/canoa/canoa");
                assert_eq!(listed[0]["args"], json!(["mcp-bridge"]));
                assert_eq!(listed[0]["env"][0]["name"], "CANOA_DELEGATE_URL");
                assert_eq!(listed[0]["env"][0]["value"], "http://127.0.0.1:1/mcp");
                assert_eq!(listed[0]["env"][1]["name"], "CANOA_DELEGATE_TOKEN");
                assert_eq!(
                    listed[0]["env"][1]["value"].as_str().map(str::len),
                    Some(64)
                );
                assert!(
                    !listed[0]["args"]
                        .to_string()
                        .contains(listed[0]["env"][1]["value"].as_str().unwrap_or("-"))
                );
            }
            None => {
                // Untouched: the client's own entry, which the scope has the say on.
                assert_eq!(listed[0]["url"], "https://example.com/mcp", "{meta} {http}");
            }
        }
    }
}

#[test]
fn a_version_two_agent_is_given_a_typed_stdio_entry_when_it_says_it_takes_stdio() {
    let take = |http: bool, stdio: bool| {
        let conn = engine(DelegateConfig::default()).connect(info(false, 0));
        conn.peek(&initialize(true));
        assert!(conn.client_said(initialize(true)).is_some());
        conn.agent_said(&initialized_v2(http, stdio));
        servers(&conn.client_said(new_session(2)).expect("it passes"))
    };
    let both = take(true, true);
    assert_eq!(
        both[0]["type"], "http",
        "HTTP is preferred when the agent takes it"
    );
    let stdio = take(false, true);
    assert_eq!(stdio[0]["type"], "stdio");
    assert_eq!(stdio[0]["command"], "/opt/canoa/canoa");
    assert_eq!(stdio[0]["env"][1]["name"], "CANOA_DELEGATE_TOKEN");
    let neither = take(false, false);
    assert_eq!(neither[0]["url"], "https://example.com/mcp");
}

#[test]
fn an_agent_that_takes_no_http_is_given_no_entry_when_there_is_nothing_to_run_the_bridge() {
    let bare = Engine::new(DelegateConfig::default());
    let _ = bare.host.set(Arc::new(Fake));
    let _ = bare.url.set("http://127.0.0.1:1/mcp".to_owned());
    let conn = bare.connect(info(false, 0));
    opt_in(&conn, true, false);
    assert_eq!(
        servers(&conn.client_said(new_session(2)).expect("it passes"))[0]["url"],
        "https://example.com/mcp"
    );
}

#[test]
fn nothing_is_offered_past_max_depth_or_when_the_host_turns_it_off() {
    let config = DelegateConfig {
        max_depth: 1,
        ..DelegateConfig::default()
    };
    let at_depth = engine(config.clone()).connect(info(true, 1));
    at_depth.agent_said(&initialize(false)); // not an answer; changes nothing
    opt_in(&at_depth, false, true);
    assert_eq!(
        servers(&at_depth.client_said(new_session(2)).expect("passes"))[0]["url"],
        "https://example.com/mcp"
    );

    let task = engine(DelegateConfig {
        max_depth: 2,
        ..config
    })
    .connect(info(true, 1));
    // A task's connection asks for nothing: the host decides, and it may.
    task.agent_said(&initialized(true));
    let sent = task.client_said(initialize(false)).expect("passes");
    assert!(!sent.is_empty());
    task.agent_said(&initialized(true));
    assert_eq!(
        servers(&task.client_said(new_session(3)).expect("passes"))[0]["name"],
        SERVER_NAME
    );

    let off = engine(DelegateConfig {
        enabled: false,
        ..DelegateConfig::default()
    })
    .connect(info(false, 0));
    opt_in(&off, true, true);
    assert_eq!(
        servers(&off.client_said(new_session(2)).expect("passes"))[0]["url"],
        "https://example.com/mcp"
    );
}

#[test]
fn a_bearer_opens_its_session_once_the_agent_names_it_and_stops_with_the_connection() {
    let engine = engine(DelegateConfig::default());
    let conn = engine.connect(info(false, 0));
    opt_in(&conn, true, true);
    let first = token_of(&servers(&conn.client_said(new_session(2)).expect("passes"))[0]);
    let second = token_of(&servers(&conn.client_said(new_session(3)).expect("passes"))[0]);
    assert_ne!(first, second);
    // Valid from the moment it was minted, but it can delegate once the session is named.
    let early = engine.session_of_token(&first).expect("known");
    assert!(early.peer().is_none());
    conn.agent_said(&opened(2, "s1", "manual"));
    conn.agent_said(&opened(3, "s2", "manual"));
    let one = engine.session_of_token(&first).expect("known");
    let two = engine.session_of_token(&second).expect("known");
    assert!(!Arc::ptr_eq(&one, &two));
    assert_eq!(
        one.peer().as_deref(),
        Some(&*format!("local:{}:s1", conn.id))
    );
    assert!(
        engine
            .session_of_peer(&one.peer().expect("named"))
            .is_some()
    );
    assert!(engine.session_of_token("nothing").is_none());
    assert!(engine.session_of_token(&format!("{first}0")).is_none());
    conn.close();
    assert!(engine.session_of_token(&first).is_none());
    assert!(engine.session_of_token(&second).is_none());
    assert!(
        engine
            .session_of_peer(&one.peer().expect("named"))
            .is_none()
    );
    // After the end nothing is offered.
    let late = conn.client_said(new_session(4)).expect("passes");
    assert_eq!(servers(&late)[0]["url"], "https://example.com/mcp");
}

#[test]
fn a_session_the_agent_refuses_loses_its_bearer() {
    let engine = engine(DelegateConfig::default());
    let conn = engine.connect(info(false, 0));
    opt_in(&conn, true, true);
    let token = token_of(&servers(&conn.client_said(new_session(2)).expect("passes"))[0]);
    conn.agent_said(&bytes(
        &json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32603, "message": "no"}}),
    ));
    assert!(engine.session_of_token(&token).is_none());
}

#[test]
fn a_loaded_session_is_named_by_the_request() {
    let engine = engine(DelegateConfig::default());
    let conn = engine.connect(info(false, 0));
    opt_in(&conn, true, true);
    let load = bytes(
        &json!({"jsonrpc": "2.0", "id": 5, "method": "session/load", "params": {
        "sessionId": "old", "cwd": "/work", "mcpServers": []}}),
    );
    let sent = conn.client_said(load).expect("passes");
    let token = token_of(&servers(&sent)[0]);
    let session = engine.session_of_token(&token).expect("known");
    assert_eq!(session.session_id().as_deref(), Some("old"));
    conn.agent_said(&bytes(&json!({"jsonrpc": "2.0", "id": 5, "result": {}})));
    assert!(engine.session_of_token(&token).is_some());
}

#[test]
fn the_parents_mode_is_followed_from_what_the_host_relays() {
    let engine = engine(DelegateConfig::default());
    let conn = engine.connect(info(false, 0));
    opt_in(&conn, true, true);
    let token = token_of(&servers(&conn.client_said(new_session(2)).expect("passes"))[0]);
    conn.agent_said(&opened(2, "s1", "manual"));
    let session = engine.session_of_token(&token).expect("known");
    assert_eq!(session.mode().as_deref(), Some("manual"));
    // A request that fails changes nothing; one that succeeds does.
    let set = |id: u64, value: &str| {
        conn.client_said(bytes(
            &json!({"jsonrpc": "2.0", "id": id, "method": "session/set_config_option",
            "params": {"sessionId": "s1", "configId": "mode", "value": value}}),
        ))
    };
    set(10, "bypass");
    conn.agent_said(&bytes(
        &json!({"jsonrpc": "2.0", "id": 10, "error": {"code": -32602, "message": "no"}}),
    ));
    assert_eq!(session.mode().as_deref(), Some("manual"));
    set(11, "bypass");
    conn.agent_said(&bytes(&json!({"jsonrpc": "2.0", "id": 11, "result": {}})));
    assert_eq!(session.mode().as_deref(), Some("bypass"));
    // The agent's own notice of a change, and the answer's list of options.
    conn.agent_said(&bytes(&json!({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": "s1", "update": {"sessionUpdate": "current_mode_update", "currentModeId": "manual"}}})));
    assert_eq!(session.mode().as_deref(), Some("manual"));
    set(12, "bypass");
    conn.agent_said(&bytes(&json!({"jsonrpc": "2.0", "id": 12, "result": {"configOptions": [
        {"id": "mode", "category": "mode", "currentValue": "manual", "options": [{"value": "manual"}]}]}})));
    assert_eq!(
        session.mode().as_deref(),
        Some("manual"),
        "the agent's own list wins"
    );
    // Another session's name changes nothing here.
    conn.client_said(bytes(
        &json!({"jsonrpc": "2.0", "id": 13, "method": "session/set_mode",
        "params": {"sessionId": "other", "modeId": "bypass"}}),
    ));
    conn.agent_said(&bytes(&json!({"jsonrpc": "2.0", "id": 13, "result": {}})));
    assert_eq!(session.mode().as_deref(), Some("manual"));
}

#[test]
fn the_host_asks_the_person_on_the_connection_and_the_answer_is_not_the_agents() {
    let engine = engine(DelegateConfig::default());
    let conn = engine.connect(info(false, 0));
    let asking = Arc::clone(&conn);
    let asked = std::thread::spawn(move || {
        asking.ask(
            "s1",
            &json!({"taskId": "task-1", "title": "t", "source": "a"}),
            json!({"sessionId": "task's own", "toolCall": {"toolCallId": "c1"}, "options": []}),
            &|| false,
        )
    });
    let request = loop {
        if let Some(message) = conn.drain().into_iter().next() {
            break serde_json::from_slice::<Value>(&message).expect("JSON");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert_eq!(request["method"], "session/request_permission");
    assert_eq!(request["params"]["sessionId"], "s1");
    assert_eq!(request["params"]["toolCall"]["toolCallId"], "task-1:c1");
    assert_eq!(request["params"]["_meta"][META]["taskId"], "task-1");
    // The client's answer is the host's, and does not reach the agent.
    let answer = bytes(
        &json!({"jsonrpc": "2.0", "id": request["id"], "result": {"outcome": {"outcome": "selected", "optionId": "yes"}}}),
    );
    assert!(conn.client_said(answer).is_none());
    let result = asked.join().expect("no panic").expect("answered");
    assert_eq!(result["outcome"]["optionId"], "yes");
    // An answer to nothing the host asked passes as any message.
    let stray = bytes(&json!({"jsonrpc": "2.0", "id": "canoa-delegate-9-ab", "result": {}}));
    assert!(conn.client_said(stray).is_some());
}

#[test]
fn a_question_is_cancelled_when_the_connection_ends_or_the_task_is_stopped() {
    let engine = engine(DelegateConfig::default());
    let conn = engine.connect(info(false, 0));
    let asking = Arc::clone(&conn);
    let stopped = std::sync::atomic::AtomicBool::new(false);
    let stopped = Arc::new(stopped);
    let flag = Arc::clone(&stopped);
    let first = std::thread::spawn(move || {
        asking.ask("s", &json!({}), json!({"toolCall": {}}), &|| {
            flag.load(std::sync::atomic::Ordering::SeqCst)
        })
    });
    stopped.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(first.join().expect("no panic").is_none());
    let asking = Arc::clone(&conn);
    let second =
        std::thread::spawn(move || asking.ask("s", &json!({}), json!({"toolCall": {}}), &|| false));
    std::thread::sleep(std::time::Duration::from_millis(50));
    conn.close();
    assert!(second.join().expect("no panic").is_none());
}

#[test]
fn a_tasks_updates_are_kept_newest_last_within_history_bytes() {
    let child = child::Child::new("task-1".to_owned(), "t".to_owned(), "a".to_owned());
    for index in 0..10 {
        child.keep(format!("{index:04}"), 12);
    }
    assert_eq!(child.history(), ["0007", "0008", "0009"]);
    child.keep("0123456789abcdef".to_owned(), 12);
    assert!(
        child.history().is_empty(),
        "an update past the limit by itself is not kept"
    );
}

#[test]
fn the_settings_default_to_the_documented_values_and_refuse_what_they_do_not_know() {
    let none = Config::from_str_in("", std::path::Path::new(".")).expect("parses");
    assert_eq!(none.delegate, DelegateConfig::default());
    let text = "[host.delegate]\nenabled = false\nsources = [\"a\", \"b\"]\nmax_children = 2\nmax_running = 3\nmax_depth = 0\n\
                task_bytes = 10\nresult_bytes = 20\nhistory_bytes = 30\n";
    let set = Config::from_str_in(text, std::path::Path::new("."))
        .expect("parses")
        .delegate;
    assert_eq!(
        set,
        DelegateConfig {
            enabled: false,
            sources: Some(vec!["a".to_owned(), "b".to_owned()]),
            max_children: 2,
            max_running: 3,
            max_depth: 0,
            task_bytes: 10,
            result_bytes: 20,
            history_bytes: 30,
        }
    );
    let defaults = DelegateConfig::default();
    assert!(defaults.enabled && defaults.sources.is_none());
    assert_eq!(
        (
            defaults.max_children,
            defaults.max_running,
            defaults.max_depth
        ),
        (8, 16, 1)
    );
    assert!(
        Config::from_str_in(
            "[host.delegate]\nmax_childs = 2\n",
            std::path::Path::new(".")
        )
        .is_err()
    );
    assert!(
        Config::from_str_in(
            "[host.delegate]\nsources = [\"\"]\n",
            std::path::Path::new(".")
        )
        .is_err()
    );
    let narrowed =
        Config::from_str_in("[host.delegate]\nsources = []\n", std::path::Path::new("."))
            .expect("parses")
            .delegate;
    assert_eq!(
        narrowed.sources,
        Some(Vec::new()),
        "an empty list allows none"
    );
}

mod bridge;
