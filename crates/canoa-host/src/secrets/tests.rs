use std::sync::Arc;

use coqui_acp::secret::CAPABILITY;
use coqui_agentic_peer::auth::{Identity, Method};
use coqui_peer_webcrypto::TrustStore;
use serde_json::{Value, json};

use super::link::is_bypass;
use super::{Hub, Link};

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
    Hub::new(&host, &store)
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
    // The agent takes no HTTP server.
    answer(
        &link,
        3,
        &json!({"agentCapabilities": {"mcpCapabilities": {"http": false}}}),
    );
    assert_eq!(
        servers(&new_session(&link, 4, &cwd))[0]["url"],
        "http://elsewhere"
    );

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
