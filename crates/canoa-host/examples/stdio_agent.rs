//! A small ACP agent on its standard streams, the stdio entry the host's tests serve. It echoes prompts; a prompt starting
//! `ask` first asks the client's permission and answers with the option chosen. It answers `session/new` with the
//! parameters it received, and `_fixture/received` with every method it has seen, so a test can tell what reached it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, Write as _};

use serde_json::{Value, json};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let mut send = |message: Value| {
        let _ = writeln!(stdout, "{message}");
        let _ = stdout.flush();
    };
    let mut seen: Vec<String> = Vec::new();
    // A permission asked, by its request id: the prompt's id and session waiting for the answer.
    let mut asking: BTreeMap<u64, (Value, Value)> = BTreeMap::new();
    let mut next_request = 1000_u64;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { return };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let chunk = |session: &Value, text: String| {
            json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                "sessionId": session,
                "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}},
            }})
        };
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            let answered = message.get("id").and_then(Value::as_u64);
            if let Some((prompt, session)) = answered.and_then(|id| asking.remove(&id)) {
                let chose = &message["result"]["outcome"];
                let said = chose["optionId"]
                    .as_str()
                    .or(chose["outcome"].as_str())
                    .unwrap_or("nothing");
                send(chunk(&session, format!("the person chose {said}")));
                send(json!({"jsonrpc": "2.0", "id": prompt, "result": {"stopReason": "end_turn"}}));
            }
            continue;
        };
        seen.push(method.to_owned());
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => json!({"protocolVersion": 1, "agentCapabilities": {}}),
            "session/new" => json!({"sessionId": "fixture-session", "received": params}),
            "session/prompt" => {
                let text = params["prompt"]
                    .as_array()
                    .and_then(|blocks| blocks.iter().find_map(|block| block["text"].as_str()))
                    .unwrap_or("")
                    .to_owned();
                let session = params["sessionId"].clone();
                if text.starts_with("ask") {
                    next_request += 1;
                    asking.insert(next_request, (id, session.clone()));
                    send(
                        json!({"jsonrpc": "2.0", "id": next_request, "method": "session/request_permission", "params": {
                            "sessionId": session,
                            "toolCall": {"toolCallId": "t1", "title": "Write a file", "kind": "edit"},
                            "options": [
                                {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
                                {"optionId": "reject", "name": "Reject", "kind": "reject_once"},
                            ],
                        }}),
                    );
                    continue;
                }
                send(chunk(&session, format!("echo: {text}")));
                json!({"stopReason": "end_turn"})
            }
            "_fixture/received" => json!({"methods": seen}),
            _ => {
                send(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
                );
                continue;
            }
        };
        send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }
}
