//! The loopback server behind the tools the host gives an agent is `canoa::loopback`; what is here is what the MCP servers on it
//! share: JSON-RPC errors, `initialize` and the tool methods.

use serde_json::{Value, json};

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
