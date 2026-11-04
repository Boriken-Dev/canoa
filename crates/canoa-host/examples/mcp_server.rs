//! A tiny MCP server on its standard streams, the stdio server of the host's profile tests. Its one tool, `probe`, answers
//! the SHA-256 of `PROBE_TOKEN` in its environment and its arguments, so a test sees what reached it without the value.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::{BufRead as _, Write as _};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

fn main() {
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { return };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = message.get("id").cloned() else {
            continue;
        };
        let result = match message.get("method").and_then(Value::as_str) {
            Some("initialize") => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "probe", "version": "0"},
            }),
            Some("tools/list") => {
                json!({"tools": [{"name": "probe", "inputSchema": {"type": "object"}}]})
            }
            Some("tools/call") => {
                let token = std::env::var("PROBE_TOKEN").ok().map(|token| {
                    Sha256::digest(token.as_bytes())
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>()
                });
                let args: Vec<String> = std::env::args().skip(1).collect();
                let text = json!({"token_sha256": token, "args": args}).to_string();
                json!({"content": [{"type": "text", "text": text}]})
            }
            _ => {
                let _ = writeln!(
                    stdout,
                    "{}",
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})
                );
                let _ = stdout.flush();
                continue;
            }
        };
        let _ = writeln!(
            stdout,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "result": result})
        );
        let _ = stdout.flush();
    }
}
