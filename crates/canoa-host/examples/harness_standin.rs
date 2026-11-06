//! A stand-in for Claude Code's and Codex's own command lines, for tests: no model, nothing spent, nobody's login read.
//! Started with `app-server` first it is Codex's app server, otherwise Claude Code in stream-json mode; `--version` answers
//! as the versions coqui-acp's translators were tested with.
//!
//! Each prompt asks one permission, a file write for Claude Code and a command for Codex, then answers `Done: <prompt>`
//! when it was allowed, writing `standin.txt` in its directory, or `Refused: <prompt>`. When `STANDIN_RECORD` names a file,
//! one JSON line is appended to it per process and per thing it was given: its arguments, the MCP servers its
//! configuration file names, the skills in each plugin directory, the variables `STANDIN_ENV` names (comma-separated) with
//! their values, its process id, and each request.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    clippy::print_stdout,
    reason = "a test program on its standard streams"
)]

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

fn record(entry: &Value) {
    let Ok(path) = std::env::var("STANDIN_RECORD") else {
        return;
    };
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{entry}");
    }
}

fn send(value: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let prefix = format!("--{name}=");
    args.iter()
        .find_map(|arg| arg.strip_prefix(prefix.as_str()))
}

/// The process's arguments, its MCP configuration's server names and the variables asked for.
fn record_start(harness: &str, args: &[String]) {
    let servers: Vec<String> = flag(args, "mcp-config")
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|config| {
            config["mcpServers"]
                .as_object()
                .map(|servers| servers.keys().cloned().collect())
        })
        .unwrap_or_default();
    let env: serde_json::Map<String, Value> = std::env::var("STANDIN_ENV")
        .unwrap_or_default()
        .split(',')
        .filter(|name| !name.is_empty())
        .map(|name| (name.to_owned(), json!(std::env::var(name).ok())))
        .collect();
    let plugin_skills: Vec<String> = args
        .iter()
        .filter_map(|arg| arg.strip_prefix("--plugin-dir="))
        .filter_map(|dir| std::fs::read_dir(Path::new(dir).join("skills")).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    record(
        &json!({"harness": harness, "pid": std::process::id(), "args": args, "mcp_servers": servers,
        "plugin_skills": plugin_skills, "env": env}),
    );
}

fn prompt_text(content: &Value) -> String {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn write_marker(cwd: &Path) {
    let _ = std::fs::write(cwd.join("standin.txt"), "standin\n");
}

fn claude(args: &[String]) {
    record_start("claude", args);
    let session = flag(args, "session-id")
        .or_else(|| flag(args, "resume"))
        .unwrap_or("00000000-0000-4000-8000-000000000000")
        .to_owned();
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut turn = 0_u32;
    let mut waiting: Option<(String, String)> = None;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match message["type"].as_str() {
            Some("control_request") => {
                let id = message["request_id"].clone();
                let subtype = message["request"]["subtype"].as_str().unwrap_or("");
                record(&json!({"control": subtype, "request": message["request"]}));
                let response = if subtype == "initialize" {
                    json!({"commands": [{"name": "standin", "description": "A stand-in's command", "argumentHint": ""}],
                        "models": [{"value": "standin-small", "displayName": "Small", "description": "A stand-in model",
                            "supportsEffort": true, "supportedEffortLevels": ["low", "high"]}]})
                } else {
                    json!({})
                };
                send(&json!({"type": "control_response",
                    "response": {"subtype": "success", "request_id": id, "response": response}}));
                if subtype == "initialize" {
                    send(
                        &json!({"type": "system", "subtype": "init", "cwd": cwd, "session_id": session,
                        "claude_code_version": "2.1.318", "tools": ["Write"], "mcp_servers": [], "model": "standin-small",
                        "permissionMode": "default"}),
                    );
                }
            }
            Some("user") => {
                turn += 1;
                let text = prompt_text(&message["message"]["content"]);
                record(&json!({"prompt": text}));
                let tool = format!("toolu_standin_{turn}");
                let file = cwd.join("standin.txt");
                let input = json!({"file_path": file, "content": "standin\n"});
                send(
                    &json!({"type": "assistant", "session_id": session, "parent_tool_use_id": null,
                    "message": {"id": format!("msg_standin_{turn}"), "type": "message", "role": "assistant",
                        "model": "standin-small", "content": [{"type": "tool_use", "id": tool, "name": "Write", "input": input}],
                        "stop_reason": null, "usage": {"input_tokens": 1, "output_tokens": 1}}}),
                );
                let request = format!("perm-{turn}");
                send(
                    &json!({"type": "control_request", "request_id": request, "request": {
                    "subtype": "can_use_tool", "tool_name": "Write", "display_name": "Write", "input": input,
                    "description": "standin.txt", "permission_suggestions": [], "tool_use_id": tool}}),
                );
                waiting = Some((request, text));
            }
            Some("control_response") => {
                let answered = message["response"]["request_id"].as_str().unwrap_or("");
                let Some((request, text)) = waiting.take_if(|(request, _)| request == answered)
                else {
                    continue;
                };
                let allowed = message["response"]["response"]["behavior"] == "allow";
                record(&json!({"answered": request, "allowed": allowed}));
                let tool = format!("toolu_standin_{turn}");
                let said = if allowed {
                    write_marker(&cwd);
                    send(
                        &json!({"type": "user", "session_id": session, "parent_tool_use_id": null,
                        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool,
                            "content": "File created"}]}}),
                    );
                    format!("Done: {text}")
                } else {
                    send(
                        &json!({"type": "user", "session_id": session, "parent_tool_use_id": null,
                        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool,
                            "content": "The person refused this.", "is_error": true}]}}),
                    );
                    format!("Refused: {text}")
                };
                send(
                    &json!({"type": "assistant", "session_id": session, "parent_tool_use_id": null,
                    "message": {"id": format!("msg_standin_{turn}_end"), "type": "message", "role": "assistant",
                        "model": "standin-small", "content": [{"type": "text", "text": said}],
                        "stop_reason": null, "usage": {"input_tokens": 1, "output_tokens": 1}}}),
                );
                send(
                    &json!({"type": "result", "subtype": "success", "is_error": false, "result": said,
                    "stop_reason": "end_turn", "session_id": session, "total_cost_usd": 0.0,
                    "usage": {"input_tokens": 1, "output_tokens": 1, "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0},
                    "modelUsage": {"standin-small": {"contextWindow": 200_000}}, "permission_denials": []}),
                );
            }
            _ => {}
        }
    }
}

fn codex(args: &[String]) {
    record_start("codex", args);
    let mut thread = String::from("th-standin");
    let mut cwd = PathBuf::from(".");
    let mut turn = 0_u32;
    let mut waiting: Option<(u64, String)> = None;
    let answer = |id: &Value, result: Value| send(&json!({"id": id, "result": result}));
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = message.get("id").cloned();
        match (message["method"].as_str(), id) {
            (Some(method), Some(id)) => {
                let params = message["params"].clone();
                record(&json!({"method": method, "params": params}));
                match method {
                    "initialize" => answer(
                        &id,
                        json!({"userAgent": "standin/0.166.0 (stand-in)",
                        "platformFamily": "standin", "platformOs": "standin"}),
                    ),
                    "model/list" => answer(
                        &id,
                        json!({"data": [{"id": "standin-small", "model": "standin-small",
                        "displayName": "Small", "description": "A stand-in model", "hidden": false,
                        "supportedReasoningEfforts": [{"reasoningEffort": "low", "description": ""},
                            {"reasoningEffort": "high", "description": ""}],
                        "defaultReasoningEffort": "low", "isDefault": true}], "nextCursor": null}),
                    ),
                    "thread/start" | "thread/resume" => {
                        if let Some(asked) = params["threadId"].as_str() {
                            asked.clone_into(&mut thread);
                        }
                        if let Some(dir) = params["cwd"].as_str() {
                            cwd = PathBuf::from(dir);
                        }
                        answer(
                            &id,
                            json!({"thread": {"id": thread, "cwd": cwd, "turns": []},
                            "model": "standin-small", "cwd": cwd}),
                        );
                    }
                    "turn/start" => {
                        turn += 1;
                        let text = prompt_text(&params["input"]);
                        let turn_id = format!("tu-{turn}");
                        answer(
                            &id,
                            json!({"turn": {"id": turn_id, "items": [], "status": "inProgress", "error": null}}),
                        );
                        send(
                            &json!({"method": "turn/started", "params": {"threadId": thread,
                            "turn": {"id": turn_id, "items": [], "status": "inProgress", "error": null}}}),
                        );
                        let request = 9000 + u64::from(turn);
                        send(
                            &json!({"method": "item/commandExecution/requestApproval", "id": request, "params": {
                            "threadId": thread, "turnId": turn_id, "itemId": format!("cmd-{turn}"),
                            "command": "echo standin > standin.txt", "cwd": cwd,
                            "availableDecisions": ["accept", "decline"]}}),
                        );
                        waiting = Some((request, text));
                    }
                    "turn/interrupt" => answer(&id, json!({})),
                    _ => answer(&id, json!({})),
                }
            }
            (None, Some(id)) => {
                let Some((request, text)) = waiting.take_if(|(request, _)| id == json!(*request))
                else {
                    continue;
                };
                let allowed = message["result"]["decision"] == "accept";
                record(&json!({"answered": request, "allowed": allowed}));
                let turn_id = format!("tu-{turn}");
                let said = if allowed {
                    write_marker(&cwd);
                    format!("Done: {text}")
                } else {
                    format!("Refused: {text}")
                };
                let item = format!("msg-{turn}");
                send(
                    &json!({"method": "item/agentMessage/delta", "params": {"threadId": thread, "turnId": turn_id,
                    "itemId": item, "delta": said}}),
                );
                send(
                    &json!({"method": "item/completed", "params": {"threadId": thread, "turnId": turn_id,
                    "item": {"type": "agentMessage", "id": item, "text": said}}}),
                );
                send(
                    &json!({"method": "turn/completed", "params": {"threadId": thread,
                    "turn": {"id": turn_id, "items": [], "status": "completed", "error": null}}}),
                );
            }
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        let codex = std::env::current_exe()
            .ok()
            .and_then(|exe| {
                exe.file_stem()
                    .map(|stem| stem.to_string_lossy().starts_with("codex"))
            })
            .unwrap_or(false);
        println!(
            "{}",
            if codex {
                "codex-cli 0.166.0"
            } else {
                "2.1.318 (Claude Code)"
            }
        );
        return;
    }
    if args.first().map(String::as_str) == Some("app-server") {
        codex(&args);
    } else {
        claude(&args);
    }
}
