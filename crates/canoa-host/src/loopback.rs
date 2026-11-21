//! The loopback server behind the tools the host gives an agent is `canoa::loopback`; what is here is what the MCP servers on it
//! share: JSON-RPC errors, `initialize` and the tool methods.

use alloc::sync::Arc;
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

pub(crate) use canoa::loopback::{
    Answer, Caller, Listener, MAX_HEAD, Reply, Request, Route, Service, digest, ours,
};

/// The protocol versions the server speaks, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub(crate) fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// The result of `initialize` for a server of tools called `server`: the newest protocol version both speak, and whether the
/// server says its tool list may change.
pub(crate) fn initialize(server: &str, params: &Value, list_changed: bool) -> Value {
    initialize_with(server, params, list_changed, None)
}

/// [`initialize`], with the `instructions` an agent may put in front of its model for this server.
pub(crate) fn initialize_with(
    server: &str,
    params: &Value,
    list_changed: bool,
    instructions: Option<&str>,
) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = VERSIONS
        .iter()
        .find(|version| Some(**version) == asked)
        .unwrap_or(&VERSIONS[0]);
    let mut result = json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": list_changed}},
        "serverInfo": {"name": server, "version": env!("CARGO_PKG_VERSION")},
    });
    if let Some(instructions) = instructions {
        result["instructions"] = json!(instructions);
    }
    result
}

/// The answer to one JSON-RPC message of the client's for a server of tools, called `server`: its tools as `list` gives them, a
/// call as `call` makes it (an `Err` is the reason it cannot be made). `None` when the message needs no answer.
pub(crate) fn rpc(
    server: &str,
    message: &Value,
    list: impl FnOnce() -> Value,
    call: impl FnOnce(&str, &Value) -> Result<Value, &'static str>,
) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error(
            &Value::Null,
            -32600,
            "one JSON-RPC request or notification per request",
        ));
    };
    let id = object.get("id").cloned();
    let method = object.get("method").and_then(Value::as_str);
    let (Some(id), Some(method)) = (id, method) else {
        // A notification, or an answer to nothing the server asked.
        return None;
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => initialize(server, &params, false),
        "ping" => json!({}),
        "tools/list" => list(),
        "tools/call" => {
            let Some(name) = params.get("name").and_then(Value::as_str) else {
                return Some(error(&id, -32602, "a tool call names its tool"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match call(name, &arguments) {
                Ok(result) => result,
                Err(reason) => return Some(error(&id, -32603, reason)),
            }
        }
        _ => return Some(error(&id, -32601, "no such method")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

/// Bearers in force for a loopback MCP server, keyed by the SHA-256 digest of the token.
pub(crate) struct Tokens<A>(pub(crate) Mutex<HashMap<[u8; 32], Arc<A>>>);

impl<A> Default for Tokens<A> {
    fn default() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
}

impl<A> Tokens<A> {
    pub(crate) fn add(&self, digest: [u8; 32], access: Arc<A>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(digest, access);
    }

    pub(crate) fn find(&self, digest: &[u8; 32]) -> Option<Arc<A>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(digest)
            .cloned()
    }

    pub(crate) fn remove(&self, digest: &[u8; 32]) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(digest);
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }
}

/// Mints a random 32-byte bearer token as lowercase hex together with its SHA-256 digest.
pub(crate) fn mint_bearer() -> Option<(String, [u8; 32])> {
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret).ok()?;
    let token = data_encoding::HEXLOWER.encode(&secret);
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    Some((token, digest))
}

/// Builds an ACP stdio `mcpServers` entry: v1 omits `type`, while v2 sets `"type": "stdio"`.
pub(crate) fn stdio_server_entry(
    name: &str,
    command: &str,
    args: Value,
    env: Value,
    acp_version: u64,
) -> Value {
    let mut entry = json!({
        "name": name,
        "command": command,
        "args": args,
        "env": env,
    });
    if acp_version >= 2 {
        entry["type"] = json!("stdio");
    }
    entry
}

/// Builds a host MCP `mcpServers` entry over HTTP with `Authorization: Bearer` or, when `bridge` is `Some`, the stdio bridge.
pub(crate) fn server_entry(
    name: &str,
    url: &str,
    token: &str,
    bridge: Option<&crate::delegate::BridgeCommand>,
    acp_version: u64,
) -> Value {
    if let Some(bridge) = bridge {
        stdio_server_entry(
            name,
            &bridge.program,
            json!(bridge.args),
            json!([
                {"name": crate::delegate::bridge::URL_VAR, "value": url},
                {"name": crate::delegate::bridge::TOKEN_VAR, "value": token},
            ]),
            acp_version,
        )
    } else {
        json!({
            "type": "http",
            "name": name,
            "url": url,
            "headers": [{"name": "Authorization", "value": format!("Bearer {token}")}],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minted_bearer_digests_and_round_trips_in_tokens() {
        let (token, digest) = mint_bearer().expect("bearer minted");
        assert_eq!(token.len(), 64);
        assert_eq!(<[u8; 32]>::from(Sha256::digest(token.as_bytes())), digest);
        let tokens = Tokens::default();
        let entry = Arc::new(42_u8);
        tokens.add(digest, Arc::clone(&entry));
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens.find(&digest).as_deref(), Some(&42_u8));
        tokens.remove(&digest);
        assert_eq!(tokens.len(), 0);
    }

    #[test]
    fn stdio_server_entry_marks_type_from_acp_v2_only() {
        let bridge = crate::delegate::BridgeCommand {
            program: "/bin/canoa".to_owned(),
            args: vec!["mcp-bridge".to_owned()],
        };
        let v1 = server_entry("canoa-kb", "http://127.0.0.1:9", "tok", Some(&bridge), 1);
        assert_eq!(v1.get("type"), None);
        assert_eq!(v1["name"], "canoa-kb");

        let v2 = server_entry("canoa-kb", "http://127.0.0.1:9", "tok", Some(&bridge), 2);
        assert_eq!(v2["type"], "stdio");

        let http = server_entry("canoa-kb", "http://127.0.0.1:9", "tok", None, 2);
        assert_eq!(http["type"], "http");
        assert_eq!(http["headers"][0]["value"], "Bearer tok");
    }
}
