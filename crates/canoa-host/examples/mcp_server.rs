//! A tiny MCP server on its standard streams, the stdio server of the host's and the app's profile tests. Its tool `probe`
//! answers the SHA-256 of `PROBE_TOKEN` in its environment and its arguments, so a test sees what reached it without the
//! value. Its tool `marker` answers the value of `MARKER_WORD` and, when `MARKER_CALLED` names a file, appends the tool's
//! name to it, so a test can tell the tool was called and the environment reached it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "an example names what it uses from std"
)]

use std::io::{BufRead as _, Write as _};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--fail") {
        let _ = writeln!(std::io::stderr(), "mcp_server: failed as requested");
        std::process::exit(1);
    }
    let many = args.iter().any(|arg| arg == "--many");
    // `--slow` answers `initialize` only after the given number of milliseconds, `--cwd` puts the folder it runs in in the
    // description of its resource.
    let slow = args
        .iter()
        .position(|arg| arg == "--slow")
        .and_then(|at| args.get(at + 1))
        .and_then(|millis| millis.parse::<u64>().ok());
    let in_folder = args.iter().any(|arg| arg == "--cwd");
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
            Some("initialize") => {
                if let Some(millis) = slow {
                    std::thread::sleep(std::time::Duration::from_millis(millis));
                }
                json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {}, "resources": {}},
                "serverInfo": {"name": "probe", "version": "0"},
                })
            }
            Some("tools/list") if many => {
                let cursor = message.pointer("/params/cursor").and_then(Value::as_str);
                match cursor {
                    None => json!({
                        "tools": [
                            {"name": "probe", "inputSchema": {"type": "object"}},
                            {"name": "marker", "description": "Returns the marker word.",
                             "inputSchema": {"type": "object", "properties": {}}},
                        ],
                        "nextCursor": "p2",
                    }),
                    Some("p2") => json!({
                        "tools": [
                            {"name": "extra-1", "description": "First extra tool.", "inputSchema": {"type": "object"}},
                            {"name": "extra-2", "description": "Second extra tool.", "inputSchema": {"type": "object"}},
                        ],
                    }),
                    Some(_) => json!({"tools": []}),
                }
            }
            Some("tools/list") => {
                json!({"tools": [
                    {"name": "probe", "inputSchema": {"type": "object"}},
                    {"name": "marker", "description": "Returns the marker word.",
                     "inputSchema": {"type": "object", "properties": {}}},
                ]})
            }
            Some("resources/list") if many => {
                let cursor = message.pointer("/params/cursor").and_then(Value::as_str);
                match cursor {
                    None => json!({
                        "resources": [
                            {"uri": "canoa://probe/readme", "name": "readme", "description": "Probe server readme.", "mimeType": "text/plain"},
                            {"uri": "canoa://probe/extra-1", "name": "extra-1", "mimeType": "text/plain"},
                        ],
                        "nextCursor": "r2",
                    }),
                    Some("r2") => json!({
                        "resources": [
                            {"uri": "canoa://probe/extra-2", "name": "extra-2", "mimeType": "application/json"},
                        ],
                    }),
                    Some(_) => json!({"resources": []}),
                }
            }
            Some("resources/list") => {
                json!({"resources": [
                    {
                        "uri": "canoa://probe/readme",
                        "name": "readme",
                        "description": if in_folder {
                            std::env::current_dir().map_or_else(|_| String::new(), |dir| dir.to_string_lossy().into_owned())
                        } else {
                            "Probe server readme.".to_owned()
                        },
                        "mimeType": "text/plain",
                    }
                ]})
            }
            Some("tools/call") if message["params"]["name"] == "marker" => {
                if let Some(called) = std::env::var_os("MARKER_CALLED")
                    && let Ok(mut file) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(called)
                {
                    let _ = writeln!(file, "marker");
                }
                let word = std::env::var("MARKER_WORD")
                    .unwrap_or_else(|_| "(MARKER_WORD is not set)".to_owned());
                json!({"content": [{"type": "text", "text": word}]})
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
