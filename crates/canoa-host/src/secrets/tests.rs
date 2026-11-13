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

#[test]
fn a_connection_that_ends_revokes_its_bearers_and_lets_a_waiting_prompt_go() {
    let dir = Dir::new("close");
    let link = link(&dir, Method::WebCrypto);
    link.close();
    assert!(link.is_closed());
    assert!(link.drain().is_empty());
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
        send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}));
        heard("initialize");
        answer(json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}));
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
    for absent in [SEALED, "canoa-secret-", "secret_request", "Registry login"] {
        assert!(!kept.contains(absent), "{absent} is in the history: {kept}");
    }
}
