//! A scripted ACP agent on its standard streams for the host's profile tests. It says it takes a system prompt and more
//! directories; `session/new` starts each stdio MCP server it is given, calls that server's `probe` tool, and answers with
//! the system prompt's text, each tool's answer, the SHA-256 of `TOKEN` in its own environment, the directories and
//! whether `_meta` came. It never answers a variable's value.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Starts `server`, asks its `probe` tool, and ends it: the tool's text, or why there is none.
fn probe(server: &Value) -> String {
    let command = server["command"].as_str().unwrap_or("");
    let args: Vec<&str> = server["args"]
        .as_array()
        .map(|args| args.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let env: Vec<(&str, &str)> = server["env"]
        .as_array()
        .map(|pairs| {
            pairs
                .iter()
                .filter_map(|pair| Some((pair["name"].as_str()?, pair["value"].as_str()?)))
                .collect()
        })
        .unwrap_or_default();
    let Ok(mut child) = Command::new(command)
        .args(&args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
    else {
        return "it did not start".to_owned();
    };
    let (Some(mut input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        return "no streams".to_owned();
    };
    let mut lines = BufReader::new(output).lines();
    let mut answer = |message: Value, id: u64| -> Option<Value> {
        writeln!(input, "{message}").ok()?;
        input.flush().ok()?;
        loop {
            let line = lines.next()?.ok()?;
            let value: Value = serde_json::from_str(&line).ok()?;
            if value["id"] == id {
                return Some(value);
            }
        }
    };
    let said = answer(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "profile-agent", "version": "0"},
        }}),
        1,
    )
    .and_then(|_| {
        answer(
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "probe", "arguments": {}}}),
            2,
        )
    })
    .and_then(|value| value["result"]["content"][0]["text"].as_str().map(str::to_owned))
    .unwrap_or_else(|| "no answer".to_owned());
    let _ = child.kill();
    let _ = child.wait();
    said
}

fn main() {
    let mut stdout = std::io::stdout().lock();
    let mut send = |message: Value| {
        let _ = writeln!(stdout, "{message}");
        let _ = stdout.flush();
    };
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let (Some(method), Some(id)) = (
            message.get("method").and_then(Value::as_str),
            message.get("id").cloned(),
        ) else {
            continue;
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => json!({
                "protocolVersion": 1,
                "agentCapabilities": {"sessionCapabilities": {"systemPrompt": {}, "additionalDirectories": {}}},
                "agentInfo": {"name": "profile-agent", "version": "0"},
            }),
            "session/new" | "session/load" => {
                let prompt: Vec<&str> = params["systemPrompt"]["content"]
                    .as_array()
                    .map(|blocks| {
                        blocks
                            .iter()
                            .filter_map(|block| block["text"].as_str())
                            .collect()
                    })
                    .unwrap_or_default();
                let mut tools = BTreeMap::new();
                let mut names = Vec::new();
                for server in params["mcpServers"].as_array().into_iter().flatten() {
                    let name = server["name"].as_str().unwrap_or("").to_owned();
                    names.push(name.clone());
                    if server.get("command").is_some() {
                        tools.insert(name, probe(server));
                    }
                }
                json!({
                    "sessionId": "profiled",
                    "systemPrompt": prompt.join("\n\n"),
                    "tools": tools,
                    "mcpServers": names,
                    "token_sha256": std::env::var("TOKEN").ok().map(|token| sha256(&token)),
                    "additionalDirectories": params["additionalDirectories"],
                    "meta": params.get("_meta").is_some(),
                })
            }
            "session/prompt" => {
                let text = params["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_owned();
                send(
                    json!({"jsonrpc": "2.0", "method": "session/update", "params": {
                        "sessionId": params["sessionId"],
                        "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": format!("echo: {text}")}},
                    }}),
                );
                json!({"stopReason": "end_turn"})
            }
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
