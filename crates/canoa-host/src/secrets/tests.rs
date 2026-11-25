use std::sync::Arc;

use cabuya::auth::{Identity, Method};
use coqui_acp::secret::CAPABILITY;
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

/// A task's connection is opened by the host for the delegation, in this process, with the host's own identity: nobody answers a
/// prompt there, so whatever it offers, it gets no tools.
#[test]
fn a_task_connection_offering_the_in_process_binding_gets_no_tools() {
    let dir = Dir::new("task");
    let (hub, host) = hub(&dir, Method::WebCrypto);
    assert!(hub.for_connection(&host.peer_id(), true, true).is_none());
    // The same process's own link for the application is given them, and a peer with a key is bound by its key.
    assert!(hub.for_connection(&host.peer_id(), true, false).is_some());
    assert!(hub.for_connection(&host.peer_id(), false, false).is_none());
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

#[test]
fn the_host_asks_a_paired_key_a_form_of_its_own_and_asks_no_other() {
    for method in [Method::WebCrypto, Method::Noise] {
        let dir = Dir::new("asks");
        let host = Identity::generate(method).expect("a key");
        let store = Arc::new(TrustStore::open(&dir.0).expect("a trust store"));
        let paired = Identity::generate(method).expect("a key");
        store
            .accept_key(&paired.key_material())
            .expect("the client is paired");
        let hub = Hub::new(
            &host,
            &store,
            BridgeCommand::new(std::path::Path::new("canoa"), &[]),
        );
        let asker = hub
            .asker_for(&paired.peer_id().to_string())
            .expect("a paired key is asked");
        let schema = r#"{"type":"object","properties":{"api_key":{"type":"string","writeOnly":true}},"required":["api_key"]}"#;
        let (params, _) = asker
            .ask(
                "accounts",
                "Anthropic API key",
                schema,
                crate::secrets::ask::ASK_WAIT,
            )
            .expect("a form");
        assert!(params.get("recipient").is_some(), "{method:?}: {params}");
        let stranger = Identity::generate(method).expect("a key");
        assert!(
            hub.asker_for(&stranger.peer_id().to_string()).is_none(),
            "{method:?}: a key the trust store does not hold"
        );
        assert!(
            hub.asker_for(&host.peer_id().to_string()).is_none(),
            "{method:?}: the host's own key"
        );
        assert!(hub.asker_for("not a peer").is_none());
    }
}

/// What `request` gets back from a program run as the user `nobody` that connects to `port`: `None` when this machine cannot run
/// one (it takes a passwordless `sudo` and `setpriv`).
#[cfg(target_os = "linux")]
fn answer_to_another_user(port: u16, request: &str) -> Option<String> {
    use std::process::Command;

    let can = Command::new("sudo")
        .args([
            "-n",
            "setpriv",
            "--reuid=65534",
            "--regid=65534",
            "--clear-groups",
            "true",
        ])
        .status()
        .is_ok_and(|status| status.success());
    if !can {
        return None;
    }
    let script = r#"exec 3<>"/dev/tcp/127.0.0.1/$0"; printf '%s' "$1" >&3; timeout 5 cat <&3"#;
    let output = Command::new("sudo")
        .args([
            "-n",
            "setpriv",
            "--reuid=65534",
            "--regid=65534",
            "--clear-groups",
        ])
        .args(["bash", "-c", script])
        .arg(port.to_string())
        .arg(request)
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "needs passwordless sudo and setpriv, to run a program as another user"]
fn a_connection_from_another_user_is_dropped_even_with_the_right_bearer() {
    use std::io::{Read as _, Write as _};

    use sha2::{Digest as _, Sha256};

    let tokens = Arc::new(super::Tokens::default());
    let token = "a".repeat(64);
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let _grant = tokens.add(digest);
    let server = super::http::Listener::start(&tokens).expect("the server starts");
    let port = server.addr().port();
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\n\r\n{{}}"
    );

    // The same user gets past the bearer: this grant has no tools yet, which is a 503.
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connects");
    stream.write_all(request.as_bytes()).expect("writes");
    let mut mine = String::new();
    stream.read_to_string(&mut mine).expect("reads");
    assert!(mine.starts_with("HTTP/1.1 503"), "the same user: {mine}");

    let theirs = answer_to_another_user(port, &request)
        .expect("this machine can run a program as another user: passwordless sudo and setpriv");
    assert!(theirs.is_empty(), "another user was answered: {theirs}");
}

/// What the application in this process answers a form the host put to it, with `plain` the plain values as JSON text and `typed` the
/// value of the one secret field: the same sealing the person's app does, bound by the link's secret.
fn app_answers(hub: &Hub, request: &Value, plain: &str, field: &str, typed: &str) -> Value {
    use coqui_acp::secret::{
        DEFAULT_LIFETIME, LinkKey, LinkRole, Method as WireMethod, Prover, RequestParams,
        RequestReply, Verifier,
    };

    let key = LinkKey::derive(&*hub.link_secret().expect("a link secret")).expect("a key");
    let verifier = Verifier::builder(key.identity(LinkRole::App))
        .link(&key, LinkRole::App)
        .expect("the app's side")
        .build();
    let prover = Prover::linked(&key, LinkRole::App);
    let params = RequestParams::from_value(&request["params"]).expect("a request of the extension");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("a clock")
        .as_secs();
    let collector = verifier
        .collector(
            &key.identity(LinkRole::Host),
            WireMethod::Request,
            &params.context,
            &params.recipient,
            now,
        )
        .expect("a collector");
    let sealed = std::collections::BTreeMap::from([(
        field.to_owned(),
        collector.seal(field, typed).expect("sealed"),
    )]);
    let auth = collector
        .authenticate(&prover, plain, &sealed, now, DEFAULT_LIFETIME)
        .expect("bound to the request");
    let reply = RequestReply::Accept {
        plain: plain.to_owned(),
        sealed,
        auth,
    };
    json!({"jsonrpc": "2.0", "id": request["id"], "result": reply.to_value()})
}

#[test]
fn an_agents_request_is_answered_from_what_is_held_and_otherwise_asked_with_the_ways_to_keep_it_and_kept_as_chosen()
 {
    use sha2::{Digest as _, Sha256};

    const VALUE: &str = "zq9-not-a-real-secret-7f3a";
    let dir = Dir::new("kept");
    let (hub, _) = hub(&dir, Method::WebCrypto);
    let link = hub.local_link().expect("the in-process link");
    offer(&link, "link-hmac-sha256");
    http_agent(&link);
    let given = servers(&new_session(&link, 2, &std::env::temp_dir()));
    let token = given[0]["headers"][0]["value"]
        .as_str()
        .expect("a bearer")
        .trim_start_matches("Bearer ")
        .to_owned();
    answer(&link, 2, &json!({"sessionId": "s1"}));
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let grant = hub
        .tokens
        .find(&digest)
        .expect("the bearer opens the session");
    let tools = super::link::tools_of(&grant).expect("the session has its tools");
    let ask = json!({"message": "The deploy needs a token",
        "schema": {"type": "object", "properties": {"deploy_token": {"type": "string", "writeOnly": true}}, "required": ["deploy_token"]}});

    // Nothing is held: the person is asked, in a form that also asks how long to keep it.
    let first = {
        let tools = tools.clone();
        let ask = ask.clone();
        std::thread::spawn(move || tools.call("secret_request", &ask))
    };
    let request = loop {
        if let Some(message) = link.drain().into_iter().next() {
            break serde_json::from_slice::<Value>(&message).expect("JSON");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let schema = request["params"]["schema"].as_str().expect("a schema");
    assert!(
        schema.contains("_keep") && schema.contains("Until Canoa closes"),
        "{schema}"
    );
    assert!(schema.contains("\"const\":\"close\""), "{schema}");
    let reply = app_answers(
        &hub,
        &request,
        r#"{"_keep":"close"}"#,
        "deploy_token",
        VALUE,
    );
    assert!(link.taken_from_client(reply.to_string().as_bytes()));
    let result = first.join().expect("the call ended");
    let result: Value = serde_json::from_str(&result.text).expect("JSON");
    assert_eq!(result["status"], "accepted", "{result}");
    assert!(result["handles"]["deploy_token"].is_string());
    assert!(!result.to_string().contains(VALUE));
    assert!(
        result["plain"].get("_keep").is_none(),
        "the agent is not told how it was kept"
    );

    // It was kept until Canoa closes, as chosen: another session's agent asking for the name is answered, and nobody is asked.
    assert_eq!(
        hub.vault()
            .recall("s2", "deploy_token")
            .as_deref()
            .map(|v| v.as_slice()),
        Some(VALUE.as_bytes())
    );
    let second = tools.call("secret_request", &ask);
    let second: Value = serde_json::from_str(&second.text).expect("JSON");
    assert_eq!(second["status"], "accepted", "{second}");
    assert_ne!(
        second["handles"]["deploy_token"],
        result["handles"]["deploy_token"]
    );
    assert!(link.drain().is_empty(), "the person was asked again");
    assert!(!second.to_string().contains(VALUE));
}

#[test]
fn what_was_kept_for_a_session_ends_with_the_session() {
    let dir = Dir::new("session_end");
    let (hub, _) = hub(&dir, Method::WebCrypto);
    let link = hub.local_link().expect("the in-process link");
    offer(&link, "link-hmac-sha256");
    http_agent(&link);
    new_session(&link, 2, &std::env::temp_dir());
    answer(&link, 2, &json!({"sessionId": "s1"}));
    hub.vault()
        .remember(Some("s1"), "mine", b"zq9-not-a-real-secret-7f3a", "session")
        .expect("kept");
    assert!(hub.vault().recall("s1", "mine").is_some());
    link.client_said(
        json!({"jsonrpc": "2.0", "id": 9, "method": "session/close", "params": {"sessionId": "s1"}})
            .to_string()
            .into_bytes(),
    );
    assert!(
        hub.vault().recall("s1", "mine").is_none(),
        "the session closed"
    );
    // A connection that ends clears what its sessions were given, whichever way they ended.
    new_session(&link, 3, &std::env::temp_dir());
    answer(&link, 3, &json!({"sessionId": "s3"}));
    hub.vault()
        .remember(
            Some("s3"),
            "theirs",
            b"zq9-not-a-real-secret-7f3a",
            "session",
        )
        .expect("kept");
    assert!(hub.vault().recall("s3", "theirs").is_some());
    link.close();
    assert!(
        hub.vault().recall("s3", "theirs").is_none(),
        "the connection ended"
    );
}

/// The tools of the session `s1` of an in-process link, taken through the bearer the host gave its agent.
fn session_tools(hub: &Arc<Hub>, link: &Arc<Link>) -> (coqui_acp_secrets::SessionTools, String) {
    use sha2::{Digest as _, Sha256};

    offer(link, "link-hmac-sha256");
    http_agent(link);
    let given = servers(&new_session(link, 2, &std::env::temp_dir()));
    let token = given[0]["headers"][0]["value"]
        .as_str()
        .expect("a bearer")
        .trim_start_matches("Bearer ")
        .to_owned();
    answer(link, 2, &json!({"sessionId": "s1"}));
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    let grant = hub
        .tokens
        .find(&digest)
        .expect("the bearer opens the session");
    (super::link::tools_of(&grant).expect("its tools"), token)
}

#[test]
fn the_tools_state_the_longest_wait_and_a_question_nobody_answers_in_time_says_so_and_is_withdrawn()
{
    let dir = Dir::new("wait");
    let (hub, _) = hub(&dir, Method::WebCrypto);
    let link = hub.local_link().expect("the in-process link");
    let (tools, _) = session_tools(&hub, &link);
    let listed = tools.definitions();
    let property = &listed
        .iter()
        .find(|tool| tool.name == "secret_request")
        .expect("the request tool")
        .input_schema["properties"]["wait_seconds"];
    assert_eq!(
        property["maximum"], 240,
        "the host's longest wait is in the schema: {property}"
    );
    let started = std::time::Instant::now();
    let result = {
        let tools = tools.clone();
        std::thread::spawn(move || {
            tools.call(
                "secret_request",
                &json!({"message": "m", "wait_seconds": 1, "schema": {"type": "object",
                    "properties": {"waited_token": {"type": "string", "writeOnly": true}}, "required": ["waited_token"]}}),
            )
        })
    }
    .join()
    .expect("the call ended");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "the agent's limit, not the host's, ended the wait"
    );
    let result: Value = serde_json::from_str(&result.text).expect("JSON");
    assert_eq!(result["status"], "timed_out", "{result}");
    let sent: Vec<Value> = link
        .drain()
        .iter()
        .map(|message| serde_json::from_slice(message).expect("JSON"))
        .collect();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[0]["method"], "_coqui_acp/secret_request");
    assert_eq!(sent[1]["method"], "_coqui_acp/secret_withdraw");
    assert!(sent[1].get("id").is_none(), "a notice, not a question");
    assert_eq!(
        sent[1]["params"]["requestId"], sent[0]["params"]["requestId"],
        "the card withdrawn is the one asked"
    );
    assert_eq!(sent[1]["params"]["sessionId"], "s1");
}

#[test]
fn an_agent_whose_person_has_gone_is_told_at_once_that_no_one_is_present() {
    use crate::loopback::Service;
    use sha2::Digest as _;

    let dir = Dir::new("unattended");
    let (hub, _) = hub(&dir, Method::WebCrypto);
    let link = hub.local_link().expect("the in-process link");
    let (_, token) = session_tools(&hub, &link);
    link.close();
    let digest: [u8; 32] = sha2::Sha256::digest(token.as_bytes()).into();
    let grant = hub.tokens.find(&digest).expect("kept to answer the agent");
    assert!(super::link::tools_of(&grant).is_none(), "its tools ended");
    let call = |name: &str| {
        let message = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": name, "arguments": {"message": "m", "schema": {}}}});
        match hub.tokens.answer(&grant, &message) {
            crate::loopback::Answer::Json(answer) => answer,
            _ => panic!("the agent was not answered"),
        }
    };
    let asked = call("secret_request");
    assert_eq!(asked["id"], 7);
    assert_eq!(asked["result"]["isError"], false);
    let text = asked["result"]["content"][0]["text"]
        .as_str()
        .expect("text");
    assert!(text.contains("no_one_present"), "{text}");
    let used = call("secret_use");
    assert_eq!(used["result"]["isError"], true, "{used}");
    let listing = json!({"jsonrpc": "2.0", "id": 8, "method": "tools/list"});
    assert!(matches!(
        hub.tokens.answer(&grant, &listing),
        crate::loopback::Answer::Unavailable
    ));
    assert!(!link.bearer_open(&token), "it opens no session any more");
}

#[test]
fn the_longest_wait_is_a_setting_of_whole_seconds() {
    use super::settings::Settings;

    assert_eq!(Settings::read(None).expect("defaults").max_wait, 240);
    let read = |value: Value| Settings::read(Some(&json!({"max_wait": value})));
    assert_eq!(read(json!(90)).expect("seconds").max_wait, 90);
    for bad in [json!(0), json!(-1), json!("90"), json!(1.5)] {
        assert!(read(bad.clone()).is_err(), "{bad}");
    }
}
