use std::sync::Arc;

use coqui_acp::secret::CAPABILITY;
use coqui_agentic_peer::auth::{Identity, Method};
use coqui_peer_webcrypto::TrustStore;
use serde_json::{Value, json};

use super::link::is_bypass;
use super::{Hub, Link};
use crate::delegate::BridgeCommand;

/// A directory of its own for a trust store, removed when dropped.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("canoa-secrets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A link of a paired client whose key is of `method`, on a host with a key of the same method.
fn link(dir: &Dir, method: Method) -> Arc<Link> {
    let store = Arc::new(TrustStore::open(&dir.0).expect("a trust store"));
    let client = Identity::generate(method).expect("a key");
    store
        .accept_key(&client.key_material())
        .expect("the client is paired");
    let host = Identity::generate(method).expect("a key");
    Hub::new(
        &host,
        &store,
        BridgeCommand::new(std::path::Path::new("canoa"), &[]),
    )
    .link(&client.peer_id())
    .expect("a link")
}

fn offer(link: &Link, binding: &str) {
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": 1,
        "clientCapabilities": {"_meta": {CAPABILITY: {
            "version": 1, "methods": ["request", "show"], "bindings": [binding],
        }}},
    }});
    assert!(!link.taken_from_client(initialize.to_string().as_bytes()));
}

fn answer(link: &Arc<Link>, id: u64, result: &Value) {
    link.agent_said(
        json!({"jsonrpc": "2.0", "id": id, "result": result})
            .to_string()
            .as_bytes(),
    );
}

fn servers(message: &[u8]) -> Vec<Value> {
    let value: Value = serde_json::from_slice(message).expect("JSON");
    value["params"]["mcpServers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn new_session(link: &Arc<Link>, id: u64, cwd: &std::path::Path) -> Vec<u8> {
    let message = json!({"jsonrpc": "2.0", "id": id, "method": "session/new",
        "params": {"cwd": cwd, "mcpServers": [{"name": "canoa-secrets", "type": "http", "url": "http://elsewhere", "headers": []}]}});
    link.client_said(message.to_string().into_bytes())
}

#[test]
fn the_mode_names_that_ask_for_nothing_are_the_apps() {
    for id in [
        "bypass",
        "bypassPermissions",
        "full-access",
        "FullAccess",
        "yolo",
    ] {
        assert!(is_bypass(id), "{id}");
    }
    for id in ["default", "acceptEdits", "plan", "manual", ""] {
        assert!(!is_bypass(id), "{id}");
    }
}

#[test]
fn a_session_is_given_the_tools_only_when_the_app_offered_them_and_the_agent_takes_http() {
    let dir = Dir::new("offer");
    let link = link(&dir, Method::WebCrypto);
    let cwd = std::env::temp_dir();
    // Nothing was offered: the client's own entry of that name is left as it is, and is no entry of the host's.
    let untouched = new_session(&link, 2, &cwd);
    assert_eq!(servers(&untouched)[0]["url"], "http://elsewhere");

    offer(&link, "ecdsa-p256-sha256");
    let initialize = json!({"jsonrpc": "2.0", "id": 3, "method": "initialize", "params": {}});
    link.client_said(initialize.to_string().into_bytes());
    // The agent takes no HTTP server: version 1 has stdio in its baseline, so it gets the bridge, the bearer in its environment.
    answer(
        &link,
        3,
        &json!({"agentCapabilities": {"mcpCapabilities": {"http": false}}}),
    );
    let stdio = servers(&new_session(&link, 4, &cwd));
    assert_eq!(stdio[0]["args"], json!(["mcp-bridge"]));
    assert!(stdio[0].get("type").is_none() && stdio[0].get("url").is_none());
    assert_eq!(stdio[0]["env"][1]["value"].as_str().map(str::len), Some(64));

    let initialize = json!({"jsonrpc": "2.0", "id": 5, "method": "initialize", "params": {}});
    link.client_said(initialize.to_string().into_bytes());
    answer(
        &link,
        5,
        &json!({"agentCapabilities": {"mcpCapabilities": {"http": true}}}),
    );
    let given = servers(&new_session(&link, 6, &cwd));
    assert_eq!(
        given.len(),
        1,
        "the client's entry of that name is replaced"
    );
    assert!(
        given[0]["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("http://127.0.0.1:"))
    );
    let bearer = given[0]["headers"][0]["value"].as_str().expect("a bearer");
    assert!(bearer.starts_with("Bearer ") && bearer.len() == "Bearer ".len() + 64);
}

#[test]
fn an_offer_of_a_binding_the_host_does_not_make_is_no_offer() {
    let dir = Dir::new("binding");
    let link = link(&dir, Method::WebCrypto);
    offer(&link, "x25519-hmac-sha256");
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    link.client_said(initialize.to_string().into_bytes());
    answer(
        &link,
        1,
        &json!({"agentCapabilities": {"mcpCapabilities": {"http": true}}}),
    );
    let message = new_session(&link, 2, &std::env::temp_dir());
    assert_eq!(servers(&message)[0]["url"], "http://elsewhere");
}

#[test]
fn the_mode_follows_what_the_client_and_the_agent_say_and_bypass_is_asked_of_it() {
    let dir = Dir::new("mode");
    let link = link(&dir, Method::WebCrypto);
    offer(&link, "ecdsa-p256-sha256");
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    link.client_said(initialize.to_string().into_bytes());
    answer(
        &link,
        1,
        &json!({"agentCapabilities": {"mcpCapabilities": {"http": true}}}),
    );
    let cwd = std::env::temp_dir();
    new_session(&link, 2, &cwd);
    answer(
        &link,
        2,
        &json!({"sessionId": "s1", "modes": {"currentModeId": "default",
        "availableModes": [{"id": "default"}, {"id": "bypassPermissions"}]}}),
    );
    assert!(!link.bypass("s1"));
    link.client_said(
        json!({"jsonrpc": "2.0", "id": 3, "method": "session/set_mode",
        "params": {"sessionId": "s1", "modeId": "bypassPermissions"}})
        .to_string()
        .into_bytes(),
    );
    // Not until the agent has said yes.
    assert!(!link.bypass("s1"));
    answer(&link, 3, &json!({}));
    assert!(link.bypass("s1"));
    link.agent_said(json!({"jsonrpc": "2.0", "method": "session/update", "params": {
        "sessionId": "s1", "update": {"sessionUpdate": "current_mode_update", "currentModeId": "default"}}}).to_string().as_bytes());
    assert!(!link.bypass("s1"));
    // A mode of a session the link does not hold is not Bypass.
    assert!(!link.bypass("s2"));
}

/// A link whose app offered the extension and whose agent takes HTTP servers, with a session `s1` open: its bearer.
fn with_session(link: &Arc<Link>, modes: &Value) -> String {
    offer(link, "ecdsa-p256-sha256");
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    link.client_said(initialize.to_string().into_bytes());
    answer(
        link,
        1,
        &json!({"agentCapabilities": {"mcpCapabilities": {"http": true}}}),
    );
    let given = servers(&new_session(link, 2, &std::env::temp_dir()));
    let bearer = given[0]["headers"][0]["value"].as_str().expect("a bearer");
    let token = bearer.trim_start_matches("Bearer ").to_owned();
    let mut result = json!({"sessionId": "s1"});
    result["modes"] = modes.clone();
    answer(link, 2, &result);
    token
}

fn listed() -> Value {
    json!({"currentModeId": "default", "availableModes": [{"id": "default"}, {"id": "bypassPermissions"}]})
}

#[test]
fn a_connection_that_ends_revokes_its_bearers_and_lets_a_waiting_prompt_go() {
    let dir = Dir::new("close");
    let link = link(&dir, Method::WebCrypto);
    let token = with_session(&link, &listed());
    assert!(
        link.bearer_open(&token),
        "the bearer opens the session while it lasts"
    );
    let waiting = {
        let link = Arc::clone(&link);
        std::thread::spawn(move || {
            link.ask("_coqui_acp/secret_request", json!({"sessionId": "s1"}))
        })
    };
    // The question is on its way to the app, and the thread waits for the person.
    for _ in 0..200 {
        if !link.drain().is_empty() {
            break;
        }
        std::thread::sleep(core::time::Duration::from_millis(10));
    }
    link.close();
    assert!(link.is_closed());
    assert!(
        waiting.join().expect("the thread ended").is_err(),
        "the prompt was let go"
    );
    assert!(!link.bearer_open(&token), "the bearer is revoked");
}

#[test]
fn a_session_that_is_closed_loses_its_bearer() {
    let dir = Dir::new("closed-session");
    let link = link(&dir, Method::WebCrypto);
    let token = with_session(&link, &listed());
    assert!(link.bearer_open(&token));
    let close = json!({"jsonrpc": "2.0", "id": 9, "method": "session/close", "params": {"sessionId": "s1"}});
    link.client_said(close.to_string().into_bytes());
    assert!(!link.bearer_open(&token));
}

#[test]
fn a_version_two_agent_is_given_a_named_stdio_server() {
    let dir = Dir::new("v2");
    let link = link(&dir, Method::WebCrypto);
    // The offer is under `capabilities._meta` in version 2.
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": 2,
        "capabilities": {"_meta": {CAPABILITY: {
            "version": 1, "methods": ["request"], "bindings": ["ecdsa-p256-sha256"],
        }}},
    }});
    assert!(!link.taken_from_client(initialize.to_string().as_bytes()));
    link.client_said(initialize.to_string().into_bytes());
    answer(
        &link,
        1,
        &json!({"protocolVersion": 2, "capabilities": {"session": {"mcp": {"stdio": true}}}}),
    );
    let given = servers(&new_session(&link, 2, &std::env::temp_dir()));
    assert_eq!(given[0]["type"], "stdio");
    assert_eq!(given[0]["args"], json!(["mcp-bridge"]));
    // A version 2 agent that does not say stdio is given nothing.
    let other = Dir::new("v2-none");
    let none = self::link(&other, Method::WebCrypto);
    assert!(!none.taken_from_client(initialize.to_string().as_bytes()));
    none.client_said(initialize.to_string().into_bytes());
    answer(
        &none,
        1,
        &json!({"protocolVersion": 2, "capabilities": {"session": {}}}),
    );
    assert_eq!(
        servers(&new_session(&none, 2, &std::env::temp_dir()))[0]["url"],
        "http://elsewhere"
    );
}

#[test]
fn an_agents_own_report_cannot_raise_its_session_to_full_access() {
    let dir = Dir::new("reports");
    let link = link(&dir, Method::WebCrypto);
    with_session(&link, &listed());
    let update = |update: Value| {
        link.agent_said(
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": update}})
                .to_string()
                .as_bytes(),
        );
    };
    // The three shapes: a listed mode nobody chose, a mode the session never listed, an options update.
    update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "bypassPermissions"}));
    assert!(!link.bypass("s1"));
    update(json!({"sessionUpdate": "current_mode_update", "currentModeId": "yolo"}));
    assert!(!link.bypass("s1"));
    update(
        json!({"sessionUpdate": "config_option_update", "configOptions": [{"id": "mode", "name": "Mode",
        "category": "mode", "type": "select", "currentValue": "full-access",
        "options": [{"value": "full-access", "name": "Full access"}]}]}),
    );
    assert!(!link.bypass("s1"));
}

#[test]
fn an_agent_that_starts_in_full_access_is_not_in_it_for_the_person() {
    let dir = Dir::new("starts");
    let link = link(&dir, Method::WebCrypto);
    let modes = json!({"currentModeId": "bypassPermissions", "availableModes": [{"id": "default"}, {"id": "bypassPermissions"}]});
    with_session(&link, &modes);
    assert!(!link.bypass("s1"));
}

#[test]
fn only_what_the_person_chose_and_the_agent_listed_raises_the_mode_and_a_report_lowers_it() {
    let dir = Dir::new("chosen");
    let link = link(&dir, Method::WebCrypto);
    with_session(&link, &listed());
    let set = |id: u64, mode: &str| {
        link.client_said(
            json!({"jsonrpc": "2.0", "id": id, "method": "session/set_mode", "params": {"sessionId": "s1", "modeId": mode}})
                .to_string()
                .into_bytes(),
        );
        answer(&link, id, &json!({}));
    };
    // A mode the session never listed is refused even when the person's request names it and the agent says yes.
    set(3, "yolo");
    assert!(!link.bypass("s1"));
    set(4, "bypassPermissions");
    assert!(link.bypass("s1"));
    // A report of the agent's own that is not Bypass lowers it.
    link.agent_said(
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1",
            "update": {"sessionUpdate": "current_mode_update", "currentModeId": "default"}}})
        .to_string()
        .as_bytes(),
    );
    assert!(!link.bypass("s1"));
}

#[test]
fn an_answer_nobody_waits_for_never_reaches_the_agent_and_an_initialize_naming_both_is_read() {
    let dir = Dir::new("answers");
    let link = link(&dir, Method::WebCrypto);
    let late = json!({"jsonrpc": "2.0", "id": "canoa-secret-7-00ff00ff00ff00ff", "result": {"action": "accept"}});
    assert!(
        link.taken_from_client(late.to_string().as_bytes()),
        "an answer after the wait is still the host's"
    );
    let escaped = r#"{"jsonrpc":"2.0","id":"canoa-secret-8-00","result":{}}"#;
    assert!(
        link.taken_from_client(escaped.as_bytes()),
        "so is one that escapes its id"
    );
    // The words `initialize` and the capability's name inside an answer must not keep it from being the host's.
    let naming = json!({"jsonrpc": "2.0", "id": "canoa-secret-9-00", "result": {"text": format!("initialize {CAPABILITY}")}});
    assert!(link.taken_from_client(naming.to_string().as_bytes()));
    let other = json!({"jsonrpc": "2.0", "id": 3, "result": {}});
    assert!(
        !link.taken_from_client(other.to_string().as_bytes()),
        "an answer to the agent goes on"
    );
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "clientCapabilities": {"_meta": {CAPABILITY: {"version": 1, "methods": ["request"], "bindings": ["ecdsa-p256-sha256"]}}}}});
    assert!(!link.taken_from_client(initialize.to_string().as_bytes()));
    assert!(link.offered_for_test());
}

#[test]
fn what_the_host_asks_the_app_reaches_neither_the_agent_nor_the_history() {
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::time::Duration;
    use std::sync::Mutex;

    use coqui_transport::Channel as _;

    use super::{Layer, Raw};
    use crate::config::HistoryConfig;
    use crate::history::{Recording, Store};

    const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
    const SEALED: &str = "sealed-ciphertext-stand-in";
    let dir = Dir::new("history");
    let link = link(&dir, Method::WebCrypto);
    let store = Arc::new(Store::open(
        Some(dir.0.join("history")),
        HistoryConfig::default(),
    ));
    let (client, host) = crate::local::pair(1 << 20, Arc::new(AtomicBool::new(false)));
    // The chain a harness entry has: the wire, the host's own questions, the history, then the relay.
    let raw = Raw::new(&host, Some(&link));
    let recording = Recording::new(&raw, &store, "Claude", "claude");
    let layer = Layer::new(&recording, Some(&link));

    let agent_heard = Mutex::new(Vec::<String>::new());
    let stop = AtomicBool::new(false);
    let asked_of_app = Mutex::new(Vec::<String>::new());
    std::thread::scope(|scope| {
        // The relay's side that reads the client.
        scope.spawn(|| {
            while !stop.load(Ordering::SeqCst) {
                if let Ok(Some(message)) = layer.receive(Some(Duration::from_millis(20))) {
                    agent_heard
                        .lock()
                        .expect("not poisoned")
                        .push(String::from_utf8_lossy(&message).into_owned());
                }
            }
        });
        // The person's app: answers what the host asks it, with a sealed value.
        scope.spawn(|| {
            while !stop.load(Ordering::SeqCst) {
                let Ok(Some(message)) = client.receive(Some(Duration::from_millis(20))) else {
                    continue;
                };
                let Ok(value) = serde_json::from_slice::<Value>(&message) else {
                    continue;
                };
                if value["method"] == "_coqui_acp/secret_request" {
                    asked_of_app
                        .lock()
                        .expect("not poisoned")
                        .push(value.to_string());
                    let answer = json!({"jsonrpc": "2.0", "id": value["id"], "result": {"action": "accept", "values": {"password": SEALED}}});
                    client.send(answer.to_string().as_bytes()).expect("sent");
                }
            }
        });
        let send = |value: Value| client.send(value.to_string().as_bytes()).expect("sent");
        let answer = |value: Value| layer.send(value.to_string().as_bytes()).expect("sent");
        // The relay has read a request before the agent answers it, as it does.
        let heard = |text: &str| {
            for _ in 0..200 {
                if agent_heard
                    .lock()
                    .expect("not poisoned")
                    .iter()
                    .any(|m| m.contains(text))
                {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("the relay never read {text}");
        };
        send(
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,
            "clientCapabilities":{"_meta":{CAPABILITY:{"version":1,"methods":["request"],"bindings":["ecdsa-p256-sha256"]}}}}}),
        );
        heard("initialize");
        answer(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,
            "agentCapabilities":{"mcpCapabilities":{"http":true}}}}));
        send(
            json!({"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/work","mcpServers":[]}}),
        );
        heard("session/new");
        answer(json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":ID}}));
        send(
            json!({"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":ID,"prompt":[{"type":"text","text":"log in"}]}}),
        );
        heard("log in");
        // The agent's tool is waiting for the person while its turn goes on.
        let reply = link
            .ask(
                "_coqui_acp/secret_request",
                json!({"sessionId": ID, "requestId": "r1", "message": "Registry login"}),
            )
            .expect("the app answered");
        assert_eq!(reply["values"]["password"], SEALED);
        answer(
            json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":ID,"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"kept as secret_0a1b2c3d"}}}}),
        );
        answer(json!({"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}));
        std::thread::sleep(Duration::from_millis(200));
        stop.store(true, Ordering::SeqCst);
    });
    // The question reached the app, and the answer was kept from the agent, which heard only the client's own messages.
    assert_eq!(asked_of_app.lock().expect("not poisoned").len(), 1);
    let heard = agent_heard.lock().expect("not poisoned").join("\n");
    assert!(heard.contains("session/prompt"), "{heard}");
    // The agent was given the host's entry, so the link was offered; its bearer is nowhere in the history.
    let bearer = heard
        .split("Bearer ")
        .nth(1)
        .and_then(|rest| rest.get(..64))
        .expect("the agent was given a bearer");
    assert!(
        !heard.contains(SEALED) && !heard.contains("canoa-secret-"),
        "{heard}"
    );
    // The history holds the handle the agent said and nothing of the extension.
    let kept = store
        .load("Claude", ID)
        .expect("kept")
        .to_string()
        .expect("written");
    assert!(kept.contains("secret_0a1b2c3d"), "{kept}");
    assert!(!kept.contains(bearer), "the bearer is in the history");
    for file in std::fs::read_dir(dir.0.join("history"))
        .expect("a history directory")
        .flatten()
    {
        let text = std::fs::read_to_string(file.path()).unwrap_or_default();
        assert!(
            !text.contains(bearer),
            "the bearer is in {}",
            file.path().display()
        );
    }
    for absent in [SEALED, "canoa-secret-", "secret_request", "Registry login"] {
        assert!(!kept.contains(absent), "{absent} is in the history: {kept}");
    }
}

#[test]
fn a_version_two_agent_can_be_put_in_full_access_by_the_person() {
    let dir = Dir::new("v2-mode");
    let link = link(&dir, Method::WebCrypto);
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": 2,
        "capabilities": {"_meta": {CAPABILITY: {"version": 1, "methods": ["request"], "bindings": ["ecdsa-p256-sha256"]}}}}});
    assert!(!link.taken_from_client(initialize.to_string().as_bytes()));
    link.client_said(initialize.to_string().into_bytes());
    answer(
        &link,
        1,
        &json!({"protocolVersion": 2, "capabilities": {"session": {"mcp": {"http": true}}}}),
    );
    new_session(&link, 2, &std::env::temp_dir());
    // Version 2 names an option `configId`.
    let options = |current: &str| {
        json!([{"configId": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": current,
            "options": [{"value": "default"}, {"value": "bypass"}]}])
    };
    answer(
        &link,
        2,
        &json!({"sessionId": "s1", "configOptions": options("default")}),
    );
    assert!(!link.bypass("s1"));
    link.client_said(
        json!({"jsonrpc": "2.0", "id": 3, "method": "session/set_config_option",
            "params": {"sessionId": "s1", "configId": "mode", "value": "bypass"}})
        .to_string()
        .into_bytes(),
    );
    answer(&link, 3, &json!({"configOptions": options("bypass")}));
    assert!(link.bypass("s1"));
}

/// A host of `method`, for the in-process link and a paired client's.
fn hub(dir: &Dir, method: Method) -> (Arc<Hub>, Identity) {
    let store = Arc::new(TrustStore::open(&dir.0).expect("a trust store"));
    let host = Identity::generate(method).expect("a key");
    let hub = Hub::new(
        &host,
        &store,
        BridgeCommand::new(std::path::Path::new("canoa"), &[]),
    );
    (hub, host)
}

fn http_agent(link: &Arc<Link>) {
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
    link.client_said(initialize.to_string().into_bytes());
    answer(
        link,
        1,
        &json!({"agentCapabilities": {"mcpCapabilities": {"http": true}}}),
    );
}

#[test]
fn the_in_process_link_is_given_the_tools_on_its_own_binding_and_no_other() {
    for method in [Method::WebCrypto, Method::Noise] {
        let dir = Dir::new("local");
        let (hub, host) = hub(&dir, method);
        let cwd = std::env::temp_dir();

        let local = hub.local_link().expect("the in-process link");
        offer(&local, "link-hmac-sha256");
        http_agent(&local);
        let given = servers(&new_session(&local, 2, &cwd));
        assert!(
            given[0]["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("http://127.0.0.1:")),
            "{method:?}: {given:?}"
        );

        // The binding of a link with a key is not this link's: that offer takes no tools here.
        let local = hub.local_link().expect("the in-process link");
        offer(&local, "ecdsa-p256-sha256");
        http_agent(&local);
        assert_eq!(
            servers(&new_session(&local, 2, &cwd))[0]["url"],
            "http://elsewhere"
        );
        // No peer, the host's own included, is given the in-process link's secret by being asked about its key.
        assert!(hub.link(&host.peer_id()).is_none());
    }
}

#[test]
fn a_connection_with_a_key_never_takes_the_in_process_binding() {
    for method in [Method::WebCrypto, Method::Noise] {
        let dir = Dir::new("relay");
        let link = link(&dir, method);
        offer(&link, "link-hmac-sha256");
        http_agent(&link);
        assert_eq!(
            servers(&new_session(&link, 2, &std::env::temp_dir()))[0]["url"],
            "http://elsewhere",
            "{method:?}"
        );
    }
}

#[test]
fn each_host_mints_its_own_secret_and_keeps_it_for_its_life() {
    let (dir_a, dir_b) = (Dir::new("secret-a"), Dir::new("secret-b"));
    let (a, _) = hub(&dir_a, Method::WebCrypto);
    let (b, _) = hub(&dir_b, Method::WebCrypto);
    let (first, again) = (a.link_secret().expect("a"), a.link_secret().expect("a"));
    assert_eq!(*first, *again);
    assert_ne!(*first, [0_u8; 32]);
    assert_ne!(*first, *b.link_secret().expect("b"));
}
